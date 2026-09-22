import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  contentKey,
  filters,
  intent,
  sameOriginDirectory,
  resourceDirectory,
  resourceOrigin,
  EMBED_VERSION,
} from "./embed-contract.mjs";
import { createStageController } from "./stage-controller.mjs";
import { mountStage, ACTIVE_MOUNT } from "./embed-stage.mjs";

const state = (extra) => ({
  schemaVersion: 1,
  ready: true,
  region: "cn",
  version: "6.0.0",
  revision: 1,
  tab: "conversations",
  rows: [],
  characters: [],
  selected: null,
  status: { phase: "idle", canStop: false },
  ...extra,
});
function driver(initial = {}) {
  let value = state(initial);
  const commands = [],
    events = [];
  const controller = createStageController({
    region: "cn",
    version: "6.0.0",
    wasm: {
      library_command: (text) => commands.push(JSON.parse(text)),
      library_snapshot: () => JSON.stringify(value),
    },
    post: (type, value) => events.push({ type, value }),
  });
  return {
    controller,
    commands,
    events,
    set: (next) => {
      value = state(next);
    },
  };
}
test("contract accepts exact keys and refuses ambiguous IDs and gameplay commands", () => {
  for (const key of [
    "talk:general:3912",
    "talk:fixture:261",
    "fixture:8",
    "activity:preaction:6791:272",
  ])
    assert.equal(contentKey(key), key);
  for (const key of [
    "fixture:0",
    "fixture:-1",
    "fixture:01",
    "fixture:2147483648",
    "talk:3912",
    "activity:1:1",
    "fixture:1:2",
  ])
    assert.throws(() => contentKey(key));
  for (const type of ["execute", "spawn", "eval", "camera", "teleport"])
    assert.throws(() => intent(type, { script: "anything" }));
  assert.throws(() => filters({ entity: 123 }));
  assert.throws(() => filters({ pageSize: 101 }));
  assert.throws(() =>
    sameOriginDirectory("https://other.test/assets/", "https://host.test/"),
  );
  assert.throws(() =>
    sameOriginDirectory(
      "https://user:pw@host.test/assets/",
      "https://host.test/",
    ),
  );
  assert.throws(() =>
    sameOriginDirectory("/assets/?region=jp", "https://host.test/"),
  );
});
test("one-time initial configuration cannot be rewound by theme/locale updates", () => {
  const d = driver();
  d.controller.configure({ initial: { tab: "furniture", fixture: 157 } });
  d.controller.poll();
  const before = d.commands.length;
  d.controller.configure({ initial: { tab: "activities", fixture: 423 } });
  assert.equal(d.commands.length, before);
  assert.ok(d.commands.some((c) => c.type === "fixture" && c.value === 157));
  assert.ok(d.commands.every((c) => c.schemaVersion === 1));
});

test("public resources support changing the configured HTTPS origin without rebuilding", () => {
  const cdn = "https://assets-one.example";
  const base = "https://host.test/moly/releases/stage-test/stage.html";
  assert.equal(resourceOrigin(cdn), cdn);
  assert.equal(resourceOrigin(undefined), null);
  for (const origin of [cdn, "https://cdn-two.example:8443"]) {
    assert.equal(resourceOrigin(origin), origin);
    assert.equal(resourceDirectory(origin + "/moly/snapshots/cn-test/assets/", base, origin).origin, origin);
    assert.equal(resourceDirectory(origin + "/moly/asset-store/", base, origin).origin, origin);
    assert.throws(() => resourceDirectory(origin + "/moly/asset-store/", base, null));
    assert.throws(() => resourceDirectory(origin + "/moly/asset-store/", base, "https://unselected.example"));
  }
  for (const value of [
    "http://assets-one.example/moly/asset-store/",
    cdn + ".evil.test/moly/asset-store/",
    "https://user@" + cdn.slice(8) + "/moly/asset-store/",
    cdn + "/api/player/", cdn + "/moly/sources/cn/assets/",
    cdn + "/moly/snapshots/cn-test/assets/?token=private",
    cdn + "/moly/snapshots/old/../cn-test/assets/",
  ]) assert.throws(() => resourceDirectory(value, base, cdn), value);
  for (const value of ["http://other.test", "https://other.test/api/", "https://user@other.test", "https://other.test?key=private", "https://other.test/#fragment"])
    assert.throws(() => resourceOrigin(value));
});
test("wrong source snapshot rejects pending Play without admitting any action", () => {
  const d = driver({ region: "jp", version: "6.8.1" });
  d.controller.configure();
  d.controller.dispatch("play", "fixture:157");
  assert.equal(d.controller.poll(), null);
  assert.ok(
    d.events.some(
      (e) => e.type === "error" && e.value.code === "source_mismatch",
    ),
  );
  assert.ok(!d.commands.some((c) => c.type === "play"));
  assert.equal(d.controller.poll(), null);
});
test("browse/select never synthesize cancellation or runtime eligibility", () => {
  const d = driver({
    status: { phase: "playing", canStop: true, activeKey: "talk:general:3912" },
  });
  d.controller.configure();
  d.controller.poll();
  d.commands.length = 0;
  d.controller.dispatch("browse", { tab: "activities" });
  d.controller.dispatch("select", "activity:notalk:1:1");
  assert.ok(
    d.commands.some((c) => c.type === "tab" && c.value === "activities"),
  );
  assert.ok(
    d.commands.some(
      (c) => c.type === "select" && c.key === "activity:notalk:1:1",
    ),
  );
  assert.ok(
    !d.commands.some((c) => ["stop", "close", "play"].includes(c.type)),
  );
});
test("close acknowledges only a newer idle owner snapshot and cancels queued Play", () => {
  const d = driver({
    revision: 8,
    status: { phase: "playing", canStop: true },
  });
  d.controller.configure();
  d.controller.poll();
  d.controller.dispatch("close");
  d.controller.dispatch("play", "fixture:157");
  d.controller.poll();
  assert.ok(!d.events.some((e) => e.type === "closed"));
  assert.ok(!d.commands.some((c) => c.type === "play"));
  d.set({ revision: 9 });
  d.controller.poll();
  assert.deepEqual(d.events.at(-1), {
    type: "closed",
    value: { restored: true },
  });
});
test("unready protocol queues are bounded", () => {
  const d = driver({ ready: false });
  d.controller.configure();
  for (let i = 0; i < 64; i++) d.controller.dispatch("stop");
  assert.throws(() => d.controller.dispatch("stop"), /full/);
});

