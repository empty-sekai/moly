//! Preserve source surface queues before Bevy prepares opaque mesh batches.
//!
//! Rug materials deliberately use Always depth comparison and no depth writes.
//! They must follow ground and precede objects. Pipeline creation order cannot
//! supply that ordering. The queue is pipeline metadata (an unused shader def),
//! so all instances in a batch share it without breaking instancing.

use std::cmp::Ordering;
use std::collections::HashMap;

use bevy::{
    core_pipeline::core_3d::Opaque3d,
    prelude::*,
    render::{
        batching::sort_binned_render_phase,
        render_phase::ViewBinnedRenderPhases,
        render_resource::{
            CachedRenderPipelineId, PipelineCache, RenderPipelineDescriptor,
        },
        Render, RenderApp, RenderSystems,
    },
};

use bevy::shader::ShaderDefVal;

const QUEUE_DEF: &str = "MOLY_SOURCE_RENDER_QUEUE";

pub fn set_queue(descriptor: &mut RenderPipelineDescriptor, queue: u32) {
    descriptor
        .vertex
        .shader_defs
        .push(ShaderDefVal::UInt(QUEUE_DEF.into(), queue));
}

/// The queue a pipeline was specialized with. A cached pipeline id always
/// names the same descriptor (the cache only appends), so the value is read
/// from the descriptor once per id and remembered.
fn queue_of(
    queues: &mut HashMap<CachedRenderPipelineId, u32>,
    cache: &PipelineCache,
    id: CachedRenderPipelineId,
) -> u32 {
    *queues.entry(id).or_insert_with(|| {
        cache
            .get_render_pipeline_descriptor(id)
            .vertex
            .shader_defs
            .iter()
            .find_map(|def| match def {
                ShaderDefVal::UInt(name, value) if name == QUEUE_DEF => Some(*value),
                _ => None,
            })
            .unwrap_or(2065)
    })
}

fn order_surfaces(
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut cache: ResMut<PipelineCache>,
    mut queues: Local<HashMap<CachedRenderPipelineId, u32>>,
) {
    // PhaseSort precedes Bevy's usual Render-time queue processing. Publish
    // queued descriptors now so even the first rendered frame has the source
    // ordering. This is the public early-processing path of PipelineCache.
    cache.process_queue();
    let cache = &*cache;
    let queues = &mut *queues;
    // The order is (queue, bin key). The keys of one map are distinct, so this
    // is a strict total order: a map already in that order is left as it is,
    // and sorting it again would reproduce the same sequence.
    let mut order = |a: CachedRenderPipelineId, b: CachedRenderPipelineId| {
        queue_of(queues, cache, a).cmp(&queue_of(queues, cache, b))
    };
    for phase in phases.values_mut() {
        if !phase.multidrawable_meshes.keys().is_sorted_by(|a, b| {
            order(a.pipeline, b.pipeline).then_with(|| a.cmp(b)) != Ordering::Greater
        }) {
            phase.multidrawable_meshes.sort_by(|a, _, b, _| {
                order(a.pipeline, b.pipeline).then_with(|| a.cmp(b))
            });
        }
        if !phase.batchable_meshes.keys().is_sorted_by(|a, b| {
            order(a.0.pipeline, b.0.pipeline).then_with(|| a.cmp(b)) != Ordering::Greater
        }) {
            phase.batchable_meshes.sort_by(|a, _, b, _| {
                order(a.0.pipeline, b.0.pipeline).then_with(|| a.cmp(b))
            });
        }
        if !phase.unbatchable_meshes.keys().is_sorted_by(|a, b| {
            order(a.0.pipeline, b.0.pipeline).then_with(|| a.cmp(b)) != Ordering::Greater
        }) {
            phase.unbatchable_meshes.sort_by(|a, _, b, _| {
                order(a.0.pipeline, b.0.pipeline).then_with(|| a.cmp(b))
            });
        }
    }
}

pub struct MaterialOrderPlugin;

impl Plugin for MaterialOrderPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                Render,
                order_surfaces
                    .in_set(RenderSystems::PhaseSort)
                    .after(sort_binned_render_phase::<Opaque3d>),
            );
        }
    }
}
