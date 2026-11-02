//! Character toon material: the Bevy Material of the `Mysekai/Character`
//! shader.
//!
//! All formulas live in moly-law's shading::character (pure functions plus
//! `resolve`); this module only assembles: it feeds the Unity properties of
//! the rig profile (rig json) into the law's `resolve` and spreads the
//! resulting values over the uniform slots. The WGSL lives in
//! `shaders/character_material.wesl` and
//! implements the law's fragment chain. Two variants: the eyebrow variant
//! is keyed on its texture slot being present (already decided), the fog
//! variant on the global
//! bridge being present (wait for the fog profile to arrive, e.g. on an
//! outfit change, so the variant is the same on every side).
//!
//! The head reference point is a **material slot** value, not a global: it
//! is a MaterialPropertyBlock uniform, written separately per renderer, so it
//! goes into each material's own parameter block and is written back every
//! frame from the world position of the head bone entity. Left at the origin
//! by default, the centre of the sphere distance field slides to the world
//! origin and the whole sphere boundary shifts (one cause of the boundary
//! line).
//!
//! The parameter block lives in a uniform buffer that the material owns for
//! its whole life. The per-frame head reference and dither values are stored
//! into the asset without an asset event, and the render world copies every
//! material's current parameters into that buffer during extraction whenever
//! they differ from the last upload. The shader reads the same bytes a
//! re-prepared material would carry, without a new bind group every frame.

use std::collections::{BTreeMap, HashMap};

use bevy::asset::{AssetPath, LoadState};
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::gltf::Gltf;
use bevy::image::Image;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, StandardMaterial};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_assets::sidecar::MolyJson;
#[cfg(target_arch = "wasm32")]
use moly_assets::sidecar::MolyJsonLoader;
use moly_law::material::MaterialSlot;
use moly_law::shading::character as law;
use moly_law::shading::character::CharacterGlobals;

use crate::light;
use crate::markers::{CharacterDriver, CharacterModel, CharacterPack, CharacterUnitId};
use crate::render::gpu::{store_bits, SharedSamplers};
use crate::source_stencil::StencilClipMask;

// ---- 现象常量（晴天配置的源值照抄） ----

/// 晴天现象的角色皮肤阴面色（rgba，源浮点空间）。
const SUNNY_SKIN_SHADE: [f32; 4] = [
    0.9528301954269409,
    0.8944019079208374,
    0.8944019079208374,
    1.0,
];
/// 晴天现象的角色身体阴面色。
const SUNNY_BODY_SHADE: [f32; 4] = [
    0.9339622855186462,
    0.8502581119537354,
    0.8502581119537354,
    1.0,
];
/// 晴天现象的角色方向光色：源值纯白、混合因子 1。
const SUNNY_LIGHT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// 球面支 edge（`_FaceSphereShadowEdge`）。源每帧从图形配置的角色档
/// （`MysekaiGraphicsConfig._characterData`，Resources 里
/// `graphics/data/charactersettings` 那一个 `MysekaiGraphicsConfigCharacterData`）
/// 读 `FaceSphereShadowEdge` 推成全局量；这份档随客户端出货、不分天气
/// 与地点，全游戏只有这一份。序列化字：`0x00000000`。
const FACE_SPHERE_SHADOW_EDGE: f32 = f32::from_bits(0x0000_0000);
/// 球面支 smoothness（`_FaceSphereShadowSmoothness`），同一份档的
/// `FaceSphereShadowSmoothness`。序列化字：`0x3f800000`。另有一个嵌套类
/// 的同名两字段（取值 0 / 0.112）不喂这个 shader：推全局量的那段只解引用
/// `_characterData`。
const FACE_SPHERE_SHADOW_SMOOTHNESS: f32 = f32::from_bits(0x3f80_0000);

// ---- 材质参数块 ----

/// 参数 uniform 的槽数与字节数。槽序是本文件与
/// `shaders/character_material.wesl` 里 `CharacterParams` 结构体之间的
/// 契约，两边同改。
pub const PARAMS_SLOTS: usize = 9;
pub const PARAMS_BYTES: usize = PARAMS_SLOTS * 16;

/// 每条 Unity 属性一个槽位（前四个标量合一个 vec4）；头参考点是本块
/// 唯一的逐帧回写槽。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterParams {
    /// `_CharacterShaderUsage`（x）、`_EyebrowClip`（y）、`_EyebrowAlpha`
    /// （z）、`_UseDither`（w）。眉标量只在眉变体在场时被消费。
    pub shader_usage: f32,
    pub eyebrow_clip: f32,
    pub eyebrow_alpha: f32,
    pub use_dither: f32,
    pub brightness: f32,
    /// `_EyebrowStencil`: the stencil reference the base pass writes and the
    /// eyebrow pass tests (written as a float beside the brightness).
    pub eyebrow_stencil: u32,
    pub override_shading_parameter: f32,
    pub local_body_shading_intensity: f32,
    pub local_body_shading_edge_threshold: f32,
    pub local_body_shading_edge_smoothness: f32,
    pub dither_alpha: f32,
    /// `_MainTex` 的 `(scaleX, scaleY, offsetX, offsetY)`；档案不带 ST
    /// 记录时取 Unity 缺省的恒等变换。
    pub main_tex_st: [f32; 4],
    /// 头参考点：头骨世界位（xyz），源只吃 `.xz`。
    pub head_position: [f32; 4],
}

impl CharacterParams {
    /// 按上面的槽序摊平成字节。
    pub fn byte_array(&self) -> [u8; PARAMS_BYTES] {
        let mut bytes = [0u8; PARAMS_BYTES];
        let mut chunks = bytes.as_chunks_mut::<4>().0.iter_mut();
        for slot in [
            [
                self.shader_usage,
                self.eyebrow_clip,
                self.eyebrow_alpha,
                self.use_dither,
            ],
            [self.brightness, self.eyebrow_stencil as f32, 0.0, 0.0],
            [self.override_shading_parameter, 0.0, 0.0, 0.0],
            [self.local_body_shading_intensity, 0.0, 0.0, 0.0],
            [self.local_body_shading_edge_threshold, 0.0, 0.0, 0.0],
            [self.local_body_shading_edge_smoothness, 0.0, 0.0, 0.0],
            [self.dither_alpha, 0.0, 0.0, 0.0],
            self.main_tex_st,
            self.head_position,
        ] {
            for component in slot {
                chunks
                    .next()
                    .expect("PARAMS_SLOTS vec4 slots fill PARAMS_BYTES exactly")
                    .copy_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert!(chunks.next().is_none());
        bytes
    }
}

/// 管线特化键：眉与雾两个变体。眉按材质自己的纹理槽在场分（已裁）；
/// 雾按全局量桥在场分——换装等雾档案到齐才建材质，全侧一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharacterKey {
    pub eyebrow: bool,
    pub fog: bool,
}

/// One eye texture pair: the atlas bound to `_MainTex` and the mask bound to
/// `_EyebrowTex`.
#[derive(Debug, Clone)]
pub struct EyeTextures {
    pub main: Handle<Image>,
    pub eyebrow: Option<Handle<Image>>,
}

/// The eye slot's texture pairs. An eye pattern row's `TextureIndex` 0 is the
/// material's own pair (`default`); N >= 1 is `additional[N - 1]`. An index
/// with no pair falls back to 0.
#[derive(Debug, Clone)]
pub struct EyeTextureSets {
    pub default: EyeTextures,
    pub additional: Vec<EyeTextures>,
    /// The index whose pair the material currently holds.
    current: i32,
    /// The last unusable index reported, so a repeated request reports once.
    last_fallback_reported: i32,
}

impl EyeTextureSets {
    fn new(default: EyeTextures, additional: Vec<EyeTextures>) -> Self {
        Self { default, additional, current: 0, last_fallback_reported: 0 }
    }

