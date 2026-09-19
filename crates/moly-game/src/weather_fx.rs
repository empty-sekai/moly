//! 天气粒子链：现象档案的 `fx/effects.json` → 天空/相机/站点三锚 →
//! UberUnlit 粒子绘制路径（与站点链共用同一份材质与管线特化）→ 粒子律
//! 仿真。**几何半在 `billboard`、着色半在 `uber_particle`，本模块只做
//! 装载、判读、锚定与推进。**
//!
//! # 装载形
//!
//! 现象切换（天气链写 `CurrentPhenomenon`）与换站（站点链换
//! `SiteActive`）共同决定一个锚点 `(档位, 站点)`：锚点变了就拆链重装。
//! 档位清单按 `phenomena/<档位>/fx/file` 取该档的 effects.json，然后按
//! 三类锚挑选 effect：
//!
//! - **sky / camera**：先取 `unique__<站点>` 变体，没有再回退 `global`
//!   （语料里每个 (档, 类) 至多一个匹配，挑选与对象序无关）。
//! - **site**：`unique__<站点>` 变体**全部**装上（站点专属效果没有
//!   全局回退）。
//!
//! 站点名是主表行的 `assetbundleName`（`SiteActive::env_site`）。
//!
//! # 锚定语义
//!
//! - **sky**：随玩家平移（穹顶跟人走），不随任何旋转。
//! - **camera**：随相机平移；旋转仅当 `effectiveRotation == "normal"`
//!   继承（`"fix"` 档源里每帧用父级旋转的逆抵消自己 ⇒ 世界恒等）。
//! - **site**：恒等——本仓站点系是站点局部系（`SiteRoot` 恒等生成、
//!   主表位点无人消费），effect 的节点链在语料里也全为恒等。
//!
//! 节点链 TRS（根记录 → 发射节点）在判读时合成一次；局部空间仿真逐帧
//! `锚 ∘ 链` 换算，世界空间仿真在**出生时**过一次全变换（位置过全变换、
//! 方向过旋转后归一、尺寸吃链缩放）。
//!
//! # 判读门与盘点
//!
//! 逐条具名拒绝（分母是选中 effect 的全部 `particles[]` 条目）：材质族、
//! 绘制模式、对齐档、发射率形状、发射形状、仿真空间、起始三轴旋转、
//! 状态档、关键字、律解析、渲染器字段。语料里整类够不着的（Mesh 绘制、
//! 非 View 对齐、Donut/ConeVolume 形状、带权曲线）按档计数，不静默。
//! 律对单条条目解析（单条档案包裹，一条坏不拖垮整包——`Effects` 的
//! 入口对档案级失败成立，包裹后失败按条计）。
//!
//! 语料事实（放行 78 条上现算）：`limitVelocity` 29 条（system 块的键名
//! 就是 `limitVelocity`，不是引擎的 `LimitVelocityOverLifetime`——律的
//! 映射表与之一致），全部能过律构造；`subEmitters` 56 条声明但 `emitter`
//! 全为 null（源数据里就没接线，没有可发射的子体，不构成缺口）；
//! `sortMode` 全 0、`renderQueue` 全 3000（系统间无序差）；`randomSeed`
//! 全 0 且 `autoRandomSeed` 全 true（引擎运行时自造种子，不可复算——
//! 固定种子流是本仓的表示选择）。
//!
//! # 抽签纪律
//!
//! 出生抽签次数是式的组成部分（固定种子下可复算）：形状抽样按律表
//! （圆 1 抽、锥 2 抽、球/半球 2 抽、单边棱 1 抽），随后出生取值表逐项
//! 各抽一次（速度与重力共用稳定因子不抽），种子 u32 最后一抽。**取值
//! 顺序是本仓的表示选择**（源引擎的流分配不可见），仓内各链顺序不一，
//! 次数与条件规则按引擎侧转录（表情链的出生取值表）。
//!
//! # 仓内已记录的分歧
//!
//! - **圆盘 1 抽 vs 2 抽**：律表（`StartCircle<Random>` 反汇编）是 1 抽
//!   （径向由 `radiusThickness` 单参数决定）；雨/站点/表情三条链都抽 2。
//!   本链按**律表**。
//! - **重力次序**：本链按表情链的引擎次序（模块批 → 推进 → **重力**）；
//!   雨/站点链是重力在积分前（各自注释具名「未核」）。
//! - **环形模式 2 回卷**：本链调律的 `set_remaining` 落回卷值（语料里
//!   14 条模式 2 发射器，回卷区间全为 [0,1] 即整段循环）；站点链对
//!   `Looped` 裁决不落值（站点语料无模式 2）。

use bevy::asset::{AssetPath, LoadState, RecursiveDependencyLoadState};
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use moly_assets::particle_geometry::ParticleMeshReference;
use std::sync::Arc;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::{Effects, EmissionState, EmitterParams, LimitVelocity, MinMaxCurve, RotationOverLifetime};
use serde_json::Value;
use std::collections::HashMap;
use moly_assets::weather_effect::WeatherEffectLifecycle;

use crate::billboard::{self, Alignment, SizeClamp};
use crate::character::AvatarRoot;
use crate::site::SiteActive;
use crate::uber_particle::{BlendArm, CullArm, UberParticleMaterial, UberT1Params};
use crate::weather_transition::{EnvironmentSelection, GlobalEffectIdentity, WeatherTransition, WeatherFxPrepared};
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, compose_to_world, simulate};

/// 源族的 shader 名（与站点链同一族、同一份材质实现）。
const SHADER_NAME: &str = "Mysekai/Effect/UberUnlit";

/// 原版粒子族的 shader 名。天气档案里两族并存——现算全 15 份现象档案：
/// 该族 144 条、UberUnlit 380 条、无材质/无 shader 名 36 条。
const PLAIN_SHADER_NAME: &str = "Particles/Standard Unlit";

/// 粒子材质族。两族的属性名与特性面**不相交**，但化简后的片元链同形：
/// 原版族的程序体（该族全部变体逐份读过）恰三步——
///   `c = texture(_MainTex, uv)` → `c *= _Color` → `c *= 顶点色`
/// 即本链所有特性臂关闭、材质色改为平乘。所以两族共用一条绘制路径，
/// 由这个枚举决定按哪套属性名取值、以及着色器走哪条臂。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// 染色 / 亮度键控透明 / 现象光 / 逐粒子流都在这一族。
    Uber,
    /// 原版族：特性面由它自己的一组开关表达，全 0 才等于那条三步链。
    Plain,
}

/// 零缩放的四边形没有面积，画不出来；尺寸下限。
const MIN_PARTICLE_SIZE: f32 = 0.0001;

/// 出生抽签的确定性随机种子。与站点链**不同流**：两条链同时在跑，
/// 同流会在两族上画出同一图形的错觉（逐系统再乘质数散列）。
const RNG_SEED: u64 = 0x7765_6174_0001_0125;

/// 全局 mip 偏置：与站点链同口径（无动态分辨率 ⇒ 0）。
const GLOBAL_MIP_BIAS: f32 = 0.0;

/// prewarm 快进步长：与运行帧率同一量级（雨链同值）。prewarm 的档位
/// 语义是「开播前把一个周期快进完」，不是精确复算。
const PREWARM_STEP: f32 = 1.0 / 60.0;

/// T1 关键字全集之外的「放行但未实现」项。
const SOFT_PARTICLES: &str = "_SOFT_PARTICLES_ENABLED";

// ---- 资源 ----

/// 当前装载锚点：档位名 + 站点名。与 `(CurrentPhenomenon, SiteActive)`
/// 现值不一致时拆链重装。
#[derive(Resource, PartialEq)]
pub(crate) struct WeatherFxAnchor {
    request_serial: u64,
    tier: String,
    env_site: String,
}

/// 档位清单请求（`phenomena/index.json`）。
#[derive(Resource)]
pub(crate) struct WeatherFxRequest {
    request_serial: u64,
    index: Handle<JsonAsset>,
    tier: String,
    env_site: String,
}

/// 该档 effects.json 的请求与锚点账目。
#[derive(Resource)]
pub(crate) struct WeatherFxDoc {
    request_serial: u64,
    handle: Handle<JsonAsset>,
    tier: String,
    env_site: String,
}

