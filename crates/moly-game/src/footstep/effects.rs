//! The player's foot effects: `EffectManager` types 14 (dash smoke, a
//! `FollowEffect`) and 17 (water splash, a `FootEffect`), both emitted under
//! the pool key `PlayerFootEffect`.
//!
//! Source behaviour:
//! - `EffectManager.Setup` makes one pooled copy of each (pool size 1),
//!   inactive. `EmitFollowTargetTransForm(type, target, offset, ...)` takes
//!   that copy, calls its `Emit`, and adds its pool to the key's list.
//! - `FollowEffect.Emit` puts the copy at the target's position plus the
//!   offset (its rotation is left as it is: the footstep passes no rotation
//!   inheritance) and plays; its `LateUpdate` then copies the target's
//!   position plus offset and, axis by axis (no axis frozen), the target's
//!   rotation every frame. `FootEffect` does the same with the position only.
//! - `ManagedEffect.Play` activates the copy and calls `ParticleSystem.Play()`
//!   on its root system, children included. `ManagedEffect.Stop` calls
//!   `ParticleSystem.Stop()`: emission stops and live particles finish; the
//!   copy stays active and keeps following.
//! - `EffectManager.Stop(key)` stops every pool listed under the key;
//!   `Stop(type)` stops that type's pool.
//!
//! Product: each copy is built from its particle document (the packages
//! hold no mesh, so there is no prefab glb; the node hierarchy comes from the
//! document), prepared through the source-particle control path, and then
//! stepped here rather than by the fixture particle host, because a stopped
//! system must keep simulating without emitting. Each renderer takes its
//! emitter's authored `enabled` flag at Play. Named differences:
//! - Play after Stop restarts each system's clock and emission (bursts at
//!   time 0 fire again) and keeps its live particles; the engine's exact
//!   restart behaviour is not read.
//! - The particles step after this frame's follow write; the engine begins
//!   its particle update before `LateUpdate`, one follow write earlier.
//! - The rotation copy goes through Euler angles in the source; the product
//!   copies the rotation itself.
//! - A world-space particle already emitted is not moved when the world is
//!   re-origined by a site move.
//! - The water splash's two emitters emit by distance travelled, which the
//!   particle host does not run; both are refused, so the splash emits and
//!   stops (and logs) but draws nothing.

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::collections::HashMap;

use crate::particle_runtime::{Context, Runtime, PREWARM_STEP};
use crate::site_move::effects::{self as shared, EffectType, Prepared};
use moly_law::particle::schema::SimulationSpace;

/// The pool key the footstep emits and stops its effects under.
pub(crate) const KEY: &str = "PlayerFootEffect";

const KINDS: [EffectType; 2] = [EffectType::Dash, EffectType::WalkWater];

/// A foot effect system, stepped by [`step`].
#[derive(Component)]
pub(crate) struct FootParticle(pub(crate) Runtime);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Follow {
    /// `FollowEffect`: position plus offset, and rotation.
    Transform,
    /// `FootEffect`: position plus offset.
    Position,
}

fn follow_of(kind: EffectType) -> Follow {
    match kind {
        EffectType::Dash => Follow::Transform,
        _ => Follow::Position,
    }
}

enum Prep {
    /// Waiting for the presence gate or the document.
    Waiting,
    /// Hierarchy built; systems being prepared.
    Building,
    Ready,
    /// The host refused every emitter.
    Refused(String),
    /// No document, or one this port cannot build from.
    Unavailable(String),
}

struct Instance {
    kind: EffectType,
    doc: Option<Handle<JsonAsset>>,
    parsed: Option<Value>,
    root: Option<Entity>,
    selected: Vec<(Entity, usize)>,
    prep: Prep,
    draws: Vec<(Entity, bool)>,
    /// Activated by a first Play.
    active: bool,
    /// Between a Play and the next Stop.
    emitting: bool,
    offset: Vec3,
    target: Option<Entity>,
}

enum Gate {
    Pending(Handle<JsonAsset>),
    Present,
    Absent,
}

#[derive(Resource)]
pub(crate) struct FootEffects {
    gate: Gate,
    instances: Vec<Instance>,
    /// Pools listed under [`KEY`] (`AddPoolList`), in first-emit order.
    key_pools: Vec<EffectType>,
}

