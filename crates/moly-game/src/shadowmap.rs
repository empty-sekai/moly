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
//! 属性即世界坐标，不承载投影语义）、
//! 雨粒子（[`NoShadowCast`]，真源粒子不进主光 shadowmap）。
//!
//! Skinned meshes take a second route. A skinned mesh casts only when it is a
//! character renderer: an NPC mesh with the character material or a mesh of
//! the player's body with the avatar material. It is drawn with the joint
//! matrices of the current frame (each joint's world transform times its
//! inverse bind pose, the same palette the colour pass skins with), so the
//! shadow follows the playing animation. The rules are the source's:
//! - the player's body renderer comes from the avatar prefab, whose
//!   serialized cast-shadows mode is On, and nothing at runtime changes it;
//! - every NPC renderer is set On each frame, except Off while the NPC's
//!   dither alpha is exactly 0 (read here from the material the dither writes);
//! - the NPC accessory program declares no ShadowCaster pass, so its meshes
//!   carry [`NoShadowCast`];
//! - the character ShadowCaster programs cull nothing (their cull state is
//!   bound to a property no character material or global sets, which leaves it
//!   Off), move each vertex by the pipeline's caster bias along the light and
//!   along the normal, clamp to the near plane, and are drawn under the
//!   pipeline's global raster bias of the shadow slice (see
//!   [`SHADOW_SLICE_RASTER_BIAS`]).
//! Other skinned meshes (skinned fixtures) stay out, as before, and are counted.
//!
//! The rigid casters carry the same source bias: every site program's
//! ShadowCaster pass (field object, tree, ground, water, fixture, object,
//! fence, canvas) runs the same vertex bias along the light and along the
//! normal, with the same near-plane clamp, under the same raster bias of the
//! shadow slice. A rigid mesh without a normal stream gets the bias along
//! the light only, and is counted in the account line.
//!
//! # 数值口径（真源档已读出，本单替换自选值）
//!
//! 站点跑的是引擎资源里那份站点管线资产（游戏进站点时切换装上的那份，
//! 不是全局默认管线资产——默认那份主光渲染整个是关的），三值全部读自
//! 真源序列化档：主光影图 1024²、阴影距离 25 m、级联边距 0.1、主光
//! m_Shadows 强度 1（软影档）。`_MainLightShadowParams` 四分量按引擎
//! 装配式逐项可推导：x = 主光强度 1、y = 软影开 1（律式不读此槽，账面
//! 与源一致）、z/w = 距离 fade 两系数，按引擎线性距离 fade 算式从
//! 距离与边距现算（见 [`SHADOW_FADE_SCALE`] 的推导注释）。Caster 偏置同样
//! 读自真源：管线资产的深度/法线偏置 1.0/1.0 × texel × 软影核 2.5（顶点侧，
//! 沿光与沿法线），与阴影切片的全局光栅偏置 1 / 2.5
//! （[`SHADOW_SLICE_RASTER_BIAS`]）。仍在自选的：光源正交框的前后垫量
//! （见 [`DEPTH_PAD`]），以及 texel 的口径（本图的光框宽 / 1024，不是源
//! 级联投影的宽）。
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
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{Node, NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel};
use bevy::render::render_resource::binding_types::{uniform_buffer, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

use crate::avatar_material::AvatarMaterial;
use crate::character_material::CharacterMaterial;
use crate::emoticon::EmoteDraw;
use crate::env::SiteEnv;
use crate::render::gpu::{Bound, SharedBindGroupCache};
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

/// The read-back interval: `MOLY_SHADOW_PROBE_INTERVAL` (frames, at least 1)
/// overrides [`PROBE_INTERVAL`] for a diagnosis run; read once.
fn probe_interval() -> u64 {
    static INTERVAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *INTERVAL.get_or_init(|| {
        std::env::var("MOLY_SHADOW_PROBE_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(PROBE_INTERVAL)
    })
}

/// 深度图读回里判「非空」的阈值：clear 是 1.0，光栅深度只会更小。
/// 只在 native 读回路径用（wasm 只落计数行，不读图）。
#[cfg(not(target_arch = "wasm32"))]
const PROBE_CLEAR_EPS: f32 = 0.9999;

/// 深度 pass 帧块的 GPU 字节数：两个 4×4（裁剪矩阵 + 采样矩阵）+ 两个
/// vec4（caster 偏置 + 向光方向）。
const FRAME_BYTES: usize = 160;

/// The site pipeline asset's shadow depth bias and normal bias (both 1.0).
/// The main light's caster bias takes them when the light defers to the
/// pipeline settings, which is the light data's default; the site light's own
/// light data is not read here.
const PIPELINE_SHADOW_DEPTH_BIAS: f32 = 1.0;
const PIPELINE_SHADOW_NORMAL_BIAS: f32 = 1.0;

/// The soft-shadow kernel radius the pipeline scales both caster biases by
/// when soft shadows are supported and the light is soft: 2.5 for the medium
/// quality, which is also the value when the light defers to the pipeline.
const SOFT_SHADOW_KERNEL_RADIUS: f32 = 2.5;

/// The raster bias the pipeline sets globally around the shadow slice's draw
/// (units 1.0, slope 2.5). No ShadowCaster pass of the site or character
/// programs declares an offset of its own, so this is every caster's whole
/// raster bias.
const SHADOW_SLICE_RASTER_BIAS: DepthBiasState = DepthBiasState {
    constant: 1,
    slope_scale: 2.5,
    clamp: 0.0,
};

/// Joints one palette window holds (the engine's skinning limit), and the
/// window's byte size: the dynamic uniform binding of the skinned entry point.
pub(crate) const MAX_JOINTS: usize = 256;
pub(crate) const PALETTE_WINDOW: u64 = (MAX_JOINTS * 64) as u64;

/// Palettes start at this alignment (the dynamic-offset alignment that every
/// device supports).
pub(crate) const PALETTE_ALIGN: usize = 256;

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
    /// The caster bias in world units: x along the light, y along
    /// the normal (both negative, as the pipeline computes them).
    shadow_bias: Vec4,
    /// xyz: the direction towards the light.
    light_direction: Vec4,
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
    /// The skinned entry point's group 1 layout: one palette window, bound at
    /// a dynamic offset.
    skin_layout: BindGroupLayoutDescriptor,
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

/// One skinned depth draw: a character renderer and the byte offset of its
/// joint palette (this frame's) in the palette buffer.
struct SkinnedShadowDraw {
    mesh: AssetId<Mesh>,
    palette_offset: u32,
    pipeline: CachedRenderPipelineId,
    /// Whether the renderer is a mesh of the player's body.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    player: bool,
    /// World bounds of the skinned vertices: the mesh's bind-pose box carried
    /// by every joint matrix of its palette (a superset of every blend). Read
    /// by the read-back's player window only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    bounds: (Vec3, Vec3),
}

/// Skinned-caster tallies of one extraction, for the account line.
#[derive(Default, Clone, Copy)]
struct SkinnedTally {
    player: usize,
    npc: usize,
    /// NPC renderers set Off because the NPC's dither alpha is 0.
    npc_off_by_dither: usize,
    /// Skinned meshes that are no character renderer (not casting).
    other_skinned: usize,
    palettes: usize,
    joints: usize,
}

/// 抽取与解算结果：本帧的 caster 名单 + 内容世界包围盒。
#[derive(Resource, Default)]
struct ShadowDrawList {
    draws: Vec<ShadowDraw>,
    bounds: Option<(Vec3, Vec3)>,
    /// Whether this extraction's draws differ from the previous one's in any
    /// mesh, world matrix bit, winding flag or count. The object matrix pool
    /// holds the previous draws' matrices, so it is rewritten only then.
    changed: bool,
    /// The skinned character casters of this extraction.
    skinned: Vec<SkinnedShadowDraw>,
    /// Their joint palettes, each at a [`PALETTE_ALIGN`]-aligned offset.
    palette_bytes: Vec<u8>,
    skinned_tally: SkinnedTally,
}

/// The world-space bounds corners of each draw in [`ShadowDrawList`], at the
/// same index as the draw.
#[derive(Default)]
struct CasterCorners(Vec<[Vec3; 8]>);

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
    /// The latest prepare's skinned casters, those whose layout has no
    /// position/normal/joint streams, and the skinned draws last recorded.
    skinned_casters: usize,
    skinned_invalid: usize,
    skinned_recorded: usize,
    skinned_tally: SkinnedTally,
    /// 最近一次解出的光框几何与消费矩阵。
    light_box: Option<(f32, f32, f32)>,
    /// The latest caster bias written: texel size, depth bias and normal
    /// bias, in metres.
    caster_bias: Option<(f32, f32, f32)>,
    /// The latest prepare's rigid casters whose mesh has no normal stream.
    rigid_without_normals: usize,
    world_to_shadow: Option<Mat4>,
    /// 深度图读回缓冲（复用；native 侧诊断，wasm 不建）。
    #[cfg(not(target_arch = "wasm32"))]
    probe_buffer: Option<Buffer>,
    /// 有拷贝已录、下一帧节点头读回。
    probe_pending: bool,
    /// 下一次读回的帧序（frames_drawn 口径）。
    next_probe_frame: u64,
    /// The previous read-back's texels and player window (native diagnosis).
    #[cfg(not(target_arch = "wasm32"))]
    previous_map: Option<Vec<u8>>,
    #[cfg(not(target_arch = "wasm32"))]
    previous_window: Option<(usize, usize, usize, usize)>,
}

/// The joint palette buffer of the skinned casters: replaced by a larger one
/// when a frame's palettes do not fit, so the node's bind group is looked up
/// by this buffer's id.
#[derive(Resource)]
struct ShadowPaletteGpu {
    buffer: Buffer,
    capacity: u64,
}

/// 管线特化键：同一顶点布局 + 图元拓扑共用一条深度管线。
#[derive(Clone, PartialEq, Eq, Hash)]
struct DepthPipelineKey {
    layout: MeshVertexBufferLayoutRef,
    topology: PrimitiveTopology,
    two_sided: bool,
    /// The skinned character entry point instead of the rigid one.
    skinned: bool,
    /// The mesh has a normal stream (the rigid entry point's normal bias).
    normals: bool,
}

fn depth_pipeline_key(mesh: &RenderMesh, two_sided: bool, skinned: bool) -> DepthPipelineKey {
    DepthPipelineKey {
        layout: mesh.layout.clone(),
        topology: mesh.primitive_topology(),
        two_sided,
        skinned,
        normals: mesh.layout.0.contains(Mesh::ATTRIBUTE_NORMAL),
    }
}

/// 已入队的深度管线：键 → 缓存 id（id 在编译完成前取不到真管线）。
#[derive(Resource, Default)]
struct DepthPipelines {
    queued: HashMap<DepthPipelineKey, CachedRenderPipelineId>,
}

/// Extract：抽本帧 caster（含预计算包围盒）。蒙皮/天空/表情/雨在查询里
/// 排除（模块注释「Caster 集」）；包围盒按网格 id 缓存，首帧后零开销。
/// The list is updated in place. A draw's corners depend only on its world
/// matrix and its mesh's cached local bounds; when the previous extraction's
/// draw at the same index had the same mesh and world matrix bits, and those
/// bounds were not dropped since, its corners are taken as they are. The
/// total bounds are folded over every caster's corners in query order, as
/// before, so the list and the bounds come out bit for bit the same.
///
/// 层级显隐与取景剔除是两件事：光源能照到的物体即使在主相机画外，
/// 仍可能把影子投进画内。不能用 ViewVisibility 筛本名单，否则旋转相机
/// 会同时删改 caster 与光源正交框，使整张深度图的尺度、采样格和影子跳变。
/// InheritedVisibility 保留显式隐藏和父级显隐，不把相机视锥变成光源视锥。
#[allow(clippy::type_complexity)]
fn extract_shadow_casters(
    mut list: ResMut<ShadowDrawList>,
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
    mut corners_of: Local<CasterCorners>,
    mut overflow_warned: Local<bool>,
) {
    let list = &mut *list;
    let stored = &mut corners_of.0;
    let previous = list.draws.len();
    // Bounds of meshes that left the asset store are dropped; a draw of such
    // a mesh is recomputed from the bounds its first sighting now caches.
    let mut dropped: Vec<AssetId<Mesh>> = Vec::new();
    local_bounds.retain(|id, _| {
        let keep = meshes.contains(*id);
        if !keep {
            dropped.push(*id);
        }
        keep
    });
    let mut changed = false;
    let mut count = 0usize;
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
        let two_sided = renderer.is_some_and(|renderer|
            renderer.shadow_casting() == Some(SourceShadowCastingMode::TwoSided));
        let world = transform.to_matrix();
        let slot = count;
        count += 1;
        let before = list.draws.get_mut(slot).filter(|_| slot < stored.len());
        let same_world = before.as_ref().is_some_and(|draw| {
            draw.world.to_cols_array().map(f32::to_bits) == world.to_cols_array().map(f32::to_bits)
        });
        let reused = same_world
            && before.as_ref().is_some_and(|draw| draw.mesh == id)
            && !dropped.contains(&id);
        let corners = if reused {
            let draw = before.expect("reused draw exists");
            draw.pipeline = CachedRenderPipelineId::INVALID;
            if draw.two_sided != two_sided {
                draw.two_sided = two_sided;
                changed = true;
            }
            stored[slot]
        } else {
            let local = local_bounds.entry(id).or_insert_with(|| {
                meshes
                    .get(&mesh.0)
                    .and_then(|mesh| mesh.final_aabb)
                    .unwrap_or(Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO))
            });
            let mut corners = [Vec3::ZERO; 8];
            let mut index = 0;
            for sx in [local.min.x, local.max.x] {
                for sy in [local.min.y, local.max.y] {
                    for sz in [local.min.z, local.max.z] {
                        corners[index] = world.transform_point3(Vec3::new(sx, sy, sz));
                        index += 1;
                    }
                }
            }
            if slot < OBJECT_CAPACITY {
                let draw = ShadowDraw {
                    mesh: id,
                    world,
                    pipeline: CachedRenderPipelineId::INVALID,
                    two_sided,
                };
                match before {
                    Some(old) => {
                        changed |= !same_world || old.mesh != id || old.two_sided != two_sided;
                        *old = draw;
                        stored[slot] = corners;
                    }
                    None => {
                        changed = true;
                        list.draws.truncate(slot);
                        stored.truncate(slot);
                        list.draws.push(draw);
                        stored.push(corners);
                    }
                }
            }
            corners
        };
        // 8 个角换到世界系并入总包围盒：光源正交框从这里解出，不发明场地尺寸。
        for corner in corners {
            bounds = Some(match bounds {
                Some((min, max)) => (min.min(corner), max.max(corner)),
                None => (corner, corner),
            });
        }
    }
    if count > OBJECT_CAPACITY && !*overflow_warned {
        *overflow_warned = true;
        warn!(
            "主光阴影 caster 超过容量上限 {OBJECT_CAPACITY}，超出部分不投影（仅告警一次）"
        );
    }
    let kept = count.min(OBJECT_CAPACITY);
    changed |= kept != previous;
    list.draws.truncate(kept);
    stored.truncate(kept);
    list.bounds = bounds;
    list.changed = changed;
}

