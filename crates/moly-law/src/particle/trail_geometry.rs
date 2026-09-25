//! Per-particle trail geometry, as the renderer's trail geometry job builds it
//! for a per-particle trail with the fixed vertex layout (position, RGBA32
//! colour, UV): per particle a line strip of the live particle position (the
//! head) followed by its recorded points newest first, the oldest point pulled
//! toward the next one to the trail lifetime, two vertices per point offset
//! by half the width along the view-space normal, and six indices per
//! segment. The strip is built in view space and brought back with the
//! approximate inverse of the view the renderer computes, so the vertices are
//! world positions. A particle with no recorded point draws nothing.
//!
//! Inputs are the view (world to camera), the simulation owner (identity for
//! World simulation), the trail module's clock and the particles. Everything
//! outside the transcribed composition is refused, never approximated.
use super::armf as a;
use super::gradient::{time_code, GradientMode};
use super::random::ParticleRandom;
use super::schema::{TrailParams, TrailTextureMode};
use super::trail::{trail_lifetime, Refused, TrailClock, TrailRecording, TrailRing, TrailSize};
use super::{Gradient, MinMaxCurve, MinMaxGradient};

const ONE: f32 = 1.0;
const HALF: f32 = 0.5;
const NEG_HALF: f32 = -0.5;
const NEG_CENTI: f32 = f32::from_bits(0xbc23_d70a);
const CENTI: f32 = f32::from_bits(0x3c23_d70a);
const THIRD_APPROX: f32 = f32::from_bits(0x3eaa_aa9f);
const TINY_SCALE: f32 = f32::from_bits(0x0da2_4260);
const RSQ_BIAS: f32 = f32::from_bits(0x3f80_4020);
const DET_EPS: f32 = f32::from_bits(0x3586_37bd);
const TINY_SEGMENT: [f32; 2] = [f32::from_bits(0x0da2_4260), f32::from_bits(0x3727_c5ac)];
const SLOPE_EPS: f32 = f32::from_bits(0x3727_c5ac);
const SLOPE_CLAMP: f32 = f32::from_bits(0x47c3_5000);
const FIFTY: f32 = 50.0;
const THREE: f32 = 3.0;
const EPS_SPAN: f32 = f32::from_bits(0x3586_37bd);
const TIME_SCALE: f32 = f32::from_bits(0x477f_ff00);
const DIR_FALLBACK: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
const WIDTH_SALT: u32 = 0xfedc_345b;
const TRAIL_COLOUR_SALT: u32 = 0x6cf2_ac20;
const COLOUR_SALT: u32 = 0x591b_c05c;
const HUNDRED: f32 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Width {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
}

impl Width {
    fn evaluate(self, random: f32) -> f32 {
        match self {
            Self::Constant(v) => v,
            Self::TwoConstants { min, max } => a::add(min, a::mul(random, a::sub(max, min))),
        }
    }
}

/// The geometry half of an authored TrailModule (the recording half included,
/// for the tail clip's lifetime).
#[derive(Clone, Debug, PartialEq)]
pub struct TrailGeometryLaw {
    pub recording: TrailRecording,
    texture_mode: TrailTextureMode,
    texture_scale: [f32; 2],
    shadow_bias: f32,
    world_space: bool,
    size_affects_width: bool,
    inherit_particle_color: bool,
    color_over_lifetime: MinMaxGradient,
    width_over_trail: Width,
    color_over_trail: MinMaxGradient,
}

/// The colour dispatch evaluates a two-gradient block whose minimum is Fixed
/// and maximum Blend through a perceptual kernel (device libm).
pub fn check_gradient(block: &MinMaxGradient) -> Result<(), Refused> {
    match block {
        MinMaxGradient::TwoGradients { min, max }
            if min.mode == GradientMode::Fixed && max.mode == GradientMode::Blend =>
        {
            Err(Refused::PerceptualGradient)
        }
        _ => Ok(()),
    }
}

impl TrailGeometryLaw {
    pub fn from_params(params: &TrailParams) -> Result<Self, Refused> {
        let recording = TrailRecording::from_params(params)?;
        if params.generate_lighting_data {
            return Err(Refused::LightingData);
        }
        let width_over_trail = match params.width_over_trail {
            MinMaxCurve::Constant(v) => Width::Constant(v),
            MinMaxCurve::TwoConstants { min, max } => Width::TwoConstants { min, max },
            _ => return Err(Refused::CurveMode("widthOverTrail")),
        };
        check_gradient(&params.color_over_lifetime)?;
        check_gradient(&params.color_over_trail)?;
        Ok(Self {
            recording,
            texture_mode: params.texture_mode,
            texture_scale: params.texture_scale,
            shadow_bias: params.shadow_bias,
            world_space: params.world_space,
            size_affects_width: params.size_affects_width,
            inherit_particle_color: params.inherit_particle_color,
            color_over_lifetime: params.color_over_lifetime.clone(),
            width_over_trail,
            color_over_trail: params.color_over_trail.clone(),
        })
    }
}

