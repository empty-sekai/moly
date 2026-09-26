//! The home site's expansion cut-scene.
//!
//! Entry: `ScreenLayerMysekaiHome.OnFinishStartAnimation` runs
//! `SiteActionExecutor.PlayHomeSitePerformAsync` when no performance is
//! executing; its first step with a visible effect is
//! `PlaySiteExpansionCutSceneIfNeeded(siteId)`: when the site needs an
//! expansion ([`crate::site_expansion`]), it waits until no UI layer is
//! working, then `MysekaiUtility.PlayUnlockSiteLevelCutScene(level)`:
//!
//! - `GetMasterMysekaiCutScene(level id, mysekai_site_level)`; with no row it
//!   saves the unlocked level and publishes the rank-up event only;
//! - otherwise `SetCanUpdateDitherController(false)`, `SaveUnlockSiteLevel`,
//!   `CutSceneExecutor.HideFixtureAll` (every fixture that is neither the gate
//!   nor the house is hidden), then `CutSceneExecutor.PlayAsync` with the
//!   current site's transform as the start transform.
//!
//! `CutSceneExecutor.PlayAsync`: the cut-scene view is instantiated under the
//! cut-scene root and placed at the start transform
//! (`CutSceneView.Setup`), its assets load, the presenter fades the cut-scene
//! screen in (`FadeIn`, 0.5 s to opaque black), then `SetupInternal`: game
//! state 5 (`CutSceneGameState.OnEnter`: the cut-scene screen and camera
//! state 9), the near fixtures and lists, the Cinemachine brain bound to the
//! field camera, and the view activated. `CutSceneView.PlayAsync` plays the
//! director (extrapolation Hold) until its time reaches its duration.
//! `EndAsync`: `ResetCameraState` (game state Normal, the brain removed, the
//! camera planes reset), the view disposed (its graph stops), the fixtures
//! shown (`ShowAllFixtures` shows every fixture), then the presenter fades
//! the cut-scene screen out (`FadeOut`, 0.5 s).
//!
//! The director is a session of the shared timeline runner owned as a
//! cut-scene; this module drives its cut-scene tracks from the runner's
//! sampled time:
//!
//! - Cinemachine: the shot's virtual camera by the director's exposed
//!   reference ([`crate::cutscene_camera`]);
//! - fade panel: `FadePanelBehaviour.UpdateAlpha` while a clip is active
//!   (fade out: alpha `1 - p`, fade in: alpha `p`, `p` the clip progress,
//!   held at 0 or 1 once `p > 0.95`) and `FadePanelMixerBehaviour.ClipSetup`
//!   once a clip has ended (0 or 1), both through the screen's `SetAlpha`;
//! - update obstacle: `HomeSiteController.UpdateObstacle(level)` on the
//!   clip's play edge;
//! - hide site obstacle: `SetDither(level, 1)` on play, `SetDither(level,
//!   1 - time/duration)` each frame, `SetDither(level, 0)` when the graph
//!   stops; logged (named render gap: the ring dither is not drawn);
//! - site expansion: `SetSiteExtension` each frame with the clip's centre,
//!   colours and radius `start + t (end - start)`, and `ResetGlobalDissolve`
//!   when the graph stops, through [`crate::site_extension`] (the site
//!   shaders' global dissolve);
//! - effect clips (`EffectTrack`, `EffectBehaviour`): the template prefab
//!   is instantiated inactive with no parent (built from the package's
//!   particle document), `SetEffectInstance` at graph build lists its
//!   systems, takes the first as the root particle and activates the
//!   instance, whose playOnAwake systems play; the clip's play edge places
//!   it at the clip offset and plays the root particle, its end stops it
//!   (`Stop()`, or stop and clear for a looping matched-duration clip), and
//!   the graph's end destroys it; played and stopped through the particle
//!   host;
//! - control clips: driven by the runner through the particle host where
//!   the director's exposed reference resolves to a spawned node, refused
//!   by name otherwise;
//! - SE: played by the runner; an SE with no audio route plays silent.
//!
//! Named differences: the cut-scene root's own pose is not in any package and
//! is taken as the identity; the level-release dialog, the rank-up dialog,
//! the mission screen and the housing competition step are logged only; the
//! current site keeps its floor grid until it next loads (the displayed
//! level is written for that load).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use crate::cutscene_camera::{self, Shot};
use crate::fixture_activity_provider::FixtureActivityProvider;
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner};
use crate::fixture_activity_timeline::{
    self as timeline, CutScenePayload, ExpansionEffect, FixtureActivityTimelines, StartTimeline,
    TimelineBindings, TimelineClip, TimelineClipKey, TimelineDefinition, TimelineOwner,
    TimelineOwnerKind, TimelinePayload, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
};
use crate::fixture_timeline_particles::{self as particles, ParticlePlayBinding};
use crate::game_state::{self, GameStateType};
use crate::site::{HomeObstacleLevel, HomeObstacleRing, SiteActive, SiteSelection};
use crate::site_expansion::law::SiteLevelRow;
use crate::site_expansion::{ExpansionMasters, MysekaiLocalSettings, UserMysekaiRank};

const PACKAGE_PREFIX: &str = "mysekai__cut_scene__";
/// `PlayableDirector.Play` has no timeout.
const NO_TIMEOUT: f64 = f64::MAX;
/// The cut-scene screen's `FadeIn`/`FadeOut` default duration.
const LAYER_FADE: f32 = 0.5;
/// `EnvironmentSiteExtensionData.Default`: `DissolveSmoothness`.
const DEFAULT_SMOOTHNESS: f32 = 0.2;
/// `EnvironmentSiteExtensionData.Default`: `LimitLineWidth`.
const DEFAULT_LIMIT_LINE_WIDTH: f32 = 0.0;
/// `FadePanelBehaviour.UpdateAlpha`: past this progress the clip holds its
/// end value.
const FADE_HOLD: f32 = 0.95;
/// Product report interval, not a source value: a load still waiting is
/// named every this many real seconds (the source awaits it with no
/// timeout).
const LOAD_REPORT_REAL: f64 = 30.0;

/// `ScreenLayerMysekaiHome.OnFinishStartAnimation`: the home screen became
/// current (the entry's home setup, a door or cannon arrival at home).
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct HomeScreenStartAnimation {
    pub(crate) caller: &'static str,
}

/// A virtual camera of the prefab: its GameObject, world pose (source frame,
/// relative to the prefab root) and lens.
#[derive(Clone, Debug)]
struct VirtualCamera {
    name: String,
    position: Vec3,
    rotation: Quat,
    fov: f32,
    near: f32,
    far: f32,
}

/// An effect prefab an effect clip instantiates.
#[derive(Clone, Debug)]
struct EffectPrefab {
    name: String,
    /// The prefab's serialized file and root GameObject.
    file: String,
    game_object: i64,
    /// The GameObjects that carry a ParticleSystem, in the hierarchy's
    /// depth-first order (children in their serialized order): the order
    /// `GetComponentsInChildren<ParticleSystem>` lists them in.
    particle_systems: Vec<i64>,
    root_scripts: Vec<String>,
}

/// An effect clip's template instance (`EffectTrack.CreateEffectObject`)
/// and what `EffectBehaviour.SetEffectInstance` reads from it.
struct EffectInstance {
    /// The clip's display name.
    name: String,
    prefab: String,
    /// The instance root (inactive until `SetEffectInstance` activates it).
    root: Entity,
    /// `rootParticle`: the first system the instance lists, by node path.
    root_particle: String,
    /// The root particle's subtrees the particle host plays, each prepared
    /// as one object (`Play()` and `Stop()` on the root particle reach every
    /// system below it), by node path.
    played: Vec<(String, ParticlePlayBinding)>,
    /// The listed systems below the root particle the host does not play,
    /// each with the reason.
    not_played: Vec<String>,
    /// How many systems `GetComponentsInChildren<ParticleSystem>()` lists.
    listed: usize,
    /// The listed systems whose serialized seed is not the manual seed 0
    /// (automatic, or another manual seed).
    auto_seeded: Vec<String>,
    /// How many systems the activation plays (playOnAwake, listed).
    awake: usize,
    included_loop: bool,
    matched_duration: bool,
    random_seed: bool,
    /// `offset`, in the source frame.
    offset: Vec3,
}

/// An effect clip's instance while it is being built and prepared.
enum EffectSlot {
    Pending(Option<Entity>),
    Ready(Box<EffectInstance>),
    /// Refused by name: the rest of the cut-scene plays.
    Refused,
}

struct Load {
    unlock: SiteLevelRow,
    rank: i32,
    package: String,
    prefab: String,
    tracks: Handle<JsonAsset>,
    /// The particle index and the package's particle document the effect
    /// clips' template prefabs are built from.
    particle_index: Handle<JsonAsset>,
    particle_document: Option<Handle<JsonAsset>>,
    particles: Option<Arc<Value>>,
    effects: HashMap<TimelineClipKey, EffectSlot>,
    root: Option<Entity>,
    draft: Option<TimelineBindings>,
    waiting: Option<String>,
    since_real: f64,
    /// How many report intervals the wait has passed.
    reported: u32,
}

struct Plan {
    unlock: SiteLevelRow,
    rank: i32,
    prefab: String,
    root: Entity,
    request: Option<StartTimeline>,
    definition: Arc<TimelineDefinition>,
    /// Exposed name -> virtual camera.
    cameras: HashMap<String, VirtualCamera>,
    effects: HashMap<String, EffectPrefab>,
    /// The view's `_hideFixtureDistance`, white and black lists.
    hide_distance: i64,
    white: Vec<String>,
    black: Vec<String>,
    /// The start transform's position (source frame).
    start: Vec3,
    /// SE clips that play silent.
    silent: HashSet<TimelineClipKey>,
    /// The effect clips' prepared instances.
    instances: HashMap<TimelineClipKey, EffectInstance>,
}

