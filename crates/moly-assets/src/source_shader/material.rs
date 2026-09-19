//! Bind a source material to a source program without a shader-family switch.
//!
//! The raw property sheet is authoritative; preview colours, flattened texture
//! atlases and UI metadata cannot supply executable shader inputs. Runtime/global
//! writers remain explicit unresolved inputs until their owner supplies a value.
use super::{
    abi::{ScalarType, UniformField},
    require, ContentReference, ObjectIdentity, Result, SourceShaderCatalogue, SourceShaderError,
    UniformValue,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SourcePointer {
    #[serde(rename = "m_FileID")]
    pub file: i32,
    #[serde(rename = "m_PathID")]
    pub path: i64,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct TextureVector {
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialTextureReference {
    #[serde(flatten)]
    pub content: ContentReference,
    pub source: ObjectIdentity,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialTextureBinding {
    pub pointer: SourcePointer,
    pub scale: TextureVector,
    pub offset: TextureVector,
    pub status: String,
    pub error: Option<String>,
    pub texture: Option<MaterialTextureReference>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct MaterialShaderReference {
    #[serde(flatten)]
    pub content: ContentReference,
    pub source: ObjectIdentity,
    pub variants: ContentReference,
}

#[derive(Debug, Clone)]
pub struct MaterialSnapshot {
    pub source: ObjectIdentity,
    pub shader: MaterialShaderReference,
    pub textures: BTreeMap<String, MaterialTextureBinding>,
    pub keywords: Vec<String>,
    pub disabled_passes: BTreeSet<String>,
    pub custom_render_queue: i32,
    /// Retain instancing, GI, tag overrides and inactive properties. Successful
    /// parsing is not permission for a renderer to ignore these source states.
    pub source_material: Value,
    floats: BTreeMap<String, SourceNumber>,
    ints: BTreeMap<String, i64>,
    vectors: BTreeMap<String, [SourceNumber; 4]>,
}

fn pairs(value: Option<&Value>, field: &str) -> Result<BTreeMap<String, Value>> {
    let entries = value
        .and_then(Value::as_array)
        .ok_or_else(|| SourceShaderError(format!("source material lacks {field}")))?;
    let mut result = BTreeMap::new();
    for entry in entries {
        let entry = entry
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or_else(|| SourceShaderError(format!("invalid source property pair in {field}")))?;
        let name = entry[0]
            .as_str()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| SourceShaderError(format!("invalid source property name in {field}")))?;
        require(
            result.insert(name.into(), entry[1].clone()).is_none(),
            format!("duplicate {field} property {name}"),
        )?;
    }
    Ok(result)
}
// Unity can serialize nonfinite values in properties of disabled keyword
// branches. Preserve that state; reject it only if an executable input reads it.
#[derive(Debug, Clone, Copy)]
enum SourceNumber {
    Finite(f64),
    PositiveInfinity,
    NegativeInfinity,
    Nan,
}
impl SourceNumber {
    fn parse(value: &Value, name: &str) -> Result<Self> {
        match value.as_str() {
            Some("Infinity") => Ok(Self::PositiveInfinity),
            Some("-Infinity") => Ok(Self::NegativeInfinity),
            Some("NaN") => Ok(Self::Nan),
            Some(_) => Err(SourceShaderError(format!("invalid source number {name}"))),
            None => Ok(Self::Finite(finite(value, name)?)),
        }
    }
    fn finite(self, name: &str) -> Result<f64> {
        match self {
            Self::Finite(value) => Ok(value),
            _ => Err(SourceShaderError(format!(
                "active source input {name} is nonfinite"
            ))),
        }
    }
}
fn finite(value: &Value, name: &str) -> Result<f64> {
    let value = value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| SourceShaderError(format!("non-numeric source property {name}")))?;
    require(
        (value as f32).is_finite(),
        format!("source property {name} exceeds f32"),
    )?;
    Ok(value)
}
fn string_set(value: &Value, name: &str) -> Result<BTreeSet<String>> {
    let values = value
        .as_array()
        .ok_or_else(|| SourceShaderError(format!("source {name} is not an array")))?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| SourceShaderError(format!("invalid source {name}")))?;
        require(
            result.insert(value.into()),
            format!("duplicate source {name}: {value}"),
        )?;
    }
    Ok(result)
}

impl MaterialSnapshot {
    pub fn parse(document: &Value) -> Result<Self> {
        let raw = &document["sourceMaterial"];
        require(
            raw["schemaVersion"] == 1,
            "unsupported source material schema",
        )?;
        let source: ObjectIdentity = serde_json::from_value(raw["source"].clone())?;
        source.validate()?;
        let shader: MaterialShaderReference =
            serde_json::from_value(document["shaderProgram"].clone())?;
        shader.source.validate()?;
        shader.content.validate()?;
        shader.variants.validate()?;
        let shader_pointer: SourcePointer = serde_json::from_value(raw["shaderPointer"].clone())?;
        require(
            shader_pointer.path.to_string() == shader.source.path_id,
            "material shader PPtr and shader object identity disagree",
        )?;
        let saved = &raw["savedProperties"];
        let floats = pairs(saved.get("m_Floats"), "m_Floats")?
            .into_iter()
            .map(|(name, v)| Ok((name.clone(), SourceNumber::parse(&v, &name)?)))
            .collect::<Result<_>>()?;
        let ints = pairs(saved.get("m_Ints"), "m_Ints")?
            .into_iter()
            .map(|(name, v)| {
                let number = v
                    .as_i64()
                    .ok_or_else(|| SourceShaderError(format!("invalid integer property {name}")))?;
                Ok((name, number))
            })
            .collect::<Result<_>>()?;
        let vectors = pairs(saved.get("m_Colors"), "m_Colors")?
            .into_iter()
            .map(|(name, v)| {
                let mut vector = [SourceNumber::Finite(0.0); 4];
                for (index, key) in ["r", "g", "b", "a"].iter().enumerate() {
                    vector[index] = SourceNumber::parse(&v[*key], &name)?;
                }
                Ok((name, vector))
            })
            .collect::<Result<_>>()?;
        let texture_sheet = pairs(saved.get("m_TexEnvs"), "m_TexEnvs")?;
        let textures: BTreeMap<String, MaterialTextureBinding> =
            serde_json::from_value(document["textureSources"].clone())?;
        require(
            textures.len() == texture_sheet.len(),
            "source texture bindings do not cover the property sheet",
        )?;
        for (name, binding) in &textures {
            let original = texture_sheet.get(name).ok_or_else(|| {
                SourceShaderError(format!("texture {name} has no source property"))
            })?;
            let pointer: SourcePointer = serde_json::from_value(original["m_Texture"].clone())?;
            let scale: TextureVector = serde_json::from_value(original["m_Scale"].clone())?;
            let offset: TextureVector = serde_json::from_value(original["m_Offset"].clone())?;
            require(
                pointer == binding.pointer && scale == binding.scale && offset == binding.offset,
                format!(
                    "source texture {name} pointer/transform disagrees with the property sheet"
                ),
            )?;
            require(
                [scale.x, scale.y, offset.x, offset.y]
                    .iter()
                    .all(|v| v.is_finite() && (*v as f32).is_finite()),
                "nonfinite source texture transform",
            )?;
            if let Some(texture) = &binding.texture {
                texture.content.validate()?;
                texture.source.validate()?;
                require(
                    pointer.path != 0 && pointer.path.to_string() == texture.source.path_id,
                    format!("source texture {name} PPtr and content owner disagree"),
                )?;
                require(
                    texture.status == binding.status,
                    "source texture status disagrees with its payload",
                )?;
            } else {
                require(
                    (pointer.path == 0 && binding.status == "unassigned")
                        || (binding.status == "unavailable"
                            && binding.error.as_ref().is_some_and(|e| !e.is_empty())),
                    format!("source texture {name} has no payload or explicit source reason"),
                )?;
            }
        }
        let keywords = if !raw["validKeywords"].is_null() {
            string_set(&raw["validKeywords"], "valid keywords")?
        } else {
            let legacy = raw["legacyKeywords"]
                .as_str()
                .ok_or_else(|| SourceShaderError("source material has no keyword state".into()))?;
            let words: Vec<Value> = legacy
                .split_whitespace()
                .map(|w| Value::String(w.into()))
                .collect();
            string_set(&Value::Array(words), "legacy keywords")?
        };
        let disabled_passes = string_set(&raw["disabledShaderPasses"], "disabled shader passes")?;
        let custom_render_queue = raw["customRenderQueue"]
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v == -1 || (0..=5000).contains(v))
            .ok_or_else(|| SourceShaderError("invalid source material render queue".into()))?;
        Ok(Self {
            source,
            shader,
            textures,
            keywords: keywords.into_iter().collect(),
            disabled_passes,
            custom_render_queue,
            source_material: raw.clone(),
            floats,
            ints,
            vectors,
        })
    }
    pub fn matches(&self, catalogue: &SourceShaderCatalogue) -> Result<()> {
        require(
            self.shader.source == catalogue.source
                && self.shader.content == catalogue.source_document,
            "material and shader catalogue have different source owners",
        )
    }
    fn property<'a>(
        &self,
        catalogue: &'a SourceShaderCatalogue,
        name: &str,
    ) -> Result<Option<&'a Value>> {
        self.matches(catalogue)?;
        let matches: Vec<_> = catalogue
            .properties
            .iter()
            .filter(|v| v["m_Name"] == name)
            .collect();
        require(
            matches.len() <= 1,
            format!("duplicate source shader property {name}"),
        )?;
        Ok(matches.first().copied())
    }
    /// Source values are returned in their recorded colour space. A renderer
    /// must qualify any colour-space conversion; the binder does not invent it.
    pub fn scalar_property(&self, catalogue: &SourceShaderCatalogue, name: &str) -> Result<f64> {
        let declaration = self
            .property(catalogue, name)?
            .ok_or_else(|| SourceShaderError(format!("unowned scalar property {name}")))?;
        let kind = declaration["m_Type"].as_i64();
        require(
            matches!(kind, Some(2 | 3 | 5)),
            format!("{name} is not a scalar source property"),
        )?;
        if kind == Some(5) {
            if let Some(value) = self.ints.get(name) {
                return Ok(*value as f64);
            }
        } else if let Some(value) = self.floats.get(name) {
            return value.finite(name);
        }
        finite(&declaration["m_DefValue[0]"], name)
    }
    /// Resolve only material-owned inputs. None is an external writer request,
    /// not a zero or a shader-default substitution for an unknown global.
    pub fn uniform(
        &self,
        catalogue: &SourceShaderCatalogue,
        field: &UniformField,
    ) -> Result<Option<UniformValue>> {
        let name = &field.name;
        if let Some(declaration) = self.property(catalogue, name)? {
            let kind = declaration["m_Type"].as_i64();
            require(
                field.array.is_none(),
                format!("source material array {name} needs a runtime writer"),
            )?;
            return match kind {
                Some(2 | 3) if field.ty == ScalarType::Float => {
                    Ok(Some(UniformValue::Float(vec![
                        self.scalar_property(catalogue, name)? as f32,
                    ])))
                }
                Some(2 | 3) if field.ty == ScalarType::Int => {
                    // The current native GLES ApplyFloat int-target branch
                    // executes FCVTZS, not a bit reinterpretation. Convert the
                    // recorded value to f32 first, then truncate toward zero
                    // with signed saturation. Nonfinite input was rejected at
                    // the source-property boundary. Vulkan uses the same law.
                    Ok(Some(UniformValue::Signed(vec![
                        self.scalar_property(catalogue, name)? as f32 as i32,
                    ])))
                }
                Some(5) if field.ty == ScalarType::Int => {
                    let value = self.scalar_property(catalogue, name)?;
                    require(
                        value >= i32::MIN as f64
                            && value <= i32::MAX as f64
                            && value.fract() == 0.0,
                        "source integer uniform exceeds i32",
                    )?;
                    Ok(Some(UniformValue::Signed(vec![value as i32])))
                }
                Some(0 | 1)
                    if matches!(
                        field.ty,
                        ScalarType::Vec2 | ScalarType::Vec3 | ScalarType::Vec4
                    ) =>
                {
                    let vector = if let Some(source) = self.vectors.get(name) {
                        let mut vector = [0.0; 4];
                        // Only declared active components are uploaded. Inactive
                        // trailing components remain preserved in the snapshot.
                        for index in 0..field.ty.components() {
                            vector[index] = source[index].finite(name)?;
                        }
                        vector
                    } else {
                        let mut vector = [0.0; 4];
                        for (i, item) in vector.iter_mut().enumerate() {
                            *item = finite(&declaration[format!("m_DefValue[{i}]")], name)?;
                        }
                        vector
                    };
                    // The compiled parameter's vector width determines how many
                    // recorded components are uploaded, not a guessed vec4 ABI.
                    Ok(Some(UniformValue::Float(
                        vector[..field.ty.components()]
                            .iter()
                            .map(|v| *v as f32)
                            .collect(),
                    )))
                }
                _ => Err(SourceShaderError(format!(
                    "source material property {name} and compiled uniform type disagree"
                ))),
            };
        }
        if let Some(texture_name) = name.strip_suffix("_ST") {
            if self
                .property(catalogue, texture_name)?
                .is_some_and(|p| p["m_Type"] == 4)
            {
                require(
                    field.ty == ScalarType::Vec4 && field.array.is_none(),
                    format!("texture transform {name} does not have vec4 ABI"),
                )?;
                let texture = self.textures.get(texture_name).ok_or_else(|| {
                    SourceShaderError(format!("source texture transform {name} is absent"))
                })?;
                return Ok(Some(UniformValue::Float(vec![
                    texture.scale.x as f32,
                    texture.scale.y as f32,
                    texture.offset.x as f32,
                    texture.offset.y as f32,
                ])));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
