//! Camera state 2, `FloorEdit`: the ground layout editor's camera.
//!
//! Construction (`FloorEditCameraState..ctor`): the look pitches are
//! [55, 85] (`RotationXValueList`), the direction starts at 2 (Back), and the
//! state's private model is a copy of the field camera's model with FOV 20,
//! distance 12.14, distance range [4.8, 17], yaw 0 and pitch 55 written over
//! it (`InitializeData`). The state object lives as long as the camera; its
//! private model, direction and look count carry over from one edit session
//! to the next.
//!
//! `OnEnter`: the live model's distance range becomes [4.8, 17]. Coming from
//! Normal (or the site environment editor) the private yaw is the live yaw
//! snapped to a quarter turn (`GetNearestFixedAngle`) and the direction
//! follows from it (`ConvertYawToDirection`); from the wall editor the
//! private yaw is the live yaw as it is. The private LookAt is the live
//! LookAt at the site's height. The camera tweens 0.5 s
//! (`DoTweenCameraSetting`, OutQuad) to the private LookAt, (private pitch,
//! private yaw), FOV 20 and distance 12.14, and 12.14 is kept as the backup
//! distance. The camera is unlocked.
//!
//! `OnUpdate`: the eye is placed from the live model (LookAt + Offset, back
//! along the view direction by the distance) and looks at LookAt, without
//! the offset. That arm is in the field camera's follow system.
//!
//! `OnDrag`: the LookAt moves by `-delta * MoveLookAtRatio` (FloatConfigs 66)
//! along the camera's flattened forward (screen y) and its right (screen x),
//! scaled by distance / setting distance, then is clamped to the floor
//! grid's bounds. `OnPinch`: the distance changes by
//! `AddDistanceRatio * pinch * -3` (FloatConfigs 65), clamped to the live
//! range. Duo drag and the camera reset do nothing.
//!
//! Layout events. A focus (`FocusOnLayoutEdit`, sent when a placed fixture is
//! picked and when a new one is put) tweens 0.5 s to the fixture's position
//! with the private rotation, FOV and distance. With zoom support (the
//! fixture is not a block) the live distance is kept as the backup first,
//! and for footprints up to 4 cells the private distance becomes
//! `clamp(4.8 + 1.5 * max(size.x, size.z), 4.8, 17)`; without zoom support
//! the private distance takes the live one. A decide (`LayoutAction` 1)
//! restores the backup distance when the decided fixture is not a block (or
//! is gone) and the live distance is below the backup: 0.5 s to the live
//! LookAt with the private rotation and FOV. The camera rotate button
//! (`LayoutAction` 13) turns to the next direction (Back -> Right -> Front ->
//! Left -> Back, yaw 0 -> 90 -> 180 -> -90 -> 0) and the change-look button
//! (14) toggles the pitch between 55 and 85; both tween only the rotation,
//! 0.5 s OutQuad (`ROTATE_CAMERA_DURATION`). `OnExit` only removes the
//! event registrations.
//!
//! Leaving: the edit game state's exit hands the camera to Normal
//! (`NormalGameState.OnEnter`); Normal's `TransferCameraSettings` takes the
//! live yaw into its private model for a previous state 2 and tweens 1.0 s
//! to that model, unless its inherit branch applies (see
//! `zoom_player_camera::normal_enter`).
//!
//! Named differences:
//! - The camera lock (`FieldCamera` lock flag): the floor editor locks the
//!   camera while a fixture is dragged. Here a drag with a selected fixture
//!   moves the fixture (the edit pointer's rule), so the camera drag runs
//!   only without a selection.
//! - Pinch comes from the mouse wheel, the product's pinch input.
//! - The focus completion callback (`onFocusCompleted`) and the tutorial
//!   focus (`FocusOnTutorialLayoutEdit`) are not ported.
//! - Entering from a state other than Normal is refused: the first-person
//!   and other states' exits are not reachable from here; the editor runs
//!   with the camera where it is.
//! - The private model lives for the app run (the source's lives as long as
//!   its field camera object).

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::camera::{
    perspective_fov_deg, to_rotation, wrap180, CameraSetting, CameraStateType, CameraTween,
    FieldCameraModel, FieldCameraState, TweenCompletion,
};
use crate::gesture::{GestureEvent, GestureKind, GestureState};

