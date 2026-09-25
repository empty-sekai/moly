//! Learning a site phenomenon, the client part: GameState
//! LearnSiteEnvironment (6) and the camera state LearnSiteEnvironment (17).
//!
//! The source, in order:
//! - `HarvestSiteController.OnFinishEnterAsync`, outside the tutorial: when
//!   `SiteEnvironmentUtility.HasTodayEnvironment()` is false,
//!   `GameStateManager.ChangeState(LearnSiteEnvironment)`. The predicate is
//!   an `Any` over the user's `userMysekaiPhenomena` rows whose
//!   `mysekaiPhenomenaId` equals `UserDataManager.TodayMysekaiPhenomenaId`
//!   (false when the list is null); it has no site argument.
//! - `LearnSiteEnvironmentGameState.OnEnter` publishes `ChangeGameState(6)`
//!   and turns the gesture layer off; `CustomJoyStick.OnChangeGameState`
//!   resets the stick and hides it on state 6 (its dispatch table), and
//!   shows it again on state 1. `OnExit` turns the gesture layer back on.
//! - `HarvestPresenter.OnChangeGameState`, on state 6 outside the tutorial:
//!   `SiteEnvironmentUtility.ExecuteReleaseAPI(siteId)` (the server records
//!   the phenomenon; the reply's `SuiteUser` is merged with `UpdateAll`), the
//!   phenomenon master row of today's id, then `PlayLearnEnvironment`:
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
//! Named gaps: the dialog is the UI lane's (a seam patch describes it); the
//! product logs where it would open and closes it at once; the release reply
//! lands in the same frame (the source's API call is asynchronous); the
//! harvest button during GameState 6 is not gated (the harvest screen is the
//! UI lane's).

use bevy::animation::AnimatedBy;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use crate::camera::{
    perspective_fov_deg, view_dir, CameraSetting, CameraStateType, CameraTween, FieldCameraModel,
    FieldCameraState, NormalCameraMemory,
};
use crate::character::AvatarRoot;
use crate::site_move::timeline::Delay;

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

/// One `UserMysekaiPhenomena` row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserPhenomenon {
    pub(crate) phenomena_id: i32,
    pub(crate) obtained_at: i64,
}

/// `SiteEnvironmentUtility.HasTodayEnvironment`: the list is non-null and
/// some row's id minus today's id is 0.
pub(crate) fn has_today_environment(rows: Option<&[UserPhenomenon]>, today: i32) -> bool {
    rows.is_some_and(|rows| rows.iter().any(|row| row.phenomena_id - today == 0))
}

/// GameState LearnSiteEnvironment (6) is current: the joystick is hidden and
/// the gesture layer is off.
#[derive(Resource)]
pub(crate) struct LearnSiteEnvironmentActive;

