//! Generated binding ABI, validated independently of the shader's display name.
//! Padding is deterministic; a missing input is an error, never a zero uniform.
use super::{require, Result, SourceShaderError};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ShaderStage {
    Vertex,
    Fragment,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum TextureDimension {
    #[serde(rename = "2d")]
    D2,
    #[serde(rename = "2d-array")]
    D2Array,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScalarType {
    Float,
    Vec2,
    Vec3,
    Vec4,
    Int,
    Ivec2,
    Ivec3,
    Ivec4,
    Uint,
    Uvec2,
    Uvec3,
    Uvec4,
    Bool,
}
impl ScalarType {
    pub fn components(self) -> usize {
        match self {
            Self::Vec2 | Self::Ivec2 | Self::Uvec2 => 2,
            Self::Vec3 | Self::Ivec3 | Self::Uvec3 => 3,
            Self::Vec4 | Self::Ivec4 | Self::Uvec4 => 4,
            _ => 1,
        }
    }
    fn reflected_kind(self) -> &'static str {
        match self {
            Self::Float | Self::Vec2 | Self::Vec3 | Self::Vec4 => "Float",
            // The lowering stores signed ints as uint and reinterprets the
            // bits at the source expression, preserving the original int.
            _ => "Uint",
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct UniformField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ScalarType,
    pub array: Option<u32>,
    pub offset: u32,
    pub bytes: u32,
    pub member: String,
    pub stages: Vec<ShaderStage>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureBinding {
    pub name: String,
    pub dimension: TextureDimension,
    pub texture_binding: u32,
    pub sampler_binding: u32,
    pub stages: Vec<ShaderStage>,
    pub array: Option<u32>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Interface {
    pub name: String,
    pub direction: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub location: u32,
    pub interpolation: String,
    pub adapter: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Interfaces {
    pub vertex: Vec<Interface>,
    pub fragment: Vec<Interface>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramAbi {
    pub version: u32,
    pub bind_group: u32,
    pub uniform_binding: u32,
    pub uniform_bytes: u32,
    pub uniforms: Vec<UniformField>,
    pub textures: Vec<TextureBinding>,
    pub interfaces: Interfaces,
    pub source_clip_space: String,
    pub texture_data_origin: String,
    pub array_layer_rule: String,
}

/// Inputs supplied by a material/global/vertex owner. Scalar types and arities
/// are not inferred from the source's uniform name.
#[derive(Debug, Clone, PartialEq)]
pub enum UniformValue {
    Float(Vec<f32>),
    Signed(Vec<i32>),
    Unsigned(Vec<u32>),
    Bool(bool),
}
impl ProgramAbi {
    pub fn validate(&self) -> Result<()> {
        require(
            self.version == 1 && self.bind_group == 0 && self.uniform_binding == 0,
            "unsupported source binding ABI",
        )?;
        require(
            self.source_clip_space == "gl-minus-one-to-one"
                && self.texture_data_origin == "unity-lower-left"
                && self.array_layer_rule == "clamp(floor(layer+0.5),0,layers-1)",
            "unknown source coordinate or array-layer convention",
        )?;
        let mut cursor = 0u32;
        let mut names = BTreeSet::new();
        let mut members = BTreeSet::new();
        for field in &self.uniforms {
            require(
                !field.name.is_empty()
                    && names.insert(field.name.as_str())
                    && !field.member.is_empty()
                    && members.insert(field.member.as_str()),
                "duplicate or empty uniform identity",
            )?;
            require(
                !field.stages.is_empty()
                    && field.stages.iter().collect::<BTreeSet<_>>().len() == field.stages.len(),
                "invalid uniform stage visibility",
            )?;
            let count = field.array.unwrap_or(1);
            let bytes = count
                .checked_mul(16)
                .ok_or_else(|| SourceShaderError("uniform array byte count overflow".into()))?;
            require(
                count > 0 && field.bytes == bytes && field.offset == cursor,
                format!(
                    "{}: overlapping or inconsistent uniform byte layout",
                    field.name
                ),
            )?;
            cursor = cursor
                .checked_add(bytes)
                .ok_or_else(|| SourceShaderError("uniform span overflow".into()))?;
        }
        require(
            cursor == self.uniform_bytes && cursor % 16 == 0,
            "uniform block span disagrees with its fields",
        )?;
        let mut bindings = BTreeSet::from([self.uniform_binding]);
        for texture in &self.textures {
            require(
                !texture.name.is_empty()
                    && names.insert(texture.name.as_str())
                    && texture.array.is_none(),
                "duplicate texture identity or unimplemented texture binding array",
            )?;
            require(
                bindings.insert(texture.texture_binding)
                    && bindings.insert(texture.sampler_binding),
                "shader resource bindings overlap",
            )?;
            require(
                !texture.stages.is_empty()
                    && texture.stages.iter().collect::<BTreeSet<_>>().len() == texture.stages.len(),
                "invalid texture stage visibility",
            )?;
        }
        for interfaces in [&self.interfaces.vertex, &self.interfaces.fragment] {
            let mut locations = BTreeSet::new();
            for field in interfaces {
                require(
                    matches!(field.direction.as_str(), "in" | "out")
                        && locations.insert((field.direction.as_str(), field.location)),
                    "duplicate shader interface location",
                )?;
            }
        }
        for input in self
            .interfaces
            .fragment
            .iter()
            .filter(|i| i.direction == "in")
        {
            let output = self
                .interfaces
                .vertex
                .iter()
                .find(|o| o.direction == "out" && o.location == input.location)
                .ok_or_else(|| {
                    SourceShaderError(format!(
                        "fragment varying {} has no source vertex output",
                        input.name
                    ))
                })?;
            require(
                output.ty == input.ty && output.interpolation == input.interpolation,
                "vertex/fragment varying ABI mismatch",
            )?;
        }
        Ok(())
    }

    pub fn validate_reflection(&self, bindings: &[Value]) -> Result<()> {
        let mut seen = BTreeSet::new();
        let mut uniform_found = self.uniform_bytes == 0;
        for value in bindings {
            let group = value.get("group").and_then(Value::as_u64);
            let binding = value
                .get("binding")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| SourceShaderError("reflected binding address is absent".into()))?;
            require(
                group == Some(u64::from(self.bind_group)) && seen.insert(binding),
                "reflected bind group differs from source ABI",
            )?;
            if binding == self.uniform_binding {
                require(
                    value["kind"] == "struct"
                        && value["addressSpace"] == "Uniform"
                        && value["bytes"].as_u64() == Some(self.uniform_bytes as u64),
                    "reflected uniform span differs from source ABI",
                )?;
                let fields = value["fields"]
                    .as_array()
                    .ok_or_else(|| SourceShaderError("reflected uniform fields absent".into()))?;
                require(
                    fields.len() == self.uniforms.len(),
                    "reflected uniform member count differs",
                )?;
                for (source, reflected) in self.uniforms.iter().zip(fields) {
                    require(
                        reflected["offset"].as_u64() == Some(source.offset as u64),
                        "reflected uniform member offset differs",
                    )?;
                    let ty = &reflected["type"];
                    let element = if let Some(count) = source.array {
                        require(
                            ty["kind"] == "array" && ty["stride"] == 16 && ty["count"] == count,
                            "reflected uniform array layout differs",
                        )?;
                        &ty["element"]
                    } else {
                        ty
                    };
                    require(
                        element["kind"] == "vector"
                            && element["components"] == 4
                            && element["width"] == 4
                            && element["scalar"] == source.ty.reflected_kind(),
                        "reflected uniform scalar representation differs",
                    )?;
                }
                uniform_found = true;
            } else if let Some(texture) =
                self.textures.iter().find(|t| t.texture_binding == binding)
            {
                require(
                    value["kind"] == "texture"
                        && value["dimension"] == "D2"
                        && value["array"].as_bool()
                            == Some(texture.dimension == TextureDimension::D2Array)
                        && value["class"] == "Sampled { kind: Float, multi: false }",
                    "reflected texture dimension or sample type differs",
                )?;
            } else if self.textures.iter().any(|t| t.sampler_binding == binding) {
                require(
                    value["kind"] == "sampler" && value["comparison"] == false,
                    "reflected sampler type differs",
                )?;
            } else {
                return Err(SourceShaderError(
                    "generated shader declares an unowned resource binding".into(),
                ));
            }
        }
        require(
            uniform_found,
            "generated shader lacks its source uniform block",
        )
    }

    /// Pack one complete update. Resolving all fields first prevents an error
    /// from committing a partly updated material or leaking old global values.
    pub fn pack_uniforms(
        &self,
        mut resolve: impl FnMut(&UniformField) -> Result<UniformValue>,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = vec![0u8; self.uniform_bytes as usize];
        for field in &self.uniforms {
            let count = field.array.unwrap_or(1) as usize;
            let components = field.ty.components();
            let value = resolve(field)?;
            let words: Vec<u32> = match (field.ty, value) {
                (
                    ScalarType::Float | ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4,
                    UniformValue::Float(values),
                ) => {
                    require(
                        values.iter().all(|v| v.is_finite()),
                        format!("{}: nonfinite source uniform", field.name),
                    )?;
                    values.into_iter().map(f32::to_bits).collect()
                }
                (
                    ScalarType::Int | ScalarType::Ivec2 | ScalarType::Ivec3 | ScalarType::Ivec4,
                    UniformValue::Signed(values),
                ) => values.into_iter().map(|v| v as u32).collect(),
                (
                    ScalarType::Uint | ScalarType::Uvec2 | ScalarType::Uvec3 | ScalarType::Uvec4,
                    UniformValue::Unsigned(values),
                ) => values,
                (ScalarType::Bool, UniformValue::Bool(value)) if field.array.is_none() => {
                    vec![u32::from(value)]
                }
                _ => {
                    return Err(SourceShaderError(format!(
                        "{}: source uniform type mismatch",
                        field.name
                    )))
                }
            };
            require(
                words.len() == count * components,
                format!("{}: source uniform component count mismatch", field.name),
            )?;
            for (index, word) in words.iter().enumerate() {
                let offset =
                    field.offset as usize + (index / components) * 16 + (index % components) * 4;
                bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        Ok(bytes)
    }
}
