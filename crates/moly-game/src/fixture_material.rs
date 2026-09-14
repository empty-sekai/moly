//! 家具 Basic / Rug / Fence 材质：`Mysekai/Fixture/Basic` 的 Base 程序 Bevy Material。
//!
//! 材质值来自 glb 场景展开实体上的 `GltfMaterialExtras`（提取侧把 Unity
//! 属性原样写进 glTF 材质 extras：`floats` 一张浮点表、顶层
//! `fixtureShaderUsage`、`shader` 名）与 `GltfMaterialName`；主贴图句柄取
//! glb 材质本身的 `baseColorTexture`（提取侧把 `_MainTex` 写进它）。
//! 变体由 `FixtureMaterialKey` 编码进管线特化；WGSL 在
//! `shaders/fixture_material.wgsl`。
//!
//! 拒绝语义：族门（shader 名闭集 + 具名拒绝 URP/Lit）不过、必需浮点
//! 缺失、(usage, blend) 二维选择越界或与混合模式矛盾、glb 材质没有
//! 主贴图——都按材质名具名拒绝，该实体保留原材质，不用默认值近似。
//! 闭集内但非 Basic 的族（ShadowMesh / Road / Object 等）是本单范围外：
//! 具名记数保留原材质，不是拒绝。两个例外，都按「源里像素贡献恒 0」
//! 隐藏而不是保留原材质：
//! - 渲染状态「写零」的 pass（见 [`StencilOnlyPass`]）：保留原材质会把
//!   一个源里不可见的网格画成白板，按 Unity 渲染状态隐藏。
//! - ShadowMesh 族（见 [`ShadowMeshGated`]）：源的片元以材质浮点
//!   `_Show` 为门，低于 0.5 全部 discard；live 数据里该族全部材质的
//!   `_Show` 恒 0.0（提取侧对创作浮点逐字拷贝），且 1,429 条
//!   AnimationClip 的浮点曲线里没有一条指向它——没有运行时改写者。
//!
//! 窗外观支路（`fixtureShaderUsage` = 1 → 二维表落点 26/27）：源以模板
//! 缓冲把它裁进窗洞（窗包自己的写零面先在窗洞上写 ref4，外观网按
//! Equal ref4 只在洞里画）。本管线没有模板缓冲，等价门在片元里做：
//! 写零面是平面凸四边形，顶点里把四角与本顶点投到 NDC，片元做「点在
//! 凸四边形内」判定，窗外 discard。四角在换装时从同一 glb 的写零面
//! 网格读出（两个节点的局部变换均为恒等，四角即原始顶点位置），烘进
//! 材质 uniform 尾部四个槽。
//!
//! 变体开关盘：glb extras 顶层 `validKeywords` 列是源运行态 shader 变体
//! 开关的唯一来源（源在材质构建时按整型属性重派生 keyword；`invalidKeywords`
//! 列是「曾经有效现禁用」的编辑器残留，运行态不认——工具生成的
//! `_SHOW_*` 预览门族全在那侧，零消费）。本管线的处置分三档：
//! - `_USE_ALPHA_CLIP` / `_DISABLE_DITHER` / `_ENABLE_MODULE_FRESNEL` /
//!   `_ENABLE_MODULE_REFLECTION`：已实现的四个编译期变体（alpha 丢弃 /
//!   Bayer 抖动块 / 菲涅尔加色 / 反射加色），开关取 keyword 在场与否
//!   （反射那条另有一个数据面前提，见下）；
//! - `_RECEIVE_SHADOWS_OFF`：源 Basic 程序全部组合的编译产物里这个
//!   开关逐字节无差——该程序任何变体都不采样主光阴影贴图，开关是
//!   引擎侧「不接收阴影」标记，程序级零效果；本管线构造同形（无阴影
//!   接收 pass），只入账不动画面；
//! - 其余 keyword：逐材质具名入账（换装日志 + 计数），不静默吞，
//!   见挂账清单。
//!
//! 质感分支有两条并列的加色支路，各自一个 keyword，可单开也可同开。
//! 逐句律与逐位比对在 `moly_law::fixture::fresnel`。
//!
//! fresnel（`_ENABLE_MODULE_FRESNEL`）：开关打开的变体在宝藏阴影之后、
//! 雾之前往 rgb 上加色 exp2(log2(clamp(1−N·V, 0, 1)) × `_FresnelPower`)
//! × `_FresnelColor.rgb` × `_FresnelColor.a`——先钳再取对数（N·V 略超 1
//! 时负数进 log2 是 NaN），alpha 是因子不是摆设；视线方向分透视（相机位
//! 减片元位归一化，无 epsilon 钳制）与正交（视图矩阵第三行，
//! unity_OrthoParams.w 非零时）两支。⚠ live 全量带它的 3 个材质全是载具
//! （bike/car/flyingcar 的 *_ref），**摆放表里一个都没有**——修完的当前
//! 画面上看不见它的效果，这是为载具摆入准备的。
//!
//! reflection（`_ENABLE_MODULE_REFLECTION`）：紧跟在 fresnel 支之后、雾
//! 之前加 min(pow(1−N·V, `_ReflectionFresnelPower`), 1) × 立方图采样 ×
//! `_ReflectionIntensity`。三个承重点：这一支的 log2 **入参不钳**（与
//! fresnel 支相反），只有上钳 min(·, 1)；两支在源里**共用同一个 1−N·V**
//! （同开的变体里那个减法只算一次，fresnel 吃钳后副本、反射吃未钳原件）；
//! 立方图那一项**折叠成了常数**——材质没给那个槽位绑对象，引擎按纹理维度
//! 顶上自己的内建默认立方图，而那张是六面同色的 ⇒ 采样值与方向、mip、
//! mip 偏置全都无关 ⇒ 源里那个反射向量在本管线连算都不用算。
//! ⚠ **折叠只对槽位为空的材质成立**：live 全量 19 个材质开着这条分支，
//! 16 个（egg 系，摆放表里全在）槽位是空引用 ⇒ 走折叠形；3 个（载具的
//! *_ref）绑了一张真立方图 ⇒ 折叠对它们是错的，**分支不开、按名字入账**
//! （换装收尾的具名告警一行），需要真采样那条路，
//! 未实现；它们当前也全未摆放。判据是提取侧写进 extras 纹理引用表的
//! `_ReflectionCubeMap` **键在不在**，与真源侧 PPtr 空/非空逐条一致。
//!
//! 挂账（源 Base 程序里有、本管线没有的消费方）：
//!
//! 裁定（数据面有、程序面不接，与为什么）：
//! - 顶点色混合不接线：源 Basic 族的编译产物里片元/顶点程序都没有
//!   COLOR0 的消费点。glb 里的顶点 COLOR_0 属性是提取侧对全族统一的
//!   数据面，「数据里有活值」不等于「程序消费它」。阳性对照在另外
//!   两族：FieldObject 族 14 个浮点属性里 12 个进 COLOR0 语义、Tree
//!   族 59 个里 56 个进——那两族程序真消费顶点色，Basic 不消费；给
//!   Basic 接上会把一个源里没有的乘法加进片元。
//! - `_NormalShadingIntensity` / `_ShadingNormalBlend` / 裸 `_Edge*` /
//!   `_ReceiveShadow`：在源 Base 程序的编译产物里被翻译器加上
//!   「声明了但未被引用」的重名前缀（死 uniform 标记，随产物一起
//!   下发）。按死键具名入账，不进管线。
//!
//! 第二颜色目标（源 Base 程序的 SV_Target1 = 自发射遮罩 × 现象自发光
//! 开关链）由自建的 emission pass 承接（[`crate::fixture_emission`]）：
//! 类型 1 时门 = bright==1、类型 2 时门 = dark==1、其余 0；两个目标的
//! alpha 都 = 主贴图 alpha。三个材质 int（`_BrightPhenomenaEmission` /
//! `_DarkPhenomenaEmission` / `_PhenomenaEmissionOverrideMode`）与
//! `_EmissionMaskTex` 贴图在换装时读出：门 int 进 [`FixtureEmission`]
//! 组件（着色器消费），遮罩均值进 [`EmissionAccount`]（初始材质账目）。
//! 初始 override 为 0；机关动画事件可在实际实例上强制开/关，动态值由
//! emission pass 的独立实例属性承接，不用初始材质表推断没有写者。遮罩缺失而
//! 门 int 非零的材质按零贡献处理并具名告警（fail-closed：不静默取空白
//! 遮罩）。
//!
//! 挂账（源 Base 程序里有、本管线没有的消费方）：
//! - 墙布局 ShadowCaster 运行时关闭：源在墙布局时逐材质关 ShadowCaster
//!   pass；本单没有影子 pass（prepass 与阴影接收全关，与站点族同形），
//!   标记面是 [`WallLayoutShadowCasterOff`]，见 `fixture.rs`。

#[path = "fixture_surfaces.rs"]
pub mod surfaces;

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::gltf::{GltfMaterialExtras, GltfMaterialName};
use bevy::image::Image;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, StandardMaterial};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;
use moly_assets::material_textures::SourceMaterialTextures;
use moly_law::fixture::family::{basic_fixture_shader_attribute, check_family_shader};
use moly_law::fixture::mip::mip_levels;

use crate::env::{SiteEnv, SiteEnvGpuBuffer};
use crate::fixture::{FixtureVisualRoot, FixtureVisualSceneReady, FixtureVisualReady, FixtureScenesReady};
use crate::fixture_emission::FixtureEmission;

/// 本单交付的族键：源 shader 名，与律里族闭集的「Basic」同名。
/// 闭集内其他名字是范围外（换装时具名保留）。
const BASIC_SHADER: &str = "Mysekai/Fixture/Basic";

/// ShadowMesh 族的源 shader 名：整族按材质浮点 `_Show` 门恒 0 像素，
/// 见 [`ShadowMeshGated`]。
const SHADOWMESH_SHADER: &str = "Mysekai/Fixture/ShadowMesh";
const RUG_SHADER: &str = "Mysekai/Fixture/Rug";
const FENCE_SHADER: &str = "Mysekai/Fixture/Fence";

/// 材质 uniform 的槽数与字节数。槽序是本文件与
/// `shaders/fixture_material.wgsl` 里 `FixtureParams` 结构体之间的契约，
/// 两边同改。
pub const PARAMS_SLOTS: usize = 12;
pub const PARAMS_BYTES: usize = PARAMS_SLOTS * 16;