pub(crate) fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(FootEffects {
        gate: Gate::Pending(server.load(crate::site_move::products::INDEX)),
        instances: KINDS
            .into_iter()
            .map(|kind| Instance {
                kind,
                doc: None,
                parsed: None,
                root: None,
                selected: Vec::new(),
                prep: Prep::Waiting,
                draws: Vec::new(),
                active: false,
                emitting: false,
                offset: Vec3::ZERO,
                target: None,
            })
            .collect(),
        key_pools: Vec::new(),
    });
}

/// Update: the presence gate, the documents and the preparation.
pub(crate) fn advance(world: &mut World) {
    world.resource_scope(|world, mut effects: Mut<FootEffects>| effects.advance(world));
}

impl FootEffects {
    fn advance(&mut self, world: &mut World) {
        if let Gate::Pending(handle) = &self.gate {
            let handle = handle.clone();
            let server = world.resource::<AssetServer>();
            let packages: Vec<String> = KINDS.iter().map(|kind| kind.package()).collect();
            let missing = if server.load_state(&handle).is_failed() {
                packages.clone()
            } else {
                let Some(text) = world
                    .resource::<Assets<JsonAsset>>()
                    .get(&handle)
                    .map(|json| json.0.clone())
                else {
                    return;
                };
                let index: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                crate::site_move::products::unlisted(&index, packages.clone())
            };
            if missing.is_empty() {
                info!("[foot-effect] the release root lists the foot effect products {packages:?}; requesting them");
                self.gate = Gate::Present;
                let server = world.resource::<AssetServer>().clone();
                for instance in &mut self.instances {
                    instance.doc = Some(server.load(instance.kind.doc_path()));
                }
            } else {
                warn!(
                    "[foot-effect] release root predates the foot effects: {} does not list {missing:?}; no request, and footsteps run without the dash smoke and the water splash",
                    crate::site_move::products::INDEX
                );
                self.gate = Gate::Absent;
                for instance in &mut self.instances {
                    instance.prep = Prep::Unavailable("not in the release root".into());
                }
            }
        }
        for index in 0..self.instances.len() {
            self.advance_instance(world, index);
        }
    }

    fn advance_instance(&mut self, world: &mut World, index: usize) {
        let instance = &mut self.instances[index];
        let kind = instance.kind;
        match instance.prep {
            Prep::Waiting => {
                let Some(handle) = instance.doc.clone() else {
                    return;
                };
                if world
                    .resource::<AssetServer>()
                    .load_state(&handle)
                    .is_failed()
                {
                    warn!("[foot-effect] {kind:?} ({}) particle document {} failed to load: not shown", kind as u8, kind.doc_path());
                    instance.prep = Prep::Unavailable("document failed to load".into());
                    return;
                }
                let Some(text) = world
                    .resource::<Assets<JsonAsset>>()
                    .get(&handle)
                    .map(|json| json.0.clone())
                else {
                    return;
                };
                let doc: Value = match serde_json::from_str(&text) {
                    Ok(doc) => doc,
                    Err(error) => {
                        warn!("[foot-effect] {kind:?} particle document unreadable: {error}");
                        instance.prep = Prep::Unavailable(format!("document unreadable: {error}"));
                        return;
                    }
                };
                match build_hierarchy(world, kind, &doc) {
                    Ok((root, paths)) => {
                        instance.selected = shared::select_all(&doc, &paths);
                        instance.root = Some(root);
                        instance.parsed = Some(doc);
                        instance.prep = Prep::Building;
                        info!(
                            "[foot-effect] {kind:?} ({}) copy built from its document: {} nodes, {} emitters with a source renderer selected",
                            kind as u8,
                            paths.len(),
                            instance.selected.len()
                        );
                    }
                    Err(error) => {
                        warn!("[foot-effect] {kind:?} ({}) not shown: {error}", kind as u8);
                        instance.prep = Prep::Unavailable(error);
                    }
                }
            }
            Prep::Building => {
                let root = instance.root.expect("built");
                let doc = instance.parsed.take().expect("parsed");
                let label = format!("{kind:?}");
                let outcome = shared::prepare_skipping_refused(
                    world,
                    root,
                    &doc,
                    &mut instance.selected,
                    &label,
                );
                match outcome {
                    Prepared::Pending => {}
                    Prepared::Ready(draws) => {
                        let authored = shared::authored_renderers(world, &doc, &draws);
                        let mut nodes = Vec::new();
                        for &draw in &draws {
                            if let Some(live) = world
                                .entity_mut(draw)
                                .take::<crate::uber_particle::FixtureParticleLive>()
                            {
                                nodes.push(live.0.node.clone());
                                world.entity_mut(draw).insert(FootParticle(live.0));
                            }
                        }
                        instance.draws = authored;
                        instance.prep = Prep::Ready;
                        info!(
                            "[foot-effect] {kind:?} ({}) prepared: {} systems {nodes:?}, {}",
                            kind as u8,
                            nodes.len(),
                            if instance.active {
                                "already played"
                            } else {
                                "inactive in its pool"
                            }
                        );
                        if instance.emitting {
                            // Played before its systems were prepared: they
                            // start now, and so do their renderers.
                            restart(world, &draws);
                            let draws = instance.draws.clone();
                            shared::enable_renderers(world, kind, &draws);
                        }
                    }
                    Prepared::Refused(error) => {
                        warn!(
                            "[foot-effect] {kind:?} ({}) particles refused by the particle host: {error}; it emits and stops but draws nothing",
                            kind as u8
                        );
                        instance.prep = Prep::Refused(error);
                    }
                }
                instance.parsed = Some(doc);
            }
            Prep::Ready | Prep::Refused(_) | Prep::Unavailable(_) => {}
        }
    }

