//! Research instrument: replays the native ring-buffer lifecycle frames of the
//! current player library through the law functions the runtime composes on
//! every ordinary slice. The frames come from a private receipt; nothing here
//! compares against this crate's own output.
//!
//! Per frame, in the order of the engine's ordinary incremental update:
//! 1. existing particles: `advance_lifetime` (Loop wraps only indices below
//!    maxParticles; the overflow span ages as Disabled) then `compact_with_side`,
//!    compared with the pool the engine holds when the post-simulation modules run;
//! 2. `birth_capacity` over the count StartParticles saw and the requested count,
//!    compared with the newborn count handed to CopyParticlesToUnalignedDst;
//! 3. `finish_births` over the old prefix plus the newborn lanes, compared with
//!    the frame-end pool, the ring cursor and every Pause RecordParticleDeath.
//!
//! The negative arm replays every ring case as if the mode were Disabled; each
//! of them must then disagree with the engine somewhere.
use super::buffer::{birth_capacity, compact_with_side, finish_births, RingBufferMode};
use super::json::{self, Value};
use super::step::{advance_lifetime, Particle};

fn words(value: &Value) -> Vec<u32> {
    value.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as u32).collect()
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).unwrap().as_f64().unwrap() as usize
}

fn present<'a>(frame: &'a Value, key: &str) -> Option<&'a Value> {
    frame.get(key).filter(|v| !matches!(v, Value::Null))
}

fn pool(value: &Value) -> (Vec<Particle>, Vec<u32>) {
    let age = words(value.get("age").unwrap());
    let inv = words(value.get("inv").unwrap());
    let seed = words(value.get("seed").unwrap());
    let particles = age.iter().zip(&inv).map(|(&a, &i)| {
        let inverse = f32::from_bits(i);
        Particle { position: [0.0; 3], velocity: [0.0; 3], start_lifetime: 1.0 / inverse,
            age_percent: f32::from_bits(a), inverse_lifetime: inverse }
    }).collect();
    (particles, seed)
}

fn same(particles: &[Particle], seeds: &[u32], expected: &Value) -> bool {
    let (want, want_seeds) = pool(expected);
    particles.len() == want.len() && seeds == want_seeds.as_slice()
        && particles.iter().zip(&want).all(|(a, b)| a.age_percent.to_bits() == b.age_percent.to_bits()
            && a.inverse_lifetime.to_bits() == b.inverse_lifetime.to_bits())
}

#[derive(Default)]
struct Replay { frames: usize, births: usize, victims: usize, ring_birth_frames: usize,
    existing: usize, capacity: usize, packing: usize, failures: Vec<String> }

