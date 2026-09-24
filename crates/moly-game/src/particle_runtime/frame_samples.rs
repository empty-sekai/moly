//! The installed per-frame update against native Update1b rows: the frame
//! head (emitter velocity, speed scale, minimum-step skip, pending time), the
//! incremental slices and the emitter-motion placement of World births, read
//! through the product frame entry. The native harness drove one exported
//! start block with Shape, Velocity, Colour and CustomData off; the probe RNG
//! words it wrote replace the installed streams, never a claim about client
//! entropy.
use super::*;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

fn f(bits: &Value) -> f32 {
    f32::from_bits(bits.as_u64().expect("native f32 bits") as u32)
}

fn word(value: &Value) -> u32 {
    value.as_u64().expect("native word") as u32
}

fn read(key: &str) -> Value {
    let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} is not set"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The harness emitter of one case: the recorded start block, the case's
/// rates, space, speed, capacity and clock, every other module off.
fn harness_system(start: &Value, config: &Value) -> Runtime {
    let space = if config["space"].as_u64() == Some(0) { "Local" } else { "World" };
    let system = json!({
        "duration": config["duration"], "looping": config["looping"], "prewarm": false,
        "playOnAwake": true, "simulationSpeed": 1.0, "simulationSpace": space,
        // The installer draws an owner; the probe words replace its streams.
        "randomSeed": 0, "autoRandomSeed": true,
        "emitterVelocityMode": config["velocity_mode"],
        "startDelay": {"mode": "constant", "value": 0.0},
        "ringBufferMode": 0, "ringBufferLoopRange": [0.0, 1.0],
        "maxParticles": config["maximum"], "start": start.clone(),
        "emission": {"rateOverTime": {"mode": "constant", "value": config["rate_time"]},
            "rateOverDistance": {"mode": "constant", "value": config["rate_distance"]}, "bursts": []},
        "shapeEnabled": false,
        "sourceModules": {"version": 1, "enabled": ["EmissionModule", "InitialModule"], "unsupported": []},
    });
    let document = json!({"effects": {"harness": {"particles": [{"node": "root/harness", "system": system}]}}});
    let mut decoded = moly_law::particle::schema::Effects::from_json_str(
        &serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(decoded.emitters.len(), 1);
    let mut emitter = decoded.emitters.remove(0);
    emitter.simulation_speed = match &config["speed"] {
        Value::String(text) if text == "nan" => f32::NAN,
        value => value.as_f64().unwrap() as f32,
    };
    let mut runtime = test_support::runtime();
    runtime.node = emitter.node.clone();
    runtime.effect = emitter.effect.clone();
    runtime.kind = EffectKind::Sky;
    runtime.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier);
    runtime.custom_law = None;
    runtime.emitter = emitter;
    runtime.pool.clear();
    runtime.side.clear();
    runtime.playback_head = 0.0;
    runtime.previous_head = 0.0;
    runtime.prewarmed = true;
    runtime.born_total = 0;
    runtime.died_total = 0;
    runtime.full_total = 0;
    runtime.refused_total = 0;
    runtime.native_birth = None;
    runtime
}

fn install(system: &mut Runtime, seeds: &Value) {
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(matches!(install_native_birth(system, &mut manager, &SourceRoute::Ordinary, None).unwrap(),
        BirthPath::Native));
    let state = system.native_birth.as_mut().unwrap();
    state.owner = None;
    let initial = seeds["initialWords"].as_array().unwrap();
    assert_eq!(initial.len(), 16);
    state.initial = ModuleRandom {
        words: std::array::from_fn(|w| std::array::from_fn(|lane| word(&initial[w * 4 + lane]))),
    };
    let emission = seeds["emissionWords"].as_array().unwrap();
    state.emission.random = ScalarRandom { words: std::array::from_fn(|i| word(&emission[i])) };
}

/// Runtime X is the reflection of source X. A zero reflects to the other
/// signed zero, so X zeros compare by value and everything else by bits.
fn same_x(runtime: f32, native: &Value) -> bool {
    let native = f(native);
    if native == 0.0 { runtime == 0.0 } else { (-runtime).to_bits() == native.to_bits() }
}

fn same(runtime: f32, native: &Value) -> bool {
    runtime.to_bits() == f(native).to_bits()
}

#[derive(Default)]
struct Tally {
    frames: usize,
    mismatched_frames: usize,
    fields: std::collections::BTreeMap<&'static str, (usize, usize)>,
    first: Vec<String>,
}

impl Tally {
    fn check(&mut self, field: &'static str, ok: bool, frame_ok: &mut bool, label: &str) {
        let entry = self.fields.entry(field).or_default();
        entry.0 += 1;
        if !ok {
            entry.1 += 1;
            *frame_ok = false;
            if self.first.len() < 12 {
                self.first.push(format!("{label} {field}"));
            }
        }
    }
}

