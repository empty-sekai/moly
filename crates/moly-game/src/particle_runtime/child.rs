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
//! lifetime, size (one axis or three), speed and rotation, a constant start colour source the
//! Initial law takes, a constant gravity modifier, Shape through the target's
//! own Shape law (or no Shape with zero speed), RotationOverLifetime with
//! constant, two-constant or single-curve axes, VelocityOverLifetime (constant or
//! two-constant linear axes of one mode, not in world space, the constant
//! speed modifier one; orbital, offset and radial constants or two constants,
//! their orbital section only in a Local target), Noise (the qualified Noise
//! law, with the target's installed owner seed and scroll), ClampVelocity (one
//! axis group, a constant limit, zero drag), InheritVelocity (Initial with a
//! constant or two-constant curve; Current outside World space, which does
//! nothing here), SizeOverLifetime,
//! ColorOverLifetime and the texture sheet (render-time), CustomData, no ring buffer, simulation
//! speed one. Any other module on the target, and any other configuration of
//! those, refuses.
//!
//! InheritVelocity: the child Emit hands its StartModules the command velocity
//! in the target's space (the start frame's `emitter_velocity`) with the
//! inheritance on, in every simulation space; StartVelocity multiplies each
//! newborn's shape direction by the start speed and then, in Initial mode
//! with a constant or two-constant curve, adds that velocity times the curve
//! (its random word salted apart from the speed's) to the persistent
//! velocity. In Current mode the start adds nothing and the per-update module
//! acts only in World space. The newborn and catch-up pre-simulation modules run in the
//! engine's order: gravity and the animated velocity cleared,
//! RotationOverLifetime, Velocity (linear, then orbital), Noise,
//! ClampVelocity, CustomData. The newborn call does not advance the Noise
//! scroll; each catch-up step's call does, once, before the modules read it.
//! The inherited block must be the neutral one
//! or carry only the size (an edge that inherits the size): each stored start
//! size axis is then the inherited axis times the target's own start size of
//! that axis (the target's arrays store three axes when its start size is 3D
//! or its SizeModule has separate axes, else x alone, which the runtime
//! expands); any other inherited word refuses by name. The parent particle's seed the
//! block ends with is not a child random word.
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
    inherit::{self, ChildInherit},
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
        inherited_size(&self.inherited_words)?;
        self.distribution
            .timing(0, self.rate_count as u32, self.dt, self.previous, self.current)
            .map_err(|_| Refused::InvalidCommand("birth distribution"))?;
        Ok(())
    }
}

/// The inherited size of a command's block, or the inherited word it carries
/// that the child side does not transcribe, by name.
fn inherited_size(words: &[u32; 13]) -> Result<ChildInherit, Refused> {
    ChildInherit::from_words(words).map_err(|refused| Refused::Unsupported(match refused {
        inherit::Refused::Block("color") => "inherited colour block",
        inherit::Refused::Block("rotation") => "inherited rotation block",
        inherit::Refused::Block("lifetime") => "inherited lifetime block",
        inherit::Refused::Block("duration") => "inherited duration block",
        _ => "inherited block outside the size inheritance",
    }))
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
    /// The particle arrays carry three size axes.
    size_3d: bool,
    /// VelocityOverLifetime (linear axes, and the orbital blocks).
    velocity: Option<moly_law::particle::velocity::VelocityOverLifetime>,
    /// The orbital section contributes (some orbital, offset or radial block
    /// is not zero); the target is Local.
    orbital: bool,
    /// ClampVelocity (one axis group, a constant limit, zero drag).
    limit: Option<moly_law::particle::LimitVelocity>,
    /// InheritVelocity in Initial mode: the curve the start velocity adds
    /// the command velocity (in the target's space) times.
    inherit: Option<CurveSampler>,
}

