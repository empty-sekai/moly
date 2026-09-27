//! Installation of the player's body: the avatar model file, its default
//! scene under the player entity, and its motion group.
//!
//! The model file carries its animations, so the glTF loader itself makes the
//! scene's model root the animation root: the animator sits on that node and
//! every node below it carries its target id. This module only adds the graph
//! and the crossfade component to that animator, reads the motion manifest
//! (clip lengths, loop flags, events) beside the file, and hands both to one
//! [`AvatarDriver`]. A missing clip, a second animator or an absent file is a
//! loud refusal.

use super::{AvatarDriver, BodyClip, BodyClips, IDLE_CLIP};
use crate::avatar_material::AudienceBody;
use crate::character::CharacterShell;
use bevy::animation::AnimationClip;
use bevy::asset::{AssetId, AssetPath, LoadState, RecursiveDependencyLoadState};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::observer::On;
use bevy::gltf::Gltf;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneRoot};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;
use std::sync::Arc;

/// The avatar model: the audience mesh on its own skeleton, with the player
/// motion group as its animations.
pub(crate) const AVATAR_MODEL: &str = "moly://avatar/motion/mysekai__player_avatar.glb";
/// The motion group's manifest beside it.
pub(crate) const AVATAR_MANIFEST: &str =
    "moly://avatar/motion/mysekai__player_avatar.motion-manifest.json";

/// The body input: which model file and which motion manifest.
#[derive(Component, Clone, Debug)]
pub(crate) struct PlayerBody {
    pub(crate) model: &'static str,
    pub(crate) manifest: &'static str,
}

impl PlayerBody {
    pub(crate) fn avatar() -> Self {
        Self {
            model: AVATAR_MODEL,
            manifest: AVATAR_MANIFEST,
        }
    }
}

#[derive(Component)]
pub(crate) struct BodyLoad {
    gltf: Handle<Gltf>,
    manifest: Handle<JsonAsset>,
}

/// The body's scene root, a child of the player entity.
#[derive(Component)]
pub(crate) struct BodyModel;

#[derive(Component)]
pub(crate) struct BodyAttached;

#[derive(Component)]
pub(crate) struct BodySceneReady;

/// Clip names by handle (the model file's animation table), for the probe.
#[derive(Component, Clone)]
pub(crate) struct BodyNames(pub HashMap<AssetId<AnimationClip>, String>);

/// Update: request the body's model file and manifest.
pub(crate) fn request(
    mut commands: Commands,
    server: Res<AssetServer>,
    players: Query<(Entity, &PlayerBody), Without<BodyLoad>>,
) {
    for (player, body) in &players {
        commands.entity(player).insert(BodyLoad {
            gltf: moly_assets::residency::load_gltf(
                &server,
                AssetPath::from(body.model.to_owned()),
                moly_assets::residency::GltfResidency::Character,
            ),
            manifest: server.load::<JsonAsset>(AssetPath::from(body.manifest.to_owned())),
        });
        info!(
            "[player] body requested: {} + {}",
            body.model, body.manifest
        );
    }
}

/// Update: the model file is in; its default scene goes under the player.
pub(crate) fn attach(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    players: Query<(Entity, &PlayerBody, &BodyLoad), Without<BodyAttached>>,
) {
    for (player, body, load) in &players {
        if let LoadState::Failed(error) = server.load_state(&load.gltf) {
            panic!("player body {} failed to load: {error:?}", body.model);
        }
        if let RecursiveDependencyLoadState::Failed(error) =
            server.recursive_dependency_load_state(&load.gltf)
        {
            panic!("player body {} dependencies failed: {error:?}", body.model);
        }
        if !server.is_loaded_with_dependencies(&load.gltf) {
            continue;
        }
        let gltf = gltfs.get(&load.gltf).expect("load gate passed");
        let Some(scene) = gltf.default_scene.clone() else {
            panic!("player body {} has no default scene", body.model);
        };
        commands
            .entity(player)
            .with_child((SceneRoot(scene), BodyModel))
            .insert(BodyAttached);
        info!("[player] body attached: {}", body.model);
    }
}

/// Observer: the body's scene has spawned.
pub(crate) fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    models: Query<&BodyModel>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let model = trigger.event().entity;
    if models.get(model).is_err() {
        return;
    }
    let player = parents.get(model).expect("body model has a parent").0;
    commands.entity(player).insert(BodySceneReady);
}

