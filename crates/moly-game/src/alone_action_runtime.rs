//! One source-ordered Rest script per character and Rest lifetime.
//!
//! The extracted program selects the control-flow family and ordered blocks.
//! Integer probability draws, script-start wall-second windows, coroutine waits,
//! block-tail memory writes and unconditional loop tails live in the law layer.
//! This host supplies actual Rest epochs and sends the same script's animation,
//! face and ordered emote commands to the existing presentation channels.
//!
//! CutScene's global command gate is explicit but awaits a scene-state producer;
//! it must not be inferred from camera mode or from an open menu.

use std::collections::HashMap;
use std::time::Duration;

use bevy::animation::graph::{AnimationGraph, AnimationNodeIndex};
use bevy::animation::AnimationPlayer;
use bevy::asset::{AssetPath, Assets, LoadState};
use bevy::gltf::Gltf;
use bevy::prelude::*;

use moly_assets::json::JsonAsset;
use moly_law::alone_action as law;
use moly_law::facial::LipPattern;

use crate::character::{MotionDriver, MotionKind, MotionLibrary, SEGMENT_BLEND};
use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::npc::CharacterUnitId;

// ---- 图集几何（三个骨架档案的 eyeAtlas/mouthAtlas 逐值相同） ----

/// 眼图集每格的 UV 尺寸（角色侧 gltf 空间，V 为正；家具脸侧同格异
/// 号——见 `fixture_talk.rs` 的 ST 式）。
pub(crate) const EYE_CELL: [f32; 2] = [0.25, 0.125];
/// 口图集每格的 UV 尺寸（角色侧 gltf 空间，V 为正；家具脸侧同格异
/// 号——见 `fixture_talk.rs` 的 ST 式）。
pub(crate) const MOUTH_CELL: [f32; 2] = [0.25, 0.25];

/// 图集格序（源换格式）：下标先夹到 ≥1（0、-1 与 1 同选第 1 格），再
/// 减一得零基；列 = 零基 % 4，行 = 零基 / 4。
fn cell_of(index: i32) -> (i32, i32) {
    let clamped = if index < 2 { 1 } else { index };
    let zero = clamped - 1;
    (zero % 4, zero / 4)
}

// ---- 装载 ----

/// 已请求装载的待机动作表（解析后撤）。
#[derive(Resource)]
pub(crate) struct AloneActionsHandle {
    actions: Handle<JsonAsset>,
    characters: Handle<JsonAsset>,
}

/// 已请求装载的 facial 两表（解析后撤）。
#[derive(Resource)]
pub(crate) struct FacialTablesHandle(Handle<JsonAsset>);

/// 解析后的待机动作表：单位 id → 场景池 + 尾表。
#[derive(Resource)]
pub(crate) struct AloneActions {
    units: HashMap<u32, UnitPool>,
}

struct UnitPool {
    asset: String,
    program: law::Program,
    scenarios: Vec<law::Scenario>,
    tail: Vec<law::Step>,
}

impl FacialTables {
    /// Authored per-unit defaults; absence is not a request for a guessed face.
    pub(crate) fn default_patterns(&self, unit: u32) -> Option<(&str, &str)> {
        self.defaults.get(&unit).map(|(eye, mouth)| (eye.as_str(), mouth.as_str()))
    }

    /// 眼表查键：pattern → open 格值（缺键 None——消费侧响亮失败用）。
    /// 对话步的眼图样与待机动作共用同一张眼表（键即 `PatternName`）。
    pub(crate) fn eye_open(&self, pattern: &str) -> Option<i32> {
        self.eye.get(pattern).copied()
    }

    /// 口型表查键：pattern → close 格值（缺键 None）。
    pub(crate) fn lip_close(&self, pattern: &str) -> Option<i32> {
        self.lip.get(pattern).map(|row| row.close)
    }

    /// Preserve Open/Middle/Close for the audio-driven mouth continuation.
    pub(crate) fn lip_pattern(&self, pattern: &str) -> Option<LipPattern> {
        self.lip.get(pattern).copied()
    }
}

/// Facial tables and authored defaults. Static writers still use Close;
/// preserve the full lip row for the separately owned speech continuation.
#[derive(Resource)]
pub(crate) struct FacialTables {
    eye: HashMap<String, i32>,
    lip: HashMap<String, LipPattern>,
    defaults: HashMap<u32, (String, String)>,
}

/// Startup：请求装载两份 JSON（内联路径——moly-assets 没有这两张表的
/// 取件函数，本单不碰那个 crate；角色骨架档案同款先例）。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(AloneActionsHandle {
        actions: server.load::<JsonAsset>("moly://alone-actions.json"),
        characters: server.load::<JsonAsset>("moly://characters.json"),
    });
    commands.insert_resource(FacialTablesHandle(
        server.load::<JsonAsset>(AssetPath::from("moly://facial-tables.json".to_owned())),
    ));
}

