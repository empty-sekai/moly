//! `CutSceneView.HideEffects` and `RestoreEffects`: the view's
//! `_setHiddenEffectDataList` (`HiddenEffectData`
//! {Name, Restore}) on the particle systems of the focus object (the start
//! transform's object, the fixture the cut-scene plays at).
//!
//! - `HideEffects` (in `SetupInternal`): nothing without a list or a focus;
//!   otherwise, over the focus's `GetComponentsInChildren<ParticleSystem>()`
//!   (active in the hierarchy, hierarchy order), each system whose object's
//!   name is a row's `Name` is deactivated (`SetActive(false)`), and when
//!   the first row of that name has `Restore`, its `activeSelf` before is
//!   kept for the restore.
//! - `RestoreEffects` (in `EndAsync`): for each kept row, the focus's
//!   `GetComponentInChildren<ParticleSystem>(includeInactive: true)`, the
//!   first system of the whole hierarchy; only when its name is the row's
//!   `Name` is it set active again as it was. On a fixture whose first
//!   system is another one, the hidden system stays hidden.
//!
//! Which nodes carry a ParticleSystem is the fixture's particle document's
//! (`fixture-particles-v2`, its emitter nodes); the nodes' hierarchy order
//! and activity are the placed fixture's instance.

use std::collections::HashSet;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::{SetSourceActive, SourceInactive, SourceNodeActivity};
use serde_json::Value;

const PARTICLE_INDEX: &str = "moly://fixture-particles-v2/index.json";

/// `HiddenEffectData`.
#[derive(Clone, Debug)]
pub(super) struct HiddenEffect {
    name: String,
    restore: bool,
}

