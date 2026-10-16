//! The SizeModule's current size as the CollisionModule reads it.
//!
//! The engine writes the per-particle current size at two points, both only
//! for a system whose CollisionModule is enabled (or whose Noise module drives
//! size): in the module batch after the simulation, right after the collision
//! call, over the whole pool; and in each newborn block, right before that
//! block's collision call. So the collision call after the simulation reads
//! the size the previous write left (the previous slice's, or the newborn
//! block's), and a newborn block's collision call reads the size of its own
//! newborns.
//!
//! Per lane: `t = fmax(age * 0.01, +0)` (a NaN age propagates), the size curve
//! at `t` (a constant, two constants blended by the particle's size random,
//! one curve times its multiplier, or two curves blended), and
//! `current = start * fmax(v, +0)`.
//!
//! The asset reader decides once per curve whether the module reads it as an
//! optimised polynomial ([`optimised`]; for two curves both must qualify, the
//! maximum curve being tested first). Such a curve is read only through its
//! two-segment polynomial ([`Polynomial`]): the multiplier is folded into the
//! coefficients when the asset loads, the first segment is evaluated at `t`
//! itself, the second at `t - switch`, and the second is taken where
//! `fmin(t, 0.99999) >= switch`, so past the last key the cubic is extended,
//! not clamped, and the wrap modes are never read. Every other keyed curve is
//! evaluated as the engine's animation curve evaluation does: the clamp
//! wraps outside the key range, the segment found by the sampling search,
//! its cubic coefficients and the nested polynomial, then the multiplier.
//! The module passes no cache, so the evaluation uses the one the curve
//! object carries from lane to lane and call to call (the lanes past the end
//! of the last four-lane group included). A weighted segment (the left key's
//! out-weight bit or the right key's in-weight bit) takes the evaluation's
//! weighted branch ([`crate::particle::curve::interpolate_keyframe`]), which
//! writes only the search hint and leaves the cached window unmatchable. With
//! key times strictly increasing that cache only ever answers a time with the
//! segment the search from index 0 finds (a fresh cache), which is what this
//! law computes; with keys out of order or sharing a time, a cached segment
//! or the cached search hint can answer differently, so such curves are
//! refused on this path, as are separate axes and the 3D size. The law reads
//! the keys only, not the wrap fields of its curve type; the caller checks
//! that the export's are the clamp.
//!
//! The size random (two constants, two curves) is one draw per lane, a pure
//! function of the seed, read by the blend alone.
//!
//! The engine also writes the lanes of the last four-lane group past the end
//! of the range; those slots hold no particle, so only the range is written.

use crate::particle::armf as a;
use crate::particle::random::ParticleRandom;
use crate::particle::schema::SizeOverLifetimeParams;
use crate::particle::value::{Curve, CurveKey, MinMaxCurve};

/// The age to normalized-time factor (0.01).
const AGE_FACTOR: f32 = f32::from_bits(0x3c23_d70a);
/// The salt of the particle's size random.
const SIZE_SALT: u32 = 0x8d2c_8431;
/// The smallest segment width the cubic coefficients divide by, and the
/// tolerance of the optimised-curve endpoint tests (1e-4).
const WIDTH_FLOOR: f32 = f32::from_bits(0x38d1_b717);
/// The value step below which the optimised-curve test skips its slope tests.
const STEP_TOLERANCE: f32 = f32::from_bits(0x3089_705f);
/// The time clamp of the optimised polynomial's segment choice (0.99999).
const SWITCH_CLAMP: f32 = f32::from_bits(0x3f7f_ff58);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Separate axes: one curve per axis.
    SeparateAxes,
    /// The 3D start size: three current-size components.
    Size3d,
    /// Key times not strictly increasing (or NaN) on a curve the module
    /// evaluates key by key: the value then depends on the cache the curve
    /// carries from earlier lanes and calls.
    UnorderedKeys,
}

#[derive(Clone, Debug, PartialEq)]
enum Size {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
    Curve { scalar: f32, keys: Vec<CurveKey> },
    TwoCurves { scalar: f32, min: Vec<CurveKey>, max: Vec<CurveKey> },
    Polynomial(Polynomial),
    TwoPolynomials { min: Polynomial, max: Polynomial },
}

/// The optimised polynomial of one curve as the asset reader builds it:
/// two segments of four coefficients, highest power first, the multiplier
/// already applied, and the time where the second segment starts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Polynomial {
    a: [f32; 4],
    b: [f32; 4],
    switch: f32,
}

