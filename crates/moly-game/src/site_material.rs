//! 站点材质：八个站点族的 Base 程序共用一个 Bevy Material。
//!
//! 材质值来自 sidecar 的 [`MaterialSlot`]（key 是 Unity 属性名的原拼写），
//! 族与 keyword 变体由 [`SiteMaterialKey`] 编码进管线特化；WGSL 在
//! `shaders/site_material.wgsl`，八族共用一份、按宏分派。
//!
//! 族门（按真源重定）：门 = 「序列化 keyword 全部落在本族的
//! 已实现轴清单内」+「族要求的必在键都在」+「值域开关在已实现的取值上」。
//! 关键词集合不再从「我方旧闭集」长出来，而是逐族对着源变体表写；
//! 每条轴（overlay、fresnel、clip、dither、高度淡出、收影、树动画）
//! 在哪个族、以什么形式（编译期 keyword / 运行时标量）生效，都有源
//! 变体逐行钉住。
//!
//! 族门不过、必需槽缺失或值域开关越界保留原材质；缺失 UV1 使用
//! 引擎顶点输入的零值，再按导出坐标约定翻转 V，不改用 UV0。

use std::collections::HashMap;

use bevy::asset::{AssetApp, LoadState};
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::gltf::Gltf;
use bevy::image::Image;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, StandardMaterial};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::{FallbackImage, FallbackImageZero, GpuImage};
use bevy::shader::ShaderRef;
use bevy::image::ImageLoaderSettings;
use moly_assets::sidecar::{MolyJson, MolyJsonLoader, SiteSidecar};

/// 按声明的著色空间装载贴图：sRGB（色彩）走装载器默认，线性数据
/// （法线/遮罩一族）关掉解码——色彩空间的唯一声明在 sidecar 的
/// `textureColourSpace`（提取侧从源纹理的 m_ColorSpace 记录；PNG 本身
/// 不带元数据）。声明缺席（旧产物）按 sRGB：既有口径。
///
/// `dir` 是 `moly://` 之后的目录段：站点场景包 `site/scenes/<site>`、
/// 采集物包 `site/props/<leaf>`（目录布局沿用提取产物）。
///
/// # 线性声明是一道 fail-closed 联锁
///
/// `shaders/site_material.wgsl` 采样后**无条件**把值编回存储（gamma）域
/// ——真源是 gamma 色彩空间构建，整条片元链在存储域上算。那一步只有在
/// 硬件确实解码过（= 纹理按 sRGB 装载）时才是恒等的往返。
///
/// 走这个函数的贴图**全部**被当作颜色消费（八族解析与采集物复用
/// `resolve_fieldobject` / `resolve_tree` 的臂只装 `_MainTex`、两层
/// `_OverlayColorMap`、`_LeafMaskTex`，后者也是拿存储值比阈值）⇒ 一张
/// 线性声明的贴图进到这里，着色器会**多编一次**、那个材质整体偏亮，而那
/// 是个静默的错。现算（2026-09-09，按包目录点名数 scenes 8 包 + props 62 包
/// 的已绑定槽实例）：本族消费的四个槽 `_MainTex` 387 · `_OverlayColorMap` 33
/// · `_OverlayColorMap2nd` 11 · `_LeafMaskTex` 1 = **432/432 全声明 sRGB**，
/// 整份产物里唯一的线性声明是一张 `_NormalTexture`，而本族没有法线槽、
/// 它没有消费者 ⇒ 当前数据下这条支路走不到。
/// ⇒ 所以着色器那边不做运行时旋钮（不为一个枚举不出来的输入加每帧分支），
/// 换成在这里具名喊：**提取产物哪天真给出一张线性的颜色贴图，先听见再说。**
pub(crate) fn load_dir_texture(
    server: &AssetServer,
    sidecar: &SiteSidecar,
    dir: &str,
    uri: &str,
) -> Handle<Image> {
    let path = bevy::asset::AssetPath::from(format!("moly://{dir}/{uri}"));
    match sidecar.texture_colour_space.get(uri) {
        Some(false) => {
            warn!(
                "站点/采集物贴图 {dir}/{uri} 的色彩空间声明是线性，而站点着色器\
                 采样后无条件编回存储域（源是 gamma 空间构建）⇒ 这张会被多编\
                 一次、该材质整体偏亮。装载按声明照做（不擅自改域），但这条\
                 前提已经不成立：要么提取侧的声明错了，要么本族真的开始消费\
                 非颜色贴图、着色器需要一个按槽的域旋钮。"
            );
            server.load_with_settings::<Image, _>(path, |settings: &mut ImageLoaderSettings| {
                settings.is_srgb = false;
            })
        }
        _ => server.load::<Image>(path),
    }
}

/// 站点场景包的贴图装载（[`load_dir_texture`] 的站点专形）。
fn load_site_texture(
    server: &AssetServer,
    sidecar: &SiteSidecar,
    site: &str,
    uri: &str,
) -> Handle<Image> {
    load_dir_texture(server, sidecar, &format!("site/scenes/{site}"), uri)
}
use moly_law::material::{texture_slot, FloatLookup, MaterialSlot};
use moly_law::shading::fieldobject;

use crate::env::SiteEnvGpuBuffer;
use crate::shadowmap::ShadowMapGpu;
use crate::site::{SiteAssets, SiteScenesReady};

/// Ground-Birthday 族名（真源 shader 名）。law 侧只收了四个老族；
/// 这四个新族的族名在本层声明。
pub const GROUND_BIRTHDAY_SHADER_NAME: &str = "Mysekai/Site/Ground-Birthday";
/// Object 族名。
pub const OBJECT_SHADER_NAME: &str = "Mysekai/Object";
/// DropItem 族名。
pub const DROPITEM_SHADER_NAME: &str = "Mysekai/DropItem";
/// UI-Uber 族名。
pub const UI_UBER_SHADER_NAME: &str = "Mysekai/Effect/UI-Uber";

/// 材质 uniform 的槽数与字节数。槽序是本文件与
/// `shaders/site_material.wgsl` 里 `SiteParams` 结构体之间的契约，两边同改。
pub const PARAMS_SLOTS: usize = 46;
pub const PARAMS_BYTES: usize = PARAMS_SLOTS * 16;

/// 每条 Unity 属性一个 vec4 槽；族不消费的槽写零。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiteParams {
    /// `_UseVertexColorBlend`：FieldObject/Object 是浮点 lerp，其余族是
    /// 布尔选择（Ground/Water/Birthday 作用在平方色上，Tree 在原始色上）。
    pub use_vertex_color_blend: f32,
    /// `_AlphaClip`：FieldObject/Object 的 uniform clip 阈值；其余族的
    /// clip 是编译期常量 0.5，不消费它。
    pub alpha_clip: f32,
    /// `_BaseOpacity`。DropItem 与 Tree 不消费（alpha 另有出处）。
    pub base_opacity: f32,
    /// `_DitherAlpha`。FieldObject 恒抖动；Birthday 在 !_DISABLE_DITHER
    /// 时抖动；Tree/Object/DropItem/UI-Uber 无抖动轴。
    pub dither_alpha: f32,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
    pub fresnel_power: f32,
    /// `_UseFresnel`。FieldObject 的运行时门是 0.5 比较；Object 的运行时
    /// 门是精确等 1；Ground/Water/Tree 的 fresnel 是 keyword 编译期的，
    /// 这个槽恒零。
    pub use_fresnel: f32,
    pub fresnel_color: [f32; 4],
    /// `_TextureCoord_Overlay1st`。Ground/Water/Birthday/FieldObject 的
    /// 第一层 overlay 专属。
    pub texture_coord_overlay1st: f32,
    /// `_OverlayColorMap` 的 `(scaleX, scaleY, offsetX, offsetY)`。
    pub overlay_st: [f32; 4],
    /// `(_UVScrollX, _UVScrollY)`。Ground/Water/Birthday/Object 专属。
    pub uv_scroll: [f32; 2],
    /// `(_UVScrollX_Overlay1st, _UVScrollY_Overlay1st)`。
    pub uv_scroll_overlay1st: [f32; 2],
    pub use_overlay_texture_vertex_alpha: f32,
    pub use_vertex_alpha_opacity: f32,
    /// `_UsePhenomenaLighting`。Tree/Object 专属：不开则底色直通
    /// （雾也在门内）。
    pub use_phenomena_lighting: f32,
    /// `_TextureCoord_Overlay2nd`。Ground/Water 第二层 overlay 专属。
    pub texture_coord_overlay2nd: f32,
    /// `_OverlayColorMap2nd` 的 `(scaleX, scaleY, offsetX, offsetY)`。
    pub overlay_st_2nd: [f32; 4],
    /// `(_UVScrollX_Overlay2nd, _UVScrollY_Overlay2nd)`。
    pub uv_scroll_overlay2nd: [f32; 2],
    pub use_overlay_texture_vertex_alpha_2nd: f32,
    /// `_turbulenceValue`。Tree 动画（风摆频率因子）。
    pub turbulence: f32,
    /// `_strengthValue`。Tree 动画（风摆幅度因子）。
    pub strength: f32,
    /// `_LeafRotationSpeed`。Tree 动画（叶旋转）。
    pub leaf_rotation_speed: f32,
    /// `_LeafRotationRange`。Tree 动画（叶旋转）。
    pub leaf_rotation_range: f32,
    /// 收影轴（真源 `_RECEIVE_SHADOWS_OFF`）：带此 keyword 的变体在源里
    /// 整族无影针（FO/Tree/Birthday record24/DropItem/Object 的针计数全
    /// 零），解析时按 keyword 定 0/1；不带则 1（Ground/Water 全链）。
    pub receive_shadow: f32,
    /// `(_UVScrollX, _UVScrollY)` 的 Object 副本（Object 用同名键、无
    /// fract 的滚动）。
    pub object_uv_scroll: [f32; 2],
    /// `_Overlay1st_Opacity`。Birthday 第一层 overlay 的门标量。
    pub birthday_overlay_opacity: f32,
    /// `_HeightFadeRcpLength`。Ground/Tree 高度淡出（倒数长度形）。
    pub height_fade_rcp_length: f32,
    /// `_HeightFadeStartTimeRcpLength`。Ground/Tree 高度淡出。
    pub height_fade_start_time_rcp_length: f32,
    /// `_HeightFadeExponent`。三个高度淡出族共用。
    pub height_fade_exponent: f32,
    /// `_UseHeightFade` 的运行时开关。Ground/Tree 的块由 keyword 编译、
    /// 块内运行时选择；Object 的块无 keyword 恒编译、只有运行时选择
    /// （当前数据全 0，块保留以备非零行）。
    pub use_height_fade: f32,
    /// `_HeightFadePosition`。Object 高度淡出（位置-长度形）。
    pub height_fade_position: f32,
    /// `_HeightFadeLength`。Object 高度淡出。
    pub height_fade_length: f32,
    /// `_HeightGradientPos01`。Tree 高度渐变第一段端点。
    pub height_gradient_pos01: f32,
    /// `_HeightGradientPos12`。Tree 高度渐变第二段端点。
    pub height_gradient_pos12: f32,
    pub height_gradient_color0: [f32; 4],
    pub height_gradient_color1: [f32; 4],
    pub height_gradient_color2: [f32; 4],
    /// `_UVSelection`。DropItem 的 uv0/uv1 选择。
    pub dropitem_uv_selection: f32,
    /// `(_MainTex_UVScrollX, _MainTex_UVScrollY)`。DropItem 专属键名。
    pub dropitem_uv_scroll: [f32; 2],
    /// `_MainTex_ST`。UI-Uber 顶点段的 uv 变换。
    pub ui_uber_main_st: [f32; 4],
    /// `_AdditiveColor`。Object 门后无条件加色。
    pub additive_color: [f32; 4],
    /// `_BaseTextureMappingMode`。Object 主贴图坐标源开关。
    pub object_texture_mapping: f32,
    /// `_MainTextureLocalMapping`。Object 的 uv0 覆写开关。
    pub object_main_texture_local_mapping: f32,
}

