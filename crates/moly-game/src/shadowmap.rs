//! 主光阴影深度图：单图路线（非级联、无图集）的生产侧。
//!
//! 一张光源向正交深度图（单图单矩阵，非级联、无图集），
//! Core3d 图内自建节点在主 pass 之前直画全部 caster；消费侧是四族站点
//! 材质（`shaders/site_material.wgsl` 的 DropShadow 律消费口，绑定
//! group 3 的 10/11/12）。节点与绘制范式：weather.rs 的 render graph
//! 节点 + bevy_pbr volumetric_fog 的 MeshAllocator 手画（0.18 registry
//! 逐一核对，非记忆）。
//!
//! # Caster 集
//!
//! 主世界全部带 [`Mesh3d`] 且层级启用的实体，除了：天空壳（巨大的相机跟随球，
//! 从光源看是一整圈挡板）、表情绘制位（逐帧重建的世界系 billboard，顶点
//! 属性即世界坐标，不承载投影语义）、蒙皮网格（角色/玩家——深度 pass
//! 不做蒙皮，绑定位姿深度是「错影」；缺影好过错影）、
//! 雨粒子（[`NoShadowCast`]，真源粒子不进主光 shadowmap）。
//!
//! # 数值口径（真源档已读出，本单替换自选值）
//!
//! 站点跑的是引擎资源里那份站点管线资产（游戏进站点时切换装上的那份，
//! 不是全局默认管线资产——默认那份主光渲染整个是关的），三值全部读自
//! 真源序列化档：主光影图 1024²、阴影距离 25 m、级联边距 0.1、主光
//! m_Shadows 强度 1（软影档）。`_MainLightShadowParams` 四分量按引擎
//! 装配式逐项可推导：x = 主光强度 1、y = 软影开 1（律式不读此槽，账面
//! 与源一致）、z/w = 距离 fade 两系数，按引擎线性距离 fade 算式从
//! 距离与边距现算（见 [`SHADOW_FADE_SCALE`] 的推导注释）。仍在自选的：
//! 光源正交框的前后垫量与光栅深度偏置（见 [`DEPTH_PAD`] 与管线
//! `DepthBiasState` 的注释）。
//!
//! # 阴影账目（本单）
//!
//! 「pass 没跑 / 跑了没画 / 图里没内容 / 图有内容但投影对不上」四态
//! 在画面上同形（无影且无错影），靠一行账目行分开：节点只拿 `&World`，
//! 计数走 [`ShadowAccount`] 的内部可变性；深度图定期读回（native 侧，
//! wasm 无阻塞 poll 只落计数），覆盖率、caster 原点对齐与单 texel 比较
//! 命中并排落日志，供验收逐值复算。
//!
//! # 帧内节奏
//!
//! Extract 抽 caster（含主世界网格的预计算包围盒）→ PrepareResources
//! 解出光源正交框、写三块 uniform、按顶点布局特化管线 → 节点在
//! EndPrepasses 与 StartMainPass 之间跑深度 pass（管线没编好或 caster
//! 为空的帧整段跳过）。

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Mutex;

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::math::bounding::Aabb3d;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{Node, NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel};
use bevy::render::render_resource::binding_types::uniform_buffer;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

use crate::emoticon::EmoteDraw;
use crate::env::SiteEnv;
use crate::fixture_material::WallLayoutShadowCasterOff;
use crate::sky::SkyDome;
use moly_assets::material_passes::SourceMaterialPasses;
use moly_assets::scene_state::{SourceRenderer, SourceShadowCastingMode};

/// 阴影链着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`（weather
/// 同款形状——仓内没有默认资产源目录）。
const SHADOW_DEPTH_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x6271_62d3_a2e2_4f19_b6c6_9f3e_51c8_0d44),
    PhantomData,
);

/// 深度图边长：站点管线资产的主光影图档 1024²（单级联单图，模块注释
/// 「数值口径」）。
pub const SHADOWMAP_SIZE: u32 = 1024;

/// 光源正交框的深度安全垫（世界单位）：近/远各外扩一档，避免贴边 caster
/// 被裁掉。本仓自选值。
const DEPTH_PAD: f32 = 1.0;

/// 阴影强度（`SiteShadow.params.x`）：站点环境主光的序列化 m_Shadows
/// 强度 1.0，软影档。
const SHADOW_STRENGTH: f32 = 1.0;

/// 软影槽（`params.y`）：站点管线资产开软影、环境主光同为软影，源装配
/// 值 1。律的 fade 式不读此槽，写 1 是账面与源一致。
const SHADOW_SOFT: f32 = 1.0;

/// 距离 fade 两系数（`params.z` / `params.w`）。按引擎线性距离 fade
/// 算式从两档现算：阴影距离 25 m（取平方 625）× 级联边距 0.1 →
/// 边带 (1−0.1)² = 0.81 → 近点 0.81·625 = 506.25、分母 625−506.25 =
/// 118.75，故 z = 1/118.75、w = −506.25/118.75。fade 在距相机
/// 22.5–25 m 间从 0 升到 1——站点相机钳在个位数米，恒不触发，两数
/// 是保真值不是病灶。
const SHADOW_FADE_SCALE: f32 = 1.0 / 118.75;
const SHADOW_FADE_BIAS: f32 = -506.25 / 118.75;

