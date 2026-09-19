//! Site particle lifecycle and shared UberUnlit material.
use bevy::asset::uuid::Uuid;
use bevy::asset::LoadState;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;
use moly_assets::sidecar::{MolyJson, ParticleRenderer, ParticleSystem, SiteSidecar};
use moly_law::material::MaterialSlot;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::{Effects, EmissionState, EmitterParams, RotationOverLifetime, LimitVelocity};
use std::collections::HashMap;
use std::marker::PhantomData;

use crate::billboard::{self, Alignment, SizeClamp};
use crate::env::SiteEnvGpuBuffer;
use crate::particle_runtime::{Runtime, Rng, Context, EffectKind, simulate, PREWARM_STEP};
use crate::site::{SiteActive, SiteRoot, SiteScenesReady};
use crate::site_material::SiteSidecarAsset;

/// 源族的 shader 名。
const SHADER_NAME: &str = "Mysekai/Effect/UberUnlit";

/// 零缩放的四边形没有面积，画不出来；尺寸下限。
const MIN_PARTICLE_SIZE: f32 = 0.0001;

/// 出生抽签的确定性随机种子（固定值换可复算）。
const RNG_SEED: u64 = 0x7562_6572_0001_0124;

/// 全局 mip 偏置：引擎的**全局**量，不是材质属性（材质记录里恰 0 条
/// 带它，而编译产物里每个采样点都读它）。动态分辨率关时它是 0，本管线
/// 没有动态分辨率 ⇒ 0。喂进 uniform 而不是从着色器里删掉，是为了让
/// 后来接动态分辨率的人有地方写。
const GLOBAL_MIP_BIAS: f32 = 0.0;

/// 着色程序的稳定句柄。
pub(crate) const UBER_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x4b7e_1c92_6a03_4d18_9f52_0e7a_c1d4_8b36),
    PhantomData,
);

// ---- 材质 ----

/// 主颜色 pass 的片元链输入。
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct UberT1Params {
    /// `_BaseMap_ST`。
    pub base_st: Vec4,
    /// `_TintColor`（HDR 分量是常态，源程序不钳制）。
    pub tint_colour: Vec4,
    /// x = `_TintBlendRate` · y = 全局 mip 偏置 · z = `_TranceparencyByLuminanceEnabled`
    /// · w = `_PhenomenaLightEnabled`。后两个在源程序里就是 uniform 分支的
    /// 判别量（`lessThan(0.5, …)`），这里照那个形状喂进来，不做管线特化。
    pub scalars: Vec4,
    /// 亮度键控透明的标量：x = `_LuminanceTransparencyProgress`
    /// · y = `_LuminanceTransparencySharpness` · z = `_InverseLuminanceTransparency`
    /// · w 留空。
    ///
    /// 源程序里 progress 与 sharpness 各自还叠一个逐粒子自定义流
    /// （`coord = 分量号 × 10 + 来源号`，来源 0 取常量零向量），本站材质集上
    /// 两个 coord 恰全为 0（2121/2121）⇒ 叠加项恒为 0，只剩这两个标量本身。
    /// coord ≠ 0 的材质仍由门拦下，不会静默走到这里。
    pub luminance: Vec4,
    /// 逐粒子流选择器。x = `_TintBlendRateCoord`；其余分量留给同族的其它
    /// coord，接线方式相同（门未放开的仍由门拦下）。
    pub coords: Vec4,
}

impl UberT1Params {
    /// 手写绑定组用的字节序；槽序与 `shaders/uber_particle.wgsl` 的
    /// `UberT1Params` 是契约，也与 `fixture_emission` 的对象块前四槽一致。
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(80);
        for slot in [self.base_st, self.tint_colour, self.scalars, self.luminance, self.coords] {
            for value in slot.to_array() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }
}

#[derive(Component, Clone, Copy)]
pub(crate) struct ParticleEmission {
    pub source_state: Option<moly_assets::material_passes::SourceRenderState>,
    pub render_queue: i32,
    pub params: UberT1Params,
    pub colour: Vec4,
    pub intensity: f32,
    pub colour_type: f32,
    pub area: bool,
    pub tint_area: bool,
    /// 原版粒子族：材质色平乘（见 wgsl 的 `UBER_PLAIN_COLOUR`）。
    pub plain_colour: bool,
    pub cull: CullArm,
    pub blend: BlendArm,
}

/// 剔除档（材质记录的 `_Cull`，Unity `CullMode`：Off=0 · Front=1 · Back=2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CullArm {
    /// 0：双面，单份三角形索引，管线不剔。
    Off,
    /// 1：剔正面。
    Front,
    /// 2：剔背面。
    Back,
}

/// 混合档（材质记录的 `_BlendDst`；`_BlendSrc` 在全部记录上恒 5 = SrcAlpha）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendArm {
    /// 10 = OneMinusSrcAlpha。
    AlphaBlend,
    /// 1 = One（加法）。
    Additive,
}

/// 管线特化键：染色支由**关键字**决定（不是由给混合率喂 0 决定），
/// 剔除档与源混合因子都进管线状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UberT1Key {
    /// 关键字集里有「染色作用于整片」或「染色作用于边缘」之一。
    pub tint_area: bool,
    /// 材质族是原版粒子族 ⇒ 材质色平乘。与 `tint_area` 互斥：
    /// 两族的属性面不相交（该族无 `_TINT_AREA_*`，染色族无 `_Color`）。
    pub plain_colour: bool,
    pub cull: CullArm,
    pub blend: BlendArm,
}

#[derive(Asset, TypePath, Debug, Clone)]
pub struct UberParticleMaterial {
    params: UberT1Params,
    pub(crate) base_map: Handle<Image>,
    tint_area: bool,
    plain_colour: bool,
    cull: CullArm,
    blend: BlendArm,
}

impl AsBindGroup for UberParticleMaterial {
    type Data = UberT1Key;
    type Param = (SRes<SiteEnvGpuBuffer>, SRes<RenderAssets<GpuImage>>);

