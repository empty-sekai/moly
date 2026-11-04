//! The ragdoll as the game runs it: the crate's plugin on the game's frame,
//! the scripts' fall requests, and the `moly.ragdoll.*` methods of the control
//! surface.
//!
//! The crate holds the description, the drop, the write-back, the settle, the
//! recovery of one character and the cleanup after a drop another state takes
//! away; this module puts them in the game's frame order and lets the hub
//! ([`moly_features::act`]) drop a character when a script step asks it to
//! fall.

use bevy::prelude::*;

use moly_features::act::FallRequests;

/// Installs the ragdoll: its plugin, the hub's fall requests and the control
/// surface's methods.
pub fn install(app: &mut App) {
    app.init_resource::<FallRequests>();
    app.world_mut().insert_resource(moly_ragdoll::fall_requests());
    moly_ragdoll::install(app);
}
