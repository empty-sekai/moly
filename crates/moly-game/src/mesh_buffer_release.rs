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
//! This render-world system runs after the allocator has freed and allocated
//! the frame's meshes and destroys the buffers no allocated mesh references any
//! more. Work already submitted with such a buffer completes before its memory
//! is reused, and nothing recorded later in the frame can refer to a freed or
//! grown slab. On native wgpu `destroy` releases the same buffers that `Drop`
//! would release.
//!
//! The work is proportional to the meshes extracted, modified or removed in a
//! frame, not to all allocated meshes. For every allocated mesh the system keeps
//! the slabs holding its vertex and index data, and for every such slab its
//! buffer and the number of mesh slices in it. The allocator changes a slab only
//! when it frees a changed mesh from it or allocates a changed mesh into it: it
//! never moves an unchanged mesh to another slab (growth keeps the slab and
//! copies it into a larger buffer), a free never replaces a buffer, an emptied
//! slab is removed and its id is never handed out again, and every buffer
//! belongs to one slab. So only the slabs of changed meshes can retire a
//! buffer: the old buffer of such a slab is retired when no mesh slice is left
//! in the slab, or when growth gave the slab a new buffer.

use bevy::{
    platform::collections::{hash_map::Entry, HashMap},
    prelude::*,
    render::{
        mesh::{
            allocator::{allocate_and_free_meshes, MeshAllocator, SlabId},
            RenderMesh,
        },
        render_asset::{prepare_assets, ExtractedAssets},
        render_resource::Buffer,
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
                // The allocator has applied this frame's changes, and
                // prepare_assets::<RenderMesh> has not yet drained the
                // extracted-asset sets this system reads.
                destroy_retired_buffers
                    .in_set(RenderSystems::PrepareAssets)
                    .after(allocate_and_free_meshes)
                    .before(prepare_assets::<RenderMesh>),
            );
    }
}

#[derive(Resource, Default)]
struct AllocatedMeshBuffers {
    /// The slabs of the vertex and index slices of every allocated mesh, as
    /// the allocator placed them.
    meshes: HashMap<AssetId<Mesh>, [Option<SlabId>; 2]>,
    /// Every slab holding a slice of an allocated mesh: its current buffer and
    /// the number of mesh slices in it.
    slabs: HashMap<SlabId, (Buffer, u32)>,
}

/// The vertex and index slices of a mesh: the slab and buffer each lies in.
/// Only a mesh whose data has been copied in has a slice.
fn slices<'a>(
    allocator: &'a MeshAllocator,
    id: &AssetId<Mesh>,
) -> [Option<(SlabId, &'a Buffer)>; 2] {
    let (vertex_slab, index_slab) = allocator.mesh_slabs(id);
    [
        vertex_slab
            .zip(allocator.mesh_vertex_slice(id))
            .map(|(slab, slice)| (slab, slice.buffer)),
        index_slab
            .zip(allocator.mesh_index_slice(id))
            .map(|(slab, slice)| (slab, slice.buffer)),
    ]
}

impl AllocatedMeshBuffers {
    /// Brings the tracking up to date with the allocation the allocator just
    /// made for the frame's changed meshes, and returns the buffers it retired.
    fn retire(
        &mut self,
        allocator: &MeshAllocator,
        extracted: &ExtractedAssets<RenderMesh>,
    ) -> Vec<Buffer> {
        let extracted_ids = extracted.extracted.iter().map(|(id, _)| id);
        let changed = extracted
            .removed
            .iter()
            .chain(&extracted.modified)
            .chain(extracted_ids.clone());
        // The buffer each touched slab had before this frame's allocation.
        let mut before: HashMap<SlabId, Buffer> = HashMap::default();
        for id in changed {
            let Some(slabs) = self.meshes.remove(id) else {
                continue;
            };
            for slab in slabs.into_iter().flatten() {
                let (buffer, count) = self
                    .slabs
                    .get_mut(&slab)
                    .expect("the slab of a tracked mesh slice is tracked");
                *count -= 1;
                before.entry(slab).or_insert_with(|| buffer.clone());
            }
        }
        // Removed and modified meshes were freed; only the extracted ones were
        // allocated again. Each extracted id occurs once.
        for id in extracted_ids {
            let slices = slices(allocator, id);
            if slices.iter().all(Option::is_none) {
                continue;
            }
            for (slab, buffer) in slices.iter().flatten() {
                match self.slabs.entry(*slab) {
                    Entry::Occupied(mut entry) => {
                        let (tracked, count) = entry.get_mut();
                        if tracked.id() != buffer.id() {
                            before.entry(*slab).or_insert_with(|| tracked.clone());
                            *tracked = (*buffer).clone();
                        }
                        *count += 1;
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(((*buffer).clone(), 1));
                    }
                }
            }
            self.meshes
                .insert(*id, slices.map(|slice| slice.map(|(slab, _)| slab)));
        }
        let mut retired = Vec::new();
        for (slab, old) in before {
            let (buffer, count) = &self.slabs[&slab];
            if *count == 0 {
                self.slabs.remove(&slab);
                retired.push(old);
            } else if buffer.id() != old.id() {
                retired.push(old);
            }
        }
        retired
    }

    /// The buffers to destroy this frame. Buffers are destroyed on frames on
    /// which a mesh was extracted or removed. A frame whose only change is a
    /// modified mesh that was not extracted again (its asset left the main
    /// world or lost render-world usage) frees that mesh without destroying
    /// what the free retired.
    fn release(
        &mut self,
        allocator: &MeshAllocator,
        extracted: &ExtractedAssets<RenderMesh>,
    ) -> Vec<Buffer> {
        if extracted.removed.is_empty()
            && extracted.modified.is_empty()
            && extracted.extracted.is_empty()
        {
            return Vec::new();
        }
        let retired = self.retire(allocator, extracted);
        if extracted.removed.is_empty() && extracted.extracted.is_empty() {
            return Vec::new();
        }
        retired
    }
}

fn destroy_retired_buffers(
    extracted: Res<ExtractedAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    mut state: ResMut<AllocatedMeshBuffers>,
) {
    for buffer in state.release(&allocator, &extracted) {
        buffer.destroy();
    }
}
