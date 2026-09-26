//! The seed pass of `EffectBehaviour.SetEffectInstance` and the stop and
//! clear of `EffectBehaviour.OnBehaviourPause` against the native bodies (the
//! seed-zero receipt): `ParticleSystem.randomSeed`'s setter, Stop with
//! StopEmittingAndClear, and the Play after them, on the walk-water sprash
//! system of the Play-after-Stop receipt. Frames go through the product frame
//! entry with the host's emitting flag, as the Play-after-Stop replay steps
//! them; every other event goes through the played-object entries of the
//! particle host (or, in the step arm, the host calls they are made of).
use super::*;
use crate::fixture_timeline_particles as played;
use crate::weather_fx::fixture as host;

#[derive(Clone, Copy, PartialEq, Debug)]
enum SeedArm {
    /// The played-object entries: the seed pass, the stop and clear of the
    /// pause, the object's Play and Stop.
    Entry,
    /// The host calls one by one, each event compared.
    Steps,
    /// The setter writes the seed and leaves the automatic flag.
    KeepAuto,
    /// The setter resets the streams at once.
    SetterResets,
    /// StopEmitting in place of StopEmittingAndClear.
    StopWithoutClear,
    /// No seed write.
    NoSeed,
}

struct Case {
    world: World,
    root: Entity,
    node: Entity,
    draw: Entity,
    binding: played::ParticlePlayBinding,
    emitting: bool,
}

impl Case {
    fn new(case: &Value) -> Self {
        let mut system = harness_system(&case["start"], &case["config"]);
        system.emitter.random_seed = Some(word(&case["owner"]["randomSeed"]));
        system.emitter.auto_random_seed = Some(case["owner"]["autoRandomSeed"].as_bool().unwrap());
        system.emitter.start_delay = moly_law::particle::MinMaxCurve::Constant(0.0);
        // The receipt starts after the first Play; its probe words replace the
        // streams the install drew.
        let mut install = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
        let played_state = host::install_played(&mut system, &SourceRoute::Ordinary, &mut install).unwrap();
        replace_streams(&mut system, &case["seeds"]);
        system.native_birth.as_mut().unwrap().frame.start_delay = 0.0;
        let words: [u32; 4] = std::array::from_fn(|i| word(&case["owner"]["managerWords"][i]));
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.insert_resource(seed::SystemSeedManager::from_entropy_words(words));
        let root = world.spawn_empty().id();
        let node = world.spawn(ChildOf(root)).id();
        system.anchor = Some(node);
        let draw = world.spawn((crate::uber_particle::FixtureParticleLive(system), played_state, ChildOf(root))).id();
        let binding = played::played_object_for_replay(&mut world, root, vec![draw]);
        Self { world, root, node, draw, binding, emitting: true }
    }

    fn system(&self) -> &Runtime {
        &self.world.get::<crate::uber_particle::FixtureParticleLive>(self.draw).unwrap().0
    }

    fn system_mut(&mut self) -> Mut<'_, crate::uber_particle::FixtureParticleLive> {
        self.world.get_mut::<crate::uber_particle::FixtureParticleLive>(self.draw).unwrap()
    }
}

/// The read-only seed state the setter writes, as the product holds it: the
/// installed birth owner (a reset writes the drawn seed there), else the
/// emitter's.
fn read_only_seed(system: &Runtime) -> (u32, bool) {
    match system.native_birth.as_ref().and_then(|native| native.owner) {
        Some(owner) => (owner.seed, owner.automatic),
        None => (system.emitter.random_seed.unwrap(), system.emitter.auto_random_seed.unwrap()),
    }
}

