//! Optional engine observations exercise the same runtime used by live draws.
//! Input data is supplied externally; synthetic unit tests remain self-contained.
use super::*;
use moly_law::particle::{Curve, CurveKey, MinMaxCurve};
use moly_law::particle::schema::{SizeOverLifetimeParams, VelocityOverLifetimeParams};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn number(value: &Value, key: &str) -> f32 {
    value[key].as_f64().unwrap_or_else(|| panic!("missing numeric field {key}")) as f32
}
fn flag(value: &Value, key: &str) -> bool {
    value[key].as_bool().unwrap_or_else(|| panic!("missing boolean field {key}"))
}
fn vector(value: &Value, key: &str) -> Vec3 {
    let value = &value[key];
    Vec3::new(number(value, "x"), number(value, "y"), number(value, "z"))
}
fn constant(value: f32) -> MinMaxCurve { MinMaxCurve::Constant(value) }
fn line(end: f32) -> MinMaxCurve {
    let keys = [0.0, 1.0].map(|time| CurveKey {
        time, value: if time == 0.0 { 1.0 } else { end },
        in_slope: end - 1.0, out_slope: end - 1.0,
        weighted_mode: 0, in_weight: 0.0, out_weight: 0.0,
    });
    MinMaxCurve::Curve { multiplier: 1.0, max: Curve { keys: keys.to_vec(), multiplier: 1.0 } }
}

fn from_observation(row: &Value) -> Runtime {
    let mut system = test_support::runtime();
    let reflect = crate::particle_geometry::reflect;
    let angles = vector(row, "ownerEuler").map(f32::to_radians);
    // Unity applies fixed-axis Z, then X, then Y: matrix Ry * Rx * Rz.
    let source_rotation = Quat::from_euler(EulerRot::YXZ, angles.y, angles.x, angles.z);
    let reflected_rotation = Quat::from_xyzw(source_rotation.x, -source_rotation.y,
        -source_rotation.z, source_rotation.w);
    system.node_affine = GlobalTransform::from(Transform {
        translation: reflect(vector(row, "ownerPosition")),
        rotation: reflected_rotation,
        scale: vector(row, "ownerScale"),
    });
    system.kind = EffectKind::Site;
    system.emitter.emission = None;
    system.emitter.looping = false;
    system.emitter.simulation_space = match row["simulation"].as_str().unwrap() {
        "Local" => SimulationSpace::Local,
        "World" => SimulationSpace::World,
        other => panic!("unknown source simulation space {other}"),
    };
    system.prewarmed = true;
    system.pool[0] = Particle {
        position: reflect(vector(row, "beforePosition")).to_array(),
        velocity: reflect(vector(row, "beforeVelocity")).to_array(),
        remaining_lifetime: number(row, "beforeLifetime"),
        start_lifetime: number(row, "startLifetime"),
    };
    let side = &mut system.side[0];
    side.seed = row["seed"].as_u64().unwrap().try_into().unwrap();
    let birth_size = vector(row, "birthSize");
    system.emitter.start.size3d = flag(row, "size3D");
    side.size = if system.emitter.start.size3d { birth_size.to_array() } else { [birth_size.x; 3] };
    system.emitter.size_over_lifetime = flag(row, "sizeModule").then(|| {
        let end = number(row, "sizeEnd");
        SizeOverLifetimeParams { separate_axes: flag(row, "separateSize"),
            curve: line(end), y: Some(line(end * 0.5)), z: Some(line(end * 2.0)) }
    });
    let linear = vector(row, "linear");
    let orbital = vector(row, "orbital").to_array().map(constant);
    let orbital_offset = vector(row, "offset").to_array().map(constant);
    system.emitter.velocity_over_lifetime = Some(VelocityOverLifetimeParams {
        x: constant(linear.x), y: constant(linear.y), z: constant(linear.z),
        speed_modifier: constant(number(row, "modifier")), in_world_space: flag(row, "moduleWorld"),
        orbital, orbital_offset, radial: constant(number(row, "radial")),
    });
    system.size_law = system.emitter.size_over_lifetime.as_ref()
        .map(moly_law::particle::size::SizeOverLifetime::from_params);
    system.velocity_law = system.emitter.velocity_over_lifetime.as_ref()
        .map(moly_law::particle::velocity::VelocityOverLifetime::from_params);
    if row["family"] == "drag" {
        system.limit = Some(LimitVelocity::from_parts(false, &constant(10000.0), 0.0,
            Some(&constant(number(row, "drag"))), Some(flag(row, "multiplySize")),
            Some(flag(row, "multiplyVelocity"))).unwrap());
    }
    system
}