impl SiteParams {
    /// 全零基底：族不消费的槽写零（死支路加零）。各族解析器在基底上
    /// 逐槽覆写自己消费的键——每族恰好触碰它消费的槽，槽序审查因此
    /// 是逐族可做的。
    fn zeroed() -> Self {
        Self {
            use_vertex_color_blend: 0.0,
            alpha_clip: 0.0,
            base_opacity: 0.0,
            dither_alpha: 0.0,
            override_shading_parameter: 0.0,
            local_shading_intensity: 0.0,
            local_edge_threshold: 0.0,
            local_edge_smoothness: 0.0,
            fresnel_power: 0.0,
            use_fresnel: 0.0,
            fresnel_color: [0.0; 4],
            texture_coord_overlay1st: 0.0,
            overlay_st: [0.0; 4],
            uv_scroll: [0.0; 2],
            uv_scroll_overlay1st: [0.0; 2],
            use_overlay_texture_vertex_alpha: 0.0,
            use_vertex_alpha_opacity: 0.0,
            use_phenomena_lighting: 0.0,
            texture_coord_overlay2nd: 0.0,
            overlay_st_2nd: [0.0; 4],
            uv_scroll_overlay2nd: [0.0; 2],
            use_overlay_texture_vertex_alpha_2nd: 0.0,
            turbulence: 0.0,
            strength: 0.0,
            leaf_rotation_speed: 0.0,
            leaf_rotation_range: 0.0,
            receive_shadow: 0.0,
            object_uv_scroll: [0.0; 2],
            birthday_overlay_opacity: 0.0,
            height_fade_rcp_length: 0.0,
            height_fade_start_time_rcp_length: 0.0,
            height_fade_exponent: 0.0,
            use_height_fade: 0.0,
            height_fade_position: 0.0,
            height_fade_length: 0.0,
            height_gradient_pos01: 0.0,
            height_gradient_pos12: 0.0,
            height_gradient_color0: [0.0; 4],
            height_gradient_color1: [0.0; 4],
            height_gradient_color2: [0.0; 4],
            dropitem_uv_selection: 0.0,
            dropitem_uv_scroll: [0.0; 2],
            ui_uber_main_st: [0.0; 4],
            additive_color: [0.0; 4],
            object_texture_mapping: 0.0,
            object_main_texture_local_mapping: 0.0,
        }
    }

    /// 按上面的槽序摊平成字节。
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        for slot in [
            [self.use_vertex_color_blend, 0.0, 0.0, 0.0],
            [self.alpha_clip, 0.0, 0.0, 0.0],
            [self.base_opacity, 0.0, 0.0, 0.0],
            [self.dither_alpha, 0.0, 0.0, 0.0],
            [self.override_shading_parameter, 0.0, 0.0, 0.0],
            [self.local_shading_intensity, 0.0, 0.0, 0.0],
            [self.local_edge_threshold, 0.0, 0.0, 0.0],
            [self.local_edge_smoothness, 0.0, 0.0, 0.0],
            [self.fresnel_power, 0.0, 0.0, 0.0],
            [self.use_fresnel, 0.0, 0.0, 0.0],
            self.fresnel_color,
            [self.texture_coord_overlay1st, 0.0, 0.0, 0.0],
            self.overlay_st,
            [self.uv_scroll[0], self.uv_scroll[1], 0.0, 0.0],
            [
                self.uv_scroll_overlay1st[0],
                self.uv_scroll_overlay1st[1],
                0.0,
                0.0,
            ],
            [self.use_overlay_texture_vertex_alpha, 0.0, 0.0, 0.0],
            [self.use_vertex_alpha_opacity, 0.0, 0.0, 0.0],
            [self.use_phenomena_lighting, 0.0, 0.0, 0.0],
            [self.texture_coord_overlay2nd, 0.0, 0.0, 0.0],
            self.overlay_st_2nd,
            [self.uv_scroll_overlay2nd[0], self.uv_scroll_overlay2nd[1], 0.0, 0.0],
            [self.use_overlay_texture_vertex_alpha_2nd, 0.0, 0.0, 0.0],
            [self.turbulence, 0.0, 0.0, 0.0],
            [self.strength, 0.0, 0.0, 0.0],
            [self.leaf_rotation_speed, 0.0, 0.0, 0.0],
            [self.leaf_rotation_range, 0.0, 0.0, 0.0],
            [self.receive_shadow, 0.0, 0.0, 0.0],
            [self.object_uv_scroll[0], self.object_uv_scroll[1], 0.0, 0.0],
            [self.birthday_overlay_opacity, 0.0, 0.0, 0.0],
            [self.height_fade_rcp_length, 0.0, 0.0, 0.0],
            [self.height_fade_start_time_rcp_length, 0.0, 0.0, 0.0],
            [self.height_fade_exponent, 0.0, 0.0, 0.0],
            [self.use_height_fade, 0.0, 0.0, 0.0],
            [self.height_fade_position, 0.0, 0.0, 0.0],
            [self.height_fade_length, 0.0, 0.0, 0.0],
            [self.height_gradient_pos01, 0.0, 0.0, 0.0],
            [self.height_gradient_pos12, 0.0, 0.0, 0.0],
            self.height_gradient_color0,
            self.height_gradient_color1,
            self.height_gradient_color2,
            [self.dropitem_uv_selection, 0.0, 0.0, 0.0],
            [self.dropitem_uv_scroll[0], self.dropitem_uv_scroll[1], 0.0, 0.0],
            self.ui_uber_main_st,
            self.additive_color,
            [self.object_texture_mapping, 0.0, 0.0, 0.0],
            [self.object_main_texture_local_mapping, 0.0, 0.0, 0.0],
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), PARAMS_BYTES);
        bytes
    }
}

/// 站点族。shader 名精确分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SiteFamily {
    FieldObject,
    Ground,
    Tree,
    Water,
    GroundBirthday,
    Object,
    DropItem,
    UiUber,
}

/// 管线特化键：族 + keyword 变体。作为 `AsBindGroup::Data` 进管线缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SiteMaterialKey {
    pub family: SiteFamily,
    /// `_USE_OVERLAY_TEXTURE`：第一层 overlay。四族共用同一式子
    /// （Birthday 的门多乘一个 opacity 标量，FO 的门不乘）。
    pub overlay_1st: bool,
    /// `_USE_OVERLAY_TEXTURE_2ND`：第二层 overlay。Ground/Water 专属；
    /// 与第一层同开的组合在源变体表里不存在，解析层拒绝。
    pub overlay_2nd: bool,
    /// `_ENABLE_MODULE_FRESNEL`：keyword 编译期 fresnel。Ground/Water
    /// 在链尾、Tree 在现象光门内第一；FO 的 fresnel 是运行时开关
    /// （不走这个键）。
    pub module_fresnel: bool,
    /// `_USE_TREE_ANIMATION`：树动画（风摆 + 叶旋转）。Tree 专属；
    /// Birthday 的同名 keyword 惰性（恒 false）。
    pub tree_animation: bool,
    /// `_USE_ALPHA_CLIP` 的 uniform 阈形式：`tex.a − _AlphaClip < 0`
    /// 丢弃。FieldObject/Object。
    pub uniform_alpha_clip: bool,
    /// `_USE_ALPHA_CLIP` 的常量阈形式：`tex.a − 0.5 < 0` 丢弃。
    /// Tree/DropItem。
    pub const_alpha_clip: bool,
    /// `_USE_ALPHA_CLIP` 的被选 alpha 形式：`selected − 0.5 < 0` 丢弃
    /// （selected = vao ? tex.a·vc.w : tex.a）。Ground/Water/Birthday。
    pub selected_alpha_clip: bool,
    /// `_USE_HEIGHT_FADE`：Ground 的高度淡出（倒数长度形，雾后）。
    pub ground_height_fade: bool,
    /// `_USE_HEIGHT_FADE`：Tree 的高度渐变（三色两段，门内末尾）。
    pub tree_height_fade: bool,
    /// `!_DISABLE_DITHER`：Birthday 的抖动（量化 0.125；FO 恒抖动不
    /// 走这个键，Tree 全带 _DD 无抖动）。
    pub birthday_dither: bool,
}

