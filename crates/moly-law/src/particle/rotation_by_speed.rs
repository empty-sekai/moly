//! RotationBySpeedModule: angular velocity by particle speed.
//!
//! Tier: engine native, instruction for instruction (constants pinned by bit).
//!
//! Where it runs. The pre-simulation module chain (the same function for the
//! existing particles of a slice and for the newborns of a birth command)
//! runs Initial, ExternalForces, RotationOverLifetime, Velocity, Noise,
//! InheritVelocity, Force, ClampVelocity, then this module, then CustomData.
//! Initial clears the angular-velocity lanes of the range when either
//! rotation module is enabled (all three axes with 3D rotation, only z
//! without); RotationOverLifetime and this module each ADD their value to
//! those lanes; the rotation is integrated once afterwards as
//! `rotation = angular * dt + rotation` (a multiply then an add), after the
//! position integration of a birth and after the kill pass of an update.
//! So with both modules on, the integrated rate is `(0 + rol) + rbs`, rounded
//! as two adds, not two separate integrations.
//!
//! What it adds, per particle and per axis (z only unless separateAxes, in
//! which case x, y, z in that order):
//!
//! | mode | value |
//! |---|---|
//! | constant | the scalar (the speed is not read) |
//! | two constants | `min + r * (max - min)`, r from this module's own lerp hash |
//! | curve, optimized | the polynomial at `t` (multiplier folded into the coefficients) |
//!
//! and the value is negated unless the sign hash is above the Initial
//! module's randomize-rotation-direction value. The sign hash is the one the
//! rotation-over-lifetime module uses; the two-constant lerp hash is this
//! module's own (seed + 0xdec4aea1, seed * M + 0xf029defc), not the
//! rotation-over-lifetime one. The curve time is
//! `t = min(max(speed * scale + offset, +0), 1)` with
//! `speed = sqrt(x*x + (y*y + z*z))` over persistent plus animated velocity
//! as the chain leaves them after ClampVelocity, and `(scale, offset)` the
//! range remap: `scale = 1 / (b - a)` when `|b - a| > 0x3089705f`, else the
//! range start `a` itself; `offset = scale * -a`.
//!
//! Not transcribed (the module dispatches them to a separate templated body
//! that has not been read): a curve lane without the optimized bit and the
//! two-curve mode. Those refuse at construction by name.
//!
//! Random stream consumption: none. Every factor is a pure hash of the
//! particle seed.

use crate::particle::curve::{arm_fmax, arm_fmin, CurveSampler, CurveTime};
use crate::particle::random::hash_mix;
use crate::particle::schema::RotationBySpeedParams;

/// Multiplier of the hash family (shared with the rotation modules).
const HASH_M: u32 = 0x6ab5_1b9d;
/// Sign hash addends (the rotation-over-lifetime module's).
const HASH_T_SIGN: u32 = 0xff2b_b1a4;
const HASH_C_SIGN: u32 = 0x0bc7_08d3;
/// Two-constant lerp hash addends of this module.
const HASH_T_LERP: u32 = 0xdec4_aea1;
const HASH_C_LERP: u32 = 0xf029_defc;
/// Range remap threshold on `|b - a|`.
const RANGE_EPSILON: u32 = 0x3089_705f;

fn sign_hash(seed: u32) -> f32 {
    hash_mix(seed.wrapping_add(HASH_T_SIGN), seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_SIGN))
}

fn lerp_hash(seed: u32) -> f32 {
    hash_mix(seed.wrapping_add(HASH_T_LERP), seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_LERP))
}

/// The speed range remap `(scale, offset)`.
pub fn range_remap(range: [f32; 2]) -> (f32, f32) {
    let [a, b] = range;
    let span = b - a;
    let scale = if (b - a).abs() > f32::from_bits(RANGE_EPSILON) { 1.0 / span } else { a };
    (scale, scale * -a)
}

#[derive(Clone, Debug)]
enum AxisLaw {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
    Curve(CurveSampler),
}

#[derive(Clone, Debug)]
pub struct RotationBySpeed {
    /// x, y, z; x and y only with separate axes.
    axes: [Option<AxisLaw>; 3],
    scale: f32,
    offset: f32,
}

