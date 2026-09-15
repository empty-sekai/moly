//! 家具对话演出的呈现件：脸部件句柄解析、家具脸的 ST 写、家具侧
//! 转体的逐帧推进、时间轴（动画剪辑序列）的装载与推进。消费方是
//! `talk.rs` 的步进分发（家具 op 族）。
//!
//! * **脸**：eye/mouth 与角色脸共用同一张 facial 表与同一个图集格尺
//!   寸；差别有二。一是格序：家具侧的格下标**原样**进源的 offset 式
//!   （`X = 格宽·(i%4)`，`Y = −格高·⌊i/4⌋`——行向 v 减小方向走），
//!   不做角色侧的下标钳制与移基（那是角色表的下标族，两族不同源）。
//!   二是 uv 朝向：家具 glb 的 uv 是导出原样（authored 空间），与本
//!   管线的左上采样原点在 v 上相反——脸材质由名字判别后在 shader 里
//!   对求和后的 v 取 1−v（`fixture_material.rs` 的解析侧同一条判别
//!   式），所以这里的 Y 可以逐字写源式（负号原样）。
//! * **脸部件解析**：换装把 glb 材质换成逐实体的 `FixtureMaterial`
//!   后，按材质名认脸。脸材质可能被镜像对实例化两份（左右眼各一份，
//!   逐实体各一份句柄），槽里存**全部**句柄、写时逐个写——角色侧的
//!   写是共享材质资产一份见效，这里是逐实体克隆，不写全就会剩一张
//!   旧脸。无脸家具是合法形态（源的待机动画件为空＝静默无操作），
//!   解析结果具名记账。
//! * **转体**：`look_at_to_npc` 的源式 = 家具位置减角色位置取水平分
//!   量做朝向，时长秒内缓动转到位（补间缺省二次出线，对话侧角色转
//!   身同一条式）。时长 0 经同一组件当帧完成。
//! * **时间轴**：源的 `ChangeFixtureTimeline` 是具名时间轴资源（装载
//!   器 + 待机动画件），离线数据里它拆成三张提取产物（tracks：具名
//!   时间轴 → 轨道 pathId；clips：轨道 → 逐剪辑的起点/过渡；clip-
//!   targets：(轨道, 剪辑序) → 模型包内剪辑名）。播放语义按数据形状
//!   落地：待机动画轨道的剪辑按起点排序**顺序播放**，循环旗标轨道的
//!   首剪辑起点标记**从哪一段起循环**（该段播完接 repeat，例如点头
//!   = 动作段 → 待机循环段；三段例 = 起手段 → 收手段 → 待机循环段）。
//!   剪辑名解不到模型包（悬空引用 / 指回时间轴包自身）的时间轴**不
//!   进可播集**，步进点名时响亮拒绝——不发明替身动作。

use std::collections::HashMap;
use std::time::Duration;

use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::{AnimatedBy, AnimationClip, AnimationPlayer};
use bevy::asset::{AssetPath, Assets, LoadState};
use bevy::gltf::{Gltf, GltfMaterialName};
use bevy::prelude::*;

use crate::fixture::{FixturePlacement, FixturePlacements, FixtureRoot, FixtureSource};
use crate::fixture_material::{FixtureMaterial, FixtureMaterialsSwapped};
use crate::talk::TalkStore;
use moly_assets::json::JsonAsset;

/// 构建标记（编译验证用：串进二进制即证本模块参与链接，运行期首拍
/// 打一行作在跑证据）。
const BUILD_MARKER: &str = "fixture-talk-ops";

/// 家具的脸部件句柄（一条摆放一份）。槽里是**全部**实例句柄（脸材质
/// 可被镜像对实例化，逐实体克隆后各有各的句柄）；两槽皆空＝无脸家具
/// （源侧静默无操作的形态）。
#[derive(Component, Default)]
pub(crate) struct FixtureFace {
    pub(crate) eyes: Vec<Handle<FixtureMaterial>>,
    pub(crate) mouths: Vec<Handle<FixtureMaterial>>,
}