/// Update：待机动作表到齐即解析成律行。装载失败响亮失败。
pub(crate) fn parse_alone_actions(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<AloneActionsHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.actions) {
        panic!("待机动作表装载失败：{err:?}");
    }
    if let LoadState::Failed(err) = server.load_state(&handle.characters) {
        panic!("角色待机脚本映射装载失败：{err:?}");
    }
    let (Some(json), Some(characters)) =
        (jsons.get(&handle.actions), jsons.get(&handle.characters))
    else {
        return;
    };
    let mut actions = parse_actions(&json.0);
    let bindings: serde_json::Value =
        serde_json::from_str(&characters.0).expect("角色待机脚本映射不是合法 JSON");
    let rows = bindings
        .get("characters")
        .and_then(|v| v.as_object())
        .expect("角色待机脚本映射缺 characters");
    actions.units.retain(|unit, pool| {
        let Some(binding) = rows
            .get(&unit.to_string())
            .and_then(|row| row.get("soloAction"))
            .and_then(|v| v.as_str())
        else {
            return false;
        };
        assert_eq!(
            pool.asset.strip_suffix(".lua").unwrap_or(&pool.asset),
            binding.strip_suffix(".lua").unwrap_or(binding),
            "角色待机脚本映射与提取资产不一致"
        );
        true
    });
    let units = actions.units.len();
    let scenarios: usize = actions
        .units
        .values()
        .map(|pool| pool.scenarios.len())
        .sum();
    info!("待机动作表就绪：{units} 单位 {scenarios} 场景（行结构已折成律类型）");
    commands.insert_resource(actions);
    commands.remove_resource::<AloneActionsHandle>();
}

/// Update：facial 两表到齐即解析。装载失败响亮失败。
pub(crate) fn parse_facial_tables(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<FacialTablesHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("facial 两表装载失败：{err:?}");
    }
    let Some(json) = jsons.get(&handle.0) else {
        return;
    };
    let tables = parse_facial(&json.0);
    info!(
        "facial 两表就绪：眼 {} 行 口 {} 行 默认脸 {} 单位",
        tables.eye.len(),
        tables.lip.len(),
        tables.defaults.len()
    );
    commands.insert_resource(tables);
    commands.remove_resource::<FacialTablesHandle>();
}

// ---- JSON → 律行 ----

fn parse_actions(text: &str) -> AloneActions {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|err| panic!("待机动作表不是合法 JSON：{err}"));
    assert_eq!(
        value.get("version").and_then(|v| v.as_u64()),
        Some(3),
        "待机动作需要带源循环与包装调用顺序的 v3 提取产物"
    );
    let units = value
        .get("units")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("待机动作表缺 units 对象"));
    let mut map = HashMap::with_capacity(units.len());
    for (key, unit) in units {
        let unit_id: u32 = key
            .parse()
            .unwrap_or_else(|_| panic!("待机动作表的单位键不是数字 id：{key}"));
        let scenarios = unit
            .get("scenarios")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("单位 {unit_id} 缺 scenarios 数组"));
        let mut rows = Vec::with_capacity(scenarios.len());
        for scenario in scenarios {
            let id = string_of(scenario, "id", unit_id);
            let trigger = parse_trigger(
                scenario
                    .get("trigger")
                    .unwrap_or_else(|| panic!("单位 {unit_id} 场景 {id} 缺 trigger")),
                unit_id,
                &id,
            );
            let steps = parse_steps(
                scenario
                    .get("steps")
                    .and_then(|v| v.as_array())
                    .unwrap_or_else(|| panic!("单位 {unit_id} 场景 {id} 缺 steps 数组")),
                unit_id,
                &id,
            );
            rows.push(law::Scenario {
                id: id.to_owned(),
                trigger,
                steps,
            });
        }
        let tail = parse_steps(
            unit.get("tail")
                .and_then(|t| t.get("steps"))
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("单位 {unit_id} 缺 tail.steps 数组")),
            unit_id,
            "tail",
        );
        let program = parse_program(unit, unit_id, &rows, &tail);
        let asset = string_of(unit, "asset", unit_id).to_owned();
        map.insert(
            unit_id,
            UnitPool {
                asset,
                program,
                scenarios: rows,
                tail,
            },
        );
    }
    AloneActions { units: map }
}

