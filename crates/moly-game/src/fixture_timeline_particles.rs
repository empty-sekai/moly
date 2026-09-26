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
    /// The particle package this holder's document was chosen by: the
    /// fixture's own source package, or the package a director owner named.
    package: Option<String>,
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

/// On an emitter node whose system a prefab's director owner took over
/// ([`prepare_object`], [`prepare_played_object`]): from the owner's first
/// preparation the system is the owner's (a ControlPlayable's Initialize stops
/// and clears it at graph build; a Signal-played system waits for its Play),
/// so a scene host that plays the scene's systems from load skips it. Never
/// removed: Simulate and Stop leave a system out of the per-frame manager,
/// and nothing plays it again but its owner or an activation edge.
#[derive(Component)]
pub(crate) struct DrivenByPrefabOwner;

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

/// The particle document a holder's controls read: `package` names it for a
/// director owner whose prefab carries no fixture source of its own (a step
/// item, a site scene, a cut scene); `None` reads the fixture's own source
/// package. One holder reads one package.
fn archive(world: &mut World, fixture: Entity, package: Option<&str>) -> Result<Arc<Value>, TimelineFailure> {
    let now = realtime(world);
    if let Some(mut archive) = world.get_mut::<Archive>(fixture) {
        archive.last_used = now;
    }
    if let (Some(wanted), Some(held)) = (package, world.get::<Archive>(fixture).and_then(|a| a.package.clone())) {
        if wanted != held {
            return Err(invalid(format!(
                "particle owner already reads the {held} particle document, not {wanted}"
            )));
        }
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
            package: package.map(str::to_owned),
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
            let package = match package {
                Some(package) => package.to_owned(),
                None => world
                    .get::<crate::fixture::FixtureSource>(fixture)
                    .ok_or_else(|| invalid("particle control fixture has no source GLB"))?
                    .0
                    .path()
                    .and_then(|path| path.path().file_stem())
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("particle control source package is missing"))?,
            };
            let package = package.as_str();
            world.get_mut::<Archive>(fixture).unwrap().package = Some(package.to_owned());
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
    let parsed = shared_document(world, &document)?;
    world.get_mut::<Archive>(fixture).unwrap().parsed = Some(parsed.clone());
    Ok(parsed)
}

/// Parsed particle documents by asset, shared by every holder reading the
/// same package (a step item, the site it plays on and the site's director
/// read one document); a document lives while a holder keeps it.
#[derive(Resource, Default)]
struct ParsedDocuments(HashMap<AssetId<JsonAsset>, Weak<Value>>);

