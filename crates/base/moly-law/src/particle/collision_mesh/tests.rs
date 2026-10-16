//! Replay of the scene side against the engine's own rows: the cooked
//! meshes, the sphere sweeps (the processHit face order and every returned
//! field), the box overlaps and the world bounds; for the BV4 midphase also
//! the triSphereSweep call order, the MTD midphase's report order and its
//! triangle-box test count, and the triangle-box order of the box query.

use super::cook::Midphase;
use super::*;
use crate::particle::json::{parse, Value};
use std::collections::BTreeMap;

fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
    v.get(key).unwrap_or_else(|| panic!("row field {key}"))
}
fn items(v: &Value) -> &[Value] {
    v.as_array().expect("row array")
}
fn word(v: &Value) -> u32 {
    let x = v.as_f64().expect("row word");
    assert!(x.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&x), "row word {x}");
    x as u32
}
fn words(v: &Value) -> Vec<u32> {
    items(v).iter().map(word).collect()
}
fn f(u: u32) -> f32 {
    f32::from_bits(u)
}
fn v3(w: &[u32]) -> [f32; 3] {
    [f(w[0]), f(w[1]), f(w[2])]
}
fn bits3(v: [f32; 3]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}
fn pose(v: &Value) -> Pose {
    let w = words(v);
    Pose::new([f(w[0]), f(w[1]), f(w[2]), f(w[3])], v3(&w[4..])).expect("row pose")
}

fn cook_row(m: &Value) -> CookedMesh {
    let positions: Vec<[f32; 3]> = words(field(m, "positionBits")).chunks_exact(3).map(v3).collect();
    let triangles: Vec<[u32; 3]> = words(field(m, "triangles")).chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    cook(&positions, &triangles, word(field(m, "cookingOptions"))).expect("cook")
}

/// Every cooked family that differs from the engine's cooked mesh.
fn cook_mismatch(mesh: &CookedMesh, native: &Value) -> Vec<&'static str> {
    let mut bad = Vec::new();
    if mesh.vertices.iter().flat_map(|v| bits3(*v)).collect::<Vec<_>>() != words(field(native, "vertexBits")) {
        bad.push("vertices");
    }
    if mesh.triangles.iter().flatten().copied().collect::<Vec<_>>() != words(field(native, "triangles")) {
        bad.push("triangles");
    }
    if mesh.extra.iter().map(|&x| u32::from(x)).collect::<Vec<_>>() != words(field(native, "extra")) {
        bad.push("extra");
    }
    if [bits3(mesh.center), bits3(mesh.extents)].concat() != words(field(native, "centerExtentsBits")) {
        bad.push("centerExtents");
    }
    let Midphase::Bvh33(rtree) = &mesh.midphase else {
        bad.push("midphase");
        return bad;
    };
    let pages: Vec<u32> = rtree.pages.iter().flat_map(|p| {
        p.min.iter().flatten().map(|x| x.to_bits()).chain(p.max.iter().flatten().map(|x| x.to_bits()))
            .chain(p.ptrs.iter().copied()).collect::<Vec<_>>()
    }).collect();
    if pages != words(field(native, "pages")) {
        bad.push("pages");
    }
    if rtree.num_levels != word(field(native, "numLevels")) || rtree.total_nodes != word(field(native, "totalNodes")) {
        bad.push("header");
    }
    if [bits3(rtree.tree_min), bits3(rtree.tree_max)].concat() != words(field(native, "boundsBits")) {
        bad.push("treeBounds");
    }
    bad
}

/// A sweep row through the port: the families that differ, and whether any
/// returned field (everything but the processHit face order) differs.
fn sweep_mismatch(meshes: &[CookedMesh], s: &Value) -> (Vec<&'static str>, bool) {
    let mesh = &meshes[word(field(s, "mesh")) as usize];
    let mut trace = SweepTrace::default();
    let got = sweep_sphere(mesh, &pose(field(s, "poseBits")), v3(&words(field(s, "originBits"))),
        f(word(field(s, "radiusBits"))), v3(&words(field(s, "dirBits"))), f(word(field(s, "distanceBits"))),
        Some(&mut trace));
    let mut bad = Vec::new();
    let got = match got {
        Ok(got) => got,
        Err(_) => return (vec!["refused"], true),
    };
    if trace.leaf_faces != words(field(s, "faces")) {
        bad.push("faces");
    }
    hit_mismatch(got, s, &mut bad);
    let outputs = bad.iter().any(|&b| b != "faces");
    (bad, outputs)
}

