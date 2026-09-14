//! rotationOverLifetime：绕轴自旋的每帧角速度采样律。
//!
//! 档位：**引擎原生逐指令**（原生实现逐指令转录；常量按位钉死）。
//!
//! 采样链按模块模式分六路（模式字 u16）：
//!
//! | 模式 | 路 | 值 | 翻转因子 |
//! |---|---|---|---|
//! | 0 常数 | 通用 | v = 标量 | 常数杂凑（平移式） |
//! | 3 双常数 | 通用 | v = (max−min)·r_lerp + min，**不钳制** lerp | 符号杂凑（原式） |
//! | 1 曲线 | 通用 | v = 求值(曲线, t)·乘子 | 符号杂凑（原式） |
//! | 2 双曲线 | 通用 | lo/hi 各乘乘子，v = (hi−lo)·r_lerp + lo | 符号杂凑（原式） |
//! | 1 曲线 | 烘制 | v = 多项式（**不乘乘子**——乘子折入系数） | 常数杂凑（平移式） |
//! | 2 双曲线 | 烘制 | 两结构各取多项式，v = (hi−lo)·r_lerp + lo | 符号杂凑（原式） |
//!
//! t = max(agePercent·0.01, 0)，agePercent 取**本帧推进前**的值（模块
//! 在推进之前跑）。翻转：randomizeDirection < r ? v : −v。本作资产该
//! 数组结构上恒 0（正负散布来自双常数/双曲线的 lerp 抽签本身），法仍
//! 取参数不硬编码。
//!
//! 随机数消耗：**每帧 0 次流抽取**。全部因子都是逐粒子 u32 种子的纯
//! 杂凑（t 与杂凑输入无关），同一颗粒子终生同批因子；种子在出生时
//! 抽定一次，见消费侧。
//!
//! 与 value.rs 文档语义的三处差异（因此不复用其求值）：
//! lerp 不钳制（value.rs 钳 [0,1]）；等时键退化给 v0（value.rs 给
//! k1.value）；合成次序逐指令对齐（value.rs 是 a·v0+b·m0+c·v1+d·m1）。

use crate::particle::value::{bezier_interpolate, CurveKey, MinMaxCurve};

// ---- 杂凑核（常量按位钉死） ----

/// 乘法因子 M。
const HASH_M: u32 = 0x6ab5_1b9d;
/// lerp 因子的加项 T（原式）。
const HASH_T_LERP: u32 = 0x6aed_452e;
/// lerp 因子的加项 C（原式）。
const HASH_C_LERP: u32 = 0x00a0_1275;
/// 符号/常数因子共用的加项 T（两者同一常量）。
const HASH_T_SIGN: u32 = 0xff2b_b1a4;
/// 符号因子的加项 C（原式）。
const HASH_C_SIGN: u32 = 0x0bc7_08d3;
/// 常数因子的加项 C（平移式）。
const HASH_C_CONST: u32 = 0x714a_cb3f;
/// 尾数掩码（最大非规格数）。
const HASH_MASK: u32 = 0x007f_ffff;
/// 整数转 [0,1) 的乘子：1.00000012·2^-23。
const HASH_UNIT: f32 = f32::from_bits(0x3400_0001);

/// 杂凑核（u32 回绕；限速族与角速度族共用）：
/// h = t ^ (t<<11)；x = ((h ^ (h>>8) ^ m) & 0x007fffff) ^ (m>>19)；
/// r = f32(x)·1.00000012·2^-23 ∈ [0, 1−2^-24)。
pub(crate) fn hash_mix(t: u32, m: u32) -> f32 {
    let h = t ^ (t << 11);
    let x = ((h ^ (h >> 8) ^ m) & HASH_MASK) ^ (m >> 19);
    f32::from_bits(x) * HASH_UNIT
}

/// lerp 因子（原式）：t = 种子+T_lerp，m = 种子·M + C_lerp。
fn hash_lerp_raw(seed: u32) -> f32 {
    hash_mix(
        seed.wrapping_add(HASH_T_LERP),
        seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_LERP),
    )
}

