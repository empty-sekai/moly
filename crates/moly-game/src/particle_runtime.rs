//! Shared particle simulation, module evaluation and world-space presentation.
//! Domain adapters own selection, asset loading, instance anchors and teardown.
use bevy::prelude::*;
mod motion;
mod birth;
mod child;
mod sub_events;
mod trails;
mod collision;
pub(crate) mod collision_scene;
pub(crate) use collision::{collision_eligible, current_size_source_gate, is_planes};
pub(crate) use trails::{
    attach_owner as attach_trail_owner, draw_eligible as trail_draw_eligible, owner_ready as trail_owner_ready,
    write_mesh as write_trail_mesh, TrailOwner, TrailState,
};
pub(crate) use sub_events::{BirthEdge, BirthEvents, CollisionEdge, DeathEdge, EventEdges};
pub(crate) use child::{child_target_eligible, deliver_command, deliver_command_with, install_child_target, restart_child_target};
pub(crate) mod seed;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod motion_samples;
#[cfg(test)]
mod force_samples;
#[cfg(test)]
mod texture_sheet_samples;
#[cfg(test)]
mod sort_samples;
#[cfg(test)]
mod ring_samples;
#[cfg(test)]
mod gravity_samples;
#[cfg(test)]
mod custom_samples;
#[cfg(test)]
mod birth_samples;
#[cfg(test)]
mod source_birth_samples;
#[cfg(test)]
mod shape_owner_samples;
#[cfg(test)]
mod shape_birth_samples;
#[cfg(test)]
mod snow_full_samples;
#[cfg(test)]
mod frame_clock_samples;
#[cfg(test)]
mod noise_samples;
#[cfg(test)]
mod initial_colour_samples;
#[cfg(test)]
pub(crate) mod frame_samples;
#[cfg(test)]
mod sub_event_samples;
#[cfg(test)]
mod death_event_samples;
#[cfg(test)]
mod child_emit_samples;
#[cfg(test)]
mod collision_samples;
#[cfg(test)]
mod recursive_emit_samples;
#[cfg(test)]
mod warm_samples;
#[cfg(test)]
mod mesh_axis_samples;
#[cfg(test)]
mod custom_cache_samples;
#[cfg(test)]
mod procedural_samples;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::shape::{circle_base, cone_base, cone_volume, donut_position, hemisphere_position, single_sided_edge, sphere_position};
use moly_law::particle::{accumulate_rate, advance_lifetime, burst_check,
    birth_capacity, compact_with_sides_indexed, euler_rotate_deg, finish_births, finish_births_with,
    integrate, BurstOutcome, DragSize,
    EmissionState, EmitterParams, LimitVelocity, Particle,
    RingBufferMode, RotationOverLifetime, StepVerdict};
use moly_law::particle::noise::{NoiseLaw, NoiseState};
use moly_law::particle::death_event::DeathParent;
use moly_law::particle::prewarm::{FirstPlayWarm, Lifetime, PlayState, PrewarmPlan};
use crate::billboard::{Alignment, Quad, SizeClamp};
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

pub(crate) const PREWARM_STEP: f32 = 1.0 / 60.0;

/// The player's TimeManager. The JP 6.8.1 and CN 6.0.0 players serialize the
/// same one (Fixed Timestep 0.02, Maximum Allowed Timestep 1/3, time scale 1,
/// Maximum Particle Timestep 0.03, the engine's reset defaults), so one
/// constant serves both regions. The game assembly calls only Time getters, so
/// these hold for the whole session. TimeManager::Update clamps Time.deltaTime
/// to the Maximum Allowed Timestep, and GetTimeStep cuts each particle frame by
/// the Maximum Particle Timestep.
pub(crate) const PLAYER_TIME: moly_law::particle::prewarm::TimeManagerSnapshot =
    moly_law::particle::prewarm::TimeManagerSnapshot {
        fixed_timestep: f32::from_bits(0x3ca3_d70a),
        maximum_particle_timestep: f32::from_bits(0x3cf5_c28f),
        maximum_delta_time: f32::from_bits(0x3eaa_aaab),
    };

/// Maximum frame of the app's virtual clock, the engine's Time.maximumDeltaTime
/// in whole nanoseconds. The engine clamps a frame longer than
/// 0.3333333432674408 s (333,333,343.27 ns); the virtual clock clamps a frame
/// longer than this many nanoseconds, which is the same decision for every
/// whole-nanosecond frame, and the clamped frame rounds to the same float as the
/// engine's maximum.
pub(crate) const PLAYER_MAXIMUM_DELTA: std::time::Duration = std::time::Duration::from_nanos(333_333_343);

/// Time.deltaTime of a frame whose real (or virtual) duration is `elapsed`:
/// TimeManager::Update clamps it at Time.maximumDeltaTime, floors it at 1e-5 s
/// and otherwise rounds the elapsed seconds once to float.
pub(crate) fn source_delta_time(elapsed: std::time::Duration) -> f32 {
    moly_law::particle::frame_time::source_delta_time(elapsed, PLAYER_TIME)
}

/// Which engine clock a system's frame reads: ParticleSystem::BeginUpdate
/// takes Time.unscaledDeltaTime for a system with useUnscaledTime and
/// Time.deltaTime for any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameClock {
    Scaled,
    Unscaled,
}

impl FrameClock {
    pub(crate) fn from_use_unscaled_time(flag: bool) -> Self {
        if flag { Self::Unscaled } else { Self::Scaled }
    }
}

/// The player's one Time.unscaledDeltaTime holder, advanced once per frame from
/// the real clock (never from the clamped virtual clock).
#[derive(Resource, Default)]
pub(crate) struct UnscaledFrameClock {
    clock: moly_law::particle::frame_time::UnscaledClock,
    delta: f32,
}

impl UnscaledFrameClock {
    /// This frame's Time.unscaledDeltaTime.
    pub(crate) fn delta(&self) -> f32 {
        self.delta
    }
}

pub(crate) fn advance_unscaled_clock(real: Res<Time<Real>>, mut unscaled: ResMut<UnscaledFrameClock>) {
    let delta = unscaled.clock.advance(real.delta());
    unscaled.delta = delta;
}

/// One frame of a playing system: ParticleSystem::Update1b's frame part and
/// Update1Incremental. The frame's delta is scaled by FMAX(simulationSpeed, 0),
/// cut into equal pieces by GetTimeStep with the Maximum Particle Timestep and
/// added to the time left over from the previous update; a step below 1e-5 s
/// skips the whole update without keeping its time. Each slice runs one
/// ordinary update; what is left below 1e-6 s carries to the next frame.
/// `slice_start` sees the system at the start of every slice, where the
/// engine checks a non-looping system's time against its duration. Returns
/// whether the update ran (Update1b then stores the system's bounds). Each
/// slice runs through `step_frame`; its first refusal ends the frame and is
/// returned, and the weather host then retires the system (see `step_frame`).
/// A system with the native birth owner runs the same head and slices in
/// `birth::advance_frame`, which also refreshes the emitter velocity from the
/// owner translation, runs the emission over distance once before the slices
/// and places World births back along the emitter motion. A sub-emitter target
/// runs it stopped: the engine marks every target stopped each frame, so its
/// clock and particles advance and its births come only from its parents'
/// commands. `emitting` false is Stop.
pub(crate) fn advance_frame(system: &mut Runtime, dt: f32, emitting: bool, ctx: &Context,
    slice_start: impl FnMut(&Runtime)) -> Result<bool, String> {
    advance_frame_entry(system, dt, emitting, ctx, slice_start,
        moly_law::particle::frame_time::IncrementalEntry::PerFrame)
}

/// [`advance_frame`] for an update entered as `entry`.
fn advance_frame_entry(system: &mut Runtime, dt: f32, emitting: bool, ctx: &Context,
    mut slice_start: impl FnMut(&Runtime), entry: moly_law::particle::frame_time::IncrementalEntry)
    -> Result<bool, String> {
    if let Some(mut native) = system.native_birth.take() {
        let stopped = !emitting || native.target.is_some();
        let result = birth::advance_frame(system, &mut native, dt, stopped, entry, ctx, &mut slice_start);
        system.native_birth = Some(native);
        let result = result.map_err(|error| format!("{error:?}"))
            .and_then(|ran| if ran { end_of_update_size(system).map(|()| ran) } else { Ok(ran) });
        return result.map_err(|error| {
            system.refused_total += 1;
            error
        });
    }
    use moly_law::particle::frame_time::{frame_step_entry, FrameStep};
    match frame_step_entry(system.pending, dt, system.emitter.simulation_speed, PLAYER_TIME, system.emitter.duration,
        entry) {
        Ok(FrameStep::Skipped) => Ok(false),
        Ok(FrameStep::Slices(mut slices)) => {
            let mut refused = None;
            for slice in slices.by_ref() {
                slice_start(system);
                match slice {
                    Ok(slice) => {
                        if let Err(reason) = step_frame(system, slice.duration, ctx, emitting) {
                            refused = Some(reason);
                            break;
                        }
                    }
                    Err(error) => {
                        system.refused_total += 1;
                        error!(%error, effect=%system.effect, node=%system.node, "particle frame slice refused");
                        break;
                    }
                }
            }
            system.pending = slices.remaining();
            match refused {
                Some(reason) => Err(reason),
                None => Ok(true),
            }
        }
        Err(error) => {
            system.refused_total += 1;
            error!(%error, effect=%system.effect, node=%system.node, "particle frame refused");
            Ok(false)
        }
    }
}

/// Native update route the source selects for a system. Ordinary systems
/// advance through the incremental Update1 path this runtime transcribes.
/// Procedural systems (DetermineSupportsProcedural true) take the same
/// incremental update every frame; only their warm (Play's, Update flags 3,
/// and a script Simulate restart's, flags 2) evaluates them from time, which
/// is not transcribed, so a system that warms that way stays on the legacy
/// step. `Undecided` names the control the exported block cannot settle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceRoute {
    Ordinary,
    Procedural,
    Undecided(String),
}

/// Transcription of the native DetermineSupportsProcedural decision over the
/// exported system block. The native body reads fixed fields and has no side
/// effects, so the order of the tests does not matter: one failing control makes
/// the route ordinary, and only a block passing all of them is procedural. An
/// input the export does not carry makes the verdict undecided, never a default.
/// Curve validity follows IsValidPolynomialCurve: no keys is valid; otherwise the
/// first key must sit at time 0 or the pre-wrap mode be 2 or more, the last key
/// at time 1 or the post-wrap mode be 2 or more, and the segment count (one per
/// key gap plus one per clamped end) must be fewer than nine.
pub(crate) fn source_route(system: &serde_json::Value) -> SourceRoute {
    use serde_json::Value;
    fn number(value: &Value) -> Option<f64> {
        value.as_f64().or_else(|| match value.as_str() {
            Some("Infinity") => Some(f64::INFINITY),
            Some("-Infinity") => Some(f64::NEG_INFINITY),
            _ => None,
        })
    }
    // Upper lane of a serialized MinMaxCurve: the constant, the two-constant
    // maximum, or the curve multiplier.
    fn upper_scalar(curve: Option<&Value>) -> Option<f64> {
        let curve = curve?;
        match curve.get("mode").and_then(Value::as_str)? {
            "constant" => number(curve.get("value")?),
            "twoConstants" => number(curve.get("max")?),
            "curve" | "twoCurves" => curve.get("multiplier").and_then(number),
            _ => None,
        }
    }
    // One curve lane: its keys and, when exported, its wrap modes.
    fn lane_valid(curve: &Value, keys: &str, pre: &str, post: &str) -> Option<bool> {
        let keys = curve.get(keys)?.as_array()?;
        if keys.is_empty() {
            return Some(true);
        }
        let time = |key: &Value| key.get("time").and_then(Value::as_f64);
        let first_open = time(&keys[0])? != 0.0;
        let last_open = time(&keys[keys.len() - 1])? != 1.0;
        let wrap = |name: &str| curve.get(name).and_then(Value::as_u64);
        if first_open && wrap(pre)? < 2 {
            return Some(false);
        }
        if last_open && wrap(post)? < 2 {
            return Some(false);
        }
        Some(keys.len() - 1 + usize::from(first_open) + usize::from(last_open) < 9)
    }
    fn curve_valid(curve: Option<&Value>) -> Option<bool> {
        let curve = curve?;
        match curve.get("mode").and_then(Value::as_str)? {
            "constant" | "twoConstants" => Some(true),
            "curve" => lane_valid(curve, "keys", "preInfinity", "postInfinity"),
            "twoCurves" => {
                if !lane_valid(curve, "maxKeys", "maxPreInfinity", "maxPostInfinity")? {
                    return Some(false);
                }
                lane_valid(curve, "minKeys", "minPreInfinity", "minPostInfinity")
            }
            _ => None,
        }
    }
    let mut reasons: Vec<String> = Vec::new();
    let mut undecided: Vec<String> = Vec::new();
    fn curves(label: &str, block: &Value, axes: &[&str], reasons: &mut Vec<String>, undecided: &mut Vec<String>) {
        for axis in axes {
            match curve_valid(block.get(*axis)) {
                Some(false) => reasons.push(format!("{label} {axis} polynomial")),
                None => undecided.push(format!("{label} {axis} not decidable from the export")),
                Some(true) => {}
            }
        }
    }
    let Some(enabled) = system.pointer("/sourceModules/enabled").and_then(Value::as_array) else {
        return SourceRoute::Undecided("sourceModules not exported".into());
    };
    let enabled: Vec<&str> = enabled.iter().filter_map(Value::as_str).collect();
    let has = |module: &str| enabled.contains(&module);
    match system.get("simulationSpace").and_then(Value::as_str) {
        Some("Local") => {}
        Some(_) => reasons.push("simulationSpace".into()),
        None => undecided.push("simulationSpace not exported".into()),
    }
    match system.get("stopAction").and_then(Value::as_i64) {
        Some(0) => {}
        Some(_) => reasons.push("stopAction".into()),
        None => undecided.push("stopAction not exported".into()),
    }
    // Read whether or not the Emission module is enabled; the export carries it
    // outside the emission block for exactly that reason.
    let distance = system.get("emissionRateOverDistance")
        .or_else(|| system.pointer("/emission/rateOverDistance"));
    match upper_scalar(distance) {
        Some(rate) if rate == 0.0 => {}
        Some(_) => reasons.push("rateOverDistance".into()),
        None => undecided.push("rateOverDistance not exported".into()),
    }
    for module in ["ExternalForcesModule", "ClampVelocityModule", "RotationBySpeedModule",
        "CollisionModule", "TriggerModule", "SubModule", "NoiseModule"] {
        if has(module) { reasons.push(module.into()); }
    }
    // Only a per-particle trail fails the test; ribbon trails stay procedural.
    // The Lights module is not read by the decision.
    if has("TrailModule") {
        match system.pointer("/trails/mode").and_then(Value::as_str) {
            Some("perParticle") => reasons.push("TrailModule perParticle".into()),
            Some(_) => {}
            None => undecided.push("trails.mode not exported".into()),
        }
    }
    // Gravity and lifetime are read only while the Initial module is enabled.
    if has("InitialModule") {
        match system.pointer("/start/gravityModifier/mode").and_then(Value::as_str) {
            Some("constant") => {}
            Some(_) => reasons.push("gravityModifier not constant".into()),
            None => undecided.push("gravityModifier not exported".into()),
        }
        match upper_scalar(system.pointer("/start/lifetime")) {
            Some(value) if value == f64::INFINITY => reasons.push("startLifetime +Infinity".into()),
            Some(_) => {}
            None => undecided.push("startLifetime not exported".into()),
        }
    }
    if system.get("shapeEnabled").and_then(Value::as_bool) == Some(true) {
        match system.get("shape").filter(|v| v.is_object()) {
            None => undecided.push("shape not exported".into()),
            Some(shape) => {
                let kind = shape.get("type").and_then(Value::as_str);
                let mode = |key: &str| shape.get(key).and_then(Value::as_str);
                match kind {
                    None => undecided.push("shape type not exported".into()),
                    Some("Cone" | "ConeVolume" | "Circle" | "Donut") => match mode("arcMode") {
                        Some("Random") => {}
                        Some(_) => reasons.push("shape arcMode".into()),
                        None => undecided.push("shape arcMode not exported".into()),
                    },
                    Some("SingleSidedEdge") => match mode("radiusMode") {
                        Some("Random") => {}
                        Some(_) => reasons.push("shape radiusMode".into()),
                        None => undecided.push("shape radiusMode not exported".into()),
                    },
                    Some(_) => {}
                }
            }
        }
    }
    if has("RotationModule") {
        match system.get("rotationOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("rotationOverLifetime not exported".into()),
            Some(rotation) => match rotation.get("separateAxes").and_then(Value::as_bool) {
                None => undecided.push("rotation separateAxes not exported".into()),
                Some(separate) => {
                    let axes: &[&str] = if separate { &["curve", "x", "y"] } else { &["curve"] };
                    curves("rotation", rotation, axes, &mut reasons, &mut undecided);
                }
            },
        }
    }
    if has("VelocityModule") {
        match system.get("velocityOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("velocityOverLifetime not exported".into()),
            Some(velocity) => {
                curves("velocity", velocity, &["x", "y", "z"], &mut reasons, &mut undecided);
                for key in ["orbitalX", "orbitalY", "orbitalZ", "radial"] {
                    match upper_scalar(velocity.get(key)) {
                        Some(value) if value == 0.0 => {}
                        Some(_) => reasons.push(format!("velocity {key}")),
                        None => undecided.push(format!("velocity {key} not exported")),
                    }
                }
            }
        }
    }
    if has("ForceModule") {
        match system.get("forceOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("forceOverLifetime not exported".into()),
            Some(force) => {
                curves("force", force, &["x", "y", "z"], &mut reasons, &mut undecided);
                match force.get("randomizePerFrame").and_then(Value::as_bool) {
                    Some(true) => reasons.push("force randomizePerFrame".into()),
                    Some(false) => {}
                    None => undecided.push("force randomizePerFrame not exported".into()),
                }
            }
        }
    }
    if !reasons.is_empty() {
        SourceRoute::Ordinary
    } else if !undecided.is_empty() {
        SourceRoute::Undecided(undecided.join(", "))
    } else {
        SourceRoute::Procedural
    }
}

