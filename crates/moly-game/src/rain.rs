//! 雨粒子：粒子律（moly-law::particle）的呈现层消费。
//!
//! 分工：仿真侧**只消费不重写**——率发射、形状采样、重力、积分、寿命、
//! 池的进出全部走律函数；渲染侧把律状态逐帧喂进顶点属性（一张网格、
//! 每粒子四个顶点），着色按源程序片元链的闸后子集转录（`rain.wgsl`）。
//!
//! 范围（本单）：只仿真雨现象档案里 common 站点份的雨滴本体——站点
//! 特定 effect 内嵌的是同一棵发射树的副本，不重复跑；天空/相机侧
//! effect 与子发射器的处置在装载时分类计数（见 `parse` 的盘点行）。
//! 路径沿用提取产物布局，在本仓内联成 `moly://` 路径——路径助手是
//! moly-assets 的逐域清单，本单不为此碰那个 crate（与天空壳同款做法）。
//!
//! 挂账不静默：软粒子按范围裁决不做（keyword 在场，装载时 warn 计数）；
//! 染色环被 HDR 闸拒（源材质的染色乘数超线性域，装载时 warn 计数）；
//! 子发射器与 Mesh 绘制模式不接（盘点行报数）。逐条清单在收工报告。
//!
//! 随机流：源引擎的随机不可见，出生取值表在演示件里是**逐模块独立
//! 取值**（不是全体共用一个因子）——此处按同形状用一条确定性流顺序
//! 取值（形状两次、寿命/尺寸/颜色/种子各一次），种子因子另存供生命
//! 期内的稳定求值（速度/重力/VoL 插值）。固定种子换可复算：同一份
//! 档案、同一个种子，逐帧状态可复算。

use bevy::asset::uuid::Uuid;
use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_law::particle::value::MinMaxGradient;
use moly_law::particle::{
    accumulate_rate, advance_lifetime, apply_gravity, circle_position, euler_rotate_deg, integrate,
    ring_push, Effects, EmissionState, EmitterParams, LifetimeVerdict, Particle, RingPushVerdict,
    StepVerdict,
};
use moly_law::particle::schema::SimulationSpace;
use std::marker::PhantomData;

/// 雨现象的粒子档案目录（现象切换归天气域，这里定死一份）。
const PHENOMENON: &str = "006_rain";

/// 主发射器：common 站点份里的雨滴本体。
const EMITTER_EFFECT: &str = "fx_env_site_006_common_raindrop";
const EMITTER_NODE: &str = "root/raindrop_01";

/// 重力常量：引擎侧 Vector3(0, -9.81, 0) 的口径，演示件同值。
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// 零缩放粒子会被背面剔除吞掉，尺寸下限同演示件。
const MIN_PARTICLE_SCALE: f32 = 0.0001;

/// prewarm 快进步长：与运行帧率同一量级即可。prewarm 的档位语义是
/// 「开播前把一个周期快进完」，不是精确复算（收工报告具名）。
const PREWARM_STEP: f32 = 1.0 / 60.0;

/// 出生抽签的确定性随机种子（固定值换可复算）。
const RNG_SEED: u64 = 0x7261_696E_0060_0001;

/// 雨着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`。
const RAIN_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x9a3d_2b71_c4e8_4f0f_8e17_5b26_d9a0_6c52),
    PhantomData,
);

/// 雨粒子材质：独立一族，不混站点 SiteMaterial。uniform 两槽是片元链
/// 的静态输入（片表与基础槽的 ST）；逐粒子量（颜色）由顶点流携带。
#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
pub struct RainMaterial {
    /// 片表动画的格数与偏移。本材质基础图是单张 2D 图，无片表 →
    /// (1, 1, 0, 0)，装载时按档位断言。
    #[uniform(0)]
    sheet: Vec4,
    /// 基础图的缩放平移（textureScaleOffset._BaseMap）。
    #[uniform(1)]
    base_st: Vec4,
    #[texture(2)]
    #[sampler(3)]
    base_map: Handle<Image>,
}

impl Material for RainMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(RAIN_SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(RAIN_SHADER.clone())
    }
    /// 混合态：SrcAlpha / OneMinusSrcAlpha、深度不写、透明队列——与材质
    /// 记录的混合与深度档一致（装载时逐值断言）。双面（源记录关闭剔除）
    /// 由两条绕序的索引实现；颜色掩码只写 RGB 在本管线无对应开关，多写
    /// 的 alpha 无下游读者。
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

/// 档案的装载请求；解析完成即撤。
#[derive(Resource)]
pub(crate) struct RainArchive(Handle<JsonAsset>);

