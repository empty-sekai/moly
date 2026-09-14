//! Per-instance source color selection before furniture material conversion.

use bevy::{
    asset::LoadState,
    image::{
        ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    prelude::*,
};
use moly_assets::{json::JsonAsset, material_textures::SourceMaterialTextures};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Component, Clone)]
pub(crate) struct FixtureColorChoice {
    pub package: String,
    pub texture_id: u32,
}

#[derive(Component)]
struct ColorApplied;

type MaterialKey = (
    AssetId<StandardMaterial>,
    AssetId<Image>,
    Option<AssetId<Image>>,
);

#[derive(Resource)]
pub(crate) struct FixtureColors {
    ready: bool,
    revision: u64,
    request: Option<Handle<JsonAsset>>,
    palettes: Option<Value>,
    images: HashMap<String, Handle<Image>>,
    materials: HashMap<MaterialKey, Handle<StandardMaterial>>,
    error: Option<String>,
}

impl Default for FixtureColors {
    fn default() -> Self {
        Self {
            ready: true,
            revision: 0,
            request: None,
            palettes: None,
            images: HashMap::new(),
            materials: HashMap::new(),
            error: None,
        }
    }
}

pub(crate) fn ready(colors: Res<FixtureColors>) -> bool {
    colors.ready
}

fn sampler(value: &Value) -> ImageSamplerDescriptor {
    let address = |key: &str| match value[key].as_u64() {
        Some(33071) => ImageAddressMode::ClampToEdge,
        Some(33648) => ImageAddressMode::MirrorRepeat,
        _ => ImageAddressMode::Repeat,
    };
    let min = value["minFilter"].as_u64().unwrap_or(9729);
    ImageSamplerDescriptor {
        address_mode_u: address("wrapS"),
        address_mode_v: address("wrapT"),
        mag_filter: if value["magFilter"].as_u64() == Some(9728) {
            ImageFilterMode::Nearest
        } else {
            ImageFilterMode::Linear
        },
        min_filter: if matches!(min, 9728 | 9984 | 9986) {
            ImageFilterMode::Nearest
        } else {
            ImageFilterMode::Linear
        },
        mipmap_filter: if matches!(min, 9986 | 9987) {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        },
        ..default()
    }
}

fn image(
    world: &World,
    colors: &mut FixtureColors,
    value: &Value,
) -> Result<Option<Handle<Image>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let path = value["path"]
        .as_str()
        .ok_or("Color texture path is missing")?;
    if !path.starts_with("fixture-models/colors/")
        || !path.ends_with(".png")
        || path.contains("..")
        || path.contains('\\')
        || path.contains(':')
    {
        return Err("Color texture path is outside the furniture catalog".into());
    }
    if let Some(handle) = colors.images.get(path) {
        return Ok(Some(handle.clone()));
    }
    let sampler = sampler(&value["sampler"]);
    let handle = world.resource::<AssetServer>().load_with_settings(
        format!("moly://{path}"),
        move |settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::Descriptor(sampler.clone());
            settings.asset_usage = bevy::asset::RenderAssetUsages::all();
        },
    );
    colors.images.insert(path.to_owned(), handle.clone());
    Ok(Some(handle))
}

