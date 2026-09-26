//! The installed per-frame update against native Update1b rows: the frame
//! head (emitter velocity, speed scale, minimum-step skip, pending time), the
//! emission over distance it runs before the slices, the incremental slices
//! and the emitter-motion placement of World births, read through the product
//! frame entry. The native harness drove one exported start block with Shape
//! and Colour off, and Velocity and CustomData off or with their exported
//! blocks; the probe RNG words it wrote replace the installed streams, never a
//! claim about client entropy.
use super::*;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

mod seed_zero;

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
    harness_system_with(start, config, None)
}

/// The same with the recorded Velocity and CustomData blocks enabled.
fn harness_system_with(start: &Value, config: &Value, modules: Option<&Value>) -> Runtime {
    let space = if config["space"].as_u64() == Some(0) { "Local" } else { "World" };
    let mut system = json!({
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
    if let Some(modules) = modules {
        system["velocityOverLifetime"] = modules["velocityOverLifetime"].clone();
        system["customData"] = modules["customData"].clone();
        system["sourceModules"]["enabled"] =
            json!(["CustomDataModule", "EmissionModule", "InitialModule", "VelocityModule"]);
    }
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
    runtime.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    runtime.velocity_law = emitter.velocity_over_lifetime.as_ref()
        .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).unwrap());
    runtime.custom_law = emitter.custom_data.as_ref()
        .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).unwrap());
    assert_eq!(runtime.velocity_law.is_some() && runtime.custom_law.is_some(), modules.is_some());
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
    replace_streams(system, seeds);
}

/// The probe RNG words replace the installed streams of the native owner.
fn replace_streams(system: &mut Runtime, seeds: &Value) {
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
    // A host that left the system without the native owner has none of the
    // owner state to compare: the frame mismatches as a whole.
    let Some(state) = system.native_birth.as_ref() else {
        let mut ok = true;
        tally.check("nativeOwner", false, &mut ok, label);
        tally.frames += 1;
        tally.mismatched_frames += 1;
        return;
    };
    let after = &row["after"];
    let mut ok = true;
    if frame_state {
        tally.check("pending", same(system.pending, &after["pendingBits"]), &mut ok, label);
        let prev = &after["prevBits"];
        let velocity = &row["velocityAfterUpdate1bBits"];
        tally.check("previousPosition", same_x(state.frame.previous_position[0], &prev[0])
            && (1..3).all(|a| same(state.frame.previous_position[a], &prev[a])), &mut ok, label);
        tally.check("emitterVelocity", same_x(state.frame.velocity[0], &velocity[0])
            && (1..3).all(|a| same(state.frame.velocity[a], &velocity[a])), &mut ok, label);
    }
    tally.check("clock", same(system.playback_head, &after["clockBits"]), &mut ok, label);
    if let Some(bits) = after.get("delayBits") {
        tally.check("startDelay", state.frame.start_delay.to_bits() == word(bits), &mut ok, label);
    }
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
        // The modules receipt also records the animated velocity and the
        // first custom stream of every particle at the frame end.
        if let (Some(anim), Some(custom)) = (particles.get("anim"), particles.get("custom1")) {
            let animated = (0..count).all(|i| same_x(system.side[i].animated[0], &anim[0][i])
                && (1..3).all(|a| same(system.side[i].animated[a], &anim[a][i])));
            tally.check("animatedVelocity", animated, &mut ok, label);
            let custom1 = (0..count).all(|i| (0..4).all(|c| same(system.side[i].custom_data[0][c], &custom[c][i])));
            tally.check("custom1", custom1, &mut ok, label);
        }
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
    run_case_with(start, case, None, driver, tally);
}

