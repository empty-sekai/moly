//! effects.json（rain 家族那份形状）→ 律侧发射器参数。
//!
//! 字段词汇按语料实测枚举（全部 25 个现象档案里 MinMax 值对象的键形
//! 各只有一族）：曲线 `constant{value}` / `twoConstants{min,max}` /
//! `curve{multiplier,keys}` / `twoCurves{multiplier,minKeys,maxKeys}`；
//! 梯度 `color{color}` / `gradient{gradient}` / `twoColors{min,max}` /
//! `twoGradients{minGradient,maxGradient}` / `randomColor{gradient}`。
//! 解析**失败响亮**：键在而形状不对是数据损伤，静默兜底会重建
//! 「接没接线分不清」。
//!
//! 识别但**未映射**的键不丢弃也不报错——收进 `unmapped` 具名清单
//! （subEmitters、collision、forceOverLifetime、scalingMode、
//! emitterVelocityMode、randomSeed、autoRandomSeed、renderer、以及 shape
//! 的非圆参数族），消费侧可见「这条数据在，但律没管它」。

use std::fmt;

use crate::particle::buffer::RingBufferMode;
use crate::particle::emit::{Burst, BurstCycles};
use crate::particle::json::{self, Value};
use crate::particle::value::{
    Curve, CurveKey, Gradient, GradientAlphaKey, GradientColorKey, MinMaxCurve, MinMaxGradient,
};

/// 解析失败：一条人话，带 effect/node 定位。
#[derive(Debug, Clone, PartialEq)]
pub struct EffectsError(pub String);

impl fmt::Display for EffectsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "effects.json: {}", self.0)
    }
}

impl std::error::Error for EffectsError {}

/// 一份档案的全部发射器（含未映射键清单）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Effects {
    pub emitters: Vec<EmitterParams>,
    /// effect 层识别到但未映射的键（去重）。
    pub unmapped_effect_keys: Vec<String>,
}

impl Effects {
    /// 从档案字节解析。
    pub fn from_json_str(bytes: &[u8]) -> Result<Self, EffectsError> {
        let v = json::parse(bytes).map_err(|e| EffectsError(e))?;
        Self::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<Self, EffectsError> {
        let effects = v
            .get("effects")
            .and_then(Value::as_object)
            .ok_or_else(|| EffectsError("top-level \"effects\" object missing".into()))?;
        let mut out = Effects::default();
        for (name, e) in effects {
            let particles = e
                .get("particles")
                .and_then(Value::as_array)
                .ok_or_else(|| EffectsError(format!("effect {name}: \"particles\" array missing")))?;
            // effect 层的键里只有 particles 进律；其余全收清单。
            for (k, _) in e.as_object().unwrap_or(&[]) {
                if k != "particles" && !out.unmapped_effect_keys.contains(k) {
                    out.unmapped_effect_keys.push(k.clone());
                }
            }
            for p in particles {
                out.emitters.push(EmitterParams::from_particle(name, p)?);
            }
        }
        Ok(out)
    }
}

/// 模拟空间。字符串名与数值 `C# 可读`
/// （`ParticleSystemSimulationSpace` Local=0/World=1/Custom=2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationSpace {
    Local,
    World,
    Custom,
}

impl SimulationSpace {
    fn from_str(s: &str, ctx: &str) -> Result<Self, EffectsError> {
        match s {
            "Local" => Ok(SimulationSpace::Local),
            "World" => Ok(SimulationSpace::World),
            "Custom" => Ok(SimulationSpace::Custom),
            _ => Err(EffectsError(format!("{ctx}: unknown simulationSpace {s:?}"))),
        }
    }
}

/// start 模块：出生参数（全部 MinMax 值，rand lerp 求值）。
#[derive(Debug, Clone, PartialEq)]
pub struct StartParams {
    pub lifetime: MinMaxCurve,
    pub speed: MinMaxCurve,
    pub size: MinMaxCurve,
    /// size3D 时的 Y/Z 分量曲线（缺省回落到 `size`）。
    pub size_y: Option<MinMaxCurve>,
    pub size_z: Option<MinMaxCurve>,
    pub size3d: bool,
    pub rotation: MinMaxCurve,
    pub rotation_x: Option<MinMaxCurve>,
    pub rotation_y: Option<MinMaxCurve>,
    pub rotation3d: bool,
    pub color: MinMaxGradient,
    pub gravity_modifier: MinMaxCurve,
}

/// emission 模块。
#[derive(Debug, Clone, PartialEq)]
pub struct EmissionParams {
    pub rate_over_time: MinMaxCurve,
    pub rate_over_distance: MinMaxCurve,
    pub bursts: Vec<Burst>,
}

/// shape 模块：只映射 Circle 律用到的参数；其余收 `unmapped`。
/// 非圆 type 本身**解析不拒**（schema 是数据面），采样时由
/// `shape::circle_position` 的调用方拒。
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeParams {
    pub shape_type: String,
    pub radius: f32,
    pub radius_thickness: f32,
    pub arc: f32,
    /// 欧拉旋转（度）。施加口径见 `shape::euler_rotate_deg`。
    pub rotation: [f32; 3],
    pub position: [f32; 3],
    /// Authored controls are preserved even when a renderer cannot consume
    /// them yet. Missing legacy metadata is distinguishable from authored zero.
    pub controls: ShapeControls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeMode { Random, Loop, PingPong, BurstSpread }

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapeControls {
    pub source_version: Option<u32>,
    pub angle: Option<f32>,
    pub length: Option<f32>,
    pub donut_radius: Option<f32>,
    pub scale: Option<[f32; 3]>,
    pub box_thickness: Option<[f32; 3]>,
    pub arc_mode: Option<ShapeMode>,
    pub arc_spread: Option<f32>,
    pub arc_speed: Option<MinMaxCurve>,
    pub radius_mode: Option<ShapeMode>,
    pub radius_spread: Option<f32>,
    pub radius_speed: Option<MinMaxCurve>,
    pub align_to_direction: Option<bool>,
    pub random_direction: Option<f32>,
    pub spherical_direction: Option<f32>,
    pub random_position: Option<f32>,
}


/// velocityOverLifetime 模块。
#[derive(Debug, Clone, PartialEq)]
pub struct VelocityOverLifetimeParams {
    pub x: MinMaxCurve,
    pub y: MinMaxCurve,
    pub z: MinMaxCurve,
    pub speed_modifier: MinMaxCurve,
    pub in_world_space: bool,
    /// Orbital motion is emitter-local even when linear motion is world-space.
    pub orbital: [MinMaxCurve; 3],
    pub orbital_offset: [MinMaxCurve; 3],
    pub radial: MinMaxCurve,
}

/// customData 模块的一个槽（`custom1` / `custom2`）。
///
/// 逐粒子自定义流：每个分量是一条独立的 MinMax 曲线，按归一化寿命逐粒子
/// 求值，结果进顶点属性的对应分量。着色器侧的选择器按
/// `coord = 分量号 × 10 + 来源号` 取值，来源 1 = `custom1`、2 = `custom2`、
/// 0 = 常量零向量。
///
/// `componentCount` 之外的分量不写，留零——源程序的选择器可以指到那里，
/// 取到的就是零。
#[derive(Debug, Clone, PartialEq)]
pub struct CustomDataSlot {
    /// 实际产出的分量数（1..=4）。
    pub component_count: usize,
    /// 逐分量曲线，长度等于 `component_count`。
    pub components: Vec<MinMaxCurve>,
}

/// customData 模块：两个槽，缺席或 `disabled` 为 None。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomDataParams {
    pub custom1: Option<CustomDataSlot>,
    pub custom2: Option<CustomDataSlot>,
}

