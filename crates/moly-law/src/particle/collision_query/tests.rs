//! Replays of the module update against native rows.

use super::*;
use crate::particle::collision_event::EdgeBurst;
use crate::particle::json::{parse, Value};
use std::collections::BTreeMap;

fn field<'v>(v: &'v Value, key: &str) -> &'v Value {
    v.get(key).unwrap_or_else(|| panic!("row field {key}"))
}
fn items(v: &Value) -> &[Value] {
    v.as_array().expect("row array")
}
fn int(v: &Value) -> i64 {
    let x = v.as_f64().expect("row integer");
    assert!(x.fract() == 0.0 && x.abs() <= 9.0e15, "row integer {x}");
    x as i64
}
fn word(v: &Value) -> u32 {
    int(v) as u32
}
fn bits(v: &Value) -> f32 {
    f32::from_bits(word(v))
}
fn words(v: &Value) -> Vec<u32> {
    items(v).iter().map(word).collect()
}
fn hex(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().expect("hex word"), 16).expect("hex word")
}
fn vec3(v: &[f32; 3]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// The arrays of one row: the pool, then the slots past its end as the
/// native memory held them; beyond those, zero words.
struct Arrays {
    words: BTreeMap<u32, Vec<u32>>,
    current_size: bool,
}

impl Arrays {
    fn at(&self, key: u32, index: usize) -> u32 {
        self.words[&key].get(index).copied().unwrap_or(0)
    }
    fn f(&self, key: u32, index: usize) -> f32 {
        f32::from_bits(self.at(key, index))
    }
    fn v3(&self, key: u32, index: usize) -> [f32; 3] {
        std::array::from_fn(|k| self.f(key + 32 * k as u32, index))
    }
}

impl CollisionParticles for Arrays {
    fn lane(&self, i: usize) -> Option<ParticleLane> {
        Some(ParticleLane {
            position: self.v3(0, i),
            velocity: self.v3(96, i),
            animated: self.v3(192, i),
            size: self.v3(if self.current_size { 768 } else { 672 }, i),
            speed_modifier: self.f(1216, i),
            age_percent: self.f(960, i),
            inverse_lifetime: self.f(992, i),
            seed: self.at(896, i),
        })
    }
}

/// The recorded native sweep results, returned in call order.
struct Replay {
    shapes: Vec<Candidate>,
    returns: Vec<Option<SweepHit>>,
    next: usize,
    overflow: bool,
}

impl CollisionScene for Replay {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        self.shapes.iter().take(query.max_shapes.max(0) as usize).copied().collect()
    }
    fn reachable(&self, _: u32) -> Vec<Candidate> {
        self.shapes.clone()
    }
    fn sweep_sphere(&mut self, _: &SweepRequest) -> Option<SweepHit> {
        match self.returns.get(self.next) {
            Some(hit) => {
                self.next += 1;
                *hit
            }
            None => {
                self.overflow = true;
                None
            }
        }
    }
}

pub(crate) struct Row {
    pub law: CollisionLaw,
    pub state: CollisionState,
    pub random: CollisionRandom,
    pub edges: Vec<CollisionEmitEdge>,
    pub from: usize,
    pub to: usize,
    pub count: usize,
    pub dt: [f32; 4],
    pub owner: Option<OwnerPair>,
    pub emission_word: u32,
    pub pending: f32,
    pub shapes: Vec<Candidate>,
    pub returns: Vec<Option<SweepHit>>,
}

