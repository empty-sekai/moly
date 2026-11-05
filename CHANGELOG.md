# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.0.7](https://github.com/empty-sekai/moly/compare/v0.0.6...v0.0.7) - 2026-11-05

This is the last release; the repository is archived after it.

### Features

- **sequence:** Add the `moly-sequence` crate: a JSON sequence document with times in seconds, by line, by marker or relative to another clip, compiled and sampled for camera shots, actor moves and facings, motions, faces, looks, sounds, lines, captions, screen effects and events. `docs/sequence.md` describes the format.
- **sequence-player:** Play a sequence document in the game through `moly.sequence.load|play|pause|seek|speed|stop|status|schema` and the `sequence.play` and `sequence.stop` steps. While paused, `moly.sequence.frame` frees the camera and `moly.sequence.key` writes the live camera into the document.
- **ragdoll:** Add the `moly-ragdoll` crate: a character falls as a body of rigid parts and gets up where it came to rest. `moly.ragdoll.drop|recover|status|settings|schema` and the `npc.ragdoll` step drive it; `docs/ragdoll.md` describes it.
- **render:** Set the main light's shadow in the `[graphics]` section of `moly.toml` and change it at runtime through `moly.graphics.get` and `moly.graphics.set`: map resolution, coverage, distance and fade, receiver filter (`hard`, `tent`, `pcf` or `pcss`) and biases.
- **render:** Draw fixture parts whose material uses the character program.
- **devtools:** Record the window and the game's mix to MP4 through `moly.record.start|stop|status`; `docs/recording.md` describes it.
- **director:** `camera.talk_lens` takes `glide`, `push` and `clear`; the lens follows the speaker of the line on screen and fades out a fixture that blocks the view. `npc.play_talk` takes a fixture.
- **director:** `moly.director.schema` lists every registered step; a test keeps the schema, the registry and `docs/plugins.md` in agreement.
- **features:** A stage set service switches site, lays out the temporary set, casts and parks characters and restores everything afterwards; the content library's independent mode uses it.
- **world:** NPC bodies are handed out by token, so only the newest state that takes a body writes to it, and fixture timelines are layered over the body's own clips.

### Bug fixes