    fn label() -> &'static str {
        "uber_particle_material"
    }

    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        (env_buffer, images): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let base = images
            .get(&self.base_map)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        let bindings = BindingResources(vec![
            (0, OwnedBindingResource::Data(OwnedData(self.params.bytes()))),
            (
                1,
                OwnedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    base.texture_view.clone(),
                ),
            ),
            (
                2,
                OwnedBindingResource::Sampler(SamplerBindingType::Filtering, base.sampler.clone()),
            ),
            // binding 3：站点全局量，与家具/站点材质共用同一个 buffer。
            // `_GlobalPhenomenaDirectionalLightColor` 是逐帧全局量，不能烘进
            // 材质——烘进去天气一换材质就是陈旧值，而材质不会因此重建。
            (3, OwnedBindingResource::Buffer(env_buffer.buffer.clone())),
        ]);
        Ok(UnpreparedBindGroup { bindings })
    }

    fn bind_group_data(&self) -> Self::Data {
        UberT1Key::from(self)
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }
}

/// 天气链共用这一份材质与管线特化（同一条 T1 片元链，不另起第二份
/// 绘制路径）：字段保持私有，这个构造器是它的唯一共用口。
impl UberParticleMaterial {
    pub(crate) fn new(
        params: UberT1Params,
        base_map: Handle<Image>,
        tint_area: bool,
        plain_colour: bool,
        cull: CullArm,
        blend: BlendArm,
    ) -> Self {
        Self {
            params,
            base_map,
            tint_area,
            plain_colour,
            cull,
            blend,
        }
    }
}

impl From<&UberParticleMaterial> for UberT1Key {
    fn from(material: &UberParticleMaterial) -> Self {
        UberT1Key {
            tint_area: material.tint_area,
            plain_colour: material.plain_colour,
            cull: material.cull,
            blend: material.blend,
        }
    }
}

impl Material for UberParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(UBER_SHADER.clone())
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(UBER_SHADER.clone())
    }

    /// 两档都进入透明队列且不写深度；实际 RGB 混合因子在 specialize
    /// 显式装配。通用 Add 档依赖标准材质着色器先预乘 RGB 并清零 alpha，
    /// 本族写的是未预乘颜色，不能只用该档位代替源的 SrcAlpha/One。
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// 粒子不进 prepass、不投影（源族的粒子渲染器不作 shadow caster）。
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // This shader does not use Bevy's bindless light-probe array. Reuse that
        // group slot for a PER-VIEW raw-depth alias; material instances remain
        // view-independent. The matching draw command installs the same layout.
        descriptor.layout[1] = crate::weather_depth::raw_depth_layout();
        // 顶点布局显式装配：默认的网格布局不含自定义流那两条，而源程序的
        // 逐粒子选择器要从它们取分量。槽位与 `shaders/uber_particle.wgsl`
        // 的 `UberVertex` 是契约。
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
            crate::billboard::ATTRIBUTE_CUSTOM1.at_shader_location(8),
            crate::billboard::ATTRIBUTE_CUSTOM2.at_shader_location(9),
        ])?];
        if key.bind_group_data.tint_area {
            descriptor
                .vertex
                .shader_defs
                .push("UBER_TINT_AREA_ALL".into());
            if let Some(ref mut fragment) = descriptor.fragment {
                fragment.shader_defs.push("UBER_TINT_AREA_ALL".into());
            }
        }
        if key.bind_group_data.plain_colour {
            descriptor.vertex.shader_defs.push("UBER_PLAIN_COLOUR".into());
            if let Some(ref mut fragment) = descriptor.fragment {
                fragment.shader_defs.push("UBER_PLAIN_COLOUR".into());
            }
        }
        descriptor.primitive.cull_mode = match key.bind_group_data.cull {
            CullArm::Off => None,
            CullArm::Front => Some(bevy::render::render_resource::Face::Front),
            CullArm::Back => Some(bevy::render::render_resource::Face::Back),
        };
        if let Some(ref mut fragment) = descriptor.fragment {
            use bevy::render::render_resource::{
                BlendComponent, BlendFactor, BlendOperation, BlendState,
            };
            let dst_factor = match key.bind_group_data.blend {
                BlendArm::AlphaBlend => BlendFactor::OneMinusSrcAlpha,
                BlendArm::Additive => BlendFactor::One,
            };
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::SrcAlpha,
                        dst_factor,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent::OVER,
                });
            }
        }
        Ok(())
    }
}

/// Material 管线注册：在 `app()` 里 DefaultPlugins 之后调用一次。
pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<UberParticleMaterial>::default());
    crate::weather_depth::install_raw_depth(app);
}

/// Startup：内嵌着色程序。
pub(crate) fn load(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        UBER_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/uber_particle.wgsl"),
            "moly_game/src/shaders/uber_particle.wgsl".to_owned(),
        ),
    );
}

// ---- 判读 ----

/// 一条放行的粒子系统：律侧参数 + 呈现侧输入 + 待装载的贴图。
struct Planned {
    node: String,
    emitter: EmitterParams,
    /// 发射节点实体：世界变换逐帧从它读（局部空间仿真要它）。
    anchor: Entity,
    alignment: Alignment,
    cone_angle: Option<f32>,
    rol: Option<RotationOverLifetime>,
    limit: Option<LimitVelocity>,
    params: UberT1Params,
    tint_area: bool,
    cull: CullArm,
    blend: BlendArm,
    clamp: SizeClamp,
    pivot: [f32; 3],
    texture: Handle<Image>,
    effect: Option<ParticleEmission>,
}

/// 判读结果：放行的计划 + 逐档拒绝盘点。
#[derive(Resource)]
pub(crate) struct UberParticlePlan {
    planned: Vec<Planned>,
    tally: Tally,
}

