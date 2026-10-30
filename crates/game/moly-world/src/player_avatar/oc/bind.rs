//! Binding a loaded OC model to the player avatar's skeleton.
//!
//! The avatar's rest is captured when its body is wired, before any clip
//! plays ([`capture_proxy`]). Once both are in, the OC's scene is spawned
//! under the avatar's animator, so that the OC and the avatar share the body
//! frame, and on its scene's arrival the retarget plan is built from both
//! rests ([`super::rig::plan`]). The plan fits the OC (its root transform),
//! aligns its rest (the bone locals written once), and lists the bones the
//! pose pass drives every frame. The OC's materials become [`OcMaterial`]s,
//! and the avatar's own body mesh and worn accessory are hidden while the OC
//! is worn; the avatar's skeleton keeps playing every clip and carries every
//! socket. A model the plan refuses is removed and the avatar's body stays.
//!
//! A character body keeps its file's node names and its materials: it is
//! shown once [`super::dress`] has given it the character material, and the
//! avatar body stays until then.

use super::choice::{OcChoice, OcChoiceBodies, OcChoiceLoad};
use super::constraint::{self, BoundConstraint, NodeConstraint};
use super::humanoid::{self, Slot, SLOT_COUNT};
use super::material::OcMaterial;
use super::rig::{self, Frame, Skeleton};
use super::source::{CharacterBody, OcSelection, OcStatus};
use super::vrm::MapSource;
use crate::avatar_wear::WearPart;
use crate::player_avatar::AvatarDriver;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::gltf::Gltf;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use std::collections::{BTreeMap, HashMap};

/// The avatar skeleton's rest: every node below its animator, parent-first,
/// with the local transforms of the file.
#[derive(Component, Clone, Debug)]
pub struct ProxyRest {
    pub animator: Entity,
    pub joints: Vec<Entity>,
    pub names: Vec<String>,
    pub skeleton: Skeleton,
}

impl ProxyRest {
    /// The avatar joint of each slot: the first node of the slot's name.
    pub fn slots(&self) -> Result<[usize; SLOT_COUNT], String> {
        let mut slots = [0usize; SLOT_COUNT];
        for slot in Slot::ALL {
            slots[slot.index()] = self
                .names
                .iter()
                .position(|name| name == slot.avatar_name())
                .ok_or_else(|| format!("the avatar skeleton has no {} joint", slot.avatar_name()))?;
        }
        Ok(slots)
    }
}

/// A transform's uniform scale (`None` when the axes differ or it mirrors).
fn uniform_scale(scale: Vec3) -> Option<f32> {
    let mean = (scale.x + scale.y + scale.z) / 3.0;
    let spread = (scale - Vec3::splat(mean)).abs().max_element();
    (mean > 1e-6 && spread <= 1e-3 * mean).then_some(mean)
}

fn frame_of(transform: &Transform) -> Frame {
    Frame::new(
        transform.translation,
        transform.rotation,
        (transform.scale.x + transform.scale.y + transform.scale.z) / 3.0,
    )
}

/// The avatar skeleton below `animator` as its transforms are now. Called
/// when the body is wired: no clip has played on it yet.
pub fn capture_proxy(
    animator: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    transforms: &Query<&Transform>,
) -> ProxyRest {
    let mut joints = Vec::new();
    let mut joint_names = Vec::new();
    let mut parent = Vec::new();
    let mut local = Vec::new();
    let mut stack: Vec<(Entity, Option<usize>)> = children
        .get(animator)
        .map(|kids| kids.iter().rev().map(|kid| (kid, None)).collect())
        .unwrap_or_default();
    while let Some((entity, up)) = stack.pop() {
        let index = joints.len();
        joints.push(entity);
        joint_names.push(names.get(entity).map_or_else(|_| String::new(), |name| name.as_str().to_owned()));
        parent.push(up);
        local.push(transforms.get(entity).map_or(Frame::IDENTITY, frame_of));
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter().rev().map(|kid| (kid, Some(index))));
        }
    }
    ProxyRest {
        animator,
        joints,
        names: joint_names,
        skeleton: Skeleton {
            parent,
            local,
            base: Frame::IDENTITY,
        },
    }
}

