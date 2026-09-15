//! Player SD interaction: one dispatcher selects source content, then starts one
//! of the existing script backends. The NPC AI owns Current/Previous, action
//! identity and Before; a player window never writes its own previous-talk truth.
//!
//! Appearance, click qualification and content routing are separate. General
//! performs its lottery before CurrentGeneral overwrite; shared Previous fallback
//! resolves against complete loaded rows, not the weather/roster candidate view.
//! Ordinary Common is prepared by the objective factory before the click.
//! Factories that have not supplied a master/activity remain explicitly pending.
//!
//! Existing text, facial, motion, VoiceLine, window and camera consumers stay in
//! place. Talk enters through the NPC lifecycle, ends the old Rest script and
//! stops navigation without inventing arrival or resetting the AI goal.

use std::collections::{HashMap, HashSet};

use bevy::animation::graph::{AnimationGraph, AnimationNodeIndex};
use bevy::animation::AnimationPlayer;
use bevy::asset::{AssetPath, Assets, LoadState};
use bevy::ecs::system::SystemParam;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::path::{angle_between, facing_direction, turn_motion, TurnMotion};
use moly_law::talk::{
    advance, condition_group_matches, condition_type_discriminant, effective_animation_speed,
    is_finished, ConditionContext, FaceSlot, StepOp, StreamState, TalkRow, TalkStep, TweetRef,
    UniformDraw, CONDITION_AFTER_SET_FIXTURE, CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT,
    CONDITION_MYSEKAI_PHENOMENA_ID, CONDITION_READ_EVENT_STORY_EPISODE_ID,
};

use crate::alone_action_runtime::{
    apply_eye, apply_mouth_pattern, node_for, play_idle, FacialTables,
};
use crate::audio::{VoiceLine, VoiceSpeaker, VoiceWho};
use crate::character::{MotionDriver, MotionLibrary, SEGMENT_BLEND};
use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::delayed_faces::{DelayedFaces, FaceCommand};
use crate::emoticon::EmoticonArchive;
use crate::fixture::{FixturePlacement, FixtureRoot};
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::npc::{CharacterUnitId, MotionPhase, Registry, WalkState};
use crate::player::PlayerControlled;
use crate::talk::{TalkEmoteReqs, TalkHold};
use crate::talk_window::{TalkSession, TalkWindowRoot, TalkWindowState};

/// 玩家对话剧本表的资产路径（提取产物目录布局，内联路径同配对对话侧
/// 先例）。
const PLAYER_TALK_DATA: &str = "moly://talks.json";

/// 站点主表的资产路径：站点门要 `siteType → 站点 id` 的对照，主表行
/// 自带 id 列，装载期建表（不硬编码对照表）。
const SITES_DATA: &str = "moly://site/sites.json";

/// 站点组 → 站点 id 集合：服务端主表 `mysekaiSiteGroups` 的具名 mock
/// （值由服务端下发，按范围通则做成可改参数）。产物当前只引用组 1
/// 与 4，四组全录——组表是主表事实，不是本域的选取结果。
const SITE_GROUP_SITES: [(i32, &[i32]); 4] = [
    (1, &[1]),
    (2, &[2, 3, 4]),
    (3, &[5, 6, 7, 8]),
    (4, &[1, 2, 3, 4]),
];

/// 冒烟合成的点按节奏（秒）。
const SMOKE_TAP_INTERVAL: f32 = 0.8;

/// 拾取链 NPC 沿发出的玩家对话请求（命中实体 + 角色单位 id；消费侧
/// 按实体取位并核对 unit——同帧链内实体必在，核不上即响亮拒绝）。
#[derive(Debug, Clone, Message)]
pub(crate) struct PlayerTalkRequest {
    pub(crate) entity: Entity,
    pub(crate) unit: u32,
    pub(crate) exact: Option<crate::npc_objective::TalkContent>,
    pub(crate) target_fixture: Option<Entity>,
}

#[derive(Debug, Clone, Copy, Message)]
pub(crate) struct TalkCancelRequest;

pub(crate) fn drain_talk_cancellation(reader: &mut MessageReader<TalkCancelRequest>) -> bool {
    reader.read().next().is_some()
}

pub(crate) fn reset_cancelled_player_status(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        let Some(mut states) = world.get_resource_mut::<crate::player_state::PlayerAvatarStates>()
        else {
            return;
        };
        if states.current == crate::player_state::PlayerActionState::Talk {
            states.change_status(crate::player_state::PlayerActionState::Idle);
        }
    });
}

// ---------------------------------------------------------------------------
// 数据装载
// ---------------------------------------------------------------------------

/// 装载请求（剧本表 + 站点主表）；两表都到齐即解析，成功即撤。
#[derive(Resource)]
pub(crate) struct PlayerTalkDataHandle {
    talks: Handle<JsonAsset>,
    sites: Handle<JsonAsset>,
}

/// 解析后的全部剧本行（按 unit 分桶，原样镜像）＋装载期可导出的指称
/// 名表（名 → unit）。
#[derive(Resource)]
pub(crate) struct PlayerTalkStore {
    pub(crate) units: Vec<(u32, Vec<TalkRow>)>,
    pub(crate) names: HashMap<String, u32>,
}

/// A resolved row retains one of the two existing script representations.
#[derive(Clone)]
pub(crate) enum ResolvedTalk {
    General { unit: u32, row: TalkRow },
    Fixture(moly_law::talk::FixtureTalkRow),
}

impl ResolvedTalk {
    pub(crate) fn content(&self) -> crate::npc_objective::TalkContent {
        use crate::npc_objective::{TalkBackend, TalkContent};
        match self {
            Self::General { row, .. } => TalkContent {
                master_id: row.talk_id,
                backend: TalkBackend::General,
                is_general: Some(is_general_row(row)),
            },
            // The fixture product retains a condition-group id, not the
            // group's predicates. Its backend name is not an IsGeneral result.
            Self::Fixture(row) => TalkContent {
                master_id: row.talk_id,
                backend: TalkBackend::Fixture,
                is_general: None,
            },
        }
    }

    pub(crate) fn pre_action(&self) -> TweetRef {
        match self {
            Self::General { row, .. } => row.tweet.clone(),
            Self::Fixture(row) => row.tweet.clone(),
        }
    }

    fn units(&self) -> Vec<u32> {
        match self {
            Self::General { unit, .. } => vec![*unit],
            Self::Fixture(row) => row.unit_ids.iter().map(|unit| *unit as u32).collect(),
        }
    }
}

/// Shared read-only catalog: candidates never replace the complete loaded rows.
#[derive(SystemParam)]
pub(crate) struct TalkCatalog<'w> {
    store: Option<Res<'w, PlayerTalkStore>>,
    fixtures: Option<Res<'w, crate::talk::TalkStore>>,
    sites: Option<Res<'w, PlayerTalkSites>>,
    phenomena: Res<'w, crate::weather::CurrentPhenomenonId>,
}

pub(crate) struct GeneralSelection {
    pub(crate) master_id: i32,
    pub(crate) pool_len: usize,
    pub(crate) replayed: bool,
}

pub(crate) fn is_general_row(row: &TalkRow) -> bool {
    row.conditions.iter().any(|condition| {
        matches!(
            condition_type_discriminant(condition),
            Some(
                CONDITION_READ_EVENT_STORY_EPISODE_ID
                    | CONDITION_MYSEKAI_PHENOMENA_ID
                    | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT
            )
        )
    })
}

impl TalkCatalog<'_> {
    pub(crate) fn ready(&self) -> bool {
        self.store.is_some() && self.sites.is_some()
    }

    fn site_id(&self, site_type: &str) -> Option<i32> {
        self.sites
            .as_deref()?
            .by_type
            .iter()
            .find(|(kind, _)| kind == site_type)
            .map(|(_, id)| *id)
    }

    pub(crate) fn resolve_content(
        &self,
        content: &crate::npc_objective::TalkContent,
    ) -> Option<ResolvedTalk> {
        match content.backend {
            crate::npc_objective::TalkBackend::General => {
                for (unit, rows) in &self.store.as_deref()?.units {
                    if let Some(row) = rows.iter().find(|row| row.talk_id == content.master_id) {
                        return Some(ResolvedTalk::General {
                            unit: *unit,
                            row: row.clone(),
                        });
                    }
                }
                None
            }
            crate::npc_objective::TalkBackend::Fixture => self
                .fixtures
                .as_deref()?
                .row(content.master_id)
                .cloned()
                .map(ResolvedTalk::Fixture),
        }
    }

    pub(crate) fn resolve(&self, master_id: i32) -> Option<ResolvedTalk> {
        if let Some(store) = self.store.as_deref() {
            for (unit, rows) in &store.units {
                if let Some(row) = rows.iter().find(|row| row.talk_id == master_id) {
                    return Some(ResolvedTalk::General {
                        unit: *unit,
                        row: row.clone(),
                    });
                }
            }
        }
        self.fixtures
            .as_deref()?
            .row(master_id)
            .cloned()
            .map(ResolvedTalk::Fixture)
    }

    fn name(&self, unit: u32) -> String {
        self.store
            .as_deref()
            .and_then(|store| {
                store
                    .names
                    .iter()
                    .find(|(_, member)| **member == unit)
                    .map(|(name, _)| name.clone())
            })
            .unwrap_or_default()
    }

    /// The source lottery runs even if PlayGeneralTalk subsequently overwrites
    /// the result with CurrentGeneral. Previous fallback bypasses every filter.
    pub(crate) fn lottery(
        &self,
        unit: u32,
        site_type: &str,
        previous: Option<i32>,
        mut draw: impl FnMut(usize) -> usize,
    ) -> Result<GeneralSelection, &'static str> {
        let store = self
            .store
            .as_deref()
            .ok_or("ordinary talk data is not loaded")?;
        let sites = self.sites.as_deref().ok_or("site data is not loaded")?;
        let site_id = sites
            .by_type
            .iter()
            .find(|(kind, _)| kind == site_type)
            .map(|(_, id)| *id)
            .ok_or("NPC site is absent from the loaded site table")?;
        let rows = store
            .units
            .iter()
            .find(|(member, _)| *member == unit)
            .map(|(_, rows)| rows.as_slice())
            .unwrap_or(&[]);
        let matches_site = |row: &TalkRow| {
            SITE_GROUP_SITES
                .iter()
                .find(|(group, _)| *group == row.site_group_id)
                .is_some_and(|(_, sites)| sites.contains(&site_id))
        };
        let pool: Vec<_> = rows
            .iter()
            .filter(|row| {
                if !is_general_row(row) || previous == Some(row.talk_id) {
                    return false;
                }
                // Preserve the existing ordinary product's supported condition
                // surface. A fixture predicate needs its own actual activity data.
                if row.conditions.iter().any(|condition| {
                    !matches!(
                        condition_type_discriminant(condition),
                        Some(
                            CONDITION_READ_EVENT_STORY_EPISODE_ID
                                | CONDITION_MYSEKAI_PHENOMENA_ID
                                | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT
                                | CONDITION_AFTER_SET_FIXTURE
                        )
                    )
                }) {
                    return false;
                }
                condition_group_matches(
                    row.conditions
                        .iter()
                        .zip(&row.condition_values)
                        .filter_map(|(kind, value)| value.map(|value| (kind.as_str(), value))),
                    &ConditionContext {
                        current_phenomena_id: self.phenomena.0,
                        ..Default::default()
                    },
                ) && matches_site(row)
            })
            .collect();
        if pool.is_empty() {
            let general_rows = rows.iter().filter(|row| is_general_row(row)).count();
            let general_site_rows = rows
                .iter()
                .filter(|row| is_general_row(row) && matches_site(row))
                .count();
            trace!(
                "[player-talk-pool] unit={unit} site_type={site_type} site_id={site_id} phenomena={} rows={} general_rows={general_rows} general_after_site={general_site_rows} candidates={} previous={previous:?} previous_has_master={}",
                self.phenomena.0, rows.len(), pool.len(), previous.is_some_and(|id| id != 0),
            );
            return previous
                .filter(|id| *id != 0)
                .map(|master_id| GeneralSelection {
                    master_id,
                    pool_len: 0,
                    replayed: true,
                })
                .ok_or("ordinary pool is empty and shared Previous has no master");
        }
        let index = draw(pool.len());
        Ok(GeneralSelection {
            master_id: pool[index].talk_id,
            pool_len: pool.len(),
            replayed: false,
        })
    }
}

/// 玩家对话剧本的文本字符集（全部 unit 的正文/名字栏/tweet 文案）：
/// `balloon::bake_atlas` 把它并进对话窗共用的图集一次烘全。独立成资源
/// 而不挂在候选上：图集是**一次烘**（烘过即定格），烘焙门等它——
/// 候选要等名册（慢），字符集不等（装载即定格，全 unit——候选筛晚
/// 于装载，字形只多不少）。
#[derive(Resource)]
pub(crate) struct PlayerTalkCharset {
    pub(crate) chars: Vec<char>,
}

