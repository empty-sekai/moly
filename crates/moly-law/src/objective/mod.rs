//! 目标系统域：NPC 下一步做什么、停多久、走到哪——决策梯、步间停顿、
//! 与三条目的地解算链（游走 / 靠近家具 / 靠近他人）。
//!
//! 源的形状：主循环每轮「步间停顿 → 决策梯选目标 → 执行目标 → 丢弃
//! 目标」。梯子是一列谓词短路（[`ladder`]），全部落空才落到百分比抽签；
//! 抽签本体在同仓 `talk::select`（同源的目标层百分比梯），本模块**组合
//! 不重写**。停顿是一个独立目标（`Rest`），时长公式在 [`rest`]。
//!
//! 三条目的地链各自的形状（共同点：**失败不报错，返回未命中**，由调用
//! 方决定下一步；每条链的随机源次数都数在文档里）：
//!
//! - [`wander`]（源 `GetRandomPosition` 一族）：可行格减去玩家与 NPC
//!   占格，环滤 `[min², max²]`（双端含），均匀置换取前 5，逐格「表面
//!   采样 + 有路」双探，首个双过即目的地；全 miss 保持原位。恰一次
//!   置换（环大小枚键），无其他随机。
//! - [`approach`]（源 `GetLittleFarPosition`）：家具包围盒外一圈环带
//!   （界外与盒内格剔除），按到出发点 3D 距离稳定升序，逐格探
//!   「格角 + 半格偏移」；首个可采样即目的地。**零抽签**。
//! - [`social`]（源 `GetNearOtherCharacterPosition` 一族）：同站、有
//!   对话数据、非自身的候选；目的地彼此 1.0 内重叠的整员跳过；每员
//!   4 世界轴向 × 3 尺度生成点位，全 NPC 0.7 内即弃、可导航才收；
//!   终滤对每个候选的当前位置与对话目的地都保持 0.7 外；池非空时
//!   **恰一次均匀抽**。
//!
//! # 停顿目标不入梯子
//!
//! 源的停顿（`Rest`）由主循环在决策**之前**无条件插入（调试开关在类
//! 构造里写死为开），不经过梯子；「立即可执行下一目标」的旗能让一轮
//! 停顿整个跳过。这条时序归 [`rest`]，不归 [`ladder`]。
//!
//! # 槽位与断点
//!
//! 梯子读一张「对话槽位」（`AITalkData`，带 19 值类型
//! [`TalkType`]）与一个「当前目标」（21 值 [`ObjectiveType`]）。源里
//! 槽位为空时梯子不判谓词，直接补数据后立对话目标（[`Decision::RefuelAndTalk`]）；
//! 打断标记命中时按标记类型查分发表（`6` 号分派到 18 号目标的不对称
//! 保留在 [`ladder::interrupt_dispatch`]）。枚举值域是源闭集，本模块
//! 不增删。

pub mod approach;
pub mod ladder;
pub mod overlap;
pub mod rest;
pub mod social;
pub mod wander;

pub use approach::{APPROACH_HALF_TILE_OFFSET, APPROACH_SEARCH_RANGE, approach_ring_cells, approach_target};
pub use ladder::{Decision, InterruptDispatch, InterruptMarker, LadderView, decide, interrupt_dispatch};
pub use rest::{TALK_HOLD_STATE_TYPE, RestGateOutcome, rest_delay_milliseconds, rest_holds, try_rest_gate};
pub use social::{
    DESTINATION_OVERLAP, NPC_SPOT_DIRECTIONS, SPOT_CLEARANCE, SPOT_SCALES, SocialCandidate,
    destination_overlaps, is_navigable, nearby_spots, social_target, spot_clear_of_candidates,
};
pub use wander::{TILE_SCALE, WANDER_TAKE, Permute, ring_filter, wander_target};

/// 格坐标 `(x, z)`：站点网格的整数格位（高度轴不在格坐标里）。
pub type Cell = (i32, i32);

/// 表面与通路探测合同（源 `NavMeshAgent` 的 `SamplePosition` /
/// `CalculatePath`，由宿主提供实现）。
///
/// 两条探的语义按各链原样：游走链「采样 + 验路」双探；靠近家具链
/// **只采样不验路**；社交链的可行谓词两者都用且丢弃采样位。
pub trait SurfaceProbe {
    /// 表面采样：容差内把目标落成表面点（源 `SamplePosition(target,
    /// out hit, tolerance, -1)`）；未命中返回 `None`。
    fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]>;
    /// 通路判定：从 `source` 到 `target` 是否有路（源 `CalculatePath`，
    /// 路径对象是一次性的，只取成败）。
    fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool;
}

/// 目标类型（源 `NPCObjectiveType`，21 值闭集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ObjectiveType {
    Invalid = 0,
    AfterEditLayoutHighPriorityTalkReaction = 1,
    AfterEditLayoutReaction = 2,
    EntryMyRoomSite = 3,
    Talk = 4,
    ChangeSite = 5,
    SomeCharacterFixtureActionSub = 6,
    MainSomeCharacterCommunication = 7,
    SubSomeCharacterCommunication = 8,
    SomeCharacterFixtureActionCommunicationWhileDoingWaitSub = 9,
    Greeting = 10,
    PhotoShot = 11,
    ImmediatelyFixtureTimeline = 12,
    RandomMove = 13,
    EntrySite = 14,
    CutScene = 15,
    NoneTalk = 16,
    Rest = 17,
    SubCharacterFixtureAction = 18,
    TutorialWait = 19,
    BirthdayPartyWait = 20,
}