/// `InitializeData`: minimum and maximum distance, FOV and distance.
const MIN_DISTANCE: f32 = 4.8;
const MAX_DISTANCE: f32 = 17.0;
const FOV: f32 = 20.0;
const DISTANCE: f32 = 12.14;
/// `RotationXValueList`; `InitializeData`'s pitch is its first element.
const PITCHES: [f32; 2] = [55.0, 85.0];
/// `InitializeData`'s yaw.
const INIT_YAW: f32 = 0.0;
/// The constructor's direction (Back).
const INIT_DIRECTION: u8 = 2;
/// `OnEnter`, the focus and the distance restore pass this duration.
const TWEEN_SECONDS: f32 = 0.5;
/// `SiteLayoutUtility.ROTATE_CAMERA_DURATION`.
const ROTATE_SECONDS: f32 = 0.5;
/// `SetZoomSupportIfNeeded`: footprints wider than this keep the distance.
const ZOOM_MAX_CELLS: f32 = 4.0;
/// `SetZoomSupportIfNeeded`: distance per footprint cell.
const ZOOM_PER_CELL: f32 = 1.5;
/// `OnPinch`'s factor on the pinch delta.
const PINCH_FACTOR: f32 = -3.0;
/// `MysekaiConstants.TILE_SCALE`.
const TILE_SCALE: f32 = moly_law::objective::TILE_SCALE;

/// The state object: its private model and its own fields.
#[derive(Resource, Debug, Clone)]
pub(crate) struct FloorEditCamera {
    look_at: Vec3,
    pitch: f32,
    yaw: f32,
    fov: f32,
    distance: f32,
    min_distance: f32,
    max_distance: f32,
    /// `_direction` (Front 0, Left 1, Back 2, Right 3).
    direction: u8,
    /// `_lookCount`, the index into [`PITCHES`].
    look_count: usize,
    /// `_backupDistance`.
    backup_distance: f32,
}

impl Default for FloorEditCamera {
    fn default() -> Self {
        Self {
            look_at: Vec3::ZERO,
            pitch: PITCHES[0],
            yaw: INIT_YAW,
            fov: FOV,
            distance: DISTANCE,
            min_distance: MIN_DISTANCE,
            max_distance: MAX_DISTANCE,
            direction: INIT_DIRECTION,
            look_count: 0,
            backup_distance: 0.0,
        }
    }
}

/// `GetNearestFixedAngle`: the angle folded into [0, 360] (`Mathf.Repeat`),
/// then below 45 -> 0, [45, 135] -> 90, (135, 225] -> 180, (225, 315) -> 270,
/// from 315 -> 360.
pub(crate) fn nearest_fixed_angle(angle: f32) -> f32 {
    let folded = (angle - (angle / 360.0).floor() * 360.0).clamp(0.0, 360.0);
    if folded >= 315.0 {
        360.0
    } else if folded < 45.0 {
        0.0
    } else if folded <= 135.0 {
        90.0
    } else if folded <= 225.0 {
        180.0
    } else {
        270.0
    }
}

/// `ConvertYawToDirection`: `((int)yaw / 90) % 4` (truncating, remainder
/// keeps the sign) maps 1 -> Right, 2 -> Front, 3 -> Left, anything else ->
/// Back.
pub(crate) fn yaw_direction(yaw: f32) -> u8 {
    match ((yaw as i32) / 90) % 4 {
        1 => 3,
        2 => 0,
        3 => 1,
        _ => 2,
    }
}

/// `GetNextDirection` and `GetRotationY` (table [180, -90, 0, 90] by
/// direction): the direction and yaw the rotate button turns to.
pub(crate) fn next_direction(direction: u8) -> (u8, f32) {
    match direction {
        0 => (1, -90.0),
        2 => (3, 90.0),
        3 => (0, 180.0),
        _ => (2, 0.0),
    }
}

/// `SetZoomSupportIfNeeded`'s distance for a footprint.
pub(crate) fn focus_distance(size: Vec3, private: f32, min: f32, max: f32) -> f32 {
    let cells = size.x.max(size.z);
    if cells > ZOOM_MAX_CELLS {
        return private;
    }
    let distance = min + ZOOM_PER_CELL * cells;
    if distance < min {
        min
    } else {
        distance.min(max)
    }
}

fn camera_fov(world: &mut World) -> Option<f32> {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    cameras.single(world).ok().map(perspective_fov_deg)
}

fn instrument() -> bool {
    std::env::var("MOLY_EDIT_AUTOPLAY").is_ok()
}

