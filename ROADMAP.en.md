# Roadmap

[中文](ROADMAP.md) | English

This file lists what is done, the release in progress and later plans. It is updated with each
release. Known issues are listed in [Issues](https://github.com/empty-sekai/moly/issues).

moly has two parts:

- **Original game**: client behaviour is compatible with the original game. Values decided by
  the server are gathered in one editable simulation panel, with no network connection. The
  browser build runs the original game only.
- **Plugins**: gameplay the original game does not have is built as plugins. Plugins are off by
  default and run in the native program. With every plugin disabled, the game is the original.

## Done

**Regions**: the master data uses the Japanese server as its base and merges each region's
exclusive rows; the Japanese server is enabled.

### Sites and movement
- The flow and banner for entering MySekai.
- Cannons and doors move between sites; doors can be used to change scenes; the map selection screen.
- Indoor rooms (indoor shell and wall culling); several sites stay loaded.
- Player characters, footsteps and expressions.

### Harvesting
- Obstacles, gathering particles and holding limits.
- Notices and dialogs for obtained items.
- Gathering HUD: current tool, action icons, a hint bubble when a tool is missing, and the stamina bar.
- Hammer gathering for clearing a map in one action: hold to charge, cancel, target settings,
  effects and sound, and the settlement of costs and rewards.

### Delivery
- The full delivery flow, with honor rewards and total rewards.

### Crafting and conversion
- The crafting screen and its result dialog, and the server replies for crafting and blueprints.
- The server rules for furniture conversion.

### Furniture editing
- The inventory screen, placing furniture from the inventory, and layout editing for floor furniture.
- The road and tile page: choosing a style, planning the paving, confirm and cancel, moving the
  selection, taking back and saving; the inventory and placed objects update in the same layout
  transaction.

### Cutscenes
- Birthday parties: cutscenes, the three conversations and birthday voice lines during the party,
  the closing and the site restoration.

### NPCs and dialogue
- Pathfinding, conversations after pairing, overhead tweet bubbles, idle action decisions,
  autonomous behaviour and furniture interaction.
- Target positions chosen from the original walkable cells, and the behaviour route while
  waiting to talk.
- The layered NPC state machine of the Japanese 7.0.0 version, and conversations between
  characters of different units.

### Weather and particles
- The weather phenomena pipeline, end to end: host lifecycle, sub-emitters, curves, raindrop quads
  and more.
- The weather page: choosing an environment from unlocked content, saved or discarded together
  with the layout.
- UI particle effects.

### Audio
- BGM, sound effects, voice and ambient sound; record BGM and the BGM selection screen.

### UI and settings
- Menus, information pages and settings, the phenomenon learning dialog, and screen transitions.
- Checks for new versions and updates the game and the launcher itself in one click, downloading
  the full package.

### Server and platforms
- The server runs as a separate process, stores the player document and answers the game's
  requests. The game sends it binary-encoded requests over HTTP.
- Launcher: chooses, downloads and verifies a resource version, and starts the server and the game.
- Packages for Windows x64, Linux x64 and macOS arm64, and a separate server package.
  The Linux and macOS packages have not been tested on real machines; feedback is welcome.
- Stable launcher locations: the home folder, data, logs, program versions and resources can each
  be changed, and existing content is moved along; settings and saves can be imported from the
  folder of an earlier version.
- When the server refuses a save, the game shows the error and the save location instead of
  staying on the entry screen.

### Data and regions
- Master tables are read at run time from the public per-region master repositories, with a
  mirror as a fallback.

**Director scripts and the agent interface**
- JSON scripts that orchestrate players, characters, the camera, weather and the server simulation panel.
- MCP server: lets AI clients start and drive native or browser instances, read state, run scripts
  and capture frames.

**Plugin layer**
- The native program loads Luau plugins. Each plugin has a `plugin.toml` manifest: name, version,
  the moly versions it supports, dependencies and conflicts. Which plugins are enabled is saved in
  the personal profile; no plugin is enabled by default.
- Replaceable parts of the original logic are interfaces. A plugin can replace a provider as a
  whole or for selected characters only; when two plugins provide the same interface and the
  profile does not choose one, the game reports it by name at startup and does not start.
- Plugins can read scene state, subscribe to events, schedule asynchronous tasks, call game
  actions and keep their own data.
- Each plugin has its own sandbox, a memory limit and a per-frame execution budget.
- Example plugins: a greeting example, and a scripted companion that walks, builds furniture with
  a hammer, turns to react to the player and uses the furniture it has just built.

**Browser stage and text**
- The embedded stage is a separate program, `moly-stage`; the full game is provided separately.
  The browser packages each contain WebGPU and WebGL2 builds.
- When Chinese is selected in the browser, matching official text from the Chinese server can
  replace the Japanese display text; without a matching translation the Japanese text is shown.
  The text pack, font pack and resource snapshot are versioned separately.
- The open-source Resource Han Rounded font is used.

**Engineering**
- Crates per gameplay domain: harvest, delivery, conversion, missions, photos and furniture
  editing are separate crates that install into the `moly-features` hub.
- The crate dependency levels are guarded by a check script.
- One shared lint configuration; clippy warnings have a per-crate baseline that may only go down.
- Test coverage across the whole workspace.

## Next release (0.0.6): one configuration file and crate boundaries

**Configuration**
- One file, `moly.toml`, configures the launcher, the game, the developer window and the server.
  Each key can also be set through its environment variable. An unknown key or a value of the
  wrong kind stops the start with the key named. `moly.example.toml` and
  [Configuration](docs/configuration.md) list every key.
- The programs carry no network addresses of their own: the resource publication, the master
  repositories, the player data API and the shared server are set in the file. A feature whose
  address is not set is refused with a message that names the key.

**Engineering**
- The content library and the cut-scenes are crates of their own. The hub provides the interfaces
  they register through: input claims per channel, and a cut-scene player through which any
  caller starts a cut-scene with a cast.
- Built on Bevy 0.20.

## After 0.0.6: remaining plugin layer and AI player

**Plugin layer**
- The game generates type definitions for the plugin API from its own type registry, so editors
  can type-check plugins.
- Human input always takes precedence over plugin control.
- Built-in plugins: the MCP agent interface, SD character switching and the AI player, each of
  which can be turned on or off.

**AI player**
- A second player with an SD character body. When no human is controlling it, rules or a language
  model drive it: it gathers materials, crafts furniture and places it.
- Each character acts according to its own persona, starting with Emu Otori.
- A human can take over at any time.

## After 0.0.6: story staging

Story scenes are staged again with 3D sets, SD characters and a camera: lines, voices and timing
come from the script, while sets, blocking and camera work are written scene by scene. It is a
plugin; the original mode does not change. Four steps, in dependency order:

**1. Set and video output**
- A separate shooting scene: original furniture and custom models can be placed at any position,
  rotation and scale, and the site terrain can be hidden. A set is saved as a file and reloads
  when the file changes.
- Custom lights: point, spot and area lights with colour, intensity and shadows; a glowing screen
  lights its surroundings.
- The camera follows a shot list shot by shot: position, look-at target, field of view, depth of
  field, easing and handheld shake; hard cuts and transitions.
- Frames are rendered at a fixed frame rate and muxed with the audio into a video; the same input
  gives the same result.

**2. Actors, lines and sound**
- SD characters follow the shot list: they walk to marks, turn, look at targets, play motions and
  expressions, and move their mouths with the voice; poses such as sitting, resting on a desk and
  typing.
- Dialogue box, speaker name, full-screen text and location captions; voices, BGM and sound
  effects play and fade on the timeline.

**3. Custom assets**
- Custom glTF models, textures and motions can be imported, with materials that match the
  original toon shading; custom assets live in their own directory outside the resource directory.
- New motions on the SD skeleton.
- Runtime images rendered onto a model's screen surface, for example a chat window or a flickering
  screen.

**4. Custom scenes and weather**
- Custom weather presets: sky, fog, lighting and floating particles; the original weather does
  not change.
- Screen effects: fade to white or black, flicker, colour grading, vignette and depth of field.
- Places that have no 3D model in the original are built from custom assets and custom weather.

## Later plugins (unscheduled, in dependency order)

**1. Agent interface**
- Semantic observation: sites and cells, gatherables, inventory, recipes and blueprints, furniture
  and layout, character state and relationships, weather and time, as structured data rather than
  screenshots.
- Semantic actions: walk to, gather, craft, deliver, place and edit layouts, talk to characters,
  change outfits and switch sites. Each action reports its result or a named refusal reason and
  follows the same rules as the UI.
- Event subscriptions, time control (pause, single step, speed, fixed random seed), headless runs
  and several instances in parallel.
- A reference agent and task set, with run records and replays.

**2. Taking over character behaviour**
- Each character's decisions can be given to the original logic, a director script, a plugin or
  an external agent, either for target selection only (movement and actions still follow the
  original rules) or as a whole.
- Character-view observation, an action set that covers the original behaviour and can be
  extended with custom actions, and memory and schedules saved per character.

**3. Talking with characters**
- Free-text conversation through a language model, with expressions, actions and bubbles that
  follow the replies; a conversation can lead to gathering, crafting, decorating or visiting
  another site together.
- The model can be a local model or the user's own service, configured by the user.

**4. Character homes and a world that keeps moving**
- Each character has its own home site, which it decorates and changes over time.
- The world clock keeps running; while the player is away, routines, visits and home changes are
  caught up by time.

**5. Lifted limits**
- Optionally remove or raise limits on furniture count, holding, site cells and site level;
  larger sites, custom and stitched maps, more indoor floors.
- The rendering and simulation performance this needs: instanced drawing, distance-based levels
  of detail and chunked loading.

**6. Free stacking and vertical movement**
- Furniture can be stacked on any furniture and placed raised as a whole.
- Players and characters can walk onto stacked furniture and steps, jump and drop; pathfinding
  includes the surfaces that can be stood on.

## Original content (unscheduled)

### Sites and movement
- The full visitor flow.

### Harvesting
- The gathering tool selector.

### Furniture editing
- Wall and stacking editing.
- Room upgrades.

### Content library
- Inventory: sorting, filters, category tabs, dismantling and 3D preview.
- Shop and exchange.

### Missions
- Missions.

### Photo mode
- Photo mode.

### NPCs and dialogue
- Outfit changes.

### Weather and particles
- Rendering details such as soft depth, some light effects and orbital particles.

### Audio
- Audio details.

### UI and settings
- The secondary dialogs of each screen.

### Data and regions
- Enable other regions once the resource library provides what their exclusive content needs.
- UI language switching (Japanese, Simplified Chinese, Traditional Chinese, Korean, English),
  following the system language by default; each language replaces only the displayed text and
  shares the same resources.

## Graphics (unscheduled)

**Shadows**
- Cascaded shadow maps: fine near the camera and coarser with distance, aligned to texels so
  edges do not shimmer while moving, with bias that scales with the map's resolution.
- Smooth soft-shadow filtering in place of the single-texel comparison, so edges lose their
  stair-stepping and small shapes stay readable.

**Post effects**
- Screen-space ambient occlusion and contact shadows, each switchable in the graphics panel.

**Graphics panel**
- A settings panel whose changes take effect immediately, without a restart: shadow quality,
  anti-aliasing, render scale and further post-processing effects, with presets and per-option
  control. Choices are saved in the personal profile.

## Engineering

- Move the materials' mesh, depth and shadow passes onto the engine's standard interfaces (depth
  and normal prepass, the engine's cascaded shadow maps), keeping each material's shading
  arithmetic as it is.
- Move the remaining values decided by the server into the simulation panel.
- Clear the remaining clippy warnings.
- One code format across the workspace.
