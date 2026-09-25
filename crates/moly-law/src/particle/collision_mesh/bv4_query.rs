//! The queries against a cooked BV4 mesh (identity mesh scale, a
//! single-sided mesh, zero inflation), as the engine's build executes them:
//!
//! - the sphere sweep: the world matrix set up by the pose's bits (none when
//!   the rotation bits are exactly (0, 0, 0, 1) and the translation bits are
//!   zero), the local ray (`R^T c - R^T t`), the ordered traversal of the
//!   quantised nodes (node bounds widened by the radius and 1e-7, the
//!   reciprocal direction by FRECPE, one FRECPS step and one unfused Newton
//!   step, lanes tested 3 down to 0 against the running distance plus 1e-3,
//!   leaves swept at once, inner nodes pushed in the order the sorting bits
//!   of the direction's octant give), or every triangle when the tree has no
//!   nodes; `triSphereSweep` (backface cull, the sphere-triangle sweep with
//!   the initial overlap first, the distance gate `<=`, keepTriangle with
//!   the relative epsilon inline) and the impact data on the world triangle;
//! - the MTD midphase: the box around the sphere in vertex space (the
//!   inverse pose matrix, the unit mesh scale's inverse, the box transform
//!   and its orthonormalisation), the unordered traversal with the
//!   box-versus-node test on the dequantised bounds and every triangle of a
//!   touched leaf tested against the box at once, reported in that order;
//! - the scene overlap of a box: the box in local space, the same traversal,
//!   the first touching triangle ends it.
use super::bv4_cook::Bv4Tree;
use super::cook::CookedMesh;
use super::overlap::tri_box;
use super::sweep::{closest_pt_point_triangle, sweep_sphere_vs_tri, Hit};
use super::vector::*;
use super::{arms, Pose, Refused};
use crate::particle::armf as a;

/// 1e-7: the traversal widens the node bounds by the radius plus this.
const FATTEN: f32 = f32::from_bits(0x33d6_bf95);
/// 1e-9: the floor of a ray direction component's magnitude.
const DIR_FLOOR: f32 = f32::from_bits(0x3089_705f);
/// 1e-6: the box test's widening of the absolute rotation.
const PRECA_EPS: f32 = f32::from_bits(0x3586_37bd);
const INVALID: u32 = 0xffff_ffff;
const IDENTITY_ROTATION_BITS: [u32; 4] = [0, 0, 0, 0x3f80_0000];

/// The world matrix of a pose as the sweep sets it up: `None` for the
/// identity bit patterns.
fn world_matrix(pose: &Pose) -> Option<Matrix34> {
    let (q, t) = (pose.rotation(), pose.translation());
    let mut m = Matrix34 { columns: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation: [0.0; 3] };
    let mut identity = true;
    if q.map(f32::to_bits) != IDENTITY_ROTATION_BITS {
        m.columns = Matrix34::from_pose(q, [0.0; 3]).columns;
        identity = false;
    }
    if t.map(f32::to_bits) != [0; 3] {
        m.translation = t;
        identity = false;
    }
    (!identity).then_some(m)
}

fn local_ray(tm: Option<&Matrix34>, d: V3, o: V3) -> (V3, V3) {
    let Some(tm) = tm else { return (d, o) };
    let [c0, c1, c2] = tm.columns;
    let t = tm.translation;
    ([dot(c0, d), dot(c1, d), dot(c2, d)], [c0, c1, c2].map(|c| a::sub(dot(c, o), dot(c, t))))
}

fn dequantise(word: u32, min_coeff: f32, max_coeff: f32) -> (f32, f32) {
    let lo = (word & 0xffff) as u16 as i16;
    let hi = (word >> 16) as u16 as i16;
    (a::mul(min_coeff, lo as f32), a::mul(max_coeff, hi as f32))
}

fn node_words(tree: &Bv4Tree, data: u32) -> Result<&[u32], Refused> {
    let base = (data >> 11) as usize * 4;
    tree.words.get(base..base + 16).ok_or(Refused("a BV4 node word past the tree"))
}

