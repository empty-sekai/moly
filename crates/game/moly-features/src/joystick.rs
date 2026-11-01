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

/// 底盘 RectTransform 边长（canvas 单位；序列化 sizeDelta 256×256）。
const BASE_SIZE: f32 = 256.0;

/// The EventSystem's serialized `m_DragThreshold`, in device pixels: the
/// input module sends OnDrag only once the pointer is this far from where it
/// was pressed (`(press - position).sqrMagnitude >= threshold²`).
const DRAG_THRESHOLD_DEVICE_PIXELS: f32 = 10.0;

/// 摇杆两件所在页（UI atlas 的 MysekaiAtlas 页）。
const PAGE: &str =
    "moly://ui/atlas/textures/sactx-0-1024x1024-ASTC 4x4-MysekaiAtlas-0a6c3f39-000001d9.png";

/// 底盘源矩形（页图像坐标、y 从顶量）：sprite textureRect (0,0,272,272)
/// 底原点 → 顶原点翻成 min=(0, 1024-272)。
const BASE_SRC: Rect = Rect {
    min: Vec2::new(0.0, 752.0),
    max: Vec2::new(272.0, 1024.0),
};

/// 手柄源矩形：sprite textureRect (416,568,112,112) → 顶原点
/// min=(416, 1024-568-112)。
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

/// Update（手势链内、手势层之前）：摇杆层推进。读 `TouchInput` 消息流
/// ——与手势层同一条流，两层按同一谓词各自分流（见模块注释）。
pub fn advance(
    mut state: ResMut<JoystickState>,
    mut touches: MessageReader<TouchInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    edits: Res<crate::field_modes::EditSessionActive>,
    player_talk: Option<Res<crate::player_talk::PlayerTalkSession>>,
    overlays: crate::game_settings::OverlayPanels,
    library: Res<crate::content_library::ContentLibrary>,
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
    // 让位门（GameStateType 分派）：Edit=2 / Talk=3（玩家自己的对话）
    // → 禁用。环境配对对话不改游戏态（SomeCharacterTalk=7 不在摇杆的
    // 分派表里），不算禁用源。
    // MysekaiUtility.EnableJoyStick(true) runs in the entry's Finish.
    let was_enabled = state.enabled;
    state.enabled = !photo.is_some_and(|held|held.0) && !tutorial.is_some_and(|input|input.movement_blocked)
        && !edits.is_active()
        && player_talk.is_none()
        && !overlays.block_world_input()
        && !library.blocks_exploration_input()
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


/// 摇杆 UI 根（两件 sprite 的父；可见性随捕获态，子件沿父链继承）。
#[derive(Component)]
pub struct JoystickUiRoot;

/// 底盘件。
#[derive(Component)]
pub struct JoystickBase;

/// 手柄件。
#[derive(Component)]
pub struct JoystickHandle;

/// 纹源：UI atlas 整页。
#[derive(Resource)]
pub struct JoystickArt {
    page: Handle<Image>,
}

/// Startup：请求整页贴图（两件共用一页，一次装载）。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let page = moly_assets::residency::load_image(&server, AssetPath::from(PAGE.to_owned()));
    commands.insert_resource(JoystickArt { page });
}

/// Update：页到齐铺两件（隐藏起步，捕获时才显）。子件按源 rect 裁、
/// custom_size = RectTransform 尺寸（canvas 单位），place_ui 每帧压
/// canvas 缩放。
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

/// Update（advance 之后）：按捕获态摆件。捕获中：底盘心 = 触点
/// （屏点 → 覆盖相机世界系：原点屏幕中心、y 向上、1 单位 = 1 逻辑
/// 像素）；手柄心 = 底盘心 + 方向 × 128 × 缩放（方向本就是 y 向上
/// 语义，与该世界系一致，无需再翻）；两件 custom_size 压当帧 canvas
/// 缩放。未捕获：整树隐藏。
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

/// 冒烟剧本的推进账（[`smoke_autojoystick`] 的 Local）。
#[derive(Default)]
pub struct AutojoystickRun {
    /// 剧本步号：0 未开始，1 周期一拖动中，2 周期一保持，3 周期一收场，
    /// 4 周期二拖动中，5 周期二保持，6 周期二收场，7 完结。
    step: usize,
}

/// Update（手势链首、joystick::advance 之前）：摇杆层的冒烟口
/// （`dev.joystick_autosmoke_secs`，`MOLY_JOYSTICK_AUTOSMOKE_SECS`）。往
/// `TouchInput` 消息流注入剧本触摸——引擎触摸事件就落在同一条流上，注入即走
/// 摇杆层与手势层
/// 的真实分流，不绕过任何一行。剧本两周期（起手均在区内同一点）：
///
/// | 周期 | 时刻 | 段 | 摇杆方向 | 预期位移 |
/// |---|---|---|---|---|
/// | 一 | 1.0s 起手，1.6s 满偏，1.8s 松开 | 拖向右 128×缩放 px | (1, 0) | 相机右 × 步速 |
/// | 二 | 2.4s 起手，3.0s 满偏，3.2s 松开 | 拖向上 128×缩放 px | (0, 1) | 相机前（水平归一）× 步速 |
///
/// 段界记玩家位置快照（含方向与时刻），配合玩家域的输入开始/结束行，
/// 「摇杆方向 → 相机基 → 世界位移」逐段可对账。满偏拖动是线性插值
/// （时刻决定位置，与帧率无关）。
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
    // 时刻表：起手 → 拖到满偏 → 保持 → 松开，两周期。
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
    // 起手点：区内中部（15% 宽、75% 高——顶原点坐标的左下象限）。
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
    // 拖动位置线性插值：t 从段起到段末，s∈[0,1] 走满偏距离。
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