/// 判读完成待铺的完整计划：律侧参数、渲染侧输入、停发盘点结果。
#[derive(Resource)]
pub(crate) struct RainPlan {
    params: EmitterParams,
    /// colorOverLifetime 的梯度（缺 None：颜色只剩出生色）。
    col: Option<MinMaxGradient>,
    /// 发射节点链的累计平移（装载时逐节点断言过：只有平移）。
    node_offset: [f32; 3],
    /// 视口占比上限（渲染器记录 maxParticleSize）。
    max_screen_frac: f32,
    /// 基础图 ST（材质记录的静态输入）。
    base_st: [f32; 4],
    texture: Handle<Image>,
    texture_file: String,
    /// 停发盘点行（parse 现算，spawn 后随状态行一起印）。
    dispositions: String,
}

/// 逐粒子的平行属性。律池是 `Vec<Particle>`（律函数的签名容器），渲染
/// 侧的出生抽定值以同下标平行存储，死亡时与律池同步交换删除。
#[derive(Clone, Copy)]
struct ParticleSide {
    /// 出生种子因子（归一 [0,1)）：生命期内稳定求值（速度/重力/VoL 插值）
    /// 都读它，出生一次、终生不变。
    rand: f32,
    /// 出生尺寸（米）：X 与 Y 是两笔独立取值（演示件出生取值表逐轴独立）。
    /// sizeOverLifetime 缺席 → 全寿命恒定。
    size: [f32; 2],
    /// 重力系数（start.gravityModifier 出生求值）。
    gravity: f32,
}

/// 仿真状态 + 渲染喂入池。
#[derive(Resource)]
pub(crate) struct RainState {
    params: EmitterParams,
    col: Option<MinMaxGradient>,
    pool: Vec<Particle>,
    side: Vec<ParticleSide>,
    emission: EmissionState,
    /// 环形替换游标（本档案模式 0 不参与，律签名要它）。
    cursor: usize,
    /// 系统播头（秒）：率曲线的时间轴。
    playback_head: f32,
    rng: Rng,
    node_offset: [f32; 3],
    max_screen_frac: f32,
    mesh: Handle<Mesh>,
    born_total: u64,
    died_total: u64,
    full_total: u64,
    refused_total: u64,
    clamped_total: u64,
}

/// 出生抽签的确定性随机：splitmix64 高位取 [0,1)。
struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }
}

/// Material 管线注册：材质资产类型与专用渲染管线集合由它落进 App；
/// 不挂它，引用该材质的 system 会在参数校验上响亮失败。在 `app()` 里
/// DefaultPlugins 之后调用一次。
pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<RainMaterial>::default());
}

/// Startup：请求装载雨粒子档案，内嵌雨着色程序。
///
/// 路径沿用提取产物布局，在本仓内联成 `moly://` 资产路径（同天空壳）。
pub(crate) fn load(mut commands: Commands, mut shaders: ResMut<Assets<Shader>>, server: Res<AssetServer>) {
    // 返回的句柄就是钉死的 RAIN_SHADER，丢弃以免 must_use 告警。
    let _ = shaders.insert(
        RAIN_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/rain.wgsl"),
            "moly_game/src/shaders/rain.wgsl".to_owned(),
        ),
    );
    commands.insert_resource(RainArchive(server.load::<JsonAsset>(AssetPath::from(
        format!("moly://phenomena/{PHENOMENON}/fx/effects.json"),
    ))));
}

