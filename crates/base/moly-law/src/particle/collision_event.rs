//! RecordEmit for the Collision trigger: the event one hit sends to one
//! collision sub-emitter edge, and the two child commands it issues.
//!
//! The module update ([`crate::particle::collision_query::CollisionLaw::update`])
//! calls [`record_emit`] after a hit's response, for every collision edge,
//! when the particle's age before the lifetime loss was not above 100 percent.
//! Child births, recursive scheduling and the child-side gates are not here.

use crate::particle::armf as a;
use crate::particle::collision_response::QueryAffine;
use crate::particle::schema::{SubEmitterParams, SubEmitterTrigger};
use crate::particle::seed_owner::ScalarRandom;
use crate::particle::sub_emission::{neutral_inherited, BirthDistribution, SubEmitterCommand};
use crate::particle::{EmitterParams, MinMaxCurve};

/// The probability draws scale a 23-bit word by this.
const RANDOM_SCALE: f32 = f32::from_bits(0x3400_0001);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a Collision edge.
    Trigger,
    /// Inherited properties are not transcribed for this trigger.
    Properties,
    /// A probability outside [0, 1] or not finite.
    Probability,
    /// The child's first burst count is not a finite, non-negative constant.
    BurstCount,
    /// The child has no emission block to read bursts from.
    MissingEmission,
}

/// The child's first burst as the event reads it: its probability and its
/// constant or two-constant count. The burst time, cycles and interval are
/// not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeBurst {
    probability: f32,
    count: BurstCount,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BurstCount {
    Constant(f32),
    /// The smaller and larger constant, each truncated toward zero. The gate
    /// keeps 0 <= lo <= hi <= 16,777,215, so the modulus below is never zero
    /// (the engine's unsigned divide by zero would give a quotient of 0).
    TwoConstants { lo: i32, hi: i32 },
}

/// The largest count the two-constant gate admits (2^24 - 1).
const MAX_TWO_CONSTANT_COUNT: f32 = 16_777_215.0;

impl EdgeBurst {
    pub fn new(probability: f32, count: f32) -> Result<Self, Refused> {
        if !(0.0..=1.0).contains(&probability) {
            return Err(Refused::Probability);
        }
        if !count.is_finite() || count < 0.0 {
            return Err(Refused::BurstCount);
        }
        Ok(Self { probability, count: BurstCount::Constant(count) })
    }

    /// A two-constant count. The smaller and the larger are taken by the
    /// same strict comparisons the burst accumulation uses; negative or
    /// non-finite ends, and a larger end past 2^24 - 1, are refused.
    pub fn two_constants(probability: f32, min: f32, max: f32) -> Result<Self, Refused> {
        if !(0.0..=1.0).contains(&probability) {
            return Err(Refused::Probability);
        }
        if !min.is_finite() || !max.is_finite() {
            return Err(Refused::BurstCount);
        }
        let low = if max < min { max } else { min };
        let high = if min < max { max } else { min };
        if low < 0.0 || high > MAX_TWO_CONSTANT_COUNT {
            return Err(Refused::BurstCount);
        }
        Ok(Self {
            probability,
            count: BurstCount::TwoConstants {
                lo: a::to_i32_toward_zero(low),
                hi: a::to_i32_toward_zero(high),
            },
        })
    }

    /// The burst count: none at probability 0; below 1 one draw on the
    /// event's own words decides; then the constant count truncated, or for
    /// two constants one more draw w giving lo + w mod (hi + 1 - lo).
    fn count(self, random: &mut ScalarRandom) -> i32 {
        if self.probability == 0.0 {
            return 0;
        }
        if self.probability < 1.0 {
            let word = random.next_u32();
            if self.probability <= a::mul((word & 0x7f_ffff) as f32, RANDOM_SCALE) {
                return 0;
            }
        }
        match self.count {
            BurstCount::Constant(count) => a::to_i32_toward_zero(count),
            BurstCount::TwoConstants { lo, hi } => {
                let word = random.next_u32();
                // 1 <= range <= 2^24 by the gate.
                let range = (hi - lo + 1) as u32;
                (word % range) as i32 + lo
            }
        }
    }

    /// The burst count of an event, drawing on the event's own words.
    pub(crate) fn draw(self, random: &mut ScalarRandom) -> i32 {
        self.count(random)
    }
}

/// The edge probability gate of RecordEmit: none at probability 0; else a
/// hash of the particle seed (not a draw on the event's words) scaled to
/// [0, 1) must not exceed the probability.
pub(crate) fn edge_emits(seed: u32, probability: f32) -> bool {
    if probability == 0.0 {
        return false;
    }
    let w8 = seed.wrapping_add(0x5aa4_7f98);
    let w9 = w8.wrapping_mul(0x6ab5_1b9d).wrapping_add(0x714a_cb3f);
    let t = w8 ^ (w8 << 11);
    let r = ((w9 ^ t ^ (t >> 8)) & 0x7f_ffff) ^ (w9 >> 19);
    !(a::mul(r as f32, RANDOM_SCALE) > probability)
}

