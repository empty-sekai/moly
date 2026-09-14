//! 待待场景的步进序列律：一张标称时刻表，`dt` 显式入参推进。
//!
//! 标称时间轴：每步的 `t` == 此前各 `wait` 的 `seconds` 之和（数据
//! 333/333 场景零容差成立）。`wait` 不出事件——它是时间轴的承重，
//! 消费它的是「时钟过了没有」，不是「谁被触发了」。
//!
//! 场景步表与 tail 步表**拼接**成一张表：tail 每轮都跑，时间轴接
//! 在场景步表之后（[`schedule`]）。一段播完（全部步触发且时钟过
//! 尾）即回 idle，由宿主在下一拍重新选取——重选循环不在本律里。
//!
//! # 与执行宿主的分工
//!
//! 事件只携带数据面。源执行宿主的运行时门（宿主状态机处于 idle
//! 态才执行 eye/mouth/animation/emoticon；游戏状态 5 除外）做成
//! 可复算的纯谓词 [`op_executable`]，供消费侧判定；数值换算
//! （速度哨值 0 读作 1.0、等待时长毫秒化）是宿主方法体里逐值
//! 可读的式子，直接迁移为本模块的 [`effective_speed`] 与
//! [`wait_milliseconds`]。

use super::row::{Scenario, SegmentSuffix, Step};

/// 触发时刻比较的浮点容差。
///
/// 源的比较在剧本墙后读不到；数据里 `t` 全为整毫秒、时钟由调用方
/// 按 `dt` 累计，容差吸收累计误差。取同语义重建件的值。
pub const TIME_EPSILON: f32 = 1e-6;

/// 宿主状态机的 idle 态编号：eye/mouth/animation/emoticon/
/// hideEmoticon 只有在该态才执行；非该态静默丢弃。
///
/// 源执行门读到的字面量。换动作的带后缀变体**不查**此门。
pub const OP_STATE_TYPE_IDLE: i32 = 7;

/// 执行门排除的游戏状态类型：`IsEnableExecute` 对它返回假。
pub const EXCLUDED_GAME_STATE_TYPE: i32 = 5;

/// eye/mouth/animation/emoticon/hideEmoticon 的执行门。
///
/// `state_type` 是宿主状态机当前态、`game_state_type` 是游戏状态；
/// 双真才执行，否则源**静默丢弃**该拍（不补、不重排）。
/// 等待不走此门（源只查结束字典，游戏状态与宿主态都不看）。
pub fn op_executable(state_type: i32, game_state_type: i32) -> bool {
    state_type == OP_STATE_TYPE_IDLE && game_state_type != EXCLUDED_GAME_STATE_TYPE
}

/// 播放速度的哨值换算：源执行门读到 `0` 时按 `1.0` 播。
///
/// 手算锚：`0 -> 1.0`、`0.5 -> 0.5`、`1.0 -> 1.0`。
pub fn effective_speed(speed: f32) -> f32 {
    if speed == 0.0 {
        1.0
    } else {
        speed
    }
}

/// 等待时长的毫秒化：源取 `(int)(time * 1000)`——向零截断；
/// 非有限值（含无穷与 NaN）落到 `i32::MIN`（与源平台转换的
/// 「不确定值」一致）。
///
/// 手算锚：`0.1 -> 100`、`3.0 -> 3000`、`8.0 -> 8000`。
pub fn wait_milliseconds(seconds: f32) -> i32 {
    if seconds.is_finite() {
        (seconds * 1000.0) as i32
    } else {
        i32::MIN
    }
}

/// 一张待触发的时刻表项：步与其（拼接后）触发时刻。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledStep<'a> {
    pub step: &'a Step,
    pub t: f32,
}

