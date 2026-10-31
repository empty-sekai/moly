//! Corpus test: the rain family's three `effects.json` files go through the
//! schema whole, and a few hand-computed anchors are checked against them.
//! Per-emitter semantics are pinned by the synthetic tests; what is pinned
//! here is that the real files decode as a whole and their key values keep
//! their shape.
//!
//! `#[ignore]` plus an environment gate (`MOLY_ASSET_ROOT`): without the
//! variable the test is an ignored test listed by `cargo test -- --ignored`,
//! and with it the assertions run.

use crate::particle::buffer::RingBufferMode;
use crate::particle::schema::{Effects, SimulationSpace};
use crate::particle::value::MinMaxCurve;

/// The three rain-family files (relative to `<MOLY_ASSET_ROOT>/phenomena/`).
const RAIN_FAMILY: [&str; 3] = ["006_rain", "007_rainnight", "008_thunder"];

/// The hand anchors of the raindrop's primary emitter, from `006_rain`'s
/// `fx_env_site_006_common_raindrop` (the entry the consumer side carries
/// under the same name).
const RAINDROP_ANCHORS: &str = include_str!(
    // A synthetic anchor file, written by hand rather than copied from an
    // asset, whose emitter carries the same key values as the real file's
    // emitter of that name; the corpus test checks the two side by side.
    "anchor/006_raindrop.json"
);

