//! talk 选取律：哪段对话出场、按什么权重。纯函数，抽签显式入参。
//!
//! 三层链，全部逐源迁移：
//! - 目标层 [`select_objective`]：源 `NPCAvatarAIController` 的百分比
//!   梯——一张 [0,100) 整数抽签在「家具对话 / 已读家具对话窗 / 通用
//!   对话」间分道，家具道再进 [`select_fixture_lane`]；
//! - 池层 [`lottery_talk_id`]：源 `MysekaiTalkUtility` 的通用对话抽签
//!   ——八门短路与筛池，空池返回空且**零抽签**；余池按成员数启用表
//!   ([`lottery_member_count`]) 做一次权重抽，再在抽中的成员数档内
//!   恰好一次均匀抽（[`pick_talk_by_member_count`]）；
//! - 条件层 [`condition_matches`]：源条件枚举六值域全落律（见下）。
//!
//! # 权重与百分比是可下发参数
//!
//! 全部权重与百分比来自源 `ClientConfig`（服务端下发）——本律一律
//! 以参数结构带入（[`LotteryWeights`] / [`LotteryPercents`]），不写死
//! 任何值。源里另有一族 `NPCAvatarAITalkType*Weight` 配置键，反编译
//! 全树找不到消费者（疑似死配置），不迁。
//!
//! # 条件族与 tweet 问候族的关系
//!
//! 对话条件枚举六值里，`read_event_story_episode_id` 与
//! `mysekai_character_visit_count` 在此路径**恒通过**（源各有无条件
//! 真的判定，其中前者有具名空参判定方法）；`mysekai_phenomena_id`
//! 是值与当前现象 id 相等；`mysekai_fixture_id` 是在场家具 id 命中；
//! `mysekai_fixture_tag_id` 是在场家具标签与条件值解析出的标签集
//! 相交；`after_set_fixture` **不计入**匹配累积。与 tweet 问候族的
//! visit_count 半开区间条件（`crate::tweet::select`）不是同一枚举。

use std::collections::HashSet;

use super::row::{ConditionRow, condition_type_discriminant, CONDITION_AFTER_SET_FIXTURE,
    CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT, CONDITION_MYSEKAI_FIXTURE_ID,
    CONDITION_MYSEKAI_FIXTURE_TAG_ID, CONDITION_MYSEKAI_PHENOMENA_ID,
    CONDITION_READ_EVENT_STORY_EPISODE_ID};

/// 条件门需要的运行时现场（源里各自由站点管理器/家具管理器提供）。
#[derive(Debug, Clone, Default)]
pub struct ConditionContext<'a> {
    /// 当前现象 id（现象门：条件值 == 此值）。
    pub current_phenomena_id: i32,
    /// 在场家具的 id 集（家具门：条件值命中即过）。
    pub placed_fixture_ids: &'a [i32],
    /// 在场家具的标签集（标签门：与条件值的标签集相交即过）。
    pub placed_fixture_tag_ids: &'a [i32],
    /// 条件值按源主表家具行解析出的标签集（标签门的另一侧；源走
    /// 家具主表的标签组，主表不在提取产物内，由调用方解析带入）。
    pub condition_value_tag_ids: &'a [i32],
}

/// 单条条件是否成立：六值域逐值落律。
///
/// 未知条件类型在源里抛参数越界异常；本律按不匹配处理（fail-closed），
/// 由调用方决定如何响亮。
pub fn condition_matches(condition: &ConditionRow, ctx: &ConditionContext<'_>) -> bool {
    match condition_type_discriminant(&condition.condition_type) {
        Some(CONDITION_READ_EVENT_STORY_EPISODE_ID | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT) => {
            true
        }
        Some(CONDITION_MYSEKAI_PHENOMENA_ID) => {
            condition.value == ctx.current_phenomena_id
        }
        Some(CONDITION_MYSEKAI_FIXTURE_ID) => {
            ctx.placed_fixture_ids.contains(&condition.value)
        }
        Some(CONDITION_MYSEKAI_FIXTURE_TAG_ID) => ctx
            .placed_fixture_tag_ids
            .iter()
            .any(|tag| ctx.condition_value_tag_ids.contains(tag)),
        Some(CONDITION_AFTER_SET_FIXTURE) => false,
        _ => false,
    }
}

/// 带值载荷的条件组判定：源判定的本形——对条件主表行做或累积，
/// 恒真型直接置真，忽略型零贡献。提取形不带值，值载荷由调用方从
/// 源主表连接（见 [`talk_conditions_verdict`]）。
pub fn condition_group_matches<'a>(
    conditions: impl IntoIterator<Item = (&'a str, i32)>,
    ctx: &ConditionContext<'_>,
) -> bool {
    let mut matched = false;
    for (condition_type, value) in conditions {
        let condition = ConditionRow {
            id: 0,
            condition_type: condition_type.to_string(),
            value,
        };
        matched |= condition_matches(&condition, ctx);
    }
    matched
}

