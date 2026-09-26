//! Explicit native birth commands. The graph/lifecycle owner supplies RNG and
//! timing; this entry never advances autonomous clocks or invents a seed.
use super::*;
use moly_law::particle::autonomous_emission::{
    AutonomousEmissionState, ConstantAutonomousEmission, ConstantDistanceEmission,
};
use moly_law::particle::{
    curve::CurveSampler,
    initial::{InitialContext, InitialGroupInput, InitialLaw},
    noise::NoiseLaw,
    random::ParticleRandom,
    schema::{SubEmitterParams, SubEmitterSourcePointer, SubEmitterTrigger},
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
    pub frame: FrameState,
    /// Parent side of the sub-emitter birth and death events, attached by the
    /// installer when the emitter has real birth or death edges; `None`
    /// otherwise.
    pub events: Option<super::sub_events::BirthEvents>,
    /// The child side when this system is an installed sub-emitter target;
    /// `None` for every other system.
    pub target: Option<super::child::ChildTarget>,
}

/// Per-system state the engine's per-frame update head keeps between frames.
/// The pending time (system seconds still to simulate, carried below the loop
/// threshold) is `Runtime::pending`, which every system's frame head keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameState {
    /// Emitter translation at the end of the last frame with a nonzero dt.
    pub previous_position: [f32; 3],
    /// Emitter velocity (translation change over the raw frame dt). Zero at
    /// construction; Play does not clear it.
    pub velocity: [f32; 3],
    /// Set at construction and by Play: the next frame takes its own
    /// translation as the previous one.
    pub reset_previous: bool,
    /// The system state's start delay word: zero at construction; Play with
    /// prewarm off writes the start delay's value (see
    /// [`super::play_start_delay`]). While it is not zero the frame head runs
    /// no emission over distance; every slice with the clock at zero on the
    /// update's entry counts it down (see `step_slice`).
    pub start_delay: f32,
}

impl Default for FrameState {
    fn default() -> Self {
        Self { previous_position: [0.0; 3], velocity: [0.0; 3], reset_previous: true, start_delay: 0.0 }
    }
}

/// The emitter velocity is refreshed only for a raw frame dt above this.
const MIN_VELOCITY_DT: f32 = f32::from_bits(0x38d1_b717);

impl FrameState {
    /// The frame state after `ParticleSystem.Simulate`'s restart branch:
    /// Play sets the emitter reset and does not clear the emitter velocity,
    /// which keeps `velocity`; a warm update of raw dt `warm_dt` then takes its
    /// own translation as the previous one, so it refreshes the velocity to
    /// zero when the dt is above the refresh threshold, and its end of update
    /// (a nonzero dt) stores that translation.
    pub(super) fn after_restart(velocity: [f32; 3], warm_dt: Option<f32>, translation: [f32; 3],
        start_delay: f32) -> Self {
        let mut frame = Self { velocity, start_delay, ..Self::default() };
        if let Some(dt) = warm_dt {
            if dt > MIN_VELOCITY_DT {
                frame.velocity = [0.0; 3];
            }
            if dt != 0.0 {
                frame.previous_position = translation;
                frame.reset_previous = false;
            }
        }
        frame
    }
}

/// Placement of the births of one command along the emitter's motion.
/// StartModules moves every World-space birth back by
/// (births_ahead + fraction) * (dt / speed) times the emitter velocity, after
/// Shape, start velocity and the newborn pre-simulation modules and before the
/// newborn integration; in Local space the velocity is masked to zero. The
/// elapsed time of a lane is dt * fraction - pending: the slices' time births
/// pass no pending time, the frame head's distance births the pending time
/// after the frame's sum.
#[derive(Clone, Copy, Debug)]
pub(super) struct BirthBacktrack {
    pub births_ahead: f32,
    pub emitter_velocity: [f32; 3],
    pub pending: f32,
}

/// Name-independent qualification of the native birth composition: autonomous
/// emission with a constant or two-constant rate and up to eight bursts with a
/// constant or two-constant count, the Initial
/// law, the qualified Shape configurations,
/// the qualified Noise subset and authored-null or real birth child edges
/// (their event owner is attached by the installer). The source update
/// route (ordinary versus procedural) is decided by the caller from the
/// exported system block, not here.
pub(super) fn qualify_emitter(emitter: &EmitterParams) -> Result<(), BirthRefused> {
    qualify(emitter, false)
}

/// The same for a system that runs its own per-frame update (not a
/// sub-emitter target): emission over distance driven by the emitter's
/// translation is qualified too. A target never takes that branch (the engine
/// marks it stopped every frame); its distance births come from its parent.
pub(super) fn qualify_frame_emitter(emitter: &EmitterParams) -> Result<(), BirthRefused> {
    qualify(emitter, true)
}

