#!/usr/bin/env node
// End-to-end check: spawn server.mjs over stdio, drive fake-game.mjs through it, stop it.
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { existsSync } from 'node:fs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const TEST_PORT = 15799;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const EXPECTED_TOOLS = [
  'instance_list', 'native_launch', 'native_stop', 'instance_logs',
  'rpc', 'methods', 'act', 'run_script', 'check', 'capture', 'director_schema',
  'layout_view', 'layout_command', 'layout_place',
  'browser_launch', 'browser_screenshot', 'browser_stop',
];
const STEP_MS = 15_000;
const OVERALL_MS = 45_000;

const serverStderr = [];
const transport = new StdioClientTransport({
  command: process.execPath,
  args: [path.join(HERE, '..', 'server.mjs')],
  cwd: path.join(HERE, '..'),
  env: { ...process.env },
  stderr: 'pipe',
});
transport.stderr?.on('data', (chunk) => serverStderr.push(chunk.toString()));
const client = new Client({ name: 'moly-test-client', version: '0.1.0' });

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label}: no answer within ${ms} ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function assert(condition, message) {
  if (!condition) throw new Error(`assertion failed: ${message}`);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Calls a tool and returns its text; throws if the tool reported isError.
async function callText(name, args = {}) {
  const res = await withTimeout(client.callTool({ name, arguments: args }), STEP_MS, `tool ${name}`);
  const body = (res.content ?? []).map((c) => c.text ?? '').join('\n');
  if (res.isError) throw new Error(`${name} returned an error: ${body}`);
  return body;
}

const callJson = async (name, args) => JSON.parse(await callText(name, args));

// Calls rpc and returns the raw tool result, so the error path can be inspected.
async function callRpc(args) {
  const res = await withTimeout(client.callTool({ name: 'rpc', arguments: args }), STEP_MS, 'tool rpc');
  return { isError: res.isError === true, value: JSON.parse(res.content[0].text) };
}

async function waitFor(check, ms, label) {
  const deadline = Date.now() + ms;
  for (;;) {
    if (await check()) return;
    if (Date.now() > deadline) throw new Error(`${label}: not seen within ${ms} ms`);
    await sleep(200);
  }
}

// Calls a tool and returns the raw result, so isError and image content can be inspected.
async function callRaw(name, args = {}) {
  return withTimeout(client.callTool({ name, arguments: args }), STEP_MS, `tool ${name}`);
}

// Returns the data of the first chunk with this type in a PNG buffer.
function pngChunk(png, type) {
  let off = 8;
  while (off + 8 <= png.length) {
    const len = png.readUInt32BE(off);
    if (png.toString('ascii', off + 4, off + 8) === type) return png.subarray(off + 8, off + 8 + len);
    off += 12 + len;
  }
  return null;
}

// True while the pid still exists. process.kill(pid, 0) throws ESRCH once it is gone.
function pidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

async function portAnswers(port) {
  try {
    await fetch(`http://127.0.0.1:${port}/`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 0, method: 'ping' }),
      signal: AbortSignal.timeout(1000),
    });
    return true;
  } catch {
    return false;
  }
}