/// First-Play prewarm. The warm length and the starting clock are the native
/// ones (ComputePrewarmStartParameters, then Update1b's speed scale). A system
/// with an installed native birth owner advances through the native
/// incremental slice schedule (1 s / 0.2 s / GetTimeStep pieces, partial birth
/// times per particle). The legacy step births whole-slice cohorts, so it
/// covers the same system time in PREWARM_STEP pieces instead. Play only warms
/// a looping prewarm system; the caller owns that gate.
pub(crate) fn prewarm_first_play(system: &mut Runtime, ctx: &Context) -> Result<(), &'static str> {
    if let Some(mut state) = system.native_birth.take() {
        let result = prewarm_native(system, &mut state, ctx);
        system.native_birth = Some(state);
        return result.map(|()| later_play(system));
    }
    let warm = FirstPlayWarm::from_source(
        first_play_lifetime(&system.emitter)?, PLAYER_TIME, first_play_state(system))?;
    system.playback_head = warm.initial_clock;
    if warm.initial_clock != 0.0 {
        // The window opens mid-cycle: no zero-time burst precedes it.
        system.emission_started = true;
        system.previous_head = warm.initial_clock;
    }
    let mut remaining = warm.total;
    while remaining > 0.0 {
        let step = remaining.min(PREWARM_STEP);
        simulate(system, step, ctx);
        remaining -= step;
    }
    later_play(system);
    Ok(())
}

/// The effector plays every child system more than once within one call (the
/// managed recursion and the effector's own child array both reach it). A later
/// Play finds live particles, so it neither resets seeds nor warms again; it
/// only returns the playback clock, the pending time and the loop count to zero.
/// A warm that left no particle alive would be reset and warmed again by that
/// Play; this runtime does not repeat the warm and says so.
fn later_play(system: &mut Runtime) {
    if system.pool.is_empty() {
        warn!(effect=%system.effect, node=%system.node,
            "first-Play warm left no particle alive; the later Play's reset and second warm are not repeated");
        return;
    }
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.pending = 0.0;
}

/// `ParticleSystem.Play` on a system that still holds particles (a host's
/// Play after Stop): as the later Play above, it keeps the seeds and does not
/// warm; the clock, the pending time and the loop count return to zero, and
/// the native frame head takes its next frame's translation as the previous
/// one (Play's emitter reset).
pub(crate) fn later_play_with_particles(system: &mut Runtime) {
    later_play(system);
    let delay = play_start_delay(&system.emitter);
    if let Some(native) = system.native_birth.as_mut() {
        native.frame.reset_previous = true;
        // The same Play writes the start delay word again.
        if let Some(delay) = delay {
            native.frame.start_delay = delay;
        }
    }
}

/// `ParticleSystem.Play()` on a system its host stopped (Stop set the restart
/// flag). With particles alive it is [`later_play_with_particles`]: seeds and
/// emission carry stay. With none alive the same Play also resets the seeds
/// (an automatic owner takes the next shared-manager word), which re-expands
/// the module streams and zeroes the emission carry, as the first Play did.
/// Neither branch clears the emitter velocity; both set the emitter reset, so
/// the next frame takes its own translation as the previous one and its
/// emission over distance sees no motion. A CollisionModule system with none
/// alive is refused unchanged: its law is re-installed with the reset and the
/// host passes no ground scene here. A looping prewarm system's second warm
/// is the host's (the reset leaves it unwarmed).
pub(crate) fn play_after_stop(system: &mut Runtime, seeds: &mut seed::SystemSeedManager, route: &SourceRoute,
    edges: Option<EventEdges>) -> Result<(), String> {
    if !system.pool.is_empty() {
        later_play_with_particles(system);
        return Ok(());
    }
    if system.emitter.collision.is_some() && system.native_birth.is_some() {
        return Err("CollisionModule: Play's seed reset re-installs the collision law without its ground scene".into());
    }
    let velocity = system.native_birth.as_ref().map(|native| native.frame.velocity);
    // Play does not clear the emit replays a procedural warm left.
    let replays = system.native_birth.as_mut().map(|native| std::mem::take(&mut native.replays));
    reset_for_first_play(system);
    install_native_birth(system, seeds, route, None).map_err(|error| format!("source seed owner unavailable: {error}"))?;
    if let Some(native) = system.native_birth.as_mut() {
        if let Some(velocity) = velocity {
            native.frame.velocity = velocity;
        }
        if let Some(replays) = replays {
            native.replays = replays;
        }
        if let Some(edges) = edges {
            native.events = Some(BirthEvents::with_edges(edges));
        }
    }
    Ok(())
}

/// `ParticleSystem.randomSeed = value` (after its job sync): the system
/// becomes a manual owner with `value`, useAutoRandomSeed going false even
/// when the seed is unchanged, and nothing is reset: the live streams keep
/// the words of the last reset, and the next seed reset (a Play with no
/// particle alive, or a first Play) expands `value` without a shared-manager
/// draw. The installed owners mirror the read-only state the setter writes:
/// the birth owner, and Noise, which reads that state's seed.
pub(crate) fn set_random_seed(system: &mut Runtime, value: u32) {
    system.emitter.random_seed = Some(value);
    system.emitter.auto_random_seed = Some(false);
    if let Some(owner) = system.native_birth.as_mut().and_then(|native| native.owner.as_mut()) {
        owner.set_manual_seed(value);
    }
    if let Some(noise) = system.noise.as_mut() {
        noise.owner.set_manual_seed(value);
        noise.owner_seed = value;
    }
}

/// The start delay word ParticleSystem::Play writes when it restarts a
/// stopped system: with prewarm on it writes nothing (the word keeps its
/// construction zero), otherwise the start delay curve evaluated at time zero
/// with a random factor from the system seed's hash, which for a constant is
/// its scalar. `None` for a random delay (the seed hash is not transcribed).
pub(crate) fn play_start_delay(emitter: &EmitterParams) -> Option<f32> {
    if emitter.prewarm {
        return Some(0.0);
    }
    match emitter.start_delay {
        moly_law::particle::MinMaxCurve::Constant(value) if value.is_finite() && value >= 0.0 => Some(value),
        _ => None,
    }
}

/// Whether the system's Play leaves a start delay word other than zero.
pub(crate) fn has_start_delay(emitter: &EmitterParams) -> bool {
    play_start_delay(emitter) != Some(0.0)
}

/// `ParticleSystem.Clear` of the particles: the live count returns to zero.
/// The laws that follow the engine's storage (a CustomData or size law with
/// its slot tail) keep the cleared particles' slots, as the storage does.
pub(crate) fn clear_particles(system: &mut Runtime) {
    // Clear also empties the state's emit replays.
    if let Some(native) = system.native_birth.as_mut() {
        native.replays.clear();
    }
    if let Some(custom) = system.custom_law.as_mut() {
        custom.clear(&system.pool);
    }
    if let Some(calls) = system.size_law.as_mut().and_then(|size| size.calls_mut()) {
        calls.clear(&system.pool);
    }
    system.pool.clear();
    system.side.clear();
}

/// `ParticleSystem.Play` on a system that holds no particle: it plays as at
/// its first Play. The birth owner goes (the host installs it again, with the
/// seed reset), the clock, the pending time and the emission state return to
/// zero and the first-Play warm runs again.
pub(crate) fn reset_for_first_play(system: &mut Runtime) {
    system.native_birth = None;
    system.noise = None;
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.pending = 0.0;
    system.emission_started = false;
    system.emission = EmissionState::default();
    system.ring_cursor = 0;
    system.prewarmed = false;
}

/// The native first-Play slice schedule of an installed ordinary system.
pub(crate) fn first_play_plan(system: &Runtime) -> Result<PrewarmPlan, &'static str> {
    PrewarmPlan::from_source(first_play_lifetime(&system.emitter)?, PLAYER_TIME, first_play_state(system))
}

fn prewarm_native(system: &mut Runtime, state: &mut birth::NativeBirthState, ctx: &Context)
    -> Result<(), &'static str> {
    if state.procedural {
        return prewarm_procedural(system, state, ctx);
    }
    let plan = first_play_plan(system)?;
    system.playback_head = plan.initial_clock();
    // One ordinary update of the warm length: its slices record the
    // sub-emitter events with their pending time. The rest below the loop
    // threshold stays pending. The emitter reset set by Play is left for the
    // first frame, which takes its own translation.
    if let Err(error) = birth::run_warm(system, state, plan, ctx) {
        system.refused_total += 1;
        error!(?error, effect=%system.effect, node=%system.node, "native prewarm slice refused");
        return Err("native prewarm slice refused");
    }
    if let Err(reason) = end_of_update_size(system) {
        system.refused_total += 1;
        error!(%reason, effect=%system.effect, node=%system.node, "native prewarm size call refused");
        return Err("native prewarm size call refused");
    }
    Ok(())
}

/// Play's warm on the procedural route: Compute as for the ordinary warm,
/// then `Update` with flags 3 (the fixed-step slices recording emit replays,
/// then `UpdateProcedural`). Its Update1Incremental ends with the size call
/// over the particles present then, none at the first Play.
fn prewarm_procedural(system: &mut Runtime, state: &mut birth::NativeBirthState, ctx: &Context)
    -> Result<(), &'static str> {
    let warm = FirstPlayWarm::from_source(first_play_lifetime(&system.emitter)?, PLAYER_TIME, first_play_state(system))?;
    let plan = moly_law::particle::prewarm::PrewarmPlan::procedural(warm, PLAYER_TIME, system.emitter.duration)?;
    system.playback_head = plan.initial_clock();
    if let Err(error) = birth::run_warm_procedural(system, state, plan, ctx) {
        system.refused_total += 1;
        error!(?error, effect=%system.effect, node=%system.node, "procedural prewarm refused");
        return Err("procedural prewarm refused");
    }
    Ok(())
}

