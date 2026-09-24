//! Parent sub-emitter death events through the product slice path against
//! native incremental updates of the ground-strike parents carrying death
//! edges. The corpus has no death-edge parent the native harness hosts (each
//! also collides), so the runs add death edges to real children of the same
//! effect, and some change the start lifetime, the simulation space or write
//! the age a collision kill leaves (0x42c80001) between slices; every such
//! override is named in the receipt and applied here to the exported record
//! and the pool before the product runs. As in the birth-event replay, the
//! probe RNG words, the owner matrix and the per-frame pending time and slice
//! length replace the installed streams and the frame head.
//!
//! Every frame is compared with its native row: the sub-emitter calls (kind,
//! range, lane times, pending time, emission word), every birth record and
//! its commands, every death record in the order the kill passes made them
//! (edge slot, dying slot, the particle it read, pending time, state words,
//! command bytes), the kills per pass (recording or not), then the pool after
//! the frame (count, seeds, ages, carries, positions and velocities) and the
//! parent's pending time, clock and emission state. Zeros in positions,
//! velocities and the owner compare by value (the X-axis reflection can flip
//! a zero's sign); everything else by bits.
use super::*;
use super::sub_event_samples::{event_state, f, hex, parent_runtime, read, same_value, word, words};
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";
/// Return addresses: the newborn sub-emitter call, the newborn kill pass's
/// KillParticle call, and RecordEmit's return inside KillParticle.
const NEWBORN_CALL: &str = "0xd81240";
const NEWBORN_KILL: &str = "0xd8129c";
const KILL_RETURN: &str = "0xd6fe78";
const AGE_KILLED: u32 = 0x42c8_0001;

#[derive(Default)]
struct Tally {
    frames: usize,
    calls: usize,
    birth_records: usize,
    death_records: usize,
    death_commands: usize,
    kills: usize,
    newborn_kills: usize,
    unrecorded_kills: usize,
    age_writes: usize,
    local_death_commands: usize,
    refused_runs: Vec<String>,
    fields: BTreeMap<&'static str, (usize, usize)>,
    first: Vec<String>,
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

    fn bad(&self, field: &str) -> usize {
        self.fields.get(field).map_or(0, |(_, bad)| *bad)
    }

    fn mismatched(&self) -> usize {
        self.fields.values().map(|(_, bad)| bad).sum()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arm {
    Product,
    /// The owner matrix left at identity.
    IdentityOwner,
    /// The newborn kill pass records every kill, ignoring its counter.
    NewbornAlwaysRecords,
    /// The existing kill pass reads the emission word after this slice's
    /// draws.
    WordAfterDraws,
}

impl Arm {
    fn name(self) -> Option<&'static str> {
        match self {
            Arm::NewbornAlwaysRecords => Some("newbornDeathAlwaysRecords"),
            Arm::WordAfterDraws => Some("deathWordAfterDraws"),
            Arm::Product | Arm::IdentityOwner => None,
        }
    }
}

fn compare_command(tally: &mut Tally, ours: &moly_law::particle::sub_emission::SubEmitterCommand, native: &Value,
    at: &str, prefix: &'static [&'static str; 4]) {
    let raw = hex(&native["rawHex"]);
    let bytes = ours.to_bytes();
    // The pointer word and the padding word after the inherited block are
    // not fields.
    let vectors = raw.len() == 0x78 && (0x08..0x20).step_by(4).all(|w| same_value(
        f32::from_le_bytes(bytes[w..w + 4].try_into().unwrap()),
        u32::from_le_bytes(raw[w..w + 4].try_into().unwrap())));
    tally.check(prefix[0], vectors, at);
    tally.check(prefix[1], raw.get(0x20..0x54) == Some(&bytes[0x20..0x54]), at);
    tally.check(prefix[2], raw.get(0x58..0x78) == Some(&bytes[0x58..0x78]), at);
    tally.check(prefix[3], hex(&native["emissionHex"]) == ours.emission_bytes(), at);
}

