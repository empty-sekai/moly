//! The tool in the player's hand (`PlayerAvatarHarvestView.ShowToolModel` /
//! `HideToolModel`).
//!
//! `ShowToolModel(tool, type)` acts only for a tool whose quantity is not
//! zero and a wood or mineral target: it hides every cached tool model, then
//! shows the cached one of that tool id or creates it
//! (`mysekai/tool/<assetbundleName>`) under `PlayerAvatarView.RightArm`
//! (`FindDeep(avatarRoot, "Penlight_R")`, the avatar body's right hand).
//! `HideToolModel` hides them all. The tool keeps its glTF materials (its
//! source shader is not ported).

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use moly_assets::json::JsonAsset;

use crate::player::PlayerControlled;
use crate::player_avatar::AvatarDriver;

const INDEX: &str = "moly://avatar/tools/index.json";
const MOUNT: &str = "Penlight_R";

/// A request from the harvest presenter to the avatar's tool view.
#[derive(Debug, Clone)]
pub(crate) enum ToolModelRequest {
    Show { tool_id: i64, assetbundle: String },
    Hide,
}

#[derive(Resource, Default)]
pub(crate) struct ToolModelRequests(pub(crate) Vec<ToolModelRequest>);

struct ToolFiles {
    glb: String,
    scene: usize,
}

#[derive(Resource, Default)]
pub(crate) struct HarvestToolModels {
    index_request: Option<Handle<JsonAsset>>,
    index: Option<HashMap<String, ToolFiles>>,
    absent: bool,
    gltfs: HashMap<i64, Handle<Gltf>>,
    instances: HashMap<i64, Entity>,
    /// The tool that should be visible once its model exists.
    wanted: Option<(i64, String)>,
    pending_show: bool,
}

pub(crate) fn load(mut models: ResMut<HarvestToolModels>, server: Res<AssetServer>) {
    models.index_request = Some(server.load(bevy::asset::AssetPath::from(INDEX.to_owned())));
}

pub(crate) fn parse(
    mut models: ResMut<HarvestToolModels>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
) {
    if models.index.is_some() || models.absent {
        return;
    }
    let Some(handle) = models.index_request.clone() else {
        return;
    };
    match server.load_state(&handle) {
        LoadState::Failed(error) => {
            warn!("[harvest] input absent, no tool model in the hand: {INDEX}: {error}");
            models.absent = true;
            return;
        }
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&handle) else {
        return;
    };
    let value: serde_json::Value =
        serde_json::from_str(&asset.0).unwrap_or_else(|error| panic!("{INDEX}: not JSON: {error}"));
    let tools = value["tools"]
        .as_object()
        .unwrap_or_else(|| panic!("{INDEX}: no tools object"));
    let mut index = HashMap::new();
    for (leaf, row) in tools {
        let glb = row["glb"]
            .as_str()
            .unwrap_or_else(|| panic!("{INDEX}: tool {leaf} without glb"));
        let scene = row["sourcePrefab"]["scene"]
            .as_u64()
            .unwrap_or_else(|| panic!("{INDEX}: tool {leaf} without a prefab scene"))
            as usize;
        index.insert(
            leaf.clone(),
            ToolFiles {
                glb: glb.to_owned(),
                scene,
            },
        );
    }
    info!("[harvest] tool model index: {} tools", index.len());
    models.index = Some(index);
}

/// Update: apply the show / hide requests; create a model under the hand
/// mount once its glb is loaded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut requests: ResMut<ToolModelRequests>,
    mut models: ResMut<HarvestToolModels>,
    players: Query<&AvatarDriver, With<PlayerControlled>>,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    mut visibility: Query<&mut Visibility>,
) {
    for request in std::mem::take(&mut requests.0) {
        match request {
            ToolModelRequest::Hide => {
                for entity in models.instances.values() {
                    if let Ok(mut visibility) = visibility.get_mut(*entity) {
                        *visibility = Visibility::Hidden;
                    }
                }
                models.wanted = None;
                models.pending_show = false;
            }
            ToolModelRequest::Show {
                tool_id,
                assetbundle,
            } => {
                for entity in models.instances.values() {
                    if let Ok(mut visibility) = visibility.get_mut(*entity) {
                        *visibility = Visibility::Hidden;
                    }
                }
                models.wanted = Some((tool_id, assetbundle));
                models.pending_show = true;
            }
        }
    }
    if !models.pending_show {
        return;
    }
    let Some((tool_id, assetbundle)) = models.wanted.clone() else {
        return;
    };
    if let Some(entity) = models.instances.get(&tool_id).copied() {
        if let Ok(mut visibility) = visibility.get_mut(entity) {
            *visibility = Visibility::Inherited;
        }
        models.pending_show = false;
        return;
    }
    if models.absent {
        models.pending_show = false;
        return;
    }
    let Some(index) = models.index.as_ref() else {
        return;
    };
    let Some(files) = index.get(&assetbundle) else {
        warn!(
            "[harvest] tool {tool_id}: no model {assetbundle} in the tool index; the hand stays empty"
        );
        models.pending_show = false;
        return;
    };
    let (glb, scene) = (files.glb.clone(), files.scene);
    let handle = models
        .gltfs
        .entry(tool_id)
        .or_insert_with(|| {
            server.load(bevy::asset::AssetPath::from(format!(
                "moly://avatar/tools/{glb}"
            )))
        })
        .clone();
    if let LoadState::Failed(error) = server.load_state(&handle) {
        panic!("tool model {assetbundle} failed to load: {error:?}");
    }
    let Some(gltf) = gltfs.get(&handle) else {
        return;
    };
    let Ok(driver) = players.single() else {
        return;
    };
    let mount = names.iter().find_map(|(entity, name)| {
        if name.as_str() != MOUNT {
            return None;
        }
        let mut cursor = entity;
        while let Ok(parent) = parents.get(cursor) {
            cursor = parent.parent();
            if cursor == driver.visual_root || cursor == driver.player {
                return Some(entity);
            }
        }
        None
    });
    let Some(mount) = mount else {
        return;
    };
    let scene_handle =
        gltf.scenes.get(scene).cloned().unwrap_or_else(|| {
            panic!("tool model {assetbundle}: prefab scene {scene} out of range")
        });
    let entity = commands
        .spawn((
            SceneRoot(scene_handle),
            Transform::IDENTITY,
            Visibility::Inherited,
            ChildOf(mount),
        ))
        .id();
    models.instances.insert(tool_id, entity);
    models.pending_show = false;
    info!(
        "[harvest] tool model {assetbundle} (tool {tool_id}) created under {MOUNT} (PlayerAvatarView.RightArm)"
    );
}

/// Queued by the site change: the models stay cached on the body, hidden.
pub(crate) fn hide_all(world: &mut World) {
    world
        .resource_mut::<ToolModelRequests>()
        .0
        .push(ToolModelRequest::Hide);
}
