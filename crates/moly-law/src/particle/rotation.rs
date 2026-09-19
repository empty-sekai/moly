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

use crate::particle::value::MinMaxCurve;
pub use super::curve::{BakedCurve, CurveSampler as AngularVelocityLaw, eval_curve, normalized_age};
pub(crate) use super::random::hash_mix;

// ---- 杂凑核（常量按位钉死） ----

/// 乘法因子 M。
const HASH_M: u32 = 0x6ab5_1b9d;
/// lerp 因子的加项 T（原式）。
const HASH_T_LERP: u32 = 0x6aed_452e;
/// lerp 因子的加项 C（原式）。
const HASH_C_LERP: u32 = 0x00a0_1275;
/// 符号因子的加项 T。
const HASH_T_SIGN: u32 = 0xff2b_b1a4;
/// 符号因子的加项 C（原式）。
const HASH_C_SIGN: u32 = 0x0bc7_08d3;
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

// ---- 曲线求值（引擎原生逐指令） ----

impl AngularVelocityLaw {
    /// Sample the prepared curve with the rotation module's independent lerp
    /// and sign streams. Expanded and seed-shifted native hash forms are
    /// algebraically identical under wrapping u32 arithmetic.
    pub fn sample(&self, seed: u32, randomize_direction: f32, age_percent: f32) -> f32 {
        let value = self.evaluate(normalized_age(age_percent), hash_lerp_raw(seed));
        if randomize_direction < hash_sign_raw(seed) { value } else { -value }
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