/// Update：档案到达 → 律侧 + 渲染侧两半各自判读 → 请求贴图、落计划。
/// 装载与解析失败一律 panic（资产边界的唯一拒绝点，不造替身）；族外
/// 形态（未实现的档位）同样响亮拒绝——静默降级会把未做完的活伪装成
/// 交付物。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    archives: Res<Assets<JsonAsset>>,
    handle: Option<Res<RainArchive>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("雨粒子档案装载失败：{err:?}");
    }
    let Some(asset) = archives.get(&handle.0) else {
        return;
    };

    // ---- 律半：整份档案进律 schema，选出发射器 ----
    let effects = Effects::from_json_str(asset.0.as_bytes())
        .unwrap_or_else(|err| panic!("雨粒子档案解析失败：{err}"));
    let emitter = effects
        .emitters
        .iter()
        .find(|e| e.effect == EMITTER_EFFECT && e.node == EMITTER_NODE)
        .unwrap_or_else(|| panic!("档案里找不到发射器 {EMITTER_EFFECT}/{EMITTER_NODE}"));

    // ---- 渲染半：同一份 JSON 里的 renderer/material 记录 ----
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|err| panic!("雨粒子档案不是合法 JSON：{err}"));
    let record = effect_record(&value)
        .and_then(|effect| {
            effect
                .get("particles")
                .and_then(|v| v.as_array())
                .and_then(|list| {
                    list.iter().find(|p| p.get("node").and_then(|n| n.as_str())
                        == Some(EMITTER_NODE))
                })
        })
        .unwrap_or_else(|| panic!("档案 JSON 里定位不到 {EMITTER_EFFECT}/{EMITTER_NODE} 的记录"));
    let renderer = record
        .get("renderer")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 缺 renderer 记录"));
    if renderer.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
        warn!("[rain] {EMITTER_NODE} 的渲染器是关的：整条停发，不画（fail-closed）");
        commands.remove_resource::<RainArchive>();
        return;
    }
    let render_mode = renderer.get("renderMode").and_then(|v| v.as_str()).unwrap_or("");
    if render_mode != "Billboard" {
        panic!("{EMITTER_NODE} 的绘制模式是 {render_mode:?}：粒子族只实现了朝相机四边形");
    }
    // `ParticleSystemRenderSpace` 的档位是 View=0 / World=1 / Local=2 / Facing=3 /
    // Velocity=4（锚在引擎自己的枚举声明，两份独立的引擎源逐字相同）。
    //
    // ⛔ 档案给的是 1 = World（与世界轴对齐），而下面的四边形构造用相机 forward、
    // fov 与 aspect 逐帧算，实现的是 0 = View（朝相机平面）。**这两者不是一回事，
    // 当前画的是错档。** 此处曾写「1 = View」并对 `!= 1` 拒绝：那个映射是从我方
    // 期望倒推的——已知雨该朝相机，量到值是 1，于是把 1 标成 View。于是这道门拦
    // 的条件变成「不等于我实测到的那个值」，恒绿，而朝向一直错。
    //
    // 不在此处拒绝：World 档的四边形构造是能写的（缺的是实现，不是来源），
    // 把它做成拒绝等于把一个未做完的实现封装成交付物。所以响亮报出、继续画，
    // 缺口具名挂在这里，等 World 档接上后连同这段注释一起改掉。
    let alignment = renderer.get("alignment").and_then(|v| v.as_f64());
    if alignment == Some(1.0) {
        warn!(
            "[rain] {EMITTER_NODE} 的 alignment 是 1（World，与世界轴对齐），\
             而本族的四边形构造实现的是 0（View，朝相机平面）：朝向按错档画，已具名挂账"
        );
    } else if alignment != Some(0.0) {
        panic!(
            "{EMITTER_NODE} 的 alignment 是 {alignment:?}：本族只认 0（View）与 1（World），\
             Local/Facing/Velocity 三档未实现"
        );
    }
    let material = renderer
        .get("material")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 缺 material 记录"));

    // 混合与深度态逐值断言：管线由 Blend 档与双绕序实现，值对不上说明
    // 来了族外形态。
    let floats = material
        .get("floats")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 floats 表"));
    expect_float(floats, "_BlendSrc", 5.0);
    expect_float(floats, "_BlendDst", 1.0);
    expect_float(floats, "_ZTest", 4.0);
    expect_float(floats, "_ZWrite", 0.0);
    expect_float(floats, "_ColorMask", 14.0);
    expect_float(floats, "_Cull", 0.0);
    // 基础图是单张 2D 图（无片表动画、无绕轴心旋转）。
    expect_float(floats, "_BaseMapMode", 0.0);
    expect_float(floats, "_BaseMapRotationEnabled", 0.0);
    // 假光、alpha 过渡、现象光照全关（演示件同款关闭环，链上无此步）。
    expect_float(floats, "_FakeLightEnabled", 0.0);
    expect_float(floats, "_AlphaTransitionMode", 0.0);
    expect_float(floats, "_PhenomenaLightEnabled", 0.0);

    let keywords = material
        .get("keywords")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 keywords"))
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    let has = |k: &str| keywords.contains(&k);

    // 染色闸（与演示件同式）：全域档时乘数 = 1 + 混合率·(色 − 1)，任一
    // RGB 通道超线性域（> 1.0001）即 HDR 不可表示 → 拒，片元链无染色步
    // （`rain.wgsl` 注释同此）。过闸的低动态染色路径本族未实现，来了就
    // panic——那是要补实现，不是要静默。
    if has("_TINT_AREA_RIM") {
        panic!("{EMITTER_NODE} 染色是边缘档：未实现");
    }
    if has("_TINT_AREA_ALL") {
        if has("_TINT_MAP_ENABLED") {
            panic!("{EMITTER_NODE} 染色带图档：未实现");
        }
        let rate = read_float(floats, "_TintBlendRate");
        let color = read_color(material, "_TintColor");
        let mul = [
            1.0 + rate * (color[0] - 1.0),
            1.0 + rate * (color[1] - 1.0),
            1.0 + rate * (color[2] - 1.0),
        ];
        if mul.iter().any(|v| *v > 1.0001) {
            warn!(
                "[rain] 染色 HDR 不可表示（_TintColor {:?} × 混合率 {}）：闸拒，片元链无染色步",
                color, rate
            );
        } else {
            panic!("{EMITTER_NODE} 的 _TintColor 是低动态值：染色环未实现，来了就补");
        }
    }
    if has("_DISSOLVE_TRANSITION_ENABLED") || has("_FADE_TRANSITION_ENABLED") {
        panic!("{EMITTER_NODE} 带 alpha 过渡关键字：未实现");
    }
    if material
        .get("textures")
        .and_then(|v| v.as_object())
        .map(|t| t.contains_key("_FlowMap"))
        .unwrap_or(false)
    {
        panic!("{EMITTER_NODE} 带 flow 图：未实现");
    }
    if has("_SOFT_PARTICLES_ENABLED") {
        // 裁决不做：软粒子要读场景深度，本单不建深度读取链。alpha 乘法链
        // 少最后一步，遮挡边缘会略实——挂账具名，不静默。
        warn!("[rain] 软粒子关键字在场：按裁决不做，alpha 链少末段");
    }

    let read_vec4 = |map: &serde_json::Map<String, serde_json::Value>, key: &str| {
        map.get(key)
            .and_then(|v| v.as_array())
            .map(|a| [read_st(a, 0), read_st(a, 1), read_st(a, 2), read_st(a, 3)])
            .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 {key}"))
    };
    // 基础图与 ST。缺槽即 panic（fail-closed：不拿默认图顶替）。
    let texture_file = material
        .get("textures")
        .and_then(|t| t.get("_BaseMap"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 _BaseMap 贴图槽"))
        .to_owned();
    let base_st = read_vec4(
        material
            .get("textureScaleOffset")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 textureScaleOffset 表")),
        "_BaseMap",
    );
    let max_screen_frac = renderer
        .get("maxParticleSize")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 缺 maxParticleSize")) as f32;
    let min_screen_frac = renderer
        .get("minParticleSize")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 缺 minParticleSize")) as f32;
    if min_screen_frac != 0.0 {
        panic!("{EMITTER_NODE} 的 minParticleSize 非 0：视口占比下限未实现");
    }

    // ---- 仿真侧的门：本单只支持这组取值，族外形态响亮拒绝 ----
    if emitter.shape.as_ref().map(|s| s.shape_type.as_str()) != Some("Circle") {
        panic!("{EMITTER_NODE} 的发射形状不是 Circle：形状律未落");
    }
    if !emitter.looping {
        panic!("{EMITTER_NODE} 非循环系统：播放到头的停发语义未实现");
    }
    if !emitter.prewarm {
        panic!("{EMITTER_NODE} prewarm 关：冷启动语义未实现（本单按快进档实现）");
    }
    if let SimulationSpace::World = emitter.simulation_space {
        // 世界空间：粒子位置即世界坐标，实体变换恒等。
    } else {
        panic!("{EMITTER_NODE} 的仿真空间不是 World：局部空间跟随未实现");
    }
    if emitter.start_delay.evaluate(0.0, 0.5) != 0.0 {
        panic!("{EMITTER_NODE} startDelay 非零：延迟发射未实现");
    }
    if emitter.start.lifetime.evaluate(0.0, 0.5) <= 0.0 {
        panic!("{EMITTER_NODE} startLifetime 非正：造不出活粒子");
    }
    if emitter.emission.is_none() {
        panic!("{EMITTER_NODE} 缺 emission 模块：发射率无来源");
    }

    // 发射节点链：只支持平移（旋转近恒等、缩放 1，逐节点断言）。
    let node_offset = node_world_offset(&value);

    // ---- 其余发射器的处置盘点（停发分类计数） ----
    let dispositions = report_dispositions(&effects, record);

    let texture = server.load::<Image>(AssetPath::from(format!(
        "moly://phenomena/{texture_file}"
    )));
    commands.insert_resource(RainPlan {
        params: emitter.clone(),
        col: emitter.color_over_lifetime.clone(),
        node_offset,
        max_screen_frac,
        base_st,
        texture,
        texture_file,
        dispositions,
    });
    commands.remove_resource::<RainArchive>();
}

/// Update：贴图到齐后铺实体与状态，并把 prewarm 快进完。
///
/// 贴图装载失败 panic；没到就等（材质的 bind group 需要贴图在场，
/// 缺着 spawn 会在渲染侧炸）。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RainMaterial>>,
    plan: Option<Res<RainPlan>>,
) {
    let Some(plan) = plan else {
        return;
    };
    match server.load_state(&plan.texture) {
        LoadState::Failed(err) => panic!("雨滴贴图装载失败：{err:?}"),
        s if s.is_loaded() => {}
        _ => return,
    }
    let mesh = meshes.add(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    ));
    let material = materials.add(RainMaterial {
        // 无片表动画（基础图是单张 2D 图，装载时按档位断言）→ 恒等格。
        sheet: Vec4::new(1.0, 1.0, 0.0, 0.0),
        base_st: Vec4::from_array(plan.base_st),
        base_map: plan.texture.clone(),
    });
    // 世界空间仿真：实体变换恒等，顶点属性即世界坐标。逐帧重建的属性池
    // 没有稳定的包围盒，剔除交给 NoFrustumCulling 直通。NoShadowCast：雨
    // 不进主光深度图（真源粒子不作 shadow caster）。
    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        NoFrustumCulling,
        crate::shadowmap::NoShadowCast,
    ));
    let mut state = RainState {
        params: plan.params.clone(),
        col: plan.col.clone(),
        pool: Vec::new(),
        side: Vec::new(),
        emission: EmissionState::default(),
        cursor: 0,
        playback_head: 0.0,
        rng: Rng(RNG_SEED),
        node_offset: plan.node_offset,
        max_screen_frac: plan.max_screen_frac,
        mesh,
        born_total: 0,
        died_total: 0,
        full_total: 0,
        refused_total: 0,
        clamped_total: 0,
    };
    // prewarm：开播前把一个周期快进完（档位语义：让开播即是稳态，不是
    // 空池起跑）。快进只动仿真状态，不喂渲染。
    let cycle = state.params.duration / PREWARM_STEP;
    for _ in 0..cycle.max(1.0) as usize {
        simulate_step(&mut state, PREWARM_STEP);
    }
    commands.insert_resource(state);
    commands.remove_resource::<RainPlan>();
    let p = &plan;
    let shape = p.params.shape.as_ref().expect("parse 已门形状在场");
    info!(
        "[rain] 雨滴上屏：发射器 {}/{}，率 {}，容量 {}，寿命 {}，盘 r{} 厚 {} 弧 {}，节点抬升 {:?}，贴图 {}，视口占比上限 {}; {}",
        EMITTER_EFFECT,
        EMITTER_NODE,
        p.params.emission.as_ref().expect("parse 已门 emission 在场")
            .rate_over_time
            .evaluate(0.0, 0.5),
        p.params.max_particles,
        p.params.start.lifetime.evaluate(0.0, 0.5),
        shape.radius,
        shape.radius_thickness,
        shape.arc,
        p.node_offset,
        p.texture_file,
        p.max_screen_frac,
        p.dispositions
    );
}

