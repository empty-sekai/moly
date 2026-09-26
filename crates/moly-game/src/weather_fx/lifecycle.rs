//! Engine play state of every installed weather particle system, the
//! environment effector's `DestroyOnTime`, and renderer-visibility culling.
//!
//! Play state. `Play` sets the play state; `Stop(StopEmitting)` (the
//! effector's `ParticleSystem.Stop()`, through `StopChildrenRecursive`) stops
//! emission, stamps the stop time and, when the system holds no particle and
//! no emit-replay record, runs `Clear`, which with emission stopped ends the
//! play state at once. A non-looping system stops emission by itself on the
//! first slice whose system time has reached its duration. `EndUpdateAll` ends
//! the play state of a system in the manager whose update left no particle
//! while emission is stopped; a system that is not playing is no longer
//! updated. `ParticleSystem::IsPlaying` answers from the play state, except for
//! a culled system, which it answers from time (the ring guard first).
//!
//! `DestroyOnTime` (the effector's destruction loop) accumulates
//! `Time.deltaTime` from the Stop frame while any system of the instance
//! plays and destroys the instance once the sum reaches `_timeUntilDestroy`;
//! once none plays it waits the remaining time rounded down to whole seconds
//! through `UniTask.Delay` (never less than one further frame).
//!
//! Culling. `RendererScene::NotifyInvisible` (early in the frame) and
//! `ParticleSystem::Update2` (end of the update) cull a cullable system whose
//! renderer is visible in no culling pass; a culled system is not updated, and
//! `RendererScene::UpdateVisibility` un-culls it during the rendering of the
//! first frame whose pass finds it visible (`RendererBecameVisible`: a time
//! stop, or a re-Play without reset or warm). The pass reads the bounds of the
//! system's last `UpdateBounds`, the renderer's world box and the camera's
//! frustum planes, computed by `moly_law::particle::culling` in the engine's
//! binary32 order. A system the bounds law does not cover is refused by name
//! and keeps updating every frame.
use super::*;
use moly_law::particle::culling::{
    self as law, Bounds, BoundsConfig, BoundsRenderMode, BoundsSpace, BoundsState, FrustumCamera, Live, Plane,
    ShapeBounds, VelocityBounds,
};
use moly_law::particle::RingBufferMode;

/// A particle record of the effect prefab that the effector's child set
/// holds: `GetComponentsInChildren<ParticleSystem>()` without inactive
/// objects, so every system on an active object, admitted or not.
#[derive(Clone, Debug)]
pub(super) struct EffectChild {
    pub(super) node: String,
    /// The system's Emission module is off and no owner emits into it, so it
    /// holds no particle: `Stop` clears it and it is not playing afterwards.
    pub(super) never_holds_particles: bool,
}

/// The effector's child set of one effect prefab.
pub(super) fn effect_children(particles: &[Value], by_path: &HashMap<String, &Value>,
    sub_emitter_owners: &HashMap<String, Vec<String>>) -> Arc<Vec<EffectChild>> {
    let children = particles.iter().filter_map(|particle| {
        let node = particle.get("node").and_then(Value::as_str)?;
        if !active_in_hierarchy(by_path, node) { return None; }
        let system = particle.get("system").filter(|v| v.is_object());
        let emission_block = system.and_then(|s| s.get("emission")).filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
        let never_holds_particles = system.is_some_and(source_emission_disabled) && emission_block.is_none()
            && !sub_emitter_owners.contains_key(node);
        Some(EffectChild { node: node.to_owned(), never_holds_particles })
    }).collect();
    Arc::new(children)
}

// ---- Culling classification ----

/// Whether and how the source culls a system when its renderer is invisible.
#[derive(Clone, Debug)]
pub(super) enum Culling {
    /// `RendererBecameInvisible` never culls it: cullingMode AlwaysSimulate,
    /// or Automatic without looping and the procedural update.
    Never,
    /// The source culls it, but the product cannot compute its bounds or its
    /// camera test from what the renderer law covers; it keeps updating every
    /// frame, and the reason is named.
    Refused(String),
    Cullable(Box<Cullable>),
}