    fn instance(&mut self, kind: EffectType) -> &mut Instance {
        self.instances
            .iter_mut()
            .find(|instance| instance.kind == kind)
            .expect("an instance per foot effect type")
    }

    /// `EffectManager.Stop(key)` for [`KEY`].
    pub(crate) fn stop_key(&mut self, world: &mut World, reason: &str) {
        for kind in self.key_pools.clone() {
            self.stop(world, kind, &format!("key {KEY}: {reason}"));
        }
    }

    /// `EffectManager.Stop(type)` (no effect when the type has no pool).
    pub(crate) fn stop_type(&mut self, world: &mut World, kind: EffectType, reason: &str) {
        if matches!(self.gate, Gate::Present) {
            self.stop(world, kind, reason);
        }
    }

    fn stop(&mut self, world: &mut World, kind: EffectType, reason: &str) {
        let instance = self.instance(kind);
        let was = std::mem::replace(&mut instance.emitting, false);
        let live: usize = live_counts(world, &instance.draws).iter().sum();
        info!(
            "[foot-effect] Stop {kind:?} ({}) ({reason}): {}; {live} live particles finish their lifetimes",
            kind as u8,
            if was { "emission stopped" } else { "was not emitting" }
        );
    }

    /// `EmitFollowTargetTransForm(type, target, offset, false, false, false,
    /// false, KEY)`. Returns false when the type has no pool (the release
    /// root predates the product).
    pub(crate) fn emit(
        &mut self,
        world: &mut World,
        kind: EffectType,
        target: Entity,
        target_pose: Transform,
        offset: Vec3,
    ) -> bool {
        if !matches!(self.gate, Gate::Present) {
            info!("[foot-effect] Emit {kind:?} ({}): no pool (the product is not in the release root), nothing plays", kind as u8);
            return false;
        }
        if !self.key_pools.contains(&kind) {
            self.key_pools.push(kind);
        }
        let instance = self.instance(kind);
        instance.target = Some(target);
        instance.offset = offset;
        let first = !std::mem::replace(&mut instance.active, true);
        let restarted = !std::mem::replace(&mut instance.emitting, true);
        let position = target_pose.translation + offset;
        if let Some(root) = instance.root {
            if let Some(mut transform) = world.get_mut::<Transform>(root) {
                // Emit writes the position; the rotation is left as it is.
                transform.translation = position;
            }
        }
        let state = match &instance.prep {
            Prep::Ready => {
                let draws: Vec<Entity> = instance.draws.iter().map(|(draw, _)| *draw).collect();
                let live: usize = live_counts(world, &instance.draws).iter().sum();
                if restarted {
                    restart(world, &draws);
                }
                let flags = instance.draws.clone();
                shared::enable_renderers(world, kind, &flags);
                format!(
                    "{} systems {} ({live} live kept)",
                    draws.len(),
                    if restarted {
                        "restarted from time 0"
                    } else {
                        "already playing"
                    }
                )
            }
            Prep::Waiting | Prep::Building => {
                warn!("[foot-effect] {kind:?} emitted before its systems were prepared; they start when prepared");
                "systems not prepared yet".to_owned()
            }
            Prep::Refused(reason) => {
                let node = reason.split(": source").next().unwrap_or(reason);
                format!("systems refused by the particle host (last: {node}): nothing drawn")
            }
            Prep::Unavailable(reason) => format!("not shown: {reason}"),
        };
        info!(
            "[foot-effect] Emit {kind:?} ({}) {} at ({:.3},{:.3},{:.3}) offset ({:.3},{:.3},{:.3}){}: {state}",
            kind as u8,
            match follow_of(kind) {
                Follow::Transform => "following the player's position and rotation",
                Follow::Position => "following the player's position",
            },
            position.x,
            position.y,
            position.z,
            offset.x,
            offset.y,
            offset.z,
            if first { " (first play: copy activated)" } else { "" }
        );
        true
    }
}

