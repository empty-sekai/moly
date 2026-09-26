//! `DeliveryPlaceObjectView`: the delivery tree's bloom.
//!
//! Source (`DeliveryPlaceObjectView`): the view's `_playableDirector` is a
//! director inside the site prefab itself, on the tree's `flowers_root`; its
//! animation tracks are bound to the Animators of the site scene's flowers.
//! - `ResetView` (the site is shown): `_isFlowered` off, the director paused;
//!   `OnPaused` puts its time at `_isFlowered ? duration : 0` and
//!   `LateUpdate` evaluates a paused director every frame, so the tree holds
//!   its time-0 pose.
//! - `PlayAnimation` (the first reward drop of a visit): nothing when
//!   `_isFlowered`; otherwise `_isFlowered` on, `Play`, wait until
//!   `time >= duration`, `Pause`: the tree holds its last frame.
//!
//! Here the director is a session of the shared timeline runner owned as a
//! scene director: its animation tracks play on the flowers of the spawned
//! site, each rigged at bind time with the target identities of the clip's
//! rig file (the clip's channels are named from the rig's root, the first
//! bound flower of the prefab, so each flower's nodes are named from that
//! root's name down the flower's own subtree). The session starts paused at
//! time 0 on arrival and is released on `PlayAnimation`; at its end the
//! runner stops sampling and the flowers keep the last sampled pose.
//!
//! Data: the director row of `site-timeline/step-items/manifest.json`
//! (`directors["<bundle>|<prefab>"]`, found by the place view's director),
//! its rig files next to the manifest, and the site package's timeline
//! tables in `site-timeline/`.
//!
//! Named gaps: the Control clips (the tree's particle effects) resolve their
//! objects through the director's exposed-reference table and are refused by
//! name (the particle host drives a fixture's own archive only); the flower
//! Animators' own controller is not modelled (the director's evaluation is
//! what the source shows while it is paused or held).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::coordinates::CanonicalCoordinates;
use moly_assets::json::JsonAsset;
use moly_assets::source_navigation::SourceObjectIdentity;
use serde_json::Value;

use crate::fixture_activity_provider::FixtureActivityProvider;
use crate::fixture_activity_state::FixtureActivityOwner;
use crate::fixture_activity_timeline::{
    self as timeline, AnimationCoverage, FixtureActivityTimelines, SourceAnimationEvidence,
    StartTimeline, TimelineAnimationBinding, TimelineBindings, TimelineDefinition, TimelineOwner,
    TimelineOwnerKind, TimelinePayload, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
};

const MANIFEST: &str = "site-timeline/step-items/manifest.json";
const ROOT: &str = "site-timeline/step-items/";
const SITE_PACKAGE_PREFIX: &str = "mysekai__site__field__";
/// `PlayableDirector.Play` has no timeout.
const NO_TIMEOUT: f64 = f64::MAX;

/// One rig row of the director: its file, the clip it carries and the
/// flowers (Animator components) whose tracks play that clip.
struct Rig {
    glb: String,
    root: String,
    clip: String,
    source_clip: timeline::SourceAssetId,
    tracks: Vec<(timeline::SourceAssetId, i64)>,
    handle: Option<Handle<Gltf>>,
}

struct Plan {
    package: String,
    prefab: String,
    director: i64,
    rigs: Vec<Rig>,
    /// The last attempt's bindings: its SE handles stay alive between
    /// attempts (a dropped handle cancels its load), and an SE found silent
    /// stays silent.
    draft: Option<TimelineBindings>,
    /// The wait last logged, so each distinct wait is logged once.
    waiting: Option<String>,
}

enum Stage {
    /// No delivery site, or its bloom is not set up yet.
    Idle,
    Preparing(Plan),
    Running {
        token: TimelineToken,
        played: bool,
        ended: bool,
        flowers: Vec<Entity>,
        traced: f64,
    },
    /// Refused for this visit (logged once).
    Refused,
}

impl Default for Stage {
    fn default() -> Self {
        Self::Idle
    }
}

/// The place view's director on the current delivery site.
#[derive(Resource, Default)]
pub(crate) struct DeliveryBloom {
    stage: Stage,
    /// The ground epoch this bloom belongs to (one per arrival).
    epoch: Option<u64>,
    manifest: Option<Handle<JsonAsset>>,
    /// `PlayAnimation` was called this visit.
    play_requested: bool,
    generation: u64,
}

impl DeliveryBloom {
    /// `DeliveryPlaceObjectView.PlayAnimation` (the flow calls it once per
    /// visit: its own `_isFlowered` mirror gates the call).
    pub(crate) fn play_animation(&mut self) {
        self.play_requested = true;
    }
}

