import test from "node:test";
import assert from "node:assert/strict";
import {
  resourceIdentity,
  isCacheMessage,
  RESOURCE_PREFIX,
  verifiedSharedBody,
  requiredResourceURLs,
  clientResourceOrigin,
  activeResourceRoots,
  isActiveRequiredResource,
  VisitResourceCache,
} from "./cache-worker.mjs";
import { createHash } from "node:crypto";
const origin = "https://moesekai.test";

test("visit memory has a bounded LRU lifetime and never uses CacheStorage", async () => {
  const visit = new VisitResourceCache(8);
  visit.put("a", new Uint8Array([1, 2, 3, 4]), { "Content-Type": "audio/ogg" });
  visit.put("b", new Uint8Array([5, 6, 7, 8]), {});
  assert.equal((await visit.get("a").arrayBuffer()).byteLength, 4);
  visit.put("c", new Uint8Array([9, 10, 11, 12]), {});
  assert.equal(visit.get("b"), null);
  assert.equal(visit.get("a").headers.get("X-Moly-Cache"), "visit");
  visit.clear();
  assert.equal(visit.bytes, 0);
  assert.equal(visit.get("a"), null);
});

test("only open stage versions protect engines and the selected base resources", () => {
  const cdn = "https://cdn.example";
  const base = cdn + "/moly/snapshots/cn-6.0.0-current/assets/";
  const oldBase = cdn + "/moly/snapshots/cn-6.0.0-old/assets/";
  const clients = [{ url: origin + "/moly/releases/current/stage.html?resource_origin=" + encodeURIComponent(cdn) + "&assets=" + encodeURIComponent(base) }];
  const required = new Set([base + "base.json", oldBase + "base.json"]);
  const active = activeResourceRoots(clients, origin);
  assert.ok(isActiveRequiredResource(cdn + "/moly/releases/current/pkg/webgpu/moly-app_bg.wasm", required, active));
  assert.ok(isActiveRequiredResource(base + "base.json", required, active));
  assert.equal(isActiveRequiredResource(base + "optional.ogg", required, active), false);
  assert.equal(isActiveRequiredResource(cdn + "/moly/releases/old/pkg/webgpu/moly-app_bg.wasm", required, active), false);
  assert.equal(isActiveRequiredResource(oldBase + "base.json", required, active), false);
  assert.equal(isActiveRequiredResource(base + "base.json", required, activeResourceRoots([], origin)), false);
  const sharedRoot = cdn + "/moly/asset-store/";
  const blob = sharedRoot + "blobs/aa/" + "a".repeat(64) + ".bin";
  const packed = activeResourceRoots([{ url: origin + "/moly/releases/current/stage.html?packs=1&resource_origin=" + encodeURIComponent(cdn) + "&assets=" + encodeURIComponent(sharedRoot) }], origin);
  assert.ok(isActiveRequiredResource(blob, new Set([blob]), packed));
  assert.equal(isActiveRequiredResource(blob, new Set([blob]), active), false);
});

test("CDN retention preserves immutable identities and excludes private routes", () => {
  for (const cdn of ["https://assets-one.example", "https://cdn-two.example:8443"]) {
  const descriptor = cdn + "/moly/snapshots/cn-6.0.0-test/assets/browser-base.json";
  const client = origin + "/moly/releases/stage-a/stage.html?resource_origin=" + encodeURIComponent(cdn);
  assert.equal(clientResourceOrigin(client, origin), cdn);
  assert.ok(resourceIdentity(descriptor, origin, clientResourceOrigin(client, origin)));
  assert.equal(resourceIdentity(descriptor, origin), null);
  assert.equal(resourceIdentity(descriptor, origin, "https://unselected.example"), null);
  assert.deepEqual(requiredResourceURLs({
    schemaVersion: 1, generator: "moly-browser-base-v1", region: "cn", gameVersion: "6.0.0",
    files: [{ path: "audio/loop.json" }],
  }, descriptor, origin, cdn), [descriptor, cdn + "/moly/snapshots/cn-6.0.0-test/assets/audio/loop.json"]);
  for (const url of [cdn + "/api/player", cdn + "/moly/manifest.json", cdn + ".evil.test/moly/asset-store/"])
    assert.equal(resourceIdentity(url, origin, cdn), null);
  }
});

