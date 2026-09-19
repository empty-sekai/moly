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

/// The initial module's LDR min-max-gradient dispatcher, not its HDR overload.
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

    #[test]
    fn repeated_render_evaluation_never_accumulates_into_birth_colour() {
        let law = ColorOverLifetime::from_params(&MinMaxGradient::Color([0.5; 4]));
        let birth = [255, 192, 128, 64];
        assert_eq!(law.apply(birth, 17, 0.0), [128, 96, 64, 32]);
        assert_eq!(law.apply(birth, 17, 10.0), law.apply(birth, 17, 0.0));
        assert_eq!(birth, [255, 192, 128, 64]);
    }
}
