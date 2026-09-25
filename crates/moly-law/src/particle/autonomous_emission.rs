//! Guarded ordinary Update1 / EmitOverTime / StartParticles scheduling.
//!
//! Current JP 6.8.1 libunity. Evidence: a native boundary replay
//! (405 scalar emission, 108 clock, 15 newborn cases), a native EmitOverTime
//! receipt over every two-constant emission configuration of the corpus,
//! native EmitOverTime batteries over the burst schedule, and native rows of
//! the emission load normalization, all with zero failures.
//! Scope: initialized clock, no delay, one incremental slice per call (the
//! per-frame head supplies the slices; their births-ahead argument is applied
//! by the birth placement, not here), a constant or two-constant rate, up to
//! eight bursts with a constant or two-constant count and any probability,
//! cycle count, repeat interval and time. Emission over distance is a separate
//! law (`ConstantDistanceEmission`) that the per-frame head runs once before
//! the slices; `from_params` refuses a distance rate so no other caller drops it.
//! This prepares phases; it never admits a system or simulates particles.
//! Never use an old emission-head interval for birth curves after a loop wrap:
//! StartParticles uses (current - slice_dt, current), both times native reciprocal
//! duration. Prepare clock -> existing pre/sim/death/post -> schedule -> births.

use crate::particle::emit::{Burst, BurstCycles};
use crate::particle::gradient::{arm_fmax, arm_fmin};
use crate::particle::schema::EmissionParams;
use crate::particle::seed_owner::ScalarRandom;
use crate::particle::sub_emission::{BirthDistribution, BirthTiming};
use crate::particle::value::MinMaxCurve;

const EPSILON: f32 = f32::from_bits(0x3586_37bd);
const MIN_AMOUNT: f32 = f32::from_bits(0x38d1_b717);
const MAX_COUNT: f32 = 16_777_215.0;
const MAX_WRAPS: u32 = 4096;
/// EmissionModule's load clamps every rate scalar to [0, 1e7].
const MAX_RATE: f32 = 10_000_000.0;
/// EmissionModule holds its bursts in eight inline slots and clamps its
/// serialized burst count to [0, 8].
const BURST_SLOTS: usize = 8;

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
/// for, as signed 32-bit counts: each constant goes through the saturating
/// float-to-int conversion toward zero (NaN to 0), which `as i32` is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BurstCount {
    /// Constant mode: the max scalar converted; no draw.
    Constant(i32),
    /// TwoConstants: lo and hi are the smaller and the larger constant, each
    /// converted, so lo <= hi (a monotone conversion of an ordered pair; a
    /// NaN constant makes both the other one). Every evaluation takes one
    /// full 32-bit draw, even when lo == hi, and the count is
    /// lo + word % (hi + 1 - lo) in wrapping 32-bit arithmetic; where that
    /// range wraps to zero the unsigned division yields zero, so the
    /// remainder is the whole word.
    TwoConstants { lo: i32, hi: i32 },
}

impl BurstCount {
    fn sample(self, random: &mut ScalarRandom) -> i32 {
        match self {
            Self::Constant(count) => count,
            Self::TwoConstants { lo, hi } => {
                let range = (hi.wrapping_add(1) as u32).wrapping_sub(lo as u32);
                let word = random.next_u32();
                let remainder = if range == 0 { word } else { word % range };
                remainder.wrapping_add(lo as u32) as i32
            }
        }
    }
}

/// One burst as the runtime EmissionModule holds it, after the load
/// normalization: its time, count, serialized cycle count word (1 fires once
/// per cycle, 0 repeats without end), repeat interval and probability.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScheduledBurst {
    time: f32,
    count: BurstCount,
    cycles: u32,
    interval: f32,
    probability: f32,
}

impl ScheduledBurst {
    /// EmissionModule::AccumulateBurst. A zero probability counts 0 without a
    /// draw; one at or above 1, or NaN, takes no probability draw; any other
    /// takes one draw r (its low 23 bits times UNIT_SCALE) and counts 0 where
    /// the probability is at most r, without evaluating the count. Otherwise
    /// the count is evaluated.
    fn accumulate(&self, random: &mut ScalarRandom) -> i32 {
        let probability = self.probability;
        if probability == 0.0 {
            return 0;
        }
        if !(probability >= 1.0 || probability.is_nan()) {
            let r = (random.next_u32() & 0x7f_ffff) as f32 * UNIT_SCALE;
            if probability <= r {
                return 0;
            }
        }
        self.count.sample(random)
    }

