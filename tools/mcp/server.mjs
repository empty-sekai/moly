#!/usr/bin/env node
// moly MCP server: process management and transport for game instances.
// Native instances answer JSON-RPC 2.0 over HTTP. Browser instances run in a
// Chrome started by this server and are driven over the Chrome DevTools
// Protocol; the page reaches the same JSON-RPC surface through its control module.
// stdout carries the MCP protocol; every log line goes to stderr.
import { spawn, spawnSync, execFile } from 'node:child_process';
import { once } from 'node:events';
import net from 'node:net';
import { promisify } from 'node:util';
import { existsSync, statSync, mkdirSync, openSync, closeSync, readFileSync, appendFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';

const execFileP = promisify(execFile);
const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOG_DIR = path.join(HERE, 'logs');
const PROFILE_DIR = path.join(HERE, 'profiles');
const DEFAULT_RPC_PORT = 15702;
// The Chrome executable used when browser_launch names none: MOLY_CHROME if set,
// otherwise the standard install location of the platform. On Linux the bare name
// is looked up on PATH.
function defaultChrome() {
  if (process.env.MOLY_CHROME) return process.env.MOLY_CHROME;
  switch (process.platform) {
    case 'win32':
      return 'C:/Program Files/Google/Chrome/Application/chrome.exe';
    case 'darwin':
      return '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
    default:
      return 'google-chrome';
  }
}
const DEFAULT_CHROME = defaultChrome();
const RPC_TIMEOUT_MS = 10_000;
const CDP_TIMEOUT_MS = 10_000;
const CHROME_START_MS = 30_000;
const BOOT_DEFAULT_S = 600;
const STOP_WAIT_MS = 5_000;
const POLL_MS = 250;
const CAPTURE_POLL_MS = 200;
const STEP_POLL_MS = 20;
const EVENT_POLL_MS = 100;
const TERMINAL_STATUS = new Set(['done', 'failed', 'stopped']);

const log = (...args) => console.error('[moly-mcp]', ...args);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
mkdirSync(LOG_DIR, { recursive: true });

/** @type {Map<string, object>} instance id -> instance record (this process only) */
const instances = new Map();
let launchSeq = 0;
let rpcSeq = 0;

const text = (value, isError = false) => ({
  content: [{ type: 'text', text: value }],
  ...(isError ? { isError: true } : {}),
});
const json = (value, isError = false) => text(JSON.stringify(value ?? null, null, 2), isError);

// Errors thrown inside a handler become tool errors, not protocol errors.
const guard = (name, fn) => async (args) => {
  try {
    return await fn(args);
  } catch (err) {
    log(`${name} failed:`, err.message);
    return text(`${name}: ${err.message}`, true);
  }
};

function getInstance(id) {
  const inst = instances.get(id);
  if (!inst) throw new Error(`no instance with id "${id}"`);
  return inst;
}

function describe(inst) {
  return {
    id: inst.id,
    kind: inst.kind,
    pid: inst.pid,
    started: inst.started,
    rpc: inst.rpc,
    running: !inst.exited,
    ...(inst.kind === 'browser' ? { port: inst.port, url: inst.url, profile: inst.profile } : {}),
  };
}

// Starts a child with stdin ignored and stdout and stderr appended to one log file.
function spawnLogged({ exePath, args, cwd, env, logPath, header, hide = true }) {
  appendFileSync(logPath, `# ${new Date().toISOString()} ${header}\n`);
  const fd = openSync(logPath, 'a');
  try {
    return spawn(exePath, args, { cwd, env, stdio: ['ignore', fd, fd], windowsHide: hide });
  } finally {
    closeSync(fd); // the child holds its own duplicate of the handle
  }
}

// Records the child's exit on the instance record. inst.exitPromise resolves once it is gone.
function trackExit(inst) {
  let markExited;
  inst.exitPromise = new Promise((resolve) => { markExited = resolve; });
  inst.child.once('exit', (code, signal) => {
    inst.exited = true;
    inst.exitCode = code;
    inst.signal = signal;
    markExited();
  });
  inst.child.once('error', (err) => {
    log(`instance ${inst.id} error:`, err.message);
    inst.exited = true;
    markExited();
  });
}

async function nativeLaunch({ exe, env, args = [], cwd, rpcPort = DEFAULT_RPC_PORT }) {
  const exePath = path.resolve(exe);
  if (!existsSync(exePath) || !statSync(exePath).isFile()) {
    throw new Error(`executable not found: ${exePath}`);
  }
  const cwdPath = cwd === undefined ? undefined : path.resolve(cwd);
  if (cwdPath !== undefined && !(existsSync(cwdPath) && statSync(cwdPath).isDirectory())) {
    throw new Error(`cwd not found: ${cwdPath}`);
  }

  const id = `native-${++launchSeq}-${randomBytes(3).toString('hex')}`;
  const logPath = path.join(LOG_DIR, `${id}.log`);
  // The control port: the caller's MOLY_CONTROL_PORT if given, else rpcPort. The game
  // is started with it, and inst.rpc points at it.
  const controlPort = env?.MOLY_CONTROL_PORT ?? String(rpcPort);
  // Each instance keeps its documents in its own files beside its log, so a run starts
  // from a first run's state and leaves the user's save alone. The caller's
  // MOLY_SETTINGS_FILE, if given, is used instead.
  const settingsFile = env?.MOLY_SETTINGS_FILE ?? path.join(LOG_DIR, `${id}.settings.json`);
  const child = spawnLogged({
    exePath,
    args,
    cwd: cwdPath,
    env: { ...process.env, ...(env ?? {}), MOLY_CONTROL_PORT: controlPort, MOLY_SETTINGS_FILE: settingsFile },
    logPath,
    header: `launch ${exePath} ${args.join(' ')}`,
    // A hidden first window is created at 0x0: no UI layout, one-pixel captures.
    hide: false,
  });

  const inst = {
    id,
    kind: 'native',
    pid: child.pid,
    started: new Date().toISOString(),
    rpc: `http://127.0.0.1:${controlPort}/`,
    exe: exePath,
    log: logPath,
    child,
    exited: false,
    exitCode: null,
    signal: null,
    stopped: false,
  };
  trackExit(inst);
  try {
    await once(child, 'spawn');
  } catch (err) {
    throw new Error(`failed to spawn ${exePath}: ${err.message}`);
  }
  instances.set(id, inst);
  log(`launched ${id} pid=${child.pid} log=${logPath}`);
  return { id, pid: child.pid, log: logPath };
}

// Kills exactly one process tree: the pid this server spawned.
async function killTree(inst) {
  if (process.platform === 'win32') {
    await execFileP('taskkill', ['/PID', String(inst.pid), '/T', '/F']).catch((err) =>
      log(`taskkill ${inst.pid}:`, (err.stderr || err.message).toString().trim()),
    );
  } else {
    inst.child.kill('SIGKILL');
  }
  await Promise.race([inst.exitPromise, sleep(STOP_WAIT_MS)]);
}

// Shared by native_stop and browser_stop. Only ever touches a process this server spawned.
async function stopInstance(inst) {
  inst.cdp?.close();
  if (!inst.exited) await killTree(inst);
  inst.stopped = true;
  if (!inst.exited) throw new Error(`pid ${inst.pid} did not exit within ${STOP_WAIT_MS / 1000} s after taskkill`);
  return { id: inst.id, pid: inst.pid, stopped: true, exitCode: inst.exitCode, signal: inst.signal };
}

function nativeStopTool(id) {
  const inst = getInstance(id);
  if (inst.kind !== 'native') throw new Error(`instance ${id} is not a native instance`);
  return stopInstance(inst);
}

function instanceLogs({ id, tail = 200, grep }) {
  const inst = getInstance(id);
  const raw = existsSync(inst.log) ? readFileSync(inst.log, 'utf8') : '';
  let lines = raw.split(/\r?\n/);
  if (lines.at(-1) === '') lines.pop();
  if (grep) lines = lines.filter((line) => line.includes(grep));
  const out = tail > 0 ? lines.slice(-tail) : lines;
  return text(out.join('\n'));
}

// ---------------------------------------------------------------------------
// Transport
//
// call(inst, method, params) resolves to {result} on success or {error} on a
// JSON-RPC error or a transport failure. It never throws.
// ---------------------------------------------------------------------------

function parseReply(raw, where) {
  let reply;
  try {
    reply = JSON.parse(raw);
  } catch {
    throw new Error(`non-JSON reply from ${where}: ${String(raw).slice(0, 200)}`);
  }
  return reply?.error !== undefined ? { error: reply.error } : { result: reply?.result ?? null };
}

async function nativeCall(inst, request) {
  let raw;
  try {
    const res = await fetch(inst.rpc, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
      signal: AbortSignal.timeout(RPC_TIMEOUT_MS),
    });
    raw = await res.text();
  } catch (err) {
    if (err.name === 'TimeoutError' || err.name === 'AbortError') {
      throw new Error(`timed out after ${RPC_TIMEOUT_MS / 1000} s on ${inst.rpc}`);
    }
    throw new Error(`transport error on ${inst.rpc}: ${err.cause?.code ?? err.message}`);
  }
  return parseReply(raw, inst.rpc);
}

