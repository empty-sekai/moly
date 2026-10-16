# moly MCP server

A stdio [Model Context Protocol](https://modelcontextprotocol.io) server that starts moly game instances and drives them. It handles processes and transport. Native game executables are driven over HTTP JSON-RPC 2.0. Browser instances run in a Chrome that this server starts and are driven over the Chrome DevTools Protocol; the page reaches the same JSON-RPC methods through its `control()` entry.

Standard output carries the MCP protocol, and every log line goes to standard error. Instances still running when the server exits are killed.

## Requirements

- Node.js 22.13 or later, or Node.js 24 or later, and npm.
- A native build of the game for native instances. The repository's root README describes how to build and run it.
- Google Chrome for browser instances. See [Platform notes](#platform-notes).

## Install

From the repository root:

```sh
npm ci --prefix tools/mcp
```

## Registration

Claude Code reads the repository root's `.mcp.json`, which registers the server as `moly`:

```json
{
  "mcpServers": {
    "moly": {
      "command": "node",
      "args": ["tools/mcp/server.mjs"]
    }
  }
}
```

Start Claude Code from the repository root so that the relative path resolves.

For Claude Desktop, add the server to `claude_desktop_config.json`, using the absolute path of `server.mjs` in your checkout:

```json
{
  "mcpServers": {
    "moly": {
      "command": "node",
      "args": ["/path/to/moly/tools/mcp/server.mjs"]
    }
  }
}
```

## Tools

The server provides 20 tools.

| Tool | Purpose |
|---|---|
| `instance_list` | Lists the instances this server started: `id`, `kind`, `pid`, `started`, `running`, and the RPC URL (native) or the port and URL (browser). |
| `native_launch` | Starts a native executable: `exe` (a file path, with no `PATH` lookup), and optional `env`, `args`, `cwd` and `rpcPort` (default 15702). Output goes to `logs/<id>.log`. Returns `{id, pid, log}`. |
| `native_stop` | Kills the instance's process tree (`taskkill /T /F` on Windows) and marks the instance stopped. |
| `instance_logs` | Returns the last lines of an instance log: `tail` (default 200; 0 for all lines) and an optional `grep`, which is a plain substring filter. |
| `rpc` | Sends one JSON-RPC 2.0 request to any instance: `id`, `method` and optional `params`. Returns the result, or the error object with `isError` set. 10 s timeout. |
| `browser_launch` | Starts Chrome with a fresh profile, opens `url`, and taps `#boot-enter` when the boot screen is ready. Returns `{id, pid, port, state}`. See [Browser instances](#browser-instances). |
| `browser_screenshot` | Captures the current page of a browser instance as a PNG image. |
| `browser_stop` | Closes the DevTools connection and kills the Chrome process this server started. |
| `methods` | Returns `rpc.discover` and `moly.methods` for an instance. |
| `act` | Runs one director step (`step`) and waits until the run is `done`, `failed` or `stopped`. Returns `{status, error, log}` with the last 20 log entries. `timeout` is in seconds, default 60. A `failed` status or a timeout is a tool error. |
| `run_script` | Runs a director script (`script`). With `wait` true (the default) it polls until the run is terminal or `timeout` (seconds, default 300) passes. With `wait` false it returns the first snapshot. A failed run or a timeout is a tool error. |
| `check` | Evaluates one director condition (`cond`). Returns `{holds, read}`. |
| `capture` | Captures the game frame as a PNG image. `timeout` is in seconds, default 10. An error from the game is a tool error. |
| `director_schema` | Returns the JSON Schema of director steps and conditions. |
| `time` | Reads or sets the game clock: `scale` (1 is normal speed), `paused`, `step` (frames to run, then pause, which the tool waits for) and `fixed_dt` (seconds each frame advances, `null` for real time), with `timeout` in seconds (default 60). Returns `{elapsed, scale, paused, stepping, fixed_dt, frame, seed, draws}`. |
| `events` | Reads the event stream after the event numbered `since`, oldest first, with `limit` (default 200, at most 1000) and `kinds`; `wait` is the number of seconds to wait for an event. Returns `{events, next, missed, latest}`. |
| `observe` | Reads the world as structured data by section, the ones named in `what` (all sections when it is absent): `site`, `gatherables`, `bag`, `tools`, `recipes`, `fixtures`, `characters`, `weather` and `time`. |
| `layout_view` | Reads the layout editor: whether it is active, its phase and site, the selected fixture with its put status, the placed rows, the storage, the dialogs, whether it can save, and its last feedback. Rows carry `uid`, `fixture_id`, `texture_id`, `center` (`[x, y, z]` in tiles), `direction` and `grid_size`. The owned catalogue is replaced by `catalog_rows` unless `catalog` is true. |
| `layout_command` | Sends one layout editor command (`command`, plus `index`, `uid`, `center`, or `x` and `z` as the command needs; see `moly.layout.command`) and returns the editor view once two game frames have run after the request. `move_to` takes the touched tile of a drag: an even-sized fixture's reported center can differ from it by a tile, and y follows the column height. A command the editor refuses answers through `feedback` and the selection's `put_status`. |
| `layout_place` | Places one owned fixture: opens the editor when it is closed, selects the first catalogue row of `fixture_id` (and `texture_id`) with a placeable copy, turns it a quarter turn `rotate` times (0 to 3), moves it until the center the view reports is on the x and z of `center` (y follows the column height), and decides it when its put status is `ok` (`decide` false stops before that). Returns `{placed, uid, center, direction, put_status, reason, row, view}`. A refused put leaves the fixture selected and is a tool error. Saving is a separate `save` or `save_and_exit` command. |

Relative paths given to `native_launch` (`exe`, `cwd`) and to `browser_launch` (`chrome`, when it contains a path separator) resolve against the directory the server runs in.

## Native instances

`native_launch` starts the executable with `MOLY_CONTROL_PORT` set to `rpcPort` (default 15702), unless the `env` argument sets `MOLY_CONTROL_PORT` itself. The instance's RPC URL uses that same port.

It also sets `MOLY_SETTINGS_FILE` to `logs/<id>.settings.json` unless `env` sets it, so each instance keeps its documents in its own files and starts from a first run's state. Without `MOLY_DEV=1` in `env` the program runs the game; with it, the developer window, which reads the native environment switches.

The game opens its JSON-RPC endpoint on `127.0.0.1` only when `MOLY_CONTROL_PORT` is set. A value that is not a port number from 1 to 65535 stops the game with a panic, which appears in the instance log.

A native run may start with its window hidden by setting `MOLY_WINDOW_HIDDEN` (any value): the batch runner sets it, so an instance draws without a window on screen. `native_launch` itself does not set it, and a run without it opens the window as usual.

The endpoint opens once the app has built. Poll `rpc` with `rpc.discover` until it answers, as the example does.

The game reads its resources from the directory named by `MOLY_ASSET_ROOT`, which is passed through `env`. For how to prepare that directory, see the local-run section of the repository's root README.

## Browser instances

`browser_launch` starts Chrome with a new profile under `profiles/<id>`, with remote debugging on `port` (a free port when `port` is omitted), and opens `url`. If the page has a `#boot` element, the server waits for its `data-state` attribute to reach `ready`, taps `#boot-enter`, and waits until `#boot` is gone. `state` is `running` after such a tap, `dismissed` when the screen went away without one, and `no-boot` for a page without a boot screen. A `failed` boot state fails the launch with the element's text. `timeout` is the number of seconds to wait for the page to load and for the boot screen, default 600.

The page must load a module at a URL path that ends in `/moly-app.js` and export `control(request)`. The server calls `control` with one JSON-RPC 2.0 request as text, and the function resolves to the reply as text. For browser instances, `rpc`, `methods`, `act`, `run_script`, `check`, `capture`, `director_schema` and the layout tools all use this path. Console output and exceptions go to `logs/<id>.log`.

## Director scripts

A director script is an object with a `name`, an optional `exit`, and `steps`. Each step has a `do` field. `run_script` sends a whole script, and `act` sends one step. `director_schema` returns the JSON Schema of the steps and conditions.

The "Look around" example is bundled with the game (`crates/game/moly-game/src/director/examples.json`, also returned by `moly.director.examples`). Its `script` object is:

```json
{
  "name": "Look around",
  "steps": [
    { "do": "wait_until", "until": "entry_done", "timeout": 120 },
    { "do": "wait_until", "until": "camera_settled", "timeout": 30 },
    { "do": "camera.drag", "by": [320, 0], "secs": 1 },
    { "do": "camera.drag", "by": [0, -120], "secs": 0.6 },
    { "do": "camera.zoom", "by": 240, "secs": 0.6 },
    { "do": "wait", "secs": 2 },
    { "do": "camera.zoom", "by": -240, "secs": 0.6 }
  ]
}
```

Pass the object as `script` to `run_script`, together with the instance's `id`. With the default `wait`, the call returns `{status, error, log}` once the run is `done`, `failed` or `stopped`.

## RPC methods

The game answers 26 `moly.*` methods. Their names are listed in `METHODS` in `crates/game/moly-game/src/control.rs`, and each is registered in `install` in the same file. The table also includes the four protocol methods that the repository names: `rpc.discover`, `registry.schema`, `world.list_components` and `world.list_resources`. The Bevy Remote Protocol provides further `world.*` methods that this table does not list.

| Method | Params | Purpose |
|---|---|---|
| `moly.methods` | none | The moly methods, each with its description and params schema. |
| `moly.state` | none | One read of the running game: frame, time, server clock, entry, site, weather, the player, the characters, the screen stack, dialogs and the camera. |
| `moly.ui.views` | none | The UI views shown, their controls, and the window point each control taps. `window` is `[width, height]`. |
| `moly.tap` | `at` (`[x, y]` in logical pixels) or `at_fraction` (`[x, y]` from 0 to 1 of the window), exactly one | Taps the screen, with press and release in one frame. |
| `moly.ui.tap` | `view` and `path`, both required | Taps the centre of one control of a shown UI view. |
| `moly.panel` | `open` (boolean) and `tab` (`director`, `server`, `state`, `methods` or `more`) | Opens or closes the control panel and selects its tab. |
| `moly.time` | `scale` (number, 0 or more) and `paused` (boolean) | Reads or sets the game clock. |
| `moly.observe` | `what` (section names; every section when it is absent) | The world as structured data: `site`, `gatherables`, `bag`, `tools`, `recipes`, `fixtures`, `characters`, `weather` and `time`. |
| `moly.events` | `since`, `limit` (default 200, at most 1000) and `kinds` | The event stream after the event numbered `since`, oldest first, with the drops counted. |
| `moly.director.schema` | none | The director's steps, conditions and target forms, as JSON Schema. |
| `moly.director.command` | A script `{name, exit, steps}`, or `{run}`, `{stop}`, `{pause}` or `{resume}` | Queues a director command for the next frame. A script is checked whole before it runs, and a new script replaces a running one. |
| `moly.director.examples` | none | The bundled example scripts, as `[{id, title, script}]`. |
| `moly.director.snapshot` | none | The director's status (`idle`, `queued`, `running`, `paused`, `done`, `failed` or `stopped`), the steps running now, the characters it holds, the error that ended the run, and its log. |
| `moly.director.check` | `cond` (required) | Whether a director condition holds now, and what was read. |
| `moly.server.schema` | none | The editable fields and the actions of the server document. |
| `moly.server.document` | none | The server document: its clock, the client's copies, the pending sections and the named refusals. |
| `moly.server.edit` | `path`, `value`, `action` | Makes one edit of the server document, checked the way the server panel checks its edits. Answers the receipt. |
| `moly.game.snapshot` | none | The browser game's projection: entry, storage, persistence, memory and named refusals. |
| `moly.library.command` | A command object with `type` (required), such as `{"type": "open"}` | Queues a content library command for the next frame. |
| `moly.library.snapshot` | none | The content library's page snapshot. |
| `moly.library.catalog` | none | The content library's catalogue export. |
| `moly.layout.view` | none | The layout editor's state: whether it is active, its phase and site, the selected fixture with its put status, the placed rows, the storage, the owned catalogue, whether a dialog is shown, whether it can save, and its last feedback. |
| `moly.layout.command` | `command` (required), plus `index`, `uid`, `center` (`[x, y, z]`), or `x` and `z` as the command needs | Queues one layout editor command for the next frame, as the editor's buttons, list cells and drags send it. Read `moly.layout.view` afterwards for the result. |
| `moly.perf.snapshot` | `frames` (positive integer, optional) | Frame time percentiles over the last 600 frames and over the newest `frames` frames (120 by default). |
| `moly.capture` | none | Requests a capture of the next rendered frame. Answers `{id}`. |
| `moly.capture.take` | `id` (required) | The captured frame as `{png, width, height}` (base64 PNG) once it is ready, else `null`. A frame is handed out once. |
| `rpc.discover` | protocol-defined | Protocol method: lists the methods the endpoint answers. |
| `registry.schema` | protocol-defined | Protocol method: the schema of the type registry. |
| `world.list_components` | protocol-defined | Protocol method: lists the component types. |
| `world.list_resources` | protocol-defined | Protocol method: lists the resource types. |

## Examples

`examples/move-check.mjs` runs an acceptance-style check on a native build:

```sh
node examples/move-check.mjs <exe> <outDir> <assetRoot> [catalog]
```

It starts the executable with `MOLY_ASSET_ROOT` set to `assetRoot` (and `MOLY_ASSET_CATALOG` set to `catalog` when one is given). It waits for the entry sequence and reads the player position, pushes the joystick forward, releases it, and reads the position again. It then drags the camera and compares the field camera's yaw before and after the drag. It writes `report.txt` and `server-stderr.log` into `outDir`, and `tail.txt` when the run fails. It needs a built game and a prepared resource root.

## Reference agent

`agent/reference-agent.mjs` drives an instance from a task file without a language model: it reads the world with `moly.observe`, acts through director steps (`moly.director.command`), and judges the task's success conditions against the observations it took.

```sh
node tools/mcp/agent/reference-agent.mjs <port> <task.json> <runlog.jsonl>
```

The instance must already run with its control endpoint on `<port>`; this script starts nothing. The exit code is 0 when every success condition holds and no plan item failed, and 1 otherwise.

Every request and result is appended to the runlog, one JSON object per line. The first line is the run's header — `kind: "run"`, the task, its site, the port and the world seed read from `moly.time` — and carries that first request and result. Later lines are one RPC each (`{n, request, result | error, ms}`); a plan item appends `{kind: "plan", item, result, note}`, and the last line is `{kind: "result", ...}` with the checks and the failure, if any.

`agent/rpc.mjs` is the JSON-RPC transport the three agent scripts share (`call`, `runStep`, `runScript`, `waitFrames`, `waitFor`).

## Tasks

`tasks/` holds the task files the reference agent runs:

```json
{
  "id": "gather-wood",
  "title": "Go to the harvest site and gather five wood",
  "site": "grassland",
  "plan": [ ... ],
  "success": { "checks": [ ... ] }
}
```

`site` is the site type the task runs on. It is what a launcher may pass as `MOLY_SITE`, and it is also where the plan travels: a task whose plan has no `site` item is taken there first, so a run never needs the instance to have been placed on it.

A plan item is one action the agent knows:

| Item | What the agent does |
|---|---|
| `{"do": "wait_entry", "timeout"?}` | Waits for the entry sequence with a `wait_until entry_done` step. |
| `{"do": "site", "site"?, "entry_timeout"?, "timeout"?}` | Travels to `site` (the task's site when the item names none) with the game's own move: a `wait_until all[entry_done, not site_moving]` step, a `site.switch` step and a `wait_until all[not site_moving, site_is site]` step, then reads the site back. An instance already standing there is left alone. |
| `{"do": "observe", "what": [sections]}` | One `moly.observe` read, kept as the newest reading of those sections. |
| `{"do": "director", "step": {...}, "timeout"?}` | Runs one director step and waits for the run to end. |
| `{"do": "answer_dialog", "wait"?, "ok"?}` | Waits up to `wait` seconds (default 10) for `moly.director.check` to report `dialog_shown`, then runs `dialog.answer` (with `optional` true, so a dialog that will not close does not end the plan); no dialog is not a failure. |
| `{"do": "gather", "kind"?, "count", "step_timeout"?}` | Harvests `count` gatherables of `kind` (any kind when absent), one `player.gather` step each, re-reading `gatherables` between them. |
| `{"do": "craft_first", "list"?, "step_timeout"?}` | Reads `recipes`, takes the first row whose `can_craft` is true and runs `player.craft`; with no craftable row it fails, naming the first row's reason. |
| `{"do": "place", "from"?, "center", "rotate"?, "save"?, "step_timeout"?}` | Runs `layout.place` for the fixture `from` names (`"crafted"` is the target of the last craft, else a master id) on `center` (`[x, y, z]`, or `"free"` for the first tile the placed fixtures do not cover). |
| `{"do": "talk", "unit"?, "step_timeout"?}` | Runs `player.talk_to` for `unit`, or for the first character whose `away` is false. |

Each success check is read against the newest observation of its section:

| Check | Holds when |
|---|---|
| `{"section": "bag", "rule": "bag_gained", "at_least": n}` | The summed material quantities gained `n` over the reading taken before the plan. |
| `{"section": "gatherables", "rule": "harvested_kind", "kind"?, "at_least": n}` | `n` gatherables of `kind` are harvested. |
| `{"section": "fixtures", "rule": "placed_fixtures", "at_least": n}` | `n` more fixtures are placed than before the plan. |
| `{"section": "characters", "rule": "talking_character", "at_least": n}` | `n` characters have `talking` true or the action `Talk`. |

The bundled tasks are `gather-wood` (travel to the grassland, answer the dialog the arrival opens, gather five wood), `craft-and-place` (travel home, craft the first craftable blueprint of the fixture list, place it on the first free tile and save) and `talk-to-visitor` (a character on the site). Their ids, positions and steps are the public ones the observations report; no player data is in them.

The dialog step matters on a harvest site: the arrival opens the learn-phenomenon sub-window dialog about 1.3 s later, and the joystick stays disabled until it is answered, so a plan that gathers must answer dialogs after it travels.

## Replay

`agent/replay.mjs` sends a recorded run again and compares the two runs' event kinds:

```sh
node tools/mcp/agent/replay.mjs <runlog.jsonl> <port> [--settle <seconds>] [--quiet <seconds>]
```

It replays the recorded calls that change state — `moly.director.command`, `moly.time`, `moly.server.edit` and `moly.layout.command` — in the order they were recorded, waiting for each the way the agent waited on it (the director's snapshot, the clock's `step`, two frames for a layout command). Reads are not sent again. It then reads the instance's events and compares the kind sequence with the one the runlog recorded, printing `REPLAY MATCH` or the first difference; the exit code is 0 on a match and 1 otherwise. `--settle` is the seconds one replayed call may take (default 300) and `--quiet` the seconds with no new event that end the comparison (default 10): some events follow a step later, so a window shorter than the run's own tail reports a difference that is only timing.

The seed comes from the runlog's first line. A run cannot be reseeded through the endpoint — the game reads `MOLY_SEED` once, at its first draw — so an instance whose seed is missing or different is refused by name, with the `MOLY_SEED` to start it with.

## Batch runs

`agent/batch.mjs` runs a whole task set, every (task, seed) on its own hidden instance:

```sh
node tools/mcp/agent/batch.mjs --exe <exe> --tasks <dir> --seeds 1,2,3 \
  --parallel 1 --base-port 15740 --out <dir> [--asset-root <dir>] [--boot 300] [--timeout 900]
```

Each run gets its own directory `<out>/<id>-seed<seed>/` with `instance.log` (the instance's own output), `agent.log`, `runlog.jsonl` and `settings.json`. The instance is started with `MOLY_SEED`, `MOLY_WINDOW_HIDDEN`, the task's `MOLY_SITE` when it names one, `MOLY_SETTINGS_FILE`, the run's own `MOLY_CONTROL_PORT` (from `--base-port`, one port per run) and `--asset-root` as `MOLY_ASSET_ROOT`; `--boot` is the seconds to wait for its endpoint. The run's outcome is appended to `<out>/results.json` as `{task, seed, success, seconds, runlog, port, run_dir, error}`, with the failure and the checks when the agent ran. The exit code is 0 when every run succeeded.

`--parallel` is the number of instances at once: 1 or 2 on this host, and a higher value is refused rather than lowered. A port already in use is refused for that run. `--asset-root` is required when `MOLY_ASSET_ROOT` is not set.

## Security

- The game's JSON-RPC endpoint listens on `127.0.0.1` with no authentication. Any local process can send requests while the instance runs, including requests that change game state.
- Chrome's remote debugging port is local and has no authentication either. When `port` is omitted, the server picks a free port.
- `native_launch` starts whatever executable file it is given, and `browser_launch` starts the Chrome executable it is given. Use this server only with MCP clients you trust.
- Stops affect only processes this server started. Other Chrome processes are never touched.
- Logs and browser profiles are written to `logs/` and `profiles/` next to `server.mjs`.

## Platform notes

- Windows: `native_stop`, `browser_stop` and the exit cleanup end the whole process tree with `taskkill /T /F`. The native game window is not hidden, because a hidden first window has no layout.
- macOS and Linux: those stops send `SIGKILL` to the process the server started. Its child processes are not signalled.
- Default Chrome executable: `C:/Program Files/Google/Chrome/Application/chrome.exe` on Windows, `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` on macOS, and `google-chrome` looked up on `PATH` elsewhere.
- The `MOLY_CHROME` environment variable overrides the platform default. The `chrome` argument of `browser_launch` takes precedence over both.
- A bare command name (one with no path separator) is looked up on `PATH`. Any other value is a file path.

## Tests

```sh
npm ci --prefix tools/mcp
npm test --prefix tools/mcp
```

The test starts the server over stdio and launches `test/fake-game.mjs`, a JSON-RPC stub on port 15799, as a native instance. It then checks the tool list, `ping`, the error for an unknown method, the instance log, the game tools (`methods`, `director_schema`, `act`, `run_script`, `check` and `capture`), the layout tools against a stub editor (a placement, a fixture with no placeable copy, a refused put and a cancel), and `native_stop`. It exits with code 0 on success and 1 on failure. Port 15799 must be free. The browser tools and a real game are not covered.

## Layout

- `server.mjs`: the server.
- `test/`: the self test and its stub game.
- `examples/`: the example script.
- `agent/`: the reference agent, the replay, the batch runner and their transport.
- `tasks/`: the task files the reference agent runs.
- `logs/` and `profiles/`: created at run time and not tracked.
