#!/usr/bin/env node
// Paired numeric check of the draw order of head tweet balloons (no images,
// no pixels).
//
// The `MOLY_BALLOON_ORDER_SECS` switch spawns two probe balloons that
// **fully overlap** on screen (the near root sits on the view axis straight
// ahead of the camera; the far anchor is at twice that distance along the
// same view ray through the near anchor, so both anchors project to the same
// screen point). The check runs the game, reads each part's **final sort
// key** from the log (root z + root scale × animation node scale × part
// layer z, the same formula as the renderer) and asserts two orders:
//
//   C1 across balloons: **every** part z of the near balloon is strictly
//      greater than **every** part z of the far one (the ranges do not
//      intersect: one balloon's parts form one layer and never interleave
//      with another's). The HUD layer reorders its children every frame by
//      descending Euclidean distance from the camera to each target, so the
//      nearer balloon draws last and covers the farther one.
//   C2 within a balloon: backdrop < body text < arrow. This is the child
//      order of the HUD prefab's animation node: the bg subtree first (the
//      body text is a child of bg), the arrow container last, and later
//      children draw on top.
//
// Flat run (`MOLY_BALLOON_ORDER_NO_LAYER=1`: every root z is fed 0): C1 must
// fail (the near and far ranges overlap, so there is no covering order) and
// C2 must still pass (part layer z does not depend on the root z). Both runs
// are executed; the script exits 0 only when every expectation holds.
//
// Usage: node tools/balloon-order/check.mjs [moly-app.exe]
//   The exe defaults to the newest moly-app-*.exe in the repository root.
//   moly-server(.exe) must be in the same directory: the check starts the
//   server first, passes the address it reports to the game, and stops the
//   server when the game exits.
//   Each run's full log is written to balloon-order-<run-id>-*.log in the
//   exe's directory, with a JSON status file next to it.

import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const WINDOW_SECS = '10.0';
const RUN_TIMEOUT_MS = Number(process.env.MOLY_BALLOON_ORDER_TIMEOUT_MS ?? 90_000);
if (!Number.isSafeInteger(RUN_TIMEOUT_MS) || RUN_TIMEOUT_MS <= 0) throw new Error('Invalid probe timeout');
const STRIDE = 4.0;
const SERVER_START_MS = 10_000;
const SERVER_FILE = process.platform === 'win32' ? 'moly-server.exe' : 'moly-server';

const READY_RE =
  /\[tweet-order\] probe ready near_tweet=(\d+) near_d=([\d.eE+-]+) far_tweet=(\d+) far_d=([\d.eE+-]+)/;
const PART_RE =
  /\[tweet-order\] probe=(\d) tweet=(\d+) part=(bg|text|arrow) base=([\d.eE+-]+) k=([\d.eE+-]+) z=([\d.eE+-]+)/;

function findExe() {
  if (process.argv.length > 2) {
    return process.argv[2];
  }
  const root = path.resolve(import.meta.dirname, '..', '..');
  const hits = fs
    .readdirSync(root)
    .filter((name) => /^moly-app-.*\.exe$/.test(name))
    .sort();
  if (hits.length === 0) {
    console.error('[check] no exe found: put moly-app-*.exe in the repository root or pass it as an argument');
    process.exit(2);
  }
  return path.join(root, hits[hits.length - 1]);
}

