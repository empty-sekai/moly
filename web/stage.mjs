import { warmBaseResources } from "./base-resources.mjs";
import {
  EMBED_VERSION,
  isEnvelope,
  locale as validateLocale,
  theme as validateTheme,
  filters,
  intent,
  sameOriginDirectory,
} from "./embed-contract.mjs";
import { selectRenderer } from "./boot.mjs";
import { createStageController } from "./stage-controller.mjs";
import { stageMessages, weatherLabel } from "./stage-locale.mjs";

const params = new URLSearchParams(location.search);
const $ = (id) => document.getElementById(id);
let ui = {
  locale: validateLocale(params.get("locale") || "zh-CN"),
  theme: validateTheme(params.get("theme") || "light"),
};
let configuration = null,
  wasm = null,
  controller = null,
  backend = null;
let lastPlayerData = "";
// The last weather block the runtime published. The dial is drawn from it and
// a click advances to the next檔 the runtime itself listed, so the stage never
// invents an ID the catalogue does not contain.
let weather = null;
let phase = "idle",
  loading = false,
  started = false,
  failed = false,
  timer = null;
let engineBytes = 0,
  transferredBytes = 0,
  decodedBytes = 0,
  lastProgress = performance.now();
const timings = {},
  pending = [];
const initial = filters({
  ...(params.get("tab") ? { tab: params.get("tab") } : {}),
  ...(params.get("fixture") ? { fixture: Number(params.get("fixture")) } : {}),
});
const region = params.get("region");
const version = params.get("version");
const requested = params.get("renderer") || "auto";
const send = (type, value) => {
  if (parent !== window)
    parent.postMessage(
      { source: "moly", schemaVersion: EMBED_VERSION, type, value },
      location.origin,
    );
};
function mark(name) {
  if (timings[name] === undefined) {
    timings[name] = performance.now();
    performance.mark(`moly:${name}`);
  }
}
function report(next = phase) {
  phase = next;
  send("boot", {
    phase,
    backend,
    elapsedMs: Math.round(performance.now()),
    engineDecodedBytes: engineBytes,
    transferredBytes,
    decodedBytes,
    timings: { ...timings },
  });
  render();
}
function render() {
  const t = stageMessages(ui.locale);
  document.documentElement.lang = ui.locale;
  document.documentElement.dataset.theme = ui.theme.mode;
  if (ui.theme.accent)
    document.documentElement.style.setProperty("--accent", ui.theme.accent);
  $("boot-title").textContent = t.title;
  const text = failed
    ? phase === "source_mismatch"
      ? t.mismatch
      : phase === "unsupported"
        ? t.unsupported
        : t.failed
    : ({
        idle: t.waiting,
        downloading: t.loading,
        initializing: t.initializing,
        "awaiting-gesture": t.ready,
        renderer: t.renderer,
        resources: t.scene,
        ready: t.ready,
      }[phase] ?? t.scene);
  $("boot-message").textContent = text;
  $("stage-start").textContent = wasm ? t.enter : t.prepare;
  $("stage-start").disabled = loading || started;
  $("stage-start").dataset.molyReady = String(
    Boolean(wasm && !started && !failed),
  );
  $("stage-start").hidden = failed;
  $("stage-retry").textContent = t.retry;
  $("stage-webgl").textContent = t.fallback;
  $("stage-webgl").hidden = backend === "webgl2" || requested === "webgl2";
  $("boot-recovery").hidden = !failed;
  $("boot-note").textContent =
    phase === "resources" && performance.now() - lastProgress > 30000
      ? t.stalled
      : "";
  const amount = engineBytes || decodedBytes;
  $("boot-progress").textContent =
    amount && !failed
      ? `${t.progress} ${new Intl.NumberFormat(ui.locale, { maximumFractionDigits: 1 }).format(amount / 1e6)} MB`
      : "";
  $("stage-hint").textContent = t.controls;
  renderWeather();
}
function renderWeather() {
  const button = $("stage-weather");
  if (!weather) {
    button.hidden = true;
    return;
  }
  const current = weather.options.find((option) => option.id === weather.id);
  $("weather-label").textContent = weatherLabel(
    ui.locale,
    current?.name ?? weather.name,
  );
  button.hidden = false;
  button.dataset.weatherId = String(weather.id);
}
// The runtime publishes the档位 in game terms (id + name). Names are localized
// here, once, so the host can render the catalogue verbatim in the reader's
// language instead of carrying its own copy of the weather table.
function decorateWeather(state) {
  if (!state?.weather) return state;
  const options = state.weather.options.map((option) => ({
    ...option,
    label: weatherLabel(ui.locale, option.name),
  }));
  const current = options.find((option) => option.id === state.weather.id);
  state.weather = {
    ...state.weather,
    options,
    label: current?.label ?? weatherLabel(ui.locale, state.weather.name),
  };
  return state;
}
$("stage-weather").addEventListener("click", () => {
  if (!weather || !controller) return;
  const index = weather.options.findIndex(
    (option) => option.id === weather.id,
  );
  const next = weather.options[(index + 1) % weather.options.length];
  if (next) controller.dispatch("weather", next.id);
});
function fail(code, error) {
  if (failed) return;
  failed = true;
  loading = false;
  phase = code;
  clearInterval(timer);
  console.error(`[moly-stage:${code}]`, error);
  $("stage-boot").hidden = false;
  send("error", { code });
  report(code);
}
function reload(renderer) {
  // Reload tears down the entire previous WASM/renderer/audio realm before a
  // different backend can be constructed. Never initialize both in one realm.
  const url = new URL(location.href);
  if (renderer) url.searchParams.set("renderer", renderer);
  url.searchParams.set("preload", "1");
  location.replace(url.href);
}
$("stage-retry").addEventListener("click", () => reload());
$("stage-webgl").addEventListener("click", () => reload("webgl2"));

