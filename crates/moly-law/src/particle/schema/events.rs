//! Serialized event, collision and trail controls. Decoding preserves dormant
//! arms; the simulation adapter is responsible for their actual execution.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubEmitterTrigger { Birth, Collision, Death, Trigger, Manual }

#[derive(Debug, Clone, PartialEq)]
pub struct SubEmitterParams {
    pub emitter: Option<String>,
    pub trigger: SubEmitterTrigger,
    pub properties: u32,
    pub probability: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionType { Planes, World }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionMode { ThreeDimensional, TwoDimensional }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionQuality { High, Medium, Low }

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionParams {
    pub kind: CollisionType,
    pub mode: CollisionMode,
    pub quality: CollisionQuality,
    pub dampen: MinMaxCurve,
    pub bounce: MinMaxCurve,
    pub lifetime_loss: MinMaxCurve,
    pub min_kill_speed: f32,
    pub max_kill_speed: f32,
    pub radius_scale: f32,
    pub voxel_size: f32,
    pub collides_with: u32,
    pub dynamic: bool,
    pub interior: bool,
    pub max_shapes: u32,
    pub messages: bool,
    pub collider_force: f32,
    pub multiply_force_by_size: bool,
    pub multiply_force_by_speed: bool,
    pub multiply_force_by_angle: bool,
    pub plane_slots: u32,
    pub planes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailMode { PerParticle, Ribbon }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailTextureMode { Stretch, Tile, DistributePerSegment, RepeatPerSegment }

#[derive(Debug, Clone, PartialEq)]
pub struct TrailParams {
    pub mode: TrailMode,
    pub ratio: f32,
    pub lifetime: MinMaxCurve,
    pub min_vertex_distance: f32,
    pub texture_mode: TrailTextureMode,
    pub texture_scale: [f32; 2],
    pub ribbon_count: u32,
    pub shadow_bias: f32,
    pub world_space: bool,
    pub die_with_particles: bool,
    pub size_affects_width: bool,
    pub size_affects_lifetime: bool,
    pub inherit_particle_color: bool,
    pub generate_lighting_data: bool,
    pub split_sub_emitter_ribbons: bool,
    pub attach_ribbons_to_transform: bool,
    pub color_over_lifetime: MinMaxGradient,
    pub width_over_trail: MinMaxCurve,
    pub color_over_trail: MinMaxGradient,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForceParams {
    pub axes: [MinMaxCurve; 3],
    pub in_world_space: bool,
    pub randomize_per_frame: bool,
}

fn bits(v: Option<&Value>, ctx: &str) -> Result<u32, EffectsError> {
    let n = v.and_then(Value::as_f64)
        .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= u32::MAX as f64)
        .ok_or_else(|| EffectsError(format!("{ctx}: 32-bit mask expected")))?;
    Ok(n as i64 as u32)
}

pub(super) fn sub_emitters(v: &Value, ctx: &str) -> Result<Vec<SubEmitterParams>, EffectsError> {
    let entries = v.as_array().ok_or_else(|| EffectsError(format!("{ctx}.subEmitters: array expected")))?;
    entries.iter().enumerate().map(|(index, entry)| {
        let ctx = format!("{ctx}.subEmitters[{index}]");
        let emitter = match entry.get("emitter") {
            Some(Value::Null) => None,
            Some(Value::Str(path)) if !path.is_empty() => Some(path.clone()),
            _ => return Err(EffectsError(format!("{ctx}.emitter: explicit null or node path expected"))),
        };
        let trigger = match entry.get("type").and_then(Value::as_str) {
            Some("birth") => SubEmitterTrigger::Birth,
            Some("collision") => SubEmitterTrigger::Collision,
            Some("death") => SubEmitterTrigger::Death,
            Some("trigger") => SubEmitterTrigger::Trigger,
            Some("manual") => SubEmitterTrigger::Manual,
            _ => return Err(EffectsError(format!("{ctx}.type: unknown sub-emitter trigger"))),
        };
        let probability = f32_of(entry.get("emitProbability"), &format!("{ctx}.emitProbability"))?;
        if !(0.0..=1.0).contains(&probability) {
            return Err(EffectsError(format!("{ctx}.emitProbability: outside [0,1]")));
        }
        Ok(SubEmitterParams { emitter, trigger, properties: bits(entry.get("properties"), &format!("{ctx}.properties"))?, probability })
    }).collect()
}

impl CollisionParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.collision");
        let unknown = |field| EffectsError(format!("{ctx}.{field}: unknown collision control"));
        let number = |key| f32_of(v.get(key), &format!("{ctx}.{key}"));
        let boolean = |key| bool_of(v.get(key), &format!("{ctx}.{key}"));
        let curve = |key| min_max_curve(v.get(key), &format!("{ctx}.{key}"));
        Ok(Self {
            kind: match v.get("type").and_then(Value::as_str) {
                Some("planes") => CollisionType::Planes, Some("world") => CollisionType::World,
                _ => return Err(unknown("type")),
            },
            mode: match v.get("mode").and_then(Value::as_str) {
                Some("3d") => CollisionMode::ThreeDimensional, Some("2d") => CollisionMode::TwoDimensional,
                _ => return Err(unknown("mode")),
            },
            quality: match v.get("quality").and_then(Value::as_str) {
                Some("high") => CollisionQuality::High, Some("medium") => CollisionQuality::Medium,
                Some("low") => CollisionQuality::Low, _ => return Err(unknown("quality")),
            },
            dampen: curve("dampen")?, bounce: curve("bounce")?, lifetime_loss: curve("lifetimeLoss")?,
            min_kill_speed: number("minKillSpeed")?, max_kill_speed: number("maxKillSpeed")?,
            radius_scale: number("radiusScale")?, voxel_size: number("voxelSize")?,
            collides_with: bits(v.get("collidesWith"), &format!("{ctx}.collidesWith"))?,
            dynamic: boolean("collidesWithDynamic")?, interior: boolean("interiorCollisions")?,
            max_shapes: u32_of(v.get("maxCollisionShapes"), &format!("{ctx}.maxCollisionShapes"))?,
            messages: boolean("collisionMessages")?, collider_force: number("colliderForce")?,
            multiply_force_by_size: boolean("multiplyColliderForceByParticleSize")?,
            multiply_force_by_speed: boolean("multiplyColliderForceByParticleSpeed")?,
            multiply_force_by_angle: boolean("multiplyColliderForceByCollisionAngle")?,
            plane_slots: u32_of(v.get("planeSlots"), &format!("{ctx}.planeSlots"))?,
            planes: v.get("planes").and_then(Value::as_array)
                .ok_or_else(|| EffectsError(format!("{ctx}.planes: array expected")))?
                .iter().map(|v| str_of(Some(v), &format!("{ctx}.planes[]"))).collect::<Result<_, _>>()?,
        })
    }
}

