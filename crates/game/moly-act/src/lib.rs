//! The general actions every client shares: the player, the characters, the
//! companions, the camera and the view, effects, the site, the weather and
//! the server, registered with the hub's action registry.
//!
//! The vocabulary is a script's (`player.walk_to`, `npc.take`,
//! `companion.spawn`, `camera.talk_lens`, `server.edit`, ...), and this crate
//! is where each step's fields are read and each step is carried out: the
//! player through the hub's virtual hand, the characters through their own
//! navigation and AI, the companions through their command queue. A client
//! that runs actions installs the hub's state ([`moly_features::act::install`])
//! and then [`install`].
//!
//! The actions of other domains are registered by those domains: the layout
//! editor's `layout.*` by `moly-fixture-edit`, the harvest `player.gather`
//! and the companion's `companion.gather` by `moly-harvest`, the birthday
//! party's `player.deliver` by `moly-delivery`. The flow steps of a script
//! (`wait`, `wait_until`, `assert`, `log`, `where`, `capture`, `parallel`,
//! `repeat`) belong to the script runner, not here.

use bevy::prelude::*;
use moly_features::act::{ActCtx, ActPoll, ActionRun};

pub mod camera;
pub mod companion;
pub mod companion_build;
pub mod craft;
pub mod effect;
pub mod fields;
pub mod key;
pub mod npcs;
pub mod player;
pub mod server;
pub mod site;
pub mod view;
pub mod weather;

/// Registers the actions this crate carries.
pub fn install(app: &mut App) {
    player::install(app);
    npcs::install(app);
    companion::install(app);
    camera::install(app);
    view::register(app);
    effect::register(app);
    weather::register(app);
    site::register(app);
    server::register(app);
    key::install(app);
}

/// A request that ended on the frame it was sent: it ends with `note` on its
/// first poll.
#[derive(Debug)]
pub struct Requested {
    pub note: String,
}

impl ActionRun for Requested {
    fn poll(&mut self, _world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        ActPoll::Done(self.note.clone())
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}
