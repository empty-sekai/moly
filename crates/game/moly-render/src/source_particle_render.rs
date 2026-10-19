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
use crate::source_particle_streams::{ParticleGeometry, ParticleStreams};
use moly_assets::source_shader::Result;
use moly_assets::source_shader::{
    material::MaterialSnapshot, ProgramReceipt, SourceShaderCatalogue, TextureDefault,
};
use moly_assets::source_shader::{
    abi::UniformField, loader::SourceProgramAsset, state::SourcePassState, ProgramAbi,
    SourceShaderError, UniformFault, UniformLayout, UniformValue,
};
use std::collections::BTreeMap;
use bevy::platform::collections::{HashMap, HashSet};
use std::collections::BTreeSet;
use std::sync::Arc;

struct ExtractedParticle {
    entity: Entity,
    main: MainEntity,
    source: SourceParticle,
    /// What packing reads from the draw's mesh, kept across frames and
    /// updated in place.
    geometry: ParticleGeometry,
    /// The midpoint of `geometry`'s position bounds, recomputed when the
    /// geometry changes.
    own_center: Vec3,
    /// The sort position: `own_center`, or the leader's for a follower.
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
    /// The last generation given to a changed particle geometry.
    geometry_generation: u64,
    cameras: HashMap<Entity, Projection>,
}
fn extract(
    mut frame: ResMut<ParticleFrame>,
    particles: Extract<Query<(Entity, &RenderEntity, &Mesh3d, Ref<SourceParticle>)>>,
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
    let mut previous: HashMap<MainEntity, ExtractedParticle> = frame
        .particles
        .drain(..)
        .map(|particle| (particle.main, particle))
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
        let main: MainEntity = main.into();
        // A component not written since the last extract draws as the kept
        // copy; a written one is compared on what drawing reads, since a
        // write can leave those fields as they were. Mesh writers may write
        // without marking the asset changed, so the geometry is compared by
        // content, against the kept copy.
        let (source, mut geometry, mut own_center) = match previous.remove(&main) {
            Some(kept) => (
                if !component.is_changed() || same_draw(&kept.source, &component) {
                    kept.source
                } else {
                    SourceParticle::clone(&component)
                },
                kept.geometry,
                kept.own_center,
            ),
            None => (
                SourceParticle::clone(&component),
                ParticleGeometry::default(),
                Vec3::ZERO,
            ),
        };
        if geometry.update(mesh, &mut frame.geometry_generation) {
            own_center = geometry.center();
        }
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
            geometry,
            own_center,
            center: own_center,
            rank,
        });
    }
    // A follower sorts at its leader's distance: the renderer is one object
    // with one sort position, whatever each material's geometry spans.
    let centers: HashMap<Entity, Vec3> = frame.particles.iter()
        .map(|particle| (particle.main.id(), particle.own_center)).collect();
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
struct ParticlePipeline {
    id: CachedRenderPipelineId,
    receipt: Arc<ProgramReceipt>,
    vertex: AssetId<Shader>,
    fragment: AssetId<Shader>,
}
impl ParticlePipeline {
    fn current(&self, program: &GpuSourceProgram) -> bool {
        Arc::ptr_eq(&self.receipt, &program.receipt)
            && self.vertex == program.vertex.id()
            && self.fragment == program.fragment.id()
    }
}
struct Packet {
    pipeline: CachedRenderPipelineId,
    binding: SourceGpuBinding,
    /// The bytes of this frame's [`ParticleGpu::arena`] that hold the draw's
    /// vertices; empty while the draw is disabled.
    vertices: std::ops::Range<u64>,
    index: Buffer,
    count: u32,
    distance: f32,
    sorting_order: i32,
    render_queue: i32,
    rank: u8,
    // View targets may be recycled or resized independently of material assets.
    resources: Vec<TextureViewId>,
    /// The uniform block the binding's buffer holds.
    uniform: Vec<u8>,
    /// The [`GeometryPack::index_id`] whose bytes the index buffer holds.
    indices: u64,
}
#[derive(Resource, Default)]
struct ParticleGpu {
    pipelines: HashMap<PipelineKey, ParticlePipeline>,
    packets: HashMap<(Entity, Entity, bool), Packet>,
    queued: HashMap<(Entity, Entity, bool), CachedRenderPipelineId>,
    resolutions: HashMap<(Entity, AssetId<SourceProgramAsset>), PassResolution>,
    /// Packed geometry per draw and program, kept until its inputs change.
    packs: HashMap<(Entity, AssetId<SourceProgramAsset>), GeometryPack>,
    /// The last [`GeometryPack::id`] given out.
    pack_id: u64,
    /// The vertices of every enabled draw of this frame, uploaded in one
    /// write at the end of `prepare`.
    arena: Option<Buffer>,
    arena_bytes: Vec<u8>,
    /// The range of `arena_bytes` given to each packing this frame: the
    /// draws of one geometry in several views share one copy.
    arena_ranges: HashMap<u64, std::ops::Range<u64>>,
    errors: BTreeSet<String>,
    depth_sampler: Option<Sampler>,
    /// The built-in default textures bound to unassigned slots, each created
    /// on first use.
    defaults: HashMap<TextureDefault, Arc<FallbackTexture>>,
}

