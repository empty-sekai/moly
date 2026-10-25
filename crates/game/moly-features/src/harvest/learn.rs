//! Learning a site phenomenon, the client part: GameState
//! LearnSiteEnvironment (6) and the camera state LearnSiteEnvironment (17).
//!
//! The source, in order:
//! - `HarvestSiteController.OnFinishEnterAsync`, outside the tutorial: when
//!   `SiteEnvironmentUtility.HasTodayEnvironment()` is false,
//!   `GameStateManager.ChangeState(LearnSiteEnvironment)`. The predicate is
//!   an `Any` over the user's `userMysekaiPhenomenas` rows whose
//!   `mysekaiPhenomenaId` equals `UserDataManager.TodayMysekaiPhenomenaId`
//!   (false when the list is null); it has no site argument.
//! - `LearnSiteEnvironmentGameState.OnEnter` publishes `ChangeGameState(6)`
//!   and turns the gesture layer off; `CustomJoyStick.OnChangeGameState`
//!   resets the stick and hides it on state 6 (its dispatch table), and
//!   shows it again on state 1. `OnExit` turns the gesture layer back on.
//! - `HarvestPresenter.OnChangeGameState`, on state 6 outside the tutorial:
//!   `SiteEnvironmentUtility.ExecuteReleaseAPI(siteId)` (`Call::Release` on
//!   the server link; the server appends today's phenomenon to the user's
//!   `userMysekaiPhenomenas` and the reply's `SuiteUser` carries the rows,
//!   which the client merges with `UpdateAll`), the phenomenon master row of
//!   today's id, then `PlayLearnEnvironment`:
//!   `LearnSiteEnvironmentEffectPlayer.Play` and, when it returns,
//!   `GameStateManager.ChangeState(Normal)`.
//! - `Play`: `FieldCamera.ChangeState(LearnSiteEnvironment)`, a 1.3 s
//!   `UniTask.Delay`, `HarvestUtility.ShowLearnPhenomenaDialog` (a
//!   `LearnPhenomenaSubWindowDialog`, dialog type 381, set up with the
//!   master row's name and the thumbnail bundle
//!   `mysekai/thumbnail/phenomena/` + `iconAssetbundleName`), a `WaitUntil`
//!   on its close, then `FieldCamera.ChangeState(Normal)`.
//! - `LearnSiteEnvironmentCameraState.OnEnter`: the model's pitch range
//!   becomes (0, 360), then `DoTweenCameraSetting` to the player view's
//!   root position, pitch -1, the root's euler y wrapped to [0, 360], the
//!   model FOV, distance 3.5, 1.3 s, OutQuad. `OnUpdate`: LookAt = the
//!   avatar's `Head` bone (`FindDeep("Head")` on the avatar), UpdatePosition,
//!   the view looks at LookAt + Offset. The tween's LookAt channel runs in
//!   the tween update and is overwritten by OnUpdate every frame.
//! - Leaving 17 for Normal: Normal's `OnEnter` restores the bounds and the
//!   offset; `IsInheritCameraSetting` is false on a harvest or delivery site
//!   when the previous state is 17, so `TransferCameraSettings` runs; its
//!   case 17 sets Normal's private LookAt to the owner's, its private
//!   Distance to 8 and the tween time to 0.3 s, then tweens to the private
//!   pitch, yaw and FOV (the pitch and yaw Normal's OnExit copied when 17
//!   took over; the FOV the constructor copied from the camera setting).
//!
//! Product mapping: the harvest `OnFinishEnterAsync` point is the frame the
//! cannon move's `SiteMoveActive` goes away on a harvest site (the move's
//! GameState Normal step); `TodayMysekaiPhenomenaId` is read as the weather
//! chain's current phenomenon id there (a harvest site's environment is
//! today's phenomenon); the tutorial is out of scope, so its branches are
//! never taken.
//!
//! The rows `HasTodayEnvironment` reads are the user data the server sends in
//! `userMysekaiPhenomenas`: the join carries them and the release reply
//! replaces them, so a phenomenon learned once stays learned after a site
//! switch and after a relaunch (the server keeps the rows in its document).
//! The learn flow continues on the release's reply, as the source's
//! `ExecuteAsync` continuation does, and a refused request opens no dialog
//! (the source's API error).
//!
//! The dialog is the UI lane's: this module sends
//! [`LearnPhenomenaDialogRequest`] where `ShowLearnPhenomenaDialog` opens it
//! and waits for [`LearnPhenomenaDialogClosed`] (its onClose).
//!
//! The harvest button in GameState 6: the source has no gate for it. No
//! harvest screen subscribes to `ChangeGameState` (the subscribers are the
//! game states, the joystick, the harvest presenter's release, the home and
//! my-room controllers and the photo view), and the press path
//! (`HarvestButtonInteraction.OnPressed`, `OnHarvestAction`,
//! `PlayHarvestAction` up to its `ChangeState(Harvest)`) reads no game
//! state; while the dialog is open its full-screen close area takes the
//! taps, which here is the dialog slot's field-input block.
//!
//! Named gaps: the release request is sent when the game state changes, but
//! the entry tween does not wait for its reply (the source starts the API
//! call and the tween together).

