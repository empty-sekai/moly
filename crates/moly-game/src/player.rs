//! 玩家域：玩家实体装配与输入驱动的移动。
//!
//! 真源里玩家是独立于名册的实体（`AvatarDataStore.Player` 持唯一
//! `PlayerAvatarPresenter`），移动**不寻路**：`PlayerAvatarMoveState`
//! 与 `PlayerAvatarDashState` 的 UpdateState 同形——每帧
//! `NavMeshAgent.Move(输入向量 × 速度 × dt)` 直接位移。速度来自
//! ClientConfig 面板三键（`MysekaiNormalMoveScale` /
//! `MysekaiHarvestMoveScale` / `MysekaiDashSpeedRate`，FloatConfigs 键
//! 77/78/95），**逐帧取值**——真源 UpdateState 每帧调 getter，本仓同形。
//! 取值序（走/冲两态同构）：
//! * 站点是采集场（grassland）时步速键换 78、动画乘数取 2.0；否则键
//!   77 与 1.75（真源按 `SiteManager.CurrentSiteType == 4` 分支，4 即
//!   grassland——本仓缺省站）。
//! * 场地相机处 FPS 态（`FieldCamera.CurrentState == 16`）时步速减半
//!   （减半只落在位移速度上，动画乘数不受它影响）。
//! * dash 态的 Move 乘数是 rate × scale——减半后的 scale 再乘 rate。
//!
//! 输入形态（方法体直读，非推断）：
//! * 主路是虚拟摇杆（`OnTouchJoyStick`：摇杆向量投到相机的
//!   forward/right 上，相机相对）——触摸面在 [`crate::joystick`]，活跃
//!   时优先（真源 `GetMoveDirection` 只读摇杆面）。键盘路
//!   （`GetKeyMoveDirection`）在真源**没有调用点**，我方留作桌面替身：
//!   `Input.GetAxis("Horizontal")` 与 `("Vertical")`，
//!   方向 = 纵 × 相机前（去 y 后归一）+ 横 × 相机右。
//!   WASD 各贡献 ±1，**和向量不归一**——真源如此，斜向输入模长 √2，
//!   斜走比直走快四成；照抄不修。
//! * dash 是**模式开关**不是触发：`PlayerAvatarStateMachine.MoveTo` 按
//!   玩家 `IsDashMode` 分派 Move/Dash 态，开关本身由 HUD 的 dash 按钮
//!   驱动（UI 域，不在本单）。我方替身：左 Shift 按一下切换（宿主输入
//!   面，具名）。
//! * 无输入回待机：`UpdateController` 在移动态读到空输入即转 Idle。
//!
//! 朝向**直设**：`Quaternion.Euler(0, atan2(输入.x, 输入.z), 0)`，无
//! 转身动画段——转身衔接是 NPC 巡逻域的事。动画段随 [`MotionPhase`]
//! 走玩家自己的驱动链（[`crate::player_avatar`]，在走播走姿、dash 模式
//! 播冲刺、待机播待机段）；播放速率按真源式
//! `clamp(√(输入x²+输入z²) × 站点档乘数, 0.55, 1.8)`（乘数采集场 2.0、
//! 其余站 1.75），待机 1.0（`PlayerAvatarIdleState` 起播字面量）。
//!
//! 相机契约：玩家实体持 [`AvatarRoot`]（本模块接管，名册侧不再插），
//! 相机域读它逐帧取景——玩家装配完成帧起相机追玩家。
//!
//! 模型与动作按产品选择使用 SD 角色；逻辑玩家仍独立于 NPC 名册。
//! 当前沿已有清单第 4 行选择外观，读该角色的 idle/walk/run 原动作，
//! 不复制角色的 NPC 速度、驻留或自主决策。角色装配与材质共用现有
//! 管线，播放器所有权归 [`crate::player_avatar`]，不会另外挂观众身体。
//! * 移动边界：约束面与裁决语义在 [`crate::walk_face`]（真源引擎原生
//!   层不可读）；推进帧与 NPC 共用侵蚀场，整段位移受约束。