/// 符号因子（原式）：t = 种子+T_sign，m = 种子·M + C_sign。
fn hash_sign_raw(seed: u32) -> f32 {
    hash_mix(
        seed.wrapping_add(HASH_T_SIGN),
        seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_SIGN),
    )
}

/// 常数因子（平移式）：t = 种子+T_const，m = (种子+T_const)·M + C_const。
/// 与符号因子同 T 不同形：乘法吃的是平移后的种子。
fn hash_const_shifted(seed: u32) -> f32 {
    let t = seed.wrapping_add(HASH_T_SIGN);
    hash_mix(t, t.wrapping_mul(HASH_M).wrapping_add(HASH_C_CONST))
}

// ---- 曲线求值（引擎原生逐指令） ----

/// agePercent（0..100）到曲线时刻的归一化：t = max(x·0.01, 0)。
/// 非数输入落 0（与引擎一致：maxps 的非数路返回源操作数 0）。
pub fn normalized_age(age_percent: f32) -> f32 {
    (age_percent * f32::from_bits(0x3c23_d70a)).max(0.0)
}

/// 引擎原式的动画曲线求值（单值）。斜率有限、键升序由装载处保证
/// （schema 拒绝 ±inf 斜率与乱序键）；包绕按语料全为钳制（端点值）。
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
        if keys.iter().any(|k| k.weighted_mode != 0) {
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

// ---- 角速度律 ----

/// 一根轴的角速度律（模块四模式 × 通用/烘制两路）。
#[derive(Clone, Debug)]
pub enum AngularVelocityLaw {
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

impl AngularVelocityLaw {
    /// 从文档语义的四值模式构造（烘制/通用按形状分发，见
    /// [`BakedCurve::bake`] 的重建判据）。
    pub fn from_min_max_curve(curve: &MinMaxCurve) -> Self {
        match curve {
            MinMaxCurve::Constant(v) => AngularVelocityLaw::Constant(*v),
            MinMaxCurve::TwoConstants { min, max } => AngularVelocityLaw::TwoConstants {
                min: *min,
                max: *max,
            },
            MinMaxCurve::Curve { multiplier, max } => {
                match BakedCurve::bake(&max.keys, *multiplier) {
                    Some(b) => AngularVelocityLaw::CurveBaked(b),
                    None => AngularVelocityLaw::CurveGeneric {
                        multiplier: *multiplier,
                        keys: max.keys.clone(),
                    },
                }
            }
            MinMaxCurve::TwoCurves { multiplier, min, max } => {
                match (
                    BakedCurve::bake(&min.keys, *multiplier),
                    BakedCurve::bake(&max.keys, *multiplier),
                ) {
                    (Some(lo), Some(hi)) => AngularVelocityLaw::TwoCurvesBaked { min: lo, max: hi },
                    _ => AngularVelocityLaw::TwoCurvesGeneric {
                        multiplier: *multiplier,
                        min: min.keys.clone(),
                        max: max.keys.clone(),
                    },
                }
            }
        }
    }

    /// 采样每帧角速度（弧度每秒）。`age_percent` 是 0..100 的进程量
    /// （推进前）；`randomize_direction` 结构上恒 0，仍按参数取。
    ///
    /// 翻转因子的杂凑形按路径分三族：常数与**烘制曲线**用平移式常数
    /// 因子（t = 种子+T，m = (种子+T)·M + C_const）；双常数/双曲线的
    /// lerp 因子与全部符号因子用原式（m = 种子·M + C）。
    pub fn sample(&self, seed: u32, randomize_direction: f32, age_percent: f32) -> f32 {
        let t = normalized_age(age_percent);
        let v = match self {
            AngularVelocityLaw::Constant(v) => {
                let r = hash_const_shifted(seed);
                return if randomize_direction < r { *v } else { -*v };
            }
            AngularVelocityLaw::TwoConstants { min, max } => {
                let r_lerp = hash_lerp_raw(seed);
                (max - min) * r_lerp + min
            }
            AngularVelocityLaw::CurveGeneric { multiplier, keys } => {
                eval_curve(keys, t) * multiplier
            }
            AngularVelocityLaw::TwoCurvesGeneric { multiplier, min, max } => {
                let lo = eval_curve(min, t) * multiplier;
                let hi = eval_curve(max, t) * multiplier;
                let r_lerp = hash_lerp_raw(seed);
                (hi - lo) * r_lerp + lo
            }
            // 烘制曲线模式的翻转走平移式常数因子（与常数模式同形），
            // 不落下面共享尾部的原式符号因子——两条原生路的杂凑形不同。
            AngularVelocityLaw::CurveBaked(b) => {
                let r = hash_const_shifted(seed);
                let v = b.evaluate(t);
                return if randomize_direction < r { v } else { -v };
            }
            AngularVelocityLaw::TwoCurvesBaked { min, max } => {
                let lo = min.evaluate(t);
                let hi = max.evaluate(t);
                let r_lerp = hash_lerp_raw(seed);
                (hi - lo) * r_lerp + lo
            }
        };
        let r_sign = hash_sign_raw(seed);
        if randomize_direction < r_sign {
            v
        } else {
            -v
        }
    }
}

/// rotationOverLifetime 模块：三根轴各一条角速度律。separateAxes 为假
/// 时只有 z（公告板自旋轴）；为真时三轴各自采样。
#[derive(Clone, Debug)]
pub struct RotationOverLifetime {
    pub separate_axes: bool,
    pub x: Option<AngularVelocityLaw>,
    pub y: Option<AngularVelocityLaw>,
    pub z: AngularVelocityLaw,
}

impl RotationOverLifetime {
    /// 从文档语义的轴组构造。separate_axes 为真而 x/y 缺席（提取面缺
    /// 键）时响亮拒绝——那不是「当 0」，是数据形状不对。
    pub fn from_parts(
        separate_axes: bool,
        x: Option<&MinMaxCurve>,
        y: Option<&MinMaxCurve>,
        z: &MinMaxCurve,
    ) -> Result<Self, String> {
        if separate_axes && (x.is_none() || y.is_none()) {
            return Err(format!(
                "rotationOverLifetime: separateAxes 开而 x/y 轴键缺（x={} y={}）——提取面缺键，拒绝猜测",
                x.is_some(),
                y.is_some()
            ));
        }
        Ok(Self {
            separate_axes,
            x: x.map(AngularVelocityLaw::from_min_max_curve),
            y: y.map(AngularVelocityLaw::from_min_max_curve),
            z: AngularVelocityLaw::from_min_max_curve(z),
        })
    }

    /// 三轴每帧角速度（弧度每秒）。未启用的轴为 0。
    pub fn angular_velocity(
        &self,
        seed: u32,
        randomize_direction: f32,
        age_percent: f32,
    ) -> [f32; 3] {
        if self.separate_axes {
            [
                self.x
                    .as_ref()
                    .map_or(0.0, |l| l.sample(seed, randomize_direction, age_percent)),
                self.y
                    .as_ref()
                    .map_or(0.0, |l| l.sample(seed, randomize_direction, age_percent)),
                self.z.sample(seed, randomize_direction, age_percent),
            ]
        } else {
            [0.0, 0.0, self.z.sample(seed, randomize_direction, age_percent)]
        }
    }

    /// 推进旋转：rotation[axis] += 角速度[axis]·dt。rotation3d 为真三
    /// 轴积分，为假只积 z（引擎里 x/y 的角速度也被清零，不积）。
    /// 非有限 dt 拒绝（fail-closed）。
    pub fn advance_rotation(
        &self,
        rotation: &mut [f32; 3],
        seed: u32,
        randomize_direction: f32,
        age_percent: f32,
        rotation3d: bool,
        dt: f32,
    ) -> Result<(), &'static str> {
        if !dt.is_finite() {
            return Err("rotationOverLifetime: dt 非有限");
        }
        let w = self.angular_velocity(seed, randomize_direction, age_percent);
        if rotation3d {
            for axis in 0..3 {
                rotation[axis] += w[axis] * dt;
            }
        } else {
            rotation[2] += w[2] * dt;
        }
        Ok(())
    }
}