impl Polynomial {
    /// Built from keys that pass [`optimised`]. No key: both segments +0 and
    /// the switch 1.0, the multiplier not applied. One key: both segments
    /// hold the value times the multiplier, the switch 1.0. Two keys: the
    /// second segment repeats the first and the switch stays 1.0 (never
    /// reached through the 0.99999 clamp), whatever the last key's time.
    /// Three keys: the second segment from the middle key and the switch at
    /// its time. Each segment takes the cubic coefficients of the key
    /// evaluation (its step tangents and its width floor included), in their
    /// operation order; then every coefficient is multiplied by the
    /// multiplier.
    fn build(keys: &[CurveKey], multiplier: f32) -> Self {
        let scale = |c: [f32; 4]| c.map(|v| a::mul(v, multiplier));
        let coefficients = |lhs: usize| {
            if arms::on("reassociated") {
                return reassociated(keys, lhs);
            }
            segment(keys, lhs, lhs + 1).c
        };
        match keys.len() {
            0 => Self { a: [0.0; 4], b: [0.0; 4], switch: 1.0 },
            1 => {
                let a = [0.0, 0.0, 0.0, a::mul(keys[0].value, multiplier)];
                Self { a, b: a, switch: 1.0 }
            }
            2 => {
                let a = scale(coefficients(0));
                Self { a, b: a, switch: 1.0 }
            }
            _ => Self { a: scale(coefficients(0)), b: scale(coefficients(1)), switch: keys[1].time },
        }
    }

    /// Both segments are evaluated; the second is taken where
    /// `fmin(t, 0.99999) >= switch` (a NaN time takes the first).
    fn evaluate(&self, t: f32) -> f32 {
        let horner = |c: [f32; 4], x: f32| {
            let v = a::add(c[1], a::mul(x, c[0]));
            let v = a::add(c[2], a::mul(x, v));
            a::add(c[3], a::mul(x, v))
        };
        let first = horner(self.a, t);
        let second = horner(self.b, a::sub(t, self.switch));
        if a::min(t, SWITCH_CLAMP) >= self.switch { second } else { first }
    }
}

/// The re-associated coefficients (the numerators combined, then divided by
/// the width's square and cube): a named wrong form for the replay, reached
/// only through its arm.
fn reassociated(keys: &[CurveKey], lhs: usize) -> [f32; 4] {
    let (l, r) = (&keys[lhs], &keys[lhs + 1]);
    let mut c = segment(keys, lhs, lhs + 1).c;
    if c[0] == 0.0 && c[1] == 0.0 && c[2] == 0.0 {
        return c;
    }
    let dx = a::max(a::sub(r.time, l.time), WIDTH_FLOOR);
    let dy = a::sub(r.value, l.value);
    let (len1, len2) = (a::mul(l.out_slope, dx), a::mul(dx, r.in_slope));
    let cubic = a::sub(a::sub(a::add(len1, len2), dy), dy);
    let square = a::sub(a::sub(a::sub(a::add(dy, a::add(dy, dy)), len1), len1), len2);
    c[0] = a::div(cubic, a::mul(a::mul(dx, dx), dx));
    c[1] = a::div(square, a::mul(dx, dx));
    c
}

/// The qualified size law of one system.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentSizeLaw {
    size: Size,
}

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

impl CurrentSizeLaw {
    /// `size_3d` is the system's 3D start size flag.
    pub fn from_params(params: &SizeOverLifetimeParams, size_3d: bool) -> Result<Self, Refused> {
        if params.separate_axes {
            return Err(Refused::SeparateAxes);
        }
        if size_3d {
            return Err(Refused::Size3d);
        }
        let keyed = |curve: &Curve| -> Result<Vec<CurveKey>, Refused> {
            if !curve.keys.windows(2).all(|pair| pair[0].time < pair[1].time) {
                return Err(Refused::UnorderedKeys);
            }
            Ok(curve.keys.clone())
        };
        let polynomial = !arms::on("genericPath");
        let size = match &params.curve {
            MinMaxCurve::Constant(value) => Size::Constant(*value),
            MinMaxCurve::TwoConstants { min, max } => Size::TwoConstants { min: *min, max: *max },
            MinMaxCurve::Curve { multiplier, max } if polynomial && optimised(&max.keys) => {
                Size::Polynomial(Polynomial::build(&max.keys, *multiplier))
            }
            MinMaxCurve::Curve { multiplier, max } => Size::Curve { scalar: *multiplier, keys: keyed(max)? },
            // One decision for the pair: the minimum curve is built only
            // after the maximum curve qualified, and the pair is read as
            // polynomials only when both did.
            MinMaxCurve::TwoCurves { multiplier, min, max }
                if polynomial && optimised(&max.keys) && optimised(&min.keys) =>
            {
                Size::TwoPolynomials {
                    min: Polynomial::build(&min.keys, *multiplier),
                    max: Polynomial::build(&max.keys, *multiplier),
                }
            }
            MinMaxCurve::TwoCurves { multiplier, min, max } => {
                Size::TwoCurves { scalar: *multiplier, min: keyed(min)?, max: keyed(max)? }
            }
        };
        Ok(Self { size })
    }

