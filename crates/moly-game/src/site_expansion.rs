//! Site expansion: the rank-release trigger, the client's local lists, and
//! the room's expansion performance (rooms have no cutscene).
//!
//! Trigger:
//! - A rank change of the server's reply calls
//!   `MysekaiTopicsManager.AddTopicIfNeeded(prev, current)`, which adds the
//!   site-expansion topic when a site-level release lies in
//!   `(prev, current]` and `current >= 2` ([`law`]).
//! - `SiteActionExecutor.IsNeedSiteExpansionAction(siteId)`: the topic, and
//!   the site's latest site-level release at or below the rank whose level
//!   id is not yet in the client's `UnlockedMasterSiteLevelIds`.
//! - The displayed level while the topic is held: home and the first floor
//!   show the local list's level (`GetMysekaiSiteUnlockedLevel`, 1 when the
//!   list holds none of theirs); the other floors show the rank's level.
//!
//! Server input: the rank before and after the change
//! (`UserMysekaiGamedata.mysekaiRank` of the previous and the new reply).
//! The instrument `MOLY_RANK_CHANGE=<prev>,<current>` delivers that pair as
//! the reply would, before the first site loads; without it no rank change
//! arrives and nothing here runs. Client state: [`MysekaiLocalSettings`]
//! (the topic list and the unlocked site-level list), state of the web save
//! layer; it starts empty, as on a fresh install.
//!
//! Room performance (`MyRoomSiteController.OnFinishEnterAsync` ->
//! `RoomSitePerformExecutor.PlayRoomSiteExpansionPerformAsync`):
//! `PlaySiteExpansionAnimationIfNeed` saves the unlocked level, removes the
//! topic when no site needs it, fades the screen out over 0.25 s, runs
//! `RankUpMyRoomAction` (game state 11 with the camera at None, the camera
//! pose of `UpdateRoomView`, the room at its new level, the layout reloaded,
//! a 0.75 s fade in), opens the level-release dialog (a UI dialog: logged, not
//! drawn), sets up the room's navigation and refreshes the layout editor.
//! The other-floor variant runs only for a floor the site manager has
//! loaded; one site is loaded at a time here, so `GetSite` of another floor
//! is null and the variant (with its 1.5 s fade in) does not run.
//!
//! Named differences: the room is rebuilt through the site loader under the
//! black screen (the whole site, not only its room model): the player keeps
//! its pose (restored after the loader's reseed), the camera model its
//! yaw, pitch, distance and offset (restored after the loader's framing),
//! and the NPCs are placed again by their own reseed.

pub(crate) mod law;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

use crate::camera::{CameraStateType, FieldCameraModel, FieldCameraState};
use crate::game_state::{self, GameStateType};
use crate::player::PlayerControlled;
use crate::site::{SiteActive, SiteSelection};
use law::{LocalLists, Masters};

const INSTRUMENT: &str = "MOLY_RANK_CHANGE";
const PLAYER_DATA: &str = "moly://fixture-models/player-data.json";
/// `SiteActionExecutor.DEFAULT_FADE_OUT_TIME`.
const FADE_OUT_TIME: f32 = 0.25;
/// `SiteActionExecutor.DEFAULT_FADE_IN_TIME`.
const FADE_IN_TIME: f32 = 0.75;
/// Product guard, not a source value: the room's navigation snapshot for
/// the rebuilt site is awaited this long (real seconds), then the
/// performance goes on with a warning.
const NAVIGATION_WAIT_REAL: f64 = 5.0;
/// Product guard, not a source value: the rebuilt site's own reseed and
/// framing run on the frames the site settles; the kept poses are written
/// back this many frames after the site stands.
const SETTLE_FRAMES: u64 = 1;
/// The housing sites whose displayed level follows the rank.
const HOUSING: [&str; 4] = ["home_site", "first_floor", "second_floor", "third_floor"];

/// The client's local lists (`ApplicationLocalSettings.MysekaiTopics` and
/// `.UnlockedMasterSiteLevelIds`).
#[derive(Resource, Default, Debug)]
pub(crate) struct MysekaiLocalSettings(pub(crate) LocalLists);

/// `UserMysekaiGamedata.mysekaiRank` of the server's latest reply, once a
/// rank change has delivered it.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct UserMysekaiRank(pub(crate) i32);

