//! Weather effect lifecycle and particle simulation, with source-addressed
//! material programs submitted through the scene renderer. Forward/effect GPU
//! readiness precedes environment preparation; superseded preflight entities
//! are discarded without interrupting the currently committed effects.
//! Geometry and simulation gaps remain explicit admission failures.

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
use crate::source_particle::{SourceParticle, ParticleReadiness};
use moly_assets::source_shader::SourceShaderCatalogue;
use crate::weather_transition::{EnvironmentSelection, GlobalEffectIdentity, WeatherTransition, WeatherFxPrepared};
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, compose_to_world, simulate};

/// 零缩放的四边形没有面积，画不出来；尺寸下限。
const MIN_PARTICLE_SIZE: f32 = 0.0001;

/// 出生抽签的确定性随机种子。与站点链**不同流**：两条链同时在跑，
/// 同流会在两族上画出同一图形的错觉（逐系统再乘质数散列）。
const RNG_SEED: u64 = 0x7765_6174_0001_0125;

/// prewarm 快进步长：与运行帧率同一量级（雨链同值）。prewarm 的档位
/// 语义是「开播前把一个周期快进完」，不是精确复算。
const PREWARM_STEP: f32 = 1.0 / 60.0;

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
#[derive(Component)]
pub(crate) struct WeatherFxPreflight(u64);
pub(crate) fn cleanup_preflight(mut commands: Commands, phase: Res<WeatherTransition>, draws: Query<(Entity, &WeatherFxPreflight)>) {
    for (entity, preflight) in &draws {
        if preflight.0 != phase.request_serial { commands.entity(entity).try_despawn(); }
    }
}

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
    source: SourceParticle,
    draw: Option<(Entity, Handle<Mesh>)>,
    geometry: PlannedGeometry,
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

impl WeatherFxState {
    /// Read-only simulation and generated geometry evidence. Admission alone
    /// does not establish particle birth, visibility or source equivalence.
    pub(crate) fn diagnostics(&self, meshes: &Assets<Mesh>) -> Value {
        serde_json::json!({
            "phenomenon": self.tier, "site": self.env_site,
            "records": self.records, "admitted": self.admitted,
            "emitters": self.live.iter().map(|s| {
                let mesh = meshes.get(&s.mesh);
                serde_json::json!({
                    "effect": s.effect, "node": s.node,
                    "alive": s.pool.len(), "born": s.born_total, "died": s.died_total,
                    "poolFull": s.full_total, "integrationRefused": s.refused_total,
                    "playbackTime": s.playback_head,
                    "meshVertices": mesh.map(Mesh::count_vertices),
                    "meshIndices": mesh.and_then(Mesh::indices).map(|indices| indices.len()),
                    "unmappedSimulationFields": s.emitter.unmapped,
                })
            }).collect::<Vec<_>>()
        })
    }
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
    tally.records += 1;
    let renderer = match particle.get("renderer").filter(|v| v.is_object()) {
        Some(renderer) => renderer,
        None => {
            tally.no_renderer += 1;
            return None;
        }
    };
    let material = renderer.get("material").filter(|v| v.is_object());
    let _material = match material {
        Some(material) => material,
        None => {
            tally.no_material += 1;
            return None;
        }
    };
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
    let shape = system.get("shape").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    let shape_type = if let Some(shape) = shape {
        if system.get("shapeEnabled").and_then(Value::as_bool) == Some(false) {
            tally.shape.push("disabled Shape module unexpectedly carries active parameters".into());
            return None;
        }
        let shape_type = shape.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(shape_type, "Circle" | "Cone" | "ConeVolume" | "Sphere" | "Hemisphere" | "SingleSidedEdge" | "Donut") {
            tally.shape.push(shape_type.to_owned()); return None;
        }
        if let Some(reason) = source_shape_admission(shape) {
            tally.shape.push(format!("{shape_type}: {reason}")); return None;
        }
        shape_type
    } else if system.get("shapeEnabled").and_then(Value::as_bool) == Some(false) {
        ""
    } else {
        tally.no_shape += 1;
        return None;
    };
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