/// The end of `ParticleSystem::Update1Incremental` for a size law that
/// follows the engine's calls: after the slice loop, one `SizeModule::Update`
/// over the whole live storage `[0, count)`. The engine takes this call only
/// when no module read the size during the slices (drag by size, collision,
/// lights by size, trails, trigger); such a system does not get this law
/// ([`size_storage_eligible`]). The renderer reads what it stores.
fn end_of_update_size(system: &mut Runtime) -> Result<(), String> {
    let Some(calls) = system.size_law.as_mut().and_then(|law| law.calls_mut()) else { return Ok(()); };
    let lanes: Vec<moly_law::particle::size::SizeLane> = system.pool.iter().zip(&system.side)
        .map(|(particle, side)| moly_law::particle::size::SizeLane {
            age_percent: particle.age_percent, seed: side.seed, start: side.size })
        .collect();
    calls.pass(&lanes, 0, lanes.len());
    match calls.refused() {
        Some(reason) => Err(reason.to_owned()),
        None => Ok(()),
    }
}

/// Whether a law of this system follows the engine's storage past the live
/// count (a CustomData or size law), so that a birth reports its lanes.
pub(super) fn follows_storage(system: &Runtime) -> bool {
    system.custom_law.as_ref().is_some_and(|custom| custom.tracks_storage())
        || system.size_law.as_ref().is_some_and(|size| size.calls().is_some())
}

/// A birth's storage operation, told to every law that follows the storage:
/// `lanes` are the newborn lanes after old live particles `old` as the
/// newborn death pass finds them, `live` the survivors; the runtime's pool
/// holds them packed. Returns the first law's refusal.
pub(super) fn storage_birth(system: &mut Runtime, old: usize, lanes: &[Particle], live: usize)
    -> Result<(), &'static str> {
    if let Some(custom) = system.custom_law.as_mut().filter(|custom| custom.tracks_storage()) {
        custom.birth(old, lanes, live, &system.pool[old..]);
        if let Some(reason) = custom.refused() {
            return Err(reason);
        }
    }
    if let Some(calls) = system.size_law.as_mut().and_then(|size| size.calls_mut()) {
        calls.birth(old, lanes, live, &system.pool[old..]);
        if let Some(reason) = calls.refused() {
            return Err(reason);
        }
    }
    Ok(())
}

fn first_play_lifetime(emitter: &EmitterParams) -> Result<Lifetime, &'static str> {
    match emitter.start.lifetime {
        moly_law::particle::MinMaxCurve::Constant(v) => Ok(Lifetime::Constant(v)),
        moly_law::particle::MinMaxCurve::TwoConstants { min, max } => Ok(Lifetime::TwoConstants { min, max }),
        // Curve lifetimes go through CalculateCurveRangesValue, not transcribed.
        _ => Err("curve start lifetime: prewarm range not transcribed"),
    }
}

fn first_play_state(system: &Runtime) -> PlayState {
    let e = &system.emitter;
    PlayState {
        elapsed: 0.0,
        live_count: system.pool.len(),
        // Awake/EmitEffect Play reaches native Play(true); the managed
        // withChildren mapping itself is not separately replayed.
        native_play_bool_argument: true,
        world_playing: true,
        // Only an installed ordinary system reaches the native slice plan;
        // the shared Compute/Update1b arithmetic does not read this flag.
        ordinary_incremental: true,
        sub_emitter_max_lifetime: system.sub_emitter_max_lifetime,
        prewarm: e.prewarm,
        looping: e.looping,
        simulation_speed: e.simulation_speed,
        duration: e.duration,
    }
}
/// 三类锚：粒子所在的预制件挂在天空视图的效果根、场景相机的效果根，还是站点视图下。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectKind {
    Sky,
    Camera,
    Site,
}

#[derive(Clone)]
pub(crate) enum Geometry {
    Billboard { alignment: Alignment, clamp: SizeClamp, pivot: [f32; 3] },
    Mesh(crate::particle_geometry::MeshDraw),
    SourceBillboard(crate::source_billboard::Draw),
}

/// Source emitter state the native Shape boundary reads besides the Shape
/// block: the authored MainModule scaling mode and whether the renderer is in
/// Mesh render mode (which allocates the axis-of-rotation channel; the
/// renderer reads it only without 3D rotation, `uses_rotation_3d`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapeEmitterEvidence {
    pub(crate) scaling: crate::particle_geometry::Scaling,
    pub(crate) mesh_renderer: bool,
}
impl Geometry {
    /// The legacy billboard carries no authored scaling mode or render mode.
    /// A host that places the system under run-time ancestors: Local
    /// scaling's unit-chain evidence stands only when every ancestor the
    /// document does not hold carries scale one too (`ancestry_unit`).
    pub(crate) fn keep_unit_chain(&mut self, ancestry_unit: bool) {
        let scaling = match self {
            Self::Billboard { .. } => return,
            Self::Mesh(draw) => &mut draw.scaling,
            Self::SourceBillboard(draw) => &mut draw.scaling,
        };
        if let crate::particle_geometry::Scaling::Local { unit_chain, .. } = scaling {
            *unit_chain &= ancestry_unit;
        }
    }

    /// A Shape-scaled system's lossy global scale of this frame (see
    /// [`crate::particle_geometry::Scaling::Shape`]); nothing for another
    /// scaling mode.
    pub(crate) fn set_shape_scale(&mut self, value: Vec3) {
        let scaling = match self {
            Self::Billboard { .. } => return,
            Self::Mesh(draw) => &mut draw.scaling,
            Self::SourceBillboard(draw) => &mut draw.scaling,
        };
        if let crate::particle_geometry::Scaling::Shape { shape_scale } = scaling {
            *shape_scale = value;
        }
    }

    /// Whether the authored scaling mode is Shape.
    pub(crate) fn shape_scaled(&self) -> bool {
        matches!(self.shape_evidence().map(|evidence| evidence.scaling),
            Some(crate::particle_geometry::Scaling::Shape { .. }))
    }

    pub(crate) fn shape_evidence(&self) -> Option<ShapeEmitterEvidence> {
        match self {
            Self::Billboard { .. } => None,
            Self::Mesh(draw) => Some(ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: true }),
            Self::SourceBillboard(draw) => Some(ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: false }),
        }
    }
}

/// Whether the particle arrays use 3D rotation, by the rule the source applies
/// when it allocates them, every frame before any birth or draw
/// (ParticleSystem::AllocateParticleArrays): the Initial module's 3D start
/// rotation, an enabled Shape module's align to direction, or the separate
/// axes of an enabled RotationOverLifetime or RotationBySpeed module. The
/// emitter carries the Shape and RotationOverLifetime terms (its
/// rotation-over-lifetime and rotation-by-speed blocks exist only for an
/// enabled module); `initial_enabled` is the Initial module's serialized
/// state. None when a Shape term is not decided by the export (its enabled
/// state or align flag missing) and no other term holds.
pub(crate) fn uses_rotation_3d(emitter: &EmitterParams, initial_enabled: bool) -> Option<bool> {
    let known = (initial_enabled && emitter.start.rotation3d)
        || emitter.rotation_over_lifetime.as_ref().is_some_and(|module| module.separate_axes)
        || emitter.rotation_by_speed.as_ref().is_some_and(|module| module.separate_axes);
    let align = emitter.shape.as_ref().and_then(|shape| shape.controls.align_to_direction);
    match (emitter.shape_enabled, align) {
        (Some(false), _) | (_, Some(false)) => Some(known),
        (Some(true), Some(true)) => Some(true),
        _ => known.then_some(true),
    }
}

/// Why a Mesh render-mode system's particle rotation is refused.
///
/// Without 3D rotation the source mesh renderer turns each particle by its Z
/// rotation about the particle's axis of rotation, composed with an angle the
/// draw call passes for its render space (ParticleSystemRenderer's
/// CalculateMeshParticleTransform). The transcribed kernel body is the one
/// View, World and Local share (`particle_geometry::AxisBody`); the render
/// spaces below are the ones it does not cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshRotationRefused {
    /// View without roll passes a camera-derived angle, not transcribed.
    ViewWithoutRoll,
    /// Facing has its own kernel body, not transcribed.
    FacingKernel,
    /// Velocity has its own kernel body, not transcribed.
    VelocityKernel,
    /// The export does not decide the 3D-rotation rule.
    RotationRuleUndecided,
}

impl MeshRotationRefused {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::ViewWithoutRoll =>
                "source Mesh particle rotation about the axis of rotation (no 3D rotation): the View angle without roll is not transcribed",
            Self::FacingKernel =>
                "source Mesh particle rotation about the axis of rotation (no 3D rotation): the Facing kernel is not transcribed",
            Self::VelocityKernel =>
                "source Mesh particle rotation about the axis of rotation (no 3D rotation): the Velocity kernel is not transcribed",
            Self::RotationRuleUndecided =>
                "source Mesh particle 3D rotation is not decided by the export (Shape enabled state or align flag missing)",
        }
    }
}

/// A Mesh render-mode system is admitted with 3D rotation (the Euler mesh
/// transform, `None`), or without it in a render space whose kernel about
/// the axis of rotation is transcribed (that body). The legacy step and the
/// native birth both carry the axis into the particle side data.
pub(crate) fn mesh_rotation_admission(
    emitter: &EmitterParams, initial_enabled: bool,
    alignment: crate::particle_geometry::Alignment, allow_roll: bool,
) -> Result<Option<crate::particle_geometry::AxisBody>, MeshRotationRefused> {
    use crate::particle_geometry::{Alignment, AxisBody};
    match uses_rotation_3d(emitter, initial_enabled) {
        Some(true) => Ok(None),
        Some(false) => match (AxisBody::of(alignment, allow_roll), alignment) {
            (Some(body), _) => Ok(Some(body)),
            (None, Alignment::Facing) => Err(MeshRotationRefused::FacingKernel),
            (None, Alignment::Velocity) => Err(MeshRotationRefused::VelocityKernel),
            (None, _) => Err(MeshRotationRefused::ViewWithoutRoll),
        },
        None => Err(MeshRotationRefused::RotationRuleUndecided),
    }
}

/// A start colour the legacy step does not evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyColourRefused {
    /// A random colour over a gradient with a key group whose time codes are
    /// out of order. The source evaluates a birth group's four draws in one
    /// shared key search (Gradient::EvaluateHDR), and with codes out of order
    /// a lane's colour then depends on the other lanes' draws.
    SharedKeySearch,
}

impl LegacyColourRefused {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::SharedKeySearch =>
                "legacy step start colour: a random colour over key codes out of order is evaluated by the source with one key search shared by four draws",
        }
    }
}

/// The legacy step evaluates the start colour one particle at a time, at the
/// slice's one normalized time and the particle's own draw. That is the
/// source's four-lane evaluation wherever a lane's value does not depend on
/// the other lanes: a constant or two-colour template, a gradient or two
/// gradients (the four lanes of a birth group share the command's one time),
/// and a random colour whose key groups keep their codes in order (a group
/// with fewer than two keys is skipped for every lane). A random colour over
/// codes out of order is refused here; the native birth evaluates it four
/// lanes at a time, as the source does.
pub(crate) fn legacy_start_colour(color: &moly_law::particle::MinMaxGradient)
    -> Result<(), LegacyColourRefused> {
    match color {
        moly_law::particle::MinMaxGradient::RandomColor(gradient) if !gradient.key_search_is_per_lane() =>
            Err(LegacyColourRefused::SharedKeySearch),
        _ => Ok(()),
    }
}

/// The legacy step's burst scheduler was never compared with the source for
/// an infinite repeat interval (the lens-flare bursts author +Infinity with
/// an endless cycle count), so the legacy step refuses it; the native birth
/// schedules it as the source does.
pub(crate) fn legacy_bursts(emitter: &EmitterParams) -> Result<(), &'static str> {
    if emitter.emission.as_ref().is_some_and(|emission|
        emission.bursts.iter().any(|burst| burst.repeat_interval == f32::INFINITY))
    {
        return Err("legacy step: source infinite burst repeat interval scheduling is not yet verified");
    }
    Ok(())
}

/// 一条在跑的粒子系统。
pub(crate) struct Runtime {
    pub(crate) node: String,
    pub(crate) effect: String,
    pub(crate) emitter: EmitterParams,
    pub(crate) kind: EffectKind,
    pub(crate) camera_rotation: bool,
    pub(crate) node_affine: GlobalTransform,
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) anchor: Option<Entity>,
    pub(crate) geometry: Geometry,
    pub(crate) emission_surface: Option<std::sync::Arc<crate::particle_mesh_emission::EmissionSurface>>,
    pub(crate) ring_cursor: usize,
    pub(crate) pool: Vec<Particle>,
    pub(crate) side: Vec<Side>,
    pub(crate) emission: EmissionState,
    /// 播头（秒）：率曲线与 burst 的时间轴。
    pub(crate) playback_head: f32,
    /// 上一帧的播头（burst 裁决要一个左端点）。
    pub(crate) previous_head: f32,
    /// The initial zero-time burst belongs to the first positive simulation step.
    pub(crate) emission_started: bool,
    pub(crate) rng: Rng,
    /// Installed only after source configuration and explicit seed ownership
    /// qualify for the native birth path. Retired instances keep this state.
    pub(crate) native_birth: Option<birth::NativeBirthState>,
    /// Qualified current-JP Noise consumer. The source gate still decides
    /// whether a system may be admitted; this state is only installed after
    /// an owner seed/reset has been proven by the caller.
    pub(crate) noise: Option<NoiseRuntime>,
    /// The per-particle trail, installed with the native birth owner when the
    /// system has a qualified TrailModule. Its rings move with the pool.
    pub(crate) trail: Option<TrailState>,
    /// The CollisionModule with its scene, installed with the native birth
    /// owner; `None` for every system without one.
    pub(crate) collision: Option<collision::CollisionRuntime>,
    /// 惰性 prewarm 的闸：首个推进帧快进一个周期。
    pub(crate) prewarmed: bool,
    /// Time the incremental update left pending (below 1e-6 s), added to the
    /// next frame's scaled delta. Play returns it to zero.
    pub(crate) pending: f32,
    /// The sub-emitter term of the first-Play warm length, read from the
    /// authored graph at admission; 0 without a live child.
    pub(crate) sub_emitter_max_lifetime: f32,
    pub(crate) cone_angle: Option<f32>,
    pub(crate) rol: Option<RotationOverLifetime>,
    pub(crate) limit: Option<LimitVelocity>,
    pub(crate) velocity_law: Option<moly_law::particle::velocity::VelocityOverLifetime>,
    pub(crate) force_law: Option<moly_law::particle::force::ForceOverLifetime>,
    pub(crate) gravity_law: moly_law::particle::gravity::Gravity,
    pub(crate) size_law: Option<moly_law::particle::size::SizeOverLifetime>,
    pub(crate) color_law: Option<moly_law::particle::color::ColorOverLifetime>,
    pub(crate) custom_law: Option<moly_law::particle::custom_data::CustomData>,
    pub(crate) texture_sheet: Option<moly_law::particle::texture_sheet::TextureSheet>,
    pub(crate) sort_mode: moly_law::particle::sort::ParticleSort,
    pub(crate) born_total: u64,
    pub(crate) died_total: u64,
    pub(crate) full_total: u64,
    pub(crate) refused_total: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct NoiseRuntime {
    pub(crate) law: NoiseLaw,
    pub(crate) state: NoiseState,
    /// Source ReadOnlyState owner seed, distinct from each particle birth seed.
    pub(crate) owner_seed: u32,
    /// Retain the serialized/automatic owner so a future proven ResetSeeds
    /// event can replace all module streams atomically.
    pub(crate) owner: moly_law::particle::seed_owner::SeedOwner,
}

