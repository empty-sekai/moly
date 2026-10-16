//! Transform rotation of an effect Animator whose controller layers each play one looping clip
//! additively on Euler rotation curves.
//!
//! The source path, per frame and per layer: `EvaluateState` advances the state's normalized
//! time in single precision; `PropagateStateMachineInfoToChildClips` sets the clip playable time
//! to the clip length times that normalized time, rounded to single precision before widening;
//! `ProcessAnimationClipInputPrepare` divides it back in double precision and rounds the
//! quotient upward to the next single when the nearest single fell below it; `ComputeClipTime`
//! takes the C `modff` fractional part; the streamed cubic curves are sampled; `ValuesFromClip`
//! turns the Euler angles into a quaternion with the engine's polynomial sine and cosine;
//! `DeltasFromClip` and `ValueArraySub` subtract the clip's start pose, normalized with the
//! ARM reciprocal square root estimate and two Newton steps; `ValueArrayAdd` applies the delta
//! to the bound default rotation; `SetGenericTransformPropertyValuesNoSync` normalizes with an
//! exact square root and division before storing the local rotation.

use crate::particle::shape::engine_sincos;

const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
/// `SetGenericTransformPropertyValuesNoSync` keeps a rotation only when its squared norm is
/// strictly above this value; otherwise it stores the identity.
const WRITE_THRESHOLD: f32 = f32::from_bits(0x0da2_4260);

/// Advance a state's normalized time by one `EvaluateState` call.
/// A zero duration counts as one; the division uses the signed speed times the delta time.
pub fn advance_state_time(time: f32, delta_time: f32, duration: f32, speed: f32) -> f32 {
    let duration = if duration == 0.0 { 1.0 } else { duration };
    (speed * delta_time) / duration + time
}

/// `ComputeStateSpeed` without speed parameters: the Animator speed times the absolute state speed.
pub fn state_speed(animator_speed: f32, state_speed: f32) -> f32 {
    animator_speed * (1.0 * state_speed.abs())
}

/// Duration of a state whose blend tree is one clip leaf: `EvaluateBlendTree` forces the root
/// weight to one, so the duration is the absolute clip span times the leaf duration.
pub fn single_leaf_state_duration(clip_start: f32, clip_stop: f32, leaf_duration: f32) -> f32 {
    0.0 + ((clip_stop - clip_start) * leaf_duration).abs() * 1.0
}

/// Clip playable time from the state's normalized time (single product, then widened).
pub fn clip_playable_time(clip_length: f32, state_time: f32) -> f64 {
    (clip_length * state_time) as f64
}

/// `ProcessAnimationClipInputPrepare`: the playable time over the clip length, as a single that
/// is stepped one unit toward the largest finite single when rounding fell below the quotient.
pub fn clip_input_time(time: f64, clip_length: f32) -> f32 {
    if clip_length == 0.0 {
        return 0.0;
    }
    let quotient = time / clip_length as f64;
    let single = quotient as f32;
    if single == f32::MAX || quotient <= single as f64 || single.is_nan() {
        return single;
    }
    if single == 0.0 {
        return f32::from_bits(1);
    }
    let bits = single.to_bits();
    f32::from_bits(if single > 0.0 { bits + 1 } else { bits - 1 })
}

/// C99 `modff` fractional part: exact, carries the sign of the argument when it is zero.
fn fractional(value: f32) -> f32 {
    if value.is_infinite() {
        return 0.0_f32.copysign(value);
    }
    let fraction = value - value.trunc();
    if fraction == 0.0 { 0.0_f32.copysign(value) } else { fraction }
}

/// `ComputeClipTime` for a looping clip played forward without a sample rate.
pub fn loop_clip_time(input_time: f32, start: f32, stop: f32, cycle_offset: f32) -> f32 {
    let duration = stop - start;
    let shifted = input_time + cycle_offset;
    let mut fraction = fractional(shifted);
    if shifted < 0.0 {
        fraction = fraction + 1.0;
    }
    duration * fraction + start
}

/// `ValuesFromClip` Euler conversion for rotation order Z-X-Y (binding custom type 4).
pub fn euler_zxy_degrees_to_quaternion(euler: [f32; 3]) -> [f32; 4] {
    const T0: [f32; 4] = [1.0, -1.0, 1.0, 1.0];
    const T1: [f32; 4] = [1.0, 1.0, -1.0, 1.0];
    let half = |degrees: f32| (degrees * DEG_TO_RAD) * 0.5;
    let (sx, cx) = engine_sincos(half(euler[0]));
    let (sy, cy) = engine_sincos(half(euler[1]));
    let (sz, cz) = engine_sincos(half(euler[2]));
    let v = [cz * sx, sx * sz, cx * sz, cx * cz];
    let rotated = [v[2], v[3], v[0], v[1]];
    std::array::from_fn(|i| T0[i] * (v[i] * cy) + (T1[i] * sy) * rotated[i])
}

const fn rsqrt_estimates() -> [u16; 256] {
    let mut table = [0_u16; 256];
    let mut i = 0;
    while i < 256 {
        let midpoint = (257_u64 + 2 * (i % 128) as u64) << (i / 128);
        let mut estimate = 256_u64;
        while midpoint * (2 * estimate + 1) * (2 * estimate + 1) < (1_u64 << 28) {
            estimate += 1;
        }
        table[i] = estimate as u16;
        i += 1;
    }
    table
}
const RSQRT_ESTIMATE: [u16; 256] = rsqrt_estimates();

