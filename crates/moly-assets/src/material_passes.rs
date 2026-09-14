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
                extras.get("floats").and_then(|v| v.get(name)).or_else(|| value.get("default"))
            }?.as_f64()?;
            (number.is_finite() && number >= 0.0 && number <= 255.0 && number.fract() == 0.0)
                .then_some(number as u8)
        };
        let blend = value.get("blend")?;
        let cull = scalar(value.get("culling")?)?;
        let depth_test = scalar(value.get("zTest")?)?;
        let depth_write = scalar(value.get("zWrite")?)?;
        let state = Self {
            cull, depth_test, depth_write: depth_write == 1,
            color_mask: scalar(blend.get("colMask")?)?,
            src_color: scalar(blend.get("srcBlend")?)?,
            dst_color: scalar(blend.get("destBlend")?)?,
            src_alpha: scalar(blend.get("srcBlendAlpha")?)?,
            dst_alpha: scalar(blend.get("destBlendAlpha")?)?,
            color_op: scalar(blend.get("blendOp")?)?,
            alpha_op: scalar(blend.get("blendOpAlpha")?)?,
        };
        (cull <= 2 && depth_test <= 8 && depth_write <= 1 && state.color_mask <= 15
            && [state.src_color, state.dst_color, state.src_alpha, state.dst_alpha].iter().all(|v| *v <= 10)
            && state.color_op <= 4 && state.alpha_op <= 4).then_some(state)
    }
}

impl SourceMaterialPasses {
    /// Missing metadata is distinct from a known list without a shadow pass.
    /// Untagged passes are valid, but cannot satisfy a ShadowCaster request.
    pub fn from_extras(extras: &serde_json::Value) -> Option<Self> {
        // glTF fixture extras store this directly; site sidecars retain it
        // inside the resolved shader reference. Both originate from pass tags.
        let passes = extras.get("shaderPasses")
            .or_else(|| extras.get("shader")?.get("shaderPasses"))?.as_array()?;
        let mut light_modes = Vec::new();
        let mut color_state = None;
        for pass in passes {
            let pass = pass.as_object()?;
            if pass.get("name").and_then(|v| v.as_str()).is_some_and(|name| name.eq_ignore_ascii_case("Base")) {
                if let Some(state) = pass.get("renderState") {
                    color_state = SourceRenderState::parse(state, extras);
                }
            }
            match pass.get("lightMode") {
                Some(serde_json::Value::String(mode)) => light_modes.push(mode.clone()),
                None | Some(serde_json::Value::Null) => {}
                _ => return None,
            }
        }
        Some(Self { light_modes, color_state })
    }

    pub fn has_shadow_caster(&self) -> bool {
        self.light_modes.iter().any(|mode| mode.eq_ignore_ascii_case("ShadowCaster"))
    }

    pub fn color_state(&self) -> Option<SourceRenderState> { self.color_state }
}
