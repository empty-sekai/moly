//! The player avatar's step item and its timeline view.
//!
//! Source (`PlayerAvatarView`, `PlayerAvatarItemTimelineView`):
//! - `UpdateStepItemObject(assetBundleName, fileName)`: `ClearStepItemObject`
//!   first; the bundle joins the list the view unloads when it is disposed;
//!   `fileName + ".prefab"` is loaded from the bundle (a missing asset logs an
//!   error and leaves no object) and instantiated under `Step`, the avatar's
//!   `Root` bone (`FindDeep("Root")`), at local zero position and rotation.
//! - `ClearStepItemObject`: the object is destroyed.
//! - The object's `PlayerAvatarItemTimelineView`: `Setup(view, onStop)`
//!   subscribes `onStop` to the director's `stopped`, stops the director and
//!   binds its CharacterAnimator stream to the avatar's animator; `Play`
//!   enables and plays it; `Stop` stops it; `MoveEndTime` puts its time at
//!   the duration; `ChangeLoopFlag(state)` sets the first LoopFlag clip's flag
//!   (the flag alone, no loop skip); `WaitForTimelineEnd` waits while it
//!   plays; a paused director is evaluated every LateUpdate, so it holds its
//!   frame. The step director plays once (wrap mode None): at its end it
//!   stops by itself, which calls `onStop`.
//!
//! Here the director is a session of the shared timeline runner owned by the
//! player as a step item: the CharacterAnimator clips come from the avatar's
//! own motion group by name, the prop clips from the step item's own model
//! file, the SE through the audio routing. The session holds the avatar's
//! animator (a `StepItem` lease) from its start to its release; the avatar's
//! locomotion then plays its state clip again. With `MoveEndTime` as the stop
//! callback, a stopped or finished director holds its last frame until the
//! object is cleared; with no callback it lets the avatar go at once.
//!
//! First caller: the delivery site's `PlayAvatarDeliveryAnimation`, which
//! updates the step item to the festival garden's
//! `tl_site_prop_common_dewdrop01`, sets it up with `MoveEndTime` as its stop
//! callback and plays it; its end action changes the loop flag, waits for
//! the end, stops and clears it. `MOLY_STEP_ITEM=<bundle>|<file>` runs that
//! sequence once, when the avatar is wired, to check the service alone.
//!
//! Data: `site-timeline/step-items/manifest.json` (version 1) lists each
//! exported step item under `items["<bundle>|<file>"]` with its timeline
//! `package` and `prefab`, its model file `glb` (next to the manifest) and
//! the `clips` that file carries (`name`, `sourceClip`). A step item missing
//! there, or a clip missing from its model file, refuses the item by name.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    animation::{AnimatedBy, AnimationTargetId},
    asset::LoadState,
    ecs::system::SystemState,
    gltf::Gltf,
    prelude::*,
    scene::{SceneInstance, SceneRoot, SceneSpawner},
};
use moly_assets::{coordinates::CanonicalCoordinates, json::JsonAsset};
use serde_json::Value;

use super::{AvatarDriver, PlayerActionOwner, PlayerActionToken};
use crate::{
    fixture_activity_provider::FixtureActivityProvider,
    fixture_activity_state::FixtureActivityOwner,
    fixture_activity_timeline::{
        self as timeline, AnimationCoverage, FixtureActivityTimelines, SourceAnimationEvidence,
        StartTimeline, TimelineAnimationBinding, TimelineBindings, TimelineFailure, TimelineOwner,
        TimelineOwnerKind, TimelinePayload, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
    },
    player::PlayerControlled,
};

/// The step items the extractor exports, keyed `<bundle>|<file>`.
const STEP_ITEM_ROOT: &str = "site-timeline/step-items/";
const STEP_ITEM_MANIFEST: &str = "site-timeline/step-items/manifest.json";

/// `PlayableDirector.Play` has no timeout; the runner's budget is never met.
const STEP_ITEM_TIMEOUT_SECS: f64 = f64::MAX;

/// The view's stop callback (`Setup`'s `onStop`); `setup` takes `None` for
/// no callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepItemOnStop {
    /// `PlayerAvatarItemTimelineView.MoveEndTime`: the stopped director is put
    /// at its duration and, paused, evaluated there.
    MoveEndTime,
}

