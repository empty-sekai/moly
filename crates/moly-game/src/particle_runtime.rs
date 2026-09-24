//! Shared particle simulation, module evaluation and world-space presentation.
//! Domain adapters own selection, asset loading, instance anchors and teardown.
use bevy::prelude::*;
mod motion;
mod birth;
mod child;
mod sub_events;
mod trails;
mod collision;
pub(crate) use collision::{collision_eligible, COLLISION_SCENE_NOT_PORTED};
pub(crate) use trails::{draw_eligible as trail_draw_eligible, write_mesh as write_trail_mesh, TrailState};
pub(crate) use sub_events::{BirthEdge, BirthEvents, DeathEdge, EventEdges};
pub(crate) use child::{child_target_eligible, deliver_command, install_child_target};
pub(crate) mod seed;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod motion_samples;
#[cfg(test)]
mod force_samples;
#[cfg(test)]
mod texture_sheet_samples;
#[cfg(test)]
mod sort_samples;
#[cfg(test)]
mod ring_samples;
#[cfg(test)]
mod gravity_samples;
#[cfg(test)]
mod custom_samples;
#[cfg(test)]
mod birth_samples;
#[cfg(test)]
mod source_birth_samples;
#[cfg(test)]
mod shape_owner_samples;
#[cfg(test)]
mod shape_birth_samples;
#[cfg(test)]
mod snow_full_samples;
#[cfg(test)]
mod noise_samples;
#[cfg(test)]
mod initial_colour_samples;
#[cfg(test)]
mod frame_samples;
#[cfg(test)]
mod sub_event_samples;
#[cfg(test)]
mod death_event_samples;
#[cfg(test)]
mod child_emit_samples;
#[cfg(test)]
mod collision_samples;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::shape::{circle_base, cone_base, cone_volume, donut_position, hemisphere_position, single_sided_edge, sphere_position};
use moly_law::particle::{accumulate_rate, advance_lifetime, burst_check,
    birth_capacity, compact_with_sides_indexed, euler_rotate_deg, finish_births, finish_births_with,
    integrate, BurstOutcome, DragSize,
    EmissionState, EmitterParams, LimitVelocity, Particle,
    RingBufferMode, RotationOverLifetime, StepVerdict};
use moly_law::particle::noise::{NoiseLaw, NoiseState};
use moly_law::particle::death_event::DeathParent;
use moly_law::particle::prewarm::{FirstPlayWarm, Lifetime, PlayState, PrewarmPlan};
use crate::billboard::{Alignment, Quad, SizeClamp};
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

pub(crate) const PREWARM_STEP: f32 = 1.0 / 60.0;

/// Current JP player TimeManager as serialized in globalgamemanagers
/// (Fixed Timestep 0.02, Maximum Particle Timestep 0.03). The game assembly
/// reads Time.fixedDeltaTime and never assigns it or the particle maximum.
pub(crate) const PLAYER_TIME: moly_law::particle::prewarm::TimeManagerSnapshot =
    moly_law::particle::prewarm::TimeManagerSnapshot {
        fixed_timestep: f32::from_bits(0x3ca3_d70a),
        maximum_particle_timestep: f32::from_bits(0x3cf5_c28f),
    };

/// Native update route the source selects for a system. Ordinary systems
/// advance through the incremental Update1 path this runtime transcribes.
/// Procedural systems (DetermineSupportsProcedural true) are evaluated from
/// time and prewarm through Update(flags=3); neither is transcribed, so they
/// stay on the legacy step. `Undecided` names the control the exported block
/// cannot settle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceRoute {
    Ordinary,
    Procedural,
    Undecided(String),
}

