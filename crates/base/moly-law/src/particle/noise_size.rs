//! The size streams at the collision points: which one the CollisionModule
//! reads, and whether the Noise module writes it.
//!
//! The collision radius reads the current-size stream when the particle state
//! carries one, else the start size. The engine gives the state that stream
//! when it allocates the particle arrays, for an enabled SizeModule, an
//! enabled SizeBySpeed module, or an enabled Noise module whose size amount's
//! scalar is greater than zero (zero of either sign, a negative scalar and NaN
//! do not; the same branch gives the state the noise output streams).
//!
//! The Noise module's size write runs after the Size and SizeBySpeed writes at
//! both points where the engine writes the stream. It returns before touching
//! a particle when the size amount's scalar compares equal to zero (either
//! sign; NaN does not compare equal), and when the state carries no noise
//! output streams. Otherwise, for a constant amount `s`, it rewrites the
//! current size (three axes with a 3D size, else X) as
//! `size * fmax(noise * (s * 0.5) + 1, 0)` from the current size when a Size
//! or SizeBySpeed write ran first, else from the start size, in groups of
//! four lanes; that write is not ported.
//!
//! The scalar both tests read is the curve's serialized scalar: the constant,
//! the maximum of two constants, or the multiplier of one or two curves.
//!
//! So a Noise module whose size amount's scalar is zero neither gives the
//! state the current-size stream nor writes it, and the collision reads what
//! the Size and SizeBySpeed writes left.

use super::collision_query::arms;
use crate::particle::MinMaxCurve;

/// The scalar the engine tests: the constant, the maximum of two constants,
/// or the multiplier of one or two curves.
fn size_scalar(size_amount: &MinMaxCurve) -> f32 {
    match size_amount {
        MinMaxCurve::Constant(value) => *value,
        MinMaxCurve::TwoConstants { min, max } => if arms::on("noiseTestsMinimum") { *min } else { *max },
        MinMaxCurve::Curve { multiplier, .. } | MinMaxCurve::TwoCurves { multiplier, .. } => *multiplier,
    }
}

/// Whether an enabled Noise module with this size amount gives the particle
/// state the current-size stream (and the noise output streams).
fn noise_gives_streams(size_amount: &MinMaxCurve) -> bool {
    let s = size_scalar(size_amount);
    if arms::on("noiseStreamsOnNonZero") {
        s != 0.0
    } else if arms::on("noiseStreamsOnAtLeastZero") {
        s >= 0.0
    } else if arms::on("noiseStreamsUnlessAtMostZero") {
        !(s <= 0.0)
    } else {
        s > 0.0
    }
}

/// Whether the CollisionModule reads the current-size stream (else the start
/// size), for the enabled size modules and the enabled Noise module's size
/// amount.
pub fn reads_current_size(size_over_lifetime: bool, size_by_speed: bool, noise_size_amount: Option<&MinMaxCurve>)
    -> bool {
    size_over_lifetime
        || (size_by_speed && !arms::on("noiseIgnoresSizeBySpeed"))
        || noise_size_amount.is_some_and(noise_gives_streams)
}

