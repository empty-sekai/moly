//! Explicit native birth commands. The graph/lifecycle owner supplies RNG and
//! timing; this entry never advances autonomous clocks or invents a seed.
use super::*;
use moly_law::particle::autonomous_emission::{
    AutonomousEmissionState, ConstantAutonomousEmission,
};
use moly_law::particle::{
    curve::CurveSampler,
    initial::{InitialContext, InitialGroupInput, InitialLaw},
    random::ParticleRandom,
    seed_owner::ModuleRandom,
    sub_emission::BirthBatch,
};

#[derive(Clone, Debug)]
pub(crate) struct NativeBirthState {
    pub owner: Option<moly_law::particle::seed_owner::SeedOwner>,
    pub initial: ModuleRandom,
    pub shape: ModuleRandom,
    pub emission: AutonomousEmissionState,
}

pub(super) fn qualifies(system: &Runtime) -> bool {
    qualifies_emitter(&system.emitter)
}

pub(super) fn qualifies_emitter(emitter: &EmitterParams) -> bool {
    let Some(emission) = emitter.emission.as_ref() else {
        return false;
    };
    // The new explicit Shape seam is verified independently. Full source
    // lifecycle/other module composition must qualify before automatic routing.
    emitter.shape_enabled == Some(false)
        && emitter.shape.is_none()
        && ConstantAutonomousEmission::from_params(
            &emitter.start_delay,
            emitter.duration,
            emitter.looping,
            emission,
        )
        .is_ok()
        && validate_emitter(emitter, &ModuleRandom::from_owner_seed(0)).is_ok()
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum BirthRefused {
    Unsupported(&'static str),
    InvalidTiming,
    Initial(moly_law::particle::initial::Refused),
    Emission(moly_law::particle::autonomous_emission::Refused),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Composition {
    Admitted,
    #[cfg(test)]
    SnowSourceProbe,
}

/// One initialized ordinary slice. Capacity is read only after old particles
/// have completed their full step and death compaction. No child graph or
/// automatic seed/lifecycle ownership is inferred by this explicit entry.
pub(super) fn step_explicit(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    stopped: bool,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    step_common(system, state, dt, stopped, ctx, Composition::Admitted)
}

/// Test-only current-JP source composition. Production admission is unchanged.
#[cfg(test)]
pub(super) fn step_source_snow_probe(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    let e = &system.emitter;
    if system.effect != "fx_env_sky_010_snow"
        || system.node != "root/snow_pt_01"
        || !e.prewarm
        || e.play_on_awake
        || e.auto_random_seed != Some(true)
        || e.shape.is_none()
        || e.noise.is_none()
        || e.sub_emitters.len() != 1
        || !e.sub_emitters[0].source_pointer.is_authored_null()
        || system.noise.as_ref().map(|n| n.owner_seed) != Some(71)
        || system.velocity_law.is_none()
        || system.rol.is_none()
        || system.size_law.is_none()
        || system.custom_law.is_none()
        || e.collision.is_some()
        || e.trails.is_some()
    {
        return Err(BirthRefused::Unsupported(
            "snow source probe configuration/owner",
        ));
    }
    step_common(system, state, dt, false, ctx, Composition::SnowSourceProbe)
}

fn step_common(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    stopped: bool,
    ctx: &Context,
    composition: Composition,
) -> Result<(), BirthRefused> {
    // Current Shape proof is the zero-time initialization boundary. Refuse
    // before advancing old particles/clocks until the full slice is replayed.
    if system.emitter.shape.is_some() && composition == Composition::Admitted {
        return Err(BirthRefused::Unsupported(
            "Shape full-slice module composition",
        ));
    }
    validate_scoped(system, &state.initial, composition)?;
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
    simulate_existing(system, dt, ctx);
    if let Some(batch) = batch {
        state.emission = pending;
        system.emission.to_emit_accumulator = pending.distribution.offset;
        start_common(
            system,
            &mut state.initial,
            Some(&mut state.shape),
            BirthBatch {
                count: batch.total,
                rate_count: batch.rate_count,
                distribution: batch.distribution,
            },
            batch.dt,
            batch.previous_normalized,
            batch.current_normalized,
            ctx,
            composition,
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
        batch,
        dt,
        previous_normalized,
        current_normalized,
        ctx,
        Composition::Admitted,
    )
}

/// Initial and Shape own independent streams, including all four padded
/// lanes. Explicit probe/lifecycle caller only; this does not admit new source
/// systems through the conservative automatic routing above.
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
        batch,
        dt,
        previous_normalized,
        current_normalized,
        ctx,
        Composition::Admitted,
    )
}

fn start_common(
    system: &mut Runtime,
    random: &mut ModuleRandom,
    mut shape_stream: Option<&mut ModuleRandom>,
    batch: BirthBatch,
    dt: f32,
    previous_normalized: f32,
    current_normalized: f32,
    ctx: &Context,
    composition: Composition,
) -> Result<(), BirthRefused> {
    let law = validate_scoped(system, random, composition)?;
    let shape_law = system
        .emitter
        .shape
        .as_ref()
        .map(|params| {
            moly_law::particle::shape_birth::ShapeBirthLaw::from_params(params)
                .map_err(|_| BirthRefused::Unsupported("unqualified native Shape configuration"))
        })
        .transpose()?;
    if shape_law.is_some() && shape_stream.is_none() {
        return Err(BirthRefused::Unsupported(
            "missing independent Shape stream",
        ));
    }
    if shape_law.is_some() && dt != 0.0 && composition == Composition::Admitted {
        return Err(BirthRefused::Unsupported(
            "Shape nonzero birth-step composition",
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
    let speed = CurveSampler::with_baking(&system.emitter.start.speed, false);
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
                    curve_time: std::array::from_fn(|lane| timing[lane].curve_time),
                    context: InitialContext::Autonomous,
                },
            )
            .map_err(BirthRefused::Initial)?;
        let shaped = shape_law
            .as_ref()
            .map(|shape| {
                shape
                    .sample_group(
                        next_shape
                            .as_mut()
                            .expect("validated independent Shape stream"),
                        source_owner,
                        system.emitter.simulation_space == SimulationSpace::World,
                    )
                    .map_err(|_| BirthRefused::Unsupported("unqualified native Shape owner/output"))
            })
            .transpose()?;
        for (index, lane) in group.lanes.into_iter().enumerate() {
            let sampled_speed = speed.evaluate(
                lane.curve_time,
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
    finish_births(
        &mut system.pool,
        &mut system.side,
        &mut system.ring_cursor,
        RingBufferMode::Disabled,
        system.emitter.max_particles as usize,
        old_count,
        |_, _| {},
    );
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
    validate_scoped(system, random, Composition::Admitted)
}
fn validate_scoped(
    system: &Runtime,
    random: &ModuleRandom,
    composition: Composition,
) -> Result<InitialLaw, BirthRefused> {
    validate_emitter_scoped(&system.emitter, random, composition)
}
fn validate_emitter(
    emitter: &EmitterParams,
    random: &ModuleRandom,
) -> Result<InitialLaw, BirthRefused> {
    validate_emitter_scoped(emitter, random, Composition::Admitted)
}
fn validate_emitter_scoped(
    emitter: &EmitterParams,
    random: &ModuleRandom,
    composition: Composition,
) -> Result<InitialLaw, BirthRefused> {
    match (emitter.shape_enabled, emitter.shape.as_ref()) {
        (Some(false), None) => {}
        (Some(true), Some(shape)) => {
            moly_law::particle::shape_birth::ShapeBirthLaw::from_params(shape)
                .map_err(|_| BirthRefused::Unsupported("unqualified native Shape configuration"))?;
        }
        _ => {
            return Err(BirthRefused::Unsupported(
                "missing Shape enabled/configuration evidence",
            ))
        }
    }
    if emitter.ring_buffer_mode != RingBufferMode::Disabled {
        return Err(BirthRefused::Unsupported(
            "newborn ring replacement composition",
        ));
    }
    if emitter.inherit_velocity.is_some()
        || (emitter.noise.is_some() && composition == Composition::Admitted)
        || emitter.collision.is_some()
        || emitter.trails.is_some()
        || (!emitter.sub_emitters.is_empty() && composition == Composition::Admitted)
    {
        return Err(BirthRefused::Unsupported(
            "birth module/event owner not installed",
        ));
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
