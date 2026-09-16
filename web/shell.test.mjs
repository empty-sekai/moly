import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import { JSDOM } from "jsdom";
import { ExperienceShell } from "./shell.mjs";
import { installSnapshotPicker } from "./snapshots.mjs";
import { mountMoly } from "./embed.mjs";

// Exercise the shipped HTML and event handlers. Only the WASM message boundary
// is replaced: these tests do not simulate Bevy admission or playback.
const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
const row = (key = "talk:general:42") => ({
  key,
  title: "放学后的日常",
  subtitle: "今天一起回家吧。",
  kind: "日常对话",
  available: true,
  characters: [{ id: 1, name: "星乃一歌", color: "#4455dd" }],
  lines: [{ speaker: "星乃一歌", text: "今天一起回家吧。" }],
});
const state = (selected = row(), changes = {}) => ({
  schemaVersion: 1,
  ready: true,
  tab: "conversations",
  query: "",
  character: null,
  characters: [{ id: 1, name: "星乃一歌" }],
  availability: "all",
  mode: "independent",
  region: "cn",
  version: "test",
  total: selected ? 1 : 0,
  page: 0,
  pageSize: 24,
  rows: selected ? [selected] : [],
  selected,
  status: { phase: "idle", canStop: false },
  ...changes,
});

function fixture(
  t,
  initial = state(),
  query = "?assets=/snapshots/cn/&renderer=webgl2",
) {
  const dom = new JSDOM(html, {
    url: `https://example.test/mysekai/experience/${query}`,
    pretendToBeVisual: true,
  });
  const saved = new Map();
  const globals = {
    window: dom.window,
    document: dom.window.document,
    location: dom.window.location,
    history: dom.window.history,
    localStorage: dom.window.localStorage,
    Option: dom.window.Option,
    HTMLElement: dom.window.HTMLElement,
    navigator: dom.window.navigator,
    matchMedia: () => ({ matches: false }),
    fetch: async () => ({ ok: false }),
  };
  for (const [key, value] of Object.entries(globals)) {
    saved.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
    Object.defineProperty(globalThis, key, {
      configurable: true,
      writable: true,
      value,
    });
  }
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  const commands = [];
  let snapshot = initial;
  let readError = null;
  let copied = "";
  Object.defineProperty(dom.window.navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: async (text) => {
        copied = text;
      },
    },
  });
  const shell = new ExperienceShell("/snapshots/cn/");
  const wasm = {
    library_command: (json) => commands.push(JSON.parse(json)),
    library_snapshot: () => {
      if (readError) throw readError;
      return JSON.stringify(snapshot);
    },
  };
  t.after(() => {
    clearInterval(shell.timer);
    clearTimeout(shell.queryTimer);
    clearTimeout(shell.toastTimer);
    dom.window.close();
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });
  return {
    shell,
    commands,
    dom,
    get copied() {
      return copied;
    },
    connect() {
      shell.connect(wasm);
      clearInterval(shell.timer);
    },
    show(value) {
      snapshot = value;
      shell.poll();
    },
    rejectReads(error) {
      readError = error;
    },
    get: (id) => dom.window.document.getElementById(id),
    keyboard(target, key, options = {}) {
      target.dispatchEvent(
        new dom.window.KeyboardEvent("keydown", {
          key,
          bubbles: true,
          cancelable: true,
          ...options,
        }),
      );
    },
  };
}

test("the tab chosen while modules load is the tab sent when the runtime connects", (t) => {
  const f = fixture(t);
  f.dom.window.document.querySelector('[data-tab="furniture"]').click();
  f.get("boot-start").focus();
  f.connect();
  assert.equal(
    f.commands.find((command) => command.type === "tab").value,
    "furniture",
  );
  assert.equal(
    f.commands.find((command) => command.type === "focus").value,
    true,
  );
});

test("sharing follows the actual selection, including a selection changed by pagination", async (t) => {
  const f = fixture(t);
  f.connect();
  f.dom.window.document.querySelector("#detail-panel > .text-button").click();
  await Promise.resolve();
  assert.equal(
    new URL(f.copied).searchParams.get("content"),
    "talk:general:42",
  );
  const next = row("talk:general:70");
  f.show(state(next, { page: 1, total: 30 }));
  f.dom.window.document.querySelector("#detail-panel > .text-button").click();
  await Promise.resolve();
  const link = new URL(f.copied);
  assert.equal(link.searchParams.get("content"), next.key);
  assert.equal(link.searchParams.get("assets"), "/snapshots/cn/");
  assert.equal(link.searchParams.get("renderer"), "webgl2");
});

