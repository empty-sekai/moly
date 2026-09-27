//! Source particle emission geometry, before EmitterStoreData transforms and
//! direction normalisation. Random values are independent native U01 draws.
//!
//! Current JP ARM64 functions were executed independently to produce the checked
//! numerical corpus. The circle, cone and torus do NOT share a thickness law.
//! The native trigonometric kernel is signed and contains all five coefficients.
//! A two-dimensional billboard is a renderer choice, never a substitute shape.

use super::device_libm::{exp2f, log2f};

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

/// Radial shell of the native sphere kernels: the inner radius cubed that
/// StartSphere and StartHemiSphere prepare once per call before their lanes,
/// `exp2f(log2f(1 - thickness) * 3)`, through the device libm
/// ([`shell_inner_cube`]). Thickness one is `log2f(+0) = -inf`, times three,
/// `exp2f(-inf) = +0` (a filled ball); thickness zero is `log2f(1) = +0`,
/// `exp2f(+0) = 1` (the outer surface). The kernels do not clamp the
/// thickness; a value outside [0, 1] makes `1 - thickness` negative or pushes
/// the vector cube root outside the unit volume, which no native run covered,
/// so only [0, 1] is admitted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    inner_cube: f32,
}
impl Shell {
    pub fn from_thickness(thickness: f32) -> Option<Self> {
        (0.0..=1.0).contains(&thickness).then(|| Self { inner_cube: shell_inner_cube(thickness) })
    }
    /// The inner radius cubed that the scalar shell preparation returns.
    pub fn inner_cube(self) -> f32 {
        self.inner_cube
    }
}

/// The sphere kernels' scalar shell preparation: `1 - thickness`, the device
/// `log2f`, times three, the device `exp2f`.
pub fn shell_inner_cube(thickness: f32) -> f32 {
    #[cfg(test)]
    if shell_arms::on("shellBinary64Chain") {
        return (((1.0 - thickness) as f64).log2() * 3.0).exp2() as f32;
    }
    #[cfg(test)]
    if shell_arms::on("shellThicknessNotComplemented") {
        return exp2f(log2f(thickness) * 3.0);
    }
    exp2f(log2f(1.0 - thickness) * 3.0)
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
/// The scalar shell preparation is the source log2f/exp2f call sequence
/// through the device libm ([`shell_inner_cube`]).
fn sphere_radius(radius: f32, thickness: f32, random: f32) -> f32 {
    sphere_radius_from_inner(radius, shell_inner_cube(thickness), random)
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
    cone_at(radius, thickness, angle_deg, (arc_deg * DEG_TO_RAD) * arc, radial, None)
}

/// The two extra draws of the cone's random direction: an angle draw and an
/// area draw, blended in by the authored amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeJitter {
    pub amount: f32,
    pub arc: f32,
    pub area: f32,
}

/// StartCone lane body (all four arc modes share it) at a given arc
/// angle in radians. The radial fraction is the square root of the clamped
/// inner area max(1 - thickness, 0.001) times the draw plus the rest; the base
/// position is radius times that fraction around the arc, in the XY plane.
/// The direction leans out by the cone angle: its XY is the same fraction
/// times the sine of the cone angle, its Z the cosine. The cone kernel owns
/// the random direction itself (the Store then receives zero): with a
/// positive amount two more draws give a point in the disk of the same inner
/// area bound, and the base XY moves toward it by the amount before the
/// cone-angle sine is applied. The position keeps the unblended base.
pub fn cone_at(
    radius: f32,
    thickness: f32,
    angle_deg: f32,
    arc_radians: f32,
    radial: f32,
    jitter: Option<ConeJitter>,
) -> ([f32; 3], [f32; 3]) {
    let inner = (1.0 - thickness).max(MIN_INNER);
    let fraction = (inner * radial + (1.0 - radial)).sqrt();
    let (sin, cos) = engine_sincos(arc_radians);
    let x = fraction * cos;
    let y = fraction * sin;
    let (dx, dy) = match jitter {
        Some(j) if j.amount > 0.0 => {
            let (jitter_sin, jitter_cos) = engine_sincos(j.arc * NATIVE_TAU);
            let root = (j.area * MIN_INNER + (1.0 - j.area)).sqrt();
            (
                x + j.amount * (root * jitter_cos - x),
                y + j.amount * (root * jitter_sin - y),
            )
        }
        _ => (x, y),
    };
    let (sin_angle, cos_angle) = engine_sincos(angle_deg * DEG_TO_RAD);
    (
        [radius * x, radius * y, 0.0],
        [sin_angle * dx, sin_angle * dy, cos_angle],
    )
}