/// Transcription of the native DetermineSupportsProcedural decision over the
/// exported system block. The native body reads fixed fields and has no side
/// effects, so the order of the tests does not matter: one failing control makes
/// the route ordinary, and only a block passing all of them is procedural. An
/// input the export does not carry makes the verdict undecided, never a default.
/// Curve validity follows IsValidPolynomialCurve: no keys is valid; otherwise the
/// first key must sit at time 0 or the pre-wrap mode be 2 or more, the last key
/// at time 1 or the post-wrap mode be 2 or more, and the segment count (one per
/// key gap plus one per clamped end) must be fewer than nine.
pub(crate) fn source_route(system: &serde_json::Value) -> SourceRoute {
    use serde_json::Value;
    fn number(value: &Value) -> Option<f64> {
        value.as_f64().or_else(|| match value.as_str() {
            Some("Infinity") => Some(f64::INFINITY),
            Some("-Infinity") => Some(f64::NEG_INFINITY),
            _ => None,
        })
    }
    // Upper lane of a serialized MinMaxCurve: the constant, the two-constant
    // maximum, or the curve multiplier.
    fn upper_scalar(curve: Option<&Value>) -> Option<f64> {
        let curve = curve?;
        match curve.get("mode").and_then(Value::as_str)? {
            "constant" => number(curve.get("value")?),
            "twoConstants" => number(curve.get("max")?),
            "curve" | "twoCurves" => curve.get("multiplier").and_then(number),
            _ => None,
        }
    }
    // One curve lane: its keys and, when exported, its wrap modes.
    fn lane_valid(curve: &Value, keys: &str, pre: &str, post: &str) -> Option<bool> {
        let keys = curve.get(keys)?.as_array()?;
        if keys.is_empty() {
            return Some(true);
        }
        let time = |key: &Value| key.get("time").and_then(Value::as_f64);
        let first_open = time(&keys[0])? != 0.0;
        let last_open = time(&keys[keys.len() - 1])? != 1.0;
        let wrap = |name: &str| curve.get(name).and_then(Value::as_u64);
        if first_open && wrap(pre)? < 2 {
            return Some(false);
        }
        if last_open && wrap(post)? < 2 {
            return Some(false);
        }
        Some(keys.len() - 1 + usize::from(first_open) + usize::from(last_open) < 9)
    }
    fn curve_valid(curve: Option<&Value>) -> Option<bool> {
        let curve = curve?;
        match curve.get("mode").and_then(Value::as_str)? {
            "constant" | "twoConstants" => Some(true),
            "curve" => lane_valid(curve, "keys", "preInfinity", "postInfinity"),
            "twoCurves" => {
                if !lane_valid(curve, "maxKeys", "maxPreInfinity", "maxPostInfinity")? {
                    return Some(false);
                }
                lane_valid(curve, "minKeys", "minPreInfinity", "minPostInfinity")
            }
            _ => None,
        }
    }
    let mut reasons: Vec<String> = Vec::new();
    let mut undecided: Vec<String> = Vec::new();
    fn curves(label: &str, block: &Value, axes: &[&str], reasons: &mut Vec<String>, undecided: &mut Vec<String>) {
        for axis in axes {
            match curve_valid(block.get(*axis)) {
                Some(false) => reasons.push(format!("{label} {axis} polynomial")),
                None => undecided.push(format!("{label} {axis} not decidable from the export")),
                Some(true) => {}
            }
        }
    }
    let Some(enabled) = system.pointer("/sourceModules/enabled").and_then(Value::as_array) else {
        return SourceRoute::Undecided("sourceModules not exported".into());
    };
    let enabled: Vec<&str> = enabled.iter().filter_map(Value::as_str).collect();
    let has = |module: &str| enabled.contains(&module);
    match system.get("simulationSpace").and_then(Value::as_str) {
        Some("Local") => {}
        Some(_) => reasons.push("simulationSpace".into()),
        None => undecided.push("simulationSpace not exported".into()),
    }
    match system.get("stopAction").and_then(Value::as_i64) {
        Some(0) => {}
        Some(_) => reasons.push("stopAction".into()),
        None => undecided.push("stopAction not exported".into()),
    }
    // Read whether or not the Emission module is enabled; the export carries it
    // outside the emission block for exactly that reason.
    let distance = system.get("emissionRateOverDistance")
        .or_else(|| system.pointer("/emission/rateOverDistance"));
    match upper_scalar(distance) {
        Some(rate) if rate == 0.0 => {}
        Some(_) => reasons.push("rateOverDistance".into()),
        None => undecided.push("rateOverDistance not exported".into()),
    }
    for module in ["ExternalForcesModule", "ClampVelocityModule", "RotationBySpeedModule",
        "CollisionModule", "TriggerModule", "SubModule", "NoiseModule"] {
        if has(module) { reasons.push(module.into()); }
    }
    // Only a per-particle trail fails the test; ribbon trails stay procedural.
    // The Lights module is not read by the decision.
    if has("TrailModule") {
        match system.pointer("/trails/mode").and_then(Value::as_str) {
            Some("perParticle") => reasons.push("TrailModule perParticle".into()),
            Some(_) => {}
            None => undecided.push("trails.mode not exported".into()),
        }
    }
    // Gravity and lifetime are read only while the Initial module is enabled.
    if has("InitialModule") {
        match system.pointer("/start/gravityModifier/mode").and_then(Value::as_str) {
            Some("constant") => {}
            Some(_) => reasons.push("gravityModifier not constant".into()),
            None => undecided.push("gravityModifier not exported".into()),
        }
        match upper_scalar(system.pointer("/start/lifetime")) {
            Some(value) if value == f64::INFINITY => reasons.push("startLifetime +Infinity".into()),
            Some(_) => {}
            None => undecided.push("startLifetime not exported".into()),
        }
    }
    if system.get("shapeEnabled").and_then(Value::as_bool) == Some(true) {
        match system.get("shape").filter(|v| v.is_object()) {
            None => undecided.push("shape not exported".into()),
            Some(shape) => {
                let kind = shape.get("type").and_then(Value::as_str);
                let mode = |key: &str| shape.get(key).and_then(Value::as_str);
                match kind {
                    None => undecided.push("shape type not exported".into()),
                    Some("Cone" | "ConeVolume" | "Circle" | "Donut") => match mode("arcMode") {
                        Some("Random") => {}
                        Some(_) => reasons.push("shape arcMode".into()),
                        None => undecided.push("shape arcMode not exported".into()),
                    },
                    Some("SingleSidedEdge") => match mode("radiusMode") {
                        Some("Random") => {}
                        Some(_) => reasons.push("shape radiusMode".into()),
                        None => undecided.push("shape radiusMode not exported".into()),
                    },
                    Some(_) => {}
                }
            }
        }
    }
    if has("RotationModule") {
        match system.get("rotationOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("rotationOverLifetime not exported".into()),
            Some(rotation) => match rotation.get("separateAxes").and_then(Value::as_bool) {
                None => undecided.push("rotation separateAxes not exported".into()),
                Some(separate) => {
                    let axes: &[&str] = if separate { &["curve", "x", "y"] } else { &["curve"] };
                    curves("rotation", rotation, axes, &mut reasons, &mut undecided);
                }
            },
        }
    }
    if has("VelocityModule") {
        match system.get("velocityOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("velocityOverLifetime not exported".into()),
            Some(velocity) => {
                curves("velocity", velocity, &["x", "y", "z"], &mut reasons, &mut undecided);
                for key in ["orbitalX", "orbitalY", "orbitalZ", "radial"] {
                    match upper_scalar(velocity.get(key)) {
                        Some(value) if value == 0.0 => {}
                        Some(_) => reasons.push(format!("velocity {key}")),
                        None => undecided.push(format!("velocity {key} not exported")),
                    }
                }
            }
        }
    }
    if has("ForceModule") {
        match system.get("forceOverLifetime").filter(|v| v.is_object()) {
            None => undecided.push("forceOverLifetime not exported".into()),
            Some(force) => {
                curves("force", force, &["x", "y", "z"], &mut reasons, &mut undecided);
                match force.get("randomizePerFrame").and_then(Value::as_bool) {
                    Some(true) => reasons.push("force randomizePerFrame".into()),
                    Some(false) => {}
                    None => undecided.push("force randomizePerFrame not exported".into()),
                }
            }
        }
    }
    if !reasons.is_empty() {
        SourceRoute::Ordinary
    } else if !undecided.is_empty() {
        SourceRoute::Undecided(undecided.join(", "))
    } else {
        SourceRoute::Procedural
    }
}

/// First-Play prewarm. The warm length and the starting clock are the native
/// ones (ComputePrewarmStartParameters, then Update1b's speed scale). A system
/// with an installed native birth owner advances through the native
/// incremental slice schedule (1 s / 0.2 s / GetTimeStep pieces, partial birth
/// times per particle). The legacy step births whole-slice cohorts, so it
/// covers the same system time in PREWARM_STEP pieces instead. Play only warms
/// a looping prewarm system; the caller owns that gate.
pub(crate) fn prewarm_first_play(system: &mut Runtime, ctx: &Context) -> Result<(), &'static str> {
    if let Some(mut state) = system.native_birth.take() {
        let result = prewarm_native(system, &mut state, ctx);
        system.native_birth = Some(state);
        return result.map(|()| later_play(system));
    }
    let warm = FirstPlayWarm::from_source(
        first_play_lifetime(&system.emitter)?, PLAYER_TIME, first_play_state(system))?;
    system.playback_head = warm.initial_clock;
    if warm.initial_clock != 0.0 {
        // The window opens mid-cycle: no zero-time burst precedes it.
        system.emission_started = true;
        system.previous_head = warm.initial_clock;
    }
    let mut remaining = warm.total;
    while remaining > 0.0 {
        let step = remaining.min(PREWARM_STEP);
        simulate(system, step, ctx);
        remaining -= step;
    }
    later_play(system);
    Ok(())
}

/// The effector plays every child system more than once within one call (the
/// managed recursion and the effector's own child array both reach it). A later
/// Play finds live particles, so it neither resets seeds nor warms again; it
/// only returns the playback clock, the pending time and the loop count to zero.
/// A warm that left no particle alive would be reset and warmed again by that
/// Play; this runtime does not repeat the warm and says so.
fn later_play(system: &mut Runtime) {
    if system.pool.is_empty() {
        warn!(effect=%system.effect, node=%system.node,
            "first-Play warm left no particle alive; the later Play's reset and second warm are not repeated");
        return;
    }
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    if let Some(native) = system.native_birth.as_mut() {
        native.frame.pending = 0.0;
    }
}

/// The native first-Play slice schedule of an installed ordinary system.
pub(crate) fn first_play_plan(system: &Runtime) -> Result<PrewarmPlan, &'static str> {
    PrewarmPlan::from_source(first_play_lifetime(&system.emitter)?, PLAYER_TIME, first_play_state(system))
}

