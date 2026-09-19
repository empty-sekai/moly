//! Source particle emission geometry, before EmitterStoreData transforms and
//! direction normalisation. Random values are independent native U01 draws.
//!
//! Current JP ARM64 functions were executed independently to produce the checked
//! numerical corpus. The circle, cone and torus do NOT share a thickness law.
//! The native trigonometric kernel is signed and contains all five coefficients.
//! A two-dimensional billboard is a renderer choice, never a substitute shape.

const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
const INV_TAU: f32 = f32::from_bits(0x3e22_f983);
const NATIVE_TAU: f32 = f32::from_bits(0x40c9_0fdb);
const MIN_INNER: f32 = f32::from_bits(0x3a83_126f);

/// Circle's annulus is uniform in area. A zero thickness is the outer ring;
/// full thickness is the entire disk, not a fixed-radius circle.
pub fn circle_position(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    radial: f32,
    arc: f32,
) -> [f32; 3] {
    circle_base(radius, thickness, arc_deg, arc, radial).0
}

/// The source xorshift stream projects its low 23 bits through this exact
/// float multiplier; both endpoints are possible in the source representation.
pub fn u01_from_bits(bits: u32) -> f32 {
    (bits & 0x7f_ffff) as f32 * f32::from_bits(0x3400_0001)
}

/// Return the native (sin, cos) pair. Sign-preserving nearest-even reduction is
/// intentional; using abs(sin) or an incomplete polynomial breaks half a circle.
fn engine_sincos(angle: f32) -> (f32, f32) {
    fn polynomial(value: f32) -> f32 {
        let square = value * value;
        let fourth = square * square;
        let first = f32::from_bits(0x40c9_0fda) - square * f32::from_bits(0x4225_5ddc);
        let second = fourth * (f32::from_bits(0x42a3_3422) - square * f32::from_bits(0x4299_2322));
        let third = (fourth * fourth) * f32::from_bits(0x421e_a0cd);
        value * (third + (first + second))
    }
    fn fold(value: f32) -> f32 {
        let magic = f32::from_bits(0x4b00_0000).copysign(value);
        let nearest = (value + magic) - magic;
        0.25 - (value - nearest).abs()
    }
    let turns = angle * INV_TAU;
    (polynomial(fold(turns - 0.25)), polynomial(fold(turns)))
}

/// Native vector log2/exp2 cube-root kernel. It is not the host cbrt intrinsic.
/// The scalar shell preparation remains the source log2f/exp2f call sequence.
fn sphere_radius(radius: f32, thickness: f32, random: f32) -> f32 {
    let inner_cube = ((1.0 - thickness).log2() * 3.0).exp2();
    let volume = inner_cube * random + (1.0 - random);
    let bits = volume.to_bits();
    let exponent = (bits as i32 >> 23) as f32;
    let mantissa = f32::from_bits((bits & 0x807f_ffff) | 0x3f80_0000) - 1.0;
    let first = (exponent - 127.0) + mantissa * f32::from_bits(0x3fb8_0d57);
    let second = (mantissa * mantissa)
        * (mantissa * f32::from_bits(0x3e47_0bd9) + f32::from_bits(0xbf21_dda4));
    let power = ((first + second) * f32::from_bits(0x3eaa_aaab)).max(-127.0);
    let integer = power.floor();
    let fraction = power - integer;
    let estimate = ((fraction * fraction) * f32::from_bits(0x3ea2_ad7f))
        + (fraction * f32::from_bits(0x3f2e_a941) + 1.0);
    let exponent_bits = (integer as i32).wrapping_shl(23).wrapping_add(0x3f80_0000);
    radius * (estimate * f32::from_bits(exponent_bits as u32))
}

/// Existing Z-X-Y source rotation and producer-coordinate convention.
pub fn euler_rotate_deg(angles_xyz_deg: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    let [rx, ry, rz] = angles_xyz_deg.map(f32::to_radians);
    let v = rotate_z(rz, v);
    let v = rotate_x(rx, v);
    rotate_y(ry, v)
}

