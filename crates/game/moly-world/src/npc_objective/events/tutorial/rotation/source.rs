//! Per-view controller membership and source clip lengths. No shared-union
//! existence check is allowed to stand in for NPCAvatarView.HasAnimation.

use super::kernel;
use crate::character::{CharacterPack, MotionDriver};
use crate::npc::CharacterUnitId;
use crate::npc_objective::events::{SourceAnimationClip, SourceAnimationMap};
use bevy::asset::{AssetId, LoadState};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewIdentity {
    unit: u32,
    rig: AssetId<JsonAsset>,
    model: AssetId<Gltf>,
    player: Entity,
}

pub(super) fn identity(world: &World, entity: Entity) -> Option<ViewIdentity> {
    let pack = world.get::<CharacterPack>(entity)?;
    Some(ViewIdentity {
        unit: world.get::<CharacterUnitId>(entity)?.0,
        rig: pack.rig.id(),
        model: pack.gltf.id(),
        player: world.get::<MotionDriver>(entity)?.player,
    })
}

impl ViewIdentity {
    pub(crate) fn matches(&self, world: &World, entity: Entity) -> bool {
        identity(world, entity).as_ref() == Some(self)
    }
}

#[derive(Component, Clone)]
pub(super) struct ActorSource {
    pub view: ViewIdentity,
    pub clips: Arc<SourceAnimationMap>,
    pub idle: String,
    pub uses_male: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ClipIdentity(String, String);

struct IndexedClip {
    source_name: String,
    binding: SourceAnimationClip,
}
struct Profile {
    male: bool,
    idle: String,
}

#[derive(Resource)]
struct SourceAssets {
    registry: Handle<JsonAsset>,
    index: Handle<JsonAsset>,
    parsed_registry: Option<Result<BTreeMap<u32, Profile>, String>>,
    parsed_index: Option<Result<BTreeMap<ClipIdentity, IndexedClip>, String>>,
}

pub(super) fn request_assets(world: &mut World) {
    if world.contains_resource::<SourceAssets>() {
        return;
    }
    let Some(server) = world.get_resource::<AssetServer>() else {
        return;
    };
    let registry = server.load(moly_assets::character_registry());
    let index = server.load("moly://motion-library.index.json");
    world.insert_resource(SourceAssets {
        registry,
        index,
        parsed_registry: None,
        parsed_index: None,
    });
}

fn text<'a>(
    world: &'a World,
    handle: &Handle<JsonAsset>,
    label: &str,
) -> Result<Option<&'a str>, String> {
    if let Some(server) = world.get_resource::<AssetServer>() {
        if let LoadState::Failed(reason) = server.load_state(handle) {
            return Err(format!("{label} failed to load: {reason}"));
        }
    }
    Ok(world
        .get_resource::<Assets<JsonAsset>>()
        .and_then(|assets| assets.get(handle))
        .map(|asset| asset.0.as_str()))
}

fn id(value: &Value) -> Result<ClipIdentity, String> {
    let file = value["file"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("animation identity has no serialized file")?;
    let path = value["pathId"]
        .as_str()
        .ok_or("animation identity has no signed pathId string")?;
    let number = path
        .parse::<i64>()
        .map_err(|_| "animation identity pathId is not signed i64")?;
    if number == 0 || number.to_string() != path {
        return Err("animation identity has a null or noncanonical pathId".into());
    }
    Ok(ClipIdentity(file.to_owned(), path.to_owned()))
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("source animation data lacks {key}"))
}