fn run_case_with(start: &Value, case: &Value, modules: Option<&Value>, driver: Driver, tally: &mut Tally) {
    let mut system = harness_system_with(start, &case["config"], modules);
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
            Driver::Product => { let _ = advance_frame(&mut system, dt, true, &ctx, |_| {}); }
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
        if driver == Driver::Product && !child::arms::on("distanceAfterSlices") && !child::arms::on("elapsedWithoutPending") {
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
    // Every unpatched case, with and without an emission-over-distance rate.
    let cases: Vec<&Value> = [&receipt, &extra].into_iter()
        .flat_map(|d| ["s11Cases", "s1Cases", "controls"].into_iter()
            .filter_map(move |group| d[group].as_array()))
        .flatten()
        .filter(|case| case["patched"] != true)
        .collect();
    let report = replay(start, &cases, None, "MOLY_UPDATE1B_FRAMES_REPORT");
    assert_eq!((cases.len(), report["frames"].as_u64().unwrap()), (21, 667));
}

/// The distance cases with the exported Velocity and CustomData modules on
/// (and their modules-off control): newborns at a negative elapsed time
/// through the animated velocity, compared with the native animated velocity
/// and custom stream as well.
#[test]
#[ignore = "MOLY_UPDATE1B_FRAMES_MODULES must identify the current JP per-frame Update1b modules receipt"]
fn installed_distance_emission_with_modules_matches_native_update1b_rows() {
    let receipt = read("MOLY_UPDATE1B_FRAMES_MODULES");
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    let start = &read("MOLY_UPDATE1B_FRAMES")["source"]["system"]["start"].clone();
    let cases: Vec<&Value> = ["s11Cases", "controls"].into_iter()
        .filter_map(|group| receipt[group].as_array()).flatten()
        .filter(|case| case["patched"] != true).collect();
    let report = replay(start, &cases, Some(&receipt["moduleSource"]), "MOLY_UPDATE1B_FRAMES_MODULES_REPORT");
    assert_eq!((cases.len(), report["frames"].as_u64().unwrap()), (3, 180));
}

/// The product entry over `cases` with every one-rule arm; panics unless the
/// product matches every native frame and each arm differs somewhere.
fn replay(start: &Value, cases: &[&Value], module_source: Option<&Value>, report_key: &str) -> Value {
    let modules = |case: &Value| (case["modules"] == true).then(|| module_source.expect("module blocks"));
    let run = |driver: Driver, arm: Option<&'static str>| {
        child::arms::set(arm);
        let mut tally = Tally::default();
        for case in cases {
            run_case_with(start, case, modules(case), driver, &mut tally);
        }
        child::arms::set(None);
        tally
    };
    let product = run(Driver::Product, None);
    let whole = run(Driver::WholeFrame, None);
    let unplaced = run(Driver::NoBacktrack, None);
    let distance_cases = cases.iter().any(|case| case["config"]["rate_distance"] != 0.0);
    let (late, unpending) = if distance_cases {
        (Some(run(Driver::Product, Some("distanceAfterSlices"))), Some(run(Driver::Product, Some("elapsedWithoutPending"))))
    } else {
        (None, None)
    };
    let report = json!({
        "cases": cases.iter().map(|c| c["name"].clone()).collect::<Vec<_>>(),
        "frames": product.frames, "mismatchedFrames": product.mismatched_frames,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(),
        "firstMismatches": product.first,
        "arms": {"wholeFrame": whole.mismatched_frames, "noBacktrack": unplaced.mismatched_frames,
            "noBacktrackFirst": unplaced.first,
            "distanceAfterSlices": late.as_ref().map(|t| t.mismatched_frames),
            "elapsedWithoutPending": unpending.as_ref().map(|t| t.mismatched_frames)},
    });
    println!("{report}");
    if let Some(path) = std::env::var_os(report_key) {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(product.mismatched_frames, 0, "{report}");
    // Every arm must be seen to fail on the same native rows.
    assert!(whole.mismatched_frames > 0 && unplaced.mismatched_frames > 0, "{report}");
    for arm in [&late, &unpending].into_iter().flatten() {
        assert!(arm.mismatched_frames > 0, "{report}");
    }
    report
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
    let mut distance = config.clone();
    distance["rate_distance"] = json!(10.0);
    for (speed, config) in [1.0, 0.0, -1.0, f32::NAN, f32::INFINITY, f32::MIN_POSITIVE, 1.0e30].into_iter()
        .flat_map(|speed| [(speed, &config), (speed, &distance)]) {
        let mut system = harness_system(&start, config);
        system.emitter.simulation_speed = speed;
        install(&mut system, &seeds);
        for (index, dt) in [1.0 / 60.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 1.0e-7,
            f32::MAX, 1.0e5, 0.25, f32::MIN_POSITIVE].into_iter().enumerate() {
            let ctx = Context {
                sky: GlobalTransform::from_translation(Vec3::new((index as f32).powi(9), 0.0, 0.0)),
                camera: GlobalTransform::IDENTITY,
                site: GlobalTransform::IDENTITY,
            };
            let _ = advance_frame(&mut system, dt, index % 2 == 0, &ctx, |_| {});
            assert!(system.pool.len() <= 200);
        }
    }
}

/// The per-frame native rows of this harness (the unpatched cases of the
/// three Update1b receipts: the per-frame and distance cases, their extra
/// World backtrack cases and the Velocity and CustomData module cases)
/// through a host's own install (`install`, which must leave the native
/// owner in place for the probe words to replace its streams; a host that
/// does not is compared as a whole-frame mismatch) and its own frame entry
/// (`step`, with the raw frame dt). Returns cases, frames, mismatched frames
/// and the first mismatches.
pub(crate) fn replay_through_host<H>(
    install: &dyn Fn(&mut Runtime, &mut seed::SystemSeedManager) -> H,
    step: &dyn Fn(&mut Runtime, &mut H, f32, &Context),
) -> (usize, usize, usize, Vec<String>) {
    let receipt = read("MOLY_UPDATE1B_FRAMES");
    let extra = read("MOLY_UPDATE1B_FRAMES_EXTRA");
    let modules = read("MOLY_UPDATE1B_FRAMES_MODULES");
    for document in [&receipt, &extra, &modules] {
        assert_eq!(document["sourceSha256"], SOURCE_SHA256);
    }
    let start = &receipt["source"]["system"]["start"];
    let cases: Vec<&Value> = [&receipt, &extra, &modules].into_iter()
        .flat_map(|d| ["s11Cases", "s1Cases", "controls"].into_iter().filter_map(move |group| d[group].as_array()))
        .flatten()
        .filter(|case| case["patched"] != true)
        .collect();
    let mut tally = Tally::default();
    for case in &cases {
        let block = (case["modules"] == true).then(|| &modules["moduleSource"]);
        let mut system = harness_system_with(start, &case["config"], block);
        let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
        let mut host = install(&mut system, &mut manager);
        if system.native_birth.is_some() {
            replace_streams(&mut system, &case["seeds"]);
        }
        let name = case["name"].as_str().unwrap();
        for (index, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let input = &row["input"];
            if input["reset"].as_bool().unwrap() {
                if let Some(native) = system.native_birth.as_mut() {
                    native.frame.reset_previous = true;
                }
            }
            step(&mut system, &mut host, f(&input["dtBits"]), &frame_context(input));
            compare(&system, row, true, &mut tally, &format!("{name}#{index}"));
        }
    }
    (cases.len(), tally.frames, tally.mismatched_frames, tally.first)
}

/// Run `f` with one of the runtime's one-rule arms on (`None`: none).
pub(crate) fn with_arm<T>(arm: Option<&'static str>, f: impl FnOnce() -> T) -> T {
    child::arms::set(arm);
    let result = f();
    child::arms::set(None);
    result
}

/// The harness emitter of a case that carries its own start block, module
/// blocks (any of Velocity, CustomData, ClampVelocity, RotationOverLifetime,
/// RotationBySpeed, Force, in the export's shape) and emission bursts and
/// looping in its config.
fn harness_system_modules(start: &Value, config: &Value, modules: &Value) -> Runtime {
    let mut runtime = harness_system(start, config);
    let space = if config["space"].as_u64() == Some(0) { "Local" } else { "World" };
    let mut enabled = vec!["EmissionModule", "InitialModule"];
    let mut system = json!({
        "duration": config["duration"], "looping": config["looping"], "prewarm": false,
        "playOnAwake": true, "simulationSpeed": 1.0, "simulationSpace": space,
        "randomSeed": 0, "autoRandomSeed": true, "emitterVelocityMode": config["velocity_mode"],
        "startDelay": {"mode": "constant", "value": config.get("start_delay").cloned().unwrap_or(json!(0.0))},
        "ringBufferMode": 0, "ringBufferLoopRange": [0.0, 1.0],
        "maxParticles": config["maximum"], "start": start.clone(),
        "emission": {"rateOverTime": {"mode": "constant", "value": config["rate_time"]},
            "rateOverDistance": {"mode": "constant", "value": config["rate_distance"]},
            "bursts": config.get("bursts").cloned().unwrap_or(json!([]))},
        "shapeEnabled": false,
    });
    for (key, module) in [("velocityOverLifetime", "VelocityModule"), ("customData", "CustomDataModule"),
        ("limitVelocity", "ClampVelocityModule"), ("rotationOverLifetime", "RotationModule"),
        ("rotationBySpeed", "RotationBySpeedModule"), ("forceOverLifetime", "ForceModule")] {
        if let Some(block) = modules.get(key).filter(|b| b.is_object()) {
            system[key] = block.clone();
            enabled.push(module);
        }
    }
    enabled.sort_unstable();
    system["sourceModules"] = json!({"version": 1, "enabled": enabled, "unsupported": []});
    let document = json!({"effects": {"harness": {"particles": [{"node": "root/harness", "system": system}]}}});
    let mut decoded = moly_law::particle::schema::Effects::from_json_str(
        &serde_json::to_vec(&document).unwrap()).unwrap();
    let mut emitter = decoded.emitters.remove(0);
    emitter.simulation_speed = runtime.emitter.simulation_speed;
    runtime.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    runtime.velocity_law = emitter.velocity_over_lifetime.as_ref()
        .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).unwrap());
    runtime.custom_law = emitter.custom_data.as_ref()
        .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).unwrap());
    runtime.limit = emitter.limit_velocity.as_ref().map(|p| LimitVelocity::from_parts(p.separate_axis,
        &p.magnitude, p.dampen, p.drag.as_ref(), p.multiply_drag_by_size, p.multiply_drag_by_velocity).unwrap());
    runtime.rol = emitter.rotation_over_lifetime.as_ref().map(|p|
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve).unwrap());
    runtime.force_law = emitter.force.as_ref()
        .map(|p| moly_law::particle::force::ForceOverLifetime::from_params(p).unwrap());
    runtime.emitter = emitter;
    runtime
}

