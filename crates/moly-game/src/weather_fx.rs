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
use moly_law::particle::schema::{ShapeTexture, SimulationSpace};
use moly_law::particle::{Effects, EmissionState, EmitterParams, LimitVelocity, MinMaxCurve, RotationOverLifetime};
use moly_law::particle::noise::NoiseLaw;
use serde_json::Value;
use std::collections::HashMap;
use moly_assets::weather_effect::WeatherEffectLifecycle;

use crate::billboard::{self, Alignment};
use crate::character::AvatarRoot;
use crate::site::SiteActive;
use crate::source_particle::{SourceParticle, ParticleReadiness};
use moly_assets::source_shader::SourceShaderCatalogue;
use crate::weather_transition::{EnvironmentSelection, GlobalEffectIdentity, WeatherTransition, WeatherFxPrepared};
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, compose_to_world};
pub(crate) mod fixture;

/// 出生抽签的确定性随机种子。与站点链**不同流**：两条链同时在跑，
/// 同流会在两族上画出同一图形的错觉（逐系统再乘质数散列）。
const RNG_SEED: u64 = 0x7765_6174_0001_0125;

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
    animations: Option<Handle<JsonAsset>>,
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
    Billboard(crate::source_billboard::Draw),
    Mesh {
        reference: ParticleMeshReference,
        glb: Handle<Gltf>,
        alignment: crate::particle_geometry::Alignment,
        source: Option<Arc<crate::particle_geometry::SourceMesh>>,
        scaling: crate::particle_geometry::Scaling,
        pivot: Vec3,
        flip: Vec3,
    },
}
impl PlannedGeometry {
    /// The audit's view of the emitter state; admission builds the same
    /// evidence from its locals before the geometry exists.
    #[cfg(test)]
    fn shape_evidence(&self) -> crate::particle_runtime::ShapeEmitterEvidence {
        match self {
            Self::Billboard(draw) => crate::particle_runtime::ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: false },
            Self::Mesh { scaling, .. } => crate::particle_runtime::ShapeEmitterEvidence { scaling: *scaling, mesh_renderer: true },
        }
    }
    fn into_runtime(self) -> crate::particle_runtime::Geometry {
        match self {
            Self::Billboard(draw) => crate::particle_runtime::Geometry::SourceBillboard(draw),
            Self::Mesh { alignment, source, scaling, pivot, flip, .. } => crate::particle_runtime::Geometry::Mesh(crate::particle_geometry::MeshDraw {
                source: source.expect("source mesh readiness must precede weather commit"), alignment, scaling, pivot, flip,
            }),
        }
    }
}

struct PlannedSurface {
    reference: ParticleMeshReference,
    glb: Handle<Gltf>,
    source: Option<Arc<crate::particle_mesh_emission::EmissionSurface>>,
}

/// 一条放行的粒子系统：律侧参数 + 锚定账目 + 呈现侧输入 + 待装载贴图。
struct Planned {
    ordinal: usize,
    node: String,
    effect: String,
    emitter: EmitterParams,
    /// Native update route read from the exported block; decides whether the
    /// native birth owner may be installed.
    route: crate::particle_runtime::SourceRoute,
    kind: EffectKind,
    /// camera 档 `effectiveRotation == "normal"`（继承相机旋转）；
    /// `"fix"` 与缺省都不继承。
    camera_rotation: bool,
    /// 根记录 → 发射节点链的 TRS 合成（语料根记录全档恒等，仍参与合成）。
    /// 链缩放由出生步从它折算（X 分量；语料全部均匀）。
    node_affine: GlobalTransform,
    /// The authored chain `node_affine` was composed from; `None` when an
    /// instance anchor replaces it.
    node_chain: Option<NodeChain>,
    /// A Transform on `node_chain` is rotated by the effect's accepted Animator.
    animated: bool,
    source: SourceParticle,
    draw: Option<(Entity, Handle<Mesh>)>,
    geometry: PlannedGeometry,
    emission_surface: Option<PlannedSurface>,
    lifecycle: Option<WeatherEffectLifecycle>,
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
    /// Accepted Animator players per selected effect, instantiated with the effect.
    animators: HashMap<String, Vec<crate::weather_animation::AnimatedNode>>,
    tally: Tally,
    tier: String,
    env_site: String,
}

