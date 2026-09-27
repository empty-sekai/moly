import { PackClient, packDigest } from "./asset-pack-client.mjs";

export const COORDINATE_CONTRACT = "moly-rh-y-up-reflect-x-v1";
export const COORDINATE_DOCUMENTS = Object.freeze([
  "source.json", "fixture-models/index.json", "fixture-attach/attach-points.json",
]);
export function requireCoordinateContract(value, label) {
  if (value?.coordinateContract !== COORDINATE_CONTRACT)
    throw new Error(`${label}: coordinate contract missing or unsupported; re-export this snapshot`);
  return value;
}
export function validateCoordinateSources(documents, { region, version }) {
  for (const name of COORDINATE_DOCUMENTS) requireCoordinateContract(documents[name], name);
  const source = documents["source.json"].source;
  if (source?.region !== region || source.appVersion !== version)
    throw new Error("Coordinate source manifest region/version mismatch");
  const packages = documents["fixture-models/index.json"].packages;
  if (![3, 4].includes(documents["fixture-models/index.json"].version) || documents["fixture-attach/attach-points.json"].version !== 2)
    throw new Error("Coordinate source document schema mismatch; re-export this snapshot");
  if (!packages || typeof packages !== "object" || Array.isArray(packages))
    throw new Error("Coordinate fixture index has no package identities");
  for (const [name, entry] of Object.entries(packages))
    if (entry.status === "exported") requireCoordinateContract(entry, `fixture ${name}`);
  return packages;
}
export function validateCoordinatePair(release, snapshot, { region, version, snapshotId, assetCatalog, releaseId } = {}) {
  requireCoordinateContract(release, "release");
  requireCoordinateContract(snapshot, "snapshot");
  requireCoordinateContract(snapshot?.provenance, "snapshot provenance");
  if (releaseId !== undefined && release.releaseId !== releaseId) throw new Error("Coordinate release path identity mismatch");
  if ((region !== undefined && snapshot.region !== region) ||
      (version !== undefined && snapshot.version !== version) ||
      (snapshotId !== undefined && snapshot.id !== snapshotId) ||
      (assetCatalog !== undefined && snapshot.assetCatalog !== assetCatalog))
    throw new Error("Coordinate release/snapshot identity mismatch");
  const evidence = snapshot.provenance.coordinateDocuments;
  if (!evidence || COORDINATE_DOCUMENTS.some(name => !/^[a-f0-9]{64}$/.test(evidence[name] ?? "")))
    throw new Error("Coordinate snapshot has no verified source evidence");
  const models = snapshot.provenance.coordinateModels;
  if (!models || !Number.isSafeInteger(models.files) || models.files < 0 || !/^[a-f0-9]{64}$/.test(models.sha256 ?? ""))
    throw new Error("Coordinate snapshot has no verified model evidence");
}
async function bytesFrom(url, maximum, fetchImpl, signal) {
  const response = await fetchImpl(url, { signal, credentials: "omit", redirect: "error", cache: "no-store" });
  if (!response.ok || !response.body) throw new Error("Coordinate metadata unavailable");
  const reader = response.body.getReader(), chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > maximum) throw new Error("Coordinate metadata exceeds its bounded size");
      chunks.push(value);
    }
  } catch (error) { await reader.cancel().catch(() => {}); throw error; }
  finally { reader.releaseLock(); }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return bytes;
}
const parse = bytes => JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));

const IDENTITY = /^[a-z0-9][a-z0-9._-]{0,95}$/;
/** Validate the source before requesting/initializing WASM. Old immutable
 * releases keep their own old module; this module belongs only to the v1 engine.
 * `releaseId` names the immutable release the page belongs to and
 * `resourcePrefix` is the path its publication is mounted at on the page
 * origin (the logical `/moly/`); both are the caller's, never guessed here. */