fn parse_trigger(value: &serde_json::Value, unit: u32, scenario: &str) -> law::Trigger {
    let kind = string_of(value, "kind", unit);
    match kind {
        "timeGated" => law::Trigger::TimeGated(law::TimeGatedTrigger {
            time_limit_name: string_of(value, "timeLimitName", unit).to_owned(),
            time_limit_seconds: number_of(value, "timeLimitSeconds", unit, scenario),
            probability_name: string_of(value, "probabilityName", unit).to_owned(),
            probability: number_of(value, "probability", unit, scenario),
            motion_slot: value
                .get("motionSlotValue")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string()),
            slot_memory_seconds: number_of(value, "slotMemorySeconds", unit, scenario),
        }),
        "randomBranch" => law::Trigger::RandomBranch(law::RandomBranchTrigger {
            low: number_of(value, "low", unit, scenario),
            high: number_of(value, "high", unit, scenario),
            weight: number_of(value, "weight", unit, scenario),
        }),
        other => panic!("单位 {unit} 场景 {scenario} 的触发 kind {other} 不在闭集"),
    }
}

fn parse_program(
    unit: &serde_json::Value,
    unit_id: u32,
    scenarios: &[law::Scenario],
    tail: &[law::Step],
) -> law::Program {
    let value = unit.get("program").expect("待机脚本缺 program");
    let kind = match value.get("kind").and_then(|v| v.as_str()) {
        Some("sequentialIfs") => law::ProgramKind::SequentialIfs,
        Some("randomBranch") => law::ProgramKind::RandomBranch,
        other => panic!("unit {unit_id} 的程序族不支持：{other:?}"),
    };
    assert_eq!(
        value.get("clock").and_then(|v| v.as_str()),
        Some("wallSeconds")
    );
    assert_eq!(value.get("loopRandomMin").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(
        value.get("loopRandomMax").and_then(|v| v.as_u64()),
        Some(99)
    );
    assert_eq!(
        value.get("captureLoopTime").and_then(|v| v.as_bool()),
        Some(kind == law::ProgramKind::SequentialIfs)
    );
    let blocks: Vec<_> = value
        .get("blocks")
        .and_then(|v| v.as_array())
        .expect("待机脚本缺 blocks")
        .iter()
        .map(|block| {
            let scenario = block
                .get("scenario")
                .and_then(|v| v.as_u64())
                .expect("待机 block 缺 scenario") as usize;
            let row = scenarios.get(scenario).expect("待机 block 引用不在场景表");
            let remember_slot = block
                .get("rememberSlot")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string());
            match (&row.trigger, kind) {
                (law::Trigger::TimeGated(trigger), law::ProgramKind::SequentialIfs) => {
                    if remember_slot.is_some() {
                        assert_eq!(
                            remember_slot, trigger.motion_slot,
                            "unit {unit_id} 的段尾写入指向另一个槽"
                        );
                    }
                }
                (law::Trigger::RandomBranch(_), law::ProgramKind::RandomBranch) => {
                    assert!(remember_slot.is_none());
                }
                _ => panic!("unit {unit_id} 的场景类型与程序族不一致"),
            }
            law::ProgramBlock {
                scenario,
                remember_slot,
            }
        })
        .collect();
    assert_eq!(blocks.len(), scenarios.len(), "待机程序丢失场景");
    let mut indices: Vec<_> = blocks.iter().map(|block| block.scenario).collect();
    indices.sort_unstable();
    assert!(
        indices.iter().copied().eq(0..scenarios.len()),
        "待机程序场景重复或缺失"
    );
    assert!(
        tail.iter()
            .any(|step| matches!(step, law::Step::Wait { seconds, .. } if *seconds > 0.0)),
        "待机循环尾必须有让出点"
    );
    law::Program { kind, blocks }
}

fn parse_steps(steps: &[serde_json::Value], unit: u32, scenario: &str) -> Vec<law::Step> {
    steps
        .iter()
        .map(|step| {
            let op = string_of(step, "op", unit);
            let t = number_of(step, "t", unit, scenario) as f32;
            match op {
                "eye" => law::Step::ChangeEye {
                    t,
                    pattern: string_of(step, "pattern", unit).to_owned(),
                    alias: string_of(step, "alias", unit).to_owned(),
                },
                "mouth" => law::Step::ChangeMouth {
                    t,
                    pattern: string_of(step, "pattern", unit).to_owned(),
                    alias: string_of(step, "alias", unit).to_owned(),
                },
                "animation" => law::Step::ChangeAnimation {
                    t,
                    motion: string_of(step, "motion", unit).to_owned(),
                    alias: string_of(step, "alias", unit).to_owned(),
                    speed: number_of(step, "speed", unit, scenario) as f32,
                    playback_speed: number_of(step, "playbackSpeed", unit, scenario) as f32,
                    play_end_motion: step
                        .get("playEndMotion")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    blend: number_of(step, "blend", unit, scenario) as f32,
                    phase: parse_suffix(step, "phase", unit),
                    phase_source: parse_suffix(step, "phaseSource", unit),
                },
                "emoticon" => law::Step::ShowEmoticon {
                    t,
                    name: string_of(step, "name", unit).to_owned(),
                    show_seconds: step.get("showSeconds").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                    time_arg: step.get("time").and_then(|v| v.as_f64()),
                    not_play_se_arg: step.get("notPlaySe").and_then(|v| v.as_bool()),
                    host_not_play_se: step
                        .get("hostNotPlaySe")
                        .and_then(|v| v.as_bool())
                        .expect("待机表情步缺宿主 notPlaySe 参数"),
                },
                "hideEmoticon" => law::Step::HideEmoticon { t },
                "wait" => law::Step::Wait {
                    t,
                    seconds: number_of(step, "seconds", unit, scenario) as f32,
                },
                other => panic!("单位 {unit} 场景 {scenario} 的步 op {other} 不在闭集"),
            }
        })
        .collect()
}

