//! 积分与生命周期律：一个粒子的单帧推进。
//!
//! 状态三样（位置、速度、剩余寿命）由调用方跨帧携带；dt 显式；
//! 帧内速度值（velocity-over-lifetime 等模块的输出）也由调用方求值后
//! 传入——本律不评估模块曲线，只管**怎么用**它。
//!
//! 存在性依据（`C# 可读`）：`Particle` 结构体的 `position`/`velocity`
//! /`remainingLifetime`/`startLifetime` 成员，与 `totalVelocity =
//! velocity + animatedVelocity`——后者是「位置由速度积分而来」的证据。
//! 推进次序与死亡比较符在 extern 墙后，行为口径逐条给测量方案。

use crate::particle::buffer::RingBufferMode;

/// 律侧粒子：积分所需的最小三元组 + 起始寿命（环形模式 2 的归一化
/// 年龄要用它折算）。渲染属性（颜色、尺寸）不进律——它们不参与
/// 积分，由消费侧按曲线求值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// 剩余寿命（秒），**倒计时**：`C# 可读`——真源 `Particle` 的
    /// `remainingLifetime` 读写的就是这个倒数，不是已活年龄。
    pub remaining_lifetime: f32,
    /// 起始寿命：出生时播种，此后只读（归一化年龄的分母）。
    pub start_lifetime: f32,
}

impl Particle {
    /// Normalized lifetime for modules and rendering. Infinite lifetime never
    /// progresses along the lifetime axis; explicit removal still owns teardown.
    pub fn normalized_age(&self) -> f32 {
        normalized_age(self.remaining_lifetime, self.start_lifetime)
    }

    /// 出生播种：寿命从 `start_lifetime` 起倒计时。
    pub fn born(position: [f32; 3], velocity: [f32; 3], start_lifetime: f32) -> Self {
        Self {
            position,
            velocity,
            remaining_lifetime: start_lifetime,
            start_lifetime,
        }
    }
}

/// The reciprocal lifetime of a never-expiring particle is zero. Computing
/// remaining / start directly would instead manufacture Infinity / Infinity.
pub fn normalized_age(remaining: f32, start: f32) -> f32 {
    if start == f32::INFINITY && remaining == f32::INFINITY {
        0.0
    } else {
        (1.0 - remaining / start).clamp(0.0, 1.0)
    }
}

/// 位置积分的裁决。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepVerdict {
    /// 位置本帧推进成功。
    Stepped,
    /// 输入带真源产不出的值：位置不动，速度不动。
    Refused,
}

/// 一帧的位置积分：半隐式欧拉——先定速度，再走位置。
///
/// `frame_velocity` 是本帧速度（调用方已按模块曲线求值）。档位：
/// **行为口径**。`totalVelocity` 可读出「速度驱动位置」（存在性），
/// 但「先速度后位置」的次序、以及 `velocityOverLifetime` 与重力的
/// 合成次序，都在 `Simulate` 原生侧。实现口径：半隐式欧拉
/// （位置用**已更新**的速度积分），这是引擎侧常见的粒子积分形，
/// 也是 legacy 前代实现的形状（线索，非锚）。
///
/// Editor 测量方案：`velocityOverLifetime` 恒 0、`gravityModifier=1`、
/// startLifetime=2 的系统，`Simulate(dt=0.5)` 步进 4 帧后逐帧读
/// `Particle.position`——半隐式欧拉第 1 帧 y=-g·dt²，显式欧拉为 0
/// （第 1 帧位置是否已受重力加速度影响即可分辨两种次序）。
pub fn integrate(p: &mut Particle, dt: f32, frame_velocity: [f32; 3]) -> StepVerdict {
    // `!(x >= 0.0)` 一个谓词抓负值与 NaN；非数位置/速度一旦写进状态，
    // 此后每一帧的推进都失去确定性，所以在入口拦。
    if !dt.is_finite() || !(dt >= 0.0)
        || !all_finite(p.position)
        || !all_finite(p.velocity)
        || !all_finite(frame_velocity)
    {
        return StepVerdict::Refused;
    }
    p.velocity = frame_velocity;
    for i in 0..3 {
        p.position[i] += p.velocity[i] * dt;
    }
    StepVerdict::Stepped
}

