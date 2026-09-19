//! Prepared particle curve evaluation, independent of module-specific random
//! streams, spatial transforms and integration state.
use crate::particle::value::{bezier_interpolate, step_value, CurveKey, MinMaxCurve};

/// agePercent（0..100）到曲线时刻的归一化：t = max(x·0.01, 0)。
/// 非数输入落 0（与引擎一致：maxps 的非数路返回源操作数 0）。
pub fn normalized_age(age_percent: f32) -> f32 {
    (age_percent * f32::from_bits(0x3c23_d70a)).max(0.0)
}

/// 引擎原式的动画曲线求值（单值）。斜率有限、键升序由装载处保证
/// （schema 拒绝 NaN 斜率与乱序键）；包绕按语料全为钳制（端点值）。
///
/// 次序逐指令对齐：h01·v1 + ((h00·v0 + h10·m0) + h11·m1)，
/// m0 = 出切·d、m1 = d·入切、d == 0（及非数 d——比较 Z 置位同走
/// 退化路 s=m0=m1=0，恰得 v0）恰好取 v0。段查找左闭右开：最后一
/// 个 time <= t 的键。
/// 加权段（左键出权位或右键入权位激活）转三次 Bezier 路
/// （`value::bezier_interpolate`，同为引擎原生逐指令）。
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

// ---- 烘制曲线 ----

/// 烘制曲线：构建期把 2–3 键曲线折成两段三次多项式（各四系数 +
/// 切换点），运行时 Horner 直接求值。
///
/// **系数存储次序是降幂**（原生实现逐指令钉死）：结构体首浮点
/// （lane 0）是**三次项**系数，末浮点是常数项。运行时求值形为
/// `((lane0·x + lane1)·x + lane2)·x + lane3`，展开即
/// `c3·x³ + c2·x² + c1·x + c0`——τ 空间系数：
/// c0 = v0，c1 = s0，c2 = (3Δv − d(2s0+s1))/d²，c3 = (d(s0+s1) − 2Δv)/d³。
///
/// 烘制判定是重建：构建期置位读不到，按「2–3 键且恰好张满 [0,1]」
/// 判（语料 24 根曲线全部张满 [0,1]；4+ 键结构放不下走通用路）。
/// 烘制与通用的差是重结合的舍入量级——重建侧以逐值比对报告上界。
#[derive(Clone, Debug)]
pub struct BakedCurve {
    /// 第一段 [k0, k1]（时刻从 0 起算，故首键时刻必须为 0）。
    /// 存储降幂 [c3, c2, c1, c0]。
    a: [f32; 4],
    /// 第二段 [k1, k2]（时刻从 switch 起算）。两键曲线永不选中。
    /// 存储降幂 [c3', c2', c1', c0']。
    b: [f32; 4],
    /// 切换点 = k1.time。
    switch: f32,
}

/// 单段系数；等时/逆序键返回 None（回退通用路）。
/// 返回降幂 [c3, c2, c1, c0]——与运行时 Horner 的取用次序对齐。
fn segment_coeffs(k0: CurveKey, k1: CurveKey, multiplier: f32) -> Option<[f32; 4]> {
    let d = k1.time - k0.time;
    if !(d > 0.0) {
        return None;
    }
    let dv = k1.value - k0.value;
    let s0 = k0.out_slope;
    let s1 = k1.in_slope;
    let c2 = (3.0 * dv - d * (2.0 * s0 + s1)) / (d * d);
    let c3 = (d * (s0 + s1) - 2.0 * dv) / (d * d * d);
    Some([
        c3 * multiplier,
        c2 * multiplier,
        s0 * multiplier,
        k0.value * multiplier,
    ])
}

impl BakedCurve {
    /// 烘制 2–3 键曲线；不满足形状返回 None。乘子折入系数（烘制
    /// 求值路不乘模块乘子，系数必须自带）。
    ///
    /// 任一键 `weighted_mode != 0` 即不烘：引擎的多项式优化资格门
    /// 逐键扫加权位，任一非零即判不合格（回通用求值路），且加权段
    /// 的 y 对 x 本就不是多项式，烘制在数学上也不成立。
    pub fn bake(keys: &[CurveKey], multiplier: f32) -> Option<Self> {
        if !(2..=3).contains(&keys.len()) {
            return None;
        }
        if keys.iter().any(|k| k.weighted_mode != 0 || !k.in_slope.is_finite() || !k.out_slope.is_finite()) {
            return None;
        }
        if keys[0].time != 0.0 || keys[keys.len() - 1].time != 1.0 {
            return None;
        }
        let a = segment_coeffs(keys[0], keys[1], multiplier)?;
        let (b, switch) = if keys.len() == 3 {
            (segment_coeffs(keys[1], keys[2], multiplier)?, keys[1].time)
        } else {
            // 两键：switch = 末键时刻 = 1.0，高于钳制上界，第二段
            // 永不被选中——镜像第一段当占位。
            (a, keys[1].time)
        };
        Some(Self { a, b, switch })
    }