/// 站点族的材质资产。
#[derive(Debug, Clone, TypePath, Asset)]
pub struct SiteMaterial {
    pub key: SiteMaterialKey,
    pub params: SiteParams,
    pub main_tex: Handle<Image>,
    /// overlay 变体的第二张贴图；族不变体没有它，绑 fallback。
    pub overlay_tex: Option<Handle<Image>>,
    /// 第二层 overlay 的贴图；同上。
    pub overlay2nd_tex: Option<Handle<Image>>,
    /// Tree 动画变体的叶遮罩贴图（顶点阶段采样）；无动画变体没有它。
    pub leaf_mask_tex: Option<Handle<Image>>,
}

impl AsBindGroup for SiteMaterial {
    type Data = SiteMaterialKey;
    type Param = (
        SRes<SiteEnvGpuBuffer>,
        SRes<ShadowMapGpu>,
        SRes<RenderAssets<GpuImage>>,
        SRes<FallbackImage>,
        SRes<FallbackImageZero>,
    );

    fn label() -> &'static str {
        "site_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        render_device: &RenderDevice,
        (env_buffer, shadow, images, fallback, fallback_zero): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let main = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let overlay = match &self.overlay_tex {
            Some(handle) => images
                .get(handle)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        let overlay2nd = match &self.overlay2nd_tex {
            Some(handle) => images
                .get(handle)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        // 叶遮罩缺席时绑全黑零纹理而不是白 fallback：门的比较是严格大于，
        // 源侧未绑定槽采样的默认灰 0.5 过不了这个门——零纹理同样过不了，
        // 两侧都等价于「门关」，不会凭空打开叶旋转。
        let leaf_mask = match &self.leaf_mask_tex {
            Some(handle) => images
                .get(handle)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &**fallback_zero,
        };
        // 站点贴图按源语义平铺（uv 滚动与 ST 缩放都会越出 [0,1]）；
        // GpuImage 自带的 sampler 是引擎默认的 ClampToEdge，在这里换成
        // 三向 Repeat、过滤模式与引擎默认图像 sampler 同为三级线性。
        let repeat = render_device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..Default::default()
        });
        let bindings = BindingResources(vec![
            // binding 0：材质自己的参数块。
            (0, OwnedBindingResource::Data(OwnedData(self.params.bytes()))),
            // binding 1：全局量，全部站点材质共用一个 buffer。
            (1, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (3, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, repeat.clone())),
            (
                4,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    overlay.texture_view.clone(),
                ),
            ),
            (
                5,
                OwnedBindingResource::Sampler(SamplerBindingType::Filtering, repeat.clone()),
            ),
            (
                6,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    overlay2nd.texture_view.clone(),
                ),
            ),
            (
                7,
                OwnedBindingResource::Sampler(SamplerBindingType::Filtering, repeat.clone()),
            ),
            (
                8,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    leaf_mask.texture_view.clone(),
                ),
            ),
            (9, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, repeat)),
            // binding 10：主光阴影消费块（矩阵 + 尺寸 + 参数），全族共用。
            (
                10,
                OwnedBindingResource::Buffer(shadow.consumer_buffer.clone()),
            ),
            // binding 11/12：深度图 + 比较采样器（pcf9 的硬件比较口）。
            (
                11,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    shadow.depth_view.clone(),
                ),
            ),
            (
                12,
                OwnedBindingResource::Sampler(
                    SamplerBindingType::Comparison,
                    shadow.cmp_sampler.clone(),
                ),
            ),
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
        // 叶遮罩在顶点阶段采样（源的叶旋转门），可见性要带上 VERTEX。
        let texture_vertex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
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
        let sampler_vertex = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        };
        vec![
            uniform(0),
            uniform(1),
            texture(2),
            sampler(3),
            texture(4),
            sampler(5),
            texture(6),
            sampler(7),
            texture_vertex(8),
            sampler_vertex(9),
            // binding 10：主光阴影消费块（shadowmap.rs 每帧写）。
            uniform(10),
            // binding 11：主光深度图（shadowmap.rs 一次成图的单张）。
            BindGroupLayoutEntry {
                binding: 11,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Depth,
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // binding 12：比较采样器（lod0 单 texel 硬件深度比较）。
            BindGroupLayoutEntry {
                binding: 12,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Comparison),
                count: None,
            },
        ]
    }
}

