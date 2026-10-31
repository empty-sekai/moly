//! limitVelocity: the limit (the speed magnitude converges on a threshold)
//! and the drag law.
//!
//! Tier: `bit-exact`: results, the order of operations and the constants match
//! the reference bit for bit.
//!
//! Input: the particle's state velocity and the animated velocity added before
//! this module in the same frame, `dt`, the dampen, the drag and its size and
//! speed factors, and the speed limit in its magnitude mode. Output: the state
//! velocity after both sections, which run in this order when their gates
//! open; the animated velocity is read, never written.
//!
//! - **Clamp section** (gate `dampen > 0`; a NaN dampen skips it), once per
//!   lane with that lane's `dt`:
//!   `k = 1.0 - powf(1.0 - dampen, |dt| * 30.0)`; then per particle
//!   `mag2 = x² + (y²+z²)` over the combined velocity (state plus animated),
//!   `newmag = (limit < |mag|) ? |mag| + (limit - |mag|) * k : |mag|`, and
//!   `state velocity = unit vector * newmag - animated velocity`.
//!   The limit comes from its magnitude mode: a constant (no seed read), two
//!   constants (this section's own hash, an affine form, the lerp not
//!   clamped), or a baked curve read at `t = max(agePercent * 0.01, 0)` (no
//!   seed read).
//! - **Drag section** (gate `drag != 0`; a NaN drag runs it): from the
//!   combined velocity recomputed after the clamp, `factor = ((size factor *
//!   drag) * speed factor) * dt`, `newmag = mag - factor` floored at 0, and
//!   the unit vector by dividing by `mag`.
//!
//! Named gaps (each value comparison reports their magnitude):
//! - `powf` is a library function called through the dynamic linker, so `k`
//!   is the host `powf` and its bit equivalence with the device libm is not
//!   established; this implementation calls it once per lane
//!   (`|dt[lane]| * 30`), so the caller gives `dt` per lane.
//! - The clamp section's unit vector is the combined velocity times the
//!   reciprocal square root of `mag2` (`armf::rsqrt2`: an estimate refined by
//!   two steps, the estimate itself where `mag2` is 0, so the step is not
//!   exact); outside the gate `1e-30 < mag2` it is +0, and `newmag` is
//!   multiplied by 1.0/0.0 from `(0 < mag)`. The drag section divides exactly
//!   (a rounding that does not match the reference; drag is 0 throughout the
//!   corpus, so that section does not run there).
//!
//! Random streams: 0 draws per frame. The only hash (the two-constant limit)
//! is a pure function of the per-particle seed, constant for the particle's
//! life.
//!
//! Not implemented (the engine has the path, the corpus has 0 cases, and
//! construction refuses by name): the separateAxis per-axis family, general
//! magnitude curves (modes 1/2 unbaked), the mode 2 baked magnitude, and a
//! drag that is not constant (modes 1/2/3).

use crate::particle::armf as a;
use crate::particle::buffer::RingBufferMode;
use crate::particle::curve::{curve_time_fmax, BakedCurve};
use crate::particle::rotation::hash_mix;
use crate::particle::value::MinMaxCurve;

// ---- Hashing (the clamp section's two constants) ----

/// The clamp section's added term T.
const HASH_T_CLAMP: u32 = 0x1337_1337;
/// The clamp section's added term C (the same constant as the
/// angular-velocity constant factor).
const HASH_C_CONST: u32 = 0x714a_cb3f;
/// The multiplier M (shared with the angular-velocity family).
const HASH_M: u32 = 0x6ab5_1b9d;

/// The lerp factor of the two-constant limit, an affine form:
/// `t = seed + T_clamp`, `m = (seed + T_clamp) * M + C_const`. Not clamped.
fn clamp_lerp_hash(seed: u32) -> f32 {
    let t = seed.wrapping_add(HASH_T_CLAMP);
    hash_mix(
        t,
        t.wrapping_mul(HASH_M).wrapping_add(HASH_C_CONST),
    )
}

// ---- Convergence factor ----

/// `k = 1.0 - powf(1.0 - dampen, |dt| * 30.0)`. `powf` is a library function,
/// so a bit-exact match is out of reach by construction (a named gap).
pub fn clamp_k(dampen: f32, dt: f32) -> f32 {
    1.0 - (1.0 - dampen).powf(dt.abs() * 30.0)
}

// ---- Magnitude law ----

