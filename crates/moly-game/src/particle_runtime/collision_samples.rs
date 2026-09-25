//! The collision calls of the native birth path against native module rows:
//! the pool enters in runtime axes (X reflected), goes through the product's
//! post-simulation or newborn-group call and comes back compared bit for bit
//! with the engine's written arrays, random words and range words. A row
//! whose query reads the current-size stream enters it as the engine's
//! current-size array held it (the SizeModule law that writes it has its own
//! replay), and a row with collision sub-emitter edges delivers every edge:
//! the queued commands are compared with the engine's RecordEmit commands.
use super::collision::{newborn_block, post_simulation, CollisionRuntime};
use super::collision_scene::{GroundQuery, GroundScene, SceneBuilder, SiteScene};
use super::*;
use moly_law::particle::collision_query::{
    Candidate, CollisionLaw, CollisionScene, CollisionState, OverlapQuery, OwnerPair, ParticleFlags, SweepHit,
    SweepRequest,
};
use std::collections::BTreeMap;
use moly_law::particle::collision_event::{CollisionEmitEdge, EdgeBurst};
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
    /// Compared calls whose query read the current-size stream.
    current_size: usize,
    /// Compared calls with collision edges, and the commands compared.
    with_edges: usize,
    commands: usize,
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
        let outside = if flags[0] != 0 && flags[1] != 0 {
            Some("current-size stream with the 3D size")
        } else if flags[2] != 0 {
            Some("speed modifier")
        } else if flags[3] != 0 {
            Some("collision events on")
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
            ParticleFlags { current_size: flags[0] != 0, size_3d: flags[1] != 0, speed_modifier: false });
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
            order_free: 0,
            listed_order_fallback: false,
            order_dependent: 0,
            size: None,
            edges: Vec::new(),
            pending: Vec::new(),
        };
        let edges: Vec<super::sub_events::CollisionEdge> = row["sub"].as_array().unwrap().iter().enumerate().map(|(k, edge)| {
            let e = edge.as_array().unwrap();
            assert_eq!(e[0].as_u64(), Some(0), "edge properties");
            let burst = e[2].as_array().unwrap().first().map(|b| {
                let b = b.as_array().unwrap();
                assert_eq!(b[1].as_u64(), Some(0), "constant burst count");
                EdgeBurst::new(bits(&b[0]), bits(&b[2])).expect("burst in range")
            });
            super::sub_events::CollisionEdge {
                target: format!("edge{k}"),
                law: CollisionEmitEdge::new(bits(&e[1]), burst).expect("edge in range"),
            }
        }).collect();
        let has_edges = !edges.is_empty();
        collision.attach_edges(edges);
        for k in 0..collision.edges.len() {
            collision.deliver_to(&format!("edge{k}"));
        }
        let arrays = &row["arrays"];
        let at = |key: &str, i: usize| f32::from_bits(word(&arrays[key][i]));
        let v3 = |base: u32, i: usize| std::array::from_fn(|k| at(&(base + 32 * k as u32).to_string(), i));
        let reflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let mut system = test_support::runtime();
        // The row's collision edges are the system's authored ones.
        system.emitter.sub_emitters = row["sub"].as_array().unwrap().iter().enumerate().map(|(k, edge)|
            moly_law::particle::schema::SubEmitterParams {
                emitter: Some(format!("edge{k}")),
                source_pointer: Default::default(),
                trigger: moly_law::particle::schema::SubEmitterTrigger::Collision,
                properties: 0,
                probability: bits(&edge[1]),
            }).collect();
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
            current_size: at("768", i),
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
        // Each queued command against the engine's: the three birth
        // distribution words of its emission state, then the command bytes
        // (not the emission pointer or the padding word).
        let ours: Vec<Vec<u32>> = collision.take_commands().iter().map(|(_, command)| {
            let raw = command.to_bytes();
            let d = command.emission;
            let mut w = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
            w.extend((0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else {
                u32::from_le_bytes(raw[4 * k..4 * k + 4].try_into().unwrap()) }));
            w
        }).collect();
        let theirs: Vec<Vec<u32>> = row["log"]["emits"].as_array().unwrap().iter().map(|e| {
            let w = words(e);
            let mut out = w[1..4].to_vec();
            out.extend((0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else { w[8 + k] }));
            out
        }).collect();
        if ours != theirs {
            differs.push("commands");
        }
        tally.commands += ours.len();
        tally.with_edges += usize::from(has_edges);
        tally.current_size += usize::from(flags[0] != 0);
        if !differs.is_empty() {
            tally.mismatched.push(format!("{label}: {differs:?}"));
        }
        if ["0", "32", "64", "96", "128", "160", "960"].iter().any(|key| words(&after["arrays"][*key]) != words(&arrays[*key])[..count]) {
            tally.changed += 1;
        }
    }
    println!("product collision replay: post-simulation calls {}, newborn-group calls {} (unreached {}), refused past-end {}, compared calls that wrote a particle {}, reading the current-size stream {}, with collision edges {} ({} commands), outside the product {:?}, mismatched {}",
        tally.post, tally.newborn, tally.unreached, tally.refused_past_end, tally.changed, tally.current_size,
        tally.with_edges, tally.commands, tally.outside, tally.mismatched.len());
    for line in tally.mismatched.iter().take(20) {
        println!("  {line}");
    }
    assert!(tally.post > 0 && tally.newborn > 0 && tally.changed > 0);
    assert!(tally.mismatched.is_empty());
}

