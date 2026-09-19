//! Producer-owned particle mesh references. Geometry can be content-deduplicated
//! without losing the source object, authored bounds, slot or coordinate frame.
use serde::Deserialize;
use crate::weather_timeline::SourceObjectIdentity;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SourceMeshBounds {
    pub center: [f32; 3],
    pub extents: [f32; 3],
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParticleMeshReference {
    pub geometry_schema_version: u32,
    pub file: String,
    pub node: String,
    pub coordinates: String,
    pub sha256: String,
    pub bytes: u64,
    pub mesh_slot: u32,
    pub source: SourceObjectIdentity,
    pub bounds: SourceMeshBounds,
}
impl ParticleMeshReference {
    pub fn validate(&self) -> Result<(), String> {
        if self.geometry_schema_version != 1 || self.coordinates != "gltf-reflect-x-flip-v" {
            return Err("unsupported particle mesh coordinate/schema contract".into());
        }
        if !self.file.starts_with("models/") || !self.file.ends_with(".glb")
            || self.file.split('/').any(|part| part.is_empty() || matches!(part,"."|".."))
            || self.file.chars().any(|c| c.is_control() || "\\:%?#*<>|".contains(c))
            || self.node.is_empty() || self.node.chars().any(char::is_control) {
            return Err("invalid particle mesh artifact path or node identity".into());
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.bytes < 20 || self.mesh_slot > 3 {
            return Err("invalid particle mesh hash, size or source slot".into());
        }
        self.source.validate()?;
        if self.bounds.center.iter().chain(self.bounds.extents.iter()).any(|v| !v.is_finite())
            || self.bounds.extents.iter().any(|v| *v < 0.0) {
            return Err("particle mesh authored bounds are invalid".into());
        }
        Ok(())
    }
    pub fn size(&self) -> [f32; 3] { self.bounds.extents.map(|v| 2.0 * v) }
}
