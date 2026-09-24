//! Explicit release of GPU textures that belong to removed images.
//!
//! The WebGPU backend's `Drop` for textures is a no-op: a dropped texture keeps
//! its GPU allocation until the JavaScript garbage collector finalizes the
//! `GPUTexture` and every bind group that references it. Bevy drops an image's
//! texture when the image asset is removed and when the image is uploaded
//! again after a change on the CPU (a mip chain appended to the pixels, for
//! example); in both cases the allocation outlives the asset.
//!
//! # Removed images
//!
//! When an image asset becomes unused, its current texture and the textures
//! it had before an upload that replaced them (see [`ImageTextureReplaced`])
//! are destroyed in the same frame, during extraction and so before the render
//! world drops the image. No bind group that is still used can reference
//! them:
//! - An image asset becomes unused only once no strong handle to it remains.
//!   Every material binds its images through strong handles, so every
//!   material asset that could have captured one of these textures was
//!   removed first or in the same frame, and its bind group with it. A
//!   material asset can only stop holding a handle through a mutable borrow
//!   that marks it modified, which prepares it again with a new bind group.
//! - Entities that referenced the image are gone from the main world, so
//!   nothing extracted this frame draws with it.
//! - Bind groups cached by render nodes are keyed by the ids of the texture
//!   views they bind. Nodes look views up in `RenderAssets<GpuImage>`, which
//!   holds neither of these views after this frame's image preparation, so
//!   such a cached group is never used again; it only ages out.
//!
//! Work submitted in earlier frames completes before the memory is reused.
//!
//! # Replaced textures
//!
//! A texture that an upload replaces is not destroyed at once: a material
//! prepared in an earlier frame may still bind it (the imported glTF
//! materials of furniture, for example, are prepared at import and never
//! again, and scenes spawned later draw with them until their materials are
//! switched), and destroying it would invalidate that material's bind group.
//! The old texture is kept until its image is removed and destroyed then, by
//! the argument above. Only uploads recorded with [`ImageTextureReplaced`] are
//! kept this way; each such image is replaced at most once (a chained image is
//! never chained again), so the list is bounded by the number of images.
//!
//! # Schedule
//!
//! The release system runs in the extraction schedule and orders against
//! nothing: it reads the main world's image events and the recorded
//! replacements, and the render world's prepared images, which change only in
//! image preparation after extraction. The render schedule is left as it is.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::Texture;
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, RenderApp};

pub struct GpuImageReleasePlugin;

impl Plugin for GpuImageReleasePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ImageTextureReplaced>();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<RetiredTextures>()
            .add_systems(ExtractSchedule, release_image_textures);
    }
}

/// Adds [`GpuImageReleasePlugin`] once. Every plugin that sends
/// [`ImageTextureReplaced`] calls it.
pub(crate) fn install(app: &mut App) {
    if !app.is_plugin_added::<GpuImageReleasePlugin>() {
        app.add_plugins(GpuImageReleasePlugin);
    }
}

/// The pixels of this image were changed on the CPU this frame, so that this
/// frame's upload replaces the GPU texture it already has. Code that changes
/// the pixels of an image that may already be on the GPU sends it in the same
/// frame; the previous texture is then kept until the image is removed.
#[derive(Message, Clone, Copy)]
pub(crate) struct ImageTextureReplaced(pub(crate) AssetId<Image>);

/// Textures an image had before an upload replaced them, kept until the image
/// is removed.
#[derive(Resource, Default)]
struct RetiredTextures(HashMap<AssetId<Image>, Vec<Texture>>);

fn release_image_textures(
    mut events: Extract<MessageReader<AssetEvent<Image>>>,
    mut replaced: Extract<MessageReader<ImageTextureReplaced>>,
    images: Res<RenderAssets<GpuImage>>,
    mut retired: ResMut<RetiredTextures>,
) {
    // A recorded image is extracted this frame and its texture replaced by
    // this frame's image preparation; until then the render world still holds
    // the texture it replaces.
    for ImageTextureReplaced(id) in replaced.read() {
        if let Some(previous) = images.get(*id) {
            retired.0.entry(*id).or_default().push(previous.texture.clone());
        }
    }
    for event in events.read() {
        let AssetEvent::Unused { id } = event else {
            continue;
        };
        if let Some(image) = images.get(*id) {
            image.texture.destroy();
        }
        for texture in retired.0.remove(id).into_iter().flatten() {
            texture.destroy();
        }
    }
}
