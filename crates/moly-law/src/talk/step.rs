//! talk 步进流律：一条调用流的推进——`dt` 显式入参，驻留与点跳显式。
//!
//! 源形态：剧本是 lua 协程，逐行调用宿主（`MySekaiTalkEngine`）的
//! 入口；`wait_time`/`wait_click` 系入口让出协程把流挂起，其余入口
//! 即发即过。提取产物把这段流记成**原序调用流**（无运行时刻），本
//! 律在同一形态上重建推进：
//! - 即时步发射即过（`look_at_body` 的 `duration` 是转身时长，随
//!   事件带出但**不驻留**——源剧本库的驻留形调用在本语料 0 次出现，
//!   流的驻留只来自等待步）；
//! - [`Hold::Time`] 驻留到时长耗尽；源 `WaitTime(time, enableClickSkip)`
//!   的点跳臂是延时与点击的 WhenAny——点跳开关真时点击提前放行；
//! - [`Hold::Click`] 挂起直至点击（源 `WaitClick`）；UI等待真正完成后，
//!   宿主先冲刷延迟命令队列，再执行后续脚本语句；
//! - 每次调用至多消费一次点击：点击放行挂起后，同拍内后续新挂起
//!   不再被同一击放行（源点击等待注册于挂起之后，不吃旧击）。
//!
//! 事件只带数据面：推进返回**触发步的下标**，载荷由行枚举自带的
//! 分发方法读取（[`StepOp::op`]、[`StepOp::face_change`]）。eye/mouth
//! 图样键即 facial 两表键，与 `crate::tweet::face` 对齐。

use super::row::{FixtureStep, TalkStep};

/// 到点比较的浮点容差（时长由 `dt` 累计，容差吸收累计误差）。
pub const TIME_EPSILON: f32 = 1e-6;

/// 一驻留请求：等待步对流的驻留语义。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    /// 即发即过。
    None,
    /// 驻留到时长耗尽；`click_skippable` 为真时点击可提前放行。
    Time {
        seconds: f32,
        click_skippable: bool,
    },
    /// 挂起直至点击，无时长上限。
    Click,
}

/// 步的流语义：驻留与锚点。两套提取形各实现一遍。
pub trait StepOp {
    fn hold(&self) -> Hold;
    /// 跳转锚名（仅 label 步）。
    fn label_name(&self) -> Option<&str>;
    /// 源调用名（提取词表原样）。
    fn op(&self) -> &'static str;
    /// 眼/口图样切换的载荷：(槽位, 图样键, 源记号)。非眼口步为空。
    fn face_change(&self) -> Option<(FaceSlot, &str, &str)>;
}

/// 眼/口槽位（对齐 `crate::tweet::face` 的 eye/mouth 两键）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceSlot {
    Eye,
    Mouth,
}

/// `wait_time` 第二参（点跳开关）到驻留点跳性的映射：缺参按源剧本
/// 库缺省真处理。
///
/// 提取字段名是 `auto`，按原样第二参读入——`None` 是调用缺参，
/// `Some(v)` 是显式实参。源语义是 `enableClickSkip`（真 = 点击可
/// 跳）；字段名与源语义的对应在提取侧无独立佐证，这是当前证据下
/// 的一致读法，改读法只动本函数。
pub fn wait_time_click_skippable(auto: Option<bool>) -> bool {
    auto.unwrap_or(true)
}

/// 动作速度的哨值换算：源脚本缺省 1.0（未传参），0 同样按 1.0 播
/// （提取语义注记：速度为零时播放速度用 1.0）。
///
/// 手算锚：`None -> 1.0`、`0 -> 1.0`、`0.5 -> 0.5`。
pub fn effective_animation_speed(speed: Option<f64>) -> f64 {
    match speed {
        None => 1.0,
        Some(s) if s == 0.0 => 1.0,
        Some(s) => s,
    }
}

