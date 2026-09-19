use super::OrbitalMotion;
use crate::particle::curve::{CurveSampler, normalized_age};
use crate::particle::random::ParticleRandom;
use crate::particle::schema::VelocityOverLifetimeParams;
use crate::particle::MinMaxCurve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VelocitySample {
    pub linear: [f32; 3],
    pub orbital: OrbitalMotion,
    pub speed_modifier: f32,
}

/// Prepared once per emitter, without allocating curve data in the frame loop.
#[derive(Clone, Debug)]
pub struct VelocityOverLifetime {
    pub in_world_space: bool,
    linear: [CurveSampler; 3],
    angular: [CurveSampler; 3],
    offset: [CurveSampler; 3],
    radial: CurveSampler,
    speed: CurveSampler,
}

fn axis_group(curves: [&MinMaxCurve; 3]) -> [CurveSampler; 3] {
    // Native axis-group dispatch only uses its optimized path when all axes
    // qualify. A complex axis must not leave its siblings on a different path.
    let baked = curves.iter().all(|curve| CurveSampler::can_bake(curve));
    curves.map(|curve| CurveSampler::with_baking(curve, baked))
}

impl VelocityOverLifetime {
    pub fn from_params(params: &VelocityOverLifetimeParams) -> Self {
        Self {
            in_world_space: params.in_world_space,
            linear: axis_group([&params.x, &params.y, &params.z]),
            angular: axis_group(std::array::from_fn(|axis| &params.orbital[axis])),
            offset: axis_group(std::array::from_fn(|axis| &params.orbital_offset[axis])),
            radial: CurveSampler::from_min_max_curve(&params.radial),
            speed: CurveSampler::from_min_max_curve(&params.speed_modifier),
        }
    }

    /// `batch_seed` is the first particle seed of the current four-lane native
    /// batch. The two-constant speed-modifier path broadcasts that lane's
    /// sample; the other curves retain their per-particle streams.
    pub fn sample(&self, seed: u32, batch_seed: u32, age_percent: f32) -> VelocitySample {
        let t = normalized_age(age_percent);
        let linear_random = ParticleRandom::sample3(seed, 0xe0fb_d834);
        let angular_random = ParticleRandom::sample3(seed, 0xd129_3bac);
        let offset_random = ParticleRandom::sample3(seed, 0x348b_bbc3);
        let speed_seed = if matches!(self.speed, CurveSampler::TwoConstants { .. }) {
            batch_seed
        } else { seed };
        VelocitySample {
            linear: std::array::from_fn(|axis| self.linear[axis].evaluate(t, linear_random[axis])),
            orbital: OrbitalMotion {
                angular: std::array::from_fn(|axis| self.angular[axis].evaluate(t, angular_random[axis])),
                offset: std::array::from_fn(|axis| self.offset[axis].evaluate(t, offset_random[axis])),
                radial: self.radial.evaluate(t, ParticleRandom::sample(seed, 0xcab3_921d)),
            },
            speed_modifier: self.speed.evaluate(t, ParticleRandom::sample(speed_seed, 0xba82_1f34)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> VelocityOverLifetimeParams {
        let constant = MinMaxCurve::Constant;
        VelocityOverLifetimeParams {
            x: constant(0.0), y: constant(0.0), z: constant(0.0),
            in_world_space: false, speed_modifier: constant(1.0),
            orbital: std::array::from_fn(|_| MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 }),
            orbital_offset: std::array::from_fn(|_| constant(0.0)), radial: constant(0.0),
        }
    }

    #[test]
    fn orbital_sampling_uses_the_particle_seed_without_consuming_birth_rng() {
        let law = VelocityOverLifetime::from_params(&params());
        let first = law.sample(17, 17, 0.0);
        assert_eq!(first.orbital.angular, [0.8626621961593628, 0.911041259765625, 0.6222929358482361]);
        assert_eq!(first, law.sample(17, 19, 73.0));
        assert_ne!(first, law.sample(19, 17, 0.0));
    }

    #[test]
    fn two_constant_speed_modifier_preserves_native_batch_broadcast() {
        let mut params = params();
        params.speed_modifier = MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 };
        let law = VelocityOverLifetime::from_params(&params);
        let a = law.sample(17, 17, 0.0);
        let b = law.sample(19, 17, 0.0);
        assert_eq!(a.speed_modifier, 0.9958391189575195);
        assert_eq!(a.speed_modifier, b.speed_modifier);
        assert_ne!(a.orbital.angular, b.orbital.angular);
        assert_ne!(b.speed_modifier, law.sample(19, 19, 0.0).speed_modifier);
    }
}