use bevy::animation::AnimatedBy;
use bevy::diagnostic::FrameCount;
pub use crate::field_modes::LearnSiteEnvironmentActive;

use bevy::prelude::*;
use moly_link::ServerLink;
use moly_protocol::{Call, Reply};

use crate::camera::{
    perspective_fov_deg, view_dir, CameraSetting, CameraStateType, CameraTween, FieldCameraModel,
    FieldCameraState, NormalCameraMemory,
};
use crate::character::AvatarRoot;
use crate::site_move::timeline::Delay;
use crate::userdata::client::phenomena::{
    ClientMysekaiPhenomena, UserMysekaiPhenomena, UserMysekaiReleaseRequest,
};
use crate::userdata::{Answered, ServerPending};

/// `LearnSiteEnvironmentCameraState.OnEnter`: the tween's pitch, distance
/// and duration (1.3f; the native constant is the single-precision value).
const LEARN_PITCH: f32 = -1.0;
const LEARN_DISTANCE: f32 = 3.5;
const LEARN_TWEEN_SECONDS: f32 = 1.3;
/// `LearnSiteEnvironmentEffectPlayer.Play`: the delay before the dialog.
const DIALOG_DELAY_SECONDS: f32 = 1.3;
/// `TransferCameraSettings`, previous state 17: Normal's private Distance
/// and the tween time.
const RETURN_DISTANCE: f32 = 8.0;
const RETURN_SECONDS: f32 = 0.3;
/// `HarvestUtility.ShowLearnPhenomenaDialog`: the dialog type and the
/// thumbnail bundle prefix (`AssetBundleNames.GetMysekaiPhenomenaThumbnail`).
const DIALOG_TYPE: i32 = 381;
const THUMBNAIL_PREFIX: &str = "mysekai/thumbnail/phenomena/";

/// `HarvestUtility.ShowLearnPhenomenaDialog(master, onClose)`: open a
/// `LearnPhenomenaSubWindowDialog` (dialog type 381, the dialog slot,
/// `allowCloseExternal`) and `Setup(name, thumbnail)`: the message body is
/// the wording `MSG_LEARN_PHENOMENA` formatted with the name, the image is
/// the thumbnail, the button opens the common balloon with the name, and the
/// open plays `se_get_blueprint`.
#[derive(Message, Clone, Debug)]
pub struct LearnPhenomenaDialogRequest {
    /// The phenomenon master row id (`mysekaiPhenomenas`,
    /// [`crate::sitemap_phenomena::PhenomenaMaster`]).
    pub phenomena_id: i32,
    /// `mysekai/thumbnail/phenomena/` + `iconAssetbundleName`.
    pub thumbnail: String,
}

/// The dialog's onClose: the flag the effect player's `WaitUntil` reads.
#[derive(Message, Clone, Copy, Debug)]
pub struct LearnPhenomenaDialogClosed;

/// The learn run and the camera state's lookups.
#[derive(Resource, Default)]
pub struct LearnEnvironment {
    was_moving: bool,
    run: Option<LearnRun>,
    head: Option<Entity>,
    head_missing_logged: bool,
    camera_elapsed: f32,
    camera_logged: f32,
}

struct LearnRun {
    phenomena_id: i32,
    stage: Stage,
}

impl LearnEnvironment {
    /// The release request the run waits for, if one is out.
    pub fn server_pending(&self) -> Option<ServerPending> {
        match &self.run.as_ref()?.stage {
            Stage::Release(pending) => Some(*pending),
            _ => None,
        }
    }
}

