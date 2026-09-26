//! The engine CollisionModule update for the Planes type: the plane cache
//! the module builds from its plane slots once per update, and the
//! per-particle plane test whose hits take the response, the random advance
//! and the collision events the World type shares (see
//! [`CollisionLaw::update_planes`](super::collision_query::CollisionLaw::update_planes)).
//!
//! The cache (`CollisionModule::Cache`, once per update, before the
//! slices). Each non-empty slot names a Transform by instance id; a slot
//! whose object is gone is skipped, so the kept planes stay packed in slot
//! order. A plane's normal is the up axis rotated by the Transform's world
//! rotation (the quaternion's matrix rows applied to the axis) and its
//! offset `-((n.x p.x + n.y p.y) + n.z p.z)` at the world position; both are
//! then normalized robustly, the offset scaled by the same factor. The
//! Transform is read wherever it lives: nothing on this path asks whether it
//! is in a scene, so a persistent asset Transform that was never
//! instantiated is read at its own stored pose (a root returns its stored
//! local position and rotation unchanged). A system simulated outside World
//! space then takes every kept plane into its simulation space through its
//! world-to-local matrix: the normal through the linear part, divided by its
//! length when that is above the epsilon (else +Z); the point `n * -d`
//! through the whole matrix; the new offset from those; and the robust
//! normalization again.
//!
//! The test (`PlaneCollision`). For each particle of `[from, to)` whose dt
//! lane (the particle index modulo four, not its place in the range) is not
//! below `1e-6`: the velocity is the persistent plus the animated one, times
//! the speed modifier when that flag is set; the radius is the size (with a
//! 3D size, the largest component by compare-and-select: X, then Y when X is
//! less, then Z when that is less) times half the radius scale; the
//! direction is the velocity over its length, or zero when the length is not
//! above the epsilon. The planes are tried in order. A plane hits when the
//! signed distance of the current position (`d + ((p.x n.x + p.y n.y) +
//! p.z n.z)`) is not above the radius and the direction is not within `1e-4`
//! of perpendicular to the normal; the hit point is the position moved along
//! the velocity by `-(distance - radius) / (v . n)`, and the particle takes no
//! further plane. The previous position is not read; the dt only gates. The
//! module's range words are not rewritten on this path.

use super::collision_query::{arms, CollisionParticles, HitRecord, ParticleFlags, Refused};
use crate::particle::armf as a;
use crate::particle::collision_response::QueryAffine;

/// The engine's float epsilon (the normalization and the direction floor).
const EPSILON: f32 = f32::from_bits(0x3727_c5ac);
/// A lane dt below this skips its particle.
const MIN_DT: f32 = f32::from_bits(0x3586_37bd);
/// A direction within this of perpendicular to a plane's normal misses it.
const PERPENDICULAR: f32 = f32::from_bits(0x38d1_b717);
/// The axis a plane Transform's rotation carries to the plane normal.
const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// One plane slot's Transform as the cache reads it: its world position and
/// rotation (x, y, z, w) in source axes, and its instance id (which only the
/// hit records carry).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneTransform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub id: i32,
}

/// One cached plane in the system's simulation space: the signed distance of
/// a point `p` is `offset + ((p.x n.x + p.y n.y) + p.z n.z)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachedPlane {
    pub normal: [f32; 3],
    pub offset: f32,
    pub id: i32,
}

impl CachedPlane {
    /// The engine's five words of the plane: normal, offset, id.
    pub fn words(&self) -> [u32; 5] {
        [self.normal[0].to_bits(), self.normal[1].to_bits(), self.normal[2].to_bits(), self.offset.to_bits(), self.id as u32]
    }
}

/// The sign of `sign` on the magnitude of `magnitude` (a bit-field insert).
fn signed(sign: f32, magnitude: f32) -> f32 {
    f32::from_bits((sign.to_bits() & 0x8000_0000) | (magnitude.to_bits() & 0x7fff_ffff))
}

