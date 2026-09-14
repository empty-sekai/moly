// 引导：先实例化 wasm，再由真实用户手势同步调用唯一 start()。
// 资产基址 ?assets= 仍由 Rust 侧自己读，这里不做第二套解析。
import init, { start } from "./pkg/moly-app.js";

const panel = document.querySelector("#boot-panel");
const button = document.querySelector("#boot-start");
const status = document.querySelector("#boot-status");
let started = false;

function fail(error) {
  console.error(error);
  panel.hidden = false;
  button.disabled = true;
  status.textContent = `failed to start: ${error}`;
}

button.addEventListener("click", (event) => {
  if (!event.isTrusted || button.disabled || started) return;
  // 启动手势不再传播给刚装好的游戏输入；双击也只能创建一次 event loop。
  event.stopPropagation();
  started = true;
  button.disabled = true;
  panel.hidden = true;
  try {
    // 不在这里 await init() 或任何网络操作。原 Bevy/cpal AudioContext
    // 在 start -> app -> DefaultPlugins 内同步创建和 resume，保留用户激活。
    // 没有第二个播放器，也不修改全局 AudioContext 或用户音量。
    start();
  } catch (error) {
    fail(error);
  }
});

try {
  // wasm bin main 不运行 app；init 只实例化模块，start 才创建音频输出。
  await init();
  status.textContent = "点击开始，启用游戏声音";
  button.disabled = false;
} catch (error) {
  fail(error);
}