async function run() {
  await withTimeout(client.connect(transport), STEP_MS, 'initialize');
  console.log('ok - initialized');

  const { tools } = await withTimeout(client.listTools(), STEP_MS, 'tools/list');
  const names = tools.map((t) => t.name);
  const missing = EXPECTED_TOOLS.filter((n) => !names.includes(n));
  assert(missing.length === 0, `missing tools: ${missing.join(', ')}`);
  console.log(`ok - ${EXPECTED_TOOLS.length} tools listed`);

  const launched = await callJson('native_launch', {
    exe: process.execPath,
    args: [path.join(HERE, 'fake-game.mjs')],
    env: { MOLY_CONTROL_PORT: String(TEST_PORT) },
    rpcPort: TEST_PORT,
  });
  assert(Number.isInteger(launched.pid) && typeof launched.id === 'string', 'native_launch returned no id/pid');
  assert(existsSync(launched.log), `log file not created: ${launched.log}`);
  console.log(`ok - launched ${launched.id} pid=${launched.pid}`);

  await waitFor(
    async () => (await callText('instance_logs', { id: launched.id })).includes(`fake game ready on ${TEST_PORT}`),
    STEP_MS,
    'fake game ready line',
  );
  const readyLines = await callText('instance_logs', { id: launched.id, grep: 'fake game ready' });
  assert(readyLines.includes(`fake game ready on ${TEST_PORT}`), 'ready line missing from instance_logs');
  console.log('ok - fake game ready line in instance_logs');

  const pong = await callJson('rpc', { id: launched.id, method: 'ping' });
  assert(pong === 'pong', `ping reply was ${JSON.stringify(pong)}`);
  console.log('ok - rpc ping -> "pong"');

  const unknown = await callRpc({ id: launched.id, method: 'no.such.method' });
  assert(unknown.isError && unknown.value.code === -32601, `unknown method reply: ${JSON.stringify(unknown)}`);
  console.log('ok - rpc unknown method -> error -32601');

  const methods = await callJson('methods', { id: launched.id });
  assert(methods.discover?.methods?.some((m) => m.name === 'ping'), 'methods: rpc.discover does not list ping');
  assert(
    methods.moly?.methods?.some((m) => m.name === 'moly.director.command'),
    'methods: moly.methods does not list moly.director.command',
  );
  console.log('ok - methods: rpc.discover and moly.methods');

  const schema = await callJson('director_schema', { id: launched.id });
  for (const key of ['common', 'steps', 'conditions', 'definitions']) {
    assert(key in schema, `director_schema has no "${key}" key`);
  }
  console.log('ok - director_schema: common, steps, conditions, definitions');

  const act = await callJson('act', { id: launched.id, step: { wait: 0 }, timeout: 15 });
  assert(act.status === 'done' && act.error === null, `act: ${JSON.stringify(act)}`);
  assert(Array.isArray(act.log) && act.log.length > 0, 'act: log is empty');
  console.log('ok - act: queued -> running -> done');

  const script = await callJson('run_script', {
    id: launched.id,
    script: { name: 'smoke', steps: [{ wait: 0 }, { wait: 0 }] },
    timeout: 15,
  });
  assert(script.status === 'done' && script.error === null, `run_script: ${JSON.stringify(script)}`);
  console.log('ok - run_script (wait) -> done');

  const started = await callJson('run_script', {
    id: launched.id,
    script: { name: 'smoke-nowait', steps: [{ wait: 0 }] },
    wait: false,
  });
  assert(started.status === 'queued', `run_script wait=false: ${JSON.stringify(started)}`);
  console.log('ok - run_script (wait false) returns the first snapshot, queued');

  const check = await callJson('check', { id: launched.id, cond: { name: 'always' } });
  assert(check.holds === true, `check: ${JSON.stringify(check)}`);
  console.log('ok - check: holds true');

  const cap = await callRaw('capture', { id: launched.id, timeout: 5 });
  assert(cap.isError !== true, `capture returned an error: ${JSON.stringify(cap.content)}`);
  const image = cap.content.find((c) => c.type === 'image');
  assert(image && image.mimeType === 'image/png', `capture: no image/png content item in ${JSON.stringify(cap.content)}`);
  const png = Buffer.from(image.data, 'base64');
  assert(png.subarray(0, 8).equals(PNG_SIGNATURE), 'capture: PNG signature missing');
  const ihdr = pngChunk(png, 'IHDR');
  assert(ihdr && ihdr.readUInt32BE(0) === 1 && ihdr.readUInt32BE(4) === 1, 'capture: IHDR is not 1x1');
  const idat = pngChunk(png, 'IDAT');
  assert(idat && zlib.inflateSync(idat).length > 0, 'capture: IDAT does not inflate');
  console.log('ok - capture: image/png, 1x1, IDAT inflates');

  const closed = await callJson('layout_view', { id: launched.id });
  assert(closed.active === false && closed.catalog_rows === 2 && !('catalog' in closed), `layout_view: ${JSON.stringify(closed)}`);
  const withCatalog = await callJson('layout_view', { id: launched.id, catalog: true });
  assert(Array.isArray(withCatalog.catalog) && withCatalog.catalog.length === 2, 'layout_view catalog true: no catalog rows');
  console.log('ok - layout_view: closed editor, catalog cut to its row count unless asked for');

  const placed = await callJson('layout_place', { id: launched.id, fixture_id: 695, center: [3, 0, 4], rotate: 1 });
  assert(placed.placed === true && placed.view.active === true, `layout_place: ${JSON.stringify(placed)}`);
  assert(placed.row.center.join() === '3,0,4' && placed.row.direction === 'Left', `layout_place row: ${JSON.stringify(placed.row)}`);
  assert(placed.view.selected === null && placed.view.placed.length === 1, 'layout_place: selection left or nothing placed');
  console.log('ok - layout_place: opens the editor, places fixture 695 at [3,0,4] turned once');

  const again = await callRaw('layout_place', { id: launched.id, fixture_id: 695, center: [5, 0, 5] });
  assert(again.isError === true && again.content[0].text.includes('every owned copy is placed'), `layout_place again: ${JSON.stringify(again.content)}`);
  console.log('ok - layout_place: no placeable copy left is a tool error');

  const refused = await callRaw('layout_place', { id: launched.id, fixture_id: 100, center: [9, 0, 9] });
  const refusedOut = JSON.parse(refused.content[0].text);
  assert(refused.isError === true && refusedOut.placed === false && refusedOut.put_status === 'Occupied', `layout_place refused: ${JSON.stringify(refusedOut)}`);
  assert(refusedOut.view.selected?.item.fixture_id === 100, 'layout_place refused: the selection was not kept');
  console.log('ok - layout_place: a refused put keeps the selection and is a tool error');

  const cancelled = await callJson('layout_command', { id: launched.id, command: 'cancel' });
  assert(cancelled.selected === null && cancelled.placed.length === 1, `layout_command cancel: ${JSON.stringify(cancelled)}`);
  console.log('ok - layout_command: cancel answers the view after its frame');

  await callJson('native_stop', { id: launched.id });
  await waitFor(async () => !pidAlive(launched.pid), STEP_MS, `pid ${launched.pid} gone`);
  assert(!(await portAnswers(TEST_PORT)), `port ${TEST_PORT} still answers after stop`);
  const list = await callJson('instance_list', {});
  const entry = list.find((i) => i.id === launched.id);
  assert(entry && entry.running === false, 'instance_list still shows the instance running');
  console.log(`ok - native_stop: pid ${launched.pid} gone, port ${TEST_PORT} closed`);
}

const watchdog = setTimeout(() => {
  console.error(`FAIL: overall timeout after ${OVERALL_MS / 1000} s`);
  process.exit(1);
}, OVERALL_MS);

let code = 0;
try {
  await run();
  console.log('PASS');
} catch (err) {
  code = 1;
  console.error(`FAIL: ${err.message}`);
  const tail = serverStderr.join('').split(/\r?\n/).filter(Boolean).slice(-20);
  if (tail.length) console.error(`server stderr (last ${tail.length} lines):\n${tail.join('\n')}`);
} finally {
  clearTimeout(watchdog);
  await withTimeout(client.close(), 5000, 'client close').catch(() => {});
  process.exitCode = code;
  // Force exit if the server child lingers after stdin closes.
  setTimeout(() => process.exit(code), 3000).unref();
}
