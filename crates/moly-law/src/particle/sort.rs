//! Per-particle renderer order, independent of simulation pool order.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParticleSort {
    #[default]
    None,
    Distance,
    OldestInFront,
    YoungestInFront,
}

#[derive(Clone, Copy, Debug)]
pub struct SortParticle {
    pub position: [f32; 3],
    pub age_percent: f32,
    pub inverse_lifetime: f32,
}

impl ParticleSort {
    pub fn from_source(mode: u32) -> Option<Self> {
        Some(match mode {
            0 => Self::None, 1 => Self::Distance,
            2 => Self::OldestInFront, 3 => Self::YoungestInFront,
            _ => return None,
        })
    }

    pub fn indices(self, particles: &[SortParticle], camera_in_simulation: [f32; 3]) -> Vec<usize> {
        let mut indices: Vec<_> = (0..particles.len()).collect();
        if self == Self::None { return indices; }
        let key = |index: usize| {
            let particle = &particles[index];
            let value = match self {
                Self::Distance => {
                    let delta: [f32; 3] = std::array::from_fn(|axis|
                        particle.position[axis] - camera_in_simulation[axis]);
                    -((delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2])
                }
                // The renderer sorts remaining lifetime, not normalized age.
                Self::OldestInFront | Self::YoungestInFront =>
                    (100.0 - particle.age_percent).max(0.0) / particle.inverse_lifetime,
                Self::None => unreachable!(),
            };
            // Source comparison uses unsigned float bits, then the original
            // particle index. Equal distances/lifetimes therefore have an
            // explicit order; a stable float sort would give a different one.
            (u64::from(value.to_bits()) << 32) | index as u64
        };
        if self == Self::YoungestInFront {
            indices.sort_unstable_by_key(|index| key(*index));
        } else {
            indices.sort_unstable_by_key(|index| std::cmp::Reverse(key(*index)));
        }
        indices
    }
}
