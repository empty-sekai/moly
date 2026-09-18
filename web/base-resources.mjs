import { PackClient, packDigest } from "./asset-pack-client.mjs";

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
      credentials: "same-origin",
      cache: "no-store",
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
  { fetchImpl = fetch, signal, onProgress = () => {} } = {},
) {
  const descriptor = new URL("browser-base.json", options.assets);
  const client =
    options.packs || options.assetCatalog
      ? new PackClient(options.assets, options.assetCatalog ?? null, {
          fetchImpl,
          signal,
          required: true,
        })
      : null;
  const bytes = client
    ? await client.read("browser-base.json")
    : await consume(descriptor, 1048576, signal, fetchImpl, true);
  const pack = JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(bytes),
  );
  const rows = validateBasePack(pack, options);
  let next = 0,
    completed = 0,
    decodedBytes = 0;
  const worker = async () => {
    while (next < rows.length) {
      const row = rows[next++];
      if (client) {
        const bytes = await client.read(row.path);
        if (
          bytes.byteLength !== row.decodedBytes ||
          (await packDigest(bytes)) !== row.sha256
        )
          throw new Error(
            "Packed base resource differs from its measured descriptor",
          );
        decodedBytes += bytes.byteLength;
      } else
        decodedBytes += await consume(
          row.url,
          row.decodedBytes,
          signal,
          fetchImpl,
        );
      completed++;
      onProgress({ completed, total: rows.length, decodedBytes });
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(client ? 2 : 3, rows.length) }, worker),
  );
  // Cache writes are serialized and may complete just after their response.
  // Flush their queue, without making storage denial a playback blocker.
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
        { source: "moly-cache-host", schemaVersion: 1, type: "query" },
        [channel.port2],
      );
    });
  return { completed, decodedBytes };
}
