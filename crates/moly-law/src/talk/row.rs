//! talk 域的数据行：对话剧本与家具对话剧本两套提取形，加源主表行。
//!
//! [`TalkRow`] / [`TalkStep`] 对应单角色对话剧本的提取形；
//! [`FixtureTalkRow`] / [`FixtureStep`] 对应家具对话剧本的提取形——
//! 后者保留数字指称（角色单位 id，玩家占 0 号槽），前者已解析为
//! 角色名（`Characters.<Name>`，玩家为 `Characters.Player`）。两套形
//! 的字段名与值域都按提取产物原样镜像，文本字段（`text`、label 名、
//! tweet 文案）原样携带，不在本层解释。
//!
//! 家具步里偶见小数指称（如 0.5），是提取产物中的原样存在，照录
//! 不修——指称在步进律里是不透明值。

/// 源 `MasterMysekaiCharacterTalkCondition` 行：条件类型（服务端下发
/// 的字符串）与单值载荷。
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionRow {
    pub id: i32,
    pub condition_type: String,
    pub value: i32,
}

/// 源条件枚举 `MysekaiCharacterTalkConditionType` 的值域。类型在主表
/// 与提取产物里都是字符串；判别值按源枚举声明序钉死。
pub const CONDITION_READ_EVENT_STORY_EPISODE_ID: i32 = 0;
/// 现象 id 相等条件。
pub const CONDITION_MYSEKAI_PHENOMENA_ID: i32 = 1;
/// 家具 id 条件（家具对话全域的承重条件）。
pub const CONDITION_MYSEKAI_FIXTURE_ID: i32 = 2;
/// 家具标签条件。
pub const CONDITION_MYSEKAI_FIXTURE_TAG_ID: i32 = 3;
/// 角色访问次数条件。对话选取路径上此条件恒通过（语义与 tweet 问候
/// 族的半开区间条件不是同一族，见 [`crate::tweet::select`]）。
pub const CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT: i32 = 4;
/// 摆设后条件：条件门里**不计入**匹配（被忽略）。
pub const CONDITION_AFTER_SET_FIXTURE: i32 = 5;

/// 已知条件类型字符串到判别值的映射；未知字符串返回 `None`。
pub fn condition_type_discriminant(condition_type: &str) -> Option<i32> {
    match condition_type {
        "read_event_story_episode_id" => Some(CONDITION_READ_EVENT_STORY_EPISODE_ID),
        "mysekai_phenomena_id" => Some(CONDITION_MYSEKAI_PHENOMENA_ID),
        "mysekai_fixture_id" => Some(CONDITION_MYSEKAI_FIXTURE_ID),
        "mysekai_fixture_tag_id" => Some(CONDITION_MYSEKAI_FIXTURE_TAG_ID),
        "mysekai_character_visit_count" => Some(CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT),
        "after_set_fixture" => Some(CONDITION_AFTER_SET_FIXTURE),
        _ => None,
    }
}

/// 对话行内嵌的 tweet 引用：源主表的文案、动作段与眼/口图样键。
///
/// 源行允许动作段空引用；两套提取产物实测 0 条空值，按非空镜像。
#[derive(Debug, Clone, PartialEq)]
pub struct TweetRef {
    pub id: i32,
    pub text: String,
    pub motion: String,
    /// 眼图样键（facial 眼表 `PatternName`，同 `crate::tweet::face`）。
    pub eye: String,
    /// 口型键（facial 口型表 `Name`）。
    pub mouth: String,
}

/// 单角色对话剧本行（talks 提取形）。行键在全部 1412 行上同形。
///
/// `conditions` 是条件**类型字符串**表；`condition_values` 是提取产物的
/// 平行值键（同序、同长）——每条条件在源主表
/// `mysekaiCharacterTalkConditionTypeValue` 上的值，现象门比的就是这个
/// 字段。源主表上无值的条件导出 `None`（提取侧不造默认值）；无值臂在
/// 求值处按不匹配处理（fail-closed），不发明替代值。
#[derive(Debug, Clone, PartialEq)]
pub struct TalkRow {
    pub talk_id: i32,
    pub lua: String,
    pub site_group_id: i32,
    pub term_id: i32,
    pub conditions: Vec<String>,
    pub condition_values: Vec<Option<i32>>,
    pub tweet: TweetRef,
    pub voices: Vec<String>,
    pub steps: Vec<TalkStep>,
}

/// 家具×角色配对行：提取形 `pairs = [[fixtureId, unitId], ...]` 的
/// 展开。一份剧本按它的配对表归属到「某家具旁的某角色」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairRow {
    pub fixture_id: i32,
    pub unit_id: i32,
}

