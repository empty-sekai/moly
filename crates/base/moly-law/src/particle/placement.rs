//! The local rotation words a script's Transform rotation setter leaves, as the
//! engine computes them.
//!
//! `Transform.eulerAngles = e` multiplies each component by the degree-to-radian
//! constant in managed code and converts the result natively with rotation
//! order ZXY: one device `sincosf` per half angle, then a scalar product that
//! keeps its zero terms (so the signs of zero components follow the engine).
//! `Transform::SetRotation` brings that world rotation into the parent's frame
//! through every ancestor, root first (the conjugate of the ancestor's local
//! rotation times the running value, then the sign flips of that ancestor's
//! negative scale axes), and normalizes it: divided by the square root of
//! `(x*x + y*y) + (z*z + w*w)`, or the identity when that is at or below 1e-30.
//! A node without a parent skips the ancestor pass and is still normalized.
//!
//! A placed fixture's view is its prefab root. `FixtureView.ForceSetRotation`
//! writes `eulerAngles = (0, (direction & 0xff) * 90, 0)` (an integer product
//! converted exactly), and the view hangs below the site's fixture container.
//! With identity ancestors the ancestor pass changes nothing but the signs of
//! zero components, and the normalization moves the half-turn and quarter-turn
//! words by one unit in the last place (0x3f3504f3 becomes 0x3f3504f4), which is
//! why the stored rotation is not the Euler conversion's output.
//!
//! The device `sincosf` is bionic's ([`super::device_libm::sincosf`]).
use super::device_libm::sincosf;
use super::owner::SourceTrs;

/// `Mathf.Deg2Rad` as the compiled setter loads it.
const DEG2RAD: f32 = f32::from_bits(0x3c8e_fa35);
/// A squared norm at or below this normalizes to the identity.
const TINY: f32 = f32::from_bits(0x0da2_4260);

/// The managed setter's conversion: each component times `Mathf.Deg2Rad`.
pub fn degrees_to_radians(degrees: [f32; 3]) -> [f32; 3] {
    degrees.map(|v| v * DEG2RAD)
}

/// The native Euler conversion in rotation order ZXY (radians in, quaternion
/// `(x, y, z, w)` out), every operation in the engine's order.
pub fn euler_to_quaternion_zxy(radians: [f32; 3]) -> [f32; 4] {
    let sincos = |v: f32| {
        #[cfg(test)]
        if arms::on("sincosBinary64") {
            let (s, c) = (v as f64).sin_cos();
            return (s as f32, c as f32);
        }
        sincosf(v)
    };
    let (sx, cx) = sincos(radians[0] * 0.5);
    let (sy, cy) = sincos(radians[1] * 0.5);
    let (sz, cz) = sincos(radians[2] * 0.5);
    #[cfg(test)]
    let (sx, cx, sz, cz) = if arms::on("eulerOrderSwapsXZ") { (sz, cz, sx, cx) } else { (sx, cx, sz, cz) };
    let zero = 0.0f32;
    // The product of the X and Y half-angle rotations, zero terms kept.
    let a7 = cx * zero;
    let a17 = cy * zero;
    let a18 = sx * zero;
    let a16 = sy * zero;
    let px = a16 + (a7 + sx * cy);
    let py = a18 + (cx * sy + a17);
    let pz = ((a7 + a17) + zero) - sx * sy;
    let pw = (cx * cy - a18) - a16;
    // Then the Z half-angle rotation.
    let b7 = pw * zero;
    let b2 = px * zero;
    let b4 = py * zero;
    let b3 = pz * zero;
    let x = (sz * py + (cz * px + b7)) - b3;
    let y = (b3 + (cz * py + b7)) - sz * px;
    let z = (b2 + (sz * pw + cz * pz)) - b4;
    let w = ((cz * pw - b2) - b4) - sz * pz;
    [x, y, z, w]
}

