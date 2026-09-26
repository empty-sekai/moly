//! The sphere sweep against a cooked convex mesh: `PxGeometryQuery::sweep`
//! for a sphere against a `PxConvexMeshGeometry` with the identity mesh
//! scale, the hit flags normal and MTD and zero inflation, as the engine's
//! PhysX 4.1 build runs it. The sphere is a capsule of zero half height; the
//! capsule-convex sweep takes the capsule into the mesh's frame and runs the
//! same GJK raycast, penetration and EPA as the capsule-box sweep, with the
//! hull's support mapping in place of the box's.
//!
//! The hull's support mapping reads the cooked hull vertices. With the
//! identity scale the vertex-space direction is the direction through the
//! identity columns (each lane a pairwise sum of products, the last pair
//! with the zero fourth lane), and a vertex comes back to the shape frame
//! the same way. A hull that carries a gauss map (more vertices than the
//! cooking's limit of 32) searches by hill climbing: the cube-map sample
//! nearest the direction gives the start vertex, and the search moves to a
//! neighbour whose projection is strictly greater and not yet visited in
//! this call, until no neighbour improves (a visited vertex never projects
//! above the running maximum, so the visited set never changes the answer).
//! Without a gauss map the search is
//! the first vertex of the greatest projection, in cooked order. The
//! margins are the smallest internal extent times 0.1 (margin), 0.05
//! (minimum margin) and 0.025 (sweep margin); the hull is not quadratic, so
//! its margin adds nothing to the surface points or the depth.
//!
//! After the query the normal is rotated back and normalized; an initial
//! overlap puts the position at the contact point pushed out along the
//! normal by the depth and sets the face index to -1, and a sweep hit puts
//! it at the contact point moved along the sweep by the hit distance and
//! leaves the face index as the caller set it (the flags ask for no face
//! index).
use super::box_sweep::{self as bs, BoxSweepTrace, Capsule, SweptShape};
use super::sweep::{MeshSweepHit, FLAG_NORMAL, FLAG_POSITION};
use super::vector::{add, neg, scale, sub, v3neg_scale_sub, v3scale_add, Matrix34};
use super::{arms, finite3, Pose, Refused};
use crate::particle::armf as a;

type V3 = [f32; 3];

const ZERO3: V3 = [0.0; 3];
const UNIT_X: V3 = [1.0, 0.0, 0.0];
/// The face index an initial overlap writes.
const OVERLAP_FACE: u32 = u32::MAX;
/// The columns of the identity mesh scale's vertex-to-shape matrix.
const IDENTITY: [V3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// A cooked hull holds at most 255 vertices (the count is one byte).
const MAX_HULL_VERTICES: usize = 255;

/// What the sweep reads of a cooked convex mesh: the hull vertices in their
/// cooked order, the internal extents the margins come from, and the gauss
/// map when the hull carries one.
#[derive(Clone, Debug, PartialEq)]
pub struct HullSupport {
    pub vertices: Vec<[f32; 3]>,
    pub internal_extents: [f32; 3],
    pub gauss_map: Option<GaussMap>,
}

/// The cooked hill-climbing data: the cube-map subdivision, the start
/// vertex of every cube-map sample, and per hull vertex its neighbour count
/// and offset into the adjacency list.
#[derive(Clone, Debug, PartialEq)]
pub struct GaussMap {
    pub subdiv: u16,
    pub samples: Vec<u8>,
    pub valencies: Vec<(u16, u16)>,
    pub adjacent: Vec<u8>,
}

/// The hull in its own frame with the identity scale.
struct HullShape<'h> {
    hull: &'h HullSupport,
    margin: f32,
    min_margin: f32,
    sweep_margin: f32,
}

/// `(c.x*d.x + c.y*d.y) + (c.z*d.z + 0)`: a column's pairwise dot with a
/// direction whose fourth lane is zero.
fn pair_dot(c: V3, d: V3) -> f32 {
    a::add(a::add(a::mul(d[0], c[0]), a::mul(d[1], c[1])), a::add(a::mul(d[2], c[2]), a::mul(0.0, 0.0)))
}

