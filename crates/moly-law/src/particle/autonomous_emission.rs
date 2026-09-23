//! Guarded ordinary Update1 / EmitOverTime / StartParticles scheduling.
//!
//! Current JP libunity 937c6d28...75badd9. Evidence: autonomous-birth-native
//! boundaryReplay (405 scalar emission, 108 clock, 15 newborn cases, zero failures).
//! Scope: initialized clock, flags=4, no delay or distance emission, one slice
//! with frame_dt == accumulated_dt, constant rate, <=2 deterministic cycle-1
//! bursts. This prepares phases; it never admits a system or simulates particles.
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ConstantBurst {
    time: f32,
    count: u32,
}

#[derive(Clone, Debug)]
pub struct ConstantAutonomousEmission {
    duration: f32,
    looping: bool,
    rate: f32,
    bursts: Vec<ConstantBurst>,
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
        let rate = constant(&emission.rate_over_time)?;
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
            let count = constant(&b.count)?;
            if count > MAX_COUNT {
                return Err(Refused::CountOutOfRange);
            }
            // Preserve authoring order: the last hit owns the one burst fraction.
            bursts.push(ConstantBurst {
                time: b.time,
                count: count as u32,
            });
        }
        Ok(Self {
            duration,
            looping,
            rate,
            bursts,
        })
    }

    /// This is one flags=4 normal slice with no backlog and no start delay.
    /// A caller with frame_dt < accumulated_dt must model each native slice and
    /// its backtrack argument separately; passing accumulated_dt here is wrong.
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
    /// capacity. Even a zero-rate zero-count native call advances scalar RNG.
    /// Refusals leave state untouched. Supply the native reciprocal duration
    /// instruction result; scalar 1/duration is not bit-exact for all durations.
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
        let amount = if current < previous {
            // Native two separate rounded products followed by a rounded sum.
            current * self.rate + (self.duration - previous) * self.rate
        } else {
            (current - previous) * self.rate
        };
        if !amount.is_finite() || amount < 0.0 {
            return Err(Refused::InvalidInput);
        }
        let mut burst_fraction = old.burst_fraction;
        let mut burst_count = 0_u32;
        let mut accumulate = |low: f32, high: f32| -> Result<(), Refused> {
            for b in &self.bursts {
                if low <= b.time && b.time < high {
                    burst_count = burst_count
                        .checked_add(b.count)
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
            accumulate(0.0, current)?;
            accumulate(previous, self.duration + MIN_AMOUNT)?;
        } else {
            accumulate(previous, current)?;
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
        next.random.next_u32();
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

fn constant(curve: &MinMaxCurve) -> Result<f32, Refused> {
    match curve {
        MinMaxCurve::Constant(v) if v.is_finite() && *v >= 0.0 => Ok(*v),
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
            rate: 4.0,
            bursts: vec![ConstantBurst {
                time: 0.0,
                count: 1,
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
                rate: field(row, "rate"),
                bursts: array(at(row, "bursts"))
                    .iter()
                    .map(|b| {
                        let b = array(b);
                        ConstantBurst {
                            time: number(&b[0]),
                            count: number(&b[1]) as u32,
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
}
