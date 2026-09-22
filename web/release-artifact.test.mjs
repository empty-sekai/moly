import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { stripDebugNames, splitCatalog, STAGE_FILES, verifyPortraitCoordinateMigration } from "./release-artifact.mjs";
import { sha256 } from "./build-source.mjs";
import { COORDINATE_CONTRACT } from "./coordinate-contract.mjs";

test("published stage includes every statically imported local module", () => {
  const shipped = new Set(STAGE_FILES);
  const root = path.dirname(fileURLToPath(import.meta.url));
  for (const name of STAGE_FILES.filter((file) => file.endsWith(".mjs"))) {
    const source = readFileSync(path.join(root, name), "utf8");
    for (const match of source.matchAll(/\bfrom\s+["']\.\/([^"']+\.mjs)["']/g))
      assert.ok(shipped.has(match[1]), `${name} imports unpublished ${match[1]}`);
  }
});

const header = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);
const custom = (name, value = []) =>
  Buffer.from([
    0,
    name.length + value.length + 1,
    name.length,
    ...Buffer.from(name),
    ...value,
  ]);

test("removes debug name sections without altering runtime or other custom bytes", async () => {
  const kept = custom("producers", [0]);
  const input = Buffer.concat([
    header,
    custom("name", [0]),
    kept,
    custom(".debug_info", [0]),
  ]);
  assert.deepEqual(stripDebugNames(input), Buffer.concat([header, kept]));
  assert.equal(WebAssembly.validate(stripDebugNames(input)), true);
});
test("rejects truncated lengths, invalid name bounds, and malformed headers", () => {
  for (const tail of [
    [0],
    [0, 128],
    [0, 3, 5, 1, 2],
    [0, 128, 128, 128, 128, 16],
    [1, 6, 0],
  ]) {
    assert.throws(() =>
      stripDebugNames(Buffer.concat([header, Buffer.from(tail)])),
    );
  }
  assert.throws(() => stripDebugNames(Buffer.from("not-wasm")));
  assert.deepEqual(stripDebugNames(header), header);
});
const entry = {
  key: "activity:notalk:12:1",
  title: "Source title",
  available: true,
  fixtureIds: [297],
  unitIds: [13],
  characters: [{ id: 13, name: "Source name" }],
  lines: [],
  description: "",
  fixtures: [{ id: 297, name: "Source fixture" }],
  related: [],
  presentation: {
    category: "activity",
    behavior: "one_shot",
    primaryAction: "play",
    textMode: "none",
  },
  reasonCode: null,
};
const raw = {
  schemaVersion: 1,
  ready: true,
  mode: "independent",
  region: "jp",
  version: "6.8.1",
  entries: [entry],
  characters: entry.characters,
};
test("catalogue transport preserves exact owner facts and splits only detail payload", () => {
  const result = splitCatalog(raw, "jp-6.8.1-test");
  const card = result.index.entries[0];
  assert.equal(card.available, entry.available);
  assert.deepEqual(card.presentation, entry.presentation);
  assert.deepEqual(card.fixtureIds, [297]);
  assert.equal("lines" in card, false);
  const detail = JSON.parse(result.files.get(card.detail));
  assert.deepEqual(detail, {
    schemaVersion: 1,
    snapshotId: "jp-6.8.1-test",
    entry,
  });
});
test("source identity and duplicate keys fail closed", () => {
  for (const change of [
    { region: "invalid-region" },
    { ready: false },
    { mode: "current" },
    { version: "../cn" },
    { entries: [entry, entry] },
    { entries: [{ ...entry, key: "../escape" }] },
  ]) {
    assert.throws(() => splitCatalog({ ...raw, ...change }, "jp-6.8.1-test"));
  }
});

test("all supported game regions preserve their release selection identity", () => {
  for (const region of ["cn", "jp", "tw", "en", "kr"])
    assert.equal(
      splitCatalog({ ...raw, region }, `${region}-6.8.1-test`).index.region,
      region,
    );
});

function characterGlb(document, binary = Buffer.from([1, 2, 3, 4])) {
  let json = Buffer.from(JSON.stringify(document));
  json = Buffer.concat([json, Buffer.alloc((4 - json.length % 4) % 4, 32)]);
  const head = Buffer.alloc(20), tail = Buffer.alloc(8);
  head.write("glTF"); head.writeUInt32LE(2, 4);
  head.writeUInt32LE(28 + json.length + binary.length, 8);
  head.writeUInt32LE(json.length, 12); head.write("JSON", 16);
  tail.writeUInt32LE(binary.length); tail.write("BIN\0", 4);
  return Buffer.concat([head, json, tail, binary]);
}

