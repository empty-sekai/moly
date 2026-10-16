//! Replay of the sky Animator receipts against the production player: the
//! accepted layers come from `Contract::compile` over the phenomenon's exported
//! effects and animations, the per-frame chain runs through `EffectAnimator`, and
//! every expected value is a bit pattern recorded from the game's own engine
//! library (its Animator kernels executed on sampled inputs). Research
//! instrument; nothing here compares against our own output.

use crate::source_curve::Curve;
use crate::weather_animation::{AnimatedNode, Contract, EffectAnimator};
use moly_law::animator as law;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

fn read(key: &str) -> Value {
    let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} not set"));
    serde_json::from_slice(&std::fs::read(path).expect("read receipt")).expect("parse receipt")
}

fn bits(v: &Value) -> u32 {
    u32::try_from(v.as_u64().expect("u32 bits")).expect("u32 bits")
}

fn f(v: &Value) -> f32 {
    f32::from_bits(bits(v))
}

fn f_or_one(v: &Value) -> f32 {
    if v.is_null() { 1.0 } else { f(v) }
}

fn quad(v: &Value) -> [u32; 4] {
    let a = v.as_array().expect("array");
    [bits(&a[0]), bits(&a[1]), bits(&a[2]), bits(&a[3])]
}

fn qbits(q: [f32; 4]) -> [u32; 4] {
    q.map(f32::to_bits)
}

#[derive(Default, Debug)]
struct Score {
    cases: usize,
    mismatches: usize,
    first: Option<String>,
}

impl Score {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.cases += 1;
        if !ok {
            self.mismatches += 1;
            if self.first.is_none() {
                self.first = Some(what());
            }
        }
    }
}

/// The production players of every effect in the export, keyed by the receipt's
/// clip names through the receipt's own clip-to-node map; each player's clip
/// inputs must be the ones the native harness was run with.
fn players(receipt: &Value, scores: &mut BTreeMap<String, Score>) -> HashMap<String, AnimatedNode> {
    let effects = read("MOLY_SKY_ANIMATOR_EFFECTS");
    let animations = read("MOLY_SKY_ANIMATOR_ANIMATIONS");
    let mut by_path: HashMap<String, AnimatedNode> = HashMap::new();
    let mut blocked = Vec::new();
    for effect in effects["effects"].as_object().expect("effects").values() {
        let contract = Contract::compile(effect, Some(&animations));
        blocked.extend(contract.report["blockedScopes"].as_array().into_iter().flatten().cloned());
        for player in contract.players() {
            assert!(by_path.insert(player.path.clone(), player.clone()).is_none(), "{} played twice", player.path);
        }
    }
    let mut out = HashMap::new();
    for (name, clip) in receipt["clips"].as_object().expect("receipt clips") {
        let node = clip["node"].as_str().expect("receipt clip node");
        let player = by_path.get(node).unwrap_or_else(|| panic!("{node}: no accepted player; blocked {blocked:?}"));
        let layer = &player.layer;
        let delta = clip["delta"].as_array().expect("receipt delta");
        let ok = clip["order"] == 4
            && layer.start.to_bits() == (clip["start"].as_f64().unwrap() as f32).to_bits()
            && layer.stop.to_bits() == (clip["stop"].as_f64().unwrap() as f32).to_bits()
            && layer.cycle_offset.to_bits() == (clip["cycleOffset"].as_f64().unwrap() as f32).to_bits()
            && (0..3).all(|i| layer.reference_euler[i].to_bits() == (delta[i][0].as_f64().unwrap() as f32).to_bits())
            && (0..4).all(|i| layer.default_rotation[i].to_bits()
                == (clip["default"][i].as_f64().unwrap() as f32).to_bits());
        scores.entry("playerInputs".into()).or_default().check(ok, || format!("{name}: {layer:?}"));
        out.insert(name.clone(), player.clone());
    }
    out
}

