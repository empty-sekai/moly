//! CustomData curve caches through the product slice path against native
//! Update1Incremental frames of the 008 ground-strike parent, whose first
//! custom1 component repeats past its last key: the lane the history
//! certificate refuses and the law that follows the engine's storage admits.
//! The parent is built as the sub-emitter parent replay builds it (the
//! exported record, the admission helper's edges, the native harness's RNG
//! words, owner and frame times as inputs). Every Evaluate call on each
//! custom curve object (its time, value and the cache it left, the lanes past
//! each call's end included) is compared in order with the native call on the
//! same object, then every live particle's custom values after each frame.
use super::*;
use super::sub_event_samples::{f, parent_runtime, read, word, words};
use moly_law::particle::custom_data::Evaluation;
use moly_law::particle::seed_owner::{ModuleRandom, ScalarRandom};
use serde_json::Value;

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

#[derive(Default)]
struct Tally {
    runs: usize,
    frames: usize,
    evaluations: usize,
    outputs: usize,
    mismatched: usize,
    first: Vec<String>,
}

impl Tally {
    fn check(&mut self, ok: bool, label: impl FnOnce() -> String) {
        self.mismatched += usize::from(!ok);
        if !ok && self.first.len() < 16 {
            self.first.push(label());
        }
    }
}

fn replay(doc: &Value, run: &Value, tally: &mut Tally) {
    let label = run["label"].as_str().unwrap();
    let image = &run["image"];
    let (mut system, edges) = parent_runtime(doc, &run["source"], |_| {}).expect("native birth path");
    assert!(system.custom_law.as_ref().is_some_and(|custom| custom.tracks_storage()),
        "{label}: the certificate admits this lane; the replay is for the storage law");
    let owner_words = words(&image["owner"]);
    let source_owner: [f32; 16] = std::array::from_fn(|i| f32::from_bits(owner_words[i]));
    let runtime_owner: [f32; 16] = std::array::from_fn(|i|
        if (i % 4 == 0) ^ (i / 4 == 0) { -source_owner[i] } else { source_owner[i] });
    system.node_affine = GlobalTransform::from(Mat4::from_cols_array(&runtime_owner));
    let ctx = Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(matches!(install_native_birth(&mut system, &mut manager, &SourceRoute::Ordinary, None).unwrap(), BirthPath::Native));
    {
        let state = system.native_birth.as_mut().unwrap();
        state.owner = None;
        let module = |key: &str| {
            let w = words(&image[key]);
            ModuleRandom { words: std::array::from_fn(|i| std::array::from_fn(|lane| w[i * 4 + lane])) }
        };
        state.initial = module("initialWords");
        state.shape = module("shapeWords");
        let emission = words(&image["emissionWords"]);
        state.emission.random = ScalarRandom { words: std::array::from_fn(|i| emission[i]) };
        state.events = Some(BirthEvents::with_edges(edges));
    }
    system.playback_head = f(&image["clock"]);
    system.previous_head = system.playback_head;
    system.custom_law.as_mut().unwrap().trace = Some(Vec::new());
    let counts: Vec<usize> = ["custom1", "custom2"].iter().map(|key| {
        let law = system.emitter.custom_data.as_ref().unwrap();
        let slot = if *key == "custom1" { &law.custom1 } else { &law.custom2 };
        slot.as_ref().map_or(0, |s| s.component_count)
    }).collect();
    for frame in run["frames"].as_array().unwrap() {
        let at = format!("{label} frame {}", frame["frame"]);
        let mut state = system.native_birth.take().unwrap();
        let result = birth::run_incremental(&mut system, &mut state, f(&frame["remainingBits"]),
            f(&frame["stepBits"]), false, &ctx);
        system.native_birth = Some(state);
        tally.check(result.is_ok(), || format!("{at}: refused {result:?}"));
        if result.is_err() {
            return;
        }
        tally.frames += 1;
        let after = &frame["after"];
        let count = word(&after["count"]) as usize;
        tally.check(system.pool.len() == count, || format!("{at}: count {} native {count}", system.pool.len()));
        if system.pool.len() != count {
            return;
        }
        tally.check(system.side.iter().map(|s| s.seed).collect::<Vec<_>>() == words(&after["seed"]),
            || format!("{at}: seeds"));
        tally.check(system.pool.iter().map(|p| p.age_percent.to_bits()).collect::<Vec<_>>() == words(&after["age"]),
            || format!("{at}: ages"));
        let custom = after["custom"].as_array().unwrap();
        for (stream, &n) in counts.iter().enumerate() {
            for component in 0..n {
                let native = words(&custom[stream][component]);
                for (index, side) in system.side.iter().enumerate() {
                    tally.outputs += 1;
                    let ours = side.custom_data[stream][component].to_bits();
                    tally.check(ours == native[index],
                        || format!("{at}: particle {index} custom {stream}.{component} ours {ours:#x} native {:#x}", native[index]));
                }
            }
        }
    }
    // Every evaluation of each curve object, in order.
    let ours: Vec<Evaluation> = system.custom_law.as_mut().unwrap().trace.take().unwrap();
    type Object = (usize, usize, bool);
    let mut by_object: std::collections::BTreeMap<Object, (Vec<&Evaluation>, Vec<&Value>)> = Default::default();
    for e in &ours {
        by_object.entry((e.stream, e.channel, e.min)).or_default().0.push(e);
    }
    for e in run["evaluations"].as_array().unwrap() {
        let min = e["side"].as_str() == Some("min");
        by_object.entry((word(&e["stream"]) as usize, word(&e["component"]) as usize, min)).or_default().1.push(e);
    }
    for (object, (ours, native)) in &by_object {
        tally.check(ours.len() == native.len(),
            || format!("{label} object {object:?}: {} evaluations, native {}", ours.len(), native.len()));
        for (n, (o, e)) in ours.iter().zip(native.iter()).enumerate() {
            tally.evaluations += 1;
            let after = words(&e["after"]);
            tally.check(o.time.to_bits() == word(&e["t"]) && o.value.to_bits() == word(&e["value"])
                && o.cache.words().to_vec() == after, || format!(
                "{label} object {object:?} evaluation {n}: t {:#x}/{:#x} value {:#x}/{:#x} cache {:x?}/{after:x?}",
                o.time.to_bits(), word(&e["t"]), o.value.to_bits(), word(&e["value"]), o.cache.words()));
        }
    }
    tally.runs += 1;
}

#[test]
#[ignore = "MOLY_CURVE_CACHE_FRAMES_NATIVE and MOLY_SUBEMITTER_PARENT_EFFECTS must identify the native curve-cache frame receipt and the exported 008 effects"]
fn product_custom_data_caches_match_native_frames() {
    let receipt = read("MOLY_CURVE_CACHE_FRAMES_NATIVE");
    assert_eq!(receipt["librarySha256"], SOURCE_SHA256);
    let doc = read("MOLY_SUBEMITTER_PARENT_EFFECTS");
    let mut tally = Tally::default();
    for run in receipt["frameRuns"].as_array().unwrap() {
        replay(&doc, run, &mut tally);
    }
    println!("product curve caches: {} runs, {} frames, {} evaluations, {} custom values, {} mismatches",
        tally.runs, tally.frames, tally.evaluations, tally.outputs, tally.mismatched);
    assert!(tally.mismatched == 0 && tally.first.is_empty(), "first mismatches: {:#?}", tally.first);
    assert!(tally.runs > 0 && tally.evaluations > 0 && tally.outputs > 0);
}
