//! External current-lib ordinary birth observations, through the qualified
//! birth entry only. No legacy `simulate` call and no generated reference data.
use super::*;
use moly_law::particle::seed_owner::ModuleRandom;
use moly_law::particle::sub_emission::{BirthBatch, BirthDistribution};
use moly_law::particle::{MinMaxCurve, RingBufferMode};
use serde_json::{Value, json};

fn number(value: &Value) -> f32 {
    value.as_f64().expect("native numeric observation") as f32
}

fn vector(value: &Value) -> [f32; 3] {
    std::array::from_fn(|axis| number(&value[axis]))
}

fn source_to_runtime(value: &Value) -> [f32; 3] {
    crate::particle_geometry::reflect(Vec3::from_array(vector(value))).to_array()
}

fn runtime_to_source(value: [f32; 3]) -> [f32; 3] {
    crate::particle_geometry::reflect(Vec3::from_array(value)).to_array()
}

fn read_receipt() -> Value {
    let path = std::env::var("MOLY_PARTICLE_BIRTH_SAMPLES")
        .expect("MOLY_PARTICLE_BIRTH_SAMPLES must identify autonomous-birth-native.json");
    let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        receipt["sourceSha256"],
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
    );
    receipt
}

/// Probe setup_child writes these sixteen words directly. They are not
/// ModuleRandom::from_owner_seed(17), which is a different native operation.
fn explicit_probe_random() -> ModuleRandom {
    ModuleRandom {
        words: [
            [17, 18, 19, 20],
            [21, 22, 23, 24],
            [25, 26, 27, 28],
            [29, 30, 31, 32],
        ],
    }
}

fn import_before(system: &mut Runtime, before: &Value, lifetime: f32) {
    let count = before["count"].as_u64().unwrap() as usize;
    system.pool = (0..count)
        .map(|index| Particle {
            position: source_to_runtime(&before["positions"][index]),
            velocity: source_to_runtime(&before["velocities"][index]),
            start_lifetime: lifetime,
            age_percent: number(&before["age"][index]),
            inverse_lifetime: number(&before["inverseLifetime"][index]),
        })
        .collect();
    let mut side = system.side[0];
    side.rand = 0.0;
    side.rot = [0.0; 3];
    side.size = [1.0; 3];
    side.gravity = 0.0;
    side.colour = [1.0; 4];
    side.total_velocity = [0.0; 3];
    side.custom_data = [[0.0; 4]; 2];
    system.side = (0..count)
        .map(|index| Side {
            seed: before["seeds"][index].as_u64().unwrap() as u32,
            ..side
        })
        .collect();
    system.born_total = count as u64;
    system.died_total = 0;
    system.full_total = 0;
    system.refused_total = 0;
    system.playback_head = number(&before["clock"]);
    system.previous_head = system.playback_head;
    system.emission_started = false;
}

fn compare_after(system: &Runtime, expected: &Value, index: usize, failures: &mut Vec<Value>) {
    let count = expected["count"].as_u64().unwrap() as usize;
    if system.pool.len() != count || system.side.len() != count {
        failures.push(json!({"case":index,"field":"count","actualPool":system.pool.len(),"actualSide":system.side.len(),"expected":count}));
        return;
    }
    for particle in 0..count {
        let actual = &system.pool[particle];
        for (field, actual, expected) in [
            (
                "positions",
                runtime_to_source(actual.position),
                vector(&expected["positions"][particle]),
            ),
            (
                "velocities",
                runtime_to_source(actual.velocity),
                vector(&expected["velocities"][particle]),
            ),
        ] {
            // Reflection may change the sign of a zero. All nonzero source
            // positions/velocities here are exact binary fractions.
            if actual != expected || actual.iter().any(|v| !v.is_finite()) {
                failures.push(json!({"case":index,"particle":particle,"field":field,"actual":actual,"expected":expected}));
            }
        }
        for (field, value) in [
            ("age", actual.age_percent),
            ("inverseLifetime", actual.inverse_lifetime),
        ] {
            let expected_bits = number(&expected[field][particle]).to_bits();
            if value.to_bits() != expected_bits {
                failures.push(json!({"case":index,"particle":particle,"field":field,"actualBits":value.to_bits(),"expectedBits":expected_bits}));
            }
        }
        let expected_seed = expected["seeds"][particle].as_u64().unwrap() as u32;
        if system.side[particle].seed != expected_seed {
            failures.push(json!({"case":index,"particle":particle,"field":"seed","actual":system.side[particle].seed,"expected":expected_seed}));
        }
    }
}

