//! `ScreenLayer`'s start and exit animations: what `PlayStartAnimation` and
//! `PlayExitAnimation` build for a screen's animation type, and the tween each
//! one runs.
//!
//! `PlayStartAnimation` sets the state Starting, destroys the previous
//! animation and switches on `inAnimType` (0 to 8):
//! - 0 and 8: no animation; the start finishes at once (`OnFinishStartAnim`);
//! - 1 to 4: a slide of the screen root from `ScreenManager`'s width or height
//!   times -1.2 / 1.2 to the zero vector;
//! - 5: `CreateAlphaAnimation(0, 1, IncludeChildCanvasForAlphaTransiton)`;
//! - 6: `CreateScaleAnimation(Vector3.zero, Vector3.one)`;
//! - 7: a mask animation with the data's mask textures.
//!
//! `PlayExitAnimation` switches on `outAnimType`: 5 builds
//! `CreateAlphaAnimation(1, 0, ...)`, 6 `CreateScaleAnimation(localScale,
//! Vector3.zero)` from the root's current scale, 8 sets `isManualExit` with no
//! animation, 0 has none; 1 to 4 and 7 are the slides and the mask. With no
//! animation the exit finishes at once (`OnFinishExitAnim`).
//!
//! The animation is played with a callback that ends the start
//! (`StartAnimationDone` = `OnFinishStartAnim`: state Playing, the animation
//! destroyed, `OnFinishStartAnimation`, `EnableTapScreen` unless the data
//! enables tap animation) or the exit (`ExitAnimationDone` =
//! `OnFinishExitAnim`: `EnableTapScreen` unless tap animation, the animation
//! destroyed, `isExitDone` unless `isManualExit`).
//!
//! `ReverseStartAnimationType` / `ReverseExitAnimationType` map the types 1
//! to 4 through the table (2, 1, 4, 3) and leave the others; they set the
//! reverse flag the mask reads.
//!
//! The two animations the MySekai screen data use:
//! - `ScreenAlphaInOut` on the screen root: `Setup` takes the root's
//!   `CanvasGroup` (added when missing); `IncludeChild` collects the root's
//!   `CanvasGroup`s in children (inactive ones included); `SetValue(start,
//!   target)` writes `alpha = start` at once and keeps the target; `Play` runs
//!   `DOTween.To(alpha getter, setter, target, 0.2)` with `SetEase(Linear)`.
//!   The setter writes every collected group when `IncludeChild`, else the
//!   root's group.
//! - `ScreenScaleInOut` on the screen root: `Setup` destroys the root's
//!   `CanvasGroup` when it has one, then kills its previous tween;
//!   `SetVector(start, target)` writes `localScale = start` at once; `Play`
//!   runs `transform.DOScale(target, 0.2)` with `SetEase(InOutQuad)`.
//!
//! The tweens follow the game's DOTween as `moly_law::ui::dotween` describes
//! it: the first applied step reads the current value as the start, the
//! position is the running sum of the frame deltas, and the step whose sum
//! reaches the duration completes the tween at the duration.
//!
//! Open, one frame: the product gives a new animation its first update on
//! the frame after it is built (the manager steps the animations before its
//! transitions). DOTween updates a tween in the frame it is created when the
//! creating code runs before `DOTweenComponent.Update` (a tap in the event
//! system, or a UniTask Update continuation) and in the next frame when it
//! runs after (a coroutine segment resumed after a yield). Which of these
//! reaches `PlayStartAnimation` / `PlayExitAnimation` depends on the screen
//! manager's coroutine bodies up to those calls, which this port has not
//! read.

use bevy::prelude::*;
use moly_law::ui::dotween::{Ease, FloatTween};

/// The duration both `ScreenAlphaInOut.Play` and `ScreenScaleInOut.Play`
/// pass to DOTween.
pub(crate) const SCREEN_ANIMATION_SECONDS: f32 = 0.2;

/// `ReverseStartAnimationType` / `ReverseExitAnimationType`.
pub(crate) fn reverse(animation: i64) -> i64 {
    match animation {
        1 => 2,
        2 => 1,
        3 => 4,
        4 => 3,
        other => other,
    }
}

/// `EaseManager.Evaluate` for `Ease.InOutQuad` (7), in the compiled
/// operation order: `t = time / (duration * 0.5)`; below 1, `t * (t * 0.5)`;
/// else with `u = t + -1`, `((u * (u + -2)) + -1) * -0.5`.
fn in_out_quad(time: f32, duration: f32) -> f32 {
    let t = time / (duration * 0.5);
    if t < 1.0 {
        t * (t * 0.5)
    } else {
        let u = t + -1.0;
        ((u * (u + -2.0)) + -1.0) * -0.5
    }
}

/// `DOScale`: a Vector3 tween, each axis `start + change * ease`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScaleTween {
    end: Vec3,
    started: Option<(Vec3, Vec3)>,
    position: f32,
    complete: bool,
}

impl ScaleTween {
    fn update(&mut self, current: Vec3, delta: f32) -> Option<Vec3> {
        if self.complete {
            return None;
        }
        let end = self.end;
        let (start, change) = *self.started.get_or_insert_with(|| (current, end - current));
        self.position += delta;
        self.complete = SCREEN_ANIMATION_SECONDS <= self.position;
        let position = if self.complete {
            SCREEN_ANIMATION_SECONDS
        } else {
            self.position
        };
        Some(start + change * in_out_quad(position, SCREEN_ANIMATION_SECONDS))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tween {
    Alpha(FloatTween),
    Scale(ScaleTween),
}

/// The start or the exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnimPhase {
    Start,
    Exit,
}

/// A screen root's values the animations write: the `CanvasGroup` alpha
/// (None while no animation wrote it, the prefab's own value) and the local
/// scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreenVisual {
    pub(crate) alpha: Option<f32>,
    pub(crate) scale: Vec3,
}

