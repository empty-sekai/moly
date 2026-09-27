import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { createHash } from "node:crypto";
import {
  safeResourcePath,
  createResourceIndex,
  validateResourceIndex,
  resolveResourceSource,
} from "./resource-sources.mjs";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
async function fixture() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "moly-source-"));
  for (const [file, value] of Object.entries({
    "mysekai-fixtures.json": { region: "cn", gameVersion: "6.0.0" },
    "manifest.json": { units: [] },
    "fixture-talks/talks.json": { talks: [] },
    "fixture-models/player-data.json": { region: "cn" },
  })) {
    const p = path.join(root, file);
    await fs.mkdir(path.dirname(p), { recursive: true });
    await fs.writeFile(p, JSON.stringify(value));
  }
  return root;
}
test("source paths reject traversal, URI injection, Windows aliases and hidden entries", () => {
  for (const p of [
    "../secret",
    "a/../../b",
    "/abs",
    "a\\b",
    "https:x",
    "a%2fb",
    "a?x",
    "a/#b",
    ".env",
    "CON.json",
    "a. ",
  ])
    assert.equal(safeResourcePath(p), false, p);
  assert.equal(
    safeResourcePath("ui/atlas/sprites/ScenarioAtlas/icon_pageForward_gn.png"),
    true,
  );
});
test("an explicit source cannot mix local and remote or cross regions", async () => {
  await assert.rejects(
    resolveResourceSource(
      { assets: "a", region: "cn", version: "6.0.0", remote: {} },
      { cacheRoot: "unused" },
    ),
    /explicit/,
  );
  assert.throws(
    () =>
      validateResourceIndex(
        {
          schemaVersion: 1,
          generator: "moly-runtime-source-v1",
          region: "jp",
          gameVersion: "6.0.0",
          files: [],
        },
        "cn",
        "6.0.0",
      ),
    /identity/,
  );
});
test("pinned converted runtime is materialized, reused and rejects altered bytes", async () => {
  const root = await fixture();
  const cache = await fs.mkdtemp(path.join(os.tmpdir(), "moly-cache-"));
  try {
    const { index, indexSha256 } = await createResourceIndex(
      root,
      "cn",
      "6.0.0",
    );
    let reads = 0;
    let tamper = false;
    const fetchImpl = async (url) => {
      reads++;
      const name = new URL(url).pathname.replace("/runtime/", "");
      let bytes = await fs.readFile(path.join(root, name));
      if (tamper && name === "manifest.json") bytes = Buffer.from("corrupted");
      return new Response(bytes, {
        headers: { "content-length": String(bytes.length) },
      });
    };
    const source = {
      region: "cn",
      version: "6.0.0",
      remote: {
        kind: "moesekai-asset",
        baseUrl: "https://assets.example/runtime/",
        indexSha256,
      },
    };
    const first = await resolveResourceSource(source, {
      cacheRoot: cache,
      fetchImpl,
    });
    assert.equal(first.provider, "moesekai-asset");
    assert.equal(reads, index.files.length + 1);
    assert.equal(
      hash(await fs.readFile(path.join(first.assets, "manifest.json"))),
      index.files.find((row) => row.path === "manifest.json").sha256,
    );
    await resolveResourceSource(source, { cacheRoot: cache, fetchImpl });
    assert.equal(reads, index.files.length + 2);
    await fs.writeFile(
      path.join(first.assets, "manifest.json"),
      "local corruption",
    );
    tamper = true;
    await assert.rejects(
      resolveResourceSource(source, { cacheRoot: cache, fetchImpl }),
      /wrong size|hash mismatch/,
    );
    await assert.rejects(
      resolveResourceSource(
        {
          ...source,
          remote: { ...source.remote, indexSha256: "0".repeat(64) },
        },
        { cacheRoot: cache, fetchImpl },
      ),
      /index hash mismatch/,
    );
    await assert.rejects(
      resolveResourceSource(
        {
          ...source,
          remote: { ...source.remote, baseUrl: "http://localhost/runtime/" },
        },
        { cacheRoot: cache, fetchImpl },
      ),
      /HTTPS/,
    );
  } finally {
    await fs.rm(root, { recursive: true, force: true });
    await fs.rm(cache, { recursive: true, force: true });
  }
});
