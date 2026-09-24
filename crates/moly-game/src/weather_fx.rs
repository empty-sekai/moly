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
            Self::Mesh { alignment, source, scaling, pivot, .. } => crate::particle_runtime::Geometry::Mesh(crate::particle_geometry::MeshDraw {
                source: source.expect("source mesh readiness must precede weather commit"), alignment, scaling, pivot,
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
    source: SourceParticle,
    draw: Option<(Entity, Handle<Mesh>)>,
    geometry: PlannedGeometry,
    emission_surface: Option<PlannedSurface>,
    lifecycle: Option<WeatherEffectLifecycle>,
    /// Real sub-emitter birth and death edges, each in slot order, resolved
    /// to its child and read into the law the parent's events use. Such a
    /// parent runs only on the native birth path.
    event_edges: Option<crate::particle_runtime::EventEdges>,
    /// The owner words a sub-emitter target's commands read (Local scaling,
    /// the authored chain on the site anchor); `Some` exactly for an admitted
    /// target, which is installed as its parent's child and never emits on
    /// its own.
    child_owner: Option<moly_law::particle::child_emit::ChildOwner>,
    /// The owner words a Local collision system's query and hits read (its
    /// authored chain on the site anchor); `None` for every other system.
    #[allow(dead_code)]
    collision_owner: Option<moly_law::particle::collision_query::OwnerPair>,
    /// The owner local-to-world words a Local system's trail job composes
    /// with the view (the same chain); `None` for every other system.
    trail_owner: Option<[f32; 16]>,
    /// Cone 的半顶角（shape 块的 `angle` 键；律的 `ShapeParams` 不带它）。
    cone_angle: Option<f32>,
    rol: Option<RotationOverLifetime>,
    limit: Option<LimitVelocity>,
    /// The trail draw of a system with a qualified TrailModule: the renderer's
    /// trail material and trail vertex streams, drawn after the particles.
    trail: Option<PlannedTrail>,
}

struct PlannedTrail {
    source: SourceParticle,
    draw: Option<(Entity, Handle<Mesh>)>,
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
    /// A distance rate that is not a constant, on a system that runs its own
    /// per-frame update (a sub-emitter target's distance rate is read by its
    /// parent's edge law).
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
                    "subEmitterEvents": s.native_birth.as_ref().and_then(|birth| birth.events.as_ref()).map(|events| serde_json::json!({
                        "targets": events.slots().iter().map(|slot| slot.edge.target.clone()).collect::<Vec<_>>(),
                        "childOwner": events.slots().iter().map(|slot| if slot.delivered {
                            "delivered to the installed target"
                        } else {
                            "refused: commands are counted, none is applied"
                        }).collect::<Vec<_>>(),
                        "records": events.slots().iter().map(|slot| slot.tally.records).collect::<Vec<_>>(),
                        "commands": events.slots().iter().map(|slot| slot.tally.commands).collect::<Vec<_>>(),
                        "childBirths": events.slots().iter().map(|slot| slot.tally.births).collect::<Vec<_>>(),
                        "deathTargets": events.death_slots().iter().map(|slot| slot.edge.target.clone()).collect::<Vec<_>>(),
                        "deathChildOwner": events.death_slots().iter().map(|slot| if slot.delivered {
                            "delivered to the installed target"
                        } else {
                            "refused: commands are counted, none is applied"
                        }).collect::<Vec<_>>(),
                        "deathRecords": events.death_slots().iter().map(|slot| slot.tally.records).collect::<Vec<_>>(),
                        "deathCommands": events.death_slots().iter().map(|slot| slot.tally.commands).collect::<Vec<_>>(),
                        "deathChildBirths": events.death_slots().iter().map(|slot| slot.tally.births).collect::<Vec<_>>(),
                        "broken": events.broken,
                    })),
                    "subEmitterTarget": s.native_birth.as_ref().and_then(|birth| birth.target.as_ref()).map(|target| serde_json::json!({
                        "commands": target.commands,
                        "births": target.births,
                        "refusedCommands": target.refused,
                        "lastRefusal": target.last_refusal,
                    })),
                    "trail": s.trail.as_ref().map(|trail| serde_json::json!({
                        "clock": trail.clock.time,
                        "rings": trail.rings.len(),
                        "points": trail.points(),
                        "vertices": trail.vertices,
                        "drawRefusal": trail.draw_refusal.as_ref().map(|refused| format!("{refused:?}")),
                        "drawEntity": s.trail_draw.as_ref().map(|(entity, _)| format!("{entity:?}")),
                    })),
                    "noiseConsumer": s.noise.as_ref().map(|noise| serde_json::json!({
                        "ownerSeed": noise.owner_seed,
                        "automaticSeed": noise.owner.automatic,
                        "scroll": noise.state.scroll,
                        "qualifiedLaw": true,
                    })),
                    "effectAge": s.effect_clock.age(),
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
    /// The trail draw and its mesh, when the system has a trail.
    trail_draw: Option<(Entity, Handle<Mesh>)>,
    lifecycle: WeatherEffectLifecycle,
    effect_clock: Arc<crate::weather_animation::EffectClock>,
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
        if let Some((trail, _)) = entry.emitter.trail_draw { commands.entity(trail).try_despawn(); }
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
        let animation = crate::weather_animation::Contract::compile(effect, animation_document.as_ref());
        info!("[weather-animation] {} {}", effect_name, animation.report);
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
        let sub_emitter_owners = source_sub_emitter_owners(particles);
        let effect_start = plans.len();
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
                    tally.admitted += 1;
                    plans.push(planned);
                }
                None => {}
            }
        }
        for node in drop_orphan_targets(&mut plans, effect_start) {
            tally.admitted -= 1;
            tally.law_reject.push(format!("sub-emitter target {node}: its parent is not admitted in this effect"));
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
    if !tally.animation_refused.is_empty() {
        warn!("[weather-animation] source playback refused: {:?}", tally.animation_refused);
    }
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

/// Module capability is checked from the complete serialized inventory, not
/// just whichever parameters an older producer happened to emit.
fn source_simulation_admission(system: &Value) -> Result<(), String> {
    // Preserve +Infinity at the schema boundary, but reject the unverified
    // scheduler before any other module capability can hide this gap.
    if system.get("emission").and_then(|v| v.get("bursts")).and_then(Value::as_array)
        .is_some_and(|bursts| bursts.iter().any(|burst|
            burst.get("repeatInterval").and_then(Value::as_str) == Some("Infinity")))
    {
        return Err("source infinite burst repeat interval scheduling is not yet verified".into());
    }
    let source = moly_assets::particle_source::ParticleSourceModules::from_system(system)?;
    for module in &source.enabled {
        // The current snow owner carries an authored null SubModule edge
        // (emitter=null, sourcePointer 0/0). It does not name a child system
        // and therefore adds no runtime scheduling obligation. Real edges are
        // taken when every edge is a birth, death or collision edge naming a
        // child in this file: the parent records their events (judge()
        // resolves the children and their laws; the CollisionModule records
        // the collision events). Other triggers, pointers into another
        // file, a list mixing null and real entries, or an unexported list
        // are refused here.
        if module == "SubModule" {
            let entries = system.get("subEmitters").and_then(Value::as_array).filter(|e| !e.is_empty());
            let null = |entry: &Value| entry.get("emitter") == Some(&Value::Null)
                && entry.pointer("/sourcePointer/fileId").and_then(Value::as_i64) == Some(0)
                && entry.pointer("/sourcePointer/pathId").and_then(Value::as_str) == Some("0");
            let real_event = |entry: &Value| entry.get("emitter").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
                && matches!(entry.get("type").and_then(Value::as_str), Some("birth" | "death" | "collision"))
                && entry.pointer("/sourcePointer/fileId").and_then(Value::as_i64) == Some(0)
                && entry.pointer("/sourcePointer/pathId").and_then(Value::as_str).is_some_and(|id| id != "0");
            match entries {
                Some(entries) if entries.iter().all(null) || entries.iter().all(real_event) => continue,
                Some(_) => return Err("enabled source module SubModule: only authored-null or real birth, death and collision edges into this file are consumed".into()),
                None => return Err("enabled source module SubModule lacks its authored subEmitters edges".into()),
            }
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
            // Trails likewise run only on the native birth path with their own
            // draw; judge() checks the qualified subset and the trail material.
            "TrailModule" => "trails",
            // Collision likewise; judge() refuses it with the reason the
            // native birth path gives (the module law, or the missing scene).
            "CollisionModule" => "collision",
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

/// The sub-emitter graph of one effect. Dereferences to the owners of every
/// edge target: a target must not become an autonomous emitter just because
/// its own local modules happen to parse, since its child owner is not
/// installed. It also keeps each system record by node, so a parent's birth
/// edges can be resolved to their children.
pub(crate) struct SubEmitterGraph<'a> {
    owners: HashMap<String, Vec<String>>,
    records: HashMap<&'a str, Vec<&'a Value>>,
}

impl std::ops::Deref for SubEmitterGraph<'_> {
    type Target = HashMap<String, Vec<String>>;
    fn deref(&self) -> &Self::Target {
        &self.owners
    }
}

impl<'a> SubEmitterGraph<'a> {
    /// The one system record at `node` whose system object id is `path_id`.
    fn child(&self, node: &str, path_id: &str) -> Option<&'a Value> {
        let mut found = self.records.get(node)?.iter().copied()
            .filter(|record| record.get("systemPathId").and_then(Value::as_str) == Some(path_id));
        let child = found.next()?;
        found.next().is_none().then_some(child)
    }
}

pub(crate) fn source_sub_emitter_owners(particles: &[Value]) -> SubEmitterGraph<'_> {
    let mut owners = HashMap::<String, Vec<String>>::new();
    let mut records = HashMap::<&str, Vec<&Value>>::new();
    for particle in particles {
        let Some(owner) = particle.get("node").and_then(Value::as_str) else { continue; };
        records.entry(owner).or_default().push(particle);
        let Some(entries) = particle.get("system").and_then(|s| s.get("subEmitters")).and_then(Value::as_array) else { continue; };
        for entry in entries {
            if let Some(target) = entry.get("emitter").and_then(Value::as_str) {
                owners.entry(target.to_owned()).or_default().push(owner.to_owned());
            }
        }
    }
    for value in owners.values_mut() { value.sort(); value.dedup(); }
    SubEmitterGraph { owners, records }
}

