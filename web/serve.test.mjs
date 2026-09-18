import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import {
  mkdtemp,
  realpath,
  rm,
  writeFile,
  readFile,
  mkdir,
} from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

function readResponse(url, method = "GET") {
  return new Promise((resolve, reject) => {
    const client = request(url, { method, agent: false }, (response) => {
      const hash = createHash("sha256");
      let bytes = 0;
      response.on("data", (chunk) => {
        hash.update(chunk);
        bytes += chunk.length;
      });
      response.on("error", reject);
      response.on("end", () =>
        resolve({
          status: response.statusCode,
          headers: response.headers,
          bytes,
          hash: hash.digest("hex"),
        }),
      );
    });
    client.setTimeout(15_000, () =>
      client.destroy(new Error("HTTP test request timed out")),
    );
    client.on("error", reject);
    client.end();
  });
}

function cancelResponse(url) {
  return new Promise((resolve, reject) => {
    let cancelled = false;
    const client = request(url, { agent: false }, (response) => {
      response.once("data", (chunk) => {
        cancelled = true;
        const result = {
          status: response.statusCode,
          received: chunk.length,
          total: Number(response.headers["content-length"]),
        };
        response.destroy();
        client.destroy();
        resolve(result);
      });
      response.on("error", (error) => {
        if (!cancelled) reject(error);
      });
      response.on("end", () => {
        if (!cancelled) reject(new Error("Response ended before cancellation"));
      });
    });
    client.setTimeout(15_000, () =>
      client.destroy(new Error("Cancellation request timed out")),
    );
    client.on("error", (error) => {
      if (!cancelled) reject(error);
    });
    client.end();
  });
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  const killTimer = setTimeout(() => child.kill("SIGKILL"), 5_000);
  killTimer.unref();
  child.kill("SIGTERM");
  try {
    await exited;
  } finally {
    clearTimeout(killTimer);
  }
}