fn parse_index(raw: &str) -> Result<BTreeMap<ClipIdentity, IndexedClip>, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("motion source index: {e}"))?;
    let groups = value["clips"]
        .as_object()
        .ok_or("motion source index lacks clips")?;
    let mut index = BTreeMap::new();
    for group in groups.values() {
        let segments = group["segments"]
            .as_object()
            .ok_or("motion source index lacks segments")?;
        for segment in segments.values() {
            let identity = id(&segment["sourceClip"])?;
            let source_name = required_string(segment, "sourceClipName")?.to_owned();
            let exported_name = required_string(segment, "name")?.to_owned();
            let start = segment["sourceStartTime"]
                .as_f64()
                .ok_or("source clip lacks start time")? as f32;
            let stop = segment["sourceStopTime"]
                .as_f64()
                .ok_or("source clip lacks stop time")? as f32;
            let length = stop - start;
            if !start.is_finite() || !stop.is_finite() || !length.is_finite() || length < 0.0 {
                return Err("source clip has invalid authored time bounds".into());
            }
            if index
                .insert(
                    identity,
                    IndexedClip {
                        source_name,
                        binding: SourceAnimationClip {
                            exported_name,
                            length,
                        },
                    },
                )
                .is_some()
            {
                return Err("motion source index repeats a serialized clip identity".into());
            }
        }
    }
    Ok(index)
}

fn map_for_rig(
    rig: &Value,
    index: &BTreeMap<ClipIdentity, IndexedClip>,
    idle: &str,
) -> Result<SourceAnimationMap, String> {
    let controller = rig
        .get("animationController")
        .ok_or("actor rig has no animationController metadata; re-extract its source sidecar")?;
    if controller.is_null() {
        return Err("actor view has no source Animator controller".into());
    }
    if controller["schema"].as_str() != Some("moly-character-animation-map/1") {
        return Err("unsupported actor animationController schema".into());
    }
    id(&controller["animator"])?;
    id(&controller["controller"])?;
    let references = controller["clips"]
        .as_array()
        .ok_or("actor controller lacks clip references")?;
    let mut names = HashSet::new();
    let mut result = BTreeMap::new();
    for reference in references {
        let identity = id(reference)?;
        let clip = index
            .get(&identity)
            .ok_or("actor controller clip is absent from source index")?;
        if !names.insert(clip.source_name.as_str()) {
            return Err("actor controller repeats an original animation name".into());
        }
        // Validate the entire source view, but retain only families used by
        // this operation. Both calls have hasExitTime=true, so no unrelated
        // previous family's end clip can be requested through this map.
        let idle_clip = ["_S", "_L", "_E", "_O"]
            .into_iter()
            .any(|suffix| clip.source_name.strip_suffix(suffix) == Some(idle));
        if idle_clip || kernel::is_turn_clip(&clip.source_name) {
            result.insert(clip.source_name.clone(), clip.binding.clone());
        }
    }
    Ok(result)
}

fn parse_registry(raw: &str) -> Result<BTreeMap<u32, Profile>, String> {
    let registry: Value =
        serde_json::from_str(raw).map_err(|e| format!("character source registry: {e}"))?;
    let rows = registry["characters"]
        .as_object()
        .ok_or("source registry lacks characters")?;
    let mut result = BTreeMap::new();
    for key in rows.keys() {
        let unit = key
            .parse::<u32>()
            .map_err(|_| "source registry has invalid unit key")?;
        let (male, idle) = profile(&registry, unit)?;
        result.insert(unit, Profile { male, idle });
    }
    Ok(result)
}

fn profile(registry: &Value, unit: u32) -> Result<(bool, String), String> {
    let row = registry["characters"]
        .get(unit.to_string())
        .ok_or_else(|| format!("actor unit {unit} was not exported in characters.json"))?;
    if row["unitId"].as_u64() != Some(u64::from(unit)) {
        return Err("actor registry key and unitId disagree".into());
    }
    let character = row
        .get("gameCharacter")
        .ok_or("actor registry lacks gameCharacter source metadata")?;
    let is_male = if character.is_null() {
        false // An exported absent master row, not a loading fallback.
    } else {
        let id = character["id"]
            .as_i64()
            .ok_or("actor gameCharacter lacks id")?;
        if row["identity"]["gameCharacterId"].as_i64() != Some(id) {
            return Err("actor gender and character identity joins disagree".into());
        }
        character
            .get("gender")
            .ok_or("actor gameCharacter lacks raw gender")?
            .as_str()
            == Some("male")
    };
    Ok((
        is_male,
        required_string(&row["locomotion"], "idleMotion")?.to_owned(),
    ))
}

