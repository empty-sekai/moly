//! Live source-program particle submission. Forward draws join the scene's
//! transparent phase; the independent effect program writes the bloom source.
use crate::source_particle::SourceParticle;
use crate::source_shader::*;
use bevy::core_pipeline::core_3d::{
    graph::{Core3d, Node3d},
    Transparent3d,
};
use bevy::ecs::{
    query::{QueryItem, ROQueryItem},
    system::{lifetimeless::SRes, SystemParamItem},
};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::{
    render_asset::RenderAssets,
    render_graph::{
        NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
    },
    render_phase::{
        sort_phase_system, AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex,
        RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
        ViewSortedRenderPhases,
    },
    renderer::RenderContext,
    sync_world::{MainEntity, RenderEntity},
    view::{ExtractedView, ViewDepthTexture, ViewTarget},
    Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
use moly_assets::source_shader::Result;
use moly_assets::source_shader::SourceShaderCatalogue;
use moly_assets::source_shader::{
    loader::SourceProgramAsset, state::SourcePassState, SourceShaderError, UniformValue,
};
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap};

struct ExtractedParticle {
    entity: Entity,
    main: MainEntity,
    source: SourceParticle,
    mesh: Mesh,
    center: Vec3,
}
#[derive(Resource, Default)]
struct ParticleFrame {
    particles: Vec<ExtractedParticle>,
    light: [f32; 4],
    catalogues: HashMap<AssetId<SourceShaderCatalogue>, SourceShaderCatalogue>,
    cameras: HashMap<Entity, Projection>,
}
fn extract(
    mut frame: ResMut<ParticleFrame>,
    particles: Extract<Query<(Entity, &RenderEntity, &Mesh3d, &SourceParticle)>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    catalogues: Extract<Res<Assets<SourceShaderCatalogue>>>,
    env: Extract<Res<crate::env::SiteEnv>>,
    cameras: Extract<Query<(&RenderEntity, &Projection), With<Camera3d>>>,
) {
    frame.particles.clear();
    frame.catalogues.clear();
    frame.cameras = cameras
        .iter()
        .map(|(entity, projection)| (entity.id(), projection.clone()))
        .collect();
    frame.light = env.globals.phenomena_directional_light_color;
    for (main, entity, mesh, source) in &particles {
        if source.error.is_some() || source.passes.is_empty() {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh.0) else {
            continue;
        };
        let Some(catalogue) = catalogues.get(&source.catalogue) else {
            continue;
        };
        frame
            .catalogues
            .entry(source.catalogue.id())
            .or_insert_with(|| catalogue.clone());
        let center = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(positions))
                if !positions.is_empty() =>
            {
                let (mut min, mut max) =
                    (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for p in positions {
                    min = min.min(Vec3::from_array(*p));
                    max = max.max(Vec3::from_array(*p));
                }
                (min + max) * 0.5
            }
            _ => Vec3::ZERO,
        };
        frame.particles.push(ExtractedParticle {
            entity: entity.id(),
            main: main.into(),
            source: source.clone(),
            mesh: mesh.clone(),
            center,
        });
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineKey {
    program: AssetId<SourceProgramAsset>,
    state: SourcePassState,
    color: TextureFormat,
    depth: TextureFormat,
    samples: u32,
    layout: VertexBufferLayout,
}
struct Packet {
    pipeline: CachedRenderPipelineId,
    binding: SourceGpuBinding,
    vertex: Buffer,
    index: Buffer,
    count: u32,
    distance: f32,
    sorting_order: i32,
    render_queue: i32,
    // View targets may be recycled or resized independently of material assets.
    resources: Vec<String>,
}
#[derive(Resource, Default)]
struct ParticleGpu {
    pipelines: HashMap<PipelineKey, CachedRenderPipelineId>,
    packets: HashMap<(Entity, Entity, bool), Packet>,
    queued: HashMap<(Entity, Entity, bool), CachedRenderPipelineId>,
    errors: BTreeSet<String>,
    depth_sampler: Option<Sampler>,
}
fn fail(gpu: &mut ParticleGpu, error: SourceShaderError) {
    if gpu.errors.insert(error.to_string()) {
        error!(%error, "source particle draw unresolved");
    }
}
fn fail_particle(gpu: &mut ParticleGpu, source: &SourceParticle, error: SourceShaderError) {
    *source.readiness.lock().unwrap() =
        crate::source_particle::ParticleReadiness::Failed(error.to_string());
    fail(gpu, error);
}

fn queue(
    frame: Res<ParticleFrame>,
    programs: Res<RenderAssets<GpuSourceProgram>>,
    cache: Res<PipelineCache>,
    mut gpu: ResMut<ParticleGpu>,
    views: Query<(
        Entity,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
        &crate::weather_depth::WeatherCameraRole,
    )>,
    functions: Res<DrawFunctions<Transparent3d>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    gpu.queued.clear();
    for item in &frame.particles {
        let mut readiness = item.source.readiness.lock().unwrap();
        if !matches!(
            *readiness,
            crate::source_particle::ParticleReadiness::Failed(_)
        ) {
            *readiness = crate::source_particle::ParticleReadiness::Pending;
        }
    }
    let function = functions.read().id::<DrawSourceParticle>();
    for (view_entity, view, target, depth, role) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        for item in &frame.particles {
            for pass in &item.source.passes {
                if pass.effect && *role == crate::weather_depth::WeatherCameraRole::Overlay {
                    continue;
                }
                let Some(program) = programs.get(pass.program.id()) else {
                    continue;
                };
                let layout = match item.source.streams.layout(&program.receipt.abi) {
                    Ok(layout) => layout,
                    Err(error) => {
                        fail_particle(&mut gpu, &item.source, error);
                        continue;
                    }
                };
                let key = PipelineKey {
                    program: pass.program.id(),
                    state: pass.state,
                    color: if pass.effect {
                        crate::fixture_emission::EMISSION_FORMAT
                    } else {
                        crate::source_color::ENCODED_FORMAT
                    },
                    depth: depth.texture.format(),
                    samples: depth.texture.sample_count(),
                    layout,
                };
                if !crate::source_color::compatible(target, depth) {
                    fail_particle(&mut gpu, &item.source, SourceShaderError(
                        "source Gamma draws require matching single-sample sRGB scene/depth targets".into(),
                    ));
                    continue;
                }
                if pass.effect && key.samples != 1 {
                    fail_particle(
                        &mut gpu,
                        &item.source,
                        SourceShaderError("effect target requires a single-sample camera".into()),
                    );
                    continue;
                }
                let pipeline = if let Some(pipeline) = gpu.pipelines.get(&key) {
                    *pipeline
                } else {
                    let descriptor = pipeline_descriptor(
                        program,
                        pass.state,
                        key.layout.clone(),
                        SourcePipelineTarget {
                            color: key.color,
                            depth: key.depth,
                            samples: key.samples,
                            front_face: FrontFace::Ccw,
                        },
                    )
                    .and_then(|mut descriptor| {
                        let unfilterable = program
                            .receipt
                            .abi
                            .textures
                            .iter()
                            .filter(|t| t.name == "_CameraDepthTexture")
                            .map(|t| t.name.clone())
                            .collect();
                        descriptor.layout = vec![resource_binding_layout(
                            &program.receipt.abi,
                            &unfilterable,
                        )?];
                        Ok(descriptor)
                    });
                    let descriptor = match descriptor {
                        Ok(value) => value,
                        Err(error) => {
                            fail_particle(&mut gpu, &item.source, error);
                            continue;
                        }
                    };
                    let pipeline = cache.queue_render_pipeline(descriptor);
                    gpu.pipelines.insert(key, pipeline);
                    pipeline
                };
                gpu.queued
                    .insert((view_entity, item.entity, pass.effect), pipeline);
                if !pass.effect && item.source.enabled {
                    phase.add(Transparent3d {
                        entity: (item.entity, item.main),
                        draw_function: function,
                        pipeline,
                        distance: view.rangefinder3d().distance(&item.center)
                            - item.source.sorting_fudge,
                        batch_range: 0..1,
                        extra_index: PhaseItemExtraIndex::None,
                        indexed: true,
                    });
                }
            }
        }
    }
}