impl<'h> HullShape<'h> {
    fn new(hull: &'h HullSupport) -> Self {
        let e = hull.internal_extents.map(|x| a::mul(1.0, x));
        let min = a::min(e[2], a::min(e[0], e[1]));
        Self { hull, margin: a::mul(0.1, min), min_margin: a::mul(0.05, min), sweep_margin: a::mul(0.025, min) }
    }

    /// The direction in vertex space (the transposed identity columns).
    fn to_vertex_space(dir: V3) -> V3 {
        IDENTITY.map(|c| pair_dot(c, dir))
    }

    /// A hull vertex in the shape frame.
    fn vertex(&self, index: usize) -> V3 {
        bs::columns_mul(&IDENTITY, self.hull.vertices[index])
    }

    /// `ConvexHullV::supportVertexIndex` for a vertex-space direction.
    fn support_vertex_index(&self, dir: V3) -> Result<usize, Refused> {
        match &self.hull.gauss_map {
            Some(map) if !arms::on("convexBruteForce") => hill_climbing(&self.hull.vertices, map, dir),
            _ => Ok(brute_force(&self.hull.vertices, dir)),
        }
    }

    fn support_checked(&self, dir: V3) -> Result<(V3, i32), Refused> {
        let index = self.support_vertex_index(Self::to_vertex_space(dir))?;
        Ok((self.vertex(index), index as i32))
    }
}

/// The first vertex of the greatest projection `(d.x*v.x + d.y*v.y) +
/// d.z*v.z`; a hull of fewer than two vertices answers vertex zero.
fn brute_force(verts: &[V3], dir: V3) -> usize {
    if verts.len() < 2 {
        return 0;
    }
    let dot = |v: V3| a::add(a::add(a::mul(dir[0], v[0]), a::mul(dir[1], v[1])), a::mul(dir[2], v[2]));
    let mut max = dot(verts[0]);
    let mut index = 0;
    for (i, &v) in verts.iter().enumerate().skip(1) {
        let d = dot(v);
        if d > max {
            max = d;
            index = i;
        }
    }
    index
}

/// `ComputeCubemapNearestOffset`: the face from the largest absolute bits
/// (y only when strictly above both, then z when strictly above x), the two
/// other lanes divided by the largest one's magnitude, moved to
/// `[0, subdiv - 1]`, rounded to nearest by truncating after adding a half.
fn cubemap_nearest_offset(dir: V3, subdiv: u16) -> u32 {
    let abs = dir.map(|x| x.to_bits() & 0x7fff_ffff);
    let (i1, i2, i3) = if abs[1] > abs[0] && abs[1] > abs[2] {
        (1, 2, 0)
    } else if abs[2] > abs[0] {
        (2, 0, 1)
    } else {
        (0, 1, 2)
    };
    let coeff = 1.0f32 / a::abs(dir[i1]);
    let u = a::mul(dir[i2], coeff);
    let v = a::mul(dir[i3], coeff);
    let remap = a::mul((u32::from(subdiv).wrapping_sub(1)) as f32, 0.5);
    let u = a::add(a::mul(remap, a::add(u, 1.0)), 0.5);
    let v = a::add(a::mul(remap, a::add(v, 1.0)), 0.5);
    let face = ((i1 as u32) << 1) | (dir[i1].to_bits() >> 31);
    let s = u32::from(subdiv);
    face.wrapping_mul(s).wrapping_add(truncate_u32(u)).wrapping_mul(s).wrapping_add(truncate_u32(v))
}

/// `FCVTZU`: toward zero, saturating, NaN to zero.
fn truncate_u32(x: f32) -> u32 {
    if x.is_nan() {
        0
    } else {
        x as u32
    }
}

