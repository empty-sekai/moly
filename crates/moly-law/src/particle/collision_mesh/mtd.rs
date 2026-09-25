//! `computeCapsule_TriangleMeshMTD` for a sphere (both capsule ends at the
//! centre): the vertex-space box of the sphere through the box midphase,
//! batches of 32 triangles, the centre backface test, the PCM
//! capsule-triangle contacts (`pcmDistanceSegmentTriangleSquared`, the
//! inlined normal selection, `generateContacts`, `generateEEContactsMTD`),
//! the deepest contact per triangle and over the batch, and up to four
//! depenetration steps. The vector forms are the NEON ones: `V3Dot` adds the
//! zero fourth lanes, the square root is `a * rsqrt<4>(a)` and the
//! reciprocal four refinement steps.
use super::cook::CookedMesh;
use super::sweep::{traverse_aabb, Hit};
use super::vector::*;
use super::{Pose, Refused};
use crate::particle::armf as a;

/// The contact search radius is the sphere's times this (1.15).
const INFLATION: f32 = f32::from_bits(0x3f93_3333);
/// The normal selection's barycentric bounds: 1e-6, 0.999999 and 0.9999.
const SELECT_ZERO: f32 = f32::from_bits(0x3586_37bd);
const SELECT_ONE: f32 = f32::from_bits(0x3f7f_ffef);
const SELECT_EDGE: f32 = f32::from_bits(0x3f7f_f972);
const BATCH: usize = 32;
const ITERATIONS: usize = 4;

#[derive(Clone, Copy)]
struct Contact {
    point: V3,
    normal: V3,
    penetration: f32,
}

/// The triangles the midphase returns for a sphere at `center`: the box
/// around it in vertex space (the inverse pose applied to the centre; the
/// extents through the inverse rotation's absolute columns), aligned.
fn midphase(mesh: &CookedMesh, pose: &Pose, center: V3, r: f32) -> Result<Vec<u32>, Refused> {
    let box_center = scale(add(center, center), 0.5);
    let m = inverse_matrix(pose.rotation(), pose.translation());
    let c2 = m.transform(box_center);
    let [c0, c1, c2c] = m.columns;
    let ext: V3 = std::array::from_fn(|k| {
        a::add(a::add(a::mul(a::abs(c0[k]), r), a::mul(a::abs(c1[k]), r)), a::mul(a::abs(c2c[k]), r))
    });
    let mut found = Vec::new();
    traverse_aabb(mesh, sub(c2, ext), add(c2, ext), |ti| {
        found.push(ti);
        Ok(true)
    })?;
    Ok(found)
}

/// The MTD of a sphere of `radius` at `center` that starts overlapping the
/// mesh. Returns whether an initial overlap was found; the hit then carries
/// the depth (negative), the contact point, the normal and the face.
pub(super) fn capsule_mesh_mtd(mesh: &CookedMesh, pose: &Pose, center: V3, radius: f32, hit: &mut Hit)
    -> Result<bool, Refused> {
    let world = Matrix34::from_pose(pose.rotation(), pose.translation());
    let inflated = a::mul(radius, INFLATION);
    let (mut p0, mut p1) = (center, center);
    let mut cc = scale(add(p0, p1), 0.5);
    let mut found_initial = false;
    let mut closest = [0.0f32; 3];
    let mut normal = [0.0f32; 3];
    let mut tri_index: usize = 0x0fff_ffff;
    let mut translation = [0.0f32; 3];
    for iteration in 0..ITERATIONS {
        let indices = midphase(mesh, pose, cc, radius)?;
        if indices.is_empty() {
            break;
        }
        let mut had = false;
        let mut mtd = FLT_MAX;
        for (s, batch) in indices.chunks(BATCH).enumerate() {
            for (j, &ti) in batch.iter().enumerate() {
                let local = mesh.corners(ti).ok_or(Refused("a leaf word past the cooked triangles"))?;
                let tri = local.map(|v| world.transform(v));
                let flags = mesh.extra[ti as usize];
                let tn = v3normalize(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])));
                if 0.0 > v3dot(tn, sub(cc, tri[0])) {
                    continue;
                }
                let mut out = Vec::new();
                process_triangle(tri, p0, p1, inflated, flags, &mut out);
                let Some(first) = out.first() else {
                    continue;
                };
                had = true;
                let mut deepest = *first;
                for c in &out[1..] {
                    if deepest.penetration > c.penetration {
                        deepest = *c;
                    }
                }
                if mtd > deepest.penetration {
                    tri_index = s * BATCH + j;
                    mtd = deepest.penetration;
                    normal = deepest.normal;
                    closest = deepest.point;
                }
            }
        }
        if !had {
            break;
        }
        tri_index = *indices.get(tri_index).ok_or(Refused("no deepest contact among the MTD triangles"))? as usize;
        found_initial = true;
        let dist = a::sub(mtd, radius);
        if 0.0 >= dist {
            let step = scale(normal, dist);
            translation = sub(translation, step);
            let new_center = sub(cc, step);
            let offset = sub(new_center, cc);
            cc = new_center;
            p0 = add(p0, offset);
            p1 = add(p1, offset);
        } else {
            if iteration == 0 {
                *hit = Hit { distance: 0.0, face: tri_index as u32, normal, position: Some(closest) };
                return Ok(true);
            }
            break;
        }
    }
    let length = v3length(translation);
    let normal = if length > 0.0 {
        let inv = frecip(length);
        translation.map(|c| a::mul(c, inv))
    } else {
        [0.0; 3]
    };
    if found_initial {
        *hit = Hit { distance: a::neg(length), face: tri_index as u32, normal, position: Some(closest) };
    }
    Ok(found_initial)
}