/// Renderer-side inputs of one trail draw.
#[derive(Clone, Copy, Debug)]
pub struct TrailView<'g> {
    /// World to camera, column major.
    pub view: [f32; 16],
    /// The owner the job composes with the view: the emitter's local to
    /// world for a non-World simulation, identity for World.
    pub owner: [f32; 16],
    /// The emitter's local to world, read only for a world-space trail of a
    /// non-World simulation (its head point).
    pub local_to_world: [f32; 16],
    pub simulation_world: bool,
    pub emitter_scale: [f32; 3],
    /// A shadow pass applies the shadow bias along view depth.
    pub shadow_pass: bool,
    /// Linear colour space with the renderer applying it.
    pub linear_colour: bool,
    pub mesh_renderer: bool,
    pub size3d: bool,
    /// The system's ColorModule gradient when that module is enabled.
    pub colour_module: Option<&'g MinMaxGradient>,
}

/// One particle as the geometry job reads it.
#[derive(Clone, Copy, Debug)]
pub struct GeometryParticle<'r> {
    pub position: [f32; 3],
    pub age_percent: f32,
    pub inverse_lifetime: f32,
    pub seed: u32,
    /// The particle colour array word (RGBA32, R in the low byte).
    pub colour: u32,
    /// The size triple the size-pair flag selects.
    pub size: [f32; 3],
    pub ring: &'r TrailRing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailVertex {
    pub position: [f32; 3],
    /// RGBA32, R in the low byte.
    pub colour: u32,
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrailMesh {
    pub vertices: Vec<TrailVertex>,
    pub indices: Vec<u32>,
}

// ---- vectors of four lanes ----

type V4 = [f32; 4];

fn vadd(x: V4, y: V4) -> V4 {
    std::array::from_fn(|i| a::add(x[i], y[i]))
}
fn vsub(x: V4, y: V4) -> V4 {
    std::array::from_fn(|i| a::sub(x[i], y[i]))
}
fn vmul(x: V4, y: V4) -> V4 {
    std::array::from_fn(|i| a::mul(x[i], y[i]))
}
fn vscale(x: V4, s: f32) -> V4 {
    x.map(|v| a::mul(v, s))
}
fn col(m: &[f32; 16], i: usize) -> V4 {
    std::array::from_fn(|k| m[4 * i + k])
}

/// (c0*x + c1*y) + (c2*z + c3), four lanes.
fn affine(m: &[f32; 16], p: [f32; 3]) -> V4 {
    vadd(vadd(vscale(col(m, 0), p[0]), vscale(col(m, 1), p[1])), vadd(vscale(col(m, 2), p[2]), col(m, 3)))
}

fn sum4(sq: V4) -> f32 {
    a::add(a::add(sq[0], sq[1]), a::add(sq[2], sq[3]))
}

fn len2(x: f32, y: f32) -> f32 {
    a::add(a::add(a::mul(x, x), a::mul(y, y)), a::add(0.0, 0.0))
}

// ---- matrices ----

/// The view copy and its approximate inverse (the three first columns
/// normalized by one rsqrt of the mean squared column length).
fn inverse_view(view: &[f32; 16]) -> Result<[f32; 16], Refused> {
    let c: [V4; 4] = std::array::from_fn(|i| col(view, i));
    let l: [f32; 3] = std::array::from_fn(|k| {
        let mut sq = vmul(c[k], c[k]);
        sq[3] = 0.0;
        sum4(sq)
    });
    let s = a::add(a::add(l[0], l[1]), l[2]);
    let s3 = a::mul(s, THIRD_APPROX);
    let inv_cols: [V4; 3] = if s3 < TINY_SCALE {
        [[0.0; 4]; 3]
    } else {
        let r = a::mul(a::rsqrt_estimate(s3), RSQ_BIAS);
        let (c0, c1, c2) = (vscale(c[0], r), vscale(c[1], r), vscale(c[2], r));
        let cross = |p: V4, q: V4| -> [f32; 3] {
            [
                a::sub(a::mul(p[1], q[2]), a::mul(q[1], p[2])),
                a::sub(a::mul(p[2], q[0]), a::mul(q[2], p[0])),
                a::sub(a::mul(p[0], q[1]), a::mul(q[0], p[1])),
            ]
        };
        let cr12 = cross(c1, c2);
        let prod = [a::mul(c0[0], cr12[0]), a::mul(c0[1], cr12[1]), a::mul(c0[2], cr12[2]), 0.0];
        let det = sum4(prod);
        if !(a::abs(det) > DET_EPS) {
            return Err(Refused::SingularView);
        }
        let cr20 = cross(c2, c0);
        let cr01 = cross(c0, c1);
        let idet = a::div(ONE, det);
        std::array::from_fn(|i| vscale(vscale([cr12[i], cr20[i], cr01[i], 0.0], idet), r))
    };
    let t = c[3].map(a::neg);
    let col3 = vadd(vscale(inv_cols[0], t[0]), vadd(vscale(inv_cols[1], t[1]), vscale(inv_cols[2], t[2])));
    let mut out = [0.0; 16];
    for (i, column) in [inv_cols[0], inv_cols[1], inv_cols[2], col3].into_iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&column);
    }
    Ok(out)
}

fn force_affine(mut m: [f32; 16]) -> [f32; 16] {
    m[3] = 0.0;
    m[7] = 0.0;
    m[11] = 0.0;
    m[15] = ONE;
    m
}

