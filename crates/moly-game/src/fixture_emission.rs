//! 家具 Basic 族的自发光 pass：源 Base 程序第二颜色目标的承接侧。
//!
//! 源程序同时写两个颜色目标（Target0 主色由 `fixture_material` 承接、
//! Target1 = 自发射遮罩 × 现象自发光门），并把第二目标喂给粒子泛光链的
//! 输入缓冲。本管线的主 pass 只有一个颜色目标（与站点族同一裁决），这条
//! 第二目标由本模块自建 render graph 节点直画：深度测试复用主 pass 的
//! 深度图（只读不写），颜色写进一张半浮点缓冲，天气链的泛光金字塔以它
//! 为预滤波源。着色程序在 `shaders/fixture_emission.wgsl`（片元式与绑定
//! 契约的注释在那里）。
//!
//! # 帧内节奏
//!
//! Extract 抽带 [`FixtureEmission`] 组件的可见网格及世界矩阵 →
//! PrepareResources 特化管线（顶点布局 × 材质变体 × 混合 × 采样
//! 数）、写对象 uniform 池、按视口建 [`ViewEmissionTarget`] → 节点在主
//! pass 之后跑（清缓冲 → 逐 draw 动态 offset 换窗）。
//!
//! # 排序与混合
//!
//! 源的第二目标与主目标共用一对混合因子（不透明不混、混合
//! SrcAlpha/OneMinusSrcAlpha）；本 pass 按材质逐管线取同一对因子。混合
//! draw 按视空间深度从最远画起（背对相机的先落笔），不透明 draw 先画
//! 且保持稳定序——半透明层叠的可见性由画家算法保证。
//!
//! # fail-closed 点
//!
//! - 相机不唯一（未就绪或两台 3D 相机并存）：整帧清名单，一个不画——
//!   不画好过按错投影画。
//! - 网格缺 uv 布局：该网格的管线记 INVALID（一次性告警），draw 跳过。
//! - 遮罩/主贴图任一没有 GPU 侧资源：那条 draw 跳过（不缓存失败，下一
//!   帧再试）。
//! - 本帧没有任何 draw：缓冲照样清——不清会把上一帧的余像喂进泛光。
//!   管线没编好（首几帧）只跳过对应 draw，pass 仍执行。

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::ecs::query::QueryItem;
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::pbr::{skins_use_uniform_buffers, SkinUniforms, MAX_JOINTS};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, RenderSubGraph, ViewNode,
    ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::{GpuImage, TextureCache};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{Msaa, ViewDepthTexture, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

use crate::env::SiteEnvGpuBuffer;
use crate::uber_particle::{ParticleEmission, UberParticleMaterial, CullArm, BlendArm, UBER_SHADER};
use crate::fixture_material::{FixtureMaterialKey, FixtureParams};

/// 自发光链着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`（与阴影
/// 链同一形状——仓内没有默认资产源目录）。
const FIXTURE_EMISSION_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0xa3bb_fccd_de89_481f_b19f_609f_5f9c_d85b),
    PhantomData,
);

/// 自发光缓冲的格式：半浮点，与天气链的泛光金字塔同一档——第二目标存
/// 线性域的遮罩值，泛光全链按线性消费。
const EMISSION_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// 逐对象 uniform（着色程序里的 `EmissionObject`）的字节数：对象矩阵
/// 64 + 材质参数 12 槽 192 + 自发光 vec4 16 + skin address uvec4 16。槽序契约见
/// `shaders/fixture_emission.wgsl`。家具存 world_from_local；粒子的既有
/// 对象契约仍存 clip_from_local，二者由相应着色器解释，字节偏移不变。
const EMISSION_OBJECT_BYTES: usize = 64 + 192 + 16 + 16;

/// 逐实体对象池的容量上限；超出的 draw 丢弃并告警一次（摆件量级远低于
/// 此，上限只是护栏——与阴影链同值）。
const OBJECT_CAPACITY: usize = 8192;

