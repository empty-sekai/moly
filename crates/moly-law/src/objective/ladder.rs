//! 决策梯（源 `DecideObjective`）：谓词短路序，全部落空才落到百分比
//! 抽签。
//!
//! 源的形状是一列 `if` 返回（按序）：槽位为空补数据立对话 → 摆拍 →
//! 高优反应槽 → 无对话槽 → 打断标记 → 过场保持 → 换站保持 → 摆放后
//! 反应槽 → 问候槽 → 等待通信保持 → 家具动作保持 → 抽签。
//!
//! 两条迁移决策：
//!
//! - **梯子是纯判定**：源入口对引用判空（三处同形，判空即抛异常）；
//!   产品侧引用非空由构造承载，快照 [`LadderView`] 只保留可分支态
//!   「槽位为空」。源在梯内第二次判空不可达（短路序内槽位不变更），
//!   不迁。
//! - **抽签组合不重写**：末档返回 [`Decision::Select`]，抽签本体是
//!   同仓 `talk::select` 的 [`crate::talk::select::select_objective`]
//!   （同源百分比梯）；1–13 档**零抽签**，末档恰消耗该律的 1–3 次
//!   百分比抽。

use crate::talk::select::{select_objective, LotteryPercents, Objective as TalkSelection, PercentDraw};

use super::{ObjectiveType, TalkType};

/// 打断标记（源 `AIModel` 的打断状态）：类型是分发表的键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptMarker {
    /// 标记类型（源标记判别值；有效域见 [`interrupt_dispatch`]）。
    pub marker_type: i32,
    /// 是否可打断（源 `CanInterrupt`）。分派后由调用方清除（源
    /// `SetInterruptFalse`）。
    pub can_interrupt: bool,
}

/// 梯子现场快照：决策只读这一张（槽位、标记、当前目标、问候进度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LadderView {
    /// 对话槽位（源 `AITalkData` 的类型）；`None` = 槽位为空。
    pub talk_type: Option<TalkType>,
    /// 摆拍模式（源 `IsPhotoShotMode`）。
    pub photo_shot: bool,
    /// 打断标记（源打断状态）；`None` = 无标记。
    pub interrupt: Option<InterruptMarker>,
    /// 当前目标类型（源 `CurrentNPCObjective` 的类型）；`None` = 无
    /// 当前目标。
    pub current: Option<ObjectiveType>,
    /// 首次问候已完成（源 `IsCompleteFirstTalk`）。
    pub first_talk_complete: bool,
}

/// 打断分派结局（源 `CreateInterruptTalkObjective` 的 switch）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptDispatch {
    /// 直接构造对应目标。
    Direct(ObjectiveType),
    /// 域外类型：源记错误日志、强制补数据（与空槽位同形的复位 + 抽签）
    /// 后立对话目标。
    InvalidFallback,
}

/// 梯子结局：每一档命中的目标构造（或末档的抽签标记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 槽位为空：复位对话数据并抽签补位（强制补数据），然后立对话
    /// 目标。副作用 = 补数据。
    RefuelAndTalk,
    /// 摆拍模式（构造类型 11）。
    PhotoShot,
    /// 高优反应槽（构造类型 1，`CanCancel` 写 1）。
    HighPriorityTalkReaction,
    /// 无对话槽（构造类型 16）。
    NoneTalk,
    /// 打断标记命中：按标记类型分派。副作用 = 清除标记的可打断位。
    Interrupt {
        /// 分派结局。
        dispatch: InterruptDispatch,
    },
    /// 当前已是过场（构造类型 15，保持）。
    CutScene,
    /// 换站槽且当前已是换站（构造类型 5，保持）。
    ChangeSite,
    /// 摆放后反应槽（构造类型 2，`CanCancel` 写 1）。
    AfterEditLayoutReaction,
    /// 问候槽且首次问候未完成（构造类型 10）。
    Greeting,
    /// 当前已在等待通信（构造类型 9，保持）。
    WaitCommunication,
    /// 当前已在家具动作（构造类型 18，保持）。
    SubCharacterFixtureAction,
    /// 全部谓词落空：目标层百分比抽签（同仓 `talk::select`）。
    Select(TalkSelection),
}

