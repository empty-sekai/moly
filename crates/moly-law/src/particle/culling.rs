//! Renderer-visibility culling of a particle system, in the engine's binary32
//! order: the local bounds `ParticleSystem::UpdateBounds` stores after an
//! update, the world box `ParticleSystemRenderer::CalculateWorldMatrixAndBoundsJob`
//! derives from them before a culling pass, and the camera test of that pass
//! (`ExtractProjectionPlanes`, `Camera::CalculateFrustumPlanes`,
//! `CullObjectsWithoutUmbra`, `IsNodeVisibleFast` and `IsNodeVisibleSlow`).
//!
//! Every function follows the engine's instruction order one operation at a
//! time, with the ARM semantics of each compare and each `fmin`/`fmax` (a NaN
//! operand propagates quieted, `-0` orders below `+0`). No product
//! multiplication is fused. The libm calls of the cone shape bounds
//! (`sincosf`, `sinf`) are the one place computed differently: in binary64,
//! rounded once to binary32.
//!
//! Coordinates are the engine's (left-handed) ones; a caller holding
//! reflected-X state converts at its boundary.
//!
//! What the functions refuse, they refuse by name ([`BoundsRefused`]):
//! `MinMaxCurve::FindMinMaxIntegrated` in curve modes (its cubic root solver
//! calls `pow`, `acos` and `cos`), the trail, lights, force and size-by-speed
//! contributions, the sprite-sheet factor, and a size module on 3D-size
//! particles whose Y/Z curves the caller does not carry.
use super::curve::{arm_fmax as fmax, arm_fmin as fmin, cache_coefficients};
use super::value::{CurveKey, MinMaxCurve};

const EPSILON: f32 = f32::from_bits(0x3727_c5ac);
const TINY_SQUARE: f32 = f32::from_bits(0x0da2_4260);
const SPEED_FLOOR: f32 = f32::from_bits(0x3586_37bd);
const PAD_FACTOR: f32 = f32::from_bits(0x3f35_c28f);
const SCALE_FLOOR: f32 = f32::from_bits(0x3089_705f);
const DEG2RAD: f32 = f32::from_bits(0x3c8e_fa35);
const INV_TWO_PI: f32 = f32::from_bits(0x3e22_f983);
const TENTH: f32 = f32::from_bits(0x3dcc_cccd);
const NEG_ZERO: f32 = f32::from_bits(0x8000_0000);
const QNAN: f32 = f32::from_bits(0x7fc0_0000);

/// `fcmp a, b` read with `mi`: true only for an ordered `a < b`.
fn lt(a: f32, b: f32) -> bool { a < b }
/// `fcmp a, b` read with `le`: true for `a <= b` and for unordered operands.
fn le_cc(a: f32, b: f32) -> bool { a.is_nan() || b.is_nan() || a <= b }
/// `fneg` + `fcmp #0` + `fcsel mi`: negate only a value below zero, so `-0`
/// and NaN keep their bits.
fn fabs_sel(v: f32) -> f32 { if v < 0.0 { -v } else { v } }

/// A box as the engine stores it after `UpdateBounds`: minimum then maximum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub fn words(&self) -> [u32; 6] {
        let [a, b, c] = self.min;
        let [d, e, f] = self.max;
        [a, b, c, d, e, f].map(f32::to_bits)
    }
}

/// A centre/extents box, the form the renderer and the culling pass use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CentreBox {
    pub centre: [f32; 3],
    pub extents: [f32; 3],
}

impl CentreBox {
    pub fn words(&self) -> [u32; 6] {
        let [a, b, c] = self.centre;
        let [d, e, f] = self.extents;
        [a, b, c, d, e, f].map(f32::to_bits)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsSpace {
    Local,
    World,
    Custom,
}

/// `ParticleSystemRenderer` render mode (renderer +0x1c8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsRenderMode {
    Billboard,
    Stretch,
    HorizontalBillboard,
    VerticalBillboard,
    Mesh,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsRefused {
    /// `FindMinMaxIntegrated` of a curve-mode velocity lane.
    IntegratedCurve,
    TrailModule,
    LightsModule,
    ForceModule,
    SizeBySpeedModule,
    /// A size module on 3D-size particles without its Y/Z curves.
    SizeAxisMissing,
    /// The sprite-sheet factor of the texture sheet module in Sprites mode.
    SpriteSheet,
    /// Stretched particles without their velocity, animated velocity and size.
    StretchInput,
}

impl std::fmt::Display for BoundsRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::IntegratedCurve => "curve-mode FindMinMaxIntegrated (libm cubic roots) is not transcribed",
            Self::TrailModule => "trail module bounds are not transcribed",
            Self::LightsModule => "lights module bounds are not transcribed",
            Self::ForceModule => "force module bounds are not transcribed",
            Self::SizeBySpeedModule => "size-by-speed bounds are not transcribed",
            Self::SizeAxisMissing => "size module Y/Z curves of 3D-size particles are not carried",
            Self::SpriteSheet => "sprite-sheet bounds factor is not transcribed",
            Self::StretchInput => "stretched particle bounds need velocity, animated velocity and size",
        })
    }
}

/// `ShapeModule` fields `CalculateProceduralBounds` reads. `kind` is the engine
/// shape type (0 Sphere, 2 Hemisphere, 4 Cone, 5 Box, 6 Mesh, 8 ConeVolume,
/// 10 Circle, 12 SingleSidedEdge, 17 Donut, ...).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeBounds {
    pub kind: u32,
    pub radius: f32,
    pub angle: f32,
    pub length: f32,
    pub donut_radius: f32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub random_direction: f32,
    /// The shape mesh `m_LocalAABB` as centre then extents; zero without a mesh.
    pub mesh: [f32; 6],
}

#[derive(Clone, Debug, PartialEq)]
pub struct VelocityBounds {
    pub x: MinMaxCurve,
    pub y: MinMaxCurve,
    pub z: MinMaxCurve,
    pub in_world_space: bool,
}

