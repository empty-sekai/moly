//! Compatibility re-exports. The client side of the server model now lives in
//! [`crate::userdata`]; these keep the old `crate::server::client` paths
//! resolving.

pub(crate) use crate::userdata::{
    avatar, craft, enter_game_mode, home_action, housing_layout, instrument_env, inventory, music,
    music_play, system_fixture_action, talk_read,
};
