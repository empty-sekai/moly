//! Decodes `effects.json` (the shape the rain family uses) into the emitter
//! parameters the laws take.
//!
//! Input: the file's bytes, or an already parsed value. Output: one
//! [`EmitterParams`] per emitted particle node, or an [`EffectsError`] naming
//! the effect and node. The schema is the data side: it decodes values and
//! decides which of them a law can compose, not what the law computes.
//!
//! The key shapes come from the corpus (in all 25 phenomenon archives each
//! MinMax value object has one key family): curves `constant{value}` /
//! `twoConstants{min,max}` / `curve{multiplier,keys}` /
//! `twoCurves{multiplier,minKeys,maxKeys}`; gradients `color{color}` /
//! `gradient{gradient}` / `twoColors{min,max}` /
//! `twoGradients{minGradient,maxGradient}` / `randomColor{gradient}`.
//! Decoding fails loudly: a present key in the wrong shape is damaged data,
//! and a silent default would hide which parameters were never wired.
//!
//! A key the decoder recognizes but does not map is neither dropped nor an
//! error: it goes into the named `unmapped` lists (scalingMode, renderer, and
//! the non-circle shape parameters), so a consumer can see that the data is
//! there and the laws do not read it.

use std::fmt;

mod events;
mod noise;
mod texture_sheet;
pub use noise::{NoiseParams, NoiseQuality};
pub use texture_sheet::TextureSheetParams;
pub use events::{CollisionMode, CollisionParams, CollisionQuality, CollisionType,
    ForceParams, InheritVelocityMode, InheritVelocityParams, PlaneLink, PlaneSource, SubEmitterParams, SubEmitterSourcePointer,
    SubEmitterTrigger, TrailMode, TrailParams, TrailTextureMode};

use crate::particle::buffer::RingBufferMode;
use crate::particle::emit::{Burst, BurstCycles};
use crate::particle::json::{self, Value};
use crate::particle::value::{
    Curve, CurveKey, Gradient, GradientAlphaKey, GradientColorKey, MinMaxCurve, MinMaxGradient,
};

/// A decode failure: one readable message located to the effect or node.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectsError(pub String);

impl fmt::Display for EffectsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "effects.json: {}", self.0)
    }
}

impl std::error::Error for EffectsError {}

/// Every emitter of one file, with the lists of unmapped keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Effects {
    pub emitters: Vec<EmitterParams>,
    /// Keys recognized at the effect level and not mapped (deduplicated).
    pub unmapped_effect_keys: Vec<String>,
}

impl Effects {
    /// Decodes from the file's bytes.
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
            // Of the effect level's keys only particles reaches the laws; the
            // rest go into the list.
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

/// The simulation space. `C# readable`: `ParticleSystemSimulationSpace`
/// Local=0, World=1, Custom=2.
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

/// The start module: the birth parameters, all MinMax values evaluated with
/// the caller's random lerp factor.
#[derive(Debug, Clone, PartialEq)]
pub struct StartParams {
    pub lifetime: MinMaxCurve,
    pub speed: MinMaxCurve,
    pub size: MinMaxCurve,
    /// The Y/Z curves of size3D (absent falls back to `size`).
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

/// The emission module.
#[derive(Debug, Clone, PartialEq)]
pub struct EmissionParams {
    pub rate_over_time: MinMaxCurve,
    pub rate_over_distance: MinMaxCurve,
    pub bursts: Vec<Burst>,
}

/// The shape module: only the parameters the Circle law reads are mapped; the
/// rest go into `unmapped`. A non-circle type is decoded, not refused, since
/// the schema is the data side; the caller of `shape::circle_position`
/// refuses it at sampling time.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeParams {
    pub shape_type: String,
    pub radius: f32,
    pub radius_thickness: f32,
    pub arc: f32,
    /// The Euler rotation in degrees; `shape::euler_rotate_deg` applies it.
    pub rotation: [f32; 3],
    /// The shape-space position, `_position` in the engine's shape module.
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
    /// ShapeModule's texture reference. `None` means the export carries no
    /// texture field (an older producer), so the value is undecided; it is
    /// not the null reference.
    pub texture: Option<ShapeTexture>,
    /// The mesh shape's placement (0 vertex, 1 edge, 2 triangle).
    pub mesh_placement: Option<u32>,
    pub mesh_normal_offset: Option<f32>,
    /// Whether the mesh shape takes its colours from the mesh and renderer
    /// materials (the mesh-emission block of a Mesh shape).
    pub mesh_use_colors: Option<bool>,
    /// Whether the mesh shape emits from one submesh only.
    pub mesh_use_material_index: Option<bool>,
    /// That submesh's index; the export writes it only when the filter is on.
    pub mesh_material_index: Option<u32>,
}

/// ShapeModule's texture reference. The exporter writes `null` for the null
/// reference and `{fileId, pathId}` for any other. A reference whose file
/// and path ids are both zero is the null reference too. The samplers read
/// so far (torus and cone volume) branch on it: a non-null texture makes
/// them read the texture controls and call ApplyTexture for each birth
/// group, which is not supported; the null reference skips both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeTexture {
    None,
    Reference { file_id: i32, path_id: String },
}