fn prewarm_native(system: &mut Runtime, state: &mut birth::NativeBirthState, ctx: &Context)
    -> Result<(), &'static str> {
    let mut plan = first_play_plan(system)?;
    system.playback_head = plan.initial_clock();
    for slice in plan.by_ref() {
        let slice = slice?;
        if birth::step_explicit(system, state, slice.duration, false, ctx).is_err() {
            system.refused_total += 1;
            return Err("native prewarm slice refused");
        }
    }
    // The rest below the loop threshold stays pending. The emitter reset set
    // by Play is left for the first frame, which takes its own translation.
    state.frame.pending = plan.remaining();
    Ok(())
}

fn first_play_lifetime(emitter: &EmitterParams) -> Result<Lifetime, &'static str> {
    match emitter.start.lifetime {
        moly_law::particle::MinMaxCurve::Constant(v) => Ok(Lifetime::Constant(v)),
        moly_law::particle::MinMaxCurve::TwoConstants { min, max } => Ok(Lifetime::TwoConstants { min, max }),
        // Curve lifetimes go through CalculateCurveRangesValue, not transcribed.
        _ => Err("curve start lifetime: prewarm range not transcribed"),
    }
}

fn first_play_state(system: &Runtime) -> PlayState {
    let e = &system.emitter;
    PlayState {
        elapsed: 0.0,
        live_count: system.pool.len(),
        // Awake/EmitEffect Play reaches native Play(true); the managed
        // withChildren mapping itself is not separately replayed.
        native_play_bool_argument: true,
        world_playing: true,
        // Only an installed ordinary system reaches the native slice plan;
        // the shared Compute/Update1b arithmetic does not read this flag.
        ordinary_incremental: true,
        no_real_subemitters: e.sub_emitters.iter().all(|edge| edge.source_pointer.is_authored_null()),
        prewarm: e.prewarm,
        looping: e.looping,
        simulation_speed: e.simulation_speed,
        duration: e.duration,
    }
}
/// 三类锚（effect 档案的 `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectKind {
    Sky,
    Camera,
    Site,
}

#[derive(Clone)]
pub(crate) enum Geometry {
    Billboard { alignment: Alignment, clamp: SizeClamp, pivot: [f32; 3] },
    Mesh(crate::particle_geometry::MeshDraw),
    SourceBillboard(crate::source_billboard::Draw),
}

/// Source emitter state the native Shape boundary reads besides the Shape
/// block: the authored MainModule scaling mode and whether the renderer is in
/// Mesh render mode (which allocates the axis-of-rotation channel).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapeEmitterEvidence {
    pub(crate) scaling: crate::particle_geometry::Scaling,
    pub(crate) mesh_renderer: bool,
}
impl Geometry {
    /// The legacy billboard carries no authored scaling mode or render mode.
    pub(crate) fn shape_evidence(&self) -> Option<ShapeEmitterEvidence> {
        match self {
            Self::Billboard { .. } => None,
            Self::Mesh(draw) => Some(ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: true }),
            Self::SourceBillboard(draw) => Some(ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: false }),
        }
    }
}

/// 一条在跑的粒子系统。
pub(crate) struct Runtime {
    pub(crate) node: String,
    pub(crate) effect: String,
    pub(crate) emitter: EmitterParams,
    pub(crate) kind: EffectKind,
    pub(crate) camera_rotation: bool,
    pub(crate) node_affine: GlobalTransform,
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) anchor: Option<Entity>,
    pub(crate) geometry: Geometry,
    pub(crate) emission_surface: Option<std::sync::Arc<crate::particle_mesh_emission::EmissionSurface>>,
    pub(crate) ring_cursor: usize,
    pub(crate) pool: Vec<Particle>,
    pub(crate) side: Vec<Side>,
    pub(crate) emission: EmissionState,
    /// 播头（秒）：率曲线与 burst 的时间轴。
    pub(crate) playback_head: f32,
    /// 上一帧的播头（burst 裁决要一个左端点）。
    pub(crate) previous_head: f32,
    /// The initial zero-time burst belongs to the first positive simulation step.
    pub(crate) emission_started: bool,
    pub(crate) rng: Rng,
    /// Installed only after source configuration and explicit seed ownership
    /// qualify for the native birth path. Retired instances keep this state.
    pub(crate) native_birth: Option<birth::NativeBirthState>,
    /// Qualified current-JP Noise consumer. The source gate still decides
    /// whether a system may be admitted; this state is only installed after
    /// an owner seed/reset has been proven by the caller.
    pub(crate) noise: Option<NoiseRuntime>,
    /// The per-particle trail, installed with the native birth owner when the
    /// system has a qualified TrailModule. Its rings move with the pool.
    pub(crate) trail: Option<TrailState>,
    /// The CollisionModule with its scene, installed with the native birth
    /// owner; `None` for every system without one.
    pub(crate) collision: Option<collision::CollisionRuntime>,
    /// 惰性 prewarm 的闸：首个推进帧快进一个周期。
    pub(crate) prewarmed: bool,
    pub(crate) cone_angle: Option<f32>,
    pub(crate) rol: Option<RotationOverLifetime>,
    pub(crate) limit: Option<LimitVelocity>,
    pub(crate) velocity_law: Option<moly_law::particle::velocity::VelocityOverLifetime>,
    pub(crate) force_law: Option<moly_law::particle::force::ForceOverLifetime>,
    pub(crate) gravity_law: moly_law::particle::gravity::Gravity,
    pub(crate) size_law: Option<moly_law::particle::size::SizeOverLifetime>,
    pub(crate) color_law: Option<moly_law::particle::color::ColorOverLifetime>,
    pub(crate) custom_law: Option<moly_law::particle::custom_data::CustomData>,
    pub(crate) texture_sheet: Option<moly_law::particle::texture_sheet::TextureSheet>,
    pub(crate) sort_mode: moly_law::particle::sort::ParticleSort,
    pub(crate) born_total: u64,
    pub(crate) died_total: u64,
    pub(crate) full_total: u64,
    pub(crate) refused_total: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NoiseRuntime {
    pub(crate) law: NoiseLaw,
    pub(crate) state: NoiseState,
    /// Source ReadOnlyState owner seed, distinct from each particle birth seed.
    pub(crate) owner_seed: u32,
    /// Retain the serialized/automatic owner so a future proven ResetSeeds
    /// event can replace all module streams atomically.
    pub(crate) owner: moly_law::particle::seed_owner::SeedOwner,
}

/// 逐粒子的出生抽定值，与律池同下标平行存。
#[derive(Clone, Copy)]
pub(crate) struct Side {
    /// 出生种子因子（归一 [0,1)）：生命期内稳定求值读它。
    pub(crate) rand: f32,
    /// 自旋/限速族的种子杂凑吃全宽 u32。
    pub(crate) seed: u32,
    /// 自旋状态（弧度；公告板只画 Z 分量）。
    pub(crate) rot: [f32; 3],
    /// Authored birth size on all three axes. Renderer scale is applied in
    /// the geometry transform, not prematurely collapsed into one X factor.
    pub(crate) size: [f32; 3],
    pub(crate) gravity: f32,
    pub(crate) colour: [f32; 4],
    /// Persistent plus animated velocity, before the integration speed modifier.
    pub(crate) total_velocity: [f32; 3],
    /// Persistent pre-simulation Custom1/Custom2, moved with the particle owner.
    pub(crate) custom_data: [[f32; 4]; 2],
    /// Emission carry of each cached sub-emitter birth edge (at most two),
    /// zero at birth and moved with the particle.
    pub(crate) emit_carry: [f32; 2],
    /// Animated velocity of the last pre-simulation pass (runtime axes),
    /// moved with the particle; the collision query and response read it
    /// apart from the persistent velocity.
    pub(crate) animated: [f32; 3],
}

