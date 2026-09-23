//! Persistent acceleration sampled from particle-local module streams.
use super::{curve::{CurveSampler, normalized_age}, random::ParticleRandom, schema::ForceParams};

#[derive(Clone, Debug)]
pub struct ForceOverLifetime {
    pub in_world_space: bool,
    axes: [CurveSampler; 3],
}

impl ForceOverLifetime {
    pub fn from_params(params: &ForceParams) -> Result<Self, String> {
        if params.randomize_per_frame {
            return Err("force per-frame random stream is not yet consumed".into());
        }
        // All three axes enter the same native curve dispatch. A generic axis
        // must also keep its siblings on the generic evaluation path.
        let baked = params.axes.iter().all(CurveSampler::can_bake);
        Ok(Self {
            in_world_space: params.in_world_space,
            axes: std::array::from_fn(|axis| CurveSampler::with_baking(&params.axes[axis], baked)),
        })
    }

    pub fn sample(&self, seed: u32, age_percent: f32) -> [f32; 3] {
        let random = ParticleRandom::sample3(seed, 0x1246_0f3b);
        let time = normalized_age(age_percent);
        std::array::from_fn(|axis| self.axes[axis].evaluate(time, random[axis]))
    }
}