fn qualify(emitter: &EmitterParams, frame_head: bool) -> Result<(), BirthRefused> {
    let Some(emission) = emitter.emission.as_ref() else {
        return Err(BirthRefused::Unsupported("missing emission"));
    };
    if frame_head {
        let (_, distance) = ConstantAutonomousEmission::from_params_with_distance(
            &emitter.start_delay,
            emitter.duration,
            emitter.looping,
            emission,
        )
        .map_err(BirthRefused::Emission)?;
        if distance.is_some() {
            qualify_distance_composition(emitter)?;
        }
    } else {
        ConstantAutonomousEmission::from_params(
            &emitter.start_delay,
            emitter.duration,
            emitter.looping,
            emission,
        )
        .map_err(BirthRefused::Emission)?;
    }
    // A World-space birth is placed back along the emitter velocity; only the
    // Transform mode's velocity (translation change per frame) is transcribed.
    // Local space masks the velocity, so its mode is never read.
    if emitter.simulation_space == SimulationSpace::World && emitter.emitter_velocity_mode != Some(0) {
        return Err(BirthRefused::Unsupported(
            "World-space emitter velocity other than the Transform mode is not transcribed",
        ));
    }
    // The CollisionModule law is qualified inside; its scene is bound (or
    // refused) where the system is admitted, and a slice without the
    // installed module refuses (see `validate`).
    validate_emitter(emitter, &ModuleRandom::from_owner_seed(0))?;
    Ok(())
}

