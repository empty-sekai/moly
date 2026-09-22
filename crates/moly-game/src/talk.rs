//! Consumer of the existing fixture-script representation and its authored
//! character/fixture animation, face, voice and window operations.
//!
//! The player dispatcher supplies the final master, actors and exact optional
//! target fixture. This backend does not consume PlayerTalkRequest or select a
//! different story because furniture happens to be nearby. Complete loaded rows
//! remain available even when the current fixture candidate view is empty.
//! NPC Current/Previous and action restoration belong to their shared lifecycle;
//! finishing this window is not an AI-content reset.

use std::collections::HashMap;
use std::time::Duration;

use bevy::animation::graph::{AnimationGraph, AnimationNodeIndex};
use bevy::animation::AnimationPlayer;
use bevy::asset::{AssetPath, Assets, LoadState};
use bevy::ecs::system::SystemParam;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::path::{angle_between, facing_direction, turn_motion, TurnMotion};
use moly_law::talk::{
    advance, effective_animation_speed, is_finished, FaceSlot, FixtureStep, FixtureTalkRow,
    PairRow, StepOp, StreamState, TweetRef,
};

use crate::alone_action_runtime::{node_for, play_idle, FacialTables, EYE_CELL, MOUTH_CELL};
use crate::audio::{VoiceLine, VoiceSpeaker, VoiceWho};
use crate::character::{MotionDriver, MotionLibrary, SEGMENT_BLEND};
use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::delayed_faces::{DelayedFaces, FaceCommand};
use crate::emoticon::EmoticonArchive;
use crate::fixture::{FixturePlacement, FixturePlacements, FixtureRoot};
use crate::fixture_material::FixtureMaterial;
use crate::fixture_talk::{
    apply_face, fixture_node_for, FixtureAnimation, FixtureFace, FixtureTimelineSeq,
    FixtureTimelines, FixtureTurn,
};
use crate::npc::{CharacterUnitId, MotionPhase, Registry, WalkState};
use crate::player::PlayerControlled;
use crate::talk_window::{TalkSession, TalkWindowRoot, TalkWindowState};

/// 对话剧本表的资产路径（提取产物目录布局，内联路径——moly-assets
/// 没有这张表的取件函数，本单不碰那个 crate；待机动作表同款先例）。
const TALK_DATA: &str = "moly://fixture-talks/out/fixture-talks.json";

// ---------------------------------------------------------------------------
// 数据装载：fixture-talks.json → 对话律行类型
// ---------------------------------------------------------------------------

/// 对话剧本表的装载请求；解析成功即撤。
#[derive(Resource)]
pub(crate) struct TalkStoreHandle(Handle<JsonAsset>);

/// 解析后的全部剧本行（原样镜像）＋时间轴步点名的家具 id 集（对话
/// 演出的时间轴装载按它取「锚定 ∩ 语料点名」的白名单，见
/// `fixture_talk.rs`）。
#[derive(Resource)]
pub(crate) struct TalkStore {
    pub(crate) rows: Vec<FixtureTalkRow>,
    timeline_fixtures: Vec<i32>,
}

impl TalkStore {
    pub(crate) fn row(&self, master_id: i32) -> Option<&FixtureTalkRow> {
        self.rows.iter().find(|row| row.talk_id == master_id)
    }
    /// 时间轴步点名的家具 id 去重升序集。
    pub(crate) fn timeline_fixtures(&self) -> &[i32] {
        &self.timeline_fixtures
    }
}

/// 名册就绪后筛出的候选段＋候选文本字符集：配对选取只扫这一小撮，
/// 字符集并进气泡图集（候选段之外的字形不会上屏）。筛法＝参演表全
/// 在名册、锚定家具在摆放表里——静态预筛；配对侧（半径、八门现场）
/// 逐帧重求值（现场会变）。
#[derive(Resource)]
pub(crate) struct TalkCandidates {
    rows: Vec<FixtureTalkRow>,
    roster: Vec<u32>,
}

impl TalkCandidates {
    pub(crate) fn contains_talk(&self, master_id: i32) -> bool {
        self.rows.iter().any(|row| row.talk_id == master_id)
    }
}

/// 候选段文本字符集：`balloon::bake_atlas` 并进 tweet 主表字符集一起
/// 烘。字段对气泡模块开放。
#[derive(Resource)]
pub(crate) struct TalkCharset {
    pub(crate) chars: Vec<char>,
}

/// Startup：请求装载对话剧本表，落累计账本。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(TalkStoreHandle(
        server.load::<JsonAsset>(AssetPath::from(TALK_DATA.to_owned())),
    ));
    commands.init_resource::<TalkLedger>();
    commands.init_resource::<crate::delayed_faces::DelayedFaces>();
    // 表情件请求通道与累计账本同批落地（Default 资源，消费者逐帧读，
    // 缺了会在系统参数校验处 panic）。窗体状态机在窗体模块装载
    // （常驻——真源层对象常驻，对话只是 show/hide 它）。
    commands.init_resource::<TalkEmoteReqs>();
}

/// Update：剧本表到齐即解析。装载失败响亮 panic（资产边界的拒绝点）；
/// 报错只带字段名与段 id——文本内容不进任何报错。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<TalkStoreHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        warn!("[talk-ingest] fixture conversation asset unavailable: {err:?}");
        commands.insert_resource(TalkStore {
            rows: Vec::new(),
            timeline_fixtures: Vec::new(),
        });
        commands.remove_resource::<TalkStoreHandle>();
        return;
    }
    let Some(json) = jsons.get(&handle.0) else {
        return;
    };
    let (store, issues) = parse_talks(&json.0);
    for issue in &issues {
        warn!("[talk-ingest] quarantined fixture row: {issue}");
    }
    info!(
        "对话剧本表就绪：{} 段（pairs {} 对，步 {} 步）",
        store.rows.len(),
        store.rows.iter().map(|r| r.pairs.len()).sum::<usize>(),
        store.rows.iter().map(|r| r.steps.len()).sum::<usize>(),
    );
    commands.insert_resource(store);
    commands.remove_resource::<TalkStoreHandle>();
}

/// 解析入口：顶层 `talks` 数组逐行折成律行。步的 op 词表与律枚举一一
/// 对应；未知 op 响亮失败（提取词表外的新步型，静默跳过会把整段流播
/// 残）。
fn parse_talks(text: &str) -> (TalkStore, Vec<String>) {
    let mut issues = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            issues.push(format!("document is not valid JSON: {error}"));
            return (
                TalkStore {
                    rows: Vec::new(),
                    timeline_fixtures: Vec::new(),
                },
                issues,
            );
        }
    };
    let Some(talks) = value.get("talks").and_then(|v| v.as_array()) else {
        issues.push("document has no talks array".into());
        return (
            TalkStore {
                rows: Vec::new(),
                timeline_fixtures: Vec::new(),
            },
            issues,
        );
    };
    let mut rows = Vec::with_capacity(talks.len());
    for (index, row) in talks.iter().enumerate() {
        match crate::talk_ingest::fixture_row(row) {
            Ok(()) => rows.push(parse_row(row)),
            Err(reason) => issues.push(format!("source row {}: {reason}", index + 1)),
        }
    }
    // 时间轴步点名的家具 id 去重升序（装载白名单的语料侧）。
    let mut timeline_fixtures: Vec<i32> = rows
        .iter()
        .flat_map(|row| {
            row.steps.iter().filter_map(|step| match step {
                FixtureStep::ChangeFixtureTimeline { fixture, .. } => Some(*fixture as i32),
                _ => None,
            })
        })
        .collect();
    timeline_fixtures.sort_unstable();
    timeline_fixtures.dedup();
    (
        TalkStore {
            rows,
            timeline_fixtures,
        },
        issues,
    )
}

fn parse_row(row: &serde_json::Value) -> FixtureTalkRow {
    let talk_id = i_field(row, "talkId");
    FixtureTalkRow {
        talk_id,
        lua: s_field(row, "lua"),
        form: i_field(row, "form"),
        site_group_id: i_field(row, "siteGroupId"),
        term_id: i_field(row, "termId"),
        condition_group_id: i_field(row, "conditionGroupId"),
        fixture_ids: vec_field(row, "fixtureIds", talk_id),
        unit_ids: vec_field(row, "unitIds", talk_id),
        pairs: row
            .get("pairs")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("段 {talk_id} 缺 pairs 数组"))
            .iter()
            .map(|pair| {
                let pair = pair
                    .as_array()
                    .unwrap_or_else(|| panic!("段 {talk_id} 的 pair 行不是数组"));
                if pair.len() != 2 {
                    panic!("段 {talk_id} 的 pair 行不是两列：{pair:?}");
                }
                PairRow {
                    fixture_id: pair[0]
                        .as_i64()
                        .unwrap_or_else(|| panic!("段 {talk_id} 的 pair 首列不是数字"))
                        as i32,
                    unit_id: pair[1]
                        .as_i64()
                        .unwrap_or_else(|| panic!("段 {talk_id} 的 pair 次列不是数字"))
                        as i32,
                }
            })
            .collect(),
        tweet: parse_tweet_ref(row.get("tweet"), talk_id),
        voices: row
            .get("voices")
            .and_then(|v| v.as_array())
            .map(|vs| {
                vs.iter()
                    .map(|v| {
                        v.as_str()
                            .unwrap_or_else(|| panic!("段 {talk_id} 的 voices 行不是字符串"))
                            .to_owned()
                    })
                    .collect()
            })
            .unwrap_or_default(),
        steps: row
            .get("steps")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("段 {talk_id} 缺 steps 数组"))
            .iter()
            .map(parse_step)
            .collect(),
    }
}