/// `DoTweenCameraSetting` from the live model and the camera body's FOV.
fn tween_to(
    model: &FieldCameraModel,
    fov_now: f32,
    look_at: Vec3,
    pitch: f32,
    yaw: f32,
    fov: f32,
    distance: f32,
    duration: f32,
) -> CameraTween {
    let prev_pitch = wrap180(model.pitch);
    let prev_yaw = wrap180(model.yaw);
    CameraTween {
        look_at: (model.look_at, look_at),
        fov: (fov_now, fov),
        pitch: (prev_pitch, to_rotation(prev_pitch, pitch)),
        yaw: (prev_yaw, to_rotation(prev_yaw, yaw)),
        distance: (model.distance, distance),
        duration,
        elapsed: 0.0,
        on_complete: TweenCompletion::None,
    }
}

fn in_state(world: &World) -> bool {
    world
        .get_resource::<FieldCameraState>()
        .is_some_and(|state| state.0 == CameraStateType::FloorEdit)
}

/// `FieldCamera.ChangeState(FloorEdit)` from the edit game state's entry:
/// the current state's exit (Normal's), then this state's `OnEnter`.
pub(crate) fn enter(world: &mut World) -> Result<(), String> {
    let Some(previous) = world
        .get_resource::<FieldCameraState>()
        .map(|state| state.0)
    else {
        return Err("the field camera has no state".into());
    };
    if previous == CameraStateType::FloorEdit {
        return Ok(());
    }
    if previous != CameraStateType::Normal {
        return Err(format!(
            "the camera is in {previous:?}; only Normal's exit is reachable from the floor edit camera"
        ));
    }
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        return Err("the field camera model is not built yet".into());
    };
    let fov_now = camera_fov(world).unwrap_or(model.fov);
    crate::zoom_player_camera::normal_exit(world, &model, "edit-camera");
    let site_y = crate::fixture_edit::site_origin(world).y;
    let mut private = world
        .remove_resource::<FloorEditCamera>()
        .unwrap_or_default();
    // Normal and the site environment editor snap; the wall editor copies.
    match previous {
        CameraStateType::Normal | CameraStateType::SiteEnvironmentEdit => {
            private.yaw = nearest_fixed_angle(model.yaw);
            private.direction = yaw_direction(private.yaw);
        }
        CameraStateType::WallEdit => {
            private.yaw = model.yaw;
            private.direction = yaw_direction(private.yaw);
        }
        _ => {}
    }
    private.look_at = Vec3::new(model.look_at.x, site_y, model.look_at.z);
    let tween = tween_to(
        &model,
        fov_now,
        private.look_at,
        private.pitch,
        private.yaw,
        private.fov,
        DISTANCE,
        TWEEN_SECONDS,
    );
    private.backup_distance = DISTANCE;
    info!(
        "[edit-camera] ChangeState({previous:?} -> FloorEdit): distance range [{MIN_DISTANCE}, {MAX_DISTANCE}]; live yaw {:.2} -> private yaw {:.2} (direction {}); LookAt {:.3} -> {:.3}; tween {TWEEN_SECONDS}s OutQuad pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}, FOV {:.2} -> {:.2}, distance {:.3} -> {DISTANCE}; backup distance {DISTANCE}",
        model.yaw,
        private.yaw,
        private.direction,
        model.look_at,
        private.look_at,
        tween.pitch.0,
        tween.pitch.1,
        tween.yaw.0,
        tween.yaw.1,
        tween.fov.0,
        tween.fov.1,
        model.distance
    );
    if let Some(mut live) = world.get_resource_mut::<FieldCameraModel>() {
        live.min_distance = private.min_distance;
        live.max_distance = private.max_distance;
    }
    world.insert_resource(tween);
    world.insert_resource(private);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::FloorEdit;
    Ok(())
}

/// The edit game state's exit: `FieldCamera.ChangeState(Normal)`. This
/// state's `OnExit` only removes its event registrations.
pub(crate) fn leave(world: &mut World) {
    if !in_state(world) {
        return;
    }
    info!("[edit-camera] FloorEdit.OnExit (event registrations removed); Normal's entry follows");
    crate::zoom_player_camera::normal_enter(world, CameraStateType::FloorEdit, "edit-camera");
}

/// A site change while editing: the site loader rebuilds the camera for the
/// new site; the state only returns to Normal.
pub(crate) fn leave_for_site_change(world: &mut World) {
    if !in_state(world) {
        return;
    }
    info!("[edit-camera] FloorEdit left for a site change: the new site frames the camera");
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
}