fn sort_source_particles(
    frame: Res<ParticleFrame>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    let order = |entity| {
        frame
            .particles
            .iter()
            .find(|p| p.entity == entity)
            .map(|p| (p.source.sorting_order, p.source.render_queue))
            .unwrap_or((0, 3000))
    };
    for phase in phases.values_mut() {
        phase.items.sort_by(|a, b| {
            order(a.entity())
                .cmp(&order(b.entity()))
                .then_with(|| a.distance.total_cmp(&b.distance))
        });
    }
}

// Run before the render view uniforms are built, so every opaque/transparent
// writer and the sampled depth agree on the source camera's finite far plane.
fn prepare_source_cameras(
    frame: Res<ParticleFrame>,
    mut views: Query<(Entity, &mut ExtractedView), With<crate::weather_depth::WeatherCameraRole>>,
) {
    for (entity, mut view) in &mut views {
        let Some(camera) = frame.cameras.get(&entity) else {
            continue;
        };
        match crate::source_camera::render_projection(view.clip_from_view, camera) {
            Ok(projection) => {
                if let Some(clip) = view.clip_from_world {
                    let view_from_world = view.clip_from_view.inverse() * clip;
                    view.clip_from_world = Some(projection * view_from_world);
                }
                view.clip_from_view = projection;
            }
            Err(error) => error!(%error, "source camera projection unresolved"),
        }
    }
}