/// Resolve every edge of a parent with real sub-emitter edges to its child
/// (the record at the edge's node path whose system object id is the
/// pointer's, in this file). A birth edge reads the child's start delay,
/// duration, loop flag and Emission module into its count law; a death edge
/// reads the child's first burst into its event law. Every edge is a birth
/// or death edge here (the source admission refused the rest); all of them
/// are cached, the birth edges and the death edges each in authored order.
pub(crate) fn sub_emitter_edges(emitter: &EmitterParams, graph: &SubEmitterGraph<'_>)
    -> Result<crate::particle_runtime::EventEdges, String> {
    use moly_law::particle::schema::SubEmitterTrigger;
    let cached = emitter.sub_emitters.iter().filter(|edge| edge.trigger == SubEmitterTrigger::Birth).count();
    let mut edges = crate::particle_runtime::EventEdges::default();
    for edge in &emitter.sub_emitters {
        let target = edge.emitter.as_deref().filter(|name| !name.is_empty())
            .ok_or("edge without a named child")?;
        let moly_law::particle::schema::SubEmitterSourcePointer::Pointer { file_id: 0, path_id } = &edge.source_pointer else {
            return Err(format!("{target}: child pointer is not an object of this file"));
        };
        // Two edges to one child would feed it two command streams in an
        // order the export does not hold.
        if edges.targets().any(|known| known == target) {
            return Err(format!("{target}: two edges name one child"));
        }
        let record = graph.child(target, path_id)
            .ok_or_else(|| format!("{target}: child system {path_id} not resolved to one record"))?;
        let system = record.get("system").filter(|v| v.is_object())
            .ok_or_else(|| format!("{target}: child has no system block"))?;
        let modules = moly_assets::particle_source::ParticleSourceModules::from_system(system)
            .map_err(|error| format!("{target}: {error}"))?;
        if !modules.enabled.iter().any(|module| module == "EmissionModule") {
            return Err(format!("{target}: child EmissionModule disabled; its event emission is not transcribed"));
        }
        let archive = serde_json::json!({
            "effects": { "weather": { "particles": [{ "node": target, "system": system.clone() }] } }
        });
        let child = match Effects::from_json_str(archive.to_string().as_bytes()) {
            Ok(mut effects) if effects.emitters.len() == 1 => effects.emitters.remove(0),
            Ok(effects) => return Err(format!("{target}: child parse returned {} systems", effects.emitters.len())),
            Err(error) => return Err(format!("{target}: {error}")),
        };
        match edge.trigger {
            SubEmitterTrigger::Birth => {
                let emission = child.emission.as_ref().ok_or_else(|| format!("{target}: child has no Emission block"))?;
                let law = moly_law::particle::sub_emission::BirthEdgeLaw::from_params(
                    edge, cached, &child.start_delay, child.duration, child.looping, emission)
                    .map_err(|refused| format!("{target}: child emission outside the event law ({refused:?})"))?;
                edges.births.push(crate::particle_runtime::BirthEdge { target: target.to_owned(), law });
            }
            SubEmitterTrigger::Death => {
                let law = moly_law::particle::death_event::DeathEmitEdge::from_source(edge, &child)
                    .map_err(|refused| format!("{target}: death edge outside the death event law ({refused:?})"))?;
                edges.deaths.push(crate::particle_runtime::DeathEdge { target: target.to_owned(), law });
            }
            SubEmitterTrigger::Collision => {
                let law = moly_law::particle::collision_event::CollisionEmitEdge::from_source(edge, &child)
                    .map_err(|refused| format!("{target}: collision edge outside the RecordEmit law ({refused:?})"))?;
                edges.collisions.push(crate::particle_runtime::CollisionEdge { target: target.to_owned(), law });
            }
            trigger => return Err(format!("{target}: {trigger:?} edge events are not transcribed")),
        }
    }
    Ok(edges)
}

/// Removes, from the plans of one effect (`start..`), every sub-emitter
/// target no admitted plan of that effect has an edge to, and returns
/// their nodes. The target's own judgement asks whether its parent's record
/// is admitted; a parent refused outside judge (its animation contract) is
/// seen only here.
fn drop_orphan_targets(plans: &mut Vec<Planned>, start: usize) -> Vec<String> {
    let delivered: std::collections::HashSet<String> = plans[start..].iter()
        .flat_map(|plan| plan.event_edges.iter().flat_map(|edges| edges.targets().map(str::to_owned)))
        .collect();
    let mut dropped = Vec::new();
    let mut index = start;
    while index < plans.len() {
        if plans[index].child_owner.is_some() && !delivered.contains(&plans[index].node) {
            dropped.push(plans.remove(index).node);
        } else {
            index += 1;
        }
    }
    dropped
}

/// The gates of a sub-emitter target that read only the export: one parent
/// (with two, the order of their commands is not in the export), a parent
/// that is not itself a target, a site effect on its authored chain (the
/// owner words exist only there), the scaled clock, no warm and the Local
/// scaling mode (the only owner update transcribed). Returns the parent.
fn sub_emitter_target_gate(owners: &[String], particle: &Value, graph: &SubEmitterGraph<'_>, kind: EffectKind,
    instance_anchor: Option<GlobalTransform>) -> Result<String, String> {
    let [parent] = owners else {
        return Err(format!("{} parents ({}): the order of their commands is not in the export",
            owners.len(), owners.join(", ")));
    };
    if kind != EffectKind::Site || instance_anchor.is_some() {
        return Err("owner words are composed only for a site effect on its authored chain".into());
    }
    let system = particle.get("system");
    match system.and_then(|s| s.get("useUnscaledTime")).and_then(Value::as_bool) {
        Some(false) => {}
        Some(true) => return Err("useUnscaledTime clock not ported".into()),
        None => return Err("useUnscaledTime not exported".into()),
    }
    if system.and_then(|s| s.get("prewarm")).and_then(Value::as_bool) != Some(false) {
        return Err("warm of a sub-emitter target is not transcribed".into());
    }
    match system.and_then(|s| s.get("scalingMode")).and_then(Value::as_u64) {
        Some(1) => {}
        mode => return Err(format!("scaling mode {mode:?}: only the Local owner update is transcribed")),
    }
    Ok(parent.clone())
}

/// The target's owner words from its authored chain, root first (the chain
/// the node composition walks; the site anchor adds no element). A matrix
/// below the inverse's determinant threshold is refused: the engine keeps
/// using the zero inverse, which this runtime's own transforms cannot follow.
fn child_owner_words(by_path: &HashMap<String, &Value>, path: &str)
    -> Result<moly_law::particle::child_emit::ChildOwner, String> {
    Ok(moly_law::particle::child_emit::ChildOwner::local_scaling(&owner_matrices(by_path, path)?))
}

/// The owner matrices of the node's authored chain (Local scaling), refused
/// below the inverse's determinant threshold.
fn owner_matrices(by_path: &HashMap<String, &Value>, path: &str)
    -> Result<moly_law::particle::owner::OwnerMatrices, String> {
    use moly_law::particle::owner::{local_scaling_owner, SourceTrs};
    fn words<const N: usize>(node: &Value, key: &str) -> Option<[f32; N]> {
        let list = node.get(key)?.as_array().filter(|list| list.len() == N)?;
        let mut out = [0.0f32; N];
        for (slot, value) in out.iter_mut().zip(list) {
            *slot = value.as_f64()? as f32;
        }
        Some(out)
    }
    let mut chain: Vec<&Value> = Vec::new();
    let mut current = path.to_owned();
    while let Some(node) = by_path.get(&current) {
        chain.push(*node);
        let parent = node.get("parent").and_then(Value::as_str).unwrap_or("").to_owned();
        if current == parent {
            break;
        }
        current = parent;
    }
    let trs = chain.iter().rev()
        .map(|node| Some(SourceTrs { t: words(node, "position")?, q: words(node, "rotation")?, s: words(node, "scale")? }))
        .collect::<Option<Vec<_>>>()
        .ok_or("owner chain lacks an authored TRS")?;
    let owner = local_scaling_owner(&trs).map_err(|refused| format!("owner words {refused:?}"))?;
    if !owner.invert_ok {
        return Err("owner matrix below the inverse's determinant threshold".into());
    }
    Ok(owner)
}

/// Whether the target's parent is admitted with a birth or death edge to it:
/// the parent record is judged as its own turn judges it.
#[allow(clippy::too_many_arguments)]
fn parent_delivers(effect_name: &str, parent: &str, node: &str, by_path: &HashMap<String, &Value>,
    graph: &SubEmitterGraph<'_>, kind: EffectKind, camera_rotation: bool,
    lifecycle: Option<WeatherEffectLifecycle>, asset_root: &str, server: &AssetServer) -> Result<(), String> {
    let records: Vec<&Value> = graph.records.get(parent).into_iter().flatten().copied()
        .filter(|record| record.pointer("/system/subEmitters").and_then(Value::as_array)
            .is_some_and(|edges| edges.iter().any(|edge| edge.get("emitter").and_then(Value::as_str) == Some(node))))
        .collect();
    let [record] = records.as_slice() else {
        return Err(format!("parent {parent}: {} records name this target", records.len()));
    };
    let mut scratch = Tally::default();
    match judge_in_archive(effect_name, record, by_path, graph, kind, camera_rotation, lifecycle, asset_root,
        None, server, &mut scratch) {
        Some(planned) if planned.event_edges.as_ref().is_some_and(|edges| edges.targets().any(|target| target == node)) =>
            Ok(()),
        Some(_) => Err(format!("parent {parent} admitted without an event edge to this target")),
        None => Err(format!("parent {parent} not admitted ({})", scratch.law_reject.last()
            .cloned().unwrap_or_else(|| format!("{scratch:?}")))),
    }
}

/// 逐条判读。放行回 Some，挡下回 None 并在盘点里具名。
fn judge(
    effect_name: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    sub_emitter_owners: &SubEmitterGraph<'_>,
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
    sub_emitter_owners: &SubEmitterGraph<'_>,
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
    // A sub-emitter target is admitted only as the installed child of its
    // one admitted parent; the rest of its gates follow the law parse.
    let child_parent = match particle.get("node").and_then(Value::as_str).and_then(|node| sub_emitter_owners.get(node)) {
        None => None,
        Some(owners) => match sub_emitter_target_gate(owners, particle, sub_emitter_owners, kind, instance_anchor) {
            Ok(parent) => Some(parent),
            Err(reason) => { tally.law_reject.push(format!("sub-emitter target: {reason}")); return None; }
        },
    };
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
    if let Err(error) = source_simulation_admission(system) {
        tally.law_reject.push(error); return None;
    }
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
    let distance_zero = rate_distance == Some(0.0);
    // A sub-emitter target never emits on its own: its parent's edge law
    // reads its rate over distance (from the parent particle's motion). A
    // constant distance rate of a system that runs its own per-frame update
    // is taken by the native frame head (checked after the route below).
    if !distance_zero && child_parent.is_none() && rate_distance.is_none() {
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
    // A CollisionModule runs only on the native birth path with a scene of
    // the effect's ground collider. The module law (with its current-size
    // stream and its collision events) is checked here; the scene is refused
    // after every other gate, below.
    if emitter.collision.is_some() {
        if let Err(reason) = crate::particle_runtime::collision_eligible(&emitter)
            .and_then(|()| crate::particle_runtime::current_size_source_gate(system)) {
            tally.law_reject.push(format!("{node}: CollisionModule {reason}"));
            return None;
        }
    }
    let route = crate::particle_runtime::source_route(system);
    // Real sub-emitter birth and death edges: the parent records their
    // events on the native birth path and sends each command to the edge's
    // child, which applies it when it is an installed target of the same
    // effect instance and otherwise refuses it, counting.
    let event_edges = if crate::particle_runtime::has_real_sub_emitter_edges(&emitter) {
        match sub_emitter_edges(&emitter, sub_emitter_owners) {
            Ok(edges) => Some(edges),
            Err(reason) => { tally.law_reject.push(format!("{node}: SubModule {reason}")); return None; }
        }
    } else {
        None
    };
    if event_edges.is_some() {
        if emitter.prewarm && emitter.looping {
            tally.law_reject.push(format!(
                "{node}: SubModule parent with a first-Play warm: the warm's sub-emitter events are not transcribed"));
            return None;
        }
        // A target takes no route; its composition is judged below.
        if let Err(reason) = child_parent.as_ref().map_or_else(
            || crate::particle_runtime::native_birth_eligible(&emitter, &route), |_| Ok(())) {
            tally.law_reject.push(format!("{node}: sub-emitter events require the native birth path: {reason}"));
            return None;
        }
    }
    // Emission over distance runs at the native per-frame head only, from the
    // emitter translation the frame head reads. A target never takes that
    // call (its own frame is the stopped update).
    if !distance_zero && child_parent.is_none() {
        if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route) {
            tally.law_reject.push(format!("{node}: emission over distance requires the native birth path: {reason}"));
            return None;
        }
    }
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
    if renderer.get("flip").and_then(Value::as_array)
        .is_none_or(|v| v.len()!=3 || v.iter().any(|x| x.as_f64()!=Some(0.0))) {
        tally.render_mode.push("source particle flip stream permutation is not yet consumed".into()); return None;
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
    let Some(node_affine) = instance_anchor.or_else(|| compose_affine(by_path, node)) else {
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
    if event_edges.is_some() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: sub-emitter events require the native birth path: {reason}"));
            return None;
        }
    }
    if !distance_zero && child_parent.is_none() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: emission over distance requires the native birth path: {reason}"));
            return None;
        }
    }
    // A TrailModule runs only with the native birth owner (its two update
    // points are in the native slices) and draws with the renderer's trail
    // material; a system is never admitted without its trail.
    let trail = if emitter.trails.is_some() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
        if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route)
            .and_then(|()| crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)))
            .and_then(|()| crate::particle_runtime::trail_draw_eligible(&emitter, Some(evidence)).map_err(str::to_owned))
        {
            tally.law_reject.push(format!("{node}: TrailModule requires the native birth path: {reason}"));
            return None;
        }
        match trail_renderer(renderer).and_then(|trail| SourceParticle::load_from(&trail, server, asset_root)
            .map_err(|error| error.to_string())) {
            Ok(source) => Some(PlannedTrail { source, draw: None }),
            Err(reason) => { tally.law_reject.push(format!("{node}: trail draw: {reason}")); return None; }
        }
    } else {
        None
    };
    // The owner words of a Local system whose CollisionModule or TrailModule
    // reads them (the collision query and hits go through the local-to-world
    // and its inverse, the trail job composes the view with the
    // local-to-world): the engine's owner update of the authored chain, which
    // exists only for a site effect on that chain.
    let local_owner = match emitter.simulation_space {
        moly_law::particle::schema::SimulationSpace::Local if emitter.collision.is_some() || emitter.trails.is_some() => {
            if kind != EffectKind::Site || instance_anchor.is_some() {
                tally.law_reject.push(format!(
                    "{node}: owner words of a Local collision or trail are composed only for a site effect on its authored chain"));
                return None;
            }
            match owner_matrices(by_path, node) {
                Ok(owner) => Some(owner),
                Err(reason) => {
                    tally.law_reject.push(format!("{node}: owner words of a Local collision or trail: {reason}"));
                    return None;
                }
            }
        }
        _ => None,
    };
    let collision_owner = local_owner.filter(|_| emitter.collision.is_some()).map(|owner| {
        use moly_law::particle::collision_response::QueryAffine;
        moly_law::particle::collision_query::OwnerPair {
            local_to_world: QueryAffine::from_columns(&owner.local_to_world),
            world_to_local: QueryAffine::from_columns(&owner.world_to_local),
        }
    });
    let trail_owner = local_owner.filter(|_| emitter.trails.is_some()).map(|owner| owner.local_to_world);
    let child_owner = match &child_parent {
        None => None,
        Some(parent) => {
            let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: mesh_reference.is_some() };
            let owner = crate::particle_runtime::child_target_eligible(&emitter, Some(evidence))
                .and_then(|()| child_owner_words(by_path, node))
                .and_then(|owner| parent_delivers(effect_name, parent, node, by_path, sub_emitter_owners, kind,
                    camera_rotation, lifecycle, asset_root, server).map(|()| owner));
            match owner {
                Ok(owner) => Some(owner),
                Err(reason) => {
                    tally.law_reject.push(format!("sub-emitter target {node}: {reason}"));
                    return None;
                }
            }
        }
    };
    // Every other gate passed: the scene is what a collision system lacks.
    if emitter.collision.is_some() {
        tally.law_reject.push(format!("{node}: CollisionModule {}", crate::particle_runtime::COLLISION_SCENE_NOT_PORTED));
        return None;
    }
    Some(Planned {
        ordinal: 0,
        event_edges,
        child_owner,
        collision_owner,
        trail_owner,
        emission_surface,
        node: node.to_owned(),
        effect: effect_name.to_owned(),
        lifecycle,
        emitter,
        route,
        kind,
        camera_rotation,
        node_affine,
        source,
        draw: None,
        geometry: if let Some(reference) = mesh_reference {
            let glb = server.load(AssetPath::from_path_buf(std::path::PathBuf::from(format!("{asset_root}/{}", reference.file))).with_source("moly"));
            PlannedGeometry::Mesh { reference, glb, alignment: mesh_alignment.expect("validated Mesh alignment"), source: None, scaling, pivot: Vec3::from_array(pivot) }
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
        trail,
    })
}