fn load(path: &str) -> Option<Effects> {
    let root = std::env::var("MOLY_ASSET_ROOT").ok()?;
    let full = format!("{root}/phenomena/{path}/fx/effects.json");
    let bytes = std::fs::read(&full).ok()?;
    Some(Effects::from_json_str(&bytes).expect("rain family effects.json must parse"))
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn rain_family_parses_with_expected_emitter_counts() {
    let Some(fx) = load("006_rain") else { panic!("006_rain missing") };
    // 194 is the total emitter count of the three phenomena:
    // 006=48 + 007=53 + 008=93.
    let total: usize = RAIN_FAMILY
        .iter()
        .filter_map(|p| load(p))
        .map(|fx| fx.emitters.len())
        .sum();
    assert_eq!(total, 194, "rain family emitter count drifted");
    // 006 alone holds 48 emitters; the raindrop's primary emitter is the
    // anchor below.
    assert_eq!(fx.emitters.len(), 48);
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn raindrop_primary_emitter_matches_hand_anchors() {
    let Some(fx) = load("006_rain") else { panic!("006_rain missing") };
    let e = fx
        .emitters
        .iter()
        .find(|e| e.effect == "fx_env_site_006_common_raindrop" && e.node == "root/raindrop_01")
        .expect("raindrop_01 emitter present");
    // Hand anchors (values measured on the extraction side, which the schema
    // must reproduce unchanged):
    assert_eq!(e.duration, 1.0);
    assert!(e.prewarm && e.looping);
    assert_eq!(e.simulation_space, SimulationSpace::World);
    assert_eq!(e.ring_buffer_mode, RingBufferMode::Disabled);
    assert_eq!(e.max_particles, 300);
    assert_eq!(e.start.lifetime, MinMaxCurve::Constant(5.0));
    // Measured on the extraction side: raindrop_01's speed is
    // twoConstants{0,0}, because the raindrop's velocity comes entirely from
    // velocityOverLifetime and its birth speed is zero.
    assert_eq!(
        e.start.speed,
        MinMaxCurve::TwoConstants { min: 0.0, max: 0.0 }
    );
    let vol = e.velocity_over_lifetime.as_ref().unwrap();
    assert_eq!(
        vol.y,
        MinMaxCurve::TwoConstants { min: -0.5, max: -1.0 }
    );
    let shape = e.shape.as_ref().unwrap();
    assert_eq!(shape.shape_type, "Circle");
    assert_eq!(shape.radius, 50.0);
    assert_eq!(shape.radius_thickness, 1.0);
    assert_eq!(shape.rotation, [-90.0, 0.0, 0.0]);
    let emission = e.emission.as_ref().unwrap();
    assert_eq!(emission.rate_over_time, MinMaxCurve::Constant(60.0));
    // The value chain: velocity over lifetime y at lerp=0.5 gives -0.75, the
    // rain's falling mid-speed shape.
    assert!((vol.y.evaluate(0.0, 0.5) + 0.75).abs() < 1e-6);
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn synthetic_anchor_file_agrees_with_real_raindrop() {
    // The anchor file's emitter equals the real file's emitter of the same
    // name value by value. The anchor is synthetic, so this test catches the
    // anchor drifting away from the real file.
    let anchor = Effects::from_json_str(RAINDROP_ANCHORS.as_bytes()).unwrap();
    let Some(fx) = load("006_rain") else { panic!("006_rain missing") };
    let real = fx
        .emitters
        .iter()
        .find(|e| e.effect == "fx_env_site_006_common_raindrop" && e.node == "root/raindrop_01")
        .unwrap();
    let a = &anchor.emitters[0];
    assert_eq!(a.duration, real.duration);
    assert_eq!(a.max_particles, real.max_particles);
    assert_eq!(a.start.lifetime, real.start.lifetime);
    assert_eq!(a.start.speed, real.start.speed);
    assert_eq!(a.start.size, real.start.size);
    assert_eq!(a.start.gravity_modifier, real.start.gravity_modifier);
    assert_eq!(a.simulation_space, real.simulation_space);
    assert_eq!(a.ring_buffer_mode, real.ring_buffer_mode);
    assert_eq!(a.shape.as_ref().unwrap().radius, real.shape.as_ref().unwrap().radius);
    assert_eq!(a.shape.as_ref().unwrap().rotation, real.shape.as_ref().unwrap().rotation);
    assert_eq!(
        a.velocity_over_lifetime.as_ref().unwrap().y,
        real.velocity_over_lifetime.as_ref().unwrap().y
    );
    assert_eq!(
        a.emission.as_ref().unwrap().rate_over_time,
        real.emission.as_ref().unwrap().rate_over_time
    );
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn rain_family_unmapped_keys_are_named_not_dropped() {
    // A recognized unmapped key must appear in the list, so that data being
    // present without a law reading it stays visible. The rain family
    // contains subEmitters, customData and renderer (rotationOverLifetime and
    // limitVelocity are mapped into parameters and do not land in the list).
    let mut seen = std::collections::BTreeSet::new();
    for p in RAIN_FAMILY.iter() {
        if let Some(fx) = load(p) {
            for e in &fx.emitters {
                seen.extend(e.unmapped.iter().cloned());
            }
        }
    }
    for expected in ["subEmitters", "customData", "renderer"] {
        assert!(
            seen.contains(&expected.to_string()),
            "{expected} not in unmapped set: {seen:?}"
        );
    }
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn rain_family_all_evaluations_finite() {
    // Every emitter of the family: each start MinMax value must evaluate to a
    // finite number on a (t, lerp) grid, so any curve or gradient in the
    // corpus that decodes to NaN surfaces here.
    for p in RAIN_FAMILY.iter() {
        let Some(fx) = load(p) else { panic!("{p} missing") };
        for e in &fx.emitters {
            let values: Vec<f32> = vec![
                e.start.lifetime.evaluate(0.0, 0.5),
                e.start.speed.evaluate(0.5, 0.5),
                e.start.size.evaluate(1.0, 0.5),
                e.start.gravity_modifier.evaluate(0.0, 0.0),
                e.start_delay.evaluate(0.0, 0.5),
            ];
            for v in values {
                assert!(v.is_finite(), "{}: start value {}", e.node, v);
            }
            if let Some(vol) = &e.velocity_over_lifetime {
                for t in [0.0, 0.5, 1.0] {
                    let x = vol.x.evaluate(t, 0.5);
                    let y = vol.y.evaluate(t, 0.5);
                    let z = vol.z.evaluate(t, 0.5);
                    assert!(x.is_finite() && y.is_finite() && z.is_finite(), "{}", e.node);
                }
            }
        }
    }
}
