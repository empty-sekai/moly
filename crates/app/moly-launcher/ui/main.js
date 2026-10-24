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
const FOLDER_LABELS = { home: "主目录", data: "数据", logs: "日志", versions: "程序版本", resources: "资源" };
const UPDATE_STAGES = { checking: "检查版本", downloading: "下载", extracting: "解压", done: "完成", cancelled: "已取消" };

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
  update: null,
  updateError: null,
  updateStatus: null,
  updateHidden: false,
  updateWatch: false,
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

function updating() {
  return Boolean(state.updateStatus && !state.updateStatus.finished);
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
  } else if (updating()) {
    play.disabled = true;
    title.textContent = "正在更新…";
    sub.textContent = "完成后即可开始";
  } else if (downloading()) {
    play.disabled = true;
    const kind = state.download.kind;
    title.textContent = kind === "relocate" ? "正在移动…" : kind === "migrate" ? "正在迁移…" : "下载中…";
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
  const moving = d.kind === "migrate" || d.kind === "relocate";
  const snapshot = state.snapshots.find((item) => item.id === d.snapshot);
  if (d.kind === "relocate") {
    $("download-title").textContent = `移动${FOLDER_LABELS[d.snapshot] ?? "文件夹"}`;
  } else if (d.kind === "migrate") {
    $("download-title").textContent = "迁移资源";
  } else {
    $("download-title").textContent = snapshot ? `${regionName(snapshot.region)} ${snapshot.version}` : d.snapshot;
  }
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
  // Moves of a folder cannot be cancelled from the launcher.
  action.hidden = !d.finished && d.kind === "relocate";
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

function renderUpdate() {
  const s = state.updateStatus;
  const check = state.update;
  const running = updating();
  const mode = running ? "running" : s ? "done" : check?.newer ? "offer" : null;
  $("update").hidden = !mode || (mode !== "running" && state.updateHidden);
  const versions = state.overview?.versions ?? [];
  $("installed-versions").hidden = !versions.length;
  $("installed-versions").textContent = `程序版本 ${versions.join("、")}`;
  $("update-error").hidden = !state.updateError;
  $("update-error").title = state.updateError ?? "";
  if (!mode) return;
  const text = $("update-text");
  text.className = "update-text";
  $("update-track").hidden = mode !== "running";
  $("update-start").hidden = mode !== "offer";
  $("update-start").disabled = downloading() || state.game.running;
  $("update-notes").hidden = mode !== "offer" || !check.page;
  $("update-cancel").hidden = mode !== "running";
  $("update-close").hidden = mode === "running";
  $("update-restart").hidden = !(mode === "done" && s.launcher_replaced);
  if (mode === "running") {
    const version = s.version || check?.latest || "";
    const count = s.bytes_total ? ` · ${bytes(s.bytes_done)} / ${bytes(s.bytes_total)}` : "";
    text.textContent = `正在更新到 v${version}：${UPDATE_STAGES[s.stage] ?? s.stage}${count}`;
    $("update-bar").style.width = s.bytes_total ? `${((s.bytes_done / s.bytes_total) * 100).toFixed(2)}%` : "0%";
  } else if (mode === "done") {
    if (s.error) {
      text.className = "update-text bad";
      text.textContent = `更新失败：${s.error}`;
    } else if (s.stage === "cancelled") {
      text.textContent = "更新已取消";
    } else {
      text.textContent = `已更新到 v${s.version}，${s.launcher_replaced ? "重启启动器后生效" : "下次启动游戏时使用新版本"}`;
    }
  } else {
    text.textContent = `发现新版本 v${check.latest}（约 ${bytes(check.download_bytes)}），当前为 v${check.current}`;
  }
}

function placeDirs() {
  const ov = state.overview;
  return {
    home: ov.home_dir,
    data: ov.data_dir,
    logs: ov.logs_dir,
    versions: ov.versions_dir,
    resources: ov.resource_dir,
  };
}

function renderPlaces() {
  if (!state.overview) return;
  const dirs = placeDirs();
  for (const [key, dir] of Object.entries(dirs)) {
    $(`place-${key}`).textContent = dir;
    $(`place-${key}`).title = dir;
  }
  // A folder cannot change while a download, a move, an update or the game uses it.
  const busy = state.game.running || downloading() || updating();
  for (const button of document.querySelectorAll("[data-change]")) {
    const locked = button.dataset.change === "data" && state.overview.data_dir_locked;
    button.disabled = busy || locked;
    button.title = locked ? "MOLY_DATA_DIR 指定了数据目录，不能在这里更改" : busy ? "下载、移动、更新或游戏运行时不能更改" : "";
  }
}

function renderAll() {
  if (state.overview) $("version").textContent = `v${state.overview.version}`;
  renderPlaces();
  renderPlay();
  renderSnapshots();
  renderDownload();
  renderGame();
  renderUpdate();
}

/* ---------- actions ---------- */

async function loadOverview() {
  state.overview = await invoke("overview");
  fillSettings();
}

function importedMessage(imported) {
  const taken = [imported.settings && "设置", imported.saves && "存档"].filter(Boolean);
  let message = `已沿用上一个版本的${taken.length ? taken.join("与") : "启动器配置"}：${imported.from}`;
  if (imported.resource_dir) message += `，资源目录：${imported.resource_dir}`;
  return message;
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
    if (d.kind === "relocate") {
      const label = FOLDER_LABELS[d.snapshot] ?? "文件夹";
      if (d.phase === "done") toast(`${label}已移动到新位置`);
      else if (d.phase === "cancelled") toast(`${label}的移动已取消`);
      else if (d.error) toast(`移动失败：${d.error}`, true);
      // The resources may have moved with the home directory, so their sizes are read again.
      if (d.phase === "done" && d.snapshot === "home") {
        state.sizes.clear();
        loadSnapshots();
      }
      return;
    }
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

/* ---------- folders ---------- */

/** The folder picked for a change and waiting for the confirmation. */
let placePending = null;

/** Picks a new folder for `kind`, then asks for the confirmation of the change. */
async function changePlace(kind) {
  let picked;
  try {
    picked = await invoke(kind === "resources" ? "pick_folder" : "pick_any_folder");
  } catch (error) {
    toast(String(error), true);
    return;
  }
  if (!picked) return;
  if (sameDir(picked, placeDirs()[kind])) {
    toast(`${FOLDER_LABELS[kind]}已经在这个位置`);
    return;
  }
  const installed = state.overview.installed;
  if (kind === "resources" && !installed.length) {
    await switchResources(picked);
    return;
  }
  placePending = { kind, to: picked };
  if (kind === "resources") {
    const total = installed.reduce((sum, item) => sum + item.bytes, 0);
    $("place-confirm-text").textContent = `原来的位置有已下载的资源（${bytes(total)}）。要把它们迁移到 ${picked} 吗？`;
    $("place-confirm-yes").textContent = "迁移";
    $("place-confirm-switch").hidden = false;
  } else {
    $("place-confirm-text").textContent = kind === "home"
      ? `把主目录移到 ${picked}？位于主目录里的数据、日志、程序版本与资源会一起移动。目标文件夹须为空或不存在。`
      : `把${FOLDER_LABELS[kind]}移到 ${picked}？目标文件夹须为空或不存在。`;
    $("place-confirm-yes").textContent = "移动";
    $("place-confirm-switch").hidden = true;
  }
  $("place-confirm").hidden = false;
}

/** Points the resource directory at `to`; the resources in the old directory stay there. */
async function switchResources(to) {
  try {
    state.overview = await invoke("save_config", { config: { ...state.overview.config, resource_dir: to } });
  } catch (error) {
    toast(String(error), true);
    return;
  }
  state.sizes.clear();
  renderAll();
  loadSnapshots();
  toast(`资源目录已切换到 ${to}`);
}

/** Carries out the pending change. `switchOnly` changes only the resource directory. */
async function applyPlace(switchOnly) {
  const pending = placePending;
  cancelPlace();
  if (!pending) return;
  if (pending.kind === "resources" && switchOnly) {
    await switchResources(pending.to);
    return;
  }
  try {
    if (pending.kind === "resources") {
      await invoke("start_migrate", { to: pending.to });
    } else {
      await invoke("set_location", { kind: pending.kind, to: pending.to });
    }
  } catch (error) {
    toast(String(error), true);
    return;
  }
  state.lastSample = null;
  state.speed = 0;
  state.fileRate = 0;
  pollDownload();
}

function cancelPlace() {
  placePending = null;
  $("place-confirm").hidden = true;
}

/* ---------- program updates ---------- */

/** Reads the newest release; the offer is shown when it is newer than this launcher. */
async function checkUpdate() {
  try {
    state.update = await invoke("check_update");
    state.updateError = null;
  } catch (error) {
    state.update = null;
    state.updateError = String(error);
  }
  renderAll();
}

async function startUpdate() {
  try {
    await invoke("start_update");
  } catch (error) {
    toast(String(error), true);
    return;
  }
  state.updateHidden = false;
  pollUpdate();
}

let updateTimer = 0;
async function pollUpdate() {
  clearTimeout(updateTimer);
  const s = await invoke("update_status");
  state.updateStatus = s;
  if (s && !s.finished) {
    state.updateWatch = true;
    renderAll();
    updateTimer = setTimeout(pollUpdate, 500);
    return;
  }
  if (state.updateWatch && s) {
    state.updateWatch = false;
    try {
      // The installed versions and the game program may have changed.
      state.overview = await invoke("overview");
    } catch (error) {
      toast(String(error), true);
    }
    if (s.error) {
      toast(`更新失败：${s.error}`, true);
    } else if (s.stage === "cancelled") {
      toast("更新已取消");
    } else {
      if (state.update) state.update = { ...state.update, newer: false };
      toast(`已更新到 v${s.version}`);
    }
  }
  renderAll();
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
for (const button of document.querySelectorAll("[data-change]")) {
  button.addEventListener("click", () => changePlace(button.dataset.change));
}
$("place-confirm-yes").addEventListener("click", () => applyPlace(false));
$("place-confirm-switch").addEventListener("click", () => applyPlace(true));
$("place-confirm-no").addEventListener("click", cancelPlace);
$("update-start").addEventListener("click", startUpdate);
$("update-notes").addEventListener("click", () => {
  if (state.update?.page) {
    invoke("open_url", { url: state.update.page }).catch((error) => toast(String(error), true));
  }
});
$("update-cancel").addEventListener("click", () => invoke("cancel_update"));
$("update-restart").addEventListener("click", () =>
  invoke("restart_launcher").catch((error) => toast(String(error), true)),
);
$("update-close").addEventListener("click", () => {
  state.updateHidden = true;
  renderUpdate();
});
$("update-error").addEventListener("click", checkUpdate);
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
  if (state.overview?.imported) toast(importedMessage(state.overview.imported));
  renderAll();
  appWindow.show();
  checkUpdate();
  await pollUpdate();
  await pollDownload();
  await pollGame();
  if (state.overview) await loadSnapshots();
})();
