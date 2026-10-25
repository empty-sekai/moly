//! An emoticon shown at a world point: the sprites and particles of
//! `moly://emoticons/emoticons.json` are drawn by the emoticon presenter
//! ([`crate::emoticon::timeline`]) as children of the entity the emote is
//! shown on, which need not be a character. A plain anchor entity (a
//! transform and a visibility, nothing else) stands for the point: with no
//! head, spine, hips or head-root bone under it, the presenter anchors the
//! emote at the fixed head offset it uses over a body without them.
//!
//! An anchor is spawned hidden with [`PointAnchor`], so nothing under it
//! draws before the presenter has put its draws there; it is shown once
//! they are, and the marker goes.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::emoticon::EmoteDraw;

/// An effect's anchor waiting for its first draws, hidden until then.
#[derive(Component, Clone, Copy, Default)]
pub struct PointAnchor;

/// Update: each waiting anchor with draws is shown.
pub fn anchor_places(
    mut commands: Commands,
    draws: Query<&ChildOf, With<EmoteDraw>>,
    mut anchors: Query<(Entity, &mut Visibility), With<PointAnchor>>,
) {
    if anchors.is_empty() {
        return;
    }
    let held: HashSet<Entity> = draws.iter().map(ChildOf::parent).collect();
    for (entity, mut visibility) in &mut anchors {
        if held.contains(&entity) {
            *visibility = Visibility::Inherited;
            commands.entity(entity).remove::<PointAnchor>();
        }
    }
}