/// 阴影账目：节点首个读回推迟到第 60 个成图帧（场景装配与管线编译
/// 落定），此后每 600 帧（约 10 s）一次。
const PROBE_FIRST_FRAME: u64 = 60;
const PROBE_INTERVAL: u64 = 600;

/// 深度图读回里判「非空」的阈值：clear 是 1.0，光栅深度只会更小。
/// 只在 native 读回路径用（wasm 只落计数行，不读图）。
#[cfg(not(target_arch = "wasm32"))]
const PROBE_CLEAR_EPS: f32 = 0.9999;

/// 深度 pass 帧块的 GPU 字节数：两个 4×4（裁剪矩阵 + 采样矩阵）。
const FRAME_BYTES: usize = 128;

/// 消费块（`SiteShadow`）的 GPU 字节数：4×4 + 两个 vec4。
const CONSUMER_BYTES: usize = 96;

/// 逐实体矩阵池的容量上限；超出的 caster 丢弃并告警一次（站点量级远
/// 低于此，上限只是护栏）。
const OBJECT_CAPACITY: usize = 8192;

/// 不进主光深度图的实体标记。
#[derive(Component)]
pub struct NoShadowCast;

/// 深度 pass 顶点着色器的帧块：槽序与 `shaders/shadow_depth.wgsl` 的
/// `ShadowFrame` 是契约，两边同改。矩阵按列摊平（WGSL mat4x4 的内存序）。
#[derive(ShaderType)]
struct ShadowFrame {
    /// 深度 pass 的裁剪矩阵（正交，z 近 0 远 1）。
    light_view_proj: [Vec4; 4],
    /// 采样矩阵：xy 图内 uv（含 y 翻转），z 比较深度。
    world_to_shadow: [Vec4; 4],
}

fn mat4_cols(matrix: Mat4) -> [Vec4; 4] {
    let cols = matrix.to_cols_array();
    [
        Vec4::from_array(cols[0..4].try_into().unwrap()),
        Vec4::from_array(cols[4..8].try_into().unwrap()),
        Vec4::from_array(cols[8..12].try_into().unwrap()),
        Vec4::from_array(cols[12..16].try_into().unwrap()),
    ]
}

fn mat4_bytes(matrix: Mat4, bytes: &mut Vec<u8>) {
    for component in matrix.to_cols_array() {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
}

/// 消费块的 Rust 形：槽序与 `shaders/site_material.wgsl` 的 `SiteShadow`
/// 是契约，两边同改。摊平序 = 矩阵四列 + size + params。
struct ShadowConsumer {
    world_to_shadow: Mat4,
    /// (宽, 高, 1/宽, 1/高)。
    size: [f32; 4],
    /// `_MainLightShadowParams` 形状：.x 强度，.z 距离平方系数，.w fade
    /// 起点。强度 0 是恒等档（律的强度门把 atten 折成 1）。
    params: [f32; 4],
}

impl ShadowConsumer {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CONSUMER_BYTES);
        mat4_bytes(self.world_to_shadow, &mut bytes);
        for slot in [self.size, self.params] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), CONSUMER_BYTES);
        bytes
    }
}

/// 渲染世界的共享阴影资源：一次建，逐帧只写 buffer。
#[derive(Resource)]
pub struct ShadowMapGpu {
    /// 深度图本体：读回拷贝的源（账目行的覆盖率从这里量）。
    pub depth_texture: Texture,
    /// 深度图视图：深度 pass 的 attachment，站点材质的采样源。
    pub depth_view: TextureView,
    /// 比较采样器：NEAREST 单 texel 比较（源九 tap 是 lod0 单 texel 硬件
    /// 比较，不再叠引擎级滤波）；`LessEqual` = 存储深度 ≥ 片元深度时点亮
    /// （本图 z 从近到远递增，normal-Z）。
    pub cmp_sampler: Sampler,
    /// 消费块 buffer：全族一份，绑站点材质 group 3 binding 10。
    pub consumer_buffer: Buffer,
    /// 深度 pass 帧块 buffer。
    frame_buffer: Buffer,
    /// 逐实体世界矩阵池：动态 offset 绑定，槽距 [`ShadowMapGpu::object_stride`]。
    object_buffer: Buffer,
    /// 深度管线的 group 0 布局（描述符经 PipelineCache 缓存）。
    depth_layout: BindGroupLayoutDescriptor,
    /// 动态 offset 槽距：按设备对齐取，≥ 64 字节。
    object_stride: u32,
    /// 单个矩阵的绑定尺寸（动态 offset 校验用）。
    object_binding_size: u64,
}

/// 一条深度 draw：网格句柄 + 世界矩阵 + 该网格布局特化出的管线。
struct ShadowDraw {
    mesh: AssetId<Mesh>,
    world: Mat4,
    pipeline: CachedRenderPipelineId,
    two_sided: bool,
}

/// 抽取与解算结果：本帧的 caster 名单 + 内容世界包围盒。
#[derive(Resource, Default)]
struct ShadowDrawList {
    draws: Vec<ShadowDraw>,
    bounds: Option<(Vec3, Vec3)>,
}

