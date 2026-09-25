//! Research instrument: the law inputs the product hands the source mesh
//! kernel about the axis of rotation (`particle_geometry::axis_law_input`),
//! for systems of a caller-supplied extraction stepped on their own birth
//! path, dumped with the law's output for the native receipt. No game data,
//! private paths or fixed row counts are embedded here.
use super::*;
use serde_json::{json, Value};

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

/// One extraction row stepped through the product's runtime.
fn row_runtime(roots: &[std::path::PathBuf], phenomenon: &str, effect: &str, node: &str)
    -> (Runtime, SourceRoute, crate::particle_geometry::AxisBody) {
    let path = roots.iter().map(|root| root.join(phenomenon).join("fx/effects.json"))
        .find(|path| path.is_file()).unwrap_or_else(|| panic!("{phenomenon}: no effects document under the roots"));
    let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let particle = raw["effects"][effect]["particles"].as_array().unwrap().iter()
        .find(|p| p["node"] == node).unwrap_or_else(|| panic!("{effect} {node}: no particle"));
    let selected = json!({"effects": {effect: {"particles": [particle]}}});
    let mut decoded = moly_law::particle::schema::Effects::from_json_str(&serde_json::to_vec(&selected).unwrap()).unwrap();
    let emitter = decoded.emitters.remove(0);
    let system = &particle["system"];
    let renderer = &particle["renderer"];
    let alignment = crate::particle_geometry::Alignment::from_source(renderer["alignment"].as_i64().unwrap()).unwrap();
    let allow_roll = renderer["allowRoll"].as_bool().unwrap();
    let initial_enabled = system["sourceModules"]["enabled"].as_array().unwrap().iter().any(|m| m == "InitialModule");
    let body = mesh_rotation_admission(&emitter, initial_enabled, alignment, allow_roll)
        .unwrap_or_else(|refused| panic!("{node}: {}", refused.reason()))
        .unwrap_or_else(|| panic!("{node}: the particle arrays use 3D rotation"));
    let vec3 = |v: &Value| Vec3::new(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32, v[2].as_f64().unwrap() as f32);
    let bounds_size = vec3(&renderer["meshes"][0]["bounds"]["extents"]) * 2.0;
    let scaling = match system["scalingMode"].as_i64().unwrap() {
        0 => crate::particle_geometry::Scaling::Hierarchy,
        1 => crate::particle_geometry::Scaling::Local { scale: Vec3::ONE, unit_chain: true },
        other => panic!("{node}: scaling mode {other}"),
    };
    assert!(emitter.velocity_over_lifetime.is_none() && emitter.rotation_over_lifetime.is_none()
        && emitter.size_over_lifetime.is_none() && emitter.force.is_none() && emitter.limit_velocity.is_none()
        && emitter.noise.is_none() && emitter.texture_sheet.is_none(), "{node}: a module this instrument does not install");
    let mut rt = test_support::runtime();
    rt.pool.clear();
    rt.side.clear();
    rt.born_total = 0;
    rt.node = emitter.node.clone();
    rt.effect = emitter.effect.clone();
    rt.kind = EffectKind::Sky;
    rt.geometry = Geometry::Mesh(crate::particle_geometry::MeshDraw {
        source: std::sync::Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
            uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), bounds_size }),
        alignment, scaling, pivot: vec3(&renderer["pivot"]), flip: vec3(&renderer["flip"]), axis_body: Some(body),
    });
    rt.gravity_law = moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).unwrap();
    rt.color_law = emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    rt.custom_law = emitter.custom_data.as_ref().map(|p| moly_law::particle::custom_data::CustomData::from_params(p).unwrap());
    rt.emitter = emitter;
    (rt, source_route(system), body)
}