#[derive(Default)]
struct Play {
    token: Option<TimelineToken>,
    active: HashSet<TimelineClipKey>,
    fade_index: Option<usize>,
    /// Levels given to `SetDither` and their last value.
    dither: HashMap<i32, f32>,
    dissolve: bool,
    camera_logged: f64,
    fade_logged: f64,
    /// The graph's first frame ran (`SetEffectInstance` at graph build).
    graph_built: bool,
    /// The real time of the last live-particle sample of the effect
    /// instances.
    effect_sampled: f64,
}

enum Stage {
    WaitUi {
        unlock: SiteLevelRow,
        rank: i32,
        logged: bool,
    },
    Load(Box<Load>),
    PresenterFadeIn(Box<Plan>),
    Play(Box<Plan>, Box<Play>),
    PresenterFadeOut,
}

/// `PlayHomeSitePerformAsync` in flight (the home screen's `_isExecuting`).
#[derive(Resource)]
pub(crate) struct HomePerform {
    stage: Stage,
}

fn now_real(world: &World) -> f64 {
    world.resource::<Time<Real>>().elapsed_secs_f64()
}

/// Update: the home screen's start animation finished.
pub(crate) fn home_started(world: &mut World) {
    let events: Vec<HomeScreenStartAnimation> = {
        let mut messages = world.resource_mut::<Messages<HomeScreenStartAnimation>>();
        messages.drain().collect()
    };
    for event in events {
        if world.contains_resource::<HomePerform>() {
            info!(
                "[cutscene] ScreenLayerMysekaiHome.OnFinishStartAnimation ({}): _isExecuting; PlayHomeSitePerformAsync is not started again",
                event.caller
            );
            continue;
        }
        play_home_site_perform(world, event.caller);
    }
}

/// `PlayHomeSitePerformAsync` up to its cut-scene step.
fn play_home_site_perform(world: &mut World, caller: &str) {
    let Some(rank) = world.get_resource::<UserMysekaiRank>().map(|rank| rank.0) else {
        // No reply has delivered a rank change.
        return;
    };
    let Some(site) = world.get_resource::<SiteActive>().cloned() else {
        return;
    };
    if site.site_type != "home_site" {
        return;
    }
    let masters = match world
        .get_resource::<ExpansionMasters>()
        .map(|m| m.0.clone())
    {
        Some(Ok(masters)) => masters,
        _ => return,
    };
    let site_id = site.site_id as i32;
    let local = &world.resource::<MysekaiLocalSettings>().0;
    let need = local.is_need_site_expansion_action(&masters, rank, site_id);
    let unlock = local
        .unlock_master_site_level(&masters, rank, site_id)
        .cloned();
    info!(
        "[cutscene] ScreenLayerMysekaiHome.OnFinishStartAnimation ({caller}): PlayHomeSitePerformAsync: CheckBanInfo (logged only); site {site_id}; PlaySiteExpansionCutSceneIfNeeded: IsNeedSiteExpansionAction = {need} (topics {:?}, unlock {:?}, UnlockedMasterSiteLevelIds {:?})",
        local.topics,
        unlock.as_ref().map(|row| (row.id, row.level)),
        local.unlocked_master_site_level_ids
    );
    match unlock.filter(|_| need) {
        None => after_cutscene_step(world),
        Some(unlock) => {
            world.insert_resource(HomePerform {
                stage: Stage::WaitUi {
                    unlock,
                    rank,
                    logged: false,
                },
            });
        }
    }
}

/// The steps after the cut-scene step of `PlayHomeSitePerformAsync`.
fn after_cutscene_step(world: &mut World) {
    let _ = world;
    info!("[cutscene] PlayHomeSitePerformAsync: ShowPlayerRankUpDialog, ExpansionToMissionScreen and HousingCompetitionReviewed are UI steps logged only; ends");
}

/// Update (exclusive), after the timeline runner.
pub(crate) fn advance(world: &mut World) {
    let Some(mut perform) = world.remove_resource::<HomePerform>() else {
        return;
    };
    let done = step(world, &mut perform);
    if !done {
        world.insert_resource(perform);
    }
}

fn step(world: &mut World, perform: &mut HomePerform) -> bool {
    let stage = std::mem::replace(&mut perform.stage, Stage::PresenterFadeOut);
    match stage {
        Stage::WaitUi {
            unlock,
            rank,
            logged,
        } => {
            let working = !world
                .get_resource::<crate::ui_layers::UiLayerStack>()
                .is_some_and(|layers| layers.on_field());
            if working {
                if !logged {
                    info!("[cutscene] WaitUntil(!IsUILayerWorking): a UI layer is working");
                }
                perform.stage = Stage::WaitUi {
                    unlock,
                    rank,
                    logged: true,
                };
                return false;
            }
            let site_id = unlock.site_id;
            match unlock_cutscene(world, unlock, rank) {
                Some(load) => perform.stage = Stage::Load(Box::new(load)),
                None => {
                    level_release_dialog(world, site_id, rank);
                    after_cutscene_step(world);
                    return true;
                }
            }
            false
        }
        Stage::Load(mut load) => match try_load(world, &mut load) {
            Ok(Some(plan)) => {
                presenter_fade_in(world, &plan);
                perform.stage = Stage::PresenterFadeIn(Box::new(plan));
                false
            }
            Ok(None) => {
                perform.stage = Stage::Load(load);
                false
            }
            Err(reason) => {
                error!("[cutscene] {}/{}: refused: {reason}; the cut-scene does not play (the fixtures are shown again)", load.package, load.prefab);
                if let Some(root) = load.root {
                    world.despawn(root);
                }
                show_all_fixtures(world);
                level_release_dialog(world, load.unlock.site_id, load.rank);
                after_cutscene_step(world);
                true
            }
        },
        Stage::PresenterFadeIn(mut plan) => {
            if !world
                .resource::<crate::screen_fade::CutSceneFadeImage>()
                .finished()
            {
                perform.stage = Stage::PresenterFadeIn(plan);
                return false;
            }
            let play = setup_internal(world, &mut plan);
            perform.stage = Stage::Play(plan, Box::new(play));
            false
        }
        Stage::Play(plan, mut play) => {
            if play_frame(world, &plan, &mut play) {
                end_async(world, &plan, &play);
                perform.stage = Stage::PresenterFadeOut;
                // Keep the plan's data for the dialog step.
                world.insert_resource(PendingDialog {
                    site_id: plan.unlock.site_id,
                    rank: plan.rank,
                });
            } else {
                perform.stage = Stage::Play(plan, play);
            }
            false
        }
        Stage::PresenterFadeOut => {
            if !world
                .resource::<crate::screen_fade::CutSceneFadeImage>()
                .finished()
            {
                perform.stage = Stage::PresenterFadeOut;
                return false;
            }
            let hold = world.remove_resource::<game_state::UiHold>();
            info!("[cutscene] CutScenePresenter.FadeOutAsync done; CutSceneExecutor.PlayAsync: Dispose; the cut-scene screen closes (UI interactable again; hold {:?} released)", hold.map(|hold| hold.0));
            let pending = world.remove_resource::<PendingDialog>();
            level_release_dialog(
                world,
                pending.map_or(0, |p| p.site_id),
                pending.map_or(0, |p| p.rank),
            );
            after_cutscene_step(world);
            true
        }
    }
}

#[derive(Resource, Clone, Copy)]
struct PendingDialog {
    site_id: i32,
    rank: i32,
}

fn level_release_dialog(world: &mut World, site_id: i32, rank: i32) {
    let _ = world;
    info!("[cutscene] OpenCurrentMysekaiSiteLevelReleaseDialogAsync(site {site_id}, rank {rank}): the level-release dialog is a UI dialog not drawn here");
}

/// `PlayUnlockSiteLevelCutScene(level)`.
fn unlock_cutscene(world: &mut World, unlock: SiteLevelRow, rank: i32) -> Option<Load> {
    let masters = match world
        .get_resource::<ExpansionMasters>()
        .map(|m| m.0.clone())
    {
        Some(Ok(masters)) => masters,
        _ => return None,
    };
    let row = match masters.site_level_cutscene(unlock.id) {
        Ok(row) => row.cloned(),
        Err(reason) => {
            error!("[cutscene] PlayUnlockSiteLevelCutScene({}): refused: {reason}; the unlocked level is not saved", unlock.id);
            return None;
        }
    };
    let Some(row) = row else {
        save_unlock(world, &unlock);
        info!(
            "[cutscene] PlayUnlockSiteLevelCutScene({}): GetMasterMysekaiCutScene({}, mysekai_site_level) = null: SaveUnlockSiteLevel; publish MysekaiRankUpEventData (no subscriber here)",
            unlock.id, unlock.id
        );
        return None;
    };
    info!(
        "[cutscene] PlayUnlockSiteLevelCutScene({}, level {}): GetMasterMysekaiCutScene({}, mysekai_site_level) = row {} timelineAssetbundleName {}; SiteObjectManager.SetCanUpdateDitherController(false) (logged only)",
        unlock.id, unlock.level, unlock.id, row.id, row.bundle
    );
    save_unlock(world, &unlock);
    hide_fixture_all(world);
    info!("[cutscene] CutSceneExecutor.PlayAsync: CutSceneRoot found (its own pose taken as identity); MagicaCloth RequestReset (logged only); FadeAndSetUpAsync");
    let package = format!("{PACKAGE_PREFIX}{}", row.bundle);
    let server = world.resource::<AssetServer>().clone();
    let tracks =
        server.load::<JsonAsset>(format!("moly://cutscene-timeline/tracks/{package}.json"));
    let particle_index = server.load::<JsonAsset>("moly://fixture-particles-v2/index.json");
    Some(Load {
        unlock,
        rank,
        prefab: row.bundle.clone(),
        package,
        tracks,
        particle_index,
        particle_document: None,
        particles: None,
        effects: HashMap::new(),
        root: None,
        draft: None,
        waiting: None,
        since_real: now_real(world),
        reported: 0,
    })
}

