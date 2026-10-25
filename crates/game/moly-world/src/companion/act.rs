//! A companion's motion on its own skeleton: the motion-library motion a
//! [`super::CompanionCommand::Motion`] names, played by the body's own
//! animator ([`crate::player_avatar::oc::own`]), then the body's idle.
//!
//! The motion plays as the original's `ChangeAnimation` of a tweet row does
//! with its end motion: the start segment `{motion}_S`, then the loop
//! segment `{motion}_L` once when the library has one, then the end segment
//! `{motion}_E` when it has one, and the body's idle family when the last
//! segment has played. A motion the library has no start segment of is
//! refused by name, and the body keeps what it plays.
//!
//! This module owns the start (what refuses a motion) and the end (once the
//! animator has returned to the idle family); the segment chain itself is the
//! own animator's.

use super::{Companion, CompanionState, CompanionStatus};
use crate::npc::{MotionPhase, PathSlot, RouteStops, WalkState};
use crate::player_avatar::oc::own;
use crate::player_avatar::oc::OcModel;
use bevy::prelude::*;

/// A motion in flight: the command's number and its name.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Acting {
    pub seq: u64,
    pub motion: String,
}

impl Acting {
    /// The motion `motion` of the command numbered `seq`.
    pub(super) fn new(seq: u64, motion: &str) -> Self {
        Self {
            seq,
            motion: motion.to_owned(),
        }
    }
}

/// The body `motion` plays on, the character body the companion `entity`
/// wears; or why it cannot be asked for: a body with no own animator, a
/// motion library that is not in, or a motion the library has no start
/// segment of.
pub(super) fn refuse(
    entity: Entity,
    motion: &str,
    bodies: &Query<(Entity, &OcModel), With<own::OwnAnimator>>,
    server: &AssetServer,
    libraries: &Assets<bevy::gltf::Gltf>,
) -> Result<Entity, String> {
    let Some(body) = super::own_motion::own_body(bodies, entity) else {
        return Err("the body has no own motion (it plays the avatar's idle and walk)".to_owned());
    };
    let library = server.load::<bevy::gltf::Gltf>(moly_assets::motion_library());
    let Some(library) = libraries.get(&library) else {
        return Err("the motion library is not loaded".to_owned());
    };
    let start = format!("{motion}_S");
    if !library.named_animations.contains_key(start.as_str()) {
        return Err(format!("the motion library has no start segment {start}"));
    }
    Ok(body)
}

/// Stops the walk of the companion `entity` where it stands; the reason
/// names a navigation state held elsewhere.
pub(super) fn stop_walk(
    entity: Entity,
    walks: &mut Query<(&mut WalkState, &mut PathSlot, &mut RouteStops, &mut MotionPhase)>,
) -> Result<(), String> {
    match walks.get_mut(entity) {
        Ok((mut walk, mut slot, mut route, mut phase)) => {
            crate::npc::stop_agent(&mut route, &mut slot, &mut walk, &mut phase);
            Ok(())
        }
        Err(_) => Err("the companion's navigation state is in use".to_owned()),
    }
}

/// Update, before the commands: a motion in flight whose body has gone back
/// to its idle family has ended, and its command is `Done`.
pub(super) fn finish(
    mut commands: Commands,
    mut companions: Query<(Entity, &Companion, &Acting, &mut CompanionStatus)>,
    bodies: Query<(Entity, &OcModel), With<own::OwnAnimator>>,
    own: Query<&own::OwnAnimator>,
    waiting: Res<own::OnceMotions>,
) {
    for (entity, companion, acting, mut status) in &mut companions {
        if status.state != CompanionState::Acting {
            // A later command took the body over; this motion is over.
            commands.entity(entity).remove::<Acting>();
            continue;
        }
        let body = super::own_motion::own_body(&bodies, entity);
        // The row waits until the body can take it.
        if body.is_some_and(|body| waiting.first(body).is_some()) {
            continue;
        }
        let failed = match body.and_then(|body| own.get(body).ok()) {
            None => Some("the body lost its own animator"),
            Some(body) if !body.plays_once() => Some("the motion was dropped before it started"),
            Some(body) if !body.once_over() => continue,
            Some(_) => None,
        };
        if let Some(reason) = failed {
            commands.entity(entity).remove::<Acting>();
            status.fail(reason);
            error!("[companion] unit {} motion {} ({}) failed: {reason}", companion.unit, acting.seq, acting.motion);
            continue;
        }
        commands.entity(entity).remove::<Acting>();
        status.state = CompanionState::Done;
        info!(
            "[companion] unit {} motion {} ({}): the motion is done",
            companion.unit, acting.seq, acting.motion
        );
    }
}
