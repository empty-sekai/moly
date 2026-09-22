import { warmBaseResources } from "./base-resources.mjs";
import { preflightCoordinates } from "./coordinate-contract.mjs";
import {
  EMBED_VERSION,
  isEnvelope,
  locale as validateLocale,
  theme as validateTheme,
  filters,
  intent,
  resourceDirectory,
  resourceOrigin,
} from "./embed-contract.mjs";
import { selectRenderer } from "./boot.mjs";
import { createStageController } from "./stage-controller.mjs";
import { stageMessages } from "./stage-locale.mjs";
import { presentWeather } from "./weather-presentation.mjs";
import { createWeatherArtwork } from "./weather-artwork.mjs";
import { createWeatherPicker } from "./weather-picker.mjs";
import { weatherPhaseLabel } from "./weather-ui-locale.mjs";

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
// the selector admits only IDs the runtime itself listed, so the stage never
// invents an ID the catalogue does not contain.
let weather = null;
let weatherArtwork = null;
const weatherPicker = createWeatherPicker({
  dialog: $("weather-dialog"),
  trigger: $("stage-weather"),
  onSelect: (id) => {
    if (weather?.options.some((option) => option.id === id))
      controller?.dispatch("weather", id);
  },
  onFocus: (captured) => controller?.focus(captured),
});
const weatherIconUrls = new Map();
const weatherIconRequests = new Set();
function weatherPresentationOptions() {
  return {
    locale: ui.locale,
    region,
    assets: params.get("assets"),
    baseUrl: location.href,
    packed: params.get("packs") === "1" || Boolean(params.get("asset_catalog")),
    iconUrls: weatherIconUrls,
  };
}
function loadWeatherIcons(value) {
  if (!weatherArtwork) return;
  for (const option of value.options) {
    if (!option.icon || weatherIconRequests.has(option.icon)) continue;
    weatherIconRequests.add(option.icon);
    weatherArtwork
      .resolve(option.icon, option.iconSource)
      .catch((error) => {
        console.warn("[moly-weather:thumbnail-unavailable]", option.id, error);
        return null;
      })
      .then((url) => {
        weatherIconUrls.set(option.icon, url);
        renderWeather();
        const snapshot = controller?.getSnapshot();
        if (snapshot) send("snapshot", decorateWeather(snapshot));
      });
  }
}
window.addEventListener("pagehide", () => weatherArtwork?.dispose());
let phase = "idle",
  loading = false,
  started = false,
  failed = false,
  timer = null;
let engineBytes = 0,
  transferredBytes = 0,
  decodedBytes = 0,
  baseCompleted = 0,
  baseTotal = 0,
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
  if (next !== phase && (next === "base" || next === "resources"))
    lastProgress = performance.now();
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
  const stalled = (phase === "base" || phase === "resources")
    && performance.now() - lastProgress > 30000;
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
        base: t.base,
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
  $("boot-recovery").hidden = !failed && !stalled;
  $("boot-note").textContent = stalled ? t.stalled : "";
  const amount = engineBytes || decodedBytes;
  $("boot-progress").textContent = failed ? "" : phase === "base" && baseTotal
    ? `${t.baseProgress} ${baseCompleted}/${baseTotal}`
    : amount
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
  const shown = presentWeather(weather, weatherPresentationOptions());
  $("weather-label").textContent = shown.label;
  const icon = $("weather-icon");
  const source = shown.iconUrl;
  if (source && icon.dataset.source !== source) {
    icon.dataset.source = source;
    icon.hidden = false;
    icon.src = source;
  } else if (!source) {
    delete icon.dataset.source;
    icon.removeAttribute("src");
    icon.hidden = true;
  }
  button.hidden = false;
  button.dataset.weatherId = String(shown.committedId ?? "");
  button.dataset.requestedId = String(shown.transition?.requestedId ?? "");
  button.dataset.phase = shown.transition?.phase ?? "ready";
  button.title = weatherPhaseLabel(shown, ui.locale);
  weatherPicker.update(shown, ui.locale);
}
// The runtime publishes the档位 in game terms (id + name). Names are localized
// here, once, so the host can render the catalogue verbatim in the reader's
// language instead of carrying its own copy of the weather table.
function decorateWeather(state) {
  if (!state?.weather) return state;
  loadWeatherIcons(state.weather);
  state.weather = presentWeather(state.weather, weatherPresentationOptions());
  return state;
}
$("weather-icon").addEventListener("error", () => {
  $("weather-icon").hidden = true;
  $("stage-weather").dataset.iconState = "unavailable";
});
$("weather-icon").addEventListener("load", () => {
  $("stage-weather").dataset.iconState = "ready";
});
$("stage-weather").addEventListener("click", () => weatherPicker.open());

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
    const publicOrigin = resourceOrigin(params.get("resource_origin"));
    resourceDirectory(params.get("assets") || "", location.href, publicOrigin);
    try {
      await preflightCoordinates({
        assets: new URL(params.get("assets"), location.href).href, region, version,
        packs: params.get("packs") === "1", assetCatalog: params.get("asset_catalog") ?? undefined,
        snapshotId: params.get("snapshot") ?? undefined, stageUrl: location.href, resourceOrigin: publicOrigin,
      }, { signal: abort.signal });
      lastChunk = performance.now();
    } catch (error) { fail("source_mismatch", error); throw error; }
    weatherArtwork = createWeatherArtwork({
      assets: params.get("assets"),
      baseUrl: location.href,
      packs: params.get("packs") === "1",
      assetCatalog: params.get("asset_catalog"),
    });
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
        onProgress({ completed, total }) {
          baseCompleted = completed;
          baseTotal = total;
          lastProgress = performance.now();
          if (phase === "base") render();
        },
      },
    );
    // The original promise remains rejectable at the join below.
    void basePreparation.catch(() => {});
    const renderer = await selectRenderer(requested, { navigator, document });
    backend = renderer.backend;
    mark("backendSelected");
    const localPath = new URL(`./pkg/${backend}/moly-app.js`, import.meta.url);
    if (publicOrigin && !/^\/moly\/releases\/[a-z0-9][a-z0-9._-]{0,95}\/pkg\/(?:webgpu|webgl2)\/moly-app\.js$/.test(localPath.pathname))
      throw new Error("Invalid immutable engine path");
    const path = publicOrigin ? new URL(localPath.pathname, publicOrigin) : localPath;
    const module = await import(path.href);
    if (typeof module.start_stage !== "function")
      throw new Error("Runtime does not implement the stage contract");
    const response = await fetch(new URL("./moly-app_bg.wasm", path), {
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
    report("base");
    const progressTimer = setInterval(() => {
      if (phase === "base") render();
    }, 2000);
    try {
      await basePreparation;
    } finally {
      clearInterval(progressTimer);
    }
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