/// One case of the distance module receipt through the product frame entry;
/// also compares the rotation lanes the receipt records.
fn run_module_case(receipt: &Value, case: &Value, tally: &mut Tally, rotation: &mut (usize, usize)) {
    let start = case.get("start").unwrap_or(&receipt["source"]["system"]["start"]);
    let modules = case.get("moduleSource").unwrap_or(&receipt["moduleSource"]);
    let modules = if case["modules"] == false { &Value::Null } else { modules };
    let mut system = harness_system_modules(start, &case["config"], modules);
    install(&mut system, &case["seeds"]);
    let name = case["name"].as_str().unwrap();
    for (index, row) in case["frames"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        if input["reset"].as_bool().unwrap() {
            system.native_birth.as_mut().unwrap().frame.reset_previous = true;
        }
        // The native stop byte before the frame (the non-looping end sets it
        // inside the incremental update); the host's play state stands in.
        let stopped = row.get("stoppedBefore").is_some_and(|v| v == true || v == 1);
        // UpdateData flags 0 is the per-frame update; 4 a script Simulate
        // chunk (a Director's), which the product runs through its own entry.
        let _ = match input["flags"].as_u64() {
            Some(0) => advance_frame(&mut system, f(&input["dtBits"]), !stopped, &frame_context(input), |_| {}),
            Some(4) => director_chunk(&mut system, f(&input["dtBits"]), !stopped, &frame_context(input), |_| {}),
            other => panic!("{name}: UpdateData flags {other:?} have no product entry here"),
        };
        let label = format!("{name}#{index}");
        compare(&system, row, true, tally, &label);
        let particles = &row["particles"];
        if let Some(rot) = particles.get("rot").filter(|r| r.is_array()) {
            if system.pool.len() == particles["count"].as_u64().unwrap() as usize {
                let ok = (0..system.pool.len()).all(|i| (0..3).all(|a| same(system.side[i].rot[a], &rot[a][i])));
                rotation.0 += 1;
                if !ok {
                    rotation.1 += 1;
                    if tally.first.len() < 12 {
                        let (i, a) = (0..system.pool.len()).flat_map(|i| (0..3).map(move |a| (i, a)))
                            .find(|&(i, a)| !same(system.side[i].rot[a], &rot[a][i])).unwrap();
                        tally.first.push(format!("{label} rotation particle {i} axis {a}: {:08x} native {:08x} age {:08x} seed {:08x}",
                            system.side[i].rot[a].to_bits(), f(&rot[a][i]).to_bits(),
                            system.pool[i].age_percent.to_bits(), system.side[i].seed));
                    }
                }
            }
        }
    }
}