impl ObjectiveType {
    /// 全值域（源闭集，下标即判别值）。
    pub const ALL: [ObjectiveType; 21] = [
        ObjectiveType::Invalid,
        ObjectiveType::AfterEditLayoutHighPriorityTalkReaction,
        ObjectiveType::AfterEditLayoutReaction,
        ObjectiveType::EntryMyRoomSite,
        ObjectiveType::Talk,
        ObjectiveType::ChangeSite,
        ObjectiveType::SomeCharacterFixtureActionSub,
        ObjectiveType::MainSomeCharacterCommunication,
        ObjectiveType::SubSomeCharacterCommunication,
        ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub,
        ObjectiveType::Greeting,
        ObjectiveType::PhotoShot,
        ObjectiveType::ImmediatelyFixtureTimeline,
        ObjectiveType::RandomMove,
        ObjectiveType::EntrySite,
        ObjectiveType::CutScene,
        ObjectiveType::NoneTalk,
        ObjectiveType::Rest,
        ObjectiveType::SubCharacterFixtureAction,
        ObjectiveType::TutorialWait,
        ObjectiveType::BirthdayPartyWait,
    ];

    /// 判别值到成员的闭集还原；域外一律 `None`。
    pub fn from_discriminant(value: u8) -> Option<ObjectiveType> {
        ObjectiveType::ALL.get(value as usize).copied()
    }
}

/// 对话槽位类型（源 `NPCAvatarAITalkType`，19 值闭集）——梯子的槽位输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TalkType {
    None = 0,
    Common = 1,
    SingleCharacterFixture = 2,
    MultipleCharacterFixture = 3,
    ChangeSite = 4,
    CommonFixture = 5,
    CommunicationWhileDoingWait = 6,
    ImmediatelyFixture = 7,
    NoneTalk = 8,
    TutorialFreeWalking = 9,
    TutorialWaiting = 10,
    Greeting = 11,
    AfterEditLayout = 12,
    HighPriorityTalk = 13,
    PhotoShot = 14,
    RandomWalk = 15,
    EntrySite = 16,
    CutScene = 17,
    BirthdayParty = 18,
}

impl TalkType {
    /// 全值域（源闭集，下标即判别值）。
    pub const ALL: [TalkType; 19] = [
        TalkType::None,
        TalkType::Common,
        TalkType::SingleCharacterFixture,
        TalkType::MultipleCharacterFixture,
        TalkType::ChangeSite,
        TalkType::CommonFixture,
        TalkType::CommunicationWhileDoingWait,
        TalkType::ImmediatelyFixture,
        TalkType::NoneTalk,
        TalkType::TutorialFreeWalking,
        TalkType::TutorialWaiting,
        TalkType::Greeting,
        TalkType::AfterEditLayout,
        TalkType::HighPriorityTalk,
        TalkType::PhotoShot,
        TalkType::RandomWalk,
        TalkType::EntrySite,
        TalkType::CutScene,
        TalkType::BirthdayParty,
    ];

    /// 判别值到成员的闭集还原；域外一律 `None`。
    pub fn from_discriminant(value: u8) -> Option<TalkType> {
        TalkType::ALL.get(value as usize).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_type_covers_source_closed_set() {
        // 21 值闭集：判别值 0..=20 全部可还原，21 起域外
        for (index, member) in ObjectiveType::ALL.iter().enumerate() {
            assert_eq!(ObjectiveType::from_discriminant(index as u8), Some(*member));
        }
        assert_eq!(ObjectiveType::from_discriminant(21), None);
        assert_eq!(ObjectiveType::from_discriminant(255), None);
    }

    #[test]
    fn objective_type_discriminants_match_source_values() {
        // 梯子谓词与分发表按判别值工作——锚几处承重值
        assert_eq!(ObjectiveType::Talk as u8, 4);
        assert_eq!(ObjectiveType::ChangeSite as u8, 5);
        assert_eq!(ObjectiveType::SomeCharacterFixtureActionSub as u8, 6);
        assert_eq!(
            ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub as u8,
            9
        );
        assert_eq!(ObjectiveType::NoneTalk as u8, 16);
        assert_eq!(ObjectiveType::Rest as u8, 17);
        assert_eq!(ObjectiveType::SubCharacterFixtureAction as u8, 18);
    }

    #[test]
    fn talk_type_covers_source_closed_set() {
        // 19 值闭集：判别值 0..=18 全部可还原，19 起域外
        for (index, member) in TalkType::ALL.iter().enumerate() {
            assert_eq!(TalkType::from_discriminant(index as u8), Some(*member));
        }
        assert_eq!(TalkType::from_discriminant(19), None);
    }

    #[test]
    fn talk_type_discriminants_match_source_values() {
        // 梯子谓词读的承重值：13 高优、8 无对话、4 换站、12 摆放后、11 问候
        assert_eq!(TalkType::HighPriorityTalk as u8, 13);
        assert_eq!(TalkType::NoneTalk as u8, 8);
        assert_eq!(TalkType::ChangeSite as u8, 4);
        assert_eq!(TalkType::AfterEditLayout as u8, 12);
        assert_eq!(TalkType::Greeting as u8, 11);
    }
}
