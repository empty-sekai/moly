//! limitVelocity：限速（速度模向阈值收敛）与拖拽律。
//!
//! 档位：**引擎原生逐指令**（原生实现逐指令转录；常量按位钉死）。
//!
//! 同一模块两段，按序都跑：
//!
//! 1. **钳制段**（门 `dampen > 0`，非数 dampen 跳过）：每帧一次
//!    `k = 1.0 − powf(1.0−dampen, |dt|·30.0)`；每颗粒子
//!    合速度 = 状态速度 + 叠加速度；`mag2 = x² + (y²+z²)`；
//!    `newmag = (limit < |mag|) ? |mag| + (limit−|mag|)·k : |mag|`；
//!    `状态速度 = 单位向量·newmag − 叠加速度`。limit 按幅值模式取
//!    常数（不读种子）/ 双常数（本段自己的杂凑，平移式，lerp 不钳制）
//!    / 烘制曲线（t = max(agePercent·0.01, 0)，不读种子）。
//! 2. **拖拽段**（门 `drag != 0`，**非数 drag 照跑**）：按当前合速度
//!    （钳制后重算）取 `factor = ((尺寸因子·drag)·速度因子)·dt`，
//!    `newmag = mag − factor`（下钳 0），单位向量除以 mag。
//!
//! 具名偏差（逐值比对处报告量级）：
//! - powf 是库函数（IFUNC，CPU/实现相关），k 的逐位一致按构造不可达；
//! - 钳制段单位向量引擎走 rsqrtps + 两步牛顿（CPU 相关），此处按
//!   **精确除法**；拖拽段两处一致走精确除法（转储证实 divps）。
//!
//! 随机数消耗：**每帧 0 次流抽取**——唯一的杂凑（双常数 limit）是
//! 逐粒子种子的纯函数，终生恒定。
//!
//! 未按原生路径实现（引擎有路、未逐指令读、语料 0；构造处响亮拒绝）：
//! separateAxis 轴ewise 族 · 幅值通用曲线（模式 1/2 非烘制）· 幅值
//! 模式 2 烘制 · 拖拽非常数（模式 1/2/3）。

use crate::particle::buffer::RingBufferMode;
use crate::particle::rotation::{hash_mix, normalized_age, BakedCurve};
use crate::particle::value::MinMaxCurve;

// ---- 杂凑（钳制段双常数） ----

/// 钳制段加项 T。
const HASH_T_CLAMP: u32 = 0x1337_1337;
/// 钳制段加项 C（与角速度常数因子共用同一常量）。
const HASH_C_CONST: u32 = 0x714a_cb3f;
/// 乘法因子 M（与角速度族共用）。
const HASH_M: u32 = 0x6ab5_1b9d;

/// 双常数 limit 的 lerp 因子（平移式）：
/// t = 种子+T_clamp，m = (种子+T_clamp)·M + C_const。不钳制。
fn clamp_lerp_hash(seed: u32) -> f32 {
    let t = seed.wrapping_add(HASH_T_CLAMP);
    hash_mix(
        t,
        t.wrapping_mul(HASH_M).wrapping_add(HASH_C_CONST),
    )
}

// ---- 收敛系数 ----

/// `k = 1.0 − powf(1.0−dampen, |dt|·30.0)`。powf 为库函数，逐位一致
/// 按构造不可达（具名偏差）。
pub fn clamp_k(dampen: f32, dt: f32) -> f32 {
    1.0 - (1.0 - dampen).powf(dt.abs() * 30.0)
}

// ---- 幅值律 ----

/// 限速阈值的取值律。
#[derive(Clone, Debug)]
pub enum MagnitudeLaw {
    /// 模式 0：常数。不读种子、不读年寿。
    Constant(f32),
    /// 模式 3：双常数。逐粒子杂凑一次，lerp 不钳制。
    TwoConstants { min: f32, max: f32 },
    /// 模式 1 烘制：多项式自带乘子；t = max(agePercent·0.01, 0)。
    Baked(BakedCurve),
}

impl MagnitudeLaw {
    /// 从文档语义四值模式构造。通用曲线与双曲线模式未按原生路径
    /// 实现（语料 0），响亮拒绝。
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
                    "limitVelocity.magnitude: 通用曲线模式（非烘制形状）未按原生路径实现，拒绝"
                        .to_string(),
                ),
            },
            MinMaxCurve::TwoCurves { .. } => Err(
                "limitVelocity.magnitude: 双曲线模式未按原生路径实现，拒绝".to_string(),
            ),
        }
    }

    /// 每颗粒子的限速阈值。
    pub fn limit(&self, seed: u32, age_percent: f32) -> f32 {
        match self {
            MagnitudeLaw::Constant(v) => *v,
            MagnitudeLaw::TwoConstants { min, max } => {
                let r = clamp_lerp_hash(seed);
                (max - min) * r + min
            }
            MagnitudeLaw::Baked(b) => b.evaluate(normalized_age(age_percent)),
        }
    }
}

// ---- 拖拽 ----

/// 拖拽系数的取值律。只有常数模式按原生路径实现（语料全为常数 0）。
#[derive(Clone, Copy, Debug)]
pub enum DragLaw {
    /// 模式 0：常数。不读种子、不读年寿。
    Constant(f32),
}

/// 拖拽段的尺寸输入。引擎按容器标志选「出生/当前」尺寸数组（何时
/// 写、选哪组是消费侧的职责）；三分量与此处是否三维声明。
#[derive(Clone, Copy, Debug)]
pub struct DragSize {
    pub components: [f32; 3],
    pub size3d: bool,
}

