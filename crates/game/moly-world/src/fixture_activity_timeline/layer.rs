//! The Director's animation output laid over the body's own animation.
//!
//! A timeline drives a character through its own animation output on the
//! character's Animator. Each frame the output weight is set to the total
//! input weight of the track's mixer, and the output is laid over what the
//! Animator's controller plays: at full weight the timeline owns every bone
//! it animates, and during an ease the pose is the controller's pose lerped
//! toward the timeline's by that weight. The controller is not stopped: its
//! states keep advancing underneath, and the body shows them again once the
//! timeline lets go.
//!
//! Here the timeline's clips and the body's own clips play on one
//! `AnimationPlayer` under the same blend root, which averages every playing
//! clip by its normalized weight. While a timeline holds a body,
//! [`yield_to_timeline`] scales the body's own clips to share
//! `1 - timeline weight` for this frame's evaluation, and [`restore`] puts
//! their weights back right after it, so the systems that play those clips
//! never read the scaled values.

use bevy::animation::{
    animate_targets, advance_animations, graph::AnimationNodeIndex, AnimationPlayer,
};
use bevy::app::AnimationSystems;
use bevy::prelude::*;

use super::{FixtureActivityTimelines, TimelinePlaybackOwner};

/// The body clips scaled for the current evaluation, with their own weights.
#[derive(Resource, Default)]
struct Yielded(Vec<(Entity, AnimationNodeIndex, f32)>);

pub fn install(app: &mut App) {
    app.init_resource::<Yielded>().add_systems(
        PostUpdate,
        (
            yield_to_timeline
                .after(advance_animations)
                .before(animate_targets),
            restore.after(animate_targets),
        )
            .in_set(AnimationSystems),
    );
}

fn yield_to_timeline(
    timelines: Res<FixtureActivityTimelines>,
    owners: Query<&TimelinePlaybackOwner>,
    mut players: Query<&mut AnimationPlayer>,
    mut yielded: ResMut<Yielded>,
) {
    yielded.0.clear();
    for (token, session) in &timelines.sessions {
        for (index, bound) in session.nodes.iter().enumerate() {
            let animator = bound.animator;
            // Once per body: a session binds several clips to one animator.
            if session.nodes[..index].iter().any(|b| b.animator == animator) {
                continue;
            }
            if owners.get(animator).map_or(true, |owner| owner.0 != *token) {
                continue;
            }
            let Ok(mut player) = players.get_mut(animator) else {
                continue;
            };
            let timeline_node = |node: AnimationNodeIndex| {
                session
                    .nodes
                    .iter()
                    .any(|b| b.animator == animator && b.node == node)
            };
            let (mut timeline, mut own) = (0.0, 0.0);
            for (node, active) in player.playing_animations() {
                if timeline_node(*node) {
                    timeline += active.weight();
                } else {
                    own += active.weight();
                }
            }
            if timeline <= 0.0 || own <= 0.0 {
                continue;
            }
            let share = (1.0 - timeline).max(0.0) / own;
            for (node, active) in player.playing_animations_mut() {
                if !timeline_node(*node) {
                    yielded.0.push((animator, *node, active.weight()));
                    active.set_weight(active.weight() * share);
                }
            }
        }
    }
}

fn restore(mut yielded: ResMut<Yielded>, mut players: Query<&mut AnimationPlayer>) {
    for (animator, node, weight) in yielded.0.drain(..) {
        if let Ok(mut player) = players.get_mut(animator) {
            if let Some(active) = player.animation_mut(node) {
                active.set_weight(weight);
            }
        }
    }
}
