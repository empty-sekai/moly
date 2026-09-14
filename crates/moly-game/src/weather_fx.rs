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

use bevy::asset::{AssetPath, LoadState};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::{Effects, EmissionState, EmitterParams, LimitVelocity, MinMaxCurve, RotationOverLifetime};
use serde_json::Value;
use std::collections::HashMap;

use crate::billboard::{self, Alignment, SizeClamp};
use crate::character::AvatarRoot;
use crate::site::SiteActive;
use crate::uber_particle::{BlendArm, CullArm, UberParticleMaterial, UberT1Params};
use crate::weather::CurrentPhenomenon;
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, compose_to_world, build_quads, simulate};

/// 源族的 shader 名（与站点链同一族、同一份材质实现）。
const SHADER_NAME: &str = "Mysekai/Effect/UberUnlit";

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
    tier: String,
    env_site: String,
}

/// 档位清单请求（`phenomena/index.json`）。
#[derive(Resource)]
pub(crate) struct WeatherFxRequest {
    index: Handle<JsonAsset>,
    tier: String,
    env_site: String,
}

/// 该档 effects.json 的请求与锚点账目。
#[derive(Resource)]
pub(crate) struct WeatherFxDoc {
    handle: Handle<JsonAsset>,
    tier: String,
    env_site: String,
}

/// 绘制实体标记：换档/换站时按它撤（这些实体不挂在任何场景树下）。
#[derive(Component)]
pub struct WeatherFxDraw;

/// 一条放行的粒子系统：律侧参数 + 锚定账目 + 呈现侧输入 + 待装载贴图。
struct Planned {
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
    cull: CullArm,
    blend: BlendArm,
    clamp: SizeClamp,
    pivot: [f32; 3],
    alignment: Alignment,
    texture: Handle<Image>,
    effect_pass: crate::uber_particle::ParticleEmission,
    /// Cone 的半顶角（shape 块的 `angle` 键；律的 `ShapeParams` 不带它）。
    cone_angle: Option<f32>,
    rol: Option<RotationOverLifetime>,
    limit: Option<LimitVelocity>,
}

/// 判读结果：放行的计划 + 逐档拒绝盘点。
#[derive(Resource)]
pub(crate) struct WeatherFxPlan {
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
    admitted: usize,
}

/// 全部在跑的系统。
#[derive(Resource)]
pub(crate) struct WeatherFxState {
    live: Vec<Runtime>,
    tier: String,
    env_site: String,
    admitted: usize,
    records: usize,
}

