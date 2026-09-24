//! The death event against native RecordParticleDeath and KillParticle rows.

use super::*;
use crate::particle::json::{parse, Value};
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
    assert_eq!(w.len(), 3);
    std::array::from_fn(|k| f32::from_bits(w[k]))
}
fn hex(v: &Value) -> Vec<u8> {
    let text = v.as_str().expect("hex");
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

/// The first burst of the row's child as the event reads it, or the reason
/// the gate refuses it.
fn first_burst(child: &Value) -> Result<Option<EdgeBurst>, Refused> {
    let Some(burst) = items(field(child, "bursts")).first() else {
        return Ok(None);
    };
    let probability = bits(field(burst, "probabilityBits"));
    let count = field(burst, "count");
    Ok(Some(match field(count, "mode").as_str() {
        Some("constant") => EdgeBurst::new(probability, bits(field(count, "bits")))?,
        Some("twoConstants") => EdgeBurst::two_constants(probability, bits(field(count, "minBits")),
            bits(field(count, "maxBits")))?,
        mode => panic!("burst count mode {mode:?} is not in these rows"),
    }))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Arm {
    /// The catch-up word left at zero.
    CatchUpZero,
    /// The animated velocity left out.
    NoAnimated,
    /// The translation added before the three products.
    TranslationFirst,
    /// The edge probability gate skipped.
    NoEdgeGate,
    /// Two constants counting their smaller end.
    TwoConstantsLow,
    /// The burst's own probability draw skipped.
    NoBurstDraw,
}
const ARMS: [Arm; 6] = [Arm::CatchUpZero, Arm::NoAnimated, Arm::TranslationFirst, Arm::NoEdgeGate,
    Arm::TwoConstantsLow, Arm::NoBurstDraw];

/// Our event for the row under an arm; `None` when the gate refuses the row.
fn ours(row: &Value, arm: Option<Arm>) -> Option<(RecordedDeath, bool)> {
    let edge = field(row, "edge");
    if word(field(edge, "properties")) != 0 {
        return None;
    }
    let child = field(row, "child");
    let mut burst = first_burst(child).ok()?;
    let mut probability = bits(field(edge, "probabilityBits"));
    if arm == Some(Arm::NoEdgeGate) {
        probability = 1.0;
    }
    if let (Some(Arm::TwoConstantsLow | Arm::NoBurstDraw), Some(first)) = (arm, items(field(child, "bursts")).first()) {
        let count = field(first, "count");
        let p = if arm == Some(Arm::NoBurstDraw) { 1.0 } else { bits(field(first, "probabilityBits")) };
        burst = Some(match (field(count, "mode").as_str(), arm) {
            (Some("twoConstants"), Some(Arm::TwoConstantsLow)) => {
                let (lo, hi) = (bits(field(count, "minBits")), bits(field(count, "maxBits")));
                EdgeBurst::new(p, if hi < lo { hi } else { lo }).unwrap()
            }
            (Some("twoConstants"), _) => EdgeBurst::two_constants(p, bits(field(count, "minBits")),
                bits(field(count, "maxBits"))).unwrap(),
            _ => EdgeBurst::new(p, bits(field(count, "bits"))).unwrap(),
        });
    }
    let edge = DeathEmitEdge::new(probability, burst).ok()?;
    let parent = field(row, "parent");
    let simulation = word(field(parent, "simulation"));
    let owner_words = words(field(parent, "ownerBits"));
    let mut owner = EventOwner {
        local_to_world: std::array::from_fn(|i| f32::from_bits(owner_words[i])),
        world_space: simulation == 1,
        accumulated_time: bits(field(parent, "stateZeroBits")),
        emission_word: word(field(parent, "ownerSeed")),
    };
    if arm == Some(Arm::CatchUpZero) {
        owner.accumulated_time = 0.0;
    }
    let mut dying = DeathParent {
        index: word(field(row, "index")) as usize,
        seed: word(field(parent, "seed")),
        position: vec3(field(parent, "position")),
        velocity: vec3(field(parent, "velocity")),
        animated: vec3(field(parent, "animated")),
    };
    if arm == Some(Arm::NoAnimated) {
        dying.animated = [0.0; 3];
    }
    let mut recorded = record_death(&edge, word(field(row, "slot")) as usize, &dying, &owner);
    if arm == Some(Arm::TranslationFirst) && !owner.world_space {
        let m = owner.local_to_world;
        let position: [f32; 3] = std::array::from_fn(|k|
            ((m[12 + k] + m[k] * dying.position[0]) + m[4 + k] * dying.position[1]) + m[8 + k] * dying.position[2]);
        if let Some(commands) = recorded.commands.as_mut() {
            for command in commands.iter_mut() {
                command.position = position;
            }
        }
    }
    Some((recorded, simulation == 1))
}

/// The fields of the row that differ from ours.
fn compare(row: &Value, recorded: &RecordedDeath) -> Vec<&'static str> {
    let mut bad = Vec::new();
    if word(field(row, "trigger")) != 2 {
        bad.push("trigger");
    }
    if words(field(row, "timesBits")) != DEATH_TIMES.map(f32::to_bits) {
        bad.push("times");
    }
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
                let ours = command.to_bytes();
                // The pointer word and the padding word after the inherited
                // block are not fields.
                if raw.len() != 0x78 || raw[0x08..0x54] != ours[0x08..0x54] || raw[0x58..0x78] != ours[0x58..0x78] {
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

/// Every native death record, from RecordParticleDeath over every index and
/// KillParticle with recording on, is reproduced word for word: the state
/// before and after, whether commands are issued, and every command byte.
/// Records whose edge inherits properties are refused by the gate and
/// counted. Every one-rule arm differs from native on at least one row.
#[test]
#[ignore = "needs MOLY_DEATH_EVENT_ROWS"]
fn death_records_match_native_rows() {
    let path = std::env::var("MOLY_DEATH_EVENT_ROWS").expect("MOLY_DEATH_EVENT_ROWS");
    let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
    assert_eq!(field(&doc, "librarySha256").as_str(),
        Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
    let rows = items(field(&doc, "rows"));
    assert!(!rows.is_empty());
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let (mut compared, mut refused, mut with_commands, mut world, mut two_constants) = (0, 0, 0, 0, 0);
    let mut failures = Vec::new();
    let mut red: BTreeMap<Arm, usize> = BTreeMap::new();
    for row in rows {
        *kinds.entry(field(row, "kind").as_str().unwrap().to_owned()).or_default() += 1;
        let Some((recorded, in_world)) = ours(row, None) else {
            refused += 1;
            continue;
        };
        compared += 1;
        with_commands += usize::from(recorded.commands.is_some());
        world += usize::from(in_world);
        two_constants += usize::from(items(field(field(row, "child"), "bursts")).first()
            .is_some_and(|b| field(field(b, "count"), "mode").as_str() == Some("twoConstants")));
        let bad = compare(row, &recorded);
        if !bad.is_empty() && failures.len() < 12 {
            failures.push(format!("{} slot {} index {}: {bad:?}", field(row, "label").as_str().unwrap(),
                word(field(row, "slot")), word(field(row, "index"))));
        }
        for arm in ARMS {
            if let Some((other, _)) = ours(row, Some(arm)) {
                if !compare(row, &other).is_empty() {
                    *red.entry(arm).or_default() += 1;
                }
            }
        }
    }
    println!("death rows {} {kinds:?}: compared {compared}, refused (inherited properties) {refused}, \
        with commands {with_commands}, World {world}, two-constant counts {two_constants}",
        rows.len());
    println!("arms red (rows): {red:?}");
    assert!(failures.is_empty(), "mismatches: {failures:#?}");
    for arm in ARMS {
        assert!(red.get(&arm).is_some_and(|n| *n > 0), "arm {arm:?} never differs from native");
    }
}

/// Finite and non-finite inputs never panic; the gate refuses what it does
/// not transcribe instead.
#[test]
fn record_death_never_panics() {
    let specials = [0.0, -0.0, 1.0, -1.0, f32::MAX, f32::MIN, f32::MIN_POSITIVE, f32::INFINITY,
        f32::NEG_INFINITY, f32::NAN, 16_777_215.0, 16_777_216.0, 2.5e9, -3.0e9];
    for &p in &specials {
        for &lo in &specials {
            for &hi in &specials {
                let _ = EdgeBurst::two_constants(p, lo, hi);
                let _ = EdgeBurst::new(p, lo);
            }
        }
    }
    let bursts = [None, EdgeBurst::new(0.5, 3.0).ok(), EdgeBurst::two_constants(1.0, 0.0, 16_777_215.0).ok(),
        EdgeBurst::two_constants(0.25, 7.0, 7.0).ok(), EdgeBurst::new(1.0, 1.0e9).ok()];
    let mut seed = 0x1234_5678_u32;
    for &burst in &bursts {
        for &probability in &[0.0, 0.5, 1.0] {
            let edge = DeathEmitEdge::new(probability, burst).unwrap();
            for &v in &specials {
                seed = seed.wrapping_mul(0x6c07_8965).wrapping_add(1);
                let owner = EventOwner { local_to_world: [v; 16], world_space: seed & 1 == 0, accumulated_time: v,
                    emission_word: seed.rotate_left(7) };
                let parent = DeathParent { index: usize::MAX, seed, position: [v; 3], velocity: [-v; 3], animated: [v; 3] };
                let _ = record_death(&edge, usize::MAX, &parent, &owner);
            }
        }
    }
    assert!(DeathEmitEdge::new(f32::NAN, None).is_err());
}
