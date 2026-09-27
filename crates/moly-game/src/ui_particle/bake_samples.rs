//! The canvas bake against an independent float64 transcription of the
//! source's C# order on sampled synthetic inputs (the generator is
//! `tests/data/ui_particle_bake.py`, its output `ui-particle-bake.json`):
//! `ModifyScale`, `BakingCamera.GetCamera`'s orthographic size, the extra
//! world simulation of World systems, and a baked point carried through
//! `BakeMesh`'s matrix and the UIParticle node's frame into the root canvas.
//! A term of that order changed in the transcription changes the reference
//! values and turns these red; the last test checks that the samples tell a
//! wrong composition order apart at all.

use bevy::math::{Mat4, Quat, Vec3};
use serde_json::Value;

use super::bake::{self, Node, Space};

fn data() -> Value {
    serde_json::from_str(include_str!("../../tests/data/ui-particle-bake.json"))
        .expect("bake reference values")
}

fn number(v: &Value) -> f32 {
    v.as_f64().expect("number") as f32
}

fn v3(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}

fn q(v: &Value) -> Quat {
    Quat::from_xyzw(number(&v[0]), number(&v[1]), number(&v[2]), number(&v[3]))
}

/// The frames of one bake sample: the root canvas, the UIParticle node, the
/// system node (when it is not the UIParticle node), the space and the scale
/// `BakeMesh` multiplies in last.
fn frames(sample: &Value) -> (Node, Node, Option<Node>, Space, Vec3) {
    let canvas = &sample["canvas"];
    let c = Vec3::splat(number(&canvas["scale"]));
    let canvas_root = Node::root(v3(&canvas["position"]), q(&canvas["rotation"]), c);
    let mut node = canvas_root;
    for link in sample["links"].as_array().unwrap() {
        node = node.child(
            v3(&link["position"]),
            q(&link["rotation"]),
            v3(&link["scale"]),
        );
    }
    let ignore = sample["ignoreCanvasScaler"].as_bool().unwrap();
    let ui = &sample["uiParticle"];
    let serialized = v3(&ui["serializedScale"]);
    let own = if ignore {
        bake::driven_scale(serialized, c)
    } else {
        serialized
    };
    let root = node.child(v3(&ui["position"]), q(&ui["rotation"]), own);
    let links = sample["systemLinks"].as_array().unwrap();
    let system = links.iter().fold(root, |node, link| {
        node.child(
            v3(&link["position"]),
            q(&link["rotation"]),
            v3(&link["scale"]),
        )
    });
    let space = match sample["space"].as_str().unwrap() {
        "Local" => Space::Local,
        "World" => Space::World,
        other => panic!("space {other}"),
    };
    let scale = bake::bake_scale(ignore, c, v3(&sample["scale3d"]));
    (
        canvas_root,
        root,
        (!links.is_empty()).then_some(system),
        space,
        scale,
    )
}

/// The largest error of `transform` over the bake samples, relative to each
/// expected point's largest component (at least 1).
fn bake_error(
    transform: impl Fn(&Node, &Node, Option<&Node>, Space, Vec3) -> Mat4,
) -> (f32, usize) {
    let data = data();
    let samples = data["bake"].as_array().unwrap();
    let mut worst = 0.0f32;
    for sample in samples {
        let (canvas_root, root, system, space, scale) = frames(sample);
        let got = transform(&canvas_root, &root, system.as_ref(), space, scale)
            .transform_point3(v3(&sample["point"]));
        let expected = v3(&sample["expected"]);
        let error = (got - expected).abs().max_element() / expected.abs().max_element().max(1.0);
        worst = worst.max(error);
    }
    (worst, samples.len())
}

#[test]
fn modify_scale_matches_the_transcription() {
    let data = data();
    for (i, sample) in data["modifyScale"].as_array().unwrap().iter().enumerate() {
        let got = bake::driven_scale(v3(&sample["current"]), v3(&sample["canvasScale"]));
        assert_eq!(got, v3(&sample["expected"]), "ModifyScale sample {i}");
    }
}

#[test]
fn orthographic_size_matches_the_transcription() {
    let data = data();
    for (i, sample) in data["orthographicSize"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let rect = [number(&sample["rect"][0]), number(&sample["rect"][1])];
        let camera = bake::baking_camera(rect, number(&sample["scaleFactor"]), None);
        assert_eq!(
            camera.orthographic_size,
            number(&sample["expected"]),
            "GetCamera sample {i}"
        );
        assert_eq!(camera.position, Vec3::new(0.0, 0.0, -1000.0));
        assert_eq!(camera.far, 2000.0);
    }
}

#[test]
fn world_displacement_matches_the_transcription() {
    let data = data();
    for (i, sample) in data["worldDisplacement"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let got = bake::world_displacement(
            v3(&sample["position"]),
            v3(&sample["cached"]),
            v3(&sample["scale"]),
            sample["drawnBefore"].as_bool().unwrap(),
        );
        let expected = v3(&sample["expected"]);
        let error = (got - expected).abs().max_element() / expected.abs().max_element().max(1.0);
        assert!(
            error <= 1.0e-5,
            "world displacement sample {i}: {got} vs {expected}"
        );
    }
}

#[test]
fn canvas_bake_matches_the_transcription() {
    let (worst, count) = bake_error(bake::to_canvas);
    println!("[ui-particle] canvas bake: {count} samples, largest relative error {worst:e}");
    assert!(
        worst <= 1.0e-4,
        "canvas bake: largest relative error {worst:e} over {count} samples"
    );
}

#[test]
fn samples_tell_the_scale_side_apart() {
    // The same composition with the bake scale applied before the root
    // matrix instead of after it.
    let (worst, count) = bake_error(|canvas_root, root, system, space, scale| {
        canvas_root.matrix.inverse()
            * root.matrix
            * bake::bake_matrix(root, system, space, Vec3::ONE)
            * Mat4::from_scale(scale)
    });
    println!(
        "[ui-particle] scale on the wrong side: {count} samples, largest relative error {worst:e}"
    );
    assert!(
        worst > 1.0e-2,
        "the samples do not tell the scale side apart ({worst:e})"
    );
}
