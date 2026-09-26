//! Camera state 12, `ZoomPlayer`: the close shot on the player while a home
//! action (craft, canvas, sketch) plays, and the return to the state the
//! action remembered.
//!
//! The state keeps a private model: a copy of the field camera's model taken
//! when the camera's state machine is built, right after the camera builds a
//! fresh model from its setting. That copy's LookAt is zero (the model's
//! constructor leaves it), its pitch is the setting's initial pitch and its
//! field of view the setting's; its yaw is overwritten on every entry.
//!
//! `OnEnter`: the camera is unlocked, the private yaw takes the live yaw, and
//! the camera tweens (the base state's camera tween, default ease OutQuad) to
//! LookAt = the private LookAt, rotation = (private pitch, live yaw), field
//! of view = the private one, distance 6.0, in 0.25 s. `OnUpdate`: LookAt is
//! the player's view position, the eye is placed from it and looks at
//! LookAt + Offset (the same body as the house-entry state, so it shares that
//! state's arm of the follow system; the tween's LookAt is written first and
//! this write wins each frame). Drag, duo drag and pinch do nothing: the
//! camera input's state switch has no arm for this state. `OnExit` is empty.
//!
//! Entering from Normal runs Normal's `OnExit`: its private model takes the
//! live LookAt, distance, yaw and pitch unless its own transfer tween is in
//! flight, and the per-site transfer entry takes the live pitch, yaw, field
//! of view and distance. Returning to Normal runs Normal's `OnEnter`: the
//! bounds and offset come back from Normal's private model (the setting's
//! values), then either the inherit branch (a harvest or delivery site, or
//! home after an outdoor site: 0.5 s to the site's transfer entry) or the
//! transfer from this state: the previous-state switch has no case for 12, so
//! it takes the default path, a 1.0 s tween to Normal's private model
//! (LookAt, pitch, yaw, field of view, distance).
//!
//! Named differences: the camera lock has no counterpart here; Normal's
//! "transfer tween complete" flag is read as "no Normal transition tween in
//! flight"; entering from or returning to a state other than Normal (the
//! first-person camera) is refused by the caller, because that state's own
//! exit and entry are not reachable from this module.

use bevy::prelude::*;

use crate::camera::{
    is_inherit_camera_setting, perspective_fov_deg, to_rotation, wrap180, CameraSetting,
    CameraStateType, CameraTween, FieldCameraModel, FieldCameraState, PrevSiteType,
    TweenCompletion,
};

/// `ZoomPlayerCameraState.OnEnter`'s distance literal.
const ZOOM_DISTANCE: f32 = 6.0;
/// `ZoomPlayerCameraState.OnEnter`'s tween duration literal.
const ZOOM_SECONDS: f32 = 0.25;
/// `NormalCameraState.TransferCameraSettings`: `_doTweenCameraTime` is set to
/// 1.0 before the previous-state switch; state 12 has no case of its own and
/// cases 2 and 3 leave it.
const TRANSFER_SECONDS: f32 = 1.0;

/// The state's private model (only the fields the state reads).
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct ZoomSnapshot {
    look_at: Vec3,
    pitch: f32,
    fov: f32,
    yaw: f32,
}

/// Normal's private model as its `OnExit` leaves it, written when this state
/// takes over from Normal. Its field of view is never written after the
/// state is built, so it stays the setting's.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct NormalPrivate {
    look_at: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    fov: f32,
}

/// Update: take the private model once the camera setting is in (the source
/// builds the state machine, and with it this copy, when the camera is set
/// up; its values depend only on the setting).
pub(crate) fn capture_snapshot(
    mut commands: Commands,
    setting: Option<Res<CameraSetting>>,
    snapshot: Option<Res<ZoomSnapshot>>,
) {
    if snapshot.is_some() {
        return;
    }
    let Some(setting) = setting else {
        return;
    };
    let snapshot = ZoomSnapshot {
        look_at: Vec3::ZERO,
        pitch: setting.init_pitch,
        fov: setting.fov,
        yaw: setting.init_yaw,
    };
    info!(
        "[zoom-camera] ZoomPlayer private model taken at setup: LookAt (0,0,0), pitch {:.2}, FOV {:.2}",
        snapshot.pitch, snapshot.fov
    );
    commands.insert_resource(snapshot);
}

fn camera_fov(world: &mut World) -> Option<f32> {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    cameras.single(world).ok().map(perspective_fov_deg)
}

/// `FieldCamera.ChangeState(ZoomPlayer)`. Returns the state it left (the
/// caller remembers it for the way back), or the reason it refused.
pub(crate) fn enter(world: &mut World) -> Result<CameraStateType, String> {
    let previous = world.resource::<FieldCameraState>().0;
    if previous != CameraStateType::Normal && previous != CameraStateType::ZoomPlayer {
        return Err(format!(
            "the camera is in {previous:?}; only Normal's exit and entry are reachable from the zoom state"
        ));
    }
    let Some(mut snapshot) = world.get_resource::<ZoomSnapshot>().copied() else {
        return Err("the zoom state's private model is not taken yet (no camera setting)".into());
    };
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        return Err("the field camera model is not built yet".into());
    };
    let fov_now = camera_fov(world).unwrap_or(model.fov);
    if previous == CameraStateType::Normal {
        normal_exit(world, &model, "zoom-camera");
    }
    // ZoomPlayerCameraState.OnEnter.
    snapshot.yaw = model.yaw;
    world.insert_resource(snapshot);
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let tween = CameraTween {
        look_at: (model.look_at, snapshot.look_at),
        fov: (fov_now, snapshot.fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, snapshot.pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, snapshot.yaw)),
        distance: (model.distance, ZOOM_DISTANCE),
        duration: ZOOM_SECONDS,
        elapsed: 0.0,
        on_complete: TweenCompletion::None,
    };
    info!(
        "[zoom-camera] ChangeState({previous:?} -> ZoomPlayer): tween {ZOOM_SECONDS}s OutQuad yaw {:.2} kept, pitch {:.2} -> {:.2}, distance {:.2} -> {ZOOM_DISTANCE}, FOV {:.2} -> {:.2}; LookAt follows the player",
        tween.yaw.0, tween.pitch.0, tween.pitch.1, tween.distance.0, tween.fov.0, tween.fov.1
    );
    world.insert_resource(tween);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::ZoomPlayer;
    Ok(previous)
}

