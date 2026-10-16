//! Camera state 9, `CutScene`: entered by the cut-scene game state's
//! `OnEnter` (`FieldCamera.ChangeState(CutScene)`) and left by
//! `CutScenePresenter.ResetCameraState`.
//!
//! `CutSceneCameraState` has empty bodies: while it is current the
//! Cinemachine brain the presenter adds to the field camera drives the
//! camera from the director's Cinemachine track. The brain shows the virtual
//! camera of the active shot; a virtual camera with no Follow and no LookAt
//! target has neither a valid body (Transposer) nor a valid aim (Composer),
//! so its state is its own world pose, and the brain applies that pose and
//! the camera's lens (field of view, near and far planes) to the camera.
//!
//! Entering from Normal runs Normal's `OnExit` (its private model takes the
//! live LookAt, distance, yaw and pitch unless its own transfer tween is in
//! flight, and the per-site transfer entry is written).
//!
//! Leaving: `ResetCameraState` changes the game state to Normal, whose entry
//! changes the camera to Normal; `NormalCameraState.OnEnter` takes the
//! bounds and offset back from the setting, then either the inherit branch
//! (a harvest or delivery site, or home after an outdoor site) or
//! `TransferCameraSettings`, whose previous-state switch has no case for
//! state 9: the default path, a 1.0 s tween to Normal's private model.
//! `CleanupCamera` removes the brain; `FieldCameraManager.ResetAll` puts the
//! near and far planes back to the field camera's own.
//!
//! Named difference: Normal's private model is taken here at entry, as the
//! zoom state takes its own copy; the two copies are not shared.

use bevy::prelude::*;

use crate::camera::{
    is_inherit_camera_setting, perspective_fov_deg, to_rotation, wrap180, CameraSetting,
    CameraStateType, CameraTween, FieldCameraModel, FieldCameraState, PrevSiteType,
    TweenCompletion,
};

/// `NormalCameraState.TransferCameraSettings`: `_doTweenCameraTime` is 1.0
/// and the previous-state switch has no case for state 9.
const TRANSFER_SECONDS: f32 = 1.0;

/// The brain's output: the active virtual camera's state.
#[derive(Clone, Debug)]
pub(crate) struct Shot {
    /// The virtual camera's GameObject name.
    pub(crate) name: String,
    pub(crate) eye: Vec3,
    pub(crate) rotation: Quat,
    /// Vertical field of view in degrees.
    pub(crate) fov: f32,
    pub(crate) near: f32,
    pub(crate) far: f32,
}

#[derive(Clone, Copy, Debug)]
struct NormalPrivate {
    look_at: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    fov: f32,
}

/// The state's fields.
#[derive(Resource, Default)]
pub(crate) struct CutSceneCamera {
    prev: Option<CameraStateType>,
    /// The field camera's own planes (`ResetNearClipPlane`/`ResetFarClipPlane`).
    planes: Option<(f32, f32)>,
    normal: Option<NormalPrivate>,
    shot: Option<Shot>,
    last_logged: Option<String>,
}

fn projection(world: &mut World) -> Option<(f32, f32, f32)> {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    match cameras.single(world).ok()? {
        Projection::Perspective(perspective) => Some((
            perspective_fov_deg(&Projection::Perspective(perspective.clone())),
            perspective.near,
            perspective.far,
        )),
        _ => None,
    }
}

/// `FieldCamera.ChangeState(CutScene)`.
pub(crate) fn enter(world: &mut World) {
    let from = world.resource::<FieldCameraState>().0;
    if from == CameraStateType::CutScene {
        return;
    }
    let lens = projection(world);
    let mut normal = None;
    if from == CameraStateType::Normal {
        if let Some(model) = world.get_resource::<FieldCameraModel>().cloned() {
            let transfer_in_flight = world
                .get_resource::<CameraTween>()
                .is_some_and(|tween| !tween.is_normal_reset());
            let fov = world
                .get_resource::<CameraSetting>()
                .map_or(model.fov, |setting| setting.fov);
            if transfer_in_flight {
                info!("[cutscene-camera] Normal.OnExit: its transfer tween is in flight; its private model is kept from the model's current values");
            }
            normal = Some(NormalPrivate {
                look_at: model.look_at,
                distance: model.distance,
                yaw: model.yaw,
                pitch: model.pitch,
                fov,
            });
        }
        let site = world
            .get_resource::<crate::site::SiteActive>()
            .map(|site| site.site_type.clone());
        if let Some(site) = site {
            crate::site_move::camera::record_normal_exit(world, &site);
        }
    }
    world.remove_resource::<CameraTween>();
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::CutScene;
    info!(
        "[cutscene-camera] FieldCamera.ChangeState({from:?} -> CutScene): CutSceneCameraState.OnEnter is empty; the brain drives the camera; lens before fov/near/far {:?}",
        lens
    );
    let mut state = world.resource_mut::<CutSceneCamera>();
    state.prev = Some(from);
    state.planes = lens.map(|(_, near, far)| (near, far));
    state.normal = normal;
    state.shot = None;
    state.last_logged = None;
}