/// 等待时长的毫秒化：源宿主取 `(int)(time * 1000)`——向零截断；
/// 非有限值落 `i32::MIN`（源对无穷显式回最小值；负无穷与 NaN 在
/// 目标平台转换同落最小值）。
///
/// 手算锚：`0.1 -> 100`、`3.0 -> 3000`。
pub fn wait_milliseconds(seconds: f64) -> i32 {
    let ms = seconds * 1000.0;
    if ms.is_finite() {
        ms as i32
    } else {
        i32::MIN
    }
}

impl StepOp for TalkStep {
    fn hold(&self) -> Hold {
        match self {
            TalkStep::WaitTime { seconds, auto } => Hold::Time {
                seconds: *seconds as f32,
                click_skippable: wait_time_click_skippable(*auto),
            },
            TalkStep::WaitClick => Hold::Click,
            // The player window currently exposes manual mode: the source
            // waits for either this delay or WaitClicked when auto is off.
            TalkStep::WaitTimeOnAutoMode { seconds } => Hold::Time {
                seconds: *seconds as f32,
                click_skippable: true,
            },
            _ => Hold::None,
        }
    }

    fn label_name(&self) -> Option<&str> {
        match self {
            TalkStep::Label { name } => Some(name),
            _ => None,
        }
    }

    fn op(&self) -> &'static str {
        match self {
            TalkStep::LookAtBody { .. } => "look_at_body",
            TalkStep::WaitTime { .. } => "wait_time",
            TalkStep::Label { .. } => "label",
            TalkStep::Voice { .. } => "voice",
            TalkStep::ChangeNpcEye { .. } => "change_npc_eye",
            TalkStep::ChangeNpcMouth { .. } => "change_npc_mouth",
            TalkStep::ChangeAnimation { .. } => "change_animation",
            TalkStep::Text { .. } => "text",
            TalkStep::WaitClick => "wait_click",
            TalkStep::Emoticon { .. } => "emoticon",
            TalkStep::HideEmoticon { .. } => "hide_emoticon",
            TalkStep::ShowTalkWindow => "show_talk_window",
            TalkStep::HideTalkWindow => "hide_talk_window",
            TalkStep::WaitTimeOnAutoMode { .. } => "wait_time_on_auto_mode",
        }
    }

    fn face_change(&self) -> Option<(FaceSlot, &str, &str)> {
        match self {
            TalkStep::ChangeNpcEye { pattern, alias, .. } => {
                Some((FaceSlot::Eye, pattern, alias))
            }
            TalkStep::ChangeNpcMouth { pattern, alias, .. } => {
                Some((FaceSlot::Mouth, pattern, alias))
            }
            _ => None,
        }
    }
}

impl StepOp for FixtureStep {
    fn hold(&self) -> Hold {
        match self {
            FixtureStep::WaitTime { seconds, auto } => Hold::Time {
                seconds: *seconds as f32,
                click_skippable: wait_time_click_skippable(*auto),
            },
            FixtureStep::WaitClick => Hold::Click,
            // Same manual-mode branch as the ordinary script representation.
            FixtureStep::WaitTimeOnAutoMode { seconds } => Hold::Time {
                seconds: *seconds as f32,
                click_skippable: true,
            },
            _ => Hold::None,
        }
    }

    fn label_name(&self) -> Option<&str> {
        match self {
            FixtureStep::Label { name } => Some(name),
            _ => None,
        }
    }