/// One exported step item: its timeline, its model file and its clip rows.
struct StepEntry {
    package: String,
    prefab: String,
    glb: String,
    clips: Arc<Value>,
    /// The view's `_playableDirector` (the director the prefab root's view plays).
    view_director: Option<timeline::SourceAssetId>,
}

enum ObjectState {
    /// The manifest and the model file are loading.
    Loading,
    /// The model's scene is spawned under the Root bone.
    Spawned(Entity),
}

/// One running director: the runner session and the avatar lease.
struct Director {
    token: TimelineToken,
    lease: PlayerActionToken,
    /// The stop has been applied (`MoveEndTime` requested, or released).
    stopped: bool,
    /// When the last trace line was written (seconds since start).
    traced: f64,
    /// The timeline's duration.
    duration: f64,
}

struct StepItem {
    bundle: String,
    file: String,
    entry: Option<StepEntry>,
    gltf: Option<Handle<Gltf>>,
    object: ObjectState,
    /// `Setup` has bound the view.
    set_up: bool,
    on_stop: Option<StepItemOnStop>,
    play: bool,
    stop: bool,
    loop_flag: Option<bool>,
    /// Bindings of the last preparation attempt (its SE handles and the SE
    /// found silent carry over to the next attempt).
    draft: Option<TimelineBindings>,
    director: Option<Director>,
}

/// The step item service of the player avatar.
#[derive(Resource, Default)]
pub(crate) struct PlayerStepItem {
    item: Option<StepItem>,
    manifest: Option<Handle<JsonAsset>>,
    /// Bundles named by `UpdateStepItemObject`; the view unloads them when it
    /// is disposed.
    loaded_bundles: Vec<String>,
    /// Cleared items, destroyed at the next advance.
    retired: Vec<StepItem>,
    /// Directors a `Setup` stopped, released at the next advance.
    stopped: Vec<Director>,
    generation: u64,
    harness: Harness,
}

impl PlayerStepItem {
    /// `UpdateStepItemObject(assetBundleName, fileName)`.
    pub(crate) fn update_step_item_object(&mut self, bundle: &str, file: &str) {
        self.clear_step_item_object();
        self.loaded_bundles.push(bundle.to_owned());
        self.item = Some(StepItem {
            bundle: bundle.to_owned(),
            file: file.to_owned(),
            entry: None,
            gltf: None,
            object: ObjectState::Loading,
            set_up: false,
            on_stop: None,
            play: false,
            stop: false,
            loop_flag: None,
            draft: None,
            director: None,
        });
        info!("[step-item] UpdateStepItemObject({bundle}, {file})");
    }

    /// `ClearStepItemObject`: the object is destroyed at the next advance,
    /// with its director.
    pub(crate) fn clear_step_item_object(&mut self) {
        if let Some(item) = self.item.take() {
            self.retired.push(item);
        }
    }

    /// The spawned step object (`StepItemObject`).
    pub(crate) fn step_item_object(&self) -> Option<Entity> {
        match self.item.as_ref()?.object {
            ObjectState::Spawned(entity) => Some(entity),
            ObjectState::Loading => None,
        }
    }

    /// `Setup(view, onStop)`: the director is stopped (a running one lets the
    /// avatar go) and the view is bound to the avatar.
    pub(crate) fn setup(&mut self, on_stop: Option<StepItemOnStop>) {
        if let Some(item) = self.item.as_mut() {
            item.set_up = true;
            item.on_stop = on_stop;
            item.play = false;
            item.stop = false;
            item.loop_flag = None;
            self.stopped.extend(item.director.take());
            info!("[step-item] {} Setup(onStop {on_stop:?})", item.file);
        }
    }

    /// `Play`.
    pub(crate) fn play(&mut self) {
        if let Some(item) = self.item.as_mut() {
            item.play = true;
            item.stop = false;
        }
    }

    /// `ChangeLoopFlag(state)`.
    pub(crate) fn change_loop_flag(&mut self, state: bool) {
        if let Some(item) = self.item.as_mut() {
            item.loop_flag = Some(state);
        }
    }

    /// `Stop`: the director stops and calls its stop callback.
    pub(crate) fn stop(&mut self) {
        if let Some(item) = self.item.as_mut() {
            item.stop = true;
            info!("[step-item] {} Stop", item.file);
        }
    }

