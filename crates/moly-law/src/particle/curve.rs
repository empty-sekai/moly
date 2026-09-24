//! Prepared particle curve evaluation, independent of module-specific random
//! streams, spatial transforms and integration state.
//!
//! Every module of the particle law and of the weather particle runtime that
//! evaluates a MinMaxCurve follows the engine's MinMaxCurve dispatch through
//! [`CurveSampler`] (limit velocity admits only the polynomial form, as a
//! [`BakedCurve`]). A curve lane the asset reader marks optimized
//! (`MinMaxCurve::BuildCurves`, see [`engine_optimizes_curve`]) is evaluated
//! as the two-segment polynomial that
//! `OptimizedPolynomialCurve::BuildOptimizedCurve` builds ([`BakedCurve`]).
//! Every other lane is evaluated as `AnimationCurveTpl::Evaluate` computes it
//! ([`EngineCurve`]) and multiplied by the curve multiplier afterwards.
//!
//! The emoticon host builds its rotation and limit velocity modules from the
//! particle law, so they follow these forms, but its own start, size,
//! velocity, texture sheet and emission evaluations still call
//! `MinMaxCurve::evaluate`, the documented Hermite form. The environment
//! mixers evaluate through `Curve::evaluate`, the same form.
use crate::particle::value::{bezier_interpolate, step_value, Curve, CurveKey, MinMaxCurve};

/// agePercent（0..100）到曲线时刻的归一化：t = max(x·0.01, 0)。
/// 非数输入落 0（SSE `maxps` 的非数路返回源操作数 0）。The JP ARM module
/// bodies read so far use `fmax`, which keeps a NaN age NaN
/// ([`curve_time_fmax`]); the other over-lifetime modules use this form, and
/// their NaN case was not read.
pub fn normalized_age(age_percent: f32) -> f32 {
    (age_percent * f32::from_bits(0x3c23_d70a)).max(0.0)
}

/// The particle job time `fmax(agePercent * 0.01, +0)` as the current ARM
/// module bodies compute it: a NaN age stays NaN and a negative zero becomes
/// positive zero. [`normalized_age`] turns a NaN into zero instead; this form
/// is for the modules whose body was read with `fmax` (the Noise job and the
/// ClampVelocityModule magnitude loop).
pub fn curve_time_fmax(age_percent: f32) -> f32 {
    let t = age_percent * f32::from_bits(0x3c23_d70a);
    if t.is_nan() || t > 0.0 { t } else { 0.0 }
}

/// ARM `fmax` with default NaN disabled: a NaN operand propagates quieted,
/// and the maximum of -0 and +0 is +0.
pub(crate) fn arm_fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return f32::from_bits(a.to_bits() | 0x0040_0000);
    }
    if b.is_nan() {
        return f32::from_bits(b.to_bits() | 0x0040_0000);
    }
    if a == b {
        return f32::from_bits(a.to_bits() & b.to_bits());
    }
    if a > b { a } else { b }
}

/// ARM `fmin` with default NaN disabled: a NaN operand propagates quieted,
/// and the minimum of -0 and +0 is -0.
pub(crate) fn arm_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return f32::from_bits(a.to_bits() | 0x0040_0000);
    }
    if b.is_nan() {
        return f32::from_bits(b.to_bits() | 0x0040_0000);
    }
    if a == b {
        return f32::from_bits(a.to_bits() | b.to_bits());
    }
    if a < b { a } else { b }
}

/// The unweighted branch of `InterpolateKeyframe` (Hermite basis), with the
/// weighted branch delegated to `value::bezier_interpolate`.
///
/// This is not how the engine evaluates a curve lane in range: the module
/// evaluators reach `AnimationCurveTpl::Evaluate`, which evaluates the cached
/// cubic of `CalculateCacheData` ([`EngineCurve`]); the two forms round
/// differently. `Evaluate` calls `InterpolateKeyframe` only under the
/// ping-pong wrap, for wrap values other than clamp, repeat and ping-pong, and
/// for weighted segments, and [`CurveSampler`] refuses all three.
///
/// 次序逐指令对齐：h01·v1 + ((h00·v0 + h10·m0) + h11·m1)，
/// m0 = 出切·d、m1 = d·入切、d == 0（及非数 d——比较 Z 置位同走
/// 退化路 s=m0=m1=0，恰得 v0）恰好取 v0。段查找左闭右开：最后一
/// 个 time <= t 的键。
pub fn eval_curve(keys: &[CurveKey], t: f32) -> f32 {
    match keys.len() {
        0 => return 0.0,
        1 => return keys[0].value,
        _ => {}
    }
    let last = keys.len() - 1;
    if t >= keys[last].time {
        return keys[last].value;
    }
    if t < keys[0].time {
        return keys[0].value;
    }
    let mut left = last - 1;
    while keys[left].time > t {
        left -= 1;
    }
    let (k0, k1) = (keys[left], keys[left + 1]);
    let d = k1.time - k0.time;
    // 原式 `fcmp d,#0.0; b.eq`：非数比较 Z 置位、同样跳退化路
    // （s=m0=m1=0，按基底算出恰为 v0）——位级等价形是
    // !(d < 0.0 || d > 0.0)。
    if !(d < 0.0 || d > 0.0) {
        return k0.value;
    }
    if let Some(value) = step_value(k0, k1) { return value; }
    if k0.weighted_mode & 2 != 0 || k1.weighted_mode & 1 != 0 {
        return bezier_interpolate(t, k0, k1);
    }
    let s = (t - k0.time) / d;
    let m0 = k0.out_slope * d;
    let m1 = d * k1.in_slope;
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = (s3 + s3) - 3.0 * s2 + 1.0;
    let h10 = (s3 - (s2 + s2)) + s;
    let h11 = s3 - s2;
    let h01 = 3.0 * s2 - (s3 + s3);
    h01 * k1.value + ((h00 * k0.value + h10 * m0) + h11 * m1)
}