    /// Horner `((a0·x + a1)·x + a2)·x + a3`（系数降幂存储，取用次序
    /// 逐指令对齐原生体）；切换判
    /// switch <= min(t, 0.9999899864196777) 取第二段（t−switch）。
    pub fn evaluate(&self, t: f32) -> f32 {
        let use_b = self.switch <= t.min(f32::from_bits(0x3f7f_ff58));
        if use_b {
            let tau = t - self.switch;
            let c = &self.b;
            ((c[0] * tau + c[1]) * tau + c[2]) * tau + c[3]
        } else {
            let c = &self.a;
            ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
        }
    }
}

/// One prepared scalar curve, shared across over-lifetime modules.
#[derive(Clone, Debug)]
pub enum CurveSampler {
    /// 模式 0：常数（弧度每秒）。
    Constant(f32),
    /// 模式 3：双常数，lerp 因子不钳制。
    TwoConstants { min: f32, max: f32 },
    /// 模式 1 通用：求值(曲线, t)·乘子。
    CurveGeneric { multiplier: f32, keys: Vec<CurveKey> },
    /// 模式 2 通用：lo/hi 各乘乘子后 lerp。
    TwoCurvesGeneric {
        multiplier: f32,
        min: Vec<CurveKey>,
        max: Vec<CurveKey>,
    },
    /// 模式 1 烘制：多项式自带乘子。
    CurveBaked(BakedCurve),
    /// 模式 2 烘制：两结构各取多项式后 lerp。
    TwoCurvesBaked { min: BakedCurve, max: BakedCurve },
}

impl CurveSampler {
    /// 从文档语义的四值模式构造（烘制/通用按形状分发，见
    /// [`BakedCurve::bake`] 的重建判据）。
    pub fn from_min_max_curve(curve: &MinMaxCurve) -> Self {
        Self::with_baking(curve, true)
    }

    pub fn with_baking(curve: &MinMaxCurve, allow_baking: bool) -> Self {
        let bake = |keys: &[CurveKey], multiplier: f32| {
            allow_baking.then(|| BakedCurve::bake(keys, multiplier)).flatten()
        };
        match curve {
            MinMaxCurve::Constant(v) => CurveSampler::Constant(*v),
            MinMaxCurve::TwoConstants { min, max } => CurveSampler::TwoConstants {
                min: *min,
                max: *max,
            },
            MinMaxCurve::Curve { multiplier, max } => {
                match bake(&max.keys, *multiplier) {
                    Some(b) => CurveSampler::CurveBaked(b),
                    None => CurveSampler::CurveGeneric {
                        multiplier: *multiplier,
                        keys: max.keys.clone(),
                    },
                }
            }
            MinMaxCurve::TwoCurves { multiplier, min, max } => {
                match (
                    bake(&min.keys, *multiplier),
                    bake(&max.keys, *multiplier),
                ) {
                    (Some(lo), Some(hi)) => CurveSampler::TwoCurvesBaked { min: lo, max: hi },
                    _ => CurveSampler::TwoCurvesGeneric {
                        multiplier: *multiplier,
                        min: min.keys.clone(),
                        max: max.keys.clone(),
                    },
                }
            }
        }
    }

    /// The random factor is supplied by the owning module's particle-local
    /// stream. It is not shared with the emitter's birth generator.
    pub fn evaluate(&self, t: f32, random: f32) -> f32 {
        match self {
            Self::Constant(value) => *value,
            Self::TwoConstants { min, max } => (max - min) * random + min,
            Self::CurveGeneric { multiplier, keys } => eval_curve(keys, t) * multiplier,
            Self::TwoCurvesGeneric { multiplier, min, max } => {
                let lo = eval_curve(min, t) * multiplier;
                let hi = eval_curve(max, t) * multiplier;
                (hi - lo) * random + lo
            }
            Self::CurveBaked(curve) => curve.evaluate(t),
            Self::TwoCurvesBaked { min, max } => {
                let lo = min.evaluate(t);
                let hi = max.evaluate(t);
                (hi - lo) * random + lo
            }
        }
    }

    pub fn can_bake(curve: &MinMaxCurve) -> bool {
        match curve {
            MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. } => true,
            MinMaxCurve::Curve { multiplier, max } => BakedCurve::bake(&max.keys, *multiplier).is_some(),
            MinMaxCurve::TwoCurves { multiplier, min, max } =>
                BakedCurve::bake(&min.keys, *multiplier).is_some()
                    && BakedCurve::bake(&max.keys, *multiplier).is_some(),
        }
    }
}
