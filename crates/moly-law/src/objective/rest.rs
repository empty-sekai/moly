//! 步间停顿（源 `TryRest` / `WaitWhile` 一族）：主循环在每轮决策**之前**
//! 无条件插入停顿目标，停顿的时长、跳过与保持谓词在此落律。
//!
//! 源的形状：
//! - 停顿开关在类构造里写死为开（调试开关，产品形态恒开），不迁参数；
//! - `PauseTime` 无穷 ⇒ 0 毫秒（不停）；否则 `1000 × (int)pause_seconds`
//!   毫秒，**向零截断**；
//! - 延迟走完后保持条件是「当前状态是对话」（状态判别值 4），对话
//!   未结束就继续停；
//! - 「立即可执行下一目标」旗置位 ⇒ 本轮停顿整个跳过；旗在跳过与
//!   停完两条路径上都清除（一次性）。

/// 对话状态判别值（源状态机 `CurrentStateType == Talk`）：停顿延迟
/// 走完后仍处此状态则继续停。
pub const TALK_HOLD_STATE_TYPE: i32 = 4;

/// 停顿门结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestGateOutcome {
    /// 旗置位：本轮跳过停顿（旗清除，一次性）。
    Skipped,
    /// 旗未置：进入停顿（停完旗也清除）。
    Resting,
}

/// 停顿时长（毫秒）：`pause_seconds` 无穷 ⇒ 0；否则
/// `1000 × (int)pause_seconds`，向零截断。
///
/// 负值与 NaN 拒绝（数据列产不出该形状；产出即响）——源会把它们原样
/// 送进延迟调用，这里按数据错误处理返回 `None`。
pub fn rest_delay_milliseconds(pause_seconds: f32) -> Option<u32> {
    if pause_seconds.is_nan() || pause_seconds < 0.0 {
        return None;
    }
    if pause_seconds.is_infinite() {
        return Some(0);
    }
    // 源 (int) 截断：向零取整。
    Some((1000.0 * pause_seconds.trunc()) as u32)
}

/// 停顿门：立旗跳过、未立进入；两条路径都清旗。
pub fn try_rest_gate(immediately_execute_next: bool) -> RestGateOutcome {
    if immediately_execute_next {
        RestGateOutcome::Skipped
    } else {
        RestGateOutcome::Resting
    }
}

/// 停顿的保持谓词：延迟走完后当前状态仍是对话则继续停。
pub fn rest_holds(current_state_type: i32) -> bool {
    current_state_type == TALK_HOLD_STATE_TYPE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_pause_truncates_to_whole_seconds() {
        // 15.0 → 15000；15.9 截断到 15000（向零取整）
        assert_eq!(rest_delay_milliseconds(15.0), Some(15_000));
        assert_eq!(rest_delay_milliseconds(15.9), Some(15_000));
        assert_eq!(rest_delay_milliseconds(0.5), Some(0));
        assert_eq!(rest_delay_milliseconds(0.0), Some(0));
    }

    #[test]
    fn infinite_pause_is_no_rest() {
        // 无穷 ⇒ 0 毫秒：立进立出
        assert_eq!(rest_delay_milliseconds(f32::INFINITY), Some(0));
    }

    #[test]
    fn negative_and_nan_pause_is_rejected() {
        // 数据列产不出负值/NaN：拒绝而非静默
        assert_eq!(rest_delay_milliseconds(-1.0), None);
        assert_eq!(rest_delay_milliseconds(f32::NEG_INFINITY), None);
        assert_eq!(rest_delay_milliseconds(f32::NAN), None);
    }

    #[test]
    fn gate_skips_only_when_flag_set() {
        assert_eq!(try_rest_gate(true), RestGateOutcome::Skipped);
        assert_eq!(try_rest_gate(false), RestGateOutcome::Resting);
    }

    #[test]
    fn rest_holds_exactly_on_talk_state() {
        // 保持谓词恰在对话态（判别值 4）为真
        assert!(rest_holds(TALK_HOLD_STATE_TYPE));
        assert_eq!(TALK_HOLD_STATE_TYPE, 4);
        assert!(!rest_holds(0));
        assert!(!rest_holds(3));
        assert!(!rest_holds(5));
        assert!(!rest_holds(-1));
    }
}