/// The renderer block as the trail draw reads it: the trail material and the
/// trail vertex streams in place of the particle ones. Only the fixed trail
/// vertex layout (position, colour, UV) is transcribed, so custom trail
/// streams refuse.
fn trail_renderer(renderer: &Value) -> Result<Value, String> {
    if renderer.get("useCustomTrailVertexStreams").and_then(Value::as_bool) != Some(false) {
        return Err("custom trail vertex streams are not transcribed".into());
    }
    let material = renderer.get("trailMaterial").filter(|v| v.is_object())
        .ok_or("the renderer carries no trail material")?;
    let streams = renderer.get("trailVertexStreams").filter(|v| v.is_object())
        .ok_or("the renderer carries no trail vertex streams")?;
    let mut trail = renderer.clone();
    trail["material"] = material.clone();
    trail["vertexStreams"] = streams.clone();
    trail["useCustomVertexStreams"] = Value::Bool(false);
    Ok(trail)
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

/// Whether the node and every ancestor on the chain `compose_affine` walks
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
        if let Some(trail) = &mut planned.trail {
            if let Err(error) = trail.source.resolve(&server, &catalogues) {
                if trail.source.error.as_deref() != Some(&error.0) { error!(%error, node=%planned.node, "weather trail material unresolved"); }
                trail.source.error = Some(error.0);
                return;
            }
            if trail.source.passes.is_empty() { return; }
            if trail.draw.is_none() {
                let mesh = meshes.add(billboard::empty_mesh());
                let draw = commands.spawn((Mesh3d(mesh.clone()), trail.source.clone(), Transform::IDENTITY,
                    NoFrustumCulling, WeatherFxPreflight(request_serial), crate::shadowmap::NoShadowCast)).id();
                trail.draw = Some((draw, mesh));
            }
        }
    }
    for planned in &mut plan.planned {
        if let ParticleReadiness::Failed(error) = &*planned.source.readiness.lock().unwrap() {
            if planned.source.error.as_ref() != Some(error) { error!(%error, node=%planned.node, "weather GPU preparation failed"); }
            planned.source.error = Some(error.clone());
            return;
        }
        if let Some(trail) = &mut planned.trail {
            if let ParticleReadiness::Failed(error) = &*trail.source.readiness.lock().unwrap() {
                if trail.source.error.as_ref() != Some(error) { error!(%error, node=%planned.node, "weather trail GPU preparation failed"); }
                trail.source.error = Some(error.clone());
                return;
            }
        }
    }
    let ready = |source: &SourceParticle| matches!(*source.readiness.lock().unwrap(), ParticleReadiness::Ready);
    if !plan.planned.iter().all(|p| ready(&p.source) && p.trail.as_ref().is_none_or(|trail| ready(&trail.source))) { return; }
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
    for planned in std::mem::take(&mut plan.planned) {
        let is_global = planned.kind != EffectKind::Site;
        if (is_global && preserve_global) || (!is_global && site_timed_out) {
            if let Some((draw, _)) = planned.draw { commands.entity(draw).try_despawn(); }
            if let Some((draw, _)) = planned.trail.and_then(|trail| trail.draw) { commands.entity(draw).try_despawn(); }
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
        // The trail is the renderer's second draw: same sort key, drawn right
        // after the particles.
        let trail_draw = planned.trail.map(|trail| {
            let (entity, mesh) = trail.draw.expect("GPU preflight must precede installation");
            let mut source = trail.source;
            source.enabled = true;
            source.follows = Some(draw);
            commands.entity(entity).remove::<WeatherFxPreflight>().insert((source, WeatherFxDraw));
            (entity, mesh)
        });
        let has_trail = trail_draw.is_some();
        let has_distance = crate::particle_runtime::has_distance_emission(&planned.emitter);
        let effect_clock = effect_clocks.entry(planned.effect.clone())
            .or_insert_with(|| Arc::new(crate::weather_animation::EffectClock::new(now))).clone();
        let route = planned.route.clone();
        let event_edges = planned.event_edges;
        let child_owner = planned.child_owner;
        let trail_owner = planned.trail_owner;
        state.live.push(LiveWeatherEmitter { draw, trail_draw, native_refusal: None, lifecycle: planned.lifecycle.expect("weather plans own a source lifecycle"), effect_clock, runtime: Runtime {
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
            trail: None,
            collision: None,
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
            if let Some(owner) = child_owner {
                // A sub-emitter target: its own seed owner and streams, and the
                // owner words its parent's commands read; never the legacy step.
                match crate::particle_runtime::install_child_target(&mut live.runtime, &mut seed_manager, owner) {
                    Ok(()) => {
                        // A target with its own birth edges records their events.
                        if let Some(edges) = event_edges {
                            live.runtime.native_birth.as_mut().expect("target owner just installed").events =
                                Some(crate::particle_runtime::BirthEvents::with_edges(edges));
                        }
                        info!(node=%live.node, "weather sub-emitter target installed")
                    }
                    Err(reason) => {
                        error!(%reason, node=%live.node, "sub-emitter target refused by the child installer");
                        failed = true;
                    }
                }
            } else {
                match crate::particle_runtime::install_native_birth(&mut live.runtime, &mut seed_manager, &route) {
                    Ok(crate::particle_runtime::BirthPath::Native) => {
                        if let Some(edges) = event_edges {
                            // The collision edges' events are recorded by the
                            // CollisionModule, installed with the birth owner.
                            if let Some(collision) = live.runtime.collision.as_mut() {
                                collision.attach_edges(edges.collisions.clone());
                            }
                            live.runtime.native_birth.as_mut().expect("native birth owner just installed").events =
                                Some(crate::particle_runtime::BirthEvents::with_edges(edges));
                        }
                        if let Some(words) = trail_owner {
                            if let Err(reason) = crate::particle_runtime::attach_trail_owner(&mut live.runtime, words) {
                                error!(%reason, node=%live.node, "trail owner words refused");
                                failed = true;
                            }
                        }
                        if has_trail && (live.runtime.trail.is_none() || !crate::particle_runtime::trail_owner_ready(&live.runtime)) {
                            error!(node=%live.node, "trail system installed without its trail state or owner words");
                            failed = true;
                        }
                        info!(node=%live.node, noise=live.noise.is_some(), trail=live.runtime.trail.is_some(),
                            "weather native birth owner installed");
                    }
                    // Birth events run only on the native path; a parent with
                    // them is not left on the legacy step without its events.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) if event_edges.is_some() => {
                        error!(%reason, node=%live.node, "sub-emitter parent refused by the native birth installer");
                        failed = true;
                    }
                    // Nor is a system with a trail left drawing without it.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) if has_trail => {
                        error!(%reason, node=%live.node, "trail system refused by the native birth installer");
                        failed = true;
                    }
                    // Nor one that emits over distance, which only the native
                    // per-frame head takes.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) if has_distance => {
                        error!(%reason, node=%live.node, "distance-emitting system refused by the native birth installer");
                        failed = true;
                    }
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => live.native_refusal = Some(reason),
                    Err(error) => {
                        error!(%error, node=%live.node, "weather source seed owner unavailable");
                        failed = true;
                    }
                }
            }
        }
        if failed {
            let failed = state.live.pop().expect("just installed source instance");
            commands.entity(failed.draw).try_despawn();
            if let Some((trail, _)) = failed.trail_draw { commands.entity(trail).try_despawn(); }
            continue;
        }
    }
    plan.planned = waiting;
    link_sub_emitter_targets(&mut state.live);
    state.admitted = state.live.len();
    if install_site || install_global {
        info!("[weather-fx] {} @ {} phase site={} global={} active={} retiring={}",state.tier,state.env_site,install_site,install_global,state.live.len(),retiring.live.len());
    }
    if plan.site_installed && plan.global_installed { state.selection = Some(plan.selection.clone()); }
    if create_state { commands.insert_resource(created); }
    if plan.site_installed && plan.global_installed { commands.remove_resource::<WeatherFxPlan>(); }
}

