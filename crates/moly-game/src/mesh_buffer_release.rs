//! Explicit release of mesh-allocator buffers that Bevy retires.
//!
//! The WebGPU backend's `Drop` for buffers is a no-op: a dropped buffer keeps
//! its GPU allocation until the JavaScript garbage collector finalizes the
//! `GPUBuffer`. Bevy's mesh allocator drops a slab when its last mesh is freed
//! and drops the old buffer when a slab grows. A mesh that is rewritten every
//! frame (a CPU-simulated particle system, for example) is freed and allocated
//! again every frame, so its slab is emptied and recreated each time and the
//! retired slabs accumulate between collections.
//!
//! This render-world pair snapshots the buffers referenced by allocated meshes
//! before the allocator runs and destroys the ones no remaining mesh references
//! afterwards. Work already submitted with such a buffer completes before its
//! memory is reused, and nothing recorded later in the frame can refer to a
//! freed or grown slab. On native wgpu `destroy` releases the same buffers that
//! `Drop` would release.

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::{
        mesh::{
            allocator::{allocate_and_free_meshes, MeshAllocator},
            RenderMesh,
        },
        render_asset::ExtractedAssets,
        render_resource::{Buffer, BufferId},
        Render, RenderApp, RenderSystems,
    },
};

pub struct MeshBufferReleasePlugin;

impl Plugin for MeshBufferReleasePlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<AllocatedMeshBuffers>()
            .add_systems(
                Render,
                (
                    snapshot_before_allocation
                        .in_set(RenderSystems::PrepareAssets)
                        .before(allocate_and_free_meshes),
                    destroy_retired_buffers
                        .in_set(RenderSystems::PrepareAssets)
                        .after(allocate_and_free_meshes),
                ),
            );
    }
}

#[derive(Resource, Default)]
struct AllocatedMeshBuffers {
    /// Every mesh that has passed through the allocator and was not removed.
    /// A retired slab is recognized only by comparing against all of them: an
    /// unchanged mesh may still share the slab of a freed one.
    meshes: HashSet<AssetId<Mesh>>,
    before: HashMap<BufferId, Buffer>,
    after: HashMap<BufferId, Buffer>,
}

fn referenced_buffers(
    allocator: &MeshAllocator,
    meshes: &HashSet<AssetId<Mesh>>,
    buffers: &mut HashMap<BufferId, Buffer>,
) {
    buffers.clear();
    for id in meshes {
        for slice in [allocator.mesh_vertex_slice(id), allocator.mesh_index_slice(id)]
            .into_iter()
            .flatten()
        {
            buffers
                .entry(slice.buffer.id())
                .or_insert_with(|| slice.buffer.clone());
        }
    }
}

fn changes(extracted: &ExtractedAssets<RenderMesh>) -> bool {
    !extracted.removed.is_empty() || !extracted.extracted.is_empty()
}

fn snapshot_before_allocation(
    extracted: Res<ExtractedAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    mut state: ResMut<AllocatedMeshBuffers>,
) {
    let state = &mut *state;
    state.before.clear();
    if changes(&extracted) {
        referenced_buffers(&allocator, &state.meshes, &mut state.before);
    }
}

fn destroy_retired_buffers(
    extracted: Res<ExtractedAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    mut state: ResMut<AllocatedMeshBuffers>,
) {
    if !changes(&extracted) {
        return;
    }
    let state = &mut *state;
    for id in &extracted.removed {
        state.meshes.remove(id);
    }
    state
        .meshes
        .extend(extracted.extracted.iter().map(|(id, _)| *id));
    referenced_buffers(&allocator, &state.meshes, &mut state.after);
    for (id, buffer) in &state.before {
        if !state.after.contains_key(id) {
            buffer.destroy();
        }
    }
    state.before.clear();
    state.after.clear();
}