/// The arc clock ShapeModule::Update keeps for the Loop and PingPong arc
/// modes, in binary64: each ordinary update slice the previous value takes the
/// current one and the current one grows by the arc speed times the slice dt
/// (that product in f32, then widened). A reset of the system seeds zeroes
/// both.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArcLoopClock {
    pub current: f64,
    pub previous: f64,
}

impl ArcLoopClock {
    pub fn advance(&mut self, speed: f32, dt: f32) {
        self.previous = self.current;
        self.current += (dt * speed) as f64;
    }
}

/// StartCone's Loop arc angle for one lane, in radians. The lane's fraction of
/// the update slice is the emission spacing times its lane index, clamped to
/// [0, 1] with the native maximum and minimum; the angle clock interpolates
/// from the current value (fraction zero) to the previous one (fraction one),
/// both doubled and scaled by pi as rounded to f32, all in binary64. A positive
/// arc-spread step (arc in radians times spread, in f32) floors the clock to
/// its multiple. The clock is then reduced modulo the arc in radians with the
/// C remainder (fmod, exact), narrowed to f32, and a negative remainder is
/// moved up by the arc; an arc of zero gives zero. The remainder keeps the
/// sign of the time, so the arc's sign enters only through the step (a
/// negative arc quantizes only with a negative spread, and a negative spread
/// with a positive arc is no spread) and through that correction.
pub fn loop_arc_angle(
    clock: ArcLoopClock,
    spacing: f32,
    lane_index: f32,
    arc_deg: f32,
    arc_spread: f32,
) -> f32 {
    let arc = arc_deg * DEG_TO_RAD;
    let time = arc_clock_time(clock, spacing, lane_index, arc * arc_spread);
    let angle = if arc != 0.0 { (time % arc as f64) as f32 } else { 0.0 };
    if angle >= 0.0 {
        angle
    } else {
        arc + angle
    }
}

/// The lane's arc clock time the Loop and PingPong arc modes share, in
/// binary64: the lane's fraction of the slice (spacing times lane index,
/// clamped to [0, 1] with the native maximum and minimum) interpolates from
/// the current clock (fraction zero) to the previous one (fraction one), each
/// doubled and scaled by pi as rounded to f32; a positive arc-spread step (in
/// f32) floors the time to its multiple.
fn arc_clock_time(clock: ArcLoopClock, spacing: f32, lane_index: f32, step: f32) -> f64 {
    const PI_F32_WIDENED: f64 = std::f32::consts::PI as f64;
    let fraction = arm_fmin(arm_fmax(spacing * lane_index, 0.0), 1.0) as f64;
    let current = (clock.current + clock.current) * PI_F32_WIDENED;
    let previous = (clock.previous + clock.previous) * PI_F32_WIDENED;
    let time = previous * fraction + current * (1.0 - fraction);
    if step > 0.0 {
        let step = step as f64;
        (time / step).floor() * step
    } else {
        time
    }
}

/// StartCone's PingPong arc angle for one lane, in radians. Below an arc of
/// 1e-6 radians in magnitude (the engine's own threshold; a NaN arc is not
/// below it) every lane sits at +0 and the clock is not read. Otherwise the
/// lane's arc clock time (shared with the Loop mode, spread step included) is
/// scaled by the ARM reciprocal estimate of the arc in radians, refined twice,
/// widened to binary64, and reduced modulo 2 with the C remainder (fmod,
/// exact); the remainder is narrowed to f32 and taken in magnitude. A
/// magnitude of one or more folds back as (2 - p) - 1e-6, then only the
/// fractional part is kept (the whole part through the saturating
/// conversion, one less where that overshoots), and the arc in radians scales
/// it. So the angle sweeps up the arc and back down, the sign of the arc
/// giving the side.
pub fn pingpong_arc_angle(
    clock: ArcLoopClock,
    spacing: f32,
    lane_index: f32,
    arc_deg: f32,
    arc_spread: f32,
) -> f32 {
    let arc = arc_deg * DEG_TO_RAD;
    if arc.abs() < f32::from_bits(0x3586_37bd) {
        return 0.0;
    }
    let time = arc_clock_time(clock, spacing, lane_index, arc * arc_spread);
    let phase = ((time * arm_reciprocal(arc) as f64) % 2.0) as f32;
    let mut phase = phase.abs();
    if phase >= 1.0 {
        phase = (2.0 - phase) + f32::from_bits(0xb586_37bd);
    }
    let whole = (phase as i32) as f32;
    let whole = if whole > phase { whole - 1.0 } else { whole };
    arc * (phase - whole)
}

