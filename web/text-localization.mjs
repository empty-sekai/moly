// Resource snapshots and translation publications have separate identities.
// This module is shared by the host's reading view and the stage adapter.
import { selectTextFonts } from "./text-fonts.mjs";
const HASH = /^[a-f0-9]{64}$/;
const LIMIT = 2 * 1024 * 1024;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const cache = new Map();
const REGIONS = new Set(["jp", "cn", "tw", "en", "kr"]);
const LOCALES = new Set(["zh-CN", "zh-TW", "en-US", "ja-JP", "ko-KR"]);
const ORIGINS = new Set([
  "human",
  "official_cn",
  "official_tw",
  "official_en",
  "official_kr",
]);
const record = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const validHash = (value) => typeof value === "string" && HASH.test(value);
const boundedString = (value, limit) =>
  typeof value === "string" &&
  value.length > 0 &&
  encoder.encode(value).length <= limit;

function validText(value, translated = false) {
  if (typeof value !== "string" || encoder.encode(value).length > 16 * 1024)
    return false;
  for (const char of value) {
    const code = char.codePointAt(0);
    if (
      (code < 32 && code !== 10 && code !== 9) ||
      (code >= 127 && code <= 159) ||
      (code >= 0xd800 && code <= 0xdfff)
    )
      return false;
    if (
      translated &&
      (code === 9 ||
        code === 0xad ||
        (code >= 0xfdd0 && code <= 0xfdef) ||
        (code & 0xffff) >= 0xfffe ||
        (code >= 0x202a && code <= 0x202e) ||
        (code >= 0x2066 && code <= 0x2069))
    )
      return false;
  }
  return true;
}

function sourceDocument(raw) {
  const doc = JSON.parse(raw);
  if (
    !record(doc) ||
    doc.schema !== "moly-text-source/1" ||
    !REGIONS.has(doc.region) ||
    !validHash(doc.revision) ||
    !Array.isArray(doc.scripts)
  )
    throw new Error("Invalid text source identity");
  const keys = new Set(),
    ids = new Set();
  for (const script of doc.scripts) {
    if (
      !record(script) ||
      !boundedString(script.key, 512) ||
      keys.has(script.key) ||
      !Array.isArray(script.lines)
    )
      throw new Error("Invalid text source script");
    keys.add(script.key);
    const bindings = new Set();
    for (const line of script.lines) {
      const step = line?.step ?? null;
      if (
        !record(line) ||
        !boundedString(line.id, 160) ||
        ids.has(line.id) ||
        !validHash(line.sourceHash) ||
        !validText(line.text) ||
        !["text", "speaker", "tweet"].includes(line.kind) ||
        (line.kind === "tweet"
          ? step !== null
          : !Number.isSafeInteger(step) || step < 0)
      )
        throw new Error("Invalid text source unit");
      const binding = `${line.kind}:${step}`;
      if (bindings.has(binding))
        throw new Error("Duplicate text source binding");
      bindings.add(binding);
      ids.add(line.id);
    }
  }
  return doc;
}

function overlayDocument(raw, region, locale) {
  const doc = JSON.parse(raw);
  if (
    !record(doc) ||
    doc.schema !== "moly-text-overlay/1" ||
    !REGIONS.has(doc.sourceRegion) ||
    !LOCALES.has(doc.locale) ||
    !validHash(doc.revision) ||
    !validHash(doc.sourceRevision) ||
    !Array.isArray(doc.entries) ||
    doc.sourceRegion !== region ||
    doc.locale !== locale
  )
    throw new Error("Invalid text overlay identity");
  const ids = new Set();
  for (const line of doc.entries) {
    if (
      !record(line) ||
      !boundedString(line.id, 160) ||
      ids.has(line.id) ||
      !validHash(line.sourceHash) ||
      !validText(line.text, true) ||
      !ORIGINS.has(line.source)
    )
      throw new Error("Invalid or duplicate text overlay unit");
    ids.add(line.id);
  }
  return doc;
}