/// 主世界组件：给一个网格实体挂上「第二颜色目标」的参与资格。换装侧
/// （`fixture_material`）在材质带自发射遮罩贴图时插入；遮罩在而两个门
/// int 全零的材质照插——门在着色器里按类型比较，自然为零。
#[derive(Component)]
pub struct FixtureEmission {
    pub force_emission: bool,
    /// 自发射遮罩贴图（源 `_EmissionMaskTex`；按 glb 的纹理下标装载）。
    pub mask: Handle<Image>,
    /// 主贴图：alpha clip 变体与输出 alpha 都读它（源里两个目标的 alpha
    /// 同为主贴图 alpha）。
    pub main_tex: Handle<Image>,
    /// 与主 pass 材质同一份参数（同一契约的 12 槽；本 pass 只消费其中
    /// uv 偏移 / 抖动 / v 翻转 / 窗门四角）。
    pub params: FixtureParams,
    /// 与主 pass 同一键（三条变体门在两侧各自编译，取值同源）。
    pub key: FixtureMaterialKey,
    /// 混合材质标记：混合 draw 的管线带 SrcAlpha 混合并按视深排序。
    pub blend: bool,
    /// `_BrightPhenomenaEmission`（类型 1 的门比较对象；数据缺键按 0——
    /// 源 shader 的属性默认值就是 0，换装日志对缺键原样记 None）。
    pub bright: f32,
    /// `_DarkPhenomenaEmission`（类型 2 的门比较对象）。
    pub dark: f32,
}

/// Per-instance material override written by fixture animation callbacks.
/// Material defaults follow the phenomenon; events force a branch without
/// changing the authored bright/dark flags or another instance's material.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FixtureEmissionOverride {
    #[default]
    FollowPhenomenon,
    Enabled,
    Disabled,
}

impl FixtureEmissionOverride {
    fn uniform(self) -> f32 {
        match self {
            Self::FollowPhenomenon => 0.0,
            Self::Enabled => 1.0,
            Self::Disabled => 2.0,
        }
    }
}

/// 一个视图的自发光缓冲：节点写它，天气链的泛光预滤波读 `resolved`。
/// msaa > 1 时本 pass 与主 pass 同采样数（深度图是多采样的，pass 采样数
/// 必须一致），msaa 视图只作 attachment，store 时 resolve 进 resolved。
#[derive(Component)]
pub struct ViewEmissionTarget {
    msaa: Option<TextureView>,
    /// 已 resolve 的单采样视图：泛光预滤波的采样源。
    pub resolved: TextureView,
}

/// 一条自发光 draw：extract 时算好的逐对象数据 + 特化管线。
struct EmissionDraw {
    mesh: AssetId<Mesh>,
    /// Reuse the main pass's skin allocation; never rebuild another palette.
    skin: Option<MainEntity>,
    skin_byte_offset: Option<u32>,
    /// Fixture shader: world_from_local; particle shader: clip_from_local.
    /// The fixture pass must not precombine view and model transforms: it
    /// reuses the main depth and needs the same intermediate world position.
    vertex_transform: Mat4,
    params: Option<FixtureParams>,
    particle: Option<ParticleEmission>,
    /// (bright, dark)。
    emission: [f32; 4],
    key: FixtureMaterialKey,
    blend: bool,
    /// 视空间 z（右手视空间，前方为负；越负越远）：混合 draw 的排序键。
    view_z: f32,
    pipeline: CachedRenderPipelineId,
    mask: AssetId<Image>,
    main: AssetId<Image>,
}

/// 抽取结果：本帧 draw 名单。
#[derive(Resource, Default)]
struct EmissionDrawList {
    draws: Vec<EmissionDraw>,
}

/// 管线特化键：顶点布局 × 图元 × 材质变体 × 混合 × 采样数。
#[derive(Clone, PartialEq, Eq, Hash)]
struct EmissionPipelineKey {
    layout: MeshVertexBufferLayoutRef,
    topology: PrimitiveTopology,
    key: FixtureMaterialKey,
    blend: bool,
    sample_count: u32,
    particle: Option<(bool, bool, CullArm, BlendArm)>,
    skinned: bool,
}

/// 已入队的管线：键 → 缓存 id。
#[derive(Resource, Default)]
struct EmissionPipelines {
    queued: HashMap<EmissionPipelineKey, CachedRenderPipelineId>,
}