fn valid_bary(v: f32, w: f32) -> bool {
    let zero = a::neg(EPS);
    let one = a::add(1.0, EPS);
    (v >= zero && one >= v) && (w >= zero && one >= w) && (one > a::add(v, w))
}

/// `pcmDistanceSegmentSegmentSquared4`: the segment `(p, d0)` against four
/// segments; squared distances and both parameters.
fn seg_seg_sq4(p: V3, d0: V3, pairs: [(V3, V3); 4]) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let aa = v3dot(d0, d0);
    let a_recip = frecip(aa);
    let (mut out_d, mut out_s, mut out_t) = ([0.0; 4], [0.0; 4], [0.0; 4]);
    for (i, (pk, dk)) in pairs.into_iter().enumerate() {
        let r = sub(p, pk);
        let e = a::add(a::mul(dk[2], dk[2]), a::add(a::mul(dk[0], dk[0]), a::mul(dk[1], dk[1])));
        let b = a::add(a::mul(d0[2], dk[2]), a::add(a::mul(d0[0], dk[0]), a::mul(d0[1], dk[1])));
        let c = a::add(a::mul(d0[2], r[2]), a::add(a::mul(d0[0], r[0]), a::mul(d0[1], r[1])));
        let f = a::add(a::mul(dk[2], r[2]), a::add(a::mul(dk[0], r[0]), a::mul(dk[1], r[1])));
        let e_recip = frecip(e);
        let denom = a::sub(a::mul(aa, e), a::mul(b, b));
        let temp = a::sub(a::mul(b, f), a::mul(c, e));
        let value = if denom == 0.0 { 1.0 } else { a::mul(temp, frecip(denom)) };
        let s0 = fclamp(value, 0.0, 1.0);
        let s_tmp = if EPS >= denom { 0.5 } else { s0 };
        let t_tmp = if e == 0.0 { 1.0 } else { a::mul(a::add(a::mul(b, s_tmp), f), e_recip) };
        let t2 = fclamp(t_tmp, 0.0, 1.0);
        let comp = if aa == 0.0 { 1.0 } else { a::mul(a::sub(a::mul(b, t2), c), a_recip) };
        let s2 = fclamp(comp, 0.0, 1.0);
        let c1: V3 = std::array::from_fn(|k| a::add(p[k], a::mul(d0[k], s2)));
        let c2: V3 = std::array::from_fn(|k| a::add(pk[k], a::mul(dk[k], t2)));
        let v = sub(c1, c2);
        out_d[i] = a::add(a::mul(v[0], v[0]), a::add(a::mul(v[1], v[1]), a::mul(v[2], v[2])));
        out_s[i] = s2;
        out_t[i] = t2;
    }
    (out_d, out_s, out_t)
}