/// `MysekaiTopicsManager.SaveUnlockSiteLevel`, and the displayed level the
/// next load of the site reads.
fn save_unlock(world: &mut World, unlock: &SiteLevelRow) {
    world
        .resource_mut::<MysekaiLocalSettings>()
        .0
        .save_unlock_site_level(unlock.id);
    let site_type = world
        .get_resource::<ExpansionMasters>()
        .and_then(|m| m.0.as_ref().ok().cloned())
        .and_then(|masters| {
            masters
                .site_types
                .iter()
                .find(|(id, _)| *id == unlock.site_id)
                .map(|(_, kind)| kind.clone())
        });
    if let (Some(site_type), Some(mut selection)) = (
        site_type.as_deref(),
        world.get_resource_mut::<SiteSelection>(),
    ) {
        selection.set_level(site_type, unlock.level as u32);
    }
    let local = &world.resource::<MysekaiLocalSettings>().0;
    info!(
        "[cutscene] SaveUnlockSiteLevel({}) -> UnlockedMasterSiteLevelIds {:?}; topics {:?}; {} displays level {} from its next load",
        unlock.id,
        local.unlocked_master_site_level_ids,
        local.topics,
        site_type.as_deref().unwrap_or("?"),
        unlock.level
    );
}

/// `CutSceneExecutor.HideFixtureAll`: every fixture that is neither the gate
/// nor the house is hidden.
fn hide_fixture_all(world: &mut World) {
    let placed: Vec<(Entity, String)> = {
        let mut query = world.query::<(Entity, &FixtureActivityIdentity)>();
        query
            .iter(world)
            .map(|(entity, identity)| (entity, identity.model_package.clone()))
            .collect()
    };
    let homes = world
        .get_resource::<crate::entry::house::HomeFixtures>()
        .map(|homes| {
            placed
                .iter()
                .map(|(entity, package)| (*entity, homes.can_clean_up(package)))
                .collect::<Vec<_>>()
        });
    let Some(rows) = homes else {
        warn!("[cutscene] HideFixtureAll: the home fixture tables are not loaded; no fixture is hidden");
        return;
    };
    let mut hidden = 0;
    let mut kept = 0;
    for (entity, hide) in rows {
        if hide {
            if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
                *visibility = Visibility::Hidden;
                hidden += 1;
            }
        } else {
            kept += 1;
        }
    }
    info!("[cutscene] CutSceneExecutor.HideFixtureAll: {hidden} fixtures hidden, {kept} kept (the gate and the house)");
}

/// `CutScenePresenter.ShowAllFixtures`: every fixture is shown.
fn show_all_fixtures(world: &mut World) {
    let mut query = world.query_filtered::<&mut Visibility, With<FixtureActivityIdentity>>();
    let mut shown = 0;
    for mut visibility in query.iter_mut(world) {
        *visibility = Visibility::Inherited;
        shown += 1;
    }
    info!("[cutscene] ShowAllFixtures: {shown} fixtures shown");
}

fn source_vec3(value: &Value) -> Option<Vec3> {
    Some(Vec3::new(
        value["x"].as_f64()? as f32,
        value["y"].as_f64()? as f32,
        value["z"].as_f64()? as f32,
    ))
}

fn source_quat(value: &Value) -> Option<Quat> {
    Some(Quat::from_xyzw(
        value["x"].as_f64()? as f32,
        value["y"].as_f64()? as f32,
        value["z"].as_f64()? as f32,
        value["w"].as_f64()? as f32,
    ))
}

/// The prefab record's virtual cameras, effect prefabs and view fields.
#[allow(clippy::type_complexity)]
fn read_prefab(
    document: &Value,
    prefab: &str,
) -> Result<
    (
        HashMap<String, VirtualCamera>,
        HashMap<String, EffectPrefab>,
        (i64, Vec<String>, Vec<String>),
    ),
    String,
> {
    let prefabs = document["prefabs"]
        .as_array()
        .ok_or("the track table has no prefab records (exported by an older reader)")?;
    let wanted = format!("/{prefab}.prefab");
    let record = prefabs
        .iter()
        .find(|row| {
            row["container"]
                .as_str()
                .is_some_and(|c| c.ends_with(&wanted))
        })
        .ok_or_else(|| format!("no prefab record {prefab}"))?;
    let cinemachine = record["cinemachine"]
        .as_array()
        .ok_or("the prefab record has no Cinemachine table (exported before it was)")?;
    let directors = record["directors"]
        .as_array()
        .ok_or("the prefab record has no directors")?;
    let [director] = directors.as_slice() else {
        return Err(format!("{} directors on the prefab", directors.len()));
    };
    // Component path id -> virtual camera.
    let mut by_component = HashMap::new();
    for row in cinemachine {
        if row["script"].as_str() != Some("CinemachineVirtualCamera") {
            continue;
        }
        let component = row["asset"]["pathId"].as_str().unwrap_or("").to_owned();
        let fields = &row["fields"];
        for key in ["m_Follow", "m_LookAt"] {
            if fields[key]["m_PathID"].as_str().unwrap_or("0") != "0" {
                return Err(format!(
                    "virtual camera {component} has a {key} target; its body and aim are not ported"
                ));
            }
        }
        let lens = &fields["m_Lens"];
        let chain = row["transforms"]
            .as_array()
            .ok_or("a virtual camera has no transform chain")?;
        // The prefab root is placed at the start transform by Setup; the
        // chain below it composes in the source frame.
        let mut world_transform = Transform::IDENTITY;
        for (index, node) in chain.iter().enumerate() {
            let scale = source_vec3(&node["scale"]).ok_or("a transform has no scale")?;
            let local = if index == 0 {
                Transform::from_scale(scale)
            } else {
                Transform {
                    translation: source_vec3(&node["position"])
                        .ok_or("a transform has no position")?,
                    rotation: source_quat(&node["rotation"])
                        .ok_or("a transform has no rotation")?,
                    scale,
                }
            };
            world_transform = world_transform.mul_transform(local);
        }
        by_component.insert(
            component,
            VirtualCamera {
                name: chain
                    .last()
                    .and_then(|node| node["name"].as_str())
                    .unwrap_or("?")
                    .to_owned(),
                position: world_transform.translation,
                rotation: world_transform.rotation,
                fov: lens["FieldOfView"]
                    .as_f64()
                    .ok_or("a lens has no FieldOfView")? as f32,
                near: lens["NearClipPlane"]
                    .as_f64()
                    .ok_or("a lens has no NearClipPlane")? as f32,
                far: lens["FarClipPlane"]
                    .as_f64()
                    .ok_or("a lens has no FarClipPlane")? as f32,
            },
        );
    }
    let mut cameras = HashMap::new();
    for row in director["exposedReferences"]
        .as_array()
        .ok_or("the director has no exposed-reference table")?
    {
        let (Some(name), Some(path)) = (row["name"].as_str(), row["value"]["pathId"].as_str())
        else {
            continue;
        };
        if let Some(camera) = by_component.get(path) {
            cameras.insert(name.to_owned(), camera.clone());
        }
    }
    // The prefabs the effect clips instantiate (`EffectClip.template.prefab`),
    // by the clip's playable asset.
    let mut effects = HashMap::new();
    let rows = document["effectPrefabs"]
        .as_array()
        .ok_or("the track table has no effect prefab records (exported before they were)")?;
    for row in rows {
        let prefab = &row["prefab"];
        if prefab.is_null() {
            continue;
        }
        let game_object = |row: &Value| -> Result<i64, String> {
            row["pathId"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| format!("an effect prefab record has an unreadable pathId: {row}"))
        };
        let effect = EffectPrefab {
            name: prefab["name"].as_str().unwrap_or("?").to_owned(),
            file: prefab["asset"]["file"].as_str().unwrap_or("").to_owned(),
            game_object: game_object(&prefab["asset"])?,
            particle_systems: prefab["particleSystems"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| game_object(&row["gameObject"]))
                .collect::<Result<_, _>>()?,
            root_scripts: prefab["rootScripts"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        };
        for clip in row["clips"].as_array().into_iter().flatten() {
            if let (Some(file), Some(path)) = (clip["file"].as_str(), clip["pathId"].as_str()) {
                effects.insert(format!("{file}/{path}"), effect.clone());
            }
        }
    }
    let view = record["views"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["script"].as_str() == Some("CutSceneView"))
        })
        .ok_or("the prefab root has no CutSceneView")?;
    let fields = &view["fields"];
    let names = |key: &str| -> Vec<String> {
        fields[key]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((
        cameras,
        effects,
        (
            fields["_hideFixtureDistance"].as_i64().unwrap_or(0),
            names("_whiteListFixtureNames"),
            names("_blackListFixtureNames"),
        ),
    ))
}

