//! A companion's turn: the yaw from the one it faces to the point a
//! [`super::CompanionCommand::Face`] names, tweened linearly over the
//! command's seconds, or snapped when the command names none.
//!
//! The body has no look-at tween of the roster characters'
//! ([`crate::npc_look_at`]): it carries no `CharacterUnitId`, its motion
//! phase and actions, and the one that turns its head. A companion turns its
//! whole body, as its walks and its build do (`turn_to`), and stands while
//! it turns.

use super::{turn_to, Companion, CompanionState, CompanionStatus};
use bevy::prelude::*;

/// A turn in flight: from the yaw it started at to the one asked for,
/// `elapsed` seconds into `secs`.
#[derive(Component, Clone, Copy, Debug)]
pub(super) struct Facing {
    from: f32,
    to: f32,
    secs: f32,
    elapsed: f32,
}

impl Facing {
    /// A turn from the yaw `from` to the point `to`, the body standing at
    /// `at`; `secs` zero finishes it on its first step.
    pub(super) fn new(from: f32, to: Vec3, at: Vec3, secs: f32) -> Self {
        let to = shortest(from, yaw_toward(at, to).unwrap_or(from));
        Self { from, to, secs, elapsed: 0.0 }
    }

    /// The yaw `elapsed` seconds in.
    fn yaw(&self) -> f32 {
        if self.secs <= 0.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * (self.elapsed / self.secs).clamp(0.0, 1.0)
    }
}

/// The yaw (radians) of a forward direction: the rotation `turn_to` made.
fn yaw_of(forward: Vec3) -> f32 {
    forward.x.atan2(forward.z)
}

/// The yaw turned to from `from` in the shorter way round.
fn shortest(from: f32, to: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    let delta = (to - from).rem_euclid(turn);
    from + if delta > turn * 0.5 { delta - turn } else { delta }
}

/// Every companion in the world: its unit, entity, the yaw it faces
/// (radians) and its status.
pub fn turned(world: &mut World) -> impl Iterator<Item = (u32, Entity, f32, CompanionStatus)> + '_ {
    world
        .query::<(Entity, &Companion, &Transform, &CompanionStatus)>()
        .iter(world)
        .map(|(entity, companion, transform, status)| {
            (companion.unit, entity, yaw_of(transform.rotation * Vec3::Z), status.clone())
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// The point (x, z) faces from where `at` is: `atan2(x, z)` radians, the
/// yaw [`turn_to`] gives a body turned toward it.
pub fn yaw_toward(at: Vec3, to: Vec3) -> Option<f32> {
    turn_to(at, to).map(|rotation| yaw_of(rotation * Vec3::Z))
}

/// Update, after the commands: each turn runs on the engine clock, and the
/// command it serves is `Done` when the turn reaches its end.
pub(super) fn step(
    mut commands: Commands,
    time: Res<Time>,
    mut companions: Query<(
        Entity,
        &Companion,
        &mut CompanionStatus,
        &mut Facing,
        &mut Transform,
        &mut crate::npc::WalkState,
    )>,
) {
    for (entity, companion, mut status, mut facing, mut transform, mut walk) in &mut companions {
        if facing.secs > 0.0 {
            facing.elapsed = (facing.elapsed + time.delta_secs()).min(facing.secs);
        }
        let rotation = Quat::from_rotation_y(facing.yaw());
        transform.rotation = rotation;
        walk.0.forward = (rotation * Vec3::Z).to_array();
        if facing.secs <= 0.0 || facing.elapsed >= facing.secs {
            commands.entity(entity).remove::<Facing>();
            status.state = CompanionState::Done;
            info!("[companion] unit {} seq {}: the turn is done", companion.unit, status.seq);
        }
    }
}
