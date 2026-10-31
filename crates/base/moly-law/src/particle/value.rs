//! Evaluation of `MinMaxCurve` and `MinMaxGradient`, the value families every
//! time-varying parameter of the particle modules shares.
//!
//! Input: a value in one of its modes, the normalized time and the caller's
//! lerp factor. Output: the scalar or RGBA value for that time and factor;
//! neither input is clamped on the way in.
//!
//! The four curve modes and the five colour modes follow
//! `MinMaxCurve.Evaluate` and `MinMaxGradient.Evaluate` (`C# readable`, the
//! `Mathf.Lerp` and `Color.Lerp` time clamps included).
//! `AnimationCurve.Evaluate` and `Gradient.Evaluate` behind them are native in
//! the engine; this file's curve and gradient evaluation is a `behavioural
//! spec`, implemented from their documented semantics, with a measurement plan
//! in each function's comment.
//! The particle modules evaluate curves through `curve::CurveSampler`, not
//! through this file's [`Curve::evaluate`].

use crate::particle::value::MinMaxCurve::{Constant, TwoConstants, TwoCurves};

/// One key of an animation curve. When a weighted bit (`weighted_mode` bit0
/// the in weight, bit1 the out weight) is set, the segments beside this key
/// evaluate through a cubic Bezier (see `curve::bezier_interpolate`); an
/// unset weight bit is evaluated as 1/3, and the stored value does not enter
/// the evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveKey {
    /// The key time, normalized to [0,1]: particle module curves always use
    /// normalized time.
    pub time: f32,
    pub value: f32,
    /// The in tangent: the end slope of the segment left of this key.
    pub in_slope: f32,
    /// The out tangent: the start slope of the segment right of this key.
    pub out_slope: f32,
    /// The weighted mode bits (0 unweighted, 1 in weight only, 2 out weight
    /// only, 3 both). Evaluation reads no bit above bit1 (the engine tests
    /// bit0 and bit1 only); the field keeps the value as serialized.
    pub weighted_mode: u8,
    /// The in weight: this key's Bezier control weight as a segment's right
    /// end.
    pub in_weight: f32,
    /// The out weight: this key's Bezier control weight as a segment's left
    /// end.
    pub out_weight: f32,
}

/// A sequence of unweighted keys, given in ascending time. Evaluation does not
/// sort (a key out of order is damaged data; the schema decoder refuses it
/// rather than the evaluator correcting it silently).
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    /// The curve multiplier: `Curve` mode multiplies the evaluated value,
    /// `TwoCurves` mode multiplies the interpolated value; both multiply, each
    /// exactly once (see `MinMaxCurve::evaluate`).
    pub multiplier: f32,
    pub keys: Vec<CurveKey>,
    /// The serialized AnimationCurve wrap before the first key
    /// (`m_PreInfinity`: 0 ping-pong, 1 repeat, 2 clamp), `None` when the
    /// export does not carry it. The engine evaluator reads it only for a lane
    /// the reader leaves unoptimized; `particle::curve::EngineCurve` refuses a
    /// missing one rather than assuming clamp.
    pub pre_wrap: Option<u32>,
    /// The serialized wrap past the last key (`m_PostInfinity`), as above.
    pub post_wrap: Option<u32>,
}

// ---- Weighted segments ----

/// The step override `InterpolateKeyframe` applies after either branch: +inf on
/// `k0`'s out tangent or `k1`'s in tangent gives `k0.value`; otherwise -inf on
/// either gives `k1.value`. The weighted (Bezier) branch itself is
/// `curve::interpolate_keyframe`, which reproduces the engine's body.
pub(crate) fn step_value(k0: CurveKey, k1: CurveKey) -> Option<f32> {
    // The positive-infinity branch returns before the negative check.
    if k0.out_slope == f32::INFINITY || k1.in_slope == f32::INFINITY {
        Some(k0.value)
    } else if k0.out_slope == f32::NEG_INFINITY || k1.in_slope == f32::NEG_INFINITY {
        Some(k1.value)
    } else {
        None
    }
}