/// The ARM reciprocal estimate (FRECPE, flush-to-zero off, round to nearest)
/// refined by two FRECPS/FMUL steps, with the estimate kept for a zero input:
/// the sequence the engine's shape kernels divide by. Defined for every f32:
/// NaN gives NaN, an infinity a signed zero estimate, and a magnitude below
/// 2^-128 (zero included) a signed infinite one.
pub fn arm_reciprocal(value: f32) -> f32 {
    let estimate = arm_frecpe(value);
    if value == 0.0 {
        return estimate;
    }
    let first = estimate * arm_frecps(value, estimate);
    first * arm_frecps(value, first)
}

fn arm_frecpe(value: f32) -> f32 {
    let bits = value.to_bits();
    let sign = bits & 0x8000_0000;
    let magnitude = bits & 0x7fff_ffff;
    if value.is_nan() {
        return f32::NAN;
    }
    if magnitude == 0x7f80_0000 {
        return f32::from_bits(sign);
    }
    if magnitude < 0x0020_0000 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let mut exponent = (magnitude >> 23) as i32;
    let mut fraction = magnitude & 0x007f_ffff;
    if exponent == 0 {
        // A subnormal input is normalized by one or two places first.
        if fraction & 0x0040_0000 == 0 {
            exponent = -1;
            fraction = (fraction << 2) & 0x007f_ffff;
        } else {
            fraction = (fraction << 1) & 0x007f_ffff;
        }
    }
    let index = 256 + (fraction >> 15);
    // The 8-bit table of the architecture; integer divisions are floors.
    let estimate = ((1_u32 << 19) / (2 * index + 1) + 1) / 2;
    let mut result_exponent = 253 - exponent;
    let mut result_fraction = (estimate & 0xff) << 15;
    if result_exponent == 0 {
        result_fraction = 0x0040_0000 | (result_fraction >> 1);
    } else if result_exponent == -1 {
        result_fraction = 0x0020_0000 | (result_fraction >> 2);
        result_exponent = 0;
    }
    f32::from_bits(sign | ((result_exponent as u32) << 23) | result_fraction)
}

/// FRECPS: 2 - a * b with one rounding; an infinity times a zero gives 2, any
/// other infinite product the infinity of the opposite sign.
fn arm_frecps(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return f32::NAN;
    }
    if (a.is_infinite() && b == 0.0) || (a == 0.0 && b.is_infinite()) {
        return 2.0;
    }
    if a.is_infinite() || b.is_infinite() {
        return if a.is_sign_negative() != b.is_sign_negative() {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    // The product of two f32 values is exact in binary64. The subtraction is
    // rounded to binary64 with its exact error kept (two-sum); an inexact
    // result is moved to its odd neighbour toward the error (round to odd), so
    // the single narrowing to f32 rounds as the fused operation does.
    let product = a as f64 * b as f64;
    let sum = 2.0 - product;
    let virtual_b = sum - 2.0;
    let error = (2.0 - (sum - virtual_b)) + (-product - virtual_b);
    let sum = if error != 0.0 && sum.to_bits() & 1 == 0 {
        let bits = sum.to_bits();
        f64::from_bits(if (error > 0.0) == (sum > 0.0) { bits.wrapping_add(1) } else { bits.wrapping_sub(1) })
    } else {
        sum
    };
    sum as f32
}

/// ARM FMAX: NaN if either operand is NaN, +0 over -0.
fn arm_fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// ARM FMIN: NaN if either operand is NaN, -0 over +0.
fn arm_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// Box (the volume shape): three draws give the point (x - 0.5, y - 0.5,
/// z - 0.5) of the unit cube, emitted along local +Z. The kernel reads no
/// other shape member (radius, arc, angle, thickness, box thickness and both
/// modes are not loaded); the source affine does the sizing.
pub fn box_volume(x: f32, y: f32, z: f32) -> ([f32; 3], [f32; 3]) {
    ([x - 0.5, y - 0.5, z - 0.5], [0.0, 0.0, 1.0])
}

/// The two surface forms of the box, which ShapeModule::Start runs inline, one
/// body each, differing only in the per-axis select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSurface {
    /// BoxShell: the axis the face draw names takes its extreme, the other
    /// two keep their draws (a point on one of the six faces).
    Shell,
    /// BoxEdge: the axis the face draw names keeps its draw, the other two
    /// take their extremes (a point on one of the twelve edges).
    Edge,
}

