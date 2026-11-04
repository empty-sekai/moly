//! Getting up: the pose blends from where the parts lie back into the standing
//! animation, the character is placed where its pelvis came to rest on the
//! ground, and the script's take of it is handed back.
//!
//! The pose that blends in is the animation's: while the recovery runs, the
//! animation plays again from its current time, so every bone's local rotation
//! from the animation is the pose the character gets up into. Each part's bone
//! starts the blend from the local rotation its fallen body gives it, and over
//! the recovery time the blended rotation turns from the fallen one to the
//! animated one. A node under the model with no part starts from the local pose
//! it held when the body fell, so fingers, the face and hair roots follow their
//! parents without a jump and without a blend of their own.
//!
//! The character's place moves over the same time from where it stands to
//! where it will stand: where the pelvis's bone came to rest, on the floor it
//! settled on, turned towards where it was facing when it fell, and with the
//! pelvis's bone as high over the root as it was when it fell. Its walk state
//! is written with it, as a sequence's root write does, so a paused or a
//! resumed character stands where the recovery left it.
//!
//! When the blend is done the parts, their joints and their scene are removed,
//! the animation is handed back, and the character's take ends: the one the
//! ragdoll made for the control surface, or the one a script made, whose run
//! then holds a character that has already got up and given itself back.

use bevy::prelude::*;

use moly_features::character::{MotionDriver, MotionKind, SEGMENT_BLEND};
use moly_features::npc::{CharacterUnitId, WalkState};

use crate::drop::Drop;
use crate::RagdollSettings;

/// Where a recovery is going and how far it has come.
pub struct Getting {
    /// The game seconds the blend started at.
    pub started: f32,
    /// The rotation the blend starts from, in each node's own frame, by
    /// entity: the fallen pose's for a part's bone, the pose the node held
    /// when the body fell for the rest.
    pub local: Vec<(Entity, Quat)>,
    /// Where the character's root stood when the blend started.
    pub root_from: Vec3,
    /// Where the character's root will stand: over its pelvis, on the floor.
    pub root: Vec3,
    /// How the character will be turned: towards where it faced when it fell.
    pub facing: Quat,
}

/// Starts a recovery of the character `member`.
pub fn begin(world: &mut World, member: Entity) -> Result<(), String> {
    let now = world.resource::<Time>().elapsed_secs();
    let unit = world.get::<CharacterUnitId>(member).map_or(0, |unit| unit.0);
    let Some(drop) = world.get::<Drop>(member) else {
        return Err(format!("character {unit} is not falling"));
    };
    if drop.getting_up.is_some() {
        return Err(format!("character {unit} is already getting up"));
    }
    // The pelvis's bone where its body came to rest, on the floor.
    let pelvis = drop.root_part();
    let settled = pelvis.frame(world).map_or(pelvis.position, |(position, _)| position);
    let settled = Vec3::new(settled.x, drop.floor, settled.z);
    let facing = Quat::from_rotation_arc(Vec3::Z, drop.facing);
    let root_from = world.get::<Transform>(member).map_or(settled, |transform| transform.translation);
    let mut local: Vec<(Entity, Quat)> = drop.held.iter().map(|(entity, pose)| (*entity, pose.rotation)).collect();
    // Each part's bone starts the blend from where the fall has written it:
    // the fallen rotation of the bone against the node above it.
    for part in drop.all_parts() {
        let Some(start) = world.get::<Transform>(part.bone).map(|pose| pose.rotation) else {
            continue;
        };
        match local.iter_mut().find(|(entity, _)| *entity == part.bone) {
            Some(entry) => entry.1 = start,
            None => local.push((part.bone, start)),
        }
    }
    if let Some(mut drop) = world.get_mut::<Drop>(member) {
        drop.getting_up = Some(Getting { started: now, local, root_from, root: settled, facing });
    }
    // The animation plays again from its current time: the pose getting up is
    // the animated one.
    resume(world, member);
    info!("[ragdoll] character {unit} gets up");
    Ok(())
}

/// Update, after the frame's step: a body that has come to rest for the settle
/// time, or has fallen for its longest time, starts getting up.
pub fn begin_due(world: &mut World) {
    let settings = *world.resource::<RagdollSettings>();
    let delta = world.resource::<Time>().delta_secs();
    let due: Vec<(Entity, bool, bool)> = {
        let mut falling = world.query_filtered::<(Entity, &Drop), ()>();
        falling
            .iter(world)
            .filter(|(_, drop)| drop.getting_up.is_none())
            .map(|(member, drop)| {
                let rest = settings.rest_speed(drop.height);
                (member, drop.fastest() < rest, drop.how_long >= settings.max_seconds)
            })
            .collect()
    };
    let mut settling = std::mem::take(&mut *world.resource_mut::<crate::settle::Settling>());
    for (member, still, longest) in due {
        settling.hold(member, still, delta);
        if settling.held(member) >= settings.settle_seconds || longest {
            if let Err(reason) = begin(world, member) {
                warn!("[ragdoll] {reason}");
            }
        }
    }
    world.insert_resource(settling);
}

