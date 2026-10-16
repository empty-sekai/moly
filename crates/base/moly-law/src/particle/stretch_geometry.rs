//! Stretch render mode quads: the renderer's stretched billboard with freeform
//! stretching off, as the engine's Stretch geometry body builds it and its
//! vertex writer packs it (`引擎原生逐指令` for the composition; the operand
//! order is the body's where it was read, and the receipt compares at a stated
//! tolerance, not bit for bit).
//!
//! Everything is in the engine's world basis. Per particle, with `h` the
//! camera-space position and `dv` the camera-space velocity (persistent plus
//! animated) minus the camera velocity times `cameraVelocityScale * scale.x`:
//!
//! * `r` is the reciprocal square root ESTIMATE of `|dv|^2` times
//!   `1.00196075` (no refinement step), zero at or below `1e-30`;
//! * the tail point is `h - dv * (velocityScale + (lengthScale * scale.x) *
//!   sizeY * r)`, the length taking the unclamped Y size;
//! * the width direction is the XY part of `tail x h`, normalized (two
//!   refinement steps), zero at or below `1e-30`, times the X size after the
//!   min/max screen limits (which scale the width only) and zero once the age
//!   has reached 100 percent;
//! * the width vector goes to world through the inverse view's 3x3 and then
//!   the renderer scale per world axis; the tail point through the whole
//!   inverse view;
//! * the corners are position + width, tail + width, tail - width and
//!   position - width, with UV (0,1), (1,1), (1,0) and (0,0).
//!
//! The writer's normals come from the width vector `c` and `d = tail -
//! position` (both in world): each normalized with two refinement steps, `c`
//! falling back to +X and `d` to +Y at or below `1e-30` squared; `n = (c x d)
//! * (1 - bend)`, and the corners take `n + c bend`, `n + d bend`, `n - c bend`
//! and `n - d bend`, where `bend` is `cosf(normalDirection * 90 * 0.017453292)`
//! (the Stretch mode's factor is one).
//!
//! Not transcribed and refused by the caller: freeform stretching, a non-zero
//! pivot, and a velocity speed modifier other than the constant one (the
//! engine multiplies the drawn velocity by it).
use super::armf as a;
use super::device_libm::cosf;

/// The corner UVs, in corner order.
pub const UV: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

const TINY: f32 = f32::from_bits(0x0da2_4260);
const RSQ_BIAS: f32 = f32::from_bits(0x3f80_4020);
const LARGEST_FLOOR: f32 = f32::from_bits(0x3586_37bd);
const HALF: f32 = 0.5;
const DEAD_AGE: f32 = 100.0;
const RIGHT_ANGLE: f32 = 90.0;
const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);

type V3 = [f32; 3];

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

/// The camera as the body reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchView {
    /// World to camera, column-major (the camera looks down its -Z).
    pub view: [f32; 16],
    /// Camera to world, column-major.
    pub inverse_view: [f32; 16],
    /// The camera's velocity in camera axes (world velocity through the view's 3x3).
    pub camera_velocity: V3,
}

/// The renderer inputs of the body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchRenderer {
    pub velocity_scale: f32,
    pub length_scale: f32,
    pub camera_velocity_scale: f32,
    /// The renderer scale per world axis.
    pub scale: V3,
    /// Screen limits: lower = z * limits[0] + limits[2], upper = z * limits[1]
    /// + limits[3], with z the camera-space depth coordinate; a negative limit
    /// is off.
    pub limits: [f32; 4],
    /// [`normal_bend`] of the renderer's normalDirection.
    pub normal_bend: f32,
}

/// One particle, in world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchParticle {
    pub position: V3,
    /// Persistent plus animated velocity.
    pub velocity: V3,
    /// Current X and Y size.
    pub size: [f32; 2],
    pub age_percent: f32,
}

/// Four corners and their normals, in corner order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchQuad {
    pub positions: [V3; 4],
    pub normals: [V3; 4],
}

/// The renderer's normal bend for the Stretch mode:
/// `cosf(normalDirection * 90 * 0.017453292)`, times one.
pub fn normal_bend(normal_direction: f32) -> f32 {
    cosf(a::mul(a::mul(normal_direction, RIGHT_ANGLE), DEG_TO_RAD))
}

