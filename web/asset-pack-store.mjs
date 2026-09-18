// Node publication checks for stores produced by moly-root's pack publisher.
// No blob writes, encoding, alternate hashing, or snapshot payload copies here.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import {
  PackClient,
  validCatalogId,
  validRegion,
  packedPath,
} from "./asset-pack-client.mjs";
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
function readDocument(filename) {
  if (fs.statSync(filename).size > 16 * 1048576)
    throw new Error("Pack document exceeds reader limits");
  return fs.readFileSync(filename);
}
function inside(root, child) {
  const relative = path.relative(root, child);
  return (
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}
function checkedFile(root, logical) {
  packedPath(logical);
  let current = root;
  for (const part of logical.split("/")) {
    current = path.join(current, part);
    if (fs.lstatSync(current).isSymbolicLink())
      throw new Error("Pack paths cannot traverse symbolic links");
  }
  const file = fs.realpathSync(current);
  if (!inside(root, file) || !fs.statSync(file).isFile())
    throw new Error("Packed object escaped its store");
  return file;
}
export function storeChannels(directory) {
  const root = fs.realpathSync(directory),
    channels = path.join(root, "channels");
  if (!fs.existsSync(channels)) return [];
  const selections = [];
  for (const region of fs.readdirSync(channels).sort()) {
    if (!validRegion(region)) throw new Error("Invalid store channel region");
    const name = `channels/${region}/stable.json`;
    if (!fs.existsSync(path.join(root, name))) continue;
    const pointer = JSON.parse(readDocument(checkedFile(root, name)));
    if (
      pointer.schema !== "moly-asset-channel/1" ||
      pointer.region !== region ||
      pointer.channel !== "stable" ||
      !/^catalogs\/[a-f0-9]{64}\.json$/.test(pointer.catalog)
    )
      throw new Error("Invalid store channel selector");
    const bytes = readDocument(checkedFile(root, pointer.catalog)),
      catalog = JSON.parse(bytes);
    if (
      sha(bytes) !== pointer.catalog.slice(9, -5) ||
      catalog.schema !== "moly-asset-packs/2" ||
      catalog.region !== region
    )
      throw new Error("Store channel has wrong catalog identity");
    selections.push({
      region,
      version: catalog.version,
      assetCatalog: sha(bytes),
      packs: true,
    });
  }
  return selections;
}
export async function verifyPublishedStore(
  directory,
  catalogId,
  { region, version, assets } = {},
) {
  if (!validCatalogId(catalogId))
    throw new Error("Publication requires a pinned asset catalog SHA-256");
  const root = fs.realpathSync(directory),
    source = assets ? fs.realpathSync(assets) : null;
  const client = new PackClient("https://moly-store.invalid/", catalogId, {
    fetchImpl: async (url) => {
      const logical = new URL(url).pathname
        .slice(1)
        .split("/")
        .map(decodeURIComponent)
        .join("/");
      const filename = checkedFile(root, logical),
        stat = fs.statSync(filename);
      if (stat.size > 128 * 1048576)
        throw new Error("Published pack object exceeds reader limits");
      return new Response(fs.readFileSync(filename), {
        headers: { "Content-Length": String(stat.size) },
      });
    },
  });
  const catalog = await client.catalog();
  if (
    catalog.schema !== "moly-asset-packs/2" ||
    (region !== undefined && catalog.region !== region) ||
    (version !== undefined && catalog.version !== version)
  )
    throw new Error("Published store and snapshot release differ");
  const entries = new Map(),
    verified = new Map();
  let transferBytes = 0;
  for (const row of catalog.packages) {
    const loaded = await client.package(row);
    for (const [logical, entry] of loaded.entries) {
      entries.set(logical, entry);
      const representation = JSON.stringify([
        entry.blob_bytes,
        entry.bytes,
        entry.codec,
        entry.http_encoding,
        entry.content_sha256,
        entry.xf,
      ]);
      if (
        verified.has(entry.blob) &&
        verified.get(entry.blob) !== representation
      )
        throw new Error(
          `Shared blob has conflicting content metadata: ${logical}`,
        );
      if (!verified.has(entry.blob)) {
        await client.read(logical);
        verified.set(entry.blob, representation);
        transferBytes += entry.blob_bytes;
      }
      if (source) {
        // The projection/assets used by the publisher must come from the exact
        // selected runtime, not merely carry the same region/version labels.
        const filename = checkedFile(source, logical);
        if (fs.statSync(filename).size !== entry.bytes)
          throw new Error(
            `Runtime artifact differs from pinned store: ${logical}`,
          );
        const bytes = fs.readFileSync(filename);
        if (bytes.length !== entry.bytes || sha(bytes) !== entry.content_sha256)
          throw new Error(
            `Runtime artifact differs from pinned store: ${logical}`,
          );
      }
    }
  }
  return {
    catalog,
    catalogId,
    entries,
    logicalFiles: entries.size,
    uniqueBlobs: verified.size,
    transferBytes,
  };
}
