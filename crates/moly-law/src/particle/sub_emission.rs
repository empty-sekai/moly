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
    UnverifiedCatchUpRange,
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
        if ![age, inverse_lifetime, dt].iter().all(|v| v.is_finite())
            || inverse_lifetime <= 0.0
            || dt < 0.0
        {
            return Err(Refused::InvalidInput);
        }
        let current_normalized = age * 0.01 - inverse_lifetime * self.delay;
        let current = current_normalized / inverse_lifetime;
        if !(0.0..1.0).contains(&current_normalized) || current >= self.duration {
            return Ok(None);
        }
        let previous_normalized =
            (age + inverse_lifetime * (dt * -100.0)) * 0.01 - inverse_lifetime * self.delay;
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

/// Explicit, bounded native full-frame catch-up iterator. The caller decides
/// `enabled` from a proven update owner; this type never guesses flag meaning.
/// Each yielded dt must run pre/sim/post only on the new birth span. The
/// remainder is not integrated, and normal pool packing follows the batch.
#[derive(Debug, Clone, Copy)]
pub struct CatchUp {
    remaining: f32,
    frame_dt: f32,
    enabled: bool,
}

impl CatchUp {
    pub fn new(elapsed: f32, frame_dt: f32, enabled: bool) -> Result<Self, Refused> {
        if !elapsed.is_finite() || elapsed < 0.0 || !frame_dt.is_finite() || frame_dt <= 0.0 {
            return Err(Refused::InvalidInput);
        }
        // Native changes its threshold above 5/10 seconds. The receipt for
        // this first implementation reaches one second; do not silently
        // extend its accepted domain to the larger untested interval.
        if elapsed > 1.0 {
            return Err(Refused::UnverifiedCatchUpRange);
        }
        Ok(Self {
            remaining: elapsed,
            frame_dt,
            enabled: enabled && frame_dt > MIN_AMOUNT,
        })
    }
    pub fn remainder(&self) -> f32 {
        self.remaining
    }
}

impl Iterator for CatchUp {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.enabled || self.remaining < self.frame_dt {
            return None;
        }
        self.remaining -= self.frame_dt;
        Some(self.frame_dt)
    }
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
    fn catch_up_runs_complete_steps_without_eating_fractional_remainder() {
        let mut steps = CatchUp::new(0.4375, 0.125, true).unwrap();
        assert_eq!(steps.by_ref().collect::<Vec<_>>(), vec![0.125; 3]);
        assert_eq!(steps.remainder(), 0.0625);
        assert_eq!(CatchUp::new(0.4375, 0.125, false).unwrap().count(), 0);
        assert!(matches!(
            CatchUp::new(6.0, 0.125, true),
            Err(Refused::UnverifiedCatchUpRange)
        ));
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
            let expected_steps = CatchUp::new(
                field("catchup"),
                field("frame_dt"),
                (field("flags") as u32 & 5) != 0,
            )
            .unwrap()
            .count();
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
}
