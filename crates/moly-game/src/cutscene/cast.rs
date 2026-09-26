//! A cut-scene played with a cast at a fixture (`CutSceneExecutor.PlayAsync`
//! with character unit ids and the fixture's view transform): the gate's
//! invite and go-home cut-scenes.
//!
//! `CutScenePresenter.Setup` binds each unit: with `useAlreadyExistCharacter`
//! a present NPC is taken over (`TryCancelCurrentObjective`, then
//! `ForceUpdateCutSceneObjective` and `SetImmediatelyExecuteNextObjective`);
//! otherwise a new avatar is created (`CreateAndSetupNPCAvatarView`,
//! `CreateNPCAvatarForMulti`). The NPCs not bound are hidden, the player too
//! when `hidePlayer`; `HideNearFixtures` hides every fixture other than the
//! gate and the home within `_hideFixtureDistance` of the gate (3-D);
//! `ChangeParentCharacters` puts the bound characters under the view's
//! `_characterRoot` keeping their local poses. `CutSceneView` binds the
//! director's outputs whose stream name is the unit id to that character
//! (Animator, the character component, its GameObject), the `Gate` Animator
//! output to the start transform's Animator when that object's name holds
//! `gate`, and the IK list by its `mdl_sd` bind names.
//!
//! Tracks driven here from the runner's clock:
//! - a unit's recorded clip (`AnimationTrack.m_InfiniteClip`): the
//!   character's root local pose, the track's infinite-clip offset applied
//!   (transform offsets: position `offsetRot * clip + offset`, rotation
//!   `offsetRot * clip`), Euler angles in Unity's Z, X, Y order;
//! - an activation track: the character is active while a clip has
//!   weight, and at the graph's destruction its post-playback state applies;
//! - the player activation track: the player is shown on a clip's play and
//!   hidden on its pause while the director is short of its duration;
//! - the skip track: named (its tap input is the cut-scene screen's).
//!
//! The unit's motion clips, eye and lip-sync presets, the gate's clips, the
//! SE and the Control clips are the runner's: the motion clips from the
//! unit's actor library by source identity, the gate's from the placed
//! gate's model file, the Control clips on the view's nodes (spawned from the
//! package's particle document) through the particle host.
//!
//! `EndAsync`: `RestoreStates` (the bound NPCs back to Idle),
//! `ChangeParentCharacters` back, the view disposed, the caller's end
//! callback, the NPCs shown, the fixtures shown, the player shown, then the
//! fade out with the view's end colour.
//!
//! Named differences: a new avatar is created while the package loads (the
//! runner binds its animator before the director starts), hidden until the
//! director's first frame, not after the fade in; its NPC objective is not
//! started (the product's NPC objective does not stop under the cut-scene
//! state); the taken-over NPC's objective calls are the NPC runtime's and are
//! named only; humanoid foot IK and the motion clips' own root motion are
//! not applied.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::{RefreshSourceActivity, SourceNodeActivity};
use serde_json::Value;

use crate::fixture_activity_provider::FixtureActivityProvider;
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner};
use crate::fixture_activity_timeline::{
    self as timeline, CutScenePayload, InfiniteClip, SourceAssetId, StartTimeline, TimelineClipKey,
    TimelineDefinition, TimelinePayload,
};
use crate::npc::{CharacterUnitId, MotionClips, MotionPhase};

/// The cut-scene holds this object for its director: a cast member (its
/// animator, root pose and activity) or the fixture a track binds.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct CutSceneCastLease(pub(crate) FixtureActivityOwner);

/// An avatar the presenter created for a unit that was not present.
#[derive(Component)]
pub(crate) struct CutSceneAvatar;

/// Who called `CutSceneExecutor.PlayAsync` with a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CastCaller {
    /// `ScreenLayerMysekaiGateInvitationPresenter.PlayInviteCutSceneAsync`.
    Invite,
    /// `MysekaiGateUtility.TryShowGoHomeCutSceneAsync`.
    GoHome,
}

impl CastCaller {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Invite => "PlayInviteCutSceneAsync",
            Self::GoHome => "TryShowGoHomeCutSceneAsync",
        }
    }
}

/// `CutSceneExecutor.PlayAsync(characterUnitIds, cutSceneId, startTransform,
/// useAlreadyExistCharacter, camera, hidePlayer, showUI, ...)`.
#[derive(Clone, Debug)]
pub(crate) struct CastPlay {
    pub(crate) caller: CastCaller,
    pub(crate) units: Vec<u32>,
    pub(crate) cut_scene_id: i64,
    /// The master row's `timelineAssetbundleName`.
    pub(crate) timeline: String,
    /// The start transform's object (the placed gate).
    pub(crate) start: Entity,
    pub(crate) use_already_exist_character: bool,
    pub(crate) hide_player: bool,
    pub(crate) show_ui: bool,
}

struct Member {
    unit: u32,
    entity: Entity,
    taken_over: bool,
    /// The member's visibility before the cut-scene.
    prior_visibility: Visibility,
}

/// What the view record says beyond the presenter's lists.
#[derive(Clone, Debug, Default)]
struct ViewFields {
    start_color: [f32; 4],
    end_color: [f32; 4],
    /// The prefab root GameObject and its serialized file.
    root: Option<(String, i64)>,
    /// `_characterRoot`'s GameObject.
    character_root: Option<i64>,
    ik_list: usize,
}