/// The playable times of the prepare family are genuine doubles, and the JSON
/// parser in use may land one unit away from the nearest double of a decimal.
/// Read their decimal text from the receipt and parse it with the correctly
/// rounded standard parser. The cases are flat objects in one array.
fn exact_prepare_times(key: &str) -> Vec<(f64, f64)> {
    let text = std::fs::read_to_string(std::env::var_os(key).expect(key)).expect("read receipt");
    let Some(start) = text.find("\"prepare\": [{") else { return Vec::new() };
    let mut rest = &text[start + "\"prepare\": [".len()..];
    let mut out = Vec::new();
    while rest.starts_with('{') {
        let end = rest.find('}').expect("flat prepare case");
        let object = &rest[..end];
        let field = |name: &str| -> f64 {
            let label = format!("\"{name}\": ");
            let tail = &object[object.find(&label).expect("prepare time field") + label.len()..];
            let stop = tail.find(',').unwrap_or(tail.len());
            tail[..stop].trim().parse().expect("decimal time")
        };
        out.push((field("time"), field("prev")));
        rest = rest[end + 1..].trim_start();
        match rest.strip_prefix(',') {
            Some(next) => rest = next.trim_start(),
            None => break,
        }
    }
    out
}

fn score<'a>(scores: &'a mut BTreeMap<String, Score>, tag: &str, family: &str) -> &'a mut Score {
    scores.entry(format!("{tag}/{family}")).or_default()
}