fn shared_document(world: &mut World, handle: &Handle<JsonAsset>) -> Result<Arc<Value>, TimelineFailure> {
    let id = handle.id();
    if let Some(parsed) = world
        .get_resource::<ParsedDocuments>()
        .and_then(|cache| cache.0.get(&id))
        .and_then(Weak::upgrade)
    {
        return Ok(parsed);
    }
    let parsed = json(world, handle)?;
    let mut cache = world.get_resource_or_insert_with(ParsedDocuments::default);
    cache.0.retain(|_, held| held.strong_count() > 0);
    cache.0.insert(id, Arc::downgrade(&parsed));
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
/// GameObject identity, and so does every archive node and emitter. A site
/// or step-item GLB node carries it as its source object identity; within
/// one package's document the GameObject path id names one object.
fn instance_identity(world: &World, entity: Entity) -> Option<i64> {
    let fixture = world
        .get::<bevy::gltf::GltfExtras>(entity)
        .and_then(|extras| serde_json::from_str::<Value>(&extras.value).ok())
        .and_then(|extras| extras["gameObjectId"].as_i64());
    fixture.or_else(|| {
        world
            .get::<moly_assets::source_navigation::SourceObjectIdentity>(entity)
            .map(|source| source.game_object)
    })
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

/// One package document's emitter and node lists, with each emitter by its
/// GameObject identity.
struct Inventory<'a> {
    particles: &'a [Value],
    nodes: &'a [Value],
    by_id: HashMap<i64, usize>,
}

impl<'a> Inventory<'a> {
    fn of(doc: &'a Value) -> Result<Self, TimelineFailure> {
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
        Ok(Self { particles, nodes, by_id })
    }

    /// Each spawned node under `top` with the emitter it instantiates.
    fn instances(&self, world: &World, top: Entity) -> Result<Instances, TimelineFailure> {
        let mut paths = Vec::new();
        collect(world, top, "", &mut paths);
        let mut instances = Vec::with_capacity(paths.len());
        let mut placed = HashSet::new();
        for (entity, path) in paths {
            let ordinal = instance_identity(world, entity).and_then(|id| self.by_id.get(&id).copied());
            if let Some(ordinal) = ordinal {
                if !placed.insert(ordinal) {
                    return Err(invalid(format!(
                        "{path}: one source particle has two spawned instances"
                    )));
                }
            }
            instances.push((entity, path, ordinal));
        }
        Ok(Instances { rows: instances, placed })
    }
}

struct Instances {
    rows: Vec<(Entity, String, Option<usize>)>,
    placed: HashSet<usize>,
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
    let doc = archive(world, fixture, None)?;
    let inventory = Inventory::of(&doc)?;
    let instances = inventory.instances(world, fixture)?;
    // Game BindEffect uses the first source-order ParticleSystem name inside
    // this fixture instance. A renderer or another fixture is not a candidate.
    let (root, root_ordinal) = instances
        .rows
        .iter()
        .find_map(|(entity, path, ordinal)| {
            ordinal
                .filter(|_| path.rsplit('/').next() == Some(bind_name))
                .map(|ordinal| (*entity, ordinal))
        })
        .ok_or_else(|| invalid(format!("fixture particle binding not found: {bind_name}")))?;
    let root_id = identity(&inventory.particles[root_ordinal], "particle")?;
    prepare_root(world, fixture, &doc, &inventory, &instances, root, root_id, settings, true)
}

/// A Control clip of a director whose prefab carries no fixture particle
/// archive of its own (a step item, a site scene's director, a cut scene):
/// `object` is the spawned node its `sourceGameObject` resolves to on the
/// director (`ExposedReference.Resolve`), `owner` the spawned prefab root the
/// director owns, and `package` the particle package whose document holds
/// that prefab (the timeline's own package: an exposed reference or a default
/// value of file 0 names an object of the same serialized file). The binding
/// is then driven by [`sample`], [`validate`] and [`release`] exactly as the
/// fixture route's.
///
/// ControlPlayableAsset.CreatePlayable controls a ParticleSystem only when
/// the object carries one itself or `searchHierarchy` is set
/// (GetControllableParticleSystems); it then takes every ParticleSystem of the
/// object's Transform subtree in pre-order, one ParticleControlPlayable each.
pub(crate) fn prepare_object(
    world: &mut World,
    owner: Entity,
    object: Entity,
    package: &str,
    settings: &ControlSettings,
) -> Result<ParticleControlBinding, TimelineFailure> {
    if world.get_entity(owner).is_err() || world.get_entity(object).is_err() {
        return Err(invalid("particle owner or controlled object was destroyed"));
    }
    if !instance_descends(world, object, owner) {
        return Err(invalid("controlled object is outside its director's prefab"));
    }
    let root_id = instance_identity(world, object)
        .ok_or_else(|| invalid("controlled object carries no source GameObject identity"))?;
    let doc = archive(world, owner, Some(package))?;
    let source = Inventory::of(&doc)?;
    let node = source
        .nodes
        .iter()
        .find(|node| node["gameObjectId"].as_i64() == Some(root_id))
        .ok_or_else(|| invalid(format!("GameObject {root_id} is not in the {package} particle document")))?;
    let carries_system = node["componentClasses"]
        .as_array()
        .ok_or_else(|| invalid(format!("GameObject {root_id}: component inventory missing; re-extract")))?
        .iter()
        .any(|class| class.as_str() == Some("ParticleSystem"));
    let instances = source.instances(world, object)?;
    if !carries_system && !settings.search_hierarchy {
        inventory(&doc, root_id, settings)?;
        return Ok(empty_binding(object, owner));
    }
    prepare_root(world, owner, &doc, &source, &instances, object, root_id, settings, false)
}

/// The systems of one object that an owner plays and stops by
/// `ParticleSystem.Play()` / `Stop()` on the object's own system: a timeline
/// Signal reaction (a SignalReceiver's persistent call on the target), which
/// runs them on the per-frame manager path, not by Simulate. The systems are
/// the object's: its draws are its children and go with it, whoever sent the
/// signal (a step item's director may end long before the object does).
#[derive(Clone)]
pub(crate) struct ParticlePlayBinding {
    pub root: Entity,
    draws: Vec<Entity>,
}

#[derive(Component)]
struct PreparedPlay {
    draws: Vec<Entity>,
    /// Stop() ran since the last Play(), or no Play ran yet: the next Play
    /// restarts the systems (Stop set their restart flag). A Play on a
    /// playing system restarts nothing.
    stopped: bool,
}

/// Prepare the systems `ParticleSystem.Play(withChildren: true)` on `object`'s
/// own system reaches: that system and every system of its Transform subtree,
/// stopped until [`play_object`]. `package`, `owner` and `object` as in
/// [`prepare_object`]. A sub-emitter parent is refused (its children would be
/// played by their parent, which this host does not install). Retry while
/// the error is retryable.
pub(crate) fn prepare_played_object(
    world: &mut World,
    owner: Entity,
    object: Entity,
    package: &str,
) -> Result<ParticlePlayBinding, TimelineFailure> {
    if world.get_entity(owner).is_err() || world.get_entity(object).is_err() {
        return Err(invalid("particle owner or played object was destroyed"));
    }
    if !instance_descends(world, object, owner) {
        return Err(invalid("played object is outside its owner's prefab"));
    }
    if let Some(prepared) = world.get::<PreparedPlay>(object) {
        return Ok(ParticlePlayBinding { root: object, draws: prepared.draws.clone() });
    }
    let root_id = instance_identity(world, object)
        .ok_or_else(|| invalid("played object carries no source GameObject identity"))?;
    let doc = archive(world, owner, Some(package))?;
    let inventory = Inventory::of(&doc)?;
    if !inventory.by_id.contains_key(&root_id) {
        return Err(invalid(format!("GameObject {root_id} carries no ParticleSystem in the {package} particle document")));
    }
    let instances = inventory.instances(world, object)?;
    let mut selected = Vec::new();
    for (anchor, path, ordinal) in &instances.rows {
        let Some(ordinal) = *ordinal else { continue };
        let particle = &inventory.particles[ordinal];
        if let Some(error) = particle["systemError"].as_str() {
            return Err(invalid(format!("{path}: {error}")));
        }
        let modules = ParticleSourceModules::from_system(&particle["system"])
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        // A sub-emitter parent plays with its targets installed as its
        // children at the first Play (see
        // [`crate::weather_fx::fixture::link_targets`]); an edge with no
        // system hands out no sub-emitter (the engine skips a null one).
        // A system whose Emission module is off never emits; Play leaves it empty.
        if !modules.enabled.iter().any(|name| name == "EmissionModule") {
            continue;
        }
        shared_path_reads_agree(inventory.particles, inventory.nodes, particle)?;
        selected.push((*anchor, ordinal));
    }
    for (anchor, _) in &selected {
        world.entity_mut(*anchor).insert(DrivenByPrefabOwner);
    }
    let draws = crate::weather_fx::fixture::prepare_play_later(world, object, &doc, &selected)
        .map_err(invalid)?
        .ok_or_else(|| loading("source particle shader/geometry is preparing"))?;
    for &draw in &draws {
        if let Some(mut source) = world.get_mut::<SourceParticle>(draw) {
            source.enabled = true;
        }
    }
    world.entity_mut(object).insert(PreparedPlay {
        draws: draws.clone(),
        stopped: true,
    });
    Ok(ParticlePlayBinding { root: object, draws })
}

/// `ParticleSystem.Play()` on the object's system (withChildren): a system
/// never played takes its first Play (seed reset, birth owner, first-Play
/// warm); a stopped one plays again (see
/// [`crate::weather_fx::fixture::play`]); a playing one is left as it is.
/// Returns how many systems it played.
pub(crate) fn play_object(world: &mut World, binding: &ParticlePlayBinding) -> Result<usize, String> {
    let Some(mut prepared) = world.entity_mut(binding.root).take::<PreparedPlay>() else {
        return Err("played object was released".into());
    };
    let result = (|| {
        if !prepared.stopped {
            return Ok(0);
        }
        // Systems that played before: Play after Stop.
        let mut played = crate::weather_fx::fixture::play(world, binding.root)?;
        let mut first = false;
        for &draw in &binding.draws {
            if world.get_entity(draw).is_ok() && crate::weather_fx::fixture::play_pending(world, draw)? {
                played += 1;
                first = true;
            }
        }
        if first {
            crate::weather_fx::fixture::link_targets(world, &binding.draws);
        }
        prepared.stopped = false;
        Ok(played)
    })();
    world.entity_mut(binding.root).insert(prepared);
    result
}

/// `ParticleSystem.Stop()` on the object's system: `Stop(withChildren: true,
/// StopEmitting)`, so the live particles finish their lifetimes. Returns how
/// many systems it stopped.
pub(crate) fn stop_object(world: &mut World, binding: &ParticlePlayBinding) -> usize {
    let stopped = crate::weather_fx::fixture::stop_emitting(world, binding.root);
    if let Some(mut prepared) = world.get_mut::<PreparedPlay>(binding.root) {
        prepared.stopped = true;
    }
    stopped
}

/// What the seed pass of [`seed_played_object`] did.
// Its caller is the cut-scene effect clip's player, which calls it from its
// own module.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeedPass {
    /// Listed systems that took `randomSeed = 0` alone (not playing).
    pub seeded: usize,
    /// Listed systems that were playing: stopped and cleared with their
    /// subtree, seeded, and played again with it.
    pub restarted: usize,
    /// Listed systems this host does not simulate (not prepared: no Emission
    /// module, or refused by admission); the seed they take shows nowhere.
    pub unsimulated: usize,
}