/// tweet 引用解析。源主表允许动作段等字段空引用；单角色提取产物实测
/// 0 条空值（律按非空镜像），而本表 4768 行实测 motion 空 17 条。
/// 空引用折成空串：键不匹配任何动作段/图样，与源里「无引用＝不播」
/// 同效，不发明替代动作。
fn parse_tweet_ref(value: Option<&serde_json::Value>, talk_id: i32) -> TweetRef {
    let row = value.unwrap_or_else(|| panic!("段 {talk_id} 缺 tweet 引用"));
    let s_or_empty = |name: &str| -> String {
        row.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    TweetRef {
        id: i_field(row, "id"),
        text: s_or_empty("text"),
        motion: s_or_empty("motion"),
        eye: s_or_empty("eye"),
        mouth: s_or_empty("mouth"),
    }
}

/// 一步的解析：op 词表分派，载荷键与提取产物一一对应；数字载荷按
/// f64 原样收（指称在步进律里是不透明值，小数指称照录不修）。缺键的
/// 可选载荷（blend/playEndMotion/value/auto）按律的缺省折。
fn parse_step(step: &serde_json::Value) -> FixtureStep {
    let op = step
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("对话步缺 op：{step}"));
    let f = |name: &str| -> f64 {
        step.get(name)
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("{op} 步缺 {name}"))
    };
    let opt_f = |name: &str| -> Option<f64> { step.get(name).and_then(|v| v.as_f64()) };
    let s = |name: &str| -> String {
        step.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{op} 步缺 {name}"))
            .to_owned()
    };
    let opt_b = |name: &str| -> Option<bool> { step.get(name).and_then(|v| v.as_bool()) };
    match op {
        "look_at_body" => FixtureStep::LookAtBody {
            who: f("who"),
            target: f("target"),
            duration: f("duration"),
        },
        "wait_time" => FixtureStep::WaitTime {
            seconds: f("seconds"),
            auto: opt_b("auto"),
        },
        "label" => FixtureStep::Label { name: s("name") },
        "voice" => FixtureStep::Voice {
            channel: s("channel"),
            cue: s("cue"),
            who: f("who"),
        },
        "change_npc_eye" => FixtureStep::ChangeNpcEye {
            who: f("who"),
            pattern: s("pattern"),
            alias: s("alias"),
            delay_seconds: crate::delayed_faces::delay_seconds(step)
                .unwrap_or_else(|reason| panic!("{op}: {reason}")),
        },
        "change_npc_mouth" => FixtureStep::ChangeNpcMouth {
            who: f("who"),
            pattern: s("pattern"),
            alias: s("alias"),
            delay_seconds: crate::delayed_faces::delay_seconds(step)
                .unwrap_or_else(|reason| panic!("{op}: {reason}")),
        },
        "change_animation" => FixtureStep::ChangeAnimation {
            who: f("who"),
            motion: s("motion"),
            alias: s("alias"),
            speed: opt_f("speed"),
            playback_speed: opt_f("playbackSpeed").unwrap_or(1.0),
            play_end_motion: opt_b("playEndMotion").unwrap_or(false),
            blend: opt_f("blend"),
        },
        "text" => FixtureStep::Text { text: s("text") },
        "wait_click" => FixtureStep::WaitClick,
        "emoticon" => FixtureStep::Emoticon {
            who: f("who"),
            name: s("name"),
            alias: s("alias"),
            // showSeconds 键可缺（语料 468/919 步无此键）；同键在待机
            // 编排侧的解析已定约：缺＝0.0，0＝不自动收回（hide_at
            // None）。本表同键同约。
            show_seconds: opt_f("showSeconds").unwrap_or(0.0),
        },
        "hide_emoticon" => FixtureStep::HideEmoticon { who: f("who") },
        "show_talk_window" => FixtureStep::ShowTalkWindow,
        "hide_talk_window" => FixtureStep::HideTalkWindow,
        "wait_time_on_auto_mode" => FixtureStep::WaitTimeOnAutoMode {
            seconds: f("seconds"),
        },
        "look_at_fixture" => FixtureStep::LookAtFixture {
            // 提取键位与引擎签名错位（源脚本位 1 是角色、位 2 是时长）：
            // `fixture` 键实为**角色指称**，`who` 键实为**转身时长**。
            // who 键可缺（语料 122 步里 119 步无）；缺＝无时长（f64 无
            // Option 变体，NaN 占位——消费侧按 0 走瞬时转身，源里缺参
            // 经 lua_tonumber 读 0 同效）。
            who: opt_f("who").unwrap_or(f64::NAN),
            fixture: f("fixture"),
        },
        "look_at_to_npc" => FixtureStep::LookAtToNpc {
            who: f("who"),
            fixture: f("fixture"),
            duration: f("duration"),
        },
        "fixture_voice" => FixtureStep::FixtureVoice {
            cue: s("cue"),
            fixture: f("fixture"),
        },
        "change_fixture_character_eye" => FixtureStep::ChangeFixtureCharacterEye {
            fixture: f("fixture"),
            pattern: s("pattern"),
            alias: s("alias"),
        },
        "change_fixture_character_mouth" => FixtureStep::ChangeFixtureCharacterMouth {
            fixture: f("fixture"),
            pattern: s("pattern"),
            alias: s("alias"),
        },
        "change_fixture_timeline" => FixtureStep::ChangeFixtureTimeline {
            fixture: f("fixture"),
            name: s("name"),
            value: opt_f("value").unwrap_or(0.0),
        },
        "show_fixture_emoticon" => FixtureStep::ShowFixtureEmoticon {
            fixture: f("fixture"),
            name: s("name"),
            alias: s("alias"),
            // 同 emoticon：键可缺，缺＝0.0（不自动收回）。
            show_seconds: opt_f("showSeconds").unwrap_or(0.0),
        },
        "play_fixture_gimmick" => FixtureStep::PlayFixtureGimmick {
            fixture: s("fixture"),
        },
        "stop_fixture_gimmick" => FixtureStep::StopFixtureGimmick {
            fixture: s("fixture"),
            name: f("name"),
        },
        other => panic!("对话步的 op 未建模：{other}"),
    }
}

fn s_field(row: &serde_json::Value, name: &str) -> String {
    row.get(name)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("对话行缺 {name}"))
        .to_owned()
}

fn i_field(row: &serde_json::Value, name: &str) -> i32 {
    row.get(name)
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("对话行缺 {name}")) as i32
}

