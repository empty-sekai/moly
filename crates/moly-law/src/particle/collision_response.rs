//! The engine CollisionModule's query-side coordinate helpers and its hit
//! response (World type, 3D mode, High quality).
//!
//! `CollisionResponse::respond` is one hit of the module's reverse hit loop in
//! the engine's arithmetic: every operation a NaN can reach follows the ARM
//! rules, and the associations are the engine's. The whole module update, from
//! the query packs to the child commands, is
//! [`crate::particle::collision_query::CollisionLaw::update`]; `apply_hits`
//! stays an entry for hits already supplied in simulation coordinates.
//!
//! Nothing here queries a scene, emits events or removes particles.

use crate::particle::armf as a;
use crate::particle::collision_query::arms;
use crate::particle::{schema::CollisionParams, MinMaxCurve, Particle};

/// The age a collision kill writes: just above 100 percent, so the next kill
/// pass of the simulation removes the particle.
pub const KILLED_AGE: u32 = 0x42c8_0001;
/// The age percent to normalized age factor (0.01 as the engine's constant).
pub(crate) const AGE_SCALE: f32 = f32::from_bits(0x3c23_d70a);
/// A hit normal whose squared length is not above this becomes +Z.
const NORMAL_LIMIT: f32 = f32::from_bits(0x0da2_4260);

/// Source `WorldCollision` query input after the particle/update state has
/// been sampled.  This is deliberately only the query boundary: it does not
/// run a scene broadphase, PhysX sweep, hit selection, response or event
/// ownership. The downstream sweep has its own travel cutoff; it does not
/// belong to this WorldCollision input builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryInput {
    pub position: [f32; 3],
    pub persistent_velocity: [f32; 3],
    pub animated_velocity: [f32; 3],
    /// The particle's speed modifier, read only when the particle state's
    /// speed-modifier flag is set (`None` otherwise, and then no multiply).
    pub speed_modifier: Option<f32>,
    pub dt: f32,
    /// The one size stream selected by the native particle-state flag.  Do not
    /// merge this with the other size stream: WorldCollision reads one pointer
    /// selected by a particle-state flag (current size or start size).
    pub size: [f32; 3],
    /// False consumes the X component only; true takes `fmax(x, y)` then
    /// `fmax(result, z)`, NaN propagating.
    pub size_is_3d: bool,
    /// Authored CollisionModule radiusScale.  WorldCollision's parameter
    /// setup halves this value before the multiply.
    pub radius_scale: f32,
    /// `true` for World simulation; false uses the supplied owner affine for
    /// both query endpoints.  This keeps Local and Custom source paths
    /// explicit instead of silently treating them as world coordinates.
    pub world_space: bool,
    pub owner: QueryAffine,
}

/// Minimal source-space affine used at the collision boundary. `linear[r][c]`
/// is the engine's column-major word `4c + r` and `translation[r]` the word
/// `12 + r`; it multiplies a column vector. Each helper keeps the association
/// of the engine loop it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryAffine {
    pub linear: [[f32; 3]; 3],
    pub translation: [f32; 3],
}

impl QueryAffine {
    pub const IDENTITY: Self = Self {
        linear: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [0.0; 3],
    };

    /// From the engine's column-major 4x4 words; the bottom row is not read.
    pub fn from_columns(m: &[f32; 16]) -> Self {
        Self {
            linear: std::array::from_fn(|r| std::array::from_fn(|c| m[4 * c + r])),
            translation: [m[12], m[13], m[14]],
        }
    }

    /// A query endpoint into the world: `c0 x + (c1 y + (t + c2 z))`.
    pub fn point_forward(self, v: [f32; 3]) -> [f32; 3] {
        let l = self.linear;
        if arms::on("leftAssociation") {
            let linear = self.vector_event(v);
            return std::array::from_fn(|r| a::add(linear[r], self.translation[r]));
        }
        std::array::from_fn(|r| {
            a::add(a::mul(l[r][0], v[0]),
                a::add(a::mul(l[r][1], v[1]), a::add(self.translation[r], a::mul(l[r][2], v[2]))))
        })
    }

