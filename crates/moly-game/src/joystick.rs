//! 摇杆域：虚拟摇杆——真源 CustomJoyStick（浮动摇杆）的触摸面移植。
//!
//! 输入模型（方法体直读，非推断；序列化值 APK player data 三实例一致）：
//! * **触摸区**：窗口左下 35% 宽 × 50% 高（触摸层 RectTransform 的
//!   layerSize 序列化值）。区内起手的**触摸**归摇杆；区外起手的归
//!   手势层（`gesture`）。真源里这层分派由 UI 射线完成（摇杆区的
//!   CanvasGroup blocksRaycasts 挡住射线），此处用同一谓词在两层各
//!   自复现。**鼠标不进摇杆**——真源是纯触屏产品，宿主桌面鼠标的
//!   手势面保持全屏（相机拖拽的既有分配不因本域改变）。
//! * **方向**：首指按下捕获该指、底盘生于触点（浮动式）；方向 =
//!   (触点 − 底盘心) 翻成真源屏坐标（y 向上）后 / 底盘半径，半径 =
//!   128 canvas 单位 × canvas 缩放（底盘 RectTransform 256×256 的
//!   半边）。死区 0（只捕精确 0）、上限 1.0（超限归一 × 1）、中段
//!   原始比例 ⇒ 实效 = 把比值夹到单位长。handleRange 1.0、轴双向、
//!   snap 关。
//! * **事件流**：按下 → BEGAN（按下帧差值恰为零，方向零，本层无
//!   动作）；每次拖动 → UPDATE（方向 + 相机基烘移动向量）；松开/
//!   系统取消 → END（清活跃）。真源 presenter 的 UPDATE 式
//!   （OnTouchJoyStick）：移动向量 = 相机前（去 y 归一，水平模长
//!   近零时**该项**回零、右项保留）× 方向.y + 相机右 × 方向.x，
//!   和向量不归一。烘基取 UPDATE 时刻的相机——按住不动时相机转
//!   动，移动向量不随动（真源如此，照抄）。
//! * **让位门**（真源 OnChangeGameState 的 GameStateType 分派，枚举
//!   直读）：Edit=2 与 Talk=3 → 禁用并 ResetState（正在拖的指以 END
//!   收场）；CutScene=5/LearnSiteEnvironment=6/SiteMove=10/
//!   LevelUpMyRoomSite=11/Delivery=12 同禁但本仓无对应态（挂账）；
//!   **SomeCharacterTalk=7 不在摇杆的分派表里**——环境配对对话
//!   （NPC×NPC / NPC×家具）不改游戏态，摇杆照常可用；Harvest=4 与
//!   PhotoShot=8 反而保持可用（PhotoShot 隐 UI）。本仓对应：摆放编辑
//!   （EditSessionActive）与玩家自己的对话会话（PlayerTalkSession）
//!   → 禁用；环境配对对话（talk 域的会话资源）**不算**。禁用期间
//!   触摸区归手势层（真源禁用时摇杆件整个下线，射线穿到全屏手势层）。
//! * **呈现**：捕获中底盘（256 canvas 单位）显于触点、手柄（128）
//!   随方向偏移 = 方向 × 128 × 缩放；松开隐藏。两件贴图按 UI atlas
//!   的 sprite rect 从整页裁（rect 是底原点坐标，消费侧翻成顶原点）。
//!
//! 多指：**只跟首指**。已捕获时区内第二根指按下被吞掉（真源
//! OnPointerDown 对重复捕获早退，射线也归摇杆件——手势层收不到）；
//! 区外的第二根指照常进手势层（摇杆拖动 + 区外指拖相机并存，
//! 这是触屏产品的常态操作形）。单指针是手势层的移植边界（真源
//! 多指族 DUO/PINCH 不迁），本层对第二根**区内**指同样不迁。
//!
//! 冒烟口（`MOLY_JOYSTICK_AUTOSMOKE_SECS`）：往 `TouchInput` 消息流
//! 注入剧本触摸——消息面就是真实输入面（引擎触摸事件落在同一条流
//! 上），注入即走摇杆层本体。剧本两周期（右拖满偏移、上拖满偏移，
//! 各含起手/满偏/收场三时刻），段界记玩家位置快照，位移方向逐段
//! 可对账。

