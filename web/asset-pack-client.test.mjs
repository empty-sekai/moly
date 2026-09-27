import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import {
  PackClient,
  packDigest,
  canonicalPackBytes,
  applyPackSelection,
  packedReadBytes,
} from "./asset-pack-client.mjs";
import { resourceIdentity } from "./cache-worker.mjs";
import { warmBaseResources } from "./base-resources.mjs";
import { snapshotUrl } from "./snapshots.mjs";

// Golden bytes were produced by the Python packer, not this JavaScript reader.
const golden = JSON.parse(
  fs.readFileSync(new URL("./fixtures/asset-pack-v2.json", import.meta.url)),
);
const files = () =>
  new Map(
    Object.entries(golden.files).map(([path, encoded]) => [
      path,
      Buffer.from(encoded, "base64"),
    ]),
  );
function client(
  region = "cn",
  { legacy = false, documents = files(), transform, pinned = true } = {},
) {
  const requests = [];
  const fetchImpl = async (input, options = {}) => {
    const key = new URL(input).pathname
      .slice(1)
      .split("/")
      .map(decodeURIComponent)
      .join("/");
    requests.push({ key, options });
    let bytes = documents.get(key);
    if (transform) bytes = transform(key, bytes);
    return bytes === undefined
      ? new Response(null, { status: 404 })
      : new Response(bytes, {
          headers: { "Content-Length": String(bytes.byteLength) },
        });
  };
  const id = legacy
    ? golden.index.legacy_catalog_sha256
    : golden.index.catalogs[region].id;
  return {
    reader: new PackClient(
      `https://pack.test/${legacy ? "legacy" : "store"}/`,
      pinned ? id : null,
      { fetchImpl },
    ),
    requests,
    fetchImpl,
    documents,
    id,
  };
}

test("Python-produced CN/JP/TW immutable packages resolve every declared logical asset", async () => {
  for (const region of ["cn", "jp", "tw"]) {
    const { reader } = client(region);
    assert.equal((await reader.catalog()).region, region);
    for (const [path, expected] of Object.entries(
      golden.index.expected[region],
    )) {
      const bytes = await reader.read(path);
      assert.equal(bytes.byteLength, expected.bytes, `${region}: ${path}`);
      assert.equal(
        await packDigest(bytes),
        expected.sha256,
        `${region}: ${path}`,
      );
    }
  }
});

test("two releases share physical blob URLs while their pinned catalogs differ", async () => {
  const cn = client("cn"),
    jp = client("jp");
  assert.notEqual(cn.id, jp.id);
  for (const path of golden.index.shared_logical_paths) {
    assert.equal(
      (await cn.reader.resolve(path)).url,
      (await jp.reader.resolve(path)).url,
    );
  }
  assert.notEqual(
    (await cn.reader.resolve("mysekai-fixtures.json")).url,
    (await jp.reader.resolve("mysekai-fixtures.json")).url,
  );
});

test("legacy v1 packs require explicit offline migration instead of runtime compatibility", async () => {
  for (const pinned of [true, false]) {
    const { reader, requests } = client("cn", { legacy: true, pinned });
    await assert.rejects(
      reader.read("sd_104.glb"),
      /Legacy v1 packs require explicit offline migration/,
    );
    assert.equal(requests.length, 1);
  }
});

for (const damage of [
  "missing blob",
  "corrupt blob",
  "missing package",
  "corrupt package",
  "missing catalog",
  "corrupt catalog",
]) {
  test(`${damage} is explicit failure, never a loose-tree fallback`, async () => {
    let selected;
    const original = client("cn");
    const entry = await original.reader.resolve("sd_104.glb");
    const catalogue = await original.reader.catalog();
    const row = catalogue.packages.find((row) =>
      row.paths.includes("sd_104.glb"),
    );
    if (damage.includes("blob")) selected = "store/blobs/" + entry.blob;
    else if (damage.includes("package")) selected = "store/" + row.manifest;
    else selected = `store/catalogs/${original.id}.json`;
    const { reader, requests } = client("cn", {
      transform: (key, bytes) =>
        key !== selected
          ? bytes
          : damage.startsWith("missing")
            ? undefined
            : Buffer.concat([bytes, Buffer.from(" ")]),
    });
    await assert.rejects(reader.read("sd_104.glb"));
    assert.ok(requests.every(({ key }) => !key.endsWith("/sd_104.glb")));
  });
}

test("semantic content identity is independently checked after a valid new representation address", async () => {
  const original = client("cn"),
    doc = await original.reader.catalog();
  const row = doc.packages.find((row) => row.paths.includes("sd_104.glb"));
  const manifest = JSON.parse(original.documents.get("store/" + row.manifest));
  manifest.entries[0].content_sha256 = "0".repeat(64);
  const encoded = canonicalPackBytes(manifest),
    manifestName = `packages/${await packDigest(encoded)}.json`;
  row.manifest = manifestName;
  original.documents.set("store/" + manifestName, encoded);
  original.documents.set("store/asset-packs.json", canonicalPackBytes(doc));
  await assert.rejects(
    client("cn", { documents: original.documents, pinned: false }).reader.read(
      "sd_104.glb",
    ),
    /Semantic content identity/,
  );
});

test("immutable canonical keys use Unicode codepoint order across BMP and astral text", () => {
  const bytes = canonicalPackBytes({
    "😀": "astral",
    "\ue000": "bmp",
    a: "ascii",
  });
  assert.equal(
    new TextDecoder().decode(bytes),
    '{"a":"ascii","\ue000":"bmp","😀":"astral"}\n',
  );
});