/// `FadeAndSetUpAsync`: the view is instantiated and placed, its assets and
/// the director's tables load and the session is prepared.
fn try_load(world: &mut World, load: &mut Load) -> Result<Option<Plan>, String> {
    // `LoadAssetAndSetUpAsync` is awaited with no timeout, and the fixtures
    // stay hidden meanwhile, as in the source. Only the product's own
    // preparation (the effect clips' particle programs) takes this long; a
    // failed load refuses at once below.
    let waited = now_real(world) - load.since_real;
    if waited >= LOAD_REPORT_REAL * f64::from(load.reported + 1) {
        load.reported += 1;
        warn!(
            "[cutscene] {}/{} still waiting after {waited:.0} s: {}",
            load.package,
            load.prefab,
            load.waiting.as_deref().unwrap_or("?")
        );
    }
    let server = world.resource::<AssetServer>().clone();
    if let LoadState::Failed(error) = server.load_state(&load.tracks) {
        return Err(format!(
            "the cut-scene track table is not in this root ({error})"
        ));
    }
    let Some(document) = world
        .resource::<Assets<JsonAsset>>()
        .get(&load.tracks)
        .map(|json| json.0.clone())
    else {
        wait(load, "the track table is loading");
        return Ok(None);
    };
    let document: Value =
        serde_json::from_str(&document).map_err(|error| format!("track table: {error}"))?;
    let (cameras, effects, (hide_distance, white, black)) = read_prefab(&document, &load.prefab)?;
    world.init_resource::<FixtureActivityProvider>();
    let package = load.package.clone();
    let prefab = load.prefab.clone();
    let definition =
        match world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.cutscene_definition(world, &package, &prefab)
        }) {
            Ok(definition) => definition,
            Err(pending) if pending.retryable => {
                wait(load, &format!("{}: {}", pending.stage, pending.reason));
                return Ok(None);
            }
            Err(pending) => return Err(format!("{}: {}", pending.stage, pending.reason)),
        };
    let start = world
        .get_resource::<SiteActive>()
        .map_or(Vec3::ZERO, |site| Vec3::from_array(site.position));
    // The view's frame is the product frame: every pose read from the
    // prefab (the virtual cameras) is converted explicitly on read.
    let root = *load.root.get_or_insert_with(|| {
        world
            .spawn((
                Name::new(format!("cutscene {}", load.prefab)),
                Transform::default(),
                Visibility::default(),
                moly_assets::coordinates::CanonicalCoordinates,
            ))
            .id()
    });
    let mut request = StartTimeline {
        owner: TimelineOwner {
            activity: FixtureActivityOwner {
                actor: root,
                generation: 1,
            },
            kind: TimelineOwnerKind::CutScene,
        },
        fixture: root,
        definition: definition.clone(),
        bindings: TimelineBindings::default(),
        companions: Vec::new(),
        timeout_secs: NO_TIMEOUT,
        timeout_budget: TimelineTimeoutBudget::PlayerWall,
    };
    if let Some(previous) = load.draft.take() {
        request.bindings.silent_sounds = previous.silent_sounds;
        request.bindings.sounds = previous.sounds;
    }
    // An SE with no audio route plays silent (the runner's player-owner
    // rule); the cut-scene logs its play edge either way.
    for silenced in timeline::silence_unavailable_sounds(world, &mut request) {
        warn!("[cutscene] SE {silenced}; it plays silent");
    }
    let sounds = timeline::prepare_source_sounds(world, &mut request);
    load.draft = Some(request.bindings.clone());
    if let Err(error) = sounds {
        if error.retryable {
            wait(load, &error.to_string());
            return Ok(None);
        }
        return Err(error.to_string());
    }
    if let Err(error) = timeline::prepare_source_effects(world, &mut request) {
        return Err(error.to_string());
    }
    if !prepare_effects(world, load, &definition, &effects, root) {
        return Ok(None);
    }
    if let Err(error) = timeline::validate_start(world, &request) {
        if error.retryable {
            wait(load, &error.to_string());
            return Ok(None);
        }
        return Err(error.to_string());
    }
    info!(
        "[cutscene] {}/{}: Factory: view instantiated, CutSceneView.Setup(CutSceneRoot, the site transform at ({:.1},{:.1},{:.1})); director {} (duration {:.4} s, {} tracks, {} virtual cameras by exposed name, {} Control clips driven, {} Control clips refused, {} effect clip instances prepared); CutScenePresenter: SetIsNeedLowHeightDither (logged only); LoadAssetAndSetUpAsync done",
        load.package,
        load.prefab,
        start.x,
        start.y,
        start.z,
        definition.director.path_id,
        definition.duration,
        definition.tracks.len(),
        cameras.len(),
        request.bindings.controls.len(),
        request.bindings.refused_controls.len(),
        load.effects
            .values()
            .filter(|slot| matches!(slot, EffectSlot::Ready(_)))
            .count()
    );
    let silent = request.bindings.silent_sounds.clone();
    let instances = std::mem::take(&mut load.effects)
        .into_iter()
        .filter_map(|(key, slot)| match slot {
            EffectSlot::Ready(instance) => Some((key, *instance)),
            _ => None,
        })
        .collect();
    Ok(Some(Plan {
        instances,
        silent,
        unlock: load.unlock.clone(),
        rank: load.rank,
        prefab: load.prefab.clone(),
        root,
        request: Some(request),
        definition,
        cameras,
        effects,
        hide_distance,
        white,
        black,
        start,
    }))
}

fn wait(load: &mut Load, reason: &str) {
    if load.waiting.as_deref() != Some(reason) {
        info!(
            "[cutscene] {}/{} waits: {reason}",
            load.package, load.prefab
        );
        load.waiting = Some(reason.to_owned());
    }
}

/// The package's particle document; `Ok(None)` while it loads.
fn particle_document(world: &mut World, load: &mut Load) -> Result<Option<Arc<Value>>, String> {
    if let Some(document) = &load.particles {
        return Ok(Some(document.clone()));
    }
    let server = world.resource::<AssetServer>().clone();
    let json = |world: &World, handle: &Handle<JsonAsset>| -> Result<Option<Value>, String> {
        if let LoadState::Failed(error) = server.load_state(handle) {
            return Err(format!("{error}"));
        }
        let Some(asset) = world.resource::<Assets<JsonAsset>>().get(handle) else {
            return Ok(None);
        };
        serde_json::from_str(&asset.0)
            .map(Some)
            .map_err(|error| format!("{error}"))
    };
    let handle = match &load.particle_document {
        Some(handle) => handle.clone(),
        None => {
            let Some(index) = json(world, &load.particle_index)
                .map_err(|error| format!("the particle index: {error}"))?
            else {
                return Ok(None);
            };
            let file = index["packages"][load.package.as_str()]["file"]
                .as_str()
                .filter(|file| {
                    !file.contains('/') && !file.contains('\\') && file.ends_with(".json")
                })
                .ok_or_else(|| format!("the particle index has no document for {}", load.package))?
                .to_owned();
            let handle = server.load::<JsonAsset>(format!("moly://fixture-particles-v2/{file}"));
            load.particle_document = Some(handle.clone());
            handle
        }
    };
    let Some(document) =
        json(world, &handle).map_err(|error| format!("the particle document: {error}"))?
    else {
        return Ok(None);
    };
    let document = Arc::new(document);
    load.particles = Some(document.clone());
    Ok(Some(document))
}