/// Plays the graph's idle again from its current time, so the animation sets
/// the bones the character gets up into.
fn resume(world: &mut World, member: Entity) {
    let Some(driver) = world.get::<MotionDriver>(member) else {
        return;
    };
    let (animator, idle) = (driver.player, driver.idle);
    {
        let mut players = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
        if let Ok((mut player, mut transitions)) = players.get_mut(world, animator) {
            transitions.play(&mut player, idle, SEGMENT_BLEND).repeat();
        }
    }
    if let Some(mut driver) = world.get_mut::<MotionDriver>(member) {
        driver.playing = Some(MotionKind::Idle);
    }
}

/// PostUpdate, after the animation and before the transforms propagate: the
/// recovery's blend, each frame until it is done.
pub fn blend(world: &mut World) {
    let settings = *world.resource::<RagdollSettings>();
    let now = world.resource::<Time>().elapsed_secs();
    let recovering: Vec<Entity> = world.query_filtered::<Entity, With<Drop>>().iter(world).collect();
    for member in recovering {
        // The blend reads where it goes and how far it has come; keeping
        // those, and not the body, leaves the world free to be written.
        let Some((started, root_from, root_stood, facing, pelvis_height)) = world.get::<Drop>(member).and_then(|drop| {
            drop.getting_up
                .as_ref()
                .map(|getting| (getting.started, getting.root_from, getting.root, getting.facing, drop.pelvis_height))
        }) else {
            continue;
        };
        let progress = if settings.recover_seconds <= 0.0 { 1.0 } else { ((now - started) / settings.recover_seconds).clamp(0.0, 1.0) };
        // Smooth at both ends: the body gathers itself rather than snapping.
        let weight = progress * progress * (3.0 - 2.0 * progress);
        // Each node turns from where the blend started to the animation's own
        // rotation, which the animation has just written.
        if progress < 1.0 {
            let starts: Vec<(Entity, Quat)> = world.get::<Drop>(member).and_then(|drop| drop.getting_up.as_ref()).map(|getting| getting.local.clone()).unwrap_or_default();
            let mut locals = world.query::<&mut Transform>();
            for (entity, start) in starts {
                if let Ok(mut local) = locals.get_mut(world, entity) {
                    local.rotation = start.slerp(local.rotation, weight);
                }
            }
        }
        // The character moves to where it will stand, with its walk state.
        let root = root_from.lerp(root_stood + Vec3::Y * pelvis_height, weight);
        if let Some(mut transform) = world.get_mut::<Transform>(member) {
            transform.translation = root;
            transform.rotation = facing;
        }
        if let Some(mut walk) = world.get_mut::<WalkState>(member) {
            walk.0.position = root.to_array();
            walk.0.forward = (facing * Vec3::Z).to_array();
        }
        if progress >= 1.0 {
            if let Err(reason) = finish(world, member) {
                warn!("[ragdoll] {reason}");
            }
        }
    }
}

/// Ends a recovery: the parts and the scene are removed, the animation is
/// handed back, and the take of the character ends.
fn finish(world: &mut World, member: Entity) -> Result<(), String> {
    let unit = world.get::<CharacterUnitId>(member).map_or(0, |unit| unit.0);
    let Some(drop) = world.get::<Drop>(member) else {
        return Err(format!("character {unit} is not falling"));
    };
    let standing = drop
        .getting_up
        .as_ref()
        .map(|getting| (getting.root + Vec3::Y * drop.pelvis_height, getting.facing))
        .or_else(|| world.get::<Transform>(member).map(|transform| (transform.translation, transform.rotation)));
    let took_from_ai = drop.took_from_ai;
    for entity in drop.entities() {
        world.despawn(entity);
    }
    // The ragdoll's token goes back if it is still the holder; a state that
    // took the body meanwhile keeps it.
    if let Some(mut driver) = world.get_mut::<MotionDriver>(member) {
        if driver.is_held_by(moly_features::character::BodyOwner::Ragdoll) {
            if let Some(token) = driver.token() {
                driver.hand_back(token);
            }
        }
    }
    world.entity_mut(member).remove::<(Drop, moly_features::character::Falling)>();
    // The character's own place in the world is left where it got up, so the
    // walk state and the animation agree from the first standing frame.
    if let (Some((root, facing)), Some(mut global)) = (standing, world.get_mut::<GlobalTransform>(member)) {
        let scale = global.scale();
        *global = GlobalTransform::from(Transform { translation: root, rotation: facing, scale });
    }
    let mut settling = std::mem::take(&mut *world.resource_mut::<crate::settle::Settling>());
    settling.forget(member);
    world.insert_resource(settling);
    // A character the fall took from its AI goes back to it from here; one a
    // script holds stays the script's, standing where it got up.
    if took_from_ai && world.get::<moly_features::npc::DirectedOwned>(member).is_some() {
        moly_features::npc_take::release(world, member);
    }
    info!("[ragdoll] character {unit} has got up");
    Ok(())
}