/// 家具转体（源 DORotate 的替身形状；from/to 已取同叶短弧）。
#[derive(Component)]
pub(crate) struct FixtureTurn {
    from: Quat,
    to: Quat,
    duration: f32,
    elapsed: f32,
}

#[derive(Component)]
struct FixtureTalkEffects {
    talk_id: i32,
    rotation: Option<Quat>,
    face_offsets: Vec<(Handle<FixtureMaterial>, [f32; 2])>,
    animator: Option<Entity>,
    animation_nodes: Vec<AnimationNodeIndex>,
}

pub(crate) fn start_talk_effects(commands: &mut Commands, fixture: Entity, talk_id: i32) {
    commands.queue(move |world: &mut World| {
        if world.get::<FixtureTalkEffects>(fixture).is_some() {
            return;
        }
        if world.get::<Transform>(fixture).is_none() {
            return;
        }
        world.entity_mut(fixture).insert(FixtureTalkEffects {
            talk_id,
            rotation: None,
            face_offsets: Vec::new(),
            animator: None,
            animation_nodes: Vec::new(),
        });
    });
}

pub(crate) fn remember_talk_rotation(
    commands: &mut Commands,
    fixture: Entity,
    talk_id: i32,
    rotation: Quat,
) {
    commands.queue(move |world: &mut World| {
        let Some(mut effects) = world.get_mut::<FixtureTalkEffects>(fixture) else {
            return;
        };
        if effects.talk_id == talk_id && effects.rotation.is_none() {
            effects.rotation = Some(rotation);
        }
    });
}

pub(crate) fn remember_talk_face(
    commands: &mut Commands,
    fixture: Entity,
    talk_id: i32,
    handles: &[Handle<FixtureMaterial>],
    materials: &Assets<FixtureMaterial>,
) {
    let offsets: Vec<_> = handles
        .iter()
        .filter_map(|handle| {
            materials
                .get(handle)
                .map(|material| (handle.clone(), material.params.main_tex_offset))
        })
        .collect();
    commands.queue(move |world: &mut World| {
        let Some(mut effects) = world.get_mut::<FixtureTalkEffects>(fixture) else {
            return;
        };
        if effects.talk_id != talk_id {
            return;
        }
        for (handle, offset) in offsets {
            if !effects
                .face_offsets
                .iter()
                .any(|(known, _)| *known == handle)
            {
                effects.face_offsets.push((handle, offset));
            }
        }
    });
}

pub(crate) fn remember_talk_animation(
    commands: &mut Commands,
    fixture: Entity,
    talk_id: i32,
    animator: Entity,
    nodes: Vec<AnimationNodeIndex>,
) {
    commands.queue(move |world: &mut World| {
        let Some(mut effects) = world.get_mut::<FixtureTalkEffects>(fixture) else {
            return;
        };
        if effects.talk_id != talk_id {
            return;
        }
        effects.animator = Some(animator);
        for node in nodes {
            if !effects.animation_nodes.contains(&node) {
                effects.animation_nodes.push(node);
            }
        }
    });
}

/// Restore only state owned by this fixture-script generation. A stale cleanup
/// cannot stop a replacement talk or an unrelated activity timeline.
pub(crate) fn finish_talk_effects(commands: &mut Commands, fixture: Entity, talk_id: i32) {
    commands.queue(move |world: &mut World| cleanup_talk_effects(world, fixture, talk_id));
}

