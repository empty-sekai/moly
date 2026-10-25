// Source text joins belong to the resource snapshot. Translation publications
// are intentionally absent from this closure and from its snapshot identity.
import { existsSync, readFileSync, realpathSync } from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function textSourceClosure(assets, region) {
  const root = path.join(assets, "text");
  if (!existsSync(path.join(root, "index.json"))) return null;
  const bytes = readFileSync(path.join(root, "index.json"));
  const index = JSON.parse(bytes);
  if (
    index.schema !== "moly-text-runtime/1" ||
    index.region !== region ||
    index.routing !== "sha256-utf8-prefix-2" ||
    Object.keys(index.buckets).length > 256
  )
    throw new Error("Invalid source text index");
  if (index.overlays || index.overlayRevisions || index.locales)
    throw new Error("Resource text index must not depend on translations");
  const files = new Map([["index.json", bytes]]);
  const read = (ref) => {
    if (
      !ref ||
      ref.path !== `objects/${ref.sha256}.json` ||
      !/^[a-f0-9]{64}$/.test(ref.sha256) ||
      !Number.isSafeInteger(ref.bytes) ||
      ref.bytes < 1 ||
      ref.bytes > 2 * 1048576
    )
      throw new Error("Invalid source text reference");
    const resolved = realpathSync(path.join(root, ref.path));
    if (!resolved.startsWith(realpathSync(root) + path.sep))
      throw new Error("Text path escaped source");
    const data = files.get(ref.path) ?? readFileSync(resolved);
    if (data.length !== ref.bytes || hash(data) !== ref.sha256)
      throw new Error("Text source integrity mismatch");
    files.set(ref.path, data);
    return JSON.parse(data);
  };
  for (const [bucket, ref] of Object.entries(index.buckets)) {
    if (!/^[a-f0-9]{2}$/.test(bucket))
      throw new Error("Invalid text routing bucket");
    const routes = read(ref);
    if (routes.schema !== "moly-text-routes/1")
      throw new Error("Invalid source text routes");
    for (const [key, route] of Object.entries(routes.scripts)) {
      if (hash(key).slice(0, 2) !== bucket || route.overlays)
        throw new Error("Invalid source-only text route");
      const source = read(route.source);
      if (
        source.schema !== "moly-text-source/1" ||
        source.region !== region ||
        !source.scripts.some((script) => script.key === key)
      )
        throw new Error("Text source script mismatch");
    }
  }
  for (const [bucket, ref] of Object.entries(index.searchBuckets ?? {})) {
    if (!/^[a-f0-9]$/.test(bucket))
      throw new Error("Invalid text search bucket");
    const search = read(ref);
    if (
      search.schema !== "moly-text-source-search/1" ||
      !search.scripts ||
      Object.keys(search.scripts).some(
        (key) => hash(key).slice(0, 1) !== bucket,
      )
    )
      throw new Error("Invalid source text search index");
  }
  return { files, sha256: hash(bytes), bytes: bytes.length };
}