/// 一段编排的标称长度（秒）：各步 `t + (wait 步加自身的 seconds)`
/// 的最大值；空表为 0。
///
/// 手算锚：`eye(t=0), wait(t=0, s=1.5), mouth(t=1.5), wait(t=1.5,
/// s=0.5), anim(t=2)` ⇒ `max(0, 1.5, 1.5, 2.0, 2.0) = 2.0`——
/// 尾部 `anim` 的 `t` 已把最后一段 wait 计入。
pub fn span(steps: &[Step]) -> f32 {
    steps
        .iter()
        .map(|s| match s {
            Step::Wait { t, seconds } => t + seconds,
            other => step_t(other),
        })
        .fold(0.0f32, f32::max)
}

fn step_t(step: &Step) -> f32 {
    match step {
        Step::ChangeEye { t, .. }
        | Step::ChangeMouth { t, .. }
        | Step::ChangeAnimation { t, .. }
        | Step::ShowEmoticon { t, .. }
        | Step::HideEmoticon { t }
        | Step::Wait { t, .. } => *t,
    }
}

/// 场景步表 + tail 步表拼接成一张时刻表：tail 各步的 `t` 平移
/// 场景步表的 [`span`]。
///
/// tail 每轮都跑；无 tail（空表）时表就是场景步表原样。
pub fn schedule<'a>(scenario: &'a Scenario, tail: &'a [Step]) -> Vec<ScheduledStep<'a>> {
    let base = span(&scenario.steps);
    let mut out: Vec<ScheduledStep<'a>> = scenario
        .steps
        .iter()
        .map(|step| ScheduledStep { step, t: step_t(step) })
        .collect();
    out.extend(
        tail.iter()
            .map(|step| ScheduledStep { step, t: step_t(step) + base }),
    );
    out
}

/// 一段编排触发的一拍。`wait` 无事件（时间轴承重，见模块文档）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepEvent<'a> {
    ChangeEye {
        t: f32,
        pattern: &'a str,
        alias: &'a str,
    },
    ChangeMouth {
        t: f32,
        pattern: &'a str,
        alias: &'a str,
    },
    ChangeAnimation {
        t: f32,
        motion: &'a str,
        alias: &'a str,
        phase: Option<SegmentSuffix>,
        /// 已过 [`effective_speed`] 换算的播放速度。
        speed: f32,
        playback_speed: f32,
        play_end_motion: bool,
        blend: f32,
    },
    ShowEmoticon {
        t: f32,
        name: &'a str,
        /// 到点由展示侧自收；与 hide 步非一一绑定（两通道独立）。
        show_seconds: f32,
    },
    HideEmoticon { t: f32 },
}

/// 序列推进的跨帧状态：已推进到的时钟与下一个未触发表项。
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceState {
    pub clock: f32,
    next: usize,
}

impl SequenceState {
    pub fn new() -> Self {
        Self { clock: 0.0, next: 0 }
    }
}

impl Default for SequenceState {
    fn default() -> Self {
        Self::new()
    }
}

/// 拒绝是响的：`dt` 为负或非数时本拍被拒——时钟与游标不动、
/// 无事件。源的时钟只产得出非负推进；非数一旦进时钟，之后的
/// 快照不再有可复算性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    NegativeDt,
    NonFiniteDt,
}

/// 推进一拍：时钟走 `dt`，触发所有到点表项，返回事件（表序）。
///
/// 到点判定：`t <= clock + [`TIME_EPSILON`]`。建段时先推 `dt = 0`
/// 即触发全部 `t == 0` 的首拍。
pub fn advance<'a>(
    scheduled: &[ScheduledStep<'a>],
    state: &mut SequenceState,
    dt: f32,
) -> Result<Vec<StepEvent<'a>>, Rejection> {
    if !dt.is_finite() {
        return Err(Rejection::NonFiniteDt);
    }
    if dt < 0.0 {
        return Err(Rejection::NegativeDt);
    }
    state.clock += dt;
    let mut events = Vec::new();
    while state.next < scheduled.len()
        && scheduled[state.next].t <= state.clock + TIME_EPSILON
    {
        let item = scheduled[state.next];
        state.next += 1;
        if let Some(event) = step_event(item.step, item.t) {
            events.push(event);
        }
    }
    Ok(events)
}

