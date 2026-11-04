//! The ground under a character, as a ragged body falls onto it: the height of
//! the site's navigation surface at a world point, which is where the
//! character's own feet stand.
//!
//! The navigation face is the surface the characters walk on, so a character
//! dropped where it walks falls back to the floor it was standing on, whatever
//! the floor's height. Where the face is not built yet, or has no surface at
//! the point (a character standing off the walkable area), the caller's own
//! estimate stands.

use bevy::prelude::*;

/// The navigation surface at `xz`, where the site has one.
pub fn floor_at(world: &World, xz: [f32; 2]) -> Option<f32> {
    let face = world.get_resource::<crate::npc_objective::ObjectiveFace>()?;
    face.surface_height_at(xz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_face_is_no_floor() {
        let world = World::new();
        assert_eq!(floor_at(&world, [0.0, 0.0]), None);
    }
}
