// Durable base/runtime retention and bounded persistent on-demand reuse.
// It never handles site HTML, accounts, saved layouts, or API traffic.
export const CACHE_PROTOCOL = 1;
export const RESOURCE_PREFIX = "moly-resource-v1-";
const RESOURCE_FAMILY_PREFIX = "moly-resource-";
const CONTROL_CACHE = "moly-control-v1";
const LIMIT = 512 * 1024 * 1024;
const MAX_ENTRY = 128 * 1024 * 1024;
const MAX_PENDING = 8;
function publicOrigin(value) {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" && url.hostname && !url.username && !url.password &&
      !url.search && !url.hash && url.pathname === "/" && value === url.origin
      ? url.origin : null;
  } catch { return null; }
}

/** Trust the configured origin of the requesting same-origin stage client,
 * never a global mutable allowlist shared by unrelated pages or tabs. */
export function clientResourceOrigin(clientUrl, origin) {
  try {
    const client = new URL(clientUrl);
    if (client.origin !== origin || client.username || client.password ||
        !/^\/moly\/releases\/[a-z0-9][a-z0-9._-]{0,95}\/stage\.html$/.test(client.pathname)) return null;
    return publicOrigin(client.searchParams.get("resource_origin"));
  } catch { return null; }
}

/** Only currently open stages protect an engine or a source's base resources.
 * The persisted required list describes base membership, not a permanent pin. */
export function activeResourceRoots(clients, origin) {
  const releases = new Set(), snapshots = new Set(), shared = new Set();
  for (const client of clients) {
    try {
      const url = new URL(client.url);
      const release = /^\/moly\/releases\/([a-z0-9][a-z0-9._-]{0,95})\/stage\.html$/.exec(url.pathname);
      if (url.origin !== origin || !release || url.username || url.password) continue;
      const remote = clientResourceOrigin(client.url, origin);
      const releasePath = `/moly/releases/${release[1]}/`;
      releases.add(origin + releasePath);
      if (remote) releases.add(remote + releasePath);
      const value = url.searchParams.get("assets");
      if (!value) continue;
      const assets = new URL(value, url);
      if ((assets.origin !== origin && assets.origin !== remote) || assets.username || assets.password ||
          assets.search || assets.hash || /[%\\]/.test(assets.pathname)) continue;
      if (/^\/moly\/snapshots\/[a-z0-9][a-z0-9._-]{0,95}\/assets\/$/.test(assets.pathname)) snapshots.add(assets.href);
      else if (url.searchParams.get("packs") === "1" && assets.pathname === "/moly/asset-store/") shared.add(assets.href);
    } catch { /* An unrelated or invalid client cannot pin resources. */ }
  }
  return { releases, snapshots, shared };
}

export function isActiveRequiredResource(url, required, active) {
  if ([...active.releases].some(root => url.startsWith(root))) return true;
  if (!required.has(url)) return false;
  // Shared objects do not carry a snapshot in their URL. Conservatively keep
  // required objects of a store only while a packed stage uses that store.
  return [...active.snapshots, ...active.shared].some(root => url.startsWith(root));
}

/** A bounded admission queue, never a reason to silently skip retention.
 * Waiting responses have not been cloned or consumed, so stream backpressure
 * bounds their buffers. Only admitted response bodies enter CacheStorage. */
export class CacheAdmission {
  constructor(maxCount, maxBytes) {
    this.maxCount = maxCount;
    this.maxBytes = maxBytes;
    this.count = 0;
    this.bytes = 0;
    this.waiters = [];
  }
  acquire(bytes, valid = () => true) {
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > this.maxBytes)
      return Promise.resolve(null);
    return new Promise((resolve) => {
      this.waiters.push({ bytes, valid, resolve });
      this.pump();
    });
  }
  pump() {
    while (this.waiters.length) {
      const next = this.waiters[0];
      if (!next.valid()) {
        this.waiters.shift();
        next.resolve(null);
        continue;
      }
      if (
        this.count >= this.maxCount ||
        this.bytes + next.bytes > this.maxBytes
      )
        break;
      this.waiters.shift();
      this.count++;
      this.bytes += next.bytes;
      let released = false;
      next.resolve(() => {
        if (!released) {
          released = true;
          this.count--;
          this.bytes -= next.bytes;
          this.pump();
        }
      });
    }
  }
}

