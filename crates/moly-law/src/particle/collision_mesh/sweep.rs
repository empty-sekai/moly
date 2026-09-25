//! The sphere sweep against a cooked mesh: `PxGeometryQuery::sweep` for a
//! sphere with the hit flags normal and MTD and zero inflation.
use super::cook::{CookedMesh, PAGE_BYTES};
use super::vector::*;
use super::{arms, finite3, mtd, Pose, Refused};
use crate::particle::armf as a;

/// 1e-9: the floor of a ray direction component's magnitude.
const DIR_FLOOR: f32 = f32::from_bits(0x3089_705f);
/// 1e-7: the traversal widens the node bounds by the extents plus this.
const FATTEN: f32 = f32::from_bits(0x33d6_bf95);
/// 1e-5: the ray-triangle determinant's dead band.
const DET_EPS: f32 = f32::from_bits(0x3727_c5ac);
/// 1/3 as the triangle centre scales its corner sum.
const THIRD: f32 = f32::from_bits(0x3eaa_aaab);
/// -1e-4: the coarse cull's distance allowance.
const COARSE_ALLOWANCE: f32 = f32::from_bits(0xb8d1_b717);
/// 2e-3: the cull's radius allowance.
const CULL_ALLOWANCE: f32 = f32::from_bits(0x3b03_126f);
/// 1e-6: below this a capsule segment is a sphere.
const SEGMENT_FLOOR: f32 = f32::from_bits(0x3586_37bd);
const TEN: f32 = 10.0;

pub(super) const FLAG_POSITION: u32 = 1;
pub(super) const FLAG_NORMAL: u32 = 2;
pub(super) const FLAG_FACE_INDEX: u32 = 0x400;

/// A sweep hit as the engine returns it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshSweepHit {
    /// Position (1), normal (2) and face index (0x400) bits.
    pub flags: u32,
    pub face_index: u32,
    /// Negative for the depth of an initial overlap the MTD resolved.
    pub distance: f32,
    pub normal: [f32; 3],
    /// Present exactly when the flags carry the position.
    pub position: Option<[f32; 3]>,
}

/// The running hit of the mesh sweep callback.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub(super) distance: f32,
    pub(super) face: u32,
    pub(super) normal: V3,
    pub(super) position: Option<V3>,
}

/// One triangle's result from the leaf.
#[derive(Clone, Copy)]
struct LocalHit {
    distance: f32,
    normal: V3,
    position: Option<V3>,
    tri_normal: V3,
}