    /// The current X size of one particle from its start X size, age percent
    /// and seed.
    pub fn current(&self, start: f32, age_percent: f32, seed: u32) -> f32 {
        let scaled = a::mul(age_percent, AGE_FACTOR);
        let t = a::max(scaled, 0.0);
        let salt = if arms::on("otherSalt") { SIZE_SALT.wrapping_add(1) } else { SIZE_SALT };
        let random = || {
            if arms::on("extraDraw") {
                let mut stream = ParticleRandom::from_seed(seed.wrapping_add(salt));
                stream.next_f32();
                return stream.next_f32();
            }
            ParticleRandom::sample(seed, salt)
        };
        let scalar = |value: f32, s: f32| if arms::on("noMultiplier") { value } else { a::mul(value, s) };
        let blend = |low: f32, high: f32| a::add(low, a::mul(random(), a::sub(high, low)));
        let v = match &self.size {
            Size::Constant(value) => *value,
            Size::TwoConstants { min, max } => blend(*min, *max),
            Size::Curve { scalar: s, keys } => scalar(evaluate(keys, t), *s),
            Size::TwoCurves { scalar: s, min, max } => {
                let low = scalar(evaluate(min, t), *s);
                let high = scalar(evaluate(max, t), *s);
                blend(low, high)
            }
            Size::Polynomial(curve) => curve.evaluate(t),
            Size::TwoPolynomials { min, max } => {
                let (min, max) = if arms::on("swapMinMax") { (max, min) } else { (min, max) };
                blend(min.evaluate(t), max.evaluate(t))
            }
        };
        a::mul(start, a::max(v, 0.0))
    }
}

/// Whether the asset reader turns the keys into the optimised polynomial:
/// two or three keys whose last step has finite end tangents (when the step
/// is above the tolerance), no weighted key, the first key at 0 and the last
/// at 1 within 1e-4. At most one key always builds.
pub fn optimised(keys: &[CurveKey]) -> bool {
    let n = keys.len();
    if n > 3 {
        return false;
    }
    if n < 2 {
        return true;
    }
    let (left, right) = (&keys[n - 2], &keys[n - 1]);
    let step = a::abs(a::sub(left.value, right.value));
    if step > STEP_TOLERANCE
        && (left.out_slope == f32::INFINITY
            || right.in_slope == f32::NEG_INFINITY
            || left.out_slope == f32::NEG_INFINITY
            || right.in_slope == f32::INFINITY)
    {
        return false;
    }
    if keys.iter().any(|key| key.weighted_mode != 0) {
        return false;
    }
    if !(a::abs(keys[0].time) <= WIDTH_FLOOR) {
        return false;
    }
    a::abs(a::add(keys[n - 1].time, -1.0)) <= WIDTH_FLOOR
}

/// One segment's time origin and cubic coefficients.
#[derive(Clone, Copy)]
struct Segment {
    time: f32,
    c: [f32; 4],
}

fn cubic(segment: &Segment, t: f32) -> f32 {
    let dt = a::sub(t, segment.time);
    let [c0, c1, c2, c3] = segment.c;
    let s = a::mul(dt, c0);
    let s = a::add(s, c1);
    let s = a::mul(dt, s);
    let s = a::add(c2, s);
    let s = a::mul(dt, s);
    a::add(c3, s)
}