use crate::balloon::{canvas_scale, BALLOON_LAYER};
use crate::player::PlayerControlled;
use bevy::asset::{AssetPath, LoadState};
use bevy::camera::visibility::RenderLayers;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// 触摸区占比（真源触摸层 RectTransform 的 layerSize 序列化值
/// (0.35, 0.5)）。
const ZONE_W: f32 = 0.35;
const ZONE_H: f32 = 0.5;

/// 底盘 RectTransform 边长（canvas 单位；序列化 sizeDelta 256×256）。
const BASE_SIZE: f32 = 256.0;

/// 手柄 RectTransform 边长（canvas 单位；序列化 sizeDelta 128×128）。
/// 也是方向的归一分母（OnDrag 式的分母 = 底盘半边 × handleRange ×
/// canvas 缩放 = 128 × 1.0 × 缩放）。
pub(crate) const HANDLE_SIZE: f32 = 128.0;

/// 死区（序列化 deadZone 0——只捕精确 0）。
const DEAD_ZONE: f32 = 0.0;

/// 方向上限（序列化 Limit 1.0：超限归一 × 1，中段保持原始差值）。
const LIMIT: f32 = 1.0;

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

/// 摇杆区谓词：窗口左下 35% 宽 × 50% 高。Bevy 屏坐标顶原点、y 向下
/// ⇒ x < 0.35W 且 y ≥ 0.5H。手势层读同一谓词复现射线分流。
pub(crate) fn in_zone(position: Vec2, width: f32, height: f32) -> bool {
    position.x < width * ZONE_W && position.y >= height * ZONE_H
}

/// 摇杆层状态：CustomJoyStick 字段面的移植子集 + presenter 侧的
/// 活跃沿。单实例。
#[derive(Resource, Default)]
pub(crate) struct JoystickState {
    /// 本帧让位门求值结果（编辑/对话在播 → false）。手势层读它复现
    /// 区内吞没。
    pub(crate) enabled: bool,
    /// 捕获中的手指 id（真源 _handlingPointerId 的 Nullable<int>）。
    /// 手势层读它跳过捕获指的事件。
    pub(crate) captured: Option<u64>,
    /// 底盘心（屏坐标、顶原点——捕获时等于触点）。
    base: Vec2,
    /// 最近一次 UPDATE 的方向（真源屏坐标语义：y 向上；已按上限夹）。
    direction: Vec2,
    /// presenter 的 _isJoyStickInput：收到过 UPDATE 且未收 END。
    pub(crate) active: bool,
    /// UPDATE 时刻烘好的世界移动向量（真源 _joyStickMoveDirection）。
    pub(crate) move_vector: Vec3,
}