impl Curve {
    /// `AnimationCurve.Evaluate(time)`: a `behavioural spec` for the unweighted
    /// branch, `bit-exact` for the weighted branch.
    ///
    /// Unweighted segments are the documented cubic Hermite (each tangent the
    /// key slope times the segment length); before the first key and past the
    /// last the end values hold (clamping); a single key returns its value
    /// everywhere. Two keys at the same time take the right key's value.
    ///
    /// A weighted segment (the left key's out weight bit or the right key's in
    /// weight bit set) takes the engine's Bezier path
    /// ([`crate::particle::curve::interpolate_keyframe`]); weighted segments
    /// have no documented semantics, only the engine form.
    ///
    /// Editor measurement plan: the `AnimationCurve` keys (0,0,outSlope=0) and
    /// (1,1,inSlope=0) evaluate at t=0.25 to the smoothstep value
    /// 5/32 = 0.15625 (a module test uses this anchor); then sample the
    /// asymmetric slopes (out=2,in=-1) at t=0.5 to tell Hermite from linear.
    ///
    /// This is the documented Hermite form, not what the engine computes: in
    /// range `AnimationCurveTpl::Evaluate` evaluates the cached cubic of
    /// `CalculateCacheData`, which rounds differently, and past the ends it
    /// applies the wrap modes instead of always clamping. Particle modules evaluate
    /// through `curve::CurveSampler`.
    pub fn evaluate(&self, time: f32) -> f32 {
        match self.keys.as_slice() {
            [] => 0.0,
            [only] => only.value,
            keys => {
                let first = keys[0];
                if time <= first.time {
                    return first.value;
                }
                let last = keys[keys.len() - 1];
                if time >= last.time {
                    return last.value;
                }
                // The clamps above put `time` inside (first.time, last.time).
                let i = keys
                    .windows(2)
                    .position(|w| w[0].time <= time && time < w[1].time)
                    .expect("time between first and last always falls in a segment");
                let (k0, k1) = (keys[i], keys[i + 1]);
                let dt = k1.time - k0.time;
                if !(dt > 0.0) {
                    // Equal key times: zero-width segment, the right key's
                    // value (the search has already passed the left key).
                    return k1.value;
                }
                if crate::particle::curve::weighted_segment(k0, k1) {
                    return crate::particle::curve::interpolate_keyframe(k0, k1, time);
                }
                let t = (time - k0.time) / dt;
                let m0 = k0.out_slope * dt;
                let m1 = k1.in_slope * dt;
                // The standard Hermite basis: a*v0 + b*m0 + c*v1 + d*m1.
                let t2 = t * t;
                let t3 = t2 * t;
                let a = 2.0 * t3 - 3.0 * t2 + 1.0;
                let b = t3 - 2.0 * t2 + t;
                let c = -2.0 * t3 + 3.0 * t2;
                let d = t3 - t2;
                step_value(k0, k1).unwrap_or(a * k0.value + b * m0 + c * k1.value + d * m1)
            }
        }
    }
}

/// The four modes of a particle parameter. `C# readable`:
/// `ParticleSystemCurveMode` { Constant=0, Curve=1, TwoCurves=2,
/// TwoConstants=3 }, with `evaluate` following `MinMaxCurve.Evaluate`.
#[derive(Debug, Clone, PartialEq)]
pub enum MinMaxCurve {
    /// Serialized as `constant` (stored as `constantMax`).
    Constant(f32),
    /// Serialized as `curve` (stored as `curveMax` times `curveMultiplier`).
    Curve { multiplier: f32, max: Curve },
    /// Serialized as `twoConstants` (`constantMin`/`constantMax`).
    TwoConstants { min: f32, max: f32 },
    /// Serialized as `twoCurves` (`curveMin`/`curveMax`, one shared
    /// multiplier).
    TwoCurves {
        multiplier: f32,
        min: Curve,
        max: Curve,
    },
}

