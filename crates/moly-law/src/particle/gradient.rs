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
    /// Evaluate in the source's 16-bit time domain without quantizing colour:
    /// one lane of `evaluate4` with the same time in all four lanes, where
    /// the kernels' shared key search is an ordinary per-lane search.
    pub fn evaluate(&self, time: f32) -> [f32; 4] {
        self.evaluate4([time; 4])[0]
    }

    /// Whether every key group of two or more keys has its 16-bit time codes
    /// in non-decreasing order. Then `evaluate4` gives each lane the value
    /// `evaluate` gives its query alone, whatever the other lanes' queries:
    /// a lane is bounded by its first key at or above its query and stays
    /// bounded by every later key, and a group of fewer keys is skipped for
    /// every lane. With codes out of order the shared search can carry a
    /// bounded lane on to a later key while another lane is unbounded.
    pub fn key_search_is_per_lane(&self) -> bool {
        fn ordered(codes: &[u16]) -> bool {
            codes.windows(2).all(|pair| pair[0] <= pair[1])
        }
        let colour: Vec<u16> = self.color_keys.iter().map(|key| time_code(key.time)).collect();
        let alpha: Vec<u16> = self.alpha_keys.iter().map(|key| time_code(key.time)).collect();
        ordered(&colour) && ordered(&alpha)
    }

    /// Gradient::EvaluateHDR's Blend and Fixed kernels over one four-lane
    /// query, in native order. The colour keys, then the alpha keys, each
    /// group on its own 16-bit time codes: the query is the time times 65535,
    /// raised to the first code and then lowered to the last (maximum then
    /// minimum, NaN kept; with the codes out of order every query becomes
    /// the last code). A group with fewer than two keys is skipped and its
    /// channels stay 1.0. The key search is shared by the four lanes (padding
    /// lanes included), so with unordered codes a lane's value can depend on
    /// the other lanes' queries; see `key_group`.
    pub fn evaluate4(&self, time: [f32; 4]) -> [[f32; 4]; 4] {
        let query = time.map(|t| t * 65535.0);
        let mut result = [[1.0; 4]; 4];
        let codes: Vec<f32> =
            self.color_keys.iter().map(|key| time_code(key.time) as f32).collect();
        let colours: Vec<[f32; 3]> = self.color_keys.iter().map(|key| key.color).collect();
        if let Some(values) = key_group(&codes, &colours, query, self.mode) {
            for lane in 0..4 {
                result[lane][..3].copy_from_slice(&values[lane]);
            }
        }
        let codes: Vec<f32> =
            self.alpha_keys.iter().map(|key| time_code(key.time) as f32).collect();
        let alphas: Vec<[f32; 1]> = self.alpha_keys.iter().map(|key| [key.alpha]).collect();
        if let Some(values) = key_group(&codes, &alphas, query, self.mode) {
            for lane in 0..4 {
                result[lane][3] = values[lane][0];
            }
        }
        result
    }
}

/// One key group of the four-lane kernel, or None where the kernel skips it
/// (fewer than two keys, or no key bounds any lane's query). The first key
/// examined (Blend from the second key, Fixed from the first) is the first
/// at which some lane's query is not above the code. From there, key by key
/// until every lane is bounded or the keys run out: the lanes that were not
/// bounded after the previous key (all lanes at the first) take this key's
/// value, then each lane is bounded again exactly when this key's code is
/// at or above its query. The bound is recomputed per key, not accumulated,
/// so a key with a lower code after the bounding one updates the lane again;
/// with codes in order this is the per-lane first key at or above the query.
/// Blend's value is the previous key's plus the factor times the difference,
/// the factor being (query - previous code) / max(code difference, 1e-6)
/// lowered to 1; Fixed takes the key itself.
fn key_group<const N: usize>(
    codes: &[f32],
    values: &[[f32; N]],
    query: [f32; 4],
    mode: GradientMode,
) -> Option<[[f32; N]; 4]> {
    let count = codes.len();
    if count < 2 {
        return None;
    }
    let query = query.map(|q| arm_fmin(arm_fmax(q, codes[0]), codes[count - 1]));
    let first = match mode {
        GradientMode::Blend => 1,
        GradientMode::Fixed => 0,
    };
    let start = (first..count).find(|&k| query.iter().any(|&q| !(q > codes[k])))?;
    let mut result = [[1.0; N]; 4];
    let mut bounded = [false; 4];
    for key in start..count {
        if bounded.iter().all(|&b| b) {
            break;
        }
        let update = bounded.map(|b| !b);
        let value: [[f32; N]; 4] = match mode {
            GradientMode::Fixed => [values[key]; 4],
            GradientMode::Blend => {
                let (lo, hi) = (codes[key - 1], codes[key]);
                let span = arm_fmax(hi - lo, f32::from_bits(0x3586_37bd));
                let (a, b) = (values[key - 1], values[key]);
                std::array::from_fn(|lane| {
                    let factor = arm_fmin((query[lane] - lo) / span, 1.0);
                    std::array::from_fn(|c| a[c] + factor * (b[c] - a[c]))
                })
            }
        };
        bounded = query.map(|q| codes[key] >= q);
        for lane in 0..4 {
            if update[lane] {
                result[lane] = value[lane];
            }
        }
    }
    Some(result)
}

/// ARM FMAX: NaN if either operand is NaN, +0 over -0.
pub(crate) fn arm_fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b || (a == b && a.is_sign_positive()) {
        a
    } else {
        b
    }
}

/// ARM FMIN: NaN if either operand is NaN, -0 over +0.
pub(crate) fn arm_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
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
