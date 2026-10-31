# Project structure

| Directory | Responsibility |
|---|---|
| `crates/base/moly-law` | Engine-independent behaviour and calculation logic |
| `crates/base/moly-store` | Local resource library: data directory, CDN release manifest, resource download and verification, install records |
| `crates/engine/moly-assets` | Resource directory, resource packs and data loading |
| `crates/engine/moly-perf` | Per-frame performance counters |
| `crates/game/moly-core` | Types, markers and system ordering shared by the game layers |
| `crates/game/moly-render` | Rendering: materials, shaders and drawing |
| `crates/game/moly-audio` | Background music and sound effects |
| `crates/game/moly-masters` | Master table loading and typing |
| `crates/game/moly-world` | Areas, furniture, walkable surfaces and character behaviour |
| `crates/game/moly-ui` | Prefab UI, UI management and common dialogs |
| `crates/game/moly-features` | Gameplay and UI for gathering, crafting, delivery and other features |
| `crates/game/moly-convert` | Conversion: the machine's slots, the items it converts and their screen and dialogs |
| `crates/game/moly-mission` | Missions: their screen, list cells, detail and list dialogs and the reward receive |
| `crates/game/moly-photo` | Photo mode: field capture, frame selection and saving the shot to a local file |
| `crates/game/moly-fixture-edit` | Layout editor: placing and laying out fixtures, roads and environments, with its UI and its camera |
| `crates/game/moly-delivery` | Delivery: the birthday party delivery site, its flow, drops, honor rewards and delivery screen |
| `crates/game/moly-harvest` | Harvest: the harvest sites, gathering with tools, drops and pickup, stamina, bulk harvest and the harvest tool material |
| `crates/game/moly-devtools` | Control port, director scripts and developer panel |
| `crates/game/moly-game` | Assembles the layers above into the game |
| `crates/net/moly-protocol` | Request, response and push message types between the game and the server |
| `crates/net/moly-link` | Server connection on the game side |
| `crates/net/moly-layout` | Furniture layout documents |
| `crates/server/moly-server` | Server: player documents, rules and request handling |
| `crates/app/moly-app` | Game program: native and browser entry points |
| `crates/app/moly-server-host` | Server program `moly-server` |
| `crates/app/moly-launcher` | Launcher: downloads resources, handles settings, and starts the server and the game (Tauri, with its pages in `ui/`) |
| `crates/app/moly-android` | Android native library: game entry point that connects to a separately running server |
| `crates/vendor/cpal` | CPAL, with one fix for web audio |
| `android` | Android project (Gradle) |
| `web` | Browser pages and build tools |
| `tools` | Checks for versions, repository boundaries and render order |
| `tools/mcp` | MCP server |