/// `FocusOnLayoutEdit` (event 36): `SetZoomSupportIfNeeded`, then 0.5 s to
/// the focus position with the private rotation, FOV and distance.
pub(crate) fn focus(
    world: &mut World,
    position: Vec3,
    size: Vec3,
    zoom_support: bool,
    source: &str,
) {
    if !in_state(world) {
        return;
    }
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        return;
    };
    let fov_now = camera_fov(world).unwrap_or(model.fov);
    let Some(mut private) = world.get_resource::<FloorEditCamera>().cloned() else {
        return;
    };
    let before = private.distance;
    if zoom_support {
        private.backup_distance = model.distance;
        private.distance = focus_distance(
            size,
            private.distance,
            private.min_distance,
            private.max_distance,
        );
    } else {
        private.distance = model.distance;
    }
    let tween = tween_to(
        &model,
        fov_now,
        position,
        private.pitch,
        private.yaw,
        private.fov,
        private.distance,
        TWEEN_SECONDS,
    );
    info!(
        "[edit-camera] FocusOnLayoutEdit({source}) position {position:.3} size ({:.0}, {:.0}, {:.0}) zoom support {zoom_support}: backup distance {:.3}, private distance {before:.3} -> {:.3}; tween {TWEEN_SECONDS}s to pitch {:.2}, yaw {:.2}, FOV {:.2}, distance {:.3}",
        size.x,
        size.y,
        size.z,
        private.backup_distance,
        private.distance,
        tween.pitch.1,
        tween.yaw.1,
        tween.fov.1,
        tween.distance.1
    );
    world.insert_resource(tween);
    world.insert_resource(private);
}

/// `LayoutAction` 1 (a decide): `CanRestoreCameraDistance` (no fixture, or
/// not a block) then `RestoreCameraDistance`.
pub(crate) fn decided(world: &mut World, is_block: Option<bool>, source: &str) {
    if !in_state(world) {
        return;
    }
    if is_block == Some(true) {
        info!("[edit-camera] LayoutAction 1 ({source}): the decided fixture is a block; the distance is not restored");
        return;
    }
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        return;
    };
    let fov_now = camera_fov(world).unwrap_or(model.fov);
    let Some(mut private) = world.get_resource::<FloorEditCamera>().cloned() else {
        return;
    };
    if model.distance >= private.backup_distance {
        info!(
            "[edit-camera] LayoutAction 1 ({source}): live distance {:.3} is not below the backup {:.3}; nothing restored",
            model.distance, private.backup_distance
        );
        return;
    }
    private.distance = private.backup_distance;
    let tween = tween_to(
        &model,
        fov_now,
        model.look_at,
        private.pitch,
        private.yaw,
        private.fov,
        private.backup_distance,
        TWEEN_SECONDS,
    );
    info!(
        "[edit-camera] LayoutAction 1 ({source}): RestoreCameraDistance {:.3} -> {:.3} over {TWEEN_SECONDS}s at LookAt {:.3}",
        model.distance, private.backup_distance, model.look_at
    );
    world.insert_resource(tween);
    world.insert_resource(private);
}

/// `DoTweenRotate`: the running tween is killed and the rotation alone
/// tweens, OutQuad.
fn rotate_to(world: &mut World, pitch: f32, yaw: f32, what: &str) {
    let Some(model) = world.get_resource::<FieldCameraModel>().cloned() else {
        return;
    };
    let fov_now = camera_fov(world).unwrap_or(model.fov);
    let tween = tween_to(
        &model,
        fov_now,
        model.look_at,
        pitch,
        yaw,
        fov_now,
        model.distance,
        ROTATE_SECONDS,
    );
    info!(
        "[edit-camera] {what}: DoTweenRotate {ROTATE_SECONDS}s pitch {:.2} -> {:.2}, yaw {:.2} -> {:.2}",
        tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1
    );
    world.insert_resource(tween);
}

/// `LayoutAction` 13 (the camera rotate button, event 5).
pub(crate) fn rotate(world: &mut World) {
    if !in_state(world) {
        return;
    }
    let Some(mut private) = world.get_resource::<FloorEditCamera>().cloned() else {
        return;
    };
    let from = private.direction;
    let (direction, yaw) = next_direction(from);
    private.direction = direction;
    private.yaw = yaw;
    let (pitch, yaw) = (private.pitch, private.yaw);
    world.insert_resource(private);
    rotate_to(
        world,
        pitch,
        yaw,
        &format!("OnRotateCamera direction {from} -> {direction}"),
    );
}