/// The serialized configuration `UpdateBounds` reads.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundsConfig {
    pub space: BoundsSpace,
    /// The system has a renderer component (`QueryComponentByType`).
    pub has_renderer: bool,
    pub render_mode: BoundsRenderMode,
    pub velocity_scale: f32,
    pub length_scale: f32,
    pub pivot: [f32; 3],
    /// `ParticleSystemRenderer::UpdateCachedMesh`: the union of the mesh slots'
    /// `m_LocalAABB` as minimum then maximum, `(+inf, -inf)` without a mesh.
    pub renderer_mesh: [f32; 6],
    pub lifetime: MinMaxCurve,
    pub speed: MinMaxCurve,
    /// Start size X, Y and Z; Y and Z are constant 1 when not authored.
    pub size: [MinMaxCurve; 3],
    /// The authored start size is 3D.
    pub start_size_3d: bool,
    /// `AllocateParticleArrays`' particle 3D-size flag: 3D start size, or an
    /// enabled size or size-by-speed module with separate axes.
    pub particle_size_3d: bool,
    /// The gravity modifier `MinMaxCurve` scalar.
    pub gravity_modifier: f32,
    /// Present when the Shape module is enabled.
    pub shape: Option<ShapeBounds>,
    /// Present when the Velocity module is enabled.
    pub velocity: Option<VelocityBounds>,
    /// Present when the Size module is enabled: the X curve, then the Y and Z
    /// curves when the export carries them.
    pub size_module: Option<[Option<MinMaxCurve>; 3]>,
    pub trail: bool,
    pub lights: bool,
    pub force: bool,
    pub size_by_speed: bool,
    /// The texture sheet module is in Sprites mode.
    pub uv_sprites: bool,
}

/// The per-update state `UpdateBounds` reads besides the configuration.
#[derive(Clone, Copy, Debug)]
pub struct BoundsState<'a> {
    /// supportsProcedural and not invalidateProcedural.
    pub procedural: bool,
    /// Column-major, as `Matrix4x4f`.
    pub local_to_world: [f32; 16],
    pub world_to_local: [f32; 16],
    pub shape_scale: [f32; 3],
    pub scale: [f32; 3],
    /// State +0x1b4, the external-emit size tracker.
    pub max_size_tracker: f32,
    pub gravity: [f32; 3],
    pub particles: Live<'a>,
}

/// The live particles the particle path reads. Only a stretched render mode
/// reads velocity, animated velocity and size.
#[derive(Clone, Copy, Debug, Default)]
pub struct Live<'a> {
    pub position: &'a [[f32; 3]],
    pub velocity: &'a [[f32; 3]],
    pub animated_velocity: &'a [[f32; 3]],
    pub size_x: &'a [f32],
}

// ---- MinMaxCurve ranges ----

fn min_max(range: (f32, f32), value: f32) -> (f32, f32) {
    (fmin(range.0, value), fmax(range.1, value))
}

/// The vector Horner evaluation of `CalculateCurveRangesValue`.
fn horner(c: [f32; 4], t: f32) -> f32 {
    let mut v = c[0] * t;
    v = c[1] + v;
    v = t * v;
    v = c[2] + v;
    v = t * v;
    c[3] + v
}

/// `CalculateCurveRangesValue`: from `range`, the first key value, then for
/// each segment the in-segment extrema of its cached cubic (roots of the
/// derivative `3a t^2 + 2b t + c`; the linear root when `|3a| < 1e-5`, none
/// when also `|2b| <= 1e-5`) and the segment end evaluated at `t1 - t0`.
/// Weights are not read.
pub fn curve_ranges_value(mut range: (f32, f32), keys: &[CurveKey]) -> (f32, f32) {
    if keys.is_empty() {
        return range;
    }
    range = min_max(range, keys[0].value);
    for pair in keys.windows(2) {
        let [a, b, c, d] = cache_coefficients(pair[0], pair[1]);
        let (t0, t1) = (pair[0].time, pair[1].time);
        let a3 = a * 3.0;
        let b2 = b + b;
        let mut roots = [f32::NAN; 2];
        let mut count = 0;
        if !lt(fabs_sel(a3), EPSILON) {
            let s4 = a3 * -4.0;
            let s3 = b2 * b2;
            let s0 = s4 * c;
            let disc = s3 + s0;
            if !lt(disc, 0.0) {
                let root = disc.sqrt();
                let inverse = 0.5 / a3;
                roots = [inverse * (root - b2), inverse * ((-root) - b2)];
                count = 2;
            }
        } else if !le_cc(fabs_sel(b2), EPSILON) {
            roots[0] = (-c) / b2;
            count = 1;
        }
        for &r in &roots[..count] {
            if !(r >= 0.0) || !lt(t0 + r, t1) {
                continue;
            }
            range = min_max(range, horner([a, b, c, d], r));
        }
        range = min_max(range, horner([a, b, c, d], t1 - t0));
    }
    range
}

/// The inlined `MinMaxCurve` range block of `UpdateBounds`.
pub fn curve_range(curve: &MinMaxCurve) -> (f32, f32) {
    match curve {
        MinMaxCurve::TwoConstants { min, max } => {
            if *max > *min { (*min, *max) } else { (*max, *min) }
        }
        MinMaxCurve::Constant(v) => {
            if le_cc(*v, 0.0) { (*v, 0.0) } else { (0.0, *v) }
        }
        MinMaxCurve::Curve { multiplier, max } => {
            let range = curve_ranges_value((f32::INFINITY, f32::NEG_INFINITY), &max.keys);
            (range.0 * multiplier, range.1 * multiplier)
        }
        MinMaxCurve::TwoCurves { multiplier, min, max } => {
            let range = curve_ranges_value((f32::INFINITY, f32::NEG_INFINITY), &max.keys);
            let range = curve_ranges_value(range, &min.keys);
            (range.0 * multiplier, range.1 * multiplier)
        }
    }
}

/// `MinMaxCurve::FindMinMaxIntegrated` for the constant modes.
fn integrated_range(curve: &MinMaxCurve) -> Result<(f32, f32), BoundsRefused> {
    match curve {
        MinMaxCurve::Constant(_) | MinMaxCurve::TwoConstants { .. } => Ok(curve_range(curve)),
        _ => Err(BoundsRefused::IntegratedCurve),
    }
}

