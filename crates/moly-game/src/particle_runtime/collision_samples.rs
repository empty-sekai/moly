//! The collision calls of the native birth path against native module rows:
//! the pool enters in runtime axes (X reflected), goes through the product's
//! post-simulation or newborn-group call and comes back compared bit for bit
//! with the engine's written arrays, random words and range words.
use super::collision::{newborn_block, post_simulation, CollisionRuntime};
use super::collision_scene::{GroundQuery, GroundScene, SceneBuilder};
use super::*;
use moly_law::particle::collision_query::{
    Candidate, CollisionLaw, CollisionScene, CollisionState, OverlapQuery, OwnerPair, ParticleFlags, SweepHit,
    SweepRequest,
};
use moly_law::particle::collision_response::{CollisionRandom, CollisionResponse, QueryAffine};
use serde_json::Value;

fn word(v: &Value) -> u32 {
    v.as_u64().expect("row word") as u32
}
fn words(v: &Value) -> Vec<u32> {
    v.as_array().expect("row array").iter().map(word).collect()
}
fn bits(v: &Value) -> f32 {
    f32::from_bits(word(v))
}
fn hex(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().expect("hex word"), 16).expect("hex word")
}

/// The native sweep results in call order.
struct Replay {
    shapes: Vec<Candidate>,
    returns: Vec<Option<SweepHit>>,
    next: usize,
}

impl CollisionScene for Replay {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.shapes.iter().take(query.max_shapes.max(0) as usize).copied().collect()
    }
    fn reachable(&self, _: u32) -> Vec<Candidate> {
        self.shapes.clone()
    }
    fn sweep_sphere(&mut self, _: &SweepRequest) -> Option<SweepHit> {
        let hit = self.returns.get(self.next).copied().flatten();
        self.next += 1;
        hit
    }
}

#[derive(Default, Debug)]
struct Tally {
    post: usize,
    newborn: usize,
    unreached: usize,
    refused_past_end: usize,
    /// Compared calls in which the engine wrote some particle.
    changed: usize,
    outside: std::collections::BTreeMap<&'static str, usize>,
    mismatched: Vec<String>,
}

