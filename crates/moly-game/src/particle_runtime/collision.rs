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
//! The module's scene is the effect's ground MeshCollider. The sphere sweep
//! against a cooked triangle mesh (the leaf test over the triangles, the
//! midphase order and the depth of an initial overlap) is not ported, so no
//! scene can be bound: admission refuses every collision system with
//! [`COLLISION_SCENE_NOT_PORTED`], and a runtime without an installed scene
//! refuses the slice.
use super::*;
use moly_law::particle::collision_query::{
    CollisionLaw, CollisionParticles, CollisionScene, CollisionState, OwnerPair, ParticleFlags, ParticleLane,
    UpdateInput,
};
use moly_law::particle::collision_response::CollisionRandom;
use moly_law::particle::schema::SubEmitterTrigger;

/// Why no collision system is admitted.
pub(crate) const COLLISION_SCENE_NOT_PORTED: &str = "collision scene: the sphere sweep against the ground \
    MeshCollider (the triangle leaf test, the midphase and the initial-overlap depth) is not ported";

/// The only collision mask admitted: the ground layer. With it every effect
/// ships at most one enabled collider, so the order of several broadphase
/// touches never matters; the site collider rows carry no layer, so other
/// masks cannot be resolved.
const GROUND_LAYER_MASK: u32 = 1 << 3;

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
}

/// The module law for an emitter, with the product's own limits: the ground
/// layer only, no current-size stream (its SizeModule law at the collision
/// points needs the engine curve form), no per-particle speed modifier, no
/// collision sub-emitter commands (no child delivery on this path) and World
/// or Local space. The particle-state flags follow the module set: without a
/// size-over-lifetime or noise module (a size-by-speed module has no consumer
/// on this runtime at all) the start-size stream is read, as its X or, with
/// the 3D start size, the largest component; without a speed modifier none is
/// applied. The flags of the corpus collision systems agree with this; the
/// flags' writer was not read.
pub(super) fn qualify(emitter: &EmitterParams) -> Result<Option<CollisionLaw>, String> {
    let Some(params) = emitter.collision.as_ref() else {
        return Ok(None);
    };
    if params.collides_with != GROUND_LAYER_MASK {
        return Err(format!("collision mask {:#x}: only the ground layer is resolved", params.collides_with));
    }
    if emitter.size_over_lifetime.is_some() || emitter.noise.is_some() {
        return Err("collision current-size stream: the size law at the collision points needs the engine curve form".into());
    }
    if emitter.velocity_over_lifetime.as_ref().is_some_and(|velocity|
        !matches!(velocity.speed_modifier, moly_law::particle::MinMaxCurve::Constant(m) if m == 1.0)) {
        return Err("collision with a per-particle speed modifier".into());
    }
    if emitter.sub_emitters.iter().any(|edge| edge.trigger == SubEmitterTrigger::Collision) {
        return Err("collision sub-emitter commands have no child delivery on this path".into());
    }
    let world = match emitter.simulation_space {
        SimulationSpace::World => true,
        SimulationSpace::Local => false,
        _ => return Err("collision in a custom simulation space".into()),
    };
    let flags = ParticleFlags { current_size: false, size_3d: emitter.start.size3d, speed_modifier: false };
    CollisionLaw::from_params(params, world, flags).map(Some).map_err(|refused| format!("{refused:?}"))
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
#[allow(dead_code)]
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
    });
    Ok(())
}

/// The pool in source axes; slots at or past `end` are not known.
struct PoolView<'a> {
    pool: &'a [Particle],
    side: &'a [Side],
    end: usize,
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
            size: side.size,
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
    let view = PoolView { pool: &system.pool, side: &system.side, end: to };
    let input = UpdateInput { from, to, dt, owner: collision.owner, edges: &[], emission_word, pending };
    let outcome = collision.law
        .update(&mut collision.state, collision.random.as_mut(), &view, &input, collision.scene.as_mut(), None)
        .map_err(|refused| format!("collision {refused:?}"))?;
    for written in &outcome.written {
        let particle = &mut system.pool[written.index];
        particle.position = source(written.position);
        particle.velocity = source(written.velocity);
        particle.age_percent = written.age_percent;
    }
    collision.calls += 1;
    collision.hits += outcome.hits.len() as u64;
    collision.draws += outcome.draws as u64;
    collision.unreached += u64::from(outcome.past_end_unreached);
    Ok(())
}

/// The post-simulation call over the whole pool.
pub(super) fn post_simulation(system: &mut Runtime, collision: &mut CollisionRuntime, dt: f32, pending: f32,
    emission_word: u32) -> Result<(), String> {
    let count = system.pool.len();
    call(system, collision, 0, count, [dt; 4], pending, emission_word)
}

/// One newborn group's call over `[from, to)` with the group's birth times.
pub(super) fn newborn_block(system: &mut Runtime, collision: &mut CollisionRuntime, from: usize, to: usize,
    dt: [f32; 4], pending: f32, emission_word: u32) -> Result<(), String> {
    call(system, collision, from, to, dt, pending, emission_word)
}
