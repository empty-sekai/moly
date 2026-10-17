//! The game's server.
//!
//! The server keeps one document per user and answers the game's calls
//! ([`moly_protocol::Call`]). It runs as a headless Bevy app: natively as
//! its own process behind a WebSocket listener ([`ws`]), and in the browser
//! inside a worker. Transports reach it only through a [`hub::Hub`].

pub mod hub;
#[cfg(not(target_arch = "wasm32"))]
pub mod ws;

use bevy::prelude::*;
use moly_protocol::{Call, Reply, Sections, ServerMessage};

use hub::{CallReceived, Sessions};

/// The server: sessions and the API handlers.
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        hub::install(app);
        app.add_systems(Update, answer);
    }
}

/// The transports' handle to a built app's server.
pub fn hub(app: &App) -> hub::Hub {
    app.world()
        .get_resource::<hub::Hub>()
        .cloned()
        .expect("ServerPlugin is installed")
}

/// The server clock in milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn answer(mut calls: MessageReader<CallReceived>, sessions: Res<Sessions>) {
    for call in calls.read() {
        let reply = match call.call {
            Call::Join => Reply::Joined,
        };
        sessions.send(
            call.session,
            &ServerMessage::Reply {
                seq: call.seq,
                outcome: Ok(reply),
                sections: Some(Box::new(Sections {
                    server_date_ms: now_ms(),
                    is_refreshed: false,
                })),
            },
        );
    }
}
