//! The inherited block against native RecordParticleDeath and KillParticle rows
//! whose edges set inherit bits, and the no-panic sweep.

use super::*;
use crate::particle::collision_event::EdgeBurst;
use crate::particle::death_event::{record_death, DeathEmitEdge, DeathParent, RecordedDeath};
use crate::particle::json::{parse, Value};
use crate::particle::sub_emission::EventOwner;
use std::collections::BTreeMap;

fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
    v.get(key).unwrap_or_else(|| panic!("row field {key}"))
}
fn items(v: &Value) -> &[Value] {
    v.as_array().expect("row array")
}
fn word(v: &Value) -> u32 {
    let x = v.as_f64().expect("row integer");
    assert!(x.fract() == 0.0 && (0.0..=4_294_967_295.0).contains(&x), "row word {x}");
    x as u32
}
fn bits(v: &Value) -> f32 {
    f32::from_bits(word(v))
}
fn words(v: &Value) -> Vec<u32> {
    items(v).iter().map(word).collect()
}
fn vec3(v: &Value) -> [f32; 3] {
    let w = words(v);
    std::array::from_fn(|k| f32::from_bits(w[k]))
}
fn hex(v: &Value) -> Vec<u8> {
    let text = v.as_str().expect("hex");
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

/// The parent SizeModule of a row, parsed from its authored block as the
/// export carries it.
fn size_module(row: &Value) -> Option<SizeOverLifetimeParams> {
    let spec = field(row, "sizeModule");
    if matches!(spec, Value::Null) {
        return None;
    }
    let authored = field(spec, "authored");
    let curves = items(field(authored, "curves"));
    let separate = field(authored, "separateAxes").as_bool().unwrap();
    // The export's block: `curve` is x, `y` and `z` only with separate axes.
    let mut block = vec![("separateAxes".to_owned(), Value::Bool(separate)), ("curve".to_owned(), curves[0].clone())];
    if curves.len() == 3 {
        block.push(("y".to_owned(), curves[1].clone()));
        block.push(("z".to_owned(), curves[2].clone()));
    }
    Some(SizeOverLifetimeParams::from_value(&Value::Object(block), "row").unwrap())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Arm {
    /// The size bit ignored: the neutral block.
    IgnoreBit,
    /// The parent sampled after its slot is overwritten: the pool's last
    /// particle, which the kill moves into the dead slot.
    AfterOverwrite,
    /// The SizeModule factor added to the size instead of multiplying it.
    AddFactor,
    /// The parent's SizeModule left out: the stored size alone.
    NoSizeModule,
    /// The factor at `age * 0.01` instead of the copy's `(start - remaining) / start`.
    PercentAge,
    /// The stored y and z read for a one-axis parent too (no replication of x).
    ReadsStoredYz,
}
const ARMS: [Arm; 6] = [Arm::IgnoreBit, Arm::AfterOverwrite, Arm::AddFactor, Arm::NoSizeModule, Arm::PercentAge,
    Arm::ReadsStoredYz];

fn first_burst(child: &Value) -> Option<EdgeBurst> {
    let burst = items(field(child, "bursts")).first()?;
    let probability = bits(field(burst, "probabilityBits"));
    let count = field(burst, "count");
    Some(match field(count, "mode").as_str() {
        Some("constant") => EdgeBurst::new(probability, bits(field(count, "bits"))).unwrap(),
        Some("twoConstants") => EdgeBurst::two_constants(probability, bits(field(count, "minBits")),
            bits(field(count, "maxBits"))).unwrap(),
        mode => panic!("burst count mode {mode:?} is not in these rows"),
    })
}

fn parent_of(row: &Value) -> DeathParent {
    let p = field(row, "parent");
    DeathParent {
        index: word(field(row, "index")) as usize,
        seed: word(field(p, "seed")),
        position: vec3(field(p, "position")),
        velocity: vec3(field(p, "velocity")),
        animated: vec3(field(p, "animated")),
        size: vec3(field(p, "size")),
        age_percent: bits(field(p, "ageBits")),
        inverse_lifetime: bits(field(p, "inverseBits")),
    }
}

/// The event of a row under an arm; `Err` with the refusal when the law
/// refuses the edge. `last` is the pool's last particle of the row's run.
fn ours(row: &Value, last: &DeathParent, arm: Option<Arm>) -> Result<RecordedDeath, Refused> {
    let edge = field(row, "edge");
    let p = field(row, "parent");
    let size_3d = word(field(p, "size3d")) != 0;
    let module = size_module(row);
    let inherit = InheritSize::from_parent(word(field(edge, "properties")), module.as_ref(), size_3d)?;
    let owner_words = words(field(p, "ownerBits"));
    let owner = EventOwner {
        local_to_world: std::array::from_fn(|i| f32::from_bits(owner_words[i])),
        world_space: word(field(p, "simulation")) == 1,
        accumulated_time: bits(field(p, "stateZeroBits")),
        emission_word: word(field(p, "ownerSeed")),
    };
    let mut dying = parent_of(row);
    if arm == Some(Arm::AfterOverwrite) {
        dying.size = last.size;
        dying.age_percent = last.age_percent;
        dying.inverse_lifetime = last.inverse_lifetime;
    }
    let law = DeathEmitEdge::new(bits(field(edge, "probabilityBits")), first_burst(field(row, "child"))).unwrap()
        .with_inherit(inherit.clone().filter(|_| arm != Some(Arm::IgnoreBit)));
    let mut recorded = record_death(&law, word(field(row, "slot")) as usize, &dying, &owner);
    if let (Some(arm @ (Arm::AddFactor | Arm::NoSizeModule | Arm::PercentAge | Arm::ReadsStoredYz)), Some(inherit)) =
        (arm, inherit.as_ref()) {
        let block = mutant_block(inherit, &dying, arm);
        if let Some(commands) = recorded.commands.as_mut() {
            for command in commands.iter_mut() {
                command.inherited = block;
            }
        }
    }
    Ok(recorded)
}

/// One rule of [`InheritSize::block`] changed.
fn mutant_block(inherit: &InheritSize, parent: &DeathParent, arm: Arm) -> [u32; 13] {
    let copy = InheritParent { size: parent.size, age_percent: parent.age_percent, inverse_lifetime: parent.inverse_lifetime };
    let three = inherit.size_3d || arm == Arm::ReadsStoredYz;
    let mut size = if three { copy.size } else { [copy.size[0]; 3] };
    let t = match arm {
        Arm::PercentAge => a::max_nm(a::mul(copy.age_percent, 0.01), 0.0),
        _ => copy_time(&copy),
    };
    let factor = match &inherit.factor {
        _ if arm == Arm::NoSizeModule => None,
        SizeFactor::Absent => None,
        SizeFactor::Constant(value) => Some(*value),
        SizeFactor::Curve(curve) => Some(curve.evaluate(t, 0.0)),
    };
    if let Some(factor) = factor {
        let factor = a::max_nm(factor, 0.0);
        size = size.map(|axis| if arm == Arm::AddFactor { a::add(axis, factor) } else { a::mul(axis, factor) });
    }
    if !three {
        size = [size[0]; 3];
    }
    let mut block = neutral_inherited(parent.seed);
    for axis in 0..3 {
        block[1 + axis] = size[axis].to_bits();
    }
    block
}

fn compare(row: &Value, recorded: &RecordedDeath) -> Vec<&'static str> {
    let mut bad = Vec::new();
    if words(field(row, "stateBefore")) != recorded.state_before {
        bad.push("stateBefore");
    }
    if words(field(row, "stateAfter")) != recorded.state_after {
        bad.push("stateAfter");
    }
    let native = items(field(row, "commands"));
    match &recorded.commands {
        None if native.is_empty() => {}
        Some(pair) if native.len() == 2 => {
            for (command, native) in pair.iter().zip(native) {
                let raw = hex(field(native, "rawHex"));
                let bytes = command.to_bytes();
                if raw[0x20..0x54] != bytes[0x20..0x54] {
                    bad.push("inheritedBlock");
                }
                if raw[0x08..0x20] != bytes[0x08..0x20] || raw[0x58..0x78] != bytes[0x58..0x78] {
                    bad.push("commandBytes");
                }
                if hex(field(native, "emissionHex")) != command.emission_bytes() {
                    bad.push("commandEmission");
                }
            }
        }
        _ => bad.push("commandCount"),
    }
    bad
}

/// Every native row is either compared word for word (the command bytes with
/// the inherited block, the event states) or refused by name, and a refused
/// row carries a bit or a parent size configuration the law does not
/// transcribe. Every one-rule arm differs from native on some admitted row.
#[test]
#[ignore = "needs MOLY_INHERIT_PARENT_ROWS"]
fn inherited_blocks_match_native_rows() {
    let path = std::env::var("MOLY_INHERIT_PARENT_ROWS").expect("MOLY_INHERIT_PARENT_ROWS");
    let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
    assert_eq!(field(&doc, "librarySha256").as_str(),
        Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
    let rows = items(field(&doc, "rows"));
    assert!(!rows.is_empty());
    // The pool's last particle of every run, for the overwrite arm.
    let mut last: BTreeMap<String, (usize, DeathParent)> = BTreeMap::new();
    for row in rows {
        let label = field(row, "label").as_str().unwrap().to_owned();
        let parent = parent_of(row);
        let entry = last.entry(label).or_insert((parent.index, parent));
        if parent.index >= entry.0 {
            *entry = (parent.index, parent);
        }
    }
    let (mut compared, mut non_neutral, mut bubble) = (0, 0, 0);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut red: BTreeMap<Arm, usize> = BTreeMap::new();
    for row in rows {
        let label = field(row, "label").as_str().unwrap();
        let tail = &last[label].1;
        let properties = word(field(field(row, "edge"), "properties"));
        let recorded = match ours(row, tail, None) {
            Ok(recorded) => recorded,
            Err(reason) => {
                *refused.entry(format!("{reason:?}")).or_default() += 1;
                // Only bits other than the size, or the parent size configurations named in the law, refuse.
                let module = size_module(row);
                let named = properties != INHERIT_SIZE || module.as_ref().is_some_and(|m| m.separate_axes
                    || matches!(m.curve, MinMaxCurve::TwoConstants { .. } | MinMaxCurve::TwoCurves { .. }));
                assert!(named, "{label}: refused {reason:?} although the row is inside the law");
                continue;
            }
        };
        compared += 1;
        bubble += usize::from(label.starts_with("bubble/") || label.starts_with("kill/"));
        let bad = compare(row, &recorded);
        if !bad.is_empty() && failures.len() < 12 {
            failures.push(format!("{label} slot {} index {}: {bad:?}", word(field(row, "slot")), word(field(row, "index"))));
        }
        if let Some(commands) = &recorded.commands {
            non_neutral += usize::from(commands[1].inherited[1..4] != [1.0_f32.to_bits(); 3]);
        }
        for arm in ARMS {
            if let Ok(other) = ours(row, tail, Some(arm)) {
                if !compare(row, &other).is_empty() {
                    *red.entry(arm).or_default() += 1;
                }
            }
        }
    }
    println!("inherit rows {}: compared {compared} (bubble and kill {bubble}), non-neutral size blocks {non_neutral}, \
        refused {refused:?}", rows.len());
    println!("arms red (rows): {red:?}");
    assert!(failures.is_empty(), "mismatches: {failures:#?}");
    assert!(compared > 0 && non_neutral > 0);
    for arm in ARMS {
        assert!(red.get(&arm).is_some_and(|n| *n > 0), "arm {arm:?} never differs from native");
    }
}

/// The properties gate names what it refuses; zero inherits nothing.
#[test]
fn properties_gate_names_every_untranscribed_bit() {
    assert!(InheritSize::from_parent(0, None, false).unwrap().is_none());
    assert!(InheritSize::from_parent(INHERIT_SIZE, None, false).unwrap().is_some());
    for (bit, name) in [(INHERIT_COLOR, "color"), (INHERIT_ROTATION, "rotation"), (INHERIT_LIFETIME, "lifetime"),
        (INHERIT_DURATION, "duration")] {
        assert_eq!(InheritSize::from_parent(bit | INHERIT_SIZE, None, false).unwrap_err(), Refused::Bit(name));
    }
    assert_eq!(InheritSize::from_parent(u32::MAX, None, false).unwrap_err(), Refused::UnknownBits);
    let two = SizeOverLifetimeParams { separate_axes: false, curve: MinMaxCurve::TwoConstants { min: 0.0, max: 1.0 },
        y: None, z: None };
    assert!(matches!(InheritSize::from_parent(INHERIT_SIZE, Some(&two), false), Err(Refused::ParentSize(_))));
    let words = neutral_inherited(9);
    assert!(ChildInherit::from_words(&words).unwrap().is_neutral());
    for (index, name) in [(0, "color"), (4, "rotation"), (8, "rotation"), (10, "lifetime"), (11, "duration")] {
        let mut w = words;
        w[index] ^= 1;
        assert_eq!(ChildInherit::from_words(&w).unwrap_err(), Refused::Block(name));
    }
}

/// Finite and non-finite parents never panic.
#[test]
fn inherited_block_never_panics() {
    let specials = [0.0, -0.0, 1.0, -1.0, 100.0, 1e-30, f32::MAX, f32::MIN_POSITIVE, f32::INFINITY,
        f32::NEG_INFINITY, f32::NAN, 250.0];
    let curve = MinMaxCurve::Curve { multiplier: 2.0, max: crate::particle::value::Curve { multiplier: 1.0, keys: vec![
        crate::particle::CurveKey { time: 0.0, value: 0.5, in_slope: 0.0, out_slope: 1.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
        crate::particle::CurveKey { time: 0.25, value: -1.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
        crate::particle::CurveKey { time: 0.5, value: 3.0, in_slope: 2.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
        crate::particle::CurveKey { time: 1.0, value: 1.0, in_slope: 0.0, out_slope: 0.0, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0 },
    ], pre_wrap: Some(2), post_wrap: Some(2) } };
    let modules = [None, Some(MinMaxCurve::Constant(-2.0)), Some(MinMaxCurve::Constant(f32::NAN)), Some(curve)];
    for module in &modules {
        let params = module.clone().map(|curve| SizeOverLifetimeParams { separate_axes: false, curve, y: None, z: None });
        for size_3d in [false, true] {
            let law = InheritSize::from_parent(INHERIT_SIZE, params.as_ref(), size_3d).unwrap().unwrap();
            for &age in &specials {
                for &inverse in &specials {
                    let block = law.block(&InheritParent { size: [age, inverse, 1.0], age_percent: age, inverse_lifetime: inverse }, 7);
                    let child = ChildInherit::from_words(&block).unwrap();
                    let _ = child.start_size([inverse, age, 0.5]);
                }
            }
        }
    }
}
