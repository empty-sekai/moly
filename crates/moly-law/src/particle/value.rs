//! `MinMaxCurve` / `MinMaxGradient` 的逐式求值：粒子模块全部「随时间
//! 变化的参数」共用这一族。
//!
//! 四曲线模式与五颜色模式的 `Evaluate` 是 `C# 可读`（UnityCsReference
//! `ParticleSystemStructs.cs`，逐式转写，含 `Mathf.Lerp` 与 `Color.Lerp`
//! 的 t 钳位——两者同样 `C# 可读`）。底层 `AnimationCurve.Evaluate` 与
//! `Gradient.Evaluate` 落在 extern 墙后（两者在引擎里都是原生实现），
//! 本文件的曲线与梯度求值是**行为口径**：按文档语义实现，逐条见函数
//! 注释里的测量方案。

use crate::particle::value::MinMaxCurve::{Constant, TwoConstants, TwoCurves};

/// 一根动画曲线键。加权位（`weighted_mode` 的 bit0=入权 · bit1=出权）
/// 激活时，该键相邻段的求值走三次 Bezier 路（见 `curve::bezier_interpolate`）；
/// 未激活的权重位引擎在求值时代 1/3，存值不参与求值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveKey {
    /// 键时刻，归一化到 [0,1]（粒子模块的曲线恒用归一化时间）。
    pub time: f32,
    pub value: f32,
    /// 入切线（本键左侧段的右端斜率）。
    pub in_slope: f32,
    /// 出切线（本键右侧段的左端斜率）。
    pub out_slope: f32,
    /// 加权模式位（0=未加权 · 1=仅入权 · 2=仅出权 · 3=双权）。
    /// 高于 bit1 的位求值不读（引擎只测 bit0/bit1），原样存。
    pub weighted_mode: u8,
    /// 入权（本键作为段右端时的 Bezier 控制点权重）。
    pub in_weight: f32,
    /// 出权（本键作为段左端时的 Bezier 控制点权重）。
    pub out_weight: f32,
}

/// 一根未加权键序列。多键按时刻升序给定；求值不排序（键序错是数据
/// 损伤，由 schema 解析处拒绝，不由求值处默默修正）。
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    /// 曲线乘子：`Curve` 模式乘在求值结果上，`TwoCurves` 模式乘在
    /// 插值结果上——两处都乘且只乘一次，见 `MinMaxCurve::evaluate`。
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

// ---- 加权段 ----

/// The step override `InterpolateKeyframe` applies after either branch: +inf on
/// `k0`'s out tangent or `k1`'s in tangent gives `k0.value`; otherwise -inf on
/// either gives `k1.value`. The weighted (Bezier) branch itself is
/// `curve::interpolate_keyframe`, transcribed from the engine.
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
    /// `AnimationCurve.Evaluate(time)` 的行为口径（未加权支）+ 引擎
    /// 原生逐指令（加权支）。
    ///
    /// 未加权段：按文档语义实现——区间内三次 Hermite（切线 = 键斜率 ×
    /// 区间时长），首键前/末键后取端点值（钳位），单键恒返回该键值。
    ///
    /// 加权段（左键出权位或右键入权位激活）：引擎的 Bezier 路，见
    /// [`crate::particle::curve::interpolate_keyframe`]——加权段无文档语义，只有引擎式。
    ///
    /// Editor 测量方案：`AnimationCurve` 键 (0,0,outSlope=0) 与 (1,1,
    /// inSlope=0) 在 t=0.25 处求值应为 smoothstep 值 5/32 = 0.15625
    /// （本模块测试已按此锚）；再用非对称斜率 (out=2,in=−1) 于
    /// t=0.5 采一点核对 Hermite 而非线性。
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
                // 上面的钳位保证此处 time 落在 (first.time, last.time)。
                let i = keys
                    .windows(2)
                    .position(|w| w[0].time <= time && time < w[1].time)
                    .expect("time between first and last always falls in a segment");
                let (k0, k1) = (keys[i], keys[i + 1]);
                let dt = k1.time - k0.time;
                if !(dt > 0.0) {
                    // 键时刻重合：区间宽度为 0，取右键值（左键已越过）。
                    return k1.value;
                }
                if crate::particle::curve::weighted_segment(k0, k1) {
                    return crate::particle::curve::interpolate_keyframe(k0, k1, time);
                }
                let t = (time - k0.time) / dt;
                let m0 = k0.out_slope * dt;
                let m1 = k1.in_slope * dt;
                // 标准 Hermite 基：a·v0 + b·m0 + c·v1 + d·m1。
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

/// 粒子参数的四值模式。`C# 可读`：`ParticleSystemCurveMode`
/// { Constant=0, Curve=1, TwoCurves=2, TwoConstants=3 }，`Evaluate`
/// 逐式转写自 `MinMaxCurve.Evaluate`。
#[derive(Debug, Clone, PartialEq)]
pub enum MinMaxCurve {
    /// 序列化名 `constant`（引擎内部存 `constantMax`）。
    Constant(f32),
    /// 序列化名 `curve`（引擎内部存 `curveMax` × `curveMultiplier`）。
    Curve { multiplier: f32, max: Curve },
    /// 序列化名 `twoConstants`（`constantMin`/`constantMax`）。
    TwoConstants { min: f32, max: f32 },
    /// 序列化名 `twoCurves`（`curveMin`/`curveMax` 共用一个乘子）。
    TwoCurves {
        multiplier: f32,
        min: Curve,
        max: Curve,
    },
}