/// sizeOverLifetime 模块（分轴时 `curve` 是 X 轴）。
#[derive(Debug, Clone, PartialEq)]
pub struct SizeOverLifetimeParams {
    pub separate_axes: bool,
    pub curve: MinMaxCurve,
    pub y: Option<MinMaxCurve>,
    pub z: Option<MinMaxCurve>,
}

/// rotationOverLifetime 模块。`curve` 是 z 轴（编辑器里 z 绑定在序列化
/// 名 `curve` 上，与 size 模块镜像——那边 `curve` 是 x）；`x`/`y` 只在
/// separateAxes 为真时由提取面导出（缺席 = 提取面缺键，不是当 0——
/// 律构造处拒，见 `RotationOverLifetime::from_parts`）。
#[derive(Debug, Clone, PartialEq)]
pub struct RotationOverLifetimeParams {
    pub separate_axes: bool,
    pub curve: MinMaxCurve,
    pub x: Option<MinMaxCurve>,
    pub y: Option<MinMaxCurve>,
}

/// limitVelocity 模块。`drag` 为 null（提取面在拖拽曲线为空时写 null）
/// 是「拖拽段跳过」的规范形态；乘法标志提取面暂不导出（None = 未知，
/// 非零拖拽时会在律构造处拒——缺一个会左右结果的键不是当 false）；
/// `in_world_space` 在引擎的平面向量路径里不被读取，字段只为数据
/// 可见性保留。
#[derive(Debug, Clone, PartialEq)]
pub struct LimitVelocityParams {
    pub separate_axis: bool,
    pub magnitude: MinMaxCurve,
    pub dampen: f32,
    pub drag: Option<MinMaxCurve>,
    pub in_world_space: bool,
    pub multiply_drag_by_size: Option<bool>,
    pub multiply_drag_by_velocity: Option<bool>,
}

/// 一个发射器的全部律侧参数。
#[derive(Debug, Clone, PartialEq)]
pub struct EmitterParams {
    pub effect: String,
    pub node: String,
    pub duration: f32,
    pub looping: bool,
    pub prewarm: bool,
    pub play_on_awake: bool,
    pub simulation_speed: f32,
    pub simulation_space: SimulationSpace,
    pub start_delay: MinMaxCurve,
    pub ring_buffer_mode: RingBufferMode,
    pub ring_buffer_loop_range: [f32; 2],
    pub max_particles: u32,
    pub start: StartParams,
    pub emission: Option<EmissionParams>,
    pub shape: Option<ShapeParams>,
    /// None means legacy/unknown module state, not an implicitly disabled shape.
    pub shape_enabled: Option<bool>,
    pub velocity_over_lifetime: Option<VelocityOverLifetimeParams>,
    pub color_over_lifetime: Option<MinMaxGradient>,
    pub size_over_lifetime: Option<SizeOverLifetimeParams>,
    pub rotation_over_lifetime: Option<RotationOverLifetimeParams>,
    pub limit_velocity: Option<LimitVelocityParams>,
    /// 逐粒子自定义流。两个槽都缺席或 `disabled` 时为 None。
    pub custom_data: Option<CustomDataParams>,
    /// system 层 + particle 层识别到但未映射的键（去重、按序）。
    pub unmapped: Vec<String>,
}

/// system 层已映射进参数的键——之外的键全部进 `unmapped`。
/// 「识别但具名不迁」的那四个（scalingMode、emitterVelocityMode、
/// randomSeed、autoRandomSeed）**不在**此列：它们同样落
/// `unmapped`，让消费侧看见「数据在、律没管」。
const MAPPED_SYSTEM_KEYS: [&str; 20] = [
    "duration", "looping", "prewarm", "playOnAwake", "simulationSpeed",
    "simulationSpace", "startDelay", "ringBufferMode", "ringBufferLoopRange",
    "maxParticles", "start", "emission", "shape", "velocityOverLifetime",
    "colorOverLifetime", "sizeOverLifetime", "rotationOverLifetime",
    "limitVelocity", "customData", "shapeEnabled",
];

/// start 层已映射键。
const MAPPED_START_KEYS: [&str; 12] = [
    "lifetime", "speed", "size", "sizeY", "sizeZ", "size3D", "rotation",
    "rotationX", "rotationY", "rotation3D", "color", "gravityModifier",
];

impl EmitterParams {
    fn from_particle(effect: &str, p: &Value) -> Result<Self, EffectsError> {
        let ctx = format!("effect {effect}");
        let node = str_of(p.get("node"), &format!("{ctx}: node"))?;
        let ctx = format!("{ctx}/{node}");
        let system = p
            .get("system")
            .and_then(Value::as_object)
            .ok_or_else(|| EffectsError(format!("{ctx}: \"system\" object missing")))?;
        let g = |k: &str| f32_of(system_get(system, k), &format!("{ctx}.{k}"));
        let mut unmapped = Vec::new();
        // particle 层：node 与 system 之外的键（renderer 等）收清单。
        for (k, v) in p.as_object().unwrap_or(&[]) {
            if k != "node" && k != "system" {
                let label = if k == "renderer" {
                    // renderer 对象的子键不再展开——整块是渲染侧。
                    "renderer".to_string()
                } else {
                    k.clone()
                };
                if !unmapped.contains(&label) {
                    unmapped.push(label);
                }
                let _ = v;
            }
        }
        for (k, _) in system {
            if !MAPPED_SYSTEM_KEYS.contains(&k.as_str()) && !unmapped.contains(k) {
                unmapped.push(k.clone());
            }
        }
        let ring_mode = u32_of(system_get(system, "ringBufferMode"), &format!("{ctx}.ringBufferMode"))?;
        let ring_mode = RingBufferMode::from_u32(ring_mode).ok_or_else(|| {
            EffectsError(format!("{ctx}.ringBufferMode: unknown value {ring_mode}"))
        })?;
        let loop_range = vec2_of(
            system_get(system, "ringBufferLoopRange"),
            &format!("{ctx}.ringBufferLoopRange"),
        )?;
        let emission = match system_get(system, "emission") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(EmissionParams::from_value(v, &ctx)?)
            }
            _ => None,
        };
        let shape = match system_get(system, "shape") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(ShapeParams::from_value(v, &ctx, &mut unmapped)?)
            }
            _ => None,
        };
        let velocity_over_lifetime = match system_get(system, "velocityOverLifetime") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(VelocityOverLifetimeParams::from_value(v, &ctx)?)
            }
            _ => None,
        };
        let color_over_lifetime = match system_get(system, "colorOverLifetime") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(min_max_gradient(Some(v), &format!("{ctx}.colorOverLifetime"))?)
            }
            _ => None,
        };
        let size_over_lifetime = match system_get(system, "sizeOverLifetime") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(SizeOverLifetimeParams::from_value(v, &ctx)?)
            }
            _ => None,
        };
        let custom_data = match system_get(system, "customData") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                CustomDataParams::from_value(v, &ctx)?
            }
            _ => None,
        };
        let rotation_over_lifetime = match system_get(system, "rotationOverLifetime") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(RotationOverLifetimeParams::from_value(v, &ctx)?)
            }
            _ => None,
        };
        let limit_velocity = match system_get(system, "limitVelocity") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(LimitVelocityParams::from_value(v, &ctx)?)
            }
            _ => None,
        };
        Ok(Self {
            effect: effect.to_string(),
            node,
            duration: g("duration")?,
            looping: bool_of(system_get(system, "looping"), &format!("{ctx}.looping"))?,
            prewarm: bool_of(system_get(system, "prewarm"), &format!("{ctx}.prewarm"))?,
            play_on_awake: bool_of(system_get(system, "playOnAwake"), &format!("{ctx}.playOnAwake"))?,
            simulation_speed: g("simulationSpeed")?,
            simulation_space: {
                let name = str_of(
                    system_get(system, "simulationSpace"),
                    &format!("{ctx}.simulationSpace"),
                )?;
                SimulationSpace::from_str(&name, &ctx)?
            },
            start_delay: min_max_curve(
                system_get(system, "startDelay"),
                &format!("{ctx}.startDelay"),
            )?,
            ring_buffer_mode: ring_mode,
            ring_buffer_loop_range: loop_range,
            max_particles: u32_of(
                system_get(system, "maxParticles"),
                &format!("{ctx}.maxParticles"),
            )?,
            start: StartParams::from_value(
                system_get(system, "start"),
                &ctx,
                &mut unmapped,
            )?,
            emission,
            shape,
            shape_enabled: match system_get(system, "shapeEnabled") {
                None | Some(Value::Null) => None,
                value => Some(bool_of(value, &format!("{ctx}.shapeEnabled"))?),
            },
            velocity_over_lifetime,
            color_over_lifetime,
            size_over_lifetime,
            custom_data,
            rotation_over_lifetime,
            limit_velocity,
            unmapped,
        })
    }
}