// Minimal Chrome DevTools Protocol client over the global WebSocket.
class Cdp {
  constructor(url) {
    this.seq = 0;
    this.pending = new Map();
    this.listeners = new Map();
    this.closed = false;
    this.ws = new WebSocket(url);
    this.ws.addEventListener('error', () => {});
    this.ws.addEventListener('message', (ev) => this.#onMessage(ev.data));
    this.ws.addEventListener('close', () => {
      this.closed = true;
      for (const waiter of this.pending.values()) waiter.reject(new Error('DevTools socket closed'));
      this.pending.clear();
    });
  }

  open(timeoutMs) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('DevTools socket did not open')), timeoutMs);
      this.ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
    });
  }

  send(method, params = {}, timeoutMs = CDP_TIMEOUT_MS) {
    if (this.closed) return Promise.reject(new Error('DevTools socket is closed'));
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out after ${timeoutMs / 1000} s`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value); },
        reject: (err) => { clearTimeout(timer); reject(err); },
      });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  on(method, fn) {
    if (!this.listeners.has(method)) this.listeners.set(method, new Set());
    this.listeners.get(method).add(fn);
  }

  off(method, fn) {
    this.listeners.get(method)?.delete(fn);
  }

  // Resolves with the params of the next event with this method name.
  waitFor(method, timeoutMs) {
    return new Promise((resolve, reject) => {
      const fn = (params) => {
        clearTimeout(timer);
        this.off(method, fn);
        resolve(params);
      };
      const timer = setTimeout(() => {
        this.off(method, fn);
        reject(new Error(`${method} not seen within ${timeoutMs / 1000} s`));
      }, timeoutMs);
      this.on(method, fn);
    });
  }

  close() {
    this.closed = true;
    try {
      this.ws.close();
    } catch {
      // already closed
    }
  }

  #onMessage(data) {
    let msg;
    try {
      msg = JSON.parse(String(data));
    } catch {
      return;
    }
    if (msg.id !== undefined) {
      const waiter = this.pending.get(msg.id);
      if (!waiter) return;
      this.pending.delete(msg.id);
      if (msg.error) waiter.reject(new Error(msg.error.message ?? 'DevTools error'));
      else waiter.resolve(msg.result ?? {});
      return;
    }
    for (const fn of this.listeners.get(msg.method) ?? []) fn(msg.params ?? {});
  }
}

// Runs an expression in the page and returns its value by value.
async function cdpEvaluate(cdp, expression, timeoutMs = CDP_TIMEOUT_MS) {
  const res = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, timeoutMs);
  if (res.exceptionDetails) {
    const message = res.exceptionDetails.exception?.description ?? res.exceptionDetails.text ?? 'evaluation failed';
    throw new Error(message.split('\n')[0]);
  }
  return res.result?.value;
}

// The page's control module is found among its loaded resources by path, ignoring the query string.
// The request travels as a JSON string literal, so it needs no escaping by hand.
function controlExpression(requestJson) {
  return `(async () => {
    const entry = performance.getEntriesByType('resource').map((e) => e.name).find((name) => {
      try { return new URL(name).pathname.endsWith('/moly-app.js'); } catch { return false; }
    });
    if (!entry) throw new Error('moly-app.js has not been loaded by the page');
    const m = await import(entry);
    return await m.control(${JSON.stringify(requestJson)});
  })()`;
}

async function browserCall(inst, request) {
  if (!inst.cdp) throw new Error(`browser ${inst.id} has no DevTools session`);
  const value = await cdpEvaluate(inst.cdp, controlExpression(JSON.stringify(request)), RPC_TIMEOUT_MS);
  return parseReply(String(value), `page of ${inst.id}`);
}

async function call(inst, method, params) {
  if (inst.exited) return { error: { message: `instance ${inst.id} is not running` } };
  const request = { jsonrpc: '2.0', id: ++rpcSeq, method, ...(params !== undefined ? { params } : {}) };
  try {
    return inst.kind === 'browser' ? await browserCall(inst, request) : await nativeCall(inst, request);
  } catch (err) {
    return { error: { message: err.message } };
  }
}

// Like call(), but returns the result or throws with the method name in the message.
async function callOk(inst, method, params) {
  const res = await call(inst, method, params);
  if (res.error) throw new Error(`${method}: ${res.error.message ?? JSON.stringify(res.error)}`);
  return res.result;
}

// ---------------------------------------------------------------------------
// Browser instances
// ---------------------------------------------------------------------------

function portFree(port) {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once('error', () => resolve(false));
    srv.listen(port, '127.0.0.1', () => srv.close(() => resolve(true)));
  });
}

async function freePort() {
  const srv = net.createServer();
  srv.listen(0, '127.0.0.1');
  await once(srv, 'listening');
  const { port } = srv.address();
  await new Promise((resolve) => srv.close(resolve));
  return port;
}

async function waitForPageTarget(debugPort, inst, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (inst.exited) throw new Error(`browser exited during start (exit code ${inst.exitCode})`);
    try {
      const res = await fetch(`http://127.0.0.1:${debugPort}/json/list`, { signal: AbortSignal.timeout(2_000) });
      const page = (await res.json()).find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
      if (page) return page;
    } catch {
      // not listening yet
    }
    if (Date.now() > deadline) throw new Error(`no page target on port ${debugPort} within ${timeoutMs / 1000} s`);
    await sleep(POLL_MS);
  }
}

