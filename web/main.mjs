import { selectRenderer, holdSettingsWriter } from "./boot.mjs";
import { ExperienceShell } from "./shell.mjs";
import { validAssetBase, readableError } from "./presentation.mjs";
import { installSnapshotPicker } from "./snapshots.mjs";

const url = new URL(location.href);
// The standalone bundle also works when mounted below a site's feature route.
if (!url.searchParams.has("assets")) {
  url.searchParams.set("assets", new URL("./assets/", url).pathname);
  history.replaceState(null, "", url);
}
const assetBase = url.searchParams.get("assets");
const shell = new ExperienceShell(
  validAssetBase(assetBase) ? assetBase : "/assets/",
);
installSnapshotPicker(assetBase);

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

async function boundedLoad(operation, timeout = 90_000) {
  let timer;
  try {
    return await Promise.race([
      operation(),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error("Module loading timed out")),
          timeout,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function releaseWriter() {
  if (!writer) return;
  const lease = writer;
  try {
    wasm.set_storage_writable(false);
    lease.release();
    writer = undefined;
  } catch (error) {
    console.error(error);
  }
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
  status.textContent =
    /Unable to find a GPU|did not provide an adapter|adapter request/i.test(
      reason,
    )
      ? "无法取得可用的图形设备。请重新加载，或使用兼容模式重试。"
      : readableError(error);
  shell.fail(status.textContent);
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
  shell.connect(wasm);
});
window.addEventListener("moly-error", (event) =>
  fail(event.detail ?? "Renderer initialization failed"),
);
window.addEventListener("error", (event) => fail(event.error ?? event.message));
window.addEventListener("unhandledrejection", (event) => fail(event.reason));
window.addEventListener("pagehide", releaseWriter);
window.addEventListener("pageshow", (event) => {
  // A restored event loop must not resume with a released storage lease.
  if (event.persisted) location.reload();
});

button.addEventListener("click", (event) => {
  if (!event.isTrusted || button.disabled || phase !== "ready") return;
  event.stopPropagation();
  phase = "starting";
  button.disabled = true;
  status.textContent = `正在启动 ${backend}…`;
  startupTimer = setTimeout(
    () => fail("渲染器启动超时，请重新加载或尝试 WebGL2。"),
    45_000,
  );
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
  const requested =
    new URL(location.href).searchParams.get("renderer") ?? "auto";
  if (!validAssetBase(assetBase)) throw new Error("Invalid asset base");
  const renderer = await selectRenderer(requested, { navigator, document });
  backend = renderer.backend;
  wasm = await boundedLoad(async () => {
    const module = await import(`./pkg/${backend}/moly-app.js`);
    await module.default();
    return module;
  });
  if (
    typeof wasm.library_command !== "function" ||
    typeof wasm.library_snapshot !== "function"
  ) {
    throw new Error("Browser build needs updating");
  }
  writer = await holdSettingsWriter(navigator.locks);
  phase = "ready";
  status.textContent =
    "首次进入会加载场景与声音" +
    (renderer.notice ? " · 已选择兼容模式" : "") +
    (writer.writable
      ? ""
      : "\n本页只读：无法取得独占存档权限。关闭其他 moly 页面后重新加载即可重试。");
  button.disabled = false;
} catch (error) {
  fail(error);
}