/// The job's forward matrix (view times owner, or the view alone for a
/// world-space trail of a non-World simulation) and its inverse, each with the
/// last row forced to (0, 0, 0, 1).
fn job_matrices(view: &[f32; 16], inverse: [f32; 16], owner: &[f32; 16], world_trail: bool) -> ([f32; 16], [f32; 16]) {
    let forward = if world_trail {
        *view
    } else {
        let av: [V4; 4] = std::array::from_fn(|i| col(view, i));
        let b = |j: usize, k: usize| owner[4 * j + k];
        let mut m = [0.0; 16];
        for j in 0..3 {
            let column = vadd(vscale(av[0], b(j, 0)), vadd(vscale(av[1], b(j, 1)), vscale(av[2], b(j, 2))));
            m[4 * j..4 * j + 4].copy_from_slice(&column);
        }
        let column = vadd(av[3], vadd(vscale(av[0], b(3, 0)), vadd(vscale(av[1], b(3, 1)), vscale(av[2], b(3, 2)))));
        m[12..16].copy_from_slice(&column);
        m
    };
    (force_affine(forward), force_affine(inverse))
}

// ---- colour ----

/// One gradient over four equal lanes (lane 0), as the colour templates run
/// their key search: a Blend or Fixed pass over the colour keys and one over
/// the alpha keys; a channel with fewer than two keys stays white.
fn gradient_channels(g: &Gradient, time: f32) -> [f32; 4] {
    let mut out = [ONE; 4];
    let t = a::mul(time, TIME_SCALE);
    let ccodes: Vec<f32> = g.color_keys.iter().map(|k| time_code(k.time) as f32).collect();
    let acodes: Vec<f32> = g.alpha_keys.iter().map(|k| time_code(k.time) as f32).collect();
    let colour = |j: usize, c: usize| g.color_keys[j].color[c];
    let alpha = |j: usize, _: usize| g.alpha_keys[j].alpha;
    match g.mode {
        GradientMode::Blend => {
            blend_channel(&ccodes, &colour, 3, t, &mut out[..3]);
            blend_channel(&acodes, &alpha, 1, t, &mut out[3..]);
        }
        GradientMode::Fixed => {
            fixed_channel(&ccodes, &colour, 3, t, &mut out[..3]);
            fixed_channel(&acodes, &alpha, 1, t, &mut out[3..]);
        }
    }
    out
}

fn blend_channel(codes: &[f32], value: &dyn Fn(usize, usize) -> f32, channels: usize, t: f32, out: &mut [f32]) {
    let n = codes.len();
    if n < 2 {
        return;
    }
    let q = a::min(a::max(t, codes[0]), codes[n - 1]);
    let Some(k) = (0..n - 1).find(|&x| !(q > codes[1 + x])).map(|x| x + 1) else {
        return;
    };
    for j in k..n {
        let (lo, hi) = (codes[j - 1], codes[j]);
        let span = a::max(a::sub(hi, lo), EPS_SPAN);
        let fac = a::min(a::div(a::sub(q, lo), span), ONE);
        for c in 0..channels {
            let d = a::sub(value(j, c), value(j - 1, c));
            out[c] = a::add(value(j - 1, c), a::mul(fac, d));
        }
        if hi >= q {
            break;
        }
    }
}

fn fixed_channel(codes: &[f32], value: &dyn Fn(usize, usize) -> f32, channels: usize, t: f32, out: &mut [f32]) {
    let n = codes.len();
    if n < 2 {
        return;
    }
    let q = a::min(a::max(t, codes[0]), codes[n - 1]);
    let Some(k) = (0..n).find(|&x| !(q > codes[x])) else {
        return;
    };
    for j in k..n {
        for c in 0..channels {
            out[c] = value(j, c);
        }
        if codes[j] >= q {
            break;
        }
    }
}

fn quantize(c: f32) -> u32 {
    let v = a::add(a::mul(a::min(a::max(c, 0.0), ONE), 255.0), HALF);
    (a::to_i32_toward_zero(v) as u32) & 0xff
}

fn pack(bytes: [u32; 4]) -> u32 {
    bytes[0] | (bytes[1] << 8) | (bytes[2] << 16) | (bytes[3] << 24)
}

/// The MinMaxGradient colour templates at one time and one random factor.
fn gradient_rgba32(block: &MinMaxGradient, time: f32, random: f32) -> u32 {
    let q4 = |v: [f32; 4]| v.map(quantize);
    let bytes = match block {
        MinMaxGradient::Color(c) => q4(*c),
        MinMaxGradient::TwoColors { min, max } => {
            q4(std::array::from_fn(|c| a::add(min[c], a::mul(random, a::sub(max[c], min[c])))))
        }
        MinMaxGradient::Gradient(g) => q4(gradient_channels(g, time)),
        MinMaxGradient::RandomColor(g) => q4(gradient_channels(g, random)),
        MinMaxGradient::TwoGradients { min, max } => {
            let lo = q4(gradient_channels(min, time));
            let hi = q4(gradient_channels(max, time));
            let f = a::to_i32_toward_zero(a::mul(random, 255.0)) as u32;
            let f = f | f.wrapping_shl(16);
            let f = f | f.wrapping_shl(8);
            std::array::from_fn(|c| {
                let fb = (f >> (8 * c)) & 0xff;
                let d = hi[c].wrapping_sub(lo[c]) & 0xffff;
                let acc = (0x80 + fb * d) & 0xffff;
                (lo[c] + (acc >> 8)) & 0xff
            })
        }
    };
    pack(bytes)
}