/// 重力积分：`velocity += gravity * gravity_modifier * dt`。
///
/// 档位：**行为口径**。`MainModule.gravityModifier` 属性可读（`C# 可读
/// （存在性）`），重力作为常加速度进入速度积分的方式原生。注意与
/// `velocityOverLifetime` 的合成次序未定（后者整帧覆写速度时重力
/// 贡献可能被冲掉）——律把两者拆开，合成是调用方的决定，次序问题
/// 在 REPORT 具名为未核。
///
/// Editor 测量方案：同 `integrate` 的方案——重力单独开、VoL 单独开、
/// 两者同开，各采一帧速度，即可判合成次序。
pub fn apply_gravity(p: &mut Particle, dt: f32, gravity: [f32; 3], gravity_modifier: f32) {
    if !dt.is_finite() || !(dt >= 0.0) || !all_finite(p.velocity) || !all_finite(gravity) {
        return;
    }
    for i in 0..3 {
        p.velocity[i] += gravity[i] * gravity_modifier * dt;
    }
}

/// 生命周期一帧推进的裁决。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LifetimeVerdict {
    /// 还活着，携带推进后的剩余寿命。
    Alive(f32),
    /// 死了（环形模式 Disabled 下的正常结局）：池要移除它。
    Died,
    /// 环形模式 1：走到寿命终点**停住等替换**，不移除、不复活。
    /// `C# 可读`（枚注释：到寿命终点暂停，直到被替换）。
    PausedAtEnd,
    /// 环形模式 2：到 fade-out 时刻**回卷到 fade-in 时刻**继续活。
    /// `C# 可读`（枚注释：到淡出时刻回卷到淡入时刻；被替换时先把
    /// 剩余寿命走完）。`loop_range` 是归一化寿命轴上的 [fade_in,
    /// fade_out]。
    Looped(f32),
}

/// 寿命倒计时推进与死亡门。
///
/// 档位：倒计时语义 `C# 可读`；**比较符与回卷的精确时刻行为口径**。
/// 实现口径：先扣 `dt`，模式非 2 时 `remaining <= 0` 判死（含恰为 0：
/// `<=`）。模式 2 的回卷门在**归一化年龄**轴上：`age = 1 -
/// remaining/start` 每帧判，越过 `loop_range[1]`（fade-out）即把剩余
/// 拨回 `(1 - loop_range[0]) * start`（fade-in）——不等自然寿命；
/// `loop_range = [0,1]`（语料全部 194 个发射器）时 fade-out 即寿命
/// 终点，行为退化为「永不死、整段循环」。越过门那帧的超冲量不结转
/// （回卷后从 fade-in 整步起算）。
///
/// Editor 测量方案：`startLifetime=1`、`dt=0.25` 步进 4 帧：第 4 帧后
/// `IsAlive` 应为假——恰在第 4 帧死亡钉住 `<=`（若第 5 帧才死，
/// 比较符是 `<`）。模式 2 置 `loopRange=[0.25,0.75]`、步进 60 帧，
/// 读任一存活粒子的归一化年龄序列：应反复从 0.75 跳回 0.25。
pub fn advance_lifetime(
    p: &mut Particle,
    dt: f32,
    mode: RingBufferMode,
    loop_range: [f32; 2],
) -> LifetimeVerdict {
    if !dt.is_finite() || !(dt >= 0.0) || !(p.start_lifetime > 0.0) || !loop_range_finite(loop_range) {
        // 拒绝时寿命不动。start_lifetime <= 0 的粒子造不出来（出生播种
        // 必带正寿命）；NaN 寿命同样在入口拦。
        return LifetimeVerdict::Alive(p.remaining_lifetime);
    }
    if p.start_lifetime == f32::INFINITY && p.remaining_lifetime == f32::INFINITY {
        return LifetimeVerdict::Alive(f32::INFINITY);
    }
    p.remaining_lifetime -= dt;
    if let RingBufferMode::LoopUntilReplaced = mode {
        let fade_out = loop_range[1].clamp(0.0, 1.0);
        let fade_in = loop_range[0].clamp(0.0, 1.0).min(fade_out);
        let age = p.normalized_age();
        if age >= fade_out {
            return LifetimeVerdict::Looped((1.0 - fade_in) * p.start_lifetime);
        }
        return LifetimeVerdict::Alive(p.remaining_lifetime);
    }
    if p.remaining_lifetime > 0.0 {
        LifetimeVerdict::Alive(p.remaining_lifetime)
    } else {
        match mode {
            RingBufferMode::Disabled => LifetimeVerdict::Died,
            RingBufferMode::PauseUntilReplaced => LifetimeVerdict::PausedAtEnd,
            // 上面已拦截模式 2；此处到不了。
            RingBufferMode::LoopUntilReplaced => LifetimeVerdict::Died,
        }
    }
}

