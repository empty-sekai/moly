// Prepare transport sidecars for assets actually requested by an empty stage.
// This never edits a source JSON/GLB, downloads content, or scans an entire game.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync, gunzipSync } from "node:zlib";
import { parseArgs } from "node:util";
import { sha256 } from "./build-source.mjs";

export function evidencePaths(evidence) {
  if (
    evidence.schemaVersion !== 1 ||
    evidence.failed !== false ||
    !["cn", "jp"].includes(evidence.options?.region)
  )
    throw new Error(
      "A successful source-qualified acceptance trace is required",
    );
  const source = new URL(
    evidence.options.assets ??
      (evidence.options.region === "cn" ? "/assets/" : "/assets-jp/"),
    evidence.options.origin,
  );
  const base = evidence.events.find(
    (event) => event.type === "base-resources",
  )?.data;
  if (!Array.isArray(base))
    throw new Error(
      "Evidence does not contain the empty-stage resource boundary",
    );
  const result = new Set();
  for (const entry of base) {
    const url = new URL(entry.name);
    if (
      url.origin !== source.origin ||
      !url.pathname.startsWith(source.pathname) ||
      url.search
    )
      continue;
    const relative = decodeURIComponent(
      url.pathname.slice(source.pathname.length),
    );
    if (
      !relative ||
      relative.includes("\\") ||
      relative.includes(":") ||
      relative.split("/").some((part) => !part || part.startsWith("."))
    )
      throw new Error("Unsafe evidence resource path");
    if (
      ![
        ".json",
        ".glb",
        ".gltf",
        ".bin",
        ".png",
        ".jpg",
        ".jpeg",
        ".webp",
        ".ogg",
        ".wav",
        ".mp3",
        ".ktx2",
      ].includes(path.extname(relative).toLowerCase())
    )
      throw new Error("Unsupported game resource extension");
    result.add(relative);
  }
  if (!result.has("motion-library.glb") || !result.has("mysekai-fixtures.json"))
    throw new Error("Incomplete empty-stage evidence");
  return [...result].sort();
}

export function prepareBrowserAssets(assetRoot, evidence) {
  const assets = fs.realpathSync(assetRoot);
  const master = JSON.parse(
    fs.readFileSync(path.join(assets, "mysekai-fixtures.json"), "utf8"),
  );
  if (master.region !== evidence.options.region)
    throw new Error("Evidence and asset region disagree");
  const source = evidence.events.find(
    (event) => event.type === "catalog",
  )?.data;
  if (source?.region !== master.region || source.version !== master.gameVersion)
    throw new Error("Evidence and asset version disagree");
  const files = [];
  for (const relative of evidencePaths(evidence)) {
    const filename = path.join(assets, relative),
      resolved = fs.realpathSync(filename);
    const within = path.relative(assets, resolved);
    if (
      within === ".." ||
      within.startsWith(`..${path.sep}`) ||
      path.isAbsolute(within)
    )
      throw new Error("Resource escaped its source root");
    const stat = fs.statSync(resolved);
    if (!stat.isFile() || stat.size > 192 * 1024 * 1024)
      throw new Error(`Unexpected base asset size: ${relative}`);
    const raw = fs.readFileSync(resolved),
      sourceHash = sha256(raw);
    let downloadBytes = raw.length,
      encoding = "identity";
    const sidecar = `${filename}.gz`;
    if (fs.existsSync(sidecar)) {
      const bytes = fs.readFileSync(sidecar);
      if (
        sha256(gunzipSync(bytes, { maxOutputLength: raw.length + 1 })) !==
        sourceHash
      )
        throw new Error(`Existing compression sidecar is stale: ${relative}`);
      downloadBytes = bytes.length;
      encoding = "gzip";
    } else if (
      raw.length >= 16384 &&
      [".json", ".glb", ".gltf", ".bin"].includes(path.extname(relative))
    ) {
      const bytes = gzipSync(raw, { level: 6 });
      if (bytes.length < raw.length * 0.95) {
        const temporary = `${sidecar}.${process.pid}.tmp`;
        fs.writeFileSync(temporary, bytes, { flag: "wx" });
        fs.renameSync(temporary, sidecar);
        downloadBytes = bytes.length;
        encoding = "gzip";
      }
    }
    files.push({
      path: relative,
      sha256: sourceHash,
      decodedBytes: raw.length,
      downloadBytes,
      encoding,
    });
  }
  const descriptor = {
    schemaVersion: 1,
    generator: "moly-browser-base-v1",
    region: master.region,
    gameVersion: master.gameVersion,
    measurement:
      "empty-stage observed resource set, not an offline-completeness guarantee",
    files,
    decodedBytes: files.reduce((sum, file) => sum + file.decodedBytes, 0),
    downloadBytes: files.reduce((sum, file) => sum + file.downloadBytes, 0),
  };
  const output = path.join(assets, "browser-base.json");
  if (
    fs.existsSync(output) &&
    JSON.parse(fs.readFileSync(output, "utf8")).generator !==
      descriptor.generator
  )
    throw new Error("Existing base descriptor belongs to another producer");
  const temporary = `${output}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, JSON.stringify(descriptor, null, 2), {
    flag: "wx",
  });
  fs.renameSync(temporary, output);
  return descriptor;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const { values } = parseArgs({
      options: { assets: { type: "string" }, evidence: { type: "string" } },
    });
    if (!values.assets || !values.evidence)
      throw new Error(
        "Usage: --assets /private/runtime --evidence /private/acceptance.json",
      );
    const result = prepareBrowserAssets(
      values.assets,
      JSON.parse(fs.readFileSync(values.evidence, "utf8")),
    );
    console.log(
      JSON.stringify({
        region: result.region,
        version: result.gameVersion,
        files: result.files.length,
        decodedBytes: result.decodedBytes,
        downloadBytes: result.downloadBytes,
      }),
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