/// The OC's scene root, a child of the avatar's animator.
#[derive(Component, Debug)]
pub struct OcModel {
    pub generation: u64,
    pub player: Entity,
    pub name: String,
    pub source: MapSource,
    /// The file's node constraints, in evaluation order.
    pub constraints: Vec<NodeConstraint>,
    pub gltf: Handle<Gltf>,
    pub character: Option<CharacterBody>,
}

#[derive(Component)]
pub struct OcSceneReady;

/// A bound OC: the plan and everything the pose pass writes.
#[derive(Component)]
pub struct OcBinding {
    pub plan: rig::Plan,
    /// The OC bone of each driven bone of the plan, in plan order.
    pub driven: Vec<Entity>,
    /// The avatar joints, in [`ProxyRest`] order.
    pub proxy: Vec<Entity>,
    pub locals: Vec<Frame>,
    pub solution: rig::Solution,
    /// The node constraints, in evaluation order, applied after the driven
    /// bones are posed.
    pub constraints: Vec<BoundConstraint>,
    /// Socket translations written this frame: (avatar joint, written,
    /// the clip's own), restored at the end of the frame.
    pub written: Vec<(usize, Vec3, Vec3)>,
    /// Avatar meshes hidden while the OC is worn, with their prior visibility.
    pub hidden: Vec<(Entity, Visibility)>,
    pub warned: bool,
    /// The body's own clips animate its skeleton: the pose pass writes no
    /// driven bone, and [`super::pose::blend`] brings the retarget pose back
    /// when the ownership ends.
    pub own: bool,
}

/// Observer: the OC's scene has spawned.
pub fn on_scene_ready(
    trigger: On<WorldInstanceReady>,
    models: Query<(), With<OcModel>>,
    mut commands: Commands,
) {
    let model = trigger.event().entity;
    if models.get(model).is_ok() {
        commands.entity(model).insert(OcSceneReady);
    }
}

/// Puts the hidden avatar meshes back as they were.
fn reveal(binding: &OcBinding, visibilities: &mut Query<&mut Visibility>) {
    for (entity, prior) in &binding.hidden {
        if let Ok(mut visibility) = visibilities.get_mut(*entity) {
            *visibility = *prior;
        }
    }
}

/// Update: the chosen model's scene under each player's animator; models of
/// an older choice are removed. A skeleton whose holder carries its own
/// choice ([`super::OcChoice`]) wears that body instead of the global one.
pub fn attach(
    mut commands: Commands,
    server: Res<AssetServer>,
    bodies: Res<OcChoiceBodies>,
    mut selection: ResMut<OcSelection>,
    gltfs: Res<Assets<Gltf>>,
    players: Query<(Entity, &ProxyRest, Option<&OcChoice>, Option<&OcChoiceLoad>)>,
    models: Query<(Entity, &OcModel, Option<&OcBinding>)>,
    mut visibilities: Query<&mut Visibility>,
) {
    let global = selection
        .model
        .as_ref()
        .filter(|model| model.ready)
        .map(|model| model.generation);
    // A holder's own body wins over the global choice; while its file loads
    // the avatar body is shown. Per holder: (generation, source, body, model
    // file, scene).
    let mut scenes: HashMap<Entity, (u64, MapSource, CharacterBody, Handle<Gltf>, Handle<WorldAsset>)> =
        HashMap::new();
    for (player, _, choice, load) in &players {
        let (Some(choice), Some(_)) = (choice, load) else {
            continue;
        };
        let Some(chosen) = bodies.ready(&server, choice.0) else {
            continue;
        };
        let Some(scene) = gltfs
            .get(&chosen.gltf)
            .and_then(|gltf| gltf.default_scene.clone().or_else(|| gltf.scenes.first().cloned()))
        else {
            continue;
        };
        scenes.insert(
            player,
            (
                choice.0 as u64,
                MapSource::Character,
                chosen.body.clone(),
                chosen.gltf.clone(),
                scene,
            ),
        );
    }
    for (entity, model, binding) in &models {
        let own = scenes.get(&model.player).map(|(generation, ..)| *generation);
        if Some(model.generation) != own.or(global) {
            if let Some(binding) = binding {
                reveal(binding, &mut visibilities);
            }
            commands.entity(entity).despawn();
            info!("[oc] {} removed; the avatar body is shown", model.name);
        }
    }
    let own_models: HashMap<Entity, Entity> = models
        .iter()
        .filter_map(|(entity, model, _)| {
            scenes
                .get(&model.player)
                .filter(|(generation, ..)| *generation == model.generation)
                .map(|_| (model.player, entity))
        })
        .collect();
    for (player, (generation, source, character, gltf, scene)) in &scenes {
        if own_models.contains_key(player) {
            continue;
        }
        let Ok((_, proxy, _, _)) = players.get(*player) else {
            continue;
        };
        commands.spawn((
            Name::new("oc_model"),
            OcModel {
                generation: *generation,
                player: *player,
                name: character.rig_file.clone(),
                source: *source,
                constraints: Vec::new(),
                gltf: gltf.clone(),
                character: Some(character.clone()),
            },
            WorldAssetRoot(scene.clone()),
            // Hidden until bound: the file's rest is neither fitted nor posed.
            Transform::IDENTITY,
            Visibility::Hidden,
            ChildOf(proxy.animator),
        ));
    }
    let Some(generation) = global else {
        return;
    };
    let load = selection.model.as_ref().expect("global comes from it");
    let Some(gltf) = gltfs.get(&load.gltf) else {
        return;
    };
    let Some(scene) = gltf.default_scene.clone().or_else(|| gltf.scenes.first().cloned()) else {
        let name = load.name.clone();
        selection.model = None;
        selection.refuse(&name, "the file has no scene");
        return;
    };
    for (player, proxy, choice, _) in &players {
        // A holder with its own choice does not wear the global one.
        if choice.is_some() {
            continue;
        }
        if models
            .iter()
            .any(|(_, model, _)| model.player == player && model.generation == generation)
        {
            continue;
        }
        commands.spawn((
            Name::new("oc_model"),
            OcModel {
                generation,
                player,
                name: load.name.clone(),
                source: load.source,
                constraints: load.constraints.clone(),
                gltf: load.gltf.clone(),
                character: load.character.clone(),
            },
            WorldAssetRoot(scene.clone()),
            // Hidden until bound: the file's rest is neither fitted nor posed.
            Transform::IDENTITY,
            Visibility::Hidden,
            ChildOf(proxy.animator),
        ));
    }
}

