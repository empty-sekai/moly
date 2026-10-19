//! Building the game's server into a host process.
//!
//! [`build`] assembles the headless server app and binds the HTTP endpoint on
//! the calling thread, returning the app to run. The game reaches the endpoint
//! at `POST <server>/rpc`.
//!
//! A host reads the environment and names the asset source, the remote bases
//! and the document store; this crate only assembles them.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy_remote::http::RemoteHttpPlugin;

use moly_server::{ListenAddress, Store};

/// The server's frame rate: its clock checks and responses run once a frame.
const FRAME: Duration = Duration::from_millis(10);

/// What one server start serves and reads.
pub struct Options {
    /// The HTTP endpoint's address (`host:port`). Port 0 asks the system for
    /// a free port, which [`build`] reports.
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
    /// Whether the administration calls are accepted on a loopback listen
    /// address. A server behind a reverse proxy passes `false`.
    pub admin: bool,
}

/// Assemble the server app and bind its HTTP endpoint on the calling thread.
///
/// The returned app has not run yet; the caller runs it. The address is the
/// one the endpoint actually bound, so port 0 names the system's free port.
pub fn build(options: Options) -> Result<(App, SocketAddr), String> {
    let Options {
        address,
        control_port,
        asset_source,
        remote,
        store,
        log,
        admin,
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
    let bound = moly_server::http::listen(&address, moly_server::hub(&app))
        .map_err(|error| format!("cannot listen on {address}: {error}"))?;
    app.insert_resource(ListenAddress(bound));
    if !admin {
        app.insert_resource(moly_server::AdminClosed);
    }
    Ok((app, bound))
}