/// The seed pass of `EffectBehaviour.SetEffectInstance` on a played object,
/// unless the clip keeps its seeds (`isEnabledRandomSeed`). `listed` are the
/// nodes of the systems `GetComponentsInChildren<ParticleSystem>()` lists on
/// the instance, in that order; each is `object` or lies under it. In order,
/// a listed system that is playing takes `Stop(withChildren: true,
/// StopEmittingAndClear)`, `randomSeed = 0` and `Play()` (withChildren), and
/// any other takes `randomSeed = 0` alone. The seed write makes the system a
/// manual owner (useAutoRandomSeed false, even for an unchanged seed) and
/// resets nothing (see [`crate::particle_runtime::set_random_seed`]), so the
/// seed shows at the next seed reset: the Play after the stop and clear
/// above, or the first Play of the activation that follows the pass. A
/// system's Play reaches the systems below it, so in the playing branch a
/// later listed system of the same subtree is playing again when its turn
/// comes, and it restarts in turn.
#[allow(dead_code)] // called by the cut-scene effect clip's player
pub(crate) fn seed_played_object(
    world: &mut World,
    binding: &ParticlePlayBinding,
    listed: &[Entity],
) -> Result<SeedPass, String> {
    let anchor_of = |world: &World, draw: Entity| world.get::<FixtureParticleLive>(draw).and_then(|live| live.0.anchor);
    let mut pass = SeedPass::default();
    for &node in listed {
        if !instance_descends(world, node, binding.root) {
            return Err("a listed system lies outside its played object".into());
        }
        let Some(draw) = binding.draws.iter().copied().find(|&draw| anchor_of(world, draw) == Some(node)) else {
            pass.unsimulated += 1;
            continue;
        };
        if !crate::weather_fx::fixture::system_playing(world, draw) {
            crate::weather_fx::fixture::set_random_seed(world, draw, 0);
            pass.seeded += 1;
            continue;
        }
        let subtree: Vec<Entity> = binding.draws.iter().copied()
            .filter(|&other| anchor_of(world, other).is_some_and(|anchor| instance_descends(world, anchor, node)))
            .collect();
        crate::weather_fx::fixture::stop_and_clear(world, &subtree);
        crate::weather_fx::fixture::set_random_seed(world, draw, 0);
        crate::weather_fx::fixture::play_draws(world, &subtree)?;
        for &other in &subtree {
            crate::weather_fx::fixture::play_pending(world, other)?;
        }
        crate::weather_fx::fixture::link_targets(world, &binding.draws);
        pass.restarted += 1;
    }
    Ok(pass)
}