use crate::character::AvatarRoot;
use crate::npc::{CharacterUnitId, MotionPhase};
use crate::player_fixture_action::PlayerFixtureHeld;
use crate::site::GroundMeshes;
use crate::walk_face;
use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::path::heading_yaw;

/// 玩家取角色清单第 4 行的身份与SD外观；行选择沿现有占位（挂账）。
/// 名册铺全量后这一行同时是一名名册成员（同
/// unitId 两处出现不冲突——名册域的查询按 `PlayerControlled` 或 NPC
/// 独占组件过滤，见 npc.rs 的名册铺装；要换行先动那一处的成员序）。
const PLAYER_ROW: usize = 3;

/// 出生点：站点中心外环、名册首名的对侧。真源出生点来自存档数据，
/// 未提取——位置是替身。
const SEED_RADIUS: f32 = 3.0;

/// 地表高度采样半径：取「脚下的地面」，与名册同款。
const SURFACE_RADIUS: f32 = 2.0;

/// 走姿动画速率乘数·常态档：真源状态类字面量（非采集场站）。
const ANIM_SPEED_NORMAL: f32 = 1.75;

/// 走姿动画速率乘数·采集场档：真源状态类字面量（站点类型 grassland
/// 时取代常态档）。相机 FPS 档不减半它——减半只落在位移速度上。
const ANIM_SPEED_HARVEST: f32 = 2.0;

/// 动画速率下限：真源状态类常量（跑动段动画速率的地板）。
const MIN_ANIMATION_SPEED: f32 = 0.55;

/// 动画速率上限。版本考证：与资产基线同版的原生反编译里走/冲两态都是
/// `fminf(v, 1.8)` 后 `v < 0.55 ? 0.55 : 上夹值`——上界在；后一版的伪码
/// 反编译里这条 fminf 不在（同函数其余钳制照常渲染成比较 ⇒ 是版本差，
/// 不是反编译噪声）。本仓按资产同版的原生树：上界保留，不许按伪码树
/// 「顺手删上界」。斜向输入和向量不归一（√2 × 2.0 ≈ 2.83）会超它，
/// 超出即压回。
const MAX_ANIMATION_SPEED: f32 = 1.8;

/// 玩家标记：NPC 独占的域（待机动作演出、tweet 气泡、表情编排）用它把
/// 玩家排除在外——玩家不是名册成员，那些域的触发器（驻留计时、编排表）
/// 对玩家没有真源对应物。
#[derive(Component)]
pub struct PlayerControlled;

/// dash 模式开关（真源 `IsDashMode` 同义；切换来源是替身键）。
#[derive(Component, Default)]
pub struct DashMode(pub bool);

/// 本帧输入：方向（世界系，相机相对合成）与是否活跃。输入系统写、推进
/// 系统读。
#[derive(Component, Default)]
pub struct PlayerInput {
    pub direction: Vec3,
    pub active: bool,
}

/// 装载请求：角色清单（player 块与替身行都在同一份文件里）。
#[derive(Resource)]
pub(crate) struct PlayerListHandle(Handle<JsonAsset>);

/// 解析完成的玩家身份与外观动作规格；spawn 消费后即撤。位移速度不在
/// 这儿：速度是 ClientConfig 面板三键的逐帧取值（见 [`move_speed`]），
/// 不随装配算定。只借角色模型与动作名，不借NPC逻辑。
#[derive(Resource)]
pub(crate) struct PlayerSpecs {
    unit_id: u32,
    visual_clips: crate::player_avatar::PlayerVisualClips,
}

/// 玩家出生时缓存的地表世界顶点（推进帧的脚下高度采样用）。
#[derive(Resource)]
pub(crate) struct PlayerGround(Vec<Vec3>);

/// spawn 一次性闩：玩家只铺一次。
#[derive(Resource)]
pub(crate) struct PlayerSpawned;

/// Startup：请求装载角色清单（与 npc 域同一份；装载器按路径去重，
/// 各自持句柄不重复装）。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let handle = server.load::<JsonAsset>(moly_assets::character_registry());
    commands.insert_resource(PlayerListHandle(handle));
}

