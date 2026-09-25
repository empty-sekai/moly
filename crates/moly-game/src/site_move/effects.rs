//! The move's pooled effects (`EffectManager` types 4, 5 and 16). The
//! camera's speed lines are a canvas image, in `speed_lines`.
//!
//! - 16 `Flying`: `EmitFollowTargetTransForm(Flying, camera transform)` when
//!   the `_s2` clip passes its `PublishFlyingEffectPlay` event, stopped by
//!   `EffectManager.Stop(Flying)` when a landing clip starts (its
//!   `PublishFlyingEffectEnd` event sits at 0). `FollowEffect.LateUpdate`
//!   copies the camera's position and its euler rotation every frame (no
//!   axis frozen). The emit call adds 180 degrees of yaw once, but that write
//!   is replaced by the first `LateUpdate`, before any frame renders it, so
//!   the visible effect follows the camera pose itself.
//! - 4 `SiteMoveEndPlayerEffect` / 5 `SiteMoveFailedPlayerEffect`:
//!   `SiteMoveEffect.Emit` puts the pooled effect at the player's position
//!   and plays it.
//!
//! `ManagedEffect.Play` activates the pooled object and calls
//! `_particleSystem.Play()` on the root system, which plays its children
//! with it. The landing prefabs' systems are all play-on-awake, so the
//! fixture source path plans them as authored. The flying prefab's systems
//! are not play-on-awake: they run only through that explicit `Play`, which
//! the source-particle control preparation expresses (every emitter selected,
//! no Director clock, so they run from the frame they are installed). At that
//! Play step each prepared flying system's renderer takes its emitter's
//! authored `enabled` flag; a copy inactive in its pool keeps them off, and
//! the root system, whose renderer the prefab ships off, is not selected.
//!
//! Pools: `EffectManager.Setup` (called from the field scene's setup, before
//! any move) instantiates each effect type's copies (the table's pool size)
//! and keeps them inactive. A pool is a fixed ring: each emit takes the next
//! copy in turn (`ResourcePool.GetResource` advances its index modulo the
//! count and checks nothing) and plays it, even one still playing; no copy
//! is ever returned. `SiteMoveEffect` and `FollowEffect` do not end
//! themselves: after its systems stop a copy stays active and draws nothing
//! (only `OnShotEffect`, which no pooled type uses, deactivates itself when
//! its root system stops playing). Here the pools are requested at startup
//! and each type keeps one instantiated, inactive copy whose particles are
//! prepared while inactive (their clocks do not run). An emit activates that
//! copy at the emit pose, and a fresh copy is instantiated for the next
//! emit.
//!
//! Named differences:
//! - `ManagedEffect.Stop` is `ParticleSystem.Stop()`, which stops emitting
//!   and lets live particles finish: the stopped flying copy stays active
//!   and keeps following the camera (`FollowEffect` never ends itself) until
//!   the next flying emit restarts the pool's copy, which the product plays
//!   as a fresh copy and releases the stopped one then.
//! - A played copy is not reused: the host cannot restart a system that has
//!   run, so each emit plays a fresh copy, and a copy is released once its
//!   systems end (it draws nothing by then). Visible only when more emits of
//!   one type overlap than its pool holds; the source then restarts the
//!   oldest, the product plays them all.
//! - An emitter the particle host refuses (the flying prefab's `pt_01`
//!   emits by distance only, which the host does not run) is skipped with a
//!   WARN; the prefab's other emitters play.
//! - A missing prefab or document is one WARN and no effect.

use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::SetSourceActive;
use serde_json::Value;
use std::collections::HashMap;

use super::{InstanceReady, PendingInstance, SiteMoveOwned};

/// `EffectManager`'s `EffectType` values the product plays. The move's three
/// come from the harvest action family, whose packages ship a prefab glb; the
/// player's two foot effects (played by [`crate::footstep`]) come from the
/// common action family, whose packages hold no mesh and so ship no glb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectType {
    SiteMoveEndPlayer = 4,
    SiteMoveFailedPlayer = 5,
    Dash = 14,
    Flying = 16,
    WalkWater = 17,
}

const TYPES: [EffectType; 3] = [
    EffectType::Flying,
    EffectType::SiteMoveEndPlayer,
    EffectType::SiteMoveFailedPlayer,
];

