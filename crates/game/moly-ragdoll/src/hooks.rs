//! What the game asks of the ragdoll, and what the ragdoll does when another
//! state takes a falling character away.
//!
//! A script's step asks a character to fall through the hub's fall request
//! ([`moly_features::act::FallRequests`]), which calls what a host installs
//! here. A falling character whose body another state takes (a sequence, a
//! still pose) is no longer falling: its parts, its joints and its scene are
//! removed, so no body of parts is left where a character stands again.

use bevy::prelude::*;

use crate::drop::{self, Ask};
use moly_features::act::FallRequests;
use moly_features::character::{BodyOwner, MotionDriver, MotionKind, SEGMENT_BLEND};

/// The fall a script asks for: the body is built on this frame, from the pose
/// this frame's animation leaves.
pub fn request_falling(world: &mut World, member: Entity, push: Option<Vec3>, at: Option<String>) -> Result<(), String> {
    drop::carry_out(world, member, Ask::Drop { impulse: push, at })?;
    Ok(())
}

/// The fall requests of a host whose characters can fall.
pub fn fall_requests() -> FallRequests {
    FallRequests::new(request_falling)
}

/// Update, after the frame's step: a falling character whose body another
/// state took is no longer falling. Its parts, its joints and its scene are
/// removed, and its idle is played so that the state that took it blends in
/// from a standing pose.
pub fn clean_up(world: &mut World) {
    let fallen: Vec<Entity> = world.query_filtered::<Entity, With<crate::drop::Drop>>().iter(world).collect();
    for member in fallen {
        // Who holds the body, and where its idle plays, are read out of the
        // driver before the world is written again, so no borrow of it lasts
        // past those reads.
        let held = world.get::<MotionDriver>(member).map(|driver| (driver.player, driver.idle, driver.held_by()));
        let Some((animator, idle, Some(owner))) = held else {
            continue;
        };
        if owner == BodyOwner::Ragdoll {
            continue;
        }
        drop::take_away(world, member);
        let mut players = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
        if let Ok((mut player, mut transitions)) = players.get_mut(world, animator) {
            transitions.play(&mut player, idle, SEGMENT_BLEND).repeat();
        }
        if let Some(mut driver) = world.get_mut::<MotionDriver>(member) {
            driver.playing = Some(MotionKind::Idle);
        }
        let unit = world.get::<moly_features::npc::CharacterUnitId>(member).map_or(0, |unit| unit.0);
        info!("[ragdoll] character {unit}: another state took the body, so its fall ends");
    }
}