/// The master rows, or why the root has none.
#[derive(Resource)]
pub(crate) struct ExpansionMasters(pub(crate) Result<Masters, String>);

#[derive(Resource)]
struct MastersHandle(Handle<JsonAsset>);

/// The rank change the instrument delivers.
#[derive(Resource, Clone, Copy, Debug)]
struct PendingRankChange {
    prev: i32,
    current: i32,
}

/// Holds the site loader until the rank change has set the housing
/// sites' displayed levels.
#[derive(Resource)]
pub(crate) struct LevelHold;

fn parse_instrument() -> Option<PendingRankChange> {
    let raw = std::env::var(INSTRUMENT).ok()?;
    let parsed = raw.split_once(',').and_then(|(prev, current)| {
        Some((
            prev.trim().parse::<i32>().ok()?,
            current.trim().parse::<i32>().ok()?,
        ))
    });
    match parsed {
        Some((prev, current)) if prev > 0 && current > 0 => {
            Some(PendingRankChange { prev, current })
        }
        _ => {
            error!("[site-expansion] {INSTRUMENT}={raw:?} is not <prev>,<current> with positive ranks; no rank change is delivered");
            None
        }
    }
}

/// Startup: the local lists, the master request and the instrument.
fn load(mut commands: Commands, server: Res<AssetServer>) {
    let Some(change) = parse_instrument() else {
        return;
    };
    info!(
        "[site-expansion] {INSTRUMENT}: the server's reply delivers UserMysekaiGamedata.mysekaiRank {} -> {}; the site loader waits for the displayed levels",
        change.prev, change.current
    );
    commands.insert_resource(change);
    commands.insert_resource(LevelHold);
    commands.insert_resource(MastersHandle(server.load::<JsonAsset>(PLAYER_DATA)));
}

/// Update, before the site loader: once the masters are in, deliver the rank
/// change and set the housing sites' displayed levels.
fn apply_rank_change(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<MastersHandle>>,
    pending: Option<Res<PendingRankChange>>,
    mut local: ResMut<MysekaiLocalSettings>,
    selection: Option<ResMut<SiteSelection>>,
) {
    let (Some(handle), Some(pending)) = (handle, pending) else {
        return;
    };
    let masters = match server.load_state(&handle.0) {
        LoadState::Failed(error) => Err(format!("player-data.json is not in this root ({error})")),
        _ => match jsons.get(&handle.0) {
            Some(json) => Masters::parse(&json.0),
            None => return,
        },
    };
    let Some(mut selection) = selection else {
        return;
    };
    commands.remove_resource::<MastersHandle>();
    commands.remove_resource::<PendingRankChange>();
    commands.remove_resource::<LevelHold>();
    let masters = match masters {
        Ok(masters) => masters,
        Err(reason) => {
            error!("[site-expansion] the rank change is refused: {reason}; sites load at their ordinary levels");
            commands.insert_resource(ExpansionMasters(Err(reason)));
            return;
        }
    };
    let added = local
        .0
        .add_topic_if_needed(&masters, pending.prev, pending.current);
    info!(
        "[site-expansion] MysekaiTopicsManager.AddTopicIfNeeded(rank {} -> {}): AddSiteExpansionTopicIfNeeded {}; topics {:?}, UnlockedMasterSiteLevelIds {:?}",
        pending.prev,
        pending.current,
        if added { "added topic 1 (SiteExpansion)" } else { "added nothing" },
        local.0.topics,
        local.0.unlocked_master_site_level_ids
    );
    for site_type in HOUSING {
        let Some(site_id) = masters.site_id(site_type) else {
            warn!("[site-expansion] no master site row for {site_type}");
            continue;
        };
        let level =
            local
                .0
                .mysekai_site_unlocked_level(&masters, pending.current, site_id, site_type);
        let need = local
            .0
            .unlock_master_site_level(&masters, pending.current, site_id)
            .map(|row| (row.id, row.level));
        if level > 0 {
            selection.set_level(site_type, level as u32);
        }
        info!(
            "[site-expansion] {site_type} (site {site_id}): displayed level {level} (GetMysekaiSiteUnlockedLevel at rank {}); rank level {}; unlock pending {:?}",
            pending.current,
            masters.mysekai_site_level(site_id, pending.current),
            need
        );
    }
    commands.insert_resource(UserMysekaiRank(pending.current));
    commands.insert_resource(ExpansionMasters(Ok(masters)));
}