/// 逐粒子的出生抽定值，与律池同下标平行存。
#[derive(Clone, Copy)]
pub(crate) struct Side {
    /// 出生种子因子（归一 [0,1)）：生命期内稳定求值读它。
    pub(crate) rand: f32,
    /// 自旋/限速族的种子杂凑吃全宽 u32。
    pub(crate) seed: u32,
    /// 自旋状态（弧度；公告板只画 Z 分量）。
    pub(crate) rot: [f32; 3],
    /// Authored birth size on all three axes. Renderer scale is applied in
    /// the geometry transform, not prematurely collapsed into one X factor.
    pub(crate) size: [f32; 3],
    pub(crate) gravity: f32,
    pub(crate) colour: [f32; 4],
    /// Persistent plus animated velocity, before the integration speed modifier.
    pub(crate) total_velocity: [f32; 3],
    /// Persistent pre-simulation Custom1/Custom2, moved with the particle owner.
    pub(crate) custom_data: [[f32; 4]; 2],
    /// Emission carry of each cached sub-emitter birth edge (at most two),
    /// zero at birth and moved with the particle.
    pub(crate) emit_carry: [f32; 2],
    /// Animated velocity of the last pre-simulation pass (runtime axes),
    /// moved with the particle; the collision query and response read it
    /// apart from the persistent velocity.
    pub(crate) animated: [f32; 3],
    /// The SizeModule's current X size as the collision points last wrote it
    /// (only a CollisionModule system with the current-size stream writes and
    /// reads it); moved with the particle.
    pub(crate) current_size: f32,
    /// Axis of rotation in the source basis, as the particle arrays carry it:
    /// the Shape module's store writes it at birth; without an enabled Shape
    /// module the Initial module writes +Z. The mesh renderer turns the
    /// particle about it when the arrays do not use 3D rotation.
    pub(crate) axis: [f32; 3],
}

/// 出生抽签的确定性随机：splitmix64（站点链同款流算法、不同种子）。
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }

    /// 全宽 u32（取混合输出的低 32 位，与 next_f32 的高 24 位不同位）。
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z as u32
    }
}

/// 推进帧的锚点快照（天空/相机）。
pub(crate) struct Context {
    pub(crate) sky: GlobalTransform,
    pub(crate) camera: GlobalTransform,
    pub(crate) site: GlobalTransform,
}

// ---- 链头：锚点监视 ----

/// 锚 ∘ 节点链（按 effect 类别选锚）。
pub(crate) fn compose_to_world(system: &Runtime, ctx: &Context) -> GlobalTransform {
    let anchor = match system.kind {
        EffectKind::Sky => ctx.sky,
        EffectKind::Camera => {
            if system.camera_rotation {
                ctx.camera
            } else {
                GlobalTransform::from_translation(ctx.camera.translation())
            }
        }
        EffectKind::Site => ctx.site,
    };
    anchor * system.node_affine
}

/// Apply the world-space owner to a direction after Shape/EmitterStoreData.
///
/// The current JP 6.8.1 native path normalizes inside `EmitterStoreData`
/// before multiplying the outer owner 3x3.  It does **not** normalize again
/// after that final owner multiplication.  Keeping this as a small helper
/// makes the boundary explicit and prevents a renderer-facing `normalize`
/// from silently changing non-uniform owner scales.
#[inline]
fn apply_world_owner_direction(owner: &GlobalTransform, direction: Vec3) -> Vec3 {
    owner.affine().transform_vector3(direction)
}

/// 把律状态换算成公告板批次。
pub(crate) fn build_quads(system: &Runtime, to_world: &GlobalTransform) -> Vec<Quad> {
    let mut quads = Vec::with_capacity(system.pool.len());
    for (index, particle) in system.pool.iter().enumerate() {
        let side = system.side[index];
        // 归一化年龄：律的倒计时折算。
        let age = particle.normalized_age();
        let scale = system.node_affine.to_scale_rotation_translation().0.x;
        let evaluated_size = motion::current_size(system, index, particle.age_percent);
        let size = [evaluated_size[0] * scale, evaluated_size[1] * scale];
        // The source render path multiplies byte colour into immutable birth
        // colour. Keep that operation separate from raw gradient evaluation.
        let colour = system.color_law.as_ref().map_or(side.colour, |law| {
            let birth = moly_law::particle::gradient::quantize_rgba8(side.colour);
            moly_law::particle::gradient::rgba8_to_float(
                law.apply(birth, side.seed, age * 100.0))
        });
        // CustomData is a simulation output. Color is evaluated at render time,
        // but re-evaluating CustomData here would advance it by one simulation step.
        let [custom1, custom2] = side.custom_data;
        quads.push(Quad {
            centre: to_world.transform_point(Vec3::from_array(particle.position)),
            size: Vec2::from_array(size),
            // 自旋状态（弧度）：出生角 + rotationOverLifetime 的积分。
            rotation: side.rot[2],
            colour,
            custom1,
            custom2,
        });
    }
    quads
}

/// 一个仿真步：发射（率 + burst）→ 模块批 → 推进 → 死亡移除。
pub(crate) fn simulate(system: &mut Runtime, dt: f32, ctx: &Context) {
    if let Err(reason) = simulate_with_emission(system, dt, ctx, true) {
        error!(%reason, effect=%system.effect, node=%system.node, "native particle step refused");
    }
}

/// Source ParticleSystem.Stop() uses StopEmitting, not StopEmittingAndClear.
/// The system clock and existing particles still advance. The host owns
/// the independent destruction deadline; no prewarm or burst runs after Stop.
/// The weather host steps a stopped system through `step_frame`; this entry
/// serves the replays.
#[cfg(test)]
pub(crate) fn simulate_stopped(system: &mut Runtime, dt: f32, ctx: &Context) {
    if let Err(reason) = simulate_with_emission(system, dt, ctx, false) {
        error!(%reason, effect=%system.effect, node=%system.node, "native particle step refused");
    }
}

/// One frame of an installed system (emitting, or stopped when `emitting` is
/// false), for a host that retires the system when the step is refused. The
/// native step refuses only what this port does not reproduce: a
/// configuration it does not transcribe, or a non-finite birth value. The
/// source has no such outcome. Each frame its incremental update runs the
/// emission (EmissionModule::EmitOverTime writes the emission state and
/// returns the counts), then StartParticles births them, as many as the
/// capacity allows; no source path keeps the clock and the emission state
/// running while it skips the birth. A refused system therefore cannot go on
/// as the source would. Err names the reason, and the caller must not step
/// the system again.
pub(crate) fn step_frame(system: &mut Runtime, dt: f32, ctx: &Context, emitting: bool) -> Result<(), String> {
    simulate_with_emission(system, dt, ctx, emitting)
}

fn simulate_with_emission(system: &mut Runtime, dt: f32, ctx: &Context, emitting: bool) -> Result<(), String> {
    if let Some(mut native) = system.native_birth.take() {
        let stopped = !emitting || native.target.is_some();
        let result = birth::step_explicit(system, &mut native, dt, stopped, ctx);
        system.native_birth = Some(native);
        let result = result.map_err(|error| format!("{error:?}")).and_then(|()| end_of_update_size(system));
        return result.map_err(|error| {
            system.refused_total += 1;
            error
        });
    }
    // The legacy step has no start delay word: a system whose Play leaves one
    // is refused here rather than emitting from its first frame.
    if has_start_delay(&system.emitter) {
        system.refused_total += 1;
        return Err("start delay: the legacy step does not count the start delay word down".into());
    }
    // Current Update1 calls ParticleSystemState::Tick before the stopped gate
    // and before pre-simulation modules. Only zero delay is admitted here:
    // nonzero authored delay cannot substitute for the native remaining wait
    // state, which Stop does not consume. Keep emission carry/RNG untouched.
    if !emitting && dt.is_finite() && dt > 0.0
        && matches!(system.emitter.start_delay, moly_law::particle::MinMaxCurve::Constant(0.0))
        && system.playback_head.is_finite()
        && system.emitter.duration.is_finite() && system.emitter.duration > 0.0
    {
        system.playback_head += dt;
        if system.emitter.looping {
            while system.playback_head >= system.emitter.duration {
                system.playback_head -= system.emitter.duration;
            }
        } else {
            system.playback_head = system.playback_head.min(system.emitter.duration);
        }
    }
    if emitting && dt > 0.0 {
        let emission = system
            .emitter
            .emission
            .as_ref()
            .expect("判读已门 emission 在场")
            .clone();
        system.previous_head = if system.emission_started {
            system.playback_head
        } else {
            // burst_check uses (previous, now]. Include exactly zero on first
            // playback without shifting the source clock or repeating it later.
            system.emission_started = true;
            -f32::from_bits(1)
        };
        system.playback_head += dt;
        // 非循环系统播到头就停发（现存粒子活完寿命）。循环系统的播头按
        // duration 回卷——burst 的时间轴与率曲线共用它。
        if system.emitter.looping && system.emitter.duration > 0.0 {
            while system.playback_head >= system.emitter.duration {
                system.playback_head -= system.emitter.duration;
                system.previous_head -= system.emitter.duration;
            }
        }
        // The source samples the rate at the slice end, in normalized cycle time.
        let rate = match cycle_curve(&emission.rate_over_time,
            normalized_time(system, system.playback_head), 0.5) {
            Ok(rate) => rate,
            Err(reason) => {
                system.refused_total += 1;
                error!(%reason, effect=%system.effect, node=%system.node, "rate over time refused");
                0.0
            }
        };
        let mut emitted = accumulate_rate(&mut system.emission, rate, dt);
        for burst in &emission.bursts {
            let rand_fire = system.rng.next_f32();
            let rand_count = system.rng.next_f32();
            match burst_check(
                burst,
                system.previous_head,
                system.playback_head,
                rand_fire,
                rand_count,
            ) {
                BurstOutcome::Fired(count) => emitted += count,
                BurstOutcome::NotDue | BurstOutcome::MissedByProbability => {}
            }
        }
        if !system.emitter.looping && system.previous_head > system.emitter.duration {
            emitted = 0;
        }
        let old_count = system.pool.len();
        let accepted = birth_capacity(old_count, system.emitter.ring_buffer_mode,
            system.emitter.max_particles as usize, emitted as usize);
        system.full_total += (emitted as usize - accepted) as u64;
        for _ in 0..accepted {
            spawn_one(system, ctx);
        }
        finish_births(&mut system.pool, &mut system.side, &mut system.ring_cursor,
            system.emitter.ring_buffer_mode, system.emitter.max_particles as usize, old_count,
            |_, _| system.died_total += 1);

    }

    simulate_existing(system, dt, ctx, None);
    Ok(())
}

/// Existing-particle pre/sim/death barrier. Autonomous births are separate.
/// `deaths`, when given, receives every killed particle in removal order as
/// the death event reads it, before its slot is overwritten.
fn simulate_existing(system: &mut Runtime, dt: f32, ctx: &Context, mut deaths: Option<&mut Vec<DeathParent>>) {
    simulate_range(system, 0, system.pool.len(), dt, None, None, ctx);
    let (mode, maximum) = (system.emitter.ring_buffer_mode, system.emitter.max_particles as usize);
    // CustomDataModule's call over the live particles ends in the lanes of
    // its last four-lane group past the count, and SimulateParticles advances
    // their ages; the kill pass below then leaves the slots past the new
    // count. A law that follows its curve objects' caches is told both.
    let custom_storage = follows_storage(system);
    if let Some(custom) = system.custom_law.as_mut() {
        custom.end_existing_call(system.pool.len(), dt, mode, system.emitter.ring_buffer_loop_range);
    }
    // A size law that follows the engine's calls evaluates nothing here (the
    // module runs after the slices), but its slots past the count age too.
    if let Some(calls) = system.size_law.as_mut().and_then(|size| size.calls_mut()) {
        calls.end_existing_call(system.pool.len(), dt, mode, system.emitter.ring_buffer_loop_range);
    }
    let before = custom_storage.then(|| system.pool.clone());
    let mut removed = Vec::new();
    let mut on_death = |index: usize, particle: &Particle, side: &Side| {
        removed.push(index);
        if let Some(deaths) = deaths.as_mut() {
            deaths.push(sub_events::dying(index, particle, side));
        }
    };
    // A dying particle's trail points go with it: the engine moves every
    // per-particle array of the last particle into the dead slot.
    system.died_total += match system.trail.as_mut() {
        Some(trail) => compact_with_sides_indexed(&mut system.pool, &mut system.side, &mut trail.rings, mode, maximum,
            &mut on_death),
        None => {
            let mut none = vec![(); system.pool.len()];
            compact_with_sides_indexed(&mut system.pool, &mut system.side, &mut none, mode, maximum, &mut on_death)
        }
    } as u64;
    if let Some(before) = before {
        if let Some(custom) = system.custom_law.as_mut() {
            custom.kill(&before, &removed, &system.pool);
        }
        if let Some(calls) = system.size_law.as_mut().and_then(|size| size.calls_mut()) {
            calls.kill(&before, &removed, &system.pool);
        }
    }
}

