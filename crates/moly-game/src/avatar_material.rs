//! 玩家观众 avatar 的材质：`Mysekai/Avatar` 程序的 Bevy Material。
//!
//! 式子全在 moly-law 的 shading::avatar（纯函数 + 逐值判据），本模块只做
//! 装配：把穿戴集（`avatar_wear` 面板解析出的皮肤包/调色/饰件贴图）摊进
//! uniform 槽，换到玩家 avatar 合成体的网格实体上。WGSL 在
//! `shaders/avatar_material.wgsl`，逐式镜像律。
//!
//! **这是「接错」的修复，不是「没接」。** 真源玩家 avatar 装配链
//! （`CreateMysekaiMaterialAsync`）运行时 `new Material(Mysekai/Avatar)` 现建，
//! **把模型包里序列化的那份材质整份丢弃**——而那份正是装载器导出成 glb
//! 带出来的 PBR。本模块把玩家 avatar 的材质从那份 PBR 换成真源那条管线。
//! 皮肤贴图不再写死默认包：`_SkinTex` 由穿戴集面板的解析链决定
//! （coordinate.costume ?? costume ?? "default"——服装是纯贴图换肤），
//! `_SkinColor` 由 skinColor 列决定（玩家链不读 coordinate 的色）。
//!
//! **与名册的 `Mysekai/Character` 是两份不同的程序**（16 属性 vs 20，只共有
//! `_DitherAlpha`/`_groupDither`；pass 集也不同）。**不照 character_material
//! 的 shader 抄**——它没有 face/body 遮罩、球面距离场、眉毛穿透那一族。
//! 但换装链复用同一形状：建计划 → 贴图到齐换装（`remove StandardMaterial`
//! 再 `insert AvatarMaterial`）。
//!
//! ⛔ **顶点 UV0.z（part-index）是数据，不是绘制结构。** 真源合并网格
//! （`AvatarUtility.CombineMesh`）把每个 part 的 part-index 烘进它的 UV0.z，
//! 合并出的单网格单材质靠它分派贴图。本仓的合成体 glb **只有 2 分量 UV**
//! ——那是提取缺口（`CombineMesh` 没在提取侧把 part-index 烘进 UV0.z），
//! 不是消费缺口。⇒ 本模块**缺 z 分量时响亮拒绝**，**不拆绘制**（拆成多绘制
//! 是另一个结构，真源是单网格单材质靠 UV0.z 分派）；着色器按「UV0.z 会在」写。
//!
//! Dither: the runtime material creation (`AvatarUtility`) enables the
//! keyword `_USE_DITHER`, and the dither writer (`SetDitherAlpha`) switches
//! `_ENABLE_DITHER` / `_DISABLE_DITHER` and the ints `_UseDither` /
//! `_DisableDither` besides writing `_DitherAlpha`. The program's four records
//! carry only the keyword `_USE_ALPHA_CLIP` and no dither toggle property, so
//! none of those switches selects anything: the bayer discard runs on every
//! fragment with `_DitherAlpha` as its threshold, which is what this shader
//! does. The records read are an earlier client build's shader bundle; the
//! current build's was not read. The writers of `_DitherAlpha` are ported in
//! [`player_dither`].

use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;

use crate::avatar_wear::{AvatarWear, WearPart};
use crate::player::PlayerControlled;
use moly_law::shading::avatar as law;

/// The player's audience body (the avatar model): the swap below recolours
/// only this body, never an SD character or a tool.
#[derive(Component)]
pub struct AudienceBody;

/// 晴天现象的方向光色：源值纯白、混合因子 1（与名册一族同一份现象配置）。
const SUNNY_LIGHT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// 材质参数 uniform 的槽数与字节数。槽序是本文件与
/// `shaders/avatar_material.wgsl` 里 `AvatarParams` 结构体之间的契约，两边同改。
pub const PARAMS_SLOTS: usize = 8;
pub const PARAMS_BYTES: usize = PARAMS_SLOTS * 16;