// ---------------------------------------------------------------------------
// The room's performance
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Pose {
    translation: Vec3,
    rotation: Quat,
}

#[derive(Clone, Copy, Debug)]
struct CameraKeep {
    yaw: f32,
    pitch: f32,
    distance: f32,
    offset: Vec3,
}

enum Step {
    /// `FadeOutMyRoomAsync(0.25)`: waiting for its callback.
    FadeOut,
    /// `RankUpMyRoomAction`: the rebuilt site is awaited. `epoch` is the
    /// site generation before the rebuild; `ready` the frame the rebuilt
    /// site first stood.
    Rebuild {
        since_real: f64,
        epoch: u64,
        waiting: Option<String>,
        ready: Option<u64>,
    },
    /// `FadeInMyRoomAsync(0.75)`: waiting for its callback.
    FadeIn,
    /// `SetupNavMeshField`: the navigation snapshot of the rebuilt site.
    NavMesh { since_real: f64 },
}

/// `RoomSitePerformExecutor.PlayRoomSiteExpansionPerformAsync` in flight
/// (its `IsExecuting`).
#[derive(Resource)]
pub(crate) struct RoomExpansionPerform {
    site_type: String,
    site_id: i32,
    rank: i32,
    level_id: i32,
    from_level: u32,
    to_level: u32,
    player: Option<Pose>,
    camera: Option<CameraKeep>,
    step: Step,
}

/// `MyRoomSiteController.OnFinishEnterAsync`, after its screen setup:
/// `PlayRoomSiteExpansionPerformAsync` (then `SetupLockAtCameraBounds`, which
/// the product ran when the room settled).
pub(crate) fn room_finish_enter(world: &mut World) {
    if world.contains_resource::<RoomExpansionPerform>() {
        info!("[site-expansion] PlayRoomSiteExpansionPerformAsync: already executing; returns");
        return;
    }
    let Some(site) = world.get_resource::<SiteActive>().cloned() else {
        return;
    };
    let Some(rank) = world.get_resource::<UserMysekaiRank>().map(|rank| rank.0) else {
        // No reply has delivered a rank change: no topic can be held.
        return;
    };
    let Some(Ok(masters)) = world
        .get_resource::<ExpansionMasters>()
        .map(|m| m.0.clone())
    else {
        return;
    };
    info!(
        "[site-expansion] PlayRoomSiteExpansionPerformAsync on {} (site {}): SetLayerCanvasGroup(UI, interactable false); RemoveCollisionSensorEvent; PlaySiteExpansionAnimationIfNeed",
        site.site_type, site.site_id
    );
    let site_id = site.site_id as i32;
    let unlock = {
        let local = &world.resource::<MysekaiLocalSettings>().0;
        let need = local.is_need_site_expansion_action(&masters, rank, site_id);
        let unlock = local
            .unlock_master_site_level(&masters, rank, site_id)
            .cloned();
        info!(
            "[site-expansion] IsNeedSiteExpansionAction({site_id}) = {need} (topics {:?}, GetUnlockMasterMysekaiSiteLevel(rank {rank}, {site_id}) = {:?}, UnlockedMasterSiteLevelIds {:?})",
            local.topics,
            unlock.as_ref().map(|row| (row.id, row.level)),
            local.unlocked_master_site_level_ids
        );
        need.then_some(unlock).flatten()
    };
    let Some(unlock) = unlock else {
        info!(
            "[site-expansion] no expansion for this room; the other floors: {}",
            other_floors_note()
        );
        info!("[site-expansion] PlayRoomSiteExpansionPerformAsync ends: AddCollisionSensorEvent; SetLayerCanvasGroup(UI, interactable true)");
        return;
    };
    let removed = {
        let mut local = world.resource_mut::<MysekaiLocalSettings>();
        local.0.save_unlock_site_level(unlock.id);
        local.0.remove_site_expansion_topic(&masters, rank)
    };
    {
        let local = &world.resource::<MysekaiLocalSettings>().0;
        info!(
            "[site-expansion] SaveUnlockSiteLevel({}) -> UnlockedMasterSiteLevelIds {:?}; RemoveSiteExpansionTopic: {} (topics {:?})",
            unlock.id,
            local.unlocked_master_site_level_ids,
            if removed { "removed" } else { "kept (another site still needs one)" },
            local.topics
        );
    }
    world.insert_resource(game_state::UiHold("PlayRoomSiteExpansionPerformAsync"));
    let player = player_pose(world);
    let camera = world
        .get_resource::<FieldCameraModel>()
        .map(|model| CameraKeep {
            yaw: model.yaw,
            pitch: model.pitch,
            distance: model.distance,
            offset: model.offset,
        });
    log_room(world, "before", site.level);
    crate::screen_fade::fade_out(world, 0.0, FADE_OUT_TIME, "FadeOutMyRoomAsync(0.25)");
    world.insert_resource(RoomExpansionPerform {
        site_type: site.site_type.clone(),
        site_id,
        rank,
        level_id: unlock.id,
        from_level: site.level,
        to_level: unlock.level as u32,
        player,
        camera,
        step: Step::FadeOut,
    });
}