/// The returned fields of a sweep against the row's.
fn hit_mismatch(got: Option<MeshSweepHit>, s: &Value, bad: &mut Vec<&'static str>) {
    let native_hit = field(s, "hit").as_bool().expect("hit flag");
    match (got, native_hit) {
        (None, false) => {}
        (Some(hit), true) => {
            if hit.flags != word(field(s, "flags")) {
                bad.push("flags");
            }
            if hit.face_index != word(field(s, "faceIndex")) {
                bad.push("face");
            }
            if hit.distance.to_bits() != word(field(s, "outDistanceBits")) {
                bad.push("distance");
            }
            if bits3(hit.normal) != words(field(s, "normalBits")) {
                bad.push("normal");
            }
            let native_position = field(s, "positionBits");
            let position_ok = match (hit.position, native_position.as_array()) {
                (Some(p), Some(_)) => bits3(p) == words(native_position),
                (None, None) => true,
                _ => false,
            };
            if !position_ok {
                bad.push("position");
            }
        }
        _ => bad.push("hit"),
    }
}

fn overlap_mismatch(meshes: &[CookedMesh], o: &Value) -> bool {
    let mesh = &meshes[word(field(o, "mesh")) as usize];
    let got = overlap_box(mesh, &pose(field(o, "poseBits")), v3(&words(field(o, "centerBits"))),
        v3(&words(field(o, "extentsBits"))));
    got != Ok(field(o, "overlap").as_bool().expect("overlap flag"))
}

fn bounds_mismatch(meshes: &[CookedMesh], b: &Value) -> bool {
    let mesh = &meshes[word(field(b, "mesh")) as usize];
    world_bounds(mesh, &pose(field(b, "poseBits"))).iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        != words(field(b, "bounds"))
}

const ARMS: [&str; 3] = ["sourceOrder", "absoluteKeepEpsilon", "skipMtd"];

#[derive(Default)]
struct Replayed {
    failures: Vec<String>,
    per_source: BTreeMap<String, (usize, usize)>,
    meshes: usize,
    sweeps: usize,
    hits: usize,
    initial: usize,
    zero_length: usize,
    overlaps: usize,
    overlap_true: usize,
    bounds: usize,
    /// Per arm: rows red on any field, rows red on a returned field.
    red: BTreeMap<&'static str, (usize, usize)>,
}

fn replay_file(path: &str, out: &mut Replayed) {
    let doc = parse(&std::fs::read(path).expect("read rows")).expect("parse rows");
    arms::set(None);
    let meshes: Vec<CookedMesh> = items(field(&doc, "meshes")).iter().map(cook_row).collect();
    for (m, row) in meshes.iter().zip(items(field(&doc, "meshes"))) {
        let bad = cook_mismatch(m, field(row, "native"));
        if !bad.is_empty() {
            out.failures.push(format!("cook {}: {bad:?}", field(row, "geometry").as_str().unwrap_or("?")));
        }
    }
    let sweeps = items(field(&doc, "sweeps"));
    let overlaps = items(field(&doc, "overlaps"));
    let bounds = items(field(&doc, "bounds"));
    assert!(!meshes.is_empty() && !sweeps.is_empty());
    out.meshes = out.meshes.max(meshes.len());
    for s in sweeps {
        let (bad, _) = sweep_mismatch(&meshes, s);
        let entry = out.per_source.entry(field(s, "source").as_str().unwrap_or("?").to_owned()).or_default();
        entry.0 += 1;
        out.sweeps += 1;
        if field(s, "hit").as_bool() == Some(true) {
            out.hits += 1;
            out.initial += usize::from(word(field(s, "flags")) & sweep::FLAG_POSITION == 0
                || f(word(field(s, "outDistanceBits"))) <= 0.0);
        }
        out.zero_length += usize::from(word(field(s, "distanceBits")) == 0);
        if !bad.is_empty() {
            entry.1 += 1;
            out.failures.push(format!("sweep {:?}: {bad:?}", field(s, "source").as_str()));
        }
    }
    out.overlaps += overlaps.len();
    out.overlap_true += overlaps.iter().filter(|o| field(o, "overlap").as_bool() == Some(true)).count();
    out.bounds += bounds.len();
    let overlap_bad = overlaps.iter().filter(|o| overlap_mismatch(&meshes, o)).count();
    let bounds_bad = bounds.iter().filter(|b| bounds_mismatch(&meshes, b)).count();
    if overlap_bad > 0 {
        out.failures.push(format!("overlaps: {overlap_bad} differ"));
    }
    if bounds_bad > 0 {
        out.failures.push(format!("bounds: {bounds_bad} differ"));
    }
    for arm in ARMS {
        arms::set(Some(arm));
        let entry = out.red.entry(arm).or_default();
        for s in sweeps {
            let (bad, outputs) = sweep_mismatch(&meshes, s);
            entry.0 += usize::from(!bad.is_empty());
            entry.1 += usize::from(outputs);
        }
        let other = overlaps.iter().filter(|o| overlap_mismatch(&meshes, o)).count()
            + bounds.iter().filter(|b| bounds_mismatch(&meshes, b)).count();
        entry.0 += other;
        entry.1 += other;
    }
    arms::set(None);
}

