//! Play's procedural first-Play warm against native runs of the engine (the
//! procedural warm receipt): every distinct procedural-warm block the
//! census names (a Mesh-shape block excepted) and synthetic variants of the
//! first, each installed on the native owner as the admission installs it,
//! warmed through the product's Play warm and then stepped through the
//! product frame entry. After the warm and after every frame it compares the
//! clock, the pending time, the emission state, the Initial and Shape
//! streams, every particle (position, velocity, age, inverse lifetime,
//! seed, colour, start size, rotation, the axis of rotation of a mesh
//! renderer) and the emit replays; the warm also its Compute and its slices.
//! The probe RNG words replace the installed streams: inputs, not claims
//! about client entropy. The harness places the emitter at a translation
//! with unit scale; zeros in X compare by value (the runtime reflects X).
use super::frame_samples::{compare, f, frame_context, read, same, word, Tally};
use super::*;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::{json, Value};

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("native words").iter().map(word).collect()
}

fn module_random(value: &Value) -> ModuleRandom {
    let w = words(value);
    assert_eq!(w.len(), 16);
    ModuleRandom { words: std::array::from_fn(|i| std::array::from_fn(|lane| w[i * 4 + lane])) }
}

/// The system as the admission installs it: the exported record decoded,
/// the procedural route, the native birth path, the Shape emitter state and
/// the curve admissions (a CustomData or size law that follows the storage
/// on its own conditions), a mesh renderer's geometry with an empty mesh.
fn procedural_runtime(record: &Value) -> Result<Runtime, String> {
    let wrapped = json!({"effects": {"harness": {"particles": [record]}}});
    let mut decoded = moly_law::particle::schema::Effects::from_json_str(&serde_json::to_vec(&wrapped).unwrap())
        .map_err(|e| e.to_string())?;
    assert_eq!(decoded.emitters.len(), 1);
    let emitter = decoded.emitters.remove(0);
    let route = source_route(&record["system"]);
    if route != SourceRoute::Procedural {
        return Err(format!("source route {route:?}"));
    }
    native_birth_eligible(&emitter, &route)?;
    let mesh_renderer = record["renderer"]["renderMode"] == "Mesh";
    let scaling = match record["system"]["scalingMode"].as_u64() {
        Some(0) => crate::particle_geometry::Scaling::Hierarchy,
        Some(1) => crate::particle_geometry::Scaling::Local { scale: Vec3::ONE, unit_chain: true },
        other => return Err(format!("scalingMode {other:?}")),
    };
    native_shape_state_eligible(&emitter, Some(ShapeEmitterEvidence { scaling, mesh_renderer }))?;
    let storage = || custom_data_storage_eligible(&emitter, &route);
    let size_storage = || size_storage_eligible(&emitter, &route);
    curve_admission_with(&emitter, Some(&storage), Some(&size_storage))?;
    let mut system = test_support::runtime();
    system.geometry = if mesh_renderer {
        let pivot: Vec<f32> = record["renderer"]["pivot"].as_array().unwrap().iter()
            .map(|v| v.as_f64().unwrap() as f32).collect();
        Geometry::Mesh(crate::particle_geometry::MeshDraw {
            source: std::sync::Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
                uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), submesh_ends: Vec::new(), bounds_size: Vec3::ZERO }),
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
    system.kind = EffectKind::Sky;
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    system.velocity_law = None;
    system.rol = emitter.rotation_over_lifetime.as_ref().map(|p|
        RotationOverLifetime::from_parts(p.separate_axes, p.x.as_ref(), p.y.as_ref(), &p.curve).unwrap());
    system.size_law = emitter.size_over_lifetime.as_ref()
        .map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).unwrap());
    system.color_law = emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = emitter.custom_data.as_ref().map(|p| custom_data_law(p).unwrap());
    system.emitter = emitter;
    system.pool.clear();
    system.side.clear();
    system.playback_head = 0.0;
    system.previous_head = 0.0;
    system.prewarmed = false;
    system.native_birth = None;
    Ok(system)
}