/// Every native module row whose call shape the product makes (the whole
/// pool with one slice dt, or one newborn group of at most four lanes with
/// the slots past it unknown) and whose flags and edges the product admits.
#[test]
#[ignore = "needs MOLY_COLLISION_ROWS"]
fn product_collision_calls_match_native_rows() {
    let path = std::env::var("MOLY_COLLISION_ROWS").expect("MOLY_COLLISION_ROWS");
    let doc: Value = serde_json::from_slice(&std::fs::read(&path).expect("read rows")).expect("parse rows");
    let rows = doc["rows"].as_array().expect("rows");
    let mut tally = Tally::default();
    for row in rows {
        let flags = words(&row["flags"]);
        let (from, to, count) = (row["from"].as_u64().unwrap() as usize, row["to"].as_u64().unwrap() as usize,
            row["count"].as_u64().unwrap() as usize);
        let dt = words(&row["dt"]);
        let outside = if flags[0] != 0 {
            Some("current-size stream")
        } else if flags[2] != 0 {
            Some("speed modifier")
        } else if flags[3] != 0 {
            Some("collision events on")
        } else if !row["sub"].as_array().unwrap().is_empty() {
            Some("collision sub-emitter edges")
        } else if to == from {
            Some("empty range")
        } else if from == 0 && to == count && dt.iter().all(|&w| w == dt[0]) {
            None
        } else if to - from <= 4 {
            None
        } else {
            Some("call shape the product does not make")
        };
        if let Some(reason) = outside {
            *tally.outside.entry(reason).or_default() += 1;
            continue;
        }
        let post = from == 0 && to == count && dt.iter().all(|&w| w == dt[0]);
        let module = &row["module"];
        let curve = |key: &str| bits(&module[key][1]);
        let response = CollisionResponse::new(curve("bounce"), curve("dampen"), curve("loss"),
            bits(&module["minKill"]), bits(&module["maxKill"])).expect("response in range");
        let world = row["space"].as_u64() == Some(1);
        let law = CollisionLaw::from_words(response, bits(&module["radiusScale"]), word(&module["collidesWith"]),
            word(&module["dynamic"]) != 0, module["maxShapes"].as_i64().unwrap() as i32, world,
            ParticleFlags { current_size: false, size_3d: flags[1] != 0, speed_modifier: false });
        let columns = |key: &str| {
            let w = words(&row[key]);
            QueryAffine::from_columns(&std::array::from_fn(|i| f32::from_bits(w[i])))
        };
        let shapes = row["shapes"].as_array().unwrap().iter().map(|s| {
            let s = words(s);
            Candidate {
                bounds_min: std::array::from_fn(|k| f32::from_bits(s[k])),
                bounds_max: std::array::from_fn(|k| f32::from_bits(s[3 + k])),
                is_trigger: s[6] != 0,
                collider_id: s[7] as i32,
                body_id: None,
            }
        }).collect();
        let returns = row["log"]["sweeps"].as_array().unwrap().iter().map(|s| {
            let r = words(&s["returned"]);
            (r[0] != 0).then(|| SweepHit {
                position: std::array::from_fn(|k| f32::from_bits(r[1 + k])),
                normal: std::array::from_fn(|k| f32::from_bits(r[4 + k])),
                distance: f32::from_bits(r[7]),
            })
        }).collect();
        let mut collision = CollisionRuntime {
            law,
            state: CollisionState { range_words: [hex(&row["s10"]), hex(&row["s18"])], uses_events: false, events: 0 },
            random: Some(CollisionRandom { words: std::array::from_fn(|i| words(&row["rand"])[i]) }),
            owner: (!world).then(|| OwnerPair { local_to_world: columns("owner"), world_to_local: columns("inverse") }),
            scene: Box::new(Replay { shapes, returns, next: 0 }),
            calls: 0,
            hits: 0,
            draws: 0,
            unreached: 0,
        };
        let arrays = &row["arrays"];
        let at = |key: &str, i: usize| f32::from_bits(word(&arrays[key][i]));
        let v3 = |base: u32, i: usize| std::array::from_fn(|k| at(&(base + 32 * k as u32).to_string(), i));
        let reflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let mut system = test_support::runtime();
        let template = system.side[0];
        system.pool = (0..count).map(|i| Particle {
            position: reflect(v3(0, i)),
            velocity: reflect(v3(96, i)),
            start_lifetime: 1.0,
            age_percent: at("960", i),
            inverse_lifetime: at("992", i),
        }).collect();
        system.side = (0..count).map(|i| Side {
            seed: word(&arrays["896"][i]),
            size: v3(672, i),
            animated: reflect(v3(192, i)),
            ..template
        }).collect();
        let pending = bits(&row["s0"]);
        let emission_word = word(&row["s1ec"]);
        let result = if post {
            tally.post += 1;
            post_simulation(&mut system, &mut collision, f32::from_bits(dt[0]), pending, emission_word)
        } else {
            tally.newborn += 1;
            newborn_block(&mut system, &mut collision, from, to, std::array::from_fn(|l| f32::from_bits(dt[l])),
                pending, emission_word)
        };
        let label = format!("{} {} {}", row["receipt"], row["config"], row["scenario"]);
        if let Err(reason) = result {
            if reason.contains("PastEndLanes") {
                tally.refused_past_end += 1;
            } else {
                tally.mismatched.push(format!("{label}: refused {reason}"));
            }
            continue;
        }
        tally.unreached += collision.unreached as usize;
        let after = &row["after"];
        let native = |key: &str| words(&after["arrays"][key]);
        let mut differs = Vec::new();
        for (k, key) in ["0", "32", "64"].iter().enumerate() {
            let sign = if k == 0 { 0x8000_0000 } else { 0 };
            if system.pool.iter().map(|p| p.position[k].to_bits() ^ sign).collect::<Vec<_>>() != native(key) {
                differs.push("position");
            }
        }
        for (k, key) in ["96", "128", "160"].iter().enumerate() {
            let sign = if k == 0 { 0x8000_0000 } else { 0 };
            if system.pool.iter().map(|p| p.velocity[k].to_bits() ^ sign).collect::<Vec<_>>() != native(key) {
                differs.push("velocity");
            }
        }
        if system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>() != native("960") {
            differs.push("age");
        }
        if collision.random.unwrap().words.to_vec() != words(&after["rand"]) {
            differs.push("random");
        }
        if collision.state.range_words != [hex(&after["s10"]), hex(&after["s18"])] {
            differs.push("rangeWords");
        }
        if !differs.is_empty() {
            tally.mismatched.push(format!("{label}: {differs:?}"));
        }
        if ["0", "32", "64", "96", "128", "160", "960"].iter().any(|key| words(&after["arrays"][*key]) != words(&arrays[*key])[..count]) {
            tally.changed += 1;
        }
    }
    println!("product collision replay: post-simulation calls {}, newborn-group calls {} (unreached {}), refused past-end {}, compared calls that wrote a particle {}, outside the product {:?}, mismatched {}",
        tally.post, tally.newborn, tally.unreached, tally.refused_past_end, tally.changed, tally.outside, tally.mismatched.len());
    for line in tally.mismatched.iter().take(20) {
        println!("  {line}");
    }
    assert!(tally.post > 0 && tally.newborn > 0 && tally.changed > 0);
    assert!(tally.mismatched.is_empty());
}