/// Update：锚点（档位 × 站点）变了 → 撤链、发清单请求。
pub(crate) fn watch(
    mut commands: Commands,
    server: Res<AssetServer>,
    phenomenon: Option<Res<CurrentPhenomenon>>,
    site: Option<Res<SiteActive>>,
    anchor: Option<Res<WeatherFxAnchor>>,
    stale: Query<Entity, With<WeatherFxDraw>>,
) {
    let (Some(phenomenon), Some(site)) = (phenomenon, site) else {
        return;
    };
    let tier = phenomenon.0.clone();
    let env_site = site.env_site.clone();
    if let Some(anchor) = &anchor {
        if anchor.tier == tier && anchor.env_site == env_site {
            return;
        }
    }
    for entity in &stale {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<WeatherFxPlan>();
    commands.remove_resource::<WeatherFxState>();
    commands.remove_resource::<WeatherFxDoc>();
    commands.remove_resource::<WeatherFxRequest>();
    commands.insert_resource(WeatherFxAnchor {
        tier: tier.clone(),
        env_site: env_site.clone(),
    });
    commands.insert_resource(WeatherFxRequest {
        // 同路径装载去重为同一句柄，反复换档不会堆请求。
        index: server.load::<JsonAsset>(AssetPath::from(
            "moly://phenomena/index.json".to_owned(),
        )),
        tier,
        env_site,
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
        handle,
        tier: request.tier.clone(),
        env_site: request.env_site.clone(),
    });
    commands.remove_resource::<WeatherFxRequest>();
}

/// Update：该档 effects.json 到达 → 选 effect、逐条判读。
pub(crate) fn plan(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    doc: Option<Res<WeatherFxDoc>>,
    planned: Option<Res<WeatherFxPlan>>,
    state: Option<Res<WeatherFxState>>,
) {
    if planned.is_some() || state.is_some() {
        return;
    }
    let Some(doc) = doc else {
        return;
    };
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
    commands.insert_resource(WeatherFxPlan {
        planned: plans,
        tally,
        tier: doc.tier.clone(),
        env_site: doc.env_site.clone(),
    });
    commands.remove_resource::<WeatherFxDoc>();
}

/// effect 档案的 `kind`（字符串原样）。
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
    if shader != Some(SHADER_NAME) {
        tally
            .other_shader
            .push(shader.unwrap_or("(缺 shader 名)").to_owned());
        return None;
    }
    tally.records += 1;
    if renderer.get("enabled").and_then(Value::as_bool) != Some(true) {
        tally.renderer_disabled += 1;
        return None;
    }
    if renderer.get("renderMode").and_then(Value::as_str) != Some("Billboard") {
        tally
            .render_mode
            .push(format!("{:?}", renderer.get("renderMode")));
        return None;
    }
    let alignment = renderer.get("alignment").and_then(Value::as_i64).unwrap_or(-1);
    let Some(alignment) = Alignment::from_render_space(alignment) else {
        tally
            .alignment
            .push(Alignment::render_space_name(alignment).to_owned());
        return None;
    };

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
        "Circle" | "Cone" | "Sphere" | "Hemisphere" | "SingleSidedEdge"
    ) {
        tally.shape.push(shape_type.to_owned());
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
    if start.get("rotation3D").and_then(Value::as_bool) == Some(true) {
        let x = raw_const(start.get("rotationX"));
        let y = raw_const(start.get("rotationY"));
        if !x.map_or(true, |v| v == 0.0) || !y.map_or(true, |v| v == 0.0) {
            tally.start_rotation_3d += 1;
            return None;
        }
    }

    // ---- 状态档（材质 uniform 的分支全关才走 T1 片元链） ----
    let floats = material.get("floats").and_then(Value::as_object);
    let get = |key: &str| floats.and_then(|f| f.get(key)).and_then(Value::as_f64);
    if get("_BlendSrc") != Some(5.0) {
        tally.state_arm.push(format!("_BlendSrc={:?}", get("_BlendSrc")));
        return None;
    }
    let blend = match get("_BlendDst") {
        Some(v) if v == 10.0 => BlendArm::AlphaBlend,
        Some(v) if v == 1.0 => BlendArm::Additive,
        other => {
            tally.state_arm.push(format!("_BlendDst={other:?}"));
            return None;
        }
    };
    let cull = match get("_Cull") {
        Some(v) if v == 0.0 => CullArm::Off,
        Some(v) if v == 1.0 => CullArm::Front,
        Some(v) if v == 2.0 => CullArm::Back,
        other => {
            tally.state_arm.push(format!("_Cull={other:?}"));
            return None;
        }
    };
    if get("_ZTest") != Some(4.0) {
        tally.state_arm.push(format!("_ZTest={:?}", get("_ZTest")));
        return None;
    }
    for name in [
        "_FakeLightEnabled",
        "_TranceparencyByLuminanceEnabled",
        "_PhenomenaLightEnabled",
        "_BaseMapRotationEnabled",
    ] {
        if get(name) != Some(0.0) {
            tally.state_arm.push(format!("{name}={:?}", get(name)));
            return None;
        }
    }
    for name in [
        "_TintBlendRateCoord",
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
    let uri = material
        .pointer("/textures/_BaseMap")
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
    // Cone 的半顶角在 shape 块的 `angle` 键（律的 ShapeParams 不映射它）。
    let cone_angle = if shape_type == "Cone" {
        match shape.get("angle").and_then(Value::as_f64) {
            Some(angle) => Some(angle as f32),
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
        .pointer("/textureScaleOffset/_BaseMap")
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
    let tint_colour = material
        .pointer("/colors/_TintColor")
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

    Some(Planned {
        node: node.to_owned(),
        effect: effect_name.to_owned(),
        emitter,
        kind,
        camera_rotation,
        node_affine,
        params: UberT1Params {
            base_st: Vec4::from_array(base_st),
            tint_colour: Vec4::from_array(tint_colour),
            scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, 0.0, 0.0),
        },
        tint_area,
        cull,
        blend,
        clamp: SizeClamp {
            max_screen_fraction: max_particle_size as f32,
            min_size: MIN_PARTICLE_SIZE,
        },
        pivot,
        alignment,
        texture,
        effect_pass: crate::uber_particle::ParticleEmission {
            params: UberT1Params { base_st: Vec4::from_array(base_st), tint_colour: Vec4::from_array(tint_colour), scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, 0.0, 0.0) },
            colour: Vec4::from_array(std::array::from_fn(|i| material.pointer("/colors/_EmissionColor").and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(1.0) as f32)),
            intensity: get("_EmissionIntensity").unwrap_or(1.0) as f32,
            colour_type: material.pointer("/ints/_EmissionColorType").and_then(Value::as_f64).or_else(|| get("_EmissionColorType")).unwrap_or(0.0) as f32,
            area: keywords.iter().any(|k| *k == "_EMISSION_AREA_ALL"),
            tint_area, cull, blend,
        },
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
            translation: Vec3::from_array(position),
            rotation: Quat::from_array(quat),
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
    plan: Option<Res<WeatherFxPlan>>,
) {
    let Some(plan) = plan else {
        return;
    };
    if plan.planned.is_empty() {
        commands.insert_resource(WeatherFxState {
            live: Vec::new(),
            tier: plan.tier.clone(),
            env_site: plan.env_site.clone(),
            admitted: 0,
            records: plan.tally.records,
        });
        commands.remove_resource::<WeatherFxPlan>();
        return;
    }
    for planned in &plan.planned {
        match server.load_state(&planned.texture) {
            LoadState::Failed(err) => {
                panic!("天气粒子基础贴图装载失败（{}）：{err:?}", planned.node)
            }
            state if state.is_loaded() => {}
            // 还有没到的：整批等齐再铺（材质的 bind group 需要贴图在场）。
            _ => return,
        }
    }
    let mut live = Vec::with_capacity(plan.planned.len());
    for (index, planned) in plan.planned.iter().enumerate() {
        let mesh = meshes.add(billboard::empty_mesh());
        let material = materials.add(UberParticleMaterial::new(
            planned.params,
            planned.texture.clone(),
            planned.tint_area,
            planned.cull,
            planned.blend,
        ));
        // 实体变换恒等：四角已在 CPU 展开成世界坐标，属性即世界坐标。
        // 逐帧重建的属性池没有稳定包围盒，剔除交给 NoFrustumCulling 直通。
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            NoFrustumCulling,
            WeatherFxDraw,
            planned.effect_pass,
            crate::shadowmap::NoShadowCast,
        ));
        live.push(Runtime {
            node: planned.node.clone(),
            effect: planned.effect.clone(),
            emitter: planned.emitter.clone(),
            kind: planned.kind,
            camera_rotation: planned.camera_rotation,
            node_affine: planned.node_affine,
            mesh,
            anchor: None,
            alignment: planned.alignment,
            ring_cursor: 0,
            clamp: planned.clamp,
            pivot: planned.pivot,
            pool: Vec::new(),
            side: Vec::new(),
            emission: EmissionState::default(),
            playback_head: 0.0,
            previous_head: 0.0,
            // 逐系统换一条流：同一个种子在所有系统上会画出同一个图形。
            rng: Rng(RNG_SEED ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            prewarmed: false,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            born_total: 0,
            died_total: 0,
            full_total: 0,
            refused_total: 0,
        });
    }
    info!(
        "[weather-fx] {} @ {} 上屏：放行 {} 条粒子系统（本族记录 {}），逐条 {:?}",
        plan.tier,
        plan.env_site,
        live.len(),
        plan.tally.records,
        live.iter()
            .map(|l| format!("{}/{}", l.effect, l.node))
            .collect::<Vec<_>>(),
    );
    commands.insert_resource(WeatherFxState {
        live,
        tier: plan.tier.clone(),
        env_site: plan.env_site.clone(),
        admitted: plan.planned.len(),
        records: plan.tally.records,
    });
    commands.remove_resource::<WeatherFxPlan>();
}

/// PostUpdate（变换传播之后）：推进仿真并重建属性池。
///
/// 排在传播之后是因为**局部空间仿真**要读锚点的当帧世界变换；排在相机
/// 之后是因为四角展开要读当帧机位。
pub(crate) fn advance(
    state: Option<ResMut<WeatherFxState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    avatars: Query<&GlobalTransform, With<AvatarRoot>>,
) {
    let Some(mut state) = state else {
        return;
    };
    if state.live.is_empty() {
        return;
    }
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
    let state = &mut *state;
    for system in &mut state.live {
        // 惰性 prewarm：首个推进帧把一个周期快进完（雨链同款；只动仿真
        // 状态不喂渲染，快进里的世界空间出生锚在首帧锚点上）。语料里
        // prewarm 只出现在循环系统上（49/49），非循环的 prewarm 未实现。
        if !system.prewarmed {
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
            simulate(system, step, &ctx);
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
        let quads = build_quads(system, &to_world);
        billboard::write_quads(
            mesh,
            &quads,
            system.alignment,
            basis,
            system.clamp,
            system.pivot,
        );
    }
}

/// Update：周期状态行——逐系统的活粒子数与累计账，全部可从档案复算。
pub(crate) fn report(state: Option<Res<WeatherFxState>>) {
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
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<WeatherFxPlan>();
    commands.remove_resource::<WeatherFxState>();
    commands.remove_resource::<WeatherFxDoc>();
    commands.remove_resource::<WeatherFxRequest>();
    commands.remove_resource::<WeatherFxAnchor>();
}
