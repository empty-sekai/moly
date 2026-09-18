// Optional, bounded resource retention. This worker only controls /moly/.
// It never handles site HTML, accounts, saved layouts, or API traffic.
export const CACHE_PROTOCOL = 1;
export const RESOURCE_PREFIX = "moly-resource-v1-";
const CONTROL_CACHE = "moly-control-v1";
const LIMIT = 512 * 1024 * 1024;
const MAX_ENTRY = 128 * 1024 * 1024;
const MAX_PENDING = 8;

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

export function resourceIdentity(value, origin) {
  const url = new URL(value, origin);
  if (
    url.origin !== origin ||
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
/** Only a same-origin, source-qualified measured pack can protect assets. */
export function requiredResourceURLs(pack, descriptor, origin) {
  const identity = resourceIdentity(descriptor, origin);
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
      !resourceIdentity(child.href, origin) ||
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
  const isRequired = (url) =>
    new URL(url).pathname.startsWith("/moly/releases/") || required.has(url);
  const initialized = (async () => {
    const cache = await caches.open(CONTROL_CACHE),
      response = await cache.match(controlUrl);
    if (response) enabled = (await response.json()).enabled === true;
    const pins = await cache.match(pinsUrl);
    if (pins) {
      const values = await pins.json();
      if (Array.isArray(values) && values.length <= 20000)
        for (const url of values)
          if (resourceIdentity(url, self.location.origin)) required.add(url);
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
      name.startsWith(RESOURCE_PREFIX),
    )) {
      const cache = await caches.open(name);
      for (const request of await cache.keys()) {
        const response = await cache.match(request);
        entries.push({
          name,
          request,
          bytes: Number(response?.headers.get("X-Moly-Stored-Bytes") || 0),
          time: Number(response?.headers.get("X-Moly-Stored-At") || 0),
        });
      }
    }
    return entries;
  }
  async function makeSpace(extra, incoming) {
    const entries = (await inventory()).filter(
      (entry) => entry.request.url !== incoming,
    );
    let bytes = entries.reduce((sum, entry) => sum + entry.bytes, 0);
    for (const entry of entries.sort((a, b) => a.time - b.time)) {
      if (bytes + extra <= LIMIT) break;
      if (isRequired(entry.request.url)) continue;
      await (await caches.open(entry.name)).delete(entry.request);
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
  self.addEventListener("activate", (event) =>
    event.waitUntil(self.clients.claim()),
  );
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
          await (await caches.open(CONTROL_CACHE)).delete(pinsUrl);
          await Promise.all(
            (await caches.keys())
              .filter((name) => name.startsWith(RESOURCE_PREFIX))
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
    const identity =
      request.method === "GET" && !request.headers.has("Range")
        ? resourceIdentity(request.url, self.location.origin)
        : null;
    if (!identity) return;
    event.respondWith(
      (async () => {
        await initialized;
        if (enabled) {
          // Private browsing, eviction and device quota failures must never
          // turn successful online playback into an unavailable response.
          const retained = await caches
            .match(identity.url, { cacheName: identity.cache })
            .catch(() => null);
          if (retained) {
            if (
              identity.shared &&
              request.headers.get("X-Moly-Required") === "1"
            ) {
              event.waitUntil(
                serial(async () => {
                  if (!enabled || required.size >= 20000) return;
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
        const requestEpoch = epoch;
        const response = await fetch(request, { cache: "no-store" });
        const bytes = Number(response.headers.get("X-Moly-Decoded-Bytes") || 0);
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
        // Development asset mounts are no-cache and deliberately never retained.
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