/// 逐档盘点。**每一格都是「这一档有多少条被挡在外面」**——盘面上看得见
/// 还差什么，是这条通路唯一诚实的进度量。
#[derive(Default, Debug)]
struct Tally {
    /// sidecar 里这一族的粒子系统总数。
    records: usize,
    /// 渲染器关着的。
    renderer_disabled: usize,
    /// 连渲染器记录都没有的。与下面两项一样，判读在 `records` 自增之前就
    /// 退出，所以它们不计入本族记录数——但必须计数，否则这条路径上的拒绝
    /// 既不出现在 `records` 里也不出现在任何桶里，等于静默丢弃。
    no_renderer: usize,
    /// 没有内联材质记录的。
    no_material: usize,
    /// 材质族不是这一族的，按 shader 名计数。
    other_family: Vec<String>,
    /// 发射节点路径在已展开的场景树里对不上（见模块注释：多数是运行时
    /// 实例化的 prefab，不是缺陷）。
    node_unresolved: usize,
    /// 绘制模式不是 Billboard，按模式名计数。
    render_mode: Vec<String>,
    /// 对齐档未实现，按引擎枚举体的档位名计数。
    alignment: Vec<String>,
    /// 发射形状律里没有，按形状名计数。
    shape: Vec<String>,
    /// 缺形状模块（无发射位置来源）。
    no_shape: usize,
    /// 缺 emission 模块。
    no_emission: usize,
    /// 缺 `system` 块（装载层原样存的那块）。
    no_system_block: usize,
    /// 粒子律对单条 `system` 块具名拒绝（带权曲线模式等），按消息计数。
    law_reject: Vec<String>,
    /// 仿真空间不是局部/世界二档之一。
    sim_space: usize,
    /// 缺基础贴图槽。
    no_base_map: usize,
    /// 着色轴门不过，按轴名计数（这些是**未实现**，不是拒绝理由——
    /// 放行但少一步，逐条具名）。
    shading_shortfall: Vec<String>,
    /// 状态档不认（`_BlendDst` / `_Cull` / `_ZTest` 取到族外值）。
    state_arm: Vec<String>,
    /// 放行的条数。
    admitted: usize,
}

/// 绘制实体标记：换站时按它撤（这些实体不是站点树的子节点，站点树撤除
/// 带不走它们）。
#[derive(Component)]
pub struct UberParticleDraw;

/// Update：站点 sidecar 到位、场景树展开完毕 → 逐条判读一次。
pub(crate) fn plan(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<MolyJson>>,
    sidecar: Option<Res<SiteSidecarAsset>>,
    scenes_ready: Option<Res<SiteScenesReady>>,
    active: Option<Res<SiteActive>>,
    planned: Option<Res<UberParticlePlan>>,
    state: Option<Res<UberParticleState>>,
    roots: Query<Entity, With<SiteRoot>>,
    names: Query<&Name>,
    children: Query<&Children>,
    stale: Query<Entity, With<UberParticleDraw>>,
) {
    if planned.is_some() || state.is_some() {
        return;
    }
    let (Some(sidecar), Some(_), Some(active)) = (sidecar, scenes_ready, active) else {
        return;
    };
    if roots.is_empty() {
        return;
    }
    // 上一站留下的绘制实体：计划与状态都撤了才走到这里，此刻撤它们。
    for entity in &stale {
        commands.entity(entity).despawn();
    }
    if let LoadState::Failed(err) = server.load_state(&sidecar.handle) {
        panic!("站点 sidecar 装载失败（粒子链，{}）：{err:?}", active.scene);
    }
    let Some(asset) = json.get(&sidecar.handle) else {
        return;
    };
    let MolyJson::SiteSidecar(doc) = asset else {
        panic!("站点 sidecar 路径装载到了别的资产类型（粒子链）");
    };

    // 场景树的全链路径表（与清扫链、声源挂接共用同一套坐标系：路径
    // 前缀是场景目录名，glTF 主根与它同名）。
    let mut by_path: HashMap<String, Vec<Entity>> = HashMap::new();
    for root in &roots {
        crate::inactive_nodes::collect(root, &mut Vec::new(), &names, &children, &mut by_path);
    }

    let mut tally = Tally::default();
    let mut plans = Vec::new();
    for system in &doc.particles {
        match judge(
            system,
            doc,
            &active.scene,
            &format!("site/scenes/{}", active.scene),
            &by_path,
            &server,
            &mut tally,
        ) {
            Some(plan) => {
                tally.admitted += 1;
                plans.push(plan);
            }
            None => {}
        }
    }
    if tally.records == 0 {
        // 这个站点包里没有这一族的粒子系统：不留资源，也不每帧重扫。
        commands.insert_resource(UberParticlePlan {
            planned: Vec::new(),
            tally,
        });
        return;
    }
    info!(
        "[uber-particle] {} 判读：本族记录 {}；放行 {}；\
         挡下——节点路径未解析 {}（多为运行时实例化的 prefab，见模块注释）· \
         绘制模式 {:?} · 对齐档 {:?} · 发射形状律缺 {:?} · 缺形状模块 {} · \
         缺 emission {} · 缺 system 块 {} · 律拒 {:?} · 仿真空间 {} · 缺基础贴图 {} · 状态档族外 {:?} · \
         渲染器关 {} · 无渲染器 {} · 无材质 {} · 非本族 {:?}；\
         放行但未实现（逐条具名，不静默）：{:?}",
        active.scene,
        tally.records,
        tally.admitted,
        tally.node_unresolved,
        count_names(&tally.render_mode),
        count_names(&tally.alignment),
        count_names(&tally.shape),
        tally.no_shape,
        tally.no_emission,
        tally.no_system_block,
        count_names(&tally.law_reject),
        tally.sim_space,
        tally.no_base_map,
        count_names(&tally.state_arm),
        tally.renderer_disabled,
        tally.no_renderer,
        tally.no_material,
        count_names(&tally.other_family),
        count_names(&tally.shading_shortfall),
    );
    commands.insert_resource(UberParticlePlan {
        planned: plans,
        tally,
    });
}