/// Emission over distance, as executed: the Transform-mode emitter velocity,
/// no first-Play warm (the warm's own update would take the branch), and
/// newborns whose negative elapsed time runs only through Initial gravity
/// with a constant modifier, RotationOverLifetime, a Local VelocityModule with
/// constant or two-constant linear, orbital, orbital-offset and radial terms
/// and a constant unit speed modifier (the orbital displacement over the
/// negative elapsed time turns the newborn back about the axes, in the
/// simulation space's own frame), ClampVelocity, RotationBySpeed and
/// CustomData. A distance birth takes the same pre-simulation chain as every
/// other birth, in the same order (Initial, the rotation-over-lifetime rate,
/// Velocity, ClampVelocity with its damping from the lane's |elapsed|, the
/// rotation-by-speed rate, CustomData), then the backtrack, the position
/// integration and one rotation integration with the summed rate over the
/// negative elapsed time. Every other module that reads the elapsed time, the
/// position or the newborn range is refused with it.
fn qualify_distance_composition(emitter: &EmitterParams) -> Result<(), BirthRefused> {
    use moly_law::particle::MinMaxCurve;
    let refuse = |reason| Err(BirthRefused::Unsupported(reason));
    if emitter.emitter_velocity_mode != Some(0) {
        return refuse("emission over distance: emitter velocity other than the Transform mode is not transcribed");
    }
    if emitter.prewarm {
        return refuse("emission over distance with a first-Play warm: the warm update's distance call is not transcribed");
    }
    if has_real_sub_emitter_edges(emitter) || emitter.collision.is_some() || emitter.trails.is_some()
        || emitter.noise.is_some() || emitter.force.is_some() || emitter.inherit_velocity.is_some()
    {
        return refuse("emission over distance: a module outside the executed newborn composition");
    }
    if !matches!(emitter.start.gravity_modifier, MinMaxCurve::Constant(v) if v.is_finite()) {
        return refuse("emission over distance: gravity modifier other than a constant");
    }
    if let Some(velocity) = &emitter.velocity_over_lifetime {
        let scalar = |curve: &MinMaxCurve| matches!(*curve, MinMaxCurve::Constant(v) if v.is_finite())
            || matches!(*curve, MinMaxCurve::TwoConstants { min, max } if min.is_finite() && max.is_finite());
        let orbital = velocity.orbital.iter().chain(&velocity.orbital_offset).chain([&velocity.radial]).all(scalar);
        if velocity.in_world_space
            || ![&velocity.x, &velocity.y, &velocity.z].into_iter().all(scalar)
            || !orbital
            || !matches!(velocity.speed_modifier, MinMaxCurve::Constant(v) if v == 1.0)
        {
            return refuse("emission over distance: VelocityModule outside the executed subset");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum BirthRefused {
    Unsupported(&'static str),
    InvalidTiming,
    Initial(moly_law::particle::initial::Refused),
    Emission(moly_law::particle::autonomous_emission::Refused),
    Collision(String),
}

/// One rendered frame of an installed system, as the engine's per-frame
/// update head runs it before its slices: refresh the emitter velocity from
/// the owner translation, scale the frame dt by the simulation speed, take
/// the slice length, skip the frame below the minimum step (only the velocity
/// and the end-of-frame translation change then), otherwise add the scaled
/// dt to the pending time and run every slice of the incremental loop. The
/// displacement of a skipped frame is lost, as in the engine. A stopped system
/// runs the same slices without emission. `slice_start` sees the system at the
/// start of every slice. Returns whether the update ran; a refused emission
/// over distance or slice ends the frame with its error.
pub(super) fn advance_frame(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    frame_dt: f32,
    stopped: bool,
    entry: moly_law::particle::frame_time::IncrementalEntry,
    ctx: &Context,
    slice_start: &mut dyn FnMut(&Runtime),
) -> Result<bool, BirthRefused> {
    let current = compose_to_world(system, ctx).translation().to_array();
    let frame = &mut state.frame;
    if frame.reset_previous {
        frame.previous_position = current;
        frame.reset_previous = false;
    }
    // Divided by the raw frame dt, not the speed-scaled one; a NaN dt keeps
    // the previous velocity.
    if frame_dt > MIN_VELOCITY_DT {
        let previous = frame.previous_position;
        frame.velocity = std::array::from_fn(|axis| (current[axis] - previous[axis]) / frame_dt);
    }
    let head = moly_law::particle::prewarm::FrameStep::new(
        frame_dt, system.emitter.simulation_speed, PLAYER_TIME);
    let result = if head.skips() {
        Ok(false)
    } else {
        // A backlog the bounded slice schedule refuses commits nothing: the
        // frame's time is dropped and the pending time stays as it was. The
        // engine has no such refusal. Counted and named, as the legacy head
        // does; the system is not retired.
        match head.plan_entry(system.pending, system.emitter.duration, entry) {
            Ok(plan) => {
                let pending = plan.remaining();
                let late = super::child::arms::on("distanceAfterSlices");
                // Update1b runs the head's emission over distance only while
                // the start delay word is zero.
                let delayed = state.frame.start_delay != 0.0 && !super::child::arms::on("distanceDuringDelay");
                let distance = if stopped || late || delayed { Ok(()) } else { emit_over_distance(system, state, head.scaled_dt,
                    pending, ctx) };
                let result = distance.and_then(|()| run_plan(system, state, plan, stopped, ctx, slice_start));
                let result = if late && !stopped && !delayed && result.is_ok() {
                    emit_over_distance(system, state, head.scaled_dt, pending, ctx)
                } else {
                    result
                };
                result.map(|()| true)
            }
            Err(error) => {
                system.refused_total += 1;
                error!(%error, effect=%system.effect, node=%system.node, "particle frame refused");
                Ok(false)
            }
        }
    };
    // End of the frame's update: a zero dt keeps the old translation; NaN
    // compares unequal to zero and replaces it.
    if frame_dt != 0.0 {
        state.frame.previous_position = current;
    }
    result
}

/// Emission over distance at the frame head, once per unskipped frame with the
/// whole scaled dt, after the pending sum and before the first slice; a
/// stopped system (and so every sub-emitter target) takes no such call. The
/// births are placed back along the masked emitter velocity with no births
/// ahead and carry the pending time after the sum, so their elapsed time and
/// age start negative and every slice of the frame then advances them as old
/// particles. Nothing is committed when the call or its births are refused.
fn emit_over_distance(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    scaled_dt: f32,
    pending: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    let Some(distance) = frame_distance_law(&system.emitter)? else {
        return Ok(());
    };
    let mut emission = state.emission;
    let batch = distance
        .emit(&mut emission, state.frame.velocity, scaled_dt, pending, system.playback_head,
            moly_law::particle::initial::initial_reciprocal)
        .map_err(BirthRefused::Emission)?;
    let emitter_velocity = if system.emitter.simulation_space == SimulationSpace::World {
        state.frame.velocity
    } else {
        [0.0; 3]
    };
    start_common(
        system,
        &mut state.initial,
        Some(&mut state.shape),
        // The arc clock a frame-head birth reads is not established here: a
        // Loop or PingPong cone is refused by name below, every other kernel
        // reads no clock.
        None,
        BirthBatch { count: batch.count, rate_count: batch.count, distribution: batch.distribution },
        batch.dt,
        batch.previous_normalized,
        batch.current_normalized,
        Some(BirthBacktrack { births_ahead: 0.0, emitter_velocity, pending: batch.pending }),
        None,
        ctx,
    )?;
    state.emission = emission;
    system.emission.to_emit_accumulator = emission.distribution.offset;
    Ok(())
}

/// The installed distance law of a system that runs its own per-frame update.
fn frame_distance_law(emitter: &EmitterParams) -> Result<Option<ConstantDistanceEmission>, BirthRefused> {
    let emission = emitter.emission.as_ref().ok_or(BirthRefused::Unsupported("missing emission"))?;
    let (_, distance) = ConstantAutonomousEmission::from_params_with_distance(
        &emitter.start_delay, emitter.duration, emitter.looping, emission)
        .map_err(BirthRefused::Emission)?;
    if distance.is_some() {
        qualify_distance_composition(emitter)?;
    }
    Ok(distance)
}

/// One incremental update of an installed system from its pending time and
/// slice length, as the engine's incremental loop takes them from the frame
/// head: every slice, then the rest below the loop threshold stays pending.
#[cfg(test)]
pub(super) fn run_incremental(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    pending: f32,
    step: f32,
    stopped: bool,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    let plan = moly_law::particle::prewarm::PrewarmPlan::from_incremental_input(
        pending, step, moly_law::particle::frame_time::IncrementalEntry::PerFrame, system.emitter.duration)
        .map_err(|_| BirthRefused::InvalidTiming)?;
    run_plan(system, state, plan, stopped, ctx, &mut |_| {})
}

fn run_plan(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    plan: moly_law::particle::prewarm::PrewarmPlan,
    stopped: bool,
    ctx: &Context,
    slice_start: &mut dyn FnMut(&Runtime),
) -> Result<(), BirthRefused> {
    let emitter_velocity = if system.emitter.simulation_space == SimulationSpace::World {
        state.frame.velocity
    } else {
        [0.0; 3]
    };
    run_slices(system, state, plan, stopped, emitter_velocity, ctx, slice_start)
}

/// The first-Play warm of an installed system: one ordinary update of the
/// warm length, so every slice records its sub-emitter events (births,
/// deaths, collisions) with the slice's pending time before its own
/// decrement, the commands' catch-up. Play resets the emitter motion head, so
/// the whole warm places its births with no emitter velocity. The rest below
/// the loop threshold stays pending.
pub(super) fn run_warm(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    plan: moly_law::particle::prewarm::PrewarmPlan,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    run_slices(system, state, plan, false, [0.0; 3], ctx, &mut |_| {})
}

fn run_slices(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    mut plan: moly_law::particle::prewarm::PrewarmPlan,
    stopped: bool,
    emitter_velocity: [f32; 3],
    ctx: &Context,
    slice_start: &mut dyn FnMut(&Runtime),
) -> Result<(), BirthRefused> {
    // Update1Incremental reads the clock once on entry; the start delay word
    // counts down only in an update entered with the clock at zero.
    let entry_clock = system.playback_head;
    for slice in plan.by_ref() {
        slice_start(system);
        let result = slice.map_err(|_| BirthRefused::InvalidTiming).and_then(|slice| {
            let accumulated = if super::child::arms::on("catchUpIsStep") {
                slice.duration
            } else if super::child::arms::on("catchUpAfterDecrement") {
                slice.remaining_before - slice.duration
            } else if super::child::arms::on("catchUpZero") {
                0.0
            } else {
                slice.remaining_before
            };
            step_slice(system, state, slice.duration, Some(accumulated), stopped,
                Some(BirthBacktrack { births_ahead: slice.births_ahead(), emitter_velocity, pending: 0.0 }), entry_clock, ctx)
        });
        if let Err(error) = result {
            // A refused slice ends the frame; the rest of its time is
            // dropped rather than carried as an unbounded backlog.
            system.pending = 0.0;
            return Err(error);
        }
    }
    system.pending = plan.remaining();
    Ok(())
}

/// One initialized ordinary slice: clock, existing-particle barrier, then the
/// scheduled births with their partial times. Shape and Noise ride the same
/// slice. Capacity is read only after old particles have completed their full
/// step and death compaction. No child graph or automatic seed/lifecycle
/// ownership is inferred by this explicit entry. It places births without the
/// emitter-motion backtrack, which is exact whenever the emitter velocity is
/// zero (as in the first-Play warm, whose update consumes the Play reset).
/// It carries no pending time, so a system with sub-emitter events refuses it.
pub(super) fn step_explicit(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    stopped: bool,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    let entry_clock = system.playback_head;
    step_slice(system, state, dt, None, stopped, None, entry_clock, ctx)
}

/// `accumulated` is the system time still to simulate when the slice starts
/// (this slice's own step not yet taken off); the sub-emitter events read it.
fn step_slice(
    system: &mut Runtime,
    state: &mut NativeBirthState,
    dt: f32,
    accumulated: Option<f32>,
    stopped: bool,
    backtrack: Option<BirthBacktrack>,
    entry_clock: f32,
    ctx: &Context,
) -> Result<(), BirthRefused> {
    validate(system, &state.initial)?;
    if let Some(reason) = system.custom_law.as_ref().and_then(|custom| custom.refused()) {
        return Err(BirthRefused::Unsupported(reason));
    }
    if has_real_sub_emitter_edges(&system.emitter) && state.events.is_none() {
        return Err(BirthRefused::Unsupported("sub-emitter event owner not installed"));
    }
    if state.events.is_some() && accumulated.is_none() {
        return Err(BirthRefused::Unsupported(
            "sub-emitter events need the slice's pending time",
        ));
    }
    if system.emitter.shape.is_none() {
        validate_unshaped_owner(&system.emitter, &compose_to_world(system, ctx))?;
    }
    let emission = system
        .emitter
        .emission
        .as_ref()
        .ok_or(BirthRefused::Unsupported("missing emission"))?;
    let (law, distance) = ConstantAutonomousEmission::from_params_with_distance(
        &system.emitter.start_delay,
        system.emitter.duration,
        system.emitter.looping,
        emission,
    )
    .map_err(BirthRefused::Emission)?;
    // Emission over distance belongs to the per-frame head, which also places
    // the births; an entry without that head (no placement) would drop it.
    if distance.is_some() && backtrack.is_none() {
        return Err(BirthRefused::Unsupported("emission over distance runs only from the per-frame update"));
    }
    // The start delay word, as Update1Incremental reads it per slice: Tick
    // takes the slice minus the word, and only while the word is below the
    // slice; the existing particles take the whole slice either way. After
    // them, and only when emission is not stopped, an update entered with the
    // clock at zero counts a positive word down by the slice (floored at
    // zero): while it has not run out nothing is emitted, and in the slice
    // where it runs out the births take its overshoot (none when it lands on
    // zero exactly).
    let delay = state.frame.start_delay;
    let tick = if delay < dt || super::child::arms::on("delayTicksWholeSlice") {
        Some(if super::child::arms::on("delayTicksWholeSlice") { dt } else { dt - delay })
    } else {
        None
    };
    let (birth_dt, next_delay) = if entry_clock == 0.0 && delay > 0.0 {
        let left = delay - dt;
        let overshoot = if super::child::arms::on("delayBirthsWholeSlice") { dt } else { -left };
        // FMAX with zero; the word and the slice are finite, and a difference
        // of equal values is +0, so the floor is exact.
        (Some(overshoot).filter(|b| left <= 0.0 && *b > 0.0), if left > 0.0 { left } else { 0.0 })
    } else {
        (Some(dt), delay)
    };
    let clock = law
        .prepare_slice_parts(system.playback_head, dt, tick, birth_dt, stopped)
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
    if !clock.stopped_after {
        state.frame.start_delay = next_delay;
    }
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
    // The kill pass inside the existing particles' simulation records the
    // death events: before the module batch after it, and before this
    // slice's emission draws, so the parent's emission word is still the one
    // the previous slice left. It does not test the stopped state either.
    let mut deaths = state.events.as_ref().filter(|events| events.records_deaths()).map(|_| Vec::new());
    simulate_existing(system, dt, ctx, deaths.as_mut());
    if let Some(reason) = system.custom_law.as_ref().and_then(|custom| custom.refused()) {
        return Err(BirthRefused::Unsupported(reason));
    }
    if let (Some(events), Some(deaths), Some(accumulated)) = (state.events.as_mut(), deaths.as_ref(), accumulated) {
        let word = if super::child::arms::on("deathWordAfterDraws") { pending.random.words[0] }
            else { state.emission.random.words[0] };
        events.record_deaths(system, deaths, false, 0, accumulated, word, ctx);
    }
    // Post-simulation modules. The collision call comes first, over the whole
    // pool with the slice dt; the parent's pending time and emission word are
    // read only by its collision sub-emitter commands.
    if let Some(mut collision) = system.collision.take() {
        let result = super::collision::post_simulation(system, &mut collision, dt,
            accumulated.unwrap_or(f32::NAN), state.emission.random.words[0]);
        system.collision = Some(collision);
        result.map_err(BirthRefused::Collision)?;
    }
    // Then the trail update over the whole pool with the slice dt, after the
    // simulation and death pass and before the size and sub-emitter calls.
    if let Some(trail) = system.trail.as_mut() {
        trail.update(&system.pool, &system.side, 0, system.pool.len(), dt, system.emitter.start.size3d);
    }
    if let (Some(events), Some(accumulated)) = (state.events.as_mut(), accumulated) {
        // After simulation and death, before this slice's emission draws: the
        // parent's emission word is still the one the previous slice left.
        events.record_existing(system, dt, accumulated, state.emission.random.words[0], ctx);
    }
    if let Some(batch) = batch {
        // Native order of ParticleSystem::Update1Incremental: the state Tick,
        // the pre-simulation, SimulateParticles and post-simulation passes
        // over the existing particles, then EmissionModule::EmitOverTime
        // writes the emission state and hands its two counts by value to
        // StartParticles, which leaves that state as written. The clock, the
        // existing particles and the emission state are therefore committed
        // here, before the birth, as natively; a refusal inside the birth
        // leaves exactly the state native holds on entering StartParticles.
        // The birth is staged and commits nothing when refused (no particle,
        // no Initial or Shape draw): only this slice's newborns are missing,
        // and the caller reports the refusal.
        state.emission = pending;
        system.emission.to_emit_accumulator = pending.distribution.offset;
        let emission_word = state.emission.random.words[0];
        let events = state.events.as_mut().zip(accumulated).map(|(events, accumulated)| NewbornEvents {
            events,
            accumulated,
            emission_word,
        });
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
            backtrack,
            events,
            ctx,
        )?;
    }
    Ok(())
}

/// The newborn groups' sub-emitter calls of one birth: the event owner, the
/// slice's pending time and the parent's emission word after this slice's
/// emission draws.
pub(super) struct NewbornEvents<'a> {
    events: &'a mut super::sub_events::BirthEvents,
    accumulated: f32,
    emission_word: u32,
}

/// An edge that is not authored null (no emitter, pointer 0/0).
pub(super) fn has_real_sub_emitter_edges(emitter: &EmitterParams) -> bool {
    emitter.sub_emitters.iter().any(|edge| !authored_null(edge))
}

fn authored_null(edge: &SubEmitterParams) -> bool {
    edge.emitter.is_none() && edge.source_pointer.is_authored_null()
}

/// A named birth or death edge with a complete, non-null source pointer.
pub(super) fn real_event_edge(edge: &SubEmitterParams) -> bool {
    matches!(edge.trigger, SubEmitterTrigger::Birth | SubEmitterTrigger::Death)
        && edge.emitter.as_ref().is_some_and(|name| !name.is_empty())
        && matches!(&edge.source_pointer, SubEmitterSourcePointer::Pointer { .. })
        && !edge.source_pointer.is_authored_null()
}

/// A named collision edge with a complete, non-null source pointer; only the
/// CollisionModule's call records its events.
fn real_collision_edge(edge: &SubEmitterParams) -> bool {
    edge.trigger == SubEmitterTrigger::Collision
        && edge.emitter.as_ref().is_some_and(|name| !name.is_empty())
        && matches!(&edge.source_pointer, SubEmitterSourcePointer::Pointer { .. })
        && !edge.source_pointer.is_authored_null()
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
        None,
        None,
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
        None,
        None,
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
/// carry the axis-of-rotation channel when the renderer is in Mesh render mode,
/// and the renderer reads it only when they do not use 3D rotation, so the
/// Shape law computes it only for that case, and the side data carries it to
/// the mesh transform about the axis. The runtime runs the
/// Initial module for every emitter; the admission decided the rule with the
/// module's serialized state and refused every Mesh system for which the two
/// decisions differ.
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
        uses_axis_of_rotation: evidence.mesh_renderer
            && crate::particle_runtime::uses_rotation_3d(emitter, true) != Some(true),
    }))
}

