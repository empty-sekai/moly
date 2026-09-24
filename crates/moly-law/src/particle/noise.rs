//! Qualified single-octave 3D/high particle Noise with a constant or curve strength.
//!
//! Pure law: the consumer supplies owner seed, scroll and source simulation inputs.
//! Transcribed from the current JP 6.8.1 libunity, along the call chain
//! NoiseModule::Update -> CalculateNoise -> NoiseModule::CalculateNoiseJob -> Perlin3D.
//! Perlin3D follows the native straight-line dataflow operation by operation;
//! the curl pairing and scales follow the job body.
//! Complete particle-system timing and source admission are separate obligations.
//!
//! The permutation is Ken Perlin's standard improved-noise algorithm table;
//! the gradients use the standard 12 integer cube-edge directions plus four
//! duplicates. Their duplicate order is Unity's observed algorithm variant; it
//! is not claimed to equal Perlin's canonical hash-to-gradient branch order.
//! All 512 permutation u32 and 48 gradient f32 values were checked against
//! the two constant tables of the current library. These are small mathematical algorithm
//! constants, not game artwork, assets, or an application-specific lookup table.

pub use super::schema::{NoiseParams, NoiseQuality};
use super::curve::{curve_time_fmax, CurveSampler, CurveTime};
use super::MinMaxCurve;