/// `ConvexHullV::hillClimbing`.
fn hill_climbing(verts: &[V3], map: &GaussMap, dir: V3) -> Result<usize, Refused> {
    let dot = |v: V3| a::add(a::add(a::mul(dir[0], v[0]), a::mul(dir[1], v[1])), a::add(a::mul(dir[2], v[2]), 0.0));
    let offset = cubemap_nearest_offset(dir, map.subdiv) as usize;
    let &start = map.samples.get(offset).ok_or(Refused("a gauss map sample outside the cooked samples"))?;
    let mut index = usize::from(start);
    let vertex = |i: usize| verts.get(i).copied().ok_or(Refused("a gauss map vertex outside the hull"));
    let mut max = dot(vertex(index)?);
    let mut visited = [0u32; 8];
    loop {
        let initial = index;
        let &(count, first) = map.valencies.get(initial).ok_or(Refused("a hull vertex without a valency"))?;
        for k in 0..usize::from(count) {
            let &n = map.adjacent.get(usize::from(first) + k).ok_or(Refused("a valency past the adjacency list"))?;
            let d = dot(vertex(usize::from(n))?);
            if d > max {
                let (word, bit) = (usize::from(n) >> 5, 1u32 << (n & 31));
                if visited[word] & bit == 0 {
                    visited[word] |= bit;
                    max = d;
                    index = usize::from(n);
                }
            }
        }
        if index == initial {
            return Ok(index);
        }
    }
}

/// The support mapping cannot refuse inside the GJK; a hull whose gauss
/// map is inconsistent is refused before the sweep (see `validate`).
impl SweptShape for HullShape<'_> {
    fn support(&self, dir: V3) -> V3 {
        self.support_index(dir).0
    }
    fn support_index(&self, dir: V3) -> (V3, i32) {
        self.support_checked(dir).expect("hull validated before the sweep")
    }
    fn support_point(&self, index: i32) -> V3 {
        self.vertex(index as usize)
    }
    fn margin(&self) -> f32 {
        self.margin
    }
    fn min_margin(&self) -> f32 {
        self.min_margin
    }
    fn sweep_margin(&self) -> f32 {
        self.sweep_margin
    }
}

/// Refuses a hull the support mapping could index outside of: no vertex,
/// more than a hull holds, a non-finite value, or a gauss map whose
/// samples, valencies or adjacency name a vertex or entry that is not
/// there.
fn validate(hull: &HullSupport) -> Result<(), Refused> {
    let n = hull.vertices.len();
    if n == 0 || n > MAX_HULL_VERTICES {
        return Err(Refused("a convex hull with no vertex or more than 255"));
    }
    if !hull.vertices.iter().all(|&v| finite3(v)) || !finite3(hull.internal_extents) {
        return Err(Refused("a non-finite convex hull"));
    }
    if let Some(map) = &hull.gauss_map {
        let samples = 6 * usize::from(map.subdiv) * usize::from(map.subdiv);
        if map.subdiv == 0 || map.samples.len() < samples || map.valencies.len() != n
            || map.samples[..samples].iter().any(|&s| usize::from(s) >= n)
            || map.valencies.iter().any(|&(c, o)| usize::from(c) + usize::from(o) > map.adjacent.len())
            || map.adjacent.iter().any(|&v| usize::from(v) >= n)
        {
            return Err(Refused("a convex hull whose gauss map names entries it does not hold"));
        }
    }
    Ok(())
}