/// 阴影账目（模块注释「阴影账目」）：渲染图节点只拿 `&World`，计数走
/// 内部可变性。锁的临界段都是几条赋值，渲染应用单线程，无重入。
#[derive(Resource, Default)]
struct ShadowAccount(Mutex<AccountInner>);

#[derive(Default)]
struct AccountInner {
    /// prepare 解出光源框（参数行有强度）的帧数。
    frames_prepared: u64,
    /// 节点实际录过 draw 的帧数。
    frames_drawn: u64,
    /// 因管线未编好整帧静默跳过的次数。
    frames_skipped_not_ready: u64,
    /// 节点跑了但一条 draw 都没录的帧数。
    frames_empty_pass: u64,
    /// 最近一帧 prepare 的 caster 名单数 / 其中布局取不出（INVALID）数。
    casters: usize,
    casters_invalid: usize,
    /// 最近一帧节点实际录进 pass 的 draw 数。
    draws_recorded: usize,
    /// 最近一次解出的光框几何与消费矩阵。
    light_box: Option<(f32, f32, f32)>,
    world_to_shadow: Option<Mat4>,
    /// 深度图读回缓冲（复用；native 侧诊断，wasm 不建）。
    #[cfg(not(target_arch = "wasm32"))]
    probe_buffer: Option<Buffer>,
    /// 有拷贝已录、下一帧节点头读回。
    probe_pending: bool,
    /// 下一次读回的帧序（frames_drawn 口径）。
    next_probe_frame: u64,
}

/// 管线特化键：同一顶点布局 + 图元拓扑共用一条深度管线。
#[derive(Clone, PartialEq, Eq, Hash)]
struct DepthPipelineKey {
    layout: MeshVertexBufferLayoutRef,
    topology: PrimitiveTopology,
    two_sided: bool,
}

fn depth_pipeline_key(mesh: &RenderMesh, two_sided: bool) -> DepthPipelineKey {
    DepthPipelineKey {
        layout: mesh.layout.clone(),
        topology: mesh.primitive_topology(),
        two_sided,
    }
}

/// 已入队的深度管线：键 → 缓存 id（id 在编译完成前取不到真管线）。
#[derive(Resource, Default)]
struct DepthPipelines {
    queued: HashMap<DepthPipelineKey, CachedRenderPipelineId>,
}

/// Extract：抽本帧 caster（含预计算包围盒）。蒙皮/天空/表情/雨在查询里
/// 排除（模块注释「Caster 集」）；包围盒按网格 id 缓存，首帧后零开销。
///
/// 层级显隐与取景剔除是两件事：光源能照到的物体即使在主相机画外，
/// 仍可能把影子投进画内。不能用 ViewVisibility 筛本名单，否则旋转相机
/// 会同时删改 caster 与光源正交框，使整张深度图的尺度、采样格和影子跳变。
/// InheritedVisibility 保留显式隐藏和父级显隐，不把相机视锥变成光源视锥。
#[allow(clippy::type_complexity)]
fn extract_shadow_casters(
    mut commands: Commands,
    casters: Extract<
        Query<
            (&Mesh3d, &GlobalTransform, &InheritedVisibility, Option<&SourceMaterialPasses>,
             Option<&SourceRenderer>, Option<&ChildOf>),
            (
                Without<SkyDome>,
                Without<EmoteDraw>,
                Without<SkinnedMesh>,
                Without<NoShadowCast>,
                Without<moly_assets::scene_state::SourceInactive>,
                Without<bevy::light::NotShadowCaster>,
                Without<WallLayoutShadowCasterOff>,
            ),
        >,
    >,
    parent_visibility: Extract<Query<&InheritedVisibility>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    mut local_bounds: Local<HashMap<AssetId<Mesh>, Aabb3d>>,
    mut overflow_warned: Local<bool>,
) {
    let mut draws: Vec<ShadowDraw> = Vec::new();
    local_bounds.retain(|id, _| meshes.contains(*id));
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for (mesh, transform, visibility, passes, renderer, parent) in &casters {
        if renderer.is_some_and(|renderer| !renderer.casts_shadows()) {
            continue;
        }
        // ShadowsOnly hides the primitive in colour, not the owning object's
        // hierarchy. Honour that object's inherited visibility and source
        // activity, without reviving disabled renderers or hidden ancestors.
        let visible_to_light = if renderer.is_some_and(SourceRenderer::shadows_only) {
            parent.and_then(|parent| parent_visibility.get(parent.parent()).ok())
                .is_some_and(|visibility| visibility.get())
        } else { visibility.get() };
        if !visible_to_light {
            continue;
        }
        // Colour-pass visibility is not permission to invent a ShadowCaster
        // pass. For example, ground decorations can draw alpha-cut contours
        // in colour while declaring no shadow pass at all. Use source tags,
        // not a shader-name exclusion list. Absent metadata remains a separate
        // extraction/consumer gap; do not interpret it as a known empty list.
        if passes.is_some_and(|passes| !passes.has_shadow_caster()) {
            continue;
        }
        let id = mesh.0.id();
        let local = local_bounds.entry(id).or_insert_with(|| {
            meshes
                .get(&mesh.0)
                .and_then(|mesh| mesh.final_aabb)
                .unwrap_or(Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO))
        });
        let world = transform.to_matrix();
        // 8 个角换到世界系并入总包围盒：光源正交框从这里解出，不发明场地尺寸。
        for sx in [local.min.x, local.max.x] {
            for sy in [local.min.y, local.max.y] {
                for sz in [local.min.z, local.max.z] {
                    let corner = world.transform_point3(Vec3::new(sx, sy, sz));
                    bounds = Some(match bounds {
                        Some((min, max)) => (min.min(corner), max.max(corner)),
                        None => (corner, corner),
                    });
                }
            }
        }
        draws.push(ShadowDraw {
            mesh: id,
            world,
            pipeline: CachedRenderPipelineId::INVALID,
            two_sided: renderer.is_some_and(|renderer|
                renderer.shadow_casting() == Some(SourceShadowCastingMode::TwoSided)),
        });
    }
    let overflow = draws.len() > OBJECT_CAPACITY;
    draws.truncate(OBJECT_CAPACITY);
    if overflow && !*overflow_warned {
        *overflow_warned = true;
        warn!(
            "主光阴影 caster 超过容量上限 {OBJECT_CAPACITY}，超出部分不投影（仅告警一次）"
        );
    }
    commands.insert_resource(ShadowDrawList { draws, bounds });
}