/// 渲染世界的共享资源：一次建，逐帧只写对象池。
#[derive(Resource)]
struct EmissionGpu {
    /// 逐对象 uniform 池：动态 offset 绑定，槽距 [`EmissionGpu::object_stride`]。
    object_buffer: Buffer,
    /// 动态 offset 槽距：288 不是对齐倍数，向上取整（典型对齐 256 → 512）。
    object_stride: u32,
    /// 对象 uniform 的绑定尺寸（动态 offset 校验用）。
    object_binding_size: u64,
    /// group 0：对象、主视图与原有蒙皮 buffer；前两者为动态 offset。
    object_layout: BindGroupLayoutDescriptor,
    /// Same uniform/storage choice as Bevy's main-pass skin buffers.
    skin_uniforms: bool,
    /// group 1 布局（全局量表 + 遮罩 + 主贴图 + 采样器，全 FRAGMENT）。
    texture_layout: BindGroupLayoutDescriptor,
    /// 钳边三线性采样器：与家具主材质同一组参数（同一 glb 里两张纹理
    /// 共用同一条采样器条目）。
    sampler: Sampler,
}

/// 顶点输入：位置 @0、uv @1；蒙皮另带 joints @2 / weights @3。槽位由这里
/// 指定；缺 uv 的网格报错 → 管线 INVALID（一次性告警），draw 跳过。
fn emission_vertex_layout(
    mesh: &RenderMesh,
    particle: bool,
    skinned: bool,
) -> Result<bevy::mesh::VertexBufferLayout, ()> {
    let mut attributes = vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0), Mesh::ATTRIBUTE_UV_0.at_shader_location(1)];
    if particle { attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(2)); }
    if skinned {
        attributes.push(Mesh::ATTRIBUTE_JOINT_INDEX.at_shader_location(2));
        attributes.push(Mesh::ATTRIBUTE_JOINT_WEIGHT.at_shader_location(3));
    }
    mesh.layout.0.get_layout(&attributes).map_err(|_| ())
}

/// Extract：家具保留世界矩阵，裁剪矩阵由节点绑定的主视图 uniform 提供。
/// 不能先在 CPU 合并 MVP 再乘顶点：那与主 pass 的两段乘法有不同舍入，
/// 会破坏共面自发光片元的深度比较。粒子保持其独立的已有对象契约。
/// 相机不唯一整帧清名单。
#[allow(clippy::type_complexity)]
fn extract_emission_draws(
    mut commands: Commands,
    draws: Extract<Query<(Entity, &Mesh3d, &GlobalTransform, &ViewVisibility, &FixtureEmission, Option<&FixtureEmissionOverride>, Option<&SkinnedMesh>)>>,
    particles: Extract<Query<(&Mesh3d, &GlobalTransform, &ViewVisibility, &ParticleEmission, &MeshMaterial3d<UberParticleMaterial>)>>,
    particle_materials: Extract<Res<Assets<UberParticleMaterial>>>,
    cameras: Extract<Query<(&GlobalTransform, &Camera), With<Camera3d>>>,
    mut overflow_warned: Local<bool>,
) {
    let mut list = EmissionDrawList::default();
    if let Ok((cam_global, camera)) = cameras.single() {
        let view_from_world = cam_global.to_matrix().inverse();
        let view_proj = camera.clip_from_view() * view_from_world;
        for (entity, mesh, transform, visibility, emission, mode, skin) in &draws {
            if !visibility.get() {
                continue;
            }
            let world_from_local = transform.to_matrix();
            let view_z = view_from_world
                .transform_point3(world_from_local.w_axis.xyz())
                .z;
            list.draws.push(EmissionDraw {
                mesh: mesh.0.id(),
                skin: skin.map(|_| entity.into()),
                skin_byte_offset: None,
                vertex_transform: world_from_local,
                params: Some(emission.params),
                particle: None,
                emission: [emission.bright, emission.dark, f32::from(emission.force_emission), mode.copied().unwrap_or_default().uniform()],
                key: emission.key,
                blend: emission.blend,
                view_z,
                pipeline: CachedRenderPipelineId::INVALID,
                mask: emission.mask.id(),
                main: emission.main_tex.id(),
            });
        }
        for (mesh, transform, visibility, effect, material) in &particles {
            if !visibility.get() { continue; }
            let Some(material) = particle_materials.get(&material.0) else { continue; };
            list.draws.push(EmissionDraw {
                mesh: mesh.0.id(), vertex_transform: view_proj * transform.to_matrix(),
                skin: None, skin_byte_offset: None,
                params: None, particle: Some(*effect), emission: [0.0; 4],
                key: FixtureMaterialKey { fence: false, rug: None, render_queue: 0, alpha_clip: false, dither: false, window_clip: false, fresnel: false, reflection: false },
                blend: true, view_z: view_from_world.transform_point3(transform.translation()).z,
                pipeline: CachedRenderPipelineId::INVALID,
                mask: material.base_map.id(), main: material.base_map.id(),
            });
        }
        let overflow = list.draws.len() > OBJECT_CAPACITY;
        list.draws.truncate(OBJECT_CAPACITY);
        if overflow && !*overflow_warned {
            *overflow_warned = true;
            warn!(
                "家具自发光 draw 超过容量上限 {OBJECT_CAPACITY}，超出部分不画（仅告警一次）"
            );
        }
        // 排序：不透明先画且保持抽取稳定序；混合按视空间 z 升序（越负越
        // 远先画）。稳定排序让不透明块的相对序不被混合项打乱。
        list.draws.sort_by(|a, b| match (a.blend, b.blend) {
            (false, false) => std::cmp::Ordering::Equal,
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            (true, true) => a.view_z.total_cmp(&b.view_z),
        });
    }
    commands.insert_resource(list);
}