/// The OC's skeleton: every humanoid bone and its ancestors below the scene
/// root, parent-first, with their entities and the bone table.
struct ModelTree {
    skeleton: Skeleton,
    entities: Vec<Entity>,
    bones: BTreeMap<&'static str, usize>,
    /// Every entity below the scene root.
    all: Vec<Entity>,
}

fn model_tree(
    root: Entity,
    source: MapSource,
    children: &Query<&Children>,
    names: &Query<&Name>,
    transforms: &Query<&mut Transform>,
) -> Result<ModelTree, String> {
    let mut order: Vec<(Entity, Option<usize>)> = Vec::new();
    let mut stack: Vec<(Entity, Option<usize>)> = children
        .get(root)
        .map(|kids| kids.iter().rev().map(|kid| (kid, None)).collect())
        .unwrap_or_default();
    while let Some((entity, up)) = stack.pop() {
        let index = order.len();
        order.push((entity, up));
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter().rev().map(|kid| (kid, Some(index))));
        }
    }
    let mut bone_at: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (index, (entity, _)) in order.iter().enumerate() {
        // A model file's copy names its humanoid bones `oc.<bone>`; a
        // character body keeps its file's node names.
        let bone = names.get(*entity).ok().and_then(|name| match source {
            MapSource::Character => humanoid::bone_from_node_name(name.as_str()),
            _ => name
                .as_str()
                .strip_prefix("oc.")
                .and_then(|bone| humanoid::canonical(bone, false)),
        });
        if let Some(bone) = bone {
            bone_at.entry(bone).or_insert(index);
        }
    }
    let mut needed = vec![false; order.len()];
    for &index in bone_at.values() {
        let mut at = Some(index);
        while let Some(joint) = at {
            if std::mem::replace(&mut needed[joint], true) {
                break;
            }
            at = order[joint].1;
        }
    }
    let mut remap = vec![usize::MAX; order.len()];
    let mut skeleton = Skeleton {
        parent: Vec::new(),
        local: Vec::new(),
        base: Frame::IDENTITY,
    };
    let mut entities = Vec::new();
    for (index, (entity, up)) in order.iter().enumerate() {
        if !needed[index] {
            continue;
        }
        let transform = transforms
            .get(*entity)
            .map_err(|_| "a model node has no transform".to_owned())?;
        let scale = uniform_scale(transform.scale).ok_or_else(|| {
            format!(
                "node {} has a non-uniform or mirroring scale {:?}",
                names.get(*entity).map_or("?", Name::as_str),
                transform.scale
            )
        })?;
        remap[index] = entities.len();
        skeleton.parent.push(up.map(|up| remap[up]));
        skeleton
            .local
            .push(Frame::new(transform.translation, transform.rotation, scale));
        entities.push(*entity);
    }
    let bones = bone_at
        .into_iter()
        .map(|(bone, index)| (bone, remap[index]))
        .collect();
    Ok(ModelTree {
        skeleton,
        entities,
        bones,
        all: order.into_iter().map(|(entity, _)| entity).collect(),
    })
}

