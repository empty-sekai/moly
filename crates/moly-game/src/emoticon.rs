//! 表情域的呈现层：头顶表情件（sprite 与 particle 两族）的上屏。
//!
//! 数据面：`emoticons.json` 的 53 个条目——21 个 sprite 条目（节点树 +
//! start/loop/end 三段剪辑）与 32 个 particle 条目（85 条发射器记录，其中
//! 渲染器关/绘制模式未实现/节点链断/缺基础图的按记录停发计数）。
//! `unsupported` 段按原文具名记账（条数与 reason 归并），不猜不仿。
//!
//! 仿真侧只消费不重写：积分、重力、寿命、池的进出走 `moly-law::particle`
//! 的函数；发射环（率 + burst 游标 + 循环回绕复位）、形状采样、出生取值表
//! 按演示件的式子逐条转录。粒子档案的键形与律 schema 不同（`items` 而非
//! `effects`、发射器缺 startDelay/环形缓冲键），故逐条手解进本模块的参数
//! 结构，值类型（曲线/梯度）复用律的公开类型。
//!
//! Rest and dialogue submit already-selected presentation commands. Rest has
//! one source-ordered command queue and no second scenario clock or lottery.
//! An explicit MOLY_EMOTICON_ITEMS showcase remains available; without that
//! option it never injects unrelated emotes into the character's script.
//!
//! 挂账不静默：软粒子（无深度可读）、深度偏移、发光环、片表帧数之外的
//! 逐粒子第三轴量、`loopEndFlag` 与 `soundInput`（无对应消费域）——装载时
//! 逐项计数，收工报告具名。锚定与收场时序按演示件（锚挂骨骼、
//! `disposeDelaySeconds` 宽限）。

pub(crate) mod timeline;

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::asset::uuid::Uuid;
use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Affine3A;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_law::particle::value::{
    Curve, CurveKey, Gradient, GradientAlphaKey, GradientColorKey, MinMaxCurve, MinMaxGradient,
};
use moly_law::particle::{
    accumulate_rate, advance_lifetime, apply_gravity, euler_rotate_deg, integrate, DragSize,
    EmissionState, LifetimeVerdict, LimitVelocity, Particle, RingBufferMode, RotationOverLifetime,
};

use crate::npc::CharacterUnitId;

/// 表情档案的资产目录（提取产物布局，内联成 `moly://` 路径）。
const EMOTICON_DIR: &str = "moly://emoticons";

/// 重力常量：引擎侧 Vector3(0, -9.81, 0) 的口径，演示件同值。
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// 零缩放粒子会被背面剔除吞掉，尺寸下限同演示件。
const MIN_PARTICLE_SCALE: f32 = 0.0001;

/// 单帧 dt 钳制（切件/卡顿帧会把新生粒子一帧推出老远）。演示件同值。
const DT_CLAMP: f32 = 0.1;

/// 头部参考的本地偏移（挂点缺失时的兜底，演示件同值）。
const HEAD_LOCAL: [f32; 3] = [0.0, 0.6193, 0.0];

/// 出生抽签的确定性随机种子（固定值换可复算）。
const RNG_SEED: u64 = 0x656D_6F74_6500_0035;

/// 着色程序的稳定句柄：Startup 直接插入 `Assets<Shader>`。
const EMOTICON_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x656D_6F74_6963_6F6E_0000_0040_6D40_0001),
    PhantomData,
);

/// 表情材质：sprite 子矩形与粒子 billboard 共用一条片元链（`emoticon.wgsl`）。
///
/// 两族的逐粒子/逐槽 uv 差异（sprite 的子矩形、片表动画的格位、基础图
/// 旋转）全部折进 UV_0 顶点流，uniform 侧只留材质级的 `base_st`
/// （textureScaleOffset 的基础槽）。逐粒子颜色由 COLOR0 顶点流携带。
#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
pub struct EmoticonMaterial {
    #[uniform(0)]
    base_st: Vec4,
    #[texture(1)]
    #[sampler(2)]
    base_map: Handle<Image>,
}

impl Material for EmoticonMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(EMOTICON_SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(EMOTICON_SHADER.clone())
    }
    /// 混合态：SrcAlpha / OneMinusSrcAlpha（语料两族一致，装载时逐值断言）、
    /// 透明队列不写深度。源记录的 `_ZWrite=1`（sprite 族 45 条）与深度偏移
    /// 在本管线无对应开关，装载时计数挂账。双面由两条绕序的索引实现。
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

/// Material 管线注册：在 `app()` 里 DefaultPlugins 之后调用一次（同天空壳）。
pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<EmoticonMaterial>::default());
}

/// 表情档案的装载请求；解析完成即撤。
#[derive(Resource)]
pub(crate) struct EmoticonsHandle(Handle<JsonAsset>);

/// 解析完成、等贴图到齐的完整档案。
#[derive(Resource)]
pub(crate) struct EmoticonArchive {
    archive: Archive,
}

impl EmoticonArchive {
    /// 条目名在不在档案里（对话步的装载期核验用；缺条目出件侧另有
    /// 具名记账）。
    pub(crate) fn has(&self, name: &str) -> bool {
        self.archive.index.contains_key(name)
    }
}

/// 一张表情的绘制位：属于哪个实例（报告侧数「活着的有几个绘制实体」用，
/// 网格/材质句柄在实例自己的 draws 表里）。
#[derive(Component)]
pub(crate) struct EmoteDraw {
    instance: u64,
}

/// Startup：请求表情档案，内联表情着色程序。
pub(crate) fn load(
    mut commands: Commands,
    mut shaders: ResMut<Assets<Shader>>,
    server: Res<AssetServer>,
) {
    // 返回的句柄就是钉死的 EMOTICON_SHADER，丢弃以免 must_use 告警。
    let _ = shaders.insert(
        EMOTICON_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/emoticon.wgsl"),
            "moly_game/src/shaders/emoticon.wgsl".to_owned(),
        ),
    );
    commands.insert_resource(EmoticonsHandle(
        server.load::<JsonAsset>(AssetPath::from(format!("{EMOTICON_DIR}/emoticons.json"))),
    ));
    commands.init_resource::<RestEmoteRequests>();
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

    /// 全宽 u32（取混合输出的低 32 位，与 next_f32 的高 24 位不同位）：
    /// 自旋/限速族的种子杂凑吃全宽 u32。
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z as u32
    }
}

/// Explicit presentation showcase; normal Rest decisions live elsewhere.
#[derive(Resource)]
pub(crate) struct Driver {
    showcase: Option<Showcase>,
}

/// Commands already selected by the one Rest script cursor. Keeping one FIFO
/// preserves show/hide order; this is not a second script scheduler.
#[derive(Resource, Default)]
pub(crate) struct RestEmoteRequests(Vec<RestEmoteCommand>);

enum RestEmoteCommand {
    Show {
        npc: Entity,
        name: String,
        not_play_se: bool,
    },
    Hide {
        npc: Entity,
    },
}

impl RestEmoteRequests {
    pub(crate) fn show(&mut self, npc: Entity, name: String, not_play_se: bool) {
        self.0.push(RestEmoteCommand::Show {
            npc,
            name,
            not_play_se,
        });
    }
    pub(crate) fn hide(&mut self, npc: Entity) {
        self.0.push(RestEmoteCommand::Hide { npc });
    }
}

// ===== 档案数据面（手解进本模块的类型；值类型复用律的公开类型） =====

/// 装载计数与挂账：全部现算，报告行可复算。
#[derive(Default)]
struct LoadCounts {
    sprite_items: u32,
    particle_items: u32,
    records: u32,
    gate_disabled: u32,
    gate_unsupported: u32,
    gate_inactive: u32,
    gate_missing_texture: u32,
    gate_sub_driven: u32,
    simulated: u32,
    /// sprite 槽的节点链断（含祖先 active=false）。
    gate_inactive_sprite: u32,
    // ---- 挂账（能力在数据里、本管线没有对应开关或消费面，逐项计数）----
    acc_noise: u32,
    acc_custom_data: u32,
    acc_z_write: u32,
    acc_z_test_less: u32,
    acc_z_offset: u32,
    acc_soft_particles: u32,
    acc_rotation3d: u32,
    acc_sorting_order: u32,
    acc_uv_turns: u32,
    acc_alignment: u32,
    acc_renderer_pivot: u32,
    acc_stretch: u32,
    acc_sound_input: u32,
    acc_loop_end_flag: u32,
    acc_unsupported_text: u32,
    /// 自旋模块整块拒绝的发射器数（形状未按原生路径实现/提取面缺键；
    /// 拒绝后自旋冻结，发射器照跑）。
    acc_rol_refused: u32,
    /// 限速模块整块拒绝的发射器数（同上；拒绝后不钳速）。
    acc_lim_refused: u32,
}

/// 一个表情条目：两族共用节点树，族差异落在绘制槽上。
struct Item {
    name: String,
    family: Family,
    /// 节点树（粒子族在解析时做过 x 镜像）。
    nodes: Vec<PNode>,
    /// sprite 族的贴图表（名 → 定义）。
    sprites: Vec<SpriteDef>,
    /// 有 sprite 槽的节点下标（绘制位）。
    sprite_slots: Vec<usize>,
    /// sprite 族的三段剪辑；粒子族恒 None（语料实测：粒子条目 clips 空）。
    clips: Option<Clips>,
    /// 粒子族的发射器记录（含各道门分类）。
    emitters: Vec<EmitterDef>,
    /// item.textures 的装载句柄与贴图尺寸（sprite 子矩形按像素记，归一要尺寸）。
    textures: Vec<TexEntry>,
    /// 挂点（sprite→HeadRoot；粒子族按 view.anchor）。
    anchor: AnchorName,
    /// 粒子族：顶挂旋转走 Hips 参照的偏航单轴律（只门控旋转，与挂父
    /// 无关——世界/局部挂父由 simulationSpace 决定）。语料 13 条全在粒子族。
    keep_position: bool,
}

/// item.textures 的一行：句柄 + 采样归一用的尺寸。
struct TexEntry {
    name: String,
    width: f32,
    height: f32,
    handle: Handle<Image>,
}

enum Family {
    Sprite,
    Particle,
}

/// 挂点名（骨名一致）。
#[derive(Clone, Copy)]
enum AnchorName {
    Face,
    Spine,
    Hips,
    HeadRoot,
}

/// 节点：局部 TRS + sprite 槽字段（无槽的节点这些是默认值）。
struct PNode {
    /// 剪辑通道按这个匹配（None = 在动画器之上的节点，通道不驱动）。
    anim_path: Option<String>,
    parent: Option<usize>,
    active: bool,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
    /// sprite 槽：sprites 表下标。
    sprite: Option<usize>,
    /// sprite 槽的单双面（节点材质的 `_Cull`：0=双面、2=正面；无材质记录
    /// 的节点按演示件口径退化成双面）。语料 47 槽全部带材质且全为 2。
    sprite_double_side: bool,
    color: [f32; 4],
    renderer_enabled: bool,
}

/// sprite 贴图定义（sprites 表的一行）。
struct SpriteDef {
    rect: [f32; 4],
    pivot: [f32; 2],
    ppu: f32,
    texture: usize,
}

/// sprite 族三段剪辑。
struct Clips {
    start: Option<Clip>,
    loop_: Option<Clip>,
    end: Option<Clip>,
}

/// 一段剪辑：帧率自带，取帧 `round(t·rate)` 钳到 [0, frames-1]。
struct Clip {
    rate: f32,
    duration: f32,
    channels: Vec<Channel>,
}

struct Channel {
    anim_path: String,
    prop: ChannelProp,
    values: Vec<[f32; 3]>,
}

enum ChannelProp {
    Position,
    Scale,
    EulerAngles,
}

/// unsupported 段的一条记账：条目名 + 键 + 理由分列（报告归并用），
/// 原始记录可在源档案按条目名复查。
struct UnsupportedRec {
    item: String,
    /// attribute 的整哈希或对象类型名（记录里两选一）。
    key: String,
    reason: String,
}

/// 发射器的一条记录 + 门分类。
struct EmitterDef {
    node: usize,
    gate: Gate,
    params: Box<SimParams>,
    material: EmitterMat,
}

/// 停发门（分类口径与装载日志逐条对账）。
enum Gate {
    Simulated,
    /// 渲染器 enabled=false：数据明写的门。
    DisabledRenderer,
    /// 绘制模式无实现（语料 1 条 Stretch；模式名在分类时的记账行里）。
    UnsupportedRenderer,
    /// 发射节点（含祖先）active=false。
    InactiveNode,
    /// 基础图槽空（数组模式而数组贴图 null）。
    MissingTexture,
    /// 子发射目标：不自主播放，只由父粒子死亡触发。
    SubEmitterDriven,
}

/// 发射器材质槽（绘制需要的那几个量，其余在装载时断言或挂账）。
struct EmitterMat {
    base_map: Option<usize>,
    base_st: [f32; 4],
    /// `_BaseMapRotationEnabled` 开时的整圈数（0.25 = 90°）。
    uv_turns: f32,
    cull: f32,
    /// 视口占比截断的上下限（渲染器记录的 min/maxParticleSize；米制
    /// billboard 的全尺寸对视口全宽的占比，0.5 是运行时默认值）。
    min_frac: f32,
    max_frac: f32,
}

/// 一条发射器的仿真参数（手解；值类型复用律的）。
struct SimParams {
    duration: f32,
    looping: bool,
    sim_speed: f32,
    world_space: bool,
    max_particles: usize,
    rate_over_time: MinMaxCurve,
    bursts: Vec<BurstDef>,
    shape: ShapeDef,
    start_lifetime: MinMaxCurve,
    start_size: MinMaxCurve,
    start_size_y: Option<MinMaxCurve>,
    start_size_z: Option<MinMaxCurve>,
    start_color: MinMaxGradient,
    start_rotation: MinMaxCurve,
    /// 三轴旋转声明（X/Y 两轴 billboard 不画，但出生流照抽两次值——
    /// 抽签次数是式的组成部分）。
    rotation3d: bool,
    start_speed: MinMaxCurve,
    start_gravity: MinMaxCurve,
    color_over_lifetime: Option<MinMaxGradient>,
    size_over_lifetime: Option<MinMaxCurve>,
    size_over_lifetime_y: Option<MinMaxCurve>,
    /// 自旋律（引擎原生逐指令：模式 × 通用/烘制两路的采样 + 翻转因子；
    /// 逐粒子种子驱动，每帧 0 次流抽取）。None = 冻结在出生角。
    rotation_over_lifetime: Option<RotationOverLifetime>,
    /// 限速律（钳制段 + 拖拽段；语料拖拽恒 0，非零在构造处拒）。
    /// None = 不钳速。
    limit_velocity: Option<LimitVelocity>,
    vol: Option<VolDef>,
    texture_sheet: Option<TexSheetDef>,
    /// 死亡触发的子发射（目标 = 发射器在 item.emitters 里的下标）。
    sub_emitters: Vec<SubEmitterDef>,
}