fn parse_suffix(step: &serde_json::Value, key: &str, unit: u32) -> Option<law::SegmentSuffix> {
    match step.get(key).and_then(|v| v.as_str()) {
        None => None,
        Some("S") => Some(law::SegmentSuffix::S),
        Some("L") => Some(law::SegmentSuffix::L),
        Some("E") => Some(law::SegmentSuffix::E),
        Some("O") => Some(law::SegmentSuffix::O),
        Some(other) => panic!("单位 {unit} 的步后缀 {other} 不在闭集"),
    }
}

fn string_of<'a>(value: &'a serde_json::Value, key: &str, unit: u32) -> &'a str {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("单位 {unit} 的行缺字符串列 {key}"))
}

fn number_of(value: &serde_json::Value, key: &str, unit: u32, scenario: &str) -> f64 {
    value
        .get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("单位 {unit} 场景 {scenario} 的行缺数值列 {key}"))
}

fn parse_facial(text: &str) -> FacialTables {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|err| panic!("facial 两表不是合法 JSON：{err}"));
    let mut eye = HashMap::new();
    for row in value
        .get("NPCAvatarEyeData")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("facial 表缺 NPCAvatarEyeData 数组"))
    {
        let name = row
            .get("PatternName")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("眼表有无名行"));
        let open = row
            .get("OpenEyeIndex")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("眼表行 {name} 缺 OpenEyeIndex"));
        eye.insert(name.to_owned(), open as i32);
    }
    let mut lip = HashMap::new();
    for row in value
        .get("NPCAvatarLipSyncData")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("facial 表缺 NPCAvatarLipSyncData 数组"))
    {
        let name = row
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("口型表有无名行"));
        let close = row
            .get("CloseLipSyncIndex")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("口型表行 {name} 缺 CloseLipSyncIndex"));
        let open = row.get("OpenLipSyncIndex").and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("口型表行 {name} 缺 OpenLipSyncIndex"));
        let middle = row.get("MiddleLipSyncIndex").and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("口型表行 {name} 缺 MiddleLipSyncIndex"));
        lip.insert(name.to_owned(), LipPattern { open: open as i32, middle: middle as i32, close: close as i32 });
    }
    let mut defaults = HashMap::new();
    for row in value
        .get("NPCAvatarDefaultFacialData")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("facial 表缺 NPCAvatarDefaultFacialData 数组"))
    {
        let unit = row
            .get("CharacterUnitId")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| panic!("默认脸表缺 CharacterUnitId"));
        let eye_name = row
            .get("EyePatternName")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("默认脸表单位 {unit} 缺 EyePatternName"));
        let mouth_name = row
            .get("MouthPatternName")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("默认脸表单位 {unit} 缺 MouthPatternName"));
        defaults.insert(unit as u32, (eye_name.to_owned(), mouth_name.to_owned()));
    }
    FacialTables { eye, lip, defaults }
}

// ---- 随机源 ----

/// 跨帧线性同余抽签（整数 0..99）：状态随调用前进、种子含 unitId，
/// 跨帧连续——律只约束分布与掷点次数，引擎序列不在律内。
pub(crate) struct Lcg(u64);

impl Lcg {
    /// 种子：unitId 占高位，低位常数错开（各成员序列独立）。
    pub(crate) fn seeded(unit_id: u32) -> Self {
        Self(((unit_id as u64) << 32) | 0x00a10e_u64)
    }

    fn percent(&mut self) -> u32 {
        self.below(100)
    }

    fn below(&mut self, upper: u32) -> u32 {
        assert!(upper != 0, "uniform draw requires a nonempty interval");
        // Rejection sampling avoids modulo bias without changing the count of
        // logical script draws. The PRNG stream is not an authored asset.
        loop {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let value = (self.0 >> 32) as u32;
            if value < u32::MAX - u32::MAX % upper {
                return value % upper;
            }
        }
    }
}