impl MinMaxCurve {
    /// `MinMaxCurve.Evaluate(time, lerpFactor)`，`C# 可读`逐式：
    ///
    /// ```text
    /// Constant     -> constantMax
    /// Curve        -> curveMax.Evaluate(time) * multiplier
    /// TwoConstants -> Lerp(constantMin, constantMax, lerpFactor)
    /// TwoCurves    -> Lerp(curveMin.Evaluate(time), curveMax.Evaluate(time),
    ///                lerpFactor) * multiplier
    /// ```
    ///
    /// `Lerp` 是 `Mathf.Lerp`：`a + (b-a) * Clamp01(t)`——钳位可读，
    /// 不是猜的。`lerpFactor` 是调用方给的显式随机（典型是 `Random.value`，
    /// 每粒子一次，见 `emit` 的 spawn 口径）。
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

/// `Mathf.Lerp`：插值因子钳位到 [0,1]。`C# 可读`。
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// `Color.Lerp`：RGBA 四分量各自 `Mathf.Lerp`，t 同样钳位。`C# 可读`。
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
    /// 序列化名 `color`（引擎内部存 `colorMax`）。
    Color([f32; 4]),
    /// 序列化名 `gradient`（`gradientMax`）。
    Gradient(Gradient),
    /// 序列化名 `twoColors`（`colorMin`/`colorMax`）。
    TwoColors { min: [f32; 4], max: [f32; 4] },
    /// 序列化名 `twoGradients`（`gradientMin`/`gradientMax`）。
    TwoGradients { min: Gradient, max: Gradient },
    /// 序列化名 `randomColor`：**插值因子当时间用**——`gradientMax.
    /// Evaluate(lerpFactor)`，不是 `Evaluate(time)`。这是真源自己的
    /// 怪写法（`C# 可读`），由测试钉住，别顺手「修好」它。
    RandomColor(Gradient),
}

impl MinMaxGradient {
    /// `MinMaxGradient.Evaluate(time, lerpFactor)`，`C# 可读`逐式：
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

    /// 平滑两键：值域 [0,1]、双端切线为 0 的 smoothstep 形。
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
        // 键 (0,0,切0) 与 (1,1,切0) 的 Hermite 恰是 smoothstep
        // 3t^2-2t^3。t=0.25 -> 3/16 - 2/64 = 0.1875 - 0.03125 = 0.15625。
        // 手算锚，非对拍。
        let c = smoothstep_curve();
        assert!((c.evaluate(0.25) - 0.15625).abs() < 1e-6);
        assert!((c.evaluate(0.5) - 0.5).abs() < 1e-6);
        assert!((c.evaluate(0.75) - 0.84375).abs() < 1e-6);
    }

    #[test]
    fn hermite_with_slopes_is_not_smoothstep() {
        // 键 (0,0,out=2) 与 (1,1,in=-1)：m0=2、m1=-1。
        // t=0.5 处基为 a=0.5,b=0.125,c=0.5,d=-0.125 ->
        // v = 0.125*2 + 0.5*1 + (-0.125)*(-1) = 0.25+0.5+0.125 = 0.875。
        // 若实现退化成线性会是 0.5——此锚专门钉「Hermite 真的在算切线」。
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
        // C# 可读逐式：Constant -> constantMax；Curve -> eval * multiplier。
        assert_eq!(Constant(5.0).evaluate(0.3, 0.0), 5.0);
        let c = MinMaxCurve::Curve { multiplier: 4.0, max: smoothstep_curve() };
        // t=0.5 -> 0.5 * 4。
        assert!((c.evaluate(0.5, 0.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn min_max_curve_two_constants_lerps_with_clamped_t() {
        // Mathf.Lerp 钳位 t：t<0 取 min、t>1 取 max、中点居中。
        let c = TwoConstants { min: -1.0, max: 3.0 };
        assert_eq!(c.evaluate(0.0, -2.0), -1.0);
        assert_eq!(c.evaluate(0.0, 0.5), 1.0);
        assert_eq!(c.evaluate(0.0, 7.0), 3.0);
    }

    #[test]
    fn min_max_curve_two_curves_lerps_then_multiplies_once() {
        // C# 可读：Lerp(eval(min), eval(max), lerp) * multiplier —— 乘子
        // 乘在插值结果上，不进两条曲线各自。
        let lo = smoothstep_curve(); // t=0.5 -> 0.5
        let hi = Curve {
            multiplier: 1.0,
            keys: vec![
                CurveKey { time: 0.0, value: 1.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
                CurveKey { time: 1.0, value: 3.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
            ],
            pre_wrap: Some(2),
            post_wrap: Some(2),
        }; // t=0.5 -> 2（端点切线 0，中点恰线性中值）
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
        // C# 可读的怪写法：RandomColor 求值时 lerpFactor 当 time。
        // 键 (0,黑)→(1,白)，lerp=0.25 -> 0.25 灰；若误当 Gradient 模式
        // （time 当 time）则 t=0 处恒黑——此锚钉住差异。
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
        // time=任意：两梯度各自恒值，Lerp(0,1,0.5) = 0.5。
        let c = g.evaluate(0.7, 0.5);
        assert!((c[0] - 0.5).abs() < 1e-6);
        assert!((c[3] - 0.5).abs() < 1e-6);
    }
}
