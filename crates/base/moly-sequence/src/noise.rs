//! Camera shake as a pure function of time.
//!
//! Each of the six channels (three of position, three of rotation) is a sum
//! of three sines whose phases and frequency ratios come from the seed, so a
//! shake looks the same on every run and at any seek position.

use bevy_math::Vec3;

use crate::doc::Shake;

/// Weights of the three octaves; they sum to 1 so a channel stays within
/// its amplitude.
const OCTAVES: [(f32, f32); 3] = [(1.0, 4.0 / 7.0), (2.13, 2.0 / 7.0), (4.37, 1.0 / 7.0)];

/// The positional (metres, camera-local) and rotational (degrees: yaw,
/// pitch, roll) offsets of a shake `seconds` into it.
pub(crate) fn offsets(shake: &Shake, seconds: f32) -> (Vec3, Vec3) {
    let channel = |index: u32| -> f32 {
        let mut sum = 0.0;
        for (octave, (ratio, weight)) in OCTAVES.iter().enumerate() {
            let bits = mix(shake.seed, index * 8 + octave as u32);
            let phase = unit(bits) * std::f32::consts::TAU;
            // Up to 15% detune keeps the channels from moving in step.
            let detune = 1.0 + 0.3 * (unit(bits.rotate_left(16)) - 0.5);
            let frequency = shake.frequency * ratio * detune;
            sum += weight * (std::f32::consts::TAU * frequency * seconds + phase).sin();
        }
        sum
    };
    let position = Vec3::new(channel(0), channel(1), channel(2)) * shake.amplitude;
    let rotation = Vec3::new(channel(3), channel(4), channel(5)) * shake.rotation;
    (position, rotation)
}

/// A well-mixed 32-bit value from a seed and a stream index.
fn mix(seed: u32, stream: u32) -> u32 {
    let mut x = (u64::from(seed) << 32 | u64::from(stream)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (x ^ (x >> 31)) as u32
}

/// A value in `[0, 1)` from the top 24 bits.
fn unit(bits: u32) -> f32 {
    (bits >> 8) as f32 / (1u32 << 24) as f32
}
