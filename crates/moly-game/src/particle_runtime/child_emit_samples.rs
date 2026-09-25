//! Child commands through the product's child Emit against the engine's own
//! child Emit rows (the ground-strike targets' chained and synthetic commands,
//! the target variants, the gravity rows, the death commands and the death
//! commands that carry an inherited block). Each row
//! holds the command bytes, the target's owner words, the update inputs, the
//! pool and both random streams before the command, and the pool, both
//! streams, the start matrix and the command velocity in the target's space
//! after it. The target is the exported system block (or the receipt's
//! override of it); the owner words, streams and pool are the harness's
//! inputs, not claims about scene placement or client entropy.
//!
//! Compared per row: the count and, per lane, position, velocity and animated
//! velocity (X is the reflected axis; a zero there compares by value, and so
//! does a zero animated word on every axis: an all-zero orbital block adds a
//! signed zero the product does not compose), rotation, birth size (every
//! stored axis), colour, seed, age, inverse lifetime and every CustomData
//! channel; the
//! Initial stream after; the start matrix and the command velocity in the
//! target's space. Angular speed is not stored by the product (it is rebuilt
//! every update), so it is not compared. The size the renderer evaluates at
//! the final age is compared with the engine's current size where the size
//! curve is constant or baked; a keyed size curve goes through this tree's
//! Hermite evaluator, which is not the engine's cached cubic, so those words
//! are counted with their distance instead. Only the SizeModule writes the
//! engine's current size: a target without one leaves it unwritten (zero in
//! every row), and the renderer reads the birth size, so it is not compared.
//!
//! A custom stream the target leaves disabled has no storage in the engine
//! (its rows carry no array for it); it is neither loaded nor compared.
//!
//! Where this tree's Shape law refuses the target's shape, the engine's
//! stored Shape output of each group (after its own store with the start
//! matrix) is fed in its place and the Shape stream is not compared; the
//! injected groups are counted. Rows whose block randomizes the rotation
//! direction are outside the source admission (it refuses any value but 0)
//! and are counted, not compared.
use super::child::{apply_command, arms, ChildCommand, ChildShape, ChildUpdate, Refused, SourceShape};
use moly_law::particle::death_event::size_3d;
use super::*;
use moly_law::particle::child_emit::ChildOwner;
use moly_law::particle::seed_owner::ModuleRandom;
use moly_law::particle::shape_birth::{ShapeBirthLaw, ShapeSample};
use moly_law::particle::schema::Effects;
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

fn word(value: &Value) -> u32 {
    value.as_u64().expect("native word") as u32
}
fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("native array").iter().map(word).collect()
}
fn floats<const N: usize>(value: &Value) -> [f32; N] {
    let w = words(value);
    assert_eq!(w.len(), N, "native word count");
    std::array::from_fn(|i| f32::from_bits(w[i]))
}
fn read(key: &str) -> Value {
    let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} is not set"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn hex(value: &Value) -> Vec<u8> {
    let text = value.as_str().expect("hex");
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}
fn module_random(value: &Value) -> ModuleRandom {
    let flat = words(value);
    assert_eq!(flat.len(), 16);
    ModuleRandom { words: std::array::from_fn(|w| std::array::from_fn(|lane| flat[w * 4 + lane])) }
}
fn flat_random(random: &ModuleRandom) -> Vec<u32> {
    random.words.iter().flatten().copied().collect()
}

/// The engine's stored Shape output of each group of one command, in order.
struct NativeShape {
    groups: Vec<[ShapeSample; 4]>,
    next: usize,
}
impl ChildShape for NativeShape {
    fn group(&mut self, _start: &[f32; 16]) -> Result<[ShapeSample; 4], Refused> {
        let Some(group) = self.groups.get(self.next).copied() else {
            return Err(Refused::Unsupported("native row has no further stored Shape group"));
        };
        self.next += 1;
        Ok(group)
    }
}
fn native_shape(row: &Value) -> NativeShape {
    let groups = row["shapeEvents"].as_array().unwrap().iter()
        .filter(|event| event["phase"] == "store")
        .map(|event| {
            let storage = &event["storage"];
            let axis = |key: &str, a: usize, lane: usize| f32::from_bits(word(&storage[key][a][lane]));
            std::array::from_fn(|lane| ShapeSample {
                position: std::array::from_fn(|a| axis("position", a, lane)),
                direction: std::array::from_fn(|a| axis("direction", a, lane)),
            })
        })
        .collect();
    NativeShape { groups, next: 0 }
}

