# Embedding the MYSEKAI stage

The version 2 stage is the iframe interface for a host that owns its catalogue,
navigation and UI. Rust owns content eligibility, playback and scene restoration.
The full-screen game site uses `game-boot.mjs` and the separate game contract.

Each backend's `build.json` names the shared UI font by SHA-256 and decoded
length. Loaders fetch and verify that font alongside the engine, then call
`install_ui_font` before starting. Deploy the build records and `pkg/fonts/`
with both engines. Direct WASM consumers must perform the same installation.

```js
import { mountMoly } from "/moly/releases/stage-example/embed.mjs";

const player = mountMoly(document.querySelector("#stage"), {
  view: "stage",
  src: "/moly/releases/stage-example/stage.html",
  assets: "/moly/snapshots/jp-example/assets/",
  snapshot: "jp-example",
  region: "jp",
  version: "6.8.1",
  locale: "ja-JP",
  theme: { mode: "dark", accent: "#66ccbb" },
  filters: { tab: "furniture", fixture: 157, mode: "independent" },
  content: "fixture:157",
  onSnapshot(snapshot) {
    // Render the authoritative state; do not reimplement eligibility here.
    console.log(snapshot.status.phase);
  },
  onError(error) {
    console.warn(error.code);
  },
});

// Lightweight updates never rebuild the app or implicitly stop its owner.
player.setLocale("en-US");
player.setTheme({ mode: "light", accent: "#66ccbb" });
player.browse({ tab: "activities", fixture: null });

// On an explicit user selection, preserve the complete opaque identity.
player.select("activity:notalk:1:1");
player.play("activity:notalk:1:1");

// Before a controlled route/source change, wait for restoration, then remove.
const restored = await player.close();
// false means the acknowledgment was interrupted/timed out, not success.
console.log({ restored });
```

Use only IDs verified in the selected snapshot; the example is not a guarantee
that a particular release contains a particular piece of content. A source
switch must clear numeric fixture/content context rather than guess a mapping.
`region` and game `version` are explicit, while `snapshot` qualifies a particular
published resource revision. UI locale is independent of source language.

`embed.d.ts` describes the typed interface. Stage messages have
`schemaVersion: 2`, an exact sender tag, and a bounded intention vocabulary:
`browse`, `select`, `play`, `stop`, `restore`, `close`, the `weather` and
`site` scene settings, sound, and input-focus control.
No arbitrary gameplay command, script, entity ID or camera transform is accepted.
The parent checks both the exact iframe window and the exact origin; the child
checks the exact parent and origin. Rust library commands remain an internal
schema-1 boundary, and the stage verifies the runtime region/version before
forwarding play. Applications should call the adapter instead of constructing
messages or URLs across unrelated pages.

For the production object store, pass the complete prefix explicitly. The
iframe and player-data traffic remain same-origin; only immutable engine and
resource objects are fetched from the configured S3-compatible prefix. There
is no implicit CDN or legacy OSS fallback:

```js
resourceBase: "https://assets.pjsk.moe/sekai-extra-assets/",
assets: "https://assets.pjsk.moe/sekai-extra-assets/snapshots/<region>-<gameVersion>-.../assets/",
```

Release objects use `releases/<id>/...`, snapshots use `snapshots/<id>/...`,
and the shared store uses `asset-store/...` below that prefix. `manifest.json`
is discovery metadata and is never treated as a cacheable runtime asset.

Call `mountMoly` only when the interactive feature is requested. Catalogue
browsing can use the separately published compact index/details without creating
an iframe or downloading WASM. A mounted stage fetches the engine and necessary
base pack at once and starts the scene when both are ready; it has no start
button of its own. If the browser has not yet allowed the page to play audio,
the engine's audio output starts suspended and resumes on the next user
activation inside the stage, or when `play()` / `preview()` is called
synchronously inside the host button's click (the adapter forwards that
activation to the same-origin stage). Do not await before these calls or create
another audio context. Theme, locale, selection and filters do not require
remounting. A second stage mount in the same host page is rejected
until the existing one is disposed.

The scene opens on `site` when the mount options name one (a site ID such as
`shore` or `first_floor`), otherwise on the runtime's default. `snapshot.site`
lists every site the runtime can switch to, in the source's order, with the
display name from the snapshot's master table, and the site the scene is on.
`setSite(id)` asks the runtime to move the live scene there. The runtime
refuses IDs it did not list and refuses a switch while an experience owns the
scene (`status.canStop` is true). The last site set through `setSite` is applied
again after a reload of the iframe, as the weather setting is.