/// With `MOLY_PROCEDURAL_DEBUG` set, the first particle whose position,
/// velocity or axis differs from the native row, in source coordinates.
fn debug_row(system: &Runtime, row: &Value, label: &str) {
    if std::env::var_os("MOLY_PROCEDURAL_DEBUG").is_none() {
        return;
    }
    let particles = &row["particles"];
    let count = particles["count"].as_u64().unwrap() as usize;
    if system.pool.len() != count {
        println!("debug {label}: count {} native {count}", system.pool.len());
        return;
    }
    let native = |key: &str, a: usize, i: usize| f(&particles[key][a][i]);
    for i in 0..count {
        let p = &system.pool[i];
        let ours_pos = [-p.position[0], p.position[1], p.position[2]];
        let ours_vel = [-p.velocity[0], p.velocity[1], p.velocity[2]];
        let pos: [f32; 3] = std::array::from_fn(|a| native("pos", a, i));
        let vel: [f32; 3] = std::array::from_fn(|a| native("vel", a, i));
        let axis = particles.get("axis").map(|_| std::array::from_fn::<f32, 3, _>(|a| native("axis", a, i)));
        let bits = |ours: [f32; 3], native: [f32; 3]| ours[0] != native[0]
            || (1..3).any(|a| ours[a].to_bits() != native[a].to_bits());
        let differs = bits(ours_pos, pos) || bits(ours_vel, vel)
            || axis.is_some_and(|axis| (0..3).any(|a| system.side[i].axis[a].to_bits() != axis[a].to_bits()));
        if differs {
            println!("debug {label} particle {i}/{count}: pos {ours_pos:?} native {pos:?}; vel {ours_vel:?} native {vel:?};                 axis {:?} native {axis:?}; age {} native {}", system.side[i].axis, p.age_percent, f(&particles["age"][i]));
            return;
        }
    }
}

/// The fields of the procedural path the frame comparison does not carry:
/// the Shape stream, the emit replays, the rotation and axis arrays.
fn compare_extra(system: &Runtime, row: &Value, tally: &mut Tally, label: &str) {
    let Some(state) = system.native_birth.as_ref() else { return };
    let mut ok = true;
    let shape = words(&row["after"]["shapeRng"]);
    tally.check("shapeRng", (0..16).all(|i| state.shape.words[i / 4][i % 4] == shape[i]), &mut ok, label);
    let particles = &row["particles"];
    let native = particles["replays"].as_array().unwrap();
    let replays = native.len() == state.replays.len() && native.iter().zip(&state.replays).all(|(n, r)| {
        let w = words(&Value::Array(n.as_array().unwrap()[..5].to_vec()));
        r.time.to_bits() == w[0] && r.alive_time.to_bits() == w[1] && r.offset.to_bits() == w[2]
            && r.gap.to_bits() == w[3] && r.count as u32 == w[4] && Some(r.continuous) == n[5].as_u64()
    });
    tally.check("replays", replays, &mut ok, label);
    let count = particles["count"].as_u64().unwrap() as usize;
    if system.pool.len() != count {
        return;
    }
    let rot = &particles["rot"];
    tally.check("rotation", (0..count).all(|i| (0..3).all(|a| same(system.side[i].rot[a], &rot[a][i]))), &mut ok, label);
    if let Some(axis) = particles.get("axis") {
        tally.check("axisOfRotation", (0..count).all(|i| (0..3).all(|a| same(system.side[i].axis[a], &axis[a][i]))),
            &mut ok, label);
    }
}

#[derive(Default)]
struct Run {
    cases: usize,
    refused: Vec<String>,
    tally: Tally,
}

fn run_case(case: &Value, run: &mut Run) {
    let name = case["name"].as_str().unwrap();
    let frames = case["frames"].as_array().unwrap();
    assert_eq!(case["warmFlags"], 3, "{name}: the product's Play warm is the flags-3 update");
    let mut system = match procedural_runtime(&case["record"]) {
        Ok(system) => system,
        Err(reason) => { run.refused.push(format!("{name}: {reason}")); return; }
    };
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    match install_native_birth(&mut system, &mut manager, &SourceRoute::Procedural, None).unwrap() {
        BirthPath::Native => {}
        BirthPath::Legacy(reason) => { run.refused.push(format!("{name}: install: {reason}")); return; }
    }
    {
        let seeds = &case["seeds"];
        let state = system.native_birth.as_mut().unwrap();
        assert!(state.procedural, "{name}: the installer marks the procedural warm");
        state.owner = None;
        state.initial = module_random(&seeds["initialWords"]);
        state.shape = module_random(&seeds["shapeWords"]);
        let emission = words(&seeds["emissionWords"]);
        state.emission.random = ScalarRandom { words: std::array::from_fn(|i| emission[i]) };
    }
    run.cases += 1;
    let tally = &mut run.tally;
    // ---- Compute and the warm's slices
    let warm_row = &frames[0];
    let compute = &warm_row["compute"];
    let mut ok = true;
    let plan = || -> Result<moly_law::particle::prewarm::PrewarmPlan, &'static str> {
        let warm = moly_law::particle::prewarm::FirstPlayWarm::from_source(first_play_lifetime(&system.emitter)?,
            PLAYER_TIME, first_play_state(&system))?;
        moly_law::particle::prewarm::PrewarmPlan::procedural(warm, PLAYER_TIME, system.emitter.duration)
    };
    let label = format!("{name}#warm");
    match plan() {
        Ok(plan) => {
            tally.check("computeOut", plan.compute_out().to_bits() == word(&compute["outBits"]), &mut ok, &label);
            tally.check("computeClock", plan.initial_clock().to_bits() == word(&compute["clockBits"]), &mut ok, &label);
            let slices: Vec<f32> = plan.map(|slice| slice.map_or(f32::NAN, |s| s.duration)).collect();
            let native = warm_row["slices"].as_array().unwrap();
            tally.check("warmSlices", slices.len() == native.len()
                && slices.iter().zip(native).all(|(ours, n)| ours.to_bits() == word(&n[1])), &mut ok, &label);
        }
        Err(reason) => { run.refused.push(format!("{name}: warm plan: {reason}")); return; }
    }
    let ctx = frame_context(&warm_row["input"]);
    let mut state = system.native_birth.take().unwrap();
    let warmed = prewarm_native(&mut system, &mut state, &ctx);
    system.native_birth = Some(state);
    if let Err(reason) = warmed {
        run.refused.push(format!("{name}: warm: {reason}"));
    }
    tally.check("warmPending", same(system.pending, &warm_row["after"]["pendingBits"]), &mut ok, &label);
    compare(&system, warm_row, false, tally, &label);
    compare_extra(&system, warm_row, tally, &label);
    debug_row(&system, warm_row, &label);
    // ---- the ordinary frames after it
    for (index, row) in frames.iter().enumerate().skip(1) {
        let input = &row["input"];
        assert_eq!(input["flags"], 0, "{name}");
        let label = format!("{name}#{index}");
        let _ = advance_frame(&mut system, f(&input["dtBits"]), true, &frame_context(input), |_| {});
        compare(&system, row, true, tally, &label);
        compare_extra(&system, row, tally, &label);
        debug_row(&system, row, &label);
    }
    if system.refused_total != 0 {
        run.refused.push(format!("{name}: {} refused steps", system.refused_total));
    }
}