    fn usable(&self, index: i32) -> bool {
        index == 0 || (index > 0 && index as usize <= self.additional.len())
    }

    /// Every texture handle the sets hold.
    pub fn handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        std::iter::once(&self.default)
            .chain(self.additional.iter())
            .flat_map(|set| std::iter::once(&set.main).chain(set.eyebrow.iter()))
    }
}

/// 角色 toon 材质资产。按名独立成类，不与站点的 SiteMaterial 混。
#[derive(Debug, Clone, TypePath, Asset)]
pub struct CharacterMaterial {
    pub key: CharacterKey,
    pub params: CharacterParams,
    /// Binding 0: the GPU copy of `params`, created with the material and
    /// rewritten by `upload_params` whenever `params` changes. Every material
    /// asset holds its own buffer; two assets never share one.
    pub params_buffer: Buffer,
    /// Instance-local full-row input for the actor's mouth controller. These
    /// host fields are not GPU bindings; speech texture writes never touch them.
    pub lip_pattern: Option<moly_law::facial::LipPattern>,
    pub lip_pattern_revision: u64,
    /// The view's current eye pattern (open, close, blink), set by every
    /// eye-pattern change; the blink reads its cells from it.
    pub eye_pattern: Option<moly_law::blink::EyePattern>,
    pub main_tex: Handle<Image>,
    /// body 槽的遮罩贴图；其余槽没有它，绑白色 fallback（采样值不进
    /// 有效支路）。
    pub body_mask_tex: Option<Handle<Image>>,
    /// 眉贴图；只有带 `_EyebrowTex` 槽的材质有它。
    pub eyebrow_tex: Option<Handle<Image>>,
    /// The eye slot's texture pairs; `main_tex` and `eyebrow_tex` hold the
    /// pair the current eye pattern selects. Other slots have none.
    pub eye_textures: Option<EyeTextureSets>,
}

impl CharacterMaterial {
    /// The view's eye texture change for a pattern row's `TextureIndex`:
    /// the main texture and eyebrow mask become the selected pair (a pair
    /// without a mask keeps the material's own mask). Returns whether the
    /// textures changed.
    pub fn select_eye_texture(&mut self, requested: i32) -> bool {
        let Some(sets) = self.eye_textures.as_mut() else {
            return false;
        };
        let index = if sets.usable(requested) {
            requested
        } else {
            if sets.last_fallback_reported != requested {
                warn!(
                    "eye texture set {requested} is not available ({} additional sets); using the material's own pair",
                    sets.additional.len()
                );
                sets.last_fallback_reported = requested;
            }
            0
        };
        if index == sets.current {
            return false;
        }
        sets.current = index;
        let pair = match index {
            0 => &sets.default,
            n => &sets.additional[n as usize - 1],
        };
        self.main_tex = pair.main.clone();
        self.eyebrow_tex = pair.eyebrow.clone().or_else(|| sets.default.eyebrow.clone());
        true
    }
}

impl AsBindGroup for CharacterMaterial {
    type Data = CharacterKey;
    type Param = (
        SRes<CharacterEnvGpuBuffer>,
        SRes<RenderAssets<GpuImage>>,
        SRes<FallbackImage>,
        SRes<SharedSamplers>,
        Option<SRes<StencilClipMask>>,
    );

    fn label() -> &'static str {
        "character_material"
    }

    fn build_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        (env_buffer, images, fallback, samplers, clip_mask): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
        output: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        let main = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let body_mask = match &self.body_mask_tex {
            Some(handle) => images
                .get(handle)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        let eyebrow = match &self.eyebrow_tex {
            Some(handle) => images
                .get(handle)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        // The stencil clip mask at the size the main world gave it; its
        // alpha holds the stencil value the eyebrow pass tests.
        let clip_mask = match clip_mask.as_ref() {
            Some(mask) => images
                .get(&mask.image)
                .filter(|image| mask.fits(image.texture_descriptor.size))
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        // 角色贴图不滚动、ST 是恒等变换，uv 不越 [0,1]；采样态不在语料
        // 记录里，取引擎默认的钳边与三线性——与 GpuImage 自带 sampler 的
        // 过滤一致，只有寻址模式不同。
        // The sampler is created once at render startup and shared by every
        // character material; its descriptor never varies per material.
        let clamp = samplers.clamp_linear.clone();
        let bindings = UnpreparedBindingResources(vec![
            // binding 0：材质自己的参数块（含头参考点）。
            (0, UnpreparedBindingResource::Buffer(self.params_buffer.clone())),
            // binding 1：全局量，全部角色材质共用一个 buffer。
            (1, UnpreparedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (3, UnpreparedBindingResource::Sampler(SamplerBindingType::Filtering, clamp.clone())),
            (
                4,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    body_mask.texture_view.clone(),
                ),
            ),
            (5, UnpreparedBindingResource::Sampler(SamplerBindingType::Filtering, clamp.clone())),
            (
                6,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    eyebrow.texture_view.clone(),
                ),
            ),
            (7, UnpreparedBindingResource::Sampler(SamplerBindingType::Filtering, clamp)),
            // binding 8: the stencil clip mask, read texel by texel.
            (
                8,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    clip_mask.texture_view.clone(),
                ),
            ),
        ]);
        output.binding_resources.0.extend(bindings.0);
        Ok(())
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
        vec![
            uniform(0),
            uniform(1),
            texture(2),
            sampler(3),
            texture(4),
            sampler(5),
            texture(6),
            sampler(7),
            BindGroupLayoutEntry {
                binding: 8,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ]
    }
}

