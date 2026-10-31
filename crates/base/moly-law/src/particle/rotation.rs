//! rotationOverLifetime: the per-frame angular velocity of a particle spinning
//! about an axis.
//!
//! Tier: `bit-exact`: results, the order of operations and the constants match
//! the reference bit for bit.
//!
//! Input: the particle seed, the start module's randomize-direction value, the
//! particle's age percent from before this frame's advance (this module runs
//! before the advance), and the three axis curves in their modes. Output: one
//! angular velocity per axis, in radians per second, and the rotation after
//! one integration step. The six dispatch paths of the module's mode word (a
//! u16) are:
//!
//! | mode | path | value | flip factor |
//! |---|---|---|---|
//! | 0 constant | generic | v = the scalar | constant hash (affine) |
//! | 3 two constants | generic | v = (max-min)*r_lerp + min, the lerp **not clamped** | sign hash |
//! | 1 curve | generic | v = evaluate(curve, t) * multiplier | sign hash |
//! | 2 two curves | generic | lo and hi each times the multiplier, v = (hi-lo)*r_lerp + lo | sign hash |
//! | 1 curve | baked | v = the polynomial, **no multiplier** (folded into the coefficients) | constant hash (affine) |
//! | 2 two curves | baked | a polynomial per structure, v = (hi-lo)*r_lerp + lo | sign hash |
//!
//! `t = max(agePercent * 0.01, 0)`.
//! The flip is `if randomizeDirection < r { v } else { -v }`. The game's
//! assets keep that value at 0 by construction (spread comes from the
//! two-constant and two-curve lerp draws), where the call passes 0.0; the law
//! still reads it from the parameter.
//!
//! Random streams: 0 draws per frame. Every factor is a pure hash of the
//! per-particle u32 seed, independent of `t`, so a particle keeps the same
//! factors all its life; the caller draws the seed once, at birth.
//!
//! Three differences from the documented semantics in `value`: the lerp is not
//! clamped (`value` clamps to [0,1]); keys at the same time give v0 (`value`
//! gives `k1.value`); and the composition is `h01*v1 + ((h00*v0 + h10*m0) +
//! h11*m1)` where `value` writes `a*v0 + b*m0 + c*v1 + d*m1`. This module
//! therefore does not reuse that evaluation.

use crate::particle::value::MinMaxCurve;
pub use super::curve::{BakedCurve, CurveSampler as AngularVelocityLaw, CurveTime, normalized_age};
pub(crate) use super::random::hash_mix;

// ---- Hash kernels (constants pinned by bit) ----

/// The multiplier M.
const HASH_M: u32 = 0x6ab5_1b9d;
/// The lerp factor's added term T.
const HASH_T_LERP: u32 = 0x6aed_452e;
/// The lerp factor's added term C.
const HASH_C_LERP: u32 = 0x00a0_1275;
/// The sign factor's added term T.
const HASH_T_SIGN: u32 = 0xff2b_b1a4;
/// The sign factor's added term C.
const HASH_C_SIGN: u32 = 0x0bc7_08d3;
/// The lerp factor: `t = seed + T_lerp` and `m = seed * M + C_lerp`, the two
/// arguments `hash_mix` takes.
fn hash_lerp_raw(seed: u32) -> f32 {
    hash_mix(
        seed.wrapping_add(HASH_T_LERP),
        seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_LERP),
    )
}

/// The sign factor, built like the lerp one: `t = seed + T_sign`,
/// `m = seed * M + C_sign`.
fn hash_sign_raw(seed: u32) -> f32 {
    hash_mix(
        seed.wrapping_add(HASH_T_SIGN),
        seed.wrapping_mul(HASH_M).wrapping_add(HASH_C_SIGN),
    )
}

// ---- Curve evaluation (bit-exact) ----

impl AngularVelocityLaw {
    /// Sample the prepared curve with the rotation module's independent lerp
    /// and sign streams. Expanded and seed-shifted native hash forms are
    /// algebraically identical under wrapping u32 arithmetic.
    pub fn sample(&self, seed: u32, randomize_direction: f32, age_percent: f32) -> f32 {
        let value = self.evaluate(normalized_age(age_percent), hash_lerp_raw(seed));
        if randomize_direction < hash_sign_raw(seed) { value } else { -value }
    }
}

/// The rotationOverLifetime module: one angular-velocity law per axis. With
/// separateAxes off only z is sampled (the billboard spin axis); with it on
/// each of the three axes samples on its own.
#[derive(Clone, Debug)]
pub struct RotationOverLifetime {
    pub separate_axes: bool,
    pub x: Option<AngularVelocityLaw>,
    pub y: Option<AngularVelocityLaw>,
    pub z: AngularVelocityLaw,
}

impl RotationOverLifetime {
    /// Builds from the axis curves. With `separate_axes` set and x or y
    /// absent (the key missing from the export) construction refuses, by
    /// name: a missing axis is not a zero, it is the wrong data shape.
    pub fn from_parts(
        separate_axes: bool,
        x: Option<&MinMaxCurve>,
        y: Option<&MinMaxCurve>,
        z: &MinMaxCurve,
    ) -> Result<Self, String> {
        if separate_axes && (x.is_none() || y.is_none()) {
            return Err(format!(
                "rotationOverLifetime: separateAxes is on but an x/y axis key is missing (x={} y={})",
                x.is_some(),
                y.is_some()
            ));
        }
        let axis = |curve: &MinMaxCurve| AngularVelocityLaw::new(curve, CurveTime::Normalized)
            .map_err(|reason| format!("rotationOverLifetime: {reason}"));
        Ok(Self {
            separate_axes,
            x: x.map(axis).transpose()?,
            y: y.map(axis).transpose()?,
            z: axis(z)?,
        })
    }

    /// The frame's angular velocity per axis, in radians per second. An axis
    /// that is not enabled gives 0 (the constant 0).
    pub fn angular_velocity(
        &self,
        seed: u32,
        randomize_direction: f32,
        age_percent: f32,
    ) -> [f32; 3] {
        if self.separate_axes {
            [
                self.x
                    .as_ref()
                    .map_or(0.0, |l| l.sample(seed, randomize_direction, age_percent)),
                self.y
                    .as_ref()
                    .map_or(0.0, |l| l.sample(seed, randomize_direction, age_percent)),
                self.z.sample(seed, randomize_direction, age_percent),
            ]
        } else {
            [0.0, 0.0, self.z.sample(seed, randomize_direction, age_percent)]
        }
    }

    /// Advances the rotation: `rotation[axis] += angular_velocity[axis] * dt`.
    /// With `rotation3d` all three axes integrate; without it only z does,
    /// and x and y keep their angular velocity at zero. A non-finite `dt` is
    /// refused.
    pub fn advance_rotation(
        &self,
        rotation: &mut [f32; 3],
        seed: u32,
        randomize_direction: f32,
        age_percent: f32,
        rotation3d: bool,
        dt: f32,
    ) -> Result<(), &'static str> {
        if !dt.is_finite() {
            return Err("rotationOverLifetime: non-finite dt");
        }
        let w = self.angular_velocity(seed, randomize_direction, age_percent);
        if rotation3d {
            for axis in 0..3 {
                rotation[axis] += w[axis] * dt;
            }
        } else {
            rotation[2] += w[2] * dt;
        }
        Ok(())
    }
}