/// Update, after the step item service and before the timeline runner.
pub(crate) fn advance(world: &mut World) {
    let Some(mut bloom) = world.remove_resource::<DeliveryBloom>() else {
        return;
    };
    step(world, &mut bloom);
    world.insert_resource(bloom);
}

fn step(world: &mut World, bloom: &mut DeliveryBloom) {
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|e| e.0);
    let objects = world
        .get_resource::<super::site::DeliverySite>()
        .and_then(|site| site.objects.clone());
    let Some(objects) = objects.filter(|_| epoch.is_some()) else {
        end(world, bloom, "no delivery site");
        return;
    };
    if bloom.epoch != epoch {
        end(world, bloom, "a new visit");
        bloom.epoch = epoch;
        bloom.play_requested = false;
        let Some(director) = objects.place_director else {
            error!("[delivery-bloom] the place view names no director; the tree does not bloom on this visit");
            bloom.stage = Stage::Refused;
            return;
        };
        match plan(world, bloom, &objects.bundle, director) {
            Ok(Some(plan)) => {
                info!(
                    "[delivery-bloom] the place view's director {} is the exported director of {} ({} rigs); preparing",
                    plan.director,
                    plan.prefab,
                    plan.rigs.len()
                );
                bloom.stage = Stage::Preparing(plan);
            }
            Ok(None) => {
                // The manifest is loading; plan again next frame.
                bloom.epoch = None;
            }
            Err(reason) => {
                error!("[delivery-bloom] refused: {reason}; the tree does not bloom on this visit");
                bloom.stage = Stage::Refused;
            }
        }
        return;
    }
    match std::mem::take(&mut bloom.stage) {
        Stage::Idle => {}
        Stage::Refused => bloom.stage = Stage::Refused,
        Stage::Preparing(mut plan) => match prepare(world, bloom, &mut plan) {
            Ok(Some((request, flowers))) => {
                let refused: Vec<_> = {
                    let mut rows: Vec<_> = request
                        .bindings
                        .refused_controls
                        .values()
                        .cloned()
                        .collect();
                    rows.sort();
                    rows
                };
                for reason in &refused {
                    warn!("[delivery-bloom] {reason}; the rest of the timeline plays");
                }
                let duration = request.definition.duration;
                let bound = request.bindings.animations.len();
                let mut timelines = world.resource_mut::<FixtureActivityTimelines>();
                let token = timelines.request_start(request);
                timelines.set_paused(token, true);
                info!(
                    "[delivery-bloom] ResetView: the place view's director {} of {} (duration {duration:.4} s) prepared on {} flowers, {bound} animation clips bound, {} Control clips refused; paused, held at time 0",
                    plan.director,
                    plan.prefab,
                    flowers.len(),
                    refused.len()
                );
                bloom.stage = Stage::Running {
                    token,
                    played: false,
                    ended: false,
                    flowers,
                    traced: f64::NEG_INFINITY,
                };
            }
            Ok(None) => bloom.stage = Stage::Preparing(plan),
            Err(reason) => {
                error!("[delivery-bloom] refused: {reason}; the tree does not bloom on this visit");
                bloom.stage = Stage::Refused;
            }
        },
        Stage::Running {
            token,
            mut played,
            mut ended,
            flowers,
            mut traced,
        } => {
            let status = world
                .resource::<FixtureActivityTimelines>()
                .status(token)
                .cloned();
            match &status {
                Some(TimelineStatus::Failed(error)) => {
                    error!("[delivery-bloom] the director failed: {error}");
                    bloom.stage = Stage::Refused;
                    return;
                }
                Some(TimelineStatus::Cancelled) | None => {
                    error!("[delivery-bloom] the director was cancelled");
                    bloom.stage = Stage::Refused;
                    return;
                }
                _ => {}
            }
            if bloom.play_requested && !played {
                played = world
                    .resource_mut::<FixtureActivityTimelines>()
                    .set_paused(token, false);
                let time = world
                    .resource::<FixtureActivityTimelines>()
                    .sampled_time(token);
                info!(
                    "[delivery-bloom] PlayAnimation: _isFlowered on, Play from time {:?}; WaitUntil(time >= duration)",
                    time
                );
            }
            if matches!(status, Some(TimelineStatus::Completed)) && !ended {
                ended = true;
                let time = world
                    .resource::<FixtureActivityTimelines>()
                    .sampled_time(token)
                    .unwrap_or(f64::NAN);
                info!(
                    "[delivery-bloom] the director reached time {time:.4} (its duration): Pause, held at its end"
                );
            }
            if played {
                trace(world, token, &flowers, &mut traced, ended);
            }
            bloom.stage = Stage::Running {
                token,
                played,
                ended,
                flowers,
                traced,
            };
        }
    }
}

