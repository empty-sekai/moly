// The transport the agent scripts share: one JSON-RPC 2.0 request per call to
// a game instance's own endpoint, and the small waits built on it. A call
// resolves with {result} or {error}; it never throws, so a caller decides what
// a refusal means.
const DEFAULT_TIMEOUT_MS = 10_000;
export const TERMINAL_STATUS = new Set(['done', 'failed', 'stopped']);

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function errorText(error) {
  if (error === undefined || error === null) return '';
  return typeof error === 'string' ? error : (error.message ?? JSON.stringify(error));
}

export function rpcClient(port, { timeoutMs = DEFAULT_TIMEOUT_MS, onCall } = {}) {
  const url = `http://127.0.0.1:${port}/`;
  let seq = 0;
  return {
    port,
    url,
    async call(method, params) {
      const request = { jsonrpc: '2.0', id: ++seq, method, ...(params === undefined ? {} : { params }) };
      const started = Date.now();
      let result;
      let error;
      try {
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(request),
          signal: AbortSignal.timeout(timeoutMs),
        });
        const reply = JSON.parse(await res.text());
        if (reply.error !== undefined) error = reply.error;
        else result = reply.result ?? null;
      } catch (err) {
        const timedOut = err.name === 'TimeoutError' || err.name === 'AbortError';
        error = { message: timedOut ? `timed out after ${timeoutMs / 1000} s` : `transport: ${err.cause?.code ?? err.message}` };
      }
      const ms = Date.now() - started;
      if (onCall) {
        await onCall({ request, ...(error === undefined ? { result } : { error }), ms }, error === undefined);
      }
      return { request, result, error, ms };
    },
  };
}

// The game frame number, read from moly.state.
export async function gameFrame(client) {
  const state = await client.call('moly.state');
  if (state.error !== undefined) throw new Error(`moly.state: ${errorText(state.error)}`);
  if (typeof state.result?.frame !== 'number') throw new Error('moly.state returned no frame');
  return state.result.frame;
}

// Waits until the game has run `frames` frames past the one read.
export async function waitFrames(client, frames = 2, { timeoutMs = 10_000 } = {}) {
  const target = (await gameFrame(client)) + frames;
  const deadline = Date.now() + timeoutMs;
  while ((await gameFrame(client)) < target) {
    if (Date.now() >= deadline) throw new Error(`the game did not run ${frames} frames within ${timeoutMs / 1000} s`);
    await sleep(30);
  }
}

// Polls `read` until it answers a value; the last value read is thrown with
// `what` when the deadline passes.
export async function waitFor(read, { timeoutMs, pollMs = 200, what = 'the wait' }) {
  const deadline = Date.now() + timeoutMs;
  let last;
  for (;;) {
    last = await read();
    if (last !== undefined) return last;
    if (Date.now() >= deadline) throw new Error(`${what} within ${timeoutMs / 1000} s`);
    await sleep(pollMs);
  }
}

// Runs one director script and polls its snapshot until the run is terminal
// (or the deadline passes). Answers {status, error, log, seconds} with the
// snapshot's own wording.
export async function runScript(client, script, { timeoutMs, pollMs = 250 } = {}) {
  const started = Date.now();
  const seconds = () => Math.round((Date.now() - started) / 100) / 10;
  const queued = await client.call('moly.director.command', { run: script });
  if (queued.error !== undefined) {
    return { status: 'refused', error: errorText(queued.error), log: [], seconds: seconds() };
  }
  for (;;) {
    const snapshot = await client.call('moly.director.snapshot');
    if (snapshot.error !== undefined) {
      return { status: 'unreadable', error: errorText(snapshot.error), log: [], seconds: seconds() };
    }
    const status = snapshot.result?.status;
    if (TERMINAL_STATUS.has(status)) {
      return {
        status,
        error: snapshot.result.error ?? null,
        log: (snapshot.result.log ?? []).slice(-20),
        seconds: seconds(),
      };
    }
    if (Date.now() - started >= timeoutMs) {
      return { status: 'timeout', error: `no terminal status within ${timeoutMs / 1000} s`, log: (snapshot.result?.log ?? []).slice(-20), seconds: seconds() };
    }
    await sleep(pollMs);
  }
}

// One step of a script, as the MCP server's act tool runs it.
export async function runStep(client, step, options) {
  return runScript(client, { name: 'agent', steps: [step] }, options);
}