/// The salt of the InheritVelocity curve's random word in StartVelocity (the
/// start speed's is 0x96aa4de3).
const INHERIT_VELOCITY_SALT: u32 = 0x0033_e627;

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
    // The texture sheet (UV module) takes no part in the birth: the child
    // Emit's start pass and its newborn and catch-up module passes read none
    // of it, and it adds no per-particle storage; the renderer derives each
    // particle's sheet cell from the particle's own seed (and age, for a
    // curve), which the child Emit writes as any birth does. A target's
    // sheet is drawn as any system's.
    if emitter.force.is_some()
        || emitter.collision.is_some()
        || emitter.trails.is_some()
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
    // The target's Initial curves read the time the child Emit passes, which
    // only a constant ignores; the curve modes were executed for an
    // emitter's own births only.
    let start = &emitter.start;
    if ![Some(&start.lifetime), Some(&start.size), start.size_y.as_ref(), start.size_z.as_ref(), Some(&start.rotation),
        start.rotation_x.as_ref(), start.rotation_y.as_ref()].into_iter().flatten().all(scalar) {
        return unsupported("target start lifetime, size or rotation curve mode");
    }
    let inherit = match &emitter.inherit_velocity {
        None => None,
        Some(params) => match params.mode {
            moly_law::particle::schema::InheritVelocityMode::Initial if scalar(&params.curve) => Some(
                CurveSampler::new(&params.curve, moly_law::particle::curve::CurveTime::Normalized)
                    .map_err(Refused::Unsupported)?),
            // A curve stores the velocity per particle for the per-update
            // module, which is not ported.
            moly_law::particle::schema::InheritVelocityMode::Initial => {
                return unsupported("target InheritVelocity curve mode");
            }
            moly_law::particle::schema::InheritVelocityMode::Current
                if emitter.simulation_space != SimulationSpace::World => None,
            moly_law::particle::schema::InheritVelocityMode::Current => {
                return unsupported("target InheritVelocity Current mode in World space");
            }
        },
    };
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
    // RotationOverLifetime runs in the newborn pass as in any update: each
    // axis is sampled at the lane's age before the step (a keyed curve
    // through the same evaluation as the root path). A two-curve axis was
    // not replayed in a child Emit.
    if let Some(rol) = &emitter.rotation_over_lifetime {
        let axes = [Some(&rol.curve), rol.x.as_ref(), rol.y.as_ref()];
        if axes.iter().flatten().any(|curve| matches!(curve, MinMaxCurve::TwoCurves { .. })) {
            return unsupported("target RotationOverLifetime two-curve mode");
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
    let velocity = match &emitter.velocity_over_lifetime {
        None => None,
        Some(params) => Some(child_velocity(params)?),
    };
    let limit = match &emitter.limit_velocity {
        None => None,
        Some(params) => Some(child_limit(params)?),
    };
    // Noise runs the root path's law; its owner seed and scroll are the
    // target's own, installed with its seed owner.
    if let Some(params) = &emitter.noise {
        moly_law::particle::noise::NoiseLaw::from_params(params).map_err(Refused::Unsupported)?;
    }
    let orbital = match &emitter.velocity_over_lifetime {
        Some(params) => !zero_orbital(params),
        None => false,
    };
    if orbital && emitter.simulation_space != SimulationSpace::Local {
        // The orbital section converts the particle position through the
        // owner's matrices outside Local space; that conversion was not read.
        return unsupported("target orbital Velocity outside Local space");
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
        size_3d: moly_law::particle::death_event::size_3d(emitter),
        velocity,
        orbital,
        limit,
        inherit,
    })
}

/// Whether the orbital, offset and radial blocks are all zero: the engine's
/// orbital section then adds a signed zero (its angle rotates by exactly
/// nothing and its radial step is zero), which is not composed.
fn zero_orbital(params: &moly_law::particle::schema::VelocityOverLifetimeParams) -> bool {
    let zero = |c: &MinMaxCurve| matches!(c, MinMaxCurve::Constant(v) if *v == 0.0)
        || matches!(c, MinMaxCurve::TwoConstants { min, max } if *min == 0.0 && *max == 0.0);
    params.orbital.iter().all(zero) && params.orbital_offset.iter().all(zero) && zero(&params.radial)
}