/// The asset reader's `isOptimizedCurve` decision for one curve lane
/// (`IsValidOptimizedPolynomialCurve`, called by `MinMaxCurve::BuildCurves`
/// while the curve is read; the bit is not serialized).
///
/// More than three keys is false and fewer than two is true. With two or
/// three keys the stepped-tangent test looks only at the last pair, and only
/// when their values differ by more than 1e-9 (an unordered difference skips
/// it): an infinite out tangent on the second-to-last key or an infinite in
/// tangent on the last key is false. Any key with a nonzero weighted mode is
/// false. The first key time must lie within 1e-4 of zero and the last within
/// 1e-4 of one; a NaN time fails both.
pub fn engine_optimizes_curve(keys: &[CurveKey]) -> bool {
    let n = keys.len();
    if n > 3 {
        return false;
    }
    if n < 2 {
        return true;
    }
    let (a, b) = (keys[n - 2], keys[n - 1]);
    if (a.value - b.value).abs() > f32::from_bits(0x3089_705f) {
        let (out, input) = (a.out_slope, b.in_slope);
        if out == f32::INFINITY || input == f32::NEG_INFINITY
            || out == f32::NEG_INFINITY || input == f32::INFINITY
        {
            return false;
        }
    }
    if keys.iter().any(|k| k.weighted_mode != 0) {
        return false;
    }
    let tolerance = f32::from_bits(0x38d1_b717);
    if !(keys[0].time.abs() <= tolerance) {
        return false;
    }
    (keys[n - 1].time + -1.0).abs() <= tolerance
}

/// `AnimationCurveTpl::CalculateCacheData` coefficients `[c0, c1, c2, c3]` of
/// the segment from `l` to `r`, operation for operation. The segment width is
/// floored at 1e-4 with the NaN-propagating `fmax`. An infinite tangent makes
/// the segment a step: positive infinity on the left out tangent or the right
/// in tangent holds the left value; otherwise negative infinity on either
/// holds the right value. The time offset of a repeat wrap only moves the
/// segment start, never the coefficients.
pub(crate) fn cache_coefficients(l: CurveKey, r: CurveKey) -> [f32; 4] {
    let (m1, m2) = (l.out_slope, r.in_slope);
    if m1 == f32::INFINITY || m2 == f32::INFINITY {
        return [0.0, 0.0, 0.0, l.value];
    }
    if m1 == f32::NEG_INFINITY || m2 == f32::NEG_INFINITY {
        return [0.0, 0.0, 0.0, r.value];
    }
    let dx = arm_fmax(r.time - l.time, f32::from_bits(0x38d1_b717));
    let dy = r.value - l.value;
    let inverse = 1.0 / dx;
    let length1 = m1 * dx;
    let length2 = dx * m2;
    let twice = dy + dy;
    let s1 = ((dy + twice) - length1) - length1;
    let s5 = (length1 + length2) - dy;
    let inverse_squared = inverse * inverse;
    let s4 = s5 - dy;
    let s0 = s1 - length2;
    [inverse * (inverse_squared * s4), inverse_squared * s0, m1, l.value]
}

/// The cached cubic `((dt*c0 + c1)*dt + c2)*dt + c3`, in this nesting.
fn cubic(c: [f32; 4], dt: f32) -> f32 {
    ((dt * c[0] + c[1]) * dt + c[2]) * dt + c[3]
}

// ---- The optimized polynomial ----

/// The optimized curve lane as `OptimizedPolynomialCurve::BuildOptimizedCurve`
/// builds it and `EvaluateThreaded` evaluates it.
///
/// Coefficients are stored highest power first, `[c0, c1, c2, c3]` in the
/// `CalculateCacheData` order, each already multiplied by the curve
/// multiplier; the evaluation applies no multiplier. Segment `a` is evaluated
/// at t itself (the first key may lie up to 1e-4 away from zero), segment `b`
/// at `t - switch`, and `b` is taken where `min(t, 0.99999) >= switch`.
#[derive(Clone, Copy, Debug)]
pub struct BakedCurve {
    a: [f32; 4],
    b: [f32; 4],
    switch: f32,
}