impl RotationBySpeed {
    pub fn from_params(params: &RotationBySpeedParams) -> Result<Self, String> {
        let axis = |curve: &crate::particle::MinMaxCurve| -> Result<AxisLaw, String> {
            match curve {
                crate::particle::MinMaxCurve::Constant(v) => Ok(AxisLaw::Constant(*v)),
                crate::particle::MinMaxCurve::TwoConstants { min, max } => {
                    Ok(AxisLaw::TwoConstants { min: *min, max: *max })
                }
                crate::particle::MinMaxCurve::Curve { .. } => {
                    match CurveSampler::new(curve, CurveTime::Normalized)
                        .map_err(|reason| format!("rotationBySpeed: {reason}"))? {
                        sampler @ CurveSampler::CurveBaked(_) => Ok(AxisLaw::Curve(sampler)),
                        _ => Err("rotationBySpeed: a curve without the optimized bit takes the templated body, which is not transcribed".into()),
                    }
                }
                crate::particle::MinMaxCurve::TwoCurves { .. } => {
                    Err("rotationBySpeed: the two-curve mode takes the templated body, which is not transcribed".into())
                }
            }
        };
        let axes = if params.separate_axes {
            let (Some(x), Some(y)) = (params.x.as_ref(), params.y.as_ref()) else {
                return Err("rotationBySpeed: separate axes without the exported x and y curves".into());
            };
            [Some(axis(x)?), Some(axis(y)?), Some(axis(&params.curve)?)]
        } else {
            [None, None, Some(axis(&params.curve)?)]
        };
        let (scale, offset) = range_remap(params.range);
        Ok(Self { axes, scale, offset })
    }

    /// Whether the module reads more than the z lane (the particle arrays use
    /// 3D rotation for an enabled module with separate axes).
    pub fn separate_axes(&self) -> bool {
        self.axes[0].is_some()
    }

