//! Guarded ordinary Update1 / EmitOverTime / StartParticles scheduling.
//!
//! Current JP libunity 937c6d28...75badd9. Evidence: a native boundary replay
//! (405 scalar emission, 108 clock, 15 newborn cases) and a native EmitOverTime
//! receipt over every two-constant emission configuration of the corpus, both
//! with zero failures.
//! Scope: initialized clock, no delay or distance emission, one incremental
//! slice per call (the per-frame head supplies the slices; their births-ahead
//! argument is applied by the birth placement, not here), a constant or
//! two-constant rate, <=2 cycle-1 bursts of probability 1 with a constant or
//! two-constant count.
//! This prepares phases; it never admits a system or simulates particles.
//! Never use an old emission-head interval for birth curves after a loop wrap:
//! StartParticles uses (current - slice_dt, current), both times native reciprocal
//! duration. Prepare clock -> existing pre/sim/death/post -> schedule -> births.

use crate::particle::emit::BurstCycles;
use crate::particle::schema::EmissionParams;
use crate::particle::seed_owner::ScalarRandom;
use crate::particle::sub_emission::{BirthDistribution, BirthTiming};
use crate::particle::value::MinMaxCurve;

const EPSILON: f32 = f32::from_bits(0x3586_37bd);
const MIN_AMOUNT: f32 = f32::from_bits(0x38d1_b717);
const MAX_COUNT: f32 = 16_777_215.0;
const MAX_WRAPS: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    UnsupportedConfiguration,
    InvalidInput,
    CountOutOfRange,
    ReciprocalUnavailable,
    UnverifiedClockRange,
}

/// EmitOverTime scales the low 23 bits of its entry draw by 2^-23 * (1 + 2^-23),
/// so a draw whose low 23 bits are all ones gives exactly 1.0.
const UNIT_SCALE: f32 = f32::from_bits(0x3400_0001);

/// Rate over time in the two MinMaxCurve::Evaluate modes EmitOverTime is
/// qualified for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EmissionRate {
    /// Constant mode reads only the max scalar; the min scalar is never read.
    Constant(f32),
    /// TwoConstants: min + r * (max - min) as three separately rounded
    /// operations, r coming from this call's entry draw. Evaluate neither
    /// orders min and max nor clamps r.
    TwoConstants { min: f32, max: f32 },
}

impl EmissionRate {
    /// EmitOverTime decides whether to evaluate the rate from the max scalar
    /// alone (the constant itself in Constant mode): at or below zero, or NaN,
    /// the amount is +0 and Evaluate is not called. The min is not read here.
    fn gate(self) -> f32 {
        match self {
            Self::Constant(value) => value,
            Self::TwoConstants { max, .. } => max,
        }
    }

    /// Evaluate's result followed by EmitOverTime's maximum with +0, which
    /// sends a negative value, -0 and NaN to +0. Written as a comparison
    /// because f32::max leaves the sign of a zero result unspecified.
    fn value(self, r: f32) -> f32 {
        let value = match self {
            Self::Constant(value) => value,
            Self::TwoConstants { min, max } => min + r * (max - min),
        };
        if value > 0.0 {
            value
        } else {
            0.0
        }
    }
}

/// Burst count in the two modes EmissionModule::AccumulateBurst is qualified
/// for. The probability is 1, so AccumulateBurst takes no probability draw.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BurstCount {
    /// Constant mode: the max scalar truncated toward zero; no draw.
    Constant(u32),
    /// TwoConstants: lo and hi are the smaller and the larger constant, each
    /// truncated toward zero. Every hit takes one full 32-bit draw, even when
    /// lo == hi, and the count is lo + word % (hi + 1 - lo) on the whole word.
    /// Built only by the gate, which keeps lo <= hi <= 16,777,215.
    TwoConstants { lo: u32, hi: u32 },
}

