use super::*;
use serde_json::Value;
fn vector(v: &Value) -> Vec3 {
    Vec3::new(
        v["x"].as_f64().unwrap() as f32,
        v["y"].as_f64().unwrap() as f32,
        v["z"].as_f64().unwrap() as f32,
    )
}
#[test]
fn independent_billboard_vertices_and_normals_match_all_supported_engine_cases() {
    let data: Value = serde_json::from_str(include_str!(
        "../tests/data/particle-billboard-geometry.json"
    ))
    .unwrap();
    let mut count = [0usize; 2];
    let mut failures = Vec::new();
    for row in data["cases"].as_array().unwrap() {
        let owner_rotation = euler(vector(&row["ownerRotation"]) * (std::f32::consts::PI / 180.0));
        let frame = Frame {
            rotation: owner_rotation,
            scale: vector(&row["ownerScale"]),
            camera_rotation: euler(vector(&row["cameraRotation"]) * (std::f32::consts::PI / 180.0)),
            camera_position: vector(&row["cameraPosition"]),
        };
        let local = row["simulation"] == "Local";
        let mut position = vector(&row["particlePosition"]);
        if local {
            position = owner_rotation * (frame.scale * position);
        }
        let p = Instance {
            position,
            velocity: vector(&row["velocity"]),
            rotation: vector(&row["particleRotation"]) * (std::f32::consts::PI / 180.0),
            size: vector(&row["particleSize"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
        };
        let alignment = match row["alignment"].as_str().unwrap() {
            "View" => Alignment::View,
            "World" => Alignment::World,
            "Facing" => Alignment::Facing,
            "Local" => Alignment::Local,
            "Velocity" => Alignment::Velocity,
            other => panic!("unhandled {other}"),
        };
        let horizontal = row["mode"] == "HorizontalBillboard";
        let draw = Draw {
            alignment,
            mode: if horizontal {
                Mode::Horizontal
            } else {
                Mode::Billboard
            },
            pivot: vector(&row["pivot"]),
            screen_size: Vec2::new(0.0, 10000.0),
            allow_roll: true,
            scaling: crate::particle_geometry::Scaling::Hierarchy,
        };
        let (actual, normal) = vertices(&p, &frame, &draw, local);
        for i in 0..4 {
            let position_error = (actual[i] - vector(&row["vertices"][i]))
                .abs()
                .max_element();
            let normal_error = (normal - vector(&row["normals"][i])).abs().max_element();
            if !position_error.is_finite()
                || !normal_error.is_finite()
                || position_error > 0.000025
                || normal_error > 0.000025
            {
                failures.push(format!(
                    "{}/{}/{}/{}, vertex={i}, position={position_error}, normal={normal_error}",
                    row["mode"], row["alignment"], row["simulation"], row["variant"]
                ));
            }
        }
        count[usize::from(horizontal)] += 1;
    }
    assert_eq!(count, [104, 130]);
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
#[test]
fn source_billboard_writer_retains_zero_size_and_non_unit_normal() {
    let frame = Frame {
        rotation: Mat3::IDENTITY,
        scale: Vec3::ONE,
        camera_rotation: Mat3::IDENTITY,
        camera_position: Vec3::new(0.0, 0.0, -10.0),
    };
    let draw = Draw {
        mode: Mode::Billboard,
        alignment: Alignment::World,
        pivot: Vec3::ZERO,
        screen_size: Vec2::new(0.0, 10000.0),
        allow_roll: true,
        scaling: crate::particle_geometry::Scaling::Hierarchy,
    };
    let mut p = Instance {
        position: Vec3::new(1.0, 2.0, 3.0),
        velocity: Vec3::ZERO,
        rotation: Vec3::ZERO,
        size: Vec3::new(2.0, 3.0, 4.0),
        colour: Vec4::ONE,
        custom1: Vec4::ZERO,
        custom2: Vec4::ZERO,
    };
    let (_, normal) = vertices(&p, &frame, &draw, false);
    assert!((normal.z + 12.0 / 13.0).abs() < 0.000001);
    p.size = Vec3::ZERO;
    let (corners, normal) = vertices(&p, &frame, &draw, false);
    assert_eq!(corners, [p.position; 4]);
    assert_eq!(normal, Vec3::Z);
}

fn array3(v: &Value) -> Vec3 {
    Vec3::new(
        v[0].as_f64().unwrap() as f32,
        v[1].as_f64().unwrap() as f32,
        v[2].as_f64().unwrap() as f32,
    )
}
#[test]
fn screen_limits_match_120_current_native_vertex_results() {
    let data: Value =
        serde_json::from_str(include_str!("../tests/data/particle-billboard-native.json")).unwrap();
    let frame = Frame {
        rotation: Mat3::IDENTITY,
        scale: Vec3::ONE,
        camera_rotation: Mat3::IDENTITY,
        camera_position: Vec3::new(0.0, 0.0, -10.0),
    };
    let mut failures = Vec::new();
    for (case, row) in data["cases"].as_array().unwrap().iter().enumerate() {
        let draw = Draw {
            mode: Mode::Billboard,
            alignment: Alignment::World,
            pivot: array3(&row["pivot"]),
            allow_roll: true,
            screen_size: Vec2::new(0.0, 10000.0),
            scaling: crate::particle_geometry::Scaling::Hierarchy,
        };
        let p = Instance {
            position: array3(&row["position"]),
            velocity: Vec3::ZERO,
            rotation: array3(&row["rotation"]),
            size: array3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
        };
        let limited = screen_limited(
            p.size,
            row["minimumSize"].as_f64().unwrap() as f32,
            row["maximumSize"].as_f64().unwrap() as f32,
        );
        let (actual, _) = vertices_sized(&p, &frame, &draw, false, limited);
        for (vertex, actual) in actual.into_iter().enumerate() {
            let error = (actual - array3(&row["vertices"][vertex]))
                .abs()
                .max_element();
            if !error.is_finite() || error > 0.00003 {
                failures.push(format!("case={case} vertex={vertex} error={error}"));
            }
        }
    }
    assert_eq!(data["cases"].as_array().unwrap().len(), 120);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn camera_roll_matches_24_current_native_vertex_results() {
    let data: Value =
        serde_json::from_str(include_str!("../tests/data/particle-billboard-camera.json")).unwrap();
    for (case, row) in data["cases"].as_array().unwrap().iter().enumerate() {
        let frame = Frame {
            rotation: Mat3::IDENTITY,
            scale: Vec3::ONE,
            camera_rotation: euler(array3(&row["cameraAngles"])),
            camera_position: Vec3::new(0.0, 0.0, -10.0),
        };
        let draw = Draw {
            mode: Mode::Billboard,
            alignment: Alignment::View,
            pivot: Vec3::ZERO,
            allow_roll: row["allowRoll"].as_bool().unwrap(),
            screen_size: Vec2::new(0.0, 10000.0),
            scaling: crate::particle_geometry::Scaling::Hierarchy,
        };
        let p = Instance {
            position: array3(&row["position"]),
            velocity: Vec3::ZERO,
            rotation: array3(&row["rotation"]),
            size: array3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
        };
        let (actual, _) = vertices(&p, &frame, &draw, false);
        for (vertex, actual) in actual.into_iter().enumerate() {
            let error = (actual - array3(&row["vertices"][vertex]))
                .abs()
                .max_element();
            assert!(
                error.is_finite() && error < 0.000025,
                "case={case} vertex={vertex} error={error}"
            );
        }
    }
    assert_eq!(data["cases"].as_array().unwrap().len(), 24);
}
