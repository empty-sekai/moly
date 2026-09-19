//! 粒子仿真律：Unity `ParticleSystem` 的对应语义，纯函数、dt 显式入参。
//!
//! 锚是**版本匹配的 Unity 引擎真源**（C# 参考源 + 行为实测），不是游戏反编译：
//! 站点粒子跑在引擎的 ParticleSystem 上，游戏侧只有参数数据。
//! 引用真源时只写类名/成员名与它说了什么，不写路径——坐标活在别处。
//!
//! # 出处档位（每条律的注释里标其一）
//!
//! | 档 | 含义 | 例 |
//! |---|---|---|
//! | `C# 可读` | UnityCsReference 里读得出逐式 | `MinMaxCurve.Evaluate` 的四模式 switch |
//! | `C# 可读（存在性）` | 只能证明字段/成员存在，读不出行为 | `PlaybackState.m_ToEmitAccumulator` |
//! | `行为口径` | 落在 extern 墙后（`Simulate`/`Evaluate` 全族原生） | 累加器取整规则、积分次序、死亡比较符 |
//! | `引擎原生逐指令` | extern 墙后的原生体已逐指令转录（常量按位钉死） | 角速度/限速族的采样与推进 |
//!
//! `行为口径`的条目按可复算的行为规格实现，并在注释里给 Editor 测量方案；
//! 未实测前它们是**带判据的假设**，不是已核事实。`引擎原生逐指令`的
//! 条目以逐值比对对齐（操作次序与常量位型逐条对齐原生体；库函数与
//! CPU 相关指令的逐位一致按构造不可达处，具名列出）。
//!
//! # 形状纪律
//!
//! 与 `path` 族同款：无引擎类型、无全局状态；随机一律显式 rand 入参
//! （同一个 rand 被几条律读是调用方的决定，律不自己抽签——一次取值被
//! 读两次不等于取两次值）。跨帧状态由调用方携带。
//! 「纯杂凑因子」不算抽签：角速度/限速族的因子是逐粒子种子的确定
//! 函数，每帧 0 次流抽取。

mod json;

pub mod value;
pub mod shape;
pub mod emit;
pub mod step;
pub mod buffer;
pub mod schema;
pub mod rotation;
pub mod limit_velocity;
pub mod velocity;
pub mod random;
pub mod curve;
pub mod size;
pub mod gradient;
pub mod color;

#[cfg(test)]
mod corpus;

pub use buffer::{compact, ring_push, RingBufferMode, RingPushVerdict};
pub use emit::{accumulate_rate, burst_check, Burst, BurstOutcome, EmissionState};
pub use limit_velocity::{advance_age_percent, DragLaw, DragSize, LimitVelocity, MagnitudeLaw};
pub use rotation::{BakedCurve, RotationOverLifetime};
pub use schema::{EmitterParams, Effects, EffectsError};
pub use shape::{circle_position, euler_rotate_deg};
pub use step::{advance_lifetime, apply_gravity, integrate, LifetimeVerdict, Particle,
    StepVerdict};
pub use value::{Curve, CurveKey, Gradient, GradientAlphaKey, GradientColorKey, MinMaxCurve,
    MinMaxGradient};
