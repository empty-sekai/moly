//! One source director clock for actual furniture/actor animation, facial
//! presets and source-qualified SE. Admission, slots, navigation and final
//! actor-state restoration belong to the activity owner.
//!
//! A renderable sampled pose is distinct from native humanoid equivalence.
//! Foot IK, start offsets, track matching and mixer differences remain
//! readable through `coverage`; they are never inferred from clip names.

mod clock;
mod effects;
mod source;

pub(crate) use source::{
    AnimationPlayableSettings, BlendCurve, ClipTarget, ControlSettings, CutScenePayload,
    ExpansionEffect, ExposedSource, InfiniteClip, SourceAssetId, TimelineClip, TimelineClipKey,
    TimelineDefinition, TimelinePackage, TimelinePayload, TimelineTrack,
};

use crate::{
    alone_action_runtime::{apply_eye, apply_mouth_pattern},
    audio::{BusVolume, Routing, SeClass, VolumeBus},
    character_material::{CharacterMaterial, ToonMaterials},
    fixture_activity_state::FixtureActivityOwner,
};
use bevy::gltf::Gltf;
use bevy::{
    animation::{
        graph::{AnimationGraph, AnimationNodeIndex, AnimationNodeType},
        AnimatedBy, AnimationClip, AnimationTargetId, RepeatAnimation,
    },
    asset::{AssetId, AssetPath},
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume},
    ecs::change_detection::Tick,
    prelude::*,
};
use moly_assets::json::JsonAsset;
use moly_law::facial::LipPattern;
use serde_json::Value;
use source::{array, asset, finite, flag, invalid, string};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub(crate) struct TimelineFailure {
    pub message: String,
    /// True only for a requested asset that is still in flight. Structural,
    /// source-identity and failed-load errors must not consume a timeout budget.
    pub retryable: bool,
}
impl TimelineFailure {
    fn loading(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}
impl std::fmt::Display for TimelineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TimelineFailure {}

#[derive(Clone, Debug)]
pub(crate) struct SourceAnimationEvidence {
    pub package: String,
    pub clip_name: String,
    pub asset: SourceAssetId,
    pub start_time: f64,
    pub stop_time: f64,
    pub looping: bool,
}

impl SourceAnimationEvidence {
    /// Furniture Transform animations and actor libraries share the formal
    /// resolved target record. Callers need no second timing/loop parser.
    pub(crate) fn from_clip_target(target: &ClipTarget) -> Result<Self, TimelineFailure> {
        let row = &target.source_record;
        if !array(row, "sourceMetadataMissing")?.is_empty() {
            return Err(invalid("formal source clip metadata is incomplete"));
        }
        Ok(Self {
            package: target.target_package.clone(),
            clip_name: target.clip_name.clone(),
            asset: target
                .source_clip
                .clone()
                .ok_or_else(|| invalid("formal source clip identity missing"))?,
            start_time: finite(row, "sourceStartTime")?,
            stop_time: finite(row, "sourceStopTime")?,
            looping: flag(row, "sourceLoopTime")?,
        })
    }
    /// Only the source metadata in the exported segment is accepted. Legacy
    /// suffix-derived `loop` and sampled-tail `duration` are not substitutes.
    pub(crate) fn from_index_entry(entry: &Value, package: &str) -> Result<Self, TimelineFailure> {
        if !array(entry, "sourceMetadataMissing")?.is_empty() {
            return Err(invalid("source AnimationClip metadata is incomplete"));
        }
        Ok(Self {
            package: package.into(),
            clip_name: entry
                .get("sourceClipName")
                .and_then(Value::as_str)
                .unwrap_or(string(entry, "name")?)
                .into(),
            asset: asset(&entry["sourceClip"])?,
            start_time: finite(entry, "sourceStartTime")?,
            stop_time: finite(entry, "sourceStopTime")?,
            looping: flag(entry, "sourceLoopTime")?,
        })
    }
}

#[derive(Resource, Default)]
pub(crate) struct TimelineAssetLoads {
    json: crate::asset_cache::AssetCache<Handle<JsonAsset>, 96>,
    // One successful parse per currently observed source text. This caches
    // metadata syntax only, never a prepared actor/fixture binding or its
    // liveness verdict. The original text makes invalidation collision-free.
    //
    // 第一格是「校验这条缓存时 `Assets<JsonAsset>` 的 last_changed」。代没变
    // 就说明期间没有任何 json 资产被动过，这条缓存必然仍成立——省掉一次
    // 对整份文档的逐字节比对。代不同时走原来的比对，判定结果不变。
    parsed_json: crate::asset_cache::AssetCache<(Option<Tick>, String, Arc<Value>), 96>,
    gltf: crate::asset_cache::AssetCache<Handle<Gltf>, 24>,
    // Lookup tables derived from one parsed catalogue or library index. An
    // entry is reused only while `load_json` still returns that same parsed
    // document, so a replaced text always gets tables built from itself.
    routes: crate::asset_cache::AssetCache<(Arc<Value>, Arc<CatalogRoutes>), 96>,
    segments: crate::asset_cache::AssetCache<(Arc<Value>, Arc<IndexSegments>), 96>,
}

impl TimelineAssetLoads {
    pub(crate) fn cache_counts(&self) -> Value {
        serde_json::json!({"json":self.json.len(),"documents":self.parsed_json.len(),"gltf":self.gltf.len(),
            "routes":self.routes.len(),"segments":self.segments.len()})
    }

    pub(crate) fn sweep_lookup_caches(&mut self) -> usize {
        self.json.sweep_unused()
            + self.parsed_json.sweep_unused()
            + self.gltf.sweep_unused()
            + self.routes.sweep_unused()
            + self.segments.sweep_unused()
    }
}

const ACTOR_ROOT: &str = "actor-animations/";
const ACTOR_CATALOG: &str = "actor-animations/index.json";

/// The catalogue's `clipBindings` rows grouped by the (unitId, sourceClip)
/// pair a character clip is resolved by, each group in document order. A row
/// whose unit is not an unsigned integer or whose source identity does not
/// parse equals no request, so it is not indexed. A missing array is kept as
/// the error the resolution reports at the point it first needs a route.
struct CatalogRoutes(
    Result<HashMap<(u64, SourceAssetId), Vec<usize>>, TimelineFailure>,
);

impl CatalogRoutes {
    fn build(catalog: &Value) -> Self {
        Self(array(catalog, "clipBindings").map(|rows| {
            let mut routes: HashMap<(u64, SourceAssetId), Vec<usize>> = HashMap::new();
            for (index, row) in rows.iter().enumerate() {
                if let (Some(unit), Ok(source)) =
                    (row["unitId"].as_u64(), asset(&row["sourceClip"]))
                {
                    routes.entry((unit, source)).or_default().push(index);
                }
            }
            routes
        }))
    }

    /// Indices of exactly the rows with this unit and this source clip.
    fn rows(&self, unit: u64, clip: &SourceAssetId) -> Result<&[usize], TimelineFailure> {
        let routes = self.0.as_ref().map_err(Clone::clone)?;
        Ok(routes
            .get(&(unit, clip.clone()))
            .map_or(&[][..], Vec::as_slice))
    }
}

/// A library index's segments grouped by (exported name, sourceClip), each
/// group as `(clip group, segment)` keys in document order. Entries without a
/// string name or a parseable source identity equal no request. A missing
/// `clips` object is kept as the error reported where segments are needed.
struct IndexSegments(
    Result<HashMap<(String, SourceAssetId), Vec<(String, String)>>, TimelineFailure>,
);

impl IndexSegments {
    fn build(index: &Value) -> Self {
        Self(
            index["clips"]
                .as_object()
                .ok_or_else(|| invalid("actor index clips missing"))
                .map(|groups| {
                    let mut segments: HashMap<(String, SourceAssetId), Vec<(String, String)>> =
                        HashMap::new();
                    for (group_key, group) in groups {
                        let Some(entries) = group["segments"].as_object() else {
                            continue;
                        };
                        for (segment_key, entry) in entries {
                            if let (Some(name), Ok(source)) =
                                (entry["name"].as_str(), asset(&entry["sourceClip"]))
                            {
                                segments
                                    .entry((name.to_owned(), source))
                                    .or_default()
                                    .push((group_key.clone(), segment_key.clone()));
                            }
                        }
                    }
                    segments
                }),
        )
    }

    /// Exactly the segment entries with this exported name and source clip.
    fn entries<'a>(
        &self,
        index: &'a Value,
        name: &str,
        clip: &SourceAssetId,
    ) -> Result<Vec<&'a Value>, TimelineFailure> {
        let segments = self.0.as_ref().map_err(Clone::clone)?;
        Ok(segments
            .get(&(name.to_owned(), clip.clone()))
            .map_or_else(Vec::new, |keys| {
                keys.iter()
                    .map(|(group, segment)| &index["clips"][group]["segments"][segment])
                    .collect()
            }))
    }
}

/// The tables built from `document`, reused while the same parse is current.
fn derived<T>(
    cache: &mut crate::asset_cache::AssetCache<(Arc<Value>, Arc<T>), 96>,
    path: &str,
    document: &Arc<Value>,
    build: impl FnOnce(&Value) -> T,
) -> Arc<T> {
    if let Some((cached, tables)) = cache.get(path) {
        if Arc::ptr_eq(cached, document) {
            return tables.clone();
        }
    }
    let tables = Arc::new(build(&**document));
    cache.insert(path.to_owned(), (document.clone(), tables.clone()));
    tables
}

/// Resolve the formal per-actor library into the actual player's graph. This
/// preparation function loads assets only; it never spawns the export-only
/// reference skeleton and never starts an animation. Missing assets remain
/// preparation errors until the same call can resolve the complete set.
pub(crate) fn prepare_actor_animation_bindings(
    world: &mut World,
    request: &mut StartTimeline,
    unit_id: u32,
    animator: Entity,
    graph: Handle<AnimationGraph>,
) -> Result<(), TimelineFailure> {
    let actor = request.owner.activity.actor;
    prepare_actor_tracks(world, request, unit_id, actor, animator, graph, false)
}

/// The authored multi-character directors bind their actor tracks by exact
/// unit-id names (e.g. "14", "15"), not by one shared CharacterAnimator slot.
/// The owner resolves that source name to an admitted actor exactly once.
pub(crate) fn prepare_cast_actor_bindings(
    world: &mut World,
    request: &mut StartTimeline,
    unit_id: u32,
    actor: Entity,
    animator: Entity,
    graph: Handle<AnimationGraph>,
) -> Result<(), TimelineFailure> {
    if world
        .get::<crate::npc::CharacterUnitId>(actor)
        .map(|id| id.0)
        != Some(unit_id)
    {
        return Err(invalid(
            "source actor track unit differs from admitted actor",
        ));
    }
    prepare_actor_tracks(world, request, unit_id, actor, animator, graph, true)
}

/// The player's own fixture timelines keep their CharacterAnimator clips in
/// one avatar-rig file per timeline package, listed in this manifest.
const AVATAR_CLIP_ROOT: &str = "fixture-timeline/avatar-clips/";
const AVATAR_CLIP_MANIFEST: &str = "fixture-timeline/avatar-clips/manifest.json";

fn require_avatar_clip_path(path: &str) -> Result<(), TimelineFailure> {
    require_rooted(
        path,
        AVATAR_CLIP_ROOT,
        "avatar clip path is not asset-root-relative",
    )
}

/// The CharacterAnimator tracks of a definition and their animation clips.
fn character_clips(
    definition: &TimelineDefinition,
) -> Result<(Vec<SourceAssetId>, Vec<(TimelineClipKey, ClipTarget)>), TimelineFailure> {
    let mut tracks = Vec::new();
    let mut clips = Vec::new();
    for track in &definition.tracks {
        if track.class != "AnimationTrack" || track.name != "CharacterAnimator" {
            continue;
        }
        tracks.push(track.identity.clone());
        for clip in &track.clips {
            let TimelinePayload::Animation { target, .. } = &clip.payload else {
                return Err(invalid(
                    "character animation track has another payload type",
                ));
            };
            clips.push((clip.key.clone(), target.clone()));
        }
    }
    Ok((tracks, clips))
}