#[derive(Clone, Debug)]
pub(super) struct Cullable {
    /// cullingMode (1 PauseAndCatchup, 2 Pause, 0 Automatic).
    mode: u64,
    config: BoundsConfig,
    space: BoundsSpace,
    /// The renderer's render alignment, which decides whether the world box
    /// scales its extents by the emitter scale.
    alignment: u32,
    /// supportsProcedural and not invalidateProcedural: the route decision of
    /// DetermineSupportsProcedural, and no runtime writer of the invalidation
    /// flag exists for an effector-driven system (first Play clears it).
    procedural: bool,
}

impl Culling {
    pub(super) fn label(&self) -> Value {
        match self {
            Self::Never => serde_json::json!({"culled": "never"}),
            Self::Refused(reason) => serde_json::json!({"culled": "source", "port": "refused", "reason": reason}),
            Self::Cullable(c) => serde_json::json!({"culled": "source", "port": "cullable", "cullingMode": c.mode,
                "boundsPath": if c.procedural { "procedural" } else { "particles" }}),
        }
    }
}

fn number(v: Option<&Value>) -> Option<f32> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= f32::MAX as f64).map(|n| n as f32)
}

fn numbers<const N: usize>(v: Option<&Value>) -> Option<[f32; N]> {
    let list = v?.as_array().filter(|l| l.len() == N)?;
    let mut out = [0.0f32; N];
    for (slot, value) in out.iter_mut().zip(list) { *slot = number(Some(value))?; }
    Some(out)
}

/// `{center, extents}` of a serialized `m_LocalAABB`.
fn local_aabb(v: Option<&Value>) -> Option<([f32; 3], [f32; 3])> {
    let v = v?;
    Some((numbers::<3>(v.get("center"))?, numbers::<3>(v.get("extents"))?))
}

/// ShapeModule type codes of the shapes the bounds law executed.
fn shape_code(kind: &str) -> Option<u32> {
    Some(match kind {
        "Sphere" => 0, "Hemisphere" => 2, "Box" => 5, "Mesh" => 6, "ConeVolume" => 8, "Circle" => 10,
        "SingleSidedEdge" => 12, "Donut" => 17,
        _ => return None,
    })
}

/// The start-lifetime scalar the predicates read: the constant, the
/// two-constant maximum, or the curve multiplier.
fn lifetime_scalar(curve: &MinMaxCurve) -> f32 {
    match curve {
        MinMaxCurve::Constant(v) => *v,
        MinMaxCurve::TwoConstants { max, .. } => *max,
        MinMaxCurve::Curve { multiplier, .. } | MinMaxCurve::TwoCurves { multiplier, .. } => *multiplier,
    }
}

fn constant_mode(curve: &MinMaxCurve) -> bool {
    matches!(curve, MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. })
}