/// The target block: the receipt's override, else the exported block.
fn target_block<'a>(seq: &'a Value, corpus: &'a Value) -> &'a Value {
    if let Some(block) = seq.get("systemOverride") {
        return block;
    }
    let source = &seq["source"];
    let effect = source["effect"].as_str().unwrap();
    let particles = corpus["effects"][effect]["particles"].as_array().unwrap();
    let found: Vec<_> = particles.iter().filter(|p| p["node"] == seq["node"]).collect();
    assert_eq!(found.len(), 1, "one exported block for {}", seq["node"]);
    assert_eq!(found[0]["systemPathId"], source["systemPathId"]);
    &found[0]["system"]
}

/// The target Runtime from its block, with the product's module laws.
fn target_runtime(node: &str, block: &Value, simulation: u64) -> Runtime {
    let selected = json!({"effects": {"target": {"particles": [{"node": node, "system": block}]}}});
    let mut decoded = Effects::from_json_str(&serde_json::to_vec(&selected).unwrap()).expect("decode target");
    assert_eq!(decoded.emitters.len(), 1);
    let mut emitter = decoded.emitters.remove(0);
    emitter.simulation_space = match simulation {
        0 => SimulationSpace::Local,
        1 => SimulationSpace::World,
        2 => SimulationSpace::Custom,
        other => panic!("simulation space {other}"),
    };
    let mut system = test_support::runtime();
    system.node = node.to_owned();
    system.kind = EffectKind::Site;
    system.rol = emitter.rotation_over_lifetime.as_ref().map(|p| {
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve).unwrap()
    });
    system.size_law = emitter.size_over_lifetime.as_ref().map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).unwrap());
    system.custom_law = emitter.custom_data.as_ref().map(|p| moly_law::particle::custom_data::CustomData::from_params(p).unwrap());
    system.color_law = emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.born_total = 0;
    system
}

/// The native pool as the product holds it (X reflected).
fn load_pool(system: &mut Runtime, pool: &Value) {
    system.pool.clear();
    system.side.clear();
    let count = pool["count"].as_u64().unwrap() as usize;
    let three = size_3d(&system.emitter);
    let lane = |key: &str, i: usize| f32::from_bits(word(&pool[key][i]));
    for i in 0..count {
        let position = [-lane("px", i), lane("py", i), lane("pz", i)];
        let velocity = [-lane("vx", i), lane("vy", i), lane("vz", i)];
        let inverse = lane("inv", i);
        system.pool.push(Particle {
            position,
            velocity,
            start_lifetime: 1.0 / inverse,
            inverse_lifetime: inverse,
            age_percent: lane("age", i),
        });
        let colour = word(&pool["color"][i]).to_le_bytes();
        system.side.push(Side {
            rand: 0.0,
            seed: word(&pool["seed"][i]),
            rot: [lane("rx", i), lane("ry", i), lane("rz", i)],
            size: if three { [lane("sx", i), lane("sy", i), lane("sz", i)] } else { [lane("sx", i); 3] },
            gravity: 0.0,
            colour: moly_law::particle::gradient::rgba8_to_float(colour),
            total_velocity: velocity,
            // A stream without storage has no array in the row; nothing reads it.
            custom_data: std::array::from_fn(|s| std::array::from_fn(|c| {
                let key = format!("custom{s}{c}");
                if stored(pool, &key, count) { lane(&key, i) } else { 0.0 }
            })),
            emit_carry: [0.0; 2],
            animated: [-lane("ax", i), lane("ay", i), lane("az", i)],
            current_size: 0.0,
            axis: [0.0, 0.0, 1.0],
        });
    }
}

/// Whether the row holds a stored array for `key` over the pool's `count`
/// particles (a disabled custom stream has none).
fn stored(pool: &Value, key: &str, count: usize) -> bool {
    pool[key].as_array().is_some_and(|lanes| lanes.len() >= count)
}

/// X compares with the reflection; a zero on X compares by value.
fn same_reflected(ours: f32, native: u32) -> bool {
    let native = -f32::from_bits(native);
    if native == 0.0 { ours == 0.0 } else { ours.to_bits() == native.to_bits() }
}