impl BurstCount {
    fn sample(self, random: &mut ScalarRandom) -> u32 {
        match self {
            Self::Constant(count) => count,
            Self::TwoConstants { lo, hi } => lo + random.next_u32() % (hi + 1 - lo),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScheduledBurst {
    time: f32,
    count: BurstCount,
}

#[derive(Clone, Debug)]
pub struct ConstantAutonomousEmission {
    duration: f32,
    looping: bool,
    rate: EmissionRate,
    bursts: Vec<ScheduledBurst>,
}

/// Emission's three scalar fields and its separate scalar Rand stream.
/// Seed this RNG from the owner's ResetStreams emission stream, not Initial RandN.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutonomousEmissionState {
    pub distribution: BirthDistribution,
    pub random: ScalarRandom,
}

impl AutonomousEmissionState {
    pub fn initialized(random: ScalarRandom) -> Self {
        Self {
            distribution: BirthDistribution {
                spacing: 0.0,
                offset: 0.0,
                burst_fraction: 0.0,
            },
            random,
        }
    }
}

/// A prepared single normal Update1 slice. Native clock changes before old
/// particle simulation; emission state changes only after old post simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockSlice {
    pub previous: f32,
    pub current: f32,
    /// EmitOverTime's previous parameter, including Update1's >=duration epsilon.
    pub emission_previous: f32,
    pub dt: f32,
    pub loop_count_increment: u32,
    pub stopped_after: bool,
    pub simulate: bool,
    emit: bool,
    duration: f32,
    looping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutonomousBirthBatch {
    pub rate_count: u32,
    pub total: u32,
    pub distribution: BirthDistribution,
    /// Raw StartParticles birth endpoints; previous can be negative at a wrap.
    pub previous: f32,
    pub current: f32,
    pub previous_normalized: f32,
    pub current_normalized: f32,
    pub emission_previous: f32,
    pub dt: f32,
}

impl AutonomousBirthBatch {
    /// Called in original local birth order, before four-wide death and packing.
    pub fn timing(self, index: u32) -> Result<BirthTiming, Refused> {
        // Padding lanes are real native lanes. Permit them past total, but keep
        // index within exactly representable native count range.
        self.distribution
            .timing(
                index,
                self.rate_count,
                self.dt,
                self.previous_normalized,
                self.current_normalized,
            )
            .map_err(|_| Refused::InvalidInput)
    }
}

impl ConstantAutonomousEmission {
    pub fn from_params(
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<Self, Refused> {
        let unsupported = Refused::UnsupportedConfiguration;
        if !matches!(delay, MinMaxCurve::Constant(v) if *v == 0.0)
            || !matches!(&emission.rate_over_distance, MinMaxCurve::Constant(v) if *v == 0.0)
            || !duration.is_finite()
            || duration <= 0.0
            || emission.bursts.len() > 2
        {
            return Err(unsupported);
        }
        let rate = rate(&emission.rate_over_time)?;
        let mut bursts = Vec::with_capacity(emission.bursts.len());
        for b in &emission.bursts {
            if b.probability != 1.0
                || !matches!(b.cycles, BurstCycles::Finite(n) if n.get() == 1)
                || !b.time.is_finite()
                || b.time < 0.0
                || b.time > duration
            {
                return Err(unsupported);
            }
            let count = burst_count(&b.count)?;
            // Preserve authoring order: hits in one window draw their counts in
            // this order, and the last hit owns the one burst fraction.
            bursts.push(ScheduledBurst {
                time: b.time,
                count,
            });
        }
        Ok(Self {
            duration,
            looping,
            rate,
            bursts,
        })
    }

    /// This is one normal slice with no start delay: `dt` is the slice, never
    /// the frame's pending time. A frame whose pending time exceeds one slice
    /// runs each native slice separately and places its births with that
    /// slice's backtrack argument; passing the pending time here is wrong.
    pub fn prepare_slice(
        &self,
        previous: f32,
        dt: f32,
        stopped: bool,
    ) -> Result<ClockSlice, Refused> {
        if !previous.is_finite()
            || previous < 0.0
            || previous > self.duration
            || !dt.is_finite()
            || dt < 0.0
            || (self.looping && previous >= self.duration)
        {
            return Err(Refused::InvalidInput);
        }
        let active = dt >= EPSILON;
        let mut current = previous;
        let mut loops = 0;
        if active {
            current += dt;
            if !current.is_finite() {
                return Err(Refused::UnverifiedClockRange);
            }
            if self.looping {
                while current >= self.duration {
                    if loops == MAX_WRAPS {
                        return Err(Refused::UnverifiedClockRange);
                    }
                    let next = current - self.duration;
                    if next == current {
                        return Err(Refused::UnverifiedClockRange);
                    }
                    current = next;
                    loops += 1;
                }
            } else {
                current = current.min(self.duration);
            }
        }
        let stopped_after = stopped || (active && !self.looping && current >= self.duration);
        Ok(ClockSlice {
            previous,
            current,
            emission_previous: previous.min(self.duration)
                + if dt >= self.duration { EPSILON } else { 0.0 },
            dt,
            loop_count_increment: loops,
            stopped_after,
            simulate: active,
            emit: active && !stopped_after,
            duration: self.duration,
            looping: self.looping,
        })
    }

    /// Invoke after existing particle simulation and death have released pool
    /// capacity. Even a zero-rate zero-count native call advances scalar RNG:
    /// EmitOverTime draws once at entry, and a two-constant burst count draws
    /// once more per hit. Refusals leave state untouched. Supply the native
    /// reciprocal duration instruction result; scalar 1/duration is not
    /// bit-exact for all durations.
    pub fn schedule(
        &self,
        clock: ClockSlice,
        state: &mut AutonomousEmissionState,
        reciprocal: impl FnOnce(f32) -> Option<f32>,
    ) -> Result<Option<AutonomousBirthBatch>, Refused> {
        if clock.duration != self.duration || clock.looping != self.looping {
            return Err(Refused::InvalidInput);
        }
        if !clock.emit {
            return Ok(None);
        }
        let old = state.distribution;
        if ![old.spacing, old.offset, old.burst_fraction]
            .iter()
            .all(|v| v.is_finite())
            || old.spacing < 0.0
            || !(0.0..1.0).contains(&old.offset)
        {
            return Err(Refused::InvalidInput);
        }
        let previous = clock.emission_previous.max(0.0);
        let current = clock.current.max(0.0);
        // Staged scalar stream, committed together with the distribution.
        let mut random = state.random;
        // The entry draw is unconditional and comes before the rate gate; the
        // two-constant rate takes its random factor from it.
        let word = random.next_u32();
        let amount = if self.rate.gate() > 0.0 {
            let r = (word & 0x7f_ffff) as f32 * UNIT_SCALE;
            // On a wrap native evaluates the rate once per segment with the
            // same random factor, so both segments use the same value.
            let rate = self.rate.value(r);
            if current < previous {
                // Native two separate rounded products followed by a rounded sum.
                current * rate + (self.duration - previous) * rate
            } else {
                (current - previous) * rate
            }
        } else {
            0.0
        };
        if !amount.is_finite() || amount < 0.0 {
            return Err(Refused::InvalidInput);
        }
        let mut burst_fraction = old.burst_fraction;
        let mut burst_count = 0_u32;
        // AccumulateBursts: per hit the count is taken before the fraction is
        // written, and the fraction is written on every hit, zero counts too.
        let mut accumulate = |low: f32, high: f32, random: &mut ScalarRandom| -> Result<(), Refused> {
            for b in &self.bursts {
                if low <= b.time && b.time < high {
                    let count = b.count.sample(random);
                    burst_count = burst_count
                        .checked_add(count)
                        .ok_or(Refused::CountOutOfRange)?;
                    let relative = (b.time - low) / (high - low);
                    burst_fraction = if relative < 0.0 {
                        1.0
                    } else {
                        1.0 - relative.min(1.0)
                    };
                }
            }
            Ok(())
        };
        if current < previous {
            // A wrap draws for the window [0, current) before the window
            // [previous, duration + 1e-4).
            accumulate(0.0, current, &mut random)?;
            accumulate(previous, self.duration + MIN_AMOUNT, &mut random)?;
        } else {
            accumulate(previous, current, &mut random)?;
        }
        let full = amount + old.offset;
        if !full.is_finite() || full > MAX_COUNT {
            return Err(Refused::CountOutOfRange);
        }
        let rate_count = full as u32;
        let total = rate_count
            .checked_add(burst_count)
            .ok_or(Refused::CountOutOfRange)?;
        if total > MAX_COUNT as u32 {
            return Err(Refused::CountOutOfRange);
        }
        let distribution = BirthDistribution {
            spacing: if amount < MIN_AMOUNT {
                1.0
            } else {
                1.0 / amount
            },
            offset: full - rate_count as f32,
            burst_fraction,
        };
        let inverse = reciprocal(self.duration)
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or(Refused::ReciprocalUnavailable)?;
        let birth_previous = clock.current - clock.dt;
        let previous_normalized = birth_previous * inverse;
        let current_normalized = clock.current * inverse;
        if !previous_normalized.is_finite() || !current_normalized.is_finite() {
            return Err(Refused::InvalidInput);
        }
        let mut next = *state;
        next.random = random;
        next.distribution = distribution;
        *state = next;
        Ok(Some(AutonomousBirthBatch {
            rate_count,
            total,
            distribution,
            previous: birth_previous,
            current: clock.current,
            previous_normalized,
            current_normalized,
            emission_previous: clock.emission_previous,
            dt: clock.dt,
        }))
    }
}

/// The rate over time as EmitOverTime reads it. Every finite constant is
/// admitted: a max at or below zero skips evaluation and a negative evaluated
/// value is raised to +0, both as native. Curve modes are not transcribed.
fn rate(curve: &MinMaxCurve) -> Result<EmissionRate, Refused> {
    match *curve {
        MinMaxCurve::Constant(value) if value.is_finite() => Ok(EmissionRate::Constant(value)),
        MinMaxCurve::TwoConstants { min, max } if min.is_finite() && max.is_finite() => {
            Ok(EmissionRate::TwoConstants { min, max })
        }
        _ => Err(Refused::UnsupportedConfiguration),
    }
}

/// The burst count as AccumulateBurst reads it: each constant is truncated
/// toward zero, so a value in (-1, 0) counts 0. TwoConstants orders the pair
/// itself, so a swapped pair is admitted. A count that truncates to a negative
/// number is refused: native adds it to the total as a signed value, which the
/// unsigned birth count cannot carry.
fn burst_count(curve: &MinMaxCurve) -> Result<BurstCount, Refused> {
    match *curve {
        MinMaxCurve::Constant(value) if value.is_finite() => {
            if value <= -1.0 {
                return Err(Refused::UnsupportedConfiguration);
            }
            if value > MAX_COUNT {
                return Err(Refused::CountOutOfRange);
            }
            // Truncation toward zero; the value lies in (-1, 2^24).
            Ok(BurstCount::Constant(value as u32))
        }
        MinMaxCurve::TwoConstants { min, max } if min.is_finite() && max.is_finite() => {
            // The smaller and the larger, chosen by the same strict comparisons
            // native uses.
            let low = if max < min { max } else { min };
            let high = if min < max { max } else { min };
            if low <= -1.0 {
                return Err(Refused::UnsupportedConfiguration);
            }
            if high > MAX_COUNT {
                return Err(Refused::CountOutOfRange);
            }
            // Truncation toward zero; both values lie in (-1, 2^24).
            Ok(BurstCount::TwoConstants {
                lo: low as u32,
                hi: high as u32,
            })
        }
        _ => Err(Refused::UnsupportedConfiguration),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{self, Value};
    fn law(looping: bool) -> ConstantAutonomousEmission {
        ConstantAutonomousEmission {
            duration: 1.0,
            looping,
            rate: EmissionRate::Constant(4.0),
            bursts: vec![ScheduledBurst {
                time: 0.0,
                count: BurstCount::Constant(1),
            }],
        }
    }
    fn state() -> AutonomousEmissionState {
        AutonomousEmissionState::initialized(ScalarRandom {
            words: [17, 19, 127, 2471805022],
        })
    }
    #[test]
    fn zero_boundary_is_half_open_and_rng_advances_on_zero_count() {
        let law = law(true);
        let mut s = state();
        assert_eq!(
            law.schedule(law.prepare_slice(0.0, 0.0, false).unwrap(), &mut s, |_| {
                Some(1.0)
            }),
            Ok(None)
        );
        let b = law
            .schedule(law.prepare_slice(0.0, 0.25, false).unwrap(), &mut s, |_| {
                Some(1.0)
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            (b.rate_count, b.total, b.distribution.burst_fraction),
            (1, 2, 1.0)
        );
        let words = s.random.words;
        let b = law
            .schedule(
                law.prepare_slice(0.25, EPSILON, false).unwrap(),
                &mut s,
                |_| Some(1.0),
            )
            .unwrap()
            .unwrap();
        assert_eq!(b.total, 0);
        assert_eq!(b.distribution.burst_fraction, 1.0);
        assert_ne!(words, s.random.words);
    }
    #[test]
    fn exact_wrap_has_no_zero_burst_and_birth_previous_is_negative() {
        let law = law(true);
        let mut s = state();
        let b = law
            .schedule(
                law.prepare_slice(0.75, 0.25, false).unwrap(),
                &mut s,
                |_| Some(1.0),
            )
            .unwrap()
            .unwrap();
        assert_eq!((b.rate_count, b.total), (1, 1));
        assert_eq!(
            (b.emission_previous, b.previous, b.current),
            (0.75, -0.25, 0.0)
        );
        assert_eq!(b.timing(0).unwrap().curve_time, 0.0);
    }
    #[test]
    fn multi_loop_tick_is_not_multiple_emission_calls() {
        let law = law(true);
        let mut s = state();
        let clock = law.prepare_slice(0.0, 2.0, false).unwrap();
        assert_eq!(clock.loop_count_increment, 2);
        let b = law.schedule(clock, &mut s, |_| Some(1.0)).unwrap().unwrap();
        assert_eq!((b.rate_count, b.total), (3, 3));
        assert_eq!(b.distribution.offset, 0.9999959468841553);
    }
    #[test]
    fn nonloop_final_slice_simulates_existing_but_suppresses_births() {
        let law = law(false);
        let mut s = state();
        let before = s;
        let clock = law.prepare_slice(0.75, 0.25, false).unwrap();
        assert!(clock.simulate && clock.stopped_after);
        assert_eq!(law.schedule(clock, &mut s, |_| None), Ok(None));
        assert_eq!(s, before);
    }
    #[test]
    fn missing_reciprocal_is_transactional() {
        let law = law(true);
        let mut s = state();
        let before = s;
        assert_eq!(
            law.schedule(law.prepare_slice(0.0, 0.25, false).unwrap(), &mut s, |_| {
                None
            }),
            Err(Refused::ReciprocalUnavailable)
        );
        assert_eq!(s, before);
    }

    fn at<'a>(value: &'a Value, key: &str) -> &'a Value {
        value
            .get(key)
            .unwrap_or_else(|| panic!("missing receipt field {key}"))
    }
    fn array(value: &Value) -> &[Value] {
        value.as_array().unwrap()
    }
    fn number(value: &Value) -> f32 {
        value.as_f64().unwrap() as f32
    }
    fn field(value: &Value, key: &str) -> f32 {
        number(at(value, key))
    }
    fn boolean(value: &Value, key: &str) -> bool {
        at(value, key).as_bool().unwrap()
    }
    fn distribution(value: &Value) -> BirthDistribution {
        let values = array(value);
        BirthDistribution {
            spacing: number(&values[0]),
            offset: number(&values[1]),
            burst_fraction: number(&values[2]),
        }
    }
    fn exact(actual: f32, expected: f32, message: &str) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{message}: {actual} vs {expected}"
        );
    }
    fn exact_distribution(actual: BirthDistribution, expected: BirthDistribution) {
        exact(actual.spacing, expected.spacing, "native spacing FDIV");
        exact(actual.offset, expected.offset, "native carry");
        exact(
            actual.burst_fraction,
            expected.burst_fraction,
            "native burst fraction",
        );
    }

    /// Actual Rust scheduler versus every saved native scalar/clock observation.
    /// The 15 StartParticles fixtures validate this law's timing/normalization
    /// inputs only; newborn pool math belongs to the runtime's separate replay.
    #[test]
    #[ignore = "MOLY_AUTONOMOUS_NATIVE_DIR must identify private current-native receipts"]
    fn replays_current_native_emission_clock_and_birth_timing() {
        let directory = std::env::var_os("MOLY_AUTONOMOUS_NATIVE_DIR")
            .map(std::path::PathBuf::from)
            .expect("supply external current native evidence directory");
        let receipt =
            json::parse(&std::fs::read(directory.join("autonomous-birth-native.json")).unwrap())
                .unwrap();
        let replay = at(&receipt, "boundaryReplay");
        assert_eq!(field(replay, "failureCount"), 0.0);
        let direct = array(at(replay, "directExamples"));
        assert_eq!(direct.len(), 405);
        for (index, row) in direct.iter().enumerate() {
            let duration = field(row, "duration");
            let current = field(row, "current");
            let previous = field(row, "previous");
            let law = ConstantAutonomousEmission {
                duration,
                looping: true,
                rate: EmissionRate::Constant(field(row, "rate")),
                bursts: array(at(row, "bursts"))
                    .iter()
                    .map(|b| {
                        let b = array(b);
                        ScheduledBurst {
                            time: number(&b[0]),
                            count: BurstCount::Constant(number(&b[1]) as u32),
                        }
                    })
                    .collect(),
            };
            let mut state = state();
            state.distribution = distribution(at(row, "before"));
            // Direct native EmitOverTime receipts supply an independent helper
            // interval, including negative previous and exact zero-width calls.
            // Construct that prepared internal interval without inventing Tick.
            let clock = ClockSlice {
                previous,
                current,
                emission_previous: previous,
                dt: 0.25,
                loop_count_increment: 0,
                stopped_after: false,
                simulate: true,
                emit: true,
                duration,
                looping: true,
            };
            let actual = law
                .schedule(
                    clock,
                    &mut state,
                    crate::particle::initial::initial_reciprocal,
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                actual.rate_count,
                field(row, "rateCount") as u32,
                "direct {index}"
            );
            assert_eq!(actual.total, field(row, "total") as u32, "direct {index}");
            exact_distribution(actual.distribution, distribution(at(row, "distribution")));
            assert_eq!(
                state.random.words,
                std::array::from_fn(|i| array(at(row, "rng"))[i].as_f64().unwrap() as u32)
            );
        }
        let clocks = array(at(replay, "clockRows"));
        assert_eq!(clocks.len(), 108);
        for (index, row) in clocks.iter().enumerate() {
            let law = law(boolean(row, "looping"));
            let mut state = state();
            let before = state;
            let clock = law
                .prepare_slice(
                    field(row, "previous"),
                    field(row, "dt"),
                    boolean(row, "stoppedBefore"),
                )
                .unwrap();
            exact(clock.current, field(row, "current"), "Tick current");
            assert_eq!(
                clock.loop_count_increment,
                field(row, "loopCount") as u32,
                "clock {index}"
            );
            assert_eq!(
                clock.stopped_after,
                boolean(row, "stoppedAfter"),
                "clock {index}"
            );
            let events = array(at(row, "events"));
            let event = |name: &str| {
                events
                    .iter()
                    .find(|e| at(e, "phase").as_str() == Some(name))
            };
            let actual = law
                .schedule(
                    clock,
                    &mut state,
                    crate::particle::initial::initial_reciprocal,
                )
                .unwrap();
            if let Some(native) = event("emission") {
                let actual = actual.unwrap();
                exact(
                    actual.emission_previous,
                    field(native, "previous"),
                    "EmitOverTime previous",
                );
                exact(
                    actual.current,
                    field(native, "current"),
                    "EmitOverTime current",
                );
                let start = event("startParticles").unwrap();
                assert_eq!(
                    actual.rate_count,
                    field(start, "rateCount") as u32,
                    "clock {index}"
                );
                assert_eq!(
                    actual.total,
                    field(start, "requested") as u32,
                    "clock {index}"
                );
                exact(actual.dt, field(start, "dt"), "StartParticles dt");
                let mut random = before.random;
                random.next_u32();
                assert_eq!(state.random, random);
                if let Some(modules) = event("startModules") {
                    for value in array(at(modules, "previousNormalized")) {
                        exact(
                            actual.previous_normalized,
                            number(value),
                            "StartModules previous normalized",
                        );
                    }
                    for value in array(at(modules, "currentNormalized")) {
                        exact(
                            actual.current_normalized,
                            number(value),
                            "StartModules current normalized",
                        );
                    }
                }
            } else {
                assert!(actual.is_none(), "clock {index}");
                assert_eq!(
                    state, before,
                    "suppressed call must leave RNG and carry alone"
                );
            }
            exact_distribution(state.distribution, distribution(at(row, "distribution")));
        }
        let births = array(at(replay, "newbornRows"));
        assert_eq!(births.len(), 15);
        let mut lanes = 0;
        for row in births {
            let aligned = (field(row, "old") as u32 + 3) & !3;
            let inverse = crate::particle::initial::initial_reciprocal(2.0).unwrap();
            let batch = AutonomousBirthBatch {
                rate_count: 3,
                total: 3,
                distribution: BirthDistribution {
                    spacing: 0.25,
                    offset: 0.0,
                    burst_fraction: 0.0,
                },
                previous: 0.0,
                current: 0.5,
                previous_normalized: 0.0 * inverse,
                current_normalized: 0.5 * inverse,
                emission_previous: 0.0,
                dt: 0.5,
            };
            for event in array(at(row, "timingEvents")) {
                match at(event, "phase").as_str().unwrap() {
                    "pre" | "startVelocity" => {
                        let key = if at(event, "phase").as_str() == Some("pre") {
                            "dt"
                        } else {
                            "curveTime"
                        };
                        let offset = field(event, "start") as u32 - aligned;
                        for (lane, expected) in array(at(event, key)).iter().enumerate() {
                            let timing = batch.timing(offset + lane as u32).unwrap();
                            exact(
                                if key == "dt" {
                                    timing.dt
                                } else {
                                    timing.curve_time
                                },
                                number(expected),
                                "birth lane timing",
                            );
                            lanes += 1;
                        }
                    }
                    "startModules" => {
                        for value in array(at(event, "previousNormalized")) {
                            exact(
                                batch.previous_normalized,
                                number(value),
                                "birth previous normalized",
                            );
                        }
                        for value in array(at(event, "currentNormalized")) {
                            exact(
                                batch.current_normalized,
                                number(value),
                                "birth current normalized",
                            );
                        }
                    }
                    unknown => panic!("unexpected native phase {unknown}"),
                }
            }
        }
        assert_eq!(lanes, 120);
        eprintln!("native autonomous law: 405 scalar calls, 108 clock calls, 15 birth timing cases / 120 lane observations; exact f32 bits and RNG words");
    }

    /// Exact receipt reader: a bit pattern or word is an integral JSON number
    /// inside the u32 range.
    fn word(value: &Value) -> u32 {
        let v = value.as_f64().expect("receipt word is a number");
        assert!(
            v.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&v),
            "receipt word {v}"
        );
        v as u32
    }
    fn bits_at(value: &Value, key: &str) -> f32 {
        f32::from_bits(word(at(value, key)))
    }
    /// A native 64-bit count, known exactly below 2^53; None above that, which
    /// lies far beyond every count this law can produce.
    fn native_count(value: &Value) -> Option<u64> {
        let v = value.as_f64().expect("receipt count is a number");
        assert!(v.fract() == 0.0 && v >= 0.0, "receipt count {v}");
        (v < 9_007_199_254_740_992.0).then_some(v as u64)
    }
    fn beyond_count_range(value: &Value) -> bool {
        native_count(value).map_or(true, |n| n > MAX_COUNT as u64)
    }
    fn native_curve(value: &Value) -> MinMaxCurve {
        match at(value, "mode").as_str() {
            Some("constant") => MinMaxCurve::Constant(bits_at(value, "bits")),
            Some("twoConstants") => MinMaxCurve::TwoConstants {
                min: bits_at(value, "minBits"),
                max: bits_at(value, "maxBits"),
            },
            other => panic!("receipt curve mode {other:?} is not a constant mode"),
        }
    }
    /// The emission block the native configuration was written from, through
    /// the production gate.
    fn native_law(config: &Value) -> Result<ConstantAutonomousEmission, Refused> {
        let emission = EmissionParams {
            rate_over_time: native_curve(at(config, "rate")),
            rate_over_distance: MinMaxCurve::Constant(0.0),
            bursts: array(at(config, "bursts"))
                .iter()
                .map(|b| crate::particle::emit::Burst {
                    time: bits_at(b, "timeBits"),
                    count: native_curve(at(b, "count")),
                    cycles: BurstCycles::from_serialized(word(at(b, "cycles"))),
                    repeat_interval: bits_at(b, "intervalBits"),
                    probability: bits_at(b, "probabilityBits"),
                })
                .collect(),
        };
        ConstantAutonomousEmission::from_params(
            &MinMaxCurve::Constant(0.0),
            bits_at(config, "durationBits"),
            boolean(config, "looping"),
            &emission,
        )
    }
    fn native_state(value: &Value, rng: &Value) -> AutonomousEmissionState {
        AutonomousEmissionState {
            distribution: BirthDistribution {
                spacing: bits_at(value, "spacing"),
                offset: bits_at(value, "offset"),
                burst_fraction: bits_at(value, "burstFraction"),
            },
            random: ScalarRandom {
                words: std::array::from_fn(|i| word(&array(rng)[i])),
            },
        }
    }
    fn recorded_state(value: &Value) -> AutonomousEmissionState {
        native_state(value, at(value, "rng"))
    }
    fn state_bits(state: &AutonomousEmissionState) -> ([u32; 3], [u32; 4]) {
        let d = state.distribution;
        (
            [
                d.spacing.to_bits(),
                d.offset.to_bits(),
                d.burst_fraction.to_bits(),
            ],
            state.random.words,
        )
    }
    /// A prepared slice built from EmitOverTime's own arguments, for native
    /// calls that no Tick produced (as the constant replay above does).
    fn direct_clock(law: &ConstantAutonomousEmission, previous: f32, current: f32) -> ClockSlice {
        ClockSlice {
            previous,
            current,
            emission_previous: previous,
            dt: 0.25,
            loop_count_increment: 0,
            stopped_after: false,
            simulate: true,
            emit: true,
            duration: law.duration,
            looping: law.looping,
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum NativeRows {
        /// Direct EmitOverTime calls chained on one emission state; each call
        /// records the slice the harness prepared for it.
        Direct,
        /// Real Update1 slices: Tick, EmitOverTime and StartParticles.
        Update1,
        /// One direct call with no recorded slice.
        Control,
    }

    /// Replays one native chain through the production gate, prepare_slice and
    /// schedule, carrying the Rust state from call to call. Returns the number
    /// of calls replayed, or the gate's refusal.
    fn replay_native_chain(
        label: &str,
        config: &Value,
        rows: &[Value],
        kind: NativeRows,
        direct_clocks: &mut usize,
    ) -> Result<usize, Refused> {
        let law = native_law(config)?;
        let mut state = match kind {
            NativeRows::Update1 => {
                native_state(at(&rows[0], "stateBefore"), at(&rows[0], "rngEntry"))
            }
            NativeRows::Direct | NativeRows::Control => recorded_state(at(&rows[0], "stateBefore")),
        };
        for (index, row) in rows.iter().enumerate() {
            let context = format!("{label} call {index}");
            let (clock, arguments, before, after, rate_count, total) = match kind {
                NativeRows::Direct => {
                    let tick = at(row, "clock");
                    let previous = bits_at(tick, "previous");
                    let dt = bits_at(tick, "dt");
                    let clock = if previous < 0.0 {
                        // The harness drove EmitOverTime from a negative slice
                        // start, which slice preparation refuses; replay the
                        // call from its recorded arguments instead.
                        assert_eq!(
                            law.prepare_slice(previous, dt, false),
                            Err(Refused::InvalidInput),
                            "{context}"
                        );
                        *direct_clocks += 1;
                        direct_clock(&law, bits_at(row, "previous"), bits_at(row, "current"))
                    } else {
                        law.prepare_slice(previous, dt, false)
                            .unwrap_or_else(|e| panic!("{context}: slice refused {e:?}"))
                    };
                    (
                        clock,
                        [
                            bits_at(row, "previous"),
                            bits_at(row, "current"),
                            bits_at(row, "duration"),
                        ],
                        recorded_state(at(row, "stateBefore")),
                        recorded_state(at(row, "stateAfter")),
                        at(row, "rateCount"),
                        at(row, "total"),
                    )
                }
                NativeRows::Update1 => {
                    assert_eq!(word(at(row, "emitOverTimeCalls")), 1, "{context}");
                    let clock = law
                        .prepare_slice(bits_at(row, "clockBefore"), bits_at(row, "dt"), false)
                        .unwrap_or_else(|e| panic!("{context}: slice refused {e:?}"));
                    exact(
                        clock.current,
                        bits_at(row, "clockAfter"),
                        &format!("{context}: Tick current"),
                    );
                    let start = at(row, "startParticles");
                    assert_eq!(
                        native_count(at(start, "requested")),
                        native_count(at(row, "emitReturn")),
                        "{context}: StartParticles request"
                    );
                    let before = native_state(at(row, "stateBefore"), at(row, "rngEntry"));
                    let after = native_state(at(row, "stateAfter"), at(row, "rngExit"));
                    // The emission state's recorded words are the stream's words.
                    assert_eq!(
                        state_bits(&recorded_state(at(row, "stateBefore"))),
                        state_bits(&before),
                        "{context}"
                    );
                    assert_eq!(
                        state_bits(&recorded_state(at(row, "stateAfter"))),
                        state_bits(&after),
                        "{context}"
                    );
                    let args = array(at(row, "emitArgs"));
                    (
                        clock,
                        [
                            f32::from_bits(word(&args[0])),
                            f32::from_bits(word(&args[1])),
                            f32::from_bits(word(&args[2])),
                        ],
                        before,
                        after,
                        at(start, "rateCount"),
                        at(row, "emitReturn"),
                    )
                }
                NativeRows::Control => (
                    direct_clock(&law, bits_at(row, "previous"), bits_at(row, "current")),
                    [
                        bits_at(row, "previous"),
                        bits_at(row, "current"),
                        bits_at(row, "duration"),
                    ],
                    recorded_state(at(row, "stateBefore")),
                    recorded_state(at(row, "stateAfter")),
                    at(row, "rateCount"),
                    at(row, "total"),
                ),
            };
            assert!(clock.emit, "{context}: native EmitOverTime ran");
            exact(
                clock.emission_previous,
                arguments[0],
                &format!("{context}: EmitOverTime previous"),
            );
            exact(
                clock.current,
                arguments[1],
                &format!("{context}: EmitOverTime current"),
            );
            exact(
                law.duration,
                arguments[2],
                &format!("{context}: EmitOverTime duration"),
            );
            assert_eq!(
                state_bits(&state),
                state_bits(&before),
                "{context}: state entering the call"
            );
            let batch = law
                .schedule(
                    clock,
                    &mut state,
                    crate::particle::initial::initial_reciprocal,
                )
                .unwrap_or_else(|e| panic!("{context}: refused {e:?}"))
                .unwrap_or_else(|| panic!("{context}: no batch"));
            assert_eq!(
                Some(u64::from(batch.rate_count)),
                native_count(rate_count),
                "{context}: rate count"
            );
            assert_eq!(
                Some(u64::from(batch.total)),
                native_count(total),
                "{context}: total"
            );
            assert_eq!(
                state_bits(&state),
                state_bits(&after),
                "{context}: state after the call"
            );
        }
        Ok(rows.len())
    }

    /// Every native EmitOverTime call of the two-constant receipt: chains over
    /// every corpus configuration with a two-constant rate or burst count,
    /// randomized envelopes, gate controls, real Update1 slices and the input
    /// controls. Rate count, total, spacing, carry, burst fraction and the
    /// scalar stream words must equal the native bits on every admitted call.
    #[test]
    #[ignore = "MOLY_EMISSION_TWO_CONSTANTS_RECEIPT must name the private current-native two-constant emission receipt"]
    fn replays_current_native_two_constant_emission() {
        let path = std::env::var_os("MOLY_EMISSION_TWO_CONSTANTS_RECEIPT")
            .expect("supply the external current-native two-constant emission receipt");
        let receipt = json::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            at(at(&receipt, "library"), "sha256").as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let mut direct_clocks = 0;
        let mut calls = std::collections::BTreeMap::<&str, usize>::new();
        let mut admitted = 0;
        let mut refused = Vec::new();
        let mut replay = |group: &'static str, label: &str, config: &Value, rows: &[Value], kind| {
            match replay_native_chain(label, config, rows, kind, &mut direct_clocks) {
                Ok(n) => {
                    admitted += 1;
                    *calls.entry(group).or_default() += n;
                }
                Err(refusal) => {
                    refused.push((label.split('/').next().unwrap().to_owned(), refusal))
                }
            }
        };
        let text = |value: &Value, key: &str| at(value, key).as_str().unwrap().to_owned();
        for group in array(at(&receipt, "direct")) {
            let tag = match at(group, "tag").as_str().unwrap() {
                "E2" => "E2",
                "EB" => "EB",
                "outsideItem" => "outside",
                other => panic!("receipt tag {other}"),
            };
            for chain in array(at(group, "chains")) {
                let rows = array(at(chain, "calls"));
                assert_eq!(
                    state_bits(&recorded_state(at(chain, "initialState"))),
                    state_bits(&recorded_state(at(&rows[0], "stateBefore")))
                );
                replay(tag, &text(chain, "label"), at(chain, "config"), rows, NativeRows::Direct);
            }
        }
        for (key, group) in [("extras", "extras"), ("gateControls", "gate")] {
            for chain in array(at(&receipt, key)) {
                let rows = array(at(chain, "calls"));
                assert_eq!(
                    state_bits(&recorded_state(at(chain, "initialState"))),
                    state_bits(&recorded_state(at(&rows[0], "stateBefore")))
                );
                replay(group, &text(chain, "label"), at(chain, "config"), rows, NativeRows::Direct);
            }
        }
        for chain in array(at(&receipt, "update1")) {
            let rows = array(at(chain, "rows"));
            assert_eq!(word(at(chain, "startClock")), word(at(&rows[0], "clockBefore")));
            replay("update1", &text(chain, "label"), at(chain, "config"), rows, NativeRows::Update1);
        }
        for row in array(at(at(&receipt, "controls"), "rows")) {
            replay(
                "control",
                &text(row, "name"),
                at(row, "config"),
                std::slice::from_ref(at(row, "call")),
                NativeRows::Control,
            );
        }
        // Refused, each for a reason outside this law: a repeating burst
        // (cycles other than 1) together with three bursts; three bursts; a
        // burst count whose smaller constant truncates to a negative count.
        assert!(refused
            .iter()
            .all(|(_, refusal)| *refusal == Refused::UnsupportedConfiguration));
        let mut groups: Vec<&str> = refused.iter().map(|(group, _)| group.as_str()).collect();
        groups.dedup();
        assert_eq!(
            groups,
            [
                "outside05", "outside08", "outside09", "outside10", "outside11", "outside12",
                "outside13", "gate2"
            ]
        );
        assert_eq!((admitted, refused.len(), direct_clocks), (236, 64, 18));
        assert_eq!(
            calls,
            std::collections::BTreeMap::from([
                ("E2", 398),
                ("EB", 398),
                ("control", 6),
                ("extras", 1600),
                ("gate", 18),
                ("outside", 1990),
                ("update1", 92),
            ])
        );
        eprintln!(
            "native two-constant emission: {admitted} chains, {} calls exact (E2 398, EB 398), {direct_clocks} calls from recorded arguments, {} chains refused",
            calls.values().sum::<usize>(),
            refused.len()
        );
    }

    /// What a native battery replay found: calls equal to native, calls refused
    /// where native's own count leaves the representable range, calls outside
    /// the gate (unsupported, count range), and admitted calls whose rate
    /// Evaluate drew exactly 1.0.
    #[derive(Debug, PartialEq)]
    struct BatteryTally {
        exact: usize,
        out_of_range: usize,
        gate_unsupported: usize,
        gate_count: usize,
        unit_draws: usize,
    }

    /// Replays a native battery (families of direct EmitOverTime calls), each
    /// call on its own recorded state. Inside the gate every call equals the
    /// native bits, or is refused only where native's own rate count or total
    /// exceeds the representable count range, with the state left untouched;
    /// nothing panics.
    fn replay_native_battery(variable: &str) -> BatteryTally {
        let path = std::env::var_os(variable)
            .unwrap_or_else(|| panic!("supply the external native battery in {variable}"));
        let battery = json::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            at(at(&battery, "summary"), "librarySha256").as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let mut tally = BatteryTally {
            exact: 0,
            out_of_range: 0,
            gate_unsupported: 0,
            gate_count: 0,
            unit_draws: 0,
        };
        for family in array(at(&battery, "families")) {
            let label = at(family, "family").as_str().unwrap();
            let rows = array(at(family, "rows"));
            let law = match native_law(at(family, "config")) {
                Ok(law) => law,
                Err(Refused::UnsupportedConfiguration) => {
                    tally.gate_unsupported += rows.len();
                    continue;
                }
                Err(Refused::CountOutOfRange) => {
                    tally.gate_count += rows.len();
                    continue;
                }
                Err(other) => panic!("{label}: gate {other:?}"),
            };
            for (index, row) in rows.iter().enumerate() {
                let context = format!("{label} call {index}");
                let call = at(row, "call");
                exact(law.duration, bits_at(call, "duration"), &context);
                let before = recorded_state(at(call, "stateBefore"));
                let mut state = before;
                let clock = direct_clock(&law, bits_at(call, "previous"), bits_at(call, "current"));
                let beyond = beyond_count_range(at(call, "rateCount"))
                    || beyond_count_range(at(call, "total"));
                match law.schedule(
                    clock,
                    &mut state,
                    crate::particle::initial::initial_reciprocal,
                ) {
                    Ok(Some(batch)) => {
                        assert!(!beyond, "{context}: admitted a count beyond the range");
                        assert_eq!(
                            Some(u64::from(batch.rate_count)),
                            native_count(at(call, "rateCount")),
                            "{context}: rate count"
                        );
                        assert_eq!(
                            Some(u64::from(batch.total)),
                            native_count(at(call, "total")),
                            "{context}: total"
                        );
                        assert_eq!(
                            state_bits(&state),
                            state_bits(&recorded_state(at(call, "stateAfter"))),
                            "{context}: state after the call"
                        );
                        tally.exact += 1;
                        let unit = array(at(call, "trace")).iter().any(|t| {
                            at(t, "fn").as_str() == Some("Evaluate")
                                && at(t, "curve").as_str() == Some("rate")
                                && word(&array(at(t, "random"))[0]) == 0x3f80_0000
                        });
                        tally.unit_draws += usize::from(unit);
                    }
                    Ok(None) => panic!("{context}: no batch"),
                    Err(refusal) => {
                        assert!(
                            beyond,
                            "{context}: refused {refusal:?} inside the native count range"
                        );
                        assert!(
                            matches!(refusal, Refused::InvalidInput | Refused::CountOutOfRange),
                            "{context}: {refusal:?}"
                        );
                        assert_eq!(
                            state_bits(&state),
                            state_bits(&before),
                            "{context}: a refusal leaves the state alone"
                        );
                        tally.out_of_range += 1;
                    }
                }
            }
        }
        eprintln!("native battery {variable}: {tally:?}");
        tally
    }

    /// The verifier's independent native battery over edge inputs: swapped,
    /// equal, fractional, subnormal, huge and negative constants, probability
    /// and cycle variants, burst-time edges and fuzzed configurations.
    #[test]
    #[ignore = "MOLY_EMISSION_TWO_CONSTANTS_VERIFY_ROWS must name the private independent native two-constant emission battery"]
    fn replays_independent_native_two_constant_emission_battery() {
        assert_eq!(
            replay_native_battery("MOLY_EMISSION_TWO_CONSTANTS_VERIFY_ROWS"),
            BatteryTally {
                exact: 376,
                out_of_range: 6,
                gate_unsupported: 964,
                gate_count: 8,
                unit_draws: 0,
            }
        );
    }

    /// Native calls whose entry draw was chosen to give r == 1.0, r == 0 and r
    /// just below 1 on two-constant rates (no other row reaches r == 1.0), and
    /// burst counts in (-1, 0) with the -1 boundary, which stays refused.
    #[test]
    #[ignore = "MOLY_EMISSION_TWO_CONSTANTS_EDGE_ROWS must name the private native two-constant emission edge probe"]
    fn replays_native_two_constant_emission_edge_probe() {
        assert_eq!(
            replay_native_battery("MOLY_EMISSION_TWO_CONSTANTS_EDGE_ROWS"),
            BatteryTally {
                exact: 408,
                out_of_range: 0,
                gate_unsupported: 48,
                gate_count: 0,
                unit_draws: 72,
            }
        );
    }
}