/// 每条 Unity 属性一个槽位；默认形象（四列 null + penlight 不挂）的值由
/// [`law::default_material`] 给出，penlight 七属性停默认。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarParams {
    pub skin_color: [f32; 4],
    /// x: `_Alpha` · y: `_DitherAlpha` · z: `_EnablePenlightLighting`。
    pub alpha_dither_gate: [f32; 4],
    /// x: `_LeftPenlightActive`。
    pub left_active: [f32; 4],
    /// x: `_RightPenlightActive`。
    pub right_active: [f32; 4],
    pub left_color: [f32; 4],
    pub right_color: [f32; 4],
    pub left_param: [f32; 4],
    pub right_param: [f32; 4],
}

impl AvatarParams {
    pub(crate) fn from_material(m: &law::AvatarMaterial) -> Self {
        AvatarParams {
            skin_color: m.skin_color,
            alpha_dither_gate: [m.alpha, m.dither_alpha, m.enable_penlight_lighting, 0.0],
            left_active: [m.left_penlight_active, 0.0, 0.0, 0.0],
            right_active: [m.right_penlight_active, 0.0, 0.0, 0.0],
            left_color: m.left_penlight_color,
            right_color: m.right_penlight_color,
            left_param: m.left_penlight_param,
            right_param: m.right_penlight_param,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        for slot in [
            self.skin_color,
            self.alpha_dither_gate,
            self.left_active,
            self.right_active,
            self.left_color,
            self.right_color,
            self.left_param,
            self.right_param,
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), PARAMS_BYTES);
        bytes
    }
}

/// avatar toon 材质资产。
#[derive(Debug, Clone, TypePath, Asset)]
pub struct AvatarMaterial {
    pub params: AvatarParams,
    /// `_SkinTex`（身体基础图，必有）。
    pub skin_tex: Handle<Image>,
    /// `_AccessoryTex`（默认形象不挂饰品 ⇒ `None`，绑白色 fallback）。
    pub accessory_tex: Option<Handle<Image>>,
    /// penlight 两张（默认不挂 ⇒ `None`）。
    pub penlight_body_tex: Option<Handle<Image>>,
    pub penlight_light_tex: Option<Handle<Image>>,
}

impl AsBindGroup for AvatarMaterial {
    type Data = ();
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<AvatarEnvGpuBuffer>,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
        bevy::ecs::system::lifetimeless::SRes<bevy::render::texture::FallbackImage>,
    );

    fn label() -> &'static str {
        "avatar_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        render_device: &RenderDevice,
        (env_buffer, images, fallback): &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let skin = images
            .get(&self.skin_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let view_of = |handle: &Option<Handle<Image>>| -> Result<&bevy::render::texture::GpuImage, AsBindGroupError> {
            match handle {
                Some(h) => images.get(h).ok_or(AsBindGroupError::RetryNextUpdate),
                None => Ok(&fallback.d2),
            }
        };
        let accessory = view_of(&self.accessory_tex)?;
        let penlight_body = view_of(&self.penlight_body_tex)?;
        let penlight_light = view_of(&self.penlight_light_tex)?;
        // 贴图不滚动、uv 不越 [0,1]；采样态不在语料记录里，取引擎默认的钳边
        // 与三线性——与 GpuImage 自带 sampler 的过滤一致，只有寻址模式不同。
        let clamp = render_device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..Default::default()
        });
        let tex = |binding: u32, view: &bevy::render::texture::GpuImage| {
            (
                binding,
                OwnedBindingResource::TextureView(TextureViewDimension::D2, view.texture_view.clone()),
            )
        };
        let samp = |binding: u32| {
            (
                binding,
                OwnedBindingResource::Sampler(SamplerBindingType::Filtering, clamp.clone()),
            )
        };
        let bindings = BindingResources(vec![
            (0, OwnedBindingResource::Data(OwnedData(self.params.bytes()))),
            (1, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
            tex(2, skin),
            samp(3),
            tex(4, accessory),
            samp(5),
            tex(6, penlight_body),
            samp(7),
            tex(8, penlight_light),
            samp(9),
        ]);
        Ok(UnpreparedBindGroup { bindings })
    }

    fn bind_group_data(&self) -> Self::Data {
        // Data = ()：无 per-材质特化键（Avatar 只有一个 Base 变体在册）。
        // 显式写空 tuple 而不靠 `()` 的 unit 表达式歧义。
        ()
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
            texture(8),
            sampler(9),
        ]
    }
}