/// The session ends with the visit.
fn end(world: &mut World, bloom: &mut DeliveryBloom, why: &str) {
    if let Stage::Running { token, .. } = bloom.stage {
        timeline::cancel_and_release(world, token);
        info!("[delivery-bloom] the place view's director is released ({why})");
    }
    bloom.stage = Stage::Idle;
    bloom.epoch = None;
}

/// The manifest's director row for the place view's director.
fn plan(
    world: &mut World,
    bloom: &mut DeliveryBloom,
    bundle: &str,
    director: i64,
) -> Result<Option<Plan>, String> {
    let server = world.resource::<AssetServer>().clone();
    let handle = bloom
        .manifest
        .get_or_insert_with(|| server.load(format!("moly://{MANIFEST}")))
        .clone();
    if let LoadState::Failed(error) = server.load_state(&handle) {
        return Err(format!(
            "no site director was exported ({MANIFEST}: {error})"
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
        serde_json::from_str(&text).map_err(|error| format!("{MANIFEST}: {error}"))?;
    if document["version"].as_u64() != Some(1) {
        return Err(format!("{MANIFEST}: unsupported version"));
    }
    let package = format!("{SITE_PACKAGE_PREFIX}{bundle}");
    let wanted = director.to_string();
    let rows: Vec<&Value> = document["directors"]
        .as_object()
        .map(|rows| {
            rows.values()
                .filter(|row| {
                    row["package"].as_str() == Some(package.as_str())
                        && row["director"]["pathId"].as_str() == Some(wanted.as_str())
                })
                .collect()
        })
        .unwrap_or_default();
    let [row] = rows.as_slice() else {
        return Err(format!(
            "{} exported director rows of {package} name director {director}",
            rows.len()
        ));
    };
    let plain = |value: &Value, what: &str| -> Result<String, String> {
        value
            .as_str()
            .filter(|text| !text.is_empty() && !text.contains(['/', '\\', ':']))
            .map(str::to_owned)
            .ok_or_else(|| format!("{MANIFEST}: {what} is not a plain name"))
    };
    let asset = |value: &Value| -> Result<timeline::SourceAssetId, String> {
        Ok(timeline::SourceAssetId {
            file: value["file"]
                .as_str()
                .ok_or("an identity has no file")?
                .to_owned(),
            path_id: value["pathId"]
                .as_str()
                .ok_or("an identity has no pathId")?
                .to_owned(),
        })
    };
    let mut rigs = Vec::new();
    for rig in row["rigs"]
        .as_array()
        .ok_or("the director row has no rigs")?
    {
        let [clip] = rig["clips"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        else {
            return Err("a rig does not carry exactly one clip".into());
        };
        let tracks = rig["tracks"].as_array().ok_or("a rig has no tracks")?;
        let animators = rig["animators"]
            .as_array()
            .ok_or("a rig has no animators")?;
        if tracks.len() != animators.len() {
            return Err("a rig's tracks and animators differ in count".into());
        }
        let tracks = tracks
            .iter()
            .zip(animators)
            .map(|(track, animator)| -> Result<_, String> {
                let animator = animator["pathId"]
                    .as_str()
                    .and_then(|id| id.parse::<i64>().ok())
                    .ok_or("an animator identity is unreadable")?;
                Ok((asset(track)?, animator))
            })
            .collect::<Result<Vec<_>, String>>()?;
        rigs.push(Rig {
            glb: plain(&rig["glb"], "a rig glb")?,
            root: plain(&rig["root"], "a rig root")?,
            clip: plain(&clip["name"], "a rig clip")?,
            source_clip: asset(&clip["sourceClip"])?,
            tracks,
            handle: None,
        });
    }
    Ok(Some(Plan {
        package,
        prefab: plain(&row["prefab"], "the director prefab")?,
        director,
        rigs,
        draft: None,
        waiting: None,
    }))
}

/// The spawned site entity whose source object carries component `id`.
fn source_entity(world: &mut World, file: &str, id: i64) -> Result<Entity, String> {
    let mut query = world.query::<(Entity, &SourceObjectIdentity)>();
    let hits: Vec<Entity> = query
        .iter(world)
        .filter(|(_, identity)| identity.file == file && identity.components.contains(&id))
        .map(|(entity, _)| entity)
        .collect();
    match hits.as_slice() {
        [entity] => Ok(*entity),
        [] => Err(format!("no spawned site node carries component {id}")),
        many => Err(format!(
            "{} spawned site nodes carry component {id}",
            many.len()
        )),
    }
}

enum Wait {
    Pending(String),
    Refused(String),
}

fn prepare(
    world: &mut World,
    bloom: &mut DeliveryBloom,
    plan: &mut Plan,
) -> Result<Option<(StartTimeline, Vec<Entity>)>, String> {
    match try_prepare(world, bloom, plan) {
        Ok(ready) => Ok(Some(ready)),
        Err(Wait::Pending(reason)) => {
            if plan.waiting.as_deref() != Some(reason.as_str()) {
                info!(
                    "[delivery-bloom] the place view's director {} waits: {reason}",
                    plan.director
                );
                plan.waiting = Some(reason);
            }
            Ok(None)
        }
        Err(Wait::Refused(reason)) => Err(reason),
    }
}

fn try_prepare(
    world: &mut World,
    bloom: &mut DeliveryBloom,
    plan: &mut Plan,
) -> Result<(StartTimeline, Vec<Entity>), Wait> {
    let refuse = Wait::Refused;
    world.init_resource::<FixtureActivityProvider>();
    let definition: Arc<TimelineDefinition> = world
        .resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.site_definition(world, &plan.package, &plan.prefab)
        })
        .map_err(|pending| {
            let reason = format!("{}: {}", pending.stage, pending.reason);
            if pending.retryable {
                Wait::Pending(reason)
            } else {
                Wait::Refused(reason)
            }
        })?;
    if definition.director.path_id != plan.director.to_string() {
        return Err(refuse(format!(
            "the timeline tables select director {:?} for {}, the place view names {}",
            definition.director, plan.prefab, plan.director
        )));
    }
    let file = definition.director.file.clone();
    let director = source_entity(world, &file, plan.director).map_err(refuse)?;
    let mut fixture = director;
    while world.get::<CanonicalCoordinates>(fixture).is_none() {
        fixture = world
            .get::<ChildOf>(fixture)
            .map(ChildOf::parent)
            .ok_or_else(|| {
                refuse("the director's node has no coordinate-contract ancestor".into())
            })?;
    }
    // The rig files and their clips.
    let server = world.resource::<AssetServer>().clone();
    let mut clips = HashMap::new();
    for rig in &mut plan.rigs {
        let handle = rig
            .handle
            .get_or_insert_with(|| server.load(format!("moly://{ROOT}{}", rig.glb)))
            .clone();
        if let LoadState::Failed(error) = server.load_state(&handle) {
            return Err(refuse(format!("rig {} failed to load: {error}", rig.glb)));
        }
        let Some(gltf) = world.resource::<Assets<Gltf>>().get(&handle) else {
            return Err(Wait::Pending(format!("rig {} loading", rig.glb)));
        };
        let clip = gltf
            .named_animations
            .get(rig.clip.as_str())
            .cloned()
            .ok_or_else(|| refuse(format!("clip {} is absent from rig {}", rig.clip, rig.glb)))?;
        if world
            .resource::<Assets<AnimationClip>>()
            .get(&clip)
            .is_none()
        {
            return Err(Wait::Pending(format!(
                "clip {} of rig {} loading",
                rig.clip, rig.glb
            )));
        }
        clips.insert(rig.glb.clone(), clip);
    }
    bloom.generation += 1;
    let mut request = StartTimeline {
        owner: TimelineOwner {
            activity: FixtureActivityOwner {
                actor: director,
                generation: bloom.generation,
            },
            kind: TimelineOwnerKind::SceneDirector,
        },
        fixture,
        definition: definition.clone(),
        bindings: TimelineBindings::default(),
        companions: Vec::new(),
        timeout_secs: NO_TIMEOUT,
        timeout_budget: TimelineTimeoutBudget::PlayerWall,
    };
    if let Some(previous) = plan.draft.take() {
        request.bindings.silent_sounds = previous.silent_sounds;
        request.bindings.sounds = previous.sounds;
    }
    let mut flowers = Vec::new();
    for rig in &plan.rigs {
        for (track_id, animator_id) in &rig.tracks {
            let animator = source_entity(world, &file, *animator_id).map_err(refuse)?;
            let graph = rig_flower(world, animator, &rig.root);
            let track = definition
                .tracks
                .iter()
                .find(|track| &track.identity == track_id)
                .ok_or_else(|| {
                    refuse(format!(
                        "track {track_id:?} is not in the director's timeline"
                    ))
                })?;
            for clip in &track.clips {
                let TimelinePayload::Animation { target, .. } = &clip.payload else {
                    return Err(refuse(format!(
                        "track {} holds another payload",
                        track.name
                    )));
                };
                if target.source_clip.as_ref() != Some(&rig.source_clip) {
                    return Err(refuse(format!(
                        "track {} plays {}, the rig carries {}",
                        track.name, target.clip_name, rig.clip
                    )));
                }
                let source = SourceAnimationEvidence::from_clip_target(target)
                    .map_err(|error| refuse(error.to_string()))?;
                request.bindings.animations.insert(
                    clip.key.clone(),
                    TimelineAnimationBinding {
                        animator,
                        graph: graph.clone(),
                        clip: clips[&rig.glb].clone(),
                        source,
                        coverage: AnimationCoverage::sampled_pose(),
                    },
                );
            }
            flowers.push(animator);
        }
    }
    for track in &definition.tracks {
        for clip in &track.clips {
            if matches!(clip.payload, TimelinePayload::Animation { .. })
                && !request.bindings.animations.contains_key(&clip.key)
            {
                return Err(refuse(format!(
                    "animation track {} has no rig in the exported director row",
                    track.name
                )));
            }
        }
    }
    let silence = |world: &World, request: &mut StartTimeline| {
        for silenced in timeline::silence_unavailable_sounds(world, request) {
            warn!("[delivery-bloom] SE {silenced}; it plays silent");
        }
    };
    let pending = |error: timeline::TimelineFailure| {
        if error.retryable {
            Wait::Pending(error.to_string())
        } else {
            Wait::Refused(error.to_string())
        }
    };
    silence(world, &mut request);
    let sounds = timeline::prepare_source_sounds(world, &mut request);
    silence(world, &mut request);
    plan.draft = Some(request.bindings.clone());
    sounds.map_err(pending)?;
    timeline::prepare_source_effects(world, &mut request).map_err(pending)?;
    timeline::validate_start(world, &request).map_err(pending)?;
    Ok((request, flowers))
}

/// The flower's own animator, and each of its nodes named for the rig's
/// clip: `rig_root` then the names down the flower's subtree. A flower
/// rigged by an earlier visit of the same spawned site keeps its graph.
fn rig_flower(world: &mut World, animator: Entity, rig_root: &str) -> Handle<AnimationGraph> {
    if let Some(graph) = world.get::<AnimationGraphHandle>(animator) {
        return graph.0.clone();
    }
    let graph = world
        .resource_mut::<Assets<AnimationGraph>>()
        .add(AnimationGraph::new());
    world.entity_mut(animator).insert((
        AnimationPlayer::default(),
        AnimationGraphHandle(graph.clone()),
    ));
    let mut stack = vec![(animator, vec![Name::new(rig_root.to_owned())])];
    while let Some((entity, names)) = stack.pop() {
        world.entity_mut(entity).insert((
            AnimationTargetId::from_names(names.iter()),
            AnimatedBy(animator),
        ));
        let children: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|children| children.iter().collect())
            .unwrap_or_default();
        for child in children {
            let Some(name) = world.get::<Name>(child).cloned() else {
                continue;
            };
            let mut path = names.clone();
            path.push(name);
            stack.push((child, path));
        }
    }
    graph
}

/// Every quarter second while the director runs: its time and what plays on
/// the first flowers (read from their players).
fn trace(
    world: &mut World,
    token: TimelineToken,
    flowers: &[Entity],
    traced: &mut f64,
    ended: bool,
) {
    let now = world
        .get_resource::<Time>()
        .map_or(0.0, Time::elapsed_secs_f64);
    // Every quarter second while it plays, then one line once it holds.
    if *traced == f64::INFINITY || now - *traced < 0.25 {
        return;
    }
    *traced = if ended { f64::INFINITY } else { now };
    let time = world
        .resource::<FixtureActivityTimelines>()
        .sampled_time(token);
    let rows: Vec<String> = flowers
        .iter()
        .take(3)
        .map(|flower| {
            let playing: Vec<String> = world
                .get::<AnimationPlayer>(*flower)
                .map(|player| {
                    player
                        .playing_animations()
                        .map(|(_, active)| {
                            format!("t {:.3} w {:.2}", active.seek_time(), active.weight())
                        })
                        .collect()
                })
                .unwrap_or_default();
            format!(
                "{flower:?} {:?} [{}]",
                world.get::<Name>(*flower).map(Name::as_str),
                playing.join(", ")
            )
        })
        .collect();
    info!(
        "[delivery-bloom-trace] director time {}{}: flowers {}",
        time.map_or("-".into(), |t| format!("{t:.4}")),
        if ended { " (held at its end)" } else { "" },
        rows.join("; ")
    );
}
