// 最小静态服务器（开发用）：/ → web/（页面 + pkg/），/assets/ → 资产根。
// 资产根只认 MOLY_ASSET_ROOT——没有它就拒绝启动：半通的页面（wasm 起来、
// 资产逐个 404）比不启动更难查，而这台机器的资产根路径不许写死在仓里。
// Mutable files are reloaded; content-addressed package payloads are immutable.
// 用法：node web/serve.mjs [port]   （默认 8000，只绑 127.0.0.1）

import { createServer } from "node:http";
import { closeSync, constants, createReadStream, fstatSync, openSync, realpathSync, readFileSync, readdirSync, statSync } from "node:fs";
import { pipeline } from "node:stream/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { playerDataResponse, DEFAULT_PLAYER_API } from "./player-api.mjs";

const here = path.dirname(fileURLToPath(import.meta.url)); // .../web
const workspaceRoot = path.resolve(here, "..");

const assetRoot = (process.env.MOLY_ASSET_ROOT ?? "").trim();
if (!assetRoot) {
  console.error("serve: MOLY_ASSET_ROOT is not set; it is the directory the /assets/ mount serves.");
  process.exit(1);
}
if (!statSync(assetRoot, { throwIfNoEntry: false })?.isDirectory()) {
  console.error(`serve: MOLY_ASSET_ROOT is not a directory: ${assetRoot}`);
  process.exit(1);
}

const port = Number(process.argv[2] ?? 8000);
const jpRoot = (process.env.MOLY_JP_ASSET_ROOT ?? "").trim();
if (jpRoot && !statSync(jpRoot, { throwIfNoEntry: false })?.isDirectory()) {
  throw new Error("MOLY_JP_ASSET_ROOT must name an existing resource snapshot");
}
function snapshotInfo(root, assetBase) {
  try {
    const data = JSON.parse(readFileSync(path.join(root, "mysekai-fixtures.json"), "utf8"));
    return { region: data.region, version: data.gameVersion, assetBase };
  } catch { return { region: "unknown", version: "", assetBase }; }
}
const snapshots = [snapshotInfo(assetRoot, "/assets/"), ...(jpRoot ? [snapshotInfo(jpRoot, "/assets-jp/")] : [])];
const packRoot = (process.env.MOLY_ASSET_PACK_ROOT ?? "").trim();
if (packRoot && !statSync(packRoot, { throwIfNoEntry: false })?.isDirectory()) {
  throw new Error("MOLY_ASSET_PACK_ROOT must name an existing package directory");
}

// 前缀按序匹配：/assets/ 在前，避免被 / 的兜底吃掉。挂载根先过一遍
// resolve：越界检查比对的是 resolve 后的绝对路径，根若保留正斜杠
// （Windows 上「盘符:/…」形的 MOLY_ASSET_ROOT），startsWith 对每个
// 文件都对不上，全部 404。
const mounts = [
  ...(jpRoot ? [["/assets-jp/", realpathSync(jpRoot)]] : []),
  ...(packRoot ? [["/packs/", realpathSync(packRoot)]] : []),
  ["/assets/", realpathSync(assetRoot)],
  ["/", realpathSync(here)],
];

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".woff2", "font/woff2"],
  [".js", "text/javascript"],
  [".mjs", "text/javascript"],
  [".json", "application/json"],
  [".wasm", "application/wasm"], // 流式编译只认这个 MIME
  [".glb", "model/gltf-binary"],
  [".gltf", "model/gltf+json"],
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".webp", "image/webp"],
  [".ktx2", "image/ktx2"],
  [".bin", "application/octet-stream"],
]);

function httpError(status, message) {
  return Object.assign(new Error(message), { status });
}

function within(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(".." + path.sep) && relative !== ".." && !path.isAbsolute(relative));
}

// Mounts contain trusted, read-only development assets. Links are allowed only
// when their canonical target remains in the same mount.
function resolveFile(urlPath) {
  for (const [prefix, root] of mounts) {
    if (!urlPath.startsWith(prefix)) continue;
    const relative = urlPath.slice(prefix.length);
    let candidate = path.resolve(root, "." + path.posix.normalize("/" + relative));
    if (!within(root, candidate)) throw httpError(403, "Path leaves its mount");
    candidate = realpathSync(candidate);
    if (!within(root, candidate)) throw httpError(403, "Link leaves its mount");
    if (statSync(candidate).isDirectory()) {
      candidate = realpathSync(path.join(candidate, "index.html"));
      if (!within(root, candidate)) throw httpError(403, "Link leaves its mount");
    }
    return candidate;
  }
  throw httpError(404, "Not found");
}