/// The engine's `PxGeometryQuery::sweep` of a sphere of `radius` at
/// `center` along the unit or zero `direction` for `distance` against the
/// cooked convex mesh `hull` at `pose` with the identity mesh scale, the hit
/// flags normal and MTD and zero inflation. `face_index` is the value the
/// caller initialised the hit's face index to; `trace` receives the native
/// calls the sweep makes.
pub fn sweep_sphere_convex(hull: &HullSupport, pose: &Pose, center: [f32; 3], radius: f32, direction: [f32; 3],
    distance: f32, face_index: u32, mut trace: Option<&mut BoxSweepTrace>) -> Result<Option<MeshSweepHit>, Refused> {
    if !finite3(center) || !finite3(direction) || !distance.is_finite() || !radius.is_finite() {
        return Err(Refused("a non-finite sweep"));
    }
    if !(distance >= 0.0) || !(radius > 0.0) {
        return Err(Refused("a negative sweep distance or a non-positive radius"));
    }
    validate(hull)?;
    let cap_q = [0.0, 0.0, 0.0, 1.0];
    let (q, p) = (pose.rotation(), pose.translation());
    let inv = [a::neg(q[0]), a::neg(q[1]), a::neg(q[2]), q[3]];
    let rel_p = bs::quat_rotate(inv, sub(center, p));
    let rel_rot = bs::quat_columns(bs::quat_mul(inv, cap_q));
    let half_axis = bs::columns_mul(&rel_rot, scale(UNIT_X, 0.0));
    let cap = Capsule { p0: add(rel_p, half_axis), p1: sub(rel_p, half_axis), radius };
    let shape = HullShape::new(hull);
    let r = bs::quat_rotate_inv(q, neg(scale(direction, distance)));
    // The capsule's centre minus the hull's, which is the origin.
    let initial_dir = sub(rel_p, ZERO3);
    let inflation = a::add(radius, 0.0);
    let Some((toi, normal, closest_a)) = bs::raycast_penetration(&cap, &shape, initial_dir, r, inflation, &mut trace)?
    else {
        return Ok(None);
    };
    let world_a = bs::quat_transform(q, p, closest_a);
    let world_normal = if arms::on("convexNoNormalize") { bs::quat_rotate(q, normal) } else { bs::normalize(bs::quat_rotate(q, normal)) };
    let flags = FLAG_NORMAL | FLAG_POSITION;
    Ok(Some(if 0.0 >= toi {
        MeshSweepHit { flags, face_index: OVERLAP_FACE, distance: toi, normal: world_normal,
            position: Some(v3neg_scale_sub(world_normal, toi, world_a)) }
    } else {
        let len = a::mul(distance, toi);
        MeshSweepHit { flags, face_index, distance: len, normal: world_normal,
            position: Some(v3scale_add(direction, len, world_a)) }
    }))
}