/// ((g + 1) * c) >> 8 per byte.
fn gradient_times_colour(g: u32, c: u32) -> u32 {
    (0..4).fold(0, |out, i| {
        let s = 8 * i;
        out | (((((g >> s) & 0xff) + 1) * ((c >> s) & 0xff) >> 8) << s)
    })
}

/// ((c + 1) * g) >> 8 per byte.
fn colour_times_gradient(g: u32, c: u32) -> u32 {
    (0..4).fold(0, |out, i| {
        let s = 8 * i;
        out | (((((c >> s) & 0xff) + 1) * ((g >> s) & 0xff) >> 8) << s)
    })
}

/// The particle's elapsed fraction of its lifetime as the job computes it
/// from the lifetime array and the age, floored at zero.
fn normalized_age(age: f32, inverse_lifetime: f32) -> f32 {
    let life = a::div(ONE, inverse_lifetime);
    let rem = a::mul(life, a::add(a::mul(age, NEG_CENTI), ONE));
    let t = if life == 0.0 { 0.0 } else { a::div(a::sub(life, rem), life) };
    a::max_nm(t, 0.0)
}

// ---- per particle ----

struct LineParams {
    rand1: f32,
    rand2: f32,
    width: f32,
    colour: u32,
}

fn line_params(law: &TrailGeometryLaw, view: &TrailView, p: &GeometryParticle, width_scale: f32)
    -> Result<LineParams, Refused> {
    let w0 = if law.size_affects_width {
        let s = p.size;
        if view.size3d {
            if view.mesh_renderer {
                return Err(Refused::MeshSizeWidth);
            }
            a::sqrt(a::mul(s[1], s[0]))
        } else {
            s[0]
        }
    } else {
        ONE
    };
    let width = a::mul(w0, width_scale);
    let rand1 = ParticleRandom::sample(p.seed, WIDTH_SALT);
    let mut colour = if law.inherit_particle_color {
        let mut colour = p.colour;
        if let Some(module) = view.colour_module {
            let t = normalized_age(p.age_percent, p.inverse_lifetime);
            let g = gradient_rgba32(module, t, ParticleRandom::sample(p.seed, COLOUR_SALT));
            colour = gradient_times_colour(g, colour);
        }
        colour
    } else {
        0xffff_ffff
    };
    let t = normalized_age(p.age_percent, p.inverse_lifetime);
    let g = gradient_rgba32(&law.color_over_lifetime, t, ParticleRandom::sample(p.seed, COLOUR_SALT));
    colour = gradient_times_colour(g, colour);
    let rand2 = ParticleRandom::sample(p.seed, TRAIL_COLOUR_SALT);
    Ok(LineParams { rand1, rand2, width, colour })
}

/// Head, then the recorded points newest first.
fn flatten(p: &GeometryParticle, world_trail: bool, local_to_world: &[f32; 16]) -> Vec<[f32; 3]> {
    let c = p.ring.len();
    let mut pts = vec![[0.0; 3]; c + 1];
    for (m, point) in p.ring.points().enumerate() {
        pts[c - m] = point.position;
    }
    pts[0] = if world_trail {
        if p.age_percent > HUNDRED {
            pts[1]
        } else {
            let w = affine(local_to_world, p.position);
            [w[0], w[1], w[2]]
        }
    } else {
        p.position
    };
    pts
}

/// Pull the oldest point toward the next one so the trail ends at its lifetime.
fn clip_tail(p: &GeometryParticle, pts: &mut [[f32; 3]], life: f32, time: f64) {
    let c = p.ring.len();
    let mut points = p.ring.points();
    let tb = points.next().expect("a clipped trail holds two points").time;
    let tn = points.next().expect("a clipped trail holds two points").time;
    let age = a::narrow(time - tb as f64);
    let frac = a::div(a::sub(age, life), a::sub(tn, tb));
    let frac = a::min(a::max(frac, 0.0), ONE);
    let old = [pts[c][0], pts[c][1], pts[c][2], 0.0];
    let newer = [pts[c - 1][0], pts[c - 1][1], pts[c - 1][2], 0.0];
    let moved = vadd(old, vscale(vsub(newer, old), frac));
    pts[c] = [moved[0], moved[1], moved[2]];
}

struct LineData {
    points: Vec<V4>,
    segment: Vec<f32>,
    normal: Vec<[f32; 2]>,
    corner: Vec<[f32; 2]>,
    total: f32,
}

fn unit2(v: [f32; 2]) -> ([f32; 2], f32) {
    let l2 = len2(v[0], v[1]);
    let r = a::rsqrt2(l2);
    ([a::mul(v[0], r), a::mul(v[1], r)], l2)
}

