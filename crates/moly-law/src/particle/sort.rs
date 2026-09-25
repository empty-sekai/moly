//! Per-particle renderer order, independent of simulation pool order.
//!
//! The renderer sorts before it writes geometry, in the simulation space: it
//! composes the view (world to camera) with the simulation owner (identity for
//! World simulation), inverts that 3x4 product approximately (its three first
//! columns normalized by one reciprocal square root estimate of their mean
//! squared length, then the cofactor inverse), and reads the camera position
//! (the inverse applied to minus the translation) and the camera's view-space
//! Z axis (the inverse's third column) back in the simulation space. Every
//! step below is the engine's single-precision operation in its order.
use super::armf as a;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParticleSort {
    #[default]
    None,
    Distance,
    OldestInFront,
    YoungestInFront,
    /// Planar distance along the camera axis (serialized value 4).
    Depth,
}

#[derive(Clone, Copy, Debug)]
pub struct SortParticle {
    /// Simulation-space position, in the source basis.
    pub position: [f32; 3],
    pub age_percent: f32,
    pub inverse_lifetime: f32,
}

/// The camera as the sort reads it, all in the source basis.
#[derive(Clone, Copy, Debug)]
pub struct SortCamera {
    /// World to camera (the camera's view matrix), column major.
    pub view: [f32; 16],
    /// Simulation to world (identity for World simulation), column major.
    pub owner: [f32; 16],
    /// The camera's near clip plane distance.
    pub near: f32,
    /// An orthographic camera sorts Distance by depth as well.
    pub orthographic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortRefused {
    /// The normalized view-owner product has |determinant| at or below 1e-6;
    /// the engine then takes its SVD inverse, which is not transcribed.
    SingularOwner,
}

const THIRD_APPROX: f32 = f32::from_bits(0x3eaa_aa9f);
const TINY_SCALE: f32 = f32::from_bits(0x0da2_4260);
const RSQ_BIAS: f32 = f32::from_bits(0x3f80_4020);
const DET_EPS: f32 = f32::from_bits(0x3586_37bd);
const DEPTH_CEILING: f32 = f32::from_bits(0xb586_37bd);

type V4 = [f32; 4];
fn vadd(x: V4, y: V4) -> V4 { std::array::from_fn(|i| a::add(x[i], y[i])) }
fn vmul(x: V4, y: V4) -> V4 { std::array::from_fn(|i| a::mul(x[i], y[i])) }
fn vscale(x: V4, s: f32) -> V4 { x.map(|v| a::mul(v, s)) }
fn col(m: &[f32; 16], i: usize) -> V4 { std::array::from_fn(|k| m[4 * i + k]) }
/// Horizontal sum of the three first lanes as the kernels pair them:
/// (x + y) + (z + 0).
fn sum3(v: V4) -> f32 { a::add(a::add(v[0], v[1]), a::add(v[2], 0.0)) }
fn cross(p: V4, q: V4) -> V4 {
    [
        a::sub(a::mul(p[1], q[2]), a::mul(q[1], p[2])),
        a::sub(a::mul(p[2], q[0]), a::mul(q[2], p[0])),
        a::sub(a::mul(p[0], q[1]), a::mul(q[0], p[1])),
        0.0,
    ]
}

/// FMINNM: a single quiet NaN yields the other operand; -0 below +0.
fn min_nm(x: f32, y: f32) -> f32 {
    const QUIET: u32 = 0x0040_0000;
    let (xn, yn) = (x.is_nan(), y.is_nan());
    if xn && !yn && x.to_bits() & QUIET != 0 { return y; }
    if yn && !xn && y.to_bits() & QUIET != 0 { return x; }
    a::min(x, y)
}

/// The camera position and camera Z axis the sort reads, in the simulation
/// space (see the module comment).
fn camera_in_simulation(camera: &SortCamera) -> Result<([f32; 3], [f32; 3]), SortRefused> {
    let v: [V4; 4] = std::array::from_fn(|i| col(&camera.view, i));
    let o = |j: usize, k: usize| camera.owner[4 * j + k];
    let m: [V4; 3] = std::array::from_fn(|j| vadd(vscale(v[0], o(j, 0)), vadd(vscale(v[1], o(j, 1)), vscale(v[2], o(j, 2)))));
    let t = vadd(v[3], vadd(vscale(v[0], o(3, 0)), vadd(vscale(v[1], o(3, 1)), vscale(v[2], o(3, 2)))));
    let l: [f32; 3] = std::array::from_fn(|k| sum3(vmul(m[k], m[k])));
    let s3 = a::mul(a::add(a::add(l[0], l[1]), l[2]), THIRD_APPROX);
    let inv: [V4; 3] = if s3 < TINY_SCALE {
        [[0.0; 4]; 3]
    } else {
        let r = a::mul(a::rsqrt_estimate(s3), RSQ_BIAS);
        let (c0, c1, c2) = (vscale(m[0], r), vscale(m[1], r), vscale(m[2], r));
        let cr12 = cross(c1, c2);
        let det = sum3(vmul(c0, cr12));
        if a::abs(det) <= DET_EPS {
            return Err(SortRefused::SingularOwner);
        }
        let cr20 = cross(c2, c0);
        let cr01 = cross(c0, c1);
        let idet = a::div(1.0, det);
        std::array::from_fn(|i| vscale(vscale([cr12[i], cr20[i], cr01[i], 0.0], idet), r))
    };
    let nt = t.map(a::neg);
    let cam = vadd(vscale(inv[0], nt[0]), vadd(vscale(inv[1], nt[1]), vscale(inv[2], nt[2])));
    let n = inv[2];
    let length2 = sum3(vmul(n, n));
    let n = if length2 > TINY_SCALE {
        let rs = a::rsqrt2(length2);
        [a::mul(n[0], rs), a::mul(n[1], rs), a::mul(n[2], rs)]
    } else {
        [0.0, 0.0, 1.0]
    };
    Ok(([cam[0], cam[1], cam[2]], n))
}

impl ParticleSort {
    pub fn from_source(mode: u32) -> Option<Self> {
        Some(match mode {
            0 => Self::None, 1 => Self::Distance,
            2 => Self::OldestInFront, 3 => Self::YoungestInFront,
            4 => Self::Depth,
            _ => return None,
        })
    }