/// 提取形（只有类型字符串、无值载荷）的判定结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeOnlyVerdict {
    /// 出现恒真型（`read_event_story_episode_id` /
    /// `mysekai_character_visit_count`）：或累积必然为真。
    Match,
    /// 只有忽略型（`after_set_fixture`）：或累积必然为假。
    NoMatch,
    /// 出现带值型（现象/家具/标签）：值载荷不在提取形里，需从源
    /// 主表连接后走 [`condition_group_matches`]。
    NeedsValue,
}

/// 提取形条件组的可判结局：恒真型存在即过；否则出现带值型即不可判；
/// 只剩忽略型即不过。未知类型与带值型同归不可判（fail-closed 由
/// [`condition_group_matches`] 的未知臂执行）。
pub fn talk_conditions_verdict(condition_types: &[String]) -> TypeOnlyVerdict {
    let mut verdict = TypeOnlyVerdict::NoMatch;
    for condition_type in condition_types {
        match condition_type_discriminant(condition_type) {
            Some(CONDITION_READ_EVENT_STORY_EPISODE_ID | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT) => {
                return TypeOnlyVerdict::Match;
            }
            Some(CONDITION_AFTER_SET_FIXTURE) => {}
            _ => verdict = TypeOnlyVerdict::NeedsValue,
        }
    }
    verdict
}

/// 八门短路与（源 `MatchesLotteryConditions` 的门序，缺一不可）。
///
/// 各门的运行时现场（前一对话、转身阻挡、家具动作区、入场新近度等）
/// 是宿主状态，由调用方按源门序求值后以布尔带入；本律钉住**门的全集
/// 与合取**，不重造现场。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateFlags {
    /// 角色组首指称命中（源角色条件门）。
    pub character_condition: bool,
    /// 前一对话门（源前一 AI 对话数据比对）。
    pub prev_talk: bool,
    /// 条件组门（[`talk_conditions_verdict`] /
    /// [`condition_group_matches`]）。
    pub condition: bool,
    /// 摆放家具一致门。
    pub same_putting_fixture: bool,
    /// 家具动作区门。
    pub fixture_motion_area: bool,
    /// 家具条件可玩门。
    pub can_playable_fixture_condition: bool,
    /// 环境站点条件门。
    pub environment_site_condition: bool,
    /// 入场新近忽略门对话。
    pub ignore_gate_talk_if_entry_recent: bool,
}

/// 八门全过才进池。
pub fn matches_lottery_conditions(gates: &GateFlags) -> bool {
    gates.character_condition
        && gates.prev_talk
        && gates.condition
        && gates.same_putting_fixture
        && gates.fixture_motion_area
        && gates.can_playable_fixture_condition
        && gates.environment_site_condition
        && gates.ignore_gate_talk_if_entry_recent
}

/// 池内一条候选：对话 id、成员数（参演角色数）与八门结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub talk_id: i32,
    pub member_count: i32,
    pub passes_gates: bool,
}

/// 成员数：参演表里非零槽位数（0 号槽是玩家占位，不计成员）。
///
/// 手算锚：`[1] -> 1`、`[1,2] -> 2`、`[1,2,0] -> 2`、`[] -> 0`。
pub fn member_count_of_unit_ids(unit_ids: &[i32]) -> i32 {
    unit_ids.iter().filter(|&&id| id != 0).count() as i32
}

/// 成员数启用表：池内出现过的成员数去重（首见序）。源在去重前逐行
/// 求成员数，这里等价于对成员数序列去重。
pub fn get_enable_talk_counts(member_counts: &[i32]) -> Vec<i32> {
    let mut counts: Vec<i32> = Vec::new();
    for count in member_counts {
        if !counts.contains(count) {
            counts.push(*count);
        }
    }
    counts
}

/// 一次权重抽签：按总权重标定一个落点。源以引擎浮点随机在
/// `[0, total)` 取值；分布与求值次数是本律的一部分，引擎序列不是。
pub trait WeightedDraw {
    fn draw_weight(&mut self, total: f32) -> f32;
}

/// 一次均匀抽签：在 `[0, len)` 取一个索引。与 `crate::tweet` 的
/// 均匀抽签同形。
pub trait UniformDraw {
    fn draw(&mut self, len: usize) -> usize;
}

