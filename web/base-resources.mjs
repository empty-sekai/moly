import { PackClient } from "./asset-pack-client.mjs";

// Most base files are small, so their transfer time is mostly round trips:
// enough parallel streams keep the connection busy.
export const BASE_RESOURCE_CONCURRENCY = 8;

// The first entry downloads the measured necessary pack without waiting for
// audio permission. Playback still begins from the trusted in-frame gesture.
export function validateBasePack(pack, { region, version, assets }) {
  const base = new URL(assets);
  if (
    pack?.schemaVersion !== 1 ||
    pack.generator !== "moly-browser-base-v1" ||
    pack.region !== region ||
    pack.gameVersion !== version ||
    !Array.isArray(pack.files) ||
    pack.files.length > 5000 ||
    !Number.isSafeInteger(pack.decodedBytes) ||
    pack.decodedBytes > 512 * 1048576
  )
    throw new Error("Required resource pack has a different source identity");
  const seen = new Set();
  let bytes = 0;
  const rows = pack.files.map((row) => {
    if (
      typeof row.path !== "string" ||
      /[\\:%?#\x00-\x1f]/.test(row.path) ||
      row.path.split("/").some((part) => !part || part.startsWith(".")) ||
      seen.has(row.path) ||
      !Number.isSafeInteger(row.decodedBytes) ||
      row.decodedBytes < 0 ||
      row.decodedBytes > 128 * 1048576 ||
      !/^[a-f0-9]{64}$/.test(row.sha256)
    )
      throw new Error("Invalid required resource entry");
    seen.add(row.path);
    bytes += row.decodedBytes;
    const url = new URL(row.path, base);
    if (url.origin !== base.origin || !url.pathname.startsWith(base.pathname))
      throw new Error("Required resource escaped its snapshot");
    return { ...row, url: url.href };
  });
  if (bytes !== pack.decodedBytes)
    throw new Error("Required resource totals changed");
  return rows;
}
async function consume(url, maximum, signal, fetchImpl, keep = false) {
  const abort = new AbortController();
  let progress = performance.now();
  const timeout = setInterval(() => {
    if (performance.now() - progress > 30000)
      abort.abort(new Error("Required resource download made no progress"));
  }, 1000);
  let reader;
  try {
    const response = await fetchImpl(url, {
      signal: signal ? AbortSignal.any([signal, abort.signal]) : abort.signal,
      credentials: "omit",
      redirect: "error",
    });
    if (!response.ok || !response.body)
      throw new Error("Required resource unavailable");
    reader = response.body.getReader();
    const chunks = [];
    let bytes = 0;
    while (true) {
      const row = await reader.read();
      progress = performance.now();
      if (row.done) break;
      bytes += row.value.byteLength;
      if (bytes > maximum)
        throw new Error("Required resource exceeds its declared bytes");
      if (keep) chunks.push(row.value);
    }
    if (!keep && bytes !== maximum)
      throw new Error("Required resource length mismatch");
    if (!keep) return bytes;
    const data = new Uint8Array(bytes);
    let offset = 0;
    for (const chunk of chunks) {
      data.set(chunk, offset);
      offset += chunk.length;
    }
    return data;
  } catch (error) {
    await reader?.cancel().catch(() => {});
    throw error;
  } finally {
    clearInterval(timeout);
    reader?.releaseLock();
  }
}
export async function warmBaseResources(
  options,
  {
    fetchImpl = fetch,
    signal,
    onProgress = () => {},
    concurrency = BASE_RESOURCE_CONCURRENCY,
    announce,
  } = {},
) {
  if (
    !Number.isInteger(concurrency) ||
    concurrency < 1 ||
    concurrency > BASE_RESOURCE_CONCURRENCY
  )
    throw new RangeError(
      `Required resource concurrency must be an integer from 1 to ${BASE_RESOURCE_CONCURRENCY}`,
    );
  const descriptor = new URL("browser-base.json", options.assets);
  const client =
    options.packs || options.assetCatalog
      ? new PackClient(options.assets, options.assetCatalog ?? null, {
          fetchImpl,
          signal,
          required: true,
          resourceBase: options.resourceBase,
          ...(announce ? { announce } : {}),
        })
      : null;
  const bytes = client
    ? await client.read("browser-base.json")
    : await consume(descriptor, 1048576, signal, fetchImpl, true);
  const pack = JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(bytes),
  );
  const rows = validateBasePack(pack, options);
  // Largest first: the few large files carry most of the bytes, and one that
  // starts last becomes the tail of the whole transfer.
  const queue = rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => b.row.decodedBytes - a.row.decodedBytes || a.index - b.index)
    .map(({ row }) => row);
  if (client) {
    // Request every package manifest the base needs at once instead of one
    // by one as the transfers reach them. Failures surface where a transfer
    // resolves its path.
    const { owners } = await client.selection();
    for (const owner of new Set(rows.map((row) => owners.get(row.path))))
      if (owner) void client.package(owner).catch(() => {});
  }
  let next = 0,
    completed = 0,
    decodedBytes = 0;
  const finish = (bytes) => {
    decodedBytes += bytes;
    completed++;
    onProgress({ completed, total: rows.length, decodedBytes });
  };
  // Rows whose packed blob is already being transferred wait for that
  // transfer instead of fetching the same immutable object again.
  const transfers = new Map(),
    shared = [];
  const worker = async () => {
    while (next < queue.length) {
      const row = queue[next++];
      if (client) {
        // Like the loose prewarm, this only transfers the bytes. Every
        // reader that uses them checks them against this entry, so the
        // entry must be the measured row.
        const entry = await client.resolve(row.path);
        if (
          entry.bytes !== row.decodedBytes ||
          entry.content_sha256 !== row.sha256
        )
          throw new Error(
            "Packed base resource differs from its measured descriptor",
          );
        const known = transfers.get(entry.url);
        if (known) {
          const done = known.then(() => finish(entry.bytes));
          // Rejections are reported once, by the transfer's own worker.
          done.catch(() => {});
          shared.push(done);
          continue;
        }
        const transfer = client.warmEntry(entry);
        transfers.set(entry.url, transfer);
        await transfer;
        finish(entry.bytes);
      } else {
        const received = await consume(
          row.url,
          row.decodedBytes,
          signal,
          fetchImpl,
        );
        finish(received);
      }
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(concurrency, queue.length) }, worker),
  );
  await Promise.all(shared);
  // A response can finish before its retained copy passes hashing and
  // storage admission. Flush those tasks too, with a bounded best-effort wait
  // so unavailable storage or an older worker cannot block online playback.
  const serviceWorker = globalThis.navigator?.serviceWorker?.controller;
  if (serviceWorker && typeof MessageChannel !== "undefined")
    await new Promise((resolve) => {
      const channel = new MessageChannel();
      const finish = () => {
        clearTimeout(timeout);
        channel.port1.close();
        resolve();
      };
      const timeout = setTimeout(finish, 10000);
      channel.port1.onmessage = finish;
      serviceWorker.postMessage(
        { source: "moly-cache-host", schemaVersion: 1, type: "flush" },
        [channel.port2],
      );
    });
  return { completed, decodedBytes };
}
