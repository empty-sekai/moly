//! Remembering where every bone stood, drawing the first pose of a fall, and
//! writing the fallen pose back on the bones.
//!
//! The ragdoll reads the bones after the animation has run and before the
//! transforms are propagated: the parts' speeds at the drop come from where
//! the bones stood at this point of the last frame, and every part's world
//! rotation is written on its bone at this point of each frame while it
//! falls. A bone that has no part (a finger, the face, a hair root) keeps the
//! local pose the animation gives it, and so follows the part above it.
//!
//! A character taken by a script loses its segment switches on the frame it is
//! taken, so its animator is left running from where it was: the seek draws
//! those running animations once, in the frame's last write of the bones, so
//! the body falls from the pose the character stands in before the next
//! frame's animation runs, and nothing shows a frame of the pose before it.
//!
//! At the start of each frame, after the clocks and before the animation
//! runs, the bones of each character model are recorded where the last frame's
//! propagation left them.

use std::collections::HashMap;

use avian3d::prelude::{Position, Rotation};
use bevy::prelude::*;

use crate::drop::Drop;

/// Where the bones stood at the last propagated frame, by entity.
#[derive(Resource, Default)]
pub struct Previous {
    places: HashMap<Entity, GlobalTransform>,
}

impl Previous {
    /// Where a bone stood at the last propagated frame.
    pub fn get(&self, entity: Entity) -> Option<GlobalTransform> {
        self.places.get(&entity).copied()
    }

    /// Records where a bone stands now.
    pub fn set(&mut self, entity: Entity, place: GlobalTransform) {
        self.places.insert(entity, place);
    }
}

/// The bones of a character model, marked the first frame the model is seen.
#[derive(Component, Default)]
pub(crate) struct MarkedBones {
    bones: Vec<Entity>,
}

type Bodies<'w, 's> = Query<'w, 's, (&'static Position, &'static Rotation)>;
type Locals<'w, 's> = Query<'w, 's, &'static mut Transform>;

/// A node's world rotation from the local rotations up its tree, as they
/// stand now: the bones above it may have been turned this frame, before the
/// transforms propagate.
fn world_rotation(entity: Entity, parents: &Query<&ChildOf>, locals: &Locals) -> Quat {
    let mut rotation = Quat::IDENTITY;
    let mut node = Some(entity);
    while let Some(at) = node {
        if let Ok(local) = locals.get(at) {
            rotation = local.rotation * rotation;
        }
        node = parents.get(at).ok().map(ChildOf::parent);
    }
    rotation
}

/// First, after the clocks: every character model's bones are marked once,
/// and the marked bones are recorded where the last frame left them.
pub fn remember(
    mut previous: ResMut<Previous>,
    mut commands: Commands,
    models: Query<(Entity, Option<&MarkedBones>, &Children), With<moly_features::character::CharacterModel>>,
    children: Query<&Children>,
    names: Query<&Name>,
    bones: Query<&GlobalTransform>,
) {
    for (model, marked, kids) in &models {
        let Some(marked) = marked else {
            let mut found = Vec::new();
            let mut stack: Vec<Entity> = kids.iter().collect();
            while let Some(entity) = stack.pop() {
                if names.get(entity).is_ok() {
                    found.push(entity);
                }
                if let Ok(kids) = children.get(entity) {
                    stack.extend(kids.iter());
                }
            }
            commands.entity(model).insert(MarkedBones { bones: found });
            continue;
        };
        for entity in &marked.bones {
            if let Ok(global) = bones.get(*entity) {
                previous.set(*entity, *global);
            }
        }
    }
}

/// PostUpdate, after the animation, before the transforms propagate and before
/// the cloth reads the bones: the animator of a character dropped this frame
/// is drawn once where it stands, and then each part's world rotation is
/// written on its bone, as the bone's rotation against its parent's. The
/// bone's position and scale stay as the animation set them, so every bone
/// keeps its length from its parent and the parts stay one body.
pub fn bones(drops: Query<&Drop, With<moly_features::character::Falling>>, bodies: Bodies, parents: Query<&ChildOf>, mut locals: Locals, mut players: Query<&mut AnimationPlayer>) {
    for drop in &drops {
        if let Some(animator) = drop.seek {
            if let Ok(mut player) = players.get_mut(animator) {
                player.seek_all_by(0.0);
            }
        }
        if drop.getting_up.is_some() {
            continue;
        }
        // The parts are laid out from the pelvis down, so the bone above each
        // one has been turned by the time the one below is.
        for part in drop.all_parts() {
            let bone = match bodies.get(part.body) {
                Ok((position, rotation)) => part.bone_at(position.0, rotation.0),
                Err(_) => part.bone_at(part.position, part.rotation),
            };
            let above = parents.get(part.bone).ok().map_or(Quat::IDENTITY, |child_of| world_rotation(child_of.parent(), &parents, &locals));
            if let Ok(mut local) = locals.get_mut(part.bone) {
                local.rotation = above.inverse() * bone.rotation;
            }
        }
    }
}
