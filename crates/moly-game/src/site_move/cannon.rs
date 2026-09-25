//! The cannon prop of the move: `CreateCannon` instantiates the site-move
//! bundle's `cannon` prefab at the player's view transform and unparents it;
//! `SiteMoveCannon` drives it.
//!
//! - `Awake`: animator off, `_cannonObject` (the skinned cannon mesh node)
//!   inactive, `_fireEffect.Stop()`.
//! - `PlayAnimation(Start)`: the mesh node active again, animator on, scale
//!   one, then `Animator.Play` of the controller's one state. `Awake` and
//!   `Start` run in the same continuation after the load, so no frame shows
//!   the prefab state in between.
//! - The clip carries three `m_IsActive` curves (the appear, boarding and
//!   takeoff effect groups: inactive at 0, active from their key on) and one
//!   `PlayEffect` event (`_fireEffect.Play()`). The takeoff group is the
//!   `_fireEffect`'s own group and is already playing (play-on-awake on its
//!   activation 0.05 s earlier) when the event arrives; `Play` on a playing
//!   system does not restart it, so the event has no visible effect here.
//!   (Engine behaviour of `ParticleSystem.Play` on a playing system is taken
//!   from the engine documentation, not read natively.)
//! - `PlayAnimation(End)`: `DOScale(Vector3.zero, 0.5f)` with `Ease.InBack`
//!   on the mesh node, then `Destroy` of the whole prefab. The mesh node is a
//!   skinned mesh whose vertices follow the bones, so scaling the node's own
//!   transform does not shrink what is drawn in either engine (named: the
//!   data says so and it is ported as the data says); the prefab vanishes on
//!   the tween's completion.

use bevy::animation::graph::AnimationNodeIndex;
use bevy::gltf::{Gltf, GltfMaterialName};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::SetSourceActive;
use serde_json::Value;
use std::collections::HashMap;

use super::timeline::{
    in_back, TweenClock, CANNON_GROUP_ACTIVATION, CANNON_PLAY_EFFECT_AT, CANNON_SHRINK_TIME,
};
use super::{AssetState, InstanceReady, PendingInstance, SiteMoveOwned};

const GLB: &str = "moly://site/travel/cannon/cannon.glb";
const SIDECAR: &str = "moly://site/travel/cannon/cannon.json";
const TEXTURE_DIR: &str = "site/travel/cannon";
/// The cannon's particle package in the fixture-particles-v2 index.
pub(crate) const PARTICLE_PACKAGE: &str = "mysekai__site__move__cannon";
/// The one clip of the bundle; the controller `ac_cannon` has one state that
/// plays it (the code's lower-case state name hashes to no state, so the
/// default state's entry plays this clip).
const CLIP: &str = "fixture_u000_site_cannon001_O";
/// `SiteMoveCannon._cannonObject`.
const MESH_NODE: &str = "mdl_site_tool_cannon01_01";
/// The prefab root node name; the curve paths are relative to it.
const PREFAB_ROOT: &str = "cannon";
const BASE_MATERIAL: &str = "mat_base";

/// Loads requested in the pre-action (the source loads the bundle inside
/// `CreateCannon`; preloading keeps the Core's own wait at its minimum, one
/// frame, as a cached bundle gives it).
pub(crate) struct CannonAssets {
    glb: Handle<Gltf>,
    sidecar: Handle<JsonAsset>,
    particles: Particles,
    particles_warned: bool,
}

/// The particle document, requested only when the release root lists its
/// package (see `products`).
enum Particles {
    /// The index has not been read yet.
    Undecided,
    NotListed,
    Requested(Handle<JsonAsset>),
}

impl CannonAssets {
    pub(crate) fn request(server: &AssetServer) -> Self {
        Self {
            glb: server.load(GLB),
            sidecar: server.load(SIDECAR),
            particles: Particles::Undecided,
            particles_warned: false,
        }
    }

    /// Request the particle document once the product check has decided.
    pub(crate) fn decide(&mut self, server: &AssetServer, listed: Option<bool>) {
        if matches!(self.particles, Particles::Undecided) {
            match listed {
                Some(true) => {
                    self.particles = Particles::Requested(server.load(format!(
                        "moly://fixture-particles-v2/{PARTICLE_PACKAGE}.json"
                    )));
                }
                Some(false) => self.particles = Particles::NotListed,
                None => {}
            }
        }
    }

    /// Ready when the prefab and its sidecar are loaded. The particle
    /// document is optional: its absence is one WARN and no particles.
    pub(crate) fn state(&self, server: &AssetServer) -> AssetState {
        let mut state = AssetState::Ready;
        for (name, loaded, failed) in [
            (
                "cannon glb",
                server.is_loaded_with_dependencies(&self.glb),
                server.load_state(&self.glb).is_failed(),
            ),
            (
                "cannon sidecar",
                server.load_state(&self.sidecar).is_loaded(),
                server.load_state(&self.sidecar).is_failed(),
            ),
        ] {
            if failed {
                return AssetState::Failed(format!("{name} failed to load"));
            }
            if !loaded {
                state = AssetState::Pending;
            }
        }
        match &self.particles {
            Particles::Undecided => state = AssetState::Pending,
            Particles::NotListed => {}
            Particles::Requested(particles) => {
                if !server.load_state(particles).is_loaded()
                    && !server.load_state(particles).is_failed()
                {
                    state = AssetState::Pending;
                }
            }
        }
        state
    }
}

