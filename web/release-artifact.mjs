import { verifyPublishedStore } from "./asset-pack-store.mjs";
import { resolveResourceSource } from "./resource-sources.mjs";
import {
  existsSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  renameSync,
  symlinkSync,
  realpathSync,
  lstatSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  gzipSync,
  brotliCompressSync,
  constants as zlibConstants,
} from "node:zlib";
import { contentKey, EMBED_VERSION } from "./embed-contract.mjs";
import { workspaceFingerprint, sha256 } from "./build-source.mjs";

export const STAGE_FILES = [
  "embed.mjs",
  "embed-stage.mjs",
  "embed-contract.mjs",
  "embed.d.ts",
  "stage.html",
  "stage.mjs",
  "base-resources.mjs",
  "asset-pack-client.mjs",
  "stage.css",
  "stage-controller.mjs",
  "stage-activation.mjs",
  "stage-locale.mjs",
  "weather-presentation.mjs",
  "weather-artwork.mjs",
  "weather-picker.mjs",
  "weather-ui-locale.mjs",
  "boot.mjs",
];
const MAGIC = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);

function leb(bytes, offset) {
  let value = 0,
    shift = 0;
  for (let index = 0; index < 5; index++) {
    if (offset >= bytes.length)
      throw new Error("Truncated WASM section length");
    const byte = bytes[offset++];
    if (index === 4 && byte > 15)
      throw new Error("WASM section length overflow");
    value += (byte & 127) * 2 ** shift;
    if (!(byte & 128)) return { value, offset };
    shift += 7;
  }
  throw new Error("Invalid WASM section length");
}

/** Remove debug names only. Every standard section and runtime byte stays exact. */
export function stripDebugNames(input) {
  const bytes = Buffer.from(input);
  if (!bytes.subarray(0, 8).equals(MAGIC))
    throw new Error("Unsupported WASM header");
  const chunks = [bytes.subarray(0, 8)];
  let offset = 8;
  while (offset < bytes.length) {
    const start = offset,
      id = bytes[offset++];
    const size = leb(bytes, offset);
    offset = size.offset;
    const end = offset + size.value;
    if (end > bytes.length) throw new Error("Truncated WASM section");
    let discard = false;
    if (id === 0) {
      const name = leb(bytes, offset);
      if (name.offset + name.value > end)
        throw new Error("Invalid WASM custom section name");
      const label = bytes
        .subarray(name.offset, name.offset + name.value)
        .toString("utf8");
      discard = label === "name" || label.startsWith(".debug_");
    }
    if (!discard) chunks.push(bytes.subarray(start, end));
    offset = end;
  }
  return Buffer.concat(chunks);
}

export function splitCatalog(raw, snapshotId) {
  if (
    raw.schemaVersion !== 1 ||
    raw.ready !== true ||
    raw.mode !== "independent" ||
    !["cn", "jp", "tw", "en", "kr"].includes(raw.region) ||
    !/^\d+\.\d+\.\d+$/.test(raw.version) ||
    !Array.isArray(raw.entries) ||
    !Array.isArray(raw.characters) ||
    !/^[a-z0-9][a-z0-9._-]{0,95}$/.test(snapshotId)
  )
    throw new Error("Invalid Rust catalog export");
  const keys = new Set(),
    files = new Map(),
    entries = [];
  for (const original of raw.entries) {
    const key = contentKey(original.key);
    if (keys.has(key)) throw new Error(`Duplicate content identity: ${key}`);
    keys.add(key);
    if (
      !original.presentation ||
      typeof original.available !== "boolean" ||
      !Array.isArray(original.fixtureIds) ||
      !Array.isArray(original.unitIds) ||
      !Array.isArray(original.characters)
    )
      throw new Error(`Invalid runtime projection: ${key}`);
    const detail = `entries/${key.replaceAll(":", "-")}.json`;
    files.set(
      detail,
      Buffer.from(
        JSON.stringify({ schemaVersion: 1, snapshotId, entry: original }),
      ),
    );
    // This is a transport projection, not an eligibility or playback rule.
    const {
      lines: _lines,
      description: _description,
      fixtures: _fixtures,
      related: _related,
      ...card
    } = original;
    entries.push({ ...card, detail });
  }
  const index = {
    schemaVersion: 1,
    snapshotId,
    region: raw.region,
    version: raw.version,
    characters: raw.characters,
    entries,
  };
  files.set("index.json", Buffer.from(JSON.stringify(index)));
  return { files, index };
}