/// One lane of BoxShell or BoxEdge, native operation order. Seven draws per
/// lane, in this order: the three coordinates `coordinate` (the unit
/// projection), the face as a raw word (taken modulo 3: 0 X, 1 Y, 2 Z), then
/// the three shell draws `shell`. Per axis the extreme is 1.0 where the
/// coordinate draw is at least 0.5 and 0.0 otherwise; the shell factor is
/// `(1 - box_thickness) * s + (1 - s)` as a multiply, a subtract and an add
/// (unfused); the local position is `factor * selected - 0.5`, which leaves
/// the lower extreme at -0.5 whatever the thickness. Emitted along local +Z.
/// Radius, arc, angle, thickness and both modes are not loaded.
pub fn box_surface(
    surface: BoxSurface, box_thickness: [f32; 3], coordinate: [f32; 3], face: u32, shell: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let face = face % 3;
    #[cfg(test)]
    let (surface, face, shell) = {
        let surface = if shell_arms::on("boxSurfaceSelectSwapped") {
            match surface { BoxSurface::Shell => BoxSurface::Edge, BoxSurface::Edge => BoxSurface::Shell }
        } else { surface };
        let face = if shell_arms::on("boxFaceAxesRotated") { (face + 1) % 3 } else { face };
        let shell = if shell_arms::on("boxShellDrawsReversed") { [shell[2], shell[1], shell[0]] } else { shell };
        (surface, face, shell)
    };
    let position = std::array::from_fn(|axis| {
        let u = coordinate[axis];
        let extreme = if u >= 0.5 { 1.0 } else { 0.0 };
        let named = face == axis as u32;
        let selected = match (surface, named) {
            (BoxSurface::Shell, true) | (BoxSurface::Edge, false) => extreme,
            (BoxSurface::Shell, false) | (BoxSurface::Edge, true) => u,
        };
        let k = 1.0 - box_thickness[axis];
        let s = shell[axis];
        #[cfg(test)]
        if shell_arms::on("boxShellFactorFused") {
            return k.mul_add(s, 1.0 - s) * selected - 0.5;
        }
        #[cfg(test)]
        if shell_arms::on("boxThicknessUnread") {
            return (s + (1.0 - s)) * selected - 0.5;
        }
        (k * s + (1.0 - s)) * selected - 0.5
    });
    (position, [0.0, 0.0, 1.0])
}

/// Test-only mutants of the shell preparation and the box surface kernels.
#[cfg(test)]
pub(crate) mod shell_arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub(crate) fn on(name: &str) -> bool {
        ARM.with(|arm| arm.get() == Some(name))
    }
    pub(crate) fn set(name: Option<&'static str>) {
        ARM.with(|arm| arm.set(name));
    }
    pub(crate) const ALL: [&str; 9] = ["log2fSubnormalUnscaled", "exp2fUnderflowBelow149", "shellBinary64Chain",
        "shellThicknessNotComplemented", "boxSurfaceSelectSwapped", "boxFaceAxesRotated", "boxShellDrawsReversed",
        "boxShellFactorFused", "boxThicknessUnread"];
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
    let z = z_random * 0.5 + 0.5;
    sphere_lane(radius, shell, random_arc(arc_deg, arc_spread, arc), (z + z) - 1.0, radial)
}

/// StartSphere in its Random arc mode, one lane, native operation order: the
/// Hemisphere lane with z = (draw + draw) - 1 over the whole sphere. Draws:
/// arc, z, radial fraction.
pub fn sphere_native(
    radius: f32,
    shell: Shell,
    arc_deg: f32,
    arc_spread: f32,
    arc: f32,
    z_random: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    sphere_lane(radius, shell, random_arc(arc_deg, arc_spread, arc), (z_random + z_random) - 1.0, radial)
}