/// Marks every birth edge whose child is an installed target of the same
/// effect instance (the instance's shared clock) delivered.
fn link_sub_emitter_targets(live: &mut [LiveWeatherEmitter]) {
    let targets: Vec<(Arc<crate::weather_animation::EffectClock>, String)> = live.iter()
        .filter(|emitter| emitter.native_birth.as_ref().is_some_and(|birth| birth.target.is_some()))
        .map(|emitter| (emitter.effect_clock.clone(), emitter.node.clone()))
        .collect();
    for emitter in live.iter_mut() {
        let clock = emitter.effect_clock.clone();
        for (_, node) in targets.iter().filter(|(target_clock, _)| Arc::ptr_eq(target_clock, &clock)) {
            if let Some(events) = emitter.runtime.native_birth.as_mut().and_then(|birth| birth.events.as_mut()) {
                events.deliver_to(node);
            }
            if let Some(collision) = emitter.runtime.collision.as_mut() {
                collision.deliver_to(node);
            }
        }
    }
}

/// Hands each parent's queued commands, in the order recorded, to its target
/// in the same effect instance. A refused command changes nothing but the
/// target's refusal count; the first refusal of a target is logged. A command
/// whose target has gone (destroyed before its parent) is dropped: the edge
/// tally still counts it.
/// A target that is itself a parent records commands while it takes its
/// parent's; those are handed to the next level in the same frame, until no
/// command is left. Chains are bounded by the effect's systems (each target
/// has one parent), so the rounds are too; a frame that still holds commands
/// after one round per system drops them.
fn deliver_sub_emitter_commands(systems: &mut [(&mut LiveWeatherEmitter, bool)], frame_dt: f32) {
    for _round in 0..systems.len().max(1) {
        if !deliver_round(systems, frame_dt) {
            return;
        }
    }
    for (live, _) in systems.iter_mut() {
        let dropped = live.runtime.native_birth.as_mut().and_then(|birth| birth.events.as_mut())
            .map_or(0, |events| events.take_commands().len())
            + live.runtime.collision.as_mut().map_or(0, |collision| collision.take_commands().len());
        if dropped > 0 {
            error!(effect=%live.effect, node=%live.node, dropped, "sub-emitter commands left after every delivery round");
        }
    }
}

