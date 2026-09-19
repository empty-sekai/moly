//! Colour-domain adaptation for source programs that render in encoded RGB.
//!
//! The source Gamma player blends into an UNorm attachment. EOTF-converting
//! only the fragment output before an sRGB attachment would still blend in
//! linear light, which is not equivalent. Keep the existing sorted transparent
//! phase and its draw commands, but render consecutive encoded-output batches
//! into an UNorm attachment. Point-copy the accumulated scene on each domain
//! boundary. This also retains ordering against existing linear-output draws.
use bevy::camera::{MainPassResolutionOverride, Viewport};
use bevy::core_pipeline::core_3d::{
    graph::{Core3d, Node3d},
    Transparent3d,
};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::{
    camera::ExtractedCamera,
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_graph::{NodeRunError, RenderGraph, RenderGraphContext, ViewNode, ViewNodeRunner},
    render_phase::{PhaseItem, ViewSortedRenderPhases},
    render_resource::*,
    renderer::{RenderContext, RenderDevice},
    texture::{CachedTexture, TextureCache},
    view::{ExtractedView, ViewDepthTexture, ViewTarget},
    Render, RenderApp, RenderSystems,
};
use std::collections::HashMap;

/// Explicit output contract, not inferred from a shader or material name.
#[derive(Component, Clone, Copy, Default, ExtractComponent)]
pub(crate) struct EncodedColorOutput;

pub(crate) const ENCODED_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const COPY_SHADER: Handle<Shader> = Handle::Uuid(
    bevy::asset::uuid::Uuid::from_u128(0xf6cc_ae3b_532e_40eb_a848_bab2_47e7_cad2),
    std::marker::PhantomData,
);

fn copy_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "source_colour_point_copy",
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    )
}

#[derive(Resource, Default)]
struct ColorGpu {
    pipelines: HashMap<TextureFormat, (CachedRenderPipelineId, CachedRenderPipelineId)>,
}

#[derive(Component)]
pub(crate) struct SourceColorView {
    texture: CachedTexture,
    to_encoded: CachedRenderPipelineId,
    to_linear: CachedRenderPipelineId,
}

impl SourceColorView {
    pub(crate) fn ready(&self, cache: &PipelineCache) -> Result<bool, String> {
        for id in [self.to_encoded, self.to_linear] {
            if let CachedPipelineState::Err(error) = cache.get_render_pipeline_state(id) {
                return Err(format!("source colour-domain pipeline failed: {error:?}"));
            }
        }
        Ok([self.to_encoded, self.to_linear]
            .into_iter()
            .all(|id| cache.get_render_pipeline(id).is_some()))
    }
}

/// Do not silently substitute linear blending or discard multisample depth.
pub(crate) fn compatible(target: &ViewTarget, depth: &ViewDepthTexture) -> bool {
    matches!(
        target.main_texture_format(),
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb
    ) && depth.texture.sample_count() == 1
        && target.main_texture().size() == depth.texture.size()
}

fn load_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        COPY_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/source_color.wgsl"),
            "moly_game/src/shaders/source_color.wgsl".to_owned(),
        ),
    );
}