impl ShapeTexture {
    /// `Ok(None)` when the key is absent: the texture is undecided.
    fn from_value(value: Option<&Value>, ctx: &str) -> Result<Option<Self>, EffectsError> {
        Ok(match SubEmitterSourcePointer::from_value(value, ctx)? {
            SubEmitterSourcePointer::Missing => None,
            SubEmitterSourcePointer::Null => Some(Self::None),
            pointer if pointer.is_authored_null() => Some(Self::None),
            SubEmitterSourcePointer::Pointer { file_id, path_id } => Some(Self::Reference { file_id, path_id }),
        })
    }
}


/// The velocityOverLifetime module.
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

/// One slot of the customData module (`custom1` / `custom2`).
///
/// A per-particle custom stream: each component is its own MinMax curve,
/// evaluated per particle at its normalized age, and the result fills the
/// matching component of the vertex attribute. The shader-side selector reads
/// `coord = component * 10 + source`, where source 1 is `custom1`, 2 is
/// `custom2` and 0 is a constant zero vector.
///
/// Components beyond `componentCount` are not written and stay zero, so a
/// selector that names one reads zero.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomDataSlot {
    /// The number of components produced (1..=4).
    pub component_count: usize,
    /// The per-component curves; the length equals `component_count`.
    pub components: Vec<MinMaxCurve>,
}

/// The customData module: two slots, `None` when absent or `disabled`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomDataParams {
    pub custom1: Option<CustomDataSlot>,
    pub custom2: Option<CustomDataSlot>,
}

/// The sizeOverLifetime module; with separate axes `curve` is the X axis.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeOverLifetimeParams {
    pub separate_axes: bool,
    pub curve: MinMaxCurve,
    pub y: Option<MinMaxCurve>,
    pub z: Option<MinMaxCurve>,
}

/// The rotationOverLifetime module. `curve` is the z axis (the editor binds z
/// to the serialized name `curve`, the mirror image of the size module, where
/// `curve` is x); `x` and `y` are exported only with separateAxes (an absent
/// one means the key is missing from the export, not a zero; law construction
/// refuses it, see `RotationOverLifetime::from_parts`).
#[derive(Debug, Clone, PartialEq)]
pub struct RotationOverLifetimeParams {
    pub separate_axes: bool,
    pub curve: MinMaxCurve,
    pub x: Option<MinMaxCurve>,
    pub y: Option<MinMaxCurve>,
}

/// rotationBySpeed block (RotationBySpeedModule). As in rotationOverLifetime,
/// `curve` is the z axis and `x`/`y` are exported only with separate axes;
/// `range` is the speed range the module remaps to [0, 1]. The export carries
/// the block only for an enabled module.
#[derive(Debug, Clone, PartialEq)]
pub struct RotationBySpeedParams {
    pub separate_axes: bool,
    pub curve: MinMaxCurve,
    pub x: Option<MinMaxCurve>,
    pub y: Option<MinMaxCurve>,
    pub range: [f32; 2],
}

/// The limitVelocity module. A `drag` of null (written when the drag curve is
/// empty) is the canonical "skip the drag section"; the two multiplier flags
/// are not exported yet (`None` means unknown, and a nonzero drag is refused
/// at law construction, since a key that changes the result is not defaulted
/// to false); `in_world_space` is not read on the engine's planar-vector path
/// and the field exists so the data stays visible.
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

/// Every parameter of one emitter that the laws read.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitterParams {
    pub effect: String,
    pub node: String,
    pub duration: f32,
    pub looping: bool,
    pub prewarm: bool,
    pub play_on_awake: bool,
    pub simulation_speed: f32,
    /// Serialized useUnscaledTime: the system's frame reads Time.unscaledDeltaTime
    /// instead of Time.deltaTime. Missing/null stays unknown, never false.
    pub use_unscaled_time: Option<bool>,
    pub simulation_space: SimulationSpace,
    /// MainModule emitterVelocityMode as exported (0 Transform, 1 Rigidbody,
    /// 2 Custom). None when the key is absent or not a non-negative integer;
    /// the consumer refuses what it cannot read rather than assume Transform.
    pub emitter_velocity_mode: Option<u32>,
    /// Missing/null source ownership remains unknown; zero/false are authored values.
    pub random_seed: Option<u32>,
    pub auto_random_seed: Option<bool>,
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
    pub rotation_by_speed: Option<RotationBySpeedParams>,
    pub limit_velocity: Option<LimitVelocityParams>,
    /// The per-particle custom stream; `None` when both slots are absent or
    /// `disabled`.
    pub custom_data: Option<CustomDataParams>,
    pub sub_emitters: Vec<SubEmitterParams>,
    pub collision: Option<CollisionParams>,
    pub trails: Option<TrailParams>,
    pub force: Option<ForceParams>,
    pub inherit_velocity: Option<InheritVelocityParams>,
    pub texture_sheet: Option<TextureSheetParams>,
    pub noise: Option<NoiseParams>,
    /// Keys recognized at the system and particle levels and not mapped
    /// (deduplicated, in order).
    pub unmapped: Vec<String>,
}