impl StartParams {
    fn from_value(
        v: Option<&Value>,
        ctx: &str,
        unmapped: &mut Vec<String>,
    ) -> Result<Self, EffectsError> {
        let obj = v
            .and_then(Value::as_object)
            .ok_or_else(|| EffectsError(format!("{ctx}.start: object missing")))?;
        for (k, _) in obj {
            if !MAPPED_START_KEYS.contains(&k.as_str()) && !unmapped.contains(k) {
                unmapped.push(k.clone());
            }
        }
        Ok(Self {
            lifetime: lifetime_curve(obj_get(obj, "lifetime"), &format!("{ctx}.start.lifetime"))?,
            speed: min_max_curve(obj_get(obj, "speed"), &format!("{ctx}.start.speed"))?,
            size: min_max_curve(obj_get(obj, "size"), &format!("{ctx}.start.size"))?,
            size_y: obj_get(obj, "sizeY")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.sizeY")))
                .transpose()?,
            size_z: obj_get(obj, "sizeZ")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.sizeZ")))
                .transpose()?,
            size3d: bool_of(obj_get(obj, "size3D"), &format!("{ctx}.start.size3D"))?,
            rotation: min_max_curve(obj_get(obj, "rotation"), &format!("{ctx}.start.rotation"))?,
            rotation_x: obj_get(obj, "rotationX")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.rotationX"))).transpose()?,
            rotation_y: obj_get(obj, "rotationY")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.rotationY"))).transpose()?,
            rotation3d: bool_of(obj_get(obj, "rotation3D"), &format!("{ctx}.start.rotation3D"))?,
            color: min_max_gradient(obj_get(obj, "color"), &format!("{ctx}.start.color"))?,
            gravity_modifier: min_max_curve(
                obj_get(obj, "gravityModifier"),
                &format!("{ctx}.start.gravityModifier"),
            )?,
        })
    }
}

impl EmissionParams {
    fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let bursts = match obj_get(obj, "bursts").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let bctx = format!("{ctx}.emission.bursts[{i}]");
                    let count = min_max_curve(b.get("count"), &format!("{bctx}.count"))?;
                    // 序列化的 `cycleCount == 0` 是「无限重复」，不是非法值：
                    // 引擎 Editor 的轮数下拉第 0 项写的就是字面量 0，托管
                    // setter 也只拒 `< 0`。此处此前把 0 当非法整条拒掉，
                    // 常驻发射器（喷泉水柱一族）因此一条都进不来。
                    let cycles = BurstCycles::from_serialized(u32_of(
                        b.get("cycleCount"),
                        &format!("{bctx}.cycleCount"),
                    )?);
                    Ok(Burst {
                        time: f32_of(b.get("time"), &format!("{bctx}.time"))?,
                        count,
                        cycles,
                        repeat_interval: f32_of(
                            b.get("repeatInterval"),
                            &format!("{bctx}.repeatInterval"),
                        )?,
                        probability: f32_of(
                            b.get("probability"),
                            &format!("{bctx}.probability"),
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            rate_over_time: min_max_curve(
                obj_get(obj, "rateOverTime"),
                &format!("{ctx}.emission.rateOverTime"),
            )?,
            rate_over_distance: min_max_curve(
                obj_get(obj, "rateOverDistance"),
                &format!("{ctx}.emission.rateOverDistance"),
            )?,
            bursts,
        })
    }
}

/// shape 层映射的闭集；其余（angle/length/boxThickness/donutRadius/
/// mesh*/alignToDirection/randomDirectionAmount/sphericalDirectionAmount/
/// scale）收 unmapped。
const MAPPED_SHAPE_KEYS: &[&str] = &[
    "type", "radius", "radiusThickness", "arc", "rotation", "position", "sourceVersion",
    "angle", "length", "donutRadius", "scale", "boxThickness", "arcMode", "arcSpread", "arcSpeed",
    "radiusMode", "radiusSpread", "radiusSpeed", "alignToDirection", "randomDirectionAmount",
    "sphericalDirectionAmount", "randomPositionAmount",
];

impl ShapeMode {
    fn from_value(value: &Value, ctx: &str) -> Result<Self, EffectsError> {
        match value.as_str() {
            Some("Random") => Ok(Self::Random), Some("Loop") => Ok(Self::Loop),
            Some("PingPong") => Ok(Self::PingPong), Some("BurstSpread") => Ok(Self::BurstSpread),
            _ => Err(EffectsError(format!("{ctx}: unknown source shape mode"))),
        }
    }
}

impl ShapeControls {
    fn from_value(value: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let number = |key: &str| value.get(key).filter(|v| !matches!(v, Value::Null))
            .map(|v| f32_of(Some(v), &format!("{ctx}.{key}"))).transpose();
        let vector = |key: &str| value.get(key).filter(|v| !matches!(v, Value::Null))
            .map(|v| vec3_of(Some(v), &format!("{ctx}.{key}"))).transpose();
        let curve = |key: &str| value.get(key).filter(|v| !matches!(v, Value::Null))
            .map(|v| min_max_curve(Some(v), &format!("{ctx}.{key}"))).transpose();
        let mode = |key: &str| value.get(key).filter(|v| !matches!(v, Value::Null))
            .map(|v| ShapeMode::from_value(v, &format!("{ctx}.{key}"))).transpose();
        Ok(Self {
            source_version:value.get("sourceVersion").map(|v|u32_of(Some(v),ctx)).transpose()?,
            angle:number("angle")?,length:number("length")?,donut_radius:number("donutRadius")?,
            scale:vector("scale")?,box_thickness:vector("boxThickness")?,
            arc_mode:mode("arcMode")?,arc_spread:number("arcSpread")?,arc_speed:curve("arcSpeed")?,
            radius_mode:mode("radiusMode")?,radius_spread:number("radiusSpread")?,radius_speed:curve("radiusSpeed")?,
            align_to_direction:value.get("alignToDirection").filter(|v|!matches!(v,Value::Null))
                .map(|v|bool_of(Some(v),ctx)).transpose()?,
            random_direction:number("randomDirectionAmount")?,spherical_direction:number("sphericalDirectionAmount")?,
            random_position:number("randomPositionAmount")?,
        })
    }
}


