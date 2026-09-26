// The loader a product page imports from an immutable release:
// `<publication>releases/<id>/game-boot.mjs`. It checks the selected source
// against the release before any engine byte is requested, warms the measured
// base resources, chooses a renderer and stream-compiles its engine. The page
// then calls `start(seed)` synchronously inside the user's tap.
import { warmBaseResources } from "./base-resources.mjs";
import { preflightCoordinates } from "./coordinate-contract.mjs";
import {
  resourceBase as canonicalBase,
  resourceDirectory,
} from "./embed-contract.mjs";
import { loadEngine, releaseEnginePath } from "./engine-loader.mjs";
import {
  GAME_ABI,
  GAME_EXPORTS,
  GAME_SCHEMA,
  createGameController,
} from "./game-controller.mjs";
import { audioActivation } from "./stage-audio.mjs";

export { GAME_ABI, GAME_EXPORTS, GAME_SCHEMA };
const IDENTITY = /^[a-z0-9][a-z0-9._-]{0,95}$/;
const CATALOG = /^[a-f0-9]{64}$/;
const SEED_IDENTITY = [
  "schemaVersion",
  "abi",
  "region",
  "version",
  "snapshotId",
  "releaseId",
  "assets",
  "packs",
  "assetCatalog",
  "resourceBase",
  "resourceOrigin",
];

/**
 * Where this release and its snapshot live, derived from this module's own
 * URL: mounted on the page origin at `resourcePrefix`, or below an explicit
 * HTTPS resource base on another origin.
 */
export function gameLocation({ moduleUrl, pageUrl, releaseId, resourcePrefix, snapshot }) {
  if (!IDENTITY.test(releaseId ?? "")) throw new Error("Invalid game release identity");
  if (typeof resourcePrefix !== "string" || !/^\/(?:[a-z0-9][a-z0-9._-]*\/)*$/.test(resourcePrefix))
    throw new Error("Invalid game resource prefix");
  if (
    !snapshot ||
    !IDENTITY.test(snapshot.id ?? "") ||
    !["cn", "jp"].includes(snapshot.region) ||
    !/^\d+\.\d+\.\d+$/.test(snapshot.version ?? "") ||
    (snapshot.packs && !CATALOG.test(snapshot.assetCatalog ?? ""))
  )
    throw new Error("Invalid game snapshot selection");
  const tail = snapshot.packs ? "asset-store/" : `snapshots/${snapshot.id}/assets/`;
  if (snapshot.assets !== resourcePrefix + tail)
    throw new Error("Game snapshot asset routing mismatch");
  const module = new URL(moduleUrl),
    page = new URL(pageUrl);
  const own = `releases/${releaseId}/game-boot.mjs`;
  if (module.origin === page.origin) {
    if (module.pathname !== resourcePrefix + own || module.search || module.hash)
      throw new Error("Game boot is not the selected immutable release");
    return { assets: resourcePrefix + tail, resourceBase: null, resourceOrigin: null };
  }
  const base = canonicalBase(new URL("../../", module).href);
  if (module.href !== base + own)
    throw new Error("Game boot is not the selected immutable release");
  return { assets: base + tail, resourceBase: base, resourceOrigin: new URL(base).origin };
}

/**
 * `onProgress({ phase, backend, engineBytes, baseCompleted, baseTotal })`
 * reports downloading, initializing and base phases. Resolves once the
 * engine is compiled and the base resources are warm.
 */