impl TrailParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.trails");
        let boolean = |key| bool_of(v.get(key), &format!("{ctx}.{key}"));
        let number = |key| f32_of(v.get(key), &format!("{ctx}.{key}"));
        let curve = |key| min_max_curve(v.get(key), &format!("{ctx}.{key}"));
        let color = |key| min_max_gradient(v.get(key), &format!("{ctx}.{key}"));
        Ok(Self {
            mode: match v.get("mode").and_then(Value::as_str) {
                Some("perParticle") => TrailMode::PerParticle, Some("ribbon") => TrailMode::Ribbon,
                _ => return Err(EffectsError(format!("{ctx}.mode: unknown trail mode"))),
            },
            texture_mode: match v.get("textureMode").and_then(Value::as_str) {
                Some("stretch") => TrailTextureMode::Stretch, Some("tile") => TrailTextureMode::Tile,
                Some("distributePerSegment") => TrailTextureMode::DistributePerSegment,
                Some("repeatPerSegment") => TrailTextureMode::RepeatPerSegment,
                _ => return Err(EffectsError(format!("{ctx}.textureMode: unknown trail texture mode"))),
            },
            ratio: number("ratio")?, lifetime: curve("lifetime")?, min_vertex_distance: number("minVertexDistance")?,
            texture_scale: vec2_of(v.get("textureScale"), &format!("{ctx}.textureScale"))?,
            ribbon_count: u32_of(v.get("ribbonCount"), &format!("{ctx}.ribbonCount"))?,
            shadow_bias: number("shadowBias")?, world_space: boolean("worldSpace")?,
            die_with_particles: boolean("dieWithParticles")?, size_affects_width: boolean("sizeAffectsWidth")?,
            size_affects_lifetime: boolean("sizeAffectsLifetime")?, inherit_particle_color: boolean("inheritParticleColor")?,
            generate_lighting_data: boolean("generateLightingData")?, split_sub_emitter_ribbons: boolean("splitSubEmitterRibbons")?,
            attach_ribbons_to_transform: boolean("attachRibbonsToTransform")?,
            color_over_lifetime: color("colorOverLifetime")?, width_over_trail: curve("widthOverTrail")?,
            color_over_trail: color("colorOverTrail")?,
        })
    }
}

impl ForceParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.forceOverLifetime");
        Ok(Self {
            axes: [min_max_curve(v.get("x"), &format!("{ctx}.x"))?,
                min_max_curve(v.get("y"), &format!("{ctx}.y"))?,
                min_max_curve(v.get("z"), &format!("{ctx}.z"))?],
            in_world_space: bool_of(v.get("inWorldSpace"), &format!("{ctx}.inWorldSpace"))?,
            randomize_per_frame: bool_of(v.get("randomizePerFrame"), &format!("{ctx}.randomizePerFrame"))?,
        })
    }
}
