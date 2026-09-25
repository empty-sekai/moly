//! Guarded birth-sub-emitter counts and birth timing from current native
//! SubModule, RecordEmit, EmitOverTime/Distance and StartModules.
//!
//! This law does not enable a source system. The caller resolves the actual
//! child system, owns one accumulator per parent particle/cached birth edge,
//! and sends both batches to that child's existing shared pool. Never create
//! a separate pool per parent particle. RNG, owner transforms, native update
//! flag ownership, module order and source renderer admission stay outside.
//!
//! New particles must use `buffer::birth_capacity` before initialization and
//! `buffer::finish_births` after the batch. Native four-wide birth packing and
//! ring/death side data must not be replaced with a linear append here.

use crate::particle::emit::{BurstCycles, EmissionState};
use crate::particle::seed_owner::ScalarRandom;
use crate::particle::schema::{EmissionParams, SubEmitterParams, SubEmitterTrigger};
use crate::particle::value::MinMaxCurve;

const MIN_AMOUNT: f32 = f32::from_bits(0x38d1_b717);
const MIN_BIRTH_FRACTION: f32 = f32::from_bits(0x3586_37bd);
// Native scalar counts convert to integers. Keep this initial implementation
// below the point where f32 no longer represents every integer exactly.
const MAX_EXACT_COUNT: f32 = 16_777_215.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    UnsupportedConfiguration,
    InvalidInput,
    CountOutOfRange,
    /// A birth edge that inherits properties: the inherited block is
    /// transcribed only for death edges ([`crate::particle::inherit`]).
    InheritedProperties,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BirthInterval {
    pub previous: f32,
    pub current: f32,
    pub previous_normalized: f32,
    pub current_normalized: f32,
}

/// The three native emission-state scalars passed to StartModules. These are
/// not RNG state and must not be substituted for the child birth generator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BirthDistribution {
    pub spacing: f32,
    pub offset: f32,
    pub burst_fraction: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BirthTiming {
    pub fraction: f32,
    pub dt: f32,
    pub curve_time: f32,
}

