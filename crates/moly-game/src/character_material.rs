//! 角色 toon 材质：`Mysekai/Character` 程序的 Bevy Material。
//!
//! 式子全在 moly-law 的 shading::character（纯函数 + `resolve`），本模块
//! 只做装配：把骨架档案（rig json）里的 Unity 属性喂给律的 `resolve`，
//! 得到的值摊进 uniform 槽；WGSL 在 `shaders/character_material.wgsl`，
//! 逐句转录律的片元链。变体两件：眉按纹理槽在场（已裁），雾按全局量
//! 桥在场（换装等雾档案到齐，变体全侧一致）。
//!
//! 头参考点是**材质槽**量不是全局量：源里它是 MaterialPropertyBlock
//! 的 uniform、按渲染器各写各的，所以它进每材质自己的参数块，由头骨
//! 实体的世界位逐帧回写——缺省留在原点时球面距离场中心滑到世界原点，
//! 球面界线整条错位（分界线成因之一）。

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
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_assets::sidecar::MolyJson;
#[cfg(target_arch = "wasm32")]
use moly_assets::sidecar::MolyJsonLoader;
use moly_law::material::MaterialSlot;
use moly_law::shading::character as law;
use moly_law::shading::character::CharacterGlobals;

use crate::character::{CharacterModel, CharacterPack, MotionDriver};
use crate::light;
use crate::npc::CharacterUnitId;

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

/// 球面支 edge 的具名替身。两作出处：源侧这两量是引擎全局量、提取产物
/// 与语料都读不出值；demo（用户验收过的观感基线）取 0.0。读出真值后
/// 改这里一处。
const SPHERE_EDGE_STAND_IN: f32 = 0.0;
/// 球面支 smoothness 的具名替身，同上；demo 取 1.0。
const SPHERE_SMOOTHNESS_STAND_IN: f32 = 1.0;

// ---- 材质参数块 ----

/// 参数 uniform 的槽数与字节数。槽序是本文件与
/// `shaders/character_material.wgsl` 里 `CharacterParams` 结构体之间的
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
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        for slot in [
            [
                self.shader_usage,
                self.eyebrow_clip,
                self.eyebrow_alpha,
                self.use_dither,
            ],
            [self.brightness, 0.0, 0.0, 0.0],
            [self.override_shading_parameter, 0.0, 0.0, 0.0],
            [self.local_body_shading_intensity, 0.0, 0.0, 0.0],
            [self.local_body_shading_edge_threshold, 0.0, 0.0, 0.0],
            [self.local_body_shading_edge_smoothness, 0.0, 0.0, 0.0],
            [self.dither_alpha, 0.0, 0.0, 0.0],
            self.main_tex_st,
            self.head_position,
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), PARAMS_BYTES);
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

/// 角色 toon 材质资产。按名独立成类，不与站点的 SiteMaterial 混。
#[derive(Debug, Clone, TypePath, Asset)]
pub struct CharacterMaterial {
    pub key: CharacterKey,
    pub params: CharacterParams,
    /// Instance-local full-row input for the actor's mouth controller. These
    /// host fields are not GPU bindings; speech texture writes never touch them.
    pub(crate) lip_pattern: Option<moly_law::facial::LipPattern>,
    pub(crate) lip_pattern_revision: u64,
    pub main_tex: Handle<Image>,
    /// body 槽的遮罩贴图；其余槽没有它，绑白色 fallback（采样值不进
    /// 有效支路）。
    pub body_mask_tex: Option<Handle<Image>>,
    /// 眉贴图；只有带 `_EyebrowTex` 槽的材质有它。
    pub eyebrow_tex: Option<Handle<Image>>,
}

impl AsBindGroup for CharacterMaterial {
    type Data = CharacterKey;
    type Param = (
        SRes<CharacterEnvGpuBuffer>,
        SRes<RenderAssets<GpuImage>>,
        SRes<FallbackImage>,
    );