fn other_floors_note() -> &'static str {
    "GetSite of another floor is null (one site is loaded), so PlayOtherSiteExpansionAnimationIfNeed does not run"
}

fn player_pose(world: &mut World) -> Option<Pose> {
    let mut players = world.query_filtered::<&Transform, With<PlayerControlled>>();
    players.iter(world).next().map(|transform| Pose {
        translation: transform.translation,
        rotation: transform.rotation,
    })
}

fn camera_pose(world: &mut World) -> Option<(Vec3, Vec3)> {
    let mut cameras = world.query_filtered::<&Transform, With<Camera3d>>();
    cameras
        .iter(world)
        .next()
        .map(|transform| (transform.translation, *transform.forward()))
}

/// The room's floor extent: the bounds of the ground meshes as spawned.
fn room_bounds(world: &mut World) -> Option<(Vec3, Vec3)> {
    let ground = world.get_resource::<crate::site::GroundMeshes>()?.0.clone();
    let mut parts = world.query::<(&Mesh3d, &GlobalTransform)>();
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let meshes = world.resource::<Assets<Mesh>>();
    let mut any = false;
    for (mesh3d, global) in parts.iter(world) {
        if !ground.contains(&mesh3d.0) {
            continue;
        }
        let Some(positions) = meshes
            .get(&mesh3d.0)
            .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            .and_then(|values| values.as_float3())
        else {
            continue;
        };
        for position in positions {
            let point = global.transform_point(Vec3::from(*position));
            min = min.min(point);
            max = max.max(point);
            any = true;
        }
    }
    any.then_some((min, max))
}

fn log_room(world: &mut World, when: &str, level: u32) {
    let bounds = room_bounds(world);
    let camera = camera_pose(world);
    let ledger = world.get_resource::<SiteActive>().map(|site| {
        (
            site.site_type.clone(),
            site.room.as_ref().map(|room| room.level),
        )
    });
    let grid = ledger.as_ref().and_then(|(site_type, _)| {
        world
            .get_resource::<crate::site::Sites>()
            .and_then(|sites| sites.floor_grid(site_type, level).ok())
    });
    info!(
        "[site-expansion] room {when}: level {level} (room ledger level {:?}; floor grid {}); floor bounds {}; camera {}",
        ledger.and_then(|(_, level)| level),
        grid.map_or("unknown".into(), |grid| format!(
            "layout {} width {} height {} depth {}",
            grid.layout_id, grid.width, grid.height, grid.depth
        )),
        bounds.map_or("unknown".into(), |(min, max)| format!(
            "min ({:.3},{:.3},{:.3}) max ({:.3},{:.3},{:.3}) size ({:.3} x {:.3})",
            min.x, min.y, min.z, max.x, max.y, max.z, max.x - min.x, max.z - min.z
        )),
        camera.map_or("unknown".into(), |(eye, forward)| format!(
            "eye ({:.3},{:.3},{:.3}) forward ({:.4},{:.4},{:.4})",
            eye.x, eye.y, eye.z, forward.x, forward.y, forward.z
        ))
    );
}

