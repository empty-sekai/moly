//! Bounded full source snow replay against current JP native incremental slices.
//! Opt-in receipt test. It drives the production native step, the production
//! first-Play slice plan and the production eligibility check; only the owner
//! seed and the initial streams are supplied from the receipt.
use super::*;
use moly_law::particle::{
    autonomous_emission::AutonomousEmissionState,
    schema::Effects,
    seed_owner::{ModuleRandom, ScalarRandom, SeedOwner},
};
use serde_json::{json, Value};

const SOURCE_HASH: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";
const EFFECT: &str = "fx_env_sky_010_snow";
const NODE: &str = "root/snow_pt_01";

fn f(value: &Value) -> f32 {
    value.as_f64().expect("native f32") as f32
}
fn u(value: &Value) -> u32 {
    value.as_u64().expect("native u32") as u32
}
fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("words").iter().map(u).collect()
}
fn module(value: &Value) -> ModuleRandom {
    let raw = words(value);
    assert_eq!(raw.len(), 16);
    ModuleRandom {
        words: std::array::from_fn(|w| std::array::from_fn(|lane| raw[w * 4 + lane])),
    }
}
fn scalar(value: &Value) -> ScalarRandom {
    let raw = words(value);
    ScalarRandom {
        words: raw.try_into().expect("four scalar words"),
    }
}
fn module_words(module: &ModuleRandom) -> Vec<u32> {
    (0..16).map(|i| module.words[i / 4][i % 4]).collect()
}
fn context() -> Context {
    Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    }
}

fn source_runtime(native_path: &std::path::Path, native: &Value) -> Runtime {
    // Both source inputs are supplied explicitly next to the native receipt.
    let _ = native_path;
    let raw_path = std::path::PathBuf::from(
        std::env::var_os("MOLY_SNOW_FULL_SOURCE_EFFECTS")
            .expect("MOLY_SNOW_FULL_SOURCE_EFFECTS must name the source effects.json"),
    );
    let overlay_path = std::path::PathBuf::from(
        std::env::var_os("MOLY_SNOW_FULL_OVERLAY_EFFECTS")
            .expect("MOLY_SNOW_FULL_OVERLAY_EFFECTS must name the overlay effects.json"),
    );
    let raw: Value = serde_json::from_slice(&std::fs::read(&raw_path).unwrap_or_else(|error| {
        panic!("read source effects {}: {error}", raw_path.display())
    }))
    .unwrap();
    let overlay: Value =
        serde_json::from_slice(&std::fs::read(&overlay_path).unwrap_or_else(|error| {
            panic!("read overlay effects {}: {error}", overlay_path.display())
        }))
        .unwrap();
    let find = |doc: &Value| {
        doc["effects"][EFFECT]["particles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["node"] == NODE)
            .unwrap()
            .clone()
    };
    let original = find(&raw);
    let selected = find(&overlay);
    let mut pointer_overlay = selected.clone();
    pointer_overlay["system"]["subEmitters"][0]
        .as_object_mut()
        .unwrap()
        .remove("sourcePointer");
    assert_eq!(
        pointer_overlay, original,
        "overlay must only retain original authored PPtr"
    );
    assert_eq!(
        selected["system"]["subEmitters"][0]["sourcePointer"],
        json!({"fileId":0,"pathId":"0"})
    );
    let module_names: Vec<_> = selected["system"]["sourceModules"]["enabled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(module_names.len(), 10);
    for required in [
        "InitialModule",
        "EmissionModule",
        "ShapeModule",
        "VelocityModule",
        "RotationModule",
        "SizeModule",
        "NoiseModule",
        "CustomDataModule",
        "ColorModule",
        "SubModule",
    ] {
        assert!(module_names.contains(&required), "missing {required}");
    }
    let route = source_route(&selected["system"]);
    // The emitter state the native Shape boundary reads, classified by the
    // production adapter from the authored scaling mode and node chain.
    assert_eq!(selected["renderer"]["renderMode"], "Billboard");
    let by_path: std::collections::HashMap<String, &Value> = overlay["effects"][EFFECT]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["path"].as_str().unwrap().to_owned(), n))
        .collect();
    let scaling = crate::weather_fx::source_scaling(&selected["system"], &by_path, NODE, true)
        .expect("authored snow scaling mode");
    let selected = json!({"effects":{EFFECT:{"particles":[selected]}}});
    let mut decoded = Effects::from_json_str(&serde_json::to_vec(&selected).unwrap()).unwrap();
    let emitter = decoded.emitters.remove(0);
    assert!(decoded.emitters.is_empty());
    assert_eq!(emitter.effect, EFFECT);
    assert_eq!(emitter.node, NODE);
    assert!(emitter.prewarm && !emitter.play_on_awake && emitter.looping);
    assert_eq!(emitter.auto_random_seed, Some(true));
    assert!(emitter.sub_emitters[0].source_pointer.is_authored_null());
    assert_eq!(native["owner"], format!("{EFFECT}/{NODE}"));
    let mut system = test_support::runtime();
    system.geometry = test_support::source_billboard(scaling);
    system.effect = emitter.effect.clone();
    system.node = emitter.node.clone();
    system.kind = EffectKind::Sky;
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).expect("curves validated during admission");
    system.velocity_law = emitter
        .velocity_over_lifetime
        .as_ref()
        .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission"));
    system.rol = emitter.rotation_over_lifetime.as_ref().map(|p| {
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve)
            .unwrap()
    });
    system.size_law = emitter
        .size_over_lifetime
        .as_ref()
        .map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission"));
    system.color_law = emitter
        .color_over_lifetime
        .as_ref()
        .map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = emitter
        .custom_data
        .as_ref()
        .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission"));
    let noise_law = NoiseLaw::from_params(emitter.noise.as_ref().unwrap()).unwrap();
    let owner = SeedOwner::from_serialized(71, true);
    system.noise = Some(NoiseRuntime {
        law: noise_law,
        state: NoiseState { scroll: 0.0 },
        owner_seed: 71,
        owner,
    });
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.born_total = 0;
    system.died_total = 0;
    system.full_total = 0;
    system.native_birth = None;
    // Production routes this composition natively: ordinary route, every
    // enabled module with a native consumer, authored-null child edge.
    assert_eq!(route, SourceRoute::Ordinary);
    native_birth_eligible(&system.emitter, &route)
        .expect("production admits the snow composition on the native birth path");
    system
}

