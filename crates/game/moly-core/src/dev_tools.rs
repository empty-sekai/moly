//! Developer scaffolding switch. The native window and the legacy browser
//! pages insert [`DevTools`]; the browser game never does. Every developer
//! surface that has no counterpart in the game registers behind
//! [`dev_tools`], or asks [`installed`] when it is a plain function.

use bevy::prelude::*;
use moly_config::Key;
use std::sync::atomic::{AtomicBool, Ordering};

/// Present while developer scaffolding is part of this application.
#[derive(Resource)]
pub struct DevTools;

/// Mirror of the resource for code that runs without a World borrow.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Every surface gated by [`dev_tools`], named in the game-mode startup line.
pub const GATED: [&str; 6] = [
    "content library QA auto-open",
    "fixture gallery preview rows",
    "player-data UI",
    "site Tab key",
    "site digit keys",
    "weather C key",
];

/// Called before `App::run`; the flag never changes in a running world.
pub fn insert_dev_tools(app: &mut App) {
    app.insert_resource(DevTools);
    INSTALLED.store(true, Ordering::Relaxed);
}

/// Run condition of every gated registration.
pub fn dev_tools(tools: Option<Res<DevTools>>) -> bool {
    tools.is_some()
}

/// Gate for plain functions that have no World access.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// A native instrument's time window from the text of its key (a developer
/// switch, read through the configuration): `<until>` (from the start) or
/// `<from>:<until>`, in seconds of app time. `None` when unset or not numbers.
pub fn instrument_window(key: &'static Key) -> Option<(f32, f32)> {
    let raw = moly_config::text(key)?;
    let secs = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .map(|value| value.max(0.0) as f32)
    };
    match raw.split_once(':') {
        Some((from, until)) => Some((secs(from)?, secs(until)?)),
        None => Some((0.0, secs(&raw)?)),
    }
}

/// Whether `now` lies inside the instrument window of `key`.
pub fn instrument_open(key: &'static Key, now: f32) -> bool {
    instrument_window(key).is_some_and(|(from, until)| from <= now && now < until)
}

static GAME_MODE: AtomicBool = AtomicBool::new(false);

/// The page's game mode begins: every native instrument is off from here on.
pub fn enter_game_mode() {
    GAME_MODE.store(true, Ordering::Relaxed);
}

/// The text of a native instrument's developer-switch key. Game mode reads
/// none: every instrument is off there, and the server document is the only
/// input.
pub fn instrument_env(key: &'static Key) -> Option<String> {
    if GAME_MODE.load(Ordering::Relaxed) {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = key;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        moly_config::text(key)
    }
}