    /// Add this module's value to `angular` (x, y, z) for one particle.
    /// `velocity` and `animated` are the persistent and animated velocity as
    /// the chain leaves them after ClampVelocity (any consistent axis
    /// reflection: only squares are read).
    pub fn add(&self, angular: &mut [f32; 3], seed: u32, randomize_direction: f32,
        velocity: [f32; 3], animated: [f32; 3]) {
        let mut time = None;
        for (index, law) in self.axes.iter().enumerate() {
            let Some(law) = law else { continue };
            let value = match law {
                AxisLaw::Constant(v) => *v,
                AxisLaw::TwoConstants { min, max } => min + lerp_hash(seed) * (max - min),
                AxisLaw::Curve(sampler) => {
                    let t = *time.get_or_insert_with(|| {
                        let s = |axis: usize| velocity[axis] + animated[axis];
                        let (x, y, z) = (s(0), s(1), s(2));
                        let speed = (x * x + (y * y + z * z)).sqrt();
                        arm_fmin(arm_fmax(self.offset + self.scale * speed, 0.0), 1.0)
                    });
                    sampler.evaluate(t, 0.0)
                }
            };
            let signed = if sign_hash(seed) > randomize_direction { value } else { -value };
            angular[index] += signed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::MinMaxCurve;

    fn params(curve: MinMaxCurve, range: [f32; 2]) -> RotationBySpeedParams {
        RotationBySpeedParams { separate_axes: false, curve, x: None, y: None, range }
    }

    #[test]
    fn sign_hash_equals_the_rotation_modules_translated_constant_form() {
        // The constant-mode body adds T before the multiply and C' = 0x714acb3f
        // after it; (seed + T) * M + C' == seed * M + C for every seed.
        for seed in [0_u32, 1, 17, 0xdead_beef, u32::MAX] {
            let translated = hash_mix(seed.wrapping_add(HASH_T_SIGN),
                seed.wrapping_add(HASH_T_SIGN).wrapping_mul(HASH_M).wrapping_add(0x714a_cb3f));
            assert_eq!(translated.to_bits(), sign_hash(seed).to_bits());
        }
    }

    #[test]
    fn range_remap_degenerate_range_uses_the_start() {
        assert_eq!(range_remap([0.0, 1.0]), (1.0, -0.0));
        let (scale, offset) = range_remap([2.0, 2.0]);
        assert_eq!((scale, offset), (2.0, -4.0));
    }

    #[test]
    fn constant_mode_adds_into_z_only() {
        let law = RotationBySpeed::from_params(&params(MinMaxCurve::Constant(1.5), [0.0, 1.0])).unwrap();
        let mut angular = [0.25, 0.5, 0.75];
        law.add(&mut angular, 12345, 0.0, [3.0, 0.0, 0.0], [0.0; 3]);
        assert_eq!(angular[..2], [0.25, 0.5]);
        let expected = if sign_hash(12345) > 0.0 { 0.75 + 1.5 } else { 0.75 - 1.5 };
        assert_eq!(angular[2].to_bits(), (expected as f32).to_bits());
    }

    #[test]
    fn unoptimized_and_two_curve_modes_refuse() {
        use crate::particle::{Curve, CurveKey};
        let key = |time: f32, value: f32| CurveKey { time, value, in_slope: 0.0, out_slope: 0.0,
            weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 };
        let curve = Curve { keys: vec![key(0.0, 1.0), key(1.0, 0.0)], multiplier: 1.0, pre_wrap: None, post_wrap: None };
        let two = MinMaxCurve::TwoCurves { multiplier: 1.0, min: curve.clone(), max: curve };
        assert!(RotationBySpeed::from_params(&params(two, [0.0, 1.0])).is_err());
    }
}

/// Native rows of RotationBySpeedModule::Update, called directly in the
/// game's own JP libunity with the pre-simulation chain's arguments: every
/// row's module block (in the export's shape), particle seeds, persistent and
/// animated velocity and the angular-velocity lanes before the call, and the
/// lanes after it. The replay parses the block through the product schema,
/// builds the law and adds into the same lanes; every lane of every row must
/// match bit for bit. A control file of mutated rows must fail.
#[cfg(test)]
mod native_rows {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn words(value: &Value) -> Vec<u32> {
        value.as_array().expect("array").iter().map(|v| v.as_f64().expect("word") as u32).collect()
    }

    fn lanes(value: &Value) -> [Vec<u32>; 3] {
        let axes = value.as_array().expect("three lanes");
        assert_eq!(axes.len(), 3);
        std::array::from_fn(|axis| words(&axes[axis]))
    }

    /// (rows, particles, mismatched lanes, first mismatches). Rows whose
    /// block the law refuses (the templated curve bodies) are skipped and
    /// named on stdout.
    pub(crate) fn replay(path: &std::ffi::OsStr) -> (usize, usize, usize, Vec<String>) {
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("sourceSha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let (mut rows, mut particles, mut bad) = (0, 0, 0);
        let mut first = Vec::new();
        for row in receipt.get("rows").and_then(Value::as_array).expect("rows") {
            let name = row.get("name").and_then(Value::as_str).unwrap_or("?");
            let module = row.get("module").expect("module");
            let params = RotationBySpeedParams::from_value(module, name).expect("module block");
            let law = match RotationBySpeed::from_params(&params) {
                Ok(law) => law,
                Err(reason) => { println!("rotation by speed row {name} skipped: {reason}"); continue; }
            };
            // The module compares lane j of each four-lane group (counted
            // from the range start) with lane j of the randomize vector; the
            // pre-simulation chain broadcasts one value.
            let randomize = words(row.get("randomizeBits").expect("randomize"));
            let from = row.get("from").and_then(Value::as_f64).unwrap() as usize;
            let to = row.get("to").and_then(Value::as_f64).unwrap() as usize;
            let p = row.get("particles").expect("particles");
            let seed = words(p.get("seed").unwrap());
            let velocity = lanes(p.get("vel").unwrap());
            let animated = lanes(p.get("anim").unwrap());
            let before = lanes(p.get("angularBefore").unwrap());
            let after = lanes(row.get("angularAfter").unwrap());
            // The body writes whole four-lane groups counted from the range
            // start, so the padding lanes of the last group are written too.
            let end = (from + (to.saturating_sub(from) + 3) / 4 * 4).min(seed.len());
            for index in 0..seed.len() {
                let mut angular: [f32; 3] = std::array::from_fn(|a| f32::from_bits(before[a][index]));
                if (from..end).contains(&index) {
                    law.add(&mut angular, seed[index],
                        f32::from_bits(randomize[(index - from) % 4]),
                        std::array::from_fn(|a| f32::from_bits(velocity[a][index])),
                        std::array::from_fn(|a| f32::from_bits(animated[a][index])));
                }
                for axis in 0..3 {
                    if angular[axis].to_bits() != after[axis][index] {
                        bad += 1;
                        if first.len() < 12 {
                            first.push(format!("{name} particle {index} axis {axis}: {:08x} native {:08x}",
                                angular[axis].to_bits(), after[axis][index]));
                        }
                    }
                }
                particles += 1;
            }
            rows += 1;
        }
        (rows, particles, bad, first)
    }

    #[test]
    #[ignore = "MOLY_RBS_ROWS identifies the current JP RotationBySpeedModule rows"]
    fn rotation_by_speed_matches_native_rows() {
        let path = std::env::var_os("MOLY_RBS_ROWS").expect("MOLY_RBS_ROWS");
        let (rows, particles, bad, first) = replay(&path);
        println!("rotation by speed rows: {rows} rows, {particles} particles, {bad} mismatched lanes {first:?}");
        assert!(rows > 0 && particles > 0);
        assert_eq!(bad, 0, "{first:?}");
    }

    /// Each control file (colon-separated) must mismatch somewhere.
    #[test]
    #[ignore = "MOLY_RBS_ROWS_CONTROLS identifies mutated RotationBySpeedModule rows"]
    fn rotation_by_speed_controls_fail() {
        let paths = std::env::var("MOLY_RBS_ROWS_CONTROLS").expect("MOLY_RBS_ROWS_CONTROLS");
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            let (rows, _, bad, _) = replay(std::ffi::OsStr::new(path));
            println!("rotation by speed control {path}: {rows} rows, {bad} mismatched lanes");
            assert!(bad > 0, "control {path} must mismatch");
        }
    }
}