enum Stage {
    /// `ExecuteReleaseAPI` is out: the run waits for its reply.
    Release(ServerPending),
    /// `Delay(1.3 s)` after the camera change.
    Delay(Delay),
    /// The dialog is open until its onClose. The close is a tap, handled in
    /// the scripts' update after UniTask's Update runner (inserted first in
    /// the Update loop), so `WaitUntil` sees it at the next frame's runner,
    /// and Play continues there. This system runs before the dialog's taps,
    /// so it reads the close message at that same next frame.
    Open,
}

/// `LearnSiteEnvironmentCameraState.OnEnter` on the owner model.
pub fn enter_tween(
    model: &mut FieldCameraModel,
    camera_fov: f32,
    root: Vec3,
    root_yaw: f32,
) -> CameraTween {
    model.min_pitch = 0.0;
    model.max_pitch = 360.0;
    let fov = model.fov;
    crate::entry::law::camera_setting_tween(
        model,
        camera_fov,
        root,
        LEARN_PITCH,
        root_yaw,
        fov,
        LEARN_DISTANCE,
        LEARN_TWEEN_SECONDS,
    )
}

/// `NormalCameraState.OnEnter` after state 17 on a harvest site: the bounds
/// and offset from Normal's private model (the camera setting), then
/// `TransferCameraSettings` case 17. `gestured_distance` is the product's
/// seat of Normal's private Distance, which that case sets to 8.
pub fn return_tween(
    model: &mut FieldCameraModel,
    camera_fov: f32,
    setting: &CameraSetting,
    private_pitch: f32,
    private_yaw: f32,
) -> CameraTween {
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    model.gestured_distance = RETURN_DISTANCE;
    let look_at = model.look_at;
    crate::entry::law::camera_setting_tween(
        model,
        camera_fov,
        look_at,
        private_pitch,
        private_yaw,
        setting.fov,
        RETURN_DISTANCE,
        RETURN_SECONDS,
    )
}

fn camera_fov(world: &mut World) -> Option<f32> {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    cameras.single(world).map(perspective_fov_deg).ok()
}

/// `FieldCamera.ChangeState(LearnSiteEnvironment)`: Normal's OnExit snapshot
/// (when Normal is current), then this state's OnEnter.
fn enter_learn_camera(world: &mut World) {
    let from = world.resource::<FieldCameraState>().0;
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone());
    if from == CameraStateType::Normal {
        if let Some(site) = &site {
            crate::camera_site_move::record_normal_exit(world, site);
        }
    } else {
        warn!("[harvest-learn] FieldCamera.ChangeState(LearnSiteEnvironment) from {from:?}: no Normal exit snapshot");
    }
    let mut avatars = world.query_filtered::<(&Transform, &GlobalTransform), With<AvatarRoot>>();
    let Ok((local, global)) = avatars.single(world) else {
        warn!("[harvest-learn] no avatar root: LearnSiteEnvironment entered without its tween");
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::LearnSiteEnvironment;
        return;
    };
    let root = global.translation();
    let yaw = crate::fixture::source_yaw(local.rotation);
    let fov_now = camera_fov(world);
    let tween = world.get_resource_mut::<FieldCameraModel>().map(|mut model| {
        let fov_now = fov_now.unwrap_or(model.fov);
        let tween = enter_tween(&mut model, fov_now, root, yaw);
        info!(
            "[harvest-learn] LearnSiteEnvironmentCameraState.OnEnter: pitch range (0, 360); tween {:.1}s OutQuad to root {root:.3}, pitch {:.1} -> {:.1}, yaw {:.1} -> {:.1} (root euler y {yaw:.1}), distance {:.2} -> {:.2}, fov {:.1} -> {:.1}",
            tween.duration, tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1,
            tween.distance.0, tween.distance.1, tween.fov.0, tween.fov.1
        );
        tween
    });
    if let Some(tween) = tween {
        world.insert_resource(tween);
    }
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::LearnSiteEnvironment;
}

