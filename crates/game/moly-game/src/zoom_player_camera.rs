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
//! Entering runs the current state's `OnExit` first (`FieldCamera.ChangeState`
//! does nothing when the state is already ZoomPlayer): from Normal its
//! private model takes the live LookAt, distance, yaw and pitch unless its
//! transfer tween was cut short, and the site's transfer entry is written;
//! from the first-person state the player is shown again. Returning runs the
//! remembered state's `OnEnter` (`camera::change_state`): Normal's (the
//! inherit branch on a harvest or delivery site or at home after an outdoor
//! site, else `TransferCameraSettings`, whose previous-state switch has no
//! case for 12: the default path, 1.0 s to Normal's private model) or the
//! first-person state's (player hidden, 0.2 s to 0.15 m).
//!
//! Named difference: the camera lock has no counterpart here.

use bevy::prelude::*;

use crate::camera::{
    perspective_fov_deg, to_rotation, wrap180, CameraSetting, CameraStateType, CameraTween,
    FieldCameraModel, FieldCameraState, TweenCompletion,
};

/// `ZoomPlayerCameraState.OnEnter`'s distance literal.
const ZOOM_DISTANCE: f32 = 6.0;
/// `ZoomPlayerCameraState.OnEnter`'s tween duration literal.
const ZOOM_SECONDS: f32 = 0.25;

/// The state's private model (only the fields the state reads).
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct ZoomSnapshot {
    look_at: Vec3,
    pitch: f32,
    fov: f32,
    yaw: f32,
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

/// `FieldCamera.ChangeState(ZoomPlayer)`: the current state's `OnExit`
/// (`camera::exit_state`), then this state's `OnEnter`. Returns the state it
/// left (the caller remembers it for the way back), or the reason nothing
/// happened.
pub(crate) fn enter(world: &mut World) -> Result<CameraStateType, String> {
    let previous = world.resource::<FieldCameraState>().0;
    if previous == CameraStateType::ZoomPlayer {
        info!("[zoom-camera] ChangeState(ZoomPlayer) while ZoomPlayer: nothing (the state is unchanged)");
        return Ok(previous);
    }
    let Some(mut snapshot) = world.get_resource::<ZoomSnapshot>().copied() else {
        return Err("the zoom state's private model is not taken yet (no camera setting)".into());
    };
    if world.get_resource::<FieldCameraModel>().is_none() {
        return Err("the field camera model is not built yet".into());
    }
    crate::camera::exit_state(world, previous, "zoom-camera");
    let model = world.resource::<FieldCameraModel>().clone();
    let fov_now = camera_fov(world).unwrap_or(model.fov);
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

/// `FieldCamera.ChangeState(remembered)` from ZoomPlayer: its `OnExit` is
/// empty, then the remembered state's `OnEnter` (`camera::change_state`).
pub(crate) fn leave(world: &mut World, remembered: CameraStateType) {
    if world.resource::<FieldCameraState>().0 != CameraStateType::ZoomPlayer {
        warn!(
            "[zoom-camera] return to {remembered:?} skipped: the camera already left ZoomPlayer (now {:?})",
            world.resource::<FieldCameraState>().0
        );
        return;
    }
    crate::camera::change_state(world, remembered, "zoom-camera");
}