// ---- Geometry helpers ----

fn quaternion_to_matrix(q: [f32; 4]) -> [f32; 16] {
    let [x, y, z, w] = q;
    let (s5, s6, s7) = (x + x, y + y, z + z);
    let s16 = x * s5;
    let s17 = y * s6;
    let s2 = z * s7;
    let s18 = x * s6;
    let s0 = x * s7;
    let s1 = y * s7;
    let s5 = s5 * w;
    let s6 = s6 * w;
    let s3 = w * s7;
    let s7 = s18 + s3;
    let s3 = s18 - s3;
    let s18 = s0 - s6;
    let s0 = s0 + s6;
    let s6 = s1 + s5;
    let s1 = s1 - s5;
    let s5 = s17 + s2;
    let s2 = s16 + s2;
    let s16 = s16 + s17;
    let s5 = 1.0 - s5;
    let s2 = 1.0 - s2;
    let s0b = 1.0 - s16;
    [s5, s7, s18, 0.0, s3, s2, s6, 0.0, s0, s1, s0b, 0.0, 0.0, 0.0, 0.0, 1.0]
}

fn set_trs(position: [f32; 3], q: [f32; 4], scale: [f32; 3]) -> [f32; 16] {
    let mut m = quaternion_to_matrix(q);
    for c in 0..3 {
        for r in 0..3 {
            m[c * 4 + r] = scale[c] * m[c * 4 + r];
        }
    }
    m[12] = position[0];
    m[13] = position[1];
    m[14] = position[2];
    m
}

fn set_tr(position: [f32; 3], q: [f32; 4]) -> [f32; 16] {
    let mut m = quaternion_to_matrix(q);
    m[12] = position[0];
    m[13] = position[1];
    m[14] = position[2];
    m
}

/// `TransformAABBSlow`: the eight corners through the affine part, reduced from
/// `(+inf, -inf)` with the engine's compare forms.
fn transform_aabb_slow(mn: [f32; 3], mx: [f32; 3], m: &[f32; 16]) -> ([f32; 3], [f32; 3]) {
    let corners = [
        [mn[0], mn[1], mn[2]], [mx[0], mn[1], mn[2]], [mx[0], mx[1], mn[2]], [mn[0], mx[1], mn[2]],
        [mn[0], mn[1], mx[2]], [mx[0], mn[1], mx[2]], [mx[0], mx[1], mx[2]], [mn[0], mx[1], mx[2]],
    ];
    let mut omn = [f32::INFINITY; 3];
    let mut omx = [f32::NEG_INFINITY; 3];
    for [x, y, z] in corners {
        let px = m[12] + ((m[0] * x + m[4] * y) + m[8] * z);
        let py = m[13] + ((m[1] * x + m[5] * y) + m[9] * z);
        let pz = m[14] + ((x * m[2] + y * m[6]) + z * m[10]);
        if lt(pz, omn[2]) { omn[2] = pz; }
        if omn[0] > px { omn[0] = px; }
        if omn[1] > py { omn[1] = py; }
        if px > omx[0] { omx[0] = px; }
        if py > omx[1] { omx[1] = py; }
        if lt(omx[2], pz) { omx[2] = pz; }
    }
    (omn, omx)
}

/// Round to an integer by adding and removing the signed 2^23.
fn round_magic(v: f32) -> f32 {
    let magic = f32::from_bits((v.to_bits() & 0x8000_0000) | 0x4b00_0000);
    (v + magic) - magic
}