impl Material for CharacterMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_render/shaders/character_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_render/shaders/character_material.wesl".into()
    }

    /// The eye overlay (the eyebrow variant) blends over the scene; every
    /// other character draw is opaque.
    fn alpha_mode(&self) -> AlphaMode {
        if self.key.eyebrow {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }

    /// 本单只交付 Base 片元；阴影与深度消费方不在范围里。
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
        if key.bind_group_data.eyebrow {
            defs.push("CHARACTER_EYEBROW");
            // The eyebrow pass: depth test Always with depth write; its
            // stencil test (Equal against the character's own reference)
            // is read from the clip mask in the fragment.
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_compare = Some(CompareFunction::Always);
                depth.depth_write_enabled = Some(true);
            }
        }
        if key.bind_group_data.fog {
            defs.push("CHARACTER_FOG");
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

// ---- 全局量桥 ----

/// 全局量表的 GPU 布局：11 个 vec4 槽、176 字节。槽序是本文件与
/// `shaders/character_material.wesl` 里 `CharacterEnv` 结构体之间的
/// 契约，两边同改。字段集照律的 CharacterGlobals，外加雾斜坡重建要的
/// 投影参数（near/far）。
pub const ENV_SLOTS: usize = 11;
pub const ENV_BYTES: usize = ENV_SLOTS * 16;

/// 一帧的角色全局量。
#[derive(Debug, Clone, Resource, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct CharacterEnv {
    pub globals: CharacterGlobals,
    /// `(翻转位, 近裁剪, 远裁剪, 1/远裁剪)`；翻转位在本管线的 GL 约定下
    /// 恒 1。雾斜坡的深度重建用它。
    pub projection_params: [f32; 4],
    /// 雾档案是否已进表：换装与雾变体都等它。
    pub fog_ready: bool,
}

impl CharacterEnv {
    /// 中性初值：每个消费式子精确化简为无贡献（光色与双色全 1、雾零、
    /// 投影参数与站点相机一致以免除零）。
    pub fn neutral() -> Self {
        CharacterEnv {
            globals: CharacterGlobals {
                light_vector: [0.0, 0.0, 1.0],
                light_color: [1.0, 1.0, 1.0, 1.0],
                skin_shade_color: [1.0, 1.0, 1.0, 1.0],
                body_shade_color: [1.0, 1.0, 1.0, 1.0],
                face_sphere_shadow_edge: FACE_SPHERE_SHADOW_EDGE,
                face_sphere_shadow_smoothness: FACE_SPHERE_SHADOW_SMOOTHNESS,
                screen_params: [1.0, 1.0, 1.0, 1.0],
                mip_bias: [0.0, 0.0],
                fog_params: [0.0; 4],
                fog_near_color: [0.0; 4],
                fog_far_color: [0.0; 4],
            },
            projection_params: [1.0, 1.0, 4000.0, 0.00025],
            fog_ready: false,
        }
    }

    /// 按上面的槽序摊平成字节。
    pub fn gpu_bytes(&self) -> Vec<u8> {
        let g = &self.globals;
        let mut bytes = Vec::with_capacity(ENV_BYTES);
        let mut push = |slot: [f32; 4]| {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        };
        push(moly_law::weather::light_pass::clamp_zero4([
            g.light_vector[0],
            g.light_vector[1],
            g.light_vector[2],
            0.0,
        ]));
        push(g.light_color);
        push(g.skin_shade_color);
        push(g.body_shade_color);
        push([
            g.face_sphere_shadow_edge,
            g.face_sphere_shadow_smoothness,
            0.0,
            0.0,
        ]);
        push(g.screen_params);
        push([g.mip_bias[0], g.mip_bias[1], 0.0, 0.0]);
        push(g.fog_params);
        push(g.fog_near_color);
        push(g.fog_far_color);
        push(self.projection_params);
        debug_assert_eq!(bytes.len(), ENV_BYTES);
        bytes
    }
}

/// 已请求装载的晴天雾档案。
#[derive(Resource)]
pub struct CharacterFogAsset(Handle<MolyJson>);

/// Startup：中性表落位。
pub fn insert_neutral(mut commands: Commands) {
    commands.insert_resource(CharacterEnv::neutral());
}

/// Startup：晴天现象常量写入（光向、光色、双色），与图形配置的球面支两值。
pub fn apply_sunny(mut env: ResMut<CharacterEnv>) {
    let toward = light::dir_toward_light(light::ANGLE_XZ, light::ANGLE_Y);
    env.globals.light_vector = [toward.x, toward.y, toward.z];
    env.globals.light_color = SUNNY_LIGHT_COLOR;
    env.globals.skin_shade_color = SUNNY_SKIN_SHADE;
    env.globals.body_shade_color = SUNNY_BODY_SHADE;
    env.globals.face_sphere_shadow_edge = FACE_SPHERE_SHADOW_EDGE;
    env.globals.face_sphere_shadow_smoothness = FACE_SPHERE_SHADOW_SMOOTHNESS;
}

/// Startup：请求装载晴天雾档案（与站点共用同一份 postprocess 数据，
/// 各自独立持表）。
pub fn load_fog(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CharacterFogAsset(server.load::<MolyJson>(
        moly_assets::sunny_postprocess(),
    )));
}

/// Update：雾档案到达后写雾三件并置 fog_ready，写一次。
pub fn apply_fog(
    mut applied: Local<bool>,
    fog: Option<Res<CharacterFogAsset>>,
    server: Res<AssetServer>,
    json: Res<Assets<MolyJson>>,
    mut env: ResMut<CharacterEnv>,
) {
    if *applied {
        return;
    }
    let Some(fog) = fog else {
        return;
    };
    match server.load_state(&fog.0) {
        LoadState::Failed(err) => panic!("角色侧晴天雾档案装载失败：{err:?}"),
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(MolyJson::PostProcessFog(globals)) = json.get(&fog.0) else {
        return;
    };
    env.globals.fog_params = globals.fog_params;
    env.globals.fog_near_color = globals.fog_near_color;
    env.globals.fog_far_color = globals.fog_far_color;
    env.fog_ready = true;
    *applied = true;
    info!(
        "角色全局表进晴天雾：params {:?}，near {:?}，far {:?}",
        globals.fog_params, globals.fog_near_color, globals.fog_far_color
    );
}

/// PostUpdate: camera state refresh. The screen and projection parameters are
/// frame globals.
pub fn write_frame_state(
    mut env: ResMut<CharacterEnv>,
    // 只认 3D 主相机：气泡层的 2D 覆盖相机共存，不过滤则 single() 失败
    // 早退——脸球面分界消费的屏幕/投影参数会冻结在初值。
    cameras: Query<(&Projection, &Camera), With<Camera3d>>,
) {
    let Ok((projection, camera)) = cameras.single() else {
        return;
    };
    let (near, far) = match projection {
        Projection::Perspective(p) => (p.near, p.far),
        Projection::Orthographic(o) => (o.near, o.far),
        Projection::Custom(_) => panic!("角色全局量不认识自定义投影"),
    };
    // Written through change detection only when a bit changes: extraction and
    // the GPU upload key on the flag, and the camera state is steady most frames.
    let target = env.bypass_change_detection();
    let mut changed = store_bits(&mut target.projection_params, [1.0, near, far, 1.0 / far]);
    if let Some(size) = camera.physical_viewport_size() {
        let (w, h) = (size.x as f32, size.y as f32);
        changed |= store_bits(&mut target.globals.screen_params, [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h]);
    }
    if changed {
        env.set_changed();
    }
}

/// 渲染侧的全局量 buffer；一帧一份，全部角色材质共用。
#[derive(Resource)]
pub struct CharacterEnvGpuBuffer {
    pub buffer: Buffer,
}

fn create_env_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("character_env"),
        contents: &CharacterEnv::neutral().gpu_bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(CharacterEnvGpuBuffer { buffer });
}