/// The learn run and the camera state's lookups.
#[derive(Resource, Default)]
pub(crate) struct LearnEnvironment {
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

enum Stage {
    /// `Delay(1.3 s)` after the camera change.
    Delay(Delay),
    /// The dialog closed at once; the `WaitUntil` sees it the next frame.
    WaitClose { closed_frame: u64 },
}

/// `LearnSiteEnvironmentCameraState.OnEnter` on the owner model.
pub(crate) fn enter_tween(
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
pub(crate) fn return_tween(
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
            crate::site_move::camera::record_normal_exit(world, site);
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
    let yaw = crate::site_move::camera::source_yaw(local.rotation);
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

/// Update: the trigger, the release, the delay, the dialog point and the
/// return.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    time: Res<Time>,
    frames: Res<FrameCount>,
    mut learn: ResMut<LearnEnvironment>,
    site: Option<Res<crate::site::SiteActive>>,
    site_move: Option<Res<crate::site_move::SiteMoveActive>>,
    phenomenon: Option<Res<crate::weather::CurrentPhenomenonId>>,
    mut user: Option<ResMut<super::catalog::HarvestUserData>>,
    mut mock: Option<ResMut<super::server_mock::HarvestServerMock>>,
) {
    let frame = u64::from(frames.0);
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
    if finished_enter && site.category == "harvest" {
        let Some(today) = phenomenon.as_deref().map(|id| id.0) else {
            error!("[harvest-learn] OnFinishEnterAsync: no current phenomenon id; HasTodayEnvironment is not evaluated");
            return;
        };
        let known = user.as_deref().map(|user| user.phenomena.as_slice());
        let has = has_today_environment(known, today);
        info!(
            "[harvest-learn] HarvestSiteController.OnFinishEnterAsync on {} (site {}): HasTodayEnvironment({today}) = {has} over {} user phenomena rows",
            site.site_type,
            site.site_id,
            known.map_or(0, <[UserPhenomenon]>::len)
        );
        if !has && learn.run.is_none() {
            // GameStateManager.ChangeState(LearnSiteEnvironment).
            commands.insert_resource(LearnSiteEnvironmentActive);
            info!("[harvest-learn] GameState LearnSiteEnvironment: ChangeGameState(6) published, gesture layer off, joystick reset and hidden");
            // HarvestPresenter.OnChangeGameState(6): ExecuteReleaseAPI.
            match (mock.as_deref_mut(), user.as_deref_mut()) {
                (Some(mock), Some(user)) => {
                    let rows = mock.release(site.site_id, today);
                    info!(
                        "[harvest-learn] PostUserMysekaiReleaseApi(site {}) -> ReleaseApiMock reply: userMysekaiPhenomena (id, obtainedAt) {:?} (UpdateAll)",
                        site.site_id,
                        rows.iter()
                            .map(|row| (row.phenomena_id, row.obtained_at))
                            .collect::<Vec<_>>()
                    );
                    user.phenomena = rows;
                }
                _ => error!("[harvest-learn] ExecuteReleaseAPI: the server mock or the user data is not built; no reply merged"),
            }
            // GetMysekaiPhenomena(today), then PlayLearnEnvironment -> Play.
            match crate::sitemap_phenomena::PHENOMENA_ROWS
                .iter()
                .find(|row| row.id == today)
            {
                Some(row) => info!(
                    "[harvest-learn] PlayLearnEnvironment: phenomenon master row {} ({}, {})",
                    row.id, row.asset, row.en
                ),
                None => {
                    error!("[harvest-learn] PlayLearnEnvironment: no phenomenon master row {today}")
                }
            }
            commands.queue(enter_learn_camera);
            learn.camera_elapsed = 0.0;
            learn.camera_logged = -1.0;
            learn.run = Some(LearnRun {
                phenomena_id: today,
                stage: Stage::Delay(Delay::new(DIALOG_DELAY_SECONDS, frame)),
            });
        }
    }
    let dt = time.delta_secs();
    let mut closed = false;
    let Some(run) = learn.run.as_mut() else {
        return;
    };
    match &mut run.stage {
        Stage::Delay(delay) => {
            if delay.tick(frame, dt) {
                match crate::sitemap_phenomena::PHENOMENA_ROWS
                    .iter()
                    .find(|row| row.id == run.phenomena_id)
                {
                    Some(row) => info!(
                        "[harvest-learn] Delay({:.1}s) done after {:.4}s: HarvestUtility.ShowLearnPhenomenaDialog would open here: ScreenManager.ShowSubWindowDialog<LearnPhenomenaSubWindowDialog>(dialog type {DIALOG_TYPE}, allowCloseExternal), Setup(the name of phenomenon {} ({}), thumbnail {THUMBNAIL_PREFIX}{}); the dialog is the UI lane's, closed at once here",
                        DIALOG_DELAY_SECONDS, delay.elapsed, row.id, row.en, row.icon
                    ),
                    None => error!(
                        "[harvest-learn] Delay done: ShowLearnPhenomenaDialog has no master row {}; the dialog point is passed",
                        run.phenomena_id
                    ),
                }
                run.stage = Stage::WaitClose {
                    closed_frame: frame,
                };
            }
        }
        Stage::WaitClose { closed_frame } => closed = frame > *closed_frame,
    }
    if closed {
        // WaitUntil(closed), FieldCamera.ChangeState(Normal); Play returns
        // and PlayLearnEnvironment changes the game state.
        commands.queue(leave_learn_camera);
        commands.remove_resource::<LearnSiteEnvironmentActive>();
        info!("[harvest-learn] dialog closed: FieldCamera.ChangeState(Normal), then GameState Normal: gesture layer on, joystick shown");
        learn.run = None;
    }
}

/// PostUpdate, after the field camera's follow (which advanced the entry
/// tween): `LearnSiteEnvironmentCameraState.OnUpdate`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_learn_camera(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    mut learn: ResMut<LearnEnvironment>,
    models: Option<ResMut<FieldCameraModel>>,
    drivers: Query<&crate::player_avatar::AvatarDriver>,
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

    /// HasTodayEnvironment: null is false, an empty list is false, another
    /// id is false, today's id is true.
    #[test]
    fn has_today_environment_rule() {
        let rows = [UserPhenomenon {
            phenomena_id: 3,
            obtained_at: 0,
        }];
        assert!(!has_today_environment(None, 3));
        assert!(!has_today_environment(Some(&[]), 3));
        assert!(!has_today_environment(Some(&rows), 4));
        assert!(has_today_environment(Some(&rows), 3));
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
