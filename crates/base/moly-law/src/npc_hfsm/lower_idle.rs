//! `IdleLowerState`: the universal lower fallback.
//!
//! The constructor draws its pause once, `Random.Range(5.0f, 8.0f)`, so every
//! fallback the controller builds spends one engine float draw. `OnEnter`
//! stops the presenter and reports the idle action state; `OnUpdate`
//! completes once the base clock reaches the pause. `CanInterrupt` is true
//! (no override). The state carries no other fields.

use super::timing::{ACTION_STATE_IDLE, IDLE_DURATION_MIN};
use super::LowerState;

/// One idle fallback: the drawn pause and the base clock (`_elapsedTime`,
/// `IsComplete`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IdleLower {
    duration: f32,
    elapsed: f32,
    is_complete: bool,
}

impl IdleLower {
    /// `IdleLowerState..ctor(presenter, model, arrivalContext)`: the pause is
    /// drawn now, before the base constructor runs.
    pub fn with_draw(draw: &mut dyn FnMut(f32, f32) -> f32) -> Self {
        Self {
            duration: draw(5.0, 8.0),
            elapsed: 0.0,
            is_complete: false,
        }
    }

    /// `IdleLowerState..ctor(..., float idleDuration)`: a given pause, no
    /// draw.
    pub fn with_duration(duration: f32) -> Self {
        Self {
            duration,
            elapsed: 0.0,
            is_complete: false,
        }
    }

    /// The drawn pause.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// `_elapsedTime`.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.is_complete
    }

    /// `OnEnter`: the presenter stops where it stands and the action state
    /// reports Idle (0). The stop and the state write are the host's.
    pub fn enter(&mut self) -> LowerEnter {
        self.elapsed = 0.0;
        self.is_complete = false;
        LowerEnter {
            stop: true,
            action_state: Some(ACTION_STATE_IDLE),
        }
    }

    /// `OnUpdate`: the base clock takes the frame, and the state completes
    /// once it reaches the pause (`_elapsedTime >= _idleDuration`).
    pub fn update(&mut self, dt: f32) {
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            self.is_complete = true;
        }
    }

    /// `OnExit`: nothing.
    pub fn exit(&mut self) {}

    /// `get_CanInterrupt`: true.
    pub fn can_interrupt(&self) -> bool {
        true
    }

    /// The lower state object the controller holds.
    pub fn state(&self) -> LowerState {
        LowerState::Idle { duration: self.duration }
    }

    /// A fallback with the range's low end as its pause, for a caller that
    /// has no engine draw to spend.
    pub fn undrawn() -> Self {
        Self::with_duration(IDLE_DURATION_MIN)
    }
}

/// What `OnEnter` asks the presenter to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LowerEnter {
    /// `presenter.Stop()`.
    pub stop: bool,
    /// `ChangeStateTypeOnly(state)`, when the state writes one.
    pub action_state: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constructor_draws_the_range_once() {
        let mut calls = 0;
        let mut range = (0.0f32, 0.0f32);
        let idle = IdleLower::with_draw(&mut |low, high| {
            calls += 1;
            range = (low, high);
            6.31
        });
        assert_eq!(calls, 1);
        assert_eq!(range, (5.0, 8.0));
        assert_eq!(idle.duration(), 6.31);
        assert_eq!(idle.state(), LowerState::Idle { duration: 6.31 });
        let fixed = IdleLower::with_duration(2.0);
        assert_eq!(fixed.duration(), 2.0);
    }

    #[test]
    fn it_completes_when_the_clock_reaches_the_pause() {
        let mut idle = IdleLower::with_duration(1.0);
        let enter = idle.enter();
        assert!(enter.stop);
        assert_eq!(enter.action_state, Some(0));
        assert!(!idle.is_complete());
        idle.update(0.5);
        assert!(!idle.is_complete());
        idle.update(0.5);
        assert!(idle.is_complete());
        assert_eq!(idle.elapsed(), 1.0);
        idle.exit();
        assert!(idle.can_interrupt());
    }
}
