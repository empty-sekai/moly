//! Joystick domain: the virtual touch joystick, the touch surface of the
//! game's floating `CustomJoyStick`.
//!
//! Input model (the serialized values agree across the three joystick
//! instances):
//! * **Touch zone**: the lower-left 35% width × 50% height of the window
//!   (the serialized `layerSize` of the touch layer's RectTransform). A
//!   **touch** that starts inside the zone belongs to the stick; one that
//!   starts outside belongs to the gesture layer (`gesture`). The game
//!   splits these with UI raycasts (the stick's CanvasGroup `blocksRaycasts`
//!   stops the ray), and this code reproduces that split by evaluating the
//!   same predicate in each layer. **The mouse never enters the stick**: the
//!   game is touch-only, so the host's desktop mouse keeps the full-screen
//!   gesture surface (the existing split for camera drags is unchanged by
//!   this domain).
//! * **Direction**: the first press captures that finger and the base spawns
//!   at the touch point (floating style); direction = (touch − base centre)
//!   mapped to game screen coordinates (y up) divided by the base radius,
//!   radius = 128 canvas units × canvas scale (half the base RectTransform's
//!   256×256). Dead zone 0 (only an exact 0 is caught), limit 1.0 (over the
//!   limit is normalized × 1), the middle keeps its raw ratio, so the result
//!   is the ratio clamped to unit length. `handleRange` 1.0, both axes
//!   bidirectional, snap off.
//! * **Event flow**: press → the base class's OnPointerDown calls OnDrag once
//!   first, sending UPDATE (the base spawns at the touch point, the
//!   difference is exactly zero, so the direction is zero), then sends BEGAN
//!   (the presenter ignores BEGAN), so **the press makes the input active
//!   with a zero move vector**: the player enters the Move state, faces
//!   atan2(0,0)=0, and plays the walk pose in place at the lower rate bound
//!   0.55; afterwards the engine input module sends OnDrag only once the
//!   drag starts — the drag starts once the squared distance from the press
//!   point ≥ the event system's `m_DragThreshold`² (serialized 10, device
//!   pixels; the same value in the prefab and the scene, with no runtime
//!   writer); after that each move → UPDATE (direction + camera basis baked
//!   into the move vector); release/system cancel → END (active cleared). On
//!   the press itself the game round-trips through the canvas camera in
//!   floating point, yielding zero or tiny noise; this layer skips that trip
//!   and uses exact zero (named: the noise is not reproduced). The game
//!   presenter's UPDATE formula (OnTouchJoyStick):
//!   move vector = camera forward (y removed, normalized; when the
//!   horizontal length is near zero **that term** falls back to zero and the
//!   right term is kept) × direction.y + camera right × direction.x; the sum
//!   is not normalized. The basis is simulated from the camera at UPDATE
//!   time — if the camera turns while the finger is held still, the move
//!   vector does not follow (the game behaves the same way).
//! * **Yield gate** (the game's `OnChangeGameState` dispatch on
//!   GameStateType, read from the enum): Edit=2 and Talk=3 → disabled with
//!   ResetState (a finger mid-drag ends with END); LearnSiteEnvironment=6 is
//!   disabled the same way (the harvest learn-phenomenon state);
//!   CutScene=5/SiteMove=10/LevelUpMyRoomSite=11/Delivery=12 are disabled
//!   the same way but this repository has no matching state (tracked as
//!   pending);
//!   **SomeCharacterTalk=7 is not in the stick's dispatch table** —
//!   environment-paired dialogue (NPC×NPC / NPC×fixture) does not change
//!   the game state, so the stick stays available; Harvest=4 and
//!   PhotoShot=8 likewise stay available (PhotoShot hides the UI). In this
//!   repository: placement editing (`EditSessionActive`) and the player's own
//!   dialogue session (`PlayerTalkSession`) → disabled; environment-paired
//!   dialogue (the talk domain's session resource) **does not count**. While
//!   disabled the touch zone belongs to the gesture layer (the game takes the
//!   stick object out of the layer entirely, so rays pass to the full-screen
//!   gesture layer).
//! * **Presentation**: while capturing, the base (256 canvas units) shows at
//!   the touch point and the handle (128) offsets along the direction =
//!   direction × 128 × scale; both hide on release. The two sprites are cut
//!   from the whole UI atlas page by their sprite rects (the rects are
//!   bottom-origin coordinates; the consumer flips them to top-origin).
//!
//! Multi-touch: **only the first finger is followed**. While one is
//! captured, a second finger pressed inside the zone is swallowed (the game's
//! OnPointerDown returns early on a repeated capture and the ray also belongs
//! to the stick, so the gesture layer never sees it); a second finger outside
//! the zone enters the gesture layer as usual (dragging the stick and
//! dragging the camera with an outside finger coexist, the normal touch
//! interaction). Single pointer is the gesture layer's boundary (the game's
//! multi-finger family DUO/PINCH is not carried over), and this layer does
//! not carry over a second finger **inside** the zone either.
//!
//! Smoke hook (`dev.joystick_autosmoke_secs`, `MOLY_JOYSTICK_AUTOSMOKE_SECS`):
//! injects scripted touches into the `TouchInput` message stream — the message
//! surface is the real input surface (engine touch events land on the same
//! stream), so an
//! injection goes through the stick itself. The script has two cycles (drag
//! right to full deflection, drag up to full deflection, each with
//! start/full-deflection/end instants); a player position snapshot is taken
//! at each segment boundary, so the displacement direction can be reconciled
//! per segment.

