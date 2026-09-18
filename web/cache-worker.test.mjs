import test from "node:test";
import assert from "node:assert/strict";
import {
  resourceIdentity,
  isCacheMessage,
  RESOURCE_PREFIX,
  verifiedSharedBody,
} from "./cache-worker.mjs";
import { createHash } from "node:crypto";
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