/// `FieldCamera.ChangeState(Normal)` from LearnSiteEnvironment.
fn leave_learn_camera(world: &mut World) {
    if world.resource::<FieldCameraState>().0 != CameraStateType::LearnSiteEnvironment {
        warn!("[harvest-learn] FieldCamera.ChangeState(Normal): the camera left LearnSiteEnvironment already");
        return;
    }
    let category = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.category.clone())
        .unwrap_or_default();
    let setting = world.get_resource::<CameraSetting>().copied();
    let private = world
        .get_resource::<NormalCameraMemory>()
        .map(|memory| (memory.pitch, memory.yaw));
    let fov_now = camera_fov(world);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    // IsInheritCameraSetting: a harvest or delivery site inherits unless the
    // previous state is 17, so this run takes TransferCameraSettings.
    if !matches!(category.as_str(), "harvest" | "delivery") {
        error!("[harvest-learn] Normal re-entry from LearnSiteEnvironment on a {category} site: the inherit rule reads the site-move history there, not ported");
        return;
    }
    let (Some(setting), Some((pitch, yaw))) = (setting, private) else {
        error!("[harvest-learn] Normal re-entry: the camera setting or Normal's private snapshot is missing; no TransferCameraSettings tween");
        return;
    };
    let tween = world.get_resource_mut::<FieldCameraModel>().map(|mut model| {
        let fov_now = fov_now.unwrap_or(model.fov);
        let tween = return_tween(&mut model, fov_now, &setting, pitch, yaw);
        info!(
            "[harvest-learn] NormalCameraState.OnEnter (previous 17, not inherited): TransferCameraSettings case 17: private distance 8, tween {:.1}s to pitch {:.1} -> {:.1}, yaw {:.1} -> {:.1}, distance {:.2} -> {:.2}, fov {:.1} -> {:.1}, look-at kept {:.3}",
            tween.duration, tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1,
            tween.distance.0, tween.distance.1, tween.fov.0, tween.fov.1, tween.look_at.1
        );
        tween
    });
    if let Some(tween) = tween {
        world.insert_resource(tween);
    }
}

/// The rows the release reply (or the login) gave the copy, in the log.
fn log_rows(rows: &[UserMysekaiPhenomena]) -> Vec<(i32, i64)> {
    rows.iter()
        .map(|row| (row.mysekai_phenomena_id, row.obtained_at))
        .collect()
}

/// `HasTodayEnvironment` false on a harvest site: GameState
/// `LearnSiteEnvironment` (`ChangeGameState(6)`), then
/// `ExecuteReleaseAPI`, whose reply the run waits for.
#[allow(clippy::too_many_arguments)]
fn start_learn(
    commands: &mut Commands,
    learn: &mut LearnEnvironment,
    site: &crate::site::SiteActive,
    link: Option<&mut ServerLink>,
    today: i32,
    has: bool,
    master: Option<&crate::sitemap_phenomena::PhenomenaMaster>,
) {
    if has || learn.run.is_some() {
        return;
    }
    commands.insert_resource(LearnSiteEnvironmentActive);
    info!("[harvest-learn] GameState LearnSiteEnvironment: ChangeGameState(6) published, gesture layer off, joystick reset and hidden");
    // HarvestPresenter.OnChangeGameState(6): ExecuteReleaseAPI. The server
    // decides today's phenomenon; the request names the site.
    let Some(link) = link else {
        error!("[harvest-learn] ExecuteReleaseAPI: the server link is absent; no request is sent and the run does not start");
        return;
    };
    let Ok(site_id) = i32::try_from(site.site_id) else {
        error!(
            "[harvest-learn] ExecuteReleaseAPI: site {} does not fit the request's int; no request is sent and the run does not start",
            site.site_id
        );
        return;
    };
    let call = Call::Release(UserMysekaiReleaseRequest {
        mysekai_site_id: site_id,
    });
    let kind = call.kind();
    let seq = link.call(call);
    info!("[harvest-learn] SiteEnvironmentUtility.ExecuteReleaseAPI(site {site_id}): PostUserMysekaiReleaseApi sent as seq {seq}; the learn flow continues on its reply");
    // GetMysekaiPhenomena(today), then PlayLearnEnvironment -> Play.
    match master.and_then(|master| master.phenomena(today)) {
        Some(row) => info!(
            "[harvest-learn] PlayLearnEnvironment: phenomenon master row {} ({}, {})",
            row.id, row.asset, row.en
        ),
        None => error!("[harvest-learn] PlayLearnEnvironment: no phenomenon master row {today}"),
    }
    commands.queue(enter_learn_camera);
    learn.camera_elapsed = 0.0;
    learn.camera_logged = -1.0;
    learn.run = Some(LearnRun {
        phenomena_id: today,
        stage: Stage::Release(ServerPending {
            seq: Some(seq),
            call: kind,
        }),
    });
}