/// The lanes a node's type holds, tested from 3 down to 0 (in the box
/// traversal's forward-lane arm, 0 up to 3).
fn lanes(data: u32, forward: bool) -> impl Iterator<Item = usize> {
    let ntype = (data >> 1) & 3;
    let order: [usize; 4] = if forward { [0, 1, 2, 3] } else { [3, 2, 1, 0] };
    order.into_iter().filter(move |&x| !(x == 3 && ntype <= 1) && !(x == 2 && ntype == 0))
}

/// The running state of `BV4_SphereSweepSingle`.
struct SweepState<'t> {
    mesh: &'t CookedMesh,
    radius: f32,
    ldir: V3,
    lorigin: V3,
    max_dist: f32,
    stab_dist: f32,
    stab_id: u32,
    best_dist: f32,
    best_align: f32,
    corners: [V3; 3],
    trace: Option<&'t mut Vec<u32>>,
}

impl SweepState<'_> {
    fn tri_sphere_sweep(&mut self, prim: u32) -> Result<(), Refused> {
        if let Some(trace) = self.trace.as_deref_mut() {
            trace.push(prim);
        }
        let [p0, p1, p2] = self.mesh.corners(prim).ok_or(Refused("a BV4 leaf past the cooked triangles"))?;
        let n = cross(sub(p1, p0), sub(p2, p0));
        if dot(n, self.ldir) > 0.0 {
            return Ok(());
        }
        let n = normalize(n).0;
        let cp = closest_pt_point_triangle(self.lorigin, p0, p1, p2);
        let dist = if mag2(sub(cp, self.lorigin)) <= a::mul(self.radius, self.radius) {
            0.0
        } else {
            match sweep_sphere_vs_tri([p0, p1, p2], n, self.lorigin, self.radius, self.ldir) {
                Some(t) => t,
                None => return Ok(()),
            }
        };
        let within = if arms::on("bv4StrictMaxDist") { dist < self.max_dist } else { dist <= self.max_dist };
        if !within {
            return Ok(());
        }
        let align = a::neg(a::abs(dot(n, self.ldir)));
        let best = self.best_dist;
        let eps = a::mul(a::max(sel_max(dist, best), 1.0), EPS_SAME);
        let keep = a::sub(best, eps) > dist
            || (self.best_align > align && a::add(best, eps) > dist)
            || dist == 0.0
            || (self.best_align == align && dist < best);
        if keep {
            self.stab_dist = dist;
            self.stab_id = prim;
            self.corners = [p0, p1, p2];
            self.best_dist = if dist > best { best } else { dist };
            self.best_align = align;
        }
        Ok(())
    }

    fn leaf(&mut self, data: u32) -> Result<(), Refused> {
        let prim = data >> 1;
        let (start, count) = (prim >> 4, prim & 15);
        for i in 0..count {
            self.tri_sphere_sweep(start + i)?;
        }
        Ok(())
    }

    fn traverse(&mut self, tree: &Bv4Tree) -> Result<(), Refused> {
        let fat = a::add(self.radius, FATTEN);
        let dd: V3 = std::array::from_fn(|k| {
            f32::from_bits((self.ldir[k].to_bits() & 0x8000_0000) | a::max(a::abs(self.ldir[k]), DIR_FLOOR).to_bits())
        });
        let inv: V3 = std::array::from_fn(|k| {
            let e = a::recip_estimate(dd[k]);
            let e = a::mul(e, a::recip_step(e, dd[k]));
            a::mul(e, a::sub(2.0, a::mul(e, dd[k])))
        });
        let pinv: V3 = std::array::from_fn(|k| a::sub(0.0, a::mul(self.lorigin[k], inv[k])));
        let sign = |v: f32| v.to_bits() >> 31;
        let dir_mask = 1u32 << (3 + (sign(self.ldir[2]) | (sign(self.ldir[1]) << 1) | (sign(self.ldir[0]) << 2)));
        let mut stack = vec![tree.init_data];
        while let Some(cd) = stack.pop() {
            let node = node_words(tree, cd)?;
            let max_t = self.stab_dist;
            let mut nears = [0.0f32; 4];
            let mut culled = 0u32;
            for lane in 0..4 {
                let mut tmin = [0.0f32; 3];
                let mut tmax = [0.0f32; 3];
                for axis in 0..3 {
                    let (lo, hi) = dequantise(node[axis * 4 + lane], tree.min_coeff[axis], tree.max_coeff[axis]);
                    let mn = a::sub(lo, fat);
                    let mx = a::add(fat, hi);
                    let t0 = a::add(pinv[axis], a::mul(inv[axis], mn));
                    let t1 = a::add(pinv[axis], a::mul(inv[axis], mx));
                    tmin[axis] = a::min(t0, t1);
                    tmax[axis] = a::max(t0, t1);
                }
                let near = a::max(a::max(tmin[0], tmin[1]), tmin[2]);
                let far = a::min(a::min(tmax[0], tmax[1]), tmax[2]);
                nears[lane] = near;
                if DIR_FLOOR > far || near > max_t || near > far {
                    culled |= 1 << lane;
                }
            }
            if culled == 15 {
                continue;
            }
            let mut inner = 0u32;
            for x in lanes(cd, false) {
                if culled & (1 << x) != 0 {
                    continue;
                }
                if !(nears[x] < a::add(self.stab_dist, EPS_SAME)) {
                    continue;
                }
                let data = node[12 + x];
                if data & 1 != 0 {
                    self.leaf(data)?;
                } else {
                    inner |= 1 << x;
                }
            }
            if inner != 0 {
                let (d0, d1, d2) = (node[12] & dir_mask != 0, node[13] & dir_mask != 0, node[14] & dir_mask != 0);
                let order: [usize; 4] = match (d0, d1, d2) {
                    (true, true, true) => [3, 2, 1, 0],
                    (true, true, false) => [2, 3, 1, 0],
                    (true, false, true) => [3, 2, 0, 1],
                    (true, false, false) => [2, 3, 0, 1],
                    (false, true, true) => [1, 0, 3, 2],
                    (false, true, false) => [1, 0, 2, 3],
                    (false, false, true) => [0, 1, 3, 2],
                    (false, false, false) => [0, 1, 2, 3],
                };
                let pushed: Vec<usize> = if arms::on("bv4PnsReversed") {
                    order.into_iter().rev().collect()
                } else {
                    order.to_vec()
                };
                for x in pushed {
                    if inner & (1 << x) != 0 {
                        stack.push(node[12 + x]);
                    }
                }
            }
        }
        Ok(())
    }
}