#[cfg(test)]
mod native_cache_tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
        value.get(key).expect(key)
    }
    fn number(value: &Value, key: &str) -> f32 {
        field(value, key).as_f64().expect(key) as f32
    }
    fn bits(value: &Value, key: &str) -> u32 {
        u32::from_str_radix(field(value, key).as_str().expect(key), 16).unwrap()
    }
    fn replay(variable: &str, expected_cases: usize, expected_samples: usize) {
        let path = std::env::var_os(variable).expect(variable);
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(field(&receipt, "inputsUnchanged").as_bool(), Some(true));
        assert_eq!(field(&receipt, "inputsBefore"), field(&receipt, "inputsAfter"));
        let cases = field(&receipt, "curves").as_array().unwrap();
        assert_eq!(cases.len(), expected_cases);
        let mut sample_count = 0;
        for case in cases {
            let name = field(case, "name").as_str().unwrap();
            assert_eq!(number(case, "nativeBuildReturned"), 1.0);
            assert_eq!(field(case, "nativeKeysUnchanged").as_bool(), Some(true));
            let source = field(case, "sourceCurve");
            let keys = field(source, "keys").as_array().unwrap().iter().map(|key| CurveKey {
                time: number(key, "time"), value: number(key, "value"),
                in_slope: number(key, "inSlope"), out_slope: number(key, "outSlope"),
                weighted_mode: number(key, "weightedMode") as u8,
                in_weight: number(key, "inWeight"), out_weight: number(key, "outWeight"),
            }).collect::<Vec<_>>();
            let multiplier = number(source, "multiplier");
            let baked = BakedCurve::bake(&keys, multiplier).unwrap();
            let native_cache = field(case, "cache").as_array().unwrap();
            let actual_cache = baked.a.into_iter().chain(baked.b).chain([baked.switch]);
            assert_eq!(native_cache.len(), 9);
            for (index, (actual, native)) in actual_cache.zip(native_cache).enumerate() {
                assert_eq!(actual.to_bits(), bits(native, "nativeBits"), "{name} cache[{index}]");
            }
            let sampler = CurveSampler::new(&MinMaxCurve::Curve {
                multiplier,
                max: Curve { multiplier: 1.0, keys, pre_wrap: None, post_wrap: None },
            }, CurveTime::Normalized).unwrap();
            assert!(matches!(sampler, CurveSampler::CurveBaked(_)));
            for sample in field(case, "samples").as_array().unwrap() {
                let t = normalized_age(number(sample, "agePercent"));
                assert_eq!(t.to_bits(), bits(sample, "timeBits"), "{name} age conversion");
                let expected = bits(sample, "simdBits");
                assert_eq!(baked.evaluate(t).to_bits(), expected, "{name} baked at {t}");
                assert_eq!(sampler.evaluate(t, 0.0).to_bits(), expected, "{name} sampler at {t}");
                sample_count += 1;
            }
        }
        assert_eq!(sample_count, expected_samples);
    }

    // Source three-key input, current native BuildCurves cache and native SIMD
    // samples: both endpoints, both segments and the split's neighboring f32s.
    #[test]
    #[ignore = "MOLY_SNOW_CURVE_CACHE_NATIVE identifies the current JP native receipt"]
    fn source_three_key_native_cache_and_samples_match_bits() {
        replay("MOLY_SNOW_CURVE_CACHE_NATIVE", 2, 24);
    }

    // Explicitly derived inputs, not additional source qualification: nonunit
    // positive/negative multipliers and positive segments shorter than 1e-4.
    #[test]
    #[ignore = "MOLY_SNOW_CURVE_CACHE_BOUNDARY_NATIVE identifies the native boundary receipt"]
    fn derived_multiplier_and_short_segment_native_boundaries_match_bits() {
        replay("MOLY_SNOW_CURVE_CACHE_BOUNDARY_NATIVE", 4, 48);
    }
}

impl BakedCurve {
    /// The polynomial for a lane the reader optimizes, `None` otherwise (the
    /// lane then goes through [`EngineCurve`]).
    pub fn bake(keys: &[CurveKey], multiplier: f32) -> Option<Self> {
        engine_optimizes_curve(keys).then(|| Self::build(keys, multiplier))
    }

    /// `BuildOptimizedCurve` after the validity test. No key: both segments
    /// are +0 and the multiplier is not applied. One key: both segments hold
    /// the value times the multiplier, returned before the common scaling.
    /// Two keys: `b` repeats `a` and the switch is 1.0, not the last key time.
    /// Three keys: `b` is the second segment and the switch is the middle key
    /// time. Then every coefficient of both segments is multiplied by the
    /// multiplier, step segments included.
    fn build(keys: &[CurveKey], multiplier: f32) -> Self {
        let scale = |c: [f32; 4]| c.map(|v| v * multiplier);
        match keys {
            [] => Self { a: [0.0; 4], b: [0.0; 4], switch: 1.0 },
            [only] => {
                let a = [0.0, 0.0, 0.0, only.value * multiplier];
                Self { a, b: a, switch: 1.0 }
            }
            [k0, k1] => {
                let a = scale(cache_coefficients(*k0, *k1));
                Self { a, b: a, switch: 1.0 }
            }
            [k0, k1, k2, ..] => Self {
                a: scale(cache_coefficients(*k0, *k1)),
                b: scale(cache_coefficients(*k1, *k2)),
                switch: k1.time,
            },
        }
    }

    /// `a3 + t*(a2 + t*(a1 + t*a0))`, or the same for `b` at `t - switch`
    /// where `fmin(t, 0.99999) >= switch`.
    pub fn evaluate(&self, t: f32) -> f32 {
        if self.switch <= arm_fmin(t, f32::from_bits(0x3f7f_ff58)) {
            let tau = t - self.switch;
            let c = &self.b;
            ((c[0] * tau + c[1]) * tau + c[2]) * tau + c[3]
        } else {
            let c = &self.a;
            ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
        }
    }
}

// ---- AnimationCurveTpl::Evaluate ----

/// The clock a consumer feeds its curve with, which decides how far a repeat
/// wrap past the last key is admitted (see [`EngineCurve::new`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveTime {
    /// A normalized time in [+0, 1 + 2 ulps] (or NaN or +inf): the particle
    /// age percent times 0.01 through a maximum with +0, where 100 percent and
    /// the dead-particle marker land at most two ulps above 1, and the legacy
    /// step's cycle time clamped to [0, 1].
    Normalized,
    /// A nonnegative time without an upper bound, such as the system time over
    /// the duration that InitialModule gravity reads.
    Unbounded,
}

/// The upper end of [`CurveTime::Normalized`]: 1 + 2 ulps.
const NORMALIZED_TIME_MAX: u32 = 0x3f80_0002;