/// 绘制实体标记：换档/换站时按它撤（这些实体不挂在任何场景树下）。
#[derive(Component)]
pub struct WeatherFxDraw;

enum PlannedGeometry {
    Billboard { alignment: Alignment, clamp: SizeClamp, pivot: [f32; 3] },
    Mesh {
        reference: ParticleMeshReference,
        glb: Handle<Gltf>,
        alignment: crate::particle_geometry::Alignment,
        source: Option<Arc<crate::particle_geometry::SourceMesh>>,
        pivot: Vec3,
    },
}
impl PlannedGeometry {
    fn into_runtime(self) -> crate::particle_runtime::Geometry {
        match self {
            Self::Billboard { alignment, clamp, pivot } => crate::particle_runtime::Geometry::Billboard { alignment, clamp, pivot },
            Self::Mesh { alignment, source, pivot, .. } => crate::particle_runtime::Geometry::Mesh(crate::particle_geometry::MeshDraw {
                source: source.expect("source mesh readiness must precede weather commit"), alignment, pivot,
            }),
        }
    }
}

/// 一条放行的粒子系统：律侧参数 + 锚定账目 + 呈现侧输入 + 待装载贴图。
struct Planned {
    ordinal: usize,
    node: String,
    effect: String,
    emitter: EmitterParams,
    kind: EffectKind,
    /// camera 档 `effectiveRotation == "normal"`（继承相机旋转）；
    /// `"fix"` 与缺省都不继承。
    camera_rotation: bool,
    /// 根记录 → 发射节点链的 TRS 合成（语料根记录全档恒等，仍参与合成）。
    /// 链缩放由出生步从它折算（X 分量；语料全部均匀）。
    node_affine: GlobalTransform,
    params: UberT1Params,
    tint_area: bool,
    /// 原版粒子族 ⇒ 材质色平乘（见 wgsl 的 `UBER_PLAIN_COLOUR`）。
    plain_colour: bool,
    cull: CullArm,
    blend: BlendArm,
    geometry: PlannedGeometry,
    texture: Handle<Image>,
    effect_pass: Option<crate::uber_particle::ParticleEmission>,
    lifecycle: WeatherEffectLifecycle,
    /// Cone 的半顶角（shape 块的 `angle` 键；律的 `ShapeParams` 不带它）。
    cone_angle: Option<f32>,
    rol: Option<RotationOverLifetime>,
    limit: Option<LimitVelocity>,
}

/// 判读结果：放行的计划 + 逐档拒绝盘点。
#[derive(Resource)]
pub(crate) struct WeatherFxPlan {
    request_serial: u64,
    selection: EnvironmentSelection,
    site_started_at: Option<f64>,
    site_installed: bool,
    global_installed: bool,
    planned: Vec<Planned>,
    tally: Tally,
    tier: String,
    env_site: String,
}

/// 逐档盘点。**每一格都是「这一档有多少条被挡在外面」**——盘面上看得见
/// 还差什么，是这条通路唯一诚实的进度量。
#[derive(Default, Debug)]
struct Tally {
    /// 选中条目里这一族的记录数（过 shader 门之后计）。
    records: usize,
    no_renderer: usize,
    no_material: usize,
    other_shader: Vec<String>,
    renderer_disabled: usize,
    render_mode: Vec<String>,
    alignment: Vec<String>,
    no_system_block: usize,
    no_emission: usize,
    /// 只按距离发射（本链不跟发射器位移）。
    rate_distance_only: usize,
    /// 率恒 0 且无 burst：永不发射。
    dead_emission: usize,
    no_shape: usize,
    shape: Vec<String>,
    sim_space: usize,
    /// 起始三轴旋转的非零 X/Y 常量（公告板只画 Z 自旋）。
    start_rotation_3d: usize,
    state_arm: Vec<String>,
    keyword: Vec<String>,
    /// 放行但少一步（软粒子、深度偏置）——逐条具名，不静默。
    shading_shortfall: Vec<String>,
    no_base_map: usize,
    node_unresolved: usize,
    node_inactive: usize,
    law_reject: Vec<String>,
    rol_refused: Vec<String>,
    limit_refused: Vec<String>,
    clamp_missing: usize,
    cone_no_angle: usize,
    start_delay: usize,
    effect_pass_unresolved: usize,
    effect_pass_not_declared: usize,
    effect_pass_queue_excluded: usize,
    admitted: usize,
}

/// 全部在跑的系统。
#[derive(Resource)]
pub(crate) struct WeatherFxState {
    selection: Option<EnvironmentSelection>,
    global_identity: Option<GlobalEffectIdentity>,
    sky_stopped: bool,
    live: Vec<LiveWeatherEmitter>,
    tier: String,
    env_site: String,
    admitted: usize,
    records: usize,
}

struct LiveWeatherEmitter {
    runtime: Runtime,
    draw: Entity,
    lifecycle: WeatherEffectLifecycle,
}
impl std::ops::Deref for LiveWeatherEmitter {
    type Target = Runtime;
    fn deref(&self) -> &Runtime { &self.runtime }
}
impl std::ops::DerefMut for LiveWeatherEmitter {
    fn deref_mut(&mut self) -> &mut Runtime { &mut self.runtime }
}

struct RetiringEmitter {
    emitter: LiveWeatherEmitter,
    destroy_at: f64,
}

#[derive(Resource, Default)]
pub(crate) struct WeatherFxRetirements {
    live: Vec<RetiringEmitter>,
}

impl WeatherFxRetirements {
    fn stop(&mut self, active: &mut WeatherFxState, now: f64) {
        self.stop_matching(active, now, |_| true);
    }
    fn stop_matching(&mut self, active: &mut WeatherFxState, now: f64, predicate: impl Fn(&LiveWeatherEmitter)->bool) {
        let mut kept = Vec::new();
        for emitter in std::mem::take(&mut active.live) {
            if predicate(&emitter) {
                self.live.push(RetiringEmitter {destroy_at: now + emitter.lifecycle.time_until_destroy(), emitter});
            } else { kept.push(emitter); }
        }
        active.live = kept;
        active.admitted = active.live.len();
    }
}

/// Independent of camera availability, simulationSpeed, particle age and sky fade.
pub(crate) fn expire_retirements(
    mut commands: Commands,
    mut retiring: ResMut<WeatherFxRetirements>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    retiring.live.retain(|entry| {
        if now < entry.destroy_at { return true; }
        commands.entity(entry.emitter.draw).try_despawn();
        false
    });
}

/// Stage a new request. Keep old effects running until the replacement is ready.
/// Source RefreshGlobalEffect receives loaded data, stops the old instance, then
/// emits the new instance; a request itself is not a Stop instruction.
pub(crate) fn watch(
    mut commands: Commands,
    server: Res<AssetServer>,
    phase: Option<Res<WeatherTransition>>,
    site: Option<Res<SiteActive>>,
    anchor: Option<Res<WeatherFxAnchor>>,
    active: Option<Res<WeatherFxState>>,
) {
    let (Some(phase),Some(site))=(phase,site) else {return;};
    let Some(destination)=phase.destination.as_ref() else {return;};
    let tier=destination.name.clone();let env_site=site.env_site.clone();
    if anchor.as_ref().is_some_and(|a|a.tier==tier&&a.env_site==env_site&&a.request_serial==phase.request_serial){return;}
    commands.remove_resource::<WeatherFxPlan>();
    commands.remove_resource::<WeatherFxPrepared>();
    commands.remove_resource::<WeatherFxDoc>();
    commands.remove_resource::<WeatherFxRequest>();
    commands.insert_resource(WeatherFxAnchor{tier:tier.clone(),env_site:env_site.clone(),request_serial:phase.request_serial});
    // A -> pending B -> A cancels the load without stopping the active A.
    if active.as_ref().is_some_and(|a|a.selection.as_ref()==Some(destination)&&!a.sky_stopped){
        commands.insert_resource(WeatherFxPrepared{selection:destination.clone(),request_serial:phase.request_serial});
        commands.insert_resource(crate::weather_transition::WeatherGlobalFxCommitted(phase.request_serial));
        return;
    }
    commands.insert_resource(WeatherFxRequest{
        request_serial:phase.request_serial,
        index:server.load::<JsonAsset>(AssetPath::from("moly://phenomena/index.json".to_owned())),
        tier,env_site,
    });
}