fn set(transform: &mut Transform, frame: &Frame) {
    transform.translation = frame.pos;
    transform.rotation = frame.rot;
    transform.scale = Vec3::splat(frame.scale);
}

/// Update: a spawned OC is planned against its player's avatar skeleton,
/// fitted, given its materials and shown.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn bind(
    mut commands: Commands,
    mut selection: ResMut<OcSelection>,
    models: Query<(Entity, &OcModel), (With<OcSceneReady>, Without<OcBinding>)>,
    players: Query<&ProxyRest>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut transforms: Query<&mut Transform>,
    standard: Query<&MeshMaterial3d<StandardMaterial>>,
    skinned: Query<(), With<SkinnedMesh>>,
    standard_materials: Res<Assets<StandardMaterial>>,
    mut oc_materials: ResMut<Assets<OcMaterial>>,
    mut visibilities: Query<&mut Visibility>,
    profiles: Option<Res<super::own::CharacterProfiles>>,
) {
    for (entity, model) in &models {
        let Ok(proxy) = players.get(model.player) else {
            continue;
        };
        let planned = proxy.slots().and_then(|slots| {
            let tree = model_tree(entity, model.source, &children, &names, &transforms)?;
            let plan = rig::plan(&proxy.skeleton, slots, &tree.skeleton, &tree.bones)?;
            Ok((tree, plan))
        });
        let current = selection
            .model
            .as_ref()
            .is_some_and(|load| load.generation == model.generation);
        let (tree, plan) = match planned {
            Ok(planned) => planned,
            Err(reason) => {
                commands.entity(entity).despawn();
                if current {
                    selection.model = None;
                    selection.refuse(&model.name, &reason);
                }
                continue;
            }
        };
        // A character body's imported pose, taken before the fit, the aligned
        // rest and the first retarget write any bone, as a cast member's
        // skeleton records it: the bone cloth binds against it.
        if model.character.is_some() {
            for node in &tree.all {
                if let Ok(pose) = transforms.get(*node) {
                    commands
                        .entity(*node)
                        .insert(crate::character::CharacterRestTransform(*pose));
                }
            }
        }
        if let Ok(mut transform) = transforms.get_mut(entity) {
            set(&mut transform, &plan.fit);
        }
        for (joint, frame) in &plan.constants {
            if let Ok(mut transform) = transforms.get_mut(tree.entities[*joint]) {
                set(&mut transform, frame);
            }
        }
        // The node constraints, resolved to this scene's entities by the
        // copy's node names.
        let mut node_entities: HashMap<&str, Entity> = HashMap::new();
        let mut parents: HashMap<Entity, Entity> = HashMap::new();
        for node in std::iter::once(entity).chain(tree.all.iter().copied()) {
            if let Ok(name) = names.get(node) {
                node_entities.entry(name.as_str()).or_insert(node);
            }
            if let Ok(kids) = children.get(node) {
                for kid in kids.iter() {
                    parents.insert(kid, node);
                }
            }
        }
        let (constraints, unresolved) =
            constraint::resolve(&model.constraints, &node_entities, &parents, entity);
        if !unresolved.is_empty() {
            warn!(
                "[oc] {}: node constraints left out: {}",
                model.name,
                unresolved.join("; ")
            );
        }
        let [rotations, rolls, aims] = constraint::tally(&constraints);
        // The model's materials, one OC material per source material. A
        // character body keeps its own until it is dressed.
        let character = model.source == MapSource::Character;
        let vertex_colors = model.source == MapSource::NodeNames;
        let mut made: HashMap<AssetId<StandardMaterial>, Handle<OcMaterial>> = HashMap::new();
        let (mut meshes, mut skins) = (0usize, 0usize);
        for part in &tree.all {
            if skinned.get(*part).is_ok() {
                skins += 1;
                commands.entity(*part).insert(NoFrustumCulling);
            }
            let Ok(material) = standard.get(*part) else {
                continue;
            };
            meshes += 1;
            if character {
                continue;
            }
            let handle = made
                .entry(material.0.id())
                .or_insert_with(|| {
                    let oc = standard_materials.get(&material.0).map_or_else(
                        || OcMaterial::from_standard(&StandardMaterial::default(), vertex_colors),
                        |source| OcMaterial::from_standard(source, vertex_colors),
                    );
                    oc_materials.add(oc)
                })
                .clone();
            commands
                .entity(*part)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(handle), crate::markers::PlayerBodyRenderer));
        }
        if !character {
            if let Ok(mut visibility) = visibilities.get_mut(entity) {
                *visibility = Visibility::Inherited;
            }
        }
        let report = &plan.report;
        let what = format!(
            "{} ({}, scale {:.3}{})",
            model.name,
            model.source.label(),
            report.scale,
            if report.turned { ", turned to face +Z" } else { "" }
        );
        info!(
            "[oc] {what} bound: {} bones, {} driven, hips {:.3} -> {:.3}, {} segments aligned, {} finger joints curled, {meshes} mesh parts ({skins} skinned), {} materials, node constraints {rotations} rotation / {rolls} roll / {aims} aim",
            tree.bones.len(),
            plan.driven.len(),
            report.model_hips,
            report.avatar_hips,
            report.aligned,
            report.curled,
            made.len(),
        );
        if current {
            selection.status = OcStatus::Worn(what);
        }
        let joints = proxy.joints.len();
        commands.entity(entity).insert(OcBinding {
            driven: plan.driven.iter().map(|bone| tree.entities[bone.joint]).collect(),
            plan,
            proxy: proxy.joints.clone(),
            locals: vec![Frame::IDENTITY; joints],
            solution: rig::Solution::default(),
            constraints,
            written: Vec::new(),
            hidden: Vec::new(),
            warned: false,
            own: false,
        });
        // The offer of the body's own motion: its own animator with its
        // unit's idle and walk segments (see [`super::own`]). A body without
        // them keeps the avatar's motion, and its owner says so.
        let unit = model.character.as_ref().map(|character| character.unit);
        let families = unit
            .zip(profiles.as_ref())
            .and_then(|(unit, profiles)| profiles.get(unit))
            .map(|profile| (profile.idle.clone(), profile.walk.clone()));
        commands.entity(entity).insert(super::own::OwnOffer {
            unit,
            families,
        });
    }
}