/// `sweepCapsule_MeshGeom_BV4` for a sphere (both ends at the centre): the
/// cast. Returns the hit and whether it is at distance zero (the caller runs
/// the MTD then). `radius` is already the inflated radius (inflation zero);
/// `faces` receives the triangles triSphereSweep is called for, in order.
pub(super) fn sweep(mesh: &CookedMesh, tree: &Bv4Tree, pose: &Pose, center: V3, radius: f32, direction: V3,
    distance: f32, faces: Option<&mut Vec<u32>>) -> Result<Option<(Hit, bool)>, Refused> {
    let tm = world_matrix(pose);
    let (ldir, lorigin) = local_ray(tm.as_ref(), direction, center);
    let mut state = SweepState {
        mesh,
        radius,
        ldir,
        lorigin,
        max_dist: distance,
        stab_dist: distance,
        stab_id: INVALID,
        best_dist: FLT_MAX,
        best_align: 2.0,
        corners: [[0.0; 3]; 3],
        trace: faces,
    };
    if tree.words.is_empty() {
        let n = mesh.triangles.len() as u32;
        let (start, count) = (n >> 4, n & 15);
        for i in 0..count {
            state.tri_sphere_sweep(start + i)?;
        }
    } else {
        state.traverse(tree)?;
    }
    if state.stab_id == INVALID {
        return Ok(None);
    }
    let t = state.stab_dist;
    if t == 0.0 {
        let hit = Hit { distance: 0.0, face: state.stab_id, normal: neg(direction), position: Some([0.0; 3]) };
        return Ok(Some((hit, true)));
    }
    let world = state.corners.map(|v| tm.as_ref().map_or(v, |m| m.transform(v)));
    // computeSphereTriImpactData on the world triangle.
    let new_center = add(center, scale(direction, t));
    let hit_point = closest_pt_point_triangle(new_center, world[0], world[1], world[2]);
    let (mut normal, m) = normalize(sub(new_center, hit_point));
    if m < EPS_SAME {
        normal = normalize(cross(sub(world[1], world[0]), sub(world[2], world[0]))).0;
    }
    Ok(Some((Hit { distance: t, face: state.stab_id, normal, position: Some(hit_point) }, false)))
}