fn vec_field(row: &serde_json::Value, name: &str, talk_id: i32) -> Vec<i32> {
    row.get(name)
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("段 {talk_id} 缺 {name} 数组"))
        .iter()
        .map(|v| {
            v.as_i64()
                .unwrap_or_else(|| panic!("段 {talk_id} 的 {name} 行不是数字")) as i32
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 候选预筛与数据面核验
// ---------------------------------------------------------------------------

/// Update：名册、动作库、facial 表、表情件档案、家具摆放全部就绪后做
/// 一次性预筛与核验。核验失败响亮 panic——候选要点播的键不在表/库里
/// 是数据断点，不是运行时静默跳过（待机动作装配同款纪律）。
#[allow(clippy::type_complexity)]
pub(crate) fn prepare(
    mut commands: Commands,
    store: Option<Res<TalkStore>>,
    placements: Option<Res<FixturePlacements>>,
    registry: Option<Res<Registry>>,
    library: Option<Res<MotionLibrary>>,
    libraries: Res<Assets<Gltf>>,
    tables: Option<Res<FacialTables>>,
    archive: Option<Res<EmoticonArchive>>,
    mut prepared: Local<Option<(Vec<i32>, Vec<u32>)>>,
) {
    let (Some(store), Some(placements), Some(registry), Some(library), Some(tables), Some(archive)) =
        (store, placements, registry, library, tables, archive)
    else {
        return;
    };
    let Some(lib) = libraries.get(&library.gltf) else {
        return; // 动作库未到齐（有驱动即到齐，此行只是防序）
    };
    let placed = placements.fixture_ids();
    let roster = registry.character_unit_ids.clone();
    if prepared
        .as_ref()
        .is_some_and(|key| *key == (placed.clone(), roster.clone()))
    {
        return;
    }
    let mut rows = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    for row in &store.rows {
        collect_chars(&row.tweet.text, &mut chars);
        for step in &row.steps {
            match step {
                FixtureStep::Text { text } => collect_chars(text, &mut chars),
                FixtureStep::Label { name } => collect_chars(name, &mut chars),
                _ => {}
            }
        }
        let cast_in_roster = row
            .unit_ids
            .iter()
            .all(|&unit| roster.contains(&(unit as u32)));
        let anchored = row.fixture_ids.iter().any(|fid| placed.contains(fid));
        if !cast_in_roster || !anchored {
            continue;
        }
        for issue in verify_row(row, &tables, lib, &archive) {
            warn!("[talk-preflight] talk {} degraded: {issue}", row.talk_id);
        }
        rows.push(row.clone());
        collect_chars(&row.tweet.text, &mut chars);
        for step in &row.steps {
            match step {
                FixtureStep::Text { text } => collect_chars(text, &mut chars),
                // 名字栏文本也要字形：label 名的字符集一并进图集。
                FixtureStep::Label { name } => collect_chars(name, &mut chars),
                _ => {}
            }
        }
    }
    chars.sort_unstable();
    info!(
        "对话候选就绪：{} 段（名册 {:?}，锚定家具 {:?}），图集扩字符 {} 个；数据面核验全过",
        rows.len(),
        roster,
        placed,
        chars.len(),
    );
    *prepared = Some((placed, roster.clone()));
    commands.insert_resource(TalkCandidates { rows, roster });
    commands.insert_resource(TalkCharset { chars });
}

fn collect_chars(text: &str, chars: &mut Vec<char>) {
    for ch in text.chars() {
        if ch != '\n' && !chars.contains(&ch) {
            chars.push(ch);
        }
    }
}

/// 装载期核验一段：眼/口键在 facial 两表、动作段名在共享动作库、表情件
/// 名在档案。缺什么报什么（键名与段 id——不带资产内容）。
fn verify_row(
    row: &FixtureTalkRow,
    tables: &FacialTables,
    lib: &Gltf,
    archive: &EmoticonArchive,
) -> Vec<String> {
    let mut issues = Vec::new();
    for step in &row.steps {
        if let Some((slot, pattern, _)) = step.face_change() {
            let missing = match slot {
                FaceSlot::Eye => tables.eye_open(pattern).is_none(),
                FaceSlot::Mouth => tables.lip_close(pattern).is_none(),
            };
            if missing {
                issues.push(format!("{slot:?} preset {pattern:?} is unavailable"));
            }
        }
        match step {
            FixtureStep::ChangeAnimation { motion, .. } => {
                // 动作库里的剪辑带段族后缀（`_S`/`_L`/`_E`：起始/循环/
                // 结束——与待机动作族同一条分段约定）；对话步点播的是基
                // 名。核验与起播都按 `_S` 变体（起播段），循环族同基名
                // 必同在。
                let clip = format!("{motion}_S");
                if !lib.named_animations.contains_key(clip.as_str()) {
                    issues.push(format!("motion {motion:?} has no start clip"));
                }
            }
            FixtureStep::Emoticon { name, .. } => {
                if !archive.has(name) {
                    issues.push(format!("emoticon {name:?} is unavailable"));
                }
            }
            _ => {}
        }
    }
    issues
}

/// Playback counters only; previous-content truth belongs to the NPC AI slot.
#[derive(Resource, Default)]
pub(crate) struct TalkLedger {
    selections: u32,
    completed: u32,
}

/// 对话在播的会话（全局唯一）：段行、步进流状态、参演者、说话人跟踪
/// 与等待点击状态。
#[derive(Resource)]
pub(crate) struct ActiveTalk {
    /// Selected source pre-action gate; prevents body/voice racing source clips.
    preparing: bool,
    /// The dialogue closed; its source furniture Director is playing its tail.
    ending: bool,
    ending_tokens: Vec<crate::fixture_activity_timeline::TimelineToken>,
    effect_owner: Entity,
    talk_id: i32,
    form: i32,
    fixture_id: i32,
    fixtures: Vec<(i32, Entity)>,
    row: FixtureTalkRow,
    state: StreamState,
    participants: Vec<(u32, Entity)>,
    player: Option<Entity>,
    /// 当前说话人（`who` 指称跟踪；None＝玩家槽、悬空指称或步首）。
    speaker: Option<u32>,
    /// 挂起侧已记账的步型（避免逐帧重复记账）。
    hold_logged: Option<&'static str>,
    started_at: f32,
    steps_fired: usize,
    texts: usize,
    click_releases: usize,
}

impl ActiveTalk {
    pub(crate) fn body_ready(&self) -> bool {
        !self.preparing && !self.ending
    }
    pub(crate) fn includes_player(&self) -> bool {
        self.player.is_some()
    }

    /// Playback membership only, never a substitute for the NPC EnableTalk flag.
    pub(crate) fn has_participant(&self, unit: u32) -> bool {
        self.participants.iter().any(|(u, _)| *u == unit)
    }

    /// Template identity for authored fixture script operations.
    pub(crate) fn anchor_fixture_id(&self) -> i32 {
        self.fixture_id
    }

    fn fixture_entity(&self, fixture_id: i32) -> Option<Entity> {
        self.fixtures
            .iter()
            .find(|(master, _)| *master == fixture_id)
            .map(|(_, entity)| *entity)
    }

    /// 参演者清单（unit 与实体）——对话相机的取位面读它（真源取位表由
    /// 出场角色的髋变换构成，本链的出场角色就是参演者）。
    pub(crate) fn participants(&self) -> &[(u32, Entity)] {
        &self.participants
    }

    /// 会话的段 id（窗体日志的链名读取点用）。
    pub(crate) fn fixture_instances(&self) -> &[(i32, Entity)] {
        &self.fixtures
    }

    pub(crate) fn talk_id(&self) -> i32 {
        self.talk_id
    }

    /// Exact future voice commands from the authored cursor onward. This is a
    /// load hint only: command dispatch, StopVoiceAll and playback stay on the
    /// normal line-order path.
    pub(crate) fn voice_prefetches(&self) -> Vec<crate::audio::VoicePrefetch> {
        self.row
            .steps
            .iter()
            .skip(self.state.cursor)
            .filter_map(|step| match step {
                FixtureStep::Voice { cue, who, .. } => Some(crate::audio::VoicePrefetch::new(
                    cue.clone(),
                    partvoice_who(*who),
                )),
                FixtureStep::FixtureVoice { cue, fixture }
                    if fixture.is_finite()
                        && fixture.fract() == 0.0
                        && *fixture as i32 == self.fixture_id =>
                {
                    Some(crate::audio::VoicePrefetch::new(
                        cue.clone(),
                        Some(VoiceWho::Fixture(*fixture as i32)),
                    ))
                }
                _ => None,
            })
            .collect()
    }
}

/// 参演者身上的对话运行时：眼/口材质句柄、动作节点缓存、在播动作。
/// 与 [`TalkHold`]（让位标记，别的模块读）分开挂——本组件只归对话
/// 模块读写。
#[derive(Component)]
pub(crate) struct TalkRuntime {
    unit: u32,
    eye_handle: Handle<CharacterMaterial>,
    mouth_handle: Handle<CharacterMaterial>,
    nodes: HashMap<String, AnimationNodeIndex>,
    anim_node: Option<AnimationNodeIndex>,
}

/// 对话参演者标记：位移推进、待机动作演出、tweet 驻留触发、表情件的
/// 待机编排各自持本标记整名让位（各自模块的查询过滤）。无字段，公开
/// 类型名即可（`npc::advance` 是 pub 函数，查询里带着它）。
#[derive(Component)]
pub struct TalkHold;

pub(crate) struct TalkActor {
    pub(crate) unit: u32,
    pub(crate) entity: Entity,
    pub(crate) eye: Handle<CharacterMaterial>,
    pub(crate) mouth: Handle<CharacterMaterial>,
}

#[path = "fixture_talk_action.rs"]
pub(crate) mod fixture_action;

/// Start the existing fixture-script backend from the dispatcher's final row.
#[allow(clippy::type_complexity)]
pub(crate) fn start_selected(
    commands: &mut Commands,
    window: &mut TalkWindowState,
    row: FixtureTalkRow,
    actors: &[TalkActor],
    player: Entity,
    target_fixture: Option<Entity>,
    fixture_bindings: Vec<(i32, Entity)>,
    now: f32,
    prepare_source_action: bool,
) {
    crate::delayed_faces::start_loop(commands);
    commands.queue(|world: &mut World| {
        if let Some(mut ledger) = world.get_resource_mut::<TalkLedger>() {
            ledger.selections += 1;
        }
    });
    let talk_id = row.talk_id;
    let mut participants = Vec::with_capacity(actors.len());
    for actor in actors {
        commands
            .entity(actor.entity)
            .insert(TalkHold)
            .insert(TalkRuntime {
                unit: actor.unit,
                eye_handle: actor.eye.clone(),
                mouth_handle: actor.mouth.clone(),
                nodes: HashMap::new(),
                anim_node: None,
            })
            .insert(MotionPhase::Dwelling { remaining: None });
        participants.push((actor.unit, actor.entity));
    }
    let fixture_id = target_fixture
        .and_then(|target| {
            fixture_bindings
                .iter()
                .find(|(_, entity)| *entity == target)
                .map(|(fixture_id, _)| *fixture_id)
        })
        .or_else(|| row.pairs.first().map(|pair| pair.fixture_id))
        .unwrap_or(0);
    for (fixture_master, fixture) in &fixture_bindings {
        crate::fixture_talk::start_talk_effects(commands, *fixture, talk_id);
        info!(
            "[talk] master {talk_id} owns fixture instance {fixture:?} (master {fixture_master})"
        );
    }
    let mut talk = ActiveTalk {
        preparing: prepare_source_action && target_fixture.is_some(),
        ending: false,
        ending_tokens: Vec::new(),
        effect_owner: commands.spawn_empty().id(),
        talk_id,
        form: row.form,
        fixture_id,
        fixtures: fixture_bindings,
        row,
        state: StreamState::new(),
        participants,
        player: Some(player),
        speaker: None,
        hold_logged: None,
        started_at: now,
        steps_fired: 0,
        texts: 0,
        click_releases: 0,
    };
    commands
        .entity(player)
        .insert(TalkHold)
        .insert(MotionPhase::Dwelling { remaining: None });
    crate::player_talk::snapshot_player_rotation(commands, player, talk_id, None);
    if talk.preparing {
        window.prepare(TalkSession::Pair(&mut talk));
    } else {
        window.open(TalkSession::Pair(&mut talk));
    }
    info!(
        "[talk] prepared master {} starts with actors {:?}",
        talk_id,
        talk.participants
            .iter()
            .map(|(unit, _)| *unit)
            .collect::<Vec<_>>()
    );
    commands.insert_resource(talk);
}

// ---------------------------------------------------------------------------
// 合成对话注入口（冒烟）
// ---------------------------------------------------------------------------

/// 合成对话的默认行数：字典序前 3 条 talk-voice cue，演示行→cue→包→时长
/// 全链足够，也不把冒烟拖长。`MOLY_TALK_VOICE_PROBE_LINES` 可覆写行数
/// （逐行放行节奏的账目要更多行才推得开），非法值响亮告警后按默认。
const VOICE_PROBE_LINES_DEFAULT: usize = 3;

/// Explicit synthetic voice input; ordinary player requests use the dispatcher.
#[derive(Component)]
pub(crate) struct VoiceProbeGate;

/// Update：`MOLY_TALK_VOICE_PROBE_SECS` 给定秒数后，挂门牌让真实选取
/// 让行，等无会话在播时注入一个一次性合成段（show 窗体步 + N 条 voice
/// 行 + 点跳按窗体点击闩放行；N 默认 3，
/// `MOLY_TALK_VOICE_PROBE_LINES` 覆写），走与
/// 真对话同一条步进/分发/起播通路——talk voice
/// 的起播链冒烟就从这来（fixture-talks 语料的 voice cue 与提取语料零
/// 交集，真实对话全走缺 cue 支，那是另一条必验路径）。无该环境变量时
/// 注入口自关，零开销；变量的非法值响亮告警后同样关闭。
pub(crate) fn voice_probe(
    mut commands: Commands,
    time: Res<Time>,
    active: Option<Res<ActiveTalk>>,
    player_session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    routing: Option<Res<crate::audio::Routing>>,
    gates: Query<Entity, With<VoiceProbeGate>>,
    mut window: ResMut<TalkWindowState>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let secs = match std::env::var("MOLY_TALK_VOICE_PROBE_SECS") {
        Err(_) => {
            *done = true;
            return;
        }
        Ok(raw) => match raw.parse::<f32>() {
            Ok(secs) => secs,
            Err(_) => {
                warn!("[talk-voice] MOLY_TALK_VOICE_PROBE_SECS={raw:?} 不是数，注入口关闭");
                *done = true;
                return;
            }
        },
    };
    let Some(routing) = routing else {
        return; // 流表未就绪：等（不置 done，也不挂门牌）
    };
    if time.elapsed_secs() < secs {
        return;
    }
    let Some(gate) = gates.iter().next() else {
        commands.spawn(VoiceProbeGate);
        return; // 门牌挂起：下一帧起真实选取让行
    };
    // 门牌在场：等在播会话收口，槽一空即注入（唯一写者）。
    if active.is_some() || player_session.is_some() {
        return;
    }
    let lines = match std::env::var("MOLY_TALK_VOICE_PROBE_LINES") {
        Err(_) => VOICE_PROBE_LINES_DEFAULT,
        Ok(raw) => match raw.parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                warn!(
                    "[talk-voice] MOLY_TALK_VOICE_PROBE_LINES={raw:?} 不是正整数，按默认 {VOICE_PROBE_LINES_DEFAULT} 行"
                );
                VOICE_PROBE_LINES_DEFAULT
            }
        },
    };
    let cues: Vec<(String, String)> = routing.talk_voice_cues().into_iter().take(lines).collect();
    if cues.len() < lines {
        warn!(
            "[talk-voice] 合成对话注入口：流表 talk-voice cue 不足 {lines} 条（{}），不注入",
            cues.len(),
        );
        commands.entity(gate).despawn();
        *done = true;
        return;
    }
    let mut steps = vec![FixtureStep::ShowTalkWindow];
    for (cue, _) in &cues {
        steps.push(FixtureStep::Voice {
            channel: "talk".to_string(),
            cue: cue.clone(),
            who: 1.0,
        });
        steps.push(FixtureStep::WaitClick);
    }
    info!(
        "[talk-voice] 合成对话注入：{lines} 行（首条 {}），走步进/分发/起播同一条通路",
        ascii_or(&cues[0].0),
    );
    commands.entity(gate).despawn();
    let mut talk = ActiveTalk {
        preparing: false,
        ending: false,
        ending_tokens: Vec::new(),
        effect_owner: commands.spawn_empty().id(),
        talk_id: 0,
        form: 0,
        fixture_id: 0,
        fixtures: Vec::new(),
        row: FixtureTalkRow {
            talk_id: 0,
            lua: "voice-probe".to_string(),
            form: 0,
            site_group_id: 0,
            term_id: 0,
            condition_group_id: 0,
            fixture_ids: Vec::new(),
            unit_ids: Vec::new(),
            pairs: Vec::new(),
            tweet: TweetRef {
                id: 0,
                text: String::new(),
                motion: String::new(),
                eye: String::new(),
                mouth: String::new(),
            },
            voices: Vec::new(),
            steps,
        },
        state: StreamState::new(),
        participants: Vec::new(),
        player: None,
        speaker: None,
        hold_logged: None,
        started_at: time.elapsed_secs(),
        steps_fired: 0,
        texts: 0,
        click_releases: 0,
    };
    // 窗体开场同一条路（真实对话怎么开，合成段就怎么开）。
    window.open(TalkSession::Pair(&mut talk));
    crate::delayed_faces::start_loop(&mut commands);
    commands.insert_resource(talk);
    *done = true;
}

/// partvoice 注入口的门牌：与 [`VoiceProbeGate`] 同款让行语义，独立门牌
/// ——两个注入口互不误拆对方的门，并与两种正式后端共享唯一会话槽。
/// 调度链按序应用会话插入，后一个注入口等待已在播的会话结束。
#[derive(Component)]
pub(crate) struct PartVoiceProbeGate;

