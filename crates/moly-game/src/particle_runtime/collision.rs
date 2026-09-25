//! The CollisionModule on the native birth path.
//!
//! The engine calls the module at two points of every slice: after the
//! simulation and death pass over the whole pool with the slice dt (the first
//! post-simulation module, before the trail, size and sub-emitter calls), and
//! once per new four-lane birth group with the group's birth times (after the
//! group's pre-simulation modules, age, backtrack and integration, and before
//! its sub-emitter call and the newborn deaths). The module law is
//! `moly_law::particle::collision_query`; this file maps the pool into it in
//! source axes and commits what it writes.
//!
//! The module's scene is the site's physics scene (see `collision_scene`):
//! the MeshColliders of every installed weather effect (a stopped effect's
//! until it is destroyed), the site's own colliders and the placed fixtures'.
//! The module's mask is the system's own, read from its data. Admission binds
//! a system whose mask names only layers whose colliders are all ported and
//! refuses the others by name, the native birth installer installs the
//! module with the host's live scene, and a runtime without an installed
//! scene refuses the slice. When the overlap returns several colliders the
//! scene cannot give their order; the law then selects over every order, and
//! a call the order decides is refused by name.
//!
//! With a SizeModule the query radius reads the current-size stream, which the
//! engine writes at the same two points: after the post-simulation collision
//! call over the whole pool (so that call reads the size the previous write
//! left), and in each newborn group right before its collision call. The
//! post-simulation write sits before the trail update here; the trail reads
//! no current size (a size-affected trail is refused), so the order is not
//! observable.
//!
//! Collision sub-emitter edges record RecordEmit trigger 1 events inside the
//! module call. Each event's two commands go to the edge's child: queued, in
//! the order recorded, when the child is an installed target of the same
//! effect instance (the host hands them over after every system's frame, as
//! for the birth events), otherwise counted and dropped.
use super::*;
use moly_law::particle::collision_query::{
    Candidate, CollisionLaw, CollisionParticles, CollisionScene, CollisionState, OverlapQuery, OwnerPair,
    ParticleFlags, ParticleLane, Refused, SweepHit, SweepRequest, UpdateInput,
};
use moly_law::particle::collision_event::CollisionEmitEdge;
use moly_law::particle::collision_response::CollisionRandom;
use moly_law::particle::current_size::CurrentSizeLaw;
use moly_law::particle::sub_emission::SubEmitterCommand;

pub(crate) struct CollisionRuntime {
    pub(crate) law: CollisionLaw,
    /// The module's persistent words; their first values are not
    /// established, and no output of the admitted path reads them.
    pub(crate) state: CollisionState,
    /// The module's random words: the Collision stream of the owner's seed
    /// reset. With the constant response curves the law admits no output
    /// reads the drawn values; the words only advance.
    pub(crate) random: Option<CollisionRandom>,
    /// The owner words of a Local system, from its authored chain.
    pub(crate) owner: Option<OwnerPair>,
    pub(crate) scene: Box<dyn CollisionScene + Send + Sync>,
    pub(crate) calls: u64,
    pub(crate) hits: u64,
    pub(crate) draws: u64,
    /// Newborn calls whose slots past the group end were unknown and whose
    /// lanes reached no collider (exact without a sweep).
    pub(crate) unreached: u64,
    /// Lanes selected over every order of several colliders that hit them
    /// (the scene gave no order), every order agreeing.
    pub(crate) order_free: u64,
    /// Measurement only, never set by the product: a call refused as order
    /// dependent is counted in `order_dependent` and then run with the
    /// colliders in the scene's listed order, which is not the engine's.
    pub(crate) listed_order_fallback: bool,
    /// Lanes of calls refused as order dependent (counted with the fallback).
    pub(crate) order_dependent: u64,
    /// The SizeModule law that writes the current-size stream the query
    /// reads; `None` without a SizeModule.
    pub(crate) size: Option<CurrentSizeLaw>,
    /// The collision sub-emitter edges in slot order.
    pub(crate) edges: Vec<CollisionEdgeSlot>,
    /// Commands of delivered edges not yet handed to their target, in the
    /// order recorded.
    pub(crate) pending: Vec<(usize, SubEmitterCommand)>,
}

