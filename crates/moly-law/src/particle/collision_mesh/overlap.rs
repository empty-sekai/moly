//! The scene narrowphase of an axis-aligned box query against the mesh
//! (`GeomOverlapCallback_BoxMesh` with unit mesh scale: the box in vertex
//! space through the box midphase, `intersectTriangleBox` in its vector
//! form, the first touching triangle ends the query) and the shape's world
//! bounds (`Gu::computeBounds` of a triangle mesh at inflation one).
use super::cook::CookedMesh;
use super::sweep::traverse_aabb;
use super::vector::*;
use super::{finite3, Pose, Refused};
use crate::particle::armf as a;

fn copysign_bits(magnitude: f32, sign: u32) -> f32 {
    f32::from_bits((magnitude.to_bits() & 0x7fff_ffff) | sign)
}

/// The separating-axis test of one edge `e` against the box.
fn class3(e: V3, v0: V3, v1: V3, v2: V3, ext: V3) -> bool {
    let project = |v: V3| -> V3 {
        [a::sub(a::mul(v[0], e[1]), a::mul(v[1], e[0])),
            a::sub(a::mul(v[1], e[2]), a::mul(v[2], e[1])),
            a::sub(a::mul(v[2], e[0]), a::mul(v[0], e[2]))]
    };
    let (p0, p1, p2) = (project(v0), project(v1), project(v2));
    let mn: V3 = std::array::from_fn(|k| a::min(a::min(p0[k], p1[k]), p2[k]));
    let fe = e.map(a::abs);
    let fe_yzx = [fe[1], fe[2], fe[0]];
    let ext_yzx = [ext[1], ext[2], ext[0]];
    let rad: V3 = std::array::from_fn(|k| a::add(a::mul(ext_yzx[k], fe[k]), a::mul(ext[k], fe_yzx[k])));
    if (0..3).any(|k| mn[k] > rad[k]) {
        return false;
    }
    let mx: V3 = std::array::from_fn(|k| a::max(a::max(p0[k], p1[k]), p2[k]));
    let nrad: V3 = std::array::from_fn(|k| a::sub(0.0, rad[k]));
    !(0..3).any(|k| nrad[k] > mx[k])
}

/// `intersectTriangleBox` for a box at `center` with half extents `ext`.
fn tri_box(center: V3, ext: V3, ta: V3, tb: V3, tc: V3) -> bool {
    let v0 = sub(ta, center);
    let v1 = sub(tb, center);
    let v2 = sub(tc, center);
    if (0..3).all(|k| ext[k] >= a::abs(v0[k])) {
        return true;
    }
    let mn: V3 = std::array::from_fn(|k| a::min(a::min(v0[k], v1[k]), v2[k]));
    if (0..3).any(|k| mn[k] > ext[k]) {
        return false;
    }
    let mx: V3 = std::array::from_fn(|k| a::max(a::max(v0[k], v1[k]), v2[k]));
    let ne: V3 = std::array::from_fn(|k| a::sub(0.0, ext[k]));
    if (0..3).any(|k| ne[k] > mx[k]) {
        return false;
    }
    let e0 = sub(v1, v0);
    let e1 = sub(v2, v1);
    let n = cross(e0, e1);
    let d = v3dot(n, v0);
    let toward: V3 = std::array::from_fn(|k| copysign_bits(ext[k], n[k].to_bits() & 0x8000_0000));
    if d > v3dot(n, toward) {
        return false;
    }
    let away: V3 = std::array::from_fn(|k| copysign_bits(ext[k], (n[k].to_bits() & 0x8000_0000) ^ 0x8000_0000));
    if v3dot(n, away) > d {
        return false;
    }
    if !class3(e0, v0, v1, v2, ext) || !class3(e1, v0, v1, v2, ext) {
        return false;
    }
    let e2 = sub(v0, v2);
    class3(e2, v0, v1, v2, ext)
}

/// Whether the axis-aligned box (world centre, half extents) touches a
/// triangle of the mesh at `pose`.
pub fn overlap_box(mesh: &CookedMesh, pose: &Pose, center: [f32; 3], extents: [f32; 3]) -> Result<bool, Refused> {
    if !finite3(center) || !finite3(extents) {
        return Err(Refused("a non-finite overlap box"));
    }
    let m = inverse_matrix(pose.rotation(), pose.translation());
    let c2 = m.transform(center);
    let [c0, c1, c2c] = m.columns;
    let aext: V3 = std::array::from_fn(|k| {
        a::add(a::add(a::mul(a::abs(c0[k]), extents[0]), a::mul(a::abs(c1[k]), extents[1])), a::mul(a::abs(c2c[k]), extents[2]))
    });
    let mut hit = false;
    traverse_aabb(mesh, sub(c2, aext), add(c2, aext), |ti| {
        let [ta, tb, tc] = mesh.corners(ti).ok_or(Refused("a leaf word past the cooked triangles"))?;
        // Identity rotations: box space is vertex space about the box centre.
        if tri_box(c2, extents, ta, tb, tc) {
            hit = true;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(hit)
}

/// The shape's world bounds at inflation one: min then max.
pub fn world_bounds(mesh: &CookedMesh, pose: &Pose) -> [f32; 6] {
    let m = Matrix34::from_pose(pose.rotation(), pose.translation());
    let [r0, r1, r2] = m.columns;
    let (ce, ex) = (mesh.center, mesh.extents);
    let t = pose.translation();
    let c: V3 = std::array::from_fn(|k| {
        a::add(a::add(a::add(a::mul(r0[k], ce[0]), a::mul(r1[k], ce[1])), a::mul(r2[k], ce[2])), t[k])
    });
    let e: V3 = std::array::from_fn(|k| {
        let e = a::add(a::add(a::abs(a::mul(r0[k], ex[0])), a::abs(a::mul(r1[k], ex[1]))), a::abs(a::mul(r2[k], ex[2])));
        // No contact offset, inflation one.
        a::mul(a::add(e, 0.0), 1.0)
    });
    [a::sub(c[0], e[0]), a::sub(c[1], e[1]), a::sub(c[2], e[2]), a::add(c[0], e[0]), a::add(c[1], e[1]), a::add(c[2], e[2])]
}