test("a deep-linked selection is requested after the catalogue becomes ready", (t) => {
  const f = fixture(
    t,
    state(null, { ready: false }),
    "?assets=/snapshots/cn/&tab=performances&content=talk%3Afixture%3A8",
  );
  f.connect();
  assert.equal(
    f.commands.some((command) => command.type === "select"),
    false,
  );
  f.show(state(row("talk:fixture:1"), { tab: "performances" }));
  assert.equal(
    new URL(f.dom.window.location.href).searchParams.get("content"),
    "talk:fixture:8",
    "the prior selection cannot overwrite the requested deep-link before acknowledgment",
  );
  assert.deepEqual(
    f.commands.filter((command) => command.type === "select"),
    [{ schemaVersion: 1, type: "select", key: "talk:fixture:8" }],
  );
  f.show(state(row("talk:fixture:8"), { tab: "performances" }));
  assert.equal(
    new URL(f.dom.window.location.href).searchParams.get("content"),
    "talk:fixture:8",
  );
});

test("restoration waits for the requested category and reapplies filters after a related entry", (t) => {
  const f = fixture(
    t,
    state(),
    "?assets=/snapshots/cn/&tab=activities&fixture=100&character=1&availability=ready&experience=current&q=%E6%91%A9%E6%89%98&content=activity%3Anotalk%3A12%3A89",
  );
  f.connect();
  assert.equal(
    f.commands.some((command) => command.type === "select"),
    false,
    "a ready snapshot from the previous category cannot accept the requested activity",
  );
  f.show(state(null, { tab: "activities", ready: false }));
  assert.equal(
    f.commands.some((command) => command.type === "select"),
    false,
  );
  f.show(
    state(row("activity:notalk:12:1"), { tab: "activities", ready: true }),
  );
  const relatedIndex = f.commands.findIndex(
    (command) => command.type === "related",
  );
  const selectIndex = f.commands.findIndex(
    (command) => command.type === "select",
  );
  assert.ok(relatedIndex >= 0 && selectIndex > relatedIndex);
  const afterRelated = f.commands.slice(relatedIndex + 1, selectIndex);
  assert.ok(
    afterRelated.some(
      (command) => command.type === "character" && command.value === 1,
    ),
    "related navigation resets the character filter in the runtime",
  );
  assert.ok(
    afterRelated.some(
      (command) => command.type === "availability" && command.value === "ready",
    ),
    "related navigation resets availability in the runtime",
  );
  assert.ok(
    afterRelated.some(
      (command) => command.type === "query" && command.value === "摩托",
    ),
    "related navigation clears the query in the runtime",
  );
  assert.ok(
    f.commands.some(
      (command) => command.type === "mode" && command.value === "current",
    ),
  );
});

test("sharing preserves filters and a browsing page even when the selected item is on another page", async (t) => {
  const visible = state(row(), {
    tab: "performances",
    character: 1,
    availability: "ready",
    mode: "current",
    query: "放学",
    page: 3,
    total: 120,
    relatedFixture: 100,
  });
  const f = fixture(
    t,
    visible,
    "?assets=/snapshots/cn/&tab=performances&character=1&availability=ready&experience=current&q=%E6%94%BE%E5%AD%A6&page=3&content=talk%3Ageneral%3A42",
  );
  f.connect();
  const selectIndex = f.commands.findIndex(
    (command) => command.type === "select",
  );
  const pageIndex = f.commands.findIndex((command) => command.type === "page");
  assert.ok(
    selectIndex >= 0 && pageIndex > selectIndex,
    "selection restores detail, then page restores the independently browsed list",
  );
  assert.equal(f.commands[pageIndex].value, 3);
  f.dom.window.document.querySelector("#detail-panel > .text-button").click();
  await Promise.resolve();
  const link = new URL(f.copied);
  for (const [key, value] of Object.entries({
    tab: "performances",
    character: "1",
    availability: "ready",
    experience: "current",
    q: "放学",
    page: "3",
    fixture: "100",
    content: "talk:general:42",
  }))
    assert.equal(link.searchParams.get(key), value, key);
});

test("a related furniture story link uses the resulting category in its share URL", async (t) => {
  const furniture = {
    ...row("fixture:100"),
    kind: "陈设家具",
    title: "沙发",
    characters: [],
    lines: [],
    related: [
      { fixtureId: 100, tab: "performances", label: "家具故事", count: 3 },
    ],
  };
  const f = fixture(t, state(furniture, { tab: "furniture" }));
  f.connect();
  f.dom.window.document.querySelector(".related-links button").click();
  assert.ok(
    f.commands.some(
      (command) =>
        command.type === "related" &&
        command.fixtureId === 100 &&
        command.tab === "performances",
    ),
  );
  f.show(state(row("talk:fixture:8"), { tab: "performances" }));
  f.dom.window.document.querySelector("#detail-panel > .text-button").click();
  await Promise.resolve();
  assert.equal(new URL(f.copied).searchParams.get("tab"), "performances");
  assert.equal(new URL(f.copied).searchParams.get("content"), "talk:fixture:8");
});