fn replay_families(tag: &str, key: &str, clips: &HashMap<String, AnimatedNode>, scores: &mut BTreeMap<String, Score>) {
    let doc = read(key);
    let cases = &doc["cases"];
    let clip = |c: &Value| &clips[c["clip"].as_str().expect("clip name")];
    for c in cases["sample"].as_array().into_iter().flatten() {
        let node = clip(c);
        let t = law::loop_clip_time(f(&c["normalized"]), node.layer.start, node.layer.stop, node.layer.cycle_offset);
        let values = node.curves.each_ref().map(|curve| curve.sample(t).to_bits());
        let want = [0, 1, 2].map(|i| bits(&c["values"][i]));
        score(scores, tag, "sample").check(t.to_bits() == bits(&c["clipTime"]) && values == want,
            || format!("{c} -> {} {values:?}", t.to_bits()));
    }
    for family in ["pose", "poseWeights"] {
        for c in cases[family].as_array().into_iter().flatten() {
            let node = clip(c);
            let euler = [0, 1, 2].map(|i| f(&c["values"][i]));
            let q = law::euler_zxy_degrees_to_quaternion(euler);
            let reference = law::euler_zxy_degrees_to_quaternion(node.layer.reference_euler);
            let delta = law::additive_delta(reference, q);
            let base = if c["baseMask"].as_u64() == Some(1) { [123.0, 456.0, 789.0, 1011.0] } else { node.layer.default_rotation };
            let out = law::apply_additive(base, delta, f_or_one(&c["weight"]));
            let mut ok = qbits(q) == quad(&c["quat"]) && qbits(delta) == quad(&c["delta"]) && qbits(out) == quad(&c["out"]);
            if !c["ref"].is_null() {
                ok &= qbits(reference) == quad(&c["ref"]);
            }
            score(scores, tag, family).check(ok, || format!("{c} -> {:?} {:?} {:?}", qbits(q), qbits(delta), qbits(out)));
        }
    }
    for c in cases["add"].as_array().into_iter().flatten() {
        let out = law::apply_additive(quad(&c["base"]).map(f32::from_bits), quad(&c["delta"]).map(f32::from_bits), f(&c["weight"]));
        score(scores, tag, "add").check(qbits(out) == quad(&c["out"]), || format!("{c} -> {:?}", qbits(out)));
    }
    let prepare = cases["prepare"].as_array().map_or(&[][..], Vec::as_slice);
    let times = exact_prepare_times(key);
    assert_eq!(times.len(), prepare.len(), "{tag}: prepare decimal text");
    for (c, &(time, prev)) in prepare.iter().zip(&times) {
        // Same case: the parsed value is within a few units of the exact one.
        for (exact, parsed) in [(time, &c["time"]), (prev, &c["prev"])] {
            let parsed = parsed.as_f64().expect("time");
            assert!((exact - parsed).abs() <= exact.abs() * 1e-12, "{tag}: {exact} vs {parsed}");
        }
        let length = if c["lengthBits"].is_null() { f(&c["stop"]) - f(&c["start"]) } else { f(&c["lengthBits"]) };
        let a = law::clip_input_time(time, length);
        let b = law::clip_input_time(prev, length);
        score(scores, tag, "prepare").check(a.to_bits() == bits(&c["out"][0]) && b.to_bits() == bits(&c["out"][1]),
            || format!("{c} -> {} {}", a.to_bits(), b.to_bits()));
    }
    for c in cases["state"].as_array().into_iter().flatten() {
        let speed = law::state_speed(f_or_one(&c["animatorSpeed"]), f_or_one(&c["stateSpeed"]));
        let n = law::advance_state_time(f(&c["nOld"]), f(&c["dt"]), f(&c["D"]), speed);
        score(scores, tag, "state").check(n.to_bits() == bits(&c["out"]["n"]), || format!("{c} -> {}", n.to_bits()));
    }
    for c in cases["propagate"].as_array().into_iter().flatten() {
        let node = clip(c);
        let length = node.layer.stop - node.layer.start;
        let t = law::clip_playable_time(length, f(&c["a"]));
        let p = law::clip_playable_time(length, f(&c["c"]));
        score(scores, tag, "propagate").check(
            t.to_bits() == c["out"]["timeBits"].as_u64().unwrap() && p.to_bits() == c["out"]["prevBits"].as_u64().unwrap(),
            || format!("{c} -> {} {}", t.to_bits(), p.to_bits()));
    }
    for c in cases["blendtree"].as_array().into_iter().flatten() {
        let d = law::single_leaf_state_duration(f(&c["start"]), f(&c["stop"]), f_or_one(&c["leafDuration"]));
        let want = if c["out"].is_object() { bits(&c["out"]["duration"]) } else { bits(&c["out"]) };
        score(scores, tag, "blendtree").check(d.to_bits() == want, || format!("{c} -> {}", d.to_bits()));
    }
    for c in cases["write"].as_array().into_iter().flatten() {
        let w = law::transform_rotation(quad(&c["q"]).map(f32::from_bits));
        score(scores, tag, "write").check(qbits(w) == quad(&c["out"]), || format!("{c} -> {:?}", qbits(w)));
    }
    // The per-frame chain through the production Animator: one instance per
    // recorded sequence, holding the sequence's first input state time.
    let mut running: HashMap<(String, String), (EffectAnimator, u32)> = HashMap::new();
    for c in cases["chain"].as_array().into_iter().flatten() {
        let name = c["clip"].as_str().expect("clip name").to_owned();
        let sequence = c["seq"].as_str().expect("sequence").to_owned();
        let node = &clips[&name];
        let key = (name.clone(), sequence.clone());
        if c["frame"] == 0 {
            let animator = EffectAnimator::with_state_times(vec![node.clone()], vec![f(&c["nIn"])]);
            assert!(running.insert(key.clone(), (animator, 0)).is_none(), "{name} {sequence} restarted");
        }
        let (animator, frame) = running.get_mut(&key).unwrap_or_else(|| panic!("{name} {sequence}: no first frame"));
        assert_eq!(c["frame"].as_u64(), Some(u64::from(*frame)), "{name} {sequence}: frames out of order");
        animator.advance_frame(*frame, f(&c["dt"]));
        animator.advance_frame(*frame, f(&c["dt"]));
        *frame += 1;
        let snapshot = animator.snapshot(&node.path).expect("evaluated layer");
        let ok = snapshot.state_time.to_bits() == bits(&c["n"])
            && snapshot.euler.map(f32::to_bits) == [0, 1, 2].map(|i| bits(&c["values"][i]))
            && qbits(snapshot.rotation) == quad(&c["written"])
            && animator.rotation(&node.path) == Some(snapshot.rotation);
        score(scores, tag, "chain").check(ok, || format!("{name} {sequence} frame {} -> {snapshot:?}", c["frame"]));
    }
}

