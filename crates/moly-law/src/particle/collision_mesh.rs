//! The physics scene side of the CollisionModule for a non-convex
//! MeshCollider cooked with the default options (the BVH33 midphase), as the
//! engine's PhysX 4.1 build executes it:
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
//! What is not covered refuses by name: other cooking options (the BV4
//! midphase included), collider rotations other than the identity, a
//! non-finite or negative query, and the few structural cases the cooked
//! representation cannot hold.

mod cook;
mod mtd;
mod overlap;
mod sweep;
mod vector;

pub use cook::{cook, CookedMesh};
pub use overlap::{overlap_box, world_bounds};
pub use sweep::{sweep_sphere, MeshSweepHit};

/// Why the scene cannot give the engine's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

/// A collider's world pose: the identity rotation (the zero vector part may
/// carry either sign; the engine's quaternion arithmetic is still carried)
/// and a finite translation.
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