/// Classify one admitted weather system. `unit_chain`: the emitter node and
/// every ancestor carry scale exactly one. `UpdateLocalToWorldMatrixAndScales`
/// then stores a unit shape scale in every scaling mode and, for Local
/// scaling, a unit emitter scale; for Hierarchy scaling it derives the emitter
/// scale through a lossy composition that was not transcribed, and the port
/// takes it as one.
pub(super) fn source_culling(system: &Value, renderer: &Value, emitter: &EmitterParams,
    route: &crate::particle_runtime::SourceRoute, unit_chain: bool) -> Culling {
    use crate::particle_runtime::SourceRoute;
    let refused = |reason: &str| Culling::Refused(reason.to_owned());
    let mode = match system.get("cullingMode").and_then(Value::as_u64) {
        Some(3) => return Culling::Never,
        Some(0) => match route {
            _ if !emitter.looping => return Culling::Never,
            SourceRoute::Procedural => 0,
            SourceRoute::Ordinary => return Culling::Never,
            SourceRoute::Undecided(reason) =>
                return Culling::Refused(format!("Automatic culling needs the procedural decision: {reason}")),
        },
        Some(1) => return refused("PauseAndCatchup catches up through Simulate on becoming visible; not ported"),
        Some(2) => 2,
        Some(_) => return refused("cullingMode is not an engine mode"),
        None => return refused("cullingMode not exported"),
    };
    let procedural = match route {
        SourceRoute::Procedural => true,
        SourceRoute::Ordinary => false,
        SourceRoute::Undecided(reason) => return Culling::Refused(format!("the bounds path needs the procedural decision: {reason}")),
    };
    if !unit_chain {
        return refused("the emitter scale of a non-unit chain (UpdateLocalToWorldMatrixAndScales) is not transcribed");
    }
    if system.get("stopAction").and_then(Value::as_i64) != Some(0) {
        return refused("the stop action of a culled system is not ported");
    }
    let Some(enabled) = system.pointer("/sourceModules/enabled").and_then(Value::as_array) else {
        return refused("sourceModules not exported");
    };
    let has = |module: &str| enabled.iter().any(|m| m.as_str() == Some(module));
    let space = match emitter.simulation_space {
        SimulationSpace::Local => BoundsSpace::Local,
        SimulationSpace::World => BoundsSpace::World,
        _ => return refused("custom simulation space"),
    };
    let render_mode = match renderer.get("renderMode").and_then(Value::as_str) {
        Some("Billboard") => BoundsRenderMode::Billboard,
        Some("HorizontalBillboard") => BoundsRenderMode::HorizontalBillboard,
        Some("VerticalBillboard") => BoundsRenderMode::VerticalBillboard,
        Some("Stretch") => BoundsRenderMode::Stretch,
        Some("Mesh") => BoundsRenderMode::Mesh,
        _ => return refused("render mode outside the bounds law's executed domain"),
    };
    let Some(alignment) = renderer.get("alignment").and_then(Value::as_u64).and_then(|a| u32::try_from(a).ok()) else {
        return refused("renderer alignment not exported");
    };
    let Some(pivot) = numbers::<3>(renderer.get("pivot")) else { return refused("renderer pivot not exported"); };
    let (Some(velocity_scale), Some(length_scale)) = (number(renderer.get("velocityScale")), number(renderer.get("lengthScale"))) else {
        return refused("renderer stretch scales not exported");
    };
    // ParticleSystemRenderer::UpdateCachedMesh: the union of the mesh slots'
    // local boxes, (+inf, -inf) without a mesh.
    let mut renderer_mesh = [f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    if render_mode == BoundsRenderMode::Mesh {
        let Some(slots) = renderer.get("meshSlots").and_then(Value::as_array) else {
            return refused("renderer mesh slots not exported");
        };
        for slot in slots {
            for mesh in slot.get("meshes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
                let Some((c, e)) = local_aabb(mesh.get("bounds")) else { return refused("renderer mesh bounds not exported"); };
                for i in 0..3 {
                    let lo = c[i] - e[i];
                    let hi = c[i] + e[i];
                    if renderer_mesh[i] > lo { renderer_mesh[i] = lo; }
                    if hi > renderer_mesh[i + 3] { renderer_mesh[i + 3] = hi; }
                }
            }
        }
    }
    // The texture sheet's mode is read without its enable bit. The export
    // carries it for an enabled module; for a disabled module the serialized
    // census of every weather system gives Grid.
    let uv_sprites = match system.get("textureSheet") {
        None | Some(Value::Null) => false,
        Some(sheet) => match sheet.get("mode").and_then(Value::as_u64) {
            Some(0) => false,
            Some(_) => true,
            None => return refused("texture sheet mode not exported"),
        },
    };
    let shape = if system.get("shapeEnabled").and_then(Value::as_bool) == Some(true) {
        let Some(shape) = system.get("shape").filter(|v| v.is_object()) else { return refused("shape not exported"); };
        let Some(kind) = shape.get("type").and_then(Value::as_str).and_then(shape_code) else {
            return refused("shape type outside the bounds law's executed domain");
        };
        let mesh = if kind == 6 {
            let Some((c, e)) = local_aabb(shape.get("meshes").and_then(Value::as_array).and_then(|m| m.first()).and_then(|m| m.get("bounds"))) else {
                return refused("shape mesh bounds not exported");
            };
            [c[0], c[1], c[2], e[0], e[1], e[2]]
        } else { [0.0; 6] };
        let fields = (number(shape.get("radius")), number(shape.get("angle")), number(shape.get("length")),
            number(shape.get("donutRadius")), numbers::<3>(shape.get("position")), numbers::<3>(shape.get("rotation")),
            numbers::<3>(shape.get("scale")), number(shape.get("randomDirectionAmount")));
        let (Some(radius), Some(angle), Some(length), Some(donut_radius), Some(position), Some(rotation), Some(scale),
            Some(random_direction)) = fields else { return refused("shape bounds fields not exported"); };
        Some(ShapeBounds { kind, radius, angle, length, donut_radius, position, rotation, scale, random_direction, mesh })
    } else { None };
    let velocity = if has("VelocityModule") {
        let Some(v) = emitter.velocity_over_lifetime.as_ref() else { return refused("velocity module not exported"); };
        if ![&v.x, &v.y, &v.z].into_iter().all(constant_mode) {
            return refused("curve-mode FindMinMaxIntegrated calls pow, acos and cos through the PLT; not transcribed");
        }
        if v.in_world_space && space != BoundsSpace::World {
            return refused("a world-space velocity box goes through the emitter's worldToLocal, whose inversion is not transcribed");
        }
        Some(VelocityBounds { x: v.x.clone(), y: v.y.clone(), z: v.z.clone(), in_world_space: v.in_world_space })
    } else { None };
    let size_module = if has("SizeModule") {
        let Some(s) = emitter.size_over_lifetime.as_ref() else { return refused("size module not exported"); };
        Some([Some(s.curve.clone()), s.y.clone(), s.z.clone()])
    } else { None };
    let gravity_modifier = lifetime_scalar(&emitter.start.gravity_modifier);
    if gravity_modifier != 0.0 {
        return refused("the bounds gravity term reads Physics.gravity, which was not read from the game");
    }
    let size_axis = |axis: &Option<MinMaxCurve>| axis.clone().unwrap_or(MinMaxCurve::Constant(1.0));
    let separate = |key: &str| system.pointer(&format!("/{key}/separateAxes")).and_then(Value::as_bool) == Some(true);
    let config = BoundsConfig {
        space, has_renderer: true, render_mode, velocity_scale, length_scale, pivot, renderer_mesh,
        lifetime: emitter.start.lifetime.clone(), speed: emitter.start.speed.clone(),
        size: [emitter.start.size.clone(), size_axis(&emitter.start.size_y), size_axis(&emitter.start.size_z)],
        start_size_3d: emitter.start.size3d,
        particle_size_3d: emitter.start.size3d || (has("SizeModule") && separate("sizeOverLifetime"))
            || (has("SizeBySpeedModule") && separate("sizeBySpeed")),
        gravity_modifier, shape, velocity, size_module,
        trail: has("TrailModule"), lights: has("LightsModule"), force: has("ForceModule"),
        size_by_speed: has("SizeBySpeedModule"), uv_sprites,
    };
    // Every refusal of the bounds law is a configuration refusal: try both
    // paths once on a unit state.
    let one = [[0.0f32; 3]];
    let probe = |procedural: bool| law::update_bounds(&config, &BoundsState {
        procedural, local_to_world: IDENTITY16, world_to_local: IDENTITY16, shape_scale: [1.0; 3], scale: [1.0; 3],
        max_size_tracker: 0.0, gravity: GRAVITY, particles: Live { position: &one, velocity: &one, animated_velocity: &one, size_x: &[1.0] },
    });
    if let Err(error) = probe(procedural) { return Culling::Refused(error.to_string()); }
    Culling::Cullable(Box::new(Cullable { mode, config, space, alignment, procedural }))
}

const IDENTITY16: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
/// The bounds law multiplies this by the gravity modifier on every procedural
/// path. The gate admits only a zero modifier, so this value only decides the
/// sign of the zero terms of the gravity box; a non-zero modifier is refused
/// because Physics.gravity was not read from the game.
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// Engine (left-handed) form of a product matrix: the product frame reflects
/// X, so the engine matrix is S M S with S = diag(-1, 1, 1, 1).
fn engine_matrix(m: Mat4) -> [f32; 16] {
    let mut a = m.to_cols_array();
    for column in 0..4 {
        for row in 0..4 {
            if (row == 0) != (column == 0) { a[column * 4 + row] = -a[column * 4 + row]; }
        }
    }
    a
}

/// The frame's camera pass in engine coordinates.
pub(super) struct CameraPass {
    planes: [Plane; 6],
    far_base: f32,
    far: f32,
}

impl CameraPass {
    /// `Camera::CalculateFrustumPlanes` for the product camera: no custom
    /// culling or view matrix, so the side planes come from the culling matrix
    /// (projection times world-to-camera) and near and far are rebuilt from the
    /// camera position and axis. The engine's world-to-camera equals the
    /// product view in the reflected frame, so the culling matrix is the
    /// product's clip-from-world with its X column negated and the
    /// camera-to-world matrix the product camera matrix with its X row negated.
    pub(super) fn new(camera_transform: &GlobalTransform, camera: &Camera, near: f32, far: f32) -> Self {
        let camera_matrix = camera_transform.to_matrix();
        let mut culling = (camera.clip_from_view() * camera_matrix.inverse()).to_cols_array();
        for value in &mut culling[0..4] { *value = -*value; }
        let mut camera_to_world = camera_matrix.to_cols_array();
        for column in 0..4 { camera_to_world[column * 4] = -camera_to_world[column * 4]; }
        let (planes, far_base) = law::frustum_planes(&culling, &FrustumCamera {
            implicit_culling: true, implicit_view: true, camera_to_world, near, far,
        });
        Self { planes, far_base, far }
    }
}

// ---- Play state ----

/// The engine state of one installed system that the stop paths, the
/// predicates and culling read.
pub(super) struct PlayState {
    playing: bool,
    stop_emitting: bool,
    stop_time: f64,
    culled: bool,
    cull_time: f64,
    /// The first Play warmed through the procedural update, which leaves
    /// emit-replay records that decide whether Stop clears an empty system.
    procedural_warm: bool,
    pub(super) culling: Culling,
    /// The renderer's RendererScene node byte: visible in this frame's passes
    /// (1), visible at the last notification (2), notified visible (4).
    node: u8,
    /// The renderer's visibility flag (set by Renderer::RendererBecameVisible,
    /// cleared by Renderer::RendererBecameInvisible).
    renderer_visible: bool,
    /// The renderer's scene node exists: a renderer added this frame joins
    /// the scene at the next frame's NotifyInvisible.
    registered: bool,
    /// The box of the last UpdateBounds (engine coordinates).
    bounds: Option<Bounds>,
    bounds_played: bool,
}

impl PlayState {
    /// Play at instantiation.
    pub(super) fn played(culling: Culling, procedural_warm: bool) -> Self {
        Self { playing: true, stop_emitting: false, stop_time: 0.0, culled: false, cull_time: 0.0, procedural_warm,
            culling, node: 0, renderer_visible: false, registered: false, bounds: None, bounds_played: false }
    }

    /// In the manager: updated this frame.
    pub(super) fn managed(&self) -> bool {
        self.playing && !self.culled
    }

    /// The play word alone (playing), which a parent's update reads when it
    /// collects this system as a sub-emitter target.
    pub(super) fn playing(&self) -> bool {
        self.playing
    }

    /// A parent's update collected this target: scheduling its job plays it
    /// again (and lists it in the manager when it was not), stops its
    /// emission and stamps the stop time with the frame's time. Its own
    /// emission never runs; its births come from its parents' commands.
    pub(super) fn keep_updating(&mut self, now: f64) {
        self.playing = true;
        self.stop_emitting = true;
        self.stop_time = now;
    }

    fn cullable(&self) -> Option<&Cullable> {
        match &self.culling { Culling::Cullable(c) => Some(c), _ => None }
    }

    /// `ParticleSystem::IsPlaying`.
    pub(super) fn is_playing(&self, system: &Runtime, now: f64) -> bool {
        if !self.culled {
            return self.playing;
        }
        if system.emitter.ring_buffer_mode != RingBufferMode::Disabled && !system.pool.is_empty() {
            return self.playing;
        }
        let lifetime = lifetime_scalar(&system.emitter.start.lifetime);
        if !system.emitter.looping {
            let elapsed = (now - self.cull_time) + f64::from(system.playback_head);
            if elapsed > f64::from(system.emitter.duration + lifetime) { return false; }
        }
        if !self.stop_emitting { return self.playing; }
        if now - self.stop_time > f64::from(lifetime) { return false; }
        self.playing
    }

    /// `Clear` with emission stopped: the particles and the ring cursor go,
    /// the stop and cull times return to zero, and the play state ends.
    fn clear(&mut self, system: &mut Runtime) {
        if let Some(custom) = system.custom_law.as_mut() {
            custom.clear(&system.pool);
        }
        if let Some(calls) = system.size_law.as_mut().and_then(|size| size.calls_mut()) {
            calls.clear(&system.pool);
        }
        system.pool.clear();
        system.side.clear();
        system.ring_cursor = 0;
        self.stop_time = 0.0;
        self.cull_time = 0.0;
        self.playing = false;
    }

    /// `Stop(StopEmitting)`. Returns the named reason when whether it cleared
    /// the system is undecided.
    pub(super) fn stop(&mut self, system: &mut Runtime, now: f64) -> Option<String> {
        self.stop_emitting = true;
        self.stop_time = now;
        if !system.pool.is_empty() { return None; }
        if self.procedural_warm {
            return Some(format!("{}: whether Stop clears a system warmed through the procedural update depends on its emit-replay records, which that warm does not carry", system.node));
        }
        self.clear(system);
        None
    }

    /// Start of one incremental slice: a non-looping system whose time has
    /// reached its duration stops emitting. (Its Clear, when it holds no
    /// particle, and the end of the frame give the same final state.)
    pub(super) fn slice_start(&mut self, system: &Runtime, now: f64) {
        if !system.emitter.looping && system.playback_head >= system.emitter.duration && !self.stop_emitting {
            self.stop_emitting = true;
            self.stop_time = now;
        }
    }

    /// `UpdateBounds`: at the end of an update that ran, at Play and after a
    /// first-Play warm.
    pub(super) fn update_bounds(&mut self, system: &Runtime, emitter_to_world: &GlobalTransform) {
        let Some(cullable) = self.cullable() else { return; };
        // Only the Stretch render mode reads velocity, animated velocity and
        // size: the persistent velocity, the last pre-simulation pass's
        // animated velocity and the X size array. Everything goes back to the
        // engine's frame.
        let unreflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let position: Vec<[f32; 3]> = system.pool.iter().map(|p| unreflect(p.position)).collect();
        let velocity: Vec<[f32; 3]> = system.pool.iter().map(|p| unreflect(p.velocity)).collect();
        let animated: Vec<[f32; 3]> = system.side.iter().map(|s| unreflect(s.animated)).collect();
        let size_x: Vec<f32> = system.side.iter().map(|s| s.size[0]).collect();
        let local_to_world = engine_matrix(emitter_to_world.to_matrix());
        let world_to_local = Mat4::from_cols_array(&local_to_world).inverse().to_cols_array();
        let state = BoundsState {
            procedural: cullable.procedural, local_to_world, world_to_local, shape_scale: [1.0; 3], scale: [1.0; 3],
            max_size_tracker: 0.0, gravity: GRAVITY,
            particles: Live { position: &position, velocity: &velocity, animated_velocity: &animated, size_x: &size_x },
        };
        match law::update_bounds(&cullable.config, &state) {
            Ok(bounds) => self.bounds = Some(bounds),
            Err(error) => {
                error!(%error, effect=%system.effect, node=%system.node, "weather bounds refused; the system keeps updating");
                self.culled = false;
                self.culling = Culling::Refused(error.to_string());
            }
        }
    }

    /// Play's own UpdateBounds (after the first-Play warm when it ran), once.
    pub(super) fn played_bounds(&mut self, system: &Runtime, emitter_to_world: &GlobalTransform) {
        if !self.bounds_played {
            self.bounds_played = true;
            self.update_bounds(system, emitter_to_world);
        }
    }

    /// `ParticleSystem::RendererBecameInvisible`.
    fn became_invisible(&mut self, now: f64) {
        if !self.registered || self.culled || self.cullable().is_none() { return; }
        self.culled = true;
        self.cull_time = now;
    }

    /// `ParticleSystem::RendererBecameVisible`.
    fn became_visible(&mut self, system: &mut Runtime, now: f64) {
        if !self.culled { return; }
        self.culled = false;
        if !self.playing { return; }
        let ring_guard = system.emitter.ring_buffer_mode != RingBufferMode::Disabled && !system.pool.is_empty();
        if !ring_guard {
            let lifetime = lifetime_scalar(&system.emitter.start.lifetime);
            let ended = !system.emitter.looping && (now - self.cull_time) + f64::from(system.playback_head)
                > f64::from(system.emitter.duration + lifetime);
            let expired = self.stop_emitting && now - self.stop_time > f64::from(lifetime);
            if ended || expired {
                self.stop_emitting = true;
                self.stop_time = now;
                self.clear(system);
                return;
            }
        }
        // Pause: re-Play with the stop flags kept; no reset, warm or catch-up.
    }

    /// `Update2` and `EndUpdateAll` for a system in the manager.
    /// `EndUpdateAll` runs `Update2` (and with it the invisibility cull) only
    /// for a system whose update job is pending, the byte
    /// `JobsSchedulerHelper::ScheduleUpdateJobsHelper` sets when it schedules
    /// that system's update job and `EndUpdateAll` clears, and only when the
    /// delta of the system's clock (`Time.deltaTime`, or
    /// `Time.unscaledDeltaTime` under useUnscaledTime) is non-zero. The caller
    /// passes `update2` for that. The stop transition below runs either way.
    pub(super) fn end_update(&mut self, system: &Runtime, now: f64, update2: bool) {
        if update2 && !self.culled && !self.renderer_visible && self.registered {
            self.became_invisible(now);
        }
        if system.pool.is_empty() && self.stop_emitting {
            self.playing = false;
        }
    }

    /// `RendererScene::NotifyInvisible` at the start of the frame, then the
    /// pending scene additions.
    pub(super) fn notify_invisible(&mut self, now: f64) {
        if self.node == 2 {
            self.renderer_visible = false;
            self.became_invisible(now);
        }
        self.node = (self.node & 1) << 1;
        self.registered = true;
    }

    /// The renderer's world box and the camera test of the frame's pass, then
    /// `RendererScene::UpdateVisibility`.
    pub(super) fn render_pass(&mut self, system: &mut Runtime, emitter_to_world: &GlobalTransform, pass: &CameraPass, now: f64) {
        if !self.registered { return; }
        let (Some(cullable), Some(bounds)) = (self.cullable(), self.bounds) else { return; };
        let local_to_world = engine_matrix(emitter_to_world.to_matrix());
        let world_to_local = Mat4::from_cols_array(&local_to_world).inverse().to_cols_array();
        let (world, _) = law::world_bounds(&bounds, cullable.space, cullable.alignment, &local_to_world, &world_to_local, [1.0; 3]);
        if !law::visible_to_camera(&world, &pass.planes, pass.far_base, pass.far) { return; }
        self.node |= 1;
        if self.node == 1 {
            self.renderer_visible = true;
            self.became_visible(system, now);
            self.node |= 4;
        }
    }

    pub(super) fn report(&self) -> Value {
        serde_json::json!({"playing": self.playing, "stopEmitting": self.stop_emitting, "culled": self.culled,
            "culling": self.culling.label(), "rendererVisible": self.renderer_visible, "registered": self.registered})
    }
}

// ---- DestroyOnTime ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum DestroyPhase {
    /// While any system plays: the f32 sum of the frame deltas since Stop.
    Accumulating(f32),
    /// `UniTask.Delay` of the remaining whole seconds, Update timing.
    Waiting { delay: f32, elapsed: f32 },
    /// The remaining milliseconds overflow; the native Delay throws.
    Forever,
}