/// One draw's geometry packed for one program. Packing is a function of the
/// stream layout, the program ABI and the geometry content, so it is redone
/// only when one of those changes.
struct GeometryPack {
    /// Unique per packing: equal ids mean equal bytes.
    id: u64,
    /// Changes with a packing whose index bytes may differ from the last:
    /// equal ids mean equal index bytes.
    index_id: u64,
    geometry: u64,
    /// The geometry's index generation the index bytes were packed from;
    /// 0 after a failed packing.
    index_geometry: u64,
    receipt: Option<Arc<ProgramReceipt>>,
    streams: Option<ParticleStreams>,
    vertices: Vec<u8>,
    indices: Vec<u8>,
    /// The index count, or the packing error.
    count: Result<u32>,
}
impl Default for GeometryPack {
    fn default() -> Self {
        Self {
            id: 0,
            index_id: 0,
            geometry: 0,
            index_geometry: 0,
            receipt: None,
            streams: None,
            vertices: Vec::new(),
            indices: Vec::new(),
            count: Ok(0),
        }
    }
}
impl GeometryPack {
    /// The range of this frame's vertex arena that holds the pack's
    /// vertices, copied in on the frame's first request.
    fn arena_range(
        &self,
        bytes: &mut Vec<u8>,
        ranges: &mut HashMap<u64, std::ops::Range<u64>>,
    ) -> std::ops::Range<u64> {
        ranges
            .entry(self.id)
            .or_insert_with(|| {
                let start = bytes.len() as u64;
                bytes.extend_from_slice(&self.vertices);
                // Vertex buffer offsets and write sizes are multiples of 4.
                bytes.resize(bytes.len().next_multiple_of(4), 0);
                start..start + self.vertices.len() as u64
            })
            .clone()
    }

    fn update(
        &mut self,
        next_id: &mut u64,
        streams: &ParticleStreams,
        receipt: &Arc<ProgramReceipt>,
        geometry: &ParticleGeometry,
    ) {
        if self.id != 0
            && self.geometry == geometry.generation()
            && self.receipt.as_ref().is_some_and(|r| Arc::ptr_eq(r, receipt))
            && self.streams.as_ref() == Some(streams)
        {
            return;
        }
        self.count = streams.pack(&receipt.abi, geometry, &mut self.vertices, &mut self.indices);
        *next_id += 1;
        self.id = *next_id;
        self.geometry = geometry.generation();
        // The index bytes are a function of the indices and the vertex count
        // alone; after a failed packing they hold nothing meaningful.
        if self.count.is_err() || self.index_geometry != geometry.index_generation() {
            self.index_id = *next_id;
            self.index_geometry = if self.count.is_ok() { geometry.index_generation() } else { 0 };
        }
        self.receipt = Some(receipt.clone());
        self.streams = Some(streams.clone());
    }
}

/// Which camera/global writer supplies a uniform, resolved once from the
/// uniform's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Global {
    /// `unity_ObjectToWorld` and `unity_WorldToObject`: particle vertices are
    /// already in world space.
    Identity,
    MatrixVp,
    MatrixV,
    MipBias,
    PhenomenaLight,
    CameraPosition,
    ProjectionParams,
    ZBufferParams,
    OrthoParams,
    Unresolved,
}
impl Global {
    fn of(name: &str) -> Self {
        match name {
            "hlslcc_mtx4x4unity_ObjectToWorld" | "hlslcc_mtx4x4unity_WorldToObject" => {
                Self::Identity
            }
            "hlslcc_mtx4x4unity_MatrixVP" => Self::MatrixVp,
            "hlslcc_mtx4x4unity_MatrixV" => Self::MatrixV,
            "_GlobalMipBias" => Self::MipBias,
            "_GlobalPhenomenaDirectionalLightColor" => Self::PhenomenaLight,
            "_WorldSpaceCameraPos" => Self::CameraPosition,
            "_ProjectionParams" => Self::ProjectionParams,
            "_ZBufferParams" => Self::ZBufferParams,
            "unity_OrthoParams" => Self::OrthoParams,
            _ => Self::Unresolved,
        }
    }
}

