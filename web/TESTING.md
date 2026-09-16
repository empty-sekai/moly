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

`presentation.test.mjs` checks asset-path handling, content links, pagination,
renderer selection, storage ownership and region-switch link isolation.
`shell.test.mjs` loads the shipped
`index.html` in JSDOM and uses the real shell event handlers. It supplies a mock
WASM message boundary to check startup intent, sharing, related navigation,
focus ownership, Chinese IME input, stopping, empty states, persistent errors,
snapshot menus and settings in embedded mode. It also tests the iframe adapter's
origin/window checks and idempotent disposal.
Tests run sequentially to avoid contention and because each DOM fixture owns
the process globals for its lifetime.

`serve.test.mjs` starts one isolated development server on an OS-assigned port
with a temporary 10 MiB asset. It verifies HEAD, complete response hashes,
browser MIME types, missing-file responses and client cancellation recovery.
The child server and its verified temporary directory are cleaned up afterward;
the test does not use or stop a running development instance.

These tests cannot validate the renderer, actual playback, Bevy scheduling,
responsive layout or native browser focus behavior. Check those in the running
app with real data:

- At 1280×720, a wide desktop window and a narrow phone viewport, inspect the
  initial page, every catalogue category, selected transcript and empty state.
- Follow a link to a later catalogue page, switch character/availability/mode,
  share it, and open the copied link in another tab.
- Start and stop conversations, furniture stories and character interactions;
  switch content during playback, and verify that the original scene returns.
- Focus the canvas, then click/scroll the catalogue and transcript. DOM browsing
  must not move the player, zoom the camera or advance a conversation.
- During a character interaction, move the observer and rotate/zoom the camera.
  Enter first person, continue zooming, then stop and check restored position,
  camera projection and avatar visibility.
- If both regions are mounted, switch CN/JP and verify the region and content
  change together. Numeric content IDs and searches must not cross snapshots.
- Open game settings from the stage toolbar in standalone and embedded modes.
  Check that closing settings returns control to the scene.
- Compose Chinese text, use keyboard navigation, open help, and enter/exit
  focus and fullscreen modes. Verify scrolling and controls remain accessible.
- Try a missing asset snapshot and an unavailable renderer. Recovery actions
  must stay reachable and must not leave active audio behind.

Capture and inspect screenshots from the actual browser after these flows.
JSDOM assertions alone do not establish visual or playback acceptance.
