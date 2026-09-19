//! Authored gradient metadata and unquantized native gradient evaluation.
//! Render-time packed colour interpolation is a separate operation; in
//! particular, a fixed gradient's optimized table has shifted boundaries.
mod packed;
pub use packed::{PackedGradient, lerp_rgba8, multiply_rgba8, quantize_rgba8, rgba8_to_float};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientMode {
    #[default]
    Blend,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientColorSpace {
    #[default]
    Unspecified,
    Gamma,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientColorKey {
    pub time: f32,
    pub color: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientAlphaKey {
    pub time: f32,
    pub alpha: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gradient {
    pub color_keys: Vec<GradientColorKey>,
    pub alpha_keys: Vec<GradientAlphaKey>,
    pub mode: GradientMode,
    /// Authored metadata, not permission to insert a colour conversion. The
    /// native gradient evaluation kernels consume the stored key components.
    pub color_space: GradientColorSpace,
}

/// Normalized JSON key times retain all 16-bit source time codes. Recovering
/// the code before evaluation also preserves colour/alpha table preparation's
/// distinct floating-point operations.
pub fn time_code(time: f32) -> u16 {
    (time * 65535.0).round().clamp(0.0, 65535.0) as u16
}

impl Gradient {
    /// Evaluate in the source's 16-bit time domain without quantizing colour.
    /// A raw native table with fewer than two keys leaves that channel white.
    /// Authoring APIs normally expand constant gradients to two keys.
    pub fn evaluate(&self, time: f32) -> [f32; 4] {
        let t = time * 65535.0;
        let mut result = [1.0; 4];
        if let Some((lo, hi, factor)) = interval(self.color_keys.len(),
            |index| self.color_keys[index].time, t, self.mode)
        {
            for axis in 0..3 {
                let a = self.color_keys[lo].color[axis];
                let b = self.color_keys[hi].color[axis];
                result[axis] = (b - a) * factor + a;
            }
        }
        if let Some((lo, hi, factor)) = interval(self.alpha_keys.len(),
            |index| self.alpha_keys[index].time, t, self.mode)
        {
            let a = self.alpha_keys[lo].alpha;
            let b = self.alpha_keys[hi].alpha;
            result[3] = (b - a) * factor + a;
        }
        result
    }
}

fn interval(count: usize, key_time: impl Fn(usize) -> f32, query: f32,
    mode: GradientMode) -> Option<(usize, usize, f32)>
{
    if count < 2 { return None; }
    let query = query.clamp(time_code(key_time(0)) as f32,
        time_code(key_time(count - 1)) as f32);
    if mode == GradientMode::Fixed {
        let index = (0..count).find(|&index| time_code(key_time(index)) as f32 >= query)
            .unwrap_or(count - 1);
        return Some((index, index, 0.0));
    }
    let hi = (1..count).find(|&index| time_code(key_time(index)) as f32 >= query)
        .unwrap_or(count - 1);
    let lo = hi - 1;
    let start = time_code(key_time(lo)) as f32;
    let end = time_code(key_time(hi)) as f32;
    let factor = ((query - start) / (end - start).max(f32::from_bits(0x3586_37bd))).min(1.0);
    Some((lo, hi, factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: GradientMode) -> Gradient {
        Gradient {
            mode,
            color_space: GradientColorSpace::Gamma,
            color_keys: vec![
                GradientColorKey { time: 0.0, color: [1.0, 0.0, 0.0] },
                GradientColorKey { time: 32768.0 / 65535.0, color: [0.0, 1.0, 0.0] },
                GradientColorKey { time: 1.0, color: [0.0, 0.0, 1.0] },
            ],
            alpha_keys: vec![GradientAlphaKey { time: 0.0, alpha: 0.2 },
                GradientAlphaKey { time: 1.0, alpha: 1.0 }],
        }
    }

    #[test]
    fn normalized_storage_recovers_every_source_time_code() {
        for code in 0..=u16::MAX {
            assert_eq!(time_code(code as f32 / 65535.0), code);
        }
    }

    #[test]
    fn fixed_gradient_uses_the_upcoming_key_not_the_previous_key() {
        let gradient = fixture(GradientMode::Fixed);
        assert_eq!(gradient.evaluate(0.0), [1.0, 0.0, 0.0, 0.2]);
        assert_eq!(gradient.evaluate(0.25), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(gradient.evaluate(0.75), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn key_time_quantization_is_visible_at_the_midpoint() {
        let value = fixture(GradientMode::Blend).evaluate(0.5);
        assert_eq!(value[..3], [1.52587890625e-5, 0.9999847412109375, 0.0]);
        assert_eq!(value[3], 0.6);
    }
}
