//! A sub-emitter target that is itself a parent, through the product's child
//! Emit, against the engine's own rows: the ground-contact target of the
//! meteor chain receives the collision commands of its parent (the engine's
//! RecordEmit output, as bytes), and inside each of its child Emits its own
//! sub-emitter call records the newborns' birth events for its two children.
//! The harness ran the engine's Emit, StartModules, SubModule and RecordEmit
//! bodies with explicit cached edge tables in both slot orders, the probe
//! Initial words, the identity owner and fixed state words; those are inputs,
//! not claims about scene placement or client entropy.
//!
//! Compared per command: the target's pool (count, position with X
//! reflected and zero compared by value, velocity, age, inverse lifetime,
//! seed), its Initial stream after, and every command its own events issued
//! (the child it names, the command bytes except the emission pointer and the
//! padding word, and the twelve-byte emission payload), in order. Two named
//! changes must turn rows red: the birth dt of every lane taken as the
//! command dt, and the two slots in the other order.
use super::child::{apply_command_with_events, arms, ChildCommand, ChildUpdate, TargetEvents, SOURCE_GRAVITY};
use super::sub_events::{BirthEdge, BirthEvents};
use super::*;
use moly_law::particle::child_emit::ChildOwner;
use moly_law::particle::schema::Effects;
use moly_law::particle::seed_owner::ModuleRandom;
use moly_law::particle::sub_emission::BirthEdgeLaw;
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

fn read(key: &str) -> Value {
    let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} is not set"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn hex(value: &Value) -> Vec<u8> {
    let text = value.as_str().expect("hex");
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}
fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("native array").iter().map(|v| v.as_u64().expect("word") as u32).collect()
}
fn floats(value: &Value) -> Vec<f32> {
    value.as_array().expect("native array").iter().map(|v| v.as_f64().expect("float") as f32).collect()
}

fn emitter(node: &str, system: &Value) -> EmitterParams {
    let selected = json!({"effects": {"target": {"particles": [{"node": node, "system": system}]}}});
    let mut decoded = Effects::from_json_str(&serde_json::to_vec(&selected).unwrap()).expect("decode system");
    assert_eq!(decoded.emitters.len(), 1);
    decoded.emitters.remove(0)
}

/// The identity owner the harness installed (owner, inverse, node rotation,
/// scales).
fn identity_owner() -> ChildOwner {
    let m: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
    ChildOwner {
        local_to_world: m,
        world_to_local: m,
        local_rotation: std::array::from_fn(|i| if i % 4 == 0 { 1.0 } else { 0.0 }),
        emitter_scale: [1.0; 3],
        shape_scale: [1.0; 3],
    }
}

#[derive(Default, Debug)]
struct Tally {
    commands_in: usize,
    births: usize,
    commands_out: usize,
    mismatched: Vec<String>,
}