export async function preflightCoordinates(options, { fetchImpl = fetch, signal } = {}) {
  const { assets, region, version, packs, assetCatalog, pageUrl, resourceBase, resourcePrefix } = options;
  if (typeof resourcePrefix !== "string" || !/^\/(?:[a-z0-9][a-z0-9._-]*\/)*$/.test(resourcePrefix))
    throw new Error("Coordinate preflight requires an explicit resource prefix");
  const base = new URL(assets), page = new URL(pageUrl);
  const releaseId = options.releaseId ?? undefined;
  if (releaseId !== undefined && !IDENTITY.test(releaseId)) throw new Error("Invalid coordinate release identity");
  const looseId = base.pathname.startsWith(resourcePrefix)
    ? /^snapshots\/([a-z0-9][a-z0-9._-]{0,95})\/assets\/$/.exec(base.pathname.slice(resourcePrefix.length))?.[1]
    : undefined;
  const snapshotId = options.snapshotId || looseId;
  if (options.snapshotId && looseId && options.snapshotId !== looseId)
    throw new Error("Coordinate snapshot path differs from selected identity");
  let snapshot;
  if (snapshotId) {
    if (!IDENTITY.test(snapshotId)) throw new Error("Invalid coordinate snapshot identity");
    if (!releaseId) throw new Error("Coordinate release is not an immutable stage path");
    const origin = options.resourceOrigin || page.origin;
    const remoteBase = resourceBase ? new URL(resourceBase) : null;
    const [releaseBytes, snapshotBytes] = await Promise.all([
      bytesFrom(remoteBase
        ? new URL(`releases/${releaseId}/integrity.json`, remoteBase)
        : new URL(`${resourcePrefix}releases/${releaseId}/integrity.json`, page.origin),
        1048576, fetchImpl, signal),
      bytesFrom(remoteBase
        ? new URL(`snapshots/${snapshotId}/snapshot.json`, remoteBase)
        : new URL(`${resourcePrefix}snapshots/${snapshotId}/snapshot.json`, origin),
        1048576, fetchImpl, signal),
    ]);
    snapshot = parse(snapshotBytes);
    validateCoordinatePair(parse(releaseBytes), snapshot, { region, version, snapshotId, releaseId,
      ...(packs ? { assetCatalog } : {}) });
    const expectedAssets = packs
      ? (remoteBase ? "asset-store/" : `${resourcePrefix}asset-store/`)
      : (remoteBase ? `snapshots/${snapshotId}/assets/` : `${resourcePrefix}snapshots/${snapshotId}/assets/`);
    // Published descriptors keep their logical prefixed paths; below a
    // resource base they resolve the way the host resolves them,
    // `<prefix><tail>` -> `<base><tail>`.
    const publishedAssets = remoteBase && typeof snapshot.assets === "string"
      ? snapshot.assets.startsWith(resourcePrefix)
        ? snapshot.assets.slice(resourcePrefix.length)
        : snapshot.assets.replace(/^\//, "")
      : snapshot.assets;
    if (publishedAssets !== expectedAssets || Boolean(snapshot.packs) !== Boolean(packs))
      throw new Error("Coordinate snapshot asset routing mismatch");
  } else if (packs) throw new Error("Packed coordinate preflight requires an explicit snapshot identity");
  const client = packs ? new PackClient(assets, assetCatalog, { fetchImpl, signal, required: true, resourceBase }) : null;
  const documents = {};
  for (const name of COORDINATE_DOCUMENTS) {
    const bytes = client ? await client.read(name) : await bytesFrom(new URL(name, base), 32 * 1048576, fetchImpl, signal);
    if (bytes.byteLength > 32 * 1048576) throw new Error("Coordinate metadata exceeds its bounded size");
    if (snapshot && await packDigest(bytes) !== snapshot.provenance.coordinateDocuments[name])
      throw new Error(`Coordinate source evidence changed: ${name}`);
    documents[name] = parse(bytes);
  }
  validateCoordinateSources(documents, { region, version });
  return { coordinateContract: COORDINATE_CONTRACT, snapshotId: snapshotId ?? null };
}
