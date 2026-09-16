// Deployment projection only: preserve every original package JSON byte.
// Source controller compilation and playback still belong to the Rust owner.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { sha256 } from "./build-source.mjs";

function whitespace(text, offset) {
  while (/\s/.test(text[offset] ?? "") && offset < text.length) offset++;
  return offset;
}
function endValue(text, offset) {
  const first = text[offset];
  if (first === '"') {
    for (let i = offset + 1; i < text.length; i++) {
      if (text[i] === "\\") i++;
      else if (text[i] === '"') return i + 1;
    }
    throw new Error("Unterminated JSON string");
  }
  if (first === "{" || first === "[") {
    const stack = [first];
    for (let i = offset + 1; i < text.length; i++) {
      const token = text[i];
      if (token === '"') {
        i = endValue(text, i) - 1;
        continue;
      }
      if (token === "{" || token === "[") stack.push(token);
      else if (token === "}" || token === "]") {
        if (stack.pop() !== (token === "}" ? "{" : "["))
          throw new Error("Unbalanced JSON");
        if (!stack.length) return i + 1;
      }
    }
    throw new Error("Truncated JSON");
  }
  let end = offset;
  while (end < text.length && !/[\s,\]}]/.test(text[end])) end++;
  if (end === offset) throw new Error("Missing JSON value");
  JSON.parse(text.slice(offset, end));
  return end;
}
function fields(text) {
  let offset = whitespace(text, 0);
  if (text[offset++] !== "{") throw new Error("Expected an object");
  const result = new Map();
  while (true) {
    offset = whitespace(text, offset);
    if (text[offset] === "}") {
      offset++;
      break;
    }
    if (text[offset] !== '"') throw new Error("Expected object key");
    const keyEnd = endValue(text, offset),
      key = JSON.parse(text.slice(offset, keyEnd));
    offset = whitespace(text, keyEnd);
    if (text[offset++] !== ":") throw new Error("Missing object colon");
    offset = whitespace(text, offset);
    const end = endValue(text, offset);
    if (result.has(key)) throw new Error("Duplicate source key");
    result.set(key, [offset, end]);
    offset = whitespace(text, end);
    if (text[offset] === "}") {
      offset++;
      break;
    }
    if (text[offset++] !== ",") throw new Error("Missing object delimiter");
    if (text[whitespace(text, offset)] === "}")
      throw new Error("Trailing object comma");
  }
  if (whitespace(text, offset) !== text.length)
    throw new Error("Trailing JSON data");
  return result;
}
export function* sourcePackages(text) {
  const root = fields(text),
    version = root.get("version"),
    range = root.get("packages");
  if (!version || JSON.parse(text.slice(...version)) !== 1 || !range)
    throw new Error("Unknown source controller document");
  let offset = whitespace(text, range[0]);
  if (text[offset++] !== "[") throw new Error("Expected source package array");
  const names = new Set();
  while (true) {
    offset = whitespace(text, offset);
    if (text[offset] === "]") break;
    const end = endValue(text, offset),
      raw = text.slice(offset, end);
    // Parse one package at a time for validation only. Re-serialization would
    // round large signed source IDs, so emit raw text, never this parsed value.
    const name = JSON.parse(raw).name;
    if (typeof name !== "string" || !name || names.has(name))
      throw new Error("Missing or duplicate package identity");
    names.add(name);
    yield { name, raw };
    offset = whitespace(text, end);
    if (text[offset] === "]") break;
    if (text[offset++] !== "," || text[whitespace(text, offset)] === "]")
      throw new Error("Invalid package delimiter");
  }
  if (offset + 1 !== range[1])
    throw new Error("Invalid package array boundary");
}
function putImmutable(file, bytes) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (fs.existsSync(file)) {
    if (!fs.readFileSync(file).equals(bytes))
      throw new Error(`Immutable controller package changed: ${file}`);
  } else fs.writeFileSync(file, bytes, { flag: "wx" });
}
export function splitGimmicks(assets) {
  assets = fs.realpathSync(assets);
  const master = JSON.parse(
    fs.readFileSync(path.join(assets, "mysekai-fixtures.json"), "utf8"),
  );
  const { region, gameVersion } = master;
  if (!["cn", "jp"].includes(region) || !/^\d+\.\d+\.\d+$/.test(gameVersion))
    throw new Error("Explicit snapshot identity required");
  const source = fs.readFileSync(
    path.join(assets, "fixture-gimmick/gimmicks.json"),
    "utf8",
  );
  const index = {
    schemaVersion: 1,
    generator: "moly-gimmick-split-v1",
    region,
    gameVersion,
    sourceSha256: sha256(source),
    packages: [],
  };
  for (const { name, raw } of sourcePackages(source)) {
    const bytes = Buffer.from(
      `{"schemaVersion":1,"region":${JSON.stringify(region)},"gameVersion":${JSON.stringify(gameVersion)},"package":${raw}}`,
    );
    const hash = sha256(bytes),
      relative = `fixture-gimmick/by-package/${hash}.json`;
    putImmutable(path.join(assets, relative), bytes);
    const gzip = gzipSync(bytes, { level: 9 });
    putImmutable(path.join(assets, relative + ".gz"), gzip);
    index.packages.push({
      name,
      path: relative,
      sha256: hash,
      decodedBytes: bytes.length,
      downloadBytes: gzip.length,
    });
  }
  const destination = path.join(assets, "fixture-gimmick/browser-index.json");
  if (
    fs.existsSync(destination) &&
    JSON.parse(fs.readFileSync(destination, "utf8")).generator !==
      index.generator
  )
    throw new Error("Existing index belongs to another producer");
  const temporary = `${destination}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, JSON.stringify(index), { flag: "wx" });
  fs.renameSync(temporary, destination);
  return {
    region,
    version: gameVersion,
    packages: index.packages.length,
    sourceBytes: Buffer.byteLength(source),
    indexBytes: fs.statSync(destination).size,
    maxPackageBytes: Math.max(0, ...index.packages.map((p) => p.decodedBytes)),
    sourceSha256: index.sourceSha256,
  };
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const args = process.argv.slice(2);
    if (args.length !== 2 || args[0] !== "--assets")
      throw new Error(
        "Usage: node web/split-gimmicks.mjs --assets /private/runtime",
      );
    console.log(JSON.stringify(splitGimmicks(args[1]), null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
