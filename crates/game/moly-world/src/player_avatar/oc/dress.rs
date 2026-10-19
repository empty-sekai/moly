//! A character body's material: the cast's character material, planned from
//! the body's rig profile as for a cast member
//! ([`crate::character_material::plan_body`]) and swapped in by the cast's
//! swap once its textures are in. The body is shown once dressed.

use super::bind::{OcBinding, OcModel};
use super::source::OcSelection;
use crate::character_material::{self, CharacterEnv, PlanTextures, PlayerWorn, ToonMaterials, ToonPlan};
use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use moly_assets::json::JsonAsset;

/// Update: a bound character body gets its toon plan once its rig profile
/// and the fog profile are in. A body the plan refuses is removed (by
/// [`super::bind::attach`], once it is no longer chosen) and the avatar body
/// stays.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn dress(
    mut commands: Commands,
    mut selection: ResMut<OcSelection>,
    server: Res<AssetServer>,
    render_device: Res<RenderDevice>,
    env: Option<Res<CharacterEnv>>,
    gltfs: Res<Assets<Gltf>>,
    jsons: Res<Assets<JsonAsset>>,
    models: Query<(Entity, &OcModel), (With<OcBinding>, Without<ToonPlan>, Without<ToonMaterials>)>,
    children: Query<&Children>,
    names: Query<&Name>,
    materials: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    if !env.is_some_and(|env| env.fog_ready) {
        return;
    }
    for (entity, model) in &models {
        let Some(body) = &model.character else {
            continue;
        };
        let planned = match server.load_state(&body.rig) {
            LoadState::Failed(error) => Err(format!("rig profile {}: {error}", body.rig_file)),
            LoadState::Loaded => {
                let (Some(json), Some(gltf)) = (jsons.get(&body.rig), gltfs.get(&model.gltf)) else {
                    continue;
                };
                character_material::plan_body(
                    &mut commands,
                    &server,
                    &render_device,
                    gltf,
                    &json.0,
                    &body.rig_file,
                    entity,
                    &children,
                    &names,
                    &materials,
                )
            }
            _ => continue,
        };
        match planned {
            Ok(plan) => {
                commands.entity(entity).insert((plan, PlayerWorn));
                info!("[oc] {} (unit {}): character material planned", model.name, body.unit);
            }
            Err(reason) => {
                let current = selection
                    .model
                    .as_ref()
                    .is_some_and(|load| load.generation == model.generation);
                if current {
                    selection.model = None;
                    selection.refuse(&model.name, &reason);
                }
            }
        }
    }
}

/// Update: a planned character body whose textures fail to load is refused
/// like a refused plan; the avatar body stays.
pub fn refuse_failed_textures(
    mut selection: ResMut<OcSelection>,
    server: Res<AssetServer>,
    models: Query<(&OcModel, &ToonPlan), (With<OcBinding>, With<PlayerWorn>, Without<ToonMaterials>)>,
) {
    for (model, plan) in &models {
        let PlanTextures::Failed(error) = character_material::plan_textures(plan, &server) else {
            continue;
        };
        let current = selection
            .model
            .as_ref()
            .is_some_and(|load| load.generation == model.generation);
        if current {
            selection.model = None;
            selection.refuse(&model.name, &format!("character texture: {error}"));
        }
    }
}

/// Update: a dressed character body is shown.
pub fn show(mut models: Query<(&OcModel, &mut Visibility), (With<OcBinding>, With<ToonMaterials>)>) {
    for (model, mut visibility) in &mut models {
        if *visibility == Visibility::Hidden {
            *visibility = Visibility::Inherited;
            info!("[oc] {} dressed and shown", model.name);
        }
    }
}