/// A target's VelocityOverLifetime on the child path. VelocityModule::Update
/// adds `M * linear` to the animated velocity, `M` the identity for a Local
/// or Custom target and the owner's first three columns for a World one (the
/// linear axes not in world space); a two-constant axis draws from the
/// particle seed. Its orbital section then adds the orbital velocity: the
/// lane's position minus the offset, turned by angular speed times dt (Z, X,
/// Y), moved along itself by radial speed times dt, less the unturned vector,
/// over dt (the orbital law of the root path). An all-zero orbital block adds
/// a signed zero, which is not composed. The speed modifier storage exists
/// only for a modifier other than the constant one.
fn child_velocity(params: &moly_law::particle::schema::VelocityOverLifetimeParams)
    -> Result<moly_law::particle::velocity::VelocityOverLifetime, Refused> {
    let unsupported = |reason| Err(Refused::Unsupported(reason));
    if params.in_world_space {
        return unsupported("target Velocity in world space");
    }
    if !matches!(params.speed_modifier, MinMaxCurve::Constant(v) if v == 1.0) {
        return unsupported("target Velocity speed modifier other than the constant one");
    }
    let linear = [&params.x, &params.y, &params.z];
    let constants = linear.iter().all(|c| matches!(c, MinMaxCurve::Constant(_)));
    let two_constants = linear.iter().all(|c| matches!(c, MinMaxCurve::TwoConstants { .. }));
    if !(constants || two_constants) {
        return unsupported("target Velocity linear axes other than constants or two constants of one mode");
    }
    // The orbital section's curve-mode templates were not read; constants
    // and two constants take the per-particle seed streams of the law.
    let scalar = |c: &MinMaxCurve| matches!(c, MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. });
    if !(params.orbital.iter().all(scalar) && params.orbital_offset.iter().all(scalar) && scalar(&params.radial)) {
        return unsupported("target Velocity orbital, offset or radial curve mode");
    }
    moly_law::particle::velocity::VelocityOverLifetime::from_params(params).map_err(Refused::Unsupported)
}

/// A target's ClampVelocity on the child path: one axis group, a constant
/// limit and zero drag (the drag section is skipped).
fn child_limit(params: &moly_law::particle::schema::LimitVelocityParams)
    -> Result<moly_law::particle::LimitVelocity, Refused> {
    if params.separate_axis {
        return Err(Refused::Unsupported("target ClampVelocity with separate axes"));
    }
    if !matches!(params.magnitude, MinMaxCurve::Constant(_)) {
        return Err(Refused::Unsupported("target ClampVelocity limit other than a constant"));
    }
    if !params.drag.as_ref().is_none_or(|drag| matches!(drag, MinMaxCurve::Constant(v) if *v == 0.0)) {
        return Err(Refused::Unsupported("target ClampVelocity drag other than zero"));
    }
    moly_law::particle::LimitVelocity::from_parts(false, &params.magnitude, params.dampen, params.drag.as_ref(),
        params.multiply_drag_by_size, params.multiply_drag_by_velocity)
        .map_err(|_| Refused::Unsupported("target ClampVelocity outside the limit law"))
}