/// The engine's rows for the ground meshes the collider export ships with
/// the default cooking (natively cooked; sweeps along fall, near-surface,
/// resting and arbitrary directions, zero-length sweeps, the module-driven
/// sweeps and the resting chains of the fall system, box overlaps through
/// the scene narrowphase, world bounds; poses with signed-zero and
/// translated variants; with `MOLY_SCENE_ROWS_KEEP`, long sweeps where two
/// triangles' hits lie between the absolute and the relative keepTriangle
/// epsilon apart): the port must equal every one on every field. Each
/// variant (source-order triangles instead of the cooked leaf order, an
/// absolute keepTriangle epsilon, no MTD) must differ on some row.
#[test]
#[ignore = "needs MOLY_SCENE_ROWS"]
fn scene_rows_match_native_bits() {
    let mut out = Replayed::default();
    let path = std::env::var("MOLY_SCENE_ROWS").expect("MOLY_SCENE_ROWS");
    replay_file(&path, &mut out);
    assert!(out.overlaps > 0 && out.bounds > 0);
    if let Ok(keep) = std::env::var("MOLY_SCENE_ROWS_KEEP") {
        replay_file(&keep, &mut out);
    }
    println!("scene replay: {} meshes cooked, {} sweeps ({} hits, {} hits at an initial overlap, {} zero-length), \
        per source (rows, mismatched) {:?}, {} overlaps ({} true), {} bounds, failures {}; \
        arms red (any field, returned fields) {:?}",
        out.meshes, out.sweeps, out.hits, out.initial, out.zero_length, out.per_source, out.overlaps,
        out.overlap_true, out.bounds, out.failures.len(), out.red);
    for failure in out.failures.iter().take(20) {
        println!("  {failure}");
    }
    assert!(out.failures.is_empty(), "{} scene rows differ from native", out.failures.len());
    for arm in ARMS {
        assert!(out.red[arm].0 > 0, "arm {arm} never differs from native");
    }
}

