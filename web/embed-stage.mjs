import { applyPackSelection } from "./asset-pack-client.mjs";
import { activatePreparedStage } from "./stage-activation.mjs";
import {
  EMBED_VERSION,
  filters,
  intent,
  isEnvelope,
  isSnapshot,
  locale as checkedLocale,
  theme as checkedTheme,
  sameOriginDirectory,
  contentKey,
  MAX_PENDING_INTENTS,
} from "./embed-contract.mjs";

export const ACTIVE_MOUNT = Symbol.for("moly.activeEmbed");
const TITLES = {
  "zh-CN": "MYSEKAI 对话与互动",
  "zh-TW": "MYSEKAI 對話與互動",
  "en-US": "MYSEKAI conversations and interactions",
  "ja-JP": "MYSEKAI 会話とインタラクション",
  "ko-KR": "MYSEKAI 대화와 상호작용",
};

// No playback state machine lives here. This is the host side of a narrow,
// versioned transport to the sole Rust owner in the iframe.
export function mountStage(container, options = {}) {
  if (!(container instanceof HTMLElement))
    throw new TypeError("Moly needs a container element");
  if (window[ACTIVE_MOUNT])
    throw new Error("A Moly renderer is already mounted in this host");
  const url = new URL(options.src ?? "./stage.html", import.meta.url);
  if (
    url.origin !== location.origin ||
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.hash
  )
    throw new Error("Moly must be served from the host origin");
  if (typeof options.assets !== "string" || !options.assets)
    throw new TypeError("An explicit source asset directory is required");
  if (
    options.snapshot !== undefined &&
    !/^[a-z0-9][a-z0-9._-]{0,95}$/.test(options.snapshot)
  )
    throw new TypeError("Invalid snapshot identity");
  if (
    options.renderer !== undefined &&
    !["auto", "webgpu", "webgl2"].includes(options.renderer)
  )
    throw new TypeError("Invalid renderer");
  const ui = {
    theme: checkedTheme(options.theme ?? "light"),
    locale: checkedLocale(options.locale),
    sound: options.sound !== false,
  };
  const initial = filters({
    ...(options.tab ? { tab: options.tab } : {}),
    ...(options.fixture ? { fixture: options.fixture } : {}),
    ...options.filters,
  });
  const selected = options.content ? contentKey(options.content) : null;
  url.searchParams.set("embed", "1");
  url.searchParams.set("theme", ui.theme.mode);
  url.searchParams.set("locale", ui.locale);
  if (options.assets)
    url.searchParams.set(
      "assets",
      sameOriginDirectory(options.assets, location.href).pathname,
    );
  applyPackSelection(url, options);
  if (options.snapshot) url.searchParams.set("snapshot", options.snapshot);
  if (!["cn", "jp", "tw", "en", "kr"].includes(options.region))
    throw new TypeError("An explicit supported resource region is required");
  if (
    typeof options.version !== "string" ||
    !/^\d+\.\d+\.\d+$/.test(options.version)
  )
    throw new TypeError("An explicit source version is required");
  url.searchParams.set("region", options.region);
  url.searchParams.set("version", options.version);
  if (options.renderer) url.searchParams.set("renderer", options.renderer);
  if (selected) url.searchParams.set("content", selected);
  if (initial.fixture) url.searchParams.set("fixture", String(initial.fixture));
  if (initial.tab) url.searchParams.set("tab", initial.tab);
  if (options.preload) url.searchParams.set("preload", "1");
  const frame = document.createElement("iframe");
  frame.title = TITLES[ui.locale];
  frame.allow = "autoplay; fullscreen";
  frame.referrerPolicy = "same-origin";
  frame.style.cssText =
    "display:block;width:100%;height:100%;border:0;min-height:240px";
  let disposed = false,
    connected = false,
    lastSnapshot = null,
    closeResolve = null,
    closeTimer,
    closePromise = null;
  let pending = [];
  let documentInstance = null,
    requestedPlay = null,
    requestedWeather = null;
  let currentFilters = { ...initial },
    currentSelection = selected;
  const owner = {};
  const post = (type, value) => {
    if (!disposed)
      frame.contentWindow?.postMessage(
        { source: "moly-host", schemaVersion: EMBED_VERSION, type, value },
        url.origin,
      );
  };
  const dispatch = (type, value) => {
    if (disposed) return;
    const command = intent(type, value);
    if (type === "browse")
      currentFilters = { ...currentFilters, ...command.value };
    if (["select", "play", "preview"].includes(type))
      currentSelection = command.value;
    if (type === "play" || type === "preview") requestedPlay = command;
    if (["stop", "restore", "close"].includes(type)) requestedPlay = null;
    if (type === "weather") requestedWeather = command.value;
    if (type === "sound") ui.sound = command.value;
    if (connected) post("intent", command);
    else {
      if (pending.length >= MAX_PENDING_INTENTS)
        throw new Error("Moly intent queue is full");
      pending.push(command);
    }
  };
  const configure = () =>
    post("configure", {
      ...ui,
      initial: currentFilters,
      content: currentSelection,
    });
  const receive = (event) => {
    if (
      disposed ||
      event.origin !== url.origin ||
      event.source !== frame.contentWindow ||
      !isEnvelope(event.data, "moly")
    )
      return;
    const { type, value } = event.data;
    if (type === "hello" && value?.contract === EMBED_VERSION) {
      const instance =
        typeof value.instance === "string" ? value.instance : "legacy";
      if (instance === documentInstance) return;
      const reloaded = documentInstance !== null;
      documentInstance = instance;
      connected = true;
      lastSnapshot = null;
      configure();
      const queuedPlay = pending.some(
        (command) => command.type === "play" || command.type === "preview",
      );
      for (const command of pending) post("intent", command);
      pending = [];
      // Retry after a download/initialization failure restores only the last
      // unacknowledged user Play intention. A previously playing scene is not
      // silently resumed after a crash, and current browsing is never rewound.
      if (reloaded && requestedPlay && !queuedPlay)
        post("intent", requestedPlay);
      // The weather dial is a scene setting, not a playback intention: an
      // engine reload must come back on the档 the user last chose instead of
      // silently rewinding to the default phenomenon.
      if (
        reloaded &&
        requestedWeather !== null &&
        !pending.some((command) => command.type === "weather")
      )
        post("intent", { type: "weather", value: requestedWeather });
    } else if (
      type === "player-data" &&
      value?.schemaVersion === 1 &&
      ["cn", "jp"].includes(value.region) &&
      typeof value.busy === "boolean" &&
      typeof value.error === "boolean" &&
      typeof value.status === "string"
    ) {
      options.onPlayerData?.(value);
      return;
    } else if (type === "snapshot" && isSnapshot(value)) {
      lastSnapshot = value;
      if (
        value.status?.phase === "playing" &&
        value.status.activeKey === requestedPlay?.value &&
        Boolean(value.status.preview) === (requestedPlay?.type === "preview")
      )
        requestedPlay = null;
      if (value.status?.phase === "error") requestedPlay = null;
      options.onSnapshot?.(value);
      options.onStatus?.(value.status);
      options.onSelection?.({
        key: value.selected?.key ?? null,
        tab: value.tab,
        region: value.region,
        fixture: value.relatedFixture,
      });
    } else if (type === "boot" && typeof value?.phase === "string")
      options.onBoot?.(value);
    else if (type === "closed" && typeof value?.restored === "boolean")
      closeResolve?.(value.restored);
    else if (type === "error" && typeof value?.code === "string")
      options.onError?.(value);
  };
  window.addEventListener("message", receive);
  window[ACTIVE_MOUNT] = owner;
  frame.src = url.href;
  container.append(frame);
  const handle = {
    frame,
    get disposed() {
      return disposed;
    },
    get snapshot() {
      return lastSnapshot;
    },
    setTheme(value) {
      ui.theme = checkedTheme(value);
      if (connected) configure();
    },
    setLocale(value) {
      ui.locale = checkedLocale(value);
      frame.title = TITLES[ui.locale];
      if (connected) configure();
    },
    playerData(value) {
      if (disposed || !connected || !lastSnapshot?.ready)
        throw new Error("The stage is not ready");
      if (
        !value ||
        value.region !== options.region ||
        !["preview", "explore", "restore", "cancel"].includes(
          value.operation,
        ) ||
        (value.json !== undefined && typeof value.json !== "string") ||
        (value.operation !== "preview" && value.json)
      )
        throw new Error("Invalid player-data operation");
      const text = value.json || "";
      if (
        text.length > 32 * 1048576 ||
        new TextEncoder().encode(text).byteLength > 32 * 1048576
      )
        throw new Error("Player data exceeds 32 MiB");
      post("player-data", {
        operation: value.operation,
        region: value.region,
        json: text,
      });
    },
    browse(value) {
      dispatch("browse", value);
    },
    select(value) {
      dispatch("select", value);
    },
    setWeather(value) {
      dispatch("weather", value);
    },
    setSoundEnabled(value) {
      dispatch("sound", !!value);
    },
    play(value) {
      dispatch("play", value);
      activatePreparedStage(frame);
    },
    preview(value) {
      dispatch("preview", value);
      activatePreparedStage(frame);
    },
    stop() {
      dispatch("stop");
    },
    restore() {
      dispatch("restore");
    },
    // In-page Close gives the real owner time to restore. A removed document
    // has no surviving world; its temporary layout must never be persisted.
    close() {
      if (closePromise) return closePromise;
      if (disposed) return Promise.resolve(true);
      // Multiple route/user close callers share one restore acknowledgment.
      // Never overwrite the first waiter or leave an orphan timeout behind.
      closePromise = (async () => {
        let restored = !lastSnapshot?.status.canStop;
        if (connected && !restored) {
          restored = await new Promise((resolve) => {
            closeResolve = resolve;
            closeTimer = setTimeout(() => resolve(false), 8000);
            dispatch("close");
          });
        }
        handle.dispose();
        return restored;
      })();
      return closePromise;
    },
    dispose() {
      if (disposed) return;
      post("intent", { type: "close" });
      disposed = true;
      clearTimeout(closeTimer);
      closeResolve?.(false);
      closeResolve = null;
      pending = [];
      window.removeEventListener("message", receive);
      frame.remove();
      if (window[ACTIVE_MOUNT] === owner) delete window[ACTIVE_MOUNT];
    },
  };
  return handle;
}