fn replay_modules(receipt: &Value, arm: Option<&'static str>) -> (usize, Tally, (usize, usize)) {
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    let cases: Vec<&Value> = ["s11Cases", "controls"].into_iter()
        .filter_map(|group| receipt[group].as_array()).flatten()
        .filter(|case| case["patched"] != true).collect();
    child::arms::set(arm);
    let mut tally = Tally::default();
    let mut rotation = (0, 0);
    for case in &cases {
        run_module_case(receipt, case, &mut tally, &mut rotation);
    }
    child::arms::set(None);
    (cases.len(), tally, rotation)
}

/// Distance births (and time births) through ClampVelocity and
/// RotationOverLifetime with the walk-in-water blocks, bursts sharing the
/// emission step with a distance rate, and a non-looping distance rate past
/// the end: every native frame of the receipt through the product frame
/// entry, the rotation lanes included. The one-rule arms (rotation or
/// ClampVelocity skipped at a negative elapsed time) must each mismatch.
#[test]
#[ignore = "MOLY_PLAW_DISTANCE_MODULES must identify the JP distance module receipt"]
fn distance_births_through_clamp_and_rotation_match_native_rows() {
    let receipt = read("MOLY_PLAW_DISTANCE_MODULES");
    let (cases, product, rotation) = replay_modules(&receipt, None);
    let report = json!({"cases": cases, "frames": product.frames, "mismatchedFrames": product.mismatched_frames,
        "rotationFrames": rotation.0, "rotationMismatched": rotation.1,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "firstMismatches": product.first});
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_PLAW_DISTANCE_MODULES_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(cases > 0 && product.frames > 0 && rotation.0 > 0, "{report}");
    assert_eq!((product.mismatched_frames, rotation.1), (0, 0), "{report}");
    // The newborns reach both modules at their negative elapsed time.
    for arm in ["rotationSkipsNegativeElapsed", "clampSkipsNegativeElapsed"] {
        let (_, tally, rotation) = replay_modules(&receipt, Some(arm));
        println!("arm {arm}: {} mismatched frames, {} rotation", tally.mismatched_frames, rotation.1);
        assert!(tally.mismatched_frames + rotation.1 > 0, "arm {arm} must mismatch");
    }
}