fn uses_male(male: bool, clips: &SourceAnimationMap) -> bool {
    male && kernel::MALE
        .iter()
        .flatten()
        .all(|family| clips.contains_key(&format!("{family}_S")))
}

fn validate_model(rig: &Value, unit: u32, model_file: &str) -> Result<(), String> {
    // AssetBundleNames: mdl_sd_1{unit:D2}_{category:D3}; extraction names the
    // default category's model sd_1{unit:D2}. This is formatting, not +100.
    let expected = format!("sd_1{unit:02}");
    let file = model_file.rsplit(['/', '\\']).next().unwrap_or(model_file);
    if rig["name"].as_str() != Some(expected.as_str()) || file != format!("{expected}.glb") {
        return Err("actor unit does not match its bound source model and rig".into());
    }
    if !rig["unitId"].is_null() && rig["unitId"].as_u64() != Some(u64::from(unit)) {
        return Err("actor unit disagrees with the rig's explicit unitId".into());
    }
    Ok(())
}

/// None means an actual asset is still loading. Invalid or absent producer
/// metadata is an error and never becomes a cached female/empty view.
pub(super) fn resolve(world: &mut World, entity: Entity) -> Result<Option<ActorSource>, String> {
    let view = identity(world, entity).ok_or("tutorial actor has no initialized view")?;
    if let Some(cached) = world.get::<ActorSource>(entity).filter(|c| c.view == view) {
        return Ok(Some(cached.clone()));
    }
    request_assets(world);
    let Some(assets) = world.get_resource::<SourceAssets>() else {
        return Ok(None);
    };
    if assets.parsed_registry.is_none() {
        let Some(raw) = text(world, &assets.registry, "character source registry")? else {
            return Ok(None);
        };
        let parsed = parse_registry(raw);
        world.resource_mut::<SourceAssets>().parsed_registry = Some(parsed);
    }
    let assets = world.resource::<SourceAssets>();
    if assets.parsed_index.is_none() {
        let Some(raw) = text(world, &assets.index, "motion source index")? else {
            return Ok(None);
        };
        let parsed = parse_index(raw);
        world.resource_mut::<SourceAssets>().parsed_index = Some(parsed);
    }
    // Parse borrowed asset text while the world is immutably borrowed. Only the
    // final compact actor map is installed afterward; no raw JSON is cloned.
    let assets = world.resource::<SourceAssets>();
    let registry = match assets.parsed_registry.as_ref() {
        Some(Ok(registry)) => registry,
        Some(Err(reason)) => return Err(reason.clone()),
        None => return Ok(None),
    };
    let index = match assets.parsed_index.as_ref() {
        Some(Ok(index)) => index,
        Some(Err(reason)) => return Err(reason.clone()),
        None => return Ok(None),
    };
    let pack = world
        .get::<CharacterPack>(entity)
        .ok_or("tutorial actor lost its character pack")?;
    let Some(raw) = text(world, &pack.rig, "actor rig")? else {
        return Ok(None);
    };
    let rig: Value = serde_json::from_str(raw).map_err(|e| format!("actor rig: {e}"))?;
    validate_model(&rig, view.unit, &pack.file)?;
    let profile = registry
        .get(&view.unit)
        .ok_or("actor unit was not exported in source registry")?;
    let clips = Arc::new(map_for_rig(&rig, index, &profile.idle)?);
    let idle = profile.idle.clone();
    let uses_male = uses_male(profile.male, &clips);
    let source = ActorSource {
        view,
        clips,
        idle,
        uses_male,
    };
    world.entity_mut(entity).insert(source.clone());
    Ok(Some(source))
}