/// The product state after a setter, stop or play event against the native
/// words the event left.
fn compare_state(case: &Case, event: &Value, tally: &mut Tally, label: &str) {
    let after = &event["after"];
    let system = case.system();
    let mut ok = true;
    let (seed, automatic) = read_only_seed(system);
    tally.check("state.readOnlySeed", seed == word(&after["roSeed"]) && automatic == (after["roAuto"] != 0),
        &mut ok, label);
    tally.check("state.count", system.pool.len() as u64 == after["count"].as_u64().unwrap(), &mut ok, label);
    tally.check("state.clock", same(system.playback_head, &after["clockBits"]), &mut ok, label);
    tally.check("state.playing", host::system_playing(&case.world, case.draw) == (after["playState"] != 0),
        &mut ok, label);
    match system.native_birth.as_ref() {
        Some(state) => {
            let initial = &after["initialRng"];
            tally.check("state.initialRng", (0..16).all(|i| state.initial.words[i / 4][i % 4] == word(&initial[i])),
                &mut ok, label);
            let rng = &after["emission"]["rng"];
            tally.check("state.emissionRng", (0..4).all(|i| state.emission.random.words[i] == word(&rng[i])),
                &mut ok, label);
        }
        None => tally.check("state.nativeOwner", false, &mut ok, label),
    }
    let manager: [u32; 4] = std::array::from_fn(|i| word(&after["manager"][i]));
    tally.check("state.sharedManager",
        case.world.resource::<seed::SystemSeedManager>().manager_words_for_test() == manager, &mut ok, label);
    tally.frames += 1;
    if !ok {
        tally.mismatched_frames += 1;
    }
}

fn seed_step(case: &mut Case, value: u32, arm: SeedArm) {
    if arm == SeedArm::NoSeed {
        return;
    }
    let was = case.system().emitter.auto_random_seed;
    assert!(host::set_random_seed(&mut case.world, case.draw, value));
    let mut live = case.system_mut();
    let system = &mut live.0;
    match arm {
        SeedArm::KeepAuto => {
            system.emitter.auto_random_seed = was;
            if let Some(owner) = system.native_birth.as_mut().and_then(|native| native.owner.as_mut()) {
                owner.automatic = was == Some(true);
            }
        }
        SeedArm::SetterResets => {
            let velocity = system.native_birth.as_ref().map(|native| native.frame.velocity);
            reset_for_first_play(system);
            install_native_birth(system, &mut seed::SystemSeedManager::default(), &SourceRoute::Ordinary, None).unwrap();
            if let (Some(native), Some(velocity)) = (system.native_birth.as_mut(), velocity) {
                native.frame.velocity = velocity;
            }
        }
        _ => {}
    }
}

fn run_seed_case(case_json: &Value, arm: SeedArm, tally: &mut Tally) {
    let mut case = Case::new(case_json);
    let name = case_json["name"].as_str().unwrap();
    let events = case_json["events"].as_array().unwrap();
    let mut index = 0;
    while index < events.len() {
        let event = &events[index];
        let label = format!("{name} event {index}");
        let group = event["group"].as_str().unwrap_or("");
        let kind = event["kind"].as_str().unwrap();
        match (kind, arm) {
            ("frame", _) => {
                let ctx = frame_context(&event["input"]);
                let emitting = case.emitting;
                let _ = advance_frame(&mut case.system_mut().0, f(&event["input"]["dtBits"]), emitting, &ctx, |_| {});
                compare(case.system(), event, true, tally, &label);
            }
            // The seed pass on a playing system: one entry call for the
            // native stop and clear, setter and Play, compared after the Play.
            ("stopClear", SeedArm::Entry) if group == "setEffectInstance" => {
                assert_eq!(events[index + 1]["kind"], "seed");
                assert_eq!(events[index + 2]["kind"], "play");
                let pass = played::seed_played_object(&mut case.world, &case.binding, &[case.node]).unwrap();
                assert_eq!(pass, played::SeedPass { seeded: 0, restarted: 1, unsimulated: 0 }, "{label}");
                case.emitting = true;
                index += 2;
                let play = &events[index];
                let label = format!("{name} event {index}");
                compare_play(case.system(), play, tally, &label);
                compare_state(&case, play, tally, &label);
            }
            ("seed", SeedArm::Entry) if group == "setEffectInstance" => {
                let pass = played::seed_played_object(&mut case.world, &case.binding, &[case.node]).unwrap();
                assert_eq!(pass, played::SeedPass { seeded: 1, restarted: 0, unsimulated: 0 }, "{label}");
                compare_state(&case, event, tally, &label);
            }
            ("stopClear", SeedArm::Entry) => {
                assert_eq!(played::stop_and_clear_object(&mut case.world, &case.binding), 1);
                case.emitting = false;
                compare_state(&case, event, tally, &label);
            }
            ("stop", SeedArm::Entry) => {
                played::stop_object(&mut case.world, &case.binding);
                case.emitting = false;
                compare_state(&case, event, tally, &label);
            }
            ("play", SeedArm::Entry) => {
                played::play_object(&mut case.world, &case.binding).unwrap();
                case.emitting = true;
                compare_play(case.system(), event, tally, &label);
                compare_state(&case, event, tally, &label);
            }
            ("seed", _) => {
                seed_step(&mut case, word(&event["value"]), arm);
                compare_state(&case, event, tally, &label);
            }
            ("stopClear", _) => {
                if arm == SeedArm::StopWithoutClear {
                    host::stop_emitting(&mut case.world, case.root);
                } else {
                    assert_eq!(host::stop_and_clear(&mut case.world, &[case.draw]), 1);
                }
                case.emitting = false;
                compare_state(&case, event, tally, &label);
            }
            ("stop", _) => {
                host::stop_emitting(&mut case.world, case.root);
                case.emitting = false;
                compare_state(&case, event, tally, &label);
            }
            ("play", _) => {
                host::play_draws(&mut case.world, &[case.draw]).unwrap();
                case.emitting = true;
                compare_play(case.system(), event, tally, &label);
                compare_state(&case, event, tally, &label);
            }
            (other, _) => panic!("{label}: event kind {other}"),
        }
        index += 1;
    }
}

