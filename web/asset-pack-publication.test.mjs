import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { verifyPublishedStore, storeChannels } from "./asset-pack-store.mjs";
import { publish } from "./release-artifact.mjs";
import { workspaceFingerprint } from "./build-source.mjs";
import { canonicalPackBytes } from "./asset-pack-client.mjs";
const here = path.dirname(fileURLToPath(import.meta.url));
const golden = JSON.parse(
  fs.readFileSync(new URL("./fixtures/asset-pack-v2.json", import.meta.url)),
);
const sha = (data) => createHash("sha256").update(data).digest("hex");
function extract(root) {
  for (const [relative, value] of Object.entries(golden.files)) {
    const file = path.join(root, relative);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, Buffer.from(value, "base64"));
  }
}
function uleb(value) {
  const result = [];
  do {
    let byte = value & 127;
    value >>>= 7;
    if (value) byte |= 128;
    result.push(byte);
  } while (value);
  return result;
}
function mockStageWasm() {
  // A valid minimal module to isolate publication tests, not a playable stage.
  const section = (id, bytes) => [id, ...uleb(bytes.length), ...bytes];
  const names = [
    "start_stage",
    "library_snapshot",
    "library_command",
    "library_catalog",
    "library_diagnostics",
  ];
  const exports = [
    names.length,
    ...names.flatMap((name) => [name.length, ...Buffer.from(name), 0, 0]),
  ];
  return Buffer.from([
    0,
    97,
    115,
    109,
    1,
    0,
    0,
    0,
    ...section(1, [1, 0x60, 0, 0]),
    ...section(3, [1, 0]),
    ...section(7, exports),
    ...section(10, [1, 2, 0, 0x0b]),
  ]);
}
function workspace(root) {
  const work = path.join(root, "workspace");
  fs.mkdirSync(path.join(work, "crates"), { recursive: true });
  fs.mkdirSync(path.join(work, "web"), { recursive: true });
  fs.writeFileSync(
    path.join(work, "crates/public-fixture.rs"),
    "// publication test input\n",
  );
  fs.writeFileSync(
    path.join(work, "Cargo.toml"),
    "# publication test workspace\n",
  );
  for (const name of fs.readdirSync(here))
    if (
      fs.statSync(path.join(here, name)).isFile() &&
      /\.(mjs|css|html|ts)$/.test(name)
    )
      fs.copyFileSync(path.join(here, name), path.join(work, "web", name));
  for (const backend of ["webgpu", "webgl2"]) {
    const dir = path.join(work, "web/pkg", backend);
    fs.mkdirSync(dir, { recursive: true });
    const files = {
      "moly-app.js": Buffer.from("export const fixtureOnly = true;\n"),
      "moly-app_bg.wasm": mockStageWasm(),
    };
    for (const [name, bytes] of Object.entries(files))
      fs.writeFileSync(path.join(dir, name), bytes);
    fs.writeFileSync(
      path.join(dir, "build.json"),
      JSON.stringify({
        schemaVersion: 1,
        backend,
        sourceFingerprint: workspaceFingerprint(work),
        files: Object.fromEntries(
          Object.entries(files).map(([name, bytes]) => [name, sha(bytes)]),
        ),
      }),
    );
  }
  return work;
}