/// The engine's `PxGeometryQuery::sweep` of a sphere of `radius` at `center`
/// along the unit or zero `direction` for `distance` against the mesh at
/// `pose`. `faces` receives the triangles `processHit` saw, in order.
pub fn sweep_sphere(mesh: &CookedMesh, pose: &Pose, center: [f32; 3], radius: f32, direction: [f32; 3],
    distance: f32, faces: Option<&mut Vec<u32>>) -> Result<Option<MeshSweepHit>, Refused> {
    if !finite3(center) || !finite3(direction) || !distance.is_finite() || !radius.is_finite() {
        return Err(Refused("a non-finite sweep"));
    }
    if !(distance >= 0.0) || !(radius > 0.0) {
        return Err(Refused("a negative sweep distance or a non-positive radius"));
    }
    let (q, t) = (pose.rotation(), pose.translation());
    // Zero inflation.
    let r = a::add(radius, 0.0);
    // sweepCapsule_MeshGeom_RTREE with both capsule ends at the centre.
    let lp0 = transform_inv(q, t, center);
    let lp1 = transform_inv(q, t, center);
    let origin = scale(add(lp0, lp1), 0.5);
    let local_dir = quat_rotate_inv(q, direction);
    let extents: V3 = std::array::from_fn(|k| a::add(r, a::mul(a::abs(a::sub(lp0[k], lp1[k])), 0.5)));
    let mut cb = Callback {
        world: Matrix34::from_pose(q, t),
        true_dist: distance,
        best_align: 2.0,
        best_dist: a::add(distance, EPS_SAME),
        center,
        radius: r,
        dir: direction,
        status: false,
        initial: false,
        hit: Hit { distance, face: 0xffff_ffff, normal: [0.0; 3], position: Some([0.0; 3]) },
        faces,
    };
    if distance == 0.0 {
        // The ray collider with a zero length takes the box path: every
        // triangle of each touched leaf goes to processHit.
        let bmin = sub(origin, extents);
        let bmax = add(origin, extents);
        traverse_aabb(mesh, bmin, bmax, |ti| {
            let corners = mesh.corners(ti).ok_or(Refused("a leaf word past the cooked triangles"))?;
            Ok(cb.process_hit(ti, corners, 0.0).0)
        })?;
    } else {
        traverse_ray(mesh, origin, local_dir, extents, distance, &mut cb)?;
    }
    // finalizeHit.
    if !cb.status {
        return Ok(None);
    }
    let mut hit = cb.hit;
    let flags = if cb.initial {
        let resolved = if arms::on("skipMtd") { false } else { mtd::capsule_mesh_mtd(mesh, pose, center, r, &mut hit)? };
        if resolved {
            if hit.distance == 0.0 {
                hit.normal = neg(direction);
            }
            FLAG_FACE_INDEX | FLAG_NORMAL | FLAG_POSITION
        } else {
            hit.distance = 0.0;
            hit.normal = neg(direction);
            FLAG_FACE_INDEX | FLAG_NORMAL
        }
    } else {
        FLAG_FACE_INDEX | FLAG_NORMAL | FLAG_POSITION
    };
    Ok(Some(MeshSweepHit {
        flags,
        face_index: hit.face,
        distance: hit.distance,
        normal: hit.normal,
        position: if flags & FLAG_POSITION != 0 { Some(hit.position.unwrap_or([0.0; 3])) } else { None },
    }))
}

/// `SweepCapsuleMeshHitCallback` of the mesh sweep.
struct Callback<'f> {
    world: Matrix34,
    true_dist: f32,
    best_align: f32,
    best_dist: f32,
    center: V3,
    radius: f32,
    dir: V3,
    status: bool,
    initial: bool,
    hit: Hit,
    faces: Option<&'f mut Vec<u32>>,
}

impl Callback<'_> {
    /// Returns whether to go on and the shrunk ray length.
    fn process_hit(&mut self, face: u32, local: [V3; 3], shrunk: f32) -> (bool, f32) {
        if let Some(faces) = self.faces.as_deref_mut() {
            faces.push(face);
        }
        let tri = local.map(|v| self.world.transform(v));
        let widen = a::mul(EPS_SAME, a::max(self.hit.distance, 1.0));
        let min_d = a::add(self.hit.distance, widen);
        let Some(lh) = sweep_sphere_triangle(tri, self.center, self.radius, self.dir, min_d) else {
            return (true, shrunk);
        };
        let align = align_value(lh.tri_normal, self.dir);
        let mut shrunk = shrunk;
        if keep_triangle(lh.distance, align, self.best_dist, self.best_align, self.true_dist) {
            self.best_align = align;
            shrunk = a::mul(lh.distance, 1.0);
            self.best_dist = sel_min(self.best_dist, lh.distance);
            self.hit = Hit { distance: lh.distance, face, normal: lh.normal, position: lh.position };
            self.status = true;
            if lh.distance == 0.0 {
                self.initial = true;
                return (false, shrunk);
            }
        }
        (true, shrunk)
    }
}

/// `keepTriangle`: nearer than the best by more than the relative epsilon,
/// or within it and better aligned, or equally aligned and nearer, or a
/// touching start.
fn keep_triangle(d: f32, align: f32, best_d: f32, best_align: f32, max_d: f32) -> bool {
    if d > max_d {
        return false;
    }
    let eps = if arms::on("absoluteKeepEpsilon") { EPS_SAME } else { a::mul(EPS_SAME, sel_max(1.0, sel_max(d, best_d))) };
    if d < a::sub(best_d, eps) {
        return true;
    }
    if d < a::add(best_d, eps) && align < best_align {
        return true;
    }
    if align == best_align && d < best_d {
        return true;
    }
    d == 0.0
}