/// Update：`MOLY_PARTVOICE_PROBE_SECS` 给定秒数后注入一次性合成段，
/// 选材取真实语料（候选集行序首个命中，确定性），四臂：
/// ① mysekai 主链 voice 步（说话者→双包→流表）——起播臂；
/// ② 跨说话者缺 cue 臂——真 cue 配另一位真实说话者。真源链里包随
///    说话者变体（cue 名不参与路由），同一 cue 落进别的说话者的双包
///    两包皆缺→具名跳过（ExistsCueName miss 同款）。选材不依赖候选集
///    （该 cue 所在族的步只在锚定家具未摆放的段里）；
/// ③ 蛋 fixture_voice 步（合成段锚定选材自带的家具，源门按构造过）
///    ——单参包起播臂；
/// ④ 门拒说话者（who=1，名册 band 归属）配 mysekai cue——门拒具名臂
///   （真源对门拒者不载包，cue 属谁都不响）。
/// partvoice 路由表缺席时四臂全数 fail-closed 具名跳过——那正是无表
/// 不播的验收臂。无该环境变量时注入口自关，零开销；非法值响亮告警后
/// 同样关闭。
pub(crate) fn partvoice_probe(
    mut commands: Commands,
    time: Res<Time>,
    active: Option<Res<ActiveTalk>>,
    player_session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    candidates: Option<Res<TalkCandidates>>,
    gates: Query<Entity, With<PartVoiceProbeGate>>,
    mut window: ResMut<TalkWindowState>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let secs = match std::env::var("MOLY_PARTVOICE_PROBE_SECS") {
        Err(_) => {
            *done = true;
            return;
        }
        Ok(raw) => match raw.parse::<f32>() {
            Ok(secs) => secs,
            Err(_) => {
                warn!("[partvoice] MOLY_PARTVOICE_PROBE_SECS={raw:?} 不是数，注入口关闭");
                *done = true;
                return;
            }
        },
    };
    let Some(candidates) = candidates else {
        return; // 候选集未就绪：等（常驻资源，预筛落定后一直在）
    };
    if time.elapsed_secs() < secs {
        return;
    }
    let Some(gate) = gates.iter().next() else {
        commands.spawn(PartVoiceProbeGate);
        return; // 门牌挂起：下一帧起真实选取让行
    };
    // 门牌在场：等在播会话收口，槽一空即注入（唯一写者）。
    if active.is_some() || player_session.is_some() {
        return;
    }
    // 行序确定性选材：候选里首个 mysekai 主链 voice 步与首个蛋
    // fixture_voice 步。scenario 臂不走候选（见上）。
    let mut mysekai: Option<(String, f64)> = None;
    let mut egg: Option<(String, i32)> = None;
    for row in &candidates.rows {
        for step in &row.steps {
            match step {
                FixtureStep::Voice { cue, who, .. }
                    if cue.starts_with("partvoice_mysekai_") && mysekai.is_none() =>
                {
                    mysekai = Some((cue.clone(), *who));
                }
                FixtureStep::FixtureVoice { cue, fixture }
                    if cue.ends_with("_egg") && egg.is_none() =>
                {
                    egg = Some((cue.clone(), *fixture as i32));
                }
                _ => {}
            }
        }
        if mysekai.is_some() && egg.is_some() {
            break;
        }
    }
    let (Some((mysekai_cue, mysekai_who)), Some((egg_cue, egg_fixture))) = (&mysekai, &egg) else {
        warn!(
            "[partvoice] 合成对话注入口：候选里凑不齐选材（mysekai {} / 蛋 {}），不注入",
            mysekai.is_some(),
            egg.is_some(),
        );
        commands.entity(gate).despawn();
        *done = true;
        return;
    };
    // 缺 cue 臂选材：语料里的真 cue 配另一位真实说话者。真源链里包随
    // 说话者变体（cue 名不参与路由）——同一 cue 落进别的说话者的双包
    // 两包皆缺，具名跳过（ExistsCueName miss 静默跳过同款）。不依赖
    // 候选集（该 cue 所在族的步只在锚定家具未摆放的段里）。
    let scenario_cue = "partvoice_12_023";
    let scenario_who = 42.0;
    let voice = |cue: &str, who: f64| FixtureStep::Voice {
        channel: "talk".to_string(),
        cue: cue.to_string(),
        who,
    };
    let mut steps = vec![FixtureStep::ShowTalkWindow];
    // 臂①：mysekai 主链（说话者双包，流表命中→起播）。
    steps.push(voice(mysekai_cue, *mysekai_who));
    steps.push(FixtureStep::WaitClick);
    // 臂②：跨说话者缺 cue（cue 名不路由，说话者才路由——两包皆缺）。
    steps.push(voice(scenario_cue, scenario_who));
    steps.push(FixtureStep::WaitClick);
    // 臂③：蛋链（合成段的锚定家具就是选材步的家具，源门按构造过）。
    steps.push(FixtureStep::FixtureVoice {
        cue: egg_cue.clone(),
        fixture: *egg_fixture as f64,
    });
    steps.push(FixtureStep::WaitClick);
    // 臂④：门拒说话者（who=1，名册 band 归属——真源不载包）配 mysekai
    // cue：门拒在 cue 查表之前，cue 属谁都不该响。
    steps.push(voice(mysekai_cue, 1.0));
    steps.push(FixtureStep::WaitClick);
    info!(
        "[partvoice] 合成对话注入：四臂（mysekai {} · 跨说话者缺 cue {} · 蛋 {} · 门拒 who=1），走步进/分发/起播同一条通路",
        ascii_or(mysekai_cue),
        ascii_or(scenario_cue),
        ascii_or(egg_cue),
    );
    commands.entity(gate).despawn();
    let mut talk = ActiveTalk {
        preparing: false,
        ending: false,
        ending_tokens: Vec::new(),
        effect_owner: commands.spawn_empty().id(),
        talk_id: 0,
        form: 0,
        fixture_id: *egg_fixture, // 蛋臂的源门：步的家具 == 锚定家具
        fixtures: Vec::new(),
        row: FixtureTalkRow {
            talk_id: 0,
            lua: "partvoice-probe".to_string(),
            form: 0,
            site_group_id: 0,
            term_id: 0,
            condition_group_id: 0,
            fixture_ids: Vec::new(),
            unit_ids: Vec::new(),
            pairs: Vec::new(),
            tweet: TweetRef {
                id: 0,
                text: String::new(),
                motion: String::new(),
                eye: String::new(),
                mouth: String::new(),
            },
            voices: Vec::new(),
            steps,
        },
        state: StreamState::new(),
        participants: Vec::new(),
        player: None,
        speaker: None,
        hold_logged: None,
        started_at: time.elapsed_secs(),
        steps_fired: 0,
        texts: 0,
        click_releases: 0,
    };
    // 窗体开场同一条路（真实对话怎么开，合成段就怎么开）。
    window.open(TalkSession::Pair(&mut talk));
    crate::delayed_faces::start_loop(&mut commands);
    commands.insert_resource(talk);
    *done = true;
}

// ---------------------------------------------------------------------------
// 步进与事件分发
// ---------------------------------------------------------------------------

/// 表情件步折成的请求（表情件模块的帧推进消费——同一条出件/收件通
/// 道，不另起一套）。
#[derive(Resource, Default)]
pub(crate) struct TalkEmoteReqs {
    /// (宿主实体——角色参演者或家具根, 条目名, 出件秒数)。
    pub(crate) shows: Vec<(Entity, String, f32)>,
    /// 要收件的宿主实体。
    pub(crate) hides: Vec<Entity>,
}

/// 两种正文后端共用的播放面（图集 · 播放器 · 过渡 · 延迟表情时钟）。
/// 只有持有当前会话的后端推进时钟；另一个在取不到会话时直接返回。
/// 打包成一个参数：
/// 步进系统的裸参数已到 `SystemParam` 元组的上限（16），逐个展开会让
/// 整个系统静默失去 `IntoSystem`（错误只在 schedule 的 `.chain()` 处
/// 冒出来）。这不是组织偏好，是上限逼出来的收拢。
#[derive(SystemParam)]
pub(crate) struct TalkPlayback<'w, 's> {
    pub(crate) activity_timelines:
        Option<Res<'w, crate::fixture_activity_timeline::FixtureActivityTimelines>>,
    pub(crate) graphs: ResMut<'w, Assets<AnimationGraph>>,
    pub(crate) players: Query<'w, 's, &'static mut AnimationPlayer>,
    pub(crate) transitions: Query<'w, 's, &'static mut AnimationTransitions>,
    pub(crate) delayed_faces: ResMut<'w, DelayedFaces>,
    pub(crate) cancel: MessageReader<'w, 's, crate::player_talk::TalkCancelRequest>,
}

/// 步进分发的家具侧取数面（脸材质写 · 锚定摆放 · 玩家位 · 动画执行
/// 面），与 [`TalkPlayback`] 同因收拢；`look_at_to_npc` 的源式朝向差
/// 要读玩家位，时间轴步要读写动画执行面（剪辑节点缓存按名补进图）。
#[derive(SystemParam)]
pub(crate) struct FixtureSide<'w, 's> {
    pub(crate) materials: ResMut<'w, Assets<FixtureMaterial>>,
    pub(crate) fixtures: Query<
        'w,
        's,
        (
            Entity,
            &'static FixturePlacement,
            &'static Transform,
            Option<&'static FixtureFace>,
            Option<&'static mut FixtureAnimation>,
        ),
        With<FixtureRoot>,
    >,
    pub(crate) player_transforms: Query<'w, 's, &'static Transform, With<PlayerControlled>>,
}

