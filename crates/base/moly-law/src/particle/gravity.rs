//! InitialModule gravity uses the system's normalized clock, independently of
//! particle age. Its random factor is stable per particle (salt 0xe2b7c3c3).
use super::{curve::{CurveSampler, CurveTime}, random::ParticleRandom, MinMaxCurve};

#[derive(Clone, Debug)]
pub struct Gravity {
    curve: CurveSampler,
    random: bool,
}
impl Gravity {
    /// The modifier goes through `Evaluate(MinMaxCurve)`, which follows the
    /// reader's isOptimizedCurve bit like every other MinMaxCurve entry. Its
    /// clock (system time over duration) has no upper bound, so a repeat wrap
    /// past the last key is refused.
    pub fn new(curve: &MinMaxCurve) -> Result<Self, &'static str> {
        Ok(Self { curve: CurveSampler::new(curve, CurveTime::Unbounded)?,
            random: matches!(curve, MinMaxCurve::TwoConstants {..} | MinMaxCurve::TwoCurves {..}) })
    }
    /// Delta in world coordinates, before the inverse owner transform for
    /// local simulation. Preserve the native multiplication grouping.
    pub fn delta(&self, seed: u32, system_time: f32, duration: f32, dt: f32, gravity: [f32;3]) -> [f32;3] {
        let value=self.curve.evaluate(system_time / duration, ParticleRandom::sample(seed,0xe2b7_c3c3));
        if self.random { gravity.map(|g| (dt * g) * value) }
        else { gravity.map(|g| (value * dt) * g) }
    }
}
