import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { gunzipSync } from "node:zlib";
import {
  evidencePaths,
  prepareBrowserAssets,
} from "./prepare-browser-assets.mjs";
function trace(region = "jp") {
  return {
    schemaVersion: 1,
    failed: false,
    options: { region, origin: "https://site.test", assets: "/game/" },
    events: [
      { type: "catalog", data: { region, version: "6.8.1" } },
      {
        type: "base-resources",
        data: [
          { name: "https://site.test/game/motion-library.glb" },
          { name: "https://site.test/game/mysekai-fixtures.json" },
          { name: "https://outside.test/game/private.json" },
        ],
      },
    ],
  };
}
test("base plan uses only observed assets in the exact source directory", () => {
  assert.deepEqual(evidencePaths(trace()), [
    "motion-library.glb",
    "mysekai-fixtures.json",
  ]);
  assert.throws(() => evidencePaths({ ...trace(), failed: true }));
  const bad = trace();
  bad.events[1].data.push({
    name: "https://site.test/game/folder/%2e%2e%2fsecret.json",
  });
  assert.throws(() => evidencePaths(bad));
});
test("transport sidecars preserve sources, verify repeats and reject wrong provenance", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-transport-"));
  try {
    const raw = Buffer.alloc(100000, 7);
    fs.writeFileSync(path.join(root, "motion-library.glb"), raw);
    fs.writeFileSync(
      path.join(root, "mysekai-fixtures.json"),
      JSON.stringify({ region: "jp", gameVersion: "6.8.1" }),
    );
    const first = prepareBrowserAssets(root, trace());
    assert.ok(first.downloadBytes < first.decodedBytes);
    assert.deepEqual(
      fs.readFileSync(path.join(root, "motion-library.glb")),
      raw,
    );
    assert.deepEqual(
      gunzipSync(fs.readFileSync(path.join(root, "motion-library.glb.gz"))),
      raw,
    );
    assert.deepEqual(prepareBrowserAssets(root, trace()), first);
    assert.throws(() => prepareBrowserAssets(root, trace("cn")));
    fs.writeFileSync(
      path.join(root, "motion-library.glb"),
      Buffer.alloc(raw.length, 8),
    );
    assert.throws(() => prepareBrowserAssets(root, trace()), /stale/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("observed decoded shader pixels remain in the browser resource boundary", () => {
  const evidence = trace();
  const texture = "phenomena/shader-textures/" + "a".repeat(64) + ".rgba8";
  evidence.events[1].data.push({ name: `https://site.test/game/${texture}` });
  assert.ok(evidencePaths(evidence).includes(texture));
  for (const extension of ["glsl", "wgsl", "spv"]) {
    const shader = "phenomena/shaders/compiled/" + "b".repeat(64) + "." + extension;
    evidence.events[1].data.push({ name: `https://site.test/game/${shader}` });
    assert.ok(evidencePaths(evidence).includes(shader));
  }
});
