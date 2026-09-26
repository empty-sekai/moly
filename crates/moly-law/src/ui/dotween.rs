//! DOTween's single-value tweener as the game's DOTween runs one: a
//! non-looping tween, killed on completion, updated on scaled frame time with
//! time scale 1, with an optional `SetDelay`.
//!
//! One `TweenManager.Update` step of such a tween:
//! - while the delay is not complete, `UpdateDelay(delta + elapsedDelay)`
//!   (`Tweener.DoUpdateDelay`): when the delay is below the elapsed time the
//!   delay completes, the elapsed delay becomes the delay and the excess is
//!   this step's delta; otherwise the elapsed delay becomes the elapsed time
//!   and the step applies nothing;
//! - the first applied step runs the startup, which reads the target's
//!   current value through the getter as the start and keeps
//!   `change = end - start`;
//! - the position is the running sum of the applied deltas; the step whose
//!   sum reaches the duration (`duration <= position`) completes the tween at
//!   the duration, and the tween is killed after it;
//! - the setter writes `start + change * Evaluate(ease, position, duration)`.
//!   `DOFade` on a CanvasGroup or a Graphic writes the alpha this way and
//!   `DOAnchorPosY` the y of the anchored position (the other channels are the
//!   getter's).
//!
//! `EaseManager.Evaluate` for the eases here, in the compiled operation order:
//! Linear `time / duration`; OutQuad `(t + -2) * -t` with `t = time /
//! duration`; OutQuart `-((t * (t * (t * t))) + -1)` with `t = time /
//! duration + -1`. An ease value outside the evaluated table (0, Unset) takes
//! the OutQuad arm.

/// The `DG.Tweening.Ease` values evaluated here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    /// `Ease.Linear` (1).
    Linear,
    /// `Ease.OutQuad` (6).
    OutQuad,
    /// `Ease.OutQuart` (12).
    OutQuart,
}

impl Ease {
    /// The serialized enum value.
    pub fn value(self) -> i32 {
        match self {
            Self::Linear => 1,
            Self::OutQuad => 6,
            Self::OutQuart => 12,
        }
    }

    /// `EaseManager.Evaluate(ease, time, duration, overshoot, period)` for
    /// this ease (the overshoot and period are not read by these arms).
    pub fn evaluate(self, time: f32, duration: f32) -> f32 {
        match self {
            Self::Linear => time / duration,
            Self::OutQuad => {
                let t = time / duration;
                (t + -2.0) * -t
            }
            Self::OutQuart => {
                let t = time / duration + -1.0;
                -((t * (t * (t * t))) + -1.0)
            }
        }
    }
}

/// One tween of a float channel (an alpha or one axis of a position).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatTween {
    end: f32,
    duration: f32,
    ease: Ease,
    delay: f32,
    elapsed_delay: f32,
    delay_complete: bool,
    /// Start and change, set by the startup.
    started: Option<(f32, f32)>,
    /// Running sum of the applied deltas.
    position: f32,
    complete: bool,
}

impl FloatTween {
    /// A tween to `end` over `duration` with `ease` and no delay.
    pub fn new(end: f32, duration: f32, ease: Ease) -> Self {
        Self {
            end,
            duration,
            ease,
            delay: 0.0,
            elapsed_delay: 0.0,
            delay_complete: true,
            started: None,
            position: 0.0,
            complete: false,
        }
    }

    /// `SetDelay(delay)`: the delay is complete from the start when it is
    /// not above 0.
    pub fn with_delay(mut self, delay: f32) -> Self {
        self.delay = delay;
        self.delay_complete = delay <= 0.0;
        self
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn delay(&self) -> f32 {
        self.delay
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// The applied position (the running sum, at most the duration once
    /// complete).
    pub fn position(&self) -> f32 {
        if self.complete {
            self.duration
        } else {
            self.position
        }
    }

    /// One update with `delta`. `current` is the getter's value, read only
    /// by the startup. Returns the value the setter writes, or None when the
    /// delay swallowed the step (nothing is written). A completed tween is
    /// killed: further calls write nothing.
    pub fn update(&mut self, current: f32, delta: f32) -> Option<f32> {
        if self.complete {
            return None;
        }
        let mut delta = delta;
        if !self.delay_complete {
            let elapsed = delta + self.elapsed_delay;
            if self.delay < elapsed {
                self.elapsed_delay = self.delay;
                self.delay_complete = true;
                delta = elapsed - self.delay;
            } else {
                self.elapsed_delay = elapsed;
                delta = 0.0;
            }
            if delta <= 0.0 {
                return None;
            }
        }
        let end = self.end;
        let (start, change) = *self.started.get_or_insert_with(|| (current, end - current));
        self.position += delta;
        self.complete = self.duration <= self.position;
        let position = if self.complete {
            self.duration
        } else {
            self.position
        };
        Some(start + change * self.ease.evaluate(position, self.duration))
    }
}

#[cfg(test)]
mod value_checks {
    //! Research instrument: expected values computed by hand from the
    //! compiled arms named in the module documentation.
    use super::*;

    /// Evaluate at the half position: Linear 0.5, OutQuad (0.5 - 2) * -0.5 =
    /// 0.75, OutQuart -((-0.5)^4 - 1) = 0.9375; each is 1 at the end.
    #[test]
    fn eases_at_half_and_end() {
        assert_eq!(Ease::Linear.evaluate(0.5, 1.0), 0.5);
        assert_eq!(Ease::OutQuad.evaluate(0.5, 1.0), 0.75);
        assert_eq!(Ease::OutQuart.evaluate(0.5, 1.0), 0.9375);
        for ease in [Ease::Linear, Ease::OutQuad, Ease::OutQuart] {
            assert_eq!(ease.evaluate(0.2, 0.2), 1.0);
        }
    }

    /// A delay of 0.5 with 0.25 s steps: the first two steps are swallowed
    /// (0.25 and 0.5 are not above the delay), the third carries 0.25 into
    /// the tween; a 1.0 s linear tween from 2 to 0 reads its start there.
    #[test]
    fn delay_is_strict_and_carries_the_excess() {
        let mut tween = FloatTween::new(0.0, 1.0, Ease::Linear).with_delay(0.5);
        assert_eq!(tween.update(9.0, 0.25), None);
        assert_eq!(tween.update(9.0, 0.25), None);
        assert_eq!(tween.update(2.0, 0.25), Some(1.5));
        assert_eq!(tween.update(7.0, 0.25), Some(1.0));
        assert_eq!(tween.update(7.0, 0.5), Some(0.0));
        assert!(tween.is_complete());
        assert_eq!(tween.update(7.0, 0.5), None);
    }
}
