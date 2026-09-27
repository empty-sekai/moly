use super::*;

/// Authored texture-sheet controls, including dormant mode fields. Keeping
/// them typed prevents a renderer from silently assuming the default grid.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureSheetParams {
    pub mode: u32,
    pub time_mode: u32,
    pub animation_type: u32,
    pub tiles: [u32; 2],
    pub row_mode: u32,
    pub row_index: i32,
    pub cycles: f32,
    pub fps: f32,
    pub speed_range: [f32; 2],
    pub uv_channel_mask: i32,
    pub flip: [f32; 2],
    pub frame: MinMaxCurve,
    pub start: MinMaxCurve,
}

impl TextureSheetParams {
    pub(super) fn from_value(v: &Value, ctx: &str) -> Result<Self, EffectsError> {
        let ctx = format!("{ctx}.textureSheet");
        let number = |key| f32_of(v.get(key), &format!("{ctx}.{key}"));
        let unsigned = |key| u32_of(v.get(key), &format!("{ctx}.{key}"));
        let signed = |key| v.get(key).and_then(Value::as_f64)
            .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64)
            .map(|n| n as i32).ok_or_else(|| EffectsError(format!("{ctx}.{key}: signed integer expected")));
        Ok(Self {
            mode: unsigned("mode")?, time_mode: unsigned("timeMode")?,
            animation_type: unsigned("animationType")?,
            tiles: [unsigned("tilesX")?, unsigned("tilesY")?],
            row_mode: unsigned("rowMode")?, row_index: signed("rowIndex")?,
            cycles: number("cycles")?, fps: number("fps")?,
            speed_range: vec2_of(v.get("speedRange"), &format!("{ctx}.speedRange"))?,
            uv_channel_mask: signed("uvChannelMask")?, flip: [number("flipU")?, number("flipV")?],
            frame: min_max_curve(v.get("frameOverTime"), &format!("{ctx}.frameOverTime"))?,
            start: min_max_curve(v.get("startFrame"), &format!("{ctx}.startFrame"))?,
        })
    }
}
