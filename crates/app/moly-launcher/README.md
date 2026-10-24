# moly-launcher

The native entry point. The launcher downloads and verifies a resource version, then starts the
server and the game. It also installs program updates.

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
are changed in the launcher's settings and stored in `launcher.json` in the data directory. The
release source of program updates (`update_source`) is set in the same file; see
[Program updates](#program-updates).

## Home folder

The launcher keeps its files in a home folder, which holds four folders:

- `data`: `launcher.json`, the game's settings and saves (in the `server` folder).
- `logs`: the game logs, including the server output the launcher writes into them.
- `versions`: the installed program versions.
- `resources`: the resource directory, unless the configuration names another one.

The home folder is `moly` at the root of the fixed drive, other than the system drive, that has the
most free space, or `%LOCALAPPDATA%\moly` when there is no such drive. On other systems it is
`$XDG_DATA_HOME/moly`, or `~/.local/share/moly`. The choice is saved the first time the launcher
starts, in the locations file: `%LOCALAPPDATA%\moly\locations.json` on Windows, and
`$XDG_CONFIG_HOME/moly/locations.json` (or `~/.config/moly/locations.json`) on other systems. A
folder with no place of its own in the locations file sits in the home folder.

The Places card on the main page changes the home folder and the `data`, `logs` and `versions`
folders, and opens each of them. A change moves the folder:

- Changing the home folder moves the `data`, `logs` and `versions` folders that sit in the old home
  folder, and the resource directory when it sits there too. Folders placed elsewhere stay where
  they are. The locations file is updated after each step, so a failed step leaves every folder
  where the file says it is.
- The destination of a moved folder must not exist or must be empty. On the same drive the folder
  is renamed; across drives it is copied and then removed. Moves of `data`, `logs` and `versions`
  cannot be cancelled.
- Moves are refused while the game runs, and while a download, another move or an update runs.

`MOLY_DATA_DIR` sets the data directory directly, and the launcher passes its data directory to the
game. While it is set, the data folder cannot be moved from the Places card. The launcher window's
profile is kept in the `webview` folder beside the locations file, outside the folders that move.

## Upgrading

A new version unzipped into its own folder starts with an empty data directory. On its first
start, the launcher takes the settings (including the resource directory), the game settings and
the saves (the `server` folder) from the most recent earlier install it finds. Earlier installs are
the data directories recorded in `%LOCALAPPDATA%\moly\installs.json` (written on Windows) and the
`data` folders beside the program folder; the one whose `launcher.json` was modified most recently
is used. Files that the new data directory already has are not replaced. Nothing in the earlier
folder is changed.

## Program updates

When it starts, the launcher reads the newest release of the repository named by `update_source`
in `launcher.json`. The default is `https://api.github.com/repos/empty-sekai/moly`; the address
must use https. When a release newer than the running launcher is published, a banner at the top
of the main page offers the update and links to the release notes.

Starting the update installs the newest release into `versions/<version>`:

- The full package is downloaded. Every downloaded file is checked against the release's
  `SHA256SUMS`.
- A version folder is complete when it holds `moly-app`, `moly-server` and `moly-launcher` (with
  the `.exe` suffix on Windows). The two newest complete versions are kept, together with the
  versions in use.
- A release whose launcher program differs from the running one replaces it. The banner then
  offers a restart, which starts the launcher again and ends this process.
- The game that starts next uses the newest installed version.

An update can be cancelled while its files are downloaded. An update does not start while the game
runs, the game does not start while an update runs, and the restart is refused while the game runs.

On its first start, the launcher seeds `versions/<version>` with the program files it runs from,
when that version has no complete folder yet.

## Resource directory

The resource directory is decided the first time it is needed and written to `launcher.json`: by
default `resources` in the home folder.

A folder chosen in the settings or on the Places card is used as is when it is empty, already holds
resources or is named `moly`; otherwise its `moly\` subfolder is used (choosing `E:\` gives
`E:\moly`).

When the resource directory changes, installed resources can be moved along:

- On the same drive, the folder is moved.
- Across drives, files are copied one by one and checked against the SHA-256 in their names. The
  old files are deleted only after the switch to the new directory; a cancelled or failed move
  leaves the old directory usable.
- Switching without moving leaves the installed resources in the old directory, and the launcher
  reads the new directory from then on.

The change buttons on the Places card are disabled while a download, a move, an update or the game
runs.

## Running the game directly

When `moly-app` runs without the launcher and `MOLY_ASSET_ROOT` is not set, the game reads the
version installed in the launcher's resource directory: the version selected in `launcher.json`,
or the most recently installed one when none is selected.
