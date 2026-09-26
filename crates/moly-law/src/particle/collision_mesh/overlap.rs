//! The scene narrowphase of an axis-aligned box query against the mesh
//! (`GeomOverlapCallback_BoxMesh` with unit mesh scale: the box in vertex
//! space through the box midphase, `intersectTriangleBox` in its vector
//! form, the first touching triangle ends the query), the shape's world
//! bounds (`Gu::computeBounds` of a triangle mesh at inflation one), and the
//! box the scene query's static pruner stores for a static shape (the pose
//! composition and the pool box, shared with the box and convex shapes).
//! The BV4 box query is in `bv4_query`; the triangle-box test is shared.
use super::bv4_query;
use super::cook::{CookedMesh, Midphase};
use super::sweep::traverse_aabb;
use super::vector::*;
use super::{arms, finite3, Pose, Refused};
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
pub(super) fn tri_box(center: V3, ext: V3, ta: V3, tb: V3, tc: V3) -> bool {
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
    overlap_oriented_box(mesh, pose, center, extents, [0.0, 0.0, 0.0, 1.0], None)
}

/// The same for a box at a rotation (x, y, z, w; the BV4 query only takes
/// one other than the identity). `tested` receives, for the BV4 query,
/// every face a triangle-box test is made for, in call order.
pub fn overlap_oriented_box(mesh: &CookedMesh, pose: &Pose, center: [f32; 3], extents: [f32; 3],
    rotation: [f32; 4], mut tested: Option<&mut Vec<u32>>) -> Result<bool, Refused> {
    if !finite3(center) || !finite3(extents) || !rotation.iter().all(|v| v.is_finite()) {
        return Err(Refused("a non-finite overlap box"));
    }
    if let Midphase::Bv4(tree) = &mesh.midphase {
        return bv4_query::overlap_box(mesh, tree, pose, center, extents, rotation, &mut |face| {
            if let Some(t) = tested.as_deref_mut() {
                t.push(face);
            }
        });
    }
    if !pose.identity_rotation() || rotation[..3].iter().any(|&v| v != 0.0) || rotation[3] != 1.0 {
        return Err(Refused("a rotated collider or query box on the BVH33 midphase is not transcribed"));
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
    mesh_bounds(mesh, pose, 1.0)
}

/// `Gu::computeBounds` of a triangle mesh with no contact offset at an
/// inflation: the cooked local bounds' centre through the pose, the extents
/// through the absolute rotation, then the extents times the inflation.
fn mesh_bounds(mesh: &CookedMesh, pose: &Pose, inflation: f32) -> [f32; 6] {
    let m = Matrix34::from_pose(pose.rotation(), pose.translation());
    let [r0, r1, r2] = m.columns;
    let (ce, ex) = (mesh.center, mesh.extents);
    let t = pose.translation();
    let c: V3 = std::array::from_fn(|k| {
        a::add(a::add(a::add(a::mul(r0[k], ce[0]), a::mul(r1[k], ce[1])), a::mul(r2[k], ce[2])), t[k])
    });
    let e: V3 = std::array::from_fn(|k| {
        let e = a::add(a::add(a::abs(a::mul(r0[k], ex[0])), a::abs(a::mul(r1[k], ex[1]))), a::abs(a::mul(r2[k], ex[2])));
        // No contact offset.
        a::mul(a::add(e, 0.0), inflation)
    });
    [a::sub(c[0], e[0]), a::sub(c[1], e[1]), a::sub(c[2], e[2]), a::add(c[0], e[0]), a::add(c[1], e[1]), a::add(c[2], e[2])]
}

/// The pool box's growth: half of one percent of the box's size, taken off
/// the minimum and added to the maximum (the binary32 value of 0.005).
const POOL_GROWTH: f32 = f32::from_bits(0x3ba3_d70a);

/// The inflation the engine's static-shape bounds take (the binary32 value
/// of 1.01): the pruner computes them when an add hands it no simulation
/// bounds (the colliders here never take that path; it stays as a named
/// variant of the add's replay), and the scene query's flush writes them for
/// every moved static shape.
pub(super) const UNBUFFERED_INFLATION: f32 = f32::from_bits(0x3f81_47ae);

/// The pose a static shape's bounds are taken at: the actor's pose composed
/// with the shape's local pose, in the four-lane form the simulation uses
/// when it registers the shape. The local translation `v` is rotated by the
/// actor rotation `(u, w)` as `2 (u (u.v) + (v (w w - 1/2) + (u x v) w))`,
/// then the actor translation is added; the rotation is the quaternion
/// product `u bw + (b w + u x b)`, `w bw - ((ux bx + uy by) + (uz bz + 0))`.
/// The lanes the vector form carries as zero are added in, so a zero
/// product can lose its sign here. A static collider's shape keeps the
/// identity local pose: the collider places its actor instead (at the
/// node's position and rotation; a BoxCollider at the node's transform of
/// its centre), so for these colliders the composition only settles zero
/// signs.
pub fn shape_world_pose(actor: &Pose, local_rotation: [f32; 4], local_translation: [f32; 3]) -> Result<Pose, Refused> {
    if arms::on("poolSkipPose") {
        return Ok(*actor);
    }
    let [ax, ay, az, aw] = actor.rotation();
    let u = [ax, ay, az];
    let [bx, by, bz, bw] = local_rotation;
    let bv = [bx, by, bz];
    let v = local_translation;
    let cross = |p: V3, q: V3| -> V3 {
        [
            a::sub(a::mul(p[1], q[2]), a::mul(p[2], q[1])),
            a::sub(a::mul(p[2], q[0]), a::mul(p[0], q[2])),
            a::sub(a::mul(p[0], q[1]), a::mul(p[1], q[0])),
        ]
    };
    let dot = |p: V3, q: V3| a::add(a::add(a::mul(p[0], q[0]), a::mul(p[1], q[1])), a::add(a::mul(p[2], q[2]), 0.0));
    let (ub, uv) = (cross(u, bv), cross(u, v));
    let rotation = [
        a::add(a::mul(u[0], bw), a::add(a::mul(bv[0], aw), ub[0])),
        a::add(a::mul(u[1], bw), a::add(a::mul(bv[1], aw), ub[1])),
        a::add(a::mul(u[2], bw), a::add(a::mul(bv[2], aw), ub[2])),
        a::sub(a::mul(aw, bw), dot(u, bv)),
    ];
    let half = a::add(a::mul(aw, aw), -0.5);
    let d = dot(u, v);
    let t = actor.translation();
    let translation: V3 = std::array::from_fn(|k| {
        let r = a::add(a::mul(u[k], d), a::add(a::mul(v[k], half), a::mul(uv[k], aw)));
        a::add(t[k], a::add(r, r))
    });
    Pose::rotated(rotation, translation)
}

/// The box the scene query's static pruner stores for a static shape whose
/// actor enters the scene with its simulation bounds handed over (the
/// ordinary add: simulation enabled, the scene not simulating, at most eight
/// shapes on the actor): those bounds (the shape's world bounds at
/// inflation one and no contact offset, at the composed pose) grown on
/// every side by `POOL_GROWTH` times their size. It is not the bounds at
/// inflation 1.01, which the pruner computes only when it is handed none.
pub(super) fn pool_box(bounds: [f32; 6]) -> [f32; 6] {
    if arms::on("poolInflationOne") {
        return bounds;
    }
    if arms::on("poolAboutCentre") {
        let c: V3 = std::array::from_fn(|k| a::mul(a::add(bounds[k], bounds[3 + k]), 0.5));
        let h: V3 =
            std::array::from_fn(|k| a::mul(a::mul(a::sub(bounds[3 + k], bounds[k]), 0.5), UNBUFFERED_INFLATION));
        return [a::sub(c[0], h[0]), a::sub(c[1], h[1]), a::sub(c[2], h[2]), a::add(c[0], h[0]), a::add(c[1], h[1]),
            a::add(c[2], h[2])];
    }
    let growth = if arms::on("poolFullSize") { a::add(POOL_GROWTH, POOL_GROWTH) } else { POOL_GROWTH };
    let e: V3 = std::array::from_fn(|k| a::mul(a::sub(bounds[3 + k], bounds[k]), growth));
    [a::sub(bounds[0], e[0]), a::sub(bounds[1], e[1]), a::sub(bounds[2], e[2]), a::add(bounds[3], e[0]),
        a::add(bounds[4], e[1]), a::add(bounds[5], e[2])]
}

/// The static pruner's box for a triangle mesh shape at `pose`, the
/// composed pose of `shape_world_pose`.
pub fn pool_bounds_mesh(mesh: &CookedMesh, pose: &Pose) -> [f32; 6] {
    if arms::on("poolUnbufferedRecipe") {
        return mesh_bounds(mesh, pose, UNBUFFERED_INFLATION);
    }
    pool_box(world_bounds(mesh, pose))
}

/// The replay of the static pruner's boxes against the engine's rows, shared
/// by the three shape types: per case the composed pose, the bounds at
/// inflation one and the pool box must equal the engine's bits, and the
/// unbuffered recipe (the named variant) must equal the engine's bounds at
/// 1.01; each named variant must differ from the engine on some case.
#[cfg(test)]
pub(super) mod pool_replay {
    use super::super::{arms, Pose};
    use super::shape_world_pose;
    use crate::particle::json::{parse, Value};
    use std::collections::BTreeMap;

    pub(crate) const ARMS: [&str; 5] =
        ["poolInflationOne", "poolAboutCentre", "poolFullSize", "poolUnbufferedRecipe", "poolSkipPose"];

    pub(crate) fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    pub(crate) fn word(v: &Value) -> u32 {
        let x = v.as_f64().expect("row word");
        assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
        x as u32
    }
    pub(crate) fn words(v: &Value) -> Vec<u32> {
        v.as_array().expect("row array").iter().map(word).collect()
    }
    fn f4(w: &[u32]) -> [f32; 4] {
        [w[0], w[1], w[2], w[3]].map(f32::from_bits)
    }
    fn f3(w: &[u32]) -> [f32; 3] {
        [w[0], w[1], w[2]].map(f32::from_bits)
    }
    fn bits<const N: usize>(v: [f32; N]) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    pub(crate) fn load() -> Value {
        let path = std::env::var("MOLY_POOL_BOUNDS_ROWS").expect("MOLY_POOL_BOUNDS_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        assert_eq!(word(field(&doc, "simInflationBits")), 0x3f80_0000, "the simulation bounds are at inflation one");
        assert_eq!(word(field(&doc, "simContactOffsetBits")), 0, "the simulation bounds have no contact offset");
        assert_eq!(word(field(&doc, "inflateScaleBits")), super::POOL_GROWTH.to_bits(), "the pool growth");
        assert_eq!(word(field(&doc, "fallbackInflationBits")), super::UNBUFFERED_INFLATION.to_bits());
        doc
    }

    type Bounds<'a> = &'a dyn Fn(&Value, &Pose) -> Option<[f32; 6]>;

    /// Per case, the fields that differ from the engine's; with `unbuffered`
    /// the pool box is compared with the engine's bounds at 1.01 instead.
    fn case_mismatch(c: &Value, sim: Bounds, pool: Bounds, unbuffered: bool) -> Vec<&'static str> {
        let a = words(field(c, "actorPoseBits"));
        let l = words(field(c, "localPoseBits"));
        let actor = Pose::rotated(f4(&a), f3(&a[4..])).expect("actor pose");
        let Ok(pose) = shape_world_pose(&actor, f4(&l), f3(&l[4..])) else {
            return vec!["refused"];
        };
        let mut bad = Vec::new();
        let mut got = bits(pose.rotation());
        got.extend(bits(pose.translation()));
        if got != words(field(c, "globalPoseBits")) {
            bad.push("pose");
        }
        if sim(c, &pose).map(bits) != Some(words(field(c, "simBoundsBits"))) {
            bad.push("sim");
        }
        let want = if unbuffered { "fallbackBoundsBits" } else { "poolBoundsBits" };
        if pool(c, &pose).map(bits) != Some(words(field(c, want))) {
            bad.push("pool");
        }
        bad
    }

    pub(crate) fn replay(doc: &Value, shape: &str, sim: Bounds, pool: Bounds) {
        let cases: Vec<&Value> = field(doc, "cases").as_array().expect("cases").iter()
            .filter(|c| field(c, "shape").as_str() == Some(shape)).collect();
        let pass = |unbuffered: bool| {
            let mut reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
            let mut failures = Vec::new();
            for (i, c) in cases.iter().enumerate() {
                let bad = case_mismatch(c, sim, pool, unbuffered);
                for r in &bad {
                    *reasons.entry(r).or_default() += 1;
                }
                if !bad.is_empty() {
                    failures.push(format!("case {i}: {bad:?}"));
                }
            }
            (failures, reasons)
        };
        arms::set(None);
        let (failures, reasons) = pass(false);
        arms::set(Some("poolUnbufferedRecipe"));
        let (unbuffered, _) = pass(true);
        let mut red = BTreeMap::new();
        for arm in ARMS {
            arms::set(Some(arm));
            let (f, r) = pass(false);
            red.insert(arm, (f.len(), r));
        }
        arms::set(None);
        println!("pool bounds replay ({shape}): {} cases, differing {} {reasons:?}; the recipe at 1.01 differs from \
            the engine's on {}; arms red (cases, fields) {red:?}", cases.len(), failures.len(), unbuffered.len());
        for failure in failures.iter().chain(&unbuffered).take(20) {
            println!("  {failure}");
        }
        assert!(!cases.is_empty());
        assert!(failures.is_empty(), "{} {shape} pool boxes differ from the engine's", failures.len());
        assert!(unbuffered.is_empty(), "{} {shape} bounds at 1.01 differ from the engine's", unbuffered.len());
        for arm in ARMS {
            assert!(red[arm].0 > 0, "arm {arm} stays green");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::cook::cook;
    use super::pool_replay::{field, load, replay, word, words};
    use super::*;
    use crate::particle::json::Value;

    /// The engine's pool boxes of the site meshes (natively cooked with the
    /// options 30) at identity, signed-zero, quarter-turn and random actor
    /// poses and identity local poses of either zero sign.
    #[test]
    #[ignore = "needs MOLY_POOL_BOUNDS_ROWS"]
    fn pool_bounds_mesh_match_native_bits() {
        let doc = load();
        let meshes: Vec<CookedMesh> = field(&doc, "meshes").as_array().expect("meshes").iter().map(|m| {
            let p = words(field(m, "positionBits"));
            let t = words(field(m, "triangles"));
            let positions: Vec<[f32; 3]> = p.chunks_exact(3).map(|c| [c[0], c[1], c[2]].map(f32::from_bits)).collect();
            let triangles: Vec<[u32; 3]> = t.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            let mesh = cook(&positions, &triangles, word(field(m, "cookingOptions"))).expect("cook");
            let local: Vec<u32> = mesh.center.iter().chain(&mesh.extents).map(|x| x.to_bits()).collect();
            assert_eq!(local, words(field(m, "centerExtentsBits")), "cooked local bounds");
            mesh
        }).collect();
        let at = |c: &Value| word(field(c, "index")) as usize;
        replay(&doc, "mesh", &|c, p| Some(world_bounds(&meshes[at(c)], p)),
            &|c, p| Some(pool_bounds_mesh(&meshes[at(c)], p)));
    }
}
