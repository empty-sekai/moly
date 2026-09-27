//! Source fixed-function state, resolved from the selected pass and material.
//!
//! State properties are not shader uniforms: a compiled zero alongside a named
//! property is not a material default. Likewise RGB and alpha are independent
//! blend equations. Keep Unity's enum values until the backend boundary.
use super::{require, Result, SourcePass, SourceShaderError};
use crate::material_passes::SourceRenderState;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourcePassState {
    pub raster: SourceRenderState,
    pub alpha_to_coverage: bool,
    pub depth_clip: bool,
}

/// Resolve a scalar in the original SerializedShaderState. A caller resolves
/// named properties from the material or the shader's declared property table;
/// it must report missing owners rather than return a neutral value.
fn scalar(value: &Value, resolve: &mut impl FnMut(&str) -> Result<f64>) -> Result<f64> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| SourceShaderError("source render state lacks a property identity".into()))?;
    let number = if name.is_empty() || name == "<noninit>" {
        value.get("val").and_then(Value::as_f64).ok_or_else(|| {
            SourceShaderError("source render state lacks its literal value".into())
        })?
    } else {
        resolve(name)?
    };
    require(number.is_finite(), "nonfinite source render state")?;
    Ok(number)
}
fn integer(
    value: &Value,
    maximum: u8,
    resolve: &mut impl FnMut(&str) -> Result<f64>,
) -> Result<u8> {
    let number = scalar(value, resolve)?;
    require(
        number >= 0.0 && number <= f64::from(maximum) && number.fract() == 0.0,
        format!("source render state {number} is outside its supported enum"),
    )?;
    Ok(number as u8)
}