/// A cast cut-scene from its load to the caller's return.
pub(crate) struct Cast {
    pub(crate) play: CastPlay,
    registry: Option<Handle<JsonAsset>>,
    members: Vec<Member>,
    view: Option<ViewFields>,
    placed: bool,
    nodes_spawned: bool,
    character_root: Option<Entity>,
    /// The fixture `BindGateAnimator` binds, when the start is a gate.
    gate: Option<Entity>,
    /// Tracks this owner left out of the director, each with the reason.
    refused: Vec<(SourceAssetId, String)>,
    hidden_npcs: Vec<(Entity, Visibility)>,
    hidden_player: Vec<(Entity, Visibility)>,
    hidden_fixtures: Vec<Entity>,
    /// Activation tracks active on the last frame.
    active_tracks: HashSet<SourceAssetId>,
    /// Player activation and skip clips active on the last frame.
    active_clips: HashSet<TimelineClipKey>,
    /// Real time of the last recorded-pose report, by member.
    pose_logged: HashMap<u32, f64>,
    /// Real time of the last character-clip report, by member.
    clips_logged: HashMap<u32, f64>,
    /// The director time of the last frame.
    last_time: f64,
    pub(crate) outcome: Option<Result<(), String>>,
}

impl Cast {
    pub(crate) fn new(play: CastPlay) -> Self {
        Self {
            play,
            registry: None,
            members: Vec::new(),
            view: None,
            placed: false,
            nodes_spawned: false,
            character_root: None,
            gate: None,
            refused: Vec::new(),
            hidden_npcs: Vec::new(),
            hidden_player: Vec::new(),
            hidden_fixtures: Vec::new(),
            active_tracks: HashSet::new(),
            active_clips: HashSet::new(),
            pose_logged: HashMap::new(),
            clips_logged: HashMap::new(),
            last_time: f64::NEG_INFINITY,
            outcome: None,
        }
    }

    /// The view's start and end fade colours (`_startFadeColor`,
    /// `_endFadeColor`) once read.
    pub(crate) fn fade_colors(&self) -> ([f32; 4], [f32; 4]) {
        self.view.as_ref().map_or(([0.0; 4], [0.0; 4]), |view| {
            (view.start_color, view.end_color)
        })
    }

    pub(crate) fn avatars(&self) -> Vec<(u32, Entity)> {
        self.members
            .iter()
            .filter(|member| !member.taken_over)
            .map(|member| (member.unit, member.entity))
            .collect()
    }

    pub(crate) fn start(&self) -> Entity {
        self.play.start
    }
}

fn color(value: &Value) -> [f32; 4] {
    ["r", "g", "b", "a"].map(|key| value[key].as_f64().unwrap_or(0.0) as f32)
}

fn read_view(document: &Value, prefab: &str) -> Result<ViewFields, String> {
    let wanted = format!("/{prefab}.prefab");
    let record = document["prefabs"]
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["container"]
                    .as_str()
                    .is_some_and(|c| c.ends_with(&wanted))
            })
        })
        .ok_or_else(|| format!("no prefab record {prefab}"))?;
    let view = record["views"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["script"].as_str() == Some("CutSceneView"))
        })
        .ok_or("the prefab root has no CutSceneView")?;
    let fields = &view["fields"];
    let root = view["gameObject"]["pathId"]
        .as_str()
        .and_then(|id| id.parse::<i64>().ok())
        .zip(view["gameObject"]["file"].as_str())
        .map(|(id, file)| (file.to_owned(), id));
    let character_root = view["gameObjects"]["_characterRoot"]["gameObject"]["pathId"]
        .as_str()
        .and_then(|id| id.parse::<i64>().ok());
    Ok(ViewFields {
        start_color: color(&fields["_startFadeColor"]),
        end_color: color(&fields["_endFadeColor"]),
        root,
        character_root,
        ik_list: fields["_IKDataList"].as_array().map_or(0, Vec::len),
    })
}