/// `NPCLotteryTalk1..4Wight`：成员数 1–4 档的权重（服务端下发，
/// mock 可改）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LotteryWeights {
    pub talk1: f32,
    pub talk2: f32,
    pub talk3: f32,
    pub talk4: f32,
}

/// 成员数档权重抽签：候选 `(1, w1)..(4, w4)` 先按启用表过滤，再对
/// 权重和**恰做一次**权重抽，线性扫描取首个累计和严格大于落点的档。
///
/// - 全零权重：源照样抽一次（落点 0 无档命中）后返回 0——本律消耗
///   恰一次抽签并返回 `None`；
/// - 权重和以 f32 累加，扫描序即档序 1→4。
pub fn lottery_member_count(
    enable_counts: &[i32],
    weights: &LotteryWeights,
    rand: &mut impl WeightedDraw,
) -> Option<i32> {
    let enabled: HashSet<i32> = enable_counts.iter().copied().collect();
    let candidates = [
        (1, weights.talk1),
        (2, weights.talk2),
        (3, weights.talk3),
        (4, weights.talk4),
    ];
    let filtered: Vec<(i32, f32)> = candidates
        .into_iter()
        .filter(|(count, _)| enabled.contains(count))
        .collect();
    let total: f32 = filtered.iter().map(|(_, w)| *w).sum();
    let drawn = rand.draw_weight(total);
    let mut acc = 0.0f32;
    for (count, weight) in filtered {
        acc += weight;
        if drawn < acc {
            return Some(count);
        }
    }
    None
}

/// 成员数档内恰一次均匀抽：过滤出该档候选后按表序抽一。
/// 档内为空时源在抽签处抛异常；本律不抽并返回 `None`。
pub fn pick_talk_by_member_count<'a>(
    candidates: &'a [Candidate],
    member_count: i32,
    rand: &mut impl UniformDraw,
) -> Option<&'a Candidate> {
    let matched: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.member_count == member_count)
        .collect();
    if matched.is_empty() {
        return None;
    }
    Some(matched[rand.draw(matched.len())])
}

/// 通用对话抽签全链：八门筛池 → 空池零抽签返回空 → 成员数启用表 →
/// 恰一次权重抽 → 档内恰一次均匀抽。
///
/// 求值次数（与源逐一对应）：
/// - 门筛后空池：0 次抽签；
/// - 权重档抽不中（全零权重等）：恰 1 次权重抽，无均匀抽；
/// - 正常：恰 1 次权重抽 + 恰 1 次均匀抽。
pub fn lottery_talk_id(
    candidates: &[Candidate],
    weights: &LotteryWeights,
    weighted: &mut impl WeightedDraw,
    uniform: &mut impl UniformDraw,
) -> Option<i32> {
    let pool: Vec<Candidate> = candidates
        .iter()
        .copied()
        .filter(|c| c.passes_gates)
        .collect();
    if pool.is_empty() {
        return None;
    }
    let member_counts: Vec<i32> = pool.iter().map(|c| c.member_count).collect();
    let enable_counts = get_enable_talk_counts(&member_counts);
    let member_count = lottery_member_count(&enable_counts, weights, weighted)?;
    pick_talk_by_member_count(&pool, member_count, uniform).map(|c| c.talk_id)
}

/// `NPCLottery*` 百分比族（服务端下发，mock 可改）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LotteryPercents {
    /// 家具对话道占比（源 `NPCLotteryFixtureTalkPercent`）。
    pub fixture_talk: f32,
    /// 已读家具对话窗占比（同前缀 `AlreadyReadFixtureTalkPercent`）。
    pub already_read_fixture_talk: f32,
    /// 无对话（NoTalk）家具行为占比
    /// （`NPCLotteryNoneTalkFixtureActionPercent`）。
    pub none_talk_fixture_action: f32,
    /// 未读尽时的已读家具对话占比
    /// （`NPCLotteryAlreadyReadFixtureTalkPercentWhenHasNotRead`）。
    pub already_read_when_has_not_read: f32,
}

/// 一次百分比抽签：在 `[0, 100)` 取整。源是引擎整数随机。
pub trait PercentDraw {
    fn draw_percent(&mut self) -> u32;
}

/// 目标层的分道结果：对话目标（带道别）或无对话目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// 源对话目标（NpcAvatarTalkObjective 一侧）。
    Talk(TalkLane),
    /// 源无对话目标：头顶 tweet + 表情气泡车道，不进对话窗。
    NoneTalk,
}