/// 对象 uniform 的字节：对象矩阵四列（列主序摊平）+ 参数 12 槽 + 自发光
/// vec4。槽序与 `shaders/fixture_emission.wgsl` 的 `EmissionObject` 是契约。
fn emission_object_bytes(draw: &EmissionDraw) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(EMISSION_OBJECT_BYTES);
    for component in draw.vertex_transform.to_cols_array() {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    if let Some(effect) = draw.particle {
        for slot in [effect.params.base_st, effect.params.tint_colour, effect.params.scalars, effect.colour, Vec4::new(effect.intensity, effect.colour_type, 0.0, 0.0)] {
            for value in slot.to_array() { bytes.extend_from_slice(&value.to_le_bytes()); }
        }
        bytes.resize(64 + 192, 0);
    } else if let Some(params) = draw.params { bytes.extend_from_slice(&params.bytes()); }
    for value in draw.emission {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    // Storage buffers address the whole palette; uniform buffers instead use
    // a dynamic byte offset at binding 2. Keep existing particle slots intact.
    for value in [draw.skin_byte_offset.unwrap_or(0) / 64, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    debug_assert_eq!(bytes.len(), EMISSION_OBJECT_BYTES);
    bytes
}

/// 视口尺寸的自发光缓冲描述。`attachment_only` 是多采样 intermediates
/// （从不被采样）；resolved 加 TEXTURE_BINDING（泛光预滤波的采样源）。
fn emission_texture_descriptor(
    width: u32,
    height: u32,
    samples: u32,
    attachment_only: bool,
    label: &'static str,
) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: TextureDimension::D2,
        format: EMISSION_FORMAT,
        usage: if attachment_only {
            TextureUsages::RENDER_ATTACHMENT
        } else {
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING
        },
        view_formats: &[],
    }
}