impl Material for SiteMaterial {
    // embedded 源的键前缀是调用方 crate 的 lib 名（module_path 的根），
    // 包名 moly-game 的 lib 名是 moly_game（连字符转下划线）。
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/site_material.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/site_material.wgsl".into()
    }

    /// 站点族不进 prepass：本单只交付 Base 片元，阴影消费方不在范围里。
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
        let key = key.bind_group_data;
        // CN GraphicsConfig: ground=2005, drop item=2010, field object=2040,
        // tree=2050. Transparent water keeps its separate sorted phase.
        crate::material_order::set_queue(descriptor, match key.family {
            SiteFamily::Ground | SiteFamily::GroundBirthday => 2005,
            SiteFamily::DropItem => 2010,
            SiteFamily::FieldObject => 2040,
            SiteFamily::Tree => 2050,
            _ => 2065,
        });
        let mut defs: Vec<&str> = Vec::new();
        match key.family {
            SiteFamily::FieldObject => defs.push("SITE_FIELDOBJECT"),
            // SITE_SCROLL 标记 Ground/Water/Ground-Birthday 三族共用的
            // 「滚动族」形状：顶点段（滚动 uv、视线、平方色、overlay 坐标）
            // 与片元共用臂都由它选。
            SiteFamily::Ground => {
                defs.push("SITE_GROUND");
                defs.push("SITE_SCROLL");
            }
            SiteFamily::Tree => defs.push("SITE_TREE"),
            SiteFamily::Water => {
                defs.push("SITE_WATER");
                defs.push("SITE_SCROLL");
            }
            SiteFamily::GroundBirthday => {
                defs.push("SITE_GROUND_BIRTHDAY");
                defs.push("SITE_SCROLL");
            }
            SiteFamily::Object => defs.push("SITE_OBJECT"),
            SiteFamily::DropItem => defs.push("SITE_DROPITEM"),
            SiteFamily::UiUber => {
                defs.push("SITE_UI_UBER");
                // 源 _SrcBlend=5（SrcAlpha）/_DstBlend=10（OneMinusSrcAlpha）：
                // 标准 alpha blend 状态。
                if let Some(fragment) = descriptor.fragment.as_mut() {
                    if let Some(target) =
                        fragment.targets.get_mut(0).and_then(|target| target.as_mut())
                    {
                        target.blend = Some(BlendState::ALPHA_BLENDING);
                    }
                }
            }
        }
        if key.overlay_1st {
            defs.push("SITE_OVERLAY_1ST");
        }
        if key.overlay_2nd {
            defs.push("SITE_OVERLAY_2ND");
        }
        if key.module_fresnel {
            defs.push("SITE_MODULE_FRESNEL");
        }
        if key.tree_animation {
            defs.push("SITE_TREE_ANIMATION");
        }
        if key.uniform_alpha_clip {
            defs.push("SITE_UNIFORM_ALPHA_CLIP");
        }
        if key.const_alpha_clip {
            defs.push("SITE_CONST_ALPHA_CLIP");
        }
        if key.selected_alpha_clip {
            defs.push("SITE_SELECTED_ALPHA_CLIP");
        }
        if key.ground_height_fade {
            defs.push("SITE_GROUND_HEIGHT_FADE");
        }
        if key.tree_height_fade {
            defs.push("SITE_TREE_HEIGHT_FADE");
        }
        if key.birthday_dither {
            defs.push("SITE_BIRTHDAY_DITHER");
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

// ---- 族门与解析 ----

/// 各族的 keyword 全集 = 本族已实现的轴清单。门 = 序列化 keyword 全部
/// 落在清单内；清单外的 keyword 属于未移植变体，具名拒绝。全集对着源
/// 变体表逐族写（源变体逐行钉住每条轴的式子）。
///
/// FieldObject：`_RECEIVE_SHADOWS_OFF` 必在（全活体行带它，族内片元无
/// 影针）；`_USE_ALPHA_CLIP` 选 uniform 阈 clip；`_USE_OVERLAY_TEXTURE`
/// 选第一层 overlay（门用原始顶点 alpha，不乘 opacity 标量）。
const FIELDOBJECT_KEYWORDS: [&str; 3] = [
    "_RECEIVE_SHADOWS_OFF",
    "_USE_ALPHA_CLIP",
    "_USE_OVERLAY_TEXTURE",
];
const FIELDOBJECT_REQUIRED: [&str; 1] = ["_RECEIVE_SHADOWS_OFF"];

/// Ground：overlay 两层、fresnel、被选 alpha clip、高度淡出（倒数长度
/// 形，雾后）、收影（全族无 _RSO ⇒ 恒收）。
const GROUND_KEYWORDS: [&str; 6] = [
    "_ENABLE_MODULE_FRESNEL",
    "_USE_OVERLAY_TEXTURE",
    "_USE_OVERLAY_TEXTURE_2ND",
    "_USE_ALPHA_CLIP",
    "_USE_HEIGHT_FADE",
    "_RECEIVE_SHADOWS_OFF",
];

/// Tree：抖动恒关、收影恒关（必在 _DISABLE_DITHER + _RECEIVE_SHADOWS_OFF，
/// 全活体行同形）；clip 常量阈、树动画、EMF fresnel（门内第一）、高度
/// 渐变（三色两段）各自可选。
const TREE_KEYWORDS: [&str; 6] = [
    "_DISABLE_DITHER",
    "_RECEIVE_SHADOWS_OFF",
    "_USE_ALPHA_CLIP",
    "_USE_TREE_ANIMATION",
    "_ENABLE_MODULE_FRESNEL",
    "_USE_HEIGHT_FADE",
];
const TREE_REQUIRED: [&str; 2] = ["_DISABLE_DITHER", "_RECEIVE_SHADOWS_OFF"];

/// Water：overlay 两层、fresnel、被选 alpha clip；收影恒收。
const WATER_KEYWORDS: [&str; 5] = [
    "_ENABLE_MODULE_FRESNEL",
    "_USE_OVERLAY_TEXTURE",
    "_USE_OVERLAY_TEXTURE_2ND",
    "_USE_ALPHA_CLIP",
    "_RECEIVE_SHADOWS_OFF",
];

/// Ground-Birthday：抖动在 !_DISABLE_DITHER 时开（量化 0.125）；收影按
/// _RECEIVE_SHADOWS_OFF 运行时定（record24 带影关、record28/34 收）；被选
/// alpha clip、第一层 overlay（门多乘 _Overlay1st_Opacity）。
/// `_USE_TREE_ANIMATION` 惰性收录（该 shader 的任何已编译变体都不含摆动
/// 代码，全部 record 对摆动 uniform 族的搜索零命中；数据里只有
/// gemwreath01 带它）。
const GROUND_BIRTHDAY_KEYWORDS: [&str; 5] = [
    "_DISABLE_DITHER",
    "_RECEIVE_SHADOWS_OFF",
    "_USE_ALPHA_CLIP",
    "_USE_OVERLAY_TEXTURE",
    "_USE_TREE_ANIMATION",
];

/// Object：抖动/收影恒关（必在 _DISABLE_DITHER + _RECEIVE_SHADOWS_OFF）；
/// uniform 阈 clip。值域门另查 usage/mapping/localMapping/_Cull/预览灯。
const OBJECT_KEYWORDS: [&str; 3] = [
    "_DISABLE_DITHER",
    "_RECEIVE_SHADOWS_OFF",
    "_USE_ALPHA_CLIP",
];
const OBJECT_REQUIRED: [&str; 2] = ["_DISABLE_DITHER", "_RECEIVE_SHADOWS_OFF"];

/// DropItem：常量阈 clip；收影恒关。值域门另查 _UVSelection。
const DROPITEM_KEYWORDS: [&str; 2] = ["_RECEIVE_SHADOWS_OFF", "_USE_ALPHA_CLIP"];

/// UI-Uber：无 keyword 轴；值域门另查 _BlendMode（2 = premultiplied 形未
/// 移植）。
const UI_UBER_KEYWORDS: [&str; 0] = [];

fn keywords_within(material: &MaterialSlot, allowed: &[&str]) -> bool {
    material
        .keywords
        .iter()
        .all(|keyword| allowed.contains(&keyword.as_str()))
}

fn has_keyword(material: &MaterialSlot, keyword: &str) -> bool {
    material
        .keywords
        .iter()
        .any(|candidate| candidate == keyword)
}

/// 必在 keyword 缺席 ⇒ 变体形状未见过，具名拒绝。
fn require_keywords(
    material: &MaterialSlot,
    family: &str,
    required: &[&str],
) -> Result<(), String> {
    for keyword in required {
        if !has_keyword(material, keyword) {
            return Err(format!(
                "{family} 材质 {} 缺必在 keyword {keyword}（序列化集 {:?}）",
                material.name, material.keywords
            ));
        }
    }
    Ok(())
}

/// overlay 坐标开关的值域门：源是 int 开关，只有 {0, 1} 两档。
fn overlay_coord_domain(
    material: &MaterialSlot,
    family: &str,
    key: &str,
    value: f32,
) -> Result<(), String> {
    if value != 0.0 && value != 1.0 {
        return Err(format!(
            "{family} 材质 {} 的 {key} = {value} 不在源的开关域 {{0, 1}}",
            material.name
        ));
    }
    Ok(())
}

/// 读一个浮点键的值域门：值不在已实现取值上 ⇒ 未移植分支，具名拒绝。
fn float_domain(
    material: &MaterialSlot,
    family: &str,
    key: &str,
    value: f32,
    domain: &[f32],
) -> Result<(), String> {
    if !domain.contains(&value) {
        return Err(format!(
            "{family} 材质 {} 的 {key} = {value} 不在已实现值域 {domain:?}",
            material.name
        ));
    }
    Ok(())
}

/// FieldObject 解析：law 的 `resolve` 校验 15 个标量与向量条件（含
/// overlay 三键——该族全行序列化它们）；本层补族门（keyword 全集 +
/// `_RECEIVE_SHADOWS_OFF` 必在）与 `_MainTex` 必需（Base 片元无条件采样
/// 它）。`load_texture` 把 sidecar 的 URI 翻成贴图句柄（站点场景包与
/// 采集物包的目录不同，由调用方闭包定）。
pub(crate) fn resolve_fieldobject(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &FIELDOBJECT_KEYWORDS) {
        return Err(format!(
            "FieldObject 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    require_keywords(slot, "FieldObject", &FIELDOBJECT_REQUIRED)?;
    let resolved = fieldobject::resolve(slot)?;
    let main = texture_slot(slot, "_MainTex")
        .ok_or_else(|| format!("FieldObject 材质 {} 缺非空 _MainTex 槽", slot.name))?;
    let uri = sidecar.texture_uris.get(main).ok_or_else(|| {
        format!("FieldObject 材质 {} 的 _MainTex 下标越界", slot.name)
    })?;
    let mut params = SiteParams::zeroed();
    params.use_vertex_color_blend = resolved.use_vertex_color_blend;
    params.alpha_clip = resolved.alpha_clip;
    params.base_opacity = resolved.base_opacity;
    params.dither_alpha = resolved.dither_alpha;
    params.override_shading_parameter = resolved.override_shading_parameter;
    params.local_shading_intensity = resolved.local_shading_intensity;
    params.local_edge_threshold = resolved.local_edge_threshold;
    params.local_edge_smoothness = resolved.local_edge_smoothness;
    params.fresnel_power = resolved.fresnel_power;
    params.use_fresnel = resolved.use_fresnel;
    // 运行时开关关着时源不消费它；law 只在有开关时要求它存在，缺席按
    // 零填充（死支路加零）。
    params.fresnel_color = resolved.fresnel_color.unwrap_or([0.0; 4]);
    params.use_vertex_alpha_opacity = resolved.use_vertex_alpha_opacity;
    params.receive_shadow = 0.0;
    let overlay = resolved.overlay_keyword;
    let mut overlay_tex = None;
    if overlay {
        // law 已校验：keyword 开着则 _OverlayColorMap 槽与 ST 必在。
        params.texture_coord_overlay1st = resolved.texture_coord_overlay1st as f32;
        overlay_coord_domain(
            slot,
            "FieldObject",
            "_TextureCoord_Overlay1st",
            params.texture_coord_overlay1st,
        )?;
        params.uv_scroll_overlay1st = resolved.uv_scroll_overlay1st;
        params.use_overlay_texture_vertex_alpha = resolved.use_overlay_texture_vertex_alpha;
        params.overlay_st = resolved.overlay_st.unwrap_or([0.0; 4]);
        let index = resolved
            .overlay_tex
            .ok_or_else(|| format!("FieldObject 材质 {} 开着 overlay 却缺 _OverlayColorMap 槽", slot.name))?;
        let uri = sidecar.texture_uris.get(index).ok_or_else(|| {
            format!("FieldObject 材质 {} 的 _OverlayColorMap 下标越界", slot.name)
        })?;
        overlay_tex = Some(load_texture(uri));
    }
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::FieldObject,
            overlay_1st: overlay,
            overlay_2nd: false,
            module_fresnel: false,
            tree_animation: false,
            uniform_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            const_alpha_clip: false,
            selected_alpha_clip: false,
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: false,
        },
        params,
        main_tex: load_texture(uri),
        overlay_tex,
        overlay2nd_tex: None,
        leaf_mask_tex: None,
    })
}

/// Ground/Water/Birthday 的 overlay 坐标与贴图读取（三族同形，量名同）。
/// 返回 (coord, st, scroll, uotva, texture)。
#[allow(clippy::type_complexity)]
fn read_overlay_1st(
    family: &str,
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    get: &dyn Fn(&str) -> Result<f32, String>,
    load_texture: &dyn Fn(&str) -> Handle<Image>,
) -> Result<(f32, [f32; 4], [f32; 2], f32, Option<Handle<Image>>), String> {
    let coord = get("_TextureCoord_Overlay1st")?;
    overlay_coord_domain(slot, family, "_TextureCoord_Overlay1st", coord)?;
    let scroll = [get("_UVScrollX_Overlay1st")?, get("_UVScrollY_Overlay1st")?];
    let uotva = get("_UseOverlayTextureVertexAlpha")?;
    let st = *slot
        .texture_scale_offsets
        .get("_OverlayColorMap")
        .ok_or_else(|| {
            format!(
                "{family} 材质 {} 开着 overlay 却缺 _OverlayColorMap 的 ST",
                slot.name
            )
        })?;
    let index = texture_slot(slot, "_OverlayColorMap").ok_or_else(|| {
        format!(
            "{family} 材质 {} 开着 overlay 却缺非空 _OverlayColorMap 槽",
            slot.name
        )
    })?;
    let uri = sidecar.texture_uris.get(index).ok_or_else(|| {
        format!("{family} 材质 {} 的 _OverlayColorMap 下标越界", slot.name)
    })?;
    Ok((coord, st, scroll, uotva, Some(load_texture(uri))))
}