/// 对话目标内的道别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkLane {
    /// 尚未读过的家具对话（优先直取）。
    YetUnreadFixtureTalk,
    /// 通用对话（进 [`super::select`] 的池层抽签）。
    GeneralTalk,
    /// 家具旁的常设对话。
    FixtureCommonTalk,
    /// 已读家具对话的重温。
    AlreadyReadTalkFixtureTalk,
}

/// 目标层百分比梯（源 `SelectObjective`）：一张 [0,100) 抽签 `r0`——
/// `r0 < fixture_talk` 进家具道（有未读则直取未读，否则进
/// [`select_fixture_lane`]）；`r0 < fixture_talk + already_read` 进
/// 已读窗（同进 fixture 车道）；其余通用对话。
///
/// 求值次数：直取未读或通用对话恰 1 次百分比抽；进 fixture 车道时
/// 由 [`select_fixture_lane`] 再消耗 1–2 次。
pub fn select_objective(
    percents: &LotteryPercents,
    yet_unread_available: bool,
    rand: &mut impl PercentDraw,
) -> Objective {
    let r0 = rand.draw_percent() as f32;
    if r0 < percents.fixture_talk {
        if yet_unread_available {
            return Objective::Talk(TalkLane::YetUnreadFixtureTalk);
        }
        return select_fixture_lane(percents, rand);
    }
    if r0 < percents.fixture_talk + percents.already_read_fixture_talk {
        return select_fixture_lane(percents, rand);
    }
    Objective::Talk(TalkLane::GeneralTalk)
}

