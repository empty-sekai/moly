//! Snapshot the actual opaque camera depth for weather-particle sampling.
//!
//! Copy the completed opaque attachment without replaying geometry. A raw
//! texel-fetch/depth-write pass works on both browser backends; the GLES
//! copy_texture_to_texture path instead attaches depth as colour and is invalid.
//! Do not enable an engine DepthPrepass merely to obtain a sampleable texture:
//! it performs the same invalid copy and its geometry coverage is different.
//! Effect and forward particles sample this stable per-view snapshot, while
//! Effect depth testing still uses the actual ViewDepthTexture attachment.
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::render_graph::{NodeRunError, RenderGraphContext, RenderLabel, ViewNode};
use bevy::render::renderer::RenderContext;
use bevy::render::view::ViewDepthTexture;
use bevy::render::render_resource::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, RenderLabel)]
pub(crate) struct WeatherOpaqueDepth;

/// Explicit request for a post-opaque snapshot, independent of engine prepasses.
#[derive(Component, Clone, Copy, Default, bevy::render::extract_component::ExtractComponent)]
pub(crate) struct WeatherDepthSnapshot;

#[derive(Component)]
pub(crate) struct PreparedDepthSnapshot {
    texture: CachedTexture,
    source: BindGroup,
}

#[derive(Default)]
pub(crate) struct WeatherOpaqueDepthNode;

impl ViewNode for WeatherOpaqueDepthNode {
    type ViewQuery = &'static PreparedDepthSnapshot;
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        snapshot: QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let gpu = world.resource::<RawDepthGpu>();
        let cache = world.resource::<PipelineCache>();
        let Some(pipeline) = cache.get_render_pipeline(gpu.copy_pipeline) else { return Ok(()); };
        let mut pass = render_context.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("weather_copy_actual_opaque_depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &snapshot.texture.default_view,
                depth_ops: Some(Operations { load: LoadOp::Clear(0.0), store: StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &snapshot.source, &[]);
        // No viewport/scissor subset: every texel, including the camera clear
        // outside a viewport, must be copied before the valid binding is read.
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

/// This is an authored camera role, never inferred from render order or layers.
/// The effect feature rejects Overlay cameras; unknown roles also fail closed.
#[derive(bevy::prelude::Component, Clone, Copy, Debug, PartialEq, Eq,
    bevy::render::extract_component::ExtractComponent)]
pub(crate) enum WeatherCameraRole { Base, Overlay }

pub(crate) fn effect_attachment_compatible(role: Option<WeatherCameraRole>, camera_depth_samples: u32) -> bool {
    role == Some(WeatherCameraRole::Base) && camera_depth_samples == 1
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    #[test]
    fn effect_target_never_invents_msaa_to_match_the_camera() {
        assert!(effect_attachment_compatible(Some(WeatherCameraRole::Base),1));
        for samples in [0,2,4,8,16] {
            assert!(!effect_attachment_compatible(Some(WeatherCameraRole::Base),samples));
        }
    }
    #[test]
    fn overlay_and_unclassified_cameras_do_not_emit_effect_passes() {
        assert!(!effect_attachment_compatible(Some(WeatherCameraRole::Overlay),1));
        assert!(!effect_attachment_compatible(None,1));
    }
}

#[cfg(test)]
#[path = "weather_depth_shader_tests.rs"]
mod shader_tests;


/// wgpu permits a depth-format view with an unfilterable-float binding. Naga's
/// GLSL backend can therefore use raw texelFetch, without comparison samplers.
/// The texture itself is the unchanged camera-depth snapshot, per view.
pub(crate) fn raw_depth_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new("weather_raw_camera_depth", &[
        BindGroupLayoutEntry { binding:0, visibility:ShaderStages::FRAGMENT,
            ty:BindingType::Texture { sample_type:TextureSampleType::Float {filterable:false},
                view_dimension:TextureViewDimension::D2,multisampled:false },count:None },
        BindGroupLayoutEntry {binding:1,visibility:ShaderStages::FRAGMENT,
            ty:BindingType::Buffer {ty:BufferBindingType::Uniform,has_dynamic_offset:false,
                min_binding_size:std::num::NonZeroU64::new(16)},count:None},
    ])
}