/// One ancestor of `InverseTransformRotation`: the conjugate of its local
/// rotation times `q`, then the sign flips of its negative scale axes.
fn inverse_step(q: [f32; 4], node: &SourceTrs) -> [f32; 4] {
    let r = node.q;
    #[cfg(test)]
    let c = if arms::on("ancestorNotConjugated") { r } else { [-r[0], -r[1], -r[2], r[3]] };
    #[cfg(not(test))]
    let c = [-r[0], -r[1], -r[2], r[3]];
    let [x, y, z, w] = q;
    let t0 = ((x * c[1] - z * c[3]) - w * c[2]) - y * c[0];
    let t1 = ((w * c[3] - x * c[0]) - z * c[2]) - y * c[1];
    let t2 = ((y * c[2] - z * c[1]) - x * c[3]) - w * c[0];
    let t3 = ((z * c[0] - x * c[2]) - y * c[3]) - w * c[1];
    let out = [-t2, -t3, -t0, t1];
    #[cfg(test)]
    if arms::on("scaleSignIgnored") {
        return out;
    }
    let negative = node.s.map(|v| v.is_sign_negative());
    let flip = |v: f32, a: bool, b: bool| if a != b { -v } else { v };
    [flip(out[0], negative[1], negative[2]), flip(out[1], negative[0], negative[2]),
        flip(out[2], negative[0], negative[1]), out[3]]
}

/// `InverseTransformRotation` over `ancestors`, root first.
pub fn inverse_transform_rotation(ancestors: &[SourceTrs], q: [f32; 4]) -> [f32; 4] {
    ancestors.iter().fold(q, inverse_step)
}