test("published store verification checks every object and source artifact without copying binaries", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-pack-publication-"));
  try {
    extract(root);
    const selections = storeChannels(path.join(root, "store"));
    assert.deepEqual(
      selections.map((row) => row.region),
      ["cn", "jp", "tw"],
    );
    const checked = await verifyPublishedStore(
      path.join(root, "store"),
      golden.index.catalogs.cn.id,
      { region: "cn", version: "1.0.0", assets: path.join(root, "source-cn") },
    );
    assert.equal(
      checked.logicalFiles,
      Object.keys(golden.index.expected.cn).length,
    );
    fs.appendFileSync(
      path.join(root, "source-cn/ui/action-icon/Talk.png"),
      "changed",
    );
    await assert.rejects(
      verifyPublishedStore(
        path.join(root, "store"),
        golden.index.catalogs.cn.id,
        { assets: path.join(root, "source-cn") },
      ),
      /differs from pinned store/,
    );
    await assert.rejects(
      verifyPublishedStore(
        path.join(root, "store"),
        golden.index.catalogs.cn.id,
        { region: "jp" },
      ),
      /differ/,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("stage publisher pins three regions into one existing store and preserves the previous selector on failure", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-pack-stage-"));
  try {
    extract(root);
    const work = workspace(root),
      output = path.join(root, "publication");
    fs.mkdirSync(output);
    fs.cpSync(path.join(root, "store"), path.join(output, "asset-store"), {
      recursive: true,
    });
    const sources = ["cn", "jp", "tw"].map((region) => ({
      region,
      assets: path.join(root, `source-${region}`),
      catalog: path.join(root, `catalog-${region}.json`),
      assetCatalog: golden.index.catalogs[region].id,
    }));
    await publish({ workspace: work, output, sources });
    const first = fs.readFileSync(path.join(output, "manifest.json")),
      manifest = JSON.parse(first);
    assert.equal(manifest.snapshots.length, 3);
    for (const snapshot of manifest.snapshots) {
      assert.equal(snapshot.assets, "/moly/asset-store/");
      assert.equal(
        snapshot.assetCatalog,
        golden.index.catalogs[snapshot.region].id,
      );
      assert.equal(snapshot.packs, true);
      assert.deepEqual(
        JSON.parse(
          fs.readFileSync(
            path.join(output, "snapshots", snapshot.id, "snapshot.json"),
          ),
        ),
        snapshot,
        "historical descriptor preserves the exact pinned snapshot",
      );
      assert.equal(
        fs.existsSync(path.join(output, "snapshots", snapshot.id, "assets")),
        false,
      );
    }
    await publish({ workspace: work, output, sources });
    assert.deepEqual(
      fs.readFileSync(path.join(output, "manifest.json")),
      first,
    );
    const historical = manifest.snapshots.find(
      (snapshot) => snapshot.region === "jp",
    );
    const historicalPath = path.join(
      output,
      "snapshots",
      historical.id,
      "snapshot.json",
    );
    const historicalBytes = fs.readFileSync(historicalPath);
    await publish({ workspace: work, output, sources: [sources[0]] });
    assert.deepEqual(fs.readFileSync(historicalPath), historicalBytes);
    await publish({ workspace: work, output, sources });
    const catalog = JSON.parse(
      fs.readFileSync(
        path.join(
          output,
          "asset-store/catalogs",
          golden.index.catalogs.cn.id + ".json",
        ),
      ),
    );
    const packagePath = path.join(
      output,
      "asset-store",
      catalog.packages[0].manifest,
    );
    const raw = fs.readFileSync(packagePath);
    fs.writeFileSync(packagePath, Buffer.concat([raw, Buffer.from(" ")]));
    await assert.rejects(
      publish({ workspace: work, output, sources }),
      /content address mismatch/,
    );
    assert.deepEqual(
      fs.readFileSync(path.join(output, "manifest.json")),
      first,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("publication verifies every claimed decoded identity even when entries reuse one encoded blob", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-pack-alias-"));
  try {
    extract(root);
    const store = path.join(root, "store"),
      catalog = JSON.parse(
        fs.readFileSync(
          path.join(store, "catalogs", golden.index.catalogs.cn.id + ".json"),
        ),
      );
    const row = catalog.packages.find((row) =>
      row.paths.includes("ui/action-icon/Talk.png"),
    );
    const manifest = JSON.parse(
      fs.readFileSync(path.join(store, row.manifest)),
    );
    const source = manifest.entries.find(
      (entry) => entry.path === "ui/action-icon/Talk.png",
    );
    const alias = {
      ...source,
      path: "ui/action-icon/zz-invalid-alias.png",
      content_sha256: "0".repeat(64),
    };
    manifest.entries.push(alias);
    manifest.entries.sort((a, b) =>
      Buffer.compare(Buffer.from(a.path), Buffer.from(b.path)),
    );
    manifest.logical_bytes += alias.bytes;
    manifest.content_id = sha(
      canonicalPackBytes({
        schema: "moly-asset-content/1",
        transforms: manifest.transforms,
        entries: manifest.entries.map(
          ({ path, bytes, content_sha256, xf }) => ({
            path,
            bytes,
            content_sha256,
            xf,
          }),
        ),
      }),
    );
    const encoded = canonicalPackBytes(manifest);
    row.manifest = `packages/${sha(encoded)}.json`;
    row.content_id = manifest.content_id;
    row.paths.push(alias.path);
    fs.writeFileSync(path.join(store, row.manifest), encoded);
    const release = canonicalPackBytes(catalog),
      id = sha(release);
    fs.writeFileSync(path.join(store, "catalogs", id + ".json"), release);
    await assert.rejects(
      verifyPublishedStore(store, id),
      /conflicting content metadata/,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