function inside(parent, child) {
  const relative = path.relative(parent, child);
  return (
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}
function json(file) {
  return JSON.parse(readFileSync(file, "utf8"));
}
function put(file, bytes) {
  mkdirSync(path.dirname(file), { recursive: true });
  if (existsSync(file)) {
    if (!readFileSync(file).equals(bytes))
      throw new Error(`Refusing to mutate an immutable artifact: ${file}`);
  } else writeFileSync(file, bytes, { flag: "wx" });
}
function compressedPut(file, bytes) {
  put(file, bytes);
  const gzip = gzipSync(bytes, { level: 9 });
  put(`${file}.gz`, gzip);
  // Precompress only large executable payloads. Browsers select br or gzip;
  // no runtime decompressor or WASM bytes are added to the JavaScript bundle.
  if (file.endsWith(".wasm"))
    put(
      `${file}.br`,
      brotliCompressSync(bytes, {
        params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 9 },
      }),
    );
  return { downloadBytes: gzip.length, decodedBytes: bytes.length };
}

/** Build artifacts only; proprietary source resources remain outside git/images. */
export async function publish({
  workspace,
  output,
  sources,
  reuseSnapshots = false,
  reuseEngine = false,
  developmentLinks = false,
  packageRoot,
}) {
  workspace = realpathSync(workspace);
  output = path.resolve(output);
  if (!reuseEngine)
    packageRoot = realpathSync(packageRoot ?? path.join(workspace, "web/pkg"));
  if (inside(workspace, output) || path.basename(output) === "moly-deploy")
    throw new Error("Publish outside source repositories");
  const manifestPath = path.join(output, "manifest.json");
  const retained = existsSync(manifestPath) ? json(manifestPath) : null;
  if (retained && retained.publisher !== "moly-release-artifact-v1")
    throw new Error("Output contains an unrelated manifest");
  if (reuseSnapshots) {
    if (sources !== undefined || !retained || retained.schemaVersion !== EMBED_VERSION ||
        !Array.isArray(retained.snapshots) || retained.snapshots.length < 1 || retained.snapshots.length > 2 ||
        retained.snapshots.some((snapshot) =>
          !/^[a-z0-9][a-z0-9._-]{0,95}$/.test(snapshot.id) ||
          !["cn", "jp"].includes(snapshot.region) ||
          !/^\d+\.\d+\.\d+$/.test(snapshot.version) ||
          snapshot.catalog !== `/moly/snapshots/${snapshot.id}/catalog/index.json` ||
          snapshot.assets !== (snapshot.packs ? "/moly/asset-store/" : `/moly/snapshots/${snapshot.id}/assets/`)))
      throw new Error("Cannot reuse an unverified source-qualified publication");
    const regions = new Set();
    for (const snapshot of retained.snapshots) {
      if (regions.has(snapshot.region)) throw new Error("Duplicate retained source region");
      regions.add(snapshot.region);
      const root = path.join(output, "snapshots", snapshot.id);
      const catalog = json(path.join(root, "catalog/index.json"));
      if (catalog.schemaVersion !== 1 || catalog.snapshotId !== snapshot.id ||
          catalog.region !== snapshot.region || catalog.version !== snapshot.version)
        throw new Error("Retained catalog source mismatch");
      if (!snapshot.packs) {
        const fixture = json(path.join(root, "assets/mysekai-fixtures.json"));
        if (fixture.region !== snapshot.region || fixture.gameVersion !== snapshot.version)
          throw new Error("Retained asset source mismatch");
      }
    }
  } else if (!Array.isArray(sources) || sources.length < 1 || sources.length > 5)
    throw new Error("Publish one to five explicit region snapshots");
  // A shell-only release reuses an engine that was already published and
  // verified, rather than rebuilding one: the reused bytes are checked against
  // the integrity record of the release they come from, and the fingerprint
  // recorded here stays the one that actually produced that engine.
  const reusedId =
    reuseEngine === true ? (retained?.release?.id ?? null) : reuseEngine || null;
  if (reuseEngine && !/^[a-z0-9][a-z0-9._-]{0,95}$/.test(reusedId ?? ""))
    throw new Error("Cannot reuse an engine from an unnamed release");
  const reusedRoot = reusedId ? path.join(output, "releases", reusedId) : null;
  const reused = reusedRoot
    ? json(path.join(reusedRoot, "integrity.json"))
    : null;
  if (
    reused &&
    (reused.schemaVersion !== 1 || reused.contractVersion !== EMBED_VERSION)
  )
    throw new Error("Cannot reuse an engine from an unreadable release");
  const sourceFingerprint = reused
    ? reused.sourceFingerprint
    : workspaceFingerprint(workspace);
  const files = new Map(),
    engines = {};
  for (const relative of STAGE_FILES)
    files.set(relative, readFileSync(path.join(workspace, "web", relative)));
  for (const backend of ["webgpu", "webgl2"]) {
    const directory = reused
      ? path.join(reusedRoot, "pkg", backend)
      : path.join(packageRoot, backend);
    const build = json(path.join(directory, "build.json"));
    if (
      build.schemaVersion !== 1 ||
      build.backend !== backend ||
      build.sourceFingerprint !== sourceFingerprint
    )
      throw new Error(`Stale ${backend} build; run web/build-wasm.mjs`);
    for (const name of ["moly-app.js", "moly-app_bg.wasm"]) {
      const original = readFileSync(path.join(directory, name));
      if (
        reused
          ? sha256(original) !== reused.hashes[`pkg/${backend}/${name}`]
          : sha256(original) !== build.files[name]
      )
        throw new Error(`Changed ${backend} output: ${name}`);
      // Published engine bytes already had their debug names stripped.
      const bytes =
        name.endsWith(".wasm") && !reused
          ? stripDebugNames(original)
          : original;
      if (name.endsWith(".wasm")) {
        const before = await WebAssembly.compile(original),
          after = await WebAssembly.compile(bytes);
        for (const method of ["imports", "exports"]) {
          if (
            JSON.stringify(WebAssembly.Module[method](before)) !==
            JSON.stringify(WebAssembly.Module[method](after))
          )
            throw new Error(`WASM ${method} changed`);
        }
        const exports = WebAssembly.Module.exports(after).map(
          (value) => value.name,
        );
        for (const entry of [
          "start_stage",
          "library_snapshot",
          "library_command",
          "library_catalog",
          "library_diagnostics",
        ])
          if (!exports.includes(entry))
            throw new Error(`Missing stage ABI: ${entry}`);
        const gzipBytes = gzipSync(bytes, { level: 9 }).length;
        const brotliBytes = brotliCompressSync(bytes, {
          params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 9 },
        }).length;
        engines[backend] = {
          downloadBytes: brotliBytes,
          brotliBytes,
          gzipBytes,
          decodedBytes: bytes.length,
        };
      }
      files.set(`pkg/${backend}/${name}`, bytes);
    }
    files.set(
      `pkg/${backend}/build.json`,
      reused
        ? readFileSync(path.join(directory, "build.json"))
        : Buffer.from(JSON.stringify(build)),
    );
  }
  const hashes = Object.fromEntries(
    [...files].map(([name, bytes]) => [name, sha256(bytes)]),
  );
  const releaseId = `stage-${sha256(JSON.stringify(hashes)).slice(0, 20)}`;
  const releaseRoot = path.join(output, "releases", releaseId);
  for (const [name, bytes] of files)
    compressedPut(path.join(releaseRoot, name), bytes);
  put(
    path.join(releaseRoot, "integrity.json"),
    Buffer.from(
      JSON.stringify(
        {
          schemaVersion: 1,
          contractVersion: EMBED_VERSION,
          sourceFingerprint,
          hashes,
        },
        null,
        2,
      ),
    ),
  );
  const snapshots = reuseSnapshots ? retained.snapshots.map((snapshot) => ({ ...snapshot })) : [],
    regions = new Set();
  for (const specification of sources ?? []) {
    const source = await resolveResourceSource(specification, {
      cacheRoot: path.join(output, ".source-cache"),
    });
    if (
      !["cn", "jp", "tw", "en", "kr"].includes(source.region) ||
      regions.has(source.region)
    )
      throw new Error("Unsupported or duplicated source region");
    regions.add(source.region);
    const assets = realpathSync(source.assets),
      catalogBytes = readFileSync(source.catalog),
      catalog = JSON.parse(catalogBytes);
    const fixtureBytes = readFileSync(
        path.join(assets, "mysekai-fixtures.json"),
      ),
      fixture = JSON.parse(fixtureBytes);
    if (
      fixture.region !== source.region ||
      catalog.region !== source.region ||
      fixture.gameVersion !== catalog.version
    )
      throw new Error("Catalog and mounted assets have different provenance");
    const provenance = existsSync(path.join(assets, "source.json"))
      ? json(path.join(assets, "source.json"))
      : {};
    if (provenance.source?.region && provenance.source.region !== source.region)
      throw new Error("Source manifest region mismatch");
    const controllerIndexBytes = readFileSync(
      path.join(assets, "fixture-gimmick/browser-index.json"),
    );
    const controllerIndex = JSON.parse(controllerIndexBytes);
    if (
      controllerIndex.schemaVersion !== 1 ||
      controllerIndex.generator !== "moly-gimmick-split-v1" ||
      controllerIndex.region !== source.region ||
      controllerIndex.gameVersion !== catalog.version
    )
      throw new Error(
        "Regenerate source-qualified controller packages with web/split-gimmicks.mjs before publishing",
      );
    const basePath = path.join(assets, "browser-base.json");
    const baseBytes = existsSync(basePath)
      ? readFileSync(basePath)
      : Buffer.alloc(0);
    const base = baseBytes.length ? JSON.parse(baseBytes) : null;
    if (
      base &&
      (base.schemaVersion !== 1 ||
        base.generator !== "moly-browser-base-v1" ||
        base.region !== source.region ||
        base.gameVersion !== catalog.version ||
        !Array.isArray(base.files))
    )
      throw new Error("Invalid measured base resource descriptor");
    if (base) {
      let decoded = 0,
        download = 0;
      for (const resource of base.files) {
        if (
          typeof resource.path !== "string" ||
          resource.path.includes("\\") ||
          resource.path.includes(":") ||
          resource.path.split("/").some((part) => !part || part.startsWith("."))
        )
          throw new Error("Invalid base resource path");
        const file = path.join(assets, resource.path);
        if (!inside(assets, realpathSync(file)))
          throw new Error("Base resource escaped source root");
        const bytes = readFileSync(file);
        if (
          sha256(bytes) !== resource.sha256 ||
          bytes.length !== resource.decodedBytes
        )
          throw new Error("Base resource changed after measurement");
        const transfer =
          resource.encoding === "gzip"
            ? readFileSync(file + ".gz").length
            : bytes.length;
        if (transfer !== resource.downloadBytes)
          throw new Error("Base resource transport changed after measurement");
        decoded += bytes.length;
        download += transfer;
      }
      if (decoded !== base.decodedBytes || download !== base.downloadBytes)
        throw new Error("Base resource totals do not match");
    }
    // Photos are exact source entities and are part of immutable snapshot
    // identity. Never publish a one-character sample as an all-character set.
    let portraitBytes = Buffer.alloc(0);
    const portraitFiles = [];
    if (source.portraits) {
      const photoRoot = realpathSync(source.portraits);
      portraitBytes = readFileSync(path.join(photoRoot, "manifest.json"));
      if (portraitBytes.length > 262144)
        throw new Error("Portrait index is too large");
      const photos = JSON.parse(portraitBytes);
      const entities = new Set(
        json(path.join(assets, "manifest.json")).units.map(
          (row) => Number(row.unit) - 100,
        ),
      );
      if (
        photos.schemaVersion !== 1 ||
        photos.generator !== "moly-root-chara-head-v1" ||
        photos.region !== source.region ||
        photos.version !== catalog.version ||
        photos.expected !== entities.size ||
        !Array.isArray(photos.portraits) ||
        photos.portraits.length !== entities.size
      )
        throw new Error("Incomplete source-rendered portrait set");
      for (const photo of photos.portraits) {
        if (
          !entities.delete(photo.unit) ||
          photo.file !== `unit-${photo.unit}.png` ||
          photo.model !== `sd_${photo.unit + 100}.glb` ||
          photo.rig !== `sd_${photo.unit + 100}.rig.json` ||
          photo.width !== 512 ||
          photo.height !== 512 ||
          photo.transparentPixels <= 0 ||
          photo.opaquePixels <= 0
        )
          throw new Error("Invalid SD portrait identity or alpha coverage");
        const name = path.join(photoRoot, photo.file);
        if (!inside(photoRoot, realpathSync(name)))
          throw new Error("Portrait escaped its source folder");
        const bytes = readFileSync(name);
        if (
          bytes.length < 33 ||
          bytes.length > 8 * 1048576 ||
          bytes.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a" ||
          bytes.readUInt32BE(16) !== 512 ||
          bytes.readUInt32BE(20) !== 512 ||
          bytes[25] !== 6
        )
          throw new Error("Portrait must be a real 512 x 512 RGBA PNG");
        if (photos.generator === "moly-root-chara-head-v1") {
          if (
            photos.preset?.bodyGeometry !== false ||
            photo.capture?.mode !== "head-only" ||
            photo.headBoundaryPixels !== 0 ||
            photo.sha256 !== sha256(bytes) ||
            photo.modelSha256 !==
              sha256(readFileSync(path.join(assets, photo.model))) ||
            photo.rigSha256 !==
              sha256(readFileSync(path.join(assets, photo.rig)))
          )
            throw new Error(
              "Head portrait does not match its original source or head-only capture",
            );
        }
        portraitFiles.push([photo.file, bytes]);
      }
      if (entities.size)
        throw new Error("Source SD entities are missing portraits");
    }
    const packed =
      source.assetCatalog !== undefined
        ? await verifyPublishedStore(
            path.join(output, "asset-store"),
            source.assetCatalog,
            { region: source.region, assets },
          )
        : null;
    if (packed)
      for (const required of [
        "browser-base.json",
        "mysekai-fixtures.json",
        "manifest.json",
        "fixture-gimmick/browser-index.json",
      ])
        if (!packed.entries.has(required))
          throw new Error(
            `Pinned pack omits required stage asset: ${required}`,
          );
    if (packed && base)
      for (const resource of base.files)
        if (!packed.entries.has(resource.path))
          throw new Error(
            `Pinned pack omits required base asset: ${resource.path}`,
          );
    const portraitDigest = portraitBytes.length
      ? sha256(
          Buffer.concat([
            portraitBytes,
            ...portraitFiles.map(([name, bytes]) =>
              Buffer.from(`${name}:${sha256(bytes)}`),
            ),
          ]),
        )
      : "";
    const id = `${source.region}-${catalog.version}-${sha256(Buffer.concat([catalogBytes, fixtureBytes, controllerIndexBytes, baseBytes, Buffer.from(JSON.stringify(provenance)), Buffer.from(JSON.stringify({ provider: source.provider, resourceIndexSha256: source.resourceIndexSha256, resourceOrigin: source.resourceOrigin, portraitDigest, ...(packed ? { assetCatalog: packed.catalogId } : {}) }))])).slice(0, 20)}`;
    const directory = path.join(output, "snapshots", id);
    const { files: catalogFiles } = splitCatalog(catalog, id);
    for (const [name, bytes] of catalogFiles)
      compressedPut(path.join(directory, "catalog", name), bytes);
    if (portraitBytes.length) {
      compressedPut(
        path.join(directory, "catalog/portraits/manifest.json"),
        portraitBytes,
      );
      for (const [name, bytes] of portraitFiles)
        put(path.join(directory, "catalog/portraits", name), bytes);
    }
    const mount = path.join(directory, "assets");
    if (packed) {
      // The store was verified, not copied. No per-snapshot binary mount exists.
    } else if (developmentLinks) {
      // The Go server contains all path resolution within its configured root.
      if (!inside(realpathSync(output), assets))
        throw new Error(
          "Development asset links must remain inside the serving root",
        );
      if (existsSync(mount)) {
        if (
          !lstatSync(mount).isSymbolicLink() ||
          realpathSync(mount) !== assets
        )
          throw new Error("Asset mount already exists with another identity");
      } else
        symlinkSync(
          assets,
          mount,
          process.platform === "win32" ? "junction" : "dir",
        );
    } else mkdirSync(mount, { recursive: true });
    const root = `/moly/snapshots/${id}/`;
    snapshots.push({
      id,
      region: source.region,
      version: catalog.version,
      assets: packed ? "/moly/asset-store/" : `${root}assets/`,
      ...(packed
        ? {
            packs: true,
            assetCatalog: packed.catalogId,
            assetReleaseVersion: packed.catalog.version,
          }
        : {}),
      catalog: `${root}catalog/index.json`,
      provenance: {
        resourceProvider: source.provider,
        ...(portraitDigest ? { portraitsSha256: portraitDigest } : {}),
        ...(source.resourceIndexSha256
          ? {
              resourceIndexSha256: source.resourceIndexSha256,
              resourceOrigin: source.resourceOrigin,
            }
          : {}),
        catalogSha256: sha256(catalogBytes),
        fixtureMasterSha256: sha256(fixtureBytes),
        controllerIndexSha256: sha256(controllerIndexBytes),
        ...(base ? { baseDescriptorSha256: sha256(baseBytes) } : {}),
        assetPolicy: packed
          ? "content-addressed-store"
          : developmentLinks
            ? "development-mount"
            : "immutable-readonly-mount",
        ...(packed
          ? {
              assetCatalogSha256: packed.catalogId,
              verifiedLogicalFiles: packed.logicalFiles,
            }
          : {}),
        ...(provenance.source?.assetVersion
          ? { assetVersion: provenance.source.assetVersion }
          : {}),
        ...(provenance.master?.commit
          ? { masterCommit: provenance.master.commit }
          : {}),
      },
      base: {
        downloadBytes:
          packed && base
            ? [
                ...new Map(
                  base.files.map((row) => {
                    const entry = packed.entries.get(row.path);
                    return [entry.blob, entry.blob_bytes];
                  }),
                ).values(),
              ].reduce((a, b) => a + b, 0)
            : (base?.downloadBytes ?? 0),
        decodedBytes: base?.decodedBytes ?? 0,
      },
    });
    // Keep the exact source-qualified descriptor independently addressable
    // after discovery moves on to a newer region release. Never rewrite it.
    put(
      path.join(directory, "snapshot.json"),
      Buffer.from(JSON.stringify(snapshots.at(-1), null, 2) + "\n"),
    );
    put(
      path.join(directory, "provenance.json"),
      Buffer.from(
        JSON.stringify(
          {
            schemaVersion: 1,
            snapshotId: id,
            source: provenance,
            policy: packed
              ? "Every runtime artifact is verified against the pinned package catalog; all binary objects live in the shared immutable asset store. Channels/default selection are mutable and never cached as content."
              : "This identity hashes the Rust catalogue, fixture master and provenance descriptor, not every binary. Production asset mounts MUST be immutable; replace the snapshot instead of editing the mount.",
          },
          null,
          2,
        ),
      ),
    );
  }
  const manifest = {
    publisher: "moly-release-artifact-v1",
    schemaVersion: EMBED_VERSION,
    release: {
      id: releaseId,
      module: `/moly/releases/${releaseId}/embed.mjs`,
      stage: `/moly/releases/${releaseId}/stage.html`,
      contractVersion: EMBED_VERSION,
      engines,
    },
    snapshots,
  };
  mkdirSync(output, { recursive: true });
  const workerPath = path.join(output, "cache-worker.mjs");
  const workerBytes = readFileSync(
    path.join(workspace, "web/cache-worker.mjs"),
  );
  if (
    existsSync(workerPath) &&
    !existsSync(manifestPath) &&
    !readFileSync(workerPath).equals(workerBytes)
  )
    throw new Error("Output contains an unrelated cache worker");
  const workerTemporary = `${workerPath}.${process.pid}.tmp`;
  writeFileSync(workerTemporary, workerBytes, { flag: "wx" });
  renameSync(workerTemporary, workerPath);
  const temporary = path.join(output, `manifest.${process.pid}.tmp`);
  writeFileSync(temporary, JSON.stringify(manifest, null, 2) + "\n", {
    flag: "wx",
  });
  renameSync(temporary, manifestPath);
  return manifest;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const args = process.argv.slice(2),
      configIndex = args.indexOf("--config");
    if (configIndex < 0 || !args[configIndex + 1])
      throw new Error(
        "Usage: node web/release-artifact.mjs --config /private/publication.json",
      );
    console.log(
      JSON.stringify(
        await publish(json(path.resolve(args[configIndex + 1]))),
        null,
        2,
      ),
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