/// Update：步进主循环——窗体点击闩、`advance` 一拍、逐
/// 触发步分发、对话动作播完交还、段收口。挂在选取与窗体输入之后
/// （会话当帧即可步进）。
#[allow(clippy::type_complexity)]
pub(crate) fn advance_talk(
    mut commands: Commands,
    time: Res<Time>,
    active: Option<ResMut<ActiveTalk>>,
    tables: Option<Res<FacialTables>>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    timelines: Res<FixtureTimelines>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut emote_reqs: ResMut<TalkEmoteReqs>,
    mut window: ResMut<TalkWindowState>,
    window_roots: Query<Entity, With<TalkWindowRoot>>,
    mut npcs: Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
    mut talk_runtimes: Query<&mut TalkRuntime>,
    mut playback: TalkPlayback,
    mut side: FixtureSide,
    mut prev: ResMut<TalkLedger>,
) {
    let explicit_cancel = crate::player_talk::drain_talk_cancellation(&mut playback.cancel);
    let Some(mut active) = active else {
        return;
    };
    // 拆成一条 &mut 再走字段：row（只读侧）与 state/账目（可写侧）是
    // 同一结构的互斥字段，经它取借用才分得开（经 ResMut 两次解引用则
    // 每次都是新借用，步进调用的两个实参撞车）。
    let active = &mut *active;
    let interrupted = active
        .participants
        .iter()
        .any(|(_, entity)| npcs.get(*entity).is_err())
        || active
            .fixtures
            .iter()
            .any(|(_, entity)| side.fixtures.get(*entity).is_err());
    if explicit_cancel || interrupted {
        if interrupted {
            warn!(
                "[talk] talk {} interrupted because an admitted actor or fixture disappeared",
                active.talk_id
            );
        }
        playback.delayed_faces.cancel_talk(active.talk_id);
        emote_reqs
            .hides
            .extend(active.participants.iter().map(|(_, entity)| *entity));
        emote_reqs
            .hides
            .extend(active.fixtures.iter().map(|(_, entity)| *entity));
        if let Some(tables) = tables.as_deref() {
            for (unit, entity) in &active.participants {
                if let Ok(runtime) = talk_runtimes.get(*entity) {
                    crate::player_talk::reset_default_face(
                        *unit,
                        tables,
                        &mut materials,
                        &runtime.eye_handle,
                        &runtime.mouth_handle,
                    );
                }
            }
        }
        if active.player.is_some() {
            if let Some(player) = active.player {
                crate::player_talk::restore_cancelled_player_rotation(
                    &mut commands,
                    player,
                    active.talk_id,
                );
            }
            crate::player_talk::reset_cancelled_player_status(&mut commands);
        }
        finish_talk(
            &mut commands,
            active,
            &mut window,
            &window_roots,
            &mut npcs,
            &mut playback.players,
            &mut playback.transitions,
            &mut prev,
            time.elapsed_secs(),
        );
        return;
    }
    if active.preparing {
        return;
    }
    if active.ending {
        let playing =
            playback
                .activity_timelines
                .as_ref()
                .is_some_and(|timelines| {
                    active.ending_tokens.iter().any(|token| {
                        timelines.status(*token).is_some_and(|status| matches!(status,
                    crate::fixture_activity_timeline::TimelineStatus::Preparing |
                    crate::fixture_activity_timeline::TimelineStatus::Playing { .. }))
                    })
                });
        if !playing {
            finish_talk(
                &mut commands,
                active,
                &mut window,
                &window_roots,
                &mut npcs,
                &mut playback.players,
                &mut playback.transitions,
                &mut prev,
                time.elapsed_secs(),
            );
        }
        return;
    }
    let Some(tables) = tables else {
        return;
    };
    let Some(lib) = libraries.get(&library.gltf) else {
        return;
    };
    let dt = time.delta_secs();
    // LoopUpdate yields before its first clock increment. Afterwards, due
    // callbacks precede this frame's Lua continuation, including while waiting.
    for command in playback.delayed_faces.advance(dt) {
        apply_delayed_face(active, command, &tables, &mut materials, &talk_runtimes);
    }
    // The fixture-script representation does not opt the player into auto
    // mode. WaitClick is released only by the window's input latch.
    // 放行门 = IsWaitClick：闩置位且打字机已收尾。
    let click = window.click_level();
    // 放行前快照：此刻是否挂在 wait_click 上（律的挂起域私有，按游标
    // 回看一步判——挂起记账同一条判法）。
    let holding_click = active.state.is_holding()
        && matches!(
            active.row.steps.get(active.state.cursor.saturating_sub(1)),
            Some(FixtureStep::WaitClick)
        );
    let holding_auto_wait = active.state.is_holding()
        && matches!(
            active.row.steps.get(active.state.cursor.saturating_sub(1)),
            Some(FixtureStep::WaitTimeOnAutoMode { .. })
        );

    // 一拍步进。
    let fired = match advance(&active.row.steps, &mut active.state, dt, click) {
        Ok(fired) => fired,
        Err(reason) => {
            warn!(
                "[talk] 段 {} 的步进被拒（{reason:?}）：时钟停住",
                active.talk_id
            );
            return;
        }
    };
    active.steps_fired += fired.len();
    if holding_click && click {
        // WaitClick has returned from the UI wait. Flush before dispatching
        // the next Lua statements, so their newly queued faces are not flushed.
        for command in playback.delayed_faces.wait_clicked() {
            apply_delayed_face(active, command, &tables, &mut materials, &talk_runtimes);
        }
        // 该击即耗尽：WaitClicked 放行后清闩（真源同款）。
        window.consume_click(TalkSession::Pair(&mut *active));
        active.click_releases += 1;
        // The next wait can be reached in this same frame. Announce that
        // edge once even when it has the same operation as the previous wait.
        active.hold_logged = None;
        info!(
            "[talk] 段 {} wait_click 放行（点击闩耗尽；IsWaitClick 门过）",
            active.talk_id
        );
    } else if holding_auto_wait && click {
        // The manual-mode race includes UI WaitClicked, which clears its latch.
        // Unlike the WaitClick Lua command, this branch does not flush faces.
        window.consume_click(TalkSession::Pair(&mut *active));
        active.hold_logged = None;
        info!(
            "[talk] 段 {} wait_time_on_auto_mode 手动跳过（点击闩耗尽）",
            active.talk_id
        );
    }

    // 挂起侧账：游标回看一步（等待步消费后游标已过它）；步型变化才记
    // 一行（挂起期间逐帧记账是刷屏不是证据）。
    if active.state.is_holding() {
        let parked = active.row.steps.get(active.state.cursor.saturating_sub(1));
        let word = parked.map(|step| step.op()).unwrap_or("<unknown>");
        if active.hold_logged != Some(word) {
            active.hold_logged = Some(word);
            match parked {
                Some(FixtureStep::WaitClick) => {
                    info!(
                        "[talk] 段 {} 步 {} 挂起 wait_click：等待窗体点击；无输入保持当前台词",
                        active.talk_id,
                        active.state.cursor - 1,
                    );
                }
                Some(FixtureStep::WaitTime { seconds, .. }) => {
                    info!(
                        "[talk] 段 {} 步 {} 驻留 wait_time {:.2}s",
                        active.talk_id,
                        active.state.cursor - 1,
                        seconds,
                    );
                }
                Some(FixtureStep::WaitTimeOnAutoMode { seconds }) => {
                    info!(
                        "[talk] 段 {} 步 {} 驻留 wait_time_on_auto_mode {:.2}s",
                        active.talk_id,
                        active.state.cursor - 1,
                        seconds,
                    );
                }
                _ => {}
            }
        }
    } else {
        active.hold_logged = None;
    }

    // 逐触发步分发。带 who 的步先更新说话人跟踪（每句的 voice/eye 步
    // 把指称切到说话者；玩家槽 0 与未解析小数指称＝悬空）。
    for &index in &fired {
        let step = active.row.steps[index].clone();
        if let Some(who) = referent_of(&step) {
            active.speaker = resolve_speaker(&*active, who, index);
        }
        // voice 行投递音频域（serve_voice 同帧起播）：真源逐行命令序里
        // voice 先于 text，起播挂点是行推进，不是窗体开场。partvoice 族
        // 的包随说话者变体，指称一并投递（voice_ 族不读它）。
        if let FixtureStep::Voice { cue, who, .. } = &step {
            // Resolve within this cast only. A repeated unit is ambiguous even
            // if the world contains some other actor with the same unit id.
            let speaker = active.speaker.and_then(|unit| {
                let mut actors = active
                    .participants
                    .iter()
                    .filter_map(|(candidate, entity)| (*candidate == unit).then_some(*entity));
                let entity = actors.next()?;
                actors.next().is_none().then_some(VoiceSpeaker::Npc(entity))
            });
            if speaker.is_none() {
                warn!(
                    "[talk] source={} step={} voice speaker={:?} has no unique current-cast entity; preserving audio routing without mouth binding",
                    active.talk_id, index, active.speaker,
                );
            }
            commands.spawn(VoiceLine {
                talk_id: active.talk_id,
                step: index,
                cue: cue.clone(),
                who: partvoice_who(*who),
                speaker,
            });
        }
        dispatch_step(
            &mut *active,
            &step,
            index,
            &tables,
            lib,
            &libraries,
            &timelines,
            &mut *playback.graphs,
            &mut playback.delayed_faces,
            &mut *side.materials,
            &mut window,
            &mut emote_reqs,
            &mut commands,
            &mut npcs,
            &mut side.fixtures,
            &side.player_transforms,
            &mut talk_runtimes,
            &mut playback.players,
            &mut playback.transitions,
        );
    }

    // 对话动作的播完交还：在播节点已终态即让位（位移驱动接管回待机）。
    release_finished_animations(
        &mut *active,
        &mut npcs,
        &mut talk_runtimes,
        &playback.players,
    );

    // 收口：全步消费且无挂起。
    if is_finished(active.row.steps.len(), &active.state) {
        // OnComplete executes due callbacks only. Future commands survive the
        // subsequent Dispose and can resolve against a later conversation.
        for command in playback.delayed_faces.on_complete() {
            apply_delayed_face(active, command, &tables, &mut materials, &talk_runtimes);
        }
        emote_reqs
            .hides
            .extend(active.participants.iter().map(|(_, entity)| *entity));
        emote_reqs
            .hides
            .extend(active.fixtures.iter().map(|(_, entity)| *entity));
        for (unit, entity) in &active.participants {
            if let Ok(runtime) = talk_runtimes.get(*entity) {
                crate::player_talk::reset_default_face(
                    *unit,
                    &tables,
                    &mut materials,
                    &runtime.eye_handle,
                    &runtime.mouth_handle,
                );
            }
        }
        close_talk_body(&mut commands, active, &mut window, &window_roots);
        active.ending = true;
        let owner = active.effect_owner;
        let actors: Vec<_> = active
            .participants
            .iter()
            .map(|(_, entity)| *entity)
            .collect();
        let fixtures: Vec<_> = active.fixtures.iter().map(|(_, entity)| *entity).collect();
        commands.queue(move |world: &mut World| {
            let tokens = world
                .get_resource_mut::<crate::fixture_activity_timeline::FixtureActivityTimelines>()
                .map(|mut timelines| timelines.request_talk_end(&actors, &fixtures))
                .unwrap_or_default();
            if let Some(mut talk) = world.get_resource_mut::<ActiveTalk>() {
                if talk.effect_owner == owner {
                    talk.ending_tokens = tokens;
                }
            }
        });
    }
}

/// 指称解析到说话人槽：0＝玩家槽（悬空，玩家域未接）；非整数＝悬空；
/// 整数且在参演表＝该 unit；整数但不在参演表＝悬空。
fn resolve_speaker(active: &ActiveTalk, who: f64, index: usize) -> Option<u32> {
    if who == 0.0 {
        return None;
    }
    if who.fract() != 0.0 {
        warn!(
            "[talk] 段 {} 步 {} 的指称 {} 不是整数槽：说话人悬空",
            active.talk_id, index, who
        );
        return None;
    }
    let unit = who as u32;
    if active.participants.iter().any(|(u, _)| *u == unit) {
        Some(unit)
    } else {
        None
    }
}

/// voice 步的 who 指称折成 partvoice 说话者：0＝玩家槽、非整数＝悬空
/// （与 [`resolve_speaker`] 同判，该处已有悬空告警），整数＝变体 id。
fn partvoice_who(who: f64) -> Option<VoiceWho> {
    (who != 0.0 && who.fract() == 0.0).then_some(VoiceWho::Participant(who as i32))
}

fn apply_delayed_face(
    active: &ActiveTalk,
    command: FaceCommand,
    tables: &FacialTables,
    materials: &mut Assets<CharacterMaterial>,
    runtimes: &Query<&mut TalkRuntime>,
) {
    let Some((_, entity)) = active
        .participants
        .iter()
        .find(|(unit, _)| *unit as i64 == command.unit)
    else {
        warn!(
            "[talk-face] source={} step={} unit={} is absent from current talk {}",
            command.source_talk, command.source_step, command.unit, active.talk_id,
        );
        return;
    };
    let Ok(runtime) = runtimes.get(*entity) else {
        return;
    };
    command.apply(
        tables,
        materials,
        &runtime.eye_handle,
        &runtime.mouth_handle,
    );
}