function attachConsole(cdp, logPath) {
  const write = (line) => appendFileSync(logPath, `${new Date().toISOString()} ${line}\n`);
  const remoteText = (arg) => {
    if (arg.value !== undefined) return typeof arg.value === 'string' ? arg.value : JSON.stringify(arg.value);
    return String(arg.description ?? arg.unserializableValue ?? arg.type);
  };
  cdp.on('Runtime.consoleAPICalled', (p) => write(`console.${p.type} ${(p.args ?? []).map(remoteText).join(' ')}`));
  cdp.on('Runtime.exceptionThrown', (p) => {
    const d = p.exceptionDetails ?? {};
    write(`exception ${d.exception?.description ?? d.text ?? ''}`.trimEnd());
  });
  cdp.on('Log.entryAdded', (p) => {
    const e = p.entry ?? {};
    write(`log.${e.level} ${e.source} ${e.text} ${e.url ?? ''}`.trimEnd());
  });
}

const BOOT_STATE_EXPR = `(() => {
  const el = document.getElementById('boot');
  return el
    ? { present: true, state: el.dataset.state ?? '', text: el.textContent.trim().slice(0, 300) }
    : { present: false, state: '', text: '' };
})()`;

const ENTER_CENTRE_EXPR = `(() => {
  const el = document.getElementById('boot-enter');
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})()`;

// Polls the boot screen. Ready means tap #boot-enter once; a missing #boot element means
// the boot screen has gone (or never existed, for a stage page). Returns the outcome.
async function waitForBoot(cdp, timeoutS) {
  const deadline = Date.now() + timeoutS * 1000;
  let tapped = false;
  let sawBoot = false;
  let last = 'none';
  for (;;) {
    let boot = null;
    try {
      boot = await cdpEvaluate(cdp, BOOT_STATE_EXPR, 5_000);
    } catch {
      // the page may still be settling; try again
    }
    if (boot) {
      if (!boot.present) {
        if (tapped) return 'running';
        return sawBoot ? 'dismissed' : 'no-boot';
      }
      sawBoot = true;
      last = boot.state || '(no state)';
      if (boot.state === 'failed') throw new Error(`boot failed: ${boot.text || 'no detail'}`);
      if (boot.state === 'ready' && !tapped) {
        const at = await cdpEvaluate(cdp, ENTER_CENTRE_EXPR, 5_000).catch(() => null);
        if (at) {
          await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: at.x, y: at.y, button: 'left', clickCount: 1 });
          await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: at.x, y: at.y, button: 'left', clickCount: 1 });
          tapped = true;
        }
      }
    }
    if (Date.now() > deadline) throw new Error(`boot did not finish within ${timeoutS} s (last state: ${last})`);
    await sleep(POLL_MS);
  }
}