fn replay_seed(receipt: &Value, arm: SeedArm) -> Tally {
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    let mut tally = Tally::default();
    for case in receipt["cases"].as_array().unwrap() {
        run_seed_case(case, arm, &mut tally);
    }
    tally
}

#[test]
#[ignore = "MOLY_SEED_ZERO_RECEIPT must identify the JP seed-zero receipt"]
fn seed_pass_and_stop_clear_match_native_rows() {
    let receipt = read("MOLY_SEED_ZERO_RECEIPT");
    let report_of = |tally: &Tally| json!({"events": tally.frames, "mismatched": tally.mismatched_frames,
        "fields": tally.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "firstMismatches": tally.first});
    let entry = replay_seed(&receipt, SeedArm::Entry);
    let steps = replay_seed(&receipt, SeedArm::Steps);
    let mut report = json!({"summary": receipt["summary"], "entry": report_of(&entry), "steps": report_of(&steps)});
    // The receipt's own signal: the setter turned an automatic owner manual
    // without a reset, a stop and clear emptied a live system, the Play after
    // it reset the seeds, and the no-setter control drew differently.
    let summary = receipt["summary"].as_array().unwrap();
    let total = |key: &str| summary.iter().map(|s| s[key].as_u64().unwrap()).sum::<u64>();
    assert!(total("resetSeeds") > 0 && total("clears") > 0, "{report}");
    assert!(summary.iter().flat_map(|s| s["seeds"].as_array().unwrap())
        .any(|seed| seed[1] == 1 && seed[3] == 0), "{report}");
    assert!(receipt["noSeedControlDifferingFrames"].as_u64().unwrap() > 0, "{report}");
    assert!(receipt["staticInitializers"]["playingAutoLocalVersusIdentityClearedDifferingFrames"].as_u64().unwrap() > 0);
    let mut mutants = serde_json::Map::new();
    for arm in [SeedArm::KeepAuto, SeedArm::SetterResets, SeedArm::StopWithoutClear, SeedArm::NoSeed] {
        let tally = replay_seed(&receipt, arm);
        mutants.insert(format!("{arm:?}"), json!([tally.frames, tally.mismatched_frames, tally.first.first()]));
        assert!(tally.mismatched_frames > 0, "arm {arm:?} must mismatch: {report}");
    }
    report["mutants"] = Value::Object(mutants);
    if let Some(path) = std::env::var_os("MOLY_SEED_ZERO_BITFLIP") {
        let control: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let tally = replay_seed(&control, SeedArm::Entry);
        report["bitflip"] = json!([tally.frames, tally.mismatched_frames, tally.first.first()]);
        assert!(tally.mismatched_frames > 0, "the bit-flipped receipt must mismatch: {report}");
    }
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_SEED_ZERO_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(entry.frames > 0 && steps.frames > 0, "{report}");
    assert_eq!((entry.mismatched_frames, steps.mismatched_frames), (0, 0), "{report}");
}
