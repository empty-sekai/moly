#!/usr/bin/env node
// Example check for a native game build: after the entry sequence the player moves with
// the joystick, and a horizontal one-finger drag turns the field camera. The result is
// written to report.txt in outDir.
// Usage: node examples/move-check.mjs <exe> <outDir> <assetRoot> [catalog]
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { mkdirSync, writeFileSync, appendFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SERVER_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const [exeArg, outArg, assetArg, catalog] = process.argv.slice(2);
if (!exeArg || !outArg || !assetArg) {
  throw new Error('usage: node examples/move-check.mjs <exe> <outDir> <assetRoot> [catalog]');
}
// The server resolves the executable and the asset root against its own directory,
// so both are made absolute here.
const exe = path.resolve(exeArg);
const outDir = path.resolve(outArg);
const assetRoot = path.resolve(assetArg);

mkdirSync(outDir, { recursive: true });
const report = path.join(outDir, 'report.txt');
writeFileSync(report, '');
const note = (...parts) => {
  const line = parts.map((p) => (typeof p === 'string' ? p : JSON.stringify(p))).join(' ');
  appendFileSync(report, line + '\n');
  console.log(line);
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const PORT = 15712;
// Planar distance, in the units of player.position, that counts as moved.
const MOVED_DISTANCE = 0.5;

const transport = new StdioClientTransport({
  command: process.execPath,
  args: [path.join(SERVER_DIR, 'server.mjs')],
  cwd: SERVER_DIR,
  env: { ...process.env },
  stderr: 'pipe',
});
transport.stderr?.on('data', (chunk) => appendFileSync(path.join(outDir, 'server-stderr.log'), chunk));
const client = new Client({ name: 'moly-move-check', version: '0.1.0' });
await client.connect(transport);

async function tool(name, args = {}) {
  return client.callTool({ name, arguments: args }, undefined, { timeout: 600_000 });
}
const body = (res) => (res.content ?? []).map((c) => c.text ?? '').join('\n');
async function rpc(method, params) {
  const res = await tool('rpc', { id: inst, method, ...(params === undefined ? {} : { params }) });
  return { error: res.isError === true, value: JSON.parse(body(res)) };
}
// Planar (x, z) distance between two [x, y, z] positions.
const planar = (a, b) => Math.hypot(b[0] - a[0], b[2] - a[2]);

// Director steps. player.stick holds the joystick at stick [x, y] (x right, y up) for
// secs seconds; [0, 1] is full forward. player.release lifts the finger. camera.drag
// moves one finger by [x, y] logical pixels (x right, y down) over secs seconds.
const STICK_STEP = { do: 'player.stick', stick: [0, 1], secs: 1.5 };
const RELEASE_STEP = { do: 'player.release' };
// A short wait lets the character stop after the release.
const SETTLE_STEP = { do: 'wait', secs: 1 };

let inst;
let distance = NaN;
let moved = false;
let cameraChanged = false;
try {
  const launched = JSON.parse(body(await tool('native_launch', {
    exe,
    cwd: path.dirname(exe),
    rpcPort: PORT,
    env: { MOLY_ASSET_ROOT: assetRoot, ...(catalog ? { MOLY_ASSET_CATALOG: catalog } : {}) },
  })));
  inst = launched.id;
  note('launched', launched);

  // The endpoint opens once the app has built; wait for rpc.discover to answer.
  const start = Date.now();
  for (;;) {
    const res = await tool('rpc', { id: inst, method: 'rpc.discover' });
    if (!res.isError) {
      note('discover after', Math.round((Date.now() - start) / 1000), 's');
      break;
    }
    if (Date.now() - start > 300_000) throw new Error('rpc.discover never answered: ' + body(res).slice(0, 300));
    await sleep(1000);
  }

  // Wait for the entry sequence through the director's own condition.
  const entry = await tool('act', { id: inst, step: { do: 'wait_until', until: 'entry_done', timeout: 120 }, timeout: 140 });
  note('wait entry_done', entry.isError === true ? 'error' : 'ok', entry.isError === true ? body(entry).slice(0, 400) : '');
  if (entry.isError === true) throw new Error('entry_done was not reached; stopping before the move check');

  // State after the entry sequence.
  const s0 = await rpc('moly.state');
  const p0 = s0.value?.player?.position;
  note('state after entry: error', s0.error, 'player.position', p0, 'player.state', s0.value?.player?.state, 'entry_done', s0.value?.entry_done);
  note('state after entry: camera', s0.value?.camera);

  // Stick fully forward for 1.5 s, release, settle, then read the position again.
  const stick = await tool('act', { id: inst, step: STICK_STEP, timeout: 30 });
  note('player.stick', JSON.stringify(STICK_STEP), 'status', stick.isError === true ? 'error' : 'ok', stick.isError === true ? body(stick).slice(0, 300) : '');
  const release = await tool('act', { id: inst, step: RELEASE_STEP, timeout: 30 });
  note('player.release status', release.isError === true ? 'error' : 'ok', release.isError === true ? body(release).slice(0, 300) : '');
  const settle = await tool('act', { id: inst, step: SETTLE_STEP, timeout: 30 });
  note('settle status', settle.isError === true ? 'error' : 'ok');
  const s1 = await rpc('moly.state');
  const p1 = s1.value?.player?.position;
  distance = Array.isArray(p0) && Array.isArray(p1) ? planar(p0, p1) : NaN;
  note('state after stick: error', s1.error, 'player.position', p1, 'player.state', s1.value?.player?.state);
  note('planar distance moved (sqrt(dx^2+dz^2))', Number.isFinite(distance) ? distance.toFixed(3) : 'NaN (position missing)');

  // Horizontal one-finger drag of a third of the window width. moly.ui.views reports the
  // window as [width, height]; 1280 is used when no width is reported.
  const views = await rpc('moly.ui.views');
  const reportedWidth = Array.isArray(views.value?.window) ? Number(views.value.window[0]) : NaN;
  const width = reportedWidth > 0 ? reportedWidth : 1280;
  note('window width used', width);
  const dragStep = { do: 'camera.drag', by: [Math.round(width / 3), 0], secs: 1 };
  const camBefore = (await rpc('moly.state')).value?.camera;
  const drag = await tool('act', { id: inst, step: dragStep, timeout: 30 });
  note('camera.drag', JSON.stringify(dragStep), 'status', drag.isError === true ? 'error' : 'ok', drag.isError === true ? body(drag).slice(0, 300) : '');
  const camAfter = (await rpc('moly.state')).value?.camera;
  note('camera before drag', camBefore);
  note('camera after drag', camAfter);
  // camera.yaw is the field camera's yaw as moly.state reports it.
  const yawDelta = Number(camAfter?.yaw) - Number(camBefore?.yaw);
  cameraChanged = Number.isFinite(yawDelta) && Math.abs(yawDelta) > 0.01;
  note('camera.yaw before', camBefore?.yaw, 'after', camAfter?.yaw, 'delta', Number.isFinite(yawDelta) ? yawDelta.toFixed(3) : 'NaN');

  // Log lines with errors, panics or joystick input, from the last 400 lines.
  const pattern = /ERROR|panicked|joystick/;
  const logText = body(await tool('instance_logs', { id: inst, tail: 400 }));
  const hits = logText.split('\n').filter((line) => pattern.test(line));
  note('instance_logs matching lines', hits.length);
  for (const line of hits.slice(0, 20)) note('  log', line);

  moved = Number.isFinite(distance) && distance > MOVED_DISTANCE;
  note(`VERDICT moved=${moved} (distance ${Number.isFinite(distance) ? distance.toFixed(3) : 'NaN'}) camera_changed=${cameraChanged}`);
} catch (err) {
  note('FAILED', err.message);
  if (inst) writeFileSync(path.join(outDir, 'tail.txt'), body(await tool('instance_logs', { id: inst, tail: 200 })));
} finally {
  if (inst) note('stop', body(await tool('native_stop', { id: inst })).slice(0, 200));
  await client.close();
  note('END');
}