/// Systems with a start delay, played and then stepped by the product frame
/// entry over the native Update1b rows: the delay word per frame, the clock
/// held at zero while the word is not below the slice, the births of the
/// slice where the word runs out (its overshoot, none when it lands on zero
/// exactly), the frame head's distance births held back while the word is
/// not zero, and a looping delayed system. Every one-rule arm must mismatch.
#[test]
#[ignore = "MOLY_PLAW_START_DELAY must identify the JP start delay receipt"]
fn start_delay_matches_native_rows() {
    let receipt = read("MOLY_PLAW_START_DELAY");
    let (cases, product, rotation) = replay_modules(&receipt, None);
    let report = json!({"cases": cases, "frames": product.frames, "mismatchedFrames": product.mismatched_frames,
        "rotationFrames": rotation.0, "rotationMismatched": rotation.1,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "firstMismatches": product.first});
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_PLAW_START_DELAY_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(cases > 0 && product.frames > 0 && product.fields.get("startDelay").is_some_and(|f| f.0 > 0), "{report}");
    assert_eq!((product.mismatched_frames, rotation.1), (0, 0), "{report}");
    for arm in ["delayTicksWholeSlice", "delayBirthsWholeSlice", "distanceDuringDelay"] {
        let (_, tally, rotation) = replay_modules(&receipt, Some(arm));
        println!("arm {arm}: {} mismatched frames, {} rotation", tally.mismatched_frames, rotation.1);
        assert!(tally.mismatched_frames + rotation.1 > 0, "arm {arm} must mismatch");
    }
}

