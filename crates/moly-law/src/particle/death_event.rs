//! RecordEmit for the Death trigger: the event a dying particle sends to one
//! death sub-emitter edge, and the two child commands it issues.
//!
//! The engine records it from KillParticle, once per cached death edge in slot
//! order, before the dead slot is overwritten by the pool's last particle. The
//! parent calls KillParticle at two points: the kill pass of the existing
//! particles inside their simulation (lanes 3 to 0 of each four-lane group,
//! the group retested after any kill; always recording), and the newborn kill
//! pass after the newborn groups' modules (forward over the rounded newborn
//! storage, the swapped slot retested, recording only while the accepted
//! count it decrements is still nonzero). The ring buffer's replaced particles
//! record the same event through RecordParticleDeath.
//!
//! The event reads no age: its times are 0, 0, 1, 1, 0. It starts from three
//! zero scalars and four words seeded from the particle seed plus the parent's
//! emission word; the edge probability hash, then the child's first burst (its
//! own probability draw and its constant or two-constant count) are all it
//! draws. The child's rates, its other bursts, its start delay and its
//! duration are not read. Child births, the child's own gates and the order of
//! the child's own update are not here.

use crate::particle::armf as a;
use crate::particle::collision_event::{edge_emits, EdgeBurst};
use crate::particle::schema::{SubEmitterParams, SubEmitterTrigger};
use crate::particle::seed_owner::ScalarRandom;
use crate::particle::sub_emission::{neutral_inherited, BirthDistribution, EventOwner, SubEmitterCommand};
use crate::particle::{EmitterParams, MinMaxCurve};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a Death edge.
    Trigger,
    /// Inherited properties are not transcribed for this trigger (the
    /// inherited size meets the parent's size modules, which no replay covers).
    Properties,
    /// A probability outside [0, 1] or not finite.
    Probability,
    /// The child's first burst count is neither a non-negative constant nor
    /// two constants inside the gate.
    BurstCount,
    /// The child has no emission block to read bursts from.
    MissingEmission,
}

impl From<crate::particle::collision_event::Refused> for Refused {
    fn from(refused: crate::particle::collision_event::Refused) -> Self {
        use crate::particle::collision_event::Refused as R;
        match refused {
            R::Trigger => Self::Trigger,
            R::Properties => Self::Properties,
            R::Probability => Self::Probability,
            R::BurstCount => Self::BurstCount,
            R::MissingEmission => Self::MissingEmission,
        }
    }
}

/// The event times RecordEmit receives from KillParticle and
/// RecordParticleDeath.
pub const DEATH_TIMES: [f32; 5] = [0.0, 0.0, 1.0, 1.0, 0.0];

/// One death sub-emitter edge as RecordEmit reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathEmitEdge {
    probability: f32,
    burst: Option<EdgeBurst>,
}

impl DeathEmitEdge {
    /// `first_burst` is the child's first burst; `None` when it has none (the
    /// event then issues no command and draws nothing past the edge gate).
    pub fn new(probability: f32, first_burst: Option<EdgeBurst>) -> Result<Self, Refused> {
        if !(0.0..=1.0).contains(&probability) {
            return Err(Refused::Probability);
        }
        Ok(Self { probability, burst: first_burst })
    }

    /// The edge and its child from the export: a Death edge that inherits
    /// nothing, with the child's first burst count a constant or two
    /// constants.
    pub fn from_source(edge: &SubEmitterParams, target: &EmitterParams) -> Result<Self, Refused> {
        if edge.trigger != SubEmitterTrigger::Death {
            return Err(Refused::Trigger);
        }
        if edge.properties != 0 {
            return Err(Refused::Properties);
        }
        let emission = target.emission.as_ref().ok_or(Refused::MissingEmission)?;
        let burst = match emission.bursts.first() {
            None => None,
            Some(burst) => Some(match burst.count {
                MinMaxCurve::Constant(count) => EdgeBurst::new(burst.probability, count)?,
                MinMaxCurve::TwoConstants { min, max } => EdgeBurst::two_constants(burst.probability, min, max)?,
                _ => return Err(Refused::BurstCount),
            }),
        };
        Self::new(edge.probability, burst)
    }
}

/// The dying particle as KillParticle hands it to RecordEmit, in simulation
/// coordinates (source axes): the slot it dies in, its seed, its position and
/// its persistent and animated velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathParent {
    pub index: usize,
    pub seed: u32,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub animated: [f32; 3],
}

/// One RecordEmit call of the Death trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedDeath {
    pub particle: usize,
    pub edge: usize,
    /// The event's emission state before and after: three scalars (all zero,
    /// and never written) and the four random words.
    pub state_before: [u32; 7],
    pub state_after: [u32; 7],
    /// `None` when the event returned before the burst count (the edge
    /// probability, or a child without bursts).
    pub burst_count: Option<i32>,
    /// The empty command (count 0) and the burst command, when the count is
    /// not zero.
    pub commands: Option<[SubEmitterCommand; 2]>,
}

/// RecordEmit, trigger Death. `owner` is the parent as the call reads it: its
/// local-to-world matrix (not read in World space), its space, its system
/// time still to simulate (the commands' catch-up word) and its emission
/// word, which the event seed adds to the particle seed. All seed arithmetic
/// wraps in 32 bits.
pub fn record_death(edge: &DeathEmitEdge, edge_index: usize, parent: &DeathParent, owner: &EventOwner) -> RecordedDeath {
    let mut random = ScalarRandom::from_seed(parent.seed.wrapping_add(owner.emission_word));
    let words = |random: &ScalarRandom| {
        let w = random.words;
        [0, 0, 0, w[0], w[1], w[2], w[3]]
    };
    let state_before = words(&random);
    let mut recorded = RecordedDeath {
        particle: parent.index,
        edge: edge_index,
        state_before,
        state_after: state_before,
        burst_count: None,
        commands: None,
    };
    if !edge_emits(parent.seed, edge.probability) {
        return recorded;
    }
    // The persistent plus the animated velocity, one addition per axis; then,
    // outside World space, both through the owner: each axis the three
    // products summed first to last, the translation added after them.
    let velocity: [f32; 3] = std::array::from_fn(|k| a::add(parent.velocity[k], parent.animated[k]));
    let (position, velocity) = if owner.world_space {
        (parent.position, velocity)
    } else {
        let m = &owner.local_to_world;
        let linear = |v: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|k| a::add(a::add(a::mul(m[k], v[0]), a::mul(m[4 + k], v[1])), a::mul(m[8 + k], v[2])))
        };
        let local = linear(parent.position);
        (std::array::from_fn(|k| a::add(m[12 + k], local[k])), linear(velocity))
    };
    let Some(burst) = edge.burst else {
        return recorded;
    };
    let count = burst.draw(&mut random);
    recorded.burst_count = Some(count);
    recorded.state_after = words(&random);
    if count == 0 {
        return recorded;
    }
    let [_, t1, t2, t3, _] = DEATH_TIMES;
    let command = |count: i32| SubEmitterCommand {
        position,
        velocity,
        inherited: neutral_inherited(parent.seed),
        // The gate keeps the count non-negative.
        count: count as u32 as u64,
        rate_count: 0,
        dt: a::sub(t1, DEATH_TIMES[0]),
        previous_normalized: t2,
        current_normalized: t3,
        catch_up: owner.accumulated_time,
        emission: BirthDistribution { spacing: 0.0, offset: 0.0, burst_fraction: 0.0 },
    };
    recorded.commands = Some([command(0), command(count)]);
    recorded
}

#[cfg(test)]
mod tests;