// A bare name (no path separator) is looked up on PATH, which is how the Linux default
// is found. Any other value is a file path, relative to the directory the server runs in.
function resolveExecutable(name) {
  if (/[\\/]/.test(name)) return path.resolve(name);
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    if (!dir) continue;
    const candidate = path.join(dir, name);
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  throw new Error(`browser executable not found on PATH: ${name}`);
}

async function browserLaunch({ url, width = 1280, height = 720, port, chrome = DEFAULT_CHROME, timeout = BOOT_DEFAULT_S }) {
  const chromePath = resolveExecutable(chrome);
  if (!existsSync(chromePath) || !statSync(chromePath).isFile()) {
    throw new Error(`browser executable not found: ${chromePath}`);
  }
  let debugPort = port;
  if (debugPort === undefined) {
    debugPort = await freePort();
  } else if (!(await portFree(debugPort))) {
    throw new Error(`port ${debugPort} is already in use`);
  }

  const id = `browser-${++launchSeq}-${randomBytes(3).toString('hex')}`;
  const profile = path.join(PROFILE_DIR, id);
  mkdirSync(profile, { recursive: true }); // the id is new, so the profile is new and empty
  const logPath = path.join(LOG_DIR, `${id}.log`);
  const args = [
    `--remote-debugging-port=${debugPort}`,
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--no-default-browser-check',
    `--window-size=${width},${height}`,
    '--autoplay-policy=no-user-gesture-required',
    '--disable-background-timer-throttling',
    '--disable-renderer-backgrounding',
    '--disable-backgrounding-occluded-windows',
    '--new-window',
    'about:blank',
  ];
  const child = spawnLogged({ exePath: chromePath, args, env: process.env, logPath, header: `launch ${chromePath} ${url}` });

  const inst = {
    id,
    kind: 'browser',
    pid: child.pid,
    started: new Date().toISOString(),
    rpc: null,
    port: debugPort,
    url,
    profile,
    log: logPath,
    child,
    exited: false,
    exitCode: null,
    signal: null,
    stopped: false,
    cdp: null,
  };
  trackExit(inst);
  try {
    await once(child, 'spawn');
  } catch (err) {
    throw new Error(`failed to start ${chromePath}: ${err.message}`);
  }
  instances.set(id, inst);

  let state;
  try {
    const target = await waitForPageTarget(debugPort, inst, CHROME_START_MS);
    const cdp = new Cdp(target.webSocketDebuggerUrl);
    inst.cdp = cdp;
    await cdp.open(CDP_TIMEOUT_MS);
    attachConsole(cdp, logPath);
    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    await cdp.send('Log.enable');
    // The page loads hundreds of resources; keep every timing entry so the
    // engine module's URL is still listed when the control transport looks for it.
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', {
      source: 'performance.setResourceTimingBufferSize(100000);',
    });
    // Register the load waiter before navigating so the event cannot be missed.
    const loaded = cdp.waitFor('Page.loadEventFired', timeout * 1000);
    loaded.catch(() => {}); // awaited below; this only marks an early rejection as handled
    const nav = await cdp.send('Page.navigate', { url });
    if (nav.errorText) throw new Error(`navigation failed: ${nav.errorText}`);
    await loaded;
    state = await waitForBoot(cdp, timeout);
  } catch (err) {
    await stopInstance(inst).catch((stopErr) => log(`cleanup of ${id}:`, stopErr.message));
    throw new Error(`${err.message} (log: ${logPath})`);
  }
  log(`launched ${id} pid=${child.pid} port=${debugPort} state=${state}`);
  return { id, pid: child.pid, port: debugPort, state };
}

async function browserScreenshot({ id }) {
  const inst = getInstance(id);
  if (inst.kind !== 'browser') throw new Error(`instance ${id} is not a browser instance`);
  if (inst.exited || !inst.cdp) throw new Error(`instance ${id} is not running`);
  const res = await inst.cdp.send('Page.captureScreenshot', { format: 'png' });
  return { content: [{ type: 'image', data: res.data, mimeType: 'image/png' }] };
}

function browserStopTool(id) {
  const inst = getInstance(id);
  if (inst.kind !== 'browser') throw new Error(`instance ${id} is not a browser instance`);
  return stopInstance(inst);
}

// ---------------------------------------------------------------------------
// Game tools
// ---------------------------------------------------------------------------

const summarize = (snap) => ({
  status: snap.status,
  error: snap.error ?? null,
  log: (snap.log ?? []).slice(-20),
});

// Sends a director command and polls the snapshot until the run is terminal,
// or once for a fire-and-forget start when wait is false.
async function runCommand(inst, script, { wait, timeoutS }) {
  await callOk(inst, 'moly.director.command', { run: script });
  const deadline = Date.now() + timeoutS * 1000;
  for (;;) {
    const snap = (await callOk(inst, 'moly.director.snapshot')) ?? {};
    const out = summarize(snap);
    if (!wait || TERMINAL_STATUS.has(snap.status)) return out;
    if (Date.now() >= deadline) return { ...out, error: `timed out after ${timeoutS} s`, timedOut: true };
    await sleep(POLL_MS);
  }
}

// ---------------------------------------------------------------------------
// Layout editor
// ---------------------------------------------------------------------------

// A layout command runs in the frame after the request; once two frames have
// run past the request the view shows its result.
const LAYOUT_SETTLE_FRAMES = 2;
const LAYOUT_WAIT_MS = 10_000;
const LAYOUT_POLL_MS = 30;

async function gameFrame(inst) {
  const state = (await callOk(inst, 'moly.state')) ?? {};
  if (typeof state.frame !== 'number') throw new Error('moly.state returned no frame');
  return state.frame;
}

// Waits until the game has run `frames` frames past the current one.
async function waitFrames(inst, frames = LAYOUT_SETTLE_FRAMES) {
  const target = (await gameFrame(inst)) + frames;
  const deadline = Date.now() + LAYOUT_WAIT_MS;
  while ((await gameFrame(inst)) < target) {
    if (Date.now() >= deadline) throw new Error(`the game did not run ${frames} frames within ${LAYOUT_WAIT_MS / 1000} s`);
    await sleep(LAYOUT_POLL_MS);
  }
}

