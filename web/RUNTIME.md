# Browser runtime

How the browser build is laid out, how a page picks its renderer and server, and the limits
the resource loader applies.

## Build output

`node web/build-wasm.mjs` writes one module per renderer, `web/pkg/webgpu/` and `web/pkg/webgl2/`;
a page downloads only the module of the renderer it selects. The font is written once to
`web/pkg/fonts/<sha256>.ttf`, and each renderer's `build.json` records its length and hash.

Deploy the whole `pkg/` directory. Before starting, the page verifies and installs the font, and it
refuses to start when the font is missing or does not match.

## Renderer

The page uses WebGPU and selects WebGL2 when WebGPU is not available. A start failure page offers
a reload, or a retry with `?renderer=webgl2`; each retry reloads the page.

## Server

The full-screen game connects to the server named by its `?server=` parameter and otherwise to the
public server `https://server.emptysekai.dev`. Each browser keeps a random user id in local
storage. The embedded stage runs the server's rules inside the page and needs no server address.

## Save ownership

Only one page at a time may write the save; other pages can browse. After the writing page closes,
reloading another page requests write access. Environments without Web Locks stay read-only.

## Development server

`node web/serve.mjs` listens on the loopback address only. It serves the embedded stage at
`/stage.html?assets=/assets/` and forwards `/player-api/{region}/{target_user_id}` for player data
import. The full-screen game is served by the `moly-web` site server from its `game` release
directory; its enter button enables audio inside the user's gesture.

The resource directory should be trusted and read-only, and the real target of every link inside it
must stay within the same mount root.

## Pack loading limits

In pack mode (`?packs=1`) the loader applies these limits:

| Limit | Value |
|---|---|
| Catalog or manifest | 16 MiB |
| One blob | 128 MiB |
| One decoded asset (pack payload) | 256 MiB |
| Decompressed size | at most 256 × the blob size + 64 KiB |
| Concurrent reads | 8 |
| Transfer and decode buffers, combined | 512 MiB |

Pack responses are read chunk by chunk and aborted when a limit or a timeout is hit. The budget
does not cover data the loader has already received, the parsed catalog, resources and scene
objects, GPU resources or the browser's own caches.

- A request that does not fit the remaining budget waits in request order; cancelling a read
  releases both its place in the queue and the budget it holds. A single request above a limit is
  refused at once.
- Identity-encoded data reserves only the transfer buffer; gzip reserves both the transfer and the
  decode buffer.
- The pack reader consumes in order and releases its budget when it is read to the end or dropped.
  A custom loader that needs to seek can fall back to Bevy's `VecReader`.
