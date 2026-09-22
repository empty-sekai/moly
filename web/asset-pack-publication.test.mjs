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
import { COORDINATE_CONTRACT } from "./coordinate-contract.mjs";
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
function addCoordinateFixture(root, region) {
  const version = JSON.parse(fs.readFileSync(path.join(root, `catalog-${region}.json`))).version;
  const documents = {
    "source.json": { coordinateContract: COORDINATE_CONTRACT, source: { region, appVersion: version } },
    "fixture-models/index.json": { version: 3, coordinateContract: COORDINATE_CONTRACT, packages: {} },
    "fixture-attach/attach-points.json": { version: 2, coordinateContract: COORDINATE_CONTRACT, packages: {} },
  };
  const store = path.join(root, "store");
  const old = JSON.parse(fs.readFileSync(path.join(store, "catalogs", golden.index.catalogs[region].id + ".json")));
  const entries = [];
  for (const [logical, document] of Object.entries(documents)) {
    const bytes = canonicalPackBytes(document), digest = sha(bytes), blob = `${digest.slice(0,2)}/${digest}.bin`;
    const sourceFile = path.join(root, `source-${region}`, logical);
    fs.mkdirSync(path.dirname(sourceFile), { recursive: true }); fs.writeFileSync(sourceFile, bytes);
    fs.mkdirSync(path.join(store, "blobs", digest.slice(0,2)), { recursive: true }); fs.writeFileSync(path.join(store, "blobs", blob), bytes);
    entries.push({ path: logical, blob, blob_sha256: digest, bytes: bytes.length, blob_bytes: bytes.length, content_sha256: digest, codec: "identity", http_encoding: "identity", xf: null });
  }
  entries.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  const first = JSON.parse(fs.readFileSync(path.join(store, old.packages[0].manifest)));
  const total = entries.reduce((sum, row) => sum + row.bytes, 0);
  const manifest = { ...first, entries, download_bytes: total, resident_bytes: total, content_bytes: total, logical_bytes: total, transforms: {} };
  manifest.content_id = sha(canonicalPackBytes({ schema: "moly-asset-content/1", transforms: {}, entries: entries.map(({path,bytes,content_sha256,xf}) => ({path,bytes,content_sha256,xf})) }));
  const encoded = canonicalPackBytes(manifest), name = `packages/${sha(encoded)}.json`;
  fs.writeFileSync(path.join(store, name), encoded);
  old.packages.push({ ...old.packages[0], id: "common/coordinates", kind: "common", manifest: name, content_id: manifest.content_id,
    dependencies: [], paths: entries.map(row => row.path), download_bytes: total, content_bytes: total });
  const catalog = canonicalPackBytes(old), id = sha(catalog);
  fs.writeFileSync(path.join(store, "catalogs", `${id}.json`), catalog);
  return id;
}
function canonicalTestStore(root, region, catalogId) {
  const source = path.join(root, `source-${region}`), store = path.join(root, "store");
  const catalog = JSON.parse(fs.readFileSync(path.join(store,"catalogs",catalogId+".json")));
  const model = {asset:{version:"2.0",extras:{coordinateContract:COORDINATE_CONTRACT}},scenes:[{nodes:[0]}],nodes:[{extras:{coordinateContract:COORDINATE_CONTRACT}}]};
  let document=Buffer.from(JSON.stringify(model));document=Buffer.concat([document,Buffer.alloc((4-document.length%4)%4,32)]);
  const header=Buffer.alloc(20);header.write("glTF");header.writeUInt32LE(2,4);header.writeUInt32LE(20+document.length,8);header.writeUInt32LE(document.length,12);header.write("JSON",16);
  const bytes=Buffer.concat([header,document]);
  for(const row of catalog.packages)for(const name of row.paths)if(name.endsWith(".glb"))fs.writeFileSync(path.join(source,name),bytes);
  const basePath=path.join(source,"browser-base.json"),base=JSON.parse(fs.readFileSync(basePath));
  for(const row of base.files){const data=fs.readFileSync(path.join(source,row.path));row.sha256=sha(data);row.decodedBytes=data.length;row.downloadBytes=data.length;row.encoding="identity";}
  base.decodedBytes=base.files.reduce((n,row)=>n+row.decodedBytes,0);base.downloadBytes=base.decodedBytes;fs.writeFileSync(basePath,JSON.stringify(base));
  for(const row of catalog.packages){
    const entries=row.paths.map(name=>{const data=fs.readFileSync(path.join(source,name)),digest=sha(data),blob=`${digest.slice(0,2)}/${digest}.bin`;fs.mkdirSync(path.join(store,"blobs",digest.slice(0,2)),{recursive:true});fs.writeFileSync(path.join(store,"blobs",blob),data);return {path:name,blob,blob_sha256:digest,bytes:data.length,blob_bytes:data.length,content_sha256:digest,codec:"identity",http_encoding:"identity",xf:null};}).sort((a,b)=>a.path<b.path?-1:a.path>b.path?1:0);
    const total=entries.reduce((n,e)=>n+e.bytes,0),manifest={schema:"moly-asset-manifest/2",entries,download_bytes:total,resident_bytes:total,content_bytes:total,logical_bytes:total,transforms:{},encoders:{}};
    manifest.content_id=sha(canonicalPackBytes({schema:"moly-asset-content/1",transforms:{},entries:entries.map(({path,bytes,content_sha256,xf})=>({path,bytes,content_sha256,xf}))}));
    const encoded=canonicalPackBytes(manifest);row.manifest=`packages/${sha(encoded)}.json`;row.content_id=manifest.content_id;row.download_bytes=total;row.content_bytes=total;fs.writeFileSync(path.join(store,row.manifest),encoded);
  }
  const encoded=canonicalPackBytes(catalog),id=sha(encoded);fs.writeFileSync(path.join(store,"catalogs",id+".json"),encoded);return id;
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

test("latest stage publisher rejects legacy packed model fixtures before publishing a selector", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-pack-stage-"));
  try {
    extract(root);
    const work = workspace(root),
      output = path.join(root, "publication");
    const catalogIds = Object.fromEntries(["cn", "jp", "tw"].map(region => [region, addCoordinateFixture(root, region)]));
    fs.mkdirSync(output);
    fs.cpSync(path.join(root, "store"), path.join(output, "asset-store"), {
      recursive: true,
    });
    const sources = ["cn", "jp", "tw"].map((region) => ({
      region,
      assets: path.join(root, `source-${region}`),
      catalog: path.join(root, `catalog-${region}.json`),
      assetCatalog: catalogIds[region],
    }));
    await assert.rejects(publish({ workspace: work, output, sources }), /GLB asset: coordinate contract/);
    assert.equal(fs.existsSync(path.join(output, "manifest.json")), false);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("stage publisher pins canonical packed metadata and preserves the previous selector on failure", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-pack-canonical-stage-"));
  try {
    extract(root);
    // Rebuild only this temporary synthetic publication corpus. The immutable
    // legacy golden fixture remains unchanged and has a rejection test above.
    const catalogIds = Object.fromEntries(["cn", "jp", "tw"].map(region => [region, canonicalTestStore(root,region,addCoordinateFixture(root,region))]));
    const work=workspace(root), output=path.join(root,"publication");fs.mkdirSync(output);fs.cpSync(path.join(root,"store"),path.join(output,"asset-store"),{recursive:true});
    const sources=["cn","jp","tw"].map(region=>({region,assets:path.join(root,`source-${region}`),catalog:path.join(root,`catalog-${region}.json`),assetCatalog:catalogIds[region]}));
    await publish({workspace:work,output,sources});
    const first = fs.readFileSync(path.join(output, "manifest.json")),
      manifest = JSON.parse(first);
    assert.equal(manifest.snapshots.length, 3);
    for (const snapshot of manifest.snapshots) {
      assert.equal(snapshot.assets, "/moly/asset-store/");
      assert.equal(
        snapshot.assetCatalog,
        catalogIds[snapshot.region],
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
    const retained = JSON.parse(fs.readFileSync(path.join(output,"manifest.json")));
    const retainedPath = path.join(output,"snapshots",retained.snapshots[0].id,"snapshot.json");
    fs.writeFileSync(retainedPath,JSON.stringify(Object.fromEntries(Object.entries(retained.snapshots[0]).reverse())));
    await publish({workspace:work,output,reuseEngine:true,reuseSnapshots:true});
    const missing = JSON.parse(fs.readFileSync(retainedPath));delete missing.coordinateContract;fs.writeFileSync(retainedPath,JSON.stringify(missing));
    await assert.rejects(publish({workspace:work,output,reuseEngine:true,reuseSnapshots:true}),/coordinate contract/);
    fs.writeFileSync(retainedPath,JSON.stringify(retained.snapshots[0],null,2)+"\n");
    await publish({ workspace: work, output, sources });
    const catalog = JSON.parse(
      fs.readFileSync(
        path.join(
          output,
          "asset-store/catalogs",
          catalogIds.cn + ".json",
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