#[derive(Component)]
pub(crate) struct RawDepthBinding { pub group: BindGroup }

#[derive(Resource)]
struct RawDepthGpu {
    invalid_view: TextureView,
    valid: Buffer,
    invalid: Buffer,
    copy_pipeline: CachedRenderPipelineId,
}

const DEPTH_COPY_SHADER: Handle<Shader> = Handle::Uuid(
    bevy::asset::uuid::Uuid::from_u128(0x62f5_a004_42ac_43ec_901b_6a1d_2731_f7a9),
    std::marker::PhantomData,
);

fn load_depth_copy(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(DEPTH_COPY_SHADER.id(), Shader::from_wgsl(
        include_str!("shaders/weather_depth_copy.wgsl"),
        "moly_game/src/shaders/weather_depth_copy.wgsl".to_owned(),
    ));
}

fn depth_copy_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new("weather_copy_raw_depth", &[raw_depth_layout().entries[0]])
}

fn init_raw_depth(mut commands:Commands, device:Res<bevy::render::renderer::RenderDevice>,
    cache: Res<PipelineCache>) {
    let invalid_view=device.create_texture(&TextureDescriptor {
        label:Some("weather_depth_unavailable_not_a_source_depth"),size:Extent3d {width:1,height:1,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:TextureDimension::D2,format:TextureFormat::R32Float,
        usage:TextureUsages::TEXTURE_BINDING,view_formats:&[],
    }).create_view(&TextureViewDescriptor::default());
    let valid=device.create_buffer_with_data(&BufferInitDescriptor {
        label:Some("weather_depth_available"),contents:&[1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],usage:BufferUsages::UNIFORM,
    });
    let invalid=device.create_buffer_with_data(&BufferInitDescriptor {
        label:Some("weather_depth_unavailable"),contents:&[0;16],usage:BufferUsages::UNIFORM,
    });
    let copy_pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("weather_raw_depth_copy_pipeline".into()),
        layout: vec![depth_copy_layout()],
        vertex: VertexState {
            shader: DEPTH_COPY_SHADER.clone(), entry_point: Some("vertex".into()),
            ..Default::default()
        },
        fragment: Some(FragmentState {
            shader: DEPTH_COPY_SHADER.clone(), entry_point: Some("fragment".into()),
            targets: vec![], ..Default::default()
        }),
        primitive: PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: CompareFunction::Always,
            stencil: StencilState::default(), bias: DepthBiasState::default(),
        }),
        multisample: MultisampleState::default(),
        ..Default::default()
    });
    commands.insert_resource(RawDepthGpu {invalid_view,valid,invalid,copy_pipeline});
}

