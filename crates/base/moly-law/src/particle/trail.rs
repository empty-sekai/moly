//! Per-particle trail recording of the TrailModule in its per-particle mode:
//! a clock the module keeps, and per particle an ordered run of recorded
//! points with a path length. The module's own update does three things per
//! call: advance the clock by the call's dt once, before the particle loop
//! (an empty range still advances it); per particle, age the tail against the
//! trail lifetime; then record the particle's position when it passes the
//! ratio, age and minimum-distance gates.
//!
//! The engine calls this update twice per slice: once per new four-lane group
//! of a birth with dt 0 (recording the birth point) and once after the
//! simulation over the whole pool with the slice dt. Those call sites belong
//! to the caller.
//!
//! Storage: the engine keeps each particle's points in a ring whose capacity
//! is shared by all particles; it starts at four and doubles when a ring is
//! full, copying every live point in order to the front of the new storage.
//! Nothing downstream reads a slot index, only the order, so an ordered queue
//! without a capacity stores the same trail. The capacity is a power of two
//! throughout, so a ring emptied before a reallocation restarts at its oldest
//! slot there as well.
use std::collections::VecDeque;

use super::armf as a;
use super::random::ParticleRandom;
use super::schema::{TrailMode, TrailParams};
use super::MinMaxCurve;

const RATIO_SALT: u32 = 0x8abf_f360;
const LIFETIME_SALT: u32 = 0x34bb_ab1b;
const HUNDRED: f32 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrailLifetime {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
}

/// The recording half of an authored TrailModule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailRecording {
    pub ratio: f32,
    pub lifetime: TrailLifetime,
    pub min_vertex_distance: f32,
    pub world_space: bool,
    pub size_affects_lifetime: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Ribbon trails share points across particles; not transcribed.
    Ribbon,
    /// Curve-mode lifetime or width reads the animation curve evaluator.
    CurveMode(&'static str),
    /// The engine clamps these at load (ratio and lifetime to [0, 1], minimum
    /// vertex distance to at least 0); only values inside the clamped domain
    /// are taken, so the clamp itself is never assumed.
    OutsideLoadClamp(&'static str),
    /// Lighting data selects another vertex layout.
    LightingData,
    /// The dispatch evaluates the minimum gradient of this pairing with a
    /// perceptual kernel that calls the device libm.
    PerceptualGradient,
    /// The view matrix is too close to singular: the engine takes its general
    /// inverse, which is not transcribed.
    SingularView,
    /// Linear colour space converts every channel with the device libm.
    LinearColour,
    /// The width scale is a cube root of the emitter scale product through
    /// the device libm; only a product of magnitude one is exact.
    EmitterScale,
    /// Size affecting width with 3D sizes on a mesh renderer uses the device libm.
    MeshSizeWidth,
}

fn unit_interval(v: f32) -> bool {
    (0.0..=1.0).contains(&v)
}

impl TrailRecording {
    pub fn from_params(params: &TrailParams) -> Result<Self, Refused> {
        if params.mode != TrailMode::PerParticle {
            return Err(Refused::Ribbon);
        }
        let lifetime = match params.lifetime {
            MinMaxCurve::Constant(v) if unit_interval(v) => TrailLifetime::Constant(v),
            MinMaxCurve::TwoConstants { min, max } if unit_interval(min) && unit_interval(max) => {
                TrailLifetime::TwoConstants { min, max }
            }
            MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. } => {
                return Err(Refused::OutsideLoadClamp("lifetime"))
            }
            _ => return Err(Refused::CurveMode("lifetime")),
        };
        if !unit_interval(params.ratio) {
            return Err(Refused::OutsideLoadClamp("ratio"));
        }
        if !(params.min_vertex_distance.is_finite() && params.min_vertex_distance >= 0.0) {
            return Err(Refused::OutsideLoadClamp("minVertexDistance"));
        }
        Ok(Self {
            ratio: params.ratio,
            lifetime,
            min_vertex_distance: params.min_vertex_distance,
            world_space: params.world_space,
            size_affects_lifetime: params.size_affects_lifetime,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailPoint {
    pub position: [f32; 3],
    /// The module clock narrowed to f32 when the point was recorded.
    pub time: f32,
}

/// One particle's recorded points, oldest first, and the path length summed
/// over every added segment. The length is not reduced when the tail ages;
/// only the last point expiring resets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrailRing {
    points: VecDeque<TrailPoint>,
    length: f32,
}

impl TrailRing {
    /// The newborn reset InitialModule applies to every lane of a new group.
    pub fn reset(&mut self) {
        self.points.clear();
        self.length = 0.0;
    }
    pub fn len(&self) -> usize {
        self.points.len()
    }
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
    pub fn length(&self) -> f32 {
        self.length
    }
    /// Points oldest first.
    pub fn points(&self) -> impl ExactSizeIterator<Item = &TrailPoint> + DoubleEndedIterator + '_ {
        self.points.iter()
    }
    /// A ring holding these points (oldest first) and this path length.
    pub fn from_points(points: impl IntoIterator<Item = TrailPoint>, length: f32) -> Self {
        Self { points: points.into_iter().collect(), length }
    }
}

/// The module clock: binary64 seconds, advanced by each call's f32 dt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrailClock {
    pub time: f64,
}

impl TrailClock {
    pub fn advance(&mut self, dt: f32) {
        self.time += dt as f64;
    }
}

/// The particle's size triple the size-pair flag selects, and whether sizes
/// are three-dimensional. Read only when size affects the trail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailSize {
    pub components: [f32; 3],
    pub size3d: bool,
}