/// 一步的分发：按 op 落到呈现通道。日志只带 id/步号/指称/键名。
#[allow(clippy::too_many_arguments)]
fn dispatch_step(
    active: &mut ActiveTalk,
    step: &FixtureStep,
    index: usize,
    tables: &FacialTables,
    lib: &Gltf,
    libraries: &Assets<Gltf>,
    timelines: &FixtureTimelines,
    graphs: &mut Assets<AnimationGraph>,
    delayed_faces: &mut DelayedFaces,
    fixture_materials: &mut Assets<FixtureMaterial>,
    window: &mut TalkWindowState,
    emote_reqs: &mut TalkEmoteReqs,
    commands: &mut Commands,
    npcs: &mut Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
    fixtures: &mut Query<
        (
            Entity,
            &FixturePlacement,
            &Transform,
            Option<&FixtureFace>,
            Option<&mut FixtureAnimation>,
        ),
        With<FixtureRoot>,
    >,
    player_transforms: &Query<&Transform, With<PlayerControlled>>,
    talk_runtimes: &mut Query<&mut TalkRuntime>,
    players: &mut Query<&mut AnimationPlayer>,
    transitions: &mut Query<&mut AnimationTransitions>,
) {
    let talk_id = active.talk_id;
    let step_word = step.op();
    match step {
        FixtureStep::LookAtBody {
            who,
            target,
            duration,
        } => {
            let entity = participant_entity(active, *who);
            let target_entity = participant_entity(active, *target);
            match (entity, target_entity) {
                (Some(entity), Some(target_entity)) => {
                    let motion = turn_toward(entity, target_entity, *duration, npcs);
                    info!(
                        "[talk] 段 {} 步 {} look_at_body who={} target={}：转身 {:.2}s（{}）",
                        talk_id,
                        index,
                        who,
                        target,
                        duration,
                        motion.label(),
                    );
                }
                (Some(entity), None) if *target == 0.0 => {
                    if let Some(player) = active.player {
                        if let Ok(transform) = player_transforms.get(player) {
                            turn_toward_point(
                                entity,
                                transform.translation.to_array(),
                                *duration,
                                npcs,
                            );
                        }
                    }
                }
                (None, Some(target_entity)) if *who == 0.0 => {
                    if let Some(player) = active.player {
                        if let Ok((_, walk, _, _)) = npcs.get(target_entity) {
                            crate::player_talk::turn_player_to_point(
                                commands,
                                player,
                                walk.0.position,
                                *duration,
                            );
                        }
                    }
                }
                _ => {
                    info!(
                        "[talk] 段 {} 步 {} look_at_body who={} target={}：指称未在最终演员组中",
                        talk_id, index, who, target,
                    );
                }
            }
        }
        FixtureStep::WaitTime { .. }
        | FixtureStep::WaitTimeOnAutoMode { .. }
        | FixtureStep::WaitClick => {
            // 驻留步不进触发表（步进律：等待步挂起、即时步发射）；
            // 此处不可达，防御记账。
            warn!(
                "[talk] 段 {} 步 {} {step_word} 出现在触发表（步进律口径不符）",
                talk_id, index
            );
        }
        FixtureStep::Label { name } => {
            // 名字栏 SetText（真源 Label → 名字栏文本组件）——窗体开在这条
            // 链上。语料里锚点跳转的 label 步 0 出现，此支防御记账。
            window.set_label(TalkSession::Pair(&mut *active), name);
            info!(
                "[talk] 段 {} 步 {} label：名字栏 SetText（{} 字，名字不进日志）",
                talk_id,
                index,
                name.chars().count(),
            );
        }
        FixtureStep::Voice { .. } => {
            // 起播与缺 cue 账归音频域（serve_voice）——行推进处已投递，
            // 这里无事可做。
        }
        FixtureStep::ChangeNpcEye {
            who,
            pattern,
            delay_seconds,
            ..
        }
        | FixtureStep::ChangeNpcMouth {
            who,
            pattern,
            delay_seconds,
            ..
        } => {
            if !who.is_finite() || who.fract() != 0.0 {
                warn!(
                    "[talk] source={talk_id} step={index} {step_word} has a non-integral character id"
                );
                return;
            }
            let slot = if matches!(step, FixtureStep::ChangeNpcEye { .. }) {
                FaceSlot::Eye
            } else {
                FaceSlot::Mouth
            };
            // Capture identity now, not a current entity/material. Even a
            // zero delay must pass through the shared queue before writing.
            delayed_faces.enqueue(
                *delay_seconds,
                FaceCommand {
                    unit: *who as i64,
                    slot,
                    pattern: pattern.clone(),
                    source_talk: talk_id,
                    source_step: index,
                },
            );
        }
        FixtureStep::ChangeAnimation {
            who,
            motion,
            speed,
            playback_speed,
            play_end_motion,
            blend,
            ..
        } => {
            let entity = participant_entity(active, *who);
            let Some(entity) = entity else {
                info!(
                    "[talk] 段 {} 步 {} animation who={} motion={}：玩家侧/悬空指称，不起播",
                    talk_id,
                    index,
                    who,
                    ascii_or(motion),
                );
                return;
            };
            if *play_end_motion {
                warn!(
                    "[talk] 段 {} 步 {} 动作步带 playEndMotion=true：End 段跟随未实现，本步只播主段",
                    talk_id, index,
                );
            }
            let Some(mut runtime) = talk_runtimes.get_mut(entity).ok() else {
                return;
            };
            let unit = runtime.unit;
            let Ok((_, _, mut driver, _)) = npcs.get_mut(entity) else {
                warn!(
                    "[talk] 段 {} 步 {} animation who={}：成员不在名册驱动里，不起播",
                    talk_id, index, who
                );
                return;
            };
            // 起播段取 `_S` 变体（段族约定同待机动作族：基名＋后缀）。
            let node = node_for(
                &mut runtime.nodes,
                graphs,
                lib,
                &driver.graph,
                &format!("{motion}_S"),
                unit,
            );
            let mut player = players
                .get_mut(driver.player)
                .expect("装配系统应已插上播放器");
            let player = &mut *player;
            let transitions = transitions
                .get_mut(driver.player)
                .expect("装配系统应已插上过渡组件")
                .into_inner();
            let blend_seconds = blend
                .map(|b| b as f32)
                .unwrap_or_else(|| SEGMENT_BLEND.as_secs_f32());
            let animation = transitions.play(player, node, Duration::from_secs_f32(blend_seconds));
            // 速度换算走律（0 哨值读作 1.0）；playbackSpeed 载荷当前不
            // 进换算（源里它与 speed 并存、语义未取证），记日志对账。
            let effective = effective_animation_speed(*speed);
            animation.set_speed(effective as f32);
            runtime.anim_node = Some(node);
            // 播放器归演出侧（位移驱动让位同款约定）；playing 的位移
            // 簿记清空。
            driver.playing = None;
            driver.alone_holds = true;
            info!(
                "[talk] 段 {} 步 {} animation who={} motion={}：起播（速度 {:.2}（载荷 speed {:?}），混合 {:.2}s，playbackSpeed 载荷 {:.2}）",
                talk_id,
                index,
                who,
                ascii_or(motion),
                effective,
                speed,
                blend_seconds,
                playback_speed,
            );
        }
        FixtureStep::Text { text } => {
            // 正文进窗（ShowTextAsync 同款）：配对剧情与玩家对话共用这扇
            // 窗（真源窗体引擎服务全部引擎播的对话）；说话人指称不参与
            // 摆位（窗体是固定窗，正文只进打字机）。
            active.texts += 1;
            window.show_text(TalkSession::Pair(&mut *active), text);
            let speaker_word = match active.speaker {
                Some(unit) => format!("npc({unit})"),
                None => "悬空".to_owned(),
            };
            info!(
                "[talk] 段 {} 步 {} text：进窗体打字机（{} 字；说话人 {} 不参与摆位）",
                talk_id,
                index,
                text.chars().count(),
                speaker_word,
            );
        }
        FixtureStep::Emoticon {
            who,
            name,
            show_seconds,
            ..
        } => {
            let entity = participant_entity(active, *who);
            match entity {
                Some(entity) => {
                    emote_reqs
                        .shows
                        .push((entity, name.clone(), *show_seconds as f32));
                    info!(
                        "[talk] 段 {} 步 {} emoticon who={} name={} showSeconds={:.1}（出件走表情件通道）",
                        talk_id,
                        index,
                        who,
                        ascii_or(name),
                        show_seconds,
                    );
                }
                None => {
                    info!(
                        "[talk] 段 {} 步 {} emoticon who={} name={}：玩家侧/悬空指称，不出件",
                        talk_id,
                        index,
                        who,
                        ascii_or(name),
                    );
                }
            }
        }
        FixtureStep::HideEmoticon { who } => {
            let entity = participant_entity(active, *who);
            match entity {
                Some(entity) => {
                    emote_reqs.hides.push(entity);
                    info!(
                        "[talk] 段 {} 步 {} hide_emoticon who={}（收件走表情件通道）",
                        talk_id, index, who,
                    );
                }
                None => {
                    info!(
                        "[talk] 段 {} 步 {} hide_emoticon who={}：玩家侧/悬空指称，不收件",
                        talk_id, index, who,
                    );
                }
            }
        }
        FixtureStep::ShowTalkWindow => {
            // α 已 ≈1 时被 Approximately 短路（对话开场即在场——常例）；
            // 被 hide 过再 show 走 0.3s 淡入（与玩家链同一条路）。
            match window.show(TalkSession::Pair(&mut *active)) {
                Some(from) => {
                    info!(
                        "[talk] 段 {} 步 {} show_talk_window：α={from:.2} → 1 淡入 0.3s（OutQuad）",
                        talk_id, index
                    );
                }
                None => {
                    info!(
                        "[talk] 段 {} 步 {} show_talk_window：α≈1 即时（Approximately 短路，无淡入）",
                        talk_id, index
                    );
                }
            }
        }
        FixtureStep::HideTalkWindow => {
            window.hide(TalkSession::Pair(&mut *active));
            info!(
                "[talk] 段 {} 步 {} hide_talk_window：α→0 淡出 0.5s（OutQuad），窗体留场",
                talk_id, index
            );
        }
        FixtureStep::LookAtFixture { who, fixture } => {
            // 源式：角色转身面向锚定家具（引擎入参只有角色与时长——
            // 目标恒为会话锚定的那只家具）。提取键位错位：`fixture` 键
            // 是角色指称、`who` 键是时长（缺＝NaN＝瞬时，见步解析处）。
            let Some((_, _, fixture_transform, _, _)) =
                anchored_fixture(active, active.fixture_id, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} look_at_fixture：锚定家具 {} 不在摆放里，不转",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            match participant_entity(active, *fixture) {
                Some(entity) => {
                    let duration = if who.is_nan() { 0.0 } else { *who };
                    let motion = turn_toward_point(
                        entity,
                        fixture_transform.translation.to_array(),
                        duration,
                        npcs,
                    );
                    info!(
                        "[talk] 段 {} 步 {} look_at_fixture who={} fixture={}：角色 {} 转身面向锚定家具 {}（{:.2}s，{}）",
                        talk_id,
                        index,
                        who,
                        fixture,
                        *fixture as u32,
                        active.fixture_id,
                        duration,
                        motion.label(),
                    );
                }
                None => {
                    // 玩家槽（0 号）与悬空指称：玩家 Transform 的写者归
                    // 玩家域（独占），具名记日志不转。
                    info!(
                        "[talk] 段 {} 步 {} look_at_fixture who={} fixture={}：玩家侧/悬空指称（玩家域独占），记日志不转身",
                        talk_id, index, who, fixture,
                    );
                }
            }
        }
        FixtureStep::LookAtToNpc {
            who,
            fixture,
            duration,
        } => {
            // 源门：引擎按「锚定家具的 FixtureId == 步的 fixtureId」才
            // 转发。源式（家具侧）＝ 家具位置 − 角色位置 取水平分量做
            // 朝向，LookRotation 后按时长缓动转到位（直读，不翻转方向）。
            let Some((root_entity, _, fixture_transform, _, _)) =
                anchored_fixture(active, *fixture as i32, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} look_at_to_npc：锚定家具 {} 不在摆放里，不转",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            let target_pos = match participant_entity(active, *who) {
                Some(entity) => npcs
                    .get(entity)
                    .map(|(_, walk, _, _)| walk.0.position)
                    .unwrap_or([0.0, 0.0, 0.0]),
                None if *who == 0.0 => match player_transforms.single() {
                    Ok(transform) => transform.translation.to_array(),
                    Err(err) => {
                        warn!(
                            "[talk] 段 {} 步 {} look_at_to_npc：玩家位读取失败（{err}），不转",
                            talk_id, index,
                        );
                        return;
                    }
                },
                None => {
                    info!(
                        "[talk] 段 {} 步 {} look_at_to_npc who={} fixture={}：指称非参演者/玩家，不转",
                        talk_id, index, who, fixture,
                    );
                    return;
                }
            };
            let mut direction = fixture_transform.translation - Vec3::from(target_pos);
            direction.y = 0.0;
            if direction.length_squared() < 1e-12 {
                info!(
                    "[talk] 段 {} 步 {} look_at_to_npc who={}：家具与角色水平同位，朝向无定义，不转",
                    talk_id, index, who,
                );
                return;
            }
            let mut to = Quat::from_rotation_arc(Vec3::Z, direction.normalize());
            let from = fixture_transform.rotation;
            // 最短角路径：四元数双覆盖下取与起点同叶的那一枝（角色侧
            // 转身同一条）。
            if from.dot(to) < 0.0 {
                to = Quat::from_xyzw(-to.x, -to.y, -to.z, -to.w);
            }
            commands
                .entity(root_entity)
                .insert(FixtureTurn::toward(from, to, *duration));
            crate::fixture_talk::remember_talk_rotation(commands, root_entity, talk_id, from);
            info!(
                "[talk] 段 {} 步 {} look_at_to_npc who={} fixture={}：家具转向角色（+Z 沿「家具−角色」水平分量，源式直读）{:.2}s",
                talk_id, index, who, fixture, duration,
            );
        }
        FixtureStep::FixtureVoice { cue, fixture } => {
            // 源门：fixture_voice 只对锚定家具生效（步的家具 == 锚定家具；
            // 不等＝源侧静默无操作，无 else 无日志）。门过即投递音频域：
            // 真源同一 voice 单通道（StopVoiceAll 先行）、同一存在性门，
            // 蛋链无变体门——包随家具角色（单参，不拼 unit）。
            let speaker = active.fixture_entity(*fixture as i32).and_then(|target| {
                fixtures
                    .get(target)
                    .ok()
                    .map(|_| VoiceSpeaker::Fixture(target))
            });
            if speaker.is_none() {
                warn!(
                    "[talk] source={} step={} fixture_voice has no live explicit target={:?}; preserving audio routing without mouth binding",
                    talk_id, index, speaker,
                );
            }
            commands.spawn(VoiceLine {
                talk_id,
                step: index,
                cue: cue.clone(),
                who: (fixture.fract() == 0.0).then_some(VoiceWho::Fixture(*fixture as i32)),
                speaker,
            });
            info!(
                "[talk] 段 {} 步 {} fixture_voice cue={} fixture={} 投递音频域（蛋链单参包，路由见 partvoice 面）",
                talk_id,
                index,
                ascii_or(cue),
                fixture,
            );
        }
        FixtureStep::ChangeFixtureCharacterEye {
            fixture, pattern, ..
        } => {
            // 源闭包再核「锚定家具的 FixtureId == 步的 fixtureId」；表
            // 与角色侧同一张（PatternName → OpenEyeIndex），格下标**原样**
            // 进源式（不钳不移——角色侧的下标族是另一族，见
            // `fixture_talk.rs` 模块注释）。
            let Some((root_entity, _, _, face, _)) =
                anchored_fixture(active, *fixture as i32, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_character_eye：锚定家具 {} 不在摆放里，不写",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            let Some(face) = face else {
                // 无脸家具：源侧静默无操作（待机动画件为空）。
                info!(
                    "[talk] 段 {} 步 {} change_fixture_character_eye fixture={} pattern={}：无脸家具（源侧静默无操作），不写",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(pattern),
                );
                return;
            };
            if face.eyes.is_empty() {
                info!(
                    "[talk] 段 {} 步 {} change_fixture_character_eye fixture={} pattern={}：该家具无 eye 材质，不写",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(pattern),
                );
                return;
            }
            match tables.eye_open(pattern) {
                Some(open) => {
                    crate::fixture_talk::remember_talk_face(
                        commands,
                        root_entity,
                        talk_id,
                        &face.eyes,
                        fixture_materials,
                    );
                    let cell = apply_face(fixture_materials, &face.eyes, open, EYE_CELL);
                    info!(
                        "[talk] 段 {} 步 {} change_fixture_character_eye fixture={} pattern={}（open {} -> 格 {},{}，ST 源式原样）",
                        talk_id,
                        index,
                        fixture,
                        ascii_or(pattern),
                        open,
                        cell.0,
                        cell.1,
                    );
                }
                None => {
                    warn!(
                        "[talk] 段 {} 步 {} change_fixture_character_eye fixture={} pattern={}：facial 眼表缺键，不写",
                        talk_id,
                        index,
                        fixture,
                        ascii_or(pattern),
                    );
                }
            }
        }
        FixtureStep::ChangeFixtureCharacterMouth {
            fixture, pattern, ..
        } => {
            // 同 eye：lip 表 PatternName → CloseLipSyncIndex，格下标原样
            // 进源式。
            let Some((root_entity, _, _, face, _)) =
                anchored_fixture(active, *fixture as i32, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_character_mouth：锚定家具 {} 不在摆放里，不写",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            let Some(face) = face else {
                info!(
                    "[talk] 段 {} 步 {} change_fixture_character_mouth fixture={} pattern={}：无脸家具（源侧静默无操作），不写",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(pattern),
                );
                return;
            };
            if face.mouths.is_empty() {
                info!(
                    "[talk] 段 {} 步 {} change_fixture_character_mouth fixture={} pattern={}：该家具无 mouth 材质，不写",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(pattern),
                );
                return;
            }
            match tables.lip_close(pattern) {
                Some(close) => {
                    crate::fixture_talk::remember_talk_face(
                        commands,
                        root_entity,
                        talk_id,
                        &face.mouths,
                        fixture_materials,
                    );
                    let cell = apply_face(fixture_materials, &face.mouths, close, MOUTH_CELL);
                    info!(
                        "[talk] 段 {} 步 {} change_fixture_character_mouth fixture={} pattern={}（close {} -> 格 {},{}，ST 源式原样）",
                        talk_id,
                        index,
                        fixture,
                        ascii_or(pattern),
                        close,
                        cell.0,
                        cell.1,
                    );
                }
                None => {
                    warn!(
                        "[talk] 段 {} 步 {} change_fixture_character_mouth fixture={} pattern={}：facial 口型表缺键，不写",
                        talk_id,
                        index,
                        fixture,
                        ascii_or(pattern),
                    );
                }
            }
        }
        FixtureStep::ChangeFixtureTimeline {
            fixture,
            name,
            value,
        } => {
            // 源门（与 eye/mouth 同门）：引擎闭包再核「锚定家具的
            // FixtureId == 步的 fixtureId」。过门后按源语义换时间轴：
            // 具名时间轴 → 逐剪辑起播（顺序播放 + 旗标段循环，解析规则
            // 见 `fixture_talk.rs`）。`value` 是步的数值载荷（语料全 0，
            // 源把它当延迟秒传进延迟队列，0 = 即时）。
            let fixture_id = *fixture as i32;
            let Some((root_entity, _, _, _, animation)) =
                anchored_fixture(active, fixture_id, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline：锚定家具 {} 不在摆放里，不起播",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            let Some(plan) = timelines.plan(fixture_id, name) else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：时间轴不在可播集（剪辑解不到模型包或悬空引用），不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            let Some(animation) = animation else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：该家具没有动画执行面（解析期未解析到动画根），不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            let Some(player_entity) = animation.player else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：该家具模型无骨架动画根，不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            let Some(gltf) = libraries.get(&animation.gltf) else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：源 glb 不在资产集，不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            // 逐剪辑名 → 句柄 → 图节点（缓存按名）。缺名响亮拒绝：悬空
            // 引用族是源侧数据缺陷（两实现交叉证），不发明替身动作。
            let mut nodes = Vec::with_capacity(plan.clips.len());
            for clip_name in &plan.clips {
                let Some(clip) = gltf.named_animations.get(clip_name.as_str()) else {
                    warn!(
                        "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：剪辑 {} 不在 glb 动画集（悬空引用族），不起播",
                        talk_id,
                        index,
                        fixture,
                        ascii_or(name),
                        ascii_or(clip_name),
                    );
                    return;
                };
                nodes.push(fixture_node_for(animation, graphs, clip, clip_name));
            }
            let Ok(mut player) = players.get_mut(player_entity) else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：动画播放器不在（装载期已插），不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            let player = &mut *player;
            let Ok(transitions) = transitions.get_mut(player_entity) else {
                warn!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={}：动画过渡件不在（装载期已插），不起播",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                );
                return;
            };
            let transitions = transitions.into_inner();
            crate::fixture_talk::remember_talk_animation(
                commands,
                root_entity,
                talk_id,
                player_entity,
                nodes.clone(),
            );
            let first = transitions.play(
                player,
                nodes[0],
                Duration::from_secs_f32(plan.eases[0].max(0.0)),
            );
            if plan.loop_at == 0 {
                first.repeat();
                info!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={} value={}：起播 {}（{} 段，首段即循环）",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                    value,
                    ascii_or(&plan.clips[0]),
                    plan.clips.len(),
                );
            } else {
                info!(
                    "[talk] 段 {} 步 {} change_fixture_timeline fixture={} name={} value={}：起播 {}（{} 段顺序，第 {} 段起循环）",
                    talk_id,
                    index,
                    fixture,
                    ascii_or(name),
                    value,
                    ascii_or(&plan.clips[0]),
                    plan.clips.len(),
                    plan.loop_at + 1,
                );
                commands.entity(root_entity).insert(FixtureTimelineSeq::new(
                    nodes,
                    plan.eases.clone(),
                    plan.loop_at,
                ));
            }
        }
        FixtureStep::ShowFixtureEmoticon {
            fixture,
            name,
            show_seconds,
            ..
        } => {
            // 家具侧表情件走同一条出件通道（实例身份按实体键控，家具
            // 根直接可用；挂点解析归表情件模块）。
            let Some((root_entity, _, _, _, _)) =
                anchored_fixture(active, *fixture as i32, fixtures)
            else {
                warn!(
                    "[talk] 段 {} 步 {} show_fixture_emoticon：锚定家具 {} 不在摆放里，不折请求",
                    talk_id, index, active.fixture_id,
                );
                return;
            };
            emote_reqs
                .shows
                .push((root_entity, name.clone(), *show_seconds as f32));
            info!(
                "[talk] 段 {} 步 {} show_fixture_emoticon fixture={} name={} showSeconds={:.1}：折请求进表情件通道（对家具根）",
                talk_id,
                index,
                fixture,
                ascii_or(name),
                show_seconds,
            );
        }
        FixtureStep::PlayFixtureGimmick { fixture } => {
            let owner = active.effect_owner;
            if let Some(entity) = active.fixture_entity(active.fixture_id) {
                let source = fixture.clone();
                commands.queue(move |world: &mut World| {
                    if let Err(reason) = crate::fixture_gimmick::session::talk_trigger(
                        world, entity, owner, true, 0.,
                    ) {
                        warn!("[talk] gimmick {source} could not start: {reason}");
                        world.write_message(crate::player_talk::TalkCancelRequest);
                    }
                });
            }
        }
        FixtureStep::StopFixtureGimmick { name, .. } => {
            // The extractor's `name` operand is the authored stop delay.
            let owner = active.effect_owner;
            let delay = *name as f32;
            if let Some(entity) = active.fixture_entity(active.fixture_id) {
                commands.queue(move |world: &mut World| {
                    if let Err(reason) = crate::fixture_gimmick::session::talk_trigger(
                        world, entity, owner, false, delay,
                    ) {
                        warn!("[talk] gimmick stop could not be applied: {reason}");
                        world.write_message(crate::player_talk::TalkCancelRequest);
                    }
                });
            }
        }
    }
}

