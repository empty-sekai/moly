# Browser WASM size audit, 2026-09-16

The browser profile is `wasm-size`: `opt-level = "s"`, fat LTO, one codegen
unit, abort-on-panic, and stripped DWARF. The release publisher additionally
removes only the WASM `name` and `.debug_*` custom sections and creates gzip
(level 9) and Brotli (quality 9) representations. No `wasm-opt` output is used.

## Feature boundary

`Cargo.toml` now explicitly expands Bevy 0.18.1's previous default `2d`, `3d`
and `ui` feature groups. This audit changes one group: unused Bevy gizmos and
engine picking. The removed functional features are `bevy_gizmos`,
`bevy_gizmos_render`, `bevy_picking`, `mesh_picking`, `sprite_picking`, and
`ui_picking`. Umbrella names also disappear because their leaf features are
listed explicitly; that does not remove their retained capabilities.

Moly handles scene picking through its own `gesture.rs`, `pick.rs` camera-ray
tests and `ui_layout::PointerUi`. The source does not use Bevy's `Pointer`
events, picking backends or gizmos. Bevy's `ui_focus_system`, responsible for
existing `Button`/`Interaction` behavior, is installed independently of the
optional `UiPickingPlugin`.

Retained capabilities include:

- Audio output and Vorbis, including `AudioPlayer`, `AudioSink`, `AudioSource`,
  `PlaybackSettings` and the existing PCM voice path.
- glTF, animation, morph targets, PBR, sprite and UI rendering, Bevy text, PNG,
  HDR, KTX2 and zstd support.
- `bevy_anti_alias`: FXAA is a real option in `game_settings.rs`, enabled by
  default. Postprocessing, SMAA/tonemapping LUTs and the custom weather and
  fixture-emission render graph remain present.
- Existing native platform support, input, gamepad, diagnostics, multithreading,
  default font, asset reflection and logging. Target-gated platform crates are
  not treated as downloaded browser bytes merely because Cargo lists them.
- WebGPU and WebGL2 remain separate builds. Only the application `webgpu`
  feature distinguishes them; the WebGL2 build uses `--no-default-features`.

The resolved feature graph and the exact difference are saved in the local
audit evidence directory as `feature-diff.json` and
`candidate-feature-tree.txt`. The target dependency tree contains no Bevy
gizmos or picking crates. `Cargo.lock` only removes obsolete dependency edges;
no dependency versions change.

## Font audit

The actual `ResourceHanRoundedSC-Medium.subset.ttf` is **2,909,156 bytes**.
Both original `wasm-size` artifacts contain exactly **one** byte-for-byte copy
of that complete font. The older four-copy audit belongs to the previous
release profile; its result must not be attributed to the current profile.
Fat LTO already merges the shared literal. No glyphs, font coverage, source
font or UI appearance were changed. Adding a font resource abstraction would
not shrink the current WASM and was therefore avoided.

The browser stage also skips native settings/catalogue font asset construction
when the external host UI is active. Source-level references remaining in those
native views do not demonstrate duplicate font bytes in the browser artifact.

## Reproduction

From the runtime workspace, choose an ignored output directory:

```powershell
node web/build-wasm.mjs --out-dir=web/pkg
$env:CARGO_BUILD_JOBS = '2'
cargo test --workspace --lib --locked
# Set MOLY_ASSET_ROOT to an extracted source asset directory, then:
cargo test --workspace --lib --locked -- --ignored
```

The build checks the installed wasm-bindgen against the lockfile and records
the hash of actual working-tree Rust/Cargo inputs, including uncommitted
inputs. Publishing rejects a source fingerprint mismatch. The measurement
script verifies the build digest and current fingerprint, compiles each
stripped module with `WebAssembly.compile`, checks the stage/library exports,
counts exact font blobs and hashes every compression representation.

Toolchain: Rust/Cargo 1.94.0, wasm-bindgen 0.2.127. The evidence directory also
contains the current baseline/candidate symbol audits and all build/test logs. `raw` means
wasm-bindgen's output before release debug-name removal; `decoded` means the
published WASM bytes after that removal. Compressed sizes are representations
of the decoded bytes, not additional runtime payloads.

## Baseline

| Backend | Raw bytes | Decoded bytes | gzip bytes | Brotli bytes |
| --- | ---: | ---: | ---: | ---: |
| WebGPU | 54,733,150 | 42,706,470 | 12,171,282 | 9,137,240 |
| WebGL2 | 54,519,444 | 42,617,157 | 12,148,574 | 9,090,906 |

Baseline fingerprint:
`df755a73afa4c69c32bce2abc65269598b631f0a2069539830ae85cb5c7a37c1`.

## Final artifacts

Fingerprint:
`bdc619fb48e70c4b239c65531512e6cd087b8eda45709e0d7123914605653c99`.

| Backend | Raw bytes | Decoded bytes | gzip bytes | Brotli bytes |
| --- | ---: | ---: | ---: | ---: |
| WebGPU | 52,712,781 | 41,306,822 | 11,884,955 | 8,928,672 |
| WebGL2 | 52,509,170 | 41,226,927 | 11,863,137 | 8,933,047 |

For WebGPU this removes 1,399,648 decoded bytes (3.28%), 286,327 gzip bytes
(2.35%) and 208,568 Brotli bytes (2.28%) relative to the existing size profile.
The reduction is modest relative to the earlier profile improvement; it is
retained only with both renderer regressions passing. The module also contains
5,318 fewer functions (105,874 to 100,556), and the named picking/gizmos function
groups disappear entirely. There is no new runtime code path or per-platform
implementation to maintain.

For WebGL2 this removes 1,390,230 decoded bytes (3.26%), 285,437 gzip bytes
(2.35%) and 157,859 Brotli bytes (1.74%). Both modules contain exactly one
complete font and pass digest, fingerprint, compile and exported API checks.

Both backends build and pass stripped-module compile/export checks. Workspace
library tests pass: app 2, assets 14, game 109, law 506. Six normally ignored
source-corpus tests were then run with `MOLY_ASSET_ROOT` set to the extracted
source assets and also pass: 637 tests exercised in total, zero failures.
The extra corpus run checks the three rain-family effect profiles and the
29 placed furniture packages' source mip counts.

The final local QA release is `stage-85a6f76ccc10d0ffe25b`. Real Chrome
regressions cover CN furniture preview/full dialogue and JP general dialogue
across WebGPU and WebGL2, including JP mobile WebGL2. The acceptance records
verify the expected speaker/text lines, natural completion, scene restoration,
the original dialogue UI and nonzero audio PCM with a running AudioContext.
The WebGPU matrix also exercises 11 representative entries and lifecycle
stop, cancellation, replacement and failure recovery.

The publisher's final decoded/gzip/Brotli numbers exactly match the measurements
above. Actual local HTTP requests pass Brotli, gzip and identity negotiation,
range 206 and cache revalidation 304 on both renderer artifacts. Full reports
are stored as `final-cn-webgl2.json`, `final-jp-webgl2-mobile.json`,
`candidate-matrix.log`, `candidate-lifecycle-final.log`, `measurements.json`
and `serving-final.log` in the local QA evidence directories. The final feature
group is retained based on those results.

This is a working-tree fingerprint, not a claim that every pre-existing native
source change belongs to these commits. Unrelated native portrait exporter
work remains outside this closeout's commits and is excluded from WASM by its
target configuration. Desktop interactive gameplay and other native operating
systems were not exercised by this browser regression; native coverage here
is the complete workspace library and source-corpus test suite.
