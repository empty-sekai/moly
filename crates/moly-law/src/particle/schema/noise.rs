//! Authored Noise configuration. Decoding is preservation, not runtime admission.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseQuality {
    Low,
    Medium,
    High,
}

/// Keep dormant axes/remaps and unsupported curve modes intact. The qualified
/// NoiseLaw subset decides what it can execute after source decoding succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseParams {
    pub separate_axes: bool,
    pub strength: MinMaxCurve,
    pub strength_y: MinMaxCurve,
    pub strength_z: MinMaxCurve,
    pub frequency: f32,
    pub damping: bool,
    pub octaves: u32,
    pub octave_multiplier: f32,
    pub octave_scale: f32,
    pub quality: NoiseQuality,
    pub dimensions: u32,
    pub scroll_speed: MinMaxCurve,
    pub remap_enabled: bool,
    pub remap: MinMaxCurve,
    pub remap_y: MinMaxCurve,
    pub remap_z: MinMaxCurve,
    pub position_amount: MinMaxCurve,
    pub rotation_amount: MinMaxCurve,
    pub size_amount: MinMaxCurve,
}

impl NoiseParams {
    pub(super) fn from_value(value: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.noise");
        let obj = value
            .as_object()
            .ok_or_else(|| EffectsError(format!("{ctx}: object expected")))?;
        let curve = |key: &str| min_max_curve(obj_get(obj, key), &format!("{ctx}.{key}"));
        let number = |key: &str| f32_of(obj_get(obj, key), &format!("{ctx}.{key}"));
        let boolean = |key: &str| bool_of(obj_get(obj, key), &format!("{ctx}.{key}"));
        let unsigned = |key: &str| u32_of(obj_get(obj, key), &format!("{ctx}.{key}"));
        let quality = match obj_get(obj, "quality").and_then(Value::as_str) {
            Some("low") => NoiseQuality::Low,
            Some("medium") => NoiseQuality::Medium,
            Some("high") => NoiseQuality::High,
            _ => {
                return Err(EffectsError(format!(
                    "{ctx}.quality: unknown noise quality"
                )));
            }
        };
        Ok(Self {
            separate_axes: boolean("separateAxes")?,
            strength: curve("strength")?,
            strength_y: curve("strengthY")?,
            strength_z: curve("strengthZ")?,
            frequency: number("frequency")?,
            damping: boolean("damping")?,
            octaves: unsigned("octaves")?,
            octave_multiplier: number("octaveMultiplier")?,
            octave_scale: number("octaveScale")?,
            quality,
            dimensions: unsigned("dimensions")?,
            scroll_speed: curve("scrollSpeed")?,
            remap_enabled: boolean("remapEnabled")?,
            remap: curve("remap")?,
            remap_y: curve("remapY")?,
            remap_z: curve("remapZ")?,
            position_amount: curve("positionAmount")?,
            rotation_amount: curve("rotationAmount")?,
            size_amount: curve("sizeAmount")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::noise::NoiseLaw;

    const SOURCE: &str = r#"{
        "separateAxes":false,"strength":{"mode":"constant","value":0.3},
        "strengthY":{"mode":"twoConstants","min":-2,"max":3},
        "strengthZ":{"mode":"constant","value":-0.0},
        "frequency":0.5,"damping":true,"octaves":1,"octaveMultiplier":0.75,"octaveScale":2.25,
        "quality":"high","dimensions":3,"scrollSpeed":{"mode":"constant","value":-1},
        "remapEnabled":false,"remap":{"mode":"constant","value":1},
        "remapY":{"mode":"twoConstants","min":0.25,"max":0.75},
        "remapZ":{"mode":"constant","value":-1},
        "positionAmount":{"mode":"constant","value":1},
        "rotationAmount":{"mode":"constant","value":0},"sizeAmount":{"mode":"constant","value":0}
    }"#;

    fn decode(source: &str) -> Result<NoiseParams, EffectsError> {
        NoiseParams::from_value(&json::parse(source.as_bytes()).unwrap(), "effect/test")
    }

    #[test]
    fn dormant_axes_and_remaps_remain_typed_without_changing_qualified_primary_path() {
        let params = decode(SOURCE).unwrap();
        assert_eq!(
            params.strength_y,
            MinMaxCurve::TwoConstants {
                min: -2.0,
                max: 3.0
            }
        );
        assert_eq!(
            params.remap_y,
            MinMaxCurve::TwoConstants {
                min: 0.25,
                max: 0.75
            }
        );
        assert_eq!(params.remap_z, MinMaxCurve::Constant(-1.0));
        let MinMaxCurve::Constant(z) = params.strength_z else {
            panic!("constant Z");
        };
        assert_eq!(z.to_bits(), (-0.0f32).to_bits());
        assert_eq!(
            (params.octave_multiplier, params.octave_scale),
            (0.75, 2.25)
        );
        assert!(NoiseLaw::from_params(&params).is_ok());
    }

    #[test]
    fn unsupported_authored_controls_survive_decoding_and_fail_only_the_law_gate() {
        for source in [
            SOURCE.replace("\"separateAxes\":false", "\"separateAxes\":true"),
            SOURCE.replace("\"remapEnabled\":false", "\"remapEnabled\":true"),
            SOURCE.replace("\"octaves\":1", "\"octaves\":3"),
            SOURCE.replace(
                "\"quality\":\"high\",\"dimensions\":3",
                "\"quality\":\"low\",\"dimensions\":1",
            ),
            SOURCE.replace(
                "\"strength\":{\"mode\":\"constant\",\"value\":0.3}",
                "\"strength\":{\"mode\":\"twoConstants\",\"min\":0.1,\"max\":0.9}",
            ),
        ] {
            let params = decode(&source).expect("authored unsupported input must remain decodable");
            assert!(NoiseLaw::from_params(&params).is_err());
        }
        let low = decode(&SOURCE.replace("\"high\"", "\"medium\"")).unwrap();
        assert_eq!(low.quality, NoiseQuality::Medium);
        assert_eq!(
            low.dimensions, 3,
            "preserve both fields; law rejects inconsistent pair"
        );
    }

    #[test]
    fn curve_and_two_curve_noise_fields_keep_coefficients_and_weighted_keys() {
        let source = SOURCE.replace("\"strength\":{\"mode\":\"constant\",\"value\":0.3}",
            r#""strength":{"mode":"curve","multiplier":2,"keys":[
                {"time":0,"value":3,"inSlope":-4,"outSlope":5,"weightedMode":3,"inWeight":0.25,"outWeight":0.75}]}"#)
            .replace("\"remapZ\":{\"mode\":\"constant\",\"value\":-1}",
                r#""remapZ":{"mode":"twoCurves","multiplier":-2,"minKeys":[],"maxKeys":[
                    {"time":1,"value":-3,"inSlope":"Infinity","outSlope":"-Infinity","weightedMode":0}]}"#);
        let params = decode(&source).unwrap();
        let MinMaxCurve::Curve { multiplier, max } = &params.strength else {
            panic!("curve preserved");
        };
        assert_eq!(*multiplier, 2.0);
        assert_eq!((max.keys[0].in_slope, max.keys[0].out_slope), (-4.0, 5.0));
        assert_eq!(
            (
                max.keys[0].weighted_mode,
                max.keys[0].in_weight,
                max.keys[0].out_weight
            ),
            (3, 0.25, 0.75)
        );
        let MinMaxCurve::TwoCurves {
            multiplier,
            min,
            max,
        } = &params.remap_z
        else {
            panic!("two curves preserved");
        };
        assert_eq!(*multiplier, -2.0);
        assert!(min.keys.is_empty());
        assert_eq!(
            (max.keys[0].in_slope, max.keys[0].out_slope),
            (f32::INFINITY, f32::NEG_INFINITY)
        );
        assert!(NoiseLaw::from_params(&params).is_err());
    }

    #[test]
    fn malformed_required_noise_values_have_source_location() {
        for (source, key) in [
            (
                SOURCE.replace("\"quality\":\"high\"", "\"quality\":\"future\""),
                "quality",
            ),
            (
                SOURCE.replace("\"octaves\":1", "\"octaves\":1.5"),
                "octaves",
            ),
            (
                SOURCE.replace("\"frequency\":0.5", "\"frequency\":null"),
                "frequency",
            ),
            (SOURCE.replace("\"remapY\":", "\"absentRemapY\":"), "remapY"),
        ] {
            let error = decode(&source).unwrap_err();
            assert!(
                error.0.contains(&format!("effect/test.noise.{key}")),
                "{error}"
            );
        }
    }
}
