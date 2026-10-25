# moly-stage

The embedded browser application used by the content SDK. It owns its Bevy
application and schedule and calls the existing field, conversation, fixture,
camera, weather and audio systems directly. It does not depend on `moly-game`
or `moly-devtools`.

The page keeps control of the catalogue, locale and playback intents. Temporary
cast and furniture staging, restoration, and player-layout preview/exploration
use the same runtime owners as the game. The local server remains because those
owners consume its weather schedule, talk qualification and client data copies.
The stage has no durable save lease and no remote game server endpoint.

The application does not install harvesting, inventory, crafting, conversion,
missions, the layout editor or developer controls. Shared read-only UI state is
installed where the field's input qualification requires it. Proximity buttons
only offer stage actions; absent game screens cannot trap the field input.

Build both renderers with `node web/build-wasm.mjs --product stage`, or use
`--renderer webgpu` / `--renderer webgl2`. The binding output keeps the public
`moly-app` filename for existing SDK URL and cache compatibility; `build.json`
identifies the `moly-stage` crate and stage product. Release tooling checks the
stage ABI and rejects game persistence, director and control exports.

Use the same source revision, profile, toolchain and backend when comparing
size against `--product game`. A crate boundary alone does not demonstrate
smaller downloaded or decoded bytes.
