//! The environment colour blend: `BlendExtension.BlendWith(Color)`, which tail-calls
//! `MysekaiColorUtility.Lerp(a, b, t)`.
//!
//! `EnvironmentShaderView.OnUpdate` blends seven light colours of the current and the
//! next environment config through it every frame (directional light and phenomena
//! light, skin and body shade, phenomena shade, the two drop-shadow colours). The call
//! is unconditional: after a cross-fade `SetEnvironmentData` clears the next config and
//! resets the progress to 0, and `OnUpdate` then blends the current config with itself.
//! So the steady-state globals are the round trip below, not the stored colour.
//!
//! `Lerp` branches on `QualitySettings.activeColorSpace`:
//! * `Gamma` (0): `Mathf.GammaToLinearSpace` on r, g and b of both colours (alpha passes
//!   through), a per-channel `la + t * (lb - la)` on all four channels, then
//!   `Mathf.LinearToGammaSpace` on r, g and b.
//! * any other value: the plain per-channel lerp on the stored values.
//!
//! In both branches the weight is `t' = if t >= 0 { min(t, 1) } else { 0 }`: one
//! comparison against zero and one `fmin`, so a NaN weight stays NaN (`fmin` returns
//! NaN), unlike `f32::min`, which would return 1.
//!
//! The JP player data sets the active colour space to Gamma, so the product blends on
//! the gamma arm. The two engine conversions are the native free functions behind the
//! `Mathf` internal calls; [`super::sky::gamma_to_linear`] and
//! [`super::sky::linear_to_gamma`] carry them (three and four branches, the single-precision
//! constants of the native bodies), and the value receipt below executes those native bodies
//! too.

use super::sky::{gamma_to_linear, linear_to_gamma};

/// `QualitySettings.activeColorSpace` as the blend reads it: only `Gamma` (enum value 0)
/// takes the conversion arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Gamma,
    Linear,
}

/// The colour space of the JP player (`PlayerSettings` active colour space 0 = Gamma).
pub const PLAYER_COLOR_SPACE: ColorSpace = ColorSpace::Gamma;