/// Update: the trigger, the release, the delay, the dialog point and the
/// return.
#[allow(clippy::too_many_arguments)]
pub fn advance(
    mut commands: Commands,
    time: Res<Time>,
    frames: Res<FrameCount>,
    mut learn: ResMut<LearnEnvironment>,
    site: Option<Res<crate::site::SiteActive>>,
    site_move: Option<Res<crate::site_move::SiteMoveActive>>,
    phenomenon: Option<Res<crate::weather::CurrentPhenomenonId>>,
    known: Option<Res<ClientMysekaiPhenomena>>,
    mut link: Option<ResMut<ServerLink>>,
    mut answers: MessageReader<Answered>,
    (mut dialog_open, mut dialog_closed): (
        MessageWriter<LearnPhenomenaDialogRequest>,
        MessageReader<LearnPhenomenaDialogClosed>,
    ),
    master: Option<Res<crate::sitemap_phenomena::PhenomenaMaster>>,
) {
    let frame = u64::from(frames.0);
    let closed_now = dialog_closed.read().count() > 0;
    let moving = site_move.is_some();
    let finished_enter = learn.was_moving && !moving;
    learn.was_moving = moving;
    if moving {
        if learn.run.take().is_some() {
            commands.remove_resource::<LearnSiteEnvironmentActive>();
            warn!("[harvest-learn] a site move began during GameState LearnSiteEnvironment: the run is dropped");
        }
        return;
    }
    let Some(site) = site.as_deref() else {
        return;
    };
    // UserDataManager.UserMysekaiPhenomenas, as the server last served it.
    let Some(known) = known.as_deref() else {
        return;
    };
    if finished_enter && site.category == "harvest" {
        if let Some(today) = phenomenon.as_deref().map(|id| id.0) {
            let has = known.has_today_environment(today);
            info!(
                "[harvest-learn] HarvestSiteController.OnFinishEnterAsync on {} (site {}): HasTodayEnvironment({today}) = {has} over {} userMysekaiPhenomenas rows {:?}",
                site.site_type,
                site.site_id,
                known.rows().len(),
                log_rows(known.rows())
            );
            start_learn(
                &mut commands,
                &mut learn,
                site,
                link.as_deref_mut(),
                today,
                has,
                master.as_deref(),
            );
        } else {
            error!("[harvest-learn] OnFinishEnterAsync: no current phenomenon id; HasTodayEnvironment is not evaluated");
        }
    }
    // The release reply: a `SuiteUser` answer whose sections the copy
    // received; GetMysekaiPhenomena(today), then PlayLearnEnvironment -> Play.
    let mut released = None;
    for answer in answers.read() {
        if learn
            .server_pending()
            .is_some_and(|pending| pending.seq == Some(answer.seq))
        {
            released = Some(answer.outcome.clone());
        }
    }
    if let Some(outcome) = released {
        let today = learn.run.as_ref().map_or(0, |run| run.phenomena_id);
        match outcome {
            Ok(Reply::Release) => {
                let rows = known.rows();
                info!(
                    "[harvest-learn] PostUserMysekaiReleaseApi reply -> userMysekaiPhenomenas (id, obtainedAt) {:?} (UpdateAll); phenomenon {today} learned: {}",
                    log_rows(rows),
                    known.has_today_environment(today)
                );
                match master.as_deref().and_then(|master| master.phenomena(today)) {
                    Some(row) => info!(
                        "[harvest-learn] PlayLearnEnvironment: phenomenon master row {} ({}, {})",
                        row.id, row.asset, row.en
                    ),
                    None => {
                        error!("[harvest-learn] PlayLearnEnvironment: no phenomenon master row {today}")
                    }
                }
                if let Some(run) = learn.run.as_mut() {
                    run.stage = Stage::Delay(Delay::new(DIALOG_DELAY_SECONDS, frame));
                }
            }
            Ok(other) => {
                error!("[harvest-learn] PostUserMysekaiReleaseApi answered an unexpected {other:?}: no dialog plays; GameState Normal");
                commands.queue(leave_learn_camera);
                commands.remove_resource::<LearnSiteEnvironmentActive>();
                learn.run = None;
            }
            Err(refusal) => {
                error!("[harvest-learn] PostUserMysekaiReleaseApi refused ({}: {}): the API error, no dialog plays; GameState Normal", refusal.code, refusal.reason);
                commands.queue(leave_learn_camera);
                commands.remove_resource::<LearnSiteEnvironmentActive>();
                learn.run = None;
            }
        }
    }
    let dt = time.delta_secs();
    let mut closed = false;
    let Some(run) = learn.run.as_mut() else {
        return;
    };
    match &mut run.stage {
        Stage::Release(_) => {}
        Stage::Delay(delay) => {
            if delay.tick(frame, dt) {
                match master
                    .as_deref()
                    .and_then(|master| master.phenomena(run.phenomena_id))
                {
                    Some(row) => {
                        info!(
                            "[harvest-learn] Delay({:.1}s) done after {:.4}s: HarvestUtility.ShowLearnPhenomenaDialog: LearnPhenomenaSubWindowDialog (dialog type {DIALOG_TYPE}) requested for phenomenon {} ({}), thumbnail {THUMBNAIL_PREFIX}{}",
                            DIALOG_DELAY_SECONDS, delay.elapsed, row.id, row.en, row.icon
                        );
                        dialog_open.write(LearnPhenomenaDialogRequest {
                            phenomena_id: row.id,
                            thumbnail: format!("{THUMBNAIL_PREFIX}{}", row.icon),
                        });
                    }
                    None => error!(
                        "[harvest-learn] Delay done: ShowLearnPhenomenaDialog has no master row {}; the dialog point is passed",
                        run.phenomena_id
                    ),
                }
                run.stage = Stage::Open;
            }
        }
        Stage::Open => closed = closed_now,
    }
    if closed {
        // WaitUntil(closed), FieldCamera.ChangeState(Normal); Play returns
        // and PlayLearnEnvironment changes the game state.
        commands.queue(leave_learn_camera);
        commands.remove_resource::<LearnSiteEnvironmentActive>();
        info!(
            "[harvest-learn] frame {frame} t {:.4}: dialog closed (WaitUntil at the frame after the tap): FieldCamera.ChangeState(Normal), then GameState Normal: gesture layer on, joystick shown",
            time.elapsed_secs_f64()
        );
        learn.run = None;
    }
}

