//! Source ControlPlayableAsset adapter: exact instance binding, dormant GPU
//! preparation and Director-owned particle time. Activation belongs to Timeline.
use bevy::{asset::LoadState, prelude::*};
use moly_assets::{json::JsonAsset, particle_source::ParticleSourceModules};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use crate::{
    fixture_activity_timeline::{ControlSettings, TimelineFailure},
    particle_runtime::{Context, Runtime, SourceRoute},
    source_particle::{ParticleReadiness, SourceParticle},
    uber_particle::FixtureParticleLive,
};

fn invalid(message: impl Into<String>) -> TimelineFailure {
    TimelineFailure {
        message: message.into(),
        retryable: false,
    }
}
fn loading(message: impl Into<String>) -> TimelineFailure {
    TimelineFailure {
        message: message.into(),
        retryable: true,
    }
}

#[derive(Clone)]
pub(crate) struct ParticleControlBinding {
    pub root: Entity,
    fixture: Entity,
    draws: Vec<Entity>,
    created: Vec<Entity>,
    seeds: HashMap<Entity, Option<u32>>,
    lease: Arc<()>,
}

#[derive(Component)]
struct Archive {
    index: Handle<JsonAsset>,
    document: Option<Handle<JsonAsset>>,
    parsed: Option<Arc<Value>>,
    last_used: f64,
}

#[derive(Component, Clone)]
struct Prepared {
    root: Entity,
    fixture: Entity,
    draws: Vec<Entity>,
    created: Vec<Entity>,
    seeds: HashMap<Entity, Option<u32>>,
    lease: Weak<()>,
}

impl Prepared {
    fn new(binding: &ParticleControlBinding) -> Self {
        Self {
            root: binding.root,
            fixture: binding.fixture,
            draws: binding.draws.clone(),
            created: binding.created.clone(),
            seeds: binding.seeds.clone(),
            lease: Arc::downgrade(&binding.lease),
        }
    }
    fn binding(&self) -> Option<ParticleControlBinding> {
        Some(ParticleControlBinding {
            root: self.root,
            fixture: self.fixture,
            draws: self.draws.clone(),
            created: self.created.clone(),
            seeds: self.seeds.clone(),
            lease: self.lease.upgrade()?,
        })
    }
}

pub(crate) fn realtime(world: &World) -> f64 {
    world
        .get_resource::<Time<Real>>()
        .map_or(0.0, Time::elapsed_secs_f64)
}

/// One controlled system's ParticleControlPlayable state and the system state
/// its `ParticleSystem.Simulate` calls leave behind. Presence suppresses
/// wall-clock playOnAwake advancement, including when the selected clip is
/// currently outside its interval (requested == None): a Simulate leaves the
/// system paused and out of the per-frame update.
#[derive(Component)]
pub(crate) struct DirectorClock {
    /// The clip-local time of the playable evaluated this frame; `None` when
    /// no playable is evaluated (no PrepareFrame).
    requested: Option<f64>,
    times: PlayableTimes,
    /// The system's stop-emitting state: set at the non-looping end inside an
    /// update, cleared by the restart's Play.
    stop_emitting: bool,
    /// The route the restart's birth decision and warm take.
    route: SourceRoute,
    /// A sub-emitter parent's birth events, which each restart installs.
    event_edges: Option<crate::particle_runtime::EventEdges>,
    /// Sub-emitter commands with no installed target, dropped.
    dropped: u64,
    /// A refused Simulate retires the system: it draws nothing until released.
    failed: bool,
}

/// A ParticleControlPlayable's two floats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlayableTimes {
    /// The playable's last time: FLT_MAX at creation, after OnBehaviourPlay
    /// or OnBehaviourPause and after a frame with the system inactive.
    pub(crate) last: f32,
    /// The system time the playable read after its last Simulate (FLT_MAX at
    /// creation); a system time that moved away from it is an external change.
    pub(crate) last_particle: f32,
}

impl PlayableTimes {
    pub(crate) const NEW: Self = Self { last: f32::MAX, last_particle: f32::MAX };
}

/// What a ParticleControlPlayable drives: the controlled system's time and
/// its two `ParticleSystem.Simulate` entries (withChildren false,
/// fixedTimeStep false).
pub(crate) trait Controlled {
    /// `ParticleSystem.time`: the system clock.
    fn system_time(&self) -> f32;
    /// `Simulate(0, restart: true)`.
    fn restart(&mut self) -> Result<(), String>;
    /// `Simulate(dt, restart: false)`.
    fn chunk(&mut self, dt: f32) -> Result<(), String>;
}

/// The seed a ControlPlayable's Initialize leaves on a controlled system:
/// the first Initialize makes an automatic owner manual with its playable's
/// seed and leaves a manual owner alone, so every later Initialize finds it
/// manual. Kept on the system's instance node, which outlives the playables,
/// the clips and the prepared draws.
#[derive(Component, Clone, Copy)]
pub(crate) struct DirectorOwnerSeed(pub(crate) u32);

/// Simulate leaves a Unity ParticleSystem paused; destroying its playable does
/// not call Play. Only an actual source activation edge may resume playOnAwake.
#[derive(Component)]
pub(crate) struct StoppedByDirector {
    pub(crate) was_inactive: bool,
}

impl StoppedByDirector {
    pub(crate) fn reactivated(&mut self, inactive: bool, play_on_awake: bool) -> bool {
        let resume = self.was_inactive && !inactive && play_on_awake;
        self.was_inactive = inactive;
        resume
    }
}

#[derive(Component)]
pub(crate) struct RestoredAutonomous;

fn json(world: &World, handle: &Handle<JsonAsset>) -> Result<Arc<Value>, TimelineFailure> {
    if let LoadState::Failed(error) = world.resource::<AssetServer>().load_state(handle) {
        return Err(invalid(format!("source particle archive failed: {error}")));
    }
    let asset = world
        .resource::<Assets<JsonAsset>>()
        .get(handle)
        .ok_or_else(|| loading("source particle archive is loading"))?;
    serde_json::from_str(&asset.0)
        .map(Arc::new)
        .map_err(|error| invalid(format!("invalid source particle archive: {error}")))
}