/// `ParticleSystem.Stop(withChildren: true, StopEmittingAndClear)` on the
/// object's system (the looping matched-duration branch of
/// `EffectBehaviour.OnBehaviourPause`): every system it reaches stops
/// emitting and is cleared at once, and its play ends. Returns how many
/// played systems it stopped.
#[allow(dead_code)] // called by the cut-scene effect clip's player
pub(crate) fn stop_and_clear_object(world: &mut World, binding: &ParticlePlayBinding) -> usize {
    let stopped = crate::weather_fx::fixture::stop_and_clear(world, &binding.draws);
    if let Some(mut prepared) = world.get_mut::<PreparedPlay>(binding.root) {
        prepared.stopped = true;
    }
    stopped
}

/// A played object over draws a replay built itself (no document, no GPU
/// preparation), already playing.
#[cfg(test)]
pub(crate) fn played_object_for_replay(world: &mut World, root: Entity, draws: Vec<Entity>) -> ParticlePlayBinding {
    world.entity_mut(root).insert(PreparedPlay { draws: draws.clone(), stopped: false });
    ParticlePlayBinding { root, draws }
}

/// The call a SignalReceiver reaction makes on a ParticleSystem: its
/// persistent call with no argument (`ParticleSystem.Play()`, `Stop()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignalCall {
    Play,
    Stop,
}