/// The scene's answers, recorded for the comparison with the logged ones.
type SweepLog = std::sync::Arc<std::sync::Mutex<Vec<(SweepRequest, Option<SweepHit>)>>>;

struct Recording {
    inner: GroundQuery,
    log: SweepLog,
}

impl CollisionScene for Recording {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.inner.overlap(query)
    }
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.inner.reachable(collides_with)
    }
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        let hit = self.inner.sweep_sphere(request);
        self.log.lock().unwrap().push((*request, hit));
        hit
    }
    fn refusal(&self) -> Option<&'static str> {
        self.inner.refusal()
    }
}

fn hit_words(hit: &Option<SweepHit>) -> Vec<u32> {
    match hit {
        None => vec![0],
        Some(h) => [1].into_iter().chain(h.position.iter().chain(&h.normal).map(|x| x.to_bits()))
            .chain([h.distance.to_bits()]).collect(),
    }
}

/// The product's own ground scene (built from the collider export by the
/// admission path) in place of the recorded sweep results: every native
/// module row of the natively swept receipt (`MOLY_COLLISION_SCENE_LABEL`
/// names it) whose one collider is an exported ground collider with the
/// default cooking, through the product's post-simulation or newborn-group
/// call. The written arrays, random words, range words and hit records must
/// equal the engine's, and every sweep the scene answers must equal the
/// logged native sweep of the same particle. The receipt's scene lookup
/// listed the collider without the scene narrowphase, so a native sweep the
/// product does not issue (its box touches no triangle) must be a miss.
#[test]
#[ignore = "needs MOLY_COLLISION_ROWS, MOLY_COLLISION_EXPORT and MOLY_COLLISION_SCENE_LABEL"]
fn product_scene_calls_match_native_rows() {
    let read = |key: &str| -> Value {
        let path = std::env::var(key).unwrap_or_else(|_| panic!("{key}"));
        serde_json::from_slice(&std::fs::read(&path).expect("read rows")).expect("parse rows")
    };
    let doc = read("MOLY_COLLISION_ROWS");
    let export = read("MOLY_COLLISION_EXPORT");
    let receipt = std::env::var("MOLY_COLLISION_SCENE_LABEL").expect("MOLY_COLLISION_SCENE_LABEL");
    let effects: std::collections::BTreeSet<String> = export["colliders"].as_array().expect("colliders").iter()
        .filter_map(|c| c["effect"].as_str().map(str::to_owned)).collect();
    let mut builder = SceneBuilder::new(Ok(export));
    let scenes: Vec<(std::sync::Arc<GroundScene>, Vec<u32>)> = effects.iter()
        .filter_map(|effect| builder.for_effect(effect).ok())
        .map(|scene| {
            let c = GroundQuery::new(scene.clone()).reachable(u32::MAX);
            let bits = c[0].bounds_min.iter().chain(&c[0].bounds_max).map(|x| x.to_bits()).collect();
            (scene, bits)
        }).collect();
    assert!(!scenes.is_empty());
    let mut tally = std::collections::BTreeMap::<&str, usize>::new();
    let mut mismatched: Vec<String> = Vec::new();
    for row in doc["rows"].as_array().expect("rows") {
        if row["receipt"].as_str() != Some(receipt.as_str()) {
            continue;
        }
        *tally.entry("receipt rows").or_default() += 1;
        let shapes = row["shapes"].as_array().unwrap();
        let scene = match shapes.as_slice() {
            [shape] if words(shape)[6] == 0 => scenes.iter().find(|(_, bits)| words(shape)[..6] == bits[..]),
            _ => None,
        };
        let Some((scene, _)) = scene else {
            *tally.entry("outside: not one exported default-cooked ground collider").or_default() += 1;
            continue;
        };
        let flags = words(&row["flags"]);
        let (from, to, count) = (row["from"].as_u64().unwrap() as usize, row["to"].as_u64().unwrap() as usize,
            row["count"].as_u64().unwrap() as usize);
        let dt = words(&row["dt"]);
        let post = from == 0 && to == count && dt.iter().all(|&w| w == dt[0]);
        let outside = if flags[0] != 0 {
            Some("outside: current-size stream")
        } else if flags[2] != 0 || flags[3] != 0 || !row["sub"].as_array().unwrap().is_empty() {
            Some("outside: speed modifier, events or collision edges")
        } else if row["space"].as_u64() != Some(1) {
            Some("outside: Local space")
        } else if to == from || (!post && to - from > 4) {
            Some("outside: call shape")
        } else {
            None
        };
        if let Some(reason) = outside {
            *tally.entry(reason).or_default() += 1;
            continue;
        }
        let module = &row["module"];
        let curve = |key: &str| bits(&module[key][1]);
        let response = CollisionResponse::new(curve("bounce"), curve("dampen"), curve("loss"),
            bits(&module["minKill"]), bits(&module["maxKill"])).expect("response in range");
        let law = CollisionLaw::from_words(response, bits(&module["radiusScale"]), word(&module["collidesWith"]),
            word(&module["dynamic"]) != 0, module["maxShapes"].as_i64().unwrap() as i32, true,
            ParticleFlags { current_size: false, size_3d: flags[1] != 0, speed_modifier: false });
        let log: SweepLog = Default::default();
        let mut collision = CollisionRuntime {
            law,
            state: CollisionState { range_words: [hex(&row["s10"]), hex(&row["s18"])], uses_events: false, events: 0 },
            random: Some(CollisionRandom { words: std::array::from_fn(|i| words(&row["rand"])[i]) }),
            owner: None,
            scene: Box::new(Recording { inner: GroundQuery::new(scene.clone()), log: log.clone() }),
            calls: 0,
            hits: 0,
            draws: 0,
            unreached: 0,
        };
        let arrays = &row["arrays"];
        let at = |key: &str, i: usize| f32::from_bits(word(&arrays[key][i]));
        let v3 = |base: u32, i: usize| std::array::from_fn(|k| at(&(base + 32 * k as u32).to_string(), i));
        let reflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let mut system = test_support::runtime();
        let template = system.side[0];
        system.pool = (0..count).map(|i| Particle {
            position: reflect(v3(0, i)),
            velocity: reflect(v3(96, i)),
            start_lifetime: 1.0,
            age_percent: at("960", i),
            inverse_lifetime: at("992", i),
        }).collect();
        system.side = (0..count).map(|i| Side {
            seed: word(&arrays["896"][i]),
            size: v3(672, i),
            animated: reflect(v3(192, i)),
            ..template
        }).collect();
        let (pending, emission_word) = (bits(&row["s0"]), word(&row["s1ec"]));
        let result = if post {
            post_simulation(&mut system, &mut collision, f32::from_bits(dt[0]), pending, emission_word)
        } else {
            newborn_block(&mut system, &mut collision, from, to, std::array::from_fn(|l| f32::from_bits(dt[l])),
                pending, emission_word)
        };
        let label = format!("{} {}", row["config"], row["scenario"]);
        match result {
            Err(reason) if reason.contains("PastEndLanes") => {
                *tally.entry("refused: past-end lanes reach the collider").or_default() += 1;
                continue;
            }
            Err(reason) => {
                mismatched.push(format!("{label}: refused {reason}"));
                continue;
            }
            Ok(()) => {}
        }
        *tally.entry(if post { "compared post-simulation calls" } else { "compared newborn-group calls" }).or_default() += 1;
        let after = &row["after"];
        let native = |key: &str| words(&after["arrays"][key]);
        let mut differs = Vec::new();
        for (k, key) in ["0", "32", "64"].iter().enumerate() {
            let sign = if k == 0 { 0x8000_0000 } else { 0 };
            if system.pool.iter().map(|p| p.position[k].to_bits() ^ sign).collect::<Vec<_>>() != native(key) {
                differs.push("position");
            }
        }
        for (k, key) in ["96", "128", "160"].iter().enumerate() {
            let sign = if k == 0 { 0x8000_0000 } else { 0 };
            if system.pool.iter().map(|p| p.velocity[k].to_bits() ^ sign).collect::<Vec<_>>() != native(key) {
                differs.push("velocity");
            }
        }
        if system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>() != native("960") {
            differs.push("age");
        }
        if collision.random.unwrap().words.to_vec() != words(&after["rand"]) {
            differs.push("random");
        }
        if collision.state.range_words != [hex(&after["s10"]), hex(&after["s18"])] {
            differs.push("rangeWords");
        }
        if collision.hits as usize != row["log"]["performInput"].as_array().unwrap().len() {
            differs.push("hitCount");
        }
        let native_sweeps = row["log"]["sweeps"].as_array().unwrap();
        let ours = log.lock().unwrap().clone();
        let mut used = vec![false; native_sweeps.len()];
        for (request, hit) in &ours {
            let found = native_sweeps.iter().enumerate().position(|(i, s)| !used[i]
                && word(&s["particle"]) == request.particle
                && words(&s["dir"]) == request.direction.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
                && word(&s["distance"]) == request.distance.to_bits() && word(&s["radius"]) == request.sphere_radius.to_bits()
                && words(&s["pose"])[4..] == request.origin.iter().map(|x| x.to_bits()).collect::<Vec<_>>()[..]);
            match found {
                Some(i) => {
                    used[i] = true;
                    let native = words(&native_sweeps[i]["returned"]);
                    // A miss leaves the rest of the hit record unread.
                    let same = match hit { None => native[0] == 0, Some(_) => native == hit_words(hit) };
                    if !same {
                        differs.push("sweepReturn");
                    }
                }
                None => differs.push("sweepNotLogged"),
            }
            *tally.entry("scene sweeps compared").or_default() += 1;
            *tally.entry("scene sweep hits").or_default() += usize::from(hit.is_some());
        }
        for (i, s) in native_sweeps.iter().enumerate() {
            if !used[i] {
                *tally.entry("native sweeps not issued (box touches no triangle)").or_default() += 1;
                if words(&s["returned"])[0] != 0 {
                    differs.push("unissuedNativeHit");
                }
            }
        }
        if ["0", "32", "64", "96", "128", "160", "960"].iter().any(|key| words(&after["arrays"][*key]) != words(&arrays[*key])[..count]) {
            *tally.entry("compared calls that wrote a particle").or_default() += 1;
        }
        if !differs.is_empty() {
            mismatched.push(format!("{label}: {differs:?}"));
        }
    }
    println!("product scene replay: {tally:?}, mismatched {}", mismatched.len());
    for line in mismatched.iter().take(20) {
        println!("  {line}");
    }
    assert!(tally.get("scene sweeps compared").copied().unwrap_or(0) > 0);
    assert!(tally.get("compared calls that wrote a particle").copied().unwrap_or(0) > 0);
    assert!(mismatched.is_empty());
}