/// 出生抽签的确定性随机：splitmix64（站点链同款流算法、不同种子）。
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }

    /// 全宽 u32（取混合输出的低 32 位，与 next_f32 的高 24 位不同位）。
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z as u32
    }
}

/// 推进帧的锚点快照（天空/相机）。
pub(crate) struct Context {
    pub(crate) sky: GlobalTransform,
    pub(crate) camera: GlobalTransform,
    pub(crate) site: GlobalTransform,
}

// ---- 链头：锚点监视 ----

/// 锚 ∘ 节点链（按 effect 类别选锚）。
pub(crate) fn compose_to_world(system: &Runtime, ctx: &Context) -> GlobalTransform {
    let anchor = match system.kind {
        EffectKind::Sky => ctx.sky,
        EffectKind::Camera => {
            if system.camera_rotation {
                ctx.camera
            } else {
                GlobalTransform::from_translation(ctx.camera.translation())
            }
        }
        EffectKind::Site => ctx.site,
    };
    anchor * system.node_affine
}

/// Apply the world-space owner to a direction after Shape/EmitterStoreData.
///
/// The current JP 6.8.1 native path normalizes inside `EmitterStoreData`
/// before multiplying the outer owner 3x3.  It does **not** normalize again
/// after that final owner multiplication.  Keeping this as a small helper
/// makes the boundary explicit and prevents a renderer-facing `normalize`
/// from silently changing non-uniform owner scales.
#[inline]
fn apply_world_owner_direction(owner: &GlobalTransform, direction: Vec3) -> Vec3 {
    owner.affine().transform_vector3(direction)
}

/// 把律状态换算成公告板批次。
pub(crate) fn build_quads(system: &Runtime, to_world: &GlobalTransform) -> Vec<Quad> {
    let mut quads = Vec::with_capacity(system.pool.len());
    for (index, particle) in system.pool.iter().enumerate() {
        let side = system.side[index];
        // 归一化年龄：律的倒计时折算。
        let age = particle.normalized_age();
        let scale = system.node_affine.to_scale_rotation_translation().0.x;
        let evaluated_size = motion::size_at_age_percent(system, &side, particle.age_percent);
        let size = [evaluated_size[0] * scale, evaluated_size[1] * scale];
        // The source render path multiplies byte colour into immutable birth
        // colour. Keep that operation separate from raw gradient evaluation.
        let colour = system.color_law.as_ref().map_or(side.colour, |law| {
            let birth = moly_law::particle::gradient::quantize_rgba8(side.colour);
            moly_law::particle::gradient::rgba8_to_float(
                law.apply(birth, side.seed, age * 100.0))
        });
        // CustomData is a simulation output. Color is evaluated at render time,
        // but re-evaluating CustomData here would advance it by one simulation step.
        let [custom1, custom2] = side.custom_data;
        quads.push(Quad {
            centre: to_world.transform_point(Vec3::from_array(particle.position)),
            size: Vec2::from_array(size),
            // 自旋状态（弧度）：出生角 + rotationOverLifetime 的积分。
            rotation: side.rot[2],
            colour,
            custom1,
            custom2,
        });
    }
    quads
}

/// One rendered frame of raw dt. A system with the native birth owner runs the
/// engine's per-frame update (speed scale, minimum-step skip, pending time,
/// every incremental slice, emitter-motion placement of World births); the
/// legacy step keeps one whole-frame step of dt times the simulation speed.
/// `emitting` false is Stop: the clock and existing particles still advance.
pub(crate) fn advance_frame(system: &mut Runtime, frame_dt: f32, ctx: &Context, emitting: bool) {
    if let Some(mut native) = system.native_birth.take() {
        // A sub-emitter target never emits on its own: the engine marks every
        // target stopped each frame, so its clock and particles advance and
        // its births come only from its parents' commands.
        let stopped = !emitting || native.target.is_some();
        if let Err(error) = birth::advance_frame(system, &mut native, frame_dt, stopped, ctx) {
            system.refused_total += 1;
            error!(?error, effect=%system.effect, node=%system.node, "native particle frame refused");
        }
        system.native_birth = Some(native);
        return;
    }
    let step = frame_dt * system.emitter.simulation_speed;
    if step > 0.0 {
        if emitting { simulate(system, step, ctx) } else { simulate_stopped(system, step, ctx) }
    }
}

/// 一个仿真步：发射（率 + burst）→ 模块批 → 推进 → 死亡移除。
pub(crate) fn simulate(system: &mut Runtime, dt: f32, ctx: &Context) {
    simulate_with_emission(system, dt, ctx, true);
}

/// Source ParticleSystem.Stop() uses StopEmitting, not StopEmittingAndClear.
/// The system clock and existing particles still advance. The host owns
/// the independent destruction deadline; no prewarm or burst runs after Stop.
pub(crate) fn simulate_stopped(system: &mut Runtime, dt: f32, ctx: &Context) {
    simulate_with_emission(system, dt, ctx, false);
}

fn simulate_with_emission(system: &mut Runtime, dt: f32, ctx: &Context, emitting: bool) {
    if let Some(mut native) = system.native_birth.take() {
        let stopped = !emitting || native.target.is_some();
        if let Err(error) = birth::step_explicit(system, &mut native, dt, stopped, ctx) {
            system.refused_total += 1;
            error!(?error, effect=%system.effect, node=%system.node, "native particle step refused");
        }
        system.native_birth = Some(native);
        return;
    }
    // Current Update1 calls ParticleSystemState::Tick before the stopped gate
    // and before pre-simulation modules. Only zero delay is admitted here:
    // nonzero authored delay cannot substitute for the native remaining wait
    // state, which Stop does not consume. Keep emission carry/RNG untouched.
    if !emitting && dt.is_finite() && dt > 0.0
        && matches!(system.emitter.start_delay, moly_law::particle::MinMaxCurve::Constant(0.0))
        && system.playback_head.is_finite()
        && system.emitter.duration.is_finite() && system.emitter.duration > 0.0
    {
        system.playback_head += dt;
        if system.emitter.looping {
            while system.playback_head >= system.emitter.duration {
                system.playback_head -= system.emitter.duration;
            }
        } else {
            system.playback_head = system.playback_head.min(system.emitter.duration);
        }
    }
    if emitting && dt > 0.0 {
        let emission = system
            .emitter
            .emission
            .as_ref()
            .expect("判读已门 emission 在场")
            .clone();
        system.previous_head = if system.emission_started {
            system.playback_head
        } else {
            // burst_check uses (previous, now]. Include exactly zero on first
            // playback without shifting the source clock or repeating it later.
            system.emission_started = true;
            -f32::from_bits(1)
        };
        system.playback_head += dt;
        // 非循环系统播到头就停发（现存粒子活完寿命）。循环系统的播头按
        // duration 回卷——burst 的时间轴与率曲线共用它。
        if system.emitter.looping && system.emitter.duration > 0.0 {
            while system.playback_head >= system.emitter.duration {
                system.playback_head -= system.emitter.duration;
                system.previous_head -= system.emitter.duration;
            }
        }
        // The source samples the rate at the slice end, in normalized cycle time.
        let rate = emission
            .rate_over_time
            .evaluate(normalized_time(system, system.playback_head), 0.5);
        let mut emitted = accumulate_rate(&mut system.emission, rate, dt);
        for burst in &emission.bursts {
            let rand_fire = system.rng.next_f32();
            let rand_count = system.rng.next_f32();
            match burst_check(
                burst,
                system.previous_head,
                system.playback_head,
                rand_fire,
                rand_count,
            ) {
                BurstOutcome::Fired(count) => emitted += count,
                BurstOutcome::NotDue | BurstOutcome::MissedByProbability => {}
            }
        }
        if !system.emitter.looping && system.previous_head > system.emitter.duration {
            emitted = 0;
        }
        let old_count = system.pool.len();
        let accepted = birth_capacity(old_count, system.emitter.ring_buffer_mode,
            system.emitter.max_particles as usize, emitted as usize);
        system.full_total += (emitted as usize - accepted) as u64;
        for _ in 0..accepted {
            spawn_one(system, ctx);
        }
        finish_births(&mut system.pool, &mut system.side, &mut system.ring_cursor,
            system.emitter.ring_buffer_mode, system.emitter.max_particles as usize, old_count,
            |_, _| system.died_total += 1);

    }

    simulate_existing(system, dt, ctx, None);
}