    fn label() -> &'static str {
        "character_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        render_device: &RenderDevice,
        (env_buffer, images, fallback): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
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
        // 角色贴图不滚动、ST 是恒等变换，uv 不越 [0,1]；采样态不在语料
        // 记录里，取引擎默认的钳边与三线性——与 GpuImage 自带 sampler 的
        // 过滤一致，只有寻址模式不同。
        let clamp = render_device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..Default::default()
        });
        let bindings = BindingResources(vec![
            // binding 0：材质自己的参数块（含头参考点）。
            (0, OwnedBindingResource::Data(OwnedData(self.params.bytes()))),
            // binding 1：全局量，全部角色材质共用一个 buffer。
            (1, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (3, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, clamp.clone())),
            (
                4,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    body_mask.texture_view.clone(),
                ),
            ),
            (5, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, clamp.clone())),
            (
                6,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    eyebrow.texture_view.clone(),
                ),
            ),
            (7, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, clamp)),
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
        vec![
            uniform(0),
            uniform(1),
            texture(2),
            sampler(3),
            texture(4),
            sampler(5),
            texture(6),
            sampler(7),
        ]
    }
}

impl Material for CharacterMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/character_material.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/character_material.wgsl".into()
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
/// `shaders/character_material.wgsl` 里 `CharacterEnv` 结构体之间的
/// 契约，两边同改。字段集照律的 CharacterGlobals，外加雾斜坡重建要的
/// 投影参数（near/far）。
pub const ENV_SLOTS: usize = 11;
pub const ENV_BYTES: usize = ENV_SLOTS * 16;

/// 一帧的角色全局量。
#[derive(Debug, Clone, Resource, ExtractResource)]
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
                face_sphere_shadow_edge: SPHERE_EDGE_STAND_IN,
                face_sphere_shadow_smoothness: SPHERE_SMOOTHNESS_STAND_IN,
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
        push([g.light_vector[0], g.light_vector[1], g.light_vector[2], 0.0]);
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

/// Startup：晴天现象常量写入（光向、光色、双色、球面替身值）。
pub fn apply_sunny(mut env: ResMut<CharacterEnv>) {
    let toward = light::dir_toward_light(light::ANGLE_XZ, light::ANGLE_Y);
    env.globals.light_vector = [toward.x, toward.y, toward.z];
    env.globals.light_color = SUNNY_LIGHT_COLOR;
    env.globals.skin_shade_color = SUNNY_SKIN_SHADE;
    env.globals.body_shade_color = SUNNY_BODY_SHADE;
    env.globals.face_sphere_shadow_edge = SPHERE_EDGE_STAND_IN;
    env.globals.face_sphere_shadow_smoothness = SPHERE_SMOOTHNESS_STAND_IN;
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

/// PostUpdate：相机态刷新——屏幕参数与投影参数，真源里是每帧全局量。
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
    env.projection_params = [1.0, near, far, 1.0 / far];
    if let Some(size) = camera.physical_viewport_size() {
        let (w, h) = (size.x as f32, size.y as f32);
        env.globals.screen_params = [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h];
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
    let bytes = env.gpu_bytes();
    queue.write_buffer(&buffer.buffer, 0, &bytes);
}

// ---- 骨架档案解析 ----

/// 一份骨架档案：顶层纹理表（URI 列表）与按名索引的材质槽。
struct RigFile {
    textures: Vec<String>,
    slots: HashMap<String, MaterialSlot>,
}

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
    Ok(RigFile { textures, slots })
}