fn cleanup_talk_effects(world: &mut World, fixture: Entity, talk_id: i32) {
    let Some(effects) = world.get::<FixtureTalkEffects>(fixture) else {
        return;
    };
    if effects.talk_id != talk_id {
        return;
    }
    let rotation = effects.rotation;
    let face_offsets = effects.face_offsets.clone();
    let animator = effects.animator;
    let animation_nodes = effects.animation_nodes.clone();
    if let (Some(rotation), Some(mut transform)) = (rotation, world.get_mut::<Transform>(fixture)) {
        transform.rotation = rotation;
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<FixtureMaterial>>() {
        for (handle, offset) in face_offsets {
            if let Some(material) = materials.get_mut(&handle) {
                material.params.main_tex_offset = offset;
            }
        }
    }
    if let Some(mut animation) =
        animator.and_then(|entity| world.get_mut::<AnimationPlayer>(entity))
    {
        for node in animation_nodes {
            animation.stop(node);
        }
    }
    world
        .entity_mut(fixture)
        .remove::<FixtureTurn>()
        .remove::<FixtureTimelineSeq>()
        .remove::<FixtureTalkEffects>();
}

impl FixtureTurn {
    /// 起一次转体。时长 ≤ 0 或 NaN 的瞬时档由推进侧当帧完成（源里
    /// 缺时长参按 0 读、走瞬时 LookAt）。
    pub(crate) fn toward(from: Quat, to: Quat, duration: f64) -> Self {
        let duration = if duration.is_finite() && duration > 0.0 {
            duration as f32
        } else {
            0.0
        };
        FixtureTurn {
            from,
            to,
            duration,
            elapsed: 0.0,
        }
    }
}

/// 写一个脸槽的全部材质句柄（格序直用源式：偏移 = (格宽·列，
/// −格高·行)，下标不钳不移）。返回格坐标（日志对账用）。
pub(crate) fn apply_face(
    materials: &mut Assets<FixtureMaterial>,
    handles: &[Handle<FixtureMaterial>],
    index: i32,
    cell: [f32; 2],
) -> (i32, i32) {
    let (col, row) = (index % 4, index / 4);
    let offset = [col as f32 * cell[0], -(row as f32 * cell[1])];
    for handle in handles {
        if let Some(material) = materials.get_mut(handle) {
            material.params.main_tex_offset = offset;
        }
    }
    (col, row)
}

/// Update：换装闩上后，逐摆放解析脸部件并落组件（`FixtureFace` 在身
/// ＝已解析，自闩）。此前每帧空转。
pub(crate) fn discover_faces(
    mut commands: Commands,
    swapped: Option<Res<FixtureMaterialsSwapped>>,
    roots: Query<(Entity, &FixturePlacement), (With<FixtureRoot>, Without<FixtureFace>)>,
    children: Query<&Children>,
    parts: Query<(&MeshMaterial3d<FixtureMaterial>, &GltfMaterialName)>,
    mut first: Local<bool>,
) {
    if swapped.is_none() {
        return;
    }
    if !*first {
        *first = true;
        info!("[fixture-talk] 构建标记 {BUILD_MARKER}：脸部件解析启动（换装已闩）");
    }
    for (root, placement) in roots.iter() {
        let mut face = FixtureFace::default();
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            let Ok((material, name)) = parts.get(entity) else {
                continue;
            };
            // Material roles select animation channels; UV conversion is shared by all fixtures.
            if name.0.contains("eye") {
                face.eyes.push(material.0.clone());
            } else if name.0.contains("mouth") {
                face.mouths.push(material.0.clone());
            }
        }
        info!(
            "[fixture-talk] 摆放 fixture_id {} 的脸部件：eye {} 柄 · mouth {} 柄",
            placement.fixture_id,
            face.eyes.len(),
            face.mouths.len(),
        );
        commands.entity(root).insert(face);
    }
}

/// Update：家具转体逐帧走——朝向按步时长插值（对话侧角色转身同一条
/// 缓动式），完成帧朝向钉终点并撤组件（转体完成不驻留）。
pub(crate) fn progress_turns(
    mut commands: Commands,
    time: Res<Time>,
    mut fixtures: Query<(Entity, &mut FixtureTurn, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut turn, mut transform) in &mut fixtures {
        turn.elapsed += dt;
        let t = (turn.elapsed / turn.duration.max(1e-6)).clamp(0.0, 1.0);
        // 补间库缺省的二次出线（先快后慢）。
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        transform.rotation = turn.from.slerp(turn.to, eased);
        if turn.elapsed >= turn.duration {
            transform.rotation = turn.to;
            commands.entity(entity).remove::<FixtureTurn>();
            info!("[fixture-talk] 家具转体完成：朝向钉终点");
        }
    }
}