/// PostUpdate, after the field camera's follow (which advanced the entry
/// tween): `LearnSiteEnvironmentCameraState.OnUpdate`.
#[allow(clippy::too_many_arguments)]
pub fn advance_learn_camera(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    mut learn: ResMut<LearnEnvironment>,
    models: Option<ResMut<FieldCameraModel>>,
    drivers: Query<&crate::player_avatar::AvatarDriver, With<crate::markers::PlayerControlled>>,
    bones: Query<(Entity, &Name, &AnimatedBy)>,
    globals: Query<&GlobalTransform, Without<Camera3d>>,
    avatars: Query<Entity, With<AvatarRoot>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    if state.0 != CameraStateType::LearnSiteEnvironment {
        return;
    }
    let Some(mut models) = models else {
        return;
    };
    let Some(root) = avatars.single().ok().and_then(|e| globals.get(e).ok()) else {
        return;
    };
    let root = root.translation();
    if learn.head.is_none_or(|head| globals.get(head).is_err()) {
        let animator = drivers.iter().next().map(|driver| driver.player);
        learn.head = animator.and_then(|animator| {
            bones
                .iter()
                .find(|(_, name, by)| by.0 == animator && name.as_str() == "Head")
                .map(|(entity, _, _)| entity)
        });
    }
    let head = match learn.head.and_then(|head| globals.get(head).ok()) {
        Some(head) => head.translation(),
        None => {
            if !learn.head_missing_logged {
                learn.head_missing_logged = true;
                error!("[harvest-learn] the avatar's Head bone is not found: the look-at follows the avatar root instead");
            }
            root
        }
    };
    models.look_at = head;
    let pivot = models.look_at + models.offset;
    let eye = pivot + view_dir(models.pitch, models.yaw) * models.distance;
    if let Ok(mut camera) = cameras.single_mut() {
        *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
    }
    learn.camera_elapsed += time.delta_secs();
    if (learn.camera_elapsed / 0.25).floor() != (learn.camera_logged / 0.25).floor() {
        learn.camera_logged = learn.camera_elapsed;
        info!(
            "[harvest-learn] camera 17 t {:.3}: look-at = Head {head:.3} ({:.3} above the root), distance {:.3} pitch {:.2} yaw {:.2} (read back from the model)",
            learn.camera_elapsed,
            head.y - root.y,
            models.distance,
            models.pitch,
            models.yaw
        );
    }
}

