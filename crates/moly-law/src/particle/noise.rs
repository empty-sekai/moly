//! Qualified constant, single-octave 3D/high particle Noise.
//!
//! Pure law: the consumer supplies owner seed, scroll and source simulation inputs.
//! Current JP 6.8.1 libunity SHA256 937c6d28...75badd9:
//! Update 0xef207c -> CalculateNoise 0xefb474 -> job 0xefbe30 -> Perlin3D 0xef137c.
//! Independent analytic equations: snow-noise-equation.py, 128 cases / 512 particles,
//! max error 7.51e-7 against the actual native Update (double reference arithmetic).
//! Rust f32 replay: 1,024 particle samples, max absolute error 3.58e-7.
//! Complete particle-system timing and source admission are separate obligations.
//!
//! The permutation is Ken Perlin's standard improved-noise algorithm table;
//! the gradients use the standard 12 integer cube-edge directions plus four
//! duplicates. Their duplicate order is Unity's observed algorithm variant; it
//! is not claimed to equal Perlin's canonical hash-to-gradient branch order.
//! All 512 permutation u32 and 48 gradient f32 values were checked against
//! current bytes at 0x1b3938 and 0x1b3878. These are small mathematical algorithm
//! constants, not game artwork, assets, or an application-specific lookup table.

pub use super::schema::{NoiseParams, NoiseQuality};
use super::MinMaxCurve;

#[derive(Clone, Copy, Debug)]
pub struct NoiseLaw {
    frequency: f32,
    strength: f32,
    scroll_speed: f32,
    position_amount: f32,
    damping: bool,
}

/// One instance per actual source particle system; survives individual births.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoiseState {
    pub scroll: f32,
}

impl NoiseLaw {
    pub fn from_params(p: &NoiseParams) -> Result<Self, &'static str> {
        if p.dimensions != 3 || p.quality != NoiseQuality::High {
            return Err("noise dimensions/quality outside qualified 3D/high subset");
        }
        if p.separate_axes || p.octaves != 1 || p.remap_enabled {
            return Err("noise separate axes, octaves, or remap outside qualified subset");
        }
        let constant = |x: &MinMaxCurve| match x {
            MinMaxCurve::Constant(v) if v.is_finite() => Ok(*v),
            _ => Err("noise nonconstant/nonfinite curve outside qualified subset"),
        };
        if constant(&p.rotation_amount)? != 0.0 || constant(&p.size_amount)? != 0.0 {
            return Err("noise rotation/size output outside qualified subset");
        }
        if !p.frequency.is_finite() || p.frequency <= 0.0 {
            return Err("noise nonpositive/nonfinite frequency outside qualified subset");
        }
        Ok(Self {
            frequency: p.frequency.max(f32::from_bits(0x3586_37bd)),
            strength: constant(&p.strength)?,
            scroll_speed: constant(&p.scroll_speed)?,
            position_amount: constant(&p.position_amount)?,
            damping: p.damping,
        })
    }

    /// Call once before a nonempty particle range, when native updateScroll is true.
    /// PreSimulation returns before Noise for an empty range; do not advance
    /// this accumulator on every empty frame.
    /// The source scroll curve's time is systemTime/duration; constant subset
    /// needs neither input. Never call once per particle or infer it from birth RNG.
    pub fn advance_scroll(&self, state: &mut NoiseState, dt: f32, update_scroll: bool) {
        if update_scroll {
            state.scroll += dt * self.scroll_speed;
        }
    }

    /// Contribution added to animated velocity (+0xc0/+0xe0/+0x100), before
    /// native motion integration. Position is in particle simulation space.
    /// owner_seed is the source system seed from ReadOnlyState +0x30, never the
    /// individual birth seed. Constant curves ignore individual particle seeds.
    pub fn sample(&self, state: NoiseState, position: [f32; 3], owner_seed: u32) -> [f32; 3] {
        let shift = owner_offset(owner_seed);
        let [x, y, z] = std::array::from_fn(|i| position[i] + shift[i] * 100.0);
        let a = perlin_xy([z, y, x + state.scroll], self.frequency);
        let b = perlin_xy([x + 100.0, z, y + state.scroll], self.frequency);
        let c = perlin_xy([y, x + 100.0, z + state.scroll], self.frequency);
        let scale = if self.damping {
            self.frequency.recip()
        } else {
            1.0
        } * self.strength;
        // Source derivative pairing is deliberate; neither three independent
        // scalar noise calls nor a random displacement has this curl law.
        [
            (c[0] - b[1]) * scale * self.position_amount,
            (a[0] - c[1]) * scale * self.position_amount,
            (b[0] - a[1]) * scale * self.position_amount,
        ]
    }
}

