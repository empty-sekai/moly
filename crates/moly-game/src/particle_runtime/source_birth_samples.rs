//! Actual source JSON through the installed native runtime route. The explicit
//! native probe RNG words below are observed initialized state, never a claim
//! that synthetic entropy equals the original client's automatic seed owner.
use super::*;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::{json, Value};

fn number(value: &Value) -> f32 {
    value.as_f64().expect("native number") as f32
}

fn explicit_initial(value: &Value) -> ModuleRandom {
    assert_eq!(value.as_array().unwrap().len(), 16);
    ModuleRandom {
        words: std::array::from_fn(|word| {
            std::array::from_fn(|lane| value[word * 4 + lane].as_u64().unwrap() as u32)
        }),
    }
}

fn explicit_emission(value: &Value) -> ScalarRandom {
    assert_eq!(value.as_array().unwrap().len(), 4);
    ScalarRandom {
        words: std::array::from_fn(|i| value[i].as_u64().unwrap() as u32),
    }
}

fn source_runtime(root: &std::path::Path, source: &Value) -> Runtime {
    let phenomenon = source["phenomenon"].as_str().unwrap();
    let effect = source["effect"].as_str().unwrap();
    let node = source["node"].as_str().unwrap();
    let bytes = std::fs::read(
        root.join("phenomena")
            .join(phenomenon)
            .join("fx/effects.json"),
    )
    .unwrap();
    let raw: Value = serde_json::from_slice(&bytes).unwrap();
    let particle = raw["effects"][effect]["particles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["node"] == node)
        .expect("actual source particle");
    assert_eq!(particle["systemPathId"], source["systemPathId"]);
    assert_eq!(particle["rendererPathId"], source["rendererPathId"]);
    let mut enabled: Vec<_> = particle["system"]["sourceModules"]["enabled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    enabled.sort_unstable();
    assert_eq!(
        enabled,
        ["CustomDataModule", "EmissionModule", "InitialModule"]
    );
    // Isolate an unchanged source entry because another effect's unsupported
    // schema must not rewrite or prevent decoding this one fully retained entry.
    let selected = json!({"effects":{effect:{"particles":[particle]}}});
    let mut decoded =
        moly_law::particle::schema::Effects::from_json_str(&serde_json::to_vec(&selected).unwrap())
            .unwrap();
    assert_eq!(decoded.emitters.len(), 1);
    let emitter = decoded.emitters.remove(0);
    let mut system = test_support::runtime();
    system.node = emitter.node.clone();
    system.effect = emitter.effect.clone();
    system.kind = EffectKind::Sky;
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier);
    system.custom_law = emitter
        .custom_data
        .as_ref()
        .map(moly_law::particle::custom_data::CustomData::from_params);
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.prewarmed = false;
    system.born_total = 0;
    system.died_total = 0;
    system.full_total = 0;
    system.refused_total = 0;
    system.native_birth = None;
    system
}

fn exact(actual: f32, expected: &Value, step: usize, field: &str) {
    let expected = number(expected);
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "step {step} {field}: {actual} vs {expected}"
    );
}

