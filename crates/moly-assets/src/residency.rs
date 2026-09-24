//! Where decoded asset payloads live once the GPU has them.
//!
//! Bevy keeps a decoded image in the main world for as long as its handle
//! lives, and clones it in full the first time it reaches the render world.
//! An image marked render-world-only is moved there instead: the main world
//! keeps only its descriptor (size, format, sampler), which is all that
//! layout, readiness and sizing code reads. Every family loaded through this
//! module is sampled only by the GPU after upload, except the one that says
//! otherwise ([`GltfResidency::CpuTextures`]).
//!
//! Meshes keep Bevy's default residency everywhere: walk faces, navigation,
//! picking, character bounds and particles read their vertices on the CPU.
//!
//! Handles are keyed by path and asset type, and the first request's loader
//! settings win. Every requester of one path therefore has to use the same
//! entry point here, or the residency depends on which request came first.

use bevy::asset::{AssetPath, AssetServer, Handle, RenderAssetUsages};
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::image::{Image, ImageLoaderSettings};

/// Usage of a payload that only the GPU reads after its upload.
pub const GPU_ONLY: RenderAssetUsages = RenderAssetUsages::RENDER_WORLD;

/// How a glTF package's material textures are held after loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GltfResidency {
    /// Only the GPU samples the textures: site scenes, site props, indoor
    /// modules and navigation packages.
    GpuTextures,
    /// A skinned character pack. Its textures are GPU-only as above, and it
    /// spawns no cameras or lights.
    Character,
    /// A CPU pass reads the textures after loading (fixture mip chains and
    /// emission mask means) and hands each one to the render world itself
    /// with [`release_to_render_world`]. These are Bevy's default settings,
    /// so plain `AssetServer::load` requests of the same paths agree.
    CpuTextures,
}

/// Loads an image that only the GPU samples.
pub fn load_image<'a>(server: &AssetServer, path: impl Into<AssetPath<'a>>) -> Handle<Image> {
    load_image_with(server, path, |_| {})
}

/// [`load_image`] with further loader settings, such as the colour space or
/// the sampler. The residency is set after `configure` runs.
pub fn load_image_with<'a>(
    server: &AssetServer,
    path: impl Into<AssetPath<'a>>,
    configure: impl Fn(&mut ImageLoaderSettings) + Send + Sync + 'static,
) -> Handle<Image> {
    server.load_with_settings(path, move |settings: &mut ImageLoaderSettings| {
        configure(settings);
        settings.asset_usage = GPU_ONLY;
    })
}

/// Loads a glTF package with the residency its family needs.
pub fn load_gltf<'a>(
    server: &AssetServer,
    path: impl Into<AssetPath<'a>>,
    residency: GltfResidency,
) -> Handle<Gltf> {
    match residency {
        GltfResidency::CpuTextures => server.load(path),
        GltfResidency::GpuTextures => {
            server.load_with_settings(path, |settings: &mut GltfLoaderSettings| {
                settings.load_materials = GPU_ONLY;
            })
        }
        GltfResidency::Character => {
            server.load_with_settings(path, |settings: &mut GltfLoaderSettings| {
                settings.load_materials = GPU_ONLY;
                settings.load_cameras = false;
                settings.load_lights = false;
            })
        }
    }
}

/// Hands an image's pixels to the render world once its CPU readers are done.
/// Call it while the image is borrowed mutably, so that the extraction this
/// frame moves the pixels instead of cloning them and the main world keeps
/// only the descriptor. A later mutable borrow would find no pixels to
/// extract, so readers must check what they need through a shared borrow
/// first.
pub fn release_to_render_world(image: &mut Image) {
    image.asset_usage = GPU_ONLY;
}