/// `UpdateRoomView`: `LookAt` = the room site view's position (the loaded
/// site is the origin here), `FieldCamera.UpdatePosition`, then the view
/// looks at `LookAt + Offset`.
fn update_room_view_camera(world: &mut World, keep: Option<CameraKeep>) {
    let Some(mut model) = world.get_resource::<FieldCameraModel>().cloned() else {
        warn!("[site-expansion] UpdateRoomView: no camera model");
        return;
    };
    if let Some(keep) = keep {
        model.yaw = keep.yaw;
        model.pitch = keep.pitch;
        model.distance = keep.distance;
        model.offset = keep.offset;
    }
    model.look_at = Vec3::ZERO;
    let pivot = model.look_at + model.offset;
    let eye = pivot + crate::camera::view_dir(model.pitch, model.yaw) * model.distance;
    let mut cameras = world.query_filtered::<&mut Transform, With<Camera3d>>();
    if let Some(mut camera) = cameras.iter_mut(world).next() {
        *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
    }
    world.insert_resource(model.clone());
    info!(
        "[site-expansion] UpdateRoomView: LookAt (0,0,0) (the room site view), UpdatePosition: eye ({:.3},{:.3},{:.3}) yaw {:.2} pitch {:.2} distance {:.3}, LookAt(LookAt + Offset ({:.3},{:.3},{:.3}))",
        eye.x, eye.y, eye.z, model.yaw, model.pitch, model.distance, pivot.x, pivot.y, pivot.z
    );
}

/// Update (exclusive), before the screen fader advances: a callback of the
/// fader fired last frame is read this frame, as `WaitUntil` reads it.
pub(crate) fn advance_room(world: &mut World) {
    let Some(mut perform) = world.remove_resource::<RoomExpansionPerform>() else {
        return;
    };
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let done = match &mut perform.step {
        Step::FadeOut => {
            if world
                .resource::<crate::screen_fade::ScreenFader>()
                .finished()
            {
                rank_up(world, &mut perform, now);
            }
            false
        }
        Step::Rebuild {
            since_real,
            epoch,
            waiting,
            ready,
        } => {
            let frame = world
                .get_resource::<bevy::diagnostic::FrameCount>()
                .map_or(0, |count| u64::from(count.0));
            let blocker = if world
                .get_resource::<crate::site::GroundEpoch>()
                .is_none_or(|current| current.0 <= *epoch)
            {
                Some("site generation unchanged".to_owned())
            } else if world.contains_resource::<crate::inactive_nodes::SiteSettled>() {
                Some("site framing".to_owned())
            } else {
                crate::site_move::door::destination_blocker(
                    world,
                    &perform.site_type,
                    crate::site_move::door_law::DoorKind::HomeToRoom,
                )
            };
            match blocker {
                Some(reason) => {
                    *ready = None;
                    if waiting.as_deref() != Some(reason.as_str()) {
                        info!(
                            "[site-expansion] MyRoomSiteController.UpdateRoomView({}): the rebuilt room waits: {reason} ({:.3} s)",
                            perform.to_level,
                            now - *since_real
                        );
                        *waiting = Some(reason);
                    }
                }
                None => match *ready {
                    None => *ready = Some(frame),
                    Some(at) if frame >= at + SETTLE_FRAMES => {
                        after_rebuild(world, &mut perform, now)
                    }
                    Some(_) => {}
                },
            }
            false
        }
        Step::FadeIn => {
            if world
                .resource::<crate::screen_fade::ScreenFader>()
                .finished()
            {
                info!(
                    "[site-expansion] RankUpMyRoomAction ends; OpenCurrentMysekaiSiteLevelReleaseDialogAsync(site {}, rank {}): the level-release dialog (MSG_MYROOM_SITE_SIZE_EXPAND) is a UI dialog not drawn here; the performance goes on",
                    perform.site_id, perform.rank
                );
                perform.step = Step::NavMesh { since_real: now };
            }
            false
        }
        Step::NavMesh { since_real } => {
            let epoch = world
                .get_resource::<crate::site::GroundEpoch>()
                .map(|e| e.0);
            let ready = world
                .get_resource::<crate::player_fixture_action::PlayerFixtureNavigation>()
                .is_some_and(|navigation| Some(navigation.site_generation) == epoch);
            if ready || now - *since_real > NAVIGATION_WAIT_REAL {
                if ready {
                    info!("[site-expansion] SetupNavMeshField: the rebuilt room's navigation (site generation {epoch:?}) is set up");
                } else {
                    warn!("[site-expansion] SetupNavMeshField: no navigation snapshot for site generation {epoch:?} after {NAVIGATION_WAIT_REAL} s; going on");
                }
                info!(
                    "[site-expansion] SiteLayoutEditor.Refresh({}): no edit session is open; PlaySiteExpansionAnimationIfNeed ends",
                    perform.site_id
                );
                info!("[site-expansion] the other floors: {}", other_floors_note());
                world.remove_resource::<game_state::UiHold>();
                info!("[site-expansion] PlayRoomSiteExpansionPerformAsync ends: AddCollisionSensorEvent; SetLayerCanvasGroup(UI, interactable true); game state 11 holds until the next ChangeState");
                true
            } else {
                false
            }
        }
    };
    if !done {
        world.insert_resource(perform);
    }
}