// ---------------------------------------------------------------------------
// 时间轴（对话 op `change_fixture_timeline` 的执行面）
// ---------------------------------------------------------------------------

/// 一条具名时间轴的可播计划：待机动画轨道的剪辑按起点排序后的剪辑名
/// 序列、逐段过渡秒（源的 `m_EaseInDuration`）、循环旗标覆盖的首段
/// 下标（`usize::MAX` = 无旗标不循环）。
pub(crate) struct TimelinePlan {
    pub(crate) clips: Vec<String>,
    pub(crate) eases: Vec<f32>,
    pub(crate) loop_at: usize,
}

/// 摆放家具 → 具名时间轴 → 可播计划。装载期由三张提取产物解析一次。
#[derive(Resource, Default)]
pub(crate) struct FixtureTimelines {
    plans: HashMap<i32, HashMap<String, TimelinePlan>>,
}

impl FixtureTimelines {
    /// 取一条可播计划（不在集 = 该时间轴解不到模型包剪辑或悬空引用）。
    pub(crate) fn plan(&self, fixture_id: i32, name: &str) -> Option<&TimelinePlan> {
        self.plans.get(&fixture_id)?.get(name)
    }
}

/// 家具的动画执行面（一条摆放一份）：源 glb 句柄（取剪辑集）、动画
/// 图句柄（装载期自建）、剪辑名 → 图节点缓存、播放器所在实体（场景
/// 内的动画根——装载器把 `AnimatedBy` 指到它）。`player` 为 None＝
/// 该家具模型无骨架动画（时间轴步响亮拒绝，与源「待机动画件为空」
/// 的静默族不同：这里步已经点名要播，缺件要报）。
#[derive(Component)]
pub(crate) struct FixtureAnimation {
    pub(crate) player: Option<Entity>,
    pub(crate) gltf: Handle<Gltf>,
    graph: Handle<AnimationGraph>,
    nodes: HashMap<String, AnimationNodeIndex>,
}

/// 一条时间轴的播放中序列：`next` 是接下来要起的段下标（`next - 1`
/// 是在播段），`loop_at` 起的段播完接 repeat 并撤本组件（循环段是终
/// 态，不再推进）。
#[derive(Component)]
pub(crate) struct FixtureTimelineSeq {
    nodes: Vec<AnimationNodeIndex>,
    eases: Vec<f32>,
    next: usize,
    loop_at: usize,
}

impl FixtureTimelineSeq {
    /// 起一条播放序列：首段已起播（步进侧），`next` 指向第二段。
    pub(crate) fn new(nodes: Vec<AnimationNodeIndex>, eases: Vec<f32>, loop_at: usize) -> Self {
        FixtureTimelineSeq {
            nodes,
            eases,
            next: 1,
            loop_at,
        }
    }
}

/// 已请求装载的时间轴三件套（tracks / clips / clip-targets）逐锚定
/// 家具的清单；全部到齐即解析撤除。
#[derive(Resource)]
pub(crate) struct TimelineAssets(Vec<TimelineRequest>);

/// 装载计划只走一次的闩（解析撤除 `TimelineAssets` 后防止下一帧重
/// 排计划）。
#[derive(Resource)]
pub(crate) struct TimelinesPlanned;

struct TimelineRequest {
    fixture_id: i32,
    model_package: String,
    tracks: Handle<JsonAsset>,
    clips: Handle<JsonAsset>,
    targets: Handle<JsonAsset>,
}