/// `ParticleSystem.Play()` on a stopped system: its clock and emission
/// start again from time 0; live particles stay.
fn restart(world: &mut World, draws: &[Entity]) {
    for &draw in draws {
        if let Some(mut particle) = world.get_mut::<FootParticle>(draw) {
            let system = &mut particle.0;
            system.playback_head = 0.0;
            system.previous_head = 0.0;
            system.emission_started = false;
            system.emission = Default::default();
            system.prewarmed = false;
        }
    }
}

fn live_counts(world: &World, draws: &[(Entity, bool)]) -> Vec<usize> {
    draws
        .iter()
        .filter_map(|(draw, _)| world.get::<FootParticle>(*draw).map(|p| p.0.pool.len()))
        .collect()
}

/// Build the copy's node hierarchy from the document: a wrapper root (the
/// pooled copy's own transform) and every document node below it, with its
/// authored local transform reflected into the runtime frame.
fn build_hierarchy(
    world: &mut World,
    kind: EffectType,
    doc: &Value,
) -> Result<(Entity, HashMap<String, Vec<Entity>>), String> {
    if doc["nodeCoordinates"] != "unity-lh-y-up-authored" {
        return Err(format!(
            "unsupported node coordinates {}",
            doc["nodeCoordinates"]
        ));
    }
    let nodes = doc["nodes"].as_array().ok_or("document has no node list")?;
    let root = world
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Name::new(format!("foot effect {}", kind.prefab())),
        ))
        .id();
    let mut by_id: HashMap<i64, Entity> = HashMap::new();
    let mut paths: HashMap<String, Vec<Entity>> = HashMap::new();
    let mut remaining: Vec<&Value> = nodes.iter().collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut next = Vec::new();
        for node in remaining {
            let id = node["gameObjectId"]
                .as_i64()
                .ok_or("node without gameObjectId")?;
            let parent_id = node["parentGameObjectId"].as_i64().unwrap_or(0);
            let parent = if parent_id == 0 {
                root
            } else if let Some(parent) = by_id.get(&parent_id) {
                *parent
            } else {
                next.push(node);
                continue;
            };
            let path = node["node"].as_str().ok_or("node without a path")?;
            let transform = authored_transform(node)?;
            let name = path.rsplit('/').next().unwrap_or(path).to_owned();
            let entity = world
                .spawn((
                    transform,
                    Visibility::Inherited,
                    Name::new(name),
                    ChildOf(parent),
                ))
                .id();
            by_id.insert(id, entity);
            paths.entry(path.to_owned()).or_default().push(entity);
        }
        if next.len() == before {
            world.entity_mut(root).despawn();
            return Err("document nodes name parents that are not in it".into());
        }
        remaining = next;
    }
    if !paths.contains_key(kind.prefab()) {
        world.entity_mut(root).despawn();
        return Err(format!(
            "document has no prefab root node {}",
            kind.prefab()
        ));
    }
    Ok((root, paths))
}

/// A node's authored local transform (Unity, left-handed) in the runtime
/// frame, which reflects the x axis: position (-x, y, z), rotation
/// (x, -y, -z, w), scale unchanged.
fn authored_transform(node: &Value) -> Result<Transform, String> {
    let floats = |key: &str, len: usize| -> Result<Vec<f32>, String> {
        let list = node[key]
            .as_array()
            .ok_or_else(|| format!("node {} has no {key}", node["node"]))?;
        if list.len() != len {
            return Err(format!(
                "node {} {key} has {} values",
                node["node"],
                list.len()
            ));
        }
        list.iter()
            .map(|v| {
                v.as_f64()
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("node {} {key} is not numeric", node["node"]))
            })
            .collect()
    };
    let p = floats("position", 3)?;
    let r = floats("rotation", 4)?;
    let s = floats("scale", 3)?;
    Ok(Transform {
        translation: Vec3::new(-p[0], p[1], p[2]),
        rotation: Quat::from_xyzw(r[0], -r[1], -r[2], r[3]),
        scale: Vec3::new(s[0], s[1], s[2]),
    })
}