function migrationFixture() {
  const document = { asset: { version: "2.0", generator: "moly-root character extractor",
    extras: { coordinates: "unity x-axis reflected" } },
    scenes: [{ nodes: [0] }], nodes: [{ name: "Root", children: [1] },
      { name: "Hips", translation: [0, 0.5, 0], extras: { preserved: true } }] };
  const revised = structuredClone(document);
  const metadata = { coordinateContract: COORDINATE_CONTRACT, coordinateUnits: "source-unity-unit" };
  Object.assign(revised.asset.extras, metadata);
  revised.nodes[0].extras = { ...metadata };
  const originalBytes = characterGlb(document), currentBytes = characterGlb(revised);
  const tail = bytes => bytes.subarray(20 + bytes.readUInt32LE(12));
  const row = { path: "sd_101.glb", family: "character", generator: document.asset.generator,
    sourceSha256: sha256(originalBytes), outputSha256: sha256(currentBytes),
    originalJsonSha256: sha256(JSON.stringify(document)),
    outputJsonWithoutAddedMetadataSha256: sha256(JSON.stringify(document)),
    binaryChunksSha256: sha256(tail(originalBytes)), outputBinaryChunksSha256: sha256(tail(currentBytes)),
    machineCheck: { originalJsonPreserved: true, binaryChunksPreserved: true, addedMetadata: [
      "asset.extras.coordinateContract", "asset.extras.coordinateUnits",
      "sceneRoots[].extras.coordinateContract", "sceneRoots[].extras.coordinateUnits",
    ] } };
  const receipt = { coordinateContract: COORDINATE_CONTRACT,
    baselineCommit: "f79162417628b26ec54155f698a4f197d27c6fcb",
    status: "complete", verifiedMetadataOnly: true, files: [row] };
  const photo = { model: row.path, modelSha256: row.sourceSha256, sha256: "capture-unchanged",
    capture: { mode: "head-only", camera: [0, 1, 2] } };
  return { document, revised, row, receipt, photo, originalBytes, currentBytes,
    args() { return { photo: this.photo, originalBytes: this.originalBytes,
      currentBytes: this.currentBytes, receiptBytes: Buffer.from(JSON.stringify(this.receipt)) }; } };
}

test("portrait coordinate migration preserves capture and verifies original JSON and binary", () => {
  const fixture = migrationFixture(), originalPhoto = structuredClone(fixture.photo);
  const migrated = verifyPortraitCoordinateMigration(fixture.args());
  assert.equal(migrated.modelSha256, sha256(fixture.currentBytes));
  assert.equal(migrated.originalModelSha256, originalPhoto.modelSha256);
  assert.deepEqual(migrated.capture, originalPhoto.capture);
  assert.equal(migrated.sha256, originalPhoto.sha256);
  assert.deepEqual(fixture.photo, originalPhoto);
  assert.equal(migrated.modelMigration.receiptSha256, sha256(fixture.args().receiptBytes));
  assert.deepEqual(verifyPortraitCoordinateMigration({ ...fixture.args(), photo: migrated }), migrated);
});

test("portrait migration cannot use a forged receipt to bless geometry, animation or arbitrary extras changes", () => {
  for (const mutate of [
    document => { document.nodes[1].translation[0] = 1; },
    document => { document.animations = [{ name: "new animation" }]; },
    document => { document.nodes[1].extras.coordinateContract = COORDINATE_CONTRACT; },
    document => { document.nodes[0].extras.unrelated = true; },
    document => { document.asset.extras.coordinates = "different basis"; },
  ]) {
    const fixture = migrationFixture();
    mutate(fixture.revised);
    fixture.currentBytes = characterGlb(fixture.revised);
    fixture.row.outputSha256 = sha256(fixture.currentBytes);
    assert.throws(() => verifyPortraitCoordinateMigration(fixture.args()), /Unverified/);
  }
  const fixture = migrationFixture();
  fixture.currentBytes = characterGlb(fixture.revised, Buffer.from([5, 6, 7, 8]));
  fixture.row.outputSha256 = sha256(fixture.currentBytes);
  fixture.row.outputBinaryChunksSha256 = fixture.row.binaryChunksSha256 =
    sha256(fixture.currentBytes.subarray(20 + fixture.currentBytes.readUInt32LE(12)));
  assert.throws(() => verifyPortraitCoordinateMigration(fixture.args()), /Unverified/);
});

test("portrait migration rejects missing proof, wrong captures, duplicate receipts and conflicting tags", () => {
  for (const mutate of [
    fixture => { fixture.photo.modelSha256 = "0".repeat(64); },
    fixture => { fixture.receipt.verifiedMetadataOnly = false; },
    fixture => { fixture.receipt.files.push(structuredClone(fixture.row)); },
    fixture => { fixture.row.family = "avatar-parts"; },
    fixture => { fixture.row.sourceSha256 = "0".repeat(64); },
    fixture => { fixture.row.outputSha256 = "0".repeat(64); },
    fixture => { fixture.row.machineCheck.addedMetadata.push("nodes[].translation"); },
    fixture => { fixture.row.outputJsonWithoutAddedMetadataSha256 = "0".repeat(64); },
    fixture => { fixture.photo.modelMigration = {}; },
    fixture => {
      fixture.revised.nodes[0].extras.coordinateContract = "other";
      fixture.currentBytes = characterGlb(fixture.revised);
      fixture.row.outputSha256 = sha256(fixture.currentBytes);
    },
  ]) {
    const fixture = migrationFixture();
    mutate(fixture);
    assert.throws(() => verifyPortraitCoordinateMigration(fixture.args()), /Unverified/);
  }
});