    /// `WaitForTimelineEnd` waits while this holds: the director plays.
    pub(crate) fn is_playing(&self, world: &World) -> bool {
        self.is_playing_on(world.get_resource::<FixtureActivityTimelines>())
    }

    /// [`Self::is_playing`] for a caller that holds the runner's resource.
    pub(crate) fn is_playing_on(&self, timelines: Option<&FixtureActivityTimelines>) -> bool {
        let Some(item) = self.item.as_ref() else {
            return false;
        };
        if item.stop || !item.play {
            return false;
        }
        match &item.director {
            // Play was called; the director starts once its object is ready.
            None => true,
            Some(director) => matches!(
                timelines.and_then(|timelines| timelines.status(director.token)),
                Some(TimelineStatus::Preparing | TimelineStatus::Playing { .. })
            ),
        }
    }

    /// The director's last sampled time and its duration, once it has started.
    pub(crate) fn director_time_on(
        &self,
        timelines: Option<&FixtureActivityTimelines>,
    ) -> Option<(f64, f64)> {
        let director = self.item.as_ref()?.director.as_ref()?;
        Some((timelines?.sampled_time(director.token)?, director.duration))
    }
}

/// Update, before the timeline runner: load and spawn the step object, start
/// and steer its director, and destroy what was cleared.
pub(crate) fn advance(world: &mut World) {
    let Some(mut service) = world.remove_resource::<PlayerStepItem>() else {
        return;
    };
    drive_harness(world, &mut service);
    for director in std::mem::take(&mut service.stopped) {
        release_director(world, director);
    }
    for item in std::mem::take(&mut service.retired) {
        retire(world, item);
    }
    let failed = match service.item.as_mut() {
        Some(item) => step(world, &mut service.manifest, &mut service.generation, item).err(),
        None => None,
    };
    if let Some(reason) = failed {
        if let Some(item) = service.item.take() {
            error!(
                "[step-item] {}|{} refused: {reason}",
                item.bundle, item.file
            );
            retire(world, item);
        }
    }
    world.insert_resource(service);
}

/// One frame of the current item. `Err` refuses the item for good.
fn step(
    world: &mut World,
    manifest: &mut Option<Handle<JsonAsset>>,
    generation: &mut u64,
    item: &mut StepItem,
) -> Result<(), String> {
    if item.entry.is_none() {
        let Some(entry) = read_entry(world, manifest, &item.bundle, &item.file)? else {
            return Ok(());
        };
        let server = world.resource::<AssetServer>().clone();
        item.gltf = Some(server.load::<Gltf>(format!("moly://{}", entry.glb)));
        item.entry = Some(entry);
    }
    if let ObjectState::Loading = item.object {
        let handle = item.gltf.clone().expect("requested with its entry");
        let server = world.resource::<AssetServer>().clone();
        if let LoadState::Failed(error) = server.load_state(&handle) {
            return Err(format!("step item model failed to load: {error}"));
        }
        let Some(scene) = world
            .resource::<Assets<Gltf>>()
            .get(&handle)
            .map(|gltf| gltf.default_scene.clone())
        else {
            return Ok(());
        };
        let scene = scene.ok_or("step item model has no default scene")?;
        let root = step_bone(world)?;
        let object = world
            .spawn((SceneRoot(scene), Transform::IDENTITY, Visibility::default()))
            .insert(ChildOf(root))
            .id();
        item.object = ObjectState::Spawned(object);
        info!(
            "[step-item] {}.prefab instantiated as {object:?} under bone {root:?} {:?} (the avatar's Root), local position zero, rotation identity",
            item.file,
            world.get::<Name>(root).map(Name::as_str)
        );
    }
    let ObjectState::Spawned(object) = item.object else {
        return Ok(());
    };
    if let Some(director) = item.director.as_mut() {
        trace(world, director, &item.file, object);
        let release = steer(
            world,
            item.on_stop,
            item.stop,
            &mut item.loop_flag,
            director,
            &item.file,
        )?;
        if release {
            let director = item.director.take().expect("steered above");
            release_director(world, director);
        }
        return Ok(());
    }
    if !item.play || item.stop {
        return Ok(());
    }
    if !item.set_up {
        return Err("Play before Setup: the view has not bound the avatar".into());
    }
    let Some(instance) = world.get::<SceneInstance>(object) else {
        return Ok(());
    };
    if !world
        .resource::<SceneSpawner>()
        .instance_is_ready(**instance)
    {
        return Ok(());
    }
    let view = step_view(world, object)?;
    install_graphs(world, view);
    let request = match prepare(world, generation, item, view) {
        Ok(request) => request,
        Err(Pending::Wait) => return Ok(()),
        Err(Pending::Refused(reason)) => return Err(reason),
    };
    if let Some(expected) = item
        .entry
        .as_ref()
        .and_then(|entry| entry.view_director.as_ref())
    {
        if &request.definition.director != expected {
            return Err(format!(
                "the view's director {expected:?} is not the timeline's selected director {:?}",
                request.definition.director
            ));
        }
    }
    let lease = {
        let mut drivers = world.query_filtered::<&mut AvatarDriver, With<PlayerControlled>>();
        let mut driver = drivers
            .single_mut(world)
            .map_err(|_| "no single player avatar")?;
        driver
            .acquire(PlayerActionOwner::StepItem)
            .map_err(|error| format!("the avatar's animator: {error}"))?
    };
    let mut refused: Vec<_> = request
        .bindings
        .refused_controls
        .values()
        .cloned()
        .collect();
    refused.sort();
    for reason in &refused {
        warn!(
            "[step-item] {}: {reason}; the rest of the timeline plays",
            item.file
        );
    }
    let duration = request.definition.duration;
    let bound = request.bindings.animations.len();
    let token = world
        .resource_mut::<FixtureActivityTimelines>()
        .request_start(request);
    item.draft = None;
    item.director = Some(Director {
        token,
        lease,
        stopped: false,
        traced: f64::NEG_INFINITY,
        duration,
    });
    info!(
        "[step-item] {} Play: director {token:?} started (duration {duration:.4} s, {bound} animation clips bound, {} Control clips refused)",
        item.file,
        refused.len()
    );
    Ok(())
}