async function readLayout(inst) {
  return (await callOk(inst, 'moly.layout.view')) ?? {};
}

// The view with the owned catalog cut to its row count unless it is asked for.
function layoutOut(view, catalog = false) {
  if (catalog || !Array.isArray(view.catalog)) return view;
  const { catalog: rows, ...rest } = view;
  return { ...rest, catalog_rows: rows.length };
}

// Sends one editor command and answers the editor's view after the frame
// that ran it.
async function layoutCommand(inst, params) {
  await callOk(inst, 'moly.layout.command', params);
  await waitFrames(inst);
  return readLayout(inst);
}

// Polls the view until `holds(view)`, failing with `what` at the deadline.
async function layoutUntil(inst, holds, what) {
  const deadline = Date.now() + LAYOUT_WAIT_MS;
  for (;;) {
    const view = await readLayout(inst);
    if (holds(view)) return view;
    if (Date.now() >= deadline) throw new Error(`${what} within ${LAYOUT_WAIT_MS / 1000} s; feedback: ${view.feedback ?? ''}`);
    await sleep(LAYOUT_POLL_MS * 4);
  }
}

// The editor picks y from the column height, so only x and z are compared.
const sameColumn = (a, b) => Array.isArray(a) && Array.isArray(b) && a[0] === b[0] && a[2] === b[2];

// move_to takes the touched tile; the center the view reports can differ
// from it by a tile (an even-sized fixture's center mirrored into the view's
// frame). Moves again by the difference until the reported center is on the
// wanted column, at most twice more.
async function layoutMoveCenter(inst, center) {
  let target = [...center];
  let view = await layoutCommand(inst, { command: 'move_to', center: target });
  for (let again = 0; again < 2; again++) {
    const at = view.selected?.item.center;
    if (!at || sameColumn(at, center)) break;
    target = [target[0] + center[0] - at[0], target[1], target[2] + center[2] - at[2]];
    view = await layoutCommand(inst, { command: 'move_to', center: target });
  }
  return view;
}

// Opens the editor when it is closed, picks the catalog row of the fixture,
// turns it, moves its center to the tile and decides it when its put status
// allows.
async function layoutPlace(inst, { fixture_id, texture_id, center, rotate = 0, decide = true }) {
  let view = await readLayout(inst);
  if (!view.active) {
    await layoutCommand(inst, { command: 'enter' });
    view = await layoutUntil(inst, (v) => v.active === true, 'the layout editor did not open');
  }
  if (view.selected) {
    throw new Error(`fixture ${view.selected.item?.uid} is selected; decide or cancel it first`);
  }
  const rows = (view.catalog ?? []).filter(
    (row) => row.fixture_id === fixture_id && (texture_id === undefined || row.texture_id === texture_id),
  );
  if (rows.length === 0) throw new Error(`fixture ${fixture_id}${texture_id === undefined ? '' : ` texture ${texture_id}`} is not in the owned catalog`);
  const row = rows.find((candidate) => candidate.placeable > 0);
  if (!row) throw new Error(`fixture ${fixture_id}: every owned copy is placed (catalog rows ${rows.map((r) => r.index).join(', ')})`);
  view = await layoutCommand(inst, { command: 'select_catalog', index: row.index });
  if (!view.selected) throw new Error(`select_catalog ${row.index} selected nothing; feedback: ${view.feedback}`);
  for (let turn = 0; turn < rotate; turn++) view = await layoutCommand(inst, { command: 'rotate' });
  view = await layoutMoveCenter(inst, center);
  const selected = view.selected;
  if (!selected) throw new Error(`the selection was dropped; feedback: ${view.feedback}`);
  const out = { catalog_index: row.index, uid: selected.item.uid, center: selected.item.center, direction: selected.item.direction, put_status: selected.put_status };
  if (!sameColumn(selected.item.center, center)) {
    return { ...out, placed: false, reason: `the editor kept the fixture's center at ${JSON.stringify(selected.item.center)}; feedback: ${view.feedback}`, view: layoutOut(view) };
  }
  if (selected.put_status !== 'ok' || !decide) {
    return { ...out, placed: false, reason: selected.put_status !== 'ok' ? `put status ${selected.put_status}` : 'decide was false', view: layoutOut(view) };
  }
  view = await layoutCommand(inst, { command: 'decide' });
  const placed = (view.placed ?? []).find((item) => item.uid === selected.item.uid);
  if (view.selected || !placed) {
    return { ...out, placed: false, reason: `decide did not place it; feedback: ${view.feedback}`, view: layoutOut(view) };
  }
  return { ...out, placed: true, row: placed, view: layoutOut(view) };
}

// ---------------------------------------------------------------------------
// MCP tools
// ---------------------------------------------------------------------------

const server = new McpServer({ name: 'moly', version: '0.0.3' });

server.registerTool(
  'instance_list',
  { description: 'List instances started by this server: id, kind, pid, started, rpc URL (native) or port/url (browser), running.' },
  guard('instance_list', async () => json([...instances.values()].map(describe))),
);

server.registerTool(
  'native_launch',
  {
    description:
      'Start a native executable (absolute or relative file path, no PATH lookup). stdin is closed; stdout and stderr go to logs/<id>.log. MOLY_CONTROL_PORT is set from rpcPort and MOLY_SETTINGS_FILE to logs/<id>.settings.json unless env overrides them. Without MOLY_DEV=1 in env the program runs the game; with it, the developer window.',
    inputSchema: {
      exe: z.string().describe('Path to the executable file'),
      env: z.record(z.string(), z.string()).optional().describe('Environment variables merged over process.env'),
      args: z.array(z.string()).optional().describe('Command-line arguments'),
      cwd: z.string().optional().describe('Working directory; must exist'),
      rpcPort: z.number().int().min(1).max(65535).optional().describe('JSON-RPC port, default 15702'),
    },
  },
  guard('native_launch', async (args) => json(await nativeLaunch(args))),
);