/// The clamp of both arms: compare with zero, then `fmin(t, 1)` on the non-negative
/// side (the unordered comparison of a NaN weight also takes that side).
fn weight(t: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t.is_nan() || t <= 1.0 {
        t
    } else {
        1.0
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

/// `MysekaiColorUtility.Lerp(a, b, t)` (reached through `BlendExtension.BlendWith`).
pub fn color_lerp(space: ColorSpace, a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = weight(t);
    match space {
        ColorSpace::Linear => [0, 1, 2, 3].map(|i| mix(a[i], b[i], t)),
        ColorSpace::Gamma => {
            let la = [
                gamma_to_linear(a[0]),
                gamma_to_linear(a[1]),
                gamma_to_linear(a[2]),
                a[3],
            ];
            let lb = [
                gamma_to_linear(b[0]),
                gamma_to_linear(b[1]),
                gamma_to_linear(b[2]),
                b[3],
            ];
            let m = [0, 1, 2, 3].map(|i| mix(la[i], lb[i], t));
            [
                linear_to_gamma(m[0]),
                linear_to_gamma(m[1]),
                linear_to_gamma(m[2]),
                m[3],
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(value: &super::super::json::Value) -> u32 {
        u32::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    /// Value by value against the executed game and engine code: the blend entry, the
    /// colour helpers and the native `Mathf` conversions run in an emulator with the
    /// active colour space set to Gamma (and, as a control arm, Linear). Every
    /// non-NaN output channel must equal the port bit for bit; NaN outputs must stay
    /// NaN (payload bits differ between the emulated core and this host).
    #[test]
    #[ignore = "MOLY_COLOR_LERP_NATIVE must name the executed colour-lerp receipt"]
    fn color_lerp_matches_the_native_blend() {
        let path = std::env::var("MOLY_COLOR_LERP_NATIVE")
            .expect("MOLY_COLOR_LERP_NATIVE must name the executed colour-lerp receipt");
        let receipt = super::super::json::parse(&std::fs::read(path).unwrap()).unwrap();
        // The emulated colour-space query must have returned what each arm names; a
        // stale translation of the rewritten stub once ran the Gamma arm twice.
        let stub = receipt.get("colourSpaceStubReturns").unwrap();
        for (arm, value) in [("gammaArm", "0"), ("linearArm", "1")] {
            let seen = stub.get(arm).unwrap().as_object().unwrap();
            assert!(
                seen.len() == 1 && seen[0].0 == value,
                "{arm}: colour-space stub returned {seen:?}"
            );
        }
        let mut compared = 0usize;
        let mut plain_differs = 0usize;
        let mut round_trip_differs = 0usize;
        let mut linear_not_gamma = 0usize;
        for (arm, space) in [
            ("lerpGamma", ColorSpace::Gamma),
            ("lerpLinear", ColorSpace::Linear),
        ] {
            let rows = receipt.get(arm).unwrap().as_array().unwrap();
            assert!(!rows.is_empty(), "{arm}");
            for row in rows {
                let v: Vec<u32> = row.as_array().unwrap().iter().map(bits).collect();
                assert_eq!(v.len(), 13, "{arm}");
                let f = |i: usize| f32::from_bits(v[i]);
                let (a, b, t) = ([f(0), f(1), f(2), f(3)], [f(4), f(5), f(6), f(7)], f(8));
                let ours = color_lerp(space, a, b, t);
                let plain = color_lerp(ColorSpace::Linear, a, b, t);
                let converted = color_lerp(ColorSpace::Gamma, a, b, t);
                for c in 0..4 {
                    let native = f32::from_bits(v[9 + c]);
                    if native.is_nan() {
                        assert!(
                            ours[c].is_nan(),
                            "{arm} row {v:08x?} channel {c}: native NaN, ours {}",
                            ours[c]
                        );
                    } else {
                        assert_eq!(
                            ours[c].to_bits(),
                            v[9 + c],
                            "{arm} row {v:08x?} channel {c}"
                        );
                    }
                    compared += 1;
                    if space == ColorSpace::Gamma && !native.is_nan() {
                        plain_differs += usize::from(plain[c].to_bits() != v[9 + c]);
                        round_trip_differs += usize::from(t == 0.0 && c < 3 && v[c] != v[9 + c]);
                    }
                    if space == ColorSpace::Linear && !native.is_nan() {
                        linear_not_gamma += usize::from(converted[c].to_bits() != v[9 + c]);
                    }
                }
            }
        }
        // Positive arms: the gamma arm is not the plain lerp on these inputs, and even
        // a zero weight changes some stored channels through the round trip.
        assert!(
            plain_differs > 0,
            "gamma arm indistinguishable from the plain lerp"
        );
        assert!(
            round_trip_differs > 0,
            "no zero-weight round-trip change in the receipt"
        );
        assert!(
            linear_not_gamma > 0,
            "the linear control arm is indistinguishable from the gamma arm"
        );
        eprintln!("compared {compared} channels; plain lerp differs from the gamma arm on {plain_differs}; zero-weight round trip changes {round_trip_differs}; the conversion differs from the linear arm on {linear_not_gamma}");
    }

    /// The two conversions alone against the native `Mathf` bodies.
    #[test]
    #[ignore = "MOLY_COLOR_LERP_NATIVE must name the executed colour-lerp receipt"]
    fn engine_conversions_match_the_native_mathf_bodies() {
        let path = std::env::var("MOLY_COLOR_LERP_NATIVE")
            .expect("MOLY_COLOR_LERP_NATIVE must name the executed colour-lerp receipt");
        let receipt = super::super::json::parse(&std::fs::read(path).unwrap()).unwrap();
        for (key, port) in [
            ("g2l", gamma_to_linear as fn(f32) -> f32),
            ("l2g", linear_to_gamma),
        ] {
            let rows = receipt.get(key).unwrap().as_array().unwrap();
            assert!(rows.len() > 1000, "{key}");
            for row in rows {
                let row = row.as_array().unwrap();
                let (x, native) = (bits(&row[0]), bits(&row[1]));
                let ours = port(f32::from_bits(x));
                if f32::from_bits(native).is_nan() {
                    assert!(ours.is_nan(), "{key} {x:#010x}");
                } else {
                    assert_eq!(ours.to_bits(), native, "{key} {x:#010x}");
                }
            }
        }
    }
}
