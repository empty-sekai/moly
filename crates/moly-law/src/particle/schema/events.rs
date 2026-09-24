//! Serialized event, collision and trail controls. Decoding preserves dormant
//! arms; the simulation adapter is responsible for their actual execution.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubEmitterTrigger {
    Birth,
    Collision,
    Death,
    Trigger,
    Manual,
}

/// Original serialized PPtr, independently of whether its node path resolved.
/// Missing/null contracts are unknown provenance, not authored null pointers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SubEmitterSourcePointer {
    #[default]
    Missing,
    Null,
    Pointer {
        file_id: i32,
        path_id: String,
    },
}

impl SubEmitterSourcePointer {
    /// Only a complete source PPtr (0, "0") proves an authored empty edge.
    /// This is evidence classification, not permission to run a SubModule.
    pub fn is_authored_null(&self) -> bool {
        matches!(self, Self::Pointer { file_id: 0, path_id } if path_id == "0")
    }

    pub(super) fn from_value(value: Option<&Value>, ctx: &str) -> Result<Self, EffectsError> {
        let Some(value) = value else {
            return Ok(Self::Missing);
        };
        if value == &Value::Null {
            return Ok(Self::Null);
        }
        let obj = value
            .as_object()
            .ok_or_else(|| EffectsError(format!("{ctx}: object or null expected")))?;
        let file_id = obj_get(obj, "fileId")
            .and_then(Value::as_f64)
            .filter(|n| {
                n.is_finite() && n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64
            })
            .ok_or_else(|| EffectsError(format!("{ctx}.fileId: signed 32-bit integer expected")))?
            as i32;
        let path_id = obj_get(obj, "pathId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                EffectsError(format!(
                    "{ctx}.pathId: signed 64-bit decimal string expected"
                ))
            })?;
        let parsed = path_id.parse::<i64>().map_err(|_| {
            EffectsError(format!(
                "{ctx}.pathId: signed 64-bit decimal string expected"
            ))
        })?;
        // The producer stringifies the source integer. Reject numeric JSON and
        // noncanonical encodings instead of accepting a rounded/ambiguous ID.
        if parsed.to_string() != path_id {
            return Err(EffectsError(format!(
                "{ctx}.pathId: canonical signed 64-bit decimal string expected"
            )));
        }
        Ok(Self::Pointer {
            file_id,
            path_id: path_id.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubEmitterParams {
    pub emitter: Option<String>,
    pub source_pointer: SubEmitterSourcePointer,
    pub trigger: SubEmitterTrigger,
    pub properties: u32,
    pub probability: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionType {
    Planes,
    World,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionMode {
    ThreeDimensional,
    TwoDimensional,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionQuality {
    High,
    Medium,
    Low,
}

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
pub enum TrailMode {
    PerParticle,
    Ribbon,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailTextureMode {
    Stretch,
    Tile,
    DistributePerSegment,
    RepeatPerSegment,
}

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

/// Emitter velocity inheritance.  Unity stores this as one scalar curve and
/// a mode (initial-at-birth or current-each-update); keeping the mode in the
/// typed schema prevents a consumer from silently treating an enabled module
/// as absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InheritVelocityMode {
    Initial,
    Current,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InheritVelocityParams {
    pub mode: InheritVelocityMode,
    pub curve: MinMaxCurve,
}

fn bits(v: Option<&Value>, ctx: &str) -> Result<u32, EffectsError> {
    let n = v
        .and_then(Value::as_f64)
        .filter(|n| {
            n.is_finite() && n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= u32::MAX as f64
        })
        .ok_or_else(|| EffectsError(format!("{ctx}: 32-bit mask expected")))?;
    Ok(n as i64 as u32)
}

pub(super) fn sub_emitters(v: &Value, ctx: &str) -> Result<Vec<SubEmitterParams>, EffectsError> {
    let entries = v
        .as_array()
        .ok_or_else(|| EffectsError(format!("{ctx}.subEmitters: array expected")))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let ctx = format!("{ctx}.subEmitters[{index}]");
            let emitter = match entry.get("emitter") {
                Some(Value::Null) => None,
                Some(Value::Str(path)) if !path.is_empty() => Some(path.clone()),
                _ => {
                    return Err(EffectsError(format!(
                        "{ctx}.emitter: explicit null or node path expected"
                    )));
                }
            };
            let trigger = match entry.get("type").and_then(Value::as_str) {
                Some("birth") => SubEmitterTrigger::Birth,
                Some("collision") => SubEmitterTrigger::Collision,
                Some("death") => SubEmitterTrigger::Death,
                Some("trigger") => SubEmitterTrigger::Trigger,
                Some("manual") => SubEmitterTrigger::Manual,
                _ => {
                    return Err(EffectsError(format!(
                        "{ctx}.type: unknown sub-emitter trigger"
                    )));
                }
            };
            let probability = f32_of(
                entry.get("emitProbability"),
                &format!("{ctx}.emitProbability"),
            )?;
            if !(0.0..=1.0).contains(&probability) {
                return Err(EffectsError(format!(
                    "{ctx}.emitProbability: outside [0,1]"
                )));
            }
            let source_pointer = SubEmitterSourcePointer::from_value(
                entry.get("sourcePointer"),
                &format!("{ctx}.sourcePointer"),
            )?;
            Ok(SubEmitterParams {
                emitter,
                source_pointer,
                trigger,
                properties: bits(entry.get("properties"), &format!("{ctx}.properties"))?,
                probability,
            })
        })
        .collect()
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
                Some("planes") => CollisionType::Planes,
                Some("world") => CollisionType::World,
                _ => return Err(unknown("type")),
            },
            mode: match v.get("mode").and_then(Value::as_str) {
                Some("3d") => CollisionMode::ThreeDimensional,
                Some("2d") => CollisionMode::TwoDimensional,
                _ => return Err(unknown("mode")),
            },
            quality: match v.get("quality").and_then(Value::as_str) {
                Some("high") => CollisionQuality::High,
                Some("medium") => CollisionQuality::Medium,
                Some("low") => CollisionQuality::Low,
                _ => return Err(unknown("quality")),
            },
            dampen: curve("dampen")?,
            bounce: curve("bounce")?,
            lifetime_loss: curve("lifetimeLoss")?,
            min_kill_speed: number("minKillSpeed")?,
            max_kill_speed: number("maxKillSpeed")?,
            radius_scale: number("radiusScale")?,
            voxel_size: number("voxelSize")?,
            collides_with: bits(v.get("collidesWith"), &format!("{ctx}.collidesWith"))?,
            dynamic: boolean("collidesWithDynamic")?,
            interior: boolean("interiorCollisions")?,
            max_shapes: u32_of(
                v.get("maxCollisionShapes"),
                &format!("{ctx}.maxCollisionShapes"),
            )?,
            messages: boolean("collisionMessages")?,
            collider_force: number("colliderForce")?,
            multiply_force_by_size: boolean("multiplyColliderForceByParticleSize")?,
            multiply_force_by_speed: boolean("multiplyColliderForceByParticleSpeed")?,
            multiply_force_by_angle: boolean("multiplyColliderForceByCollisionAngle")?,
            plane_slots: u32_of(v.get("planeSlots"), &format!("{ctx}.planeSlots"))?,
            planes: v
                .get("planes")
                .and_then(Value::as_array)
                .ok_or_else(|| EffectsError(format!("{ctx}.planes: array expected")))?
                .iter()
                .map(|v| str_of(Some(v), &format!("{ctx}.planes[]")))
                .collect::<Result<_, _>>()?,
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
                Some("perParticle") => TrailMode::PerParticle,
                Some("ribbon") => TrailMode::Ribbon,
                _ => return Err(EffectsError(format!("{ctx}.mode: unknown trail mode"))),
            },
            texture_mode: match v.get("textureMode").and_then(Value::as_str) {
                Some("stretch") => TrailTextureMode::Stretch,
                Some("tile") => TrailTextureMode::Tile,
                Some("distributePerSegment") => TrailTextureMode::DistributePerSegment,
                Some("repeatPerSegment") => TrailTextureMode::RepeatPerSegment,
                _ => {
                    return Err(EffectsError(format!(
                        "{ctx}.textureMode: unknown trail texture mode"
                    )));
                }
            },
            ratio: number("ratio")?,
            lifetime: curve("lifetime")?,
            min_vertex_distance: number("minVertexDistance")?,
            texture_scale: vec2_of(v.get("textureScale"), &format!("{ctx}.textureScale"))?,
            ribbon_count: u32_of(v.get("ribbonCount"), &format!("{ctx}.ribbonCount"))?,
            shadow_bias: number("shadowBias")?,
            world_space: boolean("worldSpace")?,
            die_with_particles: boolean("dieWithParticles")?,
            size_affects_width: boolean("sizeAffectsWidth")?,
            size_affects_lifetime: boolean("sizeAffectsLifetime")?,
            inherit_particle_color: boolean("inheritParticleColor")?,
            generate_lighting_data: boolean("generateLightingData")?,
            split_sub_emitter_ribbons: boolean("splitSubEmitterRibbons")?,
            attach_ribbons_to_transform: boolean("attachRibbonsToTransform")?,
            color_over_lifetime: color("colorOverLifetime")?,
            width_over_trail: curve("widthOverTrail")?,
            color_over_trail: color("colorOverTrail")?,
        })
    }
}

