//! The clean view's switches from the environment: `MOLY_HIDE_HUD=1` hides
//! every HUD, button and dialog view, so only the scene, the in-world
//! balloons and the talk window remain; `MOLY_HIDE_PLAYER=1` separately
//! hides the player's own body, so a first-person shot does not have the body
//! (and its hair) fill the frame. Each start reads them into the game's
//! switches ([`moly_game::clean_view`]), and the players and the plugins then
//! set the switches as they go (`view.set`).

use bevy::prelude::App;
use moly_game::clean_view::{switch, CleanView};

/// Reads both switches into the game; `0` or unset leaves the game's own
/// interface untouched, anything else is refused by name.
pub fn install(app: &mut App) -> Result<(), String> {
    let hide_hud = switch("MOLY_HIDE_HUD")?;
    let hide_player = switch("MOLY_HIDE_PLAYER")?;
    if let Some(mut view) = app.world_mut().get_resource_mut::<CleanView>() {
        view.set(Some(hide_hud), Some(hide_player));
    }
    Ok(())
}