/// PostUpdate, before transform propagation: the `LateUpdate` follow of
/// every activated copy.
pub(crate) fn follow(
    effects: Option<Res<FootEffects>>,
    targets: Query<&Transform, With<crate::player::PlayerControlled>>,
    mut roots: Query<&mut Transform, Without<crate::player::PlayerControlled>>,
) {
    let Some(effects) = effects else { return };
    for instance in &effects.instances {
        let (Some(root), Some(target), true) = (instance.root, instance.target, instance.active)
        else {
            continue;
        };
        let Ok(target) = targets.get(target) else {
            continue;
        };
        let Ok(mut transform) = roots.get_mut(root) else {
            continue;
        };
        transform.translation = target.translation + instance.offset;
        if follow_of(instance.kind) == Follow::Transform {
            transform.rotation = target.rotation;
        }
    }
}

/// PostUpdate, after transform propagation and the camera: one step of
/// every activated system (emitting between Play and Stop), then its
/// geometry, as the fixture particle host does for its systems.
pub(crate) fn step(
    mut commands: Commands,
    effects: Option<Res<FootEffects>>,
    mut systems: Query<&mut FootParticle>,
    anchors: Query<&GlobalTransform>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    frame: Res<bevy::diagnostic::FrameCount>,
    sources: Query<(
        &crate::source_particle::SourceParticle,
        &InheritedVisibility,
    )>,
) {
    let Some(effects) = effects else { return };
    let Some((camera_transform, Projection::Perspective(projection), camera)) =
        cameras.iter().next()
    else {
        return;
    };
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let basis = crate::billboard::basis_from_matrix(
        camera_transform.affine().matrix3.into(),
        camera_transform.translation(),
        projection.fov,
        viewport.x as f32 / viewport.y.max(1) as f32,
    );
    let trace = shared::trace_enabled();
    for instance in &effects.instances {
        if !instance.active || !matches!(instance.prep, Prep::Ready) {
            continue;
        }
        for (draw, _) in &instance.draws {
            let Ok(mut particle) = systems.get_mut(*draw) else {
                continue;
            };
            let system = &mut particle.0;
            let Some(anchor) = system.anchor.and_then(|e| anchors.get(e).ok()).copied() else {
                continue;
            };
            let ctx = Context {
                site: anchor,
                sky: GlobalTransform::IDENTITY,
                camera: *camera_transform,
            };
            if !system.prewarmed {
                system.prewarmed = true;
                if system.emitter.prewarm && system.emitter.looping && instance.emitting {
                    for _ in 0..(system.emitter.duration / PREWARM_STEP).max(1.0) as usize {
                        crate::particle_runtime::simulate(system, PREWARM_STEP, &ctx);
                    }
                }
            }
            let dt = crate::particle_runtime::source_delta_time(time.delta());
            if dt > 0.0 {
                if let Err(reason) =
                    crate::particle_runtime::advance_frame(system, dt, instance.emitting, &ctx, |_| {})
                {
                    // The step must not run again on this system.
                    error!(
                        "[foot-effect] {:?} system {} step refused: {reason}; it is retired",
                        instance.kind, system.node
                    );
                    system.pool.clear();
                    if let Some(mesh) = meshes.get_mut(&system.mesh) {
                        *mesh = crate::billboard::empty_mesh();
                    }
                    commands.entity(*draw).remove::<FootParticle>();
                    continue;
                }
            }
            let transform = if system.emitter.simulation_space == SimulationSpace::World {
                GlobalTransform::IDENTITY
            } else {
                anchor
            };
            if let Some(mesh) = meshes.get_mut(&system.mesh) {
                crate::particle_runtime::write_geometry(
                    mesh,
                    system,
                    &transform,
                    &anchor,
                    camera_transform,
                    basis,
                );
            }
            if trace {
                let (enabled, visible) = sources
                    .get(*draw)
                    .map(|(source, visible)| {
                        (source.enabled.to_string(), visible.get().to_string())
                    })
                    .unwrap_or(("none".into(), "none".into()));
                info!(
                    "[effect-trace] frame {} {:?} draw {} enabled={enabled} inherited_visible={visible} live={} head={:.3} emitting={}",
                    frame.0,
                    instance.kind,
                    system.node,
                    system.pool.len(),
                    system.playback_head,
                    instance.emitting
                );
            }
        }
    }
}
