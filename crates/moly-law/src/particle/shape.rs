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

/// Source EmitterStoreData offsets position on a sphere of the authored
/// radius, not inside a cube/ball. This consumes two additional source draws
/// only when amount is positive, before shape scaling/rotation/translation.
/// Direction is unchanged by position jitter.
pub fn randomize_position(position: [f32; 3], amount: f32, arc: f32, polar: f32) -> [f32; 3] {
    if amount <= 0.0 { return position; }
    let (sin, cos) = engine_sincos(arc * NATIVE_TAU);
    let z = (polar + polar) - 1.0;
    let xy = (1.0 - z * z).sqrt();
    [position[0] + (cos * xy) * amount,
     position[1] + (sin * xy) * amount,
     position[2] + z * amount]
}

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
pub(crate) fn engine_sincos(angle: f32) -> (f32, f32) {
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

/// Radial shell of the native sphere kernels, restricted to the two authored
/// thickness values whose scalar shell preparation is fixed bit for bit by
/// IEEE 754 / C99 Annex F: `exp2f(log2f(1 - thickness) * 3)`. Thickness one is
/// `log2f(+0) = -inf`, times three, `exp2f(-inf) = +0` (a filled ball);
/// thickness zero is `log2f(1) = +0`, `exp2f(+0) = 1` (the outer surface).
/// Every other thickness takes its bits from the device libm, which is not
/// part of the engine library, so it cannot be constructed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Full,
    Surface,
}
impl Shell {
    pub fn from_thickness(thickness: f32) -> Option<Self> {
        if thickness == 1.0 {
            Some(Self::Full)
        } else if thickness == 0.0 {
            Some(Self::Surface)
        } else {
            None
        }
    }
    /// The inner radius cubed that the scalar shell preparation returns.
    pub fn inner_cube(self) -> f32 {
        match self {
            Self::Full => 0.0,
            Self::Surface => 1.0,
        }
    }
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

// Current ARM FRSQRTE normalized positive domain. Integer midpoint/table
// quantization, not a host reciprocal sqrt. Native refinement FRSQRTS uses
// fused rounding of (3-a*b)/2 after the preceding separate f32 FMUL.
fn rsqrt_estimate(value: f32) -> f32 {
    assert!(value.is_normal() && value > 0.0);
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 255) as i32;
    let index = ((bits & 0x7fffff) >> 16) as usize + if exponent & 1 == 0 { 128 } else { 0 };
    let estimate = RSQRT_ESTIMATE[index] as u32;
    let estimate_bits = ((((380 - exponent) / 2) as u32) << 23) | (((estimate as u32) - 256) << 15);
    f32::from_bits(estimate_bits)
}

/// The FRSQRTE table lookup alone, for replaying recorded estimates.
#[cfg(test)]
pub(crate) fn native_rsqrt_estimate(value: f32) -> f32 {
    rsqrt_estimate(value)
}

/// FRSQRTE followed by two FRSQRTS refinements, each multiplying the current
/// estimate by the square first. Every caller first masks its square at or
/// below a positive threshold, so the value is normal positive, +inf or NaN.
/// FRSQRTE of +inf is +0, whose first refinement multiplies back into inf
/// times zero, so the native result is NaN, as it is for NaN input; both
/// return NaN here and the caller's output refusal reports them.
pub(crate) fn native_rsqrt(value: f32) -> f32 {
    if !(value.is_normal() && value > 0.0) {
        return f32::NAN;
    }
    let r0 = rsqrt_estimate(value);
    let step = |a: f32, b: f32| ((3.0_f64 - (a as f64) * (b as f64)) * 0.5) as f32;
    let r1 = r0 * step(r0 * value, r0);
    r1 * step(value * r1, r1)
}

/// Native vector log2/exp2 cube-root kernel. It is not the host cbrt intrinsic.
/// The scalar shell preparation remains the source log2f/exp2f call sequence;
/// host libm stands in for the device one here, exact only at `Shell` values.
fn sphere_radius(radius: f32, thickness: f32, random: f32) -> f32 {
    sphere_radius_from_inner(radius, ((1.0 - thickness).log2() * 3.0).exp2(), random)
}

