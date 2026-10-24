# Roadmap

[中文](ROADMAP.md) | English

This file lists what is done, the release in progress and later plans. It is updated with each
release. Known issues are listed in [Issues](https://github.com/empty-sekai/moly/issues).

moly has two parts:

- **Original game**: client behaviour is recreated to match the original game. Values decided by
  the server are gathered in one editable simulation panel, with no network connection. The
  browser build runs the original game only.
- **Plugins**: gameplay the original game does not have is built as plugins. Plugins are off by
  default and run in the native program. With every plugin disabled, the game is the original.

## Done

**Regions**: the master data uses the Japanese server as its base and merges each region's
exclusive rows; the Japanese server is enabled.

**Server and platforms**
- The server runs as a separate process, stores the player document and answers the game's
  requests. The game sends it binary-encoded requests over HTTP.
- Launcher: chooses, downloads and verifies a resource version, and starts the server and the game.

**Entry and movement**
- The flow and banner for entering MySekai.
- Cannons and doors move between sites; doors can be used to change scenes; the map selection screen.
- Indoor rooms (indoor shell and wall culling); several sites stay loaded.
- Player characters, footsteps and expressions.

**Gathering**
- Obstacles, gathering particles and holding limits.
- Notices and dialogs for obtained items.
- Gathering HUD: current tool, action icons, a hint bubble when a tool is missing, and the stamina bar.

**Delivery, crafting and placement**
- The full delivery flow, with honor rewards and total rewards.
- The crafting screen and its result dialog, and the server replies for crafting and blueprints.
- The server rules for furniture conversion.
- The inventory screen, placing furniture from the inventory, and layout editing for floor furniture.

**NPCs**
- Pathfinding, conversations after pairing, overhead tweet bubbles, idle action decisions,
  autonomous behaviour and furniture interaction.
- Target positions chosen from the original walkable cells, and the behaviour route while
  waiting to talk.

**Weather**
- The weather phenomena pipeline, end to end: host lifecycle, sub-emitters, curves, raindrop quads
  and more.

**Audio and presentation**
- BGM, sound effects, voice and ambient sound; record BGM and the BGM selection screen.
- Birthday parties: cutscenes, the three conversations and birthday voice lines during the party,
  the closing and the site restoration.
- UI particle effects.

**UI**
- Menus, information pages and settings, the phenomenon learning dialog, and screen transitions.

**Director scripts and the agent interface**
- JSON scripts that orchestrate players, characters, the camera, weather and the server simulation panel.
- MCP server: lets AI clients start and drive native or browser instances, read state, run scripts
  and capture frames.

**Data**
- Master tables are read at run time from the public per-region master repositories, with a
  mirror as a fallback.

## 0.0.4 (in progress)

**Releases**
- Packages for Windows x64, Linux x64 and macOS arm64, and a separate server package.
  The Linux and macOS packages have not been tested on real machines; feedback is welcome.

**Launcher**
- Stable locations: the home folder, data, logs, program versions and resources can each be
  changed, and existing content is moved along.
- Imports settings and saves from the folder of an earlier version.
- Checks for new versions and updates the game and the launcher itself in one click, downloading
  the full package.

**Game**
- When the server refuses a save, the game shows the error and the save location instead of
  staying on the entry screen.

## 0.0.5: plugin layer and AI player

**Plugin layer**
- Each plugin has a manifest: name, version, the moly versions it supports, dependencies and
  conflicts. Which plugins are enabled, and their settings, are saved per profile and managed in
  the launcher.
- Replaceable parts of the original logic (NPC decisions, player control and others) become
  interfaces whose default provider is the original implementation. A plugin can replace a
  provider as a whole or for selected characters only.
- Plugins can read game state, subscribe to events and change the world through the same actions
  as the UI.
- Conflicts are reported by name at startup: when two plugins provide the same interface and the
  profile does not choose one, the game does not start. Human input always takes precedence over
  plugin control.
- Plugin data is kept per plugin in the save; disabling a plugin does not delete its data.
- Plugins are written in Luau. The game generates type definitions for the plugin API from its
  own type registry, so editors can type-check plugins.
- Plugins run in a sandbox and can use only the interfaces the game provides.
- Built-in plugins: the MCP agent interface, SD character switching and the AI player, each of
  which can be turned on or off.

**AI player**
- A second player with an SD character body. When no human is controlling it, rules or a language
  model drive it: it gathers materials, crafts furniture and places it.
- Each character acts according to its own persona, starting with Emu Otori.
- A human can take over at any time.

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

**7. Multiplayer**
- Visit other players' homes and gather, craft and decorate together on one site; the host's
  world state is authoritative.
- Agents can join as a player or by taking over a character.

## Original content (unscheduled)

**UI and gameplay**
- Missions, photo mode, shop and exchange, outfit changes, room upgrades.
- Wall and stacking editing, the gathering tool selector, and the full visitor flow.

**Inventory**
- Sorting, filters, category tabs, dismantling and 3D preview.

**Weather and particles**
- Rendering details such as soft depth, some light effects and orbital particles.

**Details**
- Small furniture animations, audio details and the secondary dialogs of each screen.

**Regions**
- Enable other regions once the resource library provides what their exclusive content needs.
- UI language switching (Japanese, Simplified Chinese, Traditional Chinese, Korean, English),
  following the system language by default; each language replaces only the displayed text and
  shares the same resources.

## Engineering

- Upgrade Bevy to 0.20.
- Tidy the environment variable switches, moving values decided by the server into the
  simulation panel.
- Split oversized source files along the original game's classes.