/// A row's inputs as the law takes them; `None` when the row is outside the
/// law's qualified module (no native row is).
pub(crate) fn row_inputs(row: &Value) -> Option<Row> {
    let module = field(row, "module");
    assert_eq!((int(field(module, "type")), int(field(module, "mode")), int(field(module, "quality"))), (1, 0, 0));
    assert_eq!(int(field(module, "messages")), 0);
    assert_eq!(int(field(module, "force")), 0);
    assert_eq!(int(field(module, "interior")), 0);
    let curve = |key: &str| {
        let c = items(field(module, key));
        assert_eq!(int(&c[0]), 0, "constant response curve");
        bits(&c[1])
    };
    let response = CollisionResponse::new(curve("bounce"), curve("dampen"), curve("loss"),
        bits(field(module, "minKill")), bits(field(module, "maxKill"))).ok()?;
    let flags = words(field(row, "flags"));
    let world = int(field(row, "space")) == 1;
    let law = CollisionLaw::from_words(response, bits(field(module, "radiusScale")), word(field(module, "collidesWith")),
        int(field(module, "dynamic")) != 0, int(field(module, "maxShapes")) as i32, world,
        ParticleFlags { current_size: flags[0] != 0, size_3d: flags[1] != 0, speed_modifier: flags[2] != 0 });
    let columns = |key: &str| {
        let w = words(field(row, key));
        QueryAffine::from_columns(&std::array::from_fn(|i| f32::from_bits(w[i])))
    };
    let edges = items(field(row, "sub")).iter().map(|edge| {
        let e = items(edge);
        assert_eq!(int(&e[0]), 0, "edge properties");
        let burst = items(&e[2]).first().map(|b| {
            let b = items(b);
            assert_eq!(int(&b[1]), 0, "constant burst count");
            EdgeBurst::new(bits(&b[0]), bits(&b[2])).expect("burst in range")
        });
        CollisionEmitEdge::new(bits(&e[1]), burst).expect("edge in range")
    }).collect();
    let shapes = items(field(row, "shapes")).iter().map(|s| {
        let s = words(s);
        Candidate {
            bounds_min: std::array::from_fn(|k| f32::from_bits(s[k])),
            bounds_max: std::array::from_fn(|k| f32::from_bits(s[3 + k])),
            is_trigger: s[6] != 0,
            collider_id: s[7] as i32,
            body_id: None,
        }
    }).collect();
    let returns = items(field(field(row, "log"), "sweeps")).iter().map(|s| {
        let r = words(field(s, "returned"));
        (r[0] != 0).then(|| SweepHit {
            position: std::array::from_fn(|k| f32::from_bits(r[1 + k])),
            normal: std::array::from_fn(|k| f32::from_bits(r[4 + k])),
            distance: f32::from_bits(r[7]),
        })
    }).collect();
    let dt = words(field(row, "dt"));
    Some(Row {
        law,
        state: CollisionState {
            range_words: [hex(field(row, "s10")), hex(field(row, "s18"))],
            uses_events: flags[3] != 0,
            events: int(field(row, "events")) as u64,
        },
        random: CollisionRandom { words: std::array::from_fn(|i| words(field(row, "rand"))[i]) },
        edges,
        from: int(field(row, "from")) as usize,
        to: int(field(row, "to")) as usize,
        count: int(field(row, "count")) as usize,
        dt: std::array::from_fn(|l| f32::from_bits(dt[l])),
        owner: (!world).then(|| OwnerPair { local_to_world: columns("owner"), world_to_local: columns("inverse") }),
        emission_word: word(field(row, "s1ec")),
        pending: bits(field(row, "s0")),
        shapes,
        returns,
    })
}

pub(crate) fn row_arrays(row: &Value) -> BTreeMap<u32, Vec<u32>> {
    field(row, "arrays").as_object().expect("arrays").iter()
        .map(|(k, v)| (k.parse().expect("array offset"), words(v))).collect()
}

