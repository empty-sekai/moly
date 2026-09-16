// 造浏览器侧产物：cargo 编 wasm32 → wasm-bindgen 出 --target web 胶水，
// 全部落进本目录的 pkg/（gitignore，可随时重建）。只用 Node 内建，无 npm
// 依赖。用法：node web/build-wasm.mjs

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { workspaceFingerprint, sha256 } from "./build-source.mjs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url)); // .../web
const workspaceRoot = path.resolve(here, "..");
const profileName =
  process.argv.find((arg) => arg.startsWith("--profile="))?.split("=")[1] ??
  "wasm-size";
if (!["release", "wasm-size"].includes(profileName))
  throw new Error("Unsupported WASM profile");
const outputArg = process.argv
  .find((arg) => arg.startsWith("--out-dir="))
  ?.slice("--out-dir=".length);
const outDir = outputArg ? path.resolve(outputArg) : path.join(here, "pkg");
const lockPath = path.join(workspaceRoot, "Cargo.lock");
const requestedBackend = process.argv
  .find((arg) => arg.startsWith("--renderer="))
  ?.split("=")[1];
if (requestedBackend && !["webgpu", "webgl2"].includes(requestedBackend)) {
  throw new Error("--renderer must be webgpu or webgl2");
}
// Keep local builds responsive; callers can explicitly raise this limit.
process.env.CARGO_BUILD_JOBS ??= "2";

function findOnPath(command) {
  const entries = (process.env.PATH ?? "")
    .split(path.delimiter)
    .filter(Boolean);
  const candidates =
    process.platform === "win32" ? [command, `${command}.exe`] : [command];
  for (const entry of entries) {
    for (const candidate of candidates) {
      const fullPath = path.join(entry, candidate);
      if (existsSync(fullPath)) return fullPath;
    }
  }
  return null;
}

function resolveTool(envName, command) {
  const configured = process.env[envName];
  if (configured) {
    if (!existsSync(configured)) {
      throw new Error(
        `${envName} points at a missing executable: ${configured}`,
      );
    }
    return configured;
  }
  const onPath = findOnPath(command);
  if (onPath) return onPath;
  throw new Error(
    `${command} not found; set ${envName} to its executable path`,
  );
}

function run(cmd, args) {
  console.log(`+ ${cmd} ${args.join(" ")}`);
  const result = spawnSync(cmd, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${cmd} exited with status ${result.status}`);
  }
}

const CARGO = resolveTool("CARGO", "cargo");
const WASM_BINDGEN = resolveTool("WASM_BINDGEN", "wasm-bindgen");

// 工件名随包名原样带连字符（native exe 同为 moly-app.exe）；wasm-bindgen
// 的输出则归一成下划线（moly_app.js / moly_app_bg.wasm）。target 目录不写
// 死 `target/`：让 cargo metadata 给出 target_directory，也尊重使用者
// 通过 CARGO_TARGET_DIR 配置的缓存目录。
const metadata = spawnSync(
  CARGO,
  [
    "metadata",
    "--format-version",
    "1",
    "--no-deps",
    "--manifest-path",
    path.join(workspaceRoot, "Cargo.toml"),
  ],
  { encoding: "utf8" },
);
if (metadata.error) throw metadata.error;
if (metadata.status !== 0) {
  throw new Error(
    `cargo metadata exited with status ${metadata.status}: ${metadata.stderr}`,
  );
}
const cargoWasmPath = path.join(
  JSON.parse(metadata.stdout).target_directory,
  "wasm32-unknown-unknown",
  profileName,
  "moly-app.wasm",
);

// wasm-bindgen CLI 只吃与自己 schema 版本精确一致的输入，不一致时报的是
// 一段多行 schema 文案、不直说版本不匹配。这里在对编之前先一行说清两边
// 版本。（顺序上放在 cargo 前：版本错了不必先烧一场长编译。）
const lockText = readFileSync(lockPath, "utf8");
// \r? 是承重的：本树 Cargo.lock 在 Windows 检出是 CRLF，裸 \n 一行都配不上。
const lockMatch = lockText.match(
  /name = "wasm-bindgen"\r?\nversion = "([^"]+)"/,
);
if (!lockMatch) {
  throw new Error(`no "wasm-bindgen" package entry found in ${lockPath}`);
}
const cliVersion = spawnSync(WASM_BINDGEN, ["--version"], { encoding: "utf8" });
if (cliVersion.error) throw cliVersion.error;
const cliMatch = /wasm-bindgen\s+(\S+)/.exec(cliVersion.stdout ?? "");
if (!cliMatch) {
  throw new Error(
    `could not parse a version out of "${WASM_BINDGEN} --version" output: ${JSON.stringify(cliVersion.stdout)}`,
  );
}
if (lockMatch[1] !== cliMatch[1]) {
  throw new Error(
    `wasm-bindgen version mismatch: Cargo.lock has ${lockMatch[1]}, installed CLI is ${cliMatch[1]}. ` +
      `Fix with: cargo install wasm-bindgen-cli --version ${lockMatch[1]}`,
  );
}
console.log(
  `wasm-bindgen version check: Cargo.lock and CLI both ${cliMatch[1]}`,
);

// release：这份产物就是浏览器要下载的东西；dev 档的 bevy wasm 体积大到不可用。
// Engine buffer layouts depend on these features at compile time. Each module
// has the layout for its backend; the bootstrap downloads only the chosen one.
for (const [backend, features] of [
  ["webgpu", ["--features", "webgpu"]],
  ["webgl2", ["--no-default-features"]],
]) {
  if (requestedBackend && requestedBackend !== backend) continue;
  const sourceFingerprint = workspaceFingerprint(workspaceRoot);
  run(CARGO, [
    "build",
    "--profile",
    profileName,
    "--target",
    "wasm32-unknown-unknown",
    "-p",
    "moly-app",
    ...features,
    "--manifest-path",
    path.join(workspaceRoot, "Cargo.toml"),
  ]);
  if (!existsSync(cargoWasmPath))
    throw new Error(`cargo build did not produce ${cargoWasmPath}`);
  const backendDir = path.join(outDir, backend);
  mkdirSync(backendDir, { recursive: true });
  run(WASM_BINDGEN, [
    "--target",
    "web",
    "--out-dir",
    backendDir,
    cargoWasmPath,
  ]);
  for (const name of ["moly-app.js", "moly-app_bg.wasm"]) {
    const file = path.join(backendDir, name);
    if (!existsSync(file))
      throw new Error(`wasm-bindgen did not produce ${file}`);
    console.log(`${backend}/${name}: ${statSync(file).size} bytes`);
  }
  if (workspaceFingerprint(workspaceRoot) !== sourceFingerprint)
    throw new Error(
      "Rust source changed during build; rebuild before publishing",
    );
  writeFileSync(
    path.join(backendDir, "build.json"),
    JSON.stringify(
      {
        schemaVersion: 1,
        backend,
        profile: profileName,
        sourceFingerprint,
        bindgenVersion: cliMatch[1],
        files: Object.fromEntries(
          ["moly-app.js", "moly-app_bg.wasm"].map((name) => [
            name,
            sha256(readFileSync(path.join(backendDir, name))),
          ]),
        ),
      },
      null,
      2,
    ) + "\n",
  );
}
console.log(`output: ${outDir}`);