/// What one call reads of one particle.
#[derive(Clone, Copy, Debug)]
pub struct TrailParticle {
    pub seed: u32,
    pub age_percent: f32,
    pub inverse_lifetime: f32,
    pub position: [f32; 3],
    pub size: TrailSize,
}

/// The trail lifetime in seconds: the authored lifetime is a fraction of the
/// particle's start lifetime, so it is divided by the inverse lifetime.
/// With size affecting the lifetime it is first multiplied by the X size, or
/// by the largest of the three sizes (unordered comparisons keep the earlier
/// operand) when sizes are three-dimensional.
pub fn trail_lifetime(law: &TrailRecording, seed: u32, inverse_lifetime: f32, size: TrailSize) -> f32 {
    let value = match law.lifetime {
        TrailLifetime::Constant(v) => v,
        TrailLifetime::TwoConstants { min, max } => {
            let r = ParticleRandom::sample(seed, LIFETIME_SALT);
            a::add(min, a::mul(r, a::sub(max, min)))
        }
    };
    let value = if law.size_affects_lifetime {
        let [x, y, z] = size.components;
        let factor = if size.size3d {
            let m = if y < z { z } else { y };
            if x < m { m } else { x }
        } else {
            x
        };
        a::mul(value, factor)
    } else {
        value
    };
    a::div(value, inverse_lifetime)
}

/// The owner transform applied to a recorded point when the trail is in world
/// space and the simulation is not: (c0*x + c1*y) + (c3 + c2*z) per row.
fn to_world(matrix: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| {
        a::add(
            a::add(a::mul(matrix[k], p[0]), a::mul(matrix[4 + k], p[1])),
            a::add(matrix[12 + k], a::mul(matrix[8 + k], p[2])),
        )
    })
}