/// Whether the target's size curve leaves the optimised polynomial for the
/// keyed evaluation. Its size words are then counted with their distance,
/// not compared; the polynomial and constant forms are compared bit for bit.
fn size_curve_generic(system: &Runtime) -> bool {
    use moly_law::particle::curve::CurveSampler;
    system.emitter.size_over_lifetime.as_ref().is_some_and(|p| {
        [Some(&p.curve), p.y.as_ref(), p.z.as_ref()].into_iter().flatten().any(|c| !CurveSampler::engine_optimized(c))
    })
}

/// The size words of one row: compared bit for bit, or, under the keyed
/// evaluator, counted with their largest distance in f32 steps.
#[derive(Default, Debug)]
struct SizeWords {
    exact: usize,
    generic: usize,
    generic_differ: usize,
    generic_max_steps: u32,
}

/// The fields of the pool after the command that differ from native.
fn pool_mismatches(system: &Runtime, pool: &Value, size: &mut SizeWords) -> Vec<String> {
    let count = pool["count"].as_u64().unwrap() as usize;
    if system.pool.len() != count || system.side.len() != count {
        return vec![format!("count {} vs native {count}", system.pool.len())];
    }
    let mut bad = Vec::new();
    let lane = |key: &str, i: usize| word(&pool[key][i]);
    let generic = size_curve_generic(system);
    let three = size_3d(&system.emitter);
    let axes = if three { 3 } else { 1 };
    for i in 0..count {
        let p = &system.pool[i];
        let s = &system.side[i];
        let current = motion::size_at_age_percent(system, s, p.age_percent);
        let mut size_bad = Vec::new();
        for (axis, key) in ["f300", "f320", "f340"].iter().enumerate().take(axes) {
            if system.size_law.is_none() {
                break;
            }
            let native_size = lane(key, i);
            let current = current[axis];
            if generic {
                size.generic += 1;
                if current.to_bits() != native_size {
                    size.generic_differ += 1;
                    let steps = if current.is_sign_negative() == f32::from_bits(native_size).is_sign_negative() {
                        current.to_bits().abs_diff(native_size)
                    } else {
                        u32::MAX
                    };
                    size.generic_max_steps = size.generic_max_steps.max(steps);
                }
            } else {
                size.exact += 1;
                if current.to_bits() != native_size {
                    size_bad.push(format!("{key}[{i}] age {:08x} ours {:08x} native {native_size:08x}",
                        p.age_percent.to_bits(), current.to_bits()));
                }
            }
        }
        let mut check = |name: &str, ok: bool| if !ok { bad.push(format!("{name}[{i}]")) };
        check("px", same_reflected(p.position[0], lane("px", i)));
        check("py", p.position[1].to_bits() == lane("py", i));
        check("pz", p.position[2].to_bits() == lane("pz", i));
        check("vx", same_reflected(p.velocity[0], lane("vx", i)));
        check("vy", p.velocity[1].to_bits() == lane("vy", i));
        check("vz", p.velocity[2].to_bits() == lane("vz", i));
        // The animated velocity; a zero compares by value.
        let animated = |ours: f32, native: u32| {
            let native = f32::from_bits(native);
            if native == 0.0 { ours == 0.0 } else { ours.to_bits() == native.to_bits() }
        };
        check("ax", animated(-s.animated[0], lane("ax", i)));
        check("ay", animated(s.animated[1], lane("ay", i)));
        check("az", animated(s.animated[2], lane("az", i)));
        for (a, key) in ["rx", "ry", "rz"].iter().enumerate() {
            check(key, s.rot[a].to_bits() == lane(key, i));
        }
        check("sx", s.size[0].to_bits() == lane("sx", i));
        if three {
            check("sy", s.size[1].to_bits() == lane("sy", i));
            check("sz", s.size[2].to_bits() == lane("sz", i));
        }
        let colour = u32::from_le_bytes(moly_law::particle::gradient::quantize_rgba8(s.colour));
        check("color", colour == lane("color", i));
        check("seed", s.seed == lane("seed", i));
        check("age", p.age_percent.to_bits() == lane("age", i));
        check("inv", p.inverse_lifetime.to_bits() == lane("inv", i));
        for stream in 0..2 {
            for channel in 0..4 {
                let key = format!("custom{stream}{channel}");
                if stored(pool, &key, count) {
                    check(&key, s.custom_data[stream][channel].to_bits() == lane(&key, i));
                }
            }
        }
        bad.extend(size_bad);
    }
    bad
}