/// The scene's answers with the collider each went to, recorded for the
/// comparison with the logged ones.
type SweepLog = std::sync::Arc<std::sync::Mutex<Vec<(SweepRequest, Option<SweepHit>, i32)>>>;

struct Recording {
    inner: Box<dyn CollisionScene + Send + Sync>,
    log: SweepLog,
    /// The last overlap's collider ids.
    last: Vec<i32>,
}

impl Recording {
    fn new(inner: Box<dyn CollisionScene + Send + Sync>, log: SweepLog) -> Self {
        Self { inner, log, last: Vec::new() }
    }
}

impl CollisionScene for Recording {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        let candidates = self.inner.overlap(query);
        self.last = candidates.iter().map(|c| c.collider_id).collect();
        candidates
    }
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.inner.reachable(collides_with)
    }
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        let hit = self.inner.sweep_sphere(request);
        let collider = self.last.get(request.shape).copied().unwrap_or(-1);
        self.log.lock().unwrap().push((*request, hit, collider));
        hit
    }
    fn refusal(&self) -> Option<&'static str> {
        self.inner.refusal()
    }
    fn order_known(&self) -> bool {
        self.inner.order_known()
    }
}

/// A product scene whose listed order is taken as the engine's: the rows
/// were run natively with the colliders in that order.
struct Listed(GroundQuery);

impl CollisionScene for Listed {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.0.overlap(query)
    }
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.0.reachable(collides_with)
    }
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        self.0.sweep_sphere(request)
    }
    fn refusal(&self) -> Option<&'static str> {
        self.0.refusal()
    }
}

fn hit_words(hit: &Option<SweepHit>) -> Vec<u32> {
    match hit {
        None => vec![0],
        Some(h) => [1].into_iter().chain(h.position.iter().chain(&h.normal).map(|x| x.to_bits()))
            .chain([h.distance.to_bits()]).collect(),
    }
}

/// Why a row is outside the product's collision call, if it is.
fn outside_product(row: &Value) -> Option<&'static str> {
    let flags = words(&row["flags"]);
    let (from, to, count) = (row["from"].as_u64().unwrap() as usize, row["to"].as_u64().unwrap() as usize,
        row["count"].as_u64().unwrap() as usize);
    let dt = words(&row["dt"]);
    let post = from == 0 && to == count && dt.iter().all(|&w| w == dt[0]);
    if flags[0] != 0 && flags[1] != 0 {
        Some("outside: current-size stream with the 3D size")
    } else if flags[2] != 0 {
        Some("outside: speed modifier")
    } else if flags[3] != 0 {
        Some("outside: collision events on")
    } else if to == from || (!post && to - from > 4) {
        Some("outside: call shape")
    } else {
        None
    }
}

/// One product collision call over a module row with `scene` in place of
/// the recorded sweeps.
struct ProductCall {
    post: bool,
    /// The call's result: a refusal leaves the pool as it was.
    result: Result<(), String>,
    /// The outcome words that differ from the engine's.
    differs: Vec<&'static str>,
    commands: usize,
}