fn sphere_lane(radius: f32, shell: Shell, angle: f32, z: f32, radial: f32) -> ([f32; 3], [f32; 3]) {
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

/// The BurstSpread divisor of StartSingleSidedEdge: the accepted batch count
/// less one, or one when that is zero.
pub fn edge_burst_divisor(accepted: std::num::NonZeroU32) -> std::num::NonZeroU32 {
    std::num::NonZeroU32::new(accepted.get() - 1).unwrap_or(std::num::NonZeroU32::MIN)
}

/// The BurstSpread divisor of StartCircle and StartCone: the accepted batch
/// count itself for an arc of exactly 360 degrees (the full circle, so the
/// last lane does not land on the first), otherwise the count less one, or
/// one when that is zero.
pub fn circle_burst_divisor(arc_deg: f32, accepted: std::num::NonZeroU32) -> std::num::NonZeroU32 {
    if arc_deg == 360.0 {
        accepted
    } else {
        edge_burst_divisor(accepted)
    }
}

/// The BurstSpread reciprocal of a divisor converted to f32: the ARM estimate
/// refined twice (the same FRECPE/FRECPS pair as the Initial lifetime
/// reciprocal). The divisor is at least one, so the estimate never meets
/// zero, infinity or a subnormal.
pub fn burst_spread_reciprocal(divisor: std::num::NonZeroU32) -> f32 {
    super::initial::initial_reciprocal(divisor.get() as f32).unwrap_or(f32::NAN)
}

/// StartCircle in its BurstSpread arc mode, one lane, with the lane's radial
/// draw: the arc angle is the arc in radians times (the batch reciprocal
/// times the lane's native index); a positive step (arc in radians times
/// spread) floors the angle to a multiple of the step with the saturating
/// conversion. The radial sample is the radius times the square root of the
/// squared inner radius (one less the thickness) plus the rest of the disk
/// times the draw.
pub fn circle_burst(
    radius: f32,
    thickness: f32,
    arc_deg: f32,
    arc_spread: f32,
    reciprocal: f32,
    lane_index: f32,
    radial: f32,
) -> ([f32; 3], [f32; 3]) {
    let angle = burst_spread_arc(arc_deg, arc_spread, reciprocal, lane_index);
    let inner = 1.0 - thickness;
    let inner_square = inner * inner;
    let rest = 1.0 - inner_square;
    let (sin, cos) = engine_sincos(angle);
    let sample = radius * (inner_square + rest * radial).sqrt();
    ([cos * sample, sample * sin, 0.0], [cos, sin, 0.0])
}

/// StartSingleSidedEdge in its BurstSpread radius mode, one lane, no draw: the
/// lane's native index (0, 1, 2, 3 from the first newborn group, plus 4.0 per
/// group, in f32) times the batch reciprocal, times the radius; a positive
/// step (radius times spread) floors it to a multiple of the step with the
/// saturating conversion; then doubled less the radius.
pub fn single_sided_edge_burst(radius: f32, spread: f32, reciprocal: f32, lane_index: f32) -> ([f32; 3], [f32; 3]) {
    let scaled = burst_spread_scaled(radius, spread, reciprocal, lane_index);
    ([(scaled + scaled) - radius, 0.0, 0.0], [0.0, 1.0, 0.0])
}

/// The BurstSpread arc angle of StartCircle and StartCone, one lane, in
/// radians: `burst_spread_scaled` over the arc in radians.
pub fn burst_spread_arc(arc_deg: f32, arc_spread: f32, reciprocal: f32, lane_index: f32) -> f32 {
    burst_spread_scaled(arc_deg * DEG_TO_RAD, arc_spread, reciprocal, lane_index)
}

/// The BurstSpread lane value the edge, circle and cone kernels share: the
/// extent (the edge radius, or the arc in radians) times (the batch
/// reciprocal times the lane's native index); a step (extent times spread)
/// that compares greater than zero floors it to a multiple of the step with
/// the saturating conversion, one less where the conversion overshoots. A
/// step that is not positive, NaN included, leaves it continuous.
fn burst_spread_scaled(extent: f32, spread: f32, reciprocal: f32, lane_index: f32) -> f32 {
    let scaled = extent * (reciprocal * lane_index);
    let step = extent * spread;
    if step > 0.0 {
        let steps = scaled / step;
        let whole = (steps as i32) as f32;
        step * if whole > steps { whole - 1.0 } else { whole }
    } else {
        scaled
    }
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