/// The unit's locomotion clips from the character registry.
fn motion_clips(registry: &Value, unit: u32) -> Option<MotionClips> {
    registry["characters"]
        .as_object()?
        .values()
        .find_map(|row| {
            (row["unitId"].as_u64() == Some(u64::from(unit))).then(|| MotionClips {
                idle: row
                    .pointer("/locomotion/idleMotion")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                walk: row
                    .pointer("/locomotion/walkMotion")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
}

/// The NPC of `unit` present on the field (not the player's avatar).
fn present_npc(world: &mut World, unit: u32) -> Option<Entity> {
    world
        .query_filtered::<(Entity, &CharacterUnitId), (
            Without<crate::player::PlayerControlled>,
            Without<CutSceneAvatar>,
        )>()
        .iter(world)
        .find(|(_, id)| id.0 == unit)
        .map(|(entity, _)| entity)
}

/// Whether the fixture's object name holds `gate` (`CutSceneView.BindGate`
/// is called only then): the placed object is named after its model.
fn is_gate_object(world: &World, fixture: Entity) -> bool {
    world
        .get::<FixtureActivityIdentity>(fixture)
        .is_some_and(|identity| identity.model_package.contains("gate"))
}

/// The view's nodes from the package's particle document, under the
/// cut-scene root, which stands for the prefab root: each node at its
/// authored local pose (reflected into the product frame), with its source
/// GameObject and its authored activity.
fn spawn_view_nodes(
    world: &mut World,
    document: &Value,
    root: Entity,
    (file, root_id): (&str, i64),
) -> Result<usize, String> {
    if document["nodeCoordinates"] != "unity-lh-y-up-authored" {
        return Err(format!(
            "unsupported node coordinates {} in the particle document",
            document["nodeCoordinates"]
        ));
    }
    let nodes = document["nodes"]
        .as_array()
        .ok_or("the particle document has no node list")?;
    let mut children: HashMap<i64, Vec<&Value>> = HashMap::new();
    let mut root_active = true;
    for node in nodes {
        let id = node["gameObjectId"]
            .as_i64()
            .ok_or("a document node has no gameObjectId")?;
        if id == root_id {
            root_active = node["active"].as_bool() != Some(false);
        }
        let parent = node["parentGameObjectId"]
            .as_i64()
            .ok_or("a document node has no parentGameObjectId")?;
        children.entry(parent).or_default().push(node);
    }
    if !nodes
        .iter()
        .any(|node| node["gameObjectId"].as_i64() == Some(root_id))
    {
        return Err(format!(
            "the prefab root GameObject {root_id} is not in the package's particle document"
        ));
    }
    let floats = |node: &Value, key: &str, len: usize| -> Result<Vec<f32>, String> {
        node[key]
            .as_array()
            .filter(|list| list.len() == len)
            .and_then(|list| {
                list.iter()
                    .map(|v| v.as_f64().map(|v| v as f32))
                    .collect::<Option<Vec<f32>>>()
            })
            .ok_or_else(|| format!("node {} has no numeric {key}", node["node"]))
    };
    let identity = |id: i64| moly_assets::source_navigation::SourceObjectIdentity {
        file: file.to_owned(),
        game_object: id,
        transform: 0,
        components: Vec::new(),
        child_order: Vec::new(),
    };
    world
        .entity_mut(root)
        .insert((identity(root_id), SourceNodeActivity::authored(root_active)));
    let mut stack = vec![(root_id, root)];
    let mut count = 0usize;
    while let Some((id, entity)) = stack.pop() {
        for node in children.get(&id).into_iter().flatten() {
            let child_id = node["gameObjectId"].as_i64().unwrap_or(0);
            let p = floats(node, "position", 3)?;
            let r = floats(node, "rotation", 4)?;
            let s = floats(node, "scale", 3)?;
            let active = node["active"].as_bool() != Some(false);
            let leaf = node["node"]
                .as_str()
                .and_then(|path| path.rsplit('/').next())
                .unwrap_or("?")
                .to_owned();
            let child = world
                .spawn((
                    Name::new(leaf),
                    Transform {
                        translation: moly_assets::coordinates::source_position(Vec3::new(
                            p[0], p[1], p[2],
                        )),
                        rotation: moly_assets::coordinates::source_rotation(Quat::from_xyzw(
                            r[0], r[1], r[2], r[3],
                        )),
                        scale: Vec3::new(s[0], s[1], s[2]),
                    },
                    if active {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                    SourceNodeActivity::authored(active),
                    identity(child_id),
                    ChildOf(entity),
                ))
                .id();
            stack.push((child_id, child));
            count += 1;
            if count > nodes.len() {
                return Err("the particle document's node parents form a cycle".into());
            }
        }
    }
    RefreshSourceActivity(root).apply(world);
    Ok(count)
}

/// The object spawned for a source GameObject under `root`.
fn node_of(world: &World, root: Entity, game_object: i64) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world
            .get::<moly_assets::source_navigation::SourceObjectIdentity>(entity)
            .is_some_and(|identity| identity.game_object == game_object)
        {
            return Some(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    None
}

/// A load step of the cast.
pub(super) enum Step {
    Wait(String),
    Ready,
}

/// `FadeAndSetUpAsync` for a cast: the view placed at the start transform,
/// its nodes spawned, each unit's character bound, the gate bound, and the
/// director's tracks this owner cannot bind left out by name. `request` is
/// the session being prepared; its definition may be replaced by one
/// without the refused tracks.
pub(super) fn prepare(
    world: &mut World,
    cast: &mut Cast,
    request: &mut StartTimeline,
    root: Entity,
    tracks_document: &Value,
    particles: Option<&Value>,
    prefab: &str,
) -> Result<Step, String> {
    let start = cast.play.start;
    if world.get_entity(start).is_err() {
        return Err("the start transform's object no longer exists".into());
    }
    if cast.view.is_none() {
        cast.view = Some(read_view(tracks_document, prefab)?);
    }
    let view = cast.view.clone().unwrap_or_default();
    if !cast.placed {
        let Some(global) = world.get::<GlobalTransform>(start).copied() else {
            return Ok(Step::Wait(
                "the start transform has no world pose yet".into(),
            ));
        };
        let pose = global.compute_transform();
        if let Some(mut transform) = world.get_mut::<Transform>(root) {
            *transform = Transform {
                translation: pose.translation,
                rotation: pose.rotation,
                scale: Vec3::ONE,
            };
        }
        cast.placed = true;
        info!(
            "[cutscene-cast] CutSceneView.Setup: the view is placed at the start transform ({:.3},{:.3},{:.3}) rotation ({:.4},{:.4},{:.4},{:.4}) of {:?}",
            pose.translation.x,
            pose.translation.y,
            pose.translation.z,
            pose.rotation.x,
            pose.rotation.y,
            pose.rotation.z,
            pose.rotation.w,
            world
                .get::<FixtureActivityIdentity>(start)
                .map(|identity| identity.model_package.clone())
        );
    }
    if !cast.nodes_spawned {
        match (particles, view.root.as_ref()) {
            (Some(document), Some((file, id))) => {
                let count = spawn_view_nodes(world, document, root, (file, *id))?;
                info!("[cutscene-cast] the view's {count} nodes are spawned from the package's particle document under the view root");
            }
            (None, _) => {
                return Ok(Step::Wait(
                    "the package's particle document is loading".into(),
                ))
            }
            (_, None) => {
                warn!("[cutscene-cast] the view record names no root GameObject; no view node is spawned (the Control clips have no source object)")
            }
        }
        cast.character_root = view.character_root.and_then(|id| node_of(world, root, id));
        cast.nodes_spawned = true;
        info!(
            "[cutscene-cast] _characterRoot {:?} -> {:?}; _IKDataList {} rows",
            view.character_root, cast.character_root, view.ik_list
        );
    }
    // The characters.
    if cast.members.is_empty() {
        let registry = cast
            .registry
            .get_or_insert_with(|| {
                world
                    .resource::<AssetServer>()
                    .load::<JsonAsset>(moly_assets::character_registry())
            })
            .clone();
        let server = world.resource::<AssetServer>().clone();
        if let LoadState::Failed(error) = server.load_state(&registry) {
            return Err(format!("the character registry failed to load: {error}"));
        }
        let Some(text) = world
            .resource::<Assets<JsonAsset>>()
            .get(&registry)
            .map(|json| json.0.clone())
        else {
            return Ok(Step::Wait("the character registry is loading".into()));
        };
        let registry: Value =
            serde_json::from_str(&text).map_err(|error| format!("character registry: {error}"))?;
        for &unit in &cast.play.units.clone() {
            let present = present_npc(world, unit);
            let (entity, taken_over) = match (cast.play.use_already_exist_character, present) {
                (true, Some(npc)) => {
                    info!("[cutscene-cast] Setup: unit {unit}: FindNPC found it; useAlreadyExistCharacter: taken over (TryCancelCurrentObjective, ForceUpdateCutSceneObjective, SetImmediatelyExecuteNextObjective are the NPC runtime's: not called here)");
                    (npc, true)
                }
                (use_existing, present) => {
                    let clips = motion_clips(&registry, unit)
                        .ok_or_else(|| format!("unit {unit} is not in the character registry"))?;
                    let entity = world
                        .spawn((
                            CharacterUnitId(unit),
                            clips,
                            MotionPhase::Dwelling { remaining: None },
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            Name::new(format!("cut-scene avatar unit {unit}")),
                            CutSceneAvatar,
                        ))
                        .id();
                    info!(
                        "[cutscene-cast] Setup: unit {unit}: useAlreadyExistCharacter {use_existing}, present NPC {present:?}: CreateAndSetupNPCAvatarView + CreateNPCAvatarForMulti(unit, view, 1): new avatar {entity:?} (hidden until the director's first frame)"
                    );
                    (entity, false)
                }
            };
            let prior_visibility = world.get::<Visibility>(entity).copied().unwrap_or_default();
            cast.members.push(Member {
                unit,
                entity,
                taken_over,
                prior_visibility,
            });
        }
    }
    for member in &cast.members {
        if world.get_entity(member.entity).is_err() {
            return Err(format!("unit {}'s character no longer exists", member.unit));
        }
        if world
            .get::<crate::character::MotionDriver>(member.entity)
            .is_none()
            || world
                .get::<crate::character_material::ToonMaterials>(member.entity)
                .is_none()
        {
            return Ok(Step::Wait(format!(
                "unit {}'s character is being assembled",
                member.unit
            )));
        }
    }
    // The leases: the director holds each bound object.
    let lease = CutSceneCastLease(request.owner.activity);
    for member in &cast.members {
        world.entity_mut(member.entity).insert(lease);
    }
    let mut refused: Vec<(SourceAssetId, String)> = Vec::new();
    for member in &cast.members {
        let (animator, graph) = {
            let driver = world
                .get::<crate::character::MotionDriver>(member.entity)
                .expect("checked above");
            (driver.player, driver.graph.clone())
        };
        match timeline::prepare_cast_actor_bindings(
            world,
            request,
            member.unit,
            member.entity,
            animator,
            graph,
        ) {
            Ok(()) => {}
            Err(error) if error.retryable => {
                return Ok(Step::Wait(format!(
                    "unit {}'s clips: {}",
                    member.unit, error.message
                )))
            }
            Err(error) => {
                for track in &request.definition.tracks {
                    if track.name.parse::<u32>().ok() == Some(member.unit) {
                        refused.push((
                            track.identity.clone(),
                            format!(
                                "track {} ({}): unit {}'s clips are not bound: {}",
                                track.name, track.class, member.unit, error.message
                            ),
                        ));
                    }
                }
            }
        }
    }
    // Every unit-named track needs a bound member.
    for track in &request.definition.tracks {
        if let Ok(unit) = track.name.parse::<u32>() {
            if !cast.members.iter().any(|member| member.unit == unit) {
                refused.push((
                    track.identity.clone(),
                    format!(
                        "track {} ({}): no character of unit {unit} is bound (not in characterUnitIds)",
                        track.name, track.class
                    ),
                ));
            }
        }
        if let Some(clip) = track.infinite.as_ref() {
            // Foot IK solves a humanoid's feet; a recorded clip that binds
            // only the root Transform leaves nothing for it to move, so the
            // flag changes no pose here.
            if clip.track_offset != 0 || clip.remove_offset {
                refused.push((
                    track.identity.clone(),
                    format!(
                        "track {}: recorded clip {} ({}/{}) with track offset mode {} and remove-start-offset {} (only the transform offsets are applied)",
                        track.name,
                        clip.name,
                        clip.asset.file,
                        clip.asset.path_id,
                        clip.track_offset,
                        clip.remove_offset
                    ),
                ));
            } else {
                info!(
                    "[cutscene-cast] track {}: recorded clip {} ({}/{}), length {:.4} s, foot IK flag {} (root Transform curves only)",
                    track.name,
                    clip.name,
                    clip.asset.file,
                    clip.asset.path_id,
                    clip.length,
                    clip.apply_foot_ik
                );
            }
        }
    }
    // The gate.
    let gate_tracks: Vec<SourceAssetId> = request
        .definition
        .tracks
        .iter()
        .filter(|track| track.class == "AnimationTrack" && track.name == "Gate")
        .map(|track| track.identity.clone())
        .collect();
    if !gate_tracks.is_empty() {
        if is_gate_object(world, start) {
            world.entity_mut(start).insert(lease);
            cast.gate = Some(start);
            world.init_resource::<FixtureActivityProvider>();
            for track in &gate_tracks {
                let result =
                    world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
                        provider.prepare_cut_scene_fixture_track(world, request, track, start)
                    });
                match result {
                    Ok(()) => {}
                    Err(pending) if pending.retryable => {
                        return Ok(Step::Wait(format!(
                            "the gate's clips: {}: {}",
                            pending.stage, pending.reason
                        )))
                    }
                    Err(pending) => refused.push((
                        track.clone(),
                        format!(
                            "track Gate: the gate's clips are not bound: {}: {}",
                            pending.stage, pending.reason
                        ),
                    )),
                }
            }
        } else {
            for track in &gate_tracks {
                refused.push((track.clone(), "track Gate: the start transform's name holds no \"gate\", so BindGateAnimator is not called and the track has no binding".into()));
            }
        }
    }
    // The tracks this owner cannot bind are left out of the director.
    if !refused.is_empty() {
        let removed: HashSet<SourceAssetId> =
            refused.iter().map(|(track, _)| track.clone()).collect();
        let mut definition = (*request.definition).clone();
        definition
            .tracks
            .retain(|track| !removed.contains(&track.identity));
        request.definition = Arc::new(definition);
        request
            .bindings
            .animations
            .retain(|key, _| !removed.contains(&key.track));
        request
            .bindings
            .actors
            .retain(|track, _| !removed.contains(track));
    }
    if cast.refused != refused {
        for (_, reason) in &refused {
            error!(
                "[cutscene-cast] {}: refused: {reason}; the rest of the cut-scene plays",
                cast.play.timeline
            );
        }
        cast.refused = refused;
    }
    Ok(Step::Ready)
}

/// The members, the gate and every track this owner drives, for the load
/// report.
pub(super) fn describe(cast: &Cast, definition: &TimelineDefinition) -> String {
    let members: Vec<String> = cast
        .members
        .iter()
        .map(|member| {
            format!(
                "unit {} {} {:?}",
                member.unit,
                if member.taken_over {
                    "taken over"
                } else {
                    "new avatar"
                },
                member.entity
            )
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for track in &definition.tracks {
        for clip in &track.clips {
            let kind = match &clip.payload {
                TimelinePayload::CutScene(CutScenePayload::Activation) => "activation",
                TimelinePayload::CutScene(CutScenePayload::PlayerActivation) => "player activation",
                TimelinePayload::CutScene(CutScenePayload::SkipNext) => "skip next",
                TimelinePayload::Eye { .. } => "eye preset",
                TimelinePayload::Lip { .. } => "lip-sync preset",
                TimelinePayload::NoPresetChange => "preset (no change)",
                _ => continue,
            };
            *counts.entry(kind).or_default() += 1;
        }
    }
    let recorded = definition
        .tracks
        .iter()
        .filter(|track| track.infinite.is_some())
        .count();
    let ik = definition
        .tracks
        .iter()
        .filter(|track| track.class == "ChangeIKTargetTrack")
        .map(|track| track.clips.len())
        .sum::<usize>();
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort();
    format!(
        "cast [{}]; gate {:?}; recorded-clip tracks {recorded}; clips driven or named here {counts:?}; IK target clips {ik}; tracks left out {}",
        members.join(", "),
        cast.gate,
        cast.refused.len()
    )
}

/// `CutScenePresenter.Setup`'s hiding and `ChangeParentCharacters`, after
/// `ChangeState(CutScene)`.
pub(super) fn setup(
    world: &mut World,
    cast: &mut Cast,
    hide_distance: i64,
    white: &[String],
    black: &[String],
) {
    let members: HashSet<Entity> = cast.members.iter().map(|member| member.entity).collect();
    // The NPCs not bound are hidden.
    let others: Vec<(Entity, u32, Visibility)> = world
        .query_filtered::<(Entity, &CharacterUnitId, Option<&Visibility>), Without<crate::player::PlayerControlled>>()
        .iter(world)
        .filter(|(entity, _, _)| !members.contains(entity))
        .map(|(entity, unit, visibility)| (entity, unit.0, visibility.copied().unwrap_or_default()))
        .collect();
    for (entity, _, prior) in &others {
        cast.hidden_npcs.push((*entity, *prior));
        world.entity_mut(*entity).insert(Visibility::Hidden);
    }
    info!(
        "[cutscene-cast] Setup: the NPCs not bound are hidden: units {:?}",
        others.iter().map(|(_, unit, _)| *unit).collect::<Vec<_>>()
    );
    if cast.play.hide_player {
        let players: Vec<(Entity, Visibility)> = world
            .query_filtered::<(Entity, Option<&Visibility>), With<crate::player::PlayerControlled>>(
            )
            .iter(world)
            .map(|(entity, visibility)| (entity, visibility.copied().unwrap_or_default()))
            .collect();
        for (entity, prior) in &players {
            cast.hidden_player.push((*entity, *prior));
            world.entity_mut(*entity).insert(Visibility::Hidden);
        }
        info!(
            "[cutscene-cast] Setup: hidePlayer: {} player avatar(s) hidden",
            players.len()
        );
    }
    hide_near_fixtures(world, cast, hide_distance);
    info!(
        "[cutscene-cast] Setup: ShowWhiteListFixtures {white:?}; HideBlackListFixtures {black:?}"
    );
    // ChangeParentCharacters(characterRoot, worldPositionStays false).
    if let Some(parent) = cast.character_root {
        for member in &cast.members {
            world.entity_mut(member.entity).insert(ChildOf(parent));
        }
        info!(
            "[cutscene-cast] SetupInternal: CacheCharacterBeforeParent; ChangeParentCharacters(_characterRoot, worldPositionStays false): {} characters under {parent:?} keep their local poses",
            cast.members.len()
        );
    } else {
        error!("[cutscene-cast] SetupInternal: the view has no _characterRoot node; the characters keep their parents (named gap)");
    }
    info!("[cutscene-cast] SetupInternal: DisableIKCharacters: the characters' IK is off (the product has no character IK solver)");
}

/// `HideNearFixtures(hideFixtureDistance)`: every fixture whose master id is
/// neither the gate's nor the home's, within the distance of the gate's
/// world position (3-D, strict), is hidden; nothing without a gate.
fn hide_near_fixtures(world: &mut World, cast: &mut Cast, distance: i64) {
    let Some(gate) = cast
        .gate
        .or_else(|| is_gate_object(world, cast.play.start).then_some(cast.play.start))
    else {
        info!("[cutscene-cast] Setup: HideNearFixtures({distance}): no gate, nothing hidden");
        return;
    };
    let Some(center) = world
        .get::<GlobalTransform>(gate)
        .map(GlobalTransform::translation)
    else {
        return;
    };
    let gate_master = world
        .get::<FixtureActivityIdentity>(gate)
        .map(|identity| identity.master_id);
    let rows: Vec<(Entity, String, f32)> = {
        let mut query = world.query::<(Entity, &FixtureActivityIdentity, &GlobalTransform)>();
        query
            .iter(world)
            .filter(|(_, identity, _)| Some(identity.master_id) != gate_master)
            .map(|(entity, identity, global)| {
                (
                    entity,
                    identity.model_package.clone(),
                    global.translation().distance(center),
                )
            })
            .collect()
    };
    let homes = world.get_resource::<crate::entry::house::HomeFixtures>();
    let homes: Vec<bool> = rows
        .iter()
        .map(|(_, package, _)| homes.is_some_and(|homes| homes.is_home(package).unwrap_or(false)))
        .collect();
    let mut hidden = Vec::new();
    for ((entity, package, d), home) in rows.into_iter().zip(homes) {
        if home || d - distance as f32 >= 0.0 {
            continue;
        }
        if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
            *visibility = Visibility::Hidden;
            cast.hidden_fixtures.push(entity);
            hidden.push(format!("{package} at {d:.2}"));
        }
    }
    info!(
        "[cutscene-cast] Setup: HideNearFixtures({distance}) around the gate: {} hidden {hidden:?}",
        hidden.len()
    );
}

/// Unity's `Quaternion.Euler(x, y, z)` (degrees): Z, then X, then Y.
fn unity_euler(euler: [f64; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        (euler[1] as f32).to_radians(),
        (euler[0] as f32).to_radians(),
        (euler[2] as f32).to_radians(),
    )
}

fn v3(value: [f64; 3]) -> Vec3 {
    Vec3::new(value[0] as f32, value[1] as f32, value[2] as f32)
}

/// The recorded clip's root local pose at director time `t` (source frame),
/// the transform offsets applied.
fn recorded_pose(clip: &InfiniteClip, t: f64) -> Option<(Vec3, Quat, f64)> {
    let local = clip.local_time(t)?;
    let (position, euler) = clip.sample(local);
    let (mut position, mut rotation) = (v3(position), unity_euler(euler));
    if clip.track_offset == 0 && clip.has_root_transforms() {
        let offset = unity_euler(clip.offset_euler);
        position = offset * position + v3(clip.offset_position);
        rotation = offset * rotation;
    }
    Some((position, rotation, local))
}

/// One director frame of the tracks this owner drives.
/// Every 0.5 s of real time, per member: the character clips its unit's
/// animation tracks hold at `t` and what its animator is playing.
fn report_character_clips(
    world: &mut World,
    cast: &mut Cast,
    definition: &TimelineDefinition,
    t: f64,
    now: f64,
) {
    for member in &cast.members {
        let logged = cast
            .clips_logged
            .get(&member.unit)
            .copied()
            .unwrap_or(f64::NEG_INFINITY);
        if now - logged < 0.5 {
            continue;
        }
        let unit = member.unit.to_string();
        let clips: Vec<String> = definition
            .tracks
            .iter()
            .filter(|track| track.class == "AnimationTrack" && track.name == unit)
            .flat_map(|track| &track.clips)
            .filter(|clip| clip.contains(t))
            .filter_map(|clip| match &clip.payload {
                TimelinePayload::Animation { target, .. } => Some(format!(
                    "{} [{:.3}, {:.3})",
                    target.clip_name,
                    clip.start,
                    clip.end()
                )),
                _ => None,
            })
            .collect();
        let mut stack = vec![member.entity];
        let mut playing = Vec::new();
        while let Some(entity) = stack.pop() {
            if let Some(player) = world.get::<AnimationPlayer>(entity) {
                for (node, active) in player.playing_animations() {
                    playing.push(format!(
                        "node {} t={:.3} w={:.2}",
                        node.index(),
                        active.seek_time(),
                        active.weight()
                    ));
                }
            }
            if let Some(children) = world.get::<Children>(entity) {
                stack.extend(children.iter());
            }
        }
        cast.clips_logged.insert(member.unit, now);
        info!(
            "[cutscene-cast] t={t:.4} unit {} character clips {:?}; animator playing {:?}",
            member.unit, clips, playing
        );
    }
}

pub(super) fn frame(world: &mut World, cast: &mut Cast, definition: &TimelineDefinition, t: f64) {
    let duration = definition.duration;
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    report_character_clips(world, cast, definition, t, now);
    let mut active_tracks = HashSet::new();
    let mut active_clips = HashSet::new();
    for track in &definition.tracks {
        let member = track
            .name
            .parse::<u32>()
            .ok()
            .and_then(|unit| cast.members.iter().find(|member| member.unit == unit));
        if let (Some(clip), Some(member)) = (track.infinite.as_ref(), member) {
            if let Some((position, rotation, local)) = recorded_pose(clip, t) {
                let product = Transform {
                    translation: moly_assets::coordinates::source_position(position),
                    rotation: moly_assets::coordinates::source_rotation(rotation),
                    scale: Vec3::ONE,
                };
                if let Some(mut transform) = world.get_mut::<Transform>(member.entity) {
                    *transform = product;
                }
                let logged = cast
                    .pose_logged
                    .get(&member.unit)
                    .copied()
                    .unwrap_or(f64::NEG_INFINITY);
                if now - logged >= 0.5 {
                    cast.pose_logged.insert(member.unit, now);
                    let world_position = world
                        .get::<GlobalTransform>(member.entity)
                        .map(GlobalTransform::translation);
                    info!(
                        "[cutscene-cast] t={t:.4} unit {} recorded clip {} at {local:.4}: local position ({:.3},{:.3},{:.3}) rotation ({:.4},{:.4},{:.4},{:.4}) (source frame, offset applied); world {:?}",
                        member.unit, clip.name, position.x, position.y, position.z, rotation.x, rotation.y, rotation.z, rotation.w, world_position
                    );
                }
            }
        }
        for clip in &track.clips {
            let now_in = clip.contains(t);
            match &clip.payload {
                TimelinePayload::CutScene(CutScenePayload::Activation) => {
                    if now_in {
                        active_tracks.insert(track.identity.clone());
                    }
                }
                TimelinePayload::CutScene(CutScenePayload::PlayerActivation)
                | TimelinePayload::CutScene(CutScenePayload::SkipNext) => {
                    if now_in {
                        active_clips.insert(clip.key.clone());
                    }
                    let was = cast.active_clips.contains(&clip.key);
                    let bounds = format!("[{:.4}, {:.4})", clip.start, clip.end());
                    let player = matches!(
                        clip.payload,
                        TimelinePayload::CutScene(CutScenePayload::PlayerActivation)
                    );
                    if now_in && !was {
                        if player {
                            let shown = t < duration;
                            if shown {
                                set_player_visible(world, true);
                            }
                            info!("[cutscene-cast] t={t:.4} PlayerActivationBehaviour.OnBehaviourPlay {bounds}: time < duration {shown}: player SetVisible(true){}", if shown { "" } else { " not called" });
                        } else {
                            info!("[cutscene-cast] t={t:.4} SkipNextClip {bounds} active: a tap (CutSceneNextSituation) would jump the director to {:.4}; the tap input is the cut-scene screen's (not here)", clip.end());
                        }
                    } else if !now_in && was && player {
                        let hidden = t < duration;
                        if hidden {
                            set_player_visible(world, false);
                        }
                        info!("[cutscene-cast] t={t:.4} PlayerActivationBehaviour.OnBehaviourPause {bounds}: time < duration {hidden}: player SetVisible(false){}", if hidden { "" } else { " not called" });
                    }
                }
                _ => {}
            }
        }
        if track.class == "ActivationTrack" {
            let active = active_tracks.contains(&track.identity);
            let was = cast.active_tracks.contains(&track.identity);
            if let Some(member) = member {
                if active != was || cast.last_time == f64::NEG_INFINITY {
                    world.entity_mut(member.entity).insert(if active {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    });
                    info!(
                        "[cutscene-cast] t={t:.4} ActivationMixerPlayable.ProcessFrame: track {}: unit {} SetActive({active})",
                        track.name, member.unit
                    );
                }
            }
        }
    }
    cast.active_tracks = active_tracks;
    cast.active_clips = active_clips;
    cast.last_time = t;
}

fn set_player_visible(world: &mut World, visible: bool) {
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<crate::player::PlayerControlled>>()
        .iter(world)
        .collect();
    for player in players {
        world.entity_mut(player).insert(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// `EndAsync`: `RestoreStates` and `ChangeParentCharacters` back, before the
/// view is disposed.
pub(super) fn restore_states(world: &mut World, cast: &mut Cast) {
    for member in &cast.members {
        if world.get_entity(member.entity).is_err() {
            continue;
        }
        world.entity_mut(member.entity).remove::<ChildOf>();
        if member.taken_over {
            world
                .entity_mut(member.entity)
                .remove::<CutSceneCastLease>();
            let idle = crate::character::resume_idle_after_fixture(world, member.entity);
            info!(
                "[cutscene-cast] RestoreStates: unit {}: ChangeState(Idle) is the NPC runtime's (not called here); its idle motion resumes: {idle:?}",
                member.unit
            );
        }
    }
    info!(
        "[cutscene-cast] ChangeParentCharacters(_avatarRoot, worldPositionStays false): {} characters back at the avatar root, local poses kept",
        cast.members.len()
    );
}

/// `EndAsync` after the view is disposed: the activation tracks'
/// post-playback states, the caller's end callback, the NPCs, fixtures and
/// player shown.
pub(super) fn after_dispose(world: &mut World, cast: &mut Cast, definition: &TimelineDefinition) {
    for track in definition
        .tracks
        .iter()
        .filter(|track| track.class == "ActivationTrack")
    {
        let Some(member) = track
            .name
            .parse::<u32>()
            .ok()
            .and_then(|unit| cast.members.iter().find(|member| member.unit == unit))
        else {
            continue;
        };
        let state = track.settings["m_PostPlaybackState"].as_u64();
        let visibility = match state {
            Some(0) => Some(Visibility::Inherited),
            Some(1) => Some(Visibility::Hidden),
            Some(2) => Some(member.prior_visibility),
            _ => None,
        };
        if let (Some(visibility), true) = (visibility, world.get_entity(member.entity).is_ok()) {
            world.entity_mut(member.entity).insert(visibility);
        }
        info!(
            "[cutscene-cast] ActivationMixerPlayable.OnPlayableDestroy: track {}: post-playback state {state:?} ({})",
            track.name,
            match state {
                Some(0) => "Active",
                Some(1) => "Inactive",
                Some(2) => "Revert",
                Some(3) => "LeaveAsIs: the object keeps its state",
                _ => "unknown",
            }
        );
    }
    if let Some(gate) = cast.gate {
        if world.get_entity(gate).is_ok() {
            world.entity_mut(gate).remove::<CutSceneCastLease>();
        }
    }
    for member in &cast.members {
        if world.get_entity(member.entity).is_ok() && !member.taken_over {
            world
                .entity_mut(member.entity)
                .remove::<CutSceneCastLease>();
        }
    }
    crate::gate_flow::cut_scene_end_callback(world, cast);
    let shown: Vec<Entity> = cast
        .hidden_npcs
        .drain(..)
        .filter_map(|(entity, prior)| {
            world.get_entity(entity).is_ok().then(|| {
                world.entity_mut(entity).insert(prior);
                entity
            })
        })
        .collect();
    info!(
        "[cutscene-cast] EndAsync: the NPCs not bound are shown again ({} still present)",
        shown.len()
    );
    cast.hidden_fixtures.clear();
    for (entity, prior) in cast.hidden_player.drain(..) {
        if world.get_entity(entity).is_ok() {
            world.entity_mut(entity).insert(prior);
        }
    }
    set_player_visible(world, true);
    info!("[cutscene-cast] EndAsync: the player is shown");
}

/// The load was refused: the cast is released as `EndAsync` would.
pub(super) fn release_refused(world: &mut World, cast: &mut Cast) {
    for member in &cast.members {
        if world.get_entity(member.entity).is_err() {
            continue;
        }
        world
            .entity_mut(member.entity)
            .remove::<CutSceneCastLease>();
        world.entity_mut(member.entity).remove::<ChildOf>();
        if !member.taken_over {
            world.despawn(member.entity);
        }
    }
    if let Some(gate) = cast.gate {
        if world.get_entity(gate).is_ok() {
            world.entity_mut(gate).remove::<CutSceneCastLease>();
        }
    }
    for (entity, prior) in cast
        .hidden_npcs
        .drain(..)
        .chain(cast.hidden_player.drain(..))
    {
        if world.get_entity(entity).is_ok() {
            world.entity_mut(entity).insert(prior);
        }
    }
}

/// `DisposeNPC` of a cut-scene avatar.
pub(crate) fn dispose_avatar(world: &mut World, entity: Entity) -> bool {
    if world.get::<CutSceneAvatar>(entity).is_none() {
        return false;
    }
    world.despawn(entity);
    true
}