/// Existing-particle pre/sim/death barrier. Autonomous births are separate.
/// `deaths`, when given, receives every killed particle in removal order as
/// the death event reads it, before its slot is overwritten.
fn simulate_existing(system: &mut Runtime, dt: f32, ctx: &Context, mut deaths: Option<&mut Vec<DeathParent>>) {
    simulate_range(system, 0, system.pool.len(), dt, None, None, ctx);
    let (mode, maximum) = (system.emitter.ring_buffer_mode, system.emitter.max_particles as usize);
    let mut on_death = |index: usize, particle: &Particle, side: &Side| {
        if let Some(deaths) = deaths.as_mut() {
            deaths.push(sub_events::dying(index, particle, side));
        }
    };
    // A dying particle's trail points go with it: the engine moves every
    // per-particle array of the last particle into the dead slot.
    system.died_total += match system.trail.as_mut() {
        Some(trail) => compact_with_sides_indexed(&mut system.pool, &mut system.side, &mut trail.rings, mode, maximum,
            &mut on_death),
        None => {
            let mut none = vec![(); system.pool.len()];
            compact_with_sides_indexed(&mut system.pool, &mut system.side, &mut none, mode, maximum, &mut on_death)
        }
    } as u64;
}

/// Run module math before packing. Birth lanes use times relative to their own
/// aligned native block, so an unaligned old prefix cannot supply a batch seed.
/// Newborn inline integration differs from the existing SimulateParticles path.
fn simulate_range(system: &mut Runtime, start: usize, end: usize, dt: f32,
    birth_dts: Option<&[f32]>, birth_backtrack: Option<(&[f32], [f32; 3])>, ctx: &Context) {
    assert!(start <= end && end <= system.pool.len());
    assert_eq!(system.pool.len(), system.side.len());
    if let Some(times) = birth_dts { assert_eq!(times.len(), end - start); }
    if let Some((scales, _)) = birth_backtrack {
        assert!(birth_dts.is_some() && scales.len() == end - start);
    }
    // Native Noise.Update advances its system scroll once for a nonempty
    // existing-particle range. StartModules birth ranges reuse that scroll and
    // must not advance it again. Empty ranges do not consume a scroll step.
    if birth_dts.is_none() && start < end {
        if let Some(noise) = &mut system.noise {
            noise.law.advance_scroll(&mut noise.state, dt, true);
        }
    }
    let owner = compose_to_world(system, ctx);
    let velocity_over_lifetime = system.velocity_law.as_ref();
    for index in start..end {
        let dt = birth_dts.map_or(dt, |times| times[index - start]);
        let side = system.side[index];
        let start_lifetime = system.pool[index].start_lifetime;
        // 引擎的模块批（自旋/叠加速/限速）先于推进跑，读的是**推进前**
        // 的年寿进程量——余寿先留底。
        let age_pre = system.pool[index].normalized_age();
        // 寿限非正的按出生即死（律对非正寿限拒绝推进，会永生）。
        if !(start_lifetime > 0.0) {
            system.pool[index].age_percent = f32::from_bits(0x42c80001);
            continue;
        }
        // Loop protection belongs to the inner span; displaced particles in
        // overflow finish their lifetime normally. Pause does not freeze motion.
        let lifetime_mode = if system.emitter.ring_buffer_mode == RingBufferMode::LoopUntilReplaced
            && index >= system.emitter.max_particles as usize {
            RingBufferMode::Disabled
        } else { system.emitter.ring_buffer_mode };
        // InitialModule writes gravity into persistent velocity before the
        // animated velocity, Force, ClampVelocity and integration batches.
        // Gravity is a world vector; local simulation applies the inverse owner
        // directly, without the extra emitter-scale factor used by Force.
        let gravity = Vec3::from_array(system.gravity_law.delta(side.seed, system.playback_head,
            system.emitter.duration, dt, GRAVITY));
        let gravity = if system.emitter.simulation_space == SimulationSpace::World { gravity }
            else { owner.affine().inverse().transform_vector3(gravity) };
        for axis in 0..3 { system.pool[index].velocity[axis] += gravity[axis]; }
        if let Some(rol) = &system.rol {
            let mut rot = system.side[index].rot;
            let _ = rol.advance_rotation(
                &mut rot,
                side.seed,
                0.0,
                age_pre * 100.0,
                system.emitter.start.rotation3d,
                dt,
            );
            system.side[index].rot = rot;
        }
        let batch_seed = system.side[start + ((index - start) & !3)].seed;
        let (velocity_anim, modifier) = match velocity_over_lifetime {
            Some(value) => motion::velocity_at_age(value, &system.pool[index], &side, batch_seed,
                system.emitter.simulation_space, &owner, age_pre, dt),
            None => ([0.0; 3], 1.0),
        };
        // Noise contributes transient animated velocity after authored
        // VelocityOverLifetime and before Force/LimitVelocity. Runtime stores
        // reflected coordinates, so reflect the source-space kernel once at
        // this boundary; it is not a world-space acceleration.
        let noise_anim = system.noise.map_or([0.0; 3], |noise| {
            let source_position = crate::particle_geometry::reflect(
                Vec3::from_array(system.pool[index].position)).to_array();
            crate::particle_geometry::reflect(Vec3::from_array(
                noise.law.sample(noise.state, source_position, noise.owner_seed),
            )).to_array()
        });
        let anim = std::array::from_fn(|axis| velocity_anim[axis] + noise_anim[axis]);
        system.side[index].animated = anim;
        // Force modifies persistent velocity before velocity limiting. Animated
        // velocity is still transient and shares the final integration modifier.
        if let Some(force) = &system.force_law {
            let acceleration = motion::module_vector(force.sample(side.seed, age_pre * 100.0),
                force.in_world_space, system.emitter.simulation_space, &owner);
            for axis in 0..3 {
                system.pool[index].velocity[axis] += acceleration[axis] * dt;
            }
        }
        if let Some(law) = &system.limit {
            let mut velocity = system.pool[index].velocity;
            let _ = law.step(
                &mut velocity,
                anim,
                side.seed,
                age_pre * 100.0,
                dt,
                DragSize {
                    components: motion::size_at_age_percent(system, &side, system.pool[index].age_percent),
                    size3d: system.emitter.start.size3d || system.emitter.size_over_lifetime
                        .as_ref().is_some_and(|size| size.separate_axes),
                },
            );
            system.pool[index].velocity = velocity;
        }
        if let Some(custom) = &system.custom_law {
            custom.update(side.seed, system.pool[index].age_percent, &mut system.side[index].custom_data);
        }
        if birth_dts.is_some() {
            // StartModules writes birth age directly, without ring looping or
            // pause. It then kills >100 lanes before CopyParticles packing.
            let p = &mut system.pool[index];
            let age = if p.age_percent >= 100.0 { p.age_percent }
                else { (dt * 100.0) * p.inverse_lifetime };
            p.age_percent = age.min(f32::from_bits(0x42c8_0001));
        } else {
            advance_lifetime(&mut system.pool[index], dt, lifetime_mode,
                system.emitter.ring_buffer_loop_range);
        }
        // The same pre-simulation speed modifier owns both orbital displacement
        // and integration. Animated velocity remains transient, including when
        // a zero speed modifier stops movement without clearing base velocity.
        let total = std::array::from_fn(|axis|
            system.pool[index].velocity[axis] + anim[axis]);
        let eff = total.map(|value| value * modifier);
        system.side[index].total_velocity = total;
        // 积分：律的 `integrate` 会用帧速度覆写状态速度——先存后还原。
        let state_velocity = system.pool[index].velocity;
        if birth_dts.is_some() {
            // The emitter-motion backtrack comes after the pre-simulation
            // modules above (they see the unmoved position) and before the
            // newborn integration.
            if let Some((scales, velocity)) = birth_backtrack {
                let scale = scales[index - start];
                for axis in 0..3 { system.pool[index].position[axis] -= scale * velocity[axis]; }
            }
            // Native inline birth arithmetic groups modifier*dt before the
            // velocity multiply. Reassociation changes several f32 results.
            let step = modifier * dt;
            for axis in 0..3 { system.pool[index].position[axis] += total[axis] * step; }
        } else if let StepVerdict::Refused = integrate(&mut system.pool[index], dt, eff) {
            system.refused_total += 1;
        }
        system.pool[index].velocity = state_velocity;
    }
 }