fn write_env_buffer(
    env: Res<CharacterEnv>,
    buffer: Res<CharacterEnvGpuBuffer>,
    queue: Res<RenderQueue>,
) {
    // Extraction replaces the render-world table only on a frame where the
    // main-world table changed; the buffer already holds every earlier copy.
    if !env.is_changed() {
        return;
    }
    let bytes = env.gpu_bytes();
    queue.write_buffer(&buffer.buffer, 0, &bytes);
}

// ---- 骨架档案解析 ----

/// 一份骨架档案：顶层纹理表（URI 列表）与按名索引的材质槽。
struct RigFile {
    textures: Vec<String>,
    slots: HashMap<String, MaterialSlot>,
    /// The additional eye texture sets as (eye atlas URI, eyebrow mask URI).
    eye_texture_sets: Vec<(String, Option<String>)>,
}

/// The material slot that carries the eye atlas.
const EYE_SLOT: &str = "eye";

/// 解析骨架档案的 JSON 原串。材质槽的形状与站点 sidecar 同构
/// （浮点表/颜色表/纹理表），但纹理槽是 URI 直指、由顶层表折成下标。
/// 解析失败具名报错——资产加载边界，拒绝在这里发生一次。
fn parse_rig(raw: &str, file: &str) -> Result<RigFile, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("{file} 不是合法 JSON：{e}"))?;
    let textures: Vec<String> = value
        .get("textures")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{file} 缺顶层 textures 数组"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{file} 的 textures 数组里有非字符串项"))
        })
        .collect::<Result<_, _>>()?;
    let materials = value
        .get("materials")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{file} 缺 materials 数组"))?;
    let mut slots = HashMap::new();
    for material in materials {
        let name = material
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("{file} 有无名材质"))?;
        let shader = material
            .get("shader")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let keywords: Vec<String> = material
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|array| {
                array
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut slot_textures = Vec::new();
        if let Some(map) = material.get("textures").and_then(|v| v.as_object()) {
            for (prop, uri) in map {
                let uri = uri
                    .as_str()
                    .ok_or_else(|| format!("{file} 材质 {name} 的纹理槽 {prop} 不是 URI 字符串"))?;
                let index = textures
                    .iter()
                    .position(|candidate| candidate == uri)
                    .ok_or_else(|| {
                        format!("{file} 材质 {name} 的纹理 {uri} 不在顶层 textures 表里")
                    })?;
                slot_textures.push((prop.clone(), index));
            }
        }
        let mut colors = BTreeMap::new();
        if let Some(map) = material.get("colors").and_then(|v| v.as_object()) {
            for (prop, value) in map {
                let rgba: Vec<f64> = value
                    .as_array()
                    .ok_or_else(|| format!("{file} 材质 {name} 的颜色 {prop} 不是数组"))?
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .ok_or_else(|| format!("{file} 材质 {name} 的颜色 {prop} 有非数值项"))
                    })
                    .collect::<Result<_, _>>()?;
                if rgba.len() != 4 {
                    return Err(format!(
                        "{file} 材质 {name} 的颜色 {prop} 不是四分量"
                    ));
                }
                let mut fixed = [0.0f32; 4];
                for (target, source) in fixed.iter_mut().zip(rgba) {
                    *target = source as f32;
                }
                colors.insert(prop.clone(), fixed);
            }
        }
        let mut floats = BTreeMap::new();
        if let Some(map) = material.get("floats").and_then(|v| v.as_object()) {
            for (prop, value) in map {
                let number = value
                    .as_f64()
                    .ok_or_else(|| format!("{file} 材质 {name} 的浮点 {prop} 不是数值"))?;
                floats.insert(prop.clone(), number as f32);
            }
        }
        // 档案不带任何 ST 记录（顶层无、材质内也无）——变换表空着，
        // 律按缺省恒等变换消费。
        slots.insert(
            name.to_string(),
            MaterialSlot {
                name: name.to_string(),
                shader,
                keywords,
                textures: slot_textures,
                colors,
                texture_scale_offsets: BTreeMap::new(),
                floats,
            },
        );
    }
    let eye_texture_sets = match value.get("eyeTextureSets") {
        None => Vec::new(),
        Some(sets) => sets
            .as_array()
            .ok_or_else(|| format!("{file} 的 eyeTextureSets 不是数组"))?
            .iter()
            .map(|set| {
                let eye = set
                    .get("eye")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("{file} 的 eyeTextureSets 项缺 eye 字符串"))?;
                let mask = match set.get("eyebrowMask") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(uri)) => Some(uri.clone()),
                    Some(_) => {
                        return Err(format!("{file} 的 eyeTextureSets 项的 eyebrowMask 不是字符串"));
                    }
                };
                Ok((eye.to_string(), mask))
            })
            .collect::<Result<_, String>>()?,
    };
    Ok(RigFile { textures, slots, eye_texture_sets })
}