export function resourceIdentity(value, origin, configuredOrigin = null) {
  let url;
  try { url = new URL(value, origin); } catch { return null; }
  if (
    (url.origin !== origin && url.origin !== publicOrigin(configuredOrigin)) ||
    url.search ||
    url.hash ||
    url.username ||
    url.password ||
    !["http:", "https:"].includes(url.protocol) ||
    /%2f|%5c|%00/i.test(url.pathname)
  )
    return null;
  const shared = url.pathname.match(
    /^\/moly\/asset-store\/(?:(blobs)\/([a-f0-9]{2})\/([a-f0-9]{64})\.(bin|gzz|brz|br)|(packages|catalogs)\/([a-f0-9]{64})\.json)$/,
  );
  if (shared) {
    const sha256 = shared[3] || shared[6];
    if (shared[2] && shared[2] !== sha256.slice(0, 2)) return null;
    return {
      cache: RESOURCE_PREFIX + "shared-pack-store",
      url: url.href,
      sha256,
      shared: true,
      maximum: shared[1] ? MAX_ENTRY : 16 * 1024 * 1024,
    };
  }
  const match =
    /^\/moly\/(releases|snapshots)\/([a-z0-9][a-z0-9._-]{0,95})\/(.+)$/.exec(
      url.pathname,
    );
  if (
    !match ||
    match[3]
      .split("/")
      .some((part) => !part || part.startsWith(".") || part.includes("\\"))
  )
    return null;
  return {
    cache: RESOURCE_PREFIX + (match[1] === "releases" ? "r-" : "s-") + match[2],
    url: url.href,
  };
}
/** Only a trusted, source-qualified measured pack can protect assets. */
export function requiredResourceURLs(pack, descriptor, origin, configuredOrigin = null) {
  const identity = resourceIdentity(descriptor, origin, configuredOrigin);
  if (
    !identity ||
    pack?.schemaVersion !== 1 ||
    pack.generator !== "moly-browser-base-v1" ||
    !["cn", "jp", "tw", "en", "kr"].includes(pack.region) ||
    !/^\d+\.\d+\.\d+$/.test(pack.gameVersion) ||
    !Array.isArray(pack.files) ||
    pack.files.length > 5000
  )
    return [];
  const url = new URL(descriptor);
  if (
    !url.pathname.startsWith(
      `/moly/snapshots/${pack.region}-${pack.gameVersion}-`,
    ) ||
    !url.pathname.endsWith("/assets/browser-base.json")
  )
    return [];
  const base = new URL("./", url);
  const urls = [url.href];
  for (const file of pack.files) {
    if (
      typeof file.path !== "string" ||
      /[\\:%?#\x00-\x1f]/.test(file.path) ||
      file.path.split("/").some((part) => !part || part.startsWith("."))
    )
      return [];
    const child = new URL(file.path, base);
    if (
      !resourceIdentity(child.href, origin, configuredOrigin) ||
      !child.pathname.startsWith(base.pathname)
    )
      return [];
    urls.push(child.href);
  }
  return [...new Set(urls)];
}

export function isCacheMessage(value) {
  return (
    value &&
    typeof value === "object" &&
    value.source === "moly-cache-host" &&
    value.schemaVersion === CACHE_PROTOCOL &&
    ["query", "retain", "clear"].includes(value.type) &&
    (value.type !== "retain" || typeof value.enabled === "boolean")
  );
}

/** A size header is only an admission hint. Bound the stream before hashing so
 * a malformed immutable response cannot allocate an unbounded arrayBuffer. */
export async function verifiedSharedBody(response, bytes, sha256) {
  if (
    !response.body ||
    !Number.isSafeInteger(bytes) ||
    bytes < 0 ||
    bytes > MAX_ENTRY
  )
    throw new Error("Invalid shared response size");
  const body = new Uint8Array(bytes),
    reader = response.body.getReader();
  let used = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (used + value.byteLength > bytes)
        throw new Error("Shared response exceeded its declared size");
      body.set(value, used);
      used += value.byteLength;
    }
    if (used !== bytes) throw new Error("Shared response length mismatch");
    const actual = [
      ...new Uint8Array(await crypto.subtle.digest("SHA-256", body)),
    ]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
    if (actual !== sha256) throw new Error("Shared response checksum mismatch");
    return body;
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
}