/// The inlined shape Euler rotation (degrees) to quaternion of
/// `ShapeModule::CalculateProceduralBounds`: the float4 sincos polynomial on
/// half angles, the product in ZXY order with the translation unit's sign
/// constants, normalised with the identity below 1e-30.
fn euler_quaternion(rotation: [f32; 3], scale_x: f32) -> [f32; 4] {
    let lanes = [rotation[0], rotation[1], rotation[2], scale_x];
    let c4 = f32::from_bits(0x4299_2322);
    let c5 = f32::from_bits(0x42a3_3422);
    let c6 = f32::from_bits(0x4225_5ddc);
    let c7 = f32::from_bits(0x421e_a0cd);
    let two_pi = f32::from_bits(0x40c9_0fda);
    let mut sins = [0.0f32; 4];
    let mut coss = [0.0f32; 4];
    for (lane, &l) in lanes.iter().enumerate() {
        let v = ((l * DEG2RAD) * 0.5) * INV_TWO_PI;
        let v19 = v + -0.25;
        let a = (v - round_magic(v)).abs();
        let b = (v19 - round_magic(v19)).abs();
        let x = 0.25 - a;
        let y = 0.25 - b;
        let x2 = x * x;
        let y2 = y * y;
        let p3 = c5 - x2 * c4;
        let p4 = c5 - y2 * c4;
        let p5 = two_pi - x2 * c6;
        let x4 = x2 * x2;
        let p6 = two_pi - y2 * c6;
        let p3 = x4 * p3;
        let x8 = x4 * x4;
        let y4 = y2 * y2;
        let p3 = p5 + p3;
        let x8c = x8 * c7;
        let p4 = y4 * p4;
        let y8 = y4 * y4;
        let y8c = y8 * c7;
        let s_poly = x8c + p3;
        let c_poly = p6 + p4;
        sins[lane] = x * s_poly;
        let c_poly = y8c + c_poly;
        coss[lane] = y * c_poly;
    }
    let (sx, sy, sz) = (sins[0], sins[1], sins[2]);
    let (cx, cy, cz) = (coss[0], coss[1], coss[2]);
    let q16 = [1.0f32, -1.0, 1.0, 1.0];
    let q17 = [1.0f32, 1.0, -1.0, 1.0];
    let v2 = [sz * cx, cx * cz, sx * cz, sx * sz];
    let v2e = [v2[2], v2[3], v2[0], v2[1]];
    let mut q = [0.0f32; 4];
    for i in 0..4 {
        let v1 = q17[i] * cy;
        let v0 = v2[i] * sy;
        let v0 = q16[i] * v0;
        let v1 = v1 * v2e[i];
        q[i] = v0 + v1;
    }
    let sq = q.map(|v| v * v);
    let s = (sq[0] + sq[1]) + (sq[2] + sq[3]);
    if s > TINY_SQUARE {
        let r = s.sqrt();
        q.map(|v| v / r)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Device libm `sincosf`, computed in binary64 and rounded once.
fn libm_sin_cos(x: f32) -> (f32, f32) {
    let (s, c) = (x as f64).sin_cos();
    (s as f32, c as f32)
}

fn libm_sin(x: f32) -> f32 {
    (x as f64).sin() as f32
}

/// `ShapeModule::CalculateProceduralBounds` over the running box. `speed` is the
/// start speed range times the maximum lifetime; the random-direction branch
/// replaces it with its magnitudes in place.
fn shape_procedural_bounds(sh: &ShapeBounds, bmn: [f32; 3], bmx: [f32; 3], shape_scale: [f32; 3],
    speed: &mut [f32; 2]) -> ([f32; 3], [f32; 3]) {
    let t = sh.kind;
    let (r, ang, length, dr) = (sh.radius, sh.angle, sh.length, sh.donut_radius);
    let (mut mn, mut mx) = (bmn, bmx);
    match t {
        6 | 13 | 14 | 19 | 20 => {
            let c = [sh.mesh[0], sh.mesh[1], sh.mesh[2]];
            let e = [sh.mesh[3], sh.mesh[4], sh.mesh[5]];
            mn = [c[0] - e[0], c[1] - e[1], c[2] - e[2]];
            mx = [c[0] + e[0], c[1] + e[1], c[2] + e[2]];
        }
        5 | 15 | 16 => { mn = [-0.5; 3]; mx = [0.5; 3]; }
        0 => { mn = [-r, -r, -r]; mx = [r, r, r]; }
        2 => { mn = [-r, -r, 0.0]; mx = [r, r, r]; }
        4 => { mn = [-r, -r, NEG_ZERO]; mx = [r, r, 0.0]; }
        8 => {
            let (s, c) = libm_sin_cos(ang * DEG2RAD);
            let s0 = length * s;
            let s1 = length * c;
            let s0 = r + s0;
            mn = [-s0, -s0, NEG_ZERO];
            mx = [s0, s0, s1];
        }
        10 => { mn = [-r, -r, -TENTH]; mx = [r, r, TENTH]; }
        12 => { mn = [-r, -TENTH, -TENTH]; mx = [r, TENTH, TENTH]; }
        17 => {
            let s0 = r + dr;
            mn = [-s0, -s0, -dr];
            mx = [s0, s0, dr];
        }
        18 => { mn = [-0.5, -0.5, NEG_ZERO]; mx = [0.5, 0.5, 0.0]; }
        _ => {}
    }
    let q = euler_quaternion(sh.rotation, sh.scale[0]);
    let m = set_trs(sh.position, q, sh.scale);
    let (tmn, tmx) = transform_aabb_slow(mn, mx, &m);
    let mn = [tmn[0] * shape_scale[0], tmn[1] * shape_scale[1], tmn[2] * shape_scale[2]];
    let mx = [tmx[0] * shape_scale[0], tmx[1] * shape_scale[1], tmx[2] * shape_scale[2]];
    let mut dmn = [f32::INFINITY; 3];
    let mut dmx = [f32::NEG_INFINITY; 3];
    if !le_cc(sh.random_direction, 0.0) {
        match t {
            4 => {
                let s = libm_sin(ang * DEG2RAD);
                dmn = [-s, -s, 0.0];
                dmx = [s, s, 1.0];
            }
            7 => {}
            _ => {
                dmn = [-1.0; 3];
                dmx = [1.0; 3];
                if lt(speed[0], 0.0) { speed[0] = -speed[0]; }
                if lt(speed[1], 0.0) { speed[1] = -speed[1]; }
            }
        }
    } else {
        match t {
            0 | 6 | 10 | 13 | 14 | 17 => { dmn = [-1.0; 3]; dmx = [1.0; 3]; }
            2 => { dmn = [-1.0, -1.0, 0.0]; dmx = [1.0; 3]; }
            4 | 8 => {
                let s = libm_sin(ang * DEG2RAD);
                dmn = [-s, -s, 0.0];
                dmx = [s, s, 1.0];
            }
            5 | 15 | 16 | 18 | 19 | 20 => { dmn = [0.0; 3]; dmx = [0.0, 0.0, 1.0]; }
            12 => { dmn = [0.0; 3]; dmx = [0.0, 1.0, 0.0]; }
            _ => {}
        }
    }
    let m2 = set_tr([0.0; 3], q);
    let (dmn, dmx) = transform_aabb_slow(dmn, dmx, &m2);
    let s0 = speed[1];
    let mut mn = mn;
    let mut mx = mx;
    for i in 0..3 {
        let a = s0 * dmn[i] + mn[i];
        let b = s0 * dmx[i] + mx[i];
        if lt(a, mn[i]) { mn[i] = a; }
        if lt(mx[i], b) { mx[i] = b; }
    }
    let s20 = speed[0];
    for i in 0..3 {
        let lo = dmn[i] * s20;
        let hi = dmx[i] * s20;
        let pmin = if lt(hi, lo) { hi } else { lo };
        let pmax = if lt(lo, hi) { hi } else { lo };
        if lt(pmin, mn[i]) { mn[i] = pmin; }
        if lt(mx[i], pmax) { mx[i] = pmax; }
    }
    (mn, mx)
}

fn velocity_procedural_bounds(velocity: &VelocityBounds, world_to_local: &[f32; 16], lmax: f32)
    -> Result<([f32; 3], [f32; 3]), BoundsRefused> {
    let rx = integrated_range(&velocity.x)?;
    let ry = integrated_range(&velocity.y)?;
    let rz = integrated_range(&velocity.z)?;
    let mn = [lmax * rx.0, lmax * ry.0, rz.0 * lmax];
    let mx = [rx.1 * lmax, ry.1 * lmax, rz.1 * lmax];
    if velocity.in_world_space {
        let mut m = *world_to_local;
        m[12] = 0.0;
        m[13] = 0.0;
        m[14] = 0.0;
        return Ok(transform_aabb_slow(mn, mx, &m));
    }
    Ok((mn, mx))
}

// ---- FRSQRTE / FRSQRTS ----

fn recip_sqrt_estimate(a: u32) -> u32 {
    let a: u64 = if a < 256 { a as u64 * 2 + 1 } else { (((a >> 1) << 1) as u64 + 1) * 2 };
    let mut b: u64 = 512;
    while a * (b + 1) * (b + 1) < (1 << 28) {
        b += 1;
    }
    ((b + 1) / 2) as u32
}

/// ARM `FRSQRTE` on a single lane.
fn frsqrte(x: f32) -> f32 {
    let w = x.to_bits();
    let sign = w >> 31;
    let mut exp = ((w >> 23) & 0xff) as i32;
    let mut frac = w & 0x007f_ffff;
    if x.is_nan() {
        return QNAN;
    }
    if exp == 0 && frac == 0 {
        return if sign != 0 { f32::NEG_INFINITY } else { f32::INFINITY };
    }
    if sign != 0 {
        return QNAN;
    }
    if exp == 0xff {
        return 0.0;
    }
    if exp == 0 {
        while frac & 0x0040_0000 == 0 {
            frac = (frac << 1) & 0x007f_ffff;
            exp -= 1;
        }
        frac = (frac << 1) & 0x007f_ffff;
    }
    let scaled = if exp & 1 == 0 { 256 + (frac >> 15) } else { 128 + (frac >> 16) };
    let result_exp = ((380 - exp) / 2) as u32;
    let estimate = recip_sqrt_estimate(scaled);
    f32::from_bits(((result_exp & 0xff) << 23) | ((estimate & 0xff) << 15))
}

/// The neighbouring binary32 value towards `+inf` (`up`) or `-inf`.
fn step_f32(v: f32, up: bool) -> f32 {
    if v == 0.0 {
        let tiny = f32::from_bits(1);
        return if up { tiny } else { -tiny };
    }
    let bits = v.to_bits();
    let away = (v > 0.0) == up;
    f32::from_bits(if away { bits + 1 } else { bits - 1 })
}

/// ARM `FRSQRTS`: `(3 - a*b) / 2` with the product not rounded and one final
/// rounding to binary32.
fn frsqrts(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return QNAN;
    }
    if (a.is_infinite() && b == 0.0) || (b.is_infinite() && a == 0.0) {
        return 1.5;
    }
    if a.is_infinite() || b.is_infinite() {
        let negative_product = a.is_sign_negative() != b.is_sign_negative();
        return if negative_product { f32::INFINITY } else { f32::NEG_INFINITY };
    }
    // The binary32 product is exact in binary64; the difference is s + e exactly.
    let p = a as f64 * b as f64;
    let s = 3.0 - p;
    let bb = s - 3.0;
    let e = (3.0 - (s - bb)) + (-p - bb);
    let (r, re) = (s * 0.5, e * 0.5);
    let f = r as f32;
    if re == 0.0 || f.is_infinite() || f as f64 == r {
        return f;
    }
    // A binary64 value can round differently from the exact sum only at a
    // binary32 midpoint; there the sign of the lost part decides.
    let other = step_f32(f, (f as f64) < r);
    let midpoint = (f as f64 + other as f64) * 0.5;
    if r != midpoint {
        return f;
    }
    let upper = if (f as f64) > (other as f64) { f } else { other };
    let lower = if (f as f64) > (other as f64) { other } else { f };
    if re > 0.0 { upper } else { lower }
}

// ---- UpdateBounds ----

/// `ParticleSystem::UpdateBounds`: the local box the renderer reads.
///
/// The procedural path (supportsProcedural and not invalidateProcedural)
/// bounds every particle the system can emit from its modules; otherwise the
/// box is the live particles' (with the tails of stretched particles), or the
/// emitter position plus and minus 1e-5 without padding when none is alive.
/// Both non-empty paths are padded by the largest particle size, the pivot,
/// 0.71 and, in World or Custom space, the largest emitter scale.
pub fn update_bounds(config: &BoundsConfig, state: &BoundsState<'_>) -> Result<Bounds, BoundsRefused> {
    if config.trail { return Err(BoundsRefused::TrailModule); }
    if config.lights { return Err(BoundsRefused::LightsModule); }
    if config.force { return Err(BoundsRefused::ForceModule); }
    let render_mode = if config.has_renderer { config.render_mode } else { BoundsRenderMode::Billboard };
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    let live = state.particles;
    let count = live.position.len();
    if !state.procedural {
        if count == 0 {
            let base = if config.space == BoundsSpace::World {
                [state.local_to_world[12], state.local_to_world[13], state.local_to_world[14]]
            } else {
                [0.0; 3]
            };
            return Ok(Bounds {
                min: [base[0] - EPSILON, base[1] - EPSILON, base[2] - EPSILON],
                max: [EPSILON + base[0], EPSILON + base[1], EPSILON + base[2]],
            });
        }
        for axis in 0..3 {
            for p in live.position {
                mn[axis] = fmin(mn[axis], p[axis]);
                mx[axis] = fmax(mx[axis], p[axis]);
            }
        }
        if render_mode == BoundsRenderMode::Stretch {
            if live.velocity.len() != count || live.animated_velocity.len() != count || live.size_x.len() != count {
                return Err(BoundsRefused::StretchInput);
            }
            let (vs, ls) = (config.velocity_scale, config.length_scale);
            for i in 0..count {
                let v = [0, 1, 2].map(|a| live.velocity[i][a] + live.animated_velocity[i][a]);
                let sq = v[0] * v[0] + (v[1] * v[1] + v[2] * v[2]);
                let e = frsqrte(sq);
                let t1 = e * sq;
                let t1 = frsqrts(t1, e);
                let t1 = e * t1;
                let t2 = t1 * sq;
                let t2 = frsqrts(t2, t1);
                let t1 = t1 * t2;
                let mut rs = if sq == 0.0 { e } else { t1 };
                if TINY_SQUARE >= sq { rs = 0.0; }
                let k = ls * rs;
                let k = live.size_x[i] * k;
                let k = vs + k;
                for a in 0..3 {
                    let end = live.position[i][a] - v[a] * k;
                    mn[a] = fmin(mn[a], end);
                    mx[a] = fmax(mx[a], end);
                }
            }
        }
    } else {
        let life = curve_range(&config.lifetime);
        let speed = curve_range(&config.speed);
        let lmax = life.1;
        let s0 = speed.0 * lmax;
        let s1 = speed.1 * lmax;
        let zero = 0.0f32;
        let mn0 = fmin(zero, 0.0);
        let mn1 = fmin(zero, 0.0);
        let mx0 = fmax(zero, 0.0);
        let mx1 = fmax(zero, 0.0);
        let mut s4 = if lt(s0, zero) { s0 } else { zero };
        let mut s3 = if lt(zero, s0) { s0 } else { zero };
        if lt(s1, s4) { s4 = s1; }
        if lt(s3, s1) { s3 = s1; }
        mn = [mn0, mn1, s4];
        mx = [mx0, mx1, s3];
        let mut speed_io = [s0, s1];
        if let Some(shape) = &config.shape {
            (mn, mx) = shape_procedural_bounds(shape, mn, mx, state.shape_scale, &mut speed_io);
        }
        let g = state.gravity;
        let gm = config.gravity_modifier;
        let v0 = g.map(|gi| lmax * gi).map(|vi| lmax * vi).map(|vi| vi * 0.5);
        let mut d = [v0[0] * gm, v0[1] * gm, gm * v0[2]];
        if config.space != BoundsSpace::World {
            let m = &state.world_to_local;
            let [dx, dy, dz] = d;
            let dxy0 = m[0] * dx + m[4] * dy;
            let dxy1 = m[1] * dx + m[5] * dy;
            let dzz = m[2] * dx + m[6] * dy;
            let dxy0 = dxy0 + m[8] * dz;
            let dxy1 = dxy1 + m[9] * dz;
            let dzz = dzz + dz * m[10];
            d = [dxy0, dxy1, dzz];
        }
        let pos_d = [
            if 0.0 > d[0] { 0.0 } else { d[0] },
            if 0.0 > d[1] { 0.0 } else { d[1] },
            if lt(d[2], 0.0) { 0.0 } else { d[2] },
        ];
        mx = [pos_d[0] + mx[0], pos_d[1] + mx[1], pos_d[2] + mx[2]];
        let neg_d = [
            if d[0] > 0.0 { 0.0 } else { d[0] },
            if d[1] > 0.0 { 0.0 } else { d[1] },
            if lt(0.0, d[2]) { 0.0 } else { d[2] },
        ];
        mn = [neg_d[0] + mn[0], neg_d[1] + mn[1], neg_d[2] + mn[2]];
        let (vmn, vmx) = match &config.velocity {
            Some(velocity) => velocity_procedural_bounds(velocity, &state.world_to_local, lmax)?,
            None => ([0.0; 3], [0.0; 3]),
        };
        let a = [mn[0] + vmn[0], mn[1] + vmn[1], mn[2] + vmn[2]];
        let b = [mx[0] + vmx[0], mx[1] + vmx[1], mx[2] + vmx[2]];
        if mn[0] > a[0] { mn[0] = a[0]; }
        if mn[1] > a[1] { mn[1] = a[1]; }
        if lt(a[2], mn[2]) { mn[2] = a[2]; }
        if b[0] > mx[0] { mx[0] = b[0]; }
        if b[1] > mx[1] { mx[1] = b[1]; }
        if lt(mx[2], b[2]) { mx[2] = b[2]; }
        // The disabled force module contributes an all-zero box.
        for i in 0..3 {
            let a = mn[i] + 0.0;
            let b = mx[i] + 0.0;
            if lt(a, mn[i]) { mn[i] = a; }
            if lt(mx[i], b) { mx[i] = b; }
        }
        if render_mode == BoundsRenderMode::Stretch {
            let size = curve_range(if config.start_size_3d { &config.size[1] } else { &config.size[0] });
            let mut vs = config.velocity_scale.abs();
            let smax = speed.1;
            if !le_cc(smax, SPEED_FLOOR) {
                let t = config.length_scale.abs() * size.1;
                let t = t / smax;
                vs = vs + t;
            }
            let k = smax * vs;
            mn = [mn[0] - k, mn[1] - k, mn[2] - k];
            mx = [mx[0] + k, k + mx[1], k + mx[2]];
        }
    }
    // ---- tail padding
    let mut mesh = [1.0f32; 3];
    if config.has_renderer {
        let rb = config.renderer_mesh;
        let skip = (rb[0] == f32::INFINITY && rb[1] == f32::INFINITY && rb[2] == f32::INFINITY)
            || (rb[3] == f32::NEG_INFINITY && rb[4] == f32::NEG_INFINITY && rb[5] == f32::NEG_INFINITY);
        if !skip {
            let a = [fabs_sel(rb[0]), fabs_sel(rb[1]), fabs_sel(rb[2])];
            let b = [fabs_sel(rb[3]), fabs_sel(rb[4]), fabs_sel(rb[5])];
            let m = [0, 1, 2].map(|i| if lt(a[i], b[i]) { b[i] } else { a[i] });
            let m = m.map(|v| v + v);
            mesh = m;
            if !config.particle_size_3d {
                let m1b = if lt(m[1], m[2]) { m[2] } else { m[1] };
                mesh[0] = if lt(m[0], m1b) { m1b } else { m[0] };
            }
        }
    }
    let axes = if config.particle_size_3d { 3 } else { 1 };
    let mut s8 = 0.0f32;
    for axis in 0..axes {
        let start = curve_range(&config.size[axis]);
        let mut s10 = match &config.size_module {
            Some(curves) => {
                let curve = curves[axis].as_ref().ok_or(BoundsRefused::SizeAxisMissing)?;
                let module = curve_range(curve);
                start.1 * module.1
            }
            None => start.1,
        };
        if config.size_by_speed {
            return Err(BoundsRefused::SizeBySpeedModule);
        }
        if render_mode == BoundsRenderMode::Mesh {
            s10 = s10 * mesh[axis];
        }
        if lt(s8, s10) { s8 = s10; }
    }
    let tracker = state.max_size_tracker;
    let mut pad = if lt(s8, tracker) { tracker } else { s8 };
    if config.has_renderer {
        let piv = config.pivot.map(f32::abs);
        let mp = fmax(fmax(piv[0], piv[1]), fmax(piv[2], piv[0]));
        let k = pad * mp;
        mn = [mn[0] - k, mn[1] - k, mn[2] - k];
        mx = [mx[0] + k, k + mx[1], k + mx[2]];
    }
    pad = pad * PAD_FACTOR;
    if matches!(config.space, BoundsSpace::World | BoundsSpace::Custom) {
        let sc = state.scale.map(fabs_sel);
        let s2 = if lt(sc[1], sc[2]) { sc[2] } else { sc[1] };
        let s1 = if lt(sc[0], s2) { s2 } else { sc[0] };
        pad = pad * s1;
    }
    if config.uv_sprites {
        return Err(BoundsRefused::SpriteSheet);
    }
    Ok(Bounds {
        min: [mn[0] - pad, mn[1] - pad, mn[2] - pad],
        max: [mx[0] + pad, pad + mx[1], pad + mx[2]],
    })
}

// ---- Renderer world box ----

/// The bounds part of `ParticleSystemRenderer::CalculateWorldMatrixAndBoundsJob`
/// for a renderer without custom bounds and without a transform change this
/// frame: the world box (also the scene entry) and the local box.
pub fn world_bounds(bounds: &Bounds, space: BoundsSpace, alignment: u32, local_to_world: &[f32; 16],
    world_to_local: &[f32; 16], scale: [f32; 3]) -> (CentreBox, CentreBox) {
    let (mn, mx) = (bounds.min, bounds.max);
    let cen = [0, 1, 2].map(|i| (mn[i] + mx[i]) * 0.5);
    let ext = [0, 1, 2].map(|i| (mx[i] - mn[i]) * 0.5);
    let apply = |m: &[f32; 16], k: [[f32; 3]; 3]| -> CentreBox {
        let c0 = [m[0], m[1], m[2]];
        let c1 = [m[4], m[5], m[6]];
        let c2 = [m[8], m[9], m[10]];
        let t = [m[12], m[13], m[14]];
        let mut centre = [0.0f32; 3];
        let mut extents = [0.0f32; 3];
        for i in 0..3 {
            let v16 = c0[i] * cen[0];
            let v18 = c2[i] * cen[2];
            let v7 = c1[i] * cen[1];
            let e0 = k[0][i] * ext[0];
            let e1 = k[1][i] * ext[1];
            let e2 = k[2][i] * ext[2];
            let v7 = v7 + v18;
            let v7 = v16 + v7;
            let ev = e0.abs() + e1.abs();
            centre[i] = t[i] + v7;
            extents[i] = ev + e2.abs();
        }
        CentreBox { centre, extents }
    };
    let columns = |m: &[f32; 16]| [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]];
    let own = CentreBox { centre: cen, extents: ext };
    if space == BoundsSpace::World {
        return (own, apply(world_to_local, columns(world_to_local)));
    }
    let mut k = columns(local_to_world);
    if !(space == BoundsSpace::Custom || alignment > 3 || alignment == 2) {
        let aa = scale.map(f32::abs);
        let mm = fmax(fmax(aa[0], aa[1]), fmax(aa[2], aa[0]));
        for c in 0..3 {
            let f = if aa[c] > SCALE_FLOOR { mm / scale[c] } else { 1.0 };
            k[c] = k[c].map(|v| v * f);
        }
    }
    (apply(local_to_world, k), own)
}

