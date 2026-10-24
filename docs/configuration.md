# Configuration

## Environment variables

| Variable | Description | Default |
|---|---|---|
| `MOLY_ASSET_ROOT` | Resource root directory | The version selected in the launcher. If none is selected, the most recently installed version is used. |
| `MOLY_SERVER_URL` | Server that the game connects to, an `http://` or `https://` address | `http://127.0.0.1:15800` |
| `MOLY_SERVER_ADDR` | Server listen address | `127.0.0.1:15800` |
| `MOLY_SERVER_CONTROL_PORT` | Server control port | `15801` |
| `MOLY_SERVER_DATA` | Directory where player documents are saved | Not set: documents are kept only in memory and start over on each launch |
| `MOLY_DATA_DIR` | Data directory (saves, settings, logs) | See the [launcher documentation](../crates/app/moly-launcher/README.md) |
| `MOLY_SETTINGS_FILE` | Settings file | `settings.json` in the data directory |
| `MOLY_DEV` | Starts the developer window when set to `1` | Off |
| `MOLY_MASTER_HARUKI_BASE` | First master table source: an HTTPS template with a `{repo}` placeholder | See [master tables and regions](masters.md) |
| `MOLY_MASTER_BASE`, `MOLY_MASTER_MIRROR_BASE` | The two master table sources after the first | See [master tables and regions](masters.md) |
| `MOLY_PLAYER_API` | URL template for player data (developer window) | See [importing player data](player-data.md) |
| `MOLY_PLAYER_REGION` | Region of the player data, `cn` or `jp` (developer window) | |
| `MOLY_PLAYER_UID` | Fetches and previews data for this UID at startup (developer window) | |
| `MOLY_PLAYER_DATA_FILE` | Imports the specified JSON file at startup (developer window) | |

Set only one of `MOLY_PLAYER_UID` and `MOLY_PLAYER_DATA_FILE`.

Settings, server documents and the local list are stored separately: in the settings file, and in `settings.server.json` and `settings.local.json` next to it. A second process that runs at the same time only reads these files.

## Page parameters (browser version)

| Parameter | Description |
|---|---|
| `server` | Address of the server to connect to. Defaults to the public server. |
| `renderer=webgl2` | Uses the WebGL2 backend |
| `player_uid`, `player_region`, `player_api` | Inputs for fetching player data |

Each browser stores a random user ID in its local storage.