/// 一段播完：表项全部触发且时钟过表尾。
///
/// 手算锚：四步表（t=0,0,0,0；末步 wait 3 秒）⇒ span 3；`dt=2`
/// 后步全触发但未过尾 ⇒ 未完；再推 `dt=1` ⇒ 完。
pub fn is_finished(scheduled: &[ScheduledStep<'_>], state: &SequenceState) -> bool {
    state.next >= scheduled.len() && state.clock + TIME_EPSILON >= span_of(scheduled)
}

fn span_of(scheduled: &[ScheduledStep<'_>]) -> f32 {
    scheduled
        .iter()
        .map(|s| match s.step {
            Step::Wait { seconds, .. } => s.t + seconds,
            _ => s.t,
        })
        .fold(0.0f32, f32::max)
}

fn step_event<'a>(step: &'a Step, t: f32) -> Option<StepEvent<'a>> {
    match step {
        Step::ChangeEye { pattern, alias, .. } => Some(StepEvent::ChangeEye {
            t,
            pattern,
            alias,
        }),
        Step::ChangeMouth { pattern, alias, .. } => Some(StepEvent::ChangeMouth {
            t,
            pattern,
            alias,
        }),
        Step::ChangeAnimation {
            motion,
            alias,
            speed,
            playback_speed,
            play_end_motion,
            blend,
            phase,
            ..
        } => Some(StepEvent::ChangeAnimation {
            t,
            motion,
            alias,
            phase: *phase,
            speed: effective_speed(*speed),
            playback_speed: *playback_speed,
            play_end_motion: *play_end_motion,
            blend: *blend,
        }),
        Step::ShowEmoticon {
            name, show_seconds, ..
        } => Some(StepEvent::ShowEmoticon {
            t,
            name,
            show_seconds: *show_seconds,
        }),
        Step::HideEmoticon { .. } => Some(StepEvent::HideEmoticon { t }),
        Step::Wait { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::row::{TimeGatedTrigger, Trigger};

    fn eye(t: f32) -> Step {
        Step::ChangeEye {
            t,
            pattern: "normal".to_string(),
            alias: "synthetic".to_string(),
        }
    }

    fn mouth(t: f32) -> Step {
        Step::ChangeMouth {
            t,
            pattern: "smile01".to_string(),
            alias: "synthetic".to_string(),
        }
    }

    fn animation(t: f32, speed: f32) -> Step {
        Step::ChangeAnimation {
            t,
            motion: "mov_idle".to_string(),
            alias: "synthetic".to_string(),
            speed,
            playback_speed: 1.0,
            play_end_motion: false,
            blend: 0.5,
            phase: None,
            phase_source: None,
        }
    }

    fn wait(t: f32, seconds: f32) -> Step {
        Step::Wait { t, seconds }
    }

    fn emoticon(t: f32, show_seconds: f32) -> Step {
        Step::ShowEmoticon {
            t,
            name: "fx_emote_016_loop".to_string(),
            show_seconds,
            time_arg: Some(0.0),
            not_play_se_arg: Some(false),
            host_not_play_se: true,
        }
    }

    // ---- 换算 ----

    #[test]
    fn effective_speed_reads_zero_as_one() {
        // 手算锚（源执行门）：speed==0 -> 1.0；其余原样
        assert_eq!(effective_speed(0.0), 1.0);
        assert_eq!(effective_speed(0.5), 0.5);
        assert_eq!(effective_speed(1.0), 1.0);
        assert_eq!(effective_speed(2.0), 2.0);
    }

    #[test]
    fn wait_milliseconds_truncates_and_non_finite_is_int_min() {
        // 手算锚（源 (int)(time*1000)）：整秒直乘；0.1s 的 f32 乘积
        // 微高于 100 仍截断为 100
        assert_eq!(wait_milliseconds(0.1), 100);
        assert_eq!(wait_milliseconds(3.0), 3000);
        assert_eq!(wait_milliseconds(8.0), 8000);
        // 非有限（无穷与 NaN）-> i32::MIN（源平台转换的不确定值）
        assert_eq!(wait_milliseconds(f32::INFINITY), i32::MIN);
        assert_eq!(wait_milliseconds(f32::NAN), i32::MIN);
    }

    #[test]
    fn op_gate_requires_idle_state_and_excludes_game_state_five() {
        // 手算锚：==7 门 + 游戏状态 != 5
        assert!(op_executable(7, 0));
        assert!(op_executable(7, 5 - 1));
        assert!(!op_executable(7, 5), "游戏状态 5 排除");
        assert!(!op_executable(6, 0), "非 idle 态静默丢弃");
        assert!(!op_executable(0, 0));
    }

    // ---- span 与拼接 ----

    #[test]
    fn span_is_max_t_plus_wait_seconds() {
        // 手算锚：见 span 文档——尾 anim 的 t=2 已把最后一段 wait 计入
        let steps = vec![
            eye(0.0),
            wait(0.0, 1.5),
            mouth(1.5),
            wait(1.5, 0.5),
            animation(2.0, 0.0),
        ];
        assert_eq!(span(&steps), 2.0);
        assert_eq!(span(&[]), 0.0);
        // 末步是 wait 时 span = t + seconds
        let tail_like = vec![eye(0.0), wait(0.0, 3.0)];
        assert_eq!(span(&tail_like), 3.0);
    }

    #[test]
    fn schedule_appends_tail_behind_the_scenario_span() {
        // 手算锚：场景 span 2 + tail(t 全 0，末步 wait 3s)
        // -> tail 四步 t 全为 2；整表 span 5
        let scenario = Scenario {
            id: "sc".to_string(),
            trigger: Trigger::TimeGated(TimeGatedTrigger {
                time_limit_name: "synthetic".to_string(),
                time_limit_seconds: 0.0,
                probability_name: "synthetic".to_string(),
                probability: 1.0,
                motion_slot: None,
                slot_memory_seconds: 0.0,
            }),
            steps: vec![eye(0.0), wait(0.0, 2.0)],
        };
        let tail = vec![eye(0.0), mouth(0.0), animation(0.0, 0.0), wait(0.0, 3.0)];
        let scheduled = schedule(&scenario, &tail);
        assert_eq!(scheduled.len(), 6);
        assert_eq!(scheduled[0].t, 0.0);
        assert_eq!(scheduled[1].t, 0.0);
        for item in &scheduled[2..] {
            assert_eq!(item.t, 2.0, "tail 平移场景 span");
        }
        assert_eq!(span_of(&scheduled), 5.0);
        // 空 tail：表即场景步表
        let no_tail = schedule(&scenario, &[]);
        assert_eq!(no_tail.len(), 2);
    }

    // ---- 推进 ----

    #[test]
    fn advance_fires_due_steps_in_table_order() {
        // 手算锚：表 eye(0) wait(0,1.5) mouth(1.5) anim(2)
        // dt=0  -> eye；dt=1.4 -> 无；dt=0.1 -> clock 1.5 -> mouth
        // dt=0.5 -> clock 2.0 -> anim；wait 无事件
        let steps = vec![eye(0.0), wait(0.0, 1.5), mouth(1.5), animation(2.0, 0.0)];
        let scheduled: Vec<ScheduledStep> = steps
            .iter()
            .map(|step| ScheduledStep { step, t: step_t(step) })
            .collect();
        let mut state = SequenceState::new();

        let ev0 = advance(&scheduled, &mut state, 0.0).unwrap();
        assert_eq!(ev0.len(), 1);
        assert!(matches!(ev0[0], StepEvent::ChangeEye { t: 0.0, .. }));

        let ev1 = advance(&scheduled, &mut state, 1.4).unwrap();
        assert_eq!(ev1.len(), 0, "1.4 < 1.5 未到点");
        assert_eq!(state.clock, 1.4);

        let ev2 = advance(&scheduled, &mut state, 0.1).unwrap();
        assert_eq!(state.clock, 1.5);
        assert_eq!(ev2.len(), 1);
        assert!(matches!(ev2[0], StepEvent::ChangeMouth { t: 1.5, .. }));

        let ev3 = advance(&scheduled, &mut state, 0.5).unwrap();
        assert_eq!(ev3.len(), 1);
        assert!(matches!(ev3[0], StepEvent::ChangeAnimation { t: 2.0, .. }));
        // 事件携带换算后的速度：speed 哨值 0 -> 1.0
        match ev3[0] {
            StepEvent::ChangeAnimation { speed, .. } => assert_eq!(speed, 1.0),
            _ => unreachable!(),
        }
    }

    #[test]
    fn advance_is_idempotent_across_zero_dt() {
        // 同拍零推进：不重复触发已触发步
        let steps = vec![eye(0.0)];
        let scheduled: Vec<ScheduledStep> = steps
            .iter()
            .map(|step| ScheduledStep { step, t: step_t(step) })
            .collect();
        let mut state = SequenceState::new();
        advance(&scheduled, &mut state, 0.0).unwrap();
        let again = advance(&scheduled, &mut state, 0.0).unwrap();
        assert_eq!(again.len(), 0);
    }

    #[test]
    fn advance_rejects_negative_and_non_finite_dt_without_moving() {
        let scheduled: Vec<ScheduledStep> = vec![];
        let mut state = SequenceState::new();
        state.clock = 5.0;
        assert_eq!(
            advance(&scheduled, &mut state, -0.1),
            Err(Rejection::NegativeDt)
        );
        assert_eq!(
            advance(&scheduled, &mut state, f32::NAN),
            Err(Rejection::NonFiniteDt)
        );
        assert_eq!(
            advance(&scheduled, &mut state, f32::INFINITY),
            Err(Rejection::NonFiniteDt)
        );
        assert_eq!(state.clock, 5.0, "拒绝不动时钟");
    }

    #[test]
    fn finished_needs_all_steps_fired_and_clock_past_the_end() {
        // 手算锚：四步表 t 全 0（末步 wait 3s）⇒ span 3
        let steps = vec![eye(0.0), mouth(0.0), animation(0.0, 0.0), wait(0.0, 3.0)];
        let scheduled: Vec<ScheduledStep> = steps
            .iter()
            .map(|step| ScheduledStep { step, t: step_t(step) })
            .collect();
        let mut state = SequenceState::new();
        advance(&scheduled, &mut state, 2.0).unwrap();
        assert!(!is_finished(&scheduled, &state), "步全触发但时钟未过尾");
        advance(&scheduled, &mut state, 1.0).unwrap();
        assert!(is_finished(&scheduled, &state), "clock 3.0 == span 3 过尾");
    }

    #[test]
    fn emoticon_events_carry_show_seconds_and_fire_on_time() {
        let steps = vec![emoticon(0.6, 5.0)];
        let scheduled: Vec<ScheduledStep> = steps
            .iter()
            .map(|step| ScheduledStep { step, t: step_t(step) })
            .collect();
        let mut state = SequenceState::new();
        let ev = advance(&scheduled, &mut state, 0.0).unwrap();
        assert_eq!(ev.len(), 0, "0.6 > 0 未到点");
        let ev = advance(&scheduled, &mut state, 0.6).unwrap();
        match &ev[0] {
            StepEvent::ShowEmoticon {
                t,
                name,
                show_seconds,
            } => {
                assert_eq!(*t, 0.6);
                assert_eq!(*name, "fx_emote_016_loop");
                assert_eq!(*show_seconds, 5.0);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn hide_emoticon_fires_without_data() {
        let steps = vec![Step::HideEmoticon { t: 2.0 }];
        let scheduled: Vec<ScheduledStep> = steps
            .iter()
            .map(|step| ScheduledStep { step, t: step_t(step) })
            .collect();
        let mut state = SequenceState::new();
        let ev = advance(&scheduled, &mut state, 2.0).unwrap();
        assert!(matches!(ev[0], StepEvent::HideEmoticon { t: 2.0 }));
    }
}
