//! The source mesh-particle transform for particle arrays without 3D rotation
//! (`ParticleSystemRenderer`'s `CalculateMeshParticleTransform`, the kernel
//! body the View, World and Local render spaces share; its instructions are
//! the same in the three).
//!
//! `引擎原生逐指令`. The kernel branches once on the particle arrays'
//! 3D-rotation flag. Without 3D rotation it loads, for the particle, its
//! position, seed, age, size (X only when the size is not 3D, otherwise all
//! three), its Z rotation and its axis of rotation, each once, plus the angle
//! the draw call passes by reference; it never reads the X and Y rotations or
//! the flag argument the 3D branch negates its angles with. The basis columns
//! are loaded twice (once per matrix product below), every other input once.
//!
//! The particle's rotation is a quaternion: the Euler quaternion of the angle
//! `(0, 0, -angle)` composed with the axis-angle quaternion of the Z rotation
//! about the axis. The axis is normalised with the estimate-and-two-steps
//! reciprocal square root when its square exceeds a tiny threshold, else +Y
//! stands in. Sine and cosine are the engine's own polynomial on the argument
//! reduced to a quarter turn. The rotation matrix `R` follows from the
//! quaternion by the kernel's shuffled products. Then:
//!
//! - the particle size is mirrored per axis by the renderer flip (three
//!   successive draws of the particle seed's flip stream; an axis is mirrored
//!   when its proportion is strictly greater than its draw) and collapses to
//!   zero from an age of 100 percent;
//! - the linear part is `B * (diag(S) * R * diag(size))`, the scale applied
//!   lane by lane to each rotation column before the size, and the basis
//!   product summed as `c0 * x + (c1 * y + c2 * z)`;
//! - the translation is `A.c3 + (A.c0 * px + (A.c1 * py + A.c2 * pz))` plus
//!   the pivot offset carried through the linear columns in the same
//!   association;
//! - the normal matrix the kernel writes beside it is `B * R` (neither scale
//!   nor size).
//!
//! Every operation is one rounded single-precision operation in the kernel's
//! order and lane layout; all four lanes are kept, so the fourth row of the
//! output is the kernel's too. The sign table the quaternion product reads is
//! filled by the geometry job's static initializer at load; its two rows are
//! constants here.
//!
//! What the draw call passes: the basis is the camera basis (View), the
//! identity (World) or the emitter's rotation (Local); the angle is zero for
//! World and Local, and for View whenever the renderer allows roll (otherwise
//! View passes a camera-derived angle, not transcribed here: the caller
//! refuses that case). The Facing and Velocity spaces have their own kernel
//! bodies, not transcribed here.

use super::random::ParticleRandom;
use super::shape::native_rsqrt;

/// The render space inputs of one kernel call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshSpace {
    /// The basis `B`, three columns of four lanes. The fourth lane reaches
    /// only the fourth row of the output.
    pub basis: [[f32; 4]; 3],
    /// The affine `A` the particle position goes through, column-major; all
    /// sixteen words are read.
    pub affine: [f32; 16],
    /// The scale `S` (its fourth lane never reaches an output).
    pub scale: [f32; 3],
    /// The pivot offset: mesh bounds size times the renderer pivot, with the
    /// Z pivot negated, as the draw call computes it.
    pub offset: [f32; 3],
    /// The renderer flip proportions.
    pub flip: [f32; 3],
    /// The render-space angle the draw call passes by reference.
    pub angle: f32,
}

/// The particle channels the kernel reads without 3D rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisParticle {
    pub position: [f32; 3],
    /// Per-axis size; a system without 3D size passes its one size three times.
    pub size: [f32; 3],
    pub age_percent: f32,
    pub seed: u32,
    pub rotation_z: f32,
    /// The axis of rotation as the particle arrays carry it (the Shape
    /// module's store writes it at birth; without an enabled Shape module the
    /// Initial module writes +Z).
    pub axis: [f32; 3],
}