// The game and the server are two processes: start the server first and read
// its address back from the line it reports. The server's own output goes to
// <run>-server.log, separate from the game log the check reads.
function startServer(exe, logPath) {
  const serverExe = path.join(path.dirname(exe), SERVER_FILE);
  if (!fs.existsSync(serverExe) || !fs.statSync(serverExe).isFile()) {
    throw new Error(`${SERVER_FILE} not found: put it in the same directory as ${path.basename(exe)} (${serverExe})`);
  }
  const fd = fs.openSync(logPath, 'a');
  // Pipe output that arrives after close is not written: 'end' can come one
  // tick after shutdown.
  let open = true;
  const append = (text) => {
    if (!open) return;
    try {
      fs.writeSync(fd, text);
    } catch {
      // The log is diagnostic: a failed write must not crash the check.
    }
  };
  append(`# ${new Date().toISOString()} server ${serverExe}\n`);
  const child = spawn(serverExe, [], {
    cwd: path.dirname(exe),
    env: { ...process.env, MOLY_SERVER_ADDR: '127.0.0.1:0' },
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  // Read both pipes to the end: an unread pipe fills up and blocks the server.
  const pipe = (stream, onLine) => {
    let pending = '';
    stream.setEncoding('utf8');
    stream.on('data', (chunk) => {
      pending += chunk;
      const lines = pending.split(/\r?\n/);
      pending = lines.pop();
      for (const line of lines) {
        append(`[server] ${line}\n`);
        onLine(line);
      }
    });
    stream.on('end', () => {
      if (!pending) return;
      append(`[server] ${pending}\n`);
      onLine(pending);
      pending = '';
    });
  };
  const ready = new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error, url) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolve(url);
    };
    const timer = setTimeout(
      () => finish(new Error(`the server reported no address within ${SERVER_START_MS / 1000} s; log ${logPath}`)),
      SERVER_START_MS
    );
    if (child.stdout && child.stderr) {
      pipe(child.stdout, (line) => {
        const match = /listening on (http:\/\/\S+)/.exec(line);
        if (match) finish(null, match[1]);
      });
      pipe(child.stderr, () => {});
    }
    child.once('error', (err) => finish(new Error(`cannot start ${serverExe}: ${err.message}`)));
    child.once('exit', (code, signal) => {
      finish(new Error(`the server exited before reporting an address (${code ?? signal}); log ${logPath}`));
    });
  });
  return {
    child,
    exe: serverExe,
    logPath,
    ready,
    close() {
      if (!open) return;
      open = false;
      fs.closeSync(fd);
    },
  };
}

// Stop the server after the game ends (including a timeout kill), then close
// its log.
async function stopServer(server) {
  const { child } = server;
  if (child.pid !== undefined && child.exitCode === null && child.signalCode === null) {
    if (process.platform === 'win32') {
      spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    } else {
      child.kill('SIGKILL');
    }
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 5000)),
    ]);
  }
  server.close();
}