// ---- Camera planes and the culling tests ----

/// A plane as `(normal, distance)`: a point is inside when `n.p + d >= 0`.
pub type Plane = [f32; 4];

/// `ExtractProjectionPlanes`: `row3 + row_i` and `row3 - row_i` of the column-
/// major matrix for rows 0, 1 and 2, each normalised by `1 / sqrt(|n|^2)`.
pub fn extract_projection_planes(m: &[f32; 16]) -> [Plane; 6] {
    let r3 = [m[3], m[7], m[11], m[15]];
    let mut out = [[0.0f32; 4]; 6];
    for row in 0..3 {
        let r = [m[row], m[row + 4], m[row + 8], m[row + 12]];
        let a = [r3[0] + r[0], r3[1] + r[1], r3[2] + r[2]];
        let b = [r3[0] - r[0], r3[1] - r[1], r3[2] - r[2]];
        let la = (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2];
        let lb = (b[0] * b[0] + b[1] * b[1]) + b[2] * b[2];
        let ia = 1.0 / la.sqrt();
        let ib = 1.0 / lb.sqrt();
        let da = r3[3] + r[3];
        let db = r3[3] - r[3];
        out[row * 2] = [a[0] * ia, a[1] * ia, a[2] * ia, da * ia];
        out[row * 2 + 1] = [b[0] * ib, b[1] * ib, b[2] * ib, db * ib];
    }
    out
}

