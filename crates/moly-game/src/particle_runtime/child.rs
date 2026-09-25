//! A sub-emitter command applied to its target system's one shared pool, as
//! the engine's child Emit does it: the start matrix (the command velocity's
//! direction looked along, the target's own rotation, the command position,
//! the target's world-to-local unless it is World-space, the emitter scale),
//! the capacity clip, Initial and Shape over whole four-lane groups,
//! StartVelocity, the newborn pre-simulation modules and position and age
//! update, the newborn deaths, the catch-up steps, and the pack into the
//! alignment gap. The old particles of the pool are never touched.
//!
//! Qualified target composition: Initial with constant or two-constant
//! lifetime, size, speed and rotation, a constant start colour source the
//! Initial law takes, a constant gravity modifier, Shape through the target's
//! own Shape law (or no Shape with zero speed), RotationOverLifetime with
//! constant or two-constant axes, SizeOverLifetime and ColorOverLifetime
//! (render-time), CustomData, no ring buffer, simulation speed one. Any other
//! module on the target refuses. The inherited block must be the neutral one
//! (the parent inherits nothing); the parent particle's seed it ends with is
//! not a child random word.
//!
//! The catch-up runs only when the parent's update flags enable it (bit 0 or
//! bit 2); the per-frame update passes no such flag. Size is not stored: the
//! renderer evaluates the size law at the final age, which is what the
//! engine's size write after the catch-up holds.
//!
//! In the product a target is installed with its own seed owner and the
//! owner words of its authored chain; its own frame is the stopped update
//! (the engine marks every target stopped each frame), and its parent's
//! commands of a frame are delivered after every system's frame, in the order
//! the parent recorded them, with the world's default gravity.
//!
//! A World-space target may itself own birth edges. The engine's child Emit
//! runs the target's own StartModules, whose sub-emitter call records the
//! newborns' birth events once per new four-lane group (after the newborn
//! position and age update, before the newborn deaths), and each event's
//! RecordEmit emits into that target's children at once, inside the parent's
//! update. Every target has one parent, so handing the recorded commands to
//! the next level after the command, until no command is left in the frame,
//! gives each child the same command stream in the same order. The recording
//! works on a copy that is kept only when the whole command succeeds.
use super::*;
use moly_law::particle::{
    child_emit::{self, ChildOwner, StartFrame},
    curve::CurveSampler,
    initial::{InitialContext, InitialGroupInput, InitialLaw},
    random::ParticleRandom,
    seed_owner::ModuleRandom,
    schema::ShapeMode,
    shape::ArcLoopClock,
    shape_birth::{ShapeBatch, ShapeBirthLaw, ShapeSample},
    sub_emission::BirthDistribution,
    MinMaxCurve,
};

const MAX_EXACT_COUNT: u64 = 16_777_215;
/// Newborn and catch-up ages are capped at this (just above 100 percent).
const AGE_CAP: f32 = f32::from_bits(0x42c8_0001);
/// The world's gravity in source axes (y up), as the physics settings hold it.
pub(super) const SOURCE_GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChildCommand {
    pub count: u64,
    pub rate_count: u64,
    pub distribution: BirthDistribution,
    /// Source-world coordinates, not yet reflected to runtime coordinates.
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub inherited_words: [u32; 13],
    pub dt: f32,
    pub previous: f32,
    pub current: f32,
    pub catch_up: f32,
}

/// The parent's update inputs the command reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct ChildUpdate {
    /// The parent's update flags; only bits 0 and 2 are read here.
    pub flags: u32,
    pub frame_dt: f32,
    pub world_playing: bool,
    /// The world's gravity the Initial update reads, source axes.
    pub gravity: [f32; 3],
}