/// Update: the scene and the manifest are in; wire the loader's animator and
/// install the driver.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn wire(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    jsons: Res<Assets<JsonAsset>>,
    mesh_assets: Res<Assets<Mesh>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    players: Query<(Entity, &PlayerBody, &BodyLoad), With<BodySceneReady>>,
    children: Query<&Children>,
    models: Query<(), With<BodyModel>>,
    animators: Query<(), With<AnimationPlayer>>,
    names: Query<&Name>,
    meshes_3d: Query<&Mesh3d>,
    skinned: Query<(), With<SkinnedMesh>>,
    globals: Query<&GlobalTransform>,
) {
    for (player, body, load) in &players {
        if let LoadState::Failed(error) = server.load_state(&load.manifest) {
            panic!(
                "player motion manifest {} failed to load: {error:?}",
                body.manifest
            );
        }
        let Some(manifest) = jsons.get(&load.manifest) else {
            continue;
        };
        let gltf = gltfs.get(&load.gltf).expect("attached body is loaded");
        let model = children
            .get(player)
            .ok()
            .and_then(|kids| kids.iter().find(|kid| models.get(*kid).is_ok()))
            .expect("BodySceneReady comes from a BodyModel child");
        // The loader's animator: exactly one node of the body carries it.
        let mut stack = vec![model];
        let mut found = Vec::new();
        let mut mesh_entities = 0usize;
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(-f32::MAX);
        while let Some(entity) = stack.pop() {
            if animators.get(entity).is_ok() {
                found.push(entity);
            }
            if let Ok(mesh3d) = meshes_3d.get(entity) {
                mesh_entities += 1;
                if skinned.get(entity).is_ok() {
                    // Bind-pose bounds do not follow the animated joints.
                    commands.entity(entity).insert(NoFrustumCulling);
                }
                let mesh = mesh_assets
                    .get(&mesh3d.0)
                    .expect("body mesh is loaded with its model");
                let positions = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|values| values.as_float3())
                    .expect("body mesh has float3 positions");
                let global = globals
                    .get(entity)
                    .expect("body mesh has a GlobalTransform");
                for position in positions {
                    let world = global.transform_point(Vec3::from(*position));
                    min = min.min(world);
                    max = max.max(world);
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        let [animator] = found.as_slice() else {
            panic!(
                "player body {} has {} animators; the loader gives the animated model root exactly one",
                body.model,
                found.len()
            );
        };
        if mesh_entities == 0 {
            panic!("player body {} has no mesh", body.model);
        }
        let clips = read_manifest(&manifest.0, gltf, body);
        let clip_names = BodyNames(
            clips
                .0
                .values()
                .map(|clip| (clip.handle.id(), clip.name.clone()))
                .collect(),
        );
        let clip_count = clips.0.len();
        let graph = graphs.add(AnimationGraph::new());
        commands.entity(*animator).insert((
            AnimationGraphHandle(graph.clone()),
            AnimationTransitions::new(),
        ));
        let driver = AvatarDriver::new(*animator, model, graph, Arc::new(clips));
        let idle_length = driver.clip_length(IDLE_CLIP).unwrap_or(f32::NAN);
        let height = max.y - min.y;
        commands.entity(player).remove::<BodySceneReady>().insert((
            driver,
            clip_names,
            AudienceBody,
            CharacterShell {
                height,
                lowest: min.y,
            },
        ));
        info!(
            "[player] body wired: {} animator {:?} ({}) model {model:?}, {mesh_entities} mesh entities, height {height:.3}, {clip_count} clips, idle {IDLE_CLIP} {idle_length:.3}s",
            body.model,
            animator,
            names.get(*animator).map_or("unnamed", Name::as_str),
        );
    }
}

/// The manifest's clip rows joined to the model file's animations by name.
fn read_manifest(raw: &str, gltf: &Gltf, body: &PlayerBody) -> BodyClips {
    let value: serde_json::Value = serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("{}: not JSON: {error}", body.manifest));
    let rows = value["clips"]
        .as_array()
        .unwrap_or_else(|| panic!("{}: no clips list", body.manifest));
    let mut clips = HashMap::new();
    for row in rows {
        let name = row["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: clip row without a name", body.manifest));
        let container = row["container"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: clip {name} without a container", body.manifest));
        let stem = container
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".anim"))
            .unwrap_or_else(|| panic!("{}: clip {name} container is not an .anim", body.manifest));
        if stem != name.to_lowercase() {
            panic!(
                "{}: clip {name} file {stem} is not its lower-case name; the source literal would not resolve",
                body.manifest
            );
        }
        let handle =
            gltf.named_animations.get(name).cloned().unwrap_or_else(|| {
                panic!("{}: clip {name} is not in {}", body.manifest, body.model)
            });
        let length = row["durationSeconds"]
            .as_f64()
            .unwrap_or_else(|| panic!("{}: clip {name} without a length", body.manifest))
            as f32;
        let looping = row["loopTime"]
            .as_bool()
            .unwrap_or_else(|| panic!("{}: clip {name} without a loop flag", body.manifest));
        let events = row["events"]
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .map(|event| {
                        (
                            event["time"].as_f64().unwrap_or(f64::NAN) as f32,
                            event["functionName"].as_str().unwrap_or("").to_owned(),
                            event["stringParameter"].as_str().unwrap_or("").to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if clips
            .insert(
                stem.to_owned(),
                BodyClip {
                    name: name.to_owned(),
                    handle,
                    length,
                    looping,
                    events,
                },
            )
            .is_some()
        {
            panic!("{}: two clips resolve to {stem}", body.manifest);
        }
    }
    BodyClips(clips)
}
