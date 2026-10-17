# Changelog

All notable changes to this project are listed here. Versions follow [Semantic Versioning](https://semver.org/).

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
