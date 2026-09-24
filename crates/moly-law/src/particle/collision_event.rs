//! Supplied-hit collision response -> RecordEmit input -> two child commands.
//!
//! Qualified by a current-native supplied-hit collision-to-child receipt.
//! No scene queries, pool compaction, child births, recursive scheduling, live
//! instance enumeration or source admission are implemented here. Hit indices
//! refer to one stable supplied slice; they are not persistent particle IDs.

use crate::particle::collision_response::{
    CollisionResponse, FrameVelocity, Refusal as ResponseRefusal, SuppliedHit,
};
use crate::particle::emit::BurstCycles;
use crate::particle::schema::{
    CollisionParams, SimulationSpace, SubEmitterParams, SubEmitterSourcePointer, SubEmitterTrigger,
};
use crate::particle::sub_emission::{BirthBatch, BirthDistribution};
use crate::particle::{EmitterParams, MinMaxCurve, Particle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    UnqualifiedConfiguration,
    UnqualifiedOwner,
    InvalidParent,
    Response(ResponseRefusal),
}

/// Only the current one-cached-edge, neutral, zero-rate, count-one collision
/// burst configuration has a complete event-to-command receipt. A general
/// constant burst, nonzero time/distance rate, probability or inheritance law
/// must first gain its own native event-path replay.
#[derive(Clone, Copy, Debug)]
pub struct CollisionBurstSchedule {
    response: CollisionResponse,
    burst_count: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct CollisionCommand {
    pub batch: BirthBatch,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// Native neutral inheritance, including current Math-initialized forward Z.
    /// Last word is the parent particle seed; it is never a child RNG reset.
    pub inherited_words: [u32; 13],
    /// [dt, previous, current, catch_up] consumed by child Emit.
    pub timing: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct CollisionEvent {
    pub hit_ordinal: usize,
    pub particle_index: usize,
    pub trigger: u32,
    pub child_index: usize,
    pub times: [f32; 5],
    pub parent_position: [f32; 3],
    pub persistent_velocity: [f32; 3],
    pub animated_velocity: [f32; 3],
    /// The explicit empty distance command precedes the burst command.
    pub commands: [CollisionCommand; 2],
}

impl CollisionBurstSchedule {
    pub fn from_source(
        parent_collision: &CollisionParams,
        edge: &SubEmitterParams,
        target: &EmitterParams,
        target_system_path_id: &str,
        cached_collision_edges: usize,
    ) -> Result<Self, Refused> {
        let constant = |curve: &MinMaxCurve, value: f32| matches!(curve, MinMaxCurve::Constant(v) if v.to_bits() == value.to_bits());
        let Some(emission) = target.emission.as_ref() else {
            return Err(Refused::UnqualifiedConfiguration);
        };
        let pointer_matches = matches!(&edge.source_pointer,
            SubEmitterSourcePointer::Pointer { file_id: 0, path_id }
                if path_id != "0" && path_id == target_system_path_id);
        if edge.trigger != SubEmitterTrigger::Collision
            || edge.properties != 0
            || edge.probability != 1.0
            || edge.emitter.as_deref() != Some(target.node.as_str())
            || !pointer_matches
            || cached_collision_edges != 1
            || target.simulation_space != SimulationSpace::World
            || target.simulation_speed != 1.0
            || target.duration != 1.0
            || target.looping
            || !constant(&target.start_delay, 0.0)
            || !constant(&emission.rate_over_time, 0.0)
            || !constant(&emission.rate_over_distance, 0.0)
            || emission.bursts.len() != 1
            || !constant(&parent_collision.bounce, 1.0)
            || !constant(&parent_collision.dampen, 1.0)
            || !constant(&parent_collision.lifetime_loss, 0.0)
            || parent_collision.min_kill_speed != 0.0
            || parent_collision.max_kill_speed != 1000.0
        {
            return Err(Refused::UnqualifiedConfiguration);
        }
        let burst = &emission.bursts[0];
        if !constant(&burst.count, 1.0)
            || burst.time != 0.0
            || burst.probability != 1.0
            || !matches!(burst.cycles, BurstCycles::Finite(n) if n.get() == 1)
        {
            return Err(Refused::UnqualifiedConfiguration);
        }
        Ok(Self {
            response: CollisionResponse::from_params(parent_collision)
                .map_err(Refused::Response)?,
            burst_count: 1,
        })
    }

    pub fn apply_supplied_hits(
        &self,
        particles: &mut [Particle],
        frames: &[FrameVelocity],
        parent_seeds: &[u32],
        hits: &[SuppliedHit],
        parent_owner: [f32; 16],
    ) -> Result<Vec<CollisionEvent>, Refused> {
        const IDENTITY: [f32; 16] = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        if parent_owner.map(f32::to_bits) != IDENTITY.map(f32::to_bits) {
            return Err(Refused::UnqualifiedOwner);
        }
        if particles.len() != frames.len()
            || particles.len() != parent_seeds.len()
            || frames.iter().any(|frame| frame.modifier != 1.0)
            || particles.iter().any(|particle| {
                !particle.inverse_lifetime.is_finite()
                    || particle.inverse_lifetime <= 0.0
                    || !particle.age_percent.is_finite()
                    || particle.age_percent < 0.0
            })
        {
            return Err(Refused::InvalidParent);
        }
        // Transactional scratch keeps a later invalid hit from committing a
        // partial response or leaking events. Never compact or reorder indices.
        let mut staged = particles.to_vec();
        let mut events = Vec::new();
        for (ordinal, hit) in hits.iter().enumerate().rev() {
            let before_age = staged
                .get(hit.particle_index)
                .ok_or(Refused::Response(ResponseRefusal::InvalidHitIndex))?
                .age_percent;
            self.response
                .apply_hits(&mut staged, frames, std::slice::from_ref(hit))
                .map_err(Refused::Response)?;
            if staged[hit.particle_index]
                .position
                .iter()
                .chain(staged[hit.particle_index].velocity.iter())
                .any(|v| !v.is_finite())
            {
                return Err(Refused::InvalidParent);
            }
            // Native tests age BEFORE this response for event eligibility.
            // Already dead skips only RecordEmit; response writes still occur.
            // Equality and a new speed kill still produce an event. Nonzero
            // lifetime-loss composition is outside this qualified subset.
            if before_age > 100.0 {
                continue;
            }
            let parent = &staged[hit.particle_index];
            let frame = frames[hit.particle_index];
            let normalized = (parent.age_percent * 0.01_f32).max(0.0).min(1.0);
            let seconds = normalized / parent.inverse_lifetime;
            if !seconds.is_finite() {
                return Err(Refused::InvalidParent);
            }
            let times = [0.0, seconds, normalized, normalized, 0.0];
            let velocity = std::array::from_fn(|axis| parent.velocity[axis] + frame.animated[axis]);
            let inherited_words = [
                u32::MAX,
                1.0_f32.to_bits(),
                1.0_f32.to_bits(),
                1.0_f32.to_bits(),
                0,
                0,
                0,
                0,
                0,
                1.0_f32.to_bits(),
                1.0_f32.to_bits(),
                f32::INFINITY.to_bits(),
                parent_seeds[hit.particle_index],
            ];
            let command = |count| CollisionCommand {
                batch: BirthBatch {
                    count,
                    rate_count: 0,
                    distribution: BirthDistribution {
                        spacing: 0.0,
                        offset: 0.0,
                        burst_fraction: 0.0,
                    },
                },
                position: parent.position,
                velocity,
                inherited_words,
                timing: [seconds, normalized, normalized, 0.0],
            };
            events.push(CollisionEvent {
                hit_ordinal: ordinal,
                particle_index: hit.particle_index,
                trigger: 1,
                child_index: 0,
                times,
                parent_position: parent.position,
                persistent_velocity: parent.velocity,
                animated_velocity: frame.animated,
                commands: [command(0), command(self.burst_count)],
            });
        }
        particles.copy_from_slice(&staged);
        Ok(events)
    }
}