/// The particle packages of the pooled effects (the release lists them in
/// the fixture-particles-v2 index when it ships them).
pub(crate) fn packages() -> impl Iterator<Item = String> {
    TYPES.into_iter().map(EffectType::package)
}

impl EffectType {
    /// The package's last name segment (`EffectResourceData` bundle names).
    fn leaf(self) -> &'static str {
        match self {
            Self::SiteMoveEndPlayer => "fx_act_user_landing",
            Self::SiteMoveFailedPlayer => "fx_act_user_landing_fail",
            Self::Flying => "fx_act_user_flying",
            Self::Dash => "dash",
            Self::WalkWater => "walk_water",
        }
    }

    /// The prefab `EffectResourceData` names in the package; its root node.
    pub(crate) fn prefab(self) -> &'static str {
        match self {
            Self::Dash => "fx_act_user_walking",
            Self::WalkWater => "fx_act_user_walking_water",
            other => other.leaf(),
        }
    }

    pub(crate) fn package(self) -> String {
        match self {
            Self::Dash | Self::WalkWater => {
                format!("mysekai__effect__site__common__action__{}", self.leaf())
            }
            _ => format!("mysekai__effect__site__harvest__action__{}", self.leaf()),
        }
    }

    /// The prefab glb, for the harvest action family only.
    fn glb_path(self) -> Option<String> {
        match self {
            Self::Dash | Self::WalkWater => None,
            _ => Some(format!(
                "moly://site-action-effects/{0}/{0}.glb",
                self.leaf()
            )),
        }
    }

    pub(crate) fn doc_path(self) -> String {
        format!("moly://fixture-particles-v2/{}.json", self.package())
    }
}

struct Pool {
    kind: EffectType,
    glb: Handle<Gltf>,
    doc: Handle<JsonAsset>,
    warned: bool,
}

/// One effect instance: the inactive copy in its pool, or a played one.
struct Live {
    kind: EffectType,
    root: Entity,
    planned: bool,
    age: f32,
    /// Still in the pool (not played yet).
    pooled: bool,
    /// Its source nodes were deactivated for the pool.
    held: bool,
    /// Age at which it was played.
    played_at: Option<f32>,
    /// Each prepared control draw with its emitter's authored
    /// `renderer.enabled` (the flying effect's preparation).
    draws: Vec<(Entity, bool)>,
}

pub(crate) struct Effects {
    pools: Vec<Pool>,
    live: Vec<Live>,
    flying: Option<Entity>,
    /// The flying copy after its Stop, still following.
    stopped: Option<Entity>,
}

impl Effects {
    pub(crate) fn request(server: &AssetServer) -> Self {
        let pools = TYPES
            .into_iter()
            .map(|kind| Pool {
                kind,
                glb: server.load(
                    kind.glb_path()
                        .expect("the move's effects ship a prefab glb"),
                ),
                doc: server.load(kind.doc_path()),
                warned: false,
            })
            .collect();
        Self {
            pools,
            live: Vec::new(),
            flying: None,
            stopped: None,
        }
    }

    /// Settled when every prefab and document has loaded or failed (a
    /// failure only disables that effect).
    pub(crate) fn settled(&self, server: &AssetServer) -> bool {
        self.pools.iter().all(|pool| {
            let glb = server.is_loaded_with_dependencies(&pool.glb)
                || server.load_state(&pool.glb).is_failed()
                || server
                    .recursive_dependency_load_state(&pool.glb)
                    .is_failed();
            let doc = server.load_state(&pool.doc).is_loaded()
                || server.load_state(&pool.doc).is_failed();
            glb && doc
        })
    }

    /// Keep one inactive copy of every loaded effect type in its pool.
    fn fill_pools(&mut self, world: &mut World) {
        for kind in TYPES {
            if self
                .live
                .iter()
                .any(|live| live.pooled && live.kind == kind)
            {
                continue;
            }
            let server = world.resource::<AssetServer>();
            let pool = self
                .pools
                .iter()
                .find(|pool| pool.kind == kind)
                .expect("pool per kind");
            let loaded = server.is_loaded_with_dependencies(&pool.glb)
                && server.load_state(&pool.doc).is_loaded();
            let failed = server.load_state(&pool.glb).is_failed()
                || server
                    .recursive_dependency_load_state(&pool.glb)
                    .is_failed()
                || server.load_state(&pool.doc).is_failed();
            if loaded || (failed && !pool.warned) {
                // A failed prefab warns once through the instantiation.
                self.instantiate(world, kind, Transform::IDENTITY, true);
            }
        }
    }