/// 站点主表的消费面：`siteType → 站点 id`（行自带 id 列）。
#[derive(Resource)]
pub(crate) struct PlayerTalkSites {
    by_type: Vec<(String, i32)>,
}

/// 名册就绪后筛出的候选段：unit 桶全在名册（单角色条件门的静态半边
/// ——组的成员表全在名册才可能被点到），装载期数据面核验同批做。
#[derive(Resource)]
pub(crate) struct PlayerTalkCandidates {
    units: Vec<(u32, Vec<TalkRow>)>,
    /// 罗马指称名（不带 `Characters.` 前缀）→ unit。
    names: HashMap<String, u32>,
}

/// Playback/input counters and throttle only. Current/Previous is owned by AI.
#[derive(Resource, Default)]
pub(crate) struct PlayerTalkLedger {
    requests: u32,
    busy_skips: u32,
    selections: u32,
    replays: u32,
    refusals: u32,
    completed: u32,
    last_request_at: Option<f32>,
    pub(crate) last_outcome: Option<PlayerTalkOutcome>,
}

#[derive(Clone, Debug)]
pub(crate) enum PlayerTalkOutcome {
    Started(crate::npc_objective::TalkContent),
    Rejected {
        requested: Option<crate::npc_objective::TalkContent>,
        reason: String,
    },
}

impl PlayerTalkLedger {
    fn reject(&mut self, request: &PlayerTalkRequest, reason: impl Into<String>) {
        self.refusals += 1;
        self.last_outcome = Some(PlayerTalkOutcome::Rejected {
            requested: request.exact.clone(),
            reason: reason.into(),
        });
    }
}

/// Startup：请求装载剧本表与站点主表，落账本资源。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(PlayerTalkDataHandle {
        talks: server.load::<JsonAsset>(AssetPath::from(PLAYER_TALK_DATA.to_owned())),
        sites: server.load::<JsonAsset>(AssetPath::from(SITES_DATA.to_owned())),
    });
    commands.init_resource::<PlayerTalkLedger>();
}

/// Update：两表到齐即解析。装载失败响亮 panic（资产边界的拒绝点）；
/// 报错只带字段名与段 id——文本内容不进任何报错。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<PlayerTalkDataHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    for asset in [&handle.talks, &handle.sites] {
        if let LoadState::Failed(err) = server.load_state(asset) {
            warn!("[talk-ingest] player conversation asset unavailable: {err:?}");
            commands.insert_resource(PlayerTalkStore {
                units: Vec::new(),
                names: HashMap::new(),
            });
            commands.insert_resource(PlayerTalkCharset { chars: Vec::new() });
            commands.insert_resource(PlayerTalkSites {
                by_type: Vec::new(),
            });
            commands.remove_resource::<PlayerTalkDataHandle>();
            return;
        }
    }
    let (Some(talks), Some(sites)) = (jsons.get(&handle.talks), jsons.get(&handle.sites)) else {
        return;
    };
    let (store, charset, issues) = parse_talks(&talks.0);
    for issue in &issues {
        warn!("[talk-ingest] quarantined general row: {issue}");
    }
    let site_map = match parse_sites(&sites.0) {
        Ok(sites) => sites,
        Err(reason) => {
            warn!("[talk-ingest] site lookup unavailable: {reason}");
            PlayerTalkSites {
                by_type: Vec::new(),
            }
        }
    };
    // 现象门控段（条件表含现象型）与值缺位现象臂各点一次名：前者是
    // 冒烟账目的分母（档位切换下这些段的入池/出池可推导），后者当前
    // 全语料为 0——非 0 即提取侧形状变化，装载期就要看见。
    let phenomena_gated = store
        .units
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .filter(|row| {
            row.conditions.iter().any(|condition| {
                condition_type_discriminant(condition) == Some(CONDITION_MYSEKAI_PHENOMENA_ID)
            })
        })
        .count();
    let phenomena_value_missing = store
        .units
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .flat_map(|row| {
            row.conditions
                .iter()
                .zip(&row.condition_values)
                .filter(|&(condition, _)| {
                    condition_type_discriminant(condition) == Some(CONDITION_MYSEKAI_PHENOMENA_ID)
                })
                .map(|(_, value)| *value)
        })
        .filter(Option::is_none)
        .count();
    info!(
        "[player-talk] 剧本表就绪：{} unit / {} 段（现象门控 {}、值缺位现象臂 {}）/ {} 步 / 字符集 {} 字；站点主表 {} 行",
        store.units.len(),
        store
            .units
            .iter()
            .map(|(_, rows)| rows.len())
            .sum::<usize>(),
        phenomena_gated,
        phenomena_value_missing,
        store
            .units
            .iter()
            .map(|(_, rows)| rows.iter().map(|r| r.steps.len()).sum::<usize>())
            .sum::<usize>(),
        charset.chars.len(),
        site_map.by_type.len(),
    );
    commands.insert_resource(store);
    commands.insert_resource(charset);
    commands.insert_resource(site_map);
    commands.remove_resource::<PlayerTalkDataHandle>();
}

/// 解析入口：顶层 `units` 对象按 unit 键分桶，逐行折成律行；同批导出
/// 指称名表（非玩家指称 `Characters.<名>` → 本桶 unit；一名两 unit 即
/// 响亮失败——指称解析按单名假设）与字符集（正文/名字栏/tweet 文案，
/// 口径同配对对话侧的收集）。未知 op 响亮失败（提取词表外的新步型，
/// 静默跳过会把整段流播残）。
fn parse_talks(text: &str) -> (PlayerTalkStore, PlayerTalkCharset, Vec<String>) {
    let mut issues = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            issues.push(format!("document is not valid JSON: {error}"));
            return (
                PlayerTalkStore {
                    units: Vec::new(),
                    names: HashMap::new(),
                },
                PlayerTalkCharset { chars: Vec::new() },
                issues,
            );
        }
    };
    let Some(units) = value.get("units").and_then(|v| v.as_object()) else {
        issues.push("document has no units object".into());
        return (
            PlayerTalkStore {
                units: Vec::new(),
                names: HashMap::new(),
            },
            PlayerTalkCharset { chars: Vec::new() },
            issues,
        );
    };
    let mut buckets = Vec::with_capacity(units.len());
    let mut chars: Vec<char> = Vec::new();
    let mut names: HashMap<String, u32> = HashMap::new();
    for (key, entry) in units {
        let Ok(unit) = key.parse::<u32>() else {
            issues.push(format!("unit key {key:?} is not numeric"));
            continue;
        };
        let Some(rows) = entry.get("talks").and_then(|v| v.as_array()) else {
            issues.push(format!("unit {unit} has no talks array"));
            buckets.push((unit, Vec::new()));
            continue;
        };
        let talks: Vec<TalkRow> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match crate::talk_ingest::general_row(row) {
                Ok(()) => Some(parse_row(row)),
                Err(reason) => {
                    issues.push(format!("unit {unit}, source row {}: {reason}", index + 1));
                    None
                }
            })
            .collect();
        for row in &talks {
            collect_chars(&row.tweet.text, &mut chars);
            for step in &row.steps {
                if let Some(name) = referent_name(step) {
                    match names.get(name) {
                        Some(&known) if known != unit => {
                            issues.push(format!(
                                "referent {name:?} belongs to both unit {known} and {unit}; keeping first binding"
                            ));
                        }
                        None => {
                            names.insert(name.to_owned(), unit);
                        }
                        _ => {}
                    }
                }
                match step {
                    TalkStep::Text { text } => collect_chars(text, &mut chars),
                    // 名字栏文本也要字形：label 名的字符集一并进图集。
                    TalkStep::Label { name } => collect_chars(name, &mut chars),
                    _ => {}
                }
            }
        }
        buckets.push((unit, talks));
    }
    buckets.sort_by_key(|(unit, _)| *unit);
    chars.sort_unstable();
    (
        PlayerTalkStore {
            units: buckets,
            names,
        },
        PlayerTalkCharset { chars },
        issues,
    )
}

/// 一步里的指称载荷（`who`/`target` 任一面的非玩家 `Characters.<名>`）：
/// 指称名表的来源。
fn referent_name(step: &TalkStep) -> Option<&str> {
    let candidates = [
        match step {
            TalkStep::LookAtBody { who, .. }
            | TalkStep::Voice { who, .. }
            | TalkStep::ChangeNpcEye { who, .. }
            | TalkStep::ChangeNpcMouth { who, .. }
            | TalkStep::ChangeAnimation { who, .. }
            | TalkStep::Emoticon { who, .. }
            | TalkStep::HideEmoticon { who } => Some(who.as_str()),
            _ => None,
        },
        match step {
            TalkStep::LookAtBody { target, .. } => Some(target.as_str()),
            _ => None,
        },
    ];
    candidates
        .into_iter()
        .flatten()
        .filter_map(|referent| referent.strip_prefix("Characters."))
        .find(|name| *name != "Player")
}

fn parse_row(row: &serde_json::Value) -> TalkRow {
    let talk_id = i_field(row, "talkId");
    let conditions: Vec<String> = row
        .get("conditions")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("段 {talk_id} 缺 conditions 数组"))
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("段 {talk_id} 的 conditions 行不是字符串"))
                .to_owned()
        })
        .collect();
    // 平行值键（与 conditions 同序同长）：每条条件在源条件主表上的值，
    // 现象门比的就是它。形状错位（缺数组/长度不齐/逐位类型名不符）都
    // 是提取产物的形状破坏，装载期响亮失败——错位会把现象值配到别的
    // 条件上，静默降级会把门关错方向。值 `null` 是源主表该条件无值
    // （提取不造默认值），折成 `None`，求值处按不匹配处理。
    let value_rows = row
        .get("conditionValues")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("段 {talk_id} 缺 conditionValues 平行数组"));
    if value_rows.len() != conditions.len() {
        panic!(
            "段 {talk_id} 的 conditionValues（{} 行）与 conditions（{} 行）不平行",
            value_rows.len(),
            conditions.len()
        );
    }
    let condition_values: Vec<Option<i32>> = value_rows
        .iter()
        .zip(&conditions)
        .enumerate()
        .map(|(index, (value, condition))| {
            let paired_type = value
                .get("conditionType")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    panic!("段 {talk_id} 的 conditionValues 第 {index} 行缺 conditionType")
                });
            if paired_type != condition {
                panic!(
                    "段 {talk_id} 的 conditionValues 第 {index} 行类型 {paired_type} 与 conditions 的 {condition} 不平行"
                );
            }
            match value.get("conditionTypeValue") {
                None => panic!(
                    "段 {talk_id} 的 conditionValues 第 {index} 行缺 conditionTypeValue 键"
                ),
                Some(v) if v.is_null() => None,
                Some(v) => Some(
                    v.as_i64()
                        .unwrap_or_else(|| {
                            panic!("段 {talk_id} 的 conditionValues 第 {index} 行值不是整数")
                        }) as i32,
                ),
            }
        })
        .collect();
    TalkRow {
        talk_id,
        lua: s_field(row, "lua"),
        site_group_id: i_field(row, "siteGroupId"),
        term_id: i_field(row, "termId"),
        conditions,
        condition_values,
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

fn parse_tweet_ref(value: Option<&serde_json::Value>, talk_id: i32) -> TweetRef {
    let row = value.unwrap_or_else(|| panic!("段 {talk_id} 缺 tweet 引用"));
    TweetRef {
        id: i_field(row, "id"),
        text: s_field(row, "text"),
        motion: s_field(row, "motion"),
        eye: s_field(row, "eye"),
        mouth: s_field(row, "mouth"),
    }
}

fn parse_step(step_value: &serde_json::Value) -> TalkStep {
    let op = step_value
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("剧本步缺 op 字段：{step_value:?}"));
    let f_field = |name: &str| -> f64 {
        step_value
            .get(name)
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("{op} 步缺 {name} 数字字段"))
    };
    let s_field_step = |name: &str| -> String {
        step_value
            .get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{op} 步缺 {name} 字符串字段"))
            .to_owned()
    };
    // 可缺数字段：值缺省约定与配对对话侧同键同约。
    let opt_f = |name: &str| -> Option<f64> {
        step_value
            .get(name)
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
    };
    // 指称字段：两代产物形状都收（字符串形 / 数字形，见模块注释）。
    // 数字 0＝玩家；正整数＝unit id（规范成内部形 `id:<n>`）；键缺＝
    // 无指称（配对对话侧同键同约：空串＝悬空）。
    let who_field = |key: &str| -> String {
        match step_value.get(key) {
            Some(v) if v.is_string() => v.as_str().unwrap().to_owned(),
            Some(v) if v.is_number() => {
                let n = v.as_f64().unwrap_or(f64::NAN);
                if n == 0.0 {
                    "Characters.Player".to_owned()
                } else if n.fract() == 0.0 && n > 0.0 {
                    format!("id:{}", n as u32)
                } else {
                    panic!("{op} 步的 {key} 数字不是合法指称：{n}");
                }
            }
            _ => String::new(),
        }
    };
    // 眼/口图样：剥「表名.」前缀得键（两张常量表恒等映射，见模块注释）。
    let facial_field = |name: &str| -> String {
        let raw = s_field_step(name);
        match raw
            .strip_prefix("EyePresets.")
            .or_else(|| raw.strip_prefix("LipSyncPresets."))
        {
            Some(key) => key.to_owned(),
            None => raw,
        }
    };
    match op {
        "look_at_body" => TalkStep::LookAtBody {
            who: who_field("who"),
            target: who_field("target"),
            duration: f_field("duration"),
        },
        "wait_time" => TalkStep::WaitTime {
            seconds: f_field("seconds"),
            auto: step_value.get("auto").and_then(|v| v.as_bool()),
        },
        "label" => TalkStep::Label {
            name: s_field_step("name"),
        },
        "voice" => TalkStep::Voice {
            channel: s_field_step("channel"),
            cue: s_field_step("cue"),
            who: who_field("who"),
        },
        "change_npc_eye" => TalkStep::ChangeNpcEye {
            who: who_field("who"),
            pattern: facial_field("pattern"),
            alias: s_field_step("alias"),
            delay_seconds: crate::delayed_faces::delay_seconds(step_value)
                .unwrap_or_else(|reason| panic!("change_npc_eye: {reason}")),
        },
        "change_npc_mouth" => TalkStep::ChangeNpcMouth {
            who: who_field("who"),
            pattern: facial_field("pattern"),
            alias: s_field_step("alias"),
            delay_seconds: crate::delayed_faces::delay_seconds(step_value)
                .unwrap_or_else(|reason| panic!("change_npc_mouth: {reason}")),
        },
        "change_animation" => TalkStep::ChangeAnimation {
            who: who_field("who"),
            motion: s_field_step("motion"),
            alias: s_field_step("alias"),
            // speed 载荷：源未传参或 null 都落 None（缺省 1.0 由律换算）。
            speed: opt_f("speed"),
            playback_speed: opt_f("playbackSpeed").unwrap_or(1.0),
            play_end_motion: step_value
                .get("playEndMotion")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        "text" => TalkStep::Text {
            text: s_field_step("text"),
        },
        "wait_click" => TalkStep::WaitClick,
        "emoticon" => TalkStep::Emoticon {
            who: who_field("who"),
            name: s_field_step("name"),
            alias: s_field_step("alias"),
            // showSeconds 键可缺（语料 216/355 步无此键）；同键在配对
            // 对话与待机编排侧的解析已定约：缺＝0.0，0＝不自动收回。
            // 本表同键同约。
            show_seconds: opt_f("showSeconds").unwrap_or(0.0),
        },
        "hide_emoticon" => TalkStep::HideEmoticon {
            who: who_field("who"),
        },
        "show_talk_window" => TalkStep::ShowTalkWindow,
        "hide_talk_window" => TalkStep::HideTalkWindow,
        "wait_time_on_auto_mode" => TalkStep::WaitTimeOnAutoMode {
            seconds: f_field("seconds"),
        },
        other => panic!("玩家对话剧本步的 op 不在词表里：{other}"),
    }
}

