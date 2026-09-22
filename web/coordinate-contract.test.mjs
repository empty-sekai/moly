import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { COORDINATE_CONTRACT as contract, COORDINATE_DOCUMENTS, preflightCoordinates, validateCoordinatePair, validateCoordinateSources } from "./coordinate-contract.mjs";
import { validatePublicationCoordinates } from "./coordinate-publication.mjs";
const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const identity = { region: "cn", version: "6.0.0" };
function sourceDocuments() {
  return {
    "source.json": { coordinateContract: contract, source: { region: "cn", appVersion: "6.0.0" } },
    "fixture-models/index.json": { version: 3, coordinateContract: contract, packages: { sample: { status: "exported", glb: "sample.glb", coordinateContract: contract } } },
    "fixture-attach/attach-points.json": { version: 2, coordinateContract: contract, packages: {} },
  };
}
function descriptors(documents) {
  const release = { schemaVersion: 1, releaseId: "new", contractVersion: 2, coordinateContract: contract };
  const snapshot = { id: "cn-6.0.0-test", ...identity, coordinateContract: contract,
    assets: "/moly/snapshots/cn-6.0.0-test/assets/",
    provenance: { coordinateContract: contract, coordinateModels: {files: 1,sha256:"a".repeat(64)}, coordinateDocuments: Object.fromEntries(Object.entries(documents).map(([name, value]) => [name, sha(JSON.stringify(value))])) } };
  return { release, snapshot };
}
test("source declarations reject missing/mixed contracts and wrong provenance", () => {
  assert.doesNotThrow(() => validateCoordinateSources(sourceDocuments(), identity));
  for (const name of COORDINATE_DOCUMENTS) {
    const documents = sourceDocuments(); delete documents[name].coordinateContract;
    assert.throws(() => validateCoordinateSources(documents, identity), /coordinate contract/);
  }
  const mixed = sourceDocuments(); mixed["fixture-models/index.json"].packages.sample.coordinateContract = "unity";
  assert.throws(() => validateCoordinateSources(mixed, identity), /fixture sample/);
  const wrong = sourceDocuments(); wrong["source.json"].source.region = "jp";
  assert.throws(() => validateCoordinateSources(wrong, identity), /region\/version/);
});
test("latest engine refuses old pinned descriptors and mixed release/snapshot evidence", () => {
  const {release, snapshot} = descriptors(sourceDocuments());
  assert.doesNotThrow(() => validateCoordinatePair(release, snapshot, { ...identity, snapshotId: snapshot.id }));
  for (const change of [{coordinateContract:undefined}, {coordinateContract:"old-native"}, {provenance:{coordinateContract:contract}}])
    assert.throws(() => validateCoordinatePair(release, {...snapshot,...change}), /coordinate|Coordinate/);
  assert.throws(() => validateCoordinatePair({}, snapshot), /release/);
  assert.throws(() => validateCoordinatePair(release, snapshot, {snapshotId:"old-pin"}), /identity mismatch/);
});
test("preflight checks exact immutable descriptor pair and bytes before engine admission", async () => {
  const documents = sourceDocuments(), {release,snapshot} = descriptors(documents), calls = [];
  const options = { ...identity, assets:"https://cdn.test" + snapshot.assets, snapshotId:snapshot.id,
    stageUrl:"https://host.test/moly/releases/new/stage.html", resourceOrigin:"https://cdn.test" };
  const fetchImpl = async url => {
    const address = new URL(url); calls.push(address.href);
    const value = address.pathname.endsWith("integrity.json") ? release : address.pathname.endsWith("snapshot.json") ? snapshot : documents[address.pathname.split("/assets/")[1]];
    return new Response(JSON.stringify(value));
  };
  assert.equal((await preflightCoordinates(options,{fetchImpl})).coordinateContract, contract);
  assert.equal(calls.length, 5);
  assert.ok(calls.includes("https://cdn.test/moly/snapshots/cn-6.0.0-test/snapshot.json"));
  delete snapshot.coordinateContract; calls.length = 0;
  await assert.rejects(preflightCoordinates(options,{fetchImpl}), /snapshot: coordinate contract/);
  assert.equal(calls.length, 2, "legacy snapshot fails before source downloads or WASM");
  snapshot.coordinateContract = contract;
  documents["fixture-attach/attach-points.json"].packages.changed = {};
  await assert.rejects(preflightCoordinates(options,{fetchImpl}), /evidence changed/);
});
test("publication verifies every GLB/root/collision metadata, not only index labels", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-coordinate-source-"));
  try {
    const documents = sourceDocuments();
    for(const [name,document] of Object.entries(documents)) { const file=path.join(root,name);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,JSON.stringify(document)); }
    const gltf = { asset:{version:"2.0",extras:{coordinateContract:contract}}, scenes:[{nodes:[0]}],
      nodes:[{extras:{coordinateContract:contract,fixtureCollision:{coordinateContract:contract}}}] };
    const write = () => { let json=Buffer.from(JSON.stringify(gltf));json=Buffer.concat([json,Buffer.alloc((4-json.length%4)%4,32)]);const header=Buffer.alloc(20);header.write("glTF");header.writeUInt32LE(2,4);header.writeUInt32LE(20+json.length,8);header.writeUInt32LE(json.length,12);header.write("JSON",16);fs.writeFileSync(path.join(root,"fixture-models/sample.glb"),Buffer.concat([header,json])); };
    write(); const evidence = validatePublicationCoordinates(root,identity);
    assert.equal(evidence.coordinateModels.files, 1);
    assert.match(evidence.coordinateModels.sha256, /^[a-f0-9]{64}$/);
    const extra=path.join(root,"actor-animations/old.glb");fs.mkdirSync(path.dirname(extra),{recursive:true});
    fs.copyFileSync(path.join(root,"fixture-models/sample.glb"),extra);
    assert.equal(validatePublicationCoordinates(root,identity).coordinateModels.files,2);
    const bad=fs.readFileSync(extra);bad[24]=0;
    fs.writeFileSync(extra,bad);assert.throws(()=>validatePublicationCoordinates(root,identity));fs.unlinkSync(extra);
    gltf.nodes[0].extras.fixtureCollision.coordinateContract="old";write();
    assert.throws(()=>validatePublicationCoordinates(root,identity),/collision/);
    delete gltf.asset.extras.coordinateContract;write();
    assert.throws(()=>validatePublicationCoordinates(root,identity),/GLB asset/);
  } finally { fs.rmSync(root,{recursive:true,force:true}); }
});
test("stage invokes source preflight before importing or initializing WASM", () => {
  const source = fs.readFileSync(new URL("./stage.mjs",import.meta.url),"utf8");
  assert.ok(source.indexOf("await preflightCoordinates(") < source.indexOf("await import(path.href)"));
  assert.ok(source.indexOf("await preflightCoordinates(") < source.indexOf("await module.default("));
});