/// 前七个槽是「每条 Unity 属性一个 vec4 槽」；变体不消费的槽（关着的
/// dither 门）照写。尾部四个槽是窗外观门的四角（不是 Unity 属性，
/// 挂在同一块 uniform 尾部，见模块注释「窗外观支路」）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixtureParams {
    /// `(_MainTexOffsetX, _MainTexOffsetY)`：顶点里加进 uv0。
    pub main_tex_offset: [f32; 2],
    /// Source UV scroll velocity; packed in the unused zw of the UV slot.
    pub uv_scroll: [f32; 2],
    /// `_DitherAlpha`。抖动变体关着时源不消费它；照写数据值。
    pub dither_alpha: f32,
    pub use_phenomena_lighting: f32,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
    /// Fixture meshes retain source UVs; exported PNG rows begin at the top.
    /// This conversion applies to every material, including emissive masks.
    pub uv_v_flip: f32,
    /// 窗外观门四角：写零面网格顶点在本网格局部系里的位置，已排成
    /// 平面凸环序。非窗材质全零（消费代码由 `FIXTURE_WINDOW_CLIP`
    /// 变体整块编译期移除，四角不会被读）。
    pub clip_corners: [[f32; 3]; 4],
}

impl FixtureParams {
    /// 按上面的槽序摊平成字节。
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        let mut push = |slot: [f32; 4]| {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        };
        push([self.main_tex_offset[0], self.main_tex_offset[1], self.uv_scroll[0], self.uv_scroll[1]]);
        push([self.dither_alpha, 0.0, 0.0, 0.0]);
        push([self.use_phenomena_lighting, 0.0, 0.0, 0.0]);
        push([self.override_shading_parameter, 0.0, 0.0, 0.0]);
        push([self.local_shading_intensity, 0.0, 0.0, 0.0]);
        push([self.local_edge_threshold, 0.0, 0.0, 0.0]);
        push([self.local_edge_smoothness, 0.0, 0.0, 0.0]);
        push([self.uv_v_flip, 0.0, 0.0, 0.0]);
        for corner in &self.clip_corners {
            push([corner[0], corner[1], corner[2], 0.0]);
        }
        debug_assert_eq!(bytes.len(), PARAMS_BYTES);
        bytes
    }
}

/// 管线特化键：族内变体。作为 `AsBindGroup::Data` 进管线缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixtureMaterialKey {
    pub fence: bool,
    /// None = Basic; Some(blended) = Rug, with source queue 2007/2008.
    pub rug: Option<bool>,
    pub render_queue: u32,
    /// `_USE_ALPHA_CLIP`：被采 alpha − 0.5 < 0 丢弃（阈值编译期常量）。
    pub alpha_clip: bool,
    /// `_DISABLE_DITHER` 的反：Bayer 抖动块在着色器里编译期开/关。
    /// live 数据里全体材质关抖动（`_DisableDither` = 1），块保留给
    /// 未关的变体。
    pub dither: bool,
    /// 窗外观支路（usage=1 → 落点 26/27）：屏空间四边形门整块编入
    /// （`FIXTURE_WINDOW_CLIP`）。
    pub window_clip: bool,
    /// `_ENABLE_MODULE_FRESNEL`：菲涅尔加色块（`FIXTURE_MODULE_FRESNEL`，
    /// 宝藏阴影后、雾前）。live 带它的 3 个材质全是载具、均未摆放。
    pub fresnel: bool,
    /// `_ENABLE_MODULE_REFLECTION`：反射加色块
    /// （`FIXTURE_MODULE_REFLECTION`，紧跟在菲涅尔支之后、雾之前）。
    /// 只对**立方图槽位为空**的材质开——那时源那次立方图采样折叠成常数
    /// （见 [`moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE`]）。绑了真
    /// 立方图的材质（载具那 3 个）走不了这条：分支不开，换装收尾按名字
    /// 具名告警。
    pub reflection: bool,
}

/// 家具 Basic 族的材质资产。
#[derive(Debug, Clone, TypePath, Asset)]
pub struct FixtureMaterial {
    pub key: FixtureMaterialKey,
    pub params: FixtureParams,
    pub main_tex: Handle<Image>,
    /// `_FixtureObjectBlendMode` 的布尔形状：`AlphaMode::Blend` 的开关。
    /// 二维选择表的落点（5/6/26/27）在 resolve 时已与此对过账。
    pub blend: bool,
    /// 质感分支 fresnel 的两参（`_FresnelPower` / `_FresnelColor`）。
    /// 分支关着的材质填 0（块被特化整块移除，值不被读）。**不进
    /// `params.bytes()`**：那 12 槽的序是主 pass 与自发光 pass 共用的
    /// 前缀契约（emission 对象池把 params 内嵌在 64+192+16 的摊平布局
    /// 里，扩槽会错位那侧的 emission 字段）——两条质感分支的参数合用
    /// 主 pass 的独立 binding 4，见 [`Self::shading_branch_bytes`]。
    pub fresnel_power: f32,
    pub fresnel_color: [f32; 4],
    /// 质感分支反射的两参（`_ReflectionFresnelPower` /
    /// `_ReflectionIntensity`）。立方图那一项不在这里——它折叠成了着色器
    /// 里的常数（见 [`FixtureMaterialKey::reflection`]）。
    pub reflection_power: f32,
    pub reflection_intensity: f32,
}

impl FixtureMaterial {
    /// 两条质感分支参数的序列化（binding 4，3×vec4）。与 params 分块的
    /// 缘故见字段注释——那是跨 pass 的字节契约，不能扩；这一块只在主
    /// pass 的片元里读，可以扩。槽序与 WGSL 的 `FixtureShadingBranch`
    /// 是契约，两边同改。
    fn shading_branch_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(48);
        for component in [
            self.fresnel_power,
            0.0,
            0.0,
            0.0,
            self.fresnel_color[0],
            self.fresnel_color[1],
            self.fresnel_color[2],
            self.fresnel_color[3],
            self.reflection_power,
            self.reflection_intensity,
            0.0,
            0.0,
        ] {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        bytes
    }
}

impl AsBindGroup for FixtureMaterial {
    type Data = FixtureMaterialKey;
    type Param = (SRes<SiteEnvGpuBuffer>, SRes<RenderAssets<GpuImage>>);

    fn label() -> &'static str {
        "fixture_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        render_device: &RenderDevice,
        (env_buffer, images): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let main = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        // 唯一的纹理槽永远有真实贴图——缺主贴图的材质在建计划时已拒，
        // 不需要 fallback 绑定。
        let bindings = BindingResources(vec![
            // binding 0：材质自己的参数块。
            (0, OwnedBindingResource::Data(OwnedData(self.params.bytes()))),
            // binding 1：全局量，与站点材质共用同一个 buffer。
            (1, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (3, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, main.sampler.clone())),
            // binding 4：两条质感分支的参数（分支关着的变体不读它，照绑
            // ——布局是全变体共享的，多的绑定合法）。
            (4, OwnedBindingResource::Data(OwnedData(self.shading_branch_bytes()))),
        ]);
        Ok(UnpreparedBindGroup { bindings })
    }

    fn bind_group_data(&self) -> Self::Data {
        self.key
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        };
        // binding 4 只在片元里读（两条质感分支的加色块）。
        let fragment_uniform = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![
            uniform(0),
            uniform(1),
            texture(2),
            sampler(3),
            fragment_uniform(4),
        ]
    }
}