/// A control copy of the receipt with mutated native words must mismatch.
#[test]
#[ignore = "MOLY_PLAW_DISTANCE_MODULES_CONTROLS identifies mutated copies of the distance module receipt"]
fn distance_module_controls_fail() {
    let paths = std::env::var("MOLY_PLAW_DISTANCE_MODULES_CONTROLS").expect("controls");
    for path in paths.split(';').filter(|p| !p.is_empty()) {
        let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let (_, product, rotation) = replay_modules(&receipt, None);
        println!("control {path}: {} mismatched frames, {} rotation", product.mismatched_frames, rotation.1);
        assert!(product.mismatched_frames + rotation.1 > 0, "control {path} must mismatch");
    }
}

/// Distance births with orbital, orbital-offset and radial Velocity terms
/// (Local and World, per frame and on a Director clip's chunks), module
/// curves at the stored age word (Velocity, ClampVelocity, Force), script
/// Simulate chunks without the backlog widening and a delayed system on
/// them: every native frame of the receipt through the product entries. The
/// receipt's own controls must hold (the supportsProcedural byte changes no
/// row and is not read on the per-frame path while the clock is; Velocity
/// off, flags 0 and the cleared identity each change rows), and every
/// one-rule arm must mismatch.
#[test]
#[ignore = "MOLY_BIRTHS_RECEIPT must identify the JP births receipt"]
fn births_receipt_matches_native_rows() {
    let receipt = read("MOLY_BIRTHS_RECEIPT");
    let (cases, product, rotation) = replay_modules(&receipt, None);
    let report = json!({"cases": cases, "frames": product.frames, "mismatchedFrames": product.mismatched_frames,
        "rotationFrames": rotation.0, "rotationMismatched": rotation.1,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "firstMismatches": product.first});
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_BIRTHS_RECEIPT_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(cases > 0 && product.frames > 0 && rotation.0 > 0, "{report}");
    assert_eq!((product.mismatched_frames, rotation.1), (0, 0), "{report}");
    let procedural = &receipt["procedural"];
    assert_eq!(procedural["differingFrames"], 0);
    assert!(procedural["readsOfState20to23"].as_object().unwrap().is_empty());
    assert!(procedural["readsOfClock"].as_object().unwrap().values().any(|n| n.as_u64().unwrap() > 0));
    for control in ["k3VersusO3DifferingFrames", "k20VersusS1DifferingFrames", "kfVersusA1DifferingFrames"] {
        assert!(receipt[control].as_u64().unwrap() > 0, "{control}");
    }
    assert!(receipt["staticInitializers"]["o1VersusIdentityClearedDifferingFrames"].as_u64().unwrap() > 0);
    for arm in ["moduleAgeFromNormalized", "orbitalSkipsBirths", "scriptSimulateWidens"] {
        let (_, tally, rotation) = replay_modules(&receipt, Some(arm));
        println!("arm {arm}: {} mismatched frames, {} rotation", tally.mismatched_frames, rotation.1);
        assert!(tally.mismatched_frames + rotation.1 > 0, "arm {arm} must mismatch");
    }
    if let Ok(paths) = std::env::var("MOLY_BIRTHS_RECEIPT_CONTROLS") {
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            let control: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let (_, tally, rotation) = replay_modules(&control, None);
            println!("control {path}: {} mismatched frames, {} rotation", tally.mismatched_frames, rotation.1);
            assert!(tally.mismatched_frames + rotation.1 > 0, "control {path} must mismatch");
        }
    }
}

