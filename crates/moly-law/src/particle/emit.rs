//! 发射律：率累加与 burst 排程。两者都是**跨帧状态 + dt 显式**的纯函数。
//!
//! 依据：`PlaybackState` 里可读出 `m_ToEmitAccumulator`（发射累加器）
//! 与 `m_PlaybackTime`（播头），`C# 可读（存在性）`——它们证明引擎用
//! 「累加小数、整帧吐出」的方式消化非整帧率，但取整与吐出的精确规则
//! 在 extern 墙后（`Simulate` 原生），行为口径逐条给测量方案。

use crate::particle::value::MinMaxCurve;

/// 率发射的跨帧状态：小数发射额的累计。
///
/// 字段名对齐真源 `PlaybackState` 的 `m_ToEmitAccumulator`
/// （`C# 可读（存在性）`）——引擎确有此累加器，是本律存在的证据。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmissionState {
    pub to_emit_accumulator: f32,
}

impl Default for EmissionState {
    fn default() -> Self {
        Self { to_emit_accumulator: 0.0 }
    }
}

/// 一帧的率发射量：`carry += rate * dt`，向下取整吐出，余数留仓。
///
/// 档位：**行为口径**。累加器存在可读，**取整规则与余数处理不可读**。
/// 实现口径：floor（率恒非负，floor 与 truncate 同值；负率在解析层
/// 已拒）。
///
/// Editor 测量方案：rate=1、以 `Simulate(dt=0.1)` 步进 30 帧，记录
/// 粒子出生帧号——应恰在第 10/20/30 帧各生 1 个（carry 0.1→…→1.0
/// 的整点）。再以 rate=7、dt=1/30 步进，数每帧出生数应为 0 或 1
/// 交替（carry 模式 7/30），若出现一帧 2 个则取整规则不是本口径。
pub fn accumulate_rate(state: &mut EmissionState, rate: f32, dt: f32) -> u32 {
    if !(rate >= 0.0) || !(dt >= 0.0) {
        // 与 path 族同款防卫：`!(x >= 0.0)` 一个谓词同时抓负值与 NaN。
        // 拒绝时不吐粒子也不动仓——坏输入毁掉的是确定性，不是本帧。
        return 0;
    }
    state.to_emit_accumulator += rate * dt;
    let whole = state.to_emit_accumulator.floor();
    state.to_emit_accumulator -= whole;
    whole as u32
}

/// 一个 burst。序列化口径直接存用户面（`cycleCount`、`probability`）；
/// 引擎内部另有两处编码差，`C# 可读`：
///
/// - `m_RepeatCount = cycleCount - 1`（结构体默认值要 0，用户口径要 1）；
/// - `m_InvProbability = 1 - probability`（序列化存的是补数）。
///
/// schema 层按用户面存，两处编码差在此具名，防止下游拿补数当概率。
#[derive(Debug, Clone, PartialEq)]
pub struct Burst {
    /// burst 时刻（秒，播头时间轴）。
    pub time: f32,
    /// burst 数量。求值在触发时发生，`lerp_factor` 用显式 rand。
    pub count: MinMaxCurve,
    /// 总触发次数（用户口径，≥1）。
    pub cycles: u32,
    /// 重复间隔。`> 0` 才有第 2 次及以后的触发（`C# 可读`：
    /// 间隔的 setter 拒绝非正值）。
    pub repeat_interval: f32,
    /// 触发概率 p ∈ [0,1]。p=1 恒触发；引擎内部存 1−p。
    pub probability: f32,
}

/// 一帧的 burst 裁决。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BurstOutcome {
    /// 本帧无 burst 到点。
    NotDue,
    /// 到点且概率命中：携带触发数量。
    Fired(u32),
    /// 到点但概率未命中：引擎仍消耗这一次触发机会。
    MissedByProbability,
}

/// 判一个 burst 在 `[prev_time, now_time]` 帧内是否到点。
///
/// 档位：触发条件**行为口径**。实现口径：第 k 次触发时刻
/// `t_k = time + k * repeat_interval`（k≥1 需要 `repeat_interval > 0`，
/// 与真源「间隔为 0 只有单发」一致，`C# 可读`），到点判
/// `prev_time < t_k <= now_time`——**帧末跨界**：恰在 now_time 的
/// 触发算本帧，恰在 prev_time 的算上一帧。概率门用显式
/// `rand_fire`，数量用显式 `rand_count`——两个 rand 分开传，因为
/// 真源里概率判定与数量插值是不是同一次抽样**读不出来**，分开传
 /// 让调用方钉自己的口径，律不替它猜。
