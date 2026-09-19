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
/// 激活时，该键相邻段的求值走三次 Bezier 路（见 [`bezier_interpolate`]）；
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
}

// ---- 加权 Hermite（引擎原生逐指令） ----

/// 加权段求值：三次 Bezier 插值。档位：**引擎原生逐指令**——加权
/// Hermite 没有文档语义可依（参考源只给枚举与字段，求值是原生实现），
/// 本函数按原生体逐指令转录，常量按位钉死。
///
/// 段 `(lhs, rhs)` 加权 iff `lhs.weighted_mode & 2`（左键出权位）或
/// `rhs.weighted_mode & 1`（右键入权位）；未激活的权重位原式用 `fcsel`
/// 代 1/3（`0x3eaaaaab`），存值不参与。
///
/// 次序逐指令对齐：x = (t−lhs.time)/d；m0 = d·lhs.outT；m1 = d·rhs.inT；
/// u = [`bezier_extract_u`](x, ow, 1−iw)；P1y = v0 + (ow·m0)；
/// P2y = v1 − (iw·m1)；B0 = (1−u)·((1−u)·(1−u))；B1 = (u·3)·((1−u)·(1−u))；
/// B2 = (1−u)·((u·u)·3)；B3 = u·(u·u)；
/// 值 = v1·B3 + (P2y·B2 + (P1y·B1 + v0·B0))。
///
/// 原式在 Bezier 路之后还有一步 ±inf 切线覆写，见 step_value。
/// d == 0 由各档调用处的退化分支先行接管（引擎原式给 v0）。
pub fn bezier_interpolate(t: f32, k0: CurveKey, k1: CurveKey) -> f32 {
    let third = f32::from_bits(0x3eaa_aaab);
    let ow = if k0.weighted_mode & 2 == 0 { third } else { k0.out_weight };
    let iw = if k1.weighted_mode & 1 == 0 { third } else { k1.in_weight };
    let d = k1.time - k0.time;
    // 原式 `fcmp d,#0.0; b.ne`：非数比较 Z 置位、b.ne 不跳，
    // 与 d==0 同落「回左值」——位级等价形是 !(d < 0.0 || d > 0.0)
    // （非数的 `d != 0.0` 在 Rust 里为真，用 `!=` 转录会漏非数）。
    if !(d < 0.0 || d > 0.0) {
        return k0.value;
    }
    let x = (t - k0.time) / d;
    let m0 = d * k0.out_slope;
    let m1 = d * k1.in_slope;
    let u = bezier_extract_u(x, ow, 1.0 - iw);
    let uu = u * u;
    let omu = 1.0 - u;
    let u3 = u * uu;
    let omu2 = omu * omu;
    let p1y = k0.value + (ow * m0);
    let p2y = k1.value - (iw * m1);
    let b0 = omu * omu2;
    let b1 = (u * 3.0) * omu2;
    let b2 = omu * (uu * 3.0);
    let b3v1 = k1.value * u3;
    let t0 = (k0.value * b0) + (p1y * b1);
    let t1 = (p2y * b2) + t0;
    step_value(k0, k1).unwrap_or(b3v1 + t1)
}

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