/// `EffectTrack.CreateTrackMixer` ahead of the graph: each effect clip's
/// template prefab is instantiated (`CreateEffectObject`) and the systems
/// its instance plays are prepared on the particle host. False while
/// something loads or prepares. A clip whose instance cannot be built or
/// prepared is refused by name and the rest of the cut-scene plays.
fn prepare_effects(
    world: &mut World,
    load: &mut Load,
    definition: &TimelineDefinition,
    effects: &HashMap<String, EffectPrefab>,
    root: Entity,
) -> bool {
    let clips: Vec<&TimelineClip> = definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter(|clip| {
            matches!(
                &clip.payload,
                TimelinePayload::CutScene(CutScenePayload::Effect(template)) if template.prefab.is_some()
            )
        })
        .collect();
    if clips.iter().all(|clip| {
        matches!(
            load.effects.get(&clip.key),
            Some(EffectSlot::Ready(_)) | Some(EffectSlot::Refused)
        )
    }) {
        return true;
    }
    let document = match particle_document(world, load) {
        Ok(Some(document)) => document,
        Ok(None) => {
            wait(load, "the effect clips' particle document is loading");
            return false;
        }
        Err(reason) => {
            for clip in &clips {
                refuse_effect(world, load, clip, &reason);
            }
            return true;
        }
    };
    let mut pending = false;
    let mut prefabs_seen: HashSet<i64> = HashSet::new();
    for clip in clips {
        let TimelinePayload::CutScene(CutScenePayload::Effect(template)) = &clip.payload else {
            continue;
        };
        let Some(prefab) = clip
            .playable
            .as_ref()
            .and_then(|id| effects.get(&format!("{}/{}", id.file, id.path_id)))
        else {
            refuse_effect(
                world,
                load,
                clip,
                "the track table has no prefab record for its template",
            );
            continue;
        };
        // `CreateEffectObject` keys its instances by the prefab: a second
        // clip of the same prefab shares the first one's instance, which is
        // not handled here.
        if !prefabs_seen.insert(prefab.game_object) {
            refuse_effect(world, load, clip, "another effect clip of this cut-scene names the same prefab; a shared instance is not handled");
            continue;
        }
        let spawned = match load.effects.get(&clip.key) {
            Some(EffectSlot::Ready(_)) | Some(EffectSlot::Refused) => continue,
            Some(EffectSlot::Pending(spawned)) => *spawned,
            None => None,
        };
        let instance = match spawned {
            Some(instance) => instance,
            None => match instantiate_effect(world, &document, prefab, root) {
                Ok(instance) => {
                    load.effects
                        .insert(clip.key.clone(), EffectSlot::Pending(Some(instance)));
                    instance
                }
                Err(reason) => {
                    refuse_effect(world, load, clip, &reason);
                    continue;
                }
            },
        };
        let read = match read_effect_instance(world, &document, prefab, instance) {
            Ok(read) => read,
            Err(reason) => {
                refuse_effect(world, load, clip, &reason);
                continue;
            }
        };
        let mut played = Vec::new();
        let mut not_played = read.not_played.clone();
        let mut waiting = false;
        for (node, node_path) in &read.subtrees {
            match particles::prepare_played_object(world, root, *node, &load.package) {
                Ok(binding) => played.push((node_path.clone(), binding)),
                Err(error) if error.retryable => waiting = true,
                Err(error) => not_played.push(format!(
                    "{node_path} and its subtree (refused by the particle host: {})",
                    error.message
                )),
            }
        }
        if waiting {
            pending = true;
            continue;
        }
        if played.is_empty() {
            refuse_effect(
                world,
                load,
                clip,
                &format!(
                    "the particle host plays none of its systems: {}",
                    not_played.join("; ")
                ),
            );
            continue;
        }
        let name = clip.source_envelope["m_DisplayName"]
            .as_str()
            .unwrap_or("?")
            .to_owned();
        info!(
            "[cutscene] {}/{}: EffectTrack.CreateEffectObject({}): the template is deactivated and instantiated with no parent ({} nodes); GetComponentsInChildren<ParticleSystem>() lists {} systems, rootParticle {}; prepared on the particle host: {}",
            load.package,
            load.prefab,
            prefab.name,
            read.nodes,
            read.listed,
            read.root_path,
            played
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !not_played.is_empty() {
            error!(
                "[cutscene] {}/{}: EffectClip {name}: {} listed systems are not played (named gap; the others play): {}",
                load.package,
                load.prefab,
                not_played.len(),
                not_played.join("; ")
            );
        }
        load.effects.insert(
            clip.key.clone(),
            EffectSlot::Ready(Box::new(EffectInstance {
                name,
                prefab: prefab.name.clone(),
                root: instance,
                root_particle: read.root_path,
                played,
                not_played,
                listed: read.listed,
                auto_seeded: read.auto_seeded,
                awake: read.awake,
                included_loop: read.included_loop,
                matched_duration: template.matched_duration,
                random_seed: template.random_seed,
                offset: Vec3::new(
                    template.offset[0] as f32,
                    template.offset[1] as f32,
                    template.offset[2] as f32,
                ),
            })),
        );
    }
    if pending {
        wait(load, "the effect clips' systems are preparing");
    }
    !pending
}

/// An effect clip refused by name: its instance (if built) is removed and
/// the rest of the cut-scene plays.
fn refuse_effect(world: &mut World, load: &mut Load, clip: &TimelineClip, reason: &str) {
    if let Some(EffectSlot::Pending(Some(instance))) = load.effects.get(&clip.key) {
        world.despawn(*instance);
    }
    load.effects.insert(clip.key.clone(), EffectSlot::Refused);
    error!(
        "[cutscene] {}/{}: EffectClip {} [{:.4}, {:.4}): refused: {reason}; it plays nothing",
        load.package,
        load.prefab,
        clip.source_envelope["m_DisplayName"]
            .as_str()
            .unwrap_or("?"),
        clip.start,
        clip.end()
    );
}

/// `Object.Instantiate(prefab, null)` of a template prefab deactivated
/// first: the instance's nodes are the particle document's node records
/// under the prefab root (authored local poses reflected into the product
/// frame, each with its source GameObject), the root inactive at the prefab
/// root's own pose. The cut-scene root stands for the world's frame.
fn instantiate_effect(
    world: &mut World,
    document: &Value,
    prefab: &EffectPrefab,
    parent: Entity,
) -> Result<Entity, String> {
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
    let mut root_node = None;
    for node in nodes {
        let id = node["gameObjectId"]
            .as_i64()
            .ok_or("a document node has no gameObjectId")?;
        if id == prefab.game_object {
            root_node = Some(node);
        }
        let parent_id = node["parentGameObjectId"]
            .as_i64()
            .ok_or("a document node has no parentGameObjectId")?;
        children.entry(parent_id).or_default().push(node);
    }
    let root_node = root_node.ok_or_else(|| {
        format!(
            "its prefab root {} (GameObject {}) is not in the package's particle document",
            prefab.name, prefab.game_object
        )
    })?;
    let transform = |node: &Value| -> Result<Transform, String> {
        let floats = |key: &str, len: usize| -> Result<Vec<f32>, String> {
            let list = node[key]
                .as_array()
                .filter(|list| list.len() == len)
                .ok_or_else(|| format!("node {} has no {key}", node["node"]))?;
            list.iter()
                .map(|v| v.as_f64().map(|v| v as f32))
                .collect::<Option<Vec<f32>>>()
                .ok_or_else(|| format!("node {} {key} is not numeric", node["node"]))
        };
        let p = floats("position", 3)?;
        let r = floats("rotation", 4)?;
        let q = floats("scale", 3)?;
        Ok(Transform {
            translation: moly_assets::coordinates::source_position(Vec3::new(p[0], p[1], p[2])),
            rotation: moly_assets::coordinates::source_rotation(Quat::from_xyzw(
                r[0], r[1], r[2], r[3],
            )),
            scale: Vec3::new(q[0], q[1], q[2]),
        })
    };
    let identity = |node: &Value| moly_assets::source_navigation::SourceObjectIdentity {
        file: prefab.file.clone(),
        game_object: node["gameObjectId"].as_i64().unwrap_or(0),
        // The document carries no Transform or component ids.
        transform: 0,
        components: Vec::new(),
        child_order: Vec::new(),
    };
    let leaf = |node: &Value| -> String {
        let path = node["node"].as_str().unwrap_or("?");
        path.rsplit('/').next().unwrap_or(path).to_owned()
    };
    let root = world
        .spawn((
            Name::new(leaf(root_node)),
            transform(root_node)?,
            Visibility::Hidden,
            identity(root_node),
            ChildOf(parent),
        ))
        .id();
    // (GameObject, its entity, active in the hierarchy below the root)
    let mut stack = vec![(prefab.game_object, root, true)];
    let mut count = 1;
    while let Some((id, entity, active)) = stack.pop() {
        for node in children.get(&id).into_iter().flatten() {
            let child_id = node["gameObjectId"].as_i64().unwrap_or(0);
            let child_active = active && node["active"].as_bool() == Some(true);
            let local = match transform(node) {
                Ok(local) => local,
                Err(reason) => {
                    world.despawn(root);
                    return Err(reason);
                }
            };
            let mut child = world.spawn((
                Name::new(leaf(node)),
                local,
                if child_active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                identity(node),
                ChildOf(entity),
            ));
            if !child_active {
                child.insert(moly_assets::scene_state::SourceInactive);
            }
            let child = child.id();
            stack.push((child_id, child, child_active));
            count += 1;
            if count > nodes.len() {
                world.despawn(root);
                return Err("the particle document's node parents form a cycle".into());
            }
        }
    }
    Ok(root)
}

/// What `SetEffectInstance` reads from an instance.
struct InstanceRead {
    nodes: usize,
    root_path: String,
    /// The maximal subtrees below the root particle the host can play as
    /// one object, by node, with their paths.
    subtrees: Vec<(Entity, String)>,
    /// The listed systems outside those subtrees, each with the reason.
    not_played: Vec<String>,
    listed: usize,
    auto_seeded: Vec<String>,
    awake: usize,
    included_loop: bool,
}

/// `GetComponentsInChildren<ParticleSystem>()` (includeInactive false) on
/// the inactive instance: the root's own components are always searched and
/// a child only when it is active itself, so the list is the systems whose
/// nodes below the root are all active, in the hierarchy's order;
/// `rootParticle` is its first. The activation then plays each listed
/// system whose playOnAwake is set; the particle host plays and stops the
/// root particle with its subtree as one object, so the two sets must agree.
fn read_effect_instance(
    world: &World,
    document: &Value,
    prefab: &EffectPrefab,
    instance: Entity,
) -> Result<InstanceRead, String> {
    let emitters = document["emitters"]
        .as_array()
        .ok_or("the particle document has no emitter list")?;
    let by_id: HashMap<i64, &Value> = emitters
        .iter()
        .filter_map(|emitter| Some((emitter["gameObjectId"].as_i64()?, emitter)))
        .collect();
    // The instance's nodes by GameObject, with their activity.
    let mut nodes: HashMap<i64, (Entity, bool)> = HashMap::new();
    let mut stack = vec![instance];
    while let Some(entity) = stack.pop() {
        if let Some(identity) =
            world.get::<moly_assets::source_navigation::SourceObjectIdentity>(entity)
        {
            let inactive = world
                .get::<moly_assets::scene_state::SourceInactive>(entity)
                .is_some();
            nodes.insert(identity.game_object, (entity, !inactive));
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    let mut listed = Vec::new();
    for id in &prefab.particle_systems {
        let (entity, active) = nodes.get(id).copied().ok_or_else(|| {
            format!("its system on GameObject {id} is not in the package's particle document")
        })?;
        let emitter = by_id
            .get(id)
            .ok_or_else(|| format!("its system on GameObject {id} has no emitter record"))?;
        if active {
            listed.push((entity, *emitter));
        }
    }
    let Some(&(root_particle, root_emitter)) = listed.first() else {
        return Err("its instance lists no system, so it has no rootParticle; the systems its activation plays are not played by this host".into());
    };
    let path = |emitter: &Value| emitter["node"].as_str().unwrap_or("?").to_owned();
    let under_root = |entity: Entity| {
        let mut current = Some(entity);
        while let Some(node) = current {
            if node == root_particle {
                return true;
            }
            current = world.get::<ChildOf>(node).map(ChildOf::parent);
        }
        false
    };
    let flag = |emitter: &Value, key: &str| -> Result<bool, String> {
        emitter["system"][key]
            .as_bool()
            .ok_or_else(|| format!("{}: {key} is not in its record", path(emitter)))
    };
    let mut awake = 0;
    let mut auto_seeded = Vec::new();
    let mut included_loop = false;
    for &(entity, emitter) in &listed {
        match (flag(emitter, "playOnAwake")?, under_root(entity)) {
            (true, true) => awake += 1,
            (true, false) => {
                return Err(format!(
                    "{} plays on activation outside the root particle's subtree, which the particle host plays as one object",
                    path(emitter)
                ))
            }
            (false, true) => {
                return Err(format!(
                    "{} under the root particle does not play on awake, while the particle host plays the root particle's subtree as one object",
                    path(emitter)
                ))
            }
            (false, false) => {}
        }
        let seed = emitter["system"]["randomSeed"]
            .as_u64()
            .ok_or_else(|| format!("{}: randomSeed is not in its record", path(emitter)))?;
        if flag(emitter, "autoRandomSeed")? || seed != 0 {
            auto_seeded.push(path(emitter));
        }
        included_loop |= flag(emitter, "looping")?;
    }
    // The particle host plays an object's whole subtree and refuses a
    // sub-emitter parent in it (its sub-emitters would be played by their
    // parent's events, which it does not install); a sub-emitter played as
    // its own object would emit without them. So the root particle is played
    // as its maximal subtrees free of both; every other listed system is
    // left unplayed by name, as the host leaves a refused system of a
    // director owner undrawn while the others play.
    let by_path: HashMap<&str, &Value> = emitters
        .iter()
        .filter_map(|emitter| Some((emitter["node"].as_str()?, emitter)))
        .collect();
    let mut targets: HashMap<String, String> = HashMap::new();
    let mut parents: HashSet<String> = HashSet::new();
    for emitter in emitters {
        let system = &emitter["system"];
        let rows = system["subEmitters"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let sub_module = system["sourceModules"]["enabled"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name.as_str() == Some("SubModule")));
        if sub_module || !rows.is_empty() {
            parents.insert(path(emitter));
        }
        for row in rows {
            if let Some(target) = row["emitter"].as_str() {
                targets.insert(target.to_owned(), path(emitter));
            }
        }
    }
    let node_path = |entity: Entity| -> Option<String> {
        let id = world
            .get::<moly_assets::source_navigation::SourceObjectIdentity>(entity)?
            .game_object;
        Some(path(by_id.get(&id)?))
    };
    let children_of = |entity: Entity| -> Vec<Entity> {
        world
            .get::<Children>(entity)
            .map(|children| children.iter().collect())
            .unwrap_or_default()
    };
    // Whether a node's subtree holds no sub-emitter parent or target.
    fn clean(
        entity: Entity,
        node_path: &dyn Fn(Entity) -> Option<String>,
        children_of: &dyn Fn(Entity) -> Vec<Entity>,
        parents: &HashSet<String>,
        targets: &HashMap<String, String>,
    ) -> bool {
        if let Some(path) = node_path(entity) {
            if parents.contains(&path) || targets.contains_key(&path) {
                return false;
            }
        }
        children_of(entity)
            .into_iter()
            .all(|child| clean(child, node_path, children_of, parents, targets))
    }
    let mut subtrees = Vec::new();
    let mut not_played = Vec::new();
    let mut stack = vec![root_particle];
    while let Some(entity) = stack.pop() {
        let own = node_path(entity);
        if own.is_some() && clean(entity, &node_path, &children_of, &parents, &targets) {
            subtrees.push((entity, own.unwrap_or_default()));
            continue;
        }
        if let Some(own) = own {
            let emitter = by_path.get(own.as_str()).copied();
            let emits = emitter.is_some_and(|emitter| {
                emitter["system"]["sourceModules"]["enabled"]
                    .as_array()
                    .is_some_and(|names| {
                        names
                            .iter()
                            .any(|name| name.as_str() == Some("EmissionModule"))
                    })
            });
            let listed_here = listed
                .iter()
                .any(|&(listed_entity, _)| listed_entity == entity);
            if listed_here {
                not_played.push(if let Some(parent) = targets.get(&own) {
                    format!("{own} (a sub-emitter of {parent})")
                } else if parents.contains(&own) {
                    format!("{own} (a sub-emitter parent)")
                } else if !emits {
                    format!("{own} (its Emission module is off: it emits nothing)")
                } else {
                    format!("{own} (above a sub-emitter parent)")
                });
            }
        }
        let mut children = children_of(entity);
        children.reverse();
        stack.extend(children);
    }
    Ok(InstanceRead {
        nodes: nodes.len(),
        root_path: path(root_emitter),
        subtrees,
        not_played,
        listed: listed.len(),
        auto_seeded,
        awake,
        included_loop,
    })
}

/// `CutScenePresenter.FadeInAsync`: the cut-scene screen is pushed if it is
/// not active, then its `FadeIn` (default colour, 0.5 s).
fn presenter_fade_in(world: &mut World, plan: &Plan) {
    world.insert_resource(game_state::UiHold("CutScene"));
    info!(
        "[cutscene] {}: CutScenePresenter.FadeInAsync: PushUIScreen(MysekaiCutScene) (the field UI is not interactable); ScreenLayerMysekaiMysekaiCutScene.FadeIn({LAYER_FADE} s)",
        plan.prefab
    );
    crate::screen_fade::cutscene_fade_in(
        world,
        [0.0; 4],
        LAYER_FADE,
        "CutScenePresenter.FadeInAsync",
    );
}

/// `SetupInternal`, then `CutSceneView.PlayAsync` starts the director.
fn setup_internal(world: &mut World, plan: &mut Plan) -> Play {
    // Setup: ChangeState(CutScene) -> CutSceneGameState.OnEnter.
    game_state::enter(world, GameStateType::CutScene, "CutScenePresenter.Setup");
    if let Some(mut gate) =
        world.get_resource_mut::<crate::alone_action_runtime::AloneExecutionGate>()
    {
        gate.cutscene_active = true;
    }
    info!("[cutscene] AloneExecutionGate.cutscene_active = true (GameState CutScene): NPC alone actions and greetings yield");
    info!("[cutscene] CutSceneGameState.OnEnter: PushUIScreen(MysekaiCutScene); FieldCamera.ChangeState(CutScene)");
    cutscene_camera::enter(world);
    info!(
        "[cutscene] Setup: EnableTapScreen; no cut-scene characters to take over; HideNearFixtures({}) (the fixtures within it are already hidden); white list {:?}, black list {:?}",
        plan.hide_distance, plan.white, plan.black
    );
    info!("[cutscene] SetupInternal: SetupCamera; HideEffects (no hidden effect list); UI shown; SetupSkipEvent; BindComopnents: the brain on the field camera, the fade panel on the cut-scene screen, the obstacle tracks on the home site controller; DisableIK; LoadBgms; LoadVoice; PlayPreprocess");
    let request = plan.request.take().expect("prepared request");
    let token = world
        .resource_mut::<FixtureActivityTimelines>()
        .request_start(request);
    info!(
        "[cutscene] CutSceneView.PlayAsync: SetActive(true), extrapolation Hold; the director {} plays; WaitUntil(time >= {:.4})",
        plan.definition.director.path_id, plan.definition.duration
    );
    log_fade_law(plan);
    Play {
        token: Some(token),
        camera_logged: f64::NEG_INFINITY,
        fade_logged: f64::NEG_INFINITY,
        ..default()
    }
}

/// The fade panel law evaluated on the clips at the sampled times the
/// evidence names (the same law the frames run).
fn log_fade_law(plan: &Plan) {
    let clips: Vec<(f64, f64, bool, [f32; 4])> = plan
        .definition
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter_map(|clip| match &clip.payload {
            TimelinePayload::CutScene(CutScenePayload::Fade { fade_in, color }) => {
                Some((clip.start, clip.duration, *fade_in, *color))
            }
            _ => None,
        })
        .collect();
    let samples: Vec<String> = [0.0, 0.25, 0.5, 6.2, 6.6]
        .iter()
        .map(|t| match fade_panel_at(&clips, *t) {
            Some((alpha, color)) => format!(
                "t={t}: alpha {alpha:.4} colour ({:.0},{:.0},{:.0})",
                color[0], color[1], color[2]
            ),
            None => format!("t={t}: no write"),
        })
        .collect();
    info!(
        "[cutscene] fade panel law on this director's clips: {}",
        samples.join("; ")
    );
}

/// `SetAlpha(value, color)`: a colour whose squared length is below 1e-10 is
/// replaced by opaque black, then the alpha is written.
fn set_alpha_colour(color: [f32; 4], alpha: f32) -> [f32; 4] {
    let squared: f32 = color.iter().map(|c| c * c).sum();
    let base = if squared < 9.999_999_4e-11 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        color
    };
    [base[0], base[1], base[2], alpha]
}

/// The fade panel's write at time `t`: the active clip's `UpdateAlpha`, else
/// the mixer's hold of the last clip that ended (`ClipSetup`).
fn fade_panel_at(clips: &[(f64, f64, bool, [f32; 4])], t: f64) -> Option<(f32, [f32; 4])> {
    if let Some((start, duration, fade_in, color)) = clips
        .iter()
        .find(|(start, duration, _, _)| t >= *start && t < start + duration)
    {
        let p = ((t - start) / duration) as f32;
        let alpha = if *fade_in {
            if p > FADE_HOLD {
                1.0
            } else {
                p
            }
        } else if p > FADE_HOLD {
            0.0
        } else {
            1.0 - p
        };
        return Some((alpha, set_alpha_colour(*color, alpha)));
    }
    let mut ended: Vec<_> = clips
        .iter()
        .filter(|(start, duration, _, _)| start + duration < t)
        .collect();
    ended.sort_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)));
    ended.last().map(|(_, _, fade_in, color)| {
        let alpha = if *fade_in { 1.0 } else { 0.0 };
        (alpha, set_alpha_colour(*color, alpha))
    })
}