/// Play after Stop against the native bodies (the Play-after-Stop receipt): a
/// walk-water sprash system walking at a fixed speed with Stop (StopEmitting)
/// and Play every step, and Play with no particle alive after standing, with a
/// manual and an automatic owner. Frames go through the product frame entry;
/// Stop is the host's emitting flag (the runtime has no Stop state of its
/// own); Play is [`play_after_stop`] or one of the arms below.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PlayArm {
    Product,
    /// What the foot consumer's restart writes alone: the clock and its
    /// legacy emission fields.
    ConsumerFields,
    NoEmitterReset,
    ClearCarry,
    /// With none alive: the with-particles branch (no seed reset).
    NoSeedReset,
    KeepDelay,
}

fn play_arm(system: &mut Runtime, manager: &mut seed::SystemSeedManager, arm: PlayArm) {
    if arm == PlayArm::ConsumerFields {
        system.playback_head = 0.0;
        system.previous_head = 0.0;
        system.emission_started = false;
        system.emission = Default::default();
        system.prewarmed = false;
        return;
    }
    let delay = system.native_birth.as_ref().map(|native| native.frame.start_delay);
    if arm == PlayArm::NoSeedReset && system.pool.is_empty() {
        system.playback_head = 0.0;
        system.previous_head = 0.0;
        system.pending = 0.0;
        let word = play_start_delay(&system.emitter).unwrap();
        let native = system.native_birth.as_mut().unwrap();
        native.frame.reset_previous = true;
        native.frame.start_delay = word;
    } else {
        play_after_stop(system, manager, &SourceRoute::Ordinary, None).unwrap();
    }
    let native = system.native_birth.as_mut().unwrap();
    match arm {
        PlayArm::NoEmitterReset => native.frame.reset_previous = false,
        PlayArm::ClearCarry => {
            native.emission.distribution.spacing = 0.0;
            native.emission.distribution.offset = 0.0;
        }
        PlayArm::KeepDelay => native.frame.start_delay = delay.unwrap(),
        _ => {}
    }
}

/// The product state right after Play against the native state words Play
/// left.
fn compare_play(system: &Runtime, event: &Value, tally: &mut Tally, label: &str) {
    let after = &event["after"];
    let mut ok = true;
    let Some(state) = system.native_birth.as_ref() else {
        tally.check("play.nativeOwner", false, &mut ok, label);
        tally.frames += 1;
        tally.mismatched_frames += 1;
        return;
    };
    tally.check("play.pending", same(system.pending, &after["pendingBits"]), &mut ok, label);
    tally.check("play.clock", same(system.playback_head, &after["clockBits"]), &mut ok, label);
    tally.check("play.startDelay", state.frame.start_delay.to_bits() == word(&after["delayBits"]), &mut ok, label);
    tally.check("play.emitterReset", state.frame.reset_previous == (after["flag28"] == 1), &mut ok, label);
    let velocity = &after["velBits"];
    tally.check("play.emitterVelocity", same_x(state.frame.velocity[0], &velocity[0])
        && (1..3).all(|a| same(state.frame.velocity[a], &velocity[a])), &mut ok, label);
    let distribution = state.emission.distribution;
    let carry = &after["emission"]["f"];
    tally.check("play.emissionCarry", same(distribution.spacing, &carry[0]) && same(distribution.offset, &carry[1])
        && same(distribution.burst_fraction, &carry[2]), &mut ok, label);
    let rng = &after["emission"]["rng"];
    tally.check("play.emissionRng", (0..4).all(|i| state.emission.random.words[i] == word(&rng[i])), &mut ok, label);
    let initial = &after["initialRng"];
    tally.check("play.initialRng", (0..16).all(|i| state.initial.words[i / 4][i % 4] == word(&initial[i])),
        &mut ok, label);
    tally.check("play.count", system.pool.len() as u64 == after["count"].as_u64().unwrap(), &mut ok, label);
    if event["native"]["ResetSeeds"].as_u64().unwrap_or(0) > 0 {
        tally.check("play.ownerSeed", state.owner.as_ref().is_some_and(|owner| owner.seed == word(&after["roSeed"])),
            &mut ok, label);
    }
    tally.frames += 1;
    if !ok {
        tally.mismatched_frames += 1;
    }
}

