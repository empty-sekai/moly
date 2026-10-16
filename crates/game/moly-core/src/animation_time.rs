//! A refusal for animation times that are not numbers.
//!
//! The animation system samples each playing clip at its seek time and
//! panics without naming anything when that time is NaN (a repeating clip of
//! zero length wraps its time to NaN on its first frame; a seek or speed set
//! from a zero division does the same). This check runs between the clips'
//! advance and their sampling and stops the game with the player, the
//! character or object it animates, the graph node and the clip named.

use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeType};
use bevy::animation::AnimationPlayer;
use bevy::prelude::*;

/// The component that names the character unit an entity animates, for the
/// owner this check names in its message. The domain that owns the component
/// implements it, so the check itself knows no character component.
pub trait UnitId {
    fn unit_id(&self) -> u32;
}

/// PostUpdate, after the clips advance and before they are sampled.
pub fn refuse_non_finite<U: UnitId + Component>(
    players: Query<(Entity, &AnimationPlayer, Option<&AnimationGraphHandle>)>,
    graphs: Res<Assets<AnimationGraph>>,
    clips: Res<Assets<AnimationClip>>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    units: Query<&U>,
) {
    for (entity, player, graph) in &players {
        for (node, active) in player.playing_animations() {
            if active.seek_time().is_finite() && active.speed().is_finite() {
                continue;
            }
            let clip = graph
                .and_then(|graph| graphs.get(&graph.0))
                .and_then(|graph| graph.get(*node))
                .and_then(|node| match &node.node_type {
                    AnimationNodeType::Clip(handle) => Some(handle),
                    _ => None,
                });
            let clip_name = clip.map(|handle| match handle.path() {
                Some(path) => path.to_string(),
                None => format!("{:?}", handle.id()),
            });
            let duration = clip.and_then(|handle| clips.get(handle)).map(AnimationClip::duration);
            // The owner: the first ancestor with a character unit, else the
            // names up the chain.
            let mut chain = Vec::new();
            let mut unit = None;
            let mut at = Some(entity);
            while let Some(current) = at {
                if unit.is_none() {
                    unit = units.get(current).ok().map(U::unit_id);
                }
                if let Ok(name) = names.get(current) {
                    chain.push(name.as_str().to_owned());
                }
                at = parents.get(current).ok().map(ChildOf::parent);
            }
            panic!(
                "[animation-time] player {entity} (character unit {unit:?}, names {chain:?}): node {node:?} clip {clip_name:?} of duration {duration:?} has seek time {} at speed {} (elapsed {}, repeat {:?}, completions {}, weight {}); the clip cannot be sampled",
                active.seek_time(),
                active.speed(),
                active.elapsed(),
                active.repeat_mode(),
                active.completions(),
                active.weight(),
            );
        }
    }
}