/// `pcmDistanceSegmentTriangleSquared`: (squared distance, segment
/// parameter, barycentric u, v).
fn seg_tri_sq(p: V3, q: V3, ta: V3, tb: V3, tc: V3) -> (f32, f32, f32, f32) {
    let pq = sub(q, p);
    let ab = sub(tb, ta);
    let ac = sub(tc, ta);
    let bc = sub(tc, tb);
    let ap = sub(p, ta);
    let aq = sub(q, ta);
    let n = v3normalize(cross(ab, ac));
    let ab_ab = v3dot4(ab, ab);
    let ab_ac = v3dot4(ab, ac);
    let ac_ac = v3dot4(ac, ac);
    let dist3 = v3dot4(ap, n);
    let bdenom = frecip(a::sub(a::mul(ab_ab, ac_ac), a::mul(ab_ac, ab_ac)));
    let sq_dist3 = a::mul(dist3, dist3);
    let dist4 = v3dot(aq, n);
    let sq_dist4 = a::mul(dist4, dist4);
    let d_mul = a::mul(dist3, dist4);
    let bary = |v_ab: f32, v_ac: f32| {
        (a::mul(a::sub(a::mul(ac_ac, v_ab), a::mul(ab_ac, v_ac)), bdenom),
            a::mul(a::sub(a::mul(ab_ab, v_ac), a::mul(ab_ac, v_ab)), bdenom))
    };
    if 0.0 > d_mul {
        let nom = a::neg(v3dot(n, ap));
        let den = frecip(v3dot(n, pq));
        let t0 = a::mul(nom, den);
        let ip = v3scale_add(pq, t0, p);
        let v2 = sub(ip, ta);
        let (v0, w0) = bary(v3dot(v2, ab), v3dot(v2, ac));
        if valid_bary(v0, w0) {
            return (0.0, t0, v0, w0);
        }
    }
    let cp31 = v3neg_scale_sub(n, dist3, p);
    let cp41 = v3neg_scale_sub(n, dist4, q);
    let pv20 = sub(cp31, ta);
    let qv20 = sub(cp41, ta);
    let (v0, w0) = bary(v3dot4(pv20, ab), v3dot4(pv20, ac));
    let (v1, w1) = bary(v3dot4(qv20, ab), v3dot4(qv20, ac));
    let con0 = valid_bary(v0, w0);
    let con1 = valid_bary(v1, w1);
    if con0 && con1 {
        let d2 = sq_dist4 > sq_dist3;
        return if d2 { (sq_dist3, 0.0, v0, w0) } else { (sq_dist4, 1.0, v1, w1) };
    }
    let (sq, s4, t4) = seg_seg_sq4(p, pq, [(ta, ab), (tb, bc), (ta, ac), (ta, ab)]);
    let (u01, v01) = (t4[0], 0.0);
    let (u11, v11) = (a::sub(1.0, t4[1]), t4[1]);
    let (u21, v21) = (0.0, t4[2]);
    let con2 = sq[1] > sq[0] && sq[2] > sq[0];
    let con3 = sq[2] > sq[1];
    let pick = |x0: f32, x1: f32, x2: f32| if con2 { x0 } else if con3 { x1 } else { x2 };
    let sq_pe = pick(sq[0], sq[1], sq[2]);
    let u_e = pick(u01, u11, u21);
    let v_e = pick(v01, v11, v21);
    let t_s = pick(s4[0], s4[1], s4[2]);
    if con0 {
        let d2 = sq_pe > sq_dist3;
        return if d2 { (sq_dist3, 0.0, v0, w0) } else { (sq_pe, t_s, u_e, v_e) };
    }
    if con1 {
        let d2 = sq_pe > sq_dist4;
        return if d2 { (sq_dist4, 1.0, v1, w1) } else { (sq_pe, t_s, u_e, v_e) };
    }
    (sq_pe, t_s, u_e, v_e)
}