/// A Signal marker of a director's timeline with the reaction of the
/// receiver its track is bound to: at `time` the receiver's persistent call
/// runs `call` on `target`'s system. `emit_once`: the marker's own flag.
#[derive(Clone)]
pub(crate) struct SignalReaction {
    pub time: f64,
    pub emit_once: bool,
    pub signal: String,
    pub call: SignalCall,
    pub target: ParticlePlayBinding,
}

/// Run one reaction's call; returns how many systems it played or stopped.
pub(crate) fn react(world: &mut World, reaction: &SignalReaction) -> Result<usize, String> {
    match reaction.call {
        SignalCall::Play => play_object(world, &reaction.target),
        SignalCall::Stop => Ok(stop_object(world, &reaction.target)),
    }
}

/// Developer stand-in (DevTools only) for the Signal markers and receiver
/// reactions the timeline tables do not carry yet. It knows one Signal
/// track: the dewdrop step item's, which the delivery flow binds to the
/// site's delivery signal receiver. Its two markers (the tree effect's play
/// at 0.6833 s, sent once, and its stop at 1.15 s) and that receiver (on the
/// site tree's `fx_bdtree02`, whose reactions call `Play()` / `Stop()` on the
/// same object's system) are read from the site package. Every other
/// definition gets no reactions. The exported (time, signal, receiver)
/// rows replace it.
pub(crate) fn signal_stand_in(
    world: &mut World,
    definition: &crate::fixture_activity_timeline::TimelineDefinition,
) -> Result<Vec<SignalReaction>, TimelineFailure> {
    const DIRECTOR: &str = "-3154110674756471997";
    const TRACK: &str = "-1252092068188085713";
    const RECEIVER: i64 = 6653597469877562143;
    const MARKERS: [(f64, bool, &str, SignalCall); 2] = [
        (0.6833333333333335, true, "fx_bdtree02_effect_play", SignalCall::Play),
        (1.1500000000000004, false, "fx_bdtree02_effect_stop", SignalCall::Stop),
    ];
    if !crate::dev_tools::installed()
        || definition.director.path_id != DIRECTOR
        || !definition
            .tracks
            .iter()
            .any(|track| track.class == "SignalTrack" && track.identity.path_id == TRACK)
    {
        return Ok(Vec::new());
    }
    let file = definition.director.file.clone();
    let hits: Vec<Entity> = world
        .query::<(Entity, &moly_assets::source_navigation::SourceObjectIdentity)>()
        .iter(world)
        .filter(|(_, identity)| identity.file == file && identity.components.contains(&RECEIVER))
        .map(|(entity, _)| entity)
        .collect();
    let [target] = hits.as_slice() else {
        warn!(
            "[prefab-director] {} spawned nodes carry the site's delivery signal receiver; the Signal stand-in sends nothing",
            hits.len()
        );
        return Ok(Vec::new());
    };
    let mut owner = *target;
    while world
        .get::<moly_assets::coordinates::CanonicalCoordinates>(owner)
        .is_none()
    {
        owner = world
            .get::<ChildOf>(owner)
            .map(ChildOf::parent)
            .ok_or_else(|| invalid("the signal receiver has no coordinate-contract ancestor"))?;
    }
    let binding = match prepare_played_object(world, owner, *target, &definition.package) {
        Ok(binding) => binding,
        Err(error) if error.retryable => return Err(error),
        Err(error) => {
            warn!(
                "[prefab-director] the Signal stand-in's receiver target {:?} is refused by the particle host: {}; no signal is sent",
                world.get::<Name>(*target).map(Name::as_str),
                error.message
            );
            return Ok(Vec::new());
        }
    };
    Ok(MARKERS
        .iter()
        .map(|&(time, emit_once, signal, call)| SignalReaction {
            time,
            emit_once,
            signal: signal.to_owned(),
            call,
            target: binding.clone(),
        })
        .collect())
}

