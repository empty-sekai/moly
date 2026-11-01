//! The player's input routing: the keyboard adaptation and the joystick face
//! composed into the player's camera-relative move direction. It reads the
//! features' gates (the layout editor, the settings panel, the content
//! library, a site move, the entry) because those are what take the field's
//! input away, so the routing lives above them.
//!
//! Both input faces coexist, with the stick taking priority:
//! `GetMoveDirection` reads only the stick face (the `_joyStickMoveDirection`
//! baked by `OnTouchJoyStick`: active on press, with a zero vector until the
//! drag passes the threshold — see [`crate::joystick`]). The keyboard is a
//! named desktop adaptation: the WASD ratio goes through the stick's
//! `HandleInput` clamp and then the stick's bake, and is used only while the
//! stick is inactive. Dash mode is not switched here (the HUD dash button is).
//!
//! The gates it reads (layout edit, settings panel, content library, site
//! move, entry) are the domains that take the field's input away, so the
//! routing lives above them.

use bevy::prelude::*;

use crate::player::{DashMode, PlayerControlled, PlayerInput};

/// Update: reads the input faces and composes the camera-relative move
/// direction.
///
/// Smoke hook (`dev.player_autowalk_secs`, `MOLY_PLAYER_AUTOWALK_SECS`, a
/// host-side instrument, like the emoticon's `MOLY_EMOTICON_SHOW_SECS`): when
/// set to a positive number, a fixed input is composed for that many seconds
/// after startup (a walk half then a dash half), for unattended checks of the
/// movement rules — the keyboard face receives no events without window
/// focus, so the check of the real input face remains a manual user test.
/// `dev.player_autowalk_diagonal` (`MOLY_PLAYER_AUTOWALK_DIAGONAL`), when
/// positive, switches the direction to the normalized diagonal (1,0,1)/√2.
/// Reading and composing the input (the keyboard and stick faces) is consumed
/// only by this repository's schedule.
pub fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut players: Query<(&mut PlayerInput, &mut DashMode), With<PlayerControlled>>,
    time: Res<Time>,
    edits: Res<crate::field_modes::EditSessionActive>,
    joystick: Res<crate::joystick::JoystickState>,
    overlays: crate::game_settings::OverlayPanels,
    claims: Res<crate::input_claims::InputClaims>,
    site_move: Option<Res<crate::field_modes::SiteMoveActive>>,
    entry: Option<Res<crate::entry::EntrySequence>>,
    learn: Option<Res<crate::field_modes::LearnSiteEnvironmentActive>>,
    held: Option<Res<crate::field_modes::HeldGameState>>,
    screens: Option<Res<crate::ui_layers::ScreenManager>>,
    tutorial: Option<Res<crate::tutorial::TutorialInput>>,
) {
    // While the layout edit face holds input (in edit mode the gesture layer
    // and the stick belong to the edit face, the _joyStickCanvasGroup of
    // ScreenLayerMysekaiCommon), player movement yields.
    // GameState SiteMove disables the gesture layer and taps as well.
    // The keyboard adaptation follows the joystick: off until the entry's Finish.
    // A dialog over the field takes the field's input, as it takes the stick's
    // new presses (ExistsDialog(Layer_Dialog)), so the adaptation is off too.
    if tutorial.is_some_and(|input|input.movement_blocked) || edits.is_active()
        || overlays.block_world_input()
        || claims.blocks(crate::input_claims::InputChannel::Exploration)
        || site_move.is_some()
        || screens.as_deref().is_some_and(crate::ui_layers::ScreenManager::exists_dialog)
        // GameState LearnSiteEnvironment hides the stick.
        || learn.is_some()
        // GameState CutScene / LevelUpMyRoomSite disable the stick.
        || held.is_some()
        || !crate::entry::control_open(entry.as_deref())
    {
        for (mut input, _) in &mut players {
            input.active = false;
            input.direction = Vec3::ZERO;
        }
        return;
    }
    let vertical = if keys.pressed(KeyCode::KeyW) {
        1.0
    } else if keys.pressed(KeyCode::KeyS) {
        -1.0
    } else {
        0.0
    };
    let horizontal = if keys.pressed(KeyCode::KeyD) {
        1.0
    } else if keys.pressed(KeyCode::KeyA) {
        -1.0
    } else {
        0.0
    };
    let active = vertical != 0.0 || horizontal != 0.0;
    // The adaptation's stick ratio (x right, y up), clamped by HandleInput
    // and baked on the main camera exactly as a stick UPDATE is. Without a
    // camera the source's bake does not run and the input stays off.
    let baked = cameras.single().ok().map(|global| {
        crate::joystick_law::bake(
            crate::joystick_law::handle_input(Vec2::new(horizontal, vertical)),
            global,
        )
    });
    let (direction, active) = match baked {
        Some(vector) if active => (vector, true),
        _ => (Vec3::ZERO, false),
    };
    // Smoke hook: without window focus the keyboard face receives no events,
    // so when the variable is set a fixed input drives the first half (walk)
    // and the second half (dash) to check that the movement rules really move.
    // The direction is +X by default; dev.player_autowalk_diagonal
    // (MOLY_PLAYER_AUTOWALK_DIAGONAL) set to a positive number switches to the
    // **normalized** diagonal (1,0,1)/√2
    // (length 1, the same cap as HandleInput's length — this only varies the
    // direction; a deliberately unnormalized "diagonal walks about 40% faster"
    // would be wrong: the InputVec length never exceeds 1, the clamping in the
    // animation rate happens against the gathering field's top speed 2.0→1.8,
    // and the diagonal needs no inflation).
    let autowalk_secs = match moly_config::text(&moly_config::keys::DEV_PLAYER_AUTOWALK_SECS) {
        Some(raw) => raw.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as f32,
        None => 0.0,
    };
    let diagonal = match moly_config::text(&moly_config::keys::DEV_PLAYER_AUTOWALK_DIAGONAL) {
        Some(raw) => raw.trim().parse::<f64>().unwrap_or(0.0) > 0.0,
        None => false,
    };
    let smoke_direction = if diagonal {
        Vec3::new(1.0, 0.0, 1.0).normalize()
    } else {
        Vec3::X
    };
    let smoke = if autowalk_secs > 0.0 && time.elapsed_secs() < autowalk_secs {
        Some(time.elapsed_secs() < autowalk_secs * 0.5)
    } else {
        None
    };
    for (mut input, mut dash) in &mut players {
        if let Some(first_half) = smoke {
            // Smoke input: a fixed world direction, walk in the first half and
            // dash in the second — the speed difference is visible in the log
            // lines (displacement over time).
            if first_half && dash.0 {
                dash.0 = false;
            } else if !first_half && !dash.0 {
                dash.0 = true;
            }
            input.direction = smoke_direction;
            input.active = true;
        } else if joystick.active {
            // Stick priority: the movement chain reads only the stick face
            // (GetMoveDirection → _joyStickMoveDirection); the keyboard is a
            // named desktop adaptation.
            input.direction = joystick.move_vector;
            input.active = true;
        } else {
            input.direction = direction;
            input.active = active;
        }
    }
}