#[test]
#[ignore = "MOLY_MESH_AXIS_CORPUS (phenomena directories, overlay first), MOLY_MESH_AXIS_ROWS (phenomenon|effect|node;...) and MOLY_MESH_AXIS_DUMP"]
fn mesh_axis_product_rows() {
    let roots: Vec<_> = std::env::split_paths(&std::env::var_os("MOLY_MESH_AXIS_CORPUS").expect("corpus roots")).collect();
    let rows = std::env::var("MOLY_MESH_AXIS_ROWS").expect("rows");
    let out = std::env::var_os("MOLY_MESH_AXIS_DUMP").expect("dump path");
    let camera = GlobalTransform::from(Transform::from_xyz(3.0, 2.0, 8.0).looking_at(Vec3::new(-20.0, 40.0, -100.0), Vec3::Y));
    let ctx = Context { sky: GlobalTransform::from_translation(camera.translation()), camera, site: GlobalTransform::IDENTITY };
    let mut groups = Vec::new();
    for spec in rows.split(';').filter(|s| !s.is_empty()) {
        let [phenomenon, effect, node]: [&str; 3] = spec.split('|').collect::<Vec<_>>().try_into().expect("phenomenon|effect|node");
        let (mut rt, route, body) = row_runtime(&roots, phenomenon, effect, node);
        let mut seeds = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
        let path = match install_native_birth(&mut rt, &mut seeds, &route).unwrap() {
            BirthPath::Native => "native".to_string(),
            BirthPath::Legacy(reason) => format!("legacy: {reason}"),
        };
        if rt.emitter.prewarm && rt.emitter.looping {
            if let Err(reason) = prewarm_first_play(&mut rt, &ctx) {
                eprintln!("{node}: first Play warm refused: {reason}");
            }
        }
        let mut particles = Vec::new();
        for frame in 1..=300 {
            advance_frame(&mut rt, 1.0 / 30.0, true, &ctx, |_| {}).unwrap_or_else(|reason| panic!("{node}: {reason}"));
            if frame % 7 != 0 || particles.len() >= 48 {
                continue;
            }
            let owner = compose_to_world(&rt, &ctx);
            let Geometry::Mesh(draw) = &rt.geometry else { unreachable!() };
            let frame_basis = draw.scaling.apply(crate::particle_geometry::source_frame(&owner, &ctx.camera));
            for instance in geometry_instances(&rt, &owner) {
                let (space, input) = crate::particle_geometry::axis_law_input(&instance, &frame_basis, body,
                    draw.source.bounds_size, draw.pivot, draw.flip);
                let law = moly_law::particle::mesh_transform::about_axis(&space, &input);
                let product = crate::particle_geometry::mesh_transform_about_axis(&instance, &frame_basis, body,
                    draw.source.bounds_size, draw.pivot, draw.flip);
                let product_words = bits(&product.to_cols_array());
                assert_eq!(product_words, [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14].map(|i| law.affine[i].to_bits()),
                    "the product's transform is the law's");
                particles.push(json!({
                    "M": space.basis.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "A": bits(&space.affine),
                    "S": ([bits(&space.scale), vec![0]].concat()),
                    "offset": ([bits(&space.offset), vec![0]].concat()),
                    "flip": ([bits(&space.flip), vec![0]].concat()),
                    "float1": space.angle.to_bits(),
                    "pos": bits(&input.position), "size": bits(&input.size), "age": input.age_percent.to_bits(),
                    "seed": input.seed, "rz": input.rotation_z.to_bits(), "axis": bits(&input.axis),
                    "lawAffine": bits(&law.affine),
                }));
                if particles.len() >= 48 { break; }
            }
        }
        eprintln!("{phenomenon} {effect} {node}: {path}, body {body:?}, {} particles, born {}", particles.len(), rt.born_total);
        assert!(!particles.is_empty(), "{node}: no particle drawn");
        groups.push(json!({"row": spec, "body": format!("{body:?}"), "path": path, "particles": particles}));
    }
    std::fs::write(out, serde_json::to_vec(&json!({"rows": groups})).unwrap()).unwrap();
}