impl ForceParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.forceOverLifetime");
        Ok(Self {
            axes: [
                min_max_curve(v.get("x"), &format!("{ctx}.x"))?,
                min_max_curve(v.get("y"), &format!("{ctx}.y"))?,
                min_max_curve(v.get("z"), &format!("{ctx}.z"))?,
            ],
            in_world_space: bool_of(v.get("inWorldSpace"), &format!("{ctx}.inWorldSpace"))?,
            randomize_per_frame: bool_of(
                v.get("randomizePerFrame"),
                &format!("{ctx}.randomizePerFrame"),
            )?,
        })
    }
}

impl InheritVelocityParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.inheritVelocity");
        let mode = match v.get("mode").and_then(Value::as_str) {
            Some("initial") => InheritVelocityMode::Initial,
            Some("current") => InheritVelocityMode::Current,
            _ => {
                return Err(EffectsError(format!(
                    "{ctx}.mode: unknown inherit velocity mode"
                )));
            }
        };
        let curve = min_max_curve(v.get("curve"), &format!("{ctx}.curve"))?;
        Ok(Self { mode, curve })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(pointer_field: &str) -> Result<SubEmitterParams, EffectsError> {
        let source = format!(
            r#"[{{"emitter":null,"type":"birth","properties":0,"emitProbability":1{pointer_field}}}]"#
        );
        sub_emitters(&json::parse(source.as_bytes()).unwrap(), "effect/snow")
            .map(|mut entries| entries.remove(0))
    }

    #[test]
    fn sub_emitter_pointer_distinguishes_missing_null_and_authored_zero() {
        let missing = edge("").unwrap();
        let null = edge(r#", "sourcePointer":null"#).unwrap();
        let empty = edge(r#", "sourcePointer":{"fileId":0,"pathId":"0"}"#).unwrap();
        assert_eq!(missing.source_pointer, SubEmitterSourcePointer::Missing);
        assert_eq!(null.source_pointer, SubEmitterSourcePointer::Null);
        assert!(!missing.source_pointer.is_authored_null());
        assert!(!null.source_pointer.is_authored_null());
        assert!(empty.source_pointer.is_authored_null());
        assert!(missing.emitter.is_none() && null.emitter.is_none() && empty.emitter.is_none());
        let external_zero = edge(r#", "sourcePointer":{"fileId":1,"pathId":"0"}"#).unwrap();
        assert!(
            !external_zero.source_pointer.is_authored_null(),
            "do not discard file identity"
        );
    }

    #[test]
    fn unresolved_nonnull_sub_emitter_keeps_exact_signed_64bit_identity() {
        for (file_id, path_id) in [
            (0, "9007199254740993"),
            (2, "9223372036854775807"),
            (-1, "-9223372036854775808"),
        ] {
            let pointer =
                format!(r#", "sourcePointer":{{"fileId":{file_id},"pathId":"{path_id}"}}"#);
            let params = edge(&pointer).unwrap();
            assert_eq!(
                params.source_pointer,
                SubEmitterSourcePointer::Pointer {
                    file_id,
                    path_id: path_id.into()
                }
            );
            assert!(
                params.emitter.is_none(),
                "a retained source pointer does not resolve a node"
            );
            assert!(!params.source_pointer.is_authored_null());
        }
    }

    #[test]
    fn resolved_node_path_and_source_pointer_are_independent_records() {
        let value = json::parse(br#"[{"emitter":"root/child","sourcePointer":{"fileId":3,"pathId":"-9007199254740993"},"type":"death","properties":4,"emitProbability":0.25}]"#).unwrap();
        let params = sub_emitters(&value, "effect/test").unwrap().remove(0);
        assert_eq!(params.emitter.as_deref(), Some("root/child"));
        assert_eq!(
            params.source_pointer,
            SubEmitterSourcePointer::Pointer {
                file_id: 3,
                path_id: "-9007199254740993".into()
            }
        );
        assert_eq!(params.trigger, SubEmitterTrigger::Death);
        assert_eq!((params.properties, params.probability), (4, 0.25));
    }

    #[test]
    fn malformed_sub_emitter_pointer_never_turns_into_an_empty_edge() {
        for (pointer, location) in [
            (r#"false"#, "sourcePointer"),
            (
                r#"{"fileId":0,"pathId":9007199254740993}"#,
                "sourcePointer.pathId",
            ),
            (
                r#"{"fileId":0,"pathId":"9223372036854775808"}"#,
                "sourcePointer.pathId",
            ),
            (
                r#"{"fileId":0,"pathId":"-9223372036854775809"}"#,
                "sourcePointer.pathId",
            ),
            (r#"{"fileId":0,"pathId":"00"}"#, "sourcePointer.pathId"),
            (r#"{"fileId":0,"pathId":null}"#, "sourcePointer.pathId"),
            (r#"{"fileId":0}"#, "sourcePointer.pathId"),
            (
                r#"{"fileId":2147483648,"pathId":"0"}"#,
                "sourcePointer.fileId",
            ),
            (
                r#"{"fileId":-2147483649,"pathId":"0"}"#,
                "sourcePointer.fileId",
            ),
            (r#"{"fileId":0.5,"pathId":"0"}"#, "sourcePointer.fileId"),
            (r#"{"fileId":null,"pathId":"0"}"#, "sourcePointer.fileId"),
            (r#"{"pathId":"0"}"#, "sourcePointer.fileId"),
        ] {
            let error = edge(&format!(r#", "sourcePointer":{pointer}"#)).unwrap_err();
            assert!(
                error
                    .0
                    .contains(&format!("effect/snow.subEmitters[0].{location}")),
                "{error}"
            );
        }
    }

    #[test]
    fn inherit_velocity_preserves_mode_and_rejects_unknown_inputs() {
        for (name, expected) in [
            ("initial", InheritVelocityMode::Initial),
            ("current", InheritVelocityMode::Current),
        ] {
            let text = format!(r#"{{"mode":"{name}","curve":{{"mode":"constant","value":1}}}}"#);
            let value = json::parse(text.as_bytes()).unwrap();
            let params = InheritVelocityParams::from_value(&value, "test").unwrap();
            assert_eq!(params.mode, expected);
            assert_eq!(params.curve, MinMaxCurve::Constant(1.0));
        }
        for text in [
            r#"{"mode":"initial"}"#,
            r#"{"mode":"initial","curve":null}"#,
            r#"{"mode":"unknown","curve":{"mode":"constant","value":1}}"#,
        ] {
            let value = json::parse(text.as_bytes()).unwrap();
            assert!(InheritVelocityParams::from_value(&value, "test").is_err());
        }
    }
}
