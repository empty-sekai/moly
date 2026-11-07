# moly

[中文](README.md) | English

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

An open-source game runtime compatible with MYSEKAI, built with Rust and [Bevy](https://bevyengine.org/). It can run as a native program or in a browser (WebAssembly, with WebGPU or WebGL2).

It loads MYSEKAI resources supplied by the user and runs scene exploration, character behaviour, furniture interaction and weather presentation with behaviour compatible with the original client. Values decided by the server are supplied by the bundled server process and are gathered in one editable simulation panel.

> [!IMPORTANT]
> **Read before use: purpose and scope**
>
> **moly is intended only for study and for local, single-player use. It is not meant to replace the original game's client or servers.**
>
> This repository contains only moly's own program code. For game content its only job is to load the resources the user supplies and run them compatibly with the original client.
>
> - **No game resources.** It contains no models, textures, animations, audio, master tables or other game resources, and no data derived from them. It has no built-in address for resources, master tables or player data; the user configures every such address.
> - **No extraction tools.** The program reads a prepared resource directory; it does not unpack or decrypt game files.
> - **No original client code.** It contains no source code or decompiled code of the original client in any form. Behaviour is reimplemented from an understanding of the original client; see [Method](docs/METHOD.md).
> - **A server of its own.** The bundled server is moly's own implementation, entirely different from the original game's servers, with no compatible interface or implementation. It has a single endpoint that speaks moly's own binary protocol, and every request first declares moly's protocol version. The original game client cannot make any meaningful connection to it; it serves only moly's client.
> - **Local single-player only.** It does not connect to the original game's servers. The server is run by users for their own play; it has no accounts, login or payments, and no multiplayer.

## What works

Grouped by gameplay domain; the crates that implement each domain are in parentheses (see [Project structure](docs/architecture.md) for each crate's responsibility).
Behaviour is compatible with the original client; the structure is organised for this repository. How the behaviour is reimplemented, what must match and how it is checked are described in [Method](docs/METHOD.md).

- **Scenes and movement** (`moly-world`, `moly-features`): the flow and banner for entering MySekai; cannons and doors that move between sites, doors that change scenes, the map selection screen; indoor rooms (indoor shell and wall culling), with several sites kept loaded; the visitor gate: the invite cutscene, changing visitors and visitors coming out of the gate; site expansion and the room expansion performance after a MySekai rank-up; player characters, footsteps and expressions.
- **Gathering** (`moly-harvest`): harvest maps, obstacles, tools and stamina, gathering particles, drops and pickup, holding limits; the gathering HUD (current tool, action icons, a hint bubble when a tool is missing, the stamina bar); notices and dialogs for obtained items; the hold-to-charge hammer that clears a map in one action, see [Bulk harvest](docs/bulk-harvest.md).
- **Delivery** (`moly-delivery`): the birthday party delivery site and the delivery flow, drops, honor rewards and total rewards, and the delivery screen.
- **Crafting and conversion** (`moly-features`, `moly-convert`): the crafting screen and its result dialog, and the server replies for crafting and blueprints; the conversion machine's slots, its screen and the server rules for conversion.
- **Furniture editing** (`moly-fixture-edit`): the inventory screen with its category tabs, filters, sorting and breaking down, and placing furniture from it; layout editing for floor furniture (stacking included: stacked furniture moves and rotates with its base), roads and tiles, and environments, with the inventory and placed objects updated in the same layout transaction.
- **Missions and photos** (`moly-mission`, `moly-photo`): the mission screen and reward receive, and photo mode with saving the shot.
- **Content library** (`moly-content-library`): browsing conversations, furniture and NPC activities; in its independent mode it stands a temporary set, stages and plays a choice on the field, and restores the site afterwards.
- **NPCs** (`moly-world`): pathfinding, conversations after pairing, overhead tweet bubbles, idle action decisions, autonomous behaviour and furniture interaction; target positions chosen from the original walkable cells; the layered NPC state machine of the Japanese 7.0.0 version, and conversations between characters of different units.
- **Weather and particles** (`moly-law`, `moly-render`): the weather phenomena pipeline, end to end (host lifecycle, sub-emitters, curves, raindrop quads, soft particles, trails and orbital motion and more); the weather page choosing an environment from unlocked content; UI particle effects.
- **Graphics** (`moly-render`): the main light's shadow coverage, resolution, filter (`hard`, `tent`, `pcf`, `pcss`) and strength are set by the `[graphics]` section of `moly.toml` and can be changed while the game runs with `moly.graphics.set`, see [Configuration](docs/configuration.md).
- **Audio and presentation** (`moly-audio`, `moly-cutscene`): BGM, SE (sound effects), voice and ambient sound, record BGM and the BGM selection screen; birthday party cutscenes, the party's conversations and birthday voice lines, the closing and the site restoration.
- **Sequences** (`moly-sequence`, `moly-sequence-player`): JSON sequence documents that drive the camera, the cast's moves and motions, faces, lines, captions, sounds and screen effects on one timeline, with pause, seek and speed, and the paused camera written back into the document, see [Sequences](docs/sequence.md).
- **Ragdoll** (`moly-ragdoll`): characters drop out of their animation as rigid bodies, get up once at rest and return to what they were doing, see [Ragdoll](docs/ragdoll.md).
- **UI and settings** (`moly-ui`): menus, information pages and settings, the phenomenon learning dialog and screen transitions.
- **Director scripts and plugins** (`moly-act`, `moly-devtools`, `moly-plugin`): JSON scripts or Luau plugins that orchestrate players, characters, the camera, weather and the server simulation panel, both run through one action registry; a talk lens frames the speaker of each line and fades out furniture that blocks the view; plugins have manifests, dependencies and conflicts, their own sandbox and a per-frame budget, and none is enabled by default, see [Plugins](docs/plugins.md).
- **Recording** (`moly-devtools`): records the window and the game's mix to MP4, with the game clock advancing at a fixed frame rate while it records, see [Recording](docs/recording.md).
- **MCP server** (`tools/mcp`): lets AI clients start and drive native or browser instances, read state, run scripts and capture frames, see [MCP server](docs/mcp.md).
- **Server and launcher** (`moly-server`, `moly-launcher`): the server runs as a separate process, stores the player document and answers the game's requests; the launcher chooses, downloads and verifies a resource version, starts the server and the game, checks for and installs program updates, and lets each of its directories be changed, moving existing content along.
- **Data and text** (`moly-masters`, `moly-text`): the master data uses the Japanese server as its base and merges each region's exclusive rows, read at run time from master repositories the user configures, with a mirror as a fallback, see [Master tables and regions](docs/masters.md); when Chinese is selected in the browser, matching Chinese-server text can replace the Japanese display text, and the native program can apply an official or community translation overlay; the open-source Resource Han Rounded font is used.
- **Browser** (`moly-stage`, `web/`): the embedded stage and the full game, each with WebGPU and WebGL2 builds; the browser runs the original game only and loads no plugins.

## Known limitations

These parts of the original game are not implemented:

- The harvest tool picker (its list of tools is not drawn; the current tool is shown as usual); wall and fence editing.
- The inventory's 3D preview, search and tags; the shop and exchange; outfits.
- Rendering details such as some light effects; some secondary dialogs of the screens (for example the count selector when breaking down, and the level release dialog).
- Only the Japanese region is enabled; other regions' exclusive content and switching the interface language are not implemented.

Platforms:

- Android is not supported: the package crashes on a physical device after its first frames. `crates/app/moly-android` and the [build notes](docs/building.md) are kept as they are, for reference only.
- The Linux and macOS packages are built and version-checked by the release workflow and have not been tested on real machines.

## Quick start

### Resources

Models, textures, animations and audio required to run the project are supplied by the user and are not distributed with the repository. The rights to the project code and to the game resources are independent of each other.

### Windows release package

1. Download the Windows package from [Releases](https://github.com/empty-sekai/moly/releases) and extract it.
   The package contains the game `moly-app.exe`, the server `moly-server.exe` and the launcher `moly-launcher.exe`. The C and C++ runtimes are linked statically, so the Visual C++ Redistributable is not needed.
2. Run the launcher, configure **your own** resource address, choose a resource version and download it. When the download finishes, the launcher checks the SHA-256 of each file. An interrupted download can be resumed.
3. Click 开始游戏 (Start game). The launcher starts the server first, then the game.

For the launcher's data directory, resource directory and migration rules, see the [launcher documentation](crates/app/moly-launcher/README.md).

### Linux and macOS release packages

The Releases page also has Linux x64 (`x86_64-unknown-linux-gnu`) and macOS Apple Silicon (`aarch64-apple-darwin`) packages. Their contents and usage are the same as the Windows package. The release workflow builds these packages and checks their versions; they have not been tested on real machines.

- Linux: the system must provide ALSA, udev, a Vulkan driver and WebKitGTK 4.1 (for the launcher window).
- macOS: the program is not signed. After extracting, run `xattr -dr com.apple.quarantine <extracted directory>` before running the launcher.

### Running the server on another machine of your own

Each platform also has a server-only package, `moly-server-<version>-<platform>`, for running the server on another machine of your own, for your own moly client to connect to. It reads the same resources as the game. The listen address, resource directory and save directory are set in the configuration file `moly.toml` or through their environment variables; see [Server](docs/server.md) and [Configuration](docs/configuration.md#server).

## Documentation

- [Building from source](docs/building.md): requirements, and how to build the native program and the browser version (the Android build steps are kept for reference only).
- [Server](docs/server.md): the server's protocol, listen address and control port, and how to run it on another machine of your own.
- [Configuration](docs/configuration.md): the configuration file `moly.toml`, the environment variables that override its keys, and page parameters for the browser version.
- [Importing player data](docs/player-data.md): steps, backups and resource requirements for importing player data in the developer window.
- [Master tables and regions](docs/masters.md): where master tables are read from, how they are merged, and which regions are enabled.
- [MCP server](docs/mcp.md): the MCP server that lets AI clients start and drive game instances.
- [Plugins](docs/plugins.md): the Luau plugin interface, every director step, the observable sections, the events and the environment switches.
- [Sequences](docs/sequence.md): the sequence document format, and the methods that play, pause, seek and frame it.
- [Ragdoll](docs/ragdoll.md): the methods, settings and step that make a character fall and get up.
- [Recording](docs/recording.md): the methods and parameters for recording the window and the game's mix to MP4.
- [Project structure](docs/architecture.md): the responsibilities of each directory and crate.
- [Method](docs/METHOD.md): how the original client is understood and reimplemented, which behaviour matches the original value for value, which is this repository's own, and how matching is checked.
- [Development](docs/development.md): testing and checks, commit conventions, continuous integration, and versioning and releases.
- [Release package guide](docs/release-package.md): what a downloaded release package contains, and how to run and upgrade it.

## License

The code is licensed under [AGPL-3.0-only](LICENSE). Third-party content is distributed under its own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The code license does not grant any right to use or distribute game resources.