fn replay(case: &Value, mode: RingBufferMode) -> Replay {
    let label = case.get("label").unwrap().as_str().unwrap();
    let maximum = number(case, "maximum");
    let range = words(case.get("loopRange").unwrap());
    let range = [f32::from_bits(range[0]), f32::from_bits(range[1])];
    let mut out = Replay::default();
    for (index, frame) in case.get("frames").unwrap().as_array().unwrap().iter().enumerate() {
        out.frames += 1;
        let dt = f32::from_bits(frame.get("dt").unwrap().as_f64().unwrap() as u32);
        let mut cursor = number(frame, "cursorBefore");
        let (mut existing, mut seeds) = pool(frame.get("before").unwrap());
        for (i, particle) in existing.iter_mut().enumerate() {
            let lane = if mode == RingBufferMode::LoopUntilReplaced && i >= maximum {
                RingBufferMode::Disabled } else { mode };
            let _ = advance_lifetime(particle, dt, lane, range);
        }
        let _ = compact_with_side(&mut existing, &mut seeds, mode, maximum, |_, _| {});
        if let Some(post) = present(frame, "post") {
            out.existing += 1;
            if !same(&existing, &seeds, post) { out.failures.push(format!("{label} frame {index}: existing step")); }
        }
        if let (Some(start), Some(copy)) = (present(frame, "start"), present(frame, "copy")) {
            out.capacity += 1;
            let born = number(copy, "born");
            if birth_capacity(number(start, "count"), mode, maximum, number(start, "requested")) != born {
                out.failures.push(format!("{label} frame {index}: capacity"));
            }
            let (old, aligned) = (number(copy, "old"), number(copy, "aligned"));
            let (lanes, lane_seeds) = pool(copy);
            let mut work: Vec<Particle> = lanes[..old].to_vec();
            let mut work_seeds: Vec<u32> = lane_seeds[..old].to_vec();
            // A zero-birth Copy starts at the aligned index past the captured
            // lanes; there is nothing to append then.
            if born > 0 {
                work.extend_from_slice(&lanes[aligned..aligned + born]);
                work_seeds.extend_from_slice(&lane_seeds[aligned..aligned + born]);
            }
            let mut victims = Vec::new();
            finish_births(&mut work, &mut work_seeds, &mut cursor, mode, maximum, old,
                |p, s| victims.push([p.age_percent.to_bits(), *s]));
            let native: Vec<[u32; 2]> = frame.get("victims").unwrap().as_array().unwrap().iter()
                .map(|v| { let w = words(v); [w[0], w[1]] }).collect();
            out.packing += 1;
            out.births += born;
            out.victims += native.len();
            out.ring_birth_frames += usize::from(mode != RingBufferMode::Disabled && born > 0);
            if !same(&work, &work_seeds, frame.get("after").unwrap()) {
                out.failures.push(format!("{label} frame {index}: packing"));
            }
            if victims != native { out.failures.push(format!("{label} frame {index}: Pause victims")); }
        }
        if cursor != number(frame, "cursorAfter") {
            out.failures.push(format!("{label} frame {index}: cursor"));
        }
    }
    out
}

#[test]
#[ignore = "MOLY_RING_LIFECYCLE_NATIVE must name the executed ring lifecycle frames"]
fn ring_lifecycle_frames_match_the_native_update() {
    let path = std::env::var("MOLY_RING_LIFECYCLE_NATIVE")
        .expect("MOLY_RING_LIFECYCLE_NATIVE must name the executed ring lifecycle frames");
    let receipt = json::parse(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt.get("librarySha256").unwrap().as_str().unwrap(),
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
    let cases = receipt.get("cases").unwrap().as_array().unwrap();
    assert!(!cases.is_empty());
    let mut total = Replay::default();
    let (mut ring_cases, mut disabled_cases, mut negative_caught) = (0usize, 0usize, 0usize);
    for case in cases {
        let mode = RingBufferMode::from_u32(number(case, "mode") as u32).unwrap();
        let run = replay(case, mode);
        println!("case {} mode {:?} frames {} existing {} capacity {} packing {} births {} victims {} failures {}",
            case.get("label").unwrap().as_str().unwrap(), mode, run.frames, run.existing, run.capacity,
            run.packing, run.births, run.victims, run.failures.len());
        total.frames += run.frames; total.births += run.births; total.victims += run.victims;
        total.ring_birth_frames += run.ring_birth_frames; total.existing += run.existing;
        total.capacity += run.capacity; total.packing += run.packing;
        total.failures.extend(run.failures);
        if mode == RingBufferMode::Disabled { disabled_cases += 1; continue; }
        ring_cases += 1;
        // Negative arm: the same frames read as Disabled must fail somewhere.
        let naive = replay(case, RingBufferMode::Disabled);
        negative_caught += usize::from(!naive.failures.is_empty());
        println!("naive-disabled {} failures {}", case.get("label").unwrap().as_str().unwrap(), naive.failures.len());
    }
    println!("runs {} ring {ring_cases} disabled {disabled_cases} frames {} existing {} capacity {} packing {} newborns {} pause victims {} ring birth frames {} failures {} negative arm caught {negative_caught}/{ring_cases}",
        cases.len(), total.frames, total.existing, total.capacity, total.packing, total.births,
        total.victims, total.ring_birth_frames, total.failures.len());
    // Positive arms: the receipt exercises ring births and Pause replacement.
    assert!(total.ring_birth_frames > 0 && total.victims > 0);
    assert_eq!(negative_caught, ring_cases, "a ring case the Disabled reading cannot tell apart");
    assert!(total.failures.is_empty(), "{:?}", &total.failures[..total.failures.len().min(12)]);
}