/// 把一串具名条目折成 `(名字, 条数)` 表——盘点行印它。
fn count_names(items: &[String]) -> Vec<(String, usize)> {
    let mut map: HashMap<&str, usize> = HashMap::new();
    for item in items {
        *map.entry(item.as_str()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = map
        .into_iter()
        .map(|(name, n)| (name.to_owned(), n))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// 逐条判读。放行回 Some，挡下回 None 并在盘点里具名。
fn judge(
    system: &ParticleSystem,
    doc: &SiteSidecar,
    scene: &str,
    texture_dir: &str,
    by_path: &HashMap<String, Vec<Entity>>,
    server: &AssetServer,
    tally: &mut Tally,
) -> Option<Planned> {
    let Some(renderer) = system.renderer.as_ref() else {
        tally.no_renderer += 1;
        return None;
    };
    let renderer: &ParticleRenderer = renderer;
    let material = match renderer.material.as_ref() {
        Some(material) if material.shader == SHADER_NAME => material,
        Some(other) => {
            tally.other_family.push(other.shader.clone());
            return None;
        }
        None => {
            tally.no_material += 1;
            return None;
        }
    };
    tally.records += 1;
    if !renderer.enabled {
        tally.renderer_disabled += 1;
        return None;
    }
    // 发射节点：路径前缀是场景目录名。同路径多实例时取第一个（深度
    // 前序，与清扫链同序）。
    let key = format!("{scene}/{}", system.node);
    let Some(anchor) = by_path.get(&key).and_then(|list| list.first().copied()) else {
        tally.node_unresolved += 1;
        return None;
    };
    if renderer.render_mode != "Billboard" {
        // 记的是「哪个模式 + 它要哪份网格」，不只是模式名。Mesh 模式的
        // 网格引用（包内 glb + 节点）提取产物里是有的，判读行要把它报出来
        // ——否则「9 条被 Mesh 挡下」读不出接下来该去拿什么。
        tally.render_mode.push(match renderer.meshes.as_slice() {
            [] => renderer.render_mode.clone(),
            [mesh] => format!("{}({}#{})", renderer.render_mode, mesh.file, mesh.node),
            many => format!("{}({} 份网格)", renderer.render_mode, many.len()),
        });
        return None;
    }
    let Some(alignment) = Alignment::from_render_space(renderer.alignment) else {
        tally
            .alignment
            .push(Alignment::render_space_name(renderer.alignment).to_owned());
        return None;
    };

    // ---- 仿真侧的门 ----
    // `system` 块原样存在装载层（不在那里解析：律对带权曲线模式具名拒绝，
    // 全量解析会让永远不被放行的记录拖垮整个包）。这里对单条放行候选
    // 调律——能走到这一行的记录已经过了绘制模式与对齐档两道门，带权键
    // 全在被那两道门挡掉的系统里，到不了律解析器。律的档案入口吃
    // `{"effects": {<名>: {"particles": [{node, system}]}}}` 且用律自己的
    // JSON 类型，因此把条目折回那个形状（node 挂回 system 旁边）。
    let Some(system_block) = system.system.as_ref() else {
        tally.no_system_block += 1;
        return None;
    };
    let archive = serde_json::json!({
        "effects": {
            "sidecar": {
                "particles": [{ "node": system.node, "system": system_block.clone() }]
            }
        }
    });
    let archive_bytes = archive.to_string();
    let emitter = match Effects::from_json_str(archive_bytes.as_bytes()) {
        Ok(mut effects) if effects.emitters.len() == 1 => effects.emitters.remove(0),
        Ok(effects) => {
            // 一条进、一条出是构造就保证的；不符说明律的入口改了形状。
            panic!(
                "粒子律单条解析返回 {} 条（应恰 1 条，{}）",
                effects.emitters.len(),
                system.node,
            );
        }
        Err(err) => {
            tally.law_reject.push(format!("{}: {err}", system.node));
            return None;
        }
    };
    if emitter.emission.is_none() {
        tally.no_emission += 1;
        return None;
    }
    let Some(shape) = emitter.shape.as_ref() else {
        tally.no_shape += 1;
        return None;
    };
    if !matches!(shape.shape_type.as_str(), "Circle" | "Cone" | "Sphere" | "Hemisphere" | "SingleSidedEdge") {
        tally.shape.push(shape.shape_type.clone());
        return None;
    }
    let cone_angle = if shape.shape_type == "Cone" {
        match system_block.pointer("/shape/angle").and_then(serde_json::Value::as_f64) {
            Some(angle) if angle.is_finite() => Some(angle as f32),
            _ => { tally.law_reject.push(format!("{}: Cone angle missing", system.node)); return None; }
        }
    } else { None };
    let rol = match emitter.rotation_over_lifetime.as_ref().map(|p| RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve)).transpose() {
        Ok(law) => law,
        Err(reason) => { tally.law_reject.push(format!("{}: {reason}", system.node)); return None; }
    };
    let limit = match emitter.limit_velocity.as_ref().map(|p| LimitVelocity::from_parts(p.separate_axis, &p.magnitude, p.dampen, p.drag.as_ref(), p.multiply_drag_by_size, p.multiply_drag_by_velocity)).transpose() {
        Ok(law) => law,
        Err(reason) => { tally.law_reject.push(format!("{}: {reason}", system.node)); return None; }
    };
    match emitter.simulation_space {
        SimulationSpace::Local | SimulationSpace::World => {}
        _ => {
            tally.sim_space += 1;
            return None;
        }
    }

    // ---- 状态档 ----
    let floats = &material.floats;
    let get = |key: &str| floats.get(key).copied();
    if get("_BlendSrc") != Some(5.0) {
        tally.state_arm.push(format!("_BlendSrc={:?}", get("_BlendSrc")));
        return None;
    }
    let blend = match get("_BlendDst") {
        Some(v) if v == 10.0 => BlendArm::AlphaBlend,
        Some(v) if v == 1.0 => BlendArm::Additive,
        other => {
            tally.state_arm.push(format!("_BlendDst={other:?}"));
            return None;
        }
    };
    let cull = match get("_Cull") {
        Some(v) if v == 0.0 => CullArm::Off,
        Some(v) if v == 1.0 => CullArm::Front,
        Some(v) if v == 2.0 => CullArm::Back,
        other => {
            tally.state_arm.push(format!("_Cull={other:?}"));
            return None;
        }
    };
    if get("_ZTest") != Some(4.0) {
        // LEqual 之外的深度比较在本管线没有开关（透明队列固定 LEqual）。
        tally.state_arm.push(format!("_ZTest={:?}", get("_ZTest")));
        return None;
    }

    // ---- 着色轴：uniform 分支必须全关，否则源程序里那一步在场而本链没有 ----
    for (name, want) in [("_FakeLightEnabled", 0.0), ("_BaseMapRotationEnabled", 0.0)] {
        if get(name) != Some(want) {
            tally.state_arm.push(format!("{name}={:?}", get(name)));
            return None;
        }
    }
    // 亮度键控透明与现象光这两条分支本链已实现（片元链尾段）。开关只认
    // 0/1——源程序的判别是 `0.5 < x`，别的取值不会改变分支走向，但它说明
    // 这份材质不是我们读过的那一档，仍旧拒。
    let luminance_enabled = match get("_TranceparencyByLuminanceEnabled") {
        Some(v) if v == 0.0 || v == 1.0 => v,
        other => {
            tally.state_arm.push(format!("_TranceparencyByLuminanceEnabled={other:?}"));
            return None;
        }
    };
    let phenomena_enabled = match get("_PhenomenaLightEnabled") {
        Some(v) if v == 0.0 || v == 1.0 => v,
        other => {
            tally.state_arm.push(format!("_PhenomenaLightEnabled={other:?}"));
            return None;
        }
    };
    // 亮度臂的两个逐粒子流选择器：本链只实现来源 0（常量零向量）那一档。
    // 分支关着时这两个值不进链，不必管。
    if luminance_enabled == 1.0 {
        for name in [
            "_LuminanceTransparencyProgressCoord",
            "_LuminanceTransparencySharpnessCoord",
        ] {
            if get(name) != Some(0.0) {
                tally.state_arm.push(format!("{name}={:?}", get(name)));
                return None;
            }
        }
    }
    let luminance = Vec4::new(
        get("_LuminanceTransparencyProgress").unwrap_or(0.0),
        get("_LuminanceTransparencySharpness").unwrap_or(0.0),
        get("_InverseLuminanceTransparency").unwrap_or(0.0),
        0.0,
    );
    // 逐粒子自定义流选择器：`coord = 分量号 × 10 + 来源号`，0 = 取常量 0。
    // 本链只实现常量 0 那一档（本站材质集上恰好全 0）。
    // `_TintBlendRateCoord` 已接逐粒子流（顶点属性 custom1/custom2 + 着色器
    // 选择器），不再要求为 0；其余 coord 的选择器接口相同但消费面未接，
    // 仍旧拒——放行了却不喂就是静默的错误值。
    let tint_blend_rate_coord = get("_TintBlendRateCoord").unwrap_or(0.0);
    for name in [
        "_EmissionIntensityCoord",
        "_BaseMapOffsetXCoord",
        "_BaseMapOffsetYCoord",
        "_BaseMapRotationCoord",
    ] {
        if get(name) != Some(0.0) {
            tally.state_arm.push(format!("{name}={:?}", get(name)));
            return None;
        }
    }
    // 关键字：T1 全集之外的关键字意味着源程序里编进了本链没有的步。
    // 软粒子是**放行但未实现**的那一项（alpha 乘法链的末段，要读场景
    // 深度纹理）——具名计数，不静默。
    for keyword in &material.keywords {
        match keyword.as_str() {
            "_BASE_MAP_MODE_2D" | "_EMISSION_MAP_MODE_2D" | "_TINT_COLOR_ENABLED"
            | "_EMISSION_AREA_ALL" | "_TINT_AREA_ALL" => {}
            "_SOFT_PARTICLES_ENABLED" => {
                tally.shading_shortfall.push("软粒子（关键字在场）".to_owned());
            }
            other => {
                tally.state_arm.push(format!("keyword {other}"));
                return None;
            }
        }
    }
    // 深度偏置：源程序的顶点段在 |_ZOffset| > 0.004 时把裁剪空间 z 按
    // 线性视深重映射一次。本链没有这一步——放行并具名计数。
    if get("_ZOffset").map(|v| v.abs() > 0.004).unwrap_or(false) {
        tally
            .shading_shortfall
            .push(format!("深度偏置 _ZOffset={:?}", get("_ZOffset")));
    }

    // ---- 贴图与 uniform ----
    let Some(uri) = base_map_uri(doc, material) else {
        tally.no_base_map += 1;
        return None;
    };
    let texture = crate::site_material::load_dir_texture(
        server,
        doc,
        texture_dir,
        uri,
    );
    // 染色的闸是 area 那一族，不是「启用染色颜色」（后者在全部站点侧
    // 材质上恒真，而编译产物穷举 597 份零反例地表明闸在 area 上）。
    let tint_area = material
        .keywords
        .iter()
        .any(|k| k == "_TINT_AREA_ALL" || k == "_TINT_AREA_RIM");
    let base_st = material
        .texture_scale_offsets
        .get("_BaseMap")
        .copied()
        .unwrap_or([1.0, 1.0, 0.0, 0.0]);
    let tint_colour = material
        .colors
        .get("_TintColor")
        .copied()
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let tint_blend_rate = get("_TintBlendRate").unwrap_or(0.0);

    let soft_enabled = material.keywords.iter().any(|k| k == "_SOFT_PARTICLES_ENABLED");
    let soft_intensity = if soft_enabled {
        get("_SoftParticlesIntensity").filter(|v| v.is_finite())
            .expect("active source soft particles require exported intensity")
    } else { 0.0 };
    let shader_coords = Vec4::new(tint_blend_rate_coord, soft_intensity,
        f32::from(soft_enabled), get("_EmissionIntensityCoord").unwrap_or(0.0));
    Some(Planned {
        node: system.node.clone(),
        emitter,
        anchor,
        alignment,
        cone_angle,
        rol,
        limit,
        params: UberT1Params {
            base_st: Vec4::from_array(base_st),
            tint_colour: Vec4::from_array(tint_colour),
            scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, luminance_enabled, phenomena_enabled),
            luminance,
            coords: shader_coords,
        },
        tint_area,
        cull,
        blend,
        clamp: SizeClamp {
            max_screen_fraction: renderer.max_particle_size,
            min_size: MIN_PARTICLE_SIZE,
        },
        pivot: renderer.pivot,
        texture,
        effect: (renderer.effect_pass == moly_assets::material_passes::EffectPassEligibility::Eligible).then(|| ParticleEmission {
            source_state: renderer.effect_render_state,
            render_queue: renderer.render_queue.expect("eligible effect has a queue"),
            params: UberT1Params { base_st: Vec4::from_array(base_st), tint_colour: Vec4::from_array(tint_colour), scalars: Vec4::new(tint_blend_rate, GLOBAL_MIP_BIAS, luminance_enabled, phenomena_enabled), luminance, coords: shader_coords },
            colour: Vec4::from_array(material.colors.get("_EmissionColor").copied().unwrap_or([1.0; 4])),
            intensity: get("_EmissionIntensity").unwrap_or(1.0),
            colour_type: get("_EmissionColorType").unwrap_or(0.0),
            area: material.keywords.iter().any(|k| k == "_EMISSION_AREA_ALL"),
            // 站点链的材质门只认 UberUnlit（见本文件 `SHADER_NAME`），
            // 原版粒子族不会走到这里 ⇒ 平乘臂恒关。
            tint_area, plain_colour: false, cull, blend,
        }),
    })
}

/// 基础贴图槽的 URI（槽下标指顶层 `textures[]`）。
fn base_map_uri<'a>(doc: &'a SiteSidecar, material: &MaterialSlot) -> Option<&'a str> {
    let index = material
        .textures
        .iter()
        .find(|(prop, _)| prop == "_BaseMap")
        .map(|(_, index)| *index)?;
    doc.texture_uris.get(index).map(String::as_str)
}