/// Update：剧本表与摆放表都在后，为「锚定摆放 ∩ 语料时间轴步点名」
/// 的家具请求三张时间轴产物（时间轴包名 = 模型包名的前缀置换，实测
/// 全量包对上）。锚定而语料不点名的家具（屏风/沙发/机关家具）不
/// 装载；语料点名而未锚定的同样不装载——播放步到不了它们（现阶段
/// 点名集已全部锚定）。
pub(crate) fn plan_timelines(
    mut commands: Commands,
    server: Res<AssetServer>,
    store: Option<Res<TalkStore>>,
    placements: Res<FixturePlacements>,
    pending: Option<Res<TimelineAssets>>,
    planned: Option<Res<TimelinesPlanned>>,
) {
    if pending.is_some() || planned.is_some() {
        return;
    }
    if placements.site_id() == 0 {
        return;
    }
    let Some(store) = store else {
        return;
    };
    let corpus = store.timeline_fixtures();
    let mut requests = Vec::new();
    for (fixture_id, model_package) in placements.anchored() {
        if !corpus.contains(&fixture_id) {
            continue;
        }
        // 时间轴包名 = 模型包名把 `mysekai__fixture__` 前缀换成
        // `mysekai__fixture_timeline__`（提取侧命名约定，全量对上）。
        let timeline_package = model_package
            .strip_prefix("mysekai__fixture__")
            .map(|rest| format!("mysekai__fixture_timeline__{rest}"))
            .unwrap_or_else(|| panic!("摆放包名不带家具前缀：{model_package}"));
        let load = |kind: &str| {
            server.load::<JsonAsset>(AssetPath::from(format!(
                "moly://fixture-timeline/{kind}/{timeline_package}.json"
            )))
        };
        requests.push(TimelineRequest {
            fixture_id,
            model_package: model_package.to_owned(),
            tracks: load("tracks"),
            clips: load("clips"),
            targets: load("clip-targets"),
        });
    }
    if requests.is_empty() {
        info!("[fixture-talk] 时间轴装载计划：锚定且语料点名的家具 0 个，不装载");
        commands.insert_resource(TimelinesPlanned);
        return;
    }
    info!(
        "[fixture-talk] 时间轴装载计划：锚定且语料点名的家具 {} 个，各请求 tracks/clips/clip-targets 三件",
        requests.len()
    );
    commands.insert_resource(TimelineAssets(requests));
    commands.insert_resource(TimelinesPlanned);
}

/// Update：三件套到齐即解析进 [`FixtureTimelines`]。装载失败响亮失败
/// （锚定 + 语料点名 = 产品路径要件，缺件是数据断点不是静默跳过）。
pub(crate) fn resolve_timelines(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    pending: Option<Res<TimelineAssets>>,
    mut timelines: ResMut<FixtureTimelines>,
) {
    let Some(pending) = pending else {
        return;
    };
    for request in &pending.0 {
        // 逐包独立过门（部分到齐时不重解析已入集的）。
        if timelines.plans.contains_key(&request.fixture_id) {
            continue;
        }
        let states = [
            server.load_state(&request.tracks),
            server.load_state(&request.clips),
            server.load_state(&request.targets),
        ];
        for (kind, state) in ["tracks", "clips", "clip-targets"].iter().zip(&states) {
            if let LoadState::Failed(err) = state {
                panic!(
                    "时间轴产物装载失败（家具 {} 的 {kind}）：{err:?}",
                    request.fixture_id
                );
            }
        }
        if !states.iter().all(|s| matches!(s, LoadState::Loaded)) {
            return;
        }
        let text = |handle: &Handle<JsonAsset>| {
            jsons
                .get(handle)
                .unwrap_or_else(|| panic!("时间轴产物不在资产集（家具 {}）", request.fixture_id))
                .0
                .as_str()
        };
        let plans = parse_timeline_plans(
            text(&request.tracks),
            text(&request.clips),
            text(&request.targets),
            request.fixture_id,
            &request.model_package,
        );
        info!(
            "[fixture-talk] 家具 {} 的时间轴可播集：{} 条（剪辑解不到模型包或悬空引用的具名时间轴不进集）",
            request.fixture_id,
            plans.len()
        );
        timelines.plans.insert(request.fixture_id, plans);
    }
    commands.remove_resource::<TimelineAssets>();
}