/// 从档案槽建材质资产：标量与门全过律的 `resolve`，纹理按 URI 直装载。
fn build_material(
    rig: &RigFile,
    slot: &MaterialSlot,
    file: &str,
    server: &AssetServer,
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
        server.load::<Image>(AssetPath::from(format!("moly://{}", uri_of(index).expect("下标已核"))))
    };
    let main_tex = resolved
        .main_tex
        .ok_or_else(|| format!("{file} 材质 {} 缺非空 _MainTex 槽", slot.name))?;
    let eyebrow_tex = resolved
        .eyebrow
        .map(|eyebrow| load(eyebrow.tex));
    let params = CharacterParams {
        shader_usage: resolved.character_shader_usage as f32,
        // 眉标量缺席时槽写零：变体已按纹理槽关掉，死支路不消费。
        eyebrow_clip: resolved.eyebrow.map_or(0.0, |e| e.clip),
        eyebrow_alpha: resolved.eyebrow.map_or(0.0, |e| e.alpha),
        use_dither: resolved.use_dither,
        brightness: resolved.brightness,
        override_shading_parameter: resolved.override_shading_parameter,
        local_body_shading_intensity: resolved.local_body_shading_intensity,
        local_body_shading_edge_threshold: resolved.local_body_shading_edge_threshold,
        local_body_shading_edge_smoothness: resolved.local_body_shading_edge_smoothness,
        dither_alpha: resolved.dither_alpha,
        main_tex_st: resolved.main_tex_st.unwrap_or([1.0, 1.0, 0.0, 0.0]),
        head_position: [0.0; 4],
    };
    Ok(CharacterMaterial {
        key: CharacterKey {
            // 眉变体的门在真源是 keyword 变体 + 独立渲染通道（MysekaiEyebrow
            // LightMode 的 Render Eyelash pass，专用 eyebrowRenderer 与
            // mtl_chr_eyebrow_00 材质——都不在提取的角色包里）。提取的
            // eye/mouth 槽 keywords 恒空：_EyebrowTex 引用是 Unity 序列化
            // 残留，变体从未开。纹理在场 ≠ 变体开——按纹理在场开门会把
            // 眉蒙版（采样区 84% 黑）的 discard 套到眼网格上，整片脸丢。
            eyebrow: false,
            fog,
        },
        params,
        lip_pattern: None,
        lip_pattern_revision: 0,
        main_tex: load(main_tex),
        body_mask_tex: resolved.body_mask_tex.map(load),
        eyebrow_tex,
    })
}

// ---- 换装 ----

/// 已解析待换装的逐实体计划（材质资产等贴图到齐）。
#[derive(Component)]
pub struct ToonPlan {
    /// (网格实体, 材质名, 材质资产)。
    parts: Vec<(Entity, String, CharacterMaterial)>,
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
}

impl ToonMaterials {
    /// The resolved skeleton already owns this reference; interaction queries
    /// read its current world transform instead of searching the hierarchy again.
    pub(crate) fn head_entity(&self) -> Entity { self.head }

    pub(crate) fn hips_entity(&self) -> Entity {
        self.hips
    }

    /// 按材质名取该成员的 toon 材质句柄（如 `eye` / `mouth`）。
    pub(crate) fn slot_handle(&self, name: &str) -> Option<&Handle<CharacterMaterial>> {
        self.slots
            .iter()
            .find(|(slot, _)| slot == name)
            .map(|(_, handle)| handle)
    }
}

/// 头骨标记（按节点名装配时插上）。
#[derive(Component)]
pub struct HeadBone;

