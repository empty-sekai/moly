# moly

[中文](README.md) | English

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

A recreation of MYSEKAI built with Rust and [Bevy](https://bevyengine.org/). It can run as a native program or in a browser (WebAssembly, with WebGPU or WebGL2).

The project recreates scene exploration, character behaviour, furniture interaction and weather presentation. Client behaviour is recreated to match the original game. Values decided by the server are supplied by a separate server process and are gathered in one editable simulation panel.

> **The project is still under development.** The scope of the current version and later plans are described in the [roadmap](ROADMAP.en.md), and known issues are listed in [Issues](https://github.com/empty-sekai/moly/issues).

## Features

Grouped by gameplay domain; the crates that implement each domain are in parentheses (see [Project structure](docs/architecture.md) for each crate's responsibility):

- **Scenes and movement** (`moly-world`, `moly-features`): the entry flow, cannons and doors that move between sites, indoor rooms, player characters, footsteps and expressions.
- **Gathering** (`moly-harvest`): harvest maps, obstacles, tools and stamina, gathering particles, drops and pickup, holding limits, the gathering HUD, bulk harvest and notices when items are obtained.
- **Delivery** (`moly-delivery`): the birthday party delivery site, the delivery flow, drops, honor rewards and the delivery screen.
- **Crafting and conversion** (`moly-features`, `moly-convert`): the crafting UI and inventory, and the conversion machine's slots and screen.
- **Furniture editing** (`moly-fixture-edit`): placing furniture from the inventory, and layout editing for floor furniture, roads and environments.
- **Missions and photos** (`moly-mission`, `moly-photo`): the mission screen and reward receive, and photo mode with saving the shot.
- **Content library** (`moly-content-library`): browsing conversations, furniture and NPC activities, and staging and playing a choice on the field.
- **NPCs** (`moly-world`): pathfinding, conversations after pairing, overhead tweet bubbles, idle action decisions, autonomous behaviour and furniture interaction.
- **Weather and particles** (`moly-law`, `moly-render`): the weather phenomena pipeline, end to end.
- **Audio and presentation** (`moly-audio`, `moly-cutscene`): BGM, SE (sound effects), voice and ambient sound, record BGM, birthday parties and cutscenes.
- **Director scripts and plugins** (`moly-devtools`, `moly-plugin`): JSON scripts or Luau plugins that orchestrate players, characters, the camera, weather and the server simulation panel.
- **MCP server** (`tools/mcp`): lets AI clients start and drive native or browser instances.

Behaviour matches the original game; the structure is organised for this repository. How the behaviour is reimplemented, what must match and how it is checked are described in [Method](docs/METHOD.md); the scope of each item is in the [roadmap](ROADMAP.en.md).

## Quick start

### Resources

Models, textures, animations and audio required to run the project are supplied by the user and are not distributed with the repository. The rights to the project code and to the game resources are independent of each other.

### Windows release package

1. Download the Windows package from [Releases](https://github.com/empty-sekai/moly/releases) and extract it.
   The package contains the game `moly-app.exe`, the server `moly-server.exe` and the launcher `moly-launcher.exe`.
2. Run the launcher, choose a resource version and download it. When the download finishes, the launcher checks the SHA-256 of each file. An interrupted download can be resumed.
3. Click 开始游戏 (Start game). The launcher starts the server first, then the game.

For the launcher's data directory, resource directory and migration rules, see the [launcher documentation](crates/app/moly-launcher/README.md).

### Linux and macOS release packages

The Releases page also has Linux x64 (`x86_64-unknown-linux-gnu`) and macOS Apple Silicon (`aarch64-apple-darwin`) packages. Their contents and usage are the same as the Windows package. The release workflow builds these packages and checks their versions, but they have not yet been verified on real hardware. Please report any problems you encounter in [Issues](https://github.com/empty-sekai/moly/issues).

- Linux: the system must provide ALSA, udev, a Vulkan driver and WebKitGTK 4.1 (for the launcher window).
- macOS: the program is not signed. After extracting, run `xattr -dr com.apple.quarantine <extracted directory>` before running the launcher.

### Running the server separately

Each platform also has a server-only package, `moly-server-<version>-<platform>`, for running the server on a separate machine. It reads the same resources as the game. The listen address, resource directory and save directory are set in the configuration file `moly.toml` or through their environment variables; see [Server](docs/server.md) and [Configuration](docs/configuration.md#server).

## Documentation

- [Building from source](docs/building.md): requirements, and how to build the native program, the browser version and Android.
- [Server](docs/server.md): the server's listen address, control port and how to run it separately.
- [Configuration](docs/configuration.md): the configuration file `moly.toml`, the environment variables that override its keys, and page parameters for the browser version.
- [Importing player data](docs/player-data.md): steps, backups and resource requirements for importing player data in the developer window.
- [Master tables and regions](docs/masters.md): where master tables are read from, how they are merged, and which regions are enabled.
- [MCP server](docs/mcp.md): the MCP server that lets AI clients start and drive game instances.
- [Plugins](docs/plugins.md): the Luau plugin interface, every director step, the observable sections, the events and the environment switches.
- [Project structure](docs/architecture.md): the responsibilities of each directory and crate.
- [Method](docs/METHOD.md): how the client's behaviour is understood and reimplemented, which behaviour matches the original value for value, which is this repository's own, and how matching is checked.
- [Development](docs/development.md): testing and checks, commit conventions, continuous integration, and versioning and releases.
- [Release package guide](docs/release-package.md): what a downloaded release package contains, and how to run and upgrade it.

## Contributing

Before contributing code or documentation, please read [CONTRIBUTING.md](CONTRIBUTING.md).

## License

The code is licensed under [AGPL-3.0-only](LICENSE). Third-party content is distributed under its own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The code license does not grant any right to use or distribute game resources.