/// Extract, after [`extract_shadow_casters`]: the skinned character casters
/// (module comment, "Caster 集") and their joint palettes of this frame.
///
/// The light box stays the rigid casters' box. A skinned draw's box here is
/// conservative (every joint's image of the bind-pose box), so folding it in
/// would move the light box, and with it every texel of the map, whenever a
/// character animates. The characters stand inside the site's world box, which
/// the rigid box contains; a vertex nearer than the near plane is clamped to it
/// by the skinned entry point.
///
/// Renderers that share one skin (the same joints and inverse bind poses)
/// share one palette. A palette is the joint's global transform times its
/// inverse bind pose, joint by joint, exactly as the colour pass builds it.
#[allow(clippy::type_complexity)]
fn extract_skinned_casters(
    mut list: ResMut<ShadowDrawList>,
    skinned: Extract<
        Query<
            (
                &Mesh3d,
                &SkinnedMesh,
                &InheritedVisibility,
                Option<&MeshMaterial3d<CharacterMaterial>>,
                Option<&MeshMaterial3d<AvatarMaterial>>,
                Option<&ChildOf>,
            ),
            (
                Without<NoShadowCast>,
                Without<moly_assets::scene_state::SourceInactive>,
                Without<bevy::light::NotShadowCaster>,
            ),
        >,
    >,
    joints: Extract<Query<&GlobalTransform>>,
    names: Extract<Query<&Name>>,
    bindposes: Extract<Res<Assets<SkinnedMeshInverseBindposes>>>,
    character_materials: Extract<Res<Assets<CharacterMaterial>>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    mut local_bounds: Local<HashMap<AssetId<Mesh>, Aabb3d>>,
    mut too_many_warned: Local<bool>,
    mut others_logged: Local<bool>,
) {
    let list = &mut *list;
    let mut other_names: Vec<String> = Vec::new();
    local_bounds.retain(|id, _| meshes.contains(*id));
    list.skinned.clear();
    list.palette_bytes.clear();
    let mut tally = SkinnedTally::default();
    // Skin -> (palette byte offset, index into `palette_matrices`).
    let mut palettes: HashMap<(AssetId<SkinnedMeshInverseBindposes>, Vec<Entity>), (u32, usize)> =
        HashMap::new();
    let mut palette_matrices: Vec<Vec<Mat4>> = Vec::new();
    for (mesh, skin, visibility, character, avatar, parent) in &skinned {
        let player = avatar.is_some();
        if !player && character.is_none() {
            tally.other_skinned += 1;
            if !*others_logged && other_names.len() < 12 {
                let name = parent
                    .and_then(|parent| names.get(parent.parent()).ok())
                    .map(|name| name.as_str().to_owned())
                    .unwrap_or_else(|| "?".to_owned());
                other_names.push(name);
            }
            continue;
        }
        if !visibility.get() {
            continue;
        }
        if let Some(character) = character {
            // The NPC's renderers are Off exactly while its dither alpha is 0.
            if character_materials
                .get(&character.0)
                .is_some_and(|material| material.params.dither_alpha == 0.0)
            {
                tally.npc_off_by_dither += 1;
                continue;
            }
        }
        let key = (skin.inverse_bindposes.id(), skin.joints.clone());
        let (palette_offset, palette_index) = match palettes.get(&key) {
            Some(found) => *found,
            None => {
                let matrices = match joint_palette(skin, &joints, &bindposes) {
                    Ok(matrices) => matrices,
                    Err(PaletteRefusal::TooManyJoints(count)) => {
                        if !*too_many_warned {
                            *too_many_warned = true;
                            warn!(
                                "主光阴影：蒙皮网格关节数 {count} 超过调色板上限 {MAX_JOINTS}，该网格不投影（仅告警一次）"
                            );
                        }
                        continue;
                    }
                    Err(PaletteRefusal::Unavailable) => continue,
                };
                let offset = push_palette(&mut list.palette_bytes, &matrices);
                tally.palettes += 1;
                tally.joints += matrices.len();
                let entry = (offset, palette_matrices.len());
                palette_matrices.push(matrices);
                palettes.insert(key, entry);
                entry
            }
        };
        let id = mesh.0.id();
        let local = *local_bounds.entry(id).or_insert_with(|| {
            meshes
                .get(&mesh.0)
                .and_then(|mesh| mesh.final_aabb)
                .unwrap_or(Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO))
        });
        // Every skinned vertex is a convex blend of its joints' images of the
        // bind-pose vertex, so the box of every joint's image of the bind-pose
        // box holds it.
        let mut min = Vec3::INFINITY;
        let mut max = Vec3::NEG_INFINITY;
        for matrix in &palette_matrices[palette_index] {
            for sx in [local.min.x, local.max.x] {
                for sy in [local.min.y, local.max.y] {
                    for sz in [local.min.z, local.max.z] {
                        let corner = matrix.transform_point3(Vec3::new(sx, sy, sz));
                        min = min.min(corner);
                        max = max.max(corner);
                    }
                }
            }
        }
        if player {
            tally.player += 1;
        } else {
            tally.npc += 1;
        }
        list.skinned.push(SkinnedShadowDraw {
            mesh: id,
            palette_offset,
            pipeline: CachedRenderPipelineId::INVALID,
            player,
            bounds: (min, max),
        });
    }
    list.skinned_tally = tally;
    if !*others_logged && !other_names.is_empty() {
        *others_logged = true;
        info!(
            "shadow casters: {} skinned meshes are no character renderer and cast no shadow (first seen, by parent node: {})",
            tally.other_skinned,
            other_names.join(", ")
        );
    }
}