/// One collision edge in its slot: the edge, whether its child is an
/// installed target that receives the commands, and what it recorded.
#[derive(Clone, Debug)]
pub(crate) struct CollisionEdgeSlot {
    pub(crate) target: String,
    pub(crate) law: CollisionEmitEdge,
    pub(crate) delivered: bool,
    pub(crate) tally: super::sub_events::EdgeTally,
}

impl CollisionRuntime {
    /// The collision edges in slot order, as admission resolved them.
    pub(crate) fn attach_edges(&mut self, edges: Vec<super::sub_events::CollisionEdge>) {
        self.edges = edges.into_iter().map(|edge| CollisionEdgeSlot {
            target: edge.target, law: edge.law, delivered: false, tally: Default::default(),
        }).collect();
    }

    /// Marks every edge naming `target` delivered; false when none names it.
    pub(crate) fn deliver_to(&mut self, target: &str) -> bool {
        let mut found = false;
        for slot in self.edges.iter_mut().filter(|slot| slot.target == target) {
            slot.delivered = true;
            found = true;
        }
        found
    }

    /// The queued commands with their target, in the order recorded; the
    /// queue is left empty.
    pub(crate) fn take_commands(&mut self) -> Vec<(String, SubEmitterCommand)> {
        let edges = &self.edges;
        self.pending.drain(..).map(|(slot, command)| (edges[slot].target.clone(), command)).collect()
    }
}

/// The module law for an emitter, with the product's own limits: a
/// current-size stream only from a qualified SizeModule (no Noise size), no
/// per-particle speed modifier and World or Local space. The mask is the
/// system's own; the scene judges the layers it names.
/// Collision sub-emitter edges are taken; their children are resolved by
/// admission. The particle-state flags follow the module set: with a
/// size-over-lifetime module the current-size stream is read, else the
/// start-size stream (a size-by-speed module has no consumer on this runtime
/// at all), as its X or, with the 3D start size, the largest component;
/// without a speed modifier none is applied. The flags of the corpus
/// collision systems agree with this; the flags' writer was not read.
pub(super) fn qualify(emitter: &EmitterParams) -> Result<Option<CollisionLaw>, String> {
    let Some(params) = emitter.collision.as_ref() else {
        return Ok(None);
    };
    if emitter.noise.is_some() {
        return Err("collision current-size stream: the Noise size condition at the collision points is not transcribed".into());
    }
    current_size_law(emitter)?;
    if emitter.velocity_over_lifetime.as_ref().is_some_and(|velocity|
        !matches!(velocity.speed_modifier, moly_law::particle::MinMaxCurve::Constant(m) if m == 1.0)) {
        return Err("collision with a per-particle speed modifier".into());
    }
    let world = match emitter.simulation_space {
        SimulationSpace::World => true,
        SimulationSpace::Local => false,
        _ => return Err("collision in a custom simulation space".into()),
    };
    let flags = ParticleFlags {
        current_size: emitter.size_over_lifetime.is_some(),
        size_3d: emitter.start.size3d,
        speed_modifier: false,
    };
    CollisionLaw::from_params(params, world, flags).map(Some).map_err(|refused| format!("{refused:?}"))
}

/// The SizeModule law of the current-size stream, when the emitter has one.
fn current_size_law(emitter: &EmitterParams) -> Result<Option<CurrentSizeLaw>, String> {
    emitter.size_over_lifetime.as_ref()
        .map(|params| CurrentSizeLaw::from_params(params, emitter.start.size3d))
        .transpose()
        .map_err(|refused| format!("collision current-size stream: {refused:?}"))
}