/// Compare the product state after one frame with the native row.
/// `frame_state` false skips the fields only the per-frame driver keeps.
fn compare(system: &Runtime, row: &Value, frame_state: bool, tally: &mut Tally, label: &str) {
    let state = system.native_birth.as_ref().unwrap();
    let after = &row["after"];
    let mut ok = true;
    if frame_state {
        tally.check("pending", same(state.frame.pending, &after["pendingBits"]), &mut ok, label);
        let prev = &after["prevBits"];
        let velocity = &row["velocityAfterUpdate1bBits"];
        tally.check("previousPosition", same_x(state.frame.previous_position[0], &prev[0])
            && (1..3).all(|a| same(state.frame.previous_position[a], &prev[a])), &mut ok, label);
        tally.check("emitterVelocity", same_x(state.frame.velocity[0], &velocity[0])
            && (1..3).all(|a| same(state.frame.velocity[a], &velocity[a])), &mut ok, label);
    }
    tally.check("clock", same(system.playback_head, &after["clockBits"]), &mut ok, label);
    let distribution = state.emission.distribution;
    let carry = &after["emission"]["f"];
    tally.check("emissionCarry", same(distribution.spacing, &carry[0]) && same(distribution.offset, &carry[1])
        && same(distribution.burst_fraction, &carry[2]), &mut ok, label);
    let rng = &after["emission"]["rng"];
    tally.check("emissionRng", (0..4).all(|i| state.emission.random.words[i] == word(&rng[i])), &mut ok, label);
    let initial = &after["initialRng"];
    tally.check("initialRng", (0..16).all(|i| state.initial.words[i / 4][i % 4] == word(&initial[i])),
        &mut ok, label);
    let particles = &row["particles"];
    let count = particles["count"].as_u64().unwrap() as usize;
    let count_ok = system.pool.len() == count && system.side.len() == count;
    tally.check("count", count_ok, &mut ok, label);
    if count_ok {
        let mut position = true;
        let mut velocity = true;
        let mut scalars = true;
        for i in 0..count {
            let p = &system.pool[i];
            position &= same_x(p.position[0], &particles["pos"][0][i])
                && (1..3).all(|a| same(p.position[a], &particles["pos"][a][i]));
            velocity &= same_x(p.velocity[0], &particles["vel"][0][i])
                && (1..3).all(|a| same(p.velocity[a], &particles["vel"][a][i]));
            let colour = u32::from_le_bytes(moly_law::particle::gradient::quantize_rgba8(system.side[i].colour));
            scalars &= same(p.age_percent, &particles["age"][i])
                && same(p.inverse_lifetime, &particles["inv"][i])
                && system.side[i].seed == word(&particles["seed"][i])
                && colour == word(&particles["color"][i]);
        }
        tally.check("position", position, &mut ok, label);
        tally.check("velocity", velocity, &mut ok, label);
        tally.check("ageLifetimeSeedColour", scalars, &mut ok, label);
    }
    tally.frames += 1;
    if !ok {
        tally.mismatched_frames += 1;
    }
}