/// Update：一帧推进——发射、积分、寿命、死亡移除，然后按律状态重建
/// 顶点属性池。
pub(crate) fn advance(
    state: Option<ResMut<RainState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
) {
    let Some(mut state) = state else {
        return;
    };
    let dt = time.delta_secs() * state.params.simulation_speed;
    if dt > 0.0 {
        simulate_step(&mut state, dt);
    }

    // ---- 渲染喂入：逐帧重建四属性 + 索引 ----
    let Some(mesh) = meshes.get_mut(&state.mesh) else {
        return;
    };
    // 从 ResMut 里取出字段级可变借用：池、side、计数各自独立读写。
    let state = &mut *state;
    let n = state.pool.len();
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut corners: Vec<[f32; 2]> = Vec::with_capacity(n * 4);
    let mut sizes: Vec<[f32; 2]> = Vec::with_capacity(n * 4);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(n * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 12);

    // 相机态给视口占比钳制用；相机不在（无渲染环境）就跳过钳制。
    let camera = cameras.single().ok();
    let fov_y = camera.and_then(|(_, p, _)| match p {
        Projection::Perspective(p) => Some(p.fov),
        // 透视公式只对透视投影成立；正交没有 tan(fov/2) 语义。
        _ => None,
    });
    let aspect = camera.and_then(|(_, _, c)| {
        c.physical_viewport_size()
            .map(|s| s.x as f32 / s.y.max(1) as f32)
    });
    let cam_pos = camera.map(|(g, _, _)| g.translation());
    let cam_forward = camera.map(|(g, _, _)| g.forward());

    for (i, p) in state.pool.iter().enumerate() {
        let s = state.side[i];
        // 归一化年龄：律的倒计时折算（start_lifetime 恒正，parse 已门）。
        let age = p.normalized_age();
        // colorOverLifetime 按年龄求值（Gradient 模式忽略插值因子）。
        let color = match &state.col {
            Some(g) => g.evaluate(age, s.rand),
            // 缺模块时颜色只剩出生色；出生色是白（min-max 单值档），此处
            // 直接白——与「乘白无贡献」的源语义一致。
            None => [1.0, 1.0, 1.0, 1.0],
        };
        let mut sx = s.size[0].abs().max(MIN_PARTICLE_SCALE);
        let mut sy = s.size[1].abs().max(MIN_PARTICLE_SCALE);
        // 视口占比钳制（演示件同式）：深度 d 处的视口宽 W = 2·d·aspect·
        // tan(fov/2)，两轴全尺寸的最大者占比超上限时两轴同乘比例（保
        // 长宽比）。粒度是逐粒子计数，不逐帧累计。
        if let (Some(fov), Some(aspect), Some(pos), Some(forward)) =
            (fov_y, aspect, cam_pos, cam_forward)
        {
            let d = forward.dot(pos - Vec3::from_array(p.position));
            if d > 0.0 {
                let w = 2.0 * d * aspect * (fov * 0.5).tan();
                let biggest = sx.max(sy);
                let k = w * state.max_screen_frac / biggest;
                if k < 1.0 {
                    sx *= k;
                    sy *= k;
                    state.clamped_total += 1;
                }
            }
        }
        let base = (i * 4) as u32;
        // 四角点（0..1），双绕序两份索引：剔除档是源记录的关闭档，两条
        // 绕序各画一面。
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            positions.push(p.position);
            corners.push(corner);
            sizes.push([sx, sy]);
            colors.push(color);
        }
        indices.extend_from_slice(&[
            base, base + 1, base + 2, base, base + 2, base + 3, base, base + 2, base + 1,
            base, base + 3, base + 2,
        ]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, corners);
    // UV_1 槽换语义：粒子 X/Y 全尺寸（米）。着色器按同一语义读。
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, sizes);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
}

