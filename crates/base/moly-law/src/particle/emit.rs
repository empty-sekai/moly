//! The emission laws: rate accumulation and burst scheduling. Both are pure
//! functions over cross-frame state with `dt` passed explicitly.
//!
//! Input: the cross-frame accumulator, the rate or burst, and the playhead
//! times of the frame (`prev_time`, `now_time`). Output: the number of
//! particles the frame emits, or the burst's outcome for the frame. The laws
//! never advance a clock and never draw from a stream themselves:
//! [`accumulate_rate`] takes `dt` and [`burst_check`] takes its two random
//! draws from the caller.
//!
//! Evidence: `PlaybackState` has an emission accumulator
//! (`m_ToEmitAccumulator`) beside its playhead (`m_PlaybackTime`), `C#
//! readable (existence)`. Together they show the engine carries the
//! fractional emission across frames and emits in whole particles; the exact
//! rounding behind the extern wall (`Simulate` is native) is a `behavioural
//! spec`, with the measurement plan on each function.

use crate::particle::value::MinMaxCurve;

/// The cross-frame state of rate emission: the fractional emission carried
/// from one frame to the next.
///
/// The field holds the same quantity as the engine's `PlaybackState` field
/// `m_ToEmitAccumulator` (`C# readable (existence)`): particles owed to the
/// system but not yet born.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmissionState {
    pub to_emit_accumulator: f32,
}

impl Default for EmissionState {
    fn default() -> Self {
        Self { to_emit_accumulator: 0.0 }
    }
}

/// The frame's rate emission: `carry += rate * dt`, the whole part (1 or
/// more particles) is emitted, and the remainder, itself between 0.0 and 1.0,
/// stays in the carry.
///
/// Tier: `behavioural spec`. The accumulator's existence is readable; its
/// rounding and its handling of the remainder are not. The rounding is floor:
/// rates are non-negative, so floor and truncation agree (a negative rate is
/// refused at the decoder).
///
/// Editor measurement plan: with rate=1, step 30 frames of `Simulate(dt=0.1)`
/// (the carry grows by 0.1 per frame, 1 emission each, at 10 frames)
/// frames 10, 20 and 30 (0 births before the 10th), where the carry reaches 1.0. Then step with rate=7
/// and dt=1/30: the births per frame should alternate between 0 and 1 (the
/// carry cycles through 7/30), and a frame with 2 would mean another
/// rounding.
pub fn accumulate_rate(state: &mut EmissionState, rate: f32, dt: f32) -> u32 {
    if !(rate >= 0.0) || !(dt >= 0.0) {
        // One predicate catches both negative values and NaN (below 0.0). A
        // refused input emits nothing and leaves the carry as it was.
        return 0;
    }
    state.to_emit_accumulator += rate * dt;
    let whole = state.to_emit_accumulator.floor();
    state.to_emit_accumulator -= whole;
    whole as u32
}

/// The number of times a burst fires. In the serialized `cycleCount`, 0 means
/// repeat without end and a nonzero value means that exact number of times:
/// the Editor's cycle dropdown names item 0 "Infinite" and writes the literal
/// 0, and the managed setter rejects only values below `< 0`. The closed type
/// keeps this apart from a bare `u32`, where 0 could be read as invalid and a
/// minimum of `>= 1` would refuse every long-lived emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstCycles {
    /// Serialized `cycleCount == 0`: repeats every `repeat_interval` without
    /// end.
    Infinite,
    /// Serialized `cycleCount == n > 0`: fires exactly n times.
    Finite(std::num::NonZeroU32),
}

impl BurstCycles {
    /// Builds from the serialized count: 0 is infinite.
    pub fn from_serialized(cycle_count: u32) -> Self {
        match std::num::NonZeroU32::new(cycle_count) {
            Some(n) => Self::Finite(n),
            None => Self::Infinite,
        }
    }
}

/// One burst, held in the serialized form (`cycleCount`, `probability`). The
/// engine encodes two of the fields differently inside itself, `C#
/// readable`:
///
/// - `m_RepeatCount = cycleCount - 1` (the struct default is 0 where the
///   serialized form needs 1);
/// - `m_InvProbability = 1 - probability` (the engine stores the complement).
///
/// The schema layer keeps the serialized form, so these two encodings are
/// named here and a complement cannot reach a consumer as a probability.
#[derive(Debug, Clone, PartialEq)]
pub struct Burst {
    /// The burst time in seconds on the playhead's time axis.
    pub time: f32,
    /// The burst count; it is evaluated when the burst fires, with an explicit
    /// random lerp factor.
    pub count: MinMaxCurve,
    /// The number of times the burst fires. `cycleCount == 0` repeats without
    /// end and is not an invalid value (1 firing when it is 1, 0 meaning all).
    pub cycles: BurstCycles,
    /// The repeat interval. A second and later firing needs `> 0` (`C#
    /// readable`: the interval's setter rejects non-positive values).
    pub repeat_interval: f32,
    /// The firing probability p in \[0,1\]. At p=1 the burst always fires; the
    /// engine stores 1-p.
    pub probability: f32,
}

