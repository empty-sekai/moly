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

The server provides 14 tools.

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

Relative paths given to `native_launch` (`exe`, `cwd`) and to `browser_launch` (`chrome`, when it contains a path separator) resolve against the directory the server runs in.

## Native instances

`native_launch` starts the executable with `MOLY_CONTROL_PORT` set to `rpcPort` (default 15702), unless the `env` argument sets `MOLY_CONTROL_PORT` itself. The instance's RPC URL uses that same port.

The game opens its JSON-RPC endpoint on `127.0.0.1` only when `MOLY_CONTROL_PORT` is set. A value that is not a port number from 1 to 65535 stops the game with a panic, which appears in the instance log.

The endpoint opens once the app has built. Poll `rpc` with `rpc.discover` until it answers, as the example does.

The game reads its resources from the directory named by `MOLY_ASSET_ROOT`, which is passed through `env`. For how to prepare that directory, see the local-run section of the repository's root README.

## Browser instances

`browser_launch` starts Chrome with a new profile under `profiles/<id>`, with remote debugging on `port` (a free port when `port` is omitted), and opens `url`. If the page has a `#boot` element, the server waits for its `data-state` attribute to reach `ready`, taps `#boot-enter`, and waits until `#boot` is gone. `state` is `running` after such a tap, `dismissed` when the screen went away without one, and `no-boot` for a page without a boot screen. A `failed` boot state fails the launch with the element's text. `timeout` is the number of seconds to wait for the page to load and for the boot screen, default 600.

The page must load a module at a URL path that ends in `/moly-app.js` and export `control(request)`. The server calls `control` with one JSON-RPC 2.0 request as text, and the function resolves to the reply as text. For browser instances, `rpc`, `methods`, `act`, `run_script`, `check`, `capture` and `director_schema` all use this path. Console output and exceptions go to `logs/<id>.log`.

## Director scripts

A director script is an object with a `name`, an optional `exit`, and `steps`. Each step has a `do` field. `run_script` sends a whole script, and `act` sends one step. `director_schema` returns the JSON Schema of the steps and conditions.

The "Look around" example is bundled with the game (`crates/moly-game/src/director/examples.json`, also returned by `moly.director.examples`). Its `script` object is:

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

The game answers 22 `moly.*` methods. Their names are listed in `METHODS` in `crates/moly-game/src/control.rs`, and each is registered in `install` in the same file. The table also includes the four protocol methods that the repository names: `rpc.discover`, `registry.schema`, `world.list_components` and `world.list_resources`. The Bevy Remote Protocol provides further `world.*` methods that this table does not list.

| Method | Params | Purpose |
|---|---|---|
| `moly.methods` | none | The moly methods, each with its description and params schema. |
| `moly.state` | none | One read of the running game: frame, time, server clock, entry, site, weather, the player, the characters, the screen stack, dialogs and the camera. |
| `moly.ui.views` | none | The UI views shown, their controls, and the window point each control taps. `window` is `[width, height]`. |
| `moly.tap` | `at` (`[x, y]` in logical pixels) or `at_fraction` (`[x, y]` from 0 to 1 of the window), exactly one | Taps the screen, with press and release in one frame. |
| `moly.ui.tap` | `view` and `path`, both required | Taps the centre of one control of a shown UI view. |
| `moly.panel` | `open` (boolean) and `tab` (`director`, `server`, `state`, `methods` or `more`) | Opens or closes the control panel and selects its tab. |
| `moly.time` | `scale` (number, 0 or more) and `paused` (boolean) | Reads or sets the game clock. |
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

The test starts the server over stdio and launches `test/fake-game.mjs`, a JSON-RPC stub on port 15799, as a native instance. It then checks the tool list, `ping`, the error for an unknown method, the instance log, the game tools (`methods`, `director_schema`, `act`, `run_script`, `check` and `capture`), and `native_stop`. It exits with code 0 on success and 1 on failure. Port 15799 must be free. The browser tools and a real game are not covered.

## Layout

- `server.mjs`: the server.
- `test/`: the self test and its stub game.
- `examples/`: the example script.
- `logs/` and `profiles/`: created at run time and not tracked.