/// Panic freedom on finite inputs outside the rows: extreme magnitudes,
/// huge boxes whose traversal reaches the empty nodes' words, degenerate
/// triangles. Only that each call returns is checked, not what it returns.
#[test]
fn finite_extremes_return_without_panicking() {
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    let mut positions = Vec::new();
    for z in 0..6 {
        for x in 0..6 {
            positions.push([x as f32 - 2.5, (next() % 1000) as f32 * 1e-3, z as f32 - 2.5]);
        }
    }
    let mut triangles = Vec::new();
    for z in 0..5u32 {
        for x in 0..5u32 {
            let i = z * 6 + x;
            triangles.push([i, i + 6, i + 1]);
            triangles.push([i + 1, i + 6, i + 7]);
        }
    }
    // A degenerate sliver and a repeated corner on no grid edge.
    triangles.push([0, 1, 2]);
    triangles.push([35, 35, 30]);
    let mesh = cook(&positions, &triangles, 0).expect("synthetic mesh cooks");
    let bv4 = cook(&positions, &triangles, 30).expect("synthetic mesh cooks with the options 30");
    let pose = Pose::new([0.0, -0.0, 0.0, 1.0], [0.25, -0.5, 1e3]).unwrap();
    let turned = Pose::rotated([0.3, -0.5, 0.1, 0.806_225_8], [0.25, -0.5, 1e3]).unwrap();
    let pick = |w: u32| -> f32 {
        match w % 8 {
            0 => f32::MAX,
            1 => -f32::MAX,
            2 => f32::MIN_POSITIVE,
            3 => 0.0,
            4 => -0.0,
            5 => 1e-30,
            6 => 3e37,
            _ => f32::from_bits(w & 0x7f7f_ffff) - 1e3,
        }
    };
    for _ in 0..4000 {
        let c = [pick(next()), pick(next()), pick(next())];
        let d = [pick(next()), pick(next()), pick(next())];
        let r = pick(next()).abs().max(1e-6);
        let dist = pick(next()).abs();
        let _ = sweep_sphere(&mesh, &pose, c, r, d, dist, None);
        let _ = overlap_box(&mesh, &pose, c, [f32::MAX, pick(next()).abs(), f32::MAX]);
        let _ = world_bounds(&mesh, &pose);
        for p in [&pose, &turned] {
            let _ = sweep_sphere(&bv4, p, c, r, d, dist, Some(&mut SweepTrace::default()));
            let _ = overlap_oriented_box(&bv4, p, c, [f32::MAX, pick(next()).abs(), f32::MAX], [0.0, 0.6, 0.0, 0.8], None);
            let _ = world_bounds(&bv4, p);
        }
    }
    // A mesh of fewer than five triangles has no BV4 nodes (brute force).
    let small = cook(&positions, &triangles[..3], 30).expect("small mesh cooks");
    let _ = sweep_sphere(&small, &turned, [0.0, 1.0, 0.0], 0.1, [0.0, -1.0, 0.0], 5.0, None);
    let _ = overlap_box(&small, &pose, [0.0; 3], [1.0; 3]);
}

// ------------------------------------------------------------------ BV4

fn rotated_pose(v: &Value) -> Pose {
    let w = words(v);
    Pose::rotated([f(w[0]), f(w[1]), f(w[2]), f(w[3])], v3(&w[4..])).expect("row pose")
}

/// Every cooked family of an options-30 cook that differs from the engine's.
fn bv4_cook_mismatch(mesh: &CookedMesh, native: &Value) -> Vec<&'static str> {
    let mut bad = Vec::new();
    if mesh.vertices.iter().flat_map(|v| bits3(*v)).collect::<Vec<_>>() != words(field(native, "vertexBits")) {
        bad.push("vertices");
    }
    if mesh.triangles.iter().flatten().copied().collect::<Vec<_>>() != words(field(native, "triangles")) {
        bad.push("triangles");
    }
    if mesh.extra.iter().map(|&x| u32::from(x)).collect::<Vec<_>>() != words(field(native, "extra")) {
        bad.push("extra");
    }
    if mesh.source_index != words(field(native, "faceRemap")) {
        bad.push("faceRemap");
    }
    if [bits3(mesh.center), bits3(mesh.extents)].concat() != words(field(native, "centerExtentsBits")) {
        bad.push("centerExtents");
    }
    let Midphase::Bv4(tree) = &mesh.midphase else {
        bad.push("midphase");
        return bad;
    };
    if tree.geom_epsilon.to_bits() != word(field(native, "geomEpsilonBits")) {
        bad.push("geomEpsilon");
    }
    if tree.local_bounds.iter().map(|x| x.to_bits()).collect::<Vec<_>>() != words(field(native, "localBoundsBits")) {
        bad.push("localBounds");
    }
    let nb_nodes = word(field(native, "nbNodes"));
    if tree.nb_nodes != nb_nodes || tree.init_data != word(field(native, "initData")) {
        bad.push("header");
    }
    if bits3(tree.min_coeff) != words(field(native, "minCoeffBits")) || bits3(tree.max_coeff) != words(field(native, "maxCoeffBits")) {
        bad.push("coefficients");
    }
    if tree.words != words(field(native, "nodeWords")) {
        bad.push("nodeWords");
    }
    if nb_nodes != 0 && word(field(native, "quantized")) != 1 {
        bad.push("quantized");
    }
    bad
}

