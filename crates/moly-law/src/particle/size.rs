//! Current particle dimensions derived from immutable birth dimensions.
use super::curve::{CurveSampler, normalized_age};
use super::random::ParticleRandom;
use super::schema::SizeOverLifetimeParams;

#[derive(Clone, Debug)]
pub struct SizeOverLifetime {
    axes: [CurveSampler; 3],
}

impl SizeOverLifetime {
    pub fn from_params(params: &SizeOverLifetimeParams) -> Self {
        let curves = if params.separate_axes {
            [&params.curve,
                params.y.as_ref().expect("typed separate-axis size Y"),
                params.z.as_ref().expect("typed separate-axis size Z")]
        } else { [&params.curve; 3] };
        Self { axes: curves.map(CurveSampler::from_min_max_curve) }
    }

    /// Every axis uses the same particle-local random factor. The nonnegative
    /// clamp applies to the curve factor, before multiplication by birth size.
    pub fn evaluate(&self, birth_size: [f32; 3], seed: u32, age_percent: f32) -> [f32; 3] {
        let t = normalized_age(age_percent);
        let random = ParticleRandom::sample(seed, 0x8d2c_8431);
        std::array::from_fn(|axis| birth_size[axis] * self.axes[axis].evaluate(t, random).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::MinMaxCurve;

    #[test]
    fn size_axes_share_a_factor_but_not_their_authored_ranges() {
        let law = SizeOverLifetime::from_params(&SizeOverLifetimeParams {
            separate_axes: true,
            curve: MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 },
            y: Some(MinMaxCurve::TwoConstants { min: 0.0, max: 2.0 }),
            z: Some(MinMaxCurve::TwoConstants { min: 0.0, max: 4.0 }),
        });
        let size = law.evaluate([1.0; 3], 17, 45.0);
        assert!(size[0] > 0.0 && size[0] < 1.0);
        assert_eq!(size[1], size[0] * 2.0);
        assert_eq!(size[2], size[0] * 4.0);
        assert_eq!(size, law.evaluate([1.0; 3], 17, 0.0));
    }

    #[test]
    fn negative_factor_is_not_a_mirrored_particle() {
        let law = SizeOverLifetime::from_params(&SizeOverLifetimeParams {
            separate_axes: false, curve: MinMaxCurve::Constant(-2.0), y: None, z: None,
        });
        assert_eq!(law.evaluate([1.0, 2.0, 3.0], 17, 0.0), [0.0; 3]);
    }
}
