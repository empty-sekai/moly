import { parseArgs } from "node:util";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
const { values } = parseArgs({
  options: { origin: { type: "string" }, out: { type: "string" } },
});
if (!values.origin || !values.out) throw Error("Supply --origin and --out");
const origin = new URL(values.origin).origin,
  results = [];
async function get(relative, options = {}) {
  return fetch(new URL(relative, origin), { redirect: "manual", ...options });
}
const found = await get("/moly/manifest.json");
assert.equal(found.status, 200);
assert.equal(found.headers.get("Cache-Control"), "no-store");
const manifest = await found.json();
assert.equal(manifest.schemaVersion, 2);
assert.equal(manifest.release.contractVersion, 2);
for (const backend of ["webgpu", "webgl2"]) {
  const wasm = manifest.release.stage.replace(
    /stage\.html$/,
    `pkg/${backend}/moly-app_bg.wasm`,
  );
  const head = await get(wasm, {
    method: "HEAD",
    headers: { "Accept-Encoding": "gzip" },
  });
  assert.equal(head.status, 200);
  assert.equal(head.headers.get("Content-Type"), "application/wasm");
  assert.equal(head.headers.get("Content-Encoding"), "gzip");
  assert.equal(
    Number(head.headers.get("X-Moly-Decoded-Bytes")),
    manifest.release.engines[backend].decodedBytes,
  );
  assert.equal(
    Number(head.headers.get("Content-Length")),
    manifest.release.engines[backend].gzipBytes,
  );
  assert.match(head.headers.get("Cache-Control"), /immutable/);
  assert.equal(head.headers.get("X-Content-Type-Options"), "nosniff");
  const partial = await get(wasm, {
    headers: { Range: "bytes=0-7", "Accept-Encoding": "identity" },
  });
  assert.equal(partial.status, 206);
  assert.deepEqual(
    new Uint8Array(await partial.arrayBuffer()),
    Uint8Array.from([0, 97, 115, 109, 1, 0, 0, 0]),
  );
  const unchanged = await get(wasm, {
    method: "HEAD",
    headers: {
      "Accept-Encoding": "gzip",
      "If-None-Match": head.headers.get("ETag"),
    },
  });
  assert.equal(unchanged.status, 304);
  const br = await get(wasm, {
    method: "HEAD",
    headers: { "Accept-Encoding": "br, gzip;q=0.8" },
  });
  assert.equal(br.status, 200);
  assert.equal(br.headers.get("Content-Encoding"), "br");
  assert.equal(
    Number(br.headers.get("Content-Length")),
    manifest.release.engines[backend].brotliBytes,
  );
  assert.equal(
    manifest.release.engines[backend].downloadBytes,
    manifest.release.engines[backend].brotliBytes,
  );
  const identity = await get(wasm, {
    method: "HEAD",
    headers: { "Accept-Encoding": "br;q=0, gzip;q=0, identity;q=1" },
  });
  assert.equal(identity.status, 200);
  assert.equal(identity.headers.get("Content-Encoding"), null);
  assert.equal(
    Number(identity.headers.get("Content-Length")),
    manifest.release.engines[backend].decodedBytes,
  );
  results.push({
    backend,
    url: wasm,
    headers: Object.fromEntries(head.headers),
    brotli: Object.fromEntries(br.headers),
    identity: Object.fromEntries(identity.headers),
    range: partial.status,
    revalidated: unchanged.status,
  });
}
const module = await get(manifest.release.module);
assert.equal(module.status, 200);
assert.match(module.headers.get("Content-Type"), /javascript/);
await module.arrayBuffer();
const stage = await get(manifest.release.stage);
assert.equal(stage.status, 200);
assert.match(stage.headers.get("Content-Type"), /text\/html/);
assert.match(
  stage.headers.get("Content-Security-Policy"),
  /frame-ancestors 'self'/,
);
await stage.arrayBuffer();
for (const snapshot of manifest.snapshots) {
  assert.equal(snapshot.available, true);
  assert.equal(snapshot.provenance.assetPolicy, "immutable-readonly-mount");
  const master = await get(snapshot.assets + "mysekai-fixtures.json");
  assert.equal(master.status, 200);
  assert.match(master.headers.get("Cache-Control"), /immutable/);
  const identity = await master.json();
  assert.equal(identity.region, snapshot.region);
  assert.equal(identity.gameVersion, snapshot.version);
  const catalog = await get(snapshot.catalog);
  assert.equal(catalog.status, 200);
  const index = await catalog.json();
  assert.equal(index.snapshotId, snapshot.id);
  assert.equal(index.region, snapshot.region);
  assert.ok(index.entries.length > 1000);
  const absent = await get(snapshot.assets + "not-a-real-source-file.glb");
  assert.equal(absent.status, 404);
  assert.match(absent.headers.get("Cache-Control"), /no-store/);
  results.push({
    snapshot: snapshot.id,
    region: snapshot.region,
    version: snapshot.version,
    entries: index.entries.length,
    missingStatus: absent.status,
  });
}
for (const url of [
  "/moly/",
  "/moly/.env",
  "/moly/releases/not-a-release/pkg/x%2fy.wasm",
  "/moly/cache-worker.mjs/extra",
])
  assert.equal((await get(url)).status, 404, url);
assert.equal(
  (await get("/moly/manifest.json", { method: "POST" })).status,
  405,
);
const worker = await get("/moly/cache-worker.mjs");
assert.equal(worker.status, 200);
assert.equal(worker.headers.get("Service-Worker-Allowed"), "/moly/");
assert.equal(worker.headers.get("Cache-Control"), "no-cache");
await worker.arrayBuffer();
await fs.mkdir(values.out, { recursive: true });
await fs.writeFile(
  path.join(values.out, "production-serving.json"),
  JSON.stringify({ passed: true, origin, manifest, results }, null, 2),
);
console.log(
  JSON.stringify({
    passed: true,
    backends: 2,
    snapshots: manifest.snapshots.map((s) => s.id),
    checks: results,
  }),
);