#[inline]
fn align_value(n: V3, d: V3) -> f32 {
    a::neg(a::abs(dot(n, d)))
}

/// The file-static `sweepSphereTriangle` of the mesh sweep: single sided,
/// the initial overlap tested first.
fn sweep_sphere_triangle(tri: [V3; 3], center: V3, radius: f32, d: V3, distance: f32) -> Option<LocalHit> {
    let n = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    if dot(n, d) > 0.0 {
        return None;
    }
    let cp = closest_pt_point_triangle(center, tri[0], tri[1], tri[2]);
    if mag2(sub(cp, center)) <= a::mul(radius, radius) {
        return Some(LocalHit { distance: 0.0, normal: neg(d), position: None, tri_normal: get_normalized(n) });
    }
    sweep_sphere_triangles(tri, center, radius, d, distance)
}

/// `sweepSphereTriangles` over one triangle: no cache, single sided, not an
/// any-hit query, no initial-overlap test of its own.
fn sweep_sphere_triangles(tri: [V3; 3], center: V3, radius: f32, d: V3, distance: f32) -> Option<LocalHit> {
    let cur_t = distance;
    let dpc0 = dot(center, d);
    let best_align = 2.0;
    // Coarse culling around the triangle centre.
    let tc = scale(add(add(tri[0], tri[1]), tri[2]), THIRD);
    let diff = sub(tc, center);
    let ft = sel_min(sel_max(dot(diff, d), 0.0), cur_t);
    let diff = sub(diff, scale(d, ft));
    let dd = a::add(a::sub(a::sqrt(mag2(diff)), radius), COARSE_ALLOWANCE);
    if !(dd < 0.0) {
        let dd = a::mul(dd, dd);
        if !(dd <= mag2(sub(tc, tri[0])) || dd <= mag2(sub(tc, tri[1])) || dd <= mag2(sub(tc, tri[2]))) {
            return None;
        }
    }
    // Culling along the direction.
    let (dp0, dp1, dp2) = (dot(tri[0], d), dot(tri[1], d), dot(tri[2], d));
    let dp = sel_min(sel_min(dp0, dp1), dp2);
    let r2 = a::add(radius, CULL_ALLOWANCE);
    if dp > a::add(a::add(dpc0, cur_t), r2) {
        return None;
    }
    let dpc1 = a::sub(dpc0, r2);
    if dp0 < dpc1 && dp1 < dpc1 && dp2 < dpc1 {
        return None;
    }
    let n = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    if dot(n, d) > 0.0 {
        return None;
    }
    let m = magnitude(n);
    if m == 0.0 {
        return None;
    }
    let n = div_scalar(n, m);
    let cur = sweep_sphere_vs_tri(tri, n, center, radius, d)?;
    if !keep_triangle(cur, align_value(n, d), cur_t, best_align, distance) {
        return None;
    }
    if cur == 0.0 {
        return Some(LocalHit { distance: 0.0, normal: neg(d), position: None, tri_normal: neg(d) });
    }
    // computeSphereTriImpactData.
    let new_center = add(center, scale(d, cur));
    let hit_point = closest_pt_point_triangle(new_center, tri[0], tri[1], tri[2]);
    let (mut normal, m) = normalize(sub(new_center, hit_point));
    if m < EPS_SAME {
        normal = normalize(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]))).0;
    }
    Some(LocalHit { distance: cur, normal, position: Some(hit_point), tri_normal: n })
}

enum RayTri {
    Parallel,
    Outside { u: f32, v: f32 },
    Inside(f32),
}

/// The ray-triangle test of the sphere-triangle sweep.
fn ray_tri_special(orig: V3, d: V3, v0: V3, e1: V3, e2: V3) -> RayTri {
    let pvec = cross(d, e2);
    let det = dot(e1, pvec);
    if det > a::neg(DET_EPS) && det < DET_EPS {
        return RayTri::Parallel;
    }
    let one_over = a::div(1.0, det);
    let tvec = sub(orig, v0);
    let u = a::mul(dot(tvec, pvec), one_over);
    let qvec = cross(tvec, e1);
    let v = a::mul(dot(d, qvec), one_over);
    if u < 0.0 || u > 1.0 || v < 0.0 || a::add(u, v) > 1.0 {
        return RayTri::Outside { u, v };
    }
    RayTri::Inside(a::mul(dot(e2, qvec), one_over))
}