/// Why a skin has no joint palette this frame.
pub(crate) enum PaletteRefusal {
    /// More joints than a palette window holds.
    TooManyJoints(usize),
    /// The inverse bind poses are not loaded, a joint entity is gone, or the
    /// skin names more joints than it has inverse bind poses.
    Unavailable,
}

/// A skin's joint matrices of this frame, joint by joint: the joint's world
/// transform times its inverse bind pose, as the colour pass builds them, so
/// the blended matrix maps a bind-pose vertex straight to world space.
pub(crate) fn joint_palette(
    skin: &SkinnedMesh,
    joints: &Query<&GlobalTransform>,
    bindposes: &Assets<SkinnedMeshInverseBindposes>,
) -> Result<Vec<Mat4>, PaletteRefusal> {
    let Some(inverse) = bindposes.get(&skin.inverse_bindposes) else {
        return Err(PaletteRefusal::Unavailable);
    };
    if skin.joints.len() > MAX_JOINTS {
        return Err(PaletteRefusal::TooManyJoints(skin.joints.len()));
    }
    let mut matrices = Vec::with_capacity(skin.joints.len());
    for (joint, inverse_bindpose) in skin.joints.iter().zip(inverse.iter()) {
        let Ok(global) = joints.get(*joint) else {
            break;
        };
        matrices.push(Mat4::from(global.affine()) * *inverse_bindpose);
    }
    if matrices.is_empty() || matrices.len() != skin.joints.len() {
        return Err(PaletteRefusal::Unavailable);
    }
    Ok(matrices)
}

