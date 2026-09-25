//! The first-Play warm of a sub-emitter parent through the product against
//! native runs of the engine's Play on the current library:
//!
//! - the warm length of every warm parent of the exported corpus (a looping
//!   prewarm system whose SubModule is on with a real edge), its sub-emitter
//!   term read from the authored graph by the admission helper, against the
//!   native Compute of each: the out and starting clock bits;
//! - the ground-strike rain parent's warm with its CollisionModule removed,
//!   a named override the native run applied the same way (that run also
//!   carried a synthetic collision edge, which never fires without the module
//!   and is left out here): the Compute, every slice, every birth and death
//!   record in the order made with its pending time and its command bytes,
//!   the records and commands of each slice, the kills, then the pool, the
//!   pending time, the clock and the emission state after the warm;
//! - the parent's death- and collision-edge targets the product admits: each
//!   target's own update of the warm's length, the warm's native commands
//!   handed to it in the order recorded, the pool after them, then its
//!   ordinary frames with their commands (pool count, seeds, ages, inverse
//!   lifetimes and heights). A target the product's child composition
//!   refuses is counted by its refusal.
//!
//! As in the other parent replays, the probe RNG words and the owner matrix
//! replace the installed streams and the scene placement; they are inputs,
//! not claims about client entropy. The parent's emitter velocity mode goes
//! through the admission's own rewrite with the effect's component census.
//! Zeros in positions and velocities compare by value (the X-axis reflection
//! can flip a zero's sign); everything else by bits.
use super::*;
use super::sub_event_samples::{event_state, f, hex, parent_runtime_with, read, same_value, word, words};
use moly_law::particle::child_emit::ChildOwner;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use moly_law::particle::schema::Effects;
use moly_law::particle::sub_emission::{BirthDistribution, SubEmitterCommand};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

#[derive(Default)]
struct Tally {
    fields: BTreeMap<&'static str, (usize, usize)>,
    first: Vec<String>,
    counts: BTreeMap<&'static str, usize>,
    refused: Vec<String>,
}

impl Tally {
    fn check(&mut self, field: &'static str, ok: bool, label: &str) {
        let entry = self.fields.entry(field).or_default();
        entry.0 += 1;
        if !ok {
            entry.1 += 1;
            if self.first.len() < 16 {
                self.first.push(format!("{label}: {field}"));
            }
        }
    }

    fn count(&mut self, key: &'static str, n: usize) {
        *self.counts.entry(key).or_default() += n;
    }

    fn mismatched(&self) -> usize {
        self.fields.values().map(|(_, bad)| bad).sum()
    }

    fn print(&self, title: &str) {
        println!("{title}: {} mismatched fields; counts {:?}; refused {:?}", self.mismatched(), self.counts, self.refused);
        for (field, (checked, bad)) in &self.fields {
            println!("  {field}: {checked} checked, {bad} mismatched");
        }
    }
}

fn corpus_root() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(std::env::var_os("MOLY_WARM_CORPUS").expect("MOLY_WARM_CORPUS is not set"));
    assert!(root.is_dir(), "corpus root {root:?} is not a directory");
    root
}

fn corpus_file(relative: &str) -> Value {
    let path = corpus_root().join(relative);
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"))).unwrap()
}

fn floats<const N: usize>(value: &Value) -> [f32; N] {
    let w = words(value);
    assert_eq!(w.len(), N, "native word count");
    std::array::from_fn(|i| f32::from_bits(w[i]))
}

fn module_random(value: &Value) -> ModuleRandom {
    let w = words(value);
    assert_eq!(w.len(), 16);
    ModuleRandom { words: std::array::from_fn(|i| std::array::from_fn(|lane| w[i * 4 + lane])) }
}

fn decode(record: &Value) -> Result<EmitterParams, String> {
    let wrapped = json!({"effects": {"selected": {"particles": [record]}}});
    let decoded = Effects::from_json_str(&serde_json::to_vec(&wrapped).unwrap()).map_err(|e| e.to_string())?;
    decoded.emitters.into_iter().next().ok_or_else(|| "no emitter decoded".to_owned())
}

fn sub_module_enabled(record: &Value) -> bool {
    record.pointer("/system/sourceModules/enabled").and_then(Value::as_array)
        .is_some_and(|modules| modules.iter().any(|m| m.as_str() == Some("SubModule")))
}