/// 从档案槽建材质资产：标量与门全过律的 `resolve`，纹理按 URI 直装载。
fn build_material(
    rig: &RigFile,
    slot: &MaterialSlot,
    file: &str,
    server: &AssetServer,
    render_device: &RenderDevice,
    fog: bool,
) -> Result<CharacterMaterial, String> {
    // 配件族的 shader 名不同、程序与本族逐字节相同（律的注释），按名放行。
    if slot.shader != law::SHADER_NAME && slot.shader != law::SHADER_NAME_ACCESSORY {
        return Err(format!(
            "{file} 材质 {} 的 shader {} 不是角色族程序",
            slot.name, slot.shader
        ));
    }
    let resolved = law::resolve(slot)?;
    let uri_of = |index: usize| -> Result<&str, String> {
        rig.textures
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("{file} 材质 {} 的纹理下标 {index} 越界", slot.name))
    };
    let load = |index: usize| -> Handle<Image> {
        // 角色贴图是资产根下的散装 PNG，URI 即文件名。
        // Only the GPU samples them (the material binds them; nothing reads
        // their texels on the CPU), and this is the only request of these paths.
        moly_assets::residency::load_image(
            server,
            AssetPath::from(format!("moly://{}", uri_of(index).expect("下标已核"))),
        )
    };
    let main_tex = resolved
        .main_tex
        .ok_or_else(|| format!("{file} 材质 {} 缺非空 _MainTex 槽", slot.name))?;
    let eyebrow_tex = resolved
        .eyebrow
        .map(|eyebrow| load(eyebrow.tex));
    let main_handle = load(main_tex);
    let eye_textures = (slot.name == EYE_SLOT).then(|| {
        let load_uri = |uri: &str| {
            moly_assets::residency::load_image(server, AssetPath::from(format!("moly://{uri}")))
        };
        EyeTextureSets::new(
            EyeTextures { main: main_handle.clone(), eyebrow: eyebrow_tex.clone() },
            rig.eye_texture_sets
                .iter()
                .map(|(eye, mask)| EyeTextures {
                    main: load_uri(eye),
                    eyebrow: mask.as_deref().map(load_uri),
                })
                .collect(),
        )
    });
    let params = character_params(&resolved, [0.0; 4]);
    let params_buffer = params_buffer(render_device, &params);
    Ok(CharacterMaterial {
        key: CharacterKey {
            // The eyebrow variant is gated on a keyword variant plus a
            // separate render pass (the Render Eyelash pass of the
            // MysekaiEyebrow LightMode, with a dedicated eyebrowRenderer and
            // the mtl_chr_eyebrow_00 material, none of which is in the
            // extracted character packages). The extracted eye/mouth slot
            // keywords are always empty: the `_EyebrowTex` reference is a
            // leftover of Unity serialization and the variant was never
            // enabled. A texture being present is not the variant being
            // enabled; keying on texture presence would apply the eyebrow
            // mask's discard (84% black over the sampled area) to the eye
            // mesh and drop the whole face.
            eyebrow: false,
            fog,
        },
        params,
        params_buffer,
        lip_pattern: None,
        lip_pattern_revision: 0,
        eye_pattern: None,
        main_tex: main_handle,
        body_mask_tex: resolved.body_mask_tex.map(load),
        eyebrow_tex,
        eye_textures,
    })
}

/// The parameter block of a resolved character material, with the head
/// reference point it starts from.
fn character_params(resolved: &law::CharacterMaterial, head_position: [f32; 4]) -> CharacterParams {
    CharacterParams {
        shader_usage: resolved.character_shader_usage as f32,
        // 眉标量缺席时槽写零：变体已按纹理槽关掉，死支路不消费。
        eyebrow_clip: resolved.eyebrow.map_or(0.0, |e| e.clip),
        eyebrow_alpha: resolved.eyebrow.map_or(0.0, |e| e.alpha),
        use_dither: resolved.use_dither,
        brightness: resolved.brightness,
        eyebrow_stencil: resolved.eyebrow_stencil,
        override_shading_parameter: resolved.override_shading_parameter,
        local_body_shading_intensity: resolved.local_body_shading_intensity,
        local_body_shading_edge_threshold: resolved.local_body_shading_edge_threshold,
        local_body_shading_edge_smoothness: resolved.local_body_shading_edge_smoothness,
        dither_alpha: resolved.dither_alpha,
        main_tex_st: resolved.main_tex_st.unwrap_or([1.0, 1.0, 0.0, 0.0]),
        head_position,
    }
}