/// `NormalizeRobust`: the unit vector and the scale factor. Every component
/// whose magnitude is at or below `epsilon` is zeroed; the largest remaining
/// magnitude (Y over X when strictly greater, Z over the winner when strictly
/// greater, X otherwise) divides the other two, the factor
/// `s = 1 / sqrt((a^2 + b^2) + 1)` scales them and gives the largest
/// component its sign, and the returned scale is `s` over that magnitude.
/// With every component zeroed the vector is +Y and the scale zero.
pub fn normalize_robust(v: [f32; 3], epsilon: f32) -> ([f32; 3], f32) {
    let keep = |x: f32| {
        let magnitude = if x < 0.0 { a::neg(x) } else { x };
        if magnitude <= epsilon { (0.0, 0.0) } else { (x, magnitude) }
    };
    let ((x, ax), (y, ay), (z, az)) = (keep(v[0]), keep(v[1]), keep(v[2]));
    let factor = |p: f32, q: f32| a::div(1.0, a::sqrt(a::add(a::add(a::mul(p, p), a::mul(q, q)), 1.0)));
    let (out, s, largest) = if ay > ax && !(az > ay) {
        let (p, q) = (a::div(x, ay), a::div(z, ay));
        let s = factor(p, q);
        ([a::mul(p, s), signed(y, s), a::mul(q, s)], s, ay)
    } else if ay > ax || az > ax {
        let (p, q) = (a::div(x, az), a::div(y, az));
        let s = factor(p, q);
        ([a::mul(p, s), a::mul(q, s), signed(z, s)], s, az)
    } else if !(ax <= 0.0) {
        let (p, q) = (a::div(y, ax), a::div(z, ax));
        let s = factor(p, q);
        ([signed(x, s), a::mul(p, s), a::mul(q, s)], s, ax)
    } else {
        ([0.0, 1.0, 0.0], 0.0, 1.0)
    };
    (out, a::div(s, largest))
}

/// `v` rotated by the quaternion `q` (x, y, z, w) through its matrix rows,
/// each row applied as the engine sums it.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (a::add(x, x), a::add(y, y), a::add(z, z));
    let (xx, yy, zz) = (a::mul(x, x2), a::mul(y, y2), a::mul(z, z2));
    let (xy, xz, yz) = (a::mul(x, y2), a::mul(x, z2), a::mul(y, z2));
    let (wx, wy, wz) = (a::mul(w, x2), a::mul(w, y2), a::mul(w, z2));
    let r00 = a::sub(1.0, a::add(yy, zz));
    let r11 = a::sub(1.0, a::add(xx, zz));
    let r22 = a::sub(1.0, a::add(xx, yy));
    [
        a::add(a::mul(v[2], a::add(xz, wy)), a::add(a::mul(v[1], a::sub(xy, wz)), a::mul(v[0], r00))),
        a::add(a::mul(v[2], a::sub(yz, wx)), a::add(a::mul(v[0], a::add(xy, wz)), a::mul(v[1], r11))),
        a::add(a::add(a::mul(v[0], a::sub(xz, wy)), a::mul(v[1], a::add(yz, wx))), a::mul(v[2], r22)),
    ]
}

/// `(a.x b.x + a.y b.y) + a.z b.z`.
fn dot(u: [f32; 3], v: [f32; 3]) -> f32 {
    a::add(a::add(a::mul(u[0], v[0]), a::mul(u[1], v[1])), a::mul(u[2], v[2]))
}

/// The robust normalization of a plane, the offset scaled by its factor.
fn normalized(normal: [f32; 3], offset: f32, id: i32) -> CachedPlane {
    let (normal, scale) = normalize_robust(normal, EPSILON);
    let offset = if arms::on("planesUnscaledOffset") { offset } else { a::mul(scale, offset) };
    CachedPlane { normal, offset, id }
}

/// `CollisionModule::Cache` for the Planes type: the planes of the slots in
/// order (a `None` slot is empty or names no live object), in world space,
/// then through `world_to_local` when the system is simulated outside World
/// space.
pub fn cache_planes(slots: &[Option<PlaneTransform>], world_to_local: Option<&QueryAffine>) -> Vec<CachedPlane> {
    let mut planes: Vec<CachedPlane> = slots.iter().flatten().map(|t| {
        let n = rotate(t.rotation, UP);
        let offset = a::neg(dot(n, t.position));
        normalized(n, offset, t.id)
    }).collect();
    let Some(m) = world_to_local.filter(|_| !arms::on("planesNoLocalCache")) else {
        return planes;
    };
    let l = m.linear;
    for plane in &mut planes {
        let n = plane.normal;
        let linear = |v: [f32; 3], r: usize| a::add(a::add(a::mul(v[0], l[r][0]), a::mul(v[1], l[r][1])), a::mul(v[2], l[r][2]));
        let mapped = [linear(n, 0), linear(n, 1), linear(n, 2)];
        let length = a::sqrt(a::add(a::add(a::mul(mapped[0], mapped[0]), a::mul(mapped[1], mapped[1])),
            a::mul(mapped[2], mapped[2])));
        let normal = if length > EPSILON { mapped.map(|c| a::div(c, length)) } else { [0.0, 0.0, 1.0] };
        let back = a::neg(plane.offset);
        let point = n.map(|c| a::mul(c, back));
        let point: [f32; 3] = std::array::from_fn(|r| a::add(m.translation[r], linear(point, r)));
        let offset = a::neg(dot(point, normal));
        *plane = normalized(normal, offset, plane.id);
    }
    planes
}

