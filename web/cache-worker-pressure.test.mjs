import test from "node:test";
import assert from "node:assert/strict";

const origin = "https://host.example", cdn = "https://cdn.example", MiB = 1048576;
const current = cdn + "/moly/snapshots/cn-6.0.0-current/assets/";
const old = cdn + "/moly/snapshots/cn-6.0.0-old/assets/";
const engine = cdn + "/moly/releases/current/pkg/webgpu/moly-app_bg.wasm";
const oldEngine = cdn + "/moly/releases/old/pkg/webgpu/moly-app_bg.wasm";
let serial = 0;

async function withWorker(seed, required, run) {
  const stores = new Map(), handlers = new Map();
  const clients = [{ url: origin + "/moly/releases/current/stage.html?resource_origin=" + encodeURIComponent(cdn) + "&assets=" + encodeURIComponent(current) }];
  const key = value => typeof value === "string" ? value : value.url;
  const cacheApi = {
    async open(name) {
      if (!stores.has(name)) stores.set(name, new Map());
      const rows = stores.get(name);
      return {
        async put(url, response) { rows.set(key(url), response.clone()); },
        async match(url) { return rows.get(key(url))?.clone(); },
        async keys() { return [...rows.keys()].map(url => new Request(url)); },
        async delete(url) { return rows.delete(key(url)); },
      };
    },
    async keys() { return [...stores.keys()]; },
    async delete(name) { return stores.delete(name); },
    async match(url, { cacheName }) { return (await this.open(cacheName)).match(url); },
  };
  const control = await cacheApi.open("moly-control-v1");
  await control.put(origin + "/moly/__retention", Response.json({ enabled: true }));
  await control.put(origin + "/moly/__required", Response.json(required));
  // Seed declared cache-accounting sizes, without allocating hundreds of MiB
  // of duplicate payload bytes just to exercise eviction decisions.
  for (const [url, bytes, time] of seed) {
    const release = /\/moly\/releases\/([^/]+)\//.exec(url);
    const snapshot = /\/moly\/snapshots\/([^/]+)\//.exec(url);
    const name = "moly-resource-v1-" + (release ? "r-" + release[1] : "s-" + snapshot[1]);
    await (await cacheApi.open(name)).put(url, new Response("seed", { headers: { "X-Moly-Stored-Bytes": String(bytes), "X-Moly-Stored-At": String(time) } }));
  }
  class Scope {
    location = { origin };
    clients = { get: async () => clients[0], matchAll: async () => clients, claim: async () => {} };
    addEventListener(type, handler) { handlers.set(type, handler); }
    async skipWaiting() {}
  }
  const previous = Object.fromEntries(["self", "ServiceWorkerGlobalScope", "caches", "fetch"].map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
  let networkReads = 0;
  Object.assign(globalThis, {
    ServiceWorkerGlobalScope: Scope, self: new Scope(), caches: cacheApi,
    fetch: async () => { networkReads++; return new Response("abc", { headers: { "Cache-Control": "public,max-age=31536000,immutable", "x-oss-meta-moly-decoded-bytes": "3" } }); },
  });
  const has = url => [...stores].some(([name, rows]) => name.startsWith("moly-resource-v1-") && rows.has(url));
  const dispatch = async url => {
    const pending = [];
    let response;
    handlers.get("fetch")({ clientId: "stage", request: new Request(url), respondWith(work) { response = work; }, waitUntil(work) { pending.push(work); } });
    const value = await response;
    await Promise.all(pending);
    return value;
  };
  const clear = async () => {
    let work, reply;
    handlers.get("message")({ source: { url: origin + "/mysekai/interactions/" }, data: { source: "moly-cache-host", schemaVersion: 1, type: "clear" }, ports: [{ postMessage(value) { reply = value; } }], waitUntil(value) { work = value; } });
    await work;
    return reply;
  };
  const activate = async () => {
    let work;
    handlers.get("activate")({ waitUntil(value) { work = value; } });
    await work;
  };
  try {
    await import(`./cache-worker.mjs?pressure-test-${++serial}`);
    await run({ has, dispatch, clear, activate, clients, networkReads: () => networkReads });
  } finally {
    for (const [name, descriptor] of Object.entries(previous)) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete globalThis[name];
    }
  }
}