/// Update：清单到达 → 取该档的 effects.json 路径并请求。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    request: Option<Res<WeatherFxRequest>>,
) {
    let Some(request) = request else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&request.index) {
        panic!("现象清单装载失败（天气粒子链）：{err:?}");
    }
    let Some(doc) = json.get(&request.index) else {
        return;
    };
    let value: Value = serde_json::from_str(&doc.0)
        .unwrap_or_else(|err| panic!("现象清单不是合法 JSON（天气粒子链）：{err}"));
    let file = value
        .pointer(&format!("/phenomena/{}/fx/file", request.tier))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("现象 {} 缺 fx.file", request.tier))
        .to_owned();
    let handle =
        server.load::<JsonAsset>(AssetPath::from(format!("moly://phenomena/{file}")));
    commands.insert_resource(WeatherFxDoc {
        request_serial:request.request_serial,
        handle,
        tier: request.tier.clone(),
        env_site: request.env_site.clone(),
    });
    commands.remove_resource::<WeatherFxRequest>();
}

/// Update：该档 effects.json 到达 → 选 effect、逐条判读。
pub(crate) fn plan(
    mut commands: Commands,
    phase: Res<WeatherTransition>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    doc: Option<Res<WeatherFxDoc>>,
    planned: Option<Res<WeatherFxPlan>>,
 ) {
    if planned.is_some() {
        return;
    }
    let Some(doc) = doc else {
        return;
    };
    if doc.request_serial != phase.request_serial { return; }
    if let LoadState::Failed(err) = server.load_state(&doc.handle) {
        panic!("现象 {} 的特效档案装载失败（天气粒子链）：{err:?}", doc.tier);
    }
    let Some(asset) = json.get(&doc.handle) else {
        return;
    };
    let value: Value = serde_json::from_str(&asset.0).unwrap_or_else(|err| {
        panic!("现象 {} 的特效档案不是合法 JSON（天气粒子链）：{err}", doc.tier)
    });
    let Some(effects) = value.get("effects").and_then(Value::as_object) else {
        panic!("现象 {} 的特效档案缺 effects 对象", doc.tier);
    };

    // ---- 选 effect：sky/camera 各一条（专属优先、全局回退），site 全装 ----
    // 语料里每个 (档, 类) 至多一个匹配，挑选与对象序无关。
    let unique = format!("unique__{}", doc.env_site);
    let mut selected: Vec<(String, &Value)> = Vec::new();
    for kind in ["sky", "camera"] {
        let mut pick = None;
        for (name, effect) in effects {
            if effect_kind_of(effect) == Some(kind)
                && effect_variant_of(effect) == Some(unique.as_str())
            {
                pick = Some((name.clone(), effect));
                break;
            }
        }
        if pick.is_none() {
            for (name, effect) in effects {
                if effect_kind_of(effect) == Some(kind)
                    && effect_variant_of(effect) == Some("global")
                {
                    pick = Some((name.clone(), effect));
                    break;
                }
            }
        }
        if let Some(pick) = pick {
            selected.push(pick);
        }
    }
    for (name, effect) in effects {
        if effect_kind_of(effect) == Some("site")
            && effect_variant_of(effect) == Some(unique.as_str())
        {
            selected.push((name.clone(), effect));
        }
    }

    let mut tally = Tally::default();
    let mut plans = Vec::new();
    for (effect_name, effect) in &selected {
        let lifecycle = WeatherEffectLifecycle::from_effect(effect)
            .unwrap_or_else(|err| panic!("weather effect {effect_name}: {err}"));
        let kind = match effect_kind_of(effect) {
            Some("sky") => EffectKind::Sky,
            Some("camera") => EffectKind::Camera,
            Some("site") => EffectKind::Site,
            _ => continue,
        };
        let camera_rotation = effect
            .get("effectiveRotation")
            .and_then(Value::as_str)
            == Some("normal");
        let mut by_path: HashMap<String, &Value> = HashMap::new();
        if let Some(nodes) = effect.get("nodes").and_then(Value::as_array) {
            for node in nodes {
                if let Some(path) = node.get("path").and_then(Value::as_str) {
                    by_path.insert(path.to_owned(), node);
                }
            }
        }
        let Some(particles) = effect.get("particles").and_then(Value::as_array) else {
            continue;
        };
        for particle in particles {
            match judge(
                effect_name,
                particle,
                &by_path,
                kind,
                camera_rotation,
                lifecycle,
                &server,
                &mut tally,
            ) {
                Some(planned) => {
                    tally.admitted += 1;
                    plans.push(planned);
                }
                None => {}
            }
        }
    }
    info!(
        "[weather-fx] {} @ {} 判读：选中 effect {} 个；本族记录 {}；放行 {}；\
         挡下——无渲染器 {} · 无材质 {} · 非本族 {:?} · 渲染器关 {} · \
         绘制模式 {:?} · 对齐档 {:?} · 缺 system 块 {} · 缺 emission {} · \
         只按距离发射 {} · 死发射 {} · 缺形状 {} · 形状律缺 {:?} · \
         仿真空间 {} · 起始三轴旋转 {} · 状态档 {:?} · 关键字 {:?} · \
         缺基础贴图 {} · 节点未解析 {} · 节点链关 {} · 律拒 {:?} · \
         自旋律拒 {:?} · 限速律拒 {:?} · 钳制字段缺 {} · 锥缺角 {} · \
         起始延迟 {}；\
         放行但未实现（逐条具名，不静默）：{:?}",
        doc.tier,
        doc.env_site,
        selected.len(),
        tally.records,
        tally.admitted,
        tally.no_renderer,
        tally.no_material,
        count_names(&tally.other_shader),
        tally.renderer_disabled,
        count_names(&tally.render_mode),
        count_names(&tally.alignment),
        tally.no_system_block,
        tally.no_emission,
        tally.rate_distance_only,
        tally.dead_emission,
        tally.no_shape,
        count_names(&tally.shape),
        tally.sim_space,
        tally.start_rotation_3d,
        count_names(&tally.state_arm),
        count_names(&tally.keyword),
        tally.no_base_map,
        tally.node_unresolved,
        tally.node_inactive,
        count_names(&tally.law_reject),
        count_names(&tally.rol_refused),
        count_names(&tally.limit_refused),
        tally.clamp_missing,
        tally.cone_no_angle,
        tally.start_delay,
        count_names(&tally.shading_shortfall),
    );
    let Some(selection) = phase.destination.as_ref().filter(|selection| selection.name == doc.tier && selection.environment_site == doc.env_site) else { return; };
    for (ordinal, planned) in plans.iter_mut().enumerate() { planned.ordinal = ordinal; }
    commands.insert_resource(WeatherFxPlan {
        selection: selection.clone(), request_serial:doc.request_serial, site_started_at:None, site_installed:false, global_installed:false,
        planned: plans,
        tally,
        tier: doc.tier.clone(),
        env_site: doc.env_site.clone(),
    });
    commands.remove_resource::<WeatherFxDoc>();
}

/// effect 档案的 `kind`（字符串原样）。
/// Admission is based on authored controls, not an effect/phenomenon name.
/// The source keeps irrelevant sampling channels (e.g. donut radius mode) in
/// serialized data. Only the mode the actual Start* dispatcher owns is active.
fn source_shape_admission(shape: &Value) -> Option<String> {
    let kind = shape.get("type").and_then(Value::as_str).unwrap_or("");
    if shape.get("sourceVersion").and_then(Value::as_u64) != Some(1) {
        return Some("missing source shape controls; re-extract".into());
    }
    let scale = shape.get("scale").and_then(Value::as_array);
    if !scale.is_some_and(|v| v.len() == 3 && v.iter().all(|x| x.as_f64().is_some_and(f64::is_finite))) {
        return Some("missing/invalid authored shape scale".into());
    }
    let mode = if kind == "SingleSidedEdge" { "radiusMode" } else { "arcMode" };
    if shape.get(mode).and_then(Value::as_str) != Some("Random") {
        return Some(format!("{mode} {:?} consumer pending", shape.get(mode)));
    }
    if shape.get("alignToDirection").and_then(Value::as_bool) != Some(false) {
        return Some("source aligned start rotation consumer pending".into());
    }
    for key in ["randomDirectionAmount", "sphericalDirectionAmount", "randomPositionAmount"] {
        if shape.get(key).and_then(Value::as_f64) != Some(0.0) {
            return Some(format!("source {key} consumer pending"));
        }
    }
    if matches!(kind, "Cone" | "ConeVolume") {
        for key in if kind == "ConeVolume" { &["angle", "length"][..] } else { &["angle"][..] } {
            if !shape.get(*key).and_then(Value::as_f64).is_some_and(|v| v.is_finite() && v >= 0.0) {
                return Some(format!("missing/invalid authored cone {key}"));
            }
        }
    }
    if kind == "Donut" && !shape.get("donutRadius").and_then(Value::as_f64).is_some_and(|v|v.is_finite() && v>=0.0) {
        return Some("missing/invalid authored torus radius".into());
    }
    None
}

