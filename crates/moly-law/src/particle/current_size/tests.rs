//! Replay of the current-size law against the engine's own SizeModule rows:
//! each row gives the module's curves, the call range, the start sizes, ages
//! and seeds, the bit the asset reader's curve build returned, the
//! polynomial words it wrote, and the current-size words the engine wrote.
//! Rows with separate axes or the 3D size are refused by the law, and each
//! such refusal must fall on a row the engine marks as taking that form;
//! every other row is compared bit for bit over the range, the law's
//! polynomial decision against the engine's bit, and each polynomial the law
//! builds against the engine's words. Each named change to the law must turn
//! at least one compared row red. The clamp of the curve time at zero has no
//! arm: dropping it changes no replayed lane (no row reads a curve at a
//! negative time where its value differs from the value at zero).

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
    /// Compared rows the law reads through the optimised polynomial.
    polynomial_rows: usize,
    /// Rows whose polynomial decision differs from the engine's bit.
    decision_mismatches: usize,
    /// Polynomial words compared with the engine's, and those that differ.
    words: usize,
    word_mismatches: usize,
    /// The first mismatching lanes: row config, lane, age, start, ours, engine.
    first_mismatches: Vec<String>,
    /// Mismatched rows per row config.
    red_by_config: std::collections::BTreeMap<String, usize>,
}

/// The engine's polynomial words of the curve at `slot` ("max" or "min"):
/// the two segments, then the switch.
fn native_words(native: &Value, slot: &str) -> Vec<u32> {
    items(field(&items(field(native, "poly"))[0], slot)).iter().map(word).collect()
}

fn law_words(p: &Polynomial) -> Vec<u32> {
    p.a.iter().chain(&p.b).chain([&p.switch]).map(|v| v.to_bits()).collect()
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
                let unordered = |c: &MinMaxCurve| match c {
                    MinMaxCurve::Curve { max, .. } => vec![&max.keys],
                    MinMaxCurve::TwoCurves { min, max, .. } => vec![&min.keys, &max.keys],
                    _ => Vec::new(),
                }
                .into_iter()
                .any(|keys| !keys.windows(2).all(|pair| pair[0].time < pair[1].time));
                let tied = match refused {
                    Refused::SeparateAxes => separate,
                    Refused::Size3d => size_3d,
                    Refused::UnorderedKeys => !built && unordered(&params.curve),
                };
                if !tied {
                    tally.refused_untied += 1;
                }
                continue;
            }
        };
        tally.compared += 1;
        let built = items(field(native, "built")).first().is_some_and(|b| word(b) != 0);
        let polynomials: Vec<(&str, &Polynomial)> = match &law.size {
            Size::Polynomial(p) => vec![("max", p)],
            Size::TwoPolynomials { min, max } => vec![("max", max), ("min", min)],
            _ => Vec::new(),
        };
        tally.polynomial_rows += usize::from(!polynomials.is_empty());
        tally.decision_mismatches += usize::from(polynomials.is_empty() == built);
        if built {
            for (slot, p) in polynomials {
                let (ours, engine) = (law_words(p), native_words(native, slot));
                tally.words += engine.len();
                tally.word_mismatches += ours.iter().zip(&engine).filter(|(a, b)| a != b).count()
                    + ours.len().abs_diff(engine.len());
            }
        }
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
                if tally.first_mismatches.len() < 8 {
                    tally.first_mismatches.push(format!("{} {}: age {:#010x} start {:#010x} ours {:#010x} engine {:#010x}",
                        field(case, "config").as_str().unwrap_or("?"), i, word(&ages[i]), word(&items(&start[i])[0]),
                        ours.to_bits(), word(&items(&current[i])[0])));
                }
            }
        }
        tally.mismatched_rows += usize::from(bad);
        if bad {
            *tally.red_by_config.entry(field(case, "config").as_str().unwrap_or("?").to_string()).or_default() += 1;
        }
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
    replay_with_arms(field(&receipt, "results"), &["hermite", "otherSalt", "noMultiplier"]);
}

/// The replay, then each arm's count of red compared rows; every arm must
/// turn at least one row red.
fn replay_with_arms(results: &Value, arms_red: &[&'static str]) -> Tally {
    let results = items(results);
    arms::set(None);
    let tally = replay(results);
    println!("current-size replay: {tally:?}");
    assert!(tally.compared > 0 && tally.lanes > 0);
    assert_eq!(tally.mismatched_rows, 0, "{tally:?}");
    assert_eq!(tally.refused_untied, 0, "{tally:?}");
    assert_eq!(tally.decision_mismatches, 0, "{tally:?}");
    assert_eq!(tally.word_mismatches, 0, "{tally:?}");
    for &arm in arms_red {
        arms::set(Some(arm));
        let broken = replay(results);
        arms::set(None);
        println!("current-size replay arm {arm}: {} of {} compared rows red {:?}", broken.mismatched_rows, broken.compared,
            broken.red_by_config);
        assert!(broken.mismatched_rows > 0, "arm {arm} stays green");
    }
    tally
}

/// The engine's SizeModule rows on the source snow curve and on generated
/// curves of zero to three keys the asset reader builds (endpoint times up to
/// 1e-4 off, stepped tangents, zero and inverted widths, multipliers of
/// either sign), two-curve pairs where both, one or neither builds, and ages
/// across the switch, the 0.99999 clamp, past 100, NaN and the infinities.
#[test]
#[ignore = "needs MOLY_SIZE_POLYNOMIAL_RECEIPT"]
fn current_size_matches_native_polynomial_rows() {
    let path = std::env::var("MOLY_SIZE_POLYNOMIAL_RECEIPT").expect("MOLY_SIZE_POLYNOMIAL_RECEIPT");
    let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        field(&receipt, "librarySha256").as_str(),
        Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
    );
    let tally = replay_with_arms(field(&receipt, "results"),
        &["genericPath", "reassociated", "swapMinMax", "extraDraw", "hermite", "otherSalt", "noMultiplier"]);
    assert!(tally.polynomial_rows > 0 && tally.words > 0, "{tally:?}");
    sampler_side_tally(items(field(&receipt, "results")));
}