/// The native command bytes and emission scalars as the law's command.
fn native_command(row: &Value) -> SubEmitterCommand {
    let raw = hex(&row["rawHex"]);
    let emission = hex(&row["emissionHex"]);
    assert_eq!((raw.len(), emission.len()), (0x78, 12));
    let w = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
    let fl = |o: usize| f32::from_bits(w(o));
    let long = |o: usize| u64::from(w(o)) | (u64::from(w(o + 4)) << 32);
    let pay = |o: usize| f32::from_le_bytes(emission[o..o + 4].try_into().unwrap());
    SubEmitterCommand {
        position: std::array::from_fn(|a| fl(0x08 + a * 4)),
        velocity: std::array::from_fn(|a| fl(0x14 + a * 4)),
        inherited: std::array::from_fn(|i| w(0x20 + i * 4)),
        count: long(0x58),
        rate_count: long(0x60),
        dt: fl(0x68),
        previous_normalized: fl(0x6c),
        current_normalized: fl(0x70),
        catch_up: fl(0x74),
        emission: BirthDistribution { spacing: pay(0), offset: pay(4), burst_fraction: pay(8) },
    }
}

fn compare_command(tally: &mut Tally, ours: &SubEmitterCommand, native: &Value, at: &str,
    fields: &'static [&'static str; 4]) {
    let raw = hex(&native["rawHex"]);
    let bytes = ours.to_bytes();
    // The pointer word and the padding word after the inherited block are
    // not fields.
    let vectors = raw.len() == 0x78 && (0x08..0x20).step_by(4).all(|o| same_value(
        f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()),
        u32::from_le_bytes(raw[o..o + 4].try_into().unwrap())));
    tally.check(fields[0], vectors, at);
    tally.check(fields[1], raw.get(0x20..0x54) == Some(&bytes[0x20..0x54]), at);
    tally.check(fields[2], raw.get(0x58..0x78) == Some(&bytes[0x58..0x78]), at);
    tally.check(fields[3], hex(&native["emissionHex"]) == ours.emission_bytes(), at);
}

const BIRTH_COMMAND: [&str; 4] = ["birthCommandPositionVelocity", "birthCommandInherited", "birthCommandCountTimes",
    "birthCommandEmission"];
const DEATH_COMMAND: [&str; 4] = ["deathCommandPositionVelocity", "deathCommandInherited", "deathCommandCountTimes",
    "deathCommandEmission"];

// ---------------------------------------------------------------- warm lengths

/// The product's first-Play plan of an admitted system with the admission's
/// sub-emitter term (or none).
fn product_plan(emitter: &EmitterParams, sub: f32) -> Result<moly_law::particle::prewarm::PrewarmPlan, &'static str> {
    let mut system = test_support::runtime();
    system.emitter = emitter.clone();
    system.pool.clear();
    system.side.clear();
    system.sub_emitter_max_lifetime = sub;
    first_play_plan(&system)
}