use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::player::PlayerControlled;
use bevy::asset::{AssetPath, LoadState};
use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub use crate::joystick_law::{bake, bake_move_vector, handle_input, DEAD_ZONE, HANDLE_SIZE, LIMIT};

/// Touch zone fraction (the serialized `layerSize` (0.35, 0.5) of the touch
/// layer's RectTransform).
const ZONE_W: f32 = 0.35;
const ZONE_H: f32 = 0.5;

/// Side of the base's RectTransform, in canvas units (serialized sizeDelta
/// 256×256).
const BASE_SIZE: f32 = 256.0;

/// The EventSystem's serialized `m_DragThreshold`, in device pixels: the
/// input module sends OnDrag only once the pointer is this far from where it
/// was pressed (`(press - position).sqrMagnitude >= threshold²`).
const DRAG_THRESHOLD_DEVICE_PIXELS: f32 = 10.0;

/// The UI atlas page that holds both stick pieces (the MysekaiAtlas page).
const PAGE: &str =
    "moly://ui/atlas/textures/sactx-0-1024x1024-ASTC 4x4-MysekaiAtlas-0a6c3f39-000001d9.png";

/// The base's source rectangle in page image coordinates, y measured from the
/// top: sprite textureRect (0,0,272,272) with a bottom origin becomes
/// min=(0, 1024-272).
const BASE_SRC: Rect = Rect {
    min: Vec2::new(0.0, 752.0),
    max: Vec2::new(272.0, 1024.0),
};

/// The handle's source rectangle: sprite textureRect (416,568,112,112) with a
/// top origin is min=(416, 1024-568-112).
const STICK_SRC: Rect = Rect {
    min: Vec2::new(416.0, 344.0),
    max: Vec2::new(528.0, 456.0),
};

pub use crate::markers::JoystickState;
use crate::markers::in_zone;

/// MysekaiUtility.ForceResetJoyStick -> CustomJoyStick.ForcePointerUp: the
/// captured finger is let go (the stick hides and publishes END), and that
/// finger no longer drives the stick until it is lifted and pressed again.
#[derive(Message, Clone, Copy, Debug)]
pub struct ForceResetJoystick {
    pub reason: &'static str,
}

/// Update, after the action buttons' click: ForcePointerUp for each reset
/// request. END is published whether or not a finger was captured.
pub fn force_reset(
    mut state: ResMut<JoystickState>,
    mut resets: MessageReader<ForceResetJoystick>,
) {
    for reset in resets.read() {
        let finger = state.captured.take();
        let was_active = state.active;
        state.active = false;
        state.dragging = false;
        state.direction = Vec2::ZERO;
        state.move_vector = Vec3::ZERO;
        info!(
            "[joystick] ForcePointerUp ({}): END; finger {finger:?} released (input was {})",
            reset.reason,
            if was_active { "on" } else { "off" }
        );
    }
}

