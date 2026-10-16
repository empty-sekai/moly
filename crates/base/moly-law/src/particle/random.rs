//! Particle-local random streams. A module's salt selects an independent stream
//! from the particle seed; sampling an over-lifetime curve never advances the
//! emitter's birth stream.
const INITIAL_MULTIPLIER: u32 = 0x6c07_8965;
const UNIT: f32 = f32::from_bits(0x3400_0001);

/// The low 23 integer bits are converted numerically, not reinterpreted as an
/// IEEE-754 payload. The source scale can round the largest sample to 1.0.
fn unit(bits: u32) -> f32 {
    ((bits & 0x007f_ffff) as f32) * UNIT
}

/// Algebraic form of the first xorshift draw, shared with scalar module kernels.
pub(crate) fn hash_mix(t: u32, m: u32) -> f32 {
    let h = t ^ (t << 11);
    unit(h ^ (h >> 8) ^ m ^ (m >> 19))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleRandom {
    state: [u32; 4],
}

impl ParticleRandom {
    pub fn from_seed(seed: u32) -> Self {
        let mut state = [seed; 4];
        for index in 1..4 {
            state[index] = state[index - 1].wrapping_mul(INITIAL_MULTIPLIER).wrapping_add(1);
        }
        Self { state }
    }

    pub fn next_u32(&mut self) -> u32 {
        let t = self.state[0] ^ (self.state[0] << 11);
        let w = self.state[3];
        let next = w ^ (w >> 19) ^ t ^ (t >> 8);
        self.state = [self.state[1], self.state[2], w, next];
        next
    }

    pub fn next_f32(&mut self) -> f32 { unit(self.next_u32()) }

    pub fn sample(seed: u32, salt: u32) -> f32 {
        Self::from_seed(seed.wrapping_add(salt)).next_f32()
    }

    pub fn sample3(seed: u32, salt: u32) -> [f32; 3] {
        let mut random = Self::from_seed(seed.wrapping_add(salt));
        std::array::from_fn(|_| random.next_f32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_samples_are_not_subnormal_float_payloads() {
        assert_eq!(unit(0), 0.0);
        assert_eq!(unit(0x007f_ffff), 1.0);
        assert_eq!(unit(0x0040_0000), f32::from_bits(0x3f00_0001));
    }

    #[test]
    fn scalar_hash_equals_initialized_stream() {
        for seed in [0_u32, 1, 17, 127, 0x8000_0000, u32::MAX] {
            let last = seed.wrapping_mul(0x6ab5_1b9d).wrapping_add(0x714a_cb3f);
            assert_eq!(hash_mix(seed, last), ParticleRandom::from_seed(seed).next_f32());
        }
    }

    #[test]
    fn orbital_axes_use_successive_draws_not_one_shared_factor() {
        let expected = [0.8626621961593628_f32, 0.911041259765625, 0.6222929358482361];
        assert_eq!(ParticleRandom::sample3(17, 0xd129_3bac), expected);
        let offset = [0.09402777254581451_f32, 0.13847829401493073, 0.24201825261116028];
        assert_eq!(ParticleRandom::sample3(17, 0x348b_bbc3), offset);
        assert_eq!(ParticleRandom::sample(17, 0xcab3_921d), 0.3166716396808624);
    }
}