impl MinMaxCurve {
    /// `MinMaxCurve.Evaluate(time, lerpFactor)`, `C# readable`:
    ///
    /// ```text
    /// Constant     -> constantMax
    /// Curve        -> curveMax.Evaluate(time) * multiplier
    /// TwoConstants -> Lerp(constantMin, constantMax, lerpFactor)
    /// TwoCurves    -> Lerp(curveMin.Evaluate(time), curveMax.Evaluate(time),
    ///                lerpFactor) * multiplier
    /// ```
    ///
    /// `Lerp` is `Mathf.Lerp`: `a + (b-a) * Clamp01(t)`. `lerpFactor` is the
    /// caller's explicit random draw (typically `Random.value`, one per
    /// particle, as in the spawn path of `emit`).
    ///
    /// The lanes go through [`Curve::evaluate`], not the engine's curve
    /// dispatch; particle modules use `curve::CurveSampler` instead.
    pub fn evaluate(&self, time: f32, lerp_factor: f32) -> f32 {
        match self {
            Constant(c) => *c,
            MinMaxCurve::Curve { multiplier, max } => max.evaluate(time) * multiplier,
            TwoConstants { min, max } => lerp(*min, *max, lerp_factor),
            TwoCurves { multiplier, min, max } => {
                lerp(min.evaluate(time), max.evaluate(time), lerp_factor) * multiplier
            }
        }
    }
}

/// `Mathf.Lerp`: the lerp factor is clamped to [0,1]. `C# readable`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// `Color.Lerp`: each of the four RGBA components through `Mathf.Lerp`, with
/// the same clamp on t. `C# readable`.
fn color_lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

pub use super::gradient::{Gradient, GradientAlphaKey, GradientColorKey};

#[derive(Debug, Clone, PartialEq)]
pub enum MinMaxGradient {
    /// Serialized as `color` (stored as `colorMax`).
    Color([f32; 4]),
    /// Serialized as `gradient` (`gradientMax`).
    Gradient(Gradient),
    /// Serialized as `twoColors` (`colorMin`/`colorMax`).
    TwoColors { min: [f32; 4], max: [f32; 4] },
    /// Serialized as `twoGradients` (`gradientMin`/`gradientMax`).
    TwoGradients { min: Gradient, max: Gradient },
    /// Serialized as `randomColor`, where the lerp factor is the time:
    /// `gradientMax.Evaluate(lerpFactor)`, not `Evaluate(time)` (`C#
    /// readable`). A test pins this order (0 and 1 are the ends of both
    /// clocks), so a change to it is a deliberate one.
    RandomColor(Gradient),
}

