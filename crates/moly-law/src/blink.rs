//! The NPC view's automatic blink: `UpdateBlink` starts one `Blink` cycle
//! whenever none is playing; the cycle is an awaited sequence of eye writes
//! and scaled-time delays.
//!
//! Source shape (the cycle, in order):
//! 1. The view's destroy token is cancelled: nothing (no flag, no write).
//! 2. Blinking disabled on the view: write the pattern's open cell and await
//!    one Yield; the playing flag is never set, so the next frame's update
//!    starts again.
//! 3. Otherwise the playing flag is set, and inside a try whose finally
//!    clears it:
//!    - a pattern that does not blink: write open, then one delay of
//!      `Range(3000, 5000)` ms;
//!    - a blinking pattern: close, `Range(100, 150)` ms; open,
//!      `Range(100, 150)` ms; one `Range(0, 2)` draw, and on 1 a second
//!      close, `Range(100, 150)` ms, open, `Range(100, 150)` ms; then
//!      `Range(3000, 5000)` ms.
//! The draws are integer `Random.Range(min, max)` (max exclusive), one per
//! delay plus the one double draw. Every delay is a scaled-time,
//! Update-timed delay ([`DelayPromise`]). The open and close cells are read
//! from the view's current pattern at each write (a pattern change during a
//! cycle is seen by the next write); the two enable flags are read only when
//! the cycle starts.
//!
//! Frame order: a delay's continuation runs in the Update-timed task runner,
//! ahead of the presenter's per-frame update, so the write after a delay and
//! the clear of the playing flag happen at the start of the frame the delay
//! completes on; the presenter's update of that same frame then starts the
//! next cycle. A delay created in a frame is first advanced on the next.

use crate::objective::DelayPromise;

/// The view's eye pattern row (`NPCAvatarEyeData`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EyePattern {
    pub open: i32,
    pub close: i32,
    pub blink_enabled: bool,
}

/// The blink's integer draws (`UnityEngine.Random.Range(int, int)`).
pub trait RangeDraw {
    /// A value in `min..max`.
    fn range(&mut self, min: i32, max: i32) -> i32;
}

/// `Range(100, 150)`: each close and open hold, in milliseconds.
pub const BLINK_HOLD_MS: (i32, i32) = (100, 150);
/// `Range(3000, 5000)`: the interval after a cycle, in milliseconds.
pub const BLINK_INTERVAL_MS: (i32, i32) = (3000, 5000);
/// `Range(0, 2)`: a second blink when the draw is 1.
pub const BLINK_DOUBLE_RANGE: (i32, i32) = (0, 2);
/// The draw value that adds the second blink.
pub const BLINK_DOUBLE_VALUE: i32 = 1;

/// The awaited step a cycle is on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// After the first close.
    Close1,
    /// After the first open.
    Open1,
    /// After the second close.
    Close2,
    /// After the second open.
    Open2,
    /// The interval; its end clears the flag.
    Interval,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    step: Step,
    delay: DelayPromise,
}

/// One frame's writes of the blink.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlinkFrame {
    /// The eye cell written by a continuation (at the frame start).
    pub resumed: Option<i32>,
    /// The eye cell written by the update's new cycle.
    pub started: Option<i32>,
    /// A cycle ended this frame (the finally cleared the flag).
    pub ended: bool,
}

/// The view's blink: the playing flag and the awaited step.
#[derive(Clone, Debug, Default)]
pub struct Blink {
    pending: Option<Pending>,
}

impl Blink {
    /// `_isPlayingBlink`.
    pub fn is_playing(&self) -> bool {
        self.pending.is_some()
    }

    /// The view is destroyed (token cancelled): the awaited delay throws and
    /// the finally clears the flag.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    fn wait(&mut self, step: Step, (min, max): (i32, i32), draw: &mut impl RangeDraw) {
        let ms = draw.range(min, max);
        assert!(
            (min..max).contains(&ms),
            "Random.Range({min}, {max}) gave {ms}"
        );
        let delay = DelayPromise::from_milliseconds(ms).expect("blink delays are positive");
        self.pending = Some(Pending { step, delay });
    }

    /// The Update-timed runner's pass: the awaited delay advances by this
    /// frame's `delta_time`; on completion the cycle's next step runs (its
    /// eye write, its draws, its next delay). Returns the written cell and
    /// whether the cycle ended (the finally cleared the flag).
    pub fn resume(
        &mut self,
        delta_time: f32,
        pattern: EyePattern,
        draw: &mut impl RangeDraw,
    ) -> (Option<i32>, bool) {
        let Some(mut pending) = self.pending else {
            return (None, false);
        };
        if !pending.delay.advance(delta_time) {
            self.pending = Some(pending);
            return (None, false);
        }
        self.pending = None;
        match pending.step {
            Step::Close1 => {
                self.wait(Step::Open1, BLINK_HOLD_MS, draw);
                (Some(pattern.open), false)
            }
            Step::Open1 => {
                let (min, max) = BLINK_DOUBLE_RANGE;
                let double = draw.range(min, max);
                assert!((min..max).contains(&double));
                if double == BLINK_DOUBLE_VALUE {
                    self.wait(Step::Close2, BLINK_HOLD_MS, draw);
                    (Some(pattern.close), false)
                } else {
                    self.wait(Step::Interval, BLINK_INTERVAL_MS, draw);
                    (None, false)
                }
            }
            Step::Close2 => {
                self.wait(Step::Open2, BLINK_HOLD_MS, draw);
                (Some(pattern.open), false)
            }
            Step::Open2 => {
                self.wait(Step::Interval, BLINK_INTERVAL_MS, draw);
                (None, false)
            }
            Step::Interval => (None, true),
        }
    }

    /// `UpdateBlink`: a new cycle when none is playing. `enabled` is the
    /// view's blink flag and `cancelled` its destroy token, both read at the
    /// cycle's start. Returns the written cell.
    pub fn update(
        &mut self,
        enabled: bool,
        cancelled: bool,
        pattern: EyePattern,
        draw: &mut impl RangeDraw,
    ) -> Option<i32> {
        if self.is_playing() || cancelled {
            return None;
        }
        if !enabled {
            // Open, then one Yield; the flag stays clear.
            Some(pattern.open)
        } else if !pattern.blink_enabled {
            self.wait(Step::Interval, BLINK_INTERVAL_MS, draw);
            Some(pattern.open)
        } else {
            self.wait(Step::Close1, BLINK_HOLD_MS, draw);
            Some(pattern.close)
        }
    }

    /// One frame: the runner's pass ([`Self::resume`]), then the presenter's
    /// update ([`Self::update`]); `pattern` is the view's current pattern.
    pub fn frame(
        &mut self,
        delta_time: f32,
        enabled: bool,
        cancelled: bool,
        pattern: EyePattern,
        draw: &mut impl RangeDraw,
    ) -> BlinkFrame {
        let (resumed, ended) = self.resume(delta_time, pattern, draw);
        let started = self.update(enabled, cancelled, pattern, draw);
        BlinkFrame { resumed, started, ended }
    }
}

#[cfg(test)]
#[path = "blink_source_cases.rs"]
mod source_cases;
