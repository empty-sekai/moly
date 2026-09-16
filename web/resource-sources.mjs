// Operator-only resource provisioning. Browser requests cannot supply URLs.
// A remote provider must contain the converted, source-qualified Moly runtime,
// never raw Unity bundles or unverified files borrowed from another region.
import fs from "node:fs/promises";
import { createReadStream } from "node:fs";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";

const SHA = /^[a-f0-9]{64}$/;
const MAX_INDEX = 8 * 1024 * 1024;
const MAX_FILE = 512 * 1024 * 1024;
const MAX_SOURCE = 16 * 1024 * 1024 * 1024;
const REQUIRED = [
  "mysekai-fixtures.json",
  "manifest.json",
  "fixture-talks/talks.json",
  "fixture-models/player-data.json",
];
export function safeResourcePath(value) {
  return (
    typeof value === "string" &&
    value.length <= 512 &&
    !/[\\:%\x00-\x1f\x7f?#]/.test(value) &&
    value
      .split("/")
      .every(
        (part) =>
          part &&
          !part.startsWith(".") &&
          !/[. ]$/.test(part) &&
          !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part),
      )
  );
}
function sourceIdentity(region, version) {
  if (!["cn", "jp"].includes(region) || !/^\d+\.\d+\.\d+$/.test(version))
    throw new Error("Invalid source region/version");
}
export function validateResourceIndex(index, region, version) {
  sourceIdentity(region, version);
  if (
    !index ||
    index.schemaVersion !== 1 ||
    index.generator !== "moly-runtime-source-v1" ||
    index.region !== region ||
    index.gameVersion !== version ||
    !Array.isArray(index.files) ||
    index.files.length < REQUIRED.length ||
    index.files.length > 100000
  )
    throw new Error("Remote source identity mismatch");
  const seen = new Set();
  let total = 0;
  for (const row of index.files) {
    if (
      !safeResourcePath(row?.path) ||
      seen.has(row.path.toLowerCase()) ||
      !SHA.test(row.sha256) ||
      !Number.isSafeInteger(row.bytes) ||
      row.bytes < 0 ||
      row.bytes > MAX_FILE
    )
      throw new Error("Invalid remote file identity");
    seen.add(row.path.toLowerCase());
    total += row.bytes;
    if (total > MAX_SOURCE)
      throw new Error("Remote source exceeds provisioning budget");
  }
  if (
    !Number.isSafeInteger(index.totalBytes) ||
    index.totalBytes !== total ||
    REQUIRED.some((name) => !seen.has(name))
  )
    throw new Error("Incomplete remote runtime source");
  return index;
}
async function hashFile(file) {
  const hash = createHash("sha256");
  let bytes = 0;
  for await (const chunk of createReadStream(file)) {
    hash.update(chunk);
    bytes += chunk.length;
  }
  return { bytes, sha256: hash.digest("hex") };
}
async function verifiedFile(file, row) {
  try {
    const stat = await fs.lstat(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size !== row.bytes)
      return false;
    return (await hashFile(file)).sha256 === row.sha256;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}
function remoteBase(value) {
  const url = new URL(value);
  if (
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    !url.pathname.endsWith("/")
  )
    throw new Error(
      "Remote resources require a credential-free HTTPS directory",
    );
  return url;
}
async function readBounded(response, maximum) {
  if (!response.ok || Number(response.headers.get("content-length")) > maximum)
    throw new Error("Remote resource response invalid");
  const chunks = [];
  let bytes = 0;
  for await (const chunk of response.body) {
    bytes += chunk.length;
    if (bytes > maximum)
      throw new Error("Remote resource exceeded declared limit");
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks);
}
async function checkedParents(root, relative) {
  const parts = relative.split("/");
  parts.pop();
  let current = root;
  for (const part of parts) {
    current = path.join(current, part);
    await fs.mkdir(current, { recursive: true });
    const stat = await fs.lstat(current);
    if (!stat.isDirectory() || stat.isSymbolicLink())
      throw new Error("Resource cache path is not a private directory");
  }
}
/** Download a pinned manifest's exact files. No fallback between providers. */
export async function resolveResourceSource(
  source,
  { cacheRoot, fetchImpl = fetch } = {},
) {
  if (!source?.remote) {
    if (typeof source?.assets !== "string")
      throw new Error(
        "A local asset directory or explicit remote source is required",
      );
    if (source?.provider && source.provider !== "local")
      throw new Error("Provider does not match local asset configuration");
    return { ...source, provider: "local" };
  }
  if (source.assets || !cacheRoot)
    throw new Error("Remote/local source selection must be explicit");
  const remote = source.remote;
  if (
    !["moesekai-asset", "remote"].includes(remote.kind) ||
    !SHA.test(remote.indexSha256)
  )
    throw new Error("Remote index must be pinned to a source hash");
  sourceIdentity(source.region, source.version);
  const base = remoteBase(remote.baseUrl);
  const request = (url) =>
    fetchImpl(url, {
      redirect: "error",
      credentials: "omit",
      cache: "no-store",
      headers: { Accept: "application/octet-stream, application/json" },
      signal: AbortSignal.timeout(30000),
    });
  const indexBytes = await readBounded(
    await request(new URL("runtime-source.json", base)),
    MAX_INDEX,
  );
  if (
    createHash("sha256").update(indexBytes).digest("hex") !== remote.indexSha256
  )
    throw new Error("Remote index hash mismatch");
  const index = validateResourceIndex(
    JSON.parse(indexBytes.toString("utf8")),
    source.region,
    source.version,
  );
  const root = path.resolve(
    cacheRoot,
    `${source.region}-${source.version}-${remote.indexSha256}`,
  );
  await fs.mkdir(root, { recursive: true });
  if ((await fs.lstat(root)).isSymbolicLink())
    throw new Error("Remote cache root cannot be a symlink");
  let next = 0;
  const worker = async () => {
    while (next < index.files.length) {
      const row = index.files[next++];
      const file = path.join(root, ...row.path.split("/"));
      await checkedParents(root, row.path);
      if (await verifiedFile(file, row)) continue;
      const response = await request(new URL(row.path, base));
      if (
        !response.ok ||
        (response.headers.has("content-length") &&
          Number(response.headers.get("content-length")) !== row.bytes)
      )
        throw new Error(`Remote asset unavailable or wrong size: ${row.path}`);
      const temporary = `${file}.${randomUUID()}.tmp`;
      const handle = await fs.open(temporary, "wx");
      let bytes = 0;
      const hash = createHash("sha256");
      try {
        for await (const chunk of response.body) {
          bytes += chunk.length;
          if (bytes > row.bytes)
            throw new Error(
              `Remote asset exceeded declared bytes: ${row.path}`,
            );
          hash.update(chunk);
          await handle.writeFile(chunk);
        }
        if (bytes !== row.bytes || hash.digest("hex") !== row.sha256)
          throw new Error(`Remote asset hash mismatch: ${row.path}`);
        await handle.sync();
        await handle.close();
        await fs.rename(temporary, file);
      } finally {
        await handle.close().catch(() => {});
        await fs.rm(temporary, { force: true });
      }
    }
  };
  const completed = await Promise.allSettled(
    Array.from({ length: Math.min(4, index.files.length) }, worker),
  );
  const failed = completed.find((row) => row.status === "rejected");
  if (failed) throw failed.reason;
  const metadata = JSON.parse(
    await fs.readFile(path.join(root, "mysekai-fixtures.json"), "utf8"),
  );
  if (
    metadata.region !== source.region ||
    metadata.gameVersion !== source.version
  )
    throw new Error("Remote master identity mismatch");
  await fs.writeFile(path.join(root, "runtime-source.json"), indexBytes);
  return {
    ...source,
    assets: root,
    provider: remote.kind,
    resourceIndexSha256: remote.indexSha256,
    resourceOrigin: base.origin,
  };
}
/** Build the manifest beside an already-converted runtime for a CDN mirror. */
export async function createResourceIndex(root, region, version) {
  sourceIdentity(region, version);
  root = await fs.realpath(root);
  const metadata = JSON.parse(
    await fs.readFile(path.join(root, "mysekai-fixtures.json"), "utf8"),
  );
  if (metadata.region !== region || metadata.gameVersion !== version)
    throw new Error("Local runtime identity mismatch");
  const files = [];
  async function visit(relative = "") {
    const entries = await fs.readdir(path.join(root, relative), {
      withFileTypes: true,
    });
    entries.sort((a, b) => a.name.localeCompare(b.name, "en"));
    for (const entry of entries) {
      if (entry.name.startsWith(".") || entry.name === "runtime-source.json")
        continue;
      const name = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.isSymbolicLink() || !safeResourcePath(name))
        throw new Error("Non-portable source path");
      if (entry.isDirectory()) await visit(name);
      else if (entry.isFile())
        files.push({ path: name, ...(await hashFile(path.join(root, name))) });
    }
  }
  await visit();
  const index = validateResourceIndex(
    {
      schemaVersion: 1,
      generator: "moly-runtime-source-v1",
      region,
      gameVersion: version,
      totalBytes: files.reduce((total, row) => total + row.bytes, 0),
      files,
    },
    region,
    version,
  );
  const bytes = Buffer.from(JSON.stringify(index, null, 2) + "\n");
  await fs.writeFile(path.join(root, "runtime-source.json"), bytes);
  return {
    index,
    indexSha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
