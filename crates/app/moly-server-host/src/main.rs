//! The native server process. The game connects to it over WebSocket; a
//! control plane reaches it over HTTP JSON-RPC on its own loopback port.
//!
//! `MOLY_SERVER_ADDR` names the player socket's address (default
//! `127.0.0.1:15800`). `MOLY_SERVER_CONTROL_PORT` names the control port
//! (default 15801), always bound to `127.0.0.1`. The process prints one ready
//! line naming both addresses, and exits with status 1 when it cannot listen.
//!
//! The asset root and the remote master bases are read as the native game
//! reads them (`MOLY_ASSET_ROOT`, `MOLY_ASSET_CATALOG`, the four base
//! variables), so both processes read the same masters. `MOLY_SERVER_DATA`
//! names the directory the users' documents are kept in; without it the
//! server keeps them in memory and a run starts fresh.
//!
//! The assembly itself lives in the crate's library, so a host can run the
//! same server inside its own process; this binary keeps the environment
//! handling, the ready line and the exit codes.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;

use moly_server::Store;
use moly_server_host::Options;

const DEFAULT_ADDRESS: &str = "127.0.0.1:15800";
const DEFAULT_CONTROL_PORT: u16 = 15801;

fn main() {
    if std::env::args()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "--version" | "-V"))
    {
        println!("moly-server {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let address = match std::env::var("MOLY_SERVER_ADDR") {
        Ok(address) if !address.trim().is_empty() => address,
        Ok(_) | Err(std::env::VarError::NotPresent) => DEFAULT_ADDRESS.to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => fail("MOLY_SERVER_ADDR is not valid text"),
    };
    let control_port = match std::env::var("MOLY_SERVER_CONTROL_PORT") {
        Ok(value) if value.trim().is_empty() => fail(
            "MOLY_SERVER_CONTROL_PORT is empty; it names the control port",
        ),
        Ok(value) => value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .unwrap_or_else(|| {
                fail(&format!(
                    "MOLY_SERVER_CONTROL_PORT={value:?} is not a port"
                ))
            }),
        Err(std::env::VarError::NotPresent) => DEFAULT_CONTROL_PORT,
        Err(std::env::VarError::NotUnicode(_)) => {
            fail("MOLY_SERVER_CONTROL_PORT is not valid text")
        }
    };
    let control_address = SocketAddr::from((Ipv4Addr::LOCALHOST, control_port));
    // The transport binds the port when the app starts; bind it here first so
    // a taken port is a named failure instead of a silent one.
    match TcpListener::bind(control_address) {
        Ok(listener) => drop(listener),
        Err(error) => fail(&format!("cannot listen on {control_address}: {error}")),
    }
    let source = moly_assets::asset_source::resolve()
        .unwrap_or_else(|error| fail(&format!("the asset root: {error}")));
    let remote = moly_assets::asset_source::resolve_remote()
        .unwrap_or_else(|error| fail(&format!("the remote bases: {error}")));
    let store = match std::env::var("MOLY_SERVER_DATA") {
        Ok(directory) if directory.trim().is_empty() => {
            fail("MOLY_SERVER_DATA is empty; it names the directory the documents are kept in")
        }
        Ok(directory) => Store::file(PathBuf::from(directory)),
        Err(std::env::VarError::NotPresent) => Store::memory(),
        Err(std::env::VarError::NotUnicode(_)) => fail("MOLY_SERVER_DATA is not valid text"),
    };
    let options = Options {
        address,
        control_port: Some(control_port),
        asset_source: source,
        remote,
        store,
        log: true,
    };
    let (mut app, bound) = match moly_server_host::build(options) {
        Ok(built) => built,
        Err(message) => fail(&message),
    };
    println!("moly-server ready: control http://{control_address}, listening on ws://{bound}");
    app.run();
}

fn fail(message: &str) -> ! {
    eprintln!("moly-server: {message}");
    std::process::exit(1);
}