/// Update：清单装载完成后解析一次。player 块缺列或替身行缺列即响亮
/// panic（资产边界的一次性拒绝），未到齐静默等下一帧。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    lists: Res<Assets<JsonAsset>>,
    handle: Option<Res<PlayerListHandle>>,
) {
    let Some(handle) = handle else {
        return; // 未请求，或已解析并撤下
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("角色清单装载失败：{err:?}");
    }
    let Some(list) = lists.get(&handle.0) else {
        return; // 还在装
    };
    let value: serde_json::Value =
        serde_json::from_str(&list.0).unwrap_or_else(|err| panic!("角色清单不是合法 JSON：{err}"));
    let characters = value
        .get("characters")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("角色清单缺 characters 对象"));
    let row = characters
        .values()
        .nth(PLAYER_ROW)
        .unwrap_or_else(|| panic!("角色清单不足 {} 行：没有玩家替身行", PLAYER_ROW + 1));
    let unit_id = row
        .get("unitId")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| panic!("玩家替身行缺 unitId")) as u32;
    // 速度不读清单：真源 UpdateState 逐帧调 ClientConfig getter，本仓
    // 在推进帧从面板取值（见 [`move_speed`]）。
    let locomotion = row
        .get("locomotion")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("玩家外观 unit {unit_id} 缺 locomotion 动作名"));
    let motion = |key: &str| {
        locomotion
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| panic!("玩家外观 unit {unit_id} 缺 {key}"))
            .to_owned()
    };
    commands.insert_resource(PlayerSpecs {
        unit_id,
        visual_clips: crate::player_avatar::PlayerVisualClips {
            idle: motion("idleMotion"),
            walk: motion("walkMotion"),
            run: motion("runMotion"),
        },
    });
    commands.remove_resource::<PlayerListHandle>();
}

/// Update：规格、面板与站点地表都就绪后，铺一次玩家。实体形状：身份
/// （日志对账用）、移动相位、角色外观动作名（共享装配接玩家专属驱动，
/// 不加入NPC名册），另加玩家专属件：[`AvatarRoot`]
/// （相机自此追玩家）、dash 模式、输入。面板门与名册铺装同款：速度律
/// 的键要读它，面板未立不铺玩家。
/// 出生相位是待机——真源 `InitializeStatus` 起手就是 Idle。
#[allow(clippy::type_complexity)]
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    specs: Option<Res<PlayerSpecs>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    ground: Option<Res<GroundMeshes>>,
    face: Option<Res<walk_face::WalkFace>>,
    spawned: Option<Res<PlayerSpawned>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
) {
    if spawned.is_some() {
        return;
    }
    let (Some(specs), Some(configs), Some(ground)) = (specs, configs, ground) else {
        return;
    };
    let verts = crate::npc::ground_verts(&meshes, &ground, &parts);
    if verts.is_empty() {
        return; // 站点 scene 还没展开，下一帧再试
    }
    let (center, center_y) = crate::npc::center_of(&verts);
    let scale = crate::npc::ring_scale(&verts);
    // 出生在名册首名（环角 0）的对侧（环角 π），替身位（真源出生点未接）；
    // 半径经收缩系数适配房间级地面。替身出生点再落座到可行走面内
    // （真源出生点来自存档，必在 navmesh 上——见 walk_face 模块注释）。
    let radius = SEED_RADIUS * scale;
    let (mut seed_x, mut seed_z) = (center.x - radius, center.y + radius);
    if let Some(face) = face.as_deref() {
        (seed_x, seed_z) = face.seat(seed_x, seed_z);
    }
    let seed = [seed_x, surface_y(&verts, seed_x, seed_z, center_y), seed_z];
    commands.spawn((
        CharacterUnitId(specs.unit_id),
        Transform::from_translation(Vec3::from(seed)),
        // 玩家是渲染层级的节点：模型子实体的可见性沿父链向上查到本实体。
        Visibility::default(),
        // 相机跟随契约：玩家实体持 AvatarRoot（接管自名册首行——真源
        // 站点相机追的就是玩家 avatar 的视变换）。模型与动画段由
        // SD几何/目标绑定复用角色管线，玩家独占AvatarDriver而非NPC驱动。
        AvatarRoot,
        PlayerControlled,
        DashMode(false),
        PlayerInput::default(),
        MotionPhase::Dwelling { remaining: None },
        specs.visual_clips.clone(),
    ));
    commands.insert_resource(PlayerGround(verts));
    commands.insert_resource(PlayerSpawned);
    commands.remove_resource::<PlayerSpecs>();
    info!(
        "[player] 玩家就位：unit {}（名册外替身行 {}），步速键 {}={:.3} · 采集档 {}={:.3} · 冲刺率 {}={:.3}（逐帧取值），AvatarRoot 移交相机",
        specs.unit_id,
        PLAYER_ROW + 1,
        crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE,
        configs.float(crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE),
        crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE,
        configs.float(crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE),
        crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE,
        configs.float(crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE),
    );
}

