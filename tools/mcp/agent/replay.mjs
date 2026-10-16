#!/usr/bin/env node
// Replays a runlog against another instance and compares the two runs' event
// kinds. The calls that change state are sent again, in the order they were
// recorded: moly.director.command, moly.time, moly.server.edit and
// moly.layout.command. Reads are not sent again; a replayed command is waited
// on the way the agent waited on it (the director's snapshot, the clock's
// step, two frames for a layout command).
//
// The seed comes from the runlog's first line and is read once at the game's
// first draw (`MOLY_SEED`), so a run cannot be reseeded here: a target whose
// seed differs is refused by name instead of comparing two worlds.
//
// Usage: node tools/mcp/agent/replay.mjs <runlog.jsonl> <port> [--settle <s>]
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { rpcClient, runScript, waitFrames, sleep, errorText, TERMINAL_STATUS } from './rpc.mjs';

const STATE_CHANGING = ['moly.director.command', 'moly.time', 'moly.server.edit', 'moly.layout.command'];
const EVENT_POLL_MS = 200;

function readRunlog(file) {
  const lines = readFileSync(file, 'utf8').split(/\r?\n/).filter((line) => line.trim() !== '');
  const records = [];
  for (const [index, line] of lines.entries()) {
    let entry;
    try {
      entry = JSON.parse(line);
    } catch (err) {
      throw new Error(`runlog line ${index + 1} is not JSON: ${err.message}`);
    }
    records.push(entry);
  }
  if (records.length === 0) throw new Error('the runlog is empty');
  const header = records[0];
  if (typeof header.seed !== 'number') {
    throw new Error(`the runlog's first line records no seed (${JSON.stringify(header.seed ?? null)}); it was written without MOLY_SEED`);
  }
  const calls = records.filter((entry) => entry.request?.method !== undefined);
  return { seed: header.seed, task: header.task ?? null, calls, records };
}

// The event kinds of the recorded run: every event of every recorded read,
// once, oldest first.
function recordedKinds(records) {
  const events = new Map();
  for (const entry of records) {
    if (entry.request?.method !== 'moly.events') continue;
    for (const event of entry.result?.events ?? []) {
      if (typeof event.seq === 'number' && typeof event.kind === 'string') events.set(event.seq, event.kind);
    }
  }
  return [...events.entries()].sort((a, b) => a[0] - b[0]).map(([, kind]) => kind);
}

// Reads the instance's events after `since`, oldest first, until no event has
// arrived for `quietMs`. Some events follow a step later (a talk that ends,
// a character that leaves), so the window is the seconds the run is given to
// go quiet, and `ceilingMs` bounds the whole read.
async function readEvents(client, since, { quietMs, ceilingMs }) {
  const events = [];
  let cursor = since;
  let quietSince = Date.now();
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    const read = await client.call('moly.events', { since: cursor, limit: 1000 });
    if (read.error !== undefined) throw new Error(`moly.events: ${errorText(read.error)}`);
    const rows = read.result?.events ?? [];
    for (const event of rows) events.push({ seq: event.seq, kind: event.kind });
    if (typeof read.result?.next === 'number') cursor = read.result.next;
    if (rows.length > 0) quietSince = Date.now();
    if (Date.now() - quietSince >= quietMs || Date.now() >= deadline) return events;
    await sleep(EVENT_POLL_MS);
  }
}