/// Whether an enabled Noise module with this size amount may write the
/// current-size stream: its scalar does not compare equal to zero. (A
/// negative or NaN scalar writes only when the renderer's vertex streams gave
/// the state the noise output streams; this answers yes for both.)
pub fn noise_writes_size(size_amount: &MinMaxCurve) -> bool {
    let s = size_scalar(size_amount);
    if arms::on("noiseWritesOnPositive") {
        s > 0.0
    } else if arms::on("noiseWritesUnlessBitsZero") {
        s.to_bits() != 0
    } else {
        !(s == 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::armf as a;
    use crate::particle::json::{parse, Value};
    use crate::particle::Curve;
    use std::collections::BTreeMap;

    fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
        v.get(key).unwrap_or_else(|| panic!("row field {key}"))
    }
    fn items(v: &Value) -> &[Value] {
        v.as_array().expect("row array")
    }
    fn int(v: &Value) -> i64 {
        let x = v.as_f64().expect("row integer");
        assert!(x.fract() == 0.0 && x.abs() <= 9.0e15, "row integer {x}");
        x as i64
    }
    fn word(v: &Value) -> u32 {
        int(v) as u32
    }
    fn flag(v: &Value, key: &str) -> bool {
        int(field(v, key)) != 0
    }
    fn words(v: &Value) -> Vec<u32> {
        items(v).iter().map(word).collect()
    }
    fn streams(v: &Value) -> BTreeMap<u32, Vec<u32>> {
        v.as_object().expect("streams").iter().map(|(k, v)| (k.parse().expect("offset"), words(v))).collect()
    }
    fn read_rows(var: &str) -> Value {
        let path = std::env::var(var).unwrap_or_else(|_| panic!("{var}"));
        parse(&std::fs::read(&path).expect("read rows")).expect("parse rows")
    }

    /// The row's size amount: the mode word and the two scalars the engine
    /// stores (the curves are not read on the paths compared here).
    fn amount(row: &Value) -> MinMaxCurve {
        let scalar = f32::from_bits(word(field(row, "scalarBits")));
        let min = f32::from_bits(word(field(row, "minBits")));
        let unread = || Curve { multiplier: 1.0, keys: Vec::new(), pre_wrap: None, post_wrap: None };
        match int(field(row, "mode")) {
            0 => MinMaxCurve::Constant(scalar),
            1 => MinMaxCurve::Curve { multiplier: scalar, max: unread() },
            2 => MinMaxCurve::TwoCurves { multiplier: scalar, min: unread(), max: unread() },
            3 => MinMaxCurve::TwoConstants { min, max: scalar },
            other => panic!("mode {other}"),
        }
    }

    /// The research model of the constant-amount write (not product code):
    /// per axis, groups of four lanes from `from` while the group start is
    /// below `to`, only the recorded lanes compared.
    fn constant_write(row: &Value, scalar: f32, mut after: BTreeMap<u32, Vec<u32>>) -> BTreeMap<u32, Vec<u32>> {
        let before = after.clone();
        let (from, to) = (int(field(row, "from")) as usize, int(field(row, "to")) as usize);
        let axes = if flag(row, "size3d") { 3 } else { 1 };
        let from_current = flag(row, "multiply") && !arms::on("modelFromStart");
        let half = a::mul(scalar, 0.5);
        for axis in 0..axes {
            let offset = 32 * axis as u32;
            let source = &before[&(if from_current { 768 } else { 672 } + offset)];
            let noise = words(&items(field(row, "noise"))[axis]);
            let current = after.get_mut(&(768 + offset)).expect("current size");
            let mut group = from;
            while group < to {
                for i in group..(group + 4).min(current.len()) {
                    let factor = a::max(a::add(a::mul(f32::from_bits(noise[i]), half), 1.0), 0.0);
                    current[i] = a::mul(f32::from_bits(source[i]), factor).to_bits();
                }
                group += 4;
            }
        }
        after
    }

    /// The size streams after the Noise size write as the law says, with the
    /// model for a constant amount; `None` when the law says the write may
    /// run and the amount is not a constant (that write is not ported).
    fn predict(row: &Value) -> Option<BTreeMap<u32, Vec<u32>>> {
        let before = streams(field(row, "before"));
        let size_amount = amount(row);
        if !noise_writes_size(&size_amount) {
            return Some(before);
        }
        let MinMaxCurve::Constant(scalar) = size_amount else {
            return None;
        };
        if !flag(row, "noiseFlag") && !arms::on("modelIgnoresStreams") {
            return Some(before);
        }
        Some(constant_write(row, scalar, before))
    }

    const WRITE_ARMS: [&str; 5] = ["noiseWritesOnPositive", "noiseWritesUnlessBitsZero", "noiseTestsMinimum",
        "modelFromStart", "modelIgnoresStreams"];

    /// The native Noise size writes (every size-amount mode with a zero
    /// scalar of either sign; constant and two-constant amounts with
    /// positive, negative, tiny, infinite and NaN scalars; the noise output
    /// streams present and absent; the multiply and 3D flags; ranges): where
    /// the law says no write, every size stream is the native one before and
    /// after; where it says a constant amount writes, the model's streams are
    /// the native ones bit for bit. Each named variant differs on some row.
    #[test]
    #[ignore = "needs MOLY_NOISE_SIZE_ROWS"]
    fn noise_size_rows_match_native_bits() {
        let doc = read_rows("MOLY_NOISE_SIZE_ROWS");
        let rows = items(field(&doc, "rows"));
        let (mut compared, mut differing, mut unported, mut kept, mut written) = (0, 0, 0, 0, 0);
        let mut red: BTreeMap<&str, usize> = BTreeMap::new();
        for row in rows {
            arms::set(None);
            let native = streams(field(row, "after"));
            match predict(row) {
                Some(predicted) => {
                    compared += 1;
                    differing += usize::from(predicted != native);
                    if !noise_writes_size(&amount(row)) {
                        kept += 1;
                    } else if predicted != streams(field(row, "before")) {
                        written += 1;
                    }
                }
                None => unported += 1,
            }
            for arm in WRITE_ARMS {
                arms::set(Some(arm));
                *red.entry(arm).or_default() += usize::from(predict(row).is_some_and(|p| p != native));
            }
            arms::set(None);
        }
        println!("noise size rows: {} rows, compared {compared}, differing {differing}, unported {unported}; \
            law says no write {kept}; rows the model wrote and matched {written}; arms red {red:?}", rows.len());
        assert_eq!(differing, 0, "rows differ from native");
        assert!(kept > 0 && written > 0, "positive control");
        for arm in WRITE_ARMS {
            assert!(red[arm] > 0, "arm {arm} never differs from native");
        }
    }

    const FLAG_ARMS: [&str; 4] = ["noiseStreamsOnNonZero", "noiseStreamsOnAtLeastZero", "noiseStreamsUnlessAtMostZero",
        "noiseIgnoresSizeBySpeed"];

    /// The native particle-array allocation over every combination of the 3D
    /// start size, the SizeModule and the SizeBySpeed module (each with and
    /// without separate axes), the Noise module with seven size-amount
    /// scalars, and the prior bytes: the current-size byte after it is the
    /// byte before it or `reads_current_size`, and the noise output byte the
    /// byte before it or the Noise module's own term, on every row. Each
    /// named variant differs on some row.
    #[test]
    #[ignore = "needs MOLY_SIZE_FLAGS_ROWS"]
    fn size_flag_rows_match_native() {
        let doc = read_rows("MOLY_SIZE_FLAGS_ROWS");
        let rows = items(field(&doc, "rows"));
        let run = |row: &Value| -> (bool, bool) {
            let before = field(row, "before");
            let noise = flag(row, "noise").then(|| MinMaxCurve::Constant(f32::from_bits(word(field(row, "noiseScalarBits")))));
            let current = flag(before, "0x7d2")
                || reads_current_size(flag(row, "size"), flag(row, "sizeBySpeed"), noise.as_ref());
            let streams = flag(before, "0x7d7") || noise.as_ref().is_some_and(noise_gives_streams);
            (current, streams)
        };
        let (mut differing, mut by_noise, mut streams_set) = (0, 0, 0);
        let mut red: BTreeMap<&str, usize> = BTreeMap::new();
        for row in rows {
            arms::set(None);
            let after = field(row, "after");
            let native = (flag(after, "0x7d2"), flag(after, "0x7d7"));
            differing += usize::from(run(row) != native);
            let before = field(row, "before");
            by_noise += usize::from(native.0 && !flag(before, "0x7d2") && !flag(row, "size") && !flag(row, "sizeBySpeed"));
            streams_set += usize::from(native.1 && !flag(before, "0x7d7"));
            for arm in FLAG_ARMS {
                arms::set(Some(arm));
                *red.entry(arm).or_default() += usize::from(run(row) != native);
            }
            arms::set(None);
        }
        println!("size flag rows: {} rows, differing {differing}; current size set by the Noise module alone {by_noise}; \
            noise output streams set {streams_set}; arms red {red:?}", rows.len());
        assert_eq!(differing, 0, "rows differ from native");
        assert!(by_noise > 0 && streams_set > 0, "positive control");
        for arm in FLAG_ARMS {
            assert!(red[arm] > 0, "arm {arm} never differs from native");
        }
    }

    /// A constant zero size amount and its negative zero
    /// leave both streams; a positive constant gives and writes them.
    #[test]
    fn zero_size_amount_leaves_the_streams() {
        for zero in [0.0, -0.0] {
            let amount = MinMaxCurve::Constant(zero);
            assert!(!noise_writes_size(&amount) && !reads_current_size(false, false, Some(&amount)));
        }
        let positive = MinMaxCurve::Constant(0.5);
        assert!(noise_writes_size(&positive) && reads_current_size(false, false, Some(&positive)));
        let negative = MinMaxCurve::TwoConstants { min: 1.0, max: -0.5 };
        assert!(noise_writes_size(&negative) && !reads_current_size(false, false, Some(&negative)));
    }
}