impl Decision {
    /// 本结局立起的目标类型（源各档的构造点）。
    pub fn objective_type(&self) -> ObjectiveType {
        match *self {
            Decision::RefuelAndTalk => ObjectiveType::Talk,
            Decision::PhotoShot => ObjectiveType::PhotoShot,
            Decision::HighPriorityTalkReaction => {
                ObjectiveType::AfterEditLayoutHighPriorityTalkReaction
            }
            Decision::NoneTalk => ObjectiveType::NoneTalk,
            Decision::Interrupt {
                dispatch: InterruptDispatch::Direct(objective),
            } => objective,
            Decision::Interrupt {
                dispatch: InterruptDispatch::InvalidFallback,
            } => ObjectiveType::Talk,
            Decision::CutScene => ObjectiveType::CutScene,
            Decision::ChangeSite => ObjectiveType::ChangeSite,
            Decision::AfterEditLayoutReaction => ObjectiveType::AfterEditLayoutReaction,
            Decision::Greeting => ObjectiveType::Greeting,
            Decision::WaitCommunication => {
                ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub
            }
            Decision::SubCharacterFixtureAction => ObjectiveType::SubCharacterFixtureAction,
            Decision::Select(TalkSelection::Talk(_)) => ObjectiveType::Talk,
            Decision::Select(TalkSelection::NoneTalk) => ObjectiveType::NoneTalk,
        }
    }

    /// 是否可取消：源只在两处反应构造后写 `CanCancel = 1`，其余目标
    /// 一律不可取消。
    pub fn can_cancel(&self) -> bool {
        matches!(
            self,
            Decision::HighPriorityTalkReaction | Decision::AfterEditLayoutReaction
        )
    }

    /// 是否带「复位 + 抽签补数据」副作用（源 `ForceUpdateObjective`）：
    /// 空槽位档与打断域外回退两处。
    pub fn needs_refuel(&self) -> bool {
        match *self {
            Decision::RefuelAndTalk => true,
            Decision::Interrupt {
                dispatch: InterruptDispatch::InvalidFallback,
            } => true,
            _ => false,
        }
    }
}

/// 打断分发表（源 `CreateInterruptTalkObjective` 的 switch）。
///
/// 有效域恰为 `{4, 5, 6, 7, 8, 9, 12, 13, 14, 20}`，映射到目标判别值
/// 同号的目标——**唯一不对称是 6**：标记 6 分派到 18 号（家具动作）
/// 而不是 6 号。域外一律回退：记错误日志 + 强制补数据 + 立对话目标。
pub fn interrupt_dispatch(marker_type: i32) -> InterruptDispatch {
    use InterruptDispatch::Direct;
    use ObjectiveType as Objective;
    match marker_type {
        4 => Direct(Objective::Talk),
        5 => Direct(Objective::ChangeSite),
        6 => Direct(Objective::SubCharacterFixtureAction),
        7 => Direct(Objective::MainSomeCharacterCommunication),
        8 => Direct(Objective::SubSomeCharacterCommunication),
        9 => Direct(Objective::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub),
        12 => Direct(Objective::ImmediatelyFixtureTimeline),
        13 => Direct(Objective::RandomMove),
        14 => Direct(Objective::EntrySite),
        20 => Direct(Objective::BirthdayPartyWait),
        _ => InterruptDispatch::InvalidFallback,
    }
}