test("canvas and DOM focus issue explicit input ownership commands", (t) => {
  const f = fixture(t);
  f.connect();
  f.get("catalog-search").focus();
  f.get("app-canvas").dispatchEvent(
    new f.dom.window.MouseEvent("pointerdown", { bubbles: true }),
  );
  assert.equal(f.dom.window.document.activeElement, f.get("app-canvas"));
  assert.equal(
    f.commands.filter((command) => command.type === "focus").at(-1).value,
    false,
  );
  f.get("catalog-search").focus();
  assert.equal(
    f.commands.filter((command) => command.type === "focus").at(-1).value,
    true,
  );
  let escaped = 0;
  f.dom.window.addEventListener("keydown", () => escaped++);
  f.keyboard(f.get("catalog-search"), "a");
  assert.equal(
    escaped,
    0,
    "text entry must not bubble to game window listeners",
  );
  f.keyboard(f.get("app-canvas"), "a");
  assert.equal(escaped, 1, "ordinary canvas controls remain available");
});

test("Chinese IME composition does not dispatch a partial query or playback shortcut", async (t) => {
  const f = fixture(t);
  f.connect();
  const search = f.get("catalog-search");
  search.focus();
  search.dispatchEvent(
    new f.dom.window.CompositionEvent("compositionstart", { bubbles: true }),
  );
  search.value = "mo";
  search.dispatchEvent(new f.dom.window.Event("input", { bubbles: true }));
  f.keyboard(search, "Enter", { ctrlKey: true, isComposing: true });
  await delay(190);
  assert.equal(
    f.commands.some(
      (command) => command.type === "query" || command.type === "play",
    ),
    false,
  );
  search.value = "摩托";
  search.dispatchEvent(
    new f.dom.window.CompositionEvent("compositionend", {
      bubbles: true,
      data: "摩托",
    }),
  );
  search.dispatchEvent(new f.dom.window.Event("input", { bubbles: true }));
  await delay(190);
  assert.deepEqual(
    f.commands
      .filter((command) => command.type === "query")
      .map((command) => command.value),
    ["摩托"],
  );
});

test("Escape stops an active performance even while a search field has focus", (t) => {
  const f = fixture(
    t,
    state(row(), { status: { phase: "playing", canStop: true } }),
  );
  f.connect();
  f.get("catalog-search").focus();
  f.keyboard(f.get("catalog-search"), "Escape");
  assert.equal(
    f.commands.filter((command) => command.type === "stop").length,
    1,
  );
});

test("empty and unavailable entries provide a visible explanation without starting playback", (t) => {
  const f = fixture(t, state(null));
  f.connect();
  assert.match(f.get("catalog-list").textContent, /没有找到/);
  assert.equal(f.get("previous-page").disabled, true);
  assert.equal(f.get("next-page").disabled, true);
  const unavailable = {
    ...row(),
    available: false,
    reason: "这段演出所需的资源尚未准备好。",
  };
  f.show(state(unavailable));
  assert.match(f.get("detail-panel").textContent, /所需的资源尚未准备好/);
  assert.equal(
    f.dom.window.document.querySelector("[data-play]").disabled,
    true,
  );
  f.keyboard(f.get("catalog-list"), "Enter", { ctrlKey: true });
  assert.equal(
    f.commands.some((command) => command.type === "play"),
    false,
  );
});

test("a synchronization failure requests cleanup and exposes retry instead of stale playback controls", (t) => {
  const f = fixture(
    t,
    state(row(), { status: { phase: "playing", canStop: true } }),
  );
  f.connect();
  const originalError = console.error;
  console.error = () => {};
  try {
    f.rejectReads(new Error("Snapshot transport failed"));
    f.shell.poll();
  } finally {
    console.error = originalError;
  }
  assert.ok(f.commands.some((command) => command.type === "stop"));
  assert.equal(f.shell.failed, true);
  assert.equal(f.get("boot-panel").hidden, false);
  assert.equal(f.get("boot-retry").hidden, false);
  assert.equal(
    f.dom.window.document.querySelector("[data-play]").disabled,
    true,
  );
  const count = f.commands.length;
  f.shell.play();
  assert.equal(f.commands.length, count);
});