server.registerTool(
  'native_stop',
  {
    description: 'Kill a native instance and its process tree (taskkill /T /F on Windows) and mark it stopped.',
    inputSchema: { id: z.string() },
  },
  guard('native_stop', async ({ id }) => json(await nativeStopTool(id))),
);

server.registerTool(
  'instance_logs',
  {
    description: 'Return the last lines of an instance log, optionally only lines containing a substring.',
    inputSchema: {
      id: z.string(),
      tail: z.number().int().min(0).optional().describe('Number of trailing lines, default 200; 0 means all'),
      grep: z.string().optional().describe('Keep only lines containing this substring'),
    },
  },
  guard('instance_logs', async (args) => instanceLogs(args)),
);

server.registerTool(
  'rpc',
  {
    description:
      'Send one JSON-RPC 2.0 request to a native or browser instance. Returns the result, or the error object with isError set. 10 s timeout.',
    inputSchema: {
      id: z.string(),
      method: z.string(),
      params: z.any().optional(),
    },
  },
  guard('rpc', async ({ id, method, params }) => {
    const res = await call(getInstance(id), method, params);
    return res.error !== undefined ? json(res.error, true) : json(res.result);
  }),
);

server.registerTool(
  'browser_launch',
  {
    description:
      'Start Chrome with a fresh profile and remote debugging, open url, and tap #boot-enter when the boot screen is ready. Returns {id, pid, port, state}; state "running" means the boot screen was dismissed, "no-boot" means the page has no boot screen. Console output goes to logs/<id>.log.',
    inputSchema: {
      url: z.string().describe('Page to open'),
      width: z.number().int().min(1).optional().describe('Window width, default 1280'),
      height: z.number().int().min(1).optional().describe('Window height, default 720'),
      port: z.number().int().min(1).max(65535).optional().describe('Remote debugging port; picked free if omitted'),
      chrome: z.string().optional().describe(`Chrome executable (a file path, or a command name found on PATH), default ${DEFAULT_CHROME}`),
      timeout: z.number().positive().optional().describe('Seconds to wait for the boot screen, default 600'),
    },
  },
  guard('browser_launch', async (args) => json(await browserLaunch(args))),
);

server.registerTool(
  'browser_screenshot',
  {
    description: 'Capture the current page of a browser instance as a PNG image.',
    inputSchema: { id: z.string() },
  },
  guard('browser_screenshot', async ({ id }) => browserScreenshot({ id })),
);

server.registerTool(
  'browser_stop',
  {
    description: 'Close the DevTools connection and kill the Chrome process this server started (taskkill /T /F on Windows).',
    inputSchema: { id: z.string() },
  },
  guard('browser_stop', async ({ id }) => json(await browserStopTool(id))),
);

server.registerTool(
  'methods',
  {
    description: 'List the methods of a game instance: rpc.discover and moly.methods.',
    inputSchema: { id: z.string() },
  },
  guard('methods', async ({ id }) => {
    const inst = getInstance(id);
    const discover = await callOk(inst, 'rpc.discover');
    const moly = await callOk(inst, 'moly.methods');
    return json({ discover, moly });
  }),
);

server.registerTool(
  'act',
  {
    description:
      'Run one director step on a game instance and wait for it to finish. Returns {status, error, log} with the last 20 log entries. Status failed or a timeout sets isError.',
    inputSchema: {
      id: z.string(),
      step: z.any().describe(
        'One director step (director_schema names every field). Steps: wait, wait_until, assert, log, where, capture, parallel, repeat; ' +
          'player.walk_to, player.interact, player.gather, player.craft, player.talk_to, player.deliver (the delivery button of a birthday party in session, pressed and released), ' +
          'player.stick, player.release, player.wear, player.wear_avatar, tap, ui.tap, dialog.answer, key.back, ' +
          'layout.place (one owned fixture through the editor commands layout_place sends: enter, select_catalog, rotate, move_to, decide, save); ' +
          'npc.take, npc.walk_to, npc.release, npc.face, npc.emote, npc.use, npc.tweet, npc.talk; ' +
          'camera.drag, camera.zoom, server.edit, weather.set, site.switch, site.enter_room, site.go_home.',
      ),
      timeout: z.number().positive().optional().describe('Seconds to wait, default 60'),
    },
  },
  guard('act', async ({ id, step, timeout = 60 }) => {
    const out = await runCommand(getInstance(id), { name: 'act', steps: [step] }, { wait: true, timeoutS: timeout });
    return json(out, out.status === 'failed' || out.timedOut === true);
  }),
);

server.registerTool(
  'run_script',
  {
    description:
      'Run a whole director script ({name, exit?, steps}). With wait true (default) it polls until done, failed or stopped and returns {status, error, log}; with wait false it returns the first snapshot.',
    inputSchema: {
      id: z.string(),
      script: z.any().describe('Director script: {name, exit?, steps: [...]}'),
      wait: z.boolean().optional().describe('Poll until the run is terminal, default true'),
      timeout: z.number().positive().optional().describe('Seconds to wait, default 300'),
    },
  },
  guard('run_script', async ({ id, script, wait = true, timeout = 300 }) => {
    const out = await runCommand(getInstance(id), script, { wait, timeoutS: timeout });
    return json(out, out.status === 'failed' || out.timedOut === true);
  }),
);

server.registerTool(
  'check',
  {
    description: 'Evaluate one director condition on a game instance. Returns {holds, read}.',
    inputSchema: {
      id: z.string(),
      cond: z.any().describe('Condition (see director_schema)'),
    },
  },
  guard('check', async ({ id, cond }) => json(await callOk(getInstance(id), 'moly.director.check', { cond }))),
);