/// The effector's destruction loop of one stopped effect instance.
#[derive(Clone, Debug)]
pub(super) struct DestroyOnTime {
    time_until_destroy: f32,
    phase: DestroyPhase,
    /// Children whose play state the product does not hold, named once a
    /// decision depended on them; the loop then counts them as playing.
    pub(super) undecided: Vec<String>,
}

/// What one check sees of the instance's systems.
pub(super) enum InstancePlaying {
    Yes,
    No,
    /// No held system plays, and these children's play state is not held.
    Unknown(Vec<String>),
}

impl DestroyOnTime {
    pub(super) fn new(time_until_destroy: f64) -> Self {
        Self { time_until_destroy: time_until_destroy as f32, phase: DestroyPhase::Accumulating(0.0), undecided: Vec::new() }
    }

    /// One check, with this frame's `Time.deltaTime`. The first runs inside
    /// the Stop call; every later one in the next frames' Update. True when the
    /// instance is destroyed now.
    pub(super) fn check(&mut self, playing: InstancePlaying, delta: f32) -> bool {
        let t = self.time_until_destroy;
        match self.phase {
            DestroyPhase::Forever => false,
            DestroyPhase::Waiting { delay, elapsed } => {
                // The promise's creation frame never reaches this call.
                let elapsed = elapsed + delta;
                self.phase = DestroyPhase::Waiting { delay, elapsed };
                elapsed >= delay
            }
            DestroyPhase::Accumulating(acc) => {
                let playing = match playing {
                    InstancePlaying::Yes => true,
                    InstancePlaying::No => false,
                    InstancePlaying::Unknown(names) => {
                        for name in names { if !self.undecided.contains(&name) { self.undecided.push(name); } }
                        true
                    }
                };
                if playing {
                    let acc = acc + delta;
                    self.phase = DestroyPhase::Accumulating(acc);
                    return acc >= t;
                }
                if !(acc < t) { return true; }
                let remaining = t - acc;
                let ms = if remaining == f32::INFINITY { 0 } else { (remaining as i32).wrapping_mul(1000) };
                self.phase = if ms < 0 { DestroyPhase::Forever }
                    else { DestroyPhase::Waiting { delay: (f64::from(ms) / 1000.0) as f32, elapsed: 0.0 } };
                false
            }
        }
    }

    pub(super) fn report(&self) -> Value {
        let (phase, value) = match self.phase {
            DestroyPhase::Accumulating(acc) => ("accumulating", acc),
            DestroyPhase::Waiting { delay, .. } => ("waiting", delay),
            DestroyPhase::Forever => ("forever", f32::NAN),
        };
        serde_json::json!({"timeUntilDestroy": self.time_until_destroy, "phase": phase, "value": value,
            "undecidedChildren": self.undecided})
    }
}