function dom() {
  const document = new JSDOM('<main id="target"></main>', {
    url: "https://host.test/en-us/mysekai/interactions/",
  });
  const previous = {};
  for (const name of ["window", "document", "location", "HTMLElement"]) {
    previous[name] = globalThis[name];
    globalThis[name] = document.window[name];
  }
  return {
    target: document.window.document.getElementById("target"),
    window: document.window,
    cleanup() {
      document.window[ACTIVE_MOUNT] && delete document.window[ACTIVE_MOUNT];
      document.window.close();
      for (const [name, value] of Object.entries(previous)) {
        if (value === undefined) delete globalThis[name];
        else globalThis[name] = value;
      }
    },
  };
}
const options = {
  src: "https://host.test/moly/releases/stage-test/stage.html",
  assets: "/moly/snapshots/cn-test/assets/",
  region: "cn",
  version: "6.0.0",
  snapshot: "cn-test",
};
function emit(
  window,
  frame,
  type,
  value,
  origin = "https://host.test",
  source = frame.contentWindow,
  version = EMBED_VERSION,
) {
  window.dispatchEvent(
    new window.MessageEvent("message", {
      origin,
      source,
      data: { source: "moly", schemaVersion: version, type, value },
    }),
  );
}
test("stage validates mount inputs before creating a renderer", () => {
  const env = dom();
  try {
    for (const invalid of [
      { src: "https://other.test/stage.html" },
      { src: "https://user:pw@host.test/stage.html" },
      { src: "https://host.test/stage.html#other" },
      { assets: undefined },
      { region: "invalid-region" },
      { version: "current" },
      { renderer: "invented" },
      { snapshot: "../jp" },
    ]) {
      assert.throws(() => mountStage(env.target, { ...options, ...invalid }));
      assert.equal(env.target.children.length, 0);
    }
  } finally {
    env.cleanup();
  }
});
test("sound preference is carried by configure and live changes use a sound intent", () => {
  const env = dom();
  let handle;
  try {
    handle = mountStage(env.target, { ...options, sound: false });
    const sent = [];
    handle.frame.contentWindow.postMessage = (message) => sent.push(message);
    emit(env.window, handle.frame, "hello", {
      contract: 2,
      instance: "sound-test",
    });
    assert.equal(sent[0].type, "configure");
    assert.equal(sent[0].value.sound, false);
    handle.setSoundEnabled(true);
    assert.deepEqual(sent.at(-1).value, { type: "sound", value: true });
  } finally {
    handle?.dispose();
    env.cleanup();
  }
});

test("CDN resources preserve same-origin iframe and the synchronous activation gate", () => {
  const env = dom();
  let handle;
  try {
    const cdn = "https://cdn-two.example:8443";
    handle = mountStage(env.target, { ...options, assets: cdn + options.assets, resourceOrigin: cdn });
    const source = new URL(handle.frame.src);
    assert.equal(source.origin, env.window.location.origin);
    assert.equal(source.searchParams.get("assets"), cdn + options.assets);
    assert.equal(source.searchParams.get("resource_origin"), cdn);
    const gate = handle.frame.contentDocument.createElement("button");
    gate.id = "stage-start";
    gate.dataset.molyReady = "true";
    handle.frame.contentDocument.append(gate);
    let activated = false;
    gate.addEventListener("click", () => { activated = true; });
    handle.play("talk:fixture:6177");
    assert.equal(activated, true);
  } finally {
    handle?.dispose();
    env.cleanup();
  }
});