/// One curve lane the reader leaves unoptimized, as
/// `AnimationCurveTpl::Evaluate` computes it from an empty cache.
///
/// One key returns its value and no key returns 0, before any other work. With
/// two or more keys, in native branch order:
/// - from the last key on (`last <= t`), the post wrap: clamp evaluates the
///   cubic `(0, 0, 0, last value)` at `t - last`; repeat wraps t into
///   `[first, last]`, clamps it there, and evaluates the segment of the last
///   key time not above the wrapped time, its start moved by `t - wrapped`;
/// - before the first key, the pre wrap: clamp evaluates `(0, 0, 0, first
///   value)` at `t - (t + -1000)`; repeat wraps the same way without the two
///   clamps;
/// - otherwise, and for a NaN t, the segment of the last key time not above t,
///   evaluated at `t - (key time + 0)`.
///
/// Every segment is the cubic of [`cache_coefficients`]. The evaluator keeps a
/// one-segment cache (the job-local cache of `EvaluateThreaded`, shared by the
/// four lanes of a call, or the curve object's own cache); a later time inside
/// the cached window reuses it. This type evaluates as from an empty cache;
/// [`EngineCurve::new`] admits a lane only where the cache history cannot
/// change a result on the consumer's clock.
#[derive(Clone, Debug)]
pub struct EngineCurve {
    keys: Vec<CurveKey>,
    /// Per key index i, the coefficients of the segment from key i to key
    /// `min(i + 1, n - 1)`; the last one is the degenerate segment a repeat wrap
    /// clamped onto the last key uses.
    segments: Vec<[f32; 4]>,
    pre_repeat: bool,
    post_repeat: bool,
}

