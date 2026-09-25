//! The harvest effects (`EffectManager` types 101 to 143), drawn from the
//! effect table (`EffectResourceData`, exported as the effect-resources
//! document): each row names the type, the bundle and its prefab, and the
//! pool size. This module takes the rows of the harvest types, resolves each
//! bundle to its exported prefab and particle document, and plays an effect
//! where the flow emits it.
//!
//! `Emit(type, position, key)` activates a pooled copy at the position and
//! plays its root particle system with its children (`ManagedEffect.Play`);
//! the copy returns to its pool when its systems end. Emit poses, read off
//! the views: the tree and stone hit effects (normal and boost) are moved
//! 0.2 m back toward the player and 0.35 m up and turned to face the player
//! (`LookRotation(-0.2 * dir)`); every other harvest emit is at the view's
//! position; 142 at the player's position.
//!
//! Named differences (as in the move's pooled effects): a played copy is
//! released and a fresh one instantiated for the next emit instead of being
//! returned to the pool; the pool size is recorded, not preallocated; an
//! emitter the particle host refuses is skipped with a WARN and the others
//! play; a missing prefab or document is one WARN and no effect.

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use super::{EffectHook, HarvestEffectHooks};
use crate::site_move::effects::{
    authored_renderers, prepare_skipping_refused, select_all, systems_finished, Prepared,
};
use crate::site_move::{InstanceReady, PendingInstance};

const TABLE: &str = "moly://effect-resources/effect-resources.json";
const OBJECT_INDEX: &str = "moly://site-harvest-object-effects/index.json";
const PARTICLE_INDEX: &str = "moly://fixture-particles-v2/index.json";
/// A copy that prepared nothing is released after this age (housekeeping).
const UNPREPARED_RELEASE_AGE: f32 = 30.0;

/// The harvest types of the table (hits, deletes, boosts).
fn is_harvest_type(kind: u16) -> bool {
    (100..=143).contains(&kind)
}

struct Pool {
    name: String,
    prefab: String,
    pool_size: u32,
    glb: Handle<Gltf>,
    doc: Handle<JsonAsset>,
    warned: bool,
}

struct Live {
    kind: u16,
    root: Entity,
    planned: bool,
    age: f32,
}

/// A played harvest effect's root (removed with the site).
#[derive(Component)]
pub(crate) struct HarvestEffectRoot;

#[derive(Resource, Default)]
pub(crate) struct HarvestEffects {
    table: Option<Handle<JsonAsset>>,
    index: Option<Handle<JsonAsset>>,
    particles: Option<Handle<JsonAsset>>,
    pools: HashMap<u16, Pool>,
    ready: bool,
    absent: bool,
    live: Vec<Live>,
    pub(crate) played: usize,
    pub(crate) skipped: usize,
}

pub(crate) fn load(mut effects: ResMut<HarvestEffects>, server: Res<AssetServer>) {
    effects.table = Some(server.load(bevy::asset::AssetPath::from(TABLE.to_owned())));
    effects.index = Some(server.load(bevy::asset::AssetPath::from(OBJECT_INDEX.to_owned())));
    effects.particles = Some(server.load(bevy::asset::AssetPath::from(PARTICLE_INDEX.to_owned())));
}

fn loaded(server: &AssetServer, handle: &Handle<JsonAsset>, path: &str) -> Option<bool> {
    match server.load_state(handle) {
        LoadState::Loaded => Some(true),
        LoadState::Failed(error) => {
            warn!("[harvest-effect] input absent, harvest effects are not drawn: {path}: {error}");
            Some(false)
        }
        _ => None,
    }
}