impl SourcePassState {
    /// This consumer has one colour attachment and no stencil attachment. It
    /// accepts only source state that can be represented on those attachments.
    /// A non-neutral stencil, polygon offset, fixed-function fog/lighting or
    /// independently blended MRT remains an explicit rejection, not lost JSON.
    pub fn resolve(
        pass: &SourcePass,
        mut property: impl FnMut(&str) -> Result<f64>,
    ) -> Result<Self> {
        let source = &pass.serialized_state;
        require(
            source.is_object(),
            "selected source pass has no serialized state",
        )?;
        let get = |name: &str| {
            source
                .get(name)
                .ok_or_else(|| SourceShaderError(format!("selected source pass lacks {name}")))
        };
        let blend = get("rtBlend0")?;
        let blend_scalar = |name: &str| {
            blend
                .get(name)
                .ok_or_else(|| SourceShaderError(format!("source blend state lacks {name}")))
        };
        let raster = SourceRenderState {
            cull: integer(get("culling")?, 2, &mut property)?,
            depth_test: integer(get("zTest")?, 8, &mut property)?,
            depth_write: integer(get("zWrite")?, 1, &mut property)? != 0,
            color_mask: integer(blend_scalar("colMask")?, 15, &mut property)?,
            src_color: integer(blend_scalar("srcBlend")?, 10, &mut property)?,
            dst_color: integer(blend_scalar("destBlend")?, 10, &mut property)?,
            src_alpha: integer(blend_scalar("srcBlendAlpha")?, 10, &mut property)?,
            dst_alpha: integer(blend_scalar("destBlendAlpha")?, 10, &mut property)?,
            color_op: integer(blend_scalar("blendOp")?, 4, &mut property)?,
            alpha_op: integer(blend_scalar("blendOpAlpha")?, 4, &mut property)?,
        };
        require(
            raster.depth_test != 0,
            "disabled source depth testing needs a separately qualified depth-write policy",
        )?;
        require(
            source.get("rtSeparateBlend").and_then(Value::as_bool) == Some(false),
            "separate source MRT blend state needs a multi-target owner",
        )?;
        require(
            source.get("lighting").and_then(Value::as_bool) == Some(false),
            "source fixed-function lighting is not consumed",
        )?;
        require(
            source.get("fogMode").and_then(Value::as_i64) == Some(-1),
            "source fixed-function fog is not consumed",
        )?;
        for name in ["offsetFactor", "offsetUnits", "conservative"] {
            require(
                scalar(get(name)?, &mut property)? == 0.0,
                format!("source {name} requires a qualified raster adapter"),
            )?;
        }
        for name in ["stencilOp", "stencilOpFront", "stencilOpBack"] {
            let op = get(name)?;
            for (field, expected) in [("comp", 8), ("pass", 0), ("fail", 0), ("zFail", 0)] {
                let value = op
                    .get(field)
                    .ok_or_else(|| SourceShaderError(format!("source {name} lacks {field}")))?;
                require(
                    integer(value, 8, &mut property)? == expected,
                    format!("source {name}.{field} requires a stencil attachment owner"),
                )?;
            }
        }
        // These values cannot change a known Always/Keep stencil operation, but
        // still validate that they are explicit, well-formed source integers.
        for name in ["stencilReadMask", "stencilWriteMask", "stencilRef"] {
            integer(get(name)?, 255, &mut property)?;
        }
        Ok(Self {
            raster,
            alpha_to_coverage: integer(get("alphaToMask")?, 1, &mut property)? != 0,
            depth_clip: integer(get("zClip")?, 1, &mut property)? != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn literal(value: f64) -> Value {
        json!({"name":"<noninit>","val":value})
    }
    fn source() -> SourcePass {
        let op = json!({"comp":literal(8.0),"pass":literal(0.0),"fail":literal(0.0),"zFail":literal(0.0)});
        serde_json::from_value(json!({
            "subShaderIndex":2,"passIndex":3,"name":"display","lightMode":"owner",
            "renderState":{},"programBlocks":{},"variants":[],"serializedState":{
                "culling":literal(0.0),"zTest":literal(4.0),"zWrite":literal(0.0),
                "rtBlend0":{"colMask":literal(14.0),"srcBlend":{"name":"source","val":0.0},
                    "destBlend":literal(10.0),"srcBlendAlpha":literal(1.0),"destBlendAlpha":literal(1.0),
                    "blendOp":literal(0.0),"blendOpAlpha":literal(2.0)},
                "rtSeparateBlend":false,"lighting":false,"fogMode":-1,
                "offsetFactor":literal(0.0),"offsetUnits":literal(0.0),"conservative":literal(0.0),
                "alphaToMask":literal(0.0),"zClip":literal(1.0),
                "stencilOp":op,"stencilOpFront":op,"stencilOpBack":op,
                "stencilReadMask":literal(255.0),"stencilWriteMask":literal(255.0),"stencilRef":literal(0.0)
            }
        })).unwrap()
    }
    #[test]
    fn selected_pass_resolves_properties_and_keeps_independent_alpha_equation() {
        let result = SourcePassState::resolve(&source(), |name| {
            assert_eq!(name, "source");
            Ok(5.0)
        })
        .unwrap();
        assert_eq!((result.raster.src_color, result.raster.dst_color), (5, 10));
        assert_eq!(
            (
                result.raster.src_alpha,
                result.raster.dst_alpha,
                result.raster.alpha_op
            ),
            (1, 1, 2)
        );
        assert_eq!(result.raster.color_mask, 14);
        assert!(!result.raster.depth_write && result.depth_clip && !result.alpha_to_coverage);
    }
    #[test]
    fn missing_named_state_does_not_fall_back_to_the_compiled_zero() {
        assert!(
            SourcePassState::resolve(&source(), |_| Err(SourceShaderError(
                "unowned state".into()
            )))
            .unwrap_err()
            .0
            .contains("unowned")
        );
        for value in [f64::NAN, 5.5, -1.0, 11.0] {
            assert!(SourcePassState::resolve(&source(), |_| Ok(value)).is_err());
        }
    }
    #[test]
    fn unconsumed_source_state_is_rejected_before_pipeline_creation() {
        for key in ["offsetFactor", "offsetUnits", "conservative"] {
            let mut pass = source();
            pass.serialized_state[key] = literal(1.0);
            assert!(SourcePassState::resolve(&pass, |_| Ok(5.0))
                .unwrap_err()
                .0
                .contains(key));
        }
        for op in ["stencilOp", "stencilOpFront", "stencilOpBack"] {
            let mut pass = source();
            pass.serialized_state[op]["pass"] = literal(2.0);
            assert!(SourcePassState::resolve(&pass, |_| Ok(5.0))
                .unwrap_err()
                .0
                .contains("stencil"));
        }
        let mut pass = source();
        pass.serialized_state["rtSeparateBlend"] = json!(true);
        assert!(SourcePassState::resolve(&pass, |_| Ok(5.0)).is_err());
        let mut pass = source();
        pass.serialized_state
            .as_object_mut()
            .unwrap()
            .remove("zClip");
        assert!(SourcePassState::resolve(&pass, |_| Ok(5.0)).is_err());
    }
}
