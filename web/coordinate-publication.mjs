import fs from "node:fs";
import path from "node:path";
import { sha256 } from "./build-source.mjs";
import { COORDINATE_CONTRACT, COORDINATE_DOCUMENTS, requireCoordinateContract, validateCoordinateSources } from "./coordinate-contract.mjs";

function read(root, name) {
  if (typeof name !== "string" || /[:\\]/.test(name) || name.split("/").some(part => !part || part.startsWith(".")))
    throw new Error("Unsafe coordinate evidence path");
  const file = fs.realpathSync(path.join(root, name));
  const relative = path.relative(root, file);
  if (relative.startsWith("..") || path.isAbsolute(relative)) throw new Error("Coordinate evidence escaped source");
  if (fs.statSync(file).size > 128 * 1048576) throw new Error("Coordinate evidence too large");
  return fs.readFileSync(file);
}

function collisionReference(gltf, reference, label) {
  if (!reference || typeof reference !== "object" || Array.isArray(reference) ||
      Object.keys(reference).sort().join(",") !== "node,schemaVersion" ||
      reference.schemaVersion !== 1 || !Number.isSafeInteger(reference.node) || reference.node < 0)
    throw new Error(`${label}: invalid collision document reference`);
  const index = reference.node, node = gltf.nodes?.[index];
  if (!node || node.name !== "__moly_fixture_collision" ||
      Object.keys(node).some(key => !["name", "extras"].includes(key)) ||
      node.extras?.fixtureCollisionRef || node.extras?.sourceCollision ||
      gltf.scenes?.some(scene => scene.nodes?.includes(index)) ||
      gltf.nodes?.some(parent => parent.children?.includes(index)) ||
      gltf.skins?.some(skin => skin.skeleton === index || skin.joints?.includes(index)) ||
      gltf.animations?.some(animation => animation.channels?.some(channel => channel.target?.node === index)))
    throw new Error(`${label}: collision metadata node must stay outside rendered scenes`);
  const document = node.extras?.fixtureCollision;
  requireCoordinateContract(document, `${label} collision`);
  if (document.schemaVersion !== 1 || document.units !== "source-unity-unit" || !Array.isArray(document.geometry))
    throw new Error(`${label}: unsupported collision document schema`);
  return document;
}
export function validatePublicationCoordinates(assets, identity) {
  const root = fs.realpathSync(assets), documents = {}, hashes = {};
  for (const name of COORDINATE_DOCUMENTS) {
    const bytes = read(root, name);
    if (bytes.length > 32 * 1048576) throw new Error("Coordinate document too large");
    documents[name] = JSON.parse(bytes);
    hashes[name] = sha256(bytes);
  }
  const packages = validateCoordinateSources(documents, identity);
  for (const [name, row] of Object.entries(packages))
    if (row.status === "exported" && !fs.existsSync(path.join(root, "fixture-models", row.glb)))
      throw new Error(`Coordinate fixture model is missing: ${name}`);
  const models = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a,b) => a.name < b.name ? -1 : 1)) {
      if (entry.name.startsWith(".") || entry.name === "_previous" || /\.(?:bak|staging)$/.test(entry.name) || entry.name.includes(".pre-")) continue;
      const file = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error("Coordinate source cannot contain linked model paths");
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile() && /\.glb$/.test(entry.name)) models.push(path.relative(root, file).split(path.sep).join("/"));
    }
  }
  walk(root);
  const modelHashes = [];
  // Validate every published family, including actor libraries and site GLBs.
  // The aggregate is pinned in the descriptor; browsers need not refetch all
  // geometry simply to determine release compatibility before WASM.
  for (const relative of models) {
      const name = relative, bytes = read(root, relative);
      if (bytes.length < 20 || bytes.toString("ascii", 0, 4) !== "glTF" || bytes.readUInt32LE(4) !== 2 || bytes.readUInt32LE(8) !== bytes.length || bytes.toString("ascii", 16, 20) !== "JSON")
        throw new Error(`Invalid coordinate GLB evidence: ${name}`);
      const length = bytes.readUInt32LE(12);
      if (20 + length > bytes.length) throw new Error("Truncated coordinate GLB evidence");
      const gltf = JSON.parse(bytes.subarray(20, 20 + length));
      requireCoordinateContract(gltf.asset?.extras, `${name} GLB asset`);
      const roots = gltf.scenes?.flatMap(scene => scene.nodes ?? []) ?? [];
      const reference = gltf.extras?.fixtureCollisionRef;
      if (reference) {
        if (gltf.extras.fixtureCollision) throw new Error(`${name}: ambiguous collision storage`);
        collisionReference(gltf, reference, name);
      }
      if (documents["fixture-models/index.json"].version === 4 && name.startsWith("fixture-models/") && !reference)
        throw new Error(`${name}: v4 fixture collision reference missing`);
      for (const index of roots) {
        const extras = gltf.nodes?.[index]?.extras;
        requireCoordinateContract(extras, `${name} GLB root`);
        if (extras.fixtureCollision) requireCoordinateContract(extras.fixtureCollision, `${name} collision`);
        if (reference && (extras.fixtureCollision || extras.fixtureCollisionRef?.node !== reference.node ||
            extras.fixtureCollisionRef?.schemaVersion !== 1))
          throw new Error(`${name}: scene collision reference differs from its asset`);
      }
      for (const node of gltf.nodes ?? []) {
        const extras = node.extras;
        if (extras?.coordinateContract !== undefined) requireCoordinateContract(extras, `${name} node`);
        if (extras?.fixtureCollision) requireCoordinateContract(extras.fixtureCollision, `${name} collision`);
        if (extras?.fixtureCollisionRef) {
          if (!reference || extras.fixtureCollision) throw new Error(`${name}: ambiguous collision reference`);
          collisionReference(gltf, extras.fixtureCollisionRef, name);
        }
        for (const obstacle of extras?.navMeshObstacles ?? []) requireCoordinateContract(obstacle, `${name} obstacle`);
      }
      modelHashes.push([relative, sha256(bytes)]);
  }
  return { coordinateContract: COORDINATE_CONTRACT, coordinateDocuments: hashes,
    coordinateModels: { files: modelHashes.length, sha256: sha256(JSON.stringify(modelHashes)) } };
}