fn globals(
    name: &str,
    view: &ExtractedView,
    camera: &Projection,
    light: [f32; 4],
) -> Result<UniformValue> {
    let world_from_view = view.world_from_view.to_matrix();
    let source_to_render = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    let view_from_world = world_from_view.inverse() * source_to_render;
    // Invert only the qualified output adapter: source GLES [-w,+w] forward Z
    // becomes renderer [w,0] reversed Z. Matrix arithmetic remains in source VS.
    let projection = crate::source_camera::gles_projection(view.clip_from_view);
    // Validate the authored planes even when this particular shader only uses VP.
    let source_z = crate::source_camera::gles_z_buffer_params(camera)?;
    let (near, far, ortho) = match camera {
        Projection::Perspective(p) => (p.near, p.far, false),
        Projection::Orthographic(p) => (p.near, p.far, true),
        _ => {
            return Err(SourceShaderError(
                "custom camera projection needs explicit source parameter ownership".into(),
            ));
        }
    };
    let raw = view.clip_from_view;
    let value = match name {
        "hlslcc_mtx4x4unity_ObjectToWorld" | "hlslcc_mtx4x4unity_WorldToObject" => {
            Mat4::IDENTITY.to_cols_array().to_vec()
        }
        "hlslcc_mtx4x4unity_MatrixVP" => (projection * view_from_world).to_cols_array().to_vec(),
        "hlslcc_mtx4x4unity_MatrixV" => view_from_world.to_cols_array().to_vec(),
        "_GlobalMipBias" => vec![0.0, 0.0],
        "_GlobalPhenomenaDirectionalLightColor" => light.to_vec(),
        "_WorldSpaceCameraPos" => source_to_render
            .transform_point3(world_from_view.w_axis.truncate())
            .to_array()
            .to_vec(),
        "_ProjectionParams" => vec![1.0, near, far, far.recip()],
        "_ZBufferParams" => source_z.to_vec(),
        "unity_OrthoParams" => vec![
            2.0 / raw.x_axis.x,
            2.0 / raw.y_axis.y,
            0.0,
            f32::from(ortho),
        ],
        _ => {
            return Err(SourceShaderError(format!(
                "camera/global writer unresolved: {name}"
            )));
        }
    };
    Ok(UniformValue::Float(value))
}