/// Which feature the sphere meets when the offset ray misses the face:
/// `None` for the vertex `cand`, else the edge's other corner.
fn edge_or_vertex(p: V3, tri: &[V3; 3], cand: usize, a0: usize, a1: usize) -> Option<usize> {
    let e0 = sub(tri[cand], tri[a0]);
    if dot(e0, sub(p, tri[a0])) < dot(e0, e0) {
        return Some(a0);
    }
    let e1 = sub(tri[cand], tri[a1]);
    if dot(e1, sub(p, tri[a1])) < dot(e1, e1) {
        return Some(a1);
    }
    None
}

/// `sweepSphereVSTri`.
fn sweep_sphere_vs_tri(tri: [V3; 3], normal: V3, center: V3, radius: f32, d: V3) -> Option<f32> {
    let e10 = sub(tri[1], tri[0]);
    let e20 = sub(tri[2], tri[0]);
    let mut rv = scale(normal, radius);
    if !(dot(d, rv) < 0.0) {
        rv = neg(rv);
    }
    let (u, v) = match ray_tri_special(sub(center, rv), d, tri[0], e10, e20) {
        RayTri::Parallel => return None,
        RayTri::Inside(t) => return if t < 0.0 { None } else { Some(t) },
        RayTri::Outside { u, v } => (u, v),
    };
    let ipoint = || {
        let w = a::sub(a::sub(1.0, u), v);
        add(add(scale(tri[1], u), scale(tri[2], v)), scale(tri[0], w))
    };
    // (vertex test, first corner, second corner)
    let (vertex, e0, e1) = if u < 0.0 {
        if v < 0.0 {
            match edge_or_vertex(ipoint(), &tri, 0, 1, 2) { None => (true, 0, 0), Some(e1) => (false, 0, e1) }
        } else if a::add(u, v) > 1.0 {
            match edge_or_vertex(ipoint(), &tri, 2, 0, 1) { None => (true, 2, 0), Some(e1) => (false, 2, e1) }
        } else {
            (false, 0, 2)
        }
    } else if v < 0.0 {
        if a::add(u, v) > 1.0 {
            match edge_or_vertex(ipoint(), &tri, 1, 0, 2) { None => (true, 1, 0), Some(e1) => (false, 1, e1) }
        } else {
            (false, 0, 1)
        }
    } else {
        (false, 1, 2)
    };
    if vertex {
        return ray_sphere(center, d, FLT_MAX, tri[e0], radius);
    }
    match ray_capsule(center, d, tri[e0], tri[e1], radius) {
        Some(t) if t >= 0.0 => Some(t),
        _ => None,
    }
}

fn ray_sphere_basic(origin: V3, d: V3, length: f32, center: V3, radius: f32) -> Option<f32> {
    let offset = sub(center, origin);
    let ray_dist = dot(d, offset);
    let off2 = dot(offset, offset);
    let rad2 = a::mul(radius, radius);
    if off2 <= rad2 {
        return Some(0.0);
    }
    if ray_dist <= 0.0 || a::sub(ray_dist, length) > radius {
        return None;
    }
    let dd = a::sub(rad2, a::sub(off2, a::mul(ray_dist, ray_dist)));
    if dd < 0.0 {
        return None;
    }
    let dist = a::sub(ray_dist, a::sqrt(dd));
    if dist > length {
        return None;
    }
    Some(dist)
}

/// The ray-sphere test with its start moved up to ten units short of the
/// sphere.
fn ray_sphere(origin: V3, d: V3, length: f32, center: V3, radius: f32) -> Option<f32> {
    let x = sub(origin, center);
    let l = sel_max(a::sub(a::sub(a::sqrt(dot(x, x)), radius), TEN), 0.0);
    ray_sphere_basic(add(origin, scale(d, l)), d, a::sub(length, l), center, radius).map(|dist| a::add(dist, l))
}

