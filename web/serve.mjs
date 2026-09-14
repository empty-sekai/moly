// 最小静态服务器（开发用）：/ → web/（页面 + pkg/），/assets/ → 资产根。
// 资产根只认 MOLY_ASSET_ROOT——没有它就拒绝启动：半通的页面（wasm 起来、
// 资产逐个 404）比不启动更难查，而这台机器的资产根路径不许写死在仓里。
// Mutable files are reloaded; content-addressed package payloads are immutable.
// 用法：node web/serve.mjs [port]   （默认 8000，只绑 127.0.0.1）

import { createServer } from "node:http";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
const packRoot = (process.env.MOLY_ASSET_PACK_ROOT ?? "").trim();
if (packRoot && !statSync(packRoot, { throwIfNoEntry: false })?.isDirectory()) {
  throw new Error("MOLY_ASSET_PACK_ROOT must name an existing package directory");
}

// 前缀按序匹配：/assets/ 在前，避免被 / 的兜底吃掉。挂载根先过一遍
// resolve：越界检查比对的是 resolve 后的绝对路径，根若保留正斜杠
// （Windows 上「盘符:/…」形的 MOLY_ASSET_ROOT），startsWith 对每个
// 文件都对不上，全部 404。
const mounts = [
  ...(packRoot ? [["/packs/", path.resolve(packRoot)]] : []),
  ["/assets/", path.resolve(assetRoot)],
  ["/", path.join(here)],
];

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
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

function resolveFile(urlPath) {
  for (const [prefix, root] of mounts) {
    if (!urlPath.startsWith(prefix)) continue;
    const relative = urlPath.slice(prefix.length);
    const absolute = path.resolve(root, "." + path.posix.normalize("/" + relative));
    // 拒绝走出挂载根：resolve 已把 ../ 折掉，前缀对不上就是走出来了
    // （根自身无尾分隔符，须单独放行，否则裸根 404）。
    if (absolute !== root && !absolute.startsWith(root + path.sep)) return null;
    let candidate = absolute;
    const stats = statSync(candidate, { throwIfNoEntry: false });
    if (stats?.isDirectory()) {
      candidate = path.join(candidate, "index.html");
    }
    if (statSync(candidate, { throwIfNoEntry: false })?.isFile()) return candidate;
    return null;
  }
  return null;
}

const server = createServer((request, response) => {
  const urlPath = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  const file = resolveFile(urlPath);
  const send = (status, headers, body) => {
    const immutable = status === 200 && urlPath.startsWith("/packs/blobs/") && /\/[a-f0-9]{64}\.(?:bin|gzz)$/.test(urlPath);
    response.writeHead(status, { "Cache-Control": immutable ? "public, max-age=31536000, immutable" : "no-store", ...headers });
    response.end(body);
  };
  if (!file) {
    console.log("404", urlPath);
    send(404, { "Content-Type": "text/plain; charset=utf-8" }, `not found: ${urlPath}\n`);
    return;
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    send(405, { "Content-Type": "text/plain; charset=utf-8" }, "GET/HEAD only\n");
    return;
  }
  const type = contentTypes.get(path.extname(file).toLowerCase()) ?? "application/octet-stream";
  const body = request.method === "HEAD" ? undefined : readFileSync(file);
  send(200, { "Content-Type": type, "Content-Length": statSync(file).size }, body);
});

// 产物对源码的 mtime 提示（建议性，不拦服务）：陈产物页面看起来和新产物
// 一模一样。锚文件必须被走到，否则这个「没找到更新的」是走错目录的假安全。
function reportFreshness() {
  const wasmArtifact = path.join(here, "pkg", "moly-app_bg.wasm");
  const anchor = path.join("crates", "moly-app", "src", "main.rs");
  let newest = { mtimeMs: -1, file: null };
  let sawAnchor = false;
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith(".rs")) {
        if (path.relative(workspaceRoot, full) === anchor) sawAnchor = true;
        const mtimeMs = statSync(full).mtimeMs;
        if (mtimeMs > newest.mtimeMs) newest = { mtimeMs, file: full };
      }
    }
  };
  walk(path.join(workspaceRoot, "crates"));
  const artifactMtime = statSync(wasmArtifact, { throwIfNoEntry: false })?.mtimeMs ?? -1;
  if (artifactMtime < 0) {
    console.log("wasm artifact  MISSING -> run: node web/build-wasm.mjs");
  } else if (!sawAnchor) {
    console.log("wasm artifact  UNCHECKED (source walk missed its anchor file)");
  } else if (newest.mtimeMs > artifactMtime) {
    console.log(`wasm artifact  STALE: ${newest.file} is newer -> run: node web/build-wasm.mjs`);
  } else {
    console.log("wasm artifact  ok (newest .rs under crates/ is older than pkg/moly-app_bg.wasm)");
  }
}

reportFreshness();
server.listen(port, "127.0.0.1", () => {
  console.log(`serving on http://127.0.0.1:${port}/`);
  console.log(`  page   : /  (index.html)`);
  console.log(`  assets : /assets/ -> ${assetRoot}`);
  if (packRoot) console.log(`  packs  : /packs/ -> ${packRoot}`);
  console.log(`open: http://127.0.0.1:${port}/index.html?assets=/assets/`);
});
