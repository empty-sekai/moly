//! Replay of the current-size law against the engine's own SizeModule rows:
//! each row gives the module's curves, the call range, the start sizes, ages
//! and seeds, and the current-size words the engine wrote. Rows with a curve
//! the asset reader optimises, separate axes or the 3D size are refused by
//! the law, and each such refusal must fall on a row the engine marks as
//! taking that form; every other row is compared bit for bit over the range.
//! Each named change to the law must turn at least one compared row red. The
//! clamp of the curve time at zero has no arm: dropping it changes no
//! replayed lane (no row reads a curve at a negative time where its value
//! differs from the value at zero).

use super::*;
use crate::particle::json::{parse, Value};

fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
    v.get(key).unwrap_or_else(|| panic!("row field {key}"))
}
fn items(v: &Value) -> &[Value] {
    v.as_array().expect("row array")
}
fn word(v: &Value) -> u32 {
    let x = v.as_f64().expect("row integer");
    assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
    x as u32
}
fn int(v: &Value) -> usize {
    word(v) as usize
}

fn keys(v: &Value) -> Vec<CurveKey> {
    items(v)
        .iter()
        .map(|key| {
            let w: Vec<u32> = items(key).iter().map(word).collect();
            assert_eq!(w.len(), 7, "key words");
            CurveKey {
                time: f32::from_bits(w[0]),
                value: f32::from_bits(w[1]),
                in_slope: f32::from_bits(w[2]),
                out_slope: f32::from_bits(w[3]),
                weighted_mode: w[4] as u8,
                in_weight: f32::from_bits(w[5]),
                out_weight: f32::from_bits(w[6]),
            }
        })
        .collect()
}

/// One curve of a row as the law's curve type; the clamp wraps are asserted.
fn curve(v: &Value) -> MinMaxCurve {
    for key in ["pre", "post"] {
        if let Some(wrap) = v.get(key) {
            assert_eq!(word(wrap), 2, "only the clamp wrap is replayed");
        }
    }
    let scalar = f32::from_bits(word(field(v, "scalarBits")));
    match word(field(v, "mode")) {
        0 => MinMaxCurve::Constant(scalar),
        3 => MinMaxCurve::TwoConstants { min: f32::from_bits(word(field(v, "minScalarBits"))), max: scalar },
        1 => MinMaxCurve::Curve { multiplier: scalar, max: Curve { multiplier: 1.0, keys: keys(field(v, "maxKeys")), pre_wrap: None, post_wrap: None } },
        2 => MinMaxCurve::TwoCurves {
            multiplier: scalar,
            min: Curve { multiplier: 1.0, keys: keys(field(v, "minKeys")), pre_wrap: None, post_wrap: None },
            max: Curve { multiplier: 1.0, keys: keys(field(v, "maxKeys")), pre_wrap: None, post_wrap: None },
        },
        mode => panic!("curve mode {mode}"),
    }
}

#[derive(Default, Debug)]
struct Tally {
    rows: usize,
    compared: usize,
    lanes: usize,
    mismatched_rows: usize,
    refused: usize,
    refused_untied: usize,
}

fn replay(results: &[Value]) -> Tally {
    let mut tally = Tally::default();
    for result in results {
        tally.rows += 1;
        let case = field(result, "case");
        let native = field(result, "native");
        let curves = items(field(case, "curves"));
        let separate = word(field(case, "separateAxes")) != 0;
        let size_3d = word(field(case, "flag7d4")) != 0;
        let params = SizeOverLifetimeParams {
            separate_axes: separate,
            curve: curve(&curves[0]),
            y: curves.get(1).map(curve),
            z: curves.get(2).map(curve),
        };
        let law = match CurrentSizeLaw::from_params(&params, size_3d) {
            Ok(law) => law,
            Err(refused) => {
                tally.refused += 1;
                let built = items(field(native, "built")).iter().any(|b| word(b) != 0);
                let tied = match refused {
                    Refused::OptimisedCurve => built,
                    Refused::SeparateAxes => separate,
                    Refused::Size3d => size_3d,
                    Refused::WeightedKey => false,
                };
                if !tied {
                    tally.refused_untied += 1;
                }
                continue;
            }
        };
        tally.compared += 1;
        let (from, to) = (int(field(case, "from")), int(field(case, "to")));
        let start = items(field(case, "startBits"));
        let ages = items(field(case, "ageBits"));
        let seeds = items(field(case, "seeds"));
        let current = items(field(native, "current"));
        let mut bad = false;
        for i in from..to {
            let ours = law.current(f32::from_bits(word(&items(&start[i])[0])), f32::from_bits(word(&ages[i])), word(&seeds[i]));
            tally.lanes += 1;
            if ours.to_bits() != word(&items(&current[i])[0]) {
                bad = true;
            }
        }
        tally.mismatched_rows += usize::from(bad);
    }
    tally
}

#[test]
#[ignore = "needs MOLY_SIZE_RECEIPT"]
fn current_size_matches_native_size_module_rows() {
    let path = std::env::var("MOLY_SIZE_RECEIPT").expect("MOLY_SIZE_RECEIPT");
    let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        field(&receipt, "librarySha256").as_str(),
        Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
    );
    let results = items(field(&receipt, "results"));
    arms::set(None);
    let tally = replay(results);
    println!("current-size replay: {tally:?}");
    assert!(tally.compared > 0 && tally.lanes > 0);
    assert_eq!(tally.mismatched_rows, 0, "{tally:?}");
    assert_eq!(tally.refused_untied, 0, "{tally:?}");
    for arm in ["hermite", "otherSalt", "noMultiplier"] {
        arms::set(Some(arm));
        let broken = replay(results);
        arms::set(None);
        println!("current-size replay arm {arm}: {} of {} compared rows red", broken.mismatched_rows, broken.compared);
        assert!(broken.mismatched_rows > 0, "arm {arm} stays green");
    }
}

#[test]
fn no_finite_or_nonfinite_input_panics() {
    let flat = |t: f32, v: f32| CurveKey { time: t, value: v, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 };
    let keys = vec![flat(0.0, 0.0), flat(0.2, 1.0), flat(0.8, 1.0), flat(1.0, 0.0)];
    let law = CurrentSizeLaw::from_params(&SizeOverLifetimeParams {
        separate_axes: false,
        curve: MinMaxCurve::Curve { multiplier: 1.0, max: Curve { multiplier: 1.0, keys, pre_wrap: None, post_wrap: None } },
        y: None,
        z: None,
    }, false).unwrap();
    for age in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0, 0.0, 1e-30, 50.0, 100.0, 1e30, f32::MAX] {
        for start in [0.0, 1.2, f32::MAX, f32::NAN] {
            let _ = law.current(start, age, 0x1234_5678);
        }
    }
}
