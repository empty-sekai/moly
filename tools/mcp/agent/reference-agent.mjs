#!/usr/bin/env node
// A reference agent: no language model, a task file and the game's own
// observations. It reads the world with moly.observe, acts through director
// steps, appends every request and result to a JSONL runlog, and judges the
// task's success conditions against the observations it took.
//
// Usage: node tools/mcp/agent/reference-agent.mjs <port> <task.json> <runlog.jsonl>
//
// The task file:
//   { "id", "title"?, "site"?, "plan": [items], "success": { "checks": [...] } }
// Plan items, each one action the agent knows:
//   { "do": "wait_entry", "timeout"? }        the entry sequence has ended
//   { "do": "site", "site"?, "timeout"? }     travel to the task's site (or the named one)
//   { "do": "observe", "what": [sections] }   one moly.observe read
//   { "do": "director", "step": {...}, "timeout"? }   one director step
//   { "do": "answer_dialog", "wait"? }        answer a dialog that shows, if one does
//   { "do": "gather", "kind"?, "count" }      harvest that many gatherables
//   { "do": "craft_first", "list"? }          craft the first craftable blueprint
//   { "do": "place", "from"?, "center" }      place a fixture with layout.place
//   { "do": "talk", "unit"? }                 talk to a character on the site
// A task that names a site and does not travel in its plan is taken there
// first, so the agent never needs a launcher to have put the instance there.
// Success checks, each read against the newest observation of its section:
//   { "section": "bag", "rule": "bag_gained", "at_least": n }
//   { "section": "gatherables", "rule": "harvested_kind", "kind": word, "at_least": n }
//   { "section": "fixtures", "rule": "placed_fixtures", "at_least": n }
//   { "section": "characters", "rule": "talking_character", "at_least": n }
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { rpcClient, runScript, runStep, sleep, errorText } from './rpc.mjs';

const STEP_POLL_MS = 250;

export class AgentFailure extends Error {
  constructor(item, message) {
    super(message);
    this.item = item;
  }
}