    /// AccumulateBursts' test for this burst in the window [low, high). A
    /// time inside the window hits. Outside it, only a window that allows
    /// repeats, a time before the window and a cycle count other than one
    /// reach the repeat test: k = (low - time) / interval; a finite cycle
    /// count needs k below cycles - 1 (that word as a signed 32-bit value,
    /// converted to float); then it hits where (high - time) / interval
    /// converted toward zero exceeds k converted likewise. A hit accumulates
    /// the count once, however many repeats the window spans.
    fn hits(&self, low: f32, high: f32, repeats: bool) -> bool {
        let time = self.time;
        let inside = time >= low && time < high;
        if inside || !repeats {
            return inside;
        }
        if !(time < low) || self.cycles == 1 {
            return false;
        }
        let k = (low - time) / self.interval;
        if self.cycles != 0 && !(k < self.cycles.wrapping_sub(1) as i32 as f32) {
            return false;
        }
        ((high - time) / self.interval) as i32 > k as i32
    }
}

/// AccumulateBursts over one window, bursts in authoring order: a hit takes
/// its count before the fraction is written, and the fraction is written on
/// every hit, zero counts too. The window's sum wraps in signed 32-bit
/// arithmetic.
fn accumulate_window(bursts: &[ScheduledBurst], low: f32, high: f32, repeats: bool,
    random: &mut ScalarRandom, burst_fraction: &mut f32) -> i32 {
    let mut sum = 0_i32;
    for b in bursts {
        if b.hits(low, high, repeats) {
            sum = sum.wrapping_add(b.accumulate(random));
            let relative = (b.time - low) / (high - low);
            *burst_fraction = if relative < 0.0 {
                1.0
            } else {
                1.0 - arm_fmin(relative, 1.0)
            };
        }
    }
    sum
}

/// The bursts of an exported emission block as EmitOverTime reads them: the
/// load normalization, then the runtime gate (at most eight slots, a
/// constant or two-constant count), every time, cycle count, repeat interval
/// and probability transcribed. For a caller that runs its own windows: the
/// sub-emitter birth event's call of EmitOverTime.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BurstSchedule {
    bursts: Vec<ScheduledBurst>,
}

impl BurstSchedule {
    pub(crate) fn from_params(bursts: &[Burst]) -> Result<Self, Refused> {
        if bursts.len() > BURST_SLOTS {
            return Err(Refused::UnsupportedConfiguration);
        }
        let bursts = bursts
            .iter()
            .map(|b| {
                let b = load_burst(b);
                Ok(ScheduledBurst {
                    time: b.time,
                    count: burst_count(&b.count)?,
                    cycles: b.cycles,
                    interval: b.interval,
                    probability: b.probability,
                })
            })
            .collect::<Result<Vec<_>, Refused>>()?;
        Ok(Self { bursts })
    }

    /// One AccumulateBursts window [low, high): the wrapping sum of the hit
    /// counts, the fraction written on each hit, the draws taken from
    /// `random` in authoring order.
    pub(crate) fn accumulate(&self, low: f32, high: f32, repeats: bool, random: &mut ScalarRandom,
        burst_fraction: &mut f32) -> i32 {
        accumulate_window(&self.bursts, low, high, repeats, random, burst_fraction)
    }
}

