//! The cannon move's two camera states: `PreSiteMoveActionCameraState` (7),
//! entered and left in the same frame with empty hooks, and
//! `SiteMoveActionCameraState` (6), which the move holds until the executor
//! returns the game to Normal. Normal's exit snapshot and its inherit-branch
//! re-entry around the move are here as well.
//!
//! Frame-rate convention (named): the source applies every lerp factor and
//! the 1/120 grounding step once per `OnUpdate` call, and MySekai runs at
//! 60 fps. The product's follow convention (`camera::follow_avatar`) scales a
//! per-frame factor by `deltaTime / 0.016667` and clamps it to [0, 1]; the
//! accumulating factors here use the same scaling, so at 60 fps they are the
//! source values exactly. The `_s` branch's 0.4 is not a rate (the look-at is
//! reset to the root the same frame) and is not scaled.

use bevy::animation::AnimatedBy;
use bevy::prelude::*;

use super::timeline::SourceClip;
use crate::camera::{
    perspective_fov_deg, to_rotation, view_dir, wrap180, CameraSetting, CameraStateType,
    CameraTween, FieldCameraModel, FieldCameraState, NormalCameraMemory, FRAME_BASE,
};
use crate::character::AvatarRoot;

/// `SiteMoveActionCameraState` private state: `_groundedTweenTime` and the
/// avatar's `Step` (rig `Root`) and `Head` bones (`FindDeep` by name on the
/// avatar view).
#[derive(Resource, Debug)]
pub(crate) struct SiteMoveCamera {
    pub(crate) grounded: f32,
    step: Option<Entity>,
    head: Option<Entity>,
    missing_logged: bool,
}

/// Normal's exit snapshots per site (`NormalCameraState._cameraTransferData`,
/// a dictionary keyed by site id). The product's single-slot
/// `NormalCameraMemory` keeps serving the FPS and talk paths; the cannon move
/// writes both. Every Normal exit at a site is followed by the cannon exit
/// that leaves it, so the entry read at a cannon arrival equals the source's.
#[derive(Resource, Default)]
pub(crate) struct CannonCameraTransfer(std::collections::HashMap<String, NormalCameraMemory>);

/// The camera branch `SiteMoveActionCameraState.OnUpdate` takes for the
/// player's current animation name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Branch {
    Enter,
    Launch,
    Loop,
    Landing,
    Other,
}

pub(crate) fn branch(clip: Option<SourceClip>) -> Branch {
    match clip {
        Some(SourceClip::CannonS) => Branch::Enter,
        Some(SourceClip::CannonS2) => Branch::Launch,
        Some(SourceClip::CannonL) => Branch::Loop,
        Some(SourceClip::CannonE | SourceClip::Cannon02E) => Branch::Landing,
        None => Branch::Other,
    }
}

/// The model fields the branches write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Model {
    pub(crate) look_at: Vec3,
    pub(crate) distance: f32,
    pub(crate) pitch: f32,
}

/// One `OnUpdate` in the Normal mode (the Hero mode is unreachable: its field
/// is never written). Returns the look-at `UpdatePosition` places the eye
/// from; the model's final look-at is where the view looks
/// (`transform.LookAt(LookAt + Offset)` is the last call).
///
/// `rate` is the product frame scale `dt / 0.016667` (1 at 60 fps).
pub(crate) fn step(
    model: &mut Model,
    grounded: &mut f32,
    branch: Branch,
    root: Vec3,
    step_bone: Vec3,
    head: Vec3,
    setting_distance: f32,
    rate: f32,
) -> Vec3 {
    let scaled = |factor: f32| (factor * rate).clamp(0.0, 1.0);
    match branch {
        Branch::Enter => {
            model.look_at = root;
            let placed = model.look_at;
            // 0x3ECCCCCD: a blend towards the head, recomputed from the root
            // every frame.
            model.look_at += (head - model.look_at) * 0.4;
            placed
        }
        Branch::Launch | Branch::Loop => {
            model.look_at += (step_bone - model.look_at) * scaled(0.9);
            model.distance += (5.0 - model.distance) * scaled(0.03);
            if branch == Branch::Loop {
                model.pitch += (-10.0 - model.pitch) * scaled(0.01);
            }
            model.look_at
        }
        Branch::Landing => {
            model.look_at += (step_bone - model.look_at) * scaled(0.98);
            // InQuad(g) = g * g, capped at 1.
            let w = (*grounded * *grounded).min(1.0);
            model.pitch += (40.0 - model.pitch) * scaled(w);
            model.distance += (setting_distance - model.distance) * scaled(w);
            if *grounded < 1.0 {
                // 0x3C088889 = 1/120 per call.
                *grounded += f32::from_bits(0x3C08_8889) * rate;
            }
            model.look_at
        }
        Branch::Other => model.look_at,
    }
}