/// Developer trace (DevTools only), once a second: the live and born counts
/// of every object whose systems a prefab's director owner drives.
pub(crate) fn trace_owned(world: &mut World) {
    if !crate::dev_tools::installed() {
        return;
    }
    let now = realtime(world);
    let last = world.get_resource_or_insert_with(|| OwnedTrace(f64::NEG_INFINITY)).0;
    if now - last < 1.0 {
        return;
    }
    world.resource_mut::<OwnedTrace>().0 = now;
    let mut objects: Vec<(Entity, Vec<Entity>, &'static str)> = world
        .query::<(Entity, &Prepared)>()
        .iter(world)
        .map(|(entity, prepared)| (entity, prepared.draws.clone(), "control"))
        .collect();
    objects.extend(
        world
            .query::<(Entity, &PreparedPlay)>()
            .iter(world)
            .map(|(entity, prepared)| (entity, prepared.draws.clone(), "signal")),
    );
    let mut rows = Vec::new();
    for (object, draws, route) in objects {
        let (mut systems, mut live, mut born, mut owned) = (0usize, 0usize, 0u64, false);
        for draw in &draws {
            let Some(system) = world.get::<FixtureParticleLive>(*draw) else {
                continue;
            };
            owned |= system
                .0
                .anchor
                .is_some_and(|anchor| world.get::<DrivenByPrefabOwner>(anchor).is_some());
            systems += 1;
            live += system.0.pool.len();
            born += system.0.born_total;
        }
        if owned {
            rows.push(format!(
                "{object:?} {:?} {route} systems {systems} live {live} born {born}",
                world.get::<Name>(object).map(Name::as_str)
            ));
        }
    }
    if !rows.is_empty() {
        rows.sort();
        info!("[prefab-director-trace] {}", rows.join("; "));
    }
}

#[derive(Resource)]
struct OwnedTrace(f64);

fn empty_binding(root: Entity, fixture: Entity) -> ParticleControlBinding {
    ParticleControlBinding {
        root,
        fixture,
        draws: Vec::new(),
        created: Vec::new(),
        seeds: HashMap::new(),
        lease: Arc::new(()),
    }
}

/// The controlled systems under `root` and their dormant draws. `fixture_host`
/// waits for a fixture's own autonomous preparation first, so taking over an
/// already active emitter never creates two simulations for one node.
#[allow(clippy::too_many_arguments)]
fn prepare_root(
    world: &mut World,
    fixture: Entity,
    doc: &Value,
    source: &Inventory<'_>,
    instances: &Instances,
    root: Entity,
    root_id: i64,
    settings: &ControlSettings,
    fixture_host: bool,
) -> Result<ParticleControlBinding, TimelineFailure> {
    let (particles, nodes, by_id, placed) = (source.particles, source.nodes, &source.by_id, &instances.placed);
    let instances = &instances.rows;
    inventory(doc, root_id, settings)?;
    if !settings.update_particle {
        return Ok(empty_binding(root, fixture));
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
    if fixture_host
        && (world
            .get::<crate::uber_particle::FixtureParticlesResolved>(fixture)
            .is_none()
            || world
                .get::<crate::uber_particle::FixtureParticleRequest>(fixture)
                .is_some()
            || world
                .get::<crate::weather_fx::fixture::Request>(fixture)
                .is_some())
    {
        return Err(loading("fixture particle preparation is still loading"));
    }
    let mut selected = Vec::new();
    let mut seeds = HashMap::new();
    for (anchor, path, ordinal) in instances {
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
    if !fixture_host {
        for (anchor, _) in &selected {
            world.entity_mut(*anchor).insert(DrivenByPrefabOwner);
        }
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
    let created = if fixture_host {
        crate::weather_fx::fixture::prepare_director_control(world, root, doc, &new)
    } else {
        crate::weather_fx::fixture::prepare_owner_director_control(world, root, doc, &new)
    }
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
        // Preparation returned this draw only once its GPU source was Ready.
        // The render world resets readiness to Pending at each frame's queue
        // and sets Ready again once the pipelines are confirmed, so a later
        // read may see that window (or a dormant draw it did not queue):
        // after preparation only a failure refuses.
        if let Some(source) = world.get::<SourceParticle>(draw) {
            if let ParticleReadiness::Failed(error) = &*source.readiness.lock().unwrap() {
                return Err(invalid(format!("particle source GPU: {error}")));
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
    let route = world
        .get::<crate::weather_fx::fixture::DirectorRoute>(draw)
        .map(|route| route.0.clone())
        .or_else(|| world.get::<crate::weather_fx::fixture::Played>(draw).map(|played| played.route().clone()))
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
    trace_owned(world);
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
    let mut target = ControlledSystem { system: &mut *system, stop_emitting: &mut clock.stop_emitting, route: &clock.route, ctx };
    let result = prepare_frame(&mut target, &mut times, time as f32);
    clock.times = times;
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
    ctx: &'a Context,
}

impl Controlled for ControlledSystem<'_> {
    fn system_time(&self) -> f32 {
        self.system.playback_head
    }

    fn restart(&mut self) -> Result<(), String> {
        crate::particle_runtime::director_restart(self.system, self.route, self.ctx)?;
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
            package: None,
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
