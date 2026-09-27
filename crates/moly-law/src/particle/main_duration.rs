//! `ParticleSystem.main.duration = value`: the MainModule duration setter.
//!
//! The engine's setter binding writes through `ParticleSystem::SetLengthInSec`
//! ([`set_length_in_sec`]) and then, on a system that is not stopped, raises a
//! state byte whose every reader in the engine's particle code decides the
//! procedural path (the procedural support query, Play, Simulate's restart
//! warm, the bounds update and the invisible-renderer pause): a duration
//! written while the system runs turns procedural simulation off for it. The
//! write is never refused or deferred, playing or not.

/// `ParticleSystem::SetLengthInSec`: a value equal to the stored duration (an
/// ordered float equality, so +0 equals -0 and a NaN never equals) leaves it
/// as it is; any other value is stored clamped: below 0.05 (an ordered
/// comparison) 0.05, otherwise the machine `fmin` with 100000, which returns a
/// NaN operand (quieted) rather than the other one.
pub fn set_length_in_sec(stored: f32, value: f32) -> f32 {
    if stored == value {
        return stored;
    }
    if value < 0.05 {
        return 0.05;
    }
    machine_fmin(value, 100_000.0)
}

/// The machine `fmin` on single floats: a NaN operand is returned quieted;
/// of two zeros the negative one; otherwise the smaller.
fn machine_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return f32::from_bits(a.to_bits() | 0x0040_0000);
    }
    if b.is_nan() {
        return f32::from_bits(b.to_bits() | 0x0040_0000);
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn word(value: &Value, key: &str) -> u32 {
        u32::try_from(value.get(key).and_then(Value::as_f64).expect(key) as u64).expect("u32 word")
    }

    #[test]
    fn clamps_and_keeps_equal_values() {
        assert_eq!(set_length_in_sec(1.0, 0.01), 0.05);
        assert_eq!(set_length_in_sec(0.01, 0.01), 0.01);
        assert_eq!(set_length_in_sec(1.0, 1e6), 100_000.0);
        assert_eq!(set_length_in_sec(0.0, -0.0).to_bits(), 0.0f32.to_bits());
        assert!(set_length_in_sec(1.0, f32::NAN).is_nan());
    }

    /// Native rows of `ParticleSystem::SetLengthInSec` and of the setter
    /// binding's tail (JP 6.8.1, executed in an emulator with the relocations
    /// and the static initializer run first): every stored duration word must
    /// equal the native one, the binding rows' byte must be raised exactly
    /// when the system is not stopped, and each named one-rule mutant must
    /// miss.
    #[test]
    #[ignore = "needs MOLY_DURATION_ROWS (native duration setter rows)"]
    fn set_length_in_sec_matches_native_rows() {
        let path = std::env::var_os("MOLY_DURATION_ROWS").expect("MOLY_DURATION_ROWS");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("sha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let rows = receipt.get("rows").and_then(Value::as_array).unwrap();
        type Rule = fn(f32, f32) -> f32;
        let mutants: [(&str, Rule); 4] = [
            ("noEqualShortCircuit", |_, v| if v < 0.05 { 0.05 } else { machine_fmin(v, 100_000.0) }),
            ("hostMinDropsNaN", |s, v| if s == v { s } else if v < 0.05 { 0.05 } else { v.min(100_000.0) }),
            ("noUpperClamp", |s, v| if s == v { s } else if v < 0.05 { 0.05 } else { v }),
            ("noLowerClamp", |s, v| if s == v { s } else { machine_fmin(v, 100_000.0) }),
        ];
        let mut red = [0usize; 4];
        let mut mismatched = 0;
        for (index, row) in rows.iter().enumerate() {
            let (stored, value, native) = (word(row, "old"), word(row, "value"), word(row, "native"));
            let (s, v) = (f32::from_bits(stored), f32::from_bits(value));
            let ours = set_length_in_sec(s, v).to_bits();
            if ours != native {
                mismatched += 1;
                println!("duration row {index}: stored {stored:08x} value {value:08x}: ours {ours:08x} native {native:08x}");
            }
            for (k, (_, rule)) in mutants.iter().enumerate() {
                if rule(s, v).to_bits() != native {
                    red[k] += 1;
                }
            }
        }
        let binding = receipt.get("binding").and_then(Value::as_array).unwrap();
        let mut binding_mismatched = 0;
        for row in binding {
            let (s, v) = (f32::from_bits(word(row, "old")), f32::from_bits(word(row, "value")));
            let raised = word(row, "stopped") == 0;
            if set_length_in_sec(s, v).to_bits() != word(row, "duration") || u32::from(raised) != word(row, "flag22") {
                binding_mismatched += 1;
            }
        }
        println!("duration rows {}, mismatched {mismatched}; binding rows {}, mismatched {binding_mismatched}; mutants red {}",
            rows.len(), binding.len(),
            mutants.iter().zip(red).map(|((name, _), n)| format!("{name} {n}")).collect::<Vec<_>>().join(", "));
        assert!(rows.len() > 2000 && binding.len() >= 12);
        assert_eq!((mismatched, binding_mismatched), (0, 0));
        assert!(red.iter().all(|&n| n > 0), "a mutant matched every row: {red:?}");
    }
}