/// Update：周期状态行——活粒子数、累计出生/死亡、拒发与钳制，全部可
/// 从这行与档案复算。
pub(crate) fn report(state: Option<Res<RainState>>) {
    let Some(state) = state else {
        return;
    };
    let rate = state
        .params
        .emission
        .as_ref()
        .expect("parse 已门 emission 在场")
        .rate_over_time
        .evaluate(state.playback_head, 0.5);
    info!(
        "[rain] 活 {} / 容量 {}；率 {}/s；累计出生 {} 死亡 {} 池满拒发 {} 积分拒绝 {} 视口钳制 {}；播头 {:.1}s",
        state.pool.len(),
        state.params.max_particles,
        rate,
        state.born_total,
        state.died_total,
        state.full_total,
        state.refused_total,
        state.clamped_total,
        state.playback_head,
    );
}

/// 一个仿真步（发射 → 推进 → 死亡移除）。prewarm 快进与逐帧推进共用。
///
/// 速度合成序（**未核**，收工报告具名）：律的 `integrate` 会把本帧合成
/// 速度写回粒子状态，而源程序的生命期速度（VoL）逐帧叠加、不入状态
/// （演示件同口径）。此处先让重力累积进状态速度、把本帧 VoL 加在外
/// 面合成，积分后把状态速度种回去——VoL 不进状态、重力只进状态。
fn simulate_step(state: &mut RainState, dt: f32) {
    // 发射：率曲线按播头求值，律的累加器吃整帧。
    let rate = state
        .params
        .emission
        .as_ref()
        .expect("parse 已门 emission 在场")
        .rate_over_time
        .evaluate(state.playback_head, 0.5);
    let emitted = accumulate_rate(&mut state.emission, rate, dt);
    state.playback_head += dt;
    for _ in 0..emitted {
        spawn_one(state);
    }

    // 推进：逐粒子重力 → VoL 合成 → 积分 → 寿命。
    let vol = state.params.velocity_over_lifetime.clone();
    let speed_modifier = vol
        .as_ref()
        .map(|v| v.speed_modifier.evaluate(0.0, 0.5))
        .unwrap_or(1.0);
    let mut dead: Vec<usize> = Vec::new();
    for i in 0..state.pool.len() {
        let s = state.side[i];
        let p = &mut state.pool[i];
        // 重力先入状态速度（演示件同次序）。
        apply_gravity(p, dt, GRAVITY, s.gravity);
        // 本帧 VoL：按年龄求值，插值因子用出生种子（终生稳定）。
        let age = p.normalized_age();
        let vel = match &vol {
            Some(v) => [
                v.x.evaluate(age, s.rand),
                v.y.evaluate(age, s.rand),
                v.z.evaluate(age, s.rand),
            ],
            None => [0.0; 3],
        };
        let frame = [
            (p.velocity[0] + vel[0]) * speed_modifier,
            (p.velocity[1] + vel[1]) * speed_modifier,
            (p.velocity[2] + vel[2]) * speed_modifier,
        ];
        let state_vel = p.velocity;
        if let StepVerdict::Refused = integrate(p, dt, frame) {
            state.refused_total += 1;
        }
        // 把律的覆写种回状态速度：VoL 不入状态。
        p.velocity = state_vel;
        if let LifetimeVerdict::Died = advance_lifetime(
            p,
            dt,
            state.params.ring_buffer_mode,
            state.params.ring_buffer_loop_range,
        ) {
            dead.push(i);
        }
    }
    // 死亡移除：裁决来自律（模式 0 死亡即移除）。从高位往低位交换删除
    // （交换删除不保序，与律的池压实同形），side 池同步镜像。
    for &i in dead.iter().rev() {
        state.pool.swap_remove(i);
        state.side.swap_remove(i);
        state.died_total += 1;
    }
}

