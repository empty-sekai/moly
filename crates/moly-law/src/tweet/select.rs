//! tweet 选取律：哪条 tweet 出场、什么时机。纯函数，抽签显式入参。
//!
//! 四条链 + 一条确定链接，全部逐源迁移：
//! - 问候链 [`pick_greeting`]：角色 + 条件双过滤后恰一次抽签；
//! - 站点入口链 [`pick_site_entry_tweet`]：交集 + Any 门后恰一次抽签；
//! - 摆设编辑反应链 [`pick_after_edit_tweet`]：池过滤后恰一次抽签，
//!   空池静默跳过；
//! - 对话前置链接 [`talk_tweet_id`]：确定查找，零抽签。
//!
//! 抽签纪律：**每条链对随机源恰求值一次**。源的三副 `RandomPick`
//! 重载用两种引擎——IEnumerable 副新建 `System.Random` 并取
//! `Next(count)`，数组副走 `UnityEngine.Random.Range(0, len)`——
//! 两者同为 `[0, len)` 均匀。引擎序列不是本律的一部分；分布与
//! 求值次数才是。调用方经 [`UniformDraw`] 提供引擎。

use std::collections::HashSet;

use super::row::{
    AfterEditRow, GreetingConditionRow, GreetingRow, SiteEntryRow, TalkPreActionRow, TweetRow,
    WithoutRelatedTalkRow, CONDITION_TYPE_TIME_PERIOD, CONDITION_TYPE_VISIT_COUNT,
};

/// 一次均匀抽签：在 `[0, len)` 取一个索引。
pub trait UniformDraw {
    fn draw(&mut self, len: usize) -> usize;
}

/// 问候条件是否成立。
///
/// 访问次数型是半开区间 `value1 <= visit_count` 且（`value2 == 0`
/// 表示无上界，否则 `visit_count < value2`）；时段型是当前时段与
/// `value1` 相等。条件行缺失（调用方未给该 id 的行）不匹配。
pub fn condition_matches(
    condition: &GreetingConditionRow,
    visit_count: i32,
    time_period: i32,
) -> bool {
    if condition.condition_type == CONDITION_TYPE_VISIT_COUNT {
        condition.value1 <= visit_count && (condition.value2 == 0 || visit_count < condition.value2)
    } else if condition.condition_type == CONDITION_TYPE_TIME_PERIOD {
        time_period == condition.value1
    } else {
        false
    }
}

/// 问候行是否属于该角色：经 WRT 一跳比对归属。
///
/// WRT 行缺失时源记一条错误日志后按不匹配处理——行被剔除，不中断。
fn greeting_matches_character(
    greeting: &GreetingRow,
    wrt: &[WithoutRelatedTalkRow],
    character_unit_id: i32,
) -> bool {
    wrt.iter()
        .find(|w| w.id == greeting.without_related_talk_id)
        .map(|w| w.game_character_unit_id == character_unit_id)
        .unwrap_or(false)
}

/// 问候链的选取：角色匹配 → 条件匹配 → 恰一次均匀抽签。
///
/// 筛后为空时源在抽签处抛异常（IEnumerable 副对零元素序列无定义）；
/// 本律以 `None` 返回且**不**调用抽签，由调用方决定如何响亮。
/// 候选顺序 = 问候表顺序。
pub fn pick_greeting<'a>(
    greetings: &'a [GreetingRow],
    wrt: &[WithoutRelatedTalkRow],
    conditions: &[GreetingConditionRow],
    character_unit_id: i32,
    visit_count: i32,
    time_period: i32,
    rand: &mut impl UniformDraw,
) -> Option<&'a GreetingRow> {
    let matched: Vec<&GreetingRow> = greetings
        .iter()
        .filter(|g| {
            greeting_matches_character(g, wrt, character_unit_id)
                && conditions
                    .iter()
                    .find(|c| c.id == g.greeting_condition_id)
                    .is_some_and(|c| condition_matches(c, visit_count, time_period))
        })
        .collect();
    if matched.is_empty() {
        return None;
    }
    let idx = rand.draw(matched.len());
    Some(matched[idx])
}

/// 问候行到 tweet id 的解析（源 `SetGreetingsTweetId` 的 WRT 一跳）。
///
/// 选取成功的行必然能解析（角色匹配已要求 WRT 在场）；独立调用时
/// WRT 缺失返回 `None`。
pub fn greeting_tweet_id(
    greeting: &GreetingRow,
    wrt: &[WithoutRelatedTalkRow],
) -> Option<i32> {
    wrt.iter()
        .find(|w| w.id == greeting.without_related_talk_id)
        .map(|w| w.tweet_id)
}