/// The system-level keys mapped into parameters; every other key goes into
/// `unmapped`. scalingMode is recognized but deliberately not mapped: it
/// goes into `unmapped`, where a consumer can see that the data is present
/// and the laws do not read it.
const MAPPED_SYSTEM_KEYS: [&str; 32] = [
    "duration", "looping", "prewarm", "playOnAwake", "simulationSpeed", "useUnscaledTime",
    "simulationSpace", "emitterVelocityMode", "startDelay", "ringBufferMode", "ringBufferLoopRange",
    "maxParticles", "start", "emission", "shape", "velocityOverLifetime",
    "colorOverLifetime", "sizeOverLifetime", "rotationOverLifetime", "rotationBySpeed",
    "limitVelocity", "customData", "shapeEnabled", "subEmitters", "collision",
    "trails", "forceOverLifetime", "inheritVelocity", "textureSheet",
    "noise", "randomSeed", "autoRandomSeed",
];

/// The start-level keys that are mapped.
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
        // At the particle level, keys besides node and system (renderer and
        // others) go into the list.
        for (k, v) in p.as_object().unwrap_or(&[]) {
            if k != "node" && k != "system" {
                let label = if k == "renderer" {
                    // The renderer object's sub-keys are not expanded: the
                    // block belongs to the render side.
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
        let rotation_by_speed = match system_get(system, "rotationBySpeed") {
            Some(v) if !v.as_object().map(|o| o.is_empty()).unwrap_or(false) => {
                Some(RotationBySpeedParams::from_value(v, &ctx)?)
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
            use_unscaled_time: opt_bool_of(system_get(system, "useUnscaledTime"), &format!("{ctx}.useUnscaledTime"))?,
            random_seed: match system_get(system, "randomSeed") {
                None | Some(Value::Null) => None,
                value => Some(u32_of(value, &format!("{ctx}.randomSeed"))?),
            },
            auto_random_seed: opt_bool_of(system_get(system, "autoRandomSeed"), &format!("{ctx}.autoRandomSeed"))?,
            simulation_space: {
                let name = str_of(
                    system_get(system, "simulationSpace"),
                    &format!("{ctx}.simulationSpace"),
                )?;
                SimulationSpace::from_str(&name, &ctx)?
            },
            emitter_velocity_mode: system_get(system, "emitterVelocityMode")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= u32::MAX as f64)
                .map(|n| n as u32),
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
            rotation_by_speed,
            limit_velocity,
            sub_emitters: system_get(system, "subEmitters")
                .map(|v| events::sub_emitters(v, &ctx)).transpose()?.unwrap_or_default(),
            collision: system_get(system, "collision")
                .map(|v| CollisionParams::from_value(v, &ctx)).transpose()?,
            trails: system_get(system, "trails")
                .map(|v| TrailParams::from_value(v, &ctx)).transpose()?,
            force: system_get(system, "forceOverLifetime")
                .map(|v| ForceParams::from_value(v, &ctx)).transpose()?,
            inherit_velocity: system_get(system, "inheritVelocity")
                .map(|v| events::InheritVelocityParams::from_value(v, &ctx)).transpose()?,
            texture_sheet: system_get(system, "textureSheet")
                .map(|v| TextureSheetParams::from_value(v, &ctx)).transpose()?,
            noise: system_get(system, "noise")
                .map(|v| NoiseParams::from_value(v, &ctx)).transpose()?,
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
        // The serialized scalars, as InitialModule's load clamps them.
        let (life, speed, size, rotation) = (StartClamp::Lifetime, StartClamp::Speed, StartClamp::Size, StartClamp::Rotation);
        Ok(Self {
            lifetime: life.apply(lifetime_curve(obj_get(obj, "lifetime"), &format!("{ctx}.start.lifetime"))?),
            speed: speed.apply(min_max_curve(obj_get(obj, "speed"), &format!("{ctx}.start.speed"))?),
            size: size.apply(min_max_curve(obj_get(obj, "size"), &format!("{ctx}.start.size"))?),
            size_y: obj_get(obj, "sizeY")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.sizeY")).map(|c| size.apply(c)))
                .transpose()?,
            size_z: obj_get(obj, "sizeZ")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.sizeZ")).map(|c| size.apply(c)))
                .transpose()?,
            size3d: bool_of(obj_get(obj, "size3D"), &format!("{ctx}.start.size3D"))?,
            rotation: rotation.apply(min_max_curve(obj_get(obj, "rotation"), &format!("{ctx}.start.rotation"))?),
            rotation_x: obj_get(obj, "rotationX")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.rotationX")).map(|c| rotation.apply(c)))
                .transpose()?,
            rotation_y: obj_get(obj, "rotationY")
                .map(|v| min_max_curve(Some(v), &format!("{ctx}.start.rotationY")).map(|c| rotation.apply(c)))
                .transpose()?,
            rotation3d: bool_of(obj_get(obj, "rotation3D"), &format!("{ctx}.start.rotation3D"))?,
            color: start_color(obj_get(obj, "color"), &format!("{ctx}.start.color"))?,
            gravity_modifier: min_max_curve(
                obj_get(obj, "gravityModifier"),
                &format!("{ctx}.start.gravityModifier"),
            )?,
        })
    }
}

/// The ranges InitialModule's load (its Transfer) clamps the start curves'
/// scalars into: first the scalar the curve's polynomial is built with (the
/// constant, the two-constant maximum or the curve multiplier), then the
/// two-constant minimum. The lifetime is at least 1e-4 (`fmax`); the speed
/// lies within +-1e5; each size axis within [0, 1e5], a negative value
/// becoming +0; each rotation axis within +-1745.3293 (1e5 degrees in
/// radians). Each bounded clamp is a compare against the lower bound and an
/// `fmin` against the upper, so a NaN scalar stays NaN, as through `fmax`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StartClamp {
    Lifetime,
    Speed,
    Size,
    Rotation,
}

impl StartClamp {
    pub(crate) fn scalar(self, v: f32) -> f32 {
        let fmin = |a: f32, b: f32| if a.is_nan() { a } else { a.min(b) };
        let bounded = |v: f32, lo: u32, hi: u32| {
            if v < f32::from_bits(lo) { f32::from_bits(lo) } else { fmin(v, f32::from_bits(hi)) }
        };
        match self {
            Self::Lifetime => if v.is_nan() { v } else { v.max(f32::from_bits(0x38d1_b717)) },
            Self::Speed => bounded(v, 0xc7c3_5000, 0x47c3_5000),
            Self::Size => if v < 0.0 { 0.0 } else { fmin(v, f32::from_bits(0x47c3_5000)) },
            Self::Rotation => bounded(v, 0xc4da_2a89, 0x44da_2a89),
        }
    }

    pub(crate) fn apply(self, curve: MinMaxCurve) -> MinMaxCurve {
        match curve {
            MinMaxCurve::Constant(v) => MinMaxCurve::Constant(self.scalar(v)),
            MinMaxCurve::TwoConstants { min, max } => {
                let max = self.scalar(max);
                MinMaxCurve::TwoConstants { min: self.scalar(min), max }
            }
            MinMaxCurve::Curve { multiplier, max } => MinMaxCurve::Curve { multiplier: self.scalar(multiplier), max },
            MinMaxCurve::TwoCurves { multiplier, min, max } => {
                MinMaxCurve::TwoCurves { multiplier: self.scalar(multiplier), min, max }
            }
        }
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
                    // A serialized `cycleCount == 0` means repeat without
                    // end, not an invalid value: the Editor's cycle dropdown
                    // writes the literal 0 for its item 0 and the managed
                    // setter rejects only `< 0`. A decoder that refused 0
                    // would drop every long-lived emitter unread.
                    let cycles = BurstCycles::from_serialized(u32_of(
                        b.get("cycleCount"),
                        &format!("{bctx}.cycleCount"),
                    )?);
                    Ok(Burst {
                        time: f32_of(b.get("time"), &format!("{bctx}.time"))?,
                        count,
                        cycles,
                        repeat_interval: burst_repeat_interval(
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

/// Current source lens-flare bursts store +Infinity with cycleCount=0. Keep
/// that exact interval and cycle count; this decoder does not qualify their
/// runtime scheduling. Other fields retain their finite-number contracts.
fn burst_repeat_interval(v: Option<&Value>, ctx: &str) -> Result<f32, EffectsError> {
    if v.and_then(Value::as_str) == Some("Infinity") {
        return Ok(f32::INFINITY);
    }
    f32_of(v, ctx)
}

/// The closed set of shape-level keys that are mapped; the rest (the six
/// texture control keys besides meshes and texture) go to unmapped.
const MAPPED_SHAPE_KEYS: &[&str] = &[
    "type", "radius", "radiusThickness", "arc", "rotation", "position", "sourceVersion",
    "angle", "length", "donutRadius", "scale", "boxThickness", "arcMode", "arcSpread", "arcSpeed",
    "radiusMode", "radiusSpread", "radiusSpeed", "alignToDirection", "randomDirectionAmount",
    "sphericalDirectionAmount", "randomPositionAmount", "texture",
    "meshPlacement", "meshNormalOffset", "meshMaterialIndex", "meshEmission",
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
        let mesh_flag = |key: &str| value.get("meshEmission").and_then(|m| m.get(key))
            .filter(|v| !matches!(v, Value::Null))
            .map(|v| bool_of(Some(v), &format!("{ctx}.meshEmission.{key}"))).transpose();
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
            texture:ShapeTexture::from_value(value.get("texture"),&format!("{ctx}.texture"))?,
            mesh_placement:value.get("meshPlacement").filter(|v|!matches!(v,Value::Null))
                .map(|v|u32_of(Some(v),&format!("{ctx}.meshPlacement"))).transpose()?,
            mesh_normal_offset:number("meshNormalOffset")?,
            mesh_use_colors:mesh_flag("useColors")?,
            mesh_use_material_index:mesh_flag("useMaterialIndex")?,
            mesh_material_index:value.get("meshMaterialIndex").filter(|v|!matches!(v,Value::Null))
                .map(|v|u32_of(Some(v),&format!("{ctx}.meshMaterialIndex"))).transpose()?,
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
    /// `None` when both slots are absent or `disabled`: no stream to produce.
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
            // The `color` mode holds a MinMaxGradient rather than per-component
            // curves; the site corpus has 0 of them, so it is refused loudly
            // (0 such cases) rather than being read as a vector.
            "vector" => {
                let count = f32_of(
                    obj_get(obj, "componentCount"),
                    &format!("{ctx}.componentCount"),
                )? as usize;
                if count == 0 || count > 4 {
                    return Err(EffectsError(format!(
                        "{ctx}.componentCount: {count} is outside 1..=4"
                    )));
                }
                let components = obj_get(obj, "components")
                    .and_then(Value::as_array)
                    .ok_or_else(|| EffectsError(format!("{ctx}.components: array missing")))?;
                if components.len() < count {
                    return Err(EffectsError(format!(
                        "{ctx}.components: {} curves, fewer than componentCount {count}",
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
            other => Err(EffectsError(format!("{ctx}.mode: the mode {other} is unsupported"))),
        }
    }
}

impl SizeOverLifetimeParams {
    pub(crate) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
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

/// A MinMax object or null: null or absent gives `None` (the canonical form
/// for skipping the drag section); any other shape is refused loudly by
/// `min_max_curve`.
fn opt_min_max_curve(v: Option<&Value>, ctx: &str) -> Result<Option<MinMaxCurve>, EffectsError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => min_max_curve(Some(v), ctx).map(Some),
    }
}

/// An optional boolean: absent or null gives `None` (not exported, so
/// unknown); a present value in the wrong shape is still refused loudly.
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

impl RotationBySpeedParams {
    pub(crate) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let obj = v.as_object().unwrap_or(&[]);
        let m = |k: &str| format!("{ctx}.rotationBySpeed.{k}");
        Ok(Self {
            separate_axes: bool_of(obj_get(obj, "separateAxes"), &m("separateAxes"))?,
            curve: min_max_curve(obj_get(obj, "curve"), &m("curve"))?,
            x: obj_get(obj, "x").map(|v| min_max_curve(Some(v), &m("x"))).transpose()?,
            y: obj_get(obj, "y").map(|v| min_max_curve(Some(v), &m("y"))).transpose()?,
            range: vec2_of(obj_get(obj, "range"), &m("range"))?,
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

// ---- MinMax value vocabulary ----

/// `constant{value}` / `twoConstants{min,max}` / `curve{multiplier,keys}` /
/// `twoCurves{multiplier,minKeys,maxKeys}`. Keys outside the mode are
/// ignored: the vocabulary is closed, so an extra key is a later upstream
/// extension rather than damage.
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

pub(crate) fn min_max_curve(v: Option<&Value>, ctx: &str) -> Result<MinMaxCurve, EffectsError> {
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
            max: wrapped(curve_of(obj_get(obj, "keys"), &format!("{ctx}.keys"))?,
                obj, ["preInfinity", "postInfinity"], ctx)?,
        }),
        "twoCurves" => Ok(MinMaxCurve::TwoCurves {
            multiplier: f32_of(obj_get(obj, "multiplier"), &format!("{ctx}.multiplier"))?,
            min: wrapped(curve_of(obj_get(obj, "minKeys"), &format!("{ctx}.minKeys"))?,
                obj, ["minPreInfinity", "minPostInfinity"], ctx)?,
            max: wrapped(curve_of(obj_get(obj, "maxKeys"), &format!("{ctx}.maxKeys"))?,
                obj, ["maxPreInfinity", "maxPostInfinity"], ctx)?,
        }),
        _ => Err(EffectsError(format!("{ctx}.mode: unknown {mode:?}"))),
    }
}

/// The lane's serialized wrap modes (`m_PreInfinity`, `m_PostInfinity`). An
/// absent or null field stays unknown: only a lane the reader leaves
/// unoptimized reads it, and that evaluator refuses an unknown wrap. A present
/// field that is not a non-negative integer is a parse error.
fn wrapped(mut curve: Curve, obj: &[(String, Value)], keys: [&str; 2], ctx: &str) -> Result<Curve, EffectsError> {
    let wrap = |key: &str| match obj_get(obj, key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => u32_of(Some(v), &format!("{ctx}.{key}")).map(Some),
    };
    curve.pre_wrap = wrap(keys[0])?;
    curve.post_wrap = wrap(keys[1])?;
    Ok(curve)
}

/// A key array to a curve. A step tangent keeps its signed Infinity; the old
/// null encoding lost the sign, so the value has to be re-extracted from the
/// source rather than inferred as either end. A weighted key is decoded bit
/// by bit: a set bit's weight is read and refused when missing; an unset
/// bit's weight is inert (evaluated as 1/3) and may be missing.
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
    // A key out of order is damaged data: evaluation does not sort (see the
    // `Curve` documentation).
    for w in keys.windows(2) {
        if w[0].time > w[1].time {
            return Err(EffectsError(format!("{ctx}: keys not sorted by time")));
        }
    }
    Ok(Curve { multiplier: 1.0, keys, pre_wrap: None, post_wrap: None })
}

/// `color{color:[r,g,b,a]}` / `gradient{gradient}` / `twoColors{min,max}` /
/// `twoGradients{minGradient,maxGradient}` / `randomColor{gradient}`。
///
/// Random colour requires its authored gradient; a flat colour is lost source data.
/// Every gradient consumer but the start colour takes 2..8 keys per group in
/// time order: the start colour's evaluation is the only one supported for
/// a single key and for keys out of order (`start_color`).
pub(crate) fn min_max_gradient(v: Option<&Value>, ctx: &str) -> Result<MinMaxGradient, EffectsError> {
    min_max_gradient_with(v, ctx, KeyTable::Ordered)
}

/// The start colour block: 1..8 keys per group in any time order, as the
/// native table holds them. Crate-visible so native start-colour replays
/// decode exported blocks with this decoder rather than a second one.
pub(crate) fn start_color(v: Option<&Value>, ctx: &str) -> Result<MinMaxGradient, EffectsError> {
    min_max_gradient_with(v, ctx, KeyTable::StartColor)
}

/// Which gradient key tables a decode admits.
#[derive(Clone, Copy, PartialEq)]
enum KeyTable {
    /// 2..8 keys per group, times non-decreasing.
    Ordered,
    /// 1..8 keys per group, any order.
    StartColor,
}

fn min_max_gradient_with(
    v: Option<&Value>,
    ctx: &str,
    keys: KeyTable,
) -> Result<MinMaxGradient, EffectsError> {
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
            keys,
        )?)),
        "twoColors" => Ok(MinMaxGradient::TwoColors {
            min: vec4_of(obj_get(obj, "min"), &format!("{ctx}.min"))?,
            max: vec4_of(obj_get(obj, "max"), &format!("{ctx}.max"))?,
        }),
        "twoGradients" => Ok(MinMaxGradient::TwoGradients {
            min: gradient_of(obj_get(obj, "minGradient"), &format!("{ctx}.minGradient"), keys)?,
            max: gradient_of(obj_get(obj, "maxGradient"), &format!("{ctx}.maxGradient"), keys)?,
        }),
        "randomColor" => Ok(MinMaxGradient::RandomColor(gradient_of(
            obj_get(obj, "gradient"), &format!("{ctx}.gradient"), keys,
        )?)),
        _ => Err(EffectsError(format!("{ctx}.mode: unknown {mode:?}"))),
    }
}

fn gradient_of(v: Option<&Value>, ctx: &str, keys: KeyTable) -> Result<Gradient, EffectsError> {
    let obj = v
        .and_then(Value::as_object)
        .ok_or_else(|| EffectsError(format!("{ctx}: gradient object missing")))?;
    use super::gradient::{GradientColorSpace, GradientMode};
    let mode = match str_of(obj_get(obj, "interpolation"), &format!("{ctx}.interpolation"))?.as_str() {
        "blend" => GradientMode::Blend,
        "fixed" => GradientMode::Fixed,
        other => return Err(EffectsError(format!("{ctx}.interpolation: unsupported {other:?}"))),
    };
    // Decode integrity of the exported ColorSpace enum (-1 uninitialized, 0
    // gamma, 1 linear). It is not a start-colour envelope: the native gradient
    // kernels never read the colour-space byte.
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
        let in_range = times.iter().all(|time| (0.0..=1.0).contains(time));
        let admitted = match keys {
            KeyTable::Ordered => {
                (2..=8).contains(&times.len()) && times.windows(2).all(|pair| pair[0] <= pair[1])
            }
            KeyTable::StartColor => (1..=8).contains(&times.len()),
        };
        if !in_range || !admitted {
            let expected = match keys {
                KeyTable::Ordered => "2..8 ordered keys in [0,1]",
                KeyTable::StartColor => "1..8 keys in [0,1]",
            };
            return Err(EffectsError(format!("{ctx}.{field}: expected {expected}")));
        }
        Ok(())
    };
    validate_times(color_keys.iter().map(|key| key.time).collect(), "colorKeys")?;
    validate_times(alpha_keys.iter().map(|key| key.time).collect(), "alphaKeys")?;
    Ok(Gradient { color_keys, alpha_keys, mode, color_space })
}

// ---- Primitive readers ----

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

    /// The smallest synthetic file that covers the whole vocabulary; it is
    /// written by hand, not copied from an asset.
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
    fn burst_interval_preserves_positive_infinity_and_original_cycle_count() {
        for cycles in [0, 1, 3] {
            let source = SYNTHETIC.replace(
                "\"cycleCount\": 1, \"repeatInterval\": 0.01",
                &format!("\"cycleCount\": {cycles}, \"repeatInterval\": \"Infinity\""),
            );
            let effects = Effects::from_json_str(source.as_bytes()).unwrap();
            let burst = &effects.emitters[0].emission.as_ref().unwrap().bursts[0];
            assert_eq!(burst.repeat_interval.to_bits(), f32::INFINITY.to_bits());
            assert_eq!(burst.cycles, BurstCycles::from_serialized(cycles));
        }
        for interval in [0.0_f32, -0.0, 0.01, 2.0] {
            let source = SYNTHETIC.replace(
                "\"repeatInterval\": 0.01",
                &format!("\"repeatInterval\": {interval:?}"),
            );
            let effects = Effects::from_json_str(source.as_bytes()).unwrap();
            let burst = &effects.emitters[0].emission.as_ref().unwrap().bursts[0];
            assert_eq!(burst.repeat_interval.to_bits(), interval.to_bits());
        }
    }

    #[test]
    fn burst_infinity_exception_is_field_specific_and_rejects_other_spellings() {
        for interval in ["\"-Infinity\"", "\"NaN\"", "\"inf\"", "\"1\"", "null", "true", "1e300"] {
            let source = SYNTHETIC.replace(
                "\"repeatInterval\": 0.01",
                &format!("\"repeatInterval\": {interval}"),
            );
            let error = Effects::from_json_str(source.as_bytes()).unwrap_err();
            assert!(error.0.contains("effect fx_rain/root/drop.emission.bursts[0].repeatInterval"), "{error}");
        }
        for (before, after, field) in [
            ("\"time\": 0.0, \"count\"", "\"time\": \"Infinity\", \"count\"", "time"),
            ("\"probability\": 1.0", "\"probability\": \"Infinity\"", "probability"),
            ("\"count\": {\"mode\": \"constant\", \"value\": 1.0}",
             "\"count\": {\"mode\": \"constant\", \"value\": \"Infinity\"}", "count.value"),
        ] {
            let source = SYNTHETIC.replace(before, after);
            let error = Effects::from_json_str(source.as_bytes()).unwrap_err();
            assert!(error.0.contains(&format!("effect fx_rain/root/drop.emission.bursts[0].{field}")), "{error}");
        }
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
        // Each of the four start modes lands in its own variant.
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
        // The burst fields decode straight from the serialized form.
        let b = &e.emission.as_ref().unwrap().bursts[1];
        assert_eq!(b.cycles, BurstCycles::from_serialized(3));
        assert_eq!(b.repeat_interval, 2.0);
        assert_eq!(b.probability, 0.25);
        // The shape maps the circle parameters only.
        let s = e.shape.as_ref().unwrap();
        assert_eq!(s.shape_type, "Circle");
        assert_eq!(s.rotation, [-90.0, 0.0, 0.0]);
        assert_eq!(s.radius, 50.0);
        // The velocity over lifetime keeps two constants on y.
        assert_eq!(
            e.velocity_over_lifetime.as_ref().unwrap().y,
            MinMaxCurve::TwoConstants { min: -0.5, max: -1.0 }
        );
        // Typed seed ownership preserves explicit zero/true without starting RNG.
        assert_eq!(e.random_seed, Some(0));
        assert_eq!(e.auto_random_seed, Some(true));
        // Unmapped keys stay visible: scalingMode at the system level, and
        // renderer.
        assert!(e.unmapped.contains(&"scalingMode".to_string()));
        assert!(!e.unmapped.contains(&"randomSeed".to_string()));
        assert!(!e.unmapped.contains(&"autoRandomSeed".to_string()));
        assert_eq!(e.shape.as_ref().unwrap().controls.angle, Some(25.0));
        assert!(!e.unmapped.contains(&"shape.angle".to_string()));
        assert!(e.unmapped.contains(&"renderer".to_string()));
        // Effect-level keys (kind and others) go into the Effects list.
        assert!(fx.unmapped_effect_keys.contains(&"kind".to_string()));
    }

    #[test]
    fn source_seed_unknown_is_distinct_from_explicit_zero_and_manual_false() {
        for source in [
            SYNTHETIC.replace("\"randomSeed\": 0,", "").replace("\"autoRandomSeed\": true,", ""),
            SYNTHETIC.replace("\"randomSeed\": 0", "\"randomSeed\": null")
                .replace("\"autoRandomSeed\": true", "\"autoRandomSeed\": null"),
        ] {
            let emitter = Effects::from_json_str(source.as_bytes()).unwrap().emitters.remove(0);
            assert_eq!(emitter.random_seed, None);
            assert_eq!(emitter.auto_random_seed, None);
        }
        for seed in [0, u32::MAX] {
            let source = SYNTHETIC.replace("\"randomSeed\": 0", &format!("\"randomSeed\": {seed}"))
                .replace("\"autoRandomSeed\": true", "\"autoRandomSeed\": false");
            let emitter = Effects::from_json_str(source.as_bytes()).unwrap().emitters.remove(0);
            assert_eq!(emitter.random_seed, Some(seed));
            assert_eq!(emitter.auto_random_seed, Some(false));
        }
    }

    #[test]
    fn malformed_seed_controls_fail_with_source_location_instead_of_defaulting() {
        for seed in ["-1", "4294967296", "0.5", "true", "\"0\""] {
            let source = SYNTHETIC.replace("\"randomSeed\": 0", &format!("\"randomSeed\": {seed}"));
            let error = Effects::from_json_str(source.as_bytes()).unwrap_err();
            assert!(error.0.contains("effect fx_rain/root/drop.randomSeed"), "{error}");
        }
        let source = SYNTHETIC.replace("\"autoRandomSeed\": true", "\"autoRandomSeed\": 0");
        let error = Effects::from_json_str(source.as_bytes()).unwrap_err();
        assert!(error.0.contains("effect fx_rain/root/drop.autoRandomSeed"), "{error}");
    }

    #[test]
    fn emitter_preserves_noise_block_and_keeps_unqualified_controls_for_law_gate() {
        let noise = r#""noise":{"separateAxes":true,
            "strength":{"mode":"twoConstants","min":0.1,"max":0.9},
            "strengthY":{"mode":"constant","value":2},"strengthZ":{"mode":"constant","value":3},
            "frequency":0.5,"damping":false,"octaves":3,"octaveMultiplier":0.4,"octaveScale":2,
            "quality":"medium","dimensions":2,"scrollSpeed":{"mode":"constant","value":-1},
            "remapEnabled":true,"remap":{"mode":"constant","value":4},
            "remapY":{"mode":"constant","value":5},"remapZ":{"mode":"constant","value":6},
            "positionAmount":{"mode":"constant","value":7},"rotationAmount":{"mode":"constant","value":8},
            "sizeAmount":{"mode":"constant","value":9}},"duration": 5.0"#;
        let source = SYNTHETIC.replace("\"duration\": 5.0", noise);
        let emitter = Effects::from_json_str(source.as_bytes()).unwrap().emitters.remove(0);
        assert!(!emitter.unmapped.iter().any(|key| key == "noise"));
        let params = emitter.noise.unwrap();
        assert_eq!(params.quality, NoiseQuality::Medium);
        assert_eq!(params.dimensions, 2);
        assert_eq!(params.octaves, 3);
        assert_eq!(params.strength, MinMaxCurve::TwoConstants { min: 0.1, max: 0.9 });
        assert_eq!(params.remap_z, MinMaxCurve::Constant(6.0));
        assert!(crate::particle::noise::NoiseLaw::from_params(&params).is_err());
    }

    #[test]
    fn synthetic_values_evaluate_against_hand_anchors() {
        // The values the schema decodes feed the laws directly, and
        // hand-computed anchors pin the whole chain.
        let fx = Effects::from_json_str(SYNTHETIC.as_bytes()).unwrap();
        let e = &fx.emitters[0];
        // Raindrop speed: velocity over lifetime y has two constants,
        // Lerp(-0.5,-1.0, r=0.5) = -0.75.
        let vol = e.velocity_over_lifetime.as_ref().unwrap();
        assert!((vol.y.evaluate(0.0, 0.5) + 0.75).abs() < 1e-6);
        // The midpoint of two constant colours.
        assert_eq!(
            e.start.color.evaluate(0.0, 0.5),
            [0.5, 0.5, 0.5, 1.0]
        );
        // The colorOverLifetime gradient's alpha is 0.5 at 0.5.
        let col = e.color_over_lifetime.as_ref().unwrap();
        assert!((col.evaluate(0.5, 0.0)[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rejects_weighted_keys_missing_active_weight() {
        // A weighted key decodes on its own (evaluation takes the law's
        // weighted Bezier path); the load-bearing edge is that a set bit's
        // missing weight is refused, since a key that changes the result is
        // not defaulted.
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

    /// InitialModule's load clamps against the reference rows (rows
    /// `field input output`: 0 lifetime maximum, 1 lifetime minimum, 2 speed
    /// maximum, 3 speed minimum, 4 size, 5 rotation), NaN as a class. Each
    /// plausible misreading must mismatch somewhere on the same rows.
    #[test]
    #[ignore = "MOLY_INITIAL_CLAMP_ROWS must identify the native load clamp rows"]
    fn start_load_clamps_match_native_transfer() {
        let text = std::fs::read_to_string(std::env::var_os("MOLY_INITIAL_CLAMP_ROWS").expect("MOLY_INITIAL_CLAMP_ROWS")).unwrap();
        let rows: Vec<[u32; 3]> = text.lines().filter(|l| !l.trim().is_empty()).map(|l| {
            let v: Vec<u32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        }).collect();
        let rule = |field: u32| match field {
            0 | 1 => StartClamp::Lifetime,
            2 | 3 => StartClamp::Speed,
            4 => StartClamp::Size,
            5 => StartClamp::Rotation,
            other => panic!("field {other}"),
        };
        let mismatches = |clamp: &dyn Fn(u32, f32) -> f32| rows.iter().filter(|&&[field, input, native]| {
            let ours = clamp(field, f32::from_bits(input));
            !(ours.to_bits() == native || (ours.is_nan() && f32::from_bits(native).is_nan()))
        }).count();
        let product = mismatches(&|field, v| rule(field).scalar(v));
        println!("load clamp rows {}, mismatched {product}", rows.len());
        assert!(rows.len() > 6 && product == 0);
        let arms: [(&str, &dyn Fn(u32, f32) -> f32); 4] = [
            ("lifetimeFloorIsStartMinimum", &|field, v| if field < 2 { v.max(f32::from_bits(0x3727_c5ac)) } else { rule(field).scalar(v) }),
            ("negativeSizeKept", &|field, v| if field == 4 { v.min(f32::from_bits(0x47c3_5000)) } else { rule(field).scalar(v) }),
            ("speedUnclamped", &|field, v| if field == 2 || field == 3 { v } else { rule(field).scalar(v) }),
            ("rotationInDegrees", &|field, v| if field == 5 { v.clamp(-1.0e5, 1.0e5) } else { rule(field).scalar(v) }),
        ];
        for (name, arm) in arms {
            let red = mismatches(arm);
            println!("arm {name}: mismatched {red}");
            assert!(red > 0, "arm {name} stays green");
        }
    }
}
