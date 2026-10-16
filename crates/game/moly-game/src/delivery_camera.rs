//! The honor reward camera (`DeliveryHonorRewardCameraState`, camera state
//! 20), entered by the player's honor reward state and left by its
//! `Dispose`.
//!
//! - Construction: its own copies of the setting's minimum and maximum
//!   pitch and offset, the camera model's minimum distance and the setting's
//!   maximum distance; the target distance is
//!   `DeliveryHonorRewardCameraDistance` (FloatConfigs 175).
//! - OnEnter: the camera locks; the model's pitch bounds, minimum distance
//!   and offset take the copies and its maximum distance takes the minimum
//!   distance copy; coming from the first-person state the look-at is the
//!   player's position; a sequence tweens the distance to the target and the
//!   pitch to 40 degrees, both 3.0 s OutQuint, joined.
//! - OnUpdate: the look-at moves toward the player by
//!   `dt / (1/60) * 0.1` clamped to [0, 1]; the camera sits on its orbit
//!   (look-at plus offset, back along the view direction by the distance)
//!   and looks at the look-at plus the offset. Pinch and drag do nothing.
//! - OnExit: the camera unlocks and the sequence is killed.
//!
//! Entering runs the current state's OnExit first (`camera::exit_state`):
//! Normal's writes its private model and the site's transfer entry; the
//! first-person state's shows the player. Leaving goes back to the previous
//! state through its OnEnter (`camera::enter_state`): Normal's (the inherit
//! branch on the delivery site, the tween back to the site's transfer entry)
//! or the first-person state's.
//!
//! The construction copies are taken at entry from the current camera
//! setting (the product builds no camera state objects up front; the
//! setting does not change within a site).

use bevy::prelude::*;
use moly_law::delivery as law;

use crate::camera::{CameraSetting, CameraStateType, FieldCameraModel, FieldCameraState};
use crate::player::PlayerControlled;

/// `DeliveryHonorRewardCameraDistance` (FloatConfigs 175).
pub(crate) const KEY_HONOR_CAMERA_DISTANCE: i32 = 175;

#[derive(Clone, Copy, Debug)]
struct Sequence {
    elapsed: f32,
    from_distance: f32,
    from_pitch: f32,
    to_distance: f32,
}

/// The state's own fields.
#[derive(Resource, Default)]
pub(crate) struct DeliveryHonorCamera {
    prev: Option<CameraStateType>,
    sequence: Option<Sequence>,
    last_logged: f32,
}

/// `ChangeState(DeliveryHonorReward)`: the current state's OnExit
/// (`camera::exit_state`), then this state's OnEnter.
pub(crate) fn enter(world: &mut World) {
    let from = world.resource::<FieldCameraState>().0;
    if from == CameraStateType::DeliveryHonorReward {
        return;
    }
    crate::camera::exit_state(world, from, "delivery-camera");
    let distance = world
        .get_resource::<crate::client_config::ClientConfigs>()
        .map(|configs| configs.float(KEY_HONOR_CAMERA_DISTANCE));
    let setting = world.get_resource::<CameraSetting>().copied();
    let player = {
        let mut players = world.query_filtered::<&GlobalTransform, With<PlayerControlled>>();
        players.single(world).ok().map(|p| p.translation())
    };
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::DeliveryHonorReward;
    let (Some(distance), Some(setting)) = (distance, setting) else {
        warn!("[delivery-camera] DeliveryHonorRewardCameraState.OnEnter without the camera setting or the client config: the camera stays");
        world.resource_mut::<DeliveryHonorCamera>().prev = Some(from);
        return;
    };
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!("[delivery-camera] no camera model at OnEnter");
        return;
    };
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.min_distance = setting.min_distance;
    model.max_distance = setting.min_distance;
    model.offset = setting.offset;
    if from == CameraStateType::Fps {
        if let Some(player) = player {
            model.look_at = player;
        }
    }
    let sequence = Sequence {
        elapsed: 0.0,
        from_distance: model.distance,
        from_pitch: model.pitch,
        to_distance: distance,
    };
    info!(
        "[delivery-camera] DeliveryHonorRewardCameraState.OnEnter from {from:?}: locked; pitch bounds [{}, {}], distance bounds [{}, {}], offset {:.3}; sequence distance {:.3} -> {distance} and pitch {:.2} -> {} over {} s OutQuint",
        model.min_pitch,
        model.max_pitch,
        model.min_distance,
        model.max_distance,
        model.offset,
        model.distance,
        model.pitch,
        law::HONOR_CAMERA_END_PITCH,
        law::HONOR_CAMERA_SECONDS
    );
    let mut state = world.resource_mut::<DeliveryHonorCamera>();
    state.prev = Some(from);
    state.sequence = Some(sequence);
    state.last_logged = -1.0;
}

