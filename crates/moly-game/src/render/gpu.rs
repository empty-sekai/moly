//! GPU object lifetime helpers.
//!
//! On the WebGPU backend a dropped buffer, texture, sampler or bind group is
//! only released when the browser garbage-collects its JS wrapper, so objects
//! that are identical from frame to frame are created once and reused here.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bevy::prelude::*;
use bevy::render::render_resource::{
    AddressMode, BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutId, BindingResource,
    Buffer, BufferBinding, BufferId, FilterMode, Sampler, SamplerDescriptor, SamplerId, TextureView,
    TextureViewId,
};
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

/// One resource bound through [`BindGroupCache`]. The cache key and the bind
/// group entries are both derived from this list, so a cached group is reused
/// only when it binds exactly the same objects and buffer ranges.
#[derive(Clone, Copy)]
pub(crate) enum Bound<'a> {
    View(&'a TextureView),
    Sampler(&'a Sampler),
    /// A buffer range: byte offset and size, `None` meaning to the end.
    Buffer(&'a Buffer, u64, Option<NonZeroU64>),
}

impl<'a> Bound<'a> {
    /// The whole buffer, as `as_entire_binding` binds it.
    pub(crate) fn whole(buffer: &'a Buffer) -> Self {
        Bound::Buffer(buffer, 0, None)
    }

    fn id(&self) -> BoundId {
        match *self {
            Bound::View(view) => BoundId::View(view.id()),
            Bound::Sampler(sampler) => BoundId::Sampler(sampler.id()),
            Bound::Buffer(buffer, offset, size) => BoundId::Buffer(buffer.id(), offset, size),
        }
    }

    fn resource(&self) -> BindingResource<'a> {
        match *self {
            Bound::View(view) => BindingResource::TextureView(view),
            Bound::Sampler(sampler) => BindingResource::Sampler(sampler),
            Bound::Buffer(buffer, offset, size) => {
                BindingResource::Buffer(BufferBinding { buffer, offset, size })
            }
        }
    }
}

/// Most resources a cached bind group may bind.
const MAX_BOUND: usize = 8;

/// Frames a cached bind group may stay unused before it is dropped. The
/// engine's texture cache frees an unused texture after the same number of
/// frames, so a group cannot keep a released texture alive for long.
const MAX_IDLE_FRAMES: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum BoundId {
    Unused,
    View(TextureViewId),
    Sampler(SamplerId),
    Buffer(BufferId, u64, Option<NonZeroU64>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BindGroupKey {
    layout: BindGroupLayoutId,
    bound: [(u32, BoundId); MAX_BOUND],
}

/// Bind groups keyed by their layout and the ids of every bound resource
/// (resource ids are never reused), with frame-age eviction. A render node
/// keeps one behind a `Mutex`, because nodes run with shared access only.
#[derive(Default)]
pub(crate) struct BindGroupCache {
    groups: HashMap<BindGroupKey, (BindGroup, u32)>,
}

impl BindGroupCache {
    /// The bind group of `layout` binding exactly `entries`, created on a miss
    /// with `label`. `frame` stamps the group as used in this frame.
    pub(crate) fn get(
        &mut self,
        device: &RenderDevice,
        label: &'static str,
        layout: &BindGroupLayout,
        entries: &[(u32, Bound)],
        frame: u32,
    ) -> BindGroup {
        assert!(entries.len() <= MAX_BOUND, "a cached bind group binds at most {MAX_BOUND} resources");
        let mut bound = [(0, BoundId::Unused); MAX_BOUND];
        for (slot, (binding, resource)) in bound.iter_mut().zip(entries) {
            *slot = (*binding, resource.id());
        }
        let key = BindGroupKey { layout: layout.id(), bound };
        if let Some((group, used)) = self.groups.get_mut(&key) {
            *used = frame;
            return group.clone();
        }
        let wgpu_entries: Vec<BindGroupEntry> = entries
            .iter()
            .map(|(binding, resource)| BindGroupEntry { binding: *binding, resource: resource.resource() })
            .collect();
        let group = device.create_bind_group(label, layout, &wgpu_entries);
        self.groups.insert(key, (group.clone(), frame));
        group
    }

    /// Drops the groups not used during the last [`MAX_IDLE_FRAMES`] frames.
    pub(crate) fn evict_idle(&mut self, frame: u32) {
        self.groups.retain(|_, (_, used)| frame.wrapping_sub(*used) <= MAX_IDLE_FRAMES);
    }
}
