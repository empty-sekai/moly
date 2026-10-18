//! The timing helpers every layer's fades and delays use: the colour fader of
//! a cover or a screen (a coroutine, not a DOTween call) and the UniTask
//! delay. Both reproduce the source's frame semantics: the fader counts the
//! frame that started it, a UniTask delay does not.

use bevy::prelude::*;

/// `Mathf.Approximately`.
fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (0.000_001 * a.abs().max(b.abs())).max(f32::from_bits(1) * 8.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FadeStage {
    /// Inside the nested `CP_SystemUtility.WaitForSeconds(delay)`.
    Delay,
    /// The fade loop of `ColorFader.PlaySimpleFade`.
    Fade,
    /// The target colour is written; the coroutine has ended.
    Done,
}

/// `ColorFader.Play(target, delay, duration)`: not a DOTween call. It starts
/// the coroutine `PlaySimpleFade`, which keeps the current colour as base,
/// yields `CP_SystemUtility.WaitForSeconds(delay)` (a loop that adds
/// `Time.deltaTime` and yields null while `time < duration`), then on every
/// frame either writes the target (once `time >= duration`) or writes
/// `base + (target - base) * clamp(time / duration, 0, 1)` and only then adds
/// this frame's delta. The first fade frame therefore writes the base colour.
#[derive(Clone, Debug)]
pub(crate) struct ColorFade {
    from: [f32; 4],
    to: [f32; 4],
    delay: f32,
    duration: f32,
    waited: f32,
    time: f32,
    stage: FadeStage,
}

impl ColorFade {
    /// The frame `Play` is called: `StartCoroutine` runs the body up to its
    /// first yield, and the nested wait already adds this frame's delta.
    pub(crate) fn start(
        from: [f32; 4],
        to: [f32; 4],
        delay: f32,
        duration: f32,
        dt: f32,
    ) -> (Self, Option<[f32; 4]>) {
        let mut fade = Self {
            from,
            to,
            delay,
            duration,
            waited: 0.0,
            time: 0.0,
            stage: FadeStage::Delay,
        };
        if approximately(delay, 0.0) && approximately(duration, 0.0) {
            fade.stage = FadeStage::Done;
            return (fade, Some(to));
        }
        let written = fade.run(dt);
        (fade, written)
    }

    /// One later frame's resumption. Returns the colour written this frame.
    pub(crate) fn resume(&mut self, dt: f32) -> Option<[f32; 4]> {
        self.run(dt)
    }

    pub(crate) fn stage(&self) -> FadeStage {
        self.stage
    }

    fn run(&mut self, dt: f32) -> Option<[f32; 4]> {
        if self.stage == FadeStage::Delay {
            if self.waited < self.delay {
                self.waited += dt;
                return None;
            }
            // The nested routine ends; the outer coroutine resumes this frame.
            self.stage = FadeStage::Fade;
        }
        if self.stage != FadeStage::Fade {
            return None;
        }
        if self.time >= self.duration {
            self.stage = FadeStage::Done;
            return Some(self.to);
        }
        let mut u = (self.time / self.duration).min(1.0);
        if u < 0.0 {
            u = 0.0;
        }
        let mut colour = [0.0; 4];
        for (channel, value) in colour.iter_mut().enumerate() {
            *value = self.from[channel] + (self.to[channel] - self.from[channel]) * u;
        }
        self.time += dt;
        Some(colour)
    }
}

/// `UniTask.Delay(seconds)` in the default delta-time mode: the delay promise
/// does nothing on the frame it was created, then adds `Time.deltaTime` each
/// frame and completes on the frame where `elapsed >= delay`; the awaiting
/// continuation runs in that same frame. [`Self::advance`] is called from the
/// frame after creation on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UniTaskDelay {
    delay: f32,
    elapsed: f32,
}

impl UniTaskDelay {
    pub(crate) fn new(delay: f32) -> Self {
        Self {
            delay,
            elapsed: 0.0,
        }
    }

    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed >= self.delay
    }
}