fn prepare_views(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    mut gpu: ResMut<ColorGpu>,
    mut textures: ResMut<TextureCache>,
    views: Query<(Entity, &ViewTarget, &ViewDepthTexture)>,
) {
    for (entity, target, depth) in &views {
        if !compatible(target, depth) {
            commands.entity(entity).remove::<SourceColorView>();
            continue;
        }
        let &mut (to_encoded, to_linear) = gpu
            .pipelines
            .entry(target.main_texture_format())
            .or_insert_with(|| {
                let pipeline = |entry: &'static str, format| {
                    cache.queue_render_pipeline(RenderPipelineDescriptor {
                        label: Some(format!("source colour {entry}").into()),
                        layout: vec![copy_layout()],
                        vertex: VertexState {
                            shader: COPY_SHADER.clone(),
                            entry_point: Some("vertex".into()),
                            ..default()
                        },
                        fragment: Some(FragmentState {
                            shader: COPY_SHADER.clone(),
                            entry_point: Some(entry.into()),
                            targets: vec![Some(ColorTargetState {
                                format,
                                blend: None,
                                write_mask: ColorWrites::ALL,
                            })],
                            ..default()
                        }),
                        primitive: PrimitiveState {
                            cull_mode: None,
                            ..default()
                        },
                        ..default()
                    })
                };
                (
                    pipeline("to_encoded", ENCODED_FORMAT),
                    pipeline("to_linear", target.main_texture_format()),
                )
            });
        let texture = textures.get(
            &device,
            TextureDescriptor {
                label: Some("source encoded RGB blend attachment"),
                size: target.main_texture().size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: ENCODED_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(SourceColorView {
            texture,
            to_encoded,
            to_linear,
        });
    }
}

fn point_copy(
    context: &mut RenderContext,
    world: &World,
    pipeline: CachedRenderPipelineId,
    source: &TextureView,
    destination: &TextureView,
) {
    let cache = world.resource::<PipelineCache>();
    let Some(pipeline) = cache.get_render_pipeline(pipeline) else {
        return;
    };
    let group = context.render_device().create_bind_group(
        "source colour format conversion",
        &cache.get_bind_group_layout(&copy_layout()),
        &BindGroupEntries::single(source),
    );
    let attachments = [Some(RenderPassColorAttachment {
        view: destination,
        resolve_target: None,
        depth_slice: None,
        ops: Operations {
            load: LoadOp::Load,
            store: StoreOp::Store,
        },
    })];
    let mut pass = context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("source colour point copy"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.draw(0..3, 0..1);
}

#[derive(Default)]
struct SourceTransparentNode;
impl ViewNode for SourceTransparentNode {
    type ViewQuery = (
        &'static ExtractedCamera,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static ViewDepthTexture,
        Option<&'static MainPassResolutionOverride>,
        Option<&'static SourceColorView>,
    );

    fn run(
        &self,
        graph: &mut RenderGraphContext,
        context: &mut RenderContext,
        (camera, view, target, depth, resolution, color): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let Some(phases) = world.get_resource::<ViewSortedRenderPhases<Transparent3d>>() else {
            return Ok(());
        };
        let Some(phase) = phases.get(&view.retained_view_entity) else {
            return Ok(());
        };
        let cache = world.resource::<PipelineCache>();
        let mut start = 0;
        while start < phase.items.len() {
            let encoded = world
                .get::<EncodedColorOutput>(phase.items[start].entity())
                .is_some();
            let mut end = start;
            // Advance across complete batches: render_range itself skips the
            // other items covered by a batch leader. Never cut one in half.
            loop {
                end += phase.items[end].batch_range().len().max(1);
                if end >= phase.items.len()
                    || world
                        .get::<EncodedColorOutput>(phase.items[end].entity())
                        .is_some()
                        != encoded
                {
                    break;
                }
            }
            end = end.min(phase.items.len());
            let raw = if encoded {
                match color.filter(|color| color.ready(cache) == Ok(true)) {
                    Some(color) => Some(color),
                    None => {
                        // Source preflight remains pending/failed, not ready,
                        // until this route is usable. Do not draw into sRGB.
                        error!("encoded transparent draw has no ready colour-domain attachment");
                        start = end;
                        continue;
                    }
                }
            } else {
                None
            };
            if let Some(raw) = raw {
                point_copy(
                    context,
                    world,
                    raw.to_encoded,
                    target.main_texture_view(),
                    &raw.texture.default_view,
                );
            }
            {
                let attachment = match raw {
                    Some(raw) => RenderPassColorAttachment {
                        view: &raw.texture.default_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        },
                    },
                    None => target.get_color_attachment(),
                };
                let attachments = [Some(attachment)];
                let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("sorted transparent colour-domain span"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let Some(viewport) =
                    Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
                {
                    pass.set_camera_viewport(&viewport);
                }
                if let Err(error) =
                    phase.render_range(&mut pass, world, graph.view_entity(), start..end)
                {
                    error!(?error, "transparent phase draw failed");
                }
            }
            if let Some(raw) = raw {
                point_copy(
                    context,
                    world,
                    raw.to_linear,
                    &raw.texture.default_view,
                    target.main_texture_view(),
                );
            }
            start = end;
        }
        // A WebGL pass can leave a custom viewport in effect for the next node.
        // Retain the core pipeline's reset even when the last span is linear.
        if camera.viewport.is_some() || resolution.is_some() {
            let attachments = [Some(target.get_color_attachment())];
            let _pass = context
                .command_encoder()
                .begin_render_pass(&RenderPassDescriptor {
                    label: Some("transparent viewport reset"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
        }
        Ok(())
    }
}

pub(crate) struct SourceColorPlugin;
impl Plugin for SourceColorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<EncodedColorOutput>::default())
            .add_systems(Startup, load_shader);
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render.init_resource::<ColorGpu>().add_systems(
            Render,
            prepare_views.in_set(RenderSystems::PrepareResources),
        );
        // Replace only the runner, retaining every incoming/outgoing graph edge
        // and the existing phase. add_node would discard the node's own edges.
        let runner = ViewNodeRunner::new(SourceTransparentNode, render.world_mut());
        let mut graph = render.world_mut().resource_mut::<RenderGraph>();
        let state = graph
            .sub_graph_mut(Core3d)
            .get_node_state_mut(Node3d::MainTransparentPass)
            .expect("core transparent phase is installed before source colour adaptation");
        state.node = Box::new(runner);
        state.type_name = std::any::type_name::<ViewNodeRunner<SourceTransparentNode>>();
    }
}
