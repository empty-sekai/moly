# moly-launcher

The native entry point. The launcher downloads and verifies a resource version, then starts the
server and the game.

## Resource versions

The launcher reads the release manifest `game/manifest.json` from a CDN (by default
`https://assets.pjsk.moe/sekai-extra-assets/`) and lists the resource versions it offers. For the
selected version it downloads the catalog, the pack list and the resource files into the resource
directory and checks the SHA-256 of each file.

- A download can be cancelled at any time and continues where it stopped next time.
- Running it again on an installed version verifies and repairs it.
- Versions share files whose content is identical.

When it starts the game, the launcher first starts the server, reads the address the server
actually bound, then starts `moly-app` from the same folder.

## Settings

The CDN address, the resource directory, the number of parallel downloads and the developer window
are changed in the launcher's settings and stored in `launcher.json` in the data directory.

## Data directory

- Windows: `data\` in the folder that holds the program; the launcher and the game in the same
  folder share it.
- Other systems: `$XDG_DATA_HOME/moly`, or `~/.local/share/moly`.

`MOLY_DATA_DIR` selects another directory; the launcher passes its own data directory to the game.
The game's save and settings files and the game logs written by the launcher (`logs/`) are kept in
the data directory as well.

## Resource directory

The resource directory is decided the first time it is needed and written to `launcher.json`:
by default `resources/` in the data directory, or, when that is on the Windows system drive,
`moly\` at the root of the other fixed drive with the most free space.

A folder chosen in the settings is used as is when it is empty, already holds resources or is named
`moly`; otherwise its `moly\` subfolder is used (choosing `E:\` gives `E:\moly`).

When the resource directory changes, installed resources can be moved along:

- On the same drive, the folder is moved.
- Across drives, files are copied one by one and checked against the SHA-256 in their names. The
  old files are deleted only after the switch to the new directory; a cancelled or failed move
  leaves the old directory usable.

## Running the game directly

When `moly-app` runs without the launcher and `MOLY_ASSET_ROOT` is not set, the game reads the
version installed in the launcher's resource directory: the version selected in `launcher.json`,
or the most recently installed one when none is selected.