/// 光源向正交的一对矩阵：深度 pass 用裁剪矩阵，消费侧用采样矩阵
/// （xy 已折进 [0,1] 且带 y 翻转——WebGPU 帧缓冲原点在左上而 NDC y 向上，
/// 纹理 v=0 对应 NDC y=+1；z 是 [0,1] 比较深度，近 0 远 1，
/// `orthographic_rh` 的 z 语义恰是这一约定）。随矩阵带回框的几何三数
/// （光系宽/高/深度程，米），账目行用它折 texel 尺寸与深度差。
struct LightMatrices {
    clip: Mat4,
    consumer: Mat4,
    width: f32,
    height: f32,
    depth: f32,
}

fn light_matrices(env: &SiteEnv, bounds: (Vec3, Vec3)) -> Option<LightMatrices> {
    let to_light = Vec3::from_array(env.globals.light_vector).normalize_or_zero();
    if to_light == Vec3::ZERO {
        return None;
    }
    let center = (bounds.0 + bounds.1) * 0.5;
    // up 与光向近平行的档位换轴，避免 look_at 退化。
    let up = if to_light.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(center + to_light * 100.0, center, up);
    let mut light_min = Vec3::INFINITY;
    let mut light_max = Vec3::NEG_INFINITY;
    for sx in [bounds.0.x, bounds.1.x] {
        for sy in [bounds.0.y, bounds.1.y] {
            for sz in [bounds.0.z, bounds.1.z] {
                let corner = view.transform_point3(Vec3::new(sx, sy, sz));
                light_min = light_min.min(corner);
                light_max = light_max.max(corner);
            }
        }
    }
    let near = (-light_max.z - DEPTH_PAD).max(0.01);
    let far = -light_min.z + DEPTH_PAD;
    if far <= near || !light_max.x.is_finite() {
        return None;
    }
    let proj = Mat4::orthographic_rh(
        light_min.x,
        light_max.x,
        light_min.y,
        light_max.y,
        near,
        far,
    );
    let light_view_proj = proj * view;
    // uv 折算：u = 0.5x + 0.5，v = −0.5y + 0.5（y 翻转见上），z 直通。
    let to_uv = Mat4::from_cols_array(&[
        0.5, 0.0, 0.0, 0.0, //
        0.0, -0.5, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.5, 0.5, 0.0, 1.0,
    ]);
    Some(LightMatrices {
        clip: light_view_proj,
        consumer: to_uv * light_view_proj,
        width: light_max.x - light_min.x,
        height: light_max.y - light_min.y,
        depth: far - near,
    })
}