fn replay(receipt: &Value, arm: Option<&'static str>) -> Run {
    assert_eq!(receipt["sourceSha256"], SOURCE_SHA256);
    super::frame_samples::with_arm(arm, || {
        let mut run = Run::default();
        for case in receipt["cases"].as_array().unwrap() {
            run_case(case, &mut run);
        }
        run
    })
}

fn mismatched(run: &Run) -> usize {
    run.tally.fields.values().map(|(_, bad)| bad).sum()
}

/// The procedural warm receipt through the product: every case the product
/// admits matches every native field of the warm and the frames after it;
/// the receipt's positive control (the same block warmed without the
/// procedural bit) and its static-initializer control changed rows; every
/// one-rule arm of the procedural path mismatches, and so does a bit-flipped
/// receipt.
#[test]
#[ignore = "MOLY_PROCEDURAL_RECEIPT must identify the JP procedural warm receipt"]
fn procedural_warm_matches_native_rows() {
    let receipt = read("MOLY_PROCEDURAL_RECEIPT");
    let run = replay(&receipt, None);
    let report = json!({"cases": run.cases, "receiptCases": receipt["cases"].as_array().unwrap().len(),
        "frames": run.tally.frames, "mismatchedFrames": run.tally.mismatched_frames, "mismatchedFields": mismatched(&run),
        "fields": run.tally.fields.iter().map(|(k, (n, bad))| (k.to_string(), json!([n, bad])))
            .collect::<serde_json::Map<_, _>>(), "refused": run.refused, "firstMismatches": run.tally.first});
    println!("{report}");
    if let Some(path) = std::env::var_os("MOLY_PROCEDURAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(run.cases > 0 && run.tally.frames > 0, "{report}");
    assert_eq!((run.tally.mismatched_frames, mismatched(&run)), (0, 0), "{report}");
    assert!(receipt["controls"]["k1VersusBaseDifferingFrames"].as_u64().unwrap() > 0);
    assert!(receipt["staticInitializers"]["baseVersusIdentityClearedDifferingFrames"].as_u64().unwrap() > 0);
    // The regeneration's kill pass is reached only through a Mesh shape's
    // vertex colour, which the procedural warm refuses: the receipt records
    // no KillParticle call inside UpdateProcedural, while the frames kill.
    let kills = &receipt["killParticleCalls"];
    assert_eq!(kills["insideUpdateProcedural"], 0);
    assert!(kills["inFrames"].as_u64().unwrap() > 0);
    for arm in ["proceduralReplaysNotAged", "proceduralCapNotDoubled", "proceduralAgeNoStagger", "proceduralRotationSkipped"] {
        let armed = replay(&receipt, Some(arm));
        println!("arm {arm}: {} mismatched fields over {} frames", mismatched(&armed), armed.tally.frames);
        assert!(mismatched(&armed) > 0, "arm {arm} must mismatch");
    }
    if let Some(path) = std::env::var_os("MOLY_PROCEDURAL_BITFLIP") {
        let control: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let flipped = replay(&control, None);
        println!("bitflip: {} mismatched fields", mismatched(&flipped));
        assert!(mismatched(&flipped) > 0, "the bit-flipped receipt must mismatch");
    }
}