/// An oriented box: rotation columns, centre, half extents.
#[derive(Clone, Copy, Debug)]
struct Obb {
    rot: [V3; 3],
    center: V3,
    ext: V3,
}

/// `(c0*v.x + c1*v.y) + c2*v.z`.
fn mat_col(cols: [V3; 3], v: V3) -> V3 {
    let [c0, c1, c2] = cols;
    std::array::from_fn(|k| a::add(a::add(a::mul(c0[k], v[0]), a::mul(c1[k], v[1])), a::mul(c2[k], v[2])))
}

/// The matrix of the pose's inverse as the vertex-space box builds it.
fn inverse_pose_matrix(pose: &Pose) -> Matrix34 {
    let [qx, qy, qz, qw] = pose.rotation();
    let [px, py, pz] = pose.translation();
    let m2 = -2.0f32;
    let (vx, vy, vz) = (a::mul(px, m2), a::mul(py, m2), a::mul(pz, m2));
    let w2 = a::add(a::mul(qw, qw), -0.5);
    let s3 = a::sub(a::mul(vy, qx), a::mul(vx, qy));
    let (x2, y2, z2) = (a::sub(a::neg(qx), qx), a::sub(a::neg(qy), qy), a::sub(a::neg(qz), qz));
    let dot2 = a::add(a::add(a::mul(vx, qx), a::mul(vy, qy)), a::mul(vz, qz));
    let s23 = a::sub(a::mul(vz, qy), a::mul(vy, qz));
    let s25 = a::sub(a::mul(vx, qz), a::mul(vz, qx));
    let tz = a::sub(a::mul(vz, w2), a::mul(qw, s3));
    let tx = a::sub(a::mul(vx, w2), a::mul(qw, s23));
    let ty = a::sub(a::mul(vy, w2), a::mul(qw, s25));
    let tx = a::add(a::mul(qx, dot2), tx);
    let ty = a::add(a::mul(qy, dot2), ty);
    let tz = a::add(tz, a::mul(qz, dot2));
    let xxn = a::mul(x2, qx);
    let yy = a::mul(y2, a::neg(qy));
    let zz = a::mul(z2, a::neg(qz));
    let xy = a::mul(x2, a::neg(qy));
    let xz = a::mul(x2, a::neg(qz));
    let xw = a::mul(qw, x2);
    let yz = a::mul(y2, a::neg(qz));
    let yw = a::mul(qw, y2);
    let zw = a::mul(qw, z2);
    let one_xx = a::add(xxn, 1.0);
    Matrix34 {
        columns: [
            [a::sub(a::sub(1.0, yy), zz), a::add(xy, zw), a::sub(xz, yw)],
            [a::sub(xy, zw), a::sub(one_xx, zz), a::add(yz, xw)],
            [a::add(xz, yw), a::sub(yz, xw), a::sub(one_xx, yy)],
        ],
        translation: [tx, ty, tz],
    }
}

/// `optimizeBoundingBox`: the columns orthonormalised (largest first) and
/// the extents they carry.
fn optimize_bounding_box(cols: [V3; 3]) -> (V3, [V3; 3]) {
    let mut v = cols;
    let mut mag = cols.map(mag2);
    let mut i = if mag[1] > mag[0] { 1 } else { 0 };
    let mut j = if mag[2] > mag[1 - i] { 2 } else { 1 - i };
    let k = 3 - i - j;
    if mag[i] < mag[j] {
        std::mem::swap(&mut i, &mut j);
    }
    let inv = a::div(1.0, a::sqrt(mag[i]));
    mag[i] = a::mul(mag[i], inv);
    v[i] = [a::mul(v[i][0], inv), a::mul(v[i][1], inv), a::mul(inv, v[i][2])];
    let dij = dot(v[i], v[j]);
    let dik = dot(v[i], v[k]);
    mag[i] = a::add(mag[i], a::add(a::abs(dij), a::abs(dik)));
    v[j] = std::array::from_fn(|c| a::sub(v[j][c], a::mul(v[i][c], dij)));
    v[k] = std::array::from_fn(|c| a::sub(v[k][c], a::mul(v[i][c], dik)));
    let m = a::sqrt(mag2(v[j]));
    if m > 0.0 {
        let r = a::div(1.0, m);
        v[j] = [a::mul(r, v[j][0]), a::mul(r, v[j][1]), a::mul(r, v[j][2])];
    }
    let djk = dot(v[j], v[k]);
    mag[j] = a::add(m, a::abs(djk));
    v[k] = std::array::from_fn(|c| a::sub(v[k][c], a::mul(v[j][c], djk)));
    let m = a::sqrt(mag2(v[k]));
    if m > 0.0 {
        let r = a::div(1.0, m);
        v[k] = [a::mul(v[k][0], r), a::mul(v[k][1], r), a::mul(r, v[k][2])];
    }
    mag[k] = m;
    (mag, v)
}

