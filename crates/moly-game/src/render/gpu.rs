//! GPU object lifetime helpers.
//!
//! On the WebGPU backend a dropped buffer, texture, sampler or bind group is
//! only released when the browser garbage-collects its JS wrapper, so objects
//! that are identical from frame to frame are created once and reused here.

use bevy::prelude::*;
use bevy::render::render_resource::{AddressMode, FilterMode, Sampler, SamplerDescriptor};
use bevy::render::renderer::RenderDevice;
use bevy::render::{RenderApp, RenderStartup};

/// Stores `value` into `slot` only when a bit differs and reports whether it
/// wrote. Floats compare by bit pattern, so a store is skipped only when it
/// would write the very same bits (`-0.0` still replaces `0.0`).
pub(crate) fn store_bits<const N: usize>(slot: &mut [f32; N], value: [f32; N]) -> bool {
    if slot.map(f32::to_bits) == value.map(f32::to_bits) {
        return false;
    }
    *slot = value;
    true
}

/// Samplers whose descriptors never change, created once at render startup and
/// bound by every material that samples with them. Public because material
/// bind-group parameters name it.
#[derive(Resource)]
pub struct SharedSamplers {
    /// Repeat on all three axes with trilinear filtering.
    pub repeat_linear: Sampler,
    /// Clamp-to-edge on all three axes with trilinear filtering.
    pub clamp_linear: Sampler,
}

fn create_shared_samplers(mut commands: Commands, device: Res<RenderDevice>) {
    let trilinear = |mode: AddressMode| SamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        address_mode_w: mode,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: FilterMode::Linear,
        ..Default::default()
    };
    commands.insert_resource(SharedSamplers {
        repeat_linear: device.create_sampler(&trilinear(AddressMode::Repeat)),
        clamp_linear: device.create_sampler(&trilinear(AddressMode::ClampToEdge)),
    });
}

/// Installs [`SharedSamplers`] in the render world. Every material plugin that
/// binds them adds this plugin once through [`install_shared_samplers`].
pub(crate) struct SharedSamplersPlugin;

impl Plugin for SharedSamplersPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(RenderStartup, create_shared_samplers);
        }
    }
}

pub(crate) fn install_shared_samplers(app: &mut App) {
    if !app.is_plugin_added::<SharedSamplersPlugin>() {
        app.add_plugins(SharedSamplersPlugin);
    }
}