impl MinMaxGradient {
    /// `MinMaxGradient.Evaluate(time, lerpFactor)`, `C# readable`:
    ///
    /// ```text
    /// Color         -> colorMax
    /// TwoColors     -> Color.Lerp(colorMin, colorMax, lerpFactor)
    /// Gradient      -> gradientMax.Evaluate(time)
    /// TwoGradients  -> Color.Lerp(gmin.Evaluate(time), gmax.Evaluate(time), lerpFactor)
    /// RandomColor   -> gradientMax.Evaluate(lerpFactor)
    /// ```
    pub fn evaluate(&self, time: f32, lerp_factor: f32) -> [f32; 4] {
        match self {
            MinMaxGradient::Color(c) => *c,
            MinMaxGradient::TwoColors { min, max } => color_lerp(*min, *max, lerp_factor),
            MinMaxGradient::Gradient(g) => g.evaluate(time),
            MinMaxGradient::TwoGradients { min, max } => {
                color_lerp(min.evaluate(time), max.evaluate(time), lerp_factor)
            }
            MinMaxGradient::RandomColor(g) => g.evaluate(lerp_factor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two smooth keys: range [0,1], zero tangents at both ends
    /// (smoothstep shape).
    fn smoothstep_curve() -> Curve {
        Curve {
            multiplier: 1.0,
            keys: vec![
                CurveKey { time: 0.0, value: 0.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
                CurveKey { time: 1.0, value: 1.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
            ],
            pre_wrap: Some(2),
            post_wrap: Some(2),
        }
    }

    #[test]
    fn hermite_segment_matches_smoothstep_anchor() {
        // The Hermite of the keys (0,0,tangent 0) and (1,1,tangent 0) is
        // exactly smoothstep, 3t^2-2t^3. At t=0.25:
        // 3/16 - 2/64 = 0.1875 - 0.03125 = 0.15625. A hand-computed anchor,
        // not a native comparison.
        let c = smoothstep_curve();
        assert!((c.evaluate(0.25) - 0.15625).abs() < 1e-6);
        assert!((c.evaluate(0.5) - 0.5).abs() < 1e-6);
        assert!((c.evaluate(0.75) - 0.84375).abs() < 1e-6);
    }

    #[test]
    fn hermite_with_slopes_is_not_smoothstep() {
        // Keys (0,0,out=2) and (1,1,in=-1): m0=2, m1=-1. At t=0.5 the basis
        // is a=0.5, b=0.125, c=0.5, d=-0.125, so
        // v = 0.125*2 + 0.5*1 + (-0.125)*(-1) = 0.25+0.5+0.125 = 0.875.
        // A linear fallback would give 0.5; this anchor pins that the
        // tangents enter the Hermite.
        let c = Curve {
            multiplier: 1.0,
            keys: vec![
                CurveKey { time: 0.0, value: 0.0, in_slope: 0.0, out_slope: 2.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
                CurveKey { time: 1.0, value: 1.0, in_slope: -1.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
            ],
            pre_wrap: Some(2),
            post_wrap: Some(2),
        };
        assert!((c.evaluate(0.5) - 0.875).abs() < 1e-6);
    }

    #[test]
    fn curve_clamps_outside_key_range() {
        let c = smoothstep_curve();
        assert_eq!(c.evaluate(-1.0), 0.0);
        assert_eq!(c.evaluate(2.0), 1.0);
    }

    #[test]
    fn single_key_curve_is_constant() {
        let c = Curve {
            multiplier: 1.0,
            keys: vec![CurveKey { time: 0.0, value: 7.0, in_slope: 1.0, out_slope: 1.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 }],
            pre_wrap: Some(2),
            post_wrap: Some(2),
        };
        assert_eq!(c.evaluate(0.0), 7.0);
        assert_eq!(c.evaluate(0.5), 7.0);
        assert_eq!(c.evaluate(1.0), 7.0);
    }

    #[test]
    fn min_max_curve_constant_and_multiplier() {
        // `C# readable`: Constant -> constantMax; Curve -> eval * multiplier.
        assert_eq!(Constant(5.0).evaluate(0.3, 0.0), 5.0);
        let c = MinMaxCurve::Curve { multiplier: 4.0, max: smoothstep_curve() };
        // t=0.5 -> 0.5 * 4。
        assert!((c.evaluate(0.5, 0.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn min_max_curve_two_constants_lerps_with_clamped_t() {
        // Mathf.Lerp clamps t: t<0 gives min, t>1 gives max, the midpoint
        // stays centered.
        let c = TwoConstants { min: -1.0, max: 3.0 };
        assert_eq!(c.evaluate(0.0, -2.0), -1.0);
        assert_eq!(c.evaluate(0.0, 0.5), 1.0);
        assert_eq!(c.evaluate(0.0, 7.0), 3.0);
    }

    #[test]
    fn min_max_curve_two_curves_lerps_then_multiplies_once() {
        // `C# readable`: Lerp(eval(min), eval(max), lerp) * multiplier; the
        // multiplier applies to the interpolated value, not to either curve.
        let lo = smoothstep_curve(); // t=0.5 -> 0.5
        let hi = Curve {
            multiplier: 1.0,
            keys: vec![
                CurveKey { time: 0.0, value: 1.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
                CurveKey { time: 1.0, value: 3.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
            ],
            pre_wrap: Some(2),
            post_wrap: Some(2),
        }; // t=0.5 -> 2 (zero end tangents, so the midpoint is the linear mean)
        let c = TwoCurves { multiplier: 10.0, min: lo, max: hi };
        // Lerp(0.5, 2.0, 0.25) = 0.875 * 10 = 8.75。
        assert!((c.evaluate(0.5, 0.25) - 8.75).abs() < 1e-6);
    }

    #[test]
    fn gradient_mixes_color_and_alpha_independently() {
        // RGB stays red while the independent alpha table crosses its midpoint.
        // Source time codes around 0.25 and 0.75 are symmetric around t=0.5.
        let g = Gradient {
            mode: super::super::gradient::GradientMode::Blend,
            color_space: super::super::gradient::GradientColorSpace::Unspecified,
            color_keys: vec![GradientColorKey { time: 0.0, color: [1.0, 0.0, 0.0] },
                GradientColorKey { time: 1.0, color: [1.0, 0.0, 0.0] }],
            alpha_keys: vec![
                GradientAlphaKey { time: 0.25, alpha: 0.0 },
                GradientAlphaKey { time: 0.75, alpha: 1.0 },
            ],
        };
        let c = g.evaluate(0.5);
        assert!((c[0] - 1.0).abs() < 1e-6 && c[1] == 0.0 && c[2] == 0.0);
        assert!((c[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gradient_clamps_outside_keys() {
        let g = Gradient {
            mode: super::super::gradient::GradientMode::Blend,
            color_space: super::super::gradient::GradientColorSpace::Unspecified,
            color_keys: vec![
                GradientColorKey { time: 0.0, color: [0.0, 0.0, 0.0] },
                GradientColorKey { time: 1.0, color: [1.0, 1.0, 1.0] },
            ],
            alpha_keys: vec![GradientAlphaKey { time: 0.0, alpha: 0.5 },
                GradientAlphaKey { time: 1.0, alpha: 0.5 }],
        };
        assert_eq!(g.evaluate(-1.0), [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(g.evaluate(2.0), [1.0, 1.0, 1.0, 0.5]);
        assert!((g.evaluate(0.5)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn random_color_uses_lerp_factor_as_time() {
        // `C# readable`: RandomColor evaluates with the lerp factor as the
        // time. Keys (0, black) to (1, white) with lerp=0.25 give 0.25 grey;
        // reading it as Gradient mode (time as time) would give black at t=0.
        // This anchor pins the difference.
        let g = MinMaxGradient::RandomColor(Gradient {
            mode: super::super::gradient::GradientMode::Blend,
            color_space: super::super::gradient::GradientColorSpace::Unspecified,
            color_keys: vec![
                GradientColorKey { time: 0.0, color: [0.0, 0.0, 0.0] },
                GradientColorKey { time: 1.0, color: [1.0, 1.0, 1.0] },
            ],
            alpha_keys: vec![GradientAlphaKey { time: 0.0, alpha: 1.0 },
                GradientAlphaKey { time: 1.0, alpha: 1.0 }],
        });
        let c = g.evaluate(0.0, 0.25);
        assert!((c[0] - 0.25).abs() < 1e-6);
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn min_max_gradient_two_colors_componentwise_clamped() {
        let g = MinMaxGradient::TwoColors {
            min: [0.0, 0.0, 0.0, 1.0],
            max: [1.0, 2.0, 4.0, 0.0],
        };
        assert_eq!(g.evaluate(0.0, 0.5), [0.5, 1.0, 2.0, 0.5]);
        assert_eq!(g.evaluate(0.0, 2.0), [1.0, 2.0, 4.0, 0.0]);
    }

    #[test]
    fn two_gradients_lerps_each_evaluation() {
        let mk = |v: f32| Gradient {
            mode: super::super::gradient::GradientMode::Blend,
            color_space: super::super::gradient::GradientColorSpace::Unspecified,
            color_keys: vec![
                GradientColorKey { time: 0.0, color: [v, v, v] },
                GradientColorKey { time: 1.0, color: [v, v, v] },
            ],
            alpha_keys: vec![GradientAlphaKey { time: 0.0, alpha: v },
                GradientAlphaKey { time: 1.0, alpha: v }],
        };
        let g = MinMaxGradient::TwoGradients { min: mk(0.0), max: mk(1.0) };
        // Any time: both gradients are constant, so Lerp(0,1,0.5) = 0.5.
        let c = g.evaluate(0.7, 0.5);
        assert!((c[0] - 0.5).abs() < 1e-6);
        assert!((c[3] - 0.5).abs() < 1e-6);
    }
}