performance.setResourceTimingBufferSize?.(10000);
const observer = new PerformanceObserver((list) => {
  for (const entry of list.getEntries()) {
    transferredBytes += entry.transferSize || 0;
    decodedBytes += entry.decodedBodySize || 0;
    lastProgress = performance.now();
  }
});
observer.observe({ type: "resource", buffered: true });

async function loadEngine() {
  if (loading || wasm || started || failed) return;
  loading = true;
  mark("bundleStart");
  report("downloading");
  const abort = new AbortController();
  let lastChunk = performance.now();
  const watchdog = setInterval(() => {
    if (performance.now() - lastChunk > 30000)
      abort.abort(new Error("Engine download made no progress for 30 seconds"));
  }, 2000);
  try {
    if (!["cn", "jp", "tw", "en", "kr"].includes(region))
      throw new Error("An explicit supported resource region is required");
    sameOriginDirectory(params.get("assets") || "", location.href);
    const basePreparation = warmBaseResources(
      {
        region,
        version,
        assets: new URL(params.get("assets"), location.href).href,
        packs: params.get("packs") === "1",
        assetCatalog: params.get("asset_catalog") ?? undefined,
      },
      {
        signal: abort.signal,
        onProgress() {
          lastProgress = performance.now();
        },
      },
    );
    // The original promise remains rejectable at the join below.
    void basePreparation.catch(() => {});
    const renderer = await selectRenderer(requested, { navigator, document });
    backend = renderer.backend;
    mark("backendSelected");
    const path = new URL(`./pkg/${backend}/moly-app.js`, import.meta.url);
    const module = await import(path.href);
    if (typeof module.start_stage !== "function")
      throw new Error("Runtime does not implement the stage contract");
    const response = await fetch(new URL("./moly-app_bg.wasm", path), {
      signal: abort.signal,
      credentials: "same-origin",
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
          if (engineBytes > 192 * 1024 * 1024) {
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
    await basePreparation;
    mark("baseResourcesReady");
    wasm = module;
    loading = false;
    report("awaiting-gesture");
  } catch (error) {
    abort.abort();
    fail(backend ? "engine_failed" : "unsupported", error);
  } finally {
    clearInterval(watchdog);
  }
}

function enter() {
  if (!wasm || started || failed) return;
  started = true;
  mark("startClick");
  report("renderer");
  try {
    controller = createStageController({
      wasm,
      region,
      version,
      initial,
      content: params.get("content"),
      post(type, value) {
        if (type === "error" && value.code === "source_mismatch")
          fail("source_mismatch", value);
        if (type === "snapshot") decorateWeather(value);
        send(type, value);
      },
    });
    controller.configure(
      configuration || { initial, content: params.get("content") },
    );
    for (const command of pending.splice(0))
      controller.dispatch(command.type, command.value);
    // Must stay synchronous in this trusted click. No await, warm-up audio
    // context or host-created second playback bus belongs here.
    wasm.start_stage(backend);
  } catch (error) {
    fail("renderer_failed", error);
  }
}
$("stage-start").addEventListener("click", () =>
  wasm ? enter() : void loadEngine(),
);

window.addEventListener("message", (event) => {
  if (
    event.origin !== location.origin ||
    event.source !== parent ||
    parent === window ||
    !isEnvelope(event.data, "moly-host")
  )
    return;
  const { type, value } = event.data;
  try {
    if (type === "configure") {
      ui = {
        locale: validateLocale(value.locale),
        theme: validateTheme(value.theme),
      };
      if (!configuration)
        configuration = {
          initial: filters(value.initial || {}),
          content: value.content || null,
          sound: value.sound !== false,
        };
      controller?.configure(configuration);
      render();
      // A locale change relabels the weather dial; the host keeps reading the
      // same projection instead of caching a stale translation.
      const snapshot = controller?.getSnapshot();
      if (snapshot) send("snapshot", decorateWeather(snapshot));
    } else if (type === "player-data") {
      if (
        !wasm ||
        !controller?.getSnapshot()?.ready ||
        value?.region !== region ||
        !["preview", "explore", "restore", "cancel"].includes(
          value.operation,
        ) ||
        (value.json !== undefined && typeof value.json !== "string") ||
        (value.operation !== "preview" && value.json)
      )
        throw new Error("Invalid player data operation");
      const json = value.json || "";
      if (
        json.length > 32 * 1048576 ||
        new TextEncoder().encode(json).byteLength > 32 * 1048576
      )
        throw new Error("Player data is too large");
      wasm.player_data_command(value.operation, region, json);
    } else if (type === "intent") {
      const command = intent(value?.type, value?.value);
      if (controller) controller.dispatch(command.type, command.value);
      else if (command.type === "close") {
        pending.length = 0;
        send("closed", { restored: true });
      } else {
        if (pending.length >= 64) throw new Error("Intent queue is full");
        pending.push(command);
      }
    }
  } catch {
    send("error", { code: "invalid_intent" });
  }
});
window.addEventListener("moly-ready", () => {
  if (failed) return;
  mark("rendererReady");
  report("resources");
  timer = setInterval(() => {
    try {
      const playerData = wasm?.player_data_snapshot?.();
      if (playerData && playerData !== lastPlayerData) {
        lastPlayerData = playerData;
        send("player-data", JSON.parse(playerData));
      }
      const state = controller?.poll();
      if (!state) return;
      if (state.weather) {
        weather = state.weather;
        renderWeather();
      }
      if (state.ready) mark("catalogReady");
      if (state.scene?.ready) {
        mark("sceneReady");
        $("stage-boot").hidden = true;
        if (phase !== "ready") report("ready");
      }
      if (state.status.phase === "playing") mark("firstPlayback");
      if (phase !== "ready") report();
    } catch (error) {
      fail("runtime_failed", error);
    }
  }, 100);
});
window.addEventListener("moly-error", (event) =>
  fail("runtime_failed", event.detail),
);
window.addEventListener("error", (event) => {
  if (event.error) fail("runtime_failed", event.error);
});
window.addEventListener("unhandledrejection", (event) =>
  fail("runtime_failed", event.reason),
);
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    send("exit-immersive", {});
    event.preventDefault();
  }
});
window.addEventListener("blur", () => controller?.focus(true));
window.addEventListener("focus", () => controller?.focus(false));
$("app-canvas").addEventListener("pointerdown", () => {
  controller?.focus(false);
  $("app-canvas").focus({ preventScroll: true });
});
window.addEventListener("pagehide", () => {
  clearInterval(timer);
  observer.disconnect();
  controller?.dispatch("close");
});

render();
send("hello", {
  contract: EMBED_VERSION,
  instance:
    globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
});
if (params.get("preload") === "1") void loadEngine();