#[derive(Clone, Debug)]
pub struct NoiseLaw {
    frequency: f32,
    /// A finite constant or a mode 1 curve. `CalculateNoiseJob` evaluates the
    /// strength per particle through `EvaluateThreaded` at the particle's age
    /// (see [`NoiseLaw::sample`]); the two-constant and two-curve modes read
    /// the job's own per-particle random stream, which is not compared, and
    /// stay refused.
    strength: CurveSampler,
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
        let strength = match &p.strength {
            MinMaxCurve::Curve { .. } => CurveSampler::new(&p.strength, CurveTime::Normalized)?,
            other => CurveSampler::Constant(constant(other)?),
        };
        Ok(Self {
            frequency: p.frequency.max(f32::from_bits(0x3586_37bd)),
            strength,
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

    /// Contribution added to the particles' animated velocity arrays (x, y
    /// and z), before native motion integration. Position is in particle
    /// simulation space. owner_seed is the source system's random seed from
    /// its read-only state, never the individual birth seed. Constant curves
    /// ignore individual particle seeds.
    /// `age_percent` is the particle's age before this frame's lifetime
    /// advance (the job reads the age array before SimulateParticles writes
    /// it; a newborn reads 0). A strength curve is evaluated at
    /// `fmax(age_percent * 0.01, +0)`; a constant ignores it.
    pub fn sample(&self, state: NoiseState, position: [f32; 3], owner_seed: u32,
        age_percent: f32) -> [f32; 3] {
        let shift = owner_offset(owner_seed);
        let [x, y, z] = std::array::from_fn(|i| position[i] + shift[i] * 100.0);
        let a = perlin_xy([z, y, x + state.scroll], self.frequency);
        let b = perlin_xy([x + 100.0, z, y + state.scroll], self.frequency);
        let c = perlin_xy([y, x + 100.0, z + state.scroll], self.frequency);
        // CalculateNoiseJob: damping uses the refined ARM
        // reciprocal estimate of the clamped frequency, not host division.
        let scale = if self.damping {
            super::velocity::orbital_reciprocal(self.frequency)
        } else {
            1.0
        } * self.strength.evaluate(curve_time_fmax(age_percent), 0.0);
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
/// Transcribed from the current straight-line Perlin3D body (no fused
/// ops): trilinear coefficient form, every f32 operation in native
/// order. Only the integer corner hash is written semantically. A per-corner
/// sum is mathematically equal but rounds differently.
fn perlin_xy(position: [f32; 3], frequency: f32) -> [f32; 2] {
    let scaled = position.map(|v| frequency * v);
    // fcvtzs/scvtf/fcmgt floor: truncation minus one where it exceeds the value.
    let floors = scaled.map(|v| {
        let truncated = (v as i32) as f32;
        truncated - if truncated > v { 1.0 } else { 0.0 }
    });
    let cell = floors.map(|v| (v as i32 & 255) as usize);
    let [x, y, z]: [f32; 3] = std::array::from_fn(|i| scaled[i] - floors[i]);
    let fade = |t: f32| (t * (t * t)) * (t * (t * 6.0 + -15.0) + 10.0);
    let slope = |t: f32| (t * (t * 30.0)) * (t * (t + -2.0) + 1.0);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let perm = |i: usize| PERMUTATION[i & 255] as usize;
    let g = |i: usize, j: usize, k: usize| {
        GRADIENTS[perm(perm(perm(cell[0] + i) + cell[1] + j) + cell[2] + k) & 15]
    };
    let (g000, g100, g010, g110) = (g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0));
    let (g001, g101, g011, g111) = (g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1));
    let (x1, y1, z1) = (x + -1.0, y + -1.0, z + -1.0);
    let dot = |g: [f32; 3], x: f32, y: f32, z: f32| x * g[0] + (y * g[1] + z * g[2]);
    let n000 = dot(g000, x, y, z);
    let n100 = dot(g100, x1, y, z);
    let n010 = dot(g010, x, y1, z);
    let n110 = dot(g110, x1, y1, z);
    let n001 = dot(g001, x, y, z1);
    let n101 = dot(g101, x1, y, z1);
    let n011 = dot(g011, x, y1, z1);
    let n111 = dot(g111, x1, y1, z1);
    // Interpolated gradient component: the direct term of each derivative.
    let direct = |c: usize| {
        let (a000, a100, a010, a110) = (g000[c], g100[c], g010[c], g110[c]);
        let (a001, a101, a011, a111) = (g001[c], g101[c], g011[c], g111[c]);
        let near = (a000 + u * (a100 - a000))
            + v * ((a010 - a000) + u * (a000 + ((a110 - a010) - a100)));
        let far_u = (a001 - a000) + u * (a000 + ((a101 - a001) - a100));
        let far_uv = (a000 + ((a011 - a001) - a010))
            + u * ((a100 + (a010 + ((a001 + ((a111 - a011) - a101)) - a110))) - a000);
        near + w * (far_u + v * far_uv)
    };
    let k4 = n000 + ((n110 - n010) - n100);
    let k7 = (n100 + (n010 + ((n001 + ((n111 - n011) - n101)) - n110))) - n000;
    let x_chain = ((n100 - n000) + v * k4) + w * ((n000 + ((n101 - n001) - n100)) + v * k7);
    let y_chain = ((n010 - n000) + u * k4) + w * ((n000 + ((n011 - n001) - n010)) + u * k7);
    [
        frequency * (direct(0) + slope(x) * x_chain),
        frequency * (direct(1) + slope(y) * y_chain),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    const SOURCE: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

    fn read(key: &str) -> Value {
        let path = std::env::var_os(key).expect(key);
        parse(&std::fs::read(path).unwrap()).unwrap()
    }
    fn word(v: &Value) -> u32 {
        v.as_f64().unwrap() as u32
    }
    fn number(v: &Value) -> f32 {
        v.as_f64().unwrap() as f32
    }

    #[test]
    #[ignore = "MOLY_CURRENT_PERLIN3D must identify the current JP native Perlin3D bit receipt"]
    fn current_native_perlin3d_matches_bits() {
        let receipt = read("MOLY_CURRENT_PERLIN3D");
        assert_eq!(receipt.get("sourceSha256").unwrap().as_str(), Some(SOURCE));
        let groups = receipt.get("groups").unwrap().as_array().unwrap();
        let mut lanes = 0;
        for (index, group) in groups.iter().enumerate() {
            let frequency = f32::from_bits(word(group.get("frequencyBits").unwrap()));
            let positions = group.get("positionBits").unwrap().as_array().unwrap();
            let outputs = group.get("outputBits").unwrap().as_array().unwrap();
            for (lane, (position, output)) in positions.iter().zip(outputs).enumerate() {
                let position = position.as_array().unwrap();
                let position = [0, 1, 2].map(|i| f32::from_bits(word(&position[i])));
                let actual = perlin_xy(position, frequency);
                let output = output.as_array().unwrap();
                for axis in 0..2 {
                    assert_eq!(actual[axis].to_bits(), word(&output[axis]),
                        "group {index} lane {lane} axis {axis}");
                }
                lanes += 1;
            }
        }
        assert_eq!(lanes, 4096);
    }

    fn key(words: &Value) -> crate::particle::CurveKey {
        let w: Vec<u32> = words.as_array().unwrap().iter().map(word).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        crate::particle::CurveKey {
            time: f(0), value: f(1), in_slope: f(2), out_slope: f(3),
            weighted_mode: w[4] as u8, in_weight: f(5), out_weight: f(6),
        }
    }

    // The receipt installs its curves with clamp wraps on both sides.
    fn curve_params(keys: Vec<crate::particle::CurveKey>, multiplier: f32, frequency: f32,
        damping: bool, position: f32) -> NoiseParams {
        let constant = MinMaxCurve::Constant;
        NoiseParams {
            separate_axes: false,
            strength: MinMaxCurve::Curve { multiplier, max: crate::particle::Curve {
                multiplier: 1.0, keys, pre_wrap: Some(2), post_wrap: Some(2) } },
            strength_y: constant(1.0),
            strength_z: constant(1.0),
            frequency,
            damping,
            octaves: 1,
            octave_multiplier: 0.5,
            octave_scale: 2.0,
            quality: NoiseQuality::High,
            dimensions: 3,
            scroll_speed: constant(0.0),
            remap_enabled: false,
            remap: constant(1.0),
            remap_y: constant(1.0),
            remap_z: constant(1.0),
            position_amount: constant(position),
            rotation_amount: constant(0.0),
            size_amount: constant(0.0),
        }
    }

    // NaN lanes compare as NaN; every other lane compares bit for bit.
    fn same(actual: f32, native: u32) -> bool {
        let native_value = f32::from_bits(native);
        (actual.is_nan() && native_value.is_nan()) || actual.to_bits() == native
    }

    fn replay_block(block: &Value, keys: &Value) -> usize {
        let keys: Vec<_> = keys.as_array().unwrap().iter().map(key).collect();
        let bits = |name: &str| f32::from_bits(word(block.get(name).unwrap()));
        let damping = block.get("damping").unwrap().as_bool().unwrap();
        let params = curve_params(keys, bits("scalar"), bits("frequency"), damping, bits("position"));
        let law = NoiseLaw::from_params(&params).expect("receipt curve admitted");
        let mut lanes = 0;
        for call in block.get("calls").unwrap().as_array().unwrap() {
            let owner = word(call.get("owner").unwrap());
            let state = NoiseState { scroll: f32::from_bits(word(call.get("scroll").unwrap())) };
            for row in call.get("particles").unwrap().as_array().unwrap() {
                let row: Vec<u32> = row.as_array().unwrap().iter().map(word).collect();
                let position = [1, 2, 3].map(|i| f32::from_bits(row[i]));
                let sample = law.sample(state, position, owner, f32::from_bits(row[0]));
                for axis in 0..3 {
                    // The job adds onto a zeroed animated lane.
                    assert!(same(sample[axis] + 0.0, row[5 + axis]),
                        "age {:#x} axis {axis}: {:#x} vs native {:#x}", row[0],
                        (sample[axis] + 0.0).to_bits(), row[5 + axis]);
                }
                lanes += 1;
            }
        }
        lanes
    }

    // Unchanged NoiseModule::Update on the source strength curve and on
    // derived curves of the same unoptimized path, every particle's bits.
    #[test]
    #[ignore = "MOLY_NOISE_STRENGTH_CURVE_NATIVE must identify the current JP Noise strength-curve receipt"]
    fn strength_curve_noise_matches_native_update() {
        let receipt = read("MOLY_NOISE_STRENGTH_CURVE_NATIVE");
        assert_eq!(receipt.get("sourceSha256").unwrap().as_str(), Some(SOURCE));
        let source = receipt.get("sourceCases").unwrap();
        assert_eq!(replay_block(source, source.get("keys").unwrap()), 2944);
        let mut derived = 0;
        for block in receipt.get("derivedCases").unwrap().as_array().unwrap() {
            derived += replay_block(block, block.get("keys").unwrap());
        }
        assert_eq!(derived, 15040);
    }

    // The asset reader's isOptimizedCurve bit, native BuildCurves per curve.
    #[test]
    #[ignore = "MOLY_NOISE_STRENGTH_CURVE_NATIVE must identify the current JP Noise strength-curve receipt"]
    fn optimized_curve_decision_matches_native_build() {
        let receipt = read("MOLY_NOISE_STRENGTH_CURVE_NATIVE");
        let cases = receipt.get("decisionCases").unwrap().as_array().unwrap();
        let mut optimized = 0;
        for (index, case) in cases.iter().enumerate() {
            let case = case.as_array().unwrap();
            let keys: Vec<_> = case[0].as_array().unwrap().iter().map(key).collect();
            let native = word(&case[1]) != 0;
            assert_eq!(super::super::curve::engine_optimizes_curve(&keys), native, "case {index}");
            optimized += usize::from(native);
        }
        assert_eq!((cases.len(), optimized), (1500, 411));
    }

    #[test]
    #[ignore = "MOLY_SNOW_NOISE_NATIVE_REPLAY must identify the current JP snow Noise job receipt"]
    fn current_snow_noise_job_matches_bits() {
        let receipt = read("MOLY_SNOW_NOISE_NATIVE_REPLAY");
        assert_eq!(receipt.get("sourceSha256").unwrap().as_str(), Some(SOURCE));
        // Serialized snow_pt_01 subset: high 3D, one octave, damping, no remap.
        let law = NoiseLaw {
            frequency: 0.5,
            strength: CurveSampler::Constant(f32::from_bits(0x3e4c_cccd)),
            scroll_speed: 1.0,
            position_amount: 1.0,
            damping: true,
        };
        let cases = receipt.get("cases").unwrap().as_array().unwrap();
        assert_eq!(cases.len(), 32);
        for (index, case) in cases.iter().enumerate() {
            let input = case.get("input").unwrap();
            let output = case.get("output").unwrap();
            let owner = input.get("owner_seed").unwrap().as_f64().unwrap() as u32;
            let mut state = NoiseState { scroll: number(input.get("scroll").unwrap()) };
            law.advance_scroll(&mut state, number(input.get("dt").unwrap()), true);
            assert_eq!(state.scroll.to_bits(), number(output.get("scroll").unwrap()).to_bits());
            let positions = input.get("positions").unwrap().as_array().unwrap();
            let animated = output.get("animated").unwrap().as_array().unwrap();
            for (lane, (position, expected)) in positions.iter().zip(animated).enumerate() {
                let position = position.as_array().unwrap();
                let sample = law.sample(state, [0, 1, 2].map(|i| number(&position[i])), owner, 0.0);
                let expected = expected.as_array().unwrap();
                for axis in 0..3 {
                    // Job adds positionAmount * noise onto the zeroed animated lane.
                    assert_eq!((sample[axis] + 0.0).to_bits(), number(&expected[axis]).to_bits(),
                        "case {index} lane {lane} axis {axis}");
                }
            }
        }
    }
}