/// The kernel's two outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTransform {
    /// Column-major affine: three linear columns, then the translation.
    pub affine: [f32; 16],
    /// The normal matrix `B * R`, three columns of four lanes.
    pub normal: [[f32; 4]; 3],
}

/// Salt of the per-particle stream that decides renderer flipping.
const FLIP_SALT: u32 = 0xee4f_2bc1;
/// 1 / (2 pi), the angle-to-turns factor of the engine's sine.
const TURNS: f32 = f32::from_bits(0x3e22_f983);
/// Odd polynomial coefficients of the engine's sine of a quarter-turn argument.
const C1: f32 = f32::from_bits(0x4299_2322);
const C2: f32 = f32::from_bits(0x42a3_3422);
const C3: f32 = f32::from_bits(0x4225_5ddc);
const C4: f32 = f32::from_bits(0x40c9_0fda);
const C5: f32 = f32::from_bits(0x421e_a0cd);
/// The axis square the kernel requires before it normalises the axis.
const AXIS_EPSILON: f32 = f32::from_bits(0x0da2_4260);
/// The two sign-table rows the product of the no-3D-rotation branch reads.
const TABLE_A: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TABLE_B: [f32; 4] = [-1.0, 1.0, -1.0, 1.0];

fn neg(x: f32) -> f32 {
    f32::from_bits(x.to_bits() ^ 0x8000_0000)
}

/// Round to an integer by adding and removing the signed 2^23 magic number.
fn magic_round(t: f32) -> f32 {
    let magic = f32::from_bits((t.to_bits() & 0x8000_0000) | 0x4b00_0000);
    (t + magic) - magic
}

/// The quarter-turn argument of the engine's cosine of `t` turns.
fn cosine_argument(t: f32) -> f32 {
    0.25 - (t - magic_round(t)).abs()
}

/// `x * (x^8 C5 + ((C4 - x^2 C3) + x^4 (C2 - x^2 C1)))`.
fn polynomial(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    let x8 = x4 * x4;
    x * (x8 * C5 + ((C4 - x2 * C3) + x4 * (C2 - x2 * C1)))
}

/// The quaternion of the Z rotation about the axis, after the Euler
/// quaternion of `(0, 0, -angle)`, in the kernel's lane layout.
fn axis_quaternion(rotation_z: f32, axis: [f32; 3], angle: f32) -> [f32; 4] {
    // Euler quaternion of (0, 0, -angle): per lane t = (a * 0.5) * TURNS,
    // with a = 0 in the X and Y lanes.
    let t = [0.0 * TURNS, 0.0 * TURNS, (angle * -0.5) * TURNS];
    let cosine = t.map(|t| polynomial(cosine_argument(t)));
    let sine = t.map(|t| polynomial(cosine_argument(t + -0.25)));
    let b = [cosine[2] * sine[0], sine[0] * sine[2], cosine[0] * sine[2], cosine[0] * cosine[2]];
    let swapped = [b[2], b[3], b[0], b[1]];
    let f: [f32; 4] =
        std::array::from_fn(|k| TABLE_A[k] * (b[k] * cosine[1]) + (TABLE_B[k] * sine[1]) * swapped[k]);
    let e = [f[1], f[2], f[3], f[0]];
    // Axis-angle of the Z rotation: sine of half the angle in three lanes,
    // cosine in the fourth.
    let half = (rotation_z * 0.5) * TURNS;
    let lanes = [half + -0.25, half + -0.25, half + -0.25, half + 0.0];
    let w = lanes.map(|t| polynomial(cosine_argument(t)));
    let square = (axis[0] * axis[0] + axis[1] * axis[1]) + (axis[2] * axis[2] + 0.0);
    let unit = if square > AXIS_EPSILON {
        let r = native_rsqrt(square);
        axis.map(|v| v * r)
    } else {
        [0.0, 1.0, 0.0]
    };
    let [a0, a1, a2] = unit.map(|v| v * w[0]);
    let a3 = w[3];
    let [q0, q1, q2, q3] = f;
    let [e0, e1, _, e3] = e;
    let v17 = [a1 * e3, a3 * q3, a2 * e0, a0 * e1];
    let v19 = [a3 * q2, a0 * q0, a1 * q2, a2 * q0];
    let v21 = [a2 * q3, a2 * q2, a3 * q0, a3 * q1];
    let v18 = [a0 * q1, a1 * q1, a0 * q3, a1 * q3];
    let v: [f32; 4] = std::array::from_fn(|k| ((v17[k] - v19[k]) - v21[k]) - v18[k]);
    let v = [neg(v[0]), v[1], neg(v[2]), neg(v[3])];
    [v[2], v[3], v[0], v[1]]
}