    /// The per-particle sort key words, in pool order, as the renderer writes
    /// them next to each particle index.
    pub fn keys(self, particles: &[SortParticle], camera: &SortCamera) -> Result<Vec<f32>, SortRefused> {
        Ok(match self {
            Self::None => vec![0.0; particles.len()],
            // The renderer sorts remaining lifetime, not normalized age.
            Self::OldestInFront | Self::YoungestInFront => particles.iter()
                .map(|p| a::div(a::max(a::sub(100.0, p.age_percent), 0.0), p.inverse_lifetime)).collect(),
            Self::Distance | Self::Depth => {
                let (cam, n) = camera_in_simulation(camera)?;
                if self == Self::Depth || camera.orthographic {
                    let dot = a::add(a::add(a::mul(cam[0], n[0]), a::mul(cam[1], n[1])), a::mul(cam[2], n[2]));
                    let base = a::sub(camera.near, dot);
                    particles.iter().map(|p| {
                        let q = p.position;
                        let along = a::add(a::add(a::mul(n[0], q[0]), a::mul(n[1], q[1])), a::mul(n[2], q[2]));
                        min_nm(a::add(base, along), DEPTH_CEILING)
                    }).collect()
                } else {
                    particles.iter().map(|p| {
                        let d: V4 = [a::sub(p.position[0], cam[0]), a::sub(p.position[1], cam[1]), a::sub(p.position[2], cam[2]), 0.0];
                        a::neg(sum3(vmul(d, d)))
                    }).collect()
                }
            }
        })
    }