/// `PlaneCollision`: at most one hit per particle of `[from, to)`, in
/// particle order, in simulation space (start: the position; direction: the
/// velocity the test used; both ids the plane Transform's).
pub(super) fn plane_hits(planes: &[CachedPlane], particles: &dyn CollisionParticles, flags: ParticleFlags,
    radius_scale: f32, from: usize, to: usize, dt: [f32; 4]) -> Result<Vec<HitRecord>, Refused> {
    let half = if arms::on("planesRadiusFull") { radius_scale } else { a::mul(radius_scale, 0.5) };
    let mut hits = Vec::new();
    for index in from..to {
        let lane = if arms::on("planesLaneFromRange") { (index - from) & 3 } else { index & 3 };
        if dt[lane] < MIN_DT {
            continue;
        }
        let p = particles.lane(index).ok_or(Refused::MissingParticle)?;
        let mut v: [f32; 3] = std::array::from_fn(|k| a::add(p.velocity[k], p.animated[k]));
        if flags.speed_modifier {
            v = v.map(|c| a::mul(c, p.speed_modifier));
        }
        let size = if !flags.size_3d {
            p.size[0]
        } else if arms::on("planesFmaxSize") {
            a::max(a::max(p.size[0], p.size[1]), p.size[2])
        } else {
            let first = if p.size[0] < p.size[1] { p.size[1] } else { p.size[0] };
            if first < p.size[2] { p.size[2] } else { first }
        };
        if planes.is_empty() {
            continue;
        }
        let radius = a::mul(size, half);
        let speed = a::sqrt(a::add(a::add(a::mul(v[0], v[0]), a::mul(v[1], v[1])), a::mul(v[2], v[2])));
        let direction = if speed > EPSILON { v.map(|c| a::div(c, speed)) } else { [0.0; 3] };
        let mut found = None;
        for plane in planes {
            let n = plane.normal;
            let distance = a::add(plane.offset, dot(p.position, n));
            if distance > radius {
                continue;
            }
            if !arms::on("planesNoPerpendicularGate") && a::abs(dot(direction, n)) < PERPENDICULAR {
                continue;
            }
            let along = if arms::on("planesDirectionForTravel") { dot(direction, n) } else { dot(v, n) };
            let t = a::div(a::neg(a::sub(distance, radius)), along);
            let point = std::array::from_fn(|k| a::add(p.position[k], a::mul(v[k], t)));
            found = Some(HitRecord {
                index: index as u32,
                start: p.position,
                direction: v,
                normal: n,
                point,
                collider_id: plane.id,
                body_id: plane.id,
                order_free: false,
            });
            if !arms::on("planesLastPlane") {
                break;
            }
        }
        hits.extend(found);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::super::collision_query::{
        CollisionLaw, CollisionState, OwnerPair, ParticleLane, UpdateInput, UpdateOutcome,
    };
    use super::*;
    use crate::particle::collision_event::{CollisionEmitEdge, EdgeBurst};
    use crate::particle::collision_response::{CollisionRandom, CollisionResponse};
    use crate::particle::json::{parse, Value};
    use std::collections::BTreeMap;

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn items(v: &Value) -> &[Value] {
        v.as_array().expect("row array")
    }
    fn int(v: &Value) -> i64 {
        let x = v.as_f64().expect("row integer");
        assert!(x.fract() == 0.0 && x.abs() <= 9.0e15, "row integer {x}");
        x as i64
    }
    fn word(v: &Value) -> u32 {
        int(v) as u32
    }
    fn bits(v: &Value) -> f32 {
        f32::from_bits(word(v))
    }
    fn words(v: &Value) -> Vec<u32> {
        items(v).iter().map(word).collect()
    }
    fn flag(v: &Value, key: &str) -> bool {
        field(v, key).as_f64().expect("flag byte") != 0.0
    }
    fn arrays_of(v: &Value) -> BTreeMap<u32, Vec<u32>> {
        field(v, "arrays").as_object().expect("arrays").iter().map(|(k, v)| (k.parse().expect("offset"), words(v))).collect()
    }

    /// The native arrays, read as the module reads them.
    struct Arrays {
        words: BTreeMap<u32, Vec<u32>>,
        current_size: bool,
    }

    impl Arrays {
        fn f(&self, key: u32, index: usize) -> f32 {
            f32::from_bits(self.words[&key].get(index).copied().unwrap_or(0))
        }
        fn v3(&self, key: u32, index: usize) -> [f32; 3] {
            std::array::from_fn(|k| self.f(key + 32 * k as u32, index))
        }
    }

    impl CollisionParticles for Arrays {
        fn lane(&self, i: usize) -> Option<ParticleLane> {
            (i < self.words[&0].len()).then(|| ParticleLane {
                position: self.v3(0, i),
                velocity: self.v3(96, i),
                animated: self.v3(192, i),
                size: self.v3(if self.current_size { 768 } else { 672 }, i),
                speed_modifier: self.f(1216, i),
                age_percent: self.f(960, i),
                inverse_lifetime: self.f(992, i),
                seed: self.words[&896][i],
            })
        }
    }

    /// The plane slots of a row as the cache receives them.
    fn slots_of(input: &Value) -> Vec<Option<PlaneTransform>> {
        let planes = items(field(input, "planes"));
        items(field(input, "slots")).iter().map(|slot| {
            let id = word(slot);
            if id == 0 {
                return None;
            }
            let p = planes.iter().find(|p| word(field(p, "id")) == id).expect("slot names a plane");
            if field(p, "dead").as_bool() == Some(true) {
                return None;
            }
            let pos = words(field(p, "positionBits"));
            let rot = words(field(p, "rotationBits"));
            Some(PlaneTransform {
                position: std::array::from_fn(|k| f32::from_bits(pos[k])),
                rotation: std::array::from_fn(|k| f32::from_bits(rot[k])),
                id: id as i32,
            })
        }).collect()
    }

    fn columns(input: &Value, key: &str) -> QueryAffine {
        let w = words(field(input, key));
        QueryAffine::from_columns(&std::array::from_fn(|i| f32::from_bits(w[i])))
    }

    /// Where one row differs from the engine, empty when it matches.
    fn replay(row: &Value) -> (Vec<&'static str>, Option<UpdateOutcome>) {
        let mut bad = Vec::new();
        let input = field(row, "input");
        let (before, after, log) = (field(row, "before"), field(row, "after"), field(row, "log"));
        let module = field(input, "module");
        assert_eq!((int(field(module, "type")), int(field(module, "mode"))), (0, 0), "Planes type, 3D");
        let curve = |key: &str| bits(field(field(module, key), "scalarBits"));
        let Ok(response) = CollisionResponse::new(curve("bounce"), curve("dampen"), curve("lifetimeLoss"),
            bits(field(module, "minKillSpeedBits")), bits(field(module, "maxKillSpeedBits"))) else {
            return (vec!["refused response"], None);
        };
        let pf = field(input, "particleFlags");
        let flags = ParticleFlags { current_size: flag(pf, "2002"), size_3d: flag(pf, "2004"), speed_modifier: flag(pf, "2008") };
        let world = int(field(input, "simulationSpace")) == 1;
        let law = CollisionLaw::planes_from_words(response, bits(field(module, "radiusScaleBits")), world, flags);
        let owner = (!world).then(|| OwnerPair { local_to_world: columns(input, "ownerBits"), world_to_local: columns(input, "inverseBits") });
        let planes = cache_planes(&slots_of(input), owner.as_ref().map(|o| &o.world_to_local));
        let cached: Vec<Vec<u32>> = planes.iter().map(|p| p.words().to_vec()).collect();
        if cached != items(field(row, "cachedPlanes")).iter().map(words).collect::<Vec<_>>() {
            bad.push("cache");
        }
        let edges: Vec<CollisionEmitEdge> = items(field(input, "subEmitters")).iter().map(|se| {
            assert_eq!(int(field(se, "properties")), 0);
            let burst = items(field(se, "bursts")).first().map(|b| {
                assert_eq!(int(field(b, "countMode")), 0, "constant burst count");
                EdgeBurst::new(bits(field(b, "probabilityBits")), bits(field(b, "countMaxBits"))).expect("burst in range")
            });
            CollisionEmitEdge::new(bits(field(se, "probabilityBits")), burst).expect("edge in range")
        }).collect();
        let arrays = Arrays { words: arrays_of(input), current_size: flags.current_size };
        let mut state = CollisionState {
            range_words: [int(field(input, "state10")) as u64, int(field(input, "state18")) as u64],
            uses_events: flag(field(before, "flags"), "2013"),
            events: int(field(before, "collisionEvents")) as u64,
        };
        let mut random = CollisionRandom { words: std::array::from_fn(|i| words(field(input, "randWords"))[i]) };
        let dt = words(field(input, "dtBits"));
        let update = UpdateInput {
            from: int(field(input, "from")) as usize,
            to: int(field(input, "to")) as usize,
            dt: std::array::from_fn(|l| f32::from_bits(dt[l])),
            owner,
            edges: &edges,
            emission_word: word(field(input, "state1ec")),
            pending: bits(field(input, "state0Bits")),
        };
        let outcome = match law.update_planes(&mut state, Some(&mut random), &arrays, &update, &planes) {
            Ok(outcome) => outcome,
            Err(_) => return (vec!["refused"], None),
        };
        // The written arrays and every other array.
        let count = int(field(input, "count")) as usize;
        let mut written = arrays.words.clone();
        for w in &outcome.written {
            for k in 0..3 {
                written.get_mut(&(32 * k as u32)).unwrap()[w.index] = w.position[k].to_bits();
                written.get_mut(&(96 + 32 * k as u32)).unwrap()[w.index] = w.velocity[k].to_bits();
            }
            written.get_mut(&960).unwrap()[w.index] = w.age_percent.to_bits();
        }
        let native = arrays_of(after);
        for (key, values) in &native {
            let ours = &written[key][..count.min(written[key].len())];
            if ours != &values[..] {
                bad.push(if [0, 32, 64, 96, 128, 160, 960].contains(key) { "arrays" } else { "otherArrays" });
                break;
            }
        }
        if random.words.to_vec() != words(field(after, "randWords")) {
            bad.push("random");
        }
        if state.range_words != [int(field(after, "state10")) as u64, int(field(after, "state18")) as u64] {
            bad.push("rangeWords");
        }
        if state.uses_events != flag(field(after, "flags"), "2013") || state.events as i64 != int(field(after, "collisionEvents")) {
            bad.push("eventFlag");
        }
        let hits: Vec<Vec<u32>> = outcome.hits.iter().map(|h| {
            let mut w = vec![h.index];
            w.extend(h.start.iter().chain(&h.direction).chain(&h.normal).chain(&h.point).map(|v| v.to_bits()));
            w.extend([h.collider_id as u32, h.body_id as u32]);
            w
        }).collect();
        let native_hits: Vec<Vec<u32>> = log.get("performInput").map_or_else(Vec::new, |p| items(p).iter().map(words).collect());
        if hits != native_hits {
            bad.push("hits");
        }
        let records = items(field(log, "recordEmit"));
        if records.len() != outcome.emits.len() || records.iter().zip(&outcome.emits).any(|(r, e)| {
            int(field(r, "particle")) as usize != e.particle || int(field(r, "subIndex")) as usize != e.edge
                || int(field(r, "trigger")) != 1 || words(field(r, "stateWords")) != e.state_words.to_vec()
                || words(field(r, "sBits")) != e.times.map(f32::to_bits).to_vec()
                || r.get("burstCount").map(|b| int(b) as i32) != e.burst_count
        }) {
            bad.push("recordEmit");
        }
        let ours: Vec<(usize, Vec<u32>, Vec<u32>)> = outcome.emits.iter()
            .flat_map(|e| e.commands.iter().flatten().map(move |(command, state)| (e.edge, command, state)))
            .map(|(edge, command, state)| {
                let raw = command.to_bytes();
                let raw: Vec<u32> = (0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else {
                    u32::from_le_bytes(raw[4 * k..4 * k + 4].try_into().unwrap()) }).collect();
                (edge, state.to_vec(), raw)
            }).collect();
        let native_commands: Vec<(usize, Vec<u32>, Vec<u32>)> = items(field(log, "emits")).iter().map(|e| {
            let mut raw = words(field(e, "rawWords"));
            for k in [0, 1, 21] {
                raw[k] = 0;
            }
            (int(field(e, "childIndex")) as usize, words(field(e, "emissionStateWords")), raw)
        }).collect();
        if ours != native_commands {
            bad.push("commands");
        }
        (bad, Some(outcome))
    }

    const ARMS: [&str; 9] = ["planesFmaxSize", "planesUnscaledOffset", "planesNoLocalCache", "planesDirectionForTravel",
        "planesNoPerpendicularGate", "planesLaneFromRange", "planesLastPlane", "planesRadiusFull", "noRandom"];

    /// The native Planes-type update rows (the plane slots through the cache,
    /// then the module update over the cached planes): every row's cached
    /// planes, written arrays, untouched arrays, random words, range words,
    /// event flag, hits, event records and commands, bit for bit. Each named
    /// one-rule variant must differ from native on at least one row.
    #[test]
    #[ignore = "needs MOLY_PLANES_ROWS"]
    fn planes_rows_match_native_bits() {
        let path = std::env::var("MOLY_PLANES_ROWS").expect("MOLY_PLANES_ROWS");
        let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let rows = items(field(&doc, "rows"));
        assert!(!rows.is_empty());
        let mut failures = Vec::new();
        let mut red: BTreeMap<&str, usize> = BTreeMap::new();
        let (mut hits, mut written, mut emits, mut local, mut tilted, mut corpus, mut corpus_hits) = (0, 0, 0, 0, 0, 0, 0);
        for row in rows {
            arms::set(None);
            let (bad, outcome) = replay(row);
            if !bad.is_empty() {
                failures.push(format!("{} #{}: {bad:?}", field(row, "scenario").as_str().unwrap(), int(field(row, "repeat"))));
            }
            if let Some(outcome) = &outcome {
                hits += outcome.hits.len();
                written += outcome.written.iter().filter(|w| w.position.iter().any(|v| v.is_finite())).count();
                emits += outcome.emits.len();
            }
            let input = field(row, "input");
            local += usize::from(int(field(input, "simulationSpace")) != 1);
            tilted += items(field(row, "cachedPlanes")).iter()
                .filter(|p| words(p)[..3] != [0, 1.0f32.to_bits(), 0]).count();
            if field(row, "corpus").as_bool() == Some(true) {
                corpus += 1;
                corpus_hits += outcome.as_ref().map_or(0, |o| o.hits.len());
            }
            for arm in ARMS {
                arms::set(Some(arm));
                *red.entry(arm).or_default() += usize::from(!replay(row).0.is_empty());
            }
            arms::set(None);
        }
        println!("planes replay: {} rows ({local} local, {corpus} with the asset plane and the module words of the \
            fixture effect, {corpus_hits} hits there), {hits} hits, {written} written with finite positions, {emits} events, \
            {tilted} cached planes off +Y; rows differing {}; arms red {red:?}", rows.len(), failures.len());
        for failure in failures.iter().take(20) {
            println!("  {failure}");
        }
        assert!(failures.is_empty(), "{} rows differ from native", failures.len());
        assert!(hits > 0 && tilted > 0 && local > 0 && emits > 0 && corpus_hits > 0, "positive control");
        for arm in ARMS {
            assert!(red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
        }
    }

    /// Hand-checked values: the fixture's asset plane (rotation (0, -0, -0,
    /// 1) at (-0, 0, 0)) caches as +Y with offset -0; a zero vector
    /// normalizes to +Y with scale zero; a particle inside the radius and
    /// moving down hits at the plane moved by the radius.
    #[test]
    fn asset_plane_and_one_hit() {
        let slot = PlaneTransform { position: [-0.0, 0.0, 0.0], rotation: [0.0, -0.0, -0.0, 1.0], id: 7 };
        let planes = cache_planes(&[None, Some(slot)], None);
        assert_eq!(planes.len(), 1);
        assert_eq!(planes[0].words(), [0, 1.0f32.to_bits(), 0, 0x8000_0000, 7]);
        assert_eq!(normalize_robust([0.0, 1e-6, -1e-6], EPSILON), ([0.0, 1.0, 0.0], 0.0));
        struct One;
        impl CollisionParticles for One {
            fn lane(&self, _: usize) -> Option<ParticleLane> {
                Some(ParticleLane { position: [1.0, 0.25, 2.0], velocity: [0.0, -2.0, 0.0], animated: [0.0; 3],
                    size: [1.0; 3], speed_modifier: 1.0, age_percent: 10.0, inverse_lifetime: 1.0, seed: 3 })
            }
        }
        let flags = ParticleFlags { current_size: false, size_3d: false, speed_modifier: false };
        let hits = plane_hits(&planes, &One, flags, 1.0, 0, 1, [1.0 / 30.0; 4]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].point, [1.0, 0.5, 2.0]);
        assert_eq!(hits[0].normal, [0.0, 1.0, 0.0]);
    }
}