impl moly_law::talk::UniformDraw for Lcg {
    fn draw(&mut self, length: usize) -> usize {
        let upper = u32::try_from(length).expect("draw interval exceeds existing PRNG width");
        self.below(upper) as usize
    }
}

/// Logical integer draws made by the script, including the loop-head draw.
struct CountedDraw<'a> {
    inner: &'a mut Lcg,
    values: Vec<u32>,
}

impl law::PercentDraw for CountedDraw<'_> {
    fn percent(&mut self) -> u32 {
        let value = self.inner.percent();
        self.values.push(value);
        value
    }
}

/// A script invocation is rebuilt on every actual Rest entry. Asset references,
/// animation nodes and the random stream remain on the character.
#[derive(Component)]
pub(crate) struct AloneRuntime {
    program: Option<law::Program>,
    scenarios: Vec<law::Scenario>,
    tail: Vec<law::Step>,
    rng: Lcg,
    script: Option<law::ScriptState>,
    rest_epoch: Option<u64>,
    seg: SegPlay,
    eye_handle: Handle<CharacterMaterial>,
    mouth_handle: Handle<CharacterMaterial>,
    eye_pattern: String,
    mouth_pattern: String,
    nodes: HashMap<String, AnimationNodeIndex>,
    steps_fired: usize,
}

/// CutScene is the source's only global command exclusion. The current scene
/// host has no CutScene executor; that executor must supply this input when it
/// is added. Ordinary menus and Edit are not CutScene and must not be guessed
/// into this gate.
#[derive(Resource, Default)]
pub(crate) struct AloneExecutionGate {
    pub(crate) cutscene_active: bool,
}

enum SegPlay {
    /// 序列还没点播过动作段。
    Empty,
    /// 在播 `_S`（基名），播完接 `_L` 循环。
    Start {
        base: String,
        speed: f32,
        blend: f32,
    },
    /// 在播 `_L` 循环（终态：循环到序列回收）。
    Looping,
    /// 在播单段（带后缀变体），播一遍停在段尾（终态）。
    Once,
}

/// Update：两份表到齐 + 装配完成（有驱动与 toon 材质）的成员逐名挂
/// 运行时。挂上即核数据面：池里用到的 facial 键与动作段必须全部在
/// 表/库里——核不过响亮失败（事件分发时才查就是静默丢步）。
#[allow(clippy::type_complexity)]
pub(crate) fn attach(
    mut commands: Commands,
    actions: Option<Res<AloneActions>>,
    tables: Option<Res<FacialTables>>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    npcs: Query<
        (Entity, &CharacterUnitId, &ToonMaterials),
        (
            With<MotionDriver>,
            Without<AloneRuntime>,
            Without<crate::player::PlayerControlled>,
        ),
    >,
) {
    let (Some(actions), Some(tables)) = (actions, tables) else {
        return;
    };
    let Some(lib) = libraries.get(&library.gltf) else {
        return; // 动作库还没到齐（有驱动即已到齐，此行只是防序）
    };
    for (npc, unit, toon) in &npcs {
        let pool = actions.units.get(&unit.0);
        if let Some(pool) = pool {
            for step in pool
                .scenarios
                .iter()
                .flat_map(|scenario| scenario.steps.iter())
                .chain(pool.tail.iter())
            {
                verify_step(step, &tables.eye, &tables.lip, lib, unit.0);
            }
        }
        let eye_handle = toon
            .slot_handle("eye")
            .unwrap_or_else(|| panic!("unit {} 的角色材质槽没有 eye", unit.0))
            .clone();
        let mouth_handle = toon
            .slot_handle("mouth")
            .unwrap_or_else(|| panic!("unit {} 的角色材质槽没有 mouth", unit.0))
            .clone();
        let (eye_name, mouth_name) = tables
            .defaults
            .get(&unit.0)
            .unwrap_or_else(|| panic!("facial 默认脸表没有 unit {}", unit.0));
        apply_eye(&mut materials, &eye_handle, tables.eye.get(eye_name));
        apply_mouth_pattern(&mut materials, &mouth_handle, tables.lip.get(mouth_name).copied().unwrap_or_default());
        commands.entity(npc).insert(AloneRuntime {
            program: pool.map(|pool| pool.program.clone()),
            scenarios: pool.map(|pool| pool.scenarios.clone()).unwrap_or_default(),
            tail: pool.map(|pool| pool.tail.clone()).unwrap_or_default(),
            rng: Lcg::seeded(unit.0),
            script: None,
            rest_epoch: None,
            seg: SegPlay::Empty,
            eye_handle,
            mouth_handle,
            eye_pattern: eye_name.clone(),
            mouth_pattern: mouth_name.clone(),
            nodes: HashMap::new(),
            steps_fired: 0,
        });
        info!(
            "[alone] unit={} 脚本绑定={}（仅真实 Rest 进入时启动）",
            unit.0,
            pool.is_some()
        );
    }
}

