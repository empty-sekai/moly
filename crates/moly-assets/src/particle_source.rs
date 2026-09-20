//! Source-owned module inventory and mesh-emission contracts.
//! Unknown or missing controls are not evidence that a module is disabled.
use crate::particle_geometry::ParticleMeshReference;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticleSourceModules {
    pub version: u32,
    pub enabled: Vec<String>,
    pub unsupported: Vec<serde_json::Value>,
}
impl ParticleSourceModules {
    pub fn from_system(system: &serde_json::Value) -> Result<Self, String> {
        let source = system
            .get("sourceModules")
            .ok_or("missing source module inventory; re-extract")?;
        let value: Self = serde_json::from_value(source.clone()).map_err(|e| e.to_string())?;
        if value.version != 1 || value.enabled.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("invalid source module inventory version/order".into());
        }
        if !value.unsupported.is_empty() {
            return Err(format!(
                "unconsumed source module evidence: {}",
                serde_json::Value::Array(value.unsupported.clone())
            ));
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MeshControls {
    source_version: u32,
    mode: String,
    spread: f32,
    speed: serde_json::Value,
    use_colors: bool,
    use_material_index: bool,
    texture: MeshTexture,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MeshTexture {
    file_id: i32,
    path_id: String,
}

/// The admitted source subset is area-weighted triangle emission, without mesh
/// colour, texture masking or material filtering. Other branches remain errors.
#[derive(Clone, Debug)]
pub struct ParticleMeshEmission {
    pub mesh: ParticleMeshReference,
}
impl ParticleMeshEmission {
    pub fn from_shape(shape: &serde_json::Value) -> Result<Self, String> {
        let controls: MeshControls = serde_json::from_value(
            shape
                .get("meshEmission")
                .ok_or("missing source mesh-emission controls; re-extract")?
                .clone(),
        )
        .map_err(|e| e.to_string())?;
        if controls.source_version != 1 || controls.mode != "Random" || controls.spread != 0.0 {
            return Err("unconsumed source mesh spawn mode/spread".into());
        }
        // Speed belongs to non-random mesh modes. Preserve it without applying
        // an unrelated circular arc's speed to this random triangle sampler.
        if !controls.speed.is_object() {
            return Err("missing authored mesh spawn speed".into());
        }
        if controls.use_colors
            || controls.use_material_index
            || controls.texture.file_id != 0
            || controls.texture.path_id != "0"
        {
            return Err("unconsumed source mesh colours, material filter or shape texture".into());
        }
        if shape
            .get("meshPlacement")
            .and_then(serde_json::Value::as_u64)
            != Some(2)
            || shape
                .get("meshNormalOffset")
                .and_then(serde_json::Value::as_f64)
                != Some(0.0)
            || !shape
                .get("meshMaterialIndex")
                .is_some_and(serde_json::Value::is_null)
        {
            return Err("unconsumed source mesh placement or normal offset".into());
        }
        let references = shape
            .get("meshes")
            .and_then(serde_json::Value::as_array)
            .filter(|references| references.len() == 1)
            .ok_or("mesh emission needs one resolved source surface")?;
        let mesh: ParticleMeshReference =
            serde_json::from_value(references[0].clone()).map_err(|e| e.to_string())?;
        mesh.validate()?;
        if mesh.mesh_slot != 0 {
            return Err("unconsumed emission mesh slot".into());
        }
        Ok(Self { mesh })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_module_inventory_never_turns_missing_or_unknown_into_disabled() {
        assert!(ParticleSourceModules::from_system(&json!({})).is_err());
        assert!(ParticleSourceModules::from_system(
            &json!({"sourceModules":{"version":1,"enabled":[],"unsupported":[]}})
        )
        .is_ok());
        for inventory in [
            json!({"version":2,"enabled":[],"unsupported":[]}),
            json!({"version":1,"enabled":["ShapeModule","ShapeModule"],"unsupported":[]}),
            json!({"version":1,"enabled":["InheritVelocityModule"],"unsupported":[{"module":"InheritVelocityModule","reason":"unmodelled"}]}),
            json!({"version":1,"enabled":[]}),
        ] {
            assert!(
                ParticleSourceModules::from_system(&json!({"sourceModules":inventory})).is_err()
            );
        }
    }
    #[test]
    fn incomplete_mesh_control_ownership_cannot_admit_a_surface() {
        for shape in [
            json!({}),
            json!({"meshEmission":{}}),
            json!({"meshEmission":{"sourceVersion":1,"mode":"Random","spread":0,"speed":{},"useColors":false,"useMaterialIndex":false,"texture":{"fileId":0,"pathId":"0"}}}),
        ] {
            assert!(ParticleMeshEmission::from_shape(&shape).is_err());
        }
    }
}