/// Update：装配完成的成员解析骨架档案、建材质计划。等雾档案进表
/// （雾变体按桥在场分，全侧一致）；档案装载失败响亮失败。
#[allow(clippy::type_complexity)]
pub fn plan_when_wired(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    jsons: Res<Assets<JsonAsset>>,
    env: Res<CharacterEnv>,
    // 闩是双层的：ToonPlan 是「计划已建、贴图未齐」的闩，换装完成后即
    // 撤；ToonMaterials 是永不撤的完成闩。排除两个都不排除后者时，换装
    // 撤掉 ToonPlan 的下一帧本系统会重进成员，把已换装的树再走一遍，
    // 在「没有 StandardMaterial 网格」上响亮误报。
    npcs: Query<
        (Entity, &CharacterUnitId, &CharacterPack),
        (Or<(With<MotionDriver>, With<crate::player_avatar::AvatarDriver>)>, Without<ToonPlan>, Without<ToonMaterials>),
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
        let rig = match parse_rig(&json.0, &pack.rig_file) {
            Ok(rig) => rig,
            Err(reason) => panic!("unit {} 的骨架档案解析失败：{reason}", unit.0),
        };
        let Some(gltf) = gltfs.get(&pack.gltf) else {
            continue;
        };
        // glb 材质句柄 → 名：换装按名 join 档案。
        let mut name_by_handle: HashMap<&Handle<StandardMaterial>, String> = HashMap::new();
        for (name, handle) in &gltf.named_materials {
            name_by_handle.insert(handle, name.to_string());
        }
        // 模型子实体 → 整树走一遍：收网格实体的材质与头骨节点。
        let Some(model) = children
            .get(npc)
            .expect("模型挂载后有子链")
            .iter()
            .find(|kid| models.get(*kid).is_ok())
        else {
            panic!("unit {} 的成员实体下没有 CharacterModel 子实体", unit.0);
        };
        let mut stack = vec![model];
        let mut mesh_parts: Vec<(Entity, String)> = Vec::new();
        let mut head = None;
        let mut hips = None;
        while let Some(entity) = stack.pop() {
            if let Ok(name) = names.get(entity) {
                if name.as_str() == "Hips" { hips = Some(entity); }
                if name.as_str() == HEAD_BONE_NAME {
                    if let Some(previous) = head {
                        panic!(
                            "unit {} 的角色包里不止一个 {HEAD_BONE_NAME} 节点（{previous:?} 与 {entity:?}）",
                            unit.0
                        );
                    }
                    head = Some(entity);
                    commands.entity(entity).insert(HeadBone);
                }
            }
            if let Ok(material) = materials.get(entity) {
                let name = name_by_handle.get(&material.0).unwrap_or_else(|| {
                    panic!("unit {} 有网格实体的材质不在 glb 命名材质表里", unit.0)
                });
                mesh_parts.push((entity, name.clone()));
            }
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    stack.push(kid);
                }
            }
        }
        let head = head.unwrap_or_else(|| {
            panic!(
                "unit {} 的角色包里没有 {HEAD_BONE_NAME} 节点：头参考点无处可写",
                unit.0
            )
        });
        if mesh_parts.is_empty() {
            panic!("unit {} 的角色包里没有带 StandardMaterial 的网格实体", unit.0);
        }
        let mut parts = Vec::new();
        for (entity, name) in mesh_parts {
            let slot = rig.slots.get(name.as_str()).unwrap_or_else(|| {
                panic!("unit {} 的骨架档案里没有 glb 网格材质 {name}", unit.0)
            });
            let material = match build_material(&rig, slot, &pack.rig_file, &server, true) {
                Ok(material) => material,
                Err(reason) => panic!("unit {} 的角色材质解析失败：{reason}", unit.0),
            };
            parts.push((entity, name, material));
        }
        commands.entity(npc).insert(ToonPlan { parts, head, hips: hips.expect("角色缺少 Hips 骨节点") });
    }
}

/// 头骨骼名：标准人形链的头节点（上接 Spine1）。档案里另有 HeadRoot
/// （挂在 Hips 下的无子节点），不是头骨。
const HEAD_BONE_NAME: &str = "Head";

