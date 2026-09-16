import test from "node:test";
import assert from "node:assert/strict";
import { CacheAdmission, requiredResourceURLs } from "./cache-worker.mjs";

test("all responses beyond the old eight-request cap are eventually cached", async () => {
  const queue = new CacheAdmission(2, 12);
  const completed = [];
  await Promise.all(
    Array.from({ length: 40 }, (_, index) =>
      (async () => {
        const release = await queue.acquire(6);
        assert.equal(typeof release, "function");
        assert.ok(queue.count <= 2 && queue.bytes <= 12);
        await new Promise((resolve) => setTimeout(resolve, 1));
        completed.push(index);
        release();
        release();
      })(),
    ),
  );
  assert.equal(completed.length, 40);
  assert.equal(queue.count, 0);
  assert.equal(queue.bytes, 0);
});
test("clearing a cache invalidates waiting requests instead of restoring deleted data", async () => {
  const queue = new CacheAdmission(1, 12);
  const release = await queue.acquire(12);
  let generation = 0;
  const next = queue.acquire(6, () => generation === 0);
  generation++;
  release();
  assert.equal(await next, null);
  assert.equal(queue.bytes, 0);
});
test("oversized and invalid reservations cannot consume queue memory", async () => {
  const queue = new CacheAdmission(2, 12);
  for (const bytes of [-1, 0, Infinity, 13, NaN])
    assert.equal(await queue.acquire(bytes), null);
  assert.equal(queue.count, 0);
});

test("only the matching measured source can mark required cache entries", () => {
  const origin = "https://qa.test",
    url = origin + "/moly/snapshots/cn-1.2.3-a/assets/browser-base.json";
  const pack = {
    schemaVersion: 1,
    generator: "moly-browser-base-v1",
    region: "cn",
    gameVersion: "1.2.3",
    files: [{ path: "motion-library.glb" }],
  };
  assert.equal(requiredResourceURLs(pack, url, origin).length, 2);
  assert.deepEqual(
    requiredResourceURLs({ ...pack, region: "jp" }, url, origin),
    [],
  );
  assert.deepEqual(
    requiredResourceURLs(
      { ...pack, files: [{ path: "../jp/secret" }] },
      url,
      origin,
    ),
    [],
  );
});