#[test]
#[ignore = "MOLY_PARTICLE_BIRTH_SAMPLES must identify current native observations"]
fn explicit_birth_matches_current_native_start_particles() {
    let receipt = read_receipt();
    let rows = receipt["startReplay"]["examples"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        2,
        "receipt stores two complete examples, not all 160 probe cases"
    );
    let context = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let mut failures = Vec::new();
    let mut compared_particles = 0;
    for (index, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let before = &row["observed"]["before"];
        let after = &row["observed"]["after"];
        let mut system = test_support::runtime();
        system.emitter.duration = number(&input["duration"]);
        system.emitter.max_particles = 32;
        system.emitter.prewarm = false;
        system.prewarmed = true;
        system.emitter.ring_buffer_mode = RingBufferMode::Disabled;
        system.emitter.start.lifetime = MinMaxCurve::Constant(number(&input["lifetime"]));
        system.emitter.start.speed = MinMaxCurve::Constant(number(&input["speed"]));
        system.emitter.start.size = MinMaxCurve::Constant(1.0);
        system.emitter.start.rotation = MinMaxCurve::Constant(0.0);
        system.emitter.simulation_space = SimulationSpace::Local;
        system.emitter.shape = None;
        system.emitter.shape_enabled = Some(false);
        import_before(&mut system, before, 2.0);
        assert_eq!(system.pool.len(), input["old"].as_u64().unwrap() as usize);
        let batch = BirthBatch {
            count: input["count"].as_u64().unwrap() as u32,
            rate_count: input["rate"].as_u64().unwrap() as u32,
            distribution: BirthDistribution {
                spacing: number(&input["distribution"][0]),
                offset: number(&input["distribution"][1]),
                burst_fraction: number(&input["distribution"][2]),
            },
        };
        let dt = number(&input["dt"]);
        let current = number(&input["current"]);
        let duration = number(&input["duration"]);
        let mut random = explicit_probe_random();
        super::birth::start_explicit(
            &mut system,
            &mut random,
            batch,
            dt,
            (current - dt) / duration,
            current / duration,
            &context,
        )
        .unwrap();
        compared_particles += system.pool.len();
        compare_after(&system, after, index, &mut failures);
    }
    let report = json!({
        "directExamples":rows.len(),"comparedParticles":compared_particles,
        "failureCount":failures.len(),"firstFailures":failures.iter().take(12).collect::<Vec<_>>(),
        "meaning":"Qualified explicit Initial/StartVelocity/birth simulation/packing only; no normal Update1, automatic seed owner, hierarchy or renderer equivalence claim."
    });
    if let Ok(path) = std::env::var("MOLY_PARTICLE_BIRTH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(failures.is_empty(), "native birth mismatches: {report}");
}

#[test]
#[ignore = "MOLY_PARTICLE_BIRTH_SAMPLES must identify current native observations"]
fn normal_update_recomputes_birth_capacity_after_existing_deaths() {
    use moly_law::particle::autonomous_emission::AutonomousEmissionState;
    use moly_law::particle::emit::{Burst, BurstCycles};
    use moly_law::particle::schema::EmissionParams;
    use moly_law::particle::seed_owner::ScalarRandom;

    let receipt = read_receipt();
    let rows = receipt["normalUpdateReplay"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    let context = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let mut failures = Vec::new();
    let mut survivor_cases = 0;
    let mut death_cases = 0;
    for (index, row) in rows.iter().enumerate() {
        let old = row["old"].as_u64().unwrap() as usize;
        let age = number(&row["age"]);
        let dt = number(&row["dt"]);
        let mut system = test_support::runtime();
        system.emitter.max_particles = old as u32;
        system.emitter.duration = 5.0;
        system.emitter.looping = true;
        system.emitter.prewarm = false;
        system.prewarmed = true;
        system.emitter.start.lifetime = MinMaxCurve::Constant(2.0);
        system.emitter.start.speed = MinMaxCurve::Constant(1.0);
        system.emitter.start.size = MinMaxCurve::Constant(1.0);
        system.emitter.start.rotation = MinMaxCurve::Constant(0.0);
        system.emitter.simulation_space = SimulationSpace::Local;
        system.emitter.ring_buffer_mode = RingBufferMode::Disabled;
        system.emitter.shape = None;
        system.emitter.shape_enabled = Some(false);
        system.emitter.emission = Some(EmissionParams {
            rate_over_time: MinMaxCurve::Constant(0.0),
            rate_over_distance: MinMaxCurve::Constant(0.0),
            bursts: vec![Burst {
                time: 0.0,
                count: MinMaxCurve::Constant(3.0),
                cycles: BurstCycles::from_serialized(1),
                repeat_interval: 0.01,
                probability: 1.0,
            }],
        });
        // These are probe input assignments from update_replay/setup_child,
        // never a reconstruction from the expected after snapshot.
        let before = json!({
            "count":old,"positions":(0..old).map(|i|[i as f32,0.0,0.0]).collect::<Vec<_>>(),
            "velocities":vec![[0.0;3];old],"age":vec![age;old],
            "inverseLifetime":vec![1.0;old],"seeds":vec![0;old],"clock":0.0,
        });
        import_before(&mut system, &before, 1.0);
        let mut state = super::birth::NativeBirthState {
            owner: None,
            initial: explicit_probe_random(),
            shape: explicit_probe_random(),
            emission: AutonomousEmissionState::initialized(ScalarRandom {
                words: [17, 19, 127, 2471805022],
            }),
            frame: Default::default(),
            events: None,
            target: None,
        };
        let initial_before = state.initial;
        super::birth::step_explicit(&mut system, &mut state, dt, false, &context).unwrap();
        compare_after(&system, &row["after"], index, &mut failures);
        if system.playback_head.to_bits() != number(&row["after"]["clock"]).to_bits() {
            failures.push(json!({"case":index,"field":"clock","actual":system.playback_head,"expected":row["after"]["clock"]}));
        }
        let distribution = state.emission.distribution;
        let carry = [
            distribution.spacing,
            distribution.offset,
            distribution.burst_fraction,
        ];
        if carry.map(f32::to_bits) != vector(&row["after"]["carry"]).map(f32::to_bits) {
            failures.push(json!({"case":index,"field":"carry","actual":carry,"expected":row["after"]["carry"]}));
        }
        let born = row["born"].as_u64().unwrap();
        if system.born_total != old as u64 + born {
            failures.push(json!({"case":index,"field":"bornTotal","actual":system.born_total,"expected":old as u64+born}));
        }
        let dead = row["dead"].as_bool().unwrap();
        if dead {
            death_cases += 1;
            assert_ne!(state.initial, initial_before);
        } else {
            survivor_cases += 1;
            assert_eq!(
                state.initial, initial_before,
                "fully occupied pool must not run Initial"
            );
        }
        let expected_deaths = if dead { old as u64 } else { 0 };
        if system.died_total != expected_deaths {
            failures.push(json!({"case":index,"field":"diedTotal","actual":system.died_total,"expected":expected_deaths}));
        }
    }
    assert_eq!((survivor_cases, death_cases), (8, 16));
    let report = json!({
        "normalUpdateCases":rows.len(),"survivorCases":survivor_cases,"deathCases":death_cases,
        "failureCount":failures.len(),"firstFailures":failures.iter().take(12).collect::<Vec<_>>(),
        "meaning":"Initialized normal step with explicit captured RNG words, no Shape/events/prewarm. Count/state observations verify old death before birth capacity; no automatic seed owner or original-client equivalence claim."
    });
    if let Ok(path) = std::env::var("MOLY_PARTICLE_BIRTH_NORMAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(
        failures.is_empty(),
        "native normal birth mismatches: {report}"
    );
}

// Append-only fixture draft. Applied after the parent build has completed.

fn raw_u32(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("raw u32 channel")).unwrap()
}

fn raw_float(value: &Value) -> f32 {
    f32::from_bits(raw_u32(value))
}

fn raw_vector(channels: &Value, family: &str, index: usize) -> [f32; 3] {
    ["X", "Y", "Z"].map(|axis| raw_float(&channels[format!("{family}{axis}")][index]))
}

fn import_raw_before(system: &mut Runtime, channels: &Value, old: usize) {
    for values in channels.as_object().unwrap().values() {
        assert_eq!(values.as_array().unwrap().len(), old);
    }
    let side_template = system.side[0];
    system.pool = (0..old)
        .map(|index| Particle {
            position: crate::particle_geometry::reflect(Vec3::from_array(raw_vector(
                channels, "position", index,
            )))
            .to_array(),
            velocity: crate::particle_geometry::reflect(Vec3::from_array(raw_vector(
                channels, "velocity", index,
            )))
            .to_array(),
            // Probe old particles are initialized with inverseLifetime 0.5.
            // This API-only lifetime field is not compared as native storage.
            start_lifetime: 2.0,
            age_percent: raw_float(&channels["age"][index]),
            inverse_lifetime: raw_float(&channels["inverseLifetime"][index]),
        })
        .collect();
    system.side = (0..old)
        .map(|index| Side {
            rand: 0.0,
            seed: raw_u32(&channels["seed"][index]),
            rot: [0.0, 0.0, raw_float(&channels["rotationZ"][index])],
            size: [raw_float(&channels["sizeX"][index]); 3],
            colour: moly_law::particle::gradient::rgba8_to_float(
                raw_u32(&channels["color"][index]).to_le_bytes(),
            ),
            gravity: 0.0,
            total_velocity: system.pool[index].velocity,
            custom_data: [[0.0; 4]; 2],
            ..side_template
        })
        .collect();
    system.born_total = old as u64;
    system.died_total = 0;
    system.full_total = 0;
    system.refused_total = 0;
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.emission_started = false;
}

fn compare_raw_after(
    system: &Runtime,
    after: &Value,
    case: usize,
    failures: &mut Vec<Value>,
) -> usize {
    let count = after["count"].as_u64().unwrap() as usize;
    let expected = &after["channels"];
    for values in expected.as_object().unwrap().values() {
        assert_eq!(values.as_array().unwrap().len(), count);
    }
    if system.pool.len() != count || system.side.len() != count {
        failures.push(json!({"case":case,"field":"count","actualPool":system.pool.len(),"actualSide":system.side.len(),"expected":count}));
        return 0;
    }
    for index in 0..count {
        let particle = &system.pool[index];
        let side = &system.side[index];
        for (family, values) in [
            ("position", runtime_to_source(particle.position)),
            ("velocity", runtime_to_source(particle.velocity)),
        ] {
            for (axis, value) in ["X", "Y", "Z"].into_iter().zip(values) {
                let key = format!("{family}{axis}");
                let expected_bits = raw_u32(&expected[&key][index]);
                // Handedness reflection can change the sign of zero only.
                if value.to_bits() != expected_bits
                    && !(value == 0.0 && f32::from_bits(expected_bits) == 0.0)
                {
                    failures.push(json!({"case":case,"particle":index,"field":key,"actualBits":value.to_bits(),"expectedBits":expected_bits}));
                }
            }
        }
        for (key, value) in [
            ("age", particle.age_percent),
            ("inverseLifetime", particle.inverse_lifetime),
            ("sizeX", side.size[0]),
            ("rotationZ", side.rot[2]),
        ] {
            let expected_bits = raw_u32(&expected[key][index]);
            if value.to_bits() != expected_bits {
                failures.push(json!({"case":case,"particle":index,"field":key,"actualBits":value.to_bits(),"expectedBits":expected_bits}));
            }
        }
        let packed_color =
            u32::from_le_bytes(moly_law::particle::gradient::quantize_rgba8(side.colour));
        for (key, value) in [("seed", side.seed), ("color", packed_color)] {
            let expected = raw_u32(&expected[key][index]);
            if value != expected {
                failures.push(json!({"case":case,"particle":index,"field":key,"actual":value,"expected":expected}));
            }
        }
    }
    count
}

#[test]
#[ignore = "MOLY_PARTICLE_BIRTH_DEATH_SAMPLES must identify current native observations"]
fn rounded_birth_deaths_match_current_native() {
    let path = std::env::var("MOLY_PARTICLE_BIRTH_DEATH_SAMPLES")
        .expect("birth-death-native.json path required");
    let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        receipt["sourceSha256"],
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
    );
    let rows = receipt["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 110);
    let context = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let mut failures = Vec::new();
    let mut compared_particles = 0;
    let mut retained_padding_cases = 0;
    let mut discarded_live_accepted_cases = 0;
    let mut native_kills = 0;
    for (case, row) in rows.iter().enumerate() {
        assert_eq!(row["case"].as_u64().unwrap() as usize, case);
        let input = &row["input"];
        let old = input["oldCount"].as_u64().unwrap() as usize;
        let mut system = test_support::runtime();
        system.emitter.max_particles = raw_u32(&input["maximum"]);
        system.emitter.duration = raw_float(&input["durationBits"]);
        system.emitter.prewarm = false;
        system.prewarmed = true;
        assert_eq!(input["ringMode"], 0);
        system.emitter.ring_buffer_mode = RingBufferMode::Disabled;
        system.emitter.simulation_space = SimulationSpace::Local;
        system.emitter.shape = None;
        system.emitter.shape_enabled = Some(false);
        system.emitter.start.lifetime = MinMaxCurve::Constant(raw_float(&input["lifetimeBits"]));
        system.emitter.start.speed = MinMaxCurve::Constant(raw_float(&input["speedBits"]));
        system.emitter.start.size = MinMaxCurve::Constant(1.0);
        system.emitter.start.rotation = MinMaxCurve::Constant(0.0);
        import_raw_before(&mut system, &row["before"], old);
        let words = input["initialRandomWords"].as_array().unwrap();
        assert_eq!(words.len(), 16);
        let mut random = ModuleRandom {
            words: std::array::from_fn(|word| {
                std::array::from_fn(|lane| raw_u32(&words[word * 4 + lane]))
            }),
        };
        let batch = BirthBatch {
            count: raw_u32(&input["requested"]),
            rate_count: raw_u32(&input["rateCount"]),
            distribution: BirthDistribution {
                spacing: raw_float(&input["distributionBits"][0]),
                offset: raw_float(&input["distributionBits"][1]),
                burst_fraction: raw_float(&input["distributionBits"][2]),
            },
        };
        let dt = raw_float(&input["dtBits"]);
        let current = raw_float(&input["currentBits"]);
        let inverse_duration =
            moly_law::particle::initial::initial_reciprocal(system.emitter.duration).unwrap();
        super::birth::start_explicit(
            &mut system,
            &mut random,
            batch,
            dt,
            (current - dt) * inverse_duration,
            current * inverse_duration,
            &context,
        )
        .unwrap();
        compared_particles += compare_raw_after(&system, &row["after"], case, &mut failures);
        let killed = row["killTrace"].as_array().unwrap().len() as u64;
        native_kills += killed;
        if system.died_total != killed {
            failures.push(json!({"case":case,"field":"diedTotal","actual":system.died_total,"expected":killed}));
        }
        assert_eq!(system.born_total, old as u64 + batch.count as u64);
        assert_eq!(system.full_total, 0);
        retained_padding_cases += usize::from(
            !row["verification"]["retainedPadding"]
                .as_array()
                .unwrap()
                .is_empty(),
        );
        discarded_live_accepted_cases += usize::from(
            !row["verification"]["discardedLiveAccepted"]
                .as_array()
                .unwrap()
                .is_empty(),
        );
    }
    assert_eq!(
        (
            native_kills,
            retained_padding_cases,
            discarded_live_accepted_cases
        ),
        (244, 15, 35)
    );
    let report = json!({
        "cases":rows.len(),"comparedParticles":compared_particles,"nativeKillCalls":native_kills,
        "retainedPaddingCases":retained_padding_cases,"discardedLiveAcceptedCases":discarded_live_accepted_cases,
        "failureCount":failures.len(),"firstFailures":failures.iter().take(12).collect::<Vec<_>>(),
        "meaning":"Raw current native final-channel comparison, including rounded padding death and packing. No source graph, automatic seed owner or renderer equivalence claim."
    });
    if let Ok(path) = std::env::var("MOLY_PARTICLE_BIRTH_DEATH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(
        failures.is_empty(),
        "native rounded birth mismatches: {report}"
    );
}

fn mutable_runtime_snapshot(system: &Runtime) -> Value {
    json!({
        "clock":[system.previous_head.to_bits(),system.playback_head.to_bits()],
        "emissionStarted":system.emission_started,"prewarmed":system.prewarmed,
        "emissionCarry":system.emission.to_emit_accumulator.to_bits(),"rng":system.rng.0,
        "ringCursor":system.ring_cursor,
        "totals":[system.born_total,system.died_total,system.full_total,system.refused_total],
        "pool":system.pool.iter().map(|p|json!({"position":p.position.map(f32::to_bits),"velocity":p.velocity.map(f32::to_bits),
            "life":p.start_lifetime.to_bits(),"age":p.age_percent.to_bits(),"inverse":p.inverse_lifetime.to_bits()})).collect::<Vec<_>>(),
        "side":system.side.iter().map(|s|json!({"rand":s.rand.to_bits(),"seed":s.seed,"rotation":s.rot.map(f32::to_bits),
            "size":s.size.map(f32::to_bits),"gravity":s.gravity.to_bits(),"color":s.colour.map(f32::to_bits),
            "totalVelocity":s.total_velocity.map(f32::to_bits),"custom":s.custom_data.map(|v|v.map(f32::to_bits))})).collect::<Vec<_>>(),
    })
}

#[test]
fn unqualified_initial_curve_refuses_before_any_normal_step_state_changes() {
    use moly_law::particle::autonomous_emission::AutonomousEmissionState;
    use moly_law::particle::initial::{InitialField, Refused};
    use moly_law::particle::seed_owner::ScalarRandom;
    use moly_law::particle::{Curve, CurveKey};
    let mut system = test_support::runtime();
    system.emitter.start.lifetime = MinMaxCurve::Curve {
        multiplier: 1.0,
        max: Curve {
            multiplier: 1.0,
            keys: [0.0, 1.0]
                .map(|time| CurveKey {
                    time,
                    value: 2.0,
                    in_slope: 0.0,
                    out_slope: 0.0,
                    weighted_mode: 0,
                    in_weight: 0.0,
                    out_weight: 0.0,
                })
                .to_vec(),
        },
    };
    system.playback_head = 0.75;
    system.previous_head = 0.5;
    system.emission.to_emit_accumulator = 0.625;
    let mut state = super::birth::NativeBirthState {
        owner: None,
        initial: explicit_probe_random(),
        shape: explicit_probe_random(),
        emission: AutonomousEmissionState {
            distribution: BirthDistribution {
                spacing: 0.25,
                offset: 0.625,
                burst_fraction: 0.5,
            },
            random: ScalarRandom {
                words: [17, 19, 127, 2471805022],
            },
        },
        frame: Default::default(),
        events: None,
        target: None,
    };
    let runtime_before = mutable_runtime_snapshot(&system);
    let initial_before = state.initial;
    let emission_before = state.emission;
    let context = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let result = super::birth::step_explicit(&mut system, &mut state, 0.125, false, &context);
    assert_eq!(
        result,
        Err(super::birth::BirthRefused::Initial(
            Refused::UnsupportedCurve(InitialField::Lifetime)
        ))
    );
    assert_eq!(mutable_runtime_snapshot(&system), runtime_before);
    assert_eq!(state.initial, initial_before);
    assert_eq!(state.emission, emission_before);
}