/// Where one ABI uniform gets its value.
enum FieldPlan {
    /// Written into [`UniformPlan::block`]; the fault if the value does not
    /// fit its slot.
    Material(Option<UniformFault>),
    Failed(SourceShaderError),
    Camera(Global),
}

/// A pass's uniform block with the material-owned fields already written.
struct UniformPlan {
    layout: Result<UniformLayout>,
    /// The whole block: material fields written, camera fields and padding zero.
    block: Vec<u8>,
    fields: Vec<FieldPlan>,
}
impl UniformPlan {
    /// `material` gives a field's material-owned value, `None` for a
    /// camera/global writer's field, or why the material cannot supply it.
    fn new(
        abi: &ProgramAbi,
        mut material: impl FnMut(&UniformField) -> Result<Option<UniformValue>>,
    ) -> Self {
        let layout = abi.uniform_layout();
        let mut block = match &layout {
            Ok(layout) => vec![0; layout.bytes()],
            Err(_) => Vec::new(),
        };
        let fields = abi
            .uniforms
            .iter()
            .enumerate()
            .map(|(index, field)| match material(field) {
                Ok(Some(value)) => FieldPlan::Material(match &layout {
                    Ok(layout) => layout.write(index, &value, &mut block).err(),
                    Err(_) => None,
                }),
                Ok(None) => FieldPlan::Camera(Global::of(&field.name)),
                Err(error) => FieldPlan::Failed(error),
            })
            .collect();
        Self {
            layout,
            block,
            fields,
        }
    }

