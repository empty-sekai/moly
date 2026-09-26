import { selectRenderer } from "./boot.mjs";

export const ENGINE_STALL_MS = 30000;
export const ENGINE_MAX_BYTES = 192 * 1024 * 1024;
const RELEASE_ID = /^[a-z0-9][a-z0-9._-]{0,95}$/;
const escape = (value) => value.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");

/**
 * The engine module of an immutable release. Below an explicit resource base
 * it is `<base>releases/<id>/pkg/<backend>/moly-app.js`; otherwise it sits
 * next to the loading module, and a configured public origin admits only the
 * canonical `<prefix>releases/<id>/pkg/<backend>/moly-app.js` path there.
 */
export function releaseEnginePath({
  backend,
  moduleUrl,
  releaseId,
  resourceBase,
  publicOrigin,
  prefix = "/moly/",
  pageError = "Invalid immutable stage path",
}) {
  const localPath = new URL(`./pkg/${backend}/moly-app.js`, moduleUrl);
  if (resourceBase) {
    if (!releaseId || !RELEASE_ID.test(releaseId)) throw new Error(pageError);
    const path = new URL(
      `releases/${releaseId}/pkg/${backend}/moly-app.js`,
      resourceBase,
    );
    if (!path.pathname.startsWith(new URL(resourceBase).pathname + "releases/"))
      throw new Error("Invalid immutable engine path");
    return path;
  }
  if (
    publicOrigin &&
    !new RegExp(
      `^${escape(prefix)}releases\\/[a-z0-9][a-z0-9._-]{0,95}\\/pkg\\/(?:webgpu|webgl2)\\/moly-app\\.js$`,
    ).test(localPath.pathname)
  )
    throw new Error("Invalid immutable engine path");
  return publicOrigin ? new URL(localPath.pathname, publicOrigin) : localPath;
}

/**
 * Choose a renderer, import its engine module and stream-compile its WASM.
 *
 * `prepare({ signal, progress })` runs first, inside the stall watchdog; it
 * may resolve to `{ ready }`, a promise (base resources) that is awaited only
 * after the engine is initialized. A download that delivers no chunk for `stallMs` is aborted, a
 * body over `maxBytes` is refused, and only an `application/wasm` response
 * is compiled. Every failure aborts the shared signal and rejects.
 */
export async function loadEngine({
  requested = "auto",
  environment,
  prepare = async () => undefined,
  resolve,
  exports = [],
  contract,
  mark = () => {},
  report = () => {},
  onBackend = () => {},
  onBytes = () => {},
  waitTick,
  fetchImpl = (...args) => fetch(...args),
  importModule = (href) => import(href),
  chooseRenderer = selectRenderer,
  stallMs = ENGINE_STALL_MS,
  maxBytes = ENGINE_MAX_BYTES,
}) {
  mark("bundleStart");
  report("downloading");
  const abort = new AbortController();
  let lastChunk = performance.now(),
    engineBytes = 0;
  const watchdog = setInterval(() => {
    if (performance.now() - lastChunk > stallMs)
      abort.abort(new Error("Engine download made no progress for 30 seconds"));
  }, 2000);
  try {
    const { ready } =
      (await prepare({
        signal: abort.signal,
        progress() {
          lastChunk = performance.now();
        },
      })) ?? {};
    const renderer = await chooseRenderer(requested, environment);
    const backend = renderer.backend;
    onBackend(backend, renderer);
    mark("backendSelected");
    const path = resolve(backend);
    const module = await importModule(path.href);
    if (exports.some((name) => typeof module[name] !== "function"))
      throw new Error(`Runtime does not implement the ${contract} contract`);
    const response = await fetchImpl(new URL("./moly-app_bg.wasm", path), {
      signal: abort.signal,
      credentials: "omit",
      redirect: "error",
    });
    if (
      !response.ok ||
      !response.headers.get("content-type")?.startsWith("application/wasm")
    )
      throw new Error(`Invalid WASM response ${response.status}`);
    if (!response.body) throw new Error("Streaming response is unavailable");
    mark("engineDownloadStart");
    const reader = response.body.getReader();
    const stream = new ReadableStream({
      async pull(sink) {
        try {
          const { done, value } = await reader.read();
          lastChunk = performance.now();
          if (done) {
            clearInterval(watchdog);
            mark("engineDownloadEnd");
            report("initializing");
            sink.close();
            return;
          }
          engineBytes += value.byteLength;
          onBytes(engineBytes);
          if (engineBytes > maxBytes) {
            abort.abort();
            throw new Error("Engine exceeds the bounded release payload");
          }
          sink.enqueue(value);
        } catch (error) {
          sink.error(error);
        }
      },
      cancel(reason) {
        return reader.cancel(reason);
      },
    });
    await module.default({
      module_or_path: new Response(stream, {
        status: response.status,
        headers: response.headers,
      }),
    });
    mark("wasmReady");
    report("base");
    const progressTimer = waitTick ? setInterval(waitTick, 2000) : null;
    try {
      await ready;
    } finally {
      clearInterval(progressTimer);
    }
    mark("baseResourcesReady");
    return { module, backend };
  } catch (error) {
    abort.abort();
    throw error;
  } finally {
    clearInterval(watchdog);
  }
}