impl EngineCurve {
    /// Admits the lane for a consumer whose clock is `time`.
    ///
    /// The arithmetic domain ([`EngineCurve::fresh`]) comes first. Then the
    /// cache: a clamp wrap and a repeat wrap before a first key at time zero
    /// cannot change a result for any t at or above +0 (every window a fresh
    /// evaluation writes holds only times whose own fresh evaluation takes the
    /// same segment with the same start). A repeat wrap past the last key can:
    /// a window's ends are rounded sums, and the period index can round up, so
    /// a window may straddle a wrapped knot or a period boundary. It is
    /// admitted on the normalized clock only when every such window is
    /// consistent by value ([`EngineCurve::post_repeat_certificate`]), and
    /// refused on an unbounded clock.
    pub fn new(curve: &Curve, time: CurveTime) -> Result<Self, &'static str> {
        let engine = Self::fresh(curve)?;
        if engine.post_repeat {
            match time {
                CurveTime::Normalized => engine.post_repeat_certificate()?,
                CurveTime::Unbounded => return Err(
                    "curve repeat wrap past the last key on an unbounded clock: the evaluator cache history can change the result"),
            }
        }
        Ok(engine)
    }

    /// The arithmetic domain, without the cache question. Two or more keys
    /// must be unweighted (a weighted segment takes the Bezier branch, not
    /// transcribed), have finite times and values, strictly increasing times
    /// and no NaN tangent (an infinite one is a step). Each wrap must be
    /// exported and be clamp (2), or repeat (1) with the first key at time
    /// zero of either sign (a nonzero first key makes the wrapped offset
    /// inexact, and the result then depends on the cache history). Ping-pong
    /// and other wrap values reach `InterpolateKeyframe`, and a wrapped time
    /// that rounds below the first key reads before the key array; both stay
    /// refused.
    pub(crate) fn fresh(curve: &Curve) -> Result<Self, &'static str> {
        let keys = &curve.keys;
        let n = keys.len();
        let (mut pre_repeat, mut post_repeat) = (false, false);
        if n >= 2 {
            if keys.iter().any(|k| k.weighted_mode != 0) {
                return Err("curve lane with a weighted key: the evaluator's Bezier branch is not transcribed");
            }
            if keys.iter().any(|k| !k.time.is_finite() || !k.value.is_finite()) {
                return Err("curve lane with a nonfinite key time or value");
            }
            if keys.iter().any(|k| k.in_slope.is_nan() || k.out_slope.is_nan()) {
                return Err("curve lane with a NaN tangent");
            }
            if keys.windows(2).any(|w| !(w[0].time < w[1].time)) {
                return Err("curve lane key times not strictly increasing");
            }
            let repeat = |wrap: Option<u32>| match wrap {
                None => Err("curve lane wrap mode not exported"),
                Some(2) => Ok(false),
                Some(1) => Ok(true),
                Some(0) => Err("curve ping-pong wrap: the evaluator's InterpolateKeyframe path is not transcribed"),
                Some(_) => Err("curve wrap mode other than clamp, repeat or ping-pong"),
            };
            pre_repeat = repeat(curve.pre_wrap)?;
            post_repeat = repeat(curve.post_wrap)?;
            if (pre_repeat || post_repeat) && keys[0].time != 0.0 {
                return Err("curve repeat wrap with a first key time other than zero");
            }
        }
        let segments = (0..n).map(|i| cache_coefficients(keys[i], keys[(i + 1).min(n - 1)])).collect();
        Ok(Self { keys: keys.clone(), segments, pre_repeat, post_repeat })
    }

    /// The last key index whose time is not above `x`, if any.
    fn last_at_or_below(&self, x: f32) -> Option<usize> {
        self.keys.partition_point(|k| k.time <= x).checked_sub(1)
    }

    /// The repeat wrap of t: the segment index and its moved start. `clamp`
    /// is true past the last key. `None` where the wrapped time lies below the
    /// first key (the engine reads before the key array there).
    fn repeat(&self, t: f32, clamp: bool) -> Option<(usize, f32)> {
        let first = self.keys[0].time;
        let last = self.keys[self.keys.len() - 1].time;
        let range = last - first;
        let shifted = t - first;
        let period = (shifted / range).floor();
        let wrapped = shifted - range * period;
        let mut tw = wrapped + first;
        if clamp {
            tw = if tw > first { tw } else { first };
            tw = if tw < last { tw } else { last };
        }
        let lhs = self.last_at_or_below(tw)?;
        Some((lhs, self.keys[lhs].time + (t - tw)))
    }

    /// The value from an empty cache.
    pub fn evaluate(&self, t: f32) -> f32 {
        let n = self.keys.len();
        match n {
            0 => return 0.0,
            1 => return self.keys[0].value,
            _ => {}
        }
        if t.is_nan() {
            return t;
        }
        let (first, last) = (self.keys[0], self.keys[n - 1]);
        let (coefficients, start) = if last.time <= t {
            if !self.post_repeat {
                return cubic([0.0, 0.0, 0.0, last.value], t - last.time);
            }
            match self.repeat(t, true) {
                Some((lhs, start)) => (self.segments[lhs], start),
                None => return f32::NAN,
            }
        } else if t < first.time {
            if !self.pre_repeat {
                return cubic([0.0, 0.0, 0.0, first.value], t - (t + -1000.0));
            }
            match self.repeat(t, false) {
                Some((lhs, start)) => (self.segments[lhs], start),
                None => return f32::NAN,
            }
        } else {
            let lhs = self.last_at_or_below(t).unwrap_or(0);
            (self.segments[lhs], self.keys[lhs].time + 0.0)
        };
        cubic(coefficients, t - start)
    }

    /// The post-repeat evaluation at the time whose bits are `word`, which
    /// lies at or past the last key: segment index, window start and end bits
    /// as the evaluator caches them, and the value.
    fn post_repeat_key(&self, word: u32) -> Option<(usize, u32, u32, f32)> {
        let t = f32::from_bits(word);
        let n = self.keys.len();
        let first = self.keys[0].time;
        let last = self.keys[n - 1].time;
        let range = last - first;
        let shifted = t - first;
        let period = (shifted / range).floor();
        let wrapped = shifted - range * period;
        let mut tw = wrapped + first;
        tw = if tw > first { tw } else { first };
        tw = if tw < last { tw } else { last };
        let lhs = self.last_at_or_below(tw)?;
        let rhs = (lhs + 1).min(n - 1);
        let offset = t - tw;
        let start = self.keys[lhs].time + offset;
        let end = self.keys[rhs].time + offset;
        Some((lhs, start.to_bits(), end.to_bits(), cubic(self.segments[lhs], t - start)))
    }

    /// Proves that the evaluator cache cannot change a result of this repeat
    /// wrap for any t in [last key, 1 + 2 ulps], or refuses.
    ///
    /// A fresh evaluation at t writes the window `[time[lhs] + o, time[rhs] +
    /// o)` with `o = t - wrapped`; a later t' inside it returns the window
    /// segment's cubic at `t' - window start`. So every window must hold only
    /// times (inside the domain) whose own fresh value equals that cubic. The
    /// domain is split into runs of equal fresh (segment, start, end): the
    /// period index `floor((t - first) / range)` is nondecreasing in t, and
    /// inside one period the clamped wrapped time is nondecreasing too, so
    /// runs are found by binary search over the float bits. Inside a period,
    /// where the unclamped wrapped time is positive, `t - p` is exact (p the
    /// period start, which lies within a factor of two of t) and so the
    /// offset equals p for the whole run. Times where the wrapped time clamps
    /// to the first key (the period index rounded up) or to the last key are
    /// single runs. Each window is then checked against every other run it
    /// covers, by value; beyond a few thousand such times it refuses.
    fn post_repeat_certificate(&self) -> Result<(), &'static str> {
        const REFUSED: &str = "curve repeat wrap past the last key: the evaluator cache history can change the result";
        const TOO_MANY: &str = "curve repeat wrap past the last key: evaluator cache windows beyond the certified subset";
        let n = self.keys.len();
        let first = self.keys[0].time;
        let last = self.keys[n - 1].time;
        let range = last - first;
        let (lo, hi) = (last.to_bits(), NORMALIZED_TIME_MAX);
        if !(last > 0.0) {
            return Err(TOO_MANY);
        }
        if lo > hi {
            return Ok(());
        }
        let period_of = |word: u32| ((f32::from_bits(word) - first) / range).floor();
        let unclamped = |word: u32, p: f32| (f32::from_bits(word) - first - p) + first;
        let clamped = |word: u32, p: f32| {
            let tw = unclamped(word, p);
            let tw = if tw > first { tw } else { first };
            if tw < last { tw } else { last }
        };
        // Smallest word in [lo, hi) where the monotone predicate holds.
        fn first_true(mut lo: u32, mut hi: u32, predicate: impl Fn(u32) -> bool) -> u32 {
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if predicate(mid) { hi = mid } else { lo = mid + 1 }
            }
            lo
        }
        let (q_lo, q_hi) = (period_of(lo), period_of(hi));
        if !(q_lo >= 1.0 && q_hi.is_finite() && q_hi - q_lo < 64.0) {
            return Err(TOO_MANY);
        }
        // Complete partition of [lo, hi]: (first word, last word, segment, start, end).
        let mut runs: Vec<(u32, u32, usize, u32, u32)> = Vec::new();
        let end_word = hi + 1;
        let mut q = q_lo;
        while q <= q_hi {
            let a = first_true(lo, end_word, |w| period_of(w) >= q);
            let b = first_true(lo, end_word, |w| period_of(w) >= q + 1.0);
            let p = range * q;
            q += 1.0;
            if a >= b {
                continue;
            }
            let c = first_true(a, b, |w| unclamped(w, p) > first);
            if c - a > 16 {
                return Err(TOO_MANY);
            }
            for word in a..c {
                let (lhs, start, end, _) = self.post_repeat_key(word).ok_or(TOO_MANY)?;
                runs.push((word, word, lhs, start, end));
            }
            let mut bounds = Vec::with_capacity(n + 1);
            bounds.push(c);
            for key in &self.keys[1..] {
                bounds.push(first_true(c, b, |w| clamped(w, p) >= key.time));
            }
            bounds.push(b);
            for i in 0..n {
                let (r0, r1) = (bounds[i], bounds[i + 1]);
                if r0 >= r1 {
                    continue;
                }
                if i == n - 1 {
                    // Clamped onto the last key: each writes an empty window.
                    if r1 - r0 > 16 {
                        return Err(TOO_MANY);
                    }
                    for word in r0..r1 {
                        let (lhs, start, end, _) = self.post_repeat_key(word).ok_or(TOO_MANY)?;
                        runs.push((word, word, lhs, start, end));
                    }
                    continue;
                }
                let head = self.post_repeat_key(r0).ok_or(TOO_MANY)?;
                let tail = self.post_repeat_key(r1 - 1).ok_or(TOO_MANY)?;
                if head.0 != i || (head.0, head.1, head.2) != (tail.0, tail.1, tail.2) {
                    return Err(TOO_MANY);
                }
                runs.push((r0, r1 - 1, head.0, head.1, head.2));
            }
        }
        let mut cover = lo;
        for &(r0, r1, ..) in &runs {
            if r0 != cover {
                return Err(TOO_MANY);
            }
            cover = r1 + 1;
        }
        if cover != end_word {
            return Err(TOO_MANY);
        }
        // Runs are sorted and contiguous, so each window meets a contiguous
        // slice of them.
        let mut budget: u32 = 4096;
        for &(_, _, lhs, start, end) in &runs {
            let (a, b) = (start.max(lo), end.min(end_word));
            if b <= a {
                continue;
            }
            let first_run = runs.partition_point(|run| run.1 < a);
            for &(r0, r1, other, other_start, _) in runs[first_run..].iter().take_while(|run| run.0 < b) {
                let (from, to) = (a.max(r0), b.min(r1 + 1));
                if to <= from || (other, other_start) == (lhs, start) {
                    continue;
                }
                budget = budget.checked_sub(to - from).ok_or(TOO_MANY)?;
                for word in from..to {
                    let (_, _, _, fresh) = self.post_repeat_key(word).ok_or(TOO_MANY)?;
                    let cached = cubic(self.segments[lhs], f32::from_bits(word) - f32::from_bits(start));
                    if !(cached.to_bits() == fresh.to_bits() || (cached.is_nan() && fresh.is_nan())) {
                        return Err(REFUSED);
                    }
                }
            }
        }
        Ok(())
    }
}

