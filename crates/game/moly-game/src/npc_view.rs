//! The NPC view's per-frame work that the presenter's update calls: the
//! collision box center (presenter call 1) and the view update (call 2):
//! the automatic blink, then the mouth.
//!
//! - Collision box: the view's box center is set from the view's own
//!   transform position, once per presenter update; readers see the center
//!   of the last update, not the live transform.
//! - Blink: [`moly_law::blink`]; the eye cells go through the view's eye
//!   material (the pattern's open and close cells, read from the material's
//!   current pattern at each write). The view's blink flag starts true;
//!   `SetBlinkEnabled(bool)` only stores it. Its writers are a timeline's
//!   blink-state mixer (every evaluated frame: whether a clip of the track
//!   is active, see `fixture_activity_timeline`) and photo mode (not in
//!   this host). A timeline's eye-preset mixer changes the view's pattern
//!   (`ChangeEyePattern`: the eye table row by name, then its open cell),
//!   which the next blink write reads.
//! - Mouth: `UpdateMouth` starts a lip-sync cycle when none is playing. Its
//!   inputs are the view's lip-sync flag (set by the talk engine and a
//!   timeline's lip track), the attached voice's output analyzer level
//!   against the view's threshold (a missing analyzer passes), and the view's
//!   lip pattern (open, middle, close cells). It is driven by the voice
//!   meter in [`crate::voice_mouth`], which runs after the voice chain.
//!
//! Frame order: the presenter's update runs among the scripts' updates and
//! a game-time director evaluates after them, so the blink reads the flag
//! the director wrote on the previous frame, and an eye-preset clip's open
//! cell replaces a closed cell the blink wrote earlier in the same frame.
//! This host does not order the view update against the timeline's
//! advance (named gap; the order is declared where the presenter's calls
//! are scheduled). The draws are this host's presentation stream, not the
//! engine's shared generator.

use bevy::prelude::*;
use moly_law::blink::{Blink, EyePattern, RangeDraw};

use crate::alone_action_runtime::{apply_eye, Lcg};
use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::npc::CharacterUnitId;

/// The view's collision box center (x, z), written by presenter call 1.
#[derive(Component, Clone, Copy, Debug, Default)]
pub(crate) struct NpcViewBox {
    pub(crate) center: [f32; 2],
}

/// The view's blink state and its blink flag (`_isBlinkEnabled`, true from
/// construction).
#[derive(Component, Debug)]
pub(crate) struct NpcBlink {
    blink: Blink,
    enabled: bool,
}

impl Default for NpcBlink {
    fn default() -> Self {
        Self {
            blink: Blink::default(),
            enabled: true,
        }
    }
}

impl NpcBlink {
    /// `SetBlinkEnabled(bool)`: stores the flag, nothing else.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// The blink draws: one presentation stream shared by every view.
#[derive(Resource)]
pub(crate) struct BlinkRandom(Lcg);

impl Default for BlinkRandom {
    fn default() -> Self {
        Self(Lcg::seeded(0x0b11))
    }
}

impl RangeDraw for BlinkRandom {
    fn range(&mut self, min: i32, max: i32) -> i32 {
        let span = usize::try_from(max - min).expect("Random.Range has max > min here");
        min + moly_law::talk::UniformDraw::draw(&mut self.0, span) as i32
    }
}

/// Presenter call 1: `UpdateCollisionBox2D`.
pub(crate) fn update_collision_box(
    mut commands: Commands,
    mut npcs: Query<
        (Entity, &Transform, Option<&mut NpcViewBox>),
        (
            With<CharacterUnitId>,
            Without<crate::player::PlayerControlled>,
        ),
    >,
    gate: crate::npc_presenter::PresenterGate,
) {
    for (entity, transform, view_box) in &mut npcs {
        if !gate.runs(entity) {
            continue;
        }
        let center = [transform.translation.x, transform.translation.z];
        match view_box {
            Some(mut view_box) => view_box.center = center,
            None => {
                commands.entity(entity).insert(NpcViewBox { center });
            }
        }
    }
}

/// Presenter call 2: `UpdateView` = `UpdateBlink`, then `UpdateMouth`.
///
/// The blink's continuations (the writes after a completed delay) belong to
/// the frame start; this host runs them here, in the same pass, before the
/// update's start check, which is the same order within the frame.
#[allow(clippy::type_complexity)]
pub(crate) fn update_view(
    mut commands: Commands,
    clock: Res<crate::npc_clock::NpcClock>,
    mut random: ResMut<BlinkRandom>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &ToonMaterials,
            Option<&mut NpcBlink>,
        ),
        Without<crate::player::PlayerControlled>,
    >,
    gate: crate::npc_presenter::PresenterGate,
) {
    let dt = clock.delta();
    for (entity, unit, toon, blink) in &mut npcs {
        let Some(mut blink) = blink else {
            commands.entity(entity).insert(NpcBlink::default());
            continue;
        };
        if !gate.runs(entity) {
            continue;
        }
        let Some(handle) = toon.slot_handle("eye") else {
            continue;
        };
        // The view's pattern is set when its default face is applied; before
        // that nothing has run on this view.
        let Some(pattern) = materials
            .get(handle)
            .and_then(|material| material.eye_pattern)
        else {
            continue;
        };
        let enabled = blink.enabled;
        let frame = blink.blink.frame(dt, enabled, false, pattern, &mut *random);
        for cell in [frame.resumed, frame.started].into_iter().flatten() {
            apply_eye(&mut materials, handle, Some(&cell));
        }
        if frame.resumed.is_some() || frame.started.is_some() || frame.ended {
            trace!(
                "[npc-blink] unit={} resumed={:?} started={:?} ended={} playing={}",
                unit.0,
                frame.resumed,
                frame.started,
                frame.ended,
                blink.blink.is_playing()
            );
        }
    }
}

/// The pattern the view reads for a missing eye row: `FindBy`'s zero-valued
/// struct.
pub(crate) fn pattern_or_zero(pattern: Option<EyePattern>) -> EyePattern {
    pattern.unwrap_or_default()
}