/// `PlayerFixtureTimelineView.BindPlayer`: the timeline's CharacterAnimator
/// stream plays on the player avatar's own animator. The source clips of
/// that stream ship in the fixture bundles and bind the avatar skeleton; the
/// extractor exports them, for each timeline package, into one file against
/// that skeleton. A clip is taken from that file by its name and its source
/// identity. A clip the file lacks refuses the timeline, by name.
pub(crate) fn prepare_player_avatar_bindings(
    world: &mut World,
    request: &mut StartTimeline,
    animator: Entity,
    graph: Handle<AnimationGraph>,
) -> Result<(), TimelineFailure> {
    let prefab = request.definition.prefab.clone();
    let package = request.definition.package.clone();
    let (tracks, clips) = character_clips(&request.definition)?;
    if !clips.is_empty() {
        world.init_resource::<TimelineAssetLoads>();
        let server = world.resource::<AssetServer>().clone();
        let manifest = load_json(
            world,
            &server,
            AVATAR_CLIP_MANIFEST,
            require_avatar_clip_path,
        )?;
        if manifest["version"].as_u64() != Some(1) {
            return Err(invalid("unsupported avatar clip manifest"));
        }
        let entry = manifest["packages"].get(package.as_str()).ok_or_else(|| {
            invalid(format!(
                "timeline {prefab}: package {package} has no avatar clip file"
            ))
        })?;
        let file = |key: &str| -> Result<String, TimelineFailure> {
            let name = string(entry, key)?;
            if name.contains('/') {
                return Err(invalid("avatar clip manifest names a nested file"));
            }
            let path = format!("{AVATAR_CLIP_ROOT}{name}");
            require_avatar_clip_path(&path)?;
            Ok(path)
        };
        let glb = file("glb")?;
        let index_path = file("index")?;
        let index = load_json(world, &server, &index_path, require_avatar_clip_path)?;
        if index["version"].as_u64() != Some(1)
            || index["package"].as_str() != Some(package.as_str())
        {
            return Err(invalid(format!(
                "avatar clip index {index_path} does not describe {package}"
            )));
        }
        // The loader keys every curve by the names on its path from the
        // file's scene root, so that root must be the animator's own node.
        let root = string(&index, "bindingRootName")?;
        if world.get::<Name>(animator).map(Name::as_str) != Some(root) {
            return Err(invalid(format!(
                "avatar clip file {glb} binds root {root}, not the player's animator"
            )));
        }
        let handle = {
            let mut loads = world.resource_mut::<TimelineAssetLoads>();
            loads
                .gltf
                .get_or_insert_with(&glb, || server.load(format!("moly://{glb}")))
                .clone()
        };
        if let bevy::asset::LoadState::Failed(error) = server.load_state(&handle) {
            return Err(invalid(format!(
                "avatar clip file failed to load: {glb}: {error}"
            )));
        }
        let rows = array(&index, "clips")?;
        let gltf = world
            .resource::<Assets<Gltf>>()
            .get(&handle)
            .ok_or_else(|| {
                TimelineFailure::loading(format!("avatar clip file still loading: {glb}"))
            })?;
        let mut prepared = Vec::new();
        for (key, target) in &clips {
            let expected = SourceAnimationEvidence::from_clip_target(target)?;
            let found: Vec<&Value> = rows
                .iter()
                .filter(|row| {
                    row["name"].as_str() == Some(target.clip_name.as_str())
                        && asset(&row["sourceClip"]).ok().as_ref() == Some(&expected.asset)
                })
                .collect();
            let [row] = found.as_slice() else {
                return Err(invalid(format!(
                    "timeline {prefab}: clip {} is {} in its avatar clip file {glb}",
                    target.clip_name,
                    if found.is_empty() {
                        "absent"
                    } else {
                        "duplicated"
                    }
                )));
            };
            if row["sourcePackage"].as_str() != Some(target.target_package.as_str()) {
                return Err(invalid(format!(
                    "timeline {prefab}: clip {} in {glb} comes from another source package",
                    target.clip_name
                )));
            }
            let source = SourceAnimationEvidence::from_index_entry(row, &target.target_package)?;
            if source.asset != expected.asset
                || source.clip_name != expected.clip_name
                || source.start_time != expected.start_time
                || source.stop_time != expected.stop_time
                || source.looping != expected.looping
            {
                return Err(invalid(format!(
                    "timeline {prefab}: clip {} in {glb} disagrees with the timeline's source clip",
                    target.clip_name
                )));
            }
            let animation = gltf
                .named_animations
                .get(target.clip_name.as_str())
                .cloned()
                .ok_or_else(|| {
                    invalid(format!(
                        "timeline {prefab}: clip {} is listed but not in {glb}",
                        target.clip_name
                    ))
                })?;
            prepared.push((
                key.clone(),
                TimelineAnimationBinding {
                    animator,
                    graph: graph.clone(),
                    clip: animation,
                    source,
                    coverage: AnimationCoverage::sampled_pose(),
                },
            ));
        }
        for (key, binding) in prepared {
            request.bindings.animations.insert(key, binding);
        }
    }
    let actor = request.owner.activity.actor;
    for identity in tracks {
        request.bindings.actors.insert(identity, actor);
    }
    Ok(())
}

/// `PlayerAvatarItemTimelineView.BindPlayer`: a step item's CharacterAnimator
/// stream plays clips of the avatar's own motion group, taken from the group
/// by name. The group's own length and loop flag of each clip must agree
/// with the timeline's clip. A clip the group lacks refuses the timeline, by
/// name. Returns each bound clip that carries animation events, with their
/// count; the runner does not dispatch them.
pub(crate) fn prepare_player_group_bindings(
    request: &mut StartTimeline,
    animator: Entity,
    graph: Handle<AnimationGraph>,
    group: &crate::player_avatar::BodyClips,
) -> Result<Vec<(String, usize)>, TimelineFailure> {
    let prefab = request.definition.prefab.clone();
    let (tracks, clips) = character_clips(&request.definition)?;
    let mut prepared = Vec::new();
    let mut with_events = Vec::new();
    for (key, target) in &clips {
        let expected = SourceAnimationEvidence::from_clip_target(target)?;
        let clip = group.get(&target.clip_name).ok_or_else(|| {
            invalid(format!(
                "timeline {prefab}: clip {} is not in the avatar motion group",
                target.clip_name
            ))
        })?;
        let authored_length = (expected.stop_time - expected.start_time) as f32;
        if clip.name != target.clip_name
            || clip.looping != expected.looping
            || clip.length != authored_length
        {
            return Err(invalid(format!(
                "timeline {prefab}: clip {} differs from the group's {} ({} s, loop {}; the timeline's {} s, loop {})",
                target.clip_name, clip.name, clip.length, clip.looping, authored_length, expected.looping
            )));
        }
        if !clip.events.is_empty() {
            with_events.push((clip.name.clone(), clip.events.len()));
        }
        prepared.push((
            key.clone(),
            TimelineAnimationBinding {
                animator,
                graph: graph.clone(),
                clip: clip.handle.clone(),
                source: expected,
                coverage: AnimationCoverage::sampled_pose(),
            },
        ));
    }
    for (key, binding) in prepared {
        request.bindings.animations.insert(key, binding);
    }
    let actor = request.owner.activity.actor;
    for identity in tracks {
        request.bindings.actors.insert(identity, actor);
    }
    Ok(with_events)
}

fn prepare_actor_tracks(
    world: &mut World,
    request: &mut StartTimeline,
    unit_id: u32,
    actor: Entity,
    animator: Entity,
    graph: Handle<AnimationGraph>,
    explicit_cast: bool,
) -> Result<(), TimelineFailure> {
    world.init_resource::<TimelineAssetLoads>();
    let server = world.resource::<AssetServer>().clone();
    let catalog = load_json(world, &server, ACTOR_CATALOG, require_actor_path)?;
    if catalog["version"].as_u64() != Some(1) {
        return Err(invalid("unsupported actor animation catalog"));
    }
    // Built on first need, once per parsed catalogue; every clip then looks
    // its route up instead of rescanning all rows of the catalogue.
    let mut catalog_routes: Option<Arc<CatalogRoutes>> = None;
    let mut prepared = Vec::new();
    for track in &request.definition.tracks {
        if track.class != "AnimationTrack"
            || if explicit_cast {
                track.name.parse::<u32>().ok() != Some(unit_id)
            } else {
                track.name != "CharacterAnimator"
            }
        {
            continue;
        }
        for clip in &track.clips {
            let TimelinePayload::Animation { target, .. } = &clip.payload else {
                return Err(invalid(
                    "character animation track has another payload type",
                ));
            };
            let source_clip = target.source_clip.as_ref().ok_or_else(|| {
                invalid("formal clip target has no resolved source file identity")
            })?;
            let routes = catalog_routes.get_or_insert_with(|| {
                derived(
                    &mut world.resource_mut::<TimelineAssetLoads>().routes,
                    ACTOR_CATALOG,
                    &catalog,
                    CatalogRoutes::build,
                )
            });
            let routes = routes.rows(unit_id as u64, source_clip)?;
            if routes.len() != 1 {
                return Err(invalid(
                    "actor source clip is missing or ambiguous in the formal catalog",
                ));
            }
            let route = &array(&catalog, "clipBindings")?[routes[0]];
            let route_identity = asset(&route["sourceClip"])?;
            let exported_name = string(route, "clipName")?;
            let library = array(&catalog, "libraries")?
                .get(source::unsigned(route, "library")? as usize)
                .ok_or_else(|| invalid("actor library route out of range"))?;
            if library["unitId"].as_u64() != Some(unit_id as u64) {
                return Err(invalid("actor library unit mismatch"));
            }
            let index_path = string(library, "index")?;
            let index = load_json(world, &server, index_path, require_actor_path)?;
            if index["version"].as_u64() != Some(2) {
                return Err(invalid("source-qualified v2 actor index required"));
            }
            for field in ["sourceRootName", "bindingRootName"] {
                if string(&index["binding"], field)? != string(library, field)? {
                    return Err(invalid("actor binding root route mismatch"));
                }
            }
            let segments = derived(
                &mut world.resource_mut::<TimelineAssetLoads>().segments,
                index_path,
                &index,
                IndexSegments::build,
            );
            let segments = segments.entries(&index, exported_name, &route_identity)?;
            if segments.len() != 1 {
                return Err(invalid("exact actor source segment missing or duplicated"));
            }
            let source =
                SourceAnimationEvidence::from_index_entry(segments[0], &target.target_package)?;
            let expected = SourceAnimationEvidence::from_clip_target(target)?;
            if source.asset != expected.asset
                || source.clip_name != expected.clip_name
                || source.start_time != expected.start_time
                || source.stop_time != expected.stop_time
                || source.looping != expected.looping
            {
                return Err(invalid(
                    "actor index disagrees with the referenced source AnimationClip metadata",
                ));
            }
            let path = string(library, "glb")?;
            require_actor_path(path)?;
            let handle = {
                let mut loads = world.resource_mut::<TimelineAssetLoads>();
                loads
                    .gltf
                    .get_or_insert_with(path, || server.load(format!("moly://{path}")))
                    .clone()
            };
            if let bevy::asset::LoadState::Failed(error) = server.load_state(&handle) {
                return Err(invalid(format!(
                    "actor animation library failed to load: {path}: {error}"
                )));
            }
            let gltfs = world.resource::<Assets<Gltf>>();
            let gltf = gltfs
                .get(&handle)
                .ok_or_else(|| TimelineFailure::loading("actor animation library still loading"))?;
            let animation = gltf
                .named_animations
                .get(exported_name)
                .ok_or_else(|| invalid("source-routed animation is absent from library"))?
                .clone();
            prepared.push((
                clip.key.clone(),
                TimelineAnimationBinding {
                    animator,
                    graph: graph.clone(),
                    clip: animation,
                    source,
                    coverage: AnimationCoverage::sampled_pose(),
                },
            ));
        }
    }
    for (key, binding) in prepared {
        request.bindings.animations.insert(key, binding);
    }
    for track in &request.definition.tracks {
        let named_cast = explicit_cast && track.name.parse::<u32>().ok() == Some(unit_id);
        let single_body = !explicit_cast && track.name == "CharacterAnimator";
        let single_face = !explicit_cast
            && track.clips.iter().any(|clip| {
                matches!(
                    clip.payload,
                    TimelinePayload::Eye { .. }
                        | TimelinePayload::Lip { .. }
                        | TimelinePayload::BlinkGate
                        | TimelinePayload::LipGate
                        | TimelinePayload::NpcIkTalkGate
                        | TimelinePayload::Emoticon { .. }
                )
            });
        if named_cast || single_body || single_face {
            request
                .bindings
                .actors
                .insert(track.identity.clone(), actor);
        }
    }
    Ok(())
}

fn require_actor_path(path: &str) -> Result<(), TimelineFailure> {
    require_rooted(
        path,
        ACTOR_ROOT,
        "actor catalog path is not asset-root-relative",
    )
}

/// A path the runner loads itself: asset-root-relative and under `root`.
fn require_rooted(path: &str, root: &str, refusal: &str) -> Result<(), TimelineFailure> {
    if !path.starts_with(root)
        || path.contains(':')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(invalid(refusal));
    }
    Ok(())
}

