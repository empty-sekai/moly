//! Owner matrices of a particle system: the engine's local-to-world matrix of
//! the emitting node, its general 3D inverse, the normalized global rotation,
//! the node's own rotation as a 3x3 and the emitter scale, computed from the
//! node TRS words exactly as the engine's per-frame owner update does for the
//! Local scaling mode with the default transform (no mesh-renderer shape, not
//! Custom simulation space). Hierarchy and Shape scaling, Custom space and
//! mesh-renderer shapes take other branches and are not transcribed.
//!
//! Every step is one f32 operation in the engine's association; no fused
//! multiply-add, no f64 (except the determinant threshold test, which the
//! engine itself does in double precision), no matrix library. A float64
//! inverse rounded to f32, or a composition that reassociates the products,
//! does not give the engine's words.
//!
//! The value is a pure function of the TRS words: the engine recomputes it
//! before the frame's first particle update whenever the owner transform
//! changed, and at Play, so every consumer of one frame sees that frame's
//! matrices.

/// One node of the chain in source (Unity) axes: translation, rotation
/// quaternion (x, y, z, w) and scale, each the serialized f32 word.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceTrs {
    pub t: [f32; 3],
    pub q: [f32; 4],
    pub s: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnerMatrices {
    /// Column-major local-to-world of the emitting node.
    pub local_to_world: [f32; 16],
    /// Column-major general 3D inverse of the local-to-world matrix; all
    /// +0.0 when the determinant is below the engine's threshold, which the
    /// engine then keeps using (its caller ignores the flag).
    pub world_to_local: [f32; 16],
    pub invert_ok: bool,
    /// Normalized global rotation (x, y, z, w).
    pub rotation: [f32; 4],
    /// Column-major 3x3 of the node's own normalized rotation.
    pub local_rotation: [f32; 9],
    /// Local scaling: the node's own local scale. The shape scale the same
    /// update stores next to it is (1, 1, 1) in this mode.
    pub emitter_scale: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerRefusal {
    EmptyChain,
    /// A non-finite TRS word. The engine computes through it; the NaN
    /// payloads it produces are not reproduced here.
    NonfiniteInput,
    /// A non-finite word in the result from finite inputs.
    NonfiniteOutput,
}

/// A squared quaternion norm at or below this normalizes to identity.
const TINY: f32 = f32::from_bits(0x0da2_4260);
/// The inverse's determinant threshold (1e-25 as a double): the engine
/// squares the f32 determinant in f32 and compares it in double precision.
const INVERT_LIMIT: f64 = f64::from_bits(0x3abe_f2d0_f5da_7dd9);

/// Owner matrices for Local scaling from the chain, root first, the emitting
/// node last. Starting from the node's own translation and rotation, each
/// ancestor, nearest first, scales the position component-wise by its scale,
/// rotates it and adds its translation, and multiplies the rotation by its
/// own (with the scale sign flips). Only the emitting node's scale scales the
/// matrix columns.
pub fn local_scaling_owner(chain: &[SourceTrs]) -> Result<OwnerMatrices, OwnerRefusal> {
    let (leaf, ancestors) = chain.split_last().ok_or(OwnerRefusal::EmptyChain)?;
    if chain
        .iter()
        .any(|node| node.t.iter().chain(&node.q).chain(&node.s).any(|v| !v.is_finite()))
    {
        return Err(OwnerRefusal::NonfiniteInput);
    }
    let mut position = leaf.t;
    let mut rotation = leaf.q;
    for parent in ancestors.iter().rev() {
        let scaled = std::array::from_fn(|k| position[k] * parent.s[k]);
        let rotated = rotate(parent.q, scaled);
        position = std::array::from_fn(|k| parent.t[k] + rotated[k]);
        rotation = quat_mul(parent.q, sign_flip(rotation, parent.s));
    }
    let columns = rotation_matrix(rotation);
    let mut local_to_world = [0.0; 16];
    for c in 0..3 {
        for r in 0..3 {
            local_to_world[4 * c + r] = leaf.s[c] * columns[c][r];
        }
        local_to_world[4 * c + 3] = leaf.s[c] * 0.0;
    }
    local_to_world[12..15].copy_from_slice(&position);
    local_to_world[15] = 1.0;
    let (world_to_local, invert_ok) = invert_general_3d(&local_to_world);
    let out = OwnerMatrices {
        local_to_world,
        world_to_local,
        invert_ok,
        rotation: normalize(rotation),
        local_rotation: quat_to_matrix3(normalize(leaf.q)),
        emitter_scale: leaf.s,
    };
    let finite = out
        .local_to_world
        .iter()
        .chain(&out.world_to_local)
        .chain(&out.rotation)
        .chain(&out.local_rotation)
        .chain(&out.emitter_scale)
        .all(|v| v.is_finite());
    if !finite {
        return Err(OwnerRefusal::NonfiniteOutput);
    }
    Ok(out)
}

/// The world rotation of the chain's last node (root first) as the engine's
/// transform rotation getter returns it: the node's own rotation, then each
/// ancestor, nearest first, multiplied in with the child's scale sign flips,
/// in the same association as the owner update; not normalized.
pub fn global_rotation(chain: &[SourceTrs]) -> Result<[f32; 4], OwnerRefusal> {
    let (leaf, ancestors) = chain.split_last().ok_or(OwnerRefusal::EmptyChain)?;
    if chain.iter().any(|node| node.q.iter().chain(&node.s).any(|v| !v.is_finite())) {
        return Err(OwnerRefusal::NonfiniteInput);
    }
    let mut rotation = leaf.q;
    for parent in ancestors.iter().rev() {
        rotation = quat_mul(parent.q, sign_flip(rotation, parent.s));
    }
    Ok(rotation)
}

/// The local rotation a rotation-cancelling effector writes on its node every
/// update: the inverse of its parent's world rotation (x, y and z negated, no
/// normalization), stored through the local-rotation setter, which normalizes
/// with the same sum as the owner update and falls back to the identity at or
/// below the threshold. `parent_chain` ends at the parent.
pub fn cancel_rotation(parent_chain: &[SourceTrs]) -> Result<[f32; 4], OwnerRefusal> {
    let [x, y, z, w] = global_rotation(parent_chain)?;
    Ok(normalize([-x, -y, -z, w]))
}

/// The engine's general 3D inverse of a column-major matrix whose fourth row
/// is not read. Below the determinant threshold every word is +0.0 and the
/// flag is false; a NaN determinant takes the compute path.
pub fn invert_general_3d(m: &[f32; 16]) -> ([f32; 16], bool) {
    let c0 = &m[0..4];
    let c1 = &m[4..8];
    let c2 = &m[8..12];
    let c3 = &m[12..16];
    let l0 = (c0[0] * c1[1]) * c2[2] - (c0[0] * c1[2]) * c2[1];
    let l1 = (c0[1] * c1[2]) * c2[0] - (c0[1] * c1[0]) * c2[2];
    let l2 = (c0[2] * c1[0]) * c2[1] - (c0[2] * c1[1]) * c2[0];
    let det = l2 + (l0 + l1);
    if ((det * det) as f64) < INVERT_LIMIT {
        return ([0.0; 16], false);
    }
    let inv = 1.0 / det;
    let ninv = -inv;
    let zinv = 0.0 * inv;
    let a = [
        (c1[1] * c2[2] - c1[2] * c2[1]) * inv,
        (c1[0] * c2[2] - c1[2] * c2[0]) * ninv,
        (c1[0] * c2[1] - c1[1] * c2[0]) * inv,
    ];
    let b = [
        (c0[1] * c2[2] - c0[2] * c2[1]) * (-inv),
        (c0[0] * c2[2] - c0[2] * c2[0]) * (-ninv),
        (c0[0] * c2[1] - c0[1] * c2[0]) * (-inv),
    ];
    let c = [
        (c0[1] * c1[2] - c0[2] * c1[1]) * inv,
        (c0[0] * c1[2] - c0[2] * c1[0]) * ninv,
        (c0[0] * c1[1] - c0[1] * c1[0]) * inv,
    ];
    // The fourth lane of the first three columns: zero with the sign of the
    // inverse (a vanishing difference times 0 * inv).
    let w = (c0[1] * c1[1] - c0[1] * c1[1]) * zinv;
    let columns = [[a[0], b[0], c[0], w], [a[1], b[1], c[1], w], [a[2], b[2], c[2], w]];
    let mut out = [0.0; 16];
    for (k, column) in columns.iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(column);
    }
    for r in 0..3 {
        out[12 + r] =
            -((columns[2][r] * c3[2]) + ((columns[0][r] * c3[0]) + (columns[1][r] * c3[1])));
    }
    out[15] = 1.0;
    (out, true)
}

