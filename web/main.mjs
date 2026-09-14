import { selectRenderer, holdSettingsWriter } from "./boot.mjs";

const panel = document.querySelector("#boot-panel");
const button = document.querySelector("#boot-start");
const retry = document.querySelector("#boot-retry");
const fallback = document.querySelector("#boot-webgl");
const status = document.querySelector("#boot-status");
let phase = "loading";
let backend;
let wasm;
let writer;
let startupTimer;

function releaseWriter() {
  if (!writer) return;
  const lease = writer;
  try {
    wasm.set_storage_writable(false);
    lease.release();
    writer = undefined;
  }
  catch (error) { console.error(error); }
}

function fail(error) {
  console.error(error);
  // A Rust panic reports its cause before the resulting generic wasm trap.
  if (phase === "failed") return;
  clearTimeout(startupTimer);
  phase = "failed";
  releaseWriter();
  panel.hidden = false;
  button.disabled = true;
  retry.hidden = false;
  fallback.hidden = backend === "webgl2";
  const reason = String(error);
  status.textContent = /Unable to find a GPU|did not provide an adapter|adapter request/i.test(reason)
    ? "无法取得可用的图形设备。请重新加载，或切换到 WebGL2 后重试。"
    : `游戏启动或运行失败：${reason}`;
}

retry.addEventListener("click", () => location.reload());
fallback.addEventListener("click", () => {
  const url = new URL(location.href);
  url.searchParams.set("renderer", "webgl2");
  location.replace(url.href);
});
window.addEventListener("moly-ready", () => {
  if (phase !== "starting") return;
  clearTimeout(startupTimer);
  phase = "running";
  panel.hidden = true;
});
window.addEventListener("moly-error", event => fail(event.detail ?? "Renderer initialization failed"));
window.addEventListener("error", event => fail(event.error ?? event.message));
window.addEventListener("unhandledrejection", event => fail(event.reason));
window.addEventListener("pagehide", releaseWriter);
window.addEventListener("pageshow", event => {
  // A restored event loop must not resume with a released storage lease.
  if (event.persisted) location.reload();
});

button.addEventListener("click", (event) => {
  if (!event.isTrusted || button.disabled || phase !== "ready") return;
  event.stopPropagation();
  phase = "starting";
  button.disabled = true;
  status.textContent = `正在启动 ${backend}…`;
  startupTimer = setTimeout(() => fail("渲染器启动超时，请重新加载或尝试 WebGL2。"), 45_000);
  try {
    // Keep this synchronous: DefaultPlugins creates/resumes audio in this gesture.
    // A failed renderer is retried by navigation, never by a second App here.
    wasm.start(backend, writer.writable);
  } catch (error) {
    fail(error);
  }
});

try {
  phase = "preflight";
  status.textContent = "正在检查图形支持并加载游戏…";
  const requested = new URL(location.href).searchParams.get("renderer") ?? "auto";
  const renderer = await selectRenderer(requested, { navigator, document });
  backend = renderer.backend;
  wasm = await import(`./pkg/${backend}/moly-app.js`);
  await wasm.default();
  writer = await holdSettingsWriter(navigator.locks);
  phase = "ready";
  status.textContent = `点击开始，启用游戏声音（${backend}）` +
    (renderer.notice ? "\nWebGPU 暂不可用，将使用 WebGL2。" : "") +
    (writer.writable ? "" : "\n本页只读：无法取得独占存档权限。关闭其他 moly 页面后重新加载即可重试。");
  button.disabled = false;
} catch (error) {
  fail(error);
}
