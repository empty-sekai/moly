//! Fixture parts drawn with the character program.
//!
//! A fixture can carry a mesh whose material uses the character program (a
//! footbath's soaking legs). Its UVs and textures follow the character
//! program's conventions, so the fixture material pass leaves it alone and
//! this pass gives it a [`CharacterMaterial`] built from the part's own
//! material table and textures, once the character fog profile is in (as
//! every character material is built). A part that cannot be built is
//! hidden and named in the log: it never keeps the import's placeholder
//! material.

use std::collections::BTreeMap;

use bevy::gltf::{GltfMaterialExtras, GltfMaterialName};
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use moly_assets::material_textures::SourceMaterialTextures;
use moly_assets::sidecar::parse_fixture_material;
use moly_law::shading::character::SHADER_NAME;

use crate::character_material::{fixture_part, CharacterEnv, CharacterMaterial};
use crate::render_inputs::FixtureVisualRoot;

/// Imported meshes whose material names the character program, until each is
/// found to sit under a fixture view (and is built) or not.
#[derive(Default)]
pub struct Candidates(Vec<Entity>);

/// Whether the material's program is the character program.
pub fn uses_character_program(extras: &serde_json::Value) -> bool {
    extras.get("shader").and_then(serde_json::Value::as_str) == Some(SHADER_NAME)
}

/// The imported parts still on the import's material.
type ImportedParts<'w, 's> = Query<
    'w,
    's,
    (
        &'static GltfMaterialName,
        &'static GltfMaterialExtras,
        Option<&'static SourceMaterialTextures>,
    ),
    With<MeshMaterial3d<StandardMaterial>>,
>;

pub fn swap(
    mut commands: Commands,
    env: Res<CharacterEnv>,
    render_device: Res<RenderDevice>,
    added: Query<(Entity, &GltfMaterialExtras), Added<GltfMaterialExtras>>,
    parts: ImportedParts,
    parents: Query<&ChildOf>,
    roots: Query<(), With<FixtureVisualRoot>>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut candidates: Local<Candidates>,
) {
    for (entity, extras) in &added {
        if extras.value.contains(SHADER_NAME) {
            candidates.0.push(entity);
        }
    }
    if candidates.0.is_empty() || !env.fog_ready {
        return;
    }
    let under_fixture = |entity: Entity| {
        parents
            .iter_ancestors(entity)
            .any(|ancestor| roots.contains(ancestor))
    };
    let empty = BTreeMap::new();
    for entity in std::mem::take(&mut candidates.0) {
        let Ok((name, extras, textures)) = parts.get(entity) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        if !uses_character_program(&value) || !under_fixture(entity) {
            continue;
        }
        let built = parse_fixture_material(&name.0, &value)
            .map_err(|err| err.to_string())
            .and_then(|slot| {
                fixture_part(
                    &slot,
                    textures.map_or(&empty, |textures| &textures.0),
                    &render_device,
                    true,
                )
            });
        match built {
            Ok(material) => {
                commands
                    .entity(entity)
                    .remove::<MeshMaterial3d<StandardMaterial>>()
                    .insert(MeshMaterial3d(materials.add(material)));
                info!("[fixture-character-part] {} drawn with the character program", name.0);
            }
            Err(reason) => {
                commands.entity(entity).insert(Visibility::Hidden);
                error!("[fixture-character-part] {} refused and hidden: {reason}", name.0);
            }
        }
    }
}