/// `LayoutAction` 14 (the change-look button).
pub(crate) fn change_look(world: &mut World) {
    if !in_state(world) {
        return;
    }
    let Some(mut private) = world.get_resource::<FloorEditCamera>().cloned() else {
        return;
    };
    private.look_count = (private.look_count + 1) % PITCHES.len();
    private.pitch = PITCHES[private.look_count];
    let (pitch, yaw, count) = (private.pitch, private.yaw, private.look_count);
    world.insert_resource(private);
    rotate_to(
        world,
        pitch,
        yaw,
        &format!("OnChangeLookCamera look {count}"),
    );
}

/// The floor grid's bounds for the LookAt (`MysekaiSiteModel.ClampToFloorBounds`):
/// the site position plus the grid bound's corners times `TILE_SCALE`, in
/// the product frame (x mirrored, so the x range's ends swap).
fn floor_bounds(origin: Vec3, width: i32, depth: i32) -> (Vec2, Vec2) {
    let half_x = ((width + 1) / 2) as f32;
    let half_z = ((depth + 1) / 2) as f32;
    (
        Vec2::new(
            origin.x - (half_x - 1.0) * TILE_SCALE,
            origin.z - half_z * TILE_SCALE,
        ),
        Vec2::new(
            origin.x + half_x * TILE_SCALE,
            origin.z + (half_z - 1.0) * TILE_SCALE,
        ),
    )
}

/// Update: `OnDrag` and `OnPinch` while the floor edit camera is current.
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut gestures: MessageReader<GestureEvent>,
    scroll: Res<AccumulatedMouseScroll>,
    state: Res<FieldCameraState>,
    view: Res<crate::fixture_edit::EditView>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    setting: Option<Res<CameraSetting>>,
    placements: Option<Res<crate::fixture::FixturePlacements>>,
    model: Option<ResMut<FieldCameraModel>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    origins: Query<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    ui: crate::ui_layout::PointerUi,
) {
    let mut drag = Vec2::ZERO;
    for event in gestures.read() {
        if event.kind == GestureKind::Drag && event.state == GestureState::Moved && !event.ui_owned
        {
            drag += event.delta;
        }
    }
    if state.0 != CameraStateType::FloorEdit || !view.active || view.exit_dialog {
        return;
    }
    let (Some(mut model), Some(configs), Some(setting)) = (model, configs, setting) else {
        return;
    };
    // A drag with a selected fixture moves the fixture (see the module's
    // named differences).
    if drag != Vec2::ZERO && view.selected.is_none() {
        let Ok(camera) = cameras.single() else {
            return;
        };
        // Host screen y grows down; the source's grows up.
        let delta = -Vec2::new(drag.x, -drag.y)
            * configs.float(crate::client_config::KEY_FIELD_CAMERA_MOVE_LOOK_AT_RATIO);
        let forward = camera.forward();
        let flat = Vec3::new(forward.x, 0.0, forward.z);
        let flat = if flat.length() > 1e-5 {
            flat / flat.length()
        } else {
            Vec3::ZERO
        };
        let right = camera.right();
        let scale = model.distance / setting.distance;
        let step = (flat * delta.y + *right * delta.x) * scale;
        let before = model.look_at;
        model.look_at += step;
        let origin = origins
            .iter()
            .next()
            .map_or(Vec3::ZERO, GlobalTransform::translation);
        if let Some(floor) = placements.as_deref().and_then(|p| p.floor_grid()) {
            let (min, max) = floor_bounds(origin, floor.width, floor.depth);
            model.look_at.x = model.look_at.x.clamp(min.x, max.x);
            model.look_at.z = model.look_at.z.clamp(min.y, max.y);
        }
        if instrument() {
            info!(
                "[edit-camera] OnDrag delta ({:.1}, {:.1}): LookAt {before:.3} -> {:.3}",
                drag.x, drag.y, model.look_at
            );
        }
    }
    let mut pinch = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y * crate::camera::WHEEL_PINCH_PIXELS,
        MouseScrollUnit::Pixel => scroll.delta.y,
    };
    if pinch != 0.0 {
        let over_ui = windows.single().ok().is_some_and(|window| {
            window
                .cursor_position()
                .is_some_and(|cursor| ui.captures(cursor, window))
        });
        if over_ui {
            pinch = 0.0;
        }
    }
    if pinch != 0.0 {
        let ratio = configs.float(crate::client_config::KEY_FIELD_CAMERA_ADD_DISTANCE_RATIO);
        let before = model.distance;
        let raw = model.distance + ratio * pinch * PINCH_FACTOR;
        model.distance = if raw < model.min_distance {
            model.min_distance
        } else {
            raw.min(model.max_distance)
        };
        info!(
            "[edit-camera] OnPinch {pinch:.1}: distance {before:.3} -> {:.3} (range [{:.2}, {:.2}])",
            model.distance, model.min_distance, model.max_distance
        );
    }
}