test("stage callback boundary requires both exact iframe window and same origin/version", () => {
  const env = dom();
  let handle;
  try {
    let snapshots = 0;
    handle = mountStage(env.target, {
      ...options,
      onSnapshot: () => snapshots++,
    });
    assert.throws(() => mountStage(env.target, options), /already mounted/);
    emit(env.window, handle.frame, "snapshot", state(), "https://other.test");
    emit(
      env.window,
      handle.frame,
      "snapshot",
      state(),
      "https://host.test",
      env.window,
    );
    emit(
      env.window,
      handle.frame,
      "snapshot",
      state(),
      "https://host.test",
      handle.frame.contentWindow,
      999,
    );
    assert.equal(snapshots, 0);
    emit(env.window, handle.frame, "snapshot", state());
    assert.equal(snapshots, 1);
    const frame = handle.frame;
    handle.setTheme({ mode: "dark", accent: "#123456" });
    handle.setLocale("ja-JP");
    assert.equal(handle.frame, frame);
    assert.equal(env.target.children.length, 1);
    handle.dispose();
    handle.dispose();
    assert.equal(env.target.children.length, 0);
    assert.equal(env.window[ACTIVE_MOUNT], undefined);
  } finally {
    handle?.dispose();
    env.cleanup();
  }
});
test("concurrent close callers share one promise and honor the actual restore result", async () => {
  const env = dom();
  let handle;
  try {
    handle = mountStage(env.target, options);
    emit(env.window, handle.frame, "hello", { contract: 2 });
    emit(
      env.window,
      handle.frame,
      "snapshot",
      state({ status: { canStop: true, phase: "playing" } }),
    );
    const first = handle.close(),
      second = handle.close();
    assert.equal(first, second);
    emit(env.window, handle.frame, "closed", { restored: false });
    assert.equal(await first, false);
    assert.equal(await second, false);
    assert.equal(env.target.children.length, 0);
  } finally {
    handle?.dispose();
    env.cleanup();
  }
});

test("document reload keeps current filters and retries only an unacknowledged Play", () => {
  const env = dom();
  let handle;
  try {
    handle = mountStage(env.target, options);
    const sent = [];
    handle.frame.contentWindow.postMessage = (message) => sent.push(message);
    emit(env.window, handle.frame, "hello", {
      contract: 2,
      instance: "document-one",
    });
    handle.browse({ tab: "activities", fixture: 727, query: "" });
    handle.select("activity:preaction:6791:272");
    handle.play("activity:preaction:6791:272");
    const count = sent.length;
    emit(env.window, handle.frame, "hello", {
      contract: 2,
      instance: "document-one",
    });
    assert.equal(sent.length, count);
    sent.length = 0;
    emit(env.window, handle.frame, "hello", {
      contract: 2,
      instance: "document-two",
    });
    assert.equal(sent[0].type, "configure");
    assert.equal(sent[0].value.initial.tab, "activities");
    assert.equal(sent[0].value.initial.fixture, 727);
    assert.equal(sent[0].value.content, "activity:preaction:6791:272");
    assert.deepEqual(sent[1].value, {
      type: "play",
      value: "activity:preaction:6791:272",
    });
    emit(
      env.window,
      handle.frame,
      "snapshot",
      state({
        status: {
          phase: "playing",
          canStop: true,
          activeKey: "activity:preaction:6791:272",
        },
      }),
    );
    sent.length = 0;
    emit(env.window, handle.frame, "hello", {
      contract: 2,
      instance: "document-three",
    });
    assert.equal(sent.length, 1);
    assert.equal(sent[0].type, "configure");
  } finally {
    handle?.dispose();
    env.cleanup();
  }
});

test("legacy shell and stage share one renderer lease in either mount order", async () => {
  const { mountMoly } = await import("./embed.mjs");
  const env = dom();
  let shell, stage;
  try {
    shell = mountMoly(env.target, { src: "/index.html" });
    assert.throws(
      () => mountMoly(env.target, { ...options, view: "stage" }),
      /already mounted/,
    );
    assert.throws(
      () => mountMoly(env.target, { src: "/index.html" }),
      /already mounted/,
    );
    shell.dispose();
    stage = mountMoly(env.target, { ...options, view: "stage" });
    assert.throws(
      () => mountMoly(env.target, { src: "/index.html" }),
      /already mounted/,
    );
    stage.dispose();
    assert.equal(env.target.children.length, 0);
  } finally {
    shell?.dispose();
    stage?.dispose();
    env.cleanup();
  }
});
