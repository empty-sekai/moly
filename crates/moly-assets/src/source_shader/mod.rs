//! Source-addressed shader programs. Display names never select executable code.
//!
//! Source metadata is retained independently of generated content: two program
//! addresses with identical code are still two candidates, not an exact match.
//! Loaders verify bytes before exposing shader or texture assets to a renderer.

pub mod abi;
pub mod loader;
pub mod material;
pub mod sampler;
pub mod state;
pub mod texture;

use bevy::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub use abi::{ProgramAbi, UniformValue};
pub use loader::{register, SourceProgramAsset, SourceTextureAsset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceShaderError(pub String);
impl std::fmt::Display for SourceShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SourceShaderError {}
impl From<serde_json::Error> for SourceShaderError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}
pub type Result<T> = std::result::Result<T, SourceShaderError>;
pub(crate) fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(SourceShaderError(message.into()))
    }
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let sorted: std::collections::BTreeMap<_, _> = fields.iter().collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(k, v)| (k.clone(), canonical_value(v)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentReference {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}
impl ContentReference {
    pub fn validate(&self) -> Result<()> {
        require(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "invalid SHA-256 content identity",
        )?;
        // References are extraction-root-relative, never URLs, asset labels,
        // filesystem paths or a request to climb outside the source namespace.
        require(
            !self.file.is_empty()
                && !self.file.contains(['\\', ':', '#', '?', '%'])
                && self
                    .file
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != ".."),
            format!("invalid source content path {}", self.file),
        )?;
        require(self.bytes > 0, "empty source content reference")
    }
    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        self.validate()?;
        require(
            bytes.len() as u64 == self.bytes,
            format!("{}: source byte count mismatch", self.file),
        )?;
        require(
            sha256(bytes) == self.sha256,
            format!("{}: source SHA-256 mismatch", self.file),
        )
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObjectIdentity {
    /// The complete supplied-package receipt, not just its human-readable name.
    pub package: Value,
    pub serialized_file: String,
    /// A string prevents JavaScript from rounding an IL2CPP/Unity 64-bit PPtr.
    pub path_id: String,
}
impl ObjectIdentity {
    pub fn validate(&self) -> Result<()> {
        require(
            self.package.is_object()
                && !self.serialized_file.is_empty()
                && self.path_id.parse::<i64>().is_ok_and(|p| p != 0),
            "invalid source object identity",
        )
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgramAddress {
    pub subshader: u32,
    pub pass: u32,
    pub platform: u32,
    pub record: u32,
    pub gpu_program_type: u32,
    pub program_block: String,
    pub parameter_record: Option<u32>,
    #[serde(default)]
    pub platform_group_index: Option<u32>,
    #[serde(default)]
    pub subprogram_index: Option<u32>,
    #[serde(flatten)]
    pub source_metadata: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceVariant {
    pub reference: ProgramAddress,
    pub status: String,
    pub identity: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub local_keywords: Vec<String>,
    pub conversion: Option<ContentReference>,
    pub error: Option<String>,
}
impl SourceVariant {
    pub fn effective_keywords(&self) -> BTreeSet<&str> {
        self.keywords
            .iter()
            .chain(&self.local_keywords)
            .map(String::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePass {
    pub sub_shader_index: u32,
    pub pass_index: u32,
    pub name: Option<String>,
    pub light_mode: Option<String>,
    pub pass_type: Option<u32>,
    pub render_state: Value,
    /// Uninterpreted state remains present for a renderer's admission check.
    /// Successful deserialization does not authorize ignoring a stencil or MRT.
    pub serialized_state: Value,
    pub program_blocks: Value,
    pub tags: Option<Value>,
    pub sub_shader_tags: Option<Value>,
    pub variants: Vec<SourceVariant>,
}

#[derive(Asset, TypePath, Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceShaderCatalogue {
    pub schema_version: u32,
    pub source: ObjectIdentity,
    pub name: Option<String>,
    pub source_document: ContentReference,
    pub properties: Vec<Value>,
    pub passes: Vec<SourcePass>,
    #[serde(default)]
    pub source_errors: Vec<String>,
}
impl SourceShaderCatalogue {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let result: Self = serde_json::from_slice(bytes)?;
        require(
            result.schema_version == 1,
            "unsupported source shader catalogue schema",
        )?;
        result.source.validate()?;
        result.source_document.validate()?;
        require(
            !result.passes.is_empty(),
            "source shader declares no readable passes",
        )?;
        let mut ids = BTreeSet::new();
        for pass in &result.passes {
            require(
                ids.insert((pass.sub_shader_index, pass.pass_index)),
                "duplicate source pass identity",
            )?;
            for variant in &pass.variants {
                require(
                    (variant.reference.subshader, variant.reference.pass)
                        == (pass.sub_shader_index, pass.pass_index),
                    "variant address belongs to another source pass",
                )?;
                if variant.status == "converted" {
                    require(
                        variant.identity.as_ref().is_some_and(|s| s.len() == 64),
                        "converted variant lacks program identity",
                    )?;
                    variant
                        .conversion
                        .as_ref()
                        .ok_or_else(|| {
                            SourceShaderError("converted variant lacks conversion receipt".into())
                        })?
                        .validate()?;
                } else {
                    require(
                        variant.status == "unavailable"
                            && variant.error.as_ref().is_some_and(|s| !s.is_empty()),
                        "unavailable variant lacks a source reason",
                    )?;
                }
            }
        }
        Ok(result)
    }

    /// Select one exact address and effective keyword set. The renderer supplies
    /// its source-derived SubShader/pass; this function makes no owner decision.
    pub fn select(
        &self,
        subshader: u32,
        pass_index: u32,
        platform: u32,
        program_type: u32,
        keywords: &[String],
    ) -> Result<(&SourcePass, &SourceVariant)> {
        let pass = self
            .passes
            .iter()
            .find(|p| (p.sub_shader_index, p.pass_index) == (subshader, pass_index))
            .ok_or_else(|| SourceShaderError("requested source pass is absent".into()))?;
        let requested: BTreeSet<_> = keywords.iter().map(String::as_str).collect();
        require(
            requested.len() == keywords.len(),
            "duplicate effective source keyword",
        )?;
        let candidates: Vec<_> = pass
            .variants
            .iter()
            .filter(|v| {
                v.reference.platform == platform
                    && v.reference.gpu_program_type == program_type
                    && v.effective_keywords() == requested
            })
            .collect();
        require(
            candidates.len() == 1,
            format!(
                "{} source variants match the exact pass/keyword state",
                candidates.len()
            ),
        )?;
        let selected = candidates[0];
        require(
            selected.status == "converted",
            selected
                .error
                .clone()
                .unwrap_or_else(|| "source variant is not converted".into()),
        )?;
        Ok((pass, selected))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShaderStageReceipt {
    pub stage: abi::ShaderStage,
    pub entry_point: String,
    pub shader: ContentReference,
    pub wgsl_sha256: String,
    pub round_trip_validated: bool,
    pub bindings: Vec<Value>,
    pub renderer: Option<RendererStageReceipt>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererStageReceipt {
    pub schema_version: u32,
    pub source_clip_space: String,
    pub target_clip_space: String,
    pub operation: String,
    pub adapted_returns: u32,
    pub entry_point: String,
    pub validated: bool,
    pub shader: ContentReference,
    pub wgsl_sha256: String,
    pub bindings: Vec<Value>,
}
impl RendererStageReceipt {
    pub fn validate(&self, abi: &ProgramAbi) -> Result<()> {
        require(
            self.schema_version == 1
                && self.validated
                && self.adapted_returns > 0
                && self.source_clip_space == "gl-minus-one-to-one"
                && self.target_clip_space == "reverse-zero-to-one"
                && self.operation == "z=(w-z)*0.5"
                && !self.entry_point.is_empty(),
            "unsupported renderer clip adapter",
        )?;
        self.shader.validate()?;
        require(
            self.wgsl_sha256 == self.shader.sha256,
            "renderer adapter hash disagrees with shader content",
        )?;
        abi.validate_reflection(&self.bindings)
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct ProgramStages {
    pub vertex: ShaderStageReceipt,
    pub fragment: ShaderStageReceipt,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramSource {
    pub reference: ProgramAddress,
    pub program: SourceProgramIdentity,
    pub effective_keywords: Vec<String>,
    pub pass: Value,
    pub parameters: Option<Value>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SourceProgramIdentity {
    pub identity: String,
    pub source: Value,
    pub code: ContentReference,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReceipt {
    pub schema_version: u32,
    pub source: ProgramSource,
    pub abi: ProgramAbi,
    pub stages: ProgramStages,
    pub conversion: Value,
}
impl ProgramReceipt {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let result: Self = serde_json::from_slice(bytes)?;
        require(
            result.schema_version == 1,
            "unsupported compiled source program schema",
        )?;
        result.abi.validate()?;
        result.source.program.code.validate()?;
        let address = &result.source.program.source;
        require(
            sha256(&serde_json::to_vec(&canonical_value(address))?)
                == result.source.program.identity,
            "source program identity does not match its binary address",
        )?;
        let owner: ObjectIdentity = serde_json::from_value(address["shader"].clone())?;
        owner.validate()?;
        require(
            address["codeSha256"] == result.source.program.code.sha256
                && address["platform"].as_u64() == Some(result.source.reference.platform as u64)
                && address["record"].as_u64() == Some(result.source.reference.record as u64),
            "program address, code identity and selected reference disagree",
        )?;
        for (expected, stage) in [
            (abi::ShaderStage::Vertex, &result.stages.vertex),
            (abi::ShaderStage::Fragment, &result.stages.fragment),
        ] {
            require(
                stage.stage == expected
                    && !stage.entry_point.is_empty()
                    && stage.round_trip_validated,
                "invalid compiled shader stage receipt",
            )?;
            stage.shader.validate()?;
            require(
                stage.wgsl_sha256 == stage.shader.sha256,
                "shader receipt contains conflicting content identities",
            )?;
            result.abi.validate_reflection(&stage.bindings)?;
            if let Some(adapter) = &stage.renderer {
                require(
                    expected == abi::ShaderStage::Vertex,
                    "unexpected clip adapter on a fragment program",
                )?;
                adapter.validate(&result.abi)?;
            }
        }
        Ok(result)
    }
    pub fn matches(
        &self,
        catalogue: &SourceShaderCatalogue,
        variant: &SourceVariant,
    ) -> Result<()> {
        let owner: ObjectIdentity =
            serde_json::from_value(self.source.program.source["shader"].clone())?;
        require(
            owner == catalogue.source,
            "compiled program belongs to a different source shader object",
        )?;
        require(
            self.source.reference == variant.reference
                && Some(&self.source.program.identity) == variant.identity.as_ref()
                && self
                    .source
                    .effective_keywords
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    == variant.effective_keywords(),
            "compiled shader receipt does not belong to the selected source program",
        )
    }
}

#[cfg(test)]
mod tests;
