use super::{Gradient, GradientMode, time_code};

/// The non-incremental colour module interpolates byte colours using an
/// eight-bit factor and a denominator of 256, not floating-point Color.Lerp.
pub fn lerp_rgba8(min: [u8; 4], max: [u8; 4], factor: u8) -> [u8; 4] {
    std::array::from_fn(|axis| {
        let lo = min[axis] as i32;
        let delta = max[axis] as i32 - lo;
        (lo + ((delta * factor as i32 + 128) >> 8)) as u8
    })
}

/// Rounded division by 255, distinct from the gradient's interpolation rule.
pub fn multiply_rgba8(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    std::array::from_fn(|axis| {
        let product = a[axis] as u32 * b[axis] as u32 + 128;
        ((product + (product >> 8)) >> 8) as u8
    })
}

pub fn quantize_rgba8(value: [f32; 4]) -> [u8; 4] {
    value.map(|channel| (channel.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

pub fn rgba8_to_float(value: [u8; 4]) -> [f32; 4] {
    value.map(|channel| channel as f32 * f32::from_bits(0x3b80_8081))
}

#[derive(Clone, Debug)]
pub struct PackedGradient {
    mode: GradientMode,
    len: usize,
    times: [f32; 16],
    colours: [[u8; 4]; 16],
    inverse_spans: [f32; 16],
}

impl PackedGradient {
    pub fn from_gradient(gradient: &Gradient) -> Self {
        let mut union = Vec::with_capacity(16);
        for key in &gradient.color_keys {
            union.push(time_code(key.time) as f32 * f32::from_bits(0x3780_0080));
        }
        for key in &gradient.alpha_keys {
            union.push(time_code(key.time) as f32 / 65535.0);
        }
        union.sort_by(f32::total_cmp);
        union.dedup_by(|a, b| *a == *b);
        if gradient.mode == GradientMode::Fixed {
            for time in &mut union { *time -= f32::from_bits(0x3780_0080); }
        }
        if union.len() < 16 { union.push(1.0); }
        else { union.truncate(16); union[15] = 1.0; }
        let mut result = Self { mode: gradient.mode, len: union.len(),
            times: [0.0; 16], colours: [[255; 4]; 16], inverse_spans: [0.0; 16] };
        for (index, time) in union.into_iter().enumerate() {
            result.times[index] = time;
            result.colours[index] = quantize_rgba8(gradient.evaluate(time));
            if index > 0 {
                result.inverse_spans[index] = reciprocal(
                    (time - result.times[index - 1]).max(f32::from_bits(0x3586_37bd)));
            }
        }
        result
    }

    pub fn evaluate(&self, time: f32) -> [u8; 4] {
        match self.mode {
            GradientMode::Fixed => {
                let Some(start) = (0..self.len).find(|&index| self.times[index] >= time) else {
                    return [255; 4];
                };
                let mut result = [255; 4];
                for index in start..self.len {
                    result = self.colours[index];
                    if self.times[index] > time { break; }
                }
                result
            }
            GradientMode::Blend => {
                let Some(start) = (1..self.len).find(|&index| self.times[index] >= time) else {
                    return [255; 4];
                };
                let mut result = [255; 4];
                for hi in start..self.len {
                    let lo = hi - 1;
                    let delta = (time - self.times[lo]).clamp(0.0, 1.0);
                    let factor = ((delta * self.inverse_spans[hi]) * 255.0) as i32 as u8;
                    result = lerp_rgba8(self.colours[lo], self.colours[hi], factor);
                    if self.times[hi] > time { break; }
                }
                result
            }
        }
    }
}

/// Reciprocal-estimate refinement used when preparing native gradient tables.
/// All callers pass a positive, finite, normal interval bounded below by 1e-6.
/// The initial estimate has eight mantissa bits. Each refinement has one fused
/// residual followed by a rounded multiply, matching the source SIMD kernel.
fn reciprocal(value: f32) -> f32 {
    debug_assert!(value.is_normal() && value > 0.0);
    let bits = value.to_bits();
    let exponent = (bits >> 23) & 255;
    let bucket = ((bits & 0x007f_ffff) | 0x0080_0000) >> 15;
    let estimate = (262144 + bucket) / (2 * bucket + 1);
    let first = f32::from_bits(((253 - exponent) << 23) | ((estimate - 256) << 15));
    let second = first * (-value).mul_add(first, 2.0);
    second * (-value).mul_add(second, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::gradient::{GradientColorKey, GradientAlphaKey};

    #[test]
    fn byte_lerp_and_multiply_have_different_endpoint_rules() {
        assert_eq!(lerp_rgba8([0; 4], [255; 4], 255), [254; 4]);
        assert_eq!(multiply_rgba8([255; 4], [255; 4]), [255; 4]);
        assert_eq!(multiply_rgba8([0, 1, 128, 255], [255; 4]), [0, 1, 128, 255]);
    }

    #[test]
    fn source_reciprocal_preserves_refinement_rounding() {
        assert_eq!(reciprocal(f32::from_bits(0x3586_37bd)), 1000000.0625);
        assert_eq!(reciprocal(1.0), 1.0);
    }

    #[test]
    fn a_fixed_optimized_table_is_not_an_unshifted_float_gradient() {
        let gradient = Gradient {
            mode: GradientMode::Fixed,
            color_keys: vec![
                GradientColorKey { time: 0.0, color: [1.0, 0.0, 0.0] },
                GradientColorKey { time: 0.5, color: [0.0, 1.0, 0.0] },
                GradientColorKey { time: 1.0, color: [0.0, 0.0, 1.0] }],
            alpha_keys: vec![GradientAlphaKey { time: 0.0, alpha: 1.0 },
                GradientAlphaKey { time: 1.0, alpha: 1.0 }],
            ..Default::default()
        };
        assert_eq!(gradient.evaluate(0.0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(PackedGradient::from_gradient(&gradient).evaluate(0.0), [0, 255, 0, 255]);
    }
}