/// A running director this frame. `Ok(true)` lets the avatar go.
fn steer(
    world: &mut World,
    on_stop: Option<StepItemOnStop>,
    stop: bool,
    loop_flag: &mut Option<bool>,
    director: &mut Director,
    file: &str,
) -> Result<bool, String> {
    let status = world
        .resource::<FixtureActivityTimelines>()
        .status(director.token)
        .cloned();
    match status {
        Some(TimelineStatus::Failed(error)) => return Err(format!("director failed: {error}")),
        Some(TimelineStatus::Cancelled) | None => return Err("director was cancelled".into()),
        Some(TimelineStatus::Completed) => {
            if director.stopped {
                return Ok(false);
            }
            director.stopped = true;
            let sampled = world
                .resource::<FixtureActivityTimelines>()
                .sampled_time(director.token)
                .unwrap_or(f64::NAN);
            info!(
                "[step-item] {file} director reached its end at time {sampled:.4} and stopped (onStop {on_stop:?})"
            );
            return Ok(on_stop != Some(StepItemOnStop::MoveEndTime));
        }
        Some(TimelineStatus::Preparing | TimelineStatus::Playing { .. }) => {}
    }
    if stop && !director.stopped {
        director.stopped = true;
        if on_stop == Some(StepItemOnStop::MoveEndTime) {
            world
                .resource_mut::<FixtureActivityTimelines>()
                .move_end_time(director.token);
            info!("[step-item] {file} MoveEndTime: the director holds its last frame");
            return Ok(false);
        }
        return Ok(true);
    }
    if let Some(state) = *loop_flag {
        if world
            .resource_mut::<FixtureActivityTimelines>()
            .set_loop_flag(director.token, state)
        {
            *loop_flag = None;
            info!("[step-item] {file} ChangeLoopFlag({state})");
        }
    }
    Ok(false)
}

enum Pending {
    Wait,
    Refused(String),
}

impl From<TimelineFailure> for Pending {
    fn from(error: TimelineFailure) -> Self {
        if error.retryable {
            Self::Wait
        } else {
            Self::Refused(error.to_string())
        }
    }
}