struct BurstDef {
    time: f32,
    count: MinMaxCurve,
    /// 用户口径；0 = 无限轮。
    cycle_count: u32,
    repeat_interval: f32,
    probability: f32,
}

struct VolDef {
    x: MinMaxCurve,
    y: MinMaxCurve,
    z: MinMaxCurve,
    speed_modifier: MinMaxCurve,
}

struct TexSheetDef {
    tiles_x: u32,
    tiles_y: u32,
    frame_over_time: MinMaxCurve,
}

struct SubEmitterDef {
    target: usize,
    probability: f32,
}

/// 形状模块。语料六种全部有实现（none/Circle/Sphere/Cone/SingleSidedEdge/BoxEdge），
/// 其余形状键出现即响亮拒绝（fail-closed，不猜几何）。
struct ShapeDef {
    kind: ShapeKind,
    radius: f32,
    radius_thickness: f32,
    /// 弧与锥角（度）。
    arc_deg: f32,
    angle_deg: f32,
    position: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 3],
}

enum ShapeKind {
    None,
    Circle,
    Sphere,
    Cone,
    SingleSidedEdge,
    BoxEdge,
}

// ===== JSON 读取助手（定向取键，不整档解释） =====

use serde_json::Value;

fn f_field(obj: &Value, key: &str, default: f32) -> f32 {
    obj.get(key)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .unwrap_or(default)
}

fn i_field(obj: &Value, key: &str, default: u32) -> u32 {
    obj.get(key)
        .and_then(Value::as_i64)
        .map(|v| v as u32)
        .unwrap_or(default)
}

