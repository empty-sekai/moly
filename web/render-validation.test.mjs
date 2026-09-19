import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import {
  isRenderFailure,
  assertVisibleScene,
} from "./qa/render-validation.mjs";

test("browser GPU warnings cannot masquerade as successful playback", () => {
  assert.equal(
    isRenderFailure(
      "Binding doesn't exist in [BindGroupLayoutInternal mesh_view_layout_multisampled_depth]",
    ),
    true,
  );
  assert.equal(
    isRenderFailure(
      "[Invalid CommandBuffer] is invalid due to a previous error.",
    ),
    true,
  );
  assert.equal(
    isRenderFailure(
      "The powerPreference option is currently ignored on Windows.",
    ),
    false,
  );
});
test("HTML controls do not validate an empty central scene", () => {
  assert.throws(
    () =>
      assertVisibleScene({
        samples: 10000,
        nonBlack: 0,
        minimum: 0,
        maximum: 0,
        distinct: 1,
      }),
    /blank/,
  );
  assert.throws(
    () =>
      assertVisibleScene({
        samples: 10000,
        nonBlack: 10000,
        minimum: 100,
        maximum: 100,
        distinct: 1,
      }),
    /uniform/,
  );
  assert.doesNotThrow(() =>
    assertVisibleScene({
      samples: 10000,
      nonBlack: 8000,
      minimum: 0,
      maximum: 200,
      distinct: 500,
    }),
  );
});
test("WebGPU and WebGL2 renderer features are selected separately", () => {
  const workspace = fs.readFileSync(
    new URL("../Cargo.toml", import.meta.url),
    "utf8",
  );
  const application = fs.readFileSync(
    new URL("../crates/moly-app/Cargo.toml", import.meta.url),
    "utf8",
  );
  const build = fs.readFileSync(
    new URL("./build-wasm.mjs", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(workspace, /"webgl2"/);
  assert.match(application, /webgl2\s*=\s*\["bevy\/webgl2"\]/);
  assert.match(application, /webgpu\s*=\s*\["bevy\/webgpu"\]/);
  assert.match(
    build,
    /\["webgl2", \["--no-default-features", "--features", "webgl2"\]\]/,
  );
});

test("actual GLES depth-copy warnings are fatal even when console type is warning", () => {
  assert.equal(
    isRenderFailure(
      "[.WebGL-0x380c00f9c200] GL_INVALID_FRAMEBUFFER_OPERATION: glCopyTexSubImage2D: Framebuffer is incomplete: Depth stencil texture in color attachment.",
    ),
    true,
  );
  assert.equal(
    isRenderFailure(
      "WebGL: too many errors, no more errors will be reported to the console for this context.",
    ),
    true,
  );
  assert.equal(
    isRenderFailure("GL_INVALID_OPERATION: drawElements: feedback loop"),
    true,
  );
  assert.equal(isRenderFailure("GL_OUT_OF_MEMORY: texImage2D"), true);
  assert.equal(isRenderFailure("WebGL renderer selected"), false);
});