// ---- 铺装与推进 ----

/// 全部在跑的系统。
#[derive(Resource)]
pub(crate) struct UberParticleState {
    live: Vec<Runtime>,
    /// 判读盘点（状态行印它）。
    scene: String,
    admitted: usize,
    records: usize,
}

/// Update：贴图到齐后逐条铺实体与状态。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<UberParticleMaterial>>,
    plan: Option<Res<UberParticlePlan>>,
    active: Option<Res<SiteActive>>,
) {
    let (Some(plan), Some(active)) = (plan, active) else {
        return;
    };
    if plan.planned.is_empty() {
        commands.insert_resource(UberParticleState {
            live: Vec::new(),
            scene: active.scene.clone(),
            admitted: 0,
            records: plan.tally.records,
        });
        commands.remove_resource::<UberParticlePlan>();
        return;
    }
    for planned in &plan.planned {
        match server.load_state(&planned.texture) {
            LoadState::Failed(err) => {
                panic!("粒子基础贴图装载失败（{}）：{err:?}", planned.node)
            }
            state if state.is_loaded() => {}
            // 还有没到的：整批等齐再铺（材质的 bind group 需要贴图在场）。
            _ => return,
        }
    }
    let mut live = Vec::with_capacity(plan.planned.len());
    for (index, planned) in plan.planned.iter().enumerate() {
        let mesh = meshes.add(billboard::empty_mesh());
        let material = materials.add(UberParticleMaterial {
            params: planned.params,
            base_map: planned.texture.clone(),
            tint_area: planned.tint_area,
            // 站点链的材质门只认 UberUnlit ⇒ 平乘臂恒关。
            plain_colour: false,
            cull: planned.cull,
            blend: planned.blend,
        });
        // 实体变换恒等：四角已在 CPU 展开成世界坐标，属性即世界坐标。
        // 逐帧重建的属性池没有稳定包围盒，剔除交给 NoFrustumCulling 直通。
        let mut draw = commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            NoFrustumCulling,
            UberParticleDraw,
            crate::shadowmap::NoShadowCast,
        ));
        if let Some(effect) = planned.effect { draw.insert(effect); }
        live.push(runtime_from_plan(planned, mesh, index));
    }
    info!(
        "[uber-particle] {} 上屏：放行 {} 条粒子系统（本族记录 {}），逐条 {:?}",
        active.scene,
        live.len(),
        plan.tally.records,
        live.iter().map(|l| l.node.as_str()).collect::<Vec<_>>(),
    );
    commands.insert_resource(UberParticleState {
        live,
        scene: active.scene.clone(),
        admitted: plan.planned.len(),
        records: plan.tally.records,
    });
    commands.remove_resource::<UberParticlePlan>();
}

