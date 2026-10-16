//! Birth colour and render-time colour are separate native operations. Birth
//! evaluates the raw gradient; colour over lifetime uses an optimized byte
//! table and multiplies an immutable birth colour into the render buffer.
use super::MinMaxGradient;
use super::curve::normalized_age;
use super::gradient::{Gradient, PackedGradient,
    lerp_rgba8, multiply_rgba8, quantize_rgba8};
use super::random::ParticleRandom;

fn factor_byte(random: f32) -> u8 { (random * 255.0) as i32 as u8 }
fn mix_float(min: [f32; 4], max: [f32; 4], random: f32) -> [f32; 4] {
    std::array::from_fn(|axis| (max[axis] - min[axis]) * random + min[axis])
}

/// The initial module's LDR min-max-gradient dispatcher over one four-lane
/// group. The gradient kernels search their keys for the four lanes
/// together, so with unordered key codes a lane's colour can depend on the
/// other lanes' times (or, for a random colour, draws); the constant and
/// two-colour templates are per lane.
pub fn initial_rgba8x4(source: &MinMaxGradient, time: [f32; 4], random: [f32; 4]) -> [[u8; 4]; 4] {
    match source {
        MinMaxGradient::Gradient(gradient) => gradient.evaluate4(time).map(quantize_rgba8),
        MinMaxGradient::RandomColor(gradient) => gradient.evaluate4(random).map(quantize_rgba8),
        MinMaxGradient::TwoGradients { min, max } => {
            let (min, max) = (min.evaluate4(time), max.evaluate4(time));
            std::array::from_fn(|lane| {
                lerp_rgba8(quantize_rgba8(min[lane]), quantize_rgba8(max[lane]), factor_byte(random[lane]))
            })
        }
        MinMaxGradient::Color(_) | MinMaxGradient::TwoColors { .. } => {
            std::array::from_fn(|lane| initial_rgba8(source, time[lane], random[lane]))
        }
    }
}

/// The initial module's LDR min-max-gradient dispatcher, not its HDR overload,
/// for one lane evaluated on its own (the four lanes agreeing).
pub fn initial_rgba8(source: &MinMaxGradient, time: f32, random: f32) -> [u8; 4] {
    match source {
        MinMaxGradient::Color(color) => quantize_rgba8(*color),
        MinMaxGradient::Gradient(gradient) => quantize_rgba8(gradient.evaluate(time)),
        MinMaxGradient::RandomColor(gradient) => quantize_rgba8(gradient.evaluate(random)),
        MinMaxGradient::TwoColors { min, max } => quantize_rgba8(mix_float(*min, *max, random)),
        MinMaxGradient::TwoGradients { min, max } => lerp_rgba8(
            quantize_rgba8(min.evaluate(time)), quantize_rgba8(max.evaluate(time)), factor_byte(random)),
    }
}

#[derive(Clone, Debug)]
pub enum ColorOverLifetime {
    Color([u8; 4]),
    TwoColors { min: [f32; 4], max: [f32; 4] },
    Gradient(PackedGradient),
    TwoGradients { min: PackedGradient, max: PackedGradient },
    RandomColor(Gradient),
}

impl ColorOverLifetime {
    pub fn from_params(source: &MinMaxGradient) -> Self {
        match source {
            MinMaxGradient::Color(color) => Self::Color(quantize_rgba8(*color)),
            MinMaxGradient::TwoColors { min, max } => Self::TwoColors { min: *min, max: *max },
            MinMaxGradient::Gradient(gradient) => Self::Gradient(PackedGradient::from_gradient(gradient)),
            MinMaxGradient::TwoGradients { min, max } => Self::TwoGradients {
                min: PackedGradient::from_gradient(min), max: PackedGradient::from_gradient(max) },
            MinMaxGradient::RandomColor(gradient) => Self::RandomColor(gradient.clone()),
        }
    }

    pub fn evaluate(&self, seed: u32, age_percent: f32) -> [u8; 4] {
        let time = normalized_age(age_percent);
        let random = ParticleRandom::sample(seed, 0x591b_c05c);
        match self {
            Self::Color(color) => *color,
            Self::TwoColors { min, max } => quantize_rgba8(mix_float(*min, *max, random)),
            Self::Gradient(gradient) => gradient.evaluate(time),
            Self::TwoGradients { min, max } => lerp_rgba8(
                min.evaluate(time), max.evaluate(time), factor_byte(random)),
            Self::RandomColor(gradient) => quantize_rgba8(gradient.evaluate(random)),
        }
    }