/// The row through the product's post-simulation or newborn-group call, with
/// the row admission of the recorded-sweep replay (the current-size stream as
/// the engine held it, every collision edge delivered, a Local row's owner
/// words): the written arrays, random words, range words, hit count and
/// queued commands against the engine's.
fn product_call(row: &Value, scene: Box<dyn CollisionScene + Send + Sync>) -> ProductCall {
    let flags = words(&row["flags"]);
    let (from, to, count) = (row["from"].as_u64().unwrap() as usize, row["to"].as_u64().unwrap() as usize,
        row["count"].as_u64().unwrap() as usize);
    let dt = words(&row["dt"]);
    let post = from == 0 && to == count && dt.iter().all(|&w| w == dt[0]);
    let module = &row["module"];
    let curve = |key: &str| bits(&module[key][1]);
    let response = CollisionResponse::new(curve("bounce"), curve("dampen"), curve("loss"),
        bits(&module["minKill"]), bits(&module["maxKill"])).expect("response in range");
    let world = row["space"].as_u64() == Some(1);
    let law = CollisionLaw::from_words(response, bits(&module["radiusScale"]), word(&module["collidesWith"]),
        word(&module["dynamic"]) != 0, module["maxShapes"].as_i64().unwrap() as i32, world,
        ParticleFlags { current_size: flags[0] != 0, size_3d: flags[1] != 0, speed_modifier: false });
    let columns = |key: &str| {
        let w = words(&row[key]);
        QueryAffine::from_columns(&std::array::from_fn(|i| f32::from_bits(w[i])))
    };
    let mut collision = CollisionRuntime {
        law,
        state: CollisionState { range_words: [hex(&row["s10"]), hex(&row["s18"])], uses_events: false, events: 0 },
        random: Some(CollisionRandom { words: std::array::from_fn(|i| words(&row["rand"])[i]) }),
        owner: (!world).then(|| OwnerPair { local_to_world: columns("owner"), world_to_local: columns("inverse") }),
        scene,
        calls: 0,
        hits: 0,
        draws: 0,
        unreached: 0,
        order_free: 0,
        listed_order_fallback: false,
        order_dependent: 0,
        size: None,
        edges: Vec::new(),
        pending: Vec::new(),
    };
    // The row's collision edges, every one delivered: the queued commands
    // are compared with the engine's RecordEmit commands.
    let edges: Vec<super::sub_events::CollisionEdge> = row["sub"].as_array().unwrap().iter().enumerate().map(|(k, edge)| {
        let e = edge.as_array().unwrap();
        assert_eq!(e[0].as_u64(), Some(0), "edge properties");
        let burst = e[2].as_array().unwrap().first().map(|b| {
            let b = b.as_array().unwrap();
            assert_eq!(b[1].as_u64(), Some(0), "constant burst count");
            EdgeBurst::new(bits(&b[0]), bits(&b[2])).expect("burst in range")
        });
        super::sub_events::CollisionEdge {
            target: format!("edge{k}"),
            law: CollisionEmitEdge::new(bits(&e[1]), burst).expect("edge in range"),
        }
    }).collect();
    collision.attach_edges(edges);
    for k in 0..collision.edges.len() {
        collision.deliver_to(&format!("edge{k}"));
    }
    let arrays = &row["arrays"];
    let at = |key: &str, i: usize| f32::from_bits(word(&arrays[key][i]));
    let v3 = |base: u32, i: usize| std::array::from_fn(|k| at(&(base + 32 * k as u32).to_string(), i));
    let reflect = |v: [f32; 3]| [-v[0], v[1], v[2]];
    let mut system = test_support::runtime();
    system.emitter.sub_emitters = row["sub"].as_array().unwrap().iter().enumerate().map(|(k, edge)|
        moly_law::particle::schema::SubEmitterParams {
            emitter: Some(format!("edge{k}")),
            source_pointer: Default::default(),
            trigger: moly_law::particle::schema::SubEmitterTrigger::Collision,
            properties: 0,
            probability: bits(&edge[1]),
        }).collect();
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
        // The current-size stream as the engine's array held it.
        current_size: at("768", i),
        ..template
    }).collect();
    let (pending, emission_word) = (bits(&row["s0"]), word(&row["s1ec"]));
    let result = if post {
        post_simulation(&mut system, &mut collision, f32::from_bits(dt[0]), pending, emission_word)
    } else {
        newborn_block(&mut system, &mut collision, from, to, std::array::from_fn(|l| f32::from_bits(dt[l])),
            pending, emission_word)
    };
    let mut call = ProductCall { post, result, differs: Vec::new(), commands: 0 };
    if call.result.is_err() {
        return call;
    }
    let after = &row["after"];
    let native = |key: &str| words(&after["arrays"][key]);
    let differs = &mut call.differs;
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
    // Each queued command against the engine's (its emission distribution
    // words, then the command bytes but the pointer and padding words).
    let commands: Vec<Vec<u32>> = collision.take_commands().iter().map(|(_, command)| {
        let raw = command.to_bytes();
        let d = command.emission;
        let mut w = vec![d.spacing.to_bits(), d.offset.to_bits(), d.burst_fraction.to_bits()];
        w.extend((0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else {
            u32::from_le_bytes(raw[4 * k..4 * k + 4].try_into().unwrap()) }));
        w
    }).collect();
    let engine_commands: Vec<Vec<u32>> = row["log"]["emits"].as_array().unwrap().iter().map(|e| {
        let w = words(e);
        let mut out = w[1..4].to_vec();
        out.extend((0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else { w[8 + k] }));
        out
    }).collect();
    if commands != engine_commands {
        differs.push("commands");
    }
    call.commands = commands.len();
    call
}