/// Dedicated source owner-seed expansion in CalculateNoise (not birth RNG).
fn owner_offset(seed: u32) -> [f32; 3] {
    let next = |s: u32| s.wrapping_mul(0x6c07_8965).wrapping_add(1);
    let a = next(seed);
    let b = next(a);
    let c = next(b);
    let mut x = seed ^ seed.wrapping_shl(11);
    x ^= x >> 8;
    x ^= c;
    let mut y = a ^ a.wrapping_shl(11);
    y ^= y >> 8;
    let q = x ^ (c >> 19);
    y ^= q;
    let r = y ^ (x >> 19);
    let mut z = b ^ b.wrapping_shl(11);
    z ^= z >> 8;
    z ^= r;
    // Mask placement is intentionally before this final XOR in native code.
    z = (z & 0x007f_ffff) ^ (y >> 19);
    [q & 0x007f_ffff, r & 0x007f_ffff, z].map(|v| v as f32 * f32::from_bits(0x3400_0001))
}

/// Improved Perlin's analytic x/y derivatives, including frequency chain rule.
/// Accumulation is scalar f32; it is mathematically equivalent to the SIMD
/// polynomial but does not promise bit-identical evaluation order.
fn perlin_xy(position: [f32; 3], frequency: f32) -> [f32; 2] {
    let scaled = position.map(|v| v * frequency);
    let floors = scaled.map(f32::floor);
    let cell = floors.map(|v| (v as i32 & 255) as usize);
    let p: [f32; 3] = std::array::from_fn(|i| scaled[i] - floors[i]);
    let fade = p.map(|t| t * t * t * (t * (t * 6.0 - 15.0) + 10.0));
    let derivative = p.map(|t| 30.0 * t * t * (t * (t - 2.0) + 1.0));
    let perm = |v: usize| PERMUTATION[v & 255] as usize;
    let mut result = [0.0; 2];
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let corner = [x, y, z];
                let hash = perm(perm(perm(cell[0] + x) + cell[1] + y) + cell[2] + z);
                let gradient = GRADIENTS[hash & 15];
                let dot = gradient[0] * (p[0] - x as f32)
                    + gradient[1] * (p[1] - y as f32)
                    + gradient[2] * (p[2] - z as f32);
                let w: [f32; 3] = std::array::from_fn(|i| {
                    if corner[i] != 0 {
                        fade[i]
                    } else {
                        1.0 - fade[i]
                    }
                });
                for axis in 0..2 {
                    let dw = if corner[axis] != 0 {
                        derivative[axis]
                    } else {
                        -derivative[axis]
                    };
                    result[axis] += (gradient[axis] * w[axis] + dot * dw)
                        * w[(axis + 1) % 3]
                        * w[(axis + 2) % 3];
                }
            }
        }
    }
    result.map(|v| v * frequency)
}

// Standard improved Perlin algorithm constants (256 entries; repeat via &255).
const PERMUTATION: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];
const GRADIENTS: [[f32; 3]; 16] = [
    [1., 1., 0.],
    [-1., 1., 0.],
    [1., -1., 0.],
    [-1., -1., 0.],
    [1., 0., 1.],
    [-1., 0., 1.],
    [1., 0., -1.],
    [-1., 0., -1.],
    [0., 1., 1.],
    [0., -1., 1.],
    [0., 1., -1.],
    [0., -1., -1.],
    [1., 1., 0.],
    [-1., 1., 0.],
    [0., -1., 1.],
    [0., -1., -1.],
];