/// Marker on the cannon instance root.
#[derive(Component)]
pub(crate) struct CannonRoot;

pub(crate) struct Cannon {
    pub(crate) root: Entity,
    started: bool,
    mesh: Option<Entity>,
    groups: Vec<(Entity, f32, bool)>,
    animator: Option<(Entity, AnimationNodeIndex)>,
    clock: f32,
    play_effect_logged: bool,
    shrink: Option<TweenClock>,
}

/// `CreateCannon`: instantiate at the player's view transform (local
/// position/rotation identity under it, then unparented, so the world pose
/// is the player's).
pub(crate) fn spawn(world: &mut World, assets: &CannonAssets, pose: Transform) -> Option<Cannon> {
    let scene = {
        let gltfs = world.resource::<Assets<Gltf>>();
        let gltf = gltfs.get(&assets.glb)?;
        gltf.default_scene.clone()?
    };
    let root = world
        .spawn((
            SceneRoot(scene),
            pose.with_scale(Vec3::ONE),
            Visibility::Inherited,
            CannonRoot,
            SiteMoveOwned,
            PendingInstance,
            Name::new("site-move cannon"),
        ))
        .id();
    Some(Cannon {
        root,
        started: false,
        mesh: None,
        groups: Vec::new(),
        animator: None,
        clock: 0.0,
        play_effect_logged: false,
        shrink: None,
    })
}

impl Cannon {
    pub(crate) fn instance_ready(&self, world: &World) -> bool {
        world.get::<InstanceReady>(self.root).is_some()
    }

    /// `Awake` + `PlayAnimation(Start)` on the frame the instance exists.
    pub(crate) fn start(&mut self, world: &mut World, assets: &mut CannonAssets) {
        let paths = super::node_paths(world, self.root);
        let find = |path: &str| paths.get(path).and_then(|list| list.first()).copied();
        self.mesh = find(&format!("{PREFAB_ROOT}/{MESH_NODE}"));
        if self.mesh.is_none() {
            error!("[site-move] cannon prefab has no {MESH_NODE} node");
        }
        for (path, at) in CANNON_GROUP_ACTIVATION {
            match find(&format!("{PREFAB_ROOT}/{path}")) {
                // Frame 0 of the clip: every group inactive.
                Some(entity) => {
                    world.commands().queue(SetSourceActive {
                        entity,
                        active: false,
                    });
                    self.groups.push((entity, at, false));
                }
                None => error!("[site-move] cannon prefab has no curve target {path}"),
            }
        }
        // Awake hides the mesh node and Start shows it again in the same
        // continuation: net active, scale one.
        if let Some(mesh) = self.mesh {
            world.commands().queue(SetSourceActive {
                entity: mesh,
                active: true,
            });
        }
        self.animator = start_clip(world, &assets.glb, self.root);
        swap_base_material(world, &assets.sidecar, self.root);
        plan_particles(world, assets, self.root, &paths);
        world.flush();
        self.started = true;
        info!(
            "[site-move] cannon Start: mesh {:?}, {} curve groups, clip {}",
            self.mesh,
            self.groups.len(),
            if self.animator.is_some() {
                "playing"
            } else {
                "missing"
            }
        );
    }

    pub(crate) fn started(&self) -> bool {
        self.started
    }

    /// The clip clock of this frame (the Animator advances by the frame's
    /// delta from the frame it starts, the same convention as the tweens).
    /// Returns true on the frame the `PlayEffect` event passes.
    pub(crate) fn advance(&mut self, world: &mut World, dt: f32) -> bool {
        if !self.started {
            return false;
        }
        self.clock += dt;
        if let Some((animator, node)) = self.animator {
            if let Some(mut player) = world.get_mut::<AnimationPlayer>(animator) {
                if let Some(active) = player.animation_mut(node) {
                    active.seek_to(self.clock);
                }
            }
        }
        for (entity, at, active) in &mut self.groups {
            if !*active && self.clock >= *at {
                *active = true;
                world.commands().queue(SetSourceActive {
                    entity: *entity,
                    active: true,
                });
            }
        }
        let mut event = false;
        if !self.play_effect_logged && self.clock >= CANNON_PLAY_EFFECT_AT {
            self.play_effect_logged = true;
            event = true;
        }
        if let Some(shrink) = &mut self.shrink {
            let e = shrink.advance(dt);
            let scale = 1.0 - in_back(e);
            if let Some(mesh) = self.mesh {
                if let Some(mut transform) = world.get_mut::<Transform>(mesh) {
                    transform.scale = Vec3::splat(scale);
                }
            }
        }
        world.flush();
        event
    }