A retry or WebGL fallback reloads the iframe realm, so two renderers/audio
contexts are never initialized within one realm. The handshake distinguishes a
new iframe document from a duplicate message. It preserves current browsing and
an unacknowledged user play request after a load failure; an already-started
performance is not silently resumed after a crash. Explicit WebGL2 selection
uses that backend; automatic fallback is used only when WebGPU capability is
unavailable, not to mislabel a failed source snapshot as a GPU problem.

`close()` is idempotent and concurrent callers share one restoration result.
`dispose()` is the synchronous last-resort teardown for detached/unloaded hosts;
it sends close, removes the frame and releases the renderer lease. It cannot
promise a completed restore acknowledgment during browser destruction. The
runtime never saves independent temporary layouts. Controlled host navigation
should prefer awaited `close()`; page destruction must still call `dispose()`.

## Versioned serving and optional resource retention

`release-artifact.mjs` publishes a browser release with both source-stamped
backends, separate catalogue transport and per-region resource identities. The
server must provide WASM as `application/wasm`, modules as JavaScript, proper
range/HEAD behavior and same-origin frame access. Release paths are immutable;
`manifest.json` is non-cacheable discovery metadata. Keep old versioned releases
while clients can still reference them. Do not replace files below an existing
immutable ID or package copyrighted source data into the application image.

Game resources remain external, read-only mounts. The publisher checks source
masterdata and descriptor fingerprints but does not hash every game binary.
Operational immutability of the mounted snapshot is therefore required. Native
runtime extraction and masterdata provenance remain separate for each region.

Use `split-gimmicks.mjs` before freezing a snapshot. It preserves each original
package's JSON numeric literals and generates a small source-qualified index.
The browser loads only definitions required by actually instantiated furniture,
while the original Rust compiler, activity admission and controller leases remain
authoritative. This is a transport projection, not a second interaction engine.

`prepare-browser-assets.mjs` can prepare gzip sidecars from a successful
empty-stage acceptance trace. It only touches additional transport files for
observed assets; it never downloads or rewrites original JSON/GLB data. Its
`browser-base.json` records measured initial bytes. The host automatically enables
retention and prepares this pack on entering interactions; ordinary database
pages still load no runtime. Content-specific models and voice are fetched on
use. Only the necessary pack and content already retained can work offline.

The optional module service worker at `/moly/cache-worker.mjs` must have scope
`/moly/`. The worker defaults to disabled; the interactions host enables it
before mounting the stage. Only immutable versioned
Moly requests are retained, with a 512 MiB budget, 128 MiB maximum object and
128 MiB total pending copy budget. Read/write/quota failures do not turn successful
online responses into gameplay failures. A scoped clear deletes only Moly
resource namespaces, not site settings, saved layouts or other applications'
caches. Mutable development asset mounts must use `no-cache` and must not be
retained as production immutable resources.

`preview(key)` displays the independent source tweet, preserving all source
characters and line breaks. Missing tweets remain unavailable. `play(key)`
engages the complete dialogue; engaging an active preview keeps its admitted
actor, fixture and scene. Both use the original authored balloon/talk-window
geometry, atlas textures, glyphs and typewriter owner at the source canvas scale.

Host content lists use 24 entries per page and URL/history navigation. A selection
preserves the viewport, while explicit pagination may move it. Web and browser
fullscreen resize the same iframe; do not portal, reparent or remount it.
Temporary player-data preview/explore/restore never replaces durable saved
layouts. The bridge bounds imported JSON to 32 MiB and never replays it as a URL
or persistent queued command.

Publication accepts only `moly-root-chara-head-v1` portrait sets. Export them with
the moly-root `tools/head-portraits` tool using its actual chara viewer renderer.
Every source unit needs one 512 x 512 transparent RGBA image made from head-only
geometry, plus matching PNG/model/rig hashes and zero crown/side clipping.
Host artwork combines every participant head with the actual furniture texture.

Each engine has decoded `.wasm`, lossless `.gz` and `.br` representations.
`downloadBytes` identifies Brotli bytes; `decodedBytes`, `gzipBytes` and
`brotliBytes` remain separate. Servers must negotiate `Accept-Encoding`, respect
explicit `q=0`, use representation-specific lengths/ETags, and preserve WASM MIME.
The default browser build uses the `wasm-size` profile.

Browser console output defaults to warnings and errors. For a diagnostic session,
add `log=info`, `log=debug`, or `log=trace` to the stage document URL; this setting
does not persist. Per-frame loading-indicator values require `log=trace`.

An `onError` value with `fatal: true` means the document can no longer accept
runtime commands. Its last playback snapshot is reported as `error`, and `close()`
disposes the iframe immediately with `false` (restoration was not acknowledged).
Retry must create a fresh document. Recoverable content errors still use the
runtime's ordinary restore path.
