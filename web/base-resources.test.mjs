import test from "node:test";
import assert from "node:assert/strict";
import { validateBasePack, warmBaseResources } from "./base-resources.mjs";
const options = {
  region: "cn",
  version: "1.2.3",
  assets: "https://qa.test/moly/snapshots/cn-1.2.3-a/assets/",
};
const pack = {
  schemaVersion: 1,
  generator: "moly-browser-base-v1",
  region: "cn",
  gameVersion: "1.2.3",
  decodedBytes: 3,
  files: [{ path: "first.json", sha256: "a".repeat(64), decodedBytes: 3 }],
};
test("base pack validates source, paths, sizes and totals without guessing", () => {
  assert.equal(validateBasePack(pack, options).length, 1);
  for (const value of [
    { ...pack, region: "jp" },
    { ...pack, gameVersion: "6.8.1" },
    { ...pack, decodedBytes: 4 },
    { ...pack, files: [{ ...pack.files[0], path: "../jp/first.json" }] },
  ])
    assert.throws(() => validateBasePack(value, options));
});
test("necessary files download automatically and report actual byte completion", async () => {
  const calls = [];
  const updates = [];
  const result = await warmBaseResources(options, {
    fetchImpl: async (url) => {
      calls.push(String(url));
      return new Response(
        String(url).endsWith("browser-base.json")
          ? JSON.stringify(pack)
          : "abc",
      );
    },
    onProgress: (value) => updates.push(value),
  });
  assert.equal(calls.length, 2);
  assert.deepEqual(result, { completed: 1, decodedBytes: 3 });
  assert.equal(updates[0].total, 1);
});
test("truncated necessary resource cannot be reported as ready", async () => {
  await assert.rejects(
    warmBaseResources(options, {
      fetchImpl: async (url) =>
        new Response(
          String(url).endsWith("browser-base.json")
            ? JSON.stringify(pack)
            : "a",
        ),
    }),
    /length mismatch/,
  );
});

for (const concurrency of [undefined, 3]) {
  test(`base transfers admit ${concurrency ?? 8} concurrent streams and complete queued resources`, async () => {
    const files = Array.from({ length: 19 }, (_, index) => ({
      path: `resource-${index}.bin`,
      sha256: "a".repeat(64),
      decodedBytes: 3,
    }));
    let active = 0,
      peak = 0;
    const updates = [];
    const result = await warmBaseResources(options, {
      ...(concurrency === undefined ? {} : { concurrency }),
      fetchImpl: async (url) => {
        if (String(url).endsWith("browser-base.json"))
          return new Response(
            JSON.stringify({ ...pack, files, decodedBytes: files.length * 3 }),
          );
        active++;
        peak = Math.max(peak, active);
        return new Response(
          new ReadableStream({
            start(controller) {
              setTimeout(() => {
                active--;
                controller.enqueue(new TextEncoder().encode("abc"));
                controller.close();
              }, 10);
            },
          }),
        );
      },
      onProgress: (update) => updates.push(update),
    });
    assert.equal(peak, concurrency ?? 8);
    assert.equal(active, 0);
    assert.deepEqual(result, {
      completed: files.length,
      decodedBytes: files.length * 3,
    });
    assert.deepEqual(
      updates.map((row) => row.completed),
      files.map((_, index) => index + 1),
    );
  });
}

test("invalid prewarm concurrency fails before issuing a request", async () => {
  for (const concurrency of [0, -1, 1.5, 9, NaN, "8", null]) {
    await assert.rejects(
      warmBaseResources(options, {
        concurrency,
        fetchImpl: () => {
          throw new Error("Unexpected network request");
        },
      }),
      /concurrency must be an integer from 1 to 8/,
    );
  }
});
