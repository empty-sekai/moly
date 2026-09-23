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
    particle_runtime::{Context, Rng, Runtime, simulate},
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

/// Presence suppresses wall-clock playOnAwake advancement, including when the
/// selected clip is currently outside its interval (requested == None).
#[derive(Component)]
pub(crate) struct DirectorClock {
    requested: Option<f64>,
    previous: Option<f64>,
    seed: u32,
    applied_seed: Option<u32>,
    source_seed: Option<u32>,
    restart_pending: bool,
}

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

fn within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn inventory(doc: &Value, root: &str, settings: &ControlSettings) -> Result<(), TimelineFailure> {
    let nodes = doc["nodes"]
        .as_array()
        .ok_or_else(|| invalid("source node inventory is missing"))?;
    if !nodes.iter().any(|node| node["node"].as_str() == Some(root)) {
        return Err(invalid(
            "bound particle root has no source component inventory",
        ));
    }
    for node in nodes {
        let path = node["node"]
            .as_str()
            .ok_or_else(|| invalid("source node path is missing"))?;
        if !within(path, root) {
            continue;
        }
        let check_director =
            settings.update_director && (settings.search_hierarchy || path == root);
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
            if settings.update_itime_control && class == "MonoBehaviour" {
                return Err(invalid(format!(
                    "{path}: ITimeControl script capability is unresolved"
                )));
            }
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
    let mut by_path = HashMap::new();
    for (index, particle) in particles.iter().enumerate() {
        let path = particle["node"]
            .as_str()
            .ok_or_else(|| invalid("source particle path is missing"))?;
        if by_path.insert(path, index).is_some() {
            return Err(invalid(format!(
                "ambiguous source particle path requires exact object identity: {path}"
            )));
        }
    }
    let mut paths = Vec::new();
    collect(world, fixture, "", &mut paths);
    let mut instance_paths = HashSet::new();
    for (_, path) in &paths {
        if by_path.contains_key(path.as_str()) && !instance_paths.insert(path) {
            return Err(invalid(format!(
                "ambiguous particle instance path requires exact object identity: {path}"
            )));
        }
    }
    // Game BindEffect uses the first source-order ParticleSystem name inside
    // this fixture instance. A renderer or another fixture is not a candidate.
    let (root, root_path) = paths
        .iter()
        .find(|(_, path)| {
            path.rsplit('/').next() == Some(bind_name) && by_path.contains_key(path.as_str())
        })
        .cloned()
        .ok_or_else(|| invalid(format!("fixture particle binding not found: {bind_name}")))?;
    inventory(&doc, &root_path, settings)?;
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
    for node in doc["nodes"].as_array().expect("validated node inventory") {
        let Some(path) = node["node"]
            .as_str()
            .filter(|path| within(path, &root_path))
        else {
            continue;
        };
        let classes = node["componentClasses"]
            .as_array()
            .ok_or_else(|| invalid(format!("{path}: controlled component inventory is missing")))?;
        if classes
            .iter()
            .any(|class| class.as_str() == Some("ParticleSystem"))
            && (!by_path.contains_key(path) || !paths.iter().any(|(_, p)| p == path))
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
    for (anchor, path) in paths.iter().filter(|(_, path)| within(path, &root_path)) {
        let Some(&ordinal) = by_path.get(path.as_str()) else {
            continue;
        };
        let particle = &particles[ordinal];
        if let Some(error) = particle["systemError"].as_str() {
            return Err(invalid(format!("{path}: {error}")));
        }
        let modules = ParticleSourceModules::from_system(&particle["system"]).map_err(invalid)?;
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
    let created = crate::weather_fx::fixture::prepare_control(world, root, &doc, &new)
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
            world.entity_mut(draw).insert(DirectorClock {
                requested: None,
                previous: None,
                seed: settings.random_seed,
                applied_seed: None,
                source_seed: seeds[&anchor],
                restart_pending: true,
            });
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
            world.entity_mut(draw).insert(DirectorClock {
                requested: None,
                previous: None,
                seed,
                applied_seed: None,
                source_seed: binding.seeds[&draw],
                restart_pending: true,
            });
        }
        let mut clock = world
            .get_mut::<DirectorClock>(draw)
            .ok_or_else(|| invalid("particle Director clock was released"))?;
        clock.requested = time;
        clock.seed = seed;
        // A new playable may replace the old one in the same Update. Preserve
        // its restart even when None is immediately overwritten by Some.
        clock.restart_pending |= time.is_none();
    }
    Ok(())
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
        // ParticleControlPlayable changes only an auto seed at initialization;
        // Stop/Destroy do not replace that configured seed with a constant.
        let seed = world.get::<DirectorClock>(draw)
            .map(|clock| clock.applied_seed.or(clock.source_seed).unwrap_or(clock.seed))
            .or_else(|| binding.seeds.get(&draw).copied().flatten());
        let mesh = if let Some(mut live) = world.get_mut::<FixtureParticleLive>(draw) {
            clear(&mut live.0);
            if let Some(seed) = seed {
                live.0.rng = Rng(seed as u64);
            } // No Director/authored seed was applied: keep the existing stream.
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
            clock.previous = None;
            clock.restart_pending = true;
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

fn reset(system: &mut Runtime, seed: u32) {
    clear(system);
    system.rng = Rng(seed as u64);
}

fn clear(system: &mut Runtime) {
    system.pool.clear();
    system.side.clear();
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

pub(crate) fn advance(
    system: &mut Runtime,
    clock: &mut DirectorClock,
    ctx: &Context,
    inactive: bool,
) {
    let Some(target) = clock
        .requested
        .filter(|_| !inactive)
        .map(|time| time as f32 as f64)
    else {
        if clock.previous.is_some() || !system.pool.is_empty() {
            reset(system, clock.seed);
        }
        clock.previous = None;
        clock.restart_pending = true;
        return;
    };
    let seed = clock.source_seed.unwrap_or(clock.seed);
    let restart = clock.restart_pending
        || clock.previous.is_none_or(|previous| previous > target)
        || clock.applied_seed != Some(seed);
    let mut remaining = if restart {
        reset(system, seed);
        target
    } else {
        target - clock.previous.unwrap()
    } * system.emitter.simulation_speed as f64;
    // ParticleControlPlayable casts playable time to float and uses variable
    // steps capped by Time.maximumDeltaTime (fixedTimeStep=false). CN 6.0.0
    // APK data.unity3d/globalgamemanagers TimeManager pathId=8 records
    // Maximum Allowed Timestep = 0.3333333432674408, Unity 2022.3.62f3.
    const MAX_STEP: f64 = (1.0f32 / 3.0f32) as f64;
    while remaining > 0.0 {
        let step = remaining.min(MAX_STEP);
        simulate(system, step as f32, ctx);
        remaining -= step;
    }
    clock.previous = Some(target);
    clock.applied_seed = Some(seed);
    clock.restart_pending = false;
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
            previous: None,
            seed: 42,
            applied_seed: None,
            source_seed: None,
            restart_pending: true,
        }
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
            {"node":"fx", "componentClasses":["CanvasRenderer","ParticleSystem","ParticleSystemRenderer"]},
            {"node":"fx/root/steam", "componentClasses":["ParticleSystem","ParticleSystemRenderer"]}]});
        assert!(inventory(&doc, "fx", &settings()).is_ok());
        assert!(inventory(&doc, "absent", &settings()).is_err());
        doc["nodes"][1]["componentClasses"] = Value::Null;
        assert!(inventory(&doc, "fx", &settings()).is_err());
        doc["nodes"][1]["componentClasses"] = json!(["MonoBehaviour"]);
        assert!(inventory(&doc, "fx", &settings()).is_err());
        doc["nodes"][1]["componentClasses"] = json!(["PlayableDirector"]);
        assert!(inventory(&doc, "fx", &settings()).is_ok());
        let mut hierarchy = settings();
        hierarchy.search_hierarchy = true;
        assert!(inventory(&doc, "fx", &hierarchy).is_err());
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
    fn director_time_has_no_prewarm_or_wall_clock_and_restarts_on_rewind() {
        let mut system = crate::particle_runtime::test_support::runtime();
        let mut clock = clock();
        advance(&mut system, &mut clock, &context(), false);
        assert!(system.pool.is_empty());
        clock.requested = Some(0.0);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!(system.born_total, 0); // source prewarm=true is not playOnAwake
        clock.requested = Some(0.1);
        advance(&mut system, &mut clock, &context(), false);
        let born = system.born_total;
        let rng = system.rng.0;
        assert!(born > 0);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, system.rng.0), (born, rng));
        clock.requested = Some(0.0);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!(system.born_total, 0);
        clock.requested = Some(0.1);
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, system.rng.0), (born, rng));
        advance(&mut system, &mut clock, &context(), true);
        assert!(system.pool.is_empty());
        advance(&mut system, &mut clock, &context(), false);
        assert_eq!((system.born_total, system.rng.0), (born, rng));
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
                FixtureParticleLive(crate::particle_runtime::test_support::runtime()),
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
        world.get_mut::<DirectorClock>(draw).unwrap().source_seed = Some(7);
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
        sample(&mut world, &binding, None, 42).unwrap();
        sample(&mut world, &binding, Some(0.2), 99).unwrap();
        let mut clock = world.entity_mut(draw).take::<DirectorClock>().unwrap();
        assert!(clock.restart_pending);
        advance(
            &mut world.get_mut::<FixtureParticleLive>(draw).unwrap().0,
            &mut clock,
            &context(),
            false,
        );
        assert_eq!(clock.applied_seed, Some(7));
        assert!(world.get::<FixtureParticleLive>(draw).unwrap().0.born_total > born);
        release(&mut world, &binding);
        // With no clock remaining, the authored non-auto seed still survives.
        assert_eq!(world.get::<FixtureParticleLive>(draw).unwrap().0.rng.0, 7);
        // An already-applied auto seed wins over a later pending request seed.
        clock.applied_seed = Some(23);
        clock.source_seed = None;
        clock.seed = 99;
        world.entity_mut(draw).insert(clock);
        release(&mut world, &binding);
        assert_eq!(world.get::<FixtureParticleLive>(draw).unwrap().0.rng.0, 23);
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

    #[test]
    fn instance_traversal_and_path_boundaries_do_not_cross_fixtures() {
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
        assert!(within("model/fx/root", "model/fx"));
        assert!(!within("model/fx-other", "model/fx"));
    }
}