    /// A hit point back into simulation space: `t + (c0 x + (c1 y + c2 z))`.
    pub fn point_inverse(self, v: [f32; 3]) -> [f32; 3] {
        let linear = self.vector_inverse(v);
        std::array::from_fn(|r| a::add(self.translation[r], linear[r]))
    }

    /// A hit normal back into simulation space: `c0 x + (c1 y + c2 z)`.
    pub fn vector_inverse(self, v: [f32; 3]) -> [f32; 3] {
        let l = self.linear;
        std::array::from_fn(|r| {
            a::add(a::mul(l[r][0], v[0]), a::add(a::mul(l[r][1], v[1]), a::mul(l[r][2], v[2])))
        })
    }

    /// An event position into the world: `t + ((c0 x + c1 y) + c2 z)`.
    pub fn point_event(self, v: [f32; 3]) -> [f32; 3] {
        let linear = self.vector_event(v);
        std::array::from_fn(|r| a::add(self.translation[r], linear[r]))
    }

    /// An event velocity into the world: `(c0 x + c1 y) + c2 z`.
    pub fn vector_event(self, v: [f32; 3]) -> [f32; 3] {
        let l = self.linear;
        std::array::from_fn(|r| {
            a::add(a::add(a::mul(l[r][0], v[0]), a::mul(l[r][1], v[1])), a::mul(l[r][2], v[2]))
        })
    }
}

/// One query lane as WorldCollision packs it: the swept endpoints and the
/// radius before the sweep's skin margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub radius: f32,
}

/// Convert a world hit into the simulation coordinates the response reads,
/// with the inverse owner the engine's per-frame owner update stores (this
/// adapter never rebuilds it). The normal is renormalized with the reciprocal
/// square-root estimate and two refinement steps; a squared length not above
/// the engine's threshold (a zero or NaN normal included) becomes +Z. There is
/// no refusal: an infinite squared length gives a NaN normal, as natively.
pub fn world_hit_to_simulation(
    inverse: QueryAffine,
    particle_index: usize,
    point: [f32; 3],
    normal: [f32; 3],
) -> SuppliedHit {
    let v = inverse.vector_inverse(normal);
    let squared = a::add(a::add(a::mul(v[0], v[0]), a::mul(v[1], v[1])), a::mul(v[2], v[2]));
    let normal = if !(squared > NORMAL_LIMIT) {
        [0.0, 0.0, 1.0]
    } else {
        let scale = a::rsqrt2(squared);
        v.map(|x| a::mul(x, scale))
    };
    SuppliedHit { particle_index, point: inverse.point_inverse(point), normal }
}

/// Build one `WorldCollision` query lane. The caller still owns packing,
/// collider filtering, the sweep and the hit response.
pub fn build_query(input: QueryInput) -> CollisionQuery {
    // Velocity sum, the optional speed modifier, then dt, subtracted from the
    // current position; component-wise f32 with the ARM NaN rules.
    let total: [f32; 3] = std::array::from_fn(|axis| {
        let total = a::add(input.persistent_velocity[axis], input.animated_velocity[axis]);
        match input.speed_modifier {
            Some(modifier) => a::mul(total, modifier),
            None => total,
        }
    });
    let start = std::array::from_fn(|axis| a::sub(input.position[axis], a::mul(total[axis], input.dt)));
    let end = input.position;
    let (start, end) = if input.world_space {
        (start, end)
    } else {
        (input.owner.point_forward(start), input.owner.point_forward(end))
    };
    let size = if input.size_is_3d {
        a::max(a::max(input.size[0], input.size[1]), input.size[2])
    } else {
        input.size[0]
    };
    // WorldCollision receives the already-halved radius scale from its
    // parameter block and multiplies that by size.
    let radius = a::mul(size, a::mul(input.radius_scale, 0.5));
    CollisionQuery { start, end, radius }
}