/// How the speed limit is taken.
#[derive(Clone, Debug)]
pub enum MagnitudeLaw {
    /// Mode 0: a constant. Neither the seed nor the age is read.
    Constant(f32),
    /// Mode 3: two constants. Hashed once per particle; the lerp is not clamped.
    TwoConstants { min: f32, max: f32 },
    /// Mode 1 with the reader's isOptimizedCurve bit set: ClampVelocityModule
    /// evaluates the polynomial inline, operation for operation as
    /// `EvaluateThreaded`'s optimized branch, at `t = fmax(agePercent * 0.01,
    /// +0)` (a NaN age stays NaN); the coefficients carry the multiplier.
    Baked(BakedCurve),
}

impl MagnitudeLaw {
    /// Builds from the four documented modes. A general curve and the two-curve
    /// mode are refused by name: the corpus has 0 cases of them.
    pub fn from_min_max_curve(curve: &MinMaxCurve) -> Result<Self, String> {
        match curve {
            MinMaxCurve::Constant(v) => Ok(MagnitudeLaw::Constant(*v)),
            MinMaxCurve::TwoConstants { min, max } => Ok(MagnitudeLaw::TwoConstants {
                min: *min,
                max: *max,
            }),
            MinMaxCurve::Curve { multiplier, max } => match BakedCurve::bake(&max.keys, *multiplier)
            {
                Some(b) => Ok(MagnitudeLaw::Baked(b)),
                None => Err(
                    "limitVelocity.magnitude: a curve without the isOptimizedCurve bit takes MagnitudeUpdateTpl, which is unsupported"
                        .to_string(),
                ),
            },
            MinMaxCurve::TwoCurves { .. } => Err(
                "limitVelocity.magnitude: the two-curve mode is unsupported".to_string(),
            ),
        }
    }

    /// The speed limit of one particle, from the module's `_velocity`.
    pub fn limit(&self, seed: u32, age_percent: f32) -> f32 {
        match self {
            MagnitudeLaw::Constant(v) => *v,
            MagnitudeLaw::TwoConstants { min, max } => {
                let r = clamp_lerp_hash(seed);
                (max - min) * r + min
            }
            MagnitudeLaw::Baked(b) => b.evaluate(curve_time_fmax(age_percent)),
        }
    }
}

// ---- Drag ----

/// How the drag is taken. Only the constant mode is implemented; drag is 0
/// throughout the corpus.
#[derive(Clone, Copy, Debug)]
pub enum DragLaw {
    /// Mode 0: a constant. Neither the seed nor the age is read.
    Constant(f32),
}

/// The size input of the drag section. The engine picks the start or the
/// current size array by a container flag; when it is written and which array
/// is chosen is the caller's responsibility. The three components, and whether
/// the particle arrays declare a 3D size.
#[derive(Clone, Copy, Debug)]
pub struct DragSize {
    pub components: [f32; 3],
    pub size3d: bool,
}

// ---- `maxps` semantics ----

/// The engine's `maxps`: a NaN operand gives the source operand back. Rust's
/// `f32::max` returns the other operand instead, so it cannot stand in.
fn maxps(dst: f32, src: f32) -> f32 {
    if dst.is_nan() || src.is_nan() {
        src
    } else {
        dst.max(src)
    }
}

/// The module. `inWorldSpace` is not read on the planar-vector path, so it has
/// no field here.
#[derive(Clone, Debug)]
pub struct LimitVelocity {
    /// At or below 0 (NaN included) the clamp section is skipped whole.
    pub dampen: f32,
    pub magnitude: MagnitudeLaw,
    /// `None` skips the drag section; `drag == 0` is the canonical form (0).
    pub drag: Option<DragLaw>,
    /// Drag times the particle size (half the size squared, times pi). An
    /// unknown value with a nonzero drag is refused at construction; a `false`
    /// here is either the data's own value or sits beside a zero drag, where it
    /// is inert.
    pub multiply_drag_by_size: bool,
    /// Drag times the squared speed, as above.
    pub multiply_drag_by_velocity: bool,
}