fn b_field(obj: &Value, key: &str, default: bool) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn s_field(obj: &Value, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn vec2_field(obj: &Value, key: &str, default: [f32; 2]) -> [f32; 2] {
    match obj.get(key).and_then(Value::as_array) {
        Some(a) if a.len() >= 2 => [
            a[0].as_f64().unwrap_or(default[0] as f64) as f32,
            a[1].as_f64().unwrap_or(default[1] as f64) as f32,
        ],
        _ => default,
    }
}

fn vec3_field(obj: &Value, key: &str, default: [f32; 3]) -> [f32; 3] {
    match obj.get(key).and_then(Value::as_array) {
        Some(a) if a.len() >= 3 => [
            a[0].as_f64().unwrap_or(default[0] as f64) as f32,
            a[1].as_f64().unwrap_or(default[1] as f64) as f32,
            a[2].as_f64().unwrap_or(default[2] as f64) as f32,
        ],
        _ => default,
    }
}

fn vec4_field(obj: &Value, key: &str, default: [f32; 4]) -> [f32; 4] {
    match obj.get(key).and_then(Value::as_array) {
        Some(a) if a.len() >= 4 => [
            a[0].as_f64().unwrap_or(default[0] as f64) as f32,
            a[1].as_f64().unwrap_or(default[1] as f64) as f32,
            a[2].as_f64().unwrap_or(default[2] as f64) as f32,
            a[3].as_f64().unwrap_or(default[3] as f64) as f32,
        ],
        _ => default,
    }
}

fn quat_field(obj: &Value) -> Quat {
    let a = obj.get("rotation").and_then(Value::as_array);
    match a {
        Some(a) if a.len() >= 4 => Quat::from_xyzw(
            a[0].as_f64().unwrap_or(0.0) as f32,
            a[1].as_f64().unwrap_or(0.0) as f32,
            a[2].as_f64().unwrap_or(0.0) as f32,
            a[3].as_f64().unwrap_or(1.0) as f32,
        ),
        _ => Quat::IDENTITY,
    }
}

/// 空曲线（twoCurves 模式缺某一侧键表时的兜底：乘子 1、无键）。
fn empty_curve() -> Curve {
    Curve {
        multiplier: 1.0,
        keys: Vec::new(),
    }
}

/// 解一根键曲线。斜率缺省（null）按 0 处理——片表那一族实测如此；
/// 加权键按位解析（求值走律的加权 Bezier 路），激活位的权重缺失即拒
/// （缺会左右结果的键不当默认值）。
fn parse_curve(obj: &Value) -> Curve {
    Curve {
        multiplier: f_field(obj, "multiplier", 1.0),
        keys: obj
            .get("keys")
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .map(|k| {
                        let weighted = i_field(k, "weightedMode", 0) as u8;
                        let inert = f32::from_bits(0x3eaa_aaab);
                        let in_weight = if weighted & 1 != 0 {
                            k.get("inWeight")
                                .and_then(Value::as_f64)
                                .unwrap_or_else(|| panic!("加权键缺 inWeight（入权位激活）：{k}"))
                                as f32
                        } else {
                            inert
                        };
                        let out_weight = if weighted & 2 != 0 {
                            k.get("outWeight")
                                .and_then(Value::as_f64)
                                .unwrap_or_else(|| panic!("加权键缺 outWeight（出权位激活）：{k}"))
                                as f32
                        } else {
                            inert
                        };
                        CurveKey {
                            time: f_field(k, "time", 0.0),
                            value: f_field(k, "value", 0.0),
                            in_slope: f_field(k, "inSlope", 0.0),
                            out_slope: f_field(k, "outSlope", 0.0),
                            weighted_mode: weighted,
                            in_weight,
                            out_weight,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// 模式标签不认识就响亮拒绝：值域是数据决定的，猜一个默认会静默错整族。
fn parse_min_max_curve(obj: &Value) -> MinMaxCurve {
    let mode = s_field(obj, "mode").unwrap_or_else(|| panic!("粒子值缺 mode：{obj}"));
    match mode.as_str() {
        "constant" => MinMaxCurve::Constant(f_field(obj, "value", 0.0)),
        "twoConstants" => MinMaxCurve::TwoConstants {
            min: f_field(obj, "min", 0.0),
            max: f_field(obj, "max", 0.0),
        },
        "curve" => MinMaxCurve::Curve {
            multiplier: f_field(obj, "multiplier", 1.0),
            max: parse_curve(obj),
        },
        "twoCurves" => MinMaxCurve::TwoCurves {
            multiplier: f_field(obj, "multiplier", 1.0),
            min: obj
                .get("minKeys")
                .map(parse_curve)
                .unwrap_or_else(empty_curve),
            max: obj
                .get("maxKeys")
                .map(parse_curve)
                .unwrap_or_else(empty_curve),
        },
        other => panic!("未建模的粒子值模式：{other}"),
    }
}

fn parse_gradient(obj: &Value) -> Gradient {
    Gradient {
        color_keys: obj
            .get("colorKeys")
            .and_then(Value::as_array)
            .map(|ks| {
                ks.iter()
                    .map(|k| GradientColorKey {
                        time: f_field(k, "time", 0.0),
                        color: vec3_field(k, "color", [1.0, 1.0, 1.0]),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        alpha_keys: obj
            .get("alphaKeys")
            .and_then(Value::as_array)
            .map(|ks| {
                ks.iter()
                    .map(|k| GradientAlphaKey {
                        time: f_field(k, "time", 0.0),
                        alpha: f_field(k, "alpha", 1.0),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn parse_min_max_gradient(obj: &Value) -> MinMaxGradient {
    let mode = s_field(obj, "mode").unwrap_or_else(|| panic!("粒子色缺 mode：{obj}"));
    match mode.as_str() {
        "color" => MinMaxGradient::Color(vec4_field(obj, "color", [1.0, 1.0, 1.0, 1.0])),
        "gradient" => {
            MinMaxGradient::Gradient(obj.get("gradient").map(parse_gradient).unwrap_or_default())
        }
        "twoColors" => MinMaxGradient::TwoColors {
            min: vec4_field(obj, "min", [1.0, 1.0, 1.0, 1.0]),
            max: vec4_field(obj, "max", [1.0, 1.0, 1.0, 1.0]),
        },
        "twoGradients" => MinMaxGradient::TwoGradients {
            min: obj
                .get("minGradient")
                .map(parse_gradient)
                .unwrap_or_default(),
            max: obj
                .get("maxGradient")
                .map(parse_gradient)
                .unwrap_or_default(),
        },
        "randomColor" => {
            // 随机色把随机因子当梯度时刻求值（maxGradient 缺席时按 gradient）。
            let g = obj
                .get("maxGradient")
                .or_else(|| obj.get("gradient"))
                .map(parse_gradient)
                .unwrap_or_default();
            MinMaxGradient::RandomColor(g)
        }
        other => panic!("未建模的粒子色模式：{other}"),
    }
}

// ===== 档案装配（两份 JSON → 本模块的类型 + 贴图句柄） =====

/// Update：解析表情档案。装载失败响亮 panic；结构不认识同样响亮（值域由
/// 数据决定，静默取默认会把整族错掉）。解析完直接请求全部贴图。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    docs: Res<Assets<JsonAsset>>,
    handle: Option<Res<EmoticonsHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("表情档案装载失败：{err:?}");
    }
    let Some(doc) = docs.get(&handle.0) else {
        return;
    };
    let value: Value =
        serde_json::from_str(&doc.0).unwrap_or_else(|err| panic!("表情档案不是合法 JSON：{err}"));

    let mut counts = LoadCounts::default();
    let mut items = Vec::new();
    let mut unsupported_text: Vec<UnsupportedRec> = Vec::new();
    let entries = value
        .get("items")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("表情档案缺 items 对象"));
    for (name, entry) in entries {
        items.push(build_item(
            name,
            entry,
            &server,
            &mut counts,
            &mut unsupported_text,
        ));
    }
    // 摘要交叉核对：条目数与 unsupported 总数两处独立数，不等就说明解析漏了。
    let summary_items = value.pointer("/summary/items").and_then(Value::as_u64);
    let summary_unsupported = value
        .pointer("/summary/unsupported")
        .and_then(Value::as_array)
        .map(|a| a.len());
    if summary_items != Some(items.len() as u64) {
        panic!(
            "条目数与档案摘要不符：解析 {}，摘要 {:?}",
            items.len(),
            summary_items
        );
    }
    if summary_unsupported != Some(unsupported_text.len()) {
        panic!(
            "unsupported 计数与档案摘要不符：解析 {}，摘要 {:?}",
            unsupported_text.len(),
            summary_unsupported
        );
    }
    counts.acc_unsupported_text = unsupported_text.len() as u32;
    let index: HashMap<String, usize> = items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.name.clone(), i))
        .collect();
    commands.insert_resource(EmoticonArchive {
        archive: Archive {
            items,
            index,
            counts,
            unsupported: unsupported_text,
        },
    });
    commands.remove_resource::<EmoticonsHandle>();
}

/// 单条目装配：节点树（粒子族镜像）+ sprite 表 + 剪辑 + 发射器（含门分类）。
fn build_item(
    name: &str,
    entry: &Value,
    server: &AssetServer,
    counts: &mut LoadCounts,
    unsupported_text: &mut Vec<UnsupportedRec>,
) -> Item {
    // unsupported 按原文具名记账：条目名 + 键 + 理由（不猜语义、不改写，
    // 原始记录在源档案里按条目名可复查），供装载报告归并。
    for u in entry
        .get("unsupported")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = match u.get("attribute") {
            Some(a) => format!("attribute {a}"),
            None => s_field(u, "type")
                .map(|t| format!("type {t}"))
                .unwrap_or_else(|| "unkeyed".to_owned()),
        };
        unsupported_text.push(UnsupportedRec {
            item: name.to_owned(),
            key,
            reason: s_field(u, "reason").unwrap_or_default(),
        });
    }
    let view = entry.get("view").cloned().unwrap_or(Value::Null);
    let kind = s_field(&view, "kind").unwrap_or_else(|| panic!("{name} 缺 view.kind"));
    let mirror = kind == "particle";
    let family = if mirror {
        counts.particle_items += 1;
        Family::Particle
    } else {
        counts.sprite_items += 1;
        Family::Sprite
    };
    if s_field(&view, "soundInput").is_some() {
        counts.acc_sound_input += 1;
    }
    // 动画器上的收尾标记（与 end 段并存的本域无消费面，按字段挂账）。
    if entry.pointer("/animator/loopEndFlag").is_some() {
        counts.acc_loop_end_flag += 1;
    }

    // ---- 贴图表：名与文件名两套下标（材质只存文件名，sprite 表只存名）----
    let mut textures: Vec<TexEntry> = Vec::new();
    let mut tex_by_name: HashMap<String, usize> = HashMap::new();
    let mut tex_by_file: HashMap<String, usize> = HashMap::new();
    for t in entry
        .get("textures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let tname = s_field(t, "name").unwrap_or_default();
        let file = s_field(t, "file").unwrap_or_default();
        let handle = server.load::<Image>(AssetPath::from(format!("{EMOTICON_DIR}/{file}")));
        tex_by_name.insert(tname.clone(), textures.len());
        tex_by_file.insert(file, textures.len());
        textures.push(TexEntry {
            name: tname,
            width: i_field(t, "width", 1) as f32,
            height: i_field(t, "height", 1) as f32,
            handle,
        });
    }

    // ---- 节点树 ----
    let node_values = entry
        .get("nodes")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{name} 缺 nodes"));
    let mut path_index: HashMap<String, usize> = HashMap::new();
    let mut nodes: Vec<PNode> = Vec::new();
    for (i, n) in node_values.iter().enumerate() {
        let path = s_field(n, "path").unwrap_or_default();
        path_index.insert(path.clone(), i);
        // flip 用负缩放实现，会翻绕序；语料 47 个 sprite 槽全 False，出现即拒。
        if b_field(n, "flipX", false) || b_field(n, "flipY", false) {
            panic!("{name}/{path}：flipX/flipY 未建模（语料全 False）");
        }
        let position = vec3_field(n, "position", [0.0, 0.0, 0.0]);
        let rotation = quat_field(n);
        // 粒子族的本地系做过 x 镜像（与骨架同一套反射右手系；sprite 族留在
        // 件自己的相机面向系里）。位置取 −x，四元数取 (x,−y,−z,w)。
        let (position, rotation) = if mirror {
            (
                Vec3::new(-position[0], position[1], position[2]),
                Quat::from_xyzw(rotation.x, -rotation.y, -rotation.z, rotation.w),
            )
        } else {
            (Vec3::from_array(position), rotation)
        };
        // 父节点必须在表里且排在前（父先子后）；找不到就响亮拒绝。
        let parent = match s_field(n, "parent") {
            None => None,
            Some(p) => Some(
                path_index
                    .get(&p)
                    .copied()
                    .unwrap_or_else(|| panic!("{name}/{path}：父节点 {p} 不在节点表里")),
            ),
        };
        nodes.push(PNode {
            anim_path: s_field(n, "animationPath"),
            parent,
            active: b_field(n, "active", true),
            position,
            rotation,
            scale: Vec3::from_array(vec3_field(n, "scale", [1.0, 1.0, 1.0])),
            sprite: None,
            sprite_double_side: true,
            color: vec4_field(n, "color", [1.0, 1.0, 1.0, 1.0]),
            renderer_enabled: b_field(n, "rendererEnabled", true),
        });
    }

    // ---- sprite 表 ----
    let mut sprites: Vec<SpriteDef> = Vec::new();
    let mut sprite_index: HashMap<String, usize> = HashMap::new();
    for (sname, spec) in entry
        .get("sprites")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let texture = spec
            .get("texture")
            .and_then(Value::as_str)
            .and_then(|t| tex_by_name.get(t).copied())
            .unwrap_or_else(|| panic!("{name} 的 sprite {sname} 找不到贴图条目"));
        sprite_index.insert(sname.clone(), sprites.len());
        sprites.push(SpriteDef {
            rect: vec4_field(spec, "rect", [0.0, 0.0, 1.0, 1.0]),
            pivot: [
                spec.get("pivot")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5) as f32,
                spec.get("pivot")
                    .and_then(Value::as_array)
                    .and_then(|a| a.get(1))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5) as f32,
            ],
            ppu: f_field(spec, "pixelsToUnits", 100.0),
            texture,
        });
    }

    // sprite 槽：带 sprite 字段且渲染器开着的节点（关着的不画、计数）；
    // 节点链断（含祖先 active=false）的同样不画（祖先的可见性沿链生效）。
    let mut sprite_slots = Vec::new();
    let mut acc_disabled_sprite = 0u32;
    for (i, n) in node_values.iter().enumerate() {
        if let Some(sname) = s_field(n, "sprite") {
            if !nodes[i].renderer_enabled {
                acc_disabled_sprite += 1;
                continue;
            }
            if !chain_active(&nodes, i) {
                counts.gate_inactive_sprite += 1;
                continue;
            }
            nodes[i].sprite = Some(
                sprite_index
                    .get(&sname)
                    .copied()
                    .unwrap_or_else(|| panic!("{name} 的节点 {i} 引用了不存在的 sprite {sname}")),
            );
            // 绕序的单双面由节点材质的 `_Cull` 定（0=双面、2=正面朝观察者）；
            // 无材质记录的节点按演示件口径退化成双面。其它值响亮拒绝——
            // 绕序装反的画面是「整片消失」，比报错难查。
            nodes[i].sprite_double_side = match n.get("material") {
                None => true,
                Some(m) => {
                    let floats = m.get("floats").cloned().unwrap_or(Value::Null);
                    let c = f_field(&floats, "_Cull", 2.0);
                    if c == 0.0 {
                        true
                    } else if c == 2.0 {
                        false
                    } else {
                        panic!("{name} 的槽节点 {i}：sprite 材质的 _Cull {c} 未建模")
                    }
                }
            };
            if let Some(m) = n.get("material") {
                let floats = m.get("floats").cloned().unwrap_or(Value::Null);
                // sprite 族的着色器把背面剔除在 pass 里写死成关（材质里的
                // `_Cull` 不生效），绕序按双面装配。
                if f_field(&floats, "_ZWrite", 0.0) != 0.0 {
                    counts.acc_z_write += 1;
                }
                if f_field(&floats, "_ZTest", 4.0) == 2.0 {
                    counts.acc_z_test_less += 1;
                }
                if f_field(&floats, "_ZOffset", 0.0) != 0.0 {
                    counts.acc_z_offset += 1;
                }
            }
            if i_field(n, "sortingOrder", 0) != 0 {
                counts.acc_sorting_order += 1;
            }
            sprite_slots.push(i);
        }
    }
    if acc_disabled_sprite > 0 {
        info!("[emoticon] {name}：渲染器关着的 sprite 槽 {acc_disabled_sprite} 个（不画）");
    }

    // ---- 剪辑（sprite 族；粒子族语料实测为空）----
    let mut clips = None;
    if let Some(clips_obj) = entry.get("clips").and_then(Value::as_object) {
        if !clips_obj.is_empty() {
            clips = Some(build_clips(name, clips_obj));
        }
    }

    // ---- 发射器（粒子族）：第一遍建记录与门，第二遍回填子发射目标 ----
    let mut emitters: Vec<EmitterDef> = Vec::new();
    let mut emitter_by_node: HashMap<String, usize> = HashMap::new();
    let records = entry
        .get("particles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for r in &records {
        counts.records += 1;
        let renderer = r.get("renderer").cloned().unwrap_or(Value::Null);
        let system = r.get("system").cloned().unwrap_or(Value::Null);
        let node_path = s_field(r, "node").unwrap_or_default();
        let node = path_index
            .get(&node_path)
            .copied()
            .unwrap_or_else(|| panic!("{name}/{node_path}：发射节点不在节点表里"));
        if system.is_null() {
            panic!("{name}/{node_path}：发射器记录缺 system");
        }
        let (params, material) = build_emitter(name, &system, &renderer, &tex_by_file, counts);
        // 门分类（次序与演示件一致：渲染器开关 → 绘制模式 → 节点链 → 贴图槽）。
        let gate = if !b_field(&renderer, "enabled", true) {
            counts.gate_disabled += 1;
            Gate::DisabledRenderer
        } else {
            let mode = s_field(&renderer, "renderMode").unwrap_or_else(|| "Billboard".into());
            if mode != "Billboard" {
                counts.gate_unsupported += 1;
                info!("[emoticon] {name}/{node_path}：发射器绘制模式 {mode} 未建模（不画）");
                if mode == "Stretch" {
                    counts.acc_stretch += 1;
                }
                Gate::UnsupportedRenderer
            } else if !chain_active(&nodes, node) {
                counts.gate_inactive += 1;
                Gate::InactiveNode
            } else if material.base_map.is_none() {
                counts.gate_missing_texture += 1;
                Gate::MissingTexture
            } else {
                // 渲染器旁挂账项（数据在、本管线的 billboard 不消费）。
                if f_field(&renderer, "alignment", 0.0) != 0.0 {
                    counts.acc_alignment += 1;
                }
                if vec3_field(&renderer, "pivot", [0.0, 0.0, 0.0]) != [0.0, 0.0, 0.0] {
                    counts.acc_renderer_pivot += 1;
                }
                if i_field(&renderer, "sortingOrder", 0) != 0 {
                    counts.acc_sorting_order += 1;
                }
                Gate::Simulated
            }
        };
        emitter_by_node.insert(node_path, emitters.len());
        emitters.push(EmitterDef {
            node,
            gate,
            params: Box::new(params),
            material,
        });
    }
    // 第二遍：子发射目标回填（死亡触发；目标下标在同一包的记录表里）。
    for (i, r) in records.iter().enumerate() {
        for sub in r
            .pointer("/system/subEmitters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if s_field(sub, "type").as_deref() != Some("death") {
                panic!("{name}：未建模的子发射触发型 {sub}");
            }
            // 继承（出生参数从父粒子合成）的规则没有可读来源，声明了即拒——
            // 拿错尺寸错颜色的粒子画出来什么都看不出来。
            if let Some(inh) = sub.get("inherit").and_then(Value::as_object) {
                if inh.values().any(|v| v.as_bool() == Some(true)) {
                    panic!("{name}：子发射记录声明了继承（未建模）：{sub}");
                }
            }
            let target = sub
                .get("emitter")
                .and_then(Value::as_str)
                .and_then(|p| emitter_by_node.get(p).copied())
                .unwrap_or_else(|| panic!("{name}：子发射目标 {sub} 不在本包记录表里"));
            emitters[i].params.sub_emitters.push(SubEmitterDef {
                target,
                probability: f_field(sub, "emitProbability", 1.0),
            });
        }
    }
    // 子发射目标标记（要等全部记录建完）。目标已被前几道门摘掉的保持原门。
    let mut sub_driven: Vec<usize> = Vec::new();
    for e in &emitters {
        for sub in &e.params.sub_emitters {
            if matches!(emitters[sub.target].gate, Gate::Simulated) {
                sub_driven.push(sub.target);
            }
        }
    }
    for target in sub_driven {
        emitters[target].gate = Gate::SubEmitterDriven;
    }
    // 逐项累计（`counts` 跨条目共用，这里若用赋值会只剩最后一个条目的数）。
    counts.gate_sub_driven += emitters
        .iter()
        .filter(|e| matches!(e.gate, Gate::SubEmitterDriven))
        .count() as u32;
    counts.simulated += emitters
        .iter()
        .filter(|e| matches!(e.gate, Gate::Simulated))
        .count() as u32;

    let anchor = match s_field(&view, "anchor").as_deref() {
        Some("Face") => AnchorName::Face,
        Some("Spine") => AnchorName::Spine,
        Some("Hips") => AnchorName::Hips,
        None if !mirror => AnchorName::HeadRoot,
        None => AnchorName::Hips,
        Some(other) => panic!("{name}：未建模的挂点 {other}"),
    };
    Item {
        name: name.to_owned(),
        family,
        nodes,
        sprites,
        sprite_slots,
        clips,
        emitters,
        textures,
        anchor,
        keep_position: b_field(&view, "keepPosition", false),
    }
}

/// 祖先链上有 active=false 就整条不画（发射节点含祖先，与演示件同口径）。
fn chain_active(nodes: &[PNode], from: usize) -> bool {
    let mut cursor = Some(from);
    while let Some(i) = cursor {
        if !nodes[i].active {
            return false;
        }
        cursor = nodes[i].parent;
    }
    true
}

/// 三段剪辑装配。通道按 animationPath 匹配节点；通道属性出现未建模的
/// 即响亮拒绝。
fn build_clips(name: &str, clips: &serde_json::Map<String, Value>) -> Clips {
    let build = |key: &str| -> Option<Clip> {
        clips.get(key).map(|c| Clip {
            rate: f_field(c, "rate", 60.0),
            duration: f_field(c, "duration", 0.0),
            channels: c
                .get("channels")
                .and_then(Value::as_array)
                .map(|chs| {
                    chs.iter()
                        .map(|ch| {
                            let prop = match s_field(ch, "property").as_deref() {
                                Some("position") => ChannelProp::Position,
                                Some("scale") => ChannelProp::Scale,
                                Some("eulerAngles") => ChannelProp::EulerAngles,
                                Some(other) => panic!("{name}：未建模的通道属性 {other}"),
                                None => panic!("{name}：通道缺 property"),
                            };
                            Channel {
                                anim_path: s_field(ch, "path").unwrap_or_default(),
                                prop,
                                values: ch
                                    .get("values")
                                    .and_then(Value::as_array)
                                    .map(|vs| vs.iter().map(frame_vec3).collect())
                                    .unwrap_or_default(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    };
    let end = build("end");
    Clips {
        start: build("start"),
        loop_: build("loop"),
        end,
    }
}

/// 通道帧值：3 元数组原样（缺项按 0）。
fn frame_vec3(v: &Value) -> [f32; 3] {
    match v.as_array() {
        Some(a) if a.len() >= 3 => [
            a[0].as_f64().unwrap_or(0.0) as f32,
            a[1].as_f64().unwrap_or(0.0) as f32,
            a[2].as_f64().unwrap_or(0.0) as f32,
        ],
        _ => [0.0, 0.0, 0.0],
    }
}

/// 一条发射器的仿真参数与材质槽。
fn build_emitter(
    name: &str,
    system: &Value,
    renderer: &Value,
    tex_by_file: &HashMap<String, usize>,
    counts: &mut LoadCounts,
) -> (SimParams, EmitterMat) {
    let start = system.get("start").cloned().unwrap_or(Value::Null);
    let size3d = b_field(&start, "size3D", false);
    // ---- 挂账项（模块在位而未建模/未消费）----
    if system.get("noise").is_some() {
        counts.acc_noise += 1;
    }
    if system.get("customData").is_some() {
        counts.acc_custom_data += 1;
    }
    if b_field(&start, "rotation3D", false) {
        counts.acc_rotation3d += 1;
    }
    // ---- 形状 ----
    let shape_obj = system.get("shape").cloned().unwrap_or(Value::Null);
    let kind = match s_field(&shape_obj, "type").as_deref() {
        None | Some("None") => ShapeKind::None,
        Some("Circle") => ShapeKind::Circle,
        Some("Sphere") => ShapeKind::Sphere,
        Some("Cone") => ShapeKind::Cone,
        Some("SingleSidedEdge") => ShapeKind::SingleSidedEdge,
        Some("BoxEdge") => ShapeKind::BoxEdge,
        Some(other) => panic!("{name}：未建模的发射形状 {other}"),
    };
    let shape = ShapeDef {
        kind,
        radius: f_field(&shape_obj, "radius", 1.0),
        radius_thickness: f_field(&shape_obj, "radiusThickness", 1.0),
        arc_deg: f_field(&shape_obj, "arc", 360.0),
        angle_deg: f_field(&shape_obj, "angle", 25.0),
        position: vec3_field(&shape_obj, "position", [0.0, 0.0, 0.0]),
        rotation: vec3_field(&shape_obj, "rotation", [0.0, 0.0, 0.0]),
        scale: vec3_field(&shape_obj, "scale", [1.0, 1.0, 1.0]),
    };
    // ---- 发射 ----
    let emission = system.get("emission");
    let rate_over_time = emission
        .and_then(|e| e.get("rateOverTime").map(parse_min_max_curve))
        .unwrap_or(MinMaxCurve::Constant(0.0));
    let bursts = emission
        .and_then(|e| e.get("bursts"))
        .and_then(Value::as_array)
        .map(|bs| {
            bs.iter()
                .map(|b| BurstDef {
                    time: f_field(b, "time", 0.0),
                    count: b
                        .get("count")
                        .map(parse_min_max_curve)
                        .unwrap_or(MinMaxCurve::Constant(0.0)),
                    cycle_count: i_field(b, "cycleCount", 1),
                    repeat_interval: f_field(b, "repeatInterval", 0.01),
                    probability: f_field(b, "probability", 1.0),
                })
                .collect()
        })
        .unwrap_or_default();
    // ---- 生命期模块 ----
    let sol = system.get("sizeOverLifetime");
    let size_over_lifetime = sol.and_then(|s| s.get("curve").map(parse_min_max_curve));
    let size_over_lifetime_y = sol
        .filter(|s| b_field(s, "separateAxes", false))
        .and_then(|s| s.get("y").map(parse_min_max_curve));
    let vol = system.get("velocityOverLifetime").map(|v| VolDef {
        x: v.get("x")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        y: v.get("y")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        z: v.get("z")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        speed_modifier: v
            .get("speedModifier")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(1.0)),
    });
    // 限速：构造拒绝（separateAxis / 通用曲线幅值 / 非常数或非零拖拽——
    // 非零拖拽还叠加「尺寸数组取出生还是当前」的未解读选择）时模块整块
    // 摘除并计数，发射器照跑（不钳速）。
    let limit_velocity = system.get("limitVelocity").and_then(|l| {
        let magnitude = l.get("magnitude").map(parse_min_max_curve)?;
        let drag = l
            .get("drag")
            .filter(|d| !d.is_null())
            .map(parse_min_max_curve);
        match LimitVelocity::from_parts(
            b_field(l, "separateAxis", false),
            &magnitude,
            f_field(l, "dampen", 0.0),
            drag.as_ref(),
            l.get("multiplyDragBySize").and_then(Value::as_bool),
            l.get("multiplyDragByVelocity").and_then(Value::as_bool),
        ) {
            Ok(law) => Some(law),
            Err(reason) => {
                counts.acc_lim_refused += 1;
                info!("[emoticon] {name}：limitVelocity 模块拒绝（不钳速）：{reason}");
                None
            }
        }
    });
    let texture_sheet = system.get("textureSheet").map(|t| TexSheetDef {
        tiles_x: i_field(t, "tilesX", 1),
        tiles_y: i_field(t, "tilesY", 1),
        frame_over_time: t
            .get("frameOverTime")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
    });

    // 片表真的分格时才进采样链（1×1 的片表声明不进，与演示件同判）。
    let sheet_tiled = texture_sheet
        .as_ref()
        .is_some_and(|t| t.tiles_x > 1 || t.tiles_y > 1);
    let params = SimParams {
        duration: f_field(system, "duration", 1.0),
        looping: b_field(system, "looping", false),
        sim_speed: f_field(system, "simulationSpeed", 1.0),
        world_space: s_field(system, "simulationSpace").as_deref() == Some("World"),
        max_particles: i_field(system, "maxParticles", 1000) as usize,
        rate_over_time,
        bursts,
        shape,
        start_lifetime: start
            .get("lifetime")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        start_size: start
            .get("size")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        start_size_y: if size3d {
            start.get("sizeY").map(parse_min_max_curve)
        } else {
            None
        },
        start_size_z: if size3d {
            start.get("sizeZ").map(parse_min_max_curve)
        } else {
            None
        },
        start_color: start
            .get("color")
            .map(parse_min_max_gradient)
            .unwrap_or(MinMaxGradient::Color([1.0, 1.0, 1.0, 1.0])),
        start_rotation: start
            .get("rotation")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        rotation3d: b_field(&start, "rotation3D", false),
        start_speed: start
            .get("speed")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        start_gravity: start
            .get("gravityModifier")
            .map(parse_min_max_curve)
            .unwrap_or(MinMaxCurve::Constant(0.0)),
        color_over_lifetime: system.get("colorOverLifetime").map(parse_min_max_gradient),
        size_over_lifetime,
        size_over_lifetime_y,
        // 自旋：构造拒绝（separateAxes 开而 x/y 键缺 = 提取面缺键）时模块
        // 整块摘除并计数，发射器照跑（自旋冻结在出生角）。
        rotation_over_lifetime: system.get("rotationOverLifetime").and_then(|r| {
            let curve = r.get("curve").map(parse_min_max_curve)?;
            let x = r.get("x").map(parse_min_max_curve);
            let y = r.get("y").map(parse_min_max_curve);
            match RotationOverLifetime::from_parts(
                b_field(r, "separateAxes", false),
                x.as_ref(),
                y.as_ref(),
                &curve,
            ) {
                Ok(law) => Some(law),
                Err(reason) => {
                    counts.acc_rol_refused += 1;
                    info!("[emoticon] {name}：rotationOverLifetime 模块拒绝（自旋冻结）：{reason}");
                    None
                }
            }
        }),
        limit_velocity,
        vol,
        texture_sheet,
        sub_emitters: Vec::new(),
    };

    // ---- 材质槽 ----
    let material = renderer.get("material").cloned().unwrap_or(Value::Null);
    let floats = material.get("floats").cloned().unwrap_or(Value::Null);
    let textures_obj = material.get("textures").cloned().unwrap_or(Value::Null);
    let base_mode = f_field(&floats, "_BaseMapMode", 0.0) as u32;
    let base_key = match base_mode {
        0 => "_BaseMap",
        1 => "_BaseMap2DArray",
        other => panic!("{name}：未建模的基础图模式 {other}"),
    };
    let base_map = textures_obj
        .get(base_key)
        .and_then(Value::as_str)
        .and_then(|f| tex_by_file.get(f).copied());
    let tso = material
        .get("textureScaleOffset")
        .cloned()
        .unwrap_or(Value::Null);
    let uv_turns = if f_field(&floats, "_BaseMapRotationEnabled", 0.0) != 0.0 {
        counts.acc_uv_turns += 1;
        f_field(&floats, "_BaseMapRotation", 0.0)
    } else {
        0.0
    };
    // 基础图旋转的轴心与逐粒子选择器：折算前提是轴心在 (0.5, 0.5) 且
    // 旋转量不含逐粒子项，出现即拒（不静默画错 uv）。
    if uv_turns != 0.0 {
        if f_field(&floats, "_BaseMapRotationCoord", 0.0) != 0.0 {
            panic!("{name}：基础图旋转带逐粒子选择器，未建模");
        }
        if vec2_field(&floats, "_BaseMapRotationOffsets", [0.0, 0.0]) != [0.0, 0.0] {
            panic!("{name}：基础图旋转轴心偏移非零，未建模");
        }
    }
    if f_field(&floats, "_SoftParticlesEnabled", 0.0) != 0.0 {
        counts.acc_soft_particles += 1;
    }
    if f_field(&floats, "_ZWrite", 0.0) != 0.0 {
        counts.acc_z_write += 1;
    }
    if f_field(&floats, "_ZTest", 4.0) == 2.0 {
        counts.acc_z_test_less += 1;
    }
    if f_field(&floats, "_ZOffset", 0.0) != 0.0 {
        counts.acc_z_offset += 1;
    }
    let mat = EmitterMat {
        base_map,
        base_st: vec4_field(&tso, "_BaseMap", [1.0, 1.0, 0.0, 0.0]),
        uv_turns,
        cull: {
            // CULL_SIDE：0=双面、1=背面、2=正面（朝相机）。语料只有 0/2；
            // 出现 1（只画背面）时单绕序取反向即可，但先响亮拒绝——
            // 绕序反了的画面是「整片消失」，比报错难查得多。
            let c = f_field(&floats, "_Cull", 2.0);
            if c != 0.0 && c != 2.0 {
                panic!("{name}：未建模的 _Cull {c}");
            }
            c
        },
        min_frac: f_field(renderer, "minParticleSize", 0.0),
        max_frac: f_field(renderer, "maxParticleSize", 0.0),
    };
    // 采样链的折算前提：基础图旋转与片表动画都折进 UV_0 顶点流时，材质侧的
    // ST 与「先缩放后旋转」的次序必须恒等才能等价（源链是 sheet → 旋转 → ST，
    // 折算后是 旋转 → sheet → ST）。非恒等的组合出现即拒，不静默画错 UV。
    // 片表挂账只在真的分格时成立（1×1 的片表声明不进采样链，与演示件同判）。
    let identity_st = mat.base_st == [1.0, 1.0, 0.0, 0.0];
    if uv_turns != 0.0 && !identity_st {
        panic!("{name}：基础图旋转与非恒等 ST 并存，采样链折算不成立");
    }
    if sheet_tiled && !identity_st {
        panic!("{name}：片表动画与非恒等 ST 并存，采样链折算不成立");
    }
    (params, mat)
}

// ===== 档案容器与待机编排解析 =====

/// 解析完成的档案本体（`EmoticonArchive` 资源的内芯）。
struct Archive {
    items: Vec<Item>,
    /// 条目名 → items 下标（编排事件按名取件）。
    index: HashMap<String, usize>,
    counts: LoadCounts,
    /// unsupported 段的原文记账。
    unsupported: Vec<UnsupportedRec>,
}

/// Update：档案与贴图全部到齐即建驱动，并出装载报告。
///
/// 报告行可从档案复算：分族计数、发射器记录的门分类、逐项挂账、
/// unsupported 归并（键与理由分列，原文整段保留在档案里）。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    archive: Option<Res<EmoticonArchive>>,
    server: Res<AssetServer>,
    spawned: Option<Res<Spawned>>,
) {
    let Some(archive) = archive else { return };
    if spawned.is_some() {
        return;
    }
    for item in &archive.archive.items {
        for tex in &item.textures {
            match server.load_state(&tex.handle) {
                LoadState::Failed(err) => {
                    panic!("表情贴图装载失败：{}/{}：{err:?}", item.name, tex.name)
                }
                s if s.is_loaded() => {}
                _ => return,
            }
        }
    }
    let c = &archive.archive.counts;
    info!(
        "[emoticon] 档案 {} 项：sprite {} · particle {}；发射器记录 {}（仿真 {} · 渲染器关 {} · 绘制模式未实现 {} · 节点链断 {} · 缺基础图 {} · 子发射驱动 {}）；sprite 槽节点链断 {}",
        archive.archive.items.len(),
        c.sprite_items,
        c.particle_items,
        c.records,
        c.simulated,
        c.gate_disabled,
        c.gate_unsupported,
        c.gate_inactive,
        c.gate_missing_texture,
        c.gate_sub_driven,
        c.gate_inactive_sprite,
    );
    info!(
        "[emoticon] 挂账（能力在数据里、本管线无对应开关或消费面，逐项计数）：noise {} · customData {} · rotation3D {} · alignment {} · 渲染器 pivot {} · sortingOrder {} · ZWrite {} · ZTest=2 {} · ZOffset {} · 软粒子 {} · uv 旋转 {} · stretch 绘制 {} · loopEndFlag {} · soundInput {} · 自旋模块拒 {} · 限速模块拒 {}",
        c.acc_noise,
        c.acc_custom_data,
        c.acc_rotation3d,
        c.acc_alignment,
        c.acc_renderer_pivot,
        c.acc_sorting_order,
        c.acc_z_write,
        c.acc_z_test_less,
        c.acc_z_offset,
        c.acc_soft_particles,
        c.acc_uv_turns,
        c.acc_stretch,
        c.acc_loop_end_flag,
        c.acc_sound_input,
        c.acc_rol_refused,
        c.acc_lim_refused,
    );
    // unsupported 归并：键 × 理由 →（条数、去重条目名），按条数降序（键是
    // attribute 的整哈希或对象类型名；原始记录在源档案按条目名可复查）。
    let mut groups: Vec<(&str, &str, usize, Vec<&str>)> = Vec::new();
    for rec in &archive.archive.unsupported {
        match groups
            .iter_mut()
            .find(|(k, r, _, _)| *k == rec.key && *r == rec.reason)
        {
            Some((_, _, n, items)) => {
                *n += 1;
                if !items.contains(&rec.item.as_str()) {
                    items.push(rec.item.as_str());
                }
            }
            None => groups.push((
                rec.key.as_str(),
                rec.reason.as_str(),
                1,
                vec![rec.item.as_str()],
            )),
        }
    }
    groups.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    let census = groups
        .iter()
        .map(|(k, r, n, items)| format!("{k}（{r}）×{n} [{}]", items.join("、")))
        .collect::<Vec<_>>()
        .join(" · ");
    info!(
        "[emoticon] unsupported 记账 {} 条：{census}",
        c.acc_unsupported_text
    );
    commands.insert_resource(Emotes::default());
    commands.insert_resource(Driver {
        showcase: Showcase::from_env(&archive.archive),
    });
    // 一次性：此后靠本标记短路，Emotes/Driver 不再被逐帧重建。
    commands.insert_resource(Spawned);
}

/// `spawn_when_ready` 已建驱动的标记（天空链同款的一次性门）。
#[derive(Resource)]
pub(crate) struct Spawned;

// ===== 运行时数据面（实例 / 相机 / 挂点 / 相位机） =====

/// 一帧的相机读数（面相机旋转、屏幕占比截断、出场投影共用）。
struct CamInfo {
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// 纵向视场（弧度）。
    fov_y: f32,
    /// 横纵比（无视口时 0，屏幕占比截断会因此自然让位）。
    aspect: f32,
}

/// 表情件的相位（演示件剪辑机的五态；Idle 即回收）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Start,
    Loop,
    Live,
    End,
    Closing,
}

/// 一名 NPC 的四个挂点骨（缺骨的槽为 None，运行时落头部参考点）。
struct MountSet {
    face: Option<Entity>,
    spine: Option<Entity>,
    hips: Option<Entity>,
    head_root: Option<Entity>,
}

/// 一条发射器的运行时（参数本体在 `Item.emitters` 里，这里只有状态与计数）。
struct EmitterRun {
    /// `item.emitters` 的下标（参数与绘制位的回查键）。
    def: usize,
    age: f32,
    pending: EmissionState,
    /// burst 游标（每发记录一发已发的轮数）。
    bursts: Vec<u32>,
    /// 循环发射器的整轮计数（回绕那帧清 burst 游标）。
    loop_n: i64,
    particles: Vec<PRt>,
    peak: usize,
    /// 屏幕占比截断改过尺寸的粒子数（累计）。
    clamped: u32,
    spawned_total: u64,
    /// 出生速率攒满一颗但池满时的拒发数（累计）。
    refused: u64,
    /// 本帧死亡的粒子世界位（死亡路由用，帧末清空）。
    deaths: Vec<Vec3>,
}

impl EmitterRun {
    fn new(def: usize) -> EmitterRun {
        EmitterRun {
            def,
            age: 0.0,
            pending: EmissionState::default(),
            bursts: Vec::new(),
            loop_n: 0,
            particles: Vec::new(),
            peak: 0,
            clamped: 0,
            spawned_total: 0,
            refused: 0,
            deaths: Vec::new(),
        }
    }
}

/// 一颗粒子的运行时。寿命/位置/速度本体在律的 `Particle` 里（积分、重力、
/// 寿命判定全部走律）；这里只持律面之外的逐粒子量。
struct PRt {
    core: Particle,
    /// 这颗粒子自己的稳定随机因子（出生抽一次）：生命期里所有「每颗
    /// 稳定」的曲线求值共用它，速度与重力系数的出生取值也用它。
    r: f32,
    /// 逐粒子 u32 种子（出生抽一次，取值表末尾）：自旋/限速族引擎原生
    /// 路的全部杂凑因子是它的纯函数，终生恒定、每帧 0 次流抽取。
    seed: u32,
    /// 三轴自旋角（弧度；出生时 z 轴来自 startRotation，x/y 恒 0——
    /// 出生取值表对三轴声明照抽两次但只留 z，公告板也只画 z）。
    /// 此后按 rotationOverLifetime 的角速度累加。
    rot: [f32; 3],
    /// 出生尺寸两轴（米；sizeOverLifetime 的乘子另算）。
    size_x: f32,
    size_y: f32,
    /// 出生重力系数（0 = 不施加重力）。
    gravity: f32,
    /// 逐粒子色（colorOverLifetime 每帧整组覆写；无模块则恒出生色）。
    color: [f32; 4],
    /// 片表帧（无模块恒 0）。
    frame: u32,
    /// 世界位（局部空间粒子是「节点本地坐标 + 本帧节点世界系」的绘制量）。
    world: Vec3,
    /// 屏幕占比截断与节点缩放折算后的两轴有效尺寸。
    sx: f32,
    sy: f32,
}

/// 一个绘制位：网格与材质句柄成对持有（回收时三处同撤）。
struct DrawSlot {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<EmoticonMaterial>,
    kind: DrawKind,
}

#[derive(Clone, Copy)]
enum DrawKind {
    /// `item.sprite_slots` 的下标（sprite 族）。
    Sprite(usize),
    /// `item.emitters` 的下标（粒子族仿真发射器）。
    Emitter(usize),
}

/// 一块四边形的顶点暂存（角序 [TL,TR,BL,BR]；单面绕序从正面看 CCW，
/// 双面补反向三角——源侧 `_Cull` 0=双面、2=正面）。
#[derive(Default)]
struct SlotMesh {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl SlotMesh {
    fn clear(&mut self) {
        self.positions.clear();
        self.uvs.clear();
        self.colors.clear();
        self.indices.clear();
    }

    fn quad(
        &mut self,
        corners: &[[f32; 3]; 4],
        uv: &[[f32; 2]; 4],
        color: [f32; 4],
        double_side: bool,
    ) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(corners);
        self.uvs.extend_from_slice(uv);
        self.colors.extend([color; 4]);
        self.indices
            .extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);
        if double_side {
            self.indices.extend_from_slice(&[
                base,
                base + 1,
                base + 2,
                base + 1,
                base + 3,
                base + 2,
            ]);
        }
    }
}

/// 一个在屏的表情件实例（每 NPC 至多一个）。
struct Instance {
    key: u64,
    item: usize,
    npc: Entity,
    timeline_rotation_reference: Option<Entity>,
    /// 节点 TRS 的运行副本（剪辑通道写这里；顶挂旋转写 top）。
    node_pos: Vec<Vec3>,
    node_scale: Vec<Vec3>,
    node_rot: Vec<Quat>,
    /// 剪辑通道的 animPath → 节点下标。
    channel_nodes: HashMap<String, usize>,
    /// 根节点（parent=None）下标。
    top: Option<usize>,
    mounts: MountSet,
    phase: Phase,
    clock: f32,
    end_clock: f32,
    /// 出件秒数到点自动收件（演示件 play(sec)：null 不自动收）。
    hide_at: Option<f32>,
    emitters: Vec<EmitterRun>,
    /// `item.emitters` 下标 → emitters 槽位（死亡路由的回查）。
    emitter_index: HashMap<usize, usize>,
    draws: Vec<DrawSlot>,
    buffers: Vec<SlotMesh>,
    rng: Rng,
    /// 节点世界仿形（发射/死亡路由读 A 拍，绘制读 B 拍，见 `compute_worlds`）。
    worlds: Vec<Affine3A>,
}

/// 在屏实例的账本（报告行现算）。
#[derive(Default, Resource)]
pub(crate) struct Emotes {
    instances: Vec<Instance>,
    next_key: u64,
    /// 累计出件数（含同 NPC 换件）。
    shown_total: u64,
    /// 编排点名了、档案里没有的条目名（逐次记录，报告归并）。
    missing: Vec<String>,
}

/// 环境变量驱动的展示轮播（名册空转时兜底出件，冒烟可复算）。
struct Showcase {
    items: Vec<usize>,
    idx: usize,
    show_secs: f32,
}

impl Showcase {
    /// `MOLY_EMOTICON_ITEMS`：逗号分隔的条目名（点名不在档案里的条目即拒绝）；
    /// 未指定不轮播。`MOLY_EMOTICON_SHOW_SECS`：出件秒数（缺省 6）。
    fn from_env(archive: &Archive) -> Option<Showcase> {
        let items = match std::env::var("MOLY_EMOTICON_ITEMS") {
            Ok(list) => list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|name| {
                    archive.index.get(name).copied().unwrap_or_else(|| {
                        panic!("MOLY_EMOTICON_ITEMS 点名了档案里没有的条目：{name}")
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => return None,
        };
        if items.is_empty() {
            panic!("展示清单为空：档案里两族都没有条目");
        }
        let show_secs = std::env::var("MOLY_EMOTICON_SHOW_SECS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(6.0);
        Some(Showcase {
            idx: items.len() - 1,
            items,
            show_secs,
        })
    }
}

/// 演示件 `num`：非有限值取缺省（曲线求值的口径，0 是常态缺省）。
fn num(v: f32, d: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        d
    }
}

/// 深度优先找挂点骨（先父后子；第一个同名命中即挂，与演示件
/// `getObjectByName` 同判）。返回顺序对应 [Head, Spine, Hips, HeadRoot]。
fn find_mounts(root: Entity, children: &Query<&Children>, names: &Query<&Name>) -> MountSet {
    const WANTED: [&str; 4] = ["Head", "Spine", "Hips", "HeadRoot"];
    let mut found = [None; 4];
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(name) = names.get(entity) {
            if let Some(i) = WANTED.iter().position(|w| name.as_str() == *w) {
                if found[i].is_none() {
                    found[i] = Some(entity);
                }
            }
        }
        if found.iter().all(|f| f.is_some()) {
            break;
        }
        if let Ok(children) = children.get(entity) {
            // 先序遍历：子代逆序入栈，弹出即正序。
            for child in children.iter().rev() {
                stack.push(child);
            }
        }
    }
    MountSet {
        face: found[0],
        spine: found[1],
        hips: found[2],
        head_root: found[3],
    }
}

/// 条目的挂点实体（缺骨 → None，由 `mount_frame` 落头部参考点）。
fn mount_entity(anchor: AnchorName, mounts: &MountSet) -> Option<Entity> {
    match anchor {
        AnchorName::Face => mounts.face,
        AnchorName::Spine => mounts.spine,
        AnchorName::Hips => mounts.hips,
        AnchorName::HeadRoot => mounts.head_root,
    }
}

/// 挂点骨的世界系；缺骨时落头部参考点（NPC 世界系里的固定偏移、旋转恒等，
/// 与演示件的兜底同口径——兜底锚不随骨骼动）。
fn mount_frame(
    mount: Option<Entity>,
    globals: &Query<&GlobalTransform>,
    npc_global: Affine3A,
) -> Affine3A {
    mount
        .and_then(|entity| globals.get(entity).ok())
        .map(|g| g.affine())
        .unwrap_or_else(|| {
            Affine3A::from_translation(npc_global.transform_point3(Vec3::from(HEAD_LOCAL)))
        })
}

/// 相位对应的剪辑段。
fn clip_of<'a>(phase: Phase, clips: Option<&'a Clips>) -> Option<&'a Clip> {
    let clips = clips?;
    match phase {
        Phase::Start => clips.start.as_ref(),
        Phase::Loop => clips.loop_.as_ref(),
        Phase::End | Phase::Closing => clips.end.as_ref(),
        _ => None,
    }
}

/// 撤一个实例：绘制实体、网格、材质三处同撤。
fn clear_instance(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<EmoticonMaterial>,
    emotes: &mut Emotes,
    idx: usize,
) {
    let inst = emotes.instances.swap_remove(idx);
    for slot in &inst.draws {
        commands.entity(slot.entity).despawn();
        meshes.remove(&slot.mesh);
        materials.remove(&slot.material);
    }
}

/// 收件请求（演示件 `hide()`）：idle/end 不再受理；有 end 段进 end，
/// 没有进 closing；closing 重入重开宽限钟。
fn request_hide(inst: &mut Instance, clips: Option<&Clips>) {
    if inst.phase == Phase::Idle || inst.phase == Phase::End {
        return;
    }
    inst.phase = if clips.is_some_and(|c| c.end.is_some()) {
        Phase::End
    } else {
        Phase::Closing
    };
    inst.end_clock = 0.0;
}

/// 剪辑通道的施加：取帧 `round(t·rate)` 钳到 [0, 帧数-1]，eulerAngles 按
/// 度转弧度、XYZ 序（演示件 `setFromEuler` 缺省序）。animPath 对不上的
/// 通道跳过（演示件同判）。
fn apply_clip(inst: &mut Instance, clip: Option<&Clip>, t: f32) {
    let Some(clip) = clip else { return };
    for ch in &clip.channels {
        let Some(&node) = inst.channel_nodes.get(&ch.anim_path) else {
            continue;
        };
        if ch.values.is_empty() {
            continue;
        }
        let idx = (t * num(clip.rate, 60.0))
            .round()
            .clamp(0.0, (ch.values.len() - 1) as f32) as usize;
        let v = ch.values[idx];
        match ch.prop {
            ChannelProp::Position => inst.node_pos[node] = v.into(),
            ChannelProp::Scale => inst.node_scale[node] = v.into(),
            ChannelProp::EulerAngles => {
                inst.node_rot[node] = Quat::from_euler(
                    EulerRot::XYZ,
                    v[0].to_radians(),
                    v[1].to_radians(),
                    v[2].to_radians(),
                );
            }
        }
    }
}

/// 节点世界仿形：父先子后（解析侧保证父节点排在子节点前），根的父是
/// 挂点世界系。发射/死亡路由读「A 拍」（本帧顶旋转 + 上一帧剪辑 TRS），
/// 绘制读「B 拍」（本帧剪辑 TRS）——两拍各调一次，参数不同。
fn compute_worlds(item: &Item, inst: &mut Instance, anchor: &Affine3A) {
    for i in 0..item.nodes.len() {
        let node = &item.nodes[i];
        let local = Affine3A::from_scale_rotation_translation(
            inst.node_scale[i],
            inst.node_rot[i],
            inst.node_pos[i],
        );
        inst.worlds[i] = match node.parent {
            None => *anchor * local,
            Some(p) => inst.worlds[p] * local,
        };
    }
}

/// 长度平方为零的向量原样返回（演示件形状出口的宽容归一；严格归一在
/// 零向量上会出 NaN）。
fn lenient_normalize(v: Vec3) -> Vec3 {
    let len_sq = v.length_squared();
    if len_sq > 0.0 {
        v / len_sq.sqrt()
    } else {
        v
    }
}

/// 演示件的单向量朝向构造：局部 +Z 指向 forward、隐式上轴 (0,1,0)。
/// 退化分支（forward 平行上轴）按演示件微扰再归一。
fn look_rotation(forward: Vec3) -> Quat {
    let up = Vec3::Y;
    let mut z = if forward.length_squared() > 0.0 {
        forward.normalize()
    } else {
        Vec3::Z
    };
    let mut x = up.cross(z);
    if x.length_squared() == 0.0 {
        if up.z.abs() == 1.0 {
            z.x += 0.0001;
        } else {
            z.z += 0.0001;
        }
        z = z.normalize();
        x = up.cross(z);
    }
    x = x.normalize();
    let y = z.cross(x);
    // 旋转矩阵三列 (x, y, z) 即基——与演示件 setFromRotationMatrix 同构。
    Quat::from_mat3(&Mat3::from_cols(x, y, z))
}

/// 粒子族的 keepPosition 顶挂单轴偏航（演示件 `particleYawRad`，参照
/// Hips 挂点）：只比 XZ 平面上的方向，退化时 0。返回值带负号成弧度。
fn particle_yaw_rad(anchor_pos: Vec3, anchor_quat: Quat, cam_pos: Vec3) -> f32 {
    let f = anchor_quat * Vec3::Z;
    let dx = cam_pos.x - anchor_pos.x;
    let dz = cam_pos.z - anchor_pos.z;
    let len = ((dx * dx + dz * dz) * (f.x * f.x + f.z * f.z)).sqrt();
    let deg = if len >= 1e-15 {
        ((dx * f.x + dz * f.z) / len).clamp(-1.0, 1.0).acos() * 57.29578
    } else {
        0.0
    };
    let signed = if (dx * f.z - dz * f.x) < 0.0 {
        -deg
    } else {
        deg
    };
    signed * -0.017453292
}

/// 屏幕占比截断的比例（演示件 `_screenClampRatio`）：粒子的全尺寸对
/// 该深度处视口的世界宽度（横向）之比钳到 [min, max] 占比，两轴同乘同一
/// 比例（各轴独立裁会毁掉作者写的长宽比）。相机在背后、深度/宽度非有限
/// 或占比上下限全关时返回 1（不截）。
fn screen_clamp_ratio(cam: &CamInfo, world: Vec3, size: f32, min_frac: f32, max_frac: f32) -> f32 {
    if min_frac <= 0.0 && max_frac <= 0.0 {
        return 1.0;
    }
    if !(size > 1e-6) {
        return 1.0;
    }
    let depth = cam.forward.dot(world - cam.pos);
    if !depth.is_finite() || depth <= 1e-4 {
        return 1.0;
    }
    let width = 2.0 * depth * cam.aspect * (cam.fov_y / 2.0).tan();
    if !width.is_finite() || width <= 1e-6 {
        return 1.0;
    }
    let mut target = size;
    if min_frac > 0.0 {
        target = target.max(min_frac * width);
    }
    if max_frac > 0.0 {
        target = target.min(max_frac * width);
    }
    let ratio = target / size;
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}

/// 出件（演示件 `playEmoticon` + `EmoticonView` 的装配）：同 NPC 换件先撤
/// 旧的；挂点骨解析；发射器运行时只给仿真与子发射驱动两类门；绘制位按
/// 条目序建（sprite 槽 + 仿真发射器）；初拍 `applyClip(phase, 0)`；出场
/// 日志带锚点世界位与 NDC 投影（可从相机读数复算）。
#[allow(clippy::too_many_arguments)]
fn show_emote(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<EmoticonMaterial>,
    globals: &Query<&GlobalTransform>,
    children: &Query<&Children>,
    names: &Query<&Name>,
    cameras: &Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    emotes: &mut Emotes,
    archive: &Archive,
    item_name: &str,
    npc: Entity,
    show_seconds: f32,
) {
    let Some(&item_idx) = archive.index.get(item_name) else {
        // 缺条目具名记账（同名单次入账，报告行按归并后的名单数）。
        if !emotes.missing.iter().any(|m| m == item_name) {
            emotes.missing.push(item_name.to_owned());
        }
        return;
    };
    let item = &archive.items[item_idx];
    if let Some(old) = emotes.instances.iter().position(|i| i.npc == npc) {
        clear_instance(commands, meshes, materials, emotes, old);
    }
    let key = emotes.next_key;
    emotes.next_key += 1;
    emotes.shown_total += 1;

    let mut channel_nodes = HashMap::new();
    for (i, n) in item.nodes.iter().enumerate() {
        if let Some(path) = &n.anim_path {
            channel_nodes.insert(path.clone(), i);
        }
    }
    let top = item.nodes.iter().position(|n| n.parent.is_none());
    let npc_global = globals
        .get(npc)
        .map(|g| g.affine())
        .unwrap_or(Affine3A::IDENTITY);
    let mounts = find_mounts(npc, children, names);

    // 发射器运行时：仿真门与子发射驱动门（其余四类门在装载侧已摘）。
    let mut emitters = Vec::new();
    let mut emitter_index = HashMap::new();
    for (i, e) in item.emitters.iter().enumerate() {
        if matches!(e.gate, Gate::Simulated | Gate::SubEmitterDriven) {
            emitter_index.insert(i, emitters.len());
            emitters.push(EmitterRun::new(i));
        }
    }

    // 绘制位（条目序，确定性遍历）：sprite 槽的材质是恒等 ST + 槽位自己的
    // 贴图；发射器位带材质记录的 ST 与基础图。
    let mut draws = Vec::new();
    for &slot in &item.sprite_slots {
        let node = &item.nodes[slot];
        let Some(sprite_idx) = node.sprite else {
            continue;
        };
        let texture = item.sprites[sprite_idx].texture;
        let mesh_handle = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        let material_handle = materials.add(EmoticonMaterial {
            base_st: Vec4::new(1.0, 1.0, 0.0, 0.0),
            base_map: item.textures[texture].handle.clone(),
        });
        let entity = commands
            .spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                EmoteDraw { instance: key },
            ))
            .id();
        draws.push(DrawSlot {
            entity,
            mesh: mesh_handle,
            material: material_handle,
            kind: DrawKind::Sprite(slot),
        });
    }
    for (i, e) in item.emitters.iter().enumerate() {
        if !matches!(e.gate, Gate::Simulated | Gate::SubEmitterDriven) {
            continue;
        }
        let Some(texture) = e.material.base_map else {
            panic!("{} 的发射器 {i} 过了缺基础图门却没有贴图", item.name);
        };
        let mesh_handle = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        let material_handle = materials.add(EmoticonMaterial {
            base_st: Vec4::from_array(e.material.base_st),
            base_map: item.textures[texture].handle.clone(),
        });
        let entity = commands
            .spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                EmoteDraw { instance: key },
            ))
            .id();
        draws.push(DrawSlot {
            entity,
            mesh: mesh_handle,
            material: material_handle,
            kind: DrawKind::Emitter(i),
        });
    }

    let clips = item.clips.as_ref();
    let phase = if clips.is_some_and(|c| c.start.is_some()) {
        Phase::Start
    } else if clips.is_some_and(|c| c.loop_.is_some()) {
        Phase::Loop
    } else {
        Phase::Live
    };
    let mut inst = Instance {
        key,
        item: item_idx,
        npc,
        timeline_rotation_reference: None,
        node_pos: item.nodes.iter().map(|n| n.position).collect(),
        node_scale: item.nodes.iter().map(|n| n.scale).collect(),
        node_rot: item.nodes.iter().map(|n| n.rotation).collect(),
        channel_nodes,
        top,
        mounts,
        phase,
        clock: 0.0,
        end_clock: 0.0,
        hide_at: if show_seconds > 0.0 {
            Some(show_seconds.max(0.0))
        } else {
            None
        },
        emitters,
        emitter_index,
        draws,
        buffers: Vec::new(),
        rng: Rng(RNG_SEED ^ key.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        worlds: vec![Affine3A::IDENTITY; item.nodes.len()],
    };
    inst.buffers = inst.draws.iter().map(|_| SlotMesh::default()).collect();
    // 初拍：演示件 play() 末尾的 applyClip(clips[phase], 0)。
    apply_clip(&mut inst, clip_of(phase, clips), 0.0);

    // 出场日志：锚点世界位 + NDC 投影（带名可推导；挂点骨找到与否
    // 分别记，缺骨兜底不是静默路径）。
    let mount = mount_entity(item.anchor, &inst.mounts);
    let anchor = mount_frame(mount, globals, npc_global);
    let anchor_pos = Vec3::from(anchor.translation);
    let mount_note = if mount.is_some() {
        "骨"
    } else {
        "缺骨兜底"
    };
    let family = match item.family {
        Family::Sprite => "sprite",
        Family::Particle => "particle",
    };
    let projection = cameras
        .iter()
        .find_map(|(g, _, cam)| cam.world_to_ndc(g, anchor_pos));
    match projection {
        Some(p) => info!(
            "[emoticon] 出件 npc={:?} item={}（{family}）锚点={mount_note}({:.3},{:.3},{:.3}) ndc=({:.2},{:.2})",
            npc, item.name, anchor_pos.x, anchor_pos.y, anchor_pos.z, p.x, p.y
        ),
        None => info!(
            "[emoticon] 出件 npc={:?} item={}（{family}）锚点={mount_note}({:.3},{:.3},{:.3}) ndc=（相机读不到）",
            npc, item.name, anchor_pos.x, anchor_pos.y, anchor_pos.z
        ),
    }
    emotes.instances.push(inst);
}

// ===== 仿真面（形状采样 / 出生 / 发射环 / 实例推进） =====

/// 半径采样（演示件 sampleShapeRadius）：实心度 `thickness` 把半径分成
/// 内核与外环两段，`power` 次幂控制体密度（圆 2、球 3）。
fn sample_shape_radius(radius: f32, thickness: f32, power: f32, u: f32) -> f32 {
    let inner = (1.0 - num(thickness, 0.0)).clamp(0.0, 1.0).powf(power);
    radius * (inner + (1.0 - inner) * u).powf(1.0 / power)
}

/// 形状自身变换（演示件 applyShapeTransform）：缩放先乘位置与方向 →
/// YXZ 序欧拉旋转（律的 `euler_rotate_deg` 同式）→ 平移只加给位置。
fn apply_shape_transform(shape: &ShapeDef, pos: &mut Vec3, dir: &mut Vec3) {
    *pos *= Vec3::from(shape.scale);
    *dir *= Vec3::from(shape.scale);
    *pos = Vec3::from(euler_rotate_deg(shape.rotation, pos.to_array()));
    *dir = Vec3::from(euler_rotate_deg(shape.rotation, dir.to_array()));
    *pos += Vec3::from(shape.position);
}

/// 形状采样（演示件 emitFrom 的逐形状公式 + 公共尾巴）。语料六种：
/// none（点发射就是它的语义）/Sphere/Circle/Cone/SingleSidedEdge/BoxEdge。
/// 尾巴：形状变换后方向归一，零向量（零缩放轴塌缩）落 +Z 而不是「无速度」。
fn sample_shape(shape: &ShapeDef, rng: &mut Rng) -> (Vec3, Vec3) {
    let radius = num(shape.radius, 0.0);
    let arc = num(shape.arc_deg, 360.0).to_radians();
    let mut pos = Vec3::ZERO;
    let mut dir = Vec3::Z;
    match shape.kind {
        ShapeKind::None => {}
        ShapeKind::Sphere => {
            let u = rng.next_f32() * std::f32::consts::TAU;
            let v = (2.0 * rng.next_f32() - 1.0).acos();
            dir = Vec3::new(v.sin() * u.cos(), v.sin() * u.sin(), v.cos());
            pos = dir * sample_shape_radius(radius, shape.radius_thickness, 3.0, rng.next_f32());
        }
        ShapeKind::Circle => {
            // 两次取值**先角度后半径**（顺序是式的组成部分）；角度是
            // `arc·u`，没有「arc 为 0 就当整圈」这回事。
            let a = rng.next_f32() * arc;
            let ur = rng.next_f32();
            dir = Vec3::new(a.cos(), a.sin(), 0.0);
            pos = dir * sample_shape_radius(radius, shape.radius_thickness, 2.0, ur);
        }
        ShapeKind::Cone => {
            // 底圆在局部 XY 平面，主轴 +Z；angle=0 时方向恰为 (0,0,1)。
            // 这一支才有「arc 为 0 当整圈」的回退。
            let a = rng.next_f32()
                * (if arc != 0.0 {
                    arc
                } else {
                    std::f32::consts::TAU
                });
            let rr = sample_shape_radius(radius, shape.radius_thickness, 2.0, rng.next_f32());
            pos = Vec3::new(a.cos() * rr, a.sin() * rr, 0.0);
            let spread = num(shape.angle_deg, 0.0).to_radians();
            dir = Vec3::new(a.cos() * spread.sin(), a.sin() * spread.sin(), spread.cos());
        }
        ShapeKind::SingleSidedEdge => {
            // 半长是 radius，方向恒 +Y。
            pos = Vec3::new((rng.next_f32() * 2.0 - 1.0) * radius, 0.0, 0.0);
            dir = Vec3::Y;
        }
        ShapeKind::BoxEdge => {
            // 单位立方体的 12 条棱：一根轴连续取值，另两根钉在端点；
            // 方向恒 +Z。实际尺寸来自 shape.scale（公共尾巴）。
            let axis = (rng.next_f32() * 3.0).floor() as usize;
            let coin = |rng: &mut Rng| if rng.next_f32() < 0.5 { -0.5 } else { 0.5 };
            let c0 = coin(rng);
            let c1 = coin(rng);
            let r = rng.next_f32() - 0.5;
            pos = Vec3::from(match axis {
                0 => [r, c0, c1],
                1 => [c0, r, c1],
                _ => [c0, c1, r],
            });
            dir = Vec3::Z;
        }
    }
    apply_shape_transform(shape, &mut pos, &mut dir);
    if dir.length_squared() > 1e-30 {
        dir = dir.normalize();
    } else {
        dir = Vec3::Z;
    }
    (pos, dir)
}

/// 一颗粒子的出生（演示件 spawn 的转录）。`origin` 为 Some 时是死亡子发射
/// 的「搬到父粒子处」分支；None 时按世界/局部空间走节点整条变换。
/// 出生取值表逐项各取一次值；速度与重力系数共用这颗粒子自己的稳定因子。
fn spawn_particle(
    run: &mut EmitterRun,
    params: &SimParams,
    node_world: &Affine3A,
    origin: Option<Vec3>,
    rng: &mut Rng,
) {
    let (mut pos, mut dir) = sample_shape(&params.shape, rng);
    // 同一节点内容的换手另一半：出生点/方向 x 取负（节点链在装载时已共轭）。
    pos.x = -pos.x;
    dir.x = -dir.x;
    let (_, node_quat, node_translation) = node_world.to_scale_rotation_translation();
    match origin {
        Some(origin) => {
            // 只取节点旋转，平移由 origin 给；局部空间再把节点平移减回来
            // （渲染时节点矩阵会再加回去，两相抵消只留旋转）。
            pos = node_quat * pos + origin;
            if !params.world_space {
                pos -= node_translation;
            }
            dir = lenient_normalize(node_quat * dir);
        }
        None => {
            if params.world_space {
                // 局部点/方向 → 世界：父链的旋转也一起吃掉。
                pos = node_world.transform_point3(pos);
                dir = lenient_normalize(node_world.transform_vector3(dir));
            }
        }
    }
    // ---- 出生取值表（每一项各取自己的一次值）----
    // 稳定因子：生命期里所有「每颗稳定」的求值共用它。
    let r = rng.next_f32();
    let life = params
        .start_lifetime
        .evaluate(0.0, rng.next_f32())
        .max(0.01);
    let size_x = params.start_size.evaluate(0.0, rng.next_f32());
    let size_y = match &params.start_size_y {
        Some(curve) => curve.evaluate(0.0, rng.next_f32()),
        None => size_x,
    };
    // 第三轴只有网格绘制件用得上（billboard 是二维片），取值照抽——
    // 抽签次数是式的组成部分，值弃掉。
    if let Some(curve) = &params.start_size_z {
        let _ = curve.evaluate(0.0, rng.next_f32());
    }
    let color = params.start_color.evaluate(0.0, rng.next_f32());
    let spin0 = params.start_rotation.evaluate(0.0, rng.next_f32());
    // 三轴旋转声明：X/Y 两轴 billboard 不画，取值照抽两次。
    if params.rotation3d {
        let _ = rng.next_f32();
        let _ = rng.next_f32();
    }
    let speed = params.start_speed.evaluate(0.0, r);
    let gravity = params.start_gravity.evaluate(0.0, r);
    // 逐粒子种子：取值表的最后一抽（引擎侧是出生时写好的专用数组，写入
    // 点未读到——位置是重建，抽签次数是式的组成部分）。
    let seed = rng.next_u32();
    let world = if params.world_space {
        pos
    } else {
        node_world.transform_point3(pos)
    };
    run.particles.push(PRt {
        core: Particle::born(pos.to_array(), (dir * speed).to_array(), life),
        r,
        seed,
        rot: [0.0, 0.0, spin0],
        size_x,
        size_y,
        gravity,
        color,
        frame: 0,
        world,
        sx: 0.0,
        sy: 0.0,
    });
    run.spawned_total += 1;
}

/// 死亡触发的子发射（演示件 emitBurstZero 的转录）：只取目标的第一发
/// burst 记一次数，就地发那么多颗——不读速率、时刻、轮数与循环性。
fn emit_burst_zero(
    run: &mut EmitterRun,
    params: &SimParams,
    node_world: &Affine3A,
    origin: &Vec3,
    rng: &mut Rng,
) {
    let Some(burst) = params.bursts.first() else {
        return;
    };
    if rng.next_f32() > num(burst.probability, 1.0) {
        return;
    }
    let count = burst.count.evaluate(0.0, rng.next_f32()).round() as i32;
    let mut k = 0;
    while k < count && run.particles.len() < params.max_particles {
        spawn_particle(run, params, node_world, Some(*origin), rng);
        k += 1;
    }
}

/// 一条发射器的一帧（演示件 update 的转录）。发射环（率 + burst 游标 +
/// 循环回绕复位）只在自主播放时跑；粒子环按「寿限判定 → 重力 → 速度
/// 钳制 → 有效速度合成 → 积分 → 世界位 → 尺寸/截断/颜色/自旋/片表」
/// 的次序推进。年寿与发射钟走仿真速度，重力/积分/自旋走原始 dt——
/// 演示件就是这么分的。
#[allow(clippy::too_many_arguments)]
fn advance_emitter(
    run: &mut EmitterRun,
    params: &SimParams,
    mat: &EmitterMat,
    node_world: &Affine3A,
    cam: Option<&CamInfo>,
    rng: &mut Rng,
    auto: bool,
    dt: f32,
) {
    let sim_dt = dt * num(params.sim_speed, 1.0);
    run.age += sim_dt;
    if auto {
        // ---- 发射环 ----
        // 循环系统的发射钟在 duration 处回绕，burst 游标随之复位——只靠
        // burst 发射的循环系统少了这步会在第一轮把游标推过轮数后永远沉默。
        let duration = num(params.duration, 1.0).max(0.0001);
        let mut age = run.age;
        if params.looping {
            let loop_n = (run.age / duration).floor() as i64;
            if loop_n != run.loop_n {
                run.loop_n = loop_n;
                run.bursts.fill(0);
            }
            age -= loop_n as f32 * duration;
        }
        let rate = params.rate_over_time.evaluate(0.0, rng.next_f32());
        let ready = accumulate_rate(&mut run.pending, rate, dt);
        for _ in 0..ready {
            if run.particles.len() < params.max_particles {
                spawn_particle(run, params, node_world, None, rng);
            } else {
                // 池满时余数照扣（演示件 pending 照减），这里另记拒发数。
                run.refused += 1;
            }
        }
        for (bi, burst) in params.bursts.iter().enumerate() {
            if run.bursts.len() <= bi {
                run.bursts.resize(bi + 1, 0);
            }
            // cycleCount 0 = 无限轮；非 0 = 固定轮数（至少 1）。
            let cycles = if burst.cycle_count == 0 {
                f32::INFINITY
            } else {
                burst.cycle_count.max(1) as f32
            };
            let interval = num(burst.repeat_interval, 0.01).max(0.01);
            let mut c = run.bursts[bi] as f32;
            while c < cycles && age >= num(burst.time, 0.0) + c * interval {
                run.bursts[bi] = (c + 1.0) as u32;
                c += 1.0;
                // 概率门按轮抽（抽了可能丢弃，轮数照走）。
                if rng.next_f32() > num(burst.probability, 1.0) {
                    continue;
                }
                let count = burst.count.evaluate(0.0, rng.next_f32()).round() as i32;
                let mut k = 0;
                while k < count && run.particles.len() < params.max_particles {
                    spawn_particle(run, params, node_world, None, rng);
                    k += 1;
                }
            }
        }
    }
    // ---- 粒子环 ----
    let node_scale = node_world.to_scale_rotation_translation().0;
    let mut i = 0;
    while i < run.particles.len() {
        // 寿限：倒计时走律。寿限非正的按演示件「出生即死」——律对非正
        // 寿限拒绝推进（会永生），这里直接判死。推进前的余寿先留底：
        // 引擎的模块批（自旋/叠加速/限速）先于推进跑，读的是**推进前**
        // 的年寿进程量。
        let start_lifetime = run.particles[i].core.start_lifetime;
        let pre_remaining = run.particles[i].core.remaining_lifetime;
        let verdict = advance_lifetime(
            &mut run.particles[i].core,
            sim_dt,
            RingBufferMode::Disabled,
            [0.0, 1.0],
        );
        if matches!(verdict, LifetimeVerdict::Died) || !(start_lifetime > 0.0) {
            let p = run.particles.remove(i);
            // 死亡事件在回收之前送出：子发射要在死亡那一点的世界位上发射
            // （本帧节点世界系现算，不读上一帧的绘制位）。
            let death_world = if params.world_space {
                Vec3::from(p.core.position)
            } else {
                node_world.transform_point3(Vec3::from(p.core.position))
            };
            run.deaths.push(death_world);
            continue;
        }
        let p = &mut run.particles[i];
        // 推进前年寿进程量：模块批的曲线时刻（0..1 口径；喂引擎原生律时
        // ×100 成 agePercent 口径）。
        let age_pre = (1.0 - pre_remaining / start_lifetime).clamp(0.0, 1.0);
        // ---- 模块批（引擎次序：自旋 → 叠加速度 → 限速）----
        // 自旋：弧度每秒；公告板只画 z 轴，三轴声明时 x/y 角速度照积
        // （状态存三轴，绘制件只读 z）。randomizeDirection 本作资产结构
        // 上恒 0，按参数喂 0（律不硬编码）。
        if let Some(rol) = &params.rotation_over_lifetime {
            let _ = rol.advance_rotation(
                &mut p.rot,
                p.seed,
                0.0,
                age_pre * 100.0,
                params.rotation3d,
                dt,
            );
        }
        // velocityOverLifetime 的逐帧叠加值（不入状态）：限速段同帧要读
        // 它，先于限速算好。
        let mut anim = Vec3::ZERO;
        if let Some(vol) = &params.vol {
            anim.x = vol.x.evaluate(age_pre, p.r);
            anim.y = vol.y.evaluate(age_pre, p.r);
            anim.z = vol.z.evaluate(age_pre, p.r);
        }
        // 限速：钳的是**状态速度**（合速度 = 状态 + 叠加速度，写回只改
        // 状态速度）。拖拽非零已在装载处整块拒绝，尺寸输入恒惰性。
        if let Some(law) = &params.limit_velocity {
            let _ = law.step(
                &mut p.core.velocity,
                anim.to_array(),
                p.seed,
                age_pre * 100.0,
                dt,
                DragSize {
                    components: [p.size_x, p.size_y, 0.0],
                    size3d: false,
                },
            );
        }
        // ---- 推进（引擎的 Simulate 段）----
        // 有效速度 = 状态速度 + 叠加速度，再整体乘 speedModifier（叠加
        // 值不入状态）。
        let u = (1.0 - p.core.remaining_lifetime / p.core.start_lifetime).clamp(0.0, 1.0);
        let mut eff = Vec3::from(p.core.velocity) + anim;
        if let Some(vol) = &params.vol {
            let modifier = vol.speed_modifier.evaluate(u, p.r);
            if modifier != 0.0 {
                eff *= modifier;
            }
        }
        // 积分：律的 `integrate` 会用帧速度覆写状态速度——先存后还原。
        let state_vel = p.core.velocity;
        let _ = integrate(&mut p.core, dt, eff.to_array());
        p.core.velocity = state_vel;
        // 重力在积分**之后**（引擎次序：模块批 → 推进 → 重力）。
        if p.gravity != 0.0 {
            apply_gravity(&mut p.core, dt, GRAVITY, p.gravity);
        }
        // 世界位（绘制量）：局部空间按本帧节点世界系换算。
        p.world = if params.world_space {
            Vec3::from(p.core.position)
        } else {
            node_world.transform_point3(Vec3::from(p.core.position))
        };
        // 尺寸：sizeOverLifetime 的 X 轴乘子（Y 轴另有曲线，否则共用 X）。
        let kx = match &params.size_over_lifetime {
            Some(curve) => curve.evaluate(u, p.r),
            None => 1.0,
        };
        let ky = match &params.size_over_lifetime_y {
            Some(curve) => curve.evaluate(u, p.r),
            None => kx,
        };
        let mut sx = (p.size_x * kx).abs().max(MIN_PARTICLE_SCALE);
        let mut sy = (p.size_y * ky).abs().max(MIN_PARTICLE_SCALE);
        // 屏幕占比截断：比的是全尺寸对视口全宽，两轴同乘一个比例（保持
        // 作者写的长宽比）。
        if let Some(cam) = cam {
            let ratio = screen_clamp_ratio(cam, p.world, sx.max(sy), mat.min_frac, mat.max_frac);
            if ratio != 1.0 {
                sx *= ratio;
                sy *= ratio;
                run.clamped += 1;
            }
        }
        // 节点链缩放折算进有效尺寸（世界空间粒子挂在世界父节点下，不吃）。
        p.sx = if params.world_space {
            sx
        } else {
            sx * node_scale.x
        };
        p.sy = if params.world_space {
            sy
        } else {
            sy * node_scale.y
        };
        // 颜色：colorOverLifetime 每帧整组覆写。
        if let Some(col) = &params.color_over_lifetime {
            p.color = col.evaluate(u, p.r);
        }
        // 片表帧。
        if let Some(sheet) = &params.texture_sheet {
            let total = sheet.tiles_x * sheet.tiles_y;
            if total > 0 {
                p.frame = (sheet.frame_over_time.evaluate(u, p.r) * total as f32)
                    .floor()
                    .clamp(0.0, (total - 1) as f32) as u32;
            }
        }
        i += 1;
    }
    if run.particles.len() > run.peak {
        run.peak = run.particles.len();
    }
}

/// 一个实例的一帧（演示件 updateEmoticon + EmoticonView.update 的合并
/// 转录）。次序：挂点帧 → 顶挂旋转 → 时钟/收件请求 → 世界仿形 A 拍 →
/// 发射器推进与死亡路由 → 剪辑相位机。返回 false 表示已到 Idle（调用方
/// 回收）。绘制用的世界仿形 B 拍由调用方在相位机之后另取（顶旋转读 A
/// 拍、剪辑 TRS 读 B 拍，两拍各算一次，正是演示件「先转再播」的求值序）。
fn advance_instance(
    item: &Item,
    inst: &mut Instance,
    globals: &Query<&GlobalTransform>,
    cam: Option<&CamInfo>,
    dt: f32,
) -> bool {
    if inst.phase == Phase::Idle {
        return false;
    }
    // ---- 挂点帧（根的父）----
    let npc_global = globals
        .get(inst.npc)
        .map(|g| g.affine())
        .unwrap_or(Affine3A::IDENTITY);
    let anchor = mount_frame(mount_entity(item.anchor, &inst.mounts), globals, npc_global);
    let anchor_quat = anchor.to_scale_rotation_translation().1;
    // ---- 顶挂旋转 ----
    // sprite 朝相机（局部 +Z 指向相机，按挂点世界旋转的逆折回局部）；
    // keepPosition 的粒子族按 Hips 参照的偏航单轴转；其余粒子族保留
    // 数据姿态（else-if 结构，演示件同判）。
    if let (Some(cam), Some(top)) = (cam, inst.top) {
        if matches!(item.family, Family::Sprite) {
            let top_pos = anchor.transform_point3(inst.node_pos[top]);
            let dir = cam.pos - top_pos;
            if dir.length_squared() > 1e-12 {
                let q = look_rotation(dir.normalize());
                inst.node_rot[top] = anchor_quat.inverse() * q;
            }
        } else if item.keep_position {
            let hips = mount_frame(
                inst.timeline_rotation_reference.or(inst.mounts.hips),
                globals,
                npc_global,
            );
            let (_, hips_quat, hips_pos) = hips.to_scale_rotation_translation();
            let yaw = particle_yaw_rad(hips_pos, hips_quat, cam.pos);
            inst.node_rot[top] = Quat::from_rotation_x(-yaw);
        }
    }
    // ---- 时钟与收件请求 ----
    inst.clock += dt;
    if let Some(hide_at) = inst.hide_at {
        if inst.clock >= hide_at {
            inst.hide_at = None;
            request_hide(inst, item.clips.as_ref());
        }
    }
    // ---- 世界仿形 A 拍（发射与死亡路由读它）----
    if !inst.emitters.is_empty() {
        compute_worlds(item, inst, &anchor);
    }
    // ---- 发射器推进 + 死亡路由 ----
    // 逐条按条目序推进（子发射目标在语料里排在父之后，本帧出生的新生
    // 粒子当帧就被目标推进——与演示件的发射器遍历序等价）。
    for ri in 0..inst.emitters.len() {
        let def = inst.emitters[ri].def;
        let node = item.emitters[def].node;
        let node_world = inst.worlds[node];
        let auto = matches!(item.emitters[def].gate, Gate::Simulated);
        advance_emitter(
            &mut inst.emitters[ri],
            &item.emitters[def].params,
            &item.emitters[def].material,
            &node_world,
            cam,
            &mut inst.rng,
            auto,
            dt,
        );
        // 死亡路由（演示件 onDeath 内联）：记录概率门 → 目标的第一发
        // burst。目标没建运行时（被别的门摘掉）就不触发。
        let deaths = std::mem::take(&mut inst.emitters[ri].deaths);
        for sub in &item.emitters[def].params.sub_emitters {
            let Some(&ti) = inst.emitter_index.get(&sub.target) else {
                continue;
            };
            let target_node = item.emitters[sub.target].node;
            let target_world = inst.worlds[target_node];
            for death_pos in &deaths {
                if sub.probability < 1.0 && inst.rng.next_f32() > sub.probability {
                    continue;
                }
                emit_burst_zero(
                    &mut inst.emitters[ti],
                    &item.emitters[sub.target].params,
                    &target_world,
                    death_pos,
                    &mut inst.rng,
                );
            }
        }
    }
    // ---- 剪辑相位机 ----
    let clips = item.clips.as_ref();
    // start 段：播完落穿到 loop（或 live），同帧生效。
    if inst.phase == Phase::Start {
        let start = clips.and_then(|c| c.start.as_ref());
        let dur = start.map(|c| c.duration).unwrap_or(0.0);
        if inst.clock < num(dur, 0.0) {
            apply_clip(inst, start, inst.clock);
            return true;
        }
        inst.phase = if clips.is_some_and(|c| c.loop_.is_some()) {
            Phase::Loop
        } else {
            Phase::Live
        };
    }
    if inst.phase == Phase::Loop {
        if let Some(loop_clip) = clips.and_then(|c| c.loop_.as_ref()) {
            let base = clips
                .and_then(|c| c.start.as_ref())
                .map(|c| c.duration)
                .unwrap_or(0.0);
            let span = num(loop_clip.duration, 0.0).max(0.001);
            let t = (inst.clock - num(base, 0.0)) % span;
            apply_clip(inst, Some(loop_clip), t);
            return true;
        }
    }
    if inst.phase == Phase::End || inst.phase == Phase::Closing {
        inst.end_clock += dt;
        let end = clips.and_then(|c| c.end.as_ref());
        if let Some(clip) = end {
            apply_clip(
                inst,
                Some(clip),
                inst.end_clock.min(num(clip.duration, 0.0)),
            );
        }
        // 收场宽限：end 段时长 + disposeDelaySeconds（语料恒 1.0）。
        let dur = end.map(|c| c.duration).unwrap_or(0.0);
        if inst.end_clock >= num(dur, 0.0) + 1.0 {
            inst.phase = Phase::Idle;
        }
    }
    inst.phase != Phase::Idle
}

// ===== 绘制面与驱动面（顶点流 / 编排事件 / 帧系统） =====

/// sprite 槽的一帧顶点流：四角走节点世界仿形（B 拍），uv 是贴图子矩形
/// （sprite 位的材质 ST 恒等），颜色是节点色。pixelsPerUnit 非正按演示件
/// 缺省 100。
fn fill_sprite_buffer(item: &Item, inst: &Instance, node_idx: usize, buffer: &mut SlotMesh) {
    let node = &item.nodes[node_idx];
    let Some(sprite_idx) = node.sprite else {
        return;
    };
    let sprite = &item.sprites[sprite_idx];
    let tex = &item.textures[sprite.texture];
    let ppu = num(sprite.ppu, 100.0);
    let ppu = if ppu > 0.0 { ppu } else { 100.0 };
    let w = sprite.rect[2] / ppu;
    let h = sprite.rect[3] / ppu;
    let [px, py] = sprite.pivot;
    // 子矩形（贴图坐标；尺寸缺失的退化记录按 1 兜底，语料实测不会出现）。
    let tw = tex.width.max(1.0);
    let th = tex.height.max(1.0);
    let x0 = sprite.rect[0] / tw;
    let y0 = sprite.rect[1] / th;
    let x1 = (sprite.rect[0] + sprite.rect[2]) / tw;
    let y1 = (sprite.rect[1] + sprite.rect[3]) / th;
    // 角序 [TL, TR, BL, BR]（v=1 是贴图上沿；pivot 是 0..1 系里的锚点）。
    let mut pos = [[0.0f32; 3]; 4];
    let mut uv = [[0.0f32; 2]; 4];
    for (c, (u, v)) in [(0.0, 1.0), (1.0, 1.0), (0.0, 0.0), (1.0, 0.0)]
        .into_iter()
        .enumerate()
    {
        let local = Vec3::new((u - px) * w, (v - py) * h, 0.0);
        let world = inst.worlds[node_idx].transform_point3(local);
        pos[c] = [world.x, world.y, world.z];
        uv[c] = [x0 + u * (x1 - x0), y0 + v * (y1 - y0)];
    }
    buffer.quad(&pos, &uv, node.color, node.sprite_double_side);
}

/// 发射器位的一帧顶点流：逐粒子 billboard——四角走粒子世界位 + 相机基
/// （自旋角转正交基），uv 走「片表折格 → 基础图旋转」链（材质级 ST 在
/// 着色器里乘）。相机缺席时不画（billboard 基不存在，退化成空流）。
fn fill_emitter_buffer(
    item: &Item,
    inst: &Instance,
    emitter_idx: usize,
    cam: Option<&CamInfo>,
    buffer: &mut SlotMesh,
) {
    let Some(&ri) = inst.emitter_index.get(&emitter_idx) else {
        return;
    };
    let run = &inst.emitters[ri];
    let e = &item.emitters[emitter_idx];
    let double = e.material.cull == 0.0;
    let Some(cam) = cam else { return };
    let sheet = e
        .params
        .texture_sheet
        .as_ref()
        .filter(|t| t.tiles_x > 0 && t.tiles_y > 0);
    for p in &run.particles {
        let (cs, sn) = p.rot[2].sin_cos();
        let tx = sheet.map(|t| t.tiles_x as f32).unwrap_or(1.0);
        let ty = sheet.map(|t| t.tiles_y as f32).unwrap_or(1.0);
        let mut pos = [[0.0f32; 3]; 4];
        let mut uv = [[0.0f32; 2]; 4];
        // 角序 [TL, TR, BL, BR]：v=1 是格子上沿。
        for (c, (u, v)) in [(0.0, 1.0), (1.0, 1.0), (0.0, 0.0), (1.0, 0.0)]
            .into_iter()
            .enumerate()
        {
            let ax = (u - 0.5) * p.sx;
            let ay = (v - 0.5) * p.sy;
            let corner = p.world + cam.right * (cs * ax - sn * ay) + cam.up * (sn * ax + cs * ay);
            pos[c] = [corner.x, corner.y, corner.z];
            // 片表：格内 (u,v) 折到该帧的格位（帧号行优先、从上往下）。
            let (mut ux, mut uy) = match sheet {
                Some(s) => (
                    u / tx + (p.frame % s.tiles_x) as f32 / tx,
                    v / ty + (1.0 - 1.0 / ty - (p.frame / s.tiles_x) as f32 / ty),
                ),
                None => (u, v),
            };
            if e.material.uv_turns != 0.0 {
                // 基础图旋转：轴心 (0.5, 0.5) 的整圈数旋转（装载侧已断言与
                // 片表/非恒等 ST 互斥，此处只做旋转这一段）。
                let (rc, rs) = (std::f32::consts::TAU * e.material.uv_turns).sin_cos();
                let qx = ux - 0.5;
                let qy = uy - 0.5;
                ux = 0.5 + qx * rc + qy * rs;
                uy = 0.5 - qx * rs + qy * rc;
            }
            uv[c] = [ux, uy];
        }
        buffer.quad(&pos, &uv, p.color, double);
    }
}

/// 一个实例的全部绘制位折成顶点流并写进网格（逐帧重建；暂存借出归还，
/// 容量跨帧复用）。空位写空流——网格归零即不画。
fn build_and_write(
    item: &Item,
    inst: &mut Instance,
    cam: Option<&CamInfo>,
    meshes: &mut Assets<Mesh>,
) {
    for di in 0..inst.draws.len() {
        let kind = inst.draws[di].kind;
        let mesh_handle = inst.draws[di].mesh.clone();
        let mut buffer = std::mem::take(&mut inst.buffers[di]);
        buffer.clear();
        match kind {
            DrawKind::Sprite(node_idx) => fill_sprite_buffer(item, inst, node_idx, &mut buffer),
            DrawKind::Emitter(emitter_idx) => {
                fill_emitter_buffer(item, inst, emitter_idx, cam, &mut buffer)
            }
        }
        if let Some(mesh) = meshes.get_mut(&mesh_handle) {
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                std::mem::take(&mut buffer.positions),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, std::mem::take(&mut buffer.uvs));
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, std::mem::take(&mut buffer.colors));
            mesh.insert_indices(Indices::U32(std::mem::take(&mut buffer.indices)));
        }
        inst.buffers[di] = buffer;
    }
}

/// PostUpdate：驱动面 + 实例面的一帧。
///
/// 次序：显式展示轮播 → 存活实例推进（挂点帧 → 顶旋转 →
/// 仿真与死亡路由 → 相位机）→ B 拍世界仿形与顶点流写入。相机缺席时
/// 粒子位的顶点流退化为空（billboard 基不存在），sprite 位照画。
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<EmoticonMaterial>>,
    driver: Option<ResMut<Driver>>,
    emotes: Option<ResMut<Emotes>>,
    archive: Option<Res<EmoticonArchive>>,
    roster: Query<(Entity, &CharacterUnitId), Without<crate::player::PlayerControlled>>,
    live_entities: Query<()>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    names: Query<&Name>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
    holds: Query<&crate::talk::TalkHold>,
    time: Res<Time>,
) {
    let (Some(mut driver), Some(mut emotes), Some(archive)) = (driver, emotes, archive) else {
        return;
    };
    let dt = time.delta_secs().min(DT_CLAMP);
    // 相机读数（billboard 基 / 屏幕占比截断 / 出场投影共用；正交投影没有
    // fov 语义，占比截断自然让位）。
    let cam = cameras.single().ok().map(|(g, proj, camera)| {
        let fov_y = match proj {
            Projection::Perspective(p) => p.fov,
            _ => 0.0,
        };
        let aspect = camera
            .physical_viewport_size()
            .map(|s| s.x as f32 / s.y.max(1) as f32)
            .unwrap_or(0.0);
        CamInfo {
            pos: g.translation(),
            right: g.right().as_vec3(),
            up: g.up().as_vec3(),
            forward: g.forward().as_vec3(),
            fov_y,
            aspect,
        }
    });

    let items = &archive.archive.items;

    // ---- 展示轮播：第一名册成员没有在屏实例的帧推一轮（冒烟可复算）；
    // 对话持留的成员跳过（同一实例槽归对话侧）----
    if let (Some(first), Some(showcase)) = (
        roster
            .iter()
            .min_by_key(|(_, unit)| unit.0)
            .map(|(npc, _)| npc),
        driver.showcase.as_mut(),
    ) {
        if globals.get(first).is_ok()
            && !holds.contains(first)
            && !emotes.instances.iter().any(|i| i.npc == first)
        {
            showcase.idx = (showcase.idx + 1) % showcase.items.len();
            let item_idx = showcase.items[showcase.idx];
            let name = items[item_idx].name.clone();
            info!("[emoticon] 展示轮播 → {name}");
            show_emote(
                &mut commands,
                &mut *meshes,
                &mut *materials,
                &globals,
                &children,
                &names,
                &cameras,
                &mut *emotes,
                &archive.archive,
                &name,
                first,
                showcase.show_secs,
            );
        }
    }

    // ---- 存活实例推进 + B 拍世界仿形 + 顶点流 ----
    let mut dead: Vec<usize> = Vec::new();
    for ii in 0..emotes.instances.len() {
        // Draw slots are standalone entities, not descendants of the avatar.
        // A zero-duration Live/Loop instance otherwise survives its owner and
        // keeps rendering at the IDENTITY fallback indefinitely.
        if !live_entities.contains(emotes.instances[ii].npc) {
            dead.push(ii);
            continue;
        }
        let item = &items[emotes.instances[ii].item];
        let alive = advance_instance(item, &mut emotes.instances[ii], &globals, cam.as_ref(), dt);
        if !alive {
            dead.push(ii);
            continue;
        }
        let inst = &mut emotes.instances[ii];
        let npc_global = globals
            .get(inst.npc)
            .map(|g| g.affine())
            .unwrap_or(Affine3A::IDENTITY);
        let anchor = mount_frame(
            mount_entity(item.anchor, &inst.mounts),
            &globals,
            npc_global,
        );
        compute_worlds(item, inst, &anchor);
        build_and_write(item, inst, cam.as_ref(), &mut *meshes);
    }
    // 回收（倒序：swap_remove 动的是尾部，先删大下标不动小下标）。
    for &ii in dead.iter().rev() {
        clear_instance(
            &mut commands,
            &mut *meshes,
            &mut *materials,
            &mut *emotes,
            ii,
        );
    }
}

/// Rest commands enter the same prepared renderer as dialogue commands. Hide
/// requests start the existing end/closing phase; they do not erase pixels
/// synchronously. Cold asynchronous view creation remains an asset-host concern.
#[allow(clippy::type_complexity)]
pub(crate) fn serve_rest(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<EmoticonMaterial>>,
    mut requests: ResMut<RestEmoteRequests>,
    emotes: Option<ResMut<Emotes>>,
    archive: Option<Res<EmoticonArchive>>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    names: Query<&Name>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
) {
    let (Some(mut emotes), Some(archive)) = (emotes, archive) else {
        return;
    };
    for request in requests.0.drain(..) {
        match request {
            RestEmoteCommand::Show {
                npc,
                name,
                not_play_se,
            } => {
                if !not_play_se {
                    warn!("[emoticon] Rest 表情请求音效：当前表情 soundInput 播放器尚未供给");
                }
                show_emote(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &globals,
                    &children,
                    &names,
                    &cameras,
                    &mut emotes,
                    &archive.archive,
                    &name,
                    npc,
                    0.0,
                );
            }
            RestEmoteCommand::Hide { npc } => {
                if let Some(index) = emotes
                    .instances
                    .iter()
                    .position(|instance| instance.npc == npc)
                {
                    let clips = archive.archive.items[emotes.instances[index].item]
                        .clips
                        .as_ref();
                    request_hide(&mut emotes.instances[index], clips);
                }
            }
        }
    }
}

/// Update：对话步的表情件请求消费——对话模块折好的出件/收件按表序
/// 走本域的同一条出件/收件通道（[`show_emote`] 与收件路径），不另起
/// 一套呈现。同 NPC 的在屏实例唯一：出件自带撤旧件，对话件与待机编排
/// 件在同一槽上自然互斥。档案未到齐时请求弃掉（对话候选的装载期核验
/// 已拦过缺条目，这里的弃件不是静默跳过）。
#[allow(clippy::type_complexity)]
pub(crate) fn serve_talk(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<EmoticonMaterial>>,
    mut reqs: ResMut<crate::talk::TalkEmoteReqs>,
    emotes: Option<ResMut<Emotes>>,
    archive: Option<Res<EmoticonArchive>>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    names: Query<&Name>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
) {
    if reqs.shows.is_empty() && reqs.hides.is_empty() {
        return;
    }
    let (Some(mut emotes), Some(archive)) = (emotes, archive) else {
        reqs.shows.clear();
        reqs.hides.clear();
        return;
    };
    let items = &archive.archive.items;
    for (npc, name, seconds) in reqs.shows.drain(..) {
        show_emote(
            &mut commands,
            &mut *meshes,
            &mut *materials,
            &globals,
            &children,
            &names,
            &cameras,
            &mut *emotes,
            &archive.archive,
            &name,
            npc,
            seconds,
        );
    }
    for npc in reqs.hides.drain(..) {
        if let Some(idx) = emotes.instances.iter().position(|i| i.npc == npc) {
            let clips = items[emotes.instances[idx].item].clips.as_ref();
            request_hide(&mut emotes.instances[idx], clips);
        }
    }
}

/// PostUpdate（周期）：在屏账目，全部可从装载行与出场行复算——每实例
/// 一行（相位/时钟/绘制位/活粒子/峰值/累计），汇总一行（绘制实体带在册
/// 核对：与实例绘制位总数不符即有孤儿绘制实体）。
pub(crate) fn report(
    emotes: Option<Res<Emotes>>,
    driver: Option<Res<Driver>>,
    archive: Option<Res<EmoticonArchive>>,
    draws: Query<&EmoteDraw>,
) {
    if !bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
        return;
    }
    let (Some(emotes), Some(_driver), Some(archive)) = (emotes, driver, archive) else {
        return;
    };
    for inst in &emotes.instances {
        let item = &archive.archive.items[inst.item];
        let live: usize = inst.emitters.iter().map(|r| r.particles.len()).sum();
        let peak: usize = inst.emitters.iter().map(|r| r.peak).sum();
        let spawned: u64 = inst.emitters.iter().map(|r| r.spawned_total).sum();
        let refused: u64 = inst.emitters.iter().map(|r| r.refused).sum();
        let clamped: u32 = inst.emitters.iter().map(|r| r.clamped).sum();
        debug!(
            "[emoticon] 在屏 {}（{:?}）clock={:.2} 绘制位 {} · 活粒子 {} · 峰值 {} · 累计出生 {} · 拒发 {} · 截断 {}",
            item.name,
            inst.phase,
            inst.clock,
            inst.draws.len(),
            live,
            peak,
            spawned,
            refused,
            clamped
        );
    }
    let owned: usize = emotes.instances.iter().map(|i| i.draws.len()).sum();
    let keys: Vec<u64> = emotes.instances.iter().map(|i| i.key).collect();
    let live_draws = draws.iter().filter(|d| keys.contains(&d.instance)).count();
    let missing = if emotes.missing.is_empty() {
        "无".to_owned()
    } else {
        emotes.missing.join("、")
    };
    debug!(
        "[emoticon] 汇总：在屏 {} · 绘制实体 {}/{} · 累计出件 {} · 缺条目 [{}]",
        emotes.instances.len(),
        live_draws,
        owned,
        emotes.shown_total,
        missing
    );
}