/// 决策梯：按源短路序判定，末档组合同仓 `talk::select` 的目标层
/// 百分比抽签。
///
/// 抽签次数：1–13 档零抽签；末档恰 [`select_objective`] 的 1–3 次。
pub fn decide(
    view: &LadderView,
    percents: &LotteryPercents,
    yet_unread_available: bool,
    rand: &mut impl PercentDraw,
) -> Decision {
    // 档 2：槽位为空，不判谓词直接补数据立对话。
    let Some(talk) = view.talk_type else {
        return Decision::RefuelAndTalk;
    };
    // 档 3：摆拍模式。
    if view.photo_shot {
        return Decision::PhotoShot;
    }
    // 档 4：高优反应槽。
    if talk == TalkType::HighPriorityTalk {
        return Decision::HighPriorityTalkReaction;
    }
    // 档 5：无对话槽。
    if talk == TalkType::NoneTalk {
        return Decision::NoneTalk;
    }
    // 档 6：可打断标记（标记存在且可打断才命中）。
    if let Some(marker) = view.interrupt {
        if marker.can_interrupt {
            return Decision::Interrupt {
                dispatch: interrupt_dispatch(marker.marker_type),
            };
        }
    }
    // 档 7：当前已是过场，保持。
    if view.current == Some(ObjectiveType::CutScene) {
        return Decision::CutScene;
    }
    // 档 8：换站槽且当前已是换站，保持。
    if talk == TalkType::ChangeSite && view.current == Some(ObjectiveType::ChangeSite) {
        return Decision::ChangeSite;
    }
    // 档 9：摆放后反应槽。
    if talk == TalkType::AfterEditLayout {
        return Decision::AfterEditLayoutReaction;
    }
    // 档 11：问候槽且首次问候未完成。
    if talk == TalkType::Greeting && !view.first_talk_complete {
        return Decision::Greeting;
    }
    // 档 12：当前已在等待通信，保持。
    if view.current == Some(ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub)
    {
        return Decision::WaitCommunication;
    }
    // 档 13：当前已在家具动作，保持。
    if view.current == Some(ObjectiveType::SomeCharacterFixtureActionSub) {
        return Decision::SubCharacterFixtureAction;
    }
    // 档 14：目标层百分比抽签（组合同仓律，不重写）。
    Decision::Select(select_objective(percents, yet_unread_available, rand))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::talk::select::{Objective as TalkSelection, TalkLane};

    /// 百分比抽签替身：返回固定序列并数调用。
    struct FixedPercent {
        values: [u32; 3],
        calls: usize,
    }

    impl FixedPercent {
        fn at(values: [u32; 3]) -> Self {
            Self { values, calls: 0 }
        }
    }

    impl PercentDraw for FixedPercent {
        fn draw_percent(&mut self) -> u32 {
            let value = self.values[self.calls.min(2)];
            self.calls += 1;
            value
        }
    }

    /// 面板默认形：家具 50 / 已读窗 20 / 无对话 25 / 未读尽已读 50。
    fn percents() -> LotteryPercents {
        LotteryPercents {
            fixture_talk: 50.0,
            already_read_fixture_talk: 20.0,
            none_talk_fixture_action: 25.0,
            already_read_when_has_not_read: 50.0,
        }
    }

    /// 全落空现场：普通槽、无摆拍、无标记、无当前目标、问候已完成。
    fn plain(talk: TalkType) -> LadderView {
        LadderView {
            talk_type: Some(talk),
            photo_shot: false,
            interrupt: None,
            current: None,
            first_talk_complete: true,
        }
    }

    #[test]
    fn empty_slot_refuels_and_talks_without_drawing() {
        // 档 2：槽位为空直接补数据立对话，不判谓词、零抽签
        let mut rand = FixedPercent::at([100, 100, 100]);
        let view = LadderView {
            talk_type: None,
            photo_shot: true,
            interrupt: Some(InterruptMarker { marker_type: 4, can_interrupt: true }),
            current: Some(ObjectiveType::CutScene),
            first_talk_complete: false,
        };
        let decision = decide(&view, &percents(), false, &mut rand);
        assert_eq!(decision, Decision::RefuelAndTalk);
        assert_eq!(decision.objective_type(), ObjectiveType::Talk);
        assert!(!decision.can_cancel());
        assert!(decision.needs_refuel());
        assert_eq!(rand.calls, 0, "空槽位档零抽签");
    }

    #[test]
    fn photo_shot_beats_high_priority_and_none_talk() {
        // 档 3 在档 4/5 之前
        for talk in [TalkType::HighPriorityTalk, TalkType::NoneTalk] {
            let mut view = plain(talk);
            view.photo_shot = true;
            let decision = decide(&view, &percents(), false, &mut FixedPercent::at([100, 0, 0]));
            assert_eq!(decision, Decision::PhotoShot);
            assert_eq!(decision.objective_type(), ObjectiveType::PhotoShot);
        }
    }

    #[test]
    fn high_priority_slot_talks_with_cancel() {
        // 档 4：构造类型 1，唯一写 CanCancel 的两处之一
        let decision =
            decide(&plain(TalkType::HighPriorityTalk), &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::HighPriorityTalkReaction);
        assert_eq!(
            decision.objective_type(),
            ObjectiveType::AfterEditLayoutHighPriorityTalkReaction
        );
        assert!(decision.can_cancel());
        assert!(!decision.needs_refuel());
    }

    #[test]
    fn none_talk_slot_beats_interrupt_marker() {
        // 档 5 在档 6 之前：无对话槽压制可打断标记
        let mut view = plain(TalkType::NoneTalk);
        view.interrupt = Some(InterruptMarker { marker_type: 4, can_interrupt: true });
        let decision = decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::NoneTalk);
        assert_eq!(decision.objective_type(), ObjectiveType::NoneTalk);
    }

    #[test]
    fn interrupt_dispatch_covers_source_switch_table() {
        // 有效域恰为 {4,5,6,7,8,9,12,13,14,20}；同号直构，6 除外
        let table = [
            (4, ObjectiveType::Talk),
            (5, ObjectiveType::ChangeSite),
            (6, ObjectiveType::SubCharacterFixtureAction),
            (7, ObjectiveType::MainSomeCharacterCommunication),
            (8, ObjectiveType::SubSomeCharacterCommunication),
            (
                9,
                ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub,
            ),
            (12, ObjectiveType::ImmediatelyFixtureTimeline),
            (13, ObjectiveType::RandomMove),
            (14, ObjectiveType::EntrySite),
            (20, ObjectiveType::BirthdayPartyWait),
        ];
        for (marker_type, expected) in table {
            assert_eq!(
                interrupt_dispatch(marker_type),
                InterruptDispatch::Direct(expected),
                "标记 {marker_type}"
            );
        }
    }

    #[test]
    fn interrupt_asymmetry_6_maps_to_18_not_6() {
        // 源 switch 的唯一不对称：标记 6 构造 18 号（家具动作），
        // 6 号目标不从打断路径构造
        assert_eq!(
            interrupt_dispatch(6),
            InterruptDispatch::Direct(ObjectiveType::SubCharacterFixtureAction)
        );
        assert_ne!(
            interrupt_dispatch(6),
            InterruptDispatch::Direct(ObjectiveType::SomeCharacterFixtureActionSub)
        );
    }

    #[test]
    fn interrupt_out_of_domain_falls_back_to_refuel_and_talk() {
        // 域外（0..3、10、11、15..17、19 及任意越界值）一律回退
        for marker_type in [0, 1, 2, 3, 10, 11, 15, 16, 17, 19, -1, 99] {
            assert_eq!(
                interrupt_dispatch(marker_type),
                InterruptDispatch::InvalidFallback,
                "标记 {marker_type}"
            );
        }
        let decision = decide(
            &LadderView {
                talk_type: Some(TalkType::Common),
                interrupt: Some(InterruptMarker { marker_type: 3, can_interrupt: true }),
                ..plain(TalkType::Common)
            },
            &percents(),
            false,
            &mut FixedPercent::at([0, 0, 0]),
        );
        assert_eq!(
            decision,
            Decision::Interrupt { dispatch: InterruptDispatch::InvalidFallback }
        );
        assert_eq!(decision.objective_type(), ObjectiveType::Talk);
        assert!(decision.needs_refuel(), "域外回退带强制补数据副作用");
    }

    #[test]
    fn non_interruptible_marker_falls_through() {
        // 不可打断的标记不命中档 6，继续向下
        let mut view = plain(TalkType::Greeting);
        view.first_talk_complete = false;
        view.interrupt = Some(InterruptMarker { marker_type: 4, can_interrupt: false });
        let decision = decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::Greeting);
    }

    #[test]
    fn cut_scene_current_holds() {
        // 档 7：当前已是过场则保持，槽位与标记都不再翻动
        let mut view = plain(TalkType::Common);
        view.current = Some(ObjectiveType::CutScene);
        let decision = decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::CutScene);
        assert_eq!(decision.objective_type(), ObjectiveType::CutScene);
    }

    #[test]
    fn change_site_slot_needs_current_change_site() {
        // 档 8 是合取：槽位与当前目标都得是换站；只占槽位不命中
        let mut only_slot = plain(TalkType::ChangeSite);
        let decision = decide(&only_slot, &percents(), false, &mut FixedPercent::at([100, 100, 100]));
        assert_eq!(
            decision,
            Decision::Select(TalkSelection::Talk(TalkLane::GeneralTalk)),
            "只占换站槽不命中保持档，落到抽签"
        );

        only_slot.current = Some(ObjectiveType::ChangeSite);
        let decision = decide(&only_slot, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::ChangeSite);
    }

    #[test]
    fn after_edit_layout_slot_reacts_with_cancel() {
        // 档 9：构造类型 2，第二个写 CanCancel 的档
        let decision =
            decide(&plain(TalkType::AfterEditLayout), &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::AfterEditLayoutReaction);
        assert_eq!(decision.objective_type(), ObjectiveType::AfterEditLayoutReaction);
        assert!(decision.can_cancel());
    }

    #[test]
    fn greeting_slot_only_before_first_talk_completes() {
        // 档 11：问候槽且首问未完成才问候；完成后落抽签
        let mut view = plain(TalkType::Greeting);
        view.first_talk_complete = false;
        assert_eq!(
            decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0])),
            Decision::Greeting
        );

        view.first_talk_complete = true;
        assert_eq!(
            decide(&view, &percents(), false, &mut FixedPercent::at([100, 100, 100])),
            Decision::Select(TalkSelection::Talk(TalkLane::GeneralTalk))
        );
    }

    #[test]
    fn wait_communication_current_holds() {
        // 档 12：当前已在等待通信（9 号）则保持
        let mut view = plain(TalkType::Common);
        view.current = Some(ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub);
        let decision = decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::WaitCommunication);
        assert_eq!(
            decision.objective_type(),
            ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub
        );
    }

    #[test]
    fn sub_character_fixture_action_current_holds() {
        // 档 13：当前已在家具动作（6 号）则保持（18 号构造）
        let mut view = plain(TalkType::Common);
        view.current = Some(ObjectiveType::SomeCharacterFixtureActionSub);
        let decision = decide(&view, &percents(), false, &mut FixedPercent::at([0, 0, 0]));
        assert_eq!(decision, Decision::SubCharacterFixtureAction);
        assert_eq!(decision.objective_type(), ObjectiveType::SubCharacterFixtureAction);
    }

    #[test]
    fn fallen_through_selects_via_talk_law_with_unread_shortcut() {
        // 档 14 组合同仓律：家具道 + 未读直取，恰 1 次抽
        let mut rand = FixedPercent::at([10, 0, 0]);
        let decision = decide(&plain(TalkType::Common), &percents(), true, &mut rand);
        assert_eq!(
            decision,
            Decision::Select(TalkSelection::Talk(TalkLane::YetUnreadFixtureTalk))
        );
        assert_eq!(decision.objective_type(), ObjectiveType::Talk);
        assert_eq!(rand.calls, 1, "未读直取恰 1 次百分比抽");
    }

    #[test]
    fn fallen_through_fixture_lane_consumes_three_draws() {
        // 家具道无未读 → fixture 车道再抽 2 次：10 < 25 进无对话道，
        // 共 2 次（r0 + r1）
        let mut rand = FixedPercent::at([10, 10, 10]);
        let decision = decide(&plain(TalkType::Common), &percents(), false, &mut rand);
        assert_eq!(decision, Decision::Select(TalkSelection::NoneTalk));
        assert_eq!(rand.calls, 2, "r0 + fixture 车道 r1 共 2 次");
    }

    #[test]
    fn fallen_through_general_talk_consumes_one_draw() {
        // 通用对话道：恰 1 次抽（r0 落在家具与已读窗之外）
        let mut rand = FixedPercent::at([100, 0, 0]);
        let decision = decide(&plain(TalkType::Common), &percents(), false, &mut rand);
        assert_eq!(
            decision,
            Decision::Select(TalkSelection::Talk(TalkLane::GeneralTalk))
        );
        assert_eq!(rand.calls, 1);
    }

    #[test]
    fn objective_types_for_reaction_and_hold_ladders() {
        // 各档构造点的目标类型锚
        assert_eq!(
            Decision::RefuelAndTalk.objective_type(),
            ObjectiveType::Talk
        );
        assert_eq!(Decision::PhotoShot.objective_type(), ObjectiveType::PhotoShot);
        assert_eq!(Decision::NoneTalk.objective_type(), ObjectiveType::NoneTalk);
        assert_eq!(Decision::Greeting.objective_type(), ObjectiveType::Greeting);
        assert_eq!(
            Decision::Select(TalkSelection::NoneTalk).objective_type(),
            ObjectiveType::NoneTalk
        );
    }

    #[test]
    fn only_two_ladders_write_can_cancel() {
        // 源只在两处反应构造写 CanCancel = 1，其余档一律不可取消
        let decisions = [
            Decision::RefuelAndTalk,
            Decision::PhotoShot,
            Decision::NoneTalk,
            Decision::CutScene,
            Decision::ChangeSite,
            Decision::Greeting,
            Decision::WaitCommunication,
            Decision::SubCharacterFixtureAction,
            Decision::Select(TalkSelection::Talk(TalkLane::GeneralTalk)),
            Decision::Select(TalkSelection::NoneTalk),
        ];
        for decision in decisions {
            assert!(!decision.can_cancel(), "{decision:?} 不应可取消");
        }
    }
}