/// Every order's commands through one target; `reverse` swaps the slots.
fn replay(recursive: &Value, upstream: &[Value], corpus: &Value, reverse: bool) -> Tally {
    let mut tally = Tally::default();
    let effect = &corpus["effects"]["fx_env_site_009_common_ground"]["particles"];
    let block = |node: &str, path_id: &Value| {
        let found: Vec<&Value> = effect.as_array().unwrap().iter().filter(|p| p["node"] == node).collect();
        assert_eq!(found.len(), 1, "one exported block for {node}");
        assert_eq!(&found[0]["systemPathId"], path_id, "{node} is the receipt's system");
        found[0]["system"].clone()
    };
    let target = &recursive["sourceTargets"][0];
    let ground_node = target["node"].as_str().unwrap();
    let ground = emitter(ground_node, &block(ground_node, &target["systemPathId"]));
    for output in recursive["outputs"].as_array().unwrap() {
        let mut order: Vec<usize> = output["explicitCachedOrder"].as_array().unwrap().iter()
            .map(|v| v.as_u64().unwrap() as usize).collect();
        if reverse {
            order.reverse();
        }
        let edges: Vec<BirthEdge> = order.iter().map(|&ordinal| {
            let edge = &ground.sub_emitters[ordinal];
            let child_node = edge.emitter.clone().unwrap();
            let source = recursive["sourceTargets"].as_array().unwrap().iter()
                .find(|t| t["node"] == child_node.as_str()).expect("child in the receipt");
            let child = emitter(&child_node, &block(&child_node, &source["systemPathId"]));
            let law = BirthEdgeLaw::from_params(edge, ground.sub_emitters.len(), &child.start_delay, child.duration,
                child.looping, child.emission.as_ref().unwrap()).expect("child emission inside the event law");
            BirthEdge { target: child_node, law }
        }).collect();
        let mut events = BirthEvents::new(edges.clone());
        for edge in &edges {
            events.deliver_to(&edge.target);
        }
        let mut system = test_support::runtime();
        system.node = ground_node.to_owned();
        system.kind = EffectKind::Site;
        system.custom_law = ground.custom_data.as_ref().map(moly_law::particle::custom_data::CustomData::from_params);
        system.emitter = ground.clone();
        system.pool.clear();
        system.side.clear();
        let rows = output["rows"].as_array().unwrap();
        let first = words(&rows[0]["ground"]["beforeWords"]);
        let mut initial = ModuleRandom { words: std::array::from_fn(|w| std::array::from_fn(|l| first[w * 4 + l])) };
        let owner = identity_owner();
        for row in rows {
            let index = row["upstreamCommand"].as_u64().unwrap() as usize;
            let native_command = &upstream[index];
            let command = ChildCommand::from_native_bytes(&hex(&native_command["rawHex"]), &hex(&native_command["emissionHex"]))
                .expect("native command bytes");
            tally.commands_in += 1;
            let update = ChildUpdate { flags: 0, frame_dt: 0.125, world_playing: true, gravity: SOURCE_GRAVITY };
            let label = format!("order {:?} command {index}", order);
            let result = apply_command_with_events(&mut system, &owner, &mut initial, None, &command, update,
                Some(TargetEvents { events: &mut events, accumulated: 0.0, emission_word: 1729 }));
            let applied = match result {
                Ok(applied) => applied,
                Err(refused) => { tally.mismatched.push(format!("{label}: refused {refused:?}")); continue; }
            };
            tally.births += applied.born;
            let out = &row["ground"]["output"];
            let mut differs = Vec::new();
            let count = out["count"].as_u64().unwrap() as usize;
            if system.pool.len() != count {
                differs.push(format!("count {} vs {count}", system.pool.len()));
            } else {
                for i in 0..count {
                    let p = floats(&out["position"][i]);
                    let v = floats(&out["velocity"][i]);
                    let particle = &system.pool[i];
                    let x = |ours: f32, native: f32| if native == 0.0 { ours == 0.0 } else { ours.to_bits() == (-native).to_bits() };
                    if !x(particle.position[0], p[0]) || particle.position[1].to_bits() != p[1].to_bits()
                        || particle.position[2].to_bits() != p[2].to_bits() {
                        differs.push(format!("position {i}"));
                    }
                    if !x(particle.velocity[0], v[0]) || particle.velocity[1] != v[1] || particle.velocity[2] != v[2] {
                        differs.push(format!("velocity {i}"));
                    }
                    if particle.age_percent.to_bits() != (out["age"][i].as_f64().unwrap() as f32).to_bits() {
                        differs.push(format!("age {i}"));
                    }
                    if particle.inverse_lifetime.to_bits() != (out["inverseLifetime"][i].as_f64().unwrap() as f32).to_bits() {
                        differs.push(format!("inverse lifetime {i}"));
                    }
                    if u64::from(system.side[i].seed) != out["seeds"][i].as_u64().unwrap() {
                        differs.push(format!("seed {i}"));
                    }
                }
            }
            let after: Vec<u32> = initial.words.iter().flatten().copied().collect();
            if after != words(&row["ground"]["afterWords"]) {
                differs.push("Initial stream".into());
            }
            let ours = events.take_commands();
            let theirs = row["commands"].as_array().unwrap();
            tally.commands_out += theirs.len();
            if ours.len() != theirs.len() {
                differs.push(format!("{} commands vs {}", ours.len(), theirs.len()));
            } else {
                for (k, ((target, command), native)) in ours.iter().zip(theirs).enumerate() {
                    let raw = command.to_bytes();
                    let native_raw = hex(&native["rawHex"]);
                    let same = |range: std::ops::Range<usize>| raw[range.clone()] == native_raw[range];
                    if target.as_str() != native["targetNode"].as_str().unwrap() || !same(8..0x54) || !same(0x58..0x78) {
                        differs.push(format!("command {k}"));
                    }
                    let d = command.emission;
                    let payload: Vec<u8> = [d.spacing, d.offset, d.burst_fraction].iter()
                        .flat_map(|v| v.to_le_bytes()).collect();
                    if payload != hex(&native["emissionHex"]) {
                        differs.push(format!("command {k} emission"));
                    }
                }
            }
            if !differs.is_empty() {
                tally.mismatched.push(format!("{label}: {differs:?}"));
            }
        }
    }
    tally
}

#[test]
#[ignore = "needs MOLY_RECURSIVE_EMIT_RECEIPT, MOLY_COLLISION_CHILD_RECEIPT and MOLY_RECURSIVE_EMIT_EFFECTS"]
fn target_with_own_birth_edges_matches_native_recursive_emit() {
    let recursive = read("MOLY_RECURSIVE_EMIT_RECEIPT");
    let collision = read("MOLY_COLLISION_CHILD_RECEIPT");
    let corpus = read("MOLY_RECURSIVE_EMIT_EFFECTS");
    for receipt in [&recursive, &collision] {
        assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
        assert_eq!(receipt["failureCount"], 0);
    }
    let upstream = collision["rows"].as_array().unwrap().iter()
        .find(|row| row["input"]["name"] == "reverse-two").expect("upstream collision row")["commands"]
        .as_array().unwrap().clone();
    arms::set(None);
    let tally = replay(&recursive, &upstream, &corpus, false);
    println!("recursive emit replay: {} commands in, {} births, {} own commands compared, mismatched {}",
        tally.commands_in, tally.births, tally.commands_out, tally.mismatched.len());
    for line in tally.mismatched.iter().take(20) {
        println!("  {line}");
    }
    assert!(tally.commands_in > 0 && tally.births > 0 && tally.commands_out > 0);
    assert!(tally.mismatched.is_empty());
    arms::set(Some("birthDtIsCommandDt"));
    let broken = replay(&recursive, &upstream, &corpus, false);
    arms::set(None);
    let reversed = replay(&recursive, &upstream, &corpus, true);
    println!("recursive emit replay arms: birthDtIsCommandDt {} rows red, reversedSlots {} rows red",
        broken.mismatched.len(), reversed.mismatched.len());
    assert!(!broken.mismatched.is_empty() && !reversed.mismatched.is_empty());
}