/// Update（手势链内、手势层之前）：摇杆层推进。读 `TouchInput` 消息流
/// ——与手势层同一条流，两层按同一谓词各自分流（见模块注释）。
pub(crate) fn advance(
    mut state: ResMut<JoystickState>,
    mut touches: MessageReader<TouchInput>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    edits: Res<crate::fixture_edit::EditSessionActive>,
    player_talk: Option<Res<crate::player_talk::PlayerTalkSession>>,
    settings_panel: Res<crate::game_settings::SettingsPanel>,
    mut last_logged: Local<Vec2>,
) {
    let Some(window) = windows.single().ok() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    // 让位门（GameStateType 分派）：Edit=2 / Talk=3（玩家自己的对话）
    // → 禁用。环境配对对话不改游戏态（SomeCharacterTalk=7 不在摇杆的
    // 分派表里），不算禁用源。
    state.enabled = !edits.is_active() && player_talk.is_none()
        && !settings_panel.blocks_world_input();
    if !state.enabled {
        touches.clear();
        if state.captured.take().is_some() {
            // 真源状态切换收场：发布 END（ResetStateByCurrentPos 同拍发
            // 一条 END 态事件，presenter 清 _isJoyStickInput）。
            state.active = false;
            state.direction = Vec2::ZERO;
            state.move_vector = Vec3::ZERO;
            info!("[joystick] 让位门落下，拖动中的指以 END 收场");
        }
        return;
    }
    let radius = HANDLE_SIZE * canvas_scale(width, height);
    for touch in touches.read() {
        match touch.phase {
            TouchPhase::Started => {
                if state.captured.is_none() && in_zone(touch.position, width, height) {
                    // 首指捕获：底盘生于触点，方向零（真源基类按下即虚调
                    // 一次拖动，差值恰为零）。已捕获时区内第二指吞掉
                    //（真源早退 + 射线归摇杆件）；区外指不归本层。
                    state.captured = Some(touch.id);
                    state.base = touch.position;
                    state.direction = Vec2::ZERO;
                    state.active = false;
                    *last_logged = Vec2::ZERO;
                    info!(
                        "[joystick] 捕获指 {} @({:.0},{:.0})，底盘半径 {:.0}px（死区 {} 上限 {}）",
                        touch.id, touch.position.x, touch.position.y, radius, DEAD_ZONE, LIMIT
                    );
                }
            }
            TouchPhase::Moved => {
                if state.captured == Some(touch.id) {
                    // OnDrag：差值翻成真源屏坐标（y 向上）除以底盘半径，
                    // 再过 HandleInput 的夹（死区 0 只捕精确 0；超上限
                    // 归一 × 1；中段保持原始比例）。
                    let raw = Vec2::new(
                        touch.position.x - state.base.x,
                        state.base.y - touch.position.y,
                    ) / radius;
                    let magnitude = raw.length();
                    state.direction = if magnitude <= DEAD_ZONE {
                        Vec2::ZERO
                    } else if magnitude > LIMIT {
                        raw / magnitude * LIMIT
                    } else {
                        raw
                    };
                    // presenter 的 UPDATE：烘移动向量并置活跃。相机不在
                    // 场（装载期触摸）真源整式短路——不置活跃，等下一
                    // 次 UPDATE 重烘。
                    match bake_move_vector(state.direction, &cameras) {
                        Some(vector) => {
                            state.move_vector = vector;
                            state.active = true;
                        }
                        None => {}
                    }
                    if (state.direction - *last_logged).length() > 0.2 {
                        *last_logged = state.direction;
                        info!(
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
                    // 系统取消与正常松开同式收场（真源手势层只认
                    // OnPointerUp；指被系统收走后跟踪面若不收会僵住，
                    // 触屏产品的让位路径按松开处理）。
                    state.captured = None;
                    state.active = false;
                    state.direction = Vec2::ZERO;
                    state.move_vector = Vec3::ZERO;
                    info!("[joystick] 指收场：END，方向清零，输入停止");
                }
            }
        }
    }
}

/// presenter 的 UPDATE 烘基式（OnTouchJoyStick）：移动向量 =
/// 相机前（去 y 归一；水平模长近零时**该项**回零，右项保留）×
/// 方向.y + 相机右 × 方向.x，和向量不归一。相机不在场 → None
/// （真源整式短路，不置活跃）。
fn bake_move_vector(
    direction: Vec2,
    cameras: &Query<&GlobalTransform, With<Camera3d>>,
) -> Option<Vec3> {
    let global = cameras.single().ok()?;
    let forward = global.forward();
    let horizontal = (forward.x * forward.x + forward.z * forward.z).sqrt();
    let forward_flat = if horizontal <= 0.00001 {
        Vec3::ZERO
    } else {
        Vec3::new(forward.x / horizontal, 0.0, forward.z / horizontal)
    };
    Some(forward_flat * direction.y + global.right().as_vec3() * direction.x)
}

/// 摇杆 UI 根（两件 sprite 的父；可见性随捕获态，子件沿父链继承）。
#[derive(Component)]
pub(crate) struct JoystickUiRoot;

/// 底盘件。
#[derive(Component)]
pub(crate) struct JoystickBase;

/// 手柄件。
#[derive(Component)]
pub(crate) struct JoystickHandle;

/// 纹源：UI atlas 整页。
#[derive(Resource)]
pub(crate) struct JoystickArt {
    page: Handle<Image>,
}

/// Startup：请求整页贴图（两件共用一页，一次装载）。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    let page = server.load::<Image>(AssetPath::from(PAGE.to_owned()));
    commands.insert_resource(JoystickArt { page });
}

/// Update：页到齐铺两件（隐藏起步，捕获时才显）。子件按源 rect 裁、
/// custom_size = RectTransform 尺寸（canvas 单位），place_ui 每帧压
/// canvas 缩放。
pub(crate) fn spawn_when_ready(
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
pub(crate) fn place_ui(
    state: Res<JoystickState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut roots: Query<&mut Visibility, With<JoystickUiRoot>>,
    mut bases: Query<(&mut Transform, &mut Sprite), (With<JoystickBase>, Without<JoystickHandle>)>,
    mut handles: Query<(&mut Transform, &mut Sprite), (With<JoystickHandle>, Without<JoystickBase>)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let captured = state.captured.is_some();
    for mut root in &mut roots {
        *root = if captured {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !captured {
        return;
    }
    let (width, height) = (window.width(), window.height());
    let scale = canvas_scale(width, height);
    let base_world = Vec2::new(state.base.x - width / 2.0, height / 2.0 - state.base.y);
    if let Ok((mut transform, mut sprite)) = bases.single_mut() {
        transform.translation = base_world.extend(0.2);
        sprite.custom_size = Some(Vec2::splat(BASE_SIZE * scale));
    }
    if let Ok((mut transform, mut sprite)) = handles.single_mut() {
        let handle_world = base_world + state.direction * HANDLE_SIZE * scale;
        transform.translation = handle_world.extend(0.3);
        sprite.custom_size = Some(Vec2::splat(HANDLE_SIZE * scale));
    }
}

/// 冒烟剧本的推进账（[`smoke_autojoystick`] 的 Local）。
#[derive(Default)]
pub(crate) struct AutojoystickRun {
    /// 剧本步号：0 未开始，1 周期一拖动中，2 周期一保持，3 周期一收场，
    /// 4 周期二拖动中，5 周期二保持，6 周期二收场，7 完结。
    step: usize,
}

/// Update（手势链首、joystick::advance 之前）：摇杆层的冒烟口
/// （`MOLY_JOYSTICK_AUTOSMOKE_SECS`）。往 `TouchInput` 消息流注入
/// 剧本触摸——引擎触摸事件就落在同一条流上，注入即走摇杆层与手势层
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
pub(crate) fn smoke_autojoystick(
    mut touches: MessageWriter<TouchInput>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    players: Query<&Transform, With<PlayerControlled>>,
    time: Res<Time>,
    mut run: Local<AutojoystickRun>,
) {
    let armed = env_secs("MOLY_JOYSTICK_AUTOSMOKE_SECS");
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
    let radius = HANDLE_SIZE * canvas_scale(width, height);
    // 起手点：区内中部（15% 宽、75% 高——顶原点坐标的左下象限）。
    let start = Vec2::new(width * 0.15, height * 0.75);
    let write = |touches: &mut MessageWriter<TouchInput>,
                 phase: TouchPhase,
                 position: Vec2| {
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
            write(&mut touches, TouchPhase::Moved, start + Vec2::new(radius * s, 0.0));
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

/// 环境变量秒数（缺省 0）：与各域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}
