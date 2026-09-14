//! tweet 域：哪条 tweet 出场（选取律）+ 出场带什么（角色域装配输入）。
//!
//! [`row`] 镜像真源主表的行结构；[`select`] 是四条选取链与一条
//! 确定链接——每条抽签链对随机源**恰求值一次**；[`face`] 把 tweet 行
//! 翻译成角色域消费的动作/眼/口三键。`text` 原样携带不解释，
//! 排版归 `crate::text`。

pub mod face;
pub mod row;
pub mod select;

pub use face::TweetFace;
pub use row::{
    AfterEditRow, GreetingConditionRow, GreetingRow, SiteEntryRow, TalkPreActionRow, TweetRow,
    WithoutRelatedTalkRow, CONDITION_TYPE_TIME_PERIOD, CONDITION_TYPE_VISIT_COUNT,
};
pub use select::{
    condition_matches, greeting_tweet_id, pick_after_edit_tweet, pick_greeting,
    pick_site_entry_tweet, talk_tweet_id, UniformDraw, AFTER_EDIT_REACTION_DELAY_SECONDS,
    GREETING_ABORT_DISTANCE, GREETING_BALLOON_MIN_SECONDS, GREETING_STATE_SECONDS,
    TWEET_DISPLAY_SECONDS,
};