fn sphere_radius_from_inner(radius: f32, inner_cube: f32, random: f32) -> f32 {
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
    circle_at(radius, thickness, (arc_deg * DEG_TO_RAD) * arc, radial)
}

/// StartCircle's lane body at an angle in radians: the plain kernel's angle
/// is the arc in radians times the arc draw (`circle_base`), the Random arc
/// mode's is `random_arc`; both then take the same sine, cosine and radial
/// fraction.
pub fn circle_at(
    radius: f32,
    thickness: f32,
    arc_radians: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let (sin, cos) = engine_sincos(arc_radians);
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

/// The Random mode's spread quantization of one draw over an extent (the arc
/// in radians, or the single-sided edge's radius). With a positive step
/// (extent times spread) the value snaps down to a multiple of the step: the
/// step count is the extent over the step rounded up, and both roundings go
/// through a saturating float-to-int conversion (FCVTZS, NaN to zero) and a
/// compare-and-adjust, not floor/ceil, which agree only below 2^31. The
/// quotient keeps the native order ((step * count) * u) / step; the
/// algebraic shortcut count * u rounds differently for a few draws per
/// configuration. A zero, negative or unordered step keeps the continuous
/// extent * u.
fn spread_quantized(extent: f32, spread: f32, u: f32) -> f32 {
    let step = extent * spread;
    if !(step > 0.0) {
        return extent * u;
    }
    let ratio = extent / step;
    let whole = (ratio as i32) as f32;
    let count = if ratio > whole { 1.0 + whole } else { whole };
    let steps = ((step * count) * u) / step;
    let whole = (steps as i32) as f32;
    step * if whole > steps { whole - 1.0 } else { whole }
}

/// The Random arc mode's angle in radians, shared by StartHemiSphere,
/// StartConeVolume, StartDonut and StartCircle: `spread_quantized` over the
/// arc in radians. Where the arc in radians times the spread is not above
/// zero (NaN included) this is the continuous arc times the draw, which is
/// StartCircle's plain kernel angle; above zero it is StartCircle's
/// arc-spread branch, the same operations in the same order.
pub fn random_arc(arc_deg: f32, arc_spread: f32, u: f32) -> f32 {
    spread_quantized(arc_deg * DEG_TO_RAD, arc_spread, u)
}

/// Current native StartConeVolume lane body: the Cone base (clamped linear
/// inner-area fraction, radial draw reversed) at the given arc angle, then a
/// uniform authored distance along the unit initial direction. It is not a
/// uniformly filled frustum and is not radius + tan(angle) * height. The unit
/// vector comes from the native reciprocal square root of the direction's
/// square (x*x + (cos^2 + y*y)), and where that square is not above the tiny
/// threshold (NaN included) the unit vector is masked to +0, not to +Z; the
/// travelled z keeps the native + 0.0. The returned direction is the
/// un-normalised pre-EmitterStoreData source vector.
pub fn cone_volume_at(
    radius: f32, thickness: f32, angle_deg: f32, arc_radians: f32,
    length: f32, radial: f32, distance: f32,
) -> ([f32; 3], [f32; 3]) {
    let inner = (1.0 - thickness).max(MIN_INNER);
    let (sin, cos) = engine_sincos(arc_radians);
    let fraction = (inner * radial + (1.0 - radial)).sqrt();
    let x = fraction * cos;
    let y = fraction * sin;
    let (sin_angle, cos_angle) = engine_sincos(angle_deg * DEG_TO_RAD);
    let direction = [sin_angle * x, sin_angle * y, cos_angle];
    let square = direction[0] * direction[0]
        + (direction[2] * direction[2] + direction[1] * direction[1]);
    let keep = square > f32::from_bits(0x0da2_4260);
    let inverse = if keep { native_rsqrt(square) } else { 0.0 };
    let unit = direction.map(|v| if keep { v * inverse } else { 0.0 });
    let travel = length * distance;
    (
        [
            radius * x + unit[0] * travel,
            radius * y + unit[1] * travel,
            unit[2] * travel + 0.0,
        ],
        direction,
    )
}

/// StartConeVolume with a zero arc spread, for the legacy step: independent
/// draws arc, radial fraction, travelled distance.
pub fn cone_volume(
    radius: f32, thickness: f32, angle_deg: f32, arc_deg: f32,
    length: f32, arc: f32, radial: f32, distance: f32,
) -> ([f32; 3], [f32; 3]) {
    cone_volume_at(radius, thickness, angle_deg, random_arc(arc_deg, 0.0, arc), length, radial, distance)
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

/// Hemisphere folds its draw into the sphere kernel's upper half. Preserve
/// the native f32 operations of StartHemiSphere's fold: replacing
/// this mathematically equivalent expression with z_random changes low bits.
pub fn hemisphere_position(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    z_random: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let z = z_random * 0.5 + 0.5;
    let z = (z + z) - 1.0;
    sphere_sample(radius, thickness, arc_deg, arc, z, radial)
}

/// StartHemiSphere in its Random arc mode, one lane, native operation order.
/// The arc angle is `random_arc`'s. The three draws are the arc, the
/// hemisphere z and the radial fraction, in that order.
pub fn hemisphere_native(
    radius: f32,
    shell: Shell,
    arc_deg: f32,
    arc_spread: f32,
    arc: f32,
    z_random: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let angle = random_arc(arc_deg, arc_spread, arc);
    let z = z_random * 0.5 + 0.5;
    let z = (z + z) - 1.0;
    let (sin, cos) = engine_sincos(angle);
    let xy = (1.0 - z * z).sqrt();
    let direction = [cos * xy, sin * xy, z];
    let sample_radius = sphere_radius_from_inner(radius, shell.inner_cube(), radial);
    (
        direction.map(|component| component * sample_radius),
        direction,
    )
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
/// Three draws belong to the major arc, tube angle and tube radius
/// respectively. Zero arc spread: the major angle is the arc in radians
/// times the draw, as `random_arc` gives it.
pub fn donut_position(
    radius: f32,
    donut_radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc: f32,
    tube_angle: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    donut_at(radius, donut_radius, thickness, random_arc(arc_deg, 0.0, arc), tube_angle, radial)
}

/// Current native StartDonut lane body at a given major angle in radians
/// (`random_arc` of the Random arc mode, the stepped arc included). The tube
/// fraction is the inner bound max(1 - thickness, tiny) plus the rest times
/// the draw, times the torus radius. The native code factors the major
/// cosine and sine out of the ring: x = cos * (radius + tube * tube_cos),
/// not radius * cos + (cos * tube_cos) * tube, which rounds differently in
/// most lanes; z is tube * tube_sin. The returned direction is the
/// un-normalised pre-EmitterStoreData source vector (cos * tube_cos,
/// sin * tube_cos, tube_sin). The inner bound uses a maximum that would
/// drop a NaN where the native one keeps it; 1 - thickness is finite for
/// every finite thickness, so the two agree on every finite input.
pub fn donut_at(
    radius: f32,
    donut_radius: f32,
    thickness: f32,
    arc_radians: f32,
    tube_angle: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let (sin, cos) = engine_sincos(arc_radians);
    let (tube_sin, tube_cos) = engine_sincos(tube_angle * NATIVE_TAU);
    let inner = (1.0 - thickness).max(MIN_INNER);
    let tube = (inner + (1.0 - inner) * radial) * donut_radius;
    let ring = radius + tube * tube_cos;
    (
        [cos * ring, sin * ring, tube * tube_sin],
        [cos * tube_cos, sin * tube_cos, tube_sin],
    )
}

/// Single-sided edge emits along local +Y, unlike rectangle/box's local +Z.
pub fn single_sided_edge(radius: f32, random: f32) -> ([f32; 3], [f32; 3]) {
    // Current native scales first, then doubles and subtracts. Reassociating
    // (2*u-1)*R produces a measurable error for samples near the edge centre.
    let scaled = radius * random;
    ([(scaled + scaled) - radius, 0.0, 0.0], [0.0, 1.0, 0.0])
}

/// StartSingleSidedEdge in its Random radius mode, one lane, one draw: the
/// draw is `spread_quantized` over the radius with the radius spread, then
/// doubled and the radius subtracted (never reassociated to (2u-1)R). A
/// zero, negative or unordered step (radius times spread) is exactly
/// `single_sided_edge`. The kernel reads only the radius and its spread:
/// thickness, arc, arc mode and arc spread never reach it.
pub fn single_sided_edge_spread(radius: f32, spread: f32, random: f32) -> ([f32; 3], [f32; 3]) {
    let scaled = spread_quantized(radius, spread, random);
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
        assert_eq!(hemisphere.1[2].to_bits(), 0x3dcc_ccd0);
        assert!((magnitude(sphere.0) - 10.0).abs() < 1e-5);
    }
    #[test]
    fn source_snow_hemisphere_first_group_matches_native_bits() {
        // The current native shape birth receipt: source 0, old=0, first one-particle
        // request. All four native storage lanes remain observable at Store.
        let expected_position = [
            [3256462770, 3229795817, 1105218461, 3251943192],
            [3252131890, 3253306700, 3248049907, 1090117893],
            [1092670063, 1108744017, 1096315287, 1060524061],
        ];
        let expected_direction = [
            [3209490136, 3182389207, 1061443572, 3212151733],
            [3205480418, 3206323265, 3204854778, 1049639170],
            [1045853568, 1061769498, 1052604524, 1020425088],
        ];
        let mut stream = crate::particle::seed_owner::ModuleRandom::from_owner_seed(1729);
        let arc = stream.next4_u32().map(u01_from_bits);
        let polar = stream.next4_u32().map(u01_from_bits);
        let radial = stream.next4_u32().map(u01_from_bits);
        for lane in 0..4 {
            let (position, direction) = hemisphere_position(50.0, 1.0, 360.0,
                arc[lane], polar[lane], radial[lane]);
            for axis in 0..3 {
                assert_eq!(position[axis].to_bits(), expected_position[axis][lane]);
                assert_eq!(direction[axis].to_bits(), expected_direction[axis][lane]);
            }
        }
    }
    #[test]
    fn euler_zxy_keeps_existing_coordinate_contract() {
        let v = euler_rotate_deg([-90.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((v[2] + 1.0).abs() < 1e-6);
        let v = euler_rotate_deg([0.0, 90.0, 0.0], [0.0, 0.0, 1.0]);
        assert!((v[0] - 1.0).abs() < 1e-6);
    }
    #[test]
    fn position_jitter_matches_independently_executed_native_instructions() {
        let mut count = 0;
        for row in include_str!("../../tests/data/particle-position-jitter.tsv").lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#')) {
            let v: Vec<f32> = row.split('\t').map(|x| x.parse().unwrap()).collect();
            assert_eq!(v.len(), 9);
            let actual = randomize_position([v[0], v[1], v[2]], v[3], v[4], v[5]);
            for (axis, (actual, expected)) in actual.into_iter().zip(v[6..].iter().copied()).enumerate() {
                assert!(actual == expected || actual.to_bits().abs_diff(expected.to_bits()) <= 4,
                    "case={count} axis={axis} actual={actual:?} native={expected:?}");
            }
            count += 1;
        }
        assert_eq!(count, 256);
        assert_eq!(randomize_position([1.0, 2.0, 3.0], 0.0, f32::NAN, f32::NAN), [1.0, 2.0, 3.0]);
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
            // The torus kernel calls no libm and is exact.
            let exact = v.shape == "Donut";
            for (kind, actual, expected) in [
                ("position", position, v.position),
                ("direction", direction, v.direction),
            ] {
                for (axis, (a, b)) in actual.into_iter().zip(expected).enumerate() {
                    let ulp = a.to_bits().abs_diff(b.to_bits());
                    if (exact && a.to_bits() != b.to_bits()) || (a != b && (ulp > 8 || !a.is_finite())) {
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
            // Exact: the reciprocal square root is the native table and
            // refinement, not a host 1/sqrt.
            for (axis,(actual,expected)) in position.into_iter().chain(direction).zip(v[8..].iter().copied()).enumerate() {
                if actual.to_bits() != expected.to_bits() {
                    let ulp=actual.to_bits().abs_diff(expected.to_bits());
                    failures.push(format!("case={count} axis={axis} actual={actual:?} source={expected:?} ulp={ulp}"));
                }
            }
            count+=1;
        }
        assert!(count>=184,"the native corpus must not disappear");
        assert!(failures.is_empty(),"{} native differences: {}",failures.len(),failures.join("\n"));
    }

}
