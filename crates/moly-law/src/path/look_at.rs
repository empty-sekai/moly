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
