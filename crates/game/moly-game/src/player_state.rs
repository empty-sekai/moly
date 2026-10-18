//! 玩家 avatar 行为状态机（第一批：门依赖态）。
//!
//! 真源骨架（`PlayerAvatarStateMachine`，原生树与伪代码树双核）：
//!
//! - **每帧**：`UpdateController` 先推 `OnUpdate`（调当前态的
//!   `UpdateState`，随后若态自设了 `nextStatus` 就代它走一次
//!   `ChangeStatus`），再取输入向量：有输入 → `MoveTo`（写 `InputVec`
//!   后 `ChangeStatus`，dash 位开时目标 23、否则 1）；无输入且当前 ∈
//!   {Move, AutoMove, Dash} → `OnMoveFinish`（= `ChangeStatus(Idle)`）；
//!   无输入且是别的态 → 不写（收场旗只对这三个态为 0）。
//! - **`ChangeStatus(status)` 五段**：① 同态早退；② 动作数据为空抛错；
//!   ③ **截获门**——`CanIntercept` 为假且是本人 avatar 时静默丢弃；
//!   ④ 从 Move 离开去往 Move/AutoMove 之外时 NavMeshAgent 停在当前位；
//!   ⑤ 旧态 Dispose → 换 `currentStatus` → 新态 Initialize → 本人
//!   avatar 时多人广播。
//! - **门默认开**：`PlayerAvatarStateBase` 构造在挂上动作数据后立刻
//!   `SetInterceptFlag(true)`——Setup 期 31 个状态对象全建，门从状态机
//!   装好起就是开的。各独占流程的舞步都是同一形状：**门开着换态进去 →
//!   关门 → 等收场 → 开门 → 换态出去**（采集进出、自动寻路、演出家具
//!   播放全是它）。说话人不关这扇门：挡玩家走动的是输入被对话面收走，
//!   不是状态机。
//!
//! 本仓等价物（与 `camera::FieldCameraState` 同形：常驻 Resource，状态
//! 是被动标记——真源的状态对象也只是标记，行为活在各自域的协程里）：
//!
//! - [`PlayerAvatarStates`]：当前态 + 截获门。
//! - [`PlayerAvatarStates::change_status`]：五段中的 ①③⑤。②的动作数据
//!   在 Resource 形态下构造即真；④的 agent 停位没有对应物——本仓位移
//!   是输入驱动运动学（`player::advance`），让位由各域自己的持留承担
//!   （对话 `TalkHold`、家具会话 `PlayerFixtureHeld`；采集见下），状态机
//!   不再叠加一道；⑤的 Dispose /
//!   Initialize 与多人广播：状态是标记，进出场行为归各自域，单机无
//!   广播。
//! - **`nextStatus` 通道不建**：第一批四态的 `UpdateState` 体没有一个
//!   写它（Idle 只累加计时；Harvest、UseTimelineFixture、Talk 是空体）
//!   ——通道等第一个真用它的态进批再落。
//!
//! 写者：采集动作（`harvest::action`：`PlayHarvestAction` 进 7 关门、
//! 收场开门回 0）· 对话会话（[`drive_from_talk`]）· 输入
//! （[`drive_from_input`]），以及家具会话的进出场。家具控制租约存在时，本模块三个域写者全部让位；截获门由
//! 该会话在最终释放时恢复，不能被别的域的延迟收场提前打开。门读者：
//! 相机近距进入守卫（`camera::can_switch_to_fps`，在 {Harvest,
//! UseTimelineFixture} 拒入）。

use bevy::prelude::*;

use crate::player::{DashMode, PlayerControlled, PlayerInput};
use crate::player_fixture_action::PlayerFixtureControlOwner;
use crate::player_talk::PlayerTalkSession;
use crate::talk::TalkHold;

pub use crate::ui_inputs::{PlayerActionState, PlayerAvatarStates};