/// Resolve the table's harvest rows to prefab and document files.
fn build(effects: &mut HarvestEffects, world: &mut World) {
    if effects.ready || effects.absent {
        return;
    }
    let (Some(table), Some(index), Some(particles)) = (
        effects.table.clone(),
        effects.index.clone(),
        effects.particles.clone(),
    ) else {
        return;
    };
    let server = world.resource::<AssetServer>().clone();
    match (
        loaded(&server, &table, TABLE),
        loaded(&server, &index, OBJECT_INDEX),
        loaded(&server, &particles, PARTICLE_INDEX),
    ) {
        (Some(true), Some(true), Some(true)) => {}
        (Some(false), _, _) | (_, Some(false), _) | (_, _, Some(false)) => {
            effects.absent = true;
            return;
        }
        _ => return,
    }
    let json = world.resource::<Assets<JsonAsset>>();
    let (Some(table), Some(index), Some(particles)) =
        (json.get(&table), json.get(&index), json.get(&particles))
    else {
        return;
    };
    let particles: Value = serde_json::from_str(&particles.0)
        .unwrap_or_else(|error| panic!("{PARTICLE_INDEX}: not JSON: {error}"));
    let particle_docs = particles["packages"]
        .as_object()
        .unwrap_or_else(|| panic!("{PARTICLE_INDEX}: no packages"));
    let table: Value =
        serde_json::from_str(&table.0).unwrap_or_else(|error| panic!("{TABLE}: not JSON: {error}"));
    let index: Value = serde_json::from_str(&index.0)
        .unwrap_or_else(|error| panic!("{OBJECT_INDEX}: not JSON: {error}"));
    let objects = index["effects"]
        .as_object()
        .unwrap_or_else(|| panic!("{OBJECT_INDEX}: no effects"));
    let glb_of_package: HashMap<&str, &str> = objects
        .values()
        .filter_map(|row| Some((row["package"].as_str()?, row["glb"].as_str()?)))
        .collect();
    let rows = table["resources"]
        .as_array()
        .unwrap_or_else(|| panic!("{TABLE}: no resources"));
    let mut pools = HashMap::new();
    for row in rows {
        let kind = row["effectType"]
            .as_u64()
            .unwrap_or_else(|| panic!("{TABLE}: row without effectType")) as u16;
        if !is_harvest_type(kind) {
            continue;
        }
        let name = row["effectTypeName"].as_str().unwrap_or("").to_owned();
        let prefab = row["prefabName"]
            .as_str()
            .unwrap_or_else(|| panic!("{TABLE}: type {kind} without prefab"))
            .to_owned();
        let package = row["bundle"]["package"]
            .as_str()
            .unwrap_or_else(|| panic!("{TABLE}: type {kind} without a bundle package"))
            .to_owned();
        let glb = if let Some(glb) = glb_of_package.get(package.as_str()) {
            format!("moly://site-harvest-object-effects/{glb}")
        } else if package.contains("__harvest__action__") {
            format!("moly://site-action-effects/{prefab}/{prefab}.glb")
        } else {
            warn!(
                "[harvest-effect] type {kind} ({name}): bundle {package} has no exported prefab; not drawn"
            );
            continue;
        };
        // The particle document is requested only when the release lists it.
        let Some(file) = particle_docs
            .get(&package)
            .filter(|row| row["missing"] != true)
            .and_then(|row| row["file"].as_str())
        else {
            warn!(
                "[harvest-effect] type {kind} ({name}): the particle document of {package} is not in the release's particle index; not drawn"
            );
            continue;
        };
        let doc = format!("moly://fixture-particles-v2/{file}");
        let pool_size = row["poolSize"].as_u64().unwrap_or(0) as u32;
        pools.insert(
            kind,
            Pool {
                glb: server.load(bevy::asset::AssetPath::from(glb)),
                doc: server.load(bevy::asset::AssetPath::from(doc)),
                name,
                prefab,
                pool_size,
                warned: false,
            },
        );
    }
    let mut kinds: Vec<u16> = pools.keys().copied().collect();
    kinds.sort_unstable();
    info!(
        "[harvest-effect] effect table: {} harvest types {kinds:?} (prefabs and particle documents requested)",
        pools.len()
    );
    effects.pools = pools;
    effects.ready = true;
}

/// Exclusive: emit the flow's requests, prepare new copies, release ended
/// ones.
pub(crate) fn advance(world: &mut World) {
    world.resource_scope(|world, mut effects: Mut<HarvestEffects>| {
        build(&mut effects, world);
        let pending = std::mem::take(&mut world.resource_mut::<HarvestEffectHooks>().pending);
        world.resource_mut::<HarvestEffectHooks>().total += pending.len();
        for hook in pending {
            emit(&mut effects, world, hook);
        }
        let dt = world.resource::<Time>().delta_secs();
        prepare(&mut effects, world, dt);
    });
}