/// Update：换站后的玩家重播种。吃站点定案代数（见 `site::GroundEpoch`）：
/// 首代只记录不重排——玩家刚以同式铺位；代数翻新（换站）时实体原样保留
/// （AvatarRoot 与相机契约不动），位置落到新站出生点（缩放同 spawn），
/// 脚下地表顶点表换新站的——推进帧的高度采样从此读新地面。
pub(crate) fn reseed(
    epoch: Option<Res<crate::site::GroundEpoch>>,
    mut last: Local<u64>,
    meshes: Res<Assets<Mesh>>,
    ground: Option<Res<GroundMeshes>>,
    face: Option<Res<walk_face::WalkFace>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
    mut players: Query<
        (
            &mut Transform,
            &mut MotionPhase,
            Option<&mut crate::player_avatar::AvatarDriver>,
        ),
        With<PlayerControlled>,
    >,
    mut animators: Query<&mut AnimationPlayer>,
    mut ground_cache: Option<ResMut<PlayerGround>>,
) {
    let Some(epoch) = epoch else {
        return;
    };
    // Scene readiness advances GroundEpoch only after its roots are settled.
    // An unchanged generation cannot need a new world-space vertex snapshot.
    if epoch.0 == *last {
        return;
    }
    let Some(ground) = ground else {
        return;
    };
    let verts = crate::npc::ground_verts(&meshes, &ground, &parts);
    if verts.is_empty() {
        return;
    }
    let first = *last == 0;
    *last = epoch.0;
    if first {
        return;
    }
    let (center, center_y) = crate::npc::center_of(&verts);
    let radius = SEED_RADIUS * crate::npc::ring_scale(&verts);
    let (mut seed_x, mut seed_z) = (center.x - radius, center.y + radius);
    if let Some(face) = face.as_deref() {
        (seed_x, seed_z) = face.seat(seed_x, seed_z);
    }
    let seed = [seed_x, surface_y(&verts, seed_x, seed_z, center_y), seed_z];
    for (mut transform, mut phase, driver) in &mut players {
        if let Some(mut driver) = driver {
            if let Ok(mut animator) = animators.get_mut(driver.player) {
                driver.cancel_for_site_change(&mut animator);
            }
        }
        transform.translation = Vec3::from(seed);
        *phase = MotionPhase::Dwelling { remaining: None };
    }
    if let Some(cache) = &mut ground_cache {
        **cache = PlayerGround(verts);
    }
    info!(
        "[player] 换站重播种：落位 ({:.2},{:.2},{:.2})，脚下地表顶点 {} 个",
        seed[0],
        seed[1],
        seed[2],
        ground_cache.map(|cache| cache.0.len()).unwrap_or(0)
    );
}

