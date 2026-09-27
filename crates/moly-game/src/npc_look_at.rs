//! The view's `DoLookAt` tween on an NPC (see [`moly_law::path::look_at`]).
//!
//! The greeting state's enter starts it towards the player's position at
//! that moment, for 1.0 s with a linear ease, and does not wait for it. The
//! tween reads the character's position and rotation at its first update
//! (the frame after its start), then writes the rotation every update until
//! the duration; a later look-at on the same character replaces it. The
//! character stands while it turns; a move that starts meanwhile ends it.
//!
//! The presenter starts a turn clip with the tween, chosen from the signed
//! angle between the character's forward and the target point itself
//! (`look_at_turn_motion`), and plays the idle clip (the character's talk
//! motion idle) when the tween ends: the motion driver plays the turn clip
//! while the look-at is in flight and the idle clip after it. Not played
//! here: the cloth reset after the idle clip, and the wait on that idle
//! clip's change (taken as none); the greeting's own clip starts on the
//! state's first update, and what the idle clip then does to it is not read.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_law::path::{look_at, TurnMotion};
use moly_law::ui::dotween::Ease;

use crate::npc::{CharacterUnitId, MotionPhase, WalkState};

/// The greeting's look-at duration in seconds.
pub(crate) const GREETING_LOOK_AT_SECONDS: f32 = 1.0;

/// A look-at tween in flight.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct NpcLookAt {
    towards: Vec3,
    duration: f32,
    ease: Ease,
    /// Start yaw and change in degrees, from the first update.
    started: Option<(f32, f32)>,
    elapsed: f32,
    created: u32,
    /// The turn clip the presenter started with the tween.
    clip: TurnMotion,
}

impl NpcLookAt {
    /// A look-at towards `towards` by a character facing `forward` when it
    /// is called (the turn clip is chosen then).
    pub(crate) fn new(
        towards: Vec3,
        forward: [f32; 3],
        duration: f32,
        ease: Ease,
        frame: u32,
    ) -> Self {
        Self {
            towards,
            duration,
            ease,
            started: None,
            elapsed: 0.0,
            created: frame,
            clip: look_at::look_at_turn_motion(forward, towards.to_array()),
        }
    }

    /// The turn clip the motion driver plays while the look-at runs.
    pub(crate) fn clip(&self) -> TurnMotion {
        self.clip
    }
}

/// Update (after the agent step): one tween update per look-at in flight.
pub(crate) fn step(
    mut commands: Commands,
    clock: Res<crate::npc_clock::NpcClock>,
    frame: Res<FrameCount>,
    mut npcs: Query<(
        Entity,
        &CharacterUnitId,
        &mut NpcLookAt,
        &mut Transform,
        &mut WalkState,
        &MotionPhase,
    )>,
) {
    let frame = frame.0;
    let dt = clock.delta();
    for (entity, unit, mut look, mut transform, mut walk, phase) in &mut npcs {
        if look.created == frame {
            continue;
        }
        if !matches!(phase, MotionPhase::Dwelling { .. }) {
            commands.entity(entity).remove::<NpcLookAt>();
            info!(
                "[npc unit={}] frame={frame} DoLookAt ended by a move",
                unit.0
            );
            continue;
        }
        let (start, change) = match look.started {
            Some(started) => started,
            None => {
                let forward = transform.rotation * Vec3::Z;
                let start = look_at::yaw_of([forward.x, forward.z]);
                // The Y axis is constrained: the direction loses its height.
                let d = look.towards - transform.translation;
                let end = look_at::yaw_of([d.x, d.z]);
                let started = (start, look_at::fast_change(start, end));
                look.started = Some(started);
                info!(
                    "[npc unit={}] frame={frame} DoLookAt start: yaw {start:.3} -> {end:.3} (change {:.3}) over {} s, ease {}, turn clip {}",
                    unit.0,
                    started.1,
                    look.duration,
                    look.ease.value(),
                    look.clip.base_name()
                );
                started
            }
        };
        look.elapsed += dt;
        let done = look.duration <= look.elapsed;
        if done {
            look.elapsed = look.duration;
        }
        let eased = if look.duration > 0.0 {
            look.ease.evaluate(look.elapsed, look.duration)
        } else {
            1.0
        };
        let yaw = start + change * eased;
        let rotation = Quat::from_rotation_y(yaw.to_radians());
        transform.rotation = rotation;
        walk.0.forward = (rotation * Vec3::Z).to_array();
        if done {
            commands.entity(entity).remove::<NpcLookAt>();
            info!(
                "[npc unit={}] frame={frame} DoLookAt done at yaw {yaw:.3}",
                unit.0
            );
        }
    }
}