impl LimitVelocity {
    /// Builds from the documented fields. `separate_axis` (the per-axis family
    /// is not implemented; 0 corpus cases) and a drag that is not constant are
    /// refused by name. The extraction side does not export the two multiplier
    /// flags yet (`None` means unknown): with a zero drag they are inert and may
    /// be absent, but a nonzero drag with an unknown flag is refused, because a
    /// key that can change the result is not defaulted to false.
    pub fn from_parts(
        separate_axis: bool,
        magnitude: &MinMaxCurve,
        dampen: f32,
        drag: Option<&MinMaxCurve>,
        multiply_drag_by_size: Option<bool>,
        multiply_drag_by_velocity: Option<bool>,
    ) -> Result<Self, String> {
        if separate_axis {
            return Err("limitVelocity: the separateAxis per-axis family is unsupported".to_string());
        }
        let drag = match drag {
            None => None,
            Some(MinMaxCurve::Constant(v)) => Some(DragLaw::Constant(*v)),
            Some(_) => {
                return Err(
                    "limitVelocity.drag: a non-constant drag is unsupported (the corpus is constant throughout)"
                        .to_string(),
                )
            }
        };
        // Whether the drag section runs: the gate is `drag != 0`, and a NaN
        // drag passes it.
        let drag_runs = matches!(drag, Some(DragLaw::Constant(v)) if v != 0.0);
        if drag_runs && (multiply_drag_by_size.is_none() || multiply_drag_by_velocity.is_none()) {
            return Err(
                "limitVelocity: a drag multiplier flag (size, speed squared) is missing with a nonzero drag; a key that changes the result is not defaulted"
                    .to_string(),
            );
        }
        Ok(Self {
            dampen,
            magnitude: MagnitudeLaw::from_min_max_curve(magnitude)?,
            drag,
            multiply_drag_by_size: multiply_drag_by_size.unwrap_or(false),
            multiply_drag_by_velocity: multiply_drag_by_velocity.unwrap_or(false),
        })
    }

    /// One step: the clamp section, then the drag section, in that order within
    /// the frame. `velocity` is the state velocity, rewritten in place;
    /// `animated_velocity` is the added velocity computed earlier in the frame
    /// (both sections use the combined velocity, and only the state velocity is
    /// written back). The drag section recomputes the combined velocity from the
    /// post-clamp state velocity. A non-finite `dt` is refused.
    pub fn step(
        &self,
        velocity: &mut [f32; 3],
        animated_velocity: [f32; 3],
        seed: u32,
        age_percent: f32,
        dt: f32,
        drag_size: DragSize,
    ) -> Result<(), &'static str> {
        if !dt.is_finite() {
            return Err("limitVelocity: non-finite dt");
        }
        if self.dampen > 0.0 {
            let k = clamp_k(self.dampen, dt);
            let limit = self.magnitude.limit(seed, age_percent);
            clamp_body(velocity, animated_velocity, limit, k);
        }
        if let Some(DragLaw::Constant(drag)) = self.drag {
            if drag != 0.0 {
                drag_body(
                    velocity,
                    animated_velocity,
                    drag,
                    dt,
                    drag_size,
                    self.multiply_drag_by_size,
                    self.multiply_drag_by_velocity,
                );
            }
        }
        Ok(())
    }
}

/// The clamp section body: `mag2 = x² + (y²+z²)`; the `(0 < mag)` mask tests
/// the square root before its absolute value is taken; the unit vector gate is
/// `1e-30 < mag2`; `newmag` is then multiplied by the `(0 < mag)` mask.
fn clamp_body(velocity: &mut [f32; 3], anim: [f32; 3], limit: f32, k: f32) {
    let total = [
        velocity[0] + anim[0],
        velocity[1] + anim[1],
        velocity[2] + anim[2],
    ];
    let mag2 = total[0] * total[0] + (total[1] * total[1] + total[2] * total[2]);
    let mag_raw = mag2.sqrt();
    let mag_positive = 0.0 < mag_raw;
    let mag = mag_raw.abs();
    let newmag = if limit < mag {
        (limit - mag) * k + mag
    } else {
        mag
    };
    // The unit vector is the combined velocity times a reciprocal square root
    // estimate refined twice on mag2 (the estimate itself where mag2 is
    // zero), masked to +0 unless 1e-30 < mag2; newmag is multiplied by the
    // (0 < mag) mask as 1.0 or 0.0, then each axis is unit * newmag - animated.
    let r = a::rsqrt2(mag2);
    let keep = f32::from_bits(0x0da2_4260) < mag2;
    let unit = total.map(|v| if keep { v * r } else { 0.0 });
    let newmag = newmag * if mag_positive { 1.0 } else { 0.0 };
    velocity[0] = unit[0] * newmag - anim[0];
    velocity[1] = unit[1] * newmag - anim[1];
    velocity[2] = unit[2] * newmag - anim[2];
}