test("only a same-origin stage can select a cache resource origin", () => {
  for (const client of [
    "https://other.test/moly/releases/a/stage.html?resource_origin=https://cdn.example",
    origin + "/api/player?resource_origin=https://cdn.example",
    origin + "/moly/releases/a/stage.html?resource_origin=http://cdn.example",
    origin + "/moly/releases/a/stage.html?resource_origin=https://user@cdn.example",
    origin + "/moly/releases/a/stage.html?resource_origin=https://cdn.example/private/",
    origin + "/moly/releases/a/stage.html",
  ]) assert.equal(clientResourceOrigin(client, origin), null);
});

test("resource retention keys preserve both release and snapshot identities", () => {
  const cn = resourceIdentity(
    "/moly/snapshots/cn-test/assets/fixture-models/model.glb",
    origin,
  );
  const jp = resourceIdentity(
    "/moly/snapshots/jp-6.8.1-test/assets/fixture-models/model.glb",
    origin,
  );
  assert.notEqual(cn.cache, jp.cache);
  assert.ok(cn.cache.startsWith(RESOURCE_PREFIX));
  assert.equal(
    resourceIdentity(
      "/moly/releases/stage-a/pkg/webgpu/moly-app_bg.wasm",
      origin,
    ).cache,
    RESOURCE_PREFIX + "r-stage-a",
  );
});
test("site, credentials, mutable manifests, query variants and other origins are excluded", () => {
  for (const path of [
    "/",
    "/api/profile",
    "/mysekai/8/",
    "/moly/manifest.json",
    "/moly/cache-worker.mjs",
    "/moly/releases/stage-a/.env",
    "/moly/snapshots/cn-a/assets/a%2fb.json",
    "/moly/snapshots/cn-a/assets/a%5cb.json",
    "/moly/snapshots/cn-a/assets/a.json?source=jp",
    "https://other.test/moly/releases/stage-a/module.mjs",
  ])
    assert.equal(resourceIdentity(path, origin), null, path);
});
test("cache protocol exposes bounded intentions, never arbitrary purge/fetch commands", () => {
  const base = { source: "moly-cache-host", schemaVersion: 1 };
  assert.ok(isCacheMessage({ ...base, type: "query" }));
  assert.ok(isCacheMessage({ ...base, type: "clear" }));
  assert.ok(isCacheMessage({ ...base, type: "retain", enabled: false }));
  for (const payload of [
    { ...base, type: "retain", enabled: "true" },
    { ...base, type: "fetch", url: "/api" },
    { ...base, type: "query", schemaVersion: 2 },
    { ...base, type: "query", source: "other" },
    null,
  ])
    assert.ok(!isCacheMessage(payload));
});

test("shared cache stream validation enforces size while reading and verifies SHA-256", async () => {
  const hash = createHash("sha256").update("resource").digest("hex");
  assert.equal(
    new TextDecoder().decode(
      await verifiedSharedBody(new Response("resource"), 8, hash),
    ),
    "resource",
  );
  await assert.rejects(
    verifiedSharedBody(new Response("resource"), 9, hash),
    /length mismatch/,
  );
  await assert.rejects(
    verifiedSharedBody(new Response("resource"), 8, "ab".repeat(32)),
    /checksum/,
  );
  let pulls = 0,
    cancelled = false;
  const endless = new ReadableStream(
    {
      pull(controller) {
        pulls++;
        controller.enqueue(new Uint8Array(8));
      },
      cancel() {
        cancelled = true;
      },
    },
    { highWaterMark: 0 },
  );
  await assert.rejects(
    verifiedSharedBody(new Response(endless), 8, hash),
    /exceeded/,
  );
  assert.equal(pulls, 2);
  assert.equal(cancelled, true);
  assert.equal(
    resourceIdentity(`/moly/asset-store/packages/${hash}.json`, origin).maximum,
    16 * 1048576,
  );
});
