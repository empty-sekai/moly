import test from "node:test";
import assert from "node:assert/strict";
import {
  resourceIdentity,
  isCacheMessage,
  RESOURCE_PREFIX,
} from "./cache-worker.mjs";
const origin = "https://moesekai.test";

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
