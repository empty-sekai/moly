# Browser shell checks

The published browser app is plain HTML, CSS and ES modules. npm dependencies
are used only by the DOM regression tests and are not part of the runtime build.

Use Node.js 22.13 or later (or Node.js 24):

```sh
cd web
npm ci
npm run format:check
npm test
```

`npm run format` formats the browser shell, adapter, tests and their documentation
with the pinned Prettier version. Runtime behavior is unchanged by formatting.

`engine-loader.test.mjs` checks engine and font loading. Stage and game contract
tests exercise the maintained iframe and full-screen entry protocols. Tests run
sequentially because DOM fixtures own process globals.

`serve.test.mjs` checks resource transport, MIME types, missing files and
cancellation through an isolated development server with synthetic assets.

The full-screen game's browser acceptance must use the `moly-web` site and a
publication with the `game` profile. Check entry, in-game UI, persistence and
input ownership there. Stage acceptance below covers the embedding interface.
Inspect actual browser screenshots; DOM assertions do not establish rendering
or playback acceptance.

## Portable stage acceptance

The real-browser harness is in `web/qa/acceptance.mjs`; Playwright is a pinned
build/test dependency, never part of the public runtime. Use a locally installed
Chrome, or set `MOLY_BROWSER_CHANNEL` to the desired installed channel. The
optional `MOLY_PLAYWRIGHT_MODULE` environment variable can reuse an existing
Playwright installation. It is not required after `npm ci`.

Start the existing development server with explicit external CN/JP asset roots,
then run from the repository root:

```sh
node web/qa/acceptance.mjs --region jp --backend webgpu --out /tmp/moly-evidence
node web/qa/acceptance.mjs --region cn --backend webgpu --suite matrix --out /tmp/moly-evidence
node web/qa/acceptance.mjs --region cn --backend auto-fallback --out /tmp/moly-evidence
node web/qa/acceptance.mjs --region jp --backend webgl2 --mobile --out /tmp/moly-evidence
node web/qa/acceptance.mjs --region cn --suite lifecycle --out /tmp/moly-evidence
```

For production paths, add `--origin`, `--stage` and `--assets` matching the
published same-origin manifest. The harness checks the real master descriptor
before selecting the runtime. Evidence must go outside source and is not added
to the application bundle or Git. It exports a separate source-qualified
catalogue for each region; do not rename one region's export as another.

`--content` is repeatable for focused real-source samples. `--video` records the
continuous browser output, and `--orbit` performs real pointer camera gestures
before checking restoration. Use the resulting actual frames and recording to
judge character contact, sitting/riding, bubbles, expression and small furniture
visibility. A `Playing` flag or a single screenshot does not prove the action.

The matrix covers the exact conversation, furniture-story, loop/one-shot,
Timeline, NoTalk, short-bubble and riding samples used in runtime acceptance.
It starts with no NPC cast and no placed furniture. It verifies the clean
preview has no gathering objects, and compares the original player pose,
camera state/model/lens, layout/cast, appearance, source owners, controller
leases, voice players, scoped sounds and bubbles after each completion/stop.
Source-specific identities are validated before each sample.

The lifecycle suite checks rejected current-scene admission, browse while
playing, replacing an active controller, cancelling preparation, and recovery
from an explicitly injected HTTP 503 for one real source controller package.
Only that intentional fault's exact URL is excluded from unexpected HTTP-error
assertions. The source fixture and animation data are never fabricated or
patched. Permanent load failure must promptly use the ordinary restoration
path rather than wait out an enlarged timeout. Subsequent genuine playback
must still work in the same renderer and audio context.

`library_diagnostics()` is an explicitly requested read-only WASM QA projection
of the existing native diagnostic owner. Normal hosts do not call it. It exposes
no command, does not modify the world and is not forwarded by postMessage.
The compact UI snapshot and full exported catalogue remain separate APIs.
The diagnostic actor list includes the player; the UI's `scene.actorUnits`
excludes the player, so compare the correct baseline instead of confusing those
lists. Full diagnostics are produced at the native QA sampling cadence.

## Transport and protocol regression

`stage-contract.test.mjs` checks exact origin/window/version validation, source
mismatch, allowed intentions, queue limits, concurrent close acknowledgment,
retry handshakes, and the stage singleton. It explicitly checks
that theme/locale/browse do not create another iframe or cancel playback.

`cache-worker-runtime.test.mjs` executes the shipped worker under controlled
storage failures, checking quota/read failures, preference persistence,
aggregate pending-byte bounds, scoped clear and disabled retention. This
complements, but does not replace, real-browser cold/warm/clear/disabled tests.

`split-gimmicks.test.mjs` checks source JSON slicing, large integer preservation,
malformed input and duplicate identities. `prepare-browser-assets.test.mjs`
checks measured source paths, lossless gzip, region/version mismatch and stale
sidecars. `release-artifact.test.mjs` checks exact catalogue identity and WASM
section stripping without changing runtime sections.

Run the repository boundary and version gates before committing:

```sh
node tools/boundary/check.mjs
python tools/version/check.py
cargo test --workspace --lib
node web/build-wasm.mjs
```

Both backends must have the same current workspace source fingerprint; a stale
`build.json`, old published release or previous screenshot is not evidence for
a new runtime change. Keep failure evidence separately when rerunning. Refresh
the host's development HTML cache after changing a build; stale HTML can produce
false hydration failures. A running standalone process must be stopped before
rebuilding its output directory on platforms that lock those files.

## Automatic cache lifecycle

Run the published host in a fresh Chromium context for each resource region:

```sh
node web/qa/cache-lifecycle.mjs --origin http://127.0.0.1:8082 --region cn --out /tmp/moly-cache-cn
node web/qa/cache-lifecycle.mjs --origin http://127.0.0.1:8082 --region jp --out /tmp/moly-cache-jp
node web/qa/serving.mjs --origin http://127.0.0.1:8082 --out /tmp/moly-serving
```

This checks automatic base preparation, every necessary file offline with its
declared length/SHA-256, warmed dialogue in a freshly reloaded offline runtime,
the visible resource-manager clear button, zero bytes/entries, preservation of
localStorage and unrelated caches, and download followed by another offline
verification. All HTTP failure URLs are captured; unexpected Moly errors fail.
The host's mutable discovery metadata and Next.js shell are outside worker scope;
this does not promise a full site launch without a network connection.

The portable stage acceptance records one running AudioContext and actual
non-silent PCM buffer starts. Every tested talk must show its source speaker/text
lines and complete naturally. The cache suite warms and replays the complete
selected dialogue in a fresh offline runtime, including later lines, then
verifies network-only loading with retention disabled and zero cached bytes.
Mobile checks preserve the source panel/text scale
ratio; they do not impose new responsive dialogue geometry. Screenshots still
need visual inspection. Serving acceptance covers Brotli, gzip, identity,
explicit encoding exclusions, byte lengths, range reads and revalidation.