/// 出生一颗：形状采样 → 出生取值 → 律的入池裁决。
fn spawn_one(state: &mut RainState) {
    // 出生抽签：形状两次（半径、角度），寿命/尺寸X/尺寸Y/种子各一次，
    // 颜色因子一次（单值档忽略）。顺序取值是本仓的表示选择（源引擎的
    // 流分配不可见），固定种子下可复算。
    let r = state.rng.next_f32();
    let shape_r = state.rng.next_f32();
    let shape_theta = state.rng.next_f32();
    let life_r = state.rng.next_f32();
    let sx_r = state.rng.next_f32();
    let sy_r = state.rng.next_f32();

    let shape = state.params.shape.as_ref().expect("parse 已门形状在场");
    let local = circle_position(
        shape.radius,
        shape.radius_thickness,
        shape.arc,
        shape_r,
        shape_theta,
    );
    let local = euler_rotate_deg(shape.rotation, local);
    let pos = [
        local[0] + shape.position[0] + state.node_offset[0],
        local[1] + shape.position[1] + state.node_offset[1],
        local[2] + shape.position[2] + state.node_offset[2],
    ];

    let start = &state.params.start;
    let lifetime = start.lifetime.evaluate(0.0, life_r).max(0.01);
    // 出发方向：圆盘法线（局部 +Z）经形状旋转。速度按出生种子求值
    // （源引擎里这两条流未取得，演示件用同因子并计数——同口径）。
    let dir = euler_rotate_deg(shape.rotation, [0.0, 0.0, 1.0]);
    let speed = start.speed.evaluate(0.0, r);
    let velocity = [dir[0] * speed, dir[1] * speed, dir[2] * speed];
    let sx = start.size.evaluate(0.0, sx_r);
    let sy = if start.size3d {
        start
            .size_y
            .as_ref()
            .map(|c| c.evaluate(0.0, sy_r))
            .unwrap_or(sx)
    } else {
        sx
    };
    let gravity = start.gravity_modifier.evaluate(0.0, r);

    let side = ParticleSide {
        rand: r,
        size: [sx, sy],
        gravity,
    };
    let particle = Particle::born(pos, velocity, lifetime);
    match ring_push(
        &mut state.pool,
        &mut state.cursor,
        state.params.ring_buffer_mode,
        state.params.max_particles as usize,
        particle,
    ) {
        RingPushVerdict::Appended => {
            state.side.push(side);
            state.born_total += 1;
        }
        RingPushVerdict::Replaced { index } => {
            state.side[index] = side;
            state.born_total += 1;
        }
        RingPushVerdict::Full => {
            state.full_total += 1;
        }
    }
}