/// Every sweep the scene answered against the logged native sweep of the
/// same particle and request (and, with `native_colliders`, which maps the
/// row's shape index to the product's collider id, the same collider); a
/// native sweep the product does not issue must be a miss.
fn compare_sweeps(row: &Value, ours: &[(SweepRequest, Option<SweepHit>, i32)], native_colliders: Option<&[i32]>,
    tally: &mut BTreeMap<&'static str, usize>, differs: &mut Vec<&'static str>, details: &mut Vec<String>) {
    let native_sweeps = row["log"]["sweeps"].as_array().unwrap();
    let mut used = vec![false; native_sweeps.len()];
    for (request, hit, collider) in ours {
        let found = native_sweeps.iter().enumerate().position(|(i, s)| !used[i]
            && word(&s["particle"]) == request.particle
            && native_colliders.is_none_or(|ids| ids.get(s["shape"].as_u64().unwrap() as usize) == Some(collider))
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
            None => {
                differs.push("sweepNotLogged");
                if details.len() < 12 {
                    let same_particle: Vec<String> = native_sweeps.iter().filter(|s| word(&s["particle"]) == request.particle)
                        .map(|s| format!("shape {} dir {:?} distance {:#x} radius {:#x} origin {:?}", s["shape"], words(&s["dir"]),
                            word(&s["distance"]), word(&s["radius"]), &words(&s["pose"])[4..])).collect();
                    details.push(format!("ours: particle {} collider {collider} dir {:?} distance {:#x} radius {:#x} origin {:?}; native for the particle: {same_particle:?}",
                        request.particle, request.direction.map(f32::to_bits), request.distance.to_bits(),
                        request.sphere_radius.to_bits(), request.origin.map(f32::to_bits)));
                }
            }
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
}

/// The product's own ground scene (built from the collider export by the
/// admission path) in place of the recorded sweep results: every native
/// module row of the natively swept receipt (`MOLY_COLLISION_SCENE_LABEL`
/// names it) whose one collider is an exported ground collider, through the
/// product's post-simulation or newborn-group call, with the row admission
/// of the recorded-sweep replay (the current-size stream as the engine held
/// it, every collision edge delivered, a Local row's owner words). The
/// written arrays, random words, range words, hit records and queued
/// commands must equal the engine's, and every sweep the scene answers must
/// equal the logged native sweep of the same particle. The receipt's scene lookup
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
        .filter(|scene| scene.collider_count() == 1)
        .map(|scene| {
            let c = GroundQuery::new(scene.clone()).reachable(u32::MAX);
            let bits = c[0].bounds_min.iter().chain(&c[0].bounds_max).map(|x| x.to_bits()).collect();
            (scene, bits)
        }).collect();
    assert!(!scenes.is_empty());
    let mut tally = BTreeMap::<&str, usize>::new();
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
            *tally.entry("outside: not one exported ground collider the scene binds").or_default() += 1;
            continue;
        };
        if let Some(reason) = outside_product(row) {
            *tally.entry(reason).or_default() += 1;
            continue;
        }
        let log: SweepLog = Default::default();
        let call = product_call(row, Box::new(Recording::new(Box::new(GroundQuery::new(scene.clone())), log.clone())));
        let label = format!("{} {}", row["config"], row["scenario"]);
        match &call.result {
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
        let flags = words(&row["flags"]);
        let world = row["space"].as_u64() == Some(1);
        *tally.entry(if call.post { "compared post-simulation calls" } else { "compared newborn-group calls" }).or_default() += 1;
        *tally.entry("scene commands compared").or_default() += call.commands;
        *tally.entry("compared calls reading the current-size stream").or_default() += usize::from(flags[0] != 0);
        *tally.entry("compared calls with collision edges").or_default() += usize::from(!row["sub"].as_array().unwrap().is_empty());
        *tally.entry("compared Local calls").or_default() += usize::from(!world);
        let mut differs = call.differs;
        let mut details = Vec::new();
        compare_sweeps(row, &log.lock().unwrap(), None, &mut tally, &mut differs, &mut details);
        let count = row["count"].as_u64().unwrap() as usize;
        let (after, arrays) = (&row["after"], &row["arrays"]);
        if ["0", "32", "64", "96", "128", "160", "960"].iter().any(|key| words(&after["arrays"][*key]) != words(&arrays[*key])[..count]) {
            *tally.entry("compared calls that wrote a particle").or_default() += 1;
        }
        if !differs.is_empty() {
            mismatched.push(format!("{label}: {differs:?} {details:?}"));
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

/// The product's site scene with two installed effects against the native
/// two-collider rows (`MOLY_COLLISION_MULTI_ROWS`, with the collider export
/// `MOLY_COLLISION_EXPORT`): the scene is built by the admission path from
/// each row's two site effects, installed in the row's collider order.
/// First every row with that order taken as the engine's: the outcome words
/// and every sweep (same particle, request and collider) equal to the
/// engine's. Then every case as the product runs it, the order unknown: where
/// the engine's two orders agree the product's outcome must equal them, and
/// where they differ the call must be refused as order dependent.
#[test]
#[ignore = "needs MOLY_COLLISION_MULTI_ROWS and MOLY_COLLISION_EXPORT"]
fn product_site_scene_matches_native_multi_rows() {
    let read = |key: &str| -> Value {
        let path = std::env::var(key).unwrap_or_else(|_| panic!("{key}"));
        serde_json::from_slice(&std::fs::read(&path).expect("read rows")).expect("parse rows")
    };
    let doc = read("MOLY_COLLISION_MULTI_ROWS");
    let export = read("MOLY_COLLISION_EXPORT");
    // The product's collider id of each site effect's ground collider: its
    // ordinal in the export.
    let ordinal: std::collections::HashMap<String, i32> = export["colliders"].as_array().expect("colliders").iter().enumerate()
        .filter(|(_, c)| c["variant"].as_str() != Some("common"))
        .map(|(i, c)| (c["effect"].as_str().unwrap().to_owned(), i as i32)).collect();
    let mut builder = SceneBuilder::new(Ok(export));
    let mut site_for = |row: &Value| -> SiteScene {
        let site = SiteScene::default();
        for collider in row["colliders"].as_array().expect("row colliders") {
            let effect = collider["effect"].as_str().expect("collider effect");
            let scene = builder.effect_colliders(effect).expect("effect colliders");
            assert_eq!(scene.collider_count(), 1, "{effect}: one ground collider");
            site.install(scene);
        }
        site
    };
    let mut tally = BTreeMap::<&str, usize>::new();
    let mut mismatched: Vec<String> = Vec::new();
    let mut cases: BTreeMap<(u64, String, String), [Option<&Value>; 2]> = BTreeMap::new();
    for row in doc["rows"].as_array().expect("rows") {
        let label = format!("pair {} {} {} {}", row["pair"], row["config"], row["scenario"], row["order"]);
        let slot = cases.entry((row["pair"].as_u64().unwrap(), row["config"].as_str().unwrap().to_owned(),
            row["scenario"].as_str().unwrap().to_owned())).or_default();
        slot[usize::from(row["order"].as_str() == Some("BA"))] = Some(row);
        if let Some(reason) = outside_product(row) {
            *tally.entry(reason).or_default() += 1;
            continue;
        }
        let log: SweepLog = Default::default();
        let site = site_for(row);
        let natives: Vec<i32> = row["colliders"].as_array().unwrap().iter().map(|c| ordinal[c["effect"].as_str().unwrap()]).collect();
        // The product's world bounds of each collider against the engine's.
        let ours: Vec<Vec<u32>> = GroundQuery::live(site.clone()).reachable(u32::MAX).iter()
            .map(|c| c.bounds_min.iter().chain(&c.bounds_max).map(|x| x.to_bits()).collect()).collect();
        let engine: Vec<Vec<u32>> = row["shapes"].as_array().unwrap().iter().map(|s| words(s)[..6].to_vec()).collect();
        if ours != engine {
            *tally.entry("rows whose collider bounds differ from the engine's").or_default() += 1;
            if !mismatched.iter().any(|m| m.starts_with("bounds")) {
                mismatched.push(format!("bounds {label}: ours {ours:x?} engine {engine:x?}"));
            }
        }
        let call = product_call(row, Box::new(Recording::new(Box::new(Listed(GroundQuery::live(site))), log.clone())));
        match &call.result {
            Err(reason) if reason.contains("PastEndLanes") => {
                *tally.entry("refused: past-end lanes reach a collider").or_default() += 1;
                continue;
            }
            Err(reason) => {
                mismatched.push(format!("{label}: refused {reason}"));
                continue;
            }
            Ok(()) => {}
        }
        *tally.entry(if call.post { "compared post-simulation calls" } else { "compared newborn-group calls" }).or_default() += 1;
        *tally.entry("scene commands compared").or_default() += call.commands;
        let mut differs = call.differs;
        let mut details = Vec::new();
        compare_sweeps(row, &log.lock().unwrap(), Some(&natives), &mut tally, &mut differs, &mut details);
        let count = row["count"].as_u64().unwrap() as usize;
        let (after, arrays) = (&row["after"], &row["arrays"]);
        if ["0", "32", "64", "96", "128", "160", "960"].iter().any(|key| words(&after["arrays"][*key]) != words(&arrays[*key])[..count]) {
            *tally.entry("compared calls that wrote a particle").or_default() += 1;
        }
        if !differs.is_empty() {
            mismatched.push(format!("{label}: {differs:?} {details:?}"));
        }
    }
    for ((pair, config, scenario), [ab, ba]) in &cases {
        let label = format!("pair {pair} {config} {scenario} unordered");
        let (Some(ab), Some(ba)) = (ab, ba) else {
            mismatched.push(format!("{label}: one order missing"));
            continue;
        };
        if outside_product(ab).is_some() {
            continue;
        }
        let agree = ab["after"] == ba["after"] && ab["log"]["emits"] == ba["log"]["emits"];
        let call = product_call(ab, Box::new(GroundQuery::live(site_for(ab))));
        match (&call.result, agree) {
            (Err(reason), _) if reason.contains("PastEndLanes") => {
                *tally.entry("unordered: refused, past-end lanes reach a collider").or_default() += 1;
            }
            (Ok(()), true) => {
                *tally.entry("unordered: cases the engine's orders agree on, compared").or_default() += 1;
                if !call.differs.is_empty() {
                    mismatched.push(format!("{label}: {:?}", call.differs));
                }
            }
            (Err(reason), false) if reason.contains("OrderDependent") => {
                *tally.entry("unordered: cases the engine's orders differ on, refused as order dependent").or_default() += 1;
            }
            (Ok(()), false) => mismatched.push(format!("{label}: the engine's orders differ and the product did not refuse")),
            (Err(reason), _) => mismatched.push(format!("{label}: refused {reason}")),
        }
    }
    println!("product site scene multi-collider replay: {tally:?}, mismatched {}", mismatched.len());
    for line in mismatched.iter().take(20) {
        println!("  {}", &line[..line.len().min(6000)]);
    }
    assert!(tally.get("scene sweeps compared").copied().unwrap_or(0) > 0);
    assert!(tally.get("unordered: cases the engine's orders agree on, compared").copied().unwrap_or(0) > 0);
    assert!(mismatched.is_empty());
}
