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
  if (documents["fixture-models/index.json"].version !== 3 || documents["fixture-attach/attach-points.json"].version !== 2)
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

/** Validate the source before requesting/initializing WASM. Old immutable
 * releases keep their own old module; this module belongs only to the v1 engine. */
export async function preflightCoordinates(options, { fetchImpl = fetch, signal } = {}) {
  const { assets, region, version, packs, assetCatalog, stageUrl } = options;
  const base = new URL(assets), stage = new URL(stageUrl);
  const releaseId = /^\/moly\/releases\/([a-z0-9][a-z0-9._-]{0,95})\/stage\.html$/.exec(stage.pathname)?.[1];
  const looseId = /^\/moly\/snapshots\/([a-z0-9][a-z0-9._-]{0,95})\/assets\/$/.exec(base.pathname)?.[1];
  const snapshotId = options.snapshotId || looseId;
  if (options.snapshotId && looseId && options.snapshotId !== looseId)
    throw new Error("Coordinate snapshot path differs from selected identity");
  let snapshot;
  if (snapshotId) {
    if (!/^[a-z0-9][a-z0-9._-]{0,95}$/.test(snapshotId)) throw new Error("Invalid coordinate snapshot identity");
    const origin = options.resourceOrigin || stage.origin;
    const [releaseBytes, snapshotBytes] = await Promise.all([
      bytesFrom(new URL("./integrity.json", stage), 1048576, fetchImpl, signal),
      bytesFrom(new URL(`/moly/snapshots/${snapshotId}/snapshot.json`, origin), 1048576, fetchImpl, signal),
    ]);
    snapshot = parse(snapshotBytes);
    if (!releaseId) throw new Error("Coordinate release is not an immutable stage path");
    validateCoordinatePair(parse(releaseBytes), snapshot, { region, version, snapshotId, releaseId,
      ...(packs ? { assetCatalog } : {}) });
    const expectedAssets = packs ? "/moly/asset-store/" : `/moly/snapshots/${snapshotId}/assets/`;
    if (snapshot.assets !== expectedAssets || Boolean(snapshot.packs) !== Boolean(packs))
      throw new Error("Coordinate snapshot asset routing mismatch");
  } else if (packs) throw new Error("Packed coordinate preflight requires an explicit snapshot identity");
  const client = packs ? new PackClient(assets, assetCatalog, { fetchImpl, signal, required: true }) : null;
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