/// Every mismatching family of one row, empty when the row matches.
fn replay(row: &Value) -> Vec<&'static str> {
    let mut bad = Vec::new();
    let Some(mut input) = row_inputs(row) else {
        return vec!["refused response"];
    };
    let arrays = Arrays { words: row_arrays(row), current_size: words(field(row, "flags"))[0] != 0 };
    let mut scene = Replay { shapes: input.shapes.clone(), returns: input.returns.clone(), next: 0, overflow: false };
    let mut trace = Trace::default();
    let update = UpdateInput {
        from: input.from, to: input.to, dt: input.dt, owner: input.owner, edges: &input.edges,
        emission_word: input.emission_word, pending: input.pending,
    };
    let outcome = match input.law.update(&mut input.state, Some(&mut input.random), &arrays, &update, &mut scene, Some(&mut trace)) {
        Ok(outcome) => outcome,
        Err(_) => return vec!["refused"],
    };
    if scene.overflow || scene.next != scene.returns.len() {
        bad.push("sweepCount");
    }
    let after = field(row, "after");
    let mut written: BTreeMap<u32, Vec<u32>> = arrays.words.iter()
        .filter(|(k, _)| [0, 32, 64, 96, 128, 160, 960].contains(*k))
        .map(|(k, v)| (*k, v[..input.count].to_vec())).collect();
    for w in &outcome.written {
        for k in 0..3 {
            written.get_mut(&(32 * k as u32)).unwrap()[w.index] = w.position[k].to_bits();
            written.get_mut(&(96 + 32 * k as u32)).unwrap()[w.index] = w.velocity[k].to_bits();
        }
        written.get_mut(&960).unwrap()[w.index] = w.age_percent.to_bits();
    }
    for (key, native) in field(after, "arrays").as_object().unwrap() {
        if written[&key.parse::<u32>().unwrap()] != words(native) {
            bad.push("arrays");
            break;
        }
    }
    if field(row, "otherArraysUnchanged").as_bool() != Some(true) {
        bad.push("otherArrays");
    }
    if input.random.words.to_vec() != words(field(after, "rand")) {
        bad.push("random");
    }
    if input.state.range_words != [hex(field(after, "s10")), hex(field(after, "s18"))] {
        bad.push("rangeWords");
    }
    if u32::from(input.state.uses_events) != word(field(after, "flag")) || input.state.events as i64 != int(field(after, "events")) {
        bad.push("eventFlag");
    }
    let log = field(row, "log");
    let find = items(field(log, "find"));
    match (find.first(), trace.packs.is_empty()) {
        (None, true) => {}
        (Some(f), false) => {
            let f = items(f);
            let packs: Vec<Vec<u32>> = items(&f[4]).iter().map(words).collect();
            if packs != trace.packs.iter().map(|p| p.to_vec()).collect::<Vec<_>>() {
                bad.push("packs");
            }
            if word(&f[0]) != input.law.collides_with || int(&f[1]) as i32 != input.law.max_shapes
                || word(&f[2]) != 0 || word(&f[3]) & 0xff != u32::from(input.law.dynamic) {
                bad.push("findParams");
            }
        }
        _ => bad.push("packs"),
    }
    let shapes = items(field(log, "getShapes"));
    match (shapes.first(), trace.overlap) {
        (None, None) => {}
        (Some(g), Some(q)) => {
            let g = words(g);
            let expect: Vec<u32> = q.center.iter().chain(&q.extents).map(|x| x.to_bits()).collect();
            if g[..6] != expect[..] || g[6] as i32 != q.max_shapes || g[7] != q.collides_with
                || g[8] != u32::from(q.dynamic) || g[9] != 0 {
                bad.push("overlap");
            }
        }
        _ => bad.push("overlap"),
    }
    let bounds = items(field(log, "worldBounds"));
    if bounds.len() != trace.bounds_calls
        || bounds.iter().enumerate().any(|(i, b)| words(b) != [i as u32, 1.0f32.to_bits()]) {
        bad.push("worldBounds");
    }
    let sweeps = items(field(log, "sweeps"));
    if sweeps.len() != trace.sweeps.len() || sweeps.iter().zip(&trace.sweeps).any(|(s, r)| {
        let pose = words(field(s, "pose"));
        int(field(s, "shape")) as usize != r.shape || word(field(s, "particle")) != r.particle
            || words(field(s, "dir")) != vec3(&r.direction) || word(field(s, "distance")) != r.distance.to_bits()
            || word(field(s, "radius")) != r.sphere_radius.to_bits() || int(field(s, "geomType")) != 0
            || pose[..4] != [0, 0, 0, 1.0f32.to_bits()] || pose[4..] != vec3(&r.origin)[..]
            || int(field(s, "flags")) != 0x202 || int(field(s, "inflation")) != 0
    }) {
        bad.push("sweeps");
    }
    let intersects: Vec<Vec<i64>> = trace.intersects.iter().map(|x| {
        let mut w: Vec<i64> = vec![x.particle as i64];
        w.extend(x.start.iter().chain(&x.direction).chain(&x.aabb).map(|v| v.to_bits() as i64));
        w.extend([x.length.to_bits() as i64, x.radius.to_bits() as i64, x.candidates as i64,
            x.returned.map_or(-1, |r| r as i64)]);
        w
    }).collect();
    if intersects != items(field(log, "intersect")).iter().map(|x| items(x).iter().map(int).collect::<Vec<_>>()).collect::<Vec<_>>() {
        bad.push("intersect");
    }
    if trace.pack_tests.iter().map(|&t| i64::from(t)).collect::<Vec<_>>() != items(field(log, "packTests")).iter().map(int).collect::<Vec<_>>() {
        bad.push("packTests");
    }
    let hits: Vec<Vec<u32>> = outcome.hits.iter().map(|h| {
        let mut w = vec![h.index];
        w.extend(h.start.iter().chain(&h.direction).chain(&h.normal).chain(&h.point).map(|v| v.to_bits()));
        w.extend([h.collider_id as u32, h.body_id as u32]);
        w
    }).collect();
    if hits != items(field(log, "performInput")).iter().map(words).collect::<Vec<_>>() {
        bad.push("hits");
    }
    let records: Vec<Vec<i64>> = outcome.emits.iter().map(|e| {
        let mut w = vec![e.particle as i64, e.edge as i64, 1];
        w.extend(e.state_words.iter().map(|&x| x as i64));
        w.extend(e.times.iter().map(|x| x.to_bits() as i64));
        w.push(e.burst_count.map_or(-1, i64::from));
        w
    }).collect();
    if records != items(field(log, "recordEmit")).iter().map(|x| items(x).iter().map(int).collect::<Vec<_>>()).collect::<Vec<_>>() {
        bad.push("recordEmit");
    }
    let commands: Vec<Vec<u32>> = outcome.emits.iter().filter_map(|e| e.commands).flatten().enumerate().map(|(i, (command, state))| {
        let raw = command.to_bytes();
        let mut w = vec![(i % 2) as u32];
        w.extend(state);
        w.extend((0..30).map(|k| if matches!(k, 0 | 1 | 21) { 0 } else { u32::from_le_bytes(raw[4 * k..4 * k + 4].try_into().unwrap()) }));
        w
    }).collect();
    let native: Vec<Vec<u32>> = items(field(log, "emits")).iter().map(|e| {
        let mut w = words(e);
        for k in [0, 1, 21] {
            w[8 + k] = 0;
        }
        w
    }).collect();
    if commands != native {
        bad.push("commands");
    }
    bad
}