fn effect_kind_of(effect: &Value) -> Option<&str> {
    effect.get("kind").and_then(Value::as_str)
}

/// effect 档案的 `variant`。
fn effect_variant_of(effect: &Value) -> Option<&str> {
    effect.get("variant").and_then(Value::as_str)
}

/// 把一串具名条目折成 `(名字, 条数)` 表——盘点行印它。
fn count_names(items: &[String]) -> Vec<(String, usize)> {
    let mut map: HashMap<&str, usize> = HashMap::new();
    for item in items {
        *map.entry(item.as_str()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = map
        .into_iter()
        .map(|(name, n)| (name.to_owned(), n))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// 逐条判读。放行回 Some，挡下回 None 并在盘点里具名。
fn judge(
    effect_name: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    kind: EffectKind,
    camera_rotation: bool,
    lifecycle: WeatherEffectLifecycle,
    server: &AssetServer,
    tally: &mut Tally,
) -> Option<Planned> {
    let renderer = match particle.get("renderer").filter(|v| v.is_object()) {
        Some(renderer) => renderer,
        None => {
            tally.no_renderer += 1;
            return None;
        }
    };
    let material = renderer.get("material").filter(|v| v.is_object());
    let material = match material {
        Some(material) => material,
        None => {
            tally.no_material += 1;
            return None;
        }
    };
    let shader = material.get("shader").and_then(Value::as_str);
    let family = match shader {
        Some(SHADER_NAME) => Family::Uber,
        Some(PLAIN_SHADER_NAME) => Family::Plain,
        other => {
            tally
                .other_shader
                .push(other.unwrap_or("(缺 shader 名)").to_owned());
            return None;
        }
    };
    tally.records += 1;
    if renderer.get("enabled").and_then(Value::as_bool) != Some(true) {
        tally.renderer_disabled += 1;
        return None;
    }
    let render_mode = renderer.get("renderMode").and_then(Value::as_str).unwrap_or("");
    if !matches!(render_mode, "Billboard" | "Mesh") {
        tally.render_mode.push(format!("unsupported source render mode {render_mode}"));
        return None;
    }
    let alignment_id = renderer.get("alignment").and_then(Value::as_i64).unwrap_or(-1);
    let mesh_alignment = crate::particle_geometry::Alignment::from_source(alignment_id);
    let alignment = Alignment::from_render_space(alignment_id);
    if mesh_alignment.is_none() || (render_mode == "Billboard" && alignment.is_none()) {
        tally.alignment.push(Alignment::render_space_name(alignment_id).to_owned());
        return None;
    }
    let mesh_reference = if render_mode == "Mesh" {
        let references = renderer.get("meshes").and_then(Value::as_array);
        let Some(references) = references.filter(|r| r.len() == 1) else {
            tally.render_mode.push("source Mesh requires one fully resolved mesh slot; multi-mesh selection not yet consumed".into());
            return None;
        };
        let reference = match serde_json::from_value::<ParticleMeshReference>(references[0].clone()) {
            Ok(value) if value.validate().is_ok() => value,
            value => { tally.render_mode.push(format!("invalid source Mesh reference: {value:?}")); return None; },
        };
        if reference.mesh_slot != 0 || renderer.get("flip").and_then(Value::as_array)
            .is_none_or(|v| v.len()!=3 || v.iter().any(|x| x.as_f64()!=Some(0.0))) {
            tally.render_mode.push("unconsumed source Mesh slot selection or particle flip".into()); return None;
        }
        Some(reference)
    } else { None };

    let system = match particle.get("system").filter(|v| v.is_object()) {
        Some(system) => system,
        None => {
            tally.no_system_block += 1;
            return None;
        }
    };
    // 发射率形状：只按距离发射（本链不跟发射器位移）与死发射（率恒 0 且
    // 无 burst）都挡。
    let emission = match system.get("emission").filter(|v| v.as_object().is_some_and(|o| !o.is_empty())) {
        Some(emission) => emission,
        None => {
            tally.no_emission += 1;
            return None;
        }
    };
    let rate_time = raw_const(emission.get("rateOverTime"));
    let rate_distance = raw_const(emission.get("rateOverDistance"));
    let time_zero = rate_time.map_or(true, |v| v == 0.0);
    let distance_zero = rate_distance.map_or(true, |v| v == 0.0);
    if time_zero && !distance_zero {
        tally.rate_distance_only += 1;
        return None;
    }
    let bursts_present = emission
        .get("bursts")
        .and_then(Value::as_array)
        .is_some_and(|bursts| !bursts.is_empty());
    if time_zero && distance_zero && !bursts_present {
        tally.dead_emission += 1;
        return None;
    }
    let shape = match system.get("shape").filter(|v| v.as_object().is_some_and(|o| !o.is_empty())) {
        Some(shape) => shape,
        None => {
            tally.no_shape += 1;
            return None;
        }
    };
    let shape_type = shape.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        shape_type,
        "Circle" | "Cone" | "ConeVolume" | "Sphere" | "Hemisphere" | "SingleSidedEdge" | "Donut"
    ) {
        tally.shape.push(shape_type.to_owned());
        return None;
    }
    if let Some(reason) = source_shape_admission(shape) {
        tally.shape.push(format!("{shape_type}: {reason}"));
        return None;
    }
    if !matches!(
        system.get("simulationSpace").and_then(Value::as_str),
        Some("Local") | Some("World")
    ) {
        tally.sim_space += 1;
        return None;
    }
    // 起始三轴旋转：X/Y 常量非零的挡（公告板只画 Z 自旋）。
    let start = system.get("start").cloned().unwrap_or(Value::Null);
    if render_mode == "Billboard" && start.get("rotation3D").and_then(Value::as_bool) == Some(true) {
        let x = raw_const(start.get("rotationX"));
        let y = raw_const(start.get("rotationY"));
        if !x.map_or(true, |v| v == 0.0) || !y.map_or(true, |v| v == 0.0) {
            tally.start_rotation_3d += 1;
            return None;
        }
    }

    if start.get("rotation3D").and_then(Value::as_bool) == Some(true)
        && ["rotationX", "rotationY"].iter().any(|key| !start.get(*key).is_some_and(Value::is_object)) {
        tally.start_rotation_3d += 1;
        return None;
    }

    // ---- 状态档（材质 uniform 的分支全关才走 T1 片元链） ----
    let floats = material.get("floats").and_then(Value::as_object);
    let get = |key: &str| floats.and_then(|f| f.get(key)).and_then(Value::as_f64);
    // 混合因子：两族的属性名不同——染色族 `_BlendSrc`/`_BlendDst`，原版族
    // `_SrcBlend`/`_DstBlend`（后者由该 shader 的 pass 状态 `rtBlend0` 具名
    // 引用，读自真源而非推断）。取值口径同为 Unity `BlendMode`：5 = SrcAlpha。
    let (src_key, dst_key) = match family {
        Family::Uber => ("_BlendSrc", "_BlendDst"),
        Family::Plain => ("_SrcBlend", "_DstBlend"),
    };
    if get(src_key) != Some(5.0) {
        tally
            .state_arm
            .push(format!("{src_key}={:?}", get(src_key)));
        return None;
    }
    let blend = match get(dst_key) {
        Some(v) if v == 10.0 => BlendArm::AlphaBlend,
        Some(v) if v == 1.0 => BlendArm::Additive,
        other => {
            tally.state_arm.push(format!("{dst_key}={other:?}"));
            return None;
        }
    };
    // `_Cull` 两族同名同口径（原版族的 pass 状态把 culling 具名引到它）。
    let cull = match get("_Cull") {
        Some(v) if v == 0.0 => CullArm::Off,
        Some(v) if v == 1.0 => CullArm::Front,
        Some(v) if v == 2.0 => CullArm::Back,
        other => {
            tally.state_arm.push(format!("_Cull={other:?}"));
            return None;
        }
    };
    // 深度比较：染色族由材质属性给；原版族的 pass 状态把 zTest 写成常量
    // 4（LEqual）且**不开放为属性**，所以该族按 4 读——这是从 pass 状态读出
    // 来的值，不是「属性缺失就当默认」的倒推。
    let z_test = match family {
        Family::Uber => get("_ZTest"),
        Family::Plain => Some(4.0),
    };
    if z_test != Some(4.0) {
        tally.state_arm.push(format!("_ZTest={z_test:?}"));
        return None;
    }

    // 特性开关：两族各有自己的一组，含义相同——「全关」才等于本链实现的
    // 那条片元链。任何一个非 0 都说明源程序里编进了本链没有的步，具名挡下。
    let mut luminance_enabled = 0.0f32;
    let mut phenomena_enabled = 0.0f32;
    let mut tint_blend_rate_coord = 0.0f32;
    match family {
        Family::Uber => {
            for name in ["_FakeLightEnabled", "_BaseMapRotationEnabled"] {
                if get(name) != Some(0.0) {
                    tally.state_arm.push(format!("{name}={:?}", get(name)));
                    return None;
                }
            }
            // 亮度键控透明与现象光这两条分支本链已实现（片元链尾段，与站点粒子
            // 共用同一条）。开关只认 0/1，别的取值说明这份材质不是我们读过的那一档。
            luminance_enabled = match get("_TranceparencyByLuminanceEnabled") {
                Some(v) if v == 0.0 || v == 1.0 => v as f32,
                other => {
                    tally
                        .state_arm
                        .push(format!("_TranceparencyByLuminanceEnabled={other:?}"));
                    return None;
                }
            };
            phenomena_enabled = match get("_PhenomenaLightEnabled") {
                Some(v) if v == 0.0 || v == 1.0 => v as f32,
                other => {
                    tally
                        .state_arm
                        .push(format!("_PhenomenaLightEnabled={other:?}"));
                    return None;
                }
            };
            if luminance_enabled == 1.0 {
                for name in [
                    "_LuminanceTransparencyProgressCoord",
                    "_LuminanceTransparencySharpnessCoord",
                ] {
                    if get(name) != Some(0.0) {
                        tally.state_arm.push(format!("{name}={:?}", get(name)));
                        return None;
                    }
                }
            }
            // `_TintBlendRateCoord` 已接逐粒子流（顶点属性 custom1/custom2 + 着色器
            // 选择器），不再要求为 0；其余 coord 的选择器接口相同但消费面未接，
            // 仍旧拒——放行了却不喂就是静默的错误值。
            tint_blend_rate_coord = get("_TintBlendRateCoord").unwrap_or(0.0) as f32;
            for name in [
                "_EmissionIntensityCoord",
                "_BaseMapOffsetXCoord",
                "_BaseMapOffsetYCoord",
                "_BaseMapRotationCoord",
            ] {
                if get(name) != Some(0.0) {
                    tally.state_arm.push(format!("{name}={:?}", get(name)));
                    return None;
                }
            }
        }
        Family::Plain => {
            // 原版族的特性面。`_ColorMode` 非 0 会把「平乘」换成加/减/叠加/
            // 取色/差值里的另一支；其余六个各自把一整段接进片元链。这一族
            // 没有亮度键控与现象光那两条分支（属性面不含它们）⇒ 两臂恒关。
            for name in [
                "_ColorMode",
                "_LightingEnabled",
                "_EmissionEnabled",
                "_DistortionEnabled",
                "_FlipbookMode",
                "_SoftParticlesEnabled",
                "_CameraFadingEnabled",
            ] {
                if get(name) != Some(0.0) {
                    tally.state_arm.push(format!("{name}={:?}", get(name)));
                    return None;
                }
            }
        }
    }
    let luminance = Vec4::new(
        get("_LuminanceTransparencyProgress").unwrap_or(0.0) as f32,
        get("_LuminanceTransparencySharpness").unwrap_or(0.0) as f32,
        get("_InverseLuminanceTransparency").unwrap_or(0.0) as f32,
        0.0,
    );
    // 关键字：T1 全集之外的关键字意味着源程序里编进了本链没有的步。
    // 软粒子是「放行但未实现」（alpha 乘法链末段，要读场景深度）。
    let keywords = material
        .get("keywords")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<&str>>()
        })
        .unwrap_or_default();
    for keyword in &keywords {
        match *keyword {
            "_BASE_MAP_MODE_2D" | "_EMISSION_MAP_MODE_2D" | "_TINT_COLOR_ENABLED"
            | "_EMISSION_AREA_ALL" | "_TINT_AREA_ALL" => {}
            // 原版族的混合变体关键字。它与 `_SrcBlend=5`/`_DstBlend=10` 表达
            // 同一件事（该族语料 144/144 两者同时在场），混合因子已由上面的
            // 状态档接走，这里不额外改片元链。只对该族放行：出现在染色族上
            // 说明读到的不是我们读过的那一档。
            "_ALPHABLEND_ON" if family == Family::Plain => {}
            SOFT_PARTICLES => {
                tally
                    .shading_shortfall
                    .push("软粒子（关键字在场）".to_owned());
            }
            other => {
                tally.keyword.push(other.to_owned());
                return None;
            }
        }
    }
    // 深度偏置：源程序的顶点段在 |_ZOffset| > 0.004 时把裁剪空间 z 重映射
    // 一次，本链没有这一步——放行并具名计数。
    if get("_ZOffset").map(|v| v.abs() > 0.004).unwrap_or(false) {
        tally
            .shading_shortfall
            .push(format!("深度偏置 _ZOffset={:?}", get("_ZOffset")));
    }
    // 基础贴图：现象根相对 URI（字符串直存，不是站点侧车的槽下标）。
    // 两族的贴图槽名不同：染色族 `_BaseMap`，原版族 `_MainTex`。
    let base_map_key = match family {
        Family::Uber => "_BaseMap",
        Family::Plain => "_MainTex",
    };
    let uri = material
        .pointer(&format!("/textures/{base_map_key}"))
        .and_then(Value::as_str)
        .filter(|uri| !uri.is_empty());
    let Some(uri) = uri else {
        tally.no_base_map += 1;
        return None;
    };

    // ---- 律解析（单条档案包裹：一条坏只拒这一条，不拖垮整档） ----
    // 包裹键用字面量——律只把它当标签，effect 名留在 Planned 里。
    let node = particle.get("node").and_then(Value::as_str).unwrap_or("");
    let archive = serde_json::json!({
        "effects": {
            "weather": {
                "particles": [{ "node": node, "system": system.clone() }]
            }
        }
    });
    let archive_bytes = archive.to_string();
    let emitter = match Effects::from_json_str(archive_bytes.as_bytes()) {
        Ok(mut effects) if effects.emitters.len() == 1 => effects.emitters.remove(0),
        Ok(effects) => {
            // 一条进、一条出是构造就保证的；不符说明律的入口改了形状。
            panic!(
                "粒子律单条解析返回 {} 条（应恰 1 条，{}/{node}）",
                effects.emitters.len(),
                effect_name,
            );
        }
        Err(err) => {
            tally.law_reject.push(format!("{node}: {err}"));
            return None;
        }
    };
    // 起始延迟：语料全 0；非 0 的发射次序未实现，具名挡下。
    if const_of(&emitter.start_delay).map_or(false, |v| v != 0.0) {
        tally.start_delay += 1;
        return None;
    }
    // ---- 模块律构造（表情链的条目级拒绝形：构造拒绝就摘模块，发射器照跑） ----
    let rol = emitter.rotation_over_lifetime.as_ref().and_then(|params| {
        match RotationOverLifetime::from_parts(
            params.separate_axes,
            params.x.as_ref(),
            params.y.as_ref(),
            &params.curve,
        ) {
            Ok(law) => Some(law),
            Err(reason) => {
                tally
                    .rol_refused
                    .push(format!("{node}: {reason}（自旋冻结）"));
                None
            }
        }
    });
    // 限速：语料里 29 条放行条目带 limitVelocity（全部 separateAxis=false、
    // 幅值恒定、drag=0、dampen∈{0.02, 0.1}、无乘法标志键），律构造全部
    // 接受——这一支是活的，不是占位。
    let limit = emitter.limit_velocity.as_ref().and_then(|params| {
        match LimitVelocity::from_parts(
            params.separate_axis,
            &params.magnitude,
            params.dampen,
            params.drag.as_ref(),
            params.multiply_drag_by_size,
            params.multiply_drag_by_velocity,
        ) {
            Ok(law) => Some(law),
            Err(reason) => {
                tally
                    .limit_refused
                    .push(format!("{node}: {reason}（不钳速）"));
                None
            }
        }
    });

    // ---- 渲染器字段：钳制上限与轴心是本链消费的两个，缺了没有可用的
    // 呈现输入（语料 78/78 都带；`minParticleSize` 全 0 且律侧下限用
    // 常量，不消费）。----
    let max_particle_size = renderer.get("maxParticleSize").and_then(Value::as_f64);
    let pivot = renderer
        .get("pivot")
        .and_then(Value::as_array)
        .filter(|list| list.len() == 3)
        .and_then(|list| {
            let mut out = [0.0f32; 3];
            for (slot, value) in out.iter_mut().zip(list) {
                *slot = value.as_f64()? as f32;
            }
            Some(out)
        });
    let (Some(max_particle_size), Some(pivot)) = (max_particle_size, pivot) else {
        tally.clamp_missing += 1;
        return None;
    };
    // The typed contract owns authored cone parameters for both cone modes.
    let cone_angle = if matches!(shape_type, "Cone" | "ConeVolume") {
        match emitter.shape.as_ref().and_then(|s| s.controls.angle) {
            Some(angle) => Some(angle),
            None => {
                tally.cone_no_angle += 1;
                return None;
            }
        }
    } else {
        None
    };

    // ---- 节点链：路径解析、激活走查、TRS 合成 ----
    if !by_path.contains_key(node) {
        tally.node_unresolved += 1;
        return None;
    }
    if !active_in_hierarchy(by_path, node) {
        tally.node_inactive += 1;
        return None;
    }
    let Some(node_affine) = compose_affine(by_path, node) else {
        tally.node_unresolved += 1;
        return None;
    };

    // ---- uniform 组装（与站点链同一份 T1 片元链） ----
    let tint_area = keywords
        .iter()
        .any(|k| *k == "_TINT_AREA_ALL" || *k == "_TINT_AREA_RIM");
    let base_st = material
        .pointer(&format!("/textureScaleOffset/{base_map_key}"))
        .and_then(Value::as_array)
        .filter(|list| list.len() == 4)
        .map(|list| {
            let mut out = [1.0f32; 4];
            for (slot, value) in out.iter_mut().zip(list) {
                if let Some(v) = value.as_f64() {
                    *slot = v as f32;
                }
            }
            out
        })
        .unwrap_or([1.0, 1.0, 0.0, 0.0]);
    // 材质色。染色族读 `_TintColor` 并按混合率朝它插值；原版族读 `_Color`
    // 并**无条件平乘**。两者共用这一槽，由 `plain_colour` 决定着色器按哪
    // 种语义读它（见 wgsl 的 `UBER_PLAIN_COLOUR`）。
    // ⚠ 该族语料里 `_Color` 与 `_MainTex_ST` 恰好都是单位元（144/144 分别
    // 为 (1,1,1,1) 与 (1,1,0,0)）⇒ **这份语料区分不出这两项有没有真的接上**。
    // 照真源接线，但不能拿它当已验证。
    let colour_key = match family {
        Family::Uber => "/colors/_TintColor",
        Family::Plain => "/colors/_Color",
    };
    let tint_colour = material
        .pointer(colour_key)
        .and_then(Value::as_array)
        .filter(|list| list.len() == 4)
        .map(|list| {
            let mut out = [1.0f32; 4];
            for (slot, value) in out.iter_mut().zip(list) {
                if let Some(v) = value.as_f64() {
                    *slot = v as f32;
                }
            }
            out
        })
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let tint_blend_rate = get("_TintBlendRate").unwrap_or(0.0) as f32;
    let texture =
        server.load::<Image>(AssetPath::from(format!("moly://phenomena/{uri}")));

    use moly_assets::material_passes::EffectPassEligibility;
    let effect_eligibility = EffectPassEligibility::from_material(material);
    match effect_eligibility {
        EffectPassEligibility::Unresolved => { tally.effect_pass_unresolved += 1; }
        EffectPassEligibility::NotDeclared => { tally.effect_pass_not_declared += 1; }
        EffectPassEligibility::QueueExcluded => { tally.effect_pass_queue_excluded += 1; }
        EffectPassEligibility::Eligible => {}
    }
    let soft_enabled = keywords.contains(&"_SOFT_PARTICLES_ENABLED");
    let soft_intensity = if soft_enabled {
        get("_SoftParticlesIntensity").filter(|v| v.is_finite())
            .expect("active source soft particles require exported intensity") as f32
    } else { 0.0 };
    let shader_coords = Vec4::new(tint_blend_rate_coord, soft_intensity,
        f32::from(soft_enabled), get("_EmissionIntensityCoord").unwrap_or(0.0) as f32);
    Some(Planned {
        ordinal: 0,
        node: node.to_owned(),
        effect: effect_name.to_owned(),
        lifecycle,
        emitter,
        kind,
        camera_rotation,
        node_affine,
        params: UberT1Params {
            base_st: Vec4::from_array(base_st),
            tint_colour: Vec4::from_array(tint_colour),
            scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, luminance_enabled, phenomena_enabled),
            luminance,
            coords: shader_coords,
        },
        tint_area,
        plain_colour: family == Family::Plain,
        cull,
        blend,
        geometry: if let Some(reference) = mesh_reference {
            let glb = server.load(AssetPath::from_path_buf(std::path::PathBuf::from(format!("phenomena/{}", reference.file))).with_source("moly"));
            PlannedGeometry::Mesh { reference, glb, alignment: mesh_alignment.expect("validated Mesh alignment"), source: None, pivot: Vec3::from_array(pivot) }
        } else {
            PlannedGeometry::Billboard { alignment: alignment.expect("validated Billboard alignment"),
                clamp: SizeClamp { max_screen_fraction: max_particle_size as f32, min_size: MIN_PARTICLE_SIZE }, pivot }
        },
        texture,
        effect_pass: (effect_eligibility == EffectPassEligibility::Eligible).then(|| crate::uber_particle::ParticleEmission {
            source_state: moly_assets::material_passes::SourceMaterialPasses::from_extras(material).and_then(|p| p.effect_state()),
            render_queue: material.get("renderQueue").and_then(Value::as_i64).expect("eligible effect has a queue") as i32,
            params: UberT1Params { base_st: Vec4::from_array(base_st), tint_colour: Vec4::from_array(tint_colour), scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, luminance_enabled, phenomena_enabled), luminance, coords: shader_coords },
            colour: Vec4::from_array(std::array::from_fn(|i| material.pointer("/colors/_EmissionColor").and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(1.0) as f32)),
            intensity: get("_EmissionIntensity").unwrap_or(1.0) as f32,
            colour_type: material.pointer("/ints/_EmissionColorType").and_then(Value::as_f64).or_else(|| get("_EmissionColorType")).unwrap_or(0.0) as f32,
            area: keywords.iter().any(|k| *k == "_EMISSION_AREA_ALL"),
            tint_area, plain_colour: family == Family::Plain, cull, blend,
        }),
        cone_angle,
        rol,
        limit,
    })
}

