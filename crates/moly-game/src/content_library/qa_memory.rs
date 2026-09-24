//! Read-only byte accounting for live assets, not allocator/OS memory claims.
use std::collections::BTreeMap;

use bevy::{
    asset::{AssetPath, RenderAssetUsages},
    mesh::Indices,
    prelude::*,
    render::render_resource::TextureDimension,
};
use serde_json::{Value, json};

fn texture_bytes(image: &Image) -> Option<u64> {
    let descriptor = &image.texture_descriptor;
    let block = u64::from(descriptor.format.block_copy_size(None)?);
    let (block_width, block_height) = descriptor.format.block_dimensions();
    let mut bytes = 0;
    for level in 0..descriptor.mip_level_count {
        let shrink = |size: u32| size.checked_shr(level).unwrap_or(0).max(1);
        let width = shrink(descriptor.size.width).div_ceil(block_width);
        let height = shrink(descriptor.size.height).div_ceil(block_height);
        let depth = if descriptor.dimension == TextureDimension::D3 {
            shrink(descriptor.size.depth_or_array_layers)
        } else {
            descriptor.size.depth_or_array_layers
        };
        bytes += u64::from(width)
            * u64::from(height)
            * u64::from(depth)
            * block
            * u64::from(descriptor.sample_count);
    }
    Some(bytes)
}

fn mesh_cpu_bytes(mesh: &Mesh) -> u64 {
    let vertices = mesh.try_attributes().map_or(0, |attributes| {
        attributes
            .map(|(_, values)| values.get_bytes().len() as u64)
            .sum::<u64>()
    });
    let indices = mesh.try_indices().map_or(0, |indices| match indices {
        Indices::U16(values) => (values.len() * 2) as u64,
        Indices::U32(values) => (values.len() * 4) as u64,
    });
    vertices + indices
}

#[cfg(target_arch = "wasm32")]
fn wasm_capacity() -> Option<u64> {
    use wasm_bindgen::JsCast;
    let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().unchecked_into();
    let buffer: js_sys::ArrayBuffer = memory.buffer().unchecked_into();
    Some(u64::from(buffer.byte_length()))
}

#[cfg(not(target_arch = "wasm32"))]
fn wasm_capacity() -> Option<u64> {
    None
}

/// The family an asset belongs to, derived from its asset path alone: the
/// first directory of the path (the first two under `site/`, whose
/// subdirectories are loaded by different code), then the file extension, and
/// `#` when the asset is a labelled part of a package (a glTF texture or
/// mesh). Files at the root of the asset source are `(root)`; assets created at
/// runtime have no path and are `(runtime)`.
fn family(path: Option<&AssetPath>) -> String {
    let Some(path) = path else {
        return "(runtime)".to_owned();
    };
    let segments: Vec<&str> = path.path().iter().filter_map(|segment| segment.to_str()).collect();
    let directory = match segments.as_slice() {
        [] | [_] => "(root)".to_owned(),
        ["site", second, _, ..] => format!("site/{second}"),
        [first, ..] => (*first).to_owned(),
    };
    let extension = path.path().extension().and_then(|extension| extension.to_str()).unwrap_or("");
    let label = if path.label().is_some() { "#" } else { "" };
    format!("{directory}/*.{extension}{label}")
}

#[derive(Default)]
struct ImageFamily {
    images: u64,
    cpu_images: u64,
    cpu_bytes: u64,
    cpu_capacity: u64,
    gpu_bytes: u64,
    unknown_gpu: u64,
    cpu_and_gpu: u64,
    largest_cpu: Option<(u64, String)>,
}

#[derive(Default)]
struct MeshFamily {
    meshes: u64,
    cpu_meshes: u64,
    cpu_bytes: u64,
}