/// Update：读输入面，合成相机相对的移动方向。
///
/// 两个输入面并存，摇杆优先：真源 `GetMoveDirection` 只读摇杆面
///（`OnTouchJoyStick` 烘好的 `_joyStickMoveDirection`），键盘路
///（`GetKeyMoveDirection`）在真源是孤儿——没有调用点，我方留作桌面
/// 替身。摇杆活跃（拖动中）时吃摇杆向量；否则吃键盘合成。方向合成
/// 照真源键盘路：纵 × 相机前（去 y 归一）+ 横 × 相机右，和向量不归一
///（斜向模长 √2 是真源形状——摇杆面同形，夹上限后斜向最大 √2）。
/// dash 模式在左 Shift 的按下沿切换（替身键，真源是 HUD 按钮）。
///
/// 冒烟口（`MOLY_PLAYER_AUTOWALK_SECS`，宿主侧仪表，同仓 emoticon 的
/// `MOLY_EMOTICON_SHOW_SECS` 同款）：设为正数时启动后该秒数内合成固定
/// 输入（先 walk 段后 dash 段各半），供无人值守跑验证位移律——窗口焦点
/// 不在时键盘路收不到事件，真实输入面的判据仍是用户手验。
/// `MOLY_PLAYER_AUTOWALK_DIAGONAL` 置正数时方向改 (1,0,1) 不归一（键盘
/// 斜向的和向量形状，模长 √2）。
/// 输入（键盘 + 摇杆两个面）的读取与合成，只被本仓 schedule 消费。
pub(crate) fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut players: Query<(&mut PlayerInput, &mut DashMode), With<PlayerControlled>>,
    time: Res<Time>,
    edits: Res<crate::fixture_edit::EditSessionActive>,
    joystick: Res<crate::joystick::JoystickState>,
    settings_panel: Res<crate::game_settings::SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
) {
    // 摆放编辑面持有输入期间（真源编辑模式下手势层/摇杆归编辑面，
    // ScreenLayerMysekaiCommon 的 _joyStickCanvasGroup），玩家移动让位。
    if edits.is_active()
        || settings_panel.blocks_world_input()
        || library.blocks_exploration_input()
    {
        for (mut input, _) in &mut players {
            input.active = false;
            input.direction = Vec3::ZERO;
        }
        return;
    }
    let vertical = if keys.pressed(KeyCode::KeyW) {
        1.0
    } else if keys.pressed(KeyCode::KeyS) {
        -1.0
    } else {
        0.0
    };
    let horizontal = if keys.pressed(KeyCode::KeyD) {
        1.0
    } else if keys.pressed(KeyCode::KeyA) {
        -1.0
    } else {
        0.0
    };
    let active = vertical != 0.0 || horizontal != 0.0;
    // 相机基：取主相机的世界前/右。相机前去 y 后归一（真源同式）；竖直
    // 朝地的极端姿态下回退单位前向，不做除零。
    let (forward, right) = match cameras.single() {
        Ok(global) => {
            let flat = global.forward().with_y(0.0);
            let forward = if flat.length_squared() < 1e-10 {
                Vec3::Z
            } else {
                flat.normalize()
            };
            (forward, global.right().as_vec3())
        }
        Err(_) => (Vec3::Z, Vec3::X),
    };
    let direction = if active {
        let d = forward * vertical + right * horizontal;
        // 模长上限 1（HandleInput 同形：超上限归一 ×1、中段保持原比）。
        // 真源没有键盘路（摇杆独占），这是桌面替身——但替身的输出必须守
        // InputVec 模长 ≤ 1 的不变量：游戏里斜向不比正向快。此前键盘两键
        // 同按的和向量模长 √2（斜走快四成）是我方替身没夹模长，不是真源
        // 行为（所有者实测：游戏各向移速相同）。
        if d.length() > 1.0 {
            d.normalize()
        } else {
            d
        }
    } else {
        Vec3::ZERO
    };
    // 冒烟口：窗口无焦点时键盘路收不到事件，设了环境变量就以固定输入
    // 驱动前半段（walk 档）与后半段（dash 档），验证位移律真的在动。
    // 方向缺省 +X；MOLY_PLAYER_AUTOWALK_DIAGONAL 置正数时改**归一**斜向
    // (1,0,1)/√2（模长 1，与 HandleInput 的模长上限一致——只换方向多样
    // 性；旧版故意不归一「斜走快四成」的前提是错的：真源 InputVec 模长
    // 永不超 1，动画速率上夹在采集场满速 2.0→1.8 上生效，不需要斜向膨胀）。
    let autowalk_secs = match std::env::var("MOLY_PLAYER_AUTOWALK_SECS") {
        Ok(raw) => raw.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as f32,
        Err(_) => 0.0,
    };
    let diagonal = match std::env::var("MOLY_PLAYER_AUTOWALK_DIAGONAL") {
        Ok(raw) => raw.trim().parse::<f64>().unwrap_or(0.0) > 0.0,
        Err(_) => false,
    };
    let smoke_direction = if diagonal {
        Vec3::new(1.0, 0.0, 1.0).normalize()
    } else {
        Vec3::X
    };
    let smoke = if autowalk_secs > 0.0 && time.elapsed_secs() < autowalk_secs {
        Some(time.elapsed_secs() < autowalk_secs * 0.5)
    } else {
        None
    };
    for (mut input, mut dash) in &mut players {
        if keys.just_pressed(KeyCode::ShiftLeft) {
            dash.0 = !dash.0;
            info!(
                "[player] dash 模式切换：{}",
                if dash.0 { "开" } else { "关" }
            );
        }
        if let Some(first_half) = smoke {
            // 冒烟输入：固定世界方向，前半 walk、后半 dash——速度差在
            // 日志行上现算可辨（位移对时间的斜率）。
            if first_half && dash.0 {
                dash.0 = false;
            } else if !first_half && !dash.0 {
                dash.0 = true;
            }
            input.direction = smoke_direction;
            input.active = true;
        } else if joystick.active {
            // 摇杆优先：真源移动消费链只读摇杆面（GetMoveDirection →
            // _joyStickMoveDirection），键盘路是真源孤儿、我方桌面替身。
            input.direction = joystick.move_vector;
            input.active = true;
        } else {
            input.direction = direction;
            input.active = active;
        }
    }
}