/// The engine's SizeModule rows on weighted keys: the fixture effect's size
/// curve (whose first key carries the out weight) and generated one- and
/// two-curve sizes whose keys carry either weight bit, with ages over every
/// segment. A weighted key never lets the asset reader build the polynomial,
/// so every such curve is evaluated key by key; its weighted segments take
/// the evaluation's weighted branch. Every arm must turn a row red, the
/// weighted-branch arm included, and some lane must fall in a weighted
/// segment.
#[test]
#[ignore = "needs MOLY_SIZE_WEIGHTED_RECEIPT"]
fn current_size_matches_native_weighted_rows() {
    let path = std::env::var("MOLY_SIZE_WEIGHTED_RECEIPT").expect("MOLY_SIZE_WEIGHTED_RECEIPT");
    let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        field(&receipt, "librarySha256").as_str(),
        Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
    );
    let results = items(field(&receipt, "results"));
    let weighted = weighted_lanes(results);
    println!("current-size weighted rows: {} rows, lanes in a weighted segment {weighted}", results.len());
    assert!(weighted > 0, "no lane falls in a weighted segment");
    replay_with_arms(field(&receipt, "results"), &["weightedAsCubic", "otherSalt", "noMultiplier"]);
}

/// The compared lanes whose normalized age falls inside a weighted segment
/// of a key-by-key curve (strictly increasing key times, the clamp wraps).
fn weighted_lanes(results: &[Value]) -> usize {
    let mut count = 0;
    for result in results {
        let case = field(result, "case");
        let curves = items(field(case, "curves"));
        let sides: Vec<Vec<CurveKey>> = match curve(&curves[0]) {
            MinMaxCurve::Curve { max, .. } => vec![max.keys],
            MinMaxCurve::TwoCurves { min, max, .. } => vec![min.keys, max.keys],
            _ => continue,
        };
        let ages = items(field(case, "ageBits"));
        for i in int(field(case, "from"))..int(field(case, "to")) {
            let t = a::max(a::mul(f32::from_bits(word(&ages[i])), AGE_FACTOR), 0.0);
            let inside = sides.iter().any(|keys| {
                keys.len() >= 2 && keys[0].time <= t && t < keys[keys.len() - 1].time
                    && keys.windows(2).any(|pair| pair[0].time <= t && t < pair[1].time
                        && crate::particle::curve::weighted_segment(pair[0], pair[1]))
            });
            count += usize::from(inside);
        }
    }
    count
}

/// Not asserted: how the prepared curve sampler shared by the other
/// over-lifetime modules fares on the same rows (curves it does not bake
/// although the engine builds them, and lanes whose words differ from the
/// engine's where it does).
fn sampler_side_tally(results: &[Value]) {
    use crate::particle::curve::{CurveSampler, CurveTime};
    let (mut built, mut unbaked, mut lanes, mut differ) = (0, 0, 0, 0);
    let mut examples = Vec::new();
    for result in results {
        let (case, native) = (field(result, "case"), field(result, "native"));
        if word(field(case, "separateAxes")) != 0 || word(field(case, "flag7d4")) != 0 {
            continue;
        }
        if !items(field(native, "built")).first().is_some_and(|b| word(b) != 0) {
            continue;
        }
        built += 1;
        let sampler = CurveSampler::new(&curve(&items(field(case, "curves"))[0]), CurveTime::Normalized);
        let Ok(sampler @ (CurveSampler::CurveBaked(_) | CurveSampler::TwoCurvesBaked { .. })) = sampler else {
            unbaked += 1;
            continue;
        };
        let (start, ages, seeds) = (items(field(case, "startBits")), items(field(case, "ageBits")), items(field(case, "seeds")));
        let current = items(field(native, "current"));
        for i in int(field(case, "from"))..int(field(case, "to")) {
            let t = a::max(a::mul(f32::from_bits(word(&ages[i])), AGE_FACTOR), 0.0);
            let v = sampler.evaluate(t, ParticleRandom::sample(word(&seeds[i]), SIZE_SALT));
            let ours = a::mul(f32::from_bits(word(&items(&start[i])[0])), a::max(v, 0.0));
            lanes += 1;
            if ours.to_bits() != word(&items(&current[i])[0]) {
                differ += 1;
                if examples.len() < 6 {
                    examples.push(format!("{} {i}: age {:#010x} sampler {:#010x} engine {:#010x}", field(case, "config").as_str().unwrap_or("?"),
                        word(&ages[i]), ours.to_bits(), word(&items(&current[i])[0])));
                }
            }
        }
    }
    println!("curve sampler on the built rows (not asserted): built rows {built}, not baked {unbaked}, baked lanes {lanes}, lanes differing from the engine {differ} {examples:?}");
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
