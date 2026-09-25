//! Parent sub-emitter birth events through the product slice path against
//! the native SubModule calls of the ground-strike parents. The parent is
//! the exported system block and its children are resolved by the admission
//! helper from the same effect; the native harness wrote the probe RNG words,
//! the owner matrix and the per-frame pending time and slice length, and
//! those replace the installed streams and the frame head (they are inputs,
//! not claims about client entropy or scene placement). Every call is
//! compared with its native row: kind, range, lane times, the pending time,
//! the emission word, the owner, the particles and both carries it read;
//! then every record and every command byte; then the pool after each frame.
//!
//! The receipt's children both have zero rate and distance rate, so every
//! carry stays zero here; the law's own replay over the sub-emitter receipt
//! covers the carry. The command owner is the product-composed matrix; the
//! reflection round trip can flip the sign of a zero, so zeros in positions,
//! velocities and the owner compare by value and everything else by bits.
use super::*;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use moly_law::particle::sub_emission::{BirthEdgeLaw, EventEmission};
use moly_law::particle::schema::{SubEmitterParams, SubEmitterTrigger};
use moly_law::particle::emit::{Burst, BurstCycles};
use moly_law::particle::{schema::Effects, MinMaxCurve};
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

pub(super) fn word(value: &Value) -> u32 {
    value.as_u64().expect("native word") as u32
}

pub(super) fn f(value: &Value) -> f32 {
    f32::from_bits(word(value))
}

pub(super) fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("native array").iter().map(word).collect()
}

/// The return address the receipt names for one of its call sites (its
/// callSites block): the rows are classified by the calls they were
/// recorded from, and a receipt that does not name the site is refused.
pub(super) fn call_site<'a>(receipt: &'a Value, name: &str) -> &'a str {
    receipt["callSites"][name].as_str()
        .unwrap_or_else(|| panic!("the receipt names no callSites.{name}: it cannot classify its rows"))
}

pub(super) fn read(key: &str) -> Value {
    let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} is not set"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