/// fixture 车道（源 `SelectFixtureTalk`）：恰 1 次百分比抽定是否进
/// 无对话道（`r1 < none_talk` 即无对话）；否则**再抽一次**与
/// `already_read_when_has_not_read` 比——不小于则家具常设对话，小于
/// 则已读重温。全零下界（0）不进无对话道：`0 < 0` 为假，源同形。
pub fn select_fixture_lane(
    percents: &LotteryPercents,
    rand: &mut impl PercentDraw,
) -> Objective {
    let r1 = rand.draw_percent() as f32;
    if r1 < percents.none_talk_fixture_action {
        return Objective::NoneTalk;
    }
    let r2 = rand.draw_percent() as f32;
    if r2 >= percents.already_read_when_has_not_read {
        Objective::Talk(TalkLane::FixtureCommonTalk)
    } else {
        Objective::Talk(TalkLane::AlreadyReadTalkFixtureTalk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 权重抽签替身：返回固定比例的落点并数调用——「恰一次求值」的量法。
    struct FixedWeighted {
        fraction: f32,
        calls: usize,
        last_total: f32,
    }

    impl FixedWeighted {
        fn at(fraction: f32) -> Self {
            Self {
                fraction,
                calls: 0,
                last_total: 0.0,
            }
        }
    }

    impl WeightedDraw for FixedWeighted {
        fn draw_weight(&mut self, total: f32) -> f32 {
            self.calls += 1;
            self.last_total = total;
            self.fraction * total
        }
    }

    /// 均匀抽签替身：固定索引并数调用。
    struct CountingDraw {
        index: usize,
        calls: usize,
        last_len: usize,
    }

    impl CountingDraw {
        fn at(index: usize) -> Self {
            Self {
                index,
                calls: 0,
                last_len: 0,
            }
        }
    }

    impl UniformDraw for CountingDraw {
        fn draw(&mut self, len: usize) -> usize {
            self.calls += 1;
            self.last_len = len;
            self.index
        }
    }

    /// 百分比抽签替身：返回固定值并数调用。
    struct FixedPercent {
        value: u32,
        calls: usize,
    }

    impl FixedPercent {
        fn at(value: u32) -> Self {
            Self { value, calls: 0 }
        }
    }

    impl PercentDraw for FixedPercent {
        fn draw_percent(&mut self) -> u32 {
            self.calls += 1;
            self.value
        }
    }

    fn condition(condition_type: &str, value: i32) -> ConditionRow {
        ConditionRow {
            id: 1,
            condition_type: condition_type.to_string(),
            value,
        }
    }

    /// 合成现场：在场家具 {7,9}、在场标签 {100,200}、条件值标签 {150}。
    fn ctx<'a>() -> ConditionContext<'a> {
        ConditionContext {
            current_phenomena_id: 0,
            placed_fixture_ids: &[7, 9],
            placed_fixture_tag_ids: &[100, 200],
            condition_value_tag_ids: &[150],
        }
    }

    // ---- 条件族：六值域逐值正负臂 ----

    #[test]
    fn read_event_story_condition_is_unconditionally_true() {
        // 恒真型：值与现场无关（源空参判定方法恒返回真）
        let mut c = ctx();
        c.current_phenomena_id = 5;
        assert!(condition_matches(&condition("read_event_story_episode_id", 0), &c));
        assert!(condition_matches(&condition("read_event_story_episode_id", 77), &c));
    }

    #[test]
    fn visit_count_condition_is_unconditionally_true_on_talk_path() {
        // 恒真型：对话路径上访问次数门无条件通过（与 tweet 问候族的
        // 半开区间条件不是同一族）
        assert!(condition_matches(&condition("mysekai_character_visit_count", 0), &ctx()));
        assert!(condition_matches(&condition("mysekai_character_visit_count", 42), &ctx()));
    }

    #[test]
    fn phenomena_condition_is_equality_on_current_phenomena() {
        let mut hit = ctx();
        hit.current_phenomena_id = 3;
        assert!(condition_matches(&condition("mysekai_phenomena_id", 3), &hit), "值相等");
        assert!(!condition_matches(&condition("mysekai_phenomena_id", 4), &hit), "值不等");
    }

    #[test]
    fn fixture_id_condition_hits_placed_fixtures() {
        let c = ctx();
        // 在场家具 7：命中；不在场的 8：不命中
        assert!(condition_matches(&condition("mysekai_fixture_id", 7), &c));
        assert!(!condition_matches(&condition("mysekai_fixture_id", 8), &c));
    }

    #[test]
    fn fixture_tag_condition_negative_then_positive() {
        // 负臂：条件标签集 {150} 与在场标签 {100,200} 不交
        let mut miss = ctx();
        miss.condition_value_tag_ids = &[150];
        assert!(!condition_matches(&condition("mysekai_fixture_tag_id", 0), &miss));
        // 正臂：条件标签集 {200} 与在场标签 {100,200} 相交
        let mut hit = ctx();
        hit.condition_value_tag_ids = &[200];
        assert!(condition_matches(&condition("mysekai_fixture_tag_id", 0), &hit));
    }

    #[test]
    fn after_set_fixture_condition_never_counts_toward_match() {
        // 忽略型：单独存在永不过（或累积里零贡献）
        let c = ctx();
        assert!(!condition_matches(&condition("after_set_fixture", 5), &c));
    }

    #[test]
    fn unknown_condition_type_is_fail_closed() {
        // 未知类型源抛参数越界；本律不匹配（fail-closed）
        let c = ctx();
        assert!(!condition_matches(&condition("mysekai_unknown_kind", 1), &c));
    }

    #[test]
    fn condition_group_accumulates_as_or() {
        let mut c = ctx();
        c.current_phenomena_id = 3;
        // 现象命中 + 访问恒真 → 真
        let types = ["mysekai_phenomena_id", "mysekai_character_visit_count"];
        let pairs = types.iter().copied().zip([3, 0]);
        assert!(condition_group_matches(pairs, &c));
        // 仅 after_set → 假（或累积零贡献）
        let only_after = ["after_set_fixture"];
        let pairs = only_after.iter().copied().zip([9]);
        assert!(!condition_group_matches(pairs, &c));
        // after_set + 现象命中 → 真（证明累积仍在工作）
        let mixed = ["after_set_fixture", "mysekai_phenomena_id"];
        let pairs = mixed.iter().copied().zip([9, 3]);
        assert!(condition_group_matches(pairs, &c));
        // 现象不命中 + 无恒真型 → 假
        let missing = ["mysekai_phenomena_id"];
        let pairs = missing.iter().copied().zip([4]);
        assert!(!condition_group_matches(pairs, &c));
        // 标签相交 + 访问恒真 → 真（标签门的值臂在全组形下照常工作）
        let mut tagged = ctx();
        tagged.condition_value_tag_ids = &[200];
        let tag_types = ["mysekai_fixture_tag_id", "mysekai_character_visit_count"];
        let pairs = tag_types.iter().copied().zip([0, 0]);
        assert!(condition_group_matches(pairs, &tagged));
    }

    #[test]
    fn type_only_form_verdicts_by_constant_and_value_bearing_types() {
        // 提取形（只有类型字符串）：恒真型在场即过
        let visit = ["mysekai_character_visit_count".to_string()];
        assert_eq!(talk_conditions_verdict(&visit), TypeOnlyVerdict::Match);
        let story = ["read_event_story_episode_id".to_string()];
        assert_eq!(talk_conditions_verdict(&story), TypeOnlyVerdict::Match);
        // 仅忽略型：不过
        let after = ["after_set_fixture".to_string()];
        assert_eq!(talk_conditions_verdict(&after), TypeOnlyVerdict::NoMatch);
        // 带值型在场：不可判（值载荷在源主表）
        let needs = ["mysekai_phenomena_id".to_string()];
        assert_eq!(talk_conditions_verdict(&needs), TypeOnlyVerdict::NeedsValue);
        let mixed = [
            "after_set_fixture".to_string(),
            "mysekai_fixture_id".to_string(),
        ];
        assert_eq!(talk_conditions_verdict(&mixed), TypeOnlyVerdict::NeedsValue);
        // 未知类型同样归不可判（fail-closed 在带值形执行）
        let unknown = ["mysekai_unknown_kind".to_string()];
        assert_eq!(talk_conditions_verdict(&unknown), TypeOnlyVerdict::NeedsValue);
    }

    // ---- 八门 ----

    #[test]
    fn all_eight_gates_must_pass() {
        let gates = GateFlags {
            character_condition: true,
            prev_talk: true,
            condition: true,
            same_putting_fixture: true,
            fixture_motion_area: true,
            can_playable_fixture_condition: true,
            environment_site_condition: true,
            ignore_gate_talk_if_entry_recent: true,
        };
        assert!(matches_lottery_conditions(&gates));
        // 逐门翻转负臂：任一门关即不过
        let one_off = [
            ("character_condition", GateFlags { character_condition: false, ..gates }),
            ("prev_talk", GateFlags { prev_talk: false, ..gates }),
            ("condition", GateFlags { condition: false, ..gates }),
            ("same_putting_fixture", GateFlags { same_putting_fixture: false, ..gates }),
            ("fixture_motion_area", GateFlags { fixture_motion_area: false, ..gates }),
            ("can_playable_fixture_condition", GateFlags { can_playable_fixture_condition: false, ..gates }),
            ("environment_site_condition", GateFlags { environment_site_condition: false, ..gates }),
            ("ignore_gate_talk_if_entry_recent", GateFlags { ignore_gate_talk_if_entry_recent: false, ..gates }),
        ];
        for (name, flipped) in one_off {
            assert!(!matches_lottery_conditions(&flipped), "门 {name} 关时应拒");
        }
        assert!(!matches_lottery_conditions(&GateFlags::default()));
    }

    // ---- 成员数与启用表 ----

    #[test]
    fn member_count_counts_nonzero_slots() {
        // 手算锚：0 号槽是玩家占位，不计成员
        assert_eq!(member_count_of_unit_ids(&[1]), 1);
        assert_eq!(member_count_of_unit_ids(&[1, 2]), 2);
        assert_eq!(member_count_of_unit_ids(&[1, 2, 3, 4]), 4);
        assert_eq!(member_count_of_unit_ids(&[1, 0]), 1);
        assert_eq!(member_count_of_unit_ids(&[]), 0);
    }

    #[test]
    fn enable_talk_counts_dedups_in_first_seen_order() {
        assert_eq!(get_enable_talk_counts(&[1, 2, 1, 4, 2]), vec![1, 2, 4]);
        assert_eq!(get_enable_talk_counts(&[]), Vec::<i32>::new());
    }

    // ---- 成员数档权重抽签 ----

    fn weights(t1: f32, t2: f32, t3: f32, t4: f32) -> LotteryWeights {
        LotteryWeights {
            talk1: t1,
            talk2: t2,
            talk3: t3,
            talk4: t4,
        }
    }

    #[test]
    fn member_count_lottery_consumes_exactly_one_draw() {
        let w = weights(10.0, 30.0, 0.0, 0.0);
        let mut rand = FixedWeighted::at(0.0);
        let picked = lottery_member_count(&[1, 2], &w, &mut rand);
        assert_eq!(picked, Some(1));
        assert_eq!(rand.calls, 1, "恰一次权重抽");
        // 分母只含启用的档：talk3/talk4 关着，总权重 = 40
        assert_eq!(rand.last_total, 40.0);
    }

    #[test]
    fn member_count_lottery_boundary_is_strictly_less_than_cumulative() {
        // 手算锚：w1=10, w2=30，落点 = 比例×40
        // 比例 0.25 → 落点恰 10.0：10 < 10 为假 → 落第二档（源严格小于）
        let w = weights(10.0, 30.0, 0.0, 0.0);
        let mut at_boundary = FixedWeighted::at(0.25);
        assert_eq!(lottery_member_count(&[1, 2], &w, &mut at_boundary), Some(2));
        // 比例 0.24 → 落点 9.6 < 10 → 第一档
        let mut below = FixedWeighted::at(0.24);
        assert_eq!(lottery_member_count(&[1, 2], &w, &mut below), Some(1));
        // 比例 0.99 → 落点 39.6：10 < 39.6 过一档、40 < 39.6 假 → 第二档
        let mut high = FixedWeighted::at(0.99);
        assert_eq!(lottery_member_count(&[1, 2], &w, &mut high), Some(2));
    }

    #[test]
    fn member_count_lottery_all_zero_weights_still_draws_once() {
        // 全零权重：源照样抽（落点 0 无档命中）后返 0
        let w = weights(0.0, 0.0, 0.0, 0.0);
        let mut rand = FixedWeighted::at(0.0);
        assert_eq!(lottery_member_count(&[1, 2], &w, &mut rand), None);
        assert_eq!(rand.calls, 1, "权重抽已消费");
        assert_eq!(rand.last_total, 0.0);
    }

    #[test]
    fn member_count_lottery_empty_enable_table_draws_once_and_misses() {
        let w = weights(10.0, 10.0, 10.0, 10.0);
        let mut rand = FixedWeighted::at(0.0);
        assert_eq!(lottery_member_count(&[], &w, &mut rand), None);
        assert_eq!(rand.calls, 1, "空启用表：源对零和抽一次后空手");
        assert_eq!(rand.last_total, 0.0);
    }

    #[test]
    fn member_count_lottery_disabled_tiers_leave_the_denominator() {
        // 启用表只有 {2}：总权重 = talk2 单档
        let w = weights(10.0, 30.0, 0.0, 0.0);
        let mut rand = FixedWeighted::at(0.99);
        assert_eq!(lottery_member_count(&[2], &w, &mut rand), Some(2));
        assert_eq!(rand.last_total, 30.0, "未启用档不进分母");
    }

    // ---- 档内均匀抽 ----

    fn candidate(talk_id: i32, member_count: i32, passes: bool) -> Candidate {
        Candidate {
            talk_id,
            member_count,
            passes_gates: passes,
        }
    }

    #[test]
    fn pick_by_member_count_draws_once_within_tier() {
        let pool = vec![
            candidate(101, 1, true),
            candidate(201, 2, true),
            candidate(202, 2, true),
        ];
        let mut rand = CountingDraw::at(1);
        let picked = pick_talk_by_member_count(&pool, 2, &mut rand);
        assert_eq!(picked.map(|c| c.talk_id), Some(202));
        assert_eq!(rand.calls, 1, "恰一次均匀抽");
        assert_eq!(rand.last_len, 2, "分母 = 该档候选数");

        let mut empty = CountingDraw::at(0);
        assert_eq!(pick_talk_by_member_count(&pool, 3, &mut empty), None);
        assert_eq!(empty.calls, 0, "空档不抽签");
    }

    // ---- 全链 ----

    #[test]
    fn lottery_talk_id_full_path_is_one_weighted_plus_one_uniform_draw() {
        let pool = vec![
            candidate(101, 1, true),
            candidate(201, 2, true),
            candidate(202, 2, false), // 八门未过：不进池
        ];
        let w = weights(0.0, 50.0, 0.0, 0.0);
        let mut weighted = FixedWeighted::at(0.5); // 落点 25 < 50 → 档 2
        let mut uniform = CountingDraw::at(0); // 档 2 只有 201
        let picked = lottery_talk_id(&pool, &w, &mut weighted, &mut uniform);
        assert_eq!(picked, Some(201));
        assert_eq!(weighted.calls, 1);
        assert_eq!(uniform.calls, 1);
        assert_eq!(uniform.last_len, 1, "八门未过的候选不进分母");
    }

    #[test]
    fn lottery_talk_id_empty_pool_after_gates_draws_nothing() {
        let pool = vec![candidate(101, 1, false)];
        let w = weights(10.0, 0.0, 0.0, 0.0);
        let mut weighted = FixedWeighted::at(0.0);
        let mut uniform = CountingDraw::at(0);
        assert_eq!(lottery_talk_id(&pool, &w, &mut weighted, &mut uniform), None);
        assert_eq!(weighted.calls, 0, "空池零抽签");
        assert_eq!(uniform.calls, 0);
    }

    #[test]
    fn lottery_talk_id_zero_weights_consumes_weighted_draw_only() {
        let pool = vec![candidate(101, 1, true)];
        let w = weights(0.0, 0.0, 0.0, 0.0);
        let mut weighted = FixedWeighted::at(0.0);
        let mut uniform = CountingDraw::at(0);
        assert_eq!(lottery_talk_id(&pool, &w, &mut weighted, &mut uniform), None);
        assert_eq!(weighted.calls, 1, "权重抽已消费");
        assert_eq!(uniform.calls, 0, "档抽不中时不进均匀抽");
    }

    // ---- 目标层百分比梯 ----

    fn percents(fixture: f32, already: f32, none_talk: f32, already_wnr: f32) -> LotteryPercents {
        LotteryPercents {
            fixture_talk: fixture,
            already_read_fixture_talk: already,
            none_talk_fixture_action: none_talk,
            already_read_when_has_not_read: already_wnr,
        }
    }

    #[test]
    fn objective_below_fixture_percent_enters_fixture_lane() {
        let p = percents(30.0, 20.0, 40.0, 50.0);
        // r0=29 < 30 → 家具道；无未读 → fixture 车道：r1=10 < 40 → 无对话
        let mut rand = FixedPercent::at(29);
        assert_eq!(select_objective(&p, false, &mut rand), Objective::NoneTalk);
        assert_eq!(rand.calls, 2, "r0 + fixture 车道首抽");
    }

    #[test]
    fn objective_fixture_boundary_is_at_percent_not_inclusive() {
        let p = percents(30.0, 20.0, 0.0, 100.0);
        // 固定抽 30：r0=30 不进家具道（30 < 30 假）；30 < 50 → 已读窗
        // → fixture 车道。车道内 r1=30（30 < 0 假，非无对话）、
        // r2=30（30 >= 100 假）→ 已读重温
        let mut at = FixedPercent::at(30);
        assert_eq!(
            select_objective(&p, false, &mut at),
            Objective::Talk(TalkLane::AlreadyReadTalkFixtureTalk)
        );
        assert_eq!(at.calls, 3, "已读窗 + 车道双抽");
        // r0=29 < 30 → 家具道，无未读 → fixture 车道同上
        let mut below = FixedPercent::at(29);
        assert_eq!(
            select_objective(&p, false, &mut below),
            Objective::Talk(TalkLane::AlreadyReadTalkFixtureTalk)
        );
    }

    #[test]
    fn objective_window_upper_boundary_lands_general_talk() {
        let p = percents(30.0, 20.0, 0.0, 0.0);
        // r0=50 == 30+20：50 < 50 为假 → 通用对话（源严格小于）
        let mut rand = FixedPercent::at(50);
        assert_eq!(
            select_objective(&p, true, &mut rand),
            Objective::Talk(TalkLane::GeneralTalk)
        );
        assert_eq!(rand.calls, 1, "通用对话直取，无后续抽");
        // r0=49 < 50 → 已读窗 → fixture 车道
        let mut inner = FixedPercent::at(49);
        assert_eq!(
            select_objective(&p, true, &mut inner),
            Objective::Talk(TalkLane::FixtureCommonTalk)
        );
    }

    #[test]
    fn objective_yet_unread_short_circuits_before_fixture_lane() {
        let p = percents(30.0, 20.0, 40.0, 50.0);
        let mut rand = FixedPercent::at(10);
        assert_eq!(
            select_objective(&p, true, &mut rand),
            Objective::Talk(TalkLane::YetUnreadFixtureTalk)
        );
        assert_eq!(rand.calls, 1, "直取未读：恰 1 抽");
    }

    #[test]
    fn fixture_lane_second_draw_chooses_between_common_and_already_read() {
        let p = percents(30.0, 20.0, 40.0, 50.0);
        // r1=50 >= 40 → 非无对话；r2=50 >= 50 → 常设对话（源不小于）
        let mut at = FixedPercent::at(50);
        assert_eq!(select_fixture_lane(&p, &mut at), Objective::Talk(TalkLane::FixtureCommonTalk));
        assert_eq!(at.calls, 2);
        // r2=49 < 50 → 已读重温
        let mut below = FixedPercent::at(49);
        assert_eq!(
            select_fixture_lane(&p, &mut below),
            Objective::Talk(TalkLane::AlreadyReadTalkFixtureTalk)
        );
    }

    #[test]
    fn fixture_lane_below_none_talk_percent_is_none_talk() {
        let p = percents(30.0, 20.0, 40.0, 50.0);
        // r1=39 < 40 → 无对话道，第二抽不发生
        let mut rand = FixedPercent::at(39);
        assert_eq!(select_fixture_lane(&p, &mut rand), Objective::NoneTalk);
        assert_eq!(rand.calls, 1, "无对话道只消耗首抽");
        // 边界：r1=40 → 40 < 40 为假 → 非无对话
        let mut at = FixedPercent::at(40);
        assert_ne!(select_fixture_lane(&p, &mut at), Objective::NoneTalk);
    }

    #[test]
    fn zero_none_talk_percent_never_enters_none_talk_lane() {
        // 负臂的零界：0 < 0 为假，源同形
        let p = percents(30.0, 20.0, 0.0, 100.0);
        let mut rand = FixedPercent::at(0);
        assert_ne!(select_fixture_lane(&p, &mut rand), Objective::NoneTalk);
    }
}
