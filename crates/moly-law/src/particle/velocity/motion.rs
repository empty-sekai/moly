//! Finite-step orbital motion and its transient velocity contribution.
//!
//! Orbital axes and offsets are emitter-local, independently of the space used
//! by the linear velocity module. The caller transforms positions and
//! displacements between simulation and emitter space. Persistent particle
//! velocity is never modified by this module.

/// The orbital and radial curves evaluated for one particle at the current age.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitalMotion {
    /// Angular velocity in radians per second, composed in Z-X-Y order.
    pub angular: [f32; 3],
    pub offset: [f32; 3],
    pub radial: f32,
}

impl OrbitalMotion {
    /// Displacement in emitter-local coordinates. The speed modifier changes
    /// the angle and radial step before the displacement is calculated.
    pub fn displacement(self, position: [f32; 3], dt: f32, speed_modifier: f32) -> [f32; 3] {
        let step = dt * speed_modifier;
        let p = std::array::from_fn(|axis| position[axis] - self.offset[axis]);
        let angle = self.angular.map(|value| value * step);
        let mut rotated = rotate_zxy(p, angle);
        let length_squared = rotated[0] * rotated[0]
            + (rotated[1] * rotated[1] + rotated[2] * rotated[2]);
        if length_squared > f32::from_bits(0x0da2_4260) {
            let inverse_length = 1.0 / length_squared.sqrt();
            let distance = step * self.radial;
            for value in &mut rotated {
                *value += (*value * inverse_length) * distance;
            }
        }
        std::array::from_fn(|axis| rotated[axis] - p[axis])
    }
}

/// Convert a displacement, already transformed into simulation coordinates,
/// into the animated-velocity lane. Integration later reapplies the speed
/// modifier to the sum of persistent and animated velocity.
///
/// Near-zero time steps and speed modifiers use the same explicit masks as the
/// module's native kernel. In particular, zero speed does not mean unit speed.
pub fn animated_velocity(displacement: [f32; 3], dt: f32, speed_modifier: f32) -> [f32; 3] {
    if dt <= f32::from_bits(0x3586_37bd)
        || speed_modifier.abs() <= f32::from_bits(0x3089_705f)
    {
        return [0.0; 3];
    }
    let inverse_dt = 1.0 / dt;
    displacement.map(|value| (value / speed_modifier) * inverse_dt)
}

/// The particle kernel uses a periodic polynomial rather than platform libm.
/// Keeping its range reduction and coefficients avoids per-frame radius drift
/// differences between CPU and browser backends.
fn sin_cos(angle: f32) -> (f32, f32) {
    let turns = angle * f32::from_bits(0x3e22_f983);
    let reduce = |value: f32| {
        let magic = f32::from_bits((value.to_bits() & 0x8000_0000) | 0x4b00_0000);
        let rounded = (value + magic) - magic;
        0.25 - (value - rounded).abs()
    };
    let polynomial = |value: f32| {
        let square = value * value;
        let fourth = square * square;
        let eighth = fourth * fourth;
        let low = f32::from_bits(0x40c9_0fda) - square * f32::from_bits(0x4225_5ddc);
        let middle = fourth * (f32::from_bits(0x42a3_3422) - square * f32::from_bits(0x4299_2322));
        value * ((low + middle) + eighth * f32::from_bits(0x421e_a0cd))
    };
    (polynomial(reduce(turns - 0.25)), polynomial(reduce(turns)))
}

fn rotate_zxy(p: [f32; 3], angle: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = sin_cos(angle[0]);
    let (sy, cy) = sin_cos(angle[1]);
    let (sz, cz) = sin_cos(angle[2]);
    [
        (cy * cz + (sx * sy) * sz) * p[0]
            + (((sx * cz) * sy - cy * sz) * p[1] + (cx * sy) * p[2]),
        (cx * sz) * p[0] + ((cx * cz) * p[1] - sx * p[2]),
        ((cy * sx) * sz - cz * sy) * p[0]
            + ((sx * (cy * cz) + sy * sz) * p[1] + (cx * cy) * p[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: [f32; 3], expected: [f32; 3], tolerance: f32) {
        for axis in 0..3 {
            assert!((actual[axis] - expected[axis]).abs() <= tolerance,
                "axis {axis}: {actual:?} != {expected:?}");
        }
    }

    #[test]
    fn orbital_step_keeps_base_velocity_separate() {
        let motion = OrbitalMotion { angular: [1.0, 0.0, 0.0], offset: [0.0; 3], radial: 0.0 };
        let delta = motion.displacement([2.0, 1.0, 3.0], 0.01, 1.0);
        near(animated_velocity(delta, 0.01, 1.0), [0.0, -3.0049443, 0.9850025], 0.0001);
    }

    #[test]
    fn radial_step_follows_the_rotated_offset_vector() {
        let motion = OrbitalMotion { angular: [1.0, 0.0, 0.0], offset: [1.0, 2.0, 3.0], radial: 0.5 };
        let delta = motion.displacement([2.0, 1.0, 3.0], 0.01, 1.0);
        near(animated_velocity(delta, 0.01, 1.0), [0.3535509, -0.34854412, -1.0035181], 0.0001);
    }

    #[test]
    fn stopped_speed_does_not_create_orbital_velocity() {
        let motion = OrbitalMotion { angular: [3.0, -1.0, 2.0], offset: [0.0; 3], radial: 2.0 };
        let delta = motion.displacement([2.0, 1.0, 3.0], 0.1, 0.0);
        assert_eq!(animated_velocity(delta, 0.1, 0.0), [0.0; 3]);
        assert_eq!(animated_velocity([1.0; 3], 0.0, 1.0), [0.0; 3]);
    }
}