impl BirthDistribution {
    /// `index` is the local index in this one command, before pool packing.
    /// Keep scalar operation order: native uses distinct multiply/add ops.
    pub fn timing(
        self,
        index: u32,
        rate_count: u32,
        dt: f32,
        previous: f32,
        current: f32,
    ) -> Result<BirthTiming, Refused> {
        if ![
            self.spacing,
            self.offset,
            self.burst_fraction,
            dt,
            previous,
            current,
        ]
        .iter()
        .all(|v| v.is_finite())
            || self.spacing < 0.0
            || self.offset < 0.0
            || dt < 0.0
            || current < previous
            || index as f64 > MAX_EXACT_COUNT as f64
            || rate_count as f64 > MAX_EXACT_COUNT as f64
        {
            return Err(Refused::InvalidInput);
        }
        let raw = if index < rate_count {
            self.spacing * (self.offset + index as f32)
        } else {
            self.burst_fraction
        };
        if !raw.is_finite() {
            return Err(Refused::InvalidInput);
        }
        let fraction = raw.max(MIN_BIRTH_FRACTION).min(1.0);
        Ok(BirthTiming {
            fraction,
            dt: dt * fraction,
            curve_time: (current + fraction * (previous - current))
                .max(0.0)
                .min(1.0),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BirthBatch {
    pub count: u32,
    pub rate_count: u32,
    pub distribution: BirthDistribution,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledBirths {
    pub interval: BirthInterval,
    /// Native RecordEmit issues the distance command first.
    pub distance: BirthBatch,
    /// Time-rate and the one supported burst share this second command.
    pub time: BirthBatch,
}

/// Only the native-probed count subset: resolved Birth, properties 0,
/// probability 1, at most two active cached Birth edges, constant delay/rates,
/// and at most one constant-count, probability-1, nonrepeating burst.
#[derive(Debug, Clone, Copy)]
pub struct ConstantBirthSchedule {
    delay: f32,
    duration: f32,
    time_rate: f32,
    distance_rate: f32,
    burst: Option<(f32, u32)>,
}

impl ConstantBirthSchedule {
    /// `cached_birth_edges` is the count after native-equivalent resolution,
    /// active filtering and deduplication. Serialized path IDs cannot supply
    /// native instance-ID ordering. This function does not infer that order.
    pub fn from_params(
        edge: &SubEmitterParams,
        cached_birth_edges: usize,
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<Self, Refused> {
        let unsupported = Refused::UnsupportedConfiguration;
        if edge.trigger != SubEmitterTrigger::Birth
            || edge.properties != 0
            || edge.probability != 1.0
            || edge.emitter.as_ref().is_none_or(|s| s.is_empty())
            || !(1..=2).contains(&cached_birth_edges)
            || emission.bursts.len() > 1
        {
            return Err(unsupported);
        }
        let constant = |curve: &MinMaxCurve| match curve {
            MinMaxCurve::Constant(value) if value.is_finite() && *value >= 0.0 => Ok(*value),
            _ => Err(unsupported),
        };
        if !duration.is_finite() || duration <= 0.0 {
            return Err(unsupported);
        }
        let burst = emission
            .bursts
            .first()
            .map(|b| {
                if b.probability != 1.0
                    || !matches!(b.cycles, BurstCycles::Finite(n) if n.get() == 1)
                    || !b.time.is_finite()
                    || b.time < 0.0
                {
                    return Err(unsupported);
                }
                let count = constant(&b.count)?;
                if count > MAX_EXACT_COUNT {
                    return Err(Refused::CountOutOfRange);
                }
                // Native constant burst converts toward zero. A repeating burst's
                // interval is irrelevant here because cycles is exactly one.
                Ok((b.time, count as u32))
            })
            .transpose()?;
        Ok(Self {
            delay: constant(delay)?,
            duration: if looping { f32::MAX } else { duration },
            time_rate: constant(&emission.rate_over_time)?,
            distance_rate: constant(&emission.rate_over_distance)?,
            burst,
        })
    }

    /// Returns None outside the parent's shifted [0,1) life interval or after
    /// the nonlooping child duration. Parent age is percent, not seconds.
    pub fn interval(
        self,
        age: f32,
        inverse_lifetime: f32,
        dt: f32,
    ) -> Result<Option<BirthInterval>, Refused> {
        birth_interval(self.delay, self.duration, age, inverse_lifetime, dt)
    }

    /// Distance and time consume the SAME parent-particle/edge carry in that
    /// order. `distance_speed` is the speed used by native RecordEmit's owner
    /// conversion; this scalar law does not choose a coordinate system.
    /// Refusal never modifies carry. Zero-count batches remain explicit.
    pub fn schedule(
        self,
        carry: &mut EmissionState,
        age: f32,
        inverse_lifetime: f32,
        dt: f32,
        distance_speed: f32,
    ) -> Result<Option<ScheduledBirths>, Refused> {
        if !distance_speed.is_finite()
            || distance_speed < 0.0
            || !carry.to_emit_accumulator.is_finite()
            || !(0.0..1.0).contains(&carry.to_emit_accumulator)
        {
            return Err(Refused::InvalidInput);
        }
        let Some(interval) = self.interval(age, inverse_lifetime, dt)? else {
            return Ok(None);
        };
        let mut remainder = carry.to_emit_accumulator;
        // A zero distance rate returns before writing native emission state.
        let distance = if self.distance_rate == 0.0 {
            BirthBatch {
                count: 0,
                rate_count: 0,
                distribution: BirthDistribution {
                    spacing: 0.0,
                    offset: remainder,
                    burst_fraction: 0.0,
                },
            }
        } else {
            let amount =
                (self.distance_rate * (interval.current - interval.previous)) * distance_speed;
            let (count, distribution) = accumulate(&mut remainder, amount, 0.0)?;
            BirthBatch {
                count,
                rate_count: count,
                distribution,
            }
        };
        let previous = interval.previous.max(0.0);
        let current = interval.current.max(0.0);
        let (burst_count, burst_fraction) = match self.burst {
            Some((time, count)) if previous <= time && time < current => {
                // AccumulateBursts is left-closed/right-open, unlike the
                // older autonomous `emit::burst_check` behavior hypothesis.
                let relative = (time - previous) / (current - previous);
                (
                    count,
                    if relative < 0.0 {
                        1.0
                    } else {
                        1.0 - relative.min(1.0)
                    },
                )
            }
            _ => (0, 0.0),
        };
        let (rate_count, distribution) = accumulate(
            &mut remainder,
            (current - previous) * self.time_rate,
            burst_fraction,
        )?;
        let count = rate_count
            .checked_add(burst_count)
            .ok_or(Refused::CountOutOfRange)?;
        if count as f64 > MAX_EXACT_COUNT as f64 {
            return Err(Refused::CountOutOfRange);
        }
        carry.to_emit_accumulator = remainder;
        Ok(Some(ScheduledBirths {
            interval,
            distance,
            time: BirthBatch {
                count,
                rate_count,
                distribution,
            },
        }))
    }
}

fn accumulate(
    carry: &mut f32,
    amount: f32,
    burst_fraction: f32,
) -> Result<(u32, BirthDistribution), Refused> {
    let sum = *carry + amount;
    if !sum.is_finite() || amount < 0.0 || sum > MAX_EXACT_COUNT {
        return Err(Refused::CountOutOfRange);
    }
    let count = sum as u32;
    *carry = sum - count as f32;
    Ok((
        count,
        BirthDistribution {
            spacing: if amount < MIN_AMOUNT {
                1.0
            } else {
                1.0 / amount
            },
            offset: *carry,
            burst_fraction,
        },
    ))
}

/// The parent particle's life window for one edge. The child's start delay
/// shifts it; outside the shifted [0, 1) or at and past the child duration
/// the particle records nothing. Parent age is percent, not seconds.
fn birth_interval(
    delay: f32,
    duration: f32,
    age: f32,
    inverse_lifetime: f32,
    dt: f32,
) -> Result<Option<BirthInterval>, Refused> {
    if ![age, inverse_lifetime, dt].iter().all(|v| v.is_finite())
        || inverse_lifetime <= 0.0
        || dt < 0.0
    {
        return Err(Refused::InvalidInput);
    }
    let current_normalized = age * 0.01 - inverse_lifetime * delay;
    let current = current_normalized / inverse_lifetime;
    if !(0.0..1.0).contains(&current_normalized) || current >= duration {
        return Ok(None);
    }
    let previous_normalized =
        (age + inverse_lifetime * (dt * -100.0)) * 0.01 - inverse_lifetime * delay;
    let previous = previous_normalized / inverse_lifetime;
    if !previous.is_finite() || !current.is_finite() || previous > current {
        return Err(Refused::InvalidInput);
    }
    Ok(Some(BirthInterval {
        previous,
        current,
        previous_normalized,
        current_normalized,
    }))
}

// ---- Parent side: one recorded birth event per active parent particle ----
//
// The parent's sub-emitter module visits, per cached birth edge in slot order,
// every particle of the range it is handed and records an event for each one
// whose life window is open for that edge's child. The event seeds its own
// emission state, runs the child's distance emission and then its time
// emission (rate and the one burst) on it, writes the carry back to the parent
// particle and, when anything was counted, issues two commands to the child:
// the distance batch, then the time and burst batch.

/// Parent state the recording reads besides the particle, as the parent holds
/// it at the call: its local-to-world matrix (column-major, source axes; not
/// read in World space), whether it simulates in World space, the system time
/// it still has to simulate (the current slice's own step not yet taken off)
/// and the first word of its own emission random stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventOwner {
    pub local_to_world: [f32; 16],
    pub world_space: bool,
    pub accumulated_time: f32,
    pub emission_word: u32,
}

/// One parent particle as the recording reads it, in source axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventParticle {
    pub seed: u32,
    pub age_percent: f32,
    pub inverse_lifetime: f32,
    pub position: [f32; 3],
    /// Persistent plus animated velocity, summed per axis.
    pub velocity: [f32; 3],
}

/// The emission state one event hands to the child's emission functions:
/// the three birth-distribution scalars (the carry is the middle one) and the
/// event's own random words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventEmission {
    pub distribution: BirthDistribution,
    pub random: ScalarRandom,
}

impl EventEmission {
    /// Every event is seeded afresh from the particle seed plus the parent's
    /// emission word (wrapping), expanded by three LCG steps. It never reads
    /// or advances the parent's own emission stream beyond that one word, and
    /// it is not the child's Initial stream.
    pub fn seeded(particle_seed: u32, emission_word: u32, carry: f32) -> Self {
        Self {
            distribution: BirthDistribution {
                spacing: 0.0,
                offset: carry,
                burst_fraction: 0.0,
            },
            random: ScalarRandom::from_seed(particle_seed.wrapping_add(emission_word)),
        }
    }
}

/// A command to the child system, with the fields the child consumes; the
/// native byte layout is [`SubEmitterCommand::to_bytes`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubEmitterCommand {
    /// World position and world velocity of the parent particle, source axes.
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// The inherited block; an edge that inherits nothing sends the neutral
    /// words with the parent particle's seed last.
    pub inherited: [u32; 13],
    pub count: u64,
    pub rate_count: u64,
    /// The interval in child seconds (current minus previous, one f32
    /// subtraction), both normalized ends, then the parent's time still to
    /// simulate at the call (the child's catch-up).
    pub dt: f32,
    pub previous_normalized: f32,
    pub current_normalized: f32,
    pub catch_up: f32,
    /// The scalars this command's births are spread with.
    pub emission: BirthDistribution,
}

impl SubEmitterCommand {
    /// The native command bytes. The first word is a pointer to the emission
    /// scalars ([`Self::emission_bytes`]) and the word after the inherited
    /// block is not a field; both are written as zero.
    pub fn to_bytes(&self) -> [u8; 0x78] {
        let mut raw = [0_u8; 0x78];
        let mut put = |offset: usize, bytes: [u8; 4]| raw[offset..offset + 4].copy_from_slice(&bytes);
        for axis in 0..3 {
            put(0x08 + axis * 4, self.position[axis].to_le_bytes());
            put(0x14 + axis * 4, self.velocity[axis].to_le_bytes());
        }
        for (index, word) in self.inherited.iter().enumerate() {
            put(0x20 + index * 4, word.to_le_bytes());
        }
        put(0x68, self.dt.to_le_bytes());
        put(0x6c, self.previous_normalized.to_le_bytes());
        put(0x70, self.current_normalized.to_le_bytes());
        put(0x74, self.catch_up.to_le_bytes());
        raw[0x58..0x60].copy_from_slice(&self.count.to_le_bytes());
        raw[0x60..0x68].copy_from_slice(&self.rate_count.to_le_bytes());
        raw
    }

    pub fn emission_bytes(&self) -> [u8; 12] {
        let mut out = [0_u8; 12];
        out[0..4].copy_from_slice(&self.emission.spacing.to_le_bytes());
        out[4..8].copy_from_slice(&self.emission.offset.to_le_bytes());
        out[8..12].copy_from_slice(&self.emission.burst_fraction.to_le_bytes());
        out
    }
}

/// One recorded event: the window, the emission state before and after, and
/// the two commands unless the distance count, the time count and the rate
/// count were all zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BirthEventRecord {
    pub interval: BirthInterval,
    pub before: EventEmission,
    pub after: EventEmission,
    pub commands: Option<[SubEmitterCommand; 2]>,
}

/// The count law of one cached birth edge, read from the child's start delay,
/// duration, loop flag and Emission module: a constant start delay, constant
/// rate over time and over distance, and the child's bursts (up to the eight
/// slots, a constant or two-constant count; every time, cycle count, repeat
/// interval and probability). The event's time emission is the child's
/// EmitOverTime over the window [previous, current) of the particle's life in
/// child seconds: never a wrap there (previous <= current), so its one
/// AccumulateBursts window allows repeats, as an autonomous slice without a
/// wrap does. The child's own update never runs here; this only decides what
/// the parent sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct BirthEdgeLaw {
    delay: f32,
    /// The child duration, or f32::MAX for a looping child.
    duration: f32,
    time_rate: f32,
    distance_rate: f32,
    bursts: crate::particle::autonomous_emission::BurstSchedule,
}

