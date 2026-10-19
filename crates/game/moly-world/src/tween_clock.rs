//! The frame clocks of the source's waits and tweens, shared by the world and
//! the features: a `UniTask.Delay` (which never advances on the frame that
//! creates it) and a DOTween clock, plus the millisecond rounding
//! `TimeSpan.FromSeconds` applies to a delay's seconds.

/// Frames a `UniTask.Delay` runs on. It never advances on the frame that
/// creates it; each later frame adds that frame's `Time.deltaTime` as a float
/// and completes on the first frame whose sum reaches the target
/// (`elapsed >= delay`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delay {
    pub target: f32,
    pub elapsed: f32,
    pub created_frame: u64,
}

impl Delay {
    pub fn new(target: f32, frame: u64) -> Self {
        Self {
            target,
            elapsed: 0.0,
            created_frame: frame,
        }
    }

    /// Advance on one frame; true once the delay is due. The creation frame
    /// is skipped, as `DelayPromise.MoveNext` skips it while nothing has
    /// elapsed yet.
    pub fn tick(&mut self, frame: u64, dt: f32) -> bool {
        if frame == self.created_frame {
            return false;
        }
        self.elapsed += dt;
        self.elapsed >= self.target
    }
}

/// A DOTween clock (player `DOMove`, the environment follow, the cannon's
/// `DOScale`). The product convention for tweens (see `camera::CameraTween`,
/// advanced in the frame it is inserted) is kept: the creating frame's delta
/// counts. DOTween's own first-update timing was not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TweenClock {
    pub duration: f32,
    pub elapsed: f32,
}

impl TweenClock {
    pub fn new(duration: f32) -> Self {
        Self {
            duration,
            elapsed: 0.0,
        }
    }

    /// Advance and return the normalised time in [0, 1].
    pub fn advance(&mut self, dt: f32) -> f32 {
        self.elapsed += dt;
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    pub fn done(&self) -> bool {
        self.elapsed >= self.duration
    }
}

/// `TimeSpan.FromSeconds(x)` as a delay sees it: `Interval(x, 1000)` turns
/// `x * 1000` into whole milliseconds, rounding half away from zero, scales
/// them to 100 ns ticks, and `TotalSeconds` multiplies the ticks by 1e-7 in
/// double; `DelayPromise` stores that as a float.
pub fn delay_seconds(x: f64) -> f32 {
    let scaled = x * 1000.0;
    let millis = (scaled + if scaled >= 0.0 { 0.5 } else { -0.5 }).trunc() as i64;
    let ticks = millis * 10_000;
    (ticks as f64 * 1e-7) as f32
}