/// Run module math before packing. Birth lanes use times relative to their own
/// aligned native block, so an unaligned old prefix cannot supply a batch seed.
/// Newborn inline integration differs from the existing SimulateParticles path.
fn simulate_range(system: &mut Runtime, start: usize, end: usize, dt: f32,
    birth_dts: Option<&[f32]>, birth_backtrack: Option<(&[f32], [f32; 3])>, ctx: &Context) {
    assert!(start <= end && end <= system.pool.len());
    assert_eq!(system.pool.len(), system.side.len());
    if let Some(times) = birth_dts { assert_eq!(times.len(), end - start); }
    if let Some((scales, _)) = birth_backtrack {
        assert!(birth_dts.is_some() && scales.len() == end - start);
    }
    // Native Noise.Update advances its system scroll once for a nonempty
    // existing-particle range. StartModules birth ranges reuse that scroll and
    // must not advance it again. Empty ranges do not consume a scroll step.
    if birth_dts.is_none() && start < end {
        if let Some(noise) = &mut system.noise {
            noise.law.advance_scroll(&mut noise.state, dt, true);
        }
    }
    let owner = compose_to_world(system, ctx);
    let velocity_over_lifetime = system.velocity_law.as_ref();
    // RotationBySpeed is built from the emitter block (admission refused a
    // block its law does not take). Both rotation modules add into one
    // angular-velocity lane per axis that Initial clears for the range, and
    // the rotation is integrated once with the sum after the module chain,
    // on all three axes when the particle arrays use 3D rotation.
    let by_speed = system.emitter.rotation_by_speed.as_ref()
        .and_then(|params| moly_law::particle::rotation_by_speed::RotationBySpeed::from_params(params).ok());
    let rotates = system.rol.is_some() || by_speed.is_some();
    let rotation_3d = uses_rotation_3d(&system.emitter, true).unwrap_or(system.emitter.start.rotation3d);
    for index in start..end {
        let dt = birth_dts.map_or(dt, |times| times[index - start]);
        let side = system.side[index];
        let start_lifetime = system.pool[index].start_lifetime;
        // 引擎的模块批（自旋/叠加速/限速）先于推进跑，读的是**推进前**
        // 的年寿进程量——余寿先留底。
        let age_pre = system.pool[index].normalized_age();
        // 寿限非正的按出生即死（律对非正寿限拒绝推进，会永生）。
        if !(start_lifetime > 0.0) {
            system.pool[index].age_percent = f32::from_bits(0x42c80001);
            if let Some(custom) = system.custom_law.as_mut() {
                custom.skipped_lane();
            }
            continue;
        }
        // Loop protection belongs to the inner span; displaced particles in
        // overflow finish their lifetime normally. Pause does not freeze motion.
        let lifetime_mode = if system.emitter.ring_buffer_mode == RingBufferMode::LoopUntilReplaced
            && index >= system.emitter.max_particles as usize {
            RingBufferMode::Disabled
        } else { system.emitter.ring_buffer_mode };
        // InitialModule writes gravity into persistent velocity before the
        // animated velocity, Force, ClampVelocity and integration batches.
        // Gravity is a world vector; local simulation applies the inverse owner
        // directly, without the extra emitter-scale factor used by Force.
        let gravity = Vec3::from_array(system.gravity_law.delta(side.seed, system.playback_head,
            system.emitter.duration, dt, GRAVITY));
        let gravity = if system.emitter.simulation_space == SimulationSpace::World { gravity }
            else { owner.affine().inverse().transform_vector3(gravity) };
        for axis in 0..3 { system.pool[index].velocity[axis] += gravity[axis]; }
        let mut angular = [0.0_f32; 3];
        if let Some(rol) = &system.rol {
            // The stored age-percent word before this update's advance: the
            // module's curve time is `max(age * 0.01, 0)` of that word, and
            // `normalized_age() * 100` does not give the word back.
            let w = rol.angular_velocity(side.seed, 0.0, system.pool[index].age_percent);
            for axis in 0..3 { angular[axis] += w[axis]; }
        }
        let batch_seed = system.side[start + ((index - start) & !3)].seed;
        // Velocity, Force and ClampVelocity evaluate their curves at
        // `fmax(age * 0.01, 0)` of the stored age-percent word, as
        // RotationOverLifetime does; the word taken back from the normalized
        // age rounds twice and is not the one they read.
        let module_age = if child::arms::on("moduleAgeFromNormalized") { age_pre * 100.0 }
            else { system.pool[index].age_percent };
        let (velocity_anim, modifier) = match velocity_over_lifetime {
            Some(value) => motion::velocity_at_age(value, &system.pool[index], &side, batch_seed,
                system.emitter.simulation_space, &owner, module_age, dt, birth_dts.is_some()),
            None => ([0.0; 3], 1.0),
        };
        // Noise contributes transient animated velocity after authored
        // VelocityOverLifetime and before Force/LimitVelocity. Runtime stores
        // reflected coordinates, so reflect the source-space kernel once at
        // this boundary; it is not a world-space acceleration. A strength
        // curve reads the particle's age before this frame's lifetime advance,
        // the same stored percent the pre-simulation module batch sees.
        let noise_anim = system.noise.as_ref().map_or([0.0; 3], |noise| {
            let source_position = crate::particle_geometry::reflect(
                Vec3::from_array(system.pool[index].position)).to_array();
            crate::particle_geometry::reflect(Vec3::from_array(
                noise.law.sample(noise.state, source_position, noise.owner_seed, system.side[index].seed,
                    system.pool[index].age_percent),
            )).to_array()
        });
        let anim = std::array::from_fn(|axis| velocity_anim[axis] + noise_anim[axis]);
        system.side[index].animated = anim;
        // Force modifies persistent velocity before velocity limiting. Animated
        // velocity is still transient and shares the final integration modifier.
        if let Some(force) = &system.force_law {
            let acceleration = motion::module_vector(force.sample(side.seed, module_age),
                force.in_world_space, system.emitter.simulation_space, &owner);
            for axis in 0..3 {
                system.pool[index].velocity[axis] += acceleration[axis] * dt;
            }
        }
        if let Some(law) = system.limit.as_ref().filter(|_| !(dt < 0.0 && child::arms::on("clampSkipsNegativeElapsed"))) {
            let mut velocity = system.pool[index].velocity;
            let _ = law.step(
                &mut velocity,
                anim,
                side.seed,
                module_age,
                dt,
                DragSize {
                    components: motion::size_at_age_percent(system, &side, system.pool[index].age_percent),
                    size3d: system.emitter.start.size3d || system.emitter.size_over_lifetime
                        .as_ref().is_some_and(|size| size.separate_axes),
                },
            );
            system.pool[index].velocity = velocity;
        }
        // RotationBySpeed runs after ClampVelocity and reads the speed the
        // chain leaves (persistent plus animated velocity).
        if let Some(by_speed) = &by_speed {
            by_speed.add(&mut angular, side.seed, 0.0, system.pool[index].velocity, anim);
        }
        if let Some(custom) = system.custom_law.as_mut() {
            custom.update(side.seed, system.pool[index].age_percent, &mut system.side[index].custom_data);
        }
        if birth_dts.is_some() {
            // StartModules writes birth age directly, without ring looping or
            // pause. It then kills >100 lanes before CopyParticles packing.
            let p = &mut system.pool[index];
            let age = if p.age_percent >= 100.0 { p.age_percent }
                else { (dt * 100.0) * p.inverse_lifetime };
            p.age_percent = age.min(f32::from_bits(0x42c8_0001));
        } else {
            advance_lifetime(&mut system.pool[index], dt, lifetime_mode,
                system.emitter.ring_buffer_loop_range);
        }
        // The same pre-simulation speed modifier owns both orbital displacement
        // and integration. Animated velocity remains transient, including when
        // a zero speed modifier stops movement without clearing base velocity.
        let total = std::array::from_fn(|axis|
            system.pool[index].velocity[axis] + anim[axis]);
        let eff = total.map(|value| value * modifier);
        system.side[index].total_velocity = total;
        // 积分：律的 `integrate` 会用帧速度覆写状态速度——先存后还原。
        let state_velocity = system.pool[index].velocity;
        if birth_dts.is_some() {
            // The emitter-motion backtrack comes after the pre-simulation
            // modules above (they see the unmoved position) and before the
            // newborn integration.
            if let Some((scales, velocity)) = birth_backtrack {
                let scale = scales[index - start];
                for axis in 0..3 { system.pool[index].position[axis] -= scale * velocity[axis]; }
            }
            // Native inline birth arithmetic groups modifier*dt before the
            // velocity multiply. Reassociation changes several f32 results.
            let step = modifier * dt;
            for axis in 0..3 { system.pool[index].position[axis] += total[axis] * step; }
        } else if let StepVerdict::Refused = integrate(&mut system.pool[index], dt, eff) {
            system.refused_total += 1;
        }
        system.pool[index].velocity = state_velocity;
        // One integration of the summed rate, `angular * dt + rotation`,
        // after the position (births) or the kill pass (updates; a dying
        // particle's rotation is read by no death consumer).
        if rotates && !(dt < 0.0 && child::arms::on("rotationSkipsNegativeElapsed")) {
            let rot = &mut system.side[index].rot;
            if rotation_3d {
                for axis in 0..3 { rot[axis] = angular[axis] * dt + rot[axis]; }
            } else {
                rot[2] = angular[2] * dt + rot[2];
            }
        }
    }
 }

/// The newborn span's pre-simulation modules and its position and age update.
fn simulate_birth_modules(system: &mut Runtime, old_count: usize, partial_dts: &[f32],
    backtrack: Option<(&[f32], [f32; 3])>, ctx: &Context) {
    assert_eq!(old_count + partial_dts.len(), system.pool.len());
    simulate_range(system, old_count, system.pool.len(), 0.0, Some(partial_dts), backtrack, ctx);
}

/// Newborn deaths after the span's modules; returns the surviving count.
/// `deaths`, when given, receives the killed particles whose kill records
/// death events, in removal order, before their slot is overwritten.
fn kill_newborns(system: &mut Runtime, old_count: usize, accepted: usize,
    mut deaths: Option<&mut Vec<DeathParent>>) -> usize {
    // StartModules scans its newborn range forward and retests each swapped
    // tail; existing four-wide death compaction has a different removal order.
    let mut index = old_count;
    let mut surviving_count = accepted;
    while index < system.pool.len() {
        if system.pool[index].age_percent > 100.0 {
            // The kill records the death events only while the accepted count
            // it decrements is still nonzero, so a padding lane records too
            // until that count runs out.
            if surviving_count != 0 || child::arms::on("newbornDeathAlwaysRecords") {
                if let Some(deaths) = deaths.as_mut() {
                    deaths.push(sub_events::dying(index, &system.pool[index], &system.side[index]));
                }
            }
            system.pool.swap_remove(index);
            system.side.swap_remove(index);
            if let Some(trail) = system.trail.as_mut() { trail.rings.swap_remove(index); }
            system.died_total += 1;
            // The native cleanup visits rounded SIMD storage, including the
            // padding. Every kill saturating-decrements the accepted count;
            // a swapped padding lane can therefore enter the live prefix.
            surviving_count = surviving_count.saturating_sub(1);
        } else { index += 1; }
    }
    surviving_count
}

/// Outcome of installing the source birth owner on an admitted instance.
pub(crate) enum BirthPath {
    Native,
    /// The system stays on the legacy step; the reason is recorded for the
    /// diagnostics rather than inferred later.
    Legacy(String),
}

/// Whether this emitter takes the native birth path: the source selects the
/// ordinary incremental route and every enabled module has a verified native
/// consumer (Initial, constant or two-constant Emission, the qualified Shape configurations,
/// the qualified Noise subset, authored-null child edges; lifetime modules are
/// shared with the legacy step). The emitter state a Shape reads is checked
/// separately by `native_shape_state_eligible`.
pub(crate) fn native_birth_eligible(emitter: &EmitterParams, route: &SourceRoute) -> Result<(), String> {
    match route {
        SourceRoute::Ordinary => {}
        // The per-frame update never evaluates a system in time: the manager's
        // step carries no procedural flag, so a procedural system steps
        // incrementally exactly as an ordinary one. Only Play's first warm of a
        // looping prewarm system takes the procedural update, whose
        // composition is qualified here.
        SourceRoute::Procedural if !(emitter.prewarm && emitter.looping) => {}
        SourceRoute::Procedural => birth::qualify_procedural_warm(emitter)
            .map_err(|refused| format!("procedural first-Play warm: {refused:?}"))?,
        SourceRoute::Undecided(control) => return Err(format!("source route undecided: {control}")),
    }
    birth::qualify_frame_emitter(emitter).map_err(|refused| format!("{refused:?}"))
}

/// One time update of `ParticleSystem.Simulate(t, restart: false)` from
/// script (a ControlPlayable's chunk): Update1b with UpdateData flags 4, the
/// per-frame update's head and slices without the backlog widening, `dt` the
/// chunk's raw seconds (the head scales it by the simulation speed). The
/// Simulate leaves the system paused, so nothing else steps it.
pub(crate) fn director_chunk(system: &mut Runtime, dt: f32, emitting: bool, ctx: &Context,
    slice_start: impl FnMut(&Runtime)) -> Result<bool, String> {
    use moly_law::particle::frame_time::IncrementalEntry;
    let entry = if child::arms::on("scriptSimulateWidens") { IncrementalEntry::PerFrame }
        else { IncrementalEntry::ScriptSimulate };
    advance_frame_entry(system, dt, emitting, ctx, slice_start, entry)
}

/// The time update of `ParticleSystem.Simulate(t, restart: true)` from
/// script with fixedTimeStep (the managed three-argument overload passes
/// it): Update1b with UpdateData flags 5, the frame head over `t` (the
/// explicit dt, scaled by the simulation speed), GetTimeStep's fixed step
/// (the TimeManager's Fixed Timestep while the world plays) and
/// Update1Incremental's fixed-step loop without the backlog widening (see
/// [`moly_law::particle::frame_time::IncrementalEntry::ScriptSimulateFixed`]).
/// One call, not chunked. The Simulate leaves the system paused.
pub(crate) fn script_simulate_fixed(system: &mut Runtime, t: f32, emitting: bool, ctx: &Context,
    slice_start: impl FnMut(&Runtime)) -> Result<bool, String> {
    advance_frame_entry(system, t, emitting, ctx, slice_start,
        moly_law::particle::frame_time::IncrementalEntry::ScriptSimulateFixed)
}

