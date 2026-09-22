import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { createHash, webcrypto } from "node:crypto";

// Unit tests execute the shipped worker body with a controllable storage API.
// Actual browser cache/online behavior is additionally exercised by QA scripts.
function worker(stores = new Map()) {
  const handlers = new Map();
  let network = 0,
    readFailure = false,
    writeFailure = false,
    controlFailure = false,
    hold = null,
    networkHold = null,
    responseStatus = 200,
    responseCacheControl = "public, immutable";
  const cacheFor = (name) => ({
    async match(request) {
      return stores
        .get(name)
        ?.get(typeof request === "string" ? request : request.url)
        ?.clone();
    },
    async keys() {
      return [...(stores.get(name)?.keys() || [])].map(
        (url) => new Request(url),
      );
    },
    async delete(request) {
      return stores
        .get(name)
        ?.delete(typeof request === "string" ? request : request.url);
    },
    async put(request, response) {
      if (name === "moly-control-v1" && controlFailure)
        throw new DOMException("blocked", "QuotaExceededError");
      if (name.startsWith("moly-resource-v1-")) {
        if (hold) await hold;
        if (writeFailure) throw new DOMException("full", "QuotaExceededError");
      }
      if (!stores.has(name)) stores.set(name, new Map());
      stores
        .get(name)
        .set(
          typeof request === "string" ? request : request.url,
          response.clone(),
        );
    },
  });
  const caches = {
    async open(name) {
      if (!stores.has(name)) stores.set(name, new Map());
      return cacheFor(name);
    },
    async keys() {
      return [...stores.keys()];
    },
    async delete(name) {
      return stores.delete(name);
    },
    async match(request, { cacheName }) {
      if (readFailure) throw new DOMException("blocked", "SecurityError");
      return cacheFor(cacheName).match(request);
    },
  };
  class ServiceWorkerGlobalScope {}
  const self = new ServiceWorkerGlobalScope();
  Object.assign(self, {
    location: new URL("https://qa.test/moly/cache-worker.mjs"),
    clients: { claim: async () => {}, matchAll: async () => [] },
    skipWaiting: async () => {},
    addEventListener: (name, callback) => handlers.set(name, callback),
  });
  const source = fs
    .readFileSync(new URL("./cache-worker.mjs", import.meta.url), "utf8")
    .replace(/^export /gm, "");
  vm.runInNewContext(
    source,
    {
      self,
      crypto: webcrypto,
      ServiceWorkerGlobalScope,
      caches,
      URL,
      Headers,
      Response,
      Request,
      Promise,
      Date,
      Number,
      console,
      fetch: async (request) => {
        network++;
        if (networkHold) await networkHold;
        return new Response("resource", {
          status: responseStatus,
          headers: {
            "Cache-Control": responseCacheControl,
            "X-Moly-Decoded-Bytes": new URL(request.url).pathname.includes(
              "large",
            )
              ? String(80 * 1024 * 1024)
              : "8",
          },
        });
      },
    },
    { filename: "cache-worker.mjs" },
  );
  return {
    stores,
    network: () => network,
    readFailure: (value) => {
      readFailure = value;
    },
    writeFailure: (value) => {
      writeFailure = value;
    },
    controlFailure: (value) => {
      controlFailure = value;
    },
    hold: (value) => {
      hold = value;
    },
    networkHold: (value) => {
      networkHold = value;
    },
    responseStatus: (value) => { responseStatus = value; },
    responseCacheControl: (value) => { responseCacheControl = value; },
    async message(type, enabled) {
      let result, waiting;
      handlers.get("message")({
        data: { source: "moly-cache-host", schemaVersion: 1, type, enabled },
        source: { url: "https://qa.test/en-us/mysekai/interactions/" },
        ports: [
          {
            postMessage: (value) => {
              result = value;
            },
          },
        ],
        waitUntil: (value) => {
          waiting = value;
        },
      });
      await waiting;
      return result;
    },
    async request(name = "data.json", options = {}) {
      let response;
      const waiting = [];
      handlers.get("fetch")({
        request: new Request(
          name.startsWith("/")
            ? "https://qa.test" + name
            : "https://qa.test/moly/releases/stage-test/" + name,
          options,
        ),
        respondWith: (value) => {
          response = value;
        },
        waitUntil: (value) => waiting.push(value),
      });
      const value = await response;
      return { response: value, settle: () => Promise.all(waiting) };
    },
  };
}
test("quota write failure preserves a successful online response and recoverable controls", async () => {
  const w = worker();
  assert.equal((await w.message("retain", true)).enabled, true);
  w.writeFailure(true);
  const request = await w.request();
  assert.equal(await request.response.text(), "resource");
  await request.settle();
  const state = await w.message("query");
  assert.equal(state.ok, true);
  assert.equal(state.bytes, 0);
  assert.equal(w.network(), 1);
  w.writeFailure(false);
  const next = await w.request();
  await next.settle();
  assert.equal((await w.message("query")).bytes, 8);
});
test("storage read failure falls back to the network instead of breaking gameplay loading", async () => {
  const w = worker();
  await w.message("retain", true);
  w.readFailure(true);
  const request = await w.request();
  assert.equal(request.response.status, 200);
  assert.equal(await request.response.text(), "resource");
  await request.settle();
  assert.equal(w.network(), 1);
});
test("failed preference persistence does not silently enable retention", async () => {
  const w = worker();
  w.controlFailure(true);
  assert.equal((await w.message("retain", true)).ok, false);
  assert.equal((await w.message("query")).enabled, false);
  const request = await w.request();
  await request.settle();
  assert.equal((await w.message("query")).bytes, 0);
});
test("pending cache streams are bounded by bytes, not just the number of entries", async () => {
  const w = worker();
  await w.message("retain", true);
  let release;
  w.hold(
    new Promise((resolve) => {
      release = resolve;
    }),
  );
  const first = await w.request("large-a.glb");
  let admitted = false;
  const waiting = w.request("large-b.glb").then((value) => {
    admitted = true;
    return value;
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(
    admitted,
    false,
    "second response must backpressure, not be discarded",
  );
  release();
  await first.settle();
  const second = await waiting;
  await second.settle();
  const state = await w.message("query");
  assert.equal(state.entries, 2);
  assert.equal(state.bytes, 160 * 1024 * 1024);
  assert.equal(w.network(), 2);
});
test("disable after scoped clear makes subsequent loads online without recreating resource caches", async () => {
  const w = worker();
  await w.message("retain", true);
  const cold = await w.request();
  await cold.settle();
  const warm = await w.request();
  assert.equal(warm.response.headers.get("X-Moly-Cache"), "retained");
  await warm.settle();
  assert.equal(w.network(), 1);
  w.stores.set("other-app", new Map());
  await w.message("clear");
  await w.message("retain", false);
  const online = await w.request();
  await online.settle();
  assert.equal(w.network(), 2);
  assert.equal((await w.message("query")).bytes, 0);
  assert.ok(w.stores.has("other-app"));
});

test("a response requested before clear cannot silently refill the resource store", async () => {
  const w = worker();
  await w.message("retain", true);
  let release;
  w.networkHold(
    new Promise((resolve) => {
      release = resolve;
    }),
  );
  const pending = w.request();
  while (w.network() === 0)
    await new Promise((resolve) => setTimeout(resolve, 0));
  await w.message("clear");
  release();
  const response = await pending;
  await response.settle();
  assert.equal(await response.response.text(), "resource");
  assert.equal((await w.message("query")).bytes, 0);
});

test("optional models and audio survive a fresh worker without a response memory cache", async () => {
  const first = worker();
  await first.message("retain", true);
  const names = [
    "/moly/snapshots/cn-source/assets/fixture-models/horse.glb",
    "/moly/snapshots/cn-source/assets/audio/dialogue.ogg",
  ];
  for (const name of names) {
    const response = await first.request(name);
    assert.equal(await response.response.text(), "resource");
    await response.settle();
  }
  const restarted = worker(first.stores);
  const state = await restarted.message("query");
  assert.equal(state.enabled, true);
  assert.equal(state.entries, 2);
  assert.equal(state.bytes, 16);
  assert.equal(state.limitBytes, 512 * 1024 * 1024);
  for (const name of names) {
    const response = await restarted.request(name);
    assert.equal(response.response.headers.get("X-Moly-Cache"), "retained");
    assert.equal(await response.response.text(), "resource");
    await response.settle();
  }
  assert.equal(restarted.network(), 0);
  // Removing the disk store must make the next read miss, even in the same
  // worker. A leftover visit-response map would incorrectly mask this.
  first.stores.delete("moly-resource-v1-s-cn-source");
  const miss = await restarted.request(names[0]);
  await miss.settle();
  assert.equal(restarted.network(), 1);
  assert.equal(miss.response.headers.get("X-Moly-Cache"), null);
});

test("explicit clear removes every resource-cache version, not preferences or other apps", async () => {
  const w = worker();
  await w.message("retain", true);
  for (const name of [
    "moly-resource-v0-old-engine",
    "moly-resource-v1-s-old-source",
    "moly-resource-v1-shared-pack-store",
    "moly-resource-v2-new-source",
  ]) {
    w.stores.set(name, new Map([["https://qa.test/moly/test.bin", new Response("old", {
      headers: { "X-Moly-Stored-Bytes": "3", "X-Moly-Stored-At": "1" },
    })]]));
  }
  w.stores.set("other-app-resources", new Map());
  w.stores.set("moly-layout-preferences", new Map());
  const state = await w.message("clear");
  assert.equal(state.ok, true);
  assert.equal(state.entries, 0);
  assert.equal(state.bytes, 0);
  assert.equal(state.enabled, true);
  assert.ok([...w.stores.keys()].every(name => !name.startsWith("moly-resource-")));
  assert.ok(w.stores.has("moly-control-v1"));
  assert.ok(w.stores.has("moly-layout-preferences"));
  assert.ok(w.stores.has("other-app-resources"));
});

test("optional request in flight remains readable after clear but cannot refill disk", async () => {
  const w = worker();
  await w.message("retain", true);
  let release;
  w.networkHold(new Promise(resolve => { release = resolve; }));
  const pending = w.request("/moly/snapshots/cn-source/assets/audio/dialogue.ogg");
  while (w.network() === 0) await new Promise(resolve => setTimeout(resolve, 0));
  await w.message("clear");
  release();
  const response = await pending;
  assert.equal(await response.response.text(), "resource");
  await response.settle();
  assert.equal((await w.message("query")).entries, 0);
  w.networkHold(null);
  const later = await w.request("/moly/snapshots/cn-source/assets/audio/dialogue.ogg");
  await later.settle();
  assert.equal((await w.message("query")).entries, 1);
});

test("clear waits for an active disk write then removes it without cancelling the response", async () => {
  const w = worker();
  await w.message("retain", true);
  let release;
  w.hold(new Promise(resolve => { release = resolve; }));
  const pending = await w.request("/moly/snapshots/cn-source/assets/fixture-models/horse.glb");
  let cleared = false;
  const clear = w.message("clear").then(value => { cleared = true; return value; });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(cleared, false);
  assert.equal(await pending.response.text(), "resource");
  release();
  await pending.settle();
  assert.equal((await clear).entries, 0);
  assert.equal((await w.message("query")).bytes, 0);
});

test("404 and development responses are never cached or replaced with false successes", async () => {
  const w = worker();
  await w.message("retain", true);
  const path = "/moly/snapshots/cn-source/assets/fixture-models/missing.glb";
  w.responseStatus(404);
  for (let index = 0; index < 2; index++) {
    const response = await w.request(path);
    assert.equal(response.response.status, 404);
    await response.settle();
  }
  assert.equal(w.network(), 2);
  assert.equal((await w.message("query")).entries, 0);
  w.responseStatus(200);
  w.responseCacheControl("no-cache");
  const development = await w.request(path);
  await development.settle();
  assert.equal((await w.message("query")).entries, 0);
  w.responseCacheControl("public, immutable");
  const repaired = await w.request(path);
  assert.equal(repaired.response.status, 200);
  await repaired.settle();
  assert.equal((await w.message("query")).entries, 1);
});

test("shared CAS responses are hash-checked, pinned, and retained once across releases", async () => {
  const w = worker();
  await w.message("retain", true);
  const hash = createHash("sha256").update("resource").digest("hex");
  const name = `/moly/asset-store/blobs/${hash.slice(0, 2)}/${hash}.bin`;
  const first = await w.request(name, { headers: { "X-Moly-Required": "1" } });
  assert.equal(await first.response.text(), "resource");
  await first.settle();
  assert.equal((await w.message("query")).entries, 1);
  const second = await w.request(name);
  await second.settle();
  assert.equal(second.response.headers.get("X-Moly-Cache"), "retained");
  assert.equal(w.network(), 1);
  const controls = await Promise.all(
    [...w.stores.get("moly-control-v1").values()].map((value) =>
      value.clone().text(),
    ),
  );
  assert.ok(controls.some((value) => value.includes(name)));
  await w.message("clear");
  assert.equal((await w.message("query")).entries, 0);
});

test("bad shared immutable checksums are never retained, mutable selectors are never intercepted", async () => {
  const w = worker();
  await w.message("retain", true);
  const invalid = await w.request(
    `/moly/asset-store/packages/${"ab".repeat(32)}.json`,
  );
  assert.equal(invalid.response.status, 200);
  await invalid.settle();
  assert.equal((await w.message("query")).entries, 0);
  for (const name of [
    "asset-packs.json",
    "channels/cn/stable.json",
    "channels/jp/stable.json",
  ])
    assert.equal(
      (await w.request("/moly/asset-store/" + name)).response,
      undefined,
    );
  assert.equal(w.network(), 1);
});