fn rotate_x(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

fn rotate_y(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

fn rotate_z(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

/// Circle's independent draws are arc followed by radial fraction. Its start
/// direction is radial in XY, not the billboard's view normal.
pub fn circle_base(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let (sin, cos) = engine_sincos((arc_deg * DEG_TO_RAD) * arc);
    let inner_squared = (1.0 - thickness) * (1.0 - thickness);
    let sample_radius = radius * (inner_squared + (1.0 - inner_squared) * radial).sqrt();
    (
        [cos * sample_radius, sin * sample_radius, 0.0],
        [cos, sin, 0.0],
    )
}

/// Cone uses a clamped linear inner-area fraction and reversed radial draw.
/// Direction contains that radial factor and is normalised by EmitterStoreData.
pub fn cone_base(
    radius: f32,
    thickness: f32,
    angle_deg: f32,
    arc_deg: f32,
    arc: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let inner = (1.0 - thickness).max(MIN_INNER);
    let fraction = (inner * radial + (1.0 - radial)).sqrt();
    let (sin, cos) = engine_sincos((arc_deg * DEG_TO_RAD) * arc);
    let x = fraction * cos;
    let y = fraction * sin;
    let (sin_angle, cos_angle) = engine_sincos(angle_deg * DEG_TO_RAD);
    (
        [radius * x, radius * y, 0.0],
        [sin_angle * x, sin_angle * y, cos_angle],
    )
}

/// Current native StartConeVolume samples the same base as Cone, then travels
/// a uniform authored distance along its normalised initial direction. It is
/// not a uniformly filled frustum and is not radius + tan(angle) * height.
/// The returned direction is still the pre-EmitterStoreData source vector.
pub fn cone_volume(
    radius: f32, thickness: f32, angle_deg: f32, arc_deg: f32,
    length: f32, arc: f32, radial: f32, distance: f32,
) -> ([f32; 3], [f32; 3]) {
    let (base, direction) = cone_base(radius, thickness, angle_deg, arc_deg, arc, radial);
    // Preserve the native sum order. ARM64 uses two reciprocal-sqrt refinement
    // steps; the scalar reciprocal sqrt differs by at most a few float ULP.
    let square = direction[0] * direction[0]
        + (direction[2] * direction[2] + direction[1] * direction[1]);
    let inverse = if square > f32::from_bits(0x0da2_4260) { 1.0 / square.sqrt() } else { 0.0 };
    let travel = length * distance;
    (std::array::from_fn(|axis| base[axis] + (direction[axis] * inverse) * travel), direction)
}

/// Sphere consumes three independent draws: arc, z and radius. Arc is authored
/// for spheres too; retaining it avoids a silent full-circle substitution.
pub fn sphere_position(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    z_random: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    sphere_sample(
        radius,
        thickness,
        arc_deg,
        arc,
        z_random + z_random - 1.0,
        radial,
    )
}

/// Hemisphere samples z directly in [0,1], not abs(2*u-1). Those choices have
/// equal distributions but different authored RNG-to-particle correspondence.
pub fn hemisphere_position(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    z_random: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    sphere_sample(radius, thickness, arc_deg, arc, z_random, radial)
}

fn sphere_sample(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    z: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let (sin, cos) = engine_sincos((arc_deg * DEG_TO_RAD) * arc);
    let xy = (1.0 - z * z).sqrt();
    let direction = [cos * xy, sin * xy, z];
    let sample_radius = sphere_radius(radius, thickness, radial);
    (
        direction.map(|component| component * sample_radius),
        direction,
    )
}

/// The torus tube radius is linearly sampled, unlike the circle's area law.
/// Three draws belong to the major arc, tube angle and tube radius respectively.
pub fn donut_position(
    radius: f32,
    donut_radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    tube_angle: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let (sin, cos) = engine_sincos((arc_deg * DEG_TO_RAD) * arc);
    let (tube_sin, tube_cos) = engine_sincos(tube_angle * NATIVE_TAU);
    let inner = (1.0 - thickness).max(MIN_INNER);
    let tube = (inner + (1.0 - inner) * radial) * donut_radius;
    let direction = [cos * tube_cos, sin * tube_cos, tube_sin];
    (
        [
            radius * cos + direction[0] * tube,
            radius * sin + direction[1] * tube,
            direction[2] * tube,
        ],
        direction,
    )
}

/// Single-sided edge emits along local +Y, unlike rectangle/box's local +Z.
pub fn single_sided_edge(radius: f32, random: f32) -> ([f32; 3], [f32; 3]) {
    // Current native scales first, then doubles and subtracts. Reassociating
    // (2*u-1)*R produces a measurable error for samples near the edge centre.
    let scaled = radius * random;
    ([(scaled + scaled) - radius, 0.0, 0.0], [0.0, 1.0, 0.0])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn magnitude(v: [f32; 3]) -> f32 {
        v.iter().map(|x| x * x).sum::<f32>().sqrt()
    }
    #[test]
    fn circle_shell_and_full_disk_cannot_collapse() {
        for radial in [0.0, 0.1, 0.5, 1.0] {
            assert!((magnitude(circle_base(2.0, 0.0, 360.0, 0.17, radial).0) - 2.0).abs() < 1e-5);
            assert!(
                (magnitude(circle_base(2.0, 1.0, 360.0, 0.17, radial).0) - 2.0 * radial.sqrt())
                    .abs()
                    < 1e-5
            );
        }
        let directions: Vec<_> = [0.0, 0.25, 0.5, 0.75]
            .map(|arc| circle_base(1.0, 0.0, 360.0, arc, 0.0).1)
            .into();
        for (a, b) in directions.iter().zip([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
        ]) {
            assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 2e-6));
        }
    }
    #[test]
    fn native_direction_and_shell_boundaries_are_preserved() {
        assert_eq!(
            single_sided_edge(2.0, 0.0),
            ([-2.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        );
        let outer = cone_base(10.0, 1.0, 25.0, 360.0, 0.17, 0.0);
        let inner = cone_base(10.0, 1.0, 25.0, 360.0, 0.17, 1.0);
        assert!(magnitude(outer.0) > 9.999);
        assert!((magnitude(inner.0) - 10.0 * MIN_INNER.sqrt()).abs() < 1e-5);
        assert!(
            magnitude(inner.1) < magnitude(outer.1),
            "normalisation is a later source operation"
        );
        let sphere = sphere_position(10.0, 0.0, 360.0, 0.3, 0.1, 0.8);
        let hemisphere = hemisphere_position(10.0, 0.0, 360.0, 0.3, 0.1, 0.8);
        assert!((sphere.1[2] + 0.8).abs() < 1e-7);
        assert_eq!(hemisphere.1[2], 0.1);
        assert!((magnitude(sphere.0) - 10.0).abs() < 1e-5);
    }
    #[test]
    fn euler_zxy_keeps_existing_coordinate_contract() {
        let v = euler_rotate_deg([-90.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((v[2] + 1.0).abs() < 1e-6);
        let v = euler_rotate_deg([0.0, 90.0, 0.0], [0.0, 0.0, 1.0]);
        assert!((v[0] - 1.0).abs() < 1e-6);
    }
    struct Vector {
        shape: String,
        radius: f32,
        thickness: f32,
        arc: f32,
        angle: f32,
        donut_radius: f32,
        random: [f32; 3],
        position: [f32; 3],
        direction: [f32; 3],
    }
    fn source_vectors() -> Vec<Vector> {
        include_str!("../../tests/data/particle-shape-source-vectors.tsv")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let columns: Vec<_> = line.split('\t').collect();
                assert_eq!(columns.len(), 15);
                let value = |i: usize| columns[i].parse::<f32>().unwrap();
                Vector {
                    shape: columns[0].into(),
                    radius: value(1),
                    thickness: value(2),
                    arc: value(3),
                    angle: value(4),
                    donut_radius: value(5),
                    random: [value(6), value(7), value(8)],
                    position: [value(9), value(10), value(11)],
                    direction: [value(12), value(13), value(14)],
                }
            })
            .collect()
    }
    #[test]
    fn current_arm64_vectors_match_scalar_shape_equations() {
        let vectors = source_vectors();
        assert_eq!(vectors.len(), 360);
        let mut failures = Vec::new();
        for v in vectors {
            let r = &v.random;
            let (position, direction) = match v.shape.as_str() {
                "Circle" => circle_base(v.radius, v.thickness, v.arc, r[0], r[1]),
                "Cone" => cone_base(v.radius, v.thickness, v.angle, v.arc, r[0], r[1]),
                "Sphere" => sphere_position(v.radius, v.thickness, v.arc, r[0], r[1], r[2]),
                "HemiSphere" => hemisphere_position(v.radius, v.thickness, v.arc, r[0], r[1], r[2]),
                "Donut" => donut_position(
                    v.radius,
                    v.donut_radius,
                    v.thickness,
                    v.arc,
                    r[0],
                    r[1],
                    r[2],
                ),
                "SingleSidedEdge" => single_sided_edge(v.radius, r[0]),
                _ => panic!("unhandled source shape"),
            };
            // Scalar shell preparation calls platform libm in the source too.
            // All tested components must agree within 8 f32 ULP, not pixel-level
            // tolerance. Incorrect signs, RNG counts and radial laws fail hard.
            for (kind, actual, expected) in [
                ("position", position, v.position),
                ("direction", direction, v.direction),
            ] {
                for (axis, (a, b)) in actual.into_iter().zip(expected).enumerate() {
                    let ulp = a.to_bits().abs_diff(b.to_bits());
                    if a != b && (ulp > 8 || !a.is_finite()) {
                        failures.push(format!("{} thickness={} arc={} {kind}[{axis}] actual={a:?} native={b:?} ulp={ulp}",v.shape,v.thickness,v.arc));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} source component differences:\n{}",
            failures.len(),
            failures
                .iter()
                .take(30)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    #[test]
    fn current_cone_volume_native_vectors_preserve_authored_length_and_draws() {
        let text = include_str!("../../tests/data/particle-cone-volume-source-vectors.tsv");
        let mut count = 0;
        let mut failures = Vec::new();
        for row in text.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
            let v: Vec<f32> = row.split('\t').map(|x| x.parse().unwrap()).collect();
            assert_eq!(v.len(),14);
            let (position,direction) = cone_volume(v[0],v[1],v[3],v[2],v[4],v[5],v[6],v[7]);
            for (axis,(actual,expected)) in position.into_iter().chain(direction).zip(v[8..].iter().copied()).enumerate() {
                let ulp=actual.to_bits().abs_diff(expected.to_bits());
                if actual != expected && (!actual.is_finite() || ulp>8) {
                    failures.push(format!("case={count} axis={axis} actual={actual:?} source={expected:?} ulp={ulp}"));
                }
            }
            count+=1;
        }
        assert!(count>=184,"the native corpus must not disappear");
        assert!(failures.is_empty(),"{} native differences: {}",failures.len(),failures.join("\n"));
    }

}