/// PrepareResources：特化管线、解光源正交框、写三块 uniform。
///
/// 挂 PrepareResources 而非 Prepare：与引擎的网格资产 prepare（Prepare 集）
/// 错开一拍，本帧的 `RenderAssets<RenderMesh>` 与 MeshAllocator 切片已定。
fn prepare_shadow_draws(
    mut draws: ResMut<ShadowDrawList>,
    mut pipelines: ResMut<DepthPipelines>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    env: Option<Res<SiteEnv>>,
    gpu: Res<ShadowMapGpu>,
    account: Res<ShadowAccount>,
    mut readiness_logged: Local<bool>,
    mut prepare_frames: Local<u64>,
) {
    // 矩阵池按 draw 序全量写（缺管线的 draw 也占位，动态 offset 才对得上）。
    // The dynamic binding reads at index * object_stride, not index * 64.
    // Pad every matrix, including unavailable meshes, to that same slot size.
    // Otherwise each draw after the first consumes another object's transform
    // (or unwritten/stale bytes) and casts a displaced or malformed silhouette.
    let stride = gpu.object_stride as usize;
    let mut object_bytes = Vec::with_capacity(draws.draws.len() * stride);
    for item in draws.draws.iter_mut() {
        let next_slot = object_bytes.len() + stride;
        mat4_bytes(item.world, &mut object_bytes);
        object_bytes.resize(next_slot, 0);
        let Some(render_mesh) = meshes.get(item.mesh) else {
            continue;
        };
        let key = depth_pipeline_key(render_mesh, item.two_sided);
        let topology = key.topology;
        item.pipeline = *pipelines.queued.entry(key).or_insert_with(|| {
            // 位置属性的偏移由网格布局自身给出；缺位置的网格按 INVALID
            // 记账，节点侧跳过。
            match render_mesh
                .layout
                .0
                .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])
            {
                Ok(vertex) => pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("site_shadow_depth_pipeline".into()),
                    layout: vec![gpu.depth_layout.clone()],
                    vertex: VertexState {
                        shader: SHADOW_DEPTH_SHADER.clone(),
                        shader_defs: vec![],
                        entry_point: Some("shadow_depth_vertex".into()),
                        buffers: vec![vertex],
                    },
                    fragment: None,
                    primitive: PrimitiveState {
                        topology,
                        front_face: FrontFace::Ccw,
                        cull_mode: if item.two_sided { None } else { Some(Face::Back) },
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        depth_write_enabled: true,
                        // normal-Z：保留离光最近（深度最小）的面。光栅偏置抵
                        // acne；源偏置值读不出，取正交常规档（模块注释）。
                        depth_compare: CompareFunction::Less,
                        stencil: StencilState::default(),
                        bias: DepthBiasState {
                            constant: 1,
                            slope_scale: 1.0,
                            clamp: 0.0,
                        },
                    }),
                    multisample: Default::default(),
                    ..Default::default()
                }),
                Err(_) => CachedRenderPipelineId::INVALID,
            }
        });
    }

    // 光源框与采样矩阵：现象/内容不齐的帧写强度 0——律的强度门把 atten
    // 恒等地折成 1，阴影整段无贡献（不是「跳过采样」的近似）。四分量按
    // 模块注释「数值口径」逐项取真源档：强度/软影/fade 现算见常量注释。
    let mut resolved: Option<LightMatrices> = None;
    let (frame, consumer) = match env
        .as_deref()
        .zip(draws.bounds)
        .and_then(|(env, bounds)| light_matrices(env, bounds))
    {
        Some(matrices) => {
            let clip = matrices.clip;
            let consumer_matrix = matrices.consumer;
            let _ = resolved.insert(matrices);
            (
                ShadowFrame {
                    light_view_proj: mat4_cols(clip),
                    world_to_shadow: mat4_cols(consumer_matrix),
                },
                ShadowConsumer {
                    world_to_shadow: consumer_matrix,
                    size: [
                        SHADOWMAP_SIZE as f32,
                        SHADOWMAP_SIZE as f32,
                        1.0 / SHADOWMAP_SIZE as f32,
                        1.0 / SHADOWMAP_SIZE as f32,
                    ],
                    params: [
                        SHADOW_STRENGTH,
                        SHADOW_SOFT,
                        SHADOW_FADE_SCALE,
                        SHADOW_FADE_BIAS,
                    ],
                },
            )
        }
        None => (
            ShadowFrame {
                light_view_proj: [Vec4::ZERO; 4],
                world_to_shadow: [Vec4::ZERO; 4],
            },
            ShadowConsumer {
                world_to_shadow: Mat4::ZERO,
                size: [0.0; 4],
                params: [0.0; 4],
            },
        ),
    };
    // 阴影账目：prepare 侧记帧数与名单（节点侧记执行与读回，模块注释
    // 「阴影账目」）。
    let invalid_now = draws
        .draws
        .iter()
        .filter(|item| item.pipeline == CachedRenderPipelineId::INVALID)
        .count();
    {
        let mut inner = account.0.lock().unwrap();
        inner.frames_prepared += resolved.is_some() as u64;
        inner.casters = draws.draws.len();
        inner.casters_invalid = invalid_now;
        if let Some(matrices) = &resolved {
            inner.light_box = Some((matrices.width, matrices.height, matrices.depth));
            inner.world_to_shadow = Some(matrices.consumer);
        }
    }
    let mut frame_bytes = Vec::with_capacity(FRAME_BYTES);
    for matrix in [frame.light_view_proj, frame.world_to_shadow] {
        for column in matrix {
            for component in column.to_array() {
                frame_bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
    }
    render_queue.write_buffer(&gpu.frame_buffer, 0, &frame_bytes);
    render_queue.write_buffer(&gpu.consumer_buffer, 0, &consumer.bytes());
    if !object_bytes.is_empty() {
        render_queue.write_buffer(&gpu.object_buffer, 0, &object_bytes);
    }
    // 深度图生成行（一次）：出现即代表光源框解出、矩阵与逐实体矩阵池已
    // 入 GPU，节点将在主 pass 前成图；消费行由站点材质绑定 10/11/12 的
    // 换装日志（「材质换装：…四族…」）共同推导。参数行逐值可推导：fade
    // 两数 = 1/118.75 与 −506.25/118.75，由距离 25 m、级联边距 0.1 按引
    // 擎线性距离 fade 算式现算（模块注释「数值口径」）。
    if !*readiness_logged && consumer.params[0] > 0.0 && !draws.draws.is_empty() {
        *readiness_logged = true;
        info!(
            "主光阴影深度图就绪：{}²（单图单矩阵，光源向正交，站点管线档）、caster {} 个；参数 [强度 {}、软 {}、fade {}/{}]（距离 25 m、边距 0.1 现算：1/118.75 与 −506.25/118.75）；消费侧站点四族已接（binding 10 消费块 / 11 深度图 / 12 比较采样器，pcf9 采样口）",
            SHADOWMAP_SIZE,
            draws.draws.len(),
            SHADOW_STRENGTH,
            SHADOW_SOFT,
            SHADOW_FADE_SCALE,
            SHADOW_FADE_BIAS,
        );
    }
    // 节点死锁检测（账目兜底）：pass 一次都没跑过而 prepare 已进行很久，
    // 账目行从 prepare 侧落一次计数，避免「图就绪」行与实际 pass 之间的
    // 缺口无人喊。
    *prepare_frames += 1;
    if [60u64, 600, 1800].contains(&*prepare_frames) {
        let drawn = account.0.lock().unwrap().frames_drawn;
        if drawn == 0 {
            info!(
                "主光阴影账目（prepare 视角，第 {} 帧）：pass 仍未执行（frames_drawn 0）——节点没跑或整帧静默跳过，深度图恒空；caster {} 个（布局取不出 {invalid_now}）",
                *prepare_frames,
                draws.draws.len(),
            );
        }
    }
}

/// 深度 pass 节点：一次成图，跨视图共享（站点场景只有一台 3D 相机；
/// 光源深度本就不依赖取景，多相机也不需要每视图一份）。
struct ShadowDepthNode;

impl FromWorld for ShadowDepthNode {
    fn from_world(_world: &mut World) -> Self {
        Self
    }
}

impl Node for ShadowDepthNode {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        // 上一帧的深度图读回先处理（native 诊断；wasm 无阻塞 poll 不做，
        // 模块注释「阴影账目」）：拷贝已随上一帧提交，poll 等完成即 map。
        #[cfg(not(target_arch = "wasm32"))]
        finish_pending_probe(world, render_context);

        let draws = world.resource::<ShadowDrawList>();
        if draws.draws.is_empty() {
            return Ok(());
        }
        let gpu = world.resource::<ShadowMapGpu>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let meshes = world.resource::<RenderAssets<RenderMesh>>();
        let allocator = world.resource::<MeshAllocator>();

        // 汇总用到的管线：任一没编完就整帧跳过（weather 同款——首几帧空过）。
        let mut ready: Vec<(CachedRenderPipelineId, &RenderPipeline)> = Vec::new();
        for item in &draws.draws {
            if item.pipeline == CachedRenderPipelineId::INVALID
                || ready.iter().any(|(id, _)| *id == item.pipeline)
            {
                continue;
            }
            let Some(pipeline) = pipeline_cache.get_render_pipeline(item.pipeline) else {
                world
                    .resource::<ShadowAccount>()
                    .0
                    .lock()
                    .unwrap()
                    .frames_skipped_not_ready += 1;
                return Ok(());
            };
            ready.push((item.pipeline, pipeline));
        }
        let resolve = |id: CachedRenderPipelineId| -> Option<&RenderPipeline> {
            ready
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, pipeline)| *pipeline)
        };

        // 帧块 + 矩阵池一个 bind group；逐 draw 用动态 offset 换窗。
        let bind_group = render_context.render_device().create_bind_group(
            "site_shadow_depth_bind_group",
            &pipeline_cache.get_bind_group_layout(&gpu.depth_layout),
            &BindGroupEntries::with_indices((
                (
                    0u32,
                    BindingResource::Buffer(BufferBinding {
                        buffer: &gpu.frame_buffer,
                        offset: 0,
                        size: None,
                    }),
                ),
                (
                    1u32,
                    BindingResource::Buffer(BufferBinding {
                        buffer: &gpu.object_buffer,
                        offset: 0,
                        size: Some(std::num::NonZeroU64::new(gpu.object_binding_size).unwrap()),
                    }),
                ),
            )),
        );

        let mut pass = render_context
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("site_shadow_depth_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

        let mut drawn: usize = 0;
        for (index, item) in draws.draws.iter().enumerate() {
            let (Some(pipeline), Some(render_mesh)) =
                (resolve(item.pipeline), meshes.get(item.mesh))
            else {
                continue;
            };
            let Some(vertex_slice) = allocator.mesh_vertex_slice(&item.mesh) else {
                continue;
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[(index as u32) * gpu.object_stride]);
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
            drawn += 1;
        }
        drop(pass);

        // 阴影账目（节点侧）：帧数与实录 draw 数；到档的帧顺手录一次深度
        // 图拷贝，下一帧节点头读回落账（native；wasm 只落计数行）。
        let probe_due = {
            let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
            inner.frames_drawn += 1;
            inner.draws_recorded = drawn;
            if drawn == 0 {
                inner.frames_empty_pass += 1;
            }
            frames_drawn_due(&mut inner)
        };
        if probe_due && drawn > 0 {
            #[cfg(not(target_arch = "wasm32"))]
            record_depth_probe(world, render_context);
            #[cfg(target_arch = "wasm32")]
            log_counter_line(world);
        }
        Ok(())
    }
}