/// PostUpdate（变换传播之后）：推进仿真并重建属性池。
///
/// 排在传播之后是因为**局部空间仿真**要读发射节点的当帧世界变换；
/// 排在相机之后是因为四角展开要读当帧机位。
pub(crate) fn advance(
    state: Option<ResMut<UberParticleState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    anchors: Query<&GlobalTransform>,
    inactive: Query<(), With<moly_assets::scene_state::SourceInactive>>,
) {
    let Some(mut state) = state else {
        return;
    };
    if state.live.is_empty() {
        return;
    }
    // 相机拿不到就整帧跳过：不造替身机位（上一帧的属性池还在，几何
    // 不会闪成错的）。
    let Some((camera_transform, projection, camera)) = cameras.iter().next() else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        // 透视公式只对透视投影成立。
        return;
    };
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let basis = billboard::basis_from_matrix(
        camera_transform.affine().matrix3.into(),
        camera_transform.translation(),
        perspective.fov,
        viewport.x as f32 / viewport.y.max(1) as f32,
    );

    let dt = time.delta_secs();
    let state = &mut *state;
    for system in &mut state.live {
        if system.anchor.is_some_and(|entity| inactive.get(entity).is_ok()) {
            if let Some(mesh) = meshes.get_mut(&system.mesh) {
                if mesh.count_vertices() != 0 { *mesh = billboard::empty_mesh(); }
            }
            continue;
        }
        let Some(anchor) = system.anchor.and_then(|entity| anchors.get(entity).ok()).copied() else { continue; };
        let ctx = Context { site: anchor, sky: GlobalTransform::IDENTITY, camera: *camera_transform };
        if !system.prewarmed {
            system.prewarmed = true;
            if system.emitter.prewarm && system.emitter.looping && system.emitter.duration > 0.0 {
                let steps = (system.emitter.duration / PREWARM_STEP).max(1.0) as usize;
                for _ in 0..steps { simulate(system, PREWARM_STEP, &ctx); }
            }
        }
        let step = dt * system.emitter.simulation_speed;
        if step > 0.0 { simulate(system, step, &ctx); }
        let to_world = match system.emitter.simulation_space {
            SimulationSpace::World => GlobalTransform::IDENTITY,
            _ => anchor,
        };
        let Some(mesh) = meshes.get_mut(&system.mesh) else {
            continue;
        };
        crate::particle_runtime::write_geometry(mesh, system, &to_world, &ctx.site, camera_transform, basis);
    }
}