const BIRTH_COMMAND: [&str; 4] = ["birthCommandPositionVelocity", "birthCommandInherited", "birthCommandCountTimes",
    "birthCommandEmission"];
const DEATH_COMMAND: [&str; 4] = ["deathCommandPositionVelocity", "deathCommandInherited", "deathCommandCountTimes",
    "deathCommandEmission"];

fn run_parent(doc: &Value, parent: &Value, arm: Arm, tally: &mut Tally) {
    let label = parent["label"].as_str().unwrap();
    let image = &parent["image"];
    let overrides = &parent["overrides"];
    let parsed = parent_runtime(doc, &parent["source"], |record| {
        let block = record.get_mut("system").unwrap();
        block["subEmitters"].as_array_mut().unwrap().extend(overrides["addedEdges"].as_array().unwrap().iter().cloned());
        if let Some(lifetime) = overrides.get("lifetime") {
            block["start"]["lifetime"] = lifetime.clone();
        }
        if let Some(space) = overrides.get("simulationSpace") {
            block["simulationSpace"] = space.clone();
        }
    });
    // The native run holds the emitter still, but the product path refuses a
    // World-space emitter velocity other than the Transform mode; such a run
    // is counted, not compared.
    let (mut system, edges) = match parsed {
        Ok(parsed) => parsed,
        Err(reason) => {
            tally.refused_runs.push(format!("{label}: {reason}"));
            return;
        }
    };
    // The native table order: birth edges, then death edges, each authored.
    let image_edges = image["subEmitters"]["edges"].as_array().unwrap();
    let nodes = |trigger: u64| image_edges.iter().filter(|e| e["trigger"].as_u64() == Some(trigger))
        .map(|e| e["node"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(nodes(0), edges.births.iter().map(|e| e.target.clone()).collect::<Vec<_>>(), "{label}: birth slots");
    assert_eq!(nodes(2), edges.deaths.iter().map(|e| e.target.clone()).collect::<Vec<_>>(), "{label}: death slots");
    assert!(!edges.deaths.is_empty(), "{label}");
    assert_eq!(word(&image["subEmitters"]["numEmitAccumulators"]) as usize, edges.births.len().min(2));
    assert_eq!(word(&image["maximum"]), system.emitter.max_particles, "{label}");
    assert_eq!(f(&image["duration"]), system.emitter.duration, "{label}");
    let world = system.emitter.simulation_space == SimulationSpace::World;
    assert_eq!(word(&image["simulation"]), u32::from(world), "{label}");
    // Runtime X is the reflection of source X.
    let owner_words = words(&image["owner"]);
    let source_owner: [f32; 16] = std::array::from_fn(|i| f32::from_bits(owner_words[i]));
    let runtime_owner: [f32; 16] = std::array::from_fn(|i|
        if (i % 4 == 0) ^ (i / 4 == 0) { -source_owner[i] } else { source_owner[i] });
    system.node_affine = match arm {
        Arm::IdentityOwner => GlobalTransform::IDENTITY,
        _ => GlobalTransform::from(Mat4::from_cols_array(&runtime_owner)),
    };
    let ctx = Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(matches!(install_native_birth(&mut system, &mut manager, &SourceRoute::Ordinary, None).unwrap(), BirthPath::Native));
    {
        let state = system.native_birth.as_mut().unwrap();
        state.owner = None;
        let module = |key: &str| {
            let w = words(&image[key]);
            assert_eq!(w.len(), 16);
            ModuleRandom { words: std::array::from_fn(|i| std::array::from_fn(|lane| w[i * 4 + lane])) }
        };
        state.initial = module("initialWords");
        state.shape = module("shapeWords");
        let emission = words(&image["emissionWords"]);
        state.emission.random = ScalarRandom { words: std::array::from_fn(|i| emission[i]) };
        let mut events = BirthEvents::with_edges(edges.clone());
        events.trace = Some(Vec::new());
        events.death_trace = Some(Vec::new());
        state.events = Some(events);
    }
    system.playback_head = f(&image["clock"]);
    system.previous_head = system.playback_head;
    let mut writes: HashMap<u64, Vec<usize>> = HashMap::new();
    for write in overrides["ageWrites"].as_array().unwrap() {
        writes.entry(write[0].as_u64().unwrap()).or_default().push(write[1].as_u64().unwrap() as usize);
    }
    let subcalls = parent["subcalls"].as_array().unwrap();
    let records = parent["records"].as_array().unwrap();
    let commands = parent["commands"].as_array().unwrap();
    let kills = parent["kills"].as_array().unwrap();
    for frame in parent["frames"].as_array().unwrap() {
        let at = format!("{label} frame {}", frame["frame"]);
        for &index in writes.get(&frame["frame"].as_u64().unwrap()).into_iter().flatten() {
            system.pool[index].age_percent = f32::from_bits(AGE_KILLED);
            tally.age_writes += 1;
        }
        let mut state = system.native_birth.take().unwrap();
        child::arms::set(arm.name());
        let result = birth::run_incremental(&mut system, &mut state, f(&frame["remainingBits"]),
            f(&frame["stepBits"]), false, &ctx);
        child::arms::set(None);
        let events = state.events.as_mut().unwrap();
        let trace = std::mem::take(events.trace.as_mut().unwrap());
        let death_trace = std::mem::take(events.death_trace.as_mut().unwrap());
        let broken = events.broken.clone();
        system.native_birth = Some(state);
        tally.check("frameAccepted", result.is_ok() && broken.is_none(), &at);
        if result.is_err() || broken.is_some() {
            continue;
        }
        tally.frames += 1;
        let span = |key: &str| {
            let range = words(&frame[key]);
            range[0] as usize..range[1] as usize
        };
        // ---- sub-emitter calls
        let native_calls = &subcalls[span("subcalls")];
        tally.check("callCount", trace.len() == native_calls.len(), &at);
        for (call, native) in trace.iter().zip(native_calls) {
            tally.calls += 1;
            let newborn = native["lr"].as_str() == Some(NEWBORN_CALL);
            tally.check("callKind", call.newborn == newborn, &at);
            let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
            tally.check("callRange", word(&native["start"]) as usize == call.start + shift
                && word(&native["end"]) as usize == call.end + shift, &at);
            tally.check("callLaneTimes", words(&native["dt4"]) == call.dt4.map(f32::to_bits), &at);
            tally.check("callPendingTime", word(&native["stateZero"]) == call.owner.accumulated_time.to_bits(), &at);
            tally.check("callEmissionWord", word(&native["ownerSeed"]) == call.owner.emission_word, &at);
        }
        // ---- birth and death records, each in the order made
        let frame_records = &records[span("records")];
        let native_births: Vec<&Value> = frame_records.iter().filter(|r| r["trigger"].as_u64() == Some(0)).collect();
        let native_deaths: Vec<&Value> = frame_records.iter().filter(|r| r["trigger"].as_u64() == Some(2)).collect();
        tally.check("recordTriggers", native_births.len() + native_deaths.len() == frame_records.len(), &at);
        let ours_births: Vec<_> = trace.iter().flat_map(|call| {
            let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
            call.records.iter().map(move |(slot, index, record)| (*slot, index + shift, record))
        }).collect();
        tally.check("birthRecordCount", ours_births.len() == native_births.len(), &at);
        for ((slot, index, record), native) in ours_births.iter().zip(&native_births) {
            tally.birth_records += 1;
            let at = format!("{at} birth record {slot}/{index}");
            tally.check("birthRecordSlotIndex", word(&native["slot"]) as usize == *slot
                && word(&native["index"]) as usize == *index, &at);
            let interval = record.interval;
            let times = vec![interval.previous.to_bits(), interval.current.to_bits(),
                interval.previous_normalized.to_bits(), interval.current_normalized.to_bits(),
                edges.births[*slot].law.duration().to_bits()];
            tally.check("birthRecordWindow", words(&native["timesBits"]) == times, &at);
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
        let ours_deaths: Vec<_> = death_trace.iter().flat_map(|call| {
            let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
            call.deaths.iter().flat_map(move |(dying, recorded)|
                recorded.iter().map(move |record| (call, dying, shift, record)))
        }).collect();
        tally.check("deathRecordCount", ours_deaths.len() == native_deaths.len(), &at);
        for ((call, dying, shift, record), native) in ours_deaths.iter().zip(&native_deaths) {
            tally.death_records += 1;
            let at = format!("{at} death record {}/{}", record.edge, dying.index);
            tally.check("deathRecordPath", native["returnTo"].as_str() == Some(KILL_RETURN)
                && words(&native["timesBits"]) == moly_law::particle::death_event::DEATH_TIMES.map(f32::to_bits), &at);
            tally.check("deathRecordSlotIndex", word(&native["slot"]) as usize == record.edge
                && word(&native["index"]) as usize == dying.index + shift, &at);
            let p = &native["particle"];
            tally.check("deathParticleSeed", word(&p["seed"]) == dying.seed, &at);
            let (position, velocity, animated) = (words(&p["position"]), words(&p["velocity"]), words(&p["animated"]));
            tally.check("deathParticlePosition", (0..3).all(|a| same_value(dying.position[a], position[a])), &at);
            tally.check("deathParticleVelocity", (0..3).all(|a| same_value(dying.velocity[a], velocity[a])
                && same_value(dying.animated[a], animated[a])), &at);
            tally.check("deathPendingTime", word(&native["stateZero"]) == call.owner.accumulated_time.to_bits(), &at);
            tally.check("deathStateBefore", words(&native["stateBefore"]) == record.state_before, &at);
            tally.check("deathStateAfter", words(&native["stateAfter"]) == record.state_after, &at);
            let ids = words(&native["commandIds"]);
            match &record.commands {
                None => tally.check("deathCommandCount", ids.is_empty(), &at),
                Some(pair) => {
                    tally.check("deathCommandCount", ids.len() == pair.len(), &at);
                    for (command, id) in pair.iter().zip(&ids) {
                        tally.death_commands += 1;
                        tally.local_death_commands += usize::from(!world);
                        compare_command(tally, command, &commands[*id as usize], &at, &DEATH_COMMAND);
                    }
                }
            }
        }
        // ---- the kills of each pass, recording or not
        let frame_kills = &kills[span("kills")];
        let native_newborn = frame_kills.iter().filter(|k| k["lr"].as_str() == Some(NEWBORN_KILL)).count();
        let recorded = |newborn: bool| frame_kills.iter().filter(|k| (k["lr"].as_str() == Some(NEWBORN_KILL)) == newborn
            && k["record"].as_u64() == Some(1)).count();
        let ours = |newborn: bool| death_trace.iter().filter(|c| c.newborn == newborn).map(|c| c.deaths.len()).sum::<usize>();
        tally.kills += frame_kills.len();
        tally.newborn_kills += native_newborn;
        tally.unrecorded_kills += frame_kills.iter().filter(|k| k["record"].as_u64() == Some(0)).count();
        tally.check("recordingKillsExisting", recorded(false) == ours(false), &at);
        tally.check("recordingKillsNewborn", recorded(true) == ours(true), &at);
        // ---- the pool and the parent state after the frame
        let after = &frame["after"];
        let count = word(&after["count"]) as usize;
        let count_ok = system.pool.len() == count;
        tally.check("frameCount", count_ok, &at);
        if count_ok {
            tally.check("frameSeeds", system.side.iter().map(|s| s.seed).collect::<Vec<_>>() == words(&after["seed"]), &at);
            tally.check("frameAges", system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>()
                == words(&after["age"]), &at);
            tally.check("frameCarries", system.side.iter().map(|s| s.emit_carry[0].to_bits()).collect::<Vec<_>>()
                == words(&after["carry0"]) && system.side.iter().map(|s| s.emit_carry[1].to_bits()).collect::<Vec<_>>()
                == words(&after["carry1"]), &at);
            let pool = &frame["afterPool"];
            let axis = |key: &str| words(&pool[key]);
            let (px, py, pz, vx, vy, vz) = (axis("px"), axis("py"), axis("pz"), axis("vx"), axis("vy"), axis("vz"));
            tally.check("framePositions", system.pool.iter().enumerate().all(|(i, p)|
                same_value(-p.position[0], px[i]) && same_value(p.position[1], py[i]) && same_value(p.position[2], pz[i])), &at);
            tally.check("frameVelocities", system.pool.iter().enumerate().all(|(i, p)|
                same_value(-p.velocity[0], vx[i]) && same_value(p.velocity[1], vy[i]) && same_value(p.velocity[2], vz[i])), &at);
        }
        let state = system.native_birth.as_ref().unwrap();
        tally.check("framePending", state.frame.pending.to_bits() == word(&frame["stateZeroAfter"]), &at);
        tally.check("frameClock", system.playback_head.to_bits() == word(&frame["clockAfter"]), &at);
        let emission = words(&frame["emissionStateAfter"]);
        let d = state.emission.distribution;
        let mut ours = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
        ours.extend(state.emission.random.words);
        tally.check("frameEmissionState", ours == emission, &at);
    }
}

#[test]
#[ignore = "MOLY_DEATH_PARENT_RECEIPT and MOLY_SUBEMITTER_PARENT_EFFECTS must identify the native death-edge parent receipt and the exported 008 effects"]
fn product_parent_death_events_match_current_native() {
    let receipt = read("MOLY_DEATH_PARENT_RECEIPT");
    assert_eq!(receipt["librarySha256"], SOURCE_SHA256);
    let doc = read("MOLY_SUBEMITTER_PARENT_EFFECTS");
    let parents = receipt["parentEvents"].as_array().unwrap();
    let run = |arm: Arm| {
        let mut tally = Tally::default();
        for parent in parents {
            run_parent(&doc, parent, arm, &mut tally);
        }
        tally
    };
    let tally = run(Arm::Product);
    println!("product parent death events: {} runs, {} frames, {} calls, {} birth records, {} death records \
        ({} death commands, {} of them Local), kills {} ({} newborn, {} not recording), age writes {}, {} mismatched fields",
        parents.len(), tally.frames, tally.calls, tally.birth_records, tally.death_records, tally.death_commands,
        tally.local_death_commands, tally.kills, tally.newborn_kills, tally.unrecorded_kills, tally.age_writes,
        tally.mismatched());
    for (field, (checked, bad)) in &tally.fields {
        println!("  {field}: {checked} checked, {bad} mismatched");
    }
    println!("runs the product path refuses: {:?}", tally.refused_runs);
    assert!(tally.refused_runs.len() <= 1 && tally.refused_runs.iter().all(|r| r.contains("World space")));
    assert!(tally.first.is_empty(), "first mismatches: {:#?}", tally.first);
    assert!(tally.death_records > 0 && tally.newborn_kills > 0 && tally.unrecorded_kills > 0 && tally.age_writes > 0);
    let owner = run(Arm::IdentityOwner);
    let always = run(Arm::NewbornAlwaysRecords);
    let after_draws = run(Arm::WordAfterDraws);
    println!("arm identityOwner: deathCommandPositionVelocity {} mismatched", owner.bad("deathCommandPositionVelocity"));
    println!("arm newbornDeathAlwaysRecords: deathRecordCount {} mismatched, recordingKillsNewborn {}",
        always.bad("deathRecordCount"), always.bad("recordingKillsNewborn"));
    println!("arm deathWordAfterDraws: deathStateBefore {} mismatched", after_draws.bad("deathStateBefore"));
    assert!(owner.bad("deathCommandPositionVelocity") > 0);
    assert!(always.bad("recordingKillsNewborn") > 0);
    assert!(after_draws.bad("deathStateBefore") > 0);
}
