//! The native server process. The game connects to it over WebSocket.
//!
//! `MOLY_SERVER_ADDR` names the address to listen on (default
//! `127.0.0.1:15800`). The process prints one line
//! `moly-server listening on ws://<address>` once it accepts connections,
//! and exits with status 1 when it cannot listen.
//!
//! The asset root and the remote master bases are read as the native game
//! reads them (`MOLY_ASSET_ROOT`, `MOLY_ASSET_CATALOG`, the four base
//! variables), so both processes read the same masters. `MOLY_SERVER_DATA`
//! names the directory the users' documents are kept in; without it the
//! server keeps them in memory and a run starts fresh.

use std::path::PathBuf;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;

use moly_server::Store;

const DEFAULT_ADDRESS: &str = "127.0.0.1:15800";

/// The server's frame rate: its clock checks and replies run once a frame.
const FRAME: Duration = Duration::from_millis(10);

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
    let mut app = App::new();
    // The asset root's sources are registered before `AssetPlugin` builds the
    // server; the remote bases are admitted once the sources exist.
    moly_assets::install(&mut app, source);
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(FRAME)))
        .add_plugins(AssetPlugin::default())
        .add_plugins(LogPlugin::default());
    moly_assets::json::register(&mut app);
    moly_assets::remote::admit(&mut app, remote);
    app.insert_resource(store).add_plugins((
        moly_server::MasterLoadPlugin,
        moly_server::ServerPlugin,
    ));
    let bound = moly_server::ws::listen(&address, moly_server::hub(&app))
        .unwrap_or_else(|error| fail(&format!("cannot listen on {address}: {error}")));
    println!("moly-server listening on ws://{bound}");
    app.run();
}

fn fail(message: &str) -> ! {
    eprintln!("moly-server: {message}");
    std::process::exit(1);
}