/// `NormalizeSafe` of the local rotation store.
pub fn normalize_safe(q: [f32; 4]) -> [f32; 4] {
    #[cfg(test)]
    if arms::on("normalizeSkipped") {
        return q;
    }
    let [x, y, z, w] = q;
    let len2 = (x * x + y * y) + (z * z + w * w);
    if len2 > TINY {
        let root = len2.sqrt();
        #[cfg(test)]
        if arms::on("normalizeReciprocal") {
            let inv = 1.0 / root;
            return q.map(|v| v * inv);
        }
        q.map(|v| v / root)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// The local rotation `Transform::SetRotation(world)` stores on a node below
/// `ancestors` (root first; empty for a node without a parent).
pub fn set_rotation(ancestors: &[SourceTrs], world: [f32; 4]) -> [f32; 4] {
    normalize_safe(if ancestors.is_empty() { world } else { inverse_transform_rotation(ancestors, world) })
}

/// The local rotation of a placed fixture's view after
/// `FixtureView.ForceSetRotation(direction)` below `ancestors`.
pub fn fixture_view_rotation(direction: u8, ancestors: &[SourceTrs]) -> [f32; 4] {
    let yaw = (u32::from(direction) * 90) as f32;
    set_rotation(ancestors, euler_to_quaternion_zxy(degrees_to_radians([0.0, yaw, 0.0])))
}

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub(crate) fn on(name: &str) -> bool {
        ARM.with(|arm| arm.get() == Some(name))
    }
    pub(crate) fn set(name: Option<&'static str>) {
        ARM.with(|arm| arm.set(name));
    }
    pub(crate) const ALL: [&str; 6] = ["sincosBinary64", "eulerOrderSwapsXZ", "ancestorNotConjugated",
        "scaleSignIgnored", "normalizeSkipped", "normalizeReciprocal"];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{self, Value};

    fn at<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("missing {key}"))
    }
    fn arr(v: &Value) -> &[Value] {
        v.as_array().expect("array")
    }
    fn word(v: &Value) -> u32 {
        v.as_f64().expect("number") as u32
    }
    fn words<const N: usize>(v: &Value) -> [u32; N] {
        let list = arr(v);
        assert_eq!(list.len(), N);
        std::array::from_fn(|i| word(&list[i]))
    }
    fn floats<const N: usize>(v: &Value) -> [f32; N] {
        words::<N>(v).map(f32::from_bits)
    }
    fn node(v: &Value) -> SourceTrs {
        let parts = arr(v);
        SourceTrs { t: floats(&parts[0]), q: floats(&parts[1]), s: floats(&parts[2]) }
    }
    fn bits4(q: [f32; 4]) -> [u32; 4] {
        q.map(f32::to_bits)
    }
    fn same_or_both_nan(a: f32, b: u32) -> bool {
        a.to_bits() == b || (a.is_nan() && f32::from_bits(b).is_nan())
    }

    /// The mismatches of every receipt part under the active arm.
    fn mismatches(receipt: &Value) -> [usize; 4] {
        let mut out = [0; 4];
        for row in arr(at(receipt, "sincosf")) {
            let [x, s, c] = words::<3>(row);
            let (ps, pc) = sincosf(f32::from_bits(x));
            #[cfg(test)]
            let (ps, pc) = if arms::on("sincosBinary64") {
                let (s64, c64) = (f32::from_bits(x) as f64).sin_cos();
                (s64 as f32, c64 as f32)
            } else { (ps, pc) };
            if !same_or_both_nan(ps, s) || !same_or_both_nan(pc, c) {
                out[0] += 1;
            }
        }
        for case in arr(at(receipt, "euler")) {
            if bits4(euler_to_quaternion_zxy(floats(at(case, "v")))) != words::<4>(at(case, "q")) {
                out[1] += 1;
            }
        }
        for case in arr(at(receipt, "rotation")) {
            let ancestors: Vec<SourceTrs> = arr(at(case, "ancestors")).iter().map(node).collect();
            if bits4(set_rotation(&ancestors, floats(at(case, "q")))) != words::<4>(at(case, "local")) {
                out[2] += 1;
            }
        }
        // The product entry: the four directions under the site's identity
        // ancestors, one and two of them, against the native stores.
        let identity = SourceTrs { t: [0.0; 3], q: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] };
        for d in 0..4u8 {
            for (label, ancestors) in [("two-identity-ancestors", vec![identity; 2]), ("one-identity-ancestor", vec![identity])] {
                let name = format!("G{d}-{label}");
                let case = arr(at(receipt, "rotation")).iter()
                    .find(|c| at(c, "name").as_str() == Some(name.as_str()))
                    .unwrap_or_else(|| panic!("{name} missing"));
                if bits4(fixture_view_rotation(d, &ancestors)) != words::<4>(at(case, "local")) {
                    out[3] += 1;
                }
            }
        }
        out
    }

    #[test]
    #[ignore = "needs MOLY_FIXTURE_VIEW_RECEIPT and MOLY_FIXTURE_VIEW_BITFLIP (the native receipt of the fixture view rotation)"]
    fn fixture_view_rotation_matches_native_setters() {
        let load = |key: &str| json::parse(&std::fs::read(std::env::var(key).expect(key)).expect("receipt"))
            .expect("receipt JSON");
        let receipt = load("MOLY_FIXTURE_VIEW_RECEIPT");
        let controls = at(&receipt, "controls");
        assert_eq!(at(controls, "k1RotatedAncestorDiffers").as_bool(), Some(true), "the ancestor is read");
        assert_eq!(at(controls, "k2FlippedSinDiffers").as_bool(), Some(true), "the sincosf answer is consumed");
        assert_eq!(word(at(controls, "libunityDataBssReads")), 0, "no static-initializer input");
        let counts = [arr(at(&receipt, "sincosf")).len(), arr(at(&receipt, "euler")).len(), arr(at(&receipt, "rotation")).len()];
        let base = mismatches(&receipt);
        println!("fixture view rotation: sincosf {} / euler {} / rotation {} rows, product directions 8; mismatched {base:?}",
            counts[0], counts[1], counts[2]);
        assert_eq!(base, [0; 4]);
        for arm in arms::ALL {
            arms::set(Some(arm));
            let red = mismatches(&receipt);
            arms::set(None);
            println!("arm {arm}: mismatched {red:?}");
            assert!(red.iter().sum::<usize>() > 0, "arm {arm} stays green");
        }
        let flipped = mismatches(&load("MOLY_FIXTURE_VIEW_BITFLIP"));
        println!("bit-flipped receipt: mismatched {flipped:?}");
        assert!(flipped.iter().sum::<usize>() > 0);
    }
}