/// Ground/Water 的第二层 overlay 读取（与第一层同形，量名带 2nd）。
#[allow(clippy::type_complexity)]
fn read_overlay_2nd(
    family: &str,
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    get: &dyn Fn(&str) -> Result<f32, String>,
    load_texture: &dyn Fn(&str) -> Handle<Image>,
) -> Result<(f32, [f32; 4], [f32; 2], f32, Option<Handle<Image>>), String> {
    let coord = get("_TextureCoord_Overlay2nd")?;
    overlay_coord_domain(slot, family, "_TextureCoord_Overlay2nd", coord)?;
    let scroll = [get("_UVScrollX_Overlay2nd")?, get("_UVScrollY_Overlay2nd")?];
    let uotva = get("_UseOverlayTextureVertexAlpha2nd")?;
    let st = *slot
        .texture_scale_offsets
        .get("_OverlayColorMap2nd")
        .ok_or_else(|| {
            format!(
                "{family} 材质 {} 开着第二层 overlay 却缺 _OverlayColorMap2nd 的 ST",
                slot.name
            )
        })?;
    let index = texture_slot(slot, "_OverlayColorMap2nd").ok_or_else(|| {
        format!(
            "{family} 材质 {} 开着第二层 overlay 却缺非空 _OverlayColorMap2nd 槽",
            slot.name
        )
    })?;
    let uri = sidecar.texture_uris.get(index).ok_or_else(|| {
        format!("{family} 材质 {} 的 _OverlayColorMap2nd 下标越界", slot.name)
    })?;
    Ok((coord, st, scroll, uotva, Some(load_texture(uri))))
}

/// Ground/Water 共用的基础标量读取（两族同键同形）。
fn read_scroll_base(
    family: &str,
    slot: &MaterialSlot,
    get: &dyn Fn(&str) -> Result<f32, String>,
    params: &mut SiteParams,
) -> Result<(), String> {
    params.use_vertex_color_blend = get("_UseVertexColorBlend")?;
    params.base_opacity = get("_BaseOpacity")?;
    params.use_vertex_alpha_opacity = get("_UseVertexAlphaOpacity")?;
    params.override_shading_parameter = get("_OverrideShadingParameter")?;
    params.local_shading_intensity = get("_LocalShadingIntensity")?;
    params.local_edge_threshold = get("_LocalEdgeThreshold")?;
    params.local_edge_smoothness = get("_LocalEdgeSmoothness")?;
    params.uv_scroll = [get("_UVScrollX")?, get("_UVScrollY")?];
    params.receive_shadow = if has_keyword(slot, "_RECEIVE_SHADOWS_OFF") {
        0.0
    } else {
        1.0
    };
    let _ = family;
    Ok(())
}

/// fresnel 集合（keyword 编译期；Ground/Water/Tree 同键）。
fn read_module_fresnel(
    family: &str,
    slot: &MaterialSlot,
    get: &dyn Fn(&str) -> Result<f32, String>,
    params: &mut SiteParams,
) -> Result<(), String> {
    params.fresnel_power = get("_FresnelPower")?;
    params.fresnel_color = *slot.colors.get("_FresnelColor").ok_or_else(|| {
        format!("{family} 材质 {} 开着 fresnel 却缺 _FresnelColor", slot.name)
    })?;
    Ok(())
}

/// 主贴图读取（各族通用；Base 片元无条件采样它）。
fn read_main_tex(
    family: &str,
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: &dyn Fn(&str) -> Handle<Image>,
) -> Result<Handle<Image>, String> {
    let main = texture_slot(slot, "_MainTex")
        .ok_or_else(|| format!("{family} 材质 {} 缺非空 _MainTex 槽", slot.name))?;
    let uri = sidecar
        .texture_uris
        .get(main)
        .ok_or_else(|| format!("{family} 材质 {} 的 _MainTex 下标越界", slot.name))?;
    Ok(load_texture(uri))
}