/// The camera inputs `Camera::CalculateFrustumPlanes` reads.
#[derive(Clone, Copy, Debug)]
pub struct FrustumCamera {
    /// No custom culling matrix is set.
    pub implicit_culling: bool,
    /// No custom view matrix is set, or the caller asks for implicit near/far.
    pub implicit_view: bool,
    /// The camera-to-world matrix, the inverse of scale(1,1,-1) times the
    /// camera's world-to-local matrix without scale.
    pub camera_to_world: [f32; 16],
    pub near: f32,
    pub far: f32,
}

/// `Camera::CalculateFrustumPlanes`: the planes of the culling matrix; with an
/// implicit culling and view matrix, the near and far planes rebuilt from the
/// camera position and forward axis. Returns the planes and the far base the
/// layer distances add to.
pub fn frustum_planes(culling: &[f32; 16], camera: &FrustumCamera) -> ([Plane; 6], f32) {
    let mut p = extract_projection_planes(culling);
    if !(camera.implicit_culling && camera.implicit_view) {
        return (p, p[5][3] - camera.far);
    }
    let inv = &camera.camera_to_world;
    let (mut fx, mut fy, mut fz) = (inv[8], inv[9], inv[10]);
    let (px, py, pz) = (inv[12], inv[13], inv[14]);
    let ln = ((fx * fx + fy * fy) + fz * fz).sqrt();
    if !le_cc(ln, EPSILON) {
        fx = fx / ln;
        fy = fy / ln;
        fz = fz / ln;
    } else {
        // The translation unit's default axis is all zero.
        (fx, fy, fz) = (0.0, 0.0, 0.0);
    }
    let s7 = -fy;
    let s4 = px * fx;
    let s17 = py * fy;
    let s0 = py * s7;
    let s2 = pz * fz;
    let s0 = s0 - s4;
    let s6 = -fx;
    let s0 = s0 - s2;
    let s16 = -fz;
    let near_normal = [s6, s7];
    let s6 = s4 + s17;
    let s0 = -s0;
    let s4 = s6 + s2;
    let s2 = -s4;
    let s0 = s0 - camera.near;
    p[4] = [near_normal[0], near_normal[1], s16, s0];
    p[5] = [fx, fy, fz, s2 + camera.far];
    (p, s2)
}