if (
  typeof ServiceWorkerGlobalScope !== "undefined" &&
  self instanceof ServiceWorkerGlobalScope
) {
  let enabled = false,
    epoch = 0,
    queue = Promise.resolve();
  const admission = new CacheAdmission(MAX_PENDING, MAX_ENTRY);
  const controlUrl = new URL("/moly/__retention", self.location.origin).href;
  const pinsUrl = new URL("/moly/__required", self.location.origin).href;
  const required = new Set();
  const lastUsed = new Map();
  const initialized = (async () => {
    const cache = await caches.open(CONTROL_CACHE),
      response = await cache.match(controlUrl);
    if (response) enabled = (await response.json()).enabled === true;
    const pins = await cache.match(pinsUrl);
    if (pins) {
      const values = await pins.json();
      if (Array.isArray(values) && values.length <= 20000)
        for (const url of values)
          // Restoring cache bookkeeping does not authorize any network reads.
          // Each future request must match its own stage client's origin below.
          if (typeof url === "string" && resourceIdentity(url, self.location.origin, new URL(url).origin)) required.add(url);
    }
  })().catch(() => {
    enabled = false;
  });
  const serial = (action) => {
    const result = queue.then(action);
    queue = result.catch(() => {});
    return result;
  };
  async function inventory() {
    const entries = [];
    for (const name of (await caches.keys()).filter((name) =>
      name.startsWith(RESOURCE_FAMILY_PREFIX),
    )) {
      const cache = await caches.open(name);
      for (const request of await cache.keys()) {
        const response = await cache.match(request);
        entries.push({
          name,
          request,
          bytes: Number(response?.headers.get("X-Moly-Stored-Bytes") || 0),
          time: lastUsed.get(request.url) ?? Number(response?.headers.get("X-Moly-Stored-At") || 0),
        });
      }
    }
    return entries;
  }
  async function makeSpace(extra, incoming) {
    const active = activeResourceRoots(await self.clients.matchAll({ type: "window" }), self.location.origin);
    const entries = (await inventory()).filter(
      (entry) => entry.request.url !== incoming,
    );
    let bytes = entries.reduce((sum, entry) => sum + entry.bytes, 0);
    for (const entry of entries.sort((a, b) => a.time - b.time)) {
      if (bytes + extra <= LIMIT) break;
      if (isActiveRequiredResource(entry.request.url, required, active)) continue;
      await (await caches.open(entry.name)).delete(entry.request);
      lastUsed.delete(entry.request.url);
      bytes -= entry.bytes;
    }
    // Never evict necessary base resources or engines for optional content.
    // Device quota failures remain best-effort, not a false offline promise.
    if (bytes + extra > LIMIT)
      throw new Error("Required resources fill the cache budget");
  }
  self.addEventListener("install", (event) =>
    event.waitUntil(self.skipWaiting()),
  );
  self.addEventListener("activate", (event) => event.waitUntil((async () => {
    await self.clients.claim();
    await initialized;
    // Keep previously downloaded optional resources across worker restarts.
    // Activation only enforces the same total disk budget as new writes.
    await serial(() => makeSpace(0, null)).catch(() => {});
  })()));
  self.addEventListener("message", (event) => {
    const port = event.ports?.[0],
      message = event.data;
    if (
      !port ||
      !isCacheMessage(message) ||
      !event.source?.url ||
      new URL(event.source.url).origin !== self.location.origin
    )
      return;
    event.waitUntil(
      serial(async () => {
        await initialized;
        if (message.type === "retain") {
          await (
            await caches.open(CONTROL_CACHE)
          ).put(
            controlUrl,
            new Response(JSON.stringify({ enabled: message.enabled }), {
              headers: { "Content-Type": "application/json" },
            }),
          );
          enabled = message.enabled;
          epoch++;
        } else if (message.type === "clear") {
          epoch++;
          required.clear();
          lastUsed.clear();
          await (await caches.open(CONTROL_CACHE)).delete(pinsUrl);
          await Promise.all(
            (await caches.keys())
              .filter((name) => name.startsWith(RESOURCE_FAMILY_PREFIX))
              .map((name) => caches.delete(name)),
          );
        }
        const entries = await inventory();
        port.postMessage({
          source: "moly-cache",
          schemaVersion: CACHE_PROTOCOL,
          ok: true,
          enabled,
          bytes: entries.reduce((sum, item) => sum + item.bytes, 0),
          entries: entries.length,
          limitBytes: LIMIT,
        });
      }).catch(() =>
        port.postMessage({
          source: "moly-cache",
          schemaVersion: CACHE_PROTOCOL,
          ok: false,
        }),
      ),
    );
  });
  self.addEventListener("fetch", (event) => {
    const request = event.request;
    // Catalogue entry JSON belongs to the reading page, never to the resource store.
    if (/^\/moly\/snapshots\/[^/]+\/catalog\/entries\/[^/]+\.json$/.test(new URL(request.url).pathname)) return;
    // Prefilter immutable paths synchronously; cross-origin admission also
    // requires the requesting stage client's current configuration.
    const candidate =
      request.method === "GET" && !request.headers.has("Range")
        ? resourceIdentity(request.url, self.location.origin, new URL(request.url).origin)
        : null;
    if (!candidate) return;
    event.respondWith(
      (async () => {
        const client = event.clientId ? await self.clients.get(event.clientId).catch(() => null) : null;
        const configuredOrigin = clientResourceOrigin(client?.url, self.location.origin);
        const identity = resourceIdentity(request.url, self.location.origin, configuredOrigin);
        if (!identity) return fetch(request);
        await initialized;
        const requestEpoch = epoch;
        if (enabled) {
          // Private browsing, eviction and device quota failures must never
          // turn successful online playback into an unavailable response.
          const retained = await caches
            .match(identity.url, { cacheName: identity.cache })
            .catch(() => null);
          if (retained) {
            lastUsed.set(identity.url, Date.now());
            if (
              identity.shared &&
              request.headers.get("X-Moly-Required") === "1"
            ) {
              event.waitUntil(
                serial(async () => {
                  if (!enabled || requestEpoch !== epoch || required.size >= 20000) return;
                  required.add(identity.url);
                  await (
                    await caches.open(CONTROL_CACHE)
                  ).put(
                    pinsUrl,
                    new Response(JSON.stringify([...required]), {
                      headers: { "Content-Type": "application/json" },
                    }),
                  );
                }).catch(() => {}),
              );
            }
            const headers = new Headers(retained.headers);
            headers.set("X-Moly-Cache", "retained");
            return new Response(retained.body, {
              status: retained.status,
              statusText: retained.statusText,
              headers,
            });
          }
        }
        // Controlled large runtime requests bypass the browser's opaque HTTP
        // cache. Optional retained copies are therefore measurable and removable.
        const response = await fetch(request, { cache: "no-store", credentials: "omit", redirect: "error" });
        const bytes = Number(response.headers.get("X-Moly-Decoded-Bytes") ||
          response.headers.get("x-oss-meta-moly-decoded-bytes") || 0);
        let pinsChanged = false;
        if (
          enabled &&
          requestEpoch === epoch &&
          response.ok &&
          new URL(identity.url).pathname.endsWith(
            "/assets/browser-base.json",
          ) &&
          bytes > 0 &&
          bytes <= 1048576
        ) {
          try {
            const pack = await response.clone().json();
            const urls = requiredResourceURLs(
              pack,
              identity.url,
              self.location.origin,
              configuredOrigin,
            );
            if (required.size + urls.length <= 20000)
              for (const url of urls) {
                if (!required.has(url)) {
                  required.add(url);
                  pinsChanged = true;
                }
              }
          } catch {
            /* An invalid descriptor is rejected by the stage loader. */
          }
        }
        // All immutable public resources use the bounded disk store. There is
        // no completed response-body memory cache. Development no-cache mounts
        // remain network-only, and failures never prevent online playback.
        if (
          enabled &&
          requestEpoch === epoch &&
          response.status === 200 &&
          response.headers.get("Cache-Control")?.includes("immutable") &&
          Number.isSafeInteger(bytes) &&
          bytes > 0 &&
          bytes <= (identity.maximum ?? MAX_ENTRY)
        ) {
          const generation = epoch;
          const release = await admission.acquire(
            bytes,
            () => enabled && generation === epoch && !request.signal?.aborted,
          );
          if (!release) return response;
          let copy = response.clone();
          event.waitUntil(
            serial(async () => {
              if (!enabled || generation !== epoch) {
                await copy.body?.cancel();
                return;
              }
              if (identity.sha256) {
                const data = await verifiedSharedBody(
                  copy,
                  bytes,
                  identity.sha256,
                );
                copy = new Response(data, {
                  status: response.status,
                  statusText: response.statusText,
                  headers: response.headers,
                });
              }
              if (
                identity.shared &&
                request.headers.get("X-Moly-Required") === "1" &&
                required.size < 20000
              ) {
                required.add(identity.url);
                pinsChanged = true;
              }
              if (pinsChanged)
                await (
                  await caches.open(CONTROL_CACHE)
                ).put(
                  pinsUrl,
                  new Response(JSON.stringify([...required]), {
                    headers: { "Content-Type": "application/json" },
                  }),
                );
              await makeSpace(bytes, identity.url);
              const headers = new Headers(copy.headers);
              headers.delete("Content-Encoding");
              headers.delete("Content-Length");
              headers.set("X-Moly-Stored-Bytes", String(bytes));
              headers.set("X-Moly-Stored-At", String(Date.now()));
              const cache = await caches.open(identity.cache);
              await cache.put(
                identity.url,
                new Response(copy.body, { status: 200, headers }),
              );
              lastUsed.set(identity.url, Date.now());
            })
              .catch(() => {
                /* Retention is best-effort; never fail successful online playback. */
              })
              .finally(() => {
                release();
              }),
          );
        }
        return response;
      })(),
    );
  });
}
