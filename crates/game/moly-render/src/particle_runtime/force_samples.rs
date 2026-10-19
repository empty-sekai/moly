//! Replay external native observations through the live shared simulation path.
use super::*;
use moly_law::particle::{Curve, CurveKey, MinMaxCurve, force::ForceOverLifetime, schema::ForceParams};
use serde_json::{Value, json};

fn number(v: &Value) -> f32 { v.as_f64().expect("numeric observation") as f32 }
fn vector(v: &Value) -> [f32; 3] { std::array::from_fn(|axis| number(&v[axis])) }
fn curve(v: &Value) -> MinMaxCurve {
    let line = |key: &str| {
        let (start, end) = (number(&v[key][0]), number(&v[key][1]));
        let slope = if v["linear"] == true { end - start } else { 0.0 };
        Curve { multiplier: 1.0, keys: [0.0, 1.0].map(|time| CurveKey {
            time, value: if time == 0.0 { start } else { end },
            in_slope: slope, out_slope: slope, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0,
        }).to_vec(), pre_wrap: Some(2), post_wrap: Some(2) }
    };
    match v["mode"].as_u64().unwrap() {
        0 => MinMaxCurve::Constant(number(&v["max"])),
        3 => MinMaxCurve::TwoConstants { min: number(&v["min"]), max: number(&v["max"]) },
        1 => MinMaxCurve::Curve { multiplier: number(&v["multiplier"]), max: line("high") },
        2 => MinMaxCurve::TwoCurves { multiplier: number(&v["multiplier"]), min: line("low"), max: line("high") },
        other => panic!("unknown observed curve mode {other}"),
    }
}

#[test]
#[ignore = "MOLY_PARTICLE_FORCE_SAMPLES must identify external native observations"]
fn force_matches_native_in_shared_runtime() {
    let path = std::env::var("MOLY_PARTICLE_FORCE_SAMPLES").expect("source observations path required");
    let input: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows = input["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "empty observations cannot verify simulation");
    let reflect = crate::particle_geometry::reflect;
    let reflection = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    let context = Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let mut failures = Vec::new();
    let mut maximum_error = [0.0_f32; 3];
    for (index, row) in rows.iter().enumerate() {
        let mut system = test_support::runtime();
        let params = ForceParams { axes: std::array::from_fn(|axis| curve(&row["axes"][axis])),
            in_world_space: row["moduleWorld"].as_bool().unwrap(), randomize_per_frame: false };
        system.force_law = Some(ForceOverLifetime::from_params(&params).unwrap());
        system.emitter.force = Some(params);
        system.emitter.simulation_space = if row["simulation"] == 1 { SimulationSpace::World } else { SimulationSpace::Local };
        let owner = Mat4::from_cols_array(&std::array::from_fn(|i| number(&row["owner"][i])));
        system.node_affine = GlobalTransform::from(reflection * owner * reflection);
        let seed = row["seed"].as_u64().unwrap() as u32;
        let age = number(&row["agePercent"]);
        system.side[0].seed = seed;
        system.pool[0] = Particle::born([0.0; 3], reflect(Vec3::from_array(vector(&row["velocity"]))).to_array(), 10.0);
        system.pool[0].age_percent = age;
        let sample = system.force_law.as_ref().unwrap().sample(seed, age);
        let dt = number(&row["dt"]);
        simulate_stopped(&mut system, dt, &context);
        let velocity = reflect(Vec3::from_array(system.pool[0].velocity)).to_array();
        let position = reflect(Vec3::from_array(system.pool[0].position)).to_array();
        let expected = vector(&row["expected"]);
        for (field_index, (field, actual, expected, tolerance)) in [
            ("sample", sample, vector(&row["sample"]), 0.0),
            ("velocity", velocity, expected, 0.00002),
            ("position", position, expected.map(|v| v * dt), 0.00002),
        ].into_iter().enumerate() {
            let error = actual.into_iter().zip(expected).map(|(a,b)| (a-b).abs()).fold(0.0_f32, f32::max);
            maximum_error[field_index] = maximum_error[field_index].max(error);
            if error > tolerance || actual.iter().any(|v| !v.is_finite()) {
                failures.push(json!({"row":index,"field":field,"actual":actual,"expected":expected,"error":error}));
            }
        }
    }
    let report = json!({"samples":rows.len(),"failureCount":failures.len(),"maximumErrors":maximum_error,"firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path) = std::env::var("MOLY_PARTICLE_FORCE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(failures.is_empty(), "native force mismatches: {report}");
}