test(
  "development server streams assets and survives cancelled transfers",
  { timeout: 45_000 },
  async (t) => {
    const temporaryRoot = await realpath(tmpdir());
    const assetRoot = await mkdtemp(
      path.join(temporaryRoot, "moly-serve-test-"),
    );
    const canonicalRoot = await realpath(assetRoot);
    let child;
    let output = "";
    t.after(async () => {
      await stopChild(child);
      // Delete only the exact temporary fixture created by this test, after its
      // process exits. Never derive a deletion target from the server response.
      const resolved = await realpath(assetRoot);
      assert.equal(resolved, canonicalRoot);
      assert.equal(path.dirname(resolved), temporaryRoot);
      assert.match(path.basename(resolved), /^moly-serve-test-/);
      await rm(resolved, {
        recursive: true,
        force: true,
        maxRetries: 3,
        retryDelay: 100,
      });
    });

    const payload = Buffer.alloc(10 * 1024 * 1024);
    for (let i = 0; i < payload.length; i++) payload[i] = i % 251;
    const expectedHash = createHash("sha256").update(payload).digest("hex");
    const golden = JSON.parse(
      await readFile(
        new URL("./fixtures/asset-pack-v2.json", import.meta.url),
        "utf8",
      ),
    );
    const packRoot = path.join(assetRoot, "pack-store");
    for (const [relative, value] of Object.entries(golden.files)) {
      if (!relative.startsWith("store/")) continue;
      const filename = path.join(packRoot, relative.slice("store/".length));
      await mkdir(path.dirname(filename), { recursive: true });
      await writeFile(filename, Buffer.from(value, "base64"));
    }
    await writeFile(path.join(assetRoot, "sample.bin"), payload);
    await writeFile(
      path.join(assetRoot, "sample.css"),
      ":root { color: #33ccbb; }\n",
    );
    await writeFile(
      path.join(assetRoot, "sample.svg"),
      '<svg xmlns="http://www.w3.org/2000/svg"/>',
    );
    await writeFile(
      path.join(assetRoot, "sample.woff2"),
      Buffer.from([0x77, 0x4f, 0x46, 0x32]),
    );

    child = spawn(process.execPath, [path.join(here, "serve.mjs"), "0"], {
      cwd: here,
      env: {
        ...process.env,
        MOLY_ASSET_ROOT: assetRoot,
        MOLY_ASSET_PACK_ROOT: packRoot,
      },
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const base = await new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`Server did not start: ${output}`)),
        15_000,
      );
      const fail = (error) => {
        clearTimeout(timer);
        reject(error);
      };
      child.once("error", fail);
      child.once("exit", (code) =>
        fail(new Error(`Server exited before listening (${code}): ${output}`)),
      );
      child.stderr.on("data", (chunk) => {
        output = (output + chunk.toString()).slice(-16_384);
      });
      child.stdout.on("data", (chunk) => {
        output = (output + chunk.toString()).slice(-16_384);
        const match = output.match(/serving on (http:\/\/127\.0\.0\.1:\d+\/)/);
        if (match) {
          clearTimeout(timer);
          resolve(match[1]);
        }
      });
    });
    assert.notEqual(new URL(base).port, "8017");

    await t.test(
      "shared store serves exact immutable bytes and keeps selectors mutable",
      async () => {
        const catalogPath = `catalogs/${golden.index.catalogs.cn.id}.json`;
        const catalog = JSON.parse(
          Buffer.from(golden.files[`store/${catalogPath}`], "base64"),
        );
        const manifestPath = catalog.packages[0].manifest;
        const manifest = JSON.parse(
          Buffer.from(golden.files[`store/${manifestPath}`], "base64"),
        );
        for (const logical of [
          catalogPath,
          manifestPath,
          "blobs/" + manifest.entries[0].blob,
        ]) {
          const result = await readResponse(
            `${base}moly/asset-store/${logical}`,
          );
          assert.equal(result.status, 200);
          assert.equal(
            result.hash,
            createHash("sha256")
              .update(Buffer.from(golden.files[`store/${logical}`], "base64"))
              .digest("hex"),
          );
          assert.match(
            result.headers["cache-control"],
            /immutable, no-transform/,
          );
          assert.equal(
            Number(result.headers["x-moly-decoded-bytes"]),
            result.bytes,
          );
        }
        for (const logical of [
          "asset-packs.json",
          "channels/cn/stable.json",
          "channels/jp/stable.json",
        ]) {
          const result = await readResponse(
            `${base}moly/asset-store/${logical}`,
          );
          assert.equal(result.status, 200);
          assert.equal(result.headers["cache-control"], "no-store");
        }
        const discovery = await (await fetch(`${base}snapshots.json`)).json();
        for (const region of ["cn", "jp", "tw"])
          assert.equal(
            discovery.snapshots.find(
              (row) => row.region === region && row.packs,
            )?.assetCatalog,
            golden.index.catalogs[region].id,
          );
      },
    );

    await t.test(
      "HEAD advertises the exact length without streaming a response body",
      async () => {
        const result = await readResponse(`${base}assets/sample.bin`, "HEAD");
        assert.equal(result.status, 200);
        assert.equal(Number(result.headers["content-length"]), payload.length);
        assert.equal(
          result.headers["content-type"],
          "application/octet-stream",
        );
        assert.equal(result.bytes, 0);
      },
    );

    await t.test(
      "the complete streamed response has the source length and SHA-256",
      async () => {
        const result = await readResponse(`${base}assets/sample.bin`);
        assert.equal(result.status, 200);
        assert.equal(result.bytes, payload.length);
        assert.equal(result.hash, expectedHash);
      },
    );

    await t.test(
      "browser styles, SVG and WOFF2 have their actual content types",
      async () => {
        for (const [file, type] of [
          ["sample.css", "text/css; charset=utf-8"],
          ["sample.svg", "image/svg+xml"],
          ["sample.woff2", "font/woff2"],
        ]) {
          const result = await readResponse(`${base}assets/${file}`);
          assert.equal(result.status, 200, file);
          assert.equal(result.headers["content-type"], type, file);
          assert.ok(result.bytes > 0, file);
        }
        const shell = await readResponse(`${base}shell.css`, "HEAD");
        assert.equal(shell.status, 200);
        assert.equal(shell.headers["content-type"], "text/css; charset=utf-8");
      },
    );

    await t.test(
      "missing files produce 404 for GET and a bodyless HEAD",
      async () => {
        const missing = await readResponse(`${base}assets/does-not-exist.bin`);
        assert.equal(missing.status, 404);
        const head = await readResponse(
          `${base}assets/does-not-exist.bin`,
          "HEAD",
        );
        assert.equal(head.status, 404);
        assert.equal(head.bytes, 0);
      },
    );

    await t.test(
      "cancelled large downloads release their stream without breaking later requests",
      async () => {
        for (let i = 0; i < 3; i++) {
          const aborted = await cancelResponse(`${base}assets/sample.bin`);
          assert.equal(aborted.status, 200);
          assert.ok(aborted.received > 0 && aborted.received < aborted.total);
          const subsequent = await readResponse(`${base}assets/sample.css`);
          assert.equal(subsequent.status, 200);
        }
        const complete = await readResponse(`${base}assets/sample.bin`);
        assert.equal(complete.hash, expectedHash);
        assert.equal(child.exitCode, null, output);
        assert.doesNotMatch(
          output,
          /EBADF|ERR_UNHANDLED_REJECTION|uncaughtException/,
        );
      },
    );
  },
);