fn params_buffer(render_device: &RenderDevice, params: &CharacterParams) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("character_material_params"),
        contents: &params.byte_array(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

/// A fixture part drawn with the character program (a footbath's soaking
/// legs): the material from the fixture's own property table and the
/// textures its glTF material retains, by property name.
///
/// The part belongs to the fixture, not to a character: the head reference
/// point is the material's own `_MysekaiCharacterHeadPosition` (read only
/// where the body mask's red channel selects the sphere regime), and the
/// part has no eye pattern or eyebrow pass. A part with an eyebrow mask is
/// refused.
pub fn fixture_part(
    slot: &MaterialSlot,
    textures: &BTreeMap<String, Handle<Image>>,
    render_device: &RenderDevice,
    fog: bool,
) -> Result<CharacterMaterial, String> {
    if slot.shader != law::SHADER_NAME {
        return Err(format!(
            "fixture material {}: shader {} is not the character program",
            slot.name, slot.shader
        ));
    }
    let resolved = law::resolve(slot)?;
    if resolved.eyebrow.is_some() {
        return Err(format!(
            "fixture material {}: an eyebrow mask on a fixture part has no eyebrow pass",
            slot.name
        ));
    }
    let texture = |property: &str| {
        textures.get(property).cloned().ok_or_else(|| {
            format!("fixture material {}: {property} is not retained by the import", slot.name)
        })
    };
    let main_tex = texture("_MainTex")?;
    let body_mask_tex = match resolved.body_mask_tex {
        Some(_) => Some(texture("_BodyMaskTex")?),
        None => None,
    };
    let head_position = *slot
        .colors
        .get("_MysekaiCharacterHeadPosition")
        .ok_or_else(|| {
            format!("fixture material {}: no _MysekaiCharacterHeadPosition", slot.name)
        })?;
    let mut params = character_params(&resolved, head_position);
    // Fixture meshes keep their source UVs (rows counted from the bottom),
    // while image rows begin at the top: the texture transform folds in
    // the row flip, as the fixture program does for its own materials.
    let [scale_u, scale_v, offset_u, offset_v] = params.main_tex_st;
    params.main_tex_st = [scale_u, -scale_v, offset_u, 1.0 - offset_v];
    let params_buffer = params_buffer(render_device, &params);
    Ok(CharacterMaterial {
        key: CharacterKey { eyebrow: false, fog },
        params,
        params_buffer,
        lip_pattern: None,
        lip_pattern_revision: 0,
        eye_pattern: None,
        main_tex,
        body_mask_tex,
        eyebrow_tex: None,
        eye_textures: None,
    })
}

/// The eye overlay of a slot that carries an eye mask: the same material
/// read through the eyebrow variant, with a parameter buffer of its own. It
/// is the source's eyebrow pass: drawn after the transparents with depth
/// test Always, kept only where the stencil holds the character's own
/// reference (read from the stencil clip mask), so it shows through that
/// character's hair and nowhere another draw wrote the stencil last.
fn build_eye_overlay(base: &CharacterMaterial, render_device: &RenderDevice) -> Option<CharacterMaterial> {
    base.eyebrow_tex.as_ref()?;
    let params_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("character_material_eye_overlay_params"),
        contents: &base.params.byte_array(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    Some(CharacterMaterial {
        key: CharacterKey {
            eyebrow: true,
            ..base.key
        },
        params: base.params,
        params_buffer,
        lip_pattern: None,
        lip_pattern_revision: 0,
        eye_pattern: None,
        main_tex: base.main_tex.clone(),
        body_mask_tex: base.body_mask_tex.clone(),
        eyebrow_tex: base.eyebrow_tex.clone(),
        // The overlay copies the slot's textures every frame.
        eye_textures: None,
    })
}

// ---- 换装 ----

/// 已解析待换装的逐实体计划（材质资产等贴图到齐）。
#[derive(Component)]
pub struct ToonPlan {
    /// (网格实体, 材质名, 材质资产)。
    parts: Vec<(Entity, String, CharacterMaterial)>,
    /// (mesh entity, slot name, overlay material) of every part with an eye
    /// mask.
    overlays: Vec<(Entity, String, CharacterMaterial)>,
    /// 头骨实体：头参考点的逐帧写者读它。
    head: Entity,
    hips: Entity,
}

/// 换装完成闩：头写者与日志读它。槽带材质名（`eye` / `mouth` 等——
/// 按名取槽是 facial 图集格的写点）。
#[derive(Component)]
pub struct ToonMaterials {
    head: Entity,
    hips: Entity,
    slots: Vec<(String, Handle<CharacterMaterial>)>,
    /// (slot material, its eye overlay material): the overlay copies the
    /// slot's parameters every frame.
    overlays: Vec<(Handle<CharacterMaterial>, Handle<CharacterMaterial>)>,
}

impl ToonMaterials {
    /// The resolved skeleton already owns this reference; interaction queries
    /// read its current world transform instead of searching the hierarchy again.
    pub fn head_entity(&self) -> Entity { self.head }

    pub fn hips_entity(&self) -> Entity {
        self.hips
    }

    /// 按材质名取该成员的 toon 材质句柄（如 `eye` / `mouth`）。
    pub fn slot_handle(&self, name: &str) -> Option<&Handle<CharacterMaterial>> {
        self.slots
            .iter()
            .find(|(slot, _)| slot == name)
            .map(|(_, handle)| handle)
    }

    /// Every material handle of the member.
    pub fn slot_handles(&self) -> impl Iterator<Item = &Handle<CharacterMaterial>> {
        self.slots.iter().map(|(_, handle)| handle)
    }
}

/// 头骨标记（按节点名装配时插上）。
#[derive(Component)]
pub struct HeadBone;

/// A character body the player wears in place of the avatar body. Like the
/// member the player controls, its materials keep alpha 1.0 without dither.
#[derive(Component)]
pub struct PlayerWorn;

/// A member's name in logs and refusals: the cast member's unit, or the
/// player's worn body.
fn member_label(unit: Option<&CharacterUnitId>) -> String {
    unit.map_or_else(|| "player body".to_owned(), |unit| format!("npc unit={}", unit.0))
}

/// One body's toon plan: every mesh below `model` joined by its glb material
/// name to the rig profile's slots, and its Head and Hips bones. The head
/// marker and the accessories' shadow opt-out are inserted once the plan
/// holds.
pub fn plan_body(
    commands: &mut Commands,
    server: &AssetServer,
    render_device: &RenderDevice,
    gltf: &Gltf,
    rig_raw: &str,
    rig_file: &str,
    model: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    materials: &Query<&MeshMaterial3d<StandardMaterial>>,
) -> Result<ToonPlan, String> {
    let rig = parse_rig(rig_raw, rig_file).map_err(|reason| format!("骨架档案解析失败：{reason}"))?;
    // glb 材质句柄 → 名：换装按名 join 档案。
    let mut name_by_handle: HashMap<Handle<StandardMaterial>, String> = HashMap::new();
    for (name, handle) in &gltf.named_materials {
        let standard = crate::site_material::standard_material_of(server, handle)
            .ok_or_else(|| format!("glb 命名材质 {name} 没有装载器派生的 StandardMaterial"))?;
        name_by_handle.insert(standard, name.to_string());
    }
    // 模型子实体 → 整树走一遍：收网格实体的材质与头骨节点。
    let mut stack = vec![model];
    let mut mesh_parts: Vec<(Entity, String)> = Vec::new();
    let mut head = None;
    let mut hips = None;
    while let Some(entity) = stack.pop() {
        if let Ok(name) = names.get(entity) {
            if name.as_str() == "Hips" { hips = Some(entity); }
            if name.as_str() == HEAD_BONE_NAME {
                if let Some(previous) = head {
                    return Err(format!(
                        "角色包里不止一个 {HEAD_BONE_NAME} 节点（{previous:?} 与 {entity:?}）"
                    ));
                }
                head = Some(entity);
            }
        }
        if let Ok(material) = materials.get(entity) {
            let name = name_by_handle
                .get(&material.0)
                .ok_or("有网格实体的材质不在 glb 命名材质表里")?;
            mesh_parts.push((entity, name.clone()));
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids.iter() {
                stack.push(kid);
            }
        }
    }
    let head = head.ok_or_else(|| format!("角色包里没有 {HEAD_BONE_NAME} 节点：头参考点无处可写"))?;
    let hips = hips.ok_or("角色缺少 Hips 骨节点")?;
    if mesh_parts.is_empty() {
        return Err("角色包里没有带 StandardMaterial 的网格实体".into());
    }
    let mut parts = Vec::new();
    let mut overlays = Vec::new();
    let mut accessories = Vec::new();
    for (entity, name) in mesh_parts {
        let slot = rig
            .slots
            .get(name.as_str())
            .ok_or_else(|| format!("骨架档案里没有 glb 网格材质 {name}"))?;
        // The accessory program declares no ShadowCaster pass, so its
        // renderer never draws into the main light's shadow map.
        if slot.shader == law::SHADER_NAME_ACCESSORY {
            accessories.push(entity);
        }
        let material = build_material(&rig, slot, rig_file, server, render_device, true)
            .map_err(|reason| format!("角色材质解析失败：{reason}"))?;
        if let Some(overlay) = build_eye_overlay(&material, render_device) {
            overlays.push((entity, name.clone(), overlay));
        }
        parts.push((entity, name, material));
    }
    commands.entity(head).insert(HeadBone);
    for entity in accessories {
        commands.entity(entity).insert(crate::shadowmap::NoShadowCast);
    }
    Ok(ToonPlan { parts, overlays, head, hips })
}

/// Update：装配完成的成员解析骨架档案、建材质计划。等雾档案进表
/// （雾变体按桥在场分，全侧一致）；档案装载失败响亮失败。
pub fn plan_when_wired(
    mut commands: Commands,
    server: Res<AssetServer>,
    render_device: Res<RenderDevice>,
    gltfs: Res<Assets<Gltf>>,
    jsons: Res<Assets<JsonAsset>>,
    env: Res<CharacterEnv>,
    // 闩是双层的：ToonPlan 是「计划已建、贴图未齐」的闩，换装完成后即
    // 撤；ToonMaterials 是永不撤的完成闩。排除两个都不排除后者时，换装
    // 撤掉 ToonPlan 的下一帧本系统会重进成员，把已换装的树再走一遍，
    // 在「没有 StandardMaterial 网格」上响亮误报。
    npcs: Query<
        (Entity, &CharacterUnitId, &CharacterPack),
        (With<CharacterDriver>, Without<ToonPlan>, Without<ToonMaterials>),
    >,
    children: Query<&Children>,
    names: Query<&Name>,
    models: Query<&CharacterModel>,
    materials: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    if !env.fog_ready {
        return;
    }
    for (npc, unit, pack) in &npcs {
        match server.load_state(&pack.rig) {
            LoadState::Failed(err) => {
                panic!("unit {} 的骨架档案 {} 装载失败：{err:?}", unit.0, pack.rig_file)
            }
            LoadState::Loaded => {}
            _ => continue, // 这名等下一帧，不挡别人
        }
        let Some(json) = jsons.get(&pack.rig) else {
            continue;
        };
        let Some(gltf) = gltfs.get(&pack.gltf) else {
            continue;
        };
        let Some(model) = children
            .get(npc)
            .expect("模型挂载后有子链")
            .iter()
            .find(|kid| models.get(*kid).is_ok())
        else {
            panic!("unit {} 的成员实体下没有 CharacterModel 子实体", unit.0);
        };
        let plan = plan_body(
            &mut commands,
            &server,
            &render_device,
            gltf,
            &json.0,
            &pack.rig_file,
            model,
            &children,
            &names,
            &materials,
        )
        .unwrap_or_else(|reason| panic!("unit {}：{reason}", unit.0));
        commands.entity(npc).insert(plan);
    }
}

/// 头骨骼名：标准人形链的头节点（上接 Spine1）。档案里另有 HeadRoot
/// （挂在 Hips 下的无子节点），不是头骨。
const HEAD_BONE_NAME: &str = "Head";

/// 一份计划的贴图装载进度：全到、未齐、或第一张失败的原因。
pub enum PlanTextures {
    Loaded,
    Pending,
    Failed(String),
}

pub fn plan_textures(plan: &ToonPlan, server: &AssetServer) -> PlanTextures {
    let mut state = PlanTextures::Loaded;
    for (_, _, material) in &plan.parts {
        let textures = std::iter::once(&material.main_tex)
            .chain(material.body_mask_tex.iter())
            .chain(material.eyebrow_tex.iter())
            .chain(material.eye_textures.iter().flat_map(EyeTextureSets::handles));
        for texture in textures {
            match server.load_state(texture) {
                LoadState::Failed(err) => return PlanTextures::Failed(format!("{err:?}")),
                LoadState::Loaded => {}
                _ => state = PlanTextures::Pending,
            }
        }
    }
    state
}

/// Update：计划就绪且贴图到齐后换装——摘 StandardMaterial、挂角色
/// toon 材质。早于贴图到位会让实体闪回默认材质，所以等齐再换。
/// 成员的贴图装载失败即 panic；玩家穿的角色身体由
/// `crate::player_avatar::oc` 拒绝该选择、退回豆腐人身体。
pub fn swap_when_planned(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    npcs: Query<(Entity, Option<&CharacterUnitId>, &ToonPlan, Has<PlayerWorn>), Without<ToonMaterials>>,
    meshes: Query<(&Mesh3d, Option<&bevy::mesh::skinning::SkinnedMesh>, Option<&bevy::camera::visibility::RenderLayers>)>,
) {
    for (npc, unit, plan, worn) in &npcs {
        let who = member_label(unit);
        match plan_textures(plan, &server) {
            PlanTextures::Loaded => {}
            PlanTextures::Pending => continue,
            PlanTextures::Failed(_) if worn => continue,
            PlanTextures::Failed(err) => panic!("{who} 的角色贴图装载失败：{err}"),
        }
        let mut slots = Vec::new();
        let mut names = Vec::new();
        for (entity, name, material) in &plan.parts {
            let handle = materials.add(material.clone());
            commands
                .entity(*entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle.clone()));
            slots.push((name.clone(), handle));
            names.push(name.clone());
        }
        // The eye overlay: a child draw of the slot's mesh with the overlay
        // material, skinned by the same joints.
        let mut overlays = Vec::new();
        for (entity, name, material) in &plan.overlays {
            let Some((_, source)) = slots.iter().find(|(slot, _)| slot == name) else {
                panic!("{who} 的眼睛叠层材质 {name} 没有对应的槽材质");
            };
            let Ok((mesh, skin, layers)) = meshes.get(*entity) else {
                panic!("{who} 的网格实体 {entity:?} 没有 Mesh3d：眼睛叠层无处挂");
            };
            let handle = materials.add(material.clone());
            let mut overlay = commands.spawn((
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(handle.clone()),
                Transform::default(),
                Visibility::Inherited,
                bevy::camera::visibility::NoFrustumCulling,
                crate::shadowmap::NoShadowCast,
                ChildOf(*entity),
            ));
            if let Some(skin) = skin {
                overlay.insert(skin.clone());
            }
            if let Some(layers) = layers {
                overlay.insert(layers.clone());
            }
            overlays.push((source.clone(), handle));
        }
        commands
            .entity(npc)
            .insert(ToonMaterials {
                head: plan.head,
                hips: plan.hips,
                slots,
                overlays,
            })
            .remove::<ToonPlan>();
        info!(
            "[{who}] 角色材质换装 {} 实体（{}）",
            names.len(),
            names.join(",")
        );
    }
}

/// PostUpdate（TransformPropagate 之后）：头参考点回写——头骨实体的
/// 世界位写进该成员全部材质槽。源每帧写、这里同节奏；动画在
/// Propagate 之前推进，读到的是当帧姿势。
///
/// The values are stored without an asset event, so the material is not
/// re-prepared; `upload_params` copies them into the material's parameter
/// buffer during this frame's extraction.
pub fn update_head(
    mut materials: ResMut<Assets<CharacterMaterial>>,
    npcs: Query<(
        Option<&CharacterUnitId>,
        &ToonMaterials,
        Has<crate::markers::PlayerControlled>,
        Has<PlayerWorn>,
    )>,
    globals: Query<&GlobalTransform>,
) {
    // The NPC dither alpha is the presenter's call 5 (npc_dither). A member
    // the player controls, and a body the player wears, keep alpha 1.0
    // without dither: the player view's own dither is not in this host.
    for (unit, toon, controlled, worn) in &npcs {
        let Ok(global) = globals.get(toon.head) else { continue; };
        let position = global.translation();
        if position == Vec3::ZERO {
            panic!("{} 的头参考点是零向量：头骨的世界变换没有传播", member_label(unit));
        }
        let head_position = [position.x, position.y, position.z, 0.0];
        let player = controlled || worn;
        for (_, handle) in &toon.slots {
            let changed = materials.get(handle).is_some_and(|material| {
                material.params.head_position.map(f32::to_bits) != head_position.map(f32::to_bits)
                    || (player
                        && (material.params.dither_alpha.to_bits() != 1.0_f32.to_bits()
                            || material.params.use_dither.to_bits() != 0.0_f32.to_bits()))
            });
            if !changed {
                continue;
            }
            if let Some(material) = materials.get_mut_untracked(handle) {
                material.params.head_position = head_position;
                if player {
                    material.params.dither_alpha = 1.0;
                    material.params.use_dither = 0.0;
                }
            }
        }
    }
}

/// PostUpdate（update_head 之后）：眼睛叠层材质逐帧抄它的槽材质参数
/// （表情格、抖动、头参考点都写在槽材质上）。
///
/// Like `update_head`, the copy is stored without an asset event;
/// `upload_params` writes the buffer during extraction.
pub fn sync_eye_overlay(
    mut materials: ResMut<Assets<CharacterMaterial>>,
    toons: Query<&ToonMaterials>,
) {
    for toon in &toons {
        for (source, overlay) in &toon.overlays {
            let Some((params, main_tex, eyebrow_tex)) = materials.get(source).map(|material| {
                (material.params, material.main_tex.clone(), material.eyebrow_tex.clone())
            }) else {
                continue;
            };
            let Some(current) = materials.get(overlay) else {
                continue;
            };
            let textures_stale = current.main_tex != main_tex || current.eyebrow_tex != eyebrow_tex;
            let params_stale = current.params.byte_array() != params.byte_array();
            if textures_stale {
                // A texture change rebuilds the bind group, so it is a tracked write.
                if let Some(mut material) = materials.get_mut(overlay) {
                    material.params = params;
                    material.main_tex = main_tex;
                    material.eyebrow_tex = eyebrow_tex;
                }
            } else if params_stale {
                if let Some(material) = materials.get_mut_untracked(overlay) {
                    material.params = params;
                }
            }
        }
    }
}

/// 周期日志：头参考点的世界位现算证据（非零、随走路移动可从日志推导）。
pub fn report_head(
    npcs: Query<(&CharacterUnitId, &ToonMaterials)>,
    globals: Query<&GlobalTransform>,
) {
    if !bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
        return;
    }
    for (unit, toon) in &npcs {
        if let Ok(global) = globals.get(toon.head) {
            let position = global.translation();
            debug!(
                "[npc unit={}] 头参考点世界位 {:.3} {:.3} {:.3}",
                unit.0, position.x, position.y, position.z
            );
        }
    }
}

