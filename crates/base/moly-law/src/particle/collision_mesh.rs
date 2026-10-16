//! The physics scene side of the CollisionModule for a non-convex
//! MeshCollider cooked with the default options (the BVH33 midphase) or
//! with the options 30 (the BV4 midphase: `bv4_cook` and `bv4_query`), as
//! the engine's PhysX 4.1 build executes it. The BVH33 side:
//!
//! - cooking: the engine's vertex weld (a hash of the position bits, the chain
//!   walked from the newest entry, float equality, first-occurrence order),
//!   the COOKING_PERFORMANCE RTree build (per-triangle bounds widened by
//!   5e-4, a quick-select split on the centre sums, eight triangles per leaf,
//!   empty nodes that add the origin to their parent bound, four-lane pages),
//!   the triangle remap into leaf order, the local bounds and the active
//!   edge flags of each triangle;
//! - the sphere sweep (`PxGeometryQuery::sweep` for a sphere with hit flags
//!   normal and MTD, zero inflation): `sweepCapsule_MeshGeom_RTREE`, the ray
//!   traversal `RTree::traverseRay<1>` with the per-triangle box test of the
//!   ray callback, `processHit` with `keepTriangle`, the file-static
//!   `sweepSphereTriangle` (single sided, initial overlap first), the
//!   `sweepSphereTriangles` leaf with its culling, `sweepSphereVSTri`, the
//!   ray-sphere and ray-capsule forms and `computeSphereTriImpactData`; a
//!   sweep of distance exactly zero takes the box traversal of the same entry
//!   instead of the ray; `finalizeHit`;
//! - the depth of an initial overlap: `computeCapsule_TriangleMeshMTD` for a
//!   sphere (the box midphase `RTree::traverseAABB`, batches of 32, the
//!   centre backface test, the PCM capsule-triangle contact generation with
//!   its inlined normal selection, up to four depenetration steps);
//! - the scene narrowphase of a box query (`intersectTriangleBox` over the
//!   box traversal, the first touching triangle ends it) and the shape's
//!   world bounds.
//!
//! Order matters and is kept: `processHit` keeps a triangle by distance
//! within a relative 1e-3 of the running best and then by alignment, and the
//! MTD keeps the first of equal penetrations, so the cooked leaf order is the
//! order the triangles are visited in. The vector code of this path has no
//! fused multiply-add; every operation below is a separate binary32
//! operation in the engine's association, through the ARM rules (NaN
//! propagation, FMIN/FMAX signed zeros, the reciprocal and reciprocal square
//! root estimate tables and their fused step instructions).
//!
//! The BV4 side runs the same MTD body with its own box midphase; its
//! sweep, box queries and cooking are described in those files.
//!
//! The sphere sweep against a BoxCollider's box (`box_sweep`) is the
//! capsule-box sweep's GJK raycast, and for an initial overlap the GJK
//! penetration and the EPA, in the vector library's NEON arithmetic.
//!
//! What is not covered refuses by name: other cooking options, a rotated
//! collider on the BVH33 midphase (the BV4 queries take any rotation), a
//! non-finite or negative query, and the few structural cases the cooked
//! representation cannot hold. The queries here have no any-hit, double
//! sided, inflated, scaled or two-point capsule form, and the options-30
//! cook builds four triangles per leaf only: those engine paths cannot be
//! asked for.

mod box_sweep;
mod convex_cook;
mod convex_sweep;
mod bv4_cook;
mod bv4_query;
mod cook;
mod mtd;
mod overlap;
pub mod physics_steps;
pub mod static_pruner;
mod sweep;
mod vector;

pub use box_sweep::{box_world_bounds, flush_bounds_box, pool_bounds_box, sweep_sphere_box, BoxSweepTrace};
pub use convex_cook::{cook_convex, engine_cooks_nothing};
pub use convex_sweep::{convex_world_bounds, pool_bounds_convex, sweep_sphere_convex, GaussMap, HullSupport};
pub use cook::{cook, CookedMesh};
pub use overlap::{overlap_box, overlap_oriented_box, pool_bounds_mesh, shape_world_pose, world_bounds};
pub use sweep::{sweep_sphere, MeshSweepHit, SweepTrace};

/// Why the scene cannot give the engine's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

/// A collider's world pose: a rotation and a finite translation. `new`
/// takes the identity rotation only (the zero vector part may carry either
/// sign; the engine's quaternion arithmetic is still carried), `rotated`
/// any finite rotation. The bits are the query's: the BV4 sweep and box
/// query set up their world matrix from them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    rotation: [f32; 4],
    translation: [f32; 3],
}

impl Pose {
    /// `rotation` is x, y, z, w.
    pub fn new(rotation: [f32; 4], translation: [f32; 3]) -> Result<Self, Refused> {
        if rotation[..3].iter().any(|&v| v != 0.0) || rotation[3] != 1.0 {
            return Err(Refused("collider rotation other than the identity"));
        }
        if translation.iter().any(|v| !v.is_finite()) {
            return Err(Refused("non-finite collider translation"));
        }
        Ok(Self { rotation, translation })
    }

    /// Any finite rotation (x, y, z, w). Only the BV4 queries take a
    /// rotation other than the identity; the BVH33 ones refuse it.
    pub fn rotated(rotation: [f32; 4], translation: [f32; 3]) -> Result<Self, Refused> {
        if rotation.iter().chain(&translation).any(|v| !v.is_finite()) {
            return Err(Refused("a non-finite collider pose"));
        }
        Ok(Self { rotation, translation })
    }

    /// The identity rotation by value (zero parts of either sign).
    pub(crate) fn identity_rotation(&self) -> bool {
        self.rotation[..3].iter().all(|&v| v == 0.0) && self.rotation[3] == 1.0
    }

    pub fn rotation(&self) -> [f32; 4] {
        self.rotation
    }

    pub fn translation(&self) -> [f32; 3] {
        self.translation
    }
}

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

#[inline]
fn finite3(v: [f32; 3]) -> bool {
    v.iter().all(|x| x.is_finite())
}

#[cfg(test)]
mod tests;