/// 逐档盘点。**每一格都是「这一档有多少条被挡在外面」**——盘面上看得见
/// 还差什么，是这条通路唯一诚实的进度量。
#[derive(Default, Debug)]
struct Tally {
    animation_refused: Vec<String>,
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
    /// The source EmissionModule is disabled and no owner can emit into the
    /// system, so it never holds a particle. Counted, not refused.
    source_emission_disabled: usize,
    /// Any authored distance emission requires an emitter-travel consumer.
    rate_distance_only: usize,
    /// 率恒 0 且无 burst：永不发射。
    dead_emission: usize,
    no_shape: usize,
    shape: Vec<String>,
    sim_space: usize,
    /// Historical admission counter, retained for diagnostic compatibility.
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
                let bounds = (!s.pool.is_empty()).then(|| {
                    let mut min = Vec3::splat(f32::INFINITY);
                    let mut max = Vec3::splat(f32::NEG_INFINITY);
                    for particle in &s.pool {
                        let position = Vec3::from_array(particle.position);
                        min = min.min(position); max = max.max(position);
                    }
                    serde_json::json!({"min":min.to_array(),"max":max.to_array()})
                });
                let geometry = match &s.geometry {
                    crate::particle_runtime::Geometry::SourceBillboard(draw) => match draw.mode {
                        crate::source_billboard::Mode::Billboard => "source_billboard",
                        crate::source_billboard::Mode::Horizontal => "source_horizontal_billboard",
                    },
                    crate::particle_runtime::Geometry::Mesh(_) => "source_mesh",
                    crate::particle_runtime::Geometry::Billboard { .. } => "legacy_billboard",
                };
                let sheet = s.texture_sheet.map(|sheet| {
                    let first = s.side.first();
                    let uv = mesh.and_then(|mesh| match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
                        Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) => Some(uv.iter().take(4).copied().collect::<Vec<_>>()),
                        _ => None,
                    });
                    serde_json::json!({"firstSeed": first.map(|p| p.seed),
                        "firstTablePosition": first.map(|p| sheet.position(p.seed)), "firstUvs": uv})
                });
                serde_json::json!({
                    "effect": s.effect, "node": s.node,
                    "alive": s.pool.len(), "born": s.born_total, "died": s.died_total,
                    "poolFull": s.full_total, "integrationRefused": s.refused_total,
                    "playbackTime": s.playback_head,
                    "nativeRefusal": s.native_refusal,
                    "nativeBirth": s.native_birth.as_ref().map(|birth|serde_json::json!({
                        "ownerSeed":birth.owner.map(|owner|owner.seed),
                        "automaticSeed":birth.owner.map(|owner|owner.automatic),
                        "initialWords":birth.initial.words,
                        "emissionWords":birth.emission.random.words,
                        "firstSeed":s.side.first().map(|side|side.seed),
                        "firstAgePercent":s.pool.first().map(|particle|particle.age_percent),
                        "firstInverseLifetime":s.pool.first().map(|particle|particle.inverse_lifetime),
                    })),
                    "noiseConsumer": s.noise.as_ref().map(|noise| serde_json::json!({
                        "ownerSeed": noise.owner_seed,
                        "automaticSeed": noise.owner.automatic,
                        "scroll": noise.state.scroll,
                        "qualifiedLaw": true,
                    })),
                    "effectAge": s.effect_clock.age(),
                    "animator": s.effect_animator.as_ref().map(|animator| animator.report()),
                    "animatedChain": s.animated_chain.is_some(),
                    "geometry": geometry,
                    "particleSort": format!("{:?}", s.sort_mode),
                    "textureSheet": sheet,
                    "forceOverLifetime": s.force_law.as_ref().map(|law| {
                        let first = s.side.first().zip(s.pool.first());
                        serde_json::json!({"worldSpace":law.in_world_space,
                            "firstSeed": first.map(|(side, _)| side.seed),
                            "firstAgePercent": first.map(|(_, particle)| particle.normalized_age() * 100.0),
                            "firstSample": first.map(|(side, particle)| law.sample(side.seed, particle.normalized_age() * 100.0)),
                            "firstVelocity": first.map(|(_, particle)| particle.velocity)})
                    }),
                    "emissionSurfaceTriangles": s.emission_surface.as_ref().map(|surface| surface.triangles()),
                    "particleBounds": bounds,
                    "nonFiniteParticles": s.pool.iter().filter(|p| p.position.iter().chain(p.velocity.iter()).any(|v| !v.is_finite())).count(),
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
    /// Named reason the native birth owner was not installed (legacy step).
    native_refusal: Option<String>,
    draw: Entity,
    lifecycle: WeatherEffectLifecycle,
    effect_clock: Arc<crate::weather_animation::EffectClock>,
    /// The effect instance's Animator, shared by every emitter of the instance
    /// and kept through retirement (the GameObject outlives its particles).
    effect_animator: Option<Arc<crate::weather_animation::EffectAnimator>>,
    /// Set when the Animator rotates a Transform on this emitter's chain: the
    /// node affine is recomposed from it every frame.
    animated_chain: Option<NodeChain>,
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
    let animations = value.pointer(&format!("/phenomena/{}/animations/file", request.tier))
        .and_then(Value::as_str).map(|file| server.load::<JsonAsset>(AssetPath::from(format!("moly://phenomena/{file}"))));
    commands.insert_resource(WeatherFxDoc {
        request_serial:request.request_serial,
        handle,
        animations,
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
    let animation_document = if let Some(handle) = &doc.animations {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("现象 {} 的动画源档案装载失败：{err:?}", doc.tier);
        }
        let Some(asset) = json.get(handle) else { return; };
        Some(serde_json::from_str::<Value>(&asset.0).unwrap_or_else(|err| panic!("weather animation source JSON: {err}")))
    } else { None };

    // ---- 选 effect：只取源环境装载器按名字构造的三份预制件（见 source_environment_selection）----
    let selected = source_environment_selection(effects, &doc.tier, &doc.env_site);

    let mut tally = Tally::default();
    let mut plans = Vec::new();
    let mut animators = HashMap::new();
    for &(ref effect_name, effect, kind) in &selected {
        let animation = crate::weather_animation::Contract::compile(effect, animation_document.as_ref());
        info!("[weather-animation] {} {}", effect_name, animation.report);
        if !animation.players().is_empty() {
            animators.insert(effect_name.clone(), animation.players().to_vec());
        }
        let lifecycle = WeatherEffectLifecycle::from_effect(effect)
            .unwrap_or_else(|err| panic!("weather effect {effect_name}: {err}"));
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
        let sub_emitter_owners = source_sub_emitter_owners(particles);
        for particle in particles {
            if let Some(reason) = particle["node"].as_str().and_then(|node| animation.refusal(node)) {
                tally.records += 1;
                tally.animation_refused.push(format!("{effect_name}/{}: {reason}", particle["node"].as_str().unwrap_or("")));
                continue;
            }
            match judge(
                effect_name,
                particle,
                &by_path,
                &sub_emitter_owners,
                kind,
                camera_rotation,
                lifecycle,
                &server,
                &mut tally,
            ) {
                Some(planned) => {
                    if let Some(planned) = admit_animated(&animation, planned, &mut tally) {
                        tally.admitted += 1;
                        plans.push(planned);
                    }
                }
                None => {}
            }
        }
    }
    info!(
        "[weather-fx] {} @ {} 判读：选中 effect {} 个；本族记录 {}；放行 {}；\
         挡下——无渲染器 {} · 无材质 {} · 非本族 {:?} · 渲染器关 {} · \
         绘制模式 {:?} · 对齐档 {:?} · 缺 system 块 {} · 缺 emission {} · 源发射模块关 {} · \
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
        tally.source_emission_disabled,
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
    if !tally.animation_refused.is_empty() {
        warn!("[weather-animation] source playback refused: {:?}", tally.animation_refused);
    }
    let Some(selection) = phase.destination.as_ref().filter(|selection| selection.name == doc.tier && selection.environment_site == doc.env_site) else { return; };
    for (ordinal, planned) in plans.iter_mut().enumerate() { planned.ordinal = ordinal; }
    commands.insert_resource(WeatherFxPlan {
        selection: selection.clone(), request_serial:doc.request_serial, site_started_at:None, site_installed:false, global_installed:false,
        planned: plans,
        animators,
        tally,
        tier: doc.tier.clone(),
        env_site: doc.env_site.clone(),
    });
    commands.remove_resource::<WeatherFxDoc>();
}

/// Module capability is checked from the complete serialized inventory, not
/// just whichever parameters an older producer happened to emit.
fn source_simulation_admission(system: &Value) -> Result<(), String> {
    // A +Infinity burst repeat interval is kept at the schema boundary; the
    // native birth schedules it as the source does, and the legacy step
    // refuses it (`legacy_bursts`, below where the birth path is known).
    let source = moly_assets::particle_source::ParticleSourceModules::from_system(system)?;
    for module in &source.enabled {
        // The current snow owner carries an authored null SubModule edge
        // (emitter=null, sourcePointer 0/0). It does not name a child system
        // and therefore adds no runtime scheduling obligation. Keep real
        // non-null SubModule edges gated below via the ordinary capability
        // rejection path.
        if module == "SubModule"
            && system.get("subEmitters").and_then(Value::as_array).is_some_and(|entries| {
                !entries.is_empty()
                    && entries.iter().all(|entry| entry.get("emitter") == Some(&Value::Null)
                        && entry.pointer("/sourcePointer/fileId").and_then(Value::as_i64) == Some(0)
                        && entry.pointer("/sourcePointer/pathId").and_then(Value::as_str) == Some("0"))
            })
        {
            continue;
        }
        let field = match module.as_str() {
            "InitialModule" => "start", "EmissionModule" => "emission",
            "ShapeModule" => "shape", "ColorModule" => "colorOverLifetime",
            "SizeModule" => "sizeOverLifetime", "RotationModule" => "rotationOverLifetime",
            "VelocityModule" => "velocityOverLifetime", "ClampVelocityModule" => "limitVelocity",
            "CustomDataModule" => "customData", "UVModule" => "textureSheet",
            "ForceModule" => "forceOverLifetime",
            // Noise is consumed only together with the native birth owner;
            // judge() checks that route/composition after the law parse.
            "NoiseModule" => "noise",
            _ => return Err(format!("enabled source module {module} has no runtime consumer")),
        };
        if !system.get(field).is_some_and(Value::is_object) {
            return Err(format!("enabled source module {module} lacks its authored {field} parameters"));
        }
    }
    if system.get("ringBufferMode").and_then(Value::as_u64) != Some(0) {
        return Err("source RingPause/RingLoop lifecycle is not yet verified".into());
    }
    if system.get("start").and_then(|s| s.get("randomizeRotationDirection")).and_then(Value::as_f64) != Some(0.0) {
        return Err("source birth rotation direction randomization is not consumed".into());
    }
    Ok(())
}

/// A system whose serialized EmissionModule is disabled never births a
/// particle through the engine's own update: the per-frame update, the
/// emitter-travel pass and the prewarm pass all skip rate and burst emission
/// on the module's enabled flag. The remaining ways in are a script call to
/// ParticleSystem.Emit or SetParticles, or one that re-enables the module
/// (the game makes none on the weather effects: its effect component only
/// plays and stops them), an animation or timeline binding (the weather clips
/// bind transforms, the weather timeline tracks drive environment colours and
/// values) and a parent's sub-emitter event, which births into the child
/// regardless of the child's enabled flag; the owner check in judge refuses
/// that case before this point. Only the module inventory the evidence covers
/// is accepted: the main module alone. Such a system still plays and stops
/// with its effect and reads as playing until its clock reaches the duration;
/// it holds no particle and draws nothing, so nothing is installed for it.
fn source_emission_disabled(system: &Value) -> bool {
    moly_assets::particle_source::ParticleSourceModules::from_system(system)
        .is_ok_and(|source| source.enabled.len() == 1 && source.enabled[0] == "InitialModule")
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
    if kind != "Mesh" && shape.get(mode).and_then(Value::as_str) != Some("Random") {
        return Some(format!("{mode} {:?} consumer pending", shape.get(mode)));
    }
    if shape.get("alignToDirection").and_then(Value::as_bool) != Some(false) {
        return Some("source aligned start rotation consumer pending".into());
    }
    for key in ["randomDirectionAmount", "sphericalDirectionAmount"] {
        if shape.get(key).and_then(Value::as_f64) != Some(0.0) {
            return Some(format!("source {key} consumer pending"));
        }
    }
    if !shape.get("randomPositionAmount").and_then(Value::as_f64)
        .is_some_and(|value| value.is_finite() && value >= 0.0 && value <= f32::MAX as f64) {
        return Some("missing/invalid authored position randomization".into());
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

/// Which of the source environment loader's prefabs an exported effect is, if any.
///
/// `SiteEnvironmentAssetBundleLoader.LoadEnvironmentDataAsync` builds exactly three prefab names from the
/// phenomenon package name P and the site package name S, and reads no other `GameObject` from a phenomenon
/// package:
/// - sky `fx_env_sky_` + P and camera `fx_env_camera_` + P, each read from the site's unique package when that
///   package holds it, otherwise from the global package;
/// - the unique effect `fx_env_site_` + P + `_` + S, read from the unique package only; the global package is
///   never asked for it.
///
/// `SiteEnvironmentUtility.EmitEffect` instantiates them under the sky view's effect root, the field camera's
/// effect root and the site view. Every other prefab of a phenomenon package is never instantiated on its own.
/// This includes the templates in the shared `common` package (the rain-night raindrop tree, the snow-night ice,
/// the rain and thunder raindrops, the meteor and rainbow ground trees):
/// - the loader never names the common package; it arrives only as a bundle dependency of the global and unique
///   packages, for its materials and meshes;
/// - no code names those prefabs, and no serialized object references a template member.
///
/// The templates reach the screen as baked copies nested inside the unique site prefab, so they are simulated
/// as part of that prefab. Instantiating a template as well would draw the same emitter tree twice, and some
/// templates cannot emit alone: the ice template's mesh shape has no mesh, and each site copy assigns its own.
/// A `fx_env_site_` prefab placed in a global package is not read either.
fn source_environment_role(effect_name: &str, variant: &str, phenomenon: &str) -> Option<EffectKind> {
    let site = variant.strip_prefix("unique__");
    if variant != "global" && site.is_none() {
        return None;
    }
    if effect_name.strip_prefix("fx_env_sky_") == Some(phenomenon) {
        return Some(EffectKind::Sky);
    }
    if effect_name.strip_prefix("fx_env_camera_") == Some(phenomenon) {
        return Some(EffectKind::Camera);
    }
    let site = site?;
    (effect_name == format!("fx_env_site_{phenomenon}_{site}")).then_some(EffectKind::Site)
}

/// The loader's three prefabs for one phenomenon and site, in the order sky, camera, unique; an absent one is
/// skipped the way the loader leaves its field null (see `source_environment_role`). The sky and camera prefabs
/// come from the site's unique package when it holds them, otherwise from the global package; the unique prefab
/// only from the unique package. An entry of the right name whose package variant is not a string is refused:
/// the extraction always records the package, so reading such an entry as absent would hide a malformed file.
fn source_environment_selection<'a>(
    effects: &'a serde_json::Map<String, Value>,
    phenomenon: &str,
    site: &str,
) -> Vec<(String, &'a Value, EffectKind)> {
    let unique = format!("unique__{site}");
    let held = |name: &str, variant: &str| -> Option<&'a Value> {
        let effect = effects.get(name)?;
        let package = effect_variant_of(effect)
            .unwrap_or_else(|| panic!("weather effect {name}: package variant is not a string"));
        (package == variant).then_some(effect)
    };
    let mut selected = Vec::new();
    for (kind, name) in [
        (EffectKind::Sky, format!("fx_env_sky_{phenomenon}")),
        (EffectKind::Camera, format!("fx_env_camera_{phenomenon}")),
    ] {
        if let Some(effect) = held(&name, &unique).or_else(|| held(&name, "global")) {
            selected.push((name, effect, kind));
        }
    }
    let name = format!("fx_env_site_{phenomenon}_{site}");
    if let Some(effect) = held(&name, &unique) {
        selected.push((name, effect, EffectKind::Site));
    }
    selected
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

/// A target of an unimplemented sub-emitter graph must not become an
/// autonomous emitter just because its own local modules happen to parse.
fn source_sub_emitter_owners(particles: &[Value]) -> HashMap<String, Vec<String>> {
    let mut owners = HashMap::<String, Vec<String>>::new();
    for particle in particles {
        let Some(owner) = particle.get("node").and_then(Value::as_str) else { continue; };
        let Some(entries) = particle.get("system").and_then(|s| s.get("subEmitters")).and_then(Value::as_array) else { continue; };
        for entry in entries {
            if let Some(target) = entry.get("emitter").and_then(Value::as_str) {
                owners.entry(target.to_owned()).or_default().push(owner.to_owned());
            }
        }
    }
    for value in owners.values_mut() { value.sort(); value.dedup(); }
    owners
}

/// An emitter at or below a Transform the effect's accepted Animator rotates
/// follows that rotation through its authored node chain every frame. Only an
/// emitter that composes the chain itself and simulates in Local space is
/// covered: World-space births, inherited emitter velocity and shape placement
/// against a moving Transform were not part of the evaluation that was read.
fn admit_animated(animation: &crate::weather_animation::Contract, mut planned: Planned, tally: &mut Tally)
    -> Option<Planned> {
    let animated = animation.animated_ancestors(&planned.node);
    if animated.is_empty() {
        return Some(planned);
    }
    let composed = planned.node_chain.as_ref().is_some_and(|chain|
        animated.iter().all(|path| chain.links.iter().any(|link| link.path == *path)));
    let reason = if !composed {
        "emitter below an animated Transform does not compose its authored node chain"
    } else if planned.emitter.simulation_space != SimulationSpace::Local {
        "emitter below an animated Transform does not simulate in Local space"
    } else {
        planned.animated = true;
        return Some(planned);
    };
    tally.animation_refused.push(format!("{}/{}: {reason}", planned.effect, planned.node));
    None
}

/// 逐条判读。放行回 Some，挡下回 None 并在盘点里具名。
fn judge(
    effect_name: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    sub_emitter_owners: &HashMap<String, Vec<String>>,
    kind: EffectKind,
    camera_rotation: bool,
    lifecycle: WeatherEffectLifecycle,
    server: &AssetServer,
    tally: &mut Tally,
) -> Option<Planned> {
    judge_in_archive(effect_name, particle, by_path, sub_emitter_owners, kind,
        camera_rotation, Some(lifecycle), "phenomena", None, server, tally)
}

fn judge_in_archive(
    effect_name: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    sub_emitter_owners: &HashMap<String, Vec<String>>,
    kind: EffectKind,
    camera_rotation: bool,
    lifecycle: Option<WeatherEffectLifecycle>,
    asset_root: &str,
    instance_anchor: Option<GlobalTransform>,
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
    if let Some(owners) = particle.get("node").and_then(Value::as_str).and_then(|node| sub_emitter_owners.get(node)) {
        tally.law_reject.push(format!("source sub-emitter event ownership is not installed: {}", owners.join(", ")));
        return None;
    }
    let render_mode = renderer.get("renderMode").and_then(Value::as_str).unwrap_or("");
    if !matches!(render_mode, "Billboard" | "HorizontalBillboard" | "Mesh") {
        tally.render_mode.push(format!("unsupported source render mode {render_mode}"));
        return None;
    }
    let alignment_id = renderer.get("alignment").and_then(Value::as_i64).unwrap_or(-1);
    let mesh_alignment = crate::particle_geometry::Alignment::from_source(alignment_id);
    if mesh_alignment.is_none() || (render_mode == "Billboard" && alignment_id == 4) {
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
        // ParticleSystemRenderer caches, in slot order, its populated slots
        // whose mesh is drawable, so a single populated slot draws the same mesh
        // whichever slot index holds it. The resolved list above names only the
        // slots the export could resolve; a populated slot that failed to
        // resolve still takes part in the cache and in the per-particle mesh
        // selection, so the serialized slot table must hold exactly one
        // populated reference, the resolved one.
        let populated = renderer.get("meshSlots").and_then(Value::as_array).and_then(|slots| {
            slots.iter().map(|slot| {
                let index = slot.get("slot")?.as_u64()?;
                let path_id = slot.get("reference")?.get("pathId")?.as_str()?.parse::<i64>().ok()?;
                Some((index, path_id != 0))
            }).collect::<Option<Vec<_>>>()
        });
        let Some(populated) = populated else {
            tally.render_mode.push("source Mesh renderer slot table is missing or malformed".into()); return None;
        };
        let populated: Vec<u64> = populated.into_iter().filter_map(|(index, used)| used.then_some(index)).collect();
        if populated != [u64::from(reference.mesh_slot)] {
            tally.render_mode.push("source Mesh requires exactly one populated mesh slot, the resolved one; multi-mesh selection not yet consumed".into());
            return None;
        }
        // Mesh flipping is consumed by the mesh transform.
        let flip = renderer.get("flip").and_then(Value::as_array)
            .and_then(|v| v.iter().map(|x| x.as_f64().filter(|n| n.is_finite()).map(|n| n as f32)).collect::<Option<Vec<_>>>())
            .and_then(|v| <[f32; 3]>::try_from(v).ok());
        let Some(flip) = flip else {
            tally.render_mode.push("source Mesh renderer flip is not three finite numbers".into()); return None;
        };
        // Whether a particle mirrors follows its own seed only for a proportion
        // strictly between zero and one: at or below zero none mirrors, above one
        // all do, and at exactly one all but the particle whose draw is exactly
        // one. Only the native birth path carries the engine's particle seed; a
        // system on the legacy step path draws its particle seeds itself, so at
        // exactly one the rare particle that keeps its orientation (one draw
        // value in 2^23 per axis) is a different particle than in the source.
        if flip.iter().any(|p| *p > 0.0 && *p < 1.0) {
            tally.render_mode.push("source Mesh flip proportion between zero and one needs the native particle seed".into()); return None;
        }
        Some((reference, Vec3::from_array(flip)))
    } else { None };

    let system = match particle.get("system").filter(|v| v.is_object()) {
        Some(system) => system,
        None => {
            tally.no_system_block += 1;
            return None;
        }
    };
    if let Err(error) = source_simulation_admission(system) {
        tally.law_reject.push(error); return None;
    }
    // 发射率形状：只按距离发射（本链不跟发射器位移）与死发射（率恒 0 且
    // 无 burst）都挡。
    let emission = match system.get("emission").filter(|v| v.as_object().is_some_and(|o| !o.is_empty())) {
        Some(emission) => emission,
        None => {
            // Only a weather effect played by the environment effect component
            // has had its owners enumerated; a fixture system can be driven by
            // fixture views and timelines, so it keeps the refusal.
            if lifecycle.is_some() && source_emission_disabled(system) {
                tally.source_emission_disabled += 1;
            } else {
                tally.no_emission += 1;
            }
            return None;
        }
    };
    let rate_time = raw_const(emission.get("rateOverTime"));
    let rate_distance = raw_const(emission.get("rateOverDistance"));
    let time_zero = rate_time.map_or(true, |v| v == 0.0);
    let distance_zero = rate_distance == Some(0.0);
    if !distance_zero {
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
        if !matches!(shape_type, "Circle" | "Cone" | "ConeVolume" | "Sphere" | "Hemisphere" | "SingleSidedEdge" | "Donut" | "Mesh") {
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
    // The source vertex writer consumes all three authored rotation axes.
    let start = system.get("start").cloned().unwrap_or(Value::Null);

    if start.get("rotation3D").and_then(Value::as_bool) == Some(true)
        && ["rotationX", "rotationY"].iter().any(|key| !start.get(*key).is_some_and(Value::is_object)) {
        tally.start_rotation_3d += 1;
        return None;
    }

    let source = match SourceParticle::load_from(renderer, server, asset_root) {
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
    // The Start* samplers read so far take an ApplyTexture step for each
    // birth group when ShapeModule holds a texture. That step is not
    // transcribed, so a texture is refused for every shape type and either
    // birth path; an export without the texture field leaves it undecided.
    if let Some(shape) = &emitter.shape {
        match &shape.controls.texture {
            Some(ShapeTexture::None) => {}
            None => {
                tally.shape.push(format!("{shape_type}: missing source shape texture reference; re-extract"));
                return None;
            }
            Some(ShapeTexture::Reference { .. }) => {
                tally.shape.push(format!("{shape_type}: source shape texture (ApplyTexture) consumer pending"));
                return None;
            }
        }
    }
    if let Some(sheet) = &emitter.texture_sheet {
        if let Err(error) = moly_law::particle::texture_sheet::TextureSheet::from_params(sheet) {
            tally.law_reject.push(format!("{node}: {error}")); return None;
        }
    }
    if let Some(noise) = &emitter.noise {
        if let Err(error) = NoiseLaw::from_params(noise) {
            tally.law_reject.push(format!("{node}: {error}")); return None;
        }
    }
    let route = crate::particle_runtime::source_route(system);
    if emitter.noise.is_some() {
        // Noise reads the system owner seed and the reset scroll, which only
        // the native birth owner supplies.
        if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route) {
            tally.law_reject.push(format!("{node}: Noise requires the native birth path: {reason}")); return None;
        }
    }
    if let Some(force) = &emitter.force {
        if let Err(error) = moly_law::particle::force::ForceOverLifetime::from_params(force) {
            tally.law_reject.push(format!("{node}: {error}")); return None;
        }
    }
    if const_of(&emitter.start_delay) != Some(0.0) {
        // Current source includes nonzero delays. Their emission clock and
        // existing-particle simulation are separate native paths, still pending.
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

    // Renderer-owned size limits, pivot, camera roll and vertex attributes are
    // mandatory source inputs. No visual minimum is substituted for zero size.
    let max_particle_size = renderer.get("maxParticleSize").and_then(Value::as_f64);
    let min_particle_size = renderer.get("minParticleSize").and_then(Value::as_f64);
    let allow_roll = renderer.get("allowRoll").and_then(Value::as_bool);
    if renderer.get("normalDirection").and_then(Value::as_f64) != Some(1.0) {
        tally.render_mode.push("source billboard normalDirection other than one is not yet verified".into()); return None;
    }
    if mesh_reference.is_none() && renderer.get("flip").and_then(Value::as_array)
        .is_none_or(|v| v.len()!=3 || v.iter().any(|x| x.as_f64()!=Some(0.0))) {
        tally.render_mode.push("source billboard particle flip is not yet consumed".into()); return None;
    }
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
    let (Some(max_particle_size), Some(min_particle_size), Some(allow_roll), Some(pivot)) = (max_particle_size, min_particle_size, allow_roll, pivot) else {
        tally.clamp_missing += 1;
        return None;
    };
    if !max_particle_size.is_finite() || !min_particle_size.is_finite()
        || min_particle_size < 0.0 || max_particle_size < min_particle_size
        || max_particle_size > f32::MAX as f64 || pivot.iter().any(|v| !v.is_finite()) {
        tally.clamp_missing += 1; return None;
    }
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
    let node_chain = if instance_anchor.is_none() { authored_chain(by_path, node) } else { None };
    let Some(node_affine) = instance_anchor.or_else(|| node_chain.as_ref().map(NodeChain::serialized_affine)) else {
        tally.node_unresolved += 1;
        return None;
    };

    let emission_surface = if shape_type == "Mesh" {
        let contract = match moly_assets::particle_source::ParticleMeshEmission::from_shape(shape.expect("Mesh shape present")) {
            Ok(contract) => contract,
            Err(error) => { tally.shape.push(error); return None; }
        };
        // Native mesh normals are barycentrically interpolated. Until the
        // velocity normalization branch is independently observed, admit only
        // stationary births (the surface is still sampled, never the origin).
        if const_of(&emitter.start.speed) != Some(0.0) {
            tally.shape.push("source mesh start-velocity normalization needs independent verification".into()); return None;
        }
        let glb = server.load(AssetPath::from_path_buf(std::path::PathBuf::from(format!("{asset_root}/{}", contract.mesh.file))).with_source("moly"));
        Some(PlannedSurface { reference: contract.mesh, glb, source: None })
    } else { None };
    let scaling = match source_scaling(system, by_path, node, instance_anchor.is_none()) {
        Ok(scaling) => scaling,
        Err(reason) => { tally.render_mode.push(reason); return None; }
    };
    if emitter.noise.is_some() {
        // Noise runs only with the native birth owner; the emitter state the
        // native Shape boundary reads must qualify too, or Noise would drop.
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: Noise requires the native birth path: {reason}")); return None;
        }
    }
    // The mesh geometry has no transform about the particle's axis of
    // rotation, whichever birth path feeds it.
    if mesh_reference.is_some() {
        let initial_enabled = system.pointer("/sourceModules/enabled").and_then(Value::as_array)
            .is_some_and(|modules| modules.iter().any(|module| module.as_str() == Some("InitialModule")));
        if let Err(refused) = crate::particle_runtime::mesh_rotation_admission(&emitter, initial_enabled) {
            tally.render_mode.push(refused.reason().into());
            return None;
        }
    }
    // A system that runs the legacy step must carry a start colour that step
    // evaluates as the source does. The weather host installs the native
    // birth owner where `native_birth_path` allows it; the fixture host (no
    // lifecycle) never installs it.
    let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
    if lifecycle.is_none() || crate::particle_runtime::native_birth_path(&emitter, &route, Some(evidence)).is_err() {
        if let Err(refused) = crate::particle_runtime::legacy_start_colour(&emitter.start.color) {
            tally.law_reject.push(format!("{node}: {}", refused.reason()));
            return None;
        }
        if let Err(reason) = crate::particle_runtime::legacy_bursts(&emitter) {
            tally.law_reject.push(format!("{node}: {reason}"));
            return None;
        }
    }
    Some(Planned {
        ordinal: 0,
        emission_surface,
        node: node.to_owned(),
        effect: effect_name.to_owned(),
        lifecycle,
        emitter,
        route,
        kind,
        camera_rotation,
        node_affine,
        node_chain,
        animated: false,
        source,
        draw: None,
        geometry: if let Some((reference, flip)) = mesh_reference {
            let glb = server.load(AssetPath::from_path_buf(std::path::PathBuf::from(format!("{asset_root}/{}", reference.file))).with_source("moly"));
            PlannedGeometry::Mesh { reference, glb, alignment: mesh_alignment.expect("validated Mesh alignment"), source: None, scaling, pivot: Vec3::from_array(pivot), flip }
        } else {
            PlannedGeometry::Billboard(crate::source_billboard::Draw {
                mode: if render_mode == "HorizontalBillboard" { crate::source_billboard::Mode::Horizontal } else { crate::source_billboard::Mode::Billboard },
                alignment: mesh_alignment.expect("validated source Billboard alignment"),
                screen_size: Vec2::new(min_particle_size as f32, max_particle_size as f32),
                allow_roll, scaling, pivot: Vec3::from_array(pivot),
            })
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

/// One link of an authored node chain: its path and its local TRS in the
/// product frame.
#[derive(Clone)]
struct ChainLink {
    path: String,
    local: Transform,
}

/// The authored Transform chain from the effect root down to an emitter node,
/// parent first.
#[derive(Clone)]
struct NodeChain {
    links: Vec<ChainLink>,
}

impl NodeChain {
    /// 父∘子 composition of the chain, with the local rotation of every link for
    /// which `rotation` returns a stored Unity rotation (x, y, z, w) replaced by it,
    /// converted exactly as a serialized rotation is.
    fn affine(&self, rotation: impl Fn(&str) -> Option<[f32; 4]>) -> GlobalTransform {
        let mut affine = GlobalTransform::IDENTITY;
        for link in &self.links {
            let mut local = link.local;
            if let Some(quat) = rotation(&link.path) {
                local.rotation = source_rotation(quat);
            }
            affine = affine * GlobalTransform::from(local);
        }
        affine
    }

    fn serialized_affine(&self) -> GlobalTransform {
        self.affine(|_| None)
    }
}

fn source_rotation(quat: [f32; 4]) -> Quat {
    Quat::from_xyzw(quat[0], -quat[1], -quat[2], quat[3])
}

/// 根记录 → 发射节点链（父先）。记录缺席或 TRS 形状不对回 None（调用方按节点
/// 未解析挡下）。
fn authored_chain(by_path: &HashMap<String, &Value>, path: &str) -> Option<NodeChain> {
    let mut chain: Vec<(String, &Value)> = Vec::new();
    let mut current = path.to_owned();
    while let Some(node) = by_path.get(&current) {
        chain.push((current.clone(), *node));
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
    let mut links = Vec::with_capacity(chain.len());
    for (path, node) in chain.into_iter().rev() {
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
            rotation: source_rotation(quat),
            scale: Vec3::from_array(scale),
        };
        links.push(ChainLink { path, local });
    }
    Some(NodeChain { links })
}

/// Authored MainModule scaling mode of one emitter. Local keeps this node's
/// own scale for the renderer and whether the node chain carries unit scale
/// throughout; an instance anchor replaces the authored chain, so it gives no
/// such evidence.
pub(crate) fn source_scaling(system: &Value, by_path: &HashMap<String, &Value>, node: &str,
    authored_chain: bool) -> Result<crate::particle_geometry::Scaling, String> {
    match system.get("scalingMode").and_then(Value::as_u64) {
        Some(0) => Ok(crate::particle_geometry::Scaling::Hierarchy),
        Some(1) => {
            let values = by_path.get(node).and_then(|n| n.get("scale")).and_then(Value::as_array);
            let Some(values) = values.filter(|v| v.len() == 3 && v.iter().all(|x| x.as_f64().is_some_and(|n| n.is_finite() && n.abs() <= f32::MAX as f64))) else {
                return Err("Local particle scale lacks its authored emitter transform".into());
            };
            Ok(crate::particle_geometry::Scaling::Local {
                scale: Vec3::new(values[0].as_f64().unwrap() as f32,
                    values[1].as_f64().unwrap() as f32, values[2].as_f64().unwrap() as f32),
                unit_chain: authored_chain && unit_scale_chain(by_path, node),
            })
        }
        value => Err(format!("unconsumed source particle scalingMode {value:?}")),
    }
}

/// Whether the node and every ancestor on the chain `authored_chain` walks
/// carry scale exactly one.
fn unit_scale_chain(by_path: &HashMap<String, &Value>, path: &str) -> bool {
    let mut current = path.to_owned();
    while let Some(node) = by_path.get(&current) {
        if triple(node.get("scale")) != Some([1.0; 3]) {
            return false;
        }
        let parent = node.get("parent").and_then(Value::as_str).unwrap_or("").to_owned();
        if current == parent {
            break;
        }
        current = parent;
    }
    true
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
    mut seed_manager: ResMut<crate::particle_runtime::seed::SystemSeedManager>,
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
        if let Some(surface) = &mut planned.emission_surface {
            match (server.load_state(&surface.glb), server.recursive_dependency_load_state(&surface.glb)) {
                (LoadState::Failed(error), _) => panic!("source emission surface {} failed: {error:?}", surface.reference.file),
                (_, RecursiveDependencyLoadState::Failed(error)) => panic!("source emission surface dependency {} failed: {error:?}", surface.reference.file),
                _ => {},
            }
            if !server.is_loaded_with_dependencies(&surface.glb) { return; }
            if surface.source.is_none() {
                let Some(asset) = gltfs.get(&surface.glb) else { return; };
                let node_handle = asset.named_nodes.get(surface.reference.node.as_str())
                    .unwrap_or_else(|| panic!("source emission node {} missing", surface.reference.node));
                let Some(node) = gltf_nodes.get(node_handle) else { return; };
                let mesh_handle = node.mesh.as_ref().expect("source emission node must own geometry");
                let Some(asset_mesh) = gltf_meshes.get(mesh_handle) else { return; };
                let Some(primitives) = asset_mesh.primitives.iter().map(|p| meshes.get(&p.mesh)).collect::<Option<Vec<_>>>() else { return; };
                let source_mesh = crate::particle_geometry::SourceMesh::from_primitives(&primitives, Vec3::from_array(surface.reference.size()))
                    .unwrap_or_else(|error| panic!("source emission mesh {} invalid: {error}", surface.reference.file));
                surface.source = Some(Arc::new(crate::particle_mesh_emission::EmissionSurface::from_source(&source_mesh)
                    .unwrap_or_else(|error| panic!("source emission surface {} invalid: {error}", surface.reference.file))));
            }
        }
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
    // Prepare the fallible entropy service before retiring the previous scene
    // or publishing readiness. This does not draw any system seed. A transient
    // failure retains the plan and existing instances for the next attempt.
    // Every admitted system resets its seed at first Play, native or legacy.
    for planned in &plan.planned {
        if planned.emitter.random_seed.is_none() || planned.emitter.auto_random_seed.is_none() {
            error!(node=%planned.node, "weather source seed ownership is unknown");
            return;
        }
        if planned.emitter.auto_random_seed == Some(true) {
            if let Err(error) = seed_manager.try_init() {
                error!(%error, "weather seed entropy preparation failed");
                return;
            }
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
    // Clocks belong to instantiated effects. They are shared by their emitters,
    // preserved with unchanged global instances, and move intact into retirement.
    let mut effect_clocks: HashMap<String, Arc<crate::weather_animation::EffectClock>> = HashMap::new();
    let mut effect_animators: HashMap<String, Option<Arc<crate::weather_animation::EffectAnimator>>> = HashMap::new();
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
        let sort_mode = source.sort_mode;
        source.enabled = true;
        commands.entity(draw).remove::<WeatherFxPreflight>().insert((source, WeatherFxDraw));
        let effect_clock = effect_clocks.entry(planned.effect.clone())
            .or_insert_with(|| Arc::new(crate::weather_animation::EffectClock::new(now))).clone();
        let effect_animator = effect_animators.entry(planned.effect.clone())
            .or_insert_with(|| plan.animators.get(&planned.effect)
                .map(|nodes| Arc::new(crate::weather_animation::EffectAnimator::new(nodes.clone())))).clone();
        let animated_chain = if planned.animated { planned.node_chain } else { None };
        let route = planned.route.clone();
        state.live.push(LiveWeatherEmitter { draw, native_refusal: None, lifecycle: planned.lifecycle.expect("weather plans own a source lifecycle"), effect_clock,
            effect_animator, animated_chain, runtime: Runtime {
            node: planned.node.clone(),
            effect: planned.effect.clone(),
            emitter: planned.emitter.clone(),
            kind: planned.kind,
            camera_rotation: planned.camera_rotation,
            node_affine: planned.node_affine,
            mesh,
            anchor: None,
            geometry: planned.geometry.into_runtime(),
            emission_surface: planned.emission_surface.map(|surface| surface.source.expect("source surface verified before installation")),
            ring_cursor: 0,
            pool: Vec::new(),
            side: Vec::new(),
            emission: EmissionState::default(),
            playback_head: 0.0,
            previous_head: 0.0,
            emission_started: false,
            native_birth: None,
            noise: None,
            // 逐系统换一条流：同一个种子在所有系统上会画出同一个图形。
            rng: Rng(RNG_SEED ^ (planned.ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            prewarmed: false,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            velocity_law: planned.emitter.velocity_over_lifetime.as_ref()
                .map(moly_law::particle::velocity::VelocityOverLifetime::from_params),
            force_law: planned.emitter.force.as_ref().map(|p|
                moly_law::particle::force::ForceOverLifetime::from_params(p).expect("force validated during admission")),
            gravity_law: moly_law::particle::gravity::Gravity::new(&planned.emitter.start.gravity_modifier),
            size_law: planned.emitter.size_over_lifetime.as_ref()
                .map(moly_law::particle::size::SizeOverLifetime::from_params),
            color_law: planned.emitter.color_over_lifetime.as_ref()
                .map(moly_law::particle::color::ColorOverLifetime::from_params),
            custom_law: planned.emitter.custom_data.as_ref()
                .map(moly_law::particle::custom_data::CustomData::from_params),
            texture_sheet: planned.emitter.texture_sheet.as_ref().map(|p|
                moly_law::particle::texture_sheet::TextureSheet::from_params(p).expect("sheet validated during admission")),
            sort_mode,
            born_total: 0,
            died_total: 0,
            full_total: 0,
            refused_total: 0,
        }});
        let mut failed = false;
        {
            let live = state.live.last_mut().expect("just installed source instance");
            match crate::particle_runtime::install_native_birth(&mut live.runtime, &mut seed_manager, &route) {
                Ok(crate::particle_runtime::BirthPath::Native) => {
                    info!(node=%live.node, noise=live.noise.is_some(), "weather native birth owner installed");
                }
                Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => live.native_refusal = Some(reason),
                Err(error) => {
                    error!(%error, node=%live.node, "weather source seed owner unavailable");
                    failed = true;
                }
            }
        }
        if failed {
            let failed = state.live.pop().expect("just installed source instance");
            commands.entity(failed.draw).try_despawn();
            continue;
        }
    }
    plan.planned = waiting;
    state.admitted = state.live.len();
    if install_site || install_global {
        info!("[weather-fx] {} @ {} phase site={} global={} active={} retiring={}",state.tier,state.env_site,install_site,install_global,state.live.len(),retiring.live.len());
    }
    if plan.site_installed && plan.global_installed { state.selection = Some(plan.selection.clone()); }
    if create_state { commands.insert_resource(created); }
    // The plan also owns the commit notice: an unchanged global effect is "installed" at
    // the fade's start, and the site effect may install while the fade still runs, so the
    // plan stays until the fade has ended and the notice above was published.
    if plan.site_installed && plan.global_installed && phase.can_commit_global_fx(&plan.selection) {
        commands.remove_resource::<WeatherFxPlan>();
    }
}

/// `SiteEnvironmentViewController.RefreshEffectVisible`: on an indoor site the
/// source deactivates the GameObjects of the global sky effect, of the current
/// site view's unique effect and of the field camera's effect, and activates
/// them again on an outdoor one. The instances stay installed (an unchanged sky
/// is kept across the move); an inactive GameObject neither draws nor updates
/// its particle systems, so [`advance`] skips the live emitters as well.
/// Retiring emitters are no longer referenced by those three owners and keep
/// their own lifecycle.
pub(crate) fn refresh_effect_visible(
    state: Option<Res<WeatherFxState>>,
    site: Option<Res<SiteActive>>,
    mut draws: Query<&mut SourceParticle, With<WeatherFxDraw>>,
) {
    let Some(state) = state else { return; };
    let shown = !site.as_deref().is_some_and(SiteActive::is_indoor);
    for live in &state.live {
        if let Ok(mut particle) = draws.get_mut(live.draw) {
            if particle.enabled != shown { particle.enabled = shown; }
        }
    }
}

/// PostUpdate（变换传播之后）：推进仿真并重建属性池。
///
/// 排在传播之后是因为**局部空间仿真**要读锚点的当帧世界变换；排在相机
/// 之后是因为四角展开要读当帧机位。
pub(crate) fn advance(
    mut commands: Commands,
    mut state: Option<ResMut<WeatherFxState>>,
    mut retiring: ResMut<WeatherFxRetirements>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    frame: Res<bevy::diagnostic::FrameCount>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    avatars: Query<&GlobalTransform, With<AvatarRoot>>,
    site: Option<Res<SiteActive>>,
) {
    // Observe instance age even when no camera can produce a particle draw.
    // The effect Animators evaluate here too, once per frame with the frame's
    // delta time: before the frame's particle update, and whether or not a
    // camera can draw (culling mode "always animate").
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    for live in state.as_deref().into_iter().flat_map(|state| &state.live)
        .chain(retiring.live.iter().map(|entry| &entry.emitter)) {
        live.effect_clock.observe(now);
        if let Some(animator) = &live.effect_animator {
            animator.advance_frame(frame.0, dt);
        }
    }
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

    // Indoors the three effect owners are inactive (see refresh_effect_visible).
    let live_active = !site.as_deref().is_some_and(SiteActive::is_indoor);
    let active = state.as_deref_mut().into_iter().filter(|_| live_active).flat_map(|s| s.live.iter_mut())
        .map(|s| (s, true));
    let retired = retiring.live.iter_mut().map(|s| (&mut s.emitter, false));
    // Systems whose native step was refused this frame; see step_frame.
    let mut refused = Vec::new();
    for (live, emitting) in active.chain(retired) {
        let LiveWeatherEmitter { draw, runtime: system, effect_animator, animated_chain, .. } = live;
        let draw = *draw;
        // 惰性 prewarm：首个推进帧快进原版首次 Play 的暖机窗口（只动仿真
        // 状态不喂渲染，快进里的世界空间出生锚在首帧锚点上）。窗口长度与
        // 起始时钟取原生 Compute/Update1b 的值；Play 只在 prewarm 且循环时
        // 暖机（非循环 prewarm 原生不暖机，不是缺口）。
        if emitting && !system.prewarmed {
            system.prewarmed = true;
            if system.emitter.prewarm && system.emitter.looping {
                if let Err(error) = crate::particle_runtime::prewarm_first_play(system, &ctx) {
                    error!(%error, effect=%system.effect, node=%system.node, "weather prewarm refused");
                }
            }
        }
        // The first Play warms inside the instantiating call, before any
        // Animator write, so the prewarm above saw the serialized chain. The
        // Animator's rotation of this frame lands before the particle update
        // and is what rendering reads.
        if let (Some(chain), Some(animator)) = (animated_chain.as_ref(), effect_animator.as_ref()) {
            system.node_affine = chain.affine(|path| animator.rotation(path));
        }
        let step = dt * system.emitter.simulation_speed;
        if step > 0.0 {
            if let Err(reason) = crate::particle_runtime::step_frame(system, step, &ctx, emitting) {
                error!(%reason, effect=%system.effect, node=%system.node,
                    "native particle step refused: the system is retired and draws nothing");
                refused.push(draw);
                continue;
            }
        }
        // 局部空间仿真：律状态是发射节点局部坐标，用锚∘链的当帧值换算成
        // 世界坐标。世界空间仿真的状态出生时就是世界坐标，恒等。
        let to_world = match system.emitter.simulation_space {
            SimulationSpace::World => GlobalTransform::IDENTITY,
            _ => compose_to_world(system, &ctx),
        };
        // The source-program renderer extracts Assets<Mesh> directly each frame
        // and no Bevy material draws this mesh, so a Modified event would only
        // make the mesh allocator re-upload a buffer nothing reads.
        let Some(mesh) = meshes.get_mut_untracked(&system.mesh) else {
            continue;
        };
        crate::particle_runtime::write_geometry(mesh, system, &to_world,
            &compose_to_world(system, &ctx), camera_transform, basis);
    }
    // A refused system is final: it leaves the active and retiring sets and
    // its draw is despawned, rather than running a clock without births.
    if !refused.is_empty() {
        if let Some(state) = state.as_deref_mut() {
            state.live.retain(|live| !refused.contains(&live.draw));
            state.admitted = state.live.len();
        }
        retiring.live.retain(|entry| !refused.contains(&entry.emitter.draw));
        for draw in refused {
            commands.entity(draw).try_despawn();
        }
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
