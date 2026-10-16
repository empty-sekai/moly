//! Retain source shader textures that glTF's StandardMaterial cannot reference.
//!
//! Without these strong handles, an extras-only emission mask can be unloaded
//! after import. Loading its TextureN label later reloads the entire GLB, which
//! respawns its scenes and discards installed materials and animation bindings.

use crate::material_passes::SourceMaterialPasses;
use std::collections::BTreeMap;

use bevy::{
    asset::LoadContext,
    gltf::extensions::{GltfExtensionHandler, GltfExtensionHandlers},
    prelude::*,
};

#[derive(Component, Reflect, Clone, Default)]
#[reflect(Component)]
pub struct SourceMaterialTextures(pub BTreeMap<String, Handle<Image>>);

/// Source submesh index, retained independently of material names/asset IDs.
#[derive(Component, Reflect, Clone, Default)]
#[reflect(Component)]
pub struct SourcePrimitiveIndex(pub usize);

#[derive(Default)]
struct SourceTextureLoader {
    textures: BTreeMap<usize, Handle<Image>>,
}

impl GltfExtensionHandler for SourceTextureLoader {
    fn dyn_clone(&self) -> Box<dyn GltfExtensionHandler> {
        // Every GLB owns its own texture indices and handle lifetime.
        Box::new(Self::default())
    }

    fn on_texture(&mut self, texture: &gltf::Texture, image: Handle<Image>) {
        self.textures.insert(texture.index(), image);
    }

    fn on_spawn_mesh_and_material(
        &mut self,
        _context: &mut LoadContext<'_>,
        primitive: &gltf::Primitive,
        _mesh: &gltf::Mesh,
        material: &gltf::Material,
        entity: &mut EntityWorldMut,
    ) {
        entity.insert(SourcePrimitiveIndex(primitive.index()));
        let Some(extras) = material.extras().as_ref() else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(extras.get()) else {
            return;
        };
        if let Some(passes) = SourceMaterialPasses::from_extras(&value) {
            entity.insert(passes);
        }
        let Some(textures) = value.get("textures").and_then(|v| v.as_object()) else {
            return;
        };
        let retained = textures
            .iter()
            .filter_map(|(name, index)| {
                let index = usize::try_from(index.as_u64()?).ok()?;
                Some((name.clone(), self.textures.get(&index)?.clone()))
            })
            .collect();
        entity.insert(SourceMaterialTextures(retained));
    }
}

/// Register after DefaultPlugins, before any GLB is requested.
pub fn register(app: &mut App) {
    crate::scene_state::register_types(app);
    app.register_type::<SourceMaterialPasses>();
    app.register_type::<SourceMaterialTextures>();
    app.register_type::<SourcePrimitiveIndex>();
    app.world()
        .resource::<GltfExtensionHandlers>()
        .0
        .try_write()
        .expect("glTF handlers are registered before asset loading")
        .extend([
            Box::<SourceTextureLoader>::default() as Box<dyn GltfExtensionHandler>,
            Box::<crate::scene_state::SceneStateLoader>::default(),
        ]);
}