function validateFonts(fonts) {
  if (fonts === undefined) return;
  if (
    !Array.isArray(fonts) ||
    fonts.length > 8 ||
    fonts.some((font) => {
      if (
        !record(font) ||
        !validHash(font.sha256) ||
        !Number.isSafeInteger(font.bytes) ||
        font.bytes < 1 ||
        font.bytes > 32 * 1048576 ||
        typeof font.url !== "string"
      )
        return true;
      const url = new URL(font.url);
      return (
        (url.protocol !== "https:" &&
          !(
            url.protocol === "http:" &&
            ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
          )) ||
        !!url.username ||
        !!url.password ||
        !!url.search ||
        !!url.hash
      );
    })
  )
    throw new Error("Invalid translation font reference");
}

async function sha256(bytes) {
  return Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}
function address(base, path) {
  if (
    typeof path !== "string" ||
    !path ||
    /[%\\?#:]/.test(path) ||
    path.split("/").some((part) => !part || part === "." || part === "..")
  )
    throw new Error("Invalid text object path");
  const directory = new URL(".", base);
  const result = new URL(path, directory);
  if (
    result.origin !== directory.origin ||
    !result.pathname.startsWith(directory.pathname)
  )
    throw new Error("Text object escaped its publication");
  return result.href;
}
async function read(url, ref, signal, optional = false) {
  const target = new URL(url, globalThis.location?.href);
  if (!/^https?:$/.test(target.protocol) || target.username || target.password)
    throw new Error("Invalid text publication URL");
  if (
    ref &&
    (!HASH.test(ref.sha256) ||
      !Number.isSafeInteger(ref.bytes) ||
      ref.bytes < 1 ||
      ref.bytes > LIMIT)
  )
    throw new Error("Invalid text object identity");
  const key = ref ? `${target.href}:${ref.sha256}` : null;
  if (key && cache.has(key)) return cache.get(key);
  const response = await fetch(target, {
    signal,
    credentials: "omit",
    redirect: "error",
    cache: ref ? "force-cache" : "no-store",
  });
  if (optional && response.status === 404) return null;
  if (!response.ok) throw new Error(`Text publication HTTP ${response.status}`);
  const length = Number(response.headers.get("content-length"));
  if (length > LIMIT) throw new Error("Text document too large");
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > LIMIT) {
        await reader.cancel();
        throw new Error("Text document too large");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  if (ref && (size !== ref.bytes || (await sha256(bytes)) !== ref.sha256))
    throw new Error("Text object integrity mismatch");
  const result = { text: decoder.decode(bytes), value: null };
  result.value = JSON.parse(result.text);
  if (key) {
    cache.set(key, result);
    if (cache.size > 192) cache.delete(cache.keys().next().value);
  }
  return result;
}
async function object(base, ref, signal) {
  if (!record(ref)) throw new Error("Invalid text object reference");
  return read(address(base, ref.path), ref, signal);
}
function publicationDirectory(url) {
  const root = new URL(url, globalThis.location?.href);
  return /\/revisions\/[a-f0-9]{64}\.json$/.test(root.pathname)
    ? new URL("../latest.json", root).href
    : root.href;
}
async function translation(url, region, locale, bucket, script, signal) {
  if (!url) return null;
  signal = signal
    ? AbortSignal.any([signal, AbortSignal.timeout(15000)])
    : AbortSignal.timeout(15000);
  // Archived roots live one directory below latest.json; object paths in
  // either form are relative to the same locale publication directory.
  const publicationBase = publicationDirectory(url);
  const root = await read(url, null, signal, true);
  if (!root) return null;
  const value = root.value;
  if (
    !record(value) ||
    value.schema !== "moly-text-publication/1" ||
    value.region !== region ||
    value.locale !== locale ||
    value.routing !== "sha256-utf8-prefix-2" ||
    !validHash(value.revision) ||
    !record(value.buckets)
  )
    throw new Error("Translation publication identity mismatch");
  if (!value.buckets[bucket]) return null;
  const routes = (await object(publicationBase, value.buckets[bucket], signal))
    .value;
  if (
    !record(routes) ||
    routes.schema !== "moly-text-translation-routes/1" ||
    !record(routes.scripts)
  )
    throw new Error("Invalid translation routes");
  const ref = routes.scripts[script];
  if (!ref) return null;
  const doc = await object(publicationBase, ref, signal);
  overlayDocument(doc.text, region, locale);
  if (value.fonts !== undefined && !Array.isArray(value.fonts))
    throw new Error("Invalid translation fonts");
  const fonts = (value.fonts ?? []).map((ref) => ({
    ...ref,
    url: address(publicationBase, ref.path),
  }));
  validateFonts(fonts);
  return { text: doc.text, publication: value.revision, fonts };
}

async function searchShards(base, root, translated, signal) {
  const refs = Object.entries(root.searchBuckets ?? {});
  if (refs.length > 16) throw new Error("Too many text search shards");
  const scripts = new Map();
  let next = 0;
  const readNext = async () => {
    while (next < refs.length) {
      const [prefix, ref] = refs[next++];
      if (!/^[a-f0-9]$/.test(prefix))
        throw new Error("Invalid text search prefix");
      const doc = (await object(base, ref, signal)).value;
      if (
        !record(doc) ||
        doc.schema !==
          (translated
            ? "moly-text-translation-search/1"
            : "moly-text-source-search/1") ||
        !record(doc.scripts)
      )
        throw new Error("Invalid text search shard");
      for (const [key, lines] of Object.entries(doc.scripts)) {
        if (
          !boundedString(key, 512) ||
          scripts.has(key) ||
          !Array.isArray(lines) ||
          (await sha256(encoder.encode(key)))[0] !== prefix
        )
          throw new Error("Invalid text search script");
        const ids = new Set();
        for (const row of lines) {
          if (
            !Array.isArray(row) ||
            row.length !== 4 ||
            !boundedString(row[0], 160) ||
            ids.has(row[0]) ||
            !validHash(row[1]) ||
            (translated
              ? !validText(row[2], true) || !ORIGINS.has(row[3])
              : !["text", "speaker", "tweet"].includes(row[2]) ||
                !validText(row[3]))
          )
            throw new Error("Invalid text search unit");
          ids.add(row[0]);
        }
        scripts.set(key, lines);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(4, refs.length) }, readNext));
  return scripts;
}

async function translationSearch(url, region, locale, signal) {
  if (!url) return new Map();
  signal = signal
    ? AbortSignal.any([signal, AbortSignal.timeout(15000)])
    : AbortSignal.timeout(15000);
  const doc = await read(url, null, signal, true);
  if (!doc) return new Map();
  const root = doc.value;
  if (
    !record(root) ||
    root.schema !== "moly-text-publication/1" ||
    root.region !== region ||
    root.locale !== locale ||
    !validHash(root.revision) ||
    root.routing !== "sha256-utf8-prefix-2"
  )
    throw new Error("Invalid translation search publication");
  return searchShards(publicationDirectory(url), root, true, signal);
}

/** A compact display/search view, loaded independently from resource catalogues. */
export async function loadTextIndex(
  { source, official, community, region, locale },
  signal,
) {
  if (!source) return {};
  if (!REGIONS.has(region) || !LOCALES.has(locale))
    throw new Error("Invalid text search scope");
  const sourceSignal = signal
    ? AbortSignal.any([signal, AbortSignal.timeout(30000)])
    : AbortSignal.timeout(30000);
  const root = (await read(source.url, source, sourceSignal)).value;
  if (
    !record(root) ||
    root.schema !== "moly-text-runtime/1" ||
    root.region !== region ||
    root.routing !== "sha256-utf8-prefix-2" ||
    !validHash(root.revision)
  )
    throw new Error("Invalid source search identity");
  const sources = await searchShards(source.url, root, false, sourceSignal);
  const results = await Promise.allSettled([
    translationSearch(official, region, locale, signal),
    translationSearch(community, region, locale, signal),
  ]);
  signal?.throwIfAborted();
  const [officialRows, communityRows] = results.map((result) =>
    result.status === "fulfilled" ? result.value : new Map(),
  );
  const expected = {
    "zh-CN": "official_cn",
    "zh-TW": "official_tw",
    "en-US": "official_en",
    "ko-KR": "official_kr",
  }[locale];
  const index = Object.create(null);
  for (const [key, lines] of sources) {
    const overrides = [
      [officialRows.get(key) ?? [], expected],
      [communityRows.get(key) ?? [], "human"],
    ].map(([rows, kind]) => [new Map(rows.map((row) => [row[0], row])), kind]);
    const resolved = lines.map((line) => {
      for (const [rows, kind] of overrides) {
        const row = rows.get(line[0]);
        if (row?.[1] === line[1] && row[3] === kind && row[2].trim())
          return row[2];
      }
      return line[3].replace(/<[^>]*>/g, "");
    });
    const firstText = lines.findIndex(
      (line) => line[2] === "text" || line[2] === "tweet",
    );
    const title =
      firstText < 0 ? "" : resolved[firstText].replace(/\s+/g, " ").trim();
    index[key] = {
      title:
        Array.from(title).slice(0, 44).join("") +
        (Array.from(title).length > 44 ? "…" : ""),
      search: [...resolved, ...lines.map((line) => line[3])].join(" "),
    };
  }
  return index;
}

/** Resolve one playback's immutable documents. A later call may discover a new publication. */
export async function loadTextSession(
  { source, official, community, region, locale, scriptKey },
  signal,
) {
  const sourceSignal = signal
    ? AbortSignal.any([signal, AbortSignal.timeout(15000)])
    : AbortSignal.timeout(15000);
  if (!source || !scriptKey) return null;
  if (
    !REGIONS.has(region) ||
    !LOCALES.has(locale) ||
    !boundedString(scriptKey, 512)
  )
    throw new Error("Invalid text session identity");
  const root = (await read(source.url, source, sourceSignal)).value;
  if (
    !record(root) ||
    root.schema !== "moly-text-runtime/1" ||
    root.region !== region ||
    root.routing !== "sha256-utf8-prefix-2" ||
    !validHash(root.revision) ||
    !record(root.buckets)
  )
    throw new Error("Text source identity mismatch");
  const bucket = (await sha256(encoder.encode(scriptKey))).slice(0, 2);
  if (!root.buckets[bucket]) return null;
  const routes = (await object(source.url, root.buckets[bucket], sourceSignal))
    .value;
  if (
    !record(routes) ||
    routes.schema !== "moly-text-routes/1" ||
    !record(routes.scripts)
  )
    throw new Error("Invalid text source routes");
  const route = routes.scripts[scriptKey];
  if (!route) return null;
  const display = await object(source.url, route.source, sourceSignal);
  const displaySource = sourceDocument(display.text);
  if (
    displaySource.region !== region ||
    !displaySource.scripts.some((script) => script.key === scriptKey)
  )
    throw new Error("Invalid text source script");
  const resolved = await Promise.allSettled([
    translation(official, region, locale, bucket, scriptKey, signal),
    translation(community, region, locale, bucket, scriptKey, signal),
  ]);
  signal?.throwIfAborted();
  // Each optional publication can be unavailable independently. An official
  // outage must not discard an admitted human translation, or vice versa.
  const [officialText, humanText] = resolved.map((result) =>
    result.status === "fulfilled" ? result.value : null,
  );
  const fonts = [
    ...new Map(
      [...(officialText?.fonts ?? []), ...(humanText?.fonts ?? [])].map(
        (font) => [font.sha256, font],
      ),
    ).values(),
  ];
  return validateTextSession({
    source: display.text,
    official: officialText?.text ?? "",
    community: humanText?.text ?? "",
    locale,
    fonts,
    revisions: {
      source: root.revision,
      official: officialText?.publication ?? null,
      community: humanText?.publication ?? null,
    },
  });
}

export function validateTextSession(session) {
  if (session === null) return null;
  if (!record(session) || !LOCALES.has(session.locale))
    throw new Error("Invalid text session locale");
  for (const key of ["source", "official", "community"])
    if (
      typeof session[key] !== "string" ||
      encoder.encode(session[key]).length > LIMIT
    )
      throw new Error("Invalid text session document");
  const source = sourceDocument(session.source);
  for (const key of ["official", "community"])
    if (session[key])
      overlayDocument(session[key], source.region, session.locale);
  validateFonts(session.fonts);
  return session;
}

/** Download and verify a complete font set before changing the runtime. */
export async function prepareTextFonts(session, signal, automatic) {
  if (session) validateTextSession(session);
  validateFonts(session?.fonts);
  const fonts = automatic
    ? await selectTextFonts(session, automatic, signal)
    : (session?.fonts ?? []);
  validateFonts(fonts);
  const downloaded = [],
    refs = [];
  let byteLength = 0;
  for (const font of fonts) {
    const response = await fetch(font.url, {
      signal,
      credentials: "omit",
      redirect: "error",
      cache: "force-cache",
    });
    if (
      !response.ok ||
      Number(response.headers.get("content-length")) > 32 * 1048576
    )
      throw new Error("Translation font download failed");
    const reader = response.body.getReader();
    const chunks = [];
    let total = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        total += value.byteLength;
        if (total > font.bytes) {
          await reader.cancel();
          throw new Error("Translation font exceeds declared size");
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
    if (bytes.length !== font.bytes || (await sha256(bytes)) !== font.sha256)
      throw new Error("Translation font integrity mismatch");
    downloaded.push(bytes);
    refs.push({ sha256: font.sha256, bytes: font.bytes });
    byteLength += bytes.byteLength;
  }
  signal?.throwIfAborted();
  const bytes = new Uint8Array(byteLength);
  let offset = 0;
  for (const font of downloaded) {
    bytes.set(font, offset);
    offset += font.byteLength;
  }
  return { bytes, refs: JSON.stringify(refs) };
}

/** Compatibility helper; stage playback installs the whole session atomically. */
export async function installTextFonts(session, wasm, signal) {
  const prepared = await prepareTextFonts(session, signal);
  let offset = 0;
  for (const font of JSON.parse(prepared.refs)) {
    wasm.install_text_font(
      prepared.bytes.slice(offset, offset + font.bytes),
      font.sha256,
    );
    offset += font.bytes;
  }
}

/** Display-only view; execution labels and the source entry stay unchanged. */
export function localizeEntry(entry, session) {
  if (!session || !entry) return entry;
  validateTextSession(session);
  const source = JSON.parse(session.source);
  const scripts = new Map(source.scripts.map((script) => [script.key, script]));
  const expected = {
    "zh-CN": "official_cn",
    "zh-TW": "official_tw",
    "en-US": "official_en",
    "ko-KR": "official_kr",
  }[session.locale];
  const overlays = [
    [session.official, expected],
    [session.community, "human"],
  ].map(([raw, kind]) => {
    if (!raw) return [new Map(), kind];
    const doc = JSON.parse(raw);
    return [new Map(doc.entries.map((line) => [line.id, line])), kind];
  });
  const resolve = (scriptKey, step, kind, original) => {
    const sourcePlain = (text) =>
      text
        .replace(/<[^>]*>/g, "")
        .replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, "");
    const line = scripts
      .get(scriptKey)
      ?.lines.find(
        (line) =>
          line.kind === kind &&
          line.step === step &&
          (line.text === original || sourcePlain(line.text) === original),
      );
    if (!line) return original;
    for (const [rows, origin] of overlays) {
      const translated = rows.get(line.id);
      if (
        translated?.sourceHash === line.sourceHash &&
        translated.source === origin &&
        translated.text.trim()
      )
        return translated.text;
    }
    return original;
  };
  const lines = entry.lines?.map((line) => ({
    ...line,
    speaker: resolve(
      entry.scriptKey,
      line.speakerStep,
      "speaker",
      line.speaker,
    ),
    text: resolve(entry.scriptKey, line.textStep, "text", line.text),
  }));
  const excerpt = (value, limit) => {
    const chars = Array.from(value.replace(/\s+/g, " ").trim());
    return chars.slice(0, limit).join("") + (chars.length > limit ? "…" : "");
  };
  return {
    ...entry,
    ...(lines?.length
      ? {
          title: excerpt(lines[0].text, 44),
          subtitle: excerpt(
            lines.map((line) => `${line.speaker}\n${line.text}`).join("\n\n"),
            150,
          ),
        }
      : {}),
    lines,
    preview: entry.preview
      ? {
          ...entry.preview,
          text: resolve(
            `@tweet/${entry.preview.tweetId}`,
            null,
            "tweet",
            entry.preview.text,
          ),
        }
      : undefined,
  };
}
