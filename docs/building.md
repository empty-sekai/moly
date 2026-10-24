# Building from source

## Requirements

- Stable Rust.
- Node.js 22 (for the browser build, the development server and the MCP server).
- An extracted resource directory, set with `MOLY_ASSET_ROOT`. If it is not set, the resources installed by the launcher are used.

Build output is written to `target/` in the repository by default. To use a shared cache, set `CARGO_TARGET_DIR`.

## Native program

The native version consists of two processes. The server `moly-server` (crate `moly-server-host`) stores player documents and answers requests, and the game `moly-app` sends it binary-encoded requests over HTTP. Start the server first, then the game:

```sh
cargo run --release -p moly-server-host
cargo run --release -p moly-app
```

- By default, the server listens on `127.0.0.1:15800`, and its control port is `127.0.0.1:15801`. When it is ready, it prints one line with its address. If a port is already in use, it exits with status 1.
- The game connects to the server at `MOLY_SERVER_URL`. If the connection fails, the game shows a specific message and does not fall back to an in-process implementation.
- The server and the game read the same resource root and master table variables, so they see the same master tables.

By default, the native program runs the same game as the web version: it starts from the home, and the home level comes from the server's player document. To start the developer window, set `MOLY_DEV=1`. It reads the variables that [configuration](configuration.md) marks as "developer window" variables. The server's player documents are kept only in memory, and the developer window provides development tools such as player data import.

| Key | Action |
|---|---|
| **F10** | Graphics and volume settings |
| **F9** | 控制面板 (Control panel) |

## Browser version

You need the `wasm32-unknown-unknown` target and a wasm-bindgen CLI that matches `Cargo.lock`:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
node web/build-wasm.mjs
node web/serve.mjs
```

The build script generates `web/pkg/webgpu/` and `web/pkg/webgl2/` separately. The page downloads only the module for the selected backend, and WebGL2 is selected when WebGPU is not available. The development server listens only on the loopback address and serves the embedded stage at `/stage.html?assets=/assets/`.

- Build outputs, backend selection, write access for saves, and resource loading limits: [web/RUNTIME.md](../web/RUNTIME.md)
- Embedding the stage in an existing site: [web/EMBEDDING.md](../web/EMBEDDING.md)
- Resource sources: [web/RESOURCE-SOURCES.md](../web/RESOURCE-SOURCES.md)
- Browser checks: [web/TESTING.md](../web/TESTING.md)

## Android

> The Android version currently crashes shortly after it starts rendering frames on a physical device, so it is not yet usable. See [#2](https://github.com/empty-sekai/moly/issues/2).

`android/` is the Android project. The native library (crate `moly-android`, target `arm64-v8a`) is built with `cargo ndk`, and Gradle packages the APK. The game connects to a separately running server. On first launch, it downloads resources from the public resource library into the app's private cache.

```sh
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p moly-android --release
cd android && ./gradlew :app:assembleRelease
```

For the SDK, NDK and JDK versions, the server address and how to view logs, see [android/README.md](../android/README.md).