#[test]
#[ignore = "MOLY_PARTICLE_BIRTH_SOURCE_DIR must identify current private source and native receipt"]
fn source_json_installed_birth_matches_current_native_three_frames() {
    let root =
        std::path::PathBuf::from(std::env::var_os("MOLY_PARTICLE_BIRTH_SOURCE_DIR").unwrap());
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(root.join("autonomous-birth-native.json")).unwrap())
            .unwrap();
    assert_eq!(
        receipt["sourceSha256"],
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
    );
    let rows = receipt["sourceReplay"]["rows"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| {
            r["source"]["phenomenon"] == "009_meteorshower"
                && r["source"]["node"] == "root/sky_star_milky_1"
        })
        .unwrap();
    let mut system = source_runtime(&root, &row["source"]);
    assert!(system.emitter.play_on_awake && !system.emitter.prewarm);
    assert_eq!(system.emitter.shape_enabled, Some(false));
    assert_eq!(system.emitter.ring_buffer_mode, RingBufferMode::Disabled);
    assert_eq!(system.emitter.simulation_space, SimulationSpace::Local);
    assert!(
        matches!(system.emitter.start.lifetime, moly_law::particle::MinMaxCurve::Constant(v) if v == f32::INFINITY)
    );
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(install_native_birth(&mut system, &mut manager).unwrap());
    let state = system
        .native_birth
        .as_mut()
        .expect("installed runtime native branch");
    // Replace only probe state, after exercising the production installer. The
    // probe writes these independent raw RNG states, not ResetSeeds(owner=17).
    state.owner = None;
    state.initial = explicit_initial(&row["initialRngWords"]["initial"]);
    state.emission.random = explicit_emission(&row["initialRngWords"]["emission"]);
    let legacy_rng = system.rng.0;
    let ctx = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let steps = row["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    let mut particle_frames = 0;
    let mut custom_channels = 0;
    for (index, step) in steps.iter().enumerate() {
        let state = system.native_birth.as_ref().unwrap();
        assert_eq!(
            state.initial,
            explicit_initial(&step["rngBefore"]["initial"])
        );
        assert_eq!(
            state.emission.random,
            explicit_emission(&step["rngBefore"]["emission"])
        );
        let start = step["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["phase"] == "startParticles")
            .unwrap();
        simulate(&mut system, number(&start["dt"]), &ctx);
        let after = &step["after"];
        let channels = &step["channels"];
        let count = after["count"].as_u64().unwrap() as usize;
        assert_eq!(system.pool.len(), count);
        assert_eq!(system.side.len(), count);
        assert_eq!(system.refused_total, 0);
        assert_eq!(
            system.rng.0, legacy_rng,
            "installed native route must leave legacy RNG alone"
        );
        exact(system.playback_head, &after["clock"], index, "clock");
        for particle in 0..count {
            particle_frames += 1;
            let p = &system.pool[particle];
            let side = &system.side[particle];
            assert_eq!(side.seed, after["seeds"][particle].as_u64().unwrap() as u32);
            exact(p.age_percent, &after["age"][particle], index, "age");
            exact(
                p.inverse_lifetime,
                &after["inverseLifetime"][particle],
                index,
                "inverseLifetime",
            );
            assert_eq!(p.start_lifetime, f32::INFINITY);
            for axis in 0..3 {
                // Reflection can negate zero. These source position/velocity
                // values are numerical zero in all frames, unlike age bits.
                assert_eq!(
                    p.position[axis],
                    number(&after["positions"][particle][axis])
                );
                assert_eq!(
                    p.velocity[axis],
                    number(&after["velocities"][particle][axis])
                );
                exact(
                    side.rot[axis],
                    &channels["rotation"][particle][axis],
                    index,
                    "rotation",
                );
                // Native 1D size has only X storage. Runtime explicitly expands
                // its render-facing side value; absent native Y/Z zeros are not
                // authored dimensions and must not be mistaken for size 0.
                let source_axis = if system.emitter.start.size3d { axis } else { 0 };
                exact(
                    side.size[axis],
                    &channels["size"][particle][source_axis],
                    index,
                    "size",
                );
            }
            let actual_color = moly_law::particle::gradient::quantize_rgba8(side.colour);
            for channel in 0..4 {
                assert_eq!(
                    actual_color[channel],
                    channels["colorBytes"][particle][channel].as_u64().unwrap() as u8
                );
            }
            for stream in 0..2 {
                for channel in 0..4 {
                    exact(
                        side.custom_data[stream][channel],
                        &channels["custom"][particle][stream * 4 + channel],
                        index,
                        "custom",
                    );
                    custom_channels += 1;
                }
            }
        }
        let state = system.native_birth.as_ref().unwrap();
        assert_eq!(
            state.initial,
            explicit_initial(&step["rngAfter"]["initial"])
        );
        assert_eq!(
            state.emission.random,
            explicit_emission(&step["rngAfter"]["emission"])
        );
        for (axis, value) in [
            state.emission.distribution.spacing,
            state.emission.distribution.offset,
            state.emission.distribution.burst_fraction,
        ]
        .into_iter()
        .enumerate()
        {
            exact(value, &after["carry"][axis], index, "emission distribution");
        }
    }
    assert_eq!((particle_frames, custom_channels), (3, 24));
    assert_eq!(system.born_total, 1);
    let other = rows
        .iter()
        .find(|r| r["source"]["phenomenon"] == "014_sekai")
        .unwrap();
    let mut other_system = source_runtime(&root, &other["source"]);
    assert!(other_system.emitter.prewarm);
    assert_eq!(
        other_system.emitter.ring_buffer_mode,
        RingBufferMode::LoopUntilReplaced
    );
    assert!(!install_native_birth(&mut other_system, &mut manager).unwrap());
    assert!(other_system.native_birth.is_none());
    let report = json!({"source":row["source"],"sourceFrames":3,"particleFrames":particle_frames,
        "customChannels":custom_channels,"failureCount":0,"comparison":"Exact scalar bits and all captured Initial/Emission RNG words; zero vector signs ignored; 1D native size X expanded for runtime render side",
        "route":"actual effects.json -> Effects/EmitterParams -> install_native_birth -> simulate -> installed native step",
        "scope":"Initialized source 009 only, captured probe RNG overwritten after installer, identity owner, all enabled simulation modules retained. Shared OS manager order across nonqualified systems and original-client entropy/world/renderer are not proven. Source 014 prewarm/ringLoop remains refused."});
    if let Some(path) = std::env::var_os("MOLY_PARTICLE_BIRTH_SOURCE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
}
