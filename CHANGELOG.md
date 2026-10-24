# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org/).

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