/// 锚定家具（会话 fixture_id 对应的摆放）：引擎 `_targetFixture` 的宿
/// 主对应物。会话选取已核锚点在摆放里，取不到是数据断点。末位是动画
/// 执行面（时间轴步用；无骨架家具 = None，多数摆件如此）。
fn anchored_fixture<'a>(
    active: &ActiveTalk,
    fixture_id: i32,
    fixtures: &'a mut Query<
        (
            Entity,
            &FixturePlacement,
            &Transform,
            Option<&FixtureFace>,
            Option<&mut FixtureAnimation>,
        ),
        With<FixtureRoot>,
    >,
) -> Option<(
    Entity,
    &'a FixturePlacement,
    &'a Transform,
    Option<&'a FixtureFace>,
    Option<&'a mut FixtureAnimation>,
)> {
    fixtures
        .iter_mut()
        .find(|(entity, placement, _, _, _)| {
            active.fixture_entity(fixture_id) == Some(*entity)
                || (active.player.is_none() && placement.fixture_id == fixture_id)
        })
        .map(|(entity, placement, transform, face, animation)| {
            (
                entity,
                placement,
                transform,
                face,
                animation.map(|a| a.into_inner()),
            )
        })
}

/// 步的指称载荷（`who`；无指称的步为 None）。`look_at_fixture` 例外：
/// 它的指称在 `fixture` 键上（提取键位错位——`who` 键实为转身时长，
/// 见步解析处）。
fn referent_of(step: &FixtureStep) -> Option<f64> {
    match step {
        FixtureStep::LookAtBody { who, .. }
        | FixtureStep::Voice { who, .. }
        | FixtureStep::ChangeNpcEye { who, .. }
        | FixtureStep::ChangeNpcMouth { who, .. }
        | FixtureStep::ChangeAnimation { who, .. }
        | FixtureStep::Emoticon { who, .. }
        | FixtureStep::HideEmoticon { who, .. }
        | FixtureStep::LookAtToNpc { who, .. } => Some(*who),
        FixtureStep::LookAtFixture { fixture, .. } => Some(*fixture),
        _ => None,
    }
}

