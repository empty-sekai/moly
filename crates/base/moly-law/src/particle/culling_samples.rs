//! Research instrument: replays native culling cases (the game's own libunity
//! run in an emulator) through this module's functions, word for
//! word. The inputs and native outputs come from files named by environment
//! variables; nothing here compares against this crate's own output.
//!
//! - `MOLY_CULLING_BOUNDS_NATIVE`: `UpdateBounds` cases (procedural, particle,
//!   stretched and empty paths, World and Custom space, invalidated systems).
//! - `MOLY_CULLING_CURVES_NATIVE`: `CalculateCurveRangesValue` on corpus and
//!   random curves.
//! - `MOLY_CULLING_CAMERA_NATIVE`: the renderer world box, the projection
//!   planes, `CalculateFrustumPlanes`, `CullObjectsWithoutUmbra`, step 1 and
//!   step 2 of the culling job.
use super::*;
use crate::particle::json::{self, Value};
use crate::particle::value::Curve;

const LIBRARY: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

fn load(var: &str) -> Value {
    let path = std::env::var(var).unwrap_or_else(|_| panic!("{var} must name the exported native cases"));
    let doc = json::parse(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(doc.get("librarySha256").and_then(Value::as_str), Some(LIBRARY));
    doc
}

fn u(v: &Value) -> u32 { v.as_f64().unwrap() as u32 }
fn f(v: &Value) -> f32 { f32::from_bits(u(v)) }
fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] { v.get(key).unwrap().as_array().unwrap() }
fn words<const N: usize>(v: &Value) -> [f32; N] {
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), N);
    std::array::from_fn(|i| f(&a[i]))
}
fn vec3s(v: &Value) -> Vec<[f32; 3]> { v.as_array().unwrap().iter().map(words::<3>).collect() }
fn flag(v: &Value, key: &str) -> bool { v.get(key).unwrap().as_bool().unwrap() }

fn keys(v: &Value) -> Vec<CurveKey> {
    v.as_array().unwrap().iter().map(|k| {
        let k = k.as_array().unwrap();
        CurveKey { time: f(&k[0]), value: f(&k[1]), in_slope: f(&k[2]), out_slope: f(&k[3]),
            weighted_mode: u(&k[4]) as u8, in_weight: f(&k[5]), out_weight: f(&k[6]) }
    }).collect()
}

fn curve(v: &Value) -> MinMaxCurve {
    let lane = |key: &str| Curve { multiplier: 1.0, keys: keys(v.get(key).unwrap()), pre_wrap: None, post_wrap: None };
    let (a, b) = (f(v.get("a").unwrap()), f(v.get("b").unwrap()));
    match u(v.get("mode").unwrap()) {
        0 => MinMaxCurve::Constant(b),
        1 => MinMaxCurve::Curve { multiplier: b, max: lane("max") },
        2 => MinMaxCurve::TwoCurves { multiplier: b, min: lane("min"), max: lane("max") },
        3 => MinMaxCurve::TwoConstants { min: a, max: b },
        other => panic!("curve mode {other}"),
    }
}

fn present<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|v| !matches!(v, Value::Null))
}