/// The export's wrap modes of the size curves the current-size stream reads
/// (the law reads the keys only): only the clamp wraps are transcribed.
pub(crate) fn current_size_source_gate(system: &serde_json::Value) -> Result<(), String> {
    let Some(size) = system.get("sizeOverLifetime").filter(|v| v.is_object()) else {
        return Ok(());
    };
    let separate = size.get("separateAxes").and_then(serde_json::Value::as_bool) == Some(true);
    let keys: &[&str] = if separate { &["curve", "y", "z"] } else { &["curve"] };
    for key in keys {
        let curve = size.get(*key).ok_or_else(|| format!("collision current-size stream: size {key} not exported"))?;
        if !matches!(curve.get("mode").and_then(serde_json::Value::as_str), Some("curve" | "twoCurves")) {
            continue;
        }
        for wrap in ["preInfinity", "postInfinity"] {
            if curve.get(wrap).and_then(serde_json::Value::as_u64) != Some(2) {
                return Err(format!("collision current-size stream: size curve {wrap} {:?} is not the clamp wrap",
                    curve.get(wrap)));
            }
        }
    }
    Ok(())
}

/// Whether the emitter's CollisionModule is inside the ported subset (the
/// scene is a separate refusal).
pub(crate) fn collision_eligible(emitter: &EmitterParams) -> Result<(), String> {
    qualify(emitter).map(|_| ())
}

/// Install the module with its scene on a system that has the native birth
/// owner. A Local system needs its owner words; `random` is the Collision
/// stream of the same seed reset that gave the owner its Initial and Shape
/// streams.
pub(crate) fn install(system: &mut Runtime, scene: Box<dyn CollisionScene + Send + Sync>, owner: Option<OwnerPair>,
    random: moly_law::particle::seed_owner::ModuleRandom) -> Result<(), String> {
    let law = qualify(&system.emitter)?.ok_or("no CollisionModule")?;
    if system.emitter.simulation_space == SimulationSpace::Local && owner.is_none() {
        return Err("a Local collision system needs its owner words".into());
    }
    system.collision = Some(CollisionRuntime {
        law,
        state: CollisionState::default(),
        random: Some(CollisionRandom { words: std::array::from_fn(|i| random.words[i / 4][i % 4]) }),
        owner,
        scene,
        calls: 0,
        hits: 0,
        draws: 0,
        unreached: 0,
        order_free: 0,
        listed_order_fallback: false,
        order_dependent: 0,
        size: current_size_law(&system.emitter)?,
        edges: Vec::new(),
        pending: Vec::new(),
    });
    Ok(())
}

/// The pool in source axes; slots at or past `end` are not known.
struct PoolView<'a> {
    pool: &'a [Particle],
    side: &'a [Side],
    end: usize,
    /// The size read is the current-size stream (else the start size).
    current_size: bool,
}

fn source(v: [f32; 3]) -> [f32; 3] {
    [-v[0], v[1], v[2]]
}

