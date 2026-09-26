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
use crate::site::SiteActive;
use crate::source_particle::{SourceParticle, ParticleReadiness};
use moly_assets::source_shader::SourceShaderCatalogue;
use crate::weather_transition::{EnvironmentSelection, GlobalEffectIdentity, WeatherTransition, WeatherFxPrepared};
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, compose_to_world};
pub(crate) mod fixture;
mod lifecycle;

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
    /// The collider export the index names; `None` when the index names
    /// none (every collision system is then refused by name).
    collision: Option<Handle<JsonAsset>>,
    /// The index's records of components the extraction does not model, with
    /// their effect; `None` when the index carries no such census.
    unsupported: Option<Value>,
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
        /// The admission's decision (`particle_runtime::mesh_rotation_admission`).
        axis_body: Option<crate::particle_geometry::AxisBody>,
    },
    /// Mesh render mode with no drawable mesh in any slot: the renderer's
    /// mesh cache is empty, so the geometry preparation takes a mesh count of
    /// zero and draws nothing; the system still simulates.
    EmptyMesh {
        alignment: crate::particle_geometry::Alignment,
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
            Self::Mesh { scaling, .. } | Self::EmptyMesh { scaling, .. } =>
                crate::particle_runtime::ShapeEmitterEvidence { scaling: *scaling, mesh_renderer: true },
        }
    }
    fn into_runtime(self) -> crate::particle_runtime::Geometry {
        match self {
            Self::Billboard(draw) => crate::particle_runtime::Geometry::SourceBillboard(draw),
            Self::Mesh { alignment, source, scaling, pivot, flip, axis_body, .. } => crate::particle_runtime::Geometry::Mesh(crate::particle_geometry::MeshDraw {
                source: source.expect("source mesh readiness must precede weather commit"), alignment, scaling, pivot, flip, axis_body,
            }),
            Self::EmptyMesh { alignment, scaling, pivot } => crate::particle_runtime::Geometry::Mesh(
                crate::particle_geometry::MeshDraw::empty(alignment, scaling, pivot)),
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
    collision_owner: Option<moly_law::particle::collision_query::OwnerPair>,
    /// The owner local-to-world words a Local system's trail job composes
    /// with the view (the same chain); `None` for every other system.
    trail_owner: Option<[f32; 16]>,
    /// A sky system with owner words (child, collision or trail): its authored
    /// chain from the prefab root down, which the environment root's chain
    /// carries. The words above are composed with the root at the site
    /// origin; the installer and every frame recompose them from the tracked
    /// root.
    sky_owner_chain: Option<SkyChain>,
    /// Cone 的半顶角（shape 块的 `angle` 键；律的 `ShapeParams` 不带它）。
    cone_angle: Option<f32>,
    rol: Option<RotationOverLifetime>,
    limit: Option<LimitVelocity>,
    /// Whether the source culls this system while its renderer is invisible.
    culling: lifecycle::Culling,
    /// The trail draw of a system with a qualified TrailModule: the renderer's
    /// trail material and trail vertex streams, drawn after the particles.
    trail: Option<PlannedTrail>,
    /// The installed effects' ground scene for a system with a CollisionModule; the
    /// native birth installer installs the module with it.
    collision_scene: Option<Arc<crate::particle_runtime::collision_scene::GroundScene>>,
    /// The sub-emitter term of the first-Play warm length (0 for a system
    /// that does not warm or reaches no live child).
    sub_emitter_max_lifetime: f32,
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
    /// Accepted Animator players per selected effect, instantiated with the effect.
    animators: HashMap<String, Vec<crate::weather_animation::AnimatedNode>>,
    /// The effector's child set per selected effect.
    children: HashMap<String, Arc<Vec<lifecycle::EffectChild>>>,
    /// The colliders each selected effect adds to the physics scene when it
    /// is installed.
    colliders: Vec<PlannedColliders>,
    /// The site's own colliders, static for the whole visit.
    site_colliders: Arc<crate::particle_runtime::collision_scene::GroundScene>,
    tally: Tally,
    tier: String,
    env_site: String,
}

/// One selected effect's colliders, installed into the physics scene with
/// the effect and removed when the stopped effect is destroyed.
struct PlannedColliders {
    kind: EffectKind,
    effect: String,
    scene: Arc<crate::particle_runtime::collision_scene::GroundScene>,
    /// The effect's own delay from Stop to its destruction.
    destroy_delay: f64,
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
                        crate::source_billboard::Mode::Vertical => "source_vertical_billboard",
                        crate::source_billboard::Mode::Stretch(_) => "source_stretch",
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
                    "collision": s.collision.as_ref().map(|collision| serde_json::json!({
                        "calls": collision.calls, "hits": collision.hits, "draws": collision.draws,
                        "unreached": collision.unreached, "orderFree": collision.order_free,
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
                    "lifecycle": s.play.report(),
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
    /// The trail draw and its mesh, when the system has a trail.
    trail_draw: Option<(Entity, Handle<Mesh>)>,
    lifecycle: WeatherEffectLifecycle,
    effect_clock: Arc<crate::weather_animation::EffectClock>,
    /// The effect instance's Animator, shared by every emitter of the instance
    /// and kept through retirement (the GameObject outlives its particles).
    effect_animator: Option<Arc<crate::weather_animation::EffectAnimator>>,
    /// Set when the Animator rotates a Transform on this emitter's chain: the
    /// node affine is recomposed from it every frame.
    animated_chain: Option<NodeChain>,
    /// The clock this system's frame reads (its serialized useUnscaledTime).
    frame_clock: crate::particle_runtime::FrameClock,
    /// Engine play state, stop flags and culling state of this system.
    play: lifecycle::PlayState,
    /// The effector's child set of this system's effect instance.
    children: Arc<Vec<lifecycle::EffectChild>>,
    /// A sky system's owner words follow the tracked environment root.
    sky_owner: Option<SkyOwner>,
}

/// The chain a sky system's owner words are composed from and the root
/// position they were last composed with (see [`refresh_sky_owner`]).
struct SkyOwner {
    chain: SkyChain,
    anchor: [u32; 3],
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
}

/// One stopped effect instance: the effector's destruction loop and the
/// children of its child set whose play state the product does not hold.
struct RetiringInstance {
    clock: Arc<crate::weather_animation::EffectClock>,
    destroy: lifecycle::DestroyOnTime,
    unheld: Vec<String>,
    /// The effect's entries in the physics scene, removed with the instance.
    colliders: Vec<u64>,
}

#[derive(Resource, Default)]
pub(crate) struct WeatherFxRetirements {
    live: Vec<RetiringEmitter>,
    instances: Vec<RetiringInstance>,
    /// Draws of instances destroyed inside a Stop call, for the caller's commands.
    despawn: Vec<Entity>,
    /// The site's physics scene the collision systems query: the colliders
    /// of every installed effect, a stopped effect's until it is destroyed.
    physics: crate::particle_runtime::collision_scene::SiteScene,
    /// The installed effects' collider entries: kind, effect, entry and the
    /// effect's destroy delay.
    installed_colliders: Vec<(EffectKind, String, u64, f64)>,
    /// Colliders of stopped effects without an installed system (so without
    /// a stopped instance), and when they leave the scene.
    retiring_colliders: Vec<(u64, f64)>,
    /// The installed site's collider entry, by site.
    site_colliders: Option<(String, u64)>,
    /// The placed fixtures' collider entry while the active site has any, and
    /// the layout revision it was built for (none: the placeholder).
    fixture_colliders: Option<(u64, Option<u64>)>,
}

impl WeatherFxRetirements {
    fn stop(&mut self, active: &mut WeatherFxState, now: f64, delta: f32) {
        self.stop_matching(active, now, delta, |_| true);
    }
    /// SiteEnvironmentEffector.Stop for every instance holding a matching
    /// system: `Stop(StopEmitting)` on each system, then the first check of
    /// `DestroyOnTime` inside the same call with this frame's delta.
    /// The instance's colliders leave the physics scene when it is destroyed.
    fn stop_matching(&mut self, active: &mut WeatherFxState, now: f64, delta: f32, kinds: impl Fn(EffectKind)->bool) {
        let mut kept = Vec::new();
        let mut stopped: Vec<LiveWeatherEmitter> = Vec::new();
        for emitter in std::mem::take(&mut active.live) {
            if kinds(emitter.kind) { stopped.push(emitter); } else { kept.push(emitter); }
        }
        active.live = kept;
        active.admitted = active.live.len();
        let (mut stopped_colliders, installed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.installed_colliders).into_iter()
            .partition(|(kind, _, _, _)| kinds(*kind));
        self.installed_colliders = installed;
        let mut clocks: Vec<Arc<crate::weather_animation::EffectClock>> = Vec::new();
        for emitter in &stopped {
            if !clocks.iter().any(|clock| Arc::ptr_eq(clock, &emitter.effect_clock)) { clocks.push(emitter.effect_clock.clone()); }
        }
        for clock in clocks {
            let (mut members, rest): (Vec<_>, Vec<_>) = stopped.into_iter().partition(|e| Arc::ptr_eq(&e.effect_clock, &clock));
            stopped = rest;
            let mut undecided = Vec::new();
            for member in &mut members {
                let LiveWeatherEmitter { runtime, play, .. } = member;
                undecided.extend(play.stop(runtime, now));
            }
            let held: Vec<&str> = members.iter().map(|m| m.node.as_str()).collect();
            let unheld: Vec<String> = members[0].children.iter()
                .filter(|child| !child.never_holds_particles && !held.contains(&child.node.as_str()))
                .map(|child| format!("{}: not installed, so its play state is not held", child.node)).collect();
            let mut instance = RetiringInstance {
                clock, destroy: lifecycle::DestroyOnTime::new(members[0].lifecycle.time_until_destroy()), unheld,
                colliders: Vec::new(),
            };
            // The effect's colliders belong to this instance.
            let (own, rest): (Vec<_>, Vec<_>) = stopped_colliders.into_iter().partition(|(_, effect, _, _)| *effect == members[0].effect);
            stopped_colliders = rest;
            instance.colliders = own.into_iter().map(|(_, _, entry, _)| entry).collect();
            let playing = instance_playing(members.iter(), &instance.unheld, undecided, now);
            let destroyed = instance.destroy.check(playing, delta);
            if destroyed {
                self.despawn.extend(members.iter().map(|m| m.draw));
                for entry in &instance.colliders { self.physics.remove(*entry); }
            } else {
                self.live.extend(members.into_iter().map(|emitter| RetiringEmitter { emitter }));
                self.instances.push(instance);
            }
        }
        // An effect none of whose systems is installed has no instance: its
        // colliders leave after its own destroy delay from Stop.
        self.retiring_colliders.extend(stopped_colliders.into_iter().map(|(_, _, entry, delay)| (entry, now + delay)));
    }
    /// Installs the colliders of the plan's effects of the matching kinds.
    fn install_colliders(&mut self, planned: &[PlannedColliders], kinds: impl Fn(EffectKind)->bool) {
        for colliders in planned.iter().filter(|colliders| kinds(colliders.kind)) {
            let entry = self.physics.install(colliders.scene.clone());
            self.installed_colliders.push((colliders.kind, colliders.effect.clone(), entry, colliders.destroy_delay));
            if colliders.scene.collider_count() > 0 {
                info!(effect=%colliders.effect, colliders=colliders.scene.collider_count(),
                    "[weather-fx] effect colliders installed in the physics scene");
            }
        }
    }
    /// Removes the colliders of stopped effects without an instance whose
    /// delay has run out by `now`.
    fn expire_colliders(&mut self, now: f64) {
        let physics = self.physics.clone();
        self.retiring_colliders.retain(|(entry, destroy_at)| {
            if now < *destroy_at { return true; }
            physics.remove(*entry);
            false
        });
    }
    /// Installs the site's own colliders (static, for the whole visit), in
    /// place of another site's; `layout` when the site loads the player's
    /// layout (the home site).
    fn install_site(&mut self, site: &str, scene: &Arc<crate::particle_runtime::collision_scene::GroundScene>,
        layout: bool) {
        if self.site_colliders.as_ref().is_some_and(|(installed, _)| installed == site) { return; }
        if let Some((_, entry)) = self.site_colliders.take() { self.physics.remove(entry); }
        let entry = self.physics.install_site(scene.clone(), layout);
        self.site_colliders = Some((site.to_owned(), entry));
        info!(site=%site, colliders=scene.collider_count(), scene=%scene.describe(),
            "[weather-fx] site colliders installed in the physics scene");
    }
    /// Keeps the placed fixtures' colliders in the physics scene exactly
    /// while the active site has placed fixtures: the placeholder until their
    /// collision scenes are loaded, then their own colliders, rebuilt when the
    /// layout revision changes.
    fn sync_fixtures(&mut self, present: bool, revision: u64,
        build: impl FnOnce() -> Result<Arc<crate::particle_runtime::collision_scene::GroundScene>, String>) {
        use crate::particle_runtime::collision_scene as scene;
        if !present {
            if let Some((entry, _)) = self.fixture_colliders.take() {
                self.physics.remove(entry);
            }
            return;
        }
        if matches!(self.fixture_colliders, Some((_, Some(built))) if built == revision) {
            return;
        }
        match build() {
            Ok(built) => {
                if let Some((entry, _)) = self.fixture_colliders.take() {
                    self.physics.remove(entry);
                }
                info!("[weather-fx] placed fixtures' colliders in the physics scene: revision {revision}, {} answered \
                    against, {} bounded and refused in; {}", built.collider_count(), built.bounded_count(), built.describe());
                self.fixture_colliders = Some((self.physics.install_fixtures(built), Some(revision)));
            }
            Err(reason) => {
                if self.fixture_colliders.is_none() {
                    info!("[weather-fx] placed fixtures' colliders in the physics scene: {} ({reason})", scene::FIXTURE_PENDING);
                    self.fixture_colliders = Some((self.physics.install_fixtures(scene::fixture_pending()), None));
                }
            }
        }
    }
    fn clear_colliders(&mut self) {
        self.physics.clear();
        self.installed_colliders.clear();
        self.retiring_colliders.clear();
        self.site_colliders = None;
        self.fixture_colliders = None;
    }
}

/// Whether the active site has placed fixtures (their colliders are in the
/// physics scene).
fn fixtures_placed(placements: Option<&crate::fixture::FixturePlacements>, site: Option<&SiteActive>) -> bool {
    match (placements, site) {
        (Some(placements), Some(site)) => placements.site_id() == site.site_id && !placements.placed_rows().is_empty(),
        _ => false,
    }
}

/// The placed fixtures' colliders follow the active site's placements.
pub(crate) fn sync_fixture_colliders(
    placements: Option<Res<crate::fixture::FixturePlacements>>,
    site: Option<Res<SiteActive>>,
    revision: Option<Res<crate::fixture::FixtureLayoutRevision>>,
    collision: crate::fixture_collision::CollisionInputs,
    mut retiring: ResMut<WeatherFxRetirements>,
) {
    let present = fixtures_placed(placements.as_deref(), site.as_deref());
    let revision = revision.map_or(0, |revision| revision.0);
    retiring.sync_fixtures(present, revision, || {
        let placements = placements.as_deref().ok_or_else(|| "no placements".to_owned())?;
        fixture_parts(&collision, placements).map(crate::particle_runtime::collision_scene::fixture_scene)
    });
}

/// A canonical-frame pose (source X reflected) in source axes: the rotation
/// (x, y, z, w) and the translation.
fn source_pose(rotation: Quat, translation: Vec3) -> ([f32; 4], [f32; 3]) {
    ([rotation.x, -rotation.y, -rotation.z, rotation.w], [-translation.x, translation.y, translation.z])
}

/// A canonical-frame matrix (source X reflected) in source axes.
fn source_matrix(world: &GlobalTransform) -> [f32; 16] {
    let m = world.to_matrix().to_cols_array();
    let sign = |i: usize| if i == 0 { -1.0 } else { 1.0 };
    std::array::from_fn(|n| sign(n % 4) * sign(n / 4) * m[n])
}

/// The placed fixtures' colliders in source axes: the colliders of their
/// collision scenes, and each placed view's touch box at the placement's pose.
/// The view receives the fixture's own (unturned) grid size for its touch box
/// and turns its own transform to the direction, so the touch box, a child
/// at the view's origin, turns with it.
fn fixture_parts(collision: &crate::fixture_collision::CollisionInputs, placements: &crate::fixture::FixturePlacements)
    -> Result<Vec<crate::particle_runtime::collision_scene::FixturePart>, String> {
    use crate::fixture_collision::PhysicsShape;
    use crate::particle_runtime::collision_scene::{touch_box, FixturePart, FixtureShape, TOUCH_BOX_LAYERS};
    let reflect = |p: [f32; 3]| [-p[0], p[1], p[2]];
    let mut parts = Vec::new();
    // Each row's slot in the site view's fixture list (layout type and the
    // footprint's minimum y, in the placement mock's row order); a collider
    // finds its row by its fixture's package when no other row shares it.
    let rows = placements.editor_rows();
    let keys: Vec<Option<(u8, i32)>> =
        rows.iter().map(|row| row.footprint().ok().map(|(min, _)| (row.layout, i32::from(min.y)))).collect();
    let slots: Vec<Option<usize>> = if keys.iter().all(Option::is_some) {
        let keys: Vec<(u8, i32)> = keys.iter().flatten().copied().collect();
        crate::particle_runtime::collision_scene::fixture_slots(&keys).into_iter().map(Some).collect()
    } else {
        vec![None; rows.len()]
    };
    let slot_of = |package: &str| {
        let mut found = rows.iter().enumerate().filter(|(_, row)| row.package == package);
        match (found.next(), found.next()) {
            (Some((r, _)), None) => slots[r],
            _ => None,
        }
    };
    for (i, collider) in collision.physics_colliders(placements.total())?.into_iter().enumerate() {
        let shape = match collider.shape {
            PhysicsShape::Mesh { convex, cooking, positions, triangles } => FixtureShape::Mesh {
                convex, cooking,
                positions: positions.into_iter().map(reflect).collect(),
                // The canonical frame reverses the winding.
                triangles: triangles.into_iter().map(|t| [t[0], t[2], t[1]].map(|v| v as u32)).collect(),
            },
            PhysicsShape::Box { center, size } => FixtureShape::Box { center: reflect(center), half: size.map(|v| v * 0.5) },
            PhysicsShape::Other(kind) => FixtureShape::Unknown(kind),
        };
        let (scale, rotation, translation) = collider.world.to_scale_rotation_translation();
        let pose = (scale == Vec3::ONE).then(|| source_pose(rotation, translation));
        let slot = collider.package.as_deref().and_then(slot_of);
        parts.push(FixturePart { what: format!("collider {i}"), layers: 1 << collider.layer,
            world: source_matrix(&collider.world), pose, shape, slot, touch: false });
    }
    for (r, row) in rows.iter().enumerate() {
        let view = row.pose()?;
        let world = source_matrix(&GlobalTransform::from(view));
        let pose = (view.scale == Vec3::ONE).then(|| source_pose(view.rotation, view.translation));
        let (center, half) = touch_box([row.grid_size.x, row.grid_size.y, row.grid_size.z], row.layout);
        let shape = FixtureShape::Box { center, half };
        parts.push(FixturePart { what: format!("{} touch box", row.package), layers: TOUCH_BOX_LAYERS, world, pose,
            shape, slot: slots[r], touch: true });
    }
    Ok(parts)
}

/// `IsActiveParticle`: whether any child of the instance is playing.
fn instance_playing<'a>(members: impl Iterator<Item = &'a LiveWeatherEmitter>, unheld: &[String],
    mut undecided: Vec<String>, now: f64) -> lifecycle::InstancePlaying {
    if members.into_iter().any(|m| m.play.is_playing(&m.runtime, now)) {
        return lifecycle::InstancePlaying::Yes;
    }
    undecided.extend(unheld.iter().cloned());
    if undecided.is_empty() { lifecycle::InstancePlaying::No } else { lifecycle::InstancePlaying::Unknown(undecided) }
}

/// `DestroyOnTime` in the Update phase: independent of camera availability,
/// simulationSpeed and sky fade; it reads the play states the previous
/// frame's update left and this frame's `Time.deltaTime`.
pub(crate) fn expire_retirements(
    mut commands: Commands,
    mut retiring: ResMut<WeatherFxRetirements>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let delta = crate::particle_runtime::source_delta_time(time.delta());
    let WeatherFxRetirements { live, instances, despawn, physics, .. } = &mut *retiring;
    // The frame's physics steps come before its scripts and particle update.
    physics.advance(delta);
    for entity in despawn.drain(..) { commands.entity(entity).try_despawn(); }
    let mut destroyed: Vec<Arc<crate::weather_animation::EffectClock>> = Vec::new();
    instances.retain_mut(|instance| {
        let members = live.iter().map(|entry| &entry.emitter).filter(|e| Arc::ptr_eq(&e.effect_clock, &instance.clock));
        let playing = instance_playing(members, &instance.unheld, Vec::new(), now);
        if instance.destroy.check(playing, delta) {
            destroyed.push(instance.clock.clone());
            for entry in &instance.colliders { physics.remove(*entry); }
            return false;
        }
        true
    });
    live.retain(|entry| {
        if !destroyed.iter().any(|clock| Arc::ptr_eq(clock, &entry.emitter.effect_clock)) { return true; }
        commands.entity(entry.emitter.draw).try_despawn();
        if let Some((trail, _)) = entry.emitter.trail_draw { commands.entity(trail).try_despawn(); }
        false
    });
    retiring.expire_colliders(now);
}

/// `RendererScene::NotifyInvisible` (EarlyUpdate): renderers visible in the
/// previous frame's passes and in none of the last frame's become invisible,
/// then renderers added since join the scene.
pub(crate) fn notify_invisible(
    mut state: Option<ResMut<WeatherFxState>>,
    mut retiring: ResMut<WeatherFxRetirements>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for live in state.as_deref_mut().into_iter().flat_map(|s| s.live.iter_mut())
        .chain(retiring.live.iter_mut().map(|entry| &mut entry.emitter)) {
        live.play.notify_invisible(now);
    }
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
    let collision = value.pointer("/collision/file").and_then(Value::as_str)
        .map(|file| server.load::<JsonAsset>(AssetPath::from(format!("moly://phenomena/{file}"))));
    commands.insert_resource(WeatherFxDoc {
        request_serial:request.request_serial,
        handle,
        animations,
        collision,
        unsupported: value.pointer("/summary/unsupported").cloned(),
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
    site: Option<Res<SiteActive>>,
    placements: Option<Res<crate::fixture::FixturePlacements>>,
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
    // The collider export: read once per plan, one ground scene per effect.
    let collision_document = match &doc.collision {
        None => Err("this asset root carries no collider export".to_owned()),
        Some(handle) => {
            if let LoadState::Failed(err) = server.load_state(handle) {
                Err(format!("the collider export failed to load: {err:?}"))
            } else if let Some(asset) = json.get(handle) {
                serde_json::from_str::<Value>(&asset.0).map_err(|err| format!("the collider export is not JSON: {err}"))
            } else {
                return;
            }
        }
    };
    let mut scenes = crate::particle_runtime::collision_scene::SceneBuilder::new(collision_document);

    // ---- 选 effect：只取源环境装载器按名字构造的三份预制件（见 source_environment_selection）----
    let selected = source_environment_selection(effects, &doc.tier, &doc.env_site);
    // Every collision system of the plan queries the colliders of all the
    // effects it installs (and of the stopped ones still retiring).
    let names: Vec<&str> = selected.iter().map(|(name, _, _)| name.as_str()).collect();
    // ... with the site's own colliders, and the placed fixtures' when the
    // site has any at planning time.
    let fixtures = fixtures_placed(placements.as_deref(), site.as_deref().filter(|site| site.env_site == doc.env_site));
    scenes.select(crate::particle_runtime::collision_scene::Installation::Together {
        effects: &names, site: Some(&doc.env_site), fixtures });
    let site_colliders = scenes.site_colliders(&doc.env_site);
    info!("[weather-fx] site {} colliders: {} answered against, placed fixtures {}; {}", doc.env_site,
        site_colliders.collider_count(), fixtures, site_colliders.describe());
    let mut colliders = Vec::new();
    for &(ref name, effect, kind) in &selected {
        let Ok(lifecycle) = WeatherEffectLifecycle::from_effect(effect) else { continue; };
        if let Ok(scene) = scenes.effect_colliders(name) {
            colliders.push(PlannedColliders { kind, effect: name.clone(), scene,
                destroy_delay: lifecycle.time_until_destroy() });
        }
    }

    let mut tally = Tally::default();
    let mut plans = Vec::new();
    let mut animators = HashMap::new();
    let mut children = HashMap::new();
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
        children.insert(effect_name.clone(), lifecycle::effect_children(particles, &by_path, &sub_emitter_owners));
        let ground = scenes.for_effect(effect_name);
        let bodies = census_names_no_body(effect_name, effect, doc.unsupported.as_ref());
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
                &ground,
                &bodies,
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
        for node in drop_orphan_targets(&mut plans, effect_start) {
            tally.admitted -= 1;
            tally.law_reject.push(format!("sub-emitter target {node}: its parent is not admitted in this effect"));
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
        children,
        colliders,
        site_colliders,
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
            "RotationBySpeedModule" => "rotationBySpeed",
            "CustomDataModule" => "customData", "UVModule" => "textureSheet",
            "ForceModule" => "forceOverLifetime",
            // Noise is consumed only together with the native birth owner;
            // judge() checks that route/composition after the law parse.
            "NoiseModule" => "noise",
            // Trails likewise run only on the native birth path with their own
            // draw; judge() checks the qualified subset and the trail material.
            "TrailModule" => "trails",
            // Collision likewise; judge() checks the module law's subset and
            // binds the effect's ground scene, or refuses by name.
            "CollisionModule" => "collision",
            // A system's own emission inherits the emitter velocity only
            // outside Local space (and the per-update module only in World
            // space), which neither step ports; a child Emit inherits the
            // command velocity in every space, which the child composition
            // takes or refuses by name.
            "InheritVelocityModule" if system.get("simulationSpace").and_then(Value::as_str) == Some("Local") => "inheritVelocity",
            "InheritVelocityModule" => return Err("enabled source module InheritVelocityModule outside Local space: the emitter-velocity inheritance of the system's own emission is not ported".into()),
            _ => return Err(format!("enabled source module {module} has no runtime consumer")),
        };
        if !system.get(field).is_some_and(Value::is_object) {
            return Err(format!("enabled source module {module} lacks its authored {field} parameters"));
        }
    }
    source_ring_buffer_admission(system)?;
    if system.get("start").and_then(|s| s.get("randomizeRotationDirection")).and_then(Value::as_f64) != Some(0.0) {
        return Err("source birth rotation direction randomization is not consumed".into());
    }
    Ok(())
}

/// The ring buffer is the serialized InitialModule mode and loop range. The
/// player writes neither at runtime: the managed MainModule ring accessors and
/// the native SetRingBufferMode/SetRingBufferLoopRange are absent from the
/// build, and SiteEnvironmentEffector only plays the cached child systems,
/// stops them with StopEmitting and destroys the instance later.
///
/// The ordinary incremental update consumes both on every slice: StartParticles
/// accepts up to twice maxParticles, SimulateParticles caps a Pause age just
/// below the end of life and wraps a Loop age inside the loop range for indices
/// below maxParticles, KillParticles never runs for Pause and spares indices
/// below maxParticles for Loop, and CopyParticlesToUnalignedDst replaces at the
/// ring cursor. The particle count therefore never returns to zero after the
/// first birth, so EndUpdateAll never moves a stopped ring system to the
/// stopped play state: isPlaying stays true after Stop and the effector's
/// destruction loop accumulates Time.deltaTime every frame until
/// timeUntilDestroy.
///
/// A looping prewarm system warms at its first Play before any frame; with
/// supportsProcedural that warm is Update with the procedural flag, otherwise
/// BeginUpdate over the ordinary slices. Neither warm has been executed for a
/// ring system, so both stay refused here. The authored loop range is admitted
/// inside the inspector domain 0 <= lo <= hi <= 1, where the executed cases lie.
fn source_ring_buffer_admission(system: &Value) -> Result<(), String> {
    match system.get("ringBufferMode").and_then(Value::as_u64) {
        Some(0) => return Ok(()),
        Some(1 | 2) => {}
        other => return Err(format!("source ring buffer mode {other:?} is not an engine mode")),
    }
    let range = system.get("ringBufferLoopRange").and_then(Value::as_array)
        .filter(|range| range.len() == 2)
        .and_then(|range| Some((range[0].as_f64()?, range[1].as_f64()?)));
    match range {
        Some((lo, hi)) if 0.0 <= lo && lo <= hi && hi <= 1.0 => {}
        Some(range) => return Err(format!("ring buffer loop range {range:?} outside the executed domain")),
        None => return Err("ring buffer system lacks its authored loop range".into()),
    }
    let flag = |key: &str| system.get(key).and_then(Value::as_bool);
    let (Some(prewarm), Some(looping)) = (flag("prewarm"), flag("looping")) else {
        return Err("ring buffer system lacks its authored prewarm/looping controls".into());
    };
    if prewarm && looping {
        return Err(match crate::particle_runtime::source_route(system) {
            crate::particle_runtime::SourceRoute::Procedural =>
                "ring buffer first-Play warm runs Update with the procedural flag; not transcribed".into(),
            crate::particle_runtime::SourceRoute::Ordinary =>
                "ring buffer first-Play warm over the ordinary slices has not been executed".into(),
            crate::particle_runtime::SourceRoute::Undecided(control) =>
                format!("ring buffer first-Play warm route undecided: {control}"),
        });
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

/// Shapes whose kernels exist only on the native birth path: Box, the
/// BurstSpread circle and single-sided edge, and the cone in its Loop,
/// PingPong and BurstSpread arc modes. Their births read the accepted count
/// of the StartParticles call, the lane index within it or the arc clock,
/// none of which the legacy step carries.
fn shape_needs_native_birth(shape: &moly_law::particle::schema::ShapeParams) -> bool {
    use moly_law::particle::schema::ShapeMode;
    match shape.shape_type.as_str() {
        "Box" => true,
        "Circle" => shape.controls.arc_mode == Some(ShapeMode::BurstSpread),
        "SingleSidedEdge" => shape.controls.radius_mode == Some(ShapeMode::BurstSpread),
        "Cone" => matches!(shape.controls.arc_mode, Some(ShapeMode::Loop | ShapeMode::PingPong | ShapeMode::BurstSpread)),
        _ => false,
    }
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
    // Box reads no mode. The BurstSpread circle and edge and the cone's Loop,
    // PingPong and BurstSpread modes have native kernels; they are admitted
    // only on the native birth path, which the judge checks once the route is
    // known.
    let native_only_mode = matches!(
        (kind, shape.get(mode).and_then(Value::as_str)),
        ("Circle", Some("BurstSpread")) | ("SingleSidedEdge", Some("BurstSpread"))
            | ("Cone", Some("Loop" | "PingPong" | "BurstSpread"))
    );
    if !matches!(kind, "Mesh" | "Box") && !native_only_mode && shape.get(mode).and_then(Value::as_str) != Some("Random") {
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
    /// The id is one 64-bit integer written two ways: the weather export and
    /// every sub-emitter pointer carry it as a decimal string, the fixture
    /// export's records as a JSON integer.
    fn child(&self, node: &str, path_id: &str) -> Option<&'a Value> {
        let same = |id: &Value| match id {
            Value::String(text) => text == path_id,
            Value::Number(number) => number.as_i64().is_some_and(|id| id.to_string() == path_id),
            _ => false,
        };
        let mut found = self.records.get(node)?.iter().copied()
            .filter(|record| record.get("systemPathId").is_some_and(same));
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
                let law = moly_law::particle::death_event::DeathEmitEdge::from_source(edge, emitter, &child)
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

/// Whether the effect's extracted component census rules out a Rigidbody or
/// Rigidbody2D on its nodes: the effect's omitted and collider records, and
/// the index's records of the components the extraction does not model,
/// which name their effect. A census that is not exported rules out nothing,
/// and neither does a component whose kind the export could not read.
pub(crate) fn census_names_no_body(effect_name: &str, effect: &Value, unsupported: Option<&Value>)
    -> Result<(), String> {
    for (list, name) in [(effect.get("omitted"), "omitted"), (effect.get("colliders"), "collider"),
        (unsupported, "unsupported")] {
        let Some(list) = list.and_then(Value::as_array) else {
            return Err(format!("the export carries no {name} component census"));
        };
        for entry in list {
            if name == "unsupported" && entry.get("effect").and_then(Value::as_str) != Some(effect_name) {
                continue;
            }
            let node = entry.get("node").and_then(Value::as_str).unwrap_or("");
            match entry.get("component") {
                Some(Value::Null) => return Err(format!("a component of unread kind on {node}")),
                Some(Value::String(component)) if component.starts_with("Rigidbody") =>
                    return Err(format!("{component} on {node}")),
                _ => {}
            }
        }
    }
    Ok(())
}

/// emitterVelocityMode 1 reads the velocity of a Rigidbody or Rigidbody2D on
/// the system's GameObject or a Transform ancestor. With none, or with a
/// kinematic one, the engine caches that no body was found and takes the
/// Transform delta, which is mode 0 bit for bit in every consumer (emission
/// over distance, the World birth placement, Initial and InheritVelocity
/// inheritance). When the effect's census names no body (`bodies`), and the
/// run-time ancestors are this runtime's anchors, which carry none, the system
/// takes the Transform mode. Without that proof mode 1 stays, and the native
/// path refuses it.
pub(crate) fn resolve_velocity_mode(emitter: &mut EmitterParams, bodies: &Result<(), String>) {
    if emitter.emitter_velocity_mode == Some(1) && bodies.is_ok() {
        emitter.emitter_velocity_mode = Some(0);
    }
}

/// An exported constant or two-constant start lifetime; `None` for any other
/// mode.
fn source_lifetime(curve: &Value) -> Option<moly_law::particle::prewarm::Lifetime> {
    use moly_law::particle::prewarm::Lifetime;
    let number = |value: &Value| value.as_f64().map(|n| n as f32).or_else(|| match value.as_str() {
        Some("Infinity") => Some(f32::INFINITY),
        Some("-Infinity") => Some(f32::NEG_INFINITY),
        _ => None,
    });
    match curve.get("mode").and_then(Value::as_str)? {
        "constant" => Some(Lifetime::Constant(number(curve.get("value")?)?)),
        "twoConstants" => Some(Lifetime::TwoConstants { min: number(curve.get("min")?)?, max: number(curve.get("max")?)? }),
        _ => None,
    }
}

/// The sub-emitter term of the first-Play warm length of `record`, read from
/// the authored graph: every system its edges reach, each edge's child
/// resolved as admission resolves it (the record at the edge's node whose
/// system object id is the pointer's) and a null pointer as no child, over
/// every edge whatever its trigger and whatever admission later makes of it.
/// A system whose SubModule is disabled hands out no child. A curve-mode
/// start lifetime of the system itself is refused by the warm before the
/// term is read, so the term is 0 there.
pub(crate) fn warm_child_term(record: &Value, emitter: &EmitterParams, graph: &SubEmitterGraph<'_>) -> Result<f32, String> {
    use moly_law::particle::prewarm::{sub_emitter_maximum_lifetime, Lifetime, SubEmitterNode};
    let own = match emitter.start.lifetime {
        MinMaxCurve::Constant(v) => Lifetime::Constant(v),
        MinMaxCurve::TwoConstants { min, max } => Lifetime::TwoConstants { min, max },
        _ => return Ok(0.0),
    };
    let mut records: Vec<&Value> = vec![record];
    let mut nodes = Vec::new();
    while nodes.len() < records.len() {
        let system = &records[nodes.len()]["system"];
        let enabled = system.pointer("/sourceModules/enabled").and_then(Value::as_array)
            .is_some_and(|modules| modules.iter().any(|module| module.as_str() == Some("SubModule")));
        let mut children = Vec::new();
        if enabled {
            for edge in system.get("subEmitters").and_then(Value::as_array).into_iter().flatten() {
                let pointer = (edge.pointer("/sourcePointer/fileId").and_then(Value::as_i64),
                    edge.pointer("/sourcePointer/pathId").and_then(Value::as_str));
                match pointer {
                    (Some(0), Some("0")) => children.push(None),
                    (Some(0), Some(path_id)) => {
                        let target = edge.get("emitter").and_then(Value::as_str)
                            .ok_or("an edge with a pointer names no child")?;
                        let child = graph.child(target, path_id)
                            .ok_or_else(|| format!("{target}: child system {path_id} not resolved to one record"))?;
                        let index = match records.iter().position(|known| std::ptr::eq(*known, child)) {
                            Some(index) => index,
                            None => {
                                records.push(child);
                                records.len() - 1
                            }
                        };
                        children.push(Some(index));
                    }
                    _ => return Err("a sub-emitter pointer that is not an object of this file".into()),
                }
            }
        }
        nodes.push(SubEmitterNode { sub_module_enabled: enabled, children,
            lifetime: source_lifetime(&system["start"]["lifetime"]) });
    }
    sub_emitter_maximum_lifetime(&nodes, 0, own.first_play_upper(emitter.duration)).map_err(str::to_owned)
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
/// (with two, the order of their commands is not in the export), a chain of
/// parents that does not come back to a node, an authored chain of a site or
/// sky effect (the owner words are composed there, see `chain_owner`), the
/// scaled clock, no warm and
/// the Local or Hierarchy scaling mode (the owner updates transcribed).
/// Returns the parent.
fn sub_emitter_target_gate(owners: &[String], particle: &Value, graph: &SubEmitterGraph<'_>, kind: EffectKind,
    instance_anchor: Option<GlobalTransform>, instance_targets: bool) -> Result<String, String> {
    let [parent] = owners else {
        return Err(format!("{} parents ({}): the order of their commands is not in the export",
            owners.len(), owners.join(", ")));
    };
    // A target's admission judges its parent's record, and that judgement
    // judges the parent's own parent when the parent is a target in turn:
    // the recursion climbs exactly these one-parent edges, and stops at a
    // node that is not a target or that its own run of this gate refuses
    // (a count of parents other than one, above). Walk them here, before
    // anything recurses: a self-edge or a cycle is refused by name, and
    // every chain let through ends, so the recursion is at most as deep as
    // the chain has targets.
    let node = particle.get("node").and_then(Value::as_str).unwrap_or("");
    let mut visited = std::collections::HashSet::from([node]);
    let mut next = parent.as_str();
    loop {
        if !visited.insert(next) {
            return Err(format!("sub-emitter chain revisits {next}"));
        }
        match graph.get(next).map(Vec::as_slice) {
            Some([above]) => next = above.as_str(),
            _ => break,
        }
    }
    // A host that composes a target's owner words from its instance's own
    // hierarchy (the played route) admits it on an instance anchor.
    if instance_anchor.is_some() && !instance_targets {
        return Err("owner words are composed only for a site effect on its authored chain".into());
    }
    if kind == EffectKind::Camera {
        return Err(format!("owner words: {CAMERA_OWNER_REFUSAL}"));
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
        Some(0 | 1) => {}
        mode => return Err(format!("scaling mode {mode:?}: only the Local and Hierarchy owner updates are transcribed")),
    }
    Ok(parent.clone())
}

/// The target's owner words from its effect's chain, root first (see
/// [`chain_owner`]). A matrix below the inverse's determinant threshold is
/// refused: the engine keeps using the zero inverse, which this runtime's own
/// transforms cannot follow.
fn child_owner_words(by_path: &HashMap<String, &Value>, path: &str, kind: EffectKind,
    scaling: moly_law::particle::owner::OwnerScaling)
    -> Result<(moly_law::particle::child_emit::ChildOwner, Option<SkyChain>), String> {
    let (owner, sky_chain) = chain_owner(by_path, path, kind, scaling)?;
    Ok((moly_law::particle::child_emit::ChildOwner::from_owner(&owner), sky_chain))
}

/// The owner update's branch for an admitted scaling mode.
fn owner_scaling(scaling: crate::particle_geometry::Scaling) -> moly_law::particle::owner::OwnerScaling {
    match scaling {
        crate::particle_geometry::Scaling::Hierarchy => moly_law::particle::owner::OwnerScaling::Hierarchy,
        crate::particle_geometry::Scaling::Local { .. } => moly_law::particle::owner::OwnerScaling::Local,
    }
}

/// A sky system's authored chain from the prefab root down and the scaling
/// mode its owner words are composed in.
#[derive(Clone)]
struct SkyChain {
    chain: Arc<[moly_law::particle::owner::SourceTrs]>,
    scaling: moly_law::particle::owner::OwnerScaling,
}

/// Why a camera effect's owner words are not composed. The effect's root is
/// a child of the rendering camera, and a "fix" effect's effector cancels
/// the camera's rotation in its Update; both compose exactly
/// ([`environment_owner`] with the cancel). But the camera moves in its
/// LateUpdate, after the frame's particle update, so the owner update reads
/// the camera pose of the previous frame (and the cancel written in this
/// frame's Update from that pose), which this host does not keep. No camera
/// effect of the corpus has a system that reads owner words.
const CAMERA_OWNER_REFUSAL: &str =
    "a camera effect's owner update reads the previous frame's camera pose (the camera moves in LateUpdate, after the particle update), which this host does not keep";

/// The owner words of a system on its effect's chain in its scaling mode,
/// with the chain the words follow at run time.
///
/// A site effect's prefab is instantiated under the site view, the field
/// prefab's root at the site origin; this host draws the active site at its
/// origin, and the site colliders sit in the same frame, so the words are the
/// authored chain's alone.
///
/// A sky effect's prefab is instantiated under `SiteRoot/SiteEnvironment-
/// ViewController/Sky/EffectRoot` (see [`EnvironmentRoot`]). The four nodes
/// take part in the engine's owner update, so they are composed with the
/// prefab chain ([`environment_owner`]); a sky effect's effector never
/// cancels a rotation (the loader passes rotation type 0 and every sky
/// effector serializes the normal type). The words returned here have the
/// root at the active site's origin; the installer and the frame recompose
/// them from the tracked root, and the returned chain is what they compose.
fn chain_owner(by_path: &HashMap<String, &Value>, path: &str, kind: EffectKind,
    scaling: moly_law::particle::owner::OwnerScaling)
    -> Result<(moly_law::particle::owner::OwnerMatrices, Option<SkyChain>), String> {
    let prefab = authored_trs(by_path, path)?;
    match kind {
        EffectKind::Site => Ok((environment_owner(&[], &prefab, false, scaling)?, None)),
        EffectKind::Sky => {
            let owner = environment_owner(&sky_prefix([0.0; 3]), &prefab, false, scaling)?;
            Ok((owner, Some(SkyChain { chain: prefab.into(), scaling })))
        }
        EffectKind::Camera => Err(CAMERA_OWNER_REFUSAL.into()),
    }
}

/// The nodes above a sky prefab's root, root first: the site root, the
/// environment view controller, the sky view and its effect root. The source
/// authors all four at the identity, and the only writer of any of them is
/// the cannon move, which translates the controller; `anchor` is the
/// controller's position (source axes) seen from the active site.
pub(crate) fn sky_prefix(anchor: [f32; 3]) -> [moly_law::particle::owner::SourceTrs; 4] {
    let identity = moly_law::particle::owner::SourceTrs { t: [0.0; 3], q: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] };
    [identity, moly_law::particle::owner::SourceTrs { t: anchor, ..identity }, identity, identity]
}

/// The owner words in `scaling` of the last node of `prefab`, a prefab
/// chain from its root down, instantiated below `prefix` (root first;
/// the instantiation keeps the prefab root's local TRS). With `cancel`, the
/// prefab root carries the rotation a rotation-type-1 effector writes on it,
/// the inverse of its parent's world rotation through the local-rotation
/// setter. Refused below the inverse's determinant threshold.
pub(crate) fn environment_owner(prefix: &[moly_law::particle::owner::SourceTrs],
    prefab: &[moly_law::particle::owner::SourceTrs], cancel: bool, scaling: moly_law::particle::owner::OwnerScaling)
    -> Result<moly_law::particle::owner::OwnerMatrices, String> {
    use moly_law::particle::owner::{cancel_rotation, owner_matrices, SourceTrs};
    let mut chain: Vec<SourceTrs> = prefix.iter().chain(prefab).copied().collect();
    if cancel {
        let root = prefab.first().ok_or("owner chain lacks the prefab root")?;
        let q = cancel_rotation(prefix).map_err(|refused| format!("rotation cancel {refused:?}"))?;
        chain[prefix.len()] = SourceTrs { q, ..*root };
    }
    let owner = owner_matrices(&chain, scaling).map_err(|refused| format!("owner words {refused:?}"))?;
    if !owner.invert_ok {
        return Err("owner matrix below the inverse's determinant threshold".into());
    }
    Ok(owner)
}

/// The node's authored chain in source TRS words, the prefab root first.
fn authored_trs(by_path: &HashMap<String, &Value>, path: &str)
    -> Result<Vec<moly_law::particle::owner::SourceTrs>, String> {
    use moly_law::particle::owner::SourceTrs;
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
    chain.iter().rev()
        .map(|node| Some(SourceTrs { t: words(node, "position")?, q: words(node, "rotation")?, s: words(node, "scale")? }))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "owner chain lacks an authored TRS".to_owned())
}

/// Whether the target's parent is admitted with a birth or death edge to it:
/// the parent record is judged as its own turn judges it.
#[allow(clippy::too_many_arguments)]
fn parent_delivers(effect_name: &str, parent: &str, node: &str, by_path: &HashMap<String, &Value>,
    graph: &SubEmitterGraph<'_>, kind: EffectKind, camera_rotation: bool,
    lifecycle: Option<WeatherEffectLifecycle>, native_owner: bool, asset_root: &str,
    instance_anchor: Option<GlobalTransform>, instance_targets: bool,
    ground: &crate::particle_runtime::collision_scene::SceneVerdict, bodies: &Result<(), String>,
    server: &AssetServer) -> Result<(), String> {
    let records: Vec<&Value> = graph.records.get(parent).into_iter().flatten().copied()
        .filter(|record| record.pointer("/system/subEmitters").and_then(Value::as_array)
            .is_some_and(|edges| edges.iter().any(|edge| edge.get("emitter").and_then(Value::as_str) == Some(node))))
        .collect();
    let [record] = records.as_slice() else {
        return Err(format!("parent {parent}: {} records name this target", records.len()));
    };
    let mut scratch = Tally::default();
    match judge_in_host(effect_name, record, by_path, graph, kind, camera_rotation, lifecycle, native_owner, asset_root,
        instance_anchor, instance_targets, ground, bodies, server, &mut scratch) {
        Some(planned) if planned.event_edges.as_ref().is_some_and(|edges| edges.targets().any(|target| target == node)) =>
            Ok(()),
        Some(_) => Err(format!("parent {parent} admitted without an event edge to this target")),
        None => Err(format!("parent {parent} not admitted ({})", scratch.law_reject.last()
            .cloned().unwrap_or_else(|| format!("{scratch:?}")))),
    }
}

/// An emitter at or below a Transform the effect's accepted Animator rotates
/// follows that rotation through its authored node chain every frame. Only an
/// emitter that composes the chain itself and simulates in Local space is
/// covered: World-space births, inherited emitter velocity and shape placement
/// against a moving Transform were not part of the evaluation that was read.
///
/// Its world matrix is the chain composition with the Animator's rotation in
/// place of the serialized one, carried by the effect's anchor. That was
/// evaluated against the engine's owner matrix
/// (`ParticleSystem::UpdateLocalToWorldMatrixAndScales` over the
/// TransformHierarchy, Local scaling; Hierarchy scaling gives the same matrix
/// on a unit-scale chain) for one form only, which is all this admits: the
/// Animator rotates the emitter's own node and no other link of its chain,
/// every link above that node keeps an identity rotation, every link has unit
/// scale, at most two links carry a translation, and the anchor carries no
/// rotation (the sky and site anchors, or the camera anchor when the effect
/// follows only the camera's position). On that form the engine's quaternion
/// accumulation leaves the rotation exact, its rotation matrix is the
/// product's term for term, and each translation component is one addition of
/// the same two values. Ancestor rotations or scales change the arithmetic, and a
/// third translation is summed in another grouping (the engine accumulates
/// from the emitter up, the product composes from the root down); neither was
/// evaluated.
fn admit_animated(animation: &crate::weather_animation::Contract, mut planned: Planned, tally: &mut Tally)
    -> Option<Planned> {
    let animated = animation.animated_ancestors(&planned.node);
    if animated.is_empty() {
        return Some(planned);
    }
    let composed = planned.node_chain.as_ref().is_some_and(|chain|
        animated.iter().all(|path| chain.links.iter().any(|link| link.path == *path)));
    let chain = planned.node_chain.as_ref().filter(|_| composed);
    let reason = if chain.is_none() {
        "emitter below an animated Transform does not compose its authored node chain"
    } else if planned.emitter.simulation_space != SimulationSpace::Local {
        "emitter below an animated Transform does not simulate in Local space"
    } else if animated.len() != 1 || animated[0] != planned.node {
        "the Animator rotates a Transform above the emitter's own node; that owner matrix was not evaluated"
    } else if chain.is_some_and(|chain| chain.links.iter().any(|link| link.local.scale != Vec3::ONE)) {
        "an animated emitter's chain carries a non-unit scale; that owner matrix was not evaluated"
    } else if chain.is_some_and(|chain| chain.links.iter()
        .any(|link| link.path != planned.node && link.local.rotation != Quat::IDENTITY)) {
        "an animated emitter's ancestor carries a rotation; that owner matrix was not evaluated"
    } else if chain.is_some_and(|chain|
        chain.links.iter().filter(|link| link.local.translation != Vec3::ZERO).count() > 2) {
        "an animated emitter's chain carries more than two translations; that owner matrix was not evaluated"
    } else if planned.kind == EffectKind::Camera && planned.camera_rotation {
        "an animated emitter under the rotating camera anchor; that owner matrix was not evaluated"
    } else if planned.collision_owner.is_some() || planned.trail_owner.is_some() || planned.child_owner.is_some() {
        "an animated emitter's owner words: they are composed from the authored rotations, not the Animator's"
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
    sub_emitter_owners: &SubEmitterGraph<'_>,
    kind: EffectKind,
    camera_rotation: bool,
    lifecycle: WeatherEffectLifecycle,
    ground: &crate::particle_runtime::collision_scene::SceneVerdict,
    bodies: &Result<(), String>,
    server: &AssetServer,
    tally: &mut Tally,
) -> Option<Planned> {
    judge_in_archive(effect_name, particle, by_path, sub_emitter_owners, kind,
        camera_rotation, Some(lifecycle), "phenomena", None, ground, bodies, server, tally)
}

#[allow(clippy::too_many_arguments)]
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
    ground: &crate::particle_runtime::collision_scene::SceneVerdict,
    // Whether the effect's component census rules out a Rigidbody on its
    // nodes ([`census_names_no_body`]).
    bodies: &Result<(), String>,
    server: &AssetServer,
    tally: &mut Tally,
) -> Option<Planned> {
    // The weather host (a lifecycle) installs the native birth owner.
    judge_in_host(effect_name, particle, by_path, sub_emitter_owners, kind, camera_rotation, lifecycle,
        lifecycle.is_some(), asset_root, instance_anchor, false, ground, bodies, server, tally)
}

/// [`judge_in_archive`] for a host that says whether it installs the native
/// birth owner and keeps both frame clocks: the weather host and the fixture
/// host do (its Director-driven systems install the owner at each
/// `ParticleSystem.Simulate` restart and step by the Director's time, which
/// reads neither frame clock).
#[allow(clippy::too_many_arguments)]
fn judge_in_host(
    effect_name: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    sub_emitter_owners: &SubEmitterGraph<'_>,
    kind: EffectKind,
    camera_rotation: bool,
    lifecycle: Option<WeatherEffectLifecycle>,
    native_owner: bool,
    asset_root: &str,
    instance_anchor: Option<GlobalTransform>,
    // The host composes a sub-emitter target's owner words from the spawned
    // instance hierarchy (see [`sub_emitter_target_gate`]).
    instance_targets: bool,
    ground: &crate::particle_runtime::collision_scene::SceneVerdict,
    bodies: &Result<(), String>,
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
        Some(owners) => match sub_emitter_target_gate(owners, particle, sub_emitter_owners, kind, instance_anchor,
            instance_targets) {
            Ok(parent) => Some(parent),
            Err(reason) => { tally.law_reject.push(format!("sub-emitter target: {reason}")); return None; }
        },
    };
    let render_mode = renderer.get("renderMode").and_then(Value::as_str).unwrap_or("");
    if !matches!(render_mode, "Billboard" | "HorizontalBillboard" | "VerticalBillboard" | "Stretch" | "Mesh") {
        tally.render_mode.push(format!("unsupported source render mode {render_mode}"));
        return None;
    }
    let alignment_id = renderer.get("alignment").and_then(Value::as_i64).unwrap_or(-1);
    let mesh_alignment = crate::particle_geometry::Alignment::from_source(alignment_id);
    if mesh_alignment.is_none() {
        tally.alignment.push(Alignment::render_space_name(alignment_id).to_owned());
        return None;
    }
    // ParticleSystemRenderer caches, in slot order, its populated slots whose
    // mesh is drawable; the geometry preparation counts the leading cached
    // meshes. No populated slot (and so nothing resolved): the count is zero
    // and nothing is drawn, while the system simulates as any other.
    let empty_mesh = render_mode == "Mesh"
        && renderer.get("meshes").and_then(Value::as_array).is_some_and(Vec::is_empty)
        && renderer.get("meshSlots").and_then(Value::as_array).is_some_and(|slots| !slots.is_empty()
            && slots.iter().all(|slot| slot.get("reference").and_then(|r| r.get("pathId")).and_then(Value::as_str) == Some("0")));
    let mesh_reference = if render_mode == "Mesh" && !empty_mesh {
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
        if !matches!(shape_type, "Circle" | "Cone" | "ConeVolume" | "Sphere" | "Hemisphere" | "SingleSidedEdge" | "Donut" | "Mesh" | "Box") {
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
    let mut emitter = match Effects::from_json_str(archive_bytes.as_bytes()) {
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
    // ParticleSystem::BeginUpdate hands a system with useUnscaledTime the
    // unscaled delta, which Time.maximumDeltaTime does not clamp, and any other
    // system Time.deltaTime. A block that does not carry the flag is refused
    // in either host rather than read as either clock. A host with the native
    // birth owner keeps both clocks; the Director path steps by the Director's
    // Simulate time and keeps no unscaled clock, so a Director-driven system
    // with the flag set is refused.
    match (emitter.use_unscaled_time, native_owner) {
        (None, _) => {
            tally.law_reject.push(format!("{node}: useUnscaledTime not exported; re-extract"));
            return None;
        }
        (Some(true), false) => {
            tally.law_reject.push(format!("{node}: useUnscaledTime needs the unscaled clock, which the Director's Simulate path does not keep"));
            return None;
        }
        _ => {}
    }
    resolve_velocity_mode(&mut emitter, bodies);
    // The first-Play warm length adds the sub-emitter term: the longest chain
    // of child lifetimes the authored graph reaches. Only a looping prewarm
    // system warms.
    let sub_emitter_max_lifetime = if emitter.prewarm && emitter.looping {
        match warm_child_term(particle, &emitter, sub_emitter_owners) {
            Ok(term) => term,
            Err(reason) => {
                tally.law_reject.push(format!("{node}: first-Play warm length: {reason}"));
                return None;
            }
        }
    } else {
        0.0
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
    // A CollisionModule runs only on the native birth path with the physics
    // scene of the installed effects. The module law (with its current-size
    // stream and its collision events) is checked here; the scene is bound
    // (or refused by name) after every other gate, below.
    if emitter.collision.is_some() {
        if let Err(reason) = crate::particle_runtime::collision_eligible(&emitter)
            .and_then(|()| crate::particle_runtime::current_size_source_gate(system)) {
            tally.law_reject.push(format!("{node}: CollisionModule {reason}"));
            return None;
        }
    }
    let route = crate::particle_runtime::source_route(system);
    let native_only_shape = emitter.shape.as_ref().is_some_and(shape_needs_native_birth);
    if native_only_shape {
        // The legacy step has no batch count, lane index or arc clock and no
        // Box sampler; these shapes run only with the native birth owner.
        if !native_owner {
            // The Director's ParticleSystem.Simulate steps run the legacy
            // step and never install the native birth owner.
            tally.shape.push(format!("{node}: {shape_type} mode needs the native birth owner, which the Director's Simulate path does not install"));
            return None;
        }
        // A sub-emitter target takes no route: its births come from its
        // parents' commands through the native Shape law.
        if child_parent.is_none() {
            if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route) {
                tally.shape.push(format!("{node}: {shape_type} mode requires the native birth path: {reason}")); return None;
            }
        }
        let reads_arc_clock = emitter.shape.as_ref().is_some_and(|s| s.shape_type == "Cone"
            && matches!(s.controls.arc_mode, Some(moly_law::particle::schema::ShapeMode::Loop | moly_law::particle::schema::ShapeMode::PingPong)));
        if emitter.prewarm && reads_arc_clock {
            tally.shape.push(format!("{node}: the arc clock over the first-Play warm is not transcribed")); return None;
        }
    }
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
    if emitter.ring_buffer_mode != moly_law::particle::RingBufferMode::Disabled {
        // The ring laws are the ordinary incremental order: simulate and kill the
        // existing particles, then emit, start and pack the newborns at the ring
        // cursor. The legacy step emits first and draws another stream.
        if !native_owner {
            // The Director's Simulate steps run the legacy step and never
            // install the native birth owner.
            tally.law_reject.push(format!("{node}: ring buffer needs the native birth owner, which the Director's Simulate path does not install"));
            return None;
        }
        if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route) {
            tally.law_reject.push(format!("{node}: ring buffer requires the native birth path: {reason}")); return None;
        }
    }
    if let Some(force) = &emitter.force {
        if let Err(error) = moly_law::particle::force::ForceOverLifetime::from_params(force) {
            tally.law_reject.push(format!("{node}: {error}")); return None;
        }
    }
    // Every other curve the runtime evaluates, through the engine's dispatch.
    // A CustomData lane whose value follows its curve objects' caches runs
    // with the law that follows the engine's storage past the live count: it
    // needs this host's native birth owner, and a sub-emitter target's births
    // (its parents' commands) are not what that law follows.
    let storage = || -> Result<(), String> {
        if !native_owner {
            return Err("the Director's Simulate path does not install the native birth owner".into());
        }
        if child_parent.is_some() {
            return Err("a sub-emitter target's storage follows its parents' commands, which the slot model does not".into());
        }
        crate::particle_runtime::custom_data_storage_eligible(&emitter, &route)
    };
    // A size lane whose value follows its curve objects' caches runs with the
    // law that follows the engine's SizeModule calls, on the same conditions.
    let size_storage = || -> Result<(), String> {
        if !native_owner {
            return Err("the Director's Simulate path does not install the native birth owner".into());
        }
        if child_parent.is_some() {
            return Err("a sub-emitter target's module calls include its parents' Emit calls, which are not ported".into());
        }
        crate::particle_runtime::size_storage_eligible(&emitter, &route)
    };
    if let Err(error) = crate::particle_runtime::curve_admission_with(&emitter, Some(&storage), Some(&size_storage)) {
        tally.law_reject.push(format!("{node}: {error}")); return None;
    }
    if crate::particle_runtime::has_start_delay(&emitter) {
        // Play writes the start delay word and Update1Incremental counts it
        // down on the native birth path (the frame head's distance births
        // wait for it too). A random delay is evaluated with the system
        // seed's hash, not transcribed; the Director's Simulate path and the
        // legacy step carry no such word; a sub-emitter target is stopped
        // every frame, so its word never counts down and holds its clock
        // (its Tick is not called while the word is not below the slice),
        // which the target install does not carry.
        let reason = if crate::particle_runtime::play_start_delay(&emitter).is_none() {
            Some("random start delay: Play's seed-hash evaluation is not transcribed".to_owned())
        } else if !native_owner {
            Some("start delay: the Director's Simulate path runs the legacy step, which has no start delay word".to_owned())
        } else if child_parent.is_some() {
            Some("start delay on a sub-emitter target: its uncounted word holds the target's clock, which the target install does not carry".to_owned())
        } else {
            crate::particle_runtime::native_birth_eligible(&emitter, &route).err()
                .map(|reason| format!("start delay needs the native birth path: {reason}"))
        };
        if let Some(reason) = reason {
            tally.start_delay += 1;
            tally.law_reject.push(format!("{node}: {reason}"));
            return None;
        }
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
    // The runtime builds RotationBySpeed from the emitter block each update;
    // a block its law does not take refuses the emitter here.
    if let Some(Err(error)) = emitter.rotation_by_speed.as_ref()
        .map(moly_law::particle::rotation_by_speed::RotationBySpeed::from_params) {
        tally.rol_refused.push(format!("{node}: {error}")); return None;
    }

    // Renderer-owned size limits, pivot, camera roll and vertex attributes are
    // mandatory source inputs. No visual minimum is substituted for zero size.
    let max_particle_size = renderer.get("maxParticleSize").and_then(Value::as_f64);
    let min_particle_size = renderer.get("minParticleSize").and_then(Value::as_f64);
    let allow_roll = renderer.get("allowRoll").and_then(Value::as_bool);
    let normal_direction = renderer.get("normalDirection").and_then(Value::as_f64);
    if render_mode != "Stretch" && normal_direction != Some(1.0) {
        tally.render_mode.push("source billboard normalDirection other than one is not yet verified".into()); return None;
    }
    if render_mode != "Mesh" && renderer.get("flip").and_then(Value::as_array)
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
    // The Stretch body reads its three scales, the normal direction and the
    // freeform flag (it reads no pivot). The freeform path, with its
    // rotate-with-stretch flag, is not transcribed; and the engine multiplies
    // the drawn velocity by the velocity module's speed modifier where that is
    // not the constant one, which the drawn instances here do not carry.
    let stretch = if render_mode == "Stretch" {
        let finite = |key: &str| renderer.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
        let (Some(velocity_scale), Some(length_scale), Some(camera_velocity_scale), Some(normal_direction)) =
            (finite("velocityScale"), finite("lengthScale"), finite("cameraVelocityScale"), normal_direction.filter(|v| v.is_finite())) else {
            tally.render_mode.push("Stretch renderer scales or normal direction not exported".into()); return None;
        };
        match renderer.get("freeformStretching").and_then(Value::as_bool) {
            Some(false) => {}
            Some(true) => { tally.render_mode.push("Stretch freeform stretching is not transcribed".into()); return None; }
            None => { tally.render_mode.push("Stretch freeform flag not exported".into()); return None; }
        }
        if emitter.velocity_over_lifetime.as_ref()
            .is_some_and(|v| v.speed_modifier != moly_law::particle::MinMaxCurve::Constant(1.0)) {
            tally.render_mode.push("Stretch with a velocity speed modifier other than the constant one: the drawn velocity carries it and is not transcribed".into());
            return None;
        }
        Some(crate::source_billboard::Stretch { velocity_scale, length_scale, camera_velocity_scale, normal_direction: normal_direction as f32 })
    } else {
        None
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
    if native_only_shape {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.shape.push(format!("{node}: {shape_type} mode requires the native birth path: {reason}")); return None;
        }
    }
    if emitter.noise.is_some() {
        // Noise runs only with the native birth owner; the emitter state the
        // native Shape boundary reads must qualify too, or Noise would drop.
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: Noise requires the native birth path: {reason}")); return None;
        }
    }
    // Without 3D rotation the mesh geometry turns each particle about its
    // axis of rotation, whichever birth path feeds it, in the render spaces
    // whose kernel is transcribed.
    let mut axis_body = None;
    if mesh_reference.is_some() {
        let initial_enabled = system.pointer("/sourceModules/enabled").and_then(Value::as_array)
            .is_some_and(|modules| modules.iter().any(|module| module.as_str() == Some("InitialModule")));
        match crate::particle_runtime::mesh_rotation_admission(&emitter, initial_enabled,
            mesh_alignment.expect("validated Mesh alignment"), allow_roll) {
            Ok(body) => axis_body = body,
            Err(refused) => {
                tally.render_mode.push(refused.reason().into());
                return None;
            }
        }
    }
    // A system that runs the legacy step must carry a start colour that step
    // evaluates as the source does. A host with the native birth owner
    // installs it where `native_birth_path` allows it; the Director path
    // never installs it.
    let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
    // A sub-emitter target never runs the legacy step (it takes no route).
    if child_parent.is_none()
        && (!native_owner || crate::particle_runtime::native_birth_path(&emitter, &route, Some(evidence)).is_err())
    {
        if let Err(refused) = crate::particle_runtime::legacy_start_colour(&emitter.start.color) {
            tally.law_reject.push(format!("{node}: {}", refused.reason()));
            return None;
        }
        if let Err(reason) = crate::particle_runtime::legacy_bursts(&emitter) {
            tally.law_reject.push(format!("{node}: {reason}"));
            return None;
        }
    }
    if emitter.ring_buffer_mode != moly_law::particle::RingBufferMode::Disabled {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: ring buffer requires the native birth path: {reason}")); return None;
        }
    }
    let culling = if lifecycle.is_some() {
        lifecycle::source_culling(system, renderer, &emitter, &route, instance_anchor.is_none() && unit_scale_chain(by_path, node))
    } else {
        lifecycle::Culling::Refused("the fixture host runs no culling pass".into())
    };
    if event_edges.is_some() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: sub-emitter events require the native birth path: {reason}"));
            return None;
        }
    }
    if !distance_zero && child_parent.is_none() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence)) {
            tally.law_reject.push(format!("{node}: emission over distance requires the native birth path: {reason}"));
            return None;
        }
    }
    // A TrailModule runs only with the native birth owner (its two update
    // points are in the native slices) and draws with the renderer's trail
    // material; a system is never admitted without its trail.
    let trail = if emitter.trails.is_some() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
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
    // local-to-world): the engine's owner update of the effect's chain (see
    // `chain_owner`), on an authored chain only.
    let mut sky_owner_chain = None;
    let local_owner = match emitter.simulation_space {
        moly_law::particle::schema::SimulationSpace::Local if emitter.collision.is_some() || emitter.trails.is_some() => {
            if instance_anchor.is_some() {
                tally.law_reject.push(format!(
                    "{node}: owner words of a Local collision or trail are composed only for a site effect on its authored chain"));
                return None;
            }
            match chain_owner(by_path, node, kind, owner_scaling(scaling)) {
                Ok((owner, chain)) => {
                    sky_owner_chain = chain;
                    Some(owner)
                }
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
    // The collision calls are in the native slices only; a collision system
    // is never admitted to the legacy step without them. (A sub-emitter
    // target with a CollisionModule is refused by the child composition.)
    if emitter.collision.is_some() && child_parent.is_none() {
        let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
        if let Err(reason) = crate::particle_runtime::native_birth_eligible(&emitter, &route)
            .and_then(|()| crate::particle_runtime::native_shape_state_eligible(&emitter, Some(evidence))) {
            tally.law_reject.push(format!("{node}: CollisionModule requires the native birth path: {reason}"));
            return None;
        }
    }
    let child_owner = match &child_parent {
        None => None,
        Some(parent) => {
            let evidence = crate::particle_runtime::ShapeEmitterEvidence { scaling, mesh_renderer: render_mode == "Mesh" };
            let owner = crate::particle_runtime::child_target_eligible(&emitter, Some(evidence))
                .and_then(|()| child_owner_words(by_path, node, kind, owner_scaling(scaling)))
                .and_then(|owner| parent_delivers(effect_name, parent, node, by_path, sub_emitter_owners, kind,
                    camera_rotation, lifecycle, native_owner, asset_root, instance_anchor, instance_targets, ground,
                    bodies, server).map(|()| owner));
            match owner {
                Ok((owner, chain)) => {
                    sky_owner_chain = sky_owner_chain.or(chain);
                    Some(owner)
                }
                Err(reason) => {
                    tally.law_reject.push(format!("sub-emitter target {node}: {reason}"));
                    return None;
                }
            }
        }
    };
    // Every other gate passed: the ground scene of the effects the plan
    // installs, bound here or refused by name.
    let collision_scene = match (emitter.collision.as_ref(), ground) {
        (None, _) => None,
        // A Planes module tests its plane slots and never queries the scene.
        (Some(params), _) if params.kind == moly_law::particle::schema::CollisionType::Planes => None,
        (Some(params), Ok(scene)) => match scene.refusal_for(params.collides_with) {
            None => Some(scene.clone()),
            Some(reason) => {
                tally.law_reject.push(format!("{node}: CollisionModule mask {:#x}: {reason}", params.collides_with));
                return None;
            }
        },
        (Some(_), Err(reason)) => {
            tally.law_reject.push(format!("{node}: CollisionModule {reason}"));
            return None;
        }
    };
    Some(Planned {
        culling,
        ordinal: 0,
        event_edges,
        child_owner,
        collision_owner,
        trail_owner,
        sky_owner_chain,
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
            PlannedGeometry::Mesh { reference, glb, alignment: mesh_alignment.expect("validated Mesh alignment"), source: None, scaling, pivot: Vec3::from_array(pivot), flip, axis_body }
        } else if empty_mesh {
            PlannedGeometry::EmptyMesh { alignment: mesh_alignment.expect("validated Mesh alignment"), scaling, pivot: Vec3::from_array(pivot) }
        } else {
            PlannedGeometry::Billboard(crate::source_billboard::Draw {
                mode: match (render_mode, stretch) {
                    ("HorizontalBillboard", _) => crate::source_billboard::Mode::Horizontal,
                    ("VerticalBillboard", _) => crate::source_billboard::Mode::Vertical,
                    ("Stretch", Some(stretch)) => crate::source_billboard::Mode::Stretch(stretch),
                    _ => crate::source_billboard::Mode::Billboard,
                },
                alignment: mesh_alignment.expect("validated source Billboard alignment"),
                screen_size: Vec2::new(min_particle_size as f32, max_particle_size as f32),
                allow_roll, scaling, pivot: Vec3::from_array(pivot),
            })
        },
        cone_angle,
        rol,
        limit,
        trail,
        collision_scene,
        sub_emitter_max_lifetime,
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
/// throughout. With an instance anchor the document's own chain (its nodes
/// by game object id, up to the prefab root) is the evidence here, and the
/// host checks the instance's ancestry when it places the system (see
/// [`crate::particle_runtime::Geometry::keep_unit_chain`]).
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
                unit_chain: if authored_chain { unit_scale_chain(by_path, node) } else { document_unit_chain(by_path, node) },
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

/// Whether the node and every ancestor of a prefab document (linked by
/// `parentGameObjectId`, up to the root, whose parent is 0) carry scale
/// exactly one. A parent the document does not hold is no evidence.
fn document_unit_chain(by_path: &HashMap<String, &Value>, path: &str) -> bool {
    let by_id: HashMap<i64, &Value> = by_path.values()
        .filter_map(|node| Some((node.get("gameObjectId")?.as_i64()?, *node))).collect();
    let Some(mut node) = by_path.get(path).copied() else { return false; };
    for _ in 0..=by_id.len() {
        if triple(node.get("scale")) != Some([1.0; 3]) {
            return false;
        }
        match node.get("parentGameObjectId").and_then(Value::as_i64) {
            Some(0) => return true,
            Some(parent) => match by_id.get(&parent) {
                Some(next) => node = next,
                None => return false,
            },
            None => return false,
        }
    }
    false
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
    (mut environment_root, frame, cannon): (
        ResMut<EnvironmentRoot>,
        Res<bevy::diagnostic::FrameCount>,
        Option<Res<crate::site_move::CannonEnvironmentWrite>>,
    ),
    (gltfs, gltf_nodes, gltf_meshes): (Res<Assets<Gltf>>, Res<Assets<GltfNode>>, Res<Assets<GltfMesh>>),
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
    // GPU readiness gates the preparation, as the source's fade waits for its
    // load. The render resets readiness to pending every frame and sets it
    // ready only when every pass pipeline and the view's colour target are
    // ready; once this plan's preparation has started the fade, a frame that
    // went back to pending does not hold the installs, which follow the
    // source order from there (a failure above still stops them).
    let ready = |source: &SourceParticle| matches!(*source.readiness.lock().unwrap(), ParticleReadiness::Ready);
    if !phase.can_start_site_fx(&plan.selection)
        && !plan.planned.iter().all(|p| ready(&p.source) && p.trail.as_ref().is_none_or(|trail| ready(&trail.source))) { return; }
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
    let delta = crate::particle_runtime::source_delta_time(time.delta());
    let phase_started = plan.site_started_at.is_none();
    // Which source path installs the new effects. At load (the session's
    // first environment: no environment to fade from) the environment view's
    // creation installs the sky effect, then the camera effect, then the site
    // effect in one synchronous run, with the site view passed in. Any later
    // change cross-fades: the site effect's task yields at least once and then
    // waits for the next site's view (up to 5 s), while the fade loop yields
    // at least once when its duration is positive (none when it is zero)
    // before the sky and camera effects are installed.
    let at_load = phase.movement.is_none();
    if phase_started {
        info!("[weather-fx] {} @ {} install path {} (movement {:?}, fade {}s) started frame={}", plan.tier, plan.env_site,
            if at_load { "load" } else { "cross-fade" }, phase.movement, phase.fade_seconds, frame.0);
        retiring.stop_matching(state, now, delta, |kind| kind == EffectKind::Site);
        if state.global_identity != Some(plan.selection.global_effect) {
            // StopSkyEffect runs before PrepareCrossFade. Camera FX deliberately
            // continue until RefreshGlobalEffect at the commit point.
            retiring.stop_matching(state, now, delta, |kind| kind == EffectKind::Sky);
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
    let install_site = !plan.site_installed && controller_ready && (at_load || !phase_started);
    let site_timed_out = !plan.site_installed && !controller_ready && waited >= 5.0;
    if install_site || site_timed_out { plan.site_installed = true; }
    if install_site { retiring.install_colliders(&plan.colliders, |kind| kind == EffectKind::Site); }
    if site_timed_out { warn!("[weather-fx] destination site controller unavailable after source 5s timeout: {}", plan.env_site); }
    let preserve_global = state.global_identity == Some(plan.selection.global_effect) && !state.sky_stopped;
    let install_global = !plan.global_installed && !preserve_global && phase.can_commit_global_fx(&plan.selection)
        && if at_load { plan.site_installed } else { !(phase_started && phase.fade_seconds > 0.0) };
    if install_global && !plan.site_installed {
        info!("[weather-fx] {} @ {} global effects before the site effect frame={}: site scenes ready {}, site matches {}",
            plan.tier, plan.env_site, frame.0, site_ready.is_some(), site.as_ref().is_some_and(|site|
                site.site_id == plan.selection.site_id && site.env_site == plan.selection.environment_site));
    }
    if install_global {
        retiring.stop_matching(state, now, delta, |kind| kind != EffectKind::Site);
        retiring.install_colliders(&plan.colliders, |kind| kind != EffectKind::Site);
        state.global_identity = Some(plan.selection.global_effect);
        state.sky_stopped = false;
    }
    for entity in retiring.despawn.drain(..) { commands.entity(entity).try_despawn(); }
    if preserve_global || install_global { plan.global_installed = true; }
    // The site's own colliders are in the scene from the first install of
    // this site's plan on (a global effect kept across the change included).
    if install_site || install_global || preserve_global {
        let layout = site.as_deref().is_some_and(|active| active.site_type == "home_site");
        let (site, scene) = (plan.env_site.clone(), plan.site_colliders.clone());
        retiring.install_site(&site, &scene, layout);
    }
    if plan.global_installed && phase.can_commit_global_fx(&plan.selection) {
        commands.insert_resource(crate::weather_transition::WeatherGlobalFxCommitted(plan.request_serial));
    }
    let mut waiting = Vec::new();
    environment_root.follow_cannon(cannon.as_deref());
    let owner_anchor = match site.as_deref() {
        Some(active) => {
            environment_root.enter(active);
            environment_root.owner_anchor(active)
        }
        None => Err("owner words on the sky chain: no active site".to_owned()),
    };
    // Clocks belong to instantiated effects. They are shared by their emitters,
    // preserved with unchanged global instances, and move intact into retirement.
    let mut effect_clocks: HashMap<String, Arc<crate::weather_animation::EffectClock>> = HashMap::new();
    let mut effect_animators: HashMap<String, Option<Arc<crate::weather_animation::EffectAnimator>>> = HashMap::new();
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
        // A sky system's owner words from the root this frame stands at.
        let mut planned = planned;
        let sky_owner = match planned.sky_owner_chain.clone() {
            None => None,
            Some(chain) => match owner_anchor.clone()
                .and_then(|anchor| environment_owner(&sky_prefix(anchor), &chain.chain, false, chain.scaling)
                    .map(|owner| (anchor, owner))) {
                Ok((anchor, owner)) => {
                    use moly_law::particle::collision_response::QueryAffine;
                    planned.child_owner = planned.child_owner.map(|_| moly_law::particle::child_emit::ChildOwner::from_owner(&owner));
                    planned.collision_owner = planned.collision_owner.map(|_| moly_law::particle::collision_query::OwnerPair {
                        local_to_world: QueryAffine::from_columns(&owner.local_to_world),
                        world_to_local: QueryAffine::from_columns(&owner.world_to_local),
                    });
                    planned.trail_owner = planned.trail_owner.map(|_| owner.local_to_world);
                    Some(SkyOwner { chain, anchor: anchor.map(f32::to_bits) })
                }
                Err(reason) => {
                    error!(%reason, node=%planned.node, "sky owner words refused: the system is not installed");
                    if let Some((draw, _)) = planned.draw { commands.entity(draw).try_despawn(); }
                    if let Some((draw, _)) = planned.trail.and_then(|trail| trail.draw) { commands.entity(draw).try_despawn(); }
                    continue;
                }
            },
        };
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
        let effect_animator = effect_animators.entry(planned.effect.clone())
            .or_insert_with(|| plan.animators.get(&planned.effect)
                .map(|nodes| Arc::new(crate::weather_animation::EffectAnimator::new(nodes.clone())))).clone();
        let animated_chain = if planned.animated { planned.node_chain } else { None };
        let route = planned.route.clone();
        let frame_clock = crate::particle_runtime::FrameClock::from_use_unscaled_time(
            planned.emitter.use_unscaled_time.expect("weather admission requires useUnscaledTime"));
        // Play's first warm runs through the procedural update only for a
        // looping prewarm system that supports it.
        let procedural_warm = planned.emitter.prewarm && planned.emitter.looping
            && route == crate::particle_runtime::SourceRoute::Procedural;
        let play = lifecycle::PlayState::played(planned.culling, procedural_warm);
        let children = plan.children.get(&planned.effect).cloned()
            .unwrap_or_else(|| panic!("weather effect {} has no child set", planned.effect));
        let event_edges = planned.event_edges;
        let child_owner = planned.child_owner;
        let trail_owner = planned.trail_owner;
        let collision_scene = planned.collision_scene.clone();
        let collision_owner = planned.collision_owner;
        state.live.push(LiveWeatherEmitter { draw, trail_draw, native_refusal: None, lifecycle: planned.lifecycle.expect("weather plans own a source lifecycle"), effect_clock,
            effect_animator, animated_chain, frame_clock, play, children, sky_owner, runtime: Runtime {
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
            pending: 0.0,
            sub_emitter_max_lifetime: planned.sub_emitter_max_lifetime,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            velocity_law: planned.emitter.velocity_over_lifetime.as_ref()
                .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission")),
            force_law: planned.emitter.force.as_ref().map(|p|
                moly_law::particle::force::ForceOverLifetime::from_params(p).expect("force validated during admission")),
            gravity_law: moly_law::particle::gravity::Gravity::new(&planned.emitter.start.gravity_modifier).expect("curves validated during admission"),
            size_law: crate::particle_runtime::size_over_lifetime_law(&planned.emitter)
                .map(|law| law.expect("curves validated during admission")),
            color_law: planned.emitter.color_over_lifetime.as_ref()
                .map(moly_law::particle::color::ColorOverLifetime::from_params),
            custom_law: planned.emitter.custom_data.as_ref()
                .map(|p| crate::particle_runtime::custom_data_law(p).expect("curves validated during admission")),
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
                let has_collision = live.runtime.emitter.collision.is_some();
                // The plan's verdict bound the scene; the module queries the
                // host's live physics scene, which holds the installed and
                // the retiring effects' colliders.
                let collision = match collision_scene {
                    Some(_) => Some(crate::particle_runtime::CollisionInstall {
                        scene: Box::new(crate::particle_runtime::collision_scene::GroundQuery::live(retiring.physics.clone())),
                        owner: collision_owner,
                    }),
                    None if crate::particle_runtime::is_planes(&live.runtime.emitter) =>
                        Some(crate::particle_runtime::CollisionInstall::planes(collision_owner)),
                    None => None,
                };
                match crate::particle_runtime::install_native_birth(&mut live.runtime, &mut seed_manager, &route, collision) {
                    Ok(crate::particle_runtime::BirthPath::Native) if has_collision && live.runtime.collision.is_none() => {
                        error!(node=%live.node, "collision system installed without its collision state");
                        failed = true;
                    }
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
                            collision=live.runtime.collision.is_some(), "weather native birth owner installed");
                    }
                    // A CustomData law that follows the engine's storage runs
                    // only in the native slices, which report it.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason))
                        if live.runtime.custom_law.as_ref().is_some_and(|custom| custom.tracks_storage()) => {
                        error!(%reason, node=%live.node, "CustomData curve-cache system refused by the native birth installer");
                        failed = true;
                    }
                    // So does a size law that follows the engine's calls.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason))
                        if live.runtime.size_law.as_ref().is_some_and(|size| size.calls().is_some()) => {
                        error!(%reason, node=%live.node, "size curve-cache system refused by the native birth installer");
                        failed = true;
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
                    // Nor a collision system left moving through its ground.
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) if has_collision => {
                        error!(%reason, node=%live.node, "collision system refused by the native birth installer");
                        failed = true;
                    }
                    Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => {
                        if live.runtime.emitter.shape.as_ref().is_some_and(shape_needs_native_birth) {
                            // Admission required the native owner for this shape; the
                            // legacy step has no sampler for it, so it is not installed.
                            error!(%reason, node=%live.node, "native-only shape without its native birth owner");
                            failed = true;
                        } else {
                            live.native_refusal = Some(reason);
                        }
                    }
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
        info!("[weather-fx] {} @ {} phase site={} global={} active={} retiring={} frame={} path={}",state.tier,state.env_site,install_site,install_global,state.live.len(),retiring.live.len(),
            frame.0, if at_load { "load" } else { "cross-fade" });
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
/// is kept across the move). The product hides their draws here, and
/// [`advance`] stops stepping their particle systems while the site is indoor;
/// back outdoors they resume from the state they stopped in (freeze and
/// resume), while their Animators and effect clocks keep running indoors. What
/// the engine does to a particle system and an Animator whose GameObject is
/// deactivated and activated again (for example stopping and clearing the
/// particles and replaying on activation) has not been read: an unverified
/// residual. Retiring emitters are no longer referenced by those three owners
/// and keep their own lifecycle.
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

/// How the engine updates one installed system in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FrameUpdate {
    /// Neither listed in the manager nor collected by a parent's update.
    Skipped,
    /// Listed and not a target of an updated parent: it updates itself with
    /// the frame's delta.
    Root,
    /// A target of an updated parent: that update collects it, listed or not,
    /// with the frame's delta when its parent or itself was playing at the
    /// end of the last frame and with none otherwise.
    Collected { delta: bool },
}

/// Each system's update this frame. The engine marks, from every listed
/// system, the sub-emitter targets below it; the update roots are the listed
/// systems left unmarked, and each root's update collects its targets (and
/// theirs) whether they are listed or not, reading both play words before
/// anything of this frame changes them. Scheduling a collected target's job
/// plays it again and lists it; the frame's end unlists any system that holds
/// no particle once its emission has stopped. So a target runs whenever its
/// parent runs, and after its parent leaves the manager it runs on its own
/// until its last particle dies. A system refused this frame is not updated,
/// and neither is what only its update would collect.
fn frame_updates(systems: &[(&mut LiveWeatherEmitter, bool)], refused: &[Entity]) -> Vec<FrameUpdate> {
    // Each target has one parent in its effect instance.
    let parents: Vec<Option<usize>> = systems.iter().enumerate().map(|(index, (live, _))| {
        if !live.native_birth.as_ref().is_some_and(|birth| birth.target.is_some()) {
            return None;
        }
        systems.iter().enumerate().position(|(other, (parent, _))| other != index
            && Arc::ptr_eq(&parent.effect_clock, &live.effect_clock)
            && direct_targets(&parent.runtime).iter().any(|target| *target == live.node))
    }).collect();
    fn resolve(index: usize, systems: &[(&mut LiveWeatherEmitter, bool)], refused: &[Entity],
        parents: &[Option<usize>], memo: &mut [Option<FrameUpdate>], depth: usize) -> FrameUpdate {
        if let Some(update) = memo[index] {
            return update;
        }
        let live = &systems[index].0;
        let update = if refused.contains(&live.draw) {
            FrameUpdate::Skipped
        } else {
            // A chain longer than the systems is a cycle, which the edges
            // never form; it is read as no parent.
            let parent = parents[index].filter(|_| depth < systems.len())
                .filter(|&parent| resolve(parent, systems, refused, parents, memo, depth + 1) != FrameUpdate::Skipped);
            match parent {
                Some(parent) => FrameUpdate::Collected { delta: systems[parent].0.play.playing() || live.play.playing() },
                None if live.play.managed() => FrameUpdate::Root,
                None => FrameUpdate::Skipped,
            }
        };
        memo[index] = Some(update);
        update
    }
    let mut memo = vec![None; systems.len()];
    (0..systems.len()).map(|index| resolve(index, systems, refused, &parents, &mut memo, 0)).collect()
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

/// The first Play of looping prewarm system `parent`, which the engine runs
/// inside Play as one frame before any ordinary one. The parent's
/// sub-emitter targets are not played; the warm frame gathers them: each
/// direct target of the same effect instance first runs its own update of
/// the warm's length (stopped, with its own speed and duration, so its clock
/// and pending time move and nothing is emitted), then the parent's warm,
/// whose slices record their sub-emitter events, then those events'
/// commands, handed to the targets in the order recorded. A command whose
/// catch-up has reached the target's lifetime does nothing, so only the last
/// slices' commands give particles, and those are first simulated by the
/// target's first ordinary frame. A target of a target takes no time in the
/// warm frame.
fn first_play_warm(systems: &mut [(&mut LiveWeatherEmitter, bool)], parent: usize, ctx: &Context,
    refused: &mut Vec<Entity>) {
    let warm_dt = crate::particle_runtime::first_play_plan(&systems[parent].0.runtime).map(|plan| plan.compute_out());
    if let Ok(warm_dt) = warm_dt {
        let clock = systems[parent].0.effect_clock.clone();
        let targets = direct_targets(&systems[parent].0.runtime);
        for (live, emitting) in systems.iter_mut() {
            if Arc::ptr_eq(&live.effect_clock, &clock) && targets.iter().any(|target| *target == live.node)
                && live.native_birth.as_ref().is_some_and(|birth| birth.target.is_some()) {
                if let Err(reason) = crate::particle_runtime::advance_frame(&mut live.runtime, warm_dt, *emitting, ctx, |_| {}) {
                    error!(%reason, effect=%live.effect, node=%live.node,
                        "native particle step refused in its parent's warm: the system is retired and draws nothing");
                    refused.push(live.draw);
                }
            }
        }
    }
    let system = &mut systems[parent].0.runtime;
    if let Err(error) = crate::particle_runtime::prewarm_first_play(system, ctx) {
        error!(%error, effect=%system.effect, node=%system.node, "weather prewarm refused");
    }
    if let Ok(warm_dt) = warm_dt {
        deliver_sub_emitter_commands(systems, warm_dt);
    }
}

/// Every child a system's edges name: its birth and death edges and its
/// CollisionModule's.
fn direct_targets(system: &Runtime) -> Vec<String> {
    let mut targets: Vec<String> = system.native_birth.as_ref().and_then(|birth| birth.events.as_ref())
        .map(|events| events.slots().iter().map(|slot| slot.edge.target.clone())
            .chain(events.death_slots().iter().map(|slot| slot.edge.target.clone())).collect())
        .unwrap_or_default();
    if let Some(collision) = system.collision.as_ref() {
        targets.extend(collision.edges.iter().map(|slot| slot.target.clone()));
    }
    targets
}

/// The source's environment root, tracked in the source world across site
/// changes. The global sky effects hang from it.
///
/// The source instantiates the sky prefab under `Sky/EffectRoot` below the
/// environment view controller, a child of the site root. The whole chain is
/// authored at the identity, and the site root is instantiated without a
/// parent. While the player walks, nothing writes that chain: the sky view
/// moves only `SkyRenderer`, the effect root's sibling, to the player's view,
/// and the effect's own component rewrites only its rotation, to cancel its
/// parent's, which changes something only under the camera. So the global
/// effects do not follow a walking player.
///
/// One writer moves the controller: the cannon site move. For the flight's
/// duration it sets the controller's position to the player view's step
/// point every tween update. The player view is tweened onto the next site's
/// arrival point over the same duration, and the move then restores the
/// height the controller had before the flight. The controller ends at the
/// arrival point's x and z, and at its old height. Door moves (home to a room,
/// a room to home or to another room) do not touch the controller. The session
/// starts at home, with the controller at the site root's origin.
///
/// Arrival points: a harvest site's is its site position. A delivery site's
/// is its site position plus an offset authored on the site view, and home's
/// is the house's inside-door point, which follows the player's layout. The
/// cannon move publishes its writes (`site_move::CannonEnvironmentWrite`):
/// the root keeps its height at the fire, takes each follow point, and gets
/// the kept height back at the landing effect. Without those writes for the
/// entered site this host stands in, and says so: on a delivery site the root
/// lands on the site origin, and on the way home it returns to where the
/// session started it, home's origin. The source has no direct move between a
/// room and a harvest or delivery site; its route runs through home, and this
/// host applies that route.
///
/// The camera effects are children of the rendering camera's own transform.
/// The site-unique effects are children of the site view, the field prefab's
/// root at the site origin. Their anchors are the camera and the identity.
///
/// The weather host's installer and frame both read it; whichever sees a
/// site first applies the move ([`EnvironmentRoot::enter`] is idempotent for
/// a site already seen).
#[derive(Resource, Default)]
pub(crate) struct EnvironmentRoot {
    /// Source-world position.
    world: Vec3,
    /// Site type and category of the site the root was last seen from; `None`
    /// before the first site of the session.
    seen: Option<(String, String)>,
    /// The arrival point this host stood in for at the root's last landing
    /// (the missing value and the stand-in); `None` while the root stands
    /// where the source puts it.
    stand_in: Option<(&'static str, &'static str)>,
    /// The cannon move whose writes the root follows.
    cannon: Option<CannonFollow>,
}

struct CannonFollow {
    move_id: u64,
    /// The destination's site type.
    site_type: String,
    /// `envDefaultPositionY`: the root's height at the fire.
    kept_y: f32,
    /// The kept height is back (logged once).
    landed: bool,
}

impl EnvironmentRoot {
    /// Apply the source's move from the last seen site to `active`, logging
    /// where this host stands in for an arrival point.
    fn enter(&mut self, active: &SiteActive) {
        let landed = self.enter_move(active);
        if let Some(landed) = landed {
            self.stand_in = landed;
            if let Some((missing, stand_in)) = landed {
                warn!(
                    "[weather-fx] sky anchor on {}: {missing} is not supplied; the environment root lands on {stand_in} instead",
                    active.site_type
                );
            }
        }
    }

    /// The owner-word position of the root: the controller's position in
    /// source axes seen from the active site (the frame this host draws the
    /// active site and its colliders in). Refused while the root stands in
    /// for an arrival point.
    fn owner_anchor(&self, active: &SiteActive) -> Result<[f32; 3], String> {
        if let Some((missing, stand_in)) = self.stand_in {
            return Err(format!("owner words on the sky chain: {missing} is not supplied (the environment root stands on {stand_in})"));
        }
        Ok((self.world - Vec3::from_array(active.position)).to_array())
    }

    /// The source's move from the last seen site to `active`: `None` when the
    /// root does not land (the same site, a door move, the session's first
    /// site); otherwise the landing, with the missing value and the stand-in
    /// when this host stands in for the arrival point.
    fn enter_move(&mut self, active: &SiteActive) -> Option<Option<(&'static str, &'static str)>> {
        const HOME: &str = "housing_home";
        const ROOM: &str = "housing_room";
        const HARVEST: &str = "harvest";
        const DELIVERY: &str = "delivery";
        let seen = (active.site_type.clone(), active.category.clone());
        let from = match self.seen.replace(seen) {
            Some((site, _)) if site == active.site_type => return None,
            Some((_, category)) => category,
            None => HOME.to_owned(),
        };
        // A cannon move into this site writes the root itself: it lands where
        // the source puts it.
        if self.cannon.as_ref().is_some_and(|cannon| cannon.site_type == active.site_type) {
            return Some(None);
        }
        let from_cannon_site = matches!(from.as_str(), HARVEST | DELIVERY);
        let site = Vec3::from_array(active.position);
        match active.category.as_str() {
            HARVEST => {
                self.land(site);
                Some(None)
            }
            DELIVERY => {
                self.land(site);
                Some(Some(("the delivery site's arrival offset", "the site origin")))
            }
            HOME | ROOM if from_cannon_site => {
                self.land(Vec3::ZERO);
                Some(Some(("the house's inside-door point", "the starting point")))
            }
            _ => None,
        }
    }

    /// Follow the cannon move's writes: keep the height at the fire, take
    /// each follow point, and set the kept height back at the landing effect.
    /// Idempotent within a frame, so both readers of the root may call it.
    fn follow_cannon(&mut self, write: Option<&crate::site_move::CannonEnvironmentWrite>) {
        let Some(write) = write else {
            self.cannon = None;
            return;
        };
        if self.cannon.as_ref().is_none_or(|cannon| cannon.move_id != write.move_id) {
            self.cannon = Some(CannonFollow {
                move_id: write.move_id,
                site_type: write.site_type.clone(),
                kept_y: self.world.y,
                landed: false,
            });
        }
        let cannon = self.cannon.as_mut().expect("set above");
        let Some(point) = write.point else {
            return;
        };
        if write.height_restored {
            self.world = Vec3::new(point.x, cannon.kept_y, point.z);
            if !cannon.landed {
                cannon.landed = true;
                info!(
                    "[weather-fx] environment root follows the cannon move to {}: lands at {:.3}",
                    cannon.site_type, self.world
                );
            }
        } else {
            self.world = point;
        }
    }

    /// The end of a cannon move: the arrival point's x and z, the old height.
    fn land(&mut self, arrival: Vec3) {
        self.world = Vec3::new(arrival.x, self.world.y, arrival.z);
    }

    /// The sky anchor in this host's frame: the root seen from the active
    /// site, which this host draws at its own origin, in the reflected-X basis
    /// that every scene anchor uses.
    fn anchor(&self, active: &SiteActive) -> Vec3 {
        crate::particle_geometry::reflect(self.world - Vec3::from_array(active.position))
    }
}

/// A sky system's owner words at the root `anchor` stands at: recomposed when
/// the root moved since they were last composed, and written where the
/// system reads them (the collision query, the trail job, a target's child
/// owner). Systems without sky owner words are left alone.
fn refresh_sky_owner(live: &mut LiveWeatherEmitter, anchor: &Result<[f32; 3], String>) -> Result<(), String> {
    let Some(sky) = live.sky_owner.as_mut() else { return Ok(()); };
    let anchor = anchor.clone()?;
    let bits = anchor.map(f32::to_bits);
    if sky.anchor == bits {
        return Ok(());
    }
    let owner = environment_owner(&sky_prefix(anchor), &sky.chain.chain, false, sky.chain.scaling)?;
    sky.anchor = bits;
    write_owner_words(&mut live.runtime, &owner)
}

/// Write owner words into every consumer the installed system has.
fn write_owner_words(runtime: &mut Runtime, owner: &moly_law::particle::owner::OwnerMatrices) -> Result<(), String> {
    use moly_law::particle::collision_response::QueryAffine;
    if let Some(collision) = runtime.collision.as_mut().filter(|collision| collision.owner.is_some()) {
        collision.owner = Some(moly_law::particle::collision_query::OwnerPair {
            local_to_world: QueryAffine::from_columns(&owner.local_to_world),
            world_to_local: QueryAffine::from_columns(&owner.world_to_local),
        });
    }
    if runtime.trail.as_ref().is_some_and(|trail| trail.owner.is_some()) {
        crate::particle_runtime::attach_trail_owner(runtime, owner.local_to_world).map_err(str::to_owned)?;
    }
    if let Some(target) = runtime.native_birth.as_mut().and_then(|native| native.target.as_mut()) {
        target.owner = moly_law::particle::child_emit::ChildOwner::from_owner(owner);
    }
    Ok(())
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
    unscaled: Option<Res<crate::particle_runtime::UnscaledFrameClock>>,
    frame: Res<bevy::diagnostic::FrameCount>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    site: Option<Res<SiteActive>>,
    mut environment_root: ResMut<EnvironmentRoot>,
    cannon: Option<Res<crate::site_move::CannonEnvironmentWrite>>,
    mut camera_speed: Local<billboard::CameraVelocity>,
) {
    // Follow every site change, also on frames that draw nothing, so the root
    // sees each move the source would make.
    environment_root.follow_cannon(cannon.as_deref());
    if let Some(active) = site.as_deref() {
        environment_root.enter(active);
    }
    // Observe instance age even when no camera can produce a particle draw.
    // The effect Animators evaluate here too, once per frame with the frame's
    // delta time: before the frame's particle update, and whether or not a
    // camera can draw (culling mode "always animate").
    let now = time.elapsed_secs_f64();
    // Time.deltaTime: the frame clamped at Time.maximumDeltaTime and floored
    // at 1e-5 s, rounded once to float. The virtual clock carries the same
    // maximum, so its delta and the real frame give the same value here.
    let dt = crate::particle_runtime::source_delta_time(time.delta());
    for live in state.as_deref().into_iter().flat_map(|state| &state.live)
        .chain(retiring.live.iter().map(|entry| &entry.emitter)) {
        live.effect_clock.observe(now);
        if let Some(animator) = &live.effect_animator {
            animator.advance_frame(frame.0, dt);
        }
    }
    // The camera's velocity follows every render, whether or not a system draws.
    let camera_velocity = cameras.iter().next().map_or(Vec3::ZERO, |(t, ..)| camera_speed.update(t.translation(), dt));
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
    let mut basis = billboard::basis_from_matrix(
        camera_transform.affine().matrix3.into(),
        camera_transform.translation(),
        perspective.fov,
        viewport.x as f32 / viewport.y.max(1) as f32,
        perspective.near,
    );
    basis.velocity = camera_velocity;
    // Without an active site the frame the anchors are expressed in does not
    // exist (a site switch is in progress); skip the frame like a missing camera.
    let Some(active_site) = site.as_deref() else {
        return;
    };
    let sky = GlobalTransform::from_translation(environment_root.anchor(active_site));
    let ctx = Context {
        sky,
        camera: *camera_transform,
        site: GlobalTransform::IDENTITY,
    };
    let pass = lifecycle::CameraPass::new(camera_transform, camera, perspective.near, perspective.far);
    // The engine recomputes a system's owner words before the frame's first
    // particle update whenever its transform changed: a sky system's follow
    // the root this frame stands at. A root this host stands in for refuses
    // them.
    let owner_anchor = environment_root.owner_anchor(active_site);
    let mut owner_refused = Vec::new();
    for live in state.as_deref_mut().into_iter().flat_map(|state| state.live.iter_mut())
        .chain(retiring.live.iter_mut().map(|entry| &mut entry.emitter)) {
        if let Err(reason) = refresh_sky_owner(live, &owner_anchor) {
            error!(%reason, effect=%live.runtime.effect, node=%live.runtime.node,
                "sky owner words refused: the system is retired and draws nothing");
            owner_refused.push(live.draw);
        }
    }

    // Indoors the three effect owners are inactive (see refresh_effect_visible).
    let live_active = !site.as_deref().is_some_and(SiteActive::is_indoor);
    let mut systems: Vec<(&mut LiveWeatherEmitter, bool)> = state.as_deref_mut().into_iter()
        .filter(|_| live_active)
        .flat_map(|s| s.live.iter_mut()).map(|s| (s, true))
        .chain(retiring.live.iter_mut().map(|s| (&mut s.emitter, false)))
        .collect();
    // Systems whose native step was refused this frame; see step_frame.
    let mut refused = owner_refused;
    // 惰性 prewarm：首个推进帧快进原版首次 Play 的暖机窗口（只动仿真
    // 状态不喂渲染，快进里的世界空间出生锚在首帧锚点上）。窗口长度与
    // 起始时钟取原生 Compute/Update1b 的值；Play 只在 prewarm 且循环时
    // 暖机（非循环 prewarm 原生不暖机，不是缺口）。Every warm, with its
    // targets' updates and its commands, lands before any system's first
    // ordinary frame.
    for index in 0..systems.len() {
        let warms = {
            let (live, emitting) = &mut systems[index];
            let system = &mut live.runtime;
            if !*emitting || system.prewarmed {
                continue;
            }
            system.prewarmed = true;
            system.emitter.prewarm && system.emitter.looping
        };
        if warms {
            first_play_warm(&mut systems, index, &ctx, &mut refused);
        }
    }
    // Which systems the engine updates this frame, and how (frame_updates).
    let updates = frame_updates(&systems, &refused);
    // Collected targets whose frame end and culling pass wait for this
    // frame's commands (see below).
    let mut collected_ends = Vec::new();
    for (index, (live, emitting)) in systems.iter_mut().enumerate() {
        let emitting = *emitting;
        let LiveWeatherEmitter { draw, runtime: system, effect_animator, animated_chain, frame_clock, play, .. } = &mut **live;
        let draw = *draw;
        // A target refused in a parent's warm is retired below; it is not
        // stepped again.
        if refused.contains(&draw) {
            continue;
        }
        // The first Play warms inside the instantiating call, before any
        // Animator write, so the prewarm above saw the serialized chain. The
        // Animator's rotation of this frame lands before the particle update
        // and is what rendering reads.
        if let (Some(chain), Some(animator)) = (animated_chain.as_ref(), effect_animator.as_ref()) {
            system.node_affine = chain.affine(|path| animator.rotation(path));
        }
        play.played_bounds(system, &compose_to_world(system, &ctx));
        // A culled or stopped system that no parent's update collects is out
        // of the manager: no update, no bounds, no play-state transition; it
        // keeps its particles.
        let delta = match updates[index] {
            FrameUpdate::Skipped => None,
            FrameUpdate::Root => Some(true),
            FrameUpdate::Collected { delta } => {
                play.keep_updating(now);
                Some(delta)
            }
        };
        if let Some(delta) = delta {
            let frame_dt = match frame_clock {
                crate::particle_runtime::FrameClock::Scaled => Some(dt),
                crate::particle_runtime::FrameClock::Unscaled => unscaled.as_deref().map(|clock| clock.delta()),
            };
            match frame_dt {
                Some(frame_dt) => {
                    let step_dt = if delta { frame_dt } else { 0.0 };
                    match crate::particle_runtime::advance_frame(system, step_dt, emitting, &ctx, |s| play.slice_start(s, now)) {
                        Ok(true) => play.update_bounds(system, &compose_to_world(system, &ctx)),
                        Ok(false) => {}
                        Err(reason) => {
                            error!(%reason, effect=%system.effect, node=%system.node,
                                "native particle step refused: the system is retired and draws nothing");
                            refused.push(draw);
                            continue;
                        }
                    }
                }
                None => {
                    system.refused_total += 1;
                    error!(effect=%system.effect, node=%system.node, "useUnscaledTime system has no unscaled clock this frame");
                }
            }
            // Every updated system is stepped in this frame, the frame its
            // update job is scheduled in; Update2 then needs a non-zero delta
            // of the system's clock (the clock's, not the collected one's).
            // Without a clock no update ran.
            let update2 = frame_dt.is_some_and(|delta| delta != 0.0);
            // A collected target ends its frame after its parents' commands of
            // this frame reached it: the engine's frame end counts the births
            // they gave it.
            if matches!(updates[index], FrameUpdate::Collected { .. }) {
                collected_ends.push((index, update2));
                continue;
            }
            play.end_update(system, now, update2);
        }
        // The frame's culling pass: the renderer's world box from the last
        // bounds and the camera's planes, before this frame's geometry (the
        // order against the renderer's geometry job was not read).
        // Residual: a culled system has left the engine's pass and is not
        // drawn there; here its frozen particles are still written and drawn
        // wherever the Bevy camera does not clip them (the source far plane
        // and per-layer cull distances are not applied to the draw).
        play.render_pass(system, &compose_to_world(system, &ctx), &pass, now);
    }
    // Every target's own frame has run: the parents' commands of this frame
    // now reach their targets, and the geometry below shows their births.
    deliver_sub_emitter_commands(&mut systems, dt);
    for (index, update2) in collected_ends {
        let LiveWeatherEmitter { runtime: system, play, .. } = &mut *systems[index].0;
        play.end_update(system, now, update2);
        play.render_pass(system, &compose_to_world(system, &ctx), &pass, now);
    }
    for (live, _) in systems.iter_mut() {
        // A system refused this frame is retired below and draws nothing.
        if refused.contains(&live.draw) {
            continue;
        }
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
    let collision: Vec<(&str, bool, u64, u64, u64)> = state.as_deref().into_iter().flat_map(|state| &state.live)
        .map(|s| (s, true)).chain(retiring.live.iter().map(|entry| (&entry.emitter, false)))
        .filter_map(|(s, emitting)| s.collision.as_ref()
            .map(|collision| (s.node.as_str(), emitting, collision.calls, collision.hits, collision.order_free)))
        .collect();
    if !collision.is_empty() {
        info!("[weather-fx] collision: physics scene colliders {} ({} installed effects, {} retiring, site {:?}, placed fixtures {}); per system (node, emitting, calls, hits, order-free lanes) {:?}",
            retiring.physics.collider_count(), retiring.installed_colliders.len(), retiring.retiring_colliders.len(),
            retiring.site_colliders.as_ref().map(|(site, _)| site.as_str()), retiring.fixture_colliders.is_some(), collision);
    }
    if !retiring.live.is_empty() {
        info!("[weather-fx] retiring systems={}, live particles={}, active systems={}, destroy loops={:?}",
            retiring.live.len(), retiring.live.iter().map(|s| s.emitter.pool.len()).sum::<usize>(),
            state.as_ref().map_or(0, |s| s.live.len()),
            retiring.instances.iter().map(|i| i.destroy.report()).collect::<Vec<_>>());
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
        if let Some(mut retiring) = world.get_resource_mut::<WeatherFxRetirements>() {
            retiring.live.clear();
            retiring.instances.clear();
            retiring.despawn.clear();
            retiring.clear_colliders();
        }
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

#[cfg(test)]
#[path = "weather_sub_emitter_chain_tests.rs"]
mod sub_emitter_chain_tests;

#[cfg(test)]
#[path = "weather_scene_reach.rs"]
mod scene_reach;

#[cfg(test)]
#[path = "weather_sky_owner_receipt.rs"]
mod sky_owner_receipt;
