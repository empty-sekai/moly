# MCP server

`tools/mcp` is an MCP server that lets AI clients start the game's native program or a browser instance. Over JSON-RPC, it reads state, runs director scripts and captures frames. It starts the server first, obtains the address the server actually bound to, and then starts the game.

```sh
npm ci --prefix tools/mcp
```

The `.mcp.json` file in the repository root registers it as `moly`. For the tools, JSON-RPC methods and security notes, see [tools/mcp/README.md](../tools/mcp/README.md).