/// The view's list from its serialized fields.
pub(super) fn parse(fields: &Value) -> Vec<HiddenEffect> {
    fields["_setHiddenEffectDataList"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| HiddenEffect {
                    name: row["Name"].as_str().unwrap_or_default().to_owned(),
                    restore: row["Restore"].as_i64().unwrap_or(0) != 0
                        || row["Restore"].as_bool() == Some(true),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The list's state from the load to the restore.
#[derive(Default)]
pub(super) struct HiddenEffects {
    index: Option<Handle<JsonAsset>>,
    document: Option<Handle<JsonAsset>>,
    /// The focus's particle system nodes (the document's emitter paths).
    emitters: Option<HashSet<String>>,
    /// `_restoreHiddenFixtureDictionary`: the row's name and `activeSelf`.
    restore: Vec<(String, bool)>,
}

fn json(world: &World, handle: &Handle<JsonAsset>, what: &str) -> Result<Option<Value>, String> {
    if let LoadState::Failed(error) = world.resource::<AssetServer>().load_state(handle) {
        return Err(format!("{what} failed to load: {error}"));
    }
    let Some(asset) = world.resource::<Assets<JsonAsset>>().get(handle) else {
        return Ok(None);
    };
    serde_json::from_str(&asset.0)
        .map(Some)
        .map_err(|error| format!("{what}: {error}"))
}

/// Loads the focus fixture's particle document when the list is not
/// empty. `Ok(Some(reason))` while it loads.
pub(super) fn prepare(
    world: &mut World,
    state: &mut HiddenEffects,
    list: &[HiddenEffect],
    focus: Entity,
) -> Result<Option<String>, String> {
    if list.is_empty() || state.emitters.is_some() {
        return Ok(None);
    }
    let Some(package) = world
        .get::<crate::fixture_activity_state::FixtureActivityIdentity>(focus)
        .map(|identity| identity.model_package.clone())
    else {
        // No fixture model: no particle system to hide.
        state.emitters = Some(HashSet::new());
        return Ok(None);
    };
    let index = state
        .index
        .get_or_insert_with(|| world.resource::<AssetServer>().load(PARTICLE_INDEX))
        .clone();
    let Some(index) = json(world, &index, "fixture-particles-v2/index.json")? else {
        return Ok(Some("the fixture particle index is loading".into()));
    };
    let Some(file) = index["packages"][package.as_str()]["file"].as_str() else {
        info!("[cutscene-cast] HideEffects: {package} has no particle document: no particle system to hide");
        state.emitters = Some(HashSet::new());
        return Ok(None);
    };
    let document = state
        .document
        .get_or_insert_with(|| {
            world
                .resource::<AssetServer>()
                .load(format!("moly://fixture-particles-v2/{file}"))
        })
        .clone();
    let Some(document) = json(world, &document, file)? else {
        return Ok(Some(format!("the particle document {file} is loading")));
    };
    let emitters = document["emitters"]
        .as_array()
        .ok_or_else(|| format!("{file} has no emitters"))?
        .iter()
        .filter_map(|emitter| emitter["node"].as_str().map(str::to_owned))
        .collect();
    state.emitters = Some(emitters);
    Ok(None)
}

/// The focus's particle system nodes in hierarchy order (pre-order), with
/// their names: the named nodes' paths as the particle host builds them.
fn particle_nodes(
    world: &World,
    focus: Entity,
    emitters: &HashSet<String>,
) -> Vec<(Entity, String)> {
    fn walk(
        world: &World,
        entity: Entity,
        prefix: &mut Vec<String>,
        emitters: &HashSet<String>,
        out: &mut Vec<(Entity, String)>,
    ) {
        let named = world
            .get::<Name>(entity)
            .map(|name| name.as_str().to_owned());
        if let Some(name) = &named {
            prefix.push(name.clone());
            if emitters.contains(&prefix.join("/")) {
                out.push((entity, name.clone()));
            }
        }
        if let Some(children) = world.get::<Children>(entity) {
            for child in children.to_vec() {
                walk(world, child, prefix, emitters, out);
            }
        }
        if named.is_some() {
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(world, focus, &mut Vec::new(), emitters, &mut out);
    out
}

/// `HideEffects`.
pub(super) fn hide(
    world: &mut World,
    state: &mut HiddenEffects,
    list: &[HiddenEffect],
    focus: Entity,
) {
    if list.is_empty() {
        info!("[cutscene-cast] SetupInternal: HideEffects: the view's hidden effect list is empty");
        return;
    }
    let Some(emitters) = state.emitters.as_ref() else {
        error!("[cutscene-cast] SetupInternal: HideEffects: the focus's particle document was not read; nothing is hidden");
        return;
    };
    let systems = particle_nodes(world, focus, emitters);
    let mut hidden = Vec::new();
    for (entity, name) in systems {
        // GetComponentsInChildren without includeInactive.
        if world.get::<SourceInactive>(entity).is_some() {
            continue;
        }
        let Some(row) = list.iter().find(|row| row.name == name) else {
            continue;
        };
        let Some(active_self) = world
            .get::<SourceNodeActivity>(entity)
            .map(SourceNodeActivity::active_self)
        else {
            error!("[cutscene-cast] HideEffects: {name} ({entity:?}) has no source activity; it is not hidden");
            continue;
        };
        SetSourceActive {
            entity,
            active: false,
        }
        .apply(world);
        if row.restore && !state.restore.iter().any(|(kept, _)| *kept == row.name) {
            state.restore.push((row.name.clone(), active_self));
        }
        hidden.push(format!("{name} ({entity:?}, activeSelf {active_self})"));
    }
    info!(
        "[cutscene-cast] SetupInternal: HideEffects {:?}: SetActive(false) on [{}]; kept for the restore {:?}",
        list.iter().map(|row| (&row.name, row.restore)).collect::<Vec<_>>(),
        hidden.join(", "),
        state.restore
    );
}

/// `RestoreEffects`.
pub(super) fn restore(world: &mut World, state: &mut HiddenEffects, focus: Entity) {
    let kept = std::mem::take(&mut state.restore);
    if kept.is_empty() {
        info!("[cutscene-cast] RestoreEffects: nothing kept");
        return;
    }
    let Some(emitters) = state.emitters.as_ref() else {
        return;
    };
    // GetComponentInChildren(includeInactive: true): the first system.
    let first = particle_nodes(world, focus, emitters).into_iter().next();
    for (name, active_self) in kept {
        match &first {
            Some((entity, first_name)) if *first_name == name => {
                SetSourceActive {
                    entity: *entity,
                    active: active_self,
                }
                .apply(world);
                info!("[cutscene-cast] RestoreEffects: {name} ({entity:?}) SetActive({active_self})");
            }
            Some((entity, first_name)) => info!("[cutscene-cast] RestoreEffects: the focus's first particle system is {first_name} ({entity:?}), not {name}: {name} stays inactive (the source's GetComponentInChildren)"),
            None => info!("[cutscene-cast] RestoreEffects: the focus has no particle system: {name} stays inactive"),
        }
    }
}