const ARMS: [&str; 10] = ["radiusNoHalf", "noSkin", "oldPadding", "forwardOrder", "noRandom", "killAtHundred",
    "hostNormalize", "leftAssociation", "catchUpZero", "stateUnbounded"];

/// Every native CollisionModule::Update row (the corpus collision
/// configurations and controls around them, unaligned ranges, slots past the
/// pool end with stale contents, range-word edges, and rows swept natively
/// against the cooked ground meshes) through the law:
/// the written arrays, the random words, the state words, the query packs,
/// the overlap box, the sweep requests, the selections, the hits, the event
/// records and the command words, bit for bit. Each one-rule variant must
/// differ from native on at least one row.
#[test]
#[ignore = "needs MOLY_COLLISION_ROWS"]
fn module_update_rows_match_native_bits() {
    let path = std::env::var("MOLY_COLLISION_ROWS").expect("MOLY_COLLISION_ROWS");
    let doc = parse(&std::fs::read(&path).expect("read rows")).expect("parse rows");
    let rows = items(field(&doc, "rows"));
    assert!(!rows.is_empty());
    let mut per_receipt: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut red: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut hits, mut emits, mut sweeps) = (0usize, 0usize, 0usize);
    for row in rows {
        let receipt = field(row, "receipt").as_str().unwrap().to_owned();
        arms::set(None);
        let bad = replay(row);
        let entry = per_receipt.entry(receipt.clone()).or_default();
        entry.0 += 1;
        if !bad.is_empty() {
            entry.1 += 1;
            failures.push(format!("{receipt} {} {}: {bad:?}", field(row, "config").as_str().unwrap(),
                field(row, "scenario").as_str().unwrap()));
        }
        let log = field(row, "log");
        hits += items(field(log, "performInput")).len();
        emits += items(field(log, "emits")).len();
        sweeps += items(field(log, "sweeps")).len();
        for arm in ARMS {
            arms::set(Some(arm));
            let differs = !replay(row).is_empty();
            *red.entry(arm).or_default() += usize::from(differs);
        }
        arms::set(None);
    }
    println!("collision module replay: {} rows, {sweeps} sweeps, {hits} hits, {emits} commands; per receipt (rows, mismatched) {per_receipt:?}; arms red {red:?}", rows.len());
    for failure in failures.iter().take(20) {
        println!("  {failure}");
    }
    assert!(failures.is_empty(), "{} rows differ from native", failures.len());
    for arm in ARMS {
        assert!(red.get(arm).copied().unwrap_or(0) > 0, "arm {arm} never differs from native");
    }
}