/// x-Bezier 求逆：解 `c·u³ + b·u² + a·u − x = 0` 取 [0,1] 内的根，
/// 其中 a = ow·3，b = 3(1−iw) + ow·(−6)，c = (ow·3 − 3(1−iw)) + 1
/// （即横标 Bezier `3u(1−u)²·ow + 3u²(1−u)·(1−iw) + u³ = x` 的系数）。
///
/// 次序逐指令对齐，四条路按原式分支：
///
/// - `|c| ≤ 0.001`（`0x3a83126f`，比较是 `b.le`——非数也走此路，转录成
///   `!(|c| > eps)`）：二次路 `b·u² + a·u − x = 0`，根
///   `(−a−√(a²+4bx))/(b+b)` 与 `(√(a²+4bx)−a)/(b+b)` 依次取 [0,1] 内者；
///   `|b| ≤ 0.001` 再退线性路 `|a| ≤ 0.001 ? 0 : x/a`（**不查** [0,1]）。
/// - 否则 Cardano：s12 = −b/(3c)；q = (a·b + (3c)·x)/(c·(c·6))；
///   p3 = a/(3c) − s12²；Δ = s12'² + p3³（s12' = s12³ + q）。
///   Δ ≥ 0（`b.pl`：N==0 才跳实根路，**非数 N 置位、不跳**，落三角
///   路——转录成 `Δ >= 0.0`）：实根
///   s12 + (scbrt(s12'+√Δ) + scbrt(s12'−√Δ))；
///   Δ < 0：φ = atan2f(√(−Δ), s12')；幅值 = 2·scbrt(√(−p3³))；
///   三候选 s12 + 幅值·cosf(φ/3 + {0, +2π/3, −2π/3})
///   （`0x40060a92` / `0xc0060a92`）。
/// - 候选判 [0,1] 的原式是位级操作，且**逐候选取形不同**：二次根与前
///   两个三角候选用 `b.lt`/`b.ls` 对——非数候选拿 `b.lt`（N≠V）不跳、
///   `b.ls`（Z 置位）跳，**非数候选会被返回**（`!(c < 0.0 || c > 1.0)`）；
///   末个三角候选用 `b.ge`（N==0）、实根候选用尾部 `fccmp+fcsel ge`——
///   这两处非数落兜底（`c >= 0.0 && c <= 1.0`）。
/// - 全部落空：x < 0.5 ? 0 : 1。
///
/// scbrt(z) = sign(z)·f32(exp_f64(f64(logf(|z|))/3.0))——logf 单精度、
/// 除以 3 与 exp 在双精度、回单精度，负数的符号在 f64 exp 之后回填。
/// 三角函数 atan2f/cosf 同为库调用。
fn bezier_extract_u(x: f32, ow: f32, omiw: f32) -> f32 {
    let a = ow * 3.0;
    let t3_omiw = omiw * 3.0;
    let c = (a - t3_omiw) + 1.0;
    let b = t3_omiw + ow * (-6.0);
    let eps = f32::from_bits(0x3a83_126f);
    if !(c.abs() > eps) {
        if !(b.abs() > eps) {
            if !(a.abs() > eps) {
                return 0.0;
            }
            return x / a;
        }
        let disc = (a * a) + ((b * 4.0) * x);
        let sq = disc.sqrt();
        let two_b = b + b;
        let cand1 = (-a - sq) / two_b;
        if !(cand1 < 0.0 || cand1 > 1.0) {
            return cand1;
        }
        let cand2 = (sq - a) / two_b;
        if !(cand2 < 0.0 || cand2 > 1.0) {
            return cand2;
        }
        return fallback_u(x);
    }
    let three_c = c * 3.0;
    let s12 = (-b) / three_c;
    let ab = a * b;
    let c3x = three_c * x;
    let six_c2 = c * (c * 6.0);
    let s12_sq = s12 * s12;
    let q = (ab + c3x) / six_c2;
    let s12p = (s12 * s12_sq) + q;
    let p3 = (a / three_c) - s12_sq;
    let delta = (s12p * s12p) + (p3 * (p3 * p3));
    // 原式 b.pl：非数不跳（落三角路），与 Δ>=0 有序同形——
    // !(Δ < 0) 会把非数错送实根路，故取 >= 0.0 字面形。
    if delta >= 0.0 {
        let sq = delta.sqrt();
        let cand = s12 + (scbrt(s12p + sq) + scbrt(s12p - sq));
        // 尾部 fccmp(≤1) + fcsel ge(≥0) 形：非数落兜底。
        if cand >= 0.0 && cand <= 1.0 {
            return cand;
        }
        return fallback_u(x);
    }
    let sqrt_nd = (-delta).sqrt();
    let sqrt_np3 = ((s12p * s12p) - delta).sqrt();
    let phi = sqrt_nd.atan2(s12p);
    let amp = {
        let v = scbrt(sqrt_np3);
        v + v
    };
    let phi3 = phi / 3.0;
    let two_pi_3 = f32::from_bits(0x4006_0a92);
    let neg_two_pi_3 = f32::from_bits(0xc006_0a92);
    let c1 = s12 + (amp * phi3.cos());
    if !(c1 < 0.0 || c1 > 1.0) {
        return c1;
    }
    let c2 = s12 + (amp * (phi3 + two_pi_3).cos());
    if !(c2 < 0.0 || c2 > 1.0) {
        return c2;
    }
    let c3 = s12 + (amp * (phi3 + neg_two_pi_3).cos());
    // 末三角候选用 b.ge 形（非数落兜底，与前两个候选的 b.lt/b.ls 对不同）。
    if c3 >= 0.0 && c3 <= 1.0 {
        return c3;
    }
    fallback_u(x)
}

/// 候选全落空时的兜底（原式 `fcsel …, mi`：x < 0.5（含非数——
/// 非数比较 N 置位，mi 成立）取 0，否则取 1）。
fn fallback_u(x: f32) -> f32 {
    if !(x >= 0.5) {
        0.0
    } else {
        1.0
    }
}

/// 带符号立方根：sign(z)·f32(exp_f64(f64(logf(|z|))/3.0))。
/// 负数的路径是先取 −z 的 logf，符号在 f64 exp 之后回填再转回 f32。
fn scbrt(z: f32) -> f32 {
    if z < 0.0 {
        let e = ((-z).ln() as f64 / 3.0).exp();
        (-e) as f32
    } else {
        let e = (z.ln() as f64 / 3.0).exp();
        e as f32
    }
}

impl Curve {
    /// `AnimationCurve.Evaluate(time)` 的行为口径（未加权支）+ 引擎
    /// 原生逐指令（加权支）。
    ///
    /// 未加权段：按文档语义实现——区间内三次 Hermite（切线 = 键斜率 ×
    /// 区间时长），首键前/末键后取端点值（钳位），单键恒返回该键值。
    ///
    /// 加权段（左键出权位或右键入权位激活）：三次 Bezier 路，见
    /// [`bezier_interpolate`]——加权 Hermite 无文档语义，只有引擎式。
    ///
    /// Editor 测量方案：`AnimationCurve` 键 (0,0,outSlope=0) 与 (1,1,
    /// inSlope=0) 在 t=0.25 处求值应为 smoothstep 值 5/32 = 0.15625
    /// （本模块测试已按此锚）；再用非对称斜率 (out=2,in=−1) 于
    /// t=0.5 采一点核对 Hermite 而非线性。
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
                if k0.weighted_mode & 2 != 0 || k1.weighted_mode & 1 != 0 {
                    return bezier_interpolate(time, k0, k1);
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