pub(super) fn hex(value: &Value) -> Vec<u8> {
    let text = value.as_str().expect("hex");
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

/// A zero compares by value, anything else by bits.
pub(super) fn same_value(ours: f32, native: u32) -> bool {
    let native = f32::from_bits(native);
    if native == 0.0 { ours == 0.0 } else { ours.to_bits() == native.to_bits() }
}

pub(super) fn image_curve(c: &Value) -> MinMaxCurve {
    match word(&c["mode"]) {
        0 => MinMaxCurve::Constant(f(&c["bits"])),
        3 => MinMaxCurve::TwoConstants { min: f(&c["minBits"]), max: f(&c["maxBits"]) },
        mode => panic!("curve mode {mode} is not in this receipt"),
    }
}

/// The count law of one edge as the harness image holds the child block.
fn image_law(edge: &Value, child: &Value, births: usize) -> BirthEdgeLaw {
    let bursts = child["bursts"].as_array().unwrap().iter().map(|b| Burst {
        time: f(&b["timeBits"]),
        count: image_curve(&b["count"]),
        cycles: BurstCycles::from_serialized(word(&b["cycles"])),
        repeat_interval: f(&b["intervalBits"]),
        probability: f(&b["probabilityBits"]),
    }).collect();
    let params = SubEmitterParams {
        emitter: Some(edge["node"].as_str().unwrap().to_owned()),
        source_pointer: Default::default(),
        trigger: SubEmitterTrigger::Birth,
        properties: word(&edge["properties"]),
        probability: f(&edge["probabilityBits"]),
    };
    BirthEdgeLaw::from_params(&params, births, &image_curve(&child["startDelay"]), f(&child["durationBits"]),
        child["looping"].as_bool().unwrap(), &moly_law::particle::schema::EmissionParams {
            rate_over_time: image_curve(&child["rate"]),
            rate_over_distance: image_curve(&child["distance"]),
            bursts,
        }).expect("receipt child block is inside the law")
}

/// The parent from the exported effect, its birth edges from the admission
/// helper, and the Shape emitter state the installer reads.
fn parent_system(doc: &Value, source: &Value) -> (Runtime, Vec<BirthEdge>) {
    let (system, edges) = parent_runtime(doc, source, |_| {}).expect("native birth path");
    assert!(edges.deaths.is_empty(), "the receipt's parents have birth edges only");
    (system, edges.births)
}

/// `parent_system` with every edge the admission helper resolves; `edit`
/// changes the parent's exported record before it is decoded (a replay's
/// named overrides of the native run). A parent the native birth path
/// refuses is returned as that refusal.
pub(super) fn parent_runtime(doc: &Value, source: &Value, edit: impl FnOnce(&mut Value))
    -> Result<(Runtime, EventEdges), String> {
    parent_runtime_with(doc, source, edit, |_| {})
}

/// `parent_runtime` with `adjust` applied to the decoded emitter before the
/// native path judges it (the admission's own emitter rewrites).
pub(super) fn parent_runtime_with(doc: &Value, source: &Value, edit: impl FnOnce(&mut Value),
    adjust: impl FnOnce(&mut moly_law::particle::EmitterParams)) -> Result<(Runtime, EventEdges), String> {
    let effect = source["effect"].as_str().unwrap();
    let node = source["node"].as_str().unwrap();
    let effect_doc = &doc["effects"][effect];
    let mut particles: Vec<Value> = effect_doc["particles"].as_array().expect("effect particles").clone();
    let at = particles.iter().position(|p| p["node"] == node).expect("parent record");
    edit(&mut particles[at]);
    let particles = particles.as_slice();
    let record = &particles[at];
    assert_eq!(record["systemPathId"], source["systemPathId"], "{node}: another system at that node");
    let mesh_renderer = record["renderer"]["renderMode"] == "Mesh";
    let by_path: std::collections::HashMap<String, &Value> = effect_doc["nodes"].as_array().unwrap().iter()
        .map(|n| (n["path"].as_str().unwrap().to_owned(), n)).collect();
    let scaling = crate::weather_fx::source_scaling(&record["system"], &by_path, node, true)
        .expect("authored scaling mode");
    let wrapped = json!({"effects": {effect: {"particles": [record]}}});
    let mut decoded = Effects::from_json_str(&serde_json::to_vec(&wrapped).unwrap()).unwrap();
    assert_eq!(decoded.emitters.len(), 1);
    let mut emitter = decoded.emitters.remove(0);
    adjust(&mut emitter);
    let graph = crate::weather_fx::source_sub_emitter_owners(particles);
    let edges = crate::weather_fx::sub_emitter_edges(&emitter, &graph).expect("admission resolves the edges");
    let route = source_route(&record["system"]);
    assert_eq!(route, SourceRoute::Ordinary);
    native_birth_eligible(&emitter, &route)?;
    native_shape_state_eligible(&emitter, Some(ShapeEmitterEvidence { scaling, mesh_renderer }))
        .expect("native Shape emitter state");
    let mut system = test_support::runtime();
    // The simulation reads only the render mode and scaling of the geometry;
    // an empty mesh stands in for the source GLB, and the flip and the axis
    // body (read by the mesh transform at draw time) are zero and none.
    system.geometry = if mesh_renderer {
        let pivot: Vec<f32> = record["renderer"]["pivot"].as_array().unwrap().iter()
            .map(|v| v.as_f64().unwrap() as f32).collect();
        Geometry::Mesh(crate::particle_geometry::MeshDraw {
            source: std::sync::Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
                uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), bounds_size: Vec3::ZERO }),
            scaling,
            alignment: crate::particle_geometry::Alignment::from_source(record["renderer"]["alignment"].as_i64().unwrap())
                .expect("source mesh alignment"),
            pivot: Vec3::new(pivot[0], pivot[1], pivot[2]),
            flip: Vec3::ZERO,
            axis_body: None,
        })
    } else {
        test_support::source_billboard(scaling)
    };
    system.effect = emitter.effect.clone();
    system.node = emitter.node.clone();
    system.kind = EffectKind::Site;
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    system.velocity_law = emitter.velocity_over_lifetime.as_ref()
        .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).unwrap());
    system.rol = emitter.rotation_over_lifetime.as_ref().map(|p|
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve).unwrap());
    system.size_law = emitter.size_over_lifetime.as_ref().map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).unwrap());
    system.color_law = emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = emitter.custom_data.as_ref().map(|p| crate::particle_runtime::custom_data_law(p).unwrap());
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.prewarmed = true;
    system.born_total = 0;
    system.died_total = 0;
    system.full_total = 0;
    system.refused_total = 0;
    system.native_birth = None;
    Ok((system, edges))
}