/// 站点主表解析：只取 `siteType → id` 对照（站点门消费面）。
fn parse_sites(text: &str) -> Result<PlayerTalkSites, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("site document is not valid JSON: {error}"))?;
    let rows = value
        .get("sites")
        .and_then(|v| v.as_array())
        .ok_or("site document has no sites array")?;
    let mut by_type = Vec::with_capacity(rows.len());
    for row in rows {
        let site_type = row
            .get("siteType")
            .and_then(|v| v.as_str())
            .ok_or("site row has no string siteType")?;
        let id = row
            .get("id")
            .and_then(|v| v.as_i64())
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| format!("site {site_type:?} has no i32 id"))?;
        by_type.push((site_type.to_owned(), id));
    }
    Ok(PlayerTalkSites { by_type })
}

fn collect_chars(text: &str, chars: &mut Vec<char>) {
    for ch in text.chars() {
        if ch != '\n' && !chars.contains(&ch) {
            chars.push(ch);
        }
    }
}

fn s_field(row: &serde_json::Value, name: &str) -> String {
    row.get(name)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("玩家对话行缺 {name}"))
        .to_owned()
}

fn i_field(row: &serde_json::Value, name: &str) -> i32 {
    row.get(name)
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("玩家对话行缺 {name}")) as i32
}

// ---------------------------------------------------------------------------
// 候选预筛（名册 + 数据面核验）
// ---------------------------------------------------------------------------

/// Update：名册/库/表/档案全就绪后筛候选。筛法＝unit 桶的名册归属
/// （单角色条件门的静态半边——组的成员表全在名册才可能被点到）；
/// 数据面核验（facial 键、动作段、表情件名）对候选段全量做成 census
/// 点名（不拒绝，见 [`Census`]）。指称名表照录全量（名 → unit 与名册
/// 无关）。名册上没有对话成员时警告一次并逐帧重试（名册随站点变，
/// 空是暂态不是数据断点）。
#[allow(clippy::type_complexity)]
pub(crate) fn prepare(
    mut commands: Commands,
    store: Option<Res<PlayerTalkStore>>,
    registry: Option<Res<Registry>>,
    library: Option<Res<MotionLibrary>>,
    libraries: Res<Assets<Gltf>>,
    tables: Option<Res<FacialTables>>,
    archive: Option<Res<EmoticonArchive>>,
    mut warned: Local<bool>,
    mut prepared_roster: Local<Vec<u32>>,
) {
    let (Some(store), Some(registry), Some(library), Some(tables), Some(archive)) =
        (store, registry, library, tables, archive)
    else {
        return;
    };
    let Some(lib) = libraries.get(&library.gltf) else {
        return; // 动作库未到齐（有驱动即到齐，此行只是防序）
    };
    let roster = &registry.character_unit_ids;
    if *prepared_roster == *roster && !prepared_roster.is_empty() {
        return;
    }
    let mut units = Vec::new();
    let mut census = Census::default();
    for (unit, rows) in &store.units {
        if !roster.contains(unit) {
            continue;
        }
        for row in rows {
            verify_row(row, *unit, &tables, lib, &archive, &mut census);
        }
        units.push((*unit, rows.clone()));
    }
    if units.is_empty() {
        if !*warned {
            *warned = true;
            warn!(
                "[player-talk] 名册 {:?} 里暂无通用对话成员：候选等待名册（换站重播报）",
                roster
            );
        }
        return;
    }
    // 缺键 census 点名（真源不预验剧本：这些不是装载失败，是执行期
    // 降级项的预告——各分发臂按真源语义处理，见模块注释）。
    for (label, map) in [
        ("facial", &census.facial),
        ("动作段", &census.motion),
        ("表情件", &census.emoticon),
    ] {
        if !map.is_empty() {
            let (kinds, steps, keys) = Census::summary(map);
            warn!(
                "[player-talk] 候选核验：{label} 缺 {kinds} 种（{steps} 步），如 {keys:?}——执行期按真源降级（眼/口 → 格 0；动作/表情件 → 不起播不出件）"
            );
        }
    }
    let rows_total: usize = units.iter().map(|(_, rows)| rows.len()).sum();
    let verdict = if census.is_empty() {
        "数据面核验全过".to_owned()
    } else {
        "数据面核验有缺项（见 census 警告）".to_owned()
    };
    info!(
        "[player-talk] 候选就绪：{} unit / {} 段（名册 {:?}）；{}",
        units.len(),
        rows_total,
        roster,
        verdict,
    );
    commands.insert_resource(PlayerTalkCandidates {
        units,
        names: store.names.clone(),
    });
    *prepared_roster = roster.clone();
}

/// 候选段数据面的装载期点名账（缺键 census）：键 →（步次，首见 unit）。
/// 真源的对话引擎不预验剧本（缺键是执行期事件，见模块注释），装载期
/// 只点名不拒绝——会话照开，缺键在执行期按真源语义降级。
#[derive(Default)]
struct Census {
    facial: std::collections::BTreeMap<String, (u32, u32)>,
    motion: std::collections::BTreeMap<String, (u32, u32)>,
    emoticon: std::collections::BTreeMap<String, (u32, u32)>,
}

impl Census {
    fn note(map: &mut std::collections::BTreeMap<String, (u32, u32)>, key: &str, unit: u32) {
        map.entry(key.to_owned()).or_insert((0, unit)).0 += 1;
    }

    fn is_empty(&self) -> bool {
        self.facial.is_empty() && self.motion.is_empty() && self.emoticon.is_empty()
    }

    /// 一张账的摘要（键种数 / 步次 / 前几个键名）。
    fn summary(map: &std::collections::BTreeMap<String, (u32, u32)>) -> (usize, u32, Vec<&str>) {
        let steps: u32 = map.values().map(|(n, _)| n).sum();
        let keys = map.keys().take(5).map(String::as_str).collect();
        (map.len(), steps, keys)
    }
}