impl Material for AvatarMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_game/shaders/avatar_material.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_game/shaders/avatar_material.wgsl".into()
    }

    /// 本单只交付 Base 片元；阴影与深度消费方不在范围里。
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

// ---- 全局量桥 ----

/// 全局量表的 GPU 布局：4 个 vec4 槽、64 字节。槽序是本文件与
/// `shaders/avatar_material.wgsl` 里 `AvatarEnv` 结构体之间的契约，两边同改。
pub const ENV_SLOTS: usize = 4;
pub const ENV_BYTES: usize = ENV_SLOTS * 16;

/// 一帧的 avatar 全局量（`_GlobalCharacterDirectionalLightColor` /
/// `_MysekaiScreenParams` / `_GlobalMipBias` / 投影参数）。
#[derive(Debug, Clone, Resource, ExtractResource)]
pub struct AvatarEnv {
    pub light_color: [f32; 4],
    pub screen_params: [f32; 4],
    pub mip_bias: [f32; 2],
    pub projection_params: [f32; 4],
}

impl AvatarEnv {
    /// 中性初值：光色纯白、混合因子 1（消费式子精确化简为无暗化）、屏幕
    /// 参数与站点相机一致以免除零。
    pub fn neutral() -> Self {
        AvatarEnv {
            light_color: SUNNY_LIGHT_COLOR,
            screen_params: [1.0, 1.0, 2.0, 2.0],
            mip_bias: [0.0, 0.0],
            projection_params: [1.0, 1.0, 4000.0, 0.00025],
        }
    }

    pub fn gpu_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ENV_BYTES);
        for slot in [
            self.light_color,
            self.screen_params,
            [self.mip_bias[0], self.mip_bias[1], 0.0, 0.0],
            self.projection_params,
        ] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), ENV_BYTES);
        bytes
    }
}

/// 渲染侧的全局量 buffer；一帧一份，全部 avatar 材质共用。
#[derive(Resource)]
pub struct AvatarEnvGpuBuffer {
    pub buffer: Buffer,
}

fn create_env_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("avatar_env"),
        contents: &AvatarEnv::neutral().gpu_bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(AvatarEnvGpuBuffer { buffer });
}

fn write_env_buffer(env: Res<AvatarEnv>, buffer: Res<AvatarEnvGpuBuffer>, queue: Res<RenderQueue>) {
    queue.write_buffer(&buffer.buffer, 0, &env.gpu_bytes());
}

/// Startup：中性表落位。
pub fn insert_neutral(mut commands: Commands) {
    commands.insert_resource(AvatarEnv::neutral());
}

/// PostUpdate：相机态刷新——屏幕参数与投影参数（bayer 表消费屏幕参数）。
pub fn write_frame_state(
    mut env: ResMut<AvatarEnv>,
    cameras: Query<(&Projection, &Camera), With<Camera3d>>,
) {
    let Ok((projection, camera)) = cameras.single() else {
        return;
    };
    let (near, far) = match projection {
        Projection::Perspective(p) => (p.near, p.far),
        Projection::Orthographic(o) => (o.near, o.far),
        Projection::Custom(_) => panic!("avatar 全局量不认识自定义投影"),
    };
    env.projection_params = [1.0, near, far, 1.0 / far];
    if let Some(size) = camera.physical_viewport_size() {
        let (w, h) = (size.x as f32, size.y as f32);
        env.screen_params = [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h];
    }
}

