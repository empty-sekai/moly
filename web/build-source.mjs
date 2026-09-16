import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
/** Fingerprint actual dirty-tree inputs, not just HEAD or file timestamps. */
export function workspaceFingerprint(root) {
  const files = [];
  const visit = (relative) => {
    for (const entry of readdirSync(path.join(root, relative), {
      withFileTypes: true,
    })) {
      if (["target", ".git", "node_modules"].includes(entry.name)) continue;
      const file = `${relative}/${entry.name}`;
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) files.push(file);
      else throw new Error(`Build inputs must not contain links: ${file}`);
    }
  };
  visit("crates");
  for (const file of [
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/config.toml",
    "rust-toolchain.toml",
    "rust-toolchain",
  ]) {
    if (existsSync(path.join(root, file))) files.push(file);
  }
  const digest = createHash("sha256");
  for (const file of files.sort())
    digest
      .update(file)
      .update("\0")
      .update(sha256(readFileSync(path.join(root, file))))
      .update("\n");
  return digest.digest("hex");
}