fn config(i: &Value) -> BoundsConfig {
    let size = list(i, "size");
    BoundsConfig {
        space: [BoundsSpace::Local, BoundsSpace::World, BoundsSpace::Custom][u(i.get("space").unwrap()) as usize],
        has_renderer: flag(i, "hasRenderer"),
        render_mode: [BoundsRenderMode::Billboard, BoundsRenderMode::Stretch, BoundsRenderMode::HorizontalBillboard,
            BoundsRenderMode::VerticalBillboard, BoundsRenderMode::Mesh, BoundsRenderMode::None][u(i.get("renderMode").unwrap()) as usize],
        velocity_scale: f(i.get("velocityScale").unwrap()),
        length_scale: f(i.get("lengthScale").unwrap()),
        pivot: words(i.get("pivot").unwrap()),
        renderer_mesh: words(i.get("rendererMesh").unwrap()),
        lifetime: curve(i.get("lifetime").unwrap()),
        speed: curve(i.get("speed").unwrap()),
        size: [curve(&size[0]), curve(&size[1]), curve(&size[2])],
        start_size_3d: flag(i, "startSize3d"),
        particle_size_3d: flag(i, "particleSize3d"),
        gravity_modifier: f(i.get("gravityModifier").unwrap()),
        shape: present(i, "shape").map(|s| ShapeBounds {
            kind: u(s.get("kind").unwrap()), radius: f(s.get("radius").unwrap()), angle: f(s.get("angle").unwrap()),
            length: f(s.get("length").unwrap()), donut_radius: f(s.get("donutRadius").unwrap()),
            position: words(s.get("position").unwrap()), rotation: words(s.get("rotation").unwrap()),
            scale: words(s.get("scale").unwrap()), random_direction: f(s.get("randomDirection").unwrap()),
            mesh: words(s.get("mesh").unwrap()),
        }),
        velocity: present(i, "velocity").map(|v| VelocityBounds { x: curve(v.get("x").unwrap()),
            y: curve(v.get("y").unwrap()), z: curve(v.get("z").unwrap()), in_world_space: flag(v, "inWorldSpace") }),
        size_module: present(i, "sizeModule").map(|m| {
            let m = m.as_array().unwrap();
            std::array::from_fn(|axis| (!matches!(m[axis], Value::Null)).then(|| curve(&m[axis])))
        }),
        trail: flag(i, "trail"), lights: flag(i, "lights"), force: flag(i, "force"),
        size_by_speed: flag(i, "sizeBySpeed"), uv_sprites: flag(i, "uvSprites"),
    }
}

#[test]
#[ignore = "MOLY_CULLING_BOUNDS_NATIVE must name the exported UpdateBounds cases"]
fn update_bounds_matches_the_native_cases() {
    let doc = load("MOLY_CULLING_BOUNDS_NATIVE");
    let (mut compared, mut refused, mut failures) = (0usize, 0usize, Vec::new());
    let mut paths = std::collections::BTreeMap::<String, usize>::new();
    for case in list(&doc, "cases") {
        let i = case.get("input").unwrap();
        let config = config(i);
        let (position, velocity, animated) = (vec3s(i.get("positions").unwrap()), vec3s(i.get("velocities").unwrap()),
            vec3s(i.get("animated").unwrap()));
        let size_x: Vec<f32> = list(i, "sizeX").iter().map(f).collect();
        let state = BoundsState {
            procedural: flag(i, "procedural"),
            local_to_world: words(i.get("l2w").unwrap()),
            world_to_local: words(i.get("w2l").unwrap()),
            shape_scale: words(i.get("shapeScale").unwrap()),
            scale: words(i.get("scale").unwrap()),
            max_size_tracker: f(i.get("maxSizeTracker").unwrap()),
            gravity: words(i.get("gravity").unwrap()),
            particles: Live { position: &position, velocity: &velocity, animated_velocity: &animated, size_x: &size_x },
        };
        let label = format!("{} {} {}", case.get("family").unwrap().as_str().unwrap(),
            case.get("key").unwrap().as_str().unwrap(), u(case.get("seed").unwrap()));
        let result = update_bounds(&config, &state);
        if case.get("status").unwrap().as_str() != Some("native") {
            // The native run did not cover this case; the port must refuse it.
            match result {
                Err(BoundsRefused::IntegratedCurve) => refused += 1,
                other => failures.push(format!("{label}: expected the integrated-curve refusal, got {other:?}")),
            }
            continue;
        }
        compared += 1;
        *paths.entry(case.get("path").unwrap().as_str().unwrap().to_owned()).or_default() += 1;
        let native: [u32; 6] = std::array::from_fn(|k| u(&list(case, "native")[k]));
        match result {
            Ok(bounds) if bounds.words() == native => {}
            other => failures.push(format!("{label}: {other:?} vs native {native:08x?}")),
        }
    }
    println!("compared {compared} refused {refused} paths {paths:?} failures {}", failures.len());
    assert!(compared > 0 && paths.len() >= 3, "every path must be reached");
    assert!(failures.is_empty(), "{:?}", &failures[..failures.len().min(8)]);
}