#[derive(Default, Debug)]
struct Tally {
    rows: usize,
    compared: usize,
    emitted: usize,
    catch_up_rows: usize,
    injected_groups: usize,
    kernel_groups: usize,
    direction_randomized: usize,
    mismatched: Vec<String>,
    families: std::collections::BTreeMap<String, usize>,
    size: SizeWords,
    /// Rows whose command carries an inherited size other than the neutral
    /// one and bore particles.
    inherited_size_births: usize,
    /// Rows whose command carries an inherited word other than the size,
    /// refused by that bit's name, per name.
    refused_by_name: std::collections::BTreeMap<&'static str, usize>,
}

/// One row through apply_command; the fields that differ from native.
fn run_row(seq: &Value, row: &Value, block: &Value, tally: &mut Tally, record: bool) -> Vec<String> {
    let image = &seq["image"];
    let node = seq["node"].as_str().unwrap();
    let mut system = target_runtime(node, block, image["simulation"].as_u64().unwrap());
    assert_eq!(system.emitter.max_particles as u64, image["maximum"].as_u64().unwrap());
    assert_eq!(system.emitter.duration.to_bits(), word(&image["duration"]));
    assert_eq!(system.emitter.simulation_speed.to_bits(), word(&image["simulationSpeed"]));
    let owner = ChildOwner {
        local_to_world: floats(&image["owner"]),
        world_to_local: floats(&image["inverse"]),
        local_rotation: floats(&image["st114"]),
        emitter_scale: floats(&image["scale"]),
        shape_scale: floats(&image["shapeScale"]),
    };
    assert!(image["unscaled"] == false, "unscaled-time rows are not in these receipts");
    let before = &row["before"];
    assert!(before.is_object(), "every row carries its own pool before");
    load_pool(&mut system, &before["pool"]);
    let mut initial = module_random(&before["rng"]["initial"]);
    let shape_before = module_random(&before["rng"]["shape"]);
    let command = ChildCommand::from_native_bytes(&hex(&row["command"]["rawHex"]), &hex(&row["command"]["emissionHex"]))
        .unwrap();
    // The gravity the harness's physics service returned.
    let gravity: [f32; 3] = floats(&image["gravity"]);
    let update = ChildUpdate {
        flags: word(&row["updateFlags"]),
        frame_dt: f32::from_bits(word(&row["frameDtBits"])),
        world_playing: row["worldPlaying"].as_bool().unwrap(),
        gravity,
    };
    let shape_params = system.emitter.shape.clone();
    let law = shape_params.as_ref().map(ShapeBirthLaw::from_params);
    let mut kernel = None;
    let mut injected = None;
    let shape: Option<&mut dyn ChildShape> = match law {
        None => None,
        Some(Ok(law)) => {
            kernel = Some(SourceShape { law, stream: shape_before, shape_scale: owner.shape_scale,
                uses_axis_of_rotation: false });
            kernel.as_mut().map(|k| k as &mut dyn ChildShape)
        }
        Some(Err(moly_law::particle::shape_birth::Refused::UnsupportedSourceShape)) => {
            injected = Some(native_shape(row));
            injected.as_mut().map(|k| k as &mut dyn ChildShape)
        }
        Some(Err(other)) => panic!("{node}: Shape law refused {other:?}"),
    };
    let applied = apply_command(&mut system, &owner, &mut initial, shape, &command, update);
    let native_after = &row["after"];
    let mut bad = match &applied {
        Ok(_) if record => pool_mismatches(&system, &native_after["pool"], &mut tally.size),
        Ok(_) => pool_mismatches(&system, &native_after["pool"], &mut SizeWords::default()),
        Err(refused) => vec![format!("refused {refused:?}")],
    };
    if flat_random(&initial) != words(&native_after["rng"]["initial"]) {
        bad.push("rngInitial".into());
    }
    if let Some(kernel) = &kernel {
        if flat_random(&kernel.stream) != words(&native_after["rng"]["shape"]) {
            bad.push("rngShape".into());
        }
    }
    if let Ok(applied) = &applied {
        if let Some(frame) = &applied.start {
            let enter = row["shapeEvents"].as_array().unwrap().iter().find(|e| e["phase"] == "shapeEnter");
            if let Some(enter) = enter {
                if frame.matrix.map(f32::to_bits).to_vec() != words(&enter["startMatrix"]) {
                    bad.push("startMatrix".into());
                }
            }
            // The StartVelocity call is the one call that records the command
            // velocity in the target's space.
            let call = row["calls"].as_array().unwrap().iter().find(|c| c.get("emitterVelocity").is_some());
            if let Some(call) = call {
                let native: Vec<u32> = call["emitterVelocity"].as_array().unwrap().iter()
                    .map(|v| (v.as_f64().unwrap() as f32).to_bits()).collect();
                if frame.emitter_velocity.map(f32::to_bits).to_vec() != native {
                    bad.push("emitterVelocity".into());
                }
            }
        }
    }
    if record {
        tally.compared += 1;
        if let Ok(applied) = &applied {
            tally.emitted += usize::from(applied.start.is_some());
            tally.catch_up_rows += usize::from(applied.catch_up_steps > 0);
            let inherited = moly_law::particle::inherit::ChildInherit::from_words(&command.inherited_words);
            tally.inherited_size_births += usize::from(applied.born > 0 && inherited.is_ok_and(|i| !i.is_neutral()));
        }
        tally.injected_groups += injected.as_ref().map_or(0, |s| s.next);
        if let (Some(_), Ok(applied)) = (&kernel, &applied) {
            tally.kernel_groups += applied.born.div_ceil(4);
        }
    }
    bad
}

