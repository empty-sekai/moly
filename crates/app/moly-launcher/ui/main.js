"use strict";

const tauri = window.__TAURI__ ?? window.__MOLY_PREVIEW__;
const invoke = (command, args) => tauri.core.invoke(command, args);
const appWindow = tauri.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

const REGIONS = { jp: "日服", cn: "国服", tw: "台服", kr: "韩服", en: "国际服" };
const KINDS = {
  site: ["站点", "#8b7bff"],
  furniture: ["家具", "#ff7eb3"],
  character: ["角色", "#4fc3ff"],
  audio: ["音频", "#ffb347"],
  weather: ["天气", "#43c6ac"],
  common: ["通用", "#b0a8d8"],
};

const state = {
  overview: null,
  snapshots: [],
  sizes: new Map(),
  download: null,
  game: { running: false },
  speed: 0,
  fileRate: 0,
  lastSample: null,
  dismissed: null,
  testedCdn: null,
};

function regionName(region) {
  return REGIONS[region] ?? region.toUpperCase();
}

function bytes(n) {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i >= 3 ? 2 : i ? 1 : 0)} ${units[i]}`;
}

function duration(seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "—";
  if (seconds < 60) return `${Math.ceil(seconds)} 秒`;
  if (seconds < 3600) return `${Math.ceil(seconds / 60)} 分钟`;
  return `${Math.floor(seconds / 3600)} 小时 ${Math.ceil((seconds % 3600) / 60)} 分`;
}

let toastTimer = 0;
function toast(message, bad = false) {
  const el = $("toast");
  el.textContent = message;
  el.className = bad ? "toast bad" : "toast";
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), bad ? 7000 : 3500);
}

function installedFor(snapshot) {
  return state.overview?.installed.find((item) => item.id === snapshot.id) ?? null;
}

function selectedInstalled() {
  const ov = state.overview;
  if (!ov) return null;
  return ov.installed.find((item) => item.id === ov.selected) ?? null;
}

function downloading() {
  return state.download && !state.download.finished;
}

/* ---------- rendering ---------- */

function renderPlay() {
  const play = $("play");
  const title = $("play-title");
  const sub = $("play-sub");
  const icon = play.querySelector(".play-icon");
  const selected = selectedInstalled();
  play.classList.remove("download");
  icon.innerHTML = '<svg viewBox="0 0 24 24"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.4-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5Z"/></svg>';
  if (state.game.running) {
    play.disabled = true;
    title.textContent = "游戏运行中";
    sub.textContent = selected ? `${regionName(selected.region)} ${selected.version}` : "";
  } else if (downloading()) {
    play.disabled = true;
    title.textContent = "下载中…";
    sub.textContent = "完成后即可开始";
  } else if (selected) {
    play.disabled = !state.overview.game_found;
    title.textContent = "开始游戏";
    sub.textContent = state.overview.game_found
      ? `${regionName(selected.region)} ${selected.version} · ${bytes(selected.bytes)}`
      : "没有找到游戏程序，请在设置里查看";
  } else {
    const first = state.snapshots[0];
    play.classList.add("download");
    icon.innerHTML = '<svg viewBox="0 0 24 24" style="fill:none;stroke:currentColor;stroke-width:2.4"><path d="M12 4v11m0 0-4.5-4.5M12 15l4.5-4.5M5 19.5h14"/></svg>';
    play.disabled = !first;
    title.textContent = "下载资源";
    const size = first && state.sizes.get(first.id);
    sub.textContent = first
      ? `首次使用需要下载${size ? `，最多 ${bytes(size.bytes)}` : ""}`
      : "正在读取资源列表…";
  }

  const pick = $("snapshot-pick");
  const select = $("snapshot-select");
  const installed = state.overview?.installed ?? [];
  pick.hidden = installed.length < 2;
  select.replaceChildren(
    ...installed.map((item) => {
      const option = document.createElement("option");
      option.value = item.id;
      option.textContent = `${regionName(item.region)} ${item.version}`;
      option.selected = item.id === state.overview.selected;
      return option;
    }),
  );
  select.disabled = state.game.running || downloading();
}

function renderSnapshots() {
  const list = $("snapshots");
  $("cdn-line").textContent = state.overview ? state.overview.config.cdn : "—";
  if (!state.snapshots.length) {
    list.innerHTML = `<li class="placeholder">${state.snapshotsError ?? "正在读取资源列表…"}</li>`;
    return;
  }
  list.replaceChildren(
    ...state.snapshots.map((snapshot) => {
      const installed = installedFor(snapshot);
      const sameRegion = state.overview.installed.find((item) => item.region === snapshot.region && item.id !== snapshot.id);
      const size = state.sizes.get(snapshot.id);
      const li = document.createElement("li");
      li.className = "snapshot" + (installed && state.overview.selected === snapshot.id ? " selected" : "");
      const badge = document.createElement("span");
      badge.className = `badge ${snapshot.region}`;
      badge.textContent = snapshot.region.toUpperCase();
      const text = document.createElement("div");
      text.className = "snap-text";
      const status = installed
        ? '<span class="pill ok">已安装</span>'
        : sameRegion
          ? '<span class="pill">新版本</span>'
          : '<span class="pill">未下载</span>';
      text.innerHTML = `<b>${regionName(snapshot.region)} ${snapshot.version}</b><small>${status}<span>${
        size
          ? `${installed ? bytes(installed.bytes) : `最多 ${bytes(size.bytes)}`} · ${size.packages.toLocaleString()} 个包`
          : "正在计算大小…"
      }</span></small>`;
      if (size) {
        const kinds = document.createElement("div");
        kinds.className = "kinds";
        kinds.title = size.kinds.map(([kind, n]) => `${KINDS[kind]?.[0] ?? kind} ${bytes(n)}`).join("  ·  ");
        for (const [kind, n] of size.kinds) {
          const bar = document.createElement("span");
          bar.style.flex = String(n);
          bar.style.background = KINDS[kind]?.[1] ?? "#999";
          kinds.append(bar);
        }
        text.append(kinds);
      }
      const action = document.createElement("button");
      action.className = installed ? "ghost small" : "ghost small accent";
      action.textContent = installed ? "校验" : "下载";
      action.title = installed ? "逐个核对文件，缺失或损坏的重新下载" : "下载到资源目录";
      action.disabled = downloading() || state.game.running;
      action.addEventListener("click", () => startDownload(snapshot.id));
      li.append(badge, text, action);
      return li;
    }),
  );
}

const PHASES = {
  idle: "准备中",
  catalog: "读取目录",
  manifests: "读取包清单",
  blobs: "下载并校验文件",
  done: "已完成",
  failed: "失败",
  cancelled: "已取消",
  moving: "复制并校验文件",
};

function renderDownload() {
  const d = state.download;
  const card = $("download");
  const visible = Boolean(d) && !(d.finished && state.dismissed === d.snapshot + d.elapsed_ms);
  card.hidden = !visible;
  if (!visible) return;
  const moving = d.kind === "migrate";
  const snapshot = state.snapshots.find((item) => item.id === d.snapshot);
  $("download-title").textContent = moving
    ? "迁移资源"
    : snapshot ? `${regionName(snapshot.region)} ${snapshot.version}` : d.snapshot;
  // A move within one drive renames folders and has no byte count.
  const blobs = d.phase === "blobs" || d.phase === "done" || (d.phase === "moving" && d.bytes_total > 0);
  // The ring follows the current phase; the bar follows the whole download.
  const phaseFraction = d.phase === "done" ? 1 : blobs && d.bytes_total ? d.bytes_done / d.bytes_total : d.files_total ? d.files_done / d.files_total : 0;
  const overall = d.phase === "done" ? 1 : blobs && d.bytes_total ? d.bytes_done / d.bytes_total : 0;
  $("ring").style.setProperty("--p", phaseFraction.toFixed(4));
  $("ring-text").textContent = `${Math.floor(phaseFraction * 100)}%`;
  $("bar").style.width = `${(overall * 100).toFixed(2)}%`;
  const files = d.files_total ? ` · ${d.files_done.toLocaleString()} / ${d.files_total.toLocaleString()}` : "";
  const phase = d.phase === "moving" && !d.bytes_total ? "移动文件夹" : PHASES[d.phase] ?? d.phase;
  $("download-phase").textContent = `${phase}${files}`;
  $("stat-bytes-label").textContent = moving ? "已复制" : "已下载";
  $("stat-bytes").textContent = blobs ? `${bytes(d.bytes_done)} / ${bytes(d.bytes_total)}` : `${bytes(d.bytes_fetched)}`;
  $("stat-speed").textContent = d.finished
    ? "—"
    : blobs
      ? `${bytes(state.speed)}/s`
      : `${Math.round(state.fileRate)} 个/秒`;
  const left = d.finished
    ? NaN
    : blobs
      ? state.speed > 0 ? (d.bytes_total - d.bytes_done) / state.speed : NaN
      : state.fileRate > 0 ? (d.files_total - d.files_done) / state.fileRate : NaN;
  $("stat-eta").textContent = duration(left);
  $("stat-eta-label").textContent = blobs || d.finished ? "剩余" : "本阶段剩余";
  const note = $("download-note");
  note.className = d.error ? "note bad" : "note";
  note.textContent = d.error ?? (d.finished ? "" : d.note);
  const action = $("cancel");
  action.textContent = d.finished ? "关闭" : "取消";
  action.classList.toggle("danger", !d.finished);
}

function renderGame() {
  const line = $("game-status");
  const g = state.game;
  if (g.running) {
    line.hidden = false;
    line.className = "status-line";
    line.textContent = "游戏正在运行";
  } else if (g.exit) {
    line.hidden = false;
    line.className = "status-line bad";
    line.textContent = `游戏已退出：${g.exit}`;
  } else {
    line.hidden = true;
  }
}

function placeDirs() {
  const ov = state.overview;
  const game = ov.game.replace(/[\\/][^\\/]*$/, "");
  return { game, resources: ov.resource_dir, data: ov.data_dir, logs: ov.logs_dir };
}

function renderPlaces() {
  if (!state.overview) return;
  const dirs = placeDirs();
  $("place-game").textContent = state.overview.game;
  $("place-game").title = state.overview.game;
  for (const key of ["resources", "data", "logs"]) {
    $(`place-${key}`).textContent = dirs[key];
    $(`place-${key}`).title = dirs[key];
  }
}

function renderAll() {
  if (state.overview) $("version").textContent = `v${state.overview.version}`;
  renderPlaces();
  renderPlay();
  renderSnapshots();
  renderDownload();
  renderGame();
}

/* ---------- actions ---------- */

async function loadOverview() {
  state.overview = await invoke("overview");
  fillSettings();
}

async function loadSnapshots(cdn = state.overview.config.cdn) {
  state.snapshotsError = null;
  try {
    state.snapshots = await invoke("publication", { cdn });
  } catch (error) {
    state.snapshots = [];
    state.snapshotsError = `读取失败：${error}`;
  }
  renderAll();
  for (const snapshot of state.snapshots) {
    if (state.sizes.has(snapshot.id)) continue;
    try {
      state.sizes.set(snapshot.id, await invoke("snapshot_size", { snapshot: snapshot.id }));
      renderAll();
    } catch (error) {
      console.warn(error);
    }
  }
}

async function startDownload(id) {
  try {
    await invoke("start_download", { snapshot: id });
    state.speed = 0;
    state.fileRate = 0;
    state.lastSample = null;
    pollDownload();
  } catch (error) {
    toast(String(error), true);
  }
}

let downloadTimer = 0;
async function pollDownload() {
  clearTimeout(downloadTimer);
  const d = await invoke("download_status");
  state.download = d;
  if (d) {
    const now = performance.now();
    if (state.lastSample) {
      const dt = (now - state.lastSample.t) / 1000;
      const db = d.bytes_fetched - state.lastSample.b;
      const df = d.phase === state.lastSample.phase ? d.files_done - state.lastSample.f : 0;
      if (dt > 0) {
        state.speed = state.speed ? state.speed * 0.8 + (db / dt) * 0.2 : db / dt;
        state.fileRate = state.fileRate ? state.fileRate * 0.8 + (df / dt) * 0.2 : df / dt;
      }
    }
    state.lastSample = { t: now, b: d.bytes_fetched, f: d.files_done, phase: d.phase };
  }
  renderAll();
  if (d && !d.finished) {
    downloadTimer = setTimeout(pollDownload, 500);
  } else if (d) {
    await loadOverview();
    renderAll();
    const moving = d.kind === "migrate";
    if (moving && d.phase === "done") {
      state.sizes.clear();
      loadSnapshots();
    }
    if (d.phase === "done") toast(moving ? (d.note ? `资源已迁移：${d.note}` : "资源已迁移到新位置") : "资源已就绪");
    else if (d.phase === "cancelled") toast(moving ? "迁移已取消，资源仍在原来的位置" : "下载已取消，下次会从断点继续");
    else if (d.error) toast(`${moving ? "迁移" : "下载"}失败：${d.error}`, true);
  }
}

let gameTimer = 0;
async function pollGame() {
  clearTimeout(gameTimer);
  state.game = await invoke("game_status");
  renderAll();
  if (state.game.running) gameTimer = setTimeout(pollGame, 1500);
}

async function play() {
  const selected = selectedInstalled();
  if (!selected) {
    const first = state.snapshots[0];
    if (first) startDownload(first.id);
    return;
  }
  try {
    await invoke("launch", { snapshot: selected.id });
    toast("正在启动游戏…");
    state.game = { running: true };
    renderAll();
    setTimeout(pollGame, 800);
  } catch (error) {
    toast(String(error), true);
  }
}

/* ---------- settings ---------- */

const DEFAULT_CDN = "https://assets.pjsk.moe/sekai-extra-assets/";
let draftDir = null;

function fillSettings() {
  const ov = state.overview;
  $("cdn").value = ov.config.cdn;
  $("jobs").value = ov.config.jobs;
  $("jobs-value").textContent = ov.config.jobs;
  $("developer").checked = ov.config.developer;
  draftDir = ov.resource_dir;
  $("dir").textContent = ov.resource_dir;
  $("cdn-result").textContent = "";
  $("save-result").textContent = "";
  showMoveConfirm(false);
}

function openSettings(open) {
  $("drawer").classList.toggle("open", open);
  $("drawer").setAttribute("aria-hidden", String(!open));
  $("scrim").hidden = !open;
  if (open) fillSettings();
  else if (state.testedCdn && state.testedCdn !== state.overview.config.cdn) {
    state.testedCdn = null;
    loadSnapshots();
  }
}

function normalizedCdn() {
  let value = $("cdn").value.trim();
  if (value && !value.endsWith("/")) value += "/";
  return value;
}

async function testCdn() {
  const cdn = normalizedCdn();
  const result = $("cdn-result");
  result.className = "hint";
  result.textContent = "连接中…";
  try {
    const list = await invoke("publication", { cdn });
    state.testedCdn = cdn;
    result.className = "hint ok";
    result.textContent = `可用 · ${list.length} 个资源版本`;
  } catch (error) {
    result.className = "hint bad";
    result.textContent = String(error);
  }
}

function showMoveConfirm(shown) {
  $("move-confirm").hidden = !shown;
  $("save").hidden = shown;
}

function sameDir(a, b) {
  const key = (path) => String(path ?? "").replace(/[\\/]+$/, "").toLowerCase();
  return key(a) === key(b);
}

async function saveSettings(event) {
  event.preventDefault();
  const ov = state.overview;
  if (!sameDir(draftDir, ov.resource_dir) && ov.installed.length) {
    const total = ov.installed.reduce((sum, item) => sum + item.bytes, 0);
    $("move-text").textContent = `原来的位置有已下载的资源（${bytes(total)}）。要把它们迁移到 ${draftDir} 吗？`;
    showMoveConfirm(true);
    return;
  }
  await commitSettings(draftDir);
}

/** Save the settings with `resourceDir`; `move` also moves the resources there. */
async function commitSettings(resourceDir, move = false) {
  const before = state.overview.config;
  const config = {
    ...before,
    cdn: normalizedCdn(),
    jobs: Number($("jobs").value),
    developer: $("developer").checked,
    resource_dir: move ? before.resource_dir : resourceDir,
  };
  try {
    state.overview = await invoke("save_config", { config });
    if (move) {
      await invoke("start_migrate", { to: resourceDir });
      state.lastSample = null;
      state.speed = 0;
      pollDownload();
    }
    showMoveConfirm(false);
    $("save-result").className = "hint ok";
    $("save-result").textContent = "已保存";
    const reload = before.cdn !== config.cdn || before.resource_dir !== config.resource_dir;
    state.testedCdn = null;
    if (reload) {
      state.sizes.clear();
      loadSnapshots();
    }
    renderAll();
    setTimeout(() => openSettings(false), 400);
  } catch (error) {
    $("save-result").className = "hint bad";
    $("save-result").textContent = String(error);
  }
}

/* ---------- wiring ---------- */

$("minimize").addEventListener("click", () => appWindow.minimize());
$("maximize").addEventListener("click", () => appWindow.toggleMaximize());
$("close").addEventListener("click", () => appWindow.close());
$("play").addEventListener("click", play);
$("refresh").addEventListener("click", () => loadSnapshots());
$("cancel").addEventListener("click", () => {
  const d = state.download;
  if (d?.finished) {
    state.dismissed = d.snapshot + d.elapsed_ms;
    renderAll();
  } else {
    invoke("cancel_download");
  }
});
$("snapshot-select").addEventListener("change", async (event) => {
  try {
    state.overview = await invoke("save_config", { config: { ...state.overview.config, snapshot: event.target.value } });
    renderAll();
  } catch (error) {
    toast(String(error), true);
  }
});
$("open-settings").addEventListener("click", () => openSettings(true));
$("close-settings").addEventListener("click", () => openSettings(false));
$("scrim").addEventListener("click", () => openSettings(false));
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") openSettings(false);
});
$("jobs").addEventListener("input", (event) => ($("jobs-value").textContent = event.target.value));
$("cdn-default").addEventListener("click", () => ($("cdn").value = DEFAULT_CDN));
$("cdn-test").addEventListener("click", testCdn);
$("dir-pick").addEventListener("click", async () => {
  const picked = await invoke("pick_folder");
  if (picked) {
    draftDir = picked;
    $("dir").textContent = picked;
  }
});
$("dir-default").addEventListener("click", () => {
  draftDir = state.overview.default_resource_dir;
  $("dir").textContent = draftDir;
});
$("dir-open").addEventListener("click", () =>
  invoke("open_folder", { path: draftDir ?? state.overview.resource_dir }).catch((error) => toast(String(error), true)),
);
$("move-yes").addEventListener("click", () => commitSettings(draftDir, true));
$("move-no").addEventListener("click", () => commitSettings(draftDir));
$("move-cancel").addEventListener("click", () => showMoveConfirm(false));
$("data-open").addEventListener("click", () =>
  invoke("open_folder", { path: state.overview.data_dir }).catch((error) => toast(String(error), true)),
);
$("settings").addEventListener("submit", saveSettings);
for (const button of document.querySelectorAll("[data-open]")) {
  button.addEventListener("click", () =>
    invoke("open_folder", { path: placeDirs()[button.dataset.open] }).catch((error) => toast(String(error), true)),
  );
}

(async () => {
  try {
    await loadOverview();
  } catch (error) {
    toast(`读取设置失败：${error}`, true);
  }
  renderAll();
  appWindow.show();
  await pollDownload();
  await pollGame();
  if (state.overview) await loadSnapshots();
})();