#[test]
#[ignore = "MOLY_CULLING_CURVES_NATIVE must name the exported CalculateCurveRangesValue cases"]
fn curve_ranges_match_the_native_kernel() {
    let doc = load("MOLY_CULLING_CURVES_NATIVE");
    let (mut compared, mut failures) = (0usize, Vec::new());
    for (index, case) in list(&doc, "curves").iter().enumerate() {
        let init = list(case, "init");
        let range = curve_ranges_value((f(&init[0]), f(&init[1])), &keys(case.get("keys").unwrap()));
        let native = list(case, "native");
        compared += 1;
        if [range.0.to_bits(), range.1.to_bits()] != [u(&native[0]), u(&native[1])] {
            failures.push(format!("curve {index}: {:08x} {:08x} vs {:08x} {:08x}", range.0.to_bits(), range.1.to_bits(),
                u(&native[0]), u(&native[1])));
        }
    }
    println!("curves {compared} failures {}", failures.len());
    assert!(compared > 6000);
    assert!(failures.is_empty(), "{:?}", &failures[..failures.len().min(8)]);
}

fn centre_box(v: &Value) -> CentreBox {
    let w: [f32; 6] = words(v);
    CentreBox { centre: [w[0], w[1], w[2]], extents: [w[3], w[4], w[5]] }
}

fn plane_list(v: &Value) -> Vec<Plane> { v.as_array().unwrap().iter().map(words::<4>).collect() }

fn indices(v: &Value) -> Vec<i64> { v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as i64).collect() }