fn dist_point_segment_sq(p0: V3, dir: V3, point: V3) -> f32 {
    let mut diff = sub(point, p0);
    let ft = dot(diff, dir);
    if ft <= 0.0 {
    } else {
        let sq = dot(dir, dir);
        if ft >= sq {
            diff = sub(diff, dir);
        } else {
            diff = sub(diff, scale(dir, a::div(ft, sq)));
        }
    }
    dot(diff, diff)
}

/// The ray-capsule roots, at most two, in the order they are found.
fn ray_capsule_internal(origin: V3, d: V3, p0: V3, p1: V3, radius: f32) -> Vec<f32> {
    let mut s = Vec::with_capacity(2);
    let mut kw = sub(p1, p0);
    let w_len = magnitude(kw);
    if w_len != 0.0 {
        kw = div_scalar(kw, w_len);
    }
    if w_len <= SEGMENT_FLOOR {
        let d0 = mag2(sub(origin, p0));
        let d1 = mag2(sub(origin, p1));
        let approx = a::mul(a::add(sel_max(d0, d1), radius), 2.0);
        return ray_sphere(origin, d, approx, p0, radius).into_iter().collect();
    }
    let mut ku = [0.0; 3];
    if w_len > 0.0 {
        if a::abs(kw[0]) >= a::abs(kw[1]) {
            let inv = a::div(1.0, a::sqrt(a::add(a::mul(kw[0], kw[0]), a::mul(kw[2], kw[2]))));
            ku = [a::mul(a::neg(kw[2]), inv), 0.0, a::mul(kw[0], inv)];
        } else {
            let inv = a::div(1.0, a::sqrt(a::add(a::mul(kw[1], kw[1]), a::mul(kw[2], kw[2]))));
            ku = [0.0, a::mul(kw[2], inv), a::mul(a::neg(kw[1]), inv)];
        }
    }
    let kv = normalize(cross(kw, ku)).0;
    let mut kd = [dot(ku, d), dot(kv, d), dot(kw, d)];
    let d_len = magnitude(kd);
    let inv_d_len = if d_len != 0.0 { a::div(1.0, d_len) } else { 0.0 };
    kd = scale(kd, inv_d_len);
    let kdiff = sub(origin, p0);
    let kp = [dot(ku, kdiff), dot(kv, kdiff), dot(kw, kdiff)];
    let r_sqr = a::mul(radius, radius);
    if a::abs(kd[2]) >= a::sub(1.0, EPS) || d_len < EPS {
        let axis_dir = dot(d, kw);
        let discr = a::sub(a::sub(r_sqr, a::mul(kp[0], kp[0])), a::mul(kp[1], kp[1]));
        if axis_dir < 0.0 && discr >= 0.0 {
            let root = a::sqrt(discr);
            return vec![a::mul(a::add(kp[2], root), inv_d_len),
                a::mul(a::neg(a::add(a::sub(w_len, kp[2]), root)), inv_d_len)];
        } else if axis_dir > 0.0 && discr >= 0.0 {
            let root = a::sqrt(discr);
            return vec![a::mul(a::neg(a::add(kp[2], root)), inv_d_len),
                a::mul(a::add(a::sub(w_len, kp[2]), root), inv_d_len)];
        }
        return Vec::new();
    }
    let fa = a::add(a::mul(kd[0], kd[0]), a::mul(kd[1], kd[1]));
    let mut fb = a::add(a::mul(kp[0], kd[0]), a::mul(kp[1], kd[1]));
    let mut fc = a::sub(a::add(a::mul(kp[0], kp[0]), a::mul(kp[1], kp[1])), r_sqr);
    let discr = a::sub(a::mul(fb, fb), a::mul(fa, fc));
    if discr < 0.0 {
        return Vec::new();
    }
    let within = |t: f32, lo: f32, hi: f32| {
        let tmp = a::add(kp[2], a::mul(t, kd[2]));
        tmp >= lo && tmp <= hi
    };
    if discr > 0.0 {
        let root = a::sqrt(discr);
        let inv = a::div(1.0, fa);
        let (lo, hi) = (a::neg(EPS_SAME), a::add(w_len, EPS_SAME));
        let t = a::mul(a::sub(a::neg(fb), root), inv);
        if within(t, lo, hi) {
            s.push(a::mul(t, inv_d_len));
        }
        let t = a::mul(a::add(a::neg(fb), root), inv);
        if within(t, lo, hi) {
            s.push(a::mul(t, inv_d_len));
        }
        if s.len() == 2 {
            return s;
        }
    } else {
        let t = a::div(a::neg(fb), fa);
        let tmp = a::add(kp[2], a::mul(t, kd[2]));
        if 0.0 <= tmp && tmp <= w_len {
            return vec![a::mul(t, inv_d_len)];
        }
    }
    // The cap at p0.
    fb = a::add(fb, a::mul(kp[2], kd[2]));
    fc = a::add(fc, a::mul(kp[2], kp[2]));
    let discr = a::sub(a::mul(fb, fb), fc);
    if discr > 0.0 {
        let root = a::sqrt(discr);
        for t in [a::sub(a::neg(fb), root), a::add(a::neg(fb), root)] {
            if a::add(kp[2], a::mul(t, kd[2])) <= 0.0 {
                s.push(a::mul(t, inv_d_len));
                if s.len() == 2 {
                    return s;
                }
            }
        }
    } else if discr == 0.0 {
        let t = a::neg(fb);
        if a::add(kp[2], a::mul(t, kd[2])) <= 0.0 {
            s.push(a::mul(t, inv_d_len));
            if s.len() == 2 {
                return s;
            }
        }
    }
    // The cap at p1.
    fb = a::sub(fb, a::mul(kd[2], w_len));
    fc = a::add(fc, a::mul(w_len, a::sub(w_len, a::mul(2.0, kp[2]))));
    let discr = a::sub(a::mul(fb, fb), fc);
    if discr > 0.0 {
        let root = a::sqrt(discr);
        for t in [a::sub(a::neg(fb), root), a::add(a::neg(fb), root)] {
            if a::add(kp[2], a::mul(t, kd[2])) >= w_len {
                s.push(a::mul(t, inv_d_len));
                if s.len() == 2 {
                    return s;
                }
            }
        }
    } else if discr == 0.0 {
        let t = a::neg(fb);
        if a::add(kp[2], a::mul(t, kd[2])) >= w_len {
            s.push(a::mul(t, inv_d_len));
            if s.len() == 2 {
                return s;
            }
        }
    }
    s
}

