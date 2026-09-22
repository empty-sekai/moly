//! Qualified supplied-hit response; query and event ownership remain separate.
//!
//! Current PerformPlaneCollisions response updates position, persistent velocity
//! and native age. The qualified query/input and supplied-hit coordinate
//! boundaries are exposed below; scene hit production (broadphase/PhysX),
//! child events/messages and later death compaction remain caller obligations.
//! This module emits no events and removes no particles.
//!
//! The full source CollisionModule stays gated until query/event ownership is
//! integrated and independently verified.

use crate::particle::{schema::CollisionParams, MinMaxCurve, Particle};

/// Source `WorldCollision` query input after the particle/update state has
/// been sampled.  This is deliberately only the query boundary: it does not
/// run a scene broadphase, PhysX sweep, hit selection, response or event
/// ownership.  The current JP receipt (`collision-query-native.md`) qualifies
/// these fields against libunity 937c6d28..., including the local/custom owner
/// transform.  The downstream PhysX leaf has its own travel cutoff; it does
/// not belong to this WorldCollision input builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryInput {
    pub position: [f32; 3],
    pub persistent_velocity: [f32; 3],
    pub animated_velocity: [f32; 3],
    pub speed_modifier: f32,
    pub dt: f32,
    /// The one size stream selected by the native particle-state flag.  Do not
    /// merge this with the other size stream: WorldCollision reads one pointer
    /// selected by `ParticleSystemParticles+0x7d2`.
    pub size: [f32; 3],
    /// Native `+0x7d4`: false consumes the X component only; true performs
    /// `fmax(x,y)` then `fmax(result,z)`.
    pub size_is_3d: bool,
    /// Authored CollisionModule radiusScale.  WorldCollision's parameter
    /// setup halves this value before the SIMD multiply.
    pub radius_scale: f32,
    /// `true` for World simulation; false uses the supplied owner affine for
    /// both query endpoints.  This keeps Local and Custom source paths
    /// explicit instead of silently treating them as world coordinates.
    pub world_space: bool,
    pub owner: QueryAffine,
}

/// Minimal source-space affine used at the collision query boundary.  The
/// matrix is row-major and multiplies a column vector.  Keeping this type
/// independent of Bevy lets the law be replayed by native probes and tests.
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

    #[inline]
    fn is_finite(self) -> bool {
        self.linear.iter().flatten().all(|value| value.is_finite())
            && self.translation.iter().all(|value| value.is_finite())
    }

    #[inline]
    fn point(self, value: [f32; 3]) -> [f32; 3] {
        [
            (self.linear[0][0] * value[0] + self.linear[0][1] * value[1])
                + self.linear[0][2] * value[2]
                + self.translation[0],
            (self.linear[1][0] * value[0] + self.linear[1][1] * value[1])
                + self.linear[1][2] * value[2]
                + self.translation[1],
            (self.linear[2][0] * value[0] + self.linear[2][1] * value[1])
                + self.linear[2][2] * value[2]
                + self.translation[2],
        ]
    }

    #[inline]
    fn vector(self, value: [f32; 3]) -> [f32; 3] {
        [
            (self.linear[0][0] * value[0] + self.linear[0][1] * value[1])
                + self.linear[0][2] * value[2],
            (self.linear[1][0] * value[0] + self.linear[1][1] * value[1])
                + self.linear[1][2] * value[2],
            (self.linear[2][0] * value[0] + self.linear[2][1] * value[1])
                + self.linear[2][2] * value[2],
        ]
    }
}

/// Current JP query record passed to the physics interface. `start` and `end`
/// are the swept endpoints; `radius` is the source radius before the native
/// skin margin and geometry dispatch. Travel direction/length are derived by
/// the downstream PhysX implementation and are intentionally not fabricated
/// here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionQuery {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub radius: f32,
}