fn affine(m: &[f32; 16], p: V3) -> V3 {
    std::array::from_fn(|r| {
        a::add(a::add(a::mul(m[r], p[0]), a::mul(m[4 + r], p[1])), a::add(a::mul(m[8 + r], p[2]), m[12 + r]))
    })
}

fn linear(m: &[f32; 16], v: V3) -> V3 {
    std::array::from_fn(|r| a::add(a::add(a::mul(m[r], v[0]), a::mul(m[4 + r], v[1])), a::mul(m[8 + r], v[2])))
}

/// The writer's normalization: two refinement steps, the fallback axis at or
/// below `1e-30` squared.
fn unit_or(v: V3, fallback: V3) -> V3 {
    let sq = a::add(a::mul(v[0], v[0]), a::add(a::mul(v[1], v[1]), a::mul(v[2], v[2])));
    if !(sq > TINY) {
        return fallback;
    }
    let r = a::rsqrt2(sq);
    v.map(|x| a::mul(x, r))
}

/// One particle's quad.
pub fn stretch_quad(view: &StretchView, renderer: &StretchRenderer, particle: &StretchParticle) -> StretchQuad {
    quad_parts(view, renderer, particle).0
}

/// The intermediates the receipt's rounding bound reads: the camera-space
/// position, the squared XY length of `tail x h`, the world span `tail - position`.
#[derive(Clone, Copy, Debug)]
struct Parts {
    #[cfg_attr(not(test), allow(dead_code))]
    h: V3,
    #[cfg_attr(not(test), allow(dead_code))]
    cross_xy2: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    span_world: V3,
}