/// `CullObjectsWithoutUmbra` with `IntersectAABBPlaneBoundsOptimized`: the box
/// is outside when `n.c + d + |n|.e < 0` for any plane, the planes taken in
/// groups of four with the last group padded by the last plane. No plane keeps
/// every box.
pub fn inside_planes(aabb: &CentreBox, planes: &[Plane]) -> bool {
    let n = planes.len();
    if n < 1 {
        return true;
    }
    let (c, e) = (aabb.centre, aabb.extents);
    let groups = n.div_ceil(4);
    for g in 0..groups {
        for k in 0..4 {
            let plane = planes[(g * 4 + k).min(n - 1)];
            let (nx, ny, nz, d) = (plane[0], plane[1], plane[2], plane[3]);
            let v6 = c[0] * nx;
            let v6 = v6 + d;
            let v6 = (c[1] * ny) + v6;
            let v6 = (c[2] * nz) + v6;
            let v7 = (e[0] * nx.abs()) + (e[1] * ny.abs());
            let v7 = v7 + e[2] * nz.abs();
            let v6 = v7 + v6;
            if v6 < 0.0 {
                return false;
            }
        }
    }
    true
}

/// `IntersectAABBPlaneBounds`, the per-plane form `IsNodeVisibleSlow` uses.
pub fn inside_planes_slow(aabb: &CentreBox, planes: &[Plane]) -> bool {
    let (c, e) = (aabb.centre, aabb.extents);
    for plane in planes {
        let (nx, ny, nz, d) = (plane[0], plane[1], plane[2], plane[3]);
        let s18 = nx * c[0];
        let s19 = ny * c[1];
        let s18 = s18 + s19;
        let s19 = nz * c[2];
        let s18 = s18 + s19;
        let s17 = d + s18;
        let (a0, a1, a2) = (fabs_sel(nx), fabs_sel(ny), fabs_sel(nz));
        let s6 = a0 * e[0];
        let s7 = a1 * e[1];
        let s6 = s6 + s7;
        let s7 = a2 * e[2];
        let s6 = s6 + s7;
        let s6 = s17 + s6;
        if s6 < 0.0 {
            return false;
        }
    }
    true
}