export async function replay({ runlog, port, settleS = 300, quietS = 10, log = console.log } = {}) {
  const { seed, task, calls, records } = readRunlog(runlog);
  const client = rpcClient(port, { timeoutMs: 30_000 });

  const clock = await client.call('moly.time');
  if (clock.error !== undefined) throw new Error(`moly.time: ${errorText(clock.error)}`);
  const here = clock.result?.seed ?? null;
  if (here === null) {
    throw new Error(`the instance has no world seed; the runlog's is ${seed}, so start it with MOLY_SEED=${seed}`);
  }
  if (here !== seed) {
    throw new Error(`the instance's seed is ${here}, the runlog's is ${seed}; start it with MOLY_SEED=${seed}`);
  }
  const start = await client.call('moly.events', { since: 0, limit: 1 });
  if (start.error !== undefined) throw new Error(`moly.events: ${errorText(start.error)}`);
  let cursor = start.result?.latest ?? 0;
  log(`replaying ${task ?? 'a run'} (seed ${seed}) on port ${port}: ${calls.length} recorded calls, from event ${cursor}`);

  for (const entry of calls) {
    const method = entry.request.method;
    if (!STATE_CHANGING.includes(method)) continue;
    const params = entry.request.params;
    if (method === 'moly.director.command') {
      log(`send ${method} ${JSON.stringify(params).slice(0, 120)}`);
      if (params?.run !== undefined || params?.steps !== undefined) {
        const script = params.run ?? params;
        const out = await runScript(client, script, { timeoutMs: settleS * 1000 });
        if (!TERMINAL_STATUS.has(out.status)) {
          throw new Error(`a replayed script did not settle: ${out.status}: ${out.error ?? ''}`);
        }
      } else {
        const sent = await client.call(method, params);
        if (sent.error !== undefined) throw new Error(`${method}: ${errorText(sent.error)}`);
      }
      continue;
    }
    if (method === 'moly.time') {
      const sent = await client.call(method, params);
      if (sent.error !== undefined) throw new Error(`${method}: ${errorText(sent.error)}`);
      if (params?.step !== undefined) {
        const deadline = Date.now() + settleS * 1000;
        for (;;) {
          const read = await client.call('moly.time');
          if (read.error !== undefined) throw new Error(`moly.time: ${errorText(read.error)}`);
          if ((read.result?.stepping ?? 0) === 0) break;
          if (Date.now() >= deadline) throw new Error(`the step of a replayed moly.time never ran out within ${settleS} s`);
          await sleep(50);
        }
      }
      continue;
    }
    log(`send ${method} ${JSON.stringify(params).slice(0, 120)}`);
    const sent = await client.call(method, params);
    if (sent.error !== undefined) throw new Error(`${method}: ${errorText(sent.error)}`);
    if (method === 'moly.layout.command') {
      // The editor runs a command in the next frame; the view reads its
      // result after that.
      await waitFrames(client, 2, { timeoutMs: 10_000 });
    }
  }

  const replayed = (await readEvents(client, cursor, { quietMs: quietS * 1000, ceilingMs: settleS * 1000 })).map((event) => event.kind);
  const original = recordedKinds(records);
  log(`recorded ${original.length} events, replayed ${replayed.length}`);
  const most = Math.max(original.length, replayed.length);
  for (let index = 0; index < most; index += 1) {
    if (original[index] !== replayed[index]) {
      log(`first difference at event ${index + 1}: recorded ${original[index] ?? '(none)'}, replayed ${replayed[index] ?? '(none)'}`);
      return { match: false, at: index, original: original[index] ?? null, replayed: replayed[index] ?? null, originalKinds: original, replayedKinds: replayed };
    }
  }
  log('REPLAY MATCH');
  return { match: true, originalKinds: original, replayedKinds: replayed };
}

async function main() {
  const [runlog, portArg, ...rest] = process.argv.slice(2);
  let settleS = 300;
  let quietS = 10;
  for (let index = 0; index < rest.length; index += 1) {
    const flag = rest[index];
    if (flag !== '--settle' && flag !== '--quiet') continue;
    const value = Number(rest[index + 1]);
    if (!Number.isFinite(value) || value <= 0) {
      console.error(`replay: ${flag} takes a number of seconds above 0, not ${JSON.stringify(rest[index + 1] ?? null)}`);
      process.exitCode = 2;
      return;
    }
    if (flag === '--settle') settleS = value;
    else quietS = value;
  }
  if (!runlog || !portArg || !Number.isInteger(Number(portArg)) || Number(portArg) <= 0) {
    console.error('usage: node tools/mcp/agent/replay.mjs <runlog.jsonl> <port> [--settle <seconds>] [--quiet <seconds>]');
    process.exitCode = 2;
    return;
  }
  try {
    const out = await replay({ runlog, port: Number(portArg), settleS, quietS });
    process.exitCode = out.match ? 0 : 1;
  } catch (err) {
    console.error(`replay: ${err.message}`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