/// 界缘裁决的跨帧态（沿触发日志用）：面内自由、刚被回吸、或被拒保持。
/// pub(crate)：schedule 在模块外登记本系统。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) enum Boundary {
    #[default]
    Free,
    Snapped,
    Held,
}

/// 步速律本体（真源走/冲两态 UpdateState 的取值序，逐帧调用）：采集场
/// 站（grassland）步速吃键 78、其余站吃键 77；场地相机处 FPS 态时步速
/// 减半（只折位移速度，不碰动画乘数）；dash 态乘键 95——真源乘法序是
/// `rate × scale`，减半后的 scale 再乘 rate。返回本帧位移速度，米/秒。
fn move_speed(
    configs: &crate::client_config::ClientConfigs,
    site: &crate::site::SiteSelection,
    camera: crate::camera::CameraStateType,
    dash: bool,
) -> f32 {
    let mut scale = if site.is_grassland() {
        configs.float(crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE)
    } else {
        configs.float(crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE)
    };
    if camera == crate::camera::CameraStateType::Fps {
        scale *= 0.5;
    }
    if dash {
        scale *= configs.float(crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE);
    }
    scale
}

/// 走姿动画速率乘数（真源同函数里与步速并列的那一支）：采集场站 2.0、
/// 其余站 1.75——相机 FPS 档不减半它。
fn animation_speed_mul(site: &crate::site::SiteSelection) -> f32 {
    if site.is_grassland() {
        ANIM_SPEED_HARVEST
    } else {
        ANIM_SPEED_NORMAL
    }
}

