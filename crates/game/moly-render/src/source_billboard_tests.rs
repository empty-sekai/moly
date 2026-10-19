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
        "../../moly-game/tests/data/particle-billboard-geometry.json"
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
            seed: 0,
            age_percent: 0.0,
            axis: Vec3::Z,
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
            scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
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
        scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
    };
    let mut p = Instance {
        position: Vec3::new(1.0, 2.0, 3.0),
        velocity: Vec3::ZERO,
        rotation: Vec3::ZERO,
        size: Vec3::new(2.0, 3.0, 4.0),
        colour: Vec4::ONE,
        custom1: Vec4::ZERO,
        custom2: Vec4::ZERO,
        seed: 0,
        age_percent: 0.0,
        axis: Vec3::Z,
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
        serde_json::from_str(include_str!("../../moly-game/tests/data/particle-billboard-native.json")).unwrap();
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
            scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
        };
        let p = Instance {
            position: array3(&row["position"]),
            velocity: Vec3::ZERO,
            rotation: array3(&row["rotation"]),
            size: array3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
            seed: 0,
            age_percent: 0.0,
            axis: Vec3::Z,
        };
        let limited = screen_limited(
            p.size,
            row["minimumSize"].as_f64().unwrap() as f32,
            row["maximumSize"].as_f64().unwrap() as f32,
        );
        let (actual, _) = vertices_sized(&p, &frame, &draw, false, limited, Mat3::IDENTITY);
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
        serde_json::from_str(include_str!("../../moly-game/tests/data/particle-billboard-camera.json")).unwrap();
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
            scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
        };
        let p = Instance {
            position: array3(&row["position"]),
            velocity: Vec3::ZERO,
            rotation: array3(&row["rotation"]),
            size: array3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
            seed: 0,
            age_percent: 0.0,
            axis: Vec3::Z,
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

/// Research instrument: the engine's Velocity billboard (GenerateParticleGeometry
/// for render space Velocity, 2D and 3D rotation, with and without a pivot)
/// executed in an ARMv8 emulator on the current engine library, its corners
/// read at the vertex writer's entry. Owners are World (identity) or rigid
/// Local; velocities include zero and velocities along the simulation +Z and
/// +Y. Every corner must match within the billboard receipts' 2.5e-5. Point
/// MOLY_VELOCITY_BILLBOARD_NATIVE at the rows; MOLY_VELOCITY_BILLBOARD_MUTANT
/// names a deliberate defect that must fail.
#[test]
#[ignore = "needs MOLY_VELOCITY_BILLBOARD_NATIVE"]
fn velocity_billboard_matches_native_rows() {
    let path = std::env::var("MOLY_VELOCITY_BILLBOARD_NATIVE").expect("MOLY_VELOCITY_BILLBOARD_NATIVE");
    let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["sourceSha256"], "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
    let mutant = std::env::var("MOLY_VELOCITY_BILLBOARD_MUTANT").unwrap_or_default();
    let word = |v: &Value| f32::from_bits(v.as_u64().unwrap() as u32);
    let v3 = |v: &Value| Vec3::new(word(&v[0]), word(&v[1]), word(&v[2]));
    let (mut rows, mut failures, mut maximum) = (0usize, Vec::new(), 0.0f32);
    for (index, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let o: Vec<f32> = row["owner"].as_array().unwrap().iter().map(word).collect();
        let owner = Mat3::from_cols(Vec3::new(o[0], o[1], o[2]), Vec3::new(o[4], o[5], o[6]), Vec3::new(o[8], o[9], o[10]));
        let translation = Vec3::new(o[12], o[13], o[14]);
        let c: Vec<f32> = row["cameraRotation"].as_array().unwrap().iter().map(word).collect();
        let frame = Frame {
            rotation: owner,
            scale: Vec3::ONE,
            camera_rotation: Mat3::from_cols_slice(&c),
            camera_position: v3(&row["cameraPosition"]),
        };
        let local = row["simulation"] == "Local";
        let p = Instance {
            position: owner * v3(&row["position"]) + translation,
            velocity: v3(&row["velocity"]),
            rotation: v3(&row["rotation"]),
            size: v3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
            seed: 0,
            age_percent: 0.0,
            axis: Vec3::Z,
        };
        let draw = Draw {
            mode: Mode::Billboard,
            alignment: if mutant == "facing" { Alignment::Facing } else { Alignment::Velocity },
            pivot: v3(&row["pivot"]),
            screen_size: Vec2::new(0.0, 10000.0),
            allow_roll: true,
            scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
        };
        // Mutants: the simulation rotation left out, the world velocity read
        // as the simulation one, or the degenerate decisions taken after the
        // rotation, in world.
        let simulation = if mutant == "world-z" { Mat3::IDENTITY } else { owner };
        let p = if mutant == "world-velocity" { Instance { velocity: owner * p.velocity, ..p } } else { p };
        let (corners, _) = if mutant == "world-decision" {
            let world = owner * p.velocity;
            let z = if world.length_squared() > 1.0e-30 { world.normalize() } else { owner * Vec3::Z };
            let x = (owner * Vec3::Z).cross(z);
            let x = if x.length_squared() > 1.0e-30 { x.normalize() } else { Vec3::X };
            let q = Instance { velocity: Vec3::ZERO, ..p };
            let d = Draw { alignment: Alignment::Local, ..draw.clone() };
            let f = Frame { rotation: Mat3::from_cols(x, z.cross(x), z), ..frame };
            vertices_sized(&q, &f, &d, true, q.size, Mat3::IDENTITY)
        } else {
            vertices_sized(&p, &frame, &draw, local, p.size, simulation)
        };
        for (k, corner) in corners.iter().enumerate() {
            let error = (*corner - v3(&row["corners"][k])).abs().max_element();
            maximum = maximum.max(error);
            if !error.is_finite() || error > 0.000025 {
                failures.push(format!("row {index} ({}) corner {k}: error {error}", row["entry"]));
            }
        }
        rows += 1;
    }
    println!("velocity billboard: {rows} particles, max corner error {maximum}, {} mismatched corners", failures.len());
    assert!(rows > 0, "no rows");
    assert!(failures.is_empty(), "{} mismatched corners:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Research instrument: the engine's Horizontal and VerticalBillboard geometry
/// (the renderer's span setup, then the shared geometry body of both modes)
/// executed in an ARMv8 emulator on the current engine library, over sampled
/// cameras (including one whose world-to-camera image of world +Z is parallel
/// to world +Y, where the vertical side span is zero), World and rotated Local
/// owners, unit and non-unit renderer scale, pivots and 2D rotation. Every
/// corner must match. Point MOLY_HV_BILLBOARD_NATIVE at the recorded rows;
/// MOLY_HV_BILLBOARD_MUTANT names a deliberate defect that must fail.
#[test]
#[ignore = "needs MOLY_HV_BILLBOARD_NATIVE"]
fn horizontal_and_vertical_billboards_match_native_rows() {
    let path = std::env::var("MOLY_HV_BILLBOARD_NATIVE").expect("MOLY_HV_BILLBOARD_NATIVE");
    let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["sourceSha256"], "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
    let mutant = std::env::var("MOLY_HV_BILLBOARD_MUTANT").unwrap_or_default();
    let word = |v: &Value| f32::from_bits(v.as_u64().unwrap() as u32);
    let v3 = |v: &Value| Vec3::new(word(&v[0]), word(&v[1]), word(&v[2]));
    let (mut rows, mut vertical, mut failures, mut maximum) = (0usize, 0usize, Vec::new(), 0.0f32);
    for (index, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let o: Vec<f32> = row["owner"].as_array().unwrap().iter().map(word).collect();
        let owner = Mat3::from_cols(Vec3::new(o[0], o[1], o[2]), Vec3::new(o[4], o[5], o[6]), Vec3::new(o[8], o[9], o[10]));
        let translation = Vec3::new(o[12], o[13], o[14]);
        let c: Vec<f32> = row["cameraRotation"].as_array().unwrap().iter().map(word).collect();
        let camera_rotation = Mat3::from_cols_slice(&c);
        // Mutants: the camera Z flip of the world-to-camera matrix left out,
        // the Horizontal spans for both modes, or the renderer scale ignored.
        let camera_rotation = if mutant == "unflipped-z" {
            camera_rotation * Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0))
        } else {
            camera_rotation
        };
        let frame = Frame {
            rotation: owner,
            scale: if mutant == "no-scale" { Vec3::ONE } else { v3(&row["scale"]) },
            camera_rotation,
            camera_position: v3(&row["cameraPosition"]),
        };
        let is_vertical = row["mode"] == "VerticalBillboard";
        let p = Instance {
            position: owner * v3(&row["position"]) + translation,
            velocity: Vec3::ZERO,
            rotation: v3(&row["rotation"]),
            size: v3(&row["size"]),
            colour: Vec4::ONE,
            custom1: Vec4::ZERO,
            custom2: Vec4::ZERO,
            seed: 0,
            age_percent: 0.0,
            axis: Vec3::Z,
        };
        let draw = Draw {
            mode: if is_vertical && mutant != "horizontal-spans" { Mode::Vertical } else { Mode::Horizontal },
            alignment: Alignment::View,
            pivot: v3(&row["pivot"]),
            screen_size: Vec2::new(0.0, 10000.0),
            allow_roll: true,
            scaling: crate::particle_geometry::Scaling::Hierarchy, flip: Vec2::ZERO,
        };
        let (corners, _) = vertices_sized(&p, &frame, &draw, row["simulation"] == "Local", p.size, owner);
        for (k, corner) in corners.iter().enumerate() {
            let error = (*corner - v3(&row["corners"][k])).abs().max_element();
            maximum = maximum.max(error);
            if !error.is_finite() || error > 0.000025 {
                failures.push(format!("row {index} ({} {}) corner {k}: error {error}", row["mode"], row["entry"]));
            }
        }
        rows += 1;
        vertical += usize::from(is_vertical);
    }
    println!("horizontal/vertical billboard: {rows} particles ({vertical} vertical), max corner error {maximum}, {} mismatched corners", failures.len());
    assert!(rows > 0 && vertical > 0 && vertical < rows, "both modes must be present");
    assert!(failures.is_empty(), "{} mismatched corners:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The flip draws as the engine's billboard writer computes them for a
/// particle seed: the X draw's xorshift words start at seed + 0x294fb5c7 and
/// seed * 0x6ab51b9d + 0x1b0c434a, the Y draw's at seed + 0x00d4cb82 and
/// seed * 0x6ab51b9d + 0x1c2c4ff9, and a draw keeps the low 23 bits times
/// 0x34000001. Its select network then gives the vertex corners from the two
/// comparisons.
#[test]
fn billboard_flip_draws_and_corners_follow_the_engine_writer() {
    let draw = |t: u32, w: u32| {
        let t = t ^ (t << 11);
        (((t ^ (t >> 8) ^ w ^ (w >> 19)) & 0x007f_ffff) as f32) * f32::from_bits(0x3400_0001)
    };
    // The writer's select network: four lanes p0..p3 take (1, 0, 3, 2) when
    // X mirrors and (0, 1, 2, 3) when not; vertex k then takes p[k], or
    // p[3 - k] when Y mirrors.
    let network = |x: bool, y: bool| {
        let p = if x { [1, 0, 3, 2] } else { [0, 1, 2, 3] };
        std::array::from_fn::<usize, 4, _>(|k| if y { p[3 - k] } else { p[k] })
    };
    let mut flipped = [0usize; 4];
    for index in 0..4096u32 {
        let seed = index.wrapping_mul(0x9e37_79b9) ^ (index << 7);
        let x_draw = draw(seed.wrapping_add(0x294f_b5c7), seed.wrapping_mul(0x6ab5_1b9d).wrapping_add(0x1b0c_434a));
        let y_draw = draw(seed.wrapping_add(0x00d4_cb82), seed.wrapping_mul(0x6ab5_1b9d).wrapping_add(0x1c2c_4ff9));
        for (slot, flip) in [Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.25), Vec2::new(0.7, 0.6), Vec2::new(1.0, 0.0)]
            .into_iter()
            .enumerate()
        {
            let expected = network(flip.x > x_draw, flip.y > y_draw);
            assert_eq!(super::flip_corners(seed, flip), expected, "seed {seed:#x} flip {flip}");
            flipped[slot] += usize::from(expected != [0, 1, 2, 3]);
        }
        assert_eq!(super::flip_corners(seed, Vec2::ZERO), [0, 1, 2, 3]);
        assert_eq!(super::flip_corners(seed, Vec2::new(f32::NAN, f32::NAN)), [0, 1, 2, 3]);
    }
    // Every case mirrors some particles and leaves others (1.0 mirrors all
    // but a draw that rounds to 1.0).
    assert!(flipped.iter().all(|&n| n > 0), "{flipped:?}");
    assert!(flipped[..3].iter().all(|&n| n < 4096), "{flipped:?}");
}
