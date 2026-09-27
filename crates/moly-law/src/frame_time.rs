//! The engine's frame clock: how the frame delta time every accumulated
//! timer reads is made from the platform clock, once per frame.
//!
//! Rule, per frame (`time` = platform seconds since start-up):
//! 1. The unscaled delta is `(float)((time - zero_real) - last_real)`, floored
//!    at 1e-5 (a floored frame leaves `last_real` where it was).
//! 2. A skip frame keeps the previous delta time and ends the update.
//! 3. A fixed-step frame advances the scaled clock by `time_scale * 0.02`.
//! 4. Otherwise, with `raw = (time - zero) - cur` in double precision: above
//!    the maximum allowed timestep the clock advances by `maximum *
//!    time_scale` (the time beyond the cap is dropped); below 1e-5 it advances
//!    by `time_scale * 1e-5`; at a time scale within 1e-6 of 1 it becomes
//!    exactly `time - zero`; else it advances by `(float)raw * time_scale`.
//! 5. The delta time is `(float)(cur' - cur)`, and `zero = time - cur'`.
//!
//! The reset the player makes at start-up (with a first delta) zeroes the
//! clocks, sets the delta times to 0.02 and raises both the skip and the
//! fixed-step flag, so the first two frames read 0.02.

/// The project's time settings that the frame clock reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSettings {
    /// The cap on one frame's scaled advance, in seconds.
    pub maximum_allowed_timestep: f32,
    /// The time scale (game code never writes it).
    pub time_scale: f32,
}

/// The fixed step of a fixed-step frame, in seconds.
const FIXED_STEP_FRAME_SECONDS: f32 = 0.02;
/// The floor of one frame's delta, in seconds.
const MINIMUM_DELTA_SECONDS: f32 = 1e-5;
/// How close to 1 the time scale must be for the exact branch.
const UNIT_SCALE_TOLERANCE: f32 = 1e-6;

/// The engine's frame clock.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameClock {
    settings: TimeSettings,
    cur: f64,
    zero: f64,
    zero_real: f64,
    last_real: f64,
    skip_frame: bool,
    fixed_step_frame: bool,
    delta: f32,
    unscaled_delta: f32,
    frames: u64,
}

impl FrameClock {
    /// The start-up reset with a first delta, at platform time `time`.
    pub fn reset_with_first_delta(settings: TimeSettings, time: f64) -> Self {
        Self {
            settings,
            cur: 0.0,
            zero: time,
            zero_real: time,
            last_real: 0.0,
            skip_frame: true,
            fixed_step_frame: true,
            delta: FIXED_STEP_FRAME_SECONDS,
            unscaled_delta: FIXED_STEP_FRAME_SECONDS,
            frames: 0,
        }
    }

    /// One frame's update at platform time `time`.
    pub fn update(&mut self, time: f64) {
        self.frames += 1;
        let unscaled = ((time - self.zero_real) - self.last_real) as f32;
        if unscaled < MINIMUM_DELTA_SECONDS {
            self.unscaled_delta = MINIMUM_DELTA_SECONDS;
        } else {
            self.unscaled_delta = unscaled;
            self.last_real = time - self.zero_real;
        }
        if self.skip_frame {
            self.skip_frame = false;
            return;
        }
        let scale = self.settings.time_scale;
        let next = if self.fixed_step_frame {
            self.fixed_step_frame = false;
            self.cur + f64::from(scale * FIXED_STEP_FRAME_SECONDS)
        } else {
            let raw = (time - self.zero) - self.cur;
            let maximum = self.settings.maximum_allowed_timestep;
            if raw > f64::from(maximum) {
                self.cur + f64::from(maximum * scale)
            } else if raw < f64::from(MINIMUM_DELTA_SECONDS) {
                self.cur + f64::from(scale * MINIMUM_DELTA_SECONDS)
            } else if (scale - 1.0).abs() <= UNIT_SCALE_TOLERANCE {
                time - self.zero
            } else {
                self.cur + f64::from(raw as f32 * scale)
            }
        };
        self.delta = (next - self.cur) as f32;
        self.cur = next;
        self.zero = time - next;
    }