/// What `ParticleSystem.Simulate`'s restart needs besides the birth decision.
/// A prewarm system warms at every restart, looping or not: through the
/// procedural update when its route is procedural (flag 2, not transcribed),
/// with the ring laws when it keeps a ring buffer (that warm has not been
/// executed against the engine), and over the start lifetime range, which a
/// curve lifetime takes through a range evaluation that is not transcribed.
pub(crate) fn director_restart_eligible(emitter: &EmitterParams, route: &SourceRoute) -> Result<(), String> {
    if !emitter.prewarm {
        return Ok(());
    }
    if *route != SourceRoute::Ordinary {
        return Err("Simulate restart warm of a prewarm system off the ordinary route: the procedural update is not transcribed".into());
    }
    if emitter.ring_buffer_mode != RingBufferMode::Disabled {
        return Err("ring buffer Simulate restart warm over the ordinary slices has not been executed".into());
    }
    first_play_lifetime(emitter).map(|_| ()).map_err(str::to_owned)
}

/// The warm of `ParticleSystem.Simulate`'s restart: ComputePrewarmStartParameters
/// with time zero, then one update of its out value. `Ok(None)`: a system with
/// prewarm off, whose update has no time and skips (its clock is zero);
/// `Ok(Some)`: the warm of a prewarm system, from its mid-cycle clock;
/// `Err`: what [`director_restart_eligible`] refuses, and a Compute that
/// fails (a non-looping system that ends before time zero, a warm window that
/// does not advance by the fixed step), after which the engine leaves the
/// system stopped and cleared, which is not ported.
pub(crate) fn director_restart_warm(emitter: &EmitterParams, route: &SourceRoute, sub_emitter_max_lifetime: f32)
    -> Result<Option<FirstPlayWarm>, String> {
    // The lifetime range's upper lane (+Infinity read as the duration).
    let upper = first_play_lifetime(emitter).ok().map(|lifetime| lifetime.first_play_upper(emitter.duration));
    // Compute's first test, for a non-looping system: the duration plus that
    // upper lane below time zero (a NaN sum passes).
    if let Some(upper) = upper.filter(|_| !emitter.looping) {
        if emitter.duration + upper < 0.0 {
            return Err("Simulate restart: Compute stops a non-looping system that ends before time zero; not ported".into());
        }
    }
    if !emitter.prewarm {
        return Ok(None);
    }
    // The warm length: that lane, or the sub-emitter term when it is longer.
    // A length of zero or less gives Compute an out value of zero from a
    // start of zero: the warm update has no time and skips, and the clock is
    // zero, whatever route the system takes.
    if let Some(upper) = upper {
        let reach = if sub_emitter_max_lifetime < upper { upper } else { sub_emitter_max_lifetime };
        if !(reach > 0.0) {
            return Ok(None);
        }
    }
    director_restart_eligible(emitter, route)?;
    let play = PlayState {
        elapsed: 0.0,
        live_count: 0,
        native_play_bool_argument: false,
        world_playing: true,
        ordinary_incremental: *route == SourceRoute::Ordinary,
        sub_emitter_max_lifetime,
        prewarm: emitter.prewarm,
        looping: emitter.looping,
        simulation_speed: emitter.simulation_speed,
        duration: emitter.duration,
    };
    FirstPlayWarm::simulate_restart(first_play_lifetime(emitter)?, PLAYER_TIME, play)
        .map(Some)
        .map_err(|error| format!("Simulate restart warm: {error}"))
}

/// Whether the one-rule arm `name` is on (test builds only; always off
/// otherwise).
pub(crate) fn arm(name: &str) -> bool {
    child::arms::on(name)
}

/// `ParticleSystem.Simulate(0, withChildren: false, restart: true)` from
/// script, on a system whose owner is manual (a ControlPlayable's Initialize
/// made it so): ResetSeeds from the serialized seed, which draws nothing from
/// the shared manager; Clear; `Play(false)`, which returns the clock, the
/// pending time and the loop count to zero, writes the start delay word and
/// sets the emitter reset; then ComputePrewarmStartParameters with time zero
/// and its update: a prewarm system (looping or not) warms over its lifetime
/// range from the mid-cycle clock as one ordinary update with the backlog
/// widening, and any other system's update has no time and skips, as does the
/// restart's own zero-time update after it. The birth path is decided as at a
/// first Play. The emitter velocity survives, as Play does not clear it.
/// A sub-emitter parent's birth events (`edges`) are installed with the
/// owner, as Play installs them; the legacy step records none, so a parent
/// the native installer leaves there is refused.
pub(crate) fn director_restart(system: &mut Runtime, route: &SourceRoute, edges: Option<&EventEdges>, ctx: &Context)
    -> Result<BirthPath, String> {
    if system.emitter.auto_random_seed != Some(false) {
        return Err("Simulate restart needs the manual owner the playable's Initialize sets".into());
    }
    let warm = director_restart_warm(&system.emitter, route, system.sub_emitter_max_lifetime)?;
    let velocity = system.native_birth.as_ref().map_or([0.0; 3], |native| native.frame.velocity);
    clear_particles(system);
    system.born_total = 0;
    system.died_total = 0;
    system.full_total = 0;
    reset_for_first_play(system);
    // A manual owner's reset expands its serialized seed and never reads the
    // shared manager, so an empty one stands in.
    let path = install_native_birth(system, &mut seed::SystemSeedManager::default(), route, None)
        .map_err(|error| error.to_string())?;
    if let Some(edges) = edges {
        match (&path, system.native_birth.as_mut()) {
            (BirthPath::Native, Some(native)) => native.events = Some(BirthEvents::with_edges(edges.clone())),
            (BirthPath::Legacy(reason), _) =>
                return Err(format!("sub-emitter parent refused by the native birth installer: {reason}")),
            (BirthPath::Native, None) => return Err("native birth owner missing after its install".into()),
        }
    }
    system.prewarmed = true;
    let mut warm_dt = None;
    if let Some(warm) = warm {
        let plan = PrewarmPlan::from_warm(warm, PLAYER_TIME, system.emitter.duration)?;
        warm_dt = Some(plan.compute_out());
        system.playback_head = plan.initial_clock();
        match system.native_birth.take() {
            Some(mut state) => {
                let result = birth::run_warm(system, &mut state, plan, ctx);
                system.native_birth = Some(state);
                result.map_err(|error| format!("Simulate restart warm refused: {error:?}"))?;
                end_of_update_size(system)?;
            }
            None => {
                let mut plan = plan;
                for slice in plan.by_ref() {
                    let slice = slice.map_err(|error| format!("Simulate restart warm refused: {error}"))?;
                    step_frame(system, slice.duration, ctx, true)?;
                }
                system.pending = plan.remaining();
            }
        }
    }
    let translation = compose_to_world(system, ctx).translation().to_array();
    if let Some(native) = system.native_birth.as_mut() {
        native.frame = birth::FrameState::after_restart(velocity, warm_dt, translation, native.frame.start_delay);
    }
    Ok(path)
}

/// Whether the emitter authors a distance rate other than zero; only the
/// native per-frame head consumes it.
pub(crate) fn has_distance_emission(emitter: &EmitterParams) -> bool {
    emitter.emission.as_ref().is_some_and(|emission|
        !matches!(emission.rate_over_distance, moly_law::particle::MinMaxCurve::Constant(v) if v == 0.0))
}

/// Whether any sub-emitter edge is real (not authored null: no emitter and
/// pointer 0/0).
pub(crate) fn has_real_sub_emitter_edges(emitter: &EmitterParams) -> bool {
    birth::has_real_sub_emitter_edges(emitter)
}

/// Whether the emitter state the native Shape boundary reads is qualified:
/// the scaling mode, the owner chain it implies and the render mode. An
/// emitter without a Shape block needs none of it.
pub(crate) fn native_shape_state_eligible(emitter: &EmitterParams, evidence: Option<ShapeEmitterEvidence>)
    -> Result<(), String> {
    birth::shape_emitter_state(emitter, evidence).map(|_| ()).map_err(|refused| format!("{refused:?}"))
}

/// The birth path `install_native_birth` takes for an admitted system: native
/// when the route and modules qualify and the emitter state a Shape reads
/// qualifies, otherwise the legacy step, for the named reason.
pub(crate) fn native_birth_path(emitter: &EmitterParams, route: &SourceRoute, evidence: Option<ShapeEmitterEvidence>)
    -> Result<(), String> {
    native_birth_eligible(emitter, route).and_then(|()| native_shape_state_eligible(emitter, evidence))
}

/// Called once when the admitted source instance is installed. The first Play
/// resets the system seeds (one shared-manager draw for an automatic owner)
/// and expands the Initial, Shape and scalar emission streams. Noise reads the
/// same owner seed and starts from the reset scroll, so it is installed from
/// this event rather than drawing a second owner. A CollisionModule is
/// installed with `collision_scene` (its effect's ground scene) and the
/// Collision stream of the same reset; without a scene the system is not
/// installed on this path.
/// What a CollisionModule system is installed with: the scene of its
/// effect's ground collider and, in Local space, the owner words its query
/// and hits read.
pub(crate) struct CollisionInstall {
    pub(crate) scene: Box<dyn moly_law::particle::collision_query::CollisionScene + Send + Sync>,
    pub(crate) owner: Option<moly_law::particle::collision_query::OwnerPair>,
}

impl CollisionInstall {
    /// A Planes-type module's install: it reads its plane slots and never
    /// the physics scene; a Local system still needs its owner words.
    pub(crate) fn planes(owner: Option<moly_law::particle::collision_query::OwnerPair>) -> Self {
        Self { scene: Box::new(collision::NoScene), owner }
    }
}

pub(crate) fn install_native_birth(system: &mut Runtime, seeds: &mut seed::SystemSeedManager,
    route: &SourceRoute, collision: Option<CollisionInstall>)
    -> Result<BirthPath, seed::SeedError> {
    if system.native_birth.is_some() { return Ok(BirthPath::Native); }
    if let Err(reason) = native_birth_path(&system.emitter, route, system.geometry.shape_evidence()) {
        // The legacy step does not consume the native streams, but the source
        // still resets this system's seed at first Play: an automatic owner
        // takes the next shared-manager word, a manual one its serialized seed.
        let (owner, _) = seeds.create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)?;
        system.rng = Rng(u64::from(owner.seed) | (u64::from(owner.seed) << 32));
        return Ok(BirthPath::Legacy(reason));
    }
    // Qualify the Noise consumer before drawing, so a refused configuration is
    // reported before the owner is created.
    let noise_law = match system.emitter.noise.as_ref() {
        Some(params) => match NoiseLaw::from_params(params) {
            Ok(law) => Some(law),
            Err(reason) => return Ok(BirthPath::Legacy(reason.to_owned())),
        },
        None => None,
    };
    // The trail law was qualified with the rest of the composition; parse it
    // again here, before the owner draw, so a refusal draws nothing.
    let trail_law = match trails::qualify(&system.emitter) {
        Ok(law) => law,
        Err(reason) => return Ok(BirthPath::Legacy(reason.to_owned())),
    };
    // The CollisionModule law was qualified with the composition; its scene
    // comes from admission, before the owner draw.
    let collision = match (system.emitter.collision.is_some(), collision) {
        (false, _) => None,
        (true, Some(install)) => Some(install),
        // A Planes module reads no scene; without owner words a Local one
        // is refused inside.
        (true, None) if collision::is_planes(&system.emitter) => Some(CollisionInstall::planes(None)),
        (true, None) => return Ok(BirthPath::Legacy("CollisionModule without its ground scene".to_owned())),
    };
    let (owner, streams) = seeds.create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)?;
    // The first Play writes the start delay word after the seed reset.
    let Some(start_delay) = play_start_delay(&system.emitter) else {
        system.rng = Rng(u64::from(owner.seed) | (u64::from(owner.seed) << 32));
        return Ok(BirthPath::Legacy("random start delay: Play's seed-hash evaluation is not transcribed".to_owned()));
    };
    system.native_birth = Some(birth::NativeBirthState {
        owner: Some(owner), initial: streams.initial, shape: streams.shape,
        shape_clock: moly_law::particle::shape::ArcLoopClock::default(),
        emission: moly_law::particle::autonomous_emission::AutonomousEmissionState::initialized(streams.scalar_birth),
        frame: birth::FrameState { start_delay, ..birth::FrameState::default() },
        events: None,
        target: None,
        procedural: matches!(route, SourceRoute::Procedural) && system.emitter.prewarm && system.emitter.looping,
        replays: Vec::new(),
    });
    if let Some(law) = noise_law {
        system.noise = Some(NoiseRuntime {
            law,
            state: NoiseState { scroll: streams.noise_scroll },
            owner_seed: owner.seed,
            owner,
        });
    }
    // The module clock starts at zero and no Play resets it; every ring starts
    // empty (the first update's reset of all rings).
    if let Some(law) = trail_law {
        trails::install(system, law);
    }
    if let Some(install) = collision {
        // A Local system without its owner words is refused inside.
        if let Err(reason) = collision::install(system, install.scene, install.owner, streams.collision) {
            system.native_birth = None;
            system.noise = None;
            system.trail = None;
            return Ok(BirthPath::Legacy(reason));
        }
    }
    Ok(BirthPath::Native)
}

/// Explicit Noise consumer with its own owner draw. Production installs Noise
/// together with the native birth owner above; only the kernel checks use this.
#[cfg(test)]
pub(crate) fn install_noise_consumer(
    system: &mut Runtime,
    seeds: &mut seed::SystemSeedManager,
) -> Result<bool, String> {
    if system.noise.is_some() {
        return Ok(true);
    }
    let Some(params) = system.emitter.noise.as_ref() else {
        return Ok(false);
    };
    let law = NoiseLaw::from_params(params).map_err(str::to_owned)?;
    let (owner, streams) = seeds
        .create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)
        .map_err(|error| format!("{error:?}"))?;
    system.noise = Some(NoiseRuntime {
        law,
        state: NoiseState {
            scroll: streams.noise_scroll,
        },
        owner_seed: owner.seed,
        owner,
    });
    Ok(true)
}

