//! The game's server.
//!
//! The server keeps one document per user and answers the game's calls
//! ([`moly_protocol::Call`]). It runs as a headless Bevy app: natively as
//! its own process behind a WebSocket listener ([`ws`]), and in the browser
//! inside a worker. Transports reach it only through a [`hub::Hub`].
//!
//! The model itself ([`model`]) holds every server-decided value and the
//! rules over them. [`MasterLoadPlugin`] loads the master tables the model
//! reads (the host adds it with the asset source; a harness seats a
//! [`Masters`] fixture instead); [`users`] keeps one model per user and its
//! stored document; [`dispatch`] answers each call; [`admin`] serves the
//! control plane's document and schema views, an edit, and the seed install.

mod admin;
mod defaults;
mod dispatch;
mod hub;
mod instrument;
mod masters;
mod model;
mod users;

pub mod store;
#[cfg(not(target_arch = "wasm32"))]
pub mod ws;

pub use hub::Hub;
pub use masters::Masters;
pub use store::{DocumentStore, Store};
pub use moly_masters::clock::RefreshPeriod;
pub use moly_masters::StaminaMax;

use bevy::prelude::*;

/// Loads the master tables through the asset source. The host adds it after
/// `AssetPlugin` and the `moly://` asset source; a harness that seats a
/// [`Masters`] resource itself (see [`Masters::fixture`]) does not add it.
pub struct MasterLoadPlugin;

impl Plugin for MasterLoadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Masters>()
            .init_resource::<masters::Parties>();
        masters::install(app);
    }
}

/// The server: sessions, the users' documents and the API handlers.
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        instrument::install();
        hub::install(app);
        app.init_resource::<Masters>();
        app.init_resource::<masters::Parties>();
        defaults::install(app);
        if !app.world().contains_resource::<Store>() {
            app.insert_resource(Store::memory());
        }
        users::install(app);
        app.add_systems(
            Update,
            (
                dispatch::answer_calls,
                users::tick,
                users::seat_parties,
                users::flush_pushes,
                users::save_changed,
            )
                .chain(),
        );
    }
}

/// The transports' handle to a built app's server.
pub fn hub(app: &App) -> hub::Hub {
    app.world()
        .get_resource::<hub::Hub>()
        .cloned()
        .expect("ServerPlugin is installed")
}