impl ShapeParams {
    fn from_value(
        v: &Value,
        ctx: &str,
        unmapped: &mut Vec<String>,
    ) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        for (k, _) in obj {
            if !MAPPED_SHAPE_KEYS.contains(&k.as_str()) && !unmapped.contains(k) {
                unmapped.push(format!("shape.{k}"));
            }
        }
        Ok(Self {
            shape_type: str_of(obj_get(obj, "type"), &format!("{ctx}.shape.type"))?,
            radius: f32_of(obj_get(obj, "radius"), &format!("{ctx}.shape.radius"))?,
            radius_thickness: f32_of(
                obj_get(obj, "radiusThickness"),
                &format!("{ctx}.shape.radiusThickness"),
            )?,
            arc: f32_of(obj_get(obj, "arc"), &format!("{ctx}.shape.arc"))?,
            rotation: vec3_of(obj_get(obj, "rotation"), &format!("{ctx}.shape.rotation"))?,
            position: vec3_of(obj_get(obj, "position"), &format!("{ctx}.shape.position"))?,
            controls: ShapeControls::from_value(v, &format!("{ctx}.shape"))?,
        })
    }
}

impl VelocityOverLifetimeParams {
    fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let curve = |key: &str| min_max_curve(obj_get(obj, key), &format!("{ctx}.velocityOverLifetime.{key}"));
        Ok(Self {
            orbital: [curve("orbitalX")?, curve("orbitalY")?, curve("orbitalZ")?],
            orbital_offset: [curve("orbitalOffsetX")?, curve("orbitalOffsetY")?, curve("orbitalOffsetZ")?],
            radial: curve("radial")?,
            x: min_max_curve(obj_get(obj, "x"), &format!("{ctx}.velocityOverLifetime.x"))?,
            y: min_max_curve(obj_get(obj, "y"), &format!("{ctx}.velocityOverLifetime.y"))?,
            z: min_max_curve(obj_get(obj, "z"), &format!("{ctx}.velocityOverLifetime.z"))?,
            speed_modifier: min_max_curve(
                obj_get(obj, "speedModifier"),
                &format!("{ctx}.velocityOverLifetime.speedModifier"),
            )?,
            in_world_space: bool_of(
                obj_get(obj, "inWorldSpace"),
                &format!("{ctx}.velocityOverLifetime.inWorldSpace"),
            )?,
        })
    }
}

impl CustomDataParams {
    /// 两个槽都缺席或 `disabled` 时返回 None——没有流要产出。
    fn from_value(v: &Value, ctx: &str) -> Result<Option<Self>, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let custom1 = CustomDataSlot::from_value(
            obj_get(obj, "custom1"),
            &format!("{ctx}.customData.custom1"),
        )?;
        let custom2 = CustomDataSlot::from_value(
            obj_get(obj, "custom2"),
            &format!("{ctx}.customData.custom2"),
        )?;
        if custom1.is_none() && custom2.is_none() {
            return Ok(None);
        }
        Ok(Some(Self { custom1, custom2 }))
    }
}

impl CustomDataSlot {
    fn from_value(v: Option<&Value>, ctx: &str) -> Result<Option<Self>, EffectsError> {
        let Some(obj) = v.and_then(Value::as_object) else {
            return Ok(None);
        };
        let mode = str_of(obj_get(obj, "mode"), &format!("{ctx}.mode"))?;
        match mode.as_str() {
            "disabled" => Ok(None),
            // `color` 档存的是 MinMaxGradient 不是逐分量曲线，是另一种形状；
            // 本站语料里 0 条，落这里要响亮拒，不要静默当 vector 读。
            "vector" => {
                let count = f32_of(
                    obj_get(obj, "componentCount"),
                    &format!("{ctx}.componentCount"),
                )? as usize;
                if count == 0 || count > 4 {
                    return Err(EffectsError(format!(
                        "{ctx}.componentCount: {count} 不在 1..=4"
                    )));
                }
                let components = obj_get(obj, "components")
                    .and_then(Value::as_array)
                    .ok_or_else(|| EffectsError(format!("{ctx}.components: array missing")))?;
                if components.len() < count {
                    return Err(EffectsError(format!(
                        "{ctx}.components: {} 条曲线不足 componentCount {count}",
                        components.len()
                    )));
                }
                let components = components
                    .iter()
                    .take(count)
                    .enumerate()
                    .map(|(i, c)| min_max_curve(Some(c), &format!("{ctx}.components[{i}]")))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Some(Self {
                    component_count: count,
                    components,
                }))
            }
            other => Err(EffectsError(format!("{ctx}.mode: 未实现的档 {other}"))),
        }
    }
}

impl SizeOverLifetimeParams {
    fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let params = Self {
            separate_axes: bool_of(
                obj_get(obj, "separateAxes"),
                &format!("{ctx}.sizeOverLifetime.separateAxes"),
            )?,
            curve: min_max_curve(
                obj_get(obj, "curve"),
                &format!("{ctx}.sizeOverLifetime.curve"),
            )?,
            y: obj_get(obj, "y")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.sizeOverLifetime.y")))
                .transpose()?,
            z: obj_get(obj, "z")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.sizeOverLifetime.z")))
                .transpose()?,
        };
        if params.separate_axes && (params.y.is_none() || params.z.is_none()) {
            return Err(EffectsError(format!("{ctx}.sizeOverLifetime: separate axes require Y and Z curves")));
        }
        Ok(params)
    }
}

/// MinMax 对象或 null：null/缺席 → None（拖拽段跳过的规范形态），
/// 其余形状交给 `min_max_curve` 响亮拒。
fn opt_min_max_curve(v: Option<&Value>, ctx: &str) -> Result<Option<MinMaxCurve>, EffectsError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => min_max_curve(Some(v), ctx).map(Some),
    }
}

/// 可选布尔：缺席/null → None（未导出 = 未知），在档而形状不对仍响亮拒。
fn opt_bool_of(v: Option<&Value>, ctx: &str) -> Result<Option<bool>, EffectsError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => bool_of(Some(v), ctx).map(Some),
    }
}

impl RotationOverLifetimeParams {
    fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let m = |k: &str| format!("{ctx}.rotationOverLifetime.{k}");
        Ok(Self {
            separate_axes: bool_of(obj_get(obj, "separateAxes"), &m("separateAxes"))?,
            curve: min_max_curve(obj_get(obj, "curve"), &m("curve"))?,
            x: obj_get(obj, "x")
                .map(|v| min_max_curve(Some(v), &m("x")))
                .transpose()?,
            y: obj_get(obj, "y")
                .map(|v| min_max_curve(Some(v), &m("y")))
                .transpose()?,
        })
    }
}