/// The newborn span's pre-simulation modules and its position and age update.
fn simulate_birth_modules(system: &mut Runtime, old_count: usize, partial_dts: &[f32],
    backtrack: Option<(&[f32], [f32; 3])>, ctx: &Context) {
    assert_eq!(old_count + partial_dts.len(), system.pool.len());
    simulate_range(system, old_count, system.pool.len(), 0.0, Some(partial_dts), backtrack, ctx);
}

/// Newborn deaths after the span's modules; returns the surviving count.
/// `deaths`, when given, receives the killed particles whose kill records
/// death events, in removal order, before their slot is overwritten.
fn kill_newborns(system: &mut Runtime, old_count: usize, accepted: usize,
    mut deaths: Option<&mut Vec<DeathParent>>) -> usize {
    // StartModules scans its newborn range forward and retests each swapped
    // tail; existing four-wide death compaction has a different removal order.
    let mut index = old_count;
    let mut surviving_count = accepted;
    while index < system.pool.len() {
        if system.pool[index].age_percent > 100.0 {
            // The kill records the death events only while the accepted count
            // it decrements is still nonzero, so a padding lane records too
            // until that count runs out.
            if surviving_count != 0 || child::arms::on("newbornDeathAlwaysRecords") {
                if let Some(deaths) = deaths.as_mut() {
                    deaths.push(sub_events::dying(index, &system.pool[index], &system.side[index]));
                }
            }
            system.pool.swap_remove(index);
            system.side.swap_remove(index);
            if let Some(trail) = system.trail.as_mut() { trail.rings.swap_remove(index); }
            system.died_total += 1;
            // The native cleanup visits rounded SIMD storage, including the
            // padding. Every kill saturating-decrements the accepted count;
            // a swapped padding lane can therefore enter the live prefix.
            surviving_count = surviving_count.saturating_sub(1);
        } else { index += 1; }
    }
    surviving_count
}

/// Outcome of installing the source birth owner on an admitted instance.
pub(crate) enum BirthPath {
    Native,
    /// The system stays on the legacy step; the reason is recorded for the
    /// diagnostics rather than inferred later.
    Legacy(String),
}

/// Whether this emitter takes the native birth path: the source selects the
/// ordinary incremental route and every enabled module has a verified native
/// consumer (Initial, constant or two-constant Emission, the qualified Shape configurations,
/// the qualified Noise subset, authored-null child edges; lifetime modules are
/// shared with the legacy step). The emitter state a Shape reads is checked
/// separately by `native_shape_state_eligible`.
pub(crate) fn native_birth_eligible(emitter: &EmitterParams, route: &SourceRoute) -> Result<(), String> {
    match route {
        SourceRoute::Ordinary => {}
        SourceRoute::Procedural => return Err(
            "procedural source route: time-evaluated simulation and Update(flags=3) prewarm are not transcribed".into()),
        SourceRoute::Undecided(control) => return Err(format!("source route undecided: {control}")),
    }
    birth::qualify_emitter(emitter).map_err(|refused| format!("{refused:?}"))
}

/// Whether any sub-emitter edge is real (not authored null: no emitter and
/// pointer 0/0).
pub(crate) fn has_real_sub_emitter_edges(emitter: &EmitterParams) -> bool {
    birth::has_real_sub_emitter_edges(emitter)
}

/// Whether the emitter state the native Shape boundary reads is qualified:
/// the scaling mode, the owner chain it implies and the render mode. An
/// emitter without a Shape block needs none of it.
pub(crate) fn native_shape_state_eligible(emitter: &EmitterParams, evidence: Option<ShapeEmitterEvidence>)
    -> Result<(), String> {
    birth::shape_emitter_state(emitter, evidence).map(|_| ()).map_err(|refused| format!("{refused:?}"))
}

/// Called once when the admitted source instance is installed. The first Play
/// resets the system seeds (one shared-manager draw for an automatic owner)
/// and expands the Initial, Shape and scalar emission streams. Noise reads the
/// same owner seed and starts from the reset scroll, so it is installed from
/// this event rather than drawing a second owner.
pub(crate) fn install_native_birth(system: &mut Runtime, seeds: &mut seed::SystemSeedManager,
    route: &SourceRoute) -> Result<BirthPath, seed::SeedError> {
    if system.native_birth.is_some() { return Ok(BirthPath::Native); }
    if let Err(reason) = native_birth_eligible(&system.emitter, route)
        .and_then(|()| native_shape_state_eligible(&system.emitter, system.geometry.shape_evidence())) {
        // The legacy step does not consume the native streams, but the source
        // still resets this system's seed at first Play: an automatic owner
        // takes the next shared-manager word, a manual one its serialized seed.
        let (owner, _) = seeds.create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)?;
        system.rng = Rng(u64::from(owner.seed) | (u64::from(owner.seed) << 32));
        return Ok(BirthPath::Legacy(reason));
    }
    // Qualify the Noise consumer before drawing, so a refused configuration is
    // reported before the owner is created.
    let noise_law = match system.emitter.noise.as_ref() {
        Some(params) => match NoiseLaw::from_params(params) {
            Ok(law) => Some(law),
            Err(reason) => return Ok(BirthPath::Legacy(reason.to_owned())),
        },
        None => None,
    };
    // The trail law was qualified with the rest of the composition; parse it
    // again here, before the owner draw, so a refusal draws nothing.
    let trail_law = match trails::qualify(&system.emitter) {
        Ok(law) => law,
        Err(reason) => return Ok(BirthPath::Legacy(reason.to_owned())),
    };
    let (owner, streams) = seeds.create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)?;
    system.native_birth = Some(birth::NativeBirthState {
        owner: Some(owner), initial: streams.initial, shape: streams.shape,
        emission: moly_law::particle::autonomous_emission::AutonomousEmissionState::initialized(streams.scalar_birth),
        frame: birth::FrameState::default(),
        events: None,
        target: None,
    });
    if let Some(law) = noise_law {
        system.noise = Some(NoiseRuntime {
            law,
            state: NoiseState { scroll: streams.noise_scroll },
            owner_seed: owner.seed,
            owner,
        });
    }
    // The module clock starts at zero and no Play resets it; every ring starts
    // empty (the first update's reset of all rings).
    if let Some(law) = trail_law {
        trails::install(system, law);
    }
    Ok(BirthPath::Native)
}