/// `ChangeState(PrevState)`: this state's OnExit, then the previous state's
/// OnEnter (`camera::enter_state`).
pub(crate) fn exit(world: &mut World) {
    if world.resource::<FieldCameraState>().0 != CameraStateType::DeliveryHonorReward {
        return;
    }
    let (prev, sequence) = {
        let mut state = world.resource_mut::<DeliveryHonorCamera>();
        (
            state.prev.take().unwrap_or(CameraStateType::Normal),
            state.sequence.take(),
        )
    };
    let model = world.get_resource::<FieldCameraModel>().cloned();
    info!(
        "[delivery-camera] DeliveryHonorRewardCameraState.OnExit: unlocked, sequence killed at {:?} s; distance {:?} pitch {:?}; back to {prev:?}",
        sequence.map(|s| s.elapsed),
        model.as_ref().map(|m| m.distance),
        model.as_ref().map(|m| m.pitch)
    );
    crate::camera::enter_state(
        world,
        prev,
        CameraStateType::DeliveryHonorReward,
        "delivery-camera",
    );
}

/// PostUpdate, after the field camera's follow: the sequence and OnUpdate.
pub(crate) fn advance(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    mut honor: ResMut<DeliveryHonorCamera>,
    models: Option<ResMut<FieldCameraModel>>,
    players: Query<&GlobalTransform, With<PlayerControlled>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    if state.0 != CameraStateType::DeliveryHonorReward {
        return;
    }
    let Some(mut models) = models else {
        return;
    };
    let dt = time.delta_secs();
    let mut log = None;
    if let Some(sequence) = honor.sequence.as_mut() {
        sequence.elapsed += dt;
        let t = (sequence.elapsed / law::HONOR_CAMERA_SECONDS).clamp(0.0, 1.0);
        let e = law::ease_out_quint(t);
        models.distance =
            sequence.from_distance + (sequence.to_distance - sequence.from_distance) * e;
        models.pitch =
            sequence.from_pitch + (law::HONOR_CAMERA_END_PITCH - sequence.from_pitch) * e;
        log = Some((sequence.elapsed, t >= 1.0));
    }
    if let Ok(player) = players.single() {
        let k = law::honor_follow_factor(dt);
        let look_at = models.look_at;
        models.look_at = look_at + (player.translation() - look_at) * k;
    }
    let pivot = models.look_at + models.offset;
    let eye = pivot + crate::camera::view_dir(models.pitch, models.yaw) * models.distance;
    if let Ok(mut camera) = cameras.single_mut() {
        *camera = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
    }
    if let Some((elapsed, done)) = log {
        if done || (elapsed / 0.5).floor() != (honor.last_logged / 0.5).floor() {
            info!(
                "[delivery-camera] t {elapsed:.3} s: distance {:.3} pitch {:.2} (read back from the model), eye ({:.3}, {:.3}, {:.3})",
                models.distance, models.pitch, eye.x, eye.y, eye.z
            );
            honor.last_logged = elapsed;
        }
        if done {
            honor.sequence = None;
        }
    }
}