/// Update：周期状态行——逐系统的活粒子数与累计账，全部可从档案复算。
pub(crate) fn report(state: Option<Res<UberParticleState>>) {
    let Some(state) = state else {
        return;
    };
    if state.live.is_empty() {
        info!(
            "[uber-particle] {}：本族记录 {}，放行 {}——本站无在跑的粒子系统",
            state.scene, state.records, state.admitted
        );
        return;
    }
    let live: usize = state.live.iter().map(|s| s.pool.len()).sum();
    let born: u64 = state.live.iter().map(|s| s.born_total).sum();
    let died: u64 = state.live.iter().map(|s| s.died_total).sum();
    let full: u64 = state.live.iter().map(|s| s.full_total).sum();
    let refused: u64 = state.live.iter().map(|s| s.refused_total).sum();
    info!(
        "[uber-particle] {}：在跑 {} 条系统，活粒子 {}（逐条 {:?}）；\
         累计出生 {} 死亡 {} 池满拒发 {} 积分拒绝 {}",
        state.scene,
        state.live.len(),
        live,
        state
            .live
            .iter()
            .map(|s| (s.node.as_str(), s.pool.len(), s.playback_head))
            .collect::<Vec<_>>(),
        born,
        died,
        full,
        refused,
    );
}

fn runtime_from_plan(planned: &Planned, mesh: Handle<Mesh>, index: usize) -> Runtime {
    Runtime {
            node: planned.node.clone(),
            emitter: planned.emitter.clone(),
            anchor: Some(planned.anchor),
            effect: String::new(),
            kind: EffectKind::Site,
            camera_rotation: false,
            node_affine: GlobalTransform::IDENTITY,
            geometry: crate::particle_runtime::Geometry::Billboard {
                alignment: planned.alignment, clamp: planned.clamp, pivot: planned.pivot,
            },
            ring_cursor: 0,
            prewarmed: false,
            cone_angle: planned.cone_angle,
            rol: planned.rol.clone(),
            limit: planned.limit.clone(),
            mesh,
            pool: Vec::new(),
            side: Vec::new(),
            emission: EmissionState::default(),
            playback_head: 0.0,
            previous_head: 0.0,
            emission_started: false,
            // 逐系统换一条流：同一个种子在所有系统上会画出同一个图形。
            rng: Rng(RNG_SEED ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            born_total: 0,
            died_total: 0,
            full_total: 0,
            refused_total: 0,
    }
}

#[derive(Component)]
pub(crate) struct FixtureParticleRequest {
    archive: Handle<moly_assets::json::JsonAsset>,
    package: String,
    planned: Option<Vec<Planned>>,
}
#[derive(Component)]
pub(crate) struct FixtureParticleLive(Runtime);
#[derive(Component)]
pub(crate) struct FixtureParticlesResolved;

pub(crate) fn request_fixture_particles(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<moly_assets::json::JsonAsset>>,
    mut index: Local<Option<Handle<moly_assets::json::JsonAsset>>>,
    roots: Query<(Entity, &crate::fixture::FixtureSource), (With<crate::fixture::FixtureRoot>, Without<FixtureParticleRequest>, Without<FixtureParticlesResolved>)>,
) {
    let handle = index.get_or_insert_with(|| server.load("moly://fixture-particles-v2/index.json"));
    let Some(json) = jsons.get(handle) else { return; };
    let archive: serde_json::Value = serde_json::from_str(&json.0).expect("fixture particle index");
    for (entity, source) in &roots {
        let Some(path) = source.0.path() else { continue; };
        let Some(package) = path.path().file_stem().and_then(|p| p.to_str()) else { continue; };
        let Some(file) = archive["packages"][package]["file"].as_str() else {
            commands.entity(entity).insert(FixtureParticlesResolved); continue;
        };
        commands.entity(entity).insert(FixtureParticleRequest {
            archive: server.load(format!("moly://fixture-particles-v2/{file}")),
            package: package.to_owned(), planned: None,
        });
    }
}

pub(crate) fn plan_fixture_particles(
    mut commands: Commands, server: Res<AssetServer>, jsons: Res<Assets<moly_assets::json::JsonAsset>>,
    mut roots: Query<(Entity, &mut FixtureParticleRequest)>,
    ready: Option<Res<crate::fixture::FixtureScenesReady>>,
    names: Query<&Name>, children: Query<&Children>,
) {
    if ready.is_none() { return; }
    for (entity, mut request) in &mut roots {
        if request.planned.is_some() { continue; }
        let Some(json) = jsons.get(&request.archive) else {
            if let LoadState::Failed(error) = server.load_state(&request.archive) {
                warn!("fixture particles {} unavailable: {error}", request.package);
                commands.entity(entity).remove::<FixtureParticleRequest>().insert(FixtureParticlesResolved);
            }
            continue;
        };
        let raw: serde_json::Value = serde_json::from_str(&json.0).expect("fixture particle JSON");
        let doc = match moly_assets::sidecar::parse_fixture_particles(json.0.as_bytes()) {
            Ok(doc) => doc,
            Err(error) => { warn!("fixture particles {}: {error}", request.package); request.planned = Some(Vec::new()); continue; }
        };
        let mut by_path = HashMap::new();
        crate::inactive_nodes::collect(entity, &mut Vec::new(), &names, &children, &mut by_path);
        let by_path: HashMap<_, _> = by_path.into_iter().map(|(p, entities)| (format!("/{p}"), entities)).collect();
        let mut tally = Tally::default();
        let mut plans = Vec::new();
        let mut not_play_on_awake = 0usize;
        for (index, particle) in doc.particles.iter().enumerate() {
            let source = &raw["emitters"][index];
            // An inactive event template requires its actual activation binding.
            // Missing activation metadata cannot be treated as an active instance.
            if source.get("activeInHierarchy").and_then(serde_json::Value::as_bool) != Some(true) {
                tally.law_reject.push(format!("{}: activeInHierarchy unresolved or inactive", particle.node)); continue;
            }
            if particle.system.as_ref().and_then(|v| v.get("playOnAwake")).and_then(serde_json::Value::as_bool) != Some(true) {
                not_play_on_awake += 1;
                continue;
            }
            if let Some(plan) = judge(particle, &doc, "", "fixture-particles-v2/textures", &by_path, &server, &mut tally) { plans.push(plan); }
        }
        // 家具这条路此前只报 `plans.len()` 与 `law_reject`，其余每一个桶都被
        // 计进 `tally` 然后丢掉——实测 304 条拒绝里 292 条不出现在任何日志
        // 里，读起来像「没有可做的事」。未实现必须可审计、可统计，所以这里
        // 与站点那条路报同一份账。
        info!(
            "[uber-particle] 家具 {}：本族记录 {}；放行 {}；\
             挡下——非 playOnAwake {} · 节点路径未解析 {} · 绘制模式 {:?} · 对齐档 {:?} · \
             发射形状律缺 {:?} · 缺形状模块 {} · 缺 emission {} · 缺 system 块 {} · \
             律拒 {:?} · 仿真空间 {} · 缺基础贴图 {} · 状态档族外 {:?} · \
             渲染器关 {} · 无渲染器 {} · 无材质 {} · 非本族 {:?}；\
             放行但未实现（逐条具名，不静默）：{:?}",
            request.package,
            tally.records,
            tally.admitted,
            not_play_on_awake,
            tally.node_unresolved,
            count_names(&tally.render_mode),
            count_names(&tally.alignment),
            count_names(&tally.shape),
            tally.no_shape,
            tally.no_emission,
            tally.no_system_block,
            count_names(&tally.law_reject),
            tally.sim_space,
            tally.no_base_map,
            count_names(&tally.state_arm),
            tally.renderer_disabled,
            tally.no_renderer,
            tally.no_material,
            count_names(&tally.other_family),
            count_names(&tally.shading_shortfall),
        );
        request.planned = Some(plans);
    }
}

pub(crate) fn spawn_fixture_particles(
    mut commands: Commands, server: Res<AssetServer>,
    mut roots: Query<(Entity, &mut FixtureParticleRequest)>,
    mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<UberParticleMaterial>>,
) {
    for (root, mut request) in &mut roots {
        let Some(plans) = request.planned.as_ref() else { continue; };
        if plans.iter().any(|p| !server.load_state(&p.texture).is_loaded()) { continue; }
        for (index, planned) in request.planned.take().unwrap().iter().enumerate() {
            let mesh = meshes.add(billboard::empty_mesh());
            let material = materials.add(UberParticleMaterial::new(planned.params, planned.texture.clone(), planned.tint_area, false, planned.cull, planned.blend));
            let mut draw = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY,
                NoFrustumCulling, crate::shadowmap::NoShadowCast,
                FixtureParticleLive(runtime_from_plan(planned, mesh, index))));
            if let Some(effect) = planned.effect { draw.insert(effect); }
        }
        commands.entity(root).remove::<FixtureParticleRequest>().insert(FixtureParticlesResolved);
    }
}