    /// `EffectManager.Emit(type, position)` / `EmitFollowTargetTransForm`:
    /// `ManagedEffect.Play` on the pool's copy, i.e. `SetActive(true)` then
    /// `Play`.
    pub(crate) fn emit(&mut self, world: &mut World, kind: EffectType, pose: Transform) -> bool {
        let root = match self
            .live
            .iter()
            .position(|live| live.pooled && live.kind == kind)
        {
            Some(index) => {
                let live = &mut self.live[index];
                live.pooled = false;
                live.played_at = Some(live.age);
                let held = std::mem::take(&mut live.held);
                let (root, age) = (live.root, live.age);
                let draws = live.draws.clone();
                if let Some(mut transform) = world.get_mut::<Transform>(root) {
                    *transform = pose;
                }
                if let Some(mut visibility) = world.get_mut::<Visibility>(root) {
                    *visibility = Visibility::Inherited;
                }
                if held {
                    set_source_nodes_active(world, root, true);
                }
                enable_renderers(world, kind, &draws);
                let installed = installed_systems(world, root);
                if installed > 0 {
                    info!("[site-move] effect {kind:?} played from its pool: {installed} systems prepared {age:.2}s before");
                } else {
                    warn!(
                        "[site-move] effect {kind:?} played from its pool {age:.2}s after its copy was made, before any of its systems was prepared; they start when prepared"
                    );
                }
                root
            }
            None => {
                // Nothing in the pool (its prefab has not loaded or failed).
                let Some(root) = self.instantiate(world, kind, pose, false) else {
                    return false;
                };
                warn!("[site-move] effect {kind:?} emitted with no prepared copy in its pool");
                if let Some(live) = self.live.iter_mut().find(|live| live.root == root) {
                    live.played_at = Some(0.0);
                }
                root
            }
        };
        if kind == EffectType::Flying {
            self.flying = Some(root);
            if let Some(stopped) = self.stopped.take().filter(|stopped| *stopped != root) {
                self.live.retain(|live| live.root != stopped);
                if let Ok(entity) = world.get_entity_mut(stopped) {
                    entity.despawn();
                }
            }
        }
        true
    }