/// 三张产物 → 逐具名时间轴的可播计划。规则见模块注释（顺序播放 +
/// 旗标段循环）。解不出（无待机动画轨道 / 任一剪辑目标悬空或指向
/// 时间轴包自身）的时间轴不进集。
fn parse_timeline_plans(
    tracks_text: &str,
    clips_text: &str,
    targets_text: &str,
    fixture_id: i32,
    model_package: &str,
) -> HashMap<String, TimelinePlan> {
    let parse = |text: &str, what: &str| -> serde_json::Value {
        serde_json::from_str(text)
            .unwrap_or_else(|err| panic!("家具 {fixture_id} 的时间轴 {what} 不是合法 JSON：{err}"))
    };
    let tracks = parse(tracks_text, "tracks");
    let clips = parse(clips_text, "clips");
    let targets = parse(targets_text, "clip-targets");

    // 轨道 pathId → 逐剪辑 (m_Start, m_EaseInDuration)（数组序）。
    let mut clip_rows: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
    for track in clips
        .get("tracks")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 clips 缺 tracks 数组"))
    {
        let path_id = track
            .get("pathId")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 clips 轨道缺 pathId"));
        let rows = track
            .get("clips")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 clips 轨道缺 clips 数组"));
        clip_rows.insert(
            path_id,
            rows.iter()
                .map(|clip| {
                    let start = clip
                        .get("m_Start")
                        .and_then(|v| v.as_f64())
                        .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴剪辑缺 m_Start"));
                    let ease = clip
                        .get("m_EaseInDuration")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    (start, ease)
                })
                .collect(),
        );
    }

    // (轨道 pathId, 剪辑序) → 模型包内剪辑名（null / 缺行 = 悬空）。
    let mut clip_names: HashMap<(&str, usize), Option<&str>> = HashMap::new();
    for entry in targets
        .get("keyedClips")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 clip-targets 缺 keyedClips 数组"))
    {
        let path_id = entry
            .get("trackPathId")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的 keyedClips 行缺 trackPathId"));
        let index = entry
            .get("clipIndex")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的 keyedClips 行缺 clipIndex"));
        let name = entry
            .get("target")
            .and_then(|t| t.get("targetPackage"))
            .and_then(|v| v.as_str())
            .filter(|pkg| *pkg == model_package)
            .and_then(|_| {
                entry
                    .get("target")
                    .and_then(|t| t.get("clipName"))
                    .and_then(|v| v.as_str())
            });
        clip_names.insert((path_id, index as usize), name);
    }

    let mut plans = HashMap::new();
    for timeline in tracks
        .get("timelines")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 tracks 缺 timelines 数组"))
    {
        let name = timeline
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴缺 name"));
        let track_list = timeline
            .get("tracks")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("家具 {fixture_id} 的时间轴 {name} 缺 tracks"));
        let path_of = |class: &str| -> Option<&str> {
            track_list
                .iter()
                .find(|track| track.get("class").and_then(|v| v.as_str()) == Some(class))
                .and_then(|track| track.get("pathId"))
                .and_then(|v| v.as_str())
        };
        let Some(idle) = path_of("FixtureIdleAnimationTrack") else {
            // 没有待机动画轨道的时间轴无从起播，不进集。
            continue;
        };
        let Some(rows) = clip_rows.get(idle) else {
            continue;
        };
        // 剪辑按 (起点, 原始序) 排序；逐剪辑解名（悬空/非模型包 → 弃整条）。
        let mut ordered: Vec<(usize, (f64, f64))> = rows.iter().cloned().enumerate().collect();
        ordered.sort_by(|a, b| {
            a.1 .0
                .partial_cmp(&b.1 .0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let mut clip_plan = Vec::with_capacity(ordered.len());
        let mut eases = Vec::with_capacity(ordered.len());
        let mut resolvable = true;
        for &(index, (start, ease)) in &ordered {
            match clip_names.get(&(idle, index)) {
                Some(Some(clip_name)) => {
                    let _ = start;
                    clip_plan.push((*clip_name).to_owned());
                    eases.push(ease as f32);
                }
                _ => {
                    resolvable = false;
                    break;
                }
            }
        }
        if !resolvable || clip_plan.is_empty() {
            continue;
        }
        // 循环旗标：旗标轨道首剪辑的起点标记从哪一段起循环（覆盖 = 起点
        // 不早于旗标起点的首段）；无旗标轨道/无剪辑 = 不循环。
        let loop_at = path_of("FixtureIdleAnimationLoopFlagTrack")
            .and_then(|flag| clip_rows.get(flag))
            .and_then(|rows| {
                rows.iter()
                    .map(|(start, _)| *start)
                    .fold(None::<f64>, |min, start| {
                        Some(match min {
                            Some(m) if m < start => m,
                            _ => start,
                        })
                    })
            })
            .and_then(|flag_start| {
                ordered
                    .iter()
                    .position(|&(_, (start, _))| start >= flag_start - 1e-6)
            })
            .unwrap_or(usize::MAX);
        plans.insert(
            name.to_owned(),
            TimelinePlan {
                clips: clip_plan,
                eases,
                loop_at,
            },
        );
    }
    plans
}

/// Update：换装闩上后，逐摆放解析动画执行面（`FixtureAnimation` 在身
/// ＝已解析，自闩）。装载器把场景内动画目标的 `AnimatedBy` 指到动画
/// 根（自指）；恰一个动画根 = 在它上面插播放器/过渡/图（图空建——
/// 剪辑节点由步进按名补进，与角色侧共享动作库同一条补图路径）。
/// 无动画目标的家具是合法形态（多数摆件无骨架），`player` 留 None。
pub(crate) fn discover_animation(
    mut commands: Commands,
    swapped: Option<Res<FixtureMaterialsSwapped>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    roots: Query<(Entity, &FixtureSource), (With<FixtureRoot>, Without<FixtureAnimation>)>,
    children: Query<&Children>,
    animated_by: Query<&AnimatedBy>,
    mut first: Local<bool>,
) {
    if swapped.is_none() {
        return;
    }
    if !*first {
        *first = true;
        info!("[fixture-talk] 动画执行面解析启动（换装已闩）");
    }
    for (root, source) in roots.iter() {
        // 收动画根（AnimatedBy 指向的实体去重）。
        let mut targets: Vec<Entity> = Vec::new();
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(by) = animated_by.get(entity) {
                if !targets.contains(&by.0) {
                    targets.push(by.0);
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        let player = match targets.len() {
            0 => None,
            1 => Some(targets[0]),
            n => {
                warn!("[fixture-talk] 摆放的家具有 {n} 个动画根（预期 1），时间轴步将拒绝起播");
                None
            }
        };
        let graph = graphs.add(AnimationGraph::new());
        if let Some(target) = player {
            commands.entity(target).insert((
                AnimationPlayer::default(),
                AnimationTransitions::new(),
                AnimationGraphHandle(graph.clone()),
            ));
        }
        commands.entity(root).insert(FixtureAnimation {
            player,
            gltf: source.0.clone(),
            graph,
            nodes: HashMap::new(),
        });
    }
}

/// 取（或首次补进图）一个家具动画剪辑的节点下标（角色侧共享动作库的
/// `node_for` 同一条路径：图节点按剪辑名缓存）。
pub(crate) fn fixture_node_for(
    animation: &mut FixtureAnimation,
    graphs: &mut Assets<AnimationGraph>,
    clip: &Handle<AnimationClip>,
    clip_name: &str,
) -> AnimationNodeIndex {
    if let Some(node) = animation.nodes.get(clip_name) {
        return *node;
    }
    let graph = graphs
        .get_mut(&animation.graph)
        .expect("家具动画图资产在集合里（装载期自建）");
    let node = graph.add_clip(clip.clone(), 1.0, graph.root);
    animation.nodes.insert(clip_name.to_owned(), node);
    node
}

/// Update：时间轴序列逐帧推进——在播段终态即起下一段；旗标段起
/// repeat 并撤组件（循环段是终态）。
pub(crate) fn progress_timelines(
    mut commands: Commands,
    mut fixtures: Query<(Entity, &mut FixtureTimelineSeq, &FixtureAnimation), With<FixtureRoot>>,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    for (root, mut seq, animation) in &mut fixtures {
        let Some(player_entity) = animation.player else {
            continue;
        };
        let current = seq.nodes[seq.next - 1];
        let Ok(mut player) = players.get_mut(player_entity) else {
            continue;
        };
        let player = &mut *player;
        if !player
            .playing_animations()
            .any(|(node, animation)| *node == current && animation.is_finished())
        {
            continue;
        }
        let Ok(transitions) = transitions.get_mut(player_entity) else {
            continue;
        };
        let transitions = transitions.into_inner();
        if seq.next >= seq.nodes.len() {
            // 序列放完且无循环旗标：终态撤组件。
            commands.entity(root).remove::<FixtureTimelineSeq>();
            info!("[fixture-talk] 时间轴序列放完（无循环段，终态）");
            continue;
        }
        let node = seq.nodes[seq.next];
        let ease = seq.eases[seq.next].max(0.0);
        let started = transitions.play(player, node, Duration::from_secs_f32(ease));
        if seq.next == seq.loop_at {
            started.repeat();
            info!(
                "[fixture-talk] 时间轴进循环段（段 {}/{}）",
                seq.next + 1,
                seq.nodes.len()
            );
            seq.next += 1;
            commands.entity(root).remove::<FixtureTimelineSeq>();
        } else {
            info!(
                "[fixture-talk] 时间轴进下一段（段 {}/{}）",
                seq.next + 2,
                seq.nodes.len()
            );
            seq.next += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effects(talk_id: i32, rotation: Quat) -> FixtureTalkEffects {
        FixtureTalkEffects {
            talk_id,
            rotation: Some(rotation),
            face_offsets: Vec::new(),
            animator: None,
            animation_nodes: Vec::new(),
        }
    }

    #[test]
    fn stale_fixture_cleanup_cannot_touch_replacement_owner() {
        let mut world = World::new();
        world.insert_resource(Assets::<FixtureMaterial>::default());
        let replacement = Quat::from_rotation_y(1.0);
        let fixture = world
            .spawn((
                Transform::from_rotation(replacement),
                effects(12, Quat::IDENTITY),
                FixtureTurn::toward(replacement, Quat::IDENTITY, 1.0),
            ))
            .id();

        cleanup_talk_effects(&mut world, fixture, 11);

        assert_eq!(
            world.get::<Transform>(fixture).unwrap().rotation,
            replacement
        );
        assert_eq!(
            world.get::<FixtureTalkEffects>(fixture).unwrap().talk_id,
            12
        );
        assert!(world.get::<FixtureTurn>(fixture).is_some());
    }

    #[test]
    fn owned_fixture_cleanup_restores_rotation_and_removes_only_talk_components() {
        let mut world = World::new();
        world.insert_resource(Assets::<FixtureMaterial>::default());
        let original = Quat::from_rotation_y(-0.5);
        let fixture = world
            .spawn((
                Transform::from_rotation(Quat::from_rotation_y(1.0)),
                effects(12, original),
                FixtureTurn::toward(Quat::IDENTITY, Quat::IDENTITY, 1.0),
                FixtureTimelineSeq::new(vec![AnimationNodeIndex::new(0)], vec![0.0], usize::MAX),
            ))
            .id();

        cleanup_talk_effects(&mut world, fixture, 12);

        assert_eq!(world.get::<Transform>(fixture).unwrap().rotation, original);
        assert!(world.get::<FixtureTalkEffects>(fixture).is_none());
        assert!(world.get::<FixtureTurn>(fixture).is_none());
        assert!(world.get::<FixtureTimelineSeq>(fixture).is_none());
    }
}