/// 读回档期判定与顺延（锁内调用）：首个读回在第 [`PROBE_FIRST_FRAME`]
/// 个成图帧，此后每 [`PROBE_INTERVAL`] 帧一次。0 是未初始化档期的哨兵。
fn frames_drawn_due(inner: &mut AccountInner) -> bool {
    if inner.probe_pending {
        return false;
    }
    if inner.next_probe_frame == 0 {
        inner.next_probe_frame = PROBE_FIRST_FRAME;
    }
    if inner.frames_drawn >= inner.next_probe_frame {
        inner.next_probe_frame = inner.frames_drawn + PROBE_INTERVAL;
        true
    } else {
        false
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ShadowDepthLabel;

/// 录一次深度图读回（native 账目）：深度 pass 之后在同一 encoder 上把
/// 整张图拷进复用 buffer，标记 pending，下一帧节点头 [`finish_pending_probe`]
/// 读回落账。
#[cfg(not(target_arch = "wasm32"))]
fn record_depth_probe(world: &World, render_context: &mut RenderContext) {
    let gpu = world.resource::<ShadowMapGpu>();
    let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    let buffer = inner
        .probe_buffer
        .get_or_insert_with(|| {
            render_context.render_device().create_buffer(&BufferDescriptor {
                label: Some("site_shadow_probe_readback"),
                size: (SHADOWMAP_SIZE as u64) * (SHADOWMAP_SIZE as u64) * 4,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .clone();
    render_context
        .command_encoder()
        .copy_texture_to_buffer(
            gpu.depth_texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SHADOWMAP_SIZE * 4),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: SHADOWMAP_SIZE,
                height: SHADOWMAP_SIZE,
                depth_or_array_layers: 1,
            },
        );
    inner.probe_pending = true;
}

/// 读回落账（native 账目）：map + 阻塞 poll 等拷贝完成，随后把
/// 「pass 跑没跑、图里有没有内容、内容对不对得上 caster、单 texel 比较
/// 命中分布」四件事并排落一行。四态在画面上同形（模块注释「阴影账目」），
/// 行里的数逐一区分它们：
/// - pass 没跑：`drawn` 帧数 0 / `draw` 实录 0；
/// - 图里没内容：`非清` texel 数 0；
/// - 投影对不上：`出框` 原点数高或 `邻域有图` 低（矩阵/框错位）；
/// - 比较全亮：`lit vs 影` 里影一路为 0（偏置或 z 折叠错向）。
#[cfg(not(target_arch = "wasm32"))]
fn finish_pending_probe(world: &World, render_context: &RenderContext) {
    let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    if !inner.probe_pending {
        return;
    }
    inner.probe_pending = false;
    let Some(buffer) = inner.probe_buffer.clone() else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    if render_context
        .render_device()
        .poll(PollType::wait_indefinitely())
        .is_err()
    {
        warn!("主光阴影账目：深度图读回 poll 失败，本次读回作废");
        return;
    }
    if !matches!(receiver.recv(), Ok(Ok(()))) {
        warn!("主光阴影账目：深度图读回 map 失败，本次读回作废");
        return;
    }

    // 深度统计：非清 texel 数（clear 是 1.0，光栅只会更小）与最小深度。
    let data = buffer.slice(..).get_mapped_range();
    let texel = |x: usize, y: usize| -> f32 {
        let i = (y * SHADOWMAP_SIZE as usize + x) * 4;
        f32::from_le_bytes(data[i..i + 4].try_into().unwrap())
    };
    let mut nonclear = 0usize;
    let mut min_depth = 1.0f32;
    for chunk in data.chunks_exact(4) {
        let d = f32::from_le_bytes(chunk.try_into().unwrap());
        if d < PROBE_CLEAR_EPS {
            nonclear += 1;
            min_depth = min_depth.min(d);
        }
    }

    // caster 原点对齐：把每个 caster 的世界原点过消费矩阵（和站点材质同
    // 一条路），看图里有没有它、单 texel 比较哪边亮。名单是本帧的，矩阵
    // 是成图那一帧的——站点场景静态，跨帧差可忽略。
    let (mut origins, mut out_of_range, mut near_nonclear) = (0usize, 0usize, 0usize);
    let (mut lit, mut shadowed, mut max_gap) = (0usize, 0usize, 0.0f32);
    if let Some(world_to_shadow) = inner.world_to_shadow {
        let draws = world.resource::<ShadowDrawList>();
        for item in &draws.draws {
            let clip = world_to_shadow.transform_point3(item.world.w_axis.truncate());
            origins += 1;
            if !(0.0..1.0).contains(&clip.x)
                || !(0.0..1.0).contains(&clip.y)
                || !(0.0..1.0).contains(&clip.z)
            {
                out_of_range += 1;
            }
            let size = SHADOWMAP_SIZE as i64;
            let tx = (clip.x * size as f32) as i64;
            let ty = (clip.y * size as f32) as i64;
            let tx = tx.clamp(0, size - 1) as usize;
            let ty = ty.clamp(0, size - 1) as usize;
            let mut hit = false;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (x, y) = (tx as i64 + dx, ty as i64 + dy);
                    if x >= 0 && y >= 0 && x < size && y < size {
                        hit |= texel(x as usize, y as usize) < PROBE_CLEAR_EPS;
                    }
                }
            }
            near_nonclear += hit as usize;
            // 单 texel 比较，方向与消费口一致：存储深度 ≥ 原点深度 → 点亮。
            let stored = texel(tx, ty);
            if clip.z <= stored {
                lit += 1;
            } else {
                shadowed += 1;
            }
            max_gap = max_gap.max((stored - clip.z).abs());
        }
    }
    let (box_w, box_h, box_d) = inner.light_box.unwrap_or((0.0, 0.0, 0.0));
    let total_texels = (SHADOWMAP_SIZE * SHADOWMAP_SIZE) as f32;
    drop(data);
    buffer.unmap();
    info!(
        "主光阴影账目：帧 prepared {} / drawn {}（静默跳过 {}、空过 {}）；caster {}（布局取不出 {}，实录 draw {}）；光框 {box_w:.1}×{box_h:.1} m 深 {box_d:.1} m（texel {:.2} cm）；深度图非清 {} texel（{:.2}%）、最小深度 {min_depth:.4}；caster 原点：投影 {origins}、出框 {out_of_range}、邻域有图 {near_nonclear}；单 texel 比较 lit {lit} vs 影 {shadowed}、最大深度差 {max_gap:.4}",
        inner.frames_prepared,
        inner.frames_drawn,
        inner.frames_skipped_not_ready,
        inner.frames_empty_pass,
        inner.casters,
        inner.casters_invalid,
        inner.draws_recorded,
        box_w * 100.0 / SHADOWMAP_SIZE as f32,
        nonclear,
        nonclear as f32 * 100.0 / total_texels,
    );
}

/// wasm 侧账目行（无阻塞读回）：只落计数，四态里能区分「跑没跑 / 画没
/// 画」，图内容与对齐在 native 侧验。
#[cfg(target_arch = "wasm32")]
fn log_counter_line(world: &World) {
    let inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    info!(
        "主光阴影账目（wasm 计数行）：帧 prepared {} / drawn {}（静默跳过 {}、空过 {}）；caster {}（布局取不出 {}，实录 draw {}）",
        inner.frames_prepared,
        inner.frames_drawn,
        inner.frames_skipped_not_ready,
        inner.frames_empty_pass,
        inner.casters,
        inner.casters_invalid,
        inner.draws_recorded,
    );
}

/// 渲染启动：深度图、比较采样器、三块 buffer、group 0 布局。
fn init_shadow_resources(mut commands: Commands, render_device: Res<RenderDevice>) {
    let texture = render_device.create_texture(&TextureDescriptor {
        label: Some("site_shadow_depth_texture"),
        size: Extent3d {
            width: SHADOWMAP_SIZE,
            height: SHADOWMAP_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_view = texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2),
        ..Default::default()
    });
    let cmp_sampler = render_device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mipmap_filter: FilterMode::Nearest,
        compare: Some(CompareFunction::LessEqual),
        ..Default::default()
    });
    let consumer_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_consumer"),
        size: CONSUMER_BYTES as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let frame_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_frame"),
        size: FRAME_BYTES as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // 动态 offset 槽距按设备对齐（通常 256）。
    let align = render_device.limits().min_uniform_buffer_offset_alignment.max(64) as u32;
    let object_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_objects"),
        size: (align as u64) * (OBJECT_CAPACITY as u64),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let depth_layout = BindGroupLayoutDescriptor::new(
        "site_shadow_depth_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX,
            (
                (0, uniform_buffer::<ShadowFrame>(false)),
                (1, uniform_buffer::<[Vec4; 4]>(true)),
            ),
        ),
    );
    commands.insert_resource(ShadowMapGpu {
        depth_texture: texture,
        depth_view,
        cmp_sampler,
        consumer_buffer,
        frame_buffer,
        object_buffer,
        object_stride: align,
        object_binding_size: std::mem::size_of::<[Vec4; 4]>() as u64,
        depth_layout,
    });
}

/// Startup：着色程序入表（weather 同款形状）。
fn load(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        SHADOW_DEPTH_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/shadow_depth.wgsl"),
            "moly_game/src/shaders/shadow_depth.wgsl".to_owned(),
        ),
    );
}

/// 主光阴影深度图插件：消费面（站点材质）在 native 与 wasm 都在，两侧都装
/// ——此前 wasm 分支的装配缺口（sky/站点材质漏挂）不在这里重演。
pub struct ShadowmapPlugin;

impl Plugin for ShadowmapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ShadowDrawList>()
            .init_resource::<DepthPipelines>()
            .init_resource::<ShadowAccount>()
            .add_systems(RenderStartup, init_shadow_resources)
            .add_systems(ExtractSchedule, extract_shadow_casters)
            .add_systems(
                Render,
                prepare_shadow_draws.in_set(RenderSystems::PrepareResources),
            )
            .add_render_graph_node::<ShadowDepthNode>(Core3d, ShadowDepthLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::EndPrepasses,
                    ShadowDepthLabel,
                    Node3d::StartMainPass,
                ),
            );
    }
}