test("injected fetch is called as a function, like browser Window.fetch requires", async () => {
  const original = client("cn");
  const reader = new PackClient("https://pack.test/store/", original.id, {
    fetchImpl: function (...args) {
      assert.equal(this, undefined);
      return original.fetchImpl(...args);
    },
  });
  assert.equal((await reader.catalog()).region, "cn");
});

test("largest legal representations fit the prewarmer memory admission budget", () => {
  const mib = 1048576;
  assert.equal(
    packedReadBytes({
      codec: "identity",
      blob_bytes: 128 * mib,
      bytes: 128 * mib,
    }),
    256 * mib,
  );
  assert.equal(
    packedReadBytes({ codec: "gzip", blob_bytes: 128 * mib, bytes: 256 * mib }),
    512 * mib,
  );
  assert.equal(
    packedReadBytes({ codec: "identity", blob_bytes: 0, bytes: 0 }),
    0,
  );
});

test("dependency cycles, missing owners and oversized documents fail before asset IO", async () => {
  for (const damage of ["cycle", "owner", "document-size"]) {
    const bad = client("cn", {
      pinned: false,
      transform: (path, bytes) => {
        if (path !== "store/asset-packs.json") return bytes;
        if (damage === "document-size") return new Uint8Array(16 * 1048576 + 1);
        const doc = JSON.parse(bytes);
        if (damage === "cycle")
          doc.packages[0].dependencies = [doc.packages[0].id];
        else doc.packages[1].paths.push(doc.packages[0].paths[0]);
        return canonicalPackBytes(doc);
      },
    });
    await assert.rejects(bad.reader.read("sd_104.glb"));
    assert.equal(bad.requests.length, 1);
  }
});

test("failed documents can be retried; missing logical paths do not trigger unbounded IO", async () => {
  let broken = true;
  const scenario = client("cn", {
    transform: (key, bytes) =>
      broken && key.startsWith("store/packages/") ? undefined : bytes,
  });
  await assert.rejects(scenario.reader.read("sd_104.glb"));
  broken = false;
  assert.equal(
    await packDigest(await scenario.reader.read("sd_104.glb")),
    golden.index.expected.cn["sd_104.glb"].sha256,
  );
  const before = scenario.requests.length;
  await assert.rejects(
    scenario.reader.read("outside/not-present.png"),
    /moly:\/\//,
  );
  assert.equal(scenario.requests.length, before);
});

test("v2 schema rejects absent required release totals and unknown fields", async () => {
  for (const damage of [
    "missing-total",
    "unknown-release",
    "unknown-package",
  ]) {
    const bad = client("cn", {
      pinned: false,
      transform: (path, bytes) => {
        if (path !== "store/asset-packs.json") return bytes;
        const doc = JSON.parse(bytes);
        if (damage === "missing-total") delete doc.packages[0].download_bytes;
        if (damage === "unknown-release") doc.machine = "ignored release field";
        if (damage === "unknown-package") doc.packages[0].region = "cn";
        return canonicalPackBytes(doc);
      },
    });
    await assert.rejects(bad.reader.catalog(), /Invalid/);
  }
});

test("bounded base prewarming uses the same shared immutable URLs", async () => {
  const cn = client("cn"),
    progress = [];
  const result = await warmBaseResources(
    {
      assets: "https://pack.test/store/",
      region: "cn",
      version: "1.0.0",
      packs: true,
      assetCatalog: cn.id,
    },
    { fetchImpl: cn.fetchImpl, onProgress: (event) => progress.push(event) },
  );
  assert.ok(result);
  assert.ok(cn.requests.length > 3);
  assert.ok(
    cn.requests.every(({ key }) =>
      /^store\/(catalogs|packages|blobs)\//.test(key),
    ),
  );
  assert.ok(
    cn.requests.every(
      ({ options }) => options.headers?.["X-Moly-Required"] === "1",
    ),
  );
});

test("mutable selectors are excluded while different releases share one immutable cache", () => {
  const origin = "https://pack.test",
    digest = "ab".repeat(32);
  const a = resourceIdentity(
    `/moly/asset-store/blobs/ab/${digest}.bin`,
    origin,
  );
  const b = resourceIdentity(
    `/moly/asset-store/packages/${digest}.json`,
    origin,
  );
  assert.equal(a.cache, b.cache);
  assert.equal(a.sha256, digest);
  for (const path of [
    "asset-packs.json",
    "channels/cn/stable.json",
    "catalogs/not-a-hash.json",
    `blobs/ff/${digest}.bin`,
    `packages/${digest}.json?version=cn`,
    `packages/${digest}.json#bad`,
  ])
    assert.equal(
      resourceIdentity(`/moly/asset-store/${path}`, origin),
      null,
      path,
    );
});

test("snapshot switching keeps the same store but changes the pinned catalog, and clears stale selection", () => {
  const cn = golden.index.catalogs.cn.id,
    jp = golden.index.catalogs.jp.id;
  const initial = `https://pack.test/moly/?assets=/moly/asset-store/&packs=1&asset_catalog=${cn}&fixture=99`;
  const switched = new URL(
    snapshotUrl(initial, "/moly/asset-store/", {
      packs: true,
      assetCatalog: jp,
    }),
  );
  assert.equal(switched.searchParams.get("asset_catalog"), jp);
  assert.equal(switched.searchParams.get("packs"), "1");
  assert.equal(switched.searchParams.has("fixture"), false);
  const loose = new URL(snapshotUrl(switched.href, "/old-assets/"));
  assert.equal(loose.searchParams.has("asset_catalog"), false);
  assert.equal(loose.searchParams.has("packs"), false);
  assert.throws(() =>
    applyPackSelection(new URL(initial), { assetCatalog: "../bad" }),
  );
  assert.throws(() =>
    applyPackSelection(new URL(initial), { assetCatalog: cn, packs: false }),
  );
});