/// Appends a palette at the next [`PALETTE_ALIGN`]-aligned offset of `bytes`
/// and returns that offset, the dynamic offset of its window.
pub(crate) fn push_palette(bytes: &mut Vec<u8>, matrices: &[Mat4]) -> u32 {
    let offset = bytes.len().next_multiple_of(PALETTE_ALIGN);
    bytes.resize(offset, 0);
    for matrix in matrices {
        mat4_bytes(*matrix, bytes);
    }
    offset as u32
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
    /// The unit direction towards the light.
    to_light: Vec3,
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
        to_light,
    })
}

/// What the three shadow buffers last received from `prepare_shadow_draws`.
#[derive(Default)]
struct ShadowUploads {
    frame: Option<Vec<u8>>,
    consumer: Option<Vec<u8>>,
    /// The object pool holds the current draw list's matrices.
    objects: bool,
}

/// Queues the depth pipeline of one key, or INVALID when the mesh layout lacks
/// a stream the entry point reads (counted, and skipped by the node).
fn queue_depth_pipeline(
    pipeline_cache: &PipelineCache,
    gpu: &ShadowMapGpu,
    key: &DepthPipelineKey,
) -> CachedRenderPipelineId {
    if key.skinned {
        let Ok(vertex) = key.layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_JOINT_INDEX.at_shader_location(2),
            Mesh::ATTRIBUTE_JOINT_WEIGHT.at_shader_location(3),
        ]) else {
            return CachedRenderPipelineId::INVALID;
        };
        return pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("site_shadow_depth_skinned_pipeline".into()),
            layout: vec![gpu.depth_layout.clone(), gpu.skin_layout.clone()],
            vertex: VertexState {
                shader: SHADOW_DEPTH_SHADER.clone(),
                shader_defs: vec![],
                entry_point: Some("shadow_depth_skinned_vertex".into()),
                buffers: vec![vertex],
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: key.topology,
                front_face: FrontFace::Ccw,
                // The character ShadowCaster passes cull nothing.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: CompareFunction::Less,
                stencil: StencilState::default(),
                bias: SHADOW_SLICE_RASTER_BIAS,
            }),
            multisample: Default::default(),
            ..Default::default()
        });
    }
    // 位置属性的偏移由网格布局自身给出；缺位置的网格按 INVALID
    // 记账，节点侧跳过。With a normal stream the rigid entry point runs the
    // whole source bias; without one, the bias along the light only.
    let (attributes, entry_point) = if key.normals {
        (
            vec![
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            ],
            "shadow_depth_vertex",
        )
    } else {
        (
            vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)],
            "shadow_depth_vertex_without_normal",
        )
    };
    match key.layout.0.get_layout(&attributes) {
        Ok(vertex) => pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("site_shadow_depth_pipeline".into()),
            layout: vec![gpu.depth_layout.clone()],
            vertex: VertexState {
                shader: SHADOW_DEPTH_SHADER.clone(),
                shader_defs: vec![],
                entry_point: Some(entry_point.into()),
                buffers: vec![vertex],
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: key.topology,
                front_face: FrontFace::Ccw,
                cull_mode: if key.two_sided { None } else { Some(Face::Back) },
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                // normal-Z：保留离光最近（深度最小）的面。
                depth_compare: CompareFunction::Less,
                stencil: StencilState::default(),
                bias: SHADOW_SLICE_RASTER_BIAS,
            }),
            multisample: Default::default(),
            ..Default::default()
        }),
        Err(_) => CachedRenderPipelineId::INVALID,
    }
}