/// 装载期核验一步：facial 键在两表里、动作段的基名与所需后缀段在
/// 动作库里。缺什么报什么（键名/段名与单位 id——不带资产内容）。
fn verify_step(
    step: &law::Step,
    eye: &HashMap<String, i32>,
    lip: &HashMap<String, LipPattern>,
    lib: &Gltf,
    unit: u32,
) {
    match step {
        law::Step::ChangeEye { pattern, .. } => {
            if !eye.contains_key(pattern) {
                panic!("unit {} 的待机步要点眼键 {pattern}，眼表里没有", unit);
            }
        }
        law::Step::ChangeMouth { pattern, .. } => {
            if !lip.contains_key(pattern) {
                panic!("unit {} 的待机步要点口型键 {pattern}，口型表里没有", unit);
            }
        }
        law::Step::ChangeAnimation { motion, phase, .. } => {
            let names = match phase {
                // 带后缀：只点那一段。
                Some(suffix) => vec![format!("{motion}_{}", suffix_letter(*suffix))],
                // 无后缀：`_S` 起播、播完接 `_L`——两段都要在。
                None => vec![format!("{motion}_S"), format!("{motion}_L")],
            };
            for name in names {
                if !lib.named_animations.contains_key(name.as_str()) {
                    panic!("共享动作库没有段 {name}（unit {} 的待机步要点播它）", unit);
                }
            }
        }
        law::Step::ShowEmoticon { .. }
        | law::Step::HideEmoticon { .. }
        | law::Step::Wait { .. } => {}
    }
}

fn suffix_letter(suffix: law::SegmentSuffix) -> &'static str {
    match suffix {
        law::SegmentSuffix::S => "S",
        law::SegmentSuffix::L => "L",
        law::SegmentSuffix::E => "E",
        law::SegmentSuffix::O => "O",
    }
}

// ---- 通道：eye / mouth ----

/// 写眼材质的 ST 槽（open 格），返回格坐标（日志对账用）。对话步的眼
/// 图样切换与本模块共用这一条通道（同一张眼表、同一个 ST 写法）。
pub(crate) fn apply_eye(
    materials: &mut Assets<CharacterMaterial>,
    handle: &Handle<CharacterMaterial>,
    open: Option<&i32>,
) -> (i32, i32) {
    let index = open.copied().unwrap_or(1);
    let (col, row) = cell_of(index);
    let st = [1.0, 1.0, col as f32 * EYE_CELL[0], row as f32 * EYE_CELL[1]];
    if let Some(material) = materials.get_mut(handle) {
        material.params.main_tex_st = st;
    }
    (col, row)
}

/// Write one mouth atlas index. Speech continuations use this same material
/// path without changing the full authored pattern or its revision.
pub(crate) fn apply_mouth(
    materials: &mut Assets<CharacterMaterial>,
    handle: &Handle<CharacterMaterial>,
    close: Option<&i32>,
) -> (i32, i32) {
    let index = close.copied().unwrap_or(1);
    let (col, row) = cell_of(index);
    let st = [
        1.0,
        1.0,
        col as f32 * MOUTH_CELL[0],
        row as f32 * MOUTH_CELL[1],
    ];
    // UpdateMouth writes Close on idle frames too. Read without marking the
    // asset changed, so an already-selected cell does not upload every frame.
    if materials.get(handle).is_some_and(|material| material.params.main_tex_st != st) {
        if let Some(material) = materials.get_mut(handle) {
            material.params.main_tex_st = st;
        }
    }
    (col, row)
}

/// ChangeLipSyncPattern updates the entire row and immediately writes Close.
/// The material is an instance-local input mailbox, not a second oscillator.
/// Its revision lets the actor-owned continuation retain an already captured
/// pending index while later continuation steps read this newly authored row.
pub(crate) fn apply_mouth_pattern(
    materials: &mut Assets<CharacterMaterial>,
    handle: &Handle<CharacterMaterial>,
    pattern: LipPattern,
) -> (i32, i32) {
    if let Some(material) = materials.get_mut(handle) {
        material.lip_pattern = Some(pattern);
        material.lip_pattern_revision = material.lip_pattern_revision.wrapping_add(1);
    }
    apply_mouth(materials, handle, Some(&pattern.close))
}

// ---- 主循环 ----