/// Update, in the gesture chain before the gesture layer: advances the stick
/// layer. It reads the `TouchInput` message stream, the same stream the
/// gesture layer reads; both layers split touches by the same predicate (see
/// the module docs).
pub fn advance(
    mut state: ResMut<JoystickState>,
    mut touches: MessageReader<TouchInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    edits: Res<crate::field_modes::EditSessionActive>,
    player_talk: Option<Res<crate::player_talk::PlayerTalkSession>>,
    overlays: crate::game_settings::OverlayPanels,
    claims: Res<crate::input_claims::InputClaims>,
    mut last_logged: Local<Vec2>,
    mut no_area: Local<bool>,
    site_move: Option<Res<crate::field_modes::SiteMoveActive>>,
    entry: Option<Res<crate::entry::EntrySequence>>,
    root_canvas: Option<Res<RootCanvas>>,
    learn: Option<Res<crate::field_modes::LearnSiteEnvironmentActive>>,
    held: Option<Res<crate::field_modes::HeldGameState>>,
    gates: (Option<Res<crate::ui_layers::ScreenManager>>, Option<Res<crate::tutorial::TutorialInput>>, Option<Res<crate::camera::PhotoGestureHeld>>),
) {
    let (screens, tutorial, photo) = gates;
    let Some(window) = windows.single().ok() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    // The stick yields to the game states Edit (2) and Talk (3, the player's
    // own conversation). An ambient pair conversation does not change the
    // game state (SomeCharacterTalk, 7, is not in the stick's table), so it
    // does not disable the stick.
    // MysekaiUtility.EnableJoyStick(true) runs in the entry's Finish.
    let was_enabled = state.enabled;
    state.enabled = !photo.is_some_and(|held|held.0) && !tutorial.is_some_and(|input|input.movement_blocked)
        && !edits.is_active()
        && player_talk.is_none()
        && !overlays.block_world_input()
        && !claims.blocks(crate::input_claims::InputChannel::Exploration)
        // GameState SiteMove: EnableGestureLayer(false).
        && site_move.is_none()
        // GameState LearnSiteEnvironment: the stick resets and hides.
        && learn.is_none()
        // GameState CutScene (5) and LevelUpMyRoomSite (11): the stick's
        // table disables it.
        && held.is_none()
        && crate::entry::control_open(entry.as_deref());
    if let Some(held) = held.as_ref() {
        if held.is_changed() {
            info!(
                "[joystick] OnChangeGameState({} {:?}): disabled",
                held.0 as i32, held.0
            );
        }
    }
    if was_enabled != state.enabled {
        info!(
            "[joystick] enabled {was_enabled} -> {} (held game state {:?})",
            state.enabled,
            held.as_ref().map(|held| held.0)
        );
    }
    if !state.enabled {
        touches.clear();
        if state.captured.take().is_some() {
            // State-change close-out in the game: END is published (ResetStateByCurrentPos sends one END-state event in the same step and the presenter clears _isJoyStickInput).
            state.active = false;
            state.dragging = false;
            state.direction = Vec2::ZERO;
            state.move_vector = Vec3::ZERO;
            info!("[joystick] 让位门落下，拖动中的指以 END 收场");
        }
        return;
    }
    // The joystick is a HUD control of the root canvas; before that canvas
    // exists there is no joystick to take a touch.
    let Some(root_canvas) = root_canvas.as_deref() else {
        touches.clear();
        return;
    };
    let radius = HANDLE_SIZE * root_canvas.scale(window);
    // A window with no drawable area has no canvas scale, so the stick has no
    // radius to measure a ratio against and every ratio would be NaN; the
    // stick is released as a lifted finger is. The refusal is logged once per
    // entry into the state, not once per frame of it.
    if !(radius.is_finite() && radius > 0.0) {
        touches.clear();
        if state.captured.take().is_some() {
            state.active = false;
            state.dragging = false;
            state.direction = Vec2::ZERO;
            state.move_vector = Vec3::ZERO;
        }
        if !*no_area {
            *no_area = true;
            info!(
                "[joystick] refused: the window has no drawable area (size {width}x{height}); the stick is released"
            );
        }
        return;
    }
    *no_area = false;
    // Touch positions are logical pixels; the threshold is in device pixels.
    let drag_threshold = DRAG_THRESHOLD_DEVICE_PIXELS / window.scale_factor();
    // A dialog over the field (ExistsDialog(Layer_Dialog)) takes a new press;
    // a finger already on the stick keeps dragging it.
    let dialog = screens
        .as_deref()
        .is_some_and(crate::ui_layers::ScreenManager::exists_dialog);
    for touch in touches.read() {
        match touch.phase {
            TouchPhase::Started => {
                if state.captured.is_none() && dialog && in_zone(touch.position, width, height) {
                    // A press under a dialog belongs to the dialog layer, as
                    // the field buttons' taps do (ExistsDialog(Layer_Dialog)).
                    // The gesture layer skips the same press while the stick
                    // is enabled, so no layer takes it.
                    info!(
                        "[joystick] press {} @({:.0},{:.0}) refused: a dialog is shown on Layer_Dialog",
                        touch.id, touch.position.x, touch.position.y
                    );
                    continue;
                }
                if state.captured.is_none() && in_zone(touch.position, width, height) {
                    // First-finger capture: the base spawns at the touch point
                    // (floating style). While one finger is captured, a second
                    // inside the zone is swallowed (the game returns early and
                    // the ray belongs to the stick); a finger outside the zone
                    // does not belong to this layer.
                    state.captured = Some(touch.id);
                    state.base = touch.position;
                    state.dragging = false;
                    state.direction = Vec2::ZERO;
                    state.active = false;
                    *last_logged = Vec2::ZERO;
                    // OnPointerDown -> OnDrag: UPDATE with the press-point
                    // difference, zero, so the input is on with a zero
                    // vector (then BEGAN, which the presenter ignores).
                    if let Some(vector) = bake_move_vector(state.direction, &cameras) {
                        state.move_vector = vector;
                        state.active = true;
                    }
                    info!(
                        "[joystick] 捕获指 {} @({:.0},{:.0})，底盘半径 {:.0}px（死区 {} 上限 {}）；按下即 UPDATE 零向量，输入 {}，拖动阈值 {:.2}px",
                        touch.id, touch.position.x, touch.position.y, radius, DEAD_ZONE, LIMIT,
                        if state.active { "开" } else { "未开（相机不在场）" },
                        drag_threshold
                    );
                }
            }
            TouchPhase::Moved => {
                if state.captured == Some(touch.id) {
                    if !state.dragging {
                        // ProcessDrag: the drag starts once the pointer is
                        // at least the threshold from the press point.
                        if (state.base - touch.position).length_squared()
                            < drag_threshold * drag_threshold
                        {
                            continue;
                        }
                        state.dragging = true;
                        info!(
                            "[joystick] 拖动开始：距按下点 {:.2}px ≥ 阈值 {:.2}px",
                            (state.base - touch.position).length(),
                            drag_threshold
                        );
                    }
                    // OnDrag: the difference is mapped to game screen
                    // coordinates (y up) and divided by the base radius, then
                    // clamped by HandleInput (dead zone 0 catches only an exact
                    // 0; over the limit is normalized × 1; the middle keeps its
                    // raw ratio).
                    let raw = Vec2::new(
                        touch.position.x - state.base.x,
                        state.base.y - touch.position.y,
                    ) / radius;
                    state.direction = handle_input(raw);
                    // The presenter's UPDATE: bake the move vector and set the
                    // input active. With no camera present (touch during
                    // loading) the game short-circuits the whole formula — the
                    // input is not set active and the next UPDATE bakes again.
                    if let Some(vector) = bake_move_vector(state.direction, &cameras) {
                        state.move_vector = vector;
                        state.active = true;
                    }
                    if (state.direction - *last_logged).length() > 0.2 {
                        *last_logged = state.direction;
                        debug!(
                            "[joystick] 方向 ({:.2},{:.2}) → 世界向量 ({:.2},{:.2},{:.2})",
                            state.direction.x,
                            state.direction.y,
                            state.move_vector.x,
                            state.move_vector.y,
                            state.move_vector.z
                        );
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if state.captured == Some(touch.id) {
                    // A system cancel closes out the same way as a normal
                    // release (the game's gesture layer only handles
                    // OnPointerUp; if a finger taken by the system were left
                    // tracked, the surface would freeze, so the touch-product
                    // release path treats it as a lift).
                    state.captured = None;
                    state.active = false;
                    state.dragging = false;
                    state.direction = Vec2::ZERO;
                    state.move_vector = Vec3::ZERO;
                    info!("[joystick] 指收场：END，方向清零，输入停止");
                }
            }
        }
    }
}


/// The stick's UI root: parent of the two sprites. Its visibility follows the
/// capture state and the pieces inherit it.
#[derive(Component)]
pub struct JoystickUiRoot;

/// The base piece.
#[derive(Component)]
pub struct JoystickBase;

/// The handle piece.
#[derive(Component)]
pub struct JoystickHandle;

/// The texture: the whole UI atlas page.
#[derive(Resource)]
pub struct JoystickArt {
    page: Handle<Image>,
}

/// Startup: requests the page texture once; both pieces share it.
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let page = moly_assets::residency::load_image(&server, AssetPath::from(PAGE.to_owned()));
    commands.insert_resource(JoystickArt { page });
}

/// Update: once the page is loaded, spawns the two pieces, hidden until a
/// finger is captured. Each piece is cut by its source rect with
/// `custom_size` set to its RectTransform size in canvas units; `place_ui`
/// applies the canvas scale every frame.
pub fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    art: Option<Res<JoystickArt>>,
    roots: Query<(), With<JoystickUiRoot>>,
) {
    if !roots.is_empty() {
        return;
    }
    let Some(art) = art else { return };
    match server.load_state(&art.page) {
        LoadState::Loaded => {}
        LoadState::Failed(err) => panic!("摇杆贴图装载失败（{err:?}）"),
        _ => return,
    }
    let base = commands
        .spawn((
            JoystickBase,
            Sprite {
                image: art.page.clone(),
                rect: Some(BASE_SRC),
                custom_size: Some(Vec2::splat(BASE_SIZE)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.2),
        ))
        .id();
    let handle = commands
        .spawn((
            JoystickHandle,
            Sprite {
                image: art.page.clone(),
                rect: Some(STICK_SRC),
                custom_size: Some(Vec2::splat(HANDLE_SIZE)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.3),
        ))
        .id();
    commands
        .spawn((
            JoystickUiRoot,
            Visibility::Hidden,
            Transform::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .add_children(&[base, handle]);
    info!("[joystick] 摇杆件铺装：底盘 256 手柄 128 canvas 单位，默认隐藏");
}

/// Update, after `advance`: places the pieces by the capture state. While a
/// finger is captured, the base centre is the touch point, mapped from the
/// screen into the overlay camera's world (origin at the screen centre, y up,
/// 1 unit = 1 logical pixel), and the handle centre is the base centre plus
/// direction × 128 × scale (the direction is already y-up, so it needs no
/// flip). Both pieces' `custom_size` takes this frame's canvas scale. With no
/// finger captured the whole tree is hidden.
pub fn place_ui(
    state: Res<JoystickState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut roots: Query<&mut Visibility, With<JoystickUiRoot>>,
    mut bases: Query<(&mut Transform, &mut Sprite), (With<JoystickBase>, Without<JoystickHandle>)>,
    mut handles: Query<
        (&mut Transform, &mut Sprite),
        (With<JoystickHandle>, Without<JoystickBase>),
    >,
    root_canvas: Option<Res<RootCanvas>>,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let captured = state.captured.is_some();
    for mut root in &mut roots {
        set_if_neq!(*root, if captured { Visibility::Visible } else { Visibility::Hidden });
    }
    if !captured {
        return;
    }
    let (width, height) = (window.width(), window.height());
    let scale = root_canvas.scale(window);
    let base_world = Vec2::new(state.base.x - width / 2.0, height / 2.0 - state.base.y);
    if let Ok((mut transform, mut sprite)) = bases.single_mut() {
        set_if_neq!(transform.translation, base_world.extend(0.2));
        sprite.custom_size = Some(Vec2::splat(BASE_SIZE * scale));
    }
    if let Ok((mut transform, mut sprite)) = handles.single_mut() {
        let handle_world = base_world + state.direction * HANDLE_SIZE * scale;
        set_if_neq!(transform.translation, handle_world.extend(0.3));
        sprite.custom_size = Some(Vec2::splat(HANDLE_SIZE * scale));
    }
}

/// Progress of the smoke script (the `Local` of [`smoke_autojoystick`]).
#[derive(Default)]
pub struct AutojoystickRun {
    /// Script step: 0 not started, 1 cycle one dragging, 2 cycle one holding,
    /// 3 cycle one released, 4 cycle two dragging, 5 cycle two holding,
    /// 6 cycle two released, 7 done.
    step: usize,
}

/// Update, at the head of the gesture chain before `joystick::advance`: the
/// stick layer's smoke hook (`dev.joystick_autosmoke_secs`,
/// `MOLY_JOYSTICK_AUTOSMOKE_SECS`). It writes scripted touches into the
/// `TouchInput` message stream, where engine touch events land too, so the
/// touches take the same split between the stick layer and the gesture layer.
/// The script runs two cycles, both pressed at the same point in the zone:
///
/// | Cycle | Time | Drag | Stick direction | Expected motion |
/// |---|---|---|---|---|
/// | 1 | press 1.0s, full deflection 1.6s, release 1.8s | right, 128 × scale px | (1, 0) | camera right × step speed |
/// | 2 | press 2.4s, full deflection 3.0s, release 3.2s | up, 128 × scale px | (0, 1) | camera forward (flattened) × step speed |
///
/// Each segment boundary logs a player position snapshot with the direction
/// and time; with the player's input start and end lines, each segment can be
/// checked from stick direction to camera basis to world motion. The drag to
/// full deflection is a linear interpolation, so the position depends on the
/// time and not on the frame rate.
pub fn smoke_autojoystick(
    mut touches: MessageWriter<TouchInput>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    players: Query<&Transform, With<PlayerControlled>>,
    time: Res<Time>,
    mut run: Local<AutojoystickRun>,
    root_canvas: Option<Res<RootCanvas>>,
) {
    let armed = env_secs(&moly_config::keys::DEV_JOYSTICK_AUTOSMOKE_SECS);
    if armed <= 0.0 || time.elapsed_secs() >= armed {
        return;
    }
    let now = time.elapsed_secs();
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    const FINGER: u64 = 99001;
    // Timetable for both cycles: press, drag to full deflection, hold, release.
    const C1_PRESS: f32 = 1.0;
    const C1_FULL: f32 = 1.6;
    const C1_RELEASE: f32 = 1.8;
    const C2_PRESS: f32 = 2.4;
    const C2_FULL: f32 = 3.0;
    const C2_RELEASE: f32 = 3.2;
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let radius = HANDLE_SIZE * root_canvas.scale(window);
    // Press point inside the zone: 15% of the width, 75% of the height (the
    // lower-left quadrant in top-origin coordinates).
    let start = Vec2::new(width * 0.15, height * 0.75);
    let write = |touches: &mut MessageWriter<TouchInput>, phase: TouchPhase, position: Vec2| {
        touches.write(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: FINGER,
        });
    };
    let snapshot = |label: &str, dir: &str| {
        if let Ok(transform) = players.single() {
            let p = transform.translation;
            info!(
                "[joystick-smoke] {label} t={:.2} 方向 {dir} 玩家位置 ({:.2},{:.2},{:.2})",
                now, p.x, p.y, p.z
            );
        }
    };
    // Drag position: linear in time from the segment's start to its end, s in
    // [0, 1] covering the full deflection distance.
    let progress = |t0: f32, t1: f32| ((now - t0) / (t1 - t0)).clamp(0.0, 1.0);
    match run.step {
        0 if now >= C1_PRESS => {
            write(&mut touches, TouchPhase::Started, start);
            info!(
                "[joystick-smoke] 周期一起手 @({:.0},{:.0})（区 = x<{:.0} 且 y≥{:.0}，满偏半径 {:.0}px）",
                start.x, start.y, width * ZONE_W, height * ZONE_H, radius
            );
            snapshot("周期一起手", "(0,0)");
            run.step = 1;
        }
        1 => {
            let s = progress(C1_PRESS, C1_FULL);
            write(
                &mut touches,
                TouchPhase::Moved,
                start + Vec2::new(radius * s, 0.0),
            );
            if now >= C1_FULL {
                snapshot("周期一满偏（向右）", "(1,0)");
                run.step = 2;
            }
        }
        2 if now >= C1_RELEASE => {
            write(
                &mut touches,
                TouchPhase::Ended,
                start + Vec2::new(radius, 0.0),
            );
            snapshot("周期一收场", "(0,0)");
            run.step = 3;
        }
        3 if now >= C2_PRESS => {
            write(&mut touches, TouchPhase::Started, start);
            snapshot("周期二起手", "(0,0)");
            run.step = 4;
        }
        4 => {
            let s = progress(C2_PRESS, C2_FULL);
            write(
                &mut touches,
                TouchPhase::Moved,
                start + Vec2::new(0.0, -radius * s),
            );
            if now >= C2_FULL {
                snapshot("周期二满偏（向上）", "(0,1)");
                run.step = 5;
            }
        }
        5 if now >= C2_RELEASE => {
            write(
                &mut touches,
                TouchPhase::Ended,
                start + Vec2::new(0.0, -radius),
            );
            snapshot("周期二收场", "(0,0)");
            run.step = 6;
        }
        _ => {}
    }
}

/// The key's seconds (default 0): the read the smoke hooks share.
fn env_secs(key: &'static moly_config::Key) -> f32 {
    moly_config::text(key)
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