impl LimitVelocityParams {
    fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let m = |k: &str| format!("{ctx}.limitVelocity.{k}");
        Ok(Self {
            separate_axis: bool_of(obj_get(obj, "separateAxis"), &m("separateAxis"))?,
            magnitude: min_max_curve(obj_get(obj, "magnitude"), &m("magnitude"))?,
            dampen: f32_of(obj_get(obj, "dampen"), &m("dampen"))?,
            drag: opt_min_max_curve(obj_get(obj, "drag"), &m("drag"))?,
            in_world_space: bool_of(obj_get(obj, "inWorldSpace"), &m("inWorldSpace"))?,
            multiply_drag_by_size: opt_bool_of(obj_get(obj, "multiplyDragBySize"), &m("multiplyDragBySize"))?,
            multiply_drag_by_velocity: opt_bool_of(
                obj_get(obj, "multiplyDragByVelocity"),
                &m("multiplyDragByVelocity"),
            )?,
        })
    }
}

// —— MinMax 值的词汇表 ————————————————————————————————————

/// `constant{value}` / `twoConstants{min,max}` / `curve{multiplier,keys}` /
/// `twoCurves{multiplier,minKeys,maxKeys}`。模式之外的键被忽略（词汇
/// 表是闭集，多余键是上游将来扩展，不是损伤）。
// Positive infinity is a lifetime sentinel, not a general numeric JSON value.
// Keep it scoped to the constant lifetime arm: interpolation involving infinities
// has no finite random range and other particle fields must stay finite.
fn lifetime_curve(v: Option<&Value>, ctx: &str) -> Result<MinMaxCurve, EffectsError> {
    if v.and_then(|v| v.get("mode")).and_then(Value::as_str) == Some("constant")
        && v.and_then(|v| v.get("value")).and_then(Value::as_str) == Some("Infinity")
    {
        return Ok(MinMaxCurve::Constant(f32::INFINITY));
    }
    min_max_curve(v, ctx)
}

fn min_max_curve(v: Option<&Value>, ctx: &str) -> Result<MinMaxCurve, EffectsError> {
    let obj = v
        .and_then(Value::as_object)
        .ok_or_else(|| EffectsError(format!("{ctx}: MinMaxCurve object missing")))?;
    let mode = str_of(obj_get(obj, "mode"), &format!("{ctx}.mode"))?;
    match mode.as_str() {
        "constant" => Ok(MinMaxCurve::Constant(f32_of(
            obj_get(obj, "value"),
            &format!("{ctx}.value"),
        )?)),
        "twoConstants" => Ok(MinMaxCurve::TwoConstants {
            min: f32_of(obj_get(obj, "min"), &format!("{ctx}.min"))?,
            max: f32_of(obj_get(obj, "max"), &format!("{ctx}.max"))?,
        }),
        "curve" => Ok(MinMaxCurve::Curve {
            multiplier: f32_of(obj_get(obj, "multiplier"), &format!("{ctx}.multiplier"))?,
            max: curve_of(obj_get(obj, "keys"), &format!("{ctx}.keys"))?,
        }),
        "twoCurves" => Ok(MinMaxCurve::TwoCurves {
            multiplier: f32_of(obj_get(obj, "multiplier"), &format!("{ctx}.multiplier"))?,
            min: curve_of(obj_get(obj, "minKeys"), &format!("{ctx}.minKeys"))?,
            max: curve_of(obj_get(obj, "maxKeys"), &format!("{ctx}.maxKeys"))?,
        }),
        _ => Err(EffectsError(format!("{ctx}.mode: unknown {mode:?}"))),
    }
}

/// 键数组 → 曲线。阶跃切线保留有符号 Infinity；旧 null 编码丢失了符号，
/// 必须从源重新提取，不能推断为任意一端。加权键按位解析：激活位的权重
/// 必读，缺失即拒；未激活位的权重惰性（求值时代 1/3），缺失容。
fn curve_of(v: Option<&Value>, ctx: &str) -> Result<Curve, EffectsError> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}: key array missing")))?;
    let mut keys = Vec::with_capacity(arr.len());
    for (i, k) in arr.iter().enumerate() {
        let kctx = format!("{ctx}[{i}]");
        let weighted = u32_of(k.get("weightedMode"), &format!("{kctx}.weightedMode"))? as u8;
        let inert = f32::from_bits(0x3eaa_aaab);
        let in_weight = if weighted & 1 != 0 {
            f32_of(k.get("inWeight"), &format!("{kctx}.inWeight"))?
        } else {
            inert
        };
        let out_weight = if weighted & 2 != 0 {
            f32_of(k.get("outWeight"), &format!("{kctx}.outWeight"))?
        } else {
            inert
        };
        let slope = |name: &str| -> Result<f32, EffectsError> {
            let s = k.get(name).ok_or_else(|| {
                EffectsError(format!("{kctx}.{name}: missing tangent"))
            })?;
            match s.as_str() {
                Some("Infinity") => Ok(f32::INFINITY),
                Some("-Infinity") => Ok(f32::NEG_INFINITY),
                _ => f32_of(Some(s), &format!("{kctx}.{name}")),
            }
        };
        keys.push(CurveKey {
            time: f32_of(k.get("time"), &format!("{kctx}.time"))?,
            value: f32_of(k.get("value"), &format!("{kctx}.value"))?,
            in_slope: slope("inSlope")?,
            out_slope: slope("outSlope")?,
            weighted_mode: weighted,
            in_weight,
            out_weight,
        });
    }
    // 键序错是数据损伤：求值不排序（见 Curve 的 doc）。
    for w in keys.windows(2) {
        if w[0].time > w[1].time {
            return Err(EffectsError(format!("{ctx}: keys not sorted by time")));
        }
    }
    Ok(Curve { multiplier: 1.0, keys })
}

/// `color{color:[r,g,b,a]}` / `gradient{gradient}` / `twoColors{min,max}` /
/// `twoGradients{minGradient,maxGradient}` / `randomColor{gradient}`。
///
/// Random colour requires its authored gradient; a flat colour is lost source data.
fn min_max_gradient(v: Option<&Value>, ctx: &str) -> Result<MinMaxGradient, EffectsError> {
    let obj = v
        .and_then(Value::as_object)
        .ok_or_else(|| EffectsError(format!("{ctx}: MinMaxGradient object missing")))?;
    let mode = str_of(obj_get(obj, "mode"), &format!("{ctx}.mode"))?;
    match mode.as_str() {
        "color" => Ok(MinMaxGradient::Color(vec4_of(
            obj_get(obj, "color"),
            &format!("{ctx}.color"),
        )?)),
        "gradient" => Ok(MinMaxGradient::Gradient(gradient_of(
            obj_get(obj, "gradient"),
            &format!("{ctx}.gradient"),
        )?)),
        "twoColors" => Ok(MinMaxGradient::TwoColors {
            min: vec4_of(obj_get(obj, "min"), &format!("{ctx}.min"))?,
            max: vec4_of(obj_get(obj, "max"), &format!("{ctx}.max"))?,
        }),
        "twoGradients" => Ok(MinMaxGradient::TwoGradients {
            min: gradient_of(obj_get(obj, "minGradient"), &format!("{ctx}.minGradient"))?,
            max: gradient_of(obj_get(obj, "maxGradient"), &format!("{ctx}.maxGradient"))?,
        }),
        "randomColor" => Ok(MinMaxGradient::RandomColor(gradient_of(
            obj_get(obj, "gradient"), &format!("{ctx}.gradient"),
        )?)),
        _ => Err(EffectsError(format!("{ctx}.mode: unknown {mode:?}"))),
    }
}

