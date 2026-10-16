#!/usr/bin/env node
// Runs every (task, seed) of a task set against its own hidden game instance
// in its own run directory, and collects one results.json. Each run starts the
// executable with MOLY_SEED, the task's site, a control port of its own and a
// settings file of its own, waits for the endpoint, runs the reference agent,
// stops the instance and keeps the runlog.
//
// This host runs at most two instances at a time: --parallel above 2 is
// refused rather than quietly lowered.
//
// Usage:
//   node tools/mcp/agent/batch.mjs --exe <exe> --tasks <dir> --seeds 1,2,3 \
//     --parallel 1 --base-port 15740 --out <dir> [--asset-root <dir>] \
//     [--boot 300] [--timeout 900] [--help]
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, openSync, closeSync, readdirSync, readFileSync, statSync, writeFileSync, appendFileSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { once } from 'node:events';
import { pathToFileURL } from 'node:url';
import { rpcClient, sleep, errorText } from './rpc.mjs';
import { runAgent } from './reference-agent.mjs';

export const MOST_PARALLEL = 2;
const USAGE = `usage: node tools/mcp/agent/batch.mjs --exe <exe> --tasks <dir> --seeds 1,2,3 --parallel 1 --base-port 15740 --out <dir> [--asset-root <dir>] [--boot 300] [--timeout 900]`;

function parseArgs(argv) {
  const out = { parallel: 1, basePort: 15740, seeds: '1', boot: 300, timeout: 900 };
  for (let index = 0; index < argv.length; index += 1) {
    const name = argv[index];
    if (name === '--help') return { help: true };
    if (!name.startsWith('--')) throw new Error(`unexpected argument ${JSON.stringify(name)}\n${USAGE}`);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith('--')) throw new Error(`${name} needs a value\n${USAGE}`);
    index += 1;
    switch (name) {
      case '--exe': out.exe = value; break;
      case '--tasks': out.tasks = value; break;
      case '--seeds': out.seeds = value; break;
      case '--parallel': out.parallel = Number(value); break;
      case '--base-port': out.basePort = Number(value); break;
      case '--out': out.out = value; break;
      case '--asset-root': out.assetRoot = value; break;
      case '--boot': out.boot = Number(value); break;
      case '--timeout': out.timeout = Number(value); break;
      default: throw new Error(`unknown option ${name}\n${USAGE}`);
    }
  }
  if (!out.exe) throw new Error(`--exe is required\n${USAGE}`);
  if (!out.tasks) throw new Error(`--tasks is required\n${USAGE}`);
  if (!out.out) throw new Error(`--out is required\n${USAGE}`);
  if (!Number.isInteger(out.parallel) || out.parallel < 1) {
    throw new Error(`--parallel is a whole number of instances, 1 or more, not ${JSON.stringify(out.parallel)}`);
  }
  if (out.parallel > MOST_PARALLEL) {
    throw new Error(`--parallel ${out.parallel} is above this host's limit of ${MOST_PARALLEL}`);
  }
  if (!Number.isInteger(out.basePort) || out.basePort < 1 || out.basePort > 65535) {
    throw new Error(`--base-port is a port number, not ${JSON.stringify(out.basePort)}`);
  }
  out.seedList = out.seeds
    .split(',')
    .map((seed) => seed.trim())
    .filter((seed) => seed !== '')
    .map((seed) => {
      if (!/^\d+$/.test(seed)) throw new Error(`--seeds takes decimal whole numbers, not ${JSON.stringify(seed)}`);
      return seed;
    });
  if (out.seedList.length === 0) throw new Error('--seeds is empty');
  return out;
}

function taskFiles(directory) {
  if (!existsSync(directory) || !statSync(directory).isDirectory()) {
    throw new Error(`--tasks is not a directory: ${path.resolve(directory)}`);
  }
  const files = readdirSync(directory)
    .filter((name) => name.endsWith('.json'))
    .sort();
  if (files.length === 0) throw new Error(`no *.json task in ${path.resolve(directory)}`);
  return files.map((name) => {
    const file = path.join(directory, name);
    const task = JSON.parse(readFileSync(file, 'utf8'));
    if (typeof task.id !== 'string' || task.id === '') throw new Error(`${file}: the task needs an id`);
    return { file, task };
  });
}

function portFree(port) {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once('error', () => resolve(false));
    server.listen(port, '127.0.0.1', () => server.close(() => resolve(true)));
  });
}

// Starts one instance with its own run directory and answers its record. The
// log file is the child's own stdout and stderr, so nothing is piped here.
function startInstance({ exe, dir, port, seed, site, assetRoot }) {
  const logPath = path.join(dir, 'instance.log');
  appendFileSync(logPath, `# ${exe} port ${port} seed ${seed} site ${site ?? 'default'}\n`);
  const fd = openSync(logPath, 'a');
  let child;
  try {
    child = spawn(exe, [], {
      cwd: path.dirname(exe),
      env: {
        ...process.env,
        MOLY_CONTROL_PORT: String(port),
        MOLY_SEED: seed,
        MOLY_WINDOW_HIDDEN: '1',
        MOLY_SETTINGS_FILE: path.join(dir, 'settings.json'),
        ...(site === undefined ? {} : { MOLY_SITE: site }),
        MOLY_ASSET_ROOT: assetRoot,
      },
      stdio: ['ignore', fd, fd],
      windowsHide: true,
    });
  } finally {
    closeSync(fd); // the child holds its own duplicate of the handle
  }
  const exited = new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }));
    child.once('error', (err) => resolve({ code: null, signal: null, error: err.message }));
  });
  return { child, pid: child.pid, logPath, exited };
}