/// A BV4 sweep row: the families that differ, and whether any returned
/// field (not a trace) differs.
fn bv4_sweep_mismatch(meshes: &[CookedMesh], s: &Value) -> (Vec<&'static str>, bool) {
    let mesh = &meshes[word(field(s, "mesh")) as usize];
    let mut trace = SweepTrace::default();
    let got = sweep_sphere(mesh, &rotated_pose(field(s, "poseBits")), v3(&words(field(s, "originBits"))),
        f(word(field(s, "radiusBits"))), v3(&words(field(s, "dirBits"))), f(word(field(s, "distanceBits"))),
        Some(&mut trace));
    let got = match got {
        Ok(got) => got,
        Err(_) => return (vec!["refused"], true),
    };
    let mut bad = Vec::new();
    hit_mismatch(got, s, &mut bad);
    let outputs = !bad.is_empty();
    if trace.leaf_faces != words(field(s, "triOrder")) {
        bad.push("triOrder");
    }
    if trace.mtd_faces != words(field(s, "mtdOrder")) {
        bad.push("mtdOrder");
    }
    if trace.mtd_box_tests != word(field(s, "mtdTriBoxCalls")) as usize {
        bad.push("mtdTriBoxCalls");
    }
    (bad, outputs)
}

/// A BV4 box row: the result and, where the row carries it, the corners of
/// every tested triangle in call order.
fn bv4_overlap_mismatch(meshes: &[CookedMesh], o: &Value) -> Vec<&'static str> {
    let mesh = &meshes[word(field(o, "mesh")) as usize];
    let rot = words(field(o, "rotBits"));
    let mut tested = Vec::new();
    let got = overlap_oriented_box(mesh, &rotated_pose(field(o, "poseBits")), v3(&words(field(o, "centerBits"))),
        v3(&words(field(o, "extentsBits"))), [f(rot[0]), f(rot[1]), f(rot[2]), f(rot[3])], Some(&mut tested));
    let mut bad = Vec::new();
    if got != Ok(field(o, "overlap").as_bool().expect("overlap flag")) {
        bad.push("overlap");
    }
    let order = field(o, "triBoxOrder");
    if order.as_array().is_some() {
        let ours: Vec<u32> = tested.iter().flat_map(|&t| mesh.triangles[t as usize]).collect();
        let theirs: Vec<u32> = items(order).iter().flat_map(words).collect();
        if ours != theirs {
            bad.push("triBoxOrder");
        }
    }
    bad
}

fn bv4_bounds_mismatch(meshes: &[CookedMesh], b: &Value) -> bool {
    let mesh = &meshes[word(field(b, "mesh")) as usize];
    world_bounds(mesh, &rotated_pose(field(b, "poseBits"))).iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        != words(field(b, "bounds"))
}

/// The BV4 mutants: triangles left in the cleaner's order under the tree
/// (no remapTopology), the box traversal's lanes 0 to 3, the sweep's
/// distance gate strict (the maxT == 0 path loses every hit), the sweep's
/// sorted pushes reversed.
const BV4_ARMS: [&str; 4] = ["bv4NoRemap", "bv4LanesForward", "bv4StrictMaxDist", "bv4PnsReversed"];

#[derive(Default, Debug)]
struct Bv4Replayed {
    failures: Vec<String>,
    reasons: BTreeMap<&'static str, usize>,
    per_source: BTreeMap<String, (usize, usize)>,
    meshes: usize,
    sweeps: usize,
    hits: usize,
    negative: usize,
    zero_distance: usize,
    tri_calls: usize,
    mtd_reports: usize,
    mtd_box_tests: usize,
    overlaps: usize,
    overlap_true: usize,
    tri_box_order_rows: usize,
    bounds: usize,
}