    /// Write this view's block into `block`. The outer error is a writer that
    /// cannot supply its value, the first in field order; it precedes every
    /// packing check. The inner one is the first value, in field order, that
    /// does not fit its slot: it is reported where the block is consumed.
    fn fill(
        &self,
        abi: &ProgramAbi,
        globals: &ViewGlobals,
        block: &mut Vec<u8>,
    ) -> Result<Result<()>> {
        block.clear();
        block.extend_from_slice(&self.block);
        let mut fault = None;
        for (index, (field, plan)) in abi.uniforms.iter().zip(&self.fields).enumerate() {
            match plan {
                FieldPlan::Failed(error) => return Err(error.clone()),
                FieldPlan::Material(material) => {
                    if fault.is_none() {
                        fault = material.map(|f| (index, f));
                    }
                }
                FieldPlan::Camera(global) => {
                    let values = globals.get(*global, &field.name)?;
                    if let (None, Ok(layout)) = (fault, &self.layout) {
                        if let Err(f) = layout.write_floats(index, values, block) {
                            fault = Some((index, f));
                        }
                    }
                }
            }
        }
        if let Err(error) = &self.layout {
            return Ok(Err(error.clone()));
        }
        Ok(match fault {
            Some((index, fault)) => Err(fault.error(&abi.uniforms[index])),
            None => Ok(()),
        })
    }
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
    plan: Result<UniformPlan>,
}
impl PassResolution {
    fn resolve(
        source: &SourceParticle,
        catalogue: &SourceShaderCatalogue,
        catalogue_generation: u64,
        program: &GpuSourceProgram,
    ) -> Self {
        let plan = (|| {
            let (_, variant) = catalogue.select(
                program.receipt.source.reference.subshader,
                program.receipt.source.reference.pass,
                9,
                4,
                &source.material.keywords,
            )?;
            program.receipt.matches(catalogue, variant)?;
            Ok(UniformPlan::new(&program.receipt.abi, |field| {
                source.material.uniform(catalogue, field)
            }))
        })();
        Self {
            material: source.material.clone(),
            catalogue: source.catalogue.id(),
            catalogue_generation,
            receipt: program.receipt.clone(),
            plan,
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
    Fallback(Arc<FallbackTexture>),
}

/// A 1x1 texture of a built-in default, for a sampled slot that the material
/// leaves unassigned.
struct FallbackTexture {
    _texture: Texture,
    view: TextureView,
    sampler: Sampler,
}
impl FallbackTexture {
    fn new(device: &RenderDevice, queue: &RenderQueue, builtin: TextureDefault) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("built-in default texture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d { x: 0, y: 0, z: 0 },
                aspect: TextureAspect::All,
            },
            &builtin.rgba8(),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("built-in default sampler"),
            ..default()
        });
        Self {
            _texture: texture,
            view,
            sampler,
        }
    }
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
                let pipeline = if let Some(pipeline) = gpu.pipelines.get(&key)
                    .filter(|pipeline| pipeline.current(program)) {
                    pipeline.id
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
                    gpu.pipelines.insert(key, ParticlePipeline {
                        id: pipeline,
                        receipt: program.receipt.clone(),
                        vertex: program.vertex.id(),
                        fragment: program.fragment.id(),
                    });
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

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
// URP ScriptableRenderer.SetPerCameraShaderVariables writes (b, 2^b), where
// b = min(-log2(camera width / scaled width), 0) and the temporal mip bias;
// without render scaling or temporal AA, b is 0.
const MIP_BIAS: [f32; 2] = [0.0, 1.0];

/// One view's camera/global writer values for this frame.
struct ViewGlobals {
    values: Result<GlobalValues>,
    light: [f32; 4],
}
struct GlobalValues {
    matrix_vp: [f32; 16],
    matrix_v: [f32; 16],
    camera_position: [f32; 3],
    projection_params: [f32; 4],
    z_buffer_params: [f32; 4],
    ortho_params: [f32; 4],
}
impl ViewGlobals {
    /// For a view's `world_from_view` and its (source camera) `clip_from_view`.
    fn new(world_from_view: Mat4, clip_from_view: Mat4, camera: &Projection, light: [f32; 4]) -> Self {
        Self {
            values: GlobalValues::new(world_from_view, clip_from_view, camera),
            light,
        }
    }

    /// The value of `global` (`name` is the uniform it was resolved from).
    /// A view whose camera planes are unusable fails every camera writer,
    /// even one that does not read them.
    fn get(&self, global: Global, name: &str) -> Result<&[f32]> {
        let values = self.values.as_ref().map_err(Clone::clone)?;
        Ok(match global {
            Global::Identity => &IDENTITY,
            Global::MatrixVp => &values.matrix_vp,
            Global::MatrixV => &values.matrix_v,
            Global::MipBias => &MIP_BIAS,
            Global::PhenomenaLight => &self.light,
            Global::CameraPosition => &values.camera_position,
            Global::ProjectionParams => &values.projection_params,
            Global::ZBufferParams => &values.z_buffer_params,
            Global::OrthoParams => &values.ortho_params,
            Global::Unresolved => {
                return Err(SourceShaderError(format!(
                    "camera/global writer unresolved: {name}"
                )));
            }
        })
    }
}
impl GlobalValues {
    fn new(world_from_view: Mat4, clip_from_view: Mat4, camera: &Projection) -> Result<Self> {
        let source_to_render = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let view_from_world = world_from_view.inverse() * source_to_render;
        // The programs run under the reversed-Z conventions, which the
        // render projection already has (see `source_camera`).
        let projection = clip_from_view;
        // Validate the authored planes even when a shader only uses VP.
        let source_z = crate::source_camera::z_buffer_params(camera)?;
        let (near, far, ortho) = match camera {
            Projection::Perspective(p) => (p.near, p.far, false),
            Projection::Orthographic(p) => (p.near, p.far, true),
            _ => {
                return Err(SourceShaderError(
                    "custom camera projection needs explicit source parameter ownership".into(),
                ));
            }
        };
        let raw = clip_from_view;
        Ok(Self {
            matrix_vp: (projection * view_from_world).to_cols_array(),
            matrix_v: view_from_world.to_cols_array(),
            camera_position: source_to_render
                .transform_point3(world_from_view.w_axis.truncate())
                .to_array(),
            projection_params: [1.0, near, far, far.recip()],
            z_buffer_params: source_z,
            ortho_params: [
                2.0 / raw.x_axis.x,
                2.0 / raw.y_axis.y,
                0.0,
                f32::from(ortho),
            ],
        })
    }
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
    mut block: Local<Vec<u8>>,
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
    let block = &mut *block;
    // A surviving draw can replace its programs. Retaining by entity alone
    // would keep every old program's packed vertices, indices and material.
    let drawn: HashSet<_> = frame.particles.iter().flat_map(|item|
        item.source.passes.iter().map(move |pass| (item.entity, pass.program.id()))).collect();
    gpu.resolutions
        .retain(|key, _| drawn.contains(key));
    gpu.packs.retain(|key, _| drawn.contains(key));
    // A program no draw uses keeps its pipelines while it stays loaded (see
    // `RecentParticleAssets`): a draw that takes it again reuses them.
    let active_programs: HashSet<_> = drawn.iter().map(|(_, program)| *program).collect();
    gpu.pipelines.retain(|key, _| active_programs.contains(&key.program) || programs.get(key.program).is_some());
    gpu.arena_bytes.clear();
    gpu.arena_ranges.clear();
    let mut live = HashSet::new();
    for (view_entity, view, target, attachment, depth, _) in &views {
        let Some(camera) = frame.cameras.get(&view_entity) else {
            continue;
        };
        if frame.particles.is_empty() {
            continue;
        }
        let globals = ViewGlobals::new(
            view.world_from_view.to_matrix(),
            view.clip_from_view,
            camera,
            frame.light,
        );
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
                // A disabled draw is not drawn, and once it has a packet for
                // this pipeline nothing of it is brought up to date until it
                // is enabled again: the enabled path compares the uniform
                // block, the indices and the sampled resources, rewrites what
                // differs and gives the draw that frame's vertices. The packet
                // stays, so the draw stays ready.
                if !item.source.enabled {
                    if let Some(packet) =
                        gpu.packets.get_mut(&key).filter(|packet| packet.pipeline == pipeline)
                    {
                        packet.vertices = 0..0;
                        live.insert(key);
                        continue;
                    }
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
                    let plan = resolution.plan.as_ref().map_err(Clone::clone)?;
                    let packed = plan.fill(abi, &globals, block)?;
                    let mut sampled = Vec::with_capacity(abi.textures.len());
                    for declaration in &abi.textures {
                        if declaration.name == "_CameraDepthTexture" {
                            let Some(depth) = depth else {
                                return Ok(None);
                            };
                            sampled.push(Sampled::Depth(depth.source_view()));
                        } else if let Some(handle) = item.source.textures.get(&declaration.name) {
                            let Some(texture) = textures.get(handle.id()) else {
                                return Ok(None);
                            };
                            let texture = texture.result.as_ref().map_err(Clone::clone)?;
                            sampled.push(Sampled::Asset(texture));
                        } else {
                            // Only an unassigned slot binds the shader's declared default.
                            // Any other missing payload stays a named error.
                            let unassigned = item
                                .source
                                .material
                                .textures
                                .get(&declaration.name)
                                .is_some_and(|binding| {
                                    binding.status == "unassigned" && binding.texture.is_none()
                                });
                            if !unassigned {
                                return Err(SourceShaderError(format!(
                                    "unowned texture {}",
                                    declaration.name
                                )));
                            }
                            let builtin = catalogue.texture_default(&declaration.name)?;
                            let fallback = gpu
                                .defaults
                                .entry(builtin)
                                .or_insert_with(|| Arc::new(FallbackTexture::new(&device, &queue, builtin)))
                                .clone();
                            sampled.push(Sampled::Fallback(fallback));
                        }
                    }
                    let resources: Vec<TextureViewId> = sampled
                        .iter()
                        .map(|resource| match resource {
                            Sampled::Depth(depth_view) => depth_view.id(),
                            Sampled::Asset(texture) => texture.raw.id(),
                            Sampled::Fallback(fallback) => fallback.view.id(),
                        })
                        .collect();
                    let pack = gpu
                        .packs
                        .entry((item.entity, pass.program.id()))
                        .or_default();
                    pack.update(
                        &mut gpu.pack_id,
                        &item.source.streams,
                        &program.receipt,
                        &item.geometry,
                    );
                    let count = *pack.count.as_ref().map_err(Clone::clone)?;
                    let indices = &pack.indices;
                    // A disabled draw is not drawn, so its vertices are not
                    // uploaded.
                    let vertices = if item.source.enabled {
                        pack.arena_range(&mut gpu.arena_bytes, &mut gpu.arena_ranges)
                    } else {
                        0..0
                    };
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
                        packed?;
                        // A disabled draw is not drawn: its buffers are
                        // brought up to date when it is enabled again. Bytes
                        // a buffer already holds are not written again.
                        if item.source.enabled {
                            if packet.uniform != *block {
                                packet.binding.write_packed(&queue, block);
                                packet.uniform.clone_from(block);
                            }
                            if packet.indices != pack.index_id {
                                if packet.index.size() < indices.len() as u64 {
                                    packet.index = buffer(
                                        "source particle indices",
                                        indices,
                                        BufferUsages::INDEX,
                                    );
                                } else if !indices.is_empty() {
                                    queue.write_buffer(&packet.index, 0, indices);
                                }
                                packet.indices = pack.index_id;
                            }
                        }
                        packet.vertices = vertices;
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
                                    Sampled::Fallback(fallback) => SourceSampledResource::View {
                                        view: &fallback.view,
                                        sampler: &fallback.sampler,
                                        dimension: TextureViewDimension::D2,
                                        filterable: true,
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
                    let binding = SourceGpuBinding::create_packed(
                        &device,
                        &cache,
                        &program.receipt,
                        packed.map(|()| block.as_slice()),
                        &bindings,
                    )?;
                    Ok(Some(Packet {
                        pipeline,
                        binding,
                        vertices,
                        index: buffer("source particle indices", indices, BufferUsages::INDEX),
                        count,
                        resources,
                        distance: view.rangefinder3d().distance(&item.center)
                            - item.source.sorting_fudge,
                        sorting_order: item.source.sorting_order,
                        render_queue: item.source.render_queue,
                        rank: item.rank,
                        uniform: block.clone(),
                        indices: pack.index_id,
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
    if !gpu.arena_bytes.is_empty() {
        let size = gpu.arena_bytes.len() as u64;
        if gpu.arena.as_ref().is_none_or(|arena| arena.size() < size) {
            gpu.arena = Some(device.create_buffer(&BufferDescriptor {
                label: Some("source particle vertices"),
                size: size.next_power_of_two(),
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(gpu.arena.as_ref().expect("set above"), 0, &gpu.arena_bytes);
    }
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
        let gpu = gpu.into_inner();
        let Some(packet) = gpu
            .packets
            .get(&(view, item.entity(), false))
            .filter(|p| p.pipeline == item.pipeline && p.count > 0 && !p.vertices.is_empty())
        else {
            return RenderCommandResult::Skip;
        };
        let Some(arena) = &gpu.arena else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(0, &packet.binding.group, &[]);
        pass.set_vertex_buffer(0, arena.slice(packet.vertices.clone()));
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
        let Some(arena) = &gpu.arena else {
            return Ok(());
        };
        let cache = world.resource::<PipelineCache>();
        let frame = world.resource::<ParticleFrame>();
        // Only packets that draw: an empty pass loads and stores the targets
        // unchanged, so none is begun.
        let mut packets = gpu
            .packets
            .iter()
            .filter(|((view, draw, effect), _)| {
                *view == entity && *effect && frame.enabled.contains(draw)
            })
            .filter(|(_, p)| p.count > 0 && !p.vertices.is_empty())
            .filter_map(|(_, p)| Some((p, cache.get_render_pipeline(p.pipeline)?)))
            .collect::<Vec<_>>();
        if packets.is_empty() {
            return Ok(());
        }
        packets.sort_by(|(a, _), (b, _)| {
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
        for (packet, pipeline) in packets {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &packet.binding.group, &[]);
            pass.set_vertex_buffer(0, *arena.slice(packet.vertices.clone()));
            pass.set_index_buffer(*packet.index.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..packet.count, 0, 0..1);
        }
        Ok(())
    }
}

pub struct SourceParticlePlugin;
impl Plugin for SourceParticlePlugin {
    fn build(&self, app: &mut App) {
        // `extract` reads the main-world component itself and keeps its copy
        // across frames; the render world only needs the draw's entity.
        app.add_plugins(bevy::render::sync_component::SyncComponentPlugin::<SourceParticle>::default())
            .init_resource::<crate::source_particle::RecentParticleAssets>()
            .add_observer(crate::source_particle::keep_released)
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

#[cfg(test)]
mod tests {
    //! The per-view global table and the pre-packed material block are
    //! compared with the writer-by-name definition: each field's value looked
    //! up by its uniform name and the block packed by
    //! [`ProgramAbi::pack_uniforms`]. Same bytes, same first error.
    use super::*;
    use moly_assets::source_shader::abi::{Interfaces, ScalarType, ShaderStage};

    /// The writer-by-name definition of a camera/global uniform.
    fn reference_global(
        name: &str,
        world_from_view: Mat4,
        clip_from_view: Mat4,
        camera: &Projection,
        light: [f32; 4],
    ) -> Result<UniformValue> {
        let source_to_render = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let view_from_world = world_from_view.inverse() * source_to_render;
        let projection = clip_from_view;
        let source_z = crate::source_camera::z_buffer_params(camera)?;
        let (near, far, ortho) = match camera {
            Projection::Perspective(p) => (p.near, p.far, false),
            Projection::Orthographic(p) => (p.near, p.far, true),
            _ => {
                return Err(SourceShaderError(
                    "custom camera projection needs explicit source parameter ownership".into(),
                ));
            }
        };
        let raw = clip_from_view;
        let value = match name {
            "hlslcc_mtx4x4unity_ObjectToWorld" | "hlslcc_mtx4x4unity_WorldToObject" => {
                Mat4::IDENTITY.to_cols_array().to_vec()
            }
            "hlslcc_mtx4x4unity_MatrixVP" => (projection * view_from_world).to_cols_array().to_vec(),
            "hlslcc_mtx4x4unity_MatrixV" => view_from_world.to_cols_array().to_vec(),
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

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn unit(&mut self) -> f32 {
            (self.next() % 100_001) as f32 / 100_000.0
        }
    }

    /// Camera writers with the declared shape each is written into, plus an
    /// unknown name; each may instead get a mismatched shape.
    const CAMERA: [(&str, ScalarType, Option<u32>); 11] = [
        ("hlslcc_mtx4x4unity_ObjectToWorld", ScalarType::Vec4, Some(4)),
        ("hlslcc_mtx4x4unity_WorldToObject", ScalarType::Vec4, Some(4)),
        ("hlslcc_mtx4x4unity_MatrixVP", ScalarType::Vec4, Some(4)),
        ("hlslcc_mtx4x4unity_MatrixV", ScalarType::Vec4, Some(4)),
        ("_GlobalMipBias", ScalarType::Vec2, None),
        ("_GlobalPhenomenaDirectionalLightColor", ScalarType::Vec4, None),
        ("_WorldSpaceCameraPos", ScalarType::Vec3, None),
        ("_ProjectionParams", ScalarType::Vec4, None),
        ("_ZBufferParams", ScalarType::Vec4, None),
        ("unity_OrthoParams", ScalarType::Vec4, None),
        ("_UnownedGlobal", ScalarType::Vec4, None),
    ];

    enum Source {
        Material(UniformValue),
        Failed,
        Camera,
    }

    fn case(rng: &mut Rng) -> (ProgramAbi, Vec<Source>) {
        let mut uniforms = Vec::new();
        let mut sources = Vec::new();
        let mut offset = 0;
        for index in 0..1 + rng.below(9) {
            let kind = rng.below(10);
            let failed = rng.below(3) == 0;
            let (name, ty, array, source) = match kind {
                0..=5 => {
                    let known = CAMERA.len() - 1 + usize::from(rng.below(8) == 0);
                    let (name, mut ty, mut array) = CAMERA[rng.below(known)];
                    if rng.below(20) == 0 {
                        (ty, array) = [(ScalarType::Vec4, None), (ScalarType::Vec3, Some(4)), (ScalarType::Uvec4, None)][rng.below(3)];
                    }
                    (name.to_string(), ty, array, Source::Camera)
                }
                9 if failed => (format!("_Failed{index}"), ScalarType::Vec4, None, Source::Failed),
                _ => {
                    let array = (rng.below(4) == 0).then(|| 1 + rng.below(3) as u32);
                    let mut length = array.unwrap_or(1) as usize * 4;
                    if rng.below(25) == 0 {
                        length += 1;
                    }
                    let mut values: Vec<f32> = (0..length).map(|_| rng.unit() * 8.0 - 4.0).collect();
                    if rng.below(40) == 0 {
                        values[0] = f32::NAN;
                    }
                    (format!("_Material{index}"), ScalarType::Vec4, array, Source::Material(UniformValue::Float(values)))
                }
            };
            if uniforms.iter().any(|u: &UniformField| u.name == name) {
                continue;
            }
            let bytes = array.unwrap_or(1) * 16;
            uniforms.push(UniformField {
                name,
                ty,
                array,
                offset,
                bytes,
                member: format!("m{index}"),
                stages: vec![ShaderStage::Vertex],
            });
            sources.push(source);
            offset += bytes;
        }
        let abi = ProgramAbi {
            version: 1,
            bind_group: 0,
            uniform_binding: 0,
            uniform_bytes: offset,
            uniforms,
            textures: Vec::new(),
            interfaces: Interfaces {
                vertex: Vec::new(),
                fragment: Vec::new(),
            },
            source_clip_space: "gl-minus-one-to-one".into(),
            texture_data_origin: "unity-lower-left".into(),
            array_layer_rule: "clamp(floor(layer+0.5),0,layers-1)".into(),
        };
        (abi, sources)
    }

    fn camera(rng: &mut Rng) -> Projection {
        let near_choices = if rng.below(10) == 0 { 6 } else { 4 };
        let near = [0.01, 0.1, 0.3, 1.0, 0.0, -1.0][rng.below(near_choices)];
        let far_choices = if rng.below(10) == 0 { 4 } else { 3 };
        let far = [5.0, 100.0, 1000.0, 0.5][rng.below(far_choices)];
        if rng.below(4) == 0 {
            Projection::Orthographic(OrthographicProjection {
                near,
                far,
                ..OrthographicProjection::default_3d()
            })
        } else {
            Projection::Perspective(PerspectiveProjection {
                near,
                far,
                fov: 0.3 + rng.unit(),
                aspect_ratio: 0.5 + rng.unit() * 2.0,
                ..default()
            })
        }
    }

    fn view(rng: &mut Rng, camera: &Projection) -> (Mat4, Mat4) {
        let world_from_view = Mat4::from_rotation_translation(
            Quat::from_euler(EulerRot::YXZ, rng.unit() * 6.0, rng.unit() - 0.5, rng.unit() * 0.2),
            Vec3::new(rng.unit() * 40.0 - 20.0, rng.unit() * 10.0, rng.unit() * 40.0 - 20.0),
        );
        let clip_from_view = match crate::source_camera::render_projection(camera.get_clip_from_view(), camera) {
            Ok(projection) => projection,
            Err(_) => camera.get_clip_from_view(),
        };
        (world_from_view, clip_from_view)
    }

    fn reference(
        abi: &ProgramAbi,
        sources: &[Source],
        world_from_view: Mat4,
        clip_from_view: Mat4,
        camera: &Projection,
        light: [f32; 4],
    ) -> Result<Vec<u8>> {
        let values = abi
            .uniforms
            .iter()
            .zip(sources)
            .map(|(field, source)| {
                Ok((
                    field.name.clone(),
                    match source {
                        Source::Material(value) => value.clone(),
                        Source::Failed => {
                            return Err(SourceShaderError(format!("no material value {}", field.name)))
                        }
                        Source::Camera => reference_global(
                            &field.name,
                            world_from_view,
                            clip_from_view,
                            camera,
                            light,
                        )?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        abi.pack_uniforms(|field| {
            values
                .get(&field.name)
                .cloned()
                .ok_or_else(|| SourceShaderError(format!("unowned source uniform {}", field.name)))
        })
    }

    fn plan(abi: &ProgramAbi, sources: &[Source]) -> UniformPlan {
        let mut index = 0;
        UniformPlan::new(abi, |field| {
            let source = &sources[index];
            index += 1;
            match source {
                Source::Material(value) => Ok(Some(value.clone())),
                Source::Failed => Err(SourceShaderError(format!("no material value {}", field.name))),
                Source::Camera => Ok(None),
            }
        })
    }

    #[test]
    fn view_table_and_material_block_pack_as_writers_by_name() {
        let mut rng = Rng(0xd1b5_4a32_d192_ed03);
        let mut block = Vec::new();
        let (mut packed, mut rejected) = (0, 0);
        for index in 0..20_000 {
            let (abi, sources) = case(&mut rng);
            let plan = plan(&abi, &sources);
            // One plan serves several views and frames, as a resolution does.
            for _ in 0..3 {
                let camera = camera(&mut rng);
                let (world_from_view, clip_from_view) = view(&mut rng, &camera);
                let light = [rng.unit(), rng.unit(), rng.unit(), 1.0];
                let expected =
                    reference(&abi, &sources, world_from_view, clip_from_view, &camera, light);
                let globals = ViewGlobals::new(world_from_view, clip_from_view, &camera, light);
                let actual = match plan.fill(&abi, &globals, &mut block) {
                    Err(error) | Ok(Err(error)) => Err(error),
                    Ok(Ok(())) => Ok(block.clone()),
                };
                assert_eq!(actual, expected, "case {index}");
                if expected.is_ok() {
                    packed += 1;
                } else {
                    rejected += 1;
                }
            }
        }
        assert!(packed > 20_000 && rejected > 5_000, "{packed} packed, {rejected} rejected");
    }

    /// The comparison above sees a camera writer that reads another view's
    /// value: a table built for a moved camera disagrees.
    #[test]
    fn another_views_table_is_seen_by_the_comparison() {
        let mut rng = Rng(0x94d0_49bb_1331_11eb);
        let mut block = Vec::new();
        let (mut compared, mut differing) = (0, 0);
        for _ in 0..2_000 {
            let (abi, sources) = case(&mut rng);
            let camera = camera(&mut rng);
            let (world_from_view, clip_from_view) = view(&mut rng, &camera);
            let light = [rng.unit(), rng.unit(), rng.unit(), 1.0];
            let Ok(expected) = reference(&abi, &sources, world_from_view, clip_from_view, &camera, light) else {
                continue;
            };
            if !abi.uniforms.iter().any(|u| {
                matches!(u.name.as_str(), "hlslcc_mtx4x4unity_MatrixVP" | "hlslcc_mtx4x4unity_MatrixV" | "_WorldSpaceCameraPos")
            }) {
                continue;
            }
            let moved = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)) * world_from_view;
            let globals = ViewGlobals::new(moved, clip_from_view, &camera, light);
            let plan = plan(&abi, &sources);
            plan.fill(&abi, &globals, &mut block).unwrap().unwrap();
            compared += 1;
            differing += usize::from(block != expected);
        }
        assert!(compared > 200 && differing == compared, "{differing} of {compared} differ");
    }
}