impl BirthEdgeLaw {
    /// `cached_birth_edges` counts the parent's resolved birth edges. Only the
    /// first two own a persistent per-particle carry; a third would use a
    /// carry computed inside the call, which is not transcribed, so a parent
    /// with more than two is refused.
    pub fn from_params(
        edge: &SubEmitterParams,
        cached_birth_edges: usize,
        delay: &MinMaxCurve,
        duration: f32,
        looping: bool,
        emission: &EmissionParams,
    ) -> Result<Self, Refused> {
        let unsupported = Refused::UnsupportedConfiguration;
        if edge.trigger == SubEmitterTrigger::Birth && edge.properties != 0 {
            return Err(Refused::InheritedProperties);
        }
        if edge.trigger != SubEmitterTrigger::Birth
            || edge.probability != 1.0
            || edge.emitter.as_ref().is_none_or(|s| s.is_empty())
            || !(1..=2).contains(&cached_birth_edges)
            || !duration.is_finite()
            || duration <= 0.0
        {
            return Err(unsupported);
        }
        let constant = |curve: &MinMaxCurve| match curve {
            MinMaxCurve::Constant(value) if value.is_finite() && *value >= 0.0 => Ok(*value),
            _ => Err(unsupported),
        };
        let bursts = crate::particle::autonomous_emission::BurstSchedule::from_params(&emission.bursts)
            .map_err(|_| unsupported)?;
        Ok(Self {
            delay: constant(delay)?,
            duration: if looping { f32::MAX } else { duration },
            time_rate: constant(&emission.rate_over_time)?,
            distance_rate: constant(&emission.rate_over_distance)?,
            bursts,
        })
    }

    /// The child duration the window closes at (f32::MAX when looping).
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Record the event of one parent particle for this edge. `carry` is the
    /// particle's persistent carry for this edge; it is replaced by the
    /// carry after the event even when no command is issued, and left alone
    /// when the particle's window is closed or the event is refused. `dt` is
    /// this particle's lane of the call's time vector.
    pub fn record(
        &self,
        particle: &EventParticle,
        carry: &mut f32,
        dt: f32,
        owner: &EventOwner,
    ) -> Result<Option<BirthEventRecord>, Refused> {
        let Some(interval) =
            birth_interval(self.delay, self.duration, particle.age_percent, particle.inverse_lifetime, dt)?
        else {
            return Ok(None);
        };
        self.record_window(particle, carry, interval, owner).map(Some)
    }

    /// The event of a particle whose window for this edge is open, from that
    /// window: world position and velocity, the seeded state, the distance
    /// and time emission, the carry write-back and the commands.
    pub fn record_window(
        &self,
        particle: &EventParticle,
        carry: &mut f32,
        interval: BirthInterval,
        owner: &EventOwner,
    ) -> Result<BirthEventRecord, Refused> {
        if !carry.is_finite()
            || !(0.0..1.0).contains(carry)
            || ![interval.previous, interval.current, interval.previous_normalized, interval.current_normalized]
                .iter()
                .all(|v| v.is_finite())
            || interval.previous > interval.current
        {
            return Err(Refused::InvalidInput);
        }
        let (position, velocity) = world_particle(particle, owner);
        if position.iter().chain(velocity.iter()).any(|v| !v.is_finite()) {
            return Err(Refused::InvalidInput);
        }
        let before = EventEmission::seeded(particle.seed, owner.emission_word, *carry);
        let mut state = before;
        let seconds = interval.current - interval.previous;
        // Distance: a zero rate returns before drawing or writing anything.
        let distance_count = if self.distance_rate == 0.0 {
            0
        } else {
            // The draw's random factor does not enter a constant rate.
            state.random.next_u32();
            let speed = ((velocity[0] * velocity[0] + velocity[1] * velocity[1])
                + velocity[2] * velocity[2])
                .sqrt();
            let amount = (self.distance_rate * seconds) * speed;
            spread(&mut state.distribution, amount)?
        };
        let distance_emission = state.distribution;
        // Time: one unconditional draw first; a constant rate does not read it.
        state.random.next_u32();
        let previous = non_negative(interval.previous);
        let current = non_negative(interval.current);
        if current < previous {
            return Err(Refused::InvalidInput);
        }
        let amount = if self.time_rate > 0.0 {
            0.0 + (current - previous) * self.time_rate
        } else {
            0.0
        };
        // AccumulateBursts over [previous, current), repeats allowed: a time
        // inside is left-closed and right-open, a repeat hits once however
        // many repeats the window spans; each hit draws its probability and
        // its count before the fraction is written, on a zero count too.
        let mut fraction = state.distribution.burst_fraction;
        let burst_sum = self.bursts.accumulate(previous, current, repeats_allowed(), &mut state.random, &mut fraction);
        state.distribution.burst_fraction = fraction;
        let rate_count = spread(&mut state.distribution, amount)?;
        // The rate count and the burst sum, sign-extended, added in 64 bits.
        let total = i64::from(rate_count) + i64::from(burst_sum);
        if !(0..=MAX_EXACT_COUNT as i64).contains(&total) {
            return Err(Refused::CountOutOfRange);
        }
        let total = total as u32;
        *carry = state.distribution.offset;
        let commands = (distance_count != 0 || total != 0 || rate_count != 0).then(|| {
            let command = |count: u32, rate_count: u32, emission: BirthDistribution| SubEmitterCommand {
                position,
                velocity,
                inherited: neutral_inherited(particle.seed),
                count: u64::from(count),
                rate_count: u64::from(rate_count),
                dt: seconds,
                previous_normalized: interval.previous_normalized,
                current_normalized: interval.current_normalized,
                catch_up: owner.accumulated_time,
                emission,
            };
            [
                command(distance_count, distance_count, distance_emission),
                command(total, rate_count, state.distribution),
            ]
        });
        Ok(BirthEventRecord {
            interval,
            before,
            after: state,
            commands,
        })
    }
}