fn prepare(
    frame: Res<ParticleFrame>,
    programs: Res<RenderAssets<GpuSourceProgram>>,
    textures: Res<RenderAssets<PreparedSourceTexture>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<ParticleGpu>,
    views: Query<(
        Entity,
        &ExtractedView,
        Option<&crate::weather_depth::PreparedDepthSnapshot>,
        Option<&crate::source_color::SourceColorView>,
    )>,
) {
    if gpu.depth_sampler.is_none() {
        gpu.depth_sampler = Some(device.create_sampler(&SamplerDescriptor {
            label: Some("opaque depth point-clamp sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            ..default()
        }));
    }
    let depth_sampler = gpu.depth_sampler.as_ref().unwrap().clone();
    let mut live = std::collections::HashSet::new();
    for (view_entity, view, depth, _) in &views {
        let Some(camera) = frame.cameras.get(&view_entity) else {
            continue;
        };
        for item in &frame.particles {
            let catalogue = &frame.catalogues[&item.source.catalogue.id()];
            for pass in &item.source.passes {
                let key = (view_entity, item.entity, pass.effect);
                let Some(&pipeline) = gpu.queued.get(&key) else {
                    continue;
                };
                let Some(program) = programs.get(pass.program.id()) else {
                    continue;
                };
                let mut updated = false;
                let result = (|| -> Result<Option<Packet>> {
                    let abi = &program.receipt.abi;
                    let (_, variant) = catalogue.select(
                        program.receipt.source.reference.subshader,
                        program.receipt.source.reference.pass,
                        9,
                        4,
                        &item.source.material.keywords,
                    )?;
                    program.receipt.matches(catalogue, variant)?;
                    let values = abi
                        .uniforms
                        .iter()
                        .map(|field| {
                            Ok((
                                field.name.clone(),
                                match item.source.material.uniform(catalogue, field)? {
                                    Some(value) => value,
                                    None => globals(&field.name, view, camera, frame.light)?,
                                },
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>>>()?;
                    let mut bindings = BTreeMap::new();
                    let mut resources = Vec::new();
                    for declaration in &abi.textures {
                        if declaration.name == "_CameraDepthTexture" {
                            let Some(depth) = depth else {
                                return Ok(None);
                            };
                            resources.push(format!("{:?}", depth.source_view().id()));
                            bindings.insert(
                                declaration.name.clone(),
                                SourceSampledResource::View {
                                    view: depth.source_view(),
                                    sampler: &depth_sampler,
                                    dimension: TextureViewDimension::D2,
                                    filterable: false,
                                },
                            );
                        } else {
                            let handle =
                                item.source.textures.get(&declaration.name).ok_or_else(|| {
                                    SourceShaderError(format!(
                                        "unowned texture {}",
                                        declaration.name
                                    ))
                                })?;
                            let Some(texture) = textures.get(handle.id()) else {
                                return Ok(None);
                            };
                            let texture = texture.result.as_ref().map_err(Clone::clone)?;
                            resources.push(format!("{:?}", texture.raw.id()));
                            bindings.insert(
                                declaration.name.clone(),
                                SourceSampledResource::Asset(SourceSampledTexture {
                                    image: texture,
                                    encoding: SourceTextureEncoding::Raw,
                                }),
                            );
                        }
                    }
                    let (vertices, indices, count) = item.source.streams.pack(abi, &item.mesh)?;
                    let buffer = |label, bytes: &[u8], usage| {
                        let buffer = device.create_buffer(&BufferDescriptor {
                            label: Some(label),
                            size: bytes.len().max(4).next_power_of_two() as u64,
                            usage: usage | BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        if !bytes.is_empty() {
                            queue.write_buffer(&buffer, 0, bytes);
                        }
                        buffer
                    };
                    if let Some(packet) = gpu
                        .packets
                        .get_mut(&key)
                        .filter(|p| p.pipeline == pipeline && p.resources == resources)
                    {
                        packet.binding.write_uniforms(&queue, abi, &values)?;
                        if packet.vertex.size() < vertices.len() as u64 {
                            packet.vertex =
                                buffer("source particle vertices", &vertices, BufferUsages::VERTEX);
                        } else if !vertices.is_empty() {
                            queue.write_buffer(&packet.vertex, 0, &vertices);
                        }
                        if packet.index.size() < indices.len() as u64 {
                            packet.index =
                                buffer("source particle indices", &indices, BufferUsages::INDEX);
                        } else if !indices.is_empty() {
                            queue.write_buffer(&packet.index, 0, &indices);
                        }
                        packet.count = count;
                        packet.distance =
                            view.rangefinder3d().distance(&item.center) - item.source.sorting_fudge;
                        packet.sorting_order = item.source.sorting_order;
                        packet.render_queue = item.source.render_queue;
                        updated = true;
                        return Ok(None);
                    }
                    let binding = SourceGpuBinding::create_resources(
                        &device,
                        &cache,
                        &program.receipt,
                        &values,
                        &bindings,
                    )?;
                    Ok(Some(Packet {
                        pipeline,
                        binding,
                        vertex: buffer("source particle vertices", &vertices, BufferUsages::VERTEX),
                        index: buffer("source particle indices", &indices, BufferUsages::INDEX),
                        count,
                        resources,
                        distance: view.rangefinder3d().distance(&item.center)
                            - item.source.sorting_fudge,
                        sorting_order: item.source.sorting_order,
                        render_queue: item.source.render_queue,
                    }))
                })();
                match result {
                    Ok(Some(packet)) => {
                        gpu.packets.insert(key, packet);
                        live.insert(key);
                    }
                    Ok(None) => {
                        if updated {
                            live.insert(key);
                        } else {
                            gpu.packets.remove(&key);
                        }
                    }
                    Err(error) => {
                        *item.source.readiness.lock().unwrap() =
                            crate::source_particle::ParticleReadiness::Failed(error.to_string());
                        gpu.packets.remove(&key);
                        fail(&mut gpu, error);
                    }
                }
            }
        }
    }
    gpu.packets.retain(|key, _| live.contains(key));
    for (view_entity, _, _, color) in &views {
        for item in &frame.particles {
            let mut readiness = item.source.readiness.lock().unwrap();
            if matches!(
                *readiness,
                crate::source_particle::ParticleReadiness::Failed(_)
            ) {
                continue;
            }
            if let Some(error) = item.source.passes.iter().find_map(|pass| {
                gpu.queued
                    .get(&(view_entity, item.entity, pass.effect))
                    .and_then(
                        |pipeline| match cache.get_render_pipeline_state(*pipeline) {
                            CachedPipelineState::Err(error) => {
                                Some(format!("source pipeline compilation failed: {error:?}"))
                            }
                            _ => None,
                        },
                    )
            }) {
                *readiness = crate::source_particle::ParticleReadiness::Failed(error);
                continue;
            }
            match color.map(|color| color.ready(&cache)) {
                Some(Ok(true)) => {}
                Some(Err(error)) => {
                    *readiness = crate::source_particle::ParticleReadiness::Failed(error);
                    continue;
                }
                _ => continue,
            }
            if item.source.passes.iter().all(|p| {
                gpu.packets
                    .get(&(view_entity, item.entity, p.effect))
                    .is_some_and(|packet| cache.get_render_pipeline(packet.pipeline).is_some())
            }) {
                *readiness = crate::source_particle::ParticleReadiness::Ready;
            }
        }
    }
}

type DrawSourceParticle = (SetItemPipeline, DrawPacket);
struct DrawPacket;
impl RenderCommand<Transparent3d> for DrawPacket {
    type Param = SRes<ParticleGpu>;
    type ViewQuery = Entity;
    type ItemQuery = ();
    fn render<'w>(
        item: &Transparent3d,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<()>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(packet) = gpu
            .into_inner()
            .packets
            .get(&(view, item.entity(), false))
            .filter(|p| p.pipeline == item.pipeline && p.count > 0)
        else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(0, &packet.binding.group, &[]);
        pass.set_vertex_buffer(0, packet.vertex.slice(..));
        pass.set_index_buffer(packet.index.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..packet.count, 0, 0..1);
        RenderCommandResult::Success
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, RenderLabel)]
struct SourceEffect;
#[derive(Default)]
struct EffectNode;
impl ViewNode for EffectNode {
    type ViewQuery = (
        Entity,
        &'static crate::fixture_emission::ViewEmissionTarget,
        &'static ViewDepthTexture,
        &'static ExtractedView,
        &'static crate::weather_depth::WeatherCameraRole,
    );
    fn run(
        &self,
        _: &mut RenderGraphContext,
        context: &mut RenderContext,
        (entity, target, depth, view, role): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> std::result::Result<(), NodeRunError> {
        if !crate::weather_depth::effect_attachment_compatible(
            Some(*role),
            depth.texture.sample_count(),
        ) {
            return Ok(());
        }
        let gpu = world.resource::<ParticleGpu>();
        let cache = world.resource::<PipelineCache>();
        let frame = world.resource::<ParticleFrame>();
        let mut packets = gpu
            .packets
            .iter()
            .filter(|((view, draw, effect), _)| {
                *view == entity
                    && *effect
                    && frame
                        .particles
                        .iter()
                        .any(|p| p.entity == *draw && p.source.enabled)
            })
            .map(|(_, p)| p)
            .collect::<Vec<_>>();
        packets.sort_by(|a, b| {
            a.sorting_order
                .cmp(&b.sorting_order)
                .then_with(|| a.render_queue.cmp(&b.render_queue))
                .then_with(|| a.distance.total_cmp(&b.distance))
        });
        let attachments = [Some(RenderPassColorAttachment {
            view: &target.resolved,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            depth_slice: None,
        })];
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("source particle effect programs"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: depth.view(),
                    depth_ops: Some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        pass.set_viewport(
            view.viewport.x as f32,
            view.viewport.y as f32,
            view.viewport.z as f32,
            view.viewport.w as f32,
            0.0,
            1.0,
        );
        for packet in packets {
            if packet.count == 0 {
                continue;
            }
            let Some(pipeline) = cache.get_render_pipeline(packet.pipeline) else {
                continue;
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &packet.binding.group, &[]);
            pass.set_vertex_buffer(0, *packet.vertex.slice(..));
            pass.set_index_buffer(*packet.index.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..packet.count, 0, 0..1);
        }
        Ok(())
    }
}

pub struct SourceParticlePlugin;
impl Plugin for SourceParticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            SourceParticle,
        >::default())
            .add_systems(
                Update,
                (
                    crate::source_particle::resolve_particles,
                    crate::weather_fx::cleanup_preflight,
                ),
            );
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<ParticleFrame>()
            .init_resource::<ParticleGpu>()
            .add_render_command::<Transparent3d, DrawSourceParticle>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(
                Render,
                prepare_source_cameras.in_set(RenderSystems::ManageViews),
            )
            .add_systems(Render, queue.in_set(RenderSystems::QueueMeshes))
            .add_systems(
                Render,
                sort_source_particles
                    .in_set(RenderSystems::PhaseSort)
                    .after(sort_phase_system::<Transparent3d>),
            )
            .add_systems(
                Render,
                prepare
                    .in_set(RenderSystems::PrepareBindGroups)
                    .after(crate::weather_depth::prepare_raw_depth),
            )
            .add_render_graph_node::<ViewNodeRunner<EffectNode>>(Core3d, SourceEffect)
            .add_render_graph_edges(
                Core3d,
                (
                    crate::fixture_emission::EmissionPassLabel,
                    SourceEffect,
                    Node3d::MainTransmissivePass,
                ),
            );
    }
}