/// 原始 MinMax 值的恒定量（`constant` 取值；`twoConstants` 两臂相等取该
/// 值；其余 None——不是恒定值）。律解析前的门（发射率形状、三轴旋转）
/// 读原始块，用这个。
fn raw_const(value: Option<&Value>) -> Option<f64> {
    let object = value?.as_object()?;
    match object.get("mode").and_then(Value::as_str) {
        Some("constant") => object.get("value").and_then(Value::as_f64),
        Some("twoConstants") => {
            let min = object.get("min").and_then(Value::as_f64)?;
            let max = object.get("max").and_then(Value::as_f64)?;
            (min == max).then_some(min)
        }
        _ => None,
    }
}

/// 律侧 `MinMaxCurve` 的恒定量（同上口径）。
fn const_of(curve: &MinMaxCurve) -> Option<f32> {
    match curve {
        MinMaxCurve::Constant(v) => Some(*v),
        MinMaxCurve::TwoConstants { min, max } if min == max => Some(*min),
        _ => None,
    }
}

/// 节点链激活走查：任一祖先 `active == false` 即不活；祖先记录缺席按活
/// （与装载侧同口径——记录缺席不是数据说它关了）。
fn active_in_hierarchy(by_path: &HashMap<String, &Value>, path: &str) -> bool {
    let mut current = path.to_owned();
    while let Some(node) = by_path.get(&current) {
        if node.get("active").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let parent = node
            .get("parent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if current == parent {
            return true;
        }
        current = parent;
    }
    true
}