#[cfg(test)]
mod value_checks {
    //! Research instrument: each expected value is computed by hand from the
    //! source routine named above it.
    use super::*;

    /// HasTodayEnvironment: an empty list (no rows served) is false, another
    /// id is false, today's id is true.
    #[test]
    fn has_today_environment_rule() {
        let mut copy = ClientMysekaiPhenomena::default();
        assert!(!copy.has_today_environment(3));
        copy.apply(vec![UserMysekaiPhenomena {
            mysekai_phenomena_id: 3,
            obtained_at: 0,
        }]);
        assert!(!copy.has_today_environment(4));
        assert!(copy.has_today_environment(3));
    }

    fn model() -> FieldCameraModel {
        FieldCameraModel {
            look_at: Vec3::new(1.0, 0.0, 2.0),
            offset: Vec3::new(0.0, 0.5, 0.0),
            fov: 35.0,
            distance: 6.0,
            min_distance: 2.0,
            max_distance: 10.0,
            yaw: 350.0,
            pitch: 20.0,
            rot_sensitivity: 0.075,
            min_pitch: 8.0,
            max_pitch: 70.0,
            gestured_distance: 6.0,
            look_at_bounds: crate::camera::BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
            max_look_at_bounds: crate::camera::BoundsXz {
                center: Vec2::ZERO,
                extents: Vec2::splat(f32::MAX),
            },
        }
    }

    /// OnEnter from pitch 20, yaw 350 (wrapped -10), root euler y 30: pitch
    /// -1, yaw -10 + 40 = 30, distance 3.5, 1.3 s, range (0, 360). The
    /// return with private pitch 20, yaw 350 from pitch -1, yaw 30: pitch 20,
    /// yaw 30 + wrap(320) = -10, distance 8, 0.3 s, the setting FOV, the
    /// look-at kept.
    #[test]
    fn learn_camera_tweens() {
        let mut m = model();
        let root = Vec3::new(3.0, 0.2, 4.0);
        let enter = enter_tween(&mut m, 40.0, root, 30.0);
        assert_eq!((m.min_pitch, m.max_pitch), (0.0, 360.0));
        assert_eq!(enter.look_at, (Vec3::new(1.0, 0.0, 2.0), root));
        assert_eq!(enter.pitch, (20.0, -1.0));
        assert_eq!(enter.yaw, (-10.0, 30.0));
        assert_eq!(enter.distance, (6.0, 3.5));
        assert_eq!(enter.fov, (40.0, 35.0));
        assert_eq!(enter.duration, 1.3);
        m.pitch = -1.0;
        m.yaw = 30.0;
        m.distance = 3.5;
        m.look_at = Vec3::new(3.0, 1.1, 4.0);
        let setting = CameraSetting {
            offset: Vec3::new(0.0, 0.4, 0.0),
            distance: 6.0,
            min_distance: 2.5,
            max_distance: 9.0,
            init_yaw: 0.0,
            init_pitch: 15.0,
            min_pitch: 5.0,
            max_pitch: 60.0,
            rot_sensitivity: 0.075,
            fov: 33.0,
        };
        let back = return_tween(&mut m, 35.0, &setting, 20.0, 350.0);
        assert_eq!((m.min_pitch, m.max_pitch), (5.0, 60.0));
        assert_eq!((m.min_distance, m.max_distance), (2.5, 9.0));
        assert_eq!(m.offset, setting.offset);
        assert_eq!(m.gestured_distance, 8.0);
        assert_eq!(
            back.look_at,
            (Vec3::new(3.0, 1.1, 4.0), Vec3::new(3.0, 1.1, 4.0))
        );
        assert_eq!(back.pitch, (-1.0, 20.0));
        assert_eq!(back.yaw, (30.0, -10.0));
        assert_eq!(back.distance, (3.5, 8.0));
        assert_eq!(back.fov, (35.0, 33.0));
        assert_eq!(back.duration, 0.3);
    }
}