/// 定位主发射器所在的 effect 记录。
fn effect_record(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value.pointer(&format!("/effects/{EMITTER_EFFECT}"))
}

/// 沿父链累计发射节点的世界平移。门：旋转近恒等（四元数虚部近零）、
/// 缩放恒 1——发射几何只支持平移链，别的形态响亮拒绝。
fn node_world_offset(value: &serde_json::Value) -> [f32; 3] {
    let nodes = effect_record(value)
        .and_then(|e| e.get("nodes"))
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("{EMITTER_EFFECT} 缺 nodes 数组"));
    let mut offset = [0.0f32; 3];
    let mut path = EMITTER_NODE.to_owned();
    loop {
        let node = nodes
            .iter()
            .find(|n| n.get("path").and_then(|p| p.as_str()) == Some(path.as_str()))
            .unwrap_or_else(|| panic!("nodes 里找不到路径 {path}"));
        let position = node
            .get("position")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("节点 {path} 缺 position"));
        for (i, slot) in offset.iter_mut().enumerate() {
            *slot += read_st(position, i);
        }
        // 旋转：四元数 (x, y, z, w)。近恒等即虚部近零。
        let rotation = node
            .get("rotation")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("节点 {path} 缺 rotation"));
        if rotation.len() != 4
            || read_st(rotation, 0).abs() > 1e-6
            || read_st(rotation, 1).abs() > 1e-6
            || read_st(rotation, 2).abs() > 1e-6
        {
            panic!("节点 {path} 带非平凡旋转：发射几何只支持平移链");
        }
        let scale = node
            .get("scale")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("节点 {path} 缺 scale"));
        if read_st(scale, 0) != 1.0 || read_st(scale, 1) != 1.0 || read_st(scale, 2) != 1.0 {
            panic!("节点 {path} 带非单位缩放：发射几何只支持平移链");
        }
        let parent = node.get("parent").and_then(|p| p.as_str());
        match parent {
            Some("") | None => break,
            Some(p) => path = p.to_owned(),
        }
    }
    offset
}