/// The inherited block of an edge that inherits nothing: white, unit size,
/// zero rotation, the forward axis, unit lifetime, unbounded duration, then
/// the parent particle's seed.
pub(crate) fn neutral_inherited(seed: u32) -> [u32; 13] {
    let one = 1.0_f32.to_bits();
    [u32::MAX, one, one, one, 0, 0, 0, 0, 0, one, one, f32::INFINITY.to_bits(), seed]
}

/// The particle's world position and velocity: in World space as stored; else
/// through the owner matrix, each axis the three products summed first to
/// last and the translation added after them (velocity takes no translation).
fn world_particle(particle: &EventParticle, owner: &EventOwner) -> ([f32; 3], [f32; 3]) {
    if owner.world_space {
        return (particle.position, particle.velocity);
    }
    let m = &owner.local_to_world;
    let linear = |v: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|a| (m[a] * v[0] + m[4 + a] * v[1]) + m[8 + a] * v[2])
    };
    let local = linear(particle.position);
    (
        std::array::from_fn(|a| m[12 + a] + local[a]),
        linear(particle.velocity),
    )
}

/// The larger of the value and +0, with a NaN or a negative value (or -0)
/// giving +0, as the emission functions clamp their window ends.
fn non_negative(value: f32) -> f32 {
    if value > 0.0 {
        value
    } else {
        0.0
    }
}