fn prepare_raw_depth(
    mut commands: Commands,
    device: Res<bevy::render::renderer::RenderDevice>,
    cache: Res<PipelineCache>,
    mut textures: ResMut<TextureCache>,
    gpu: Res<RawDepthGpu>,
    views: Query<(Entity, &ViewDepthTexture, Option<&WeatherDepthSnapshot>, Option<&WeatherCameraRole>)>,
) {
    for (entity, main, requested, role) in &views {
        let compatible = requested.is_some()
            && effect_attachment_compatible(role.copied(), main.texture.sample_count())
            && main.texture.format() == TextureFormat::Depth32Float
            && main.texture.usage().contains(TextureUsages::TEXTURE_BINDING)
            && cache.get_render_pipeline(gpu.copy_pipeline).is_some();
        let snapshot = compatible.then(|| {
            let texture = textures.get(&device, TextureDescriptor {
                label: Some("weather_actual_opaque_depth_snapshot"),
                size: main.texture.size(), mip_level_count: 1, sample_count: 1,
                dimension: TextureDimension::D2, format: main.texture.format(),
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let source_view = main.texture.create_view(&TextureViewDescriptor::default());
            let source = device.create_bind_group("weather_actual_depth_source",
                &cache.get_bind_group_layout(&depth_copy_layout()),
                &BindGroupEntries::single(&source_view));
            PreparedDepthSnapshot { texture, source }
        });
        let (view, valid) = match &snapshot {
            Some(snapshot) => (&snapshot.texture.default_view, &gpu.valid),
            None => (&gpu.invalid_view, &gpu.invalid),
        };
        // A pending copy pipeline or incompatible/missing camera produces an
        // explicitly invalid binding, never a valid flag over last-frame depth.
        let group = device.create_bind_group("weather_raw_depth_for_view",
            &cache.get_bind_group_layout(&raw_depth_layout()),
            &BindGroupEntries::sequential((view, valid.as_entire_binding())));
        let mut entity = commands.entity(entity);
        entity.insert(RawDepthBinding { group });
        if let Some(snapshot) = snapshot { entity.insert(snapshot); }
        else { entity.remove::<PreparedDepthSnapshot>(); }
    }
}

use bevy::core_pipeline::core_3d::Transparent3d;
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::render::render_phase::{DrawFunctions,PhaseItem,RenderCommand,RenderCommandResult,
    AddRenderCommand,SetItemPipeline,TrackedRenderPass,ViewSortedRenderPhases};
use bevy::pbr::{DrawMesh,SetMaterialBindGroup,SetMeshBindGroup,SetMeshViewBindGroup,RenderMaterialInstances};

type DrawWeatherParticle = (SetItemPipeline,SetMeshViewBindGroup<0>,SetRawDepth<1>,
    SetMeshBindGroup<2>,SetMaterialBindGroup<3>,DrawMesh);

struct SetRawDepth<const I:usize>;
impl<P:PhaseItem,const I:usize> RenderCommand<P> for SetRawDepth<I> {
    type Param=();
    type ViewQuery=Option<&'static RawDepthBinding>;
    type ItemQuery=();
    fn render<'w>(_item:&P,view:ROQueryItem<'w,'_,Self::ViewQuery>,_entity:Option<()>,
        _:SystemParamItem<'w,'_,Self::Param>,pass:&mut TrackedRenderPass<'w>)->RenderCommandResult {
        let Some(depth)=view else {return RenderCommandResult::Skip;};
        pass.set_bind_group(I,&depth.group,&[]);
        RenderCommandResult::Success
    }
}

fn route_raw_depth_particles(mut phases:ResMut<ViewSortedRenderPhases<Transparent3d>>,
    instances:Res<RenderMaterialInstances>,functions:Res<DrawFunctions<Transparent3d>>) {
    let draw=functions.read().id::<DrawWeatherParticle>();
    for phase in phases.0.values_mut() {
        for item in &mut phase.items {
            if instances.instances.get(&item.main_entity()).is_some_and(|instance|
                instance.asset_id.type_id()==std::any::TypeId::of::<crate::uber_particle::UberParticleMaterial>()) {
                item.draw_function=draw;
            }
        }
    }
}

pub(crate) fn install_raw_depth(app:&mut App) {
    use bevy::render::{Render,RenderApp,RenderStartup,RenderSystems};
    app.add_systems(Startup, load_depth_copy)
        .add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<WeatherDepthSnapshot>::default());
    let Some(render)=app.get_sub_app_mut(RenderApp) else {return;};
    render.add_render_command::<Transparent3d,DrawWeatherParticle>()
        .add_systems(RenderStartup,init_raw_depth)
        .add_systems(Render,prepare_raw_depth.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(Render,route_raw_depth_particles.in_set(RenderSystems::QueueMeshes)
            .after(bevy::pbr::queue_material_meshes));
}