/// PrepareResources：特化管线、解光源正交框、写三块 uniform。
///
/// 挂 PrepareResources 而非 Prepare：与引擎的网格资产 prepare（Prepare 集）
/// 错开一拍，本帧的 `RenderAssets<RenderMesh>` 与 MeshAllocator 切片已定。
fn prepare_shadow_draws(
    mut draws: ResMut<ShadowDrawList>,
    mut pipelines: ResMut<DepthPipelines>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    env: Option<Res<SiteEnv>>,
    gpu: Res<ShadowMapGpu>,
    mut palette: ResMut<ShadowPaletteGpu>,
    account: Res<ShadowAccount>,
    mut readiness_logged: Local<bool>,
    mut prepare_frames: Local<u64>,
    mut uploaded: Local<ShadowUploads>,
) {
    // 矩阵池按 draw 序全量写（缺管线的 draw 也占位，动态 offset 才对得上）。
    // The dynamic binding reads at index * object_stride, not index * 64.
    // Pad every matrix, including unavailable meshes, to that same slot size.
    // Otherwise each draw after the first consumes another object's transform
    // (or unwritten/stale bytes) and casts a displaced or malformed silhouette.
    // The pool is written once, never reallocated, and keeps what was last
    // written; it is rewritten only when the draw list changed.
    let stride = gpu.object_stride as usize;
    let write_objects = draws.changed || !uploaded.objects;
    let mut object_bytes = Vec::new();
    if write_objects {
        object_bytes.reserve(draws.draws.len() * stride);
        for item in &draws.draws {
            let next_slot = object_bytes.len() + stride;
            mat4_bytes(item.world, &mut object_bytes);
            object_bytes.resize(next_slot, 0);
        }
    }
    for item in draws.draws.iter_mut() {
        let Some(render_mesh) = meshes.get(item.mesh) else {
            continue;
        };
        let key = depth_pipeline_key(render_mesh, item.two_sided, false);
        item.pipeline = *pipelines
            .queued
            .entry(key)
            .or_insert_with_key(|key| queue_depth_pipeline(&pipeline_cache, &gpu, key));
    }
    // The skinned character casters: their own entry point and state (module
    // comment, "Caster 集"); a mesh without normals or joint streams is
    // counted as a layout that cannot be drawn.
    for item in draws.skinned.iter_mut() {
        item.pipeline = match meshes.get(item.mesh) {
            Some(render_mesh) => *pipelines
                .queued
                .entry(depth_pipeline_key(render_mesh, true, true))
                .or_insert_with_key(|key| queue_depth_pipeline(&pipeline_cache, &gpu, key)),
            None => CachedRenderPipelineId::INVALID,
        };
    }
    // This frame's joint palettes; a larger buffer replaces the old one when
    // they do not fit (the window read at the last offset must fit too).
    if !draws.palette_bytes.is_empty() {
        let needed = draws.palette_bytes.len() as u64 + PALETTE_WINDOW;
        if needed > palette.capacity {
            let capacity = needed.next_power_of_two();
            palette.buffer = render_device.create_buffer(&BufferDescriptor {
                label: Some("site_shadow_palettes"),
                size: capacity,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            palette.capacity = capacity;
        }
        render_queue.write_buffer(&palette.buffer, 0, &draws.palette_bytes);
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
            // The pipeline's caster bias: both biases in shadow-map texels of
            // world size (the orthographic width over the resolution, as the
            // pipeline derives it from the projection), negated, and scaled by
            // the soft-shadow kernel radius.
            let texel = matrices.width / SHADOWMAP_SIZE as f32;
            let depth_bias = -PIPELINE_SHADOW_DEPTH_BIAS * texel * SOFT_SHADOW_KERNEL_RADIUS;
            let normal_bias = -PIPELINE_SHADOW_NORMAL_BIAS * texel * SOFT_SHADOW_KERNEL_RADIUS;
            let to_light = matrices.to_light;
            let _ = resolved.insert(matrices);
            (
                ShadowFrame {
                    light_view_proj: mat4_cols(clip),
                    world_to_shadow: mat4_cols(consumer_matrix),
                    shadow_bias: Vec4::new(depth_bias, normal_bias, 0.0, 0.0),
                    light_direction: to_light.extend(0.0),
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
                shadow_bias: Vec4::ZERO,
                light_direction: Vec4::ZERO,
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
        inner.skinned_casters = draws.skinned.len();
        inner.skinned_invalid = draws
            .skinned
            .iter()
            .filter(|item| item.pipeline == CachedRenderPipelineId::INVALID)
            .count();
        inner.skinned_tally = draws.skinned_tally;
        if let Some(matrices) = &resolved {
            inner.light_box = Some((matrices.width, matrices.height, matrices.depth));
            inner.world_to_shadow = Some(matrices.consumer);
            inner.caster_bias = Some((
                matrices.width / SHADOWMAP_SIZE as f32,
                frame.shadow_bias.x,
                frame.shadow_bias.y,
            ));
        }
        inner.rigid_without_normals = draws
            .draws
            .iter()
            .filter(|item| {
                meshes
                    .get(item.mesh)
                    .is_some_and(|mesh| !mesh.layout.0.contains(Mesh::ATTRIBUTE_NORMAL))
            })
            .count();
    }
    let mut frame_bytes = Vec::with_capacity(FRAME_BYTES);
    for matrix in [frame.light_view_proj, frame.world_to_shadow] {
        for column in matrix {
            for component in column.to_array() {
                frame_bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
    }
    for slot in [frame.shadow_bias, frame.light_direction] {
        for component in slot.to_array() {
            frame_bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    debug_assert_eq!(frame_bytes.len(), FRAME_BYTES);
    // Both blocks keep their last contents; a write of the same bytes is
    // skipped.
    if uploaded.frame.as_deref() != Some(frame_bytes.as_slice()) {
        render_queue.write_buffer(&gpu.frame_buffer, 0, &frame_bytes);
        uploaded.frame = Some(frame_bytes);
    }
    let consumer_bytes = consumer.bytes();
    if uploaded.consumer.as_deref() != Some(consumer_bytes.as_slice()) {
        render_queue.write_buffer(&gpu.consumer_buffer, 0, &consumer_bytes);
        uploaded.consumer = Some(consumer_bytes);
    }
    if write_objects {
        if !object_bytes.is_empty() {
            render_queue.write_buffer(&gpu.object_buffer, 0, &object_bytes);
        }
        uploaded.objects = true;
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
///
/// Its one bind group binds the frame and object buffers, which are replaced
/// only when they grow, so it is cached by those buffer ids.
struct ShadowDepthNode {
    bind_groups: SharedBindGroupCache,
}

impl FromWorld for ShadowDepthNode {
    fn from_world(world: &mut World) -> Self {
        Self { bind_groups: SharedBindGroupCache::from_world(world) }
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
        let bind_group = {
            let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
            let mut groups = self.bind_groups.lock();
            groups.get(
                render_context.render_device(),
                "site_shadow_depth_bind_group",
                &pipeline_cache.get_bind_group_layout(&gpu.depth_layout),
                &[
                    (0, Bound::Buffer(&gpu.frame_buffer, 0, None)),
                    (
                        1,
                        Bound::Buffer(
                            &gpu.object_buffer,
                            0,
                            Some(std::num::NonZeroU64::new(gpu.object_binding_size).unwrap()),
                        ),
                    ),
                ],
                frame,
            )
        };
        // The palette window of the skinned casters, bound at each draw's
        // palette offset.
        let skin_group = (!draws.skinned.is_empty()).then(|| {
            let palette = world.resource::<ShadowPaletteGpu>();
            let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
            self.bind_groups.lock().get(
                render_context.render_device(),
                "site_shadow_skin_bind_group",
                &pipeline_cache.get_bind_group_layout(&gpu.skin_layout),
                &[(
                    0,
                    Bound::Buffer(&palette.buffer, 0, std::num::NonZeroU64::new(PALETTE_WINDOW)),
                )],
                frame,
            )
        });

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

        // Pipeline, vertex buffer and index buffer stay bound across draws of
        // one pass; a set that would bind the same object again is skipped, so
        // every draw still runs with exactly the state it set before.
        let mut bound_pipeline: Option<CachedRenderPipelineId> = None;
        let mut bound_vertex: Option<BufferId> = None;
        let mut bound_index: Option<(BufferId, IndexFormat)> = None;
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
            if bound_pipeline != Some(item.pipeline) {
                pass.set_pipeline(pipeline);
                bound_pipeline = Some(item.pipeline);
            }
            pass.set_bind_group(0, &bind_group, &[(index as u32) * gpu.object_stride]);
            if bound_vertex != Some(vertex_slice.buffer.id()) {
                pass.set_vertex_buffer(0, *vertex_slice.buffer.slice(..));
                bound_vertex = Some(vertex_slice.buffer.id());
            }
            match &render_mesh.buffer_info {
                RenderMeshBufferInfo::Indexed {
                    index_format,
                    count,
                } => {
                    let Some(index_slice) = allocator.mesh_index_slice(&item.mesh) else {
                        continue;
                    };
                    if bound_index != Some((index_slice.buffer.id(), *index_format)) {
                        pass.set_index_buffer(*index_slice.buffer.slice(..), *index_format);
                        bound_index = Some((index_slice.buffer.id(), *index_format));
                    }
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

        // The skinned character casters, after the rigid ones (the depth test
        // makes the order irrelevant). A skinned pipeline still compiling
        // leaves only its own draws out, not the frame.
        let mut skinned_drawn: usize = 0;
        if let Some(skin_group) = &skin_group {
            for item in &draws.skinned {
                if item.pipeline == CachedRenderPipelineId::INVALID {
                    continue;
                }
                let (Some(pipeline), Some(render_mesh)) = (
                    pipeline_cache.get_render_pipeline(item.pipeline),
                    meshes.get(item.mesh),
                ) else {
                    continue;
                };
                let Some(vertex_slice) = allocator.mesh_vertex_slice(&item.mesh) else {
                    continue;
                };
                if bound_pipeline != Some(item.pipeline) {
                    pass.set_pipeline(pipeline);
                    bound_pipeline = Some(item.pipeline);
                }
                // The rigid object window is not read by the skinned entry
                // point; offset 0 is a valid window of the pool.
                pass.set_bind_group(0, &bind_group, &[0]);
                pass.set_bind_group(1, skin_group, &[item.palette_offset]);
                if bound_vertex != Some(vertex_slice.buffer.id()) {
                    pass.set_vertex_buffer(0, *vertex_slice.buffer.slice(..));
                    bound_vertex = Some(vertex_slice.buffer.id());
                }
                match &render_mesh.buffer_info {
                    RenderMeshBufferInfo::Indexed { index_format, count } => {
                        let Some(index_slice) = allocator.mesh_index_slice(&item.mesh) else {
                            continue;
                        };
                        if bound_index != Some((index_slice.buffer.id(), *index_format)) {
                            pass.set_index_buffer(*index_slice.buffer.slice(..), *index_format);
                            bound_index = Some((index_slice.buffer.id(), *index_format));
                        }
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
                skinned_drawn += 1;
            }
        }
        drop(pass);

        // 阴影账目（节点侧）：帧数与实录 draw 数；到档的帧顺手录一次深度
        // 图拷贝，下一帧节点头读回落账（native；wasm 只落计数行）。
        let probe_due = {
            let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
            inner.frames_drawn += 1;
            inner.draws_recorded = drawn;
            inner.skinned_recorded = skinned_drawn;
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
        inner.next_probe_frame = inner.frames_drawn + probe_interval();
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
    let caster_bias = inner.caster_bias.unwrap_or((0.0, 0.0, 0.0));
    let total_texels = (SHADOWMAP_SIZE * SHADOWMAP_SIZE) as f32;

    // The player's caster window: the texel rectangle the player's skinned
    // bounds project to. Its texels are compared with the previous read-back
    // over the union of this and the previous window, and the rest of the map
    // is compared too, so a change inside the window with the player's bounds
    // centre in place is the animation moving the caster.
    let size = SHADOWMAP_SIZE as usize;
    let draws = world.resource::<ShadowDrawList>();
    let player_bounds = draws
        .skinned
        .iter()
        .filter(|item| item.player)
        .map(|item| item.bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
    let window = player_bounds.zip(inner.world_to_shadow).map(|((low, high), to_shadow)| {
        let (mut x0, mut y0, mut x1, mut y1) = (size, size, 0usize, 0usize);
        for sx in [low.x, high.x] {
            for sy in [low.y, high.y] {
                for sz in [low.z, high.z] {
                    let uv = to_shadow.transform_point3(Vec3::new(sx, sy, sz));
                    let tx = ((uv.x * size as f32) as i64).clamp(0, size as i64 - 1) as usize;
                    let ty = ((uv.y * size as f32) as i64).clamp(0, size as i64 - 1) as usize;
                    x0 = x0.min(tx);
                    y0 = y0.min(ty);
                    x1 = x1.max(tx);
                    y1 = y1.max(ty);
                }
            }
        }
        (x0, y0, x1, y1)
    });
    let mut window_nonclear = 0usize;
    let mut window_min = 1.0f32;
    if let Some((x0, y0, x1, y1)) = window {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = texel(x, y);
                if d < PROBE_CLEAR_EPS {
                    window_nonclear += 1;
                    window_min = window_min.min(d);
                }
            }
        }
    }
    let union = match (window, inner.previous_window) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
        (a, b) => a.or(b),
    };
    let (mut changed_in, mut changed_out) = (None, None);
    if let Some(previous) = inner.previous_map.as_deref() {
        let (mut inside, mut outside) = (0usize, 0usize);
        for (index, (now, before)) in data.chunks_exact(4).zip(previous.chunks_exact(4)).enumerate() {
            if now != before {
                let (x, y) = (index % size, index / size);
                let within = union.is_some_and(|(x0, y0, x1, y1)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y));
                if within {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        changed_in = Some(inside);
        changed_out = Some(outside);
    }
    let mut copy = inner.previous_map.take().unwrap_or_default();
    copy.clear();
    copy.extend_from_slice(&data);
    inner.previous_map = Some(copy);
    inner.previous_window = window;
    drop(data);
    buffer.unmap();
    let tally = inner.skinned_tally;
    let (centre, extent) = player_bounds
        .map(|(low, high)| ((low + high) * 0.5, high - low))
        .unwrap_or((Vec3::ZERO, Vec3::ZERO));
    info!(
        "shadow account, skinned casters: {} drawn {} (player {}, NPC {}; NPC Off at dither alpha 0: {}; other skinned meshes not casting: {}; layout without normal/joint streams {}); palettes {}, joints {} (joint world transform x inverse bind pose, this frame); rigid casters {}, total casters {}; player window {:?}: nonclear {} texels, min depth {:.4}, changed since last read-back {:?} (rest of map {:?}); player bounds centre ({:.3}, {:.3}, {:.3}) size ({:.3}, {:.3}, {:.3}); caster bias (all casters) from pipeline 1.0/1.0 x texel x 2.5: texel {:.6} m, depth {:.6} m, normal {:.6} m; raster bias 1 / 2.5; rigid casters without normals {}",
        inner.skinned_casters,
        inner.skinned_recorded,
        tally.player,
        tally.npc,
        tally.npc_off_by_dither,
        tally.other_skinned,
        inner.skinned_invalid,
        tally.palettes,
        tally.joints,
        inner.casters,
        inner.casters + inner.skinned_casters,
        window,
        window_nonclear,
        window_min,
        changed_in,
        changed_out,
        centre.x,
        centre.y,
        centre.z,
        extent.x,
        extent.y,
        extent.z,
        caster_bias.0,
        caster_bias.1,
        caster_bias.2,
        inner.rigid_without_normals,
    );
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
        "主光阴影账目（wasm 计数行）：帧 prepared {} / drawn {}（静默跳过 {}、空过 {}）；caster {}（布局取不出 {}，实录 draw {}）；skinned casters {} drawn {} (player {}, NPC {}, NPC Off at dither 0 {}, other skinned {}, layout invalid {}; palettes {}, joints {})",
        inner.frames_prepared,
        inner.frames_drawn,
        inner.frames_skipped_not_ready,
        inner.frames_empty_pass,
        inner.casters,
        inner.casters_invalid,
        inner.draws_recorded,
        inner.skinned_casters,
        inner.skinned_recorded,
        inner.skinned_tally.player,
        inner.skinned_tally.npc,
        inner.skinned_tally.npc_off_by_dither,
        inner.skinned_tally.other_skinned,
        inner.skinned_invalid,
        inner.skinned_tally.palettes,
        inner.skinned_tally.joints,
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
    let skin_layout = BindGroupLayoutDescriptor::new(
        "site_shadow_skin_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX,
            ((0, uniform_buffer_sized(true, std::num::NonZeroU64::new(PALETTE_WINDOW))),),
        ),
    );
    commands.insert_resource(ShadowPaletteGpu {
        buffer: render_device.create_buffer(&BufferDescriptor {
            label: Some("site_shadow_palettes"),
            size: PALETTE_WINDOW,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        capacity: PALETTE_WINDOW,
    });
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
        skin_layout,
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
        crate::render::gpu::install_bind_group_caches(app);
        app.add_systems(Startup, load);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ShadowDrawList>()
            .init_resource::<DepthPipelines>()
            .init_resource::<ShadowAccount>()
            .add_systems(RenderStartup, init_shadow_resources)
            .add_systems(
                ExtractSchedule,
                (extract_shadow_casters, extract_skinned_casters).chain(),
            )
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