/// Execute exactly one script invocation for the current Rest epoch. Other
/// activity owners keep their existing animation and presentation channels.
#[allow(clippy::type_complexity)]
pub(crate) fn advance(
    time: Res<Time>,
    gate: Res<AloneExecutionGate>,
    tables: Option<Res<FacialTables>>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    emotes_ready: Option<Res<crate::emoticon::Emotes>>,
    mut emote_requests: ResMut<crate::emoticon::RestEmoteRequests>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut npcs: Query<(
        Entity,
        &CharacterUnitId,
        &crate::npc::RestLifecycle,
        &mut AloneRuntime,
        &mut MotionDriver,
        Option<&crate::talk::TalkHold>,
    )>,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    let (Some(tables), Some(lib)) = (tables, libraries.get(&library.gltf)) else {
        return;
    };
    let wall = wall_second();
    let now = time.elapsed_secs_f64();
    for (npc, unit, rest, mut runtime, mut driver, talk) in &mut npcs {
        let rt = &mut *runtime;
        let driver = &mut *driver;
        let epoch = rest
            .active_epoch()
            .filter(|_| emotes_ready.is_some() && talk.is_none());
        if rt.rest_epoch != epoch {
            if rt.script.take().is_some() {
                emote_requests.hide(npc);
                rt.seg = SegPlay::Empty;
                // A newly admitted conversation has not emitted its animation
                // commands yet; an already active conversation is never touched.
                driver.alone_holds = false;
                driver.playing = None;
                info!("[alone] unit={} Rest 退出：脚本结束、收表情件", unit.0);
            }
            rt.rest_epoch = epoch;
            if let (Some(epoch), Some(_)) = (epoch, rt.program.as_ref()) {
                rt.script = Some(law::ScriptState::new(wall));
                play_idle(driver, &mut players, &mut transitions, unit.0);
                driver.alone_holds = true;
                info!("[alone] unit={} Rest epoch={epoch}：新脚本启动", unit.0);
            }
        }
        let (Some(program), Some(script)) = (rt.program.as_ref(), rt.script.as_mut()) else {
            continue;
        };
        let mut draw = CountedDraw {
            inner: &mut rt.rng,
            values: Vec::new(),
        };
        let events: Vec<_> = script
            .advance(program, &rt.scenarios, &rt.tail, wall, now, &mut draw)
            .into_iter()
            .cloned()
            .collect();
        if !draw.values.is_empty() {
            trace!("[alone] unit={} 整数掷点 {:?}", unit.0, draw.values);
        }
        for step in events {
            let suffix_call = matches!(&step, law::Step::ChangeAnimation { phase: Some(_), .. });
            // The suffix overload omits the Rest-state check. Script disposal
            // still wins for every overload; CutScene excludes all writes.
            if gate.cutscene_active || (!suffix_call && rest.active_epoch().is_none()) {
                continue;
            }
            rt.steps_fired += 1;
            match step {
                law::Step::ChangeEye { pattern, .. } => {
                    apply_eye(&mut materials, &rt.eye_handle, tables.eye.get(&pattern));
                    rt.eye_pattern = pattern;
                }
                law::Step::ChangeMouth { pattern, .. } => {
                    apply_mouth_pattern(&mut materials, &rt.mouth_handle, tables.lip.get(&pattern).copied().unwrap_or_default());
                    rt.mouth_pattern = pattern;
                }
                law::Step::ChangeAnimation {
                    motion,
                    phase,
                    speed,
                    blend,
                    play_end_motion,
                    ..
                } => {
                    assert!(!play_end_motion, "待机脚本的自动 End 跟随尚无已支持的编排");
                    let clip = format!("{motion}_{}", phase.map(suffix_letter).unwrap_or("S"));
                    let node = node_for(
                        &mut rt.nodes,
                        &mut graphs,
                        lib,
                        &driver.graph,
                        &clip,
                        unit.0,
                    );
                    let mut player = players.get_mut(driver.player).expect("角色播放器未装配");
                    let mut transition = transitions
                        .get_mut(driver.player)
                        .expect("角色过渡器未装配");
                    let speed = law::effective_speed(speed);
                    let playing =
                        transition.play(&mut player, node, Duration::from_secs_f32(blend));
                    playing.set_speed(speed);
                    if phase == Some(law::SegmentSuffix::L) {
                        playing.repeat();
                    }
                    rt.seg = match phase {
                        None => SegPlay::Start {
                            base: motion,
                            speed,
                            blend,
                        },
                        Some(_) => SegPlay::Once,
                    };
                    driver.playing = None;
                    driver.alone_holds = true;
                }
                law::Step::ShowEmoticon {
                    name,
                    host_not_play_se,
                    time_arg,
                    not_play_se_arg,
                    ..
                } => {
                    trace!("[alone] unit={} 表情参数 time={time_arg:?} notPlaySe={not_play_se_arg:?} hostNotPlaySe={host_not_play_se}", unit.0);
                    emote_requests.show(npc, name, host_not_play_se);
                }
                law::Step::HideEmoticon { .. } => emote_requests.hide(npc),
                law::Step::Wait { .. } => unreachable!("等待由脚本游标消费"),
            }
        }
        if let SegPlay::Start { base, speed, blend } = &rt.seg {
            let start = format!("{base}_S");
            let done = rt.nodes.get(&start).is_some_and(|node| {
                players.get(driver.player).is_ok_and(|player| {
                    player
                        .playing_animations()
                        .any(|(id, animation)| id == node && animation.is_finished())
                })
            });
            if done && !gate.cutscene_active {
                let name = format!("{base}_L");
                let speed = *speed;
                let blend = *blend;
                let node = node_for(
                    &mut rt.nodes,
                    &mut graphs,
                    lib,
                    &driver.graph,
                    &name,
                    unit.0,
                );
                let mut player = players.get_mut(driver.player).expect("角色播放器未装配");
                let mut transition = transitions
                    .get_mut(driver.player)
                    .expect("角色过渡器未装配");
                transition
                    .play(&mut player, node, Duration::from_secs_f32(blend))
                    .set_speed(speed)
                    .repeat();
                rt.seg = SegPlay::Looping;
            }
        }
        // This system runs after conversation admission. A Rest state never
        // survives TalkHold, and never steals a continuing dialogue's player.
        if talk.is_none() {
            driver.alone_holds = rt.script.is_some();
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn wall_second() -> i64 {
    (js_sys::Date::now() / 1000.0).floor() as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn wall_second() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_secs() as i64
}

/// 待机段归位：与位移驱动同款过渡参数，`playing` 簿记一并归位——
/// 归位后位移驱动读到「已在播 idle」，不重触发。对话收口也走它把
/// 参演者的姿态归位。
pub(crate) fn play_idle(
    driver: &mut MotionDriver,
    players: &mut Query<&mut AnimationPlayer>,
    transitions: &mut Query<&mut AnimationTransitions>,
    unit: u32,
) {
    let mut player = players
        .get_mut(driver.player)
        .unwrap_or_else(|_| panic!("unit {unit} 的播放器不在（装配系统应已插上）"));
    let transitions = transitions
        .get_mut(driver.player)
        .unwrap_or_else(|_| panic!("unit {unit} 的过渡组件不在（装配系统应已插上）"))
        .into_inner();
    let player = &mut *player;
    let animation = transitions.play(player, driver.idle, SEGMENT_BLEND);
    animation.repeat();
    driver.playing = Some(MotionKind::Idle);
}

/// 取（或首次补进图）一个动作段的节点下标。段名不在库里、图资产不在
/// 都响亮失败——断线不留静默。对话动作段（直接剪辑名，无段族后缀）
/// 与待机动作段共用这一条补图路径。
pub(crate) fn node_for(
    nodes: &mut HashMap<String, AnimationNodeIndex>,
    graphs: &mut Assets<AnimationGraph>,
    library: &Gltf,
    graph_handle: &Handle<AnimationGraph>,
    clip_name: &str,
    unit: u32,
) -> AnimationNodeIndex {
    if let Some(node) = nodes.get(clip_name) {
        return *node;
    }
    let clip = library
        .named_animations
        .get(clip_name)
        .unwrap_or_else(|| panic!("共享动作库没有段 {clip_name}（unit {unit} 的待机步要点播它）"));
    let graph = graphs
        .get_mut(graph_handle)
        .unwrap_or_else(|| panic!("unit {unit} 的动画图资产不在（装配时已建）"));
    let node = graph.add_clip(clip.clone(), 1.0, graph.root);
    nodes.insert(clip_name.to_owned(), node);
    node
}

/// 周期状态行（每 5 秒）：选取/命中/掷点累计、在播场景与时钟、当前脸、
/// Periodic state description; presentation is verified through the renderer.
pub(crate) fn report(
    time: Res<Time>,
    mut tick: Local<f32>,
    npcs: Query<(&CharacterUnitId, &AloneRuntime)>,
) {
    *tick += time.delta_secs();
    if *tick < 5.0 {
        return;
    }
    *tick = 0.0;
    for (unit, runtime) in &npcs {
        if let Some(script) = &runtime.script {
            let current = script
                .current_scenario()
                .map(|i| runtime.scenarios[i].id.as_str())
                .unwrap_or("tail");
            info!("[alone] unit={} Rest={:?} block={} loops={} hits={} draws={} writes={} eye={} mouth={}",
                unit.0, runtime.rest_epoch, current, script.loops, script.selected_blocks,
                script.draws, runtime.steps_fired, runtime.eye_pattern, runtime.mouth_pattern);
        }
    }
}