    /// `PlayAnimation(End)`.
    pub(crate) fn end(&mut self) {
        self.shrink = Some(TweenClock::new(CANNON_SHRINK_TIME));
    }

    /// True once the shrink completed (`OnComplete` destroys the prefab).
    pub(crate) fn destroyed(&self) -> bool {
        self.shrink.is_some_and(|shrink| shrink.done())
    }

    pub(crate) fn despawn(&self, world: &mut World) {
        if let Ok(entity) = world.get_entity_mut(self.root) {
            entity.despawn();
        }
    }

    pub(crate) fn clock(&self) -> f32 {
        self.clock
    }
}

fn start_clip(
    world: &mut World,
    glb: &Handle<Gltf>,
    root: Entity,
) -> Option<(Entity, AnimationNodeIndex)> {
    let clip = world
        .resource::<Assets<Gltf>>()
        .get(glb)
        .and_then(|gltf| gltf.named_animations.get(CLIP).cloned());
    let Some(clip) = clip else {
        error!("[site-move] cannon glb has no clip {CLIP}");
        return None;
    };
    let animator = super::descendants(world, root)
        .into_iter()
        .find(|entity| world.get::<AnimationPlayer>(*entity).is_some());
    let Some(animator) = animator else {
        error!("[site-move] cannon instance has no animation player");
        return None;
    };
    let (graph, node) = AnimationGraph::from_clip(clip);
    let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
    world
        .entity_mut(animator)
        .insert(AnimationGraphHandle(graph));
    let mut player = world
        .get_mut::<AnimationPlayer>(animator)
        .expect("found above");
    // The executor owns the clock: paused, sought every frame.
    player.play(node).pause().seek_to(0.0);
    Some((animator, node))
}

/// `mat_base` is `Mysekai/Object`: installed through the site material, as
/// the site's own object materials are. A refusal keeps the imported
/// material and is named.
fn swap_base_material(world: &mut World, sidecar: &Handle<JsonAsset>, root: Entity) {
    let Some(doc) = world
        .resource::<Assets<JsonAsset>>()
        .get(sidecar)
        .map(|json| json.0.clone())
    else {
        return;
    };
    let sidecar = match moly_assets::sidecar::parse_site_sidecar(doc.as_bytes()) {
        Ok(sidecar) => sidecar,
        Err(error) => {
            warn!("[site-move] cannon sidecar unreadable, material kept: {error:?}");
            return;
        }
    };
    let Some(slot) = sidecar.materials.iter().find(|m| m.name == BASE_MATERIAL) else {
        warn!("[site-move] cannon sidecar has no {BASE_MATERIAL}");
        return;
    };
    let server = world.resource::<AssetServer>().clone();
    let material = match crate::site_material::resolve_object(&sidecar, slot, |uri| {
        crate::site_material::load_dir_texture(&server, &sidecar, TEXTURE_DIR, uri)
    }) {
        Ok(material) => material,
        Err(reason) => {
            warn!("[site-move] cannon {BASE_MATERIAL} refused by the object material: {reason}; imported material kept");
            return;
        }
    };
    let handle = world
        .resource_mut::<Assets<crate::site_material::SiteMaterial>>()
        .add(material);
    let targets: Vec<Entity> = super::descendants(world, root)
        .into_iter()
        .filter(|entity| {
            world
                .get::<MeshMaterial3d<StandardMaterial>>(*entity)
                .is_some()
                && world
                    .get::<GltfMaterialName>(*entity)
                    .is_some_and(|name| name.0 == BASE_MATERIAL)
        })
        .collect();
    for entity in &targets {
        world
            .entity_mut(*entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle.clone()));
    }
    info!(
        "[site-move] cannon {BASE_MATERIAL} installed on {} mesh entities",
        targets.len()
    );
}

fn plan_particles(
    world: &mut World,
    assets: &mut CannonAssets,
    root: Entity,
    paths: &HashMap<String, Vec<Entity>>,
) {
    // Not requested: the release root does not list it (one WARN at
    // startup already names it).
    let Particles::Requested(handle) = &assets.particles else {
        return;
    };
    let doc = world
        .resource::<Assets<JsonAsset>>()
        .get(handle)
        .map(|json| json.0.clone());
    let Some(doc) = doc else {
        if !assets.particles_warned {
            warn!("[site-move] cannon particle document for {PARTICLE_PACKAGE} unavailable: the cannon plays without particles");
            assets.particles_warned = true;
        }
        return;
    };
    let doc: Value = match serde_json::from_str(&doc) {
        Ok(doc) => doc,
        Err(error) => {
            warn!("[site-move] cannon particle document unreadable: {error}");
            return;
        }
    };
    let anchors: HashMap<String, Vec<Entity>> = paths
        .iter()
        .map(|(path, list)| (format!("/{path}"), list.clone()))
        .collect();
    let server = world.resource::<AssetServer>().clone();
    let mut commands = world.commands();
    crate::weather_fx::fixture::plan(&mut commands, root, &doc, &anchors, &server);
}
