// Framework-independent mount for a same-origin Moly feature page.
// The iframe owns one complete Bevy app: removing it also releases audio,
// rendering, input listeners and the browser's document storage lease.
export function mountMoly(
  container,
  { src, assets, theme = "light", content, tab, onStatus, onSelection } = {},
) {
  if (!(container instanceof HTMLElement))
    throw new TypeError("Moly needs a container element");
  const url = new URL(src, location.href);
  if (url.origin !== location.origin)
    throw new Error("Moly must be served from the host origin");
  url.searchParams.set("embed", "1");
  if (assets) url.searchParams.set("assets", assets);
  if (content) url.searchParams.set("content", content);
  if (tab) url.searchParams.set("tab", tab);
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
  return {
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
    },
  };
}
