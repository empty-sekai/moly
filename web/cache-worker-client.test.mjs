import test from "node:test";
import assert from "node:assert/strict";

test("worker retains each stage's configured CDN without authorizing other origins", async () => {
  const origin = "https://host.example";
  const first = "https://cdn-one.example";
  const second = "https://cdn-two.example:8443";
  const path = "/moly/snapshots/cn-6.0.0-test/assets/audio/loop.json";
  const stores = new Map(), requests = [], handlers = new Map();
  const cacheApi = {
    async open(name) {
      if (!stores.has(name)) stores.set(name, new Map());
      const rows = stores.get(name);
      const key = value => typeof value === "string" ? value : value.url;
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
  await (await cacheApi.open("moly-control-v1")).put(origin + "/moly/__retention", Response.json({ enabled: true }));
  class Scope {
    location = { origin };
    clients = {
      get: async id => ({ url: origin + "/moly/releases/stage-test/stage.html?resource_origin=" + encodeURIComponent(id === "first" ? first : second) }),
      matchAll: async () => [first, second].map(cdn => ({ url: origin + "/moly/releases/stage-test/stage.html?resource_origin=" + encodeURIComponent(cdn) })),
      claim: async () => {},
    };
    addEventListener(type, handler) { handlers.set(type, handler); }
    async skipWaiting() {}
  }
  const previous = Object.fromEntries(["self", "ServiceWorkerGlobalScope", "caches", "fetch"].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  Object.assign(globalThis, {
    ServiceWorkerGlobalScope: Scope,
    self: new Scope(),
    caches: cacheApi,
    fetch: async (request, options) => {
      requests.push({ url: request.url, options });
      return new Response("abc", { headers: { "Cache-Control": "public,max-age=31536000,immutable", "x-oss-meta-moly-decoded-bytes": "3" } });
    },
  });
  const dispatch = async (url, clientId) => {
    let response;
    const pending = [];
    handlers.get("fetch")({
      clientId,
      request: new Request(url),
      respondWith(work) { response = work; },
      waitUntil(work) { pending.push(work); },
    });
    const value = await response;
    await Promise.all(pending);
    return value;
  };
  try {
    await import("./cache-worker.mjs?configured-origin-worker-test");
    await dispatch(first + path, "first");
    await dispatch(second + path, "second");
    assert.equal(requests.length, 2);
    for (const request of requests) {
      assert.equal(request.options.credentials, "omit");
      assert.equal(request.options.redirect, "error");
    }
    await dispatch(first + path, "first");
    await dispatch(second + path, "second");
    assert.equal(requests.length, 2, "both configured origins reuse retained bytes");
    await dispatch(second + path, "first");
    assert.equal(requests.length, 3, "the first stage cannot read the second origin's retained copy");
    assert.equal(requests.at(-1).options, undefined, "unselected origin passes through without cache interception");
    assert.equal(await dispatch(second + "/api/player", "second"), undefined, "private routes are never intercepted");
  } finally {
    for (const [key, descriptor] of Object.entries(previous)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  }
});
