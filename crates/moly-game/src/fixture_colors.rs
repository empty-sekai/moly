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
use std::collections::{HashMap, HashSet};

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
    // Furniture recolour textures are sampled by the render world after
    // import; no gameplay or editor path reads their decoded texel buffer
    // again, so they load render-world-only and Bevy releases the source
    // payload once the GPU upload has completed.
    let handle = moly_assets::residency::load_image_with(
        world.resource::<AssetServer>(),
        format!("moly://{path}"),
        move |settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::Descriptor(sampler.clone());
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
    let clear_requested = world
        .get_resource::<crate::game_settings::SettingsPanel>()
        .is_some_and(|panel| panel.resource_clear_requested);
    if clear_requested {
        // Keep the request edge-triggered even if this frame is waiting for a
        // colour asset.  The result is a cache-handle count, not an RSS claim.
        let (materials, images) = clear_inactive_cache_with(&mut colors, world);
        if let Some(mut panel) = world.get_resource_mut::<crate::game_settings::SettingsPanel>() {
            panel.resource_clear_requested = false;
            panel.resource_status = format!(
                "已释放未使用缓存句柄：材质 {}，纹理 {}（GPU 回收由引擎异步完成）",
                materials, images
            );
        }
    }
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

fn clear_inactive_cache_with(colors: &mut FixtureColors, world: &mut World) -> (usize, usize) {
    let active_materials: HashSet<AssetId<StandardMaterial>> = world
        .query::<&MeshMaterial3d<StandardMaterial>>()
        .iter(world)
        .map(|material| material.0.id())
        .collect();
    let active_images: HashSet<AssetId<Image>> = world
        .query::<&moly_assets::material_textures::SourceMaterialTextures>()
        .iter(world)
        .flat_map(|textures| textures.0.values().map(Handle::id))
        .collect();
    let mut active_images = active_images;
    // The cache key records exactly which recolour textures produced a live
    // material. Keep those IDs even when a source material has no imported
    // texture metadata component (e.g. a minimal test GLB).
    for ((_, main, emission), material) in &colors.materials {
        if active_materials.contains(&material.id()) {
            active_images.insert(*main);
            if let Some(emission) = emission {
                active_images.insert(*emission);
            }
        }
    }
    let old_materials = colors.materials.len();
    colors
        .materials
        .retain(|_, handle| active_materials.contains(&handle.id()));
    let old_images = colors.images.len();
    colors
        .images
        .retain(|_, handle| active_images.contains(&handle.id()));
    (
        old_materials.saturating_sub(colors.materials.len()),
        old_images.saturating_sub(colors.images.len()),
    )
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<FixtureColors>()
        .add_systems(Update, prepare.after(crate::fixture::FixtureLayoutSet));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recolor_images_are_render_world_only() {
        assert_eq!(moly_assets::residency::GPU_ONLY, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    }

    #[test]
    fn explicit_clear_keeps_live_recolor_and_releases_retired_handles() {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        let live_image = world.resource_mut::<Assets<Image>>().add(Image::default());
        let old_image = world.resource_mut::<Assets<Image>>().add(Image::default());
        let live_material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let old_material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let entity = world.spawn(MeshMaterial3d(live_material.clone())).id();
        let mut colors = FixtureColors::default();
        colors.images.insert("live".into(), live_image.clone());
        colors.images.insert("old".into(), old_image.clone());
        colors.materials.insert(
            (live_material.id(), live_image.id(), None),
            live_material.clone(),
        );
        colors.materials.insert(
            (old_material.id(), old_image.id(), None),
            old_material,
        );

        assert_eq!(clear_inactive_cache_with(&mut colors, &mut world), (1, 1));
        assert!(colors.images.contains_key("live"));
        assert_eq!(colors.materials.len(), 1);
        world.entity_mut(entity).despawn();
        assert_eq!(clear_inactive_cache_with(&mut colors, &mut world), (1, 1));
        assert!(colors.images.is_empty());
    }
}