fn shape_refusal(refused: moly_law::particle::shape_birth::Refused) -> BirthRefused {
    use moly_law::particle::shape_birth::ExportGap;
    match refused {
        moly_law::particle::shape_birth::Refused::ExportSchema(ExportGap::ControlsVersion) => BirthRefused::Unsupported(
            "Shape controls are not the current export schema (an export check, not a ShapeModule member)",
        ),
        moly_law::particle::shape_birth::Refused::ExportSchema(ExportGap::Texture) => BirthRefused::Unsupported(
            "Shape export has no texture reference field (an export check, not a ShapeModule member); re-extract",
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
    backtrack: Option<BirthBacktrack>,
    events: Option<NewbornEvents<'_>>,
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
    let source_owner = source_owner_matrix(&owner);
    let mut next = *random;
    let mut next_shape = shape_stream.as_deref().copied();
    let mut particles = Vec::with_capacity(accepted.next_multiple_of(4));
    let mut sides = Vec::with_capacity(particles.capacity());
    let mut partial_dts = Vec::with_capacity(particles.capacity());
    // One quotient per command: the slice dt over the simulation speed.
    let time_per_step = backtrack.map(|_| dt / system.emitter.simulation_speed);
    let mut backtrack_scales = Vec::with_capacity(particles.capacity());
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
            // The group's axis-of-rotation channel goes into the side data
            // (below). The native mesh renderer reads that channel in two
            // places only. CalculateMeshParticleTransform (every render
            // alignment, in both the instanced and the CPU-vertex mesh job)
            // loads it only when the particle arrays do not use 3D rotation,
            // and then turns the mesh by its Z rotation about the normalised
            // axis (about +Y unless the axis's squared length exceeds a tiny
            // threshold); with 3D rotation it builds the rotation from the
            // three Euler angles and never loads the axis. BuildCustomData
            // copies it only for the MeshAxisOfRotation custom vertex stream.
            // Besides the renderer, the engine reads it only in the
            // sub-emitter record, when SubModule emits a child, and in the
            // script particle API and managed particle jobs, which the weather
            // objects do not call. ParticleSystem::AllocateParticleArrays,
            // which Update1b runs before any birth or draw of the frame, turns
            // 3D rotation on for the 3D start rotation of an enabled Initial
            // module, an enabled Shape module's align to direction, or the
            // separate axes of an enabled RotationOverLifetime or
            // RotationBySpeed module. Without a Shape block the Initial
            // module's +Z stands (it writes the axis from a cached vector the
            // module's constructor and reset set to +Z).
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
                emit_carry: [0.0; 2],
                animated: [0.0; 3],
                current_size: 0.0,
                axis: shaped
                    .as_ref()
                    .and_then(|shaped| shaped.axis_of_rotation)
                    .map_or([0.0, 0.0, 1.0], |axes| axes[index]),
            });
            // Elapsed time: dt * fraction, less the command's pending argument
            // (zero for the slices' time births, where it leaves the bits).
            partial_dts.push(match backtrack {
                Some(b) if !super::child::arms::on("elapsedWithoutPending") => timing[index].dt - b.pending,
                _ => timing[index].dt,
            });
            if let (Some(backtrack), Some(time_per_step)) = (backtrack, time_per_step) {
                let scale = (backtrack.births_ahead + timing[index].fraction) * time_per_step;
                if !scale.is_finite()
                    || backtrack.emitter_velocity.iter().any(|v| !(scale * v).is_finite())
                {
                    return Err(BirthRefused::Unsupported("nonfinite birth backtrack"));
                }
                backtrack_scales.push(scale);
            }
        }
    }
    // The actual rounded-span death/packing contract is installed below. It
    // must keep generated padding available until native cleanup has finished.
    system.pool.extend(particles);
    system.side.extend(sides);
    if let Some(trail) = system.trail.as_mut() {
        // InitialModule resets the ring of every lane of each new group.
        trail.rings.resize_with(system.pool.len(), Default::default);
        // One trail update per new four-lane group with dt 0, over the group's
        // accepted lanes: after the newborn pre-simulation modules (which
        // write velocity, rotation and custom data, never position or age)
        // and before the newborn age, backtrack and integration, so it
        // records the Shape position at age 0.
        for offset in (0..accepted).step_by(4) {
            trail.update(&system.pool, &system.side, old_count + offset,
                old_count + (offset + 4).min(accepted), 0.0, system.emitter.start.size3d);
        }
    }
    let backtrack = backtrack.map(|b| (backtrack_scales.as_slice(), b.emitter_velocity));
    simulate_birth_modules(system, old_count, &partial_dts, backtrack, ctx);
    if let Some(mut collision) = system.collision.take() {
        // One collision call per new four-lane group with its birth times,
        // after the group's modules, age and integration and before its
        // sub-emitter call and the newborn deaths. Slots past the group end
        // are not known here (the engine reads the next group's unfinished
        // lanes or stale memory there). The pending time and emission word
        // are read only by collision sub-emitter commands; a parent with such
        // edges always has its event owner (the slice refuses otherwise).
        let (pending, word) = events.as_ref().map_or((f32::NAN, 0), |e| (e.accumulated, e.emission_word));
        let mut result = Ok(());
        for offset in (0..accepted).step_by(4) {
            let dts = std::array::from_fn(|lane| partial_dts.get(offset + lane).copied().unwrap_or(f32::NAN));
            result = super::collision::newborn_block(system, &mut collision, old_count + offset,
                old_count + (offset + 4).min(accepted), dts, pending, word);
            if result.is_err() {
                break;
            }
        }
        system.collision = Some(collision);
        if let Err(reason) = result {
            // Nothing of this birth is committed: the newborns leave the pool
            // and neither stream advances.
            system.pool.truncate(old_count);
            system.side.truncate(old_count);
            if let Some(trail) = system.trail.as_mut() {
                trail.rings.truncate(old_count);
            }
            return Err(BirthRefused::Collision(reason));
        }
    }
    let mut events = events;
    if let Some(NewbornEvents { events, accumulated, emission_word }) = events.as_mut() {
        // One call per new four-lane group, after the group's pre-simulation
        // modules and its position and age update and before newborn deaths
        // are removed. The range stops at the accepted count; the time vector
        // is the group's four birth times.
        for offset in (0..accepted).step_by(4) {
            events.record_newborn(system, old_count, old_count + offset,
                old_count + (offset + 4).min(accepted), &partial_dts, *accumulated, *emission_word, ctx);
        }
    }
    // The newborn kill pass records its death events after every group's
    // call, with the same pending time and emission word.
    let mut deaths = events.as_ref().filter(|newborn| newborn.events.records_deaths()).map(|_| Vec::new());
    // The newborn lanes as the death pass finds them, for the laws that
    // follow the slots past the live count (CustomData, size).
    let lanes = follows_storage(system).then(|| system.pool[old_count..].to_vec());
    let live_newborns = kill_newborns(system, old_count, accepted, deaths.as_mut());
    if let (Some(NewbornEvents { events, accumulated, emission_word }), Some(deaths)) = (events.as_mut(), deaths.as_ref()) {
        events.record_deaths(system, deaths, true, old_count, *accumulated, *emission_word, ctx);
    }
    system.pool.truncate(old_count + live_newborns);
    system.side.truncate(old_count + live_newborns);
    // CopyParticlesToUnalignedDst packs the newborns after the newborn death
    // scan. In a ring mode, once the pool exceeds maxParticles, Pause records
    // the death of the particle at the ring cursor and overwrites it, and Loop
    // swaps every newborn with the cursor, leaving the displaced particle in the
    // overflow span where it finishes its life without looping.
    let maximum = system.emitter.max_particles as usize;
    let mut replaced = 0_u64;
    match system.trail.as_mut() {
        Some(trail) => {
            trail.rings.truncate(old_count + live_newborns);
            finish_births_with(&mut system.pool, &mut system.side, &mut trail.rings, &mut system.ring_cursor,
                system.emitter.ring_buffer_mode, maximum, old_count, |_, _| replaced += 1);
        }
        None => finish_births(&mut system.pool, &mut system.side, &mut system.ring_cursor,
            system.emitter.ring_buffer_mode, maximum, old_count, |_, _| replaced += 1),
    }
    system.died_total += replaced;
    if let Some(lanes) = lanes {
        storage_birth(system, old_count, &lanes, live_newborns).map_err(BirthRefused::Unsupported)?;
    }
    *random = next;
    if let (Some(destination), Some(next)) = (shape_stream.as_mut(), next_shape) {
        **destination = next;
    }
    system.born_total += accepted as u64;
    system.full_total += batch.count as u64 - accepted as u64;
    Ok(())
}