/// `RankUpMyRoomAction(siteId, level)`: game state 11, `UpdateRoomView`.
fn rank_up(world: &mut World, perform: &mut RoomExpansionPerform, now: f64) {
    game_state::enter(
        world,
        GameStateType::LevelUpMyRoomSite,
        "SiteActionExecutor.RankUpMyRoomAction",
    );
    // LevelUpMyRoomSiteGameState.OnEnter: FieldCamera.ChangeState(None);
    // Normal's OnExit writes its model and the transfer data first.
    let state = world.resource::<FieldCameraState>().0;
    if state == CameraStateType::Normal {
        crate::site_move::camera::record_normal_exit(world, &perform.site_type);
    }
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::None;
    world.remove_resource::<crate::camera::CameraTween>();
    info!("[site-expansion] LevelUpMyRoomSiteGameState.OnEnter: FieldCamera.ChangeState(None) from {state:?}");
    update_room_view_camera(world, perform.camera);
    // MyRoomSiteController.UpdateRoomView(level): the room at its new size.
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map_or(0, |epoch| epoch.0);
    let mut next = world.resource::<SiteSelection>().clone();
    next.set_level(&perform.site_type, perform.to_level);
    let roots: Vec<Entity> = {
        let mut roots = world.query_filtered::<Entity, With<crate::site::SiteRoot>>();
        roots.iter(world).collect()
    };
    {
        let mut commands = world.commands();
        crate::site::queue_transition(&mut commands, roots, next);
    }
    world.flush();
    info!(
        "[site-expansion] MyRoomSiteController.UpdateRoomView({}): the room is rebuilt at level {} (was {}) under the black screen",
        perform.to_level, perform.to_level, perform.from_level
    );
    perform.step = Step::Rebuild {
        since_real: now,
        epoch,
        waiting: None,
        ready: None,
    };
}

/// The rebuilt room stands: the loader's reseed and framing are undone,
/// `ReloadPlayerLayoutDataAsync` has run with the rebuild, then
/// `FadeInMyRoomAsync(0.75)`.
fn after_rebuild(world: &mut World, perform: &mut RoomExpansionPerform, now: f64) {
    if let Some(pose) = perform.player {
        let mut players = world.query_filtered::<&mut Transform, With<PlayerControlled>>();
        for mut transform in players.iter_mut(world) {
            transform.translation = pose.translation;
            transform.rotation = pose.rotation;
        }
    }
    // The loader's framing wrote a new camera model and pose; the source
    // keeps the model and the pose UpdateRoomView set.
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::None;
    update_room_view_camera(world, perform.camera);
    let level = world
        .get_resource::<SiteActive>()
        .map_or(0, |site| site.level);
    let Step::Rebuild { since_real, .. } = perform.step else {
        return;
    };
    info!(
        "[site-expansion] the rebuilt room stands {:.3} s after UpdateRoomView (level id {}); player pose kept; SiteLayoutLoader.ReloadPlayerLayoutDataAsync: the layout was restored with the rebuild",
        now - since_real,
        perform.level_id
    );
    log_room(world, "after", level);
    crate::screen_fade::fade_in(world, 0.0, FADE_IN_TIME, "FadeInMyRoomAsync(0.75)");
    perform.step = Step::FadeIn;
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<MysekaiLocalSettings>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (
                apply_rank_change.before(crate::site::plan),
                advance_room.before(crate::screen_fade::advance),
            ),
        );
}