async function stopInstance(instance) {
  if (instance.child.exitCode !== null || instance.child.signalCode !== null) return;
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(instance.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    instance.child.kill('SIGKILL');
  }
  await Promise.race([instance.exited, sleep(5000)]);
}

// Waits for the instance's control endpoint.
async function waitForEndpoint(port, { bootS, log }) {
  const client = rpcClient(port, { timeoutMs: 5000 });
  const deadline = Date.now() + bootS * 1000;
  for (;;) {
    const reply = await client.call('rpc.discover');
    if (reply.error === undefined) return client;
    if (Date.now() >= deadline) {
      throw new Error(`the endpoint on port ${port} did not answer within ${bootS} s (last: ${errorText(reply.error)})`);
    }
    if (log) log(`port ${port}: ${errorText(reply.error)}`);
    await sleep(1000);
  }
}

export async function batch({ exe, tasks, seeds, parallel, basePort, out, assetRoot, bootS = 300, timeoutS = 900, log = console.log } = {}) {
  const exePath = path.resolve(exe);
  if (!existsSync(exePath) || !statSync(exePath).isFile()) throw new Error(`--exe is not a file: ${exePath}`);
  if (assetRoot === undefined || !existsSync(assetRoot) || !statSync(assetRoot).isDirectory()) {
    throw new Error('--asset-root is required when MOLY_ASSET_ROOT is not set, and must be a directory');
  }
  const files = taskFiles(tasks);
  const outDir = path.resolve(out);
  mkdirSync(outDir, { recursive: true });

  const runs = [];
  for (const { file, task } of files) {
    for (const seed of seeds) {
      runs.push({ file, task, seed, port: basePort + runs.length });
    }
  }
  log(`${runs.length} runs (${files.length} tasks x ${seeds.length} seeds), ${parallel} at a time, from port ${basePort}`);

  const results = [];
  const resultsPath = path.join(outDir, 'results.json');
  const writeResults = () => writeFileSync(resultsPath, `${JSON.stringify(results, null, 2)}\n`);

  let next = 0;
  const worker = async () => {
    for (;;) {
      const run = runs[next];
      next += 1;
      if (run === undefined) return;
      const dir = path.join(outDir, `${run.task.id}-seed${run.seed}`);
      mkdirSync(dir, { recursive: true });
      const runlog = path.join(dir, 'runlog.jsonl');
      const agentLog = path.join(dir, 'agent.log');
      const started = Date.now();
      const lines = [];
      const say = (line) => {
        const text = `[${run.task.id} seed ${run.seed} port ${run.port}] ${line}`;
        lines.push(text);
        appendFileSync(agentLog, `${text}\n`);
        log(text);
      };
      const record = { task: run.task.id, seed: run.seed, port: run.port, runlog, run_dir: dir, success: false, seconds: 0, error: null };
      let instance = null;
      try {
        if (!(await portFree(run.port))) throw new Error(`port ${run.port} is already in use`);
        say(`start ${exePath}`);
        instance = startInstance({ exe: exePath, dir, port: run.port, seed: run.seed, site: run.task.site, assetRoot });
        const client = await waitForEndpoint(run.port, { bootS, log: (line) => say(line) });
        if (client.error !== undefined) throw new Error(errorText(client.error));
        const result = await runAgent({ port: run.port, task: run.task, runlog, log: say });
        record.success = result.success;
        record.failure = result.failure;
        record.checks = result.checks;
      } catch (err) {
        record.error = err.message;
        say(`failed: ${err.message}`);
      } finally {
        if (instance) {
          await stopInstance(instance);
          say('stopped');
        }
        record.seconds = Math.round((Date.now() - started) / 100) / 10;
        results.push(record);
        writeResults();
      }
    }
  };

  await Promise.all(Array.from({ length: parallel }, worker));
  const passed = results.filter((result) => result.success).length;
  log(`${passed} of ${results.length} runs succeeded; results in ${resultsPath}`);
  return { results, resultsPath, passed, total: results.length };
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`batch: ${err.message}`);
    process.exitCode = 2;
    return;
  }
  if (args.help) {
    console.log(USAGE);
    return;
  }
  const assetRoot = args.assetRoot ?? process.env.MOLY_ASSET_ROOT;
  if (assetRoot === undefined) {
    console.error(`batch: --asset-root is required when MOLY_ASSET_ROOT is not set\n${USAGE}`);
    process.exitCode = 2;
    return;
  }
  try {
    const out = await batch({
      exe: args.exe,
      tasks: args.tasks,
      seeds: args.seedList,
      parallel: args.parallel,
      basePort: args.basePort,
      out: args.out,
      assetRoot,
      bootS: args.boot,
      timeoutS: args.timeout,
    });
    process.exitCode = out.passed === out.total ? 0 : 1;
  } catch (err) {
    console.error(`batch: ${err.message}`);
    process.exitCode = 2;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