/// View-space points, segment lengths, 2D normals with short-segment miters,
/// and corner offsets.
fn store_line_data(pts: &[[f32; 3]], forward: &[f32; 16], width: f32) -> LineData {
    let n = pts.len();
    let points: Vec<V4> = pts.iter().map(|q| affine(forward, *q)).collect();
    let mut total = 0.0;
    let mut segment = vec![0.0; n];
    let mut scaled = vec![0.0; n];
    let mut normal = vec![[0.0; 2]; n];
    let w2 = a::add(width, width);
    let rw = a::recip2(w2);
    let mut prev: Option<V4> = None;
    let threshold = TINY_SEGMENT[usize::from(n > 2)];
    let dist2 = |d: V4| sum4(vmul(d, d));
    for k in 0..n {
        let j = if k != 0 { k } else { 1 };
        let mut d = vsub(points[j - 1], points[j]);
        let mut d2 = dist2(d);
        let l = a::sqrt(d2);
        if k != 0 {
            segment[k] = l;
            total = a::add(total, l);
        }
        scaled[k] = a::mul(rw, l);
        if threshold > d2 {
            if n - 1 > j {
                d = vsub(points[j], points[j + 1]);
                d2 = dist2(d);
            }
            if threshold > d2 {
                d = prev.unwrap_or(DIR_FALLBACK);
            } else {
                prev = Some(d);
            }
        } else {
            prev = Some(d);
        }
        let p = points[k];
        let q = p.map(|x| a::mul(a::max(a::abs(x), CENTI), f32::from_bits((x.to_bits() & 0x8000_0000) | ONE.to_bits())));
        let nx = a::sub(a::mul(d[2], q[1]), a::mul(d[1], q[2]));
        let ny = a::sub(a::mul(d[0], q[2]), a::mul(d[2], q[0]));
        normal[k] = unit2([nx, ny]).0;
        if k != 0 && w2 > l {
            let (n0, n1) = (normal[k - 1], normal[k]);
            let f0 = a::add(scaled[k], NEG_HALF);
            let avg = [a::mul(a::add(n0[0], n1[0]), HALF), a::mul(a::add(n0[1], n1[1]), HALF)];
            let fct = a::max(f0, 0.0);
            let fct = a::add(fct, fct);
            let t0 = [a::add(avg[0], a::mul(a::sub(n0[0], avg[0]), fct)),
                a::add(avg[1], a::mul(a::sub(n0[1], avg[1]), fct))];
            let (u0, l2a) = unit2(t0);
            normal[k - 1] = if l2a > TINY_SCALE { u0 } else { n0 };
            let t1 = [a::add(avg[0], a::mul(fct, a::sub(n1[0], avg[0]))),
                a::add(avg[1], a::mul(fct, a::sub(n1[1], avg[1])))];
            let (u1, l2b) = unit2(t1);
            normal[k] = if l2b > TINY_SCALE { u1 } else { n1 };
        }
    }
    let mut corner = Vec::with_capacity(n.saturating_sub(2));
    for k in 0..n.saturating_sub(2) {
        let (n1, n2) = (normal[k + 1], normal[k + 2]);
        let v17 = [a::neg(n1[0]), a::neg(n1[1])];
        let v19 = [a::neg(n2[0]), a::neg(n2[1])];
        let m2 = a::div(a::neg(v19[0]), v19[1]);
        let m1 = a::div(v17[0], a::neg(v17[1]));
        let m1 = a::min(a::max(m1, a::neg(SLOPE_CLAMP)), SLOPE_CLAMP);
        let m2 = a::min(a::max(m2, a::neg(SLOPE_CLAMP)), SLOPE_CLAMP);
        let mut c = if a::abs(a::sub(m1, m2)) < SLOPE_EPS {
            v17
        } else {
            let dn = [a::sub(n2[0], n1[0]), a::sub(n2[1], n1[1])];
            let ds = [a::sub(v19[0], n1[0]), a::sub(v19[1], n1[1])];
            let b1 = a::sub(v17[1], a::mul(m1, v17[0]));
            let dm = a::sub(m1, m2);
            let v22 = [a::mul(v19[0], m2), a::mul(v19[1], m2)];
            let b2 = [a::sub(v19[1], v22[0]), a::sub(v19[1], v22[1])];
            let lan = len2(dn[0], dn[1]);
            let las = len2(ds[0], ds[1]);
            let xs = a::div(a::sub(b2[0], b1), dm);
            let ys = a::add(b1, a::mul(m1, xs));
            let ts = a::min(a::min(a::max(a::mul(lan, FIFTY), 0.0), ONE), a::min(a::max(a::mul(las, FIFTY), 0.0), ONE));
            [a::sub(a::mul(a::add(n1[0], xs), ts), n1[0]), a::sub(a::mul(a::add(n1[1], ys), ts), n1[1])]
        };
        let l2 = len2(c[0], c[1]);
        let limit = a::max(a::mul(a::min(a::max(scaled[k], 0.0), ONE), THREE), ONE);
        if l2 > a::mul(limit, limit) {
            let r = a::rsqrt2(l2);
            c = [a::mul(limit, a::mul(c[0], r)), a::mul(limit, a::mul(c[1], r))];
        }
        corner.push(c);
    }
    LineData { points, segment, normal, corner, total: a::max(total, DET_EPS) }
}