/// v rotated by q: per component (v + a*v.x) + (b*v.y + c*v.z), where a, b
/// and c are the columns of R - I as the engine builds them.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [q0, q1, q2, q3] = q;
    let a = [
        q1 * (q1 * -2.0) - q2 * (q2 * 2.0),
        q0 * (q1 * 2.0) - q3 * (q2 * -2.0),
        q3 * (q1 * -2.0) - q0 * (q2 * -2.0),
    ];
    let b = [
        q3 * (q2 * -2.0) - q1 * (q0 * -2.0),
        q2 * (q2 * -2.0) - q0 * (q0 * 2.0),
        q1 * (q2 * 2.0) - q3 * (q0 * -2.0),
    ];
    let c = [
        q2 * (q0 * 2.0) - q3 * (q1 * -2.0),
        q3 * (q0 * -2.0) - q2 * (q1 * -2.0),
        q0 * (q0 * -2.0) - q1 * (q1 * 2.0),
    ];
    std::array::from_fn(|k| (v[k] + a[k] * v[0]) + (b[k] * v[1] + c[k] * v[2]))
}

/// The child rotation with the parent scale's sign flips: x flips when the
/// sign bits of s.y and s.z differ, y when those of s.x and s.z differ, z
/// when those of s.x and s.y differ (-0.0 counts as negative).
fn sign_flip(r: [f32; 4], s: [f32; 3]) -> [f32; 4] {
    let [nx, ny, nz] = s.map(f32::is_sign_negative);
    let flip = |v: f32, yes: bool| if yes { -v } else { v };
    [flip(r[0], ny != nz), flip(r[1], nx != nz), flip(r[2], nx != ny), r[3]]
}

