//! CustomDataModule writes persistent channels in the pre-simulation batch.
//! Each channel has its own salted particle stream and independent curve path.
use super::{curve::{normalized_age, CurveSampler}, random::ParticleRandom, schema::CustomDataParams};

#[derive(Clone, Debug)]
pub struct CustomData {
    streams: [Option<Vec<CurveSampler>>; 2],
}

impl CustomData {
    pub fn from_params(params: &CustomDataParams) -> Self {
        Self {
            streams: [params.custom1.as_ref(), params.custom2.as_ref()].map(|slot| slot.map(|slot| {
                assert_eq!(slot.component_count, slot.components.len());
                assert!(slot.component_count <= 4);
                slot.components.iter().map(CurveSampler::from_min_max_curve).collect()
            })),
        }
    }

    /// Disabled streams and components outside the authored count remain intact.
    /// The renderer reads this stored result; it does not evaluate a newer age.
    pub fn update(&self, seed: u32, age_percent: f32, values: &mut [[f32; 4]; 2]) {
        let time = normalized_age(age_percent);
        for (stream, curves) in self.streams.iter().enumerate() {
            let Some(curves) = curves else { continue; };
            for (channel, curve) in curves.iter().enumerate() {
                let salt = (0x73a7_f7bb_u32 | ((stream as u32) << 2)).wrapping_add(channel as u32);
                values[stream][channel] = curve.evaluate(time, ParticleRandom::sample(seed, salt));
            }
        }
    }
}
