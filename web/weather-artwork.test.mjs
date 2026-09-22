import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createWeatherArtwork, verifyWeatherPng } from "./weather-artwork.mjs";
import { presentWeather } from "./weather-presentation.mjs";
const options = {
  assets: "/assets/",
  baseUrl: "https://example.test/stage.html",
  packs: true,
  assetCatalog: "a".repeat(64),
};
const png = new Uint8Array(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j8a4AAAAASUVORK5CYII=",
    "base64",
  ),
);
const receipt = {
  file: "icons/key.png",
  sha256: createHash("sha256").update(png).digest("hex"),
  bytes: png.length,
  width: 1,
  height: 1,
};

test("packed icons verify both content-addressed reads and authored PNG receipts, once", async () => {
  const paths = [],
    revoked = [];
  const art = createWeatherArtwork(options, {
    createClient: (root, catalog, config) => {
      assert.equal(catalog, options.assetCatalog);
      assert.equal(config.required, true);
      return {
        read: async (path) => {
          paths.push(path);
          return png;
        },
      };
    },
    createObjectURL: () => "blob:verified",
    revokeObjectURL: (url) => revoked.push(url),
  });
  const [a, b] = await Promise.all([
    art.resolve(receipt.file, receipt),
    art.resolve(receipt.file, receipt),
  ]);
  assert.equal(a, b);
  assert.deepEqual(paths, ["phenomena/icons/key.png"]);
  art.dispose();
  art.dispose();
  assert.deepEqual(revoked, [a]);
  assert.equal(await art.resolve(receipt.file, receipt), null);
});

test("flat and packed presentation never fall back to an unverified path", () => {
  const source = {
    id: 99,
    name: "new source",
    options: [{ id: 99, name: "new source", icon: receipt.file }],
  };
  for (const packed of [false, true]) {
    const settings = { ...options, packed, locale: "en-US", region: "jp" };
    assert.equal(presentWeather(source, settings).iconUrl, null);
    assert.equal(
      presentWeather(source, {
        ...settings,
        iconUrls: new Map([[receipt.file, "blob:verified"]]),
      }).iconUrl,
      "blob:verified",
    );
  }
});

test("flat resources are fetched and hashed before publication", async () => {
  const fetched = [];
  let published = 0;
  const art = createWeatherArtwork(
    { ...options, packs: false, assetCatalog: null },
    {
      fetchImpl: async (url, settings) => {
        assert.equal(settings.credentials, "omit");
        fetched.push(url);
        return new Response(png);
      },
      createObjectURL: () => {
        published++;
        return "blob:flat-verified";
      },
      revokeObjectURL: () => {},
    },
  );
  assert.equal(await art.resolve(receipt.file, receipt), "blob:flat-verified");
  assert.equal(published, 1);
  assert.deepEqual(fetched, [
    "https://example.test/assets/phenomena/icons/key.png",
  ]);
  art.dispose();
});

test("absent provenance, wrong hash, size or source dimensions cannot publish pixels", async () => {
  let urls = 0;
  const art = createWeatherArtwork(options, {
    createClient: () => ({ read: async () => png }),
    createObjectURL: () => {
      urls++;
      return "blob:bad";
    },
  });
  await assert.rejects(art.resolve(receipt.file), /receipt/);
  await assert.rejects(
    art.resolve(receipt.file, { ...receipt, sha256: "0".repeat(64) }),
    /hash mismatch/,
  );
  await assert.rejects(
    verifyWeatherPng(png, { ...receipt, bytes: png.length + 1 }),
    /byte count/,
  );
  await assert.rejects(
    verifyWeatherPng(png, { ...receipt, width: 2 }),
    /dimensions/,
  );
  assert.equal(await art.resolve("../other.png", receipt), null);
  assert.equal(urls, 0);
  art.dispose();
});

test("network errors and oversize responses stay explicit instead of label fallbacks", async () => {
  for (const response of [
    () => new Response("not found", { status: 404 }),
    () => new Response(new Uint8Array(png.length + 1)),
  ]) {
    const art = createWeatherArtwork(
      { ...options, packs: false, assetCatalog: null },
      { fetchImpl: async () => response() },
    );
    await assert.rejects(
      art.resolve(receipt.file, receipt),
      /unavailable|exceeds/,
    );
    art.dispose();
  }
});

test("a late completion never allocates or leaks a retained URL after disposal", async () => {
  let finish,
    urls = 0;
  const art = createWeatherArtwork(options, {
    createClient: () => ({
      read: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    }),
    createObjectURL: () => {
      urls++;
      return "blob:x";
    },
  });
  const pending = art.resolve(receipt.file, receipt);
  art.dispose();
  finish(png);
  assert.equal(await pending, null);
  assert.equal(urls, 0);
});
