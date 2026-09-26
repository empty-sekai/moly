//! Live source-program particle submission. Forward draws join the scene's
//! transparent phase; the independent effect program writes the bloom source.
use crate::source_particle::SourceParticle;
use crate::source_shader::*;
use bevy::core_pipeline::core_3d::{
    graph::{Core3d, Node3d},
    Transparent3d, CORE_3D_DEPTH_FORMAT,
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
    view::{ExtractedView, Msaa, ViewDepthTexture, ViewTarget},
    Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
use moly_assets::source_shader::Result;
use moly_assets::source_shader::{material::MaterialSnapshot, ProgramReceipt, SourceShaderCatalogue};
use moly_assets::source_shader::{
    loader::SourceProgramAsset, state::SourcePassState, SourceShaderError, UniformValue,
};
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

struct ExtractedParticle {
    entity: Entity,
    main: MainEntity,
    source: SourceParticle,
    mesh: Mesh,
    center: Vec3,
    /// 1 for a draw that follows another draw of the same renderer, else 0.
    rank: u8,
}
#[derive(Resource, Default)]
struct ParticleFrame {
    particles: Vec<ExtractedParticle>,
    /// Render entities of this frame's enabled particles.
    enabled: HashSet<Entity>,
    /// (sorting order, render queue) of this frame's particles.
    order: HashMap<Entity, (i32, i32)>,
    /// Rank of each render entity among the draws of its renderer (the trail
    /// draw after the particle draw at the same distance).
    rank: HashMap<Entity, u8>,
    light: [f32; 4],
    /// Copies of the catalogues this frame's particles reference. A catalogue
    /// is immutable once loaded; its copy is replaced only after an asset event.
    catalogues: HashMap<AssetId<SourceShaderCatalogue>, SourceShaderCatalogue>,
    /// Advances with every catalogue asset event, invalidating pass resolutions.
    catalogue_generation: u64,
    cameras: HashMap<Entity, Projection>,
}
fn extract(
    mut frame: ResMut<ParticleFrame>,
    particles: Extract<Query<(Entity, &RenderEntity, &Mesh3d, &SourceParticle)>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    catalogues: Extract<Res<Assets<SourceShaderCatalogue>>>,
    mut catalogue_events: Extract<MessageReader<AssetEvent<SourceShaderCatalogue>>>,
    env: Extract<Res<crate::env::SiteEnv>>,
    cameras: Extract<Query<(&RenderEntity, &Projection), With<Camera3d>>>,
) {
    let frame = &mut *frame;
    for event in catalogue_events.read() {
        match event {
            AssetEvent::Modified { id } | AssetEvent::Removed { id } | AssetEvent::Unused { id } => {
                frame.catalogues.remove(id);
            }
            _ => {}
        }
        frame.catalogue_generation += 1;
    }
    frame.cameras.clear();
    frame.cameras.extend(
        cameras
            .iter()
            .map(|(entity, projection)| (entity.id(), projection.clone())),
    );
    frame.light = env.globals.phenomena_directional_light_color;
    let mut previous: HashMap<MainEntity, SourceParticle> = frame
        .particles
        .drain(..)
        .map(|particle| (particle.main, particle.source))
        .collect();
    frame.enabled.clear();
    frame.order.clear();
    frame.rank.clear();
    let mut referenced = HashSet::new();
    for (main, entity, mesh, component) in &particles {
        if component.error.is_some() || component.passes.is_empty() {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh.0) else {
            continue;
        };
        let Some(catalogue) = catalogues.get(&component.catalogue) else {
            continue;
        };
        referenced.insert(component.catalogue.id());
        frame
            .catalogues
            .entry(component.catalogue.id())
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
        let main: MainEntity = main.into();
        // Main-world systems take the component mutably every frame, so change
        // ticks do not say whether it changed; compare what drawing reads.
        let source = match previous.remove(&main) {
            Some(kept) if same_draw(&kept, &component) => kept,
            _ => SourceParticle::clone(&component),
        };
        let entity = entity.id();
        if source.enabled {
            frame.enabled.insert(entity);
        }
        frame
            .order
            .entry(entity)
            .or_insert((source.sorting_order, source.render_queue));
        let rank = u8::from(source.follows.is_some());
        frame.rank.insert(entity, rank);
        frame.particles.push(ExtractedParticle {
            entity,
            main,
            source,
            mesh: mesh.clone(),
            center,
            rank,
        });
    }
    // A follower sorts at its leader's distance: the renderer is one object
    // with one sort position, whatever each material's geometry spans.
    let centers: HashMap<Entity, Vec3> = frame.particles.iter()
        .map(|particle| (particle.main.id(), particle.center)).collect();
    for particle in &mut frame.particles {
        if let Some(center) = particle.source.follows.and_then(|leader| centers.get(&leader)) {
            particle.center = *center;
        }
    }
    frame.catalogues.retain(|id, _| referenced.contains(id));
}

/// Whether a kept render-world copy draws exactly as a fresh clone would:
/// every field the renderer reads, by value or by asset identity. Both copies
/// share the readiness cell, so a kept copy still publishes to the component.
fn same_draw(kept: &SourceParticle, current: &SourceParticle) -> bool {
    Arc::ptr_eq(&kept.material, &current.material)
        && Arc::ptr_eq(&kept.readiness, &current.readiness)
        && kept.catalogue.id() == current.catalogue.id()
        && kept.streams == current.streams
        && kept.enabled == current.enabled
        && kept.error == current.error
        && kept.render_queue == current.render_queue
        && kept.sorting_order == current.sorting_order
        && kept.sorting_fudge.to_bits() == current.sorting_fudge.to_bits()
        && kept.follows == current.follows
        && kept.passes.len() == current.passes.len()
        && kept.passes.iter().zip(&current.passes).all(|(a, b)| {
            a.program.id() == b.program.id() && a.state == b.state && a.effect == b.effect
        })
        && kept.textures.len() == current.textures.len()
        && kept
            .textures
            .iter()
            .zip(&current.textures)
            .all(|((a_name, a), (b_name, b))| a_name == b_name && a.id() == b.id())
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
    rank: u8,
    // View targets may be recycled or resized independently of material assets.
    resources: Vec<TextureViewId>,
}
#[derive(Resource, Default)]
struct ParticleGpu {
    pipelines: HashMap<PipelineKey, CachedRenderPipelineId>,
    packets: HashMap<(Entity, Entity, bool), Packet>,
    queued: HashMap<(Entity, Entity, bool), CachedRenderPipelineId>,
    resolutions: HashMap<(Entity, AssetId<SourceProgramAsset>), PassResolution>,
    errors: BTreeSet<String>,
    depth_sampler: Option<Sampler>,
}

/// Where one ABI uniform gets its value.
enum FieldSource {
    Material(UniformValue),
    Failed(SourceShaderError),
    Camera,
}

/// The material-constant part of a pass: variant selection, the compiled
/// receipt check and material-owned uniforms. These are functions of the
/// material, the catalogue and the receipt only, so they are resolved again
/// only when one of those changes. Camera/global writers stay per frame.
struct PassResolution {
    material: Arc<MaterialSnapshot>,
    catalogue: AssetId<SourceShaderCatalogue>,
    catalogue_generation: u64,
    receipt: Arc<ProgramReceipt>,
    fields: Result<Vec<FieldSource>>,
}
impl PassResolution {
    fn resolve(
        source: &SourceParticle,
        catalogue: &SourceShaderCatalogue,
        catalogue_generation: u64,
        program: &GpuSourceProgram,
    ) -> Self {
        let fields = (|| {
            let (_, variant) = catalogue.select(
                program.receipt.source.reference.subshader,
                program.receipt.source.reference.pass,
                9,
                4,
                &source.material.keywords,
            )?;
            program.receipt.matches(catalogue, variant)?;
            Ok(program
                .receipt
                .abi
                .uniforms
                .iter()
                .map(|field| match source.material.uniform(catalogue, field) {
                    Ok(Some(value)) => FieldSource::Material(value),
                    Ok(None) => FieldSource::Camera,
                    Err(error) => FieldSource::Failed(error),
                })
                .collect())
        })();
        Self {
            material: source.material.clone(),
            catalogue: source.catalogue.id(),
            catalogue_generation,
            receipt: program.receipt.clone(),
            fields,
        }
    }

    fn current(
        &self,
        source: &SourceParticle,
        catalogue_generation: u64,
        program: &GpuSourceProgram,
    ) -> bool {
        Arc::ptr_eq(&self.material, &source.material)
            && self.catalogue == source.catalogue.id()
            && self.catalogue_generation == catalogue_generation
            && Arc::ptr_eq(&self.receipt, &program.receipt)
    }
}

/// A texture a pass samples, resolved for one view.
enum Sampled<'a> {
    Depth(&'a TextureView),
    Asset(&'a GpuSourceTexture),
}
fn attachment_mismatch() -> SourceShaderError {
    SourceShaderError(
        "source Gamma draws require matching single-sample sRGB scene/depth targets".into(),
    )
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
    // No ViewDepthTexture here: this frame's depth texture is created in
    // PrepareResources, after this queue. The component still present is the
    // previous frame's, whose size differs on the frame a view is resized.
    // The pipeline is keyed on what this frame's depth texture is created
    // from (the view's Msaa and the core depth format); `prepare` checks the
    // actual attachments once they exist. The source renderer reallocates the
    // camera colour and depth attachments for a new size in its setup, before
    // any pass of that frame (UniversalRenderer.Setup, CreateCameraRenderTarget,
    // RenderingUtils.ReAllocateIfNeeded), so a resized frame draws its
    // particles like any other.
    views: Query<(
        Entity,
        &ExtractedView,
        &ViewTarget,
        &Msaa,
        &crate::weather_depth::WeatherCameraRole,
    )>,
    functions: Res<DrawFunctions<Transparent3d>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    gpu.queued.clear();
    // Readiness is not reset here. Frame order under pipelined rendering:
    // after extracting frame N the main app runs update N+1 while the render
    // app runs frame N (QueueMeshes: this queue; PrepareBindGroups: `prepare`
    // below; then the graph). The main-world preparations read the cell
    // during update N+1, so a reset here and a set at the end of `prepare`
    // would open a Pending window inside every frame, and a reader whose
    // sampling instant keeps falling inside it reads Pending frame after
    // frame while every frame ends Ready. The cell is written once per
    // frame, at the end of `prepare`, with that frame's verdict.
    let function = functions.read().id::<DrawSourceParticle>();
    for (view_entity, view, target, msaa, role) in &views {
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
                    depth: CORE_3D_DEPTH_FORMAT,
                    samples: msaa.samples(),
                    layout,
                };
                if !crate::source_color::srgb_single_sample(target, key.samples) {
                    fail_particle(&mut gpu, &item.source, attachment_mismatch());
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
    let order = |entity| frame.order.get(&entity).copied().unwrap_or((0, 3000));
    let rank = |entity| frame.rank.get(&entity).copied().unwrap_or(0);
    for phase in phases.values_mut() {
        phase.items.sort_by(|a, b| {
            order(a.entity())
                .cmp(&order(b.entity()))
                .then_with(|| a.distance.total_cmp(&b.distance))
                .then_with(|| rank(a.entity()).cmp(&rank(b.entity())))
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
        // URP ScriptableRenderer.SetPerCameraShaderVariables writes (b, 2^b), where
        // b = min(-log2(camera width / scaled width), 0) and the temporal mip bias;
        // without render scaling or temporal AA, b is 0.
        "_GlobalMipBias" => vec![0.0, 1.0],
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
        &ViewTarget,
        &ViewDepthTexture,
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
    let gpu = &mut *gpu;
    let drawn: HashSet<Entity> = frame.particles.iter().map(|p| p.entity).collect();
    gpu.resolutions
        .retain(|(entity, _), _| drawn.contains(entity));
    // Packing depends on the program ABI and the geometry, not on the view.
    let mut packed: HashMap<(Entity, AssetId<SourceProgramAsset>), Result<(Vec<u8>, Vec<u8>, u32)>> =
        HashMap::new();
    let mut live = std::collections::HashSet::new();
    for (view_entity, view, target, attachment, depth, _) in &views {
        let Some(camera) = frame.cameras.get(&view_entity) else {
            continue;
        };
        // This frame's attachments, which the queued pipeline was keyed for.
        let attached = crate::source_color::compatible(target, attachment)
            && attachment.texture.format() == CORE_3D_DEPTH_FORMAT;
        for item in &frame.particles {
            let catalogue = &frame.catalogues[&item.source.catalogue.id()];
            for pass in &item.source.passes {
                let key = (view_entity, item.entity, pass.effect);
                let Some(&pipeline) = gpu.queued.get(&key) else {
                    continue;
                };
                if !attached {
                    gpu.packets.remove(&key);
                    fail_particle(gpu, &item.source, attachment_mismatch());
                    continue;
                }
                let Some(program) = programs.get(pass.program.id()) else {
                    continue;
                };
                let resolution = gpu
                    .resolutions
                    .entry((item.entity, pass.program.id()))
                    .or_insert_with(|| {
                        PassResolution::resolve(
                            &item.source,
                            catalogue,
                            frame.catalogue_generation,
                            program,
                        )
                    });
                if !resolution.current(&item.source, frame.catalogue_generation, program) {
                    *resolution = PassResolution::resolve(
                        &item.source,
                        catalogue,
                        frame.catalogue_generation,
                        program,
                    );
                }
                let mut updated = false;
                let result = (|| -> Result<Option<Packet>> {
                    let abi = &program.receipt.abi;
                    let fields = resolution.fields.as_ref().map_err(Clone::clone)?;
                    let values = abi
                        .uniforms
                        .iter()
                        .zip(fields)
                        .map(|(field, source)| {
                            Ok((
                                field.name.clone(),
                                match source {
                                    FieldSource::Material(value) => value.clone(),
                                    FieldSource::Failed(error) => return Err(error.clone()),
                                    FieldSource::Camera => {
                                        globals(&field.name, view, camera, frame.light)?
                                    }
                                },
                            ))
                        })
                        .collect::<Result<BTreeMap<_, _>>>()?;
                    let mut sampled = Vec::with_capacity(abi.textures.len());
                    for declaration in &abi.textures {
                        if declaration.name == "_CameraDepthTexture" {
                            let Some(depth) = depth else {
                                return Ok(None);
                            };
                            sampled.push(Sampled::Depth(depth.source_view()));
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
                            sampled.push(Sampled::Asset(texture));
                        }
                    }
                    let resources: Vec<TextureViewId> = sampled
                        .iter()
                        .map(|resource| match resource {
                            Sampled::Depth(depth_view) => depth_view.id(),
                            Sampled::Asset(texture) => texture.raw.id(),
                        })
                        .collect();
                    let (vertices, indices, count) = packed
                        .entry((item.entity, pass.program.id()))
                        .or_insert_with(|| item.source.streams.pack(abi, &item.mesh))
                        .as_ref()
                        .map_err(Clone::clone)?;
                    let count = *count;
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
                                buffer("source particle vertices", vertices, BufferUsages::VERTEX);
                        } else if !vertices.is_empty() {
                            queue.write_buffer(&packet.vertex, 0, vertices);
                        }
                        if packet.index.size() < indices.len() as u64 {
                            packet.index =
                                buffer("source particle indices", indices, BufferUsages::INDEX);
                        } else if !indices.is_empty() {
                            queue.write_buffer(&packet.index, 0, indices);
                        }
                        packet.count = count;
                        packet.distance =
                            view.rangefinder3d().distance(&item.center) - item.source.sorting_fudge;
                        packet.sorting_order = item.source.sorting_order;
                        packet.render_queue = item.source.render_queue;
                        packet.rank = item.rank;
                        updated = true;
                        return Ok(None);
                    }
                    let bindings: BTreeMap<String, SourceSampledResource> = abi
                        .textures
                        .iter()
                        .zip(&sampled)
                        .map(|(declaration, resource)| {
                            (
                                declaration.name.clone(),
                                match resource {
                                    Sampled::Depth(depth_view) => SourceSampledResource::View {
                                        view: *depth_view,
                                        sampler: &depth_sampler,
                                        dimension: TextureViewDimension::D2,
                                        filterable: false,
                                    },
                                    Sampled::Asset(texture) => {
                                        SourceSampledResource::Asset(SourceSampledTexture {
                                            image: *texture,
                                            encoding: SourceTextureEncoding::Raw,
                                        })
                                    }
                                },
                            )
                        })
                        .collect();
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
                        vertex: buffer("source particle vertices", vertices, BufferUsages::VERTEX),
                        index: buffer("source particle indices", indices, BufferUsages::INDEX),
                        count,
                        resources,
                        distance: view.rangefinder3d().distance(&item.center)
                            - item.source.sorting_fudge,
                        sorting_order: item.source.sorting_order,
                        render_queue: item.source.render_queue,
                        rank: item.rank,
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
                        fail(gpu, error);
                    }
                }
            }
        }
    }
    gpu.packets.retain(|key, _| live.contains(key));
    // This frame's verdict per readiness cell (a cell shared by several
    // draws is ready when any of them is), published below in one write.
    let cell = |source: &SourceParticle| Arc::as_ptr(&source.readiness) as usize;
    let mut ready_cells = HashSet::new();
    for (view_entity, _, _, _, _, color) in &views {
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
                ready_cells.insert(cell(&item.source));
            }
        }
    }
    for item in &frame.particles {
        let mut readiness = item.source.readiness.lock().unwrap();
        if matches!(
            *readiness,
            crate::source_particle::ParticleReadiness::Failed(_)
        ) {
            continue;
        }
        *readiness = if ready_cells.contains(&cell(&item.source)) {
            crate::source_particle::ParticleReadiness::Ready
        } else {
            crate::source_particle::ParticleReadiness::Pending
        };
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
                *view == entity && *effect && frame.enabled.contains(draw)
            })
            .map(|(_, p)| p)
            .collect::<Vec<_>>();
        packets.sort_by(|a, b| {
            a.sorting_order
                .cmp(&b.sorting_order)
                .then_with(|| a.render_queue.cmp(&b.render_queue))
                .then_with(|| a.distance.total_cmp(&b.distance))
                .then_with(|| a.rank.cmp(&b.rank))
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