/// The ray-capsule test with its start moved up to ten units short.
fn ray_capsule(origin: V3, d: V3, p0: V3, p1: V3, radius: f32) -> Option<f32> {
    let l = a::sub(a::sqrt(dist_point_segment_sq(p0, sub(p1, p0), origin)), radius);
    if l <= 0.0 {
        return Some(0.0);
    }
    let l = if l > TEN { a::sub(l, TEN) } else { 0.0 };
    let s = ray_capsule_internal(add(origin, scale(d, l)), d, p0, p1, radius);
    let t = match s.as_slice() {
        [] => return None,
        [t] => *t,
        [t0, t1, ..] => if t0 < t1 { *t0 } else { *t1 },
    };
    Some(a::add(t, l))
}

/// `closestPtPointTriangle`.
pub(super) fn closest_pt_point_triangle(p: V3, a0: V3, b: V3, c: V3) -> V3 {
    let ab = sub(b, a0);
    let ac = sub(c, a0);
    let ap = sub(p, a0);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a0;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = a::sub(a::mul(d1, d4), a::mul(d3, d2));
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = a::div(d1, a::sub(d1, d3));
        return add(a0, scale(ab, v));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = a::sub(a::mul(d5, d2), a::mul(d1, d6));
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = a::div(d2, a::sub(d2, d6));
        return add(a0, scale(ac, w));
    }
    let va = a::sub(a::mul(d3, d6), a::mul(d5, d4));
    if va <= 0.0 && a::sub(d4, d3) >= 0.0 && a::sub(d5, d6) >= 0.0 {
        let w = a::div(a::sub(d4, d3), a::add(a::sub(d4, d3), a::sub(d5, d6)));
        return add(b, scale(sub(c, b), w));
    }
    let denom = a::div(1.0, a::add(a::add(va, vb), vc));
    let v = a::mul(vb, denom);
    let w = a::mul(vc, denom);
    add(add(a0, scale(ab, v)), scale(ac, w))
}