/// Render world: the bytes last uploaded into each material's parameter
/// buffer, and which material owns each buffer in the current frame.
#[derive(Resource, Default)]
struct ParamUploads {
    written: HashMap<AssetId<CharacterMaterial>, ([u8; PARAMS_BYTES], u64)>,
    owners: HashMap<BufferId, AssetId<CharacterMaterial>>,
    frame: u64,
}

/// Extract: copies each character material's current parameters into its
/// parameter buffer when they differ from the last upload. Extraction runs
/// after the main schedule, so the buffer holds the parameters as they stand
/// at the end of the frame, which is what a material re-prepared this frame
/// would have packed into a new buffer. The write is queued before the frame
/// is submitted, so this frame's draws read it.
fn upload_params(
    materials: Extract<Res<Assets<CharacterMaterial>>>,
    mut uploads: ResMut<ParamUploads>,
    queue: Res<RenderQueue>,
) {
    let uploads = &mut *uploads;
    uploads.frame += 1;
    let frame = uploads.frame;
    uploads.owners.clear();
    for (id, material) in materials.iter() {
        if let Some(other) = uploads.owners.insert(material.params_buffer.id(), id) {
            panic!("角色材质 {other:?} 与 {id:?} 共用同一块参数 buffer：每个材质资产必须持有自己的一块");
        }
        let bytes = material.params.byte_array();
        let stale = match uploads.written.get_mut(&id) {
            Some((written, seen)) => {
                *seen = frame;
                let stale = *written != bytes;
                *written = bytes;
                stale
            }
            None => {
                uploads.written.insert(id, (bytes, frame));
                true
            }
        };
        if stale {
            queue.write_buffer(&material.params_buffer, 0, &bytes);
        }
    }
    uploads.written.retain(|_, (_, seen)| *seen == frame);
}

