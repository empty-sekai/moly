//! Read-only byte accounting for live assets, not allocator/OS memory claims.
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::TextureDimension,
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
    for (id, image) in images.iter() {
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
        if cpu > 0 && image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) {
            both_world_images += 1;
        }
        rows.push((
            capacity + gpu.unwrap_or(0),
            json!({
                "path": server.get_path(id).map(|path| path.to_string()),
                "cpuPixelBytes": cpu, "cpuPixelCapacityBytes": capacity,
                "estimatedGpuTextureBytes": gpu,
                "width": image.width(), "height": image.height(),
                "mipLevels": image.texture_descriptor.mip_level_count,
            }),
        ));
    }
    rows.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    json!({
        "scope": "live-asset-payloads; excludes allocator overhead, clips, ECS, drivers and internal render targets",
        "cpuImageBytes": image_bytes, "cpuImageCapacityBytes": image_capacity,
        "cpuMeshBufferBytes": meshes.iter().map(|(_, mesh)| mesh_cpu_bytes(mesh)).sum::<u64>(),
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
