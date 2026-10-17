//! Compatibility re-exports. The client side of the server model lives in
//! this crate's `userdata` module; these keep the old `crate::server::client`
//! paths resolving for the screens that import them.

pub(crate) use crate::userdata::{
    avatar, birthday_party, check_seed_local, client_update_talk_list, craft, enter_game_mode,
    home_action, housing_layout, instrument_env, inventory, local, music, music_play, npc_slice,
    system_fixture_action, talk_read, tools, ClientTalkListUpdates, ClientUserData, LiveSchedule,
    RankDelivered, ServerGateReplies, TalkListUpdate,
};