fn quad_parts(view: &StretchView, renderer: &StretchRenderer, particle: &StretchParticle) -> (StretchQuad, Parts) {
    let h = affine(&view.view, particle.position);
    let v = linear(&view.view, particle.velocity);
    let camera_term = if arms::on("noCameraVelocity") {
        0.0
    } else {
        a::mul(renderer.camera_velocity_scale, renderer.scale[0])
    };
    let dv: V3 = std::array::from_fn(|k| a::sub(v[k], a::mul(camera_term, view.camera_velocity[k])));
    let q = a::add(a::mul(dv[0], dv[0]), a::add(a::mul(dv[1], dv[1]), a::mul(dv[2], dv[2])));
    let r = if TINY >= q {
        0.0
    } else if arms::on("trueRsqrt") {
        a::div(1.0, a::sqrt(q))
    } else {
        a::mul(a::rsqrt_estimate(q), RSQ_BIAS)
    };
    let length = if arms::on("noScaleOnLength") {
        renderer.length_scale
    } else {
        a::mul(renderer.length_scale, renderer.scale[0])
    };
    let [size_x, size_y] = particle.size;

    // Screen limits on the largest of the two sizes (1e-6 floor); they scale
    // the width only.
    let largest = a::max(a::max(size_x, size_y), LARGEST_FLOOR);
    let lower = a::add(a::mul(h[2], renderer.limits[0]), renderer.limits[2]);
    let upper = a::add(a::mul(h[2], renderer.limits[1]), renderer.limits[3]);
    let half = if lower >= 0.0 { a::mul(a::max(largest, lower), HALF) } else { 0.0 };
    let half = if upper >= 0.0 { a::min(half, a::mul(upper, HALF)) } else { half };
    let limit = a::div(half, largest);
    let length_size = if arms::on("clampLength") { a::mul(a::mul(size_y, limit), 2.0) } else { size_y };

    let factor = a::add(renderer.velocity_scale, a::mul(a::mul(length, length_size), r));
    let tail: V3 = std::array::from_fn(|k| a::sub(h[k], a::mul(dv[k], factor)));
    let (cx, cy) = if arms::on("crossSign") {
        (a::sub(a::mul(h[1], tail[2]), a::mul(h[2], tail[1])), a::sub(a::mul(h[2], tail[0]), a::mul(h[0], tail[2])))
    } else {
        (a::sub(a::mul(tail[1], h[2]), a::mul(tail[2], h[1])), a::sub(a::mul(tail[2], h[0]), a::mul(tail[0], h[2])))
    };
    let w2 = a::add(a::mul(cx, cx), a::mul(cy, cy));
    let (wx, wy) = if w2 > TINY {
        let rw = a::rsqrt2(w2);
        (a::mul(cx, rw), a::mul(cy, rw))
    } else {
        (0.0, 0.0)
    };
    let width = if particle.age_percent >= DEAD_AGE { 0.0 } else { a::mul(size_x, limit) };
    let (wx, wy) = (a::mul(width, wx), a::mul(width, wy));
    let iv = &view.inverse_view;
    let width_world: V3 = std::array::from_fn(|r| {
        let s = renderer.scale[r];
        a::add(a::mul(a::mul(s, iv[r]), wx), a::mul(a::mul(s, iv[4 + r]), wy))
    });
    let tail_world = if arms::on("scaleSpan") {
        let span_world = linear(iv, std::array::from_fn(|k| a::sub(h[k], tail[k])));
        std::array::from_fn(|k| a::sub(particle.position[k], a::mul(renderer.scale[k], span_world[k])))
    } else {
        affine(iv, tail)
    };
    let p = particle.position;
    let w = width_world;
    let positions = [
        std::array::from_fn(|k| a::add(p[k], w[k])),
        std::array::from_fn(|k| a::add(tail_world[k], w[k])),
        std::array::from_fn(|k| a::sub(tail_world[k], w[k])),
        std::array::from_fn(|k| a::sub(p[k], w[k])),
    ];

    let span_world: V3 = std::array::from_fn(|k| a::sub(tail_world[k], p[k]));
    let c = unit_or(w, [1.0, 0.0, 0.0]);
    let d = unit_or(span_world, [0.0, 1.0, 0.0]);
    let bend = renderer.normal_bend;
    let keep = a::sub(1.0, bend);
    let n: V3 = [
        a::mul(keep, a::sub(a::mul(c[1], d[2]), a::mul(c[2], d[1]))),
        a::mul(keep, a::sub(a::mul(c[2], d[0]), a::mul(c[0], d[2]))),
        a::mul(keep, a::sub(a::mul(c[0], d[1]), a::mul(c[1], d[0]))),
    ];
    let normals = if arms::on("faceNormal") {
        [n; 4]
    } else {
        let cb = c.map(|x| a::mul(x, bend));
        let db = d.map(|x| a::mul(bend, x));
        [
            std::array::from_fn(|k| a::add(cb[k], n[k])),
            std::array::from_fn(|k| a::add(db[k], n[k])),
            std::array::from_fn(|k| a::sub(n[k], cb[k])),
            std::array::from_fn(|k| a::sub(n[k], db[k])),
        ]
    };
    (StretchQuad { positions, normals }, Parts { h, cross_xy2: w2, span_world })
}

/// Replay of the native Stretch receipt rows.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct StretchReplay {
    pub cases: usize,
    pub vertices: usize,
    /// Worst absolute position and normal differences.
    pub worst_position: f32,
    pub worst_normal: f32,
    /// Vertices whose normal is compared (the quad has width and length).
    pub normals_compared: usize,
    /// Vertices whose position, normal, colour or UV is outside tolerance.
    pub red: usize,
    pub first_red: Option<String>,
    /// Vertices whose position words are equal to the native bits.
    pub position_bits_equal: usize,
    /// Vertices within the base tolerance, and within it only with the
    /// rounding bound added.
    pub within_base: usize,
    pub within_bound_only: usize,
    /// The largest (difference - base tolerance) / rounding bound over the
    /// vertices that needed the bound.
    pub worst_bound_ratio: f32,
}

/// The receipt's base tolerance per coordinate: absolute for normals, scaled by
/// max(1, |coordinate|) for positions.
#[cfg(test)]
pub(crate) const TOLERANCE: f32 = 2.5e-5;
/// The operand rounding of the rounding bound: one unit in the last place of
/// a single-precision operand of magnitude one. The transcription's operand order differs from
/// the body's in the camera-space and world transforms, so its operands carry
/// a few ulps of their own rounding; the width direction (the normalized XY
/// part of `tail x h`) amplifies that by `|h|^2 / |(tail x h)_xy|`, and the
/// world span (`tail - position`) by `max(1, |position|) / |span|`.
#[cfg(test)]
pub(crate) const OPERAND_ULPS: f32 = f32::EPSILON;