// ---- 换装 ----

/// 换装完成闩（永不撤；防换装撤掉计划后重进）。
#[derive(Component)]
pub struct AvatarToon;

/// Update：avatar 合成体挂载后建材质换装。穿戴集（[`AvatarWear`]）供给
/// `_SkinTex`/`_SkinColor`/`_AccessoryTex`（服装 = 纯贴图换肤，皮肤包由
/// 面板解析链决定）。挂件实体（`WearPart`）不扫——它们的材质各归各
/// （饰件槽 1 / 荧光棒自己的 PBR），且件网格没有 part-index 通道，被
/// 扫到会在判据处响亮拒绝。缺 part-index 通道（UV0.z 载体）时响亮
/// panic——那是提取缺口（合成器未烘 part-index），不静默取默认、不拆
/// 绘制。一次性：换装完成插 [`AvatarToon`] 闩后不再进。
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn plan_and_swap(
    mut commands: Commands,
    server: Res<AssetServer>,
    wear: Res<AvatarWear>,
    mut materials: ResMut<Assets<AvatarMaterial>>,
    mesh_assets: Res<Assets<Mesh>>,
    players: Query<
        (Entity, &Children),
        (With<PlayerControlled>, With<AudienceBody>, Without<AvatarToon>),
    >,
    meshes_3d: Query<&Mesh3d>,
    children: Query<&Children>,
    wear_parts: Query<(), With<WearPart>>,
    standard: Query<&MeshMaterial3d<StandardMaterial>>,
    standard_materials: Res<Assets<StandardMaterial>>,
) {
    for (player, _kids) in &players {
        // 收集玩家子树里全部身体网格实体（挂件实体除外）。
        let mut stack = vec![player];
        let mut mesh_entities = Vec::new();
        while let Some(entity) = stack.pop() {
            if meshes_3d.get(entity).is_ok() && wear_parts.get(entity).is_err() {
                mesh_entities.push(entity);
            }
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    stack.push(kid);
                }
            }
        }
        if mesh_entities.is_empty() {
            continue; // audience scene has not expanded yet
        }
        // part-index 通道（UV0.z 载体）判据：真源合并网格靠它分派贴图。
        for entity in &mesh_entities {
            let mesh = mesh_assets
                .get(&meshes_3d.get(*entity).expect("网格实体有 Mesh3d").0)
                .expect("网格实体引用的 Mesh 不在 Assets 里");
            assert_part_index(mesh);
        }
        // 等皮肤贴图到齐再换（避免闪回默认材质）；它是唯一必须等待的。
        // 失败即响亮拒绝（皮肤包具名，不静默取默认）。
        match server.load_state(&wear.skin_tex) {
            LoadState::Failed(err) => panic!(
                "avatar 皮肤贴图装载失败（皮肤包 {}，未提取件具名拒绝）：{err:?}",
                wear.skin_bundle
            ),
            LoadState::Loaded => {}
            _ => continue, // 未到齐，下一帧再试
        }
        // 材质值：默认形象为底，穿戴集覆写调色与贴图（真源运行时现建材质，
        // 把模型包里序列化的那份整份丢弃）。
        let mut law_material = law::default_material();
        law_material.skin_color = wear.skin_color;
        let material = AvatarMaterial {
            params: AvatarParams::from_material(&law_material),
            skin_tex: wear.skin_tex.clone(),
            accessory_tex: wear.accessory_tex.clone(),
            penlight_body_tex: None,
            penlight_light_tex: None,
        };
        let handle = materials.add(material);
        let entity_count = mesh_entities.len();
        // 换肤生效行：换装前后的材质贴图引用对比（前 = 装载器导出的 PBR
        // 基础色贴图有无；后 = Mysekai/Avatar 的皮肤包贴图与调色）。
        let old_textured = mesh_entities.iter().any(|entity| {
            standard
                .get(*entity)
                .ok()
                .and_then(|m| standard_materials.get(&m.0))
                .is_some_and(|m| m.base_color_texture.is_some())
        });
        for entity in mesh_entities {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle.clone()));
        }
        commands.entity(player).insert(AvatarToon);
        let stored = materials
            .get(&handle)
            .expect("the material was added this frame")
            .params;
        info!(
            "[player] avatar 换肤：材质贴图引用 PBR（基础色贴图 {}） -> Mysekai/Avatar（_SkinTex {}，_SkinColor {}），{entity_count} 个网格实体；材质值 _SkinColor {:?} _Alpha {} _DitherAlpha {} _EnablePenlightLighting {} _LeftPenlightActive {} _RightPenlightActive {}",
            if old_textured { "有" } else { "无" },
            wear.skin_bundle,
            wear.skin_color_code,
            stored.skin_color,
            stored.alpha_dither_gate[0],
            stored.alpha_dither_gate[1],
            stored.alpha_dither_gate[2],
            stored.left_active[0],
            stored.right_active[0],
        );
    }
}