impl Material for FixtureMaterial {
    // embedded 源的键前缀是 lib 名（连字符转下划线），见 site 材质同注。
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/fixture_material.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/fixture_material.wgsl".into()
    }

    /// 混合模式：`_FixtureObjectBlendMode` = 1（AlphaBlended）的材质走
    /// alpha 混合管线；0（Opaque）走不透明。
    fn alpha_mode(&self) -> AlphaMode {
        // Rug blending belongs between ground and objects in the source's
        // opaque queue range; its actual blend state is installed in specialize.
        if self.key.rug.is_some() {
            AlphaMode::Opaque
        } else if self.blend {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }

    /// 家具族不进 prepass：本单只交付 Base 片元，阴影消费方不在范围里。
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let mut defs: Vec<&str> = Vec::new();
        if key.bind_group_data.fence { defs.push("FIXTURE_FENCE"); }
        crate::material_order::set_queue(descriptor, key.bind_group_data.render_queue);
        if let Some(blended) = key.bind_group_data.rug {
            defs.push("FIXTURE_RUG");
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_write_enabled = false;
                depth.depth_compare = CompareFunction::Always;
            }
            if let Some(target) = descriptor.fragment.as_mut()
                .and_then(|f| f.targets.get_mut(0)).and_then(Option::as_mut) {
                target.blend = blended.then_some(BlendState::ALPHA_BLENDING);
            }
        }
        if key.bind_group_data.alpha_clip {
            defs.push("FIXTURE_ALPHA_CLIP");
        }
        if key.bind_group_data.dither {
            defs.push("FIXTURE_DITHER");
        }
        if key.bind_group_data.window_clip {
            defs.push("FIXTURE_WINDOW_CLIP");
        }
        if key.bind_group_data.fresnel {
            defs.push("FIXTURE_MODULE_FRESNEL");
        }
        if key.bind_group_data.reflection {
            defs.push("FIXTURE_MODULE_REFLECTION");
        }
        // 视线方向的顶点输出：两条质感分支读的是同一个输出，任一开着就
        // 要它。**从那两个键派生而不是另设一个开关**——漏推它时着色器会
        // 引用一个不存在的标识符，那是运行期才喊的一条管线错误，而
        // `cargo build` 一个 wgsl 字节都不校验。
        if key.bind_group_data.fresnel || key.bind_group_data.reflection {
            defs.push("FIXTURE_VIEW_DIR");
        }
        for def in defs {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(ref mut fragment) = descriptor.fragment {
                fragment.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}

// ---- 墙布局的 ShadowCaster 标记面 ----

/// 墙布局（LayoutType 命中 0xF0 任一位）摆放的网格实体标记。
/// 源在墙布局时对逐材质关 ShadowCaster pass（SetupRenderer 律）；
/// 主光阴影消费者按这个运行时状态排除墙面家具，不改其颜色 pass 显隐。
#[derive(Component)]
pub struct WallLayoutShadowCasterOff;

/// 渲染状态「写零」pass 的网格实体标记：材质 `_ColorMask` = 0（颜色通道
/// 全关）且 `_ZWrite` = 0（深度不写）。这样的 pass 在源里只写模板缓冲，
/// 对颜色与深度的贡献恒为零——本管线没有模板消费方，隐藏即与源输出
/// 逐值相等，不是近似。注意判据是 **CPU 侧渲染状态浮点**，不是 shader
/// 名：Unity 由这些浮点配渲染状态，shader 程序本身不消费它们；按名字
/// （「stencil 材质该藏」）或按纹理在场（白板材质没贴图）判断都是错门
/// ——与变体门只认 keyword 是同族教训。模板消费方接进来时按此标记找回
/// 这些网格。
#[derive(Component)]
pub struct StencilOnlyPass;

/// ShadowMesh 族的网格实体标记：该族片元以材质浮点 `_Show` 为门，低于
/// 0.5 全部 discard；live 数据里这族全部材质的 `_Show` 恒 0.0（提取侧
/// 对创作浮点逐字拷贝，不改写），全部 AnimationClip 的浮点曲线里也没
/// 有指向它的曲线——源渲染里像素贡献恒 0。本管线未移植该族 shader，
/// 保留 glb 默认材质会把这套投影网画成错误的实心面；隐藏即与源输出
/// 逐值相等，不是近似。影子网消费方接进来时按此标记找回这些网格。
#[derive(Component)]
pub struct ShadowMeshGated;

/// 渲染状态「写零」签名：`_ColorMask` = 0 且 `_ZWrite` = 0。
/// 混合因子在无通道可写时不参与输出，不必检查。
fn is_stencil_only(extras: &serde_json::Value) -> bool {
    matches!(
        (extras_float(extras, "_ColorMask"), extras_float(extras, "_ZWrite")),
        (Some(0.0), Some(0.0))
    )
}

// ---- 解析 ----

/// glb 材质 extras 里提取侧约定的浮点表列。
fn extras_float(extras: &serde_json::Value, key: &str) -> Option<f64> {
    extras
        .get("floats")
        .and_then(|floats| floats.get(key))
        .and_then(|value| value.as_f64())
}

/// glb 材质 extras 里提取侧约定的颜色表列（四元组）。长度不是 4 的
/// 数组按缺键处理（fail-closed 由调用方具名拒绝，不截不补）。
fn extras_color(extras: &serde_json::Value, key: &str) -> Option<[f64; 4]> {
    let array = extras
        .get("colors")
        .and_then(|colors| colors.get(key))
        .and_then(|value| value.as_array())?;
    let mut color = [0.0; 4];
    for (slot, value) in color.iter_mut().zip(array) {
        *slot = value.as_f64()?;
    }
    Some(color)
}

/// glb 材质 extras 里提取侧约定的顶层 usage 列。
fn extras_usage(extras: &serde_json::Value) -> Option<f64> {
    extras
        .get("fixtureShaderUsage")
        .and_then(|value| value.as_f64())
}

/// glb 材质 extras 顶层 `validKeywords` 列（源运行态的 shader 变体开关
/// 来源，见模块注释）。None = 列缺失：fail-closed 由调用方具名拒绝，
/// 不猜空列。
fn extras_valid_keywords(extras: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    extras
        .get("validKeywords")
        .and_then(|value| value.as_array())
}

/// 解析一条 Basic 家具材质的产出：GPU 材质、二维表落点（日志对账用）、
/// 自发光三 int（挂账日志用——本管线没有第二颜色目标，不进材质）、
/// 变体开关盘对账件（日志用）。
/// 自发光两 int（进 [`FixtureEmission`] 组件的材质侧值）、遮罩贴图的
/// glTF 纹理下标（build 阶段据此建句柄）。
struct ResolvedBasic {
    force_emission: bool,
    material: FixtureMaterial,
    attribute: u32,
    emission: EmissionInts,
    /// `_RECEIVE_SHADOWS_OFF` 在场与否（源程序级零效果，只入账）。
    receive_shadows_off: bool,
    /// 开关三件之外仍出现的 keyword 逐个名字（live 数据里是两个未实现
    /// 质感分支，模块注释挂账）；出现别的名字也走同一账，不猜语义。
    unported_keywords: Vec<String>,
    /// 反射分支开着、但材质**绑了一张真立方图**——折叠形对它是错的，
    /// 本管线没有真采样那条路 ⇒ 不开分支，按名字入账（keyword 也留在
    /// `unported_keywords` 里）。live 数据里是载具那 3 个材质，全未摆放。
    reflection_needs_cubemap: bool,
    /// extras 纹理表里 `_EmissionMaskTex` 的 glTF 纹理下标；缺键 None
    /// （门 int 全零的材质无所谓；非零的组合按零贡献具名告警）。
    mask_index: Option<u32>,
}

/// 驱动源第二颜色目标开关链的两个材质 int（数据侧是 0/1 浮点）。
/// 换装时进 [`FixtureEmission`]，emission pass 的片元里按现象类型比较。
/// 缺哪个浮点记 None，不静默取 0。
/// The initial material flags are separate from the per-instance override
/// written later by fixture animation events.
#[derive(Debug, Clone, Copy)]
struct EmissionInts {
    bright: Option<f64>,
    dark: Option<f64>,
}

/// 现象自发光门值：源第二颜色目标的开关链逐句翻译（类型 1 比 bright、
/// 类型 2 比 dark、其余 0）。日志与切档账目共用这一条推导。
fn emission_gate(emission: &EmissionInts, emission_type: i32) -> Option<i32> {
    let (bright, dark) = (emission.bright?, emission.dark?);
    Some(match emission_type {
        1 => (bright == 1.0) as i32,
        2 => (dark == 1.0) as i32,
        _ => 0,
    })
}

/// 换装完成时收账的自发光账目：逐材质的亮/暗臂、遮罩线性均值。天气
/// 切档日志按当前现象类型读它推导逐材质贡献行——「切档后自发光账目
/// 可从日志推导」的数据面（着色器侧另有一份门，两处同一条开关链）。
#[derive(Resource, Default)]
pub struct EmissionAccount {
    pub rows: Vec<EmissionRow>,
}

/// 一条账目行。`mask_mean` 是遮罩贴图解码进线性域后的逐通道均值
/// （alpha 丢弃）；遮罩缺失（或纹素未到）记 None。
pub struct EmissionRow {
    pub name: String,
    pub bright: Option<f64>,
    pub dark: Option<f64>,
    pub mask_mean: Option<[f32; 3]>,
}

impl EmissionAccount {
    /// 按现象类型推逐材质贡献行（切档日志用）：门 × 遮罩均值。门缺
    /// （int 缺键）或遮罩缺的行原样带 None，不静默取 0。
    pub fn contribution_lines(&self, emission_type: i32) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
                format!(
                    "{}：亮臂 {:?} 暗臂 {:?} 遮罩均值 {:?} ⇒ 类型 {} 门 {:?} 贡献 {:?}",
                    row.name,
                    row.bright,
                    row.dark,
                    row.mask_mean,
                    emission_type,
                    emission_gate(
                        &EmissionInts { bright: row.bright, dark: row.dark },
                        emission_type
                    ),
                    match (
                        emission_gate(
                            &EmissionInts { bright: row.bright, dark: row.dark },
                            emission_type
                        ),
                        row.mask_mean,
                    ) {
                        (Some(1), Some(mean)) => Some(mean),
                        (Some(0), _) => Some([0.0; 3]),
                        _ => None,
                    }
                )
            })
            .collect()
    }
}