#[cfg(test)]
pub(crate) fn replay_stretch_rows(text: &str, animated: bool) -> StretchReplay {
    use super::json::{parse, Value};
    let doc = parse(text.as_bytes()).expect("receipt json");
    let words = |v: &Value| -> Vec<u32> {
        v.as_array().expect("word list").iter().map(|x| x.as_f64().expect("word") as u32).collect()
    };
    let float = |v: &Value| f32::from_bits(v.as_f64().expect("word") as u32);
    let floats = |v: &Value| words(v).into_iter().map(f32::from_bits).collect::<Vec<_>>();
    let m16 = |v: &Value| -> [f32; 16] {
        let f = floats(v);
        std::array::from_fn(|i| f[i])
    };
    let mut out = StretchReplay::default();
    for (case, row) in doc.get("rows").and_then(Value::as_array).expect("rows").iter().enumerate() {
        let get = |k: &str| row.get(k).unwrap_or_else(|| panic!("case {case}: {k}"));
        let owner = m16(get("owner"));
        let view = StretchView {
            view: m16(get("view")),
            inverse_view: m16(get("inverseView")),
            camera_velocity: {
                let f = floats(get("cameraVelocityCamera"));
                [f[0], f[1], f[2]]
            },
        };
        let limits = floats(get("limits"));
        let scale = floats(get("scale"));
        let renderer = StretchRenderer {
            velocity_scale: float(get("velocityScale")),
            length_scale: float(get("lengthScale")),
            camera_velocity_scale: float(get("cameraVelocityScale")),
            scale: [scale[0], scale[1], scale[2]],
            limits: [limits[0], limits[1], 0.0, 0.0],
            normal_bend: float(get("t144")),
        };
        let vertices = words(get("vertices"));
        out.cases += 1;
        for (lane, q) in get("particles").as_array().expect("particles").iter().enumerate() {
            let g = |k: &str| q.get(k).unwrap_or_else(|| panic!("case {case} lane {lane}: {k}"));
            let local = floats(g("position"));
            let mut v = floats(g("velocity"));
            if animated {
                let an = floats(g("animated"));
                for k in 0..3 {
                    v[k] = a::add(v[k], an[k]);
                }
            }
            let size = floats(g("size"));
            let particle = StretchParticle {
                position: affine(&owner, [local[0], local[1], local[2]]),
                velocity: linear(&owner, [v[0], v[1], v[2]]),
                size: [size[0], size[1]],
                age_percent: float(g("age")),
            };
            let colour = g("colour").as_f64().expect("colour") as u32;
            let (quad, parts) = quad_parts(&view, &renderer, &particle);
            let norm = |x: V3| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
            let width_len = norm(std::array::from_fn(|k| quad.positions[0][k] - particle.position[k]));
            let h_len = norm(parts.h);
            let span_len = norm(parts.span_world);
            let width_gain = if parts.cross_xy2 > 0.0 { h_len * h_len / parts.cross_xy2.sqrt() } else { 0.0 };
            let span_gain = if span_len > 0.0 { norm(particle.position).max(1.0) / span_len } else { 0.0 };
            let bound_position = OPERAND_ULPS * width_gain * width_len;
            let bound_normal = OPERAND_ULPS * (width_gain + span_gain);
            let native_w = |k: usize| &vertices[(lane * 4 + k) * 9..(lane * 4 + k + 1) * 9];
            let corner = |k: usize, o: usize| -> V3 {
                let w = native_w(k);
                [f32::from_bits(w[o]), f32::from_bits(w[o + 1]), f32::from_bits(w[o + 2])]
            };
            // The quad's normals are compared where it has both extents: a
            // quad without width or length is a line or a point (drawn empty),
            // and its normals are then the normalized rounding residue of the
            // world round trip.
            let span = |x: V3, y: V3| (0..3).map(|k| (x[k] - y[k]).abs()).fold(0.0f32, f32::max);
            let has_area = span(corner(0, 0), corner(3, 0)) > 1.0e-4 && span(corner(0, 0), corner(1, 0)) > 1.0e-4;
            for k in 0..4 {
                out.vertices += 1;
                let (np, nn, w) = (corner(k, 0), corner(k, 3), native_w(k));
                let ep = (0..3).map(|i| (np[i] - quad.positions[k][i]).abs() / np[i].abs().max(1.0)).fold(0.0f32, f32::max);
                let en = span(nn, quad.normals[k]);
                out.worst_position = out.worst_position.max(ep);
                if has_area {
                    out.normals_compared += 1;
                    out.worst_normal = out.worst_normal.max(en);
                }
                if (0..3).all(|i| np[i].to_bits() == quad.positions[k][i].to_bits()) {
                    out.position_bits_equal += 1;
                }
                let uv_ok = [f32::from_bits(w[7]), f32::from_bits(w[8])] == UV[k];
                let base = ep <= TOLERANCE && (!has_area || en <= TOLERANCE);
                let bounded = ep <= TOLERANCE + bound_position && (!has_area || en <= TOLERANCE + bound_normal);
                if base {
                    out.within_base += 1;
                } else if bounded {
                    out.within_bound_only += 1;
                    let ratio = ((ep - TOLERANCE) / bound_position.max(f32::MIN_POSITIVE))
                        .max(if has_area { (en - TOLERANCE) / bound_normal.max(f32::MIN_POSITIVE) } else { 0.0 });
                    out.worst_bound_ratio = out.worst_bound_ratio.max(ratio);
                }
                let red = !bounded || w[6] != colour || !uv_ok;
                if red {
                    out.red += 1;
                    if out.first_red.is_none() {
                        out.first_red = Some(format!(
                            "case {case} lane {lane} corner {k}: position {np:?} vs {:?}, normal {nn:?} vs {:?}, colour {:#x} vs {colour:#x}, uv ({}, {}); bounds position {bound_position:e} normal {bound_normal:e}",
                            quad.positions[k], quad.normals[k], w[6], f32::from_bits(w[7]), f32::from_bits(w[8])
                        ));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(variable: &str) -> String {
        let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is unset"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", std::path::Path::new(&path).display()))
    }

    /// The native Stretch rows: every vertex position and every normal of a
    /// quad with area within the base tolerance plus the rounding bound, colour
    /// and UV words equal.
    /// Each named mutant must go red somewhere, and so must dropping the
    /// animated velocity from the drawn velocity.
    #[test]
    #[ignore = "set MOLY_STRETCH_ROWS to the native Stretch receipt rows (json)"]
    fn current_stretch_rows_within_tolerance() {
        let text = rows("MOLY_STRETCH_ROWS");
        arms::set(None);
        let base = replay_stretch_rows(&text, true);
        assert!(base.cases > 0 && base.vertices == base.cases * 16, "row inventory {base:?}");
        assert_eq!(base.red, 0, "first red: {:?}", base.first_red);
        let mut report = Vec::new();
        for mutant in ["noCameraVelocity", "trueRsqrt", "noScaleOnLength", "clampLength", "crossSign", "scaleSpan", "faceNormal"] {
            arms::set(Some(mutant));
            let run = replay_stretch_rows(&text, true);
            arms::set(None);
            report.push((mutant, run.red));
        }
        let no_animated = replay_stretch_rows(&text, false);
        report.push(("noAnimated", no_animated.red));
        eprintln!(
            "stretch rows: cases {} vertices {} red 0 (base tolerance {TOLERANCE}, operand rounding {OPERAND_ULPS:e}); within base {}, within the bound only {} (worst ratio {:.3}); worst position {:e} (relative), worst normal {:e} over {} normals; position bits equal {}; mutants red vertices {report:?}",
            base.cases, base.vertices, base.within_base, base.within_bound_only, base.worst_bound_ratio, base.worst_position,
            base.worst_normal, base.normals_compared, base.position_bits_equal
        );
        for (mutant, red) in &report {
            assert!(*red > 0, "mutant {mutant} stayed green");
        }
    }
}
