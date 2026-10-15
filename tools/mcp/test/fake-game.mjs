#!/usr/bin/env node
// Minimal stand-in for a game: a JSON-RPC 2.0 HTTP endpoint on MOLY_CONTROL_PORT (default 15799).
// It implements just enough of the moly surface to exercise the native path of the game tools.
import http from 'node:http';

const port = Number(process.env.MOLY_CONTROL_PORT ?? 15799);

// A 1x1 transparent PNG.
const PNG_1X1 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

const invalid = (message) => Object.assign(new Error(message), { code: -32602 });

const MOLY_METHODS = [
  {
    name: 'moly.methods',
    description: 'List the moly methods of this instance.',
    params: {},
  },
  {
    name: 'moly.director.command',
    description: 'Start a script ({run}), or send {stop}, {pause} or {resume}. Returns null.',
    params: { type: 'object' },
  },
  {
    name: 'moly.director.snapshot',
    description: 'Status of the current director run.',
    params: {},
  },
  {
    name: 'moly.director.check',
    description: 'Evaluate one condition. Returns {holds, read}.',
    params: { type: 'object', required: ['cond'] },
  },
  {
    name: 'moly.director.schema',
    description: 'JSON Schema of steps and conditions.',
    params: {},
  },
  {
    name: 'moly.capture',
    description: 'Request a frame capture. Returns {id}.',
    params: {},
  },
  {
    name: 'moly.capture.take',
    description: 'Fetch a capture: null while pending, else {png, width, height}.',
    params: { type: 'object', required: ['id'] },
  },
];

// Current director run: a run reads "queued" once, "running" once, then "done".
let run = null;
const captures = new Map(); // capture id -> number of takes so far
let captureSeq = 0;

function snapshotNow() {
  if (!run) return { status: 'idle', script: null, elapsed: 0, current: null, owned: [], error: null, log: [] };
  run.reads += 1;
  let status;
  if (run.stopped) status = 'stopped';
  else if (run.reads === 1) status = 'queued';
  else if (run.reads === 2) status = 'running';
  else status = 'done';
  if (status !== run.last) {
    run.log.push(`${run.name}: ${status}`);
    run.last = status;
  }
  return { status, script: run.script, elapsed: run.reads * 0.25, current: null, owned: [], error: null, log: [...run.log] };
}

const handlers = {
  ping: () => 'pong',

  'rpc.discover': () => ({
    openrpc: '1.3.2',
    info: { title: 'fake game', version: '0.0.0' },
    methods: Object.keys(handlers).map((name) => ({ name })),
  }),

  'moly.methods': () => ({ methods: MOLY_METHODS }),

  'moly.director.command': (params) => {
    const p = params ?? {};
    if (p.run !== undefined) {
      run = { script: p.run, name: p.run.name ?? 'anonymous', reads: 0, log: [], stopped: false, last: null };
      return null;
    }
    if (p.stop === true) {
      if (run) run.stopped = true;
      return null;
    }
    if (p.pause === true || p.resume === true) return null;
    throw invalid('unsupported director command');
  },

  'moly.director.snapshot': () => snapshotNow(),

  'moly.director.check': (params) => {
    if (!params || !('cond' in params)) throw invalid('cond is required');
    return { holds: true, read: 'fake game: every condition holds' };
  },

  'moly.director.schema': () => ({ common: {}, steps: {}, conditions: {}, definitions: {} }),

  'moly.capture': () => {
    captureSeq += 1;
    const id = `cap-${captureSeq}`;
    captures.set(id, 0);
    return { id };
  },

  'moly.capture.take': (params) => {
    const id = params?.id;
    if (!captures.has(id)) throw invalid(`unknown capture id ${id}`);
    const takes = captures.get(id) + 1;
    captures.set(id, takes);
    if (takes === 1) return null; // pending once
    return { png: PNG_1X1, width: 1, height: 1 };
  },
};

function dispatch(msg) {
  const handler = handlers[msg.method];
  if (!handler) {
    return { jsonrpc: '2.0', id: msg.id ?? null, error: { code: -32601, message: `Method not found: ${msg.method}` } };
  }
  try {
    return { jsonrpc: '2.0', id: msg.id ?? null, result: handler(msg.params) };
  } catch (err) {
    return { jsonrpc: '2.0', id: msg.id ?? null, error: { code: err.code ?? -32603, message: err.message } };
  }
}

const server = http.createServer((req, res) => {
  let body = '';
  req.setEncoding('utf8');
  req.on('data', (chunk) => { body += chunk; });
  req.on('end', () => {
    let msg;
    try {
      msg = JSON.parse(body);
    } catch {
      return reply(res, { jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } });
    }
    return reply(res, dispatch(msg));
  });
});

function reply(res, payload) {
  res.writeHead(200, { 'content-type': 'application/json' });
  res.end(JSON.stringify(payload));
}

server.listen(port, '127.0.0.1', () => {
  console.log(`fake game ready on ${port}`);
});
