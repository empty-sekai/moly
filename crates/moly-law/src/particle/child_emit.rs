//! What a sub-emitter target does with one parent command before its own
//! birth modules run: the start matrix the births are placed and oriented
//! with, the command velocity in the target's space, the catch-up step, and
//! the gravity delta its Initial update adds (Initial, Shape, StartVelocity
//! and the lifetime modules are the caller's, with the target's own laws).
//!
//! All arithmetic is f32, one rounding per operation, in the engine's
//! association. Coordinates are source (Unity) axes.

use super::owner::OwnerMatrices;

/// A direction or axis shorter than this has no direction (the zero vector,
/// or the look-rotation failure that selects the identity).
pub const DIRECTION_EPSILON: f32 = f32::from_bits(0x3727_c5ac);
/// The look rotation's orthonormality tolerance on the rebuilt up axis.
const ORTHONORMAL_TOLERANCE: f32 = f32::from_bits(0x3586_37bd);
/// Catch-up runs only for a step above this.
const MIN_CATCH_UP_STEP: f32 = f32::from_bits(0x38d1_b717);
/// The frame dt the target reads when the world is not playing.
const NOT_PLAYING_DT: f32 = f32::from_bits(0x3ca3_d70a);
/// The raised catch-up step bound for a catch-up above 5 s (above 10 s: 1).
const RAISED_STEP: f32 = f32::from_bits(0x3e4c_cccd);
/// Gravity below this squared length adds nothing.
const GRAVITY_EPSILON: f32 = f32::from_bits(0x322b_cc76);

/// The target state words the command reads: its local-to-world, its
/// world-to-local, its own rotation 3x3, the emitter scale and the shape
/// scale, all column-major, as the owner update left them in the frame the
/// command is issued.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChildOwner {
    pub local_to_world: [f32; 16],
    pub world_to_local: [f32; 16],
    pub local_rotation: [f32; 9],
    pub emitter_scale: [f32; 3],
    pub shape_scale: [f32; 3],
}

impl ChildOwner {
    /// The Local scaling owner stores the shape scale (1, 1, 1).
    pub fn local_scaling(owner: &OwnerMatrices) -> Self {
        Self {
            local_to_world: owner.local_to_world,
            world_to_local: owner.world_to_local,
            local_rotation: owner.local_rotation,
            emitter_scale: owner.emitter_scale,
            shape_scale: [1.0; 3],
        }
    }
}

/// The command's start matrix and the velocity the births are placed back
/// along (the command velocity, in the target's space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartFrame {
    /// Column-major; rows 0..2 of every column and (0, 0, 0, 1) as the last
    /// row before the emitter-scale multiply, which also scales the fourth
    /// row of the first three columns.
    pub matrix: [f32; 16],
    pub emitter_velocity: [f32; 3],
    /// Whether the look rotation succeeded (the identity is used otherwise).
    pub look_rotation: bool,
}

