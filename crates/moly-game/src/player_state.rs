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
//! 写者：采集段（[`drive_from_harvest`]）· 对话会话
//! （[`drive_from_talk`]）· 输入（[`drive_from_input`]），以及家具会话的
//! 进出场。家具控制租约存在时，本模块三个域写者全部让位；截获门由
//! 该会话在最终释放时恢复，不能被别的域的延迟收场提前打开。门读者：
//! 相机近距进入守卫（`camera::can_switch_to_fps`，在 {Harvest,
//! UseTimelineFixture} 拒入）。

use bevy::prelude::*;

use crate::harvest::{HarvestHits, HarvestPunch};
use crate::player::{DashMode, PlayerControlled, PlayerInput};
use crate::player_fixture_action::PlayerFixtureControlOwner;
use crate::player_talk::PlayerTalkSession;
use crate::talk::TalkHold;

/// 玩家行为态：真源 `PlayerActionState` 枚举的**首批子集**（别的域正在
/// 等读的那几个）。全集 34 值，其余 26 个随各自写者的批次落位——本批
/// 不造无写者也无读者的值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerActionState {
    /// 0：待机。`InitializeStatus` 把状态机从这里起步（构造期的 None
    /// 占位只存在于装好之前）。
    #[default]
    Idle,
    /// 1：摇杆移动（`MoveTo` 的非 dash 支）。
    Move,
    /// 2：自动寻路（`ChangeStateAutoMove` 进）。本仓无写者，值留给
    /// 收场集合——无输入收场只认 {Move, AutoMove, Dash} 三态。
    AutoMove,
    /// 6：对话（`ChangeAvatarsStatusToTalk` 的玩家支）。
    Talk,
    /// 7：采集（`HarvestPresenter` 命中臂进、收场臂出）。
    Harvest,
    /// 10: the player's one-second fixture switch interval.
    SwitchGimmick,
    /// 23：冲刺（`MoveTo` 的 dash 支；dash 位由替身键切换）。
    Dash,
    /// 28：演出家具（`ChangeStateUseTimelineFixture` 进）。家具会话
    /// 持有该态和截获门，离座或取消释放后才开门回 Idle。
    UseTimelineFixture,
    /// 34：无。真源 `PlayerAvatarStateBase.ClearNextState` 的哨兵值。
    None,
}

/// 玩家 avatar 状态机的常驻等价物（真源字段子集：`currentStatus` 与
/// `PlayerStateActionData.CanIntercept`；真源 `CurrentState` 读面 = 本
/// 资源的 `current` 字段）。
#[derive(Resource, Debug)]
pub struct PlayerAvatarStates {
    /// 当前行为态。起步 Idle（真源 `InitializeStatus` 写 0）。
    pub current: PlayerActionState,
    /// 截获门：假时 [`Self::change_status`] 静默丢弃（真源
    /// `ChangeStatus` 第③段）。默认开（真源由状态构造开成）。采集与
    /// 家具会话的独占流程分别在自己的进出场关闭、恢复这扇门。
    pub(crate) can_intercept: bool,
}

impl Default for PlayerAvatarStates {
    fn default() -> Self {
        Self {
            current: PlayerActionState::Idle,
            can_intercept: true,
        }
    }
}

impl PlayerAvatarStates {
    /// `ChangeStatus(status)` 的本仓等价（五段中的 ①③⑤；缺段与理由见
    /// 模块注释）。被门丢弃时不报——真源就是静默 return。
    pub(crate) fn change_status(&mut self, status: PlayerActionState) {
        // ① 同态早退：在门之前——真源同态重入连门都不看。
        if self.current == status {
            return;
        }
        // ③ 截获门：关门期间任何换态请求丢弃。本仓玩家恒本人 avatar
        // （真源「非本人放行」支按单机范围省略）。
        if !self.can_intercept {
            return;
        }
        // ⑤ 换态。旧态收场/新态进场的域行为（待机动画、演出启停）不在
        // 标记上，归各自域。
        info!(
            "[player-state] ChangeStatus {:?} -> {:?}",
            self.current, status
        );
        self.current = status;
    }
}

/// 采集段写者：被击队列有货或击打演出在飞 ⇒ 玩家处于 Harvest。
///
/// 真源进出（`HarvestPresenter` 命中臂/收场臂）：**进**——`ChangeState(7)`
/// （门开着进）随后 `SetInterceptFlag(false)` 关门，整段采集演出独占，
/// 期间摇杆的 `MoveTo` 被门丢掉；**出**——`SetInterceptFlag(true)` 开门
/// 随后 `ChangeState(0)`。两处顺序照抄：进是「先换态后关门」，出是
/// 「先开门后换态」。
///
/// ⚠ 分批口径的偏差（具名）：真源**逐次按键** 7→0 循环；本仓采集域是
/// 邻近自动命中批处理形——段 = 「待处理击打非空 ∨ 击打演出在飞」，
/// 连击期间停在 7，两下之间不回 0。玩家位移在段内不持留：真源锁步靠
/// 屏幕态切换加 NavMeshAgent 停位（ChangeStatus 第④段），本仓两者都
/// 没有对应物，且采集域是「走近即命中」形，段内持留会改掉该域现有
/// 行为——位移持留挂到采集域自己的单子上，不在此造。
pub(crate) fn drive_from_harvest(
    hits: Res<HarvestHits>,
    punches: Query<(), With<HarvestPunch>>,
    fixture_owner: Option<Res<PlayerFixtureControlOwner>>,
    mut states: ResMut<PlayerAvatarStates>,
    mut span: Local<bool>,
) {
    if fixture_owner.is_some() {
        // A pending harvest end must not reopen another activity's gate.
        // Keep this writer's span until it can observe the state after release.
        return;
    }
    let active = !hits.0.is_empty() || !punches.is_empty();
    if active && !*span {
        info!("[player-state] harvest span begin: lock the intercept gate");
        states.change_status(PlayerActionState::Harvest);
        states.can_intercept = false;
        *span = true;
    } else if !active && *span {
        info!("[player-state] harvest span end: open the gate and go idle");
        states.can_intercept = true;
        states.change_status(PlayerActionState::Idle);
        *span = false;
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
    edits: Res<crate::fixture_edit::EditSessionActive>,
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