/// `SetSiteExtension` data of a site expansion clip at clip-local time.
fn site_extension(effect: &ExpansionEffect, local: f64, duration: f64) -> String {
    let t = ((local / duration) as f32).clamp(0.0, 1.0);
    let radius = effect.start_radius + t * (effect.end_radius - effect.start_radius);
    let inner = radius - effect.gradient_range.max(0.0);
    format!(
        "IsActive true, Center ({:.3},{:.3},{:.3}), DissolveSmoothness {DEFAULT_SMOOTHNESS}, EdgeColor ({:.3},{:.3},{:.3},{:.3}), FadeColor ({:.3},{:.3},{:.3},{:.3}), Radius {radius:.4}, InnerRadius {inner:.4}, FadeMin {}, FadeMax {}, LimitLineWidth {DEFAULT_LIMIT_LINE_WIDTH}",
        effect.center[0],
        effect.center[1],
        effect.center[2],
        effect.edge_color[0],
        effect.edge_color[1],
        effect.edge_color[2],
        effect.edge_color[3],
        effect.fade_color[0],
        effect.fade_color[1],
        effect.fade_color[2],
        effect.fade_color[3],
        effect.min_radius,
        effect.max_radius
    )
}

/// The rings on and off at an obstacle level.
fn rings_at(world: &mut World, level: u32) -> (Vec<u32>, Vec<u32>) {
    let mut query = world.query::<&HomeObstacleRing>();
    let mut on = Vec::new();
    let mut off = Vec::new();
    for ring in query.iter(world) {
        if ring.level >= level {
            on.push(ring.level);
        } else {
            off.push(ring.level);
        }
    }
    on.sort_unstable();
    off.sort_unstable();
    (on, off)
}