// One run of one task against one instance on `port`. Lines go to `log`;
// every request and result is appended to `runlog` as runlog.jsonl.
export async function runAgent({ port, task, runlog, log = console.log, stepTimeoutMs = 600_000 } = {}) {
  if (!Number.isInteger(port) || port <= 0) throw new Error(`the port must be a positive integer, not ${port}`);
  if (!task || typeof task.id !== 'string') throw new Error('the task file needs an id');
  if (!Array.isArray(task.plan) || task.plan.length === 0) throw new Error(`task ${task.id}: the plan is empty`);
  if (!task.success || !Array.isArray(task.success.checks) || task.success.checks.length === 0) {
    throw new Error(`task ${task.id}: the success conditions are empty`);
  }

  mkdirSync(path.dirname(path.resolve(runlog)), { recursive: true });
  writeFileSync(runlog, '');
  let lines = 0;
  let running = 0;
  const write = (entry) => {
    appendFileSync(runlog, `${JSON.stringify(entry)}\n`);
    lines += 1;
  };
  const client = rpcClient(port, {
    onCall: (entry, ok) => {
      running += 1;
      // The first call is also the run's header: it carries the task, the
      // port and the world seed the instance reported.
      const header = running === 1 ? { kind: 'run', task: task.id, title: task.title ?? null, site: task.site ?? null, port } : {};
      if (running === 1) header.seed = entry.result?.seed ?? null;
      write({ n: lines + 1, ...header, ...entry });
      if (!ok) log(`rpc ${entry.request.method}: ${errorText(entry.error)}`);
    },
  });

  const observations = new Map();
  const baseline = new Map();
  const notes = [];
  const startedAt = Date.now();
  const seconds = () => Math.round((Date.now() - startedAt) / 100) / 10;
  const step = (item, what) => {
    log(`plan ${item.do}: ${what}`);
  };

  const observe = async (what) => {
    const read = await client.call('moly.observe', what ? { what } : {});
    if (read.error !== undefined) throw new Error(`moly.observe: ${errorText(read.error)}`);
    for (const [section, value] of Object.entries(read.result ?? {})) observations.set(section, value);
    return read.result ?? {};
  };
  const section = (name) => observations.get(name);

  // The events of this run, read after every plan item so the runlog carries
  // the run's own event kinds: a replay compares its run against them.
  let cursor = 0;
  const drainEvents = async () => {
    const read = await client.call('moly.events', { since: cursor, limit: 1000 });
    if (read.error !== undefined) throw new Error(`moly.events: ${errorText(read.error)}`);
    cursor = read.result?.next ?? cursor;
  };

  // A director step, through the same method the MCP server's act tool runs.
  const director = async (item, stepObject, timeoutMs) => {
    const out = await runStep(client, stepObject, { timeoutMs: timeoutMs ?? stepTimeoutMs });
    if (out.status !== 'done') {
      const note = out.error ?? out.status;
      throw new AgentFailure(item, `${stepObject.do}: ${out.status}: ${note}`);
    }
    return out;
  };

  // Travel: the agent's own way to a site, through the game's own site move.
  // It waits for the entry and for no move to be running, sends site.switch
  // and waits for the landing, then reads the site back. An instance already
  // standing there is left alone, so a task never needs a launcher to have
  // put it there.
  const travel = async (item) => {
    const site = item.site ?? task.site;
    if (!site) throw new AgentFailure(item, 'no site to travel to (the item and the task name none)');
    const before = (await observe(['site'])).site?.site_type ?? null;
    if (before === site) {
      step(item, `already on ${site}`);
      return `already on ${site}`;
    }
    const entryS = item.entry_timeout ?? 180;
    const moveS = item.timeout ?? 180;
    step(item, `${before ?? 'no site'} -> ${site}`);
    const out = await runScript(
      client,
      {
        name: `site ${site}`,
        steps: [
          { do: 'wait_until', until: { all: ['entry_done', { not: 'site_moving' }] }, timeout: entryS },
          { do: 'site.switch', site, timeout: 60 },
          { do: 'wait_until', until: { all: [{ not: 'site_moving' }, { site_is: site }] }, timeout: moveS },
        ],
      },
      { timeoutMs: (entryS + moveS + 60) * 1000 },
    );
    if (out.status !== 'done') {
      throw new AgentFailure(item, `site.switch ${site}: ${out.status}: ${out.error ?? ''}`);
    }
    const after = (await observe(['site'])).site?.site_type ?? null;
    if (after !== site) {
      throw new AgentFailure(item, `the instance is on ${after ?? 'no site'} after the move to ${site}`);
    }
    return `on ${site}`;
  };

  const pickGatherable = async (item) => {
    const read = await observe(['gatherables']);
    const rows = read.gatherables?.objects ?? [];
    const left = rows.filter((row) => row.harvested !== true && (item.kind === undefined || row.kind === item.kind));
    if (rows.length === 0) throw new AgentFailure(item, 'this site has no gatherables');
    if (left.length === 0) {
      throw new AgentFailure(item, `no ${item.kind ?? 'gatherable'} left to harvest on this site (${rows.length} objects)`);
    }
    return left[0];
  };

  const gather = async (item) => {
    const count = item.count ?? 1;
    let harvested = 0;
    while (harvested < count) {
      const object = await pickGatherable(item);
      step(item, `gatherable uid ${object.uid} (${object.kind}, hp ${object.hp}) at [${object.position.map((n) => n.toFixed(1)).join(', ')}]`);
      await director(item, { do: 'player.gather', gatherable: object.uid, collect: true, timeout: (item.step_timeout ?? 300) }, (item.step_timeout ?? 300) * 1000 + 30_000);
      harvested += 1;
      const after = await observe(['gatherables']);
      const now = (after.gatherables?.objects ?? []).find((row) => row.uid === object.uid);
      if (now?.harvested !== true) {
        throw new AgentFailure(item, `gatherable ${object.uid} is not harvested after the step`);
      }
      step(item, `${harvested} of ${count} harvested`);
    }
    return `${harvested} gatherables harvested`;
  };

  const craftFirst = async (item) => {
    const list = item.list ?? 'fixture';
    const read = await observe(['recipes']);
    const rows = read.recipes?.[list];
    if (!rows) throw new AgentFailure(item, `the recipes have no ${list} list (${JSON.stringify(read.recipes ?? null).slice(0, 200)})`);
    if (rows.error) throw new AgentFailure(item, `the ${list} craft list is not readable: ${rows.error}`);
    const wanted = rows.find((row) => row.can_craft === true);
    if (!wanted) {
      const first = rows[0];
      const reason = first ? `${first.blueprint_id} (target ${first.target_id}) says ${first.reason ?? 'nothing'}` : 'the list is empty';
      throw new AgentFailure(item, `no blueprint in the ${list} list can be crafted: ${reason}`);
    }
    step(item, `blueprint ${wanted.blueprint_id} (target ${wanted.target_id}, ${wanted.name ?? 'unnamed'})`);
    await director(item, { do: 'player.craft', blueprint: wanted.blueprint_id, timeout: (item.step_timeout ?? 300) }, (item.step_timeout ?? 300) * 1000 + 30_000);
    return { blueprint_id: wanted.blueprint_id, target_id: wanted.target_id, name: wanted.name ?? null };
  };

  // The first tile the placed fixtures do not cover, from the grid's own size.
  const freeTile = async (item) => {
    const read = await observe(['site']);
    const floor = read.site?.floor;
    if (!floor) throw new AgentFailure(item, 'the site observation reports no floor grid');
    const occupied = read.site?.occupied ?? [];
    const covered = (x, z) => occupied.some((row) => row.min[0] <= x && x <= row.max[0] && row.min[2] <= z && z <= row.max[2]);
    for (let z = 0; z < Math.min(floor.depth ?? floor.height, 127); z += 1) {
      for (let x = 0; x < Math.min(floor.width, 127); x += 1) {
        if (!covered(x, z)) return [x, 0, z];
      }
    }
    throw new AgentFailure(item, `every tile of the floor grid (${floor.width} x ${floor.depth ?? floor.height}) is covered`);
  };

  const place = async (item, crafted) => {
    const fixture = item.from === 'crafted' ? crafted?.target_id : item.fixture;
    if (fixture === undefined || fixture === null) {
      throw new AgentFailure(item, 'no fixture to place (craft_first did not run, or the item names no fixture)');
    }
    const center = item.center === 'free' ? await freeTile(item) : item.center;
    if (!Array.isArray(center) || center.length !== 3) throw new AgentFailure(item, `the center is a tile [x, y, z], not ${JSON.stringify(item.center)}`);
    step(item, `fixture ${fixture} at [${center.join(', ')}]`);
    const out = await runStep(
      client,
      { do: 'layout.place', fixture, center, rotate: item.rotate ?? 0, save: item.save ?? true, timeout: (item.step_timeout ?? 180) },
      { timeoutMs: (item.step_timeout ?? 180) * 1000 + 30_000 },
    );
    if (out.status !== 'done') throw new AgentFailure(item, `layout.place: ${out.status}: ${out.error ?? ''}`);
    return { fixture, center, note: out.log.at(-1)?.note ?? null };
  };

  const talk = async (item) => {
    let unit = item.unit;
    if (unit === undefined || unit === 'present') {
      const read = await observe(['characters']);
      const here = (read.characters ?? []).filter((character) => character.away !== true);
      if (here.length === 0) throw new AgentFailure(item, 'no character is on this site');
      unit = here[0].unit;
    }
    step(item, `character ${unit}`);
    await director(item, { do: 'player.talk_to', unit, timeout: (item.step_timeout ?? 120) }, (item.step_timeout ?? 120) * 1000 + 30_000);
    return { unit };
  };

  const answerDialog = async (item) => {
    const waitMs = (item.wait ?? 10) * 1000;
    const deadline = Date.now() + waitMs;
    for (;;) {
      const check = await client.call('moly.director.check', { cond: 'dialog_shown' });
      if (check.error !== undefined) throw new Error(`moly.director.check: ${errorText(check.error)}`);
      if (check.result?.holds === true) {
        step(item, `a dialog is shown (${check.result.read}); answering it`);
        await director(
          item,
          { do: 'dialog.answer', ok: item.ok ?? true, optional: item.optional ?? true, timeout: item.step_timeout ?? 30 },
          (item.step_timeout ?? 30) * 1000 + 30_000,
        );
        return 'answered a dialog';
      }
      if (Date.now() >= deadline) {
        step(item, `no dialog showed within ${(waitMs / 1000).toFixed(0)} s; going on`);
        return 'no dialog showed';
      }
      await sleep(STEP_POLL_MS);
    }
  };

  const check = (condition) => {
    const { rule, section: name, at_least: atLeast = 1 } = condition;
    const read = observations.get(name);
    const holds = (value, text) => ({ holds: value, read: text });
    switch (rule) {
      case 'bag_gained': {
        const sum = (bag) => (bag?.materials ?? []).reduce((total, row) => total + row.quantity, 0);
        const before = sum(baseline.get(name));
        const after = sum(read);
        return holds(after - before >= atLeast, `materials held ${before} -> ${after}`);
      }
      case 'harvested_kind': {
        const rows = (read?.objects ?? []).filter((row) => row.harvested === true && (condition.kind === undefined || row.kind === condition.kind));
        return holds(rows.length >= atLeast, `${rows.length} ${condition.kind ?? ''} gatherables harvested`);
      }
      case 'placed_fixtures': {
        const before = (baseline.get(name)?.placed ?? []).length;
        const after = (read?.placed ?? []).length;
        return holds(after - before >= atLeast, `placed fixtures ${before} -> ${after}`);
      }
      case 'talking_character': {
        const rows = read ?? [];
        const talking = rows.filter((character) => character.talking === true || character.action === 'Talk');
        return holds(talking.length >= atLeast, `${talking.length} of ${rows.length} characters are talking`);
      }
      default:
        throw new Error(`task ${task.id}: unknown success rule ${rule}`);
    }
  };

  log(`task ${task.id} (${task.title ?? ''}) on port ${port}`);
  let failure = null;
  let crafted = null;
  let seed = null;
  try {
    // The seed read is the run's first line: a replay takes it from there.
    const clock = await client.call('moly.time');
    if (clock.error !== undefined) throw new Error(`moly.time: ${errorText(clock.error)}`);
    seed = clock.result?.seed ?? null;
    log(`seed ${seed ?? 'none (MOLY_SEED was not set)'}, frame ${clock.result?.frame}`);

    // The run's own events begin here: what the instance did before the
    // first plan item is not part of the comparison a replay makes.
    const before = await client.call('moly.events', { since: 0, limit: 1 });
    if (before.error !== undefined) throw new Error(`moly.events: ${errorText(before.error)}`);
    cursor = before.result?.latest ?? 0;

    if (task.site) log(`task site ${task.site}`);

    // The readings the success conditions compare against.
    for (const condition of task.success.checks) {
      if (!baseline.has(condition.section)) {
        await observe([condition.section]);
        baseline.set(condition.section, observations.get(condition.section));
      }
    }

    // A task that names a site and does not travel in its plan is taken
    // there first, so the plan is the whole journey.
    const plan = task.site !== undefined && !task.plan.some((item) => item.do === 'site')
      ? [{ do: 'site', site: task.site }, ...task.plan]
      : task.plan;

    const results = [];
    for (const item of plan) {
      let note;
      if (item.do === 'wait_entry') {
        note = (await director(item, { do: 'wait_until', until: 'entry_done', timeout: item.timeout ?? 120 }, ((item.timeout ?? 120) + 30) * 1000)).log.at(-1)?.note ?? 'entry done';
      } else if (item.do === 'site') {
        note = await travel(item);
      } else if (item.do === 'observe') {
        await observe(item.what);
        note = `read ${(item.what ?? []).join(', ') || 'every section'}`;
      } else if (item.do === 'director') {
        note = (await director(item, item.step, (item.timeout ?? 120) * 1000 + 30_000)).log.at(-1)?.note ?? 'done';
      } else if (item.do === 'answer_dialog') {
        note = await answerDialog(item);
      } else if (item.do === 'gather') {
        note = await gather(item);
      } else if (item.do === 'craft_first') {
        crafted = await craftFirst(item);
        note = `crafted blueprint ${crafted.blueprint_id} (fixture ${crafted.target_id})`;
      } else if (item.do === 'place') {
        const placed = await place(item, crafted);
        note = placed.note ?? `placed fixture ${placed.fixture} at [${placed.center.join(', ')}]`;
      } else if (item.do === 'talk') {
        const talked = await talk(item);
        note = `talked to character ${talked.unit}`;
      } else {
        throw new Error(`task ${task.id}: unknown plan action ${JSON.stringify(item.do)}`);
      }
      notes.push({ item: item.do, note });
      write({ kind: 'plan', item: item.do, plan: item, result: 'ok', note });
      log(`plan ${item.do}: ${note}`);
      await drainEvents();
    }

    // The newest reading of every section a condition names.
    const sections = [...new Set(task.success.checks.map((condition) => condition.section))].filter((name) => !plan.some((item) => item.do === 'observe' && (item.what ?? []).includes(name)));
    if (sections.length > 0) await observe(sections);
    await drainEvents();
  } catch (err) {
    failure = err;
    const item = err instanceof AgentFailure ? err.item.do : null;
    write({ kind: 'plan', item, result: 'failed', note: err.message });
    log(`failed: ${err.message}`);
  }

  const checks = [];
  for (const condition of task.success.checks) {
    let outcome;
    try {
      outcome = check(condition);
    } catch (err) {
      outcome = { holds: false, read: err.message };
    }
    checks.push({ ...condition, ...outcome });
    log(`check ${condition.section} ${condition.rule}: ${outcome.holds ? 'ok' : 'failed'} (${outcome.read})`);
  }
  const success = failure === null && checks.every((entry) => entry.holds);
  const result = {
    task: task.id,
    port,
    runlog: path.resolve(runlog),
    seed,
    success,
    seconds: seconds(),
    checks,
    notes,
    failure: failure === null ? null : failure.message,
  };
  write({ kind: 'result', ...result });
  log(`${success ? 'SUCCESS' : 'FAILED'} ${task.id} in ${result.seconds} s${failure === null ? '' : `: ${failure.message}`}`);
  return result;
}

async function main() {
  const [portArg, taskArg, runlogArg] = process.argv.slice(2);
  if (!portArg || !taskArg || !runlogArg) {
    console.error('usage: node tools/mcp/agent/reference-agent.mjs <port> <task.json> <runlog.jsonl>');
    process.exitCode = 2;
    return;
  }
  const port = Number(portArg);
  const task = JSON.parse(readFileSync(taskArg, 'utf8'));
  const result = await runAgent({ port, task, runlog: runlogArg, log: (line) => console.log(line) });
  process.exitCode = result.success ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