#[derive(Default)]
struct Tally {
    calls: usize,
    newborn: usize,
    records: usize,
    commands: usize,
    frames: usize,
    fields: std::collections::BTreeMap<&'static str, (usize, usize)>,
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

    fn mismatched(&self) -> usize {
        self.fields.values().map(|(_, bad)| bad).sum()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    Product,
    /// One-rule arm: the owner matrix left at identity.
    IdentityOwner,
}

pub(super) fn event_state(emission: &EventEmission) -> Vec<u32> {
    let d = emission.distribution;
    let mut out = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
    out.extend(emission.random.words);
    out
}

fn run_parent(doc: &Value, parent: &Value, newborn_call: &str, arm: Arm, tally: &mut Tally) {
    let label = parent["label"].as_str().unwrap();
    let image = &parent["image"];
    let (mut system, edges) = parent_system(doc, &parent["source"]);
    // The children as the harness imaged them read into the same laws.
    let sub = &image["subEmitters"];
    let image_edges = sub["edges"].as_array().unwrap();
    assert_eq!(image_edges.len(), edges.len(), "{label}");
    assert_eq!(word(&sub["numEmitAccumulators"]) as usize, edges.len().min(2));
    for ((edge, child), ours) in image_edges.iter().zip(sub["children"].as_array().unwrap()).zip(&edges) {
        assert_eq!(edge["node"].as_str(), Some(ours.target.as_str()), "{label}: slot order");
        assert_eq!(image_law(edge, child, image_edges.len()), ours.law, "{label}: {}", ours.target);
    }
    assert_eq!(word(&image["maximum"]), system.emitter.max_particles, "{label}");
    assert_eq!(f(&image["duration"]), system.emitter.duration, "{label}");
    assert_eq!(image["looping"].as_bool(), Some(system.emitter.looping), "{label}");
    assert_eq!(word(&image["simulation"]), 0, "{label}: the receipt's parents are Local");
    // Runtime X is the reflection of source X.
    let owner_words = words(&image["owner"]);
    let source_owner: [f32; 16] = std::array::from_fn(|i| f32::from_bits(owner_words[i]));
    let runtime_owner: [f32; 16] = std::array::from_fn(|i|
        if (i % 4 == 0) ^ (i / 4 == 0) { -source_owner[i] } else { source_owner[i] });
    system.node_affine = match arm {
        Arm::Product => GlobalTransform::from(Mat4::from_cols_array(&runtime_owner)),
        Arm::IdentityOwner => GlobalTransform::IDENTITY,
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
        let mut events = BirthEvents::new(edges.clone());
        events.trace = Some(Vec::new());
        state.events = Some(events);
    }
    system.playback_head = f(&image["clock"]);
    system.previous_head = system.playback_head;
    let subcalls = parent["subcalls"].as_array().unwrap();
    let records = parent["records"].as_array().unwrap();
    let commands = parent["commands"].as_array().unwrap();
    for frame in parent["frames"].as_array().unwrap() {
        let at = format!("{label} frame {}", frame["frame"]);
        let mut state = system.native_birth.take().unwrap();
        let result = birth::run_incremental(&mut system, &mut state, f(&frame["remainingBits"]),
            f(&frame["stepBits"]), false, &ctx);
        let trace = std::mem::take(state.events.as_mut().unwrap().trace.as_mut().unwrap());
        let broken = state.events.as_ref().unwrap().broken.clone();
        system.native_birth = Some(state);
        tally.check("frameAccepted", result.is_ok() && broken.is_none(), &at);
        if result.is_err() || broken.is_some() {
            continue;
        }
        tally.frames += 1;
        // ---- calls of this frame
        let range = words(&frame["subcalls"]);
        let native_calls = &subcalls[range[0] as usize..range[1] as usize];
        tally.check("callCount", trace.len() == native_calls.len(), &at);
        for (offset, (call, native)) in trace.iter().zip(native_calls).enumerate() {
            let index = range[0] as usize + offset;
            let at = format!("{at} call {index}");
            tally.calls += 1;
            let newborn = native["lr"].as_str() == Some(newborn_call);
            tally.newborn += usize::from(newborn);
            tally.check("callKind", call.newborn == newborn, &at);
            // Native appends newborns at the next four-aligned index.
            let shift = if call.newborn { call.first.next_multiple_of(4) - call.first } else { 0 };
            tally.check("callRange", word(&native["start"]) as usize == call.start + shift
                && word(&native["end"]) as usize == call.end + shift, &at);
            tally.check("laneTimes", words(&native["dt4"]) == call.dt4.map(f32::to_bits), &at);
            tally.check("pendingTime", word(&native["stateZero"]) == call.owner.accumulated_time.to_bits(), &at);
            tally.check("emissionWord", word(&native["ownerSeed"]) == call.owner.emission_word, &at);
            let owner = words(&native["owner"]);
            tally.check("owner", (0..16).all(|i| same_value(call.owner.local_to_world[i], owner[i])), &at);
            tally.check("space", (word(&native["simulation"]) == 1) == call.owner.world_space, &at);
            tally.check("accumulators", word(&native["numAccumulators"]) as usize == edges.len().min(2), &at);
            let p = &native["particles"];
            let lanes = |key: &str| words(&p[key]);
            let (seed, age, inv) = (lanes("seed"), lanes("age"), lanes("inv"));
            let position = [lanes("px"), lanes("py"), lanes("pz")];
            let velocity = [(lanes("vx"), lanes("ax")), (lanes("vy"), lanes("ay")), (lanes("vz"), lanes("az"))];
            let carries = [lanes("carry0"), lanes("carry1")];
            // The native arrays hold the range's whole four-lane groups; the
            // lanes past the range end are storage the call does not read.
            let count_ok = call.particles.len() == call.end - call.start
                && seed.len() == (call.end - call.start).next_multiple_of(4);
            tally.check("particleCount", count_ok, &at);
            if count_ok {
                let mut scalars = true;
                let mut positions = true;
                let mut velocities = true;
                let mut carry_ok = true;
                for (local, (particle, carry)) in call.particles.iter().enumerate() {
                    scalars &= particle.seed == seed[local] && particle.age_percent.to_bits() == age[local]
                        && particle.inverse_lifetime.to_bits() == inv[local];
                    positions &= (0..3).all(|a| same_value(particle.position[a], position[a][local]));
                    velocities &= (0..3).all(|a| {
                        let (v, anim) = &velocity[a];
                        same_value(particle.velocity[a], (f32::from_bits(v[local]) + f32::from_bits(anim[local])).to_bits())
                    });
                    carry_ok &= carry[0].to_bits() == carries[0][local] && carry[1].to_bits() == carries[1][local];
                }
                tally.check("particleSeedAgeLifetime", scalars, &at);
                tally.check("particlePosition", positions, &at);
                tally.check("particleVelocity", velocities, &at);
                tally.check("carries", carry_ok, &at);
            }
            // ---- records of this call
            let first_record = word(&native["firstRecord"]) as usize;
            let last_record = subcalls.get(index + 1).map_or(records.len(), |c| word(&c["firstRecord"]) as usize);
            let native_records = &records[first_record..last_record];
            tally.check("recordCount", call.records.len() == native_records.len(), &at);
            for ((slot, particle, record), native) in call.records.iter().zip(native_records) {
                tally.records += 1;
                let at = format!("{at} record {slot}/{particle}");
                tally.check("recordSlotIndex", word(&native["slot"]) as usize == *slot
                    && word(&native["index"]) as usize == particle + shift, &at);
                let interval = record.interval;
                let times = vec![interval.previous.to_bits(), interval.current.to_bits(),
                    interval.previous_normalized.to_bits(), interval.current_normalized.to_bits(),
                    edges[*slot].law.duration().to_bits()];
                tally.check("recordWindow", words(&native["timesBits"]) == times, &at);
                tally.check("recordStateBefore", words(&native["stateBefore"]) == event_state(&record.before), &at);
                tally.check("recordStateAfter", words(&native["stateAfter"]) == event_state(&record.after), &at);
                let ids = words(&native["commandIds"]);
                match &record.commands {
                    None => tally.check("commandCount", ids.is_empty(), &at),
                    Some(pair) => {
                        tally.check("commandCount", ids.len() == pair.len(), &at);
                        for (command, id) in pair.iter().zip(&ids) {
                            tally.commands += 1;
                            let native = &commands[*id as usize];
                            let raw = hex(&native["rawHex"]);
                            let ours = command.to_bytes();
                            // The pointer word and the padding word after the
                            // inherited block are not fields.
                            let vectors = (0x08..0x20).step_by(4).all(|w| same_value(
                                f32::from_le_bytes(ours[w..w + 4].try_into().unwrap()),
                                u32::from_le_bytes(raw[w..w + 4].try_into().unwrap())));
                            tally.check("commandPositionVelocity", raw.len() == 0x78 && vectors, &at);
                            tally.check("commandInherited", raw.get(0x20..0x54) == Some(&ours[0x20..0x54]), &at);
                            tally.check("commandCountTimes", raw.get(0x58..0x78) == Some(&ours[0x58..0x78]), &at);
                            tally.check("commandEmission", hex(&native["emissionHex"]) == command.emission_bytes(), &at);
                        }
                    }
                }
            }
        }
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
        }
        let state = system.native_birth.as_ref().unwrap();
        tally.check("framePending", system.pending.to_bits() == word(&frame["stateZeroAfter"]), &at);
        tally.check("frameClock", system.playback_head.to_bits() == word(&frame["clockAfter"]), &at);
        let emission = words(&frame["emissionStateAfter"]);
        let d = state.emission.distribution;
        let mut ours = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
        ours.extend(state.emission.random.words);
        tally.check("frameEmissionState", ours == emission, &at);
    }
}

