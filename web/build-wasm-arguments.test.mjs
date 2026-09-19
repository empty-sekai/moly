import test from "node:test";
import assert from "node:assert/strict";
import { parseWasmBuildArguments } from "./build-wasm-arguments.mjs";

test("explicit isolated output works with equals or separate arguments", () => {
  const expected = {
    profile: "wasm-size",
    renderer: null,
    outDir: "X:\\fixture folder\\wasm=one",
  };
  assert.deepEqual(
    parseWasmBuildArguments([
      "--out-dir=X:\\fixture folder\\wasm=one",
      "--renderer=both",
    ]),
    expected,
  );
  assert.deepEqual(
    parseWasmBuildArguments([
      "--renderer",
      "both",
      "--out-dir",
      "X:\\fixture folder\\wasm=one",
    ]),
    expected,
  );
});
test("backend-specific and diagnostic profiles are preserved", () => {
  for (const renderer of ["webgpu", "webgl2"])
    for (const profile of ["wasm-size", "wasm-names", "release"]) {
      assert.deepEqual(
        parseWasmBuildArguments([
          `--renderer=${renderer}`,
          `--profile=${profile}`,
        ]),
        { renderer, profile, outDir: null },
      );
    }
});
test("default publication is possible only with intentionally omitted output arguments", () => {
  assert.deepEqual(parseWasmBuildArguments([]), {
    profile: "wasm-size",
    renderer: null,
    outDir: null,
  });
  for (const args of [
    ["--out", "X:\\fixture"],
    ["--output-dir=X:\\fixture"],
    ["--out-dir"],
    ["--out-dir="],
    ["--out-dir", "--renderer=webgl2"],
    ["X:\\fixture"],
  ]) {
    assert.throws(() => parseWasmBuildArguments(args));
  }
});
test("unknown, duplicate or invalid flags fail rather than silently changing the build", () => {
  for (const args of [
    ["--profile=debug"],
    ["--renderer=vulkan"],
    ["--renderer=webgpu", "--renderer=webgl2"],
    ["--out-dir=a", "--out-dir", "b"],
    ["--unknown=1"],
  ]) {
    assert.throws(() => parseWasmBuildArguments(args));
  }
});
