//! The client side of the server model: the copies of server data the client
//! reads, and the requests it sends. It stands where the source's
//! `UserDataManager` copies and API calls stand, apart from the server.
//!
//! These files depend only on std, bevy and serde_json, never on the server
//! model, the world or the session. The server model at the top of the crate
//! is their only writer: its responses set the copies, and it installs the
//! endpoints the requests run through. A consumer imports from here only.
//!
//! - [`music`]: `UserMysekaiMusicPlayFixtureSetting` rows.
//! - [`avatar`]: `UserAvatar` and the avatar wear masters.
//! - [`home_action`]: the craft, canvas and sketch requests.
//! - [`craft`]: the craft masters, the client's craft checks and the craft
//!   and sketch requests with their `SuiteUser` replies.
//! - [`inventory`]: the owned MySekai tables and the possession masters.
//! - [`housing_layout`]: the layout editor's save request.
//! - [`talk_read`]: the talk read report and its reply.
//! - [`instrument_env`]: the native instruments' environment variables,
//!   which game mode never reads.
//! - [`music_play`]: the owned `UserMysekaiMusicRecord` rows and the music
//!   player's set and eject requests.
//! - [`system_fixture_action`]: the `UserMysekaiSystemFixtureAction` rows
//!   and the system fixture action request.

pub(crate) mod avatar;
pub(crate) mod craft;
pub(crate) mod home_action;
pub(crate) mod inventory;
pub(crate) mod housing_layout;
pub(crate) mod music;
pub(crate) mod music_play;
pub(crate) mod system_fixture_action;
pub(crate) mod talk_read;

use std::sync::atomic::{AtomicBool, Ordering};

static GAME_MODE: AtomicBool = AtomicBool::new(false);

/// The page's game mode begins: every native instrument is off from here on.
/// The browser game's storage install calls it through the server model's
/// seed install, before it marks the module instance as a game.
pub(crate) fn enter_game_mode() {
    GAME_MODE.store(true, Ordering::Relaxed);
}

/// The environment variable of a native instrument. Game mode reads none:
/// every instrument is off there, and the one server document is the only
/// input.
pub(crate) fn instrument_env(name: &str) -> Option<String> {
    if GAME_MODE.load(Ordering::Relaxed) {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var(name).ok()
    }
}