/// 根记录 → 发射节点链的 TRS 合成（父∘子）。记录缺席或 TRS 形状不对回
/// None（调用方按节点未解析挡下）。
fn compose_affine(by_path: &HashMap<String, &Value>, path: &str) -> Option<GlobalTransform> {
    let mut chain: Vec<&Value> = Vec::new();
    let mut current = path.to_owned();
    while let Some(node) = by_path.get(&current) {
        chain.push(*node);
        let parent = node
            .get("parent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if current == parent {
            break;
        }
        current = parent;
    }
    let mut affine = GlobalTransform::IDENTITY;
    for node in chain.iter().rev() {
        let position = triple(node.get("position"))?;
        let rotation = node
            .get("rotation")
            .and_then(Value::as_array)
            .filter(|list| list.len() == 4)?;
        let mut quat = [0.0f32; 4];
        for (slot, value) in quat.iter_mut().zip(rotation) {
            *slot = value.as_f64()? as f32;
        }
        let scale = triple(node.get("scale"))?;
        let local = Transform {
            translation: crate::particle_geometry::reflect(Vec3::from_array(position)),
            rotation: Quat::from_xyzw(quat[0], -quat[1], -quat[2], quat[3]),
            scale: Vec3::from_array(scale),
        };
        affine = affine * GlobalTransform::from(local);
    }
    Some(affine)
}