    /// Draw order: indices into `particles`, first drawn first.
    pub fn indices(self, particles: &[SortParticle], camera: &SortCamera) -> Result<Vec<usize>, SortRefused> {
        let mut indices: Vec<_> = (0..particles.len()).collect();
        if self == Self::None { return Ok(indices); }
        let keys = self.keys(particles, camera)?;
        // Source comparison uses unsigned float bits, then the original
        // particle index. Equal distances/lifetimes therefore have an
        // explicit order; a stable float sort would give a different one.
        let key = |index: usize| (u64::from(keys[index].to_bits()) << 32) | index as u64;
        if self == Self::YoungestInFront {
            indices.sort_unstable_by_key(|index| key(*index));
        } else {
            indices.sort_unstable_by_key(|index| std::cmp::Reverse(key(*index)));
        }
        Ok(indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{self, Value};

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value { v.get(key).unwrap_or_else(|| panic!("row field {key}")) }
    fn word(v: &Value) -> u32 { v.as_f64().expect("word") as u32 }
    fn words(v: &Value) -> Vec<u32> { v.as_array().expect("words").iter().map(word).collect() }

    /// Research instrument: ParticleSystemRenderer::Sort executed in an ARMv8
    /// emulator on the current engine library, over sampled views, owners
    /// (rotation, nonuniform and negative scale), near planes, orthographic
    /// cameras, exact position ties and particles at the camera. Every row's
    /// index order and every key word must match. Rows of the engine's two
    /// internal ascending spatial modes (5, 6) are replayed through the same
    /// keys with the ascending order. Point MOLY_PARTICLE_DEPTH_SORT_ROWS at the
    /// recorded rows; MOLY_SORT_MUTANT names a deliberate defect that must fail.
    #[test]
    #[ignore = "needs MOLY_PARTICLE_DEPTH_SORT_ROWS"]
    fn depth_and_distance_sort_match_native_rows() {
        let path = std::env::var("MOLY_PARTICLE_DEPTH_SORT_ROWS").expect("MOLY_PARTICLE_DEPTH_SORT_ROWS");
        let doc = json::parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        assert_eq!(field(&doc, "sourceSha256").as_str(), Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let mutant = std::env::var("MOLY_SORT_MUTANT").unwrap_or_default();
        let (mut rows, mut particles, mut failures) = (0usize, 0usize, Vec::new());
        let mut per_mode = [0usize; 7];
        for (r, row) in field(&doc, "rows").as_array().unwrap().iter().enumerate() {
            let mode = word(field(row, "mode"));
            let view: [u32; 16] = words(field(row, "view")).try_into().unwrap();
            let owner: [u32; 16] = words(field(row, "owner")).try_into().unwrap();
            let mut camera = SortCamera { view: view.map(f32::from_bits), owner: owner.map(f32::from_bits),
                near: f32::from_bits(word(field(row, "near"))), orthographic: field(row, "orthographic").as_bool().unwrap() };
            let ages = words(field(row, "ages"));
            let lifetimes = words(field(row, "inverseLifetimes"));
            let list: Vec<SortParticle> = field(row, "positions").as_array().unwrap().iter().enumerate().map(|(i, p)| {
                let p = words(p);
                SortParticle { position: [f32::from_bits(p[0]), f32::from_bits(p[1]), f32::from_bits(p[2])],
                    age_percent: f32::from_bits(ages[i]), inverse_lifetime: f32::from_bits(lifetimes[i]) }
            }).collect();
            // 5 and 6 are the ascending variants of Distance and Depth.
            let (sort, ascending) = match mode { 5 => (ParticleSort::Distance, true), 6 => (ParticleSort::Depth, true),
                m => (ParticleSort::from_source(m).unwrap(), false) };
            let sort = match (mutant.as_str(), sort) {
                ("depth-as-distance", ParticleSort::Depth) => ParticleSort::Distance,
                ("distance-as-depth", ParticleSort::Distance) => ParticleSort::Depth,
                _ => sort,
            };
            if mutant == "no-near" { camera.near = 0.0; }
            if mutant == "no-ortho" { camera.orthographic = false; }
            let keys = sort.keys(&list, &camera).expect("no singular owner in the rows");
            let mut order: Vec<usize> = (0..list.len()).collect();
            let key = |i: usize| (u64::from(keys[i].to_bits()) << 32) | i as u64;
            let ascending = ascending || sort == ParticleSort::YoungestInFront;
            if mutant == "float-order" {
                order.sort_by(|x, y| keys[*y].partial_cmp(&keys[*x]).unwrap_or(std::cmp::Ordering::Equal));
                if ascending { order.reverse(); }
            } else if ascending {
                order.sort_unstable_by_key(|i| key(*i));
            } else {
                order.sort_unstable_by_key(|i| std::cmp::Reverse(key(*i)));
            }
            if mode <= 4 && mutant.is_empty() {
                assert_eq!(order, sort.indices(&list, &camera).unwrap(), "indices() must be the replayed order");
            }
            let native = words(field(row, "indices"));
            let native_keys = words(field(row, "keys"));
            let ours: Vec<u32> = order.iter().map(|i| *i as u32).collect();
            let ours_keys: Vec<u32> = order.iter().map(|i| keys[*i].to_bits()).collect();
            if ours != native || ours_keys != native_keys {
                failures.push(format!("row {r} mode {mode}: native {native:?} ours {ours:?}"));
            }
            rows += 1; particles += list.len(); per_mode[mode as usize] += 1;
        }
        println!("depth sort: {rows} rows, {particles} particles, per mode {per_mode:?}, {} mismatched", failures.len());
        assert!(rows > 0 && per_mode[4] > 0, "the rows must include Depth");
        assert!(failures.is_empty(), "{} mismatched rows:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    }
}