/// The manifest row of `<bundle>|<file>`; `None` while the manifest loads.
fn read_entry(
    world: &mut World,
    manifest: &mut Option<Handle<JsonAsset>>,
    bundle: &str,
    file: &str,
) -> Result<Option<StepEntry>, String> {
    let server = world.resource::<AssetServer>().clone();
    let handle = manifest
        .get_or_insert_with(|| server.load(format!("moly://{STEP_ITEM_MANIFEST}")))
        .clone();
    if let LoadState::Failed(error) = server.load_state(&handle) {
        return Err(format!(
            "no step item was exported ({STEP_ITEM_MANIFEST}: {error})"
        ));
    }
    let Some(text) = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|json| json.0.clone())
    else {
        return Ok(None);
    };
    let document: Value =
        serde_json::from_str(&text).map_err(|error| format!("{STEP_ITEM_MANIFEST}: {error}"))?;
    if document["version"].as_u64() != Some(1) {
        return Err(format!("{STEP_ITEM_MANIFEST}: unsupported version"));
    }
    let key = format!("{bundle}|{file}");
    let row = document["items"]
        .get(key.as_str())
        .ok_or_else(|| format!("{key} is not an exported step item"))?;
    let text = |field: &str| -> Result<String, String> {
        row[field]
            .as_str()
            .filter(|value| !value.is_empty() && !value.contains(['/', '\\', ':']))
            .map(str::to_owned)
            .ok_or_else(|| format!("{key}: {field} is not a plain name"))
    };
    let view_director = row
        .pointer("/view/director")
        .filter(|value| !value.is_null())
        .map(|value| -> Result<timeline::SourceAssetId, String> {
            Ok(timeline::SourceAssetId {
                file: value["file"]
                    .as_str()
                    .ok_or_else(|| format!("{key}: view director has no file"))?
                    .to_owned(),
                path_id: value["pathId"]
                    .as_str()
                    .ok_or_else(|| format!("{key}: view director has no pathId"))?
                    .to_owned(),
            })
        })
        .transpose()?;
    Ok(Some(StepEntry {
        package: text("package")?,
        prefab: text("prefab")?,
        glb: format!("{STEP_ITEM_ROOT}{}", text("glb")?),
        clips: Arc::new(row["clips"].clone()),
        view_director,
    }))
}

/// `Step`: the avatar's `Root` bone, the first node of that name below the
/// avatar's model.
fn step_bone(world: &mut World) -> Result<Entity, String> {
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let root = drivers
        .single(world)
        .map_err(|_| "no single player avatar".to_owned())?
        .visual_root;
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if world
            .get::<Name>(entity)
            .is_some_and(|name| name.as_str() == "Root")
        {
            return Ok(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().rev());
        }
    }
    Err("the avatar has no Root bone".into())
}

