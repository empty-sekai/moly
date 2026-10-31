//! The clean view's switches from the configuration: `display.hide_hud`
//! (`MOLY_HIDE_HUD=1`) hides every HUD, button and dialog view, so only the
//! scene, the in-world balloons and the talk window remain;
//! `display.hide_player` (`MOLY_HIDE_PLAYER=1`) separately hides the player's
//! own body, so a first-person shot does not have the body (and its hair)
//! fill the frame. Each start reads them into the game's switches
//! ([`moly_game::clean_view`]), and the players and the plugins then set the
//! switches as they go (`view.set`).

use bevy::prelude::App;
use moly_config::keys;
use moly_game::clean_view::CleanView;

/// Reads both switches into the game; off or unset leaves the game's own
/// interface untouched, a malformed value is refused by name.
pub fn install(app: &mut App) -> Result<(), String> {
    let config = moly_config::global();
    let hide_hud = config.flag(&keys::HIDE_HUD).map_err(|error| error.to_string())?.unwrap_or(false);
    let hide_player = config.flag(&keys::HIDE_PLAYER).map_err(|error| error.to_string())?.unwrap_or(false);
    if let Some(mut view) = app.world_mut().get_resource_mut::<CleanView>() {
        view.set(Some(hide_hud), Some(hide_player));
    }
    Ok(())
}