/// The start matrix of one command. The command velocity's direction (the
/// zero vector when it is shorter than the direction epsilon, NaN included)
/// is looked along with an up vector blended between the owner's up and
/// forward by the absolute cosine of the direction to the owner's forward;
/// the rotation is then multiplied by the target's own rotation and the
/// command position becomes the translation. For a target that is not
/// World-space the matrix is taken through the world-to-local (3x4 product,
/// its translation added last) and the velocity through its linear part.
/// The emitter scale scales the first three columns last.
pub fn start_frame(owner: &ChildOwner, world_space: bool, position: [f32; 3], velocity: [f32; 3])
    -> StartFrame {
    let [vx, vy, vz] = velocity;
    let m = &owner.local_to_world;
    let magnitude = ((vx * vx + vy * vy) + vz * vz).sqrt();
    let forward = columns_times(m, [0.0, 0.0, 1.0]);
    let up = columns_times(m, [0.0, 1.0, 0.0]);
    let direction = if magnitude > DIRECTION_EPSILON {
        [vx / magnitude, vy / magnitude, vz / magnitude]
    } else {
        [0.0; 3]
    };
    let dot = direction[2] * forward[2] + (direction[1] * forward[1] + direction[0] * forward[0]);
    // Only a negative cosine is negated, so -0.0 stays -0.0.
    let a = if dot < 0.0 { -dot } else { dot };
    let one_a = 1.0 - a;
    let blended: [f32; 3] = std::array::from_fn(|k| up[k] * a + forward[k] * one_a);
    let (rotation, look_rotation) = match look_rotation_to_matrix(direction, blended) {
        Some(r) => (r, true),
        None => ([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], false),
    };
    let r = mat3_mul(&rotation, &owner.local_rotation);
    let mut matrix = [
        r[0], r[1], r[2], 0.0, r[3], r[4], r[5], 0.0, r[6], r[7], r[8], 0.0,
        position[0], position[1], position[2], 1.0,
    ];
    let emitter_velocity = if world_space {
        velocity
    } else {
        let i = &owner.world_to_local;
        matrix = mul3x4(i, &matrix);
        let [x, y, _] = columns_times(i, velocity);
        [x, y, (vx * i[2] + vy * i[6]) + vz * i[10]]
    };
    for c in 0..3 {
        for row in 0..4 {
            matrix[4 * c + row] = owner.emitter_scale[c] * matrix[4 * c + row];
        }
    }
    StartFrame { matrix, emitter_velocity, look_rotation }
}

/// The first three columns times v: ((c0*v.x + c1*v.y) + c2*v.z) per row.
fn columns_times(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| (m[row] * v[0] + m[4 + row] * v[1]) + m[8 + row] * v[2])
}

/// The engine's look rotation: z the unit direction, x = up cross z made
/// unit, y = z cross x; a direction or an x shorter than the epsilon fails,
/// and so does a y whose squared length is off unity by more than the
/// tolerance (a NaN anywhere fails there). Columns x, y, z.
pub fn look_rotation_to_matrix(direction: [f32; 3], up: [f32; 3]) -> Option<[f32; 9]> {
    let [dx, dy, dz] = direction;
    let length = ((dx * dx + dy * dy) + dz * dz).sqrt();
    if length < DIRECTION_EPSILON {
        return None;
    }
    let (zx, zy, zz) = (dx / length, dy / length, dz / length);
    let [ux, uy, uz] = up;
    let (xx, xy, xz) = (zz * uy - zy * uz, zx * uz - zz * ux, zy * ux - zx * uy);
    let x_length = (xz * xz + (xx * xx + xy * xy)).sqrt();
    if x_length < DIRECTION_EPSILON {
        return None;
    }
    let (xx, xy, xz) = (xx / x_length, xy / x_length, xz / x_length);
    let (yx, yy, yz) = (zy * xz - zz * xy, zz * xx - zx * xz, zx * xy - zy * xx);
    let square = (yz * yz + (yx * yx + yy * yy)) + -1.0;
    let off = if square < 0.0 { -square } else { square };
    (off <= ORTHONORMAL_TOLERANCE).then_some([xx, xy, xz, yx, yy, yz, zx, zy, zz])
}

/// a <- a * b for column-major 3x3, each entry summed k = 0, 1, 2 left to right.
fn mat3_mul(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    std::array::from_fn(|i| {
        let (c, r) = (i / 3, i % 3);
        (a[r] * b[3 * c] + a[3 + r] * b[3 * c + 1]) + a[6 + r] * b[3 * c + 2]
    })
}

/// a * b over rows 0..2; the fourth column gets a's translation added last;
/// the last row is (0, 0, 0, 1).
fn mul3x4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..3 {
            let sum = (a[r] * b[4 * c] + a[4 + r] * b[4 * c + 1]) + a[8 + r] * b[4 * c + 2];
            out[4 * c + r] = if c == 3 { a[12 + r] + sum } else { sum };
        }
    }
    out[15] = 1.0;
    out
}

