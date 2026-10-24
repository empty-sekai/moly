# moly

[中文](README.md) | English

[![Static checks](https://github.com/empty-sekai/moly/actions/workflows/ci.yml/badge.svg)](https://github.com/empty-sekai/moly/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/empty-sekai/moly)](https://github.com/empty-sekai/moly/releases)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

A recreation of MYSEKAI built with Rust and [Bevy](https://bevyengine.org/). It can run as a native program or in a browser (WebAssembly, with WebGPU or WebGL2).

The project recreates scene exploration, character behaviour, furniture interaction and weather presentation. Client behaviour is recreated to match the original game. Values decided by the server are supplied by a separate server process and are gathered in one editable simulation panel.

> **The project is still under development.** The scope of the current version and later plans are described in the [roadmap](ROADMAP.en.md), and known issues are listed in [Issues](https://github.com/empty-sekai/moly/issues).

![Characters, furniture and overhead bubbles in moly](screenshots/readme.png)

## Features

- **Scenes and movement**: the entry flow, cannons and doors that move between sites, indoor rooms, player characters, footsteps and expressions.
- **Gathering**: obstacles, gathering particles, holding limits, the gathering HUD and notices when items are obtained.
- **Delivery, crafting and placement**: the delivery flow, crafting UI, inventory, placing furniture from the inventory, and layout editing for floor furniture.
- **NPCs**: pathfinding, conversations after pairing, overhead tweet bubbles, idle action decisions, autonomous behaviour and furniture interaction.
- **Weather**: the weather phenomena pipeline, end to end.
- **Audio and presentation**: BGM, SE (sound effects), voice and ambient sound, record BGM, and birthday parties.
- **Director scripts**: JSON scripts that orchestrate players, characters, the camera, weather and the server simulation panel.
- **MCP server**: lets AI clients start and drive native or browser instances.

The scope of each item is described in the [roadmap](ROADMAP.en.md).

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

Each platform also has a server-only package, `moly-server-<version>-<platform>`, for running the server on a separate machine. It reads the same resources as the game. The listen address, resource directory and save directory are set with environment variables; see [Server](docs/server.md) and [Environment variables](docs/configuration.md#environment-variables).

## Documentation

- [Building from source](docs/building.md): requirements, and how to build the native program, the browser version and Android.
- [Server](docs/server.md): the server's listen address, control port and how to run it separately.
- [Configuration](docs/configuration.md): environment variables and page parameters for the browser version.
- [Importing player data](docs/player-data.md): steps, backups and resource requirements for importing player data in the developer window.
- [Master tables and regions](docs/masters.md): where master tables are read from, how they are merged, and which regions are enabled.
- [MCP server](docs/mcp.md): the MCP server that lets AI clients start and drive game instances.
- [Project structure](docs/architecture.md): the responsibilities of each directory and crate.
- [Development](docs/development.md): testing and checks, commit conventions, continuous integration, and versioning and releases.
- [Release package guide](docs/release-package.md): what a downloaded release package contains, and how to run and upgrade it.

## Contributing

Before contributing code or documentation, please read [CONTRIBUTING.md](CONTRIBUTING.md).

## License

The code is licensed under [AGPL-3.0-only](LICENSE). Third-party content is distributed under its own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The code license does not grant any right to use or distribute game resources.