/// The verdict on one burst for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BurstOutcome {
    /// No firing is due in this frame.
    NotDue,
    /// A firing is due and the probability passed: the count it carries.
    Fired(u32),
    /// A firing was due but the probability failed: the engine still spends
    /// that firing.
    MissedByProbability,
}

/// Whether a burst is due in the frame `[prev_time, now_time]`.
///
/// Tier: `behavioural spec` for the firing condition. The k-th firing is at
/// `t_k = time + k * repeat_interval` (k of at least 1 needs
/// `repeat_interval > 0`, 2 firings or more; an interval of 0 leaves the
/// first one only,
/// `C# readable`), and it is due when `prev_time < t_k <= now_time`: the
/// frame end is inclusive, so a firing exactly at `now_time` belongs to this
/// frame and one exactly at `prev_time` belonged to the previous frame. The
/// probability gate takes `rand_fire` and the count takes `rand_count`, passed
/// separately because whether the engine uses one draw for both cannot be
/// established; the caller fixes that choice, and the law does not guess it.
///
/// Editor measurement plan: a single burst at time=0.5 stepped with
/// `Simulate(dt=0.25)` fires at the end of frame 2 (now=0.5), telling `<=`
/// from `<`; a burst with `cycles=3` and `repeatInterval=1` fires exactly 3
/// times within 60 frames; a burst with `probability=0.5` over 200 seeds fires
/// on close to 1/2.
pub fn burst_check(
    burst: &Burst,
    prev_time: f32,
    now_time: f32,
    rand_fire: f32,
    rand_count: f32,
) -> BurstOutcome {
    if !(now_time >= prev_time) || prev_time.is_nan() || now_time.is_nan() {
        // A playhead going backwards is the caller's loop semantics, and a
        // NaN time is refused; both read as not due.
        return BurstOutcome::NotDue;
    }
    let due_at = |t_k: f32| prev_time < t_k && t_k <= now_time;
    let at_cycle = |k: u32| -> Option<f32> {
        if k == 0 {
            Some(burst.time)
        } else if burst.repeat_interval > 0.0 {
            Some(burst.time + k as f32 * burst.repeat_interval)
        } else {
            // A non-positive interval leaves only the k=0 firing. The
            // engine's setter already refuses an interval at or below 0, so
            // serialized data does not carry one; this is the second guard.
            None
        }
    };
    let hits = match burst.cycles {
        BurstCycles::Finite(n) => (0..n.get()).any(|k| at_cycle(k).is_some_and(due_at)),
        BurstCycles::Infinite => {
            // Infinite cycles are not iterated: solve directly for the k that
            // lands in (prev, now], the smallest k of at least 1 with
            // prev < time + k*interval <= now; k = 0 is tested on its own
            // because it does not depend on the interval.
            due_at(burst.time)
                || (burst.repeat_interval > 0.0 && {
                    let k = (((prev_time - burst.time) / burst.repeat_interval).floor() + 1.0)
                        .max(1.0);
                    k.is_finite() && due_at(burst.time + k * burst.repeat_interval)
                })
        }
    };
    if !hits {
        return BurstOutcome::NotDue;
    }
    // The probability gate passes on `rand < p` (0 draws below p). p=1 always passes: a draw of
    // exactly 1 would fail `1 < 1`, but p=1 means the burst always fires, and
    // that one endpoint must not change it. p=0 never passes (`0 < 0` is
    // false). Measurable in the Editor: p=0.5 over 200 seeds fires on close to
    // 1/2, since the endpoint does not change the statistical shape.
    if rand_fire < burst.probability || burst.probability >= 1.0 {
        let count = burst.count.evaluate(burst.time, rand_count);
        // Counts are non-negative integers; a negative value is damaged data
        // and is floored at 0.
        BurstOutcome::Fired(count.max(0.0) as u32)
    } else {
        BurstOutcome::MissedByProbability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_burst(time: f32) -> Burst {
        Burst {
            time,
            count: MinMaxCurve::Constant(3.0),
            cycles: BurstCycles::from_serialized(1),
            repeat_interval: 0.0,
            probability: 1.0,
        }
    }

    #[test]
    fn serialized_cycle_count_zero_repeats_without_end() {
        // Anchor: the Editor's cycle dropdown names item 0 "Infinite" and
        // writes the literal 0 into the serialized `cycleCount`; the managed
        // setter rejects only `< 0`, and nothing requires `>= 1`. So 0 means
        // 0 is infinite; 1 is one firing, 2 two, and so on.
        // repeat without end. The third assertion below turns red if item 0
        // is instead read as one firing.
        let b = Burst {
            time: 0.5,
            count: MinMaxCurve::Constant(1.0),
            cycles: BurstCycles::from_serialized(0),
            repeat_interval: 1.0,
            probability: 1.0,
        };
        // Firing 0 is due as usual.
        assert_eq!(burst_check(&b, 0.4, 0.6, 0.0, 0.0), BurstOutcome::Fired(1));
        // Firing 1.
        assert_eq!(burst_check(&b, 1.4, 1.6, 0.0, 0.0), BurstOutcome::Fired(1));
        // A distant firing: a finite count stops here, an infinite one does
        // not. This is what tells "infinite" from "exactly N".
        assert_eq!(burst_check(&b, 100.4, 100.6, 0.0, 0.0), BurstOutcome::Fired(1));
        // Nothing fires between firings.
        assert_eq!(burst_check(&b, 100.6, 101.4, 0.0, 0.0), BurstOutcome::NotDue);
        // Paired case: with the same distant window, a finite count of 3 must
        // not fire.
        let finite = Burst { cycles: BurstCycles::from_serialized(3), ..b.clone() };
        assert_eq!(burst_check(&finite, 100.4, 100.6, 0.0, 0.0), BurstOutcome::NotDue);
        // Infinite cycles with a non-positive interval: only firing 0, since
        // the interval never advances.
        let no_interval = Burst { repeat_interval: 0.0, ..b };
        assert_eq!(burst_check(&no_interval, 0.4, 0.6, 0.0, 0.0), BurstOutcome::Fired(1));
        assert_eq!(burst_check(&no_interval, 100.4, 100.6, 0.0, 0.0), BurstOutcome::NotDue);
    }

    #[test]
    fn rate_accumulator_emits_on_whole_frames() {
        // rate=1, dt=0.1: the carry passes through 0.1..0.9 without emitting,
        // then emits 1 at 1.0 and resets.
        let mut s = EmissionState::default();
        let mut born = 0;
        for _ in 0..9 {
            born += accumulate_rate(&mut s, 1.0, 0.1);
        }
        assert_eq!(born, 0);
        assert!((s.to_emit_accumulator - 0.9).abs() < 1e-6);
        assert_eq!(accumulate_rate(&mut s, 1.0, 0.1), 1);
        assert!(s.to_emit_accumulator.abs() < 1e-6);
    }

    #[test]
    fn rate_accumulator_carry_survives_zero_rate_frames() {
        // The remainder survives a zero-rate frame: the 0.2 in the carry is
        // not cleared by a frame that emits nothing.
        let mut s = EmissionState::default();
        accumulate_rate(&mut s, 2.0, 0.1); // carry 0.2
        assert_eq!(accumulate_rate(&mut s, 0.0, 0.5), 0);
        assert!((s.to_emit_accumulator - 0.2).abs() < 1e-6);
        assert_eq!(accumulate_rate(&mut s, 8.0, 0.1), 1); // 0.2+0.8=1.0
    }

    #[test]
    fn rate_accumulator_f32_carry_lands_exactly() {
        // f32(0.1) is slightly above 1/10: ten additions give
        // 1.0000001192092896, so frame 10 already reaches a whole (floor=1.0,
        // carry back to 0). A hand computation of the IEEE f32 additions; it
        // pins the rounding upward and the early landing.
        let mut s = EmissionState::default();
        let mut total = 0u32;
        for _ in 0..10 {
            total += accumulate_rate(&mut s, 1.0, 0.1);
        }
        assert_eq!(total, 1);
        assert!(s.to_emit_accumulator.abs() < 1e-6);
        // Frame 11: the carry was reset and starts again from 0.1.
        assert_eq!(accumulate_rate(&mut s, 1.0, 0.1), 0);
        assert!((s.to_emit_accumulator - 0.1).abs() < 1e-6);
    }

    #[test]
    fn rate_accumulator_rejects_negative_and_nan() {
        let mut s = EmissionState::default();
        assert_eq!(accumulate_rate(&mut s, -1.0, 0.1), 0);
        assert_eq!(accumulate_rate(&mut s, f32::NAN, 0.1), 0);
        assert_eq!(accumulate_rate(&mut s, 1.0, f32::NAN), 0);
        assert_eq!(s.to_emit_accumulator, 0.0);
    }

    #[test]
    fn burst_fires_on_frame_end_boundary() {
        // time=0.5, frame [0.25, 0.5]: t_k equals now, so it fires in this
        // frame (the `<=` of the frame end).
        let b = single_burst(0.5);
        assert_eq!(
            burst_check(&b, 0.25, 0.5, 0.0, 0.0),
            BurstOutcome::Fired(3)
        );
        // The previous frame [0.25, 0.4999] is not due.
        assert_eq!(burst_check(&b, 0.25, 0.4999, 0.0, 0.0), BurstOutcome::NotDue);
        // A frame start equal to t_k: it belonged to the previous frame (the
        // strict `prev < t_k`).
        assert_eq!(burst_check(&b, 0.5, 0.75, 0.0, 0.0), BurstOutcome::NotDue);
    }

    #[test]
    fn burst_repeats_only_with_positive_interval() {
        let b = Burst {
            time: 0.0,
            count: MinMaxCurve::Constant(1.0),
            cycles: BurstCycles::from_serialized(3),
            repeat_interval: 2.0,
            probability: 1.0,
        };
        // Firings at 0 / 2 / 4: frames [1.5, 2.0] and [3.5, 4.0] each fire
        // once, [0.5, 1.5] does not.
        assert_eq!(burst_check(&b, 0.5, 1.5, 0.0, 0.0), BurstOutcome::NotDue);
        assert_eq!(burst_check(&b, 1.5, 2.0, 0.0, 0.0), BurstOutcome::Fired(1));
        assert_eq!(burst_check(&b, 3.5, 4.0, 0.0, 0.0), BurstOutcome::Fired(1));
        // A repeat beyond the cycle count (t=6) no longer fires.
        assert_eq!(burst_check(&b, 5.5, 6.5, 0.0, 0.0), BurstOutcome::NotDue);

        let zero_interval = Burst { repeat_interval: 0.0, cycles: BurstCycles::from_serialized(5), ..b };
        // Interval 0: only firing 0, whatever the cycle count.
        assert_eq!(burst_check(&zero_interval, -0.5, 0.5, 0.0, 0.0), BurstOutcome::Fired(1));
        assert_eq!(burst_check(&zero_interval, 0.5, 5.5, 0.0, 0.0), BurstOutcome::NotDue);
    }

    #[test]
    fn burst_probability_gate_uses_rand() {
        // p=0.25: rand<0.25 passes, rand=0.25 does not (strictly below).
        let b = Burst { probability: 0.25, ..single_burst(0.0) };
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.1, 0.0), BurstOutcome::Fired(3));
        assert_eq!(
            burst_check(&b, -0.1, 0.1, 0.25, 0.0),
            BurstOutcome::MissedByProbability
        );
        assert_eq!(
            burst_check(&b, -0.1, 0.1, 0.99, 0.0),
            BurstOutcome::MissedByProbability
        );
        // p=1 always passes, rand=1 included (the engine stores 1-p=0 and
        // compares strictly below).
        let always = single_burst(0.0);
        assert_eq!(burst_check(&always, -0.1, 0.1, 1.0, 0.0), BurstOutcome::Fired(3));
        // p=0: even rand=0 does not pass (0 < 0 is false).
        let never = Burst { probability: 0.0, ..single_burst(0.0) };
        assert_eq!(
            burst_check(&never, -0.1, 0.1, 0.0, 0.0),
            BurstOutcome::MissedByProbability
        );
    }

    #[test]
    fn burst_count_two_constants_uses_lerp() {
        // count TwoConstants{min=2,max=6} with rand_count=0.5 gives 4.
        let b = Burst {
            count: MinMaxCurve::TwoConstants { min: 2.0, max: 6.0 },
            ..single_burst(0.0)
        };
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.0, 0.5), BurstOutcome::Fired(4));
        assert_eq!(burst_check(&b, -0.1, 0.1, 0.0, 0.0), BurstOutcome::Fired(2));
    }

    #[test]
    fn burst_inverted_playhead_is_refused() {
        let b = single_burst(0.5);
        assert_eq!(burst_check(&b, 1.0, 0.5, 0.0, 0.0), BurstOutcome::NotDue);
        assert_eq!(burst_check(&b, f32::NAN, 0.5, 0.0, 0.0), BurstOutcome::NotDue);
    }
}