/// The instantiated prefab: the one node of the spawned scene that carries
/// the model file's coordinate contract.
fn step_view(world: &World, object: Entity) -> Result<Entity, String> {
    let mut stack = vec![object];
    let mut found = Vec::new();
    while let Some(entity) = stack.pop() {
        if world.get::<CanonicalCoordinates>(entity).is_some() {
            found.push(entity);
            continue;
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    match found.as_slice() {
        [view] => Ok(*view),
        [] => Err("step item model carries no coordinate contract; re-export it".into()),
        _ => Err(format!(
            "step item model has {} prefab roots with a coordinate contract",
            found.len()
        )),
    }
}

/// The model's own animators get a graph the runner can add clip nodes to.
fn install_graphs(world: &mut World, view: Entity) {
    let mut stack = vec![view];
    let mut animators = Vec::new();
    while let Some(entity) = stack.pop() {
        if world.get::<AnimationPlayer>(entity).is_some()
            && world.get::<AnimationGraphHandle>(entity).is_none()
        {
            animators.push(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    for animator in animators {
        let graph = world
            .resource_mut::<Assets<AnimationGraph>>()
            .add(AnimationGraph::new());
        world
            .entity_mut(animator)
            .insert(AnimationGraphHandle(graph));
    }
}

/// The step item's director bound as its view binds it.
fn prepare(
    world: &mut World,
    generation: &mut u64,
    item: &mut StepItem,
    view: Entity,
) -> Result<StartTimeline, Pending> {
    let entry = item.entry.as_ref().expect("read before the object");
    world.init_resource::<FixtureActivityProvider>();
    let definition = world
        .resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.site_definition(world, &entry.package, &entry.prefab)
        })
        .map_err(|pending| {
            if pending.retryable {
                Pending::Wait
            } else {
                Pending::Refused(format!("{}: {}", pending.stage, pending.reason))
            }
        })?;
    let (actor, animator, graph, clips) = {
        let mut drivers = world.query_filtered::<(Entity, &AvatarDriver), With<PlayerControlled>>();
        let (actor, driver) = drivers
            .single(world)
            .map_err(|_| Pending::Refused("no single player avatar".into()))?;
        let (animator, graph) = driver.fixture_timeline_binding();
        (actor, animator, graph, driver.clips().clone())
    };
    *generation += 1;
    let mut request = StartTimeline {
        owner: TimelineOwner {
            activity: FixtureActivityOwner {
                actor,
                generation: *generation,
            },
            kind: TimelineOwnerKind::StepItem,
        },
        fixture: view,
        definition,
        bindings: TimelineBindings::default(),
        companions: Vec::new(),
        timeout_secs: STEP_ITEM_TIMEOUT_SECS,
        timeout_budget: TimelineTimeoutBudget::PlayerWall,
    };
    // An SE found silent stays silent; the last attempt's SE handles come
    // along, so a load that has failed since is seen as failed.
    if let Some(previous) = item.draft.take() {
        request.bindings.silent_sounds = previous.silent_sounds;
        request.bindings.sounds = previous.sounds;
    }
    let file = item.file.clone();
    let silence = |world: &World, request: &mut StartTimeline| {
        for silenced in timeline::silence_unavailable_sounds(world, request) {
            warn!("[step-item] {file}: SE {silenced}; it plays silent");
        }
    };
    silence(world, &mut request);
    let _ = timeline::prepare_source_sounds(world, &mut request);
    let entry = item.entry.as_ref().expect("read before the object");
    let gltf = item.gltf.clone().expect("requested with its entry");
    let prepared = bind(world, &mut request, entry, &gltf, animator, graph, &clips);
    silence(world, &mut request);
    item.draft = Some(request.bindings.clone());
    for (clip, events) in prepared? {
        warn!(
            "[step-item] {file}: clip {clip} carries {events} animation events; the runner does not dispatch them"
        );
    }
    timeline::validate_start(world, &request)?;
    Ok(request)
}

/// BindPlayer, the step object's own animation tracks, the SE and the
/// effects. Returns the avatar clips that carry animation events.
fn bind(
    world: &mut World,
    request: &mut StartTimeline,
    entry: &StepEntry,
    gltf: &Handle<Gltf>,
    animator: Entity,
    graph: Handle<AnimationGraph>,
    clips: &super::BodyClips,
) -> Result<Vec<(String, usize)>, Pending> {
    let with_events = timeline::prepare_player_group_bindings(request, animator, graph, clips)?;
    prepare_prop_bindings(world, request, entry, gltf)?;
    timeline::prepare_source_sounds(world, request)?;
    timeline::prepare_source_effects(world, request)?;
    Ok(with_events)
}

/// The step object's animation tracks: each clip plays on the one animator
/// of the object whose targets cover the clip, taken from the object's own
/// model file by name and source identity.
fn prepare_prop_bindings(
    world: &mut World,
    request: &mut StartTimeline,
    entry: &StepEntry,
    gltf: &Handle<Gltf>,
) -> Result<(), Pending> {
    let view = request.fixture;
    let rows = entry.clips.as_array().cloned().unwrap_or_default();
    let mut targets: HashMap<Entity, HashSet<AnimationTargetId>> = HashMap::new();
    for (entity, id, by) in world
        .query::<(Entity, &AnimationTargetId, &AnimatedBy)>()
        .iter(world)
    {
        if descends(world, entity, view) {
            targets.entry(by.0).or_default().insert(*id);
        }
    }
    let mut bindings = Vec::new();
    for track in &request.definition.tracks {
        if track.class != "AnimationTrack" || track.name == "CharacterAnimator" {
            continue;
        }
        for clip in &track.clips {
            let TimelinePayload::Animation { target, .. } = &clip.payload else {
                return Err(Pending::Refused(
                    "animation track holds another payload type".into(),
                ));
            };
            let evidence = SourceAnimationEvidence::from_clip_target(target)?;
            let listed = rows.iter().any(|row| {
                row["name"].as_str() == Some(target.clip_name.as_str())
                    && row.pointer("/sourceClip/file").and_then(Value::as_str)
                        == Some(evidence.asset.file.as_str())
                    && row.pointer("/sourceClip/pathId").and_then(Value::as_str)
                        == Some(evidence.asset.path_id.as_str())
            });
            if !listed {
                return Err(Pending::Refused(format!(
                    "clip {} is absent from the step item's model file {}",
                    target.clip_name, entry.glb
                )));
            }
            let animation = world
                .resource::<Assets<Gltf>>()
                .get(gltf)
                .and_then(|gltf| {
                    gltf.named_animations
                        .get(target.clip_name.as_str())
                        .cloned()
                })
                .ok_or_else(|| {
                    Pending::Refused(format!(
                        "clip {} is listed but absent from the step item's model file {}",
                        target.clip_name, entry.glb
                    ))
                })?;
            let ids: HashSet<AnimationTargetId> = world
                .resource::<Assets<AnimationClip>>()
                .get(&animation)
                .ok_or(Pending::Wait)?
                .curves()
                .iter()
                .filter(|(_, curves)| !curves.is_empty())
                .map(|(id, _)| *id)
                .collect();
            let candidates: Vec<Entity> = targets
                .iter()
                .filter(|(_, bound)| !ids.is_empty() && ids.is_subset(bound))
                .map(|(animator, _)| *animator)
                .collect();
            let [animator] = candidates.as_slice() else {
                return Err(Pending::Refused(format!(
                    "clip {} has {} matching animators in the step object",
                    target.clip_name,
                    candidates.len()
                )));
            };
            let graph = world
                .get::<AnimationGraphHandle>(*animator)
                .ok_or_else(|| Pending::Refused("the step object's animator has no graph".into()))?
                .0
                .clone();
            bindings.push((
                clip.key.clone(),
                TimelineAnimationBinding {
                    animator: *animator,
                    graph,
                    clip: animation,
                    source: evidence,
                    coverage: AnimationCoverage::sampled_pose(),
                },
            ));
        }
    }
    request.bindings.animations.extend(bindings);
    Ok(())
}

fn descends(world: &World, mut entity: Entity, root: Entity) -> bool {
    loop {
        if entity == root {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

/// Every quarter second of a running or held director: its time, and what
/// actually plays on the avatar's animator and on the step object's own
/// animators (clip by name, its time and weight), read from the players.
fn trace(world: &mut World, director: &mut Director, file: &str, object: Entity) {
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, Time::elapsed_secs_f64);
    if now - director.traced < 0.25 {
        return;
    }
    director.traced = now;
    let mut animators = Vec::new();
    let mut stack = vec![object];
    while let Some(entity) = stack.pop() {
        if world.get::<AnimationPlayer>(entity).is_some() {
            animators.push(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let timelines = world.resource::<FixtureActivityTimelines>();
    let sampled = timelines.sampled_time(director.token);
    let status = match timelines.status(director.token) {
        Some(TimelineStatus::Playing {
            loop_active: true, ..
        }) => "playing (in its loop clip)",
        Some(TimelineStatus::Playing { .. }) => "playing",
        Some(TimelineStatus::Completed) => "held at its end",
        Some(TimelineStatus::Preparing) => "preparing",
        _ => "stopped",
    };
    let graphs = world.resource::<Assets<AnimationGraph>>();
    let clip_of = |graph: &Handle<AnimationGraph>, node| -> Option<Handle<AnimationClip>> {
        match &graphs.get(graph)?.get(node)?.node_type {
            bevy::animation::graph::AnimationNodeType::Clip(handle) => Some(handle.clone()),
            _ => None,
        }
    };
    let mut avatar = Vec::new();
    if let Ok(driver) = drivers.single(world) {
        let (animator, graph) = driver.fixture_timeline_binding();
        if let Some(player) = world.get::<AnimationPlayer>(animator) {
            for (node, active) in player.playing_animations() {
                let name = clip_of(&graph, *node)
                    .and_then(|clip| {
                        driver
                            .clips()
                            .0
                            .values()
                            .find(|body| body.handle == clip)
                            .map(|body| body.name.clone())
                    })
                    .unwrap_or_else(|| "?".into());
                avatar.push(format!(
                    "{name} t {:.3} w {:.2}",
                    active.seek_time(),
                    active.weight()
                ));
            }
        }
    }
    let mut props = Vec::new();
    for entity in animators {
        let (Some(player), Some(graph)) = (
            world.get::<AnimationPlayer>(entity),
            world.get::<AnimationGraphHandle>(entity),
        ) else {
            continue;
        };
        for (node, active) in player.playing_animations() {
            let length = clip_of(&graph.0, *node)
                .and_then(|clip| world.resource::<Assets<AnimationClip>>().get(&clip))
                .map_or("?".into(), |clip| format!("{:.3} s", clip.duration()));
            props.push(format!(
                "{entity:?} {:?}: clip of {length} t {:.3} w {:.2}",
                world.get::<Name>(entity).map(Name::as_str),
                active.seek_time(),
                active.weight()
            ));
        }
    }
    info!(
        "[step-item-trace] {file} director {status} time {}: avatar [{}]; step object animators [{}]",
        sampled.map_or("-".into(), |t| format!("{t:.4}")),
        avatar.join(", "),
        props.join(", ")
    );
}

/// The director's session ends and the avatar's lease with it: locomotion
/// plays its state clip again.
fn release_director(world: &mut World, director: Director) {
    if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
        timelines.cancel(director.token);
        timelines.release(director.token);
    }
    let mut params = SystemState::<(
        Query<&mut AvatarDriver, With<PlayerControlled>>,
        Query<&mut AnimationPlayer>,
    )>::new(world);
    let (mut drivers, mut animators) = params.get_mut(world);
    let Ok(mut driver) = drivers.single_mut() else {
        return;
    };
    let animator = driver.player;
    let released = match animators.get_mut(animator) {
        Ok(mut player) => driver.release_action(director.lease, &mut player),
        // The body's animator is gone; only the lease is cleared.
        Err(_) => driver.release_action(director.lease, &mut AnimationPlayer::default()),
    };
    if released {
        info!("[step-item] the avatar's animator returns to locomotion");
    }
}

fn retire(world: &mut World, mut item: StepItem) {
    if let Some(director) = item.director.take() {
        release_director(world, director);
    }
    match item.object {
        ObjectState::Spawned(object) => {
            if let Ok(entity) = world.get_entity_mut(object) {
                entity.despawn();
            }
            info!("[step-item] ClearStepItemObject: {} destroyed", item.file);
        }
        ObjectState::Loading => {
            info!(
                "[step-item] ClearStepItemObject: {} had no object",
                item.file
            );
        }
    }
}

/// The sequence `MOLY_STEP_ITEM=<bundle>|<file>` runs once the avatar is
/// wired: Update, Setup(MoveEndTime), Play; after 2 s, ChangeLoopFlag(false);
/// WaitForTimelineEnd; Stop; after 1 s, ClearStepItemObject.
#[derive(Default)]
struct Harness {
    stage: u8,
    since: f32,
}

const HARNESS_DONE: u8 = u8::MAX;

fn drive_harness(world: &mut World, service: &mut PlayerStepItem) {
    if service.harness.stage == HARNESS_DONE {
        return;
    }
    let Ok(request) = std::env::var("MOLY_STEP_ITEM") else {
        service.harness.stage = HARNESS_DONE;
        return;
    };
    let Some((bundle, file)) = request.split_once('|') else {
        error!("[step-item] MOLY_STEP_ITEM needs <bundle>|<file>, got {request}");
        service.harness.stage = HARNESS_DONE;
        return;
    };
    service.harness.since += world.get_resource::<Time>().map_or(0.0, Time::delta_secs);
    match service.harness.stage {
        0 => {
            let mut drivers =
                world.query_filtered::<(), (With<AvatarDriver>, With<PlayerControlled>)>();
            if drivers.single(world).is_err() {
                return;
            }
            service.update_step_item_object(bundle, file);
            service.setup(Some(StepItemOnStop::MoveEndTime));
            service.play();
            service.harness.stage = 1;
            service.harness.since = 0.0;
        }
        _ if service.item.is_none() => {
            info!("[step-item] harness ends: no step item");
            service.harness.stage = HARNESS_DONE;
        }
        1 if service.harness.since >= 2.0 => {
            service.change_loop_flag(false);
            service.harness.stage = 2;
        }
        2 if !service.is_playing(world) => {
            info!(
                "[step-item] WaitForTimelineEnd returned; object {:?}",
                service.step_item_object()
            );
            service.stop();
            service.harness.stage = 3;
            service.harness.since = 0.0;
        }
        3 if service.harness.since >= 1.0 => {
            service.clear_step_item_object();
            info!(
                "[step-item] harness done; bundles recorded {:?}",
                service.loaded_bundles
            );
            service.harness.stage = HARNESS_DONE;
        }
        _ => {}
    }
}