#[test]
#[ignore = "MOLY_PARTICLE_MOTION_SAMPLES must identify externally generated engine observations"]
fn integrated_motion_matches_engine_observations() {
    let input = std::env::var("MOLY_PARTICLE_MOTION_SAMPLES").expect("source observations path required");
    let observations: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    let rows = observations["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "empty observations cannot validate a runtime");
    let reflect = crate::particle_geometry::reflect;
    let context = Context { sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let mut failures = Vec::new();
    let mut families = BTreeMap::<String, usize>::new();
    let mut maximum_error = BTreeMap::<String, f32>::new();
    for (index, row) in rows.iter().enumerate() {
        let family = row["family"].as_str().unwrap();
        *families.entry(family.to_owned()).or_default() += 1;
        let mut system = from_observation(row);
        let size = motion::size_at_age(&system, &system.side[0], system.pool[0].normalized_age());
        simulate_stopped(&mut system, number(row, "dt"), &context);
        assert_eq!(system.pool.len(), 1, "unexpected lifetime removal at row {index}");
        let fields = [
            ("position", system.pool[0].position, reflect(vector(row, "afterPosition")).to_array(), 0.00002),
            ("velocity", system.pool[0].velocity, reflect(vector(row, "afterVelocity")).to_array(), 0.00002),
            ("totalVelocity", system.side[0].total_velocity, reflect(vector(row, "totalVelocity")).to_array(), 0.0002),
            ("size", size, vector(row, "currentSize").to_array(), 0.00002),
        ];
        for (name, actual, expected, absolute_tolerance) in fields {
            let error = actual.iter().zip(expected).map(|(a, b)| (*a - b).abs()).fold(0.0_f32, f32::max);
            let key = format!("{family}.{name}");
            let entry = maximum_error.entry(key).or_default();
            *entry = entry.max(error);
            let tolerance = absolute_tolerance + 0.000002 * expected.into_iter().map(f32::abs).fold(0.0, f32::max);
            if actual.iter().any(|v| !v.is_finite()) || error > tolerance {
                failures.push(json!({ "row": index, "family": family, "field": name,
                    "actual": actual, "expected": expected, "error": error, "tolerance": tolerance,
                    "input": row }));
            }
        }
        let actual = system.pool[0].remaining_lifetime;
        let expected = number(row, "afterLifetime");
        if (actual - expected).abs() > 0.00002 {
            failures.push(json!({ "row": index, "family": family, "field": "lifetime", "actual": actual, "expected": expected }));
        }
    }
    let report = json!({ "unityVersion": observations["unityVersion"], "rows": rows.len(),
        "families": families, "maximumError": maximum_error,
        "failureCount": failures.len(), "firstFailures": failures.iter().take(40).collect::<Vec<_>>() });
    if let Ok(path) = std::env::var("MOLY_PARTICLE_MOTION_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{}", json!({ "rows": rows.len(), "families": families,
        "maximumError": maximum_error, "failureCount": failures.len() }));
    assert!(failures.is_empty(), "{} field comparisons failed; first: {}", failures.len(),
        failures.first().unwrap_or(&Value::Null));
}