/// `NormalCameraState.OnExit` with the live `model`: Normal's private model
/// takes the live LookAt, distance, yaw and pitch unless its own transfer
/// tween is in flight, and the per-site transfer entry is written.
pub(crate) fn normal_exit(world: &mut World, model: &FieldCameraModel, tag: &str) {
    let transfer_in_flight = world
        .get_resource::<CameraTween>()
        .is_some_and(|tween| !tween.is_normal_reset());
    if transfer_in_flight {
        info!("[{tag}] Normal.OnExit: its transfer tween is in flight; its private model is kept");
    } else {
        let fov = world
            .get_resource::<NormalPrivate>()
            .map(|private| private.fov)
            .or_else(|| world.get_resource::<CameraSetting>().map(|setting| setting.fov))
            .unwrap_or(model.fov);
        world.insert_resource(NormalPrivate {
            look_at: model.look_at,
            distance: model.distance,
            yaw: model.yaw,
            pitch: model.pitch,
            fov,
        });
    }
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|active| active.site_type.clone());
    if let Some(site) = site {
        crate::site_move::camera::record_normal_exit(world, &site);
    }
}

/// `FieldCamera.ChangeState(remembered)` from ZoomPlayer (its `OnExit` is
/// empty). Only Normal is accepted by [`enter`], so only Normal comes back.
pub(crate) fn leave(world: &mut World, remembered: CameraStateType) {
    if world.resource::<FieldCameraState>().0 != CameraStateType::ZoomPlayer {
        warn!(
            "[zoom-camera] return to {remembered:?} skipped: the camera already left ZoomPlayer (now {:?})",
            world.resource::<FieldCameraState>().0
        );
        return;
    }
    if remembered != CameraStateType::Normal {
        error!("[zoom-camera] return to {remembered:?} refused: only Normal's entry is reachable; the camera goes to Normal");
    }
    normal_enter(world, CameraStateType::ZoomPlayer, "zoom-camera");
}

/// `FieldCamera.ChangeState(Normal)` from `from` (whose `OnExit` already
/// ran): `NormalCameraState.OnEnter`. The inherit branch first; otherwise
/// the bounds and offset come back from the setting and
/// `TransferCameraSettings` runs. Its previous-state switch copies the live
/// yaw into Normal's private model for the floor and wall editors (2, 3);
/// the zoom state (12) has no case of its own. Both then take the default
/// path, a 1.0 s tween to Normal's private model.
pub(crate) fn normal_enter(world: &mut World, from: CameraStateType, tag: &str) {
    let site = world.get_resource::<crate::site::SiteActive>().cloned();
    let prev_site = world.resource::<PrevSiteType>().0.clone();
    let inherit = site
        .as_ref()
        .is_some_and(|site| is_inherit_camera_setting(&site.category, &prev_site));
    if inherit {
        let site = site.expect("inherit needs the active site");
        info!(
            "[{tag}] ChangeState({from:?} -> Normal) at {} after {prev_site}: the inherit branch",
            site.site_type
        );
        crate::site_move::camera::enter_normal(world, &site.site_type, &site.category, &prev_site);
        return;
    }
    // NormalCameraState.OnEnter: bounds and offset from the private model.
    let setting = world.get_resource::<CameraSetting>().copied();
    let mut private = world.get_resource::<NormalPrivate>().copied();
    let fov_now = camera_fov(world);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    // TransferCameraSettings' previous-state switch, cases 2 and 3: Normal's
    // private model takes the live yaw.
    let edit_yaw = matches!(from, CameraStateType::FloorEdit | CameraStateType::WallEdit);
    let live_yaw = world.get_resource::<FieldCameraModel>().map(|model| model.yaw);
    if let (true, Some(private), Some(yaw)) = (edit_yaw, private.as_mut(), live_yaw) {
        private.yaw = yaw;
        world.insert_resource(*private);
    }
    let (Some(setting), Some(private)) = (setting, private) else {
        warn!("[{tag}] ChangeState({from:?} -> Normal) without its tween: no camera setting or Normal private model");
        return;
    };
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!("[{tag}] ChangeState({from:?} -> Normal) without its tween: no camera model");
        return;
    };
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    // TransferCameraSettings, default path.
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let tween = CameraTween {
        look_at: (model.look_at, private.look_at),
        fov: (fov_now.unwrap_or(model.fov), private.fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, private.pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, private.yaw)),
        distance: (model.distance, private.distance),
        duration: TRANSFER_SECONDS,
        elapsed: 0.0,
        on_complete: TweenCompletion::None,
    };
    drop(model);
    info!(
        "[{tag}] ChangeState({from:?} -> Normal): TransferCameraSettings {}default path, {TRANSFER_SECONDS}s to Normal's private model: distance {:.2} -> {:.2}, pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, FOV {:.2} -> {:.2}",
        if edit_yaw { "case 2/3 (private yaw = live yaw), then the " } else { "" },
        tween.distance.0,
        tween.distance.1,
        tween.pitch.0,
        tween.pitch.1,
        tween.yaw.0,
        tween.yaw.1,
        tween.fov.0,
        tween.fov.1
    );
    world.insert_resource(tween);
}
