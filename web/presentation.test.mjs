import test from "node:test";
import assert from "node:assert/strict";
import {
  assetUrl,
  validAssetBase,
  initialSelection,
  pageSummary,
  sourceLabel,
  safeColor,
} from "./presentation.mjs";
import { selectRenderer, holdSettingsWriter } from "./boot.mjs";
import { snapshotUrl } from "./snapshots.mjs";

test("asset images stay inside the configured snapshot", () => {
  assert.equal(
    assetUrl("/snapshots/jp/", "fixture-thumbnails/textures/chair.png"),
    "/snapshots/jp/fixture-thumbnails/textures/chair.png",
  );
  for (const value of [
    "../secrets",
    "https://other/image",
    "//other/image",
    "x\\x",
    "%2e%2e/file",
    "/outside",
    "x?query",
    "x#hash",
    "a//b",
    "a/./b",
    "",
  ])
    assert.equal(assetUrl("/assets/", value), null, value);
});
test("deployment base supports a feature subdirectory without URL references", () => {
  assert.equal(validAssetBase("/mysekai/experience/assets/"), true);
  for (const base of [
    "https://example.com/",
    "//example/",
    "/a/../b/",
    "/a%20b/",
    "/assets",
    "/a?b/",
  ])
    assert.equal(validAssetBase(base), false, base);
});
test("deep links retain opaque content identities and bound search input", () => {
  assert.deepEqual(
    initialSelection(
      "https://example.test/experience/?tab=activities&content=activity%3Anotalk%3A12%3A89&q=%E6%91%A9%E6%89%98",
    ),
    { tab: "activities", key: "activity:notalk:12:89", query: "摩托" },
  );
  assert.equal(
    initialSelection("https://example.test/?tab=invalid").tab,
    "conversations",
  );
  assert.equal(
    initialSelection("https://example.test/?q=" + "x".repeat(500)).query.length,
    200,
  );
});
test("page ranges and region labels do not imply cross-region data", () => {
  assert.equal(pageSummary(25, 1, 24), "25–25 / 25");
  assert.equal(pageSummary(0, 0, 24), "0 项");
  assert.equal(sourceLabel("jp", "6.8.1"), "日服 · 6.8.1");
  assert.equal(safeColor("url(https://external)"), null);
  assert.equal(safeColor("#33aAbb"), "#33aAbb");
});
test("automatic renderer fallback releases its probe context", async () => {
  let lost = false;
  const result = await selectRenderer("auto", {
    navigator: { gpu: { requestAdapter: async () => null } },
    document: {
      createElement: () => ({
        getContext: () => ({
          getExtension: () => ({
            loseContext() {
              lost = true;
            },
          }),
        }),
      }),
    },
  });
  assert.equal(result.backend, "webgl2");
  assert.equal(lost, true);
});
test("writer failure keeps the page read-only", async () => {
  assert.equal((await holdSettingsWriter(null)).writable, false);
  assert.equal(
    (
      await holdSettingsWriter({
        request: () => Promise.reject(new Error("denied")),
      })
    ).writable,
    false,
  );
});

test("region switching drops snapshot-bound IDs while preserving the experience configuration", () => {
  const url = new URL(
    snapshotUrl(
      "https://example.test/moly/?assets=/cn/&tab=activities&content=activity:notalk:1:1&fixture=1&character=1&q=chair&page=2&packs=1&player_uid=123&renderer=webgl2&embed=1&theme=dark",
      "/jp/",
    ),
  );
  for (const key of [
    "content",
    "fixture",
    "character",
    "q",
    "page",
    "packs",
    "player_uid",
  ])
    assert.equal(url.searchParams.has(key), false, key);
  assert.equal(url.searchParams.get("assets"), "/jp/");
  assert.equal(url.searchParams.get("tab"), "activities");
  assert.equal(url.searchParams.get("renderer"), "webgl2");
  assert.equal(url.searchParams.get("embed"), "1");
  assert.equal(url.searchParams.get("theme"), "dark");
  assert.throws(() => snapshotUrl(url, "https://other/"));
});