/// A burst's fields as the runtime EmissionModule holds them, the count still
/// a curve: the load normalization's output and the runtime gate's input.
#[derive(Clone, Debug, PartialEq)]
struct LoadedBurst {
    time: f32,
    count: MinMaxCurve,
    cycles: u32,
    interval: f32,
    probability: f32,
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
    /// The per-slice law of a system without emission over distance; a
    /// distance rate other than zero is refused here.
    pub fn from_params(
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<Self, Refused> {
        if !matches!(&emission.rate_over_distance, MinMaxCurve::Constant(v) if *v == 0.0) {
            return Err(Refused::UnsupportedConfiguration);
        }
        Self::from_time_params(delay, duration, looping, emission)
    }

    /// The per-slice law together with the distance law the per-frame head
    /// runs before the slices. Both share the accumulator, spacing and scalar
    /// stream in `AutonomousEmissionState`.
    pub fn from_params_with_distance(
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<(Self, Option<ConstantDistanceEmission>), Refused> {
        let law = Self::from_time_params(delay, duration, looping, emission)?;
        let distance = ConstantDistanceEmission::from_params(&law, &emission.rate_over_distance)?;
        Ok((law, distance))
    }

    /// The law of an exported emission block: the serialized values go
    /// through the load normalization (`load_rate`, `load_burst`) and then
    /// the runtime gate. More bursts than the engine's eight slots are
    /// refused: the loader clamps its serialized burst count to them, and
    /// the export carries the burst list, not that count, so the law takes
    /// the list's length as the count.
    fn from_time_params(
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<Self, Refused> {
        if !matches!(delay, MinMaxCurve::Constant(v) if *v == 0.0)
            || emission.bursts.len() > BURST_SLOTS
        {
            return Err(Refused::UnsupportedConfiguration);
        }
        let bursts: Vec<LoadedBurst> = emission.bursts.iter().map(load_burst).collect();
        Self::from_runtime(duration, looping, &load_rate(&emission.rate_over_time), &bursts)
    }

    /// The law from the fields as the runtime EmissionModule holds them. The
    /// gate refuses a duration that is not finite and positive, a rate that
    /// is not a finite constant or pair of constants, a count in a curve
    /// mode, and more than eight bursts; every probability, cycle count,
    /// repeat interval, time and count constant is transcribed.
    fn from_runtime(
        duration: f32,
        looping: bool,
        rate_over_time: &MinMaxCurve,
        bursts: &[LoadedBurst],
    ) -> Result<Self, Refused> {
        if !duration.is_finite() || duration <= 0.0 || bursts.len() > BURST_SLOTS {
            return Err(Refused::UnsupportedConfiguration);
        }
        let rate = rate(rate_over_time)?;
        // Preserve authoring order: hits in one window draw in this order,
        // and the last hit owns the one burst fraction.
        let bursts = bursts
            .iter()
            .map(|b| {
                Ok(ScheduledBurst {
                    time: b.time,
                    count: burst_count(&b.count)?,
                    cycles: b.cycles,
                    interval: b.interval,
                    probability: b.probability,
                })
            })
            .collect::<Result<Vec<_>, Refused>>()?;
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
    /// EmitOverTime draws once at entry, and each burst hit draws once more
    /// for a probability draw and once for a two-constant count it evaluates.
    /// Refusals leave state untouched. Supply the native reciprocal duration
    /// instruction result; scalar 1/duration is not bit-exact for all
    /// durations.
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
        let mut accumulate = |low: f32, high: f32, repeats: bool, random: &mut ScalarRandom| -> i32 {
            accumulate_window(&self.bursts, low, high, repeats, random, &mut burst_fraction)
        };
        // A wrap draws for the window [0, current), which allows repeats,
        // before the window [previous, duration + 1e-4), which does not; no
        // wrap takes the one window [previous, current), which allows them.
        let (first, second) = if current < previous {
            let first = accumulate(0.0, current, true, &mut random);
            (first, accumulate(previous, self.duration + MIN_AMOUNT, false, &mut random))
        } else {
            (0, accumulate(previous, current, true, &mut random))
        };
        let full = amount + old.offset;
        if !full.is_finite() || full > MAX_COUNT {
            return Err(Refused::CountOutOfRange);
        }
        let rate_count = full as u32;
        // Native adds the rate count and both window sums, each sign-extended,
        // in 64 bits. A total below zero wraps there to a count far beyond the
        // representable one; both are refused, as is any total above it.
        let total = i64::from(rate_count) + i64::from(first) + i64::from(second);
        if !(0..=MAX_COUNT as i64).contains(&total) {
            return Err(Refused::CountOutOfRange);
        }
        let total = total as u32;
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

/// EmitOverDistance, which the per-frame update runs once at its head with the
/// frame's whole scaled dt, after the pending sum and before the first slice
/// (never per slice). It shares the accumulator, spacing and scalar stream
/// with EmitOverTime, which later draws once per slice and rewrites the
/// spacing.
///
/// Qualified subset, as executed: a finite constant rate above zero, a
/// looping system with no start delay and no bursts, and a constant rate over
/// time. A rate of exactly zero is no law at all (the caller's rate test fails
/// before any draw); a negative, curve or two-constant rate is refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstantDistanceEmission {
    rate: f32,
    duration: f32,
}

/// One EmitOverDistance call and the StartParticles command it issues: the
/// command time is the frame's end clock t1 = fmod(pending + clock, duration),
/// its dt is the scaled frame dt, every birth is a rate birth, it has no
/// births-ahead argument, and its pending argument is the pending time after
/// this frame's sum, so the elapsed time of a lane is dt * fraction - pending
/// (negative in general): the frame's slices then advance the newborns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceBirthBatch {
    pub count: u32,
    pub distribution: BirthDistribution,
    pub dt: f32,
    pub pending: f32,
    /// The command time t1.
    pub time: f32,
    /// t1 - dt and t1, both times the native reciprocal duration.
    pub previous_normalized: f32,
    pub current_normalized: f32,
}

impl ConstantDistanceEmission {
    fn from_params(law: &ConstantAutonomousEmission, rate: &MinMaxCurve) -> Result<Option<Self>, Refused> {
        let rate = match *rate {
            MinMaxCurve::Constant(value) if value == 0.0 => return Ok(None),
            MinMaxCurve::Constant(value) if value.is_finite() && value > 0.0 => value,
            _ => return Err(Refused::UnsupportedConfiguration),
        };
        if !law.looping || !law.bursts.is_empty() || !matches!(law.rate, EmissionRate::Constant(_)) {
            return Err(Refused::UnsupportedConfiguration);
        }
        Ok(Some(Self { rate, duration: law.duration }))
    }

    /// `velocity` is the emitter velocity the frame head holds (translation
    /// change over the raw frame dt, recomputed only above 1e-4 s), `scaled_dt`
    /// the frame dt times the clamped simulation speed, `pending` the pending
    /// time after this frame's sum, `clock` the system clock before the
    /// slices. One draw before the amount (a constant rate ignores its value),
    /// also at zero velocity. State is committed only on success.
    pub fn emit(
        &self,
        state: &mut AutonomousEmissionState,
        velocity: [f32; 3],
        scaled_dt: f32,
        pending: f32,
        clock: f32,
        reciprocal: impl FnOnce(f32) -> Option<f32>,
    ) -> Result<DistanceBirthBatch, Refused> {
        let old = state.distribution;
        if ![old.spacing, old.offset, old.burst_fraction, scaled_dt, pending, clock]
            .iter()
            .all(|v| v.is_finite())
            || old.spacing < 0.0
            || !(0.0..1.0).contains(&old.offset)
            || scaled_dt < 0.0
            || pending < 0.0
            || !(0.0..self.duration).contains(&clock)
            || velocity.iter().any(|v| !v.is_finite())
        {
            return Err(Refused::InvalidInput);
        }
        let mut random = state.random;
        let _ = random.next_u32();
        // The squared sum keeps its native grouping ((x*x + y*y) + z*z);
        // sqrt is the correctly rounded single-precision root.
        let length = ((velocity[0] * velocity[0] + velocity[1] * velocity[1]) + velocity[2] * velocity[2]).sqrt();
        let amount = (self.rate * scaled_dt) * length;
        let full = old.offset + amount;
        if !full.is_finite() || full > MAX_COUNT {
            return Err(Refused::CountOutOfRange);
        }
        // Truncation toward zero, as the native unsigned convert.
        let count = full as u32;
        let distribution = BirthDistribution {
            spacing: if amount < MIN_AMOUNT { 1.0 } else { 1.0 / amount },
            offset: full - count as f32,
            burst_fraction: old.burst_fraction,
        };
        // Looping only (the qualified subset): Rust `%` on f32 is fmodf.
        let current = (pending + clock) % self.duration;
        let inverse = reciprocal(self.duration)
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or(Refused::ReciprocalUnavailable)?;
        let previous_normalized = (current - scaled_dt) * inverse;
        let current_normalized = current * inverse;
        if !previous_normalized.is_finite() || !current_normalized.is_finite() {
            return Err(Refused::InvalidInput);
        }
        state.random = random;
        state.distribution = distribution;
        Ok(DistanceBirthBatch {
            count,
            distribution,
            dt: scaled_dt,
            pending,
            time: current,
            previous_normalized,
            current_normalized,
        })
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

/// The burst count as AccumulateBurst reads it, for every constant: the
/// saturating conversion toward zero (NaN to 0). TwoConstants orders the pair
/// with the same strict comparisons native uses, so a swapped pair is the
/// ordered one. The curve modes are not transcribed.
fn burst_count(curve: &MinMaxCurve) -> Result<BurstCount, Refused> {
    match *curve {
        MinMaxCurve::Constant(value) => Ok(BurstCount::Constant(value as i32)),
        MinMaxCurve::TwoConstants { min, max } => {
            let low = if max < min { max } else { min };
            let high = if min < max { max } else { min };
            Ok(BurstCount::TwoConstants {
                lo: low as i32,
                hi: high as i32,
            })
        }
        _ => Err(Refused::UnsupportedConfiguration),
    }
}

/// EmissionModule's load normalization of a rate curve, applied on every
/// load path (the binary and the type-converting reader, and the clone
/// that serializes through the same transfer) before emission reads it:
/// each scalar below zero becomes +0 and any other is at most 1e7 (a NaN
/// stays NaN). The curve modes' keys are not touched and stay refused.
fn load_rate(curve: &MinMaxCurve) -> MinMaxCurve {
    match *curve {
        MinMaxCurve::Constant(value) => MinMaxCurve::Constant(load_rate_scalar(value)),
        MinMaxCurve::TwoConstants { min, max } => MinMaxCurve::TwoConstants {
            min: load_rate_scalar(min),
            max: load_rate_scalar(max),
        },
        ref other => other.clone(),
    }
}

fn load_rate_scalar(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else {
        arm_fmin(value, MAX_RATE)
    }
}

/// ParticleSystemEmissionBurst's load normalization, on the same load paths
/// as `load_rate`: the time is at least +0, both count scalars are at least
/// +0 (so no loaded count is negative), a cycle count word that is negative
/// as a signed 32-bit value becomes 0, the repeat interval is at least 1e-4,
/// and a probability below zero becomes +0 and any other is at most 1 (a
/// NaN field stays NaN). The time is not clamped to the duration: that
/// check is the editor's consistency pass, which the player's load does not
/// run.
fn load_burst(burst: &Burst) -> LoadedBurst {
    let count = match burst.count {
        MinMaxCurve::Constant(value) => MinMaxCurve::Constant(arm_fmax(value, 0.0)),
        MinMaxCurve::TwoConstants { min, max } => MinMaxCurve::TwoConstants {
            min: arm_fmax(min, 0.0),
            max: arm_fmax(max, 0.0),
        },
        ref other => other.clone(),
    };
    let cycles = match burst.cycles {
        BurstCycles::Infinite => 0,
        BurstCycles::Finite(n) => n.get(),
    };
    LoadedBurst {
        time: arm_fmax(burst.time, 0.0),
        count,
        cycles: if (cycles as i32) < 0 { 0 } else { cycles },
        interval: arm_fmax(burst.repeat_interval, MIN_AMOUNT),
        probability: if burst.probability < 0.0 {
            0.0
        } else {
            arm_fmin(burst.probability, 1.0)
        },
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
                cycles: 1,
                interval: 0.01,
                probability: 1.0,
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
                            count: BurstCount::Constant(number(&b[1]) as i32),
                            cycles: 1,
                            interval: 0.01,
                            probability: 1.0,
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
    /// The law of a native configuration through the runtime gate. The
    /// harness wrote the configuration straight into the runtime
    /// EmissionModule, past the load normalization, so the row's values are
    /// runtime values, some of which no load produces (a negative count, a
    /// probability above 1); the load half has its own native replay.
    fn native_law(config: &Value) -> Result<ConstantAutonomousEmission, Refused> {
        let bursts: Vec<LoadedBurst> = array(at(config, "bursts"))
            .iter()
            .map(|b| LoadedBurst {
                time: bits_at(b, "timeBits"),
                count: native_curve(at(b, "count")),
                cycles: word(at(b, "cycles")),
                interval: bits_at(b, "intervalBits"),
                probability: bits_at(b, "probabilityBits"),
            })
            .collect();
        let runtime = ConstantAutonomousEmission::from_runtime(
            bits_at(config, "durationBits"),
            boolean(config, "looping"),
            &native_curve(at(config, "rate")),
            &bursts,
        );
        product_entry_agrees(config, &runtime);
        runtime
    }

    /// The product entry over a native configuration the load normalization
    /// leaves unchanged (NaN by class). Serialized, such a configuration
    /// loads to exactly the runtime values the native call ran (the load half
    /// is replayed against the native transfer rows), so from_params, the
    /// load followed by from_runtime, must admit exactly where from_runtime
    /// admits and build the same law: a gate narrowed at the product entry
    /// alone turns every replay that reaches such a configuration red. A
    /// configuration the load changes is left to the two halves' own
    /// replays.
    fn product_entry_agrees(config: &Value, runtime: &Result<ConstantAutonomousEmission, Refused>) {
        let same = |a: f32, b: f32| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
        let same_curve = |left: &MinMaxCurve, right: &MinMaxCurve| match (left, right) {
            (MinMaxCurve::Constant(x), MinMaxCurve::Constant(y)) => same(*x, *y),
            (
                MinMaxCurve::TwoConstants { min: p, max: q },
                MinMaxCurve::TwoConstants { min: s, max: t },
            ) => same(*p, *s) && same(*q, *t),
            _ => false,
        };
        let rate = native_curve(at(config, "rate"));
        let mut unchanged = same_curve(&load_rate(&rate), &rate);
        let bursts: Vec<Burst> = array(at(config, "bursts"))
            .iter()
            .map(|b| {
                let cycles = word(at(b, "cycles"));
                let serialized = Burst {
                    time: bits_at(b, "timeBits"),
                    count: native_curve(at(b, "count")),
                    cycles: BurstCycles::from_serialized(cycles),
                    repeat_interval: bits_at(b, "intervalBits"),
                    probability: bits_at(b, "probabilityBits"),
                };
                let loaded = load_burst(&serialized);
                unchanged &= same(loaded.time, serialized.time)
                    && same_curve(&loaded.count, &serialized.count)
                    && loaded.cycles == cycles
                    && same(loaded.interval, serialized.repeat_interval)
                    && same(loaded.probability, serialized.probability);
                serialized
            })
            .collect();
        if !unchanged {
            return;
        }
        let count = bursts.len();
        let product = ConstantAutonomousEmission::from_params(
            &MinMaxCurve::Constant(0.0),
            bits_at(config, "durationBits"),
            boolean(config, "looping"),
            &EmissionParams {
                rate_over_time: rate,
                rate_over_distance: MinMaxCurve::Constant(0.0),
                bursts,
            },
        );
        match (runtime, &product) {
            (Ok(runtime), Ok(product)) => assert_eq!(
                format!("{product:?}"),
                format!("{runtime:?}"),
                "the product entry builds another law from a load-invariant configuration of {count} bursts"
            ),
            (Err(_), Err(_)) => {}
            (runtime, product) => panic!(
                "the product entry gives {:?} where the runtime law gives {:?} on a load-invariant configuration of {count} bursts",
                product.as_ref().err(),
                runtime.as_ref().err()
            ),
        }
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

    /// Whether a native configuration authors something the port does not
    /// transcribe, read from the configuration record the native call ran,
    /// not through the gate: a duration that is not finite and positive, a
    /// non-finite rate constant, or more bursts than the eight slots. A gate
    /// refusal must fall on such a configuration, so a narrowed gate turns a
    /// replay red; an admitted configuration, flagged or not, is compared
    /// with the native calls. A port that transcribes one of these drops it
    /// here.
    fn untranscribed(config: &Value) -> bool {
        let constants = |curve: &Value| match native_curve(curve) {
            MinMaxCurve::Constant(value) => vec![value],
            MinMaxCurve::TwoConstants { min, max } => vec![min, max],
            _ => unreachable!("native_curve reads constant modes only"),
        };
        let duration = bits_at(config, "durationBits");
        !(duration.is_finite() && duration > 0.0)
            || constants(at(config, "rate")).iter().any(|v| !v.is_finite())
            || array(at(config, "bursts")).len() > BURST_SLOTS
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
                    // The harness drove some calls from a negative slice start;
                    // where slice preparation refuses one, replay the call from
                    // its recorded arguments instead. Either way the slice is
                    // compared with the recorded EmitOverTime arguments below.
                    let clock = match law.prepare_slice(previous, dt, false) {
                        Ok(clock) => clock,
                        Err(refused) => {
                            assert!(previous < 0.0, "{context}: slice refused {refused:?}");
                            *direct_clocks += 1;
                            direct_clock(&law, bits_at(row, "previous"), bits_at(row, "current"))
                        }
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
    /// A refused chain must author something the port does not transcribe;
    /// which chains are refused, and how many calls replay, is reported.
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
                    assert!(
                        untranscribed(config),
                        "{label}: gate refused {refusal:?} a configuration the port transcribes"
                    );
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
        assert!(admitted > 0);
        let mut groups: Vec<&str> = refused.iter().map(|(group, _)| group.as_str()).collect();
        groups.dedup();
        eprintln!(
            "native two-constant emission: {admitted} chains, {} calls exact {calls:?}, {direct_clocks} calls from recorded arguments, {} chains refused {groups:?}",
            calls.values().sum::<usize>(),
            refused.len()
        );
    }

    /// What a native battery replay found, reported rather than pinned: calls
    /// equal to native, calls refused where native's own count leaves the
    /// representable range, calls of configurations the gate refuses (each
    /// one authoring something the port does not transcribe), and admitted
    /// calls whose rate Evaluate drew exactly 1.0.
    #[derive(Debug)]
    struct BatteryTally {
        exact: usize,
        out_of_range: usize,
        gate_refused: usize,
        unit_draws: usize,
    }

    /// Replays a native battery (families of direct EmitOverTime calls), each
    /// call on its own recorded state. A family the gate refuses must author
    /// something the port does not transcribe. Inside the gate every call
    /// equals the native bits, or is refused only where native's own rate
    /// count or total exceeds the representable count range, with the state
    /// left untouched.
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
            gate_refused: 0,
            unit_draws: 0,
        };
        for family in array(at(&battery, "families")) {
            let label = at(family, "family").as_str().unwrap();
            let rows = array(at(family, "rows"));
            let law = match native_law(at(family, "config")) {
                Ok(law) => law,
                Err(refused) => {
                    assert!(
                        untranscribed(at(family, "config")),
                        "{label}: gate refused {refused:?} a configuration the port transcribes"
                    );
                    tally.gate_refused += rows.len();
                    continue;
                }
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
        let tally = replay_native_battery("MOLY_EMISSION_TWO_CONSTANTS_VERIFY_ROWS");
        assert!(tally.exact > 0);
    }

    /// Native calls whose entry draw was chosen to give r == 1.0, r == 0 and r
    /// just below 1 on two-constant rates (no other row reaches r == 1.0), and
    /// burst counts in (-1, 0) with the -1 boundary. The r == 1.0 draws must
    /// reach an exact call.
    #[test]
    #[ignore = "MOLY_EMISSION_TWO_CONSTANTS_EDGE_ROWS must name the private native two-constant emission edge probe"]
    fn replays_native_two_constant_emission_edge_probe() {
        let tally = replay_native_battery("MOLY_EMISSION_TWO_CONSTANTS_EDGE_ROWS");
        assert!(tally.exact > 0 && tally.unit_draws > 0);
    }

    /// Native calls over the burst schedule the other files leave out: repeat
    /// intervals of every class (infinite, NaN, negative, subnormal, the load
    /// minimum), cycle counts across the signed boundary, probabilities of
    /// every class, non-finite and signed-zero times and times inside the
    /// wrap window past the cycle, five to eight bursts, and counts whose
    /// signed 32-bit window sums wrap, alone and sign-extended per window.
    /// Every call equals native or is refused only where native's own count
    /// leaves the representable range.
    #[test]
    #[ignore = "MOLY_EMISSION_BURST_ROWS must name the private native burst schedule rows"]
    fn replays_native_burst_schedule_rows() {
        let tally = replay_native_battery("MOLY_EMISSION_BURST_ROWS");
        assert!(tally.exact > 0);
    }

    /// The load normalization against native rows of the burst transfer and
    /// of the module transfer up to its burst array (the curve reader itself
    /// not executed: the row gives the curve's mode and scalars as read).
    /// Every burst field and rate scalar the transfer stores equals the load
    /// of the serialized value, bit for bit or NaN where native stored NaN;
    /// the stored burst count is the serialized one clamped to the eight
    /// slots, which it reaches.
    #[test]
    #[ignore = "MOLY_EMISSION_LOAD_ROWS must name the private native emission load rows"]
    fn load_normalization_matches_native_transfer_rows() {
        let path = std::env::var_os("MOLY_EMISSION_LOAD_ROWS")
            .expect("supply the external native emission load rows");
        let doc = json::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            at(at(&doc, "library"), "sha256").as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let same = |actual: f32, native: u32, what: &str, index: usize| {
            assert!(
                actual.to_bits() == native || (actual.is_nan() && f32::from_bits(native).is_nan()),
                "row {index} {what}: load {:#010x} native {native:#010x}",
                actual.to_bits()
            );
        };
        let (mut bursts, mut modules, mut changed, mut most) = (0, 0, 0, 0);
        for (index, row) in array(at(&doc, "rows")).iter().enumerate() {
            let input = at(row, "in");
            let stored = at(row, "out");
            match at(row, "kind").as_str() {
                Some("burst") => {
                    let count = match word(at(input, "mode")) {
                        0 => MinMaxCurve::Constant(bits_at(input, "max")),
                        3 => MinMaxCurve::TwoConstants {
                            min: bits_at(input, "min"),
                            max: bits_at(input, "max"),
                        },
                        other => panic!("row {index}: count mode {other}"),
                    };
                    let serialized = Burst {
                        time: bits_at(input, "time"),
                        count,
                        cycles: BurstCycles::from_serialized(word(at(input, "cycles"))),
                        repeat_interval: bits_at(input, "interval"),
                        probability: bits_at(input, "probability"),
                    };
                    let loaded = load_burst(&serialized);
                    same(loaded.time, word(at(stored, "time")), "time", index);
                    match loaded.count {
                        MinMaxCurve::Constant(max) => same(max, word(at(stored, "max")), "count", index),
                        MinMaxCurve::TwoConstants { min, max } => {
                            same(min, word(at(stored, "min")), "count min", index);
                            same(max, word(at(stored, "max")), "count max", index);
                        }
                        _ => unreachable!(),
                    }
                    assert_eq!(loaded.cycles, word(at(stored, "cycles")), "row {index} cycles");
                    same(loaded.interval, word(at(stored, "interval")), "interval", index);
                    same(loaded.probability, word(at(stored, "probability")), "probability", index);
                    changed += usize::from(
                        ["time", "min", "max", "cycles", "interval", "probability"]
                            .iter()
                            .any(|key| word(at(input, key)) != word(at(stored, key))),
                    );
                    bursts += 1;
                }
                Some("module") => {
                    for key in ["rate", "distance"] {
                        let (serialized, loaded) = (array(at(input, key)), array(at(stored, key)));
                        for slot in 1..3 {
                            same(
                                load_rate_scalar(f32::from_bits(word(&serialized[slot]))),
                                word(&loaded[slot]),
                                key,
                                index,
                            );
                        }
                    }
                    let count = word(at(input, "burstCount")) as i32;
                    let kept = word(at(stored, "burstCount")) as i32;
                    assert_eq!(kept, count.clamp(0, BURST_SLOTS as i32), "row {index} burst count");
                    most = most.max(kept);
                    modules += 1;
                }
                other => panic!("row {index}: kind {other:?}"),
            }
        }
        assert!(bursts > 0 && modules > 0 && changed > 0);
        assert_eq!(most, BURST_SLOTS as i32, "the stored burst count reaches the slot count");
        eprintln!("native emission load: {bursts} burst rows ({changed} normalized), {modules} module rows; exact");
    }
    fn bits(value: &Value) -> f32 {
        f32::from_bits(value.as_f64().unwrap() as u32)
    }
    fn words(value: &Value) -> [u32; 4] {
        let values = array(value);
        assert_eq!(values.len(), 4);
        std::array::from_fn(|i| values[i].as_f64().unwrap() as u32)
    }
    fn carry(value: &Value) -> BirthDistribution {
        let values = array(value);
        BirthDistribution { spacing: bits(&values[0]), offset: bits(&values[1]), burst_fraction: bits(&values[2]) }
    }

    /// Every EmitOverDistance call of the native per-frame Update1b receipts
    /// (the frame head of the transform-driven distance system, with the
    /// exported start block, Velocity and CustomData modules or none) through
    /// the law: the state before and the call arguments come from the native
    /// row, the result is compared bit for bit with the native state after,
    /// the returned count and the StartParticles command it issued (time,
    /// dt, pending argument). Two one-rule arms read the same rows.
    #[test]
    #[ignore = "MOLY_UPDATE1B_FRAMES, MOLY_UPDATE1B_FRAMES_EXTRA and MOLY_UPDATE1B_FRAMES_MODULES must identify the current JP per-frame Update1b receipts"]
    fn replays_native_emit_over_distance_at_the_frame_head() {
        let (mut calls, mut births, mut nonzero) = (0_usize, 0_u64, 0_usize);
        let (mut unregrouped, mut undrawn) = (0_usize, 0_usize);
        for variable in ["MOLY_UPDATE1B_FRAMES", "MOLY_UPDATE1B_FRAMES_EXTRA", "MOLY_UPDATE1B_FRAMES_MODULES"] {
            let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is not set"));
            let receipt = json::parse(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(
                at(&receipt, "sourceSha256").as_str(),
                Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
            );
            for group in ["s11Cases", "s1Cases", "controls"] {
                let Some(cases) = receipt.get(group).and_then(Value::as_array) else { continue };
                for case in cases {
                    if case.get("patched").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let config = at(case, "config");
                    let rate = field(config, "rate_distance");
                    let emission = EmissionParams {
                        rate_over_time: MinMaxCurve::Constant(field(config, "rate_time")),
                        rate_over_distance: MinMaxCurve::Constant(rate),
                        bursts: Vec::new(),
                    };
                    let (_, law) = ConstantAutonomousEmission::from_params_with_distance(
                        &MinMaxCurve::Constant(0.0), field(config, "duration"), boolean(config, "looping"), &emission)
                        .unwrap();
                    for frame in array(at(case, "frames")) {
                        let row = at(frame, "eod");
                        if matches!(row, Value::Null) {
                            continue;
                        }
                        let law = law.expect("a native distance call has a distance rate above zero");
                        let arguments = array(at(row, "argsBits"));
                        let command = array(at(frame, "starts")).iter()
                            .find(|start| field(start, "lr") == 14_148_616.0)
                            .expect("the distance call issues its StartParticles command");
                        let command_arguments = array(at(command, "argsBits"));
                        let pending = bits(&command_arguments[3]);
                        let velocity = std::array::from_fn(|axis| bits(&array(at(row, "velocityBits"))[axis]));
                        let before = at(row, "stateBefore");
                        let mut state = AutonomousEmissionState {
                            distribution: carry(at(before, "f")),
                            random: ScalarRandom { words: words(at(before, "rng")) },
                        };
                        let batch = law.emit(&mut state, velocity, bits(&arguments[2]), pending, bits(&arguments[0]),
                            crate::particle::initial::initial_reciprocal).unwrap();
                        let after = at(row, "stateAfter");
                        exact_distribution(state.distribution, carry(at(after, "f")));
                        assert_eq!(state.random.words, words(at(after, "rng")));
                        assert_eq!(batch.count as f32, field(row, "returned"));
                        assert_eq!(batch.count as f32, field(command, "requested"));
                        assert_eq!(batch.count as f32, field(command, "rateCount"));
                        exact(batch.time, bits(&arguments[1]), "command time");
                        exact(batch.time, bits(&command_arguments[0]), "command time argument");
                        exact(batch.dt, bits(&command_arguments[1]), "command dt");
                        exact(bits(&command_arguments[2]), 0.0, "no births-ahead argument");
                        calls += 1;
                        births += u64::from(batch.count);
                        nonzero += usize::from(batch.count > 0);
                        // Arm: the squared sum regrouped as x*x + (y*y + z*z).
                        let length = (velocity[0] * velocity[0] + (velocity[1] * velocity[1] + velocity[2] * velocity[2])).sqrt();
                        let amount = (rate * bits(&arguments[2])) * length;
                        let full = carry(at(before, "f")).offset + amount;
                        unregrouped += usize::from((full - (full as u32) as f32).to_bits() != state.distribution.offset.to_bits());
                        // Arm: no draw before the amount.
                        undrawn += usize::from(words(at(before, "rng")) != words(at(after, "rng")));
                    }
                }
            }
        }
        println!("emit over distance replay: {calls} native calls, {nonzero} with births, {births} births; \
            arms: regrouped squared sum {unregrouped}, no draw {undrawn}");
        assert_eq!(calls, 591);
        assert!(nonzero > 0 && births > 0);
        assert!(unregrouped > 0 && undrawn == calls);
    }

}
