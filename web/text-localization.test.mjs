import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  loadTextSession,
  localizeEntry,
  validateTextSession,
  loadTextIndex,
} from "./text-localization.mjs";

const hash = (data) => createHash("sha256").update(data).digest("hex");
test("a pinned source survives translation publication changes and rejects changed line hashes", async () => {
  const saved = globalThis.fetch;
  const urls = new Map();
  const calls = [];
  const base = "https://example.test/text/",
    translated = "https://example.test/translations/";
  const put = (value, prefix = base) => {
    const bytes = JSON.stringify(value),
      sha256 = hash(bytes),
      path = `objects/${sha256}.json`;
    urls.set(prefix + path, bytes);
    return { path, sha256, bytes: Buffer.byteLength(bytes) };
  };
  const source = {
    schema: "moly-text-source/1",
    region: "jp",
    revision: "1".repeat(64),
    scripts: [
      {
        key: "sample",
        lines: [
          {
            id: "line",
            kind: "text",
            step: 2,
            text: "Original",
            sourceHash: "2".repeat(64),
          },
        ],
      },
    ],
  };
  const bucket = hash("sample").slice(0, 2);
  const sourceRoute = put({
    schema: "moly-text-routes/1",
    scripts: { sample: { source: put(source) } },
  });
  const index = {
    schema: "moly-text-runtime/1",
    region: "jp",
    revision: "3".repeat(64),
    routing: "sha256-utf8-prefix-2",
    buckets: { [bucket]: sourceRoute },
    searchBuckets: {
      [bucket[0]]: put({
        schema: "moly-text-source-search/1",
        scripts: { sample: [["line", "2".repeat(64), "text", "Original"]] },
      }),
    },
  };
  const root = put(index);
  const publish = (text, sourceHash = "2".repeat(64)) => {
    const overlay = put(
      {
        schema: "moly-text-overlay/1",
        sourceRegion: "jp",
        sourceRevision: "9".repeat(64),
        revision: hash(text),
        locale: "zh-CN",
        entries: [{ id: "line", sourceHash, source: "official_cn", text }],
      },
      translated,
    );
    const route = put(
      {
        schema: "moly-text-translation-routes/1",
        scripts: { sample: overlay },
      },
      translated,
    );
    const publication = JSON.stringify({
      schema: "moly-text-publication/1",
      region: "jp",
      locale: "zh-CN",
      revision: hash(text),
      routing: "sha256-utf8-prefix-2",
      buckets: { [bucket]: route },
      searchBuckets: {
        [bucket[0]]: put(
          {
            schema: "moly-text-translation-search/1",
            scripts: {
              sample: [["line", sourceHash, text, "official_cn"]],
            },
          },
          translated,
        ),
      },
    });
    urls.set(translated + "latest.json", publication);
    urls.set(translated + `revisions/${hash(text)}.json`, publication);
  };
  globalThis.fetch = async (url) => {
    calls.push(String(url));
    return new Response(urls.get(String(url)), {
      status: urls.has(String(url)) ? 200 : 404,
    });
  };
  try {
    const options = {
      source: { url: base + root.path, sha256: root.sha256, bytes: root.bytes },
      official: translated + "latest.json",
      region: "jp",
      locale: "zh-CN",
      scriptKey: "sample",
    };
    // The reference root is usually index.json, and child paths resolve from
    // the publication directory, not the content object's own subdirectory.
    urls.set(base + "index.json", urls.get(base + root.path));
    options.source.url = base + "index.json";
    const entry = {
      scriptKey: "sample",
      lines: [{ text: "Original", speaker: "", textStep: 2 }],
    };
    publish("译文一");
    const first = await loadTextSession(options);
    assert.equal(localizeEntry(entry, first).lines[0].text, "译文一");
    const archive = {
      ...options,
      official: translated + `revisions/${hash("译文一")}.json`,
    };
    assert.equal(
      localizeEntry(entry, await loadTextSession(archive)).lines[0].text,
      "译文一",
    );
    const search = await loadTextIndex(options);
    assert.equal(search.sample.title, "译文一");
    assert.match(search.sample.search, /译文一/);
    const sourceCalls = calls.filter((url) => url.startsWith(base)).length;
    publish("译文二");
    const second = await loadTextSession(options);
    assert.equal(localizeEntry(entry, second).lines[0].text, "译文二");
    assert.equal(localizeEntry(entry, first).lines[0].text, "译文一");
    assert.equal(
      calls.filter((url) => url.startsWith(base)).length,
      sourceCalls,
    );
    publish("Stale", "4".repeat(64));
    assert.equal(
      localizeEntry(entry, await loadTextSession(options)).lines[0].text,
      "Original",
    );
    assert.equal(entry.lines[0].text, "Original");
    // Match Rust/Go: BOM is not Unicode White_Space, while ordinary Unicode
    // whitespace does not override the source. Reading and search must agree.
    for (const [text, displayed] of [
      ["\uFEFF", "\uFEFF"],
      [" \n\u00A0\u1680\u2000\u2028\u2029\u202F\u205F\u3000", "Original"],
    ]) {
      publish(text);
      assert.equal(
        localizeEntry(entry, await loadTextSession(options)).lines[0].text,
        displayed,
      );
      assert.equal(
        (await loadTextIndex(options)).sample.search,
        `${displayed} Original`,
      );
    }
    publish("Still official");
    const fetchPublication = globalThis.fetch;
    globalThis.fetch = async (url, init) => {
      if (String(url).includes("unavailable"))
        throw new TypeError("Network error");
      return fetchPublication(url, init);
    };
    assert.equal(
      localizeEntry(
        entry,
        await loadTextSession({
          ...options,
          community: translated + "unavailable",
        }),
      ).lines[0].text,
      "Still official",
    );
    const invalid = JSON.parse(first.official);
    invalid.entries.push({ ...invalid.entries[0] });
    assert.throws(
      () =>
        validateTextSession({ ...first, official: JSON.stringify(invalid) }),
      /duplicate/,
    );
  } finally {
    globalThis.fetch = saved;
  }
});