/// One newborn lane in source axes while the command runs.
#[derive(Clone, Copy)]
struct Lane {
    position: [f32; 3],
    velocity: [f32; 3],
    animated: [f32; 3],
    rotation: [f32; 3],
    angular: [f32; 3],
    size: [f32; 3],
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
    let inherit = inherited_size(&command.inherited_words)?;
    let laws = child_laws(&system.emitter)?;
    // Replay arm: a sheet word drawn at birth from the Initial stream.
    if system.emitter.texture_sheet.is_some() && arms::on("uvDrawsInitialWord") {
        let _ = initial.next4_u32();
    }
    if system.emitter.noise.is_some() != system.noise.is_some() {
        return Err(Refused::Unsupported("target Noise without its installed owner seed and scroll"));
    }
    // The target's Noise scroll, staged: the catch-up steps advance it.
    let mut scroll = system.noise.as_ref().map_or(0.0, |noise| noise.state.scroll);
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
    // A child's axis of rotation on this birth path (the Initial module's +Z
    // or a record inherited from the parent) was not read, and the mesh
    // renderer turns a child without 3D rotation about it.
    if matches!(system.geometry, super::Geometry::Mesh(_)) && super::uses_rotation_3d(&system.emitter, true) != Some(true) {
        return Err(Refused::Unsupported("child Mesh particle axis of rotation"));
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
                    storage_size_3d: laws.size_3d,
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
            let mut velocity = sample.direction.map(|d| speed * d);
            if let Some(inherit) = laws.inherit.as_ref().filter(|_| !arms::on("inheritVelocityIgnored")) {
                let salt = if arms::on("inheritVelocitySpeedSalt") { 0x96aa_4de3 } else { INHERIT_VELOCITY_SALT };
                let k = inherit.evaluate(timing[index].curve_time, ParticleRandom::sample(initial_lane.seed, salt));
                let inherited = if arms::on("inheritVelocityWorldVector") { command.velocity } else { frame.emitter_velocity };
                velocity = std::array::from_fn(|a| inherited[a] * k + velocity[a]);
            }
            lanes.push(Lane {
                position: sample.position,
                velocity,
                animated: [0.0; 3],
                rotation: initial_lane.rotation.map(|v| v.unwrap_or(0.0)),
                angular: [0.0; 3],
                size: start_size(&inherit, initial_lane.size, laws.size_3d),
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
    let velocity_space = world_space.then_some(&owner.local_to_world);
    let batch_seeds: Vec<u32> = (0..lanes.len()).map(|index| lanes[index & !3].seed).collect();
    for (index, lane) in lanes.iter_mut().enumerate() {
        let clamp_dt = if arms::on("clampCommandDt") { command.dt } else { lane.birth_dt };
        // StartModules' call passes no scroll update.
        let newborn_scroll = if arms::on("noiseScrollOnNewborn") {
            advanced_scroll(system, scroll, lane.birth_dt)
        } else {
            scroll
        };
        let space = ModuleSpace { velocity: velocity_space, batch_seed: batch_seeds[index], clamp_dt,
            noise_scroll: newborn_scroll };
        pre_modules(system, &laws, lane, lane.birth_dt, update.gravity, gravity_space, space);
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
            let batch_seeds: Vec<u32> = (0..covered).map(|index| lanes[index & !3].seed).collect();
            // Emit's catch-up call passes the scroll update: Noise advances
            // the system scroll once for the (nonempty) range, then reads it.
            if !arms::on("noiseCatchUpScrollFrozen") {
                scroll = advanced_scroll(system, scroll, dt);
            }
            for (index, lane) in lanes[..covered].iter_mut().enumerate() {
                let space = ModuleSpace { velocity: velocity_space, batch_seed: batch_seeds[index], clamp_dt: dt,
                    noise_scroll: scroll };
                pre_modules(system, &laws, lane, dt, update.gravity, gravity_space, space);
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
                    size: lane.size,
                    gravity: 0.0,
                    colour: moly_law::particle::gradient::rgba8_to_float(lane.color),
                    total_velocity: reflect(std::array::from_fn(|a| lane.velocity[a] + lane.animated[a])),
                    custom_data: lane.custom,
                    emit_carry: lane.carry,
                    animated: reflect(lane.animated),
                    current_size: 0.0,
                    // The Initial module's +Z (a Mesh child without 3D rotation,
                    // the only draw that reads it, is refused above).
                    axis: [0.0, 0.0, 1.0],
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
    if let Some(noise) = system.noise.as_mut() {
        noise.state.scroll = scroll;
    }
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

/// The stored start size: InitialModule's inherited axis times its own start
/// size of that axis, for each stored axis; with x alone stored, the runtime's
/// y and z repeat x.
fn start_size(inherit: &ChildInherit, own: [Option<f32>; 3], size_3d: bool) -> [f32; 3] {
    let x = own[0].unwrap_or(0.0);
    let own = if size_3d { own.map(|axis| axis.unwrap_or(x)) } else { [x; 3] };
    let size = if arms::on("ignoreInheritedSize") {
        own
    } else if arms::on("addInheritedSize") {
        let inherited = inherit.start_size([1.0; 3]);
        std::array::from_fn(|axis| inherited[axis] + own[axis])
    } else {
        inherit.start_size(own)
    };
    if size_3d { size } else { [size[0]; 3] }
}

/// What the Velocity, Noise and ClampVelocity updates of one lane read
/// besides the lane: the matrix of the linear velocity (None for the
/// identity), the seed of the lane's four-lane group, the lane's dt and the
/// target's Noise scroll at this call.
#[derive(Clone, Copy)]
struct ModuleSpace<'a> {
    velocity: Option<&'a [f32; 16]>,
    batch_seed: u32,
    clamp_dt: f32,
    noise_scroll: f32,
}

/// The target's Noise scroll after one scroll update over `dt` (the scroll
/// unchanged without Noise).
fn advanced_scroll(system: &Runtime, scroll: f32, dt: f32) -> f32 {
    match &system.noise {
        Some(noise) => {
            let mut state = moly_law::particle::noise::NoiseState { scroll };
            noise.law.advance_scroll(&mut state, dt, true);
            state.scroll
        }
        None => scroll,
    }
}

/// Noise on one lane: the root path's Noise law at the lane's position (the
/// target's space, source axes) and age before this call's age update, with
/// the target's owner seed and the call's scroll, added to the animated
/// velocity.
fn add_noise(system: &Runtime, lane: &mut Lane, scroll: f32) {
    if let Some(noise) = &system.noise {
        let state = moly_law::particle::noise::NoiseState { scroll };
        let value = noise.law.sample(state, lane.position, noise.owner_seed, lane.seed, lane.age);
        lane.animated = std::array::from_fn(|a| lane.animated[a] + value[a]);
    }
}

/// The pre-simulation modules of one lane over dt, in the engine's order:
/// gravity into the persistent velocity, animated velocity cleared, angular
/// speed cleared and rebuilt by RotationOverLifetime, VelocityOverLifetime's
/// linear velocity and then its orbital velocity added to the animated
/// velocity, Noise added to the animated velocity, ClampVelocity on the
/// persistent velocity against persistent plus animated (its k from the
/// lane's own dt), CustomData at the current age.
fn pre_modules(system: &mut Runtime, laws: &ChildLaws, lane: &mut Lane, dt: f32, gravity: [f32; 3],
    gravity_space: Option<&[f32; 16]>, space: ModuleSpace<'_>) {
    if !arms::on("noGravity") {
        if let Some(delta) = child_emit::gravity_delta(gravity, laws.gravity_modifier, dt, gravity_space) {
            lane.velocity = std::array::from_fn(|a| delta[a] + lane.velocity[a]);
        }
    }
    lane.animated = [0.0; 3];
    if let Some(rol) = &system.rol {
        lane.angular = [0.0; 3];
        // Replay arm: a keyed curve sampled at age zero.
        let age = if arms::on("rolCurveAtAgeZero") { 0.0 } else { lane.age };
        let speed = rol.angular_velocity(lane.seed, 0.0, age);
        lane.angular = std::array::from_fn(|a| lane.angular[a] + speed[a]);
    }
    if arms::on("clampBeforeVelocity") {
        clamp_velocity(laws, lane, space.clamp_dt);
    }
    if !arms::on("noVelocity") {
        if let Some(velocity) = &laws.velocity {
            let sample = velocity.sample(lane.seed, space.batch_seed, lane.age);
            let added = linear_in_space(sample.linear, space.velocity);
            lane.animated = std::array::from_fn(|a| lane.animated[a] + added[a]);
            if laws.orbital && !arms::on("noOrbital") {
                // Local target: the position is already in the owner's space.
                let modifier = sample.speed_modifier;
                let delta = sample.orbital.displacement(lane.position, dt, modifier);
                let orbital = moly_law::particle::velocity::animated_velocity(delta, dt, modifier);
                lane.animated = std::array::from_fn(|a| lane.animated[a] + orbital[a]);
            }
        }
    }
    if !arms::on("noNoise") {
        add_noise(system, lane, space.noise_scroll);
    }
    if !arms::on("clampBeforeVelocity") && !arms::on("noClamp") {
        clamp_velocity(laws, lane, space.clamp_dt);
    }
    if let Some(custom) = system.custom_law.as_mut() {
        custom.update(lane.seed, lane.age, &mut lane.custom);
    }
}

/// `M * v` as VelocityOverLifetime adds it, axis a:
/// `v.x * c0[a] + (v.y * c1[a] + v.z * c2[a])` with the columns of the
/// owner's local-to-world matrix, or of the identity.
fn linear_in_space(v: [f32; 3], matrix: Option<&[f32; 16]>) -> [f32; 3] {
    const IDENTITY: [f32; 16] = moly_law::particle::shape_birth::IDENTITY;
    let m = matrix.unwrap_or(&IDENTITY);
    std::array::from_fn(|a| {
        if arms::on("velocityGroupedAssociation") {
            (v[0] * m[a] + v[1] * m[4 + a]) + v[2] * m[8 + a]
        } else {
            v[0] * m[a] + (v[1] * m[4 + a] + v[2] * m[8 + a])
        }
    })
}

/// ClampVelocity on one lane: the limit law's clamp segment with the lane's
/// dt (the drag section is zero on the child path).
fn clamp_velocity(laws: &ChildLaws, lane: &mut Lane, dt: f32) {
    if let Some(limit) = &laws.limit {
        let size = moly_law::particle::DragSize { components: lane.size, size3d: laws.size_3d };
        // A finite dt: the command validation and the catch-up plan keep it finite.
        let _ = limit.step(&mut lane.velocity, lane.animated, lane.seed, lane.age, dt, size);
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
        // A Mesh target reads its module's mesh cache, which the child path
        // does not carry.
        if law.reads_mesh_cache() {
            return Err("target Mesh shape: the child path carries no emission surface".into());
        }
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
/// owner and streams as any system's first Play makes them, the start delay
/// word that Play writes, and the child owner words its commands read.
///
/// The target stays stopped every frame (the engine marks every cached
/// sub-emitter stopped), and a stopped update never counts the word down; it
/// ticks the clock only by the part of a slice beyond the word. A target
/// with a start delay therefore keeps its clock at zero while the frame's
/// slices stay at or below the word, for as long as it lives. A random delay
/// is Play's evaluation with the system seed's hash, which is not
/// transcribed, and is refused.
pub(crate) fn install_child_target(system: &mut Runtime, seeds: &mut seed::SystemSeedManager,
    owner: ChildOwner) -> Result<(), String> {
    if system.native_birth.is_some() {
        return Err("target already has a birth owner".into());
    }
    child_target_eligible(&system.emitter, system.geometry.shape_evidence())?;
    let start_delay = if arms::on("targetDelayWordZero") {
        0.0
    } else {
        super::play_start_delay(&system.emitter)
            .ok_or("random start delay on a sub-emitter target: Play's seed-hash evaluation is not transcribed")?
    };
    system.emitter = own_clock_emitter(&system.emitter);
    // Replay arm: the target's texture sheet lost at install.
    if arms::on("uvDroppedAtInstall") {
        system.texture_sheet = None;
    }
    // Qualified by child_target_eligible above; built before the owner draw.
    let noise_law = system.emitter.noise.as_ref()
        .map(|params| moly_law::particle::noise::NoiseLaw::from_params(params).map_err(str::to_owned))
        .transpose()?;
    let (seed_owner, streams) = seeds
        .create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)
        .map_err(|error| format!("{error:?}"))?;
    // Noise reads the target's owner seed and starts from the reset scroll,
    // as a root system's first Play installs it.
    system.noise = noise_law.map(|law| super::NoiseRuntime {
        law,
        state: moly_law::particle::noise::NoiseState { scroll: streams.noise_scroll },
        owner_seed: seed_owner.seed,
        owner: seed_owner,
    });
    system.native_birth = Some(birth::NativeBirthState {
        owner: Some(seed_owner),
        initial: streams.initial,
        shape: streams.shape,
        shape_clock: moly_law::particle::shape::ArcLoopClock::default(),
        emission: moly_law::particle::autonomous_emission::AutonomousEmissionState::initialized(
            streams.scalar_birth),
        frame: birth::FrameState { start_delay, ..birth::FrameState::default() },
        events: None,
        target: Some(ChildTarget { owner, commands: 0, births: 0, refused: 0, last_refusal: None }),
        procedural: false,
        replays: Vec::new(),
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