/// 摆设编辑反应链的选取：AEH 引用的 WRT → 角色过滤 → tweet id 池
/// → 主表解析 → 恰一次均匀抽签。
///
/// 池为空时源**静默跳过**（不显示也不报错）。主表里解析不到的
/// tweet id 不进候选——抽签分母是**可解析**条数，不是池大小。
/// 池内重复 id 只算一个候选（源按主表键去重）。候选顺序 = 主表顺序。
pub fn pick_after_edit_tweet<'a>(
    after_edit: &[AfterEditRow],
    wrt: &[WithoutRelatedTalkRow],
    tweets: &'a [TweetRow],
    character_unit_id: i32,
    rand: &mut impl UniformDraw,
) -> Option<&'a TweetRow> {
    let aeh_ids: HashSet<i32> = after_edit
        .iter()
        .map(|a| a.without_related_talk_id)
        .collect();
    let pool: HashSet<i32> = wrt
        .iter()
        .filter(|w| w.game_character_unit_id == character_unit_id && aeh_ids.contains(&w.id))
        .map(|w| w.tweet_id)
        .collect();
    let candidates: Vec<&TweetRow> = tweets.iter().filter(|t| pool.contains(&t.id)).collect();
    if candidates.is_empty() {
        return None;
    }
    let idx = rand.draw(candidates.len());
    Some(candidates[idx])
}

/// 站点入口链的选取：入口行引用的 WRT id 集 ∩ 该角色的 WRT 行 →
/// Any 门 → 恰一次均匀抽签 → tweet 解析。
///
/// 交集为空时**不抽签**（源先 Any 门后抽签）。抽中的 WRT 其 tweet
/// 在主表缺失时源记错误日志后跳过；本律此时已消费一次抽签并返回
/// `None`。候选顺序 = WRT 表顺序。
pub fn pick_site_entry_tweet<'a>(
    entries: &[SiteEntryRow],
    wrt: &[WithoutRelatedTalkRow],
    tweets: &'a [TweetRow],
    character_unit_id: i32,
    rand: &mut impl UniformDraw,
) -> Option<&'a TweetRow> {
    let entry_ids: HashSet<i32> = entries.iter().map(|e| e.without_related_talk_id).collect();
    let candidates: Vec<&WithoutRelatedTalkRow> = wrt
        .iter()
        .filter(|w| w.game_character_unit_id == character_unit_id && entry_ids.contains(&w.id))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let picked = candidates[rand.draw(candidates.len())];
    tweets.iter().find(|t| t.id == picked.tweet_id)
}

/// 对话前置 tweet 的确定链接：按对话 id 取前置行的 tweet id。
///
/// 源按对话 id 过滤后取首条；无前置行的对话不显示 tweet。零抽签。
pub fn talk_tweet_id(pre_actions: &[TalkPreActionRow], talk_id: i32) -> Option<i32> {
    pre_actions
        .iter()
        .find(|p| p.mysekai_character_talk_id == talk_id)
        .map(|p| p.tweet_id)
}

/// tweet 气泡的显示时长（秒）。
///
/// 源 TweetState 的更新序：`elapsed < 5.0` 走显示分支，`elapsed
/// <= 5.0` 提前返回——恰等于 5.0 的那一拍两支都不发生，收场须严格
/// 大于 5.0。
pub const TWEET_DISPLAY_SECONDS: f32 = 5.0;

/// 摆设编辑到反应 tweet 出场的延迟（秒）。源以 5.0 换算毫秒入延迟等待。
pub const AFTER_EDIT_REACTION_DELAY_SECONDS: f32 = 5.0;

/// 问候状态的时长（秒）。源静态数组 `(3.0, 5.0)` 的第 0 元：计时
/// 到达即完成状态。
pub const GREETING_STATE_SECONDS: f32 = 3.0;

/// 问候气球的最短在屏（秒）。同数组第 1 元：计时不超过它则不收气泡。
pub const GREETING_BALLOON_MIN_SECONDS: f32 = 5.0;

