//! The mesh material the glTF loader derives from each glTF material.
//!
//! A loaded glTF keeps its materials as `GltfMaterial` assets, and the mesh
//! entities it spawns carry a `StandardMaterial` built from each one. The
//! loader stores that material next to its source, under the same label with
//! a `/std` suffix, so the pair is joined by asset path.

use bevy::asset::{AssetPath, Handle};
use bevy::gltf::GltfMaterial;

/// The asset path of the mesh material derived from `material`. `None` when
/// the handle has no labelled path (a material not loaded from a glTF file).
pub fn derived_mesh_material_path(material: &Handle<GltfMaterial>) -> Option<AssetPath<'static>> {
    let path = material.path()?;
    let label = path.label()?;
    Some(path.clone().with_label(format!("{label}/std")))
}

/// Whether `mesh_material` (a mesh entity's material handle) is the one the
/// loader derived from `material`.
pub fn derives<A: bevy::asset::Asset>(material: &Handle<GltfMaterial>, mesh_material: &Handle<A>) -> bool {
    mesh_material
        .path()
        .is_some_and(|path| derived_mesh_material_path(material).as_ref() == Some(path))
}
