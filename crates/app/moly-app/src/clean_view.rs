//! The developer window's clean view for recordings: `MOLY_HIDE_HUD=1` hides
//! every HUD, button and dialog view, so only the scene, the in-world
//! balloons and the talk window remain.

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use moly_game::talk_window::TalkWindowView;
use moly_game::ui_layout::UiPrefabView;

/// Adds the clean view when `MOLY_HIDE_HUD` is `1`; `0` or unset leaves the
/// game's own interface untouched. `MOLY_HIDE_PLAYER=1` separately hides the
/// player's own body: the field camera stands where the player stands, so a
/// first-person shot would have the body (and its hair) fill the frame.
pub fn install(app: &mut App) -> Result<(), String> {
    let hide_player = match std::env::var("MOLY_HIDE_PLAYER").ok().as_deref() {
        None | Some("0") => false,
        Some("1") => true,
        Some(other) => return Err(format!("MOLY_HIDE_PLAYER must be 1 or 0, not {other:?}")),
    };
    if hide_player {
        app.add_systems(
            PostUpdate,
            hide_player_body.before(VisibilitySystems::VisibilityPropagate),
        );
    }
    match std::env::var("MOLY_HIDE_HUD").ok().as_deref() {
        None | Some("0") => Ok(()),
        Some("1") => {
            app.add_systems(
                PostUpdate,
                hide_views.before(VisibilitySystems::VisibilityPropagate),
            );
            Ok(())
        }
        Some(other) => Err(format!("MOLY_HIDE_HUD must be 1 or 0, not {other:?}")),
    }
}

/// The player's own body: a hidden root propagates to every mesh under it.
fn hide_player_body(mut players: Query<&mut Visibility, With<moly_game::player::PlayerControlled>>) {
    for mut visibility in &mut players {
        visibility.set_if_neq(Visibility::Hidden);
    }
}

fn hide_views(mut views: Query<&mut Visibility, (With<UiPrefabView>, Without<TalkWindowView>)>) {
    for mut visibility in &mut views {
        visibility.set_if_neq(Visibility::Hidden);
    }
}