/// The layer-distance mode of `IsNodeVisibleSlow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerCull {
    None,
    /// One plane: the far normal with the layer's distance.
    Planar,
    /// The distance from the camera position to the box centre; a zero
    /// distance never culls.
    Spherical,
}

/// `IsNodeVisibleSlow` for one node on `layer`.
pub fn layer_visible(aabb: &CentreBox, mode: LayerCull, far_normal: [f32; 3], distance: f32,
    camera_position: [f32; 3]) -> bool {
    match mode {
        LayerCull::None => true,
        LayerCull::Planar => {
            inside_planes_slow(aabb, &[[far_normal[0], far_normal[1], far_normal[2], distance]])
        }
        LayerCull::Spherical => {
            if distance == 0.0 {
                return true;
            }
            let (c, p) = (aabb.centre, camera_position);
            let s0 = distance * distance;
            let s1 = c[0] - p[0];
            let s1 = s1 * s1;
            let y = c[1] - p[1];
            let z = c[2] - p[2];
            let y = y * y;
            let z = z * z;
            let s1 = s1 + y;
            let s1 = s1 + z;
            !(s1 > s0)
        }
    }
}

/// The node fields `IsNodeVisibleFast` and the step-1 list filter read.
#[derive(Clone, Copy, Debug)]
pub struct CullNode {
    pub has_renderer: bool,
    pub layer: u32,
    /// Bits 30-31 skip the node when both are set; bit 29 hides it; the low 28
    /// bits index the LOD byte table (0 for none).
    pub flags: u32,
    pub renderer_disabled: bool,
    pub lod_mask: u8,
}

/// `ProcessCameraIndexListIsNodeVisibleStep1` for one node.
pub fn node_visible_fast(node: &CullNode, culling_mask: u32, lod: &[u8]) -> bool {
    if node.flags >> 30 > 2 {
        return false;
    }
    if (culling_mask >> (node.layer & 31)) & 1 == 0 || !node.has_renderer
        || node.flags & (1 << 29) != 0 || node.renderer_disabled {
        return false;
    }
    let index = (node.flags & 0x0fff_ffff) as usize;
    if index != 0 {
        return lod.get(index).is_some_and(|byte| byte & node.lod_mask != 0);
    }
    true
}

/// One camera's verdict for a world box: the frustum planes, then the planar
/// layer test with the layer distance `far base + far` (a zero
/// layerCullDistance).
pub fn visible_to_camera(aabb: &CentreBox, planes: &[Plane; 6], far_base: f32, far: f32) -> bool {
    if !inside_planes(aabb, planes) {
        return false;
    }
    let far_normal = [planes[5][0], planes[5][1], planes[5][2]];
    layer_visible(aabb, LayerCull::Planar, far_normal, far_base + far, [0.0; 3])
}

#[cfg(test)]
#[path = "culling_samples.rs"]
mod culling_samples;