///
/// Editor 测量方案：单 burst time=0.5，以 `Simulate(dt=0.25)` 步进，
/// 第 2 帧末（now=0.5）应触发——验证 `<=` 还是 `<`；`cycles=3`、
/// `repeatInterval=1` 的 burst 在 60 帧内应恰触发 3 次；`probability=0.5`
/// 的 burst 跑 200 个种子，触发占比应近 1/2。
pub fn burst_check(
    burst: &Burst,
    prev_time: f32,
    now_time: f32,
    rand_fire: f32,
    rand_count: f32,
) -> BurstOutcome {
    if !(now_time >= prev_time) || prev_time.is_nan() || now_time.is_nan() {
        // 播头回卷是调用方的循环语义，律不猜；时间非数同拒。
        // NaN 播头不会静默当 NotDue：`(0..cycles)` 的 any 比较全假
        // 也会到 NotDue，但显式拦下是因为 NaN 进时间轴是状态损坏。
        return BurstOutcome::NotDue;
    }
    let hits = (0..burst.cycles.max(1)).any(|k| {
        let t_k = if k == 0 {
            burst.time
        } else if burst.repeat_interval > 0.0 {
            burst.time + k as f32 * burst.repeat_interval
        } else {
            // 间隔非正：只保第 0 次触发（真源 setter 拒绝 interval<=0，
            // 序列化数据不会带着它进来，此处是双保险）。
            return false;
        };
        prev_time < t_k && t_k <= now_time
    });
    if !hits {
        return BurstOutcome::NotDue;
    }
    // 概率门：`rand < p` 过。p=1 特判恒过——引擎把 p 存成补数 1-p=0，
    // rand=1 的极端样本撞 `1 < 1` 为假，但 p=1 的用户语义是「必发」，
    // 单点边界不该翻掉整条语义（rand 恰为 1 本身是数域闭端点的针尖）。
    // p=0 恒不过（0 < 0 假）。Editor 可测：p=0.5 跑 200 种子，
    // 触发占比应近 1/2——单点边界不改变统计形状。
    if rand_fire < burst.probability || burst.probability >= 1.0 {
        let count = burst.count.evaluate(burst.time, rand_count);
        // burst 数量是非负整数语义；负值是数据损伤，钳到 0 响亮可见。
        BurstOutcome::Fired(count.max(0.0) as u32)
    } else {
        BurstOutcome::MissedByProbability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_burst(time: f32) -> Burst {
        Burst {
            time,
            count: MinMaxCurve::Constant(3.0),
            cycles: 1,
            repeat_interval: 0.0,
            probability: 1.0,
        }
    }

    #[test]
    fn rate_accumulator_emits_on_whole_frames() {
        // rate=1、dt=0.1：carry 0.1..0.9 不吐，1.0 整点吐 1、清仓。
        let mut s = EmissionState::default();
        let mut born = 0;
        for _ in 0..9 {
            born += accumulate_rate(&mut s, 1.0, 0.1);
        }
        assert_eq!(born, 0);
        assert!((s.to_emit_accumulator - 0.9).abs() < 1e-6);
        assert_eq!(accumulate_rate(&mut s, 1.0, 0.1), 1);
        assert!(s.to_emit_accumulator.abs() < 1e-6);
    }

    #[test]
    fn rate_accumulator_carry_survives_zero_rate_frames() {
        // 余数跨零率帧存活：仓里的 0.2 不会因一帧没发射而清掉。
        let mut s = EmissionState::default();
        accumulate_rate(&mut s, 2.0, 0.1); // carry 0.2
        assert_eq!(accumulate_rate(&mut s, 0.0, 0.5), 0);
        assert!((s.to_emit_accumulator - 0.2).abs() < 1e-6);
        assert_eq!(accumulate_rate(&mut s, 8.0, 0.1), 1); // 0.2+0.8=1.0
    }

    #[test]
    fn rate_accumulator_f32_carry_lands_exactly() {
        // f32(0.1) 略大于 1/10：十个累加得 1.0000001192092896，
        // 第 10 帧就到整点（floor=1.0、仓清 0）。手算（IEEE f32
        // 逐加），把「残差向上、提前到点」钉住——换累加口径必红。
        let mut s = EmissionState::default();
        let mut total = 0u32;
        for _ in 0..10 {
            total += accumulate_rate(&mut s, 1.0, 0.1);
        }
        assert_eq!(total, 1);
        assert!(s.to_emit_accumulator.abs() < 1e-6);
        // 第 11 帧：仓已清，0.1 起步。
        assert_eq!(accumulate_rate(&mut s, 1.0, 0.1), 0);
        assert!((s.to_emit_accumulator - 0.1).abs() < 1e-6);
    }

    #[test]
    fn rate_accumulator_rejects_negative_and_nan() {
        let mut s = EmissionState::default();
        assert_eq!(accumulate_rate(&mut s, -1.0, 0.1), 0);
        assert_eq!(accumulate_rate(&mut s, f32::NAN, 0.1), 0);
        assert_eq!(accumulate_rate(&mut s, 1.0, f32::NAN), 0);
        assert_eq!(s.to_emit_accumulator, 0.0);
    }

    #[test]
    fn burst_fires_on_frame_end_boundary() {
        // time=0.5、帧 [0.25, 0.5]：t_k 恰等于 now -> 本帧触发（<= 口径）。
        let b = single_burst(0.5);
        assert_eq!(
            burst_check(&b, 0.25, 0.5, 0.0, 0.0),
            BurstOutcome::Fired(3)
        );
        // 上一帧 [0.25, 0.4999]：未到点。
        assert_eq!(burst_check(&b, 0.25, 0.4999, 0.0, 0.0), BurstOutcome::NotDue);
        // 帧首恰等于 t_k：算上一帧的（prev < t_k 严格不等）。
        assert_eq!(burst_check(&b, 0.5, 0.75, 0.0, 0.0), BurstOutcome::NotDue);
    }

    #[test]
    fn burst_repeats_only_with_positive_interval() {
        let b = Burst {
            time: 0.0,
            count: MinMaxCurve::Constant(1.0),
            cycles: 3,
            repeat_interval: 2.0,
            probability: 1.0,
        };
        // 触发时刻 0 / 2 / 4：帧 [1.5, 2.0] 与 [3.5, 4.0] 各一次，
        // [0.5, 1.5] 没有。
        assert_eq!(burst_check(&b, 0.5, 1.5, 0.0, 0.0), BurstOutcome::NotDue);
        assert_eq!(burst_check(&b, 1.5, 2.0, 0.0, 0.0), BurstOutcome::Fired(1));
        assert_eq!(burst_check(&b, 3.5, 4.0, 0.0, 0.0), BurstOutcome::Fired(1));
        // cycles 之外的重复（t=6）不再触发。
        assert_eq!(burst_check(&b, 5.5, 6.5, 0.0, 0.0), BurstOutcome::NotDue);

        let zero_interval = Burst { repeat_interval: 0.0, cycles: 5, ..b };
        // 间隔 0：无论 cycles 多大，只保第 0 次。
        assert_eq!(burst_check(&zero_interval, -0.5, 0.5, 0.0, 0.0), BurstOutcome::Fired(1));
        assert_eq!(burst_check(&zero_interval, 0.5, 5.5, 0.0, 0.0), BurstOutcome::NotDue);
    }

    #[test]
    fn burst_probability_gate_uses_rand() {
        // p=0.25：rand<0.25 过、rand=0.25 不过（严格小于口径）。
        let b = Burst { probability: 0.25, ..single_burst(0.0) };
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.1, 0.0), BurstOutcome::Fired(3));
        assert_eq!(
            burst_check(&b, -0.1, 0.1, 0.25, 0.0),
            BurstOutcome::MissedByProbability
        );
        assert_eq!(
            burst_check(&b, -0.1, 0.1, 0.99, 0.0),
            BurstOutcome::MissedByProbability
        );
        // p=1 恒过（rand=1 也过——引擎存补数 1-p=0，比较是严格小于）。
        let always = single_burst(0.0);
        assert_eq!(burst_check(&always, -0.1, 0.1, 1.0, 0.0), BurstOutcome::Fired(3));
        // p=0：rand=0 也不过（0 < 0 为假）。
        let never = Burst { probability: 0.0, ..single_burst(0.0) };
        assert_eq!(
            burst_check(&never, -0.1, 0.1, 0.0, 0.0),
            BurstOutcome::MissedByProbability
        );
    }

    #[test]
    fn burst_count_two_constants_uses_lerp() {
        // count TwoConstants{min=2,max=6}，rand_count=0.5 -> 4。
        let b = Burst {
            count: MinMaxCurve::TwoConstants { min: 2.0, max: 6.0 },
            ..single_burst(0.0)
        };
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.0, 0.5), BurstOutcome::Fired(4));
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.0, 0.0), BurstOutcome::Fired(2));
    }

    #[test]
    fn burst_inverted_playhead_is_refused() {
        let b = single_burst(0.5);
        assert_eq!(burst_check(&b, 1.0, 0.5, 0.0, 0.0), BurstOutcome::NotDue);
        assert_eq!(burst_check(&b, f32::NAN, 0.5, 0.0, 0.0), BurstOutcome::NotDue);
    }
}