/// Update：把本帧输入推进成位置与朝向——玩家域自己的位移律（直接运动
/// 学，真源 `NavMeshAgent.Move` 同形；寻路律归 NPC 巡逻域）。速度按
/// [`move_speed`] 逐帧取值（面板三键 + 站点档 + 相机档）。位移经可行
/// 走场约束：逐段取第一处阻挡前的可走前缀，不能跨过薄障碍。
/// 面未就绪不推
/// 进——真源 navmesh 自场景装载起就在，玩家不走没有约束面的帧。
///
/// 有输入：位置 += 方向 × 速度(dash 档) × dt，脚下高度贴地表采样；朝向
/// 直设 `atan2(方向.x, 方向.z)`（真源同式，无转身段）；相位进 Walking。
/// 无输入：相位进 Dwelling（待机段）。相位的读者是动画驱动链与相机。
/// 玩家 [`Transform`] 的写者：本系统（位移与朝向），与对话域的转身
/// 插值（参演玩家对话期间，本系统被持留让位跳过）。家具会话从接近、
/// 贴合到离座都由自己的移动流程持有位置；其动画租约只覆盖播放段，
/// 不能用播放器占用代替整段位移持留。输入仍由 read_input 采集，供
/// 会话响应主动结束。内部形参带私有资源，故 pub(crate)。
pub(crate) fn advance(
    time: Res<Time>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
    camera_state: Res<crate::camera::FieldCameraState>,
    ground: Option<Res<PlayerGround>>,
    face: Option<Res<walk_face::WalkFace>>,
    // 对话持留让位：玩家参演对话期间位移推进不跑（相位在开场时已钉
    // 驻留；转身由对话域插值直写）。
    mut players: Query<
        (
            &mut Transform,
            &mut MotionPhase,
            &PlayerInput,
            &DashMode,
            Option<&crate::player_avatar::AvatarDriver>,
        ),
        (
            With<PlayerControlled>,
            Without<crate::talk::TalkHold>,
            Without<PlayerFixtureHeld>,
        ),
    >,
    mut boundary: Local<Boundary>,
) {
    let dt = time.delta_secs();
    let (Some(configs), Some(site), Some(ground), Some(face)) = (configs, site, ground, face)
    else {
        return;
    };
    for (mut transform, mut phase, input, dash, driver) in &mut players {
        if driver.is_some_and(|driver| driver.blocks_manual_movement()) {
            continue;
        }
        // 新烘焙可能在脚下挖洞；仅修复无效起点，正常位移永不跨洞吸附。
        if !face.walkable_at([transform.translation.x, transform.translation.z]) {
            let (x, z) = face.seat(transform.translation.x, transform.translation.z);
            transform.translation =
                Vec3::new(x, surface_y(&ground.0, x, z, transform.translation.y), z);
        }
        if input.active {
            let speed = move_speed(&configs, &site, camera_state.0, dash.0);
            let mut position = transform.translation + input.direction * speed * dt;
            let requested = position;
            let start = [transform.translation.x, transform.translation.z];
            let accepted = face.constrain_move(start, [position.x, position.z]);
            position.x = accepted[0];
            position.z = accepted[1];
            let next_boundary = if accepted == [requested.x, requested.z] {
                Boundary::Free
            } else if accepted == start {
                Boundary::Held
            } else {
                Boundary::Snapped
            };
            if *boundary != next_boundary {
                info!(
                    "[player] 导航线段裁决 {:?}：请求 ({:.2},{:.2}) → ({:.2},{:.2})，代数 {}",
                    next_boundary,
                    requested.x,
                    requested.z,
                    position.x,
                    position.z,
                    face.generation()
                );
                *boundary = next_boundary;
            }
            let was_walking = matches!(*phase, MotionPhase::Walking);
            if !was_walking {
                info!(
                    "[player] 输入开始：方向 ({:.2},{:.2}) 模式 {} 速度 {:.3} m/s",
                    input.direction.x,
                    input.direction.z,
                    if dash.0 { "dash" } else { "walk" },
                    speed
                );
            }
            // 朝向直设：真源移动态每帧写 Euler(0, atan2(x, z), 0)。拒进
            // 帧也朝向推挤方向（真源 agent 每帧照样吃 Move 请求）。
            transform.rotation = Quat::from_rotation_y(input.direction.x.atan2(input.direction.z));
            position.y = surface_y(&ground.0, position.x, position.z, position.y);
            transform.translation = position;
            *phase = MotionPhase::Walking;
        } else if matches!(*phase, MotionPhase::Walking) {
            let p = transform.translation;
            info!(
                "[player] 输入结束：位置 ({:.2},{:.2},{:.2})，回待机",
                p.x, p.y, p.z
            );
            *boundary = Boundary::Free;
            *phase = MotionPhase::Dwelling { remaining: None };
        }
    }
}

