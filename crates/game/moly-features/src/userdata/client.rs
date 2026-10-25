//! The client copies of the server's user data, under the paths the screens
//! use (`crate::userdata::client::inventory`, `...::music_play`, ...).
//!
//! The copies themselves live in this module's siblings (`crate::userdata`
//! `inventory`, `music`, `music_play`, `craft`, `avatar`, `tools`, `phenomena`,
//! `local`, `birthday_party`, `home_action`, `housing_layout`,
//! `system_fixture_action`, `talk_read`); this module names them as one tree
//! and re-exports the shared client items (the instrument reader, the copies
//! that have no submodule, the talk-list queue and the rank message).

pub use crate::userdata::{
    avatar, birthday_party, check_seed_local, client_update_talk_list, craft,
    home_action, housing_layout, instrument_env, inventory, local, music, music_play,
    phenomena, system_fixture_action, talk_read, tools, ClientTalkListUpdates, ClientUserData, LiveSchedule,
    RankDelivered, ServerGateReplies, TalkListUpdate,
};
