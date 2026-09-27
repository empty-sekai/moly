//! The world end of the field's tap chain: in single play a tap on the field
//! picks nothing.
//!
//! The source has no world tap on NPCs or fixtures in single play. The field
//! gesture layer publishes every gesture to the field event dispatcher, and
//! the whole program registers exactly nine handlers for that event (every
//! registration call site was counted in the native code, by the event type
//! it passes; the wrapper-object registrations carry other event types):
//!
//! 1. the field camera manager, at its setup: drag, two-finger drag and pinch
//!    move the camera; a tap is ignored;
//! 2. the field menu content, in the screenshot-only mode: a gesture shows
//!    the hide-UI button and the screenshot button again;
//! 3. the talk screen, in its screenshot mode: the same for the talk;
//! 4-6. the talk engine, from its two initializers and its debug
//!    initializer: taps advance or skip the talk while it plays;
//! 7. the layout editor: its own selection, in the edit mode only;
//! 8. the action buttons' presenter, while a fixture timeline action plays:
//!    a tap ends that action (the action buttons own this path);
//! 9. the multiplay player controller, once a multiplay room has made the
//!    player network objects: a tap casts a ray and opens the info of the
//!    nearest other player's avatar. It never runs in single play, and it
//!    looks for player avatars, never NPCs or fixtures.
//!
//! None of them picks an NPC or a fixture. The field's other physics queries
//! (camera collision, drop landing, the paper airplane, the avatar's ground
//! probe, the talk camera, the edit tiles) are not tap picks either. Talk and
//! fixture actions start only from the proximity action buttons, whose press
//! writes the talk request (see the action button module). So there is no
//! pick radius to read and no fixture tap to port: a pick here would be a
//! second, non-source way to start a talk.
//!
//! What stays: this system is the tail of the tap order (the screen buttons,
//! the dialogs and the edit pointer run before it), and it logs a field tap
//! that nothing took, so a run can show that such a tap starts nothing.
//!
//! Instrument: `MOLY_PICK_NPC_TAP_SECS` (off by default; read through the
//! instrument environment, which game mode never reads) synthesizes a tap on
//! each present NPC's screen position in turn, to show that a tap on an NPC
//! starts no talk.

use bevy::prelude::*;

use crate::action_button::ActionTapConsumed;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::npc::CharacterUnitId;
use crate::player::PlayerControlled;
use crate::server::client::instrument_env;

/// The instrument's tap interval, seconds.
const SMOKE_TAP_INTERVAL: f32 = 0.8;

/// Update (the tail of the tap chain): read the field taps and log those no
/// earlier consumer took. Nothing is picked (module head).
pub(crate) fn pick(
    mut gestures: MessageReader<GestureEvent>,
    buttons: Res<ButtonInput<MouseButton>>,
    consumed: Res<ActionTapConsumed>,
) {
    // The reader advances even for a consumed tap, so a tap is never read
    // again in a later frame.
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| {
            event.kind == GestureKind::Tap && event.state == GestureState::End && !event.ui_owned
        })
        .map(|event| event.position)
        .collect();
    if consumed.0 {
        return;
    }
    for position in taps {
        info!(
            "[pick] tap ({:.0},{:.0}) on the field: no world pick in single play (none of the nine gesture handlers picks an NPC or a fixture); nothing happens",
            position.x, position.y
        );
    }
    if buttons.just_released(MouseButton::Right) {
        info!(
            "[pick] right button release on the field: no world pick in single play; nothing happens"
        );
    }
}

/// Update (in the gesture chain, before [`pick`]): the NPC tap instrument
/// (`MOLY_PICK_NPC_TAP_SECS`). Within the window, every
/// [`SMOKE_TAP_INTERVAL`] seconds it projects one present NPC's world
/// position to the screen and injects a TAP end there, through the same
/// gesture events real input uses. Targets rotate in entity order. A tap that
/// lands on a screen button is that button's.
pub(crate) fn smoke_npc_autotap(
    mut events: MessageWriter<GestureEvent>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    npcs: Query<(Entity, &Transform, &CharacterUnitId), Without<PlayerControlled>>,
    time: Res<Time>,
    mut next_at: Local<f32>,
    mut index: Local<usize>,
) {
    let armed = env_secs("MOLY_PICK_NPC_TAP_SECS");
    if armed <= 0.0 || time.elapsed_secs() >= armed || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + SMOKE_TAP_INTERVAL;
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let mut targets: Vec<(Entity, Vec3, u32)> = npcs
        .iter()
        .map(|(entity, transform, unit)| (entity, transform.translation, unit.0))
        .collect();
    targets.sort_by(|a, b| a.0.cmp(&b.0));
    if targets.is_empty() {
        return;
    }
    let (_, world, unit) = targets[*index % targets.len()];
    *index += 1;
    let Ok(position) = camera.world_to_viewport(camera_transform, world) else {
        info!("[pick-smoke] unit {unit}: its world position is outside the viewport; skipped");
        return;
    };
    events.write(GestureEvent {
        kind: GestureKind::Tap,
        state: GestureState::End,
        position,
        delta: Vec2::ZERO,
        ui_owned: false,
    });
    info!(
        "[pick-smoke] TAP injected at ({:.0},{:.0}) on unit {unit}'s screen position",
        position.x, position.y
    );
}

/// An instrument's seconds (0 when unset or in game mode).
fn env_secs(name: &str) -> f32 {
    instrument_env(name)
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