/// 缺 part-index 通道即响亮拒绝（fail-closed）：真源靠顶点 part 序号分派
/// 贴图，提取侧把它烘成第二套 UV（TEXCOORD_1，消费端读作 UV_1）的 x 分量。
/// 本仓 glb 缺 TEXCOORD_1 ⇒ 提取侧未烘，不静默取默认（那样整片喂成身体
/// 贴图），不拆绘制（真源是单网格单材质）。
fn assert_part_index(mesh: &Mesh) {
    // 载体是第二套 UV（TEXCOORD_1 → Bevy 的 ATTRIBUTE_UV_1），不是 UV0 的
    // 第三分量：Bevy 两套 UV 顶点属性都是 Float32x2，3 分量 UV 会被装载器
    // 整段丢弃；第二套 UV 的 x 是唯一既装载得进又载得了值的形状（提取侧
    // 注释记了这个取舍，消费端照它读）。
    let has_part_channel = mesh.attribute(Mesh::ATTRIBUTE_UV_1).is_some();
    if !has_part_channel {
        panic!(
            "玩家 avatar 合成体缺 part-index 通道：真源靠顶点 part 序号分派贴图，\
             提取侧把它烘成第二套 UV（TEXCOORD_1）的 x 分量，而本 glb 没有 TEXCOORD_1——\
             提取侧合成器未烘 part-index，需在提取侧补齐后再接 Mysekai/Avatar 的槽分派"
        );
    }
}

// ---------------------------------------------------------------------------
// The player avatar's dither (`PlayerAvatar.UpdateDitherValue`)
// ---------------------------------------------------------------------------

/// The talk camera states' `_ditherRange` (degrees).
const TALK_DITHER_RANGE: f32 = 45.0;
/// `NormalCameraState.FadeInPlayerIfNeeded`: after the first-person state
/// the player fades in from 0 by `DOTween.To(.., 1, 0.2)` on the default ease.
const FADE_IN_SECONDS: f32 = 0.2;
/// `Vector3.kEpsilonNormalSqrt`, the angle's zero-length guard.
const ANGLE_EPSILON: f32 = 1e-15;
/// `Mathf.Rad2Deg`.
const RAD_TO_DEG: f32 = 57.295_78;

