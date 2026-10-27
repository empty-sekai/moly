// Font delivery is independent of resource and translation publications.
// The exact engine font digest selects base coverage; no union is assumed.
const HASH = /^[a-f0-9]{64}$/;
const LIMIT = 2 * 1024 * 1024;
const cache = new Map();
const ignored = new Set([3, 0x61c, 0x200b, 0x200e, 0x200f, 0x2060]);
const record = (value) =>
  value && typeof value === "object" && !Array.isArray(value);

function ranges(value) {
  if (!Array.isArray(value) || value.length > 65536)
    throw new Error("Invalid font coverage");
  let previous = -1;
  for (const row of value) {
    if (
      !Array.isArray(row) ||
      row.length !== 2 ||
      !row.every(Number.isSafeInteger) ||
      row[0] < 0 ||
      row[1] > 0x10ffff ||
      row[0] > row[1] ||
      row[0] <= previous ||
      (row[0] <= 0xdfff && row[1] >= 0xd800)
    )
      throw new Error("Invalid font coverage range");
    previous = row[1];
  }
  return value;
}

export function covers(coverage, code) {
  let left = 0,
    right = coverage.length;
  while (left < right) {
    const middle = (left + right) >>> 1,
      row = coverage[middle];
    if (code < row[0]) right = middle;
    else if (code > row[1]) left = middle + 1;
    else return true;
  }
  return false;
}

function reference(ref, extension, maximum) {
  if (
    !record(ref) ||
    !HASH.test(ref.sha256) ||
    ref.path !== `objects/${ref.sha256}.${extension}` ||
    !Number.isSafeInteger(ref.bytes) ||
    ref.bytes < 1 ||
    ref.bytes > maximum
  )
    throw new Error("Invalid font object reference");
}

export function validateFontCatalog(doc) {
  if (
    !record(doc) ||
    doc.schema !== "moly-text-fonts/1" ||
    !HASH.test(doc.revision) ||
    !record(doc.baseFonts) ||
    Object.keys(doc.baseFonts).length > 64 ||
    !Array.isArray(doc.fonts) ||
    !doc.fonts.length ||
    doc.fonts.length > 8
  )
    throw new Error("Invalid font catalogue");
  for (const [hash, font] of Object.entries(doc.baseFonts)) {
    if (!HASH.test(hash) || !record(font))
      throw new Error("Invalid base font identity");
    ranges(font.coverage);
  }
  const seen = new Set();
  for (const font of doc.fonts) {
    reference(font, "ttf", 32 * 1048576);
    reference(font.license, "txt", 256 * 1024);
    if (seen.has(font.sha256)) throw new Error("Duplicate font object");
    seen.add(font.sha256);
    ranges(font.coverage);
  }
  return doc;
}

function endpoint(url) {
  const value = new URL(url, globalThis.location?.href);
  if (
    (value.protocol !== "https:" &&
      !(
        value.protocol === "http:" &&
        ["localhost", "127.0.0.1", "[::1]"].includes(value.hostname)
      )) ||
    value.username ||
    value.password ||
    value.search ||
    value.hash
  )
    throw new Error("Invalid font catalogue URL");
  return value;
}

async function read(url, signal, immutable) {
  const response = await fetch(url, {
    signal,
    credentials: "omit",
    redirect: "error",
    cache: immutable ? "force-cache" : "no-store",
  });
  if (
    !response.ok ||
    !response.body ||
    !response.headers.get("content-type")?.includes("application/json")
  )
    throw new Error("Font catalogue unavailable");
  const reader = response.body.getReader(),
    parts = [];
  let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      length += value.length;
      if (length > LIMIT) {
        await reader.cancel();
        throw new Error("Font catalogue exceeds limit");
      }
      parts.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

export async function loadFontCatalog(url, signal) {
  signal?.throwIfAborted();
  const target = endpoint(url),
    old = cache.get(target.href);
  if (old && Date.now() - old.time < 15000) return old.value;
  const raw = await read(
    target,
    signal,
    /\/revisions\/[a-f0-9]{64}\.json$/.test(target.pathname),
  );
  const catalog = validateFontCatalog(JSON.parse(raw));
  const directory = /\/revisions\/[a-f0-9]{64}\.json$/.test(target.pathname)
    ? new URL("../", target)
    : new URL(".", target);
  const archive = new URL(`revisions/${catalog.revision}.json`, directory);
  if (
    /\/revisions\/[a-f0-9]{64}\.json$/.test(target.pathname) &&
    archive.href !== target.href
  )
    throw new Error("Font catalogue revision mismatch");
  if (
    archive.href !== target.href &&
    (await read(archive, signal, true)) !== raw
  )
    throw new Error("Font catalogue history mismatch");
  const value = {
    catalog,
    directory: directory.href,
    revision: catalog.revision,
  };
  cache.set(target.href, { time: Date.now(), value });
  if (cache.size > 8) cache.delete(cache.keys().next().value);
  return value;
}

/** Rust str::trim and Go strings.TrimSpace use Unicode White_Space. */
export function hasDisplayText(text) {
  return /[^\p{White_Space}]/u.test(text);
}

/** Matches TextSession::characters: only winning display strings count. */
export function displayCodepoints(session) {
  if (!session) return [];
  const source = JSON.parse(session.source),
    expected =
      {
        "zh-CN": "official_cn",
        "zh-TW": "official_tw",
        "en-US": "official_en",
        "ko-KR": "official_kr",
      }[session.locale] ?? "";
  const overlays = [
    [session.official, expected],
    [session.community, "human"],
  ].map(([raw, origin]) => [
    new Map(raw ? JSON.parse(raw).entries.map((line) => [line.id, line]) : []),
    origin,
  ]);
  const points = new Set();
  for (const script of source.scripts)
    for (const line of script.lines) {
      let text = line.text;
      for (const [rows, origin] of overlays) {
        const translated = rows.get(line.id);
        if (
          translated?.sourceHash === line.sourceHash &&
          translated.source === origin &&
          hasDisplayText(translated.text)
        ) {
          text = translated.text;
          break;
        }
      }
      // Translations are literal text. Do not strip angle-bracket text or tags.
      for (const char of text) {
        const code = char.codePointAt(0);
        if (code < 32 || (code >= 127 && code <= 159) || ignored.has(code))
          continue;
        points.add(code);
      }
    }
  return [...points].sort((a, b) => a - b);
}

export async function selectTextFonts(
  session,
  { catalogUrl, baseFontSha256 },
  signal,
) {
  signal?.throwIfAborted();
  if (!session) return [];
  if (!HASH.test(baseFontSha256 ?? ""))
    throw new Error("Engine base font identity unavailable");
  const { catalog, directory } = await loadFontCatalog(catalogUrl, signal);
  const base = catalog.baseFonts[baseFontSha256];
  if (!base) throw new Error("Font catalogue does not cover this engine font");
  let missing = displayCodepoints(session).filter(
    (code) => !covers(base.coverage, code),
  );
  const selected = [];
  for (const font of catalog.fonts) {
    if (!missing.some((code) => covers(font.coverage, code))) continue;
    selected.push({
      url: new URL(font.path, directory).href,
      sha256: font.sha256,
      bytes: font.bytes,
    });
    missing = missing.filter((code) => !covers(font.coverage, code));
  }
  if (missing.length)
    throw new Error(
      `Unsupported display characters: ${missing
        .slice(0, 16)
        .map((code) => `U+${code.toString(16).toUpperCase().padStart(4, "0")}`)
        .join(" ")}`,
    );
  return selected;
}