/// 解析一条 Basic 家具材质：族门 → 二维选择表对账 → 窗外观门四角 →
/// 变体键与 uniform 槽。失败返回具名报错，调用方保留原材质。
///
/// `clip_quad` 是该材质所在 glb 根下写零面的四角（按首见根取），仅在
/// usage=1（窗外观支路）时必需：缺写零面或四角不可用都是具名拒绝，
/// 不静默放行——放行会把整片窗外网画到没有墙的场景里。
fn resolve_basic(
    name: &str,
    extras: &serde_json::Value,
    main_tex: Handle<Image>,
    clip_quad: Option<&Result<[Vec3; 4], String>>,
) -> Result<ResolvedBasic, String> {
    // 二维选择表：(usage, blend) → ShaderAttribute。越界即拒；落点的
    // 低位（5/26 不透明、6/27 混合）必须与 blend 模式一致——不一致说明
    // 两列数据互相矛盾，响亮拒绝。
    let usage = extras_usage(extras)
        .ok_or_else(|| format!("家具材质 {name} 的 extras 缺 fixtureShaderUsage"))?;
    let blend_raw = extras_float(extras, "_FixtureObjectBlendMode")
        .ok_or_else(|| format!("家具材质 {name} 缺浮点属性 _FixtureObjectBlendMode"))?;
    let attribute = basic_fixture_shader_attribute(usage as i32, blend_raw as i32)
        .map_err(|err| format!("家具材质 {name}：{err}"))?;
    let blend = blend_raw >= 0.5;
    // 落点与混合模式的一致性对账：源表里 5/26 是不透明格、6/27 是
    // 混合格。落点由 (usage, blend) 查表得出，这里只核表输出与 blend
    // 列一致——表或列哪边错都会在这里响亮。
    let expected = if blend { 6 } else { 5 };
    if attribute != expected && attribute != expected + 21 {
        return Err(format!(
            "家具材质 {name} 的二维选择落点 {attribute} 与 _FixtureObjectBlendMode \
             {blend_raw} 互相矛盾"
        ));
    }
    // 窗外观支路：usage=1 → 落点 26/27，源按模板 Equal 只画窗洞内。
    // 四角来自同一 glb 的写零面；没有或不可用即拒（具名），该材质保留
    // 原材质——比静默画整片窗外网更响亮。
    let window_clip = usage as i32 == 1;
    let clip_corners = if window_clip {
        match clip_quad {
            Some(Ok(quad)) => [
                quad[0].to_array(),
                quad[1].to_array(),
                quad[2].to_array(),
                quad[3].to_array(),
            ],
            Some(Err(reason)) => {
                return Err(format!(
                    "窗外观支路材质 {name} 的写零面四角不可用：{reason}"
                ));
            }
            None => {
                return Err(format!(
                    "窗外观支路材质 {name} 所在 glb 下没有写零面（模板写者缺失），\
                     无法建屏空间门"
                ));
            }
        }
    } else {
        [[0.0; 3]; 4]
    };
    // 变体开关盘：keyword 型开关，源运行态只认 valid keyword（材质构建
    // 时按整型属性重派生；invalid 列是编辑器残留，不读）。两个开关的
    // 旧读法（`_UseAlphaClip`/`_DisableDither` 浮点）与 keyword 侧在
    // 全量 Basic 材质上逐项一致——换的是「读哪个源」，不是值变化。
    let valid = extras_valid_keywords(extras)
        .ok_or_else(|| format!("家具材质 {name} 的 extras 缺 validKeywords 列"))?;
    let keyword_on = |keyword: &str| {
        valid
            .iter()
            .any(|item| item.as_str() == Some(keyword))
    };
    let alpha_clip = keyword_on("_USE_ALPHA_CLIP");
    // 关抖动的 keyword 在场即 `_DISABLE_DITHER`，抖动变体取反。
    let dither = !keyword_on("_DISABLE_DITHER");
    let receive_shadows_off = keyword_on("_RECEIVE_SHADOWS_OFF");
    let fresnel = keyword_on("_ENABLE_MODULE_FRESNEL");
    // 反射支：keyword 开着**且**立方图槽位为空时才走本管线的折叠形。
    // 槽位状态从提取侧写进 extras 的纹理引用表读——**键在不在**就是判据：
    // 键不在 = 源材质那个 TexEnv 是空引用（折叠成立，采样是常数）；
    // 键在（值是 null，因为 glTF 没有立方图这个类型，提取侧只能记下
    // 「这里引了一个对象」）= 材质绑了一张真立方图 ⇒ **折叠对它是错的**，
    // 必须真采样，本管线没有那条路 ⇒ 不开分支、按名字挂账。
    // 这个判据与真源侧的 PPtr 空/非空逐条一致（16 空 / 3 非空），且是
    // 两条独立读取路径；同一张表里 `_MainTex` 每条都是真下标，所以
    // 「键不在」是真的不在，不是表坏了。
    let reflection_keyword = keyword_on("_ENABLE_MODULE_REFLECTION");
    let cube_bound = extras
        .get("textures")
        .and_then(|textures| textures.as_object())
        .is_some_and(|textures| textures.contains_key("_ReflectionCubeMap"));
    let reflection = reflection_keyword && !cube_bound;
    let reflection_needs_cubemap = reflection_keyword && cube_bound;
    let unported_keywords: Vec<String> = valid
        .iter()
        .filter_map(|item| item.as_str())
        .filter(|keyword| {
            if *keyword == "_ENABLE_MODULE_REFLECTION" {
                // 折叠形接上的那 16 条不再算未实现；绑了真立方图的那几条
                // 仍然算——对它们这条分支确实没实现。
                return !reflection;
            }
            !matches!(
                *keyword,
                "_USE_ALPHA_CLIP"
                    | "_DISABLE_DITHER"
                    | "_RECEIVE_SHADOWS_OFF"
                    | "_ENABLE_MODULE_FRESNEL"
            )
        })
        .map(str::to_owned)
        .collect();
    let get = |key: &str| -> Result<f32, String> {
        let value = extras_float(extras, key)
            .ok_or_else(|| format!("家具材质 {name} 缺浮点属性 {key}"))?;
        Ok(value as f32)
    };
    let params = FixtureParams {
        uv_scroll: [0.0; 2],
        main_tex_offset: [
            get("_MainTexOffsetX")?,
            get("_MainTexOffsetY")?,
        ],
        dither_alpha: get("_DitherAlpha")?,
        use_phenomena_lighting: get("_UsePhenomenaLighting")?,
        override_shading_parameter: get("_OverrideShadingParameter")?,
        local_shading_intensity: get("_LocalShadingIntensity")?,
        local_edge_threshold: get("_LocalEdgeThreshold")?,
        local_edge_smoothness: get("_LocalEdgeSmoothness")?,
        uv_v_flip: 1.0,
        clip_corners,
    };
    // fresnel 两参：分支开着时必需（缺任一即具名拒绝——开关与数据
    // 互相矛盾，静默取默认会把一个源里没有的加色画上去）；关着时填 0
    // （块被特化移除，值不被读）。
    let (fresnel_power, fresnel_color) = if fresnel {
        (
            get("_FresnelPower")?,
            extras_color(extras, "_FresnelColor")
                .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32])
                .ok_or_else(|| {
                    format!("家具材质 {name} 开着 fresnel 却缺 _FresnelColor")
                })?,
        )
    } else {
        (0.0, [0.0; 4])
    };
    // 反射两参：分支开着时必需（缺任一即具名拒绝，与 fresnel 同形——
    // 静默取默认会把一个源里没有的加色画上去）；关着时填 0（块被特化
    // 移除，值不被读）。
    let (reflection_power, reflection_intensity) = if reflection {
        (
            get("_ReflectionFresnelPower")?,
            get("_ReflectionIntensity")?,
        )
    } else {
        (0.0, 0.0)
    };
    let emission = EmissionInts {
        bright: extras_float(extras, "_BrightPhenomenaEmission"),
        dark: extras_float(extras, "_DarkPhenomenaEmission"),
    };
    // 遮罩贴图的 glTF 纹理下标：提取侧把材质纹理引用写进 extras 的
    // textures 表（纹理名 → glTF 纹理下标）。缺键 = 该材质没有遮罩。
    let mask_index = extras
        .get("textures")
        .and_then(|textures| textures.get("_EmissionMaskTex"))
        .and_then(|value| value.as_u64())
        .map(|value| value as u32);
    let base_queue = match (window_clip, blend) {
        (true, true) => 2085, (true, false) => 2035,
        (false, true) => 3020, (false, false) => 2065,
    };
    let render_queue = u32::try_from(base_queue + extras_float(extras, "_RenderPriority").unwrap_or(0.0) as i32)
        .map_err(|_| format!("家具材质 {name} 的渲染队列为负"))?;
    Ok(ResolvedBasic {
        force_emission: false,
        material: FixtureMaterial {
            key: FixtureMaterialKey {
                fence: false,
                rug: None,
                render_queue,
                alpha_clip,
                dither,
                window_clip,
                fresnel,
                reflection,
            },
            params,
            main_tex,
            blend,
            fresnel_power,
            fresnel_color,
            reflection_power,
            reflection_intensity,
        },
        attribute,
        emission,
        receive_shadows_off,
        unported_keywords,
        reflection_needs_cubemap,
        mask_index,
    })
}

/// Fence Base shares the toon equation and UV offset with Basic, but has no
/// alpha clip, fragment normal normalization, treasure shadow, fog or emission.
/// All 16 CN Base variants reduce to two fragment programs (dither on/off).
fn resolve_fence(name: &str, extras: &serde_json::Value, main_tex: Handle<Image>) -> Result<ResolvedBasic, String> {
    let get = |key: &str| extras_float(extras, key)
        .map(|v| v as f32).ok_or_else(|| format!("Fence 材质 {name} 缺属性 {key}"));
    for (key, expected) in [("_ZWrite", 1.0), ("_ZTest", 4.0), ("_Cull", 2.0),
        ("_ColorMask", 15.0), ("_SrcBlend", 1.0), ("_DstBlend", 0.0)] {
        if get(key)? != expected { return Err(format!("Fence 材质 {name} 的 {key} 不在已实现的源状态内")); }
    }
    let valid = extras_valid_keywords(extras)
        .ok_or_else(|| format!("Fence 材质 {name} 缺 validKeywords"))?;
    let keywords: Vec<&str> = valid.iter().filter_map(|v| v.as_str()).collect();
    let queue = u32::try_from(2065 + get("_RenderPriority")? as i32)
        .map_err(|_| format!("Fence 材质 {name} 队列为负"))?;
    Ok(ResolvedBasic {
        material: FixtureMaterial {
            key: FixtureMaterialKey { fence: true, rug: None, render_queue: queue,
                alpha_clip: false, dither: !keywords.contains(&"_DISABLE_DITHER"),
                window_clip: false, fresnel: false, reflection: false },
            params: FixtureParams {
                main_tex_offset: [get("_MainTexOffsetX")?, get("_MainTexOffsetY")?], uv_scroll: [0.0; 2],
                dither_alpha: get("_DitherAlpha")?, use_phenomena_lighting: get("_UsePhenomenaLighting")?,
                override_shading_parameter: get("_OverrideShadingParameter")?,
                local_shading_intensity: get("_LocalShadingIntensity")?,
                local_edge_threshold: get("_LocalEdgeThreshold")?, local_edge_smoothness: get("_LocalEdgeSmoothness")?,
                uv_v_flip: 1.0, clip_corners: [[0.0; 3]; 4],
            },
            main_tex, blend: false, fresnel_power: 0.0, fresnel_color: [0.0; 4], reflection_power: 0.0, reflection_intensity: 0.0,
        },
        // GetAttribute maps Fence directly to 5, without the Basic usage/blend table.
        attribute: 5, emission: EmissionInts { bright: Some(0.0), dark: Some(0.0) },
        force_emission: false, mask_index: None, receive_shadows_off: true,
        reflection_needs_cubemap: false,
        unported_keywords: keywords.iter().filter(|k| !["_DISABLE_DITHER", "_RECEIVE_SHADOWS_OFF",
            "_USE_ALPHA_CLIP", "_USE_MYSEKAI_FOG", "_USE_MYSEKAI_SITE_EXTENSION", "INSTANCING_ON"].contains(k))
            .map(|k| k.to_string()).collect(),
    })
}

/// Rug Base uses source UV scroll, direct phenomena tint and treasure shadows;
/// it has no Basic half-Lambert shade, dither, Fresnel or reflection modules.
fn resolve_rug(name: &str, extras: &serde_json::Value, main_tex: Handle<Image>) -> Result<ResolvedBasic, String> {
    let get = |key: &str| extras_float(extras, key)
        .map(|v| v as f32).ok_or_else(|| format!("Rug 材质 {name} 缺属性 {key}"));
    let blend_raw = get("_FixtureObjectBlendMode")?;
    if blend_raw != 0.0 && blend_raw != 1.0 { return Err(format!("Rug 材质 {name} 混合模式越界 {blend_raw}")); }
    let blend = blend_raw == 1.0;
    for (key, expected) in [("_ZWrite", 0.0), ("_ZTest", 8.0), ("_Cull", 2.0), ("_ColorMask", 15.0),
        ("_SrcBlend", if blend { 5.0 } else { 1.0 }), ("_DstBlend", if blend { 10.0 } else { 0.0 })] {
        if get(key)? != expected { return Err(format!("Rug 材质 {name} 的 {key} 不在已实现的源状态内")); }
    }
    let keywords: Vec<&str> = extras.get("validKeywords").and_then(|v| v.as_array())
        .into_iter().flatten().filter_map(|v| v.as_str()).collect();
    let unported_keywords = keywords.iter().filter(|k| !["_USE_ALPHA_CLIP", "_USE_MYSEKAI_FOG", "_RECEIVE_SHADOWS_OFF"].contains(k))
        .map(|k| k.to_string()).collect();
    let priority = get("_RenderPriority")? as i32;
    let queue = u32::try_from(if blend { 2008 + priority } else { 2007 + priority })
        .map_err(|_| format!("Rug 材质 {name} 队列为负"))?;
    Ok(ResolvedBasic {
        material: FixtureMaterial {
            key: FixtureMaterialKey { fence: false, rug: Some(blend), render_queue: queue,
                alpha_clip: keywords.contains(&"_USE_ALPHA_CLIP"), dither: false,
                window_clip: false, fresnel: false, reflection: false },
            params: FixtureParams { main_tex_offset: [0.0; 2], uv_scroll: [get("_UVScrollX")?, get("_UVScrollY")?],
                dither_alpha: 1.0, use_phenomena_lighting: get("_UsePhenomenaLighting")?,
                override_shading_parameter: 0.0, local_shading_intensity: 0.0,
                local_edge_threshold: 0.0, local_edge_smoothness: 0.0, uv_v_flip: 1.0, clip_corners: [[0.0; 3]; 4] },
            main_tex, blend, fresnel_power: 0.0, fresnel_color: [0.0; 4], reflection_power: 0.0, reflection_intensity: 0.0,
        },
        attribute: if blend { 29 } else { 28 },
        emission: EmissionInts { bright: extras_float(extras, "_BrightPhenomenaEmission"), dark: extras_float(extras, "_DarkPhenomenaEmission") },
        force_emission: get("_EnableManualEmission")? == 1.0 || get("_DebugEmission")? == 1.0,
        receive_shadows_off: get("_ReceiveShadow")? == 0.0,
        unported_keywords, reflection_needs_cubemap: false,
        mask_index: extras.get("textures").and_then(|v| v.get("_EmissionMaskTex")).and_then(|v| v.as_u64()).map(|v| v as u32),
    })
}

