//! External native UV observations replayed through the actual mesh writer.
use super::*;
use moly_law::particle::{MinMaxCurve, schema::TextureSheetParams, texture_sheet::TextureSheet};
use serde_json::{Value, json};

#[test]
#[ignore = "MOLY_PARTICLE_UV_SAMPLES must identify external native observations"]
fn texture_sheet_matches_native_in_shared_geometry() {
    let path = std::env::var("MOLY_PARTICLE_UV_SAMPLES").expect("source observations path required");
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!rows.is_empty(), "empty observations cannot verify geometry");
    let identity = GlobalTransform::IDENTITY;
    let camera = GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 10.0));
    let basis = crate::billboard::CameraBasis { position: camera.translation(), forward: -Vec3::Z,
        right: Vec3::X, up: Vec3::Y, fov_y: 1.0, aspect: 1.0, near: 0.3, velocity: Vec3::ZERO };
    let mut system = test_support::runtime();
    system.geometry = Geometry::SourceBillboard(crate::source_billboard::Draw {
        mode: crate::source_billboard::Mode::Billboard,
        alignment: crate::particle_geometry::Alignment::View,
        pivot: Vec3::ZERO, screen_size: Vec2::new(0.0, 1.0), allow_roll: true,
        scaling: crate::particle_geometry::Scaling::Hierarchy,
    });
    let mut mesh = crate::billboard::empty_mesh();
    let mut failures = Vec::new();
    let mut maximum_error = 0.0_f32;
    for (index, row) in rows.iter().enumerate() {
        let x: Vec<f32> = row["input"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let curve = |start: usize| if x[start] == 0.0 { MinMaxCurve::Constant(x[start+2]) }
            else { MinMaxCurve::TwoConstants { min: x[start+1], max: x[start+2] } };
        let params = TextureSheetParams { mode: 0, time_mode: 0,
            animation_type: x[0] as u32, row_mode: x[1] as u32, row_index: x[2] as i32,
            tiles: [x[3] as u32, x[4] as u32], cycles: 1.0, fps: 30.0,
            speed_range: [0.0, 1.0], uv_channel_mask: -1, flip: [0.0; 2], frame: curve(5), start: curve(8) };
        let seed = row["input"][11].as_u64().unwrap() as u32;
        let sheet = TextureSheet::from_params(&params).unwrap();
        system.texture_sheet = Some(sheet);
        system.side[0].seed = seed;
        // Repeated render writes must not apply the atlas transform a second
        // time to the previous frame's coordinates.
        for _ in 0..2 {
            write_geometry(&mut mesh, &system, &identity, &identity, &camera, basis);
            let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { panic!("missing UV0") };
            let actual: Vec<_> = std::iter::once(sheet.position(seed)).chain(uv.iter().flatten().copied()).collect();
            let expected: Vec<_> = row["expected"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            assert_eq!(actual.len(), expected.len());
            let error = actual.iter().zip(&expected).map(|(a,b)| (a-b).abs()).fold(0.0_f32, f32::max);
            maximum_error = maximum_error.max(error);
            if error != 0.0 || actual.iter().any(|v| !v.is_finite()) {
                failures.push(json!({"row":index,"actual":actual,"expected":expected,"error":error}));
            }
        }
    }
    let report = json!({"samples":rows.len(),"meshWrites":rows.len()*2,"failureCount":failures.len(),
        "maximumError":maximum_error,"firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path) = std::env::var("MOLY_PARTICLE_UV_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(failures.is_empty(), "native texture-sheet mismatches: {report}");
}
