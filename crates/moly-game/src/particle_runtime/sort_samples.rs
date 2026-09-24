//! External native renderer permutations replayed through the shared mesh writer.
use super::*;
use moly_law::particle::sort::ParticleSort;
use serde_json::{Value, json};

#[test]
#[ignore = "MOLY_PARTICLE_SORT_SAMPLES must identify external native observations"]
fn renderer_sort_matches_native_in_shared_geometry() {
    let path = std::env::var("MOLY_PARTICLE_SORT_SAMPLES").expect("source observations path required");
    let input: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows = input["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "empty observations cannot verify ordering");
    let number = |v: &Value| v.as_f64().unwrap() as f32;
    let vector = |v: &Value| Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]));
    let reflect = crate::particle_geometry::reflect;
    let mut failures = Vec::new();
    let mut particles = 0;
    for (row_index, row) in rows.iter().enumerate() {
        let mut system = test_support::runtime();
        let side = system.side[0];
        system.pool.clear(); system.side.clear();
        system.sort_mode = ParticleSort::from_source(row["mode"].as_u64().unwrap() as u32).unwrap();
        let scale = vector(&row["scale"]);
        let owner = GlobalTransform::from_scale(scale);
        let camera = GlobalTransform::from_translation(reflect(vector(&row["camera"]) * scale));
        let basis = crate::billboard::CameraBasis { position: camera.translation(), forward: -Vec3::Z,
            right: Vec3::X, up: Vec3::Y, fov_y: 1.0, aspect: 1.0 };
        for (index, position) in row["positions"].as_array().unwrap().iter().enumerate() {
            let inverse = number(&row["inverseLifetimes"][index]);
            let lifetime = 1.0 / inverse;
            let mut p = Particle::born(reflect(vector(position)).to_array(), [0.0; 3], lifetime);
            p.age_percent = number(&row["ages"][index]);
            p.inverse_lifetime = inverse;
            system.pool.push(p); system.side.push(side);
        }
        particles += system.pool.len();
        let expected: Vec<u32> = row["indices"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
        let mut mesh = crate::billboard::empty_mesh();
        for frame in 0..2 {
            write_geometry(&mut mesh, &system, &owner, &owner, &camera, basis);
            let Some(bevy::mesh::Indices::U32(indices)) = mesh.indices() else { panic!("missing particle draw indices") };
            let actual: Vec<_> = indices.chunks_exact(6).map(|triangle_pair| triangle_pair[0] / 4).collect();
            if actual != expected {
                failures.push(json!({"row":row_index,"frame":frame,"actual":actual,"expected":expected}));
            }
        }
    }
    let report = json!({"cases":rows.len(),"particles":particles,"failureCount":failures.len(),
        "firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path) = std::env::var("MOLY_PARTICLE_SORT_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");
    assert!(failures.is_empty(), "native sort mismatches: {report}");
}
