// The stage's retention worker, served at `/moly/cache-worker.mjs` with scope
// `/moly/`. The retention rules live in cache-worker-core.mjs; this file only
// names where stages and their publication live.
import { createCacheCore } from "./cache-worker-core.mjs";
export {
  CACHE_PROTOCOL,
  RESOURCE_PREFIX,
  CacheAdmission,
  declaredDecodedBytes,
  isActiveRequiredResource,
  isCacheMessage,
  verifiedSharedBody,
} from "./cache-worker-core.mjs";

export const STAGE_CACHE_CONFIG = Object.freeze({
  root: "/moly/",
  client: /^\/moly\/releases\/([a-z0-9][a-z0-9._-]{0,95})\/stage\.html$/,
});
const stageCache = createCacheCore(STAGE_CACHE_CONFIG);
export const {
  clientResourceOrigin,
  clientResourceBase,
  activeResourceRoots,
  resourceIdentity,
  requiredResourceURLs,
} = stageCache;

if (
  typeof ServiceWorkerGlobalScope !== "undefined" &&
  self instanceof ServiceWorkerGlobalScope
)
  stageCache.install(self);