/// The sphere's box in vertex space: `computeBoxAroundCapsule` for equal
/// ends, then the inverse pose (times the unit mesh scale's inverse, exactly
/// the identity with positive-zero off-diagonals) applied to the box.
fn vertex_space_box(pose: &Pose, center: V3, r: f32) -> Obb {
    let c: V3 = std::array::from_fn(|k| a::mul(a::add(center[k], center[k]), 0.5));
    let d = a::sqrt(mag2(sub(center, center)));
    let box_ext = [a::add(r, a::mul(d, 0.5)), r, r];
    let box_rot = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let inv = inverse_pose_matrix(pose);
    let s = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let cols = inv.columns.map(|col| mat_col(s, col));
    let p = mat_col(s, inv.translation);
    let rotated: [V3; 3] = std::array::from_fn(|j| mat_col(cols, box_rot[j].map(|x| a::mul(box_ext[j], x))));
    let cen: V3 = std::array::from_fn(|k| {
        a::add(a::add(a::add(a::mul(cols[0][k], c[0]), a::mul(cols[1][k], c[1])), a::mul(cols[2][k], c[2])), p[k])
    });
    let (ext, rot) = optimize_bounding_box(rotated);
    Obb { rot, center: cen, ext }
}

/// The precomputed box of `setupBoxParams`.
struct BoxParams {
    /// Rows of the box rotation (the columns of its transpose).
    r: [V3; 3],
    t: V3,
    center: V3,
    ext: V3,
    p: [V3; 3],
    pb: [V3; 3],
    bb: V3,
}

fn box_params(b: &Obb) -> BoxParams {
    let [c0, c1, c2] = b.rot;
    let r = [[c0[0], c1[0], c2[0]], [c0[1], c1[1], c2[1]], [c0[2], c1[2], c2[2]]];
    let t = [c0, c1, c2].map(|c| {
        a::neg(a::add(a::add(a::mul(b.center[0], c[0]), a::mul(b.center[1], c[1])), a::mul(b.center[2], c[2])))
    });
    let abs_plus = |c: V3| c.map(|x| a::add(a::abs(x), PRECA_EPS));
    let (a0, a1, a2) = (abs_plus(c0), abs_plus(c1), abs_plus(c2));
    BoxParams {
        r,
        t,
        center: b.center,
        ext: b.ext,
        p: [[c0[0], c1[1], c2[2]], [c0[1], c1[2], c2[0]], [c0[2], c1[0], c2[1]]],
        pb: [[a0[0], a1[1], a2[2]], [a0[1], a1[2], a2[0]], [a0[2], a1[0], a2[1]]],
        bb: std::array::from_fn(|k| {
            a::add(a::add(a::mul(a0[k], b.ext[0]), a::mul(a1[k], b.ext[1])), a::mul(a2[k], b.ext[2]))
        }),
    }
}

/// The box against a node (centre `c`, half extents `e`).
fn box_node(p: &BoxParams, c: V3, e: V3) -> bool {
    for k in 0..3 {
        if a::abs(a::sub(p.center[k], c[k])) > a::add(p.bb[k], e[k]) {
            return false;
        }
    }
    let tv = sub(p.center, c);
    let yzx = [tv[1], tv[2], tv[0]];
    let zxy = [tv[2], tv[0], tv[1]];
    let eyzx = [e[1], e[2], e[0]];
    let ezxy = [e[2], e[0], e[1]];
    for k in 0..3 {
        let t = a::add(a::add(a::mul(tv[k], p.p[0][k]), a::mul(p.p[1][k], yzx[k])), a::mul(p.p[2][k], zxy[k]));
        let t2 = a::add(p.ext[k],
            a::add(a::add(a::mul(e[k], p.pb[0][k]), a::mul(p.pb[1][k], eyzx[k])), a::mul(p.pb[2][k], ezxy[k])));
        if a::abs(t) > t2 {
            return false;
        }
    }
    true
}