export async function bootGame({
  releaseId,
  resourcePrefix,
  snapshot,
  renderer = "auto",
  onProgress = () => {},
  pageUrl = location.href,
  environment = { navigator, document },
  scope = globalThis,
}) {
  const place = gameLocation({
    moduleUrl: import.meta.url,
    pageUrl,
    releaseId,
    resourcePrefix,
    snapshot,
  });
  resourceDirectory(place.assets, pageUrl, place.resourceBase ?? place.resourceOrigin ?? undefined);
  const identity = Object.freeze({
    schemaVersion: GAME_SCHEMA,
    abi: GAME_ABI,
    region: snapshot.region,
    version: snapshot.version,
    snapshotId: snapshot.id,
    releaseId,
    assets: place.assets,
    packs: snapshot.packs === true,
    assetCatalog: snapshot.packs === true ? snapshot.assetCatalog : null,
    resourceBase: place.resourceBase,
    resourceOrigin: place.resourceOrigin,
  });
  // Installed before the engine module loads, so its output context is tracked.
  const resumeAudio = audioActivation(scope);
  const timings = {};
  const progress = { phase: "idle", backend: null, engineBytes: 0, baseCompleted: 0, baseTotal: 0 };
  const report = (phase = progress.phase) => {
    progress.phase = phase;
    onProgress({ ...progress });
  };
  const mark = (name) => {
    if (timings[name] === undefined) {
      timings[name] = performance.now();
      performance.mark(`moly-game:${name}`);
    }
  };
  const absoluteAssets = new URL(place.assets, pageUrl).href;
  let lastBytesReport = 0;
  const { module, backend } = await loadEngine({
    requested: renderer,
    environment,
    mark,
    report,
    async prepare({ signal, progress: touch }) {
      await preflightCoordinates(
        {
          assets: absoluteAssets,
          region: identity.region,
          version: identity.version,
          packs: identity.packs,
          assetCatalog: identity.assetCatalog ?? undefined,
          snapshotId: identity.snapshotId,
          releaseId,
          resourcePrefix,
          pageUrl,
          resourceOrigin: identity.resourceOrigin,
          resourceBase: identity.resourceBase ?? undefined,
        },
        { signal },
      );
      touch();
      const ready = warmBaseResources(
        {
          region: identity.region,
          version: identity.version,
          assets: absoluteAssets,
          resourceBase: identity.resourceBase ?? undefined,
          packs: identity.packs,
          assetCatalog: identity.assetCatalog ?? undefined,
        },
        {
          signal,
          onProgress({ completed, total }) {
            progress.baseCompleted = completed;
            progress.baseTotal = total;
            if (progress.phase === "base") report();
          },
        },
      );
      // The promise stays rejectable where the loader awaits it.
      void ready.catch(() => {});
      return { ready };
    },
    onBackend(selected) {
      progress.backend = selected;
    },
    resolve: (selected) =>
      releaseEnginePath({
        backend: selected,
        moduleUrl: import.meta.url,
        releaseId,
        resourceBase: identity.resourceBase,
        publicOrigin: identity.resourceOrigin,
        prefix: resourcePrefix,
        pageError: "Invalid immutable game path",
      }),
    exports: GAME_EXPORTS,
    contract: "game",
    onBytes(total) {
      progress.engineBytes = total;
      // At most four download reports a second.
      const now = performance.now();
      if (now - lastBytesReport >= 250) {
        lastBytesReport = now;
        report();
      }
    },
    waitTick() {
      if (progress.phase === "base") report();
    },
  });
  report("ready");
  let started = false;
  return {
    backend,
    identity,
    timings,
    resumeAudio,
    /** The ABI seed of this boot; the page supplies only its own facts. */
    seed({ writable, settings }) {
      return {
        ...identity,
        writable: writable === true,
        documents: { settings: typeof settings === "string" ? settings : null },
      };
    },
    /** Synchronous: call it inside the trusted tap, after the page's own
     * fullscreen, wake-lock and audio requests. */
    start(seed, post) {
      if (started) throw new Error("The game was already started");
      if (
        !seed ||
        typeof seed !== "object" ||
        SEED_IDENTITY.some((key) => seed[key] !== identity[key]) ||
        typeof seed.writable !== "boolean" ||
        !seed.documents ||
        (seed.documents.settings !== null && typeof seed.documents.settings !== "string")
      )
        throw new Error("Seed does not match the booted release and snapshot");
      started = true;
      mark("gameStart");
      module.start_game(backend, JSON.stringify(seed));
      return createGameController({ wasm: module, post, writable: seed.writable });
    },
  };
}