/// Convert a current-world hit into the simulation coordinates consumed by
/// `CollisionResponse`. `inverse` is the inverse owner transform supplied by
/// the native update state; this adapter intentionally does not reconstruct
/// it from a forward transform. The current JP query receipt qualifies the
/// inverse-vector normal path, followed by native normalization.
pub fn world_hit_to_simulation(
    inverse: QueryAffine,
    particle_index: usize,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<SuppliedHit> {
    if !inverse.is_finite()
        || point
            .iter()
            .chain(normal.iter())
            .any(|value| !value.is_finite())
    {
        return None;
    }
    let normal = inverse.vector(normal);
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if !length.is_finite() || length < 1.0e-30 {
        return None;
    }
    Some(SuppliedHit {
        particle_index,
        point: inverse.point(point),
        normal: [normal[0] / length, normal[1] / length, normal[2] / length],
    })
}

/// Build the qualified `WorldCollision` query input. The caller still owns
/// collider filtering, triangle/BVH traversal and hit response; this function
/// must not be used as evidence that a source CollisionModule is admitted.
pub fn build_query(input: QueryInput) -> CollisionQuery {
    // Native arithmetic is component-wise f32: velocity sum, speed modifier,
    // then dt, followed by subtraction from the current position.
    let start = std::array::from_fn(|axis| {
        input.position[axis]
            - ((input.persistent_velocity[axis] + input.animated_velocity[axis])
                * input.speed_modifier)
                * input.dt
    });
    let end = input.position;
    let (start, end) = if input.world_space {
        (start, end)
    } else {
        (input.owner.point(start), input.owner.point(end))
    };
    let size = if input.size_is_3d {
        input.size[0].max(input.size[1]).max(input.size[2])
    } else {
        input.size[0]
    };
    // WorldCollision receives the already-halved radius scale from its
    // parameter block (`radiusScale * 0.5`) and multiplies that by size.
    let radius = size * (input.radius_scale * 0.5);
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
        // These are the bounded response ranges covered by the current receipt.
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

    /// Apply supplied records in the native reverse traversal order. Pool length,
    /// side data and hit indices remain stable until a separate compaction phase.
    /// Rejection validates the whole batch first, leaving it untouched.
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
            // The current response replay includes axis and oblique unit normals.
            // Accept only a float-rounding neighborhood, without normalizing or
            // otherwise altering the explicitly supplied query result.
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
            let total: [f32; 3] = std::array::from_fn(|axis| {
                (particle.velocity[axis] + frame.animated[axis]) * frame.modifier
            });
            let speed_squared = dot(total, total);
            particle.age_percent += self.lifetime_loss * 100.0;
            // Thresholds and native death are strict. Even a killed particle
            // completes the same hit response before the later death phase.
            if particle.age_percent > 100.0
                || speed_squared < self.minimum_speed_squared
                || speed_squared > self.maximum_speed_squared
            {
                particle.age_percent = f32::from_bits(0x42c8_0001);
            }
            let displacement =
                std::array::from_fn(|axis| particle.position[axis] - hit.point[axis]);
            let displacement = self.response(displacement, hit.normal);
            particle.position = std::array::from_fn(|axis| hit.point[axis] + displacement[axis]);
            let velocity = self.response(total, hit.normal);
            particle.velocity =
                std::array::from_fn(|axis| velocity[axis] / frame.modifier - frame.animated[axis]);
        }
        Ok(())
    }

    fn response(&self, vector: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
        let projection = dot(vector, normal) * -2.0;
        let reflected: [f32; 3] = std::array::from_fn(|axis| {
            (vector[axis] + normal[axis] * projection) * (1.0 - self.dampen)
        });
        // The native final projection adds Z to the X+Y subtotal. Replacing this
        // with a generic dot product changes rounding and sometimes signed zero.
        let normal_component =
            normal[2] * reflected[2] + (normal[0] * reflected[0] + normal[1] * reflected[1]);
        std::array::from_fn(|axis| {
            reflected[axis] - (1.0 - self.bounce) * (normal[axis] * normal_component)
        })
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
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
            speed_modifier: 0.5,
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
        let hit = world_hit_to_simulation(inverse, 7, [21.0, 10.0, -3.0], [0.0, 3.0, 0.0]).unwrap();
        assert_eq!(hit.particle_index, 7);
        assert_eq!(hit.point, [10.0, 4.0, -2.0]);
        assert_eq!(hit.normal, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn world_hit_conversion_rejects_zero_normal_but_does_not_rebuild_inverse() {
        let singular = QueryAffine {
            linear: [[0.0; 3]; 3],
            translation: [0.0; 3],
        };
        assert!(world_hit_to_simulation(singular, 0, [0.0; 3], [0.0, 1.0, 0.0]).is_none());
        assert!(world_hit_to_simulation(QueryAffine::IDENTITY, 0, [0.0; 3], [0.0; 3]).is_none());
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
        assert_eq!(particles[2].age_percent.to_bits(), 0x42c8_0001);
        assert_eq!(particles[3].age_percent.to_bits(), 0x42c8_0001);
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
