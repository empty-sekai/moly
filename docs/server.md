# Server

The server `moly-server` (crate `moly-server-host`) stores player documents and answers requests from the game. It is included in each platform's package. A server-only package, `moly-server-<version>-<platform>`, is also available for running the server on a separate machine.

The server and the game read the same resources. The listen address, resource directory and save directory are set with environment variables. For the list of variables, see [Environment variables](configuration.md#environment-variables). For the commands to build and start the server from source, see [Native program](building.md#native-program).

- By default, the server listens on `127.0.0.1:15800`, and its control port is `127.0.0.1:15801`. When it is ready, it prints one line with its address. If a port is already in use, it exits with status 1.
- The game connects to the server at `MOLY_SERVER_URL`. If the connection fails, it shows a specific message and does not fall back to an in-process implementation.
- The server and the game read the same resource root and master table variables, so they see the same master tables.
