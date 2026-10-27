import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import { sha256 } from "./build-source.mjs";

export const UI_FONT_LICENSE_SOURCE =
  "crates/game/moly-core/assets/font/OFL-1.1.txt";
export const UI_FONT_LICENSE_PATH = "fonts/OFL-1.1.txt";

function requireReviewedLicense(bytes) {
  // Verify the reviewed upstream text while permitting Git's LF/CRLF checkout.
  // https://github.com/CyanoHao/Resource-Han-Rounded/blob/v0.990/LICENSE.txt
  const canonical = bytes.toString("utf8").replaceAll("\r\n", "\n");
  if (
    sha256(canonical) !==
    "7abe5f490780cd2fd5a67502b993e244ef87d631fb7b98244e9819ff590da4af"
  )
    throw new Error("Changed Resource Han Rounded copyright or OFL text");
}

/** The checked-in Resource Han Rounded license, including its copyright. */
export function sourceUiFontLicense(workspace) {
  const bytes = readFileSync(path.join(workspace, UI_FONT_LICENSE_SOURCE));
  requireReviewedLicense(bytes);
  return {
    bytes,
    record: {
      id: "OFL-1.1",
      path: UI_FONT_LICENSE_PATH,
      sha256: sha256(bytes),
      bytes: bytes.length,
    },
  };
}

/** Verify the license explicitly declared by an external UI font's build. */
export function readUiFontLicense(packageRoot, font) {
  const license = font?.license;
  if (
    !license ||
    license.id !== "OFL-1.1" ||
    license.path !== UI_FONT_LICENSE_PATH ||
    !/^[a-f0-9]{64}$/.test(license.sha256 ?? "") ||
    !Number.isSafeInteger(license.bytes) ||
    license.bytes <= 0 ||
    license.bytes > 64 * 1024
  )
    throw new Error("Missing or invalid UI font license record");
  const bytes = readFileSync(path.join(packageRoot, license.path));
  if (bytes.length !== license.bytes || sha256(bytes) !== license.sha256)
    throw new Error("Changed UI font license");
  requireReviewedLicense(bytes);
  return bytes;
}

/** Used by native archive packaging as well as the browser build. */
export function copyUiFontLicense(workspace, destination) {
  const { bytes, record } = sourceUiFontLicense(workspace);
  const filename = path.join(destination, record.path);
  mkdirSync(path.dirname(filename), { recursive: true });
  writeFileSync(filename, bytes);
  return record;
}

/** Check downloaded renderer artifacts before the release archives are made. */
export function verifyUiFontPackage(packageRoot) {
  const fonts = ["webgpu", "webgl2"].map(
    (backend) =>
      JSON.parse(readFileSync(path.join(packageRoot, backend, "build.json")))
        .uiFont,
  );
  const [font] = fonts;
  if (
    !font ||
    !isDeepStrictEqual(font, fonts[1]) ||
    /^fonts\/([a-f0-9]{64})\.ttf$/.exec(font.path ?? "")?.[1] !== font.sha256 ||
    !Number.isSafeInteger(font.bytes) ||
    font.bytes <= 0 ||
    font.bytes > 64 * 1024 * 1024
  )
    throw new Error("Both engines must name the same valid UI font");
  const bytes = readFileSync(path.join(packageRoot, font.path));
  if (bytes.length !== font.bytes || sha256(bytes) !== font.sha256)
    throw new Error("Changed UI font");
  readUiFontLicense(packageRoot, font);
  return font;
}

const self = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === self) {
  const [action, ...destinations] = process.argv.slice(2);
  if (!["--copy-license", "--verify"].includes(action) || !destinations.length)
    throw new Error(
      "Usage: node web/ui-font-package.mjs --copy-license|--verify <package-directory>...",
    );
  const workspace = path.resolve(path.dirname(self), "..");
  for (const destination of destinations) {
    if (action === "--copy-license") copyUiFontLicense(workspace, destination);
    else verifyUiFontPackage(destination);
  }
}
