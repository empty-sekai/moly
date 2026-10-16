//! Names the render pipelines as they become ready.
//!
//! Each frame in which pipelines finish creation logs one line: the frame
//! number, how many became ready, and their labels with counts. A pipeline is
//! created on first use, and on the WebGL2 backend its shaders compile on
//! the main thread inside that frame, so the line says which draws paid for
//! a long frame.

use std::collections::BTreeMap;

use bevy::{
    diagnostic::FrameCount,
    prelude::*,
    render::{
        render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor},
        Render, RenderApp, RenderSystems,
    },
};

pub fn install(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(Render, report.in_set(RenderSystems::Cleanup));
    }
}

/// `ready[i]`: pipeline `i` was already reported ready.
fn report(cache: Res<PipelineCache>, frame: Option<Res<FrameCount>>, mut ready: Local<Vec<bool>>) {
    let mut labels: BTreeMap<&str, usize> = BTreeMap::new();
    let mut count = 0usize;
    for (index, pipeline) in cache.pipelines().enumerate() {
        if index == ready.len() {
            ready.push(false);
        }
        if ready[index] || !matches!(pipeline.state, CachedPipelineState::Ok(_)) {
            continue;
        }
        ready[index] = true;
        count += 1;
        let label = match &pipeline.descriptor {
            PipelineDescriptor::RenderPipelineDescriptor(descriptor) => descriptor.label.as_deref(),
            PipelineDescriptor::ComputePipelineDescriptor(descriptor) => descriptor.label.as_deref(),
        };
        *labels.entry(label.unwrap_or("(unlabelled)")).or_default() += 1;
    }
    if count == 0 {
        return;
    }
    let list: Vec<String> = labels
        .into_iter()
        .map(|(label, n)| if n == 1 { label.to_owned() } else { format!("{label} x{n}") })
        .collect();
    info!(
        "[pipelines] frame {}: {count} ready ({} in the cache): {}",
        frame.map_or(0, |frame| frame.0),
        ready.len(),
        list.join(", ")
    );
}