fn archive(world: &mut World, fixture: Entity) -> Result<Arc<Value>, TimelineFailure> {
    let now = realtime(world);
    if let Some(mut archive) = world.get_mut::<Archive>(fixture) {
        archive.last_used = now;
    }
    if let Some(parsed) = world.get::<Archive>(fixture).and_then(|a| a.parsed.clone()) {
        return Ok(parsed);
    }
    if world.get::<Archive>(fixture).is_none() {
        let index = world
            .resource::<AssetServer>()
            .load("moly://fixture-particles-v2/index.json");
        world.entity_mut(fixture).insert(Archive {
            index,
            document: None,
            parsed: None,
            last_used: now,
        });
    }
    let index = world.get::<Archive>(fixture).unwrap().index.clone();
    let document = match world.get::<Archive>(fixture).unwrap().document.clone() {
        Some(handle) => handle,
        None => {
            let index = json(world, &index)?;
            let source = world
                .get::<crate::fixture::FixtureSource>(fixture)
                .ok_or_else(|| invalid("particle control fixture has no source GLB"))?;
            let package = source
                .0
                .path()
                .and_then(|path| path.path().file_stem())
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("particle control source package is missing"))?;
            let file = index["packages"][package]["file"].as_str().ok_or_else(|| {
                invalid(format!("fixture {package} lacks a source particle archive"))
            })?;
            if file.contains('/')
                || file.contains('\\')
                || file == "."
                || file == ".."
                || !file.ends_with(".json")
            {
                return Err(invalid(
                    "particle archive route must be a package JSON filename",
                ));
            }
            let handle = world
                .resource::<AssetServer>()
                .load(format!("moly://fixture-particles-v2/{file}"));
            world.get_mut::<Archive>(fixture).unwrap().document = Some(handle.clone());
            handle
        }
    };
    let parsed = json(world, &document)?;
    world.get_mut::<Archive>(fixture).unwrap().parsed = Some(parsed.clone());
    Ok(parsed)
}

fn collect(world: &World, entity: Entity, prefix: &str, output: &mut Vec<(Entity, String)>) {
    let path = match world.get::<Name>(entity) {
        Some(name) if prefix.is_empty() => name.as_str().to_owned(),
        Some(name) => format!("{prefix}/{}", name.as_str()),
        None => prefix.to_owned(),
    };
    if world.get::<Name>(entity).is_some() {
        output.push((entity, path.clone()));
    }
    if let Some(children) = world.get::<Children>(entity) {
        for child in children.iter() {
            collect(world, child, &path, output);
        }
    }
}

/// The source GameObject a spawned fixture GLB node stands for. Sibling
/// GameObjects may share a name (four `sekai` emitters under one effect), so
/// a node path does not name one object; every fixture GLB node carries its
/// GameObject identity, and so does every archive node and emitter.
fn instance_identity(world: &World, entity: Entity) -> Option<i64> {
    let extras = world.get::<bevy::gltf::GltfExtras>(entity)?;
    serde_json::from_str::<Value>(&extras.value).ok()?["gameObjectId"].as_i64()
}

fn identity(record: &Value, what: &str) -> Result<i64, TimelineFailure> {
    record["gameObjectId"].as_i64().ok_or_else(|| {
        invalid(format!(
            "{}: source {what} identity is missing",
            record["node"].as_str().unwrap_or("?")
        ))
    })
}

/// Each archive node's parent GameObject (0 above a prefab root).
fn parents(nodes: &[Value]) -> Result<HashMap<i64, i64>, TimelineFailure> {
    nodes
        .iter()
        .map(|node| {
            let parent = node["parentGameObjectId"].as_i64().ok_or_else(|| {
                invalid(format!(
                    "{}: source parent identity is missing",
                    node["node"].as_str().unwrap_or("?")
                ))
            })?;
            Ok((identity(node, "node")?, parent))
        })
        .collect()
}

/// Whether archive node `id` is `root` or lies under it.
fn descends(parents: &HashMap<i64, i64>, mut id: i64, root: i64) -> bool {
    for _ in 0..=parents.len() {
        if id == root {
            return true;
        }
        match parents.get(&id) {
            Some(&parent) => id = parent,
            None => return false,
        }
    }
    false
}

/// Whether spawned `entity` is `root` or lies under it.
fn instance_descends(world: &World, mut entity: Entity, root: Entity) -> bool {
    loop {
        if entity == root {
            return true;
        }
        match world.get::<ChildOf>(entity) {
            Some(parent) => entity = parent.parent(),
            None => return false,
        }
    }
}

