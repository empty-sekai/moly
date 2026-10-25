import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  displayCodepoints,
  selectTextFonts,
  validateFontCatalog,
} from "./text-fonts.mjs";
import { prepareTextFonts, validateTextSession } from "./text-localization.mjs";
const hash = (value) => createHash("sha256").update(value).digest("hex");
const base = "b".repeat(64),
  sourceHash = "c".repeat(64),
  revision = "d".repeat(64);
const font = new TextEncoder().encode("synthetic verified font bytes"),
  fontHash = hash(font);
const catalog = {
  schema: "moly-text-fonts/1",
  revision,
  baseFonts: { [base]: { coverage: [[32, 126]] } },
  fonts: [
    {
      path: `objects/${fontHash}.ttf`,
      sha256: fontHash,
      bytes: font.length,
      coverage: [[0x4e02, 0x4e02]],
      license: {
        path: `objects/${"e".repeat(64)}.txt`,
        sha256: "e".repeat(64),
        bytes: 12,
      },
    },
  ],
};
function session(official, community) {
  const overlay = (text, source, hash = sourceHash) =>
    JSON.stringify({
      schema: "moly-text-overlay/1",
      sourceRegion: "jp",
      sourceRevision,
      revision,
      locale: "zh-CN",
      entries: [{ id: "one", sourceHash: hash, text, source }],
    });
  const sourceRevision = "a".repeat(64);
  return {
    locale: "zh-CN",
    source: JSON.stringify({
      schema: "moly-text-source/1",
      region: "jp",
      revision: sourceRevision,
      scripts: [
        {
          key: "sample",
          lines: [
            { id: "one", sourceHash, kind: "text", step: 0, text: "Original" },
          ],
        },
      ],
    }),
    official: official ? overlay(official, "official_cn") : "",
    community: community ? overlay(community, "human") : "",
  };
}

test("font selection follows effective text, exact engine digest, and verified immutable bytes", async () => {
  const previous = globalThis.fetch,
    calls = [];
  globalThis.fetch = async (url) => {
    calls.push(String(url));
    return String(url).endsWith(".ttf")
      ? new Response(font)
      : new Response(JSON.stringify(catalog), {
          headers: { "content-type": "application/json" },
        });
  };
  try {
    const options = {
      catalogUrl: "https://fonts.test/fonts/latest.json",
      baseFontSha256: base,
    };
    const plain = await prepareTextFonts(
      session("Shown", "丂"),
      undefined,
      options,
    );
    assert.equal(plain.bytes.length, 0);
    assert.equal(calls.filter((url) => url.endsWith(".ttf")).length, 0);
    const translated = await prepareTextFonts(
      session(null, "<b>丂</b>"),
      undefined,
      options,
    );
    assert.deepEqual(translated.bytes, font);
    assert.equal(JSON.parse(translated.refs)[0].sha256, fontHash);
    assert(displayCodepoints(session(null, "<b>丂</b>")).includes(60));
    await assert.rejects(
      selectTextFonts(session(null, "☃"), options),
      /U\+2603/,
    );
    await assert.rejects(
      selectTextFonts(session("Shown"), {
        ...options,
        baseFontSha256: "f".repeat(64),
      }),
      /this engine font/,
    );
    globalThis.fetch = async () => new Response(new Uint8Array(font.length));
    await assert.rejects(
      prepareTextFonts(session(null, "丂"), undefined, options),
      /integrity mismatch/,
    );
    const abort = new AbortController();
    abort.abort();
    await assert.rejects(
      prepareTextFonts(session("Shown"), abort.signal, options),
      /abort/i,
    );
  } finally {
    globalThis.fetch = previous;
  }
});

test("font catalogue refuses overlapping coverage, escaped paths, and missing licences", () => {
  for (const mutate of [
    (doc) => doc.fonts[0].coverage.push([0x4e02, 0x4e03]),
    (doc) => (doc.fonts[0].path = "../font.ttf"),
    (doc) => delete doc.fonts[0].license,
  ]) {
    const doc = structuredClone(catalog);
    mutate(doc);
    assert.throws(() => validateFontCatalog(doc));
  }
});

test("font references admit loopback development only and refuse credentials or query", () => {
  const value = session("Shown");
  const font = {
    url: "http://127.0.0.1:8080/fonts/font.ttf",
    sha256: fontHash,
    bytes: 30,
  };
  assert.doesNotThrow(() => validateTextSession({ ...value, fonts: [font] }));
  for (const url of [
    "http://example.test/font.ttf",
    "https://user@example.test/font.ttf",
    "https://example.test/font.ttf?token=x",
    "https://example.test/font.ttf#x",
  ]) {
    assert.throws(() =>
      validateTextSession({ ...value, fonts: [{ ...font, url }] }),
    );
  }
});