impl Default for ScreenVisual {
    fn default() -> Self {
        Self {
            alpha: None,
            scale: Vec3::ONE,
        }
    }
}

/// One running screen animation (`screenAnim`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreenAnim {
    pub(crate) phase: AnimPhase,
    pub(crate) animation: i64,
    pub(crate) include_child: bool,
    tween: Tween,
    /// Seconds applied so far (the tween's position).
    pub(crate) elapsed: f32,
}

/// What `PlayStartAnimation` / `PlayExitAnimation` do for a type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Built {
    /// No animation: the start or exit finishes at once (0, 8).
    None,
    /// `isManualExit`: the exit's animation step is none and the exit is not
    /// marked done by it (8).
    ManualExit,
    Anim(ScreenAnim),
    /// A slide (1 to 4) or the mask (7): not in this port.
    NotPorted,
}

impl ScreenAnim {
    /// `PlayStartAnimation` for `animation`; the setup writes its start value
    /// into `visual` at once.
    pub(crate) fn start(animation: i64, include_child: bool, visual: &mut ScreenVisual) -> Built {
        match animation {
            0 | 8 => Built::None,
            5 => Built::Anim(Self::alpha(
                AnimPhase::Start,
                animation,
                0.0,
                1.0,
                include_child,
                visual,
            )),
            6 => Built::Anim(Self::scale(
                AnimPhase::Start,
                animation,
                Vec3::ZERO,
                Vec3::ONE,
                visual,
            )),
            _ => Built::NotPorted,
        }
    }

    /// `PlayExitAnimation` for `animation`.
    pub(crate) fn exit(animation: i64, include_child: bool, visual: &mut ScreenVisual) -> Built {
        match animation {
            0 => Built::None,
            8 => Built::ManualExit,
            5 => Built::Anim(Self::alpha(
                AnimPhase::Exit,
                animation,
                1.0,
                0.0,
                include_child,
                visual,
            )),
            6 => {
                let from = visual.scale;
                Built::Anim(Self::scale(
                    AnimPhase::Exit,
                    animation,
                    from,
                    Vec3::ZERO,
                    visual,
                ))
            }
            _ => Built::NotPorted,
        }
    }

    fn alpha(
        phase: AnimPhase,
        animation: i64,
        start: f32,
        target: f32,
        include_child: bool,
        visual: &mut ScreenVisual,
    ) -> Self {
        // Setup (the root's group, added when missing), IncludeChild, then
        // SetValue writes the start.
        visual.alpha = Some(start);
        Self {
            phase,
            animation,
            include_child,
            tween: Tween::Alpha(FloatTween::new(
                target,
                SCREEN_ANIMATION_SECONDS,
                Ease::Linear,
            )),
            elapsed: 0.0,
        }
    }

    fn scale(
        phase: AnimPhase,
        animation: i64,
        start: Vec3,
        target: Vec3,
        visual: &mut ScreenVisual,
    ) -> Self {
        // Setup destroys the root's CanvasGroup: no group alpha is left.
        visual.alpha = Some(1.0);
        visual.scale = start;
        Self {
            phase,
            animation,
            include_child: false,
            tween: Tween::Scale(ScaleTween {
                end: target,
                started: None,
                position: 0.0,
                complete: false,
            }),
            elapsed: 0.0,
        }
    }

    /// One DOTween update with the frame's delta; true when the tween
    /// completed in this step (its OnComplete runs).
    pub(crate) fn step(&mut self, delta: f32, visual: &mut ScreenVisual) -> bool {
        match &mut self.tween {
            Tween::Alpha(tween) => {
                let current = visual.alpha.unwrap_or(1.0);
                if let Some(value) = tween.update(current, delta) {
                    visual.alpha = Some(value);
                }
                self.elapsed = tween.position();
                tween.is_complete()
            }
            Tween::Scale(tween) => {
                if let Some(value) = tween.update(visual.scale, delta) {
                    visual.scale = value;
                }
                self.elapsed = if tween.complete {
                    SCREEN_ANIMATION_SECONDS
                } else {
                    tween.position
                };
                tween.complete
            }
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match (self.tween, self.include_child) {
            (Tween::Alpha(_), false) => "ScreenAlphaInOut (DOTween.To alpha, 0.2 s, Linear)",
            (Tween::Alpha(_), true) => {
                "ScreenAlphaInOut (DOTween.To alpha, 0.2 s, Linear, IncludeChild: every CanvasGroup of the root)"
            }
            (Tween::Scale(_), _) => "ScreenScaleInOut (DOScale, 0.2 s, InOutQuad)",
        }
    }
}

#[cfg(test)]
mod value_checks {
    //! Research instrument: values computed by hand from the compiled arms
    //! named above.
    use super::*;

    /// InOutQuad at a quarter, the half and three quarters of 0.2 s: t = 0.5
    /// gives 0.5 * 0.5 * 0.5 = 0.125; t = 1 takes the second arm, u = 0,
    /// (0 - 1) * -0.5 = 0.5; t = 1.5, u = 0.5, (0.5 * -1.5 - 1) * -0.5 =
    /// 0.875.
    #[test]
    fn in_out_quad_arms() {
        assert_eq!(in_out_quad(0.05, 0.2), 0.125);
        assert_eq!(in_out_quad(0.1, 0.2), 0.5);
        assert!((in_out_quad(0.15, 0.2) - 0.875).abs() < 1e-6);
        assert_eq!(in_out_quad(0.2, 0.2), 1.0);
    }
}
