//! Sampler ownership for compiled GLES texture parameters.
//!
//! Current native SetTextures installs the texture-owned sampler (the explicit
//! no-inline sentinel); independently authored sampler commands are a separate
//! path. Therefore an inline-looking keyword cannot override an ordinary
//! compiled texture binding. Keep that decision at the compiled-record boundary.
use super::{
    abi::{TextureBinding, TextureDimension},
    require,
    texture::{SourceSampler, SourceTexture, SourceTextureKind},
    Result, SourceShaderError,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTextureSlot {
    pub name: String,
    pub texture_index: u32,
    pub sampler_index: u32,
    pub dimension: TextureDimension,
}
impl CompiledTextureSlot {
    /// This law is qualified only for the recovered GLES program route. Other
    /// APIs and separate/inline sampler records require their own owner proof.
    pub fn resolve(
        gpu_program_type: i32,
        parameters: Option<&Value>,
        declaration: &TextureBinding,
    ) -> Result<Self> {
        require(
            gpu_program_type == 4,
            "sampler ownership is not qualified for this source program API",
        )?;
        let bindings = parameters
            .and_then(|v| v.get("bindings"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SourceShaderError("compiled source program lacks sampler binding evidence".into())
            })?;
        for binding in bindings {
            require(
                matches!(
                    (binding["kind"].as_i64(), binding["kindName"].as_str()),
                    (Some(0), Some("texture")) | (Some(1), Some("constantBuffer"))
                ),
                "compiled source has an unqualified separate sampler/resource binding",
            )?;
        }
        let matching = bindings
            .iter()
            .filter(|v| v["kind"] == 0 && v["name"] == declaration.name)
            .collect::<Vec<_>>();
        require(
            matching.len() == 1,
            format!(
                "texture {} has {} compiled binding owners",
                declaration.name,
                matching.len()
            ),
        )?;
        let record = matching[0];
        let raw = record["raw"]
            .as_array()
            .filter(|v| v.len() == 3)
            .ok_or_else(|| {
                SourceShaderError("compiled texture has an invalid binary binding record".into())
            })?;
        let index = |v: &Value| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    SourceShaderError("compiled texture index is not an unsigned slot".into())
                })
        };
        require(
            raw[0] == 0,
            "compiled texture binding tag disagrees with its record",
        )?;
        let texture_index = index(&raw[1])?;
        let sampler_index = index(&raw[2])?;
        require(
            texture_index == sampler_index,
            "source separate texture/sampler slots require a qualified sampler owner",
        )?;
        let dimension = match record["dim"].as_i64() {
            Some(2) => TextureDimension::D2,
            Some(5) => TextureDimension::D2Array,
            _ => {
                return Err(SourceShaderError(
                    "source texture dimensionality has no qualified sampler mapping".into(),
                ))
            }
        };
        require(
            record["multisampled"] == false
                && record["packed"].as_u64() == Some(record["dim"].as_u64().unwrap() * 2),
            "compiled source texture has unqualified flags or multisampling",
        )?;
        require(
            declaration.dimension == dimension && declaration.array.is_none(),
            "compiled texture record disagrees with the generated resource ABI",
        )?;
        Ok(Self {
            name: declaration.name.clone(),
            texture_index,
            sampler_index,
            dimension,
        })
    }
    pub fn source_sampler<'a>(&self, texture: &'a SourceTexture) -> Result<&'a SourceSampler> {
        require(
            matches!(
                (&texture.kind, self.dimension),
                (SourceTextureKind::Texture2D, TextureDimension::D2)
                    | (SourceTextureKind::Texture2DArray, TextureDimension::D2Array)
            ),
            "source texture payload dimensionality disagrees with its compiled sampler owner",
        )?;
        texture.source_texture_settings.as_ref().ok_or_else(|| {
            SourceShaderError(format!(
                "source texture {} has no sampler-state evidence",
                self.name
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::abi::ShaderStage;
    use super::*;
    use serde_json::json;
    fn declaration() -> TextureBinding {
        TextureBinding {
            name: "_SourceMap".into(),
            dimension: TextureDimension::D2,
            texture_binding: 1,
            sampler_binding: 2,
            stages: vec![ShaderStage::Fragment],
            array: None,
        }
    }
    fn source() -> Value {
        json!({"bindings":[{"kind":0,"kindName":"texture","name":"_SourceMap","raw":[0,3,3],"dim":2,"packed":4,"multisampled":false}]})
    }
    #[test]
    fn compiled_slot_not_a_keyword_selects_the_sampler_owner() {
        let slot = CompiledTextureSlot::resolve(4, Some(&source()), &declaration()).unwrap();
        assert_eq!((slot.texture_index, slot.sampler_index), (3, 3));
        let mut source = source();
        source["keywords"] = json!(["_BASE_SAMPLER_STATE_LINEAR_MIRROR"]);
        assert_eq!(
            slot,
            CompiledTextureSlot::resolve(4, Some(&source), &declaration()).unwrap()
        );
    }
    #[test]
    fn missing_or_conflicting_sampler_evidence_fails_closed() {
        assert!(CompiledTextureSlot::resolve(4, None, &declaration()).is_err());
        assert!(CompiledTextureSlot::resolve(15, Some(&source()), &declaration()).is_err());
        for (field, value) in [
            ("dim", json!(5)),
            ("packed", json!(5)),
            ("multisampled", json!(true)),
            ("kind", json!(2)),
        ] {
            let mut source = source();
            source["bindings"][0][field] = value;
            assert!(CompiledTextureSlot::resolve(4, Some(&source), &declaration()).is_err());
        }
        let mut separate = source();
        separate["bindings"][0]["raw"] = json!([0, 3, 2]);
        assert!(CompiledTextureSlot::resolve(4, Some(&separate), &declaration()).is_err());
        let mut source = source();
        let duplicate = source["bindings"][0].clone();
        source["bindings"].as_array_mut().unwrap().push(duplicate);
        assert!(CompiledTextureSlot::resolve(4, Some(&source), &declaration()).is_err());
    }
}