fn emit(effects: &mut HarvestEffects, world: &mut World, hook: EffectHook) {
    if effects.absent {
        effects.skipped += 1;
        return;
    }
    let Some(pool) = effects.pools.get_mut(&hook.kind) else {
        effects.skipped += 1;
        warn!(
            "[harvest-effect] EffectManager.Emit({}) at {:.2}: the type is not in the effect table (or its table is not loaded yet); not drawn",
            hook.kind, hook.position
        );
        return;
    };
    let server = world.resource::<AssetServer>();
    if server.load_state(&pool.glb).is_failed() || server.load_state(&pool.doc).is_failed() {
        if !pool.warned {
            warn!(
                "[harvest-effect] type {} ({}) prefab or particle document failed to load; not drawn",
                hook.kind, pool.name
            );
            pool.warned = true;
        }
        effects.skipped += 1;
        return;
    }
    let scene = world
        .resource::<Assets<Gltf>>()
        .get(&pool.glb)
        .and_then(|gltf| gltf.default_scene.clone());
    let Some(scene) = scene else {
        effects.skipped += 1;
        warn!(
            "[harvest-effect] type {} ({}) emitted before its prefab loaded; not drawn",
            hook.kind, pool.name
        );
        return;
    };
    let root = world
        .spawn((
            SceneRoot(scene),
            Transform::from_translation(hook.position).with_rotation(hook.rotation),
            Visibility::Inherited,
            PendingInstance,
            HarvestEffectRoot,
            Name::new(format!("harvest effect {}", pool.prefab)),
        ))
        .id();
    effects.played += 1;
    info!(
        "[harvest-effect] EffectManager.Emit({}) {} ({}, pool size {}) at ({:.2}, {:.2}, {:.2}) yaw {:.1} deg",
        hook.kind,
        pool.name,
        pool.prefab,
        pool.pool_size,
        hook.position.x,
        hook.position.y,
        hook.position.z,
        hook.rotation.to_euler(EulerRot::YXZ).0.to_degrees()
    );
    effects.live.push(Live {
        kind: hook.kind,
        root,
        planned: false,
        age: 0.0,
    });
}

/// `ManagedEffect.Play`: the root system with its children, every emitter
/// selected, as the move's explicitly played effect.
fn prepare(effects: &mut HarvestEffects, world: &mut World, dt: f32) {
    for index in 0..effects.live.len() {
        effects.live[index].age += dt;
        let (kind, root, planned) = {
            let live = &effects.live[index];
            (live.kind, live.root, live.planned)
        };
        if planned || world.get::<InstanceReady>(root).is_none() {
            continue;
        }
        let Some(pool) = effects.pools.get(&kind) else {
            effects.live[index].planned = true;
            continue;
        };
        let doc = world
            .resource::<Assets<JsonAsset>>()
            .get(&pool.doc)
            .map(|json| json.0.clone());
        let Some(doc) = doc else {
            continue;
        };
        let doc: Value = match serde_json::from_str(&doc) {
            Ok(doc) => doc,
            Err(error) => {
                warn!("[harvest-effect] type {kind} particle document unreadable: {error}");
                effects.live[index].planned = true;
                continue;
            }
        };
        let paths = crate::site_move::node_paths(world, root);
        let mut selected = select_all(&doc, &paths);
        let label = format!("harvest {kind}");
        match prepare_skipping_refused(world, root, &doc, &mut selected, &label) {
            Prepared::Ready(draws) => {
                let authored = authored_renderers(world, &doc, &draws);
                let mut on = 0;
                for &(draw, enabled) in &authored {
                    if let Some(mut source) =
                        world.get_mut::<crate::source_particle::SourceParticle>(draw)
                    {
                        source.enabled = enabled;
                        on += usize::from(enabled);
                    }
                }
                info!(
                    "[harvest-effect] type {kind} plays: {} systems prepared, {on} renderers on as authored",
                    draws.len()
                );
                effects.live[index].planned = true;
            }
            Prepared::Pending => {}
            Prepared::Refused(error) => {
                warn!("[harvest-effect] type {kind} particles refused: {error}");
                effects.live[index].planned = true;
            }
        }
    }
    let mut finished = Vec::new();
    effects.live.retain(|live| {
        if !live.planned {
            // Its particle assets never became ready (product housekeeping).
            if live.age >= UNPREPARED_RELEASE_AGE {
                warn!(
                    "[harvest-effect] type {} copy released after {:.1} s: its particles never became ready to play",
                    live.kind, live.age
                );
                finished.push(live.root);
                return false;
            }
            return true;
        }
        let done = match systems_finished(world, live.root) {
            Some(done) => done,
            // Nothing installed (every emitter refused).
            None => live.age >= UNPREPARED_RELEASE_AGE,
        };
        if done {
            finished.push(live.root);
        }
        !done
    });
    for root in finished {
        if let Ok(entity) = world.get_entity_mut(root) {
            entity.despawn();
        }
    }
}

/// Queued by the site change.
pub(crate) fn clear_for_site_change(world: &mut World) {
    world.resource_mut::<HarvestEffects>().live.clear();
}