server.registerTool(
  'capture',
  {
    description: 'Capture the game frame as a PNG image. Polls until the image is ready; an error answer from the game is a tool error.',
    inputSchema: {
      id: z.string(),
      timeout: z.number().positive().optional().describe('Seconds to wait, default 10'),
    },
  },
  guard('capture', async ({ id, timeout = 10 }) => {
    const inst = getInstance(id);
    const { id: captureId } = (await callOk(inst, 'moly.capture')) ?? {};
    if (captureId === undefined) throw new Error('moly.capture returned no id');
    const deadline = Date.now() + timeout * 1000;
    for (;;) {
      const take = await callOk(inst, 'moly.capture.take', { id: captureId });
      if (take === null || take === undefined) {
        if (Date.now() >= deadline) throw new Error(`capture ${captureId}: no image within ${timeout} s`);
        await sleep(CAPTURE_POLL_MS);
        continue;
      }
      if (take.error !== undefined) {
        const detail = typeof take.error === 'string' ? take.error : take.error.message ?? JSON.stringify(take.error);
        throw new Error(`capture ${captureId}: ${detail}`);
      }
      if (typeof take.png !== 'string') throw new Error(`capture ${captureId}: reply has no png`);
      return {
        content: [
          { type: 'image', data: take.png, mimeType: 'image/png' },
          { type: 'text', text: JSON.stringify({ id: captureId, width: take.width, height: take.height }) },
        ],
      };
    }
  }),
);

server.registerTool(
  'director_schema',
  {
    description: 'Return the JSON Schema of director steps and conditions (keys: common, steps, conditions, definitions).',
    inputSchema: { id: z.string() },
  },
  guard('director_schema', async ({ id }) => json(await callOk(getInstance(id), 'moly.director.schema'))),
);

server.registerTool(
  'time',
  {
    description:
      'Read or set the game clock: scale (1 is normal speed), paused, step (run that many frames, then pause; the tool waits until they have run) and fixed_dt (seconds each frame advances, null for real time). Returns {elapsed, scale, paused, stepping, fixed_dt, frame, seed, draws}; seed is the world seed the instance was started with (MOLY_SEED), or null.',
    inputSchema: {
      id: z.string(),
      scale: z.number().min(0).optional(),
      paused: z.boolean().optional(),
      step: z.number().int().min(1).optional().describe('Frames to run before pausing again'),
      fixed_dt: z.number().positive().max(1).nullable().optional().describe('Seconds per frame; null for real time'),
      timeout: z.number().positive().optional().describe('Seconds to wait for a step, default 60'),
    },
  },
  guard('time', async ({ id, timeout = 60, ...params }) => {
    const inst = getInstance(id);
    const request = Object.fromEntries(Object.entries(params).filter(([, value]) => value !== undefined));
    let clock = await callOk(inst, 'moly.time', request);
    const deadline = Date.now() + timeout * 1000;
    while (request.step !== undefined && clock.stepping > 0) {
      if (Date.now() >= deadline) return json({ ...clock, timedOut: true }, true);
      await sleep(STEP_POLL_MS);
      clock = await callOk(inst, 'moly.time');
    }
    return json(clock);
  }),
);

server.registerTool(
  'events',
  {
    description:
      'Read the event stream: what happened after the event numbered since (default 0), oldest first. Kinds: item_obtained, craft_done, craft_refused, delivery_started, delivery_refused, delivery_done, character_arrived, character_left, weather_changed, site_entered, talk_started, talk_ended, tweet_shown. Each event has seq, frame, time, kind and its fields. Returns {events, next, missed, latest}; pass next as since on the following read. missed counts events dropped before they were read. With wait (seconds) the tool waits until at least one event arrives.',
    inputSchema: {
      id: z.string(),
      since: z.number().int().min(0).optional(),
      limit: z.number().int().min(1).max(1000).optional().describe('Most events to return, default 200'),
      kinds: z.array(z.string()).optional().describe('Only these kinds'),
      wait: z.number().positive().optional().describe('Seconds to wait for an event, default 0'),
    },
  },
  guard('events', async ({ id, wait = 0, ...params }) => {
    const inst = getInstance(id);
    const request = Object.fromEntries(Object.entries(params).filter(([, value]) => value !== undefined));
    const deadline = Date.now() + wait * 1000;
    for (;;) {
      const read = await callOk(inst, 'moly.events', request);
      if (read.events.length > 0 || Date.now() >= deadline) return json(read);
      request.since = read.next;
      await sleep(EVENT_POLL_MS);
    }
  }),
);

server.registerTool(
  'observe',
  {
    description:
      'Read the world as structured data: the moly.observe sections named in what (default all): site (tiles, grid size and occupied cells), gatherables, bag (materials, items and limits), tools, recipes (blueprints with costs and whether they can be crafted now), fixtures (owned and placed), characters (position, site, action, objective), weather, time.',
    inputSchema: {
      id: z.string(),
      what: z.array(z.string()).optional().describe('Sections to read, default all'),
    },
  },
  guard('observe', async ({ id, what }) => json(await callOk(getInstance(id), 'moly.observe', what ? { what } : {}))),
);

const tile = z.array(z.number().int().min(-128).max(127)).length(3);

server.registerTool(
  'layout_view',
  {
    description:
      'Read the layout editor: {revision, active, phase, site_id, site_type, dirty, selected, placed, inventory, catalog, can_save, feedback, and the exit, clean-up and save dialogs}. Placed and inventory rows carry uid, fixture_id, texture_id, center [x,y,z] in tiles, direction, grid_size and editable; the catalog lists owned fixtures with index and placeable copies. Without catalog true the catalog is replaced by catalog_rows.',
    inputSchema: {
      id: z.string(),
      catalog: z.boolean().optional().describe('Include the owned catalog rows, default false'),
    },
  },
  guard('layout_view', async ({ id, catalog = false }) => json(layoutOut(await readLayout(getInstance(id)), catalog))),
);