impl CollisionParticles for PoolView<'_> {
    fn lane(&self, index: usize) -> Option<ParticleLane> {
        if index >= self.end {
            return None;
        }
        let (particle, side) = (self.pool.get(index)?, self.side.get(index)?);
        Some(ParticleLane {
            position: source(particle.position),
            velocity: source(particle.velocity),
            animated: source(side.animated),
            size: if self.current_size { [side.current_size; 3] } else { side.size },
            // Not read: the speed-modifier flag is refused.
            speed_modifier: 1.0,
            age_percent: particle.age_percent,
            inverse_lifetime: particle.inverse_lifetime,
            seed: side.seed,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn call(system: &mut Runtime, collision: &mut CollisionRuntime, from: usize, to: usize, dt: [f32; 4],
    pending: f32, emission_word: u32) -> Result<(), String> {
    // Every collision edge must be attached: a missing one would drop its
    // events without a trace.
    let authored = system.emitter.sub_emitters.iter().filter(|edge|
        edge.trigger == moly_law::particle::schema::SubEmitterTrigger::Collision && edge.emitter.is_some()).count();
    if collision.edges.len() != authored {
        return Err(format!("collision: {} of {authored} collision sub-emitter edges attached", collision.edges.len()));
    }
    let view = PoolView { pool: &system.pool, side: &system.side, end: to,
        current_size: collision.law.reads_current_size() };
    let edges: Vec<CollisionEmitEdge> = collision.edges.iter().map(|slot| slot.law).collect();
    let input = UpdateInput { from, to, dt, owner: collision.owner, edges: &edges, emission_word, pending };
    let law = collision.law;
    let outcome = match law.update(&mut collision.state, collision.random.as_mut(), &view, &input,
        collision.scene.as_mut(), None) {
        Err(Refused::OrderDependent { dependent, order_free }) if collision.listed_order_fallback => {
            collision.order_dependent += dependent as u64;
            collision.order_free += order_free as u64;
            law.update(&mut collision.state, collision.random.as_mut(), &view, &input,
                &mut ListedOrder(collision.scene.as_mut()), None)
        }
        other => other,
    }.map_err(|refused| format!("collision {refused:?}"))?;
    collision.order_free += outcome.order_free as u64;
    for written in &outcome.written {
        let particle = &mut system.pool[written.index];
        particle.position = source(written.position);
        particle.velocity = source(written.velocity);
        particle.age_percent = written.age_percent;
    }
    for emit in &outcome.emits {
        let slot = &mut collision.edges[emit.edge];
        slot.tally.records += 1;
        if let Some(commands) = &emit.commands {
            slot.tally.commands += commands.len() as u64;
            slot.tally.births += commands.iter().map(|(command, _)| command.count).sum::<u64>();
            if slot.delivered {
                collision.pending.extend(commands.iter().map(|(command, _)| (emit.edge, *command)));
            }
        }
    }
    collision.calls += 1;
    collision.hits += outcome.hits.len() as u64;
    collision.draws += outcome.draws as u64;
    collision.unreached += u64::from(outcome.past_end_unreached);
    Ok(())
}

/// Measurement only: a scene whose listed order is taken as the engine's.
pub(crate) struct ListedOrder<'a>(pub(crate) &'a mut (dyn CollisionScene + Send + Sync));

impl CollisionScene for ListedOrder<'_> {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.0.overlap(query)
    }
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.0.reachable(collides_with)
    }
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        self.0.sweep_sphere(request)
    }
    fn refusal(&self) -> Option<&'static str> {
        self.0.refusal()
    }
}

/// The SizeModule's current size of `[from, to)` at the particles' ages.
fn write_current_size(system: &mut Runtime, collision: &CollisionRuntime, from: usize, to: usize) {
    let Some(size) = collision.size.as_ref() else {
        return;
    };
    for index in from..to.min(system.pool.len()) {
        let age = system.pool[index].age_percent;
        let side = &mut system.side[index];
        side.current_size = size.current(side.size[0], age, side.seed);
    }
}

/// The post-simulation call over the whole pool, then the current-size write
/// over it.
pub(super) fn post_simulation(system: &mut Runtime, collision: &mut CollisionRuntime, dt: f32, pending: f32,
    emission_word: u32) -> Result<(), String> {
    let count = system.pool.len();
    call(system, collision, 0, count, [dt; 4], pending, emission_word)?;
    write_current_size(system, collision, 0, system.pool.len());
    Ok(())
}

/// One newborn group's current-size write and call over `[from, to)` with the
/// group's birth times.
pub(super) fn newborn_block(system: &mut Runtime, collision: &mut CollisionRuntime, from: usize, to: usize,
    dt: [f32; 4], pending: f32, emission_word: u32) -> Result<(), String> {
    write_current_size(system, collision, from, to);
    call(system, collision, from, to, dt, pending, emission_word)
}