/// The fixture particle host these emitters are handed to looks an emitter's
/// node record up by path, so with same-named siblings it may read a
/// sibling's record. That is harmless while the siblings agree on what it
/// reads there (activity, and scale for a Local-scaled system) and refused
/// otherwise. It composes a sub-emitter target's owner from the authored
/// transform found the same way, so an archive with sub-emitters admits no
/// shared path.
fn shared_path_reads_agree(
    particles: &[Value],
    nodes: &[Value],
    particle: &Value,
) -> Result<(), TimelineFailure> {
    let path = particle["node"]
        .as_str()
        .ok_or_else(|| invalid("source particle path is missing"))?;
    let shared: Vec<&Value> = nodes
        .iter()
        .filter(|node| node["node"].as_str() == Some(path))
        .collect();
    if shared.len() < 2 {
        return Ok(());
    }
    let id = identity(particle, "particle")?;
    let own = shared
        .iter()
        .find(|node| node["gameObjectId"].as_i64() == Some(id))
        .ok_or_else(|| invalid(format!("{path}: source particle has no node record")))?;
    for field in ["active", "scale"] {
        if shared.iter().any(|node| node[field] != own[field]) {
            return Err(invalid(format!(
                "{path}: same-named source siblings differ in {field}, which the fixture particle host reads by path"
            )));
        }
    }
    for other in particles {
        // Only the enabled module names: another emitter's own unconsumed
        // evidence is its own refusal, when it is selected.
        let enabled = other["system"]["sourceModules"]["enabled"]
            .as_array()
            .ok_or_else(|| {
                invalid(format!(
                    "{}: missing source module inventory; re-extract",
                    other["node"].as_str().unwrap_or("?")
                ))
            })?;
        if enabled
            .iter()
            .any(|name| name.as_str() == Some("SubModule"))
            || other["system"]["subEmitters"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        {
            return Err(invalid(format!(
                "{path}: a same-named source sibling in an archive with sub-emitters needs the host to look node records up by identity"
            )));
        }
    }
    Ok(())
}

fn inventory(doc: &Value, root: i64, settings: &ControlSettings) -> Result<(), TimelineFailure> {
    let nodes = doc["nodes"]
        .as_array()
        .ok_or_else(|| invalid("source node inventory is missing"))?;
    if !nodes
        .iter()
        .any(|node| node["gameObjectId"].as_i64() == Some(root))
    {
        return Err(invalid(
            "bound particle root has no source component inventory",
        ));
    }
    let parents = parents(nodes)?;
    for node in nodes {
        let path = node["node"]
            .as_str()
            .ok_or_else(|| invalid("source node path is missing"))?;
        let id = identity(node, "node")?;
        if !descends(&parents, id, root) {
            continue;
        }
        let check_director = settings.update_director && (settings.search_hierarchy || id == root);
        // GetControlableScripts always searches descendants, independent of the
        // searchHierarchy switch used by GetComponent<PlayableDirector>.
        if !check_director && !settings.update_itime_control {
            continue;
        }
        let classes = node["componentClasses"]
            .as_array()
            .ok_or_else(|| invalid(format!("{path}: component inventory missing; re-extract")))?;
        for class in classes {
            let class = class
                .as_str()
                .ok_or_else(|| invalid("invalid component inventory"))?;
            if check_director && class == "PlayableDirector" {
                return Err(invalid(format!(
                    "{path}: nested Director control is not implemented"
                )));
            }
            // GetControlableScripts yields only the MonoBehaviours that
            // implement ITimeControl, and no type in the client implements
            // it: its declarations name the interface only in the Timeline
            // runtime's own time-control playable. A script under the root
            // (the car's SiteMoveEffect, a ManagedEffect) gets no playable.
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    world: &mut World,
    fixture: Entity,
    bind_name: &str,
    settings: &ControlSettings,
) -> Result<ParticleControlBinding, TimelineFailure> {
    if world.get_entity(fixture).is_err() {
        return Err(invalid("particle fixture was destroyed"));
    }
    let doc = archive(world, fixture)?;
    let particles = doc["emitters"]
        .as_array()
        .ok_or_else(|| invalid("source emitter inventory is missing"))?;
    let nodes = doc["nodes"]
        .as_array()
        .ok_or_else(|| invalid("source node inventory is missing"))?;
    let mut by_id = HashMap::new();
    for (index, particle) in particles.iter().enumerate() {
        if by_id
            .insert(identity(particle, "particle")?, index)
            .is_some()
        {
            return Err(invalid(format!(
                "{}: source particle identity is duplicated",
                particle["node"].as_str().unwrap_or("?")
            )));
        }
    }
    let mut paths = Vec::new();
    collect(world, fixture, "", &mut paths);
    // Each spawned node with the emitter it instantiates.
    let mut instances = Vec::with_capacity(paths.len());
    let mut placed = HashSet::new();
    for (entity, path) in paths {
        let ordinal = instance_identity(world, entity).and_then(|id| by_id.get(&id).copied());
        if let Some(ordinal) = ordinal {
            if !placed.insert(ordinal) {
                return Err(invalid(format!(
                    "{path}: one source particle has two spawned instances"
                )));
            }
        }
        instances.push((entity, path, ordinal));
    }
    // Game BindEffect uses the first source-order ParticleSystem name inside
    // this fixture instance. A renderer or another fixture is not a candidate.
    let (root, root_ordinal) = instances
        .iter()
        .find_map(|(entity, path, ordinal)| {
            ordinal
                .filter(|_| path.rsplit('/').next() == Some(bind_name))
                .map(|ordinal| (*entity, ordinal))
        })
        .ok_or_else(|| invalid(format!("fixture particle binding not found: {bind_name}")))?;
    let root_id = identity(&particles[root_ordinal], "particle")?;
    inventory(&doc, root_id, settings)?;
    if !settings.update_particle {
        return Ok(ParticleControlBinding {
            root,
            fixture,
            draws: Vec::new(),
            created: Vec::new(),
            seeds: HashMap::new(),
            lease: Arc::new(()),
        });
    }
    if let Some(binding) = world.get::<Prepared>(root).and_then(Prepared::binding) {
        validate(world, &binding)?;
        return Ok(binding);
    }
    if let Some(expired) = world.entity_mut(root).take::<Prepared>() {
        discard(world, expired);
    }
    let parents = parents(nodes)?;
    for node in nodes {
        let id = identity(node, "node")?;
        if !descends(&parents, id, root_id) {
            continue;
        }
        let path = node["node"].as_str().unwrap_or("?");
        let classes = node["componentClasses"]
            .as_array()
            .ok_or_else(|| invalid(format!("{path}: controlled component inventory is missing")))?;
        if classes
            .iter()
            .any(|class| class.as_str() == Some("ParticleSystem"))
            && by_id
                .get(&id)
                .is_none_or(|ordinal| !placed.contains(ordinal))
        {
            return Err(invalid(format!(
                "{path}: source particle system/archive instance is incomplete"
            )));
        }
    }
    // Let the normal fixture path finish first, so taking over an already
    // active emitter never creates two autonomous simulations for one node.
    if world
        .get::<crate::uber_particle::FixtureParticlesResolved>(fixture)
        .is_none()
        || world
            .get::<crate::uber_particle::FixtureParticleRequest>(fixture)
            .is_some()
        || world
            .get::<crate::weather_fx::fixture::Request>(fixture)
            .is_some()
    {
        return Err(loading("fixture particle preparation is still loading"));
    }
    let mut selected = Vec::new();
    let mut seeds = HashMap::new();
    for (anchor, path, ordinal) in &instances {
        let Some(ordinal) = *ordinal else {
            continue;
        };
        if !instance_descends(world, *anchor, root) {
            continue;
        }
        let particle = &particles[ordinal];
        if let Some(error) = particle["systemError"].as_str() {
            return Err(invalid(format!("{path}: {error}")));
        }
        let modules = ParticleSourceModules::from_system(&particle["system"])
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        if modules.enabled.iter().any(|name| name == "SubModule")
            || particle["system"]["subEmitters"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        {
            return Err(invalid(format!(
                "{path}: source sub-emitter control is not implemented"
            )));
        }
        // StopEmittingAndClear plus a proven disabled Emission module is an
        // empty simulation, including coffee's renderer-disabled parent PSs.
        if !modules.enabled.iter().any(|name| name == "EmissionModule") {
            continue;
        }
        shared_path_reads_agree(particles, nodes, particle)?;
        let auto_seed = particle["system"]["autoRandomSeed"]
            .as_bool()
            .ok_or_else(|| invalid(format!("{path}: source autoRandomSeed is missing")))?;
        let seed = particle["system"]["randomSeed"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| invalid(format!("{path}: source randomSeed is invalid")))?;
        seeds.insert(*anchor, if auto_seed { None } else { Some(seed) });
        selected.push((*anchor, ordinal));
    }
    let existing: Vec<_> = world
        .query::<(Entity, &FixtureParticleLive)>()
        .iter(world)
        .filter_map(|(entity, live)| {
            live.0
                .anchor
                .filter(|a| seeds.contains_key(a))
                .map(|anchor| (entity, anchor))
        })
        .collect();
    if existing
        .iter()
        .any(|(entity, _)| world.get::<SourceParticle>(*entity).is_none())
    {
        return Err(invalid(
            "existing particle instance lacks source-owned material preparation",
        ));
    }
    let new: Vec<_> = selected
        .iter()
        .copied()
        .filter(|(anchor, _)| !existing.iter().any(|(_, old)| old == anchor))
        .collect();
    let created = crate::weather_fx::fixture::prepare_director_control(world, root, &doc, &new)
        .map_err(invalid)?
        .ok_or_else(|| loading("source particle shader/geometry is preparing"))?;
    let draws: Vec<_> = existing
        .iter()
        .map(|(entity, _)| *entity)
        .chain(created.iter().copied())
        .collect();
    let mut draw_seeds = HashMap::new();
    for &draw in &draws {
        let anchor = world
            .get::<FixtureParticleLive>(draw)
            .and_then(|p| p.0.anchor)
            .ok_or_else(|| invalid("prepared source particle anchor is missing"))?;
        draw_seeds.insert(draw, seeds[&anchor]);
        // Existing autonomous PS state is untouched until owner admission and
        // sample. Only newly-created private draws receive a dormant clock.
        if created.contains(&draw) {
            let clock = initialize(world, draw, settings.random_seed)?;
            world.entity_mut(draw).insert(clock);
            if let Some(mut source) = world.get_mut::<SourceParticle>(draw) {
                source.enabled = true;
            }
        }
    }
    let binding = ParticleControlBinding {
        root,
        fixture,
        draws,
        created,
        seeds: draw_seeds,
        lease: Arc::new(()),
    };
    world.entity_mut(root).insert(Prepared::new(&binding));
    Ok(binding)
}

pub(crate) fn validate(
    world: &World,
    binding: &ParticleControlBinding,
) -> Result<(), TimelineFailure> {
    let mut ancestor = Some(binding.root);
    while let Some(entity) = ancestor {
        if entity == binding.fixture {
            break;
        }
        ancestor = world.get::<ChildOf>(entity).map(ChildOf::parent);
    }
    if ancestor.is_none() {
        return Err(invalid("particle control left its owning fixture"));
    }
    for &draw in &binding.draws {
        if world.get::<FixtureParticleLive>(draw).is_none() {
            return Err(invalid("prepared particle simulation is missing"));
        }
        if let Some(source) = world.get::<SourceParticle>(draw) {
            match &*source.readiness.lock().unwrap() {
                ParticleReadiness::Ready => {}
                ParticleReadiness::Pending => {
                    return Err(loading("particle source GPU is preparing"));
                }
                ParticleReadiness::Failed(error) => {
                    return Err(invalid(format!("particle source GPU: {error}")));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn sample(
    world: &mut World,
    binding: &ParticleControlBinding,
    time: Option<f64>,
    seed: u32,
) -> Result<(), TimelineFailure> {
    validate(world, binding)?;
    if time.is_some_and(|t| !t.is_finite() || t < 0.0 || t > 3600.0) || seed == 0 {
        return Err(invalid(
            "particle Director time/seed is outside the supported finite range",
        ));
    }
    for &draw in &binding.draws {
        world
            .entity_mut(draw)
            .remove::<(StoppedByDirector, RestoredAutonomous)>();
        if world.get::<DirectorClock>(draw).is_none() {
            // A playable taking over a system that another owner played.
            let clock = initialize(world, draw, seed)?;
            world.entity_mut(draw).insert(clock);
        }
        let mut clock = world
            .get_mut::<DirectorClock>(draw)
            .ok_or_else(|| invalid("particle Director clock was released"))?;
        clock.requested = time;
        // OnBehaviourPlay and OnBehaviourPause (a clip boundary; a new
        // playable may replace the old one in the same Update) forget the
        // last time, so the next evaluated frame restarts.
        if time.is_none() {
            clock.times.last = f32::MAX;
        }
    }
    Ok(())
}

/// The owner seed after a ControlPlayable's Initialize: an automatic owner
/// becomes manual with the playable's seed, at least 1; a manual owner keeps
/// its own.
pub(crate) fn initialize_seed(automatic: bool, serialized: u32, clip_seed: u32) -> u32 {
    if automatic { clip_seed.max(1) } else { serialized }
}

/// ParticleControlPlayable.Initialize on one controlled system: an automatic
/// owner becomes manual with the playable's seed unless an earlier Initialize
/// already did (see [`DirectorOwnerSeed`]); a manual owner keeps its
/// serialized seed. The system is stopped and cleared; the playable starts
/// with no last time.
fn initialize(world: &mut World, draw: Entity, clip_seed: u32) -> Result<DirectorClock, TimelineFailure> {
    let (route, event_edges) = world
        .get::<crate::weather_fx::fixture::DirectorRoute>(draw)
        .map(|route| (route.0.clone(), route.1.clone()))
        .or_else(|| world.get::<crate::weather_fx::fixture::Played>(draw)
            .map(|played| (played.route().clone(), played.event_edges().cloned())))
        .ok_or_else(|| invalid("controlled particle system has no source route"))?;
    let live = world
        .get::<FixtureParticleLive>(draw)
        .ok_or_else(|| invalid("prepared particle simulation is missing"))?;
    let anchor = live
        .0
        .anchor
        .ok_or_else(|| invalid("prepared source particle anchor is missing"))?;
    let seed = match world.get::<DirectorOwnerSeed>(anchor) {
        Some(owner) => owner.0,
        None => {
            let seed = match (live.0.emitter.auto_random_seed, live.0.emitter.random_seed) {
                (Some(automatic), Some(seed)) => initialize_seed(automatic, seed, clip_seed),
                (Some(true), None) => initialize_seed(true, 0, clip_seed),
                _ => return Err(invalid("source seed ownership is unknown")),
            };
            world.entity_mut(anchor).insert(DirectorOwnerSeed(seed));
            seed
        }
    };
    let mut live = world.get_mut::<FixtureParticleLive>(draw).unwrap();
    live.0.emitter.random_seed = Some(seed);
    live.0.emitter.auto_random_seed = Some(false);
    crate::particle_runtime::clear_particles(&mut live.0);
    Ok(DirectorClock {
        requested: None,
        times: PlayableTimes::NEW,
        stop_emitting: false,
        route,
        event_edges,
        dropped: 0,
        failed: false,
    })
}

pub(crate) fn release(world: &mut World, binding: &ParticleControlBinding) {
    for &draw in &binding.draws {
        let was_inactive = world
            .get::<FixtureParticleLive>(draw)
            .and_then(|p| p.0.anchor)
            .is_some_and(|anchor| {
                world
                    .get::<moly_assets::scene_state::SourceInactive>(anchor)
                    .is_some()
            });
        // The owner seed Initialize set stays on the emitter; the next restart
        // resets the system from it.
        let mesh = if let Some(mut live) = world.get_mut::<FixtureParticleLive>(draw) {
            clear(&mut live.0);
            Some(live.0.mesh.clone())
        } else {
            None
        };
        if let Some(handle) = mesh {
            if let Some(mesh) = world.resource_mut::<Assets<Mesh>>().get_mut(&handle) {
                *mesh = crate::billboard::empty_mesh();
            }
        }
        if let Some(mut clock) = world.get_mut::<DirectorClock>(draw) {
            clock.requested = None;
            clock.times.last = f32::MAX;
        }
        if let Ok(mut entity) = world.get_entity_mut(draw) {
            entity.insert(StoppedByDirector { was_inactive });
        }
    }
    // Other preparing/running requests may hold the same binding. Destruction
    // waits for the last request/owner lease to drop, never a non-owner release.
}

fn discard(world: &mut World, prepared: Prepared) {
    for draw in prepared.draws {
        if prepared.created.contains(&draw) && world.get::<RestoredAutonomous>(draw).is_none() {
            world.despawn(draw);
        } else if let Ok(mut entity) = world.get_entity_mut(draw) {
            entity.remove::<DirectorClock>();
        }
    }
}

/// Called from the existing particle PostUpdate. No cache outlives its request
/// or a short in-flight preparation grace period, even when admission fails.
pub(crate) fn collect_garbage(world: &mut World) {
    let expired: Vec<_> = world
        .query::<(Entity, &Prepared)>()
        .iter(world)
        .filter_map(|(entity, prepared)| (prepared.lease.strong_count() == 0).then_some(entity))
        .collect();
    for root in expired {
        if let Some(prepared) = world.entity_mut(root).take::<Prepared>() {
            discard(world, prepared);
        }
    }
    let now = realtime(world);
    crate::weather_fx::fixture::discard_abandoned_controls(world, now);
    let used: HashSet<_> = world
        .query::<&Prepared>()
        .iter(world)
        .map(|p| p.fixture)
        .collect();
    let stale: Vec<_> = world
        .query::<(Entity, &Archive)>()
        .iter(world)
        .filter_map(|(entity, archive)| {
            (!used.contains(&entity) && now - archive.last_used > 2.0).then_some(entity)
        })
        .collect();
    for entity in stale {
        world.entity_mut(entity).remove::<Archive>();
    }
}

fn clear(system: &mut Runtime) {
    crate::particle_runtime::clear_particles(system);
    system.emission = Default::default();
    system.ring_cursor = 0;
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.emission_started = false;
    system.prewarmed = true;
    system.born_total = 0;
    system.died_total = 0;
    system.full_total = 0;
    system.refused_total = 0;
}

/// Floor of the external-change threshold: 8 Mathf.Epsilon. Mathf.Epsilon is
/// the smallest subnormal float unless the player flushes subnormals to zero,
/// in which case it is the smallest normal one; which of the two the device
/// uses is not established here, and they differ only for system times below
/// about 1e-37 s.
const EXTERNAL_FLOOR: f32 = f32::from_bits(8);

/// One frame of a controlled system: ParticleControlPlayable.PrepareFrame
/// (see [`prepare_frame`]) on the clip-local time `requested` (none: the
/// playable is not evaluated and the paused system keeps its particles). An
/// inactive system forgets the last time and draws nothing. Returns whether
/// the system draws this frame.
///
/// Each of the playable's Simulate calls enters SimulateChildrenRecursive
/// (withChildren false): when the system's SubModule is enabled, every
/// sub-emitter it names is simulated first, by the same time (by zero at a
/// restart), and then the system itself, unless it is one of them. The
/// fixture host installs no sub-emitter target (the judgement refuses each
/// one), so the call is the system's own update; the commands its updates
/// leave for the targets are dropped, counted, as the played path drops them.
pub(crate) fn advance(
    system: &mut Runtime,
    clock: &mut DirectorClock,
    ctx: &Context,
    inactive: bool,
) -> bool {
    if clock.failed {
        return false;
    }
    if inactive {
        if clock.requested.is_some() {
            clock.times.last = f32::MAX;
        }
        return false;
    }
    let Some(time) = clock.requested else {
        return true;
    };
    let mut times = clock.times;
    let mut target = ControlledSystem { system: &mut *system, stop_emitting: &mut clock.stop_emitting, route: &clock.route,
        event_edges: clock.event_edges.as_ref(), ctx };
    let result = prepare_frame(&mut target, &mut times, time as f32);
    clock.times = times;
    let dropped = system.native_birth.as_mut().and_then(|birth| birth.events.as_mut())
        .map_or(0, |events| events.take_commands().len() as u64);
    if dropped > 0 {
        if clock.dropped == 0 {
            warn!(effect=%system.effect, node=%system.node,
                "sub-emitter commands dropped: this host installs no sub-emitter target");
        }
        clock.dropped += dropped;
    }
    match result {
        Ok(()) => true,
        Err(reason) => {
            error!(%reason, effect=%system.effect, node=%system.node,
                "Director particle Simulate refused: the system is retired and draws nothing");
            clock.failed = true;
            system.pool.clear();
            system.side.clear();
            false
        }
    }
}

/// ParticleControlPlayable.PrepareFrame on an active system, `time` the
/// playable time cast to float: the system restarts when the time went back
/// or the system time moved away from the one the playable last read (beyond
/// `max(1e-6 * magnitude, 8 Mathf.Epsilon)`, a NaN difference included),
/// steps forward by the float difference when the time went forward, and does
/// nothing when it is equal. The playable then keeps the time and the system
/// time read after the simulation.
pub(crate) fn prepare_frame(target: &mut impl Controlled, times: &mut PlayableTimes, time: f32) -> Result<(), String> {
    // Mathf.Max(a, b) is `a > b ? a : b`: a NaN compares false.
    let max = |a: f32, b: f32| if a > b { a } else { b };
    let particle_time = target.system_time();
    let magnitude = max(particle_time.abs(), times.last_particle.abs());
    let threshold = max(magnitude * f32::from_bits(0x3586_37bd), EXTERNAL_FLOOR);
    let external = !((times.last_particle - particle_time).abs() < threshold)
        && !crate::particle_runtime::arm("directorNoExternalCheck");
    let back = if crate::particle_runtime::arm("directorRestartOnEqual") { times.last >= time } else { times.last > time };
    if back || external {
        simulate_playable(target, time, true)?;
    } else if times.last < time {
        simulate_playable(target, time - times.last, false)?;
    }
    times.last = time;
    times.last_particle = target.system_time();
    Ok(())
}

/// ParticleControlPlayable.Simulate: a restart first, then the time in raw
/// chunks of at most Time.maximumDeltaTime, which the player serializes as
/// 1/3 s and never assigns.
fn simulate_playable(target: &mut impl Controlled, mut time: f32, restart: bool) -> Result<(), String> {
    let chunk = if crate::particle_runtime::arm("directorChunkIsParticleStep") {
        crate::particle_runtime::PLAYER_TIME.maximum_particle_timestep
    } else {
        crate::particle_runtime::PLAYER_TIME.maximum_delta_time
    };
    if restart {
        target.restart()?;
    }
    while time > chunk {
        target.chunk(chunk)?;
        time -= chunk;
    }
    if time > 0.0 {
        target.chunk(time)?;
    }
    Ok(())
}

/// A fixture system under a Director: the product's two Simulate entries.
struct ControlledSystem<'a> {
    system: &'a mut Runtime,
    stop_emitting: &'a mut bool,
    route: &'a SourceRoute,
    event_edges: Option<&'a crate::particle_runtime::EventEdges>,
    ctx: &'a Context,
}

impl Controlled for ControlledSystem<'_> {
    fn system_time(&self) -> f32 {
        self.system.playback_head
    }

    fn restart(&mut self) -> Result<(), String> {
        crate::particle_runtime::director_restart(self.system, self.route, self.event_edges, self.ctx)?;
        *self.stop_emitting = false;
        Ok(())
    }

    /// The non-looping end inside an update sets the stop-emitting state
    /// the next chunk's head reads.
    fn chunk(&mut self, dt: f32) -> Result<(), String> {
        let emitting = !*self.stop_emitting;
        let stop = &mut *self.stop_emitting;
        crate::particle_runtime::director_chunk(self.system, dt, emitting, self.ctx, |s| {
            if !s.emitter.looping && s.playback_head >= s.emitter.duration {
                *stop = true;
            }
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> ControlSettings {
        ControlSettings {
            exposed_name: "effect".into(),
            update_particle: true,
            update_director: true,
            update_itime_control: true,
            search_hierarchy: false,
            active: true,
            post_playback: 2,
            random_seed: 42,
        }
    }

    fn clock() -> DirectorClock {
        DirectorClock {
            requested: None,
            times: PlayableTimes::NEW,
            stop_emitting: false,
            route: SourceRoute::Ordinary,
            event_edges: None,
            dropped: 0,
            failed: false,
        }
    }

    /// The test runtime with the manual owner a playable's Initialize leaves.
    fn manual(seed: u32) -> Runtime {
        let mut system = crate::particle_runtime::test_support::runtime();
        system.emitter.random_seed = Some(seed);
        system.emitter.auto_random_seed = Some(false);
        system
    }

    fn positions(system: &Runtime) -> Vec<[u32; 3]> {
        system.pool.iter().map(|p| p.position.map(f32::to_bits)).collect()
    }

    fn context() -> Context {
        Context {
            site: GlobalTransform::IDENTITY,
            sky: GlobalTransform::IDENTITY,
            camera: GlobalTransform::IDENTITY,
        }
    }

    #[test]
    fn capability_no_op_needs_complete_source_inventory() {
        let mut doc = json!({"nodes":[
            {"node":"fx", "gameObjectId":1, "parentGameObjectId":0,
             "componentClasses":["CanvasRenderer","ParticleSystem","ParticleSystemRenderer"]},
            {"node":"fx/root/steam", "gameObjectId":3, "parentGameObjectId":1,
             "componentClasses":["ParticleSystem","ParticleSystemRenderer"]}]});
        assert!(inventory(&doc, 1, &settings()).is_ok());
        assert!(inventory(&doc, 9, &settings()).is_err());
        doc["nodes"][1]["componentClasses"] = Value::Null;
        assert!(inventory(&doc, 1, &settings()).is_err());
        doc["nodes"][1]["componentClasses"] = json!(["MonoBehaviour"]);
        assert!(inventory(&doc, 1, &settings()).is_ok());
        doc["nodes"][1]["componentClasses"] = json!(["PlayableDirector"]);
        assert!(inventory(&doc, 1, &settings()).is_ok());
        let mut hierarchy = settings();
        hierarchy.search_hierarchy = true;
        assert!(inventory(&doc, 1, &hierarchy).is_err());
    }

    #[test]
    fn removing_a_director_does_not_play_without_source_activation() {
        let mut stopped = StoppedByDirector {
            was_inactive: false,
        };
        assert!(!stopped.reactivated(false, true));
        assert!(!stopped.reactivated(false, true));
        assert!(!stopped.reactivated(true, true));
        assert!(!stopped.reactivated(false, false));
        assert!(!stopped.reactivated(true, true));
        assert!(stopped.reactivated(false, true));
    }

    #[test]
    fn director_time_restarts_on_rewind_and_warms_a_prewarm_system() {
        let mut system = manual(42);
        let mut clock = clock();
        // No playable evaluated: nothing runs, the paused system keeps what it holds.
        let held = (system.born_total, positions(&system));
        assert!(advance(&mut system, &mut clock, &context(), false));
        assert_eq!((system.born_total, positions(&system)), held);
        // The first evaluated frame restarts; the prewarm system warms.
        clock.requested = Some(0.0);
        advance(&mut system, &mut clock, &context(), false);
        let warmed = (system.born_total, positions(&system));
        assert!(warmed.0 > 0);
        clock.requested = Some(0.1);
        advance(&mut system, &mut clock, &context(), false);
        let forward = (system.born_total, positions(&system));
        assert!(forward.0 > warmed.0);
        // An equal time does nothing.
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, positions(&system)), forward);
        // A rewind restarts from the same manual seed.
        clock.requested = Some(0.0);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, positions(&system)), warmed);
        clock.requested = Some(0.1);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, positions(&system)), forward);
        // Inactive: nothing drawn and the last time forgotten, so the next
        // active frame restarts and reaches the same state.
        assert!(!advance(&mut system, &mut clock, &context(), true));
        assert!(advance(&mut system, &mut clock, &context(), false));
        assert_eq!((system.born_total, positions(&system)), forward);
    }

    #[test]
    fn same_frame_none_then_some_retains_restart_and_authored_seed() {
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        let fixture = world.spawn_empty().id();
        world.entity_mut(fixture).insert(Archive {
            index: Handle::default(),
            document: None,
            parsed: Some(Arc::new(json!({"temporary":"source inventory"}))),
            last_used: 0.0,
        });
        let root = world.spawn(ChildOf(fixture)).id();
        let draw = world
            .spawn((
                FixtureParticleLive(manual(7)),
                clock(),
                ChildOf(root),
            ))
            .id();
        let binding = ParticleControlBinding {
            root,
            fixture,
            draws: vec![draw],
            created: vec![draw],
            seeds: HashMap::from([(draw, Some(7))]),
            lease: Arc::new(()),
        };
        sample(&mut world, &binding, Some(0.1), 42).unwrap();
        world
            .entity_mut(draw)
            .take::<DirectorClock>()
            .map(|mut clock| {
                advance(
                    &mut world.get_mut::<FixtureParticleLive>(draw).unwrap().0,
                    &mut clock,
                    &context(),
                    false,
                );
                world.entity_mut(draw).insert(clock);
            });
        let born = world.get::<FixtureParticleLive>(draw).unwrap().0.born_total;
        assert!(born > 0);
        sample(&mut world, &binding, None, 42).unwrap();
        sample(&mut world, &binding, Some(0.2), 99).unwrap();
        let mut clock = world.entity_mut(draw).take::<DirectorClock>().unwrap();
        // The clip boundary forgot the last time: this frame restarts.
        assert_eq!(clock.times.last, f32::MAX);
        advance(
            &mut world.get_mut::<FixtureParticleLive>(draw).unwrap().0,
            &mut clock,
            &context(),
            false,
        );
        let live = &world.get::<FixtureParticleLive>(draw).unwrap().0;
        assert!(live.born_total > born);
        // A later playable's seed never replaces the owner seed.
        assert_eq!((live.emitter.random_seed, live.emitter.auto_random_seed), (Some(7), Some(false)));
        world.entity_mut(draw).insert(clock);
        release(&mut world, &binding);
        assert!(world.get::<FixtureParticleLive>(draw).unwrap().0.pool.is_empty());
        assert_eq!(world.get::<DirectorClock>(draw).unwrap().times.last, f32::MAX);
        world.entity_mut(root).insert(Prepared::new(&binding));
        collect_garbage(&mut world);
        assert!(world.get_entity(draw).is_ok()); // request still owns it
        drop(binding);
        let mut now = Time::<Real>::default();
        now.advance_by(std::time::Duration::from_secs(3));
        world.insert_resource(now);
        collect_garbage(&mut world);
        assert!(world.get_entity(draw).is_err());
        assert!(world.get_entity(root).is_ok());
        assert!(world.get::<Archive>(fixture).is_none());
    }

    /// The ControlPlayable side of one receipt case: the product's
    /// PrepareFrame and managed Simulate drive a target whose two Simulate
    /// entries record the native calls they stand for and keep the receipt's
    /// time stand-in (the native Update adds `dt * max(speed, 0)` to the
    /// system time while the system plays, which a Simulate guarantees); the
    /// restart takes the product's restart warm, start delay and seed reset.
    struct Standin {
        emitter: moly_law::particle::EmitterParams,
        route: SourceRoute,
        time: f32,
        delay: u32,
        seed: u32,
        calls: Vec<(String, u32, u32)>,
        warm: Option<(u32, u32)>,
        reset_seed: Option<u32>,
    }

    impl Controlled for Standin {
        fn system_time(&self) -> f32 {
            self.time
        }

        fn restart(&mut self) -> Result<(), String> {
            self.calls.push(("CUSTOM_Simulate".into(), 0, 1));
            let warm = crate::particle_runtime::director_restart_warm(&self.emitter, &self.route, 0.0)?;
            self.reset_seed = Some(self.seed);
            if !self.emitter.prewarm {
                self.delay = match self.emitter.start_delay {
                    moly_law::particle::MinMaxCurve::Constant(v) => v.to_bits(),
                    _ => unreachable!(),
                };
            }
            let procedural = if self.route == SourceRoute::Procedural { 2 } else { 0 };
            let (clock, dt) = warm.map_or((0.0, 0.0), |warm| (warm.initial_clock, warm.compute_out));
            self.warm = Some((dt.to_bits(), clock.to_bits()));
            self.time = clock;
            self.calls.push(("Update".into(), dt.to_bits(), procedural));
            self.time += dt * self.emitter.simulation_speed.max(0.0);
            self.calls.push(("Update".into(), 0, 4));
            Ok(())
        }

        fn chunk(&mut self, dt: f32) -> Result<(), String> {
            self.calls.push(("CUSTOM_Simulate".into(), dt.to_bits(), 0));
            self.calls.push(("Update".into(), dt.to_bits(), 4));
            self.time += dt * self.emitter.simulation_speed.max(0.0);
            Ok(())
        }
    }

    fn receipt_emitter(spec: &Value) -> moly_law::particle::EmitterParams {
        use moly_law::particle::MinMaxCurve;
        let number = |v: &Value| match v {
            Value::String(text) if text == "Infinity" => f32::INFINITY,
            v => v.as_f64().unwrap() as f32,
        };
        let mut emitter = crate::particle_runtime::test_support::runtime().emitter;
        emitter.duration = number(&spec["duration"]);
        emitter.looping = spec["looping"].as_bool().unwrap();
        emitter.prewarm = spec["prewarm"].as_bool().unwrap();
        emitter.simulation_speed = number(&spec["speed"]);
        emitter.start_delay = MinMaxCurve::Constant(number(&spec["startDelay"]));
        let lifetime = spec["lifetime"].as_array().unwrap();
        emitter.start.lifetime = match lifetime[0].as_str().unwrap() {
            "constant" => MinMaxCurve::Constant(number(&lifetime[1])),
            "twoConstants" => MinMaxCurve::TwoConstants { min: number(&lifetime[1]), max: number(&lifetime[2]) },
            other => panic!("lifetime mode {other}"),
        };
        emitter.ring_buffer_mode = moly_law::particle::RingBufferMode::Disabled;
        emitter
    }

    fn hex(v: &Value) -> u32 {
        u32::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    #[derive(Default)]
    struct ChainTally {
        cases: usize,
        events: usize,
        mismatched: usize,
        refused: std::collections::BTreeMap<String, usize>,
        unexplained: usize,
        restarts: usize,
        chunks: usize,
        warms: usize,
        first: Vec<String>,
    }

    /// Why the native trace of one event makes a product refusal of its
    /// restart the right answer, if it does.
    fn refusal_explained(native: &[Value], spec: &Value) -> Option<&'static str> {
        for event in native {
            let name = event[0].as_str().unwrap();
            if name == "ComputePrewarmResult" && event[1] == 0 {
                return Some("Compute fails: the engine stops and clears the system");
            }
            if name == "Update" && event[2].as_u64().unwrap() & 2 != 0 && event[1] != 0 {
                return Some("procedural restart warm");
            }
        }
        (spec["prewarm"] == true && !(spec["speed"].as_f64().unwrap() > 0.0)).then_some("prewarm at simulation speed 0")
    }

    fn replay_chain(receipt: &Value, arm: Option<&'static str>) -> ChainTally {
        let mut tally = ChainTally::default();
        crate::particle_runtime::frame_samples::with_arm(arm, || {
            for case in receipt["cases"].as_array().unwrap() {
                tally.cases += 1;
                let spec = &case["spec"];
                let name = case["name"].as_str().unwrap();
                let mut target = Standin {
                    emitter: receipt_emitter(spec),
                    route: if spec["proc"] == true { SourceRoute::Procedural } else { SourceRoute::Ordinary },
                    time: 0.0,
                    delay: 0,
                    seed: spec["seed"].as_u64().unwrap() as u32,
                    calls: Vec::new(),
                    warm: None,
                    reset_seed: None,
                };
                let mut auto = spec["auto"] == true;
                let mut times = PlayableTimes::NEW;
                'events: for (index, (event, native)) in case["events"].as_array().unwrap().iter()
                    .zip(case["native"].as_array().unwrap()).enumerate() {
                    tally.events += 1;
                    target.calls.clear();
                    target.warm = None;
                    target.reset_seed = None;
                    let kind = event[0].as_str().unwrap();
                    match kind {
                        "initialize" => {
                            target.seed = initialize_seed(auto, target.seed, event[1].as_u64().unwrap() as u32);
                            auto = false;
                        }
                        "play" | "pause" => times.last = f32::MAX,
                        "external" => target.time = f32::from_bits(event[1].as_u64().unwrap() as u32),
                        "prepare" if event[2] == false => times.last = f32::MAX,
                        "prepare" => {
                            let time = event[1].as_f64().unwrap() as f32;
                            if let Err(reason) = prepare_frame(&mut target, &mut times, time) {
                                let native_events = native["events"].as_array().unwrap();
                                match refusal_explained(native_events, spec) {
                                    Some(why) => *tally.refused.entry(why.to_owned()).or_default() += 1,
                                    None => {
                                        tally.unexplained += 1;
                                        if tally.first.len() < 12 {
                                            tally.first.push(format!("{name}#{index} unexplained refusal: {reason}"));
                                        }
                                    }
                                }
                                break 'events;
                            }
                        }
                        other => panic!("{name}: event {other}"),
                    }
                    let snapshot = &native["snapshot"];
                    let calls: Vec<(String, u32, u32)> = native["events"].as_array().unwrap().iter()
                        .filter_map(|e| match e[0].as_str().unwrap() {
                            "CUSTOM_Simulate" => Some(("CUSTOM_Simulate".to_owned(), e[1].as_u64().unwrap() as u32,
                                e[3].as_u64().unwrap() as u32)),
                            "Update" => Some(("Update".to_owned(), e[1].as_u64().unwrap() as u32, e[2].as_u64().unwrap() as u32)),
                            _ => None,
                        })
                        .collect();
                    let result = native["events"].as_array().unwrap().iter()
                        .find(|e| e[0] == "ComputePrewarmResult");
                    let mut problems = Vec::new();
                    if calls != target.calls {
                        problems.push(format!("calls {:?} native {:?}", &target.calls[..target.calls.len().min(6)],
                            &calls[..calls.len().min(6)]));
                    }
                    let ctrl = snapshot["ctrl"].as_array().unwrap();
                    if (hex(&ctrl[0]), hex(&ctrl[1])) != (times.last.to_bits(), times.last_particle.to_bits()) {
                        problems.push(format!("ctrl {:08x} {:08x} native {:?}", times.last.to_bits(),
                            times.last_particle.to_bits(), ctrl));
                    }
                    if hex(&snapshot["time"]) != target.time.to_bits() {
                        problems.push(format!("time {:08x} native {}", target.time.to_bits(), snapshot["time"]));
                    }
                    if hex(&snapshot["delay"]) != target.delay {
                        problems.push(format!("delay {:08x} native {}", target.delay, snapshot["delay"]));
                    }
                    if snapshot["seed"].as_u64().unwrap() as u32 != target.seed || snapshot["auto"] != u64::from(auto) {
                        problems.push(format!("seed {} auto {auto} native {} {}", target.seed, snapshot["seed"], snapshot["auto"]));
                    }
                    if let (Some(result), Some(warm)) = (result, target.warm) {
                        if result[1] == 1 && (result[2].as_u64().unwrap() as u32, result[3].as_u64().unwrap() as u32) != warm {
                            problems.push(format!("warm {warm:?} native {result}"));
                        }
                    }
                    if let Some(seed) = target.reset_seed {
                        tally.restarts += 1;
                        let scalar = moly_law::particle::seed_owner::ScalarRandom::from_seed(seed).words;
                        let module = moly_law::particle::seed_owner::ModuleRandom::from_owner_seed(seed).words;
                        let emission: Vec<u32> = snapshot["emissionRandom"].as_array().unwrap().iter()
                            .map(|w| w.as_u64().unwrap() as u32).collect();
                        let initial: Vec<u32> = snapshot["initialRand"].as_array().unwrap().iter()
                            .map(|w| w.as_u64().unwrap() as u32).collect();
                        let flat: Vec<u32> = (0..16).map(|i| module[i / 4][i % 4]).collect();
                        if emission[3..7] != scalar[..] || initial != flat {
                            problems.push(format!("seed words of {seed}"));
                        }
                    }
                    tally.chunks += target.calls.iter().filter(|c| c.0 == "Update" && c.2 == 4 && c.1 != 0).count();
                    tally.warms += target.calls.iter().filter(|c| c.0 == "Update" && c.2 != 4 && c.1 != 0).count();
                    if !problems.is_empty() {
                        tally.mismatched += 1;
                        if tally.first.len() < 12 {
                            tally.first.push(format!("{name}#{index} {kind}: {}", problems.join("; ")));
                        }
                        break 'events;
                    }
                }
            }
        });
        tally
    }

    /// The Director chain receipt: the source's ParticleControlPlayable
    /// (PrepareFrame, Simulate, Initialize, OnBehaviourPlay/Pause) and
    /// `ParticleSystem.Simulate` (restart branch with ResetSeeds, Play,
    /// ComputePrewarmStartParameters and the warm; chunk branch) executed
    /// natively over generated and boundary cases, its Update an observed
    /// time stand-in. Every event of every case through the product's
    /// PrepareFrame and managed Simulate: the native Simulate and Update calls
    /// (time and flags), the playable's two floats, the system time, the start
    /// delay word, the owner seed and the reset streams must be equal, and a
    /// refused restart must be one the native trace explains (a Compute that
    /// fails, a procedural warm, prewarm at speed 0). Every one-rule arm must
    /// mismatch.
    #[test]
    #[ignore = "MOLY_DIRECTOR_RECEIPT must identify the JP Director chain receipt"]
    fn director_chain_matches_native_receipt() {
        let path = std::env::var_os("MOLY_DIRECTOR_RECEIPT").expect("MOLY_DIRECTOR_RECEIPT");
        let text = std::fs::read_to_string(path).unwrap().replace("Infinity", "\"Infinity\"");
        let receipt: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(receipt["libunitySha256"], "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
        let product = replay_chain(&receipt, None);
        let report = json!({"cases": product.cases, "events": product.events, "mismatchedCases": product.mismatched,
            "refusedCases": product.refused, "unexplainedRefusals": product.unexplained, "restarts": product.restarts,
            "chunkUpdates": product.chunks, "warmUpdates": product.warms, "first": product.first});
        println!("{report}");
        if let Some(path) = std::env::var_os("MOLY_DIRECTOR_RECEIPT_REPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert!(product.cases > 1000 && product.restarts > 0 && product.chunks > 0 && product.warms > 0, "{report}");
        assert_eq!((product.mismatched, product.unexplained), (0, 0), "{report}");
        for arm in ["directorRestartOnEqual", "directorNoExternalCheck", "directorChunkIsParticleStep"] {
            let tally = replay_chain(&receipt, Some(arm));
            println!("arm {arm}: {} mismatched cases", tally.mismatched);
            assert!(tally.mismatched > 0, "arm {arm} must mismatch");
        }
    }

    #[test]
    fn instance_traversal_and_identity_subtrees_do_not_cross_fixtures() {
        let mut world = World::new();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let local = world.spawn((Name::new("model"), ChildOf(first))).id();
        let expected = world.spawn((Name::new("fx"), ChildOf(local))).id();
        world.spawn((Name::new("fx"), ChildOf(second)));
        let mut paths = Vec::new();
        collect(&world, first, "", &mut paths);
        assert_eq!(
            paths,
            vec![(local, "model".into()), (expected, "model/fx".into())]
        );
        assert!(instance_descends(&world, expected, local));
        assert!(!instance_descends(&world, local, expected));
        let parents = HashMap::from([(1, 0), (2, 1), (3, 2), (4, 1)]);
        assert!(descends(&parents, 3, 2));
        assert!(descends(&parents, 2, 2));
        assert!(!descends(&parents, 4, 2));
        assert!(!descends(&parents, 1, 2));
    }
}