/// One particle of one call, after the caller advanced the clock. `owner` is
/// the matrix a world-space trail of a non-World simulation transforms with;
/// `None` records the raw position. Returns the trail lifetime when the ring
/// was not empty (the engine evaluates it only then).
pub fn record(
    law: &TrailRecording,
    ring: &mut TrailRing,
    clock: TrailClock,
    particle: &TrailParticle,
    owner: Option<&[f32; 16]>,
) -> Option<f32> {
    let time = clock.time;
    let mut lifetime = None;
    if !ring.points.is_empty() {
        let life = trail_lifetime(law, particle.seed, particle.inverse_lifetime, particle.size);
        lifetime = Some(life);
        let life = life as f64;
        while ring.points.len() >= 2 && time - ring.points[1].time as f64 > life {
            ring.points.pop_front();
        }
        if ring.points.len() == 1 && time - ring.points[0].time as f64 > life {
            ring.length = 0.0;
            ring.points.pop_front();
        }
    }
    if law.ratio == 0.0 {
        return lifetime;
    }
    let r = ParticleRandom::sample(particle.seed, RATIO_SALT);
    if law.ratio.is_nan() || r > law.ratio {
        return lifetime;
    }
    let age = particle.age_percent;
    if !age.is_nan() && !(age < HUNDRED) {
        return lifetime;
    }
    let p = match owner {
        Some(matrix) => to_world(matrix, particle.position),
        None => particle.position,
    };
    if let Some(front) = ring.points.back() {
        let d: [f32; 3] = std::array::from_fn(|k| a::sub(front.position[k], p[k]));
        let sq = d.map(|v| a::mul(v, v));
        let dist2 = a::add(a::add(sq[0], sq[1]), a::add(sq[2], 0.0));
        let mvd2 = a::mul(law.min_vertex_distance, law.min_vertex_distance);
        if !(dist2 > mvd2) {
            return lifetime;
        }
        ring.length = a::add(ring.length, a::sqrt(dist2));
    }
    ring.points.push_back(TrailPoint { position: p, time: a::narrow(time) });
    lifetime
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::particle::json::{self, Value};

    fn num(v: &Value) -> f64 {
        v.as_f64().expect("number")
    }
    fn bits(v: &Value) -> f32 {
        f32::from_bits(num(v) as u32)
    }
    fn at<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("missing {key}"))
    }
    fn arr(v: &Value) -> &[Value] {
        v.as_array().expect("array")
    }
    /// A 64-bit word written as a hex string (a binary64 JSON number cannot hold it).
    pub(crate) fn hex64(v: &Value) -> u64 {
        let s = v.as_str().expect("hex string");
        u64::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex word")
    }

    /// Native recording rows: TrailModule::Update with CalculateLifetime and
    /// the ring reallocation, executed in the engine library, over the five
    /// exported recording blocks on birth-and-slice schedules and random
    /// cases (partial ranges, dt 0, newborn resets, size-affected lifetimes,
    /// world-space transforms, ages across 100 and NaN, capacities 1, 2 and 4
    /// growing by reallocation). Compares every call: clock bits, each ring's
    /// point count, path length, every point's four words, and every trail
    /// lifetime the call evaluated. Slot indices are storage layout and not
    /// compared.
    #[test]
    #[ignore = "needs MOLY_TRAIL_RECORD_ROWS"]
    fn native_recording_rows_match_bits() {
        let path = std::env::var("MOLY_TRAIL_RECORD_ROWS").expect("MOLY_TRAIL_RECORD_ROWS");
        let doc = json::parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let rows = arr(at(&doc, "rows"));
        let (mut calls, mut cases, mut points, mut lifetimes, mut grown) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut arms = [0usize; 3];
        for row in rows {
            let case = at(row, "case");
            let id = at(case, "id").as_str().unwrap().to_owned();
            let t = at(case, "trail");
            let lt = at(t, "lifetime");
            let lifetime = match at(lt, "mode").as_str().unwrap() {
                "constant" => TrailLifetime::Constant(bits(at(lt, "value"))),
                _ => TrailLifetime::TwoConstants { min: bits(at(lt, "min")), max: bits(at(lt, "max")) },
            };
            let law = TrailRecording {
                ratio: bits(at(t, "ratio")),
                lifetime,
                min_vertex_distance: bits(at(t, "minVertexDistance")),
                world_space: at(t, "worldSpace").as_bool().unwrap(),
                size_affects_lifetime: at(t, "sizeAffectsLifetime").as_bool().unwrap(),
            };
            let system = at(case, "system");
            let simulation_world = num(at(system, "simulationSpace")) == 1.0;
            let matrix: [f32; 16] = std::array::from_fn(|i| bits(&arr(at(system, "matrix"))[i]));
            let parts = at(case, "particles");
            let n = num(at(case, "n")) as usize;
            let pair_b = num(at(parts, "sizePair")) != 0.0;
            let size3d = at(parts, "size3D").as_bool().unwrap();
            let seeds: Vec<u32> = arr(at(parts, "seeds")).iter().map(|v| num(v) as u32).collect();
            let inv: Vec<f32> = arr(at(parts, "inv")).iter().map(bits).collect();
            let sizes: Vec<[f32; 3]> = arr(at(parts, if pair_b { "sizeB" } else { "sizeA" })).iter()
                .map(|s| std::array::from_fn(|k| bits(&arr(s)[k]))).collect();
            let owner = (law.world_space && !simulation_world).then_some(&matrix);
            let mut rings = vec![TrailRing::default(); n];
            // The same rows through three single-rule changes, each of which must disagree somewhere:
            // the authored lifetime taken as seconds, the clock advanced after the particle loop,
            // and the clock advanced once per particle.
            let mut arm_rings = [vec![TrailRing::default(); n], vec![TrailRing::default(); n], vec![TrailRing::default(); n]];
            let mut arm_clock = [TrailClock::default(); 3];
            let mut clock = TrailClock::default();
            let natives = arr(at(row, "native"));
            let mut arm_differs = [false; 3];
            for (k, call) in arr(at(case, "calls")).iter().enumerate() {
                let native = &natives[k];
                for i in arr(at(call, "resetRings")) {
                    rings[num(i) as usize].reset();
                    for arm in arm_rings.iter_mut() { arm[num(i) as usize].reset(); }
                }
                let dt = bits(at(call, "dt"));
                clock.advance(dt);
                arm_clock[0].advance(dt);
                let before = arm_clock[1];
                arm_clock[1].advance(dt);
                let from = num(at(call, "from")) as usize;
                let to = num(at(call, "to")) as usize;
                let positions = arr(at(call, "positions"));
                let ages = arr(at(call, "ages"));
                let mut evaluated = Vec::new();
                for i in from..to {
                    let particle = TrailParticle {
                        seed: seeds[i],
                        age_percent: bits(&ages[i]),
                        inverse_lifetime: inv[i],
                        position: std::array::from_fn(|c| bits(&arr(&positions[i])[c])),
                        size: TrailSize { components: sizes[i], size3d },
                    };
                    if let Some(life) = record(&law, &mut rings[i], clock, &particle, owner) {
                        evaluated.push((i, life.to_bits()));
                    }
                    let seconds = TrailParticle { inverse_lifetime: 1.0, ..particle };
                    record(&law, &mut arm_rings[0][i], arm_clock[0], &seconds, owner);
                    record(&law, &mut arm_rings[1][i], before, &particle, owner);
                    arm_clock[2].advance(dt);
                    record(&law, &mut arm_rings[2][i], arm_clock[2], &particle, owner);
                }
                if from >= to { arm_clock[2].advance(dt); }
                assert_eq!(clock.time.to_bits(), hex64(at(native, "timeBits")), "{id} call {k} clock");
                let native_lives: Vec<(usize, u32)> = arr(at(native, "lifetimeBits")).iter()
                    .map(|p| (num(&arr(p)[0]) as usize, num(&arr(p)[1]) as u32)).collect();
                assert_eq!(evaluated, native_lives, "{id} call {k} trail lifetimes");
                lifetimes += evaluated.len();
                grown += arr(at(native, "reallocate")).len();
                for (i, native_ring) in arr(at(native, "rings")).iter().enumerate() {
                    let ring = &rings[i];
                    assert_eq!(ring.len(), num(at(native_ring, "count")) as usize, "{id} call {k} ring {i} count");
                    assert_eq!(ring.length().to_bits(), num(at(native_ring, "lenBits")) as u32, "{id} call {k} ring {i} length");
                    for (p, q) in ring.points().zip(arr(at(native_ring, "points"))) {
                        let q = arr(q);
                        let words = [p.position[0].to_bits(), p.position[1].to_bits(), p.position[2].to_bits(), p.time.to_bits()];
                        let expected: [u32; 4] = std::array::from_fn(|c| num(&q[c]) as u32);
                        assert_eq!(words, expected, "{id} call {k} ring {i} point");
                        points += 1;
                    }
                    for (arm, differs) in arm_rings.iter().zip(arm_differs.iter_mut()) {
                        let r = &arm[i];
                        let same = r.len() == ring.len() && r.length().to_bits() == ring.length().to_bits()
                            && r.points().zip(ring.points()).all(|(x, y)| x == y);
                        *differs |= !same;
                    }
                }
                calls += 1;
            }
            for (count, differs) in arms.iter_mut().zip(arm_differs) { *count += differs as usize; }
            cases += 1;
        }
        println!("trail recording replay: {cases} cases, {calls} calls, {points} points, {lifetimes} lifetimes, \
            {grown} native reallocations; arms (cases differing) secondsLifetime {} clockAfterLoop {} \
            clockPerParticle {}", arms[0], arms[1], arms[2]);
        assert!(cases > 0 && grown > 0);
        assert!(arms.iter().all(|&n| n > 0), "every negative arm must disagree with native somewhere: {arms:?}");
    }
}