/// The ray callback's per-triangle box test (`intersectRayAABB2` with
/// four-step reciprocals).
fn ray_aabb2(mn: V3, mx: V3, ro: V3, rd: V3, max_t: f32) -> bool {
    let inv: V3 = std::array::from_fn(|k| {
        let sign = if rd[k] >= 0.0 { 1.0 } else { -1.0 };
        recip_n(a::mul(a::max(a::abs(rd[k]), DIR_FLOOR), sign), 4)
    });
    let t0: V3 = std::array::from_fn(|k| a::mul(inv[k], a::sub(mn[k], ro[k])));
    let t1: V3 = std::array::from_fn(|k| a::mul(inv[k], a::sub(mx[k], ro[k])));
    let tmin: V3 = std::array::from_fn(|k| a::min(t1[k], t0[k]));
    let tmax: V3 = std::array::from_fn(|k| a::max(t1[k], t0[k]));
    let near = a::max(a::max(tmin[0], a::max(tmin[1], tmin[2])), 0.0);
    let far = a::min(a::min(tmax[0], a::min(tmax[1], tmax[2])), max_t);
    far > near
}

/// The triangles of one leaf (or, in the source-order arm, of the whole
/// mesh) through the ray callback. Returns whether the query aborted.
#[allow(clippy::too_many_arguments)]
fn ray_leaf(mesh: &CookedMesh, triangles: impl Iterator<Item = u32>, origin: V3, dir: V3, inflate: V3,
    callback_max_t: &mut f32, new: &mut f32, cb: &mut Callback<'_>) -> Result<bool, Refused> {
    for ti in triangles {
        let [v0, v1, v2] = mesh.corners(ti).ok_or(Refused("a leaf word past the cooked triangles"))?;
        let mn: V3 = std::array::from_fn(|k| a::sub(a::min(a::min(v0[k], v1[k]), v2[k]), inflate[k]));
        let mx: V3 = std::array::from_fn(|k| a::add(a::max(a::max(v0[k], v1[k]), v2[k]), inflate[k]));
        let cm = *callback_max_t;
        let rel = a::add(cm, a::mul(a::max(cm, 1.0), EPS_SAME));
        if !ray_aabb2(mn, mx, origin, dir, rel) {
            continue;
        }
        let (again, shrunk) = cb.process_hit(ti, [v0, v1, v2], *new);
        if !again {
            return Ok(true);
        }
        if shrunk < *new {
            *new = shrunk;
            *callback_max_t = shrunk;
        }
    }
    Ok(false)
}