/// One round: every parent's queued commands, in the order recorded, to its
/// targets. Returns whether any command was handed over.
fn deliver_round(systems: &mut [(&mut LiveWeatherEmitter, bool)], frame_dt: f32) -> bool {
    let mut delivered = false;
    for parent in 0..systems.len() {
        let runtime = &mut systems[parent].0.runtime;
        let mut commands = runtime.native_birth.as_mut().and_then(|birth| birth.events.as_mut())
            .map_or_else(Vec::new, |events| events.take_commands());
        if let Some(collision) = runtime.collision.as_mut() {
            commands.extend(collision.take_commands());
        }
        if commands.is_empty() {
            continue;
        }
        delivered = true;
        let clock = systems[parent].0.effect_clock.clone();
        for (target, command) in commands {
            let found = systems.iter_mut().find(|(live, _)| Arc::ptr_eq(&live.effect_clock, &clock)
                && live.node == target
                && live.native_birth.as_ref().is_some_and(|birth| birth.target.is_some()));
            let Some((live, _)) = found else { continue };
            if let Err(reason) = crate::particle_runtime::deliver_command(&mut live.runtime, &command, frame_dt) {
                let first = live.native_birth.as_ref().and_then(|birth| birth.target.as_ref())
                    .is_some_and(|state| state.refused == 1);
                if first {
                    error!(%reason, effect=%live.effect, node=%live.node, "sub-emitter command refused by its target");
                }
            }
        }
    }
    delivered
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
    // Observe instance age even when no camera can produce a particle draw.
    // This records effect lifecycle time, not a claimed Unity Animator phase.
    let now = time.elapsed_secs_f64();
    for live in state.as_deref().into_iter().flat_map(|state| &state.live)
        .chain(retiring.live.iter().map(|entry| &entry.emitter)) {
        live.effect_clock.observe(now);
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

    let dt = time.delta_secs();
    let mut systems: Vec<(&mut LiveWeatherEmitter, bool)> = state.as_deref_mut().into_iter()
        .flat_map(|s| s.live.iter_mut()).map(|s| (s, true))
        .chain(retiring.live.iter_mut().map(|s| (&mut s.emitter, false)))
        .collect();
    for (live, emitting) in systems.iter_mut() {
        let (system, emitting) = (&mut live.runtime, *emitting);
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
        crate::particle_runtime::advance_frame(system, dt, &ctx, emitting);
    }
    // Every target's own frame has run: the parents' commands of this frame
    // now reach their targets, and the geometry below shows their births.
    deliver_sub_emitter_commands(&mut systems, dt);
    for (live, _) in systems.iter_mut() {
        let system = &mut live.runtime;
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
        if let Some((_, trail_mesh)) = live.trail_draw.clone() {
            let system = &mut live.runtime;
            if let Some(mesh) = meshes.get_mut_untracked(&trail_mesh) {
                let owner = compose_to_world(system, &ctx);
                crate::particle_runtime::write_trail_mesh(mesh, system, &owner, camera_transform);
            }
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
