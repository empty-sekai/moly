//! The native server process. The game connects to it over WebSocket.
//!
//! `MOLY_SERVER_ADDR` names the address to listen on (default
//! `127.0.0.1:15800`). The process prints one line
//! `moly-server listening on ws://<address>` once it accepts connections,
//! and exits with status 1 when it cannot listen.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;

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
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(FRAME)))
        .add_plugins(LogPlugin::default())
        .add_plugins(moly_server::ServerPlugin);
    let bound = moly_server::ws::listen(&address, moly_server::hub(&app))
        .unwrap_or_else(|error| fail(&format!("cannot listen on {address}: {error}")));
    println!("moly-server listening on ws://{bound}");
    app.run();
}

fn fail(message: &str) -> ! {
    eprintln!("moly-server: {message}");
    std::process::exit(1);
}
