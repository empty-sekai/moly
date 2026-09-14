//! Preserve source surface queues before Bevy prepares opaque mesh batches.
//!
//! Rug materials deliberately use Always depth comparison and no depth writes.
//! They must follow ground and precede objects. Pipeline creation order cannot
//! supply that ordering. The queue is pipeline metadata (an unused shader def),
//! so all instances in a batch share it without breaking instancing.

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

fn order_surfaces(mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>, mut cache: ResMut<PipelineCache>) {
    // PhaseSort precedes Bevy's usual Render-time queue processing. Publish
    // queued descriptors now so even the first rendered frame has the source
    // ordering. This is the public early-processing path of PipelineCache.
    cache.process_queue();
    let queue = |id: CachedRenderPipelineId| {
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
    };
    for phase in phases.values_mut() {
        phase.multidrawable_meshes.sort_by(|a, _, b, _| {
            queue(a.pipeline)
                .cmp(&queue(b.pipeline))
                .then_with(|| a.cmp(b))
        });
        phase.batchable_meshes.sort_by(|a, _, b, _| {
            queue(a.0.pipeline)
                .cmp(&queue(b.0.pipeline))
                .then_with(|| a.cmp(b))
        });
        phase.unbatchable_meshes.sort_by(|a, _, b, _| {
            queue(a.0.pipeline)
                .cmp(&queue(b.0.pipeline))
                .then_with(|| a.cmp(b))
        });
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