// ---- The MinMaxCurve dispatch ----

/// One prepared scalar curve, shared across over-lifetime modules.
#[derive(Clone, Debug)]
pub enum CurveSampler {
    /// 模式 0：常数。
    Constant(f32),
    /// Mode 3: `min + random * (max - min)`, the random factor unclamped.
    TwoConstants { min: f32, max: f32 },
    /// Mode 1 on the polynomial: the coefficients carry the multiplier.
    CurveBaked(BakedCurve),
    /// Mode 2 with both lanes on the polynomial.
    TwoCurvesBaked { min: BakedCurve, max: BakedCurve },
    /// Mode 1 through `AnimationCurveTpl::Evaluate`, times the multiplier.
    CurveEngine { multiplier: f32, curve: EngineCurve },
    /// Mode 2 through `AnimationCurveTpl::Evaluate`: each lane times the
    /// multiplier, then `lo + random * (hi - lo)`.
    TwoCurvesEngine { multiplier: f32, min: EngineCurve, max: EngineCurve },
}

impl CurveSampler {
    /// The engine's MinMaxCurve dispatch for one parameter: a mode 1 or 2
    /// curve takes the polynomial where the reader sets its isOptimizedCurve
    /// bit (for mode 2 only when both lanes pass) and `Evaluate` otherwise.
    pub fn new(curve: &MinMaxCurve, time: CurveTime) -> Result<Self, &'static str> {
        Self::build(curve, Some(time), true)
    }

    /// Three axes a module body dispatches together: the polynomial only when
    /// every axis's curve is optimized, otherwise every curve axis goes
    /// through `Evaluate` (a constant axis stays constant).
    pub fn group(curves: [&MinMaxCurve; 3], time: CurveTime) -> Result<[Self; 3], &'static str> {
        let optimized = curves.iter().all(|curve| Self::engine_optimized(curve));
        let [x, y, z] = curves;
        Ok([
            Self::build(x, Some(time), optimized)?,
            Self::build(y, Some(time), optimized)?,
            Self::build(z, Some(time), optimized)?,
        ])
    }

    /// Whether the reader would set the isOptimizedCurve bit (constants
    /// count as optimized for the axis-group decision).
    pub fn engine_optimized(curve: &MinMaxCurve) -> bool {
        match curve {
            MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. } => true,
            MinMaxCurve::Curve { max, .. } => engine_optimizes_curve(&max.keys),
            MinMaxCurve::TwoCurves { min, max, .. } =>
                engine_optimizes_curve(&min.keys) && engine_optimizes_curve(&max.keys),
        }
    }

    /// `time` None skips the cache question: the evaluation as from an empty
    /// cache, for the replay of fresh native rows.
    pub(crate) fn build(curve: &MinMaxCurve, time: Option<CurveTime>, allow_optimized: bool)
        -> Result<Self, &'static str> {
        let engine = |lane: &Curve| match time {
            Some(time) => EngineCurve::new(lane, time),
            None => EngineCurve::fresh(lane),
        };
        Ok(match curve {
            MinMaxCurve::Constant(v) => CurveSampler::Constant(*v),
            MinMaxCurve::TwoConstants { min, max } => CurveSampler::TwoConstants { min: *min, max: *max },
            MinMaxCurve::Curve { multiplier, max } => {
                if allow_optimized && engine_optimizes_curve(&max.keys) {
                    CurveSampler::CurveBaked(BakedCurve::build(&max.keys, *multiplier))
                } else {
                    CurveSampler::CurveEngine { multiplier: *multiplier, curve: engine(max)? }
                }
            }
            MinMaxCurve::TwoCurves { multiplier, min, max } => {
                if allow_optimized && engine_optimizes_curve(&min.keys) && engine_optimizes_curve(&max.keys) {
                    CurveSampler::TwoCurvesBaked {
                        min: BakedCurve::build(&min.keys, *multiplier),
                        max: BakedCurve::build(&max.keys, *multiplier),
                    }
                } else {
                    CurveSampler::TwoCurvesEngine { multiplier: *multiplier, min: engine(min)?, max: engine(max)? }
                }
            }
        })
    }

    /// The random factor is supplied by the owning module's particle-local
    /// stream. It is not shared with the emitter's birth generator.
    pub fn evaluate(&self, t: f32, random: f32) -> f32 {
        match self {
            Self::Constant(value) => *value,
            Self::TwoConstants { min, max } => (max - min) * random + min,
            Self::CurveBaked(curve) => curve.evaluate(t),
            Self::TwoCurvesBaked { min, max } => {
                let lo = min.evaluate(t);
                let hi = max.evaluate(t);
                (hi - lo) * random + lo
            }
            Self::CurveEngine { multiplier, curve } => curve.evaluate(t) * multiplier,
            Self::TwoCurvesEngine { multiplier, min, max } => {
                let lo = min.evaluate(t) * multiplier;
                let hi = max.evaluate(t) * multiplier;
                (hi - lo) * random + lo
            }
        }
    }
}