/// `intersectTriangleBoxBV4`: the corners in box space, then the box test
/// about the origin.
fn tri_box_bv4(p: &BoxParams, v: [V3; 3]) -> bool {
    let local = v.map(|x| {
        let m = mat_col(p.r, x);
        std::array::from_fn(|k| a::add(p.t[k], m[k]))
    });
    tri_box([0.0; 3], p.ext, local[0], local[1], local[2])
}

/// The unordered traversal; `hit(face)` returns whether to stop, `tested`
/// sees every face a triangle-box test is made for, in call order. Returns
/// whether it stopped.
fn traverse_boxes(mesh: &CookedMesh, tree: &Bv4Tree, p: &BoxParams, tested: &mut dyn FnMut(u32),
    mut hit: impl FnMut(u32) -> bool) -> Result<bool, Refused> {
    let mut leaf = |data: u32, tested: &mut dyn FnMut(u32)| -> Result<bool, Refused> {
        let prim = data >> 1;
        let (start, count) = (prim >> 4, prim & 15);
        for i in 0..count {
            let corners = mesh.corners(start + i).ok_or(Refused("a BV4 leaf past the cooked triangles"))?;
            tested(start + i);
            if tri_box_bv4(p, corners) && hit(start + i) {
                return Ok(true);
            }
        }
        Ok(false)
    };
    if tree.words.is_empty() {
        let n = mesh.triangles.len() as u32;
        return leaf(((((n >> 4) << 4) | (n & 15)) << 1) | 1, tested);
    }
    let mut stack = vec![tree.init_data];
    while let Some(cd) = stack.pop() {
        let node = node_words(tree, cd)?;
        for x in lanes(cd, arms::on("bv4LanesForward")) {
            let mut c = [0.0f32; 3];
            let mut e = [0.0f32; 3];
            for axis in 0..3 {
                let (mn, mx) = dequantise(node[axis * 4 + x], tree.min_coeff[axis], tree.max_coeff[axis]);
                c[axis] = a::mul(a::add(mn, mx), 0.5);
                e[axis] = a::mul(a::sub(mx, mn), 0.5);
            }
            if !box_node(p, c, e) {
                continue;
            }
            let data = node[12 + x];
            if data & 1 != 0 {
                if leaf(data, tested)? {
                    return Ok(true);
                }
            } else {
                stack.push(data);
            }
        }
    }
    Ok(false)
}

/// The MTD midphase of a sphere at `center`: the faces the box touches, in
/// report order; `tests` counts the triangle-box tests.
pub(super) fn mtd_midphase(mesh: &CookedMesh, tree: &Bv4Tree, pose: &Pose, center: V3, r: f32, tests: &mut usize)
    -> Result<Vec<u32>, Refused> {
    let p = box_params(&vertex_space_box(pose, center, r));
    let mut found = Vec::new();
    traverse_boxes(mesh, tree, &p, &mut |_: u32| *tests += 1, |face| {
        found.push(face);
        false
    })?;
    Ok(found)
}

/// Whether a box (world centre, half extents, rotation quaternion) touches
/// a triangle of the mesh at `pose`; `tested` sees each tested face.
pub(super) fn overlap_box(mesh: &CookedMesh, tree: &Bv4Tree, pose: &Pose, center: V3, extents: V3, rotation: [f32; 4],
    tested: &mut dyn FnMut(u32)) -> Result<bool, Refused> {
    let rot = Matrix34::from_pose(rotation, [0.0; 3]).columns;
    let mut b = Obb { rot, center, ext: extents };
    if let Some(tm) = world_matrix(pose) {
        // computeLocalBox: the inverse rotation's rows are the forward columns.
        let rows = tm.columns;
        let nt = rows.map(|r| dot(tm.translation, r));
        b.center = std::array::from_fn(|k| a::sub(dot(rows[k], center), nt[k]));
        b.rot = rot.map(|rc| rows.map(|r| dot(r, rc)));
    }
    let p = box_params(&b);
    traverse_boxes(mesh, tree, &p, tested, |_| true)
}
