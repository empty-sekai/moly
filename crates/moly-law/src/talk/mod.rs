//! talk 域：对话剧本的数据行、出场选取与步进流。
//!
//! [`row`] 镜像两套提取产物与源主表的行结构（`text` 等文本字段原样
//! 携带不解释）；[`select`] 是出场选取——目标层百分比梯、池层成员数
//! 权重抽加档内均匀抽（每层抽签次数钉死），条件枚举六值域全落律，
//! 权重与百分比是服务端下发的可换参数；[`step`] 是步进流推进——
//! `dt` 显式入参，等待步驻留、点击按源语义提前放行、label 供跳转
//! 锚解析。家具对话剧本（含两人/四人目）走同一套选取与推进。

pub mod row;
pub mod select;
pub mod step;

pub use row::{
    ConditionRow, FixtureStep, FixtureTalkRow, PairRow, TalkRow, TalkStep, TweetRef,
    CONDITION_AFTER_SET_FIXTURE, CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT,
    CONDITION_MYSEKAI_FIXTURE_ID, CONDITION_MYSEKAI_FIXTURE_TAG_ID,
    CONDITION_MYSEKAI_PHENOMENA_ID, CONDITION_READ_EVENT_STORY_EPISODE_ID,
    condition_type_discriminant,
};
pub use select::{
    condition_group_matches, condition_matches, get_enable_talk_counts,
    lottery_member_count, lottery_talk_id, matches_lottery_conditions, member_count_of_unit_ids,
    pick_talk_by_member_count, select_fixture_lane, select_objective, talk_conditions_verdict,
    ConditionContext, GateFlags, Candidate, LotteryPercents, LotteryWeights, Objective,
    PercentDraw, TalkLane, TypeOnlyVerdict, UniformDraw, WeightedDraw,
};
pub use step::{
    advance, effective_animation_speed, is_finished, label_anchor, wait_milliseconds,
    wait_time_click_skippable, FaceSlot, Hold, Rejection, StepOp, StreamState, TIME_EPSILON,
};