// The event's AccumulateBursts window allows repeats; the replay arm that
// turns them off must differ from native.
#[cfg(test)]
thread_local! { static NO_REPEATS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
fn repeats_allowed() -> bool {
    !NO_REPEATS.with(|arm| arm.get())
}
#[cfg(test)]
pub(crate) fn set_no_repeats_arm(on: bool) {
    NO_REPEATS.with(|arm| arm.set(on));
}
#[cfg(not(test))]
fn repeats_allowed() -> bool {
    true
}

/// Add `amount` to the carry and take the whole part as the count: the
/// spacing becomes 1 / amount (1 below the minimum amount), the carry the
/// fractional rest. The burst fraction is left as it is.
fn spread(distribution: &mut BirthDistribution, amount: f32) -> Result<u32, Refused> {
    let sum = amount + distribution.offset;
    if !amount.is_finite() || amount < 0.0 || !sum.is_finite() || sum > MAX_EXACT_COUNT {
        return Err(Refused::CountOutOfRange);
    }
    let count = sum as u32;
    distribution.spacing = if amount >= MIN_AMOUNT { 1.0 / amount } else { 1.0 };
    distribution.offset = sum - count as f32;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::emit::Burst;

    fn schedule(time: f32, distance: f32, burst: Option<(f32, f32)>) -> ConstantBirthSchedule {
        let edge = SubEmitterParams {
            emitter: Some("resolved-child".into()),
            source_pointer: Default::default(),
            trigger: SubEmitterTrigger::Birth,
            properties: 0,
            probability: 1.0,
        };
        let emission = EmissionParams {
            rate_over_time: MinMaxCurve::Constant(time),
            rate_over_distance: MinMaxCurve::Constant(distance),
            bursts: burst
                .into_iter()
                .map(|(time, count)| Burst {
                    time,
                    count: MinMaxCurve::Constant(count),
                    cycles: BurstCycles::from_serialized(1),
                    repeat_interval: 1.0,
                    probability: 1.0,
                })
                .collect(),
        };
        ConstantBirthSchedule::from_params(
            &edge,
            1,
            &MinMaxCurve::Constant(0.0),
            5.0,
            false,
            &emission,
        )
        .unwrap()
    }

    #[test]
    fn source_burst_boundary_is_left_closed_and_right_open() {
        let s = schedule(0.0, 0.0, Some((0.125, 3.0)));
        let mut carry = EmissionState::default();
        assert_eq!(
            s.schedule(&mut carry, 12.5, 1.0, 0.125, 0.0)
                .unwrap()
                .unwrap()
                .time
                .count,
            0
        );
        let next = s
            .schedule(&mut carry, 25.0, 1.0, 0.125, 0.0)
            .unwrap()
            .unwrap();
        assert_eq!(next.time.count, 3);
        assert_eq!(next.time.distribution.burst_fraction, 1.0);
        assert!(s.interval(100.0, 1.0, 0.125).unwrap().is_none());
    }

    #[test]
    fn distance_precedes_time_and_carry_is_shared() {
        let mut carry = EmissionState {
            to_emit_accumulator: 0.75,
        };
        let result = schedule(2.0, 2.0, None)
            .schedule(&mut carry, 25.0, 1.0, 0.125, 1.0)
            .unwrap()
            .unwrap();
        assert_eq!((result.distance.count, result.time.rate_count), (1, 0));
        assert_eq!(carry.to_emit_accumulator, 0.25);
        assert_eq!(result.distance.distribution.spacing, 4.0);
        assert_eq!(result.distance.distribution.offset, 0.0);
    }

    #[test]
    fn exact_frame_end_birth_keeps_native_positive_fraction() {
        let d = BirthDistribution {
            spacing: 0.25,
            offset: 0.0,
            burst_fraction: 0.5,
        };
        let first = d.timing(0, 2, 0.25, 0.25, 0.5).unwrap();
        assert_eq!(first.fraction.to_bits(), 0x358637bd);
        assert_eq!(first.dt, 0.25 * MIN_BIRTH_FRACTION);
        assert_eq!(d.timing(1, 2, 0.25, 0.25, 0.5).unwrap().dt, 0.0625);
        assert_eq!(d.timing(2, 2, 0.25, 0.25, 0.5).unwrap().dt, 0.125);
    }

    #[test]
    fn mixed_native_commands_keep_distinct_birth_distributions() {
        // Native RecordEmit observation: quarter-second interval, one unit
        // distance amount, 1.25 time-rate amount and midpoint count-three burst.
        let mut carry = EmissionState::default();
        let out = schedule(5.0, 4.0, Some((0.375, 3.0)))
            .schedule(&mut carry, 50.0, 1.0, 0.25, 1.0)
            .unwrap()
            .unwrap();
        assert_eq!((out.distance.count, out.distance.rate_count), (1, 1));
        assert_eq!(
            out.distance.distribution,
            BirthDistribution {
                spacing: 1.0,
                offset: 0.0,
                burst_fraction: 0.0
            }
        );
        assert_eq!((out.time.count, out.time.rate_count), (4, 1));
        assert_eq!(
            out.time.distribution,
            BirthDistribution {
                spacing: 0.8,
                offset: 0.25,
                burst_fraction: 0.5
            }
        );
    }

    #[test]
    fn refusal_preserves_parent_carry() {
        let mut carry = EmissionState {
            to_emit_accumulator: 0.25,
        };
        assert!(
            schedule(1.0, 1.0, None)
                .schedule(&mut carry, 25.0, 1.0, 0.25, f32::NAN)
                .is_err()
        );
        assert_eq!(carry.to_emit_accumulator, 0.25);
        let s = schedule(f32::MAX, 1.0, None);
        assert!(s.schedule(&mut carry, 25.0, 1.0, 0.25, 1.0).is_err());
        assert_eq!(carry.to_emit_accumulator, 0.25);
    }

    #[test]
    #[ignore = "MOLY_SUB_EMISSION_NATIVE_DIR must identify the private current-native receipts"]
    fn replays_external_current_native_examples() {
        // Optional sample driver. Reads the actual saved native observations;
        // no game-owned names/assets/native addresses are embedded in source.
        // Receipts retain only bounded examples, so this test reports their
        // actual case count rather than claiming the probe's whole corpus.
        use crate::particle::json::{Value, parse};
        let root = std::path::PathBuf::from(std::env::var("MOLY_SUB_EMISSION_NATIVE_DIR").unwrap());
        let read = |name: &str| parse(&std::fs::read(root.join(name)).unwrap()).unwrap();
        let number = |v: &Value| v.as_f64().unwrap() as f32;
        let at = |v: &Value, k: &str| v.get(k).unwrap().clone();
        let controls = read("module-controls-native.json");
        let examples = controls
            .get("subEmissionReplay")
            .unwrap()
            .get("examples")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(!examples.is_empty());
        let mut parent_cases = 0;
        for example in examples {
            let row = example.get("input").unwrap();
            let native = example.get("result").unwrap();
            let field = |key| number(row.get(key).unwrap());
            let burst = row.get("burst").filter(|v| **v != Value::Null).map(|b| {
                (
                    number(b.get("time").unwrap()),
                    number(b.get("count").unwrap()),
                )
            });
            let mut s = schedule(field("time_rate"), field("distance_rate"), burst);
            s.delay = field("delay");
            s.duration = if row.get("looping").unwrap().as_bool().unwrap() {
                f32::MAX
            } else {
                field("duration")
            };
            let array = |key| row.get(key).unwrap().as_array().unwrap();
            let mut expected_commands = Vec::new();
            for lane in 0..4 {
                let velocity = array("velocities")[lane].as_array().unwrap();
                let v: Vec<_> = velocity.iter().map(number).collect();
                let speed = ((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]).sqrt();
                let mut carry = EmissionState {
                    to_emit_accumulator: number(&array("accumulators")[lane]),
                };
                if let Some(out) = s
                    .schedule(
                        &mut carry,
                        number(&array("ages")[lane]),
                        number(&array("inverses")[lane]),
                        field("dt"),
                        speed,
                    )
                    .unwrap()
                {
                    if out.distance.count != 0 || out.time.count != 0 {
                        expected_commands.push((out.distance.count, out.distance.rate_count));
                        expected_commands.push((out.time.count, out.time.rate_count));
                    }
                }
                assert_eq!(
                    carry.to_emit_accumulator,
                    number(&native.get("accumulators").unwrap().as_array().unwrap()[lane])
                );
                parent_cases += 1;
            }
            let actual_commands: Vec<_> = native
                .get("childCommands")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        number(c.get("count").unwrap()) as u32,
                        number(c.get("rateCount").unwrap()) as u32,
                    )
                })
                .collect();
            assert_eq!(expected_commands, actual_commands);
        }
        let child = read("child-emit-native.json");
        let examples = child
            .get("replay")
            .unwrap()
            .get("examples")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(!examples.is_empty());
        let mut timed_lanes = 0;
        for example in examples {
            let input = example.get("input").unwrap();
            let observed = example.get("observed").unwrap();
            let field = |key| number(input.get(key).unwrap());
            let state = at(input, "emission");
            let state = state.as_array().unwrap();
            let distribution = BirthDistribution {
                spacing: number(&state[0]),
                offset: number(&state[1]),
                burst_fraction: number(&state[2]),
            };
            let aligned = (field("old") as u32 + 3) & !3;
            let calls = observed.get("calls").unwrap().as_array().unwrap();
            for call in calls {
                // The receipt's curveTime field is emitted only at the native
                // StartVelocity boundary, before a birth block is integrated.
                if let Some(times) = call.get("curveTime") {
                    let local_start = number(call.get("start").unwrap()) as u32 - aligned;
                    for (lane, t) in times.as_array().unwrap().iter().enumerate() {
                        let timing = distribution
                            .timing(
                                local_start + lane as u32,
                                field("rate_count") as u32,
                                field("dt"),
                                field("previous"),
                                field("current"),
                            )
                            .unwrap();
                        assert_eq!(timing.curve_time, number(t));
                        timed_lanes += 1;
                    }
                }
            }
            let expected_steps = {
                let plan = crate::particle::child_emit::catch_up_plan(
                    field("catchup"), field("frame_dt"), true, field("flags") as u32, f32::MAX);
                let (mut remaining, mut steps) = (field("catchup"), 0);
                while plan.runs {
                    remaining -= plan.frame_dt;
                    steps += 1;
                    if !(remaining >= plan.step) {
                        break;
                    }
                }
                steps
            };
            let actual_steps = calls
                .iter()
                .filter(|c| {
                    c.get("dt").is_some() && c.get("start").is_some() && c.get("end").is_none()
                })
                .count();
            assert_eq!(
                actual_steps,
                if field("count") == 0.0 {
                    0
                } else {
                    expected_steps
                }
            );
        }
        assert!(timed_lanes > 0);
        println!("current-native examples: {parent_cases} parent cases, {timed_lanes} birth lanes");
    }

    /// Parent birth events against native sub-emitter calls: every call of the
    /// receipt is rerun from the inputs recorded at the call (range, per-lane
    /// time vector, the parent arrays and both carries, accumulated time,
    /// emission word, owner matrix, space) and every record and command byte
    /// is compared. The child blocks are the receipt's own image of them.
    #[test]
    #[ignore = "MOLY_SUBEMITTER_PARENT_RECEIPT must identify the current native parent-event receipt"]
    fn replays_current_native_parent_birth_events() {
        parent_birth_events("MOLY_SUBEMITTER_PARENT_RECEIPT", |run| {
            let (calls, newborn, records, commands, mismatched) = run(None);
            println!("parent birth events: {calls} calls ({newborn} newborn), {records} records, {commands} commands, {mismatched} mismatched");
            assert_eq!((calls, newborn, records, commands, mismatched), (1426, 296, 13285, 800, 0));
            // One-rule arms read against the same native rows must fail.
            for arm in ["catchUpZero", "sliceDt", "burstLow", "burstHigh"] {
                let wrong = run(Some(arm)).4;
                println!("arm {arm}: {wrong} records differ");
                assert!(wrong > 0, "{arm} arm matched every native record");
            }
        });
    }

    /// The same per-call replay against the native calls of the 016 bubble
    /// parent (ConeVolume, World, rate one) whose two birth edges name
    /// children with one burst of two, cycle count 0 (repeat without end)
    /// and a 0.06 s interval: each event's window hits the burst whenever it
    /// crosses a repeat, once however many repeats it spans. The arm that
    /// turns repeats off must fail.
    #[test]
    #[ignore = "MOLY_SUBEMITTER_REPEAT_RECEIPT must identify the native repeating-burst parent-event receipt"]
    fn replays_native_repeating_burst_parent_events() {
        parent_birth_events("MOLY_SUBEMITTER_REPEAT_RECEIPT", |run| {
            let (calls, newborn, records, commands, mismatched) = run(None);
            println!("repeating-burst parent events: {calls} calls ({newborn} newborn), {records} records, {commands} commands, {mismatched} mismatched");
            assert_eq!(mismatched, 0);
            assert!(calls > 0 && newborn > 0 && records > 0 && commands > 0);
            let wrong = run(Some("noRepeats")).4;
            println!("arm noRepeats: {wrong} records differ");
            assert!(wrong > 0, "noRepeats arm matched every native record");
        });
    }

    /// Every native SubModule call of a parent-event receipt through the law:
    /// `verdict` gets the replay, which takes one arm and returns (calls,
    /// newborn calls, records, commands, mismatched).
    fn parent_birth_events(key: &str,
        verdict: impl FnOnce(&dyn Fn(Option<&str>) -> (usize, usize, usize, usize, usize))) {
        use crate::particle::emit::Burst;
        use crate::particle::json::{parse, Value};
        let path = std::env::var_os(key).expect("receipt path");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            receipt.get("librarySha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        fn word(v: &Value) -> u32 {
            v.as_f64().expect("native word") as u32
        }
        fn bits(v: &Value) -> f32 {
            f32::from_bits(word(v))
        }
        fn at<'a>(v: &'a Value, k: &str) -> &'a Value {
            v.get(k).unwrap_or_else(|| panic!("missing {k}"))
        }
        fn array(v: &Value) -> &[Value] {
            v.as_array().expect("array")
        }
        fn lane<'a>(particles: &'a Value, key: &str, index: usize) -> &'a Value {
            &array(at(particles, key))[index]
        }
        let hex = |v: &Value| -> Vec<u8> {
            let text = v.as_str().expect("hex");
            (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
        };
        let curve = |c: &Value| -> MinMaxCurve {
            match word(at(c, "mode")) {
                0 => MinMaxCurve::Constant(bits(at(c, "bits"))),
                3 => MinMaxCurve::TwoConstants { min: bits(at(c, "minBits")), max: bits(at(c, "maxBits")) },
                mode => panic!("curve mode {mode} is not in this receipt"),
            }
        };
        // The newborn sub-emitter call's return address, as the receipt names it
        // (its callSites block); a receipt that does not name it is refused.
        let newborn_call = receipt.get("callSites").and_then(|sites| sites.get("newbornCall")).and_then(Value::as_str)
            .expect("the receipt names no callSites.newbornCall: it cannot classify its calls");
        // arm: None = the law; "catchUpZero", "sliceDt", "burstLow", "burstHigh" one rule each. The
        // carry has no signal here: both children have zero rate and distance, so it stays zero; the
        // window-given replay below carries it.
        let run = |arm: Option<&str>| -> (usize, usize, usize, usize, usize) {
            let (mut calls, mut records, mut commands, mut mismatched, mut newborn) = (0, 0, 0, 0, 0);
            super::set_no_repeats_arm(arm == Some("noRepeats"));
            for parent in array(at(&receipt, "parentEvents")) {
                let sub = at(at(parent, "image"), "subEmitters");
                let edges = array(at(sub, "edges"));
                let children = array(at(sub, "children"));
                let births = edges.iter().filter(|e| word(at(e, "trigger")) == 0).count();
                assert_eq!(word(at(sub, "numEmitAccumulators")) as usize, births.min(2));
                let laws: Vec<BirthEdgeLaw> = edges.iter().zip(children).map(|(edge, child)| {
                    assert_eq!(word(at(edge, "trigger")), 0);
                    let bursts = array(at(child, "bursts")).iter().map(|b| {
                        let mut count = curve(at(b, "count"));
                        if let (Some(which), MinMaxCurve::TwoConstants { min, max }) = (arm, count.clone()) {
                            let (low, high) = if max < min { (max, min) } else { (min, max) };
                            if which == "burstLow" { count = MinMaxCurve::Constant(low); }
                            if which == "burstHigh" { count = MinMaxCurve::Constant(high); }
                        }
                        Burst {
                            time: bits(at(b, "timeBits")),
                            count,
                            cycles: BurstCycles::from_serialized(word(at(b, "cycles"))),
                            repeat_interval: bits(at(b, "intervalBits")),
                            probability: bits(at(b, "probabilityBits")),
                        }
                    }).collect();
                    let emission = EmissionParams {
                        rate_over_time: curve(at(child, "rate")),
                        rate_over_distance: curve(at(child, "distance")),
                        bursts,
                    };
                    let params = SubEmitterParams {
                        emitter: Some(at(edge, "node").as_str().unwrap().to_owned()),
                        source_pointer: Default::default(),
                        trigger: SubEmitterTrigger::Birth,
                        properties: word(at(edge, "properties")),
                        probability: bits(at(edge, "probabilityBits")),
                    };
                    BirthEdgeLaw::from_params(&params, births, &curve(at(child, "startDelay")),
                        bits(at(child, "durationBits")), at(child, "looping").as_bool().unwrap(), &emission)
                        .expect("receipt child block is inside the law")
                }).collect();
                let native_records = array(at(parent, "records"));
                let native_commands = array(at(parent, "commands"));
                let subcalls = array(at(parent, "subcalls"));
                // The slice step of every call, for the arm that gives newborn lanes the slice dt.
                let mut slice_step = vec![0.0_f32; subcalls.len()];
                for frame in array(at(parent, "frames")) {
                    let range = array(at(frame, "subcalls"));
                    for index in word(&range[0]) as usize..word(&range[1]) as usize {
                        slice_step[index] = bits(at(frame, "stepBits"));
                    }
                }
                for (index, call) in subcalls.iter().enumerate() {
                    calls += 1;
                    newborn += usize::from(at(call, "lr").as_str() == Some(newborn_call));
                    let start = word(at(call, "start")) as usize;
                    let end = word(at(call, "end")) as usize;
                    let dt4: Vec<f32> = if arm == Some("sliceDt") {
                        vec![slice_step[index]; 4]
                    } else {
                        array(at(call, "dt4")).iter().map(bits).collect()
                    };
                    let owner = EventOwner {
                        local_to_world: std::array::from_fn(|i| bits(&array(at(call, "owner"))[i])),
                        world_space: word(at(call, "simulation")) == 1,
                        accumulated_time: if arm == Some("catchUpZero") { 0.0 } else { bits(at(call, "stateZero")) },
                        emission_word: word(at(call, "ownerSeed")),
                    };
                    assert_eq!(word(at(call, "numAccumulators")) as usize, births.min(2));
                    let p = at(call, "particles");
                    let mut carries = [
                        array(at(p, "carry0")).iter().map(bits).collect::<Vec<_>>(),
                        array(at(p, "carry1")).iter().map(bits).collect::<Vec<_>>(),
                    ];
                    let first = word(at(call, "firstRecord")) as usize;
                    let last = subcalls.get(index + 1).map_or(native_records.len(), |c| word(at(c, "firstRecord")) as usize);
                    let mut cursor = first;
                    for (slot, law) in laws.iter().enumerate() {
                        for i in start..end {
                            let local = i - start;
                            let particle = EventParticle {
                                seed: word(lane(p, "seed", local)),
                                age_percent: bits(lane(p, "age", local)),
                                inverse_lifetime: bits(lane(p, "inv", local)),
                                position: ["px", "py", "pz"].map(|k| bits(lane(p, k, local))),
                                velocity: [("vx", "ax"), ("vy", "ay"), ("vz", "az")]
                                    .map(|(v, a)| bits(lane(p, v, local)) + bits(lane(p, a, local))),
                            };
                            let mut carry = carries[slot][local];
                            let Some(record) = law.record(&particle, &mut carry, dt4[local & 3], &owner).unwrap() else {
                                continue;
                            };
                            carries[slot][local] = carry;
                            records += 1;
                            let Some(native) = native_records.get(cursor).filter(|_| cursor < last) else {
                                mismatched += 1;
                                continue;
                            };
                            cursor += 1;
                            let words = |v: &Value| array(v).iter().map(word).collect::<Vec<u32>>();
                            let state = |e: &EventEmission| {
                                let d = e.distribution;
                                let mut out = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
                                out.extend(e.random.words);
                                out
                            };
                            let times = vec![record.interval.previous.to_bits(), record.interval.current.to_bits(),
                                record.interval.previous_normalized.to_bits(), record.interval.current_normalized.to_bits(),
                                law.duration().to_bits()];
                            let mut same = word(at(native, "index")) as usize == i
                                && word(at(native, "slot")) as usize == slot
                                && words(at(native, "timesBits")) == times
                                && words(at(native, "stateBefore")) == state(&record.before)
                                && words(at(native, "stateAfter")) == state(&record.after);
                            let ids = words(at(native, "commandIds"));
                            match record.commands {
                                None => same &= ids.is_empty(),
                                Some(pair) if ids.len() == 2 => {
                                    for (command, id) in pair.iter().zip(&ids) {
                                        commands += 1;
                                        let raw = hex(at(&native_commands[*id as usize], "rawHex"));
                                        let ours = command.to_bytes();
                                        // The pointer word and the word after the inherited block are not fields.
                                        same &= raw[8..0x54] == ours[8..0x54] && raw[0x58..] == ours[0x58..]
                                            && hex(at(&native_commands[*id as usize], "emissionHex")) == command.emission_bytes();
                                    }
                                }
                                Some(_) => same = false,
                            }
                            if !same {
                                mismatched += 1;
                            }
                        }
                    }
                    if cursor != last {
                        mismatched += last - cursor;
                    }
                }
            }
            super::set_no_repeats_arm(false);
            (calls, newborn, records, commands, mismatched)
        };
        verdict(&run);
    }

    /// The recording from the native window on, against the sub-emitter
    /// receipt's birth records: that receipt stores each record's window,
    /// state before and after, the parent particle it read and the command
    /// bytes, but not the particle's age or the call's time vector, so the
    /// window is taken from the record. This is the replay that carries the
    /// persistent carry (children with a nonzero rate or distance rate).
    /// Records of children outside the law (curve rates, several bursts,
    /// repeating bursts, start delays) are counted per configuration and
    /// skipped; each of those configurations is one the law refuses.
    #[test]
    #[ignore = "MOLY_SUBEMITTER_EVENT_RECEIPT and MOLY_SUBEMITTER_EVENT_CENSUS must identify the current native sub-emitter receipt and its edge census"]
    fn replays_current_native_birth_events_from_the_recorded_window() {
        use crate::particle::emit::Burst;
        use crate::particle::json::{parse, Value};
        let read = |key: &str| parse(&std::fs::read(std::env::var_os(key).expect(key)).unwrap()).unwrap();
        let receipt = read("MOLY_SUBEMITTER_EVENT_RECEIPT");
        // The receipt names each edge configuration; the census it was built from holds the child blocks.
        let census = read("MOLY_SUBEMITTER_EVENT_CENSUS");
        assert_eq!(
            receipt.get("librarySha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        fn word(v: &Value) -> u32 {
            v.as_f64().expect("native word") as u32
        }
        fn bits(v: &Value) -> f32 {
            f32::from_bits(word(v))
        }
        fn at<'a>(v: &'a Value, k: &str) -> &'a Value {
            v.get(k).unwrap_or_else(|| panic!("missing {k}"))
        }
        fn array(v: &Value) -> &[Value] {
            v.as_array().expect("array")
        }
        fn number(v: &Value) -> f32 {
            v.as_f64().expect("number") as f32
        }
        let hex = |v: &Value| -> Vec<u8> {
            let text = v.as_str().expect("hex");
            (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
        };
        // Census curves are authored JSON: constant / twoConstants carry their values, other modes are
        // outside the law and map to a curve the gate refuses.
        let curve = |c: &Value| -> MinMaxCurve {
            match at(c, "mode").as_str() {
                Some("constant") => MinMaxCurve::Constant(number(at(c, "value"))),
                Some("twoConstants") => MinMaxCurve::TwoConstants { min: number(at(c, "min")), max: number(at(c, "max")) },
                _ => MinMaxCurve::Curve { multiplier: 1.0, max: crate::particle::value::Curve { multiplier: 1.0, keys: Vec::new(), pre_wrap: None, post_wrap: None } },
            }
        };
        let groups = array(at(&census, "groups"));
        let named = array(at(at(&receipt, "census"), "groups"));
        assert_eq!(groups.len(), named.len());
        for (group, name) in groups.iter().zip(named) {
            assert_eq!(word(at(group, "configId")), word(at(name, "configId")));
            assert_eq!(word(at(group, "count")), word(at(name, "count")));
        }
        let law_of = |config: usize, cached: usize, arm: Option<&str>| -> Result<BirthEdgeLaw, Refused> {
            let key = at(&groups[config], "key");
            assert_eq!(word(at(&groups[config], "configId")) as usize, config);
            let schedule = at(key, "schedule");
            let emission = at(schedule, "emission");
            let bursts = array(at(emission, "bursts")).iter().map(|b| {
                let mut count = curve(at(b, "count"));
                if let (Some(which), MinMaxCurve::TwoConstants { min, max }) = (arm, count.clone()) {
                    let (low, high) = if max < min { (max, min) } else { (min, max) };
                    if which == "burstLow" { count = MinMaxCurve::Constant(low); }
                    if which == "burstHigh" { count = MinMaxCurve::Constant(high); }
                }
                Burst {
                    time: number(at(b, "time")),
                    count,
                    cycles: BurstCycles::from_serialized(word(at(b, "cycleCount"))),
                    repeat_interval: number(at(b, "repeatInterval")),
                    probability: number(at(b, "probability")),
                }
            }).collect();
            let params = SubEmitterParams {
                emitter: Some("resolved-child".into()),
                source_pointer: Default::default(),
                trigger: SubEmitterTrigger::Birth,
                properties: word(at(key, "properties")),
                probability: number(at(key, "probability")),
            };
            BirthEdgeLaw::from_params(&params, cached, &curve(at(schedule, "startDelay")),
                number(at(schedule, "duration")), at(schedule, "looping").as_bool().unwrap(),
                &EmissionParams {
                    rate_over_time: curve(at(emission, "rateOverTime")),
                    rate_over_distance: curve(at(emission, "rateOverDistance")),
                    bursts,
                })
        };
        // Configurations whose bursts repeat (a cycle count other than one).
        let repeating = |config: usize| array(at(at(at(at(&groups[config], "key"), "schedule"), "emission"), "bursts"))
            .iter().any(|b| word(at(b, "cycleCount")) != 1);
        // (records compared, mismatched, records per refused configuration)
        let run = |arm: Option<&str>| -> (usize, usize, std::collections::BTreeMap<usize, usize>) {
            let (mut compared, mut mismatched) = (0, 0);
            let mut refused = std::collections::BTreeMap::new();
            super::set_no_repeats_arm(arm == Some("noRepeats"));
            for parent in array(at(at(&receipt, "birthEventRecording"), "byParent")) {
                let configs: Vec<usize> = array(at(parent, "edgeConfigIds")).iter().map(|v| word(v) as usize).collect();
                let cached = word(at(parent, "cachedBirthEdges")) as usize;
                let owner_bits = array(at(parent, "ownerBits"));
                for record in array(at(parent, "records")) {
                    let slot = word(at(record, "slot")) as usize;
                    let config = configs[slot];
                    let law = match law_of(config, cached, arm) {
                        Ok(law) => law,
                        Err(_) => {
                            *refused.entry(config).or_insert(0) += 1;
                            continue;
                        }
                    };
                    let times: Vec<f32> = array(at(record, "timesBits")).iter().map(bits).collect();
                    assert_eq!(times[4].to_bits(), law.duration().to_bits());
                    let commands = array(at(record, "commands"));
                    // The accumulated time at the call is not stored; every command carries it.
                    let accumulated = commands.first().map_or(0.0, |c| {
                        let raw = hex(at(c, "rawHex"));
                        f32::from_le_bytes(raw[0x74..0x78].try_into().unwrap())
                    });
                    let owner = EventOwner {
                        local_to_world: std::array::from_fn(|i| bits(&owner_bits[i])),
                        world_space: word(at(parent, "simulation")) == 1,
                        accumulated_time: accumulated,
                        emission_word: word(at(parent, "ownerSeed")),
                    };
                    let persistent = array(at(record, "parentPersistentVelBits"));
                    let animated = array(at(record, "parentAnimatedVelBits"));
                    let particle = EventParticle {
                        seed: word(at(record, "parentSeed")),
                        age_percent: 0.0,
                        inverse_lifetime: 1.0,
                        position: std::array::from_fn(|a| bits(&array(at(record, "parentPositionBits"))[a])),
                        velocity: std::array::from_fn(|a| bits(&persistent[a]) + bits(&animated[a])),
                    };
                    let before: Vec<u32> = array(at(record, "stateBefore")).iter().map(word).collect();
                    let mut carry = if arm == Some("noCarry") { 0.0 } else { f32::from_bits(before[1]) };
                    let interval = BirthInterval {
                        previous: times[0],
                        current: times[1],
                        previous_normalized: times[2],
                        current_normalized: times[3],
                    };
                    let ours = law.record_window(&particle, &mut carry, interval, &owner).unwrap();
                    compared += 1;
                    let state = |e: &EventEmission| {
                        let d = e.distribution;
                        let mut out = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
                        out.extend(e.random.words);
                        out
                    };
                    let after: Vec<u32> = array(at(record, "stateAfter")).iter().map(word).collect();
                    let mut same = state(&ours.before) == before && state(&ours.after) == after;
                    match ours.commands {
                        None => same &= commands.is_empty(),
                        Some(pair) if commands.len() == 2 => {
                            for (command, native) in pair.iter().zip(commands) {
                                let raw = hex(at(native, "rawHex"));
                                let bytes = command.to_bytes();
                                same &= raw[8..0x54] == bytes[8..0x54] && raw[0x58..] == bytes[0x58..]
                                    && hex(at(native, "emissionHex")) == command.emission_bytes();
                            }
                        }
                        Some(_) => same = false,
                    }
                    if !same {
                        mismatched += 1;
                    }
                }
            }
            super::set_no_repeats_arm(false);
            (compared, mismatched, refused)
        };
        let (compared, mismatched, refused) = run(None);
        let repeat_records: usize = array(at(at(&receipt, "birthEventRecording"), "byParent")).iter().map(|parent| {
            let configs: Vec<usize> = array(at(parent, "edgeConfigIds")).iter().map(|v| word(v) as usize).collect();
            array(at(parent, "records")).iter()
                .filter(|record| repeating(configs[word(at(record, "slot")) as usize]))
                .filter(|record| !refused.contains_key(&configs[word(at(record, "slot")) as usize]))
                .count()
        }).sum();
        println!("birth events from the recorded window: {compared} records compared ({repeat_records} of children with repeating bursts), {mismatched} mismatched, refused configurations {refused:?}");
        assert_eq!(mismatched, 0);
        assert!(compared > 0);
        // Only configurations outside the law may be refused: curve rates (5, 8), and the edges of the
        // one parent with three cached birth edges (7, 8, 9), whose third carry is not transcribed.
        assert!(refused.keys().all(|config| [5, 7, 8, 9].contains(config)), "{refused:?}");
        assert!(repeat_records > 0);
        let wrong = run(Some("noRepeats")).1;
        println!("arm noRepeats: {wrong} records differ");
        // The burst arms have no signal here (no recorded window holds a burst time); the
        // per-call replay above carries them.
        for arm in ["noCarry"] {
            let wrong = run(Some(arm)).1;
            println!("arm {arm}: {wrong} records differ");
            assert!(wrong > 0, "{arm} arm matched every native record");
        }
    }
}