/// Already accepted contact, supplied in the particle's simulation coordinates.
/// It is not a raycast request or evidence that an intersection happened.
#[derive(Clone, Copy, Debug)]
pub struct SuppliedHit {
    pub particle_index: usize,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Same-frame module output remains separate from persistent particle velocity.
#[derive(Clone, Copy, Debug)]
pub struct FrameVelocity {
    pub animated: [f32; 3],
    pub modifier: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NonConstantResponse,
    UnqualifiedResponseRange,
    InvalidSpeedRange,
    InvalidParticleOrFrame,
    InvalidHitIndex,
    UnqualifiedNormal,
    SideArrayLengthMismatch,
}

/// The module's random stream: four lanes of xorshift128 words (`x` in
/// `words[0..4]`, then `y`, `z`, `w`). The response makes three draws for
/// every group of four hits, before its reverse loop; with constant response
/// curves (the only ones qualified) no output reads the drawn values, so the
/// words only advance. Who seeds them is not established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionRandom {
    pub words: [u32; 16],
}

impl CollisionRandom {
    /// One draw on all four lanes; returns the new `w` words.
    pub fn draw4(&mut self) -> [u32; 4] {
        let w = &mut self.words;
        let next: [u32; 4] = std::array::from_fn(|lane| {
            let x = w[lane];
            let t = x ^ (x << 11);
            let last = w[12 + lane];
            last ^ (last >> 19) ^ t ^ (t >> 8)
        });
        w.copy_within(4..16, 0);
        w[12..16].copy_from_slice(&next);
        next
    }

    /// The draws of one response call over `hits` hits: bounce, lifetime
    /// loss and dampen, per group of four hits.
    pub fn advance_for_hits(&mut self, hits: usize) {
        for _ in 0..hits.div_ceil(4) * 3 {
            self.draw4();
        }
    }
}

/// One hit's response as the engine writes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Responded {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub age_percent: f32,
    /// `fmin(age * 0.01, 1)` from the written age, 0 when negative.
    pub normalized_age: f32,
    /// The age before the lifetime loss was not above 100 percent: the
    /// collision sub-emitters record an event for this hit.
    pub records_event: bool,
}

/// Only the post-hit coefficients. Successful construction does not admit the
/// source CollisionModule: its query type, quality, masks and sinks stay outside.
#[derive(Clone, Copy, Debug)]
pub struct CollisionResponse {
    bounce: f32,
    dampen: f32,
    lifetime_loss: f32,
    minimum_speed_squared: f32,
    maximum_speed_squared: f32,
}

impl CollisionResponse {
    pub fn from_params(params: &CollisionParams) -> Result<Self, Refusal> {
        let constant = |curve: &MinMaxCurve| match curve {
            MinMaxCurve::Constant(value) => Ok(*value),
            _ => Err(Refusal::NonConstantResponse),
        };
        Self::new(
            constant(&params.bounce)?,
            constant(&params.dampen)?,
            constant(&params.lifetime_loss)?,
            params.min_kill_speed,
            params.max_kill_speed,
        )
    }