#[test]
#[ignore = "MOLY_CULLING_CAMERA_NATIVE must name the exported renderer and camera cases"]
fn renderer_box_and_camera_test_match_the_native_kernels() {
    let doc = load("MOLY_CULLING_CAMERA_NATIVE");
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for case in list(&doc, "worldBounds") {
        let c = case.get("case").unwrap();
        let aabb: [f32; 6] = words(c.get("aabb").unwrap());
        let bounds = Bounds { min: [aabb[0], aabb[1], aabb[2]], max: [aabb[3], aabb[4], aabb[5]] };
        let space = [BoundsSpace::Local, BoundsSpace::World, BoundsSpace::Custom][u(c.get("space").unwrap()) as usize];
        let (world, local) = world_bounds(&bounds, space, u(c.get("alignment").unwrap()), &words(c.get("l2w").unwrap()),
            &words(c.get("w2l").unwrap()), words(c.get("scale").unwrap()));
        let native = case.get("native").unwrap();
        let want = |key: &str| -> [u32; 6] { std::array::from_fn(|k| u(&list(native, key)[k])) };
        *counts.entry("worldBounds").or_default() += 1;
        if world.words() != want("world") || local.words() != want("local") {
            failures.push(format!("worldBounds {:08x?} {:08x?}", world.words(), local.words()));
        }
    }
    for case in list(&doc, "extractPlanes") {
        let planes = extract_projection_planes(&words(case.get("m").unwrap()));
        let got: Vec<u32> = planes.iter().flatten().map(|v| v.to_bits()).collect();
        let want: Vec<u32> = list(case, "native").iter().map(u).collect();
        *counts.entry("extractPlanes").or_default() += 1;
        if got != want { failures.push(format!("extractPlanes {got:08x?}")); }
    }
    let mut rebuilt = 0usize;
    for case in list(&doc, "frustumPlanes") {
        let camera = FrustumCamera {
            implicit_culling: flag(case, "implicitCulling"),
            implicit_view: flag(case, "implicitView"),
            camera_to_world: present(case, "inverse").map_or([f32::NAN; 16], words),
            near: f(case.get("near").unwrap()),
            far: f(case.get("far").unwrap()),
        };
        rebuilt += usize::from(camera.implicit_culling && camera.implicit_view);
        let (planes, base) = frustum_planes(&words(case.get("m").unwrap()), &camera);
        let native = case.get("native").unwrap();
        let got: Vec<u32> = planes.iter().flatten().map(|v| v.to_bits()).collect();
        let want: Vec<u32> = list(native, "planes").iter().map(u).collect();
        *counts.entry("frustumPlanes").or_default() += 1;
        if got != want || base.to_bits() != u(native.get("baseFar").unwrap()) {
            failures.push(format!("frustumPlanes {got:08x?} {:08x}", base.to_bits()));
        }
    }
    let (mut kept, mut rejected) = (0usize, 0usize);
    for case in list(&doc, "cull") {
        let planes = plane_list(case.get("planes").unwrap());
        let aabbs = case.get("aabbs").unwrap();
        let got: Vec<i64> = indices(case.get("idx").unwrap()).into_iter()
            .filter(|i| inside_planes(&centre_box(aabbs.get(&i.to_string()).unwrap()), &planes)).collect();
        let want = indices(case.get("native").unwrap());
        kept += want.len();
        rejected += case.get("idx").unwrap().as_array().unwrap().len() - want.len();
        *counts.entry("cull").or_default() += 1;
        if got != want { failures.push(format!("cull {got:?} vs {want:?}")); }
    }
    for case in list(&doc, "step1") {
        let lod: Vec<u8> = list(case, "lod").iter().map(|v| u(v) as u8).collect();
        let mask = u(case.get("mask").unwrap());
        let nodes: Vec<CullNode> = list(case, "nodes").iter().map(|n| {
            let n = n.as_array().unwrap();
            CullNode { has_renderer: u(&n[0]) != 0, layer: u(&n[1]), flags: u(&n[2]), renderer_disabled: false, lod_mask: u(&n[3]) as u8 }
        }).collect();
        let (begin, end) = (u(case.get("begin").unwrap()) as usize, u(case.get("end").unwrap()) as usize);
        let got: Vec<i64> = (begin..end).filter(|&i| node_visible_fast(&nodes[i], mask, &lod)).map(|i| i as i64).collect();
        *counts.entry("step1").or_default() += 1;
        if got != indices(case.get("native").unwrap()) { failures.push(format!("step1 {got:?}")); }
    }
    for case in list(&doc, "step2") {
        let mode = [LayerCull::None, LayerCull::Planar, LayerCull::Spherical][u(case.get("mode").unwrap()) as usize];
        let far_normal: [f32; 3] = words(case.get("farNormal").unwrap());
        let dists = list(case, "dists");
        let layers = list(case, "layers");
        let skip = list(case, "skip");
        let campos: [f32; 3] = words(case.get("campos").unwrap());
        let aabbs = case.get("aabbs").unwrap();
        let got: Vec<i64> = indices(case.get("idx").unwrap()).into_iter().filter(|&i| {
            u(&skip[i as usize]) == 0 && layer_visible(&centre_box(aabbs.get(&i.to_string()).unwrap()), mode, far_normal,
                f(&dists[u(&layers[i as usize]) as usize]), campos)
        }).collect();
        *counts.entry("step2").or_default() += 1;
        if got != indices(case.get("native").unwrap()) { failures.push(format!("step2 {got:?}")); }
    }
    println!("{counts:?} frustum rebuilt {rebuilt} cull kept {kept} rejected {rejected} failures {}", failures.len());
    assert!(counts.len() == 6 && rebuilt > 0 && kept > 0 && rejected > 0, "every kernel and branch must be reached");
    assert!(failures.is_empty(), "{:?}", &failures[..failures.len().min(8)]);
}