/// 家具对话剧本行（fixture-talks 提取形）。`unit_ids` 是参演回目，
/// 不含玩家（0 号槽）；`fixture_ids` 与 `pairs` 给出家具侧锚点。
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureTalkRow {
    pub talk_id: i32,
    pub lua: String,
    /// 静态 NoTalk 形态标：1 = 无对话窗车道（头顶 tweet + 表情气泡），
    /// 2 = 底部对话窗（玩家参与）。
    pub form: i32,
    pub site_group_id: i32,
    pub term_id: i32,
    pub condition_group_id: i32,
    pub fixture_ids: Vec<i32>,
    pub unit_ids: Vec<i32>,
    pub pairs: Vec<PairRow>,
    pub tweet: TweetRef,
    pub voices: Vec<String>,
    pub steps: Vec<FixtureStep>,
}

/// 单角色对话剧本的一步：源剧本库调用的原序扁平流，不推断运行时刻。
///
/// 各步的载荷字段与提取产物键一一对应。`text` 原样存字符串不解释。
#[derive(Debug, Clone, PartialEq)]
pub enum TalkStep {
    /// 面向对方转身，`duration` 是转身时长（秒）——发射即过，不驻留
    /// 流（源剧本库的等待形在本语料 0 次出现，见 [`crate::talk::step`]）。
    LookAtBody {
        who: String,
        target: String,
        duration: f64,
    },
    /// 定时等待。`auto` 是源第二参（点跳开关）的原样值；缺参为
    /// `None`。驻留语义见 [`crate::talk::step::Hold`]。
    WaitTime {
        seconds: f64,
        auto: Option<bool>,
    },
    /// 跳转锚：注册到对话视图的具名锚点。语料中无跳转调用。
    Label { name: String },
    Voice {
        channel: String,
        cue: String,
        who: String,
    },
    /// 眼图样切换。`pattern`/`alias` 是常量解析值与源记号对，键同
    /// facial 眼表（对齐 `crate::tweet::face` 的 `eye`）。
    ChangeNpcEye {
        who: String,
        pattern: String,
        alias: String,
        delay_seconds: f32,
    },
    /// 口型切换，键同 facial 口型表（对齐 `crate::tweet::face` 的
    /// `mouth`）。
    ChangeNpcMouth {
        who: String,
        pattern: String,
        alias: String,
        delay_seconds: f32,
    },
    /// 动作段切换。`speed` 为空表示源未传参（缺省 1.0）；换算见
    /// [`crate::talk::step::effective_animation_speed`]。
    ChangeAnimation {
        who: String,
        motion: String,
        alias: String,
        speed: Option<f64>,
        playback_speed: f64,
        play_end_motion: bool,
    },
    Text { text: String },
    /// 挂起直至点击。
    WaitClick,
    /// 头顶表情件。`show_seconds` 是源脚本排定的自动收回延迟（相对
    /// 本步时刻），收回与隐藏步两通道独立。
    Emoticon {
        who: String,
        name: String,
        alias: String,
        show_seconds: f64,
    },
    HideEmoticon { who: String },
    ShowTalkWindow,
    HideTalkWindow,
    /// 自动模式的定时等待（无点跳参数）。
    WaitTimeOnAutoMode { seconds: f64 },
}

