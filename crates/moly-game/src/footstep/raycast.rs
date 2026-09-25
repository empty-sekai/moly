//! The water splash's ground height: `Physics.Raycast(player position + up
//! * 3, down, out hit, 3)`, then the splash sits at `hit.point.y`.
//!
//! The product has no physics scene. The ray is cast against the site's
//! mesh colliders as the release lists them (the scene's collision rows in
//! the site index), in world space, and the nearest hit within the distance
//! wins. Mesh colliders are hit only from their front side (the physics
//! default does not hit back faces). Named gaps: collider layers are not
//! exported, so every row is cast against; colliders of other shapes and the
//! colliders of placed fixtures are not among the rows.

use bevy::prelude::*;

/// World-space collider triangles, wound so that the right-handed normal
/// `(b - a) x (c - a)` is the face's outward side.
#[derive(Clone, Debug, Default)]
pub(crate) struct Colliders {
    pub triangles: Vec<[Vec3; 3]>,
}

impl Colliders {
    /// Nearest front-face hit of a straight-down ray from `origin` within
    /// `distance`: the hit point.
    pub(crate) fn cast_down(&self, origin: Vec3, distance: f32) -> Option<Vec3> {
        let direction = Vec3::NEG_Y;
        let mut best: Option<f32> = None;
        for [a, b, c] in &self.triangles {
            let e1 = *b - *a;
            let e2 = *c - *a;
            let normal = e1.cross(e2);
            // Front side only: the ray must travel against the face normal.
            if normal.dot(direction) >= 0.0 {
                continue;
            }
            // Moller-Trumbore.
            let p = direction.cross(e2);
            let det = e1.dot(p);
            if det.abs() <= f32::EPSILON * normal.length().max(1.0) * 1e-3 {
                continue;
            }
            let inv = 1.0 / det;
            let s = origin - *a;
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = direction.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv;
            if t >= 0.0 && t <= distance && best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        }
        best.map(|t| origin + direction * t)
    }
}