fn run_play_case(case: &Value, arm: PlayArm, tally: &mut Tally) {
    let mut system = harness_system(&case["start"], &case["config"]);
    system.emitter.random_seed = Some(word(&case["owner"]["randomSeed"]));
    system.emitter.auto_random_seed = Some(case["owner"]["autoRandomSeed"].as_bool().unwrap());
    let delay = case["config"]["start_delay"].as_f64().unwrap() as f32;
    system.emitter.start_delay = moly_law::particle::MinMaxCurve::Constant(delay);
    install(&mut system, &case["seeds"]);
    // The receipt starts after the first Play, which wrote the delay word.
    system.native_birth.as_mut().unwrap().frame.start_delay = delay;
    let words: [u32; 4] = std::array::from_fn(|i| word(&case["owner"]["managerWords"][i]));
    let mut manager = seed::SystemSeedManager::from_entropy_words(words);
    let name = case["name"].as_str().unwrap();
    let mut emitting = true;
    for (index, event) in case["events"].as_array().unwrap().iter().enumerate() {
        let label = format!("{name} event {index}");
        match event["kind"].as_str().unwrap() {
            "frame" => {
                let ctx = frame_context(&event["input"]);
                let _ = advance_frame(&mut system, f(&event["input"]["dtBits"]), emitting, &ctx, |_| {});
                compare(&system, event, true, tally, &label);
            }
            "stop" => emitting = false,
            "play" => {
                emitting = true;
                play_arm(&mut system, &mut manager, arm);
                compare_play(&system, event, tally, &label);
            }
            other => panic!("{label}: event kind {other}"),
        }
    }
}

fn replay_play(receipt: &Value, arm: PlayArm) -> Tally {
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    let mut tally = Tally::default();
    for case in receipt["cases"].as_array().unwrap() {
        run_play_case(case, arm, &mut tally);
    }
    tally
}

#[test]
#[ignore = "MOLY_PLAY_AFTER_STOP_RECEIPT must identify the JP Play-after-Stop receipt"]
fn play_after_stop_matches_native_rows() {
    let receipt = read("MOLY_PLAY_AFTER_STOP_RECEIPT");
    let product = replay_play(&receipt, PlayArm::Product);
    let report = json!({"summary": receipt["summary"], "events": product.frames, "mismatched": product.mismatched_frames,
        "fields": product.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "firstMismatches": product.first});
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_PLAY_AFTER_STOP_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    // The positive arm has a signal to lose: Plays with particles alive and with
    // none alive, and first frames after a Play whose emitter velocity the reset
    // zeroed while the emitter moved.
    let summary = receipt["summary"].as_array().unwrap();
    let total = |key: &str| summary.iter().map(|s| s[key].as_u64().unwrap()).sum::<u64>();
    assert!(total("plays") > total("playsWithNoneAlive") && total("playsWithNoneAlive") > 0, "{report}");
    assert!(total("resetSeeds") == total("playsWithNoneAlive"), "{report}");
    assert!(total("framesAfterPlayWithZeroVelocity") > 0, "{report}");
    assert!(product.frames > 0, "{report}");
    assert_eq!(product.mismatched_frames, 0, "{report}");
    for arm in [PlayArm::ConsumerFields, PlayArm::NoEmitterReset, PlayArm::ClearCarry, PlayArm::NoSeedReset,
        PlayArm::KeepDelay] {
        let tally = replay_play(&receipt, arm);
        println!("arm {arm:?}: {} mismatched of {}; first {:?}", tally.mismatched_frames, tally.frames,
            tally.first.first());
        assert!(tally.mismatched_frames > 0, "arm {arm:?} must mismatch");
    }
    if let Some(path) = std::env::var_os("MOLY_PLAY_AFTER_STOP_BITFLIP") {
        let control: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let tally = replay_play(&control, PlayArm::Product);
        println!("bitflip: {} mismatched", tally.mismatched_frames);
        assert!(tally.mismatched_frames > 0, "the bit-flipped receipt must mismatch");
    }
}