/// The brain's state for this frame (`None` when no shot is active).
pub(crate) fn set_shot(world: &mut World, shot: Option<Shot>) {
    if let Some(mut state) = world.get_resource_mut::<CutSceneCamera>() {
        if shot.is_some() {
            state.shot = shot;
        }
    }
}

/// The camera pose now, for the logs.
pub(crate) fn pose(world: &mut World) -> Option<(Vec3, Vec3, f32)> {
    let mut cameras = world.query_filtered::<(&Transform, &Projection), With<Camera3d>>();
    let (transform, projection) = cameras.single(world).ok()?;
    Some((
        transform.translation,
        *transform.forward(),
        perspective_fov_deg(projection),
    ))
}

/// `ResetCameraState`'s camera half: Normal's entry, the brain removed, the
/// planes reset.
pub(crate) fn exit(world: &mut World) {
    if world.resource::<FieldCameraState>().0 != CameraStateType::CutScene {
        return;
    }
    let (prev, planes, normal) = {
        let mut state = world.resource_mut::<CutSceneCamera>();
        state.shot = None;
        (state.prev.take(), state.planes.take(), state.normal.take())
    };
    if prev != Some(CameraStateType::Normal) {
        warn!("[cutscene-camera] the camera entered CutScene from {prev:?}; GameState Normal's entry still changes it to Normal");
    }
    // CleanupCamera, then FieldCameraManager.ResetAll: the planes.
    if let Some((near, far)) = planes {
        let mut cameras = world.query_filtered::<&mut Projection, With<Camera3d>>();
        if let Ok(mut projection) = cameras.single_mut(world) {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.near = near;
                perspective.far = far;
                perspective.near_clip_plane = Vec4::new(0., 0., -1., -near);
            }
        }
        info!("[cutscene-camera] CleanupCamera: the brain is removed; FieldCameraManager.ResetAll: near {near}, far {far}");
    }
    let site = world.get_resource::<crate::site::SiteActive>().cloned();
    let prev_site = world.resource::<PrevSiteType>().0.clone();
    let inherit = site
        .as_ref()
        .is_some_and(|site| is_inherit_camera_setting(&site.category, &prev_site));
    if inherit {
        let site = site.expect("inherit needs the active site");
        info!(
            "[cutscene-camera] ChangeState(CutScene -> Normal) at {} after {prev_site}: the inherit branch",
            site.site_type
        );
        crate::site_move::camera::enter_normal(world, &site.site_type, &site.category, &prev_site);
        return;
    }
    let setting = world.get_resource::<CameraSetting>().copied();
    let fov_now = projection(world).map(|(fov, _, _)| fov);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    let (Some(setting), Some(private)) = (setting, normal) else {
        warn!("[cutscene-camera] ChangeState(CutScene -> Normal) without its tween: no camera setting or Normal private model");
        return;
    };
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!(
            "[cutscene-camera] ChangeState(CutScene -> Normal) without its tween: no camera model"
        );
        return;
    };
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
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
        "[cutscene-camera] ChangeState(CutScene -> Normal): TransferCameraSettings default path, {TRANSFER_SECONDS}s to Normal's private model: distance {:.2} -> {:.2}, pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, FOV {:.2} -> {:.2}",
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

/// PostUpdate, after the field camera's follow: the brain writes the active
/// shot's pose and lens.
pub(crate) fn advance(
    state: Res<FieldCameraState>,
    mut camera: ResMut<CutSceneCamera>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    if state.0 != CameraStateType::CutScene {
        return;
    }
    let Some(shot) = camera.shot.clone() else {
        return;
    };
    let Ok((mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    *transform = Transform::from_translation(shot.eye).with_rotation(shot.rotation);
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = shot.fov.to_radians();
        perspective.near = shot.near;
        perspective.far = shot.far;
        perspective.near_clip_plane = Vec4::new(0., 0., -1., -shot.near);
    }
    if camera.last_logged.as_deref() != Some(shot.name.as_str()) {
        let forward = *transform.forward();
        info!(
            "[cutscene-camera] CinemachineBrain: live camera {} at ({:.3},{:.3},{:.3}) forward ({:.4},{:.4},{:.4}), lens fov {} near {} far {}",
            shot.name,
            shot.eye.x,
            shot.eye.y,
            shot.eye.z,
            forward.x,
            forward.y,
            forward.z,
            shot.fov,
            shot.near,
            shot.far
        );
        camera.last_logged = Some(shot.name.clone());
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<CutSceneCamera>()
        .add_systems(PostUpdate, advance.after(crate::camera::follow_avatar));
}
