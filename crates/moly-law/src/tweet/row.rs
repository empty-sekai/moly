//! tweet 域的主表行：与真源 MessagePack 主表的字段一一对应。
//!
//! 行类型只镜像结构键。文本字段（`text` 与各名字）原样携带，不在本层
//! 解释——文本的排版归 `crate::text` 消费。`emoticon_name` 在源表存在
//! 而提取产物尚未导出该列（单据挂账），装载侧暂以 `None` 喂入。

/// 源 `MasterMysekaiCharacterTalkTweet`：一条头顶 tweet。
///
/// `motion_name` 与 `emoticon_name` 在源标记可空；`eye_name`、
/// `mouth_name`、`text` 不可空——空值在源的表情装配处走 LogError
/// 错误路径，不是合法数据。
#[derive(Debug, Clone, PartialEq)]
pub struct TweetRow {
    pub id: i32,
    pub motion_name: Option<String>,
    pub emoticon_name: Option<String>,
    pub eye_name: String,
    pub mouth_name: String,
    pub text: String,
}

/// 源 `MasterMysekaiCharacterTalkTweetWithoutRelatedTalk`：
/// 「无关联发言」行，把一条 tweet 归属给一个角色。
#[derive(Debug, Clone, PartialEq)]
pub struct WithoutRelatedTalkRow {
    pub id: i32,
    pub game_character_unit_id: i32,
    pub tweet_id: i32,
}

/// 源 `MasterMysekaiCharacterTalkTweetGreeting`：问候 tweet 的入选行。
///
/// 角色归属经 [`WithoutRelatedTalkRow`] 一跳，条件经
/// [`GreetingConditionRow`] 一跳。
#[derive(Debug, Clone, PartialEq)]
pub struct GreetingRow {
    pub id: i32,
    pub without_related_talk_id: i32,
    pub greeting_condition_id: i32,
}

/// 条件类型字符串之一：访问次数区间。
pub const CONDITION_TYPE_VISIT_COUNT: &str = "mysekai_character_visit_count";
/// 条件类型字符串之一：当前现象时段相等。
pub const CONDITION_TYPE_TIME_PERIOD: &str = "mysekai_phenomena_time_period";

/// 源 `MasterMysekaiCharacterTalkTweetGreetingCondition`。
///
/// 类型是服务端下发的字符串，不在已知两值（
/// [`CONDITION_TYPE_VISIT_COUNT`] / [`CONDITION_TYPE_TIME_PERIOD`]）
/// 之列的条件行永不匹配。
#[derive(Debug, Clone, PartialEq)]
pub struct GreetingConditionRow {
    pub id: i32,
    pub condition_type: String,
    pub value1: i32,
    pub value2: i32,
}

/// 源 `MasterMysekaiCharacterTalkTweetHousingRoomSiteEntry`：
/// 站点入口 tweet 的引用行。
#[derive(Debug, Clone, PartialEq)]
pub struct SiteEntryRow {
    pub id: i32,
    pub without_related_talk_id: i32,
}

/// 源 `MasterMysekaiCharacterTalkTweetAfterEditHousingLayout`：
/// 摆设编辑后反应 tweet 的池来源行。
#[derive(Debug, Clone, PartialEq)]
pub struct AfterEditRow {
    pub id: i32,
    pub without_related_talk_id: i32,
}

/// 源 `MasterMysekaiCharacterTalkPreAction` 中本域消费的两列：
/// 一段对话开场前置展示的 tweet。确定链接，无抽签。
#[derive(Debug, Clone, PartialEq)]
pub struct TalkPreActionRow {
    pub mysekai_character_talk_id: i32,
    pub tweet_id: i32,
}