/// The segment's cubic coefficients, in the engine's operation order; an
/// infinite end tangent makes the segment a step to one of its end values.
fn segment(keys: &[CurveKey], lhs: usize, rhs: usize) -> Segment {
    let (l, r) = (&keys[lhs], &keys[rhs]);
    let dx = a::max(a::sub(r.time, l.time), WIDTH_FLOOR);
    let dy = a::sub(r.value, l.value);
    let (m1, m2) = (l.out_slope, r.in_slope);
    let inv = a::div(1.0, dx);
    let len1 = a::mul(m1, dx);
    let len2 = a::mul(dx, m2);
    let dy2 = a::add(dy, dy);
    let s1 = a::add(dy, dy2);
    let s1 = a::sub(s1, len1);
    let s1 = a::sub(s1, len1);
    let s5 = a::add(len1, len2);
    let s5 = a::sub(s5, dy);
    let inv2 = a::mul(inv, inv);
    let s4 = a::sub(s5, dy);
    let s0 = a::sub(s1, len2);
    let c0 = a::mul(inv, a::mul(inv2, s4));
    let c1 = a::mul(inv2, s0);
    let mut c = [c0, c1, m1, l.value];
    if m1 == f32::INFINITY || m2 == f32::INFINITY {
        c = [0.0, 0.0, 0.0, l.value];
    } else if m1 == f32::NEG_INFINITY || m2 == f32::NEG_INFINITY {
        c = [0.0, 0.0, 0.0, r.value];
    }
    Segment { time: a::add(l.time, 0.0), c }
}

/// The sampling search from the cache's index hint, branch for branch (an
/// unordered comparison takes the branch the engine's condition codes take).
/// Called with at least two keys.
fn find_index(keys: &[CurveKey], hint: usize, t: f32) -> (usize, usize) {
    let n = keys.len();
    let time = |i: usize| keys[i].time;
    let clamp_rhs = |i: usize| if i < n { i } else { n - 1 };
    let binary = || {
        let (mut lo, mut count) = (0usize, n as isize);
        loop {
            let half = count >> 1;
            let mid = half as usize + lo;
            count = count - half - 1;
            if time(mid) > t {
                count = half;
            } else {
                lo = mid + 1;
            }
            if count <= 0 {
                break;
            }
        }
        // The engine's index is lo - 1; lo is at least 1 whenever the pre
        // wrap did not take t, and index 0 stands in otherwise.
        (lo.saturating_sub(1), if lo < n - 1 { lo } else { n - 1 })
    };
    let th = time(hint);
    if th < t {
        let next = hint + 1;
        if next < n && !(time(next) <= t || time(next).is_nan() || t.is_nan()) {
            return (hint, clamp_rhs(next));
        }
        let second = hint + 2;
        if second < n && time(second) > t {
            return (hint + 1, clamp_rhs(second));
        }
        let third = hint + 3;
        if third >= n || time(third) <= t || time(third).is_nan() || t.is_nan() {
            return binary();
        }
        return (hint + 2, clamp_rhs(third));
    }
    let mut left = hint;
    if !(th <= t) {
        if hint == 0 {
            return binary();
        }
        left = hint - 1;
        if !(time(left) <= t) {
            if hint < 2 {
                return binary();
            }
            left = hint - 2;
            let tw = time(left);
            if tw > t || tw.is_nan() || t.is_nan() {
                return binary();
            }
        }
    }
    (left, if left + 1 < n { left + 1 } else { n - 1 })
}

/// The engine's animation curve evaluation with a fresh cache (index 0, time
/// +inf, end 0: it never holds t) and the clamp wraps.
fn evaluate(keys: &[CurveKey], t: f32) -> f32 {
    if arms::on("hermite") {
        // The product's Hermite evaluator takes no NaN time.
        return if t.is_nan() { t } else { Curve { multiplier: 1.0, keys: keys.to_vec(), pre_wrap: None, post_wrap: None }.evaluate(t) };
    }
    let n = keys.len();
    if n == 1 {
        return keys[0].value;
    }
    if n == 0 {
        return 0.0;
    }
    let infinite = |v: f32| v.to_bits() & 0x7f80_0000 == 0x7f80_0000;
    if infinite(keys[0].time) || infinite(keys[n - 1].time) {
        return 0.0;
    }
    let (first, last) = (keys[0].time, keys[n - 1].time);
    if last <= t {
        return cubic(&Segment { time: last, c: [0.0, 0.0, 0.0, keys[n - 1].value] }, t);
    }
    if !(first <= t || t.is_nan()) {
        return cubic(&Segment { time: a::add(t, -1000.0), c: [0.0, 0.0, 0.0, keys[0].value] }, t);
    }
    let (lhs, rhs) = find_index(keys, 0, t);
    // The weighted branch (see the module notes); the arm takes the cubic,
    // a named wrong form for the replay.
    if crate::particle::curve::weighted_segment(keys[lhs], keys[rhs]) && !arms::on("weightedAsCubic") {
        return crate::particle::curve::interpolate_keyframe(keys[lhs], keys[rhs], t);
    }
    cubic(&segment(keys, lhs, rhs), t)
}

#[cfg(test)]
mod tests;