    pub fn apply(&self, birth: [u8; 4], seed: u32, age_percent: f32) -> [u8; 4] {
        multiply_rgba8(birth, self.evaluate(seed, age_percent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::gradient::{GradientColorKey, GradientAlphaKey};

    fn flat(value: f32) -> Gradient {
        Gradient {
            color_keys: [0.0, 1.0].map(|time| GradientColorKey { time, color: [value; 3] }).to_vec(),
            alpha_keys: [0.0, 1.0].map(|time| GradientAlphaKey { time, alpha: value }).to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn two_colours_and_two_gradients_do_not_share_a_float_lerp() {
        let colours = MinMaxGradient::TwoColors { min: [0.0; 4], max: [1.0; 4] };
        let gradients = MinMaxGradient::TwoGradients { min: flat(0.0), max: flat(1.0) };
        assert_eq!(initial_rgba8(&colours, 0.5, 1.0), [255; 4]);
        assert_eq!(initial_rgba8(&gradients, 0.5, 1.0), [254; 4]);
        assert_eq!(initial_rgba8(&colours, 0.5, 0.5), [128; 4]);
        assert_eq!(initial_rgba8(&gradients, 0.5, 0.5), [127; 4]);
    }

    /// Research instrument: native Gradient::EvaluateHDR (Blend and Fixed)
    /// and the start colour's MinMaxGradient dispatcher, executed on finite
    /// and non-finite queries (NaN of either sign and any payload, signalling
    /// NaN, infinities, finite times whose code overflows, NaN or infinity
    /// beside finite lanes) over ordered, unordered, reversed, one-key and
    /// equal-code tables, replayed through the product's `evaluate4` and
    /// `initial_rgba8x4`. Kernel channels compare bit for bit, NaN by class
    /// (the payload is not part of the contract: the start colour quantizes a
    /// NaN channel to 0); dispatcher bytes compare exactly.
    #[test]
    #[ignore = "set MOLY_GRADIENT_NONFINITE_ROWS to the native gradient rows"]
    fn gradient_evaluation_matches_native_on_non_finite_queries() {
        use crate::particle::gradient::{GradientColorSpace, GradientMode};
        use crate::particle::json::{self, Value};
        let path = std::env::var_os("MOLY_GRADIENT_NONFINITE_ROWS")
            .expect("MOLY_GRADIENT_NONFINITE_ROWS names the native gradient rows");
        let file = json::parse(&std::fs::read(path).expect("native rows path")).unwrap();
        assert_eq!(
            file.get("summary").and_then(|s| s.get("library")).and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        fn word(value: &Value) -> u32 {
            let n = value.as_f64().unwrap();
            assert!(n >= 0.0 && n <= u32::MAX as f64 && n.fract() == 0.0, "not a u32: {n}");
            n as u32
        }
        fn words4(value: &Value) -> [u32; 4] {
            let list = value.as_array().unwrap();
            assert_eq!(list.len(), 4);
            std::array::from_fn(|i| word(&list[i]))
        }
        fn gradient(value: &Value) -> Gradient {
            let keys = |name: &str| value.get(name).and_then(Value::as_array).unwrap();
            Gradient {
                color_keys: keys("colourKeys").iter().map(|key| {
                    let key = key.as_array().unwrap();
                    GradientColorKey {
                        time: word(&key[0]) as f32 / 65535.0,
                        color: std::array::from_fn(|c| f32::from_bits(word(&key[1 + c]))),
                    }
                }).collect(),
                alpha_keys: keys("alphaKeys").iter().map(|key| {
                    let key = key.as_array().unwrap();
                    GradientAlphaKey { time: word(&key[0]) as f32 / 65535.0, alpha: f32::from_bits(word(&key[1])) }
                }).collect(),
                mode: match word(value.get("mode").unwrap()) {
                    0 => GradientMode::Blend,
                    1 => GradientMode::Fixed,
                    other => panic!("gradient mode {other}"),
                },
                color_space: GradientColorSpace::Unspecified,
            }
        }
        let (mut kernels, mut dispatches, mut nan_channels) = (0, 0, 0);
        for (index, row) in file.get("rows").and_then(Value::as_array).unwrap().iter().enumerate() {
            let label = format!(
                "row {index} {} {}",
                row.get("gradient").and_then(Value::as_str).unwrap(),
                row.get("query").and_then(Value::as_str).unwrap()
            );
            match row.get("kind").and_then(Value::as_str) {
                Some("kernel") => {
                    let ours = gradient(row).evaluate4(words4(row.get("queryBits").unwrap()).map(f32::from_bits));
                    let native = row.get("outBits").and_then(Value::as_array).unwrap();
                    for lane in 0..4 {
                        let expected = words4(&native[lane]);
                        for channel in 0..4 {
                            let (value, bits) = (ours[lane][channel], expected[channel]);
                            let native_nan = f32::from_bits(bits).is_nan();
                            assert!(
                                value.to_bits() == bits || (value.is_nan() && native_nan),
                                "{label} lane {lane} channel {channel}: {:#010x} native {bits:#010x}",
                                value.to_bits()
                            );
                            nan_channels += usize::from(native_nan);
                        }
                    }
                    kernels += 1;
                }
                Some("dispatch") => {
                    let max = gradient(row.get("max").unwrap());
                    let source = match word(row.get("state").unwrap()) {
                        1 => MinMaxGradient::Gradient(max),
                        3 => MinMaxGradient::TwoGradients { min: gradient(row.get("min").unwrap()), max },
                        4 => MinMaxGradient::RandomColor(max),
                        other => panic!("{label}: dispatcher state {other}"),
                    };
                    let ours = initial_rgba8x4(
                        &source,
                        words4(row.get("timeBits").unwrap()).map(f32::from_bits),
                        words4(row.get("randomBits").unwrap()).map(f32::from_bits),
                    );
                    let native = row.get("bytes").and_then(Value::as_array).unwrap();
                    for lane in 0..4 {
                        let expected: [u8; 4] = words4(&native[lane]).map(|b| u8::try_from(b).unwrap());
                        assert_eq!(ours[lane], expected, "{label} dispatcher lane {lane}");
                    }
                    dispatches += 1;
                }
                other => panic!("{label}: row kind {other:?}"),
            }
        }
        println!("native gradient rows: {kernels} kernel rows exact ({nan_channels} NaN lane-channels), {dispatches} dispatcher rows exact");
        assert!(kernels > 0 && dispatches > 0 && nan_channels > 0);
    }

    #[test]
    fn repeated_render_evaluation_never_accumulates_into_birth_colour() {
        let law = ColorOverLifetime::from_params(&MinMaxGradient::Color([0.5; 4]));
        let birth = [255, 192, 128, 64];
        assert_eq!(law.apply(birth, 17, 0.0), [128, 96, 64, 32]);
        assert_eq!(law.apply(birth, 17, 10.0), law.apply(birth, 17, 0.0));
        assert_eq!(birth, [255, 192, 128, 64]);
    }
}
