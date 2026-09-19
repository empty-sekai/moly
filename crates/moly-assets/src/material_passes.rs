//! Source shader pass availability, separate from material shading and visibility.
//!
//! A base/colour pass does not imply a shadow pass. Preserve the exported pass
//! tags on each primitive so every renderer can ask the same source metadata.

use bevy::prelude::*;

#[derive(Component, Reflect, Clone, Debug, Default, PartialEq, Eq)]
#[reflect(Component)]
pub struct SourceMaterialPasses {
    light_modes: Vec<String>,
    color_state: Option<SourceRenderState>,
    effect_state: Option<SourceRenderState>,
}

/// Resolved source pass state. Unity numeric enums are preserved at the asset
/// boundary; the renderer translates them to its backend's conventions.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceRenderState {
    pub cull: u8,
    pub depth_test: u8,
    pub depth_write: bool,
    pub color_mask: u8,
    pub src_color: u8,
    pub dst_color: u8,
    pub src_alpha: u8,
    pub dst_alpha: u8,
    pub color_op: u8,
    pub alpha_op: u8,
}

impl SourceRenderState {
    fn parse(value: &serde_json::Value, extras: &serde_json::Value) -> Option<Self> {
        // A named state binds a material property. Its compiled `val` is not
        // the property's default; use the exported shader default if absent.
        let scalar = |value: &serde_json::Value| {
            let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let number = if name.is_empty() || name == "<noninit>" {
                value.get("val")
            } else {
                extras
                    .get("floats")
                    .and_then(|v| v.get(name))
                    .or_else(|| value.get("default"))
            }?
            .as_f64()?;
            (number.is_finite() && number >= 0.0 && number <= 255.0 && number.fract() == 0.0)
                .then_some(number as u8)
        };
        let blend = value.get("blend")?;
        let cull = scalar(value.get("culling")?)?;
        let depth_test = scalar(value.get("zTest")?)?;
        let depth_write = scalar(value.get("zWrite")?)?;
        let state = Self {
            cull,
            depth_test,
            depth_write: depth_write == 1,
            color_mask: scalar(blend.get("colMask")?)?,
            src_color: scalar(blend.get("srcBlend")?)?,
            dst_color: scalar(blend.get("destBlend")?)?,
            src_alpha: scalar(blend.get("srcBlendAlpha")?)?,
            dst_alpha: scalar(blend.get("destBlendAlpha")?)?,
            color_op: scalar(blend.get("blendOp")?)?,
            alpha_op: scalar(blend.get("blendOpAlpha")?)?,
        };
        (cull <= 2
            && depth_test <= 8
            && depth_write <= 1
            && state.color_mask <= 15
            && [
                state.src_color,
                state.dst_color,
                state.src_alpha,
                state.dst_alpha,
            ]
            .iter()
            .all(|v| *v <= 10)
            && state.color_op <= 4
            && state.alpha_op <= 4)
            .then_some(state)
    }
}

impl SourceMaterialPasses {
    /// Missing metadata is distinct from a known list without a shadow pass.
    /// Untagged passes are valid, but cannot satisfy a ShadowCaster request.
    pub fn from_extras(extras: &serde_json::Value) -> Option<Self> {
        // glTF fixture extras store this directly; site sidecars retain it
        // inside the resolved shader reference. Both originate from pass tags.
        let detailed = extras.get("shaderPasses")
            .or_else(|| extras.get("shader").and_then(|s| s.get("shaderPasses")));
        let Some(detailed) = detailed else {
            return Self::from_light_modes(extras.get("lightModes")?);
        };
        let passes = detailed.as_array()?;
        let mut light_modes = Vec::new();
        let mut color_state = None;
        let mut effect_state = None;
        let mut effect_unresolved = false;
        for pass in passes {
            let pass = pass.as_object()?;
            if pass
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Base"))
            {
                if let Some(state) = pass.get("renderState") {
                    color_state = SourceRenderState::parse(state, extras);
                }
            }
            if pass.get("lightMode").and_then(|v| v.as_str()) == Some("MysekaiEffect") {
                let state = pass.get("renderState").and_then(|s| SourceRenderState::parse(s, extras));
                match state {
                    Some(state) if effect_state.is_none_or(|previous| previous == state) => effect_state = Some(state),
                    _ => effect_unresolved = true,
                }
            }
            match pass.get("lightMode") {
                Some(serde_json::Value::String(mode)) => light_modes.push(mode.clone()),
                None | Some(serde_json::Value::Null) => {}
                _ => return None,
            }
        }
        Some(Self {
            light_modes,
            color_state,
            effect_state: if effect_unresolved { None } else { effect_state },
        })
    }