#[derive(Debug, PartialEq)]
pub(super) enum Refused {
    InvalidCommand(&'static str),
    Unsupported(&'static str),
    Initial(moly_law::particle::initial::Refused),
    Shape(moly_law::particle::shape_birth::Refused),
}

#[derive(Debug, PartialEq)]
pub(super) struct Applied {
    pub born: usize,
    pub catch_up_steps: usize,
    pub catch_up_remainder: f32,
    /// The command's start frame, when the command got that far.
    pub start: Option<StartFrame>,
}

/// The Shape step of one command: one call per new four-lane group, in order,
/// with the command's start matrix as the owner (its rotation columns and its
/// translation), returning the stored position and direction of all four
/// lanes. The seam works on its own copy of the Shape stream; the caller
/// commits that copy only after the whole command succeeded.
pub(super) trait ChildShape {
    fn group(&mut self, start: &[f32; 16]) -> Result<[ShapeSample; 4], Refused>;
}

/// The target's own Shape law with its Shape stream: the shape transform is
/// scaled by the target's shape scale and the store uses the start matrix.
pub(super) struct SourceShape {
    pub law: ShapeBirthLaw,
    pub stream: ModuleRandom,
    pub shape_scale: [f32; 3],
    pub uses_axis_of_rotation: bool,
}

impl ChildShape for SourceShape {
    fn group(&mut self, start: &[f32; 16]) -> Result<[ShapeSample; 4], Refused> {
        // Admission refuses a target whose kernel reads the birth batch (see
        // child_target_eligible), so the batch here is never read.
        let mut batch = ShapeBatch::new(std::num::NonZeroU32::MIN, 0.0, 0.0, ArcLoopClock::default());
        self.law
            .sample_group(&mut batch, &mut self.stream, *start, true, self.shape_scale, self.uses_axis_of_rotation)
            .map(|group| group.samples)
            .map_err(Refused::Shape)
    }
}

impl ChildCommand {
    /// Current ARM64 layout. The pointer at +0 is not dereferenced: the caller
    /// supplies its separately captured twelve-byte emission payload. Padding
    /// at +0x54 is not a semantic field. Preserve all inherited parameter bits.
    pub fn from_native_bytes(raw: &[u8], emission: &[u8]) -> Result<Self, Refused> {
        if raw.len() != 0x78 || emission.len() != 12 {
            return Err(Refused::InvalidCommand("native command/payload length"));
        }
        let word = |offset: usize| {
            u32::from_le_bytes([raw[offset], raw[offset + 1], raw[offset + 2], raw[offset + 3]])
        };
        let float = |offset| f32::from_bits(word(offset));
        let long = |offset: usize| u64::from(word(offset)) | (u64::from(word(offset + 4)) << 32);
        let payload = |offset: usize| {
            f32::from_le_bytes([emission[offset], emission[offset + 1], emission[offset + 2], emission[offset + 3]])
        };
        Ok(Self {
            count: long(0x58),
            rate_count: long(0x60),
            distribution: BirthDistribution {
                spacing: payload(0),
                offset: payload(4),
                burst_fraction: payload(8),
            },
            position: std::array::from_fn(|a| float(8 + a * 4)),
            velocity: std::array::from_fn(|a| float(20 + a * 4)),
            inherited_words: std::array::from_fn(|i| word(0x20 + i * 4)),
            dt: float(0x68),
            previous: float(0x6c),
            current: float(0x70),
            catch_up: float(0x74),
        })
    }

    /// The command as the parent's event recording issued it.
    pub fn from_event(command: &moly_law::particle::sub_emission::SubEmitterCommand) -> Self {
        Self {
            count: command.count,
            rate_count: command.rate_count,
            distribution: command.emission,
            position: command.position,
            velocity: command.velocity,
            inherited_words: command.inherited,
            dt: command.dt,
            previous: command.previous_normalized,
            current: command.current_normalized,
            catch_up: command.catch_up,
        }
    }

    fn validate(&self) -> Result<(), Refused> {
        if self.count > MAX_EXACT_COUNT || self.rate_count > self.count {
            return Err(Refused::InvalidCommand("count outside exact range"));
        }
        if self
            .position
            .iter()
            .chain(self.velocity.iter())
            .any(|v| !v.is_finite())
            || !self.catch_up.is_finite()
        {
            return Err(Refused::InvalidCommand("nonfinite command position, velocity or catch-up"));
        }
        // Parent command seed is retained separately; it is not child RandN.
        // Native Math initialization supplies Vector3.forward, including the
        // +1 at command +0x44. Zero there came from an uninitialized probe VM.
        let neutral = [
            u32::MAX,
            0x3f800000,
            0x3f800000,
            0x3f800000,
            0,
            0,
            0,
            0,
            0,
            0x3f800000,
            0x3f800000,
            0x7f800000,
        ];
        if self.inherited_words[..12] != neutral {
            return Err(Refused::Unsupported("non-neutral inherited context"));
        }
        self.distribution
            .timing(0, self.rate_count as u32, self.dt, self.previous, self.current)
            .map_err(|_| Refused::InvalidCommand("birth distribution"))?;
        Ok(())
    }
}

/// The target laws one command uses, read from the target's modules.
struct ChildLaws {
    initial: InitialLaw,
    speed: CurveSampler,
    gravity_modifier: f32,
    /// The start-lifetime scalar the world-playing gate compares the
    /// catch-up with: the constant, or the two-constant maximum.
    upper_lifetime: f32,
    /// The particle arrays carry 3D rotation (Initial rotation3D or a
    /// RotationOverLifetime with separate axes) and angular speed (a
    /// RotationOverLifetime module).
    rotation_3d: bool,
    angular_speed: bool,
}

/// Whether the target's modules are within the qualified composition.
pub(super) fn qualify_target(emitter: &EmitterParams) -> Result<(), Refused> {
    child_laws(emitter).map(|_| ())
}

fn child_laws(emitter: &EmitterParams) -> Result<ChildLaws, Refused> {
    let unsupported = |reason| Err(Refused::Unsupported(reason));
    if emitter.ring_buffer_mode != RingBufferMode::Disabled {
        return unsupported("ring-buffer target");
    }
    if emitter.simulation_speed != 1.0 {
        return unsupported("target simulation speed other than one");
    }
    if emitter.start.size3d {
        return unsupported("target 3D start size");
    }
    if emitter.velocity_over_lifetime.is_some()
        || emitter.force.is_some()
        || emitter.limit_velocity.is_some()
        || emitter.inherit_velocity.is_some()
        || emitter.noise.is_some()
        || emitter.collision.is_some()
        || emitter.trails.is_some()
        || emitter.texture_sheet.is_some()
    {
        return unsupported("target module outside the child composition");
    }
    // A target's own edges: birth edges only, recorded by the newborn call
    // inside the child Emit and by its own post-simulation call. Only a
    // World-space target reads no owner for its events (a Local one would
    // need the event owner words of its authored chain).
    if birth::has_real_sub_emitter_edges(emitter) {
        let births = emitter.sub_emitters.iter().all(|edge|
            edge.trigger == moly_law::particle::schema::SubEmitterTrigger::Birth && birth::real_event_edge(edge));
        if !births {
            return unsupported("target with sub-emitter edges other than birth edges");
        }
        if emitter.simulation_space != SimulationSpace::World {
            return unsupported("target with its own birth edges outside World space");
        }
    }
    let scalar = |curve: &MinMaxCurve| matches!(curve, MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. });
    if !scalar(&emitter.start.speed) {
        return unsupported("target start speed curve mode");
    }
    let gravity_modifier = match emitter.start.gravity_modifier {
        MinMaxCurve::Constant(value) if value.is_finite() => value,
        _ => return unsupported("target gravity modifier other than a finite constant"),
    };
    let upper_lifetime = match emitter.start.lifetime {
        MinMaxCurve::Constant(value) => value,
        // The engine's gate reads the max field, not the ordered maximum.
        MinMaxCurve::TwoConstants { min, max } if arms::on("gateUsesOrderedMax") => if max < min { min } else { max },
        MinMaxCurve::TwoConstants { max, .. } => max,
        _ => return unsupported("target start lifetime curve mode"),
    };
    if let Some(rol) = &emitter.rotation_over_lifetime {
        let axes = [Some(&rol.curve), rol.x.as_ref(), rol.y.as_ref()];
        if axes.iter().flatten().any(|curve| !scalar(curve)) {
            return unsupported("target RotationOverLifetime curve mode");
        }
    }
    match (emitter.shape_enabled, emitter.shape.as_ref()) {
        (Some(false), None) => {
            let zero = |curve: &MinMaxCurve| matches!(curve, MinMaxCurve::Constant(v) if *v == 0.0)
                || matches!(curve, MinMaxCurve::TwoConstants { min, max } if *min == 0.0 && *max == 0.0);
            if !zero(&emitter.start.speed) {
                return unsupported("target without Shape and with a start speed");
            }
        }
        (Some(true), Some(_)) => {}
        _ => return unsupported("missing target Shape enabled/configuration evidence"),
    }
    // The source admission fixes the rotation direction randomization at 0.
    let initial = InitialLaw::from_params(&emitter.start, 0.0).map_err(Refused::Initial)?;
    Ok(ChildLaws {
        initial,
        // A constant or two constants here (the curve modes are refused
        // above), so the dispatch has no curve lane to refuse.
        speed: CurveSampler::new(&emitter.start.speed, moly_law::particle::curve::CurveTime::Normalized)
            .map_err(Refused::Unsupported)?,
        gravity_modifier,
        upper_lifetime,
        rotation_3d: emitter.start.rotation3d
            || emitter.rotation_over_lifetime.as_ref().is_some_and(|rol| rol.separate_axes),
        angular_speed: emitter.rotation_over_lifetime.is_some(),
    })
}

/// One newborn lane in source axes while the command runs.
#[derive(Clone, Copy)]
struct Lane {
    position: [f32; 3],
    velocity: [f32; 3],
    animated: [f32; 3],
    rotation: [f32; 3],
    angular: [f32; 3],
    size: f32,
    color: [u8; 4],
    seed: u32,
    age: f32,
    inverse: f32,
    lifetime: f32,
    custom: [[f32; 4]; 2],
    fraction: f32,
    birth_dt: f32,
    /// The carries of the target's own birth edges (zero at birth).
    carry: [f32; 2],
}

/// The target's own birth events while one command runs: its event owner,
/// its system time still to simulate and its emission state word.
pub(super) struct TargetEvents<'a> {
    pub events: &'a mut super::sub_events::BirthEvents,
    pub accumulated: f32,
    pub emission_word: u32,
}

