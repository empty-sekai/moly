//! Which host owns this instance: the game, or one of the read-only hosts
//! (the content library, the isolated stage) that share the module.
//!
//! A game seats the server and the local documents; the other hosts seat
//! none, and the intents that only a game answers (the library and
//! player-data commands, the local document's writes) are refused from the
//! moment one owns the instance. The flag is decided once, by the host that
//! seats the documents, before the app is built; a host that seats none
//! leaves [`RunMode::game`] false.

use bevy::prelude::*;

/// Whether a game owns this instance.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct RunMode {
    pub game: bool,
}