/// The engine's world bounds of a convex MeshCollider's shape at inflation
/// one, min then max. The collider's geometry asks for tight bounds: every
/// hull vertex through the pose's rotation columns (`(c0*x + c1*y) +
/// c2*z`), the running lane minimum and maximum, the zero contact offset
/// subtracted from the minimum and added to the maximum, the translation
/// added, then the inflation step recomputes both from the centre and the
/// half extent. The mesh scale is the identity (the colliders' nodes carry
/// unit scales).
pub fn convex_world_bounds(hull: &HullSupport, pose: &Pose) -> Result<[f32; 6], Refused> {
    validate(hull)?;
    let [c0, c1, c2] = Matrix34::from_pose(pose.rotation(), pose.translation()).columns;
    let rotate = |v: V3| add(add(scale(c0, v[0]), scale(c1, v[1])), scale(c2, v[2]));
    let mut min = rotate(hull.vertices[0]);
    let mut max = min;
    for &v in &hull.vertices[1..] {
        let p = rotate(v);
        min = std::array::from_fn(|k| a::min(min[k], p[k]));
        max = std::array::from_fn(|k| a::max(max[k], p[k]));
    }
    let t = pose.translation();
    let max = add(add(max, ZERO3), t);
    let min = add(sub(min, ZERO3), t);
    let center = scale(add(max, min), 0.5);
    let half = if arms::on("convexBoundsNoRecentre") { None } else { Some(scale(sub(max, min), a::mul(0.5, 1.0))) };
    Ok(match half {
        Some(e) => {
            let (lo, hi) = (sub(center, e), add(center, e));
            [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
        }
        None => [min[0], min[1], min[2], max[0], max[1], max[2]],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};
    use std::collections::BTreeMap;

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn word(v: &Value) -> u32 {
        let x = v.as_f64().expect("row word");
        assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
        x as u32
    }
    fn words(v: &Value) -> Vec<u32> {
        v.as_array().expect("row array").iter().map(word).collect()
    }
    fn v3(w: &[u32]) -> V3 {
        [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])]
    }
    fn bits(v: V3) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// A hull of the rows: its vertex bits, internal extents and gauss map
    /// as the engine cooked them.
    fn hull_of(h: &Value) -> HullSupport {
        let vb = words(field(h, "vertexBits"));
        let vertices = vb.chunks(3).map(v3).collect();
        let internal_extents = v3(&words(field(h, "internalExtentsBits")));
        let gauss_map = match h.get("gaussMap") {
            Some(g) if !matches!(g, Value::Null) => {
                let valency = words(field(g, "valencies"));
                Some(GaussMap {
                    subdiv: word(field(g, "subdiv")) as u16,
                    samples: words(field(g, "samples")).into_iter().map(|x| x as u8).collect(),
                    valencies: valency.chunks(2).map(|c| (c[0] as u16, c[1] as u16)).collect(),
                    adjacent: words(field(g, "adjacent")).into_iter().map(|x| x as u8).collect(),
                })
            }
            _ => None,
        };
        HullSupport { vertices, internal_extents, gauss_map }
    }

    const ARMS: [&str; 6] = ["convexBruteForce", "convexNoNormalize", "boxRecipThree", "boxCapsuleSweepMargin",
        "boxPenetrationCoreShape", "boxSkipEpa"];

    fn row_mismatch(hulls: &[HullSupport], row: &Value) -> (Vec<&'static str>, bool, String) {
        let hull = &hulls[word(field(row, "hull")) as usize];
        let p = words(field(row, "poseBits"));
        let pose = Pose::rotated([p[0], p[1], p[2], p[3]].map(f32::from_bits), v3(&p[4..])).expect("row pose");
        let mut trace = BoxSweepTrace::default();
        let got = sweep_sphere_convex(hull, &pose, v3(&words(field(row, "originBits"))),
            f32::from_bits(word(field(row, "radiusBits"))), v3(&words(field(row, "dirBits"))),
            f32::from_bits(word(field(row, "distanceBits"))), u32::MAX, Some(&mut trace));
        let calls: Vec<String> = field(row, "calls").as_array().expect("calls").iter()
            .map(|c| c.as_str().expect("call").to_owned()).collect();
        let order = trace.calls.iter().map(|c| c.to_string()).collect::<Vec<_>>() != calls;
        let mut bad = Vec::new();
        let native_hit = field(row, "hit").as_bool().expect("hit");
        match (&got, native_hit) {
            (Err(_), _) => bad.push("refused"),
            (Ok(None), false) => {}
            (Ok(Some(hit)), true) => {
                if hit.flags != word(field(row, "flags")) {
                    bad.push("flags");
                }
                if hit.face_index != word(field(row, "faceIndex")) {
                    bad.push("face");
                }
                if hit.distance.to_bits() != word(field(row, "outDistanceBits")) {
                    bad.push("distance");
                }
                if bits(hit.normal) != words(field(row, "normalBits")) {
                    bad.push("normal");
                }
                if hit.position.map(bits) != Some(words(field(row, "positionBits"))) {
                    bad.push("position");
                }
            }
            _ => bad.push("hit"),
        }
        let port = format!("port calls {:?} {:?}", trace.calls, got.map(|h| h.map(|h| (h.distance, h.normal, h.position))));
        (bad, order, port)
    }

    /// The engine's sphere-versus-convex sweeps over natively cooked hulls
    /// (fixture collider hulls with and without a gauss map, and synthetic
    /// ones), hit flags normal and MTD, zero inflation, the identity mesh
    /// scale: toward, away, grazing, overlapping, touching, distance-zero and
    /// signed-zero-direction sweeps at identity, quarter-turn and random
    /// poses. The port must equal every returned field and the native call
    /// order on every row; each mutant must differ on some row.
    #[test]
    #[ignore = "needs MOLY_CONVEX_ROWS"]
    fn convex_rows_match_native_bits() {
        let path = std::env::var("MOLY_CONVEX_ROWS").expect("MOLY_CONVEX_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        assert_eq!(word(field(&doc, "hitFlags")), 0x202, "the rows' hit flags are the product's");
        assert_eq!(word(field(&doc, "inflationBits")), 0, "the rows sweep with zero inflation");
        let hulls: Vec<HullSupport> = field(&doc, "hulls").as_array().expect("hulls").iter().map(hull_of).collect();
        let rows = field(&doc, "rows").as_array().expect("rows");
        let pass = || {
            let mut reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
            let (mut outputs, mut orders) = (0, 0);
            let mut failures = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                let (bad, order, port) = row_mismatch(&hulls, row);
                outputs += usize::from(!bad.is_empty());
                orders += usize::from(order);
                for r in &bad {
                    *reasons.entry(r).or_default() += 1;
                }
                if !bad.is_empty() || order {
                    failures.push(format!("row {i} {:?}: {bad:?} order {order}; {port}", field(row, "class").as_str()));
                }
            }
            (outputs, orders, reasons, failures)
        };
        arms::set(None);
        let (outputs, orders, reasons, failures) = pass();
        let count = |f: &dyn Fn(&Value) -> bool| rows.iter().filter(|r| f(r)).count();
        let hits = count(&|r| field(r, "hit").as_bool() == Some(true));
        let negative = count(&|r| field(r, "hit").as_bool() == Some(true)
            && f32::from_bits(word(field(r, "outDistanceBits"))) < 0.0);
        let with = |name: &str| rows.iter().filter(|r| field(r, "calls").as_array().expect("calls").iter()
            .any(|c| c.as_str() == Some(name))).count();
        let mapped = count(&|r| hulls[word(field(r, "hull")) as usize].gauss_map.is_some());
        let mut red = BTreeMap::new();
        for arm in ARMS {
            arms::set(Some(arm));
            let (o, c, _, _) = pass();
            red.insert(arm, (o, c));
        }
        arms::set(None);
        println!("convex replay: {} hulls ({} with a gauss map), {} rows ({mapped} on gauss-map hulls, {hits} hits, \
            {negative} negative distances, penetration {}, epa {}), rows differing on returned fields {outputs}, on the \
            call order {orders}, reasons {reasons:?}; arms red (fields, order) {red:?}", hulls.len(),
            hulls.iter().filter(|h| h.gauss_map.is_some()).count(), rows.len(), with("penetration"), with("epa"));
        for failure in failures.iter().take(20) {
            println!("  {failure}");
        }
        assert!(failures.is_empty(), "{} convex rows differ from native", failures.len());
        assert!(hits > 0 && negative > 0 && with("penetration") > 0 && with("epa") > 0 && mapped > 0);
        for arm in ARMS {
            let (o, c) = red[arm];
            assert!(o + c > 0, "arm {arm} stays green");
        }
    }

    /// The engine's world bounds (`PxGeometryQuery::getWorldBounds`,
    /// inflation one) of the rows' convex geometries with the tight-bounds
    /// flag the MeshCollider sets, at every row pose: the port must equal
    /// every bit, and the mutant must differ on some row.
    #[test]
    #[ignore = "needs MOLY_CONVEX_ROWS"]
    fn convex_bounds_match_native_bits() {
        let path = std::env::var("MOLY_CONVEX_ROWS").expect("MOLY_CONVEX_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let hulls: Vec<HullSupport> = field(&doc, "hulls").as_array().expect("hulls").iter().map(hull_of).collect();
        let rows = field(&doc, "bounds").as_array().expect("bounds rows");
        let pass = || rows.iter().filter(|row| {
            let p = words(field(row, "poseBits"));
            let pose = Pose::rotated([p[0], p[1], p[2], p[3]].map(f32::from_bits), v3(&p[4..])).expect("row pose");
            let got = convex_world_bounds(&hulls[word(field(row, "hull")) as usize], &pose).expect("bounds");
            got.iter().map(|x| x.to_bits()).collect::<Vec<_>>() != words(field(row, "boundsBits"))
        }).count();
        arms::set(None);
        let bad = pass();
        arms::set(Some("convexBoundsNoRecentre"));
        let red = pass();
        arms::set(None);
        println!("convex bounds replay: {} rows, mismatched {bad}; arm convexBoundsNoRecentre red on {red}", rows.len());
        assert_eq!(bad, 0, "convex bounds rows differ from native");
        assert!(!rows.is_empty() && red > 0);
    }
}
