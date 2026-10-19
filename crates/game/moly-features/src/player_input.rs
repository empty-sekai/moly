//! The player's input routing: the keyboard adaptation and the joystick face
//! composed into the player's camera-relative move direction. It reads the
//! features' gates (the layout editor, the settings panel, the content
//! library, a site move, the entry) because those are what take the field's
//! input away, so the routing lives above them.
//!
//! 两个输入面并存，摇杆优先：真源 `GetMoveDirection` 只读摇杆面
//!（`OnTouchJoyStick` 烘好的 `_joyStickMoveDirection`，按下即活跃、
//! 拖过阈值前向量为零——见 [`crate::joystick`]）。键盘是具名的桌面
//! 适配：WASD 的比值过摇杆的 HandleInput 夹，再过摇杆的烘基式，摇杆
//! 不活跃时才吃它。dash 模式不在这里切换（HUD 冲刺按钮）。
//!
//! 它读的闸门（摆放编辑、设置面板、内容库、换站、入场）就是拿走场地
//! 输入的那几个域，所以路由住在它们上面。

use bevy::prelude::*;

use crate::player::{DashMode, PlayerControlled, PlayerInput};

/// Update：读输入面，合成相机相对的移动方向。
///
/// 冒烟口（`MOLY_PLAYER_AUTOWALK_SECS`，宿主侧仪表，同仓 emoticon 的
/// `MOLY_EMOTICON_SHOW_SECS` 同款）：设为正数时启动后该秒数内合成固定
/// 输入（先 walk 段后 dash 段各半），供无人值守跑验证位移律——窗口焦点
/// 不在时键盘路收不到事件，真实输入面的判据仍是用户手验。
/// `MOLY_PLAYER_AUTOWALK_DIAGONAL` 置正数时方向改归一斜向 (1,0,1)/√2。
/// 输入（键盘 + 摇杆两个面）的读取与合成，只被本仓 schedule 消费。
pub fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut players: Query<(&mut PlayerInput, &mut DashMode), With<PlayerControlled>>,
    time: Res<Time>,
    edits: Res<crate::field_modes::EditSessionActive>,
    joystick: Res<crate::joystick::JoystickState>,
    overlays: crate::game_settings::OverlayPanels,
    library: Res<crate::content_library::ContentLibrary>,
    site_move: Option<Res<crate::field_modes::SiteMoveActive>>,
    entry: Option<Res<crate::entry::EntrySequence>>,
    learn: Option<Res<crate::field_modes::LearnSiteEnvironmentActive>>,
    held: Option<Res<crate::field_modes::HeldGameState>>,
    screens: Option<Res<crate::ui_layers::ScreenManager>>,
) {
    // 摆放编辑面持有输入期间（真源编辑模式下手势层/摇杆归编辑面，
    // ScreenLayerMysekaiCommon 的 _joyStickCanvasGroup），玩家移动让位。
    // GameState SiteMove disables the gesture layer and taps as well.
    // The keyboard adaptation follows the joystick: off until the entry's Finish.
    // A dialog over the field takes the field's input, as it takes the stick's
    // new presses (ExistsDialog(Layer_Dialog)), so the adaptation is off too.
    if edits.is_active()
        || overlays.block_world_input()
        || library.blocks_exploration_input()
        || site_move.is_some()
        || screens.as_deref().is_some_and(crate::ui_layers::ScreenManager::exists_dialog)
        // GameState LearnSiteEnvironment hides the stick.
        || learn.is_some()
        // GameState CutScene / LevelUpMyRoomSite disable the stick.
        || held.is_some()
        || !crate::entry::control_open(entry.as_deref())
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
    // The adaptation's stick ratio (x right, y up), clamped by HandleInput
    // and baked on the main camera exactly as a stick UPDATE is. Without a
    // camera the source's bake does not run and the input stays off.
    let baked = cameras.single().ok().map(|global| {
        crate::joystick_law::bake(
            crate::joystick_law::handle_input(Vec2::new(horizontal, vertical)),
            global,
        )
    });
    let (direction, active) = match baked {
        Some(vector) if active => (vector, true),
        _ => (Vec3::ZERO, false),
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
            // _joyStickMoveDirection）；键盘是具名桌面适配。
            input.direction = joystick.move_vector;
            input.active = true;
        } else {
            input.direction = direction;
            input.active = active;
        }
    }
}