/// The runtime owner is reflected-X; module parameters are source Unity
/// coordinates. Convert its basis once, retaining native multiplication order.
pub(super) fn source_owner_matrix(owner: &GlobalTransform) -> [f32; 16] {
    let matrix = owner.to_matrix().to_cols_array();
    std::array::from_fn(|i| if (i % 4 == 0) ^ (i / 4 == 0) { -matrix[i] } else { matrix[i] })
}

/// Validate module/storage qualification without advancing any persistent RNG.
fn validate(system: &Runtime, random: &ModuleRandom) -> Result<InitialLaw, BirthRefused> {
    let law = validate_emitter(&system.emitter, random)?;
    shape_emitter_state(&system.emitter, system.geometry.shape_evidence())?;
    if system.emitter.collision.is_some() && system.collision.is_none() {
        return Err(BirthRefused::Unsupported("CollisionModule without its installed scene"));
    }
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
    // A system's own emission hands StartVelocity the emitter velocity with
    // the inheritance on only outside Local space, and the per-update module
    // acts only in World space: a Local system inherits nothing.
    if emitter.inherit_velocity.is_some() && emitter.simulation_space != SimulationSpace::Local {
        return Err(BirthRefused::Unsupported(
            "InheritVelocity outside Local space: the emitter-velocity inheritance of the system's own emission is not ported",
        ));
    }
    // A CollisionModule rides the native slices (its two call points are in
    // step_slice and start_common) inside the qualified subset.
    super::collision::qualify(emitter).map_err(BirthRefused::Collision)?;
    // A TrailModule rides the native slices (its two update points are in
    // step_slice and start_common) only inside the qualified subset.
    super::trails::qualify(emitter).map_err(BirthRefused::Unsupported)?;
    // An authored null child edge (no emitter, pointer 0/0) names no system
    // and schedules nothing. Real edges are taken only when every edge is a
    // named birth or death edge with a complete pointer: their events need
    // the event owner the installer attaches (each slice checks it is there),
    // and the installer resolves their children. Other triggers, and a list
    // mixing null and real entries, are not qualified.
    let real = emitter.sub_emitters.iter().filter(|edge| !authored_null(edge)).count();
    let event_edge = |edge: &SubEmitterParams| real_event_edge(edge)
        || (emitter.collision.is_some() && real_collision_edge(edge));
    if real > 0 && (real != emitter.sub_emitters.len() || !emitter.sub_emitters.iter().all(event_edge)) {
        return Err(BirthRefused::Unsupported(
            "sub-emitter edges other than real birth and death edges",
        ));
    }
    // A particle with a non-positive start lifetime is killed here without
    // its update (see simulate_range); what its death event would read then
    // was never executed. The trail keep-alive branch of the kill pass (a
    // per-particle trail that outlives its particle skips the kill and its
    // event) is refused with every trail that does not die with its
    // particles, so every admitted kill records.
    if emitter.sub_emitters.iter().any(|edge| edge.trigger == SubEmitterTrigger::Death && !authored_null(edge))
        && !match emitter.start.lifetime {
            moly_law::particle::MinMaxCurve::Constant(value) => value > 0.0,
            moly_law::particle::MinMaxCurve::TwoConstants { min, max } => min > 0.0 && max > 0.0,
            _ => false,
        }
    {
        return Err(BirthRefused::Unsupported(
            "death events of a start lifetime that is not a positive constant or two positive constants",
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