/// `TalkCameraState.SetPlayerDitherValue` (the SomeCharacterTalk state's is
/// the same code) on one frame: unless the camera is within 1 of its minimum
/// distance the player is fully shown. Otherwise, for each target after the
/// first, the signed planar angle at that target between the first target
/// and the camera; within `range` of ±180 degrees (the target stands between
/// the first target and the camera) it gives `1 - (1 - (1 - t)^5)` with
/// `t = (range - (180 - |angle|)) / range`, and the player takes the smallest.
/// Operation order and precision follow the compiled method: the angle and
/// the power are taken in double, the rest in single precision.
pub(crate) fn talk_player_dither(
    distance: f32,
    min_distance: f32,
    targets: &[Vec3],
    camera: Vec3,
    range: f32,
) -> f32 {
    if distance - min_distance > 1.0 {
        return 1.0;
    }
    let Some((first, others)) = targets.split_first() else {
        return 1.0;
    };
    let mut value = 1.0f32;
    for other in others {
        let (ax, az) = (first.x - other.x, first.z - other.z);
        let (bx, bz) = (camera.x - other.x, camera.z - other.z);
        let denominator = ((az * az + (ax * ax + 0.0)) * (bz * bz + (bx * bx + 0.0))).sqrt();
        let mut angle = 0.0f32;
        if !(denominator < ANGLE_EPSILON) {
            let cosine = (az * bz + (ax * bx + 0.0)) / denominator;
            let clamped = if cosine >= -1.0 {
                cosine.min(1.0)
            } else {
                -1.0
            };
            angle = ((clamped as f64).acos() as f32) * RAD_TO_DEG;
        }
        let signed = if az * bx - ax * bz < 0.0 {
            -angle
        } else {
            angle
        };
        let between = (signed >= -180.0 && signed < range + -180.0)
            || (signed <= 180.0 && signed > 180.0 - range);
        if between {
            let t = (range - (180.0 - angle.abs())) / range;
            let power = ((1.0 - t) as f64).powf(5.0);
            let candidate = 1.0 - (1.0 - power) as f32;
            value = if value < candidate { value } else { candidate };
        }
    }
    if value >= 1.0 {
        1.0
    } else {
        value
    }
}

/// The dither driver's state across frames.
pub(crate) struct PlayerDither {
    last: Option<crate::camera::CameraStateType>,
    value: f32,
    fade: Option<moly_law::ui::dotween::FloatTween>,
    /// Whether the talk framing's value has been logged in this state.
    talk_logged: bool,
}

impl Default for PlayerDither {
    fn default() -> Self {
        Self {
            last: None,
            value: 1.0,
            fade: None,
            talk_logged: false,
        }
    }
}

fn is_talk_state(state: crate::camera::CameraStateType) -> bool {
    use crate::camera::CameraStateType as S;
    matches!(state, S::Talk | S::SomeCharacterTalk)
}