/// 角色材质插件：材质管线 + 全局量桥（含渲染侧 buffer）。
/// `.json` 装载器只在 wasm 注册：native 侧站点插件先于本插件把同一对
/// 注册进了 AssetServer，重复注册打 WARN；wasm 没有站点插件，雾档案
/// 的装载器由本插件自带。
pub struct CharacterMaterialPlugin;

impl Plugin for CharacterMaterialPlugin {
    fn build(&self, app: &mut App) {
        // The material plugin resolves the shader path while it is added.
        bevy::asset::embedded_asset!(app, "shaders/character_material.wesl");
        crate::render::gpu::install_shared_samplers(app);
        crate::gpu_image_release::prepare_after_images::<CharacterMaterial>(app);
        app.add_plugins((
            MaterialPlugin::<CharacterMaterial>::default(),
            ExtractResourcePlugin::<CharacterEnv>::default(),
        ));
        #[cfg(target_arch = "wasm32")]
        app.init_asset::<MolyJson>()
            .init_asset_loader::<MolyJsonLoader>();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ParamUploads>()
                .add_systems(ExtractSchedule, upload_params)
                .add_systems(RenderStartup, create_env_buffer)
                // Prepare 链在 ExtractCommands 之后：抽取资源用 Commands 落
                // 渲染世界，无序挂载会读到不存在的那一帧。
                .add_systems(Render, write_env_buffer.in_set(RenderSystems::Prepare));
        }
    }
}