/// Update: while an OC is worn, the avatar's body mesh (the mesh skinned to
/// the avatar's own joints) and its worn accessory stay hidden, including
/// parts mounted after the OC was bound. Held props and tools stay visible:
/// they follow the OC's hands. Until the OC is shown, the avatar body stays.
pub fn keep_avatar_hidden(
    mut bindings: Query<(Entity, &OcModel, &mut OcBinding)>,
    drivers: Query<&AvatarDriver>,
    children: Query<&Children>,
    oc_models: Query<(), With<OcModel>>,
    parts: Query<(Option<&SkinnedMesh>, Has<WearPart>), With<Mesh3d>>,
    mut visibilities: Query<&mut Visibility>,
) {
    for (oc, model, mut binding) in &mut bindings {
        let Ok(driver) = drivers.get(model.player) else {
            continue;
        };
        if visibilities.get(oc).is_ok_and(|visibility| *visibility == Visibility::Hidden) {
            continue;
        }
        let mut stack = vec![driver.visual_root];
        while let Some(entity) = stack.pop() {
            if oc_models.get(entity).is_ok() {
                continue;
            }
            let avatar_part = parts.get(entity).is_ok_and(|(skin, worn)| {
                worn || skin.is_some_and(|skin| {
                    skin.joints.iter().any(|joint| binding.proxy.contains(joint))
                })
            });
            if avatar_part {
                if let Ok(mut visibility) = visibilities.get_mut(entity) {
                    if !binding.hidden.iter().any(|(hidden, _)| *hidden == entity) {
                        binding.hidden.push((entity, *visibility));
                    }
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
    }
}