/// PrepareResources：特化管线、写对象池、按视口建自发光缓冲。挂
/// PrepareResources 与阴影链同由：本帧的 `RenderAssets<RenderMesh>` 与
/// MeshAllocator 切片已定，且组件插入与图执行之间有同步点保证。
#[allow(clippy::too_many_arguments)]
fn prepare_emission(
    mut commands: Commands,
    mut draws: ResMut<EmissionDrawList>,
    mut pipelines: ResMut<EmissionPipelines>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    render_device: Res<RenderDevice>,
    meshes: Res<RenderAssets<RenderMesh>>,
    gpu: Res<EmissionGpu>,
    skins: Res<SkinUniforms>,
    views: Query<(Entity, &ExtractedCamera, Option<&Msaa>)>,
    mut texture_cache: ResMut<TextureCache>,
    mut missing_uv_warned: Local<bool>,
) {
    // 3D 视图的自发光缓冲与采样数（pass 与主 pass 共用同一张多采样深度
    // 图，采样数必须一致）。站点场景单 3D 相机；出现多台时每个视图各得
    // 一份缓冲，draw 名单共享（抽取侧只认单相机，多了整帧不画）。
    let mut sample_count = 1u32;
    for (entity, camera, msaa) in &views {
        if camera.render_graph != Core3d.intern() {
            continue;
        }
        let samples = msaa.map(|m| m.samples()).unwrap_or(1);
        sample_count = samples;
        let Some(viewport) = camera.physical_target_size else {
            continue;
        };
        let (msaa_view, resolved) = if samples > 1 {
            let msaa_texture = texture_cache.get(
                &render_device,
                emission_texture_descriptor(
                    viewport.x,
                    viewport.y,
                    samples,
                    true,
                    "fixture_emission_msaa",
                ),
            );
            let resolved = texture_cache.get(
                &render_device,
                emission_texture_descriptor(
                    viewport.x,
                    viewport.y,
                    1,
                    false,
                    "fixture_emission_resolved",
                ),
            );
            (Some(msaa_texture.default_view), resolved.default_view)
        } else {
            let resolved = texture_cache.get(
                &render_device,
                emission_texture_descriptor(
                    viewport.x,
                    viewport.y,
                    1,
                    false,
                    "fixture_emission_resolved",
                ),
            );
            (None, resolved.default_view)
        };
        commands
            .entity(entity)
            .insert(ViewEmissionTarget { msaa: msaa_view, resolved });
    }

    // 对象池按 draw 序全量写（缺管线的 draw 也占位，动态 offset 才对得上；
    // 288B 数据 + 槽距零填充）。
    let mut object_bytes =
        Vec::with_capacity(draws.draws.len() * gpu.object_stride as usize);
    for item in draws.draws.iter_mut() {
        item.skin_byte_offset = item.skin.and_then(|entity| skins.skin_byte_offset(entity))
            .map(|offset| offset.byte_offset);
        object_bytes.extend_from_slice(&emission_object_bytes(item));
        object_bytes.extend(
            std::iter::repeat(0u8).take(gpu.object_stride as usize - EMISSION_OBJECT_BYTES),
        );
        let Some(render_mesh) = meshes.get(item.mesh) else {
            continue;
        };
        if item.skin.is_some() && item.skin_byte_offset.is_none() {
            // A skin not yet extracted must not flash its undeformed mesh.
            continue;
        }
        let skinned = item.skin.is_some();
        let vertex_layout = match emission_vertex_layout(render_mesh, item.particle.is_some(), skinned) {
            Ok(layout) => layout,
            Err(()) => {
                if !*missing_uv_warned {
                    *missing_uv_warned = true;
                    warn!(
                        "自发光 pass 遇到缺 uv 布局的网格，该网格不进第二颜色目标（仅告警一次）"
                    );
                }
                continue;
            }
        };
        let topology = render_mesh.primitive_topology();
        let mat_key = item.key;
        let blend = item.blend;
        let particle = item.particle.map(|p| (p.tint_area, p.area, p.cull, p.blend));
        item.pipeline = *pipelines
            .queued
            .entry(EmissionPipelineKey {
                layout: render_mesh.layout.clone(),
                topology,
                key: mat_key,
                blend,
                sample_count,
                particle,
                skinned,
            })
            .or_insert_with(|| {
                let mut defs: Vec<&str> = Vec::new();
                if skinned {
                    defs.push("SKINNED");
                    if gpu.skin_uniforms { defs.push("SKINS_USE_UNIFORM_BUFFERS"); }
                }
                if mat_key.rug.is_some() { defs.push("FIXTURE_RUG"); }
                if let Some((tint, area, _, _)) = particle {
                    defs.push("UBER_EFFECT_PASS");
                    if tint { defs.push("UBER_TINT_AREA_ALL"); }
                    if area { defs.push("UBER_EMISSION_AREA_ALL"); }
                }
                let shader = if particle.is_some() { UBER_SHADER.clone() } else { FIXTURE_EMISSION_SHADER.clone() };
                if mat_key.alpha_clip {
                    defs.push("FIXTURE_ALPHA_CLIP");
                }
                if mat_key.dither {
                    defs.push("FIXTURE_DITHER");
                }
                if mat_key.window_clip {
                    defs.push("FIXTURE_WINDOW_CLIP");
                }
                let mut descriptor = RenderPipelineDescriptor {
                    label: Some("fixture_emission_pipeline".into()),
                    layout: vec![gpu.object_layout.clone(), gpu.texture_layout.clone()],
                    vertex: VertexState {
                        shader: shader.clone(),
                        shader_defs: vec![],
                        entry_point: Some("emission_vertex".into()),
                        buffers: vec![vertex_layout],
                    },
                    fragment: Some(FragmentState {
                        shader: shader.clone(),
                        shader_defs: vec![],
                        entry_point: Some("emission_fragment".into()),
                        targets: vec![Some(ColorTargetState {
                            format: EMISSION_FORMAT,
                            // 源的第二目标与主目标共用一对混合因子
                            //（不透明不混；混合 SrcAlpha/OneMinusSrcAlpha）。
                            blend: if particle.map(|p| p.3) == Some(BlendArm::Additive) {
                                Some(BlendState { color: BlendComponent { src_factor: BlendFactor::SrcAlpha, dst_factor: BlendFactor::One, operation: BlendOperation::Add }, alpha: BlendComponent::OVER })
                            } else if blend {
                                Some(BlendState::ALPHA_BLENDING)
                            } else {
                                None
                            },
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    primitive: PrimitiveState {
                        topology,
                        front_face: FrontFace::Ccw,
                        cull_mode: match particle.map(|p| p.2) { Some(CullArm::Off) => None, Some(CullArm::Front) => Some(Face::Front), _ => Some(Face::Back) },
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        // 主 pass 之后只读：reverse-Z 的 GreaterEqual 比较、
                        // 不写、不偏置（偏置属于写深度的 pass，这里没有
                        // 可偏置的写）。
                        depth_write_enabled: false,
                        depth_compare: CompareFunction::GreaterEqual,
                        stencil: StencilState::default(),
                        bias: DepthBiasState::default(),
                    }),
                    multisample: MultisampleState {
                        count: sample_count,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                for def in defs {
                    descriptor.vertex.shader_defs.push(def.into());
                    if let Some(ref mut fragment) = descriptor.fragment {
                        fragment.shader_defs.push(def.into());
                    }
                }
                pipeline_cache.queue_render_pipeline(descriptor)
            });
    }
    if !object_bytes.is_empty() {
        render_queue.write_buffer(&gpu.object_buffer, 0, &object_bytes);
    }
}

/// 自发光节点：主 pass 之后跑。每个 3D 视图一份；ViewQuery 缺件
/// （无深度图或无本 pass 组件）的视图直接不匹配，节点不跑。
#[derive(Default)]
struct EmissionPassNode;

impl ViewNode for EmissionPassNode {
    type ViewQuery = (
        &'static ViewEmissionTarget,
        &'static ViewDepthTexture,
        &'static ViewUniformOffset,
    );

    #[allow(clippy::type_complexity)]
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (target, depth, view_offset): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let draws = world.resource::<EmissionDrawList>();
        let gpu = world.resource::<EmissionGpu>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let meshes = world.resource::<RenderAssets<RenderMesh>>();
        let images = world.resource::<RenderAssets<GpuImage>>();
        let allocator = world.resource::<MeshAllocator>();
        let env = world.resource::<SiteEnvGpuBuffer>();
        let view_uniforms = world.resource::<ViewUniforms>();
        let skins = world.resource::<SkinUniforms>();
        // A ViewUniformOffset is inserted by prepare_view_uniforms only after
        // writing that view's uniform. This is the exact buffer read by the
        // main fixture vertex shader, including any adjusted projection.
        let view_binding = view_uniforms.uniforms.binding()
            .expect("main view uniforms are prepared before the emission pass");

        // 管线就绪表（去重）；没编完的只跳过那条 draw，pass 照常清缓冲。
        let mut ready: Vec<(CachedRenderPipelineId, &RenderPipeline)> = Vec::new();
        for item in &draws.draws {
            if item.pipeline == CachedRenderPipelineId::INVALID
                || ready.iter().any(|(id, _)| *id == item.pipeline)
            {
                continue;
            }
            if let Some(pipeline) = pipeline_cache.get_render_pipeline(item.pipeline) {
                ready.push((item.pipeline, pipeline));
            }
        }
        let resolve = |id: CachedRenderPipelineId| -> Option<&RenderPipeline> {
            ready
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, pipeline)| *pipeline)
        };

        // bind group 全部先建后跑：设备句柄借 render_context 的不可变引用，
        // 跑 pass 要可变借用，交叠即撞（阴影链同注）。
        let device = render_context.render_device();
        // group 0：对象池、主视图与主 pass 同帧的蒙皮 buffer。统一按
        // binding 顺序传动态 offset；storage 蒙皮分支无第三个动态 offset。
        let object_bind_group = device.create_bind_group(
            "fixture_emission_object_bind_group",
            &pipeline_cache.get_bind_group_layout(&gpu.object_layout),
            &BindGroupEntries::with_indices((
                (
                    0u32,
                    BindingResource::Buffer(BufferBinding {
                        buffer: &gpu.object_buffer,
                        offset: 0,
                        size: Some(
                            std::num::NonZeroU64::new(gpu.object_binding_size).unwrap(),
                        ),
                    }),
                ),
                (1u32, view_binding),
                (2u32, BindingResource::Buffer(BufferBinding {
                    buffer: &skins.current_buffer,
                    offset: 0,
                    size: gpu.skin_uniforms.then(|| std::num::NonZeroU64::new((MAX_JOINTS * 64) as u64).unwrap()),
                })),
            )),
        );
        // group 1：按 (遮罩, 主贴图) 对去重；缺 GPU 侧资源的对不建组，对应
        // draw 跳过（fail-closed，下一帧再试）。
        let texture_layout = pipeline_cache.get_bind_group_layout(&gpu.texture_layout);
        let mut texture_groups: HashMap<(AssetId<Image>, AssetId<Image>), BindGroup> =
            HashMap::new();
        // group 1 的建组也必须在 begin pass 之前（设备句柄借 render_context
        // 的不可变引用，pass 要可变借用——见上）。缺 GPU 侧资源的对不建组，
        // 对应 draw 在 pass 里跳过（fail-closed，下一帧再试）。
        for item in &draws.draws {
            if texture_groups.contains_key(&(item.mask, item.main)) {
                continue;
            }
            let (Some(mask_image), Some(main_image)) =
                (images.get(item.mask), images.get(item.main))
            else {
                continue;
            };
            texture_groups.insert(
                (item.mask, item.main),
                device.create_bind_group(
                    "fixture_emission_texture_bind_group",
                    &texture_layout,
                    &BindGroupEntries::with_indices((
                        (
                            0u32,
                            BindingResource::Buffer(BufferBinding {
                                buffer: &env.buffer,
                                offset: 0,
                                size: None,
                            }),
                        ),
                        (1u32, &mask_image.texture_view),
                        (2u32, &main_image.texture_view),
                        (3u32, BindingResource::Sampler(&main_image.sampler)),
                        (4u32, BindingResource::Sampler(&mask_image.sampler)),
                    )),
                ),
            );
        }

        // 颜色 attachment：msaa 时画进多采样视图、store 时 resolve；单采样
        // 直写 resolved。无论有没有 draw 都清缓冲——不清会把上一帧的余像
        // 喂进泛光。
        let (color_view, resolve_target): (&TextureView, Option<&TextureView>) =
            match &target.msaa {
                Some(msaa_view) => (msaa_view, Some(&target.resolved)),
                None => (&target.resolved, None),
            };
        let mut pass = render_context
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("fixture_emission_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: resolve_target.map(|view| &**view),
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::BLACK.into()),
                        store: StoreOp::Store,
                    },
                })],
                // 深度：主 pass 写完之后的只读口——第一次 get_attachment
                // 已被主 pass 用掉（清 + 写），这里得 Load；不写所以 Discard。
                depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Discard)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

        for (index, item) in draws.draws.iter().enumerate() {
            let (Some(pipeline), Some(render_mesh)) =
                (resolve(item.pipeline), meshes.get(item.mesh))
            else {
                continue;
            };
            let Some(vertex_slice) = allocator.mesh_vertex_slice(&item.mesh) else {
                continue;
            };
            if !texture_groups.contains_key(&(item.mask, item.main)) {
                // 缺件的对在 pass 前建组时已被跳过——这里同样跳过 draw。
                continue;
            }
            let texture_bind_group = &texture_groups[&(item.mask, item.main)];
            pass.set_pipeline(pipeline);
            let object_offsets = [
                (index as u32) * gpu.object_stride,
                view_offset.offset,
                item.skin_byte_offset.unwrap_or(0),
            ];
            pass.set_bind_group(0, &object_bind_group, if gpu.skin_uniforms {
                &object_offsets[..]
            } else {
                &object_offsets[..2]
            });
            pass.set_bind_group(1, texture_bind_group, &[]);
            pass.set_vertex_buffer(0, *vertex_slice.buffer.slice(..));
            match &render_mesh.buffer_info {
                RenderMeshBufferInfo::Indexed {
                    index_format,
                    count,
                } => {
                    let Some(index_slice) = allocator.mesh_index_slice(&item.mesh) else {
                        continue;
                    };
                    pass.set_index_buffer(*index_slice.buffer.slice(..), *index_format);
                    pass.draw_indexed(
                        index_slice.range.start..(index_slice.range.start + *count),
                        vertex_slice.range.start as i32,
                        0..1,
                    );
                }
                RenderMeshBufferInfo::NonIndexed => {
                    pass.draw(vertex_slice.range, 0..1);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct EmissionPassLabel;

/// RenderStartup：对象池 buffer、两个 bind group 布局、采样器。
fn init_emission_resources(mut commands: Commands, render_device: Res<RenderDevice>) {
    let skin_uniforms = skins_use_uniform_buffers(&render_device.limits());
    // 动态 offset 槽距按设备对齐（通常 256；288 向上取整成 512）。
    let align = render_device.limits().min_uniform_buffer_offset_alignment.max(64) as u32;
    let stride = (EMISSION_OBJECT_BYTES as u32).div_ceil(align) * align;
    let object_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("fixture_emission_objects"),
        size: stride as u64 * OBJECT_CAPACITY as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // 逐对象 uniform：动态 offset，最小绑定尺寸 = 结构体大小。原始
    // BindGroupLayoutEntry 直接进 with_indices（自带 visibility 生效，
    // binding 序号由元组位置给出，条目上的序号字段被忽略——按约定填 MAX）。
    let object_layout = BindGroupLayoutDescriptor::new(
        "fixture_emission_object_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            (
                (
                    0u32,
                    BindGroupLayoutEntry {
                        binding: u32::MAX,
                        visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: Some(
                                std::num::NonZeroU64::new(EMISSION_OBJECT_BYTES as u64).unwrap(),
                            ),
                        },
                        count: None,
                    },
                ),
                (1u32, uniform_buffer::<ViewUniform>(true)),
                (2u32, BindGroupLayoutEntry {
                    binding: u32::MAX,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: if skin_uniforms { BufferBindingType::Uniform }
                            else { BufferBindingType::Storage { read_only: true } },
                        has_dynamic_offset: skin_uniforms,
                        min_binding_size: std::num::NonZeroU64::new(
                            if skin_uniforms { (MAX_JOINTS * 64) as u64 } else { 64 },
                        ),
                    },
                    count: None,
                }),
            ),
        ),
    );
    // 全局量按整段绑（368 字节 buffer 对 368 字节结构；最小尺寸不另设，
    // 家具主材质对同一块 buffer 同一形状）。遮罩/主贴图/采样器与主材质
    // 同参数。
    let texture_layout = BindGroupLayoutDescriptor::new(
        "fixture_emission_texture_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (
                    0u32,
                    BindGroupLayoutEntry {
                        binding: u32::MAX,
                        visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ),
                (1u32, texture_2d(TextureSampleType::Float { filterable: true })),
                (2u32, texture_2d(TextureSampleType::Float { filterable: true })),
                (3u32, sampler(SamplerBindingType::Filtering)),
                (4u32, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: FilterMode::Linear,
        ..Default::default()
    });
    commands.insert_resource(EmissionGpu {
        object_buffer,
        object_stride: stride,
        object_binding_size: EMISSION_OBJECT_BYTES as u64,
        object_layout,
        skin_uniforms,
        texture_layout,
        sampler,
    });
}

/// Startup：着色程序入表（阴影链同款形状）。
fn load(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        FIXTURE_EMISSION_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/fixture_emission.wgsl"),
            "moly_game/src/shaders/fixture_emission.wgsl".to_owned(),
        ),
    );
}

/// 家具自发光 pass 插件：native 与 wasm 两侧同装（换装侧在两侧都给实体
/// 插组件；wasm 分支漏挂会让第二目标在那侧永远空）。
pub struct FixtureEmissionPlugin;

impl Plugin for FixtureEmissionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<EmissionDrawList>()
            .init_resource::<EmissionPipelines>()
            .add_systems(RenderStartup, init_emission_resources)
            .add_systems(ExtractSchedule, extract_emission_draws)
            .add_systems(
                Render,
                prepare_emission.in_set(RenderSystems::PrepareResources),
            )
            .add_render_graph_node::<ViewNodeRunner<EmissionPassNode>>(Core3d, EmissionPassLabel)
            // 主 pass 之后（读它的深度图）、色调映射之前——天气链的合成
            // 节点在色调映射之后，传递序保证本 pass 先于泛光预滤波。
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::EndMainPass,
                    EmissionPassLabel,
                    Node3d::Tonemapping,
                ),
            );
    }
}