/// 指称 → 参演者实体（非参演指称 None：玩家槽 0 或不在本段的成员）。
fn participant_entity(active: &ActiveTalk, referent: f64) -> Option<Entity> {
    if referent.fract() != 0.0 {
        return None;
    }
    let unit = referent as u32;
    active
        .participants
        .iter()
        .find(|(u, _)| *u == unit)
        .map(|(_, entity)| *entity)
}

/// 转身：从当前旋转（Transform 现值——转体中途的连环转身从当前角起
/// 算）到面向目标成员——选段按折角差（转身律），终点取同叶短弧（位
/// 移换腿的 depart 同一条式）。时长用步载荷（对话步自带时长；位移侧
/// 的除 60 估时不适用这里）。相位进转体（动画侧读它播转身段），逐帧
/// 推进归本模块（位移推进被持留跳过）。
fn turn_toward(
    entity: Entity,
    target: Entity,
    duration: f64,
    npcs: &mut Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
) -> TurnMotion {
    let target_pos = npcs
        .get(target)
        .map(|(_, walk, _, _)| walk.0.position)
        .unwrap_or([0.0, 0.0, 0.0]);
    turn_toward_point(entity, target_pos, duration, npcs)
}

/// 转身到一点（目标位直接给定——家具目标不在名册里，没有 `WalkState`
/// 可读，位置从它的 Transform 来）。其余同 [`turn_toward`]。
fn turn_toward_point(
    entity: Entity,
    target_pos: [f32; 3],
    duration: f64,
    npcs: &mut Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
) -> TurnMotion {
    let from = npcs
        .get(entity)
        .map(|(_, _, _, transform)| transform.rotation)
        .unwrap_or_default();
    let own_pos = npcs
        .get(entity)
        .map(|(_, walk, _, _)| walk.0.position)
        .unwrap_or([0.0, 0.0, 0.0]);
    let to_forward = facing_direction(own_pos, target_pos);
    let angle = angle_between([from * Vec3::Z][0].to_array(), to_forward);
    let motion = turn_motion(angle);
    let mut to = Quat::from_rotation_arc(Vec3::Z, Vec3::from(to_forward));
    // 最短角路径：四元数双覆盖下取与起点同叶的那一枝（depart 同一条）。
    if from.dot(to) < 0.0 {
        to = Quat::from_xyzw(-to.x, -to.y, -to.z, -to.w);
    }
    if let Ok((mut phase, _, _, _)) = npcs.get_mut(entity) {
        *phase = MotionPhase::Turning {
            motion,
            from,
            to,
            duration: duration as f32,
            elapsed: 0.0,
        };
    }
    motion
}

/// 对话动作播完交还播放器（位移驱动接管，回待机段）。
fn release_finished_animations(
    active: &mut ActiveTalk,
    npcs: &mut Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
    talk_runtimes: &mut Query<&mut TalkRuntime>,
    players: &Query<&mut AnimationPlayer>,
) {
    let participants = active.participants.clone();
    for (unit, entity) in participants {
        let Ok(mut runtime) = talk_runtimes.get_mut(entity) else {
            continue;
        };
        let Some(node) = runtime.anim_node else {
            continue;
        };
        let player_entity = npcs
            .get(entity)
            .map(|(_, _, driver, _)| driver.player)
            .expect("参演者的驱动应已装配");
        let done = players
            .get(player_entity)
            .map(|player| {
                player
                    .playing_animations()
                    .any(|(n, animation)| *n == node && animation.is_finished())
            })
            .unwrap_or(false);
        if !done {
            continue;
        }
        runtime.anim_node = None;
        if let Ok((_, _, mut driver, _)) = npcs.get_mut(entity) {
            driver.alone_holds = false;
            driver.playing = None;
        }
        info!(
            "[talk] 段 {} unit={unit} 对话动作播完：播放器交还位移驱动（回待机）",
            active.talk_id
        );
    }
}

/// 段收口：撤持留与运行时、撤窗体（层弹出无淡出，与玩家链同一条路）、
/// 相位回驻留、待机归位、转体未完的成员把朝向钉到转身终点（律状态
/// forward 同步——撤持留后行走不回跳）、记账与收口日志。
#[allow(clippy::too_many_arguments)]
fn finish_talk(
    commands: &mut Commands,
    active: &mut ActiveTalk,
    window: &mut TalkWindowState,
    window_roots: &Query<Entity, With<TalkWindowRoot>>,
    npcs: &mut Query<(
        &mut MotionPhase,
        &mut WalkState,
        &mut MotionDriver,
        &Transform,
    )>,
    players: &mut Query<&mut AnimationPlayer>,
    transitions: &mut Query<&mut AnimationTransitions>,
    prev: &mut ResMut<TalkLedger>,
    now: f32,
) {
    if !active.ending {
        close_talk_body(commands, active, window, window_roots);
    }
    let owner = active.effect_owner;
    commands.queue(move |world: &mut World| {
        fixture_action::cancel_owner(world, Some(owner));
        crate::fixture_gimmick::session::finish_owner(world, owner);
        if let Ok(entity) = world.get_entity_mut(owner) {
            entity.despawn();
        }
    });
    for (_, fixture) in &active.fixtures {
        crate::fixture_talk::finish_talk_effects(commands, *fixture, active.talk_id);
    }
    if let Some(player) = active.player {
        if let Ok(mut entity) = commands.get_entity(player) {
            entity
                .remove::<TalkHold>()
                .remove::<crate::player_talk::PlayerTurn>()
                .remove::<crate::player_talk::PlayerTalkRotationSnapshot>();
        }
    }
    for (unit, entity) in active.participants.clone() {
        // 转体未完：朝向钉到转身终点（相位里的 `to`），律状态 forward
        // 同步——下一次转身从当前朝向起算，行走不回跳。
        let turn_end = npcs
            .get(entity)
            .ok()
            .and_then(|(phase, _, _, _)| match phase {
                MotionPhase::Turning { to, .. } => Some(*to),
                _ => None,
            });
        if let Some(to) = turn_end {
            let fwd = to * Vec3::Z;
            if let Ok((_, mut walk, _, _)) = npcs.get_mut(entity) {
                walk.0.forward = [fwd.x, fwd.y, fwd.z];
            }
        }
        if let Ok((mut phase, _, mut driver)) = npcs.get_mut(entity).map(|(p, w, d, _)| (p, w, d)) {
            *phase = MotionPhase::Dwelling { remaining: None };
            driver.alone_holds = false;
            driver.playing = None;
            play_idle(&mut driver, players, transitions, unit);
        }
        if let Ok(mut entity_commands) = commands.get_entity(entity) {
            entity_commands.remove::<TalkHold>().remove::<TalkRuntime>();
            drop(entity_commands);
            crate::npc::leave_player_talk(commands, entity);
        }
    }
    // 段收口日志。
    info!(
        "[talk] 段收口：段 id={}（form {}，家具 {}，成员 {:?}）播完——步事件 {}、文本 {}、wait_click 放行 {}（窗体点击闩），用时 {:.1}s；AI Current/Previous保持",
        active.talk_id,
        active.form,
        active.fixture_id,
        active
            .participants
            .iter()
            .map(|(u, _)| *u)
            .collect::<Vec<_>>(),
        active.steps_fired,
        active.texts,
        active.click_releases,
        now - active.started_at,
    );
    prev.completed += 1;
    commands.remove_resource::<ActiveTalk>();
}

fn close_talk_body(
    commands: &mut Commands,
    active: &mut ActiveTalk,
    window: &mut TalkWindowState,
    window_roots: &Query<Entity, With<TalkWindowRoot>>,
) {
    // Dispose stops the delay loop, retaining future callbacks for the next talk.
    crate::delayed_faces::stop_loop(commands);
    crate::audio::dispose_talk_voice(commands);
    for root in window_roots {
        commands.entity(root).despawn();
    }
    window.close(TalkSession::Pair(active));
    info!("[talkwin] dialogue closed; source activity ending remains owned");
}

// ---------------------------------------------------------------------------
// 转身逐帧推进（参演者持留期间，位移推进被跳过）
// ---------------------------------------------------------------------------

/// Update：参演者的转体逐帧走——位置冻结，朝向按步时长插值（缓动同
/// 位移转体的补间缺省：先快后慢的二次出线），完成帧相位回驻留并把
/// 终点朝向回写律状态。只碰持留者（`TalkRuntime` 在身＝对话在播）。
pub(crate) fn progress_turns(
    time: Res<Time>,
    mut npcs: Query<(&mut MotionPhase, &mut WalkState, &mut Transform), With<TalkRuntime>>,
) {
    let dt = time.delta_secs();
    for (mut phase, mut walk, mut transform) in &mut npcs {
        let MotionPhase::Turning {
            from,
            to,
            duration,
            elapsed,
            ..
        } = &mut *phase
        else {
            continue;
        };
        *elapsed += dt;
        let t = (*elapsed / duration.max(1e-6)).clamp(0.0, 1.0);
        // 缓动取补间库缺省（位移转体同一条式）。
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        transform.rotation = from.slerp(*to, eased);
        if *elapsed >= *duration {
            // 完成：朝向钉在终点，律状态 forward 同步。
            let fwd = *to * Vec3::Z;
            walk.0.forward = [fwd.x, fwd.y, fwd.z];
            *phase = MotionPhase::Dwelling { remaining: None };
            info!("[talk] 转体完成（对话侧）：朝向回写，相位回驻留");
        }
    }
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

/// Update（周期 5s）：会话状态行——「对话真的在播」的现算证据（步进
/// 游标、文本数、点跳放行数），加累计账（配对检出、选取、收口）。
pub(crate) fn report(
    time: Res<Time>,
    active: Option<Res<ActiveTalk>>,
    prev: Res<TalkLedger>,
    mut tick: Local<f32>,
) {
    *tick += time.delta_secs();
    if *tick < 5.0 {
        return;
    }
    *tick = 0.0;
    match active {
        Some(active) => {
            info!(
                "[talk] 状态：段 id={}（form {}）进行中——步 {}/{}（触发 {}）、文本 {}、wait_click 放行 {}、说话人槽 {:?}；累计：选取 {} 收口 {}",
                active.talk_id,
                active.form,
                active.state.cursor,
                active.row.steps.len(),
                active.steps_fired,
                active.texts,
                active.click_releases,
                active.speaker,
                prev.selections,
                prev.completed,
            );
        }
        None => {
            info!(
                "[talk] 状态：无会话在播；累计：选取 {} 收口 {}",
                prev.selections, prev.completed,
            );
        }
    }
}

/// 日志守卫：非 ASCII 内容换占位符（同气泡模块的网关纪律）。
fn ascii_or(value: &str) -> &str {
    if value.is_ascii() {
        value
    } else {
        "<non-ascii>"
    }
}