/// 出生一颗：形状抽样 → 出生取值 → 律的入池裁决。
/// Clock time as the fraction of one authored cycle, the time base of every
/// start value and emission curve.
/// A MinMaxCurve the legacy step evaluates at its normalized cycle time,
/// through the engine's curve dispatch. The judge admits every such curve
/// through [`curve_admission`] before a runtime exists.
fn cycle_curve(curve: &moly_law::particle::MinMaxCurve, t: f32, random: f32) -> Result<f32, &'static str> {
    use moly_law::particle::curve::{CurveSampler, CurveTime};
    Ok(CurveSampler::new(curve, CurveTime::Normalized)?.evaluate(t, random))
}

/// Every MinMaxCurve a runtime evaluates goes through the engine's curve
/// dispatch (`moly_law::particle::curve::CurveSampler`). Each consumer is built
/// once here, so a lane outside the transcribed evaluator refuses the system
/// before any law is installed; the installation sites rely on it. Force,
/// rotation, limit velocity and Noise are built by the judge itself.
pub(crate) fn curve_admission(emitter: &EmitterParams) -> Result<(), String> {
    curve_admission_with(emitter, None, None)
}

/// [`curve_admission`] for a host that installs [`custom_data_law`] and
/// [`size_over_lifetime_law`]: a CustomData lane the history-independence
/// certificate refuses is admitted when `storage` qualifies the system for the
/// law that follows the engine's storage ([`custom_data_storage_eligible`]
/// and the host's own conditions), and a size lane when `size_storage`
/// qualifies it for the size law that follows the engine's calls
/// ([`size_storage_eligible`] and the host's own conditions).
pub(crate) fn curve_admission_with(emitter: &EmitterParams, storage: Option<&dyn Fn() -> Result<(), String>>,
    size_storage: Option<&dyn Fn() -> Result<(), String>>) -> Result<(), String> {
    use moly_law::particle::curve::{CurveSampler, CurveTime};
    use moly_law::particle::custom_data::CustomData;
    use moly_law::particle::MinMaxCurve;
    use moly_law::particle::size::SizeOverLifetime;
    // A size curve whose value can follow its cache history is admitted only
    // with the law that follows the engine's calls: the other form evaluates
    // where a consumer asks (LimitVelocity's drag, the geometry, the collision
    // radius), which is not the engine's sequence on the curve objects.
    if let Some(params) = &emitter.size_over_lifetime {
        if let Err(reason) = SizeOverLifetime::from_params(params) {
            let Some(qualify) = size_storage else { return Err(format!("sizeOverLifetime: {reason}")); };
            SizeOverLifetime::with_storage(params, size_3d(emitter), moly_law::particle::slot_tail::RESERVED_SLOTS)
                .map_err(|reason| format!("sizeOverLifetime: {reason}"))?;
            qualify().map_err(|why| format!(
                "sizeOverLifetime: {reason}; the law that follows the engine's calls is refused: {why}"))?;
        }
    }
    if let Some(params) = &emitter.custom_data {
        if let Err(reason) = CustomData::from_params(params) {
            let Some(qualify) = storage else { return Err(format!("customData: {reason}")); };
            CustomData::with_storage(params, moly_law::particle::slot_tail::RESERVED_SLOTS)
                .map_err(|reason| format!("customData: {reason}"))?;
            qualify().map_err(|why| format!(
                "customData: {reason}; the law that follows the engine's storage is refused: {why}"))?;
        }
    }
    if let Some(params) = &emitter.velocity_over_lifetime {
        moly_law::particle::velocity::VelocityOverLifetime::from_params(params)
            .map_err(|reason| format!("velocityOverLifetime: {reason}"))?;
    }
    moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier)
        .map_err(|reason| format!("start.gravityModifier: {reason}"))?;
    // The legacy step's start values, sampled at the normalized cycle time.
    let start = &emitter.start;
    for (name, curve) in [("lifetime", Some(&start.lifetime)), ("size", Some(&start.size)),
        ("sizeY", start.size_y.as_ref()), ("sizeZ", start.size_z.as_ref()),
        ("rotation", Some(&start.rotation)), ("rotationX", start.rotation_x.as_ref()),
        ("rotationY", start.rotation_y.as_ref()), ("speed", Some(&start.speed)),
        ("gravityModifier", Some(&start.gravity_modifier))] {
        if let Some(curve) = curve {
            CurveSampler::new(curve, CurveTime::Normalized).map_err(|reason| format!("start.{name}: {reason}"))?;
        }
    }
    if let Some(emission) = &emitter.emission {
        CurveSampler::new(&emission.rate_over_time, CurveTime::Normalized)
            .map_err(|reason| format!("emission.rateOverTime: {reason}"))?;
        if emission.bursts.iter().any(|burst| matches!(burst.count,
            MinMaxCurve::Curve { .. } | MinMaxCurve::TwoCurves { .. })) {
            return Err("emission burst count curve: the time the engine evaluates it at is not transcribed".into());
        }
    }
    Ok(())
}

/// The CustomData law the weather host installs: without storage where the
/// history-independence certificate admits every lane, otherwise the law that
/// follows the engine's storage ([`moly_law::particle::custom_data::CustomData::with_storage`]),
/// which [`curve_admission_with`] admitted only for a qualified system and
/// which runs only with the native birth owner.
pub(crate) fn custom_data_law(params: &moly_law::particle::schema::CustomDataParams)
    -> Result<moly_law::particle::custom_data::CustomData, &'static str> {
    use moly_law::particle::custom_data::CustomData;
    CustomData::from_params(params)
        .or_else(|_| CustomData::with_storage(params, moly_law::particle::slot_tail::RESERVED_SLOTS))
}

/// The size law the weather host installs: the form that evaluates where a
/// consumer asks, where the history-independence certificate admits every
/// lane, otherwise the form that follows the engine's SizeModule calls,
/// which [`curve_admission_with`] admitted only for a qualified system and
/// which runs only with the native birth owner.
pub(crate) fn size_over_lifetime_law(emitter: &EmitterParams)
    -> Option<Result<moly_law::particle::size::SizeOverLifetime, &'static str>> {
    use moly_law::particle::size::SizeOverLifetime;
    let params = emitter.size_over_lifetime.as_ref()?;
    Some(SizeOverLifetime::from_params(params).or_else(|_|
        SizeOverLifetime::with_storage(params, size_3d(emitter), moly_law::particle::slot_tail::RESERVED_SLOTS)))
}

/// Whether the particle arrays carry three sizes: `AllocateParticleArrays`
/// sets it for the Initial module's 3D start size and for an enabled
/// SizeModule with separate axes (SizeBySpeed has no consumer here).
fn size_3d(emitter: &EmitterParams) -> bool {
    emitter.start.size3d || emitter.size_over_lifetime.as_ref().is_some_and(|size| size.separate_axes)
}

/// Whether a system's size law can follow the engine's SizeModule calls.
///
/// The engine calls the module per slice after the simulation, and in each
/// newborn block, when a module reads the size during the slices (drag by
/// size, collision, lights with size affecting range, a trail flag, trigger);
/// otherwise once after the slices over the whole live storage, which this
/// runtime reproduces. So this law needs: none of those modules (Lights and
/// Trigger have no consumer here; any trail is refused, its flag not
/// identified), no Noise size amount (whose size update after the module is
/// not ported), the native birth path and every storage condition the slot
/// model has: no ring mode and every lane simulated (a positive start
/// lifetime). Play reserves the smaller of the authored maximum and
/// `CalculateMaxActiveParticles`, which adds the ceiling of the largest
/// lifetime times the largest rate to the largest summed count of the bursts
/// within one lifetime of a burst (each count truncated); a birth past that
/// reservation grows it, keeping every slot the model follows.
pub(crate) fn size_storage_eligible(emitter: &EmitterParams, route: &SourceRoute) -> Result<(), String> {
    use moly_law::particle::MinMaxCurve;
    native_birth_eligible(emitter, route).map_err(|reason| format!("the native birth path is refused: {reason}"))?;
    if emitter.ring_buffer_mode != RingBufferMode::Disabled {
        return Err("a ring buffer mode packs newborns in an order the slot model does not follow".into());
    }
    if emitter.collision.is_some() {
        return Err("the CollisionModule reads the size during the slices, where the module then runs".into());
    }
    if emitter.limit_velocity.as_ref().is_some_and(|limit| limit.multiply_drag_by_size != Some(false)
        && limit.drag.as_ref().is_some_and(|drag| *drag != MinMaxCurve::Constant(0.0))) {
        return Err("drag by size reads the size during the slices, where the module then runs".into());
    }
    if emitter.trails.is_some() {
        return Err("a trail's size flag moves the module into the slices; which trail field it is was not read".into());
    }
    if emitter.noise.as_ref().is_some_and(|noise| noise.size_amount != MinMaxCurve::Constant(0.0)) {
        return Err("the Noise size update after the module is not ported".into());
    }
    let lifetime = match emitter.start.lifetime {
        MinMaxCurve::Constant(value) => Some(value),
        MinMaxCurve::TwoConstants { min, max } if min > 0.0 && max > 0.0 => Some(min.max(max)),
        _ => None,
    }.filter(|value| value.is_finite() && *value > 0.0)
        .ok_or("a start lifetime that is not one or two positive constants")?;
    let emission = emitter.emission.as_ref().ok_or("no emission block: the storage reservation is not known")?;
    let upper = |curve: &MinMaxCurve| match curve {
        MinMaxCurve::Constant(value) => Some(*value),
        MinMaxCurve::TwoConstants { min, max } => Some(min.max(*max)),
        _ => None,
    };
    let rate = upper(&emission.rate_over_time).zip(upper(&emission.rate_over_distance))
        .map(|(time, distance)| time + distance)
        .ok_or("an emission rate curve: the storage reservation is not known")?;
    let burst = emission.bursts.iter().any(|burst| match burst.count {
        MinMaxCurve::Constant(value) => value >= 1.0,
        MinMaxCurve::TwoConstants { max, .. } => max >= 1.0,
        _ => false,
    });
    if emitter.max_particles == 0 || !(lifetime * rate > 0.0 || burst) {
        return Err("no positive storage reservation".into());
    }
    Ok(())
}

/// Whether a system's CustomData law can follow the engine's storage past the
/// live count. It needs the native birth path (the engine's slot order and its
/// newborn spans), the storage without a ring mode (whose packing is not
/// modelled) and without a CollisionModule (whose calls also run over the lanes
/// past the count), and every lane simulated (a positive start lifetime). The
/// slots the model follows must lie in the storage Play reserved: with a
/// positive maximum and a positive estimate (a positive largest start
/// lifetime times a positive rate) its first 32 slots always do.
pub(crate) fn custom_data_storage_eligible(emitter: &EmitterParams, route: &SourceRoute) -> Result<(), String> {
    use moly_law::particle::MinMaxCurve;
    native_birth_eligible(emitter, route).map_err(|reason| format!("the native birth path is refused: {reason}"))?;
    if emitter.ring_buffer_mode != RingBufferMode::Disabled {
        return Err("a ring buffer mode packs newborns in an order the slot model does not follow".into());
    }
    if emitter.collision.is_some() {
        return Err("the CollisionModule's calls also run over the lanes past the live count, which the slot model does not follow".into());
    }
    let lifetime = match emitter.start.lifetime {
        MinMaxCurve::Constant(value) => Some(value),
        MinMaxCurve::TwoConstants { min, max } if min > 0.0 && max > 0.0 => Some(min.max(max)),
        _ => None,
    }.filter(|value| value.is_finite() && *value > 0.0)
        .ok_or("a start lifetime that is not one or two positive constants")?;
    let rate = match emitter.emission.as_ref().map(|emission| &emission.rate_over_time) {
        Some(MinMaxCurve::Constant(value)) if value.is_finite() && *value > 0.0 => *value,
        _ => return Err("an emission rate that is not a positive constant: the storage reservation is not known".into()),
    };
    if emitter.max_particles == 0 || !(lifetime * rate > 0.0) {
        return Err("no positive storage reservation".into());
    }
    Ok(())
}

fn normalized_time(system: &Runtime, head: f32) -> f32 {
    let duration = system.emitter.duration;
    if !(duration.is_finite() && duration > 0.0) {
        return 0.0;
    }
    (head / duration).clamp(0.0, 1.0)
}

