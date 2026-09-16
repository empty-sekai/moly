import test from "node:test";
import assert from "node:assert/strict";
import { stripDebugNames, splitCatalog } from "./release-artifact.mjs";

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
    { region: "en" },
    { ready: false },
    { mode: "current" },
    { version: "../cn" },
    { entries: [entry, entry] },
    { entries: [{ ...entry, key: "../escape" }] },
  ]) {
    assert.throws(() => splitCatalog({ ...raw, ...change }, "jp-6.8.1-test"));
  }
});