    fn instantiate(
        &mut self,
        world: &mut World,
        kind: EffectType,
        pose: Transform,
        pooled: bool,
    ) -> Option<Entity> {
        let pool = self.pools.iter_mut().find(|pool| pool.kind == kind)?;
        let scene = world
            .resource::<Assets<Gltf>>()
            .get(&pool.glb)
            .and_then(|gltf| gltf.default_scene.clone());
        let Some(scene) = scene else {
            if !pool.warned {
                warn!(
                    "[site-move] effect {:?} ({}) prefab {} unavailable: not shown",
                    kind,
                    kind as u8,
                    kind.glb_path().unwrap_or_default()
                );
                pool.warned = true;
            }
            return None;
        };
        let root = world
            .spawn((
                SceneRoot(scene),
                pose,
                // A pool copy stays hidden until its source nodes are held
                // inactive (the first frame its instance exists).
                if pooled {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
                SiteMoveOwned,
                PendingInstance,
                Name::new(format!("site-move effect {}", kind.leaf())),
            ))
            .id();
        self.live.push(Live {
            kind,
            root,
            planned: false,
            age: 0.0,
            pooled,
            held: false,
            played_at: None,
            draws: Vec::new(),
        });
        Some(root)
    }

    /// `EffectManager.Stop(Flying)`.
    pub(crate) fn stop_flying(&mut self, world: &mut World) -> bool {
        let Some(root) = self.flying.take() else {
            return false;
        };
        if installed_systems(world, root) == 0 {
            warn!("[site-move] effect Flying stopped with none of its systems prepared: not shown");
        }
        // ManagedEffect.Stop: ParticleSystem.Stop(withChildren, StopEmitting);
        // the copy stays active and following, its particles finish.
        crate::weather_fx::fixture::stop_emitting(world, root);
        self.stopped = Some(root);
        true
    }

    /// The flying copy the camera carries: the playing one, else the
    /// stopped one whose particles are finishing.
    pub(crate) fn flying_root(&self) -> Option<Entity> {
        self.flying.or(self.stopped)
    }

    /// Keep the pools filled, prepare the particles of new instances and
    /// release finished landing effects.
    pub(crate) fn advance(&mut self, world: &mut World, dt: f32) {
        if let Some(root) = self.flying {
            trace_draws(world, "Flying", root);
        }
        self.fill_pools(world);
        for index in 0..self.live.len() {
            let (kind, root, planned) = {
                let live = &mut self.live[index];
                live.age += dt;
                (live.kind, live.root, live.planned)
            };
            if world.get::<InstanceReady>(root).is_none() {
                continue;
            }
            if self.live[index].pooled && !self.live[index].held {
                // Inactive in the pool: its source nodes are hidden and its
                // particle clocks do not run. The scene root itself becomes
                // visible in the same step, because the particle draws
                // (children of the root) only become ready once the renderer
                // has seen them.
                set_source_nodes_active(world, root, false);
                if let Some(mut visibility) = world.get_mut::<Visibility>(root) {
                    *visibility = Visibility::Inherited;
                }
                self.live[index].held = true;
            }
            if planned {
                continue;
            }
            let pool = self
                .pools
                .iter_mut()
                .find(|pool| pool.kind == kind)
                .expect("pool per kind");
            let doc = world
                .resource::<Assets<JsonAsset>>()
                .get(&pool.doc)
                .map(|json| json.0.clone());
            let Some(doc) = doc else {
                if !pool.warned {
                    warn!(
                        "[site-move] effect {:?} particle document {} unavailable: not shown",
                        kind,
                        kind.doc_path()
                    );
                    pool.warned = true;
                }
                self.live[index].planned = true;
                continue;
            };
            let doc: Value = match serde_json::from_str(&doc) {
                Ok(doc) => doc,
                Err(error) => {
                    warn!("[site-move] effect {kind:?} particle document unreadable: {error}");
                    self.live[index].planned = true;
                    continue;
                }
            };
            let paths = super::node_paths(world, root);
            if kind == EffectType::Flying {
                // Explicit Play of the root system with its children. An
                // emitter the particle host refuses is skipped with a WARN
                // and the others play, as the fixture path treats each
                // refused emitter (the preparation judges every selected
                // emitter before it builds anything, so a retry without the
                // refused one starts clean). The preparation readies its
                // emitters one after another.
                let mut selected = select_all(&doc, &paths);
                match prepare_skipping_refused(world, root, &doc, &mut selected, "Flying") {
                    Prepared::Ready(draws) => {
                        let authored = authored_renderers(world, &doc, &draws);
                        let live = &mut self.live[index];
                        info!(
                            "[site-move] effect Flying prepared: {} systems, {:.2}s after its copy was made ({})",
                            draws.len(),
                            live.age,
                            if live.pooled {
                                "inactive in its pool"
                            } else {
                                "playing"
                            }
                        );
                        live.planned = true;
                        live.draws = authored;
                        if !live.pooled {
                            // Played before its systems were prepared:
                            // they start now, and so do their renderers.
                            let draws = live.draws.clone();
                            enable_renderers(world, kind, &draws);
                        }
                    }
                    Prepared::Pending => {}
                    Prepared::Refused(error) => {
                        warn!("[site-move] effect Flying particles refused: {error}");
                        self.live[index].planned = true;
                    }
                }
            } else {
                let anchors: HashMap<String, Vec<Entity>> = paths
                    .iter()
                    .map(|(path, list)| (format!("/{path}"), list.clone()))
                    .collect();
                let server = world.resource::<AssetServer>().clone();
                let mut commands = world.commands();
                crate::weather_fx::fixture::plan(&mut commands, root, &doc, &anchors, &server);
                world.flush();
                self.live[index].planned = true;
            }
        }
        // A played landing effect is released when every one of its systems
        // has finished (non-looping, played through its duration, no live
        // particle left).
        let mut finished = Vec::new();
        self.live.retain(|live| {
            let Some(played_at) = live.played_at else {
                return true;
            };
            if live.kind == EffectType::Flying || !live.planned {
                return true;
            }
            let done = match systems_finished(world, live.root) {
                Some(true) => true,
                Some(false) => false,
                // Nothing installed: no document, or the host refused it.
                None => live.age - played_at >= UNINSTALLED_RELEASE_AGE,
            };
            if done {
                finished.push(live.root);
            }
            !done
        });
        for root in finished {
            if let Ok(entity) = world.get_entity_mut(root) {
                entity.despawn();
            }
        }
    }
}

/// Particle systems installed under an instance.
fn installed_systems(world: &World, root: Entity) -> usize {
    world
        .get::<Children>(root)
        .map(|children| {
            children
                .iter()
                .filter(|child| {
                    world
                        .get::<crate::uber_particle::FixtureParticleLive>(*child)
                        .is_some()
                })
                .count()
        })
        .unwrap_or(0)
}

/// Pair each prepared control draw with its emitter's authored
/// `renderer.enabled`, found by the emitter node the draw's system runs.
pub(crate) fn authored_renderers(
    world: &World,
    doc: &Value,
    draws: &[Entity],
) -> Vec<(Entity, bool)> {
    let emitters = doc["emitters"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    draws
        .iter()
        .map(|&draw| {
            let node = world
                .get::<crate::uber_particle::FixtureParticleLive>(draw)
                .map(|live| live.0.node.clone())
                .unwrap_or_default();
            let emitter = emitters.iter().find(|emitter| emitter["node"] == node.as_str());
            let Some(emitter) = emitter else {
                error!("[effects] prepared draw for {node:?} has no emitter in its document; its renderer stays off");
                return (draw, false);
            };
            (draw, emitter["renderer"]["enabled"] == true)
        })
        .collect()
}

/// The Play step (`SetActive(true)` + `Play`): each prepared draw's renderer
/// takes its emitter's authored `enabled` (a renderer the prefab ships off
/// stays off; nothing turns renderers on or off afterwards).
pub(crate) fn enable_renderers(world: &mut World, kind: EffectType, draws: &[(Entity, bool)]) {
    if draws.is_empty() {
        return;
    }
    let mut on = 0;
    for &(draw, enabled) in draws {
        if let Some(mut source) = world.get_mut::<crate::source_particle::SourceParticle>(draw) {
            source.enabled = enabled;
            on += usize::from(enabled);
        }
    }
    info!(
        "[effects] effect {kind:?} ({}) plays: {on} of {} prepared renderers on, as authored",
        kind as u8,
        draws.len()
    );
}

/// Per-frame trace of an effect instance's draws (`MOLY_EFFECT_TRACE=1`,
/// an instrument; off by default): the renderer's enabled flag, the draw's
/// inherited visibility and the live particle count of its system after the
/// last particle step.
pub(crate) fn trace_draws(world: &World, label: &str, root: Entity) {
    if !trace_enabled() {
        return;
    }
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let Some(children) = world.get::<Children>(root) else {
        info!("[effect-trace] frame {frame} {label}: no draws");
        return;
    };
    for child in children.iter() {
        let Some(live) = world.get::<crate::uber_particle::FixtureParticleLive>(child) else {
            continue;
        };
        let enabled = world
            .get::<crate::source_particle::SourceParticle>(child)
            .map(|source| source.enabled);
        let visible = world.get::<InheritedVisibility>(child).map(|v| v.get());
        info!(
            "[effect-trace] frame {frame} {label} draw {} enabled={} inherited_visible={} live={} head={:.3}",
            live.0.node,
            enabled.map_or("none".into(), |e| e.to_string()),
            visible.map_or("none".into(), |v| v.to_string()),
            live.0.pool.len(),
            live.0.playback_head
        );
    }
}

pub(crate) fn trace_enabled() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| {
        let on = std::env::var("MOLY_EFFECT_TRACE").is_ok_and(|value| value == "1");
        if on {
            warn!(
                "[effect-trace] MOLY_EFFECT_TRACE=1: per-frame effect draw trace on (instrument)"
            );
        }
        on
    })
}