    /// Compact particle exports retain each declared LightMode, including
    /// null for an untagged pass. Missing/malformed metadata is NOT an empty list.
    pub fn from_light_modes(value: &serde_json::Value) -> Option<Self> {
        let mut light_modes = Vec::new();
        for tag in value.as_array()? {
            match tag {
                serde_json::Value::String(tag) => light_modes.push(tag.clone()),
                serde_json::Value::Null => {},
                _ => return None,
            }
        }
        Some(Self { light_modes, color_state: None, effect_state: None })
    }

    /// Pass tags are identities. Shader-family names are never a substitute.
    pub fn has_light_mode(&self, mode: &str) -> bool {
        self.light_modes.iter().any(|tag| tag == mode)
    }

    pub fn has_shadow_caster(&self) -> bool {
        self.light_modes
            .iter()
            .any(|mode| mode.eq_ignore_ascii_case("ShadowCaster"))
    }

    /// Only a source-resolved effect state is usable. Differing SubShader
    /// states require actual SubShader selection; do not choose one arbitrarily.
    pub fn effect_state(&self) -> Option<SourceRenderState> {
        self.effect_state
    }

    pub fn color_state(&self) -> Option<SourceRenderState> {
        self.color_state
    }
}


/// Material-level eligibility for the source transparent MysekaiEffect draw.
/// Renderer visibility and camera gates remain separate render-extraction checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectPassEligibility {
    Eligible,
    NotDeclared,
    QueueExcluded,
    Unresolved,
}

impl EffectPassEligibility {
    pub fn from_material(value: &serde_json::Value) -> Self {
        let Some(passes) = SourceMaterialPasses::from_extras(value) else {
            return Self::Unresolved;
        };
        if !passes.has_light_mode("MysekaiEffect") {
            return Self::NotDeclared;
        }
        let Some(queue) = value.get("renderQueue").and_then(|v| v.as_i64()) else {
            return Self::Unresolved;
        };
        if (2501..=5000).contains(&queue) { Self::Eligible } else { Self::QueueExcluded }
    }
}

#[cfg(test)]
mod weather_pass_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn family_name_does_not_authorize_an_effect_draw() {
        let plain = json!({"shader":"Particles/Standard Unlit", "lightModes":[null,null], "renderQueue":3000});
        let misleading = json!({"shader":"Mysekai/Effect/UberUnlit", "lightModes":["UniversalForward"], "renderQueue":3000});
        assert_eq!(EffectPassEligibility::from_material(&plain), EffectPassEligibility::NotDeclared);
        assert_eq!(EffectPassEligibility::from_material(&misleading), EffectPassEligibility::NotDeclared);
    }

    #[test]
    fn explicit_tag_and_transparent_queue_are_both_required() {
        for (queue, expected) in [(2500, EffectPassEligibility::QueueExcluded), (2501, EffectPassEligibility::Eligible), (5000, EffectPassEligibility::Eligible), (5001, EffectPassEligibility::QueueExcluded)] {
            let value = json!({"lightModes":["UniversalForward","MysekaiEffect"], "renderQueue":queue});
            assert_eq!(EffectPassEligibility::from_material(&value), expected);
        }
        assert_eq!(EffectPassEligibility::from_material(&json!({"lightModes":["MysekaiEffect"]})), EffectPassEligibility::Unresolved);
    }

    #[test]
    fn unknown_is_not_known_absence_and_does_not_fall_back_to_a_family() {
        for value in [json!({"shader":"Mysekai/Effect/UberUnlit"}), json!({"lightModes":null}), json!({"lightModes":[7]}), json!({"shaderPasses":null,"lightModes":["MysekaiEffect"],"renderQueue":3000})] {
            assert_eq!(EffectPassEligibility::from_material(&value), EffectPassEligibility::Unresolved);
        }
        assert_eq!(EffectPassEligibility::from_material(&json!({"lightModes":[]})), EffectPassEligibility::NotDeclared);
    }

    #[test]
    fn detailed_and_compact_exports_authorize_the_same_pass() {
        let value = json!({"shaderPasses":[{"lightMode":"UniversalForward"},{"lightMode":"MysekaiEffect"}],"renderQueue":3000});
        assert_eq!(EffectPassEligibility::from_material(&value), EffectPassEligibility::Eligible);
    }
}