/// Ground 解析。已实现的轴：两层 overlay（互斥——同开的组合在源变体表
/// 里不存在，WGSL 的 overlay 链只装得下一层）、keyword fresnel、被选
/// alpha clip、高度淡出（雾后倒数长度形）、恒收影。
fn resolve_ground(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &GROUND_KEYWORDS) {
        return Err(format!(
            "Ground 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    let overlay_1st = has_keyword(slot, "_USE_OVERLAY_TEXTURE");
    let overlay_2nd = has_keyword(slot, "_USE_OVERLAY_TEXTURE_2ND");
    if overlay_1st && overlay_2nd {
        return Err(format!(
            "Ground 材质 {} 的两层 overlay 同开：源变体表里不存在的组合（WGSL 链只装一层）",
            slot.name
        ));
    }
    let fresnel = has_keyword(slot, "_ENABLE_MODULE_FRESNEL");
    let height_fade = has_keyword(slot, "_USE_HEIGHT_FADE");
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("Ground 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let mut params = SiteParams::zeroed();
    read_scroll_base("Ground", slot, &get, &mut params)?;
    if overlay_1st {
        let (coord, st, scroll, uotva, texture) =
            read_overlay_1st("Ground", sidecar, slot, &get, &load_texture)?;
        params.texture_coord_overlay1st = coord;
        params.overlay_st = st;
        params.uv_scroll_overlay1st = scroll;
        params.use_overlay_texture_vertex_alpha = uotva;
        return Ok(ground_finish(
            sidecar,
            slot,
            load_texture,
            params,
            SiteMaterialKey {
                family: SiteFamily::Ground,
                overlay_1st: true,
                overlay_2nd: false,
                module_fresnel: fresnel,
                tree_animation: false,
                uniform_alpha_clip: false,
                const_alpha_clip: false,
                selected_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
                ground_height_fade: height_fade,
                tree_height_fade: false,
                birthday_dither: false,
            },
            texture,
            None,
            &get,
        )?);
    }
    if overlay_2nd {
        let (coord, st, scroll, uotva, texture) =
            read_overlay_2nd("Ground", sidecar, slot, &get, &load_texture)?;
        params.texture_coord_overlay2nd = coord;
        params.overlay_st_2nd = st;
        params.uv_scroll_overlay2nd = scroll;
        params.use_overlay_texture_vertex_alpha_2nd = uotva;
        return Ok(ground_finish(
            sidecar,
            slot,
            load_texture,
            params,
            SiteMaterialKey {
                family: SiteFamily::Ground,
                overlay_1st: false,
                overlay_2nd: true,
                module_fresnel: fresnel,
                tree_animation: false,
                uniform_alpha_clip: false,
                const_alpha_clip: false,
                selected_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
                ground_height_fade: height_fade,
                tree_height_fade: false,
                birthday_dither: false,
            },
            None,
            texture,
            &get,
        )?);
    }
    Ok(ground_finish(
        sidecar,
        slot,
        load_texture,
        params,
        SiteMaterialKey {
            family: SiteFamily::Ground,
            overlay_1st: false,
            overlay_2nd: false,
            module_fresnel: fresnel,
            tree_animation: false,
            uniform_alpha_clip: false,
            const_alpha_clip: false,
            selected_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            ground_height_fade: height_fade,
            tree_height_fade: false,
            birthday_dither: false,
        },
        None,
        None,
        &get,
    )?)
}

/// Ground/Water/Birthday 的公共尾段：fresnel、高度淡出（Ground 形）、
/// 主贴图与 SiteMaterial 组装。
fn ground_finish(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
    mut params: SiteParams,
    key: SiteMaterialKey,
    overlay_tex: Option<Handle<Image>>,
    overlay2nd_tex: Option<Handle<Image>>,
    get: &dyn Fn(&str) -> Result<f32, String>,
) -> Result<SiteMaterial, String> {
    if key.module_fresnel {
        read_module_fresnel(
            match key.family {
                SiteFamily::Ground => "Ground",
                SiteFamily::Water => "Water",
                _ => "Ground",
            },
            slot,
            get,
            &mut params,
        )?;
    }
    if key.ground_height_fade {
        // 倒数长度形；块内还有运行时选择（_UseHeightFade 的活读）。
        params.height_fade_rcp_length = get("_HeightFadeRcpLength")?;
        params.height_fade_start_time_rcp_length = get("_HeightFadeStartTimeRcpLength")?;
        params.height_fade_exponent = get("_HeightFadeExponent")?;
        params.use_height_fade = get("_UseHeightFade")?;
    }
    let main_tex = read_main_tex(
        match key.family {
            SiteFamily::Ground => "Ground",
            SiteFamily::Water => "Water",
            _ => "Ground-Birthday",
        },
        sidecar,
        slot,
        &load_texture,
    )?;
    Ok(SiteMaterial {
        key,
        params,
        main_tex,
        overlay_tex,
        overlay2nd_tex,
        leaf_mask_tex: None,
    })
}

/// Tree 解析：抖动与收影恒关（必在 _DISABLE_DITHER + _RECEIVE_SHADOWS_OFF）；
/// clip 是编译期常量 0.5（`_AlphaClip` 标量不被消费，不读）；树动画、
/// EMF fresnel（门内第一）、高度渐变各自按 keyword。
pub(crate) fn resolve_tree(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    let main_tex = read_main_tex("Tree", sidecar, slot, &load_texture)?;
    let leaf_mask_tex = if has_keyword(slot, "_USE_TREE_ANIMATION") {
        texture_slot(slot, "_LeafMaskTex").map(|index| {
            sidecar.texture_uris.get(index).map(|uri| load_texture(uri))
                .ok_or_else(|| format!("Tree 材质 {} 的 _LeafMaskTex 下标越界", slot.name))
        }).transpose()?
    } else { None };
    resolve_tree_textures(slot, main_tex, leaf_mask_tex)
}

/// The same source Tree shader also occurs on furniture. Its material values
/// are shared, while texture identities and sampling belong to the importer.
pub(crate) fn resolve_tree_textures(
    slot: &MaterialSlot,
    main_tex: Handle<Image>,
    leaf_mask_tex: Option<Handle<Image>>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &TREE_KEYWORDS) {
        return Err(format!(
            "Tree 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    require_keywords(slot, "Tree", &TREE_REQUIRED)?;
    let animation = has_keyword(slot, "_USE_TREE_ANIMATION");
    let module_fresnel = has_keyword(slot, "_ENABLE_MODULE_FRESNEL");
    let height_fade = has_keyword(slot, "_USE_HEIGHT_FADE");
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("Tree 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let mut params = SiteParams::zeroed();
    params.use_vertex_color_blend = get("_UseVertexColorBlend")?;
    params.override_shading_parameter = get("_OverrideShadingParameter")?;
    params.local_shading_intensity = get("_LocalShadingIntensity")?;
    params.local_edge_threshold = get("_LocalEdgeThreshold")?;
    params.local_edge_smoothness = get("_LocalEdgeSmoothness")?;
    params.use_phenomena_lighting = get("_UsePhenomenaLighting")?;
    params.receive_shadow = 0.0;
    if module_fresnel {
        read_module_fresnel("Tree", slot, &get, &mut params)?;
    }
    if height_fade {
        // 渐变形：倒数长度参数算 h，Pos01/Pos12/三色做两段混合。
        params.height_fade_rcp_length = get("_HeightFadeRcpLength")?;
        params.height_fade_start_time_rcp_length = get("_HeightFadeStartTimeRcpLength")?;
        params.height_fade_exponent = get("_HeightFadeExponent")?;
        params.use_height_fade = get("_UseHeightFade")?;
        params.height_gradient_pos01 = get("_HeightGradientPos01")?;
        params.height_gradient_pos12 = get("_HeightGradientPos12")?;
        params.height_gradient_color0 = *slot
            .colors
            .get("_HeightGradientColor0")
            .ok_or_else(|| format!("Tree 材质 {} 开着高度渐变却缺 _HeightGradientColor0", slot.name))?;
        params.height_gradient_color1 = *slot
            .colors
            .get("_HeightGradientColor1")
            .ok_or_else(|| format!("Tree 材质 {} 开着高度渐变却缺 _HeightGradientColor1", slot.name))?;
        params.height_gradient_color2 = *slot
            .colors
            .get("_HeightGradientColor2")
            .ok_or_else(|| format!("Tree 材质 {} 开着高度渐变却缺 _HeightGradientColor2", slot.name))?;
    }
    if animation {
        params.turbulence = get("_turbulenceValue")?;
        params.strength = get("_strengthValue")?;
        params.leaf_rotation_speed = get("_LeafRotationSpeed")?;
        params.leaf_rotation_range = get("_LeafRotationRange")?;
    }
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::Tree,
            overlay_1st: false,
            overlay_2nd: false,
            module_fresnel,
            tree_animation: animation,
            uniform_alpha_clip: false,
            const_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            selected_alpha_clip: false,
            ground_height_fade: false,
            tree_height_fade: height_fade,
            birthday_dither: false,
        },
        params,
        main_tex,
        overlay_tex: None,
        overlay2nd_tex: None,
        leaf_mask_tex,
    })
}

/// Water 解析。已实现的轴：两层 overlay（互斥）、keyword fresnel、被选
/// alpha clip、恒收影。
fn resolve_water(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &WATER_KEYWORDS) {
        return Err(format!(
            "Water 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    let overlay_1st = has_keyword(slot, "_USE_OVERLAY_TEXTURE");
    let overlay_2nd = has_keyword(slot, "_USE_OVERLAY_TEXTURE_2ND");
    if overlay_1st && overlay_2nd {
        return Err(format!(
            "Water 材质 {} 的两层 overlay 同开：源变体表里不存在的组合（WGSL 链只装一层）",
            slot.name
        ));
    }
    let fresnel = has_keyword(slot, "_ENABLE_MODULE_FRESNEL");
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("Water 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let mut params = SiteParams::zeroed();
    read_scroll_base("Water", slot, &get, &mut params)?;
    let mut overlay_tex = None;
    let mut overlay2nd_tex = None;
    if overlay_1st {
        let (coord, st, scroll, uotva, texture) =
            read_overlay_1st("Water", sidecar, slot, &get, &load_texture)?;
        params.texture_coord_overlay1st = coord;
        params.overlay_st = st;
        params.uv_scroll_overlay1st = scroll;
        params.use_overlay_texture_vertex_alpha = uotva;
        overlay_tex = texture;
    }
    if overlay_2nd {
        let (coord, st, scroll, uotva, texture) =
            read_overlay_2nd("Water", sidecar, slot, &get, &load_texture)?;
        params.texture_coord_overlay2nd = coord;
        params.overlay_st_2nd = st;
        params.uv_scroll_overlay2nd = scroll;
        params.use_overlay_texture_vertex_alpha_2nd = uotva;
        overlay2nd_tex = texture;
    }
    ground_finish(
        sidecar,
        slot,
        load_texture,
        params,
        SiteMaterialKey {
            family: SiteFamily::Water,
            overlay_1st,
            overlay_2nd,
            module_fresnel: fresnel,
            tree_animation: false,
            uniform_alpha_clip: false,
            const_alpha_clip: false,
            selected_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: false,
        },
        overlay_tex,
        overlay2nd_tex,
        &get,
    )
}

/// Ground-Birthday 解析。已实现的轴：抖动（!_DISABLE_DITHER，量化
/// 0.125）、收影（按 _RECEIVE_SHADOWS_OFF 运行时定）、被选 alpha clip、
/// 第一层 overlay（门多乘 _Overlay1st_Opacity）。无 fresnel、无高度淡出
/// （带这两个 keyword 的行被族门拒）。`_USE_TREE_ANIMATION` 惰性（见
/// GROUND_BIRTHDAY_KEYWORDS 注释），tree_animation 恒 false。
fn resolve_ground_birthday(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &GROUND_BIRTHDAY_KEYWORDS) {
        return Err(format!(
            "Ground-Birthday 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    let dither = !has_keyword(slot, "_DISABLE_DITHER");
    let overlay = has_keyword(slot, "_USE_OVERLAY_TEXTURE");
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("Ground-Birthday 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let mut params = SiteParams::zeroed();
    params.use_vertex_color_blend = get("_UseVertexColorBlend")?;
    params.base_opacity = get("_BaseOpacity")?;
    params.use_vertex_alpha_opacity = get("_UseVertexAlphaOpacity")?;
    params.override_shading_parameter = get("_OverrideShadingParameter")?;
    params.local_shading_intensity = get("_LocalShadingIntensity")?;
    params.local_edge_threshold = get("_LocalEdgeThreshold")?;
    params.local_edge_smoothness = get("_LocalEdgeSmoothness")?;
    params.uv_scroll = [get("_UVScrollX")?, get("_UVScrollY")?];
    params.receive_shadow = if has_keyword(slot, "_RECEIVE_SHADOWS_OFF") {
        0.0
    } else {
        1.0
    };
    if dither {
        params.dither_alpha = get("_DitherAlpha")?;
    }
    let mut overlay_tex = None;
    if overlay {
        let (coord, st, scroll, uotva, texture) = read_overlay_1st(
            "Ground-Birthday",
            sidecar,
            slot,
            &get,
            &load_texture,
        )?;
        params.texture_coord_overlay1st = coord;
        params.overlay_st = st;
        params.uv_scroll_overlay1st = scroll;
        params.use_overlay_texture_vertex_alpha = uotva;
        params.birthday_overlay_opacity = get("_Overlay1st_Opacity")?;
        overlay_tex = texture;
    }
    let main_tex = read_main_tex("Ground-Birthday", sidecar, slot, &load_texture)?;
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::GroundBirthday,
            overlay_1st: overlay,
            overlay_2nd: false,
            module_fresnel: false,
            tree_animation: false,
            uniform_alpha_clip: false,
            const_alpha_clip: false,
            selected_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: dither,
        },
        params,
        main_tex,
        overlay_tex,
        overlay2nd_tex: None,
        leaf_mask_tex: None,
    })
}

/// Object 解析。值域门（未实现的分支具名拒）：usage 只在 {8, 12}
/// （2 = 墙 AO、11 = 道路、14 = 直通、其余未见过）、mapping 只在
/// {0, 1, 2}（3 = uv2）、localMapping 只在 {0, 1}、_Cull 必须是 2
/// （背面剔除——_BackFaceColor 路径因此恒死）、预览灯必须关。
fn resolve_object(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &OBJECT_KEYWORDS) {
        return Err(format!(
            "Object 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    require_keywords(slot, "Object", &OBJECT_REQUIRED)?;
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("Object 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let usage = get("_ObjectShaderUsage")?;
    float_domain(slot, "Object", "_ObjectShaderUsage", usage, &[8.0, 12.0])?;
    let mapping = get("_BaseTextureMappingMode")?;
    float_domain(slot, "Object", "_BaseTextureMappingMode", mapping, &[0.0, 1.0, 2.0])?;
    let local_mapping = get("_MainTextureLocalMapping")?;
    float_domain(slot, "Object", "_MainTextureLocalMapping", local_mapping, &[0.0, 1.0])?;
    let cull = get("_Cull")?;
    float_domain(slot, "Object", "_Cull", cull, &[2.0])?;
    let preview_light = get("_UseObject3DPreviewLight")?;
    float_domain(slot, "Object", "_UseObject3DPreviewLight", preview_light, &[0.0])?;
    let mut params = SiteParams::zeroed();
    params.use_vertex_color_blend = get("_UseVertexColorBlend")?;
    params.alpha_clip = get("_AlphaClip")?;
    params.base_opacity = get("_BaseOpacity")?;
    params.override_shading_parameter = get("_OverrideShadingParameter")?;
    params.local_shading_intensity = get("_LocalShadingIntensity")?;
    params.local_edge_threshold = get("_LocalEdgeThreshold")?;
    params.local_edge_smoothness = get("_LocalEdgeSmoothness")?;
    params.fresnel_power = get("_FresnelPower")?;
    params.use_fresnel = get("_UseFresnel")?;
    params.fresnel_color = *slot
        .colors
        .get("_FresnelColor")
        .ok_or_else(|| format!("Object 材质 {} 缺 _FresnelColor", slot.name))?;
    params.use_vertex_alpha_opacity = get("_UseVertexAlphaOpacity")?;
    params.use_phenomena_lighting = get("_UsePhenomenaLighting")?;
    params.object_uv_scroll = [get("_UVScrollX")?, get("_UVScrollY")?];
    params.additive_color = *slot
        .colors
        .get("_AdditiveColor")
        .ok_or_else(|| format!("Object 材质 {} 缺 _AdditiveColor", slot.name))?;
    params.object_texture_mapping = mapping;
    params.object_main_texture_local_mapping = local_mapping;
    // 高度淡出在源里无 keyword、恒编译；当前数据 _UseHeightFade 全 0，
    // 块保留（运行时选择）。
    params.height_fade_position = get("_HeightFadePosition")?;
    params.height_fade_length = get("_HeightFadeLength")?;
    params.height_fade_exponent = get("_HeightFadeExponent")?;
    params.use_height_fade = get("_UseHeightFade")?;
    params.receive_shadow = 0.0;
    let main_tex = read_main_tex("Object", sidecar, slot, &load_texture)?;
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::Object,
            overlay_1st: false,
            overlay_2nd: false,
            module_fresnel: false,
            tree_animation: false,
            uniform_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            const_alpha_clip: false,
            selected_alpha_clip: false,
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: false,
        },
        params,
        main_tex,
        overlay_tex: None,
        overlay2nd_tex: None,
        leaf_mask_tex: None,
    })
}

/// DropItem 解析。值域门：_UVSelection 只在 {0, 1}；clip 是常量阈 0.5
/// （`_AlphaClip` 标量不被消费）；alpha = tex.a（无 _BaseOpacity）。
fn resolve_dropitem(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &DROPITEM_KEYWORDS) {
        return Err(format!(
            "DropItem 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("DropItem 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let uv_selection = get("_UVSelection")?;
    float_domain(slot, "DropItem", "_UVSelection", uv_selection, &[0.0, 1.0])?;
    let mut params = SiteParams::zeroed();
    params.dropitem_uv_selection = uv_selection;
    params.dropitem_uv_scroll = [get("_MainTex_UVScrollX")?, get("_MainTex_UVScrollY")?];
    params.receive_shadow = 0.0;
    let main_tex = read_main_tex("DropItem", sidecar, slot, &load_texture)?;
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::DropItem,
            overlay_1st: false,
            overlay_2nd: false,
            module_fresnel: false,
            tree_animation: false,
            uniform_alpha_clip: false,
            const_alpha_clip: has_keyword(slot, "_USE_ALPHA_CLIP"),
            selected_alpha_clip: false,
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: false,
        },
        params,
        main_tex,
        overlay_tex: None,
        overlay2nd_tex: None,
        leaf_mask_tex: None,
    })
}

/// UI-Uber 解析。值域门：_BlendMode 只在 {0, 1}（2 = premultiplied 形未
/// 移植）；无 keyword 轴；片元是 tex × 顶点色的直乘（无 mip 偏置、无
/// 光照链）。
fn resolve_ui_uber(
    sidecar: &SiteSidecar,
    slot: &MaterialSlot,
    load_texture: impl Fn(&str) -> Handle<Image>,
) -> Result<SiteMaterial, String> {
    if !keywords_within(slot, &UI_UBER_KEYWORDS) {
        return Err(format!(
            "UI-Uber 材质 {} 的 keyword {:?} 不在已实现轴清单内",
            slot.name, slot.keywords
        ));
    }
    let get = |key: &str| -> Result<f32, String> {
        slot.get(key)
            .ok_or_else(|| format!("UI-Uber 材质 {} 缺浮点属性 {key}", slot.name))
    };
    let blend_mode = get("_BlendMode")?;
    float_domain(slot, "UI-Uber", "_BlendMode", blend_mode, &[0.0, 1.0])?;
    let mut params = SiteParams::zeroed();
    params.ui_uber_main_st = *slot
        .texture_scale_offsets
        .get("_MainTex")
        .ok_or_else(|| format!("UI-Uber 材质 {} 缺 _MainTex 的 ST", slot.name))?;
    let main_tex = read_main_tex("UI-Uber", sidecar, slot, &load_texture)?;
    Ok(SiteMaterial {
        key: SiteMaterialKey {
            family: SiteFamily::UiUber,
            overlay_1st: false,
            overlay_2nd: false,
            module_fresnel: false,
            tree_animation: false,
            uniform_alpha_clip: false,
            const_alpha_clip: false,
            selected_alpha_clip: false,
            ground_height_fade: false,
            tree_height_fade: false,
            birthday_dither: false,
        },
        params,
        main_tex,
        overlay_tex: None,
        overlay2nd_tex: None,
        leaf_mask_tex: None,
    })
}

// ---- 换装 ----

/// 已请求装载的站点 sidecar。
#[derive(Resource)]
pub struct SiteSidecarAsset {
    pub(crate) handle: Handle<MolyJson>,
    /// 本 sidecar 所属的场景目录名：贴图路径与日志名从它走，不再有
    /// 编译期站点常量。
    pub(crate) scene: String,
}

/// scene 实例已展开的常驻闩。站点模块自己的就绪标记会被取景系统撤掉，
/// 换装等这个不撤的。


/// 换装完成标记。
#[derive(Resource)]
pub struct SiteMaterialsSwapped;

/// 一条换装计划：解析好的站点材质与逐实体要求。
struct Planned {
    name: String,
    material: SiteMaterial,
}

/// glb 材质句柄的归类：换装的指向 plan 下标，其余保留原材质、按桶计数。
#[derive(Clone, Copy)]
enum GltfClass {
    Swap(usize),
    /// 其他（URP/Lit、被拒绝的、glb 独有的、非 glb 材质）。
    Retain,
}

/// 一次换装的全部状态。
struct SwapPlan {
    planned: Vec<Planned>,
    classes: HashMap<Handle<StandardMaterial>, ClassifiedMaterial>,
    tally: SwapTally,
}

/// Colour replacement and source pass availability are independent: even a
/// material whose colour shader is not yet ported keeps its source shadow rule.
struct ClassifiedMaterial {
    colour: GltfClass,
    passes: Option<moly_assets::material_passes::SourceMaterialPasses>,
}

/// 计数们：换装完成时一次性 `info!`，是「真的换上了吗」的现算证据。
#[derive(Default)]
struct SwapTally {
    fieldobject_materials: usize,
    fieldobject_entities: usize,
    ground_materials: usize,
    ground_entities: usize,
    tree_materials: usize,
    tree_entities: usize,
    water_materials: usize,
    water_entities: usize,
    ground_birthday_materials: usize,
    ground_birthday_entities: usize,
    object_materials: usize,
    object_entities: usize,
    dropitem_materials: usize,
    dropitem_entities: usize,
    ui_uber_materials: usize,
    ui_uber_entities: usize,
    other_entities: usize,
    /// 具名拒绝：族门/必需槽/keyword 域/值域不过。
    refused: Vec<String>,
    /// glb 有而 sidecar 没有的材质位（按下标）。
    glb_only: Vec<String>,
    /// sidecar 比 glb 多的尾槽（死条目，不产生 draw）。
    sidecar_only: usize,
    /// 槽值是 URI 而不在 sidecar 顶层 textures[] 的条数（装载层保留）。
    unmatched_texture_slots: usize,
}

/// 请求装载站点 sidecar（首次装载与每次换站都走这里；场景目录名由装载
/// 计划给出）。拆站时由 [`teardown`] 撤下在途请求与换装闩。
pub(crate) fn request(commands: &mut Commands, server: &AssetServer, scene: &str) {
    let handle = server.load::<MolyJson>(moly_assets::site_scene_json(scene));
    commands.insert_resource(SiteSidecarAsset {
        handle,
        scene: scene.to_owned(),
    });
}

/// 拆站面：撤下 sidecar 请求与换装闩，让新站的换装重新起跳。
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<SiteSidecarAsset>();
    commands.remove_resource::<SiteMaterialsSwapped>();
}


/// Update：scene 展开且 sidecar 到位后，按下标 join、按族解析、等贴图
/// 到齐、一次性换装。此前每帧空转。
fn switch_materials(
    mut commands: Commands,
    sidecar: Option<Res<SiteSidecarAsset>>,
    scene_ready: Option<Res<SiteScenesReady>>,
    swapped: Option<Res<SiteMaterialsSwapped>>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    json: Res<Assets<MolyJson>>,
    mut materials: ResMut<Assets<SiteMaterial>>,
    site: Option<Res<SiteAssets>>,
    parts: Query<(Entity, &Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    mut plan: Local<Option<SwapPlan>>,
) {
    if swapped.is_some() {
        return;
    }
    let (Some(sidecar), Some(_), Some(site)) = (sidecar, scene_ready, site) else {
        return;
    };
    let scene = sidecar.scene.as_str();
    match server.load_state(&sidecar.handle) {
        LoadState::Failed(err) => panic!("站点 sidecar 装载失败（{scene}）：{err:?}"),
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&sidecar.handle) else {
        return;
    };
    let MolyJson::SiteSidecar(sidecar) = asset else {
        panic!("站点 sidecar 路径装载到了别的资产类型（{scene}）");
    };
    let Some(mut state) = plan
        .take()
        .or_else(|| build_swap_plan(&gltfs, &site, sidecar, scene, &server))
    else {
        return;
    };

    // 等贴图到齐：换装早于贴图到位会让实体闪回默认材质。装载失败具名 panic。
    let mut all_loaded = true;
    for item in &state.planned {
        let textures = std::iter::once(&item.material.main_tex)
            .chain(item.material.overlay_tex.iter())
            .chain(item.material.overlay2nd_tex.iter())
            .chain(item.material.leaf_mask_tex.iter());
        for texture in textures {
            match server.load_state(texture) {
                LoadState::Failed(err) => {
                    panic!("材质 {} 的贴图装载失败：{err:?}", item.name)
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

    // 按 glb 材质句柄分组实体，执行换装。
    let mut entities_by_material: HashMap<_, Vec<Entity>> = HashMap::new();
    for (entity, _, material) in &parts {
        entities_by_material
            .entry(material.0.clone())
            .or_default()
            .push(entity);
    }
    for (handle, entities) in &entities_by_material {
        let classification = state.classes.get(handle);
        if let Some(passes) = classification.and_then(|entry| entry.passes.as_ref()) {
            for entity in entities {
                commands.entity(*entity).insert(passes.clone());
            }
        }
        match classification.map(|entry| entry.colour) {
            Some(GltfClass::Swap(index)) => {
                let item = &state.planned[index];
                let site_handle = materials.add(item.material.clone());
                let mut swapped_entities = 0;
                for entity in entities {
                    commands
                        .entity(*entity)
                        .remove::<MeshMaterial3d<StandardMaterial>>()
                        .insert(MeshMaterial3d(site_handle.clone()));
                    swapped_entities += 1;
                }
                if swapped_entities > 0 {
                    let tally = &mut state.tally;
                    let (materials_count, entities_count) = match item.material.key.family {
                        SiteFamily::FieldObject => {
                            let r = (&mut tally.fieldobject_materials, &mut tally.fieldobject_entities);
                            r
                        }
                        SiteFamily::Ground => (&mut tally.ground_materials, &mut tally.ground_entities),
                        SiteFamily::Tree => (&mut tally.tree_materials, &mut tally.tree_entities),
                        SiteFamily::Water => (&mut tally.water_materials, &mut tally.water_entities),
                        SiteFamily::GroundBirthday => (
                            &mut tally.ground_birthday_materials,
                            &mut tally.ground_birthday_entities,
                        ),
                        SiteFamily::Object => (&mut tally.object_materials, &mut tally.object_entities),
                        SiteFamily::DropItem => {
                            (&mut tally.dropitem_materials, &mut tally.dropitem_entities)
                        }
                        SiteFamily::UiUber => (&mut tally.ui_uber_materials, &mut tally.ui_uber_entities),
                    };
                    *materials_count += 1;
                    *entities_count += swapped_entities;
                }
            }
            _ => state.tally.other_entities += entities.len(),
        }
    }

    info!(
        "{scene} 材质换装：FieldObject {} 材质 {} 实体，Ground {} 材质 {} 实体，\
         Tree {} 材质 {} 实体，Water {} 材质 {} 实体，Ground-Birthday {} 材质 {} 实体，\
         Object {} 材质 {} 实体，DropItem {} 材质 {} 实体，UI-Uber {} 材质 {} 实体；\
         其他 {} 实体",
        state.tally.fieldobject_materials,
        state.tally.fieldobject_entities,
        state.tally.ground_materials,
        state.tally.ground_entities,
        state.tally.tree_materials,
        state.tally.tree_entities,
        state.tally.water_materials,
        state.tally.water_entities,
        state.tally.ground_birthday_materials,
        state.tally.ground_birthday_entities,
        state.tally.object_materials,
        state.tally.object_entities,
        state.tally.dropitem_materials,
        state.tally.dropitem_entities,
        state.tally.ui_uber_materials,
        state.tally.ui_uber_entities,
        state.tally.other_entities,
    );
    if !state.tally.refused.is_empty() {
        warn!(
            "具名拒绝 {} 条：{:?}",
            state.tally.refused.len(),
            state.tally.refused
        );
    }

    if !state.tally.glb_only.is_empty() {
        warn!(
            "glb 有而 sidecar 无 {} 条：{:?}",
            state.tally.glb_only.len(),
            state.tally.glb_only
        );
    }
    info!(
        "sidecar 对账：死条目 {} 条，槽 URI 不在 textures[] 的 {} 条",
        state.tally.sidecar_only, state.tally.unmatched_texture_slots,
    );
    commands.insert_resource(SiteMaterialsSwapped);
}

/// 建 plan：按 glb 材质下标 join sidecar（glb `materials[i]` 与 sidecar
/// `materials[i]` 是同一份材质表的两份导出——提取侧顺序一致，按名 join
/// 会在重名/改名时错位），shader 名分派族，族门 + 解析，全部 glb 材质
/// 归类。glb 资产未到时回 `None`（调用方下帧再试）。
fn build_swap_plan(
    gltfs: &Assets<Gltf>,
    site: &SiteAssets,
    sidecar: &SiteSidecar,
    scene: &str,
    server: &AssetServer,
) -> Option<SwapPlan> {
    let gltf = gltfs.get(&site.gltf)?;
    let mut planned = Vec::new();
    let mut classes = HashMap::new();
    let mut tally = SwapTally::default();
    let load = |uri: &str| load_site_texture(server, sidecar, scene, uri);
    // glb 材质名反查（下标 join 的日志名用；无名材质按下标记）。
    let mut name_by_handle: HashMap<&Handle<StandardMaterial>, &str> = HashMap::new();
    for (name, handle) in &gltf.named_materials {
        name_by_handle.entry(handle).or_insert(name);
    }
    for (index, glb_material) in gltf.materials.iter().enumerate() {
        let class = match sidecar.materials.get(index) {
            None => {
                tally.glb_only.push(format!(
                    "{}（glb 下标 {index}）",
                    name_by_handle.get(glb_material).copied().unwrap_or("<无名>")
                ));
                GltfClass::Retain
            }
            Some(slot) => {
                // 八族共用的入库臂：解析成功建计划，失败具名拒、保留原材质。
                let plan_material = |result: Result<SiteMaterial, String>,
                                     planned: &mut Vec<Planned>,
                                     tally: &mut SwapTally| {
                    match result {
                        Ok(material) => {
                            planned.push(Planned {
                                name: slot.name.clone(),
                                material,
                            });
                            GltfClass::Swap(planned.len() - 1)
                        }
                        Err(reason) => {
                            tally.refused.push(reason);
                            GltfClass::Retain
                        }
                    }
                };
                match slot.shader.as_str() {
                    fieldobject::SHADER_NAME => plan_material(
                        resolve_fieldobject(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    moly_law::shading::ground::SHADER_NAME => plan_material(
                        resolve_ground(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    moly_law::shading::tree::SHADER_NAME => plan_material(
                        resolve_tree(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    moly_law::shading::water::SHADER_NAME => plan_material(
                        resolve_water(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    GROUND_BIRTHDAY_SHADER_NAME => plan_material(
                        resolve_ground_birthday(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    OBJECT_SHADER_NAME => plan_material(
                        resolve_object(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    DROPITEM_SHADER_NAME => plan_material(
                        resolve_dropitem(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    UI_UBER_SHADER_NAME => plan_material(
                        resolve_ui_uber(sidecar, slot, &load),
                        &mut planned,
                        &mut tally,
                    ),
                    // 其他（URP/Lit、粒子族等）：保留。
                    _ => GltfClass::Retain,
                }
            }
        };
        classes.insert(glb_material.clone(), ClassifiedMaterial {
            colour: class,
            passes: sidecar.materials.get(index).and_then(|source| source.passes.clone()),
        });
    }
    // sidecar 死尾槽：glb 材质表短于 sidecar 材质表的部分。
    tally.sidecar_only = sidecar.materials.len().saturating_sub(gltf.materials.len());
    tally.unmatched_texture_slots = sidecar.unmatched.len();
    Some(SwapPlan {
        planned,
        classes,
        tally,
    })
}

/// 站点材质插件：全局量桥 + 材质管线 + sidecar 装载与换装。
/// `.json` 装载器在这里注册，全局量桥的雾档案与站点 sidecar 共用它。
pub struct SiteMaterialPlugin;

impl Plugin for SiteMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            crate::env::SiteEnvPlugin,
            MaterialPlugin::<SiteMaterial>::default(),
        ))
        .init_asset::<MolyJson>()
        .init_asset_loader::<MolyJsonLoader>()
        .add_systems(Update, switch_materials);
        bevy::asset::embedded_asset!(app, "shaders/site_material.wgsl");
    }
}