fn triple(value: Option<&Value>) -> Option<[f32; 3]> {
    let list = value?.as_array()?;
    if list.len() != 3 {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value.as_f64()? as f32;
    }
    Some(out)
}

// ---- 铺装与推进 ----

/// Update：贴图到齐后逐条铺实体与状态。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<UberParticleMaterial>>,
    plan: Option<ResMut<WeatherFxPlan>>,
    mut active: Option<ResMut<WeatherFxState>>,
    mut retiring: ResMut<WeatherFxRetirements>,
    anchor: Option<Res<WeatherFxAnchor>>,
    phase: Res<WeatherTransition>,
    time: Res<Time>,
    site: Option<Res<SiteActive>>,
    site_ready: Option<Res<crate::site::SiteScenesReady>>,
    gltfs: Res<Assets<Gltf>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
) {
    let Some(mut plan) = plan else { return; };
    if !anchor.as_ref().is_some_and(|a| a.tier == plan.tier && a.env_site == plan.env_site)
        || phase.destination.as_ref() != Some(&plan.selection) || phase.request_serial != plan.request_serial {
        commands.remove_resource::<WeatherFxPlan>();
        commands.remove_resource::<WeatherFxPrepared>();
        return;
    }
    for planned in &mut plan.planned {
        if let PlannedGeometry::Mesh { reference, glb, source, .. } = &mut planned.geometry {
            match (server.load_state(&*glb), server.recursive_dependency_load_state(&*glb)) {
                (LoadState::Failed(error), _) => panic!("source particle mesh {} failed: {error:?}", reference.file),
                (_, RecursiveDependencyLoadState::Failed(error)) => panic!("source particle mesh dependency {} failed: {error:?}", reference.file),
                _ => {},
            }
            if !server.is_loaded_with_dependencies(&*glb) { return; }
            if source.is_none() {
                let Some(asset) = gltfs.get(&*glb) else { return; };
                let node_handle = asset.named_nodes.get(reference.node.as_str())
                    .unwrap_or_else(|| panic!("source particle mesh node {} is absent in {}", reference.node, reference.file));
                let Some(node) = gltf_nodes.get(node_handle) else { return; };
                let mesh_handle = node.mesh.as_ref().expect("source particle mesh node must own geometry");
                let Some(asset_mesh) = gltf_meshes.get(mesh_handle) else { return; };
                let Some(primitives) = asset_mesh.primitives.iter().map(|p| meshes.get(&p.mesh)).collect::<Option<Vec<_>>>() else { return; };
                *source = Some(Arc::new(crate::particle_geometry::SourceMesh::from_primitives(&primitives, Vec3::from_array(reference.size()))
                    .unwrap_or_else(|error| panic!("invalid source particle mesh {}: {error}", reference.file))));
            }
        }
        match server.load_state(&planned.texture) {
            LoadState::Failed(err) => panic!("weather particle texture {} failed: {err:?}", planned.node),
            state if state.is_loaded() => {},
            _ => return,
        }
    }
    commands.insert_resource(WeatherFxPrepared {selection:plan.selection.clone(),request_serial:plan.request_serial});
    if !phase.can_start_site_fx(&plan.selection) { return; }

    let create_state = active.is_none();
    let mut created = WeatherFxState {selection:None, global_identity:None, sky_stopped:false, tier:plan.tier.clone(),env_site:plan.env_site.clone(),live:Vec::new(),admitted:0,records:plan.tally.records};
    let state = active.as_deref_mut().unwrap_or(&mut created);
    let now = time.elapsed_secs_f64();
    let phase_started = plan.site_started_at.is_none();
    if phase_started {
        retiring.stop_matching(state, now, |e| e.kind == EffectKind::Site);
        if state.global_identity != Some(plan.selection.global_effect) {
            // StopSkyEffect runs before PrepareCrossFade. Camera FX deliberately
            // continue until RefreshGlobalEffect at the commit point.
            retiring.stop_matching(state, now, |e| e.kind == EffectKind::Sky);
            state.sky_stopped = true;
        }
        state.tier = plan.tier.clone(); state.env_site = plan.env_site.clone();
        state.records = plan.tally.records;
        plan.site_started_at = Some(now);
    }
    // EmitCrossFadeUniqueEffect is a separately yielded task, not a condition
    // blocking the environment fade or the sky/camera commit.
    let waited = now - plan.site_started_at.unwrap();
    let controller_ready = site_ready.is_some() && site.as_ref().is_some_and(|site|
        site.site_id == plan.selection.site_id && site.env_site == plan.selection.environment_site);
    let install_site = !plan.site_installed && !phase_started && controller_ready;
    let site_timed_out = !plan.site_installed && !controller_ready && waited >= 5.0;
    if install_site || site_timed_out { plan.site_installed = true; }
    if site_timed_out { warn!("[weather-fx] destination site controller unavailable after source 5s timeout: {}", plan.env_site); }
    let preserve_global = state.global_identity == Some(plan.selection.global_effect) && !state.sky_stopped;
    let install_global = !plan.global_installed && !preserve_global && phase.can_commit_global_fx(&plan.selection);
    if install_global {
        retiring.stop_matching(state, now, |e| e.kind != EffectKind::Site);
        state.global_identity = Some(plan.selection.global_effect);
        state.sky_stopped = false;
    }
    if preserve_global || install_global { plan.global_installed = true; }
    if plan.global_installed && phase.can_commit_global_fx(&plan.selection) {
        commands.insert_resource(crate::weather_transition::WeatherGlobalFxCommitted(plan.request_serial));
    }
    let mut waiting = Vec::new();
    for planned in std::mem::take(&mut plan.planned) {
        let is_global = planned.kind != EffectKind::Site;
        if (is_global && preserve_global) || (!is_global && site_timed_out) { continue; }
        if (is_global && !install_global) || (!is_global && !install_site) {
            waiting.push(planned); continue;
        }
        let mesh = meshes.add(billboard::empty_mesh());
        let material = materials.add(UberParticleMaterial::new(
            planned.params,
            planned.texture.clone(),
            planned.tint_area,
            planned.plain_colour,
            planned.cull,
            planned.blend,
        ));
        // 实体变换恒等：四角已在 CPU 展开成世界坐标，属性即世界坐标。
        // 逐帧重建的属性池没有稳定包围盒，剔除交给 NoFrustumCulling 直通。
        let mut draw = commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            NoFrustumCulling,
            WeatherFxDraw,
            crate::shadowmap::NoShadowCast,
        ));
        if let Some(effect_pass) = planned.effect_pass { draw.insert(effect_pass); }
        state.live.push(LiveWeatherEmitter { draw: draw.id(), lifecycle: planned.lifecycle, runtime: Runtime {
            node: planned.node.clone(),
            effect: planned.effect.clone(),
            emitter: planned.emitter.clone(),
            kind: planned.kind,
            camera_rotation: planned.camera_rotation,
            node_affine: planned.node_affine,
            mesh,
            anchor: None,
            geometry: planned.geometry.into_runtime(),
            ring_cursor: 0,
            pool: Vec::new(),
            side: Vec::new(),
            emission: EmissionState::default(),
            playback_head: 0.0,
            previous_head: 0.0,
            // 逐系统换一条流：同一个种子在所有系统上会画出同一个图形。
            rng: Rng(RNG_SEED ^ (planned.ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            prewarmed: false,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            born_total: 0,
            died_total: 0,
            full_total: 0,
            refused_total: 0,
        }});
    }
    plan.planned = waiting;
    state.admitted = state.live.len();
    if install_site || install_global {
        info!("[weather-fx] {} @ {} phase site={} global={} active={} retiring={}",state.tier,state.env_site,install_site,install_global,state.live.len(),retiring.live.len());
    }
    if plan.site_installed && plan.global_installed { state.selection = Some(plan.selection.clone()); }
    if create_state { commands.insert_resource(created); }
    if plan.site_installed && plan.global_installed { commands.remove_resource::<WeatherFxPlan>(); }
}