/// Parent times child in the engine's association; the outer negations are
/// sign flips.
fn quat_mul(p: [f32; 4], r: [f32; 4]) -> [f32; 4] {
    let [p0, p1, p2, p3] = p;
    let [r0, r1, r2, r3] = r;
    [
        -(((p2 * r1 - p1 * r2) - p3 * r0) - p0 * r3),
        -(((p0 * r2 - p2 * r0) - p3 * r1) - p1 * r3),
        -(((p1 * r0 - p3 * r2) - p2 * r3) - p0 * r1),
        ((p3 * r3 - p0 * r0) - p2 * r2) - p1 * r1,
    ]
}

/// The rotation part of the matrix, per column; each "+ 0.0" is the engine's
/// add of +0.0, which turns a -0.0 sum into +0.0.
fn rotation_matrix(r: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = r;
    [
        [
            (y * (y * -2.0) + z * (z * -2.0)) + 1.0,
            (x * (y * 2.0) + w * (z * 2.0)) + 0.0,
            (w * (y * -2.0) + x * (z * 2.0)) + 0.0,
        ],
        [
            (w * (z * -2.0) + y * (x * 2.0)) + 0.0,
            (z * (z * -2.0) + x * (x * -2.0)) + 1.0,
            (y * (z * 2.0) + w * (x * 2.0)) + 0.0,
        ],
        [
            (z * (x * 2.0) + w * (y * 2.0)) + 0.0,
            (w * (x * -2.0) + z * (y * 2.0)) + 0.0,
            (x * (x * -2.0) + y * (y * -2.0)) + 1.0,
        ],
    ]
}

/// Divides by the f32 square root of (x*x + y*y) + (z*z + w*w); a norm at or
/// below the threshold, or a NaN one, gives the identity quaternion.
fn normalize(q: [f32; 4]) -> [f32; 4] {
    let sum = (q[0] * q[0] + q[1] * q[1]) + (q[2] * q[2] + q[3] * q[3]);
    if !(sum > TINY) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let norm = sum.sqrt();
    q.map(|v| v / norm)
}