    fn op(&self) -> &'static str {
        match self {
            FixtureStep::LookAtBody { .. } => "look_at_body",
            FixtureStep::WaitTime { .. } => "wait_time",
            FixtureStep::Label { .. } => "label",
            FixtureStep::Voice { .. } => "voice",
            FixtureStep::ChangeNpcEye { .. } => "change_npc_eye",
            FixtureStep::ChangeNpcMouth { .. } => "change_npc_mouth",
            FixtureStep::ChangeAnimation { .. } => "change_animation",
            FixtureStep::Text { .. } => "text",
            FixtureStep::WaitClick => "wait_click",
            FixtureStep::Emoticon { .. } => "emoticon",
            FixtureStep::HideEmoticon { .. } => "hide_emoticon",
            FixtureStep::ShowTalkWindow => "show_talk_window",
            FixtureStep::HideTalkWindow => "hide_talk_window",
            FixtureStep::WaitTimeOnAutoMode { .. } => "wait_time_on_auto_mode",
            FixtureStep::LookAtFixture { .. } => "look_at_fixture",
            FixtureStep::LookAtToNpc { .. } => "look_at_to_npc",
            FixtureStep::FixtureVoice { .. } => "fixture_voice",
            FixtureStep::ChangeFixtureCharacterEye { .. } => "change_fixture_character_eye",
            FixtureStep::ChangeFixtureCharacterMouth { .. } => "change_fixture_character_mouth",
            FixtureStep::ChangeFixtureTimeline { .. } => "change_fixture_timeline",
            FixtureStep::ShowFixtureEmoticon { .. } => "show_fixture_emoticon",
            FixtureStep::PlayFixtureGimmick { .. } => "play_fixture_gimmick",
            FixtureStep::StopFixtureGimmick { .. } => "stop_fixture_gimmick",
        }
    }

    fn face_change(&self) -> Option<(FaceSlot, &str, &str)> {
        match self {
            FixtureStep::ChangeNpcEye { pattern, alias, .. } => {
                Some((FaceSlot::Eye, pattern, alias))
            }
            FixtureStep::ChangeNpcMouth { pattern, alias, .. } => {
                Some((FaceSlot::Mouth, pattern, alias))
            }
            _ => None,
        }
    }
}

/// 挂起中的活驻留。
#[derive(Debug, Clone, Copy, PartialEq)]
enum ActiveHold {
    Time {
        remaining: f32,
        click_skippable: bool,
    },
    Click,
}

/// 推进的跨帧状态：已消费到的游标与挂起中的驻留。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamState {
    pub cursor: usize,
    hold: Option<ActiveHold>,
}

impl StreamState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否有挂起未放行。
    pub fn is_holding(&self) -> bool {
        self.hold.is_some()
    }
}

/// 拒绝是响的：`dt` 为负或非数时本拍被拒——游标与挂起不动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    NegativeDt,
    NonFiniteDt,
}

/// 推进一拍：时钟走 `dt`，返回本拍触发步的下标（表序）。
///
/// - 挂起中：先扣 `dt`；时长耗尽或（点跳开关真且）`click` 为真则
///   放行并继续发射；点击放行后该击即耗尽，同拍内新挂起不再吃击。
/// - 放行后逐表推进：即时步发射游标即过；等待步进挂起（零时长
///   挂起也在下一拍才放行）。
pub fn advance<S: StepOp>(
    steps: &[S],
    state: &mut StreamState,
    dt: f32,
    click: bool,
) -> Result<Vec<usize>, Rejection> {
    if !dt.is_finite() {
        return Err(Rejection::NonFiniteDt);
    }
    if dt < 0.0 {
        return Err(Rejection::NegativeDt);
    }
    // 挂起侧：扣时、判放行。该击只服务本次放行——发射侧的新挂起
    // 不查击（源点击等待注册于挂起之后，不吃旧击）。
    if let Some(active) = state.hold {
        let release = match active {
            ActiveHold::Time {
                mut remaining,
                click_skippable,
            } => {
                remaining -= dt;
                let done = remaining <= TIME_EPSILON;
                if done || (click && click_skippable) {
                    Some(done)
                } else {
                    state.hold = Some(ActiveHold::Time {
                        remaining,
                        click_skippable,
                    });
                    None
                }
            }
            ActiveHold::Click => {
                if click {
                    Some(true)
                } else {
                    None
                }
            }
        };
        match release {
            Some(_) => state.hold = None,
            None => return Ok(Vec::new()),
        }
    }
    // 发射侧：即时步连发，等待步挂起即停
    let mut fired = Vec::new();
    while state.cursor < steps.len() {
        let step = &steps[state.cursor];
        match step.hold() {
            Hold::None => {
                fired.push(state.cursor);
                state.cursor += 1;
            }
            Hold::Time {
                seconds,
                click_skippable,
            } => {
                state.cursor += 1;
                state.hold = Some(ActiveHold::Time {
                    remaining: seconds,
                    click_skippable,
                });
                break;
            }
            Hold::Click => {
                state.cursor += 1;
                state.hold = Some(ActiveHold::Click);
                break;
            }
        }
    }
    Ok(fired)
}