/// Instrument (`MOLY_EDIT_AUTOPLAY`): the live camera across the edit
/// camera's tweens and after the return to Normal.
#[derive(Default)]
pub(crate) struct Sampler {
    last_state: Option<CameraStateType>,
    since: f32,
    last: Option<f32>,
    after_leave: Option<f32>,
}

pub(crate) fn sample(
    time: Res<Time>,
    state: Res<FieldCameraState>,
    model: Option<Res<FieldCameraModel>>,
    tween: Option<Res<CameraTween>>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    mut run: Local<Sampler>,
) {
    if !instrument() {
        return;
    }
    let (Some(model), Ok((eye, projection))) = (model, cameras.single()) else {
        return;
    };
    let now = time.elapsed_secs();
    if run.last_state != Some(state.0) {
        if run.last_state == Some(CameraStateType::FloorEdit) {
            run.after_leave = Some(now);
        }
        run.last_state = Some(state.0);
        run.since = now;
        run.last = None;
    }
    let editing = state.0 == CameraStateType::FloorEdit;
    let returning = run.after_leave.is_some_and(|at| now - at <= 2.0);
    if !editing && !returning {
        return;
    }
    // Every 0.1 s while a tween runs (and right after leaving), else 1 s.
    let period = if tween.is_some() || returning {
        0.1
    } else {
        1.0
    };
    if run.last.is_some_and(|last| now - last < period) {
        return;
    }
    run.last = Some(now);
    info!(
        "[edit-camera] sample {:?} t {:.2}s: pitch {:.3} yaw {:.3} FOV {:.3} distance {:.3} range [{:.2}, {:.2}] LookAt {:.3} eye {:.3}{}",
        state.0,
        now - run.since,
        model.pitch,
        model.yaw,
        perspective_fov_deg(projection),
        model.distance,
        model.min_distance,
        model.max_distance,
        model.look_at,
        eye.translation,
        match tween.as_deref() {
            Some(tween) => format!(" tween {:.2}/{:.2}s", tween.elapsed, tween.duration),
            None => String::new(),
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GetNearestFixedAngle`'s comparisons: `>= 315`, `< 45`, `<= 135`,
    /// `<= 225`, else 270, on the folded angle.
    #[test]
    fn yaw_snaps_to_the_source_quarter_turns() {
        for (yaw, snapped) in [
            (0.0, 0.0),
            (44.9, 0.0),
            (45.0, 90.0),
            (135.0, 90.0),
            (135.1, 180.0),
            (225.0, 180.0),
            (225.1, 270.0),
            (314.9, 270.0),
            (315.0, 360.0),
            (-60.0, 270.0),
            (-10.0, 360.0),
            (400.0, 0.0),
        ] {
            assert_eq!(nearest_fixed_angle(yaw), snapped, "yaw {yaw}");
        }
    }

    /// `ConvertYawToDirection` and the rotate button's table agree: each
    /// direction's yaw converts back to that direction.
    #[test]
    fn rotate_table_and_yaw_conversion_agree() {
        for direction in 0..4u8 {
            let (next, yaw) = next_direction(direction);
            let yaw = if yaw < 0.0 { yaw + 360.0 } else { yaw };
            assert_eq!(yaw_direction(yaw), next, "direction {direction}");
        }
        assert_eq!(yaw_direction(360.0), 2);
    }

    /// `SetZoomSupportIfNeeded`: `4.8 + 1.5 * max(x, z)` clamped to
    /// [4.8, 17] for footprints up to 4; larger ones keep the distance.
    #[test]
    fn focus_zoom_follows_the_footprint() {
        assert_eq!(
            focus_distance(Vec3::new(1.0, 3.0, 2.0), 12.14, 4.8, 17.0),
            4.8 + 3.0
        );
        assert_eq!(
            focus_distance(Vec3::new(4.0, 1.0, 4.0), 12.14, 4.8, 17.0),
            4.8 + 6.0
        );
        assert_eq!(
            focus_distance(Vec3::new(5.0, 1.0, 1.0), 12.14, 4.8, 17.0),
            12.14
        );
    }
}