fn apply(world: &mut World, colors: &mut FixtureColors) -> Result<(), String> {
    let revision = world.resource::<crate::fixture::FixtureLayoutRevision>().0;
    if revision != colors.revision {
        colors.revision = revision;
        colors.materials.clear();
        colors.images.clear();
        colors.error = None;
    }
    let pending: Vec<_> = world
        .query_filtered::<(
            Entity,
            &FixtureColorChoice,
            Option<&crate::fixture::FixtureVisualSceneReady>,
        ), Without<ColorApplied>>()
        .iter(world)
        .map(|(entity, choice, ready)| (entity, choice.clone(), ready.is_some()))
        .collect();
    colors.ready = pending.is_empty();
    if pending.is_empty() {
        return Ok(());
    }
    if pending.iter().any(|(_, choice, _)| choice.texture_id != 1) && colors.palettes.is_none() {
        let handle = colors
            .request
            .get_or_insert_with(|| {
                world
                    .resource::<AssetServer>()
                    .load("moly://fixture-models/player-data.json")
            })
            .clone();
        if matches!(
            world.resource::<AssetServer>().load_state(&handle),
            LoadState::Failed(_)
        ) {
            return Err("Furniture color catalog could not be loaded".into());
        }
        let Some(asset) = world.resource::<Assets<JsonAsset>>().get(&handle) else {
            return Ok(());
        };
        let document: Value =
            serde_json::from_str(&asset.0).map_err(|e| format!("Furniture color catalog: {e}"))?;
        if !document["colorTextures"].is_object() {
            return Err("Furniture color textures have not been exported".into());
        }
        colors.palettes = Some(document["colorTextures"].clone());
    }
    let mut remaining = 0;
    for (root, choice, scene_ready) in pending {
        if choice.texture_id == 1 {
            world.entity_mut(root).insert(ColorApplied);
            continue;
        }
        if !scene_ready {
            remaining += 1;
            continue;
        }
        let palette = colors
            .palettes
            .as_ref()
            .and_then(|p| p.get(&choice.package))
            .and_then(|p| p.get(choice.texture_id.to_string()))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Furniture color {} is not in the exported catalog",
                    choice.texture_id
                )
            })?;
        let Some(main) = image(world, colors, &palette["main"])? else {
            world.entity_mut(root).insert(ColorApplied);
            continue;
        };
        let emission = image(world, colors, &palette["emission"])?;
        let mut loaded = true;
        for handle in std::iter::once(&main).chain(emission.iter()) {
            match world.resource::<AssetServer>().load_state(handle) {
                LoadState::Failed(_) => return Err(
                    "A furniture color texture could not be loaded; the import backup is available"
                        .into(),
                ),
                LoadState::Loaded => {}
                _ => loaded = false,
            }
        }
        if !loaded {
            remaining += 1;
            continue;
        }
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Some(children) = world.get::<Children>(entity) {
                stack.extend(children.iter());
            }
            let Some(source) = world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .map(|v| v.0.clone())
            else {
                continue;
            };
            let mut textures = world
                .get::<SourceMaterialTextures>(entity)
                .cloned()
                .unwrap_or_default();
            let Some(original) = world
                .resource::<Assets<StandardMaterial>>()
                .get(&source)
                .cloned()
            else {
                continue;
            };
            if original.base_color_texture.is_none() && !textures.0.contains_key("_MainTex") {
                continue;
            }
            let key = (source.id(), main.id(), emission.as_ref().map(Handle::id));
            let handle = if let Some(handle) = colors.materials.get(&key) {
                handle.clone()
            } else {
                let mut replacement = original;
                replacement.base_color_texture = Some(main.clone());
                let handle = world
                    .resource_mut::<Assets<StandardMaterial>>()
                    .add(replacement);
                colors.materials.insert(key, handle.clone());
                handle
            };
            textures.0.insert("_MainTex".into(), main.clone());
            if let Some(emission) = &emission {
                textures
                    .0
                    .insert("_EmissionMaskTex".into(), emission.clone());
            }
            world
                .entity_mut(entity)
                .insert((MeshMaterial3d(handle), textures));
        }
        world.entity_mut(root).insert(ColorApplied);
    }
    colors.ready = remaining == 0;
    Ok(())
}

pub(crate) fn prepare(world: &mut World) {
    let Some(mut colors) = world.remove_resource::<FixtureColors>() else {
        return;
    };
    if let Err(error) = apply(world, &mut colors) {
        colors.ready = false;
        if colors.error.as_ref() != Some(&error) {
            warn!("[player-data] {error}");
            if let Some(mut state) =
                world.get_resource_mut::<crate::player_data::PlayerDataImport>()
            {
                state.status = error.clone();
            }
            if let Some(mut panel) = world.get_resource_mut::<crate::game_settings::SettingsPanel>()
            {
                panel.open = true;
                panel.player_data = true;
            }
            colors.error = Some(error);
        }
    }
    world.insert_resource(colors);
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<FixtureColors>()
        .add_systems(Update, prepare.after(crate::fixture::FixtureLayoutSet));
}
