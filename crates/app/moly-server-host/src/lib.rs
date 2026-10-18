//! Building the game's server into a host process.
//!
//! [`build`] assembles the headless server app and binds the player socket on
//! the calling thread, returning the app to run. [`spawn`] does the same on a
//! named thread and hands the bound address back, for a host that keeps the
//! server in its own process instead of running `moly-server` beside it.
//!
//! The host reads the environment and names the asset source, the remote
//! bases and the document store; this crate only assembles them.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_remote::http::RemoteHttpPlugin;

use moly_server::{ListenAddress, Store};

/// The server's frame rate: its clock checks and replies run once a frame.
const FRAME: Duration = Duration::from_millis(10);

/// What one server start serves and reads.
pub struct Options {
    /// The player socket's address (`host:port`). Port 0 asks the system for
    /// a free port, which [`build`] and [`spawn`] report.
    pub address: String,
    /// The loopback control port; `None` serves no control plane and adds no
    /// HTTP transport.
    pub control_port: Option<u16>,
    /// The asset source the masters are loaded from.
    pub asset_source: moly_assets::AssetSource,
    /// The four remote bases the server may read.
    pub remote: moly_assets::remote::RemoteBases,
    /// Where the users' documents are kept.
    pub store: Store,
    /// Whether to add Bevy's [`LogPlugin`]. A host that installs its own
    /// logger passes `false`: one process has one global subscriber.
    pub log: bool,
}

/// Assemble the server app and bind its player socket on the calling thread.
///
/// The returned app has not run yet; the caller runs it. The address is the
/// one the socket actually bound, so port 0 names the system's free port.
pub fn build(options: Options) -> Result<(App, SocketAddr), String> {
    let Options {
        address,
        control_port,
        asset_source,
        remote,
        store,
        log,
    } = options;
    let mut app = App::new();
    // The asset root's sources are registered before `AssetPlugin` builds the
    // server; the remote bases are admitted once the sources exist.
    moly_assets::install(&mut app, asset_source);
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(FRAME)))
        .add_plugins(AssetPlugin::default());
    if log {
        app.add_plugins(LogPlugin::default());
    }
    moly_assets::json::register(&mut app);
    moly_assets::remote::admit(&mut app, remote);
    app.add_plugins(moly_server::remote_plugin());
    if let Some(port) = control_port {
        app.add_plugins(
            RemoteHttpPlugin::default()
                .with_address(Ipv4Addr::LOCALHOST)
                .with_port(port),
        );
    }
    app.insert_resource(store)
        .add_plugins((moly_server::MasterLoadPlugin, moly_server::ServerPlugin));
    let bound = moly_server::ws::listen(&address, moly_server::hub(&app))
        .map_err(|error| format!("cannot listen on {address}: {error}"))?;
    app.insert_resource(ListenAddress(bound));
    Ok((app, bound))
}

/// Assemble the server on a named thread and run it there, returning the
/// address it bound. The thread runs the app for the rest of the process.
pub fn spawn(options: Options) -> Result<SocketAddr, String> {
    let (ready, bound) = std::sync::mpsc::channel();
    // The join handle is dropped: the thread runs the server for the rest of
    // the process rather than for a scoped interval.
    let _server = std::thread::Builder::new()
        .name("moly-server-app".to_owned())
        .spawn(move || match build(options) {
            Ok((mut app, bound_address)) => {
                let _ = ready.send(Ok(bound_address));
                app.run();
            }
            Err(error) => {
                let _ = ready.send(Err(error));
            }
        })
        .map_err(|error| format!("the server thread cannot start: {error}"))?;
    bound.recv().unwrap_or_else(|_| {
        Err("the server thread ended before it bound its socket".to_owned())
    })
}
