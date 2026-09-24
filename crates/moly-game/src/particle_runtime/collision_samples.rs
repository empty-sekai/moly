//! The collision calls of the native birth path against native module rows:
//! the pool enters in runtime axes (X reflected), goes through the product's
//! post-simulation or newborn-group call and comes back compared bit for bit
//! with the engine's written arrays, random words and range words. A row
//! whose query reads the current-size stream enters it as the engine's
//! current-size array held it (the SizeModule law that writes it has its own
//! replay), and a row with collision sub-emitter edges delivers every edge:
//! the queued commands are compared with the engine's RecordEmit commands.
use super::collision::{newborn_block, post_simulation, CollisionRuntime};
use super::*;
use moly_law::particle::collision_query::{
    Candidate, CollisionLaw, CollisionScene, CollisionState, OverlapQuery, OwnerPair, ParticleFlags, SweepHit,
    SweepRequest,
};
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