/// Column-major 3x3 of a quaternion in the engine's operation order.
fn quat_to_matrix3(q: [f32; 4]) -> [f32; 9] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, yy, zz) = (x * x2, y * y2, z * z2);
    let (xy, xz, yz) = (x * y2, x * z2, y * z2);
    let (wx, wy, wz) = (x2 * w, y2 * w, w * z2);
    [
        1.0 - (yy + zz),
        xy + wz,
        xz - wy,
        xy - wz,
        1.0 - (xx + zz),
        yz + wx,
        xz + wy,
        yz - wx,
        1.0 - (xx + yy),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    #[test]
    fn no_finite_or_nonfinite_word_panics() {
        let words = [0.0, -0.0, 1.0, -1.0, 1e-30, 1e30, f32::MAX, f32::MIN_POSITIVE, f32::NAN,
            f32::INFINITY, f32::NEG_INFINITY, 3.5e-8];
        for &a in &words {
            for &b in &words {
                let node = SourceTrs { t: [a, b, a], q: [b, a, b, a], s: [a, a, b] };
                let _ = local_scaling_owner(&[node, node]);
                let _ = invert_general_3d(&[a, b, a, 0.0, b, a, b, 0.0, a, a, b, 0.0, b, b, a, 1.0]);
            }
        }
        assert_eq!(local_scaling_owner(&[]), Err(OwnerRefusal::EmptyChain));
    }

    fn words(value: &Value) -> Vec<u32> {
        value.as_array().expect("word array").iter()
            .map(|v| u32::try_from(v.as_f64().expect("word") as u64).expect("u32 word")).collect()
    }
    fn floats<const N: usize>(value: &Value) -> [f32; N] {
        let w = words(value);
        assert!(w.len() >= N, "native word count");
        std::array::from_fn(|i| f32::from_bits(w[i]))
    }
    fn bits(values: &[f32]) -> Vec<u32> {
        values.iter().map(|v| v.to_bits()).collect()
    }

    /// Native rows of the engine's owner update and inverse (path by
    /// environment variable). Each producer row carries a hierarchy as
    /// parent-indexed TRS nodes and the index of the emitting node; the chain
    /// is rebuilt root first by walking the parent indices, and every output
    /// word is compared bit for bit. Rows whose native output holds a NaN, or
    /// whose input holds a non-finite word, must be refused, never matched.
    #[test]
    #[ignore = "needs MOLY_OWNER_RECEIPT (native owner-update rows)"]
    fn owner_matrices_match_native_rows() {
        let path = std::env::var_os("MOLY_OWNER_RECEIPT").expect("MOLY_OWNER_RECEIPT");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("sha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let producer = receipt.get("producerCases").and_then(Value::as_array).unwrap();
        let (mut compared, mut refused) = (0, 0);
        let mut arms = [0usize; 3];
        for (row_index, row) in producer.iter().enumerate() {
            let nodes = row.get("chain").and_then(Value::as_array).unwrap();
            let mut index = row.get("idx").and_then(Value::as_f64).unwrap() as i64;
            let mut chain = Vec::new();
            while index >= 0 {
                let node = &nodes[index as usize];
                chain.push(SourceTrs {
                    t: floats(node.get("t").unwrap()),
                    q: floats(node.get("q").unwrap()),
                    s: floats(node.get("s").unwrap()),
                });
                index = node.get("parent").and_then(Value::as_f64).unwrap() as i64;
            }
            chain.reverse();
            let native = row.get("native").unwrap();
            let fields = ["localToWorld", "worldToLocal", "rotation", "localRotation3x3", "emitterScale"];
            let native_words: Vec<Vec<u32>> = fields.iter().map(|f| words(native.get(f).unwrap())).collect();
            let native_nan = native_words.iter().flatten().any(|w| f32::from_bits(*w).is_nan());
            let input_nonfinite = chain.iter()
                .any(|n| n.t.iter().chain(&n.q).chain(&n.s).any(|v| !v.is_finite()));
            let result = local_scaling_owner(&chain);
            if native_nan || input_nonfinite {
                assert!(result.is_err(), "row {row_index}: non-finite row admitted");
                refused += 1;
                continue;
            }
            let out = result.unwrap_or_else(|e| panic!("row {row_index}: refused {e:?}"));
            let ours = [bits(&out.local_to_world), bits(&out.world_to_local), bits(&out.rotation),
                bits(&out.local_rotation), bits(&out.emitter_scale)];
            for (field, (a, b)) in fields.iter().zip(ours.iter().zip(&native_words)) {
                assert_eq!(a, b, "row {row_index} {field}");
            }
            assert_eq!(bits(&out.local_to_world), words(native.get("copy").unwrap()), "row {row_index} copy");
            compared += 1;
            // Negative arms: each must differ from native on some row.
            if chain.len() > 1 {
                let leaf_only = local_scaling_owner(&chain[chain.len() - 1..]);
                if leaf_only.map_or(true, |p| bits(&p.local_to_world) != native_words[0]) {
                    arms[0] += 1;
                }
            }
            let (f64_inverse, _) = f64_invert(&out.local_to_world);
            if bits(&f64_inverse) != native_words[1] {
                arms[1] += 1;
            }
            let raw = rotation_matrix_raw(forward_rotation(&chain));
            let mut no_zero_add = out.local_to_world;
            for c in 0..3 {
                for r in 0..3 {
                    no_zero_add[4 * c + r] = chain[chain.len() - 1].s[c] * raw[c][r];
                }
            }
            if bits(&no_zero_add) != native_words[0] {
                arms[2] += 1;
            }
        }
        let invert = receipt.get("invertCases").and_then(Value::as_array).unwrap();
        let (mut inverted, mut singular, mut nan_rows) = (0, 0, 0);
        for (row_index, row) in invert.iter().enumerate() {
            let input: [f32; 16] = floats(row.get("in").unwrap());
            let native = words(row.get("native").unwrap());
            let ok = row.get("ok").and_then(Value::as_f64).unwrap() != 0.0;
            let (out, our_ok) = invert_general_3d(&input);
            if native.iter().any(|w| f32::from_bits(*w).is_nan()) {
                assert!(out.iter().any(|v| v.is_nan()), "invert row {row_index}: NaN row came out finite");
                nan_rows += 1;
                continue;
            }
            assert_eq!(our_ok, ok, "invert row {row_index} flag");
            assert_eq!(bits(&out), native, "invert row {row_index}");
            inverted += 1;
            singular += usize::from(!ok);
        }
        eprintln!("owner replay: producer compared {compared}, refused {refused}; invert compared \
            {inverted} (singular {singular}), NaN rows {nan_rows}; arms leafOnly {} f64Inverse {} noZeroAdd {}",
            arms[0], arms[1], arms[2]);
        assert_eq!(compared + refused, producer.len());
        assert!(compared > 0 && inverted > 0);
        assert!(arms.iter().all(|&n| n > 0), "every negative arm must differ from native somewhere: {arms:?}");
    }

    /// Negative arm only: a double-precision inverse rounded to f32.
    fn f64_invert(m: &[f32; 16]) -> ([f32; 16], bool) {
        let d: Vec<f64> = m.iter().map(|&v| v as f64).collect();
        let (c0, c1, c2, c3) = (&d[0..4], &d[4..8], &d[8..12], &d[12..16]);
        let det = c0[0] * (c1[1] * c2[2] - c1[2] * c2[1]) - c1[0] * (c0[1] * c2[2] - c0[2] * c2[1])
            + c2[0] * (c0[1] * c1[2] - c0[2] * c1[1]);
        if det == 0.0 {
            return ([0.0; 16], false);
        }
        let inv = [
            (c1[1] * c2[2] - c1[2] * c2[1]) / det,
            -(c0[1] * c2[2] - c0[2] * c2[1]) / det,
            (c0[1] * c1[2] - c0[2] * c1[1]) / det,
            -(c1[0] * c2[2] - c1[2] * c2[0]) / det,
            (c0[0] * c2[2] - c0[2] * c2[0]) / det,
            -(c0[0] * c1[2] - c0[2] * c1[0]) / det,
            (c1[0] * c2[1] - c1[1] * c2[0]) / det,
            -(c0[0] * c2[1] - c0[1] * c2[0]) / det,
            (c0[0] * c1[1] - c0[1] * c1[0]) / det,
        ];
        let mut out = [0.0f32; 16];
        for k in 0..3 {
            for r in 0..3 {
                out[4 * k + r] = inv[3 * k + r] as f32;
            }
        }
        for r in 0..3 {
            out[12 + r] = (-(inv[r] * c3[0] + inv[3 + r] * c3[1] + inv[6 + r] * c3[2])) as f32;
        }
        out[15] = 1.0;
        (out, true)
    }
    fn forward_rotation(chain: &[SourceTrs]) -> [f32; 4] {
        let (leaf, ancestors) = chain.split_last().unwrap();
        let mut rotation = leaf.q;
        for parent in ancestors.iter().rev() {
            rotation = quat_mul(parent.q, sign_flip(rotation, parent.s));
        }
        rotation
    }
    /// Negative arm only: the rotation matrix without the engine's +0.0 adds.
    fn rotation_matrix_raw(r: [f32; 4]) -> [[f32; 3]; 3] {
        let [x, y, z, w] = r;
        [
            [(y * (y * -2.0) + z * (z * -2.0)) + 1.0, x * (y * 2.0) + w * (z * 2.0), w * (y * -2.0) + x * (z * 2.0)],
            [w * (z * -2.0) + y * (x * 2.0), (z * (z * -2.0) + x * (x * -2.0)) + 1.0, y * (z * 2.0) + w * (x * 2.0)],
            [z * (x * 2.0) + w * (y * 2.0), w * (x * -2.0) + z * (y * 2.0), (x * (x * -2.0) + y * (y * -2.0)) + 1.0],
        ]
    }
}