const mixed = () => [[engine, 128 * MiB, 1], [current + "base.json", 128 * MiB, 1], [oldEngine, 128 * MiB, 2], [old + "base.json", 128 * MiB, 3]];

test("a full historical cache makes room for the current source while keeping active essentials", async () => {
  await withWorker(mixed(), [current + "base.json", old + "base.json", current + "new-base.json"], async ({ dispatch, has }) => {
    assert.equal(await (await dispatch(current + "new-base.json")).text(), "abc");
    assert.ok(has(current + "new-base.json"));
    assert.ok(has(engine));
    assert.ok(has(current + "base.json"));
    assert.equal(has(oldEngine), false, "an inactive old engine is no longer permanently pinned");
    assert.ok(has(old + "base.json"));
  });
});

test("recent reads change eviction order, and an inactive snapshot's base is evictable", async () => {
  await withWorker(mixed(), [current + "base.json", old + "base.json", current + "new-base.json"], async ({ dispatch, has }) => {
    await dispatch(oldEngine);
    await dispatch(current + "new-base.json");
    assert.ok(has(oldEngine), "recently used inactive resources survive older inactive ones");
    assert.equal(has(old + "base.json"), false, "base membership alone is not a permanent pin");
    assert.ok(has(current + "base.json"));
    assert.ok(has(engine));
  });
});

test("on-demand dialogue assets are reused during this worker visit without durable storage", async () => {
  await withWorker([], [], async ({ dispatch, has, networkReads }) => {
    const voice = current + "voice/conversation.ogg";
    assert.equal(await (await dispatch(voice)).text(), "abc");
    assert.equal(has(voice), false);
    const repeated = await dispatch(voice);
    assert.equal(await repeated.text(), "abc");
    assert.equal(repeated.headers.get("X-Moly-Cache"), "visit");
    assert.equal(networkReads(), 1);
  });
});

test("upgrading the worker removes old optional disk entries but keeps declared base files", async () => {
  const base = current + "base.json", voice = current + "voice/old.ogg";
  await withWorker([[base, 4, 1], [voice, 4, 1]], [base], async ({ activate, has }) => {
    await activate();
    assert.equal(has(base), true);
    assert.equal(has(voice), false);
  });
});

test("a closed stage releases its engine and base protection", async () => {
  await withWorker(mixed(), [current + "base.json", old + "base.json"], async ({ dispatch, has, clients }) => {
    clients[0] = { url: origin + "/moly/releases/new-stage/stage.html?resource_origin=" + encodeURIComponent(cdn) };
    await dispatch(cdn + "/moly/releases/new-stage/pkg/webgpu/moly-app.js");
    assert.equal(has(engine), false, "the now inactive oldest engine can be reclaimed");
  });
});

test("active essentials exhausting the budget never break online reads or manual clear", async () => {
  const bases = ["base-a.json", "base-b.json", "base-c.json"].map(name => current + name);
  await withWorker([[engine, 128 * MiB, 1], ...bases.map(url => [url, 128 * MiB, 1])], bases, async ({ dispatch, has, clear }) => {
    const incoming = current + "optional.ogg";
    assert.equal(await (await dispatch(incoming)).text(), "abc", "online response succeeds even when retention has no room");
    assert.equal(has(incoming), false);
    assert.ok(has(engine));
    assert.ok(bases.every(has));
    const reply = await clear();
    assert.equal(reply.ok, true);
    assert.equal(reply.entries, 0);
    assert.equal(reply.bytes, 0);
    assert.equal(has(engine), false, "manual clear overrides active protection");
  });
});