/// `RTree::traverseRay<1>` with the extents: fattened node bounds, the
/// reciprocal direction by FRECPE, one FRECPS step and one unfused Newton
/// step, children pushed in lane order and popped in reverse, the ray
/// length shortened after each leaf.
fn traverse_ray(mesh: &CookedMesh, origin: V3, dir: V3, inflate: V3, max_t: f32, cb: &mut Callback<'_>)
    -> Result<(), Refused> {
    let mut mt = max_t;
    let mut callback_max_t = max_t;
    if arms::on("sourceOrder") {
        let mut order: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
        order.sort_by_key(|&t| mesh.source_index[t as usize]);
        let mut new = mt;
        ray_leaf(mesh, order.into_iter(), origin, dir, inflate, &mut callback_max_t, &mut new, cb)?;
        return Ok(());
    }
    let fat: V3 = std::array::from_fn(|k| a::add(inflate[k], FATTEN));
    let dd: V3 = std::array::from_fn(|k| {
        let m = a::max(a::abs(dir[k]), DIR_FLOOR);
        f32::from_bits((dir[k].to_bits() & 0x8000_0000) | m.to_bits())
    });
    let inv_d: V3 = std::array::from_fn(|k| {
        let e = a::recip_estimate(dd[k]);
        let e = a::mul(e, a::recip_step(e, dd[k]));
        a::mul(e, a::sub(2.0, a::mul(e, dd[k])))
    });
    let pinv: V3 = std::array::from_fn(|k| a::sub(0.0, a::mul(origin[k], inv_d[k])));
    // One root page.
    let mut stack: Vec<u32> = vec![0];
    while let Some(ptr) = stack.pop() {
        if ptr & 1 != 0 {
            let data = ptr - 1;
            let base = data >> 5;
            let count = ((data >> 1) & 15) + 1;
            let mut new = mt;
            if ray_leaf(mesh, base..base + count, origin, dir, inflate, &mut callback_max_t, &mut new, cb)? {
                return Ok(());
            }
            if mt != new {
                mt = new;
            }
            continue;
        }
        let page = mesh.pages.get((ptr / PAGE_BYTES) as usize).ok_or(Refused("a node word past the RTree pages"))?;
        for lane in 0..4 {
            let (mnx, mny, mnz) = (page.min[0][lane], page.min[1][lane], page.min[2][lane]);
            let (mxx, mxy, mxz) = (page.max[0][lane], page.max[1][lane], page.max[2][lane]);
            let slab = |k: usize, lo: f32, hi: f32| {
                let t0 = a::add(pinv[k], a::mul(inv_d[k], a::sub(lo, fat[k])));
                let t1 = a::add(pinv[k], a::mul(inv_d[k], a::add(fat[k], hi)));
                (a::min(t0, t1), a::max(t0, t1))
            };
            let (tminx, tmaxx) = slab(0, mnx, mxx);
            let (tminy, tmaxy) = slab(1, mny, mxy);
            let (tminz, tmaxz) = slab(2, mnz, mxz);
            let fars = a::min(a::min(tmaxx, tmaxy), tmaxz);
            let nears = a::max(a::max(tminx, tminy), tminz);
            let fail = mnx > mxx || nears > mt || DIR_FLOOR > fars || nears > fars;
            if !fail {
                stack.push(page.ptrs[lane]);
            }
        }
    }
    Ok(())
}

/// `RTree::traverseAABB` with an aligned box: pages pushed high to low,
/// lanes in order, a touched leaf's triangles visited at once, inner nodes
/// pushed. The visitor returns whether to go on.
pub(super) fn traverse_aabb(mesh: &CookedMesh, bmin: V3, bmax: V3, mut visit: impl FnMut(u32) -> Result<bool, Refused>)
    -> Result<(), Refused> {
    if arms::on("sourceOrder") {
        let mut order: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
        order.sort_by_key(|&t| mesh.source_index[t as usize]);
        for ti in order {
            if !visit(ti)? {
                return Ok(());
            }
        }
        return Ok(());
    }
    let mut stack: Vec<u32> = vec![0];
    while let Some(top) = stack.pop() {
        let page = mesh.pages.get((top / PAGE_BYTES) as usize).ok_or(Refused("a node word past the RTree pages"))?;
        for i in 0..4 {
            let (mnx, mny, mnz) = (page.min[0][i], page.min[1][i], page.min[2][i]);
            let (mxx, mxy, mxz) = (page.max[0][i], page.max[1][i], page.max[2][i]);
            let miss = bmin[0] > mxx || bmin[1] > mxy || bmin[2] > mxz || mnx > bmax[0] || mny > bmax[1] || mnz > bmax[2];
            if miss {
                continue;
            }
            let ptr = page.ptrs[i];
            if ptr & 1 != 0 {
                let data = ptr & !1;
                let base = data >> 5;
                let count = ((data >> 1) & 15) + 1;
                for k in 0..count {
                    if !visit(base + k)? {
                        return Ok(());
                    }
                }
            } else {
                stack.push(ptr);
            }
        }
    }
    Ok(())
}