fn frame_context(input: &Value) -> Context {
    let t: [f32; 3] = std::array::from_fn(|a| f(&input["positionBits"][a]));
    Context {
        sky: GlobalTransform::from_translation(Vec3::new(-t[0], t[1], t[2])),
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Driver {
    /// The product per-frame entry.
    Product,
    /// One-rule arm: the whole scaled frame as one step (the entry before the
    /// slice driver).
    WholeFrame,
    /// One-rule arm: the slices without the emitter-motion placement.
    NoBacktrack,
}

fn run_case(start: &Value, case: &Value, driver: Driver, tally: &mut Tally) {
    let mut system = harness_system(start, &case["config"]);
    install(&mut system, &case["seeds"]);
    let name = case["name"].as_str().unwrap();
    let before = &case["frames"][0]["before"];
    assert!(before["pendingBits"] == 0 && before["clockBits"] == 0 && before["flag28"] == 1, "{name}");
    let mut pending = 0.0_f32;
    for (index, row) in case["frames"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        assert!(matches!(input["flags"].as_u64(), Some(0 | 8)), "{name}");
        if input["reset"].as_bool().unwrap() {
            system.native_birth.as_mut().unwrap().frame.reset_previous = true;
        }
        let ctx = frame_context(input);
        let dt = f(&input["dtBits"]);
        match driver {
            Driver::Product => advance_frame(&mut system, dt, &ctx, true),
            Driver::WholeFrame => {
                let step = dt * system.emitter.simulation_speed;
                if step > 0.0 { simulate(&mut system, step, &ctx); }
            }
            Driver::NoBacktrack => {
                let head = moly_law::particle::prewarm::FrameStep::new(dt, system.emitter.simulation_speed, PLAYER_TIME);
                if !head.skips() {
                    let mut state = system.native_birth.take().unwrap();
                    let mut plan = head.plan(pending, system.emitter.duration).unwrap();
                    for slice in plan.by_ref() {
                        let _ = birth::step_explicit(&mut system, &mut state, slice.unwrap().duration, false, &ctx);
                    }
                    pending = plan.remaining();
                    system.native_birth = Some(state);
                }
            }
        }
        if driver == Driver::Product {
            assert_eq!(system.refused_total, 0, "{name} frame {index} refused");
        }
        compare(&system, row, driver == Driver::Product, tally, &format!("{name}#{index}"));
    }
}

#[test]
#[ignore = "MOLY_UPDATE1B_FRAMES and MOLY_UPDATE1B_FRAMES_EXTRA must identify the current JP per-frame Update1b receipts"]
fn installed_per_frame_update_matches_native_update1b_rows() {
    let receipt = read("MOLY_UPDATE1B_FRAMES");
    let extra = read("MOLY_UPDATE1B_FRAMES_EXTRA");
    for document in [&receipt, &extra] {
        assert_eq!(document["sourceSha256"], SOURCE_SHA256);
    }
    let start = &receipt["source"]["system"]["start"];
    // Every unpatched case without an emission-over-distance rate: that
    // branch is not installed, so its cases are refused at qualification.
    let cases: Vec<&Value> = [&receipt, &extra].into_iter()
        .flat_map(|d| ["s11Cases", "s1Cases", "controls"].into_iter()
            .filter_map(move |group| d[group].as_array()))
        .flatten()
        .filter(|case| case["patched"] != true && case["config"]["rate_distance"] == 0.0)
        .collect();
    let mut product = Tally::default();
    for case in &cases {
        run_case(start, case, Driver::Product, &mut product);
    }
    let mut whole = Tally::default();
    let mut unplaced = Tally::default();
    for case in &cases {
        run_case(start, case, Driver::WholeFrame, &mut whole);
        run_case(start, case, Driver::NoBacktrack, &mut unplaced);
    }
    let report = json!({
        "cases": cases.iter().map(|c| c["name"].clone()).collect::<Vec<_>>(),
        "frames": product.frames, "mismatchedFrames": product.mismatched_frames,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(),
        "firstMismatches": product.first,
        "arms": {"wholeFrame": whole.mismatched_frames, "noBacktrack": unplaced.mismatched_frames,
            "noBacktrackFirst": unplaced.first},
    });
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_UPDATE1B_FRAMES_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!((cases.len(), product.frames), (10, 202));
    assert_eq!(product.mismatched_frames, 0, "{report}");
    // Both arms must be seen to fail on the same native rows.
    assert!(whole.mismatched_frames > 0 && unplaced.mismatched_frames > 0, "{report}");
}

/// Frames the engine skips or clamps, fed to the product entry: nothing may
/// panic or refuse, and a skipped frame changes only the emitter velocity and
/// the end-of-frame translation.
#[test]
fn nonfinite_and_degenerate_frame_inputs_complete() {
    let config = json!({"space": 1, "speed": 1.0, "looping": true, "duration": 1.0, "rate_time": 60.0,
        "rate_distance": 0.0, "maximum": 200, "velocity_mode": 0});
    let start = json!({"lifetime": {"mode": "constant", "value": 0.5}, "speed": {"mode": "constant", "value": 1.0},
        "size": {"mode": "constant", "value": 1.0}, "rotation": {"mode": "constant", "value": 0.0},
        "color": {"mode": "color", "color": [1.0, 1.0, 1.0, 1.0]}, "size3D": false, "rotation3D": false,
        "gravityModifier": {"mode": "constant", "value": 0.0}});
    let seeds = json!({"initialWords": (1..=16).collect::<Vec<u32>>(), "emissionWords": [17, 19, 127, 2471805022u32]});
    for speed in [1.0, 0.0, -1.0, f32::NAN, f32::INFINITY, f32::MIN_POSITIVE, 1.0e30] {
        let mut system = harness_system(&start, &config);
        system.emitter.simulation_speed = speed;
        install(&mut system, &seeds);
        for (index, dt) in [1.0 / 60.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 1.0e-7,
            f32::MAX, 1.0e5, 0.25, f32::MIN_POSITIVE].into_iter().enumerate() {
            let ctx = Context {
                sky: GlobalTransform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                camera: GlobalTransform::IDENTITY,
                site: GlobalTransform::IDENTITY,
            };
            advance_frame(&mut system, dt, &ctx, index % 2 == 0);
            assert!(system.pool.len() <= 200);
        }
    }
}