/// The rotation matrix columns from the quaternion, in the kernel's lanes.
fn rotation_matrix(q: [f32; 4]) -> [[f32; 4]; 3] {
    let [q0, q1, q2, q3] = q;
    let rev = [q1, q0, q3, q2];
    let ext = [q2, q3, q0, q1];
    let rev_ext = [q3, q2, q1, q0];
    let column = |a: [f32; 4], ka: [f32; 4], sa: f32, b: [f32; 4], kb: [f32; 4], sb: f32, unit: usize| {
        std::array::from_fn(|k| (a[k] * (ka[k] * sa) + b[k] * (kb[k] * sb)) + if k == unit { 1.0 } else { 0.0 })
    };
    [
        column(rev, [-2.0, 2.0, -2.0, 0.0], q1, ext, [-2.0, 2.0, 2.0, 0.0], q2, 0),
        column(rev_ext, [-2.0, -2.0, 2.0, 0.0], q2, rev, [2.0, -2.0, 2.0, 0.0], q0, 1),
        column(ext, [2.0, -2.0, -2.0, 0.0], q0, rev_ext, [2.0, 2.0, -2.0, 0.0], q1, 2),
    ]
}

/// `B * v` as the kernel sums it: `c0 * x + (c1 * y + c2 * z)`, four lanes.
fn basis_product(basis: &[[f32; 4]; 3], v: [f32; 3]) -> [f32; 4] {
    std::array::from_fn(|k| basis[0][k] * v[0] + (basis[1][k] * v[1] + basis[2][k] * v[2]))
}