/// 装载期核验一段（census 口径，不拒绝）：眼/口键在 facial 两表、动作
/// 段名在共享动作库（`_S` 起播段变体）、表情件名在档案。缺什么记进
/// [`Census`]，由 [`prepare`] 汇总点名；执行期的降级语义见各分发臂。
fn verify_row(
    row: &TalkRow,
    unit: u32,
    tables: &FacialTables,
    lib: &Gltf,
    archive: &EmoticonArchive,
    census: &mut Census,
) {
    for step in &row.steps {
        if let Some((slot, pattern, _)) = step.face_change() {
            let missing = match slot {
                FaceSlot::Eye => tables.eye_open(pattern).is_none(),
                FaceSlot::Mouth => tables.lip_close(pattern).is_none(),
            };
            if missing {
                Census::note(&mut census.facial, pattern, unit);
            }
        }
        if let TalkStep::ChangeAnimation { motion, .. } = step {
            // 动作库里的剪辑带段族后缀（`_S` 起播段）；对话步点播基名。
            let clip = format!("{motion}_S");
            if !lib.named_animations.contains_key(clip.as_str()) {
                Census::note(&mut census.motion, motion, unit);
            }
        }
        if let TalkStep::Emoticon { name, .. } = step {
            if !archive.has(name) {
                Census::note(&mut census.emoticon, name, unit);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 抽签引擎（与配对对话侧同款：跨帧 LCG + 计数落点）
// ---------------------------------------------------------------------------

struct TalkRng(u64);

impl TalkRng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

/// 均匀层抽签（带计数与落点记录：量出「恰一抽」并把掷点带回日志）。
struct UniformSource {
    rng: TalkRng,
    calls: usize,
    last_index: usize,
    last_len: usize,
}

impl UniformDraw for UniformSource {
    fn draw(&mut self, len: usize) -> usize {
        self.calls += 1;
        self.last_len = len;
        self.last_index = ((self.rng.next() >> 33) as usize) % len.max(1);
        self.last_index
    }
}

// ---------------------------------------------------------------------------
// 会话与运行时
// ---------------------------------------------------------------------------

/// Ordinary-script session. The single dispatcher reserves either this backend
/// or ActiveTalk; neither backend owns a second request reader.
#[derive(Resource)]
pub(crate) struct PlayerTalkSession {
    talk_id: i32,
    unit: u32,
    /// NPC 参演者的罗马指称名（不带前缀；指称解析用）。
    npc_name: String,
    npc: Entity,
    player: Entity,
    row: TalkRow,
    state: StreamState,
    /// 当前说话人（`who` 指称跟踪：npc 名 / 玩家 / 悬空）。
    speaker: Option<Speaker>,
    hold_logged: Option<&'static str>,
    started_at: f32,
    steps_fired: usize,
    texts: usize,
    click_releases: usize,
    /// 选取记账：池形（重播路径也记池空时的形）。
    selected_from_pool: usize,
    replayed: bool,
    /// 玩家位与朝向的会话内账：开场时快照（玩家参演期间位移推进被持
    /// 留跳过，位不变——快照即现值）；朝向只被本模块的转身件改写，
    /// 每次玩家转身把账同步到该步的目标朝向。
    player_position: [f32; 3],
    player_rotation: Quat,
    /// 会话开场后的首拍：发起点按的闩在此耗尽（见 [`advance_session`]）。
    first_tick: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Speaker {
    Npc(u32),
    Player,
}

impl PlayerTalkSession {
    /// 会话的段 id（窗体日志的链名读取点用）。
    pub(crate) fn talk_id(&self) -> i32 {
        self.talk_id
    }

    /// NPC 参演者实体（对话相机的取位面读它——真源取位表由出场角色的
    /// 髋变换构成，玩家链的出场角色就是这位 NPC；玩家的取位条目走
    /// avatar 视变换标记，不经本访问器）。
    pub(crate) fn npc_entity(&self) -> Entity {
        self.npc
    }
}

/// NPC 参演者身上的运行时（同配对对话侧的形状）：眼/口材质句柄、动作
/// 节点缓存、在播动作。玩家不挂（本语料玩家只有转身步）。
#[derive(Component)]
pub(crate) struct PlayerTalkRuntime {
    eye_handle: Handle<CharacterMaterial>,
    mouth_handle: Handle<CharacterMaterial>,
    nodes: HashMap<String, AnimationNodeIndex>,
    anim_node: Option<AnimationNodeIndex>,
}

/// 玩家的转身（会话期间）：玩家域不写转体相位（玩家动画驱动的转体
/// 支不可达），转身由本模块按步时长插值直写朝向，完成即撤。
#[derive(Component)]
pub(crate) struct PlayerTurn {
    from: Quat,
    to: Quat,
    duration: f32,
    elapsed: f32,
}

#[derive(Component)]
pub(crate) struct PlayerTalkRotationSnapshot {
    talk_id: i32,
    rotation: Quat,
}

pub(crate) fn snapshot_player_rotation(
    commands: &mut Commands,
    player: Entity,
    talk_id: i32,
    rotation: Option<Quat>,
) {
    commands.queue(move |world: &mut World| {
        if world.get::<PlayerTalkRotationSnapshot>(player).is_some() {
            return;
        }
        let Some(rotation) =
            rotation.or_else(|| world.get::<Transform>(player).map(|t| t.rotation))
        else {
            return;
        };
        world
            .entity_mut(player)
            .insert(PlayerTalkRotationSnapshot { talk_id, rotation });
    });
}

pub(crate) fn restore_cancelled_player_rotation(
    commands: &mut Commands,
    player: Entity,
    talk_id: i32,
) {
    commands.queue(move |world: &mut World| {
        let Some(rotation) = world
            .get::<PlayerTalkRotationSnapshot>(player)
            .filter(|snapshot| snapshot.talk_id == talk_id)
            .map(|snapshot| snapshot.rotation)
        else {
            return;
        };
        if let Some(mut transform) = world.get_mut::<Transform>(player) {
            transform.rotation = rotation;
        }
    });
}

/// The fixture-script backend uses the same player turn component and updater.
pub(crate) fn turn_player_to_point(
    commands: &mut Commands,
    player: Entity,
    target: [f32; 3],
    duration: f64,
) {
    commands.queue(move |world: &mut World| {
        let Some(transform) = world.get::<Transform>(player) else {
            return;
        };
        let from = transform.rotation;
        let forward = facing_direction(transform.translation.to_array(), target);
        let mut to = Quat::from_rotation_arc(Vec3::Z, Vec3::from(forward));
        if from.dot(to) < 0.0 {
            to = Quat::from_xyzw(-to.x, -to.y, -to.z, -to.w);
        }
        world.entity_mut(player).insert(PlayerTurn {
            from,
            to,
            duration: duration as f32,
            elapsed: 0.0,
        });
    });
}

#[derive(Clone, Copy, Debug)]
enum TalkRoute {
    General,
    CurrentSet,
}

fn route_for(
    actions: &crate::npc::NpcActions,
    data: Option<&crate::npc_objective::AiTalkData>,
) -> Result<TalkRoute, &'static str> {
    use crate::npc::NpcAction;
    use moly_law::objective::TalkType;
    let Some(data) = data else {
        return Ok(TalkRoute::General);
    };
    if matches!(
        data.kind,
        TalkType::TutorialFreeWalking | TalkType::TutorialWaiting
    ) {
        return Err("tutorial override/default/next-state context is not supplied");
    }
    if matches!(data.kind, TalkType::AfterEditLayout | TalkType::RandomWalk) {
        return Ok(TalkRoute::General);
    }
    if data.kind == TalkType::CommunicationWhileDoingWait {
        return Err("while-doing participants and their arrival slots are not supplied");
    }
    // A running furniture action needs its activity's real timeline, actors
    // and loop-restoration owner. It cannot borrow the ordinary route merely
    // because its data factory is incomplete.
    if matches!(
        actions.current,
        NpcAction::FixtureAction | NpcAction::FixtureActionIdle
    ) {
        return Err(
            "playing-fixture talk needs the registered activity and its IK/loop restore owner",
        );
    }
    if data.kind == TalkType::BirthdayParty {
        return Err("birthday context and authored scenario are not supplied");
    }

    // IsGeneralTalkState reads the *current action*: Idle, AutoMove, Walk or
    // Rest. A CommonFixture/NoneTalk objective may still be a future intent
    // while that action is ordinary. Its pending master is not a registered
    // playing fixture and does not gate PlayGeneralTalk's independent lottery.
    // IsPlayingSomeCharacterTalkFixture instead requires an actual timeline
    // registration with multiple action presenters, not an AITalkType label.
    if matches!(
        actions.current,
        NpcAction::Idle | NpcAction::AutoMove | NpcAction::Walk | NpcAction::Rest
    ) {
        return Ok(TalkRoute::General);
    }
    if let Some(reason) = data.pending_factory {
        return Err(reason);
    }
    if matches!(
        data.kind,
        TalkType::MultipleCharacterFixture
            | TalkType::SingleCharacterFixture
            | TalkType::CommonFixture
    ) || data.target_fixture.is_some()
    {
        return Err("registered NPC fixture timeline and loop control are not supplied");
    }
    if actions.current == NpcAction::Tweet {
        if data.content.is_none() {
            return Err("waiting talk has no prepared current master");
        }
        return Ok(TalkRoute::CurrentSet);
    }
    Ok(TalkRoute::General)
}

// ---------------------------------------------------------------------------
// 触发消费：六门求值 → 池 → 抽签 / 重播 → 会话开场
// ---------------------------------------------------------------------------

/// Update：消费拾取链的玩家对话请求。会话在播、自主对话在播、候选或
/// 站点未就绪都不发起（真源态梯同形：对话态下按钮不再走通用分支）。
#[allow(clippy::type_complexity)]
pub(crate) fn consume_trigger(
    time: Res<Time>,
    mut commands: Commands,
    mut requests: MessageReader<PlayerTalkRequest>,
    eligibility: crate::interaction::InteractionEligibility,
    session: Option<Res<PlayerTalkSession>>,
    active_talk: Option<Res<crate::talk::ActiveTalk>>,
    catalog: TalkCatalog,
    npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &WalkState,
            &ToonMaterials,
            &crate::npc::NpcActions,
            &crate::npc_objective::TalkSlot,
        ),
        Without<PlayerControlled>,
    >,
    player: Query<(Entity, &Transform), With<PlayerControlled>>,
    fixtures: Query<
        (
            Entity,
            &FixtureActivityIdentity,
            &FixturePlacement,
            &Transform,
        ),
        With<FixtureRoot>,
    >,
    mut ledger: ResMut<PlayerTalkLedger>,
    mut window: ResMut<TalkWindowState>,
) {
    // 同帧多请求只发起第一个（资源插入是延迟命令，循环内读不到自己
    // 刚插的会话）。
    let mut session_started = session.is_some() || active_talk.is_some();
    for request in requests.read() {
        ledger.requests += 1;
        if session_started {
            ledger.busy_skips += 1;
            ledger.reject(request, "another talk session is active");
            continue;
        }
        if !catalog.ready() {
            ledger.reject(request, "talk catalog is not ready");
            continue;
        }
        let now = time.elapsed_secs();
        if ledger
            .last_request_at
            .is_some_and(|last| now - last < moly_law::action_button::ACTION_BUTTON_INPUT_INTERVAL)
        {
            ledger.busy_skips += 1;
            ledger.reject(request, "talk request was throttled");
            continue;
        }
        let safe_position = match if request.exact.is_some() {
            eligibility.library_position(request.entity)
        } else {
            eligibility.click_position(request.entity, request.unit)
        } {
            Ok(position) => position,
            Err(reason) => {
                ledger.reject(request, reason);
                warn!("[player-talk] unit {} cannot talk: {reason}", request.unit);
                continue;
            }
        };
        let Ok((player_entity, player_transform)) = player.single() else {
            ledger.reject(request, "exactly one real player is required");
            continue;
        };
        let Ok((_, member, _, _, actions, binding)) = npcs.get(request.entity) else {
            ledger.reject(request, "requested NPC entity left the scene");
            continue;
        };
        if member.0 != request.unit {
            ledger.reject(request, "requested NPC entity/unit identity changed");
            continue;
        }
        let route = match request
            .exact
            .as_ref()
            .map(|_| Ok(TalkRoute::CurrentSet))
            .unwrap_or_else(|| route_for(actions, binding.current.as_ref()))
        {
            Ok(route) => route,
            Err(reason) => {
                ledger.reject(request, reason);
                warn!(
                    "[player-talk] unit {} activity is not prepared: {reason}",
                    request.unit
                );
                continue;
            }
        };
        info!(
            "[player-talk] unit {} route={route:?} action={:?} ai_type={:?} future_factory_pending={}",
            request.unit, actions.current, binding.kind(),
            binding.current.as_ref().is_some_and(|data| data.pending_factory.is_some()),
        );
        let previous_id = binding.previous_id();
        let selection = if let Some(content) = request.exact.as_ref() {
            GeneralSelection {
                master_id: content.master_id,
                pool_len: 1,
                replayed: false,
            }
        } else {
            match route {
                TalkRoute::General => {
                    let mut uniform = UniformSource {
                        rng: TalkRng(selection_seed(
                            request.unit,
                            catalog.site_id(&actions.site_type).unwrap_or(0),
                            previous_id,
                            ledger.requests,
                        )),
                        calls: 0,
                        last_index: 0,
                        last_len: 0,
                    };
                    let mut selected = match catalog.lottery(
                        request.unit,
                        &actions.site_type,
                        previous_id,
                        |len| uniform.draw(len),
                    ) {
                        Ok(selected) => selected,
                        Err(reason) => {
                            ledger.reject(request, reason);
                            warn!("[player-talk] unit {}: {reason}", request.unit);
                            continue;
                        }
                    };
                    let lottery_id = selected.master_id;
                    if let Some(current) = binding
                        .current
                        .as_ref()
                        .and_then(|data| data.content.as_ref())
                    {
                        match current.is_general {
                            Some(true) => selected.master_id = current.master_id,
                            Some(false) => {}
                            None => {
                                ledger.reject(
                                    request,
                                    "Current content lacks general-condition predicates",
                                );
                                warn!("[player-talk] Current {} lacks its general-condition predicates", current.master_id);
                                continue;
                            }
                        }
                    }
                    info!("[player-talk] unit {} General lottery={} draws={} then Current overwrite={} pool={}",
                    request.unit, lottery_id, uniform.calls, selected.master_id, selected.pool_len);
                    selected
                }
                TalkRoute::CurrentSet => GeneralSelection {
                    master_id: binding
                        .current
                        .as_ref()
                        .and_then(|data| data.content.as_ref())
                        .expect("route requires prepared content")
                        .master_id,
                    pool_len: 0,
                    replayed: true,
                },
            }
        };
        let bound_content = if let Some(content) = request.exact.as_ref() {
            Some(content)
        } else {
            match route {
                TalkRoute::CurrentSet => binding
                    .current
                    .as_ref()
                    .and_then(|data| data.content.as_ref()),
                TalkRoute::General => binding
                    .current
                    .as_ref()
                    .and_then(|data| data.content.as_ref())
                    .filter(|content| {
                        content.is_general == Some(true) && content.master_id == selection.master_id
                    })
                    .or_else(|| {
                        selection
                            .replayed
                            .then(|| {
                                binding
                                    .previous
                                    .as_ref()
                                    .and_then(|data| data.content.as_ref())
                            })
                            .flatten()
                    }),
            }
        };
        let resolved = match bound_content {
            Some(content) => catalog.resolve_content(content),
            None => catalog.resolve(selection.master_id),
        };
        let Some(resolved) = resolved else {
            ledger.reject(request, "selected backend/master is not loaded");
            warn!("[player-talk] selected master {} is outside the loaded content, not an empty candidate pool", selection.master_id);
            continue;
        };
        let units = resolved.units();
        let unique_units: HashSet<_> = units.iter().copied().collect();
        if units.is_empty() || unique_units.len() != units.len() {
            ledger.reject(request, "selected content has an empty or duplicate cast");
            continue;
        }
        if request.exact.is_some() && !unique_units.contains(&request.unit) {
            ledger.reject(request, "requested NPC is not in the selected content cast");
            continue;
        }
        let mut actors = Vec::new();
        for unit in &units {
            let Some((entity, _, _, toon, state, _)) = npcs
                .iter()
                .find(|(_, member, _, _, _, _)| member.0 == *unit)
            else {
                break;
            };
            if !state.ready()
                || !state.enable_talk
                || state.current == crate::npc::NpcAction::Talk
                || state.talk_owner.is_some_and(|owner| owner != player_entity)
            {
                break;
            }
            let (Some(eye), Some(mouth)) = (toon.slot_handle("eye"), toon.slot_handle("mouth"))
            else {
                break;
            };
            actors.push(crate::talk::TalkActor {
                unit: *unit,
                entity,
                eye: eye.clone(),
                mouth: mouth.clone(),
            });
        }
        if actors.len() != units.len() {
            ledger.reject(
                request,
                "not every selected actor is present, ready, and rendered",
            );
            warn!(
                "[player-talk] master {} cannot resolve its final actor group",
                selection.master_id
            );
            continue;
        }
        let target_fixture = if request.exact.is_some() {
            request.target_fixture
        } else {
            match route {
                TalkRoute::General => None,
                TalkRoute::CurrentSet => binding
                    .current
                    .as_ref()
                    .and_then(|data| data.target_fixture),
            }
        };
        let fixture_bindings = match &resolved {
            ResolvedTalk::General { .. } => Vec::new(),
            ResolvedTalk::Fixture(row) => {
                match validate_fixture_context(row, target_fixture, &actors, &npcs, &fixtures) {
                    Ok(bindings) => bindings,
                    Err(reason) => {
                        ledger.reject(request, reason);
                        warn!(
                            "[player-talk] fixture master {} cannot be admitted: {reason}",
                            selection.master_id
                        );
                        continue;
                    }
                }
            }
        };
        // Admission is now complete. Throttle state and world mutations begin
        // only after backend/master, cast, fixture instances and context passed.
        ledger.last_request_at = Some(now);
        ledger.last_outcome = Some(PlayerTalkOutcome::Started(resolved.content()));
        if safe_position.length_squared() >= 1e-10 {
            commands.queue(move |world: &mut World| {
                if let Some(mut transform) = world.get_mut::<Transform>(player_entity) {
                    transform.translation = safe_position;
                }
            });
        }
        commands.queue(|world: &mut World| {
            if let Some(mut player_state) =
                world.get_resource_mut::<crate::player_state::PlayerAvatarStates>()
            {
                player_state.change_status(crate::player_state::PlayerActionState::Talk);
            }
        });
        for actor in &actors {
            crate::npc::enter_player_talk(&mut commands, actor.entity, player_entity);
        }
        session_started = true;
        let (final_unit, row) = match resolved {
            ResolvedTalk::Fixture(row) => {
                crate::talk::start_selected(
                    &mut commands,
                    &mut window,
                    row,
                    &actors,
                    player_entity,
                    target_fixture,
                    fixture_bindings,
                    now,
                );
                continue;
            }
            ResolvedTalk::General { unit, row } => (unit, row),
        };
        let replayed = selection.replayed;
        let actor = &actors[0];
        let npc_entity = actor.entity;
        let eye_handle = actor.eye.clone();
        let mouth_handle = actor.mouth.clone();
        // 玩家位/朝向快照进会话：参演期间位移推进被持留跳过，位不变；
        // 朝向只被本模块的转身件改写（步进侧见 begin_player_turn）。
        let player_position = safe_position.to_array();
        let player_rotation = player_transform.rotation;
        let npc_name = catalog.name(final_unit);
        let needs_named_referent = row.steps.iter().any(|step| {
            [referent_of_general_step(step), target_of_general_step(step)]
                .into_iter()
                .flatten()
                .any(|referent| {
                    referent
                        .strip_prefix("Characters.")
                        .is_some_and(|name| name != "Player" && !name.is_empty())
                })
        });
        if npc_name.is_empty() && needs_named_referent {
            // 剧本里没有该 unit 的指称名：全部指称步都会悬空（记日志
            // 跳过），会话照开——转身/眼口/动作不出，文本照上屏。
            warn!(
                "[player-talk] unit {} 的指称名不在名表（剧本未提及）：指称步将悬空",
                final_unit
            );
        }
        if replayed {
            ledger.replays += 1;
        } else {
            ledger.selections += 1;
        }
        info!(
            "[player-talk] 发起：点击unit {} → 最终成员 {}、master {}、步骤 {}、池 {}（shared Previous fallback={}）",
            request.unit,
            final_unit,
            row.talk_id,
            row.steps.len(),
            selection.pool_len,
            replayed,
        );
        commands
            .entity(npc_entity)
            .insert(TalkHold)
            .insert(PlayerTalkRuntime {
                eye_handle,
                mouth_handle,
                nodes: HashMap::new(),
                anim_node: None,
            })
            .insert(MotionPhase::Dwelling { remaining: None });
        commands
            .entity(player_entity)
            .insert(TalkHold)
            .insert(MotionPhase::Dwelling { remaining: None });
        snapshot_player_rotation(
            &mut commands,
            player_entity,
            row.talk_id,
            Some(player_transform.rotation),
        );
        let mut session = PlayerTalkSession {
            talk_id: row.talk_id,
            unit: final_unit,
            npc_name,
            npc: npc_entity,
            player: player_entity,
            row: row.clone(),
            state: StreamState::new(),
            speaker: None,
            hold_logged: None,
            started_at: time.elapsed_secs(),
            steps_fired: 0,
            texts: 0,
            click_releases: 0,
            selected_from_pool: selection.pool_len,
            replayed,
            player_position,
            player_rotation,
            first_tick: true,
        };
        // 窗体开场：层压栈即在场（α=1 无淡入），ResetTexts 清两栏——
        // 本段里随后的 show_talk_window 步被 Approximately(α,1) 短路。
        // 开窗要会话作保（归属构造，见窗体模块注释）——先建会话再开窗，
        // 随后入资源。
        crate::delayed_faces::start_loop(&mut commands);
        window.open(TalkSession::Player(&mut session));
        info!(
            "[talkwin] 对话窗体开场：玩家对话段 {}（即时在场，α=1；名字栏/正文已清）",
            row.talk_id
        );
        commands.insert_resource(session);
    }
}

fn referent_of_general_step(step: &TalkStep) -> Option<&str> {
    match step {
        TalkStep::LookAtBody { who, .. }
        | TalkStep::Voice { who, .. }
        | TalkStep::ChangeNpcEye { who, .. }
        | TalkStep::ChangeNpcMouth { who, .. }
        | TalkStep::ChangeAnimation { who, .. }
        | TalkStep::Emoticon { who, .. }
        | TalkStep::HideEmoticon { who } => Some(who),
        _ => None,
    }
}

fn target_of_general_step(step: &TalkStep) -> Option<&str> {
    match step {
        TalkStep::LookAtBody { target, .. } => Some(target),
        _ => None,
    }
}

/// 抽签种子：检出现场（unit、当前站点、前一段、请求序）喂进 LCG——
/// 同现场同序列，现场变序列跟着变；请求序让连续两次点按不同掷点。
fn selection_seed(unit: u32, site: i32, prev: Option<i32>, request_no: u32) -> u64 {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    seed ^= (unit as u64) << 32;
    seed ^= (site as i64 as u64) << 16;
    seed ^= (prev.unwrap_or(0) as u64).wrapping_mul(0x0100_0019);
    seed ^= (request_no as u64).wrapping_mul(0x9E37_79B9);
    seed
}

const FIXTURE_CAST_COHERENCE_RADIUS: f32 = 4.0;

#[allow(clippy::type_complexity)]
fn validate_fixture_context(
    row: &moly_law::talk::FixtureTalkRow,
    requested: Option<Entity>,
    actors: &[crate::talk::TalkActor],
    npcs: &Query<
        (
            Entity,
            &CharacterUnitId,
            &WalkState,
            &ToonMaterials,
            &crate::npc::NpcActions,
            &crate::npc_objective::TalkSlot,
        ),
        Without<PlayerControlled>,
    >,
    fixtures: &Query<
        (
            Entity,
            &FixtureActivityIdentity,
            &FixturePlacement,
            &Transform,
        ),
        With<FixtureRoot>,
    >,
) -> Result<Vec<(i32, Entity)>, &'static str> {
    validate_fixture_declarations(
        row,
        actors.iter().map(|actor| actor.unit),
        fixtures.iter().filter_map(|(_, identity, placement, _)| {
            (identity.master_id == placement.fixture_id).then_some(identity.master_id)
        }),
    )?;
    let requested = requested.ok_or("fixture content requires a selected real fixture instance")?;
    let (_, requested_identity, requested_placement, _) = fixtures
        .get(requested)
        .map_err(|_| "selected fixture instance left the scene")?;
    if !row.fixture_ids.contains(&requested_identity.master_id)
        || requested_placement.fixture_id != requested_identity.master_id
    {
        return Err("selected fixture instance no longer matches the authored master");
    }

    let mut bindings = Vec::with_capacity(row.fixture_ids.len());
    for fixture_id in &row.fixture_ids {
        let binding = if *fixture_id == requested_identity.master_id {
            Some(requested)
        } else {
            fixtures
                .iter()
                .find(|(_, identity, placement, _)| {
                    identity.master_id == *fixture_id && placement.fixture_id == *fixture_id
                })
                .map(|(entity, _, _, _)| entity)
        };
        let Some(entity) = binding else {
            return Err("an authored fixture instance is missing from the current scene");
        };
        bindings.push((*fixture_id, entity));
    }

    for actor in actors {
        let pairs: Vec<_> = row
            .pairs
            .iter()
            .filter(|pair| pair.unit_id == actor.unit as i32)
            .collect();
        if pairs.is_empty() {
            return Err("an actor has no authored fixture pairing");
        }
        let (_, _, walk, _, _, _) = npcs
            .get(actor.entity)
            .map_err(|_| "an admitted actor left the scene")?;
        let coherent = pairs.iter().any(|pair| {
            bindings
                .iter()
                .find(|(fixture_id, _)| *fixture_id == pair.fixture_id)
                .and_then(|(_, entity)| fixtures.get(*entity).ok())
                .is_some_and(|(_, _, _, transform)| {
                    Vec3::from(walk.0.position).distance(transform.translation)
                        <= FIXTURE_CAST_COHERENCE_RADIUS
                })
        });
        if !coherent {
            return Err("the full fixture cast is not coherently staged at its authored fixture");
        }
    }
    Ok(bindings)
}

fn validate_fixture_declarations(
    row: &moly_law::talk::FixtureTalkRow,
    actor_units: impl IntoIterator<Item = u32>,
    available_fixtures: impl IntoIterator<Item = i32>,
) -> Result<(), &'static str> {
    let actors: HashSet<_> = actor_units.into_iter().collect();
    let fixtures: HashSet<_> = available_fixtures.into_iter().collect();
    if actors.len() != row.unit_ids.len()
        || !row
            .unit_ids
            .iter()
            .all(|unit| actors.contains(&(*unit as u32)))
    {
        return Err("fixture cast does not match the selected row");
    }
    if !row
        .fixture_ids
        .iter()
        .all(|fixture| fixtures.contains(fixture))
    {
        return Err("an authored fixture instance is missing from the current scene");
    }
    if row.pairs.iter().any(|pair| {
        !row.fixture_ids.contains(&pair.fixture_id) || !row.unit_ids.contains(&pair.unit_id)
    }) {
        return Err("fixture cast pairing references an undeclared actor or fixture");
    }
    if row
        .unit_ids
        .iter()
        .any(|unit| !row.pairs.iter().any(|pair| pair.unit_id == *unit))
    {
        return Err("an actor has no authored fixture pairing");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 步进与事件分发
// ---------------------------------------------------------------------------

/// Update：步进主循环（同配对对话侧的形状）：发起点按的闩耗尽、
/// 窗体点击放行门、`advance` 一拍、逐触发步分发、对话动作播完交还、
/// 段收口。挂在窗体输入之后（闩先于步进）。
#[allow(clippy::type_complexity)]
pub(crate) fn advance_session(
    mut commands: Commands,
    time: Res<Time>,
    session: Option<ResMut<PlayerTalkSession>>,
    tables: Option<Res<FacialTables>>,
    store: Option<Res<PlayerTalkStore>>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    mut playback: crate::talk::TalkPlayback,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut window: ResMut<TalkWindowState>,
    mut emote_reqs: ResMut<TalkEmoteReqs>,
    window_roots: Query<Entity, With<TalkWindowRoot>>,
    mut npcs: Query<
        (
            &mut MotionPhase,
            &mut WalkState,
            &mut MotionDriver,
            &Transform,
        ),
        Without<PlayerControlled>,
    >,
    mut runtimes: Query<&mut PlayerTalkRuntime>,
    live_entities: Query<()>,
    mut ledger: ResMut<PlayerTalkLedger>,
) {
    let explicit_cancel = drain_talk_cancellation(&mut playback.cancel);
    let Some(mut session) = session else {
        return;
    };
    // 拆成一条 &mut 再走字段：row（只读侧）与 state/账目（可写侧）是
    // 同一结构的互斥字段，经它取借用才分得开。
    let session = &mut *session;
    let interrupted =
        live_entities.get(session.npc).is_err() || live_entities.get(session.player).is_err();
    if explicit_cancel || interrupted {
        if interrupted {
            warn!(
                "[player-talk] talk {} interrupted because an admitted actor disappeared",
                session.talk_id
            );
        }
        let crate::talk::TalkPlayback {
            mut players,
            mut transitions,
            mut delayed_faces,
            ..
        } = playback;
        delayed_faces.cancel_talk(session.talk_id);
        if let (Some(tables), Ok(runtime)) = (tables.as_deref(), runtimes.get(session.npc)) {
            reset_default_face(
                session.unit,
                tables,
                &mut materials,
                &runtime.eye_handle,
                &runtime.mouth_handle,
            );
        }
        emote_reqs.hides.push(session.npc);
        restore_cancelled_player_rotation(&mut commands, session.player, session.talk_id);
        reset_cancelled_player_status(&mut commands);
        finish_session(
            &mut commands,
            session,
            &mut window,
            &window_roots,
            &mut npcs,
            &mut players,
            &mut transitions,
            &mut ledger,
            time.elapsed_secs(),
        );
        return;
    }
    let (Some(tables), Some(store)) = (tables, store) else {
        return;
    };
    let Some(lib) = libraries.get(&library.gltf) else {
        return;
    };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let crate::talk::TalkPlayback {
        mut graphs,
        mut players,
        mut transitions,
        mut delayed_faces,
        ..
    } = playback;

    // This backend and the fixture-script backend are mutually exclusive
    // owners. Only the owner present this frame resumes the shared clock.
    for command in delayed_faces.advance(dt) {
        apply_delayed_face(session, command, &tables, &mut materials, &runtimes);
    }

    // 发起点按的闩耗尽：输入系统按会话在场置闩（见窗体模块），发起帧
    // 会话尚未入资源、闩不会置——此块是发起击不进首个点跳门的语义
    // 兜底（输入门与本块同一条归属）。
    if session.first_tick {
        session.first_tick = false;
        if !window.latch_idle() {
            window.consume_click(TalkSession::Player(&mut *session));
            info!("[player-talk] 发起点按的闩耗尽（该击已被发起消费，不进首个 wait_click）");
        }
    }

    // Normal conversation starts with auto mode disabled. WaitClick has no
    // elapsed-time fallback: only the window's input latch may release it.
    // 放行门 = IsWaitClick：闩置位且打字机已收尾。
    let click = window.click_level();
    // 放行前快照：此刻是否挂在 wait_click 上（律的挂起域私有，按游标
    // 回看一步判——挂起记账同一条判法）。
    let holding_click = session.state.is_holding()
        && matches!(
            session
                .row
                .steps
                .get(session.state.cursor.saturating_sub(1)),
            Some(TalkStep::WaitClick)
        );
    let holding_auto_wait = session.state.is_holding()
        && matches!(
            session
                .row
                .steps
                .get(session.state.cursor.saturating_sub(1)),
            Some(TalkStep::WaitTimeOnAutoMode { .. })
        );

    // 一拍步进。
    let fired = match advance(&session.row.steps, &mut session.state, dt, click) {
        Ok(fired) => fired,
        Err(reason) => {
            warn!(
                "[player-talk] 段 {} 的步进被拒（{reason:?}）：时钟停住",
                session.talk_id
            );
            return;
        }
    };
    session.steps_fired += fired.len();
    if holding_click && click {
        // The UI wait has completed, unlike a click that only reveals text.
        // Flush before any following Lua command can enqueue a new face.
        for command in delayed_faces.wait_clicked() {
            apply_delayed_face(session, command, &tables, &mut materials, &runtimes);
        }
        // 该击即耗尽：WaitClicked 放行后清闩（真源同款）。
        window.consume_click(TalkSession::Player(&mut *session));
        session.click_releases += 1;
        // advance can already be parked at the next wait_click this frame.
        // Reset the log edge so an adjacent wait is still announced once.
        session.hold_logged = None;
        info!(
            "[player-talk] 段 {} wait_click 放行（点击闩耗尽；IsWaitClick 门过）",
            session.talk_id
        );
    } else if holding_auto_wait && click {
        // This timed wait also awaits the UI's WaitClicked in manual mode.
        // Consume that click, but do not run WaitClick's engine-only face flush.
        window.consume_click(TalkSession::Player(&mut *session));
        session.hold_logged = None;
        info!(
            "[player-talk] 段 {} wait_time_on_auto_mode 手动跳过（点击闩耗尽）",
            session.talk_id
        );
    }

    // 挂起侧账：游标回看一步（等待步消费后游标已过它）；步型变化才记
    // 一行（挂起期间逐帧记账是刷屏不是证据）。
    if session.state.is_holding() {
        let parked = session
            .row
            .steps
            .get(session.state.cursor.saturating_sub(1));
        let word = parked.map(|step| step.op()).unwrap_or("<unknown>");
        if session.hold_logged != Some(word) {
            session.hold_logged = Some(word);
            match parked {
                Some(TalkStep::WaitClick) => {
                    info!(
                        "[player-talk] 段 {} 步 {} 挂起 wait_click：等待窗体点击；无输入保持当前台词",
                        session.talk_id,
                        session.state.cursor - 1,
                    );
                }
                Some(TalkStep::WaitTime { seconds, .. }) => {
                    info!(
                        "[player-talk] 段 {} 步 {} 驻留 wait_time {:.2}s",
                        session.talk_id,
                        session.state.cursor - 1,
                        seconds,
                    );
                }
                Some(TalkStep::WaitTimeOnAutoMode { seconds }) => {
                    info!(
                        "[player-talk] 段 {} 步 {} 驻留 wait_time_on_auto_mode {:.2}s",
                        session.talk_id,
                        session.state.cursor - 1,
                        seconds,
                    );
                }
                _ => {}
            }
        }
    } else {
        session.hold_logged = None;
    }

    for &index in &fired {
        let step = session.row.steps[index].clone();
        if let Some(who) = referent_of(&step) {
            session.speaker = resolve_speaker(session, who, index);
        }
        dispatch_step(
            &mut commands,
            session,
            &step,
            index,
            &store,
            lib,
            &mut graphs,
            &mut delayed_faces,
            &mut window,
            &mut emote_reqs,
            &mut npcs,
            &mut runtimes,
            &mut players,
            &mut transitions,
        );
    }

    release_finished_animations(session, &mut npcs, &mut runtimes, &players);

    if is_finished(session.row.steps.len(), &session.state) {
        // OnComplete is due-only; Dispose below stops the clock but retains
        // future commands for the next engine owner, including the other backend.
        for command in delayed_faces.on_complete() {
            apply_delayed_face(session, command, &tables, &mut materials, &runtimes);
        }
        emote_reqs.hides.push(session.npc);
        if let Ok(runtime) = runtimes.get(session.npc) {
            reset_default_face(
                session.unit,
                &tables,
                &mut materials,
                &runtime.eye_handle,
                &runtime.mouth_handle,
            );
        }
        finish_session(
            &mut commands,
            session,
            &mut window,
            &window_roots,
            &mut npcs,
            &mut players,
            &mut transitions,
            &mut ledger,
            now,
        );
        ledger.completed += 1;
    }
}

fn apply_delayed_face(
    session: &PlayerTalkSession,
    command: FaceCommand,
    tables: &FacialTables,
    materials: &mut Assets<CharacterMaterial>,
    runtimes: &Query<&mut PlayerTalkRuntime>,
) {
    if command.unit != session.unit as i64 {
        warn!(
            "[talk-face] source={} step={} unit={} is absent from current talk {}",
            command.source_talk, command.source_step, command.unit, session.talk_id,
        );
        return;
    }
    let Ok(runtime) = runtimes.get(session.npc) else {
        return;
    };
    command.apply(
        tables,
        materials,
        &runtime.eye_handle,
        &runtime.mouth_handle,
    );
}

/// Convert the authored referent to a logical id without requiring that id to
/// be in today's cast. Delayed callbacks perform the cast lookup when they run.
fn delayed_face_unit(store: &PlayerTalkStore, who: &str) -> Option<i64> {
    if let Some(id) = who.strip_prefix("id:") {
        return id.parse().ok();
    }
    match who.strip_prefix("Characters.") {
        Some("Player") => Some(0),
        Some(name) => store.names.get(name).map(|unit| *unit as i64),
        None => None,
    }
}

/// 指称解析到说话人：玩家槽 → Player；本段 NPC 名 → Npc；其余悬空
/// （多角色名不出现在单角色对话语料里，出现即悬空记日志）。
fn resolve_speaker(session: &PlayerTalkSession, who: &str, index: usize) -> Option<Speaker> {
    match participant_of(session, who) {
        Some(Participant::Player) => Some(Speaker::Player),
        Some(Participant::Npc) => Some(Speaker::Npc(session.unit)),
        None => {
            warn!(
                "[player-talk] 段 {} 步 {} 的指称 {} 不在本段参演面（名 {:?}）：说话人悬空",
                session.talk_id, index, who, session.npc_name
            );
            None
        }
    }
}

/// 参演者位（指称 → 玩家或本段 NPC；其余悬空）。两代产物形状在此
/// 归一：字符串形（`Characters.Player` / `Characters.<名>`）、数字形的
/// 内部形（`id:0`＝玩家、`id:<unit>`＝名册成员）、空串＝无指称。
fn participant_of(session: &PlayerTalkSession, who: &str) -> Option<Participant> {
    if who == "Characters.Player" || who == "id:0" {
        return Some(Participant::Player);
    }
    if let Some(id) = who.strip_prefix("id:") {
        return match id.parse::<u32>() {
            Ok(unit) if unit == session.unit => Some(Participant::Npc),
            _ => None,
        };
    }
    match who.strip_prefix("Characters.") {
        Some(name) if !name.is_empty() && name == session.npc_name => Some(Participant::Npc),
        _ => None,
    }
}

enum Participant {
    Npc,
    Player,
}

/// 步的指称载荷（`who`；无指称的步为 None）。
fn referent_of(step: &TalkStep) -> Option<&str> {
    match step {
        TalkStep::LookAtBody { who, .. }
        | TalkStep::Voice { who, .. }
        | TalkStep::ChangeNpcEye { who, .. }
        | TalkStep::ChangeNpcMouth { who, .. }
        | TalkStep::ChangeAnimation { who, .. }
        | TalkStep::Emoticon { who, .. }
        | TalkStep::HideEmoticon { who } => Some(who.as_str()),
        _ => None,
    }
}

/// 一步的分发：按 op 落到呈现通道。日志只带 id/步号/指称/键名。
#[allow(clippy::too_many_arguments)]
fn dispatch_step(
    commands: &mut Commands,
    session: &mut PlayerTalkSession,
    step: &TalkStep,
    index: usize,
    store: &PlayerTalkStore,
    lib: &Gltf,
    graphs: &mut Assets<AnimationGraph>,
    delayed_faces: &mut DelayedFaces,
    window: &mut TalkWindowState,
    emote_reqs: &mut TalkEmoteReqs,
    npcs: &mut Query<
        (
            &mut MotionPhase,
            &mut WalkState,
            &mut MotionDriver,
            &Transform,
        ),
        Without<PlayerControlled>,
    >,
    runtimes: &mut Query<&mut PlayerTalkRuntime>,
    players: &mut Query<&mut AnimationPlayer>,
    transitions: &mut Query<&mut AnimationTransitions>,
) {
    let talk_id = session.talk_id;
    let step_word = step.op();
    match step {
        TalkStep::LookAtBody {
            who,
            target,
            duration,
        } => {
            let (who_part, target_part) = (
                participant_of(session, who),
                participant_of(session, target),
            );
            let (Some(who_part), Some(target_part)) = (who_part, target_part) else {
                warn!(
                    "[player-talk] 段 {} 步 {} look_at_body who={:?} target={:?}：指称悬空，不转身",
                    talk_id, index, who, target,
                );
                return;
            };
            // 目标位：NPC 取位移律状态位（权威位）；玩家取会话快照
            // （参演期间位移推进被持留跳过，快照即现值）。
            let target_pos: Option<[f32; 3]> = match target_part {
                Participant::Npc => npcs
                    .get(session.npc)
                    .map(|(_, walk, _, _)| walk.0.position)
                    .ok(),
                Participant::Player => Some(session.player_position),
            };
            let Some(target_pos) = target_pos else {
                warn!(
                    "[player-talk] 段 {} 步 {} look_at_body：目标位不可读，不转身",
                    talk_id, index
                );
                return;
            };
            match who_part {
                Participant::Npc => {
                    let motion = turn_npc_toward(session.npc, target_pos, *duration, npcs);
                    info!(
                        "[player-talk] 段 {} 步 {} look_at_body who=npc target={:?}：转身 {:.2}s（{}）",
                        talk_id, index, target, duration, motion.label(),
                    );
                }
                Participant::Player => {
                    // 起点取会话朝向账（上一次玩家转身已把账同步到其
                    // 目标——连环转身从上一目标起算），转身件落实体，
                    // 账同步到本步目标。
                    session.player_rotation = begin_player_turn(
                        commands,
                        session.player,
                        session.player_rotation,
                        session.player_position,
                        target_pos,
                        *duration,
                    );
                    info!(
                        "[player-talk] 段 {} 步 {} look_at_body who=player target={:?}：玩家转身 {:.2}s（朝向直设插值，无转身段——玩家域同款）",
                        talk_id, index, target, duration,
                    );
                }
            }
        }
        TalkStep::WaitTime { .. } | TalkStep::WaitTimeOnAutoMode { .. } | TalkStep::WaitClick => {
            // 驻留步不进触发表（步进律：等待步挂起、即时步发射）；
            // 此处不可达，防御记账。
            warn!(
                "[player-talk] 段 {} 步 {} {step_word} 出现在触发表（步进律口径不符）",
                talk_id, index
            );
        }
        TalkStep::Label { name } => {
            // 名字栏 SetText：锚点跳转的语料零出现，此 op 在窗体侧是
            // 名字栏写（真源 Label → 名字栏文本组件）。
            window.set_label(TalkSession::Player(&mut *session), name);
            info!(
                "[player-talk] 段 {} 步 {} label：名字栏 SetText（{} 字，名字不进日志）",
                talk_id,
                index,
                name.chars().count(),
            );
        }
        TalkStep::Voice { channel, cue, who } => {
            if !matches!(participant_of(session, who), Some(Participant::Npc)) {
                warn!(
                    "[player-talk] 段 {} 步 {} voice who={}：无有效NPC说话者，不起播",
                    talk_id,
                    index,
                    ascii_or(who),
                );
                return;
            }
            commands.spawn(VoiceLine {
                talk_id,
                step: index,
                cue: cue.clone(),
                who: Some(VoiceWho::Participant(session.unit as i32)),
                speaker: Some(VoiceSpeaker::Npc(session.npc)),
            });
            info!(
                "[player-talk] 段 {} 步 {} voice channel={} cue={} who={}（投递共用语音通道）",
                talk_id,
                index,
                ascii_or(channel),
                ascii_or(cue),
                who,
            );
        }
        TalkStep::ChangeNpcEye {
            who,
            pattern,
            delay_seconds,
            ..
        }
        | TalkStep::ChangeNpcMouth {
            who,
            pattern,
            delay_seconds,
            ..
        } => {
            let Some(unit) = delayed_face_unit(store, who) else {
                warn!("[player-talk] source={talk_id} step={index} {step_word} has an unresolved character id");
                return;
            };
            let slot = if matches!(step, TalkStep::ChangeNpcEye { .. }) {
                FaceSlot::Eye
            } else {
                FaceSlot::Mouth
            };
            delayed_faces.enqueue(
                *delay_seconds,
                FaceCommand {
                    unit,
                    slot,
                    pattern: pattern.clone(),
                    source_talk: talk_id,
                    source_step: index,
                },
            );
        }
        TalkStep::ChangeAnimation {
            who,
            motion,
            speed,
            playback_speed,
            play_end_motion,
            ..
        } => {
            match participant_of(session, who) {
                Some(Participant::Npc) => {}
                Some(Participant::Player) => {
                    info!(
                        "[player-talk] 段 {} 步 {} animation who={} motion={}：玩家剧情动作接口尚未接此步，不起播",
                        talk_id, index, who, ascii_or(motion),
                    );
                    return;
                }
                None => {
                    info!(
                        "[player-talk] 段 {} 步 {} animation who={} motion={}：悬空指称，不起播",
                        talk_id,
                        index,
                        who,
                        ascii_or(motion),
                    );
                    return;
                }
            }
            // 缺段按真源：动作段名为空/悬空时 `ChangeAnimationAsync` 整步
            // 短路（在播段保持）。本侧段名非空但不在共享动作库（含产物
            // 未解析的常量指称形——动作常量表非恒等映射，装载期不剥），
            // 同形降级：记警告不起播。
            let clip = format!("{motion}_S");
            if !lib.named_animations.contains_key(clip.as_str()) {
                warn!(
                    "[player-talk] 段 {} 步 {} animation motion={} 不在共享动作库：不起播（在播段保持，真源空段名同形跳过）",
                    talk_id, index, ascii_or(motion),
                );
                return;
            }
            if *play_end_motion {
                warn!(
                    "[player-talk] 段 {} 步 {} 动作步带 playEndMotion=true：End 段跟随未实现，本步只播主段",
                    talk_id, index,
                );
            }
            let Ok(mut runtime) = runtimes.get_mut(session.npc) else {
                return;
            };
            let Ok((_, _, mut driver, _)) = npcs.get_mut(session.npc) else {
                warn!(
                    "[player-talk] 段 {} 步 {} animation who={}：成员不在名册驱动里，不起播",
                    talk_id, index, who
                );
                return;
            };
            // 起播段取 `_S` 变体（段族约定同待机动作族：基名＋后缀）；
            // 缺段已在上方拦截，此处必命中。
            let node = node_for(
                &mut runtime.nodes,
                graphs,
                lib,
                &driver.graph,
                &clip,
                session.unit,
            );
            let mut player = players
                .get_mut(driver.player)
                .expect("装配系统应已插上播放器");
            let player = &mut *player;
            let transitions = transitions
                .get_mut(driver.player)
                .expect("装配系统应已插上过渡组件")
                .into_inner();
            let animation = transitions.play(player, node, SEGMENT_BLEND);
            // 速度换算走律（0 哨值读作 1.0）；playbackSpeed 载荷当前不
            // 进换算（源里它与 speed 并存、语义未取证），记日志对账。
            let effective = effective_animation_speed(*speed);
            animation.set_speed(effective as f32);
            runtime.anim_node = Some(node);
            driver.playing = None;
            driver.alone_holds = true;
            info!(
                "[player-talk] 段 {} 步 {} animation who={} motion={}：起播（速度 {:.2}（载荷 speed {:?}），混合 {:.2}s，playbackSpeed 载荷 {:.2}）",
                talk_id, index, who, ascii_or(motion), effective, speed,
                SEGMENT_BLEND.as_secs_f32(), playback_speed,
            );
        }
        TalkStep::Text { text } => {
            session.texts += 1;
            window.show_text(TalkSession::Player(&mut *session), text);
            let speaker_word = match session.speaker {
                Some(Speaker::Npc(unit)) => format!("npc({unit})"),
                Some(Speaker::Player) => "player".to_owned(),
                None => "悬空".to_owned(),
            };
            info!(
                "[player-talk] 段 {} 步 {} text：进窗体打字机（{} 字；说话人 {} 不参与摆位）",
                talk_id,
                index,
                text.chars().count(),
                speaker_word,
            );
        }
        TalkStep::Emoticon {
            who,
            name,
            show_seconds,
            ..
        } => {
            match participant_of(session, who) {
                Some(Participant::Npc) => {
                    emote_reqs
                        .shows
                        .push((session.npc, name.clone(), *show_seconds as f32));
                    info!(
                    "[player-talk] 段 {} 步 {} emoticon who={} name={} showSeconds={:.1}（出件走表情件通道）",
                    talk_id, index, who, ascii_or(name), show_seconds,
                );
                }
                _ => {
                    info!(
                    "[player-talk] 段 {} 步 {} emoticon who={} name={}：玩家侧/悬空指称，不出件",
                    talk_id, index, who, ascii_or(name),
                );
                }
            }
        }
        TalkStep::HideEmoticon { who } => match participant_of(session, who) {
            Some(Participant::Npc) => {
                emote_reqs.hides.push(session.npc);
                info!(
                    "[player-talk] 段 {} 步 {} hide_emoticon who={}（收件走表情件通道）",
                    talk_id, index, who,
                );
            }
            _ => {
                info!(
                    "[player-talk] 段 {} 步 {} hide_emoticon who={}：玩家侧/悬空指称，不收件",
                    talk_id, index, who,
                );
            }
        },
        TalkStep::ShowTalkWindow => {
            // α 已 ≈1 时被 Approximately 短路（对话开场即在场——常例）；
            // 被 hide 过再 show 走 0.3s 淡入。
            match window.show(TalkSession::Player(&mut *session)) {
                Some(from) => {
                    info!(
                        "[player-talk] 段 {} 步 {} show_talk_window：α={from:.2} → 1 淡入 0.3s（OutQuad）",
                        talk_id, index
                    );
                }
                None => {
                    info!(
                        "[player-talk] 段 {} 步 {} show_talk_window：α≈1 即时（Approximately 短路，无淡入）",
                        talk_id, index
                    );
                }
            }
        }
        TalkStep::HideTalkWindow => {
            window.hide(TalkSession::Player(&mut *session));
            info!(
                "[player-talk] 段 {} 步 {} hide_talk_window：α→0 淡出 0.5s（OutQuad），窗体留场",
                talk_id, index
            );
        }
    }
}

/// NPC 转身：从当前旋转（Transform 现值——转体中途的连环转身从当前角
/// 起算）到面向目标位——段差折角选转身段（转身律），终点取同叶短弧
/// （位移换腿的 depart 同一条式）。相位进转体（动画侧读它播转身段），
/// 逐帧推进归本模块（位移推进被持留跳过）。
fn turn_npc_toward(
    entity: Entity,
    target_pos: [f32; 3],
    duration: f64,
    npcs: &mut Query<
        (
            &mut MotionPhase,
            &mut WalkState,
            &mut MotionDriver,
            &Transform,
        ),
        Without<PlayerControlled>,
    >,
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
    let angle = angle_between((from * Vec3::Z).to_array(), to_forward);
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

/// 玩家转身：从当前朝向（会话朝向账）到面向目标位，按步时长插值（缓动
/// 同位移转体的补间缺省：先快后慢的二次出线）。写入 PlayerTurn 组件，
/// 逐帧推进归本模块（玩家位移推进被持留跳过）；返回步目标朝向（会话
/// 朝向账同步用——连环转身从上一目标起算）。
fn begin_player_turn(
    commands: &mut Commands,
    player: Entity,
    from: Quat,
    own_pos: [f32; 3],
    target_pos: [f32; 3],
    duration: f64,
) -> Quat {
    let to_forward = facing_direction(own_pos, target_pos);
    let mut to = Quat::from_rotation_arc(Vec3::Z, Vec3::from(to_forward));
    if from.dot(to) < 0.0 {
        to = Quat::from_xyzw(-to.x, -to.y, -to.z, -to.w);
    }
    commands.entity(player).insert(PlayerTurn {
        from,
        to,
        duration: duration as f32,
        elapsed: 0.0,
    });
    to
}

/// 对话动作播完交还播放器（位移驱动接管，回待机段）。
fn release_finished_animations(
    session: &mut PlayerTalkSession,
    npcs: &mut Query<
        (
            &mut MotionPhase,
            &mut WalkState,
            &mut MotionDriver,
            &Transform,
        ),
        Without<PlayerControlled>,
    >,
    runtimes: &mut Query<&mut PlayerTalkRuntime>,
    players: &Query<&mut AnimationPlayer>,
) {
    let Ok(mut runtime) = runtimes.get_mut(session.npc) else {
        return;
    };
    let Some(node) = runtime.anim_node else {
        return;
    };
    let player_entity = npcs
        .get(session.npc)
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
        return;
    }
    runtime.anim_node = None;
    if let Ok((_, _, mut driver, _)) = npcs.get_mut(session.npc) {
        driver.alone_holds = false;
        driver.playing = None;
    }
    info!(
        "[player-talk] 段 {} unit={} 对话动作播完：播放器交还位移驱动（回待机）",
        session.talk_id, session.unit
    );
}

/// 段收口：撤持留与运行时、撤窗体（层弹出无淡出）、相位回驻留、待机
/// 归位、转体未完把朝向钉到转身终点、前一对话记账与收口日志。
#[allow(clippy::too_many_arguments)]
fn finish_session(
    commands: &mut Commands,
    session: &mut PlayerTalkSession,
    window: &mut TalkWindowState,
    window_roots: &Query<Entity, With<TalkWindowRoot>>,
    npcs: &mut Query<
        (
            &mut MotionPhase,
            &mut WalkState,
            &mut MotionDriver,
            &Transform,
        ),
        Without<PlayerControlled>,
    >,
    players: &mut Query<&mut AnimationPlayer>,
    transitions: &mut Query<&mut AnimationTransitions>,
    _ledger: &mut PlayerTalkLedger,
    now: f32,
) {
    crate::delayed_faces::stop_loop(commands);
    crate::audio::dispose_talk_voice(commands);
    // NPC：转体未完钉到终点（律状态 forward 同步——撤持留后行走不
    // 回跳），回驻留 + 待机归位。
    let turn_end = npcs
        .get(session.npc)
        .ok()
        .and_then(|(phase, _, _, _)| match phase {
            MotionPhase::Turning { to, .. } => Some(*to),
            _ => None,
        });
    if let Some(to) = turn_end {
        let fwd = to * Vec3::Z;
        if let Ok((_, mut walk, _, _)) = npcs.get_mut(session.npc) {
            walk.0.forward = [fwd.x, fwd.y, fwd.z];
        }
    }
    if let Ok((mut phase, _, mut driver)) = npcs.get_mut(session.npc).map(|(p, w, d, _)| (p, w, d))
    {
        *phase = MotionPhase::Dwelling { remaining: None };
        driver.alone_holds = false;
        driver.playing = None;
        play_idle(&mut driver, players, transitions, session.unit);
    }
    if let Ok(mut entity) = commands.get_entity(session.npc) {
        entity.remove::<TalkHold>().remove::<PlayerTalkRuntime>();
        drop(entity);
        crate::npc::leave_player_talk(commands, session.npc);
    }
    // 玩家：撤持留与转身件（相位已是驻留；朝向保持当前值——玩家域
    // 朝向直设，下一次移动自会重设朝向，无律状态要同步）。
    if let Ok(mut entity) = commands.get_entity(session.player) {
        entity
            .remove::<TalkHold>()
            .remove::<PlayerTurn>()
            .remove::<PlayerTalkRotationSnapshot>();
    }
    // 窗体撤下：层弹出路径无淡出，即时收树（α 回 1 供下次开场短路）。
    for root in window_roots {
        commands.entity(root).despawn();
    }
    window.close(TalkSession::Player(&mut *session));
    info!("[talkwin] 对话窗体撤下（层弹出：即时，无淡出）");
    let selection_word = if session.replayed {
        format!("重播前一段（池 {}）", session.selected_from_pool)
    } else {
        format!("均匀抽（池 {} 段）", session.selected_from_pool)
    };
    info!(
        "[player-talk] 段收口：段 id={}（unit {}，{}）播完——步事件 {}、文本 {}、点跳放行 {}，用时 {:.1}s；AI Current/Previous保持",
        session.talk_id,
        session.unit,
        selection_word,
        session.steps_fired,
        session.texts,
        session.click_releases,
        now - session.started_at,
    );
    commands.remove_resource::<PlayerTalkSession>();
}

/// ResetFacial reads the authored unit defaults, applying mouth before eye.
pub(crate) fn reset_default_face(
    unit: u32,
    tables: &FacialTables,
    materials: &mut Assets<CharacterMaterial>,
    eye: &Handle<CharacterMaterial>,
    mouth: &Handle<CharacterMaterial>,
) {
    let Some((eye_pattern, mouth_pattern)) = tables.default_patterns(unit) else {
        warn!("[player-talk] unit {unit} has no authored default facial row");
        return;
    };
    // Pattern lookup uses the same zero-valued missing-row convention as the
    // existing script ChangeEye/ChangeMouth consumers; no default name is made up.
    let mouth_row = tables.lip_pattern(mouth_pattern).unwrap_or_default();
    apply_mouth_pattern(materials, mouth, mouth_row);
    let eye_index = tables.eye_open(eye_pattern).unwrap_or(0);
    apply_eye(materials, eye, Some(&eye_index));
}

// ---------------------------------------------------------------------------
// 转身逐帧推进（参演者持留期间，位移推进被跳过）
// ---------------------------------------------------------------------------

/// Update：NPC 参演者的转体逐帧走（位置冻结，朝向按步时长插值，缓动
/// 同位移转体的补间缺省），完成帧相位回驻留并把终点朝向回写律状态；
/// 玩家的转身插值同拍（朝向直写 Transform，相位不动——玩家域无转体
/// 相位）。
pub(crate) fn progress_turns(
    time: Res<Time>,
    mut commands: Commands,
    mut npcs: Query<
        (&mut MotionPhase, &mut WalkState, &mut Transform),
        (With<PlayerTalkRuntime>, Without<PlayerControlled>),
    >,
    mut player: Query<(Entity, &mut Transform, &mut PlayerTurn), With<PlayerControlled>>,
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
            let fwd = *to * Vec3::Z;
            walk.0.forward = [fwd.x, fwd.y, fwd.z];
            *phase = MotionPhase::Dwelling { remaining: None };
            info!("[player-talk] 转体完成（对话侧）：朝向回写，相位回驻留");
        }
    }
    for (entity, mut transform, mut turn) in &mut player {
        turn.elapsed += dt;
        let t = (turn.elapsed / turn.duration.max(1e-6)).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        transform.rotation = turn.from.slerp(turn.to, eased);
        if turn.elapsed >= turn.duration {
            commands.entity(entity).remove::<PlayerTurn>();
            info!("[player-talk] 玩家转身完成（朝向直设插值）");
        }
    }
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

/// Update（周期 5s，run_if 门控）：会话状态行 + 累计账——「玩家对话
/// 真的在播」的现算证据（步进游标、文本数、点跳放行数）。
pub(crate) fn report(session: Option<Res<PlayerTalkSession>>, ledger: Res<PlayerTalkLedger>) {
    match session {
        Some(session) => {
            info!(
                "[player-talk] 状态：段 id={}（unit {}）进行中——步 {}/{}（触发 {}）、文本 {}、点跳放行 {}、说话人 {:?}；累计：请求 {} 选取 {} 重播 {} 拒绝 {} 占用跳过 {} 收口 {}",
                session.talk_id,
                session.unit,
                session.state.cursor,
                session.row.steps.len(),
                session.steps_fired,
                session.texts,
                session.click_releases,
                session.speaker,
                ledger.requests,
                ledger.selections,
                ledger.replays,
                ledger.refusals,
                ledger.busy_skips,
                ledger.completed,
            );
        }
        None => {
            info!(
                "[player-talk] 状态：无会话在播；累计：请求 {} 选取 {} 重播 {} 拒绝 {} 占用跳过 {} 收口 {}",
                ledger.requests,
                ledger.selections,
                ledger.replays,
                ledger.refusals,
                ledger.busy_skips,
                ledger.completed,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 冒烟口：合成点按走拾取同链
// ---------------------------------------------------------------------------

/// Update（手势链内、拾取前）：冒烟口（`MOLY_PLAYER_TALK_AUTOTAP_SECS`，
/// 同仓拾取/采集物仪表同款）——窗口期内每隔 [`SMOKE_TAP_INTERVAL`] 秒
/// 对一名**有通用对话候选**的在场 NPC 合成一次 TAP：世界位投影到屏面、
/// 按该屏位发布手势事件，走与真实输入**同一条**链（投影 → 射线 →
/// 圆柱测试 → NPC 沿 → 请求 → 六门 → 会话）。任一对话会话在播时不
/// 注入（点按会被跳过，注入只是噪声）；目标按候选 unit 轮转点名。
#[allow(clippy::type_complexity)]
pub(crate) fn smoke_autotap(
    mut events: MessageWriter<GestureEvent>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    npcs: Query<(Entity, &Transform, &CharacterUnitId), Without<PlayerControlled>>,
    candidates: Option<Res<PlayerTalkCandidates>>,
    session: Option<Res<PlayerTalkSession>>,
    active_talk: Option<Res<crate::talk::ActiveTalk>>,
    time: Res<Time>,
    mut next_at: Local<f32>,
    mut index: Local<usize>,
) {
    let armed = env_secs("MOLY_PLAYER_TALK_AUTOTAP_SECS");
    if armed <= 0.0 || time.elapsed_secs() >= armed || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + SMOKE_TAP_INTERVAL;
    if session.is_some() || active_talk.is_some() {
        return; // 会话在播：注入的点按只会被跳过，等下一拍
    }
    let Some(candidates) = candidates else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    // 候选 unit 的在场成员按实体序排定，循环点名。
    let mut targets: Vec<(Entity, Vec3, u32)> = npcs
        .iter()
        .filter(|(_, _, unit)| candidates.units.iter().any(|(u, _)| *u == unit.0))
        .map(|(entity, transform, unit)| (entity, transform.translation, unit.0))
        .collect();
    targets.sort_by(|a, b| a.0.cmp(&b.0));
    if targets.is_empty() {
        return; // 候选成员不在场（站点未铺完）：等下一拍
    }
    let Some((_, world, unit)) = targets.get(*index % targets.len()) else {
        return;
    };
    *index += 1;
    let Some(position) = camera.world_to_viewport(camera_transform, *world).ok() else {
        info!("[player-talk-smoke] unit {unit} 的世界位投影失败（相机视口外），跳过");
        return;
    };
    events.write(GestureEvent {
        kind: GestureKind::Tap,
        state: GestureState::End,
        position,
        delta: Vec2::ZERO,
        ui_owned: false,
    });
    info!(
        "[player-talk-smoke] 合成 TAP @({:.0},{:.0})（投影 unit {unit} 的世界位，事件面注入）",
        position.x, position.y
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_law::talk::{FixtureTalkRow, PairRow};

    fn tweet() -> TweetRef {
        TweetRef {
            id: 1,
            text: String::new(),
            motion: String::new(),
            eye: String::new(),
            mouth: String::new(),
        }
    }

    fn fixture_row() -> FixtureTalkRow {
        FixtureTalkRow {
            talk_id: 77,
            lua: String::new(),
            form: 0,
            site_group_id: 1,
            term_id: 1,
            condition_group_id: 1,
            fixture_ids: vec![423],
            unit_ids: vec![13, 14],
            pairs: vec![
                PairRow {
                    fixture_id: 423,
                    unit_id: 13,
                },
                PairRow {
                    fixture_id: 423,
                    unit_id: 14,
                },
            ],
            tweet: tweet(),
            voices: Vec::new(),
            steps: Vec::new(),
        }
    }

    #[test]
    fn resolved_content_preserves_exact_backend_and_master_id() {
        let general = ResolvedTalk::General {
            unit: 1,
            row: TalkRow {
                talk_id: 77,
                lua: String::new(),
                site_group_id: 1,
                term_id: 1,
                conditions: Vec::new(),
                condition_values: Vec::new(),
                tweet: tweet(),
                voices: Vec::new(),
                steps: Vec::new(),
            },
        }
        .content();
        let fixture = ResolvedTalk::Fixture(fixture_row()).content();
        assert_eq!(general.master_id, 77);
        assert_eq!(general.backend, crate::npc_objective::TalkBackend::General);
        assert_eq!(fixture.master_id, 77);
        assert_eq!(fixture.backend, crate::npc_objective::TalkBackend::Fixture);
    }

    #[test]
    fn fixture_declarations_reject_missing_actor_and_fixture_references() {
        let row = fixture_row();
        assert!(validate_fixture_declarations(&row, [13], [423]).is_err());
        assert!(validate_fixture_declarations(&row, [13, 14], []).is_err());
        assert!(validate_fixture_declarations(&row, [13, 14], [423]).is_ok());
    }

    #[derive(Resource, Default)]
    struct CancelProbe {
        owner_active: bool,
        cancellations: usize,
    }

    fn cancellation_probe(
        mut requests: MessageReader<TalkCancelRequest>,
        mut probe: ResMut<CancelProbe>,
    ) {
        let cancelled = drain_talk_cancellation(&mut requests);
        if probe.owner_active && cancelled {
            probe.cancellations += 1;
        }
    }

    #[test]
    fn idle_cancellation_is_drained_before_a_new_owner_is_admitted() {
        let mut app = App::new();
        app.add_message::<TalkCancelRequest>()
            .init_resource::<CancelProbe>()
            .add_systems(Update, cancellation_probe);
        app.world_mut().write_message(TalkCancelRequest);
        app.update();
        app.world_mut().resource_mut::<CancelProbe>().owner_active = true;
        app.update();
        assert_eq!(app.world().resource::<CancelProbe>().cancellations, 0);
        app.world_mut().write_message(TalkCancelRequest);
        app.update();
        assert_eq!(app.world().resource::<CancelProbe>().cancellations, 1);
    }
}

/// 环境变量秒数（缺省 0）：与各域冒烟钩子同款读法。
fn env_secs(name: &str) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .map(|v| v.max(0.0) as f32)
        .unwrap_or(0.0)
}

/// 日志守卫：非 ASCII 内容换占位符（同对话域的网关纪律）。
fn ascii_or(value: &str) -> &str {
    if value.is_ascii() {
        value
    } else {
        "<non-ascii>"
    }
}