/// 一段流播完：全部步已消费且无挂起。
pub fn is_finished(steps_len: usize, state: &StreamState) -> bool {
    state.cursor >= steps_len && !state.is_holding()
}

/// 跳转锚解析：具名锚首次出现的下标。源把 label 注册到对话视图供
/// 跳转消费；流内无跳转调用（语料零出现），本律只供锚点解析。
pub fn label_anchor<S: StepOp>(steps: &[S], name: &str) -> Option<usize> {
    steps.iter().position(|s| s.label_name() == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talk_text() -> TalkStep {
        TalkStep::Text {
            text: "synthetic".to_string(),
        }
    }

    fn talk_wait(seconds: f64, auto: Option<bool>) -> TalkStep {
        TalkStep::WaitTime { seconds, auto }
    }

    fn talk_look() -> TalkStep {
        TalkStep::LookAtBody {
            who: "a".to_string(),
            target: "b".to_string(),
            duration: 0.5,
        }
    }

    // ---- 换算 ----

    #[test]
    fn click_skippable_defaults_true_and_reads_explicit_arg() {
        // 手算锚：缺参 -> 源剧本库缺省真；显式实参原样
        assert!(wait_time_click_skippable(None));
        assert!(wait_time_click_skippable(Some(true)));
        assert!(!wait_time_click_skippable(Some(false)));
    }

    #[test]
    fn animation_speed_sentinels_read_as_one() {
        // 手算锚：未传参与零速都按 1.0 播
        assert_eq!(effective_animation_speed(None), 1.0);
        assert_eq!(effective_animation_speed(Some(0.0)), 1.0);
        assert_eq!(effective_animation_speed(Some(0.5)), 0.5);
        assert_eq!(effective_animation_speed(Some(2.0)), 2.0);
    }

    #[test]
    fn wait_milliseconds_truncates_and_non_finite_is_int_min() {
        // 手算锚（源 (int)(time*1000)）：0.1s -> 100；3s -> 3000
        assert_eq!(wait_milliseconds(0.1), 100);
        assert_eq!(wait_milliseconds(3.0), 3000);
        // 非有限 -> i32::MIN（源对无穷显式回最小值）
        assert_eq!(wait_milliseconds(f64::INFINITY), i32::MIN);
        assert_eq!(wait_milliseconds(f64::NEG_INFINITY), i32::MIN);
        assert_eq!(wait_milliseconds(f64::NAN), i32::MIN);
    }

    // ---- 驻留语义 ----

    #[test]
    fn wait_steps_carry_hold_semantics() {
        // 手算锚：wait_time 点跳开关按参；当前手动模式可跳时间等待；wait_click 挂起到击
        assert_eq!(
            talk_wait(1.5, Some(false)).hold(),
            Hold::Time {
                seconds: 1.5,
                click_skippable: false
            }
        );
        assert_eq!(
            talk_wait(1.5, None).hold(),
            Hold::Time {
                seconds: 1.5,
                click_skippable: true
            }
        );
        assert_eq!(TalkStep::WaitClick.hold(), Hold::Click);
        assert_eq!(
            TalkStep::WaitTimeOnAutoMode { seconds: 2.0 }.hold(),
            Hold::Time {
                seconds: 2.0,
                click_skippable: true
            }
        );
        // 即时步零驻留：look_at_body 发射即过，duration 只随载荷
        assert_eq!(talk_look().hold(), Hold::None);
        assert_eq!(talk_text().hold(), Hold::None);
    }

    #[test]
    fn fixture_wait_steps_hold_identically() {
        let steps = [
            FixtureStep::WaitTime {
                seconds: 1.0,
                auto: None,
            },
            FixtureStep::WaitClick,
            FixtureStep::LookAtFixture {
                who: 1.0,
                fixture: 5.0,
            },
        ];
        assert_eq!(steps[0].hold().clone(), Hold::Time { seconds: 1.0, click_skippable: true });
        assert_eq!(steps[1].hold(), Hold::Click);
        assert_eq!(steps[2].hold(), Hold::None);
    }

    // ---- 推进 ----

    #[test]
    fn advance_fires_instant_steps_then_parks_on_wait() {
        // 手算锚：[text, voice, wait 1.5, look]
        // dt=0   -> 触发 {0,1}，挂起 1.5
        // dt=1.0 -> 未到点，空
        // dt=0.5 -> 放行，触发 {3}
        let steps = [
            talk_text(),
            TalkStep::Voice {
                channel: "talk".to_string(),
                cue: "cue".to_string(),
                who: "a".to_string(),
            },
            talk_wait(1.5, Some(false)),
            talk_look(),
        ];
        let mut state = StreamState::new();
        let fired = advance(&steps, &mut state, 0.0, false).unwrap();
        assert_eq!(fired, vec![0, 1]);
        assert!(state.is_holding());

        let fired = advance(&steps, &mut state, 1.0, false).unwrap();
        assert!(fired.is_empty(), "1.0 < 1.5 未到点");

        let fired = advance(&steps, &mut state, 0.5, false).unwrap();
        assert_eq!(fired, vec![3]);
        assert!(!state.is_holding());
        assert!(is_finished(steps.len(), &state));
    }

    #[test]
    fn look_at_body_fires_without_holding_the_stream() {
        // 手算锚：duration 0.5 不驻留——同一拍直接挂到后续 wait 上
        let steps = [talk_look(), talk_wait(0.5, None)];
        let mut state = StreamState::new();
        let fired = advance(&steps, &mut state, 0.0, false).unwrap();
        assert_eq!(fired, vec![0], "look 即发即过");
        assert!(state.is_holding(), "驻留来自 wait_time");
    }

    #[test]
    fn wait_click_parks_until_click_and_consumes_it_once() {
        // 手算锚：[wait_click, text]
        // dt(0,false) -> 空挂起；dt(0.016,true) -> 放行并触发 text
        let steps = [TalkStep::WaitClick, talk_text()];
        let mut state = StreamState::new();
        let fired = advance(&steps, &mut state, 0.0, false).unwrap();
        assert!(fired.is_empty());
        assert!(state.is_holding());

        let fired = advance(&steps, &mut state, 0.016, true).unwrap();
        assert_eq!(fired, vec![1]);
        assert!(!state.is_holding());
    }

    #[test]
    fn skippable_wait_releases_on_click_unskippable_does_not() {
        // 可点跳：None 缺参 -> 点跳真
        let steps = [talk_wait(5.0, None), talk_text()];
        let mut state = StreamState::new();
        advance(&steps, &mut state, 0.0, false).unwrap();
        let fired = advance(&steps, &mut state, 1.0, true).unwrap();
        assert_eq!(fired, vec![1], "点击提前放行");

        // 不可点跳：Some(false) -> 点击无效
        let steps = [talk_wait(5.0, Some(false)), talk_text()];
        let mut state = StreamState::new();
        advance(&steps, &mut state, 0.0, false).unwrap();
        let fired = advance(&steps, &mut state, 1.0, true).unwrap();
        assert!(fired.is_empty(), "点击不提前放行");
        assert!(state.is_holding());
        let fired = advance(&steps, &mut state, 4.0, false).unwrap();
        assert_eq!(fired, vec![1], "5 秒耗尽后放行");
    }

    #[test]
    fn one_click_releases_at_most_one_hold_per_call() {
        // 手算锚：[wait_click, wait 5 可点跳, text]
        // 第一拍带击：放行 wait_click；新挂起不吃同一击
        let steps = [TalkStep::WaitClick, talk_wait(5.0, None), talk_text()];
        let mut state = StreamState::new();
        advance(&steps, &mut state, 0.0, false).unwrap();
        let fired = advance(&steps, &mut state, 0.0, true).unwrap();
        assert!(fired.is_empty(), "击被 wait_click 消费，后续 wait 仍挂起");
        assert!(state.is_holding());
        // 第二拍再带击：放行可点跳 wait，text 触发
        let fired = advance(&steps, &mut state, 0.0, true).unwrap();
        assert_eq!(fired, vec![2]);
    }

    #[test]
    fn zero_length_wait_releases_on_next_beat() {
        let steps = [talk_wait(0.0, None), talk_text()];
        let mut state = StreamState::new();
        let fired = advance(&steps, &mut state, 0.0, false).unwrap();
        assert!(fired.is_empty(), "零时长挂起也在下一拍放行");
        let fired = advance(&steps, &mut state, 0.0, false).unwrap();
        assert_eq!(fired, vec![1]);
    }

    #[test]
    fn advance_rejects_negative_and_non_finite_dt_without_moving() {
        let steps = [talk_text()];
        let mut state = StreamState::new();
        state.cursor = 1;
        assert_eq!(
            advance(&steps, &mut state, -0.016, false),
            Err(Rejection::NegativeDt)
        );
        assert_eq!(
            advance(&steps, &mut state, f32::NAN, false),
            Err(Rejection::NonFiniteDt)
        );
        assert_eq!(
            advance(&steps, &mut state, f32::INFINITY, false),
            Err(Rejection::NonFiniteDt)
        );
        assert_eq!(state.cursor, 1, "拒绝不动游标");
        assert!(!state.is_holding());
    }

    #[test]
    fn finished_needs_all_steps_consumed_and_no_hold() {
        let steps = [talk_text(), talk_wait(1.0, None)];
        let mut state = StreamState::new();
        advance(&steps, &mut state, 0.0, false).unwrap();
        assert!(!is_finished(steps.len(), &state), "还挂着 wait");
        advance(&steps, &mut state, 1.0, false).unwrap();
        assert!(is_finished(steps.len(), &state));
        // 空流生而完成
        let empty: [TalkStep; 0] = [];
        let mut idle = StreamState::new();
        let fired = advance(&empty, &mut idle, 0.5, false).unwrap();
        assert!(fired.is_empty());
        assert!(is_finished(0, &idle));
    }

    // ---- 锚点 ----

    #[test]
    fn label_anchor_resolves_first_occurrence() {
        // 手算锚：锚在下标 1 与 3
        let steps = [
            talk_text(),
            TalkStep::Label {
                name: "anchor_a".to_string(),
            },
            talk_text(),
            TalkStep::Label {
                name: "anchor_b".to_string(),
            },
        ];
        assert_eq!(label_anchor(&steps, "anchor_a"), Some(1));
        assert_eq!(label_anchor(&steps, "anchor_b"), Some(3));
        assert_eq!(label_anchor(&steps, "missing"), None);
    }

    // ---- op 分发 ----

    #[test]
    fn talk_op_vocabulary_is_pinned() {
        // 词表钉死：十四 op 与提取直方图词表一一对应
        let steps = [
            TalkStep::LookAtBody { who: String::new(), target: String::new(), duration: 0.0 },
            talk_wait(0.0, None),
            TalkStep::Label { name: String::new() },
            TalkStep::Voice { channel: String::new(), cue: String::new(), who: String::new() },
            TalkStep::ChangeNpcEye { who: String::new(), pattern: String::new(), alias: String::new() },
            TalkStep::ChangeNpcMouth { who: String::new(), pattern: String::new(), alias: String::new() },
            TalkStep::ChangeAnimation {
                who: String::new(),
                motion: String::new(),
                alias: String::new(),
                speed: None,
                playback_speed: 1.0,
                play_end_motion: false,
            },
            talk_text(),
            TalkStep::WaitClick,
            TalkStep::Emoticon {
                who: String::new(),
                name: String::new(),
                alias: String::new(),
                show_seconds: 0.0,
            },
            TalkStep::HideEmoticon { who: String::new() },
            TalkStep::ShowTalkWindow,
            TalkStep::HideTalkWindow,
            TalkStep::WaitTimeOnAutoMode { seconds: 0.0 },
        ];
        let expected = [
            "look_at_body",
            "wait_time",
            "label",
            "voice",
            "change_npc_eye",
            "change_npc_mouth",
            "change_animation",
            "text",
            "wait_click",
            "emoticon",
            "hide_emoticon",
            "show_talk_window",
            "hide_talk_window",
            "wait_time_on_auto_mode",
        ];
        for (step, op) in steps.iter().zip(expected) {
            assert_eq!(step.op(), op);
        }
    }

    #[test]
    fn fixture_op_vocabulary_covers_the_fixture_family() {
        let steps = [
            FixtureStep::LookAtFixture { who: 0.0, fixture: 0.0 },
            FixtureStep::LookAtToNpc { who: 0.0, fixture: 0.0, duration: 0.0 },
            FixtureStep::FixtureVoice { cue: String::new(), fixture: 0.0 },
            FixtureStep::ChangeFixtureCharacterEye {
                fixture: 0.0,
                pattern: String::new(),
                alias: String::new(),
            },
            FixtureStep::ChangeFixtureCharacterMouth {
                fixture: 0.0,
                pattern: String::new(),
                alias: String::new(),
            },
            FixtureStep::ChangeFixtureTimeline {
                fixture: 0.0,
                name: String::new(),
                value: 0.0,
            },
            FixtureStep::ShowFixtureEmoticon {
                fixture: 0.0,
                name: String::new(),
                alias: String::new(),
                show_seconds: 0.0,
            },
            FixtureStep::PlayFixtureGimmick { fixture: String::new() },
            FixtureStep::StopFixtureGimmick { fixture: String::new(), name: 0.0 },
        ];
        let expected = [
            "look_at_fixture",
            "look_at_to_npc",
            "fixture_voice",
            "change_fixture_character_eye",
            "change_fixture_character_mouth",
            "change_fixture_timeline",
            "show_fixture_emoticon",
            "play_fixture_gimmick",
            "stop_fixture_gimmick",
        ];
        for (step, op) in steps.iter().zip(expected) {
            assert_eq!(step.op(), op);
        }
    }

    #[test]
    fn face_changes_align_with_facial_table_keys() {
        // 眼/口步载荷 = (槽位, 图样键, 源记号)；其余步无脸载荷
        let eye = TalkStep::ChangeNpcEye {
            who: "a".to_string(),
            pattern: "EyePresets.normal".to_string(),
            alias: "EyePresets.normal".to_string(),
        };
        assert_eq!(
            eye.face_change(),
            Some((FaceSlot::Eye, "EyePresets.normal", "EyePresets.normal"))
        );
        let mouth = TalkStep::ChangeNpcMouth {
            who: "a".to_string(),
            pattern: "MouthPresets.smile01".to_string(),
            alias: "MouthPresets.smile01".to_string(),
        };
        assert_eq!(
            mouth.face_change(),
            Some((FaceSlot::Mouth, "MouthPresets.smile01", "MouthPresets.smile01"))
        );
        assert_eq!(talk_text().face_change(), None);
        // 家具形的眼/口载荷同构
        let fx_eye = FixtureStep::ChangeNpcEye {
            who: 1.0,
            pattern: "p".to_string(),
            alias: "p".to_string(),
        };
        assert_eq!(fx_eye.face_change(), Some((FaceSlot::Eye, "p", "p")));
    }
}