/// PostUpdate, after the transforms are propagated: the camera states'
/// writes of the player's dither value, into the avatar material's
/// `_DitherAlpha` (`MysekaiMaterialExtension.SetDitherAlpha`; the avatar
/// program has neither the dither keywords nor the dither toggles that
/// method also sets, so the alpha is the only part it reads).
/// * Normal: entered from the first-person state, the value goes to 0 and
///   tweens to 1 over 0.2 s; leaving Normal kills that tween and writes 1.
/// * Talk and SomeCharacterTalk: from the talk camera's framing on, every
///   frame [`talk_player_dither`] on the targets it frames (the talking
///   NPCs' hips, bound fixtures, then the player's view) and the camera's
///   position; leaving writes 1.
/// The screenshot capture's writes have no counterpart (no photo mode).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn player_dither(
    time: Res<Time>,
    state: Res<crate::camera::FieldCameraState>,
    model: Option<Res<crate::camera::FieldCameraModel>>,
    talk_camera: Option<Res<crate::talk_camera::TalkCamera>>,
    talk: Option<Res<crate::talk::ActiveTalk>>,
    session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    skeletons: Query<&crate::character_material::ToonMaterials>,
    transforms: Query<&GlobalTransform>,
    players: Query<&GlobalTransform, With<crate::character::AvatarRoot>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    bodies: Query<&MeshMaterial3d<AvatarMaterial>>,
    mut materials: ResMut<Assets<AvatarMaterial>>,
    mut local: Local<PlayerDither>,
) {
    use crate::camera::CameraStateType as S;
    let current = state.0;
    if local.last != Some(current) {
        match local.last {
            Some(S::Normal) => {
                local.fade = None;
                local.value = 1.0;
            }
            Some(previous) if is_talk_state(previous) => local.value = 1.0,
            _ => {}
        }
        if current == S::Normal && local.last == Some(S::Fps) {
            local.value = 0.0;
            local.fade = Some(moly_law::ui::dotween::FloatTween::new(
                1.0,
                FADE_IN_SECONDS,
                moly_law::ui::dotween::Ease::OutQuad,
            ));
            info!("[player-dither] Normal after first person: fade in over {FADE_IN_SECONDS}s");
        }
        local.last = Some(current);
        local.talk_logged = false;
    } else {
        let this = &mut *local;
        if let Some(fade) = this.fade.as_mut() {
            let next = fade.update(this.value, time.delta_secs());
            let complete = fade.is_complete();
            if let Some(next) = next {
                this.value = next;
            }
            if complete {
                this.fade = None;
            }
        }
    }
    if is_talk_state(current) && talk_camera.is_some() {
        let hip = |entity: Entity| {
            let skeleton = skeletons.get(entity).ok()?;
            transforms
                .get(skeleton.hips_entity())
                .ok()
                .map(GlobalTransform::translation)
        };
        let mut targets = Vec::new();
        if let Some(talk) = talk.as_deref() {
            targets.extend(
                talk.participants()
                    .iter()
                    .filter_map(|&(_, entity)| hip(entity)),
            );
            targets.extend(talk.fixture_instances().iter().filter_map(|&(_, entity)| {
                transforms
                    .get(entity)
                    .ok()
                    .map(GlobalTransform::translation)
            }));
        } else if let Some(session) = session.as_deref() {
            targets.extend(hip(session.npc_entity()));
        }
        targets.extend(players.iter().next().map(GlobalTransform::translation));
        if let (Some(model), Ok(camera)) = (model.as_deref(), cameras.single()) {
            let previous = local.value;
            local.value = talk_player_dither(
                model.distance,
                model.min_distance,
                &targets,
                camera.translation(),
                TALK_DITHER_RANGE,
            );
            if !local.talk_logged || (previous < 1.0) != (local.value < 1.0) {
                local.talk_logged = true;
                info!(
                    "[player-dither] {current:?} framing: {} targets, distance {:.3} over minimum {:.3}, dither {:.4}",
                    targets.len(),
                    model.distance,
                    model.min_distance,
                    local.value
                );
            }
        }
    }
    let value = local.value;
    let mut ids: Vec<AssetId<AvatarMaterial>> =
        bodies.iter().map(|material| material.0.id()).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        let stale = materials
            .get(id)
            .is_some_and(|material| material.params.alpha_dither_gate[1] != value);
        if stale {
            if let Some(material) = materials.get_mut(id) {
                material.params.alpha_dither_gate[1] = value;
            }
        }
    }
}

/// avatar 材质插件：材质管线 + 全局量桥（含渲染侧 buffer）+ 玩家身体的
/// 换装链（穿戴集面板 → 身体网格换 `Mysekai/Avatar` → 饰件/荧光棒）。
pub struct AvatarMaterialPlugin;

impl Plugin for AvatarMaterialPlugin {
    fn build(&self, app: &mut App) {
        crate::gpu_image_release::prepare_after_images::<AvatarMaterial>(app);
        // The player domain's own systems ride on this registration.
        app.add_plugins(crate::player::PlayerPlugin);
        app.add_plugins((
            MaterialPlugin::<AvatarMaterial>::default(),
            ExtractResourcePlugin::<AvatarEnv>::default(),
        ));
        app.add_systems(Startup, (insert_neutral, crate::avatar_wear::load))
            .add_systems(
                Update,
                (plan_and_swap, crate::avatar_wear::attach_accessory)
                    .chain()
                    .after(crate::player_avatar::body::wire),
            )
            .add_systems(PostUpdate, write_frame_state);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, create_env_buffer)
                .add_systems(Render, write_env_buffer.in_set(RenderSystems::Prepare));
        }
        bevy::asset::embedded_asset!(app, "shaders/avatar_material.wgsl");
    }
}
