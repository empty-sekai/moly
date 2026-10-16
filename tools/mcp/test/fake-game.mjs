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
    name: 'moly.state',
    description: 'Game state; here only {frame}.',
    params: {},
  },
  {
    name: 'moly.layout.view',
    description: 'The layout editor view.',
    params: {},
  },
  {
    name: 'moly.layout.command',
    description: 'Queue one layout editor command for the next frame. Returns null.',
    params: { type: 'object', required: ['command'] },
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

// A frame every 10 ms of wall time.
const startedAt = Date.now();
const frameNow = () => Math.floor((Date.now() - startedAt) / 10);

// Layout editor: a command runs in the first frame after the request. Placing
// at BLOCKED_TILE is refused.
const BLOCKED_TILE = [9, 0, 9];
const DIRECTIONS = ['Down', 'Left', 'Up', 'Right'];
const editor = {
  revision: 0,
  active: false,
  selected: null,
  placed: [],
  catalog: [
    { index: 0, package: 'fixture_a', fixture_id: 695, texture_id: 1, grid_size: [2, 1, 1], placeable: 1 },
    { index: 1, package: 'fixture_b', fixture_id: 100, texture_id: 1, grid_size: [1, 1, 1], placeable: 1 },
  ],
  feedback: '',
  queue: [],
  uidSeq: 0,
};

function putStatus(center) {
  return center.every((v, i) => v === BLOCKED_TILE[i]) ? 'Occupied' : 'ok';
}

function runLayoutCommand(cmd) {
  editor.revision += 1;
  switch (cmd.command) {
    case 'enter':
      editor.active = true;
      return;
    case 'select_catalog': {
      const row = editor.catalog[cmd.index];
      if (!row || row.placeable <= 0) {
        editor.feedback = `catalog row ${cmd.index} has no placeable copy`;
        return;
      }
      editor.uidSeq += 1;
      const item = { uid: `new-${editor.uidSeq}`, package: row.package, fixture_id: row.fixture_id, texture_id: row.texture_id, center: [0, 0, 0], direction: 'Down', grid_size: row.grid_size, layout: 'Floor', editable: true };
      editor.selected = { row: cmd.index, item, from_inventory: false, is_new: true, can_store: true, put_status: putStatus(item.center) };
      return;
    }
    case 'move_to': {
      if (!editor.selected) return;
      // The touched tile; an even width reports its center one tile lower
      // in x, and y is the column height (0 here).
      const width = editor.selected.item.grid_size[0];
      const center = [cmd.center[0] - (width % 2 === 0 ? 1 : 0), 0, cmd.center[2]];
      editor.selected.item.center = center;
      editor.selected.put_status = putStatus(center);
      return;
    }
    case 'rotate':
      if (!editor.selected) return;
      editor.selected.item.direction = DIRECTIONS[(DIRECTIONS.indexOf(editor.selected.item.direction) + 1) % 4];
      return;
    case 'decide':
      if (!editor.selected || editor.selected.put_status !== 'ok') {
        editor.feedback = 'nothing to decide';
        return;
      }
      editor.placed.push(editor.selected.item);
      editor.catalog[editor.selected.row].placeable -= 1;
      editor.selected = null;
      return;
    case 'cancel':
      editor.selected = null;
      return;
    default:
      editor.feedback = `fake editor ignores ${cmd.command}`;
  }
}

function settleLayout() {
  const now = frameNow();
  while (editor.queue.length > 0 && editor.queue[0].frame < now) runLayoutCommand(editor.queue.shift().cmd);
}

function layoutViewNow() {
  settleLayout();
  const selected = editor.selected && (({ row, ...rest }) => rest)(editor.selected);
  return {
    revision: editor.revision,
    active: editor.active,
    phase: editor.active ? 'Editing' : 'Closed',
    site_id: 1,
    site_type: 'home',
    dirty: editor.placed.length > 0,
    exit_dialog: false,
    clean_up_dialog: false,
    save_dialog: null,
    selected,
    placed: editor.placed,
    inventory: [],
    catalog: editor.catalog,
    can_save: editor.placed.length > 0,
    feedback: editor.feedback,
  };
}

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

  'moly.state': () => ({ frame: frameNow() }),

  'moly.layout.view': () => layoutViewNow(),

  'moly.layout.command': (params) => {
    if (typeof params?.command !== 'string') throw invalid('moly.layout.command: params need command');
    settleLayout();
    editor.queue.push({ frame: frameNow(), cmd: params });
    return null;
  },

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