/// An instance whose particle systems never installed is released after
/// this age (product housekeeping; nothing of it is drawn).
const UNINSTALLED_RELEASE_AGE: f32 = 10.0;

/// Some(true) when every installed system under the instance is done;
/// None when none is installed.
pub(crate) fn systems_finished(world: &World, root: Entity) -> Option<bool> {
    let children = world.get::<Children>(root)?;
    let mut any = false;
    for child in children.iter() {
        let Some(live) = world.get::<crate::uber_particle::FixtureParticleLive>(child) else {
            continue;
        };
        any = true;
        let system = &live.0;
        if system.emitter.looping
            || !system.pool.is_empty()
            || system.playback_head < system.emitter.duration
        {
            return Some(false);
        }
    }
    any.then_some(true)
}

/// Outcome of a control preparation that skips refused emitters.
pub(crate) enum Prepared {
    /// Not ready yet; call again next frame (with the same root).
    Pending,
    /// Every remaining selected emitter has its draw and system.
    Ready(Vec<Entity>),
    /// The host refused the last remaining emitter (or the document).
    Refused(String),
}

/// Explicit Play of a prefab's root system with its children: prepare the
/// selected emitters as control-driven systems. An emitter the particle host
/// refuses is skipped with a WARN and the others play, as the fixture path
/// treats each refused emitter (the preparation judges every selected
/// emitter before it builds anything, so a retry without the refused one
/// starts clean). The preparation readies its emitters one after another.
pub(crate) fn prepare_skipping_refused(
    world: &mut World,
    root: Entity,
    doc: &Value,
    selected: &mut Vec<(Entity, usize)>,
    label: &str,
) -> Prepared {
    loop {
        match crate::weather_fx::fixture::prepare_control(world, root, doc, selected) {
            Ok(Some(draws)) => return Prepared::Ready(draws),
            Ok(None) => return Prepared::Pending,
            Err(error) => {
                let refused = selected.iter().position(|&(_, ordinal)| {
                    let node = &doc["emitters"][ordinal]["node"];
                    error.starts_with(&format!("{node}: source particle control rejected"))
                });
                match refused {
                    Some(position) if selected.len() > 1 => {
                        warn!("[effects] effect {label} emitter skipped: {error}");
                        selected.remove(position);
                    }
                    _ => return Prepared::Refused(error),
                }
            }
        }
    }
}