fn load_json(
    world: &mut World,
    server: &AssetServer,
    path: &str,
    check: fn(&str) -> Result<(), TimelineFailure>,
) -> Result<Arc<Value>, TimelineFailure> {
    check(path)?;
    let handle = {
        let mut loads = world.resource_mut::<TimelineAssetLoads>();
        loads
            .json
            .get_or_insert_with(path, || server.load(format!("moly://{path}")))
            .clone()
    };
    if let bevy::asset::LoadState::Failed(error) = server.load_state(&handle) {
        return Err(invalid(format!(
            "actor metadata failed to load: {path}: {error}"
        )));
    }
    let generation = world
        .get_resource_ref::<Assets<JsonAsset>>()
        .map(|assets| assets.last_changed());
    if let Some((validated_at, _, parsed)) =
        world.resource::<TimelineAssetLoads>().parsed_json.get(path)
    {
        if matches!((*validated_at, generation), (Some(a), Some(b)) if a == b) {
            return Ok(parsed.clone());
        }
    }
    let (text, parsed) = {
        let assets = world.resource::<Assets<JsonAsset>>();
        // Consult the live asset before the cache. Removal/loading must not
        // make a previous successful document usable in its absence.
        let json = assets.get(&handle).ok_or_else(|| {
            TimelineFailure::loading(format!("actor metadata still loading: {path}"))
        })?;
        if let Some((_, text, parsed)) =
            world.resource::<TimelineAssetLoads>().parsed_json.get(path)
        {
            if text == &json.0 {
                let parsed = parsed.clone();
                if let Some(entry) = world
                    .resource_mut::<TimelineAssetLoads>()
                    .parsed_json
                    .get_mut(path)
                {
                    entry.0 = generation;
                }
                return Ok(parsed);
            }
        }
        // A changed malformed document keeps the original parse error; an
        // older cached success is not a fallback. Do not clone the source
        // string or Value on an unchanged-document hit.
        let parsed: Arc<Value> = Arc::new(
            serde_json::from_str(&json.0)
                .map_err(|error| invalid(format!("actor metadata JSON: {error}")))?,
        );
        (json.0.clone(), parsed)
    };
    world
        .resource_mut::<TimelineAssetLoads>()
        .parsed_json
        .insert(path.into(), (generation, text, parsed.clone()));
    Ok(parsed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CoverageState {
    /// An upstream transformation has explicitly implemented this operation.
    BakedIntoClip {
        evidence: String,
    },
    Unimplemented,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationCoverage {
    pub foot_ik: CoverageState,
    pub remove_start_offset: CoverageState,
    pub track_match: CoverageState,
    pub playable_offsets: CoverageState,
}
impl AnimationCoverage {
    pub(crate) fn sampled_pose() -> Self {
        Self {
            foot_ik: CoverageState::Unimplemented,
            remove_start_offset: CoverageState::Unimplemented,
            track_match: CoverageState::Unimplemented,
            playable_offsets: CoverageState::Unimplemented,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TimelineCoverageGap {
    pub clip: Option<TimelineClipKey>,
    pub feature: &'static str,
    pub detail: String,
}

#[derive(Clone)]
pub(crate) struct TimelineAnimationBinding {
    pub animator: Entity,
    pub graph: Handle<AnimationGraph>,
    pub clip: Handle<AnimationClip>,
    pub source: SourceAnimationEvidence,
    pub coverage: AnimationCoverage,
}

#[derive(Clone, Default)]
pub(crate) struct TimelineBindings {
    pub animations: HashMap<TimelineClipKey, TimelineAnimationBinding>,
    pub actors: HashMap<SourceAssetId, Entity>,
    /// SE clips that play silent because their SE has no route or its audio
    /// failed to load. Only the player owner fills this (see
    /// `silence_unavailable_sounds`); every other owner leaves it empty and
    /// still requires each SE.
    pub silent_sounds: HashSet<TimelineClipKey>,
    pub sounds: HashMap<TimelineClipKey, Handle<AudioSource>>,
    pub controls:
        HashMap<TimelineClipKey, crate::fixture_timeline_particles::ParticleControlBinding>,
    /// Control clips of a director that binds them through its own
    /// exposed-reference table (a step item, a site prefab's director) and
    /// that this runner does not drive, each with the reason, by clip. The
    /// rest of the timeline plays; each one is a named coverage gap.
    pub refused_controls: HashMap<TimelineClipKey, String>,
    /// The `SignalReceiver` bound to a track's output (`SetGenericBinding`),
    /// by track: its markers' signals go to this receiver's reactions.
    pub signal_receivers: HashMap<SourceAssetId, SignalReceiverBinding>,
    /// The particle calls a marker's signal makes on its track's receiver,
    /// prepared on the systems they play or stop, by the `SignalEmitter`
    /// marker (see [`prepare_source_signals`]).
    pub signals: HashMap<SourceAssetId, Vec<crate::fixture_timeline_particles::SignalReaction>>,
    /// The particle calls of Signal markers that are not prepared, each with
    /// the reason: named coverage gaps that send nothing.
    pub refused_signals: Vec<String>,
}

/// A `SignalReceiver` bound to a track: its name and its reaction table.
#[derive(Clone, Debug)]
pub(crate) struct SignalReceiverBinding {
    /// The receiver's GameObject, for the logs.
    pub receiver: String,
    pub reactions: Vec<SignalReaction>,
}

/// One row of a receiver's table: a signal asset and the persistent calls
/// its UnityEvent makes.
#[derive(Clone, Debug)]
pub(crate) struct SignalReaction {
    pub signal: SourceAssetId,
    pub signal_name: String,
    pub calls: Vec<ReceiverCall>,
}

impl SignalReaction {
    /// The row's calls as `Type.Method(argument) on target`, for the logs.
    pub(crate) fn calls_text(&self) -> String {
        self.calls
            .iter()
            .map(|call| call.text.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One persistent call of a reaction's UnityEvent.
#[derive(Clone, Debug)]
pub(crate) struct ReceiverCall {
    /// `Type.Method(argument) on target`, with its call state.
    pub text: String,
    /// The call state is not `Off`: `UnityEvent.Invoke` runs it at run time.
    pub runtime: bool,
    /// A call the particle host runs: `ParticleSystem.Play()` or `Stop()`
    /// with no argument, on at run time, with the ParticleSystem component
    /// it targets. `None` for any other call.
    pub particle: Option<(crate::fixture_timeline_particles::SignalCall, SourceAssetId)>,
}
#[derive(Clone)]
pub(crate) struct TimelineCompanionTrack {
    pub source: Arc<TimelineDefinition>,
    pub track: SourceAssetId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimelineOwnerKind {
    Npc,
    Player,
    /// The selected fixture-talk cast owns one shared source Director clock.
    Talk,
    /// A step item of the player avatar (`PlayerAvatarItemTimelineView`):
    /// its director plays with no timeout, and its view sets the loop flag
    /// (`LoopFlagClip.ChangeLoopFlagState`) instead of skipping the loop.
    StepItem,
    /// A director inside a site prefab, bound to the site scene's own
    /// objects (the delivery site's tree, `DeliveryPlaceObjectView`): it
    /// plays with no timeout and its view holds it paused at a time
    /// (`Pause`, then evaluated every frame).
    SceneDirector,
    /// A cut-scene view's director (`CutSceneView.PlayAsync`): it plays with
    /// no timeout; its cut-scene tracks (camera, fade panel, obstacles,
    /// expansion effect, effects, activation, recorded root clips) are driven
    /// by the cut-scene owner from this clock. Its cast members and the
    /// fixture it binds hold the owner's cut-scene lease.
    CutScene,
    /// A fixture's own timeline played with no NPC
    /// (`FixtureTimelineFactory.ImmediateCreateFixtureTimelineForWithoutNPCAsync`):
    /// it plays with no timeout on the fixture alone, which is also its
    /// actor.
    FixtureOnly,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum TimelineTimeoutBudget {
    /// Only the local-player source entry uses this ungated budget.
    PlayerWall,
    /// The NPC activity owner supplies GameState/Talk gating. Unknown input
    /// is unfinished preparation, never an implicit true/false substitute.
    OwnerGated { advance: Option<bool> },
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum TimelineCompletionReason {
    AxisEnd,
    NpcBudgetThenAxisEnd,
    PlayerBudget,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TimelineOwner {
    pub activity: FixtureActivityOwner,
    pub kind: TimelineOwnerKind,
}
#[derive(Clone)]
pub(crate) struct StartTimeline {
    pub owner: TimelineOwner,
    pub fixture: Entity,
    pub definition: Arc<TimelineDefinition>,
    pub bindings: TimelineBindings,
    pub companions: Vec<TimelineCompanionTrack>,
    /// Supplied by the source activity entry point; budget is not axis length.
    pub timeout_secs: f64,
    pub timeout_budget: TimelineTimeoutBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct TimelineToken {
    serial: u64,
    owner: FixtureActivityOwner,
}
#[derive(Clone, Debug)]
pub(crate) enum TimelineStatus {
    Preparing,
    Playing {
        time: f64,
        loop_started: bool,
        loop_active: bool,
        enable_talk: bool,
    },
    Completed,
    Cancelled,
    Failed(TimelineFailure),
}

#[derive(Component)]
struct TimelinePlaybackOwner(TimelineToken);

/// Authored facial gates are exposed separately from the current open/closed
/// atlas cell. Mouth control consumes the source enable/analyzer assignment;
/// automatic blinking and NPC talk IK remain separate consumers.
#[derive(Component, Clone, Debug)]
pub(crate) struct TimelineFacialState {
    pub token: TimelineToken,
    pub blink_gate: bool,
    /// None means this actor has no mouth-state track, not an authored disable.
    /// Some(false) explicitly disables even when an ordinary voice is present.
    pub lip_gate: Option<bool>,
    pub npc_ik_talk_gate: bool,
}

struct BoundNode {
    key: TimelineClipKey,
    animator: Entity,
    node: AnimationNodeIndex,
}
struct PriorFace {
    st: [f32; 4],
    lip_pattern: Option<LipPattern>,
}

struct Session {
    request: StartTimeline,
    status: TimelineStatus,
    clock: clock::Clock,
    nodes: Vec<BoundNode>,
    prior_pose: Vec<(Entity, Transform)>,
    prior_face: HashMap<Handle<CharacterMaterial>, PriorFace>,
    prior_gates: HashMap<Entity, Option<TimelineFacialState>>,
    sound_entities: Vec<Entity>,
    emoticons: HashMap<TimelineClipKey, crate::emoticon::timeline::EmoteLease>,
    effects: effects::OwnedEffects,
    coverage: Vec<TimelineCoverageGap>,
    cancel: bool,
    release: bool,
    initialized: bool,
    timeout_elapsed: f64,
    timeout_requested: bool,
    completion: Option<TimelineCompletionReason>,
    active_events: HashSet<TimelineClipKey>,
    /// The sampled time of the previous tick (the notification behaviour's
    /// previous time).
    signal_time: Option<f64>,
    /// The signal emitters whose notification has fired and is not re-armed.
    signals_fired: HashSet<SourceAssetId>,
    /// The objects a signal played in this session, sampled for their live
    /// particles, whether a signal stopped them since, and the real time of
    /// the last sample.
    signal_played: Vec<(Entity, bool)>,
    signal_sampled: f64,
}
#[derive(Resource, Default)]
pub(crate) struct FixtureActivityTimelines {
    next_serial: u64,
    sessions: HashMap<TimelineToken, Session>,
    /// Cache graph nodes, not running state. Reusing a clip in a new session
    /// cannot accumulate another node every time a character sits down.
    nodes: HashMap<
        (
            AssetId<AnimationGraph>,
            TimelineClipKey,
            AssetId<AnimationClip>,
        ),
        AnimationNodeIndex,
    >,
}

impl FixtureActivityTimelines {
    pub(crate) fn has_no_loop(&self, token: TimelineToken) -> bool {
        self.sessions.get(&token).is_some_and(|session| {
            !session
                .request
                .definition
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .any(|clip| matches!(clip.payload, TimelinePayload::LoopFlag { .. }))
        })
    }

    pub(crate) fn request_start(&mut self, request: StartTimeline) -> TimelineToken {
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .expect("timeline serial exhausted");
        let token = TimelineToken {
            serial: self.next_serial,
            owner: request.owner.activity,
        };
        self.sessions.insert(
            token,
            Session {
                request,
                status: TimelineStatus::Preparing,
                clock: Default::default(),
                nodes: Vec::new(),
                prior_pose: Vec::new(),
                prior_face: HashMap::new(),
                prior_gates: HashMap::new(),
                sound_entities: Vec::new(),
                emoticons: HashMap::new(),
                effects: Default::default(),
                coverage: Vec::new(),
                cancel: false,
                release: false,
                initialized: false,
                timeout_elapsed: 0.0,
                timeout_requested: false,
                completion: None,
                active_events: HashSet::new(),
                signal_time: None,
                signals_fired: HashSet::new(),
                signal_played: Vec::new(),
                signal_sampled: f64::NEG_INFINITY,
            },
        );
        token
    }
    pub(crate) fn status(&self, token: TimelineToken) -> Option<&TimelineStatus> {
        self.sessions.get(&token).map(|s| &s.status)
    }
    pub(crate) fn coverage(&self, token: TimelineToken) -> Option<&[TimelineCoverageGap]> {
        self.sessions.get(&token).map(|s| s.coverage.as_slice())
    }
    pub(crate) fn set_timeout_budget(
        &mut self,
        token: TimelineToken,
        advance: Option<bool>,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(&token) else {
            return false;
        };
        let TimelineTimeoutBudget::OwnerGated { advance: gate } =
            &mut session.request.timeout_budget
        else {
            return false;
        };
        *gate = advance;
        true
    }
    pub(crate) fn timeout_elapsed(&self, token: TimelineToken) -> Option<f64> {
        self.sessions
            .get(&token)
            .map(|session| session.timeout_elapsed)
    }
    pub(crate) fn sampled_time(&self, token: TimelineToken) -> Option<f64> {
        self.sessions
            .get(&token)
            .map(|session| session.clock.sampled_time)
    }
    pub(crate) fn completion_reason(
        &self,
        token: TimelineToken,
    ) -> Option<TimelineCompletionReason> {
        self.sessions
            .get(&token)
            .and_then(|session| session.completion)
    }
    pub(crate) fn request_end(&mut self, token: TimelineToken) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        if !matches!(
            s.status,
            TimelineStatus::Preparing | TimelineStatus::Playing { .. }
        ) {
            return false;
        }
        s.clock.request_end();
        true
    }
    /// A step item view's `ChangeLoopFlag(state)`: the loop flag alone is set,
    /// so with `false` the current loop still plays to its end before the
    /// rest of the axis, and `true` loops again.
    pub(crate) fn set_loop_flag(&mut self, token: TimelineToken, state: bool) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        if s.request.owner.kind != TimelineOwnerKind::StepItem
            || !matches!(
                s.status,
                TimelineStatus::Preparing | TimelineStatus::Playing { .. }
            )
        {
            return false;
        }
        s.clock.set_loop_flag(state);
        true
    }
    /// A step item view's `MoveEndTime`: the director's time is put at its
    /// duration, where the view holds the last frame.
    pub(crate) fn move_end_time(&mut self, token: TimelineToken) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        if s.request.owner.kind != TimelineOwnerKind::StepItem
            || !matches!(
                s.status,
                TimelineStatus::Preparing | TimelineStatus::Playing { .. }
            )
        {
            return false;
        }
        s.clock.move_to(s.request.definition.duration);
        true
    }
    /// A scene director view's `Pause` (held: the clock stops and the paused
    /// time is evaluated every frame) or `Play` (released: the clock runs
    /// from the held time).
    pub(crate) fn set_paused(&mut self, token: TimelineToken, paused: bool) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        if s.request.owner.kind != TimelineOwnerKind::SceneDirector
            || !matches!(
                s.status,
                TimelineStatus::Preparing | TimelineStatus::Playing { .. }
            )
        {
            return false;
        }
        s.clock.set_paused(paused);
        true
    }
    /// A conversation can own a new Director or join an existing NPC activity.
    /// Release the loop of each admitted cast/fixture owner and retain the exact
    /// generations until their authored tails finish. No replacement is drawn.
    pub(crate) fn request_talk_end(
        &mut self,
        actors: &[Entity],
        fixtures: &[Entity],
    ) -> Vec<TimelineToken> {
        let tokens: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                actors.contains(&session.request.owner.activity.actor)
                    && fixtures.contains(&session.request.fixture)
                    && matches!(
                        session.request.owner.kind,
                        TimelineOwnerKind::Npc | TimelineOwnerKind::Talk
                    )
                    && matches!(
                        session.status,
                        TimelineStatus::Preparing | TimelineStatus::Playing { .. }
                    )
            })
            .map(|(token, _)| *token)
            .collect();
        for token in &tokens {
            self.request_end(*token);
        }
        tokens
    }
    pub(crate) fn cancel(&mut self, token: TimelineToken) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        s.cancel = true;
        true
    }
    /// Cleanup is processed by the exclusive system before the record is
    /// removed. Dropping a token must not leak sounds or a paused animator.
    pub(crate) fn release(&mut self, token: TimelineToken) -> bool {
        let Some(s) = self.sessions.get_mut(&token) else {
            return false;
        };
        s.release = true;
        true
    }
}

/// Release this exact generation synchronously. Replacing a conversation must
/// not leave the previous token owning an animator until an unrelated next tick.
pub(crate) fn cancel_and_release(world: &mut World, token: TimelineToken) {
    let Some(mut timelines) = world.remove_resource::<FixtureActivityTimelines>() else {
        return;
    };
    if let Some(mut session) = timelines.sessions.remove(&token) {
        cleanup(world, token, &mut session);
    }
    world.insert_resource(timelines);
}

fn all_tracks(request: &StartTimeline) -> Result<Vec<&TimelineTrack>, TimelineFailure> {
    let mut result: Vec<_> = request.definition.tracks.iter().collect();
    let mut identities: HashSet<_> = result.iter().map(|t| t.identity.clone()).collect();
    for companion in &request.companions {
        let track = companion
            .source
            .tracks
            .iter()
            .find(|t| t.identity == companion.track)
            .ok_or_else(|| invalid("companion does not belong to its exact source timeline"))?;
        if track.class != "SETrack"
            || track
                .clips
                .iter()
                .any(|c| !matches!(c.payload, TimelinePayload::Se { .. }))
        {
            return Err(invalid(
                "only explicitly selected source SE tracks may be companions",
            ));
        }
        if !identities.insert(track.identity.clone()) {
            return Err(invalid("duplicated companion track"));
        }
        result.push(track);
    }
    Ok(result)
}

/// The audio routing table is built once, from files requested at start. While
/// that loader is still running this is a wait; once it has finished without
/// the table, no later frame supplies it, so the failure is final.
fn routing_absent(world: &World) -> TimelineFailure {
    if world.contains_resource::<crate::audio::AudioRequests>() {
        TimelineFailure::loading("audio routing not ready")
    } else {
        invalid("audio routing is absent and its loader has finished")
    }
}

/// The player owner's SE rule. The source loads each SE bundle of a player
/// timeline without checking the result (LoadSound ignores what
/// LoadSoundBundleRefCount returns), and an SE clip whose bundle is missing
/// only logs when it plays. So here an SE with no route, or whose audio
/// failed to load, is marked silent instead of failing the request: the
/// timeline plays on without it. Returns `package/cue: reason` for each clip
/// newly marked. Decides nothing while the routing table is absent; the
/// strict preparation reports that wait.
pub(crate) fn silence_unavailable_sounds(
    world: &World,
    request: &mut StartTimeline,
) -> Vec<String> {
    let Some(routing) = world.get_resource::<Routing>() else {
        return Vec::new();
    };
    let server = world.get_resource::<AssetServer>();
    let Ok(tracks) = all_tracks(request) else {
        return Vec::new();
    };
    let mut silenced = Vec::new();
    for track in tracks {
        for clip in &track.clips {
            let TimelinePayload::Se { package, cue } = &clip.payload else {
                continue;
            };
            if request.bindings.silent_sounds.contains(&clip.key) {
                continue;
            }
            let reason = if routing.timeline_se_asset_path(package, cue).is_none() {
                "no route"
            } else if request.bindings.sounds.get(&clip.key).is_some_and(|handle| {
                server.is_some_and(|server| {
                    matches!(server.load_state(handle), bevy::asset::LoadState::Failed(_))
                })
            }) {
                "audio failed to load"
            } else {
                continue;
            };
            silenced.push((clip.key.clone(), format!("{package}/{cue}: {reason}")));
        }
    }
    silenced
        .into_iter()
        .map(|(key, description)| {
            request.bindings.sounds.remove(&key);
            request.bindings.silent_sounds.insert(key);
            description
        })
        .collect()
}

/// Load exact `(package,cue)` resources through the existing audio Routing
/// and AssetServer. Calling this does not start audio or admit an action.
/// A clip in `silent_sounds` is neither routed nor loaded.
pub(crate) fn prepare_source_sounds(
    world: &World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    let routing = world
        .get_resource::<Routing>()
        .ok_or_else(|| routing_absent(world))?;
    let server = world
        .get_resource::<AssetServer>()
        .ok_or_else(|| invalid("asset server missing"))?;
    let mut sounds = HashMap::new();
    for track in all_tracks(request)? {
        for clip in &track.clips {
            if let TimelinePayload::Se { package, cue } = &clip.payload {
                if request.bindings.silent_sounds.contains(&clip.key) {
                    continue;
                }
                let path = routing
                    .timeline_se_asset_path(package, cue)
                    .ok_or_else(|| invalid(format!("missing source SE {package}/{cue}")))?;
                sounds.insert(
                    clip.key.clone(),
                    server.load::<AudioSource>(AssetPath::from(format!("moly://{path}"))),
                );
            }
        }
    }
    request.bindings.sounds = sounds;
    Ok(())
}

pub(crate) fn prepare_source_effects(
    world: &mut World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    effects::prepare(world, request).map_err(source_effect_failure)
}

/// The particle calls of the director's Signal markers. A `SignalEmitter`
/// on a track bound to a receiver notifies it (`SignalReceiver.OnNotify`):
/// the receiver invokes the UnityEvent of the first row whose signal is the
/// marker's (`IndexOf`), whose persistent calls run in order. Each
/// `ParticleSystem.Play()` / `Stop()` call is prepared here on the spawned
/// object that carries its target system, read from the director's package
/// particle document, and sent by the notification pass through the
/// particle host. A call the host refuses, or any other call, is a named
/// coverage gap and sends nothing. Retry while the error is retryable.
pub(crate) fn prepare_source_signals(
    world: &mut World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    let mut signals: HashMap<
        SourceAssetId,
        Vec<crate::fixture_timeline_particles::SignalReaction>,
    > = HashMap::new();
    let mut refused = Vec::new();
    let definition = request.definition.clone();
    for track in &definition.tracks {
        let Some(binding) = request
            .bindings
            .signal_receivers
            .get(&track.identity)
            .cloned()
        else {
            continue;
        };
        for marker in track
            .markers
            .iter()
            .filter(|marker| marker.class == "SignalEmitter")
        {
            let Some(row) = marker.signal.as_ref().and_then(|signal| {
                binding
                    .reactions
                    .iter()
                    .find(|reaction| &reaction.signal == signal)
            }) else {
                continue;
            };
            for call in &row.calls {
                let Some((kind, component)) = &call.particle else {
                    continue;
                };
                let head = format!(
                    "track {} t={:.4}: {} of {}",
                    track.name, marker.time, call.text, binding.receiver
                );
                let target = match signal_target(world, component) {
                    Ok(target) => target,
                    Err(reason) => {
                        refused.push(format!("{head}: {reason}"));
                        continue;
                    }
                };
                let (object, owner) = target;
                let played = match crate::fixture_timeline_particles::prepare_played_object(
                    world,
                    owner,
                    object,
                    &definition.package,
                ) {
                    Ok(played) => played,
                    Err(error) if error.retryable => return Err(source_effect_failure(error)),
                    Err(error) => {
                        refused.push(format!(
                            "{head}: refused by the particle host: {}",
                            error.message
                        ));
                        continue;
                    }
                };
                signals.entry(marker.asset.clone()).or_default().push(
                    crate::fixture_timeline_particles::SignalReaction {
                        time: marker.time,
                        emit_once: marker.emit_once,
                        signal: row.signal_name.clone(),
                        call: *kind,
                        target: played,
                    },
                );
            }
        }
    }
    request.bindings.signals = signals;
    request.bindings.refused_signals = refused;
    Ok(())
}

/// The particle calls of a receiver's table, prepared on the systems they
/// play or stop ahead of the director that will notify it: the host's
/// preparation takes frames the source does not spend, and a director waits
/// for it before it starts. `Ok(true)` once every call is prepared or
/// refused (a refused call is named again when the director binds it).
pub(crate) fn prepare_receiver_targets(
    world: &mut World,
    receiver: &SignalReceiverBinding,
    package: &str,
) -> Result<bool, TimelineFailure> {
    let mut pending = false;
    for reaction in &receiver.reactions {
        for call in &reaction.calls {
            let Some((_, component)) = &call.particle else {
                continue;
            };
            let Ok((object, owner)) = signal_target(world, component) else {
                continue;
            };
            match crate::fixture_timeline_particles::prepare_played_object(
                world, owner, object, package,
            ) {
                Ok(_) => {}
                Err(error) if error.retryable => pending = true,
                Err(_) => {}
            }
        }
    }
    Ok(!pending)
}

/// The spawned node that carries a receiver call's target ParticleSystem
/// component, and the prefab root above it that owns the particle document
/// (the nearest ancestor under the coordinate contract).
fn signal_target(world: &mut World, component: &SourceAssetId) -> Result<(Entity, Entity), String> {
    let wanted: i64 = component
        .path_id
        .parse()
        .map_err(|_| format!("invalid target component id {}", component.path_id))?;
    let hits: Vec<Entity> = world
        .query::<(
            Entity,
            &moly_assets::source_navigation::SourceObjectIdentity,
        )>()
        .iter(world)
        .filter(|(_, identity)| {
            identity.file == component.file && identity.components.contains(&wanted)
        })
        .map(|(entity, _)| entity)
        .collect();
    let [object] = hits.as_slice() else {
        return Err(format!(
            "{} spawned nodes carry its target component {}/{}",
            hits.len(),
            component.file,
            component.path_id
        ));
    };
    let mut owner = *object;
    while world
        .get::<moly_assets::coordinates::CanonicalCoordinates>(owner)
        .is_none()
    {
        owner = world
            .get::<ChildOf>(owner)
            .map(ChildOf::parent)
            .ok_or("its node has no ancestor under the coordinate contract")?;
    }
    Ok((*object, owner))
}

fn source_effect_failure(mut failure: TimelineFailure) -> TimelineFailure {
    failure.message = format!("source-effects: {}", failure.message);
    failure
}

pub(crate) fn validate_start(
    world: &World,
    request: &StartTimeline,
) -> Result<(), TimelineFailure> {
    validate(world, request, None, true)
}

/// Check cached resource bindings without claiming an animator. An already
/// running timeline is not a reason to reload its actor GLTF every frame;
/// actual admission still goes through `validate_start` and its lease check.
pub(crate) fn validate_prepared(
    world: &World,
    request: &StartTimeline,
) -> Result<(), TimelineFailure> {
    validate(world, request, None, false)
}

fn validate(
    world: &World,
    request: &StartTimeline,
    own: Option<TimelineToken>,
    require_available: bool,
) -> Result<(), TimelineFailure> {
    if !world.entities().contains(request.fixture)
        || !world.entities().contains(request.owner.activity.actor)
    {
        return Err(invalid("activity actor or fixture no longer exists"));
    }
    if !request.timeout_secs.is_finite() || request.timeout_secs <= 0.0 {
        return Err(invalid("invalid activity timeout budget"));
    }
    match (request.owner.kind, request.timeout_budget) {
        (
            TimelineOwnerKind::Player
            | TimelineOwnerKind::StepItem
            | TimelineOwnerKind::SceneDirector
            | TimelineOwnerKind::CutScene
            | TimelineOwnerKind::FixtureOnly,
            TimelineTimeoutBudget::PlayerWall,
        )
        | (
            TimelineOwnerKind::Npc | TimelineOwnerKind::Talk,
            TimelineTimeoutBudget::OwnerGated { advance: Some(_) },
        ) => {}
        _ => {
            return Err(invalid(
                "source activity timeout budget/gate is not prepared",
            ))
        }
    }
    let clips = world
        .get_resource::<Assets<AnimationClip>>()
        .ok_or_else(|| invalid("animation assets not installed"))?;
    let graphs = world
        .get_resource::<Assets<AnimationGraph>>()
        .ok_or_else(|| invalid("animation graphs not installed"))?;
    let mut targets: HashMap<Entity, HashSet<AnimationTargetId>> = HashMap::new();
    if let Some(mut query) = QueryState::<(&AnimationTargetId, &AnimatedBy)>::try_new(world) {
        for (id, by) in query.iter(world) {
            targets.entry(by.0).or_default().insert(*id);
        }
    }
    let tracks = all_tracks(request)?;
    let mut required_animations = HashSet::new();
    let mut required_sounds = HashSet::new();
    let mut loop_count = 0;
    for track in tracks {
        // A recorded clip is driven by the cut-scene owner on the actor its
        // track is bound to; no other owner plays one.
        if track.infinite.is_some() {
            let actor = request
                .bindings
                .actors
                .get(&track.identity)
                .filter(|_| request.owner.kind == TimelineOwnerKind::CutScene)
                .ok_or_else(|| invalid("infinite animation clip needs an explicit binding"))?;
            validate_track_actor(world, request, &track.identity, *actor)?;
        }
        for clip in &track.clips {
            match &clip.payload {
                TimelinePayload::Animation { target, settings } => {
                    required_animations.insert(clip.key.clone());
                    let binding = request.bindings.animations.get(&clip.key).ok_or_else(|| {
                        invalid(format!("missing animation binding {:?}", clip.key))
                    })?;
                    if world.get::<AnimationPlayer>(binding.animator).is_none() {
                        return Err(invalid("actual AnimationPlayer not installed"));
                    }
                    let graph = world
                        .get::<AnimationGraphHandle>(binding.animator)
                        .ok_or_else(|| invalid("actual animation graph handle missing"))?;
                    if graph.0 != binding.graph || graphs.get(&binding.graph).is_none() {
                        return Err(invalid("binding graph differs from actual player's graph"));
                    }
                    if require_available
                        && world
                            .get::<TimelinePlaybackOwner>(binding.animator)
                            .is_some_and(|p| Some(p.0) != own)
                    {
                        return Err(invalid("animator already belongs to another timeline"));
                    }
                    let expected_root =
                        if let Some(actor) = request.bindings.actors.get(&track.identity) {
                            validate_track_actor(world, request, &track.identity, *actor)?;
                            *actor
                        } else if track.name == "CharacterAnimator" {
                            request.owner.activity.actor
                        } else {
                            request.fixture
                        };
                    if !descendant_of(world, binding.animator, expected_root) {
                        return Err(invalid(
                            "track is not bound to its actual actor/fixture instance",
                        ));
                    }
                    let source = &binding.source;
                    let expected_source = SourceAnimationEvidence::from_clip_target(target)?;
                    let source_clip = target.source_clip.as_ref().ok_or_else(|| {
                        invalid("formal clip target has no resolved source file identity")
                    })?;
                    if source.package != target.target_package
                        || source.clip_name != target.clip_name
                        || source.asset.path_id != target.path_id
                        || &source.asset != source_clip
                        || source.start_time != expected_source.start_time
                        || source.stop_time != expected_source.stop_time
                        || source.looping != expected_source.looping
                    {
                        return Err(invalid("rendered clip evidence does not identify the selected source animation"));
                    }
                    if !source.start_time.is_finite()
                        || !source.stop_time.is_finite()
                        || source.stop_time <= source.start_time
                    {
                        return Err(invalid("source clip bounds are missing"));
                    }
                    if source.start_time != 0.0 {
                        return Err(invalid(
                            "nonzero source clip origin needs an explicit export mapping",
                        ));
                    }
                    if settings.loop_mode > 2 {
                        return Err(invalid("unknown AnimationPlayableAsset loop mode"));
                    }
                    if (settings.position != [0.0; 3]
                        || settings.rotation != [0.0, 0.0, 0.0, 1.0]
                        || settings.euler_angles != [0.0; 3])
                        && !baked(&binding.coverage.playable_offsets)
                    {
                        return Err(invalid("nonidentity playable offset has not been applied"));
                    }
                    let animation = clips
                        .get(&binding.clip)
                        .ok_or_else(|| TimelineFailure::loading("animation clip still loading"))?;
                    if animation.curves().is_empty()
                        || animation.curves().values().all(Vec::is_empty)
                        || !animation.duration().is_finite()
                        || animation.duration() <= 0.0
                    {
                        return Err(invalid("empty AnimationClip is not playable"));
                    }
                    let bound = targets
                        .get(&binding.animator)
                        .ok_or_else(|| invalid("actual animator has no bound targets"))?;
                    if animation
                        .curves()
                        .iter()
                        .any(|(target, curves)| !curves.is_empty() && !bound.contains(target))
                    {
                        return Err(invalid(
                            "source animation has targets not bound to the actual animator",
                        ));
                    }
                }
                TimelinePayload::Se { package, cue } => {
                    // A silent SE plays nothing, so nothing is required of it.
                    if request.bindings.silent_sounds.contains(&clip.key) {
                        continue;
                    }
                    required_sounds.insert(clip.key.clone());
                    let routing = world
                        .get_resource::<Routing>()
                        .ok_or_else(|| routing_absent(world))?;
                    let path = routing
                        .timeline_se_asset_path(package, cue)
                        .ok_or_else(|| invalid("source SE route unavailable"))?;
                    let handle = request
                        .bindings
                        .sounds
                        .get(&clip.key)
                        .ok_or_else(|| invalid("source SE has not been prepared"))?;
                    let expected_path = format!("moly://{path}");
                    if handle.path().map(ToString::to_string).as_deref()
                        != Some(expected_path.as_str())
                    {
                        return Err(invalid("SE handle does not match the exact source route"));
                    }
                    if let Some(server) = world.get_resource::<AssetServer>() {
                        if let bevy::asset::LoadState::Failed(error) = server.load_state(handle) {
                            return Err(invalid(format!(
                                "source SE audio failed to load: {path}: {error}"
                            )));
                        }
                    }
                    if world
                        .get_resource::<Assets<AudioSource>>()
                        .and_then(|a| a.get(handle))
                        .is_none()
                    {
                        return Err(TimelineFailure::loading("source SE audio still loading"));
                    }
                    if world.get_resource::<VolumeBus>().is_none() {
                        return Err(invalid("source audio volume bus missing"));
                    }
                }
                TimelinePayload::Eye { .. } => {
                    face_handle(world, request, &track.identity, "eye")?;
                }
                TimelinePayload::Lip { .. } => {
                    face_handle(world, request, &track.identity, "mouth")?;
                }
                TimelinePayload::BlinkGate
                | TimelinePayload::LipGate
                | TimelinePayload::NpcIkTalkGate => {
                    let actor = request
                        .bindings
                        .actors
                        .get(&track.identity)
                        .ok_or_else(|| invalid("facial/IK track actor missing"))?;
                    validate_track_actor(world, request, &track.identity, *actor)?;
                }
                TimelinePayload::Emoticon { name, .. } => {
                    let actor = request
                        .bindings
                        .actors
                        .get(&track.identity)
                        .ok_or_else(|| invalid("emoticon track actor missing"))?;
                    validate_track_actor(world, request, &track.identity, *actor)?;
                    // Each is a wait only while its loader is still running:
                    // the archive while its request is held, the renderer
                    // until its one-time setup has run.
                    let archive = world
                        .get_resource::<crate::emoticon::EmoticonArchive>()
                        .ok_or_else(|| {
                            if world.contains_resource::<crate::emoticon::EmoticonsHandle>() {
                                TimelineFailure::loading("emoticon archive still loading")
                            } else {
                                invalid("emoticon archive is absent and its loader has finished")
                            }
                        })?;
                    if !world.contains_resource::<crate::emoticon::Emotes>() {
                        return Err(if world.contains_resource::<crate::emoticon::Spawned>() {
                            invalid("emoticon renderer is absent and its setup has run")
                        } else {
                            TimelineFailure::loading("emoticon renderer still loading")
                        });
                    }
                    if !archive.has(name) {
                        return Err(invalid(format!("source emoticon {name} is unavailable")));
                    }
                }
                TimelinePayload::LoopFlag { .. } => loop_count += 1,
                TimelinePayload::NoPresetChange => {}
                TimelinePayload::Control(_) => {}
                TimelinePayload::CutScene(payload) => {
                    if request.owner.kind != TimelineOwnerKind::CutScene {
                        return Err(invalid(format!(
                            "cut-scene track payload outside a cut-scene owner: {payload:?}"
                        )));
                    }
                }
                TimelinePayload::Unsupported { class, .. } => {
                    return Err(invalid(format!(
                        "nonempty unsupported track payload: {class}"
                    )))
                }
            }
        }
    }
    // The current source owner selects a single loop control clip. Multi-loop
    // choreography is a separate unsupported runtime scope, not a union clock.
    if loop_count > 1 {
        return Err(invalid(
            "multiple LoopFlag clips need an explicit controller policy",
        ));
    }
    if required_animations.len() != request.bindings.animations.len()
        || required_sounds.len() != request.bindings.sounds.len()
    {
        return Err(invalid(
            "bindings contain animation/sound clips outside selected source tracks",
        ));
    }
    effects::validate(world, request, own, require_available).map_err(source_effect_failure)
}

fn descendant_of(world: &World, mut entity: Entity, ancestor: Entity) -> bool {
    loop {
        if entity == ancestor {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}
fn baked(state: &CoverageState) -> bool {
    matches!(state, CoverageState::BakedIntoClip { evidence } if !evidence.is_empty())
}

/// Which actor a track of the selected Director may drive.
///
/// A player timeline drives only its owner. An NPC timeline drives its owner
/// and, for a fixture talk with several characters, every other member of
/// that talk: the source plays one Director for every member of such a talk.
/// A member other than the owner is accepted when the track is named after
/// the member's unit and the member holds the NPC fixture movement lease of
/// this same activity. The talk's timeline controller puts that lease on
/// every member before it asks for this check, so the lease is the proof of
/// membership. A talk owner binds the cast of the current conversation: the
/// source track names the exact unit and the entity is an admitted
/// participant.
fn validate_track_actor(
    world: &World,
    request: &StartTimeline,
    identity: &SourceAssetId,
    actor: Entity,
) -> Result<(), TimelineFailure> {
    if request.owner.kind == TimelineOwnerKind::CutScene {
        return validate_cut_scene_actor(world, request, identity, actor);
    }
    if request.owner.kind != TimelineOwnerKind::Talk {
        if actor == request.owner.activity.actor {
            return Ok(());
        }
        if request.owner.kind != TimelineOwnerKind::Npc {
            return Err(invalid("track actor differs from the activity owner"));
        }
        let track = request
            .definition
            .tracks
            .iter()
            .find(|track| &track.identity == identity)
            .ok_or_else(|| invalid("actor track is outside the selected source director"))?;
        let unit = track
            .name
            .parse::<u32>()
            .map_err(|_| invalid("NPC member track has no unit name"))?;
        if world
            .get::<crate::npc::CharacterUnitId>(actor)
            .map(|id| id.0)
            != Some(unit)
        {
            return Err(invalid("NPC member track is bound to a different unit"));
        }
        return match world.get::<crate::npc_fixture_activity::NpcFixtureMotionOwner>(actor) {
            None => Err(invalid("NPC member holds no fixture lease")),
            Some(lease) if lease.0 != request.owner.activity => Err(invalid(
                "NPC member's fixture lease belongs to another activity",
            )),
            Some(_) => Ok(()),
        };
    }
    let track = request
        .definition
        .tracks
        .iter()
        .find(|track| &track.identity == identity)
        .ok_or_else(|| invalid("actor track is outside the selected source director"))?;
    let common_single = request
        .definition
        .tracks
        .iter()
        .any(|track| track.name == "CharacterAnimator")
        && world
            .get_resource::<crate::talk::ActiveTalk>()
            .is_some_and(|talk| talk.participants().len() == 1);
    if common_single && actor == request.owner.activity.actor {
        return Ok(());
    }
    let unit = track
        .name
        .parse::<u32>()
        .map_err(|_| invalid("cast track has no exact source unit name"))?;
    if world
        .get::<crate::npc::CharacterUnitId>(actor)
        .map(|id| id.0)
        != Some(unit)
    {
        return Err(invalid("cast track is bound to a different source unit"));
    }
    let admitted = world
        .get_resource::<crate::talk::ActiveTalk>()
        .is_some_and(|talk| {
            talk.participants()
                .iter()
                .any(|(candidate, entity)| *candidate == unit && *entity == actor)
        });
    if !admitted {
        return Err(invalid("cast track actor is outside the admitted talk"));
    }
    Ok(())
}

/// A cut-scene track's bound object (`CutSceneView.BindCharacter` /
/// `BindGateAnimator`): the cut-scene owner puts its lease on every cast
/// member and on the fixture it binds before it asks for this check. A
/// track named after a unit is bound to that unit's character.
fn validate_cut_scene_actor(
    world: &World,
    request: &StartTimeline,
    identity: &SourceAssetId,
    actor: Entity,
) -> Result<(), TimelineFailure> {
    let track = request
        .definition
        .tracks
        .iter()
        .find(|track| &track.identity == identity)
        .ok_or_else(|| invalid("actor track is outside the selected source director"))?;
    match world.get::<crate::cutscene::CutSceneCastLease>(actor) {
        None => return Err(invalid("cut-scene track object holds no cut-scene lease")),
        Some(lease) if lease.0 != request.owner.activity => {
            return Err(invalid(
                "cut-scene track object's lease belongs to another cut-scene",
            ))
        }
        Some(_) => {}
    }
    if let Ok(unit) = track.name.parse::<u32>() {
        if world
            .get::<crate::npc::CharacterUnitId>(actor)
            .map(|id| id.0)
            != Some(unit)
        {
            return Err(invalid("cut-scene track is bound to a different unit"));
        }
    }
    Ok(())
}

fn face_handle(
    world: &World,
    request: &StartTimeline,
    track: &SourceAssetId,
    slot: &str,
) -> Result<Handle<CharacterMaterial>, TimelineFailure> {
    let actor = request
        .bindings
        .actors
        .get(track)
        .ok_or_else(|| invalid("facial track actor missing"))?;
    validate_track_actor(world, request, track, *actor)?;
    let handle = world
        .get::<ToonMaterials>(*actor)
        .and_then(|m| m.slot_handle(slot))
        .ok_or_else(|| invalid("actual facial material not installed"))?
        .clone();
    if world
        .get_resource::<Assets<CharacterMaterial>>()
        .and_then(|m| m.get(&handle))
        .is_none()
    {
        return Err(invalid("facial material asset missing"));
    }
    Ok(handle)
}

/// Install after lifecycle updates and before the engine's animation
/// evaluation. The root owns schedule registration and the driver lease.
pub(crate) fn advance(world: &mut World) {
    let delta = world
        .get_resource::<Time>()
        .map_or(0.0, Time::delta_secs_f64);
    world.resource_scope(|world, mut runtime: Mut<FixtureActivityTimelines>| {
        let mut tokens: Vec<_> = runtime.sessions.keys().copied().collect();
        tokens.sort_by_key(|t| t.serial);
        // Release old claims before admitting any new generation this frame.
        for token in &tokens {
            let session = runtime.sessions.get_mut(token).expect("listed session");
            if !world
                .entities()
                .contains(session.request.owner.activity.actor)
                || !world.entities().contains(session.request.fixture)
            {
                session.cancel = true;
                // No caller can consume this dead owner's result. Retaining a
                // cancelled session here would pin all its clips and sounds.
                session.release = true;
            }
            if session.cancel || session.release {
                cleanup(world, *token, session);
                session.status = TimelineStatus::Cancelled;
            }
        }
        runtime.sessions.retain(|_, s| !s.release);
        for token in tokens {
            let Some(mut session) = runtime.sessions.remove(&token) else {
                continue;
            };
            if matches!(
                session.status,
                TimelineStatus::Cancelled | TimelineStatus::Failed(_)
            ) {
                runtime.sessions.insert(token, session);
                continue;
            }
            let outcome = if !session.initialized {
                initialize(world, token, &mut session, &mut runtime.nodes)
            } else {
                Ok(())
            };
            if let Err(error) = outcome {
                cleanup(world, token, &mut session);
                session.status = TimelineStatus::Failed(error);
            } else if matches!(session.status, TimelineStatus::Playing { .. }) {
                let result = tick(world, token, &mut session, delta);
                if let Err(error) = result {
                    cleanup(world, token, &mut session);
                    session.status = TimelineStatus::Failed(error);
                }
            }
            runtime.sessions.insert(token, session);
        }
    });
}

fn initialize(
    world: &mut World,
    token: TimelineToken,
    session: &mut Session,
    cache: &mut HashMap<
        (
            AssetId<AnimationGraph>,
            TimelineClipKey,
            AssetId<AnimationClip>,
        ),
        AnimationNodeIndex,
    >,
) -> Result<(), TimelineFailure> {
    validate(world, &session.request, Some(token), true)?;
    if world
        .get::<moly_assets::coordinates::CanonicalCoordinates>(session.request.fixture)
        .is_none()
    {
        return Err(invalid(
            "fixture coordinate contract is missing; re-export this snapshot",
        ));
    }
    effects::claim(world, token, &session.request, &mut session.effects)
        .map_err(source_effect_failure)?;
    for track in &session.request.definition.tracks {
        for clip in &track.clips {
            let (TimelinePayload::Control(_), Some(binding)) = (
                &clip.payload,
                session.request.bindings.controls.get(&clip.key),
            ) else {
                continue;
            };
            info!(
                "[timeline-control] {}/{} Control clip {} [{:.4}, {:.4}) drives {:?}",
                session.request.definition.package,
                session.request.definition.prefab,
                clip.source_envelope["m_DisplayName"]
                    .as_str()
                    .unwrap_or("?"),
                clip.start,
                clip.end(),
                world.get::<Name>(binding.root).map(Name::as_str)
            );
        }
    }
    // Canonical locators, actor models and animation clips share one frame.
    // Validate before mutating any owned pose; an authored negative scale is
    // not a signal to introduce another spatial reflection.
    let animators: HashSet<_> = session
        .request
        .bindings
        .animations
        .values()
        .map(|binding| binding.animator)
        .collect();
    session.prior_pose = world
        .query::<(Entity, &bevy::animation::AnimatedBy, &Transform)>()
        .iter(world)
        .filter(|(_, by, _)| animators.contains(&by.0))
        .map(|(entity, _, pose)| (entity, *pose))
        .collect();
    for (entity, _) in &session.prior_pose {
        if let Some(rest) = world
            .get::<crate::character::CharacterRestTransform>(*entity)
            .copied()
        {
            *world
                .get_mut::<Transform>(*entity)
                .expect("animated transform") = rest.0;
        }
    }
    // The lookup cache owns only IDs, so a destroyed body graph cannot keep
    // its asset alive. Drop those obsolete keys when a new session starts;
    // repeated body rebuilds must not retain their metadata indefinitely.
    // Keep this out of the per-frame sampling path.
    {
        let graphs = world.resource::<Assets<AnimationGraph>>();
        cache.retain(|(graph, _, _), _| graphs.get(*graph).is_some());
    }
    let request = &session.request;
    let mut claimed = HashSet::new();
    for track in &request.definition.tracks {
        for clip in &track.clips {
            if let TimelinePayload::Animation { settings, .. } = &clip.payload {
                let binding = &request.bindings.animations[&clip.key];
                let cache_key = (binding.graph.id(), clip.key.clone(), binding.clip.id());
                let mut graphs = world.resource_mut::<Assets<AnimationGraph>>();
                let graph = graphs
                    .get_mut(&binding.graph)
                    .ok_or_else(|| invalid("animation graph disappeared"))?;
                let node = cache.get(&cache_key).copied().filter(|node| graph.graph.node_weight(*node)
                    .is_some_and(|n| matches!(&n.node_type,AnimationNodeType::Clip(handle) if *handle == binding.clip)));
                let node = node.unwrap_or_else(|| {
                    // Released slots keep stable graph indices (removing a
                    // petgraph node would move locomotion nodes). Reuse those
                    // slots instead of retaining every past clip in the graph.
                    let free = cache
                        .iter()
                        .find(|((id, _, _), node)| {
                            *id == binding.graph.id()
                                && graph.graph.node_weight(**node).is_some_and(|n| {
                                    matches!(n.node_type, AnimationNodeType::Blend)
                                })
                        })
                        .map(|(key, node)| (key.clone(), *node));
                    let node = if let Some((old_key, node)) = free {
                        cache.remove(&old_key);
                        graph.graph.node_weight_mut(node).unwrap().node_type =
                            AnimationNodeType::Clip(binding.clip.clone());
                        node
                    } else {
                        graph.add_clip(binding.clip.clone(), 1.0, graph.root)
                    };
                    cache.insert(cache_key, node);
                    node
                });
                drop(graphs);
                if claimed.insert(binding.animator) {
                    world
                        .get_mut::<AnimationPlayer>(binding.animator)
                        .expect("validated animator")
                        .stop_all();
                    if let Some(mut transitions) =
                        world.get_mut::<AnimationTransitions>(binding.animator)
                    {
                        *transitions = AnimationTransitions::new();
                    }
                    world
                        .entity_mut(binding.animator)
                        .insert(TimelinePlaybackOwner(token));
                }
                session.nodes.push(BoundNode {
                    key: clip.key.clone(),
                    animator: binding.animator,
                    node,
                });
                add_animation_coverage(&mut session.coverage, clip, settings, &binding.coverage);
            }
        }
    }
    for track in &request.definition.tracks {
        for clip in &track.clips {
            let feature = match clip.payload {
                TimelinePayload::Eye { blink: true, open, close, .. } if close >= 0 && close != open =>
                    Some(("SourceBlinkOscillator","selected eye preset carries distinct open/close cells and enables blinking; only its open cell is rendered")),
                TimelinePayload::Lip { open, middle, close, .. } if world.get::<crate::player::PlayerControlled>(request.owner.activity.actor).is_some() && (open != close || (middle >= 0 && middle != close)) =>
                    Some(("SourceLipSyncOscillator","selected lip preset supplies the actor's full row; Timeline-controlled lip enable/analyzer binding is not implemented")),
                TimelinePayload::BlinkGate => Some(("SourceBlinkOscillator","authored blink gates are retained; automatic blink cadence is not implemented")),
                TimelinePayload::LipGate if world.get::<crate::player::PlayerControlled>(request.owner.activity.actor).is_some() => Some(("SourceLipSyncOscillator","player avatar timeline lip enable/analyzer adapter is not implemented")),
                TimelinePayload::LipGate => Some(("SourceLipSyncArbitration","NPC mixer enable/null-analyzer reaches the existing oscillator; cross-owner voice setter ordering and final visual playback remain unverified")),
                TimelinePayload::NpcIkTalkGate if request.owner.kind == TimelineOwnerKind::Npc => Some(("SourceTalkIK","NPC talk IK gate is exposed but its native solver is not implemented")),
                TimelinePayload::Emoticon { .. } => Some(("SourceEmoticonSound","source Timeline emote visuals and root mode use the shared renderer; its legacy soundInput audio adapter is not implemented")),
                _ => None,
            };
            if let Some((feature, detail)) = feature {
                session.coverage.push(TimelineCoverageGap {
                    clip: Some(clip.key.clone()),
                    feature,
                    detail: detail.into(),
                });
            }
        }
    }
    let mut refused: Vec<_> = request.bindings.refused_controls.iter().collect();
    refused.sort_by(|a, b| {
        (&a.0.track.path_id, a.0.clip_index).cmp(&(&b.0.track.path_id, b.0.clip_index))
    });
    for (key, reason) in refused {
        session.coverage.push(TimelineCoverageGap {
            clip: Some(key.clone()),
            feature: "SourceControlClip",
            detail: reason.clone(),
        });
    }
    for track in &request.definition.tracks {
        let emitters = track
            .markers
            .iter()
            .filter(|marker| marker.class == "SignalEmitter")
            .count();
        if emitters == 0 {
            continue;
        }
        match request.bindings.signal_receivers.get(&track.identity) {
            None => session.coverage.push(TimelineCoverageGap {
                clip: None,
                feature: "SourceSignalBinding",
                detail: format!(
                    "track {}: {emitters} Signal markers; no receiver is bound to its output at run time and the director's own scene bindings are not read, so they notify nothing",
                    track.name
                ),
            }),
            Some(binding) => {
                for reaction in &binding.reactions {
                    for call in reaction
                        .calls
                        .iter()
                        .filter(|call| call.runtime && call.particle.is_none())
                    {
                        session.coverage.push(TimelineCoverageGap {
                            clip: None,
                            feature: "SourceSignalReaction",
                            detail: format!(
                                "track {}: {} reacts to {} with {}: not a ParticleSystem Play()/Stop() on at run time; not driven",
                                track.name, binding.receiver, reaction.signal_name, call.text
                            ),
                        });
                    }
                }
            }
        }
    }
    for reason in &request.bindings.refused_signals {
        session.coverage.push(TimelineCoverageGap {
            clip: None,
            feature: "SourceSignalReaction",
            detail: reason.clone(),
        });
    }
    if !request.bindings.sounds.is_empty() {
        session.coverage.push(TimelineCoverageGap { clip:None,feature:"SourceSESpatialization",detail:"exact source cue/package uses the existing 2D one-shot SE bus; native positional attenuation is not reproduced".into() });
    }
    session.initialized = true;
    session.status = TimelineStatus::Playing {
        time: 0.0,
        loop_started: false,
        loop_active: false,
        enable_talk: false,
    };
    Ok(())
}

fn add_animation_coverage(
    gaps: &mut Vec<TimelineCoverageGap>,
    clip: &TimelineClip,
    settings: &AnimationPlayableSettings,
    coverage: &AnimationCoverage,
) {
    let mut add = |feature, detail: &str| {
        gaps.push(TimelineCoverageGap {
            clip: Some(clip.key.clone()),
            feature,
            detail: detail.into(),
        })
    };
    if settings.apply_foot_ik && !baked(&coverage.foot_ik) {
        add(
            "SourceFootIK",
            "sampled bone curves are played; Unity humanoid FootIK is not implemented",
        );
    }
    if settings.remove_start_offset && !baked(&coverage.remove_start_offset) {
        add(
            "RemoveStartOffset",
            "source start-offset removal has not been proved baked into these curves",
        );
    }
    if settings.use_track_match_fields && !baked(&coverage.track_match) {
        add(
            "TrackMatchFields",
            "track matching/offset graph is not reproduced by the sampled-pose player",
        );
    }
    if clip.ease_in > 0.0 || clip.ease_out > 0.0 || clip.blend_in > 0.0 || clip.blend_out > 0.0 {
        add("NativeMixer","source envelope weights are evaluated; Bevy weighted pose mixing is not Unity's native mixer/default-pose residual");
    }
}

fn tick(
    world: &mut World,
    token: TimelineToken,
    session: &mut Session,
    delta: f64,
) -> Result<(), TimelineFailure> {
    if session.nodes.iter().any(|node| {
        world
            .get::<TimelinePlaybackOwner>(node.animator)
            .is_none_or(|owner| owner.0 != token)
    }) {
        return Err(invalid("timeline animator ownership was lost"));
    }
    validate(world, &session.request, Some(token), true)?;
    if !delta.is_finite() || delta < 0.0 {
        return Err(invalid("invalid director delta"));
    }
    let advance_budget = match session.request.timeout_budget {
        TimelineTimeoutBudget::PlayerWall => true,
        TimelineTimeoutBudget::OwnerGated {
            advance: Some(value),
        } => value,
        TimelineTimeoutBudget::OwnerGated { advance: None } => {
            return Err(invalid("NPC timeout budget gate is unknown"))
        }
    };
    if session.clock.started && advance_budget {
        session.timeout_elapsed += delta;
    }
    let budget_expired = session.timeout_elapsed > session.request.timeout_secs;
    if budget_expired
        && matches!(
            session.request.owner.kind,
            TimelineOwnerKind::Npc | TimelineOwnerKind::Talk
        )
    {
        // NPC PlayAsyncForNPC first waits for its gated budget, then switches
        // LoopFlag off and waits for Director.time >= duration. Timeout is a
        // request to play the source E, not an error/cancellation shortcut.
        session.timeout_requested = true;
        session.clock.request_end();
    }
    session.clock.advance(
        &session.request.definition,
        delta,
        session.request.owner.kind,
    );
    let time = session.clock.time;
    let sampled_time = session.clock.sampled_time;
    sample_animations(world, session, sampled_time)?;
    effects::sample(
        world,
        token,
        &session.request,
        &mut session.effects,
        sampled_time,
    )
    .map_err(source_effect_failure)?;
    apply_events(world, session, sampled_time)?;
    emit_signals(world, session, sampled_time)?;
    sample_signal_targets(world, session, sampled_time);
    update_facial_gates(world, token, session, sampled_time);
    session.sound_entities.retain(|entity| {
        if !world.entities().contains(*entity) {
            return false;
        }
        if world
            .get::<AudioSink>(*entity)
            .is_some_and(AudioSinkPlayback::empty)
        {
            world.despawn(*entity);
            false
        } else {
            true
        }
    });
    let player_timeout = budget_expired && session.request.owner.kind == TimelineOwnerKind::Player;
    session.status = if time >= session.request.definition.duration || player_timeout {
        session.completion = Some(if player_timeout {
            TimelineCompletionReason::PlayerBudget
        } else if session.timeout_requested {
            TimelineCompletionReason::NpcBudgetThenAxisEnd
        } else {
            TimelineCompletionReason::AxisEnd
        });
        TimelineStatus::Completed
    } else {
        TimelineStatus::Playing {
            time,
            loop_started: session.clock.loop_started,
            loop_active: session.clock.loop_active,
            enable_talk: matches!(
                session.request.owner.kind,
                TimelineOwnerKind::Npc | TimelineOwnerKind::Talk
            ) && session.clock.enable_talk,
        }
    };
    Ok(())
}

fn sample_animations(
    world: &mut World,
    session: &Session,
    time: f64,
) -> Result<(), TimelineFailure> {
    let final_sample = time >= session.request.definition.duration;
    for node in &session.nodes {
        let track = session
            .request
            .definition
            .tracks
            .iter()
            .find(|t| t.identity == node.key.track)
            .expect("prepared track");
        let clip = &track.clips[node.key.clip_index];
        let binding = &session.request.bindings.animations[&node.key];
        let duration = world
            .resource::<Assets<AnimationClip>>()
            .get(&binding.clip)
            .ok_or_else(|| invalid("clip disappeared during playback"))?
            .duration() as f64;
        let sample = clip.sample_time(time, final_sample);
        let mut player = world
            .get_mut::<AnimationPlayer>(node.animator)
            .ok_or_else(|| invalid("animator disappeared during playback"))?;
        if let Some(local) = sample {
            let TimelinePayload::Animation { settings, .. } = &clip.payload else {
                unreachable!()
            };
            let looping = match settings.loop_mode {
                0 => binding.source.looping,
                1 => true,
                2 => false,
                _ => unreachable!(),
            };
            let source_duration = binding.source.stop_time - binding.source.start_time;
            let local = if looping {
                local.rem_euclid(source_duration)
            } else {
                local.clamp(0.0, source_duration)
            };
            player
                .play(node.node)
                .pause()
                .set_repeat(RepeatAnimation::Never)
                .set_seek_time(local.min(duration) as f32)
                .set_weight(clip.weight(time));
        } else {
            player.stop(node.node);
        }
    }
    Ok(())
}

fn apply_events(
    world: &mut World,
    session: &mut Session,
    sampled_time: f64,
) -> Result<(), TimelineFailure> {
    // Clone the small event descriptors to release the immutable request borrow
    // before writing owned resource state. Animation sampling has no event clock.
    let events: Vec<_> = all_tracks(&session.request)?
        .into_iter()
        .flat_map(|track| track.clips.iter())
        .filter(|clip| {
            matches!(
                clip.payload,
                TimelinePayload::Eye { .. }
                    | TimelinePayload::Lip { .. }
                    | TimelinePayload::Se { .. }
                    | TimelinePayload::Emoticon { .. }
            )
        })
        .map(|clip| {
            (
                clip.key.clone(),
                clip.start,
                clip.duration,
                clip.payload.clone(),
            )
        })
        .collect();
    let active: HashSet<_> = events
        .iter()
        .filter(|(_, start, duration, _)| {
            sampled_time >= *start && sampled_time < *start + *duration
        })
        .map(|(key, _, _, _)| key.clone())
        .collect();
    for key in session.active_events.difference(&active) {
        if let Some(lease) = session.emoticons.get(key).copied() {
            crate::emoticon::timeline::hide(world, lease, false);
        }
    }
    // The source evaluates the current point's active interval set. A short
    // clip entirely skipped by a large delta never receives OnBehaviourPlay.
    let entered: Vec<_> = events
        .iter()
        .filter(|(key, _, _, _)| active.contains(key) && !session.active_events.contains(key))
        .collect();
    for (key, _, _, payload) in entered {
        match payload {
            TimelinePayload::Eye { .. } | TimelinePayload::Lip { .. } => {
                let eye = matches!(payload, TimelinePayload::Eye { .. });
                let handle = face_handle(
                    world,
                    &session.request,
                    &key.track,
                    if eye { "eye" } else { "mouth" },
                )?;
                let mut materials = world.resource_mut::<Assets<CharacterMaterial>>();
                let previous = materials
                    .get(&handle)
                    .ok_or_else(|| invalid("face material disappeared"))?;
                session
                    .prior_face
                    .entry(handle.clone())
                    .or_insert(PriorFace {
                        st: previous.params.main_tex_st,
                        lip_pattern: previous.lip_pattern,
                    });
                match payload {
                    TimelinePayload::Eye { open, .. } => {
                        apply_eye(&mut materials, &handle, Some(open))
                    }
                    TimelinePayload::Lip {
                        open,
                        middle,
                        close,
                        ..
                    } => apply_mouth_pattern(
                        &mut materials,
                        &handle,
                        LipPattern {
                            open: *open,
                            middle: *middle,
                            close: *close,
                        },
                    ),
                    _ => unreachable!(),
                };
            }
            TimelinePayload::Emoticon { name, use_root } => {
                let actor = session.request.bindings.actors[&key.track];
                let lease = crate::emoticon::timeline::show(world, actor, name, *use_root)
                    .map_err(invalid)?;
                session.emoticons.insert(key.clone(), lease);
            }
            TimelinePayload::Se { .. } => {
                // A silent SE has no sound; the timeline plays on without it.
                if session.request.bindings.silent_sounds.contains(key) {
                    continue;
                }
                let handle = session
                    .request
                    .bindings
                    .sounds
                    .get(key)
                    .ok_or_else(|| invalid("unprepared source SE"))?
                    .clone();
                let volume = world.resource::<VolumeBus>().se_ingame;
                let entity = world
                    .spawn((
                        AudioPlayer::new(handle),
                        PlaybackSettings::ONCE.with_volume(Volume::Linear(volume)),
                        BusVolume::Se(SeClass::Ingame),
                    ))
                    .id();
                session.sound_entities.push(entity);
            }
            _ => unreachable!(),
        }
    }
    session.active_events = active;
    Ok(())
}

/// `TimeNotificationBehaviour` over the tracks' signal emitters, on the
/// sampled time. `OnGraphStart` (the first tick) takes the start time as the
/// previous time. Each `PrepareFrame` (a playback frame, not a looped one:
/// this director never wraps, its backward jumps are loop-flag subtractions
/// and `MoveEndTime` writes) runs `TriggerNotificationsInRange(previous,
/// current, checkState true)`: nothing when current < previous; otherwise an
/// emitter not yet fired fires when previous <= time <= current (both ends
/// included), or, when retroactive, when time < current. Then every fired
/// emitter that is not emit-once is re-armed when current < previous and
/// current <= its time. `OnBehaviourPause` on a done playable fires the
/// unfired ones in [previous, duration]; the runner's last frame samples
/// the clamped duration, so that range is already covered.
/// Each signal goes to the receiver bound to its track (`OnNotify`: the
/// first row of the signal); its particle calls run through the particle
/// host ([`prepare_source_signals`]), the others are logged by name.
fn emit_signals(
    world: &mut World,
    session: &mut Session,
    time: f64,
) -> Result<(), TimelineFailure> {
    let previous = session.signal_time.replace(time).unwrap_or(time);
    let definition = session.request.definition.clone();
    let emitters = || {
        definition.tracks.iter().flat_map(|track| {
            track
                .markers
                .iter()
                .filter(|marker| marker.class == "SignalEmitter")
                .map(move |marker| (track, marker))
        })
    };
    for (track, marker) in emitters() {
        let due = previous <= time
            && ((previous <= marker.time && marker.time <= time)
                || (marker.retroactive && marker.time < time));
        if !due || !session.signals_fired.insert(marker.asset.clone()) {
            continue;
        }
        let name = marker.signal_name.as_deref().unwrap_or("?");
        let head = format!(
            "[timeline-signal] {}/{} track {} t={:.4}: SignalEmitter {} ({}, retroactive {}, emitOnce {})",
            definition.package,
            definition.prefab,
            track.name,
            marker.time,
            name,
            marker
                .signal
                .as_ref()
                .map_or("no signal asset".to_owned(), |s| s.path_id.clone()),
            marker.retroactive,
            marker.emit_once
        );
        match session
            .request
            .bindings
            .signal_receivers
            .get(&track.identity)
        {
            None => info!("{head}: the track is bound to no receiver; nothing is notified"),
            Some(binding) => {
                let row = binding
                    .reactions
                    .iter()
                    .find(|reaction| Some(&reaction.signal) == marker.signal.as_ref());
                let Some(reaction) = row else {
                    info!(
                        "{head}: receiver {} has no reaction to it",
                        binding.receiver
                    );
                    continue;
                };
                let calls = session
                    .request
                    .bindings
                    .signals
                    .get(&marker.asset)
                    .cloned()
                    .unwrap_or_default();
                info!(
                    "{head}: receiver {} reacts with {}; {} particle calls prepared",
                    binding.receiver,
                    reaction.calls_text(),
                    calls.len()
                );
                for call in &calls {
                    let count = crate::fixture_timeline_particles::react(world, call)
                        .map_err(|error| source_effect_failure(invalid(error)))?;
                    let stopped = call.call == crate::fixture_timeline_particles::SignalCall::Stop;
                    match session
                        .signal_played
                        .iter_mut()
                        .find(|(root, _)| *root == call.target.root)
                    {
                        Some(row) => row.1 = stopped,
                        None if !stopped => session.signal_played.push((call.target.root, false)),
                        None => {}
                    }
                    info!(
                        "{head}: director time {time:.4}: {} (marker {:.4} s, emitOnce {}): ParticleSystem.{:?}() on {:?} through the particle host: {count} systems",
                        call.signal,
                        call.time,
                        call.emit_once,
                        call.call,
                        world.get::<Name>(call.target.root).map(Name::as_str)
                    );
                }
            }
        }
    }
    if time < previous {
        for (track, marker) in emitters() {
            if !marker.emit_once
                && time <= marker.time
                && session.signals_fired.remove(&marker.asset)
            {
                info!(
                    "[timeline-signal] {}/{} track {} t={:.4}: SignalEmitter {} re-armed (the director went back from {previous:.4} to {time:.4})",
                    definition.package,
                    definition.prefab,
                    track.name,
                    marker.time,
                    marker.signal_name.as_deref().unwrap_or("?")
                );
            }
        }
    }
    Ok(())
}

/// The live particles of the objects a signal played in this session: every
/// frame while no signal stopped them, then four times a real second while
/// they hold any (a trace of the particle host's state, read from its
/// systems).
fn sample_signal_targets(world: &mut World, session: &mut Session, time: f64) {
    if session.signal_played.is_empty() {
        return;
    }
    let now = crate::fixture_timeline_particles::realtime(world);
    let emitting = session.signal_played.iter().any(|(_, stopped)| !stopped);
    if !emitting && now - session.signal_sampled < 0.25 {
        return;
    }
    session.signal_sampled = now;
    for &(root, _) in &session.signal_played {
        let (systems, live, born) = played_particles(world, root);
        if live == 0 {
            continue;
        }
        info!(
            "[timeline-signal] {}/{} director time {time:.4}: {:?} systems {systems} live {live} born {born}",
            session.request.definition.package,
            session.request.definition.prefab,
            world.get::<Name>(root).map(Name::as_str)
        );
    }
}

/// The systems the particle host drew under a played object: how many, their
/// live particles and their births so far.
pub(crate) fn played_particles(world: &World, root: Entity) -> (usize, usize, u64) {
    let Some(children) = world.get::<Children>(root) else {
        return (0, 0, 0);
    };
    let mut systems = 0;
    let mut live = 0;
    let mut born = 0;
    for child in children.iter() {
        if let Some(system) = world.get::<crate::uber_particle::FixtureParticleLive>(child) {
            systems += 1;
            live += system.0.pool.len();
            born += system.0.born_total;
        }
    }
    (systems, live, born)
}

fn update_facial_gates(world: &mut World, token: TimelineToken, session: &mut Session, time: f64) {
    let mut gates: HashMap<Entity, (bool, Option<bool>, bool)> = HashMap::new();
    for track in &session.request.definition.tracks {
        if track
            .clips
            .iter()
            .any(|clip| matches!(clip.payload, TimelinePayload::LipGate))
        {
            let actor = session.request.bindings.actors[&track.identity];
            // Each source mixer calls its setter on every ProcessFrame. OR
            // active clips within this track, but let a later track assignment
            // replace an earlier one rather than merging distinct setters.
            // An empty clip/curve track has no compiled mixer; its mere
            // presence in exported metadata must not disable an actor.
            gates.entry(actor).or_default().1 = Some(track.clips.iter().any(|clip| {
                matches!(clip.payload, TimelinePayload::LipGate) && clip.contains(time)
            }));
        }
        for clip in &track.clips {
            if matches!(
                clip.payload,
                TimelinePayload::BlinkGate
                    | TimelinePayload::LipGate
                    | TimelinePayload::NpcIkTalkGate
            ) {
                let actor = session.request.bindings.actors[&track.identity];
                let state = gates.entry(actor).or_default();
                match clip.payload {
                    TimelinePayload::BlinkGate => state.0 |= clip.contains(time),
                    TimelinePayload::LipGate => {}
                    TimelinePayload::NpcIkTalkGate => {
                        state.2 |= matches!(
                            session.request.owner.kind,
                            TimelineOwnerKind::Npc | TimelineOwnerKind::Talk
                        ) && clip.contains(time)
                    }
                    _ => {}
                }
            }
        }
    }
    for (actor, (blink_gate, lip_gate, npc_ik_talk_gate)) in gates {
        session
            .prior_gates
            .entry(actor)
            .or_insert_with(|| world.get::<TimelineFacialState>(actor).cloned());
        if world.entities().contains(actor) {
            world.entity_mut(actor).insert(TimelineFacialState {
                token,
                blink_gate,
                lip_gate,
                npc_ik_talk_gate,
            });
        }
    }
}

fn cleanup(world: &mut World, token: TimelineToken, session: &mut Session) {
    effects::release(world, token, &mut session.effects);
    for (_, lease) in session.emoticons.drain() {
        crate::emoticon::timeline::hide(world, lease, true);
    }
    let mut animators = HashSet::new();
    for node in &session.nodes {
        if world
            .get::<TimelinePlaybackOwner>(node.animator)
            .is_some_and(|p| p.0 == token)
        {
            if let Some(mut player) = world.get_mut::<AnimationPlayer>(node.animator) {
                player.stop(node.node);
            }
            let binding = &session.request.bindings.animations[&node.key];
            if let Some(graph) = world
                .resource_mut::<Assets<AnimationGraph>>()
                .get_mut(&binding.graph)
            {
                if let Some(slot) = graph.graph.node_weight_mut(node.node) {
                    slot.node_type = AnimationNodeType::Blend;
                }
            }
            animators.insert(node.animator);
        }
    }
    for animator in &animators {
        world
            .entity_mut(*animator)
            .remove::<TimelinePlaybackOwner>();
    }
    session.nodes.clear();
    for (entity, prior) in session.prior_pose.drain(..) {
        if !world
            .get::<bevy::animation::AnimatedBy>(entity)
            .is_some_and(|by| animators.contains(&by.0))
        {
            continue;
        }
        if let Some(mut pose) = world.get_mut::<Transform>(entity) {
            *pose = prior;
        }
    }
    for entity in session.sound_entities.drain(..) {
        if world.entities().contains(entity) {
            world.despawn(entity);
        }
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<CharacterMaterial>>() {
        for (handle, prior) in session.prior_face.drain() {
            if let Some(material) = materials.get_mut(&handle) {
                material.params.main_tex_st = prior.st;
                if material.lip_pattern != prior.lip_pattern {
                    material.lip_pattern = prior.lip_pattern;
                    material.lip_pattern_revision = material.lip_pattern_revision.wrapping_add(1);
                }
            }
        }
    }
    for (actor, prior) in session.prior_gates.drain() {
        if world
            .get::<TimelineFacialState>(actor)
            .is_some_and(|state| state.token == token)
        {
            if let Some(prior) = prior {
                world.entity_mut(actor).insert(prior);
            } else {
                world.entity_mut(actor).remove::<TimelineFacialState>();
            }
        }
    }
}

#[cfg(test)]
mod handoff_regressions {
    use super::*;

    /// A two-member NPC fixture talk: the owner (unit 1) and a member track
    /// named after unit 3, as the multiple-character Director of talk 5518
    /// is authored.
    fn npc_cast(
        world: &mut World,
        member_unit: u32,
        lease: Option<u64>,
    ) -> (StartTimeline, SourceAssetId, Entity) {
        let owner = FixtureActivityOwner {
            actor: world.spawn(crate::npc::CharacterUnitId(1)).id(),
            generation: 7,
        };
        let member = world.spawn(crate::npc::CharacterUnitId(member_unit)).id();
        if let Some(generation) = lease {
            world
                .entity_mut(member)
                .insert(crate::npc_fixture_activity::NpcFixtureMotionOwner(
                    FixtureActivityOwner {
                        actor: owner.actor,
                        generation,
                    },
                ));
        }
        let track = SourceAssetId {
            file: "director".into(),
            path_id: "3".into(),
        };
        let request = StartTimeline {
            owner: TimelineOwner {
                activity: owner,
                kind: TimelineOwnerKind::Npc,
            },
            fixture: world.spawn_empty().id(),
            definition: Arc::new(TimelineDefinition {
                package: "fixture".into(),
                prefab: "fixture".into(),
                fixture_view: None,
                director: track.clone(),
                timeline: track.clone(),
                duration: 1.,
                tracks: vec![TimelineTrack {
                    identity: track.clone(),
                    class: "AnimationTrack".into(),
                    name: "3".into(),
                    clips: vec![],
                    markers: vec![],
                    settings: Value::Null,
                    infinite: None,
                }],
            }),
            bindings: TimelineBindings::default(),
            companions: vec![],
            timeout_secs: 30.,
            timeout_budget: TimelineTimeoutBudget::OwnerGated {
                advance: Some(true),
            },
        };
        (request, track, member)
    }

    fn refusal(
        world: &World,
        request: &StartTimeline,
        track: &SourceAssetId,
        member: Entity,
    ) -> String {
        validate_track_actor(world, request, track, member)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn npc_member_leased_to_the_activity_drives_its_unit_track() {
        let mut world = World::new();
        let (request, track, member) = npc_cast(&mut world, 3, Some(7));
        assert!(validate_track_actor(&world, &request, &track, member).is_ok());
        let owner = request.owner.activity.actor;
        assert!(validate_track_actor(&world, &request, &track, owner).is_ok());
    }

    #[test]
    fn npc_member_track_of_another_unit_is_refused() {
        let mut world = World::new();
        let (request, track, member) = npc_cast(&mut world, 4, Some(7));
        assert!(refusal(&world, &request, &track, member).contains("bound to a different unit"));
    }

    #[test]
    fn npc_member_without_a_lease_is_refused() {
        let mut world = World::new();
        let (request, track, member) = npc_cast(&mut world, 3, None);
        assert!(refusal(&world, &request, &track, member).contains("holds no fixture lease"));
    }

    #[test]
    fn npc_member_leased_to_another_activity_is_refused() {
        let mut world = World::new();
        let (request, track, member) = npc_cast(&mut world, 3, Some(8));
        assert!(refusal(&world, &request, &track, member).contains("belongs to another activity"));
    }

    #[test]
    fn player_timeline_stays_bound_to_its_owner() {
        let mut world = World::new();
        let (mut request, track, member) = npc_cast(&mut world, 3, Some(7));
        request.owner.kind = TimelineOwnerKind::Player;
        assert!(
            refusal(&world, &request, &track, member).contains("differs from the activity owner")
        );
    }

    #[test]
    fn retired_actor_library_does_not_drop_the_consumers_selected_clip() {
        let libraries = Assets::<Gltf>::default();
        let clips = Assets::<AnimationClip>::default();
        let library = libraries.reserve_handle();
        let selected_clip = clips.reserve_handle();
        let library_weak = match &library {
            Handle::Strong(handle) => Arc::downgrade(handle),
            _ => unreachable!(),
        };
        let clip_weak = match &selected_clip {
            Handle::Strong(handle) => Arc::downgrade(handle),
            _ => unreachable!(),
        };
        let mut loads = TimelineAssetLoads::default();
        loads
            .gltf
            .insert("actor-animations/library.glb".into(), library);
        loads.sweep_lookup_caches();
        assert!(library_weak.upgrade().is_some());
        loads.sweep_lookup_caches();
        assert!(library_weak.upgrade().is_none());
        assert!(clip_weak.upgrade().is_some());
        drop(selected_clip);
        assert!(clip_weak.upgrade().is_none());
    }

    #[test]
    fn destroyed_fixture_releases_cancelled_session_and_its_clip_handles() {
        let mut world = World::new();
        world.init_resource::<Assets<AnimationClip>>();
        let clip = world
            .resource_mut::<Assets<AnimationClip>>()
            .add(AnimationClip::default());
        let weak = match &clip {
            Handle::Strong(handle) => Arc::downgrade(handle),
            _ => panic!("test clip must own a strong handle"),
        };
        let actor = world.spawn_empty().id();
        let fixture = world.spawn_empty().id();
        let identity = SourceAssetId {
            file: "source".into(),
            path_id: "1".into(),
        };
        let key = TimelineClipKey {
            track: identity.clone(),
            clip_index: 0,
        };
        let mut runtime = FixtureActivityTimelines::default();
        let token = runtime.request_start(StartTimeline {
            owner: TimelineOwner {
                activity: FixtureActivityOwner {
                    actor,
                    generation: 1,
                },
                kind: TimelineOwnerKind::Player,
            },
            fixture,
            definition: Arc::new(TimelineDefinition {
                package: "fixture".into(),
                prefab: "fixture".into(),
                fixture_view: None,
                director: identity.clone(),
                timeline: identity.clone(),
                duration: 1.,
                tracks: vec![],
            }),
            bindings: TimelineBindings {
                animations: HashMap::from([(
                    key,
                    TimelineAnimationBinding {
                        animator: actor,
                        graph: Handle::default(),
                        clip,
                        source: SourceAnimationEvidence {
                            package: "fixture".into(),
                            clip_name: "clip".into(),
                            asset: identity,
                            start_time: 0.,
                            stop_time: 1.,
                            looping: false,
                        },
                        coverage: AnimationCoverage::sampled_pose(),
                    },
                )]),
                ..Default::default()
            },
            companions: vec![],
            timeout_secs: 5.,
            timeout_budget: TimelineTimeoutBudget::PlayerWall,
        });
        world.insert_resource(runtime);
        world.despawn(fixture);
        advance(&mut world);
        assert!(world
            .resource::<FixtureActivityTimelines>()
            .status(token)
            .is_none());
        assert!(weak.upgrade().is_none());
        assert!(world.entities().contains(actor));
    }

    #[test]
    fn completed_director_release_removes_root_yaw_before_sparse_hips_talk() {
        let mut world = World::new();
        world.init_resource::<Assets<AnimationGraph>>();
        world.init_resource::<Assets<AnimationClip>>();
        let clip = world
            .resource_mut::<Assets<AnimationClip>>()
            .add(AnimationClip::default());
        let mut graph = AnimationGraph::new();
        let node = graph.add_clip(clip.clone(), 1., graph.root);
        let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
        let animator = world.spawn(AnimationPlayer::default()).id();
        let actor = world.spawn_empty().id();
        let fixture = world.spawn_empty().id();
        let root = world
            .spawn((
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                AnimatedBy(animator),
            ))
            .id();
        let hips = world
            .spawn((Transform::from_xyz(0., 0.5, 0.), AnimatedBy(animator)))
            .id();
        let id = SourceAssetId {
            file: "source".into(),
            path_id: "1".into(),
        };
        let key = TimelineClipKey {
            track: id.clone(),
            clip_index: 0,
        };
        let mut timelines = FixtureActivityTimelines::default();
        let token = timelines.request_start(StartTimeline {
            owner: TimelineOwner {
                activity: FixtureActivityOwner {
                    actor,
                    generation: 1,
                },
                kind: TimelineOwnerKind::Talk,
            },
            fixture,
            definition: Arc::new(TimelineDefinition {
                package: "horse2".into(),
                prefab: "horse2".into(),
                fixture_view: None,
                director: id.clone(),
                timeline: id.clone(),
                duration: 7.,
                tracks: vec![],
            }),
            bindings: TimelineBindings {
                animations: HashMap::from([(
                    key.clone(),
                    TimelineAnimationBinding {
                        animator,
                        graph: graph.clone(),
                        clip,
                        source: SourceAnimationEvidence {
                            package: "horse2".into(),
                            clip_name: "end".into(),
                            asset: id,
                            start_time: 0.,
                            stop_time: 7.,
                            looping: false,
                        },
                        coverage: AnimationCoverage::sampled_pose(),
                    },
                )]),
                ..Default::default()
            },
            companions: vec![],
            timeout_secs: 30.,
            timeout_budget: TimelineTimeoutBudget::PlayerWall,
        });
        let session = timelines.sessions.get_mut(&token).unwrap();
        session.status = TimelineStatus::Completed;
        session.nodes.push(BoundNode {
            key,
            animator,
            node,
        });
        session.prior_pose.push((root, Transform::IDENTITY));
        session
            .prior_pose
            .push((hips, Transform::from_xyz(0., 0.5, 0.)));
        world
            .entity_mut(animator)
            .insert(TimelinePlaybackOwner(token));
        world
            .get_mut::<AnimationPlayer>(animator)
            .unwrap()
            .play(node)
            .pause();
        world.insert_resource(timelines);

        cancel_and_release(&mut world, token);
        // A sparse shared talk clip writes Hips only. It must not inherit the
        // previous Director's final 180-degree Root rotation.
        world.get_mut::<Transform>(hips).unwrap().rotation = Quat::from_rotation_x(0.1);
        let wrapper = Quat::from_rotation_y(0.138);
        let visible_forward = wrapper * world.get::<Transform>(root).unwrap().rotation * Vec3::Z;
        assert!(visible_forward.dot(wrapper * Vec3::Z) > 0.9999);
        assert!(world
            .resource::<FixtureActivityTimelines>()
            .status(token)
            .is_none());
        assert!(world.get::<TimelinePlaybackOwner>(animator).is_none());
        assert!(world
            .get::<AnimationPlayer>(animator)
            .unwrap()
            .playing_animations()
            .next()
            .is_none());
        assert!(matches!(
            world
                .resource::<Assets<AnimationGraph>>()
                .get(&graph)
                .unwrap()
                .graph[node]
                .node_type,
            AnimationNodeType::Blend
        ));
    }
}