fn gradient_of(v: Option<&Value>, ctx: &str) -> Result<Gradient, EffectsError> {
    let obj = v
        .and_then(Value::as_object)
        .ok_or_else(|| EffectsError(format!("{ctx}: gradient object missing")))?;
    use super::gradient::{GradientColorSpace, GradientMode};
    let mode = match str_of(obj_get(obj, "interpolation"), &format!("{ctx}.interpolation"))?.as_str() {
        "blend" => GradientMode::Blend,
        "fixed" => GradientMode::Fixed,
        other => return Err(EffectsError(format!("{ctx}.interpolation: unsupported {other:?}"))),
    };
    let color_space = match obj_get(obj, "colorSpace").and_then(Value::as_f64) {
        Some(-1.0) => GradientColorSpace::Unspecified,
        Some(0.0) => GradientColorSpace::Gamma,
        Some(1.0) => GradientColorSpace::Linear,
        other => return Err(EffectsError(format!("{ctx}.colorSpace: invalid or missing {other:?}"))),
    };
    let color_keys = obj_get(obj, "colorKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}.colorKeys: array missing")))?
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let kctx = format!("{ctx}.colorKeys[{i}]");
            let color = vec3_of(k.get("color"), &format!("{kctx}.color"))?;
            Ok(GradientColorKey {
                time: f32_of(k.get("time"), &format!("{kctx}.time"))?,
                color,
            })
        })
        .collect::<Result<Vec<_>, EffectsError>>()?;
    let alpha_keys = obj_get(obj, "alphaKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}.alphaKeys: array missing")))?
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let kctx = format!("{ctx}.alphaKeys[{i}]");
            Ok(GradientAlphaKey {
                time: f32_of(k.get("time"), &format!("{kctx}.time"))?,
                alpha: f32_of(k.get("alpha"), &format!("{kctx}.alpha"))?,
            })
        })
        .collect::<Result<Vec<_>, EffectsError>>()?;
    let validate_times = |times: Vec<f32>, field: &str| -> Result<(), EffectsError> {
        if !(2..=8).contains(&times.len()) || times.iter().any(|time| !(0.0..=1.0).contains(time))
            || times.windows(2).any(|pair| pair[0] > pair[1])
        {
            return Err(EffectsError(format!("{ctx}.{field}: expected 2..8 ordered keys in [0,1]")));
        }
        Ok(())
    };
    validate_times(color_keys.iter().map(|key| key.time).collect(), "colorKeys")?;
    validate_times(alpha_keys.iter().map(|key| key.time).collect(), "alphaKeys")?;
    Ok(Gradient { color_keys, alpha_keys, mode, color_space })
}

// —— 基元读取 ————————————————————————————————————

fn obj_get<'a>(obj: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn system_get<'a>(obj: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    obj_get(obj, key)
}

fn str_of(v: Option<&Value>, ctx: &str) -> Result<String, EffectsError> {
    v.and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EffectsError(format!("{ctx}: string expected")))
}

fn f32_of(v: Option<&Value>, ctx: &str) -> Result<f32, EffectsError> {
    v.and_then(Value::as_f64)
        .map(|n| n as f32)
        .filter(|n| n.is_finite())
        .ok_or_else(|| EffectsError(format!("{ctx}: finite number expected")))
}

fn bool_of(v: Option<&Value>, ctx: &str) -> Result<bool, EffectsError> {
    v.and_then(Value::as_bool)
        .ok_or_else(|| EffectsError(format!("{ctx}: bool expected")))
}

fn u32_of(v: Option<&Value>, ctx: &str) -> Result<u32, EffectsError> {
    let n = v.and_then(Value::as_f64)
        .ok_or_else(|| EffectsError(format!("{ctx}: non-negative integer expected")))?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
        Ok(n as u32)
    } else {
        Err(EffectsError(format!("{ctx}: non-negative integer expected, got {n}")))
    }
}

fn vec3_of(v: Option<&Value>, ctx: &str) -> Result<[f32; 3], EffectsError> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}: [x,y,z] array expected")))?;
    if arr.len() != 3 {
        return Err(EffectsError(format!(
            "{ctx}: expected 3 components, got {}",
            arr.len()
        )));
    }
    Ok([
        f32_of(Some(&arr[0]), ctx)?,
        f32_of(Some(&arr[1]), ctx)?,
        f32_of(Some(&arr[2]), ctx)?,
    ])
}

fn vec2_of(v: Option<&Value>, ctx: &str) -> Result<[f32; 2], EffectsError> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}: [a,b] array expected")))?;
    if arr.len() != 2 {
        return Err(EffectsError(format!(
            "{ctx}: expected 2 components, got {}",
            arr.len()
        )));
    }
    Ok([f32_of(Some(&arr[0]), ctx)?, f32_of(Some(&arr[1]), ctx)?])
}

