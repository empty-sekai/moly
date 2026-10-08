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

export const UI_FONT_MAX_BYTES = 64 * 1024 * 1024;
const UI_FONT_PATH = /^fonts\/([a-f0-9]{64})\.ttf$/;

/**
 * The UI font an engine's build record names, read from beside the engine
 * module at `engineUrl` (`pkg/<backend>/moly-app.js` -> `pkg/<path>`) and
 * checked against its SHA-256. Resolves to null for an engine whose record
 * names none: such an engine carries its font itself.
 */
export async function fetchUiFont(
  engineUrl,
  { fetchImpl = (...args) => fetch(...args), signal, progress = () => {} } = {},
) {
  const options = { signal, credentials: "omit", redirect: "error" };
  const recordResponse = await fetchImpl(
    new URL("./build.json", engineUrl),
    options,
  );
  if (!recordResponse.ok)
    throw new Error(
      `Engine build record unavailable (${recordResponse.status})`,
    );
  const font = (await recordResponse.json())?.uiFont;
  if (font === undefined) return null;
  if (
    !font ||
    typeof font !== "object" ||
    Array.isArray(font) ||
    UI_FONT_PATH.exec(font?.path ?? "")?.[1] !== font.sha256 ||
    !Number.isSafeInteger(font.bytes) ||
    font.bytes <= 0 ||
    font.bytes > UI_FONT_MAX_BYTES
  )
    throw new Error("Invalid UI font record");
  const response = await fetchImpl(
    new URL(`../${font.path}`, engineUrl),
    options,
  );
  if (!response.ok || !response.body)
    throw new Error(`UI font unavailable (${response.status})`);
  const bytes = new Uint8Array(font.bytes),
    reader = response.body.getReader();
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      progress();
      if (done) break;
      if (length + value.byteLength > font.bytes)
        throw new Error("UI font exceeds its recorded length");
      bytes.set(value, length);
      length += value.byteLength;
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  if (length !== font.bytes) throw new Error("UI font length mismatch");
  const digest = [
    ...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
  ]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  if (digest !== font.sha256) throw new Error("UI font checksum mismatch");
  return bytes;
}

/** Hands an initialized engine its UI font before anything starts it. */
export function installUiFont(module, font) {
  if (!font) {
    if (typeof module.install_ui_font === "function")
      throw new Error("The engine's UI font record is missing");
    return;
  }
  if (typeof module.install_ui_font !== "function")
    throw new Error("The engine cannot install its UI font");
  module.install_ui_font(font);
}

/**
 * Choose a renderer, import its engine module and stream-compile its WASM.
 *
 * `prepare({ signal, progress })` (the source preflight) runs alongside the
 * engine download and compilation; the engine is instantiated only after it
 * resolved. It may resolve to `{ ready }`, a promise (base resources) that is
 * awaited only after the engine is initialized. A download that delivers no
 * chunk, while the preflight makes no progress either, for `stallMs` is
 * aborted, a body over `maxBytes` is refused, and only an `application/wasm`
 * response is compiled. The UI font its build record names is fetched
 * alongside and installed right after instantiation. Every failure aborts
 * the shared signal and rejects.
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
  compileModule = (response) => WebAssembly.compileStreaming(response),
  chooseRenderer = selectRenderer,
  stallMs = ENGINE_STALL_MS,
  maxBytes = ENGINE_MAX_BYTES,
}) {
  mark("bundleStart");
  report("downloading");
  const abort = new AbortController();
  let lastChunk = performance.now(),
    engineBytes = 0;
  const touch = () => {
    lastChunk = performance.now();
  };
  const watchdog = setInterval(() => {
    if (performance.now() - lastChunk > stallMs)
      abort.abort(new Error("Engine download made no progress for 30 seconds"));
  }, 2000);
  // The first of the two tasks to fail stops the other.
  const settle = (promise) =>
    promise.catch((error) => {
      abort.abort(error);
      throw error;
    });
  try {
    const prepared = settle(
      (async () => {
        const value = await prepare({ signal: abort.signal, progress: touch });
        mark("preflightReady");
        return value;
      })(),
    );
    const download = settle(
      (async () => {
        const renderer = await chooseRenderer(requested, environment);
        const backend = renderer.backend;
        onBackend(backend, renderer);
        mark("backendSelected");
        const path = resolve(backend);
        const font = settle(
          fetchUiFont(path, {
            fetchImpl,
            signal: abort.signal,
            progress: touch,
          }),
        );
        // Awaited below; a rejection meanwhile is not left unhandled.
        font.catch(() => {});
        const [module, response] = await Promise.all([
          importModule(path.href),
          fetchImpl(new URL("./moly-app_bg.wasm", path), {
            signal: abort.signal,
            credentials: "omit",
            redirect: "error",
          }),
        ]);
        if (exports.some((name) => typeof module[name] !== "function"))
          throw new Error(
            `Runtime does not implement the ${contract} contract`,
          );
        if (
          !response.ok ||
          !response.headers.get("content-type")?.startsWith("application/wasm")
        )
          throw new Error(`Invalid WASM response ${response.status}`);
        if (!response.body)
          throw new Error("Streaming response is unavailable");
        mark("engineDownloadStart");
        const reader = response.body.getReader();
        const stream = new ReadableStream({
          async pull(sink) {
            try {
              const { done, value } = await reader.read();
              touch();
              if (done) {
                mark("engineDownloadEnd");
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
        const compiled = await compileModule(
          new Response(stream, {
            status: response.status,
            headers: response.headers,
          }),
        );
        return { module, backend, compiled, font: await font };
      })(),
    );
    const [{ ready } = {}, { module, backend, compiled, font }] =
      await Promise.all([prepared, download]);
    clearInterval(watchdog);
    report("initializing");
    await module.default({ module_or_path: compiled });
    installUiFont(module, font);
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