    pub fn new(
        bounce: f32,
        dampen: f32,
        lifetime_loss: f32,
        minimum_speed: f32,
        maximum_speed: f32,
    ) -> Result<Self, Refusal> {
        // These are the bounded response ranges the native rows cover.
        // Do not clamp invalid inputs into an apparently accepted response.
        if [bounce, dampen, lifetime_loss]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(Refusal::UnqualifiedResponseRange);
        }
        let minimum_speed_squared = minimum_speed * minimum_speed;
        let maximum_speed_squared = maximum_speed * maximum_speed;
        if !minimum_speed.is_finite()
            || minimum_speed < 0.0
            || maximum_speed < minimum_speed
            || !maximum_speed_squared.is_finite()
        {
            return Err(Refusal::InvalidSpeedRange);
        }
        Ok(Self {
            bounce,
            dampen,
            lifetime_loss,
            minimum_speed_squared,
            maximum_speed_squared,
        })
    }

    /// One hit of the reverse loop. `modifier` is the particle's speed
    /// modifier when the particle state's speed-modifier flag is set. The age
    /// takes the lifetime loss, then the kill test (age above 100 percent, or
    /// the full speed below the minimum or not at most the maximum) writes the
    /// killed age; a killed particle still takes the full response. The
    /// position is reflected about the hit plane through the hit point and the
    /// velocity likewise, both dampened and with the normal part scaled by the
    /// bounce, in the engine's operation order.
    #[allow(clippy::too_many_arguments)]
    pub fn respond(
        &self,
        position: [f32; 3],
        velocity: [f32; 3],
        animated: [f32; 3],
        modifier: Option<f32>,
        age_percent: f32,
        point: [f32; 3],
        normal: [f32; 3],
    ) -> Responded {
        let keep = a::sub(1.0, self.bounce);
        let damp = a::sub(1.0, self.dampen);
        let mut total: [f32; 3] = std::array::from_fn(|k| a::add(animated[k], velocity[k]));
        if let Some(modifier) = modifier {
            total = total.map(|t| a::mul(t, modifier));
        }
        let age = a::add(age_percent, a::mul(self.lifetime_loss, 100.0));
        let speed_squared = dot(total, total);
        let over = if arms::on("killAtHundred") { age >= 100.0 } else { age > 100.0 };
        let killed = over
            || speed_squared < self.minimum_speed_squared
            || !(speed_squared <= self.maximum_speed_squared);
        let age = if killed { f32::from_bits(KILLED_AGE) } else { age };
        let scaled = a::mul(age, AGE_SCALE);
        let normalized_age = if scaled < 0.0 { 0.0 } else { a::min(scaled, 1.0) };
        let offset: [f32; 3] = std::array::from_fn(|k| a::sub(position[k], point[k]));
        let reflect = |v: [f32; 3]| -> [f32; 3] {
            let projection = a::mul(dot(v, normal), -2.0);
            std::array::from_fn(|k| a::mul(damp, a::add(v[k], a::mul(normal[k], projection))))
        };
        // The final projection adds Z to the X+Y subtotal.
        let along = |v: [f32; 3]| {
            a::add(a::mul(normal[2], v[2]), a::add(a::mul(normal[0], v[0]), a::mul(normal[1], v[1])))
        };
        let offset = reflect(offset);
        let offset_along = along(offset);
        let position = std::array::from_fn(|k| {
            a::add(point[k], a::sub(offset[k], a::mul(keep, a::mul(normal[k], offset_along))))
        });
        let total = reflect(total);
        let total_along = along(total);
        let velocity = std::array::from_fn(|k| {
            let mut v = a::sub(total[k], a::mul(keep, a::mul(normal[k], total_along)));
            if let Some(modifier) = modifier {
                v = a::div(v, modifier);
            }
            a::sub(v, animated[k])
        });
        Responded {
            position,
            velocity,
            age_percent: age,
            normalized_age,
            records_event: !(age_percent > 100.0),
        }
    }

    /// Apply supplied records in the native reverse traversal order. Pool length,
    /// side data and hit indices remain stable until a separate compaction phase.
    /// Rejection validates the whole batch first, leaving it untouched. This
    /// entry makes no random draws and records no events; the module update
    /// does both.
    pub fn apply_hits(
        &self,
        particles: &mut [Particle],
        frames: &[FrameVelocity],
        hits: &[SuppliedHit],
    ) -> Result<(), Refusal> {
        if frames.len() != particles.len() {
            return Err(Refusal::SideArrayLengthMismatch);
        }
        for hit in hits {
            let particle = particles
                .get(hit.particle_index)
                .ok_or(Refusal::InvalidHitIndex)?;
            let frame = &frames[hit.particle_index];
            // Hits supplied from outside are accepted only near unit length,
            // without normalizing or otherwise altering them.
            if !unit_normal(hit.normal) {
                return Err(Refusal::UnqualifiedNormal);
            }
            if !all_finite(hit.point)
                || !all_finite(particle.position)
                || !all_finite(particle.velocity)
                || !particle.age_percent.is_finite()
                || !all_finite(frame.animated)
                || !frame.modifier.is_finite()
                || frame.modifier <= 0.0
            {
                return Err(Refusal::InvalidParticleOrFrame);
            }
        }
        for hit in hits.iter().rev() {
            let particle = &mut particles[hit.particle_index];
            let frame = frames[hit.particle_index];
            let out = self.respond(particle.position, particle.velocity, frame.animated,
                Some(frame.modifier), particle.age_percent, hit.point, hit.normal);
            particle.position = out.position;
            particle.velocity = out.velocity;
            particle.age_percent = out.age_percent;
        }
        Ok(())
    }
}