async function runArm(exe, armName, extraEnv) {
  const runId = randomUUID();
  const env = { ...process.env, MOLY_BALLOON_ORDER_SECS: WINDOW_SECS, MOLY_BALLOON_ORDER_RUN_ID: runId };
  env.RUST_LOG = `${process.env.RUST_LOG ?? 'warn'},moly_world::balloon=info`;
  delete env.MOLY_BALLOON_ORDER_NO_LAYER;
  Object.assign(env, extraEnv);
  const executableSha256 = createHash('sha256').update(fs.readFileSync(exe)).digest('hex');
  const logPath = path.join(path.dirname(exe), `balloon-order-${runId}-${armName}.log`);
  const server = startServer(exe, logPath.replace(/\.log$/, '-server.log'));
  let url;
  try {
    url = await server.ready;
  } catch (error) {
    await stopServer(server);
    throw error;
  }
  let proc;
  try {
    proc = spawnSync(exe, [], {
      env: { ...env, MOLY_SERVER_URL: url },
      cwd: path.dirname(exe),
      timeout: RUN_TIMEOUT_MS,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    });
  } finally {
    await stopServer(server);
  }
  const out = `${proc.stdout ?? ''}${proc.stderr ?? ''}`;
  fs.writeFileSync(logPath, out, 'utf8');
  const result = { runId, armName, executableSha256,
    serverExecutableSha256: createHash('sha256').update(fs.readFileSync(server.exe)).digest('hex'),
    serverUrl: url, serverLogPath: server.logPath,
    logPath, status: proc.status, signal: proc.signal,
    error: proc.error ? { code: proc.error.code, message: proc.error.message } : null };
  fs.writeFileSync(logPath.replace(/\.log$/, '.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(`[${armName}] log: ${logPath} (server ${url}, ${server.logPath})`);
  if (result.error || result.signal || result.status !== 0) {
    throw new Error(`Probe process failed: ${JSON.stringify(result)}`);
  }
  const identity = /\[tweet-order\] run=([0-9a-f-]+) version=(\S+)/.exec(out);
  if (!identity || identity[1] !== runId || identity[2] !== expectedVersion) {
    throw new Error(`Probe run/version identity differs; see ${logPath}`);
  }
  return { out, ...result };
}

function parse(out) {
  const ready = READY_RE.exec(out) ?? null;
  const parts = [];
  for (const m of out.matchAll(new RegExp(PART_RE.source, 'g'))) {
    parts.push({
      probe: Number(m[1]),
      tweet: Number(m[2]),
      kind: m[3],
      base: Number(m[4]),
      k: Number(m[5]),
      z: Number(m[6]),
    });
  }
  return { ready, parts };
}

const zs = (parts, tweet, kind = null) =>
  parts
    .filter((p) => p.tweet === tweet && (kind === null || p.kind === kind))
    .map((p) => p.z);

function checkC1(parts, near, far, arm) {
  const nearZs = zs(parts, near);
  const farZs = zs(parts, far);
  if (nearZs.length === 0 || farZs.length === 0) {
    console.log(`[${arm}] C1 across balloons: parts missing (near ${nearZs.length} / far ${farZs.length}) -> fail`);
    return false;
  }
  const ok = Math.min(...nearZs) > Math.max(...farZs);
  console.log(
    `[${arm}] C1 across balloons: near tweet=${near} z in [${Math.min(...nearZs).toFixed(3)},${Math.max(
      ...nearZs
    ).toFixed(3)}] (${nearZs.length} parts) far tweet=${far} z in [${Math.min(...farZs).toFixed(
      3
    )},${Math.max(...farZs).toFixed(3)}] (${farZs.length} parts) -> ${ok ? 'pass: min(near) > max(far)' : 'fail'}`
  );
  return ok;
}

function checkC2(parts, tweets, arm) {
  let ok = true;
  for (const tweet of tweets) {
    const bg = zs(parts, tweet, 'bg');
    const text = zs(parts, tweet, 'text');
    const arrow = zs(parts, tweet, 'arrow');
    const complete = bg.length > 0 && text.length > 0 && arrow.length > 0;
    const good = complete && Math.max(...bg) < Math.min(...text) && Math.max(...text) < Math.min(...arrow);
    ok = ok && good;
    const desc = complete
      ? `backdrop z<=${Math.max(...bg).toFixed(3)} (${bg.length} parts) < text z>=${Math.min(...text).toFixed(
          3
        )} (${text.length} parts) < arrow z>=${Math.min(...arrow).toFixed(3)} (${arrow.length} parts)`
      : `parts missing (backdrop ${bg.length} / text ${text.length} / arrow ${arrow.length})`;
    console.log(`[${arm}] C2 within balloon tweet=${tweet}: ${desc} -> ${good ? 'pass' : 'fail'}`);
  }
  return ok;
}

function checkSelf(parts, ready, arm) {
  const errors = [];
  if (ready === null) {
    errors.push('no probe ready line (the probe was not built: master table, atlas or camera not ready, or the window is too short)');
    return { errors, near: null, far: null };
  }
  const near = Number(ready[1]);
  const far = Number(ready[3]);
  const nearD = Number(ready[2]);
  const farD = Number(ready[4]);
  const probeParts = parts.filter((p) => p.probe === 1);
  if (![near, far, nearD, farD, ...parts.flatMap(p => [p.tweet, p.base, p.k, p.z])].every(Number.isFinite)) {
    errors.push('the probe contains non-finite values');
  }
  for (const [tweet, name] of [
    [near, 'near'],
    [far, 'far'],
  ]) {
    const own = probeParts.filter((p) => p.tweet === tweet);
    const counts = {
      bg: own.filter((p) => p.kind === 'bg').length,
      text: own.filter((p) => p.kind === 'text').length,
      arrow: own.filter((p) => p.kind === 'arrow').length,
    };
    if (counts.bg < 9) errors.push(`${name} tweet=${tweet} has ${counts.bg} backdrop parts < 9 (nine-slice incomplete)`);
    if (counts.text < 1) errors.push(`${name} tweet=${tweet} has 0 text parts (glyphs missing)`);
    if (counts.arrow !== 1) errors.push(`${name} tweet=${tweet} has ${counts.arrow} arrow parts != 1`);
    if (own.length > 0 && !own.every((p) => p.k > 0)) {
      errors.push(`${name} tweet=${tweet} has parts with k<=0 (the steady-state latch caught a non-steady frame)`);
    }
  }
  if (!(nearD < farD)) errors.push(`probe built the wrong way round: near_d=${nearD} >= far_d=${farD}`);
  console.log(
    `[${arm}] self-check: near tweet=${near} d=${nearD.toFixed(3)} / far tweet=${far} d=${farD.toFixed(
      3
    )}, ${probeParts.length} probe parts -> ${errors.length === 0 ? 'pass' : `fail: ${errors.join('; ')}`}`
  );
  return { errors, near, far };
}

const root = path.resolve(import.meta.dirname, '..', '..');
const expectedVersion = /\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/
  .exec(fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8'))?.[1];
if (!expectedVersion) throw new Error('Workspace version is missing');
const exe = path.resolve(findExe());
const failures = [];

// Layered run: root z wired; C1, C2 and the self-check must pass.
{
  const { out } = await runArm(exe, 'a', {});
  const { ready, parts } = parse(out);
  const { errors, near, far } = checkSelf(parts, ready, 'layered');
  for (const e of errors) failures.push(`layered self-check: ${e}`);
  if (near !== null && far !== null) {
    if (!checkC1(parts, near, far, 'layered')) failures.push('layered C1 failed (the near balloon does not fully cover the far one)');
    if (!checkC2(parts, [near, far], 'layered')) failures.push('layered C2 failed (wrong order within a balloon)');
    const nearBases = parts.filter((p) => p.tweet === near).map((p) => p.base);
    const farBases = parts.filter((p) => p.tweet === far).map((p) => p.base);
    if (nearBases.length > 0 && farBases.length > 0) {
      const nearBase = Math.max(...nearBases);
      const farBase = Math.min(...farBases);
      console.log(
        `[layered] rank band: near base=${nearBase.toFixed(3)} far base=${farBase.toFixed(
          3
        )} (difference ${(nearBase - farBase).toFixed(3)}; the ranges are disjoint only if it is >= the stride ${STRIDE})`
      );
      if (!(nearBase - farBase >= STRIDE)) failures.push(`layered rank band difference < ${STRIDE} (stride wiring suspect)`);
    }
  }
}

// Flat run: every root z fed 0; C1 must fail and C2 must still pass.
{
  const { out } = await runArm(exe, 'b', { MOLY_BALLOON_ORDER_NO_LAYER: '1' });
  const { ready, parts } = parse(out);
  const { errors, near, far } = checkSelf(parts, ready, 'flat');
  for (const e of errors) failures.push(`flat self-check: ${e}`);
  if (near !== null && far !== null) {
    if (checkC1(parts, near, far, 'flat')) {
      failures.push('flat C1 still passes (with equal depths it must fail; the check cannot see the sort wiring)');
    } else {
      console.log('[flat] C1 fails as expected (all root z 0, so the near and far ranges overlap and there is no covering order)');
    }
    if (!checkC2(parts, [near, far], 'flat')) failures.push('flat C2 failed (part layer z must not depend on the root z)');
  }
}

if (failures.length > 0) {
  console.log('[check] failed:');
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
console.log('[check] all paired checks passed: layered C1 and C2 pass, flat C1 fails and C2 passes.');