/// PostUpdate（变换传播之后）：推进仿真并重建属性池。
///
/// 排在传播之后是因为**局部空间仿真**要读锚点的当帧世界变换；排在相机
/// 之后是因为四角展开要读当帧机位。
pub(crate) fn advance(
    mut state: Option<ResMut<WeatherFxState>>,
    mut retiring: ResMut<WeatherFxRetirements>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    avatars: Query<&GlobalTransform, With<AvatarRoot>>,
) {
    if state.as_ref().is_none_or(|s| s.live.is_empty()) && retiring.live.is_empty() { return; }
    // 相机拿不到就整帧跳过：不造替身机位（上一帧的属性池还在，几何
    // 不会闪成错的）。
    let Some((camera_transform, projection, camera)) = cameras.iter().next() else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let basis = billboard::basis_from_matrix(
        camera_transform.affine().matrix3.into(),
        camera_transform.translation(),
        perspective.fov,
        viewport.x as f32 / viewport.y.max(1) as f32,
    );
    // 天空锚：随玩家平移、不随旋转。玩家不在时落原点（穹顶锚不到人，
    // 粒子仍画，位在原点）。
    let sky = avatars
        .iter()
        .next()
        .map(|transform| GlobalTransform::from_translation(transform.translation()))
        .unwrap_or(GlobalTransform::IDENTITY);
    let ctx = Context {
        sky,
        camera: *camera_transform,
        site: GlobalTransform::IDENTITY,
    };

    let dt = time.delta_secs();
    let active = state.as_deref_mut().into_iter().flat_map(|s| s.live.iter_mut())
        .map(|s| (&mut s.runtime, true));
    let retired = retiring.live.iter_mut().map(|s| (&mut s.emitter.runtime, false));
    for (system, emitting) in active.chain(retired) {
        // 惰性 prewarm：首个推进帧把一个周期快进完（雨链同款；只动仿真
        // 状态不喂渲染，快进里的世界空间出生锚在首帧锚点上）。语料里
        // prewarm 只出现在循环系统上（49/49），非循环的 prewarm 未实现。
        if emitting && !system.prewarmed {
            system.prewarmed = true;
            if system.emitter.prewarm && system.emitter.looping && system.emitter.duration > 0.0
            {
                let steps = (system.emitter.duration / PREWARM_STEP).max(1.0) as usize;
                for _ in 0..steps {
                    simulate(system, PREWARM_STEP, &ctx);
                }
            }
        }
        let step = dt * system.emitter.simulation_speed;
        if step > 0.0 {
            if emitting { simulate(system, step, &ctx); }
            else { crate::particle_runtime::simulate_stopped(system, step, &ctx); }
        }
        // 局部空间仿真：律状态是发射节点局部坐标，用锚∘链的当帧值换算成
        // 世界坐标。世界空间仿真的状态出生时就是世界坐标，恒等。
        let to_world = match system.emitter.simulation_space {
            SimulationSpace::World => GlobalTransform::IDENTITY,
            _ => compose_to_world(system, &ctx),
        };
        let Some(mesh) = meshes.get_mut(&system.mesh) else {
            continue;
        };
        crate::particle_runtime::write_geometry(mesh, system, &to_world,
            &compose_to_world(system, &ctx), camera_transform, basis);
    }
}

/// Update：周期状态行——逐系统的活粒子数与累计账，全部可从档案复算。
pub(crate) fn report(state: Option<Res<WeatherFxState>>, retiring: Res<WeatherFxRetirements>) {
    if !retiring.live.is_empty() {
        info!("[weather-fx] retiring systems={}, live particles={}, active systems={}",
            retiring.live.len(), retiring.live.iter().map(|s| s.emitter.pool.len()).sum::<usize>(),
            state.as_ref().map_or(0, |s| s.live.len()));
    }
    let Some(state) = state else {
        return;
    };
    if state.live.is_empty() {
        info!(
            "[weather-fx] {} @ {}：本族记录 {}，放行 {}——本档本站无在跑的天气粒子",
            state.tier, state.env_site, state.records, state.admitted
        );
        return;
    }
    let live: usize = state.live.iter().map(|s| s.pool.len()).sum();
    let born: u64 = state.live.iter().map(|s| s.born_total).sum();
    let died: u64 = state.live.iter().map(|s| s.died_total).sum();
    let full: u64 = state.live.iter().map(|s| s.full_total).sum();
    let refused: u64 = state.live.iter().map(|s| s.refused_total).sum();
    info!(
        "[weather-fx] {} @ {}：在跑 {} 条系统，活粒子 {}（逐条 {:?}）；\
         累计出生 {} 死亡 {} 池满拒发 {} 积分拒绝 {}",
        state.tier,
        state.env_site,
        state.live.len(),
        live,
        state
            .live
            .iter()
            .map(|s| (s.node.as_str(), s.pool.len(), s.playback_head))
            .collect::<Vec<_>>(),
        born,
        died,
        full,
        refused,
    );
}

/// 拆链面：撤下全部资源（实体由 [`watch`] 的撤旧步收）。换站入口
/// （站点的 read_switch）与换档共用。
pub(crate) fn invalidate_site(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        if let Some(mut phase) = world.get_resource_mut::<WeatherTransition>() { phase.invalidate_site(); }
    });
    commands.remove_resource::<WeatherFxPlan>();
    commands.remove_resource::<WeatherFxPrepared>();
    commands.remove_resource::<WeatherFxDoc>();
    commands.remove_resource::<WeatherFxRequest>();
    commands.remove_resource::<WeatherFxAnchor>();
}

/// Full environment/session disposal. A normal site switch must not call this.
pub(crate) fn teardown(commands: &mut Commands) {
    // Destroying a site is distinct from weather StopEmitting. Retired draw
    // entities live outside the site hierarchy, so clear both generations here.
    commands.queue(|world: &mut World| {
        let entities: Vec<_> = world.query_filtered::<Entity, With<WeatherFxDraw>>().iter(world).collect();
        for entity in entities { world.despawn(entity); }
        if let Some(mut retiring) = world.get_resource_mut::<WeatherFxRetirements>() { retiring.live.clear(); }
    });
    commands.remove_resource::<WeatherFxPlan>();
    commands.remove_resource::<WeatherFxState>();
    commands.remove_resource::<WeatherFxPrepared>();
    commands.remove_resource::<WeatherFxDoc>();
    commands.remove_resource::<WeatherFxRequest>();
    commands.remove_resource::<WeatherFxAnchor>();
}

#[cfg(test)]
#[path = "weather_source_audit.rs"]
mod source_audit;

#[cfg(test)]
#[path = "weather_retirement_tests.rs"]
mod retirement_tests;