#[cfg(test)]
mod engine_replay_tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn word(v: &Value) -> u32 {
        v.as_f64().unwrap() as u32
    }
    fn key(words: &Value) -> CurveKey {
        let w: Vec<u32> = words.as_array().unwrap().iter().map(word).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        CurveKey { time: f(0), value: f(1), in_slope: f(2), out_slope: f(3),
            weighted_mode: w[4] as u8, in_weight: f(5), out_weight: f(6) }
    }
    fn lane_side(side: &Value) -> Curve {
        Curve {
            multiplier: 1.0,
            keys: side.get("keys").unwrap().as_array().unwrap().iter().map(key).collect(),
            pre_wrap: Some(word(side.get("pre").unwrap())),
            post_wrap: Some(word(side.get("post").unwrap())),
        }
    }
    fn lane_curve(lane: &Value) -> MinMaxCurve {
        let multiplier = f32::from_bits(word(lane.get("multiplier").unwrap()));
        let max = lane_side(lane.get("max").unwrap());
        match word(lane.get("mode").unwrap()) {
            1 => MinMaxCurve::Curve { multiplier, max },
            2 => MinMaxCurve::TwoCurves { multiplier, min: lane_side(lane.get("min").unwrap()), max },
            mode => panic!("lane mode {mode}"),
        }
    }
    fn same(actual: f32, native: u32) -> bool {
        (actual.is_nan() && f32::from_bits(native).is_nan()) || actual.to_bits() == native
    }

    // Every stored row [t, random, native] of truly fresh native EvaluateThreaded
    // (an empty job-local cache, the sample in all four lanes) over every
    // distinct corpus curve lane and the derived lanes; the dispatch follows
    // the reader's bit. Rows are evaluated from an empty cache, so the cache
    // certificate is not part of this comparison.
    #[test]
    #[ignore = "MOLY_CURVE_EVALUATION_NATIVE must identify the truly fresh native MinMaxCurve evaluation rows"]
    fn curve_evaluation_matches_truly_fresh_native() {
        let path = std::env::var_os("MOLY_CURVE_EVALUATION_NATIVE").expect("MOLY_CURVE_EVALUATION_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        let summary = receipt.get("summary").unwrap();
        assert_eq!(summary.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let mut tally = Vec::new();
        for block in ["corpusCases", "derivedCases"] {
            let (mut lanes, mut refused, mut rows, mut optimized) = (0, 0, 0, 0);
            for (index, case) in receipt.get(block).unwrap().as_array().unwrap().iter().enumerate() {
                let lane = case.get("lane").unwrap();
                let curve = lane_curve(lane);
                lanes += 1;
                let sampler = match CurveSampler::build(&curve, None, true) {
                    Ok(sampler) => sampler,
                    Err(reason) => {
                        assert_ne!(block, "corpusCases", "corpus lane {index} refused: {reason}");
                        refused += 1;
                        continue;
                    }
                };
                let baked = matches!(sampler, CurveSampler::CurveBaked(_) | CurveSampler::TwoCurvesBaked { .. });
                assert_eq!(baked, case.get("optimized").unwrap().as_bool().unwrap(), "{block} {index} decision");
                optimized += usize::from(baked);
                for row in case.get("rows").unwrap().as_array().unwrap() {
                    let row: Vec<u32> = row.as_array().unwrap().iter().map(word).collect();
                    let actual = sampler.evaluate(f32::from_bits(row[0]), f32::from_bits(row[1]));
                    assert!(same(actual, row[2]), "{block} {index} t {:#x} r {:#x}: {:#x} vs native {:#x}",
                        row[0], row[1], actual.to_bits(), row[2]);
                    rows += 1;
                }
            }
            println!("{block}: lanes {lanes}, refused {refused}, optimized {optimized}, rows {rows}, mismatches 0");
            tally.push((lanes, refused, rows));
        }
        assert_eq!((tally[0].0, tally[0].1), (202, 0));
        assert_eq!(tally[1].0, 305);
    }

    // Truly fresh native EvaluateThreaded rows on two-curve lanes that pair a
    // side of zero or one key with a side the reader leaves unoptimized. The
    // isOptimizedCurve bit is then clear, so both sides go through
    // `Evaluate`, which returns the single key's value, or 0 without keys,
    // before any wrap or cache work; each side is then times the multiplier.
    #[test]
    #[ignore = "MOLY_CURVE_SHORT_SIDE_NATIVE must identify the truly fresh native rows of two-curve lanes with a short side"]
    fn two_curve_lane_with_a_short_side_matches_truly_fresh_native() {
        let path = std::env::var_os("MOLY_CURVE_SHORT_SIDE_NATIVE").expect("MOLY_CURVE_SHORT_SIDE_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        let summary = receipt.get("summary").unwrap();
        assert_eq!(summary.get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let (mut lanes, mut rows) = (0, 0);
        for (index, case) in receipt.get("side01Cases").unwrap().as_array().unwrap().iter().enumerate() {
            let curve = lane_curve(case.get("lane").unwrap());
            let MinMaxCurve::TwoCurves { min, max, .. } = &curve else { panic!("lane {index} mode") };
            assert!(min.keys.len().min(max.keys.len()) <= 1 && min.keys.len().max(max.keys.len()) >= 2, "lane {index} shape");
            let sampler = CurveSampler::build(&curve, None, true)
                .unwrap_or_else(|reason| panic!("lane {index} refused: {reason}"));
            assert!(matches!(sampler, CurveSampler::TwoCurvesEngine { .. }), "lane {index} dispatch");
            for row in case.get("rows").unwrap().as_array().unwrap() {
                let row: Vec<u32> = row.as_array().unwrap().iter().map(word).collect();
                let actual = sampler.evaluate(f32::from_bits(row[0]), f32::from_bits(row[1]));
                assert!(same(actual, row[2]), "lane {index} t {:#x} r {:#x}: {:#x} vs native {:#x}",
                    row[0], row[1], actual.to_bits(), row[2]);
                rows += 1;
            }
            lanes += 1;
        }
        println!("short-side lanes {lanes}, rows {rows}, mismatches 0");
        assert_eq!((lanes, rows), (16, 10595));
    }

    // The cache certificate against native carried values. For every corpus
    // repeat lane past its last key: every window a native evaluation wrote and
    // then reused (t1 writes, t2 is read in the same EvaluateThreaded call) is
    // recomputed; the cached cubic at t2 must equal the native carried value
    // and the fresh evaluation the truly fresh native value. The certificate
    // must refuse every lane where native carried differs from fresh, and
    // admit every lane whose inconsistent windows were all confirmed equal.
    #[test]
    #[ignore = "MOLY_CURVE_CACHE_WINDOWS_NATIVE must identify the native cache window confirmations"]
    fn cache_certificate_matches_native_windows() {
        let path = std::env::var_os("MOLY_CURVE_CACHE_WINDOWS_NATIVE").expect("MOLY_CURVE_CACHE_WINDOWS_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        let (mut confirmed, mut admitted, mut refused) = (0, 0, 0);
        for (index, side) in receipt.get("sides").unwrap().as_array().unwrap().iter().enumerate() {
            let curve = Curve {
                multiplier: 1.0,
                keys: side.get("keys").unwrap().as_array().unwrap().iter().map(key).collect(),
                pre_wrap: Some(2),
                post_wrap: Some(1),
            };
            let engine = EngineCurve::fresh(&curve).expect("corpus repeat lane in the arithmetic domain");
            for window in side.get("confirmations").unwrap().as_array().unwrap() {
                let [t1, t2, carried, fresh] = ["t1", "t2", "carried", "trulyFresh"]
                    .map(|name| word(window.get(name).unwrap()));
                let (lhs, start, _, _) = engine.post_repeat_key(t1).unwrap();
                let cached = cubic(engine.segments[lhs], f32::from_bits(t2) - f32::from_bits(start));
                assert!(same(cached, carried), "side {index} t1 {t1:#x} t2 {t2:#x} cached");
                assert!(same(engine.evaluate(f32::from_bits(t2)), fresh), "side {index} t2 {t2:#x} fresh");
                confirmed += 1;
            }
            let verdict = engine.post_repeat_certificate();
            match side.get("expect").unwrap().as_str().unwrap() {
                "admit" => { assert_eq!(verdict, Ok(()), "side {index}"); admitted += 1; }
                "refuse" => { assert!(verdict.is_err(), "side {index}"); refused += 1; }
                other => panic!("expectation {other}"),
            }
        }
        println!("confirmed windows {confirmed}, admitted {admitted}, refused {refused}");
        assert_eq!((confirmed, admitted + refused), (16, 14));
    }
}