#[test]
#[ignore = "requires MOLY_SKY_ANIMATOR_NATIVE_1, _NATIVE_2, _FRESH, _WRAP, _REFSUB, _MULTIKEY, _EFFECTS, _ANIMATIONS and _REPORT"]
fn sky_animator_matches_native_receipts() {
    let mut scores: BTreeMap<String, Score> = BTreeMap::new();
    let clips = players(&read("MOLY_SKY_ANIMATOR_NATIVE_1"), &mut scores);
    for (tag, key) in [("native1", "MOLY_SKY_ANIMATOR_NATIVE_1"), ("native2", "MOLY_SKY_ANIMATOR_NATIVE_2"),
        ("fresh", "MOLY_SKY_ANIMATOR_FRESH"), ("wrap", "MOLY_SKY_ANIMATOR_WRAP")] {
        replay_families(tag, key, &clips, &mut scores);
    }
    // Non-identity additive references (every clip of the receipts starts at zero).
    let refsub = read("MOLY_SKY_ANIMATOR_REFSUB");
    for c in refsub["cases"].as_array().expect("refsub cases") {
        let starts = [0, 1, 2].map(|i| f(&c["starts"][i]));
        let reference = law::euler_zxy_degrees_to_quaternion(starts);
        let delta = law::additive_delta(reference, quad(&c["value"]).map(f32::from_bits));
        scores.entry("refsub/reference".into()).or_default().check(
            qbits(reference) == quad(&c["ref"]) && qbits(delta) == quad(&c["delta"]),
            || format!("{c} -> {:?} {:?}", qbits(reference), qbits(delta)));
    }
    // Streamed cubic clips with interior keys, through the product sampler.
    let multikey = read("MOLY_SKY_ANIMATOR_MULTIKEY");
    let synthetic: Vec<(f32, Vec<Curve>)> = multikey["clips"].as_array().expect("multikey clips").iter().map(|clip| {
        let curves = clip["curves"].as_array().expect("curves").iter().map(|keys| {
            Curve::Cubic(keys.as_array().expect("keys").iter().map(|key| {
                let coefficients = key[1].as_array().expect("coefficients");
                (f(&key[0]), [0, 1, 2, 3].map(|i| f(&coefficients[i])))
            }).collect())
        }).collect();
        (f(&clip["stop"]), curves)
    }).collect();
    for c in multikey["cases"].as_array().expect("multikey cases") {
        let (stop, curves) = &synthetic[c["clip"].as_u64().expect("clip index") as usize];
        let t = law::loop_clip_time(f(&c["normalized"]), 0.0, *stop, 0.0);
        let values: Vec<u32> = curves.iter().map(|curve| curve.sample(t).to_bits()).collect();
        let want: Vec<u32> = c["values"].as_array().expect("values").iter().map(bits).collect();
        scores.entry("multikey/sample".into()).or_default().check(t.to_bits() == bits(&c["clipTime"]) && values == want,
            || format!("{c} -> {} {values:?}", t.to_bits()));
    }
    let report: serde_json::Map<String, Value> = scores.iter().map(|(k, s)| (k.clone(),
        serde_json::json!({"cases": s.cases, "mismatches": s.mismatches, "first": s.first}))).collect();
    std::fs::write(std::env::var_os("MOLY_SKY_ANIMATOR_REPORT").expect("MOLY_SKY_ANIMATOR_REPORT"),
        serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for (k, s) in &scores {
        assert!(s.cases > 0, "{k}: no cases");
        assert_eq!(s.mismatches, 0, "{k}: {:?}", s.first);
    }
}