pub(super) fn diagnostics(
    images: &Assets<Image>,
    meshes: &Assets<Mesh>,
    server: &AssetServer,
) -> Value {
    let mut image_bytes = 0u64;
    let mut image_capacity = 0u64;
    let mut gpu_bytes = 0u64;
    let mut unknown_gpu_images = 0;
    let mut both_world_images = 0;
    let mut rows = Vec::new();
    let mut image_families: BTreeMap<String, ImageFamily> = BTreeMap::new();
    for (id, image) in images.iter() {
        let path = server.get_path(id);
        let cpu = image.data.as_ref().map_or(0, |data| data.len() as u64);
        let capacity = image.data.as_ref().map_or(0, |data| data.capacity() as u64);
        image_bytes += cpu;
        image_capacity += capacity;
        let gpu = if image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) {
            let bytes = texture_bytes(image);
            if bytes.is_none() {
                unknown_gpu_images += 1;
            }
            bytes
        } else {
            Some(0)
        };
        gpu_bytes += gpu.unwrap_or(0);
        let both = cpu > 0 && image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD);
        if both {
            both_world_images += 1;
        }
        let entry = image_families.entry(family(path.as_ref())).or_default();
        entry.images += 1;
        entry.cpu_bytes += cpu;
        entry.cpu_capacity += capacity;
        entry.gpu_bytes += gpu.unwrap_or(0);
        entry.unknown_gpu += u64::from(gpu.is_none());
        entry.cpu_and_gpu += u64::from(both);
        if capacity > 0 {
            entry.cpu_images += 1;
            if entry.largest_cpu.as_ref().is_none_or(|(largest, _)| capacity > *largest) {
                let name = path.as_ref().map_or_else(|| format!("{id:?}"), |path| path.to_string());
                entry.largest_cpu = Some((capacity, name));
            }
        }
        rows.push((
            capacity + gpu.unwrap_or(0),
            json!({
                "path": path.map(|path| path.to_string()),
                "cpuPixelBytes": cpu, "cpuPixelCapacityBytes": capacity,
                "estimatedGpuTextureBytes": gpu,
                "width": image.width(), "height": image.height(),
                "mipLevels": image.texture_descriptor.mip_level_count,
            }),
        ));
    }
    rows.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut mesh_families: BTreeMap<String, MeshFamily> = BTreeMap::new();
    let mut mesh_bytes = 0u64;
    for (id, mesh) in meshes.iter() {
        let bytes = mesh_cpu_bytes(mesh);
        mesh_bytes += bytes;
        let entry = mesh_families.entry(family(server.get_path(id).as_ref())).or_default();
        entry.meshes += 1;
        entry.cpu_meshes += u64::from(bytes > 0);
        entry.cpu_bytes += bytes;
    }
    let mut image_families: Vec<(String, ImageFamily)> = image_families.into_iter().collect();
    image_families.sort_by(|(a_name, a), (b_name, b)| {
        (b.cpu_capacity + b.gpu_bytes)
            .cmp(&(a.cpu_capacity + a.gpu_bytes))
            .then_with(|| a_name.cmp(b_name))
    });
    let mut mesh_families: Vec<(String, MeshFamily)> = mesh_families.into_iter().collect();
    mesh_families.sort_by(|(a_name, a), (b_name, b)| b.cpu_bytes.cmp(&a.cpu_bytes).then_with(|| a_name.cmp(b_name)));
    json!({
        "scope": "live-asset-payloads; excludes allocator overhead, clips, ECS, drivers and internal render targets",
        "cpuImageBytes": image_bytes, "cpuImageCapacityBytes": image_capacity,
        "cpuMeshBufferBytes": mesh_bytes,
        "imageFamilies": image_families.into_iter().map(|(name, family)| json!({
            "family": name, "images": family.images, "cpuImages": family.cpu_images,
            "cpuPixelBytes": family.cpu_bytes, "cpuPixelCapacityBytes": family.cpu_capacity,
            "estimatedGpuTextureBytes": family.gpu_bytes, "unknownGpuImageCount": family.unknown_gpu,
            "cpuAndGpuImageCount": family.cpu_and_gpu,
            "largestCpuImage": family.largest_cpu.map(|(_, path)| path),
        })).collect::<Vec<_>>(),
        "meshFamilies": mesh_families.into_iter().map(|(name, family)| json!({
            "family": name, "meshes": family.meshes, "cpuMeshes": family.cpu_meshes,
            "cpuMeshBufferBytes": family.cpu_bytes,
        })).collect::<Vec<_>>(),
        "estimatedGpuTextureBytes": gpu_bytes, "unknownGpuImageCount": unknown_gpu_images,
        "cpuAndGpuImageCount": both_world_images,
        "wasmLinearMemoryCapacityBytes": wasm_capacity(),
        "wasmCapacityIsNotLiveAllocationBytes": true,
        "largestImages": rows.into_iter().take(8).map(|(_, row)| row).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureFormat};

    #[test]
    fn texture_estimate_counts_mips_and_array_layers() {
        let mut image = Image::new_uninit(
            Extent3d {
                width: 4,
                height: 2,
                depth_or_array_layers: 3,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::all(),
        );
        image.texture_descriptor.mip_level_count = 3;
        assert_eq!(texture_bytes(&image), Some((8 + 2 + 1) * 3 * 4));
        image.texture_descriptor.dimension = TextureDimension::D3;
        assert_eq!(texture_bytes(&image), Some((8 * 3 + 2 + 1) * 4));
    }

    #[test]
    fn mesh_accounting_counts_unique_asset_buffers_not_scene_instances() {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
        mesh.insert_indices(Indices::U16(vec![0, 1, 2]));
        assert_eq!(mesh_cpu_bytes(&mesh), 42);
    }

    #[test]
    fn compressed_texture_estimate_rounds_to_block_boundaries() {
        let mut image = Image::new_uninit(
            Extent3d {
                width: 5,
                height: 5,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Bc1RgbaUnorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.mip_level_count = 3;
        assert_eq!(texture_bytes(&image), Some((4 + 1 + 1) * 8));
    }
}
