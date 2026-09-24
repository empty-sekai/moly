//! Explicit native birth commands. The graph/lifecycle owner supplies RNG and
//! timing; this entry never advances autonomous clocks or invents a seed.
use super::*;
use moly_law::particle::autonomous_emission::{
    AutonomousEmissionState, ConstantAutonomousEmission,
};
use moly_law::particle::{
    curve::CurveSampler,
    initial::{InitialContext, InitialGroupInput, InitialLaw},
    noise::NoiseLaw,
    random::ParticleRandom,
    seed_owner::ModuleRandom,
    sub_emission::BirthBatch,
};

#[derive(Clone, Debug)]
pub(crate) struct NativeBirthState {
    pub owner: Option<moly_law::particle::seed_owner::SeedOwner>,
    pub initial: ModuleRandom,
    pub shape: ModuleRandom,
    /// ShapeModule's arc clock (current, previous), in binary64. The seed
    /// reset of the system zeroes it together with the Shape stream; every
    /// ordinary update slice advances it before that slice's births.
    pub shape_clock: moly_law::particle::shape::ArcLoopClock,
    pub emission: AutonomousEmissionState,
}

/// Name-independent qualification of the native birth composition: autonomous
/// emission with a constant or two-constant rate and burst count, the Initial
/// law, the qualified Shape configurations,
/// the qualified Noise subset and authored-null child edges. The source update
/// route (ordinary versus procedural) is decided by the caller from the
/// exported system block, not here.
pub(super) fn qualify_emitter(emitter: &EmitterParams) -> Result<(), BirthRefused> {
    let Some(emission) = emitter.emission.as_ref() else {
        return Err(BirthRefused::Unsupported("missing emission"));
    };
    ConstantAutonomousEmission::from_params(
        &emitter.start_delay,
        emitter.duration,
        emitter.looping,
        emission,
    )
    .map_err(BirthRefused::Emission)?;
    validate_emitter(emitter, &ModuleRandom::from_owner_seed(0)).map(|_| ())
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum BirthRefused {
    Unsupported(&'static str),
    InvalidTiming,
    Initial(moly_law::particle::initial::Refused),
    Emission(moly_law::particle::autonomous_emission::Refused),
}

/// One initialized ordinary slice: clock, existing-particle barrier, then the
/// scheduled births with their partial times. Shape and Noise ride the same
/// slice. Capacity is read only after old particles have completed their full
/// step and death compaction. No child graph or automatic seed/lifecycle
/// ownership is inferred by this explicit entry.
pub(super) fn step_explicit(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    stopped: bool,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    validate(system, &state.initial)?;
    if system.emitter.shape.is_none() {
        validate_unshaped_owner(&system.emitter, &compose_to_world(system, ctx))?;
    }
    let emission = system
        .emitter
        .emission
        .as_ref()
        .ok_or(BirthRefused::Unsupported("missing emission"))?;
    let law = ConstantAutonomousEmission::from_params(
        &system.emitter.start_delay,
        system.emitter.duration,
        system.emitter.looping,
        emission,
    )
    .map_err(BirthRefused::Emission)?;
    let clock = law
        .prepare_slice(system.playback_head, dt, stopped)
        .map_err(BirthRefused::Emission)?;
    // Schedule into a copy: this validates arithmetic without advancing live
    // emission state ahead of the old-particle post/death barrier.
    let mut pending = state.emission;
    let batch = law
        .schedule(
            clock,
            &mut pending,
            moly_law::particle::initial::initial_reciprocal,
        )
        .map_err(BirthRefused::Emission)?;
    if !clock.simulate {
        return Ok(());
    }
    system.previous_head = clock.previous;
    system.playback_head = clock.current;
    // ParticleSystem::Update1Incremental runs the pre-simulation module
    // update, ShapeModule::Update among it, before the slice's births: the
    // arc clock moves by the slice dt times the arc speed. Only the Loop and
    // PingPong cones read the clock, and only a constant arc speed is admitted.
    if let Some(speed) = system
        .emitter
        .shape
        .as_ref()
        .and_then(|params| moly_law::particle::shape_birth::ShapeBirthLaw::from_params(params).ok())
        .and_then(|law| law.arc_clock_speed())
    {
        state.shape_clock.advance(speed, dt);
    }
    simulate_existing(system, dt, ctx);
    if let Some(batch) = batch {
        state.emission = pending;
        system.emission.to_emit_accumulator = pending.distribution.offset;
        start_common(
            system,
            &mut state.initial,
            Some(&mut state.shape),
            Some(state.shape_clock),
            BirthBatch {
                count: batch.total,
                rate_count: batch.rate_count,
                distribution: batch.distribution,
            },
            batch.dt,
            batch.previous_normalized,
            batch.current_normalized,
            ctx,
        )?;
    }
    Ok(())
}

/// Ordinary no-shape birth. All calculations are staged before modifying the
/// pool/RNG. Source graph admission remains separate from this explicit command.
pub(super) fn start_explicit(
    system: &mut Runtime,
    random: &mut ModuleRandom,
    batch: BirthBatch,
    dt: f32,
    previous_normalized: f32,
    current_normalized: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    start_common(
        system,
        random,
        None,
        None,
        batch,
        dt,
        previous_normalized,
        current_normalized,
        ctx,
    )
}

/// Initial and Shape own independent streams, including all four padded
/// lanes. Explicit probe/lifecycle caller; admission of a source system is
/// decided by the installer, not by this command.
pub(super) fn start_explicit_with_shape(
    system: &mut Runtime,
    initial: &mut ModuleRandom,
    shape: &mut ModuleRandom,
    batch: BirthBatch,
    dt: f32,
    previous_normalized: f32,
    current_normalized: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    start_common(
        system,
        initial,
        Some(shape),
        None,
        batch,
        dt,
        previous_normalized,
        current_normalized,
        ctx,
    )
}

/// Emitter-state inputs of the native Shape boundary beyond the Shape block.
#[derive(Clone, Copy, Debug)]
pub(super) struct ShapeEmitterState {
    pub emitter_scale: [f32; 3],
    pub uses_axis_of_rotation: bool,
}

/// UpdateLocalToWorldMatrixAndScales stores the emitter scale ShapeModule
/// reads: one for Hierarchy scaling, and one for Local scaling except for a
/// MeshRenderer shape, which the Shape law does not admit. Its Local branch
/// builds the world owner from hierarchy rotation and translation only; node
/// scales enter positions and rotation signs, never the matrix magnitudes. The
/// composed affine owner therefore equals it only over a unit-scale chain, and
/// a World-space owner over any other chain is refused: that derivation was
/// read, not executed. The Shape scaling mode has no qualified derivation and
/// never reaches here (the source adapter refuses it). The particle arrays
/// carry the axis-of-rotation channel when the renderer is in Mesh render mode.
pub(super) fn shape_emitter_state(
    emitter: &EmitterParams,
    evidence: Option<ShapeEmitterEvidence>,
) -> Result<Option<ShapeEmitterState>, BirthRefused> {
    if emitter.shape.is_none() {
        return Ok(None);
    }
    let Some(evidence) = evidence else {
        return Err(BirthRefused::Unsupported(
            "native Shape emitter state lacks the authored scaling and render modes",
        ));
    };
    let emitter_scale = match evidence.scaling {
        crate::particle_geometry::Scaling::Hierarchy => [1.0; 3],
        crate::particle_geometry::Scaling::Local { unit_chain, .. } => {
            if emitter.simulation_space == SimulationSpace::World && !unit_chain {
                return Err(BirthRefused::Unsupported(
                    "World-space Local-scaling Shape owner over a non-unit node scale is not executed",
                ));
            }
            [1.0; 3]
        }
    };
    Ok(Some(ShapeEmitterState {
        emitter_scale,
        uses_axis_of_rotation: evidence.mesh_renderer,
    }))
}

fn shape_refusal(refused: moly_law::particle::shape_birth::Refused) -> BirthRefused {
    match refused {
        moly_law::particle::shape_birth::Refused::ExportSchema => BirthRefused::Unsupported(
            "Shape controls are not the current export schema (an export check, not a ShapeModule member)",
        ),
        moly_law::particle::shape_birth::Refused::ShapeTexture => BirthRefused::Unsupported(
            "Shape texture reference: ApplyTexture is not transcribed",
        ),
        _ => BirthRefused::Unsupported("unqualified native Shape configuration"),
    }
}

fn start_common(
    system: &mut Runtime,
    random: &mut ModuleRandom,
    mut shape_stream: Option<&mut ModuleRandom>,
    shape_clock: Option<moly_law::particle::shape::ArcLoopClock>,
    batch: BirthBatch,
    dt: f32,
    previous_normalized: f32,
    current_normalized: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    let law = validate(system, random)?;
    let shape_law = system
        .emitter
        .shape
        .as_ref()
        .map(|params| {
            moly_law::particle::shape_birth::ShapeBirthLaw::from_params(params).map_err(shape_refusal)
        })
        .transpose()?;
    let shape_state = shape_emitter_state(&system.emitter, system.geometry.shape_evidence())?;
    if shape_law.is_some() && shape_stream.is_none() {
        return Err(BirthRefused::Unsupported(
            "missing independent Shape stream",
        ));
    }
    if !dt.is_finite() || dt < 0.0 || batch.rate_count > batch.count {
        return Err(BirthRefused::InvalidTiming);
    }
    let old_count = system.pool.len();
    let accepted = birth_capacity(
        old_count,
        system.emitter.ring_buffer_mode,
        system.emitter.max_particles as usize,
        batch.count as usize,
    );
    if accepted == 0 {
        system.full_total += batch.count as u64;
        return Ok(());
    }
    // One ShapeBatch per StartParticles call: the accepted count after the
    // capacity decision, the emission spacing and offset of this call, and
    // the arc clock as the slice's module update left it.
    let mut shape_batch = match shape_law.as_ref() {
        Some(law) => {
            if law.arc_clock_speed().is_some() && shape_clock.is_none() {
                return Err(BirthRefused::Unsupported("arc clock owner is not installed"));
            }
            let accepted = u32::try_from(accepted)
                .ok()
                .and_then(std::num::NonZeroU32::new)
                .ok_or(BirthRefused::InvalidTiming)?;
            Some(moly_law::particle::shape_birth::ShapeBatch::new(
                accepted,
                batch.distribution.spacing,
                batch.distribution.offset,
                shape_clock.unwrap_or_default(),
            ))
        }
        None => None,
    };
    // ParticleSystem::StartVelocity evaluates the start speed through
    // Evaluate(MinMaxCurve), which follows the reader's isOptimizedCurve bit.
    let speed = CurveSampler::new(&system.emitter.start.speed,
        moly_law::particle::curve::CurveTime::Normalized)
        .map_err(BirthRefused::Unsupported)?;
    let owner = compose_to_world(system, ctx);
    // The runtime owner is reflected-X; module parameters are source Unity
    // coordinates. Convert its basis once, retaining native multiplication order.
    let source_owner = owner.to_matrix().to_cols_array();
    let source_owner = std::array::from_fn(|i| {
        if (i % 4 == 0) ^ (i / 4 == 0) {
            -source_owner[i]
        } else {
            source_owner[i]
        }
    });
    let mut next = *random;
    let mut next_shape = shape_stream.as_deref().copied();
    let mut particles = Vec::with_capacity(accepted.next_multiple_of(4));
    let mut sides = Vec::with_capacity(particles.capacity());
    let mut partial_dts = Vec::with_capacity(particles.capacity());
    for offset in (0..accepted).step_by(4) {
        let timing = (0..4)
            .map(|lane| {
                batch
                    .distribution
                    .timing(
                        (offset + lane) as u32,
                        batch.rate_count,
                        dt,
                        previous_normalized,
                        current_normalized,
                    )
                    .map_err(|_| BirthRefused::InvalidTiming)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let group = law
            .start_group_native(
                &mut next,
                InitialGroupInput {
                    active_lanes: (accepted - offset).min(4),
                    storage_size_3d: system.emitter.start.size3d,
                    storage_rotation_3d: system.emitter.start.rotation3d,
                    birth_fraction: std::array::from_fn(|lane| timing[lane].fraction),
                    // StartParticles hands InitialModule::Start one normalized
                    // current time for the whole command (current times the
                    // duration reciprocal), broadcast to all four lanes. The
                    // per-lane interpolated time is StartVelocity's input only.
                    curve_time: [current_normalized; 4],
                    context: InitialContext::Autonomous,
                },
            )
            .map_err(BirthRefused::Initial)?;
        let shaped = shape_law
            .as_ref()
            .map(|shape| {
                let state = shape_state.expect("a Shape block has its validated emitter state");
                shape
                    .sample_group(
                        shape_batch.as_mut().expect("a Shape law has its batch"),
                        next_shape
                            .as_mut()
                            .expect("validated independent Shape stream"),
                        source_owner,
                        system.emitter.simulation_space == SimulationSpace::World,
                        state.emitter_scale,
                        state.uses_axis_of_rotation,
                    )
                    .map_err(|_| BirthRefused::Unsupported("unqualified native Shape owner/output"))
            })
            .transpose()?;
        for (index, lane) in group.lanes.into_iter().enumerate() {
            // StartVelocity evaluates at the per-lane birth time, not at
            // Initial's broadcast input that the lane echoes.
            let sampled_speed = speed.evaluate(
                timing[index].curve_time,
                ParticleRandom::sample(lane.seed, 0x96aa_4de3),
            );
            let (position, velocity) = if let Some(shaped) = &shaped {
                let sample = &shaped.samples[index];
                (
                    crate::particle_geometry::reflect(Vec3::from_array(sample.position)).to_array(),
                    crate::particle_geometry::reflect(Vec3::from_array(
                        sample.direction.map(|v| v * sampled_speed),
                    ))
                    .to_array(),
                )
            } else if system.emitter.simulation_space == SimulationSpace::World {
                (
                    owner.translation().to_array(),
                    owner
                        .affine()
                        .transform_vector3(Vec3::Z)
                        .normalize_or_zero()
                        .to_array()
                        .map(|v| v * sampled_speed),
                )
            } else {
                ([0.0; 3], [0.0, 0.0, sampled_speed])
            };
            if !sampled_speed.is_finite()
                || position
                    .iter()
                    .chain(velocity.iter())
                    .any(|v| !v.is_finite())
            {
                return Err(BirthRefused::Unsupported(
                    "nonfinite birth position or velocity",
                ));
            }
            particles.push(Particle {
                position,
                velocity,
                start_lifetime: lane.lifetime,
                inverse_lifetime: lane.inverse_lifetime,
                age_percent: 0.0,
            });
            sides.push(Side {
                rand: 0.0,
                seed: lane.seed,
                rot: lane.rotation.map(|v| v.unwrap_or(0.0)),
                size: lane
                    .size
                    .map(|v| v.unwrap_or(lane.size[0].expect("initial X size"))),
                gravity: 0.0,
                colour: moly_law::particle::gradient::rgba8_to_float(lane.color),
                total_velocity: velocity,
                custom_data: [[0.0; 4]; 2],
            });
            partial_dts.push(timing[index].dt);
        }
    }
    // The actual rounded-span death/packing contract is installed below. It
    // must keep generated padding available until native cleanup has finished.
    system.pool.extend(particles);
    system.side.extend(sides);
    let live_newborns = simulate_birth_span(system, old_count, accepted, &partial_dts, ctx);
    system.pool.truncate(old_count + live_newborns);
    system.side.truncate(old_count + live_newborns);
    // CopyParticlesToUnalignedDst packs the newborns after the newborn death
    // scan. In a ring mode, once the pool exceeds maxParticles, Pause records
    // the death of the particle at the ring cursor and overwrites it, and Loop
    // swaps every newborn with the cursor, leaving the displaced particle in the
    // overflow span where it finishes its life without looping.
    let mut replaced = 0_u64;
    finish_births(
        &mut system.pool,
        &mut system.side,
        &mut system.ring_cursor,
        system.emitter.ring_buffer_mode,
        system.emitter.max_particles as usize,
        old_count,
        |_, _| replaced += 1,
    );
    system.died_total += replaced;
    *random = next;
    if let (Some(destination), Some(next)) = (shape_stream.as_mut(), next_shape) {
        **destination = next;
    }
    system.born_total += accepted as u64;
    system.full_total += batch.count as u64 - accepted as u64;
    Ok(())
}

/// Validate module/storage qualification without advancing any persistent RNG.
fn validate(system: &Runtime, random: &ModuleRandom) -> Result<InitialLaw, BirthRefused> {
    let law = validate_emitter(&system.emitter, random)?;
    shape_emitter_state(&system.emitter, system.geometry.shape_evidence())?;
    Ok(law)
}

fn validate_emitter(
    emitter: &EmitterParams,
    random: &ModuleRandom,
) -> Result<InitialLaw, BirthRefused> {
    match (emitter.shape_enabled, emitter.shape.as_ref()) {
        (Some(false), None) => {}
        (Some(true), Some(shape)) => {
            moly_law::particle::shape_birth::ShapeBirthLaw::from_params(shape).map_err(shape_refusal)?;
        }
        _ => {
            return Err(BirthRefused::Unsupported(
                "missing Shape enabled/configuration evidence",
            ))
        }
    }
    if emitter.inherit_velocity.is_some()
        || emitter.collision.is_some()
        || emitter.trails.is_some()
    {
        return Err(BirthRefused::Unsupported(
            "birth module/event owner not installed",
        ));
    }
    // An authored null child edge (no emitter, pointer 0/0) names no system
    // and schedules nothing. A real edge needs the child owner and its
    // command order, which this step does not install.
    if emitter
        .sub_emitters
        .iter()
        .any(|edge| edge.emitter.is_some() || !edge.source_pointer.is_authored_null())
    {
        return Err(BirthRefused::Unsupported(
            "sub-emitter child owner not installed",
        ));
    }
    if let Some(noise) = emitter.noise.as_ref() {
        NoiseLaw::from_params(noise)
            .map_err(|_| BirthRefused::Unsupported("unqualified Noise configuration"))?;
    }
    if !matches!(emitter.start.speed, moly_law::particle::MinMaxCurve::Constant(v) if v.is_finite())
        && !matches!(emitter.start.speed, moly_law::particle::MinMaxCurve::TwoConstants{min,max}
            if min.is_finite() && max.is_finite() && (max-min).is_finite() && ((max-min)+min).is_finite())
    {
        return Err(BirthRefused::Unsupported(
            "initial speed curve outside qualified subset",
        ));
    }
    if !matches!(
        emitter.simulation_space,
        SimulationSpace::Local | SimulationSpace::World
    ) {
        return Err(BirthRefused::Unsupported("custom simulation owner"));
    }
    let law = InitialLaw::from_params(&emitter.start, 0.0).map_err(BirthRefused::Initial)?;
    let mut scratch = *random;
    law.start_group_native(
        &mut scratch,
        InitialGroupInput {
            active_lanes: 4,
            storage_size_3d: emitter.start.size3d,
            storage_rotation_3d: emitter.start.rotation3d,
            birth_fraction: [1.0; 4],
            curve_time: [0.0; 4],
            context: InitialContext::Autonomous,
        },
    )
    .map_err(BirthRefused::Initial)?;
    Ok(law)
}

/// No-shape direction is independent of birth RNG. Check its finite endpoint
/// velocities before the ordinary slice mutates clocks or old particles.
fn validate_unshaped_owner(
    emitter: &EmitterParams,
    owner: &GlobalTransform,
) -> Result<(), BirthRefused> {
    if emitter.simulation_space != SimulationSpace::World {
        return Ok(());
    }
    if owner
        .to_matrix()
        .to_cols_array()
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err(BirthRefused::Unsupported("nonfinite birth owner"));
    }
    let direction = owner
        .affine()
        .transform_vector3(Vec3::Z)
        .normalize_or_zero()
        .to_array();
    let speeds = match emitter.start.speed {
        moly_law::particle::MinMaxCurve::Constant(v) => [v, v],
        moly_law::particle::MinMaxCurve::TwoConstants { min, max } => [min, (max - min) + min],
        _ => {
            return Err(BirthRefused::Unsupported(
                "initial speed curve outside qualified subset",
            ))
        }
    };
    if speeds
        .into_iter()
        .any(|speed| direction.iter().any(|v| !(v * speed).is_finite()))
    {
        return Err(BirthRefused::Unsupported(
            "nonfinite birth position or velocity",
        ));
    }
    Ok(())
}