#[test]
#[ignore = "MOLY_SUBEMITTER_PARENT_RECEIPT and MOLY_SUBEMITTER_PARENT_EFFECTS must identify the current native parent-event receipt and the exported 008 effects"]
fn product_parent_birth_events_match_current_native() {
    let receipt = read("MOLY_SUBEMITTER_PARENT_RECEIPT");
    assert_eq!(receipt["librarySha256"], SOURCE_SHA256);
    let doc = read("MOLY_SUBEMITTER_PARENT_EFFECTS");
    let parents = receipt["parentEvents"].as_array().unwrap();
    // The newborn sub-emitter call.
    let newborn_call = call_site(&receipt, "newbornCall");
    let run = |arm: Arm| {
        let mut tally = Tally::default();
        for parent in parents {
            run_parent(&doc, parent, newborn_call, arm, &mut tally);
        }
        tally
    };
    let tally = run(Arm::Product);
    println!("product parent birth events: {} frames, {} calls ({} newborn), {} records, {} commands, {} mismatched fields",
        tally.frames, tally.calls, tally.newborn, tally.records, tally.commands, tally.mismatched());
    for (field, (checked, bad)) in &tally.fields {
        println!("  {field}: {checked} checked, {bad} mismatched");
    }
    assert!(tally.first.is_empty(), "first mismatches: {:#?}", tally.first);
    assert_eq!((tally.calls, tally.newborn, tally.records, tally.commands), (1426, 296, 13285, 800));
    let owner = run(Arm::IdentityOwner);
    println!("arm identityOwner: {} mismatched fields ({:?})", owner.mismatched(),
        owner.fields.get("commandPositionVelocity"));
    assert!(owner.fields.get("commandPositionVelocity").is_some_and(|(_, bad)| *bad > 0));
}