test("page departure requests the shared runtime close lifecycle", (t) => {
  const f = fixture(t);
  f.connect();
  f.dom.window.dispatchEvent(new f.dom.window.PageTransitionEvent("pagehide"));
  assert.equal(f.commands.at(-1).type, "close");
});

test("filter and category navigation remove stale deep-link context", (t) => {
  const f = fixture(
    t,
    state(row(), { tab: "conversations", relatedFixture: 77, page: 2 }),
    "?assets=/snapshots/cn/&tab=conversations&fixture=77&page=2&content=talk%3Ageneral%3A42",
  );
  f.connect();
  f.get("availability-filter").value = "ready";
  f.get("availability-filter").dispatchEvent(
    new f.dom.window.Event("change", { bubbles: true }),
  );
  let url = new URL(f.dom.window.location.href);
  assert.equal(url.searchParams.get("fixture"), "77");
  assert.equal(url.searchParams.get("page"), null);
  assert.equal(url.searchParams.get("content"), null);
  f.dom.window.document.querySelector('[data-tab="furniture"]').click();
  url = new URL(f.dom.window.location.href);
  assert.equal(url.searchParams.get("fixture"), null);
  assert.equal(url.searchParams.get("page"), null);
  assert.equal(url.searchParams.get("content"), null);
});

test("runtime errors remain readable in the transport area", (t) => {
  const f = fixture(
    t,
    state(row(), {
      status: {
        phase: "error",
        canStop: false,
        error: "角色互动暂不可用：缺少可渲染动作轨道。",
      },
    }),
  );
  f.connect();
  assert.equal(f.get("playback-error").hidden, false);
  assert.match(f.get("playback-error").textContent, /缺少可渲染动作轨道/);
});

test("embedded settings remain accessible and return input to the game panel", (t) => {
  const f = fixture(t, state(), "?assets=/snapshots/cn/&embed=1");
  f.connect();
  f.get("catalog-search").focus();
  f.get("settings-button").click();
  assert.equal(f.get("settings-button").closest(".site-header"), null);
  assert.equal(f.dom.window.document.activeElement, f.get("app-canvas"));
  assert.deepEqual(f.commands.at(-1), {
    schemaVersion: 1,
    type: "focus",
    value: false,
  });
  assert.ok(
    f.commands.some(
      (command) => command.type === "settings" && command.value === "toggle",
    ),
  );
});

test("a region picker keeps valid snapshots when an optional manifest row is malformed", async (t) => {
  const f = fixture(t);
  globalThis.fetch = async () => ({
    ok: true,
    json: async () => ({
      snapshots: [
        null,
        { assetBase: "https://other/" },
        { region: "cn", version: "1", assetBase: "/snapshots/cn/" },
        { region: "jp", version: "2", assetBase: "/snapshots/jp/" },
      ],
    }),
  });
  await installSnapshotPicker("/snapshots/cn/");
  const picker = f.get("snapshot-picker");
  assert.equal(picker.value, "/snapshots/cn/");
  assert.deepEqual(
    [...picker.options].map((option) => option.text),
    ["国服 · 1", "日服 · 2"],
  );
  await installSnapshotPicker("/snapshots/cn/");
  assert.equal(
    f.dom.window.document.querySelectorAll("#snapshot-picker").length,
    1,
  );
});

test("embed messages require their exact frame and disposal releases listeners", (t) => {
  const f = fixture(t);
  const container = f.dom.window.document.createElement("div");
  f.dom.window.document.body.append(container);
  const statuses = [];
  const mounted = mountMoly(container, {
    src: "/moly/",
    onStatus: (value) => statuses.push(value),
  });
  const source = mounted.frame.contentWindow;
  const status = {
    source: "moly",
    schemaVersion: 1,
    type: "status",
    value: { phase: "playing" },
  };
  const dispatch = (origin, sender) =>
    f.dom.window.dispatchEvent(
      new f.dom.window.MessageEvent("message", {
        origin,
        source: sender,
        data: status,
      }),
    );
  dispatch("https://other.test", source);
  dispatch("https://example.test", f.dom.window);
  assert.equal(statuses.length, 0);
  dispatch("https://example.test", source);
  assert.equal(statuses.length, 1);
  const sent = [];
  source.postMessage = (...args) => sent.push(args);
  mounted.dispose();
  mounted.dispose();
  dispatch("https://example.test", source);
  assert.equal(statuses.length, 1);
  assert.equal(container.children.length, 0);
  assert.equal(sent.length, 1);
  assert.equal(sent[0][0].type, "close");
  assert.equal(sent[0][1], "https://example.test");
});
