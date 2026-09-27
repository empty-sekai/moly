// Publishes a worker as one classic script: the retention core followed by a
// worker entry that imports it. Service workers are registered at a fixed URL,
// so the published worker must not depend on a second module file.
import { readFileSync } from "node:fs";

const CORE = "./cache-worker-core.mjs";
const IMPORT = /^import\s*\{[^}]*\}\s*from\s*"\.\/cache-worker-core\.mjs";\s*$/gm;
const REEXPORT = /^export\s*\{[^}]*\}\s*from\s*"\.\/cache-worker-core\.mjs";\s*$/gm;

/** Inline `core` ahead of `entry`. Refuses any other module syntax. */
export function bundleCacheWorker(core, entry) {
  const body = [
    core.replace(/^export (?=(?:async\s+)?(?:const|let|function|class)\b)/gm, ""),
    entry
      .replace(IMPORT, "")
      .replace(REEXPORT, "")
      .replace(/^export (?=(?:async\s+)?(?:const|let|function|class)\b)/gm, ""),
  ].join("\n");
  if (/^\s*(?:import|export)\b/m.test(body) || /\bimport\s*\(|\bimport\.meta\b/.test(body))
    throw new Error("Worker bundle still holds module syntax");
  return body;
}

/** The bundle of an entry file next to the core in `directory` (a file URL). */
export function bundleCacheWorkerFiles(entry, directory = new URL("./", import.meta.url)) {
  return bundleCacheWorker(
    readFileSync(new URL(CORE, directory), "utf8"),
    readFileSync(new URL(entry, directory), "utf8"),
  );
}