/// One pass over the rows with the current arm; for the unarmed pass the
/// counts are kept.
fn bv4_pass(doc: &Value, out: &mut Bv4Replayed) -> (usize, usize, usize, usize, usize) {
    let mesh_rows = items(field(doc, "meshes"));
    let meshes: Vec<CookedMesh> = mesh_rows.iter().map(cook_row).collect();
    let mut red = (0, 0, 0, 0, 0);
    for (m, row) in meshes.iter().zip(mesh_rows) {
        let bad = bv4_cook_mismatch(m, field(row, "native"));
        if !bad.is_empty() {
            red.0 += 1;
            for r in &bad {
                *out.reasons.entry(r).or_default() += 1;
            }
            out.failures.push(format!("cook {}: {bad:?}", field(row, "name").as_str().unwrap_or("?")));
        }
    }
    out.meshes = meshes.len();
    for s in items(field(doc, "sweeps")) {
        let (bad, outputs) = bv4_sweep_mismatch(&meshes, s);
        let entry = out.per_source.entry(field(s, "source").as_str().unwrap_or("?").to_owned()).or_default();
        entry.0 += 1;
        out.sweeps += 1;
        if field(s, "hit").as_bool() == Some(true) {
            out.hits += 1;
            out.negative += usize::from(f(word(field(s, "outDistanceBits"))) < 0.0);
        }
        out.zero_distance += usize::from(word(field(s, "distanceBits")) == 0);
        out.tri_calls += items(field(s, "triOrder")).len();
        out.mtd_reports += items(field(s, "mtdOrder")).len();
        out.mtd_box_tests += word(field(s, "mtdTriBoxCalls")) as usize;
        if !bad.is_empty() {
            red.1 += 1;
            red.2 += usize::from(outputs);
            entry.1 += 1;
            for r in &bad {
                *out.reasons.entry(r).or_default() += 1;
            }
            out.failures.push(format!("sweep {:?}: {bad:?}", field(s, "source").as_str()));
        }
    }
    for o in items(field(doc, "overlaps")) {
        out.overlaps += 1;
        out.overlap_true += usize::from(field(o, "overlap").as_bool() == Some(true));
        out.tri_box_order_rows += usize::from(field(o, "triBoxOrder").as_array().is_some());
        let bad = bv4_overlap_mismatch(&meshes, o);
        if !bad.is_empty() {
            red.3 += 1;
            for r in &bad {
                *out.reasons.entry(r).or_default() += 1;
            }
            out.failures.push(format!("overlap {:?}: {bad:?}", field(o, "source").as_str()));
        }
    }
    for b in items(field(doc, "bounds")) {
        out.bounds += 1;
        if bv4_bounds_mismatch(&meshes, b) {
            red.4 += 1;
            out.failures.push(format!("bounds {:?}", field(b, "source").as_str()));
        }
    }
    red
}

/// The engine's rows for the options-30 cook (31 meshes: the export's home
/// and flowergarden ground meshes of both regions and coverage meshes with
/// duplicate, repeated-index, collinear and tiny inputs), the sphere sweeps
/// with their triSphereSweep order, the MTD midphase's report order and its
/// triangle-box count (initial overlaps, distance-zero sweeps, resting
/// chains, signed-zero directions, rotated and translated poses), the box
/// queries with their triangle-box order where recorded, and the world
/// bounds: the port must equal every one on every field. Each mutant must
/// differ on some row.
#[test]
#[ignore = "needs MOLY_BV4_ROWS"]
fn bv4_rows_match_native_bits() {
    let path = std::env::var("MOLY_BV4_ROWS").expect("MOLY_BV4_ROWS");
    let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
    assert_eq!(word(field(&doc, "hitFlags")), 0x202, "the rows' hit flags are the product's");
    assert_eq!(word(field(&doc, "inflationBits")), 0, "the rows sweep with zero inflation");
    arms::set(None);
    let mut out = Bv4Replayed::default();
    let _ = bv4_pass(&doc, &mut out);
    let mut red = BTreeMap::new();
    for arm in BV4_ARMS {
        arms::set(Some(arm));
        let mut scratch = Bv4Replayed::default();
        red.insert(arm, bv4_pass(&doc, &mut scratch));
    }
    arms::set(None);
    println!("bv4 replay: {} meshes cooked, {} sweeps ({} hits, {} negative distances, {} with distance 0), per source \
        (rows, mismatched) {:?}, {} triSphereSweep calls, {} MTD reports, {} MTD triangle-box tests, {} overlaps \
        ({} true, {} with the triangle-box order), {} bounds, failures {} {:?}; arms red (meshes, sweeps, sweeps on \
        returned fields, overlaps, bounds) {:?}",
        out.meshes, out.sweeps, out.hits, out.negative, out.zero_distance, out.per_source, out.tri_calls,
        out.mtd_reports, out.mtd_box_tests, out.overlaps, out.overlap_true, out.tri_box_order_rows, out.bounds,
        out.failures.len(), out.reasons, red);
    for failure in out.failures.iter().take(20) {
        println!("  {failure}");
    }
    assert!(out.failures.is_empty(), "{} BV4 rows differ from native", out.failures.len());
    assert!(out.tri_calls > 0 && out.mtd_reports > 0 && out.tri_box_order_rows > 0);
    for arm in BV4_ARMS {
        let r = red[arm];
        assert!(r.0 + r.1 + r.3 + r.4 > 0, "arm {arm} never differs from native");
    }
}