    let source = match SourceParticle::load(renderer, server) {
        Ok(source) => source,
        Err(error) => { tally.law_reject.push(format!("source material: {error}")); return None; }
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
    // An invalid module invalidates this emitter; it never becomes a different
    // simulation with rotation or velocity limiting silently removed.
    let rol = match emitter.rotation_over_lifetime.as_ref().map(|p|
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve)).transpose() {
        Ok(value) => value,
        Err(error) => { tally.rol_refused.push(format!("{node}: {error}")); return None; }
    };
    let limit = match emitter.limit_velocity.as_ref().map(|p|
        LimitVelocity::from_parts(p.separate_axis, &p.magnitude, p.dampen, p.drag.as_ref(),
            p.multiply_drag_by_size, p.multiply_drag_by_velocity)).transpose() {
        Ok(value) => value,
        Err(error) => { tally.limit_refused.push(format!("{node}: {error}")); return None; }
    };

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

    Some(Planned {
        ordinal: 0,
        node: node.to_owned(),
        effect: effect_name.to_owned(),
        lifecycle,
        emitter,
        kind,
        camera_rotation,
        node_affine,
        source,
        draw: None,
        geometry: if let Some(reference) = mesh_reference {
            let glb = server.load(AssetPath::from_path_buf(std::path::PathBuf::from(format!("phenomena/{}", reference.file))).with_source("moly"));
            PlannedGeometry::Mesh { reference, glb, alignment: mesh_alignment.expect("validated Mesh alignment"), source: None, pivot: Vec3::from_array(pivot) }
        } else {
            PlannedGeometry::Billboard { alignment: alignment.expect("validated Billboard alignment"),
                clamp: SizeClamp { max_screen_fraction: max_particle_size as f32, min_size: MIN_PARTICLE_SIZE }, pivot }
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
    catalogues: Res<Assets<SourceShaderCatalogue>>,
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
    let request_serial = plan.request_serial;
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
        if let Err(error) = planned.source.resolve(&server, &catalogues) {
            if planned.source.error.as_deref() != Some(&error.0) { error!(%error, "weather source material unresolved"); }
            planned.source.error = Some(error.0);
            return;
        }
        if planned.source.passes.is_empty() { return; }
        if planned.draw.is_none() {
            let mesh = meshes.add(billboard::empty_mesh());
            let draw = commands.spawn((Mesh3d(mesh.clone()), planned.source.clone(), Transform::IDENTITY,
                NoFrustumCulling, WeatherFxPreflight(request_serial), crate::shadowmap::NoShadowCast)).id();
            planned.draw = Some((draw, mesh));
        }
    }
    for planned in &mut plan.planned {
        if let ParticleReadiness::Failed(error) = &*planned.source.readiness.lock().unwrap() {
            if planned.source.error.as_ref() != Some(error) { error!(%error, node=%planned.node, "weather GPU preparation failed"); }
            planned.source.error = Some(error.clone());
            return;
        }
    }
    if !plan.planned.iter().all(|p| matches!(*p.source.readiness.lock().unwrap(), ParticleReadiness::Ready)) { return; }
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
        if (is_global && preserve_global) || (!is_global && site_timed_out) {
            if let Some((draw, _)) = planned.draw { commands.entity(draw).try_despawn(); }
            continue;
        }
        if (is_global && !install_global) || (!is_global && !install_site) {
            waiting.push(planned); continue;
        }
        let (draw, mesh) = planned.draw.expect("GPU preflight must precede installation");
        let mut source = planned.source;
        source.enabled = true;
        commands.entity(draw).remove::<WeatherFxPreflight>().insert((source, WeatherFxDraw));
        state.live.push(LiveWeatherEmitter { draw, lifecycle: planned.lifecycle, runtime: Runtime {
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
            emission_started: false,
            // 逐系统换一条流：同一个种子在所有系统上会画出同一个图形。
            rng: Rng(RNG_SEED ^ (planned.ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            prewarmed: false,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            velocity_law: planned.emitter.velocity_over_lifetime.as_ref()
                .map(moly_law::particle::velocity::VelocityOverLifetime::from_params),
            size_law: planned.emitter.size_over_lifetime.as_ref()
                .map(moly_law::particle::size::SizeOverLifetime::from_params),
            color_law: planned.emitter.color_over_lifetime.as_ref()
                .map(moly_law::particle::color::ColorOverLifetime::from_params),
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