/// 玩家离角色多远中断问候（米）。
pub const GREETING_ABORT_DISTANCE: f32 = 5.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 脚本抽签：固定索引，并数调用次数——「恰一次求值」的量法。
    struct CountingDraw {
        index: usize,
        calls: usize,
        last_len: usize,
    }

    impl CountingDraw {
        fn new(index: usize) -> Self {
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

    fn tweet(id: i32, motion: Option<&str>, emoticon: Option<&str>) -> TweetRow {
        TweetRow {
            id,
            motion_name: motion.map(String::from),
            emoticon_name: emoticon.map(String::from),
            eye_name: "normal".to_string(),
            mouth_name: "smile01".to_string(),
            text: "synthetic".to_string(),
        }
    }

    fn wrt_row(id: i32, unit: i32, tweet_id: i32) -> WithoutRelatedTalkRow {
        WithoutRelatedTalkRow {
            id,
            game_character_unit_id: unit,
            tweet_id,
        }
    }

    fn visit_condition(id: i32, v1: i32, v2: i32) -> GreetingConditionRow {
        GreetingConditionRow {
            id,
            condition_type: CONDITION_TYPE_VISIT_COUNT.to_string(),
            value1: v1,
            value2: v2,
        }
    }

    // ---- 条件族 ----

    #[test]
    fn visit_count_interval_is_half_open_with_zero_upper_unbounded() {
        // 手算锚：v1=2, v2=5 ⇒ [2,5)
        let c = visit_condition(1, 2, 5);
        assert!(!condition_matches(&c, 1, 0)); // 低于下界
        assert!(condition_matches(&c, 2, 0)); // 恰在下界
        assert!(condition_matches(&c, 4, 0));
        assert!(!condition_matches(&c, 5, 0)); // 恰在上界：不含
        // v2 = 0 ⇒ 只有下界
        let unbounded = visit_condition(2, 2, 0);
        assert!(condition_matches(&unbounded, 100, 0));
        assert!(!condition_matches(&unbounded, 1, 0));
        // v1 = v2 = 0 ⇒ 恒真
        let always = visit_condition(3, 0, 0);
        assert!(condition_matches(&always, 0, 0));
        assert!(condition_matches(&always, 999, 0));
    }

    #[test]
    fn time_period_is_equality_on_value1() {
        let mut c = GreetingConditionRow {
            id: 1,
            condition_type: CONDITION_TYPE_TIME_PERIOD.to_string(),
            value1: 3,
            value2: 0,
        };
        assert!(condition_matches(&c, 7, 3));
        assert!(!condition_matches(&c, 7, 2));
        // value2 不参与时段型
        c.value2 = 9;
        assert!(condition_matches(&c, 7, 3));
    }

    #[test]
    fn unknown_condition_type_never_matches() {
        let c = GreetingConditionRow {
            id: 1,
            condition_type: "mysekai_something_else".to_string(),
            value1: 0,
            value2: 0,
        };
        assert!(!condition_matches(&c, 0, 0));
        assert!(!condition_matches(&c, 100, 100));
    }

    // ---- 问候链 ----

    #[test]
    fn greeting_pick_filters_character_and_condition_then_one_draw() {
        let greetings = vec![
            GreetingRow {
                id: 1,
                without_related_talk_id: 11,
                greeting_condition_id: 101,
            },
            GreetingRow {
                id: 2,
                without_related_talk_id: 12,
                greeting_condition_id: 101,
            },
        ];
        let wrt = vec![wrt_row(11, 7, 201), wrt_row(12, 8, 202)];
        let conditions = vec![visit_condition(101, 0, 0)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_greeting(&greetings, &wrt, &conditions, 7, 5, 0, &mut rand);
        assert_eq!(picked.map(|g| g.id), Some(1));
        assert_eq!(rand.calls, 1, "恰一次抽签");
        assert_eq!(rand.last_len, 1, "只有一行过筛");

        // 第二行也过筛时，抽中索引 1 得第二行
        let wrt_same_unit = vec![wrt_row(11, 7, 201), wrt_row(12, 7, 202)];
        let mut rand2 = CountingDraw::new(1);
        let picked2 = pick_greeting(&greetings, &wrt_same_unit, &conditions, 7, 5, 0, &mut rand2);
        assert_eq!(picked2.map(|g| g.id), Some(2));
        assert_eq!(rand2.last_len, 2);
    }

    #[test]
    fn greeting_empty_pool_is_none_without_drawing() {
        let greetings = vec![GreetingRow {
            id: 1,
            without_related_talk_id: 11,
            greeting_condition_id: 101,
        }];
        let wrt = vec![wrt_row(11, 8, 201)]; // 别的角色
        let conditions = vec![visit_condition(101, 0, 0)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_greeting(&greetings, &wrt, &conditions, 7, 5, 0, &mut rand);
        assert!(picked.is_none());
        assert_eq!(rand.calls, 0, "空池不抽签——源在抽签前抛异常");
    }

    #[test]
    fn greeting_condition_gap_excludes_row() {
        // 条件区间 [3,6)，访问 2：筛后空
        let greetings = vec![GreetingRow {
            id: 1,
            without_related_talk_id: 11,
            greeting_condition_id: 101,
        }];
        let wrt = vec![wrt_row(11, 7, 201)];
        let conditions = vec![visit_condition(101, 3, 6)];

        let mut rand = CountingDraw::new(0);
        assert!(pick_greeting(&greetings, &wrt, &conditions, 7, 2, 0, &mut rand).is_none());
        // 条件行缺失同样不匹配
        let mut rand2 = CountingDraw::new(0);
        assert!(
            pick_greeting(
                &greetings,
                &wrt,
                &[visit_condition(999, 0, 0)],
                7,
                5,
                0,
                &mut rand2
            )
            .is_none()
        );
    }

    #[test]
    fn greeting_wrt_missing_row_is_excluded() {
        // WRT 缺失 ⇒ 角色匹配失败（源记错误日志后剔除），不是崩溃
        let greetings = vec![GreetingRow {
            id: 1,
            without_related_talk_id: 99, // 不在 WRT 表
            greeting_condition_id: 101,
        }];
        let conditions = vec![visit_condition(101, 0, 0)];

        let mut rand = CountingDraw::new(0);
        assert!(pick_greeting(&greetings, &[], &conditions, 7, 5, 0, &mut rand).is_none());
    }

    #[test]
    fn greeting_tweet_id_resolves_through_wrt_row() {
        let greeting = GreetingRow {
            id: 1,
            without_related_talk_id: 11,
            greeting_condition_id: 101,
        };
        let wrt = vec![wrt_row(11, 7, 201)];
        assert_eq!(greeting_tweet_id(&greeting, &wrt), Some(201));
        assert_eq!(greeting_tweet_id(&greeting, &[]), None);
    }

    // ---- 摆设编辑反应链 ----

    #[test]
    fn after_edit_filters_unit_then_one_draw_over_master_order() {
        let after_edit = vec![
            AfterEditRow {
                id: 1,
                without_related_talk_id: 11,
            },
            AfterEditRow {
                id: 2,
                without_related_talk_id: 12,
            },
        ];
        let wrt = vec![wrt_row(11, 7, 301), wrt_row(12, 8, 302)];
        let tweets = vec![tweet(302, None, None), tweet(301, None, None), tweet(303, None, None)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_after_edit_tweet(&after_edit, &wrt, &tweets, 7, &mut rand);
        assert_eq!(picked.map(|t| t.id), Some(301));
        assert_eq!(rand.calls, 1, "恰一次抽签");
        assert_eq!(rand.last_len, 1, "分母 = 该角色可解析的池");
    }

    #[test]
    fn after_edit_unresolvable_tweet_ids_leave_the_denominator() {
        // 池 {401, 402}，主表只有 401 ⇒ 分母 1 而非 2
        let after_edit = vec![
            AfterEditRow {
                id: 1,
                without_related_talk_id: 11,
            },
            AfterEditRow {
                id: 2,
                without_related_talk_id: 12,
            },
        ];
        let wrt = vec![wrt_row(11, 7, 401), wrt_row(12, 7, 402)];
        let tweets = vec![tweet(401, None, None)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_after_edit_tweet(&after_edit, &wrt, &tweets, 7, &mut rand);
        assert_eq!(picked.map(|t| t.id), Some(401));
        assert_eq!(rand.last_len, 1, "解析不到的 id 不进分母");

        // 全部解析不到 ⇒ 空池静默跳过
        let tweets_empty = vec![tweet(999, None, None)];
        let mut rand2 = CountingDraw::new(0);
        assert!(pick_after_edit_tweet(&after_edit, &wrt, &tweets_empty, 7, &mut rand2).is_none());
        assert_eq!(rand2.calls, 0);
    }

    #[test]
    fn after_edit_duplicate_tweet_ids_collapse() {
        // 两条 AEH → 两条 WRT → 同一 tweet id ⇒ 一个候选
        let after_edit = vec![
            AfterEditRow {
                id: 1,
                without_related_talk_id: 11,
            },
            AfterEditRow {
                id: 2,
                without_related_talk_id: 12,
            },
        ];
        let wrt = vec![wrt_row(11, 7, 501), wrt_row(12, 7, 501)];
        let tweets = vec![tweet(501, None, None)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_after_edit_tweet(&after_edit, &wrt, &tweets, 7, &mut rand);
        assert_eq!(picked.map(|t| t.id), Some(501));
        assert_eq!(rand.last_len, 1, "重复 id 去重（源按主表键）");
    }

    #[test]
    fn after_edit_unit_absent_is_silent_skip() {
        let after_edit = vec![AfterEditRow {
            id: 1,
            without_related_talk_id: 11,
        }];
        let wrt = vec![wrt_row(11, 8, 601)];
        let tweets = vec![tweet(601, None, None)];

        let mut rand = CountingDraw::new(0);
        assert!(pick_after_edit_tweet(&after_edit, &wrt, &tweets, 7, &mut rand).is_none());
        assert_eq!(rand.calls, 0);
    }

    // ---- 站点入口链 ----

    #[test]
    fn site_entry_intersects_then_one_draw() {
        let entries = vec![
            SiteEntryRow {
                id: 1,
                without_related_talk_id: 11,
            },
            SiteEntryRow {
                id: 2,
                without_related_talk_id: 13,
            },
        ];
        let wrt = vec![wrt_row(11, 7, 701), wrt_row(12, 7, 702), wrt_row(13, 8, 703)];
        let tweets = vec![tweet(701, None, None), tweet(702, None, None)];

        let mut rand = CountingDraw::new(0);
        let picked = pick_site_entry_tweet(&entries, &wrt, &tweets, 7, &mut rand);
        assert_eq!(picked.map(|t| t.id), Some(701));
        assert_eq!(rand.calls, 1, "恰一次抽签");
        assert_eq!(rand.last_len, 1, "交集 = WRT11（12 未被入口引用，13 是别人的）");
    }

    #[test]
    fn site_entry_empty_intersection_skips_before_drawing() {
        // Any 门在抽签之前：空交集一次都不抽
        let entries = vec![SiteEntryRow {
            id: 1,
            without_related_talk_id: 13,
        }];
        let wrt = vec![wrt_row(11, 7, 701)];
        let tweets = vec![tweet(701, None, None)];

        let mut rand = CountingDraw::new(0);
        assert!(pick_site_entry_tweet(&entries, &wrt, &tweets, 7, &mut rand).is_none());
        assert_eq!(rand.calls, 0, "空交集不抽签");
    }

    #[test]
    fn site_entry_tweet_missing_consumes_the_draw_then_skips() {
        // 抽中后 tweet 解析不到：源记错误日志后跳过；抽签已发生
        let entries = vec![SiteEntryRow {
            id: 1,
            without_related_talk_id: 11,
        }];
        let wrt = vec![wrt_row(11, 7, 999)];
        let tweets = vec![tweet(701, None, None)];

        let mut rand = CountingDraw::new(0);
        assert!(pick_site_entry_tweet(&entries, &wrt, &tweets, 7, &mut rand).is_none());
        assert_eq!(rand.calls, 1, "抽签已消费");
    }

    // ---- 对话前置链接 ----

    #[test]
    fn talk_pre_action_is_a_deterministic_lookup() {
        let pre_actions = vec![
            TalkPreActionRow {
                mysekai_character_talk_id: 3912,
                tweet_id: 13912,
            },
            TalkPreActionRow {
                mysekai_character_talk_id: 4001,
                tweet_id: 14001,
            },
        ];
        assert_eq!(talk_tweet_id(&pre_actions, 3912), Some(13912));
        assert_eq!(talk_tweet_id(&pre_actions, 4001), Some(14001));
        assert_eq!(talk_tweet_id(&pre_actions, 5555), None, "无前置行 ⇒ 不显示");
    }

    #[test]
    fn timing_constants_match_source() {
        assert_eq!(TWEET_DISPLAY_SECONDS, 5.0);
        assert_eq!(AFTER_EDIT_REACTION_DELAY_SECONDS, 5.0);
        assert_eq!(GREETING_STATE_SECONDS, 3.0);
        assert_eq!(GREETING_BALLOON_MIN_SECONDS, 5.0);
        assert_eq!(GREETING_ABORT_DISTANCE, 5.0);
    }
}