/// The kernel's transform of one particle without 3D rotation.
pub fn about_axis(space: &MeshSpace, particle: &AxisParticle) -> MeshTransform {
    let a = &space.affine;
    let [px, py, pz] = particle.position;
    let base: [f32; 4] =
        std::array::from_fn(|k| a[12 + k] + (a[k] * px + (a[4 + k] * py + a[8 + k] * pz)));
    let r = rotation_matrix(axis_quaternion(particle.rotation_z, particle.axis, space.angle));
    let draws = ParticleRandom::sample3(particle.seed, FLIP_SALT);
    let mut size: [f32; 3] = std::array::from_fn(|k| {
        let sign: f32 = if space.flip[k] > draws[k] { -1.0 } else { 1.0 };
        sign * particle.size[k]
    });
    if particle.age_percent >= 100.0 {
        size = [0.0; 3];
    }
    let linear: [[f32; 4]; 3] = std::array::from_fn(|c| {
        let scaled: [f32; 3] = std::array::from_fn(|k| (r[c][k] * space.scale[k]) * size[c]);
        basis_product(&space.basis, scaled)
    });
    let normal = r.map(|column| basis_product(&space.basis, [column[0], column[1], column[2]]));
    let [ox, oy, oz] = space.offset;
    let mut affine = [0.0; 16];
    for c in 0..3 {
        affine[4 * c..4 * c + 4].copy_from_slice(&linear[c]);
    }
    for k in 0..4 {
        affine[12 + k] = base[k] + (linear[0][k] * ox + (linear[1][k] * oy + linear[2][k] * oz));
    }
    MeshTransform { affine, normal }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Research instrument: native executions of the View, World and Local
    /// kernel body without 3D rotation (the engine's own machine code run
    /// under emulation, the geometry job's static initializer run first), one
    /// row per call: random particles and render spaces, and the inputs the
    /// product hands this law for the systems it draws about the axis (with
    /// the product's own output). Row layout, hexadecimal words: set (0 random, 1 product),
    /// body (0 View, 1 World, 2 Local), basis 12, affine 16, scale 3, offset 3,
    /// flip 3, angle, position 3, size 3, age, seed, Z rotation, axis 3, native
    /// affine 16, native normal 12, product affine 16 (zero for random rows).
    /// Every row must match word for word (NaN compared by class), both
    /// outputs, and so must the product's affine on its rows. Point
    /// MOLY_MESH_AXIS_NATIVE_ROWS at the row file.
    #[test]
    #[ignore = "set MOLY_MESH_AXIS_NATIVE_ROWS to the native mesh-axis rows, 96-word layout"]
    fn about_axis_matches_native_kernel() {
        let path = std::env::var_os("MOLY_MESH_AXIS_NATIVE_ROWS").expect("MOLY_MESH_AXIS_NATIVE_ROWS names the rows");
        let text = std::fs::read_to_string(path).unwrap();
        let same = |a: u32, b: u32| {
            let nan = |x: u32| x & 0x7f80_0000 == 0x7f80_0000 && x & 0x007f_ffff != 0;
            a == b || (nan(a) && nan(b))
        };
        let (mut rows, mut product_rows, mut rotated, mut rotated_product, mut mismatches) = (0, 0, 0, 0, Vec::new());
        for line in text.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
            let w: Vec<u32> = line.split_whitespace().map(|word| u32::from_str_radix(word, 16).unwrap()).collect();
            assert_eq!(w.len(), 96, "row width");
            let f = |i: usize| f32::from_bits(w[i]);
            let space = MeshSpace {
                basis: std::array::from_fn(|c| std::array::from_fn(|k| f(2 + 4 * c + k))),
                affine: std::array::from_fn(|i| f(14 + i)),
                scale: std::array::from_fn(|k| f(30 + k)),
                offset: std::array::from_fn(|k| f(33 + k)),
                flip: std::array::from_fn(|k| f(36 + k)),
                angle: f(39),
            };
            let particle = AxisParticle {
                position: std::array::from_fn(|k| f(40 + k)),
                size: std::array::from_fn(|k| f(43 + k)),
                age_percent: f(46),
                seed: w[47],
                rotation_z: f(48),
                axis: std::array::from_fn(|k| f(49 + k)),
            };
            let out = about_axis(&space, &particle);
            let ours: Vec<u32> = out.affine.iter().chain(out.normal.iter().flatten()).map(|x| x.to_bits()).collect();
            let native = &w[52..80];
            if (0..28).any(|i| !same(ours[i], native[i])) {
                mismatches.push(format!("row {rows}: native {native:08x?} ours {ours:08x?}"));
            }
            if w[0] == 1 {
                product_rows += 1;
                if (0..16).any(|i| !same(w[80 + i], native[i])) {
                    mismatches.push(format!("row {rows}: product affine {:08x?} native {:08x?}", &w[80..96], &native[..16]));
                }
            }
            // The normal output differs in value from the basis exactly when R
            // is not the identity (compared as numbers: a signed zero is no turn).
            if (0..12).any(|i| f32::from_bits(native[16 + i]) != f32::from_bits(w[2 + i])) {
                rotated += 1;
                rotated_product += w[0];
            }
            rows += 1;
        }
        eprintln!("mesh axis: {rows} rows ({product_rows} product), R not the identity in {rotated} ({rotated_product} product), {} mismatched",
            mismatches.len());
        assert!(mismatches.is_empty(), "{} of {rows} mismatched:\n{}", mismatches.len(),
            mismatches.iter().take(8).cloned().collect::<Vec<_>>().join("\n"));
        assert!(rows > 0 && rotated > 0, "rows={rows} rotated={rotated}");
    }
}