/// 把写零面的四个唯一顶点排成平面凸环序（绕质心按平面极角，从第一条
/// 边的法向看逆时针）。环序是片元「点在凸四边形内」判定的前提；非
/// 平面 / 退化 / 非凸在这里具名拒绝——那是数据不该出现的形状，不静默
/// 近似。
fn order_clip_quad(points: [Vec3; 4]) -> Result<[Vec3; 4], String> {
    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
    let scale = normal.length();
    if scale < 1e-9 {
        return Err(format!(
            "四点退化（前三点共线）[{:?} {:?} {:?} {:?}]",
            points[0], points[1], points[2], points[3]
        ));
    }
    for point in &points {
        if (point - points[0]).dot(normal).abs() > scale * 1e-4 {
            return Err(format!(
                "四点不共面 [{:?} {:?} {:?} {:?}]",
                points[0], points[1], points[2], points[3]
            ));
        }
    }
    let u = (points[1] - points[0]).normalize();
    let v = normal.normalize().cross(u);
    let centroid = (points[0] + points[1] + points[2] + points[3]) * 0.25;
    let angle = |point: Vec3| {
        let delta = point - centroid;
        // atan2(对 v 的分量, 对 u 的分量)：从 u 轴向 v 轴量角——(u, v, n̂)
        // 右手系下绕 +n̂ 逆时针。两个参数写反会把环序镜像成顺时针，
        // 凸性判据随之全负（每个凸四边形都被拒）。
        delta.dot(v).atan2(delta.dot(u))
    };
    let mut ring = points;
    ring.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
    // 凸性：绕环相邻三点在面法向上的叉积同号（严格同号，退化边拒绝）。
    for i in 0..4 {
        let a = ring[i];
        let b = ring[(i + 1) % 4];
        let c = ring[(i + 2) % 4];
        if (b - a).cross(c - b).dot(normal) <= 0.0 {
            return Err(format!(
                "四点非凸 [{:?} {:?} {:?} {:?}]",
                points[0], points[1], points[2], points[3]
            ));
        }
    }
    Ok(ring)
}

// ---- mip 链 ----

/// sRGB EOTF 的解码/编码（GPU 采样 sRGB 格式纹素用的同一条曲线）。
/// 生成 mip 的 box 滤波要在采样输出的域（线性）里做，alpha 不经 gamma。
fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// CPU 生成完整 2×2 box mip 链：对 2 的幂形状（含非方形——级数按最长
/// 边，短边先到 1 就保持 1）的 RGBA8 主贴图，在采样输出的域里滤波
/// （sRGB 格式先解码后平均再编码），纹素按 LayerMajor 的 dense mip 序
/// 紧密排进同一份 data（wgpu 的 `create_texture_with_data` 正是这个
/// 约定，行对齐由它内部补）。
///
/// 不可处理的形状具名返回（不做静默近似）：`AlreadyChained`（ktx/dds
/// 路径已有链）、`BadShape`（任一边为 0 或非 2 的幂——这种形状源侧
/// 不出链）、`BadFormat`（非 RGBA8 两格式）、`NoData`（纹素不在——被
/// 渲染提取取走，晚于「查到 Loaded 的同一帧」）、`BadLength`（数据
/// 长度与尺寸不符）。级数律与「源序列化的 mip 计数逐值一致」的 corpus
/// 判据在 `moly_law::fixture::mip`：live 摆放面四种形状 256²→9、
/// 512²→10、2048²→12、2048×1024→12（45 行材质→贴图全对上）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MipSkip {
    AlreadyChained,
    BadShape,
    BadFormat,
    NoData,
    BadLength,
    /// 只由 [`mask_storage_mean`] 用：纹理不是 sRGB 格式，而着色器那侧的
    /// 编回存储域是无条件的 ⇒ 那一步会多编一次。见该函数的联锁说明。
    NotSrgb,
}

pub(crate) fn generate_mip_chain(image: &mut Image) -> Result<u32, MipSkip> {
    let width = image.texture_descriptor.size.width;
    let height = image.texture_descriptor.size.height;
    if image.texture_descriptor.mip_level_count > 1 {
        return Err(MipSkip::AlreadyChained);
    }
    // 级数律（moly-law）：非零 2 的幂即有完整链，级数按最长边。非方形
    // 不是坏形状——源序列化的 mip 计数就是这么给的。
    let levels = mip_levels(width, height).ok_or(MipSkip::BadShape)?;
    let srgb = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return Err(MipSkip::BadFormat),
    };
    let data = image.data.as_ref().ok_or(MipSkip::NoData)?;
    if data.len() != (width * height * 4) as usize {
        return Err(MipSkip::BadLength);
    }
    let mut chain = data.clone();
    let mut src = data.clone();
    let (mut w, mut h) = (width as usize, height as usize);
    while w > 1 || h > 1 {
        let cw = (w / 2).max(1);
        let ch = (h / 2).max(1);
        // 每轴的采样数：父级该轴还有 ≥2 纹素时是 2（box 2×2），已到 1
        // 就退化为 1（直拷）。方形时恒 2×2，与旧实现同一滤波。
        let taps_x = w / cw;
        let taps_y = h / ch;
        let mean_scale = 1.0 / (taps_x * taps_y) as f32;
        let mut dst = vec![0u8; cw * ch * 4];
        for y in 0..ch {
            for x in 0..cw {
                let mut acc = [0f32; 4];
                for dy in 0..taps_y {
                    for dx in 0..taps_x {
                        let i = ((y * taps_y + dy) * w + (x * taps_x + dx)) * 4;
                        for c in 0..4 {
                            let channel = src[i + c] as f32 / 255.0;
                            let linear = if srgb && c < 3 {
                                srgb_decode(channel)
                            } else {
                                channel
                            };
                            acc[c] += linear;
                        }
                    }
                }
                let o = (y * cw + x) * 4;
                for c in 0..4 {
                    let mean = acc[c] * mean_scale;
                    let channel = if srgb && c < 3 {
                        srgb_encode(mean)
                    } else {
                        mean
                    };
                    dst[o + c] = (channel * 255.0 + 0.5).min(255.0) as u8;
                }
            }
        }
        chain.extend_from_slice(&dst);
        src = dst;
        w = cw;
        h = ch;
    }
    image.data = Some(chain);
    image.texture_descriptor.mip_level_count = levels;
    Ok(levels)
}

/// 遮罩贴图第 0 级的逐通道均值，**在存储（gamma）域**上取——自发光缓冲
/// 存的就是存储域的值（`shaders/fixture_emission.wgsl` 采样后编回存储域，
/// 而那张缓冲是 `Rgba16Float`、硬件不再编码），账目均值同一域才可比。
/// 8 位纹素本身就是存储值，所以这里**不解码**。alpha 丢弃。
///
/// ⚠ 此前这里解码进线性域，那是**旧的缓冲域**下正确的量法；缓冲域一改，
/// 这条量法必须跟着改，否则账目报的是一个缓冲里已经不存在的域的数。
///
/// 形状不可处理（非 RGBA8 两格式/纹素不在/长度不符）具名返回——账目行记
/// None，不静默取 0。mip 链生成不改第 0 级，链补完再读也一致。
///
/// **`NotSrgb` 是一道 fail-closed 联锁，不是形状拒绝**：着色器那侧
/// **无条件**把采样值编回存储域，那一步只有在硬件确实解码过（= 纹理按
/// sRGB 装载）时才是恒等的往返。遮罩走 glb 子资产 label 直载
/// （bevy_gltf 对没被任何材质按线性用途引用的纹理按 sRGB 装载），所以
/// 常态是 sRGB；万一某张进来是线性格式，着色器会**多编一次**、自发光整片
/// 偏亮，而那是个静默的错。⇒ 在这里具名喊出来，别让它静默漂。
fn mask_storage_mean(image: &Image) -> Result<[f32; 3], MipSkip> {
    match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => {}
        TextureFormat::Rgba8Unorm => return Err(MipSkip::NotSrgb),
        _ => return Err(MipSkip::BadFormat),
    }
    let data = image.data.as_ref().ok_or(MipSkip::NoData)?;
    let width = image.texture_descriptor.size.width;
    let height = image.texture_descriptor.size.height;
    // mip 链生成后 data 含全链；均值只读第 0 级（账目行的量法是「遮罩
    // 整体亮度」，第 0 级全图均值即它的推导基）。短于一级才是真缺数据。
    let level0 = (width * height * 4) as usize;
    if data.len() < level0 {
        return Err(MipSkip::BadLength);
    }
    let mut acc = [0f32; 3];
    for i in (0..level0).step_by(4) {
        for c in 0..3 {
            acc[c] += data[i + c] as f32 / 255.0;
        }
    }
    let count = (width * height) as f32;
    Ok([acc[0] / count, acc[1] / count, acc[2] / count])
}

// ---- 换装 ----

/// 换装完成标记。
#[derive(Resource)]
pub struct FixtureMaterialsSwapped;

/// 一条换装计划：解析好的家具材质与它的 glb 原材质句柄。
struct Planned {
    force_emission: bool,
    name: String,
    material: FixtureMaterial,
    /// 解析用的 glb 原材质句柄：换装按它换。
    source: Handle<StandardMaterial>,
    /// 二维选择表的落点（5/6 普通家具、26/27 窗外观）——采样日志对账
    /// 「窗族走对支路」用。
    attribute: u32,
    /// 自发光两 int（进 [`FixtureEmission`] 组件）。
    emission: EmissionInts,
    /// 变体开关盘对账件（采样日志用，见 [`ResolvedBasic`]）。
    receive_shadows_off: bool,
    unported_keywords: Vec<String>,
    /// 反射分支开着但材质绑了真立方图（折叠形对它是错的，分支未开）。
    reflection_needs_cubemap: bool,
    /// 自发射遮罩句柄：build 阶段按 extras 的纹理下标从同一 glb 装载。
    /// None = 材质没有遮罩（门 int 全零时无贡献损失；非零组合按零贡献
    /// 具名告警，fail-closed）。
    mask: Option<Handle<Image>>,
    /// 数据侧 `_MainTex` 的 scale/offset 四元组（extras 的
    /// textureScaleOffset），仅供换装完成的采样日志对账——源 Base 变体
    /// 不消费 `_MainTex_ST`（编译期剔除），shader 侧只有 uv 偏移加法。
    /// 缺失时记 None，不静默取恒等。
    main_tex_st: Option<[f64; 4]>,
}