const server = createServer(async (request, response) => {
  const send = (status, headers, body) => {
    response.writeHead(status, { "Cache-Control": "no-store", ...headers });
    response.end(request.method === "HEAD" ? undefined : body);
  };
  try {
    const raw = request.url ?? "/";
    if (!raw.startsWith("/") || raw.startsWith("//") || /[\\\\\u0000-\u001f\u007f]/.test(raw)) {
      throw httpError(400, "Invalid request path");
    }
    let urlPath;
    try {
      urlPath = decodeURIComponent(new URL(raw, "http://localhost").pathname);
    } catch {
      throw httpError(400, "Invalid URL encoding");
    }
    if (/[\\\\\u0000-\u001f\u007f]/.test(urlPath)) throw httpError(400, "Invalid request path");
    if (urlPath === "/snapshots.json" && (request.method === "GET" || request.method === "HEAD")) {
      send(200, { "Content-Type": "application/json" }, JSON.stringify({ version: 1, snapshots }));
      return;
    }
    if (urlPath.startsWith("/player-api/")) {
      if (request.method !== "GET") {
        send(405, { "Content-Type": "application/json", Allow: "GET" }, '{"error":"GET only"}');
        return;
      }
      const proxied = await playerDataResponse(
        new Request(new URL(raw, "http://localhost"), { method: request.method }),
        process.env.MOLY_PLAYER_API ?? DEFAULT_PLAYER_API,
      );
      send(proxied.status, Object.fromEntries(proxied.headers), Buffer.from(await proxied.arrayBuffer()));
      return;
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      send(405, { "Content-Type": "text/plain; charset=utf-8", Allow: "GET, HEAD" }, "GET/HEAD only\n");
      return;
    }
    const file = resolveFile(urlPath);
    const descriptor = openSync(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    let streamOwnsDescriptor = false;
    try {
      const stat = fstatSync(descriptor);
      if (!stat.isFile()) throw httpError(404, "Not a file");
      const type = contentTypes.get(path.extname(file).toLowerCase()) ?? "application/octet-stream";
      const immutable = urlPath.startsWith("/packs/blobs/") && /\/[a-f0-9]{64}\.(?:bin|gzz)$/.test(urlPath);
      response.writeHead(200, { "Content-Type": type, "Content-Length": stat.size,
        "Cache-Control": immutable ? "public, max-age=31536000, immutable" : "no-store" });
      if (request.method === "HEAD") response.end();
      else {
        const stream = createReadStream(file, { fd: descriptor, autoClose: true });
        streamOwnsDescriptor = true;
        await pipeline(stream, response);
      }
    } finally {
      if (!streamOwnsDescriptor) closeSync(descriptor);
    }
  } catch (error) {
    const status = error.status ?? ({ ENOENT: 404, ENOTDIR: 404, EACCES: 403, EPERM: 403, ELOOP: 403 }[error.code] ?? 500);
    if (status === 500 && !["ERR_STREAM_PREMATURE_CLOSE", "ECONNRESET"].includes(error.code)) console.error("Request failed:", error);
    if (!response.headersSent) {
      send(status, { "Content-Type": "text/plain; charset=utf-8" }, `${status === 500 ? "Request failed" : error.message}\n`);
    } else {
      response.destroy();
    }
  }
});

// 产物对源码的 mtime 提示（建议性，不拦服务）：陈产物页面看起来和新产物
// 一模一样。锚文件必须被走到，否则这个「没找到更新的」是走错目录的假安全。
function reportFreshness() {
  const wasmArtifacts = ["webgpu", "webgl2"].map(backend => path.join(here, "pkg", backend, "moly-app_bg.wasm"));
  const anchor = path.join("crates", "moly-app", "src", "main.rs");
  let newest = { mtimeMs: -1, file: null };
  let sawAnchor = false;
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (/\.(rs|wgsl|toml)$/.test(entry.name)) {
        if (path.relative(workspaceRoot, full) === anchor) sawAnchor = true;
        const mtimeMs = statSync(full).mtimeMs;
        if (mtimeMs > newest.mtimeMs) newest = { mtimeMs, file: full };
      }
    }
  };
  walk(path.join(workspaceRoot, "crates"));
  newest.mtimeMs = Math.max(newest.mtimeMs, statSync(path.join(workspaceRoot, "Cargo.toml")).mtimeMs,
    statSync(path.join(workspaceRoot, "Cargo.lock")).mtimeMs);
  const artifactMtime = Math.min(...wasmArtifacts.map(file => statSync(file, { throwIfNoEntry: false })?.mtimeMs ?? -1));
  if (artifactMtime < 0) {
    console.log("wasm artifact  MISSING -> run: node web/build-wasm.mjs");
  } else if (!sawAnchor) {
    console.log("wasm artifact  UNCHECKED (source walk missed its anchor file)");
  } else if (newest.mtimeMs > artifactMtime) {
    console.log("wasm artifacts STALE: source or manifests are newer -> run: node web/build-wasm.mjs");
  } else {
    console.log("wasm artifacts ok (both renderer modules are newer than source and manifests)");
  }
}

reportFreshness();
server.listen(port, "127.0.0.1", () => {
  const listeningPort = server.address().port;
  console.log(`serving on http://127.0.0.1:${listeningPort}/`);
  console.log(`  page   : /  (index.html)`);
  console.log(`  assets : /assets/ -> ${assetRoot}`);
  if (packRoot) console.log(`  packs  : /packs/ -> ${packRoot}`);
  console.log(`open: http://127.0.0.1:${listeningPort}/index.html?assets=/assets/`);
});
