# moly release package

moly is a recreation of MYSEKAI, built with Rust and Bevy. This document is for users who have downloaded a release package, and describes its contents, how to run it and how to upgrade it. For a description of the project, see the [project repository](https://github.com/empty-sekai/moly).

Release packages do not contain any game resources. The models, textures, animations and audio needed to run the game are supplied by the user and are not distributed with the repository.

## Package contents

- The game `moly-app` (`moly-app.exe` on Windows).
- `fonts/OFL-1.1.txt`: the copyright and SIL Open Font License 1.1 for the open-source Resource Han Rounded font embedded in the game and launcher.
- The launcher `moly-launcher` (`moly-launcher.exe` on Windows): downloads resources, handles settings, and starts the server and the game.
- The server `moly-server` (`moly-server.exe` on Windows): stores player documents and answers requests from the game.

Browser stage and game archives contain `pkg/fonts/<sha256>.ttf` and
`pkg/fonts/OFL-1.1.txt`. Each renderer's `pkg/<renderer>/build.json` records the
font and license paths, SHA-256 digests and byte lengths. A published browser
release includes both files in its `integrity.json`. Keep the license alongside
the font when copying or hosting these packages; packaging rejects missing or
changed license files.

There is also a server-only package, `moly-server-<version>-<platform>`, which contains only `moly-server`.

## Running

### Windows

1. Download the Windows package from [Releases](https://github.com/empty-sekai/moly/releases) and extract it.
2. Run the launcher, choose a resource version and download it. When the download finishes, the launcher checks the SHA-256 of each file. An interrupted download can be resumed.
3. Click 开始游戏 (Start game). The launcher starts the server first, then the game.

For the launcher's data directory, resource directory and migration rules, see the [launcher documentation](https://github.com/empty-sekai/moly/blob/master/crates/app/moly-launcher/README.md).

### Linux and macOS

The Linux x64 and macOS Apple Silicon packages have the same contents and usage as the Windows package. The release workflow builds these packages and checks their versions, but they have not yet been verified on real hardware.

- Linux: the system must provide ALSA, udev, a Vulkan driver and WebKitGTK 4.1 (for the launcher window).
- macOS: the program is not signed. After extracting, run `xattr -dr com.apple.quarantine <extracted directory>` before running the launcher.

## Upgrading

On Windows, after a new version is extracted into its own folder, the launcher takes over the settings (including the resource directory), the game settings and the saves (the `server` folder) from the most recent earlier installation on first launch. Files that already exist in the new data directory are not replaced, and the earlier installation directory is not changed.

The resource directory can be changed in the 位置 (Locations) card on the launcher's home page, or in 设置 (Settings).

## Running the server separately

To run the server on a separate machine, use the `moly-server-<version>-<platform>` package. The listen address, resource directory and save directory are set with environment variables. See the [server documentation](https://github.com/empty-sekai/moly/blob/master/docs/server.md).

## Reporting issues

Please report any problems you encounter in [Issues](https://github.com/empty-sekai/moly/issues).

## License

The code is licensed under [AGPL-3.0-only](https://github.com/empty-sekai/moly/blob/master/LICENSE). Third-party content is distributed under its own licenses; see [THIRD_PARTY_NOTICES.md](https://github.com/empty-sekai/moly/blob/master/THIRD_PARTY_NOTICES.md). The code license does not grant any right to use or distribute game resources.