/// 一次换装的全部状态。
struct SwapPlan {
    planned: Vec<Planned>,
    /// 渲染状态写零的材质句柄：执行阶段对这些句柄的网格实体隐藏。
    stencil_only: Vec<Handle<StandardMaterial>>,
    /// ShadowMesh 族的材质句柄：源里恒 0 像素，执行阶段隐藏。
    shadow_mesh: Vec<Handle<StandardMaterial>>,
    tally: SwapTally,
}

type PendingVisuals<'w, 's> = Query<'w, 's, (Entity, &'static FixtureVisualRoot),
    (With<FixtureVisualSceneReady>, Without<FixtureVisualReady>)>;

/// 计数们：换装完成时一次性 `info!`/`warn!`，是「真的换上了吗」的
/// 现算证据。
#[derive(Default)]
struct SwapTally {
    fence_materials: usize,
    fence_entities: usize,
    rug_materials: usize,
    rug_entities: usize,
    basic_materials: usize,
    basic_entities: usize,
    /// 混合材质（`_FixtureObjectBlendMode` = 1）的子计数。
    blended_materials: usize,
    /// 窗外观支路（usage=1 → 落点 26/27）材质的子计数。
    window_clip_materials: usize,
    /// 墙布局标记面插上的实体数。
    wall_entities: usize,
    /// 具名拒绝：族门/必需浮点/二维选择/主贴图缺失/窗门四角不可用。
    refused: Vec<String>,
    /// 闭集内但非 Basic 的族（范围外）：具名保留。
    unported: Vec<String>,
    /// 渲染状态写零（ColorMask 0 + ZWrite 0）：隐藏的材质名与实体数。
    stencil_hidden_names: Vec<String>,
    stencil_hidden_entities: usize,
    /// ShadowMesh 族（源 `_Show` 恒 0）：隐藏的材质名与实体数。
    shadow_mesh_names: Vec<String>,
    shadow_mesh_entities: usize,
    /// valid keyword 里本管线没有的分支：逐材质具名（`材质名 (keyword)`），
    /// 不静默吞。
    unported_keyword_materials: Vec<String>,
    /// 反射分支接上的材质子计数（立方图槽位为空 ⇒ 采样折叠成常数）。
    reflection_materials: usize,
    /// 反射分支开着但绑了真立方图的材质名：折叠形对它们是错的，本管线
    /// 不开分支（keyword 同时留在未实现账里）。
    reflection_needs_cubemap_names: Vec<String>,
}

/// Update：全部家具 scene 展开后（`FixtureScenesReady`，fixture 模块的闩）
/// 建 plan、等贴图到齐、一次性换装。此前每帧空转。
fn switch_materials(
    mut commands: Commands,
    ready: Option<Res<FixtureScenesReady>>,
    swapped: Option<Res<FixtureMaterialsSwapped>>,
    server: Res<AssetServer>,
    env: Res<SiteEnv>,
    roots: PendingVisuals,
    children: Query<&Children>,
    parts: Query<(
        &MeshMaterial3d<StandardMaterial>,
        &GltfMaterialName,
        &GltfMaterialExtras,
    )>,
    mesh_parts: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    std_materials: Res<Assets<StandardMaterial>>,
    source_textures: Query<&SourceMaterialTextures>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<FixtureMaterial>>,
    mut plan: Local<Option<SwapPlan>>,
    layout: (Res<surfaces::FixtureSurfaceReadiness>, Res<crate::fixture::FixtureLayoutRevision>, Local<u64>, Local<Vec<Entity>>),
) {
    let (surfaces_ready, revision, mut seen_revision, mut seen_roots) = layout;
    let mut pending_roots: Vec<_> = roots.iter().map(|(entity, _)| entity).collect();
    pending_roots.sort_unstable();
    if *seen_roots != pending_roots {
        *plan = None;
        *seen_roots = pending_roots;
    }
    if *seen_revision != revision.0 {
        *plan = None;
        *seen_revision = revision.0;
    }
    if swapped.is_some() && roots.is_empty() {
        return;
    }
    if ready.is_none() {
        return;
    }
    if !surfaces_ready.0 { return; }
    let Some(mut state) = plan.take().or_else(|| {
        build_swap_plan(
            &roots,
            &children,
            &parts,
            &mesh_parts,
            &meshes,
            &std_materials,
            &source_textures,
        )
    }) else {
        return;
    };

    // 等贴图到齐：换装早于贴图到位会让实体闪回默认材质。装载失败具名 panic。
    let mut all_loaded = true;
    for item in &state.planned {
        match server.load_state(&item.material.main_tex) {
            LoadState::Failed(err) => {
                panic!("家具材质 {} 的贴图装载失败：{err:?}", item.name)
            }
            LoadState::Loaded => {}
            _ => all_loaded = false,
        }
        if let Some(mask) = &item.mask {
            match server.load_state(mask) {
                LoadState::Failed(err) => {
                    panic!("家具材质 {} 的自发光遮罩装载失败：{err:?}", item.name)
                }
                LoadState::Loaded => {}
                _ => all_loaded = false,
            }
        }
    }
    if !all_loaded {
        *plan = Some(state);
        return;
    }

    // mip 链补齐（换装前的同一步）：demo 侧对这批 2 的幂贴图由 GPU 生成
    // 完整链并三线性采样；bevy 的 PNG 装载路径恒 mip_level_count = 1
    // （bevy_image/bevy_render 均无生成路径），缩采样走无 mip 的 aliasing。
    // 必须在「查到 Loaded 的同一帧」做：渲染提取会把主世界的 data 取走，
    // 晚一帧就拿不到纹素了。改主世界 Image 发 Modified 事件，渲染侧按
    // 新的 descriptor 与 data 重建 GPU 纹理。遮罩同批处理：emission pass
    // 的两个采样同样带全局 mip 偏置。
    let mut mipped: Vec<(&str, u32)> = Vec::new();
    // 同一张贴图被多个材质共享：第一个材质补好链后，后来者遇到的是
    // 已补好的链（AlreadyChained）——不是跳过，单独计共享数（遮罩与主
    // 贴图同句柄、或一张遮罩被多个材质共享，都落进这个计数）。
    let mut shared_chains = 0usize;
    let mut mip_skipped: Vec<(&str, MipSkip)> = Vec::new();
    for item in &state.planned {
        let name = item.name.as_str();
        match images.get_mut(&item.material.main_tex) {
            Some(image) => match generate_mip_chain(image) {
                Ok(levels) => mipped.push((name, levels)),
                Err(MipSkip::AlreadyChained) => shared_chains += 1,
                Err(reason) => mip_skipped.push((name, reason)),
            },
            None => mip_skipped.push((name, MipSkip::NoData)),
        }
        if let Some(mask) = &item.mask {
            match images.get_mut(mask) {
                Some(image) => match generate_mip_chain(image) {
                    Ok(_) => {}
                    Err(MipSkip::AlreadyChained) => shared_chains += 1,
                    Err(reason) => mip_skipped.push((name, reason)),
                },
                None => mip_skipped.push((name, MipSkip::NoData)),
            }
        }
    }
    info!(
        "家具贴图 mip 链：补 {} 张（2×2 box，sRGB 在线性域滤波），逐张级数 \
         {mipped:?}（级数按最长边，与源序列化的 mip 计数一致，非方形不是坏形状）；\
         共享句柄复用已有链 {} 次；跳过 {} 张（具名原因）：{skipped:?}",
        mipped.len(),
        shared_chains,
        mip_skipped.len(),
        mipped = mipped,
        skipped = mip_skipped,
    );

    // 自发光账目收账：遮罩均值在纹素被渲染提取取走前（换装同帧）读。
    // 收录面 = 遮罩在手（门 int 全零也收——着色器里的门按现象类型比
    // 较，类型翻档即非零）或门 int 非零；两者皆无的材质贡献恒 0，不进
    // 账。天气切档日志读这份账目推逐材质贡献行。
    let mut account = EmissionAccount::default();
    let mut mask_mean_skipped: Vec<(String, MipSkip)> = Vec::new();
    for item in &state.planned {
        let armed = item.mask.is_some()
            || item.emission.bright == Some(1.0)
            || item.emission.dark == Some(1.0);
        if !armed {
            continue;
        }
        // 门 int 非零而遮罩缺失：fail-closed 零贡献（不静默取空白遮罩），
        // 具名告警——这类组合的实体不插自发光组件。
        if item.mask.is_none() {
            warn!(
                "家具材质 {} 的门 int 非零但 extras 缺 _EmissionMaskTex：\
                 自发光按零贡献处理（fail-closed，不取空白遮罩）",
                item.name
            );
        }
        let mask_mean = match &item.mask {
            Some(mask) => match images.get(mask) {
                Some(image) => match mask_storage_mean(image) {
                    Ok(mean) => Some(mean),
                    Err(reason) => {
                        mask_mean_skipped.push((item.name.clone(), reason));
                        None
                    }
                },
                None => {
                    mask_mean_skipped.push((item.name.clone(), MipSkip::NoData));
                    None
                }
            },
            None => None,
        };
        account.rows.push(EmissionRow {
            name: item.name.clone(),
            bright: item.emission.bright,
            dark: item.emission.dark,
            mask_mean,
        });
    }
    if !mask_mean_skipped.is_empty() {
        warn!(
            "自发光遮罩均值不可读 {} 条（账目行记 None，贡献推导不出）：{:?}",
            mask_mean_skipped.len(),
            mask_mean_skipped
        );
    }

    // 换装执行：按 glb 原材质句柄把网格实体归到计划下标，逐条换。
    // scene 展开后的实体集合在 ready 之后不再变，走一遍层级即可。
    let mut by_handle: HashMap<Handle<StandardMaterial>, usize> = HashMap::new();
    for (index, item) in state.planned.iter().enumerate() {
        by_handle.insert(item.source.clone(), index);
    }
    let mut swapped_entities: HashMap<usize, usize> = HashMap::new();
    // Fence's source draw manager groups instances by mesh/material/texture.
    // Reusing its material handle preserves Bevy instancing across source parts.
    // Basic keeps per-entity materials for animated face/UV parameters.
    let mut fence_materials: HashMap<usize, Handle<FixtureMaterial>> = HashMap::new();
    for (root_entity, placement) in roots.iter() {
        let wall = placement.is_wall_layout();
        let mut stack = vec![root_entity];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter().map(|child| child));
            }
            let Ok((material, _, _)) = parts.get(entity) else {
                // 不带 glb 材质名/extras 的实体（非 glb 材质路径）：
                // 保留原材质，不在此计数（无法具名）。
                continue;
            };
            if let Some(index) = by_handle.get(&material.0).copied() {
                let item = &state.planned[index];
                let handle = if item.material.key.fence {
                    fence_materials.entry(index)
                        .or_insert_with(|| materials.add(item.material.clone())).clone()
                } else {
                    materials.add(item.material.clone())
                };
                commands
                    .entity(entity)
                    .remove::<MeshMaterial3d<StandardMaterial>>()
                    .insert(MeshMaterial3d(handle));
                // 自发光参与资格：遮罩在手即插（门 int 全零也插——片元里
                // 的门按现象类型比较，自然为零；类型翻档即活）。遮罩缺失
                // 的材质不插（门 int 非零的组合已在收账段具名告警）。
                if let Some(mask) = &item.mask {
                    commands.entity(entity).insert(FixtureEmission {
                        mask: mask.clone(),
                        main_tex: item.material.main_tex.clone(),
                        params: item.material.params,
                        key: item.material.key,
                        blend: item.material.blend,
                        // 缺键按 0：源 shader 的属性默认值就是 0。
                        bright: item.emission.bright.unwrap_or(0.0) as f32,
                        dark: item.emission.dark.unwrap_or(0.0) as f32,
                        force_emission: item.force_emission,
                    });
                }
                *swapped_entities.entry(index).or_default() += 1;
                if wall {
                    commands.entity(entity).insert(WallLayoutShadowCasterOff);
                    state.tally.wall_entities += 1;
                }
            } else if state.stencil_only.iter().any(|handle| handle == &material.0) {
                commands
                    .entity(entity)
                    .insert((StencilOnlyPass, Visibility::Hidden));
                state.tally.stencil_hidden_entities += 1;
            } else if state.shadow_mesh.iter().any(|handle| handle == &material.0) {
                commands
                    .entity(entity)
                    .insert((ShadowMeshGated, Visibility::Hidden));
                state.tally.shadow_mesh_entities += 1;
            }
        }
        // A visible glTF scene would first enter the render queues with the
        // importer's material. Publish the prepared hierarchy in the same
        // command batch as the material replacements and hidden source passes.
        commands.entity(root_entity).insert((Visibility::Inherited, FixtureVisualReady));
    }
    for (index, entities) in &swapped_entities {
        let item = &state.planned[*index];
        if item.material.key.fence {
            state.tally.fence_materials += 1;
            state.tally.fence_entities += entities;
        } else if item.material.key.rug.is_some() {
            state.tally.rug_materials += 1;
            state.tally.rug_entities += entities;
        } else {
            state.tally.basic_materials += 1;
            state.tally.basic_entities += entities;
        }
        if item.material.blend {
            state.tally.blended_materials += 1;
        }
        if item.material.key.window_clip {
            state.tally.window_clip_materials += 1;
        }
        if item.material.key.reflection {
            state.tally.reflection_materials += 1;
        }
        if item.reflection_needs_cubemap {
            state
                .tally
                .reflection_needs_cubemap_names
                .push(item.name.clone());
        }
        if !item.unported_keywords.is_empty() {
            state.tally.unported_keyword_materials.push(format!(
                "{} ({})",
                item.name,
                item.unported_keywords.join(", ")
            ));
        }
    }

    info!(
        "家具材质换装：Basic {} 材质 {} 实体（混合 {} 材质，窗外观 {} 材质，反射折叠 {} 材质）；墙布局标记 {} 实体；\
         写零 pass 隐藏 {} 材质 {} 实体；ShadowMesh 族隐藏 {} 材质 {} 实体；\
         未实现 keyword 分支 {} 材质",
        state.tally.basic_materials,
        state.tally.basic_entities,
        state.tally.blended_materials,
        state.tally.window_clip_materials,
        state.tally.reflection_materials,
        state.tally.wall_entities,
        state.tally.stencil_hidden_names.len(),
        state.tally.stencil_hidden_entities,
        state.tally.shadow_mesh_names.len(),
        state.tally.shadow_mesh_entities,
        state.tally.unported_keyword_materials.len(),
    );
    if state.tally.fence_entities > 0 {
        info!("Fence 材质换装：{} 材质 {} 实体；源队列 2065 + priority；无 alpha clip / fog / treasure shadow / emission", state.tally.fence_materials, state.tally.fence_entities);
    }
    if state.tally.rug_entities > 0 {
        info!("Rug 材质换装：{} 材质 {} 实体；源队列 2007/2008 + priority；ZTest Always / ZWrite Off", state.tally.rug_materials, state.tally.rug_entities);
    }
    if !state.tally.stencil_hidden_names.is_empty() {
        info!(
            "写零 pass（ColorMask 0 + ZWrite 0，只写模板缓冲）：{:?}",
            state.tally.stencil_hidden_names
        );
    }
    if !state.tally.shadow_mesh_names.is_empty() {
        info!(
            "ShadowMesh 族（源片元以 _Show 为门且 live 数据恒 0，像素贡献为 0）：{:?}",
            state.tally.shadow_mesh_names
        );
    }
    // 逐参数采样日志（验收判据：光色/阈值/ST 实际值可从日志推导）。
    // 阈值三件按 _OverrideShadingParameter 现选局部或全局，两侧都印——
    // 选择本身也是被推导的对象。
    for item in &state.planned {
        let p = &item.material.params;
        let use_local = p.override_shading_parameter > 0.5;
        let (threshold, smoothness, intensity) = if use_local {
            (
                p.local_edge_threshold,
                p.local_edge_smoothness,
                p.local_shading_intensity,
            )
        } else {
            (
                env.globals.edge_threshold,
                env.globals.edge_smoothness,
                1.0,
            )
        };
        // 自发光账目行：按换装当帧的现象类型推门值与贡献（门 × 遮罩
        // 均值；门 0 贡献 [0,0,0]，门开而遮罩缺/不可读记 None）。
        let emission_type = env.emission_type as i32;
        let gate_now = emission_gate(&item.emission, emission_type);
        let mask_mean = account
            .rows
            .iter()
            .find(|row| row.name == item.name)
            .and_then(|row| row.mask_mean);
        info!(
            "家具材质采样 {name}：现象光门 {gate} 阈值 {threshold:.4}（局部 {lt:.4}/全局 {gt:.4}）\
             平滑度 {smoothness:.4}（局部 {ls:.4}/全局 {gs:.4}）强度 {intensity:.2}（局部 {li:.2}/全局 1.00）\
             阴面色 {shade:?} 光色 {light:?} uv=uv0+({ox:.2},{oy:.2}) 数据侧 ST {st:?}（源变体不乘 ST）\
             变体 clip={clip} dither={dither}（ditherAlpha={da:.2}）rso={rso}（源程序级零效果）\
             fresnel={fresnel}（power={fp:.2} color={fc:?}；live 带它的材质全未摆放）\
             reflection={reflection}（power={rp:.2} intensity={ri:.4} 立方图采样折叠为常数 {cube:.7}\
             ⇒ 掠射角每通道加 {edge:.7}）\
             未实现={unported:?} blend={blend}\
             自发光 亮臂 {bright:?} 暗臂 {dark:?}（现象类型 {etype} 门 {egate:?}，遮罩均值 {emean:?} ⇒ 贡献 {contrib:?}）",
            name = item.name,
            gate = p.use_phenomena_lighting,
            threshold = threshold,
            lt = p.local_edge_threshold,
            gt = env.globals.edge_threshold,
            smoothness = smoothness,
            ls = p.local_edge_smoothness,
            gs = env.globals.edge_smoothness,
            intensity = intensity,
            li = p.local_shading_intensity,
            shade = env.globals.phenomena_shade_color,
            light = env.globals.phenomena_directional_light_color,
            ox = p.main_tex_offset[0],
            oy = p.main_tex_offset[1],
            st = item.main_tex_st,
            clip = item.material.key.alpha_clip,
            dither = item.material.key.dither,
            da = p.dither_alpha,
            rso = item.receive_shadows_off,
            fresnel = item.material.key.fresnel,
            fp = item.material.fresnel_power,
            fc = item.material.fresnel_color,
            reflection = item.material.key.reflection,
            rp = item.material.reflection_power,
            ri = item.material.reflection_intensity,
            cube = moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE,
            // 掠射角（N·V → 0，上钳后 fresnel = 1）时这条分支给每个通道
            // 加的量：采样常数 × intensity。正对相机时该项趋 0，所以这个
            // 数是它的上界，也是画面上那圈边缘高光的强度。
            edge = moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE
                * item.material.reflection_intensity,
            unported = item.unported_keywords,
            blend = item.material.blend,
            bright = item.emission.bright,
            dark = item.emission.dark,
            etype = emission_type,
            egate = gate_now,
            emean = mask_mean,
            contrib = match (gate_now, mask_mean) {
                (Some(1), Some(mean)) => Some(mean),
                (Some(0), _) => Some([0.0; 3]),
                _ => None,
            },
        );
        // 窗外观支路对账：落点（26/27 而非 5/6）与门四角——「窗族走对
        // 支路」从日志可推导。
        if item.material.key.window_clip {
            let corners = item.material.params.clip_corners;
            info!(
                "窗外观支路 {name}：二维表落点 attr {attribute}（26 不透明/27 混合，非 5/6），\
                 屏空间门四角（写零面顶点，本网格局部系，环序）[{a:?} {b:?} {c:?} {d:?}]",
                name = item.name,
                attribute = item.attribute,
                a = corners[0],
                b = corners[1],
                c = corners[2],
                d = corners[3],
            );
        }
    }
    // 全局量实况：换装完成当帧的 SiteEnv（含天气系统接管后的值）。
    info!(
        "站点全局量实况（换装时）：光向 {:?} 光色 {:?} 阴面色 {:?} 全局阈值 {} 平滑度 {} \
         mip 偏置 {:?} 宝藏位 {:?}/{:?} 宝藏强度 {:?}",
        env.globals.light_vector,
        env.globals.phenomena_directional_light_color,
        env.globals.phenomena_shade_color,
        env.globals.edge_threshold,
        env.globals.edge_smoothness,
        env.globals.mip_bias,
        env.globals.treasure_positions[0],
        env.globals.treasure_positions[1],
        env.globals.treasure_shadow_intensity,
    );
    if !state.tally.refused.is_empty() {
        warn!(
            "家具具名拒绝 {} 条：{:?}",
            state.tally.refused.len(),
            state.tally.refused
        );
    }
    if !state.tally.unported.is_empty() {
        warn!(
            "闭集内非 Basic 族（本单范围外，保留原材质）{} 条：{:?}",
            state.tally.unported.len(),
            state.tally.unported
        );
    }
    if !state.tally.unported_keyword_materials.is_empty() {
        warn!(
            "未实现 keyword 分支（源变体开着、本管线没有的分支，逐材质具名）{} 条：{:?}",
            state.tally.unported_keyword_materials.len(),
            state.tally.unported_keyword_materials
        );
    }
    if !state.tally.reflection_needs_cubemap_names.is_empty() {
        warn!(
            "反射分支开着但材质绑了真立方图（未绑定时的常数折叠对它们是错的，\
             本管线没有真采样那条路 ⇒ 分支未开）{} 条：{:?}",
            state.tally.reflection_needs_cubemap_names.len(),
            state.tally.reflection_needs_cubemap_names
        );
    }
    // 账本入册：天气系统切档时按材质名逐行报自发光贡献（贡献行从这份
    // 账本现算——各臂可由日志推导）。
    if swapped.is_none() {
        commands.insert_resource(account);
        commands.insert_resource(FixtureMaterialsSwapped);
    }
}