/// 家具对话剧本的一步：指称是数字（角色单位 id / 家具 id），载荷新增
/// 家具操作族。步进语义与 [`TalkStep`] 同一流。
#[derive(Debug, Clone, PartialEq)]
pub enum FixtureStep {
    LookAtBody {
        who: f64,
        target: f64,
        duration: f64,
    },
    WaitTime {
        seconds: f64,
        auto: Option<bool>,
    },
    Label { name: String },
    Voice {
        channel: String,
        cue: String,
        who: f64,
    },
    ChangeNpcEye {
        who: f64,
        pattern: String,
        alias: String,
        delay_seconds: f32,
    },
    ChangeNpcMouth {
        who: f64,
        pattern: String,
        alias: String,
        delay_seconds: f32,
    },
    ChangeAnimation {
        who: f64,
        motion: String,
        alias: String,
        speed: Option<f64>,
        playback_speed: f64,
        play_end_motion: bool,
        /// 家具侧提取形多带的混合时长（秒）；单角色形无此键。
        blend: Option<f64>,
    },
    Text { text: String },
    WaitClick,
    Emoticon {
        who: f64,
        name: String,
        alias: String,
        show_seconds: f64,
    },
    HideEmoticon { who: f64 },
    ShowTalkWindow,
    HideTalkWindow,
    WaitTimeOnAutoMode { seconds: f64 },
    /// 面向家具。
    LookAtFixture { who: f64, fixture: f64 },
    /// 家具面向指定角色。
    LookAtToNpc {
        who: f64,
        fixture: f64,
        duration: f64,
    },
    /// 家具侧语音。
    FixtureVoice { cue: String, fixture: f64 },
    ChangeFixtureCharacterEye {
        fixture: f64,
        pattern: String,
        alias: String,
    },
    ChangeFixtureCharacterMouth {
        fixture: f64,
        pattern: String,
        alias: String,
    },
    /// 切换家具的时间轴贴图：`value` 是数值载荷（原样）。
    ChangeFixtureTimeline {
        fixture: f64,
        name: String,
        value: f64,
    },
    /// 家具上的表情件，自动收回语义同 [`TalkStep::Emoticon`]。
    ShowFixtureEmoticon {
        fixture: f64,
        name: String,
        alias: String,
        show_seconds: f64,
    },
    /// 播放家具机关。`fixture` 在源脚本里是机关名（字符串，非 id）。
    PlayFixtureGimmick { fixture: String },
    /// 停止家具机关。
    StopFixtureGimmick { fixture: String, name: f64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_type_domain_covers_all_six_values() {
        // 手算锚：六值域逐字对应源枚举声明序 0..=5
        assert_eq!(
            condition_type_discriminant("read_event_story_episode_id"),
            Some(CONDITION_READ_EVENT_STORY_EPISODE_ID)
        );
        assert_eq!(
            condition_type_discriminant("mysekai_phenomena_id"),
            Some(CONDITION_MYSEKAI_PHENOMENA_ID)
        );
        assert_eq!(
            condition_type_discriminant("mysekai_fixture_id"),
            Some(CONDITION_MYSEKAI_FIXTURE_ID)
        );
        assert_eq!(
            condition_type_discriminant("mysekai_fixture_tag_id"),
            Some(CONDITION_MYSEKAI_FIXTURE_TAG_ID)
        );
        assert_eq!(
            condition_type_discriminant("mysekai_character_visit_count"),
            Some(CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT)
        );
        assert_eq!(
            condition_type_discriminant("after_set_fixture"),
            Some(CONDITION_AFTER_SET_FIXTURE)
        );
        assert_eq!(condition_type_discriminant("mysekai_something_else"), None);
    }

    #[test]
    fn condition_discriminants_match_source_enum_order() {
        assert_eq!(CONDITION_READ_EVENT_STORY_EPISODE_ID, 0);
        assert_eq!(CONDITION_MYSEKAI_PHENOMENA_ID, 1);
        assert_eq!(CONDITION_MYSEKAI_FIXTURE_ID, 2);
        assert_eq!(CONDITION_MYSEKAI_FIXTURE_TAG_ID, 3);
        assert_eq!(CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT, 4);
        assert_eq!(CONDITION_AFTER_SET_FIXTURE, 5);
    }

    #[test]
    fn talk_row_round_trips_synthetic_payload() {
        // 合成形：两套行结构各过一遍字段
        let talk = TalkRow {
            talk_id: 3912,
            lua: "talk_3912".to_string(),
            site_group_id: 4,
            term_id: 1,
            conditions: vec![
                "mysekai_character_visit_count".to_string(),
                "mysekai_phenomena_id".to_string(),
            ],
            // 平行值键：同序同长，源主表无值的条件位是 None
            condition_values: vec![Some(1), Some(5)],
            tweet: TweetRef {
                id: 13912,
                text: "synthetic".to_string(),
                motion: "mov_a".to_string(),
                eye: "normal".to_string(),
                mouth: "smile01".to_string(),
            },
            voices: vec!["voice_cue".to_string()],
            steps: vec![
                TalkStep::LookAtBody {
                    who: "Characters.Player".to_string(),
                    target: "Characters.A".to_string(),
                    duration: 0.5,
                },
                TalkStep::WaitTime {
                    seconds: 0.5,
                    auto: Some(false),
                },
                TalkStep::Label {
                    name: "anchor".to_string(),
                },
                TalkStep::WaitClick,
                TalkStep::Text {
                    text: "synthetic".to_string(),
                },
                TalkStep::ChangeAnimation {
                    who: "Characters.A".to_string(),
                    motion: "mov_b".to_string(),
                    alias: "MovTable.mov_b".to_string(),
                    speed: None,
                    playback_speed: 1.0,
                    play_end_motion: false,
                },
                TalkStep::Emoticon {
                    who: "Characters.A".to_string(),
                    name: "fx_emote_001".to_string(),
                    alias: "fx_emote_001".to_string(),
                    show_seconds: 3.0,
                },
                TalkStep::WaitTimeOnAutoMode { seconds: 2.0 },
                TalkStep::ShowTalkWindow,
                TalkStep::HideTalkWindow,
                TalkStep::HideEmoticon {
                    who: "Characters.A".to_string(),
                },
                TalkStep::Voice {
                    channel: "talk".to_string(),
                    cue: "voice_cue".to_string(),
                    who: "Characters.A".to_string(),
                },
                TalkStep::ChangeNpcEye {
                    who: "Characters.A".to_string(),
                    pattern: "EyePresets.normal".to_string(),
                    alias: "EyePresets.normal".to_string(),
                },
                TalkStep::ChangeNpcMouth {
                    who: "Characters.A".to_string(),
                    pattern: "MouthPresets.smile01".to_string(),
                    alias: "MouthPresets.smile01".to_string(),
                },
            ],
        };
        assert_eq!(talk.steps.len(), 14);
        // 平行键契约：类型表与值表同序同长，逐位可 zip
        assert_eq!(talk.conditions.len(), talk.condition_values.len());
        let paired: Vec<(&str, Option<i32>)> = talk
            .conditions
            .iter()
            .zip(&talk.condition_values)
            .map(|(t, v)| (t.as_str(), *v))
            .collect();
        assert_eq!(
            paired,
            vec![
                ("mysekai_character_visit_count", Some(1)),
                ("mysekai_phenomena_id", Some(5)),
            ]
        );
        match &talk.steps[0] {
            TalkStep::LookAtBody { who, target, duration } => {
                assert_eq!(who, "Characters.Player");
                assert_eq!(target, "Characters.A");
                assert_eq!(*duration, 0.5);
            }
            other => panic!("unexpected step: {other:?}"),
        }
    }

    #[test]
    fn fixture_talk_row_carries_numeric_referents_and_pairs() {
        let fixture_talk = FixtureTalkRow {
            talk_id: 1,
            lua: "talk_0001".to_string(),
            form: 2,
            site_group_id: 4,
            term_id: 1,
            condition_group_id: 100005,
            fixture_ids: vec![5],
            unit_ids: vec![1],
            pairs: vec![PairRow {
                fixture_id: 5,
                unit_id: 1,
            }],
            tweet: TweetRef {
                id: 10001,
                text: "synthetic".to_string(),
                motion: "mov_c".to_string(),
                eye: "normal".to_string(),
                mouth: "smile01".to_string(),
            },
            voices: vec![],
            steps: vec![
                FixtureStep::LookAtBody {
                    who: 0.0,
                    target: 1.0,
                    duration: 0.5,
                },
                FixtureStep::LookAtFixture {
                    who: 1.0,
                    fixture: 5.0,
                },
                FixtureStep::LookAtToNpc {
                    who: 1.0,
                    fixture: 5.0,
                    duration: 0.5,
                },
                FixtureStep::FixtureVoice {
                    cue: "voice_fx".to_string(),
                    fixture: 5.0,
                },
                FixtureStep::ChangeFixtureCharacterEye {
                    fixture: 5.0,
                    pattern: "p".to_string(),
                    alias: "a".to_string(),
                },
                FixtureStep::ChangeFixtureCharacterMouth {
                    fixture: 5.0,
                    pattern: "p".to_string(),
                    alias: "a".to_string(),
                },
                FixtureStep::ChangeFixtureTimeline {
                    fixture: 5.0,
                    name: "tex".to_string(),
                    value: 1.0,
                },
                FixtureStep::ShowFixtureEmoticon {
                    fixture: 5.0,
                    name: "fx".to_string(),
                    alias: "fx".to_string(),
                    show_seconds: 2.0,
                },
                FixtureStep::PlayFixtureGimmick {
                    fixture: "gimmick_a".to_string(),
                },
                FixtureStep::StopFixtureGimmick {
                    fixture: "gimmick_a".to_string(),
                    name: 1.0,
                },
                FixtureStep::ChangeAnimation {
                    who: 1.0,
                    motion: "mov_d".to_string(),
                    alias: "a".to_string(),
                    speed: Some(0.0),
                    playback_speed: 1.0,
                    play_end_motion: true,
                    blend: Some(0.5),
                },
            ],
        };
        assert_eq!(fixture_talk.pairs.len(), 1);
        assert_eq!(fixture_talk.pairs[0].fixture_id, 5);
        assert_eq!(fixture_talk.pairs[0].unit_id, 1);
        assert_eq!(fixture_talk.form, 2);
        // 玩家占 0 号槽：指称 0.0 = Player
        match &fixture_talk.steps[0] {
            FixtureStep::LookAtBody { who, target, .. } => {
                assert_eq!(*who, 0.0);
                assert_eq!(*target, 1.0);
            }
            other => panic!("unexpected step: {other:?}"),
        }
    }
}