- **assets:** Resolve an empty UI prefab path to the document root, so the photo screen opens without a panic ([#28](https://github.com/empty-sekai/moly/issues/28)).
- **server-host:** Accept control port 0 and name the bound address in the ready line; the launcher, the MCP server and the balloon order check use it, so a game start no longer fails when the default control port is taken ([#26](https://github.com/empty-sekai/moly/issues/26)).
- **world:** Keep a state machine's decision pass from redrawing forever when an objective ends inside it.
- **world:** Keep characters taken by a host action out of NPC conversations and Rest scripts.
- **world:** Run NPC timers and movement on the scaled game clock, so time scale, pause and single steps apply to them.
- **world:** Keep emoticon particle velocity in simulation space, so stretched particles point along their motion.
- **render:** Shade characters on stored colour values, so the shade colours darken by their full amount.
- **render:** Sample the main-light shadow map with bilinear comparison, so shadow edges no longer step by whole texels.
- **act:** Frame the body instead of the root in the talk lens.
- **features:** Hide the action button with the clean view's HUD switch.
- **content-library:** Keep the scene when the browser closes.

### Engineering

- Run director and plugin steps through one action registry with leases on the hand and the camera; domain crates register their own steps.

### Documentation

- The READMEs list what works and the known limitations; the roadmap and the contribution guide are removed.
- Android is marked as unsupported; its build files are kept for reference only.

## [0.0.6](https://github.com/empty-sekai/moly/compare/v0.0.5...v0.0.6) - 2026-11-01

### Breaking changes

- **config:** The programs carry no network addresses of their own. The resource publication (`resources.base`), the master hosts (`masters.repository`, `masters.base`, `masters.mirror`), the per-region master repository names (`masters.repository_jp` … `masters.repository_en`), the player data API (`player_data.api`) and the server address (`server.url`) are read only from `moly.toml` or their environment variables. A feature whose address is not set is refused with a message that names the key; a master source that is not set is skipped.
- **masters:** The master repository source is an address template set by `masters.repository` (`MOLY_MASTER_REPOSITORY`); its `{repo}` path segment is replaced by the region's repository name (`masters.repository_<region>`). The earlier master repository variable is no longer read. The browser page reads `?master_repository=` and `?master_repository_<region>=`.
- **launcher:** The download address has no default; the launcher refuses a download until one is entered.
- **browser:** The page needs `?server=` to connect to its server, and the `/player-api/` path answers only when `MOLY_PLAYER_API` is set where the page is served.
- **android:** The package is built with `MOLY_RESOURCE_BASE` and `MOLY_SERVER_URL` set; a build without them stops at start with a message naming the variable.
- **config:** Developer switches are `[dev]` keys of `moly.toml`; `dev.check_systems` takes `true` or `false` (`1` or `0` in the environment) and any other value stops the start.

### Features

- **config:** Read the launcher, game, developer window and server settings from one `moly.toml` in the data directory (or the file `MOLY_CONFIG` names). Each key keeps its environment variable as an override; an unknown key or a value of the wrong kind stops the start with the key named. The launcher edits the file in place and keeps its comments, and imports an existing `launcher.json` on first start.
- **config:** Read the developer switches as `[dev]` keys and the display translation files as `[text]` keys of the same file. `docs/configuration.md` and `moly.example.toml` list every key.
- **director:** Add the `layout.free`, `layout.clear`, `layout.put`, `layout.put_many`, `layout.remove`, `view.set` and `companion.gather` steps; `docs/plugins.md` documents the Luau interface and every director step.
- **layout:** Add a per-site free layout mode and layout steps that save through the editor's own checks.
- **companion:** Let a companion gather a harvest object with the tool the player's harvest would pick; place builds on an exact cell and report the saved cell.
- **plugins:** Add the `moly.home-build` plugin (companions build queued furniture and react together) and the `moly.talk-shot` plugin.
- **view:** Add switches that hide the HUD and the player's body.
- **devtools:** Follow the speaker with a talk lens and show fixture action failures on screen.
- **mission:** Draw the mission list rows and the detail dialog's reward rows from their cell documents.
- **world:** Drive tween, dolly-cart and scale lanes from fixture clips.
- **cutscene:** Any feature can start a cut-scene with a cast through `cut_scene::start` and a `CutSceneCaller`.

### Bug fixes

- **library:** Disable the developer site-switch keys and the site tour while the content library holds the field's input
- **layout:** Name the failing cell or missing input in `moly.layout.apply` rejections through a `detail`, refuse a missing editor session, layout, grid or rule table with its own message instead of a layout code, and document the `cell` anchor's centered grid frame
- **harvest:** Follow the server's stamina in the player model, so stamina edited from the server panel is applied to the gauge and to harvest settlement
- **harvest:** Land gathered drops, delivery drops and the airplane box at the walk surface height of their landing point, so drops beside raised ground no longer float
- **world:** Simulate hair and skirt cloth on a character body worn by the player
- **masters:** Read the convert machine slots and the convert pass rate from their published single-row documents (`mysekaiConvertFixtureSlot`, `mysekaiColorfulPass`) and key `configs` rows by `configKey`, so the convert tables are no longer reported missing
- **render:** Switch the eye textures by the eye pattern's texture index, draw the eye overlay as a blended second layer, keep eyebrows from showing through what hides a character, and keep pool and bath water from being covered by the ground.
- **render:** Accept particle trails that do not die with their particles; keep balloon layers ordered while a balloon animates.
- **companion:** Start walks from the nearest navigation cell without jumping, follow the path to the target, and depart again when the navigation is rebuilt.
- **walk-face:** Rebuild an obstacle's carve only when its state changes, so walkers no longer restart their routes every frame.
- **world:** Play fixture timelines with empty or single-key animation clips; bind character-body cloth against the imported rest pose.
- **features:** Offer the action button for fixtures without a timeline on the JP region.
- **inventory:** Read the fixture layout cost from the fixture master.
- **ui:** Hold the screen manager until the screen data is read.
- **talk:** Read string speakers and targets as script integers.
- **director:** End `layout.place` on the tile when `decide` is false.
- **launcher:** Cancelling a resource download stops each transfer within the chunk it is reading instead of after the whole file, a request whose server sends no response times out after 60 seconds and is retried, and the cancel button shows that the cancel is under way.
- **launcher:** A resource download opens at most as many connections as the parallel download setting: package manifests no longer use twice that number, and each transfer reuses its connection instead of opening a new one per file.

### Performance

- Keep road materials across layout reloads.
- Leave the settings panel camera idle while the panel is closed; read back the shadow and silhouette probes on native only on request.
- Share the objective face between characters and stop rescanning settled editor icons.
- Bake the walk field inside the site's navigation volume, cut per-frame CPU work of particle draws, and write the wipe material only when its values change.

### Engineering

- Update to Bevy 0.20.
- Split the gameplay domains into their own crates: `moly-convert`, `moly-mission`, `moly-photo`, `moly-fixture-edit`, `moly-delivery`, `moly-harvest`, `moly-content-library` and `moly-cutscene`, each installing into the `moly-features` hub.
- Check crate layers and per-lint warning counts in CI, share lint settings across the workspace, build the API docs with warnings denied, and test the whole workspace.

### Documentation

- The README opens with a notice to read before use: moly is a runtime compatible with MYSEKAI for study and local single-player use only, with no game resources, extraction tools, original client code or online service, and a server that serves only moly's client.
- `docs/METHOD.md` describes how the original client is understood and its behaviour reimplemented.

## [0.0.5](https://github.com/empty-sekai/moly/compare/v0.0.4...v0.0.5) - 2026-10-28

### Features

- **plugins:** Add native Luau plugins with manifests, opt-in profiles, dependency and conflict checks, sandbox limits, events, tasks and per-plugin saved data. Include source examples for scripted companions that walk, build furniture, react and use what they built.
- **JP 7.0.0:** Run hierarchical NPC states and cross-group conversations with the source actors, navigation, animations and scene ownership.
- **editor:** Restore the road and weather tabs, source icons and controls. Roads support placement planning, confirmation, cancellation, selection, movement and returning pieces to inventory; weather edits share the layout save/discard transaction.
- **harvest:** Add the JP 7.0.0 bulk hammer: hold-to-charge, cancellation, target settings, the source hammer and timeline, gauge, effects, sounds, stamina/tool settlement and rewards.
- **stage:** Split the embedded stage into the `moly-stage` crate and publish separate stage/game WASM packages, each with WebGPU and WebGL2 builds.
- **text:** Add independent, versioned text overlays. Chinese display text can use matching official CN lines while retaining JP scenes, voices and performances; unmatched lines keep the original text. Translation updates do not replace the resource snapshot.
- **fonts:** Ship the open-source Resource Han Rounded font and its SIL Open Font License with native and browser packages.

### Fixes and performance

- Restore list overflow scrolling and scrollbar dragging, source weather cells, and editor input ownership so UI gestures do not also move the camera.
- Preserve road inventory and UIDs across edits; prevent the road release frame from also panning the camera.
- Clear the bulk-harvest gate after treasure delivery, suppress the unrelated pass-expired balloon, and bound its fade lifetime.
- Restore gameplay/UI audio delivery and reduce repeated tool-notice preparation; retain browser audio scheduling recovery after a main-thread stall.
- Bake the walk field off the main thread, honor immutable master-source pins, and preserve NPC navigation and temporary scene restoration.
- Upgrade the engine to Bevy 0.19.1 and backport the empty-mesh copy guard; place weather post-processing before anti-aliasing and load unnamed glTF scene roots.

### Distribution

- Native game packages include the game, launcher and server for Windows x64, Linux x64 and macOS Apple Silicon; server-only archives remain available.
- Browser archives are now `moly-0.0.5-web-stage.tar.gz` and `moly-0.0.5-web-game.tar.gz`, replacing the single web archive. Game resources remain separately supplied.
- The community translation upload service is a separate project and is not included in this release. Native plugins are not exposed by the embedded stage.

## [0.0.4](https://github.com/empty-sekai/moly/compare/v0.0.3...v0.0.4) - 2026-10-24

### Features

- **control:** Add moly.camera.free and moly.camera.reset
- **mcp:** Add the layout_place_many tool
- **net:** Add the protocol, server session and client link crates
- **server:** Add the moly-server native host
- **link:** Keep trying to reach a server that is still starting
- **launcher:** Start the game's server before the game
- **app:** Connect the native game to its server
- **server:** Run the server model in moly-server and answer protocol calls
- **link:** Answer every feature call through the server process
- **mcp:** Expose the server control port and server_rpc
- **server:** Share the master table loader and serve the npc panel
- **director:** Wait for entry control and refuse unreachable walk goals
- **layout:** Add the moly-layout crate skeleton
- **craft:** Show the fixture bonus, rank-up, new site and status dialogs
- **bgm:** Add the change-vocal and filter dialogs to the record screen
- **layout:** Apply batched placement ops over moly.layout.apply
- **sitemap:** Expose site icons as tap targets in ui views
- **bgm-select:** Show the search, sort, view toggle and empty state
- **ui:** Add stamina, weather and bgm dialogs; report system errors at startup
- **assets:** Read asset packs over HTTP on native targets with a local cache
- **server-host:** Expose the server as a library that binds and serves
- **craft:** Carry the blueprint term on craft requests and in user data
- **ui:** Number dialog types as the 7.0.0 client does and name them at call sites
- **assets:** Read master tables as the JP set plus each enabled region's own rows
- **android:** Package the game and its server as a native Android app
- **npc:** Add the 7.0.0 NPC controller phases beside the current decision loop
- **npc:** Add the controller's upper and lower bases, plans and decider rows
- **server:** Serve the convert machine and model the mission sheets
- **npc:** Run the controller's upper states and lower dispatch
- **server:** Normal mission rewards, convert and the home mission panel
- **net:** Send the binary protocol over HTTP requests
- **features:** Add the mission and convert furniture screens
- **server:** Close administration and validate user ids
- **convert:** Add the convert dialogs
- **link:** Connect browsers and Android to the public server
- **link:** Let the browser stage play without a server
- **server:** Answer one request through the hub
- **link:** Host the server inside the browser stage
- **convert:** Receive converted resources and play reward lotteries
- **store:** Add locations, relocation, upgrades and program updates
- **launcher:** Manage folders and update the programs

### Bug fixes

- **site-expansion:** Return the room to the normal state and camera after a level release
- **server-panel:** Clamp invalid starter texture ids and log a refused starter layout
- **assets:** Read HTTP assets and remote tables inside a web worker
- **walk:** Bake collider surfaces as height spans so raised decks are walkable
- **director:** Reach the craft apply button and close follow-up dialogs
- **server:** Face the starter gate toward the site
- **inventory:** Create the catalogue state when the screen loads
- **game:** Order the unsaved-layout flag after the editor commands only
- **web:** Time the walk-face bake with the platform clock
- **stage:** Load the empty stage scene without a server
- **stage:** Resolve the empty level and draw the talk window
- **stage:** Publish preview ownership before balloon cleanup
- **server:** Name the save file when a stored document is refused
- **userdata:** Keep the rank when two replies arrive in the same frame
- **player:** Map the auto-move destination with the agent query box
- **server:** Send the party rows and materials with the delivery reply

### Performance

- **walk:** Skip full-grid passes the span bake does not need

### Refactoring

- **protocol:** Move the user data rows, requests and replies into moly-protocol
- **game:** Keep the game's copies of user data in a userdata module
- Share the server's wire types and master tables through their own crates
- **game:** Read every server value through the link
- **masters:** Load harvest catalog tables through the shared loader
- **masters:** Serve fixture, tool and record rows from the shared loader
- **layout:** Move the tile rules and the put resolver into moly-layout
- **talk:** Draw the talk window as a prefab view
- **game:** Move shared types into vocabulary modules
- **game:** Invert the remaining world-to-feature references
- **game:** Keep each module's files beside the module
- **game:** Split moly-game into one crate per layer

### Documentation

- Describe the server process, android, master sources and crates
- **roadmap:** Describe the HTTP transport and the Android status
- Restructure the README and move details into topic documents
- Drop the browser deployment link from the quick start
- Split the README into topic documents and rewrite the roadmap

### Tests

- **game:** Census of every fixture's walkable area

### Build

- Package and start moly-server wherever the game is shipped or launched
- **android:** Use Kotlin, NDK r30 and rustls web roots for native HTTPS

### CI

- Cache native builds and add server, linux and macos packages

### Miscellaneous

- **web:** Align the runtime limits table

### Reverts

- Restore conversion resource handling

## [0.0.3](https://github.com/empty-sekai/moly/compare/v0.0.2...v0.0.3) - 2026-10-17

### Features

- **harvest:** Generate harvest maps from built-in spawn rules

### Bug fixes

- **harvest:** Place harvest objects at the navigation surface height

### CI

- Generate the changelog and release notes with git-cliff

## [0.0.2](https://github.com/empty-sekai/moly/compare/v0.0.1...v0.0.2) - 2026-10-16

### Features

- **mcp:** Add layout tools for placing furniture
- Stagger room visitors and add agent time, events and observations
- **settings:** Add a button that raises the home to its largest size
- **server:** Add an every-authored-level site release policy
- **director:** Answer the learn dialog, place layouts and deliver from scripts
- **app:** Hide the native window on request
- **mcp:** Add a reference agent, tasks, replay and batch runs

### Bug fixes

- **fixture:** Keep fixture sources loaded across a layout save
- Show the save confirmation and invalid-layout dialogs
- **joystick:** Release the stick while the window has no drawable area
- **control:** Refuse unknown parameters of moly methods

### Refactoring

- Add the event bus, input claims and run mode seams
- Move shared seams and settings into moly-core

### Documentation

- Add extension directions to the roadmap
- Plan a single resource set for all regions
- Add free stacking and vertical movement to the roadmap

### Other changes

- Add moly.layout.view and moly.layout.command

## [0.0.1](https://github.com/empty-sekai/moly/releases/tag/v0.0.1) - 2026-10-16

### Features

- **bgm_select:** Add the BGM select screen behind the record player
- Add the acquisition dialogs and the harvest HUD
- **ui:** Port the screen animations, the site move slot and the reward list dialog
- **particles:** Step sub-emitter targets, shape scaling mode 2 and object passes
- **font:** Cover the BGM select soundtrack titles
- **server_panel:** Hold a harvest site's phenomenon until the next move
- Add the UI particle programs, the birthday cut-scenes and the craft screen
- **ui:** Read the shared root's palette and tween defaults when it carries them
- **particles:** Take float3 positions and float4 texture coordinates in particle streams
- Evaluate sub-emitter rates from authored curves and order the web prewarm
- Dither home obstacle objects nearer the camera than the player
- Show the birthday party dialogs through a shared common dialog layer
- Admit ForceOverLifetime on sub-emitter targets
- Run the birthday party after its cut-scene and restore the roster
- Play the birthday party's talks and its waiting objective
- Admit world-space and curve velocity on sub-emitter targets
- Build particle velocity and force columns as the engine does
- Report the inputs that hold nine per-frame systems
- **director:** Add scripted orchestration of the player and characters
- **director:** Turn characters, show emoticons and read action states
- **particle:** Run cache-following curve laws on sub-emitter targets
- **particle:** Bind built-in default textures to unassigned slots
- **director:** Play fixture activities and show tweets on characters
- **director:** Drag and zoom the camera from scripts
- **director:** Edit the server document from scripts
- **particle:** Mirror billboard texture corners by the renderer flip
- **particle:** Admit two-curve rotation over lifetime on sub-emitter targets
- **director:** Ask a character to open a named talk
- **director:** Tap UI elements by name and wait on them
- **fixture:** Play gimmick ShowFixtureEffect and HideFixtureEffect
- **particle:** Apply _ZOffset to site particles
- Add a control surface, a control panel and worn player bodies
- **tools:** Add an MCP server that drives the game
- **store:** Add the local resource store crate
- **app:** Run the game when the native program starts
- **launcher:** Add a desktop launcher

### Bug fixes

- **talk:** Compare phenomenon conditions with the environment's current phenomenon
- Handle malformed master rows in consumers
- Derive furniture availability only while the library reads it
- Draw each screen fade cover with its own camera only
- Read depth textures through a nearest sampler
- Keep the collider order for weather effects after site entry
- Give toggle, dropdown and input field clicks the empty cue name
- Take no new stick press under a dialog
- Let NPCs decide when fixtures reach past the floor grid
- Keep the keyboard from moving the player under a dialog
- Keep soap bubble collisions running while the player turns
- **room-door:** Draw the door's Basic parts without vertex colours
- **gate:** Play visitor appearances one after another
- **particle:** Run source particle programs under reversed-Z conventions
- **site-move:** Refuse a cannon move whose destination cannot load
- **room:** Cut the doorway out of the room wall with the replayed door clip
- **npc:** Retire the balloons of a character suspended away from the loaded site
- **entry:** Stream the fixture catalogue on native too
- **clock:** Read the device time zone on native too

### Performance

- Switch off overlay cameras that have nothing to draw
- Keep static shadow casters in a separate depth map
- Cut per-frame CPU work in rendering and UI
- Cut repeated work when the weather changes
- Keep recently released particle assets and their pipelines
- Upload source particle vertices in one write per frame
- **particle:** Leave idle particle meshes untouched
- **site:** Key dithered material copies by threshold level
- **particle:** Skip draw comparison for unchanged source particles
- **web:** Grow the wasm heap geometrically

### Refactoring

- Remove the unused single-fixture talk qualification path
- **web:** Remove the stage weather and site controls
- Group the workspace crates by layer

### Documentation

- Remove the remaining preview panel labels

### CI

- Check whitespace against complete change history

### Miscellaneous

- Format the maintained browser modules
- Request eight cores in the workspace configuration
- Keep the workspace alive for a full day
- **log:** Demote per-frame trace lines to debug

### Reverts

- Prewarm base resources with eight bounded download workers

### Other changes

- Establish the public source baseline
- Add player data import and improve runtime reliability
- Handle edit candidates and bound concurrent reads
- Update to the current development tree

The tree of this commit equals the development tree at the time of the
update. It adds ROADMAP.md, which lists the scope of the current version and
later work.
- Prewarm base resources with eight bounded download workers
- Load the inventory screen documents as optional
- Resolve tables a region's hosts do not publish without a request
- Draw a wording key the root lacks instead of panicking
- Add Latin, JIS X 0208 and GB 2312 to the UI subset
- Look wordings up by the region's rule
- Read the harvest, option and learn dialog wordings through the lookup
- Embed the UI font once
- Skip contents fields the region's class lacks
- Check only the wordings the region's page writes
- Stream the controller and house records on the browser game page
- Take the checked-in slice on the browser without a request
- Stop repeating verified work on packed reads
- Prewarm packed base resources without digesting them
- Name the JP and CN clients instead of build numbers
- Bound master reads by their first byte and demote a base that times out
- Prewarm base resources with eight bounded download workers
- Refuse a package's sequence export by name instead of panicking
- Keep the harvest tools in the server document
- Read the weather panel from the phenomenon master
- Order cached birth edges by clone hierarchy
- Keep menus usable when rank data is unavailable
- Complete external font delivery
- Remove the unused rank panic path
- Remove the retired content preview panel
- Reuse particle packing and upload buffers
- Limit source metadata to portable identities
- Use serialized collider references for action bounds
- Retain only the stage and game startup exports
- Decouple the game profile from stage catalogs
- Add the Tool and LimitedCraft lists to the craft screen
- List the owned tools in the Tool list
- Name the blocker again when it changes or stays
- Show the selected tool's thumbnail
- Report the site glTF load states while scenes expand
- Read native remote files off the asset task pool with bounded waits
- Log the render pipelines that become ready each frame
- Bake the walk field inside the site's navigation volume
- Cut per-frame CPU work of source particle draws
- Write the wipe material only when its values change

<!-- generated by git-cliff -->