const ARMS: [&str; 10] = ["noLookRotation", "noInverse", "backtrackWorldVelocity", "noEmitterScale", "noUpperGate",
    "birthDtIsCommandDt", "paddingKillKeepsAccepted", "noStepRaise", "noGravity", "gravityWorld"];
/// The inherited size ignored, or added to the target's own start size.
const INHERIT_ARMS: [&str; 2] = ["ignoreInheritedSize", "addInheritedSize"];
/// Velocity or ClampVelocity left out, ClampVelocity before Velocity (the
/// wrong module order), the linear matrix product grouped as
/// `(x*c0 + y*c1) + z*c2`, and ClampVelocity's k from the command dt instead
/// of the lane's own dt.
const MODULE_ARMS: [&str; 5] = ["noVelocity", "noClamp", "clampBeforeVelocity", "velocityGroupedAssociation",
    "clampCommandDt"];

/// The refusal the child side names for a command of `count` particles whose
/// block carries an inherited word other than the size; None for a size-only
/// or neutral block, and for a count of zero (nothing is read).
fn expected_refusal(row: &Value) -> Option<&'static str> {
    let command = ChildCommand::from_native_bytes(&hex(&row["command"]["rawHex"]), &hex(&row["command"]["emissionHex"]))
        .unwrap();
    if command.count == 0 {
        return None;
    }
    match moly_law::particle::inherit::ChildInherit::from_words(&command.inherited_words) {
        Ok(_) => None,
        Err(moly_law::particle::inherit::Refused::Block("color")) => Some("inherited colour block"),
        Err(moly_law::particle::inherit::Refused::Block("rotation")) => Some("inherited rotation block"),
        Err(moly_law::particle::inherit::Refused::Block("lifetime")) => Some("inherited lifetime block"),
        Err(moly_law::particle::inherit::Refused::Block("duration")) => Some("inherited duration block"),
        Err(other) => panic!("child block refusal {other:?}"),
    }
}