/// 建 plan：从每个家具根实体向下走层级，按 glb 材质句柄收首见实体，
/// 逐条解析（材质名 + extras + 主贴图句柄）。墙布局标记在执行阶段
/// 逐实体插（同一句柄可能同时出现在墙内与非墙摆放下）。窗外观支路
/// 的门四角按「首见根」从同一 glb 的写零面网格读出（同一句柄的多次
/// 摆放共享同一 glb，四角相同）。
#[allow(clippy::type_complexity)]
fn build_swap_plan(
    roots: &PendingVisuals,
    children: &Query<&Children>,
    parts: &Query<(
        &MeshMaterial3d<StandardMaterial>,
        &GltfMaterialName,
        &GltfMaterialExtras,
    )>,
    mesh_parts: &Query<&Mesh3d>,
    meshes: &Assets<Mesh>,
    std_materials: &Assets<StandardMaterial>,
    source_textures: &Query<&SourceMaterialTextures>,
) -> Option<SwapPlan> {
    // 首见句柄 → (材质名, extras 原文, 所在根)。同句柄的实体共享同一次
    // 解析；根记录给窗外观门四角定位写零面用。
    let mut seen: HashMap<Handle<StandardMaterial>, (String, String, Entity, Entity)> = HashMap::new();
    // 句柄 → [(根, 网格实体)]：写零面的网格位置从这里读。
    let mut handle_entities: HashMap<Handle<StandardMaterial>, Vec<(Entity, Entity)>> =
        HashMap::new();
    let mut tally = SwapTally::default();
    let mut planned = Vec::new();
    let mut stencil_only = Vec::new();
    let mut shadow_mesh = Vec::new();
    for (root_entity, _) in roots.iter() {
        let mut stack = vec![root_entity];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter().map(|child| child));
            }
            let Ok((material, name, extras)) = parts.get(entity) else {
                continue;
            };
            seen.entry(material.0.clone()).or_insert_with(|| {
                (name.0.clone(), extras.value.clone(), root_entity, entity)
            });
            handle_entities
                .entry(material.0.clone())
                .or_default()
                .push((root_entity, entity));
        }
    }
    // 第一遍：逐句柄解析 extras 分类。写零面 / ShadowMesh / 范围外 /
    // 拒绝在这里落账；Basic 挂起，等写零面四角收齐后再解析。
    struct PendingBasic {
        name: String,
        extras: serde_json::Value,
        root: Entity,
        entity: Entity,
        source: Handle<StandardMaterial>,
    }
    let mut pending_basic: Vec<PendingBasic> = Vec::new();
    for (handle, (name, extras_json, root, entity)) in &seen {
        let extras: serde_json::Value = match serde_json::from_str(extras_json) {
            Ok(value) => value,
            Err(err) => {
                tally
                    .refused
                    .push(format!("家具材质 {name} 的 extras 不是合法 JSON：{err}"));
                continue;
            }
        };
        // 渲染状态「写零」：先于族门判——渲染状态由材质浮点驱动，与
        // shader 名无关；被拒族的写零材质同样一个像素都不画，保留原材质
        // 只会把不可见网格画成白板。
        if is_stencil_only(&extras) {
            tally.stencil_hidden_names.push(name.clone());
            stencil_only.push(handle.clone());
            continue;
        }
        // 族门：具名拒绝（URP/Lit）与未知名字走拒绝；闭集内非 Basic 走
        // 范围外套。ShadowMesh 单独一档：源里恒 0 像素，保留原材质会
        // 画成错误的实心面，收进隐藏集。
        let shader = match extras.get("shader").and_then(|value| value.as_str()) {
            Some(shader) => shader,
            None => {
                tally
                    .refused
                    .push(format!("家具材质 {name} 的 extras 缺 shader 名"));
                continue;
            }
        };
        if let Err(reason) = check_family_shader(shader) {
            tally.refused.push(reason);
            continue;
        }
        if shader == SHADOWMESH_SHADER {
            tally.shadow_mesh_names.push(name.clone());
            shadow_mesh.push(handle.clone());
            continue;
        }
        // Road owns a separate runtime material and neighborhood path.
        if shader == "Mysekai/Fixture/Road" { continue; }
        if surfaces::owns_shader(shader) { continue; }
        if shader != BASIC_SHADER && shader != RUG_SHADER && shader != FENCE_SHADER {
            tally.unported.push(format!("{name} ({shader})"));
            continue;
        }
        pending_basic.push(PendingBasic {
            name: name.clone(),
            extras,
            root: *root,
            entity: *entity,
            source: handle.clone(),
        });
    }
    // 写零面四角：逐根收集写零材质的全部网格唯一顶点，验形（恰好四点、
    // 平面、凸）后排环序。四点以外的形状或多张不可合并的写零面都具名
    // 记 Err——usage=1 材质解析时按 Err 拒绝，不静默放行。
    let mut stencil_points: HashMap<Entity, Vec<Vec3>> = HashMap::new();
    for handle in &stencil_only {
        for &(root, entity) in handle_entities.get(handle).into_iter().flatten() {
            let points = stencil_points.entry(root).or_default();
            let Ok(mesh_handle) = mesh_parts.get(entity) else {
                continue;
            };
            let Some(mesh) = meshes.get(&mesh_handle.0) else {
                continue;
            };
            let Some(positions) = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3())
            else {
                continue;
            };
            for position in positions {
                let vertex = Vec3::from_slice(position);
                if !points.contains(&vertex) {
                    points.push(vertex);
                }
            }
        }
    }
    let mut clip_quads: HashMap<Entity, Result<[Vec3; 4], String>> = HashMap::new();
    for (root, points) in stencil_points {
        if points.is_empty() {
            clip_quads.insert(root, Err("写零面网格没有可读的顶点属性".into()));
            continue;
        }
        if points.len() != 4 {
            clip_quads.insert(
                root,
                Err(format!(
                    "写零面的唯一顶点 {} 个（门是四边形判定，期望 4）",
                    points.len()
                )),
            );
            continue;
        }
        let quad = order_clip_quad([points[0], points[1], points[2], points[3]]);
        clip_quads.insert(root, quad);
    }
    // 第二遍：解析 Basic 材质。窗外观支路的门四角按首见根取。
    for PendingBasic {
        name,
        extras,
        root,
        entity,
        source,
    } in pending_basic
    {
        // 主贴图：glb 材质的 baseColorTexture（提取侧把 _MainTex 写进它）。
        let main_tex = match std_materials
            .get(&source)
            .and_then(|material| material.base_color_texture.clone())
        {
            Some(handle) => handle,
            None => {
                tally.refused.push(format!(
                    "家具材质 {name} 的 glb 材质没有 baseColorTexture"
                ));
                continue;
            }
        };
        let resolved = if extras.get("shader").and_then(|v| v.as_str()) == Some(FENCE_SHADER) {
            resolve_fence(&name, &extras, main_tex.clone())
        } else if extras.get("shader").and_then(|v| v.as_str()) == Some(RUG_SHADER) {
            resolve_rug(&name, &extras, main_tex.clone())
        } else {
            resolve_basic(&name, &extras, main_tex.clone(), clip_quads.get(&root))
        };
        match resolved {
            Ok(resolved) => {
                // Import retains extras-only textures in the scene. Reuse that
                // handle: loading a dropped TextureN label here reloads the GLB
                // and respawns entities after their materials were installed.
                let mask = match resolved.mask_index {
                    Some(index) => {
                        match source_textures.get(entity).ok()
                            .and_then(|textures| textures.0.get("_EmissionMaskTex")).cloned() {
                            Some(handle) => Some(handle),
                            None => {
                                tally.refused.push(format!(
                                    "家具材质 {name} 的遮罩 Texture{index} 未由 glTF 装载器保留"
                                ));
                                continue;
                            }
                        }
                    }
                    None => source_textures.get(entity).ok()
                        .and_then(|textures| textures.0.get("_EmissionMaskTex")).cloned(),
                };
                planned.push(Planned {
                    force_emission: resolved.force_emission,
                    name: name.clone(),
                    material: resolved.material,
                    source,
                    attribute: resolved.attribute,
                    emission: resolved.emission,
                    receive_shadows_off: resolved.receive_shadows_off,
                    unported_keywords: resolved.unported_keywords,
                    reflection_needs_cubemap: resolved.reflection_needs_cubemap,
                    mask,
                    main_tex_st: extras
                        .get("textureScaleOffset")
                        .and_then(|st| st.get("_MainTex"))
                        .and_then(|value| value.as_array())
                        .and_then(|array| {
                            let mut st = [0.0; 4];
                            for (slot, value) in st.iter_mut().zip(array) {
                                *slot = value.as_f64()?;
                            }
                            Some(st)
                        }),
                });
            }
            Err(reason) => tally.refused.push(reason),
        }
    }
    Some(SwapPlan {
        planned,
        stencil_only,
        shadow_mesh,
        tally,
    })
}

/// 家具材质插件：材质管线 + 换装。全局量桥（`SiteEnvGpuBuffer`）由
/// 站点材质插件装着，这里只消费——同 buffer 同槽序，两边一起改。
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct FixtureMaterialSet;

pub struct FixtureMaterialPlugin;

impl Plugin for FixtureMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<FixtureMaterial>::default())
            .add_plugins(surfaces::FixtureSurfacePlugin)
            .add_systems(Update, switch_materials.in_set(FixtureMaterialSet)
                .after(crate::fixture::FixtureLayoutSet).after(crate::fixture_colors::prepare)
                .run_if(crate::fixture_colors::ready));
        bevy::asset::embedded_asset!(app, "shaders/fixture_material.wgsl");
    }
}