fn vec4_of(v: Option<&Value>, ctx: &str) -> Result<[f32; 4], EffectsError> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| EffectsError(format!("{ctx}: [r,g,b,a] array expected")))?;
    if arr.len() != 4 {
        return Err(EffectsError(format!(
            "{ctx}: expected 4 components, got {}",
            arr.len()
        )));
    }
    Ok([
        f32_of(Some(&arr[0]), ctx)?,
        f32_of(Some(&arr[1]), ctx)?,
        f32_of(Some(&arr[2]), ctx)?,
        f32_of(Some(&arr[3]), ctx)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 覆盖全部词汇的最小合成档案（手写，非资产拷贝）。
    const SYNTHETIC: &str = r#"{
      "effects": {
        "fx_rain": {
          "kind": "site",
          "particles": [
            {
              "node": "root/drop",
              "system": {
                "duration": 5.0,
                "looping": true,
                "prewarm": true,
                "playOnAwake": true,
                "simulationSpeed": 1.0,
                "simulationSpace": "World",
                "randomSeed": 0,
                "autoRandomSeed": true,
                "startDelay": {"mode": "constant", "value": 0.0},
                "ringBufferMode": 0,
                "ringBufferLoopRange": [0.0, 1.0],
                "scalingMode": 1,
                "emitterVelocityMode": 1,
                "maxParticles": 300,
                "start": {
                  "lifetime": {"mode": "constant", "value": 5.0},
                  "speed": {"mode": "twoConstants", "min": 0.0, "max": 1.0},
                  "size": {"mode": "curve", "multiplier": 2.0,
                    "keys": [{"time": 0.0, "value": 0.0, "inSlope": 0.0,
                              "outSlope": 0.0, "weightedMode": 0,
                              "inWeight": 0.0, "outWeight": 0.0}]},
                  "rotation": {"mode": "constant", "value": 0.0},
                  "color": {"mode": "twoColors",
                            "min": [0.0, 0.0, 0.0, 1.0], "max": [1.0, 1.0, 1.0, 1.0]},
                  "gravityModifier": {"mode": "constant", "value": 0.5},
                  "size3D": false,
                  "rotation3D": false
                },
                "emission": {
                  "rateOverTime": {"mode": "constant", "value": 60.0},
                  "rateOverDistance": {"mode": "constant", "value": 0.0},
                  "bursts": [
                    {"time": 0.0, "count": {"mode": "constant", "value": 1.0},
                     "cycleCount": 1, "repeatInterval": 0.01, "probability": 1.0},
                    {"time": 1.0, "count": {"mode": "twoConstants", "min": 2.0, "max": 4.0},
                     "cycleCount": 3, "repeatInterval": 2.0, "probability": 0.25}
                  ]
                },
                "shape": {
                  "type": "Circle", "radius": 50.0, "radiusThickness": 1.0,
                  "angle": 25.0, "arc": 360.0,
                  "position": [0.0, 0.0, 0.0], "rotation": [-90.0, 0.0, 0.0]
                },
                "velocityOverLifetime": {
                  "x": {"mode": "twoConstants", "min": 0.0, "max": 0.0},
                  "y": {"mode": "twoConstants", "min": -0.5, "max": -1.0},
                  "z": {"mode": "twoConstants", "min": 0.0, "max": 0.0},
                  "speedModifier": {"mode": "constant", "value": 1.0},
                  "orbitalX": {"mode": "constant", "value": 0.0},
                  "orbitalY": {"mode": "constant", "value": 0.0},
                  "orbitalZ": {"mode": "constant", "value": 0.0},
                  "orbitalOffsetX": {"mode": "constant", "value": 0.0},
                  "orbitalOffsetY": {"mode": "constant", "value": 0.0},
                  "orbitalOffsetZ": {"mode": "constant", "value": 0.0},
                  "radial": {"mode": "constant", "value": 0.0},
                  "inWorldSpace": false
                },
                "colorOverLifetime": {
                  "mode": "gradient",
                  "gradient": {
                    "interpolation": "blend", "colorSpace": -1,
                    "colorKeys": [{"time": 0.0, "color": [1.0, 1.0, 1.0]},
                                  {"time": 1.0, "color": [1.0, 1.0, 1.0]}],
                    "alphaKeys": [{"time": 0.0, "alpha": 0.0},
                                  {"time": 1.0, "alpha": 1.0}]
                  }
                }
              },
              "renderer": {"kind": "billboard"}
            }
          ]
        }
      }
    }"#;

    #[test]
    fn missing_orbital_curve_is_not_implicitly_neutral() {
        let missing = SYNTHETIC.replace("\"orbitalX\": {\"mode\": \"constant\", \"value\": 0.0},", "");
        let error = Effects::from_json_str(missing.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("orbitalX"));
    }

    #[test]
    fn parses_full_synthetic_entry() {
        let fx = Effects::from_json_str(SYNTHETIC.as_bytes()).expect("synthetic must parse");
        assert_eq!(fx.emitters.len(), 1);
        let e = &fx.emitters[0];
        assert_eq!(e.effect, "fx_rain");
        assert_eq!(e.node, "root/drop");
        assert_eq!(e.duration, 5.0);
        assert!(e.looping && e.prewarm && e.play_on_awake);
        assert_eq!(e.simulation_space, SimulationSpace::World);
        assert_eq!(e.ring_buffer_mode, RingBufferMode::Disabled);
        assert_eq!(e.max_particles, 300);
        // start 的四种模式各就各位。
        assert_eq!(e.start.lifetime, MinMaxCurve::Constant(5.0));
        assert_eq!(
            e.start.speed,
            MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 }
        );
        match &e.start.size {
            MinMaxCurve::Curve { multiplier, max } => {
                assert_eq!(*multiplier, 2.0);
                assert_eq!(max.keys.len(), 1);
            }
            other => panic!("size mode: {other:?}"),
        }
        // burst 用户面口径直接落。
        let b = &e.emission.as_ref().unwrap().bursts[1];
        assert_eq!(b.cycles, BurstCycles::from_serialized(3));
        assert_eq!(b.repeat_interval, 2.0);
        assert_eq!(b.probability, 0.25);
        // shape 只映射圆参数。
        let s = e.shape.as_ref().unwrap();
        assert_eq!(s.shape_type, "Circle");
        assert_eq!(s.rotation, [-90.0, 0.0, 0.0]);
        assert_eq!(s.radius, 50.0);
        // VoL 落 y 的两常值。
        assert_eq!(
            e.velocity_over_lifetime.as_ref().unwrap().y,
            MinMaxCurve::TwoConstants { min: -0.5, max: -1.0 }
        );
        // 未映射键可见：system 层 4 个 + shape 的 angle + renderer。
        assert!(e.unmapped.contains(&"scalingMode".to_string()));
        assert!(e.unmapped.contains(&"emitterVelocityMode".to_string()));
        assert!(e.unmapped.contains(&"randomSeed".to_string()));
        assert!(e.unmapped.contains(&"autoRandomSeed".to_string()));
        assert_eq!(e.shape.as_ref().unwrap().controls.angle, Some(25.0));
        assert!(!e.unmapped.contains(&"shape.angle".to_string()));
        assert!(e.unmapped.contains(&"renderer".to_string()));
        // effect 层键（kind 等）收在 Effects 清单。
        assert!(fx.unmapped_effect_keys.contains(&"kind".to_string()));
    }

    #[test]
    fn synthetic_values_evaluate_against_hand_anchors() {
        // schema 落下来的值直接喂律，手算锚钉整条链。
        let fx = Effects::from_json_str(SYNTHETIC.as_bytes()).unwrap();
        let e = &fx.emitters[0];
        // 雨滴速度：VoL y 两常值 Lerp(-0.5,-1.0, r=0.5) = -0.75。
        let vol = e.velocity_over_lifetime.as_ref().unwrap();
        assert!((vol.y.evaluate(0.0, 0.5) + 0.75).abs() < 1e-6);
        // 颜色两常值中点。
        assert_eq!(
            e.start.color.evaluate(0.0, 0.5),
            [0.5, 0.5, 0.5, 1.0]
        );
        // colorOverLifetime 梯度 alpha 0.5 处 = 0.5。
        let col = e.color_over_lifetime.as_ref().unwrap();
        assert!((col.evaluate(0.5, 0.0)[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rejects_weighted_keys_missing_active_weight() {
        // 加权键本身可解析（求值走律的加权 Bezier 路）；承重边界是
        // 激活位的权重缺失即拒——缺会左右结果的键不当默认值。
        let bad = r#"{"effects":{"e":{"particles":[{"node":"n","system":{"duration":1.0,"looping":true,"prewarm":false,"playOnAwake":true,"simulationSpeed":1.0,"simulationSpace":"Local","startDelay":{"mode":"constant","value":0.0},"ringBufferMode":0,"ringBufferLoopRange":[0.0,1.0],"maxParticles":10,"start":{"lifetime":{"mode":"constant","value":1.0},"speed":{"mode":"constant","value":1.0},"size":{"mode":"constant","value":1.0},"rotation":{"mode":"constant","value":0.0},"color":{"mode":"color","color":[1.0,1.0,1.0,1.0]},"gravityModifier":{"mode":"constant","value":0.0},"size3D":false,"rotation3D":false},"sizeOverLifetime":{"separateAxes":false,"curve":{"mode":"curve","multiplier":1.0,"keys":[{"time":0.0,"value":0.0,"inSlope":0.0,"outSlope":0.0,"weightedMode":1,"outWeight":0.3}]}}}}]}}}"#;
        let err = Effects::from_json_str(bad.as_bytes()).unwrap_err();
        assert!(err.0.contains("inWeight"), "{}", err.0);
    }

    #[test]
    fn rejects_null_slope_step_keys() {
        let bad = r#"{"effects":{"e":{"particles":[{"node":"n","system":{"duration":1.0,"looping":true,"prewarm":false,"playOnAwake":true,"simulationSpeed":1.0,"simulationSpace":"Local","startDelay":{"mode":"constant","value":0.0},"ringBufferMode":0,"ringBufferLoopRange":[0.0,1.0],"maxParticles":10,"start":{"lifetime":{"mode":"constant","value":1.0},"speed":{"mode":"constant","value":1.0},"size":{"mode":"constant","value":1.0},"rotation":{"mode":"constant","value":0.0},"color":{"mode":"color","color":[1.0,1.0,1.0,1.0]},"gravityModifier":{"mode":"constant","value":0.0},"size3D":false,"rotation3D":false},"sizeOverLifetime":{"separateAxes":false,"curve":{"mode":"curve","multiplier":1.0,"keys":[{"time":0.0,"value":0.0,"inSlope":0.0,"outSlope":null,"weightedMode":0,"inWeight":0.0,"outWeight":0.0}]}}}}]}}}"#;
        let err = Effects::from_json_str(bad.as_bytes()).unwrap_err();
        assert!(err.0.contains("outSlope"), "{}", err.0);
    }

    #[test]
    fn signed_step_tangents_survive_particle_schema() {
        let keys = json::parse(br#"[
            {"time":0.0,"value":2.0,"inSlope":0.0,"outSlope":"Infinity","weightedMode":0},
            {"time":1.0,"value":7.0,"inSlope":"Infinity","outSlope":0.0,"weightedMode":0}
        ]"#).unwrap();
        let curve = curve_of(Some(&keys), "step").unwrap();
        assert_eq!(curve.evaluate(0.5), 2.0);
        assert_eq!(curve.evaluate(1.0), 7.0);
        let keys = json::parse(br#"[
            {"time":0.0,"value":2.0,"inSlope":0.0,"outSlope":"-Infinity","weightedMode":0},
            {"time":1.0,"value":7.0,"inSlope":"-Infinity","outSlope":0.0,"weightedMode":0}
        ]"#).unwrap();
        assert_eq!(curve_of(Some(&keys), "step").unwrap().evaluate(0.5), 7.0);
        // Native positive-infinity return precedes the negative check, even
        // when an authored segment supplies opposite signs at its two ends.
        let keys = json::parse(br#"[
            {"time":0.0,"value":2.0,"inSlope":0.0,"outSlope":"Infinity","weightedMode":2,"outWeight":0.25},
            {"time":1.0,"value":7.0,"inSlope":"-Infinity","outSlope":0.0,"weightedMode":1,"inWeight":0.4}
        ]"#).unwrap();
        assert_eq!(curve_of(Some(&keys), "step").unwrap().evaluate(0.5), 2.0);
    }

    #[test]
    fn rejects_unknown_mode_and_missing_particles() {
        let bad = r#"{"effects":{"e":{"particles":[{"node":"n","system":{"duration":1.0,"looping":true,"prewarm":false,"playOnAwake":true,"simulationSpeed":1.0,"simulationSpace":"Local","startDelay":{"mode":"wibble","value":0.0},"ringBufferMode":0,"ringBufferLoopRange":[0.0,1.0],"maxParticles":10,"start":{"lifetime":{"mode":"constant","value":1.0},"speed":{"mode":"constant","value":1.0},"size":{"mode":"constant","value":1.0},"rotation":{"mode":"constant","value":0.0},"color":{"mode":"color","color":[1.0,1.0,1.0,1.0]},"gravityModifier":{"mode":"constant","value":0.0},"size3D":false,"rotation3D":false}}}]}}}"#;
        let err = Effects::from_json_str(bad.as_bytes()).unwrap_err();
        assert!(err.0.contains("startDelay") && err.0.contains("wibble"), "{}", err.0);

        let no_particles = br#"{"effects": {"e": {"kind": "site"}}}"#;
        let err = Effects::from_json_str(no_particles).unwrap_err();
        assert!(err.0.contains("particles"), "{}", err.0);
    }

    #[test]
    fn random_color_rejects_flattened_source_gradient() {
        let bad = r#"{"effects":{"e":{"particles":[{"node":"n","system":{"duration":1.0,"looping":true,"prewarm":false,"playOnAwake":true,"simulationSpeed":1.0,"simulationSpace":"Local","startDelay":{"mode":"constant","value":0.0},"ringBufferMode":0,"ringBufferLoopRange":[0.0,1.0],"maxParticles":10,"start":{"lifetime":{"mode":"constant","value":1.0},"speed":{"mode":"constant","value":1.0},"size":{"mode":"constant","value":1.0},"rotation":{"mode":"constant","value":0.0},"color":{"mode":"randomColor","color":[0.25,0.5,0.75,1.0]},"gravityModifier":{"mode":"constant","value":0.0},"size3D":false,"rotation3D":false}}}]}}}"#;
        let error = Effects::from_json_str(bad.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("gradient"));
    }

    #[test]
    fn missing_required_number_is_rejected_with_location() {
        let bad = r#"{"effects":{"e":{"particles":[{"node":"n","system":{"duration":null,"looping":true,"prewarm":false,"playOnAwake":true,"simulationSpeed":1.0,"simulationSpace":"Local","startDelay":{"mode":"constant","value":0.0},"ringBufferMode":0,"ringBufferLoopRange":[0.0,1.0],"maxParticles":10,"start":{"lifetime":{"mode":"constant","value":1.0},"speed":{"mode":"constant","value":1.0},"size":{"mode":"constant","value":1.0},"rotation":{"mode":"constant","value":0.0},"color":{"mode":"color","color":[1.0,1.0,1.0,1.0]},"gravityModifier":{"mode":"constant","value":0.0},"size3D":false,"rotation3D":false}}}]}}}"#;
        let err = Effects::from_json_str(bad.as_bytes()).unwrap_err();
        assert!(err.0.contains("e/n.duration"), "{}", err.0);
    }
    #[test]
    fn source_shape_modes_and_curves_are_not_flattened() {
        let value = super::super::json::parse(br#"{
          "sourceVersion":1,"angle":25,"length":5,"donutRadius":2,
          "scale":[2,0.25,-3],"boxThickness":[0.1,0.2,0.3],
          "arcMode":"Loop","arcSpread":0.25,
          "arcSpeed":{"mode":"constant","value":0.7},
          "radiusMode":"BurstSpread","radiusSpread":0.5,
          "radiusSpeed":{"mode":"constant","value":2},
          "alignToDirection":false,"randomDirectionAmount":0,
          "sphericalDirectionAmount":0,"randomPositionAmount":0.15
        }"#).unwrap();
        let controls = ShapeControls::from_value(&value, "shape").unwrap();
        assert_eq!(controls.source_version,Some(1));
        assert_eq!(controls.arc_mode,Some(ShapeMode::Loop));
        assert_eq!(controls.radius_mode,Some(ShapeMode::BurstSpread));
        assert_eq!(controls.scale,Some([2.0,0.25,-3.0]));
        assert_eq!(controls.donut_radius,Some(2.0));
        assert_eq!(controls.random_position,Some(0.15));
        assert!(controls.arc_speed.is_some());
        let missing = super::super::json::parse(b"{}").unwrap();
        assert_eq!(ShapeControls::from_value(&missing,"shape").unwrap(),ShapeControls::default());
    }

    #[test]
    fn unknown_source_shape_modes_are_rejected_not_randomised() {
        let value = super::super::json::parse(br#"{"arcMode":"FutureMode"}"#).unwrap();
        assert!(ShapeControls::from_value(&value,"shape").unwrap_err().0.contains("shape.arcMode"));
    }

}