/// Every emitter of the document with a source-owned renderer whose node is
/// among `paths`, as `(anchor, emitter ordinal)`.
pub(crate) fn select_all(
    doc: &Value,
    paths: &HashMap<String, Vec<Entity>>,
) -> Vec<(Entity, usize)> {
    let Some(emitters) = doc["emitters"].as_array() else {
        return Vec::new();
    };
    emitters
        .iter()
        .enumerate()
        // A system without a source-owned renderer draws nothing (the root
        // system's renderer is off); Play still runs it, invisibly.
        .filter(|(_, emitter)| crate::weather_fx::fixture::is_source_particle(emitter))
        .filter_map(|(ordinal, emitter)| {
            let node = emitter["node"].as_str()?;
            let anchor = paths.get(node)?.first().copied()?;
            Some((anchor, ordinal))
        })
        .collect()
}

/// `SetActive` on the instance's source nodes (the prefab's top objects, the
/// children of the scene root; the scene root itself is not a source node).
fn set_source_nodes_active(world: &mut World, root: Entity, active: bool) {
    let nodes: Vec<Entity> = world
        .get::<Children>(root)
        .map(|children| children.to_vec())
        .unwrap_or_default();
    for entity in nodes {
        world.commands().queue(SetSourceActive { entity, active });
    }
    world.flush();
}

/// `FollowEffect.LateUpdate`: the flying effect takes the camera's pose. Its
/// subtree's global transforms are written here too, because this runs after
/// transform propagation and the particle step reads them this frame.
pub(crate) fn follow_camera(
    root: Entity,
    camera: GlobalTransform,
    nodes: &mut Query<(&mut Transform, &mut GlobalTransform), Without<Camera3d>>,
    children: &Query<&Children>,
) {
    if let Ok((mut local, _)) = nodes.get_mut(root) {
        *local = camera.compute_transform();
    }
    let mut stack = vec![(root, camera)];
    while let Some((entity, global)) = stack.pop() {
        if let Ok((_, mut own)) = nodes.get_mut(entity) {
            *own = global;
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids.iter() {
                if let Ok((local, _)) = nodes.get(kid) {
                    stack.push((kid, global.mul_transform(*local)));
                }
            }
        }
    }
}