/// Unity's euler Y of the avatar root, from the product rotation: assets are
/// reflected on x, so a product yaw `a` about +Y is the source yaw `-a`.
/// `MakePositive` and the explicit wrap put it in [0, 360).
pub(crate) fn source_yaw(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::Z;
    let product = forward.x.atan2(forward.z).to_degrees();
    let yaw = -product;
    (yaw - (yaw / 360.0).floor() * 360.0).clamp(0.0, 360.0)
}

/// `FieldCamera.ChangeState(PreSiteMoveAction)` from Normal: Normal's
/// `OnExit` writes its transfer snapshot for the site being left; the
/// PreSiteMoveAction hooks are empty.
pub(crate) fn enter_pre_site_move(world: &mut World, site: &str) {
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        warn!("[site-move] camera model not built: Normal exit snapshot skipped");
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::PreSiteMoveAction;
        return;
    };
    let snapshot = NormalCameraMemory {
        site: site.to_owned(),
        look_at: model.look_at,
        distance: model.distance,
        yaw: model.yaw,
        pitch: model.pitch,
        fov: model.fov,
    };
    world.insert_resource(snapshot.clone());
    world
        .get_resource_or_insert_with(CannonCameraTransfer::default)
        .0
        .insert(site.to_owned(), snapshot);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::PreSiteMoveAction;
    info!("[site-move] camera Normal -> PreSiteMoveAction (Normal exit snapshot for {site})");
}

/// `ChangeState(SiteMoveAction)`: `OnEnter` opens the pitch range (0, 360),
/// clears the grounding time and tweens to the avatar root: pitch 10, the
/// root's yaw, the model FOV, distance 8, 0.5 s OutQuad.
pub(crate) fn enter_site_move(world: &mut World) {
    let mut avatars = world.query_filtered::<(&Transform, &GlobalTransform), With<AvatarRoot>>();
    let Ok((local, global)) = avatars.single(world) else {
        warn!("[site-move] no avatar root: SiteMoveAction entered without its tween");
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::SiteMoveAction;
        return;
    };
    let root = global.translation();
    let yaw = source_yaw(local.rotation);
    let fov_now = {
        let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
        cameras.single(world).map(perspective_fov_deg).ok()
    };
    let (step, head) = find_bones(world);
    world.insert_resource(SiteMoveCamera {
        grounded: 0.0,
        step,
        head,
        missing_logged: false,
    });
    let tween = world.get_resource_mut::<FieldCameraModel>().map(|mut model| {
        model.min_pitch = 0.0;
        model.max_pitch = 360.0;
        let prev_pitch = wrap180(model.pitch);
        let prev_yaw = wrap180(model.yaw);
        let tween = CameraTween {
            look_at: (model.look_at, root),
            fov: (fov_now.unwrap_or(model.fov), model.fov),
            pitch: (prev_pitch, to_rotation(prev_pitch, 10.0)),
            yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
            distance: (model.distance, 8.0),
            duration: 0.5,
            elapsed: 0.0,
        };
        info!(
            "[site-move] camera SiteMoveAction OnEnter: tween 0.5s OutQuad to root {root:.2}, pitch {:.1} -> 10, yaw {:.1} -> {:.1}, distance {:.2} -> 8",
            prev_pitch, prev_yaw, tween.yaw.1, model.distance
        );
        tween
    });
    if let Some(tween) = tween {
        world.insert_resource(tween);
    }
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::SiteMoveAction;
}