/// 停发盘点：档案里其余发射器的处置，分类计数一行。
///
/// 同 effect 的另外三条（子发射器目标）不接——子发射器是出生/死亡
/// 触发的独立系统，本单范围外；站点内嵌副本不重复跑（同一棵发射树）；
/// 天空/相机侧 effect 的发射器形状律未落（Donut/Hemisphere/Sphere/
/// Cone 族），逐条停发。
fn report_dispositions(effects: &Effects, main_record: &serde_json::Value) -> String {
    let total = effects.emitters.len();
    let site_dupes = effects
        .emitters
        .iter()
        .filter(|e| e.effect.starts_with("fx_env_site_006_rain_") && e.effect != EMITTER_EFFECT)
        .count();
    // 主 effect 里的子发射器目标（同一份发射树里的另外几条）。
    let sub_emitters = main_record
        .get("system")
        .and_then(|v| v.get("subEmitters"))
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|s| s.get("type").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut others = total as i64 - 1 - site_dupes as i64;
    // 主 effect 的容器节点（root）没有自己的发射参数，从「其余」里剔除。
    let root_emitters = effects
        .emitters
        .iter()
        .filter(|e| e.effect == EMITTER_EFFECT && e.node != EMITTER_NODE)
        .count() as i64;
    others -= root_emitters;
    format!(
        "停发盘点：发射器 {} 条 = 仿真 1 + 站点内嵌副本 {} 不重复 + 主 effect 同树 {} 条（子发射器目标 {}：不接）+ 其余 {} 条（形状律未落/无发射参数）；子发射器类型 {:?}",
        total, site_dupes, root_emitters, sub_emitters.len(), others, sub_emitters
    )
}

fn expect_float(floats: &serde_json::Map<String, serde_json::Value>, key: &str, want: f32) {
    let got = read_float(floats, key);
    if (got - want).abs() > 1e-6 {
        panic!("{EMITTER_NODE} 材质的 {key} 是 {got}，本族只认 {want}");
    }
}

fn read_float(floats: &serde_json::Map<String, serde_json::Value>, key: &str) -> f32 {
    floats
        .get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 floats.{key}")) as f32
}

fn read_color(material: &serde_json::Map<String, serde_json::Value>, key: &str) -> [f32; 4] {
    let items = material
        .get("colors")
        .and_then(|c| c.get(key))
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("{EMITTER_NODE} 材质缺 colors.{key}"));
    if items.len() != 4 {
        panic!("{EMITTER_NODE} 材质的 colors.{key} 不是 4 元");
    }
    [
        read_st(items, 0),
        read_st(items, 1),
        read_st(items, 2),
        read_st(items, 3),
    ]
}

fn read_st(items: &[serde_json::Value], index: usize) -> f32 {
    items
        .get(index)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("数组第 {index} 元不是数")) as f32
}