/// Explicit Noise consumer with its own owner draw. Production installs Noise
/// together with the native birth owner above; only the kernel checks use this.
#[cfg(test)]
pub(crate) fn install_noise_consumer(
    system: &mut Runtime,
    seeds: &mut seed::SystemSeedManager,
) -> Result<bool, String> {
    if system.noise.is_some() {
        return Ok(true);
    }
    let Some(params) = system.emitter.noise.as_ref() else {
        return Ok(false);
    };
    let law = NoiseLaw::from_params(params).map_err(str::to_owned)?;
    let (owner, streams) = seeds
        .create_owner(system.emitter.random_seed, system.emitter.auto_random_seed)
        .map_err(|error| format!("{error:?}"))?;
    system.noise = Some(NoiseRuntime {
        law,
        state: NoiseState {
            scroll: streams.noise_scroll,
        },
        owner_seed: owner.seed,
        owner,
    });
    Ok(true)
}

/// 出生一颗：形状抽样 → 出生取值 → 律的入池裁决。
/// Clock time as the fraction of one authored cycle, the time base of every
/// start value and emission curve.
fn normalized_time(system: &Runtime, head: f32) -> f32 {
    let duration = system.emitter.duration;
    if !(duration.is_finite() && duration > 0.0) {
        return 0.0;
    }
    (head / duration).clamp(0.0, 1.0)
}

fn spawn_one(system: &mut Runtime, ctx: &Context) {
    let (position, direction) = if let Some(shape) = system.emitter.shape.as_ref() {    // Current native RNG consumption: Circle/Cone 2, Sphere/Hemisphere 3,
    // SingleSidedEdge 1. A billboard's facing direction is not its birth velocity.
    let (local, raw_dir) = match shape.shape_type.as_str() {
        "Circle" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            circle_base(shape.radius, shape.radius_thickness, shape.arc, arc, radial)
        }
        "Cone" => {
            let angle = system
                .cone_angle
                .expect("判读已门 Cone 的 angle 在场");
            let t_theta = system.rng.next_f32();
            let t_radial = system.rng.next_f32();
            cone_base(
                shape.radius,
                shape.radius_thickness,
                angle,
                shape.arc,
                t_theta,
                t_radial,
            )
        }
        "ConeVolume" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            let distance = system.rng.next_f32();
            cone_volume(shape.radius, shape.radius_thickness,
                shape.controls.angle.expect("validated source cone angle"), shape.arc,
                shape.controls.length.expect("validated source cone length"), arc, radial, distance)
        }
        "Sphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            sphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Hemisphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            hemisphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Donut" => {
            let major_arc = system.rng.next_f32();
            let tube_angle = system.rng.next_f32();
            let radial = system.rng.next_f32();
            donut_position(shape.radius, shape.controls.donut_radius.expect("validated source donut radius"),
                shape.radius_thickness, shape.arc, major_arc, tube_angle, radial)
        }
        "Mesh" => {
            let source = system.emission_surface.as_ref().expect("source surface must be ready before emitter installation");
            let selector = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            let u = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            let v = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
            source.sample(selector, u, v)
        }
        "SingleSidedEdge" => {
            let t_theta = system.rng.next_f32();
            single_sided_edge(shape.radius, t_theta)
        }
        other => panic!("判读已门形状族，运行时遇到 {other}——判读与推进的门不一致"),
    };
    // EmitterStoreData owns two additional draws for a positive authored
    // position jitter. Do not consume them for zero; later birth streams must
    // retain their original sequence. Apply this BEFORE the shape transform.
    let amount = shape.controls.random_position.unwrap_or(0.0);
    let local = if amount > 0.0 {
        let arc = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
        let polar = moly_law::particle::shape::u01_from_bits(system.rng.next_u32());
        moly_law::particle::shape::randomize_position(local, amount, arc, polar)
    } else { local };
    // Shape-module TRS scales both position and velocity before rotation.
    // Independent engine measurements and current EmitterStoreData agree;
    // treating the scale as a billboard-size control would flatten the wrong data.
    let shape_scale = shape.controls.scale.unwrap_or([1.0; 3]);
    let local = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| local[i] * shape_scale[i]));
    let position = [
        local[0] + shape.position[0],
        local[1] + shape.position[1],
        local[2] + shape.position[2],
    ];
    // 出发方向：形状函数的方向经形状旋转后归一（锥形函数按引擎分工返回
    // 未归一向量，归一在调用方）。
    let mut direction = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| raw_dir[i] * shape_scale[i]));
    let length = (direction[0] * direction[0]
        + direction[1] * direction[1]
        + direction[2] * direction[2])
    .sqrt();
    if length > 1e-30 {
        direction = [
            direction[0] / length,
            direction[1] / length,
            direction[2] / length,
        ];
    } else {
        direction = [0.0; 3];
    }

    (position, direction)
    } else {
        assert_eq!(system.emitter.shape_enabled, Some(false), "missing shape requires explicit disabled-module evidence");
        ([0.0; 3], [0.0, 0.0, 1.0])
    };

    // ---- 出生取值表（表情链转录：逐项各抽一次，速度与重力共用稳定
    // 因子，种子 u32 最后一抽）----
    // Start values are sampled at the normalized emission time. The legacy step
    // births a whole slice at its start, so that is the time it uses.
    let t0 = normalized_time(system, system.previous_head);
    let r = system.rng.next_f32();
    let lifetime = system
        .emitter
        .start
        .lifetime
        .evaluate(t0, system.rng.next_f32())
        .max(0.01);
    let size_x = system
        .emitter
        .start
        .size
        .evaluate(t0, system.rng.next_f32());
    let size_y = match &system.emitter.start.size_y {
        Some(curve) => curve.evaluate(t0, system.rng.next_f32()),
        None => size_x,
    };
    let size_z = match &system.emitter.start.size_z {
        Some(curve) => curve.evaluate(t0, system.rng.next_f32()),
        None => size_x,
    };
    let colour = moly_law::particle::gradient::rgba8_to_float(
        moly_law::particle::color::initial_rgba8(
            &system.emitter.start.color, t0, system.rng.next_f32()));
    let spin0 = system
        .emitter
        .start
        .rotation
        .evaluate(t0, system.rng.next_f32());
    let (spin_x, spin_y) = if system.emitter.start.rotation3d {
        (system.emitter.start.rotation_x.as_ref().expect("validated source X rotation")
            .evaluate(t0, system.rng.next_f32()),
         system.emitter.start.rotation_y.as_ref().expect("validated source Y rotation")
            .evaluate(t0, system.rng.next_f32()))
    } else { (0.0, 0.0) };
    let speed = system.emitter.start.speed.evaluate(t0, r);
    let gravity = system.emitter.start.gravity_modifier.evaluate(t0, r);
    let seed = system.rng.next_u32();

    // ---- 空间锚定：世界空间仿真出生即锚（位置过全变换、方向过线性部
    // 后已归一），局部空间仿真留在节点局部系逐帧换算 ----
    let kind = system.kind;
    let camera_rotation = system.camera_rotation;
    let node_affine = system.node_affine;
    // 尺寸吃链缩放：语料 35 条**局部空间**条目带非恒等链缩放（4.0 ×28、
    // 0.9994 ×7，全部均匀；世界空间条目的链全恒等），折进出生尺寸与
    // 「渲染时折算」给出同一结果。非均匀链缩放语料里不存在，按 X 分量
    // 处理（具名记为未实现）。
    // Particle module coordinates are raw Unity coordinates; GLB and game
    // anchors use the producer's reflected-X basis. Convert once at this boundary.
    let position = crate::particle_geometry::reflect(Vec3::from_array(position)).to_array();
    let direction = crate::particle_geometry::reflect(Vec3::from_array(direction)).to_array();
    let (position, direction) = if system.emitter.simulation_space == SimulationSpace::World {
        let anchor = match kind {
            EffectKind::Sky => ctx.sky,
            EffectKind::Camera => {
                if camera_rotation {
                    ctx.camera
                } else {
                    GlobalTransform::from_translation(ctx.camera.translation())
                }
            }
            EffectKind::Site => ctx.site,
        };
        let to_world = anchor * node_affine;
        let position = to_world.transform_point(Vec3::from_array(position)).to_array();
        // Native EmitterStoreData has already normalized the direction in
        // emitter space.  The final owner 3x3 is a plain multiply: JP 6.8.1
        // does not normalize after it, so non-uniform owner scale remains in
        // the velocity magnitude.  Do not replace this with normalize_or_zero.
        let dir = if system.emitter.shape.is_some() {
            apply_world_owner_direction(&to_world, Vec3::from_array(direction))
        } else {
            // With Shape disabled Initial.Start owns the direction and
            // normalizes the owner's Z column before StartVelocity.
            to_world.affine().transform_vector3(Vec3::from_array(direction)).normalize_or_zero()
        };
        (position, dir.to_array())
    } else {
        (position, direction)
    };
    let velocity = [
        direction[0] * speed,
        direction[1] * speed,
        direction[2] * speed,
    ];

    let side = Side {
        rand: r,
        seed,
        rot: [spin_x, spin_y, spin0],
        size: [size_x, size_y, size_z],
        gravity,
        colour,
        total_velocity: velocity,
        custom_data: [[0.0; 4]; 2],
        emit_carry: [0.0; 2],
        animated: [0.0; 3],
    };
    let particle = Particle::born(position, velocity, lifetime);
    system.pool.push(particle);
    system.side.push(side);
    system.born_total += 1;
}