/// Test instrumentation: one named change to the law per replay arm.
#[cfg(test)]
pub(super) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) { ARM.with(|a| a.set(arm)); }
    pub fn on(name: &str) -> bool { ARM.with(|a| a.get() == Some(name)) }
}
#[cfg(not(test))]
pub(super) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool { false }
}

/// Execute one complete command before the next arrival. `owner` holds the
/// target's owner words of the frame the command was issued in; `initial`
/// is the target's Initial stream; `shape` is the Shape step (None when the
/// target has no Shape module). A refusal changes nothing.
pub(super) fn apply_command(
    system: &mut Runtime,
    owner: &ChildOwner,
    initial: &mut ModuleRandom,
    shape: Option<&mut dyn ChildShape>,
    command: &ChildCommand,
    update: ChildUpdate,
) -> Result<Applied, Refused> {
    apply_command_with_events(system, owner, initial, shape, command, update, None)
}

/// [`apply_command`] for a target with its own birth edges: `events` records
/// the newborns' birth events, and its queue and carries change only when
/// the command succeeds.
pub(super) fn apply_command_with_events(
    system: &mut Runtime,
    owner: &ChildOwner,
    initial: &mut ModuleRandom,
    shape: Option<&mut dyn ChildShape>,
    command: &ChildCommand,
    update: ChildUpdate,
    events: Option<TargetEvents<'_>>,
) -> Result<Applied, Refused> {
    if birth::has_real_sub_emitter_edges(&system.emitter) != events.is_some() {
        return Err(Refused::Unsupported("target's own birth events do not match its edges"));
    }
    let noop = |start| Applied {
        born: 0,
        catch_up_steps: 0,
        catch_up_remainder: command.catch_up,
        start,
    };
    if command.count == 0 {
        return Ok(noop(None));
    }
    command.validate()?;
    let laws = child_laws(&system.emitter)?;
    if system.emitter.shape.is_some() != shape.is_some() {
        return Err(Refused::Unsupported("Shape step does not match the target's Shape module"));
    }
    if update.world_playing && command.catch_up >= laws.upper_lifetime && !arms::on("noUpperGate") {
        return Ok(noop(None));
    }
    let world_space = system.emitter.simulation_space == SimulationSpace::World;
    let mut frame_owner = *owner;
    if arms::on("noEmitterScale") {
        frame_owner.emitter_scale = [1.0; 3];
    }
    let mut frame = child_emit::start_frame(&frame_owner, world_space, command.position, command.velocity);
    if arms::on("noInverse") && !world_space {
        frame = child_emit::start_frame(&frame_owner, true, command.position, command.velocity);
    }
    if arms::on("backtrackWorldVelocity") {
        frame.emitter_velocity = command.velocity;
    }
    if arms::on("noLookRotation") {
        // The identity in place of the look rotation (a zero direction fails it).
        frame.matrix = child_emit::start_frame(&frame_owner, world_space, command.position, [0.0; 3]).matrix;
    }
    if frame.matrix.iter().chain(frame.emitter_velocity.iter()).any(|v| !v.is_finite()) {
        return Err(Refused::InvalidCommand("nonfinite start matrix"));
    }
    let mut catch_up = child_emit::catch_up_plan(command.catch_up, update.frame_dt, update.world_playing,
        update.flags, system.emitter.duration);
    if arms::on("noStepRaise") {
        catch_up = child_emit::catch_up_plan(command.catch_up, update.frame_dt, update.world_playing,
            update.flags | 4, system.emitter.duration);
        catch_up.runs &= update.flags & 5 != 0;
    }
    // The catch-up steps run the target's post-simulation modules, whose
    // sub-emitter call is not transcribed on the staged block.
    if events.is_some() && catch_up.runs {
        return Err(Refused::Unsupported("catch-up of a target with its own birth edges"));
    }
    assert_eq!(system.pool.len(), system.side.len());
    let old = system.pool.len();
    let maximum = system.emitter.max_particles as usize;
    let count = command.count as usize;
    let mut accepted = if old >= maximum { 0 } else { (maximum.min(count + old) - old).min(count) };
    let capacity_accepted = accepted;
    let groups = accepted.div_ceil(4);
    let aligned_lanes = groups * 4;

    // Staged: nothing below writes the system until the command succeeded.
    let mut next_initial = *initial;
    let mut lanes: Vec<Lane> = Vec::with_capacity(aligned_lanes);
    let mut shape = shape;
    for group_index in 0..groups {
        let offset = group_index * 4;
        let timing = (0..4)
            .map(|lane| {
                command
                    .distribution
                    .timing((offset + lane) as u32, command.rate_count as u32, command.dt,
                        command.previous, command.current)
                    .map_err(|_| Refused::InvalidCommand("birth timing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let group = laws
            .initial
            .start_group_native(
                &mut next_initial,
                InitialGroupInput {
                    active_lanes: (accepted - offset).min(4),
                    storage_size_3d: false,
                    storage_rotation_3d: laws.rotation_3d,
                    birth_fraction: std::array::from_fn(|lane| timing[lane].fraction),
                    // The command's current normalized time, broadcast.
                    curve_time: [command.current; 4],
                    context: InitialContext::Autonomous,
                },
            )
            .map_err(Refused::Initial)?;
        let translation = [frame.matrix[12], frame.matrix[13], frame.matrix[14]];
        let samples = match shape.as_deref_mut() {
            Some(shape) => shape.group(&frame.matrix)?,
            // Without Shape every newborn keeps what Initial's start wrote:
            // the start translation and the unit z axis of the start matrix.
            None => [ShapeSample { position: translation, direction: child_emit::unshaped_direction(&frame.matrix) }; 4],
        };
        for (index, initial_lane) in group.lanes.into_iter().enumerate() {
            let sample = samples[index];
            let speed = laws.speed.evaluate(timing[index].curve_time,
                ParticleRandom::sample(initial_lane.seed, 0x96aa_4de3));
            lanes.push(Lane {
                position: sample.position,
                velocity: sample.direction.map(|d| speed * d),
                animated: [0.0; 3],
                rotation: initial_lane.rotation.map(|v| v.unwrap_or(0.0)),
                angular: [0.0; 3],
                size: initial_lane.size[0].unwrap_or(0.0),
                color: initial_lane.color,
                seed: initial_lane.seed,
                age: 0.0,
                inverse: initial_lane.inverse_lifetime,
                lifetime: initial_lane.lifetime,
                custom: [[0.0; 4]; 2],
                fraction: timing[index].fraction,
                birth_dt: if arms::on("birthDtIsCommandDt") { command.dt } else { timing[index].dt },
                carry: [0.0; 2],
            });
        }
    }

    // Newborn pre-simulation modules with each lane's birth dt, then the
    // newborn position and age update; the placement moves each birth back
    // along the command velocity in the target's space.
    let back_scale = command.dt / system.emitter.simulation_speed;
    let gravity_space = (!world_space && !arms::on("gravityWorld")).then_some(&owner.world_to_local);
    for lane in &mut lanes {
        pre_modules(system, &laws, lane, lane.birth_dt, update.gravity, gravity_space);
    }
    for lane in &mut lanes {
        let dt = lane.birth_dt;
        let age = if lane.age >= 100.0 { lane.age } else { (dt * 100.0) * lane.inverse };
        lane.age = child_emit::min_propagating(age, AGE_CAP);
        let back = back_scale * (0.0 + lane.fraction);
        for a in 0..3 {
            let placed = lane.position[a] - back * frame.emitter_velocity[a];
            lane.position[a] = placed + (lane.velocity[a] + lane.animated[a]) * (1.0 * dt);
        }
        integrate_rotation(&laws, lane, dt);
    }
    // The target's own sub-emitter call, once per new four-lane group over
    // its accepted lanes with the group's four birth times, before the
    // newborn deaths.
    let mut recorded = None;
    if let Some(target) = events.as_ref() {
        let mut copy = target.events.clone();
        let mut staged: Vec<super::sub_events::StagedLane> = lanes.iter().map(|lane| super::sub_events::StagedLane {
            particle: moly_law::particle::sub_emission::EventParticle {
                seed: lane.seed,
                age_percent: lane.age,
                inverse_lifetime: lane.inverse,
                position: lane.position,
                velocity: std::array::from_fn(|a| lane.velocity[a] + lane.animated[a]),
            },
            carry: lane.carry,
        }).collect();
        let event_owner = moly_law::particle::sub_emission::EventOwner {
            local_to_world: owner.local_to_world,
            world_space,
            accumulated_time: target.accumulated,
            emission_word: target.emission_word,
        };
        for offset in (0..accepted).step_by(4) {
            let dt4 = std::array::from_fn(|lane| lanes.get(offset + lane).map_or(f32::NAN, |lane| lane.birth_dt));
            copy.record_staged(&mut staged, offset, (offset + 4).min(accepted), dt4, event_owner);
        }
        for (lane, staged) in lanes.iter_mut().zip(&staged) {
            lane.carry = staged.carry;
        }
        recorded = Some(copy);
    }
    // Newborn deaths over the padded block: the last lane moves into the
    // dead one, and every death takes one off the accepted count.
    let mut deaths = 0_u64;
    let mut index = 0;
    while index < lanes.len() {
        if lanes[index].age > 100.0 {
            lanes.swap_remove(index);
            accepted = accepted.saturating_sub(1);
            deaths += 1;
        } else {
            index += 1;
        }
    }
    // Catch-up steps on the new block only.
    let mut remaining = command.catch_up;
    let mut steps = 0;
    if catch_up.runs {
        let dt = catch_up.frame_dt;
        loop {
            if lanes.is_empty() || accepted == 0 {
                break;
            }
            let mut end = accepted;
            remaining -= dt;
            steps += 1;
            let covered = (4 * end.div_ceil(4)).min(lanes.len());
            for lane in &mut lanes[..covered] {
                pre_modules(system, &laws, lane, dt, update.gravity, gravity_space);
            }
            for lane in &mut lanes[..covered] {
                let age = lane.age;
                let advanced = child_emit::min_propagating(age + (dt * 100.0) * lane.inverse, AGE_CAP);
                lane.age = if age > 100.0 { age } else { advanced };
                for a in 0..3 {
                    lane.position[a] = (lane.velocity[a] + lane.animated[a]) * dt + lane.position[a];
                }
            }
            // Four-wide from the block start while the group starts before
            // the end: dead lanes 3..0 take the last lane, the group is
            // retested; lanes of the last group past the end are tested too.
            let mut group = 0;
            while group < end && group < lanes.len() {
                let dead: [bool; 4] =
                    std::array::from_fn(|l| group + l < lanes.len() && lanes[group + l].age > 100.0);
                if !dead.iter().any(|&d| d) {
                    group += 4;
                    continue;
                }
                for l in (0..4).rev() {
                    if dead[l] {
                        let padding = group + l >= accepted;
                        lanes.swap_remove(group + l);
                        deaths += 1;
                        if accepted > 0 && !(arms::on("paddingKillKeepsAccepted") && padding) {
                            accepted -= 1;
                        }
                    }
                }
            }
            end = end.min(lanes.len());
            let covered = (4 * end.div_ceil(4)).min(lanes.len());
            for lane in &mut lanes[..covered] {
                integrate_rotation(&laws, lane, dt);
            }
            if !(remaining >= catch_up.step) {
                break;
            }
        }
    }
    lanes.truncate(accepted);
    let born: Vec<(Particle, Side)> = lanes
        .iter()
        .map(|lane| {
            let reflect = |v: [f32; 3]| crate::particle_geometry::reflect(Vec3::from_array(v)).to_array();
            (
                Particle {
                    position: reflect(lane.position),
                    velocity: reflect(lane.velocity),
                    start_lifetime: lane.lifetime,
                    inverse_lifetime: lane.inverse,
                    age_percent: lane.age,
                },
                Side {
                    rand: 0.0,
                    seed: lane.seed,
                    rot: lane.rotation,
                    size: [lane.size; 3],
                    gravity: 0.0,
                    colour: moly_law::particle::gradient::rgba8_to_float(lane.color),
                    total_velocity: reflect(std::array::from_fn(|a| lane.velocity[a] + lane.animated[a])),
                    custom_data: lane.custom,
                    emit_carry: lane.carry,
                    animated: reflect(lane.animated),
                    current_size: 0.0,
                },
            )
        })
        .collect();
    if born
        .iter()
        .any(|(p, s)| p.position.iter().chain(&p.velocity).chain(&s.total_velocity).any(|v| !v.is_finite()))
    {
        return Err(Refused::InvalidCommand("nonfinite child birth"));
    }
    // Commit.
    let accepted = born.len();
    for (particle, side) in born {
        system.pool.push(particle);
        system.side.push(side);
    }
    finish_births(&mut system.pool, &mut system.side, &mut system.ring_cursor, RingBufferMode::Disabled,
        maximum, old, |_, _| {});
    *initial = next_initial;
    if let (Some(target), Some(recorded)) = (events, recorded) {
        *target.events = recorded;
    }
    system.born_total += capacity_accepted as u64;
    system.died_total += deaths;
    system.full_total += (count - capacity_accepted) as u64;
    Ok(Applied {
        born: accepted,
        catch_up_steps: steps,
        catch_up_remainder: remaining,
        start: Some(frame),
    })
}

/// The pre-simulation modules of one lane over dt: gravity into the
/// persistent velocity, animated velocity cleared, angular speed cleared and
/// rebuilt by RotationOverLifetime, CustomData at the current age.
fn pre_modules(system: &Runtime, laws: &ChildLaws, lane: &mut Lane, dt: f32, gravity: [f32; 3],
    gravity_space: Option<&[f32; 16]>) {
    if !arms::on("noGravity") {
        if let Some(delta) = child_emit::gravity_delta(gravity, laws.gravity_modifier, dt, gravity_space) {
            lane.velocity = std::array::from_fn(|a| delta[a] + lane.velocity[a]);
        }
    }
    lane.animated = [0.0; 3];
    if let Some(rol) = &system.rol {
        lane.angular = [0.0; 3];
        let speed = rol.angular_velocity(lane.seed, 0.0, lane.age);
        lane.angular = std::array::from_fn(|a| lane.angular[a] + speed[a]);
    }
    if let Some(custom) = &system.custom_law {
        custom.update(lane.seed, lane.age, &mut lane.custom);
    }
}

/// rotation = angular speed * dt + rotation, on the three axes when the
/// arrays carry 3D rotation, else on z; nothing without angular speed.
fn integrate_rotation(laws: &ChildLaws, lane: &mut Lane, dt: f32) {
    if !laws.angular_speed {
        return;
    }
    let axes = if laws.rotation_3d { 0..3 } else { 2..3 };
    for axis in axes {
        lane.rotation[axis] = lane.angular[axis] * dt + lane.rotation[axis];
    }
}

/// The installed child side of a sub-emitter target: the owner words its
/// commands read and what the delivered commands did. The target never
/// emits on its own (the engine marks every sub-emitter target stopped each
/// frame), so its own frame is the stopped update.
#[derive(Clone, Debug)]
pub(crate) struct ChildTarget {
    pub(crate) owner: ChildOwner,
    pub(crate) commands: u64,
    pub(crate) births: u64,
    pub(crate) refused: u64,
    pub(crate) last_refusal: Option<String>,
}

/// Whether a system can be a sub-emitter target on this path: the native
/// birth composition (without the route, which a target does not take), the
/// native Shape emitter state, the child composition and the target's own
/// Shape law.
pub(crate) fn child_target_eligible(emitter: &EmitterParams, evidence: Option<ShapeEmitterEvidence>)
    -> Result<(), String> {
    birth::qualify_emitter(&own_clock_emitter(emitter)).map_err(|refused| format!("{refused:?}"))?;
    native_shape_state_eligible(emitter, evidence)?;
    qualify_target(emitter).map_err(|refused| format!("{refused:?}"))?;
    if let Some(params) = &emitter.shape {
        let law = ShapeBirthLaw::from_params(params).map_err(|refused| format!("target Shape {refused:?}"))?;
        // The BurstSpread edge and circle and the Loop, PingPong and
        // BurstSpread cone read the birth call's batch (accepted count, arc
        // clock, emission spacing and offset); a child command's batch is not
        // built here.
        let burst = |mode: Option<ShapeMode>| mode == Some(ShapeMode::BurstSpread);
        if law.arc_clock_speed().is_some() || burst(params.controls.arc_mode) || burst(params.controls.radius_mode) {
            return Err("target Shape kernel reads the birth batch, which a child command does not build".into());
        }
    }
    Ok(())
}

/// The target as its own frame reads it. That frame is the stopped update,
/// which emits nothing, so the target's rate over distance is read only by
/// its parent's edge law (from the parent particle's motion) and never by its
/// own clock; the clock takes the Emission block without it.
fn own_clock_emitter(emitter: &EmitterParams) -> EmitterParams {
    let mut own = emitter.clone();
    if let Some(emission) = own.emission.as_mut() {
        emission.rate_over_distance = MinMaxCurve::Constant(0.0);
    }
    own
}

/// Called once when an admitted sub-emitter target is installed: its seed
/// owner and streams as any system's first Play makes them, and the child
/// owner words its commands read.
pub(crate) fn install_child_target(system: &mut Runtime, seeds: &mut seed::SystemSeedManager,
    owner: ChildOwner) -> Result<(), String> {
    if system.native_birth.is_some() {
        return Err("target already has a birth owner".into());
    }
    child_target_eligible(&system.emitter, system.geometry.shape_evidence())?;
    system.emitter = own_clock_emitter(&system.emitter);
    let (seed_owner, streams) = seeds
        .create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)
        .map_err(|error| format!("{error:?}"))?;
    system.native_birth = Some(birth::NativeBirthState {
        owner: Some(seed_owner),
        initial: streams.initial,
        shape: streams.shape,
        shape_clock: moly_law::particle::shape::ArcLoopClock::default(),
        emission: moly_law::particle::autonomous_emission::AutonomousEmissionState::initialized(
            streams.scalar_birth),
        frame: birth::FrameState::default(),
        events: None,
        target: Some(ChildTarget { owner, commands: 0, births: 0, refused: 0, last_refusal: None }),
    });
    Ok(())
}

/// One parent command delivered to its installed target in the frame it was
/// issued: the per-frame update passes no catch-up flag, the world plays and
/// the world gravity is the physics default. The Shape stream is committed
/// only when the whole command succeeded; a refusal changes nothing but the
/// target's refusal count.
pub(crate) fn deliver_command(system: &mut Runtime,
    command: &moly_law::particle::sub_emission::SubEmitterCommand, frame_dt: f32) -> Result<usize, String> {
    let Some(mut native) = system.native_birth.take() else {
        return Err("target has no child owner".into());
    };
    let result = deliver_staged(system, &mut native, command, frame_dt);
    if let Some(target) = native.target.as_mut() {
        target.commands += 1;
        match &result {
            Ok(born) => target.births += *born as u64,
            Err(reason) => {
                target.refused += 1;
                target.last_refusal = Some(reason.clone());
            }
        }
    }
    system.native_birth = Some(native);
    result
}

fn deliver_staged(system: &mut Runtime, native: &mut birth::NativeBirthState,
    command: &moly_law::particle::sub_emission::SubEmitterCommand, frame_dt: f32) -> Result<usize, String> {
    let owner = native.target.as_ref().ok_or("system is not a sub-emitter target")?.owner;
    let update = ChildUpdate { flags: 0, frame_dt, world_playing: true, gravity: SOURCE_GRAVITY };
    let mut shape = match &system.emitter.shape {
        None => None,
        Some(params) => Some(SourceShape {
            law: ShapeBirthLaw::from_params(params).map_err(|refused| format!("target Shape {refused:?}"))?,
            stream: native.shape,
            shape_scale: owner.shape_scale,
            uses_axis_of_rotation: system.geometry.shape_evidence().is_some_and(|e| e.mesh_renderer),
        }),
    };
    let events = native.events.as_mut().map(|events| TargetEvents {
        events,
        accumulated: system.pending,
        emission_word: native.emission.random.words[0],
    });
    let applied = apply_command_with_events(system, &owner, &mut native.initial,
        shape.as_mut().map(|s| s as &mut dyn ChildShape), &ChildCommand::from_event(command), update, events)
        .map_err(|refused| format!("{refused:?}"))?;
    if let Some(shape) = shape {
        native.shape = shape.stream;
    }
    Ok(applied.born)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_bytes(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex fixture has odd length");
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => panic!("invalid fixture hex digit"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    fn raw(count: u64) -> (Vec<u8>, Vec<u8>) {
        let mut bytes = vec![0_u8; 0x78];
        let put32 = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        bytes[0x58..0x60].copy_from_slice(&count.to_le_bytes());
        bytes[0x60..0x68].copy_from_slice(&0_u64.to_le_bytes());
        for (at, value) in [
            (0x20, u32::MAX),
            (0x24, 0x3f800000),
            (0x28, 0x3f800000),
            (0x2c, 0x3f800000),
            (0x44, 0x3f800000),
            (0x48, 0x3f800000),
            (0x4c, 0x7f800000),
        ] {
            put32(&mut bytes, at, value);
        }
        for (at, value) in [
            (0x08, 1.0_f32),
            (0x0c, 2.0),
            (0x10, 3.0),
            (0x68, 0.25),
            (0x6c, 0.25),
            (0x70, 0.5),
        ] {
            put32(&mut bytes, at, value.to_bits());
        }
        (
            bytes,
            0_f32
                .to_le_bytes()
                .into_iter()
                .chain(0_f32.to_le_bytes())
                .chain(1_f32.to_le_bytes())
                .collect(),
        )
    }

    #[test]
    fn native_command_layout_parses() {
        let (bytes, emission) = raw(0);
        let command = ChildCommand::from_native_bytes(&bytes, &emission).unwrap();
        assert_eq!(command.count, 0);
        assert_eq!(command.distribution.burst_fraction.to_bits(), 1.0_f32.to_bits());
        assert_eq!(command.inherited_words[9], 1.0_f32.to_bits());
        assert_eq!(command.validate(), Ok(()));
    }

    /// No finite or non-finite command input panics: every word of the
    /// command and every update input runs through a qualified target.
    #[test]
    fn no_command_input_panics() {
        let mut system = test_support::runtime();
        system.emitter.shape_enabled = Some(false);
        system.emitter.shape = None;
        system.emitter.start.speed = MinMaxCurve::Constant(0.0);
        let owner = ChildOwner {
            local_to_world: moly_law::particle::shape_birth::IDENTITY,
            world_to_local: moly_law::particle::shape_birth::IDENTITY,
            local_rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            emitter_scale: [1.0; 3],
            shape_scale: [1.0; 3],
        };
        let values = [0.0, -0.0, 1.0, -1.0, 0.5, 1e-30, 1e30, f32::MAX, f32::NAN, f32::INFINITY,
            f32::NEG_INFINITY];
        let (bytes, emission) = raw(3);
        let base = ChildCommand::from_native_bytes(&bytes, &emission).unwrap();
        for &a in &values {
            for &b in &values {
                let mut command = base.clone();
                command.position = [a, b, a];
                command.velocity = [b, a, b];
                command.catch_up = a;
                command.dt = b;
                command.distribution.spacing = a;
                command.distribution.offset = b;
                command.distribution.burst_fraction = a;
                command.count = if a > 0.0 { 40 } else { 3 };
                let mut random = ModuleRandom::from_owner_seed(7);
                let update = ChildUpdate { flags: 5, frame_dt: b, world_playing: a > 0.0, gravity: [a, b, a] };
                let _ = apply_command(&mut system, &owner, &mut random, None, &command, update);
                assert_eq!(system.pool.len(), system.side.len());
            }
        }
    }

    #[test]
    #[ignore = "set MOLY_CHILD_COMMAND_CURRENT or run from the weather-complete lane"]
    fn replays_current_flash_child_commands_into_shared_pool() {
        use serde_json::{json, Value};
        use std::path::PathBuf;

        fn number(value: &Value) -> f32 {
            let n = value.as_f64().expect("native receipt number") as f32;
            assert!(n.is_finite(), "finite native output required");
            n
        }
        fn words(value: &Value) -> ModuleRandom {
            let flat = value.as_array().expect("Initial ModuleRandom words");
            assert_eq!(flat.len(), 16);
            ModuleRandom {
                words: std::array::from_fn(|word| {
                    std::array::from_fn(|lane| {
                        u32::try_from(flat[word * 4 + lane].as_u64().unwrap()).unwrap()
                    })
                }),
            }
        }
        fn exact(actual: f32, expected: f32, label: &str) {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{label}: {actual:?} vs native {expected:?}"
            );
        }
        fn physical_vector(actual: [f32; 3], native: &Value, label: &str) {
            assert_eq!(native.as_array().unwrap().len(), 3);
            for axis in 0..3 {
                // Source->runtime handedness changes X only. A zero direction may
                // be represented as +0 after integration or -0 after reflection;
                // this equivalence applies ONLY to position/velocity zero lanes.
                let expected = number(&native[axis]) * if axis == 0 { -1.0 } else { 1.0 };
                if actual[axis] == 0.0 && expected == 0.0 {
                    continue;
                }
                exact(actual[axis], expected, &format!("{label}[{axis}]"));
            }
        }
        fn check_pool(system: &Runtime, row: &Value, label: &str) {
            let output = &row["output"];
            let draw = &row["draw"];
            let count = output["count"].as_u64().unwrap() as usize;
            assert_eq!(system.pool.len(), count, "{label} pool count");
            assert_eq!(system.side.len(), count, "{label} side count");
            for field in ["position", "velocity", "age", "inverseLifetime", "seeds"] {
                assert_eq!(
                    output[field].as_array().unwrap().len(),
                    count,
                    "{label} native {field} length"
                );
            }
            for field in ["rotation", "size", "colors", "custom"] {
                assert_eq!(
                    draw[field].as_array().unwrap().len(),
                    count,
                    "{label} native draw.{field} length"
                );
            }
            // Exercise the existing presentation consumer too. Native draw.colors
            // is ColorModule's byte result; CustomData must be retained pre-output,
            // never recomputed at the final presentation age inside this test.
            let quads = build_quads(system, &GlobalTransform::IDENTITY);
            assert_eq!(quads.len(), count);
            for index in 0..count {
                let p = &system.pool[index];
                let side = &system.side[index];
                let lane = format!("{label} particle {index}");
                physical_vector(
                    p.position,
                    &output["position"][index],
                    &format!("{lane} position"),
                );
                physical_vector(
                    p.velocity,
                    &output["velocity"][index],
                    &format!("{lane} velocity"),
                );
                exact(
                    p.age_percent,
                    number(&output["age"][index]),
                    &format!("{lane} age"),
                );
                exact(
                    p.inverse_lifetime,
                    number(&output["inverseLifetime"][index]),
                    &format!("{lane} inverseLifetime"),
                );
                assert_eq!(
                    side.seed as u64,
                    output["seeds"][index].as_u64().unwrap(),
                    "{lane} seed"
                );
                assert_eq!(draw["rotation"][index].as_array().unwrap().len(), 3);
                for axis in 0..3 {
                    exact(
                        side.rot[axis],
                        number(&draw["rotation"][index][axis]),
                        &format!("{lane} rotation[{axis}]"),
                    );
                    // This source has size3D=false. Native draw stores one size
                    // channel; Runtime expands that one authored value to XYZ.
                    exact(
                        side.size[axis],
                        number(&draw["size"][index]),
                        &format!("{lane} expanded size[{axis}]"),
                    );
                }
                let actual_color =
                    moly_law::particle::gradient::quantize_rgba8(quads[index].colour);
                assert_eq!(draw["colors"][index].as_array().unwrap().len(), 4);
                for channel in 0..4 {
                    assert_eq!(
                        actual_color[channel] as u64,
                        draw["colors"][index][channel].as_u64().unwrap(),
                        "{lane} rendered color[{channel}]"
                    );
                }
                assert_eq!(draw["custom"][index].as_array().unwrap().len(), 8);
                let [custom1, custom2] = [quads[index].custom1, quads[index].custom2];
                for stream in 0..2 {
                    for channel in 0..4 {
                        let expected = number(&draw["custom"][index][stream * 4 + channel]);
                        exact(
                            side.custom_data[stream][channel],
                            expected,
                            &format!("{lane} persistent custom[{stream}][{channel}]"),
                        );
                        let rendered = if stream == 0 {
                            custom1[channel]
                        } else {
                            custom2[channel]
                        };
                        exact(
                            rendered,
                            expected,
                            &format!("{lane} presented custom[{stream}][{channel}]"),
                        );
                    }
                }
            }
        }
        fn source_runtime(particle: &Value, effect: &str) -> Runtime {
            // Retain the whole original particle. Select it out of the source
            // document only to avoid unrelated emitters' unsupported schema arms.
            let selected = json!({"effects": {effect: {"particles": [particle]}}});
            let mut decoded = moly_law::particle::schema::Effects::from_json_str(
                &serde_json::to_vec(&selected).unwrap(),
            )
            .expect("decode original flash source");
            assert_eq!(decoded.emitters.len(), 1);
            let emitter = decoded.emitters.remove(0);
            assert_eq!(emitter.simulation_space, SimulationSpace::Local);
            assert_eq!(emitter.shape_enabled, Some(false));
            assert!(!emitter.prewarm && !emitter.start.size3d && emitter.start.rotation3d);
            assert_eq!(emitter.max_particles, 30);
            assert_eq!(emitter.simulation_speed.to_bits(), 1.0_f32.to_bits());
            assert!(emitter.color_over_lifetime.is_some());
            assert!(emitter.custom_data.is_some());
            // The probe's catchupGravity is nonzero but the actual flash authored
            // modifier is zero. Do not overwrite the source to fit a synthetic row.
            assert!(matches!(
                emitter.start.gravity_modifier,
                MinMaxCurve::Constant(0.0)
            ));
            let mut system = test_support::runtime();
            system.node = emitter.node.clone();
            system.effect = emitter.effect.clone();
            system.kind = EffectKind::Site;
            system.gravity_law =
                moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).expect("curves validated during admission");
            system.color_law = emitter
                .color_over_lifetime
                .as_ref()
                .map(moly_law::particle::color::ColorOverLifetime::from_params);
            system.custom_law = emitter
                .custom_data
                .as_ref()
                .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission"));
            system.emitter = emitter;
            system.pool.clear();
            system.side.clear();
            system.born_total = 0;
            system.died_total = 0;
            system.full_total = 0;
            system.refused_total = 0;
            system
        }
        fn replay(
            system: &mut Runtime,
            random: &mut ModuleRandom,
            row: &Value,
            update: ChildUpdate,
            label: &str,
        ) -> Applied {
            let value = &row["command"];
            let command = ChildCommand::from_native_bytes(
                &hex_bytes(value["rawHex"].as_str().unwrap()),
                &hex_bytes(value["emissionHex"].as_str().unwrap()),
            )
            .unwrap();
            assert_eq!(command.count, value["count"].as_u64().unwrap());
            for i in 0..13 {
                assert_eq!(
                    command.inherited_words[i] as u64,
                    value["inheritedWords"][i].as_u64().unwrap()
                );
            }
            assert_eq!(
                *random,
                words(&row["beforeWords"]),
                "{label} Initial RNG before"
            );
            let old_rng = system.rng.0;
            let old_clock = (
                system.playback_head.to_bits(),
                system.previous_head.to_bits(),
            );
            let old_born = system.born_total;
            // The probe owner is the identity (checked below).
            let owner = ChildOwner {
                local_to_world: moly_law::particle::shape_birth::IDENTITY,
                world_to_local: moly_law::particle::shape_birth::IDENTITY,
                local_rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                emitter_scale: [1.0; 3],
                shape_scale: [1.0; 3],
            };
            let applied = apply_command(system, &owner, random, None, &command, update)
                .unwrap_or_else(|e| panic!("{label} apply: {e:?}"));
            assert_eq!(applied.born as u64, command.count, "{label} born count");
            assert_eq!(
                system.born_total - old_born,
                command.count,
                "{label} cumulative births"
            );
            assert_eq!(
                *random,
                words(&row["afterWords"]),
                "{label} Initial RNG after"
            );
            assert_eq!(system.rng.0, old_rng, "{label} legacy RNG untouched");
            assert_eq!(
                (
                    system.playback_head.to_bits(),
                    system.previous_head.to_bits()
                ),
                old_clock,
                "{label} child entry leaves autonomous clock unchanged"
            );
            assert_eq!(
                (system.died_total, system.full_total, system.refused_total),
                (0, 0, 0)
            );
            check_pool(system, row, label);
            if command.count == 0 {
                assert_eq!(
                    words(&row["beforeWords"]),
                    *random,
                    "{label} zero-count RNG no-op"
                );
                assert!(row["initialCalls"].as_array().unwrap().is_empty());
            }
            applied
        }

        let fixture_path = std::env::var_os("MOLY_CHILD_COMMAND_CURRENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../..")
                    .join("child-command-current.json")
            });
        let fixture: Value = serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap();
        assert_eq!(
            fixture["sourceSha256"],
            "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
        );
        let rows = fixture["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(fixture["summary"]["failureCount"].as_u64(), Some(0));
        // Keep the recorded source path intact; cloud replays opt into relocation.
        let effects_path = std::env::var_os("MOLY_CHILD_COMMAND_EFFECTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fixture["source"]["effects"].as_str().unwrap()));
        let effects: Value =
            serde_json::from_slice(&std::fs::read(&effects_path).unwrap_or_else(|error| {
                panic!("read child effects {}: {error}", effects_path.display())
            }))
            .unwrap();
        let effect_name = fixture["source"]["effect"].as_str().unwrap();
        let node_name = fixture["source"]["node"].as_str().unwrap();
        let particle = effects["effects"][effect_name]["particles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["node"].as_str() == Some(node_name))
            .expect("selected original flash particle");
        assert_eq!(particle["systemPathId"], fixture["source"]["systemPathId"]);
        let mut modules: Vec<_> = particle["system"]["sourceModules"]["enabled"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        modules.sort_unstable();
        assert_eq!(
            modules,
            [
                "ColorModule",
                "CustomDataModule",
                "EmissionModule",
                "InitialModule"
            ]
        );
        let identity = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        assert_eq!(fixture["probeInputs"]["owner"], identity);
        assert_eq!(fixture["probeInputs"]["inverseChildOwner"], identity);
        assert_eq!(number(&fixture["probeInputs"]["simulationSpeed"]), 1.0);
        let update = ChildUpdate {
            flags: fixture["probeInputs"]["chainUpdateFlags"].as_u64().unwrap() as u32,
            frame_dt: number(&fixture["probeInputs"]["frameDt"]),
            world_playing: true,
            // The flash's authored modifier is zero, so gravity changes at
            // most the sign of a zero velocity, which compares by value here.
            gravity: SOURCE_GRAVITY,
        };
        let mut system = source_runtime(particle, effect_name);
        let mut random = words(&fixture["initialWords"]);
        let mut nonzero = 0;
        for (i, row) in rows.iter().enumerate() {
            let applied = replay(&mut system, &mut random, row, update, &format!("row {i}"));
            nonzero += usize::from(applied.born > 0);
            assert_eq!(applied.catch_up_steps, 0);
        }
        assert_eq!(nonzero, 4);
        assert_eq!(system.pool.len(), 4);
        assert_eq!(random, words(&fixture["finalWords"]));

        // Independently captured synthetic command, not a ninth RecordEmit arrival.
        // Fresh source target/RNG; this is the nonzero CustomData phase regression.
        let catchup = &fixture["explicitCatchup"];
        assert!(catchup["commandOrigin"]
            .as_str()
            .unwrap()
            .contains("not captured RecordEmit"));
        assert!(catchup["draw"]["custom"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|v| v.as_array().unwrap())
            .any(|v| number(v) != 0.0));
        let mut catchup_system = source_runtime(particle, effect_name);
        let mut catchup_random = words(&catchup["beforeWords"]);
        let applied = replay(
            &mut catchup_system,
            &mut catchup_random,
            catchup,
            ChildUpdate {
                flags: fixture["probeInputs"]["explicitCatchupUpdateFlags"]
                    .as_u64()
                    .unwrap() as u32,
                ..update
            },
            "explicitCatchup",
        );
        assert_eq!(applied.born, 4);
        assert_eq!(applied.catch_up_steps, 1);
        exact(applied.catch_up_remainder, 0.0, "explicitCatchup remainder");
    }
}