    /// This frame's delta time, in seconds.
    pub fn delta(&self) -> f32 {
        self.delta
    }

    /// This frame's unscaled delta time, in seconds.
    pub fn unscaled_delta(&self) -> f32 {
        self.unscaled_delta
    }

    /// Frames updated since the reset.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The settings the clock runs with.
    pub fn settings(&self) -> TimeSettings {
        self.settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source-value instrument: the engine's own frame-time update executed
    /// as compiled code after its start-up reset at time 0, per case the
    /// cap (bits) and each frame's (platform time, delta-time bits).
    #[allow(clippy::type_complexity)]
    const EXECUTED: [(&str, u32, &[(f64, u32)]); 8] = [
        ("steady 60 Hz after ResetTime(true) at t=0", 0x3EAAAAAB, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (0.1, 0x3c888889), (0.11666666666666667, 0x3c888889), (0.13333333333333333, 0x3c888889)]),
        ("60 Hz then one 1.0 s frame then 60 Hz", 0x3EAAAAAB, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (1.0833333333333333, 0x3eaaaaab), (1.0999999999999999, 0x3c888889), (1.1166666666666667, 0x3c888889)]),
        ("60 Hz then one 5.0 s frame then 60 Hz", 0x3EAAAAAB, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (5.083333333333333, 0x3eaaaaab), (5.1, 0x3c888889)]),
        ("60 Hz then frames of 0.3333333 s (double 1/3), 0.34 s, 0.3333333432674408 s", 0x3EAAAAAB, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (0.41666666666666663, 0x3eaaaaab), (0.7566666666666666, 0x3eaaaaab), (1.0900000099341074, 0x3eaaaaab)]),
        ("60 Hz then a 1e-6 s frame", 0x3EAAAAAB, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (0.08333433333333333, 0x3727c5ac), (0.10000099999999999, 0x3c888889)]),
        ("30 Hz", 0x3EAAAAAB, &[(0.03333333333333333, 0x3ca3d70a), (0.06666666666666667, 0x3ca3d70a), (0.1, 0x3d088889), (0.13333333333333333, 0x3d088889), (0.16666666666666666, 0x3d088889), (0.2, 0x3d088889)]),
        ("4 Hz (0.25 s frames)", 0x3EAAAAAB, &[(0.25, 0x3ca3d70a), (0.5, 0x3ca3d70a), (0.75, 0x3e800000), (1.0, 0x3e800000), (1.25, 0x3e800000), (1.5, 0x3e800000)]),
        ("negative arm: same 5.0 s frame with the cap raised to 10 s", 0x41200000, &[(0.016666666666666666, 0x3ca3d70a), (0.03333333333333333, 0x3ca3d70a), (0.05, 0x3c888889), (0.06666666666666667, 0x3c888889), (0.08333333333333333, 0x3c888889), (5.083333333333333, 0x40a00000), (5.1, 0x3c888889)]),
    ];

    #[test]
    fn frame_delta_equals_the_executed_engine() {
        let mut distinct = std::collections::HashSet::new();
        for (case, cap, rows) in EXECUTED {
            let settings = TimeSettings {
                maximum_allowed_timestep: f32::from_bits(cap),
                time_scale: 1.0,
            };
            let mut clock = FrameClock::reset_with_first_delta(settings, 0.0);
            for (time, bits) in rows {
                clock.update(*time);
                distinct.insert(clock.delta().to_bits());
                assert_eq!(
                    clock.delta().to_bits(),
                    *bits,
                    "{case}: frame at {time}"
                );
            }
        }
        // Positive arm: the start-up 0.02, steady deltas, the cap, the
        // floor and the unclamped long frame are all present.
        assert!(distinct.len() >= 7);
    }
}