server.registerTool(
  'layout_command',
  {
    description:
      'Send one layout editor command and return the editor view after the frame that ran it. Commands: enter, select_catalog {index}, select_placed {uid}, select_inventory {uid}, move_to {center} (the touched tile of a drag; the center an even-sized fixture reports can differ from it by a tile, and y follows the column height), nudge {x, z}, rotate, decide, cancel, store, save, save_and_exit, request_exit, keep_editing, discard_and_exit, rotate_camera, change_look, request_clean_up, clean_up_all, cancel_clean_up. A command the editor refuses answers through feedback and the selection put_status.',
    inputSchema: {
      id: z.string(),
      command: z.string(),
      index: z.number().int().nonnegative().optional().describe('select_catalog: catalog row index'),
      uid: z.string().optional().describe('select_placed / select_inventory: fixture uid'),
      center: tile.optional().describe('move_to: target tile [x, y, z]'),
      x: z.number().int().min(-128).max(127).optional().describe('nudge: tiles along x (nudge needs x and z)'),
      z: z.number().int().min(-128).max(127).optional().describe('nudge: tiles along z (nudge needs x and z)'),
      catalog: z.boolean().optional().describe('Include the owned catalog rows in the answer, default false'),
    },
  },
  guard('layout_command', async ({ id, catalog = false, ...params }) => {
    const command = Object.fromEntries(Object.entries(params).filter(([, value]) => value !== undefined));
    return json(layoutOut(await layoutCommand(getInstance(id), command), catalog));
  }),
);

server.registerTool(
  'layout_place',
  {
    description:
      'Place one owned fixture: opens the editor when it is closed, selects the first catalog row of the fixture (and texture) with a placeable copy, turns it a quarter turn rotate times, moves it until its reported center is on the x and z of center (y follows the column height), and decides it when its put status is ok. Returns {placed, uid, center, direction, put_status, reason?, row?, view}. A refused put leaves the fixture selected and sets isError; cancel it or move it with layout_command. Saving stays a separate save or save_and_exit command. The director step layout.place sends the same sequence from inside the game, and saves when its save field is set.',
    inputSchema: {
      id: z.string(),
      fixture_id: z.number().int(),
      texture_id: z.number().int().optional().describe('Catalog texture to pick, default the first placeable row'),
      center: tile.describe('Wanted center [x, y, z] in the frame of the placed rows; y is ignored'),
      rotate: z.number().int().min(0).max(3).optional().describe('Quarter turns before the move, default 0'),
      decide: z.boolean().optional().describe('Decide the placement when its put status is ok, default true'),
    },
  },
  guard('layout_place', async ({ id, ...params }) => {
    const out = await layoutPlace(getInstance(id), params);
    return json(out, out.placed !== true && params.decide !== false);
  }),
);

server.registerTool(
  'layout_place_many',
  {
    description:
      'Place many owned fixtures in one call: runs the layout_place sequence for each placement (same fields: fixture_id, texture_id?, center, rotate?, decide?) and records each result. A refused or failed placement is cancelled and the batch continues; the answer is {placed, failed, results} with the per-item uid, center, direction and put_status. save_every saves every N placements (0, the default, leaves saving to the caller).',
    inputSchema: {
      id: z.string(),
      placements: z
        .array(
          z.object({
            fixture_id: z.number().int(),
            texture_id: z.number().int().optional(),
            center: tile,
            rotate: z.number().int().min(0).max(3).optional(),
            decide: z.boolean().optional(),
          }),
        )
        .min(1),
      save_every: z
        .number()
        .int()
        .min(0)
        .optional()
        .describe('Save every N placements; 0 (default) leaves saving to the caller'),
    },
  },
  guard('layout_place_many', async ({ id, placements, save_every = 0 }) => {
    const inst = getInstance(id);
    const results = [];
    let placed = 0;
    let failed = 0;
    for (let index = 0; index < placements.length; index++) {
      const item = placements[index];
      try {
        const out = await layoutPlace(inst, item);
        const ok = out.placed === true;
        if (ok) placed += 1;
        else failed += 1;
        results.push({
          index,
          ...item,
          ok,
          uid: out.uid,
          center: out.center,
          direction: out.direction,
          put_status: out.put_status,
          ...(out.reason === undefined ? {} : { reason: out.reason }),
        });
      } catch (err) {
        failed += 1;
        results.push({ index, ...item, ok: false, error: err.message });
        // A refused put can leave its fixture selected; clear it so the next
        // placement starts from the browsing phase.
        try {
          await layoutCommand(inst, { command: 'cancel' });
        } catch {
          /* the selection may already be gone */
        }
      }
      if (save_every > 0 && (index + 1) % save_every === 0) {
        try {
          await layoutCommand(inst, { command: 'save' });
        } catch {
          /* a save confirmation is the caller's to answer */
        }
      }
    }
    return json({ placed, failed, results }, failed > 0);
  }),
);

// Kill every child this server still owns when it exits, native or browser.
// Synchronous on purpose: this runs from the 'exit' event.
function killOwnedChildrenSync() {
  for (const inst of instances.values()) {
    if (inst.exited || !inst.child) continue;
    if (process.platform === 'win32') {
      spawnSync('taskkill', ['/PID', String(inst.pid), '/T', '/F'], { stdio: 'ignore' });
    } else {
      inst.child.kill('SIGKILL');
    }
  }
}
process.on('exit', killOwnedChildrenSync);
process.on('SIGINT', () => process.exit(0));
process.on('SIGTERM', () => process.exit(0));
// The client closing stdin means the session is over.
process.stdin.on('end', () => process.exit(0));

await server.connect(new StdioServerTransport());
log(`ready; logs in ${LOG_DIR}`);