/// Update：计划就绪且贴图到齐后换装——摘 StandardMaterial、挂角色
/// toon 材质。早于贴图到位会让实体闪回默认材质，所以等齐再换。
pub fn swap_when_planned(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    npcs: Query<(Entity, &CharacterUnitId, &ToonPlan), Without<ToonMaterials>>,
) {
    for (npc, unit, plan) in &npcs {
        let mut all_loaded = true;
        for (_, _, material) in &plan.parts {
            let textures = std::iter::once(&material.main_tex)
                .chain(material.body_mask_tex.iter())
                .chain(material.eyebrow_tex.iter());
            for texture in textures {
                match server.load_state(texture) {
                    LoadState::Failed(err) => {
                        panic!("unit {} 的角色贴图装载失败：{err:?}", unit.0)
                    }
                    LoadState::Loaded => {}
                    _ => all_loaded = false,
                }
            }
        }
        if !all_loaded {
            continue;
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
        commands
            .entity(npc)
            .insert(ToonMaterials {
                head: plan.head,
                hips: plan.hips,
                slots,
            })
            .remove::<ToonPlan>();
        info!(
            "[npc unit={}] 角色材质换装 {} 实体（{}）",
            unit.0,
            names.len(),
            names.join(",")
        );
    }
}

/// PostUpdate（TransformPropagate 之后）：头参考点回写——头骨实体的
/// 世界位写进该成员全部材质槽。源每帧写、这里同节奏；动画在
/// Propagate 之前推进，读到的是当帧姿势。
pub fn update_head(
    mut materials: ResMut<Assets<CharacterMaterial>>,
    registry: Option<Res<crate::npc::Registry>>,
    npcs: Query<(Entity, &CharacterUnitId, &ToonMaterials, Option<&crate::talk::TalkHold>, Option<&crate::player::PlayerControlled>)>,
    globals: Query<&GlobalTransform>,
) {
    // Preserve Query::find's first matching NPC, including a first match whose
    // transforms are unavailable. Registry order is applied below, not map order.
    let mut neighbor_positions: HashMap<u32, Option<(Vec3, Vec3)>> = HashMap::new();
    for (entity, unit, toon, _, controlled) in &npcs {
        if controlled.is_some() {
            continue;
        }
        neighbor_positions.entry(unit.0).or_insert_with(|| {
            Some((
                globals.get(toon.hips).ok()?.translation(),
                globals.get(entity).ok()?.translation(),
            ))
        });
    }
    for (entity, unit, toon, talking, controlled) in &npcs {
        let Ok(global) = globals.get(toon.head) else { continue; };
        let position = global.translation();
        if position == Vec3::ZERO {
            panic!("unit {} 的头参考点是零向量：头骨的世界变换没有传播", unit.0);
        }
        // IsNearNPC 先用 hips 判资格，透明值再取名册首个近邻的 root 距离。
        // 当前执行器无拍照模式和家具动作播放态；TalkHold 对应正在对话。
        let mut alpha = 1.0;
        if let (None, Some(registry), Ok(hips), Ok(root)) = (controlled, registry.as_deref(), globals.get(toon.hips), globals.get(entity)) {
            let others: Vec<_> = registry.character_unit_ids.iter()
                .filter(|id| **id != unit.0)
                .filter_map(|id| neighbor_positions.get(id).copied().flatten())
                .map(|(other_hips, other_root)| moly_law::objective::overlap::DitherNeighbor {
                    hips_distance: other_hips.distance(hips.translation()),
                    root_distance: other_root.distance(root.translation()),
                }).collect();
            alpha = moly_law::objective::overlap::npc_dither_alpha(talking.is_some(), false, false, &others);
        }
        let head_position = [position.x, position.y, position.z, 0.0];
        let use_dither = if moly_law::objective::overlap::use_dither(alpha) { 1.0_f32 } else { 0.0_f32 };
        for (_, handle) in &toon.slots {
            let changed = materials.get(handle).is_some_and(|material| {
                material.params.head_position.map(f32::to_bits) != head_position.map(f32::to_bits)
                    || material.params.dither_alpha.to_bits() != alpha.to_bits()
                    || material.params.use_dither.to_bits() != use_dither.to_bits()
            });
            if !changed {
                continue;
            }
            if let Some(material) = materials.get_mut(handle) {
                material.params.head_position = head_position;
                material.params.dither_alpha = alpha;
                material.params.use_dither = use_dither;
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

/// 角色材质插件：材质管线 + 全局量桥（含渲染侧 buffer）。
/// `.json` 装载器只在 wasm 注册：native 侧站点插件先于本插件把同一对
/// 注册进了 AssetServer，重复注册打 WARN；wasm 没有站点插件，雾档案
/// 的装载器由本插件自带。
pub struct CharacterMaterialPlugin;

impl Plugin for CharacterMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<CharacterMaterial>::default(),
            ExtractResourcePlugin::<CharacterEnv>::default(),
        ));
        #[cfg(target_arch = "wasm32")]
        app.init_asset::<MolyJson>()
            .init_asset_loader::<MolyJsonLoader>();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, create_env_buffer)
                // Prepare 链在 ExtractCommands 之后：抽取资源用 Commands 落
                // 渲染世界，无序挂载会读到不存在的那一帧。
                .add_systems(Render, write_env_buffer.in_set(RenderSystems::Prepare));
        }
        bevy::asset::embedded_asset!(app, "shaders/character_material.wgsl");
    }
}