/// Update：按真源速率律调动画播放速率——移动态
/// `clamp(√(输入x²+输入z²) × 站点档乘数, 0.55, 1.8)`（乘数采集场 2.0、
/// 其余站 1.75；走/冲两态同式，dash 不加速率），待机 1.0。排在玩家动画
/// 驱动（换段）之后：换段会以缺省速率起新段，本系统同帧把速率压回
/// 真源式。
pub fn tune_animation_speed(
    site: Option<Res<crate::site::SiteSelection>>,
    mut last_speed: Local<f32>,
    // The activity's approach/exit motion owns its speed even while the
    // locomotion driver still supplies the visible walk/run clip. Raw input
    // retained for end requests must not retime that movement's animation.
    players: Query<
        (
            &MotionPhase,
            &PlayerInput,
            &crate::player_avatar::AvatarDriver,
        ),
        (With<PlayerControlled>, Without<PlayerFixtureHeld>),
    >,
    mut animators: Query<&mut AnimationPlayer>,
) {
    let Some(site) = site else {
        return;
    };
    let anim_mul = animation_speed_mul(&site);
    for (phase, input, driver) in &players {
        if !driver.locomotion_owns_animator() {
            continue;
        }
        let speed = match phase {
            MotionPhase::Walking => {
                let magnitude = (input.direction.x * input.direction.x
                    + input.direction.z * input.direction.z)
                    .sqrt();
                (magnitude * anim_mul).clamp(MIN_ANIMATION_SPEED, MAX_ANIMATION_SPEED)
            }
            _ => 1.0,
        };
        let Ok(mut animator) = animators.get_mut(driver.player) else {
            continue; // 装配未完成，驱动系统尚未起播
        };
        for (_, animation) in animator.playing_animations_mut() {
            animation.set_speed(speed);
        }
        // 速率账（变更时一行，本系统单玩家）：速率律的现算观测面——
        // 站点档乘数与上夹是否生效从这行读（斜向输入的数是它给的）。
        if speed != *last_speed {
            info!("[player] 动画速率 -> {:.3}（乘数 {:.2}）", speed, anim_mul);
            *last_speed = speed;
        }
    }
}

/// Update（定时）：玩家周期状态行。位置随帧变化是「真的在动」的现算
/// 证据，相邻行可推出位移与速度；dash 档与输入方向让推进行可对账。
/// `界=` 列逐行报可行走面裁决（内=在面内或被回吸按在面缘线上，外=面外
/// ——约束生效时不应出现，?=面未就绪），是移动边界冒烟的逐界验收列。
pub fn report(
    time: Res<Time>,
    face: Option<Res<walk_face::WalkFace>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
    camera_state: Res<crate::camera::FieldCameraState>,
    players: Query<(
        &CharacterUnitId,
        &Transform,
        &MotionPhase,
        &DashMode,
        &PlayerInput,
    )>,
) {
    let Some((configs, site)) = configs.as_deref().zip(site.as_deref()) else {
        return; // 面板或站点选择未立：速度律没有取值面，跳过本行
    };
    for (unit, transform, phase, dash, input) in &players {
        let forward = transform.rotation * Vec3::Z;
        let yaw = heading_yaw([forward.x, forward.y, forward.z]);
        let phase_word = if matches!(phase, MotionPhase::Walking) {
            "walk"
        } else {
            "dwell"
        };
        let speed = move_speed(configs, site, camera_state.0, dash.0);
        let p = transform.translation;
        let boundary = match &face {
            None => "?",
            Some(face) if face.on_face(p.x, p.z) => "内",
            Some(_) => "外",
        };
        info!(
            "[player] unit={} t={:.1} pos=({:.3},{:.3},{:.3}) yaw={:.1} phase={} dash={} 输入=({:.2},{:.2}) 速度档 {:.3} m/s 界={}",
            unit.0,
            time.elapsed_secs(),
            p.x,
            p.y,
            p.z,
            yaw,
            phase_word,
            dash.0,
            input.direction.x,
            input.direction.z,
            speed,
            boundary
        );
    }
}

/// 采样点附近的地表高度：半径内地形顶点的最高者；没有顶点时取 `fallback`
/// （走出地表时保持原高度——移动边界挂账见模块注释）。
fn surface_y(verts: &[Vec3], x: f32, z: f32, fallback: f32) -> f32 {
    verts
        .iter()
        .filter(|v| {
            let dx = v.x - x;
            let dz = v.z - z;
            dx * dx + dz * dz <= SURFACE_RADIUS * SURFACE_RADIUS
        })
        .map(|v| v.y)
        .fold(fallback, f32::max)
}