/// `NormalGameState.OnEnter` -> `ChangeState(Normal)` -> `NormalCameraState.OnEnter`:
/// the bounds and offset come back from Normal's private model (the setting
/// values), then the inherit branch tweens to this site's snapshot, or to the
/// source's fallback (setting `initPitch`, current yaw, FOV and distance).
/// Every cannon arrival takes the inherit branch: the destination is a
/// harvest or delivery site, or home entered from an outdoor site.
pub(crate) fn enter_normal(world: &mut World, site: &str, category: &str, prev_site: &str) {
    world.remove_resource::<SiteMoveCamera>();
    let inherit = crate::camera::is_inherit_camera_setting(category, prev_site);
    let setting = world.get_resource::<CameraSetting>().copied();
    let snapshot = world
        .get_resource::<CannonCameraTransfer>()
        .and_then(|table| table.0.get(site).cloned());
    let fov_now = {
        let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
        cameras.single(world).map(perspective_fov_deg).ok()
    };
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
    let Some(setting) = setting else {
        warn!("[site-move] camera setting missing at Normal re-entry");
        return;
    };
    let Some(mut model) = world.get_resource_mut::<FieldCameraModel>() else {
        warn!("[site-move] camera model missing at Normal re-entry");
        return;
    };
    model.min_distance = setting.min_distance;
    model.max_distance = setting.max_distance;
    model.min_pitch = setting.min_pitch;
    model.max_pitch = setting.max_pitch;
    model.offset = setting.offset;
    if !inherit {
        // TransferCameraSettings is not reachable from a cannon arrival.
        error!("[site-move] Normal re-entry at {site} after {prev_site} is not the inherit branch; TransferCameraSettings is not ported");
        return;
    }
    let (pitch, yaw, fov, distance, hit) = match &snapshot {
        Some(m) => (m.pitch, m.yaw, m.fov, m.distance, true),
        None => (
            setting.init_pitch,
            model.yaw,
            model.fov,
            model.distance,
            false,
        ),
    };
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    let tween = CameraTween {
        look_at: (model.look_at, model.look_at),
        fov: (fov_now.unwrap_or(model.fov), fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
        distance: (model.distance, distance),
        duration: crate::camera::FPS_EXIT_TWEEN_SECS_INHERIT,
        elapsed: 0.0,
    };
    info!(
        "[site-move] camera -> Normal at {site} (inherit, {}): distance {:.2} -> {:.2}, pitch {:.1} -> {:.1}, yaw {:.1} -> {:.1}",
        if hit { "site snapshot" } else { "fallback initPitch + current" },
        model.distance, distance, prev_pitch, tween.pitch.1, prev_yaw, tween.yaw.1
    );
    drop(model);
    world.insert_resource(tween);
}

fn find_bones(world: &mut World) -> (Option<Entity>, Option<Entity>) {
    let animator = {
        let mut drivers = world.query::<&crate::player_avatar::AvatarDriver>();
        drivers.iter(world).next().map(|driver| driver.player)
    };
    let Some(animator) = animator else {
        return (None, None);
    };
    let mut bones = world.query::<(Entity, &Name, &AnimatedBy)>();
    let mut step = None;
    let mut head = None;
    for (entity, name, by) in bones.iter(world) {
        if by.0 != animator {
            continue;
        }
        match name.as_str() {
            "Root" if step.is_none() => step = Some(entity),
            "Head" if head.is_none() => head = Some(entity),
            _ => {}
        }
    }
    (step, head)
}

/// PostUpdate, right after `camera::follow_avatar` (which advanced the entry
/// tween): one `SiteMoveActionCameraState.OnUpdate`.
pub(crate) fn update(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    session: Option<Res<super::SiteMove>>,
    mut camera_state: Option<ResMut<SiteMoveCamera>>,
    model: Option<ResMut<FieldCameraModel>>,
    setting: Option<Res<CameraSetting>>,
    avatars: Query<&GlobalTransform, (With<AvatarRoot>, Without<Camera3d>)>,
    bones: Query<&GlobalTransform, (Without<AvatarRoot>, Without<Camera3d>)>,
    mut cameras: Query<(&mut Transform, &mut GlobalTransform), With<Camera3d>>,
) {
    if state.0 != CameraStateType::SiteMoveAction {
        return;
    }
    let (Some(session), Some(camera_state), Some(mut model), Some(setting)) =
        (session, camera_state.as_deref_mut(), model, setting)
    else {
        return;
    };
    let Ok(root) = avatars.single() else {
        return;
    };
    let root = root.translation();
    let bone = |entity: Option<Entity>| {
        entity
            .and_then(|e| bones.get(e).ok())
            .map(GlobalTransform::translation)
    };
    let (step_bone, head) = match (bone(camera_state.step), bone(camera_state.head)) {
        (Some(step), Some(head)) => (step, head),
        _ => {
            if !camera_state.missing_logged {
                error!("[site-move] avatar Step/Head bones not found: the camera follows the avatar root instead");
                camera_state.missing_logged = true;
            }
            (root, root)
        }
    };
    let rate = time.delta_secs() / FRAME_BASE;
    let mut fields = Model {
        look_at: model.look_at,
        distance: model.distance,
        pitch: model.pitch,
    };
    let placed = step(
        &mut fields,
        &mut camera_state.grounded,
        branch(session.source_clip),
        root,
        step_bone,
        head,
        setting.distance,
        rate,
    );
    model.look_at = fields.look_at;
    model.distance = fields.distance;
    model.pitch = fields.pitch;
    // UpdatePosition, then the view looks at LookAt + Offset.
    let eye = placed + model.offset + view_dir(model.pitch, model.yaw) * model.distance;
    let pivot = model.look_at + model.offset;
    if let Ok((mut transform, mut global)) = cameras.single_mut() {
        *transform = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
        // Written after propagation: publish the global pose too, so every
        // reader this frame (and the render) sees this frame's camera.
        *global = GlobalTransform::from(*transform);
    }
}

#[cfg(test)]
mod tests {
    //! Research instrument: the branch arithmetic recomputed from the
    //! source's factors at the source frame rate (rate 1).
    use super::*;

    #[test]
    fn launch_and_landing_factors() {
        let mut m = Model {
            look_at: Vec3::ZERO,
            distance: 8.0,
            pitch: 10.0,
        };
        let mut g = 0.0;
        step(
            &mut m,
            &mut g,
            Branch::Launch,
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::ZERO,
            6.0,
            1.0,
        );
        assert!((m.look_at.x - 9.0).abs() < 1e-6);
        assert!((m.distance - (8.0 + (5.0 - 8.0) * 0.03)).abs() < 1e-6);
        assert_eq!(m.pitch, 10.0);
        step(
            &mut m,
            &mut g,
            Branch::Loop,
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::ZERO,
            6.0,
            1.0,
        );
        assert!((m.pitch - (10.0 + (-10.0 - 10.0) * 0.01)).abs() < 1e-6);
        // Landing: the first call has g = 0, so pitch and distance hold.
        let before = m;
        step(
            &mut m,
            &mut g,
            Branch::Landing,
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::ZERO,
            6.0,
            1.0,
        );
        assert_eq!(m.pitch, before.pitch);
        assert_eq!(m.distance, before.distance);
        assert_eq!(g, f32::from_bits(0x3C08_8889));
        assert!((g - 1.0 / 120.0).abs() < 1e-7);
        // After 120 calls g reaches 1 and w saturates.
        for _ in 0..200 {
            step(
                &mut m,
                &mut g,
                Branch::Landing,
                Vec3::ZERO,
                Vec3::ZERO,
                Vec3::ZERO,
                6.0,
                1.0,
            );
        }
        assert!(g >= 1.0 && g < 1.0 + 1.0 / 120.0 + 1e-6);
        assert!((m.pitch - 40.0).abs() < 1e-4);
        assert!((m.distance - 6.0).abs() < 1e-4);
    }

    #[test]
    fn enter_branch_places_from_the_root_and_looks_towards_the_head() {
        let mut m = Model {
            look_at: Vec3::splat(99.0),
            distance: 8.0,
            pitch: 10.0,
        };
        let mut g = 0.0;
        let root = Vec3::new(1.0, 0.0, 0.0);
        let head = Vec3::new(1.0, 1.0, 0.0);
        let placed = step(
            &mut m,
            &mut g,
            Branch::Enter,
            root,
            Vec3::ZERO,
            head,
            6.0,
            2.0,
        );
        assert_eq!(placed, root);
        assert!((m.look_at - Vec3::new(1.0, 0.4, 0.0)).length() < 1e-6);
    }

    #[test]
    fn source_yaw_is_the_reflected_product_yaw() {
        assert!((source_yaw(Quat::from_rotation_y(0.0)) - 0.0).abs() < 1e-4);
        // A product turn of +90 degrees about +Y is -90 in the source frame.
        assert!(
            (source_yaw(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)) - 270.0).abs() < 1e-3
        );
        assert!(
            (source_yaw(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)) - 90.0).abs() < 1e-3
        );
    }
}
