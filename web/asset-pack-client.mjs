// Infrastructure-only logical asset resolver. This is not a second CAS.
// Both native/WASM PackReader and this prewarmer use the same published store.
const SHA = /^[a-f0-9]{64}$/;
const DOCUMENT = 16 * 1048576,
  BLOB = 128 * 1048576,
  ASSET = 256 * 1048576;
const encoder = new TextEncoder(),
  decoder = new TextDecoder("utf-8", { fatal: true });
export const validCatalogId = (value) =>
  typeof value === "string" && SHA.test(value);
export const validRegion = (value) =>
  typeof value === "string" && /^[a-z][a-z0-9-]{1,15}$/.test(value);
export function packedPath(value) {
  if (
    typeof value !== "string" ||
    !value ||
    /[\\:\x00-\x1f\x7f]/.test(value) ||
    value.split("/").some((part) => !part || part === "." || part === "..")
  )
    throw new Error("Noncanonical packed path");
  return value;
}
function order(a, b) {
  const aa = Array.from(a),
    bb = Array.from(b);
  for (let i = 0; i < Math.min(aa.length, bb.length); i++) {
    const diff = aa[i].codePointAt(0) - bb[i].codePointAt(0);
    if (diff) return diff;
  }
  return aa.length - bb.length;
}
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object")
    return `{${Object.keys(value)
      .sort(order)
      .map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`)
      .join(",")}}`;
  return JSON.stringify(value);
}
export const canonicalPackBytes = (value) =>
  encoder.encode(canonical(value) + "\n");
export async function packDigest(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
const object = (value) =>
  value && typeof value === "object" && !Array.isArray(value);
const size = (value, maximum) =>
  Number.isSafeInteger(value) && value >= 0 && value <= maximum;
const shape = (value, keys) =>
  object(value) &&
  Object.keys(value).length === keys.length &&
  keys.every((key) => Object.hasOwn(value, key));
function address(path, directory) {
  if (
    typeof path !== "string" ||
    !path.startsWith(directory + "/") ||
    !path.endsWith(".json") ||
    !validCatalogId(path.slice(directory.length + 1, -5))
  )
    throw new Error("Invalid immutable document address");
  return path.slice(directory.length + 1, -5);
}
class ReadBudget {
  used = 0;
  waiting = [];
  acquire(bytes) {
    if (!size(bytes, 512 * 1048576))
      return Promise.reject(
        new Error("Packed prewarm exceeds its memory budget"),
      );
    return new Promise((resolve) => {
      this.waiting.push({ bytes, resolve });
      this.pump();
    });
  }
  pump() {
    while (
      this.waiting.length &&
      this.used + this.waiting[0].bytes <= 512 * 1048576
    ) {
      const { bytes, resolve } = this.waiting.shift();
      this.used += bytes;
      let released = false;
      resolve(() => {
        if (!released) {
          released = true;
          this.used -= bytes;
          this.pump();
        }
      });
    }
  }
}
const budget = new ReadBudget();
// Exact-length buffers avoid retaining every chunk plus a second full copy.
// Reserve the encoded input, a decoder/crypto working copy and decoded output;
// every representation admitted by BLOB/ASSET fits the shared 512 MiB budget.
export const packedReadBytes = (entry) =>
  entry.codec === "identity"
    ? entry.blob_bytes * 2
    : entry.blob_bytes * 2 + entry.bytes;
async function boundedBody(response, maximum, exact = false) {
  if (response.status !== 200 || !response.body)
    throw new Error(`Packed response unavailable (${response.status})`);
  const reader = response.body.getReader(),
    chunks = [],
    target = exact ? new Uint8Array(maximum) : null;
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > maximum)
        throw new Error("Packed response exceeded its byte limit");
      if (target) target.set(value, length - value.byteLength);
      else chunks.push(value);
    }
    if (exact && length !== maximum)
      throw new Error("Packed response length mismatch");
    if (target) return target;
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes;
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
}

export class PackClient {
  constructor(
    root,
    catalogId = null,
    { fetchImpl = fetch, signal, required = false } = {},
  ) {
    this.root = new URL(root, globalThis.location?.href);
    if (
      !this.root.pathname.endsWith("/") ||
      this.root.search ||
      this.root.hash ||
      this.root.username ||
      this.root.password ||
      !["http:", "https:"].includes(this.root.protocol) ||
      (globalThis.location && this.root.origin !== location.origin)
    )
      throw new Error("Pack store must be a same-origin directory");
    if (catalogId !== null && !validCatalogId(catalogId))
      throw new Error("Invalid catalog identity");
    // Keep fetch a function call: Window.fetch rejects a PackClient receiver.
    this.catalogId = catalogId;
    this.fetchImpl = (...args) => fetchImpl(...args);
    this.signal = signal;
    this.required = required;
    this.manifests = new Map();
    this.catalogPromise = null;
  }
  url(path) {
    return new URL(
      packedPath(path).split("/").map(encodeURIComponent).join("/"),
      this.root,
    ).href;
  }
  async request(path) {
    const timeout = AbortSignal.timeout(60000);
    return this.fetchImpl(this.url(path), {
      credentials: "same-origin",
      redirect: "error",
      cache: "no-store",
      signal: this.signal ? AbortSignal.any([this.signal, timeout]) : timeout,
      ...(this.required ? { headers: { "X-Moly-Required": "1" } } : {}),
    });
  }
  async document(path, expected = null) {
    const release = await budget.acquire(DOCUMENT * 3);
    try {
      const bytes = await boundedBody(await this.request(path), DOCUMENT);
      if (expected && (await packDigest(bytes)) !== expected)
        throw new Error(`Packed document content address mismatch: ${path}`);
      return { value: JSON.parse(decoder.decode(bytes)), bytes };
    } finally {
      release();
    }
  }
  async selection() {
    if (!this.catalogPromise)
      this.catalogPromise = (async () => {
        const path = this.catalogId
          ? `catalogs/${this.catalogId}.json`
          : "asset-packs.json";
        const { value: catalog } = await this.document(path, this.catalogId);
        if (catalog?.schema === "moly-asset-packs/1")
          throw new Error(
            "Legacy v1 packs require explicit offline migration to v2",
          );
        if (
          !shape(catalog, [
            "schema",
            "region",
            "version",
            "provenance",
            "packages",
          ]) ||
          catalog.schema !== "moly-asset-packs/2" ||
          typeof catalog.version !== "string" ||
          !catalog.version ||
          !Array.isArray(catalog.packages) ||
          !catalog.packages.length ||
          catalog.packages.length > 100000 ||
          !validRegion(catalog.region) ||
          !object(catalog.provenance)
        )
          throw new Error("Invalid pack catalog");
        const ids = new Map(),
          owners = new Map(),
          manifestNames = new Set();
        for (const row of catalog.packages) {
          if (
            !shape(row, [
              "id",
              "kind",
              "manifest",
              "content_id",
              "dependencies",
              "paths",
              "download_bytes",
              "content_bytes",
            ]) ||
            typeof row.kind !== "string" ||
            !row.kind ||
            !size(row.download_bytes, Number.MAX_SAFE_INTEGER) ||
            !size(row.content_bytes, Number.MAX_SAFE_INTEGER)
          )
            throw new Error("Invalid package declaration");
          packedPath(row.id);
          packedPath(row.manifest);
          address(row.manifest, "packages");
          if (!validCatalogId(row.content_id))
            throw new Error("Missing semantic identity");
          if (
            ids.has(row.id) ||
            manifestNames.has(row.manifest) ||
            !Array.isArray(row.paths) ||
            !row.paths.length ||
            !Array.isArray(row.dependencies)
          )
            throw new Error("Invalid/duplicate package ownership");
          ids.set(row.id, row);
          manifestNames.add(row.manifest);
          for (const path of row.paths) {
            packedPath(path);
            if (owners.has(path))
              throw new Error("Duplicate logical asset ownership");
            owners.set(path, row);
            if (owners.size > 100000)
              throw new Error("Catalog exceeds path budget");
          }
        }
        const incoming = new Map(),
          outgoing = new Map();
        for (const row of catalog.packages) outgoing.set(row.id, []);
        for (const row of catalog.packages) {
          incoming.set(row.id, row.dependencies.length);
          if (new Set(row.dependencies).size !== row.dependencies.length)
            throw new Error("Duplicate dependency");
          for (const dependency of row.dependencies) {
            if (!ids.has(dependency))
              throw new Error("Missing package dependency");
            outgoing.get(dependency).push(row.id);
          }
        }
        const ready = [...incoming]
          .filter(([, n]) => n === 0)
          .map(([id]) => id);
        let visited = 0;
        while (ready.length) {
          const id = ready.pop();
          visited++;
          for (const child of outgoing.get(id)) {
            const count = incoming.get(child) - 1;
            incoming.set(child, count);
            if (!count) ready.push(child);
          }
        }
        if (visited !== ids.size) throw new Error("Package dependency cycle");
        return { catalog, owners };
      })().catch((error) => {
        this.catalogPromise = null;
        throw error;
      });
    return this.catalogPromise;
  }
  async catalog() {
    return (await this.selection()).catalog;
  }
  async package(row) {
    if (!this.manifests.has(row.manifest))
      this.manifests.set(
        row.manifest,
        (async () => {
          const selection = await this.selection();
          const expected = address(row.manifest, "packages");
          const { value: manifest } = await this.document(
            row.manifest,
            expected,
          );
          if (
            !shape(manifest, [
              "schema",
              "download_bytes",
              "resident_bytes",
              "content_bytes",
              "logical_bytes",
              "transforms",
              "entries",
              "content_id",
              "encoders",
            ]) ||
            ![
              "download_bytes",
              "resident_bytes",
              "content_bytes",
              "logical_bytes",
            ].every((name) => size(manifest[name], Number.MAX_SAFE_INTEGER))
          )
            throw new Error("Invalid v2 package fields");
          if (
            !Array.isArray(manifest.entries) ||
            manifest.entries.length !== row.paths.length
          )
            throw new Error("Package ownership mismatch");
          if (
            manifest.schema !== "moly-asset-manifest/2" ||
            manifest.version !== undefined ||
            manifest.generated_utc !== undefined ||
            manifest.blob_prefix !== undefined ||
            manifest.content_id !== row.content_id ||
            !object(manifest.transforms) ||
            !object(manifest.encoders)
          )
            throw new Error("Incompatible v2 package");
          // Runtime profiles have no transforms; arbitrary transform recipes are
          // not silently treated as plain glTF/image data.
          if (Object.keys(manifest.transforms).length)
            throw new Error("Unsupported runtime transform");
          const entries = new Map();
          let previous = null;
          for (const entry of manifest.entries) {
            if (
              !shape(entry, [
                "path",
                "blob",
                "blob_bytes",
                "blob_sha256",
                "bytes",
                "content_sha256",
                "codec",
                "http_encoding",
                "xf",
              ])
            )
              throw new Error("Invalid v2 entry fields");
            packedPath(entry.path);
            packedPath(entry.blob);
            if (
              !validCatalogId(entry.blob_sha256) ||
              !validCatalogId(entry.content_sha256) ||
              !size(entry.blob_bytes, BLOB) ||
              !size(entry.bytes, ASSET) ||
              !["identity", "gzip"].includes(entry.codec) ||
              entry.http_encoding !== "identity" ||
              entry.xf !== null ||
              (entry.codec === "identity" && entry.blob_bytes !== entry.bytes)
            )
              throw new Error("Unsupported packed representation");
            const suffix = entry.codec === "gzip" ? "gzz" : "bin";
            if (
              entry.blob !==
              `${entry.blob_sha256.slice(0, 2)}/${entry.blob_sha256}.${suffix}`
            )
              throw new Error("Blob address mismatch");
            if (
              entries.has(entry.path) ||
              selection.owners.get(entry.path) !== row ||
              (previous !== null && order(previous, entry.path) >= 0)
            )
              throw new Error("Invalid package path ownership/order");
            entries.set(entry.path, entry);
            previous = entry.path;
          }
          if (row.paths.some((path) => !entries.has(path)))
            throw new Error("Package differs from release catalog");
          const projection = {
            schema: "moly-asset-content/1",
            transforms: manifest.transforms,
            entries: [...entries.values()].map(
              ({ path, bytes, content_sha256, xf }) => ({
                path,
                bytes,
                content_sha256,
                xf,
              }),
            ),
          };
          if (
            (await packDigest(canonicalPackBytes(projection))) !==
            manifest.content_id
          )
            throw new Error("Semantic content identity mismatch");
          return { manifest, entries };
        })().catch((error) => {
          this.manifests.delete(row.manifest);
          throw error;
        }),
      );
    return this.manifests.get(row.manifest);
  }
  async resolve(path) {
    packedPath(path);
    const { owners } = await this.selection(),
      row = owners.get(path);
    if (!row) throw new Error(`Packed logical asset not found: moly://${path}`);
    const { entries } = await this.package(row),
      entry = entries.get(path);
    if (!entry) throw new Error(`Package omitted logical asset: ${path}`);
    return { ...entry, url: this.url(`blobs/${entry.blob}`) };
  }
  async read(path) {
    const entry = await this.resolve(path);
    const release = await budget.acquire(packedReadBytes(entry));
    try {
      let encoded = await boundedBody(
        await this.request(`blobs/${entry.blob}`),
        entry.blob_bytes,
        true,
      );
      if ((await packDigest(encoded)) !== entry.blob_sha256)
        throw new Error(`Packed blob checksum mismatch: ${path}`);
      const decoded =
        entry.codec === "identity"
          ? encoded
          : await boundedBody(
              new Response(
                new ReadableStream({
                  start(controller) {
                    controller.enqueue(encoded);
                    controller.close();
                  },
                }).pipeThrough(new DecompressionStream("gzip")),
              ),
              entry.bytes,
              true,
            );
      encoded = null;
      if (
        decoded.byteLength !== entry.bytes ||
        (await packDigest(decoded)) !== entry.content_sha256
      )
        throw new Error(`Decoded content checksum mismatch: ${path}`);
      return decoded;
    } finally {
      release();
    }
  }
  async json(path) {
    return JSON.parse(decoder.decode(await this.read(path)));
  }
}

/** Source selection is infrastructure, never a change to a moly:// path. */
export function applyPackSelection(url, options = {}) {
  if (options.packs !== undefined && typeof options.packs !== "boolean")
    throw new TypeError("Invalid packs selection");
  if (
    options.assetCatalog !== undefined &&
    !validCatalogId(options.assetCatalog)
  )
    throw new TypeError("Invalid asset catalog identity");
  if (options.assetCatalog && options.packs === false)
    throw new TypeError("A pinned catalog requires packs");
  url.searchParams.delete("packs");
  url.searchParams.delete("asset_catalog");
  if (options.packs || options.assetCatalog) url.searchParams.set("packs", "1");
  if (options.assetCatalog)
    url.searchParams.set("asset_catalog", options.assetCatalog);
  return url;
}