/// Every row of the receipts named by `keys` through apply_command, and
/// every arm of `arm_names` on each compared row.
fn replay(corpus: &Value, keys: &[&str], arm_names: &[&'static str])
    -> (Tally, std::collections::BTreeMap<&'static str, usize>) {
    let mut arm_red = std::collections::BTreeMap::<&str, usize>::new();
    let mut tally = Tally::default();
    for &key in keys {
        let receipt = read(key);
        assert_eq!(receipt["librarySha256"], SOURCE_SHA256);
        for group in ["childChained", "childSynthetic"] {
            let Some(seqs) = receipt.get(group).and_then(Value::as_array) else { continue };
            for seq in seqs {
                let block = target_block(seq, corpus);
                let randomized = block["start"]["randomizeRotationDirection"].as_f64() != Some(0.0);
                for (index, row) in seq["rows"].as_array().unwrap().iter().enumerate() {
                    tally.rows += 1;
                    if randomized {
                        tally.direction_randomized += 1;
                        continue;
                    }
                    arms::set(None);
                    if let Some(name) = expected_refusal(row) {
                        let mut ignored = Tally::default();
                        let bad = run_row(seq, row, block, &mut ignored, false);
                        if bad.first().map(String::as_str) == Some(format!("refused Unsupported({name:?})").as_str()) {
                            *tally.refused_by_name.entry(name).or_default() += 1;
                        } else {
                            tally.mismatched.push(format!("{key} {} row {index}: expected the refusal {name:?}, got {:?}",
                                seq["label"], &bad[..bad.len().min(6)]));
                        }
                        continue;
                    }
                    let bad = run_row(seq, row, block, &mut tally, true);
                    for field in &bad {
                        let family: String = field.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                        *tally.families.entry(family).or_default() += 1;
                    }
                    if !bad.is_empty() {
                        tally.mismatched.push(format!("{key} {} row {index}: {:?}", seq["label"],
                            &bad[..bad.len().min(6)]));
                    }
                    for &arm in arm_names {
                        arms::set(Some(arm));
                        let red = !run_row(seq, row, block, &mut tally, false).is_empty();
                        *arm_red.entry(arm).or_default() += usize::from(red);
                    }
                    arms::set(None);
                }
            }
        }
    }
    (tally, arm_red)
}

fn report(tally: &Tally, arm_red: &std::collections::BTreeMap<&str, usize>) {
    eprintln!("child emit replay: rows {} compared {} emitted {} catch-up {} injected Shape groups {} \
        kernel Shape groups {} direction-randomized (not admitted) {} mismatched {}; arms {:?}",
        tally.rows, tally.compared, tally.emitted, tally.catch_up_rows, tally.injected_groups,
        tally.kernel_groups, tally.direction_randomized, tally.mismatched.len(), arm_red);
    eprintln!("mismatched field families: {:?}; size words {:?}; rows with an inherited size that bore {}; refused by name {:?}",
        tally.families, tally.size, tally.inherited_size_births, tally.refused_by_name);
    for line in tally.mismatched.iter().take(12) {
        eprintln!("  {line}");
    }
    for line in tally.mismatched.iter().filter(|l| l.contains("\"") && l.split('"').skip(3).step_by(2)
        .any(|field| !field.starts_with("f300"))).take(12) {
        eprintln!("  other: {line}");
    }
}

#[test]
#[ignore = "needs MOLY_CHILD_EMIT_RECEIPT, MOLY_CHILD_EMIT_EXTRA, MOLY_CHILD_EMIT_GRAVITY and MOLY_CHILD_EMIT_EFFECTS"]
fn product_child_emit_matches_current_native_rows() {
    let corpus = read("MOLY_CHILD_EMIT_EFFECTS");
    let (tally, arm_red) = replay(&corpus, &["MOLY_CHILD_EMIT_RECEIPT", "MOLY_CHILD_EMIT_EXTRA", "MOLY_CHILD_EMIT_GRAVITY"],
        &ARMS);
    report(&tally, &arm_red);
    assert!(tally.mismatched.is_empty(), "{} rows differ from native", tally.mismatched.len());
    assert!(tally.compared > 0 && tally.emitted > 0 && tally.catch_up_rows > 0 && tally.size.exact > 0);
    for arm in ARMS {
        assert!(arm_red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
    }
}

/// Death-event commands through the product's child Emit: the commands the
/// native death-edge parent runs recorded (count 0, then the child's first
/// burst count; dt 0, both normalized times 1, rate count 0, a zero
/// distribution, the parent's pending time as catch-up), executed by the
/// engine's child Emit into the raindrop ring target (no Shape, World, one
/// custom stream disabled) and the ground-strike core target (Sphere, Local,
/// a two-constant count), with the product's update inputs (flags 0, the
/// world playing, gravity (0, -9.81, 0)). Every row carries its target block.
/// A zero-dt command leaves the arms that act through the birth dt, the
/// catch-up or gravity without effect; the two that act through the start
/// frame must differ from native.
#[test]
#[ignore = "needs MOLY_CHILD_EMIT_DEATH (the native death-command child rows)"]
fn product_child_emit_matches_native_death_command_rows() {
    let (tally, arm_red) = replay(&Value::Null, &["MOLY_CHILD_EMIT_DEATH"], &ARMS);
    report(&tally, &arm_red);
    assert!(tally.mismatched.is_empty(), "{} rows differ from native", tally.mismatched.len());
    assert!(tally.compared > 0 && tally.emitted > 0 && tally.size.exact + tally.size.generic > 0);
    for arm in ["noLookRotation", "noInverse"] {
        assert!(arm_red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
    }
}

/// Death commands whose edge inherits: the commands of the native parent rows
/// (RecordParticleDeath and KillParticle with the inherited block the parent's
/// size module wrote: the 012 bubble's three death edges and random parents
/// with each inherit bit set) executed by the engine's child Emit into the
/// bubble's dust target (Sphere, Local, 3D start size, no SizeModule) and the
/// raindrop ring (one size axis, SizeModule, World, no Shape), with the
/// product's update inputs (flags 0, the world playing, gravity (0, -9.81,
/// 0)). A command whose block carries a word other than the size refuses by
/// that bit's name; every other row is compared. The two inherit arms and the
/// two start-frame arms must differ from native.
#[test]
#[ignore = "needs MOLY_CHILD_EMIT_INHERIT (the native inherited-command child rows)"]
fn product_child_emit_matches_native_inherit_rows() {
    let arm_names: Vec<&'static str> = ARMS.iter().chain(INHERIT_ARMS.iter()).copied().collect();
    let (tally, arm_red) = replay(&Value::Null, &["MOLY_CHILD_EMIT_INHERIT"], &arm_names);
    report(&tally, &arm_red);
    assert!(tally.mismatched.is_empty(), "{} rows differ from native", tally.mismatched.len());
    assert!(tally.compared > 0 && tally.emitted > 0 && tally.inherited_size_births > 0);
    assert!(tally.size.exact + tally.size.generic > 0 && !tally.refused_by_name.is_empty());
    for arm in ["noLookRotation", "noInverse", "ignoreInheritedSize", "addInheritedSize"] {
        assert!(arm_red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
    }
}

/// Child commands into the targets that carry VelocityOverLifetime and
/// ClampVelocity: the death commands of the death-edge parent runs into the
/// raindrops' sub_dust_01 (World, Cone, ClampVelocity with a zero limit,
/// Rotation, Size), synthetic commands (per frame, and catch-up with every
/// flag) into pt_trailr (Local and World, Sphere, two-constant linear
/// velocity) and the meteor ground's pt (Local, Cone, Velocity then
/// ClampVelocity, gravity), a World pt_trailr with a nonzero linear velocity
/// under rotated and scaled owners, and a pt whose clamp branch is taken at
/// newborn speeds. The engine's ClampVelocity called the device libm's powf;
/// the product's limit law calls the host's, and the pairs where the two
/// differ are counted. Every module arm must differ from native.
#[test]
#[ignore = "needs MOLY_CHILD_EMIT_MODULES (the native Velocity and ClampVelocity child rows)"]
fn product_child_emit_matches_native_module_rows() {
    let receipt = read("MOLY_CHILD_EMIT_MODULES");
    let (mut pairs, mut host_differs) = (0_usize, 0_usize);
    for seq in receipt["childSynthetic"].as_array().unwrap() {
        for call in seq["powfCalls"].as_array().unwrap() {
            let [x, y, r] = [0, 1, 2].map(|k| word(&call[k]));
            pairs += 1;
            host_differs += usize::from(f32::from_bits(x).powf(f32::from_bits(y)).to_bits() != r);
        }
    }
    let arm_names: Vec<&'static str> = ARMS.iter().chain(MODULE_ARMS.iter()).copied().collect();
    let (tally, arm_red) = replay(&Value::Null, &["MOLY_CHILD_EMIT_MODULES"], &arm_names);
    report(&tally, &arm_red);
    eprintln!("powf calls {pairs}, host powf differs from the device's on {host_differs}");
    assert!(pairs > 0);
    assert!(tally.mismatched.is_empty(), "{} rows differ from native", tally.mismatched.len());
    assert!(tally.compared > 0 && tally.emitted > 0 && tally.catch_up_rows > 0);
    for arm in MODULE_ARMS {
        assert!(arm_red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
    }
}