fn dot(a3: [f32; 3], b3: [f32; 3]) -> f32 {
    a::add(a::add(a::mul(a3[0], b3[0]), a::mul(a3[1], b3[1])), a::mul(a3[2], b3[2]))
}

fn all_finite(values: [f32; 3]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn unit_normal(normal: [f32; 3]) -> bool {
    all_finite(normal) && (dot(normal, normal) - 1.0).abs() <= 8.0 * f32::EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query_input(world_space: bool) -> QueryInput {
        QueryInput {
            position: [10.0, 4.0, -2.0],
            persistent_velocity: [2.0, -1.0, 0.5],
            animated_velocity: [0.5, 0.25, -0.5],
            speed_modifier: Some(0.5),
            dt: 0.25,
            size: [2.0, 3.0, 4.0],
            size_is_3d: true,
            radius_scale: 0.16,
            world_space,
            owner: QueryAffine {
                linear: [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]],
                translation: [1.0, -2.0, 5.0],
            },
        }
    }

    #[test]
    fn query_world_uses_current_position_and_native_radius_order() {
        let query = build_query(query_input(true));
        assert_eq!(query.start, [9.6875, 4.09375, -2.0]);
        assert_eq!(query.end, [10.0, 4.0, -2.0]);
        assert_eq!(query.radius, 0.32);
    }

    #[test]
    fn query_local_transforms_both_endpoints_through_owner_affine() {
        let query = build_query(query_input(false));
        assert_eq!(query.start, [20.375, 10.28125, -3.0]);
        assert_eq!(query.end, [21.0, 10.0, -3.0]);
        assert_eq!(query.radius, 0.32);
    }

    #[test]
    fn query_radius_reads_only_x_when_native_size3d_flag_is_off() {
        let mut input = query_input(true);
        input.size_is_3d = false;
        assert_eq!(build_query(input).radius, 0.16);
    }

    #[test]
    fn query_preserves_native_nonfinite_arithmetic_without_inventing_a_cutoff() {
        let mut input = query_input(true);
        input.dt = 0.999_999e-6;
        assert!(build_query(input).start.iter().all(|v| v.is_finite()));
        input.dt = 1.0e-6;
        assert!(build_query(input).start.iter().all(|v| v.is_finite()));
        input.position[1] = f32::NAN;
        assert!(build_query(input).start[1].is_nan());
    }

    #[test]
    fn world_hit_conversion_uses_supplied_inverse_vector_normal() {
        let inverse = QueryAffine {
            linear: [[0.5, 0.0, 0.0], [0.0, 1.0 / 3.0, 0.0], [0.0, 0.0, 0.25]],
            translation: [-0.5, 2.0 / 3.0, -1.25],
        };
        let hit = world_hit_to_simulation(inverse, 7, [21.0, 10.0, -3.0], [0.0, 3.0, 0.0]);
        assert_eq!(hit.particle_index, 7);
        assert_eq!(hit.point, [10.0, 4.0, -2.0]);
        assert_eq!(hit.normal, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn world_hit_conversion_turns_a_degenerate_normal_into_z_without_refusing() {
        let singular = QueryAffine {
            linear: [[0.0; 3]; 3],
            translation: [0.0; 3],
        };
        assert_eq!(world_hit_to_simulation(singular, 0, [0.0; 3], [0.0, 1.0, 0.0]).normal, [0.0, 0.0, 1.0]);
        assert_eq!(world_hit_to_simulation(QueryAffine::IDENTITY, 0, [0.0; 3], [0.0; 3]).normal, [0.0, 0.0, 1.0]);
    }

    fn hit(index: usize, axis: usize) -> SuppliedHit {
        let mut normal = [0.0; 3];
        normal[axis] = 1.0;
        SuppliedHit {
            particle_index: index,
            point: [0.0; 3],
            normal,
        }
    }

    #[test]
    fn dampening_restores_persistent_velocity_separately_from_animated() {
        let response = CollisionResponse::new(1.0, 1.0, 0.0, 0.0, 1000.0).unwrap();
        let mut particles = [Particle::born([1.0, 2.0, 3.0], [8.0, -4.0, 2.0], 5.0)];
        let frames = [FrameVelocity {
            animated: [0.5, -1.0, 0.25],
            modifier: 2.0,
        }];
        response
            .apply_hits(&mut particles, &frames, &[hit(0, 1)])
            .unwrap();
        assert_eq!(particles[0].position, [0.0; 3]);
        assert_eq!(particles[0].velocity, [-0.5, 1.0, -0.25]);
        assert_eq!(particles[0].inverse_lifetime, 0.2);
        assert_eq!(frames[0].animated, [0.5, -1.0, 0.25]);
    }

    #[test]
    fn kill_uses_incoming_full_velocity_and_strict_age_speed_boundaries() {
        let response = CollisionResponse::new(0.0, 1.0, 0.2, 1.0, 4.0).unwrap();
        let mut particles = [1.0, 4.0, 0.5, 4.5].map(|speed| {
            let mut p = Particle::born([1.0; 3], [speed - 0.5, 0.0, 0.0], 4.0);
            p.age_percent = 80.0;
            p
        });
        let frames = [FrameVelocity {
            animated: [0.5, 0.0, 0.0],
            modifier: 1.0,
        }; 4];
        response
            .apply_hits(
                &mut particles,
                &frames,
                &(0..4).map(|i| hit(i, 0)).collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(particles[0].age_percent, 100.0);
        assert_eq!(particles[1].age_percent, 100.0);
        assert_eq!(particles[2].age_percent.to_bits(), KILLED_AGE);
        assert_eq!(particles[3].age_percent.to_bits(), KILLED_AGE);
        assert_eq!(particles.len(), 4, "marking is not death compaction");
        assert_eq!(
            particles[3].velocity,
            [-0.5, 0.0, 0.0],
            "killed lanes still receive response"
        );
    }

    #[test]
    fn reverse_hit_order_is_preserved_without_relocating_indices() {
        let response = CollisionResponse::new(1.0, 0.0, 0.0, 0.0, 1000.0).unwrap();
        let mut particles = [Particle::born([3.0, 0.0, 0.0], [1.0, 0.0, 0.0], 2.0)];
        let frames = [FrameVelocity {
            animated: [0.0; 3],
            modifier: 1.0,
        }];
        let a = hit(0, 0);
        let mut b = a;
        b.point = [1.0, 0.0, 0.0];
        response
            .apply_hits(&mut particles, &frames, &[a, b])
            .unwrap();
        assert_eq!(particles[0].position, [1.0, 0.0, 0.0]);
        assert_eq!(particles[0].velocity, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn unqualified_inputs_do_not_mutate_any_particle() {
        let response = CollisionResponse::new(0.0, 0.0, 1.0, 0.0, 1000.0).unwrap();
        let original = Particle::born([1.0; 3], [2.0; 3], 2.0);
        let mut particles = [original];
        let frames = [FrameVelocity {
            animated: [0.0; 3],
            modifier: 1.0,
        }];
        let mut invalid = hit(0, 0);
        invalid.normal = [0.5, 0.5, 0.0];
        assert_eq!(
            response.apply_hits(&mut particles, &frames, &[invalid, hit(0, 0)]),
            Err(Refusal::UnqualifiedNormal)
        );
        assert_eq!(particles, [original]);
        assert_eq!(
            response.apply_hits(&mut particles, &frames, &[hit(1, 0)]),
            Err(Refusal::InvalidHitIndex)
        );
        assert_eq!(particles, [original]);
    }

    /// Expected document shape: {"samples":[{"input":{positions,velocities,
    /// animated,ages,hits,bounce,dampen,loss,minimum,maximum,modifier},
    /// "output":{positions,velocities,ages,count}}]}. `output` must come from
    /// executing the qualified native entry, never this draft's own formulas.
    #[test]
    #[ignore = "requires MOLY_COLLISION_RESPONSE_SAMPLES; this draft is not runtime admission"]
    fn supplied_hit_response_matches_external_native_rows() {
        use crate::particle::json::{parse, Value};
        fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
            value
                .get(key)
                .unwrap_or_else(|| panic!("missing field {key}"))
        }
        fn items(value: &Value) -> &[Value] {
            value.as_array().expect("source array")
        }
        fn integer(value: &Value) -> usize {
            let number = value.as_f64().expect("source integer");
            assert!(number.is_finite() && number >= 0.0 && number.fract() == 0.0);
            assert!(number <= usize::MAX as f64);
            number as usize
        }
        let path =
            std::env::var_os("MOLY_COLLISION_RESPONSE_SAMPLES").expect("external native rows");
        let document = parse(&std::fs::read(path).unwrap()).unwrap();
        let rows = items(field(&document, "samples"));
        assert!(!rows.is_empty());
        let number = |value: &Value| value.as_f64().expect("finite source scalar") as f32;
        let vector = |value: &Value| {
            let values = items(value);
            assert_eq!(values.len(), 3);
            std::array::from_fn(|i| number(&values[i]))
        };
        for (case, row) in rows.iter().enumerate() {
            let input = field(row, "input");
            let output = field(row, "output");
            let response = CollisionResponse::new(
                number(field(input, "bounce")),
                number(field(input, "dampen")),
                number(field(input, "loss")),
                number(field(input, "minimum")),
                number(field(input, "maximum")),
            )
            .unwrap();
            let mut particles: Vec<_> = items(field(input, "positions"))
                .iter()
                .enumerate()
                .map(|(i, position)| {
                    let mut particle = Particle::born(
                        vector(position),
                        vector(&items(field(input, "velocities"))[i]),
                        1.0,
                    );
                    particle.age_percent = number(&items(field(input, "ages"))[i]);
                    particle
                })
                .collect();
            let frames: Vec<_> = items(field(input, "animated"))
                .iter()
                .map(|animated| FrameVelocity {
                    animated: vector(animated),
                    modifier: number(field(input, "modifier")),
                })
                .collect();
            let hits: Vec<_> = items(field(input, "hits"))
                .iter()
                .map(|hit| SuppliedHit {
                    particle_index: integer(field(hit, "index")),
                    point: vector(field(hit, "point")),
                    normal: vector(field(hit, "normal")),
                })
                .collect();
            response.apply_hits(&mut particles, &frames, &hits).unwrap();
            assert_eq!(particles.len(), integer(field(output, "count")));
            for (index, particle) in particles.iter().enumerate() {
                for (key, values) in [
                    ("positions", particle.position),
                    ("velocities", particle.velocity),
                ] {
                    for axis in 0..3 {
                        assert_eq!(
                            values[axis].to_bits(),
                            number(&items(&items(field(output, key))[index])[axis]).to_bits(),
                            "native {case}/{index}/{key}/{axis}"
                        );
                    }
                }
                assert_eq!(
                    particle.age_percent.to_bits(),
                    number(&items(field(output, "ages"))[index]).to_bits(),
                    "native {case}/{index}/age"
                );
            }
        }
    }
}