// These are the native source-coordinate logical channels, not unused SIMD
// padding, expanded sizeY/Z, or the separate effective-size array the particle
// storage keeps apart from them.
fn particle_words(system: &Runtime) -> Vec<[u32; 15]> {
    system
        .pool
        .iter()
        .zip(&system.side)
        .map(|(p, s)| {
            let pos = crate::particle_geometry::reflect(Vec3::from_array(p.position)).to_array();
            let vel = crate::particle_geometry::reflect(Vec3::from_array(p.velocity)).to_array();
            [
                pos[0].to_bits(),
                pos[1].to_bits(),
                pos[2].to_bits(),
                vel[0].to_bits(),
                vel[1].to_bits(),
                vel[2].to_bits(),
                p.age_percent.to_bits(),
                p.inverse_lifetime.to_bits(),
                s.seed,
                s.rot[2].to_bits(),
                s.size[0].to_bits(),
                s.custom_data[0][0].to_bits(),
                s.custom_data[0][1].to_bits(),
                s.custom_data[0][2].to_bits(),
                s.custom_data[0][3].to_bits(),
            ]
        })
        .collect()
}
fn digest(rows: &[[u32; 15]]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for row in rows {
        for word in row {
            for byte in word.to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    }
    format!("{hash:016x}")
}
fn compare_field(
    actual: impl Into<Value>,
    expected: &Value,
    name: &str,
    at: usize,
    differences: &mut Vec<Value>,
) {
    let actual: Value = actual.into();
    if &actual != expected && differences.len() < 32 {
        differences.push(json!({"slice":at,"field":name,"actual":actual,"native":expected}));
    }
}
fn compare_boundary(
    system: &Runtime,
    state: &birth::NativeBirthState,
    expected: &Value,
    at: usize,
    differences: &mut Vec<Value>,
    hash_differences: &mut Vec<Value>,
) -> String {
    let actual_rows = particle_words(system);
    let hash = digest(&actual_rows);
    compare_field(
        system.pool.len(),
        &expected["count"],
        "count",
        at,
        differences,
    );
    compare_field(
        system.playback_head.to_bits(),
        &json!(f(&expected["clock"]).to_bits()),
        "clockBits",
        at,
        differences,
    );
    for (i, actual) in [
        state.emission.distribution.spacing,
        state.emission.distribution.offset,
        state.emission.distribution.burst_fraction,
    ]
    .into_iter()
    .enumerate()
    {
        compare_field(
            actual.to_bits(),
            &json!(f(&expected["carry"][i]).to_bits()),
            &format!("carry[{i}]Bits"),
            at,
            differences,
        );
    }
    compare_field(
        system.noise.clone().unwrap().state.scroll.to_bits(),
        &json!(f(&expected["noiseScroll"]).to_bits()),
        "noiseScrollBits",
        at,
        differences,
    );
    compare_field(
        module_words(&state.initial),
        &expected["initialWords"],
        "initialWords",
        at,
        differences,
    );
    compare_field(
        module_words(&state.shape),
        &expected["shapeWords"],
        "shapeWords",
        at,
        differences,
    );
    compare_field(
        state.emission.random.words.to_vec(),
        &expected["scalarEmissionWords"],
        "scalarEmissionWords",
        at,
        differences,
    );
    if expected["fnv1a64"] != hash {
        hash_differences.push(json!({"slice":at,"actual":hash,"native":expected["fnv1a64"]}));
    }
    hash
}

fn compare_particle_channels(actual: &[[u32; 15]], native: &Value, stage: &str) -> Value {
    let expected = native.as_array().unwrap();
    let names = [
        "positionX",
        "positionY",
        "positionZ",
        "velocityX",
        "velocityY",
        "velocityZ",
        "age",
        "inverseLifetime",
        "seed",
        "rotationZ",
        "sizeX",
        "custom1X",
        "custom1Y",
        "custom1Z",
        "custom1W",
    ];
    let mut counts = [0usize; 15];
    let mut examples = Vec::new();
    for (particle, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let expected = expected.as_array().unwrap();
        assert_eq!(expected.len(), 15);
        for channel in 0..15 {
            let want = u(&expected[channel]);
            if actual[channel] != want {
                counts[channel] += 1;
                if examples.len() < 80 {
                    examples.push(json!({"particle":particle,"channel":names[channel],
                        "actualBits":format!("{:08x}",actual[channel]),
                        "nativeBits":format!("{want:08x}"),
                        "actualFloat":f32::from_bits(actual[channel]),"nativeFloat":f32::from_bits(want)}));
                }
            }
        }
    }
    json!({"stage":stage,"runtimeCount":actual.len(),"nativeCount":expected.len(),
        "channelMismatchCounts":names.iter().zip(counts).map(|(name,count)|json!({"channel":name,"count":count})).collect::<Vec<_>>(),
        "firstDifferences":examples})
}

fn compare_effective_size(actual: &[u32], expected: &Value, stage: &str) -> Value {
    let expected = expected.as_array().unwrap();
    let mut count = 0;
    let mut examples = Vec::new();
    for (index, (&actual, want)) in actual.iter().zip(expected).enumerate() {
        let want = u(want);
        if actual != want {
            count += 1;
            if examples.len() < 32 {
                examples.push(
                    json!({"particle":index,"actualBits":format!("{actual:08x}"),
                    "nativeBits":format!("{want:08x}")}),
                );
            }
        }
    }
    json!({"stage":stage,"mismatchCount":count,"runtimeCount":actual.len(),
        "nativeCount":expected.len(),"firstDifferences":examples})
}

#[test]
#[ignore = "MOLY_SNOW_FULL_NATIVE must identify current JP 6.8.1 source/native receipt"]
fn source_snow_full_prewarm_matches_current_native() {
    let path = std::path::PathBuf::from(std::env::var_os("MOLY_SNOW_FULL_NATIVE").unwrap());
    let native: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(native["sourceSha256"], SOURCE_HASH);
    assert_eq!(
        native["effectsSha256"],
        "7757dd3d7b05d8552770bc9bbd7de51cf0eea143daec9dbbd1af322411b683f4"
    );
    // Production follows the first-Play warm with the effector's later Play, so
    // the normal frame is compared against the native case that includes it.
    let row = native["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["label"] == "Full-12s-prewarm+second-Play")
        .expect("native receipt with the effector's second Play");
    assert_eq!(row["status"], "native_return");
    assert_eq!(
        row["logicalChannels"],
        json!([
            "positionX",
            "positionY",
            "positionZ",
            "velocityX",
            "velocityY",
            "velocityZ",
            "age",
            "inverseLifetime",
            "seed",
            "rotationZ",
            "sizeX",
            "custom1X",
            "custom1Y",
            "custom1Z",
            "custom1W"
        ])
    );
    let mut system = source_runtime(&path, &native);
    let initial = &row["prewarmInputBoundary"];
    let mut state = birth::NativeBirthState {
        owner: Some(SeedOwner::from_serialized(71, true)),
        initial: module(&initial["initialWords"]),
        shape: module(&initial["shapeWords"]),
        shape_clock: Default::default(),
        emission: AutonomousEmissionState::initialized(scalar(&initial["scalarEmissionWords"])),
        frame: Default::default(),
        events: None,
        target: None,
    };
    state.emission.distribution.spacing = f(&initial["carry"][0]);
    state.emission.distribution.offset = f(&initial["carry"][1]);
    state.emission.distribution.burst_fraction = f(&initial["carry"][2]);
    system.playback_head = f(&initial["clock"]);
    system.noise.as_mut().unwrap().state.scroll = f(&initial["noiseScroll"]);
    let mut differences = Vec::new();
    let mut hash_differences = Vec::new();
    compare_boundary(
        &system,
        &state,
        initial,
        0,
        &mut differences,
        &mut hash_differences,
    );
    let slices = row["prewarmSlices"].as_array().unwrap();
    // The production first-Play plan: 12 s upper lifetime, start clock 0.
    let plan = first_play_plan(&system).unwrap();
    assert_eq!(plan.compute_out().to_bits(), 12.0_f32.to_bits());
    assert_eq!(plan.initial_clock().to_bits(), 0.0_f32.to_bits());
    let plan: Vec<_> = plan.collect();
    assert_eq!(plan.len(), slices.len(), "first-Play plan must yield exactly the native slices");
    let context = context();
    let mut rows = Vec::with_capacity(slices.len());
    for (index, (planned, reference)) in plan.into_iter().zip(slices).enumerate() {
        let planned = planned.unwrap();
        compare_field(
            planned.remaining_before.to_bits(),
            &json!(f(&reference["remaining"]).to_bits()),
            "remainingBits",
            index,
            &mut differences,
        );
        compare_field(
            planned.duration.to_bits(),
            &json!(f(&reference["dt"]).to_bits()),
            "dtBits",
            index,
            &mut differences,
        );
        birth::step_explicit(&mut system, &mut state, planned.duration, false, &context)
            .unwrap_or_else(|e| panic!("native snow slice {index} refused: {e:?}"));
        let hash = compare_boundary(
            &system,
            &state,
            &reference["after"],
            index + 1,
            &mut differences,
            &mut hash_differences,
        );
        rows.push(json!({"slice":index,"count":system.pool.len(),"fnv1a64":hash}));
    }
    assert_eq!(slices.len(), 174);
    let prewarm_channels = compare_particle_channels(
        &particle_words(&system),
        &row["prewarmFinalParticleWords"],
        "prewarm",
    );
    let effective: Vec<u32> = system
        .pool
        .iter()
        .zip(&system.side)
        .map(|(p, s)| motion::size_at_age_percent(&system, s, p.age_percent)[0].to_bits())
        .collect();
    let prewarm_effective = compare_effective_size(
        &effective,
        &row["prewarmEffectiveSizeXPoolWords"],
        "prewarm",
    );
    later_play(&mut system);
    birth::step_explicit(&mut system, &mut state, 1.0 / 60.0, false, &context).unwrap();
    let normal = &row["firstNormalFrame"];
    compare_boundary(
        &system,
        &state,
        &normal["logicalDigest"],
        175,
        &mut differences,
        &mut hash_differences,
    );
    let normal_channels =
        compare_particle_channels(&particle_words(&system), &normal["particleWords"], "normal");
    let effective: Vec<u32> = system
        .pool
        .iter()
        .zip(&system.side)
        .map(|(p, s)| motion::size_at_age_percent(&system, s, p.age_percent)[0].to_bits())
        .collect();
    let normal_effective =
        compare_effective_size(&effective, &normal["effectiveSizeXPoolWords"], "normal");
    let report = json!({"owner":format!("{EFFECT}/{NODE}"),"sourceSha256":SOURCE_HASH,
        "scope":"Supplied owner 71 driven through the production native step, first-Play plan and eligibility check; no live Play lifecycle or renderer claim",
        "slices":rows,"finalCount":system.pool.len(),"boundaryDifferenceCount":differences.len(),
        "boundaryDifferences":differences,"hashDifferenceCount":hash_differences.len(),
        "hashDifferences":hash_differences,"prewarmChannels":prewarm_channels,
        "prewarmEffectiveSize":prewarm_effective,"normalChannels":normal_channels,
        "normalEffectiveSize":normal_effective});
    if let Some(output) = std::env::var_os("MOLY_SNOW_FULL_REPORT") {
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty()
            && hash_differences.is_empty()
            && prewarm_channels["firstDifferences"]
                .as_array()
                .unwrap()
                .is_empty()
            && prewarm_effective["mismatchCount"] == 0
            && normal_channels["firstDifferences"]
                .as_array()
                .unwrap()
                .is_empty()
            && normal_effective["mismatchCount"] == 0,
        "snow full native mismatch, see MOLY_SNOW_FULL_REPORT: {}",
        json!({"boundary":differences,
            "hashCount":hash_differences.len(),"prewarmChannels":prewarm_channels["channelMismatchCounts"],
            "normalChannels":normal_channels["channelMismatchCounts"]})
    );
}