/// The whole per-particle trail geometry of one draw. `clock` is the trail
/// module's clock (the tail clip ages against it).
pub fn build<'r>(
    law: &TrailGeometryLaw,
    view: &TrailView,
    clock: TrailClock,
    particles: impl IntoIterator<Item = GeometryParticle<'r>>,
) -> Result<TrailMesh, Refused> {
    if view.linear_colour {
        return Err(Refused::LinearColour);
    }
    if let (true, Some(module)) = (law.inherit_particle_color, view.colour_module) {
        check_gradient(module)?;
    }
    let inverse = inverse_view(&view.view)?;
    let world_trail = law.world_space && !view.simulation_world;
    let (forward, back) = job_matrices(&view.view, inverse, &view.owner, world_trail);
    let [sx, sy, sz] = view.emitter_scale;
    if a::abs(a::mul(a::mul(sx, sy), sz)).to_bits() != ONE.to_bits() {
        return Err(Refused::EmitterScale);
    }
    let width_scale = ONE;
    let bias = if view.shadow_pass { law.shadow_bias } else { 0.0 };
    let [tx, ty] = law.texture_scale;
    let per_segment = matches!(law.texture_mode,
        TrailTextureMode::DistributePerSegment | TrailTextureMode::RepeatPerSegment);
    let scaled_u = matches!(law.texture_mode, TrailTextureMode::Tile | TrailTextureMode::RepeatPerSegment);
    let mut mesh = TrailMesh::default();
    for p in particles {
        let c = p.ring.len();
        if c == 0 {
            continue;
        }
        let mut pts = flatten(&p, world_trail, &view.local_to_world);
        if c >= 2 {
            let life = trail_lifetime(&law.recording, p.seed, p.inverse_lifetime,
                TrailSize { components: p.size, size3d: view.size3d });
            clip_tail(&p, &mut pts, life, clock.time);
        }
        let params = line_params(law, view, &p, width_scale)?;
        let line = store_line_data(&pts, &forward, params.width);
        let n = c + 1;
        let (mut acc, ubase) = if per_segment {
            let count = c as f32;
            (a::recip2(count), count)
        } else {
            (0.0, line.total)
        };
        let rt = a::recip2(line.total);
        let uscale = a::mul(if scaled_u { ubase } else { ONE }, tx);
        let vhalf = a::mul(ty, HALF);
        let (vhi, vlo) = (a::add(vhalf, HALF), a::sub(HALF, vhalf));
        let width_factor = law.width_over_trail.evaluate(params.rand1);
        let last = n - 1;
        for k in 0..n {
            let tk = if per_segment {
                a::mul(acc, k as f32)
            } else {
                acc = a::add(acc, line.segment[k]);
                a::mul(rt, acc)
            };
            let u = a::mul(uscale, tk);
            let tc = a::min(a::max(tk, 0.0), ONE);
            let g = gradient_rgba32(&law.color_over_trail, tc, params.rand2);
            let colour = colour_times_gradient(g, params.colour);
            let wk = a::mul(params.width, width_factor);
            let pk = line.points[k];
            let z = a::add(a::mul(bias, wk), pk[2]);
            let d0 = line.normal[(k + 1).min(last)];
            let hw = a::mul(wk, HALF);
            let dvec = if k == last || k == 0 { [a::neg(d0[0]), a::neg(d0[1])] } else { line.corner[k - 1] };
            let off = [a::mul(hw, dvec[0]), a::mul(hw, dvec[1])];
            for (side, v) in [(false, vhi), (true, vlo)] {
                let xy = if side {
                    [a::add(pk[0], off[0]), a::add(pk[1], off[1])]
                } else {
                    [a::sub(pk[0], off[0]), a::sub(pk[1], off[1])]
                };
                let w = affine(&back, [xy[0], xy[1], z]);
                mesh.vertices.push(TrailVertex { position: [w[0], w[1], w[2]], colour, u, v });
            }
            let v0 = (mesh.vertices.len() - 2) as u32;
            if k != 0 {
                mesh.indices.extend([v0.wrapping_sub(2), v0, v0 - 1, v0 - 1, v0, v0 + 1]);
            }
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{self, Value};
    use crate::particle::trail::TrailPoint;

    fn num(v: &Value) -> f64 {
        v.as_f64().expect("number")
    }
    fn bits(v: &Value) -> f32 {
        f32::from_bits(num(v) as u32)
    }
    fn at<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("missing {key}"))
    }
    fn arr(v: &Value) -> &[Value] {
        v.as_array().expect("array")
    }
    fn mat(v: &Value) -> [f32; 16] {
        std::array::from_fn(|i| bits(&arr(v)[i]))
    }
    fn v3(v: &Value) -> [f32; 3] {
        std::array::from_fn(|i| bits(&arr(v)[i]))
    }

    /// A gradient exactly as the row carries it: the engine's templates take
    /// any key count and order, unlike the export decoder.
    fn raw_gradient(g: &Value) -> Gradient {
        use crate::particle::gradient::{GradientAlphaKey, GradientColorKey, GradientColorSpace};
        Gradient {
            color_keys: arr(at(g, "colorKeys")).iter().map(|k| GradientColorKey {
                time: num(at(k, "time")) as f32,
                color: std::array::from_fn(|c| num(&arr(at(k, "color"))[c]) as f32),
            }).collect(),
            alpha_keys: arr(at(g, "alphaKeys")).iter().map(|k| GradientAlphaKey {
                time: num(at(k, "time")) as f32, alpha: num(at(k, "alpha")) as f32,
            }).collect(),
            mode: match at(g, "interpolation").as_str().unwrap() {
                "blend" => GradientMode::Blend,
                "fixed" => GradientMode::Fixed,
                other => panic!("interpolation {other}"),
            },
            color_space: GradientColorSpace::Unspecified,
        }
    }

    pub(super) fn raw_min_max_gradient(b: &Value) -> MinMaxGradient {
        let v4 = |v: &Value| -> [f32; 4] { std::array::from_fn(|c| num(&arr(v)[c]) as f32) };
        match at(b, "mode").as_str().unwrap() {
            "color" => MinMaxGradient::Color(v4(at(b, "color"))),
            "twoColors" => MinMaxGradient::TwoColors { min: v4(at(b, "min")), max: v4(at(b, "max")) },
            "gradient" => MinMaxGradient::Gradient(raw_gradient(at(b, "gradient"))),
            "randomColor" => MinMaxGradient::RandomColor(raw_gradient(at(b, "gradient"))),
            "twoGradients" => MinMaxGradient::TwoGradients {
                min: raw_gradient(at(b, "minGradient")), max: raw_gradient(at(b, "maxGradient")),
            },
            other => panic!("gradient mode {other}"),
        }
    }

    /// Whether the product's export decoder admits every gradient of the row.
    fn decoder_admits(case: &Value) -> bool {
        let t = at(case, "trail");
        let mut blocks = vec![at(t, "colorOverLifetime"), at(t, "colorOverTrail")];
        let cm = at(case, "colorModule");
        if at(cm, "enabled").as_bool().unwrap() { blocks.push(at(cm, "gradient")); }
        blocks.into_iter().all(|b| crate::particle::schema::min_max_gradient(Some(b), "row").is_ok())
    }

    fn law_of(case: &Value) -> Result<TrailGeometryLaw, Refused> {
        let t = at(case, "trail");
        let gradient = |key: &str| raw_min_max_gradient(at(t, key));
        let curve = |key: &str| {
            let c = at(t, key);
            match at(c, "mode").as_str().unwrap() {
                "constant" => MinMaxCurve::Constant(num(at(c, "value")) as f32),
                "twoConstants" => MinMaxCurve::TwoConstants { min: num(at(c, "min")) as f32, max: num(at(c, "max")) as f32 },
                other => panic!("curve mode {other}"),
            }
        };
        let params = TrailParams {
            mode: crate::particle::schema::TrailMode::PerParticle,
            ratio: bits(at(t, "ratio")),
            lifetime: curve("lifetime"),
            min_vertex_distance: bits(at(t, "minVertexDistance")),
            texture_mode: match at(t, "textureMode").as_str().unwrap() {
                "stretch" => TrailTextureMode::Stretch,
                "tile" => TrailTextureMode::Tile,
                "distributePerSegment" => TrailTextureMode::DistributePerSegment,
                _ => TrailTextureMode::RepeatPerSegment,
            },
            texture_scale: [bits(&arr(at(t, "textureScale"))[0]), bits(&arr(at(t, "textureScale"))[1])],
            ribbon_count: 1,
            shadow_bias: bits(at(t, "shadowBias")),
            world_space: at(t, "worldSpace").as_bool().unwrap(),
            die_with_particles: at(t, "dieWithParticles").as_bool().unwrap(),
            size_affects_width: at(t, "sizeAffectsWidth").as_bool().unwrap(),
            size_affects_lifetime: at(t, "sizeAffectsLifetime").as_bool().unwrap(),
            inherit_particle_color: at(t, "inheritParticleColor").as_bool().unwrap(),
            generate_lighting_data: at(t, "generateLightingData").as_bool().unwrap(),
            split_sub_emitter_ribbons: false,
            attach_ribbons_to_transform: false,
            color_over_lifetime: gradient("colorOverLifetime"),
            width_over_trail: curve("widthOverTrail"),
            color_over_trail: gradient("colorOverTrail"),
        };
        TrailGeometryLaw::from_params(&params)
    }

    fn rings_of(case: &Value) -> Vec<TrailRing> {
        let max_pos = num(at(case, "maxPos")) as usize;
        arr(at(case, "particles")).iter().map(|p| {
            let r = at(p, "ring");
            let (back, count) = (num(at(r, "back")) as usize, num(at(r, "count")) as usize);
            let slots = arr(at(r, "points"));
            TrailRing::from_points((0..count).map(|m| {
                let w = arr(&slots[(back + m) % max_pos]);
                TrailPoint { position: std::array::from_fn(|c| bits(&w[c])), time: bits(&w[3]) }
            }), bits(at(r, "len")))
        }).collect()
    }

    fn run(case: &Value, rings: &[TrailRing], law: &TrailGeometryLaw, time: f64, size_override: Option<&dyn Fn([f32; 3]) -> [f32; 3]>)
        -> Result<TrailMesh, Refused> {
        let system = at(case, "system");
        let colour_module = at(case, "colorModule");
        let module = at(colour_module, "enabled").as_bool().unwrap()
            .then(|| raw_min_max_gradient(at(colour_module, "gradient")));
        let view = TrailView {
            view: mat(at(at(case, "camera"), "view")),
            owner: mat(at(system, "owner")),
            local_to_world: mat(at(system, "localToWorld")),
            simulation_world: num(at(system, "simulationSpace")) == 1.0,
            emitter_scale: v3(at(system, "emitterScale")),
            shadow_pass: num(at(case, "shadowPass")) != 0.0,
            linear_colour: num(at(case, "colourSpace")) == 1.0
                && at(at(case, "renderer"), "applyActiveColorSpace").as_bool().unwrap(),
            mesh_renderer: num(at(at(case, "renderer"), "renderMode")) == 4.0,
            size3d: at(case, "size3D").as_bool().unwrap(),
            colour_module: module.as_ref(),
        };
        let pair_b = num(at(case, "sizePair")) != 0.0;
        let particles: Vec<GeometryParticle> = arr(at(case, "particles")).iter().zip(rings).map(|(p, ring)| {
            let size = v3(at(p, if pair_b { "sizeB" } else { "sizeA" }));
            GeometryParticle {
                position: v3(at(p, "pos")),
                age_percent: bits(at(p, "age")),
                inverse_lifetime: bits(at(p, "inv")),
                seed: num(at(p, "seed")) as u32,
                colour: num(at(p, "colour")) as u32,
                size: size_override.map_or(size, |f| f(size)),
                ring,
            }
        }).collect();
        build(law, &view, TrailClock { time }, particles)
    }

    fn words(mesh: &TrailMesh) -> Vec<u32> {
        mesh.vertices.iter().flat_map(|v| [v.position[0].to_bits(), v.position[1].to_bits(),
            v.position[2].to_bits(), v.colour, v.u.to_bits(), v.v.to_bits()]).collect()
    }

    /// Native trail geometry rows: SetMatrices, the trail job's Initialize and
    /// RenderJobCommon executed in the engine library over the exported trail
    /// blocks on recorded rings, edge cases and random cases (all four texture
    /// modes, Local and World simulation, world-space trails, shadow pass, u16
    /// and u32 indices, NaN-producing inputs). Every vertex word and index of
    /// every row the receipt replays must match; every row it marks as outside
    /// the transcription must be refused. Three input perturbations must each
    /// change the output somewhere: the tail clip removed (clock at the oldest
    /// point), the width taken as sqrt(x*y) with sizes not 3D, and a vertex
    /// count of two per recorded point.
    #[test]
    #[ignore = "needs MOLY_TRAIL_GEOMETRY_ROWS"]
    fn native_geometry_rows_match_bits() {
        let path = std::env::var("MOLY_TRAIL_GEOMETRY_ROWS").expect("MOLY_TRAIL_GEOMETRY_ROWS");
        let doc = json::parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let (mut matched, mut refused, mut vertices, mut indices) = (0usize, 0usize, 0usize, 0usize);
        let (mut no_clip, mut sqrt_width, mut count_formula, mut nan_words) = (0usize, 0usize, 0usize, 0usize);
        let mut decoder_refused = 0usize;
        for row in arr(at(&doc, "rows")) {
            let case = at(row, "case");
            let id = at(case, "id").as_str().unwrap();
            let time = f64::from_bits(super::super::trail::tests::hex64(at(at(case, "trail"), "timeBits")));
            let rings = rings_of(case);
            let result = law_of(case).and_then(|law| run(case, &rings, &law, time, None).map(|m| (law, m)));
            if !at(row, "replay").as_bool().unwrap() {
                assert!(result.is_err(), "{id}: the receipt marks this row outside the transcription ({:?}); it must be refused",
                    at(row, "refusal"));
                refused += 1;
                continue;
            }
            let (law, mesh) = result.unwrap_or_else(|e| panic!("{id}: refused a replayed row: {e:?}"));
            decoder_refused += usize::from(!decoder_admits(case));
            let expected: Vec<u32> = arr(at(row, "vertexWords")).iter().map(|v| num(v) as u32).collect();
            let got = words(&mesh);
            assert_eq!(got.len(), expected.len(), "{id}: vertex word count");
            for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
                assert_eq!(g, e, "{id}: vertex {} word {}", i / 6, i % 6);
            }
            let mask = if num(at(case, "indexSize")) == 2.0 { 0xffff } else { u32::MAX };
            let native_indices: Vec<u32> = arr(at(row, "indices")).iter().map(|v| num(v) as u32).collect();
            let got_indices: Vec<u32> = mesh.indices.iter().map(|i| i & mask).collect();
            assert_eq!(got_indices, native_indices, "{id}: indices");
            nan_words += got.iter().filter(|w| f32::from_bits(**w).is_nan()).count();
            vertices += mesh.vertices.len();
            indices += mesh.indices.len();
            matched += 1;
            // Perturbations.
            let oldest = rings.iter().filter(|r| r.len() >= 2).map(|r| r.points().next().unwrap().time)
                .fold(f32::INFINITY, f32::min);
            if oldest.is_finite() {
                if let Ok(m) = run(case, &rings, &law, oldest as f64, None) { no_clip += (words(&m) != expected) as usize; }
            }
            if at(at(case, "trail"), "sizeAffectsWidth").as_bool().unwrap() && !at(case, "size3D").as_bool().unwrap() {
                let root = |s: [f32; 3]| [(s[0] * s[1]).sqrt(), s[1], s[2]];
                if let Ok(m) = run(case, &rings, &law, time, Some(&root)) { sqrt_width += (words(&m) != expected) as usize; }
            }
            let recorded: usize = rings.iter().map(TrailRing::len).sum();
            count_formula += (2 * recorded != mesh.vertices.len()) as usize;
        }
        println!("trail geometry replay: {matched} rows matched ({vertices} vertices, {indices} indices, {nan_words} NaN words; \
            {decoder_refused} of them carry a gradient the export decoder refuses), {refused} refused as the receipt marks them; \
            perturbed rows differing: noTailClip {no_clip}, sqrtWidth {sqrt_width}, twoVerticesPerPoint {count_formula}");
        assert!(matched > 0 && refused > 0 && no_clip > 0 && sqrt_width > 0 && count_formula > 0);
    }
}