/// The drag section body (2 of the module's sections, this one second): the
/// size is the largest of the three components when the arrays declare a 3D
/// size, otherwise x; the size factor is
/// `(half * pi) * half`; the speed factor is `mag2`; the factors multiply as
/// `((size factor * drag) * speed factor) * dt`; the unit vector divides
/// exactly (not with rsqrt as the clamp section does) behind the gate
/// `1e-15 < mag`, on `mag` rather than `mag2`; `newmag = mag - factor` floored
/// at 0 through a comparison rather than `f32::max`, because `maxps` returns
/// the source operand on a NaN and the comparison does the same.
fn drag_body(
    velocity: &mut [f32; 3],
    anim: [f32; 3],
    drag: f32,
    dt: f32,
    drag_size: DragSize,
    multiply_by_size: bool,
    multiply_by_velocity: bool,
) {
    let total = [
        velocity[0] + anim[0],
        velocity[1] + anim[1],
        velocity[2] + anim[2],
    ];
    let mag2 = total[0] * total[0] + (total[1] * total[1] + total[2] * total[2]);
    let c = drag_size.components;
    let size = if drag_size.size3d {
        maxps(c[0], maxps(c[1], c[2]))
    } else {
        c[0]
    };
    let half = size * 0.5;
    let size_factor = if multiply_by_size {
        (half * f32::from_bits(0x4049_0fdb)) * half
    } else {
        1.0
    };
    let vel_factor = if multiply_by_velocity { mag2 } else { 1.0 };
    let factor = ((size_factor * drag) * vel_factor) * dt;
    let mag = mag2.sqrt();
    let unit = if f32::from_bits(0x2690_1d7d) < mag {
        [total[0] / mag, total[1] / mag, total[2] / mag]
    } else {
        [0.0, 0.0, 0.0]
    };
    let reduced = mag - factor;
    let newmag = if 0.0 > reduced { 0.0 } else { reduced };
    velocity[0] = unit[0] * newmag - anim[0];
    velocity[1] = unit[1] * newmag - anim[1];
    velocity[2] = unit[2] * newmag - anim[2];
}

// ---- agePercent advance (inline in the engine's advancer, same frame) ----

/// One frame of agePercent (0..100):
/// `new = old + (1 / start_lifetime) * (dt * 100)`; in mode 2 (Loop) a value
/// at or past `fade_out` wraps by `fade_out - fade_in`, where
/// `fade = loopRange * 100` (the active-lane mask `activeCount > laneIndex`
/// belongs to the buffer's implementation and is not modelled here); then
/// `min(new, cap)`, with a lower cap in mode 1 (1 ulp below the other's). An
/// `old` above `100.0` is kept, so the age never falls back. The engine stores
/// the reciprocal
/// lifetime at birth; this function divides exactly instead (a named choice).
pub fn advance_age_percent(
    age_percent: &mut f32,
    start_lifetime: f32,
    dt: f32,
    ring_mode: RingBufferMode,
    loop_range: [f32; 2],
) {
    let old = *age_percent;
    let mut new = old + (1.0 / start_lifetime) * (dt * 100.0);
    if ring_mode == RingBufferMode::LoopUntilReplaced {
        let fade_in = loop_range[0] * 100.0;
        let fade_out = loop_range[1] * 100.0;
        if fade_out <= new {
            new -= fade_out - fade_in;
        }
    }
    let cap = match ring_mode {
        RingBufferMode::PauseUntilReplaced => f32::from_bits(0x42c7_ffff),
        _ => f32::from_bits(0x42c8_0001),
    };
    new = new.min(cap);
    *age_percent = if 100.0 < old { old } else { new };
}

