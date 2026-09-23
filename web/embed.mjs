// Framework-independent mount for a same-origin Moly feature page.
// The iframe owns one complete Bevy app: removing it also releases audio,
// rendering, input listeners and the browser's document storage lease.
import { applyPackSelection } from "./asset-pack-client.mjs";
import { resourceBase, resourceDirectory, resourceOrigin } from "./embed-contract.mjs";
import { mountStage, ACTIVE_MOUNT } from "./embed-stage.mjs";

export function mountMoly(container, options = {}) {
  if (options.view === "stage") return mountStage(container, options);
  const {
    src,
    assets,
    theme = "light",
    content,
    tab,
    fixture,
    onStatus,
    onSelection,
  } = options;
  if (!(container instanceof HTMLElement))
    throw new TypeError("Moly needs a container element");
  if (window[ACTIVE_MOUNT])
    throw new Error("A Moly renderer is already mounted in this page");
  const url = new URL(src, location.href);
  if (
    url.origin !== location.origin ||
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.hash
  )
    throw new Error("Moly must be served from the host origin");
  url.searchParams.set("embed", "1");
  // The asset base reaches the shell through the same contract the stage uses
  // instead of being forwarded verbatim. A value the shell would have rejected
  // used to fall back to /assets/ inside the page; it is now refused here.
  if (assets) {
    const configuredBase = options.resourceBase === undefined
      ? undefined
      : resourceBase(options.resourceBase);
    const configuredOrigin = resourceOrigin(options.resourceOrigin ??
      (configuredBase ? new URL(configuredBase).origin : undefined));
    if (configuredBase && configuredOrigin !== new URL(configuredBase).origin)
      throw new TypeError("resourceOrigin must match resourceBase");
    const assetRoot = resourceDirectory(
      assets,
      location.href,
      configuredBase ?? configuredOrigin,
    );
    url.searchParams.set(
      "assets",
      assetRoot.origin === location.origin ? assetRoot.pathname : assetRoot.href,
    );
    if (configuredOrigin) url.searchParams.set("resource_origin", configuredOrigin);
    if (configuredBase) url.searchParams.set("resource_base", configuredBase);
  }
  applyPackSelection(url, options);
  if (content) url.searchParams.set("content", content);
  if (tab) url.searchParams.set("tab", tab);
  if (Number.isSafeInteger(fixture) && fixture > 0)
    url.searchParams.set("fixture", String(fixture));
  url.searchParams.set("theme", theme === "dark" ? "dark" : "light");
  const frame = document.createElement("iframe");
  frame.title = "MYSEKAI 对话与互动";
  frame.src = url.href;
  frame.allow = "autoplay; fullscreen";
  frame.style.cssText =
    "display:block;width:100%;height:100%;border:0;min-height:360px";
  let disposed = false;
  const send = (type, value) => {
    if (!disposed)
      frame.contentWindow?.postMessage(
        { source: "moly-host", schemaVersion: 1, type, value },
        url.origin,
      );
  };
  const receive = (event) => {
    if (
      disposed ||
      event.origin !== url.origin ||
      event.source !== frame.contentWindow
    )
      return;
    const message = event.data;
    if (message?.source !== "moly" || message.schemaVersion !== 1) return;
    if (message.type === "status") onStatus?.(message.value);
    if (message.type === "selection") onSelection?.(message.value);
  };
  window.addEventListener("message", receive);
  container.append(frame);
  const handle = {
    frame,
    setTheme(value) {
      send("theme", value === "dark" ? "dark" : "light");
    },
    stop() {
      send("stop");
    },
    dispose() {
      if (disposed) return;
      send("close");
      disposed = true;
      window.removeEventListener("message", receive);
      frame.remove();
      if (window[ACTIVE_MOUNT] === handle) delete window[ACTIVE_MOUNT];
    },
  };
  window[ACTIVE_MOUNT] = handle;
  return handle;
}