/// 修正在模式 2 回卷时写回粒子的剩余寿命（`Looped` 携带的目标值）。
pub fn set_remaining(p: &mut Particle, remaining: f32) {
    p.remaining_lifetime = remaining;
}

fn all_finite(v: [f32; 3]) -> bool {
    v.iter().all(|&x| x.is_finite())
}

fn loop_range_finite(r: [f32; 2]) -> bool {
    r.iter().all(|&x| x.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEATH: RingBufferMode = RingBufferMode::Disabled;

    #[test]
    fn lifetime_counts_down_and_dies_on_the_frame_it_reaches_zero() {
        // 寿命 1.0、dt 0.25：第 4 帧扣到恰 0 -> 死（<= 口径），
        // 第 3 帧还活着。手算锚。
        let mut p = Particle::born([0.0; 3], [0.0; 3], 1.0);
        for frame in 1..=3 {
            assert!(matches!(
                advance_lifetime(&mut p, 0.25, DEATH, [0.0, 1.0]),
                LifetimeVerdict::Alive(_)
            ));
            assert!((p.remaining_lifetime - (1.0 - 0.25 * frame as f32)).abs() < 1e-6);
        }
        assert_eq!(advance_lifetime(&mut p, 0.25, DEATH, [0.0, 1.0]), LifetimeVerdict::Died);
    }

    #[test]
    fn one_large_step_kills_the_particle() {
        // dt 扣穿 0：直接死（不存在负寿命悬停）。
        let mut p = Particle::born([0.0; 3], [0.0; 3], 0.1);
        assert_eq!(advance_lifetime(&mut p, 1.0, DEATH, [0.0, 1.0]), LifetimeVerdict::Died);
    }

    #[test]
    fn mode_one_pauses_at_end_instead_of_dying() {
        // 枚注释口径：到寿命终点暂停等替换，不移除。
        let mut p = Particle::born([0.0; 3], [0.0; 3], 0.5);
        assert_eq!(
            advance_lifetime(&mut p, 0.5, RingBufferMode::PauseUntilReplaced, [0.0, 1.0]),
            LifetimeVerdict::PausedAtEnd
        );
        // 暂停期间寿命不再往下走（再推一帧还是 Paused，不是更负）。
        assert_eq!(
            advance_lifetime(&mut p, 0.5, RingBufferMode::PauseUntilReplaced, [0.0, 1.0]),
            LifetimeVerdict::PausedAtEnd
        );
    }

    #[test]
    fn mode_two_loops_full_range_forever() {
        // loop_range=[0,1]：fade_out 即寿命终点，回卷到 0 -> 永不死。
        // 走 5 个整寿命周期，每圈终点都拿到 Looped 且剩余拨回整段。
        let mut p = Particle::born([0.0; 3], [0.0; 3], 0.5);
        for cycle in 0..5 {
            let v = advance_lifetime(&mut p, 0.5, RingBufferMode::LoopUntilReplaced, [0.0, 1.0]);
            match v {
                LifetimeVerdict::Looped(remaining) => {
                    // 扣到恰 0：age=1 已到 fade_out=1 -> 拨回 fade_in=0，
                    // 剩余 = 整段寿命。
                    assert!((remaining - 0.5).abs() < 1e-6, "cycle {cycle}");
                    set_remaining(&mut p, remaining);
                }
                other => panic!("cycle {cycle}: expected Looped, got {other:?}"),
            }
        }
    }

    #[test]
    fn mode_two_inner_range_loops_before_natural_death() {
        // loop_range=[0.25,0.75]、寿命 1.0：age 到 0.75 即回卷到 0.25，
        // 不等自然寿命（此时剩余还有 0.25）。步长 0.1：第 8 步
        // （age 0.8）首次越过 0.75。
        let mut p = Particle::born([0.0; 3], [0.0; 3], 1.0);
        let mut verdicts = Vec::new();
        for _ in 0..8 {
            let v = advance_lifetime(&mut p, 0.1, RingBufferMode::LoopUntilReplaced, [0.25, 0.75]);
            verdicts.push(v);
            if let LifetimeVerdict::Looped(r) = v {
                set_remaining(&mut p, r);
            }
        }
        assert!(matches!(verdicts[7], LifetimeVerdict::Looped(_)), "{verdicts:?}");
        // 回卷目标：fade_in 0.25 -> 剩余 0.75。
        match verdicts[7] {
            LifetimeVerdict::Looped(r) => assert!((r - 0.75).abs() < 1e-6),
            _ => unreachable!(),
        }
    }

    #[test]
    fn integrate_moves_position_by_new_velocity() {
        // 半隐式：本帧速度 (0,-2,0)、dt 0.5 -> 位置走 (0,-1,0)。
        // 若实现是显式欧拉（用旧速度），出生速度非零时会走错——
        // 此锚专门钉次序。
        let mut p = Particle::born([0.0, 10.0, 0.0], [5.0, 5.0, 5.0], 1.0);
        assert_eq!(integrate(&mut p, 0.5, [0.0, -2.0, 0.0]), StepVerdict::Stepped);
        assert_eq!(p.position, [0.0, 9.0, 0.0]);
        assert_eq!(p.velocity, [0.0, -2.0, 0.0]);
    }

    #[test]
    fn integrate_composes_across_frames() {
        // 雨滴锚：VoL y=-0.75（Lerp(-0.5,-1.0,0.5) 的中值形状）、
        // 出生 (0,15,0)、dt 1/60：60 帧后 y = 15 - 0.75 = 14.25。
        let mut p = Particle::born([0.0, 15.0, 0.0], [0.0, -0.75, 0.0], 5.0);
        for _ in 0..60 {
            integrate(&mut p, 1.0 / 60.0, [0.0, -0.75, 0.0]);
        }
        assert!((p.position[1] - 14.25).abs() < 1e-4);
    }

    #[test]
    fn integrate_refuses_nan_and_negative_dt() {
        let mut p = Particle::born([0.0; 3], [1.0; 3], 1.0);
        let before = p;
        assert_eq!(integrate(&mut p, -0.1, [0.0; 3]), StepVerdict::Refused);
        assert_eq!(integrate(&mut p, f32::NAN, [0.0; 3]), StepVerdict::Refused);
        assert_eq!(
            integrate(&mut p, 0.1, [f32::NAN, 0.0, 0.0]),
            StepVerdict::Refused
        );
        assert_eq!(p, before);
        // NaN 位置同样拒——状态已被污染时不再推进。
        let mut bad = Particle::born([f32::NAN, 0.0, 0.0], [0.0; 3], 1.0);
        assert_eq!(integrate(&mut bad, 0.1, [0.0; 3]), StepVerdict::Refused);
    }

    #[test]
    fn gravity_adds_acceleration_to_velocity() {
        let mut p = Particle::born([0.0; 3], [0.0; 3], 5.0);
        apply_gravity(&mut p, 1.0 / 60.0, [0.0, -9.81, 0.0], 0.5);
        // 0.5 × −9.81 × 1/60 = −0.08175。
        assert!((p.velocity[1] + 0.08175).abs() < 1e-6);
        // dt=0 时速度不动（拒绝而非写 NaN）。
        apply_gravity(&mut p, f32::NAN, [0.0, -9.81, 0.0], 0.5);
        assert!((p.velocity[1] + 0.08175).abs() < 1e-6);
    }
}