/// One collision sub-emitter edge as RecordEmit reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionEmitEdge {
    probability: f32,
    burst: Option<EdgeBurst>,
}

impl CollisionEmitEdge {
    /// `first_burst` is the child's first burst; `None` when it has none (the
    /// event then issues no command).
    pub fn new(probability: f32, first_burst: Option<EdgeBurst>) -> Result<Self, Refused> {
        if !(0.0..=1.0).contains(&probability) {
            return Err(Refused::Probability);
        }
        Ok(Self { probability, burst: first_burst })
    }

    /// The edge and its child from the export: a Collision edge that
    /// inherits nothing, with the child's first burst count constant.
    pub fn from_source(edge: &SubEmitterParams, target: &EmitterParams) -> Result<Self, Refused> {
        if edge.trigger != SubEmitterTrigger::Collision {
            return Err(Refused::Trigger);
        }
        if edge.properties != 0 {
            return Err(Refused::Properties);
        }
        let emission = target.emission.as_ref().ok_or(Refused::MissingEmission)?;
        let burst = match emission.bursts.first() {
            None => None,
            Some(burst) => match burst.count {
                MinMaxCurve::Constant(count) => Some(EdgeBurst::new(burst.probability, count)?),
                _ => return Err(Refused::BurstCount),
            },
        };
        Self::new(edge.probability, burst)
    }
}

/// The parent particle after its hit's response, in simulation coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventParent {
    pub index: usize,
    pub seed: u32,
    pub position: [f32; 3],
    /// Persistent velocity after the response.
    pub velocity: [f32; 3],
    pub animated: [f32; 3],
    /// The written age's normalized form and its seconds (normalized over the
    /// inverse lifetime).
    pub normalized_age: f32,
    pub seconds: f32,
}

/// One RecordEmit call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedEmit {
    pub particle: usize,
    pub edge: usize,
    /// The emission state the event starts from: three zero scalars and the
    /// four words seeded from the particle seed plus the parent's emission
    /// state word.
    pub state_words: [u32; 7],
    /// The event times `[0, seconds, normalized, normalized, 0]`.
    pub times: [f32; 5],
    /// `None` when the event returned before the burst count (probability,
    /// or a child without bursts).
    pub burst_count: Option<i32>,
    /// The empty command (count 0, a copy of the starting state) and the
    /// burst command (the live state after the count draw), when the count
    /// is not zero.
    pub commands: Option<[(SubEmitterCommand, [u32; 7]); 2]>,
}

/// RecordEmit, trigger Collision. `emission_word` is the parent's emission
/// state word the event seed adds to the particle seed; `pending` is the
/// parent's system time still to simulate (the command's catch-up word);
/// `owner` is the parent's local-to-world owner for a Local system (`None` in
/// World space). All seed arithmetic wraps in 32 bits.
pub fn record_emit(
    edge: &CollisionEmitEdge,
    edge_index: usize,
    parent: &EventParent,
    emission_word: u32,
    pending: f32,
    owner: Option<QueryAffine>,
) -> RecordedEmit {
    let mut random = ScalarRandom::from_seed(parent.seed.wrapping_add(emission_word));
    let start = random.words;
    let state_words = [0, 0, 0, start[0], start[1], start[2], start[3]];
    let normalized = parent.normalized_age;
    let mut recorded = RecordedEmit {
        particle: parent.index,
        edge: edge_index,
        state_words,
        times: [0.0, parent.seconds, normalized, normalized, 0.0],
        burst_count: None,
        commands: None,
    };
    if !edge_emits(parent.seed, edge.probability) {
        return recorded;
    }
    let mut position = parent.position;
    // The persistent plus animated velocity, without the speed modifier.
    let mut velocity: [f32; 3] = std::array::from_fn(|k| a::add(parent.velocity[k], parent.animated[k]));
    if let Some(owner) = owner {
        position = owner.point_event(position);
        velocity = owner.vector_event(velocity);
    }
    let dt = a::sub(parent.seconds, 0.0);
    let Some(burst) = edge.burst else {
        return recorded;
    };
    let count = burst.count(&mut random);
    recorded.burst_count = Some(count);
    if count == 0 {
        return recorded;
    }
    let command = |count: i32| SubEmitterCommand {
        position,
        velocity,
        inherited: neutral_inherited(parent.seed),
        count: count as u32 as u64,
        rate_count: 0,
        dt,
        previous_normalized: normalized,
        current_normalized: normalized,
        catch_up: pending,
        emission: BirthDistribution { spacing: 0.0, offset: 0.0, burst_fraction: 0.0 },
    };
    let live = random.words;
    recorded.commands = Some([
        (command(0), state_words),
        (command(count), [0, 0, 0, live[0], live[1], live[2], live[3]]),
    ]);
    recorded
}