/// One frame of the director: returns true once its time reached its
/// duration (or the session ended).
fn play_frame(world: &mut World, plan: &Plan, play: &mut Play) -> bool {
    let Some(token) = play.token else {
        return true;
    };
    let status = world
        .resource::<FixtureActivityTimelines>()
        .status(token)
        .cloned();
    match status {
        Some(TimelineStatus::Preparing) => return false,
        Some(TimelineStatus::Failed(error)) => {
            error!("[cutscene] {}: the director failed: {error}", plan.prefab);
            return true;
        }
        Some(TimelineStatus::Cancelled) | None => {
            error!("[cutscene] {}: the director was cancelled", plan.prefab);
            return true;
        }
        _ => {}
    }
    let Some(t) = world
        .resource::<FixtureActivityTimelines>()
        .sampled_time(token)
    else {
        return false;
    };
    if !play.graph_built {
        play.graph_built = true;
        set_effect_instances(world, plan, t);
    } else {
        sample_effects(world, plan, play, t);
    }
    let definition = plan.definition.clone();
    let mut shot = None;
    let mut active = HashSet::new();
    let mut fade_clips = Vec::new();
    for track in &definition.tracks {
        for clip in &track.clips {
            let now = clip.contains(t);
            let entered = now && !play.active.contains(&clip.key);
            let exited = !now && play.active.contains(&clip.key);
            if now {
                active.insert(clip.key.clone());
            }
            let bounds = format!("[{:.4}, {:.4})", clip.start, clip.end());
            let local = t - clip.start;
            match &clip.payload {
                TimelinePayload::CutScene(CutScenePayload::CinemachineShot { exposed_name }) => {
                    if now {
                        match plan.cameras.get(exposed_name) {
                            Some(camera) => shot = Some(camera.clone()),
                            None if entered => error!(
                                "[cutscene] CinemachineShot {bounds}: exposed name {exposed_name} names no virtual camera of the prefab; the camera stays"
                            ),
                            None => {}
                        }
                    }
                }
                TimelinePayload::CutScene(CutScenePayload::Fade { fade_in, color }) => {
                    fade_clips.push((clip.start, clip.duration, *fade_in, *color));
                }
                TimelinePayload::CutScene(CutScenePayload::UpdateObstacle { level }) => {
                    if entered {
                        let level_u = (*level).max(0) as u32;
                        world.insert_resource(HomeObstacleLevel(level_u));
                        let (on, off) = rings_at(world, level_u);
                        info!(
                            "[cutscene] t={t:.4} UpdateObstacleBehaviour.OnBehaviourPlay {bounds}: HomeSiteController.UpdateObstacle({level}) -> UpdateView: rings on {on:?}, off {off:?}, dither off"
                        );
                    }
                }
                TimelinePayload::CutScene(CutScenePayload::HideSiteObstacle { level }) => {
                    let value = if entered {
                        Some(1.0)
                    } else if now {
                        Some((1.0 - local / clip.duration) as f32)
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        play.dither.insert(*level, value);
                        let (_, below) = rings_at(world, (*level).max(0) as u32);
                        info!(
                            "[cutscene] t={t:.4} HideSiteObstacleBehaviour.{} {bounds}: SetDither({level}, {value:.4}) on rings {below:?} (dither {}; not drawn: named render gap)",
                            if entered { "OnBehaviourPlay" } else { "ProcessFrame" },
                            if value < 1.0 { "active" } else { "inactive" }
                        );
                    }
                }
                TimelinePayload::CutScene(CutScenePayload::SiteExpansion(effect)) => {
                    if now {
                        play.dissolve = true;
                        crate::site_extension::set_site_extension(
                            world,
                            crate::site_extension::SiteExtensionData::expansion_frame(
                                effect.center,
                                effect.start_radius,
                                effect.end_radius,
                                effect.min_radius,
                                effect.max_radius,
                                effect.gradient_range,
                                effect.edge_color,
                                effect.fade_color,
                                local,
                                clip.duration,
                            ),
                        );
                        info!(
                            "[cutscene] t={t:.4} ShowExpansionEffectBehaviour.ProcessFrame {bounds}: SetSiteExtension({})",
                            site_extension(effect, local, clip.duration)
                        );
                    }
                }
                TimelinePayload::CutScene(CutScenePayload::Effect(template)) => {
                    let name = clip.source_envelope["m_DisplayName"]
                        .as_str()
                        .unwrap_or("?");
                    let instance = plan.instances.get(&clip.key);
                    if entered {
                        let prefab = clip.playable.as_ref().and_then(|id| {
                            plan.effects.get(&format!("{}/{}", id.file, id.path_id))
                        });
                        info!(
                            "[cutscene] t={t:.4} EffectClip {name} {bounds} OnBehaviourPlay: template prefab {:?} ({} particle systems, root scripts {:?}; parentMode {}, characterID {}, fixed starting point {}, offset {:?}, matched duration {}, random seed {})",
                            prefab.map(|p| p.name.as_str()).or(template.prefab.as_deref()),
                            prefab.map_or(0, |p| p.particle_systems.len()),
                            prefab.map(|p| p.root_scripts.clone()).unwrap_or_default(),
                            template.parent_mode,
                            template.character_id,
                            template.fixed_starting_point,
                            template.offset,
                            template.matched_duration,
                            template.random_seed
                        );
                        match instance {
                            Some(instance) => effect_play(world, instance, t, local),
                            None => info!("[cutscene] t={t:.4} EffectClip {name}: no instance (its template is null or it was refused at load): nothing plays"),
                        }
                    } else if exited {
                        match instance {
                            Some(instance) => effect_pause(world, instance, t),
                            None => info!("[cutscene] t={t:.4} EffectClip {name} {bounds} OnBehaviourPause: no instance, nothing to stop"),
                        }
                    }
                }
                TimelinePayload::Se { package, cue } => {
                    if entered {
                        info!(
                            "[cutscene] t={t:.4} SEClip {bounds}: cue {cue} of package {:?} starts: {}",
                            package,
                            if plan.silent.contains(&clip.key) {
                                "silent (no audio route for it in this root)"
                            } else {
                                "played by the runner"
                            }
                        );
                    }
                }
                TimelinePayload::Control(_) => {
                    let name = clip.source_envelope["m_DisplayName"]
                        .as_str()
                        .unwrap_or("?");
                    if entered {
                        info!("[cutscene] t={t:.4} Control clip {name} {bounds} play: refused by name (see the runner's coverage)");
                    } else if exited {
                        info!("[cutscene] t={t:.4} Control clip {name} {bounds} stop");
                    }
                }
                _ => {}
            }
        }
    }
    play.active = active;
    // The fade panel.
    let wrote = fade_panel_at(&fade_clips, t);
    let mut ended: Vec<_> = fade_clips
        .iter()
        .enumerate()
        .filter(|(_, (start, duration, _, _))| start + duration < t)
        .collect();
    ended.sort_by(|a, b| (a.1 .0 + a.1 .1).total_cmp(&(b.1 .0 + b.1 .1)));
    let index = ended.last().map(|(index, _)| *index);
    let in_clip = fade_clips
        .iter()
        .any(|(start, duration, _, _)| t >= *start && t < start + duration);
    if let Some((alpha, colour)) = wrote {
        if in_clip || index != play.fade_index {
            crate::screen_fade::set_cutscene_image(world, colour);
            if t - play.fade_logged >= 0.0 {
                info!(
                    "[cutscene-fade] t={t:.4} FadePanel{} SetAlpha({alpha:.4}, ({:.0},{:.0},{:.0}))",
                    if in_clip { "Behaviour.UpdateAlpha" } else { "MixerBehaviour.ClipSetup" },
                    colour[0],
                    colour[1],
                    colour[2]
                );
                play.fade_logged = t;
            }
        }
    }
    play.fade_index = index;
    // The brain.
    if let Some(camera) = shot {
        let shot = product_shot(&camera, plan.start);
        if t - play.camera_logged >= 0.5 {
            info!(
                "[cutscene-camera] t={t:.4} live camera {}: source position ({:.3},{:.3},{:.3}) rotation ({:.4},{:.4},{:.4},{:.4}) -> eye ({:.3},{:.3},{:.3}); fov {} near {} far {}",
                camera.name,
                camera.position.x,
                camera.position.y,
                camera.position.z,
                camera.rotation.x,
                camera.rotation.y,
                camera.rotation.z,
                camera.rotation.w,
                shot.eye.x,
                shot.eye.y,
                shot.eye.z,
                shot.fov,
                shot.near,
                shot.far
            );
            if let Some((eye, forward, fov)) = cutscene_camera::pose(world) {
                info!(
                    "[cutscene-camera] t={t:.4} camera read back: eye ({:.3},{:.3},{:.3}) forward ({:.4},{:.4},{:.4}) fov {fov:.2}",
                    eye.x, eye.y, eye.z, forward.x, forward.y, forward.z
                );
            }
            play.camera_logged = t;
        }
        cutscene_camera::set_shot(world, Some(shot));
    }
    matches!(status, Some(TimelineStatus::Completed)) || t >= definition.duration
}