#[cfg(test)]
mod native_magnitude_tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn word(v: &Value) -> u32 { v.as_f64().unwrap() as u32 }

    /// Deliberate misreadings of the clamp section, for the mutant arms.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Mutant { None, NoHashOffset, LerpFromMax, AgeUnscaled, DtUnscaled, AnimatedIgnored }

    fn mutant_step(law: &LimitVelocity, v: &mut [f32; 3], anim: [f32; 3], seed: u32, age: f32, dt: f32, m: Mutant) {
        let k = if m == Mutant::DtUnscaled { 1.0 - (1.0 - law.dampen).powf(dt.abs()) } else { clamp_k(law.dampen, dt) };
        let limit = match &law.magnitude {
            MagnitudeLaw::TwoConstants { min, max } => {
                let r = if m == Mutant::NoHashOffset {
                    hash_mix(seed, seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_CONST))
                } else {
                    clamp_lerp_hash(seed)
                };
                if m == Mutant::LerpFromMax { (min - max) * r + max } else { (max - min) * r + min }
            }
            MagnitudeLaw::Baked(b) if m == Mutant::AgeUnscaled => b.evaluate(if age > 0.0 { age } else { 0.0 }),
            other => other.limit(seed, age),
        };
        if m == Mutant::AnimatedIgnored { clamp_body(v, [0.0; 3], limit, k) } else { clamp_body(v, anim, limit, k) }
    }

    // Native ClampVelocityModule rows on the two-constant and optimized-curve
    // magnitude paths (drag 0, as every data row), at dt lanes whose powf
    // exponent is 0, 1 or 2. Each admitted row must match bit for bit (a NaN
    // matches a NaN); every mutant arm must turn rows red.
    #[test]
    #[ignore = "MOLY_CLAMP_MAGNITUDE_NATIVE must identify the native clamp-velocity magnitude rows"]
    fn clamp_magnitude_paths_match_native() {
        let path = std::env::var_os("MOLY_CLAMP_MAGNITUDE_NATIVE").expect("MOLY_CLAMP_MAGNITUDE_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("summary").unwrap().get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let arms = [Mutant::NoHashOffset, Mutant::LerpFromMax, Mutant::AgeUnscaled, Mutant::DtUnscaled, Mutant::AnimatedIgnored];
        let mut red = vec![0usize; arms.len()];
        let (mut cases, mut refused, mut rows, mut two, mut baked) = (0, 0, 0, 0, 0);
        let size = DragSize { components: [1.0; 3], size3d: false };
        for case in receipt.get("cases").unwrap().as_array().unwrap() {
            cases += 1;
            let magnitude = crate::particle::schema::min_max_curve(case.get("magnitude"), "magnitude").unwrap();
            let dampen = case.get("dampen").unwrap().as_f64().unwrap() as f32;
            let optimized = case.get("optimized").unwrap().as_bool().unwrap();
            let law = match LimitVelocity::from_parts(false, &magnitude, dampen, Some(&MinMaxCurve::Constant(0.0)),
                Some(true), Some(true)) {
                Ok(law) => law,
                Err(reason) => {
                    assert!(!optimized, "case {cases}: an optimized magnitude refused: {reason}");
                    refused += 1;
                    continue;
                }
            };
            assert!(optimized, "case {cases}: the law admitted a magnitude the reader leaves unoptimized");
            match law.magnitude { MagnitudeLaw::TwoConstants { .. } => two += 1, MagnitudeLaw::Baked(_) => baked += 1, _ => {} }
            for row in case.get("rows").unwrap().as_array().unwrap() {
                let r: Vec<u32> = row.as_array().unwrap().iter().map(word).collect();
                let (seed, age, dt) = (r[0], f32::from_bits(r[1]), f32::from_bits(r[2]));
                let start = [f32::from_bits(r[3]), f32::from_bits(r[4]), f32::from_bits(r[5])];
                let anim = [f32::from_bits(r[6]), f32::from_bits(r[7]), f32::from_bits(r[8])];
                let native = [r[9], r[10], r[11]];
                let same = |v: [f32; 3]| v.iter().zip(native).all(|(a, n)| (a.is_nan() && f32::from_bits(n).is_nan()) || a.to_bits() == n);
                let mut v = start;
                law.step(&mut v, anim, seed, age, dt, size).unwrap();
                assert!(same(v), "case {cases} seed {seed:#x} age {:#x} dt {:#x}: {:?} vs native {:?}",
                    r[1], r[2], v.map(f32::to_bits), native);
                let mut m = start;
                mutant_step(&law, &mut m, anim, seed, age, dt, Mutant::None);
                assert!(same(m));
                for (arm, count) in arms.iter().zip(red.iter_mut()) {
                    let mut m = start;
                    mutant_step(&law, &mut m, anim, seed, age, dt, *arm);
                    *count += usize::from(!same(m));
                }
                rows += 1;
            }
        }
        let tally: Vec<String> = arms.iter().zip(&red).map(|(a, n)| format!("{a:?} {n}")).collect();
        println!("clamp magnitude: cases {cases}, refused {refused}, two constants {two}, baked {baked}, rows {rows}, mismatches 0; mutants red: {}",
            tally.join(", "));
        assert!(two > 0 && baked > 0);
        assert!(red.iter().all(|n| *n > 0), "a mutant arm stayed green: {tally:?}");
    }
}