#[test]
#[ignore = "MOLY_WARM_LENGTH_CENSUS and MOLY_WARM_CORPUS must identify the native warm-length census and the corpus it covers"]
fn product_warm_lengths_match_native_compute() {
    let census = read("MOLY_WARM_LENGTH_CENSUS");
    let (_, corpus) = census["corpora"].as_object().unwrap().iter()
        .find(|(name, _)| name.contains("overlay2")).expect("the census of this corpus");
    let rows = corpus["parents"].as_array().unwrap();
    let root = corpus_root();
    let mut tally = Tally::default();
    // The product's own selection of the warm parents over every exported
    // system of the corpus.
    let mut docs: HashMap<String, Value> = HashMap::new();
    let mut phenomena: Vec<_> = std::fs::read_dir(&root).unwrap().filter_map(Result::ok)
        .filter(|entry| entry.path().join("fx/effects.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    phenomena.sort();
    let mut ours = BTreeSet::new();
    for phenomenon in &phenomena {
        let doc = corpus_file(&format!("{phenomenon}/fx/effects.json"));
        for (effect, body) in doc["effects"].as_object().unwrap() {
            for record in body["particles"].as_array().unwrap() {
                tally.count("systems", 1);
                let emitter = match decode(record) {
                    Ok(emitter) => emitter,
                    Err(_) => {
                        tally.count("undecoded", 1);
                        continue;
                    }
                };
                if emitter.prewarm && emitter.looping && sub_module_enabled(record) && has_real_sub_emitter_edges(&emitter) {
                    ours.insert((phenomenon.clone(), effect.clone(), record["node"].as_str().unwrap().to_owned()));
                }
            }
        }
        docs.insert(phenomenon.clone(), doc);
    }
    let native: BTreeSet<_> = rows.iter().map(|row| (row["phenomenon"].as_str().unwrap().to_owned(),
        row["effect"].as_str().unwrap().to_owned(), row["node"].as_str().unwrap().to_owned())).collect();
    tally.check("warmParentSet", ours == native, "corpus");
    tally.count("warmParents", ours.len());
    let mut without_term_wrong = 0;
    for row in rows {
        let phenomenon = row["phenomenon"].as_str().unwrap();
        let effect = row["effect"].as_str().unwrap();
        let node = row["node"].as_str().unwrap();
        let at = format!("{phenomenon} {effect} {node}");
        let particles = docs[phenomenon]["effects"][effect]["particles"].as_array().unwrap();
        let record = particles.iter().find(|p| p["node"] == node).expect("warm parent record");
        let emitter = decode(record).expect("warm parent decodes");
        let graph = crate::weather_fx::source_sub_emitter_owners(particles);
        let warm = &row["warm"];
        let sub = match crate::weather_fx::warm_child_term(record, &emitter, &graph) {
            Ok(sub) => sub,
            Err(reason) => {
                tally.refused.push(format!("{at}: {reason}"));
                tally.check("childTermRead", false, &at);
                continue;
            }
        };
        let plan = product_plan(&emitter, sub);
        tally.check("computeValid", plan.is_ok() == (warm["valid"].as_u64() == Some(1)), &at);
        if let Ok(plan) = plan {
            tally.check("computeOut", plan.compute_out().to_bits() == (warm["out"].as_f64().unwrap() as f32).to_bits(), &at);
            tally.check("computeClock", plan.initial_clock().to_bits() == (warm["clock"].as_f64().unwrap() as f32).to_bits(), &at);
        }
        // The same system without the term gives the census's length without
        // it, and so differs from the native out.
        let bare = product_plan(&emitter, 0.0).map(|plan| plan.compute_out());
        tally.check("computeOutWithoutTerm", bare.is_ok_and(|out|
            out.to_bits() == (warm["outWithoutChildTerm"].as_f64().unwrap() as f32).to_bits()), &at);
        without_term_wrong += usize::from(bare.map_or(true, |out| out.to_bits() != (warm["out"].as_f64().unwrap() as f32).to_bits()));
    }
    tally.print("product warm lengths");
    println!("arm warmWithoutChildTerm: {without_term_wrong} of {} warm lengths differ from native", rows.len());
    assert_eq!(rows.len(), 25);
    assert!(tally.first.is_empty(), "first mismatches: {:#?}", tally.first);
    assert!(without_term_wrong > 0);
}

// ---------------------------------------------------------------- the parent's warm

fn without_collision(record: &mut Value) {
    let system = record.get_mut("system").unwrap();
    system.as_object_mut().unwrap().remove("collision").expect("the parent carries a CollisionModule");
    system["sourceModules"]["enabled"].as_array_mut().unwrap().retain(|m| m != "CollisionModule");
}

/// The parent as the admission installs it: the exported record under the
/// run's named override, the emitter velocity mode through the census
/// rewrite (or kept), the warm length's sub-emitter term from the graph.
fn warm_parent(doc: &Value, index: &Value, run: &Value, keep_mode: bool) -> Result<(Runtime, EventEdges), String> {
    let source = &run["source"];
    let effect = source["effect"].as_str().unwrap();
    let collision = run["collision"].as_bool().unwrap();
    let bodies = crate::weather_fx::census_names_no_body(effect, &doc["effects"][effect],
        index.pointer("/summary/unsupported"));
    let (mut system, edges) = parent_runtime_with(doc, source, |record| {
        if !collision {
            without_collision(record);
        }
    }, |emitter| {
        if !keep_mode {
            crate::weather_fx::resolve_velocity_mode(emitter, &bodies);
        }
    })?;
    let mut particles: Vec<Value> = doc["effects"][effect]["particles"].as_array().unwrap().clone();
    let at = particles.iter().position(|p| p["node"] == source["node"]).unwrap();
    if !collision {
        without_collision(&mut particles[at]);
    }
    let graph = crate::weather_fx::source_sub_emitter_owners(&particles);
    system.sub_emitter_max_lifetime = crate::weather_fx::warm_child_term(&particles[at], &system.emitter, &graph)?;
    Ok((system, edges))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum WarmArm {
    Product,
    /// The warm length without the sub-emitter term.
    NoChildTerm,
    /// One of the product's one-rule arms on the pending time the slices
    /// record.
    Arm(&'static str),
}

fn replay_warm(doc: &Value, index: &Value, run: &Value, arm: WarmArm, tally: &mut Tally) {
    let label = run["label"].as_str().unwrap();
    let image = &run["image"];
    let warm = &run["warm"];
    let (mut system, edges) = warm_parent(doc, index, run, false).expect("the product admits the parent");
    if arm == WarmArm::NoChildTerm {
        system.sub_emitter_max_lifetime = 0.0;
    }
    assert_eq!(word(&image["maximum"]), system.emitter.max_particles, "{label}");
    assert_eq!(f(&image["duration"]), system.emitter.duration, "{label}");
    assert_eq!(image["looping"].as_bool(), Some(system.emitter.looping), "{label}");
    assert_eq!(word(&image["simulation"]), u32::from(system.emitter.simulation_space == SimulationSpace::World));
    // The native table order: birth edges, then death edges, each authored;
    // the synthetic collision edge is not installed.
    let image_edges = image["subEmitters"]["edges"].as_array().unwrap();
    let nodes = |trigger: u64| image_edges.iter().filter(|e| e["trigger"].as_u64() == Some(trigger))
        .map(|e| e["node"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(nodes(0), edges.births.iter().map(|e| e.target.clone()).collect::<Vec<_>>(), "{label}: birth slots");
    assert_eq!(nodes(2), edges.deaths.iter().map(|e| e.target.clone()).collect::<Vec<_>>(), "{label}: death slots");
    assert!(edges.collisions.is_empty());
    // Runtime X is the reflection of source X.
    let owner_words = words(&image["owner"]);
    let source_owner: [f32; 16] = std::array::from_fn(|i| f32::from_bits(owner_words[i]));
    let runtime_owner: [f32; 16] = std::array::from_fn(|i|
        if (i % 4 == 0) ^ (i / 4 == 0) { -source_owner[i] } else { source_owner[i] });
    system.node_affine = GlobalTransform::from(Mat4::from_cols_array(&runtime_owner));
    let ctx = Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(matches!(install_native_birth(&mut system, &mut manager, &SourceRoute::Ordinary, None).unwrap(), BirthPath::Native));
    {
        let state = system.native_birth.as_mut().unwrap();
        state.owner = None;
        state.initial = module_random(&image["initialWords"]);
        state.shape = module_random(&image["shapeWords"]);
        let emission = words(&image["emissionWords"]);
        state.emission.random = ScalarRandom { words: std::array::from_fn(|i| emission[i]) };
        let mut events = BirthEvents::with_edges(edges.clone());
        events.trace = Some(Vec::new());
        events.death_trace = Some(Vec::new());
        state.events = Some(events);
    }
    // ---- Compute and the slice plan
    let compute = &run["compute"];
    let plan = first_play_plan(&system);
    tally.check("computeOut", plan.as_ref().is_ok_and(|p| p.compute_out().to_bits() == word(&compute["outBits"])), label);
    tally.check("computeClock", plan.as_ref().is_ok_and(|p| p.initial_clock().to_bits() == word(&compute["clockBits"])), label);
    let Ok(plan) = plan else { return };
    let slices: Result<Vec<_>, _> = plan.collect();
    let native_slices = warm["slices"].as_array().unwrap();
    let slices = slices.expect("the warm's slice plan");
    tally.check("sliceCount", slices.len() == native_slices.len(), label);
    for (k, (slice, native)) in slices.iter().zip(native_slices).enumerate() {
        tally.check("slice", slice.remaining_before.to_bits() == word(&native["remainingBits"])
            && slice.duration.to_bits() == word(&native["stepBits"]), &format!("{label} slice {k}"));
    }
    tally.count("slices", slices.len());
    // ---- the warm through the product
    let mut state = system.native_birth.take().unwrap();
    child::arms::set(match arm { WarmArm::Arm(name) => Some(name), _ => None });
    let result = prewarm_native(&mut system, &mut state, &ctx);
    child::arms::set(None);
    let events = state.events.as_mut().unwrap();
    let trace = std::mem::take(events.trace.as_mut().unwrap());
    let death_trace = std::mem::take(events.death_trace.as_mut().unwrap());
    let broken = events.broken.clone();
    system.native_birth = Some(state);
    tally.check("warmAccepted", result.is_ok() && broken.is_none(), label);
    if result.is_err() || broken.is_some() {
        tally.refused.push(format!("{label}: {result:?} {broken:?}"));
        return;
    }
    let records = warm["records"].as_array().unwrap();
    let commands = warm["commands"].as_array().unwrap();
    let native_births: Vec<&Value> = records.iter().filter(|r| r["trigger"].as_u64() == Some(0)).collect();
    let native_deaths: Vec<&Value> = records.iter().filter(|r| r["trigger"].as_u64() == Some(2)).collect();
    tally.check("recordTriggers", native_births.len() + native_deaths.len() == records.len(), label);
    // Records and commands of each slice, keyed by its pending time.
    let mut per_slice: HashMap<u32, (usize, usize)> = HashMap::new();
    // ---- birth records in the order made
    let ours_births: Vec<_> = trace.iter().flat_map(|call| {
        let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
        call.records.iter().map(move |(slot, index, record)| (call, *slot, index + shift, record))
    }).collect();
    tally.check("birthRecordCount", ours_births.len() == native_births.len(), label);
    for ((call, slot, index, record), native) in ours_births.iter().zip(&native_births) {
        let at = format!("{label} birth record {slot}/{index}");
        let entry = per_slice.entry(call.owner.accumulated_time.to_bits()).or_default();
        entry.0 += 1;
        entry.1 += record.commands.as_ref().map_or(0, |pair| pair.len());
        tally.check("birthRecordSlotIndex", word(&native["slot"]) as usize == *slot
            && word(&native["index"]) as usize == *index, &at);
        let interval = record.interval;
        let times = vec![interval.previous.to_bits(), interval.current.to_bits(),
            interval.previous_normalized.to_bits(), interval.current_normalized.to_bits(),
            edges.births[*slot].law.duration().to_bits()];
        tally.check("birthRecordWindow", words(&native["timesBits"]) == times, &at);
        tally.check("birthRecordPendingTime", word(&native["stateZero"]) == call.owner.accumulated_time.to_bits(), &at);
        tally.check("birthRecordStateBefore", words(&native["stateBefore"]) == event_state(&record.before), &at);
        tally.check("birthRecordStateAfter", words(&native["stateAfter"]) == event_state(&record.after), &at);
        let ids = words(&native["commandIds"]);
        match &record.commands {
            None => tally.check("birthCommandCount", ids.is_empty(), &at),
            Some(pair) => {
                tally.check("birthCommandCount", ids.len() == pair.len(), &at);
                for (command, id) in pair.iter().zip(&ids) {
                    compare_command(tally, command, &commands[*id as usize], &at, &BIRTH_COMMAND);
                }
            }
        }
    }
    tally.count("birthRecords", ours_births.len().min(native_births.len()));
    // ---- death records in the order the kill passes made them
    let ours_deaths: Vec<_> = death_trace.iter().flat_map(|call| {
        let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
        call.deaths.iter().flat_map(move |(dying, recorded)| recorded.iter().map(move |record| (call, dying, shift, record)))
    }).collect();
    tally.check("deathRecordCount", ours_deaths.len() == native_deaths.len(), label);
    for ((call, dying, shift, record), native) in ours_deaths.iter().zip(&native_deaths) {
        let at = format!("{label} death record {}/{}", record.edge, dying.index);
        let entry = per_slice.entry(call.owner.accumulated_time.to_bits()).or_default();
        entry.0 += 1;
        entry.1 += record.commands.as_ref().map_or(0, |pair| pair.len());
        tally.check("deathRecordPath", words(&native["timesBits"])
            == moly_law::particle::death_event::DEATH_TIMES.map(f32::to_bits), &at);
        tally.check("deathRecordSlotIndex", word(&native["slot"]) as usize == record.edge
            && word(&native["index"]) as usize == dying.index + shift, &at);
        tally.check("deathRecordPendingTime", word(&native["stateZero"]) == call.owner.accumulated_time.to_bits(), &at);
        tally.check("deathStateBefore", words(&native["stateBefore"]) == record.state_before, &at);
        tally.check("deathStateAfter", words(&native["stateAfter"]) == record.state_after, &at);
        let ids = words(&native["commandIds"]);
        match &record.commands {
            None => tally.check("deathCommandCount", ids.is_empty(), &at),
            Some(pair) => {
                tally.check("deathCommandCount", ids.len() == pair.len(), &at);
                for (command, id) in pair.iter().zip(&ids) {
                    compare_command(tally, command, &commands[*id as usize], &at, &DEATH_COMMAND);
                }
            }
        }
    }
    tally.count("deathRecords", ours_deaths.len().min(native_deaths.len()));
    tally.count("commands", commands.len());
    // The native slice rows count the records and commands made before the
    // slice starts.
    let (mut records_before, mut commands_before) = (0, 0);
    for (k, (slice, native)) in slices.iter().zip(native_slices).enumerate() {
        tally.check("sliceRecordsCommandsBefore", (records_before, commands_before)
            == (word(&native["records"]) as usize, word(&native["commands"]) as usize), &format!("{label} slice {k}"));
        let (made, issued) = per_slice.get(&slice.remaining_before.to_bits()).copied().unwrap_or_default();
        records_before += made;
        commands_before += issued;
    }
    tally.check("recordsCommandsTotal", (records_before, commands_before) == (records.len(), commands.len()), label);
    // ---- the kills in removal order
    let kills = warm["kills"].as_array().unwrap();
    let ours_kills: Vec<(usize, u32)> = death_trace.iter().flat_map(|call| {
        let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
        call.deaths.iter().map(move |(dying, _)| (dying.index + shift, dying.seed))
    }).collect();
    let native_kills: Vec<(usize, u32)> = kills.iter().map(|k| (word(&k["index"]) as usize, word(&k["seed"]))).collect();
    tally.check("kills", ours_kills == native_kills, label);
    tally.count("kills", native_kills.len());
    // ---- the pool and the parent's state after the warm
    let pool = &warm["pool"];
    let count_ok = system.pool.len() == word(&pool["count"]) as usize;
    tally.check("poolCount", count_ok, label);
    if count_ok {
        tally.check("poolSeeds", system.side.iter().map(|s| s.seed).collect::<Vec<_>>() == words(&pool["seed"]), label);
        tally.check("poolAges", system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>()
            == words(&pool["age"]), label);
        tally.check("poolInverseLifetimes", system.pool.iter().map(|p| p.inverse_lifetime.to_bits()).collect::<Vec<_>>()
            == words(&pool["inv"]), label);
        let py = words(&pool["py"]);
        tally.check("poolHeights", system.pool.iter().enumerate().all(|(i, p)| same_value(p.position[1], py[i])), label);
    }
    tally.count("alive", system.pool.len());
    let state = system.native_birth.as_ref().unwrap();
    tally.check("pendingAfter", state.frame.pending.to_bits() == word(&warm["stateZeroAfter"]), label);
    tally.check("clockAfter", system.playback_head.to_bits() == word(&warm["clockAfter"]), label);
    let d = state.emission.distribution;
    let mut emission = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
    emission.extend(state.emission.random.words);
    tally.check("emissionStateAfter", emission == words(&warm["emissionStateAfter"]), label);
}

#[test]
#[ignore = "MOLY_WARM_PARENT_RECEIPT and MOLY_WARM_CORPUS must identify the native parent-warm receipt and the corpus it read"]
fn product_parent_warm_matches_native_without_collision() {
    let receipt = read("MOLY_WARM_PARENT_RECEIPT");
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    let doc = corpus_file("008_thunder/fx/effects.json");
    let index = corpus_file("index.json");
    let runs = receipt["runs"].as_array().unwrap();
    // The run whose collision module is off is the one the product can
    // reproduce: the others collide with a single ground plane, which is not
    // the exported ground scene.
    let run = runs.iter().find(|run| run["collision"] == false).expect("the run without collision");
    let arm = |arm: WarmArm| {
        let mut tally = Tally::default();
        replay_warm(&doc, &index, run, arm, &mut tally);
        tally
    };
    let tally = arm(WarmArm::Product);
    tally.print("product parent warm");
    assert!(tally.first.is_empty(), "first mismatches: {:#?}", tally.first);
    assert!(tally.counts["birthRecords"] > 0 && tally.counts["deathRecords"] > 0 && tally.counts["kills"] > 0);
    // Without the census rewrite the World parent keeps mode 1 and the native
    // path refuses it by name.
    let kept = warm_parent(&doc, &index, run, true).map(|_| ());
    println!("emitter velocity mode kept: {kept:?}");
    assert!(kept.is_err());
    let mut red = Vec::new();
    for (name, mutant) in [("warmWithoutChildTerm", WarmArm::NoChildTerm), ("catchUpIsStep", WarmArm::Arm("catchUpIsStep")),
        ("catchUpAfterDecrement", WarmArm::Arm("catchUpAfterDecrement")), ("catchUpZero", WarmArm::Arm("catchUpZero"))] {
        let tally = arm(mutant);
        println!("arm {name}: {} mismatched fields {:?}", tally.mismatched(), tally.first.iter().take(3).collect::<Vec<_>>());
        red.push((name, tally.mismatched()));
    }
    assert!(red.iter().all(|(_, wrong)| *wrong > 0), "{red:?}");
}

// ---------------------------------------------------------------- the targets

/// A target of the parent as the product installs it: the exported record,
/// its render geometry, the child owner words and streams of the native
/// image.
fn warm_target(doc: &Value, seq: &Value, image: &Value) -> Result<Runtime, String> {
    let source = &seq["source"];
    let effect = source["effect"].as_str().unwrap();
    let node = seq["node"].as_str().unwrap();
    let effect_doc = &doc["effects"][effect];
    let found: Vec<&Value> = effect_doc["particles"].as_array().unwrap().iter().filter(|p| p["node"] == node).collect();
    assert_eq!(found.len(), 1, "{node}");
    let record = found[0];
    assert_eq!(record["systemPathId"], source["systemPathId"], "{node}: another system at that node");
    let by_path: HashMap<String, &Value> = effect_doc["nodes"].as_array().unwrap().iter()
        .map(|n| (n["path"].as_str().unwrap().to_owned(), n)).collect();
    let scaling = crate::weather_fx::source_scaling(&record["system"], &by_path, node, true)?;
    let emitter = decode(record)?;
    let mut system = test_support::runtime();
    system.geometry = if record["renderer"]["renderMode"] == "Mesh" {
        let pivot: Vec<f32> = record["renderer"]["pivot"].as_array().unwrap().iter()
            .map(|v| v.as_f64().unwrap() as f32).collect();
        Geometry::Mesh(crate::particle_geometry::MeshDraw {
            source: std::sync::Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
                uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), bounds_size: Vec3::ZERO }),
            scaling,
            alignment: crate::particle_geometry::Alignment::from_source(record["renderer"]["alignment"].as_i64().unwrap())
                .expect("source mesh alignment"),
            pivot: Vec3::new(pivot[0], pivot[1], pivot[2]),
        })
    } else {
        test_support::source_billboard(scaling)
    };
    system.effect = emitter.effect.clone();
    system.node = node.to_owned();
    system.kind = EffectKind::Site;
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier);
    system.velocity_law = emitter.velocity_over_lifetime.as_ref()
        .map(moly_law::particle::velocity::VelocityOverLifetime::from_params);
    system.rol = emitter.rotation_over_lifetime.as_ref().map(|p|
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve).unwrap());
    system.size_law = emitter.size_over_lifetime.as_ref().map(moly_law::particle::size::SizeOverLifetime::from_params);
    system.color_law = emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = emitter.custom_data.as_ref().map(moly_law::particle::custom_data::CustomData::from_params);
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.prewarmed = true;
    system.born_total = 0;
    system.native_birth = None;
    assert_eq!(word(&image["maximum"]), system.emitter.max_particles, "{node}");
    assert_eq!(f(&image["duration"]), system.emitter.duration, "{node}");
    assert_eq!(image["looping"].as_bool(), Some(system.emitter.looping), "{node}");
    assert_eq!(word(&image["simulation"]), u32::from(system.emitter.simulation_space == SimulationSpace::World), "{node}");
    let owner = ChildOwner {
        local_to_world: floats(&image["owner"]),
        world_to_local: floats(&image["inverse"]),
        local_rotation: floats(&image["st114"]),
        emitter_scale: floats(&image["scale"]),
        shape_scale: floats(&image["shapeScale"]),
    };
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    install_child_target(&mut system, &mut manager, owner)?;
    let state = system.native_birth.as_mut().unwrap();
    state.initial = module_random(&image["initialWords"]);
    state.shape = module_random(&image["shapeWords"]);
    Ok(system)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TargetArm {
    Product,
    /// The target takes no update in the warm frame.
    NoOwnWarm,
    /// One of the product's one-rule arms on the child's gate.
    Arm(&'static str),
}

fn compare_pool(tally: &mut Tally, system: &Runtime, pool: &Value, fields: &'static [&'static str; 5], at: &str) {
    let count_ok = system.pool.len() == word(&pool["count"]) as usize;
    tally.check(fields[0], count_ok, at);
    if count_ok {
        tally.check(fields[1], system.side.iter().map(|s| s.seed).collect::<Vec<_>>() == words(&pool["seed"]), at);
        tally.check(fields[2], system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>()
            == words(&pool["age"]), at);
        tally.check(fields[3], system.pool.iter().map(|p| p.inverse_lifetime.to_bits()).collect::<Vec<_>>()
            == words(&pool["inv"]), at);
        let py = words(&pool["py"]);
        tally.check(fields[4], system.pool.iter().enumerate().all(|(i, p)| same_value(p.position[1], py[i])), at);
    }
}

const EMIT_POOL: [&str; 5] = ["emitPoolCount", "emitPoolSeeds", "emitPoolAges", "emitPoolInverseLifetimes", "emitPoolHeights"];
const OWN_POOL: [&str; 5] = ["framePoolCount", "framePoolSeeds", "framePoolAges", "framePoolInverseLifetimes",
    "framePoolHeights"];
const FRAME_POOL: [&str; 5] = ["frameEmitPoolCount", "frameEmitPoolSeeds", "frameEmitPoolAges",
    "frameEmitPoolInverseLifetimes", "frameEmitPoolHeights"];

/// Hands the native commands `emits` names (their ids into `commands`) to
/// the target in order and checks what each one bore.
fn deliver(tally: &mut Tally, system: &mut Runtime, emits: &[Value], commands: &[Value], child: &str, frame_dt: f32,
    at: &str) {
    for emit in emits {
        let row = &commands[word(&emit["id"]) as usize];
        assert_eq!(word(&row["id"]), word(&emit["id"]));
        assert_eq!(row["target"].as_str(), Some(child), "{at}: the command's target");
        let command = native_command(row);
        assert_eq!(command.catch_up.to_bits(), word(&emit["c74Bits"]));
        let born = crate::particle_runtime::deliver_command(system, &command, frame_dt);
        let expected = word(&emit["after"]) as usize - word(&emit["before"]) as usize;
        tally.check("emitAccepted", born.is_ok(), at);
        tally.check("emitBorn", born.as_ref().is_ok_and(|born| *born == expected), at);
        tally.count("emits", 1);
        tally.count("emitBirths", expected);
    }
}

fn replay_target(doc: &Value, run: &Value, seq: &Value, variant: &str, arm: TargetArm, tally: &mut Tally) {
    let block = &seq[variant];
    let label = format!("{} {} {variant}", run["label"].as_str().unwrap(), seq["node"].as_str().unwrap());
    let image = &block["image"];
    let mut system = match warm_target(doc, seq, image) {
        Ok(system) => system,
        Err(reason) => {
            tally.refused.push(format!("{label}: {reason}"));
            return;
        }
    };
    tally.count("targets", 1);
    let ctx = Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let child = seq["slot"]["child"].as_str().unwrap();
    // ---- the target's own update in the warm frame
    let own = &block["ownWarm"];
    assert_eq!(word(&own["dtBits"]), word(&run["compute"]["outBits"]), "{label}: the warm's dt");
    let warm_dt = f(&own["dtBits"]);
    if arm != TargetArm::NoOwnWarm {
        advance_frame(&mut system, warm_dt, &ctx, true);
    }
    tally.check("ownWarmClock", system.playback_head.to_bits() == word(&own["clockAfter"]), &label);
    tally.check("ownWarmPending", system.native_birth.as_ref().unwrap().frame.pending.to_bits() == word(&own["stateZeroAfter"]),
        &label);
    tally.check("ownWarmPool", system.pool.len() == word(&own["pool"]["count"]) as usize, &label);
    // ---- the warm's commands in the order recorded
    child::arms::set(match arm { TargetArm::Arm(name) => Some(name), _ => None });
    deliver(tally, &mut system, block["emits"].as_array().unwrap(), run["warm"]["commands"].as_array().unwrap(), child,
        warm_dt, &label);
    compare_pool(tally, &system, &block["poolAfterEmits"], &EMIT_POOL, &label);
    tally.count("poolAfterWarm", system.pool.len());
    // ---- the ordinary frames after the warm
    for (k, frame) in block.get("frames").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let at = format!("{label} frame {k}");
        let own = &frame["own"];
        advance_frame(&mut system, f(&own["dtBits"]), &ctx, true);
        tally.check("frameClock", system.playback_head.to_bits() == word(&own["clockAfter"]), &at);
        tally.check("framePending", system.native_birth.as_ref().unwrap().frame.pending.to_bits()
            == word(&own["stateZeroAfter"]), &at);
        compare_pool(tally, &system, &own["pool"], &OWN_POOL, &at);
        let commands = run["frames"][k]["commands"].as_array().unwrap();
        deliver(tally, &mut system, frame["emits"].as_array().unwrap(), commands, child, f(&own["dtBits"]), &at);
        compare_pool(tally, &system, &frame["poolAfter"], &FRAME_POOL, &at);
        tally.count("frames", 1);
    }
    child::arms::set(None);
}

#[test]
#[ignore = "MOLY_WARM_PARENT_RECEIPT, MOLY_WARM_TARGET_RECEIPT and MOLY_WARM_CORPUS must identify the native parent-warm and target receipts and the corpus they read"]
fn product_targets_of_the_warm_match_native() {
    let warm = read("MOLY_WARM_PARENT_RECEIPT");
    let targets = read("MOLY_WARM_TARGET_RECEIPT");
    assert_eq!(warm["sourceSha256"], SOURCE_SHA256);
    assert_eq!(targets["sourceSha256"], SOURCE_SHA256);
    let doc = corpus_file("008_thunder/fx/effects.json");
    let runs = warm["runs"].as_array().unwrap();
    let run_of = |seq: &Value| runs.iter().find(|run| run["label"] == seq["parent"]).expect("the sequence's parent run");
    let sequences = targets["sequences"].as_array().unwrap();
    let replay = |arm: TargetArm, variant: &str| {
        let mut tally = Tally::default();
        for seq in sequences.iter().filter(|seq| seq.get(variant).is_some()) {
            replay_target(&doc, run_of(seq), seq, variant, arm, &mut tally);
        }
        tally
    };
    let product = replay(TargetArm::Product, "warm");
    product.print("product targets of the warm (flags 8)");
    let flags0 = replay(TargetArm::Product, "flags0");
    flags0.print("product targets of the warm (flags 0 control)");
    let flags1 = replay(TargetArm::Product, "flags1");
    println!("native flags 1 (catch-up) against the product: {} mismatched fields", flags1.mismatched());
    assert!(product.first.is_empty(), "first mismatches: {:#?}", product.first);
    assert!(flags0.first.is_empty(), "first mismatches: {:#?}", flags0.first);
    assert!(product.counts.get("targets").is_some_and(|n| *n > 0) && product.counts.get("frames").is_some_and(|n| *n > 0));
    let mut red = Vec::new();
    for (name, arm) in [("noOwnWarm", TargetArm::NoOwnWarm), ("noUpperGate", TargetArm::Arm("noUpperGate")),
        ("gateUsesOrderedMax", TargetArm::Arm("gateUsesOrderedMax"))] {
        let tally = replay(arm, "warm");
        println!("arm {name}: {} mismatched fields {:?}", tally.mismatched(), tally.first.iter().take(3).collect::<Vec<_>>());
        red.push((name, tally.mismatched()));
    }
    assert!(red.iter().all(|(_, wrong)| *wrong > 0), "{red:?}");
}