fn spawn_one(system: &mut Runtime, ctx: &Context) {
    let (position, direction, store_direction) = if let Some(shape) = system.emitter.shape.as_ref() {    // Current native RNG consumption: Circle/Cone 2, Sphere/Hemisphere 3,
    // SingleSidedEdge 1. A billboard's facing direction is not its birth velocity.
    let (local, raw_dir) = match shape.shape_type.as_str() {
        "Circle" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            circle_base(shape.radius, shape.radius_thickness, shape.arc, arc, radial)
        }
        "Cone" => {
            let angle = system
                .cone_angle
                .expect("判读已门 Cone 的 angle 在场");
            let t_theta = system.rng.next_f32();
            let t_radial = system.rng.next_f32();
            cone_base(
                shape.radius,
                shape.radius_thickness,
                angle,
                shape.arc,
                t_theta,
                t_radial,
            )
        }
        "ConeVolume" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            let distance = system.rng.next_f32();
            cone_volume(shape.radius, shape.radius_thickness,
                shape.controls.angle.expect("validated source cone angle"), shape.arc,
                shape.controls.length.expect("validated source cone length"), arc, radial, distance)
        }
        "Sphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            sphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Hemisphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            hemisphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Donut" => {
            let major_arc = system.rng.next_f32();
            let tube_angle = system.rng.next_f32();
            let radial = system.rng.next_f32();
            donut_position(shape.radius, shape.controls.donut_radius.expect("validated source donut radius"),
                shape.radius_thickness, shape.arc, major_arc, tube_angle, radial)
        }
        "Mesh" => {
            let source = system.emission_surface.as_ref().expect("source surface must be ready before emitter installation");
            let selector = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            let u = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            let v = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            source.sample(selector, u, v)
        }
        "SingleSidedEdge" => {
            let t_theta = system.rng.next_f32();
            single_sided_edge(shape.radius, t_theta)
        }
        other => panic!("判读已门形状族，运行时遇到 {other}——判读与推进的门不一致"),
    };
    // EmitterStoreData owns two additional draws for a positive authored
    // position jitter. Do not consume them for zero; later birth streams must
    // retain their original sequence. Apply this BEFORE the shape transform.
    let amount = shape.controls.random_position.unwrap_or(0.0);
    let local = if amount > 0.0 {
        let arc = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
        let polar = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
        moly_law::particle::shape::randomize_position(local, amount, arc, polar)
    } else { local };
    // Shape-module TRS scales both position and velocity before rotation.
    // Independent engine measurements and current EmitterStoreData agree;
    // treating the scale as a billboard-size control would flatten the wrong data.
    let shape_scale = shape.controls.scale.unwrap_or([1.0; 3]);
    let local = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| local[i] * shape_scale[i]));
    let position = [
        local[0] + shape.position[0],
        local[1] + shape.position[1],
        local[2] + shape.position[2],
    ];
    // 出发方向：形状函数的方向经形状旋转后归一（锥形函数按引擎分工返回
    // 未归一向量，归一在调用方）。
    // The Shape module's store turns the direction, normalised first (+Z
    // when short), through the Shape affine and writes the axis of rotation
    // from it (`store_axis_of_rotation`, below).
    let unit = Vec3::from_array(raw_dir).try_normalize().unwrap_or(Vec3::Z).to_array();
    let store_direction = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| unit[i] * shape_scale[i]));
    let mut direction = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| raw_dir[i] * shape_scale[i]));
    let length = (direction[0] * direction[0]
        + direction[1] * direction[1]
        + direction[2] * direction[2])
    .sqrt();
    if length > 1e-30 {
        direction = [
            direction[0] / length,
            direction[1] / length,
            direction[2] / length,
        ];
    } else {
        direction = [0.0; 3];
    }

    (position, direction, Some(store_direction))
    } else {
        assert_eq!(system.emitter.shape_enabled, Some(false), "missing shape requires explicit disabled-module evidence");
        ([0.0; 3], [0.0, 0.0, 1.0], None)
    };

    // ---- 出生取值表（表情链转录：逐项各抽一次，速度与重力共用稳定
    // 因子，种子 u32 最后一抽）----
    // Start values are sampled at the normalized emission time. The legacy step
    // births a whole slice at its start, so that is the time it uses.
    let t0 = normalized_time(system, system.previous_head);
    let r = system.rng.next_f32();
    // Each start value goes through the engine's curve dispatch; a curve the
    // judge did not admit refuses this birth and is counted.
    let start = &system.emitter.start;
    let rng = &mut system.rng;
    let sample = |curve: &moly_law::particle::MinMaxCurve, random: f32| cycle_curve(curve, t0, random);
    let values = (|| -> Result<_, &'static str> {
        let lifetime = sample(&start.lifetime, rng.next_f32())?.max(0.01);
        let size_x = sample(&start.size, rng.next_f32())?;
        let size_y = match &start.size_y {
            Some(curve) => sample(curve, rng.next_f32())?,
            None => size_x,
        };
        let size_z = match &start.size_z {
            Some(curve) => sample(curve, rng.next_f32())?,
            None => size_x,
        };
        let colour = moly_law::particle::gradient::rgba8_to_float(
            moly_law::particle::color::initial_rgba8(&start.color, t0, rng.next_f32()));
        let spin0 = sample(&start.rotation, rng.next_f32())?;
        let (spin_x, spin_y) = if start.rotation3d {
            (sample(start.rotation_x.as_ref().expect("validated source X rotation"), rng.next_f32())?,
             sample(start.rotation_y.as_ref().expect("validated source Y rotation"), rng.next_f32())?)
        } else { (0.0, 0.0) };
        let speed = sample(&start.speed, r)?;
        let gravity = sample(&start.gravity_modifier, r)?;
        Ok((lifetime, size_x, size_y, size_z, colour, spin0, spin_x, spin_y, speed, gravity))
    })();
    let (lifetime, size_x, size_y, size_z, colour, spin0, spin_x, spin_y, speed, gravity) = match values {
        Ok(values) => values,
        Err(reason) => {
            system.refused_total += 1;
            error!(%reason, effect=%system.effect, node=%system.node, "legacy start value refused");
            return;
        }
    };
    let seed = system.rng.next_u32();

    // ---- 空间锚定：世界空间仿真出生即锚（位置过全变换、方向过线性部
    // 后已归一），局部空间仿真留在节点局部系逐帧换算 ----
    let kind = system.kind;
    let camera_rotation = system.camera_rotation;
    let node_affine = system.node_affine;
    // 尺寸吃链缩放：语料 35 条**局部空间**条目带非恒等链缩放（4.0 ×28、
    // 0.9994 ×7，全部均匀；世界空间条目的链全恒等），折进出生尺寸与
    // 「渲染时折算」给出同一结果。非均匀链缩放语料里不存在，按 X 分量
    // 处理（具名记为未实现）。
    // Particle module coordinates are raw Unity coordinates; GLB and game
    // anchors use the producer's reflected-X basis. Convert once at this boundary.
    let shape_point = position;
    let position = crate::particle_geometry::reflect(Vec3::from_array(position)).to_array();
    let direction = crate::particle_geometry::reflect(Vec3::from_array(direction)).to_array();
    let (position, direction, world_owner) = if system.emitter.simulation_space == SimulationSpace::World {
        let anchor = match kind {
            EffectKind::Sky => ctx.sky,
            EffectKind::Camera => {
                if camera_rotation {
                    ctx.camera
                } else {
                    GlobalTransform::from_translation(ctx.camera.translation())
                }
            }
            EffectKind::Site => ctx.site,
        };
        let to_world = anchor * node_affine;
        let position = to_world.transform_point(Vec3::from_array(position)).to_array();
        // Native EmitterStoreData has already normalized the direction in
        // emitter space.  The final owner 3x3 is a plain multiply: JP 6.8.1
        // does not normalize after it, so non-uniform owner scale remains in
        // the velocity magnitude.  Do not replace this with normalize_or_zero.
        let dir = if system.emitter.shape.is_some() {
            apply_world_owner_direction(&to_world, Vec3::from_array(direction))
        } else {
            // With Shape disabled Initial.Start owns the direction and
            // normalizes the owner's Z column before StartVelocity.
            to_world.affine().transform_vector3(Vec3::from_array(direction)).normalize_or_zero()
        };
        (position, dir.to_array(), Some(to_world))
    } else {
        (position, direction, None)
    };
    // The store's axis crosses +Z with the turned direction, or with the
    // point the owner turned (before translation) when that cross is short;
    // Local simulation has no owner turn. Without a Shape module the Initial
    // module's +Z stands.
    let axis = store_direction.map_or([0.0, 0.0, 1.0], |store| {
        let turned = world_owner.map_or(shape_point, |owner| crate::particle_geometry::reflect(
            owner.affine().transform_vector3(crate::particle_geometry::reflect(Vec3::from_array(shape_point)))).to_array());
        moly_law::particle::shape_birth::store_axis_of_rotation(store, turned)
    });
    let velocity = [
        direction[0] * speed,
        direction[1] * speed,
        direction[2] * speed,
    ];

    let side = Side {
        rand: r,
        seed,
        rot: [spin_x, spin_y, spin0],
        size: [size_x, size_y, size_z],
        gravity,
        colour,
        total_velocity: velocity,
        custom_data: [[0.0; 4]; 2],
        emit_carry: [0.0; 2],
        animated: [0.0; 3],
        current_size: 0.0,
        axis,
    };
    let particle = Particle::born(position, velocity, lifetime);
    system.pool.push(particle);
    system.side.push(side);
    system.born_total += 1;
}



/// The per-particle draw inputs of the source-geometry renderers, in the
/// source basis: position, velocity, rotation, size, colour, custom data,
/// seed, age and axis of rotation.
pub(crate) fn geometry_instances(system: &Runtime, to_world: &GlobalTransform) -> Vec<crate::particle_geometry::Instance> {
    let appearance = build_quads(system, to_world);
    system.pool.iter().enumerate().map(|(index, particle)| {
        let side = system.side[index];
        let size = Vec3::from_array(motion::current_size(system, index, particle.age_percent));
        let view = &appearance[index];
        crate::particle_geometry::Instance {
            position: crate::particle_geometry::reflect(view.centre),
            velocity: crate::particle_geometry::reflect(to_world.affine().transform_vector3(Vec3::from_array(side.total_velocity))),
            rotation: Vec3::from_array(side.rot), size,
            colour: Vec4::from_array(view.colour),
            custom1: Vec4::from_array(view.custom1), custom2: Vec4::from_array(view.custom2),
            seed: side.seed, age_percent: particle.age_percent,
            axis: Vec3::from_array(side.axis),
        }
    }).collect()
}

/// A bevy-basis matrix in the source basis (the X mirror on both sides):
/// every element in exactly one of the X row and the X column changes sign.
fn source_matrix(m: Mat4) -> [f32; 16] {
    let mut a = m.to_cols_array();
    for c in 0..4 {
        for r in 0..4 {
            if (r == 0) != (c == 0) { a[4 * c + r] = -a[4 * c + r]; }
        }
    }
    a
}

/// The camera's world to camera matrix for a source-basis world: the bevy
/// view with its X column negated.
fn source_view(camera: &GlobalTransform) -> [f32; 16] {
    let mut a = Mat4::from(camera.affine().inverse()).to_cols_array();
    for r in 0..4 { a[r] = -a[r]; }
    a
}

pub(crate) fn write_geometry(
    mesh: &mut Mesh, system: &Runtime, to_world: &GlobalTransform,
    owner: &GlobalTransform, camera: &GlobalTransform, basis: crate::billboard::CameraBasis,
) {
    match &system.geometry {
        Geometry::Billboard { alignment, clamp, pivot } => {
            crate::billboard::write_quads(mesh, &build_quads(system, to_world), *alignment, basis, *clamp, *pivot);
        }
        Geometry::Mesh(_) | Geometry::SourceBillboard(_) => {
            let base_frame = crate::particle_geometry::source_frame(owner, camera);
            let frame = match &system.geometry {
                Geometry::Mesh(draw) => draw.scaling.apply(base_frame),
                Geometry::SourceBillboard(draw) => draw.scaling.apply(base_frame),
                _ => unreachable!(),
            };
            let mut instances = geometry_instances(system, to_world);
            match &system.geometry {
                Geometry::Mesh(draw) => crate::particle_geometry::write_mesh(mesh, draw, &instances, &frame),
                Geometry::SourceBillboard(draw) => {
                    // The Velocity basis reads the velocity in the simulation
                    // space and that space's rotation to world (source basis:
                    // the X mirror on both sides). Its composition is read for
                    // unit-scale owners only; a scaled one draws zero-area
                    // quads (the vertex and index cardinality the sheet and
                    // sort rely on stays), loudly.
                    let m = to_world.affine().matrix3;
                    let reflect = crate::particle_geometry::reflect;
                    let simulation = Mat3::from_cols(-reflect(Vec3::from(m.x_axis)), reflect(Vec3::from(m.y_axis)), reflect(Vec3::from(m.z_axis)));
                    let unit = |s: Vec3| (s - Vec3::ONE).abs().max_element() <= 1.0e-5;
                    if draw.alignment == crate::particle_geometry::Alignment::Velocity {
                        for (instance, side) in instances.iter_mut().zip(&system.side) {
                            instance.velocity = reflect(Vec3::from_array(side.total_velocity));
                        }
                        if !(unit(frame.scale) && unit(to_world.to_scale_rotation_translation().0)) {
                            warn_once!(effect = %system.effect, node = %system.node,
                                "Velocity billboard over a non-unit owner scale: the scaled basis composition is not read; drawn empty");
                            for instance in &mut instances { instance.size = Vec3::ZERO; }
                        }
                    }
                    crate::source_billboard::write(mesh, draw, &instances, &frame,
                        system.emitter.simulation_space == SimulationSpace::Local, basis.fov_y, basis.aspect, simulation,
                        reflect(basis.velocity))
                }
                _ => unreachable!(),
            }
        }
    }
    let vertices_per_particle = match &system.geometry {
        Geometry::Mesh(draw) => draw.source.positions.len(),
        Geometry::Billboard { .. } | Geometry::SourceBillboard(_) => 4,
    };
    // An empty mesh cache wrote no vertex, so there is no corner to retile.
    if let Some(sheet) = system.texture_sheet.filter(|_| vertices_per_particle > 0) {
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) else {
            unreachable!("shared particle geometry always writes UV0");
        };
        assert_eq!(uv.len(), vertices_per_particle * system.side.len(), "sheet and particle geometry cardinality");
        for (corners, side) in uv.chunks_exact_mut(vertices_per_particle).zip(&system.side) {
            let rect = sheet.rect(side.seed);
            for corner in corners { *corner = rect.apply(*corner); }
        }
    }
    // Reorder draw indices, preserving pool order and its particle-local random
    // streams. Attributes and atlas coordinates still belong to the same seed.
    if system.sort_mode != moly_law::particle::sort::ParticleSort::None && !system.pool.is_empty() {
        // The sort runs in the source basis: positions and both matrices are
        // mirrored in X (exact sign changes), the view being the camera's world
        // to camera matrix, whose view space is the same in both bases.
        let particles: Vec<_> = system.pool.iter().map(|p| moly_law::particle::sort::SortParticle {
            position: [-p.position[0], p.position[1], p.position[2]], age_percent: p.age_percent,
            inverse_lifetime: p.inverse_lifetime,
        }).collect();
        let sort_camera = moly_law::particle::sort::SortCamera {
            view: source_view(camera), owner: source_matrix(Mat4::from(to_world.affine())),
            near: basis.near, orthographic: false,
        };
        let order = match system.sort_mode.indices(&particles, &sort_camera) {
            Ok(order) => order,
            Err(refused) => {
                warn_once!(?refused, "particle sort refused: the pool order is drawn");
                (0..particles.len()).collect()
            }
        };
        let per_particle = match &system.geometry {
            Geometry::Mesh(draw) => draw.source.indices.len(),
            Geometry::Billboard { .. } | Geometry::SourceBillboard(_) => 6,
        };
        let Some(bevy::mesh::Indices::U32(indices)) = mesh.remove_indices() else {
            unreachable!("shared particle geometry writes u32 indices");
        };
        assert_eq!(indices.len(), per_particle * order.len(), "particle sort index cardinality");
        let sorted = order.into_iter().flat_map(|index|
            indices[index * per_particle..(index + 1) * per_particle].iter().copied()).collect();
        mesh.insert_indices(bevy::mesh::Indices::U32(sorted));
    }
}
