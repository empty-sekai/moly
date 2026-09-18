//! One source director clock for actual furniture/actor animation, facial
//! presets and source-qualified SE. Admission, slots, navigation and final
//! actor-state restoration belong to the activity owner.
//!
//! A renderable sampled pose is distinct from native humanoid equivalence.
//! Foot IK, start offsets, track matching and mixer differences remain
//! readable through `coverage`; they are never inferred from clip names.

mod actor_space;
mod clock;
mod source;

pub(crate) use source::{
    AnimationPlayableSettings, BlendCurve, ClipTarget, SourceAssetId, TimelineClip,
    TimelineClipKey, TimelineDefinition, TimelinePackage, TimelinePayload, TimelineTrack,
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
struct TimelineAssetLoads {
    json: HashMap<String, Handle<JsonAsset>>,
    // One successful parse per currently observed source text. This caches
    // metadata syntax only, never a prepared actor/fixture binding or its
    // liveness verdict. The original text makes invalidation collision-free.
    //
    // 第一格是「校验这条缓存时 `Assets<JsonAsset>` 的 last_changed」。代没变
    // 就说明期间没有任何 json 资产被动过，这条缓存必然仍成立——省掉一次
    // 对整份文档的逐字节比对。代不同时走原来的比对，判定结果不变。
    parsed_json: HashMap<String, (Option<Tick>, String, Arc<Value>)>,
    gltf: HashMap<String, Handle<Gltf>>,
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
    if world.get::<crate::npc::CharacterUnitId>(actor).map(|id| id.0) != Some(unit_id) {
        return Err(invalid("source actor track unit differs from admitted actor"));
    }
    prepare_actor_tracks(world, request, unit_id, actor, animator, graph, true)
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
    let catalog = load_json(world, &server, "actor-animations/index.json")?;
    if catalog["version"].as_u64() != Some(1) {
        return Err(invalid("unsupported actor animation catalog"));
    }
    let mut prepared = Vec::new();
    for track in &request.definition.tracks {
        if track.class != "AnimationTrack" || if explicit_cast {
            track.name.parse::<u32>().ok() != Some(unit_id)
        } else {
            track.name != "CharacterAnimator"
        } {
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
            let routes: Vec<_> = array(&catalog, "clipBindings")?
                .iter()
                .filter(|row| {
                    row["unitId"].as_u64() == Some(unit_id as u64)
                        && asset(&row["sourceClip"]).ok().as_ref() == Some(source_clip)
                })
                .collect();
            if routes.len() != 1 {
                return Err(invalid(
                    "actor source clip is missing or ambiguous in the formal catalog",
                ));
            }
            let route = routes[0];
            let route_identity = asset(&route["sourceClip"])?;
            let exported_name = string(route, "clipName")?;
            let library = array(&catalog, "libraries")?
                .get(source::unsigned(route, "library")? as usize)
                .ok_or_else(|| invalid("actor library route out of range"))?;
            if library["unitId"].as_u64() != Some(unit_id as u64) {
                return Err(invalid("actor library unit mismatch"));
            }
            let index = load_json(world, &server, string(library, "index")?)?;
            if index["version"].as_u64() != Some(2) {
                return Err(invalid("source-qualified v2 actor index required"));
            }
            for field in ["sourceRootName", "bindingRootName"] {
                if string(&index["binding"], field)? != string(library, field)? {
                    return Err(invalid("actor binding root route mismatch"));
                }
            }
            let groups = index["clips"]
                .as_object()
                .ok_or_else(|| invalid("actor index clips missing"))?;
            let segments: Vec<_> = groups
                .values()
                .filter_map(|group| group["segments"].as_object())
                .flat_map(|segments| segments.values())
                .filter(|entry| {
                    entry["name"].as_str() == Some(exported_name)
                        && asset(&entry["sourceClip"]).ok().as_ref() == Some(&route_identity)
                })
                .collect();
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
                    .entry(path.into())
                    .or_insert_with(|| server.load(format!("moly://{path}")))
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
        let single_face = !explicit_cast && track.clips.iter().any(|clip| matches!(clip.payload,
            TimelinePayload::Eye { .. } | TimelinePayload::Lip { .. } |
            TimelinePayload::BlinkGate | TimelinePayload::LipGate |
            TimelinePayload::NpcIkTalkGate | TimelinePayload::Emoticon { .. }));
        if named_cast || single_body || single_face {
            request.bindings.actors.insert(track.identity.clone(), actor);
        }
    }
    Ok(())
}

fn require_actor_path(path: &str) -> Result<(), TimelineFailure> {
    if !path.starts_with("actor-animations/")
        || path.contains(':')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(invalid("actor catalog path is not asset-root-relative"));
    }
    Ok(())
}

fn load_json(
    world: &mut World,
    server: &AssetServer,
    path: &str,
) -> Result<Arc<Value>, TimelineFailure> {
    require_actor_path(path)?;
    let handle = {
        let mut loads = world.resource_mut::<TimelineAssetLoads>();
        loads
            .json
            .entry(path.into())
            .or_insert_with(|| server.load(format!("moly://{path}")))
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
    if let Some((validated_at, _, parsed)) = world
        .resource::<TimelineAssetLoads>()
        .parsed_json
        .get(path)
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
        if let Some((_, text, parsed)) = world.resource::<TimelineAssetLoads>().parsed_json.get(path)
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
    pub sounds: HashMap<TimelineClipKey, Handle<AudioSource>>,
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
    prior_face: HashMap<Handle<CharacterMaterial>, PriorFace>,
    prior_gates: HashMap<Entity, Option<TimelineFacialState>>,
    sound_entities: Vec<Entity>,
    emoticons: HashMap<TimelineClipKey, crate::emoticon::timeline::EmoteLease>,
    actor_space: Vec<actor_space::ActorSpaceLease>,
    coverage: Vec<TimelineCoverageGap>,
    cancel: bool,
    release: bool,
    initialized: bool,
    timeout_elapsed: f64,
    timeout_requested: bool,
    completion: Option<TimelineCompletionReason>,
    active_events: HashSet<TimelineClipKey>,
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
        self.sessions.get(&token).is_some_and(|session| !session.request.definition.tracks.iter()
            .flat_map(|track| &track.clips).any(|clip| matches!(clip.payload, TimelinePayload::LoopFlag { .. })))
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
                prior_face: HashMap::new(),
                prior_gates: HashMap::new(),
                sound_entities: Vec::new(),
                emoticons: HashMap::new(),
                actor_space: Vec::new(),
                coverage: Vec::new(),
                cancel: false,
                release: false,
                initialized: false,
                timeout_elapsed: 0.0,
                timeout_requested: false,
                completion: None,
                active_events: HashSet::new(),
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
    let Some(mut timelines) = world.remove_resource::<FixtureActivityTimelines>() else { return; };
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

/// Load exact `(package,cue)` resources through the existing audio Routing
/// and AssetServer. Calling this does not start audio or admit an action.
pub(crate) fn prepare_source_sounds(
    world: &World,
    request: &mut StartTimeline,
) -> Result<(), TimelineFailure> {
    let routing = world
        .get_resource::<Routing>()
        .ok_or_else(|| TimelineFailure::loading("audio routing not ready"))?;
    let server = world
        .get_resource::<AssetServer>()
        .ok_or_else(|| invalid("asset server missing"))?;
    let mut sounds = HashMap::new();
    for track in all_tracks(request)? {
        for clip in &track.clips {
            if let TimelinePayload::Se { package, cue } = &clip.payload {
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

pub(crate) fn validate_start(
    world: &World,
    request: &StartTimeline,
) -> Result<(), TimelineFailure> {
    validate(world, request, None)
}

fn validate(
    world: &World,
    request: &StartTimeline,
    own: Option<TimelineToken>,
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
        (TimelineOwnerKind::Player, TimelineTimeoutBudget::PlayerWall)
        | (TimelineOwnerKind::Npc | TimelineOwnerKind::Talk, TimelineTimeoutBudget::OwnerGated { advance: Some(_) }) => {}
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
                    if world
                        .get::<TimelinePlaybackOwner>(binding.animator)
                        .is_some_and(|p| Some(p.0) != own)
                    {
                        return Err(invalid("animator already belongs to another timeline"));
                    }
                    let expected_root = if let Some(actor) = request.bindings.actors.get(&track.identity) {
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
                    required_sounds.insert(clip.key.clone());
                    let routing = world
                        .get_resource::<Routing>()
                        .ok_or_else(|| TimelineFailure::loading("audio routing not ready"))?;
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
                    let archive = world
                        .get_resource::<crate::emoticon::EmoticonArchive>()
                        .ok_or_else(|| {
                            TimelineFailure::loading("emoticon archive still loading")
                        })?;
                    if !world.contains_resource::<crate::emoticon::Emotes>() {
                        return Err(TimelineFailure::loading("emoticon renderer still loading"));
                    }
                    if !archive.has(name) {
                        return Err(invalid(format!("source emoticon {name} is unavailable")));
                    }
                }
                TimelinePayload::LoopFlag { .. } => loop_count += 1,
                TimelinePayload::NoPresetChange => {}
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
    Ok(())
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

/// Explicit cast bindings are accepted only when the source track identifies
/// that exact unit and the actual entity belongs to the current admitted talk.
/// The single-player/NPC owner contract is otherwise unchanged.
fn validate_track_actor(
    world: &World, request: &StartTimeline, identity: &SourceAssetId, actor: Entity,
) -> Result<(), TimelineFailure> {
    if request.owner.kind != TimelineOwnerKind::Talk {
        return if actor == request.owner.activity.actor { Ok(()) }
            else { Err(invalid("track actor differs from the activity owner")) };
    }
    let track = request.definition.tracks.iter().find(|track| &track.identity == identity)
        .ok_or_else(|| invalid("actor track is outside the selected source director"))?;
    let common_single = request.definition.tracks.iter().any(|track| track.name == "CharacterAnimator")
        && world.get_resource::<crate::talk::ActiveTalk>().is_some_and(|talk| talk.participants().len() == 1);
    if common_single && actor == request.owner.activity.actor { return Ok(()); }
    let unit = track.name.parse::<u32>().map_err(|_| invalid("cast track has no exact source unit name"))?;
    if world.get::<crate::npc::CharacterUnitId>(actor).map(|id| id.0) != Some(unit) {
        return Err(invalid("cast track is bound to a different source unit"));
    }
    let admitted = world.get_resource::<crate::talk::ActiveTalk>().is_some_and(|talk|
        talk.participants().iter().any(|(candidate, entity)| *candidate == unit && *entity == actor));
    if !admitted { return Err(invalid("cast track actor is outside the admitted talk")); }
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
    validate(world, &session.request, Some(token))?;
    // The lookup cache owns only IDs, so a destroyed body graph cannot keep
    // its asset alive. Drop those obsolete keys when a new session starts;
    // repeated body rebuilds must not retain their metadata indefinitely.
    // Keep this out of the per-frame sampling path.
    {
        let graphs = world.resource::<Assets<AnimationGraph>>();
        cache.retain(|(graph, _, _), _| graphs.get(*graph).is_some());
    }
    // Every admitted body needs its own coordinate-space lease, while all
    // tracks still sample ONE Director clock. Never clone/retime the timeline.
    let mut actors = HashSet::new();
    for track in &session.request.definition.tracks {
        if track.clips.iter().any(|clip| matches!(clip.payload, TimelinePayload::Animation { .. })) {
            if let Some(actor) = session.request.bindings.actors.get(&track.identity) {
                actors.insert(*actor);
            } else if track.name == "CharacterAnimator" {
                actors.insert(session.request.owner.activity.actor);
            }
        }
    }
    let mut actors: Vec<_> = actors.into_iter().collect();
    actors.sort();
    for actor in actors {
        session.actor_space.push(actor_space::acquire(world, actor, token)?);
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
                    let node = graph.add_clip(binding.clip.clone(), 1.0, graph.root);
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
    validate(world, &session.request, Some(token))?;
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
    if budget_expired && matches!(session.request.owner.kind, TimelineOwnerKind::Npc | TimelineOwnerKind::Talk) {
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
    apply_events(world, session, sampled_time)?;
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
            enable_talk: matches!(session.request.owner.kind, TimelineOwnerKind::Npc | TimelineOwnerKind::Talk)
                && session.clock.enable_talk,
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
                        state.2 |= matches!(session.request.owner.kind, TimelineOwnerKind::Npc | TimelineOwnerKind::Talk)
                            && clip.contains(time)
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
    for lease in session.actor_space.drain(..) {
        actor_space::restore(world, token, lease);
    }
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
            animators.insert(node.animator);
        }
    }
    for animator in animators {
        world.entity_mut(animator).remove::<TimelinePlaybackOwner>();
    }
    session.nodes.clear();
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
