//! The site's own places, read from the world: the origin every placed
//! fixture's pose is measured from.

use bevy::prelude::*;

/// The site's own origin: its coordinate origin's world position, or the
/// world origin on a site that has none yet.
pub fn origin(world: &mut World) -> Vec3 {
    let mut origins = world.query_filtered::<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>();
    origins
        .iter(world)
        .next()
        .map_or(Vec3::ZERO, GlobalTransform::translation)
}
