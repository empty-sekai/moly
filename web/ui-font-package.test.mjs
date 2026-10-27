import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { sha256 } from "./build-source.mjs";
import {
  copyUiFontLicense,
  readUiFontLicense,
  sourceUiFontLicense,
  UI_FONT_LICENSE_SOURCE,
  verifyUiFontPackage,
} from "./ui-font-package.mjs";

const workspace = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

function temporaryDirectory(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "moly-font-package-"));
  t.after(() => {
    assert.equal(path.dirname(root), path.resolve(os.tmpdir()));
    assert.match(path.basename(root), /^moly-font-package-/);
    fs.rmSync(root, { recursive: true, force: true });
  });
  return root;
}

test("native packages copy the reviewed OFL copyright and license without changing bytes", (t) => {
  const root = temporaryDirectory(t);
  const record = copyUiFontLicense(workspace, root);
  const source = fs.readFileSync(path.join(workspace, UI_FONT_LICENSE_SOURCE));
  assert.deepEqual(fs.readFileSync(path.join(root, record.path)), source);
  assert.equal(record.id, "OFL-1.1");
  assert.equal(record.sha256, sha256(source));
  assert.equal(record.bytes, source.length);
  assert.deepEqual(readUiFontLicense(root, { license: record }), source);

  fs.unlinkSync(path.join(root, record.path));
  assert.throws(() => readUiFontLicense(root, { license: record }), /ENOENT/);
  assert.throws(() => sourceUiFontLicense(root), /ENOENT/);
});

test("font source license accepts Git line endings but rejects a different copyright or license", (t) => {
  const root = temporaryDirectory(t);
  const filename = path.join(root, UI_FONT_LICENSE_SOURCE);
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  const canonical = sourceUiFontLicense(workspace)
    .bytes.toString("utf8")
    .replaceAll("\r\n", "\n");
  for (const text of [canonical, canonical.replaceAll("\n", "\r\n")]) {
    fs.writeFileSync(filename, text);
    assert.equal(sourceUiFontLicense(root).record.sha256, sha256(text));
  }
  fs.writeFileSync(filename, canonical.replace("Cyano Hao", "Unknown Author"));
  assert.throws(
    () => sourceUiFontLicense(root),
    /Changed Resource Han Rounded/,
  );
});

test("browser archive verification requires matching renderer records and the complete license", (t) => {
  const root = temporaryDirectory(t);
  const license = copyUiFontLicense(workspace, root);
  const bytes = Buffer.from(
    "synthetic font for packaging, not a playable font",
  );
  const font = {
    path: `fonts/${sha256(bytes)}.ttf`,
    sha256: sha256(bytes),
    bytes: bytes.length,
    license,
  };
  fs.writeFileSync(path.join(root, font.path), bytes);
  const writeRecords = (record = font) => {
    for (const backend of ["webgpu", "webgl2"]) {
      fs.mkdirSync(path.join(root, backend), { recursive: true });
      fs.writeFileSync(
        path.join(root, backend, "build.json"),
        JSON.stringify({ uiFont: record }),
      );
    }
  };
  writeRecords();
  assert.deepEqual(verifyUiFontPackage(root), font);

  for (const replacement of [
    undefined,
    { ...license, id: "unknown" },
    { ...license, path: "../OFL-1.1.txt" },
  ]) {
    writeRecords({ ...font, license: replacement });
    assert.throws(() => verifyUiFontPackage(root), /license record/);
  }
  writeRecords();
  fs.writeFileSync(path.join(root, license.path), "truncated license");
  assert.throws(() => verifyUiFontPackage(root), /Changed UI font license/);
  const unknown = Buffer.from(
    "Copyright unknown, not the reviewed font license",
  );
  fs.writeFileSync(path.join(root, license.path), unknown);
  writeRecords({
    ...font,
    license: { ...license, sha256: sha256(unknown), bytes: unknown.length },
  });
  assert.throws(
    () => verifyUiFontPackage(root),
    /Changed Resource Han Rounded/,
  );
  writeRecords();
  fs.unlinkSync(path.join(root, license.path));
  assert.throws(() => verifyUiFontPackage(root), /ENOENT/);
});
