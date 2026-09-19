import { PackClient } from "./asset-pack-client.mjs";
import { sourceWeatherIcon } from "./weather-presentation.mjs";

const MAX_PNG_BYTES = 4 * 1048576;
function sourceReceipt(file, receipt) {
  if (
    !receipt ||
    receipt.file !== file ||
    !/^icons\/[^/%\\:?#\x00-\x1f]+\.png$/.test(file) ||
    !/^[a-f0-9]{64}$/.test(receipt.sha256) ||
    !Number.isSafeInteger(receipt.bytes) ||
    receipt.bytes < 33 ||
    receipt.bytes > MAX_PNG_BYTES ||
    ![receipt.width, receipt.height].every(
      (n) => Number.isInteger(n) && n > 0 && n <= 8192,
    )
  ) {
    throw new Error("Missing or invalid source weather thumbnail receipt");
  }
  return receipt;
}

export async function verifyWeatherPng(bytes, receipt) {
  sourceReceipt(receipt?.file, receipt);
  if (!(bytes instanceof Uint8Array) || bytes.byteLength !== receipt.bytes) {
    throw new Error("Source weather thumbnail byte count mismatch");
  }
  const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (
    ![137, 80, 78, 71, 13, 10, 26, 10].every((v, i) => bytes[i] === v) ||
    header.getUint32(8) !== 13 ||
    header.getUint32(12) !== 0x49484452 ||
    header.getUint32(16) !== receipt.width ||
    header.getUint32(20) !== receipt.height
  ) {
    throw new Error(
      "Source weather thumbnail PNG header or dimensions mismatch",
    );
  }
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const actual = Array.from(hash, (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  if (actual !== receipt.sha256)
    throw new Error("Source weather thumbnail hash mismatch");
}

async function readPng(response, expectedBytes) {
  if (!response.ok)
    throw new Error(
      `Source weather thumbnail unavailable (HTTP ${response.status})`,
    );
  if (!response.body)
    throw new Error("Source weather thumbnail has no response body");
  const reader = response.body.getReader();
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > expectedBytes)
        throw new Error(
          "Source weather thumbnail response exceeds its receipt",
        );
      chunks.push(value);
    }
  } catch (error) {
    await reader.cancel(error).catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

/** Owns verified thumbnail URLs. Both flat and packed resources must match the
 * producer's typed artifact receipt before any pixels are exposed to the UI. */
export function createWeatherArtwork(
  { assets, baseUrl, packs = false, assetCatalog = null },
  {
    createClient = (root, catalog, options) =>
      new PackClient(root, catalog, options),
    fetchImpl = (...args) => fetch(...args),
    createObjectURL = (blob) => URL.createObjectURL(blob),
    revokeObjectURL = (url) => URL.revokeObjectURL(url),
  } = {},
) {
  const abort = new AbortController();
  const packed = Boolean(packs || assetCatalog);
  const root = new URL(assets, baseUrl).href;
  const client = packed
    ? createClient(root, assetCatalog, { required: true, signal: abort.signal })
    : null;
  const pending = new Map();
  const urls = new Set();
  let disposed = false;
  return {
    resolve(file, receipt) {
      const direct = sourceWeatherIcon(file, assets, baseUrl);
      if (!direct || disposed) return Promise.resolve(null);
      try {
        sourceReceipt(file, receipt);
      } catch (error) {
        return Promise.reject(error);
      }
      // A new source revision at the same logical path is not the old image.
      const key = `${file}\0${receipt.sha256}`;
      if (!pending.has(key))
        pending.set(
          key,
          (async () => {
            const bytes = client
              ? await client.read(`phenomena/${file}`)
              : await readPng(
                  await fetchImpl(direct, {
                    signal: abort.signal,
                    credentials: "same-origin",
                  }),
                  receipt.bytes,
                );
            if (disposed) return null;
            await verifyWeatherPng(bytes, receipt);
            if (disposed) return null;
            const url = createObjectURL(
              new Blob([bytes], { type: "image/png" }),
            );
            urls.add(url);
            return url;
          })(),
        );
      return pending.get(key);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      abort.abort();
      for (const url of urls) revokeObjectURL(url);
      urls.clear();
      pending.clear();
    },
  };
}
