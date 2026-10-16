//! The view's look-at tween (`DOLookAt` with the Y axis constrained) on a
//! character: at the tween's first update the direction to the target
//! point loses its height, the end rotation is the look rotation of that
//! direction as Euler angles, and each Euler axis turns from the start
//! rotation's angles by [`fast_change`], eased; the rotation is rebuilt
//! from the angles every update.

/// The quaternion plugin's change on one Euler axis in fast mode (not
/// relative): an end above 360 is taken modulo 360, the change is the end
/// minus the start, and a change beyond 180 either way is taken the short
/// way round.
pub fn fast_change(start: f32, end: f32) -> f32 {
    let end = if end > 360.0 { end % 360.0 } else { end };
    let change = end - start;
    let magnitude = if change > 0.0 { change } else { -change };
    if magnitude > 180.0 {
        let short = 360.0 - magnitude;
        if change > 0.0 {
            -short
        } else {
            short
        }
    } else {
        change
    }
}

/// The yaw in degrees, in [0, 360), of a horizontal direction (the Euler
/// angle a look rotation of it carries); 0 for the zero direction (the
/// identity rotation).
pub fn yaw_of(direction: [f32; 2]) -> f32 {
    if direction[0] == 0.0 && direction[1] == 0.0 {
        return 0.0;
    }
    let yaw = direction[0].atan2(direction[1]).to_degrees();
    if yaw < 0.0 {
        yaw + 360.0
    } else {
        yaw
    }
}

/// The engine's small bound under which the unsigned angle is zero (the
/// square root of the two squared lengths' product; f32 bits `0x26901d7d`).
pub const SIGNED_ANGLE_ZERO_BOUND: f32 = 1e-15;

/// Degrees per radian as the engine multiplies it (f32 bits `0x42652ee1`).
pub const RAD_TO_DEG: f32 = 57.29578;

/// The engine's signed angle in degrees from `from` to `to` about `axis`:
/// the unsigned angle (`sqrt(|from|² · |to|²)` under
/// [`SIGNED_ANGLE_ZERO_BOUND`] gives 0; otherwise the dot product over it,
/// clamped to [-1, 1], through the double-precision arc cosine, times
/// [`RAD_TO_DEG`]), negative when the cross product points against the axis
/// (a zero or unordered side test counts as against only when it is not
/// at or above zero). `acos` is the library's double arc cosine.
pub fn signed_angle_by(
    from: [f32; 3],
    to: [f32; 3],
    axis: [f32; 3],
    acos: impl Fn(f64) -> f64,
) -> f32 {
    let from2 = (from[0] * from[0] + from[1] * from[1]) + from[2] * from[2];
    let to2 = (to[0] * to[0] + to[1] * to[1]) + to[2] * to[2];
    let denominator = (from2 * to2).sqrt();
    let angle = if denominator < SIGNED_ANGLE_ZERO_BOUND {
        0.0
    } else {
        let dot = (from[0] * to[0] + from[1] * to[1]) + from[2] * to[2];
        let cosine = dot / denominator;
        // The vector min keeps an unordered value; below -1 takes -1.
        let upper = if cosine.is_nan() || cosine < 1.0 {
            cosine
        } else {
            1.0
        };
        let clamped = if cosine < -1.0 { -1.0 } else { upper };
        (acos(clamped as f64) as f32) * RAD_TO_DEG
    };
    let cross = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    let side = (axis[0] * cross[0] + axis[1] * cross[1]) + axis[2] * cross[2];
    if side >= 0.0 {
        angle
    } else {
        -angle
    }
}

/// [`signed_angle_by`] with the platform's double arc cosine.
pub fn signed_angle(from: [f32; 3], to: [f32; 3], axis: [f32; 3]) -> f32 {
    signed_angle_by(from, to, axis, f64::acos)
}

/// The turn clip the presenter starts with a look-at: the signed angle about
/// the up axis from the character's forward to the target point itself (the
/// point taken from the world origin, not from the character), through the
/// six-band clip choice ([`super::turn_motion`]).
pub fn look_at_turn_motion(forward: [f32; 3], towards: [f32; 3]) -> super::TurnMotion {
    super::turn_motion(signed_angle(forward, towards, [0.0, 1.0, 0.0]))
}