/// Update, after the site-move executor (a state the move enters this frame
/// counts this frame's delta, as the state machine's Update runs after the
/// move's continuation): `UpdateState` of the four house and room states.
/// The avatar is hidden through the player's visibility; the move shows it
/// again.
pub(crate) fn update_house_states(
    time: Res<Time>,
    mut states: ResMut<PlayerAvatarStates>,
    mut players: Query<&mut Visibility, With<PlayerControlled>>,
) {
    use crate::door_law::{
        state_timer, ENTER_MOVE_HOUSE_TIME, ENTER_MOVE_MY_ROOM_TIME, EXIT_MOVE_MY_ROOM_TIME,
    };
    let current = states.current;
    let threshold = match current {
        PlayerActionState::EnterMoveHouse => ENTER_MOVE_HOUSE_TIME,
        PlayerActionState::EnterMoveMyRoom => ENTER_MOVE_MY_ROOM_TIME,
        PlayerActionState::ExitMoveMyRoom => EXIT_MOVE_MY_ROOM_TIME,
        // ExitMoveHouse only counts its time.
        PlayerActionState::ExitMoveHouse => f32::INFINITY,
        _ => return,
    };
    let (elapsed, act) = state_timer(states.state_elapsed, time.delta_secs(), threshold);
    states.state_elapsed = elapsed;
    if !act {
        return;
    }
    let hide = matches!(
        current,
        PlayerActionState::EnterMoveHouse | PlayerActionState::ExitMoveMyRoom
    );
    if hide {
        for mut visibility in &mut players {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
    states.can_intercept = true;
    info!(
        "[player-state] {current:?}: {elapsed:.4}s >= {threshold}: gate open{}, next state Idle",
        if hide { ", avatar hidden" } else { "" }
    );
    states.change_status(PlayerActionState::Idle);
}

/// Update: `UpdateState` of the two site exit states. `ElapsedTime` grows by
/// `Time.deltaTime`; once it is no longer below 1.0 (compared in double) a
/// closed gate reopens.
pub(crate) fn update_site_exit_states(time: Res<Time>, mut states: ResMut<PlayerAvatarStates>) {
    if !matches!(
        states.current,
        PlayerActionState::ExitHarvestSite | PlayerActionState::ExitDeliverySite
    ) {
        return;
    }
    let elapsed = states.state_elapsed + time.delta_secs();
    states.state_elapsed = elapsed;
    if !((elapsed as f64) < 1.0) && !states.can_intercept {
        states.can_intercept = true;
        info!("[player-state] {:?}: intercept gate reopened after {elapsed:.3}s", states.current);
    }
}

/// 对话写者：玩家对话会话在播 ⇒ 玩家处于 Talk。
///
/// 真源进场是 `ChangeAvatarsStatusToTalk`（玩家 `ChangeState(6)`、参演
/// NPC `ChangeState(4)`——NPC 支归对话域，不在此重复）。**收场没有专门
/// 写者**：Talk 态留着，由下一次 `MoveTo`/`OnMoveFinish` 把状态带走。
/// 等价地，本写者只在会话在播时重申 Talk（同态时 ① 早退），会话散场
/// 不写，等输入写者接手。会话期间门保持开（真源对话不关 CanIntercept
/// ——挡玩家走动的是输入被对话面收走，本仓对应物是玩家身上的
/// `TalkHold`，见 [`drive_from_input`] 的让位）。玩家点击参加的配对段
/// 同样进入 Talk；纯 NPC 演出和合成探针不改变玩家态。
pub(crate) fn drive_from_talk(
    session: Option<Res<PlayerTalkSession>>,
    pair: Option<Res<crate::talk::ActiveTalk>>,
    fixture_owner: Option<Res<PlayerFixtureControlOwner>>,
    mut states: ResMut<PlayerAvatarStates>,
) {
    if fixture_owner.is_some() {
        return;
    }
    if session.is_some() || pair.is_some_and(|talk| talk.includes_player()) {
        states.change_status(PlayerActionState::Talk);
    }
}

/// 输入写者：`UpdateController` 每帧分派的等价。
///
/// 有输入 → `MoveTo` 的尾部二选一：dash 位开 ⇒ Dash，否则 Move。无输入
/// 且当前 ∈ {Move, AutoMove, Dash} → `OnMoveFinish`（= Idle）；无输入
/// 且是别的态 → 不写。
///
/// 两处输入让位（真源在 `GetMoveDirection` 一层就把输入收走，状态机看到的
/// 是「无输入」）：摆放编辑面持有输入期间；玩家参演对话期间
/// （`TalkHold`——与 `player::advance` 的位移让位同一标记）。
/// 家具会话持有控制租约时不消费移动输入，但不清除输入；该会话仍需
/// 观察点按和摇杆的主动结束请求，实际位移由它自己的生命周期推进。
pub(crate) fn drive_from_input(
    edits: Res<crate::field_modes::EditSessionActive>,
    players: Query<(&PlayerInput, &DashMode, Option<&TalkHold>), With<PlayerControlled>>,
    fixture_owner: Option<Res<PlayerFixtureControlOwner>>,
    mut states: ResMut<PlayerAvatarStates>,
) {
    if fixture_owner.is_some() {
        return;
    }
    let held = players.single().ok();
    let is_input =
        !edits.is_active() && matches!(held, Some((input, _, None)) if input.active);
    if is_input {
        // `MoveTo` 尾部：dash 位决定目标态。
        let dash = matches!(held, Some((_, dash, _)) if dash.0);
        states.change_status(if dash {
            PlayerActionState::Dash
        } else {
            PlayerActionState::Move
        });
    } else if matches!(
        states.current,
        PlayerActionState::Move | PlayerActionState::AutoMove | PlayerActionState::Dash
    ) {
        states.change_status(PlayerActionState::Idle);
    }
}