// ---- maxps 语义 ----

/// maxps：任一非数取**源**操作数（Rust 的 `f32::max` 会吞掉非数，
/// 与引擎不同）。
fn maxps(dst: f32, src: f32) -> f32 {
    if dst.is_nan() || src.is_nan() {
        src
    } else {
        dst.max(src)
    }
}

/// 模块本体。inWorldSpace 在平面向量路径里不被读取（转储证实），
/// 不设字段。
#[derive(Clone, Debug)]
pub struct LimitVelocity {
    /// dampen <= 0（含非数）时钳制段整段跳过。
    pub dampen: f32,
    pub magnitude: MagnitudeLaw,
    /// None = 拖拽段跳过（drag == 0 的规范形态）。
    pub drag: Option<DragLaw>,
    /// 拖拽 × 粒子尺寸（半尺寸平方 × π）。构造时未知而拖拽非零已拒；
    /// 到达这里的 false 要么是数据真值、要么拖拽为零（惰性）。
    pub multiply_drag_by_size: bool,
    /// 拖拽 × 速度平方。同上。
    pub multiply_drag_by_velocity: bool,
}

impl LimitVelocity {
    /// 从文档语义构造。separateAxis 为真（轴ewise 族未实现，语料 0）
    /// 与拖拽非常数模式响亮拒绝。乘法标志提取面暂不导出（None =
    /// 未知）：拖拽为零时标志惰性（不进拖拽段），缺席可容；**拖拽
    /// 非零而标志未知**时拒绝——缺一个会左右结果的键不是当 false。
    pub fn from_parts(
        separate_axis: bool,
        magnitude: &MinMaxCurve,
        dampen: f32,
        drag: Option<&MinMaxCurve>,
        multiply_drag_by_size: Option<bool>,
        multiply_drag_by_velocity: Option<bool>,
    ) -> Result<Self, String> {
        if separate_axis {
            return Err("limitVelocity: separateAxis 轴ewise 族未按原生路径实现，拒绝".to_string());
        }
        let drag = match drag {
            None => None,
            Some(MinMaxCurve::Constant(v)) => Some(DragLaw::Constant(*v)),
            Some(_) => {
                return Err(
                    "limitVelocity.drag: 非常数模式未按原生路径实现（语料全为常数），拒绝"
                        .to_string(),
                )
            }
        };
        // 拖拽段会不会跑：门是 `drag != 0`（非数照跑）。
        let drag_runs = matches!(drag, Some(DragLaw::Constant(v)) if v != 0.0);
        if drag_runs && (multiply_drag_by_size.is_none() || multiply_drag_by_velocity.is_none()) {
            return Err(
                "limitVelocity: 拖拽非零而乘法标志未导出（尺寸×/速度×）——缺会左右结果的键，拒绝"
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

    /// 一步推进：钳制段 → 拖拽段（同帧同序）。`velocity` 是状态速度
    /// （就地改写）；`animated_velocity` 是同帧早于本模块算好的叠加
    /// 速度（两段都以「状态 + 叠加」的合速度为准，写回只改状态速度）。
    /// 拖拽段从**钳制后**的状态速度重算合速度（转储证实）。
    /// 非有限 dt 拒绝。
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
            return Err("limitVelocity: dt 非有限");
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

/// 钳制段主体（逐指令）：mag2 = x² + (y²+z²)；(0 < mag) 取**开方
/// 后、取绝对值前**的值；单位向量门 `1e-30 < mag2`；newmag 再乘
/// `(0 < mag)` 掩码。
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
    // 单位向量：引擎走 rsqrtps+两步牛顿（CPU 相关），此处精确除法
    // （具名偏差）。门 1e-30 对 mag2。
    let unit = if f32::from_bits(0x0da2_4260) < mag2 {
        [total[0] / mag, total[1] / mag, total[2] / mag]
    } else {
        [0.0, 0.0, 0.0]
    };
    let newmag = if mag_positive { newmag } else { 0.0 };
    velocity[0] = unit[0] * newmag - anim[0];
    velocity[1] = unit[1] * newmag - anim[1];
    velocity[2] = unit[2] * newmag - anim[2];
}

/// 拖拽段主体（逐指令）：尺寸 = 三声明取三分量最大否则取 x；
/// 尺寸因子 = (半·π)·半；速度因子 = mag2；factor 乘链
/// ((尺寸因子·drag)·速度因子)·dt；单位向量门 `1e-15 < mag`（对
/// mag，不是 mag2），**精确除法**（与钳制段不同，转储证实 divps）；
/// newmag = mag − factor 下钳 0（三元式，非 `f32::max`——maxps 的
/// 非数路返回源，三元式逐位一致，`f32::max` 不一致）。
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

// ---- agePercent 推进（引擎推进器内联段，与本模块同帧） ----

/// agePercent（0..100）一帧推进：
/// `new = old + (1/寿限)·(dt·100)`；模式 2 回绕（门
/// `fade_out <= new`，fade = loopRange·100，回绕量 = fade_out−fade_in；
/// 环形缓冲的活动道掩码 `activeCount > laneIndex` 是缓冲实现细节，
/// 不在此建模）；然后 `min(new, cap)`（模式 1 的 cap 略低）；
/// `100.0 < old` 时保持 old（过百不回落）。寿限倒数在引擎里是出生时
/// 写好的数组，写入点未读——此处按精确除法现算（具名）。
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