pub(crate) fn write_geometry(
    mesh: &mut Mesh, system: &Runtime, to_world: &GlobalTransform,
    owner: &GlobalTransform, camera: &GlobalTransform, basis: crate::billboard::CameraBasis,
) {
    match &system.geometry {
        Geometry::Billboard { alignment, clamp, pivot } => {
            crate::billboard::write_quads(mesh, &build_quads(system, to_world), *alignment, basis, *clamp, *pivot);
        }
        Geometry::Mesh(_) | Geometry::SourceBillboard(_) => {
            let base_frame = crate::particle_geometry::source_frame(owner, camera);
            let frame = match &system.geometry {
                Geometry::Mesh(draw) => draw.scaling.apply(base_frame),
                Geometry::SourceBillboard(draw) => draw.scaling.apply(base_frame),
                _ => unreachable!(),
            };
            let appearance = build_quads(system, to_world);
            let instances: Vec<_> = system.pool.iter().enumerate().map(|(index, particle)| {
                let side = system.side[index];
                let size = Vec3::from_array(motion::size_at_age_percent(system, &side, particle.age_percent));
                let view = &appearance[index];
                crate::particle_geometry::Instance {
                    position: crate::particle_geometry::reflect(view.centre),
                    velocity: crate::particle_geometry::reflect(to_world.affine().transform_vector3(Vec3::from_array(side.total_velocity))),
                    rotation: Vec3::from_array(side.rot), size,
                    colour: Vec4::from_array(view.colour),
                    custom1: Vec4::from_array(view.custom1), custom2: Vec4::from_array(view.custom2),
                }
            }).collect();
            match &system.geometry {
                Geometry::Mesh(draw) => crate::particle_geometry::write_mesh(mesh, draw, &instances, &frame),
                Geometry::SourceBillboard(draw) => crate::source_billboard::write(mesh, draw, &instances, &frame,
                    system.emitter.simulation_space == SimulationSpace::Local, basis.fov_y, basis.aspect),
                _ => unreachable!(),
            }
        }
    }
    if let Some(sheet) = system.texture_sheet {
        let vertices_per_particle = match &system.geometry {
            Geometry::Mesh(draw) => draw.source.positions.len(),
            Geometry::Billboard { .. } | Geometry::SourceBillboard(_) => 4,
        };
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) else {
            unreachable!("shared particle geometry always writes UV0");
        };
        assert_eq!(uv.len(), vertices_per_particle * system.side.len(), "sheet and particle geometry cardinality");
        for (corners, side) in uv.chunks_exact_mut(vertices_per_particle).zip(&system.side) {
            let rect = sheet.rect(side.seed);
            for corner in corners { *corner = rect.apply(*corner); }
        }
    }
    // Reorder draw indices, preserving pool order and its particle-local random
    // streams. Attributes and atlas coordinates still belong to the same seed.
    if system.sort_mode != moly_law::particle::sort::ParticleSort::None && !system.pool.is_empty() {
        let camera_local = to_world.affine().inverse().transform_point3(camera.translation());
        let particles: Vec<_> = system.pool.iter().map(|p| moly_law::particle::sort::SortParticle {
            position: p.position, age_percent: p.age_percent,
            inverse_lifetime: p.inverse_lifetime,
        }).collect();
        let order = system.sort_mode.indices(&particles, camera_local.to_array());
        let per_particle = match &system.geometry {
            Geometry::Mesh(draw) => draw.source.indices.len(),
            Geometry::Billboard { .. } | Geometry::SourceBillboard(_) => 6,
        };
        let Some(bevy::mesh::Indices::U32(indices)) = mesh.remove_indices() else {
            unreachable!("shared particle geometry writes u32 indices");
        };
        assert_eq!(indices.len(), per_particle * order.len(), "particle sort index cardinality");
        let sorted = order.into_iter().flat_map(|index|
            indices[index * per_particle..(index + 1) * per_particle].iter().copied()).collect();
        mesh.insert_indices(bevy::mesh::Indices::U32(sorted));
    }
}