/// The inlined normal selection: whether the triangle normal is kept at
/// barycentric (u, v), given the triangle's convex-edge bits.
fn select_normal(u: f32, v: f32, data: u8) -> bool {
    if SELECT_ZERO > u {
        if SELECT_ZERO > v {
            data & 0x28 == 0
        } else if v > SELECT_ONE {
            data & 0x30 == 0
        } else {
            data & 0x20 == 0
        }
    } else if u > SELECT_ONE {
        if SELECT_ZERO > v {
            data & 0x18 == 0
        } else {
            false
        }
    } else if SELECT_ZERO > v {
        data & 0x08 == 0
    } else if a::add(u, v) >= SELECT_EDGE {
        data & 0x10 == 0
    } else {
        true
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_contacts(ta: V3, tb: V3, tc: V3, plane_n: V3, normal: V3, p: V3, q: V3, r: f32, out: &mut Vec<Contact>) {
    let ab = sub(tb, ta);
    let ac = sub(tc, ta);
    let ap = sub(p, ta);
    let aq = sub(q, ta);
    let ab_ab = v3dot(ab, ab);
    let ab_ac = v3dot(ab, ac);
    let ac_ac = v3dot(ac, ac);
    let bdenom = frecip(a::sub(a::mul(ab_ab, ac_ac), a::mul(ab_ac, ab_ac)));
    let ideom = v3dot(plane_n, normal);
    let bary = |pt: V3| {
        let v20 = sub(pt, ta);
        let (v_ab, v_ac) = (v3dot(v20, ab), v3dot(v20, ac));
        (a::mul(a::sub(a::mul(ac_ac, v_ab), a::mul(ab_ac, v_ac)), bdenom),
            a::mul(a::sub(a::mul(ab_ab, v_ac), a::mul(ab_ac, v_ab)), bdenom))
    };
    let inomp = v3dot(plane_n, neg(ap));
    let ipt = if ideom > 0.0 { a::mul(inomp, frecip(ideom)) } else { 0.0 };
    let dist3 = v3dot(ap, plane_n);
    let cp31 = v3scale_add(normal, ipt, p);
    let (v0, w0) = bary(cp31);
    if valid_bary(v0, w0) && r > dist3 {
        out.push(Contact { point: cp31, normal, penetration: a::neg(ipt) });
    }
    let inomq = v3dot(plane_n, neg(aq));
    let dist4 = v3dot(aq, plane_n);
    let iqt = if ideom > 0.0 { a::mul(inomq, frecip(ideom)) } else { 0.0 };
    let cp41 = v3scale_add(normal, iqt, q);
    let (v1, w1) = bary(cp41);
    if valid_bary(v1, w1) && r > dist4 {
        out.push(Contact { point: cp41, normal, penetration: a::neg(iqt) });
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_ee_mtd(p: V3, q: V3, r: f32, normal: V3, ta: V3, tb: V3, out: &mut Vec<Contact>) {
    let ab = sub(tb, ta);
    let n = cross(ab, normal);
    let d = v3dot(n, ta);
    let np = v3dot(n, p);
    let nq = v3dot(n, q);
    let sign_p = a::sub(np, d);
    let sign_q = a::sub(nq, d);
    if a::mul(sign_p, sign_q) > 0.0 {
        return;
    }
    let pq = sub(q, p);
    let npq = v3dot(n, pq);
    if npq == 0.0 {
        return;
    }
    let seg_t = a::mul(a::sub(d, np), frecip(npq));
    let lpa = v3scale_add(pq, seg_t, p);
    let per_n = cross(normal, pq);
    let ap = sub(lpa, ta);
    let nom = v3dot(per_n, ap);
    let denom = v3dot(per_n, ab);
    let tv = fclamp(a::mul(nom, frecip(denom)), 0.0, 1.0);
    let v = v3neg_scale_sub(ab, tv, ap);
    let sd = v3dot(v, normal);
    if r > sd {
        out.push(Contact { point: sub(lpa, v), normal, penetration: sd });
    }
}

/// `PCMCapsuleVsMeshContactGeneration::processTriangle` for the MTD.
fn process_triangle(tri: [V3; 3], p0: V3, p1: V3, r: f32, flags: u8, out: &mut Vec<Contact>) {
    let [ta, tb, tc] = tri;
    let n = v3normalize(cross(sub(tb, ta), sub(tc, ta)));
    let sq_r = a::mul(r, r);
    let (sq_dist, t, u, v) = seg_tri_sq(p0, p1, ta, tb, tc);
    if !(sq_r > sq_dist) {
        return;
    }
    let patch = if select_normal(u, v, flags) || sq_dist == 0.0 {
        n
    } else {
        let pos = v3scale_add(sub(p1, p0), t, p0);
        let w = a::sub(1.0, a::add(u, v));
        let pot = add(scale(ta, w), add(scale(tb, u), scale(tc, v)));
        v3normalize(sub(pos, pot))
    };
    generate_contacts(ta, tb, tc, n, patch, p0, p1, r, out);
    generate_ee_mtd(p0, p1, r, patch, ta, tb, out);
    generate_ee_mtd(p0, p1, r, patch, tb, tc, out);
    generate_ee_mtd(p0, p1, r, patch, ta, tc, out);
}