/// ARM FRSQRTE on the whole single domain.
fn reciprocal_sqrt_estimate(value: f32) -> f32 {
    if value == 0.0 {
        return f32::INFINITY.copysign(value);
    }
    if value.is_nan() || value < 0.0 {
        return f32::NAN;
    }
    if value.is_infinite() {
        return 0.0;
    }
    let bits = value.to_bits();
    let mut exponent = ((bits >> 23) & 255) as i32;
    let mut fraction = bits & 0x7f_ffff;
    if exponent == 0 {
        while fraction & 0x40_0000 == 0 {
            fraction <<= 1;
            exponent -= 1;
        }
        fraction = (fraction << 1) & 0x7f_ffff;
    }
    let index = (fraction >> 16) as usize + if exponent & 1 == 0 { 128 } else { 0 };
    let estimate = RSQRT_ESTIMATE[index] as u32;
    f32::from_bits((((380 - exponent) / 2) as u32) << 23 | ((estimate - 256) << 15))
}

/// ARM FRSQRTS: (3 - a*b) / 2 with one rounding; infinity times zero gives 1.5.
fn reciprocal_sqrt_step(a: f32, b: f32) -> f32 {
    if (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite()) {
        return 1.5;
    }
    ((3.0_f64 - (a as f64) * (b as f64)) * 0.5) as f32
}

/// Quaternion normalization of `ValueArraySub` and `ValueArrayAdd`.
fn estimate_normalize(q: [f32; 4]) -> [f32; 4] {
    let n2 = (q[0] * q[0] + q[1] * q[1]) + (q[2] * q[2] + q[3] * q[3]);
    let estimate = reciprocal_sqrt_estimate(n2);
    let first = estimate * reciprocal_sqrt_step(n2 * estimate, estimate);
    let second = first * reciprocal_sqrt_step(n2 * first, first);
    let scale = if n2 == 0.0 { estimate } else { second };
    q.map(|v| v * scale)
}

/// `ValueArraySub`: the value relative to the reference pose, normalized.
pub fn additive_delta(reference: [f32; 4], value: [f32; 4]) -> [f32; 4] {
    let [cx, cy, cz, cw] = [-reference[0], -reference[1], -reference[2], reference[3]];
    let [bx, by, bz, bw] = value;
    estimate_normalize([
        -(((by * cz - bz * cy) - bx * cw) - bw * cx),
        -(((bz * cx - bx * cz) - by * cw) - bw * cy),
        -(((bx * cy - bz * cw) - bw * cz) - by * cx),
        ((bw * cw - bx * cx) - bz * cz) - by * cy,
    ])
}

/// `ValueArrayAdd` additive branch: the weighted delta, normalized, applied to the base.
/// No normalization after the product.
pub fn apply_additive(base: [f32; 4], delta: [f32; 4], weight: f32) -> [f32; 4] {
    let [dx, dy, dz, dw] = estimate_normalize([weight * delta[0], weight * delta[1], weight * delta[2], delta[3]]);
    let [bx, by, bz, bw] = base;
    [
        -(((bz * dy - by * dz) - bw * dx) - bx * dw),
        -(((bx * dz - bz * dx) - bw * dy) - by * dw),
        -(((by * dx - bw * dz) - bz * dw) - bx * dy),
        ((bw * dw - bx * dx) - bz * dz) - by * dy,
    ]
}

/// `SetGenericTransformPropertyValuesNoSync` rotation store.
pub fn transform_rotation(q: [f32; 4]) -> [f32; 4] {
    let n2 = (q[0] * q[0] + q[1] * q[1]) + (q[2] * q[2] + q[3] * q[3]);
    if n2 > WRITE_THRESHOLD {
        let length = n2.sqrt();
        q.map(|v| v / length)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// One additive layer driving one Transform's Euler rotation curves.
#[derive(Clone, Debug, PartialEq)]
pub struct AdditiveRotationLayer {
    pub start: f32,
    pub stop: f32,
    pub cycle_offset: f32,
    /// Start values of the clip's delta pose (the reference of an empty serialized reference pose).
    pub reference_euler: [f32; 3],
    /// The Transform's local rotation when the Animator bound it.
    pub default_rotation: [f32; 4],
}

impl AdditiveRotationLayer {
    pub fn state_duration(&self) -> f32 {
        single_leaf_state_duration(self.start, self.stop, 1.0)
    }

    /// Clip time for a state normalized time.
    pub fn clip_time(&self, state_time: f32) -> f32 {
        let length = self.stop - self.start;
        let input = clip_input_time(clip_playable_time(length, state_time), length);
        loop_clip_time(input, self.start, self.stop, self.cycle_offset)
    }

    /// The local rotation the Animator writes for sampled Euler angles (degrees).
    pub fn written_rotation(&self, euler: [f32; 3]) -> [f32; 4] {
        let value = euler_zxy_degrees_to_quaternion(euler);
        let reference = euler_zxy_degrees_to_quaternion(self.reference_euler);
        transform_rotation(apply_additive(self.default_rotation, additive_delta(reference, value), 1.0))
    }
}
