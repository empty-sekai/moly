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

/// 停顿目标的毫秒值（源停顿目标体内的换算，逐条机器指令）：`PauseTime`
/// 正无穷 ⇒ 0；否则 `(int)PauseTime` 按 32 位向零截断（NaN 得 0、越界
/// 饱和），再乘 1000 按 32 位回绕。负值原样返回——延迟入口对负时长抛
/// 越界异常，调用方把它当具名失败，不钳成 0。
pub fn rest_objective_milliseconds(pause_time: f32) -> i32 {
    if pause_time == f32::INFINITY {
        return 0;
    }
    // Rust 的 `as i32` 与 32 位 fcvtzs 同形：NaN → 0，越界饱和。
    (pause_time as i32).wrapping_mul(1000)
}

/// 帧计时的延迟（源协程库的缩放时间延迟对象）。
///
/// 构造：毫秒数先成 TimeSpan（毫秒 × 10000 个 tick），延迟 = `(float)`
/// 其总秒数（tick × 1e-7，双精度）。负毫秒在构造处抛越界异常。推进：
/// 创建帧不累加（同帧跳过）；此后每帧一次 `elapsed += deltaTime`（单精度
/// 累加并存回），`delay <= elapsed` 的那一帧完成。0 毫秒因此在创建后的
/// 下一帧完成，不在当帧。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayPromise {
    delay: f32,
    elapsed: f32,
}

impl DelayPromise {
    /// 按毫秒建延迟；负值是源的越界异常（`Err` 带原值）。
    pub fn from_milliseconds(milliseconds: i32) -> Result<Self, i32> {
        if milliseconds < 0 {
            return Err(milliseconds);
        }
        let ticks = i64::from(milliseconds) * 10_000;
        let total_seconds = ticks as f64 * (1.0 / 10_000_000.0);
        Ok(Self {
            delay: total_seconds as f32,
            elapsed: 0.0,
        })
    }

    /// 创建帧之后的一帧：累加这一帧的 deltaTime，返回是否完成。
    pub fn advance(&mut self, delta_time: f32) -> bool {
        self.elapsed += delta_time;
        self.delay <= self.elapsed
    }

    /// 延迟的秒数（单精度，构造时定）。
    pub fn delay(&self) -> f32 {
        self.delay
    }

    /// 已累加的秒数。
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }
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

    /// 源值仪器：停顿毫秒换算的游戏机器码执行结果（逐行：PauseTime 位型 →
    /// 毫秒）。
    #[test]
    fn rest_objective_milliseconds_equal_the_executed_source() {
        let rows: [(u32, i32); 14] = [
            (0x41700000, 15000),
            (0x41780000, 15000),
            (0x416fff97, 14000),
            (0x3f7fbe77, 0),
            (0x00000000, 0),
            (0x80000000, 0),
            (0x3f800000, 1000),
            (0x7f800000, 0),
            (0xff800000, 0),
            (0x7fc00000, 0),
            (0xbfc00000, -1000),
            (0x4a03126c, 2147483000),
            (0x4a031270, -2147483296),
            (0x4f32d05e, -1000),
        ];
        let mut distinct = std::collections::HashSet::new();
        for (bits, expected) in rows {
            let got = rest_objective_milliseconds(f32::from_bits(bits));
            distinct.insert(got);
            assert_eq!(got, expected, "PauseTime bits {bits:#x}");
        }
        // 正向臂：比较的这一维有信号（不是一列同值）。
        assert!(distinct.len() >= 5);
    }

    /// 源值仪器：延迟对象在游戏代码里执行得到的完成帧数（创建帧之后到完成
    /// 那次推进的帧数），三种稳定帧长与一帧长帧（被引擎钳到最大帧长）。
    #[test]
    fn delay_promise_frames_equal_the_executed_source() {
        let steady = |bits: u32| move |_frame: usize| f32::from_bits(bits);
        let frames = |ms: i32, dt: &dyn Fn(usize) -> f32| {
            let mut promise = DelayPromise::from_milliseconds(ms).unwrap();
            let mut n = 0;
            loop {
                n += 1;
                if promise.advance(dt(n)) {
                    return n;
                }
            }
        };
        let cases: [(u32, [usize; 5]); 3] = [
            (0x3C888889, [901, 931, 61, 60, 1]),
            (0x3D088889, [450, 465, 30, 30, 1]),
            (0x3E800000, [60, 62, 4, 4, 1]),
        ];
        for (bits, expected) in cases {
            let dt = steady(bits);
            let got: Vec<usize> = [15000, 15500, 1000, 999, 0]
                .iter()
                .map(|ms| frames(*ms, &dt))
                .collect();
            assert_eq!(got, expected, "dt bits {bits:#x}");
        }
        // 60 Hz 里第 20 次推进是一帧被钳到最大帧长的长帧。
        let long = |n: usize| {
            if n == 20 {
                f32::from_bits(0x3EAAAAAB)
            } else {
                f32::from_bits(0x3C888889)
            }
        };
        let got: Vec<usize> = [15000, 15500, 1000, 999, 0]
            .iter()
            .map(|ms| frames(*ms, &long))
            .collect();
        assert_eq!(got, vec![882, 912, 42, 41, 1]);
        assert_eq!(
            DelayPromise::from_milliseconds(999).unwrap().delay().to_bits(),
            0x3F7FBE77
        );
        assert_eq!(DelayPromise::from_milliseconds(-1000), Err(-1000));
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