/// `EffectClip.CreatePlayable` at graph build: `SetEffectInstance` on each
/// clip's instance. Unless `isEnabledRandomSeed`, every listed system gets
/// `randomSeed = 0` (one already playing is first stopped and cleared and
/// played again; a fresh instance has none playing); the first is the root
/// particle and is stopped; a looping system sets `isIncludedLoop`; then
/// `SetActive(true)` activates the instance, and its playOnAwake systems
/// play (the activation's Play, through the particle host).
fn set_effect_instances(world: &mut World, plan: &Plan, t: f64) {
    let mut instances: Vec<&EffectInstance> = plan.instances.values().collect();
    instances.sort_by(|a, b| a.name.cmp(&b.name));
    for instance in instances {
        let seed = if instance.random_seed {
            "isEnabledRandomSeed: the systems keep their seeds".to_owned()
        } else if instance.auto_seeded.is_empty() {
            "randomSeed = 0 on each: every listed system's serialized seed is already the manual seed 0".to_owned()
        } else {
            format!(
                "randomSeed = 0 on each: not written (the particle host has no seed write; named gap): {} listed systems keep their own seed ({})",
                instance.auto_seeded.len(),
                instance.auto_seeded.join(", ")
            )
        };
        if let Some(mut visibility) = world.get_mut::<Visibility>(instance.root) {
            *visibility = Visibility::Inherited;
        }
        let played = play_subtrees(world, instance);
        info!(
            "[cutscene] t={t:.4} EffectClip {}: EffectClip.CreatePlayable -> SetEffectInstance({}): {} systems listed; {seed}; none playing; rootParticle {}: Stop(); isIncludedLoop {}; SetActive(true): {} playOnAwake systems play; the particle host plays {played} systems ({} listed systems not played)",
            instance.name,
            instance.prefab,
            instance.listed,
            instance.root_particle,
            instance.included_loop,
            instance.awake,
            instance.not_played.len()
        );
    }
}

/// The live particles of the effect instances' played subtrees, four times
/// a real second while they hold any (a trace of the particle host's
/// state, read from its systems).
fn sample_effects(world: &mut World, plan: &Plan, play: &mut Play, t: f64) {
    if plan.instances.is_empty() || now_real(world) - play.effect_sampled < 0.25 {
        return;
    }
    play.effect_sampled = now_real(world);
    for instance in plan.instances.values() {
        for (path, binding) in &instance.played {
            let (systems, live, born) = timeline::played_particles(world, binding.root);
            if live > 0 {
                info!(
                    "[cutscene] t={t:.4} EffectClip {}: {path}: systems {systems} live {live} born {born}",
                    instance.name
                );
            }
        }
    }
}

/// `Play()` on the root particle: each subtree the particle host plays.
/// Returns how many systems played; a refused Play is named.
fn play_subtrees(world: &mut World, instance: &EffectInstance) -> usize {
    let mut played = 0;
    for (path, binding) in &instance.played {
        match particles::play_object(world, binding) {
            Ok(count) => played += count,
            Err(error) => error!(
                "[cutscene] EffectClip {}: {path}: Play refused by the particle host: {error}",
                instance.name
            ),
        }
    }
    played
}

/// `EffectBehaviour.OnBehaviourPlay` at run time: no parent is set (the
/// parent is looked up only outside play mode); a matched-duration clip
/// without a looping system rewrites the systems' durations; the instance
/// is placed at the clip's offset with the identity rotation; past 0.1 s of
/// clip time the root particle is first simulated to it; then the root
/// particle plays with its children.
fn effect_play(world: &mut World, instance: &EffectInstance, t: f64, local: f64) {
    if instance.matched_duration && !instance.included_loop {
        error!(
            "[cutscene] t={t:.4} EffectClip {}: isMatchedDuration: the systems' main.duration writes are not in the particle host's API (named gap)",
            instance.name
        );
    }
    let position = moly_assets::coordinates::source_position(instance.offset);
    if let Some(mut transform) = world.get_mut::<Transform>(instance.root) {
        transform.translation = position;
        transform.rotation = Quat::IDENTITY;
    }
    if local > 0.1 {
        error!(
            "[cutscene] t={t:.4} EffectClip {}: clip time {local:.4} > 0.1: rootParticle.Simulate({local:.4}, true, true) is not in the particle host's played API (named gap); the systems keep their clocks",
            instance.name
        );
    }
    let played = play_subtrees(world, instance);
    info!(
        "[cutscene] t={t:.4} EffectClip {}: localPosition = offset {:?} (product {:?}), localRotation = identity; clip time {local:.4}; rootParticle {}.Play(): {played} systems played (the particle host keeps a playing object as it is)",
        instance.name,
        instance.offset,
        position,
        instance.root_particle
    );
}

/// `EffectBehaviour.OnBehaviourPause` on the clip's end (the playable is
/// paused): a clip with a looping system and a matched duration stops and
/// clears the root particle, any other stops its emission (`Stop()`).
fn effect_pause(world: &mut World, instance: &EffectInstance, t: f64) {
    if instance.included_loop && instance.matched_duration {
        error!(
            "[cutscene] t={t:.4} EffectClip {}: OnBehaviourPause: Stop(true, StopEmittingAndClear) is not in the particle host's API (named gap); not stopped",
            instance.name
        );
        return;
    }
    let stopped: usize = instance
        .played
        .iter()
        .map(|(_, binding)| particles::stop_object(world, binding))
        .sum();
    info!(
        "[cutscene] t={t:.4} EffectClip {}: OnBehaviourPause: rootParticle {}.Stop() (isIncludedLoop {}, isMatchedDuration {}): {stopped} systems stop emitting",
        instance.name, instance.root_particle, instance.included_loop, instance.matched_duration
    );
}

/// A virtual camera's state in the product frame: the start transform is
/// the loaded site's own origin, the x axis reflects.
fn product_shot(camera: &VirtualCamera, start: Vec3) -> Shot {
    let _ = start;
    let eye = moly_assets::coordinates::source_position(camera.position);
    let reflect = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
    let forward = reflect(camera.rotation * Vec3::Z);
    let up = reflect(camera.rotation * Vec3::Y);
    let rotation = Transform::IDENTITY.looking_to(forward, up).rotation;
    Shot {
        name: camera.name.clone(),
        eye,
        rotation,
        fov: camera.fov,
        near: camera.near,
        far: camera.far,
    }
}

/// `CutScenePresenter.EndAsync`, up to its fade out.
fn end_async(world: &mut World, plan: &Plan, play: &Play) {
    let time = play.token.and_then(|token| {
        world
            .resource::<FixtureActivityTimelines>()
            .sampled_time(token)
    });
    info!(
        "[cutscene] {}: director time {:?} >= duration {:.4}: CutSceneView.PlayAsync returns; EndAsync: OnCutSceneEndAsync; ResetCameraState: GameStateManager.ChangeState(Normal)",
        plan.prefab, time, plan.definition.duration
    );
    game_state::leave(
        world,
        "CutScenePresenter.ResetCameraState: ChangeState(1 Normal)",
    );
    if let Some(mut gate) =
        world.get_resource_mut::<crate::alone_action_runtime::AloneExecutionGate>()
    {
        gate.cutscene_active = false;
    }
    info!("[cutscene] AloneExecutionGate.cutscene_active = false (GameState Normal): NPC alone actions and greetings resume");
    cutscene_camera::exit(world);
    info!("[cutscene] RestoreStates (no cut-scene characters); ChangeParentCharacters; CutSceneView.Dispose: the director's graph stops");
    for (level, value) in &play.dither {
        let (_, below) = rings_at(world, (*level).max(0) as u32);
        info!(
            "[cutscene] HideSiteObstacleBehaviour.OnGraphStop: SetDither({level}, 0) on rings {below:?} (last value {value:.4}; not drawn)"
        );
    }
    if play.dissolve {
        crate::site_extension::reset_global_dissolve(world);
        info!("[cutscene] ShowExpansionEffectBehaviour.OnGraphStop: ResetGlobalDissolve");
    }
    if let Some(token) = play.token {
        timeline::cancel_and_release(world, token);
    }
    for instance in plan.instances.values() {
        info!(
            "[cutscene] EffectBehaviour.OnPlayableDestroy -> EffectClip.OnDestroyPlayable: Destroy({}) (EffectClip {})",
            instance.prefab, instance.name
        );
    }
    world.despawn(plan.root);
    info!("[cutscene] BGM restored (no cut-scene BGM); NPC Show; player Show");
    show_all_fixtures(world);
    info!("[cutscene] RestoreEffects; CutScenePresenter.FadeOutAsync: ScreenLayerMysekaiMysekaiCutScene.FadeOut({LAYER_FADE} s)");
    crate::screen_fade::cutscene_fade_out(world, LAYER_FADE, "CutScenePresenter.FadeOutAsync");
}

pub(crate) fn install(app: &mut App) {
    app.add_message::<HomeScreenStartAnimation>().add_systems(
        Update,
        (
            home_started,
            advance.after(crate::fixture_activity_timeline::advance),
            crate::site::apply_obstacle_level,
        ),
    );
}