/// The catch-up of one command: the frame dt each step subtracts and
/// integrates with, and the step the loop compares against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatchUpPlan {
    pub frame_dt: f32,
    pub step: f32,
    /// Whether any catch-up step runs: the update flags enable it (bit 0 or
    /// bit 2), the step exceeds the minimum and the catch-up covers one step.
    pub runs: bool,
}

/// The frame dt is the frame's own when the world plays, 0.02 otherwise.
/// Unless flag bit 2 is set, a catch-up above 5 s (10 s) raises the step to
/// the smaller of the system duration and 0.2 (1) when the frame dt is not
/// already larger. Each step still subtracts and integrates the frame dt.
/// The unscaled frame dt of a target with unscaled time is not taken here.
pub fn catch_up_plan(catch_up: f32, frame_dt: f32, world_playing: bool, flags: u32, duration: f32)
    -> CatchUpPlan {
    let frame_dt = if world_playing { frame_dt } else { NOT_PLAYING_DT };
    let mut step = frame_dt;
    if flags & 4 == 0 {
        let bound = if catch_up > 10.0 {
            Some(1.0)
        } else if catch_up > 5.0 {
            Some(RAISED_STEP)
        } else {
            None
        };
        if let Some(bound) = bound {
            if !(step > bound) {
                step = min_number(duration, bound);
            }
        }
    }
    let runs = step > MIN_CATCH_UP_STEP && flags & 5 != 0 && catch_up >= step;
    CatchUpPlan { frame_dt, step, runs }
}

/// The smaller of two values, the other one when one is NaN, -0.0 below
/// +0.0 (the ARM minimum-number instruction).
fn min_number(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

/// The velocity the target's Initial update adds for gravity over dt, or
/// None when the gravity vector is shorter than the epsilon (nothing is
/// added then): the modifier times dt times gravity, taken through the
/// world-to-local's linear part for a target that is not World-space.
pub fn gravity_delta(gravity: [f32; 3], modifier: f32, dt: f32, world_to_local: Option<&[f32; 16]>)
    -> Option<[f32; 3]> {
    let square = gravity.map(|g| (0.0 - g) * (0.0 - g));
    if (square[0] + square[1]) + (square[2] + 0.0) <= GRAVITY_EPSILON {
        return None;
    }
    let k = modifier * dt;
    let d = gravity.map(|g| k * g);
    Some(match world_to_local {
        None => d,
        Some(m) => std::array::from_fn(|a| d[0] * m[a] + (d[1] * m[4 + a] + d[2] * m[8 + a])),
    })
}

/// The minimum with NaN propagated (the ARM minimum instruction).
pub fn min_propagating(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return f32::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_finite_or_nonfinite_word_panics() {
        let words = [0.0, -0.0, 1.0, -1.0, 1e-30, 1e30, f32::MAX, f32::NAN, f32::INFINITY,
            f32::NEG_INFINITY, 3.5e-8, 0.5];
        for &a in &words {
            for &b in &words {
                let owner = ChildOwner {
                    local_to_world: [a, b, a, 0.0, b, a, b, 0.0, a, a, b, 0.0, b, b, a, 1.0],
                    world_to_local: [b, a, b, 0.0, a, b, a, 0.0, b, b, a, 0.0, a, a, b, 1.0],
                    local_rotation: [a, b, a, b, a, b, a, b, a],
                    emitter_scale: [a, b, a],
                    shape_scale: [b, a, b],
                };
                let _ = start_frame(&owner, false, [a, b, a], [b, a, b]);
                let _ = start_frame(&owner, true, [a, b, a], [b, a, b]);
                let _ = catch_up_plan(a, b, a > 0.0, 5, b);
                let _ = gravity_delta([a, b, a], b, a, Some(&owner.world_to_local));
            }
        }
    }
}