pub(crate) fn advance_fixture_particles(
    mut commands: Commands, mut live: Query<(Entity, &mut FixtureParticleLive)>,
    anchors: Query<&GlobalTransform>, cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    inactive: Query<(), With<moly_assets::scene_state::SourceInactive>>,
    time: Res<Time>, mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some((camera_transform, Projection::Perspective(projection), camera)) = cameras.iter().next() else { return; };
    let Some(viewport) = camera.physical_viewport_size() else { return; };
    let basis = billboard::basis_from_matrix(camera_transform.affine().matrix3.into(), camera_transform.translation(), projection.fov, viewport.x as f32 / viewport.y.max(1) as f32);
    for (entity, mut particle) in &mut live {
        let system = &mut particle.0;
        if system.anchor.is_some_and(|entity| inactive.get(entity).is_ok()) {
            if let Some(mesh) = meshes.get_mut(&system.mesh) {
                if mesh.count_vertices() != 0 { *mesh = billboard::empty_mesh(); }
            }
            continue;
        }
        let Some(anchor) = system.anchor.and_then(|e| anchors.get(e).ok()).copied() else { commands.entity(entity).despawn(); continue; };
        let ctx = Context { site: anchor, sky: GlobalTransform::IDENTITY, camera: *camera_transform };
        if !system.prewarmed {
            system.prewarmed = true;
            if system.emitter.prewarm && system.emitter.looping {
                for _ in 0..(system.emitter.duration / PREWARM_STEP).max(1.0) as usize { simulate(system, PREWARM_STEP, &ctx); }
            }
        }
        let dt = time.delta_secs() * system.emitter.simulation_speed;
        if dt > 0.0 { simulate(system, dt, &ctx); }
        let transform = if system.emitter.simulation_space == SimulationSpace::World { GlobalTransform::IDENTITY } else { anchor };
        if let Some(mesh) = meshes.get_mut(&system.mesh) {
            crate::particle_runtime::write_geometry(mesh, system, &transform, &anchor, camera_transform, basis);
        }
    }
}

/// 拆站面：撤下计划与状态，让新站的判读重新起跳。实体随场景树一起撤。
pub(crate) fn teardown(commands: &mut Commands) {
    commands.remove_resource::<UberParticlePlan>();
    commands.remove_resource::<UberParticleState>();
}
