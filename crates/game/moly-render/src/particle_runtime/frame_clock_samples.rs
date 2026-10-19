//! The app's frame clocks against native TimeManager receipts: Bevy's time
//! system drives `Time<Virtual>` with the product's maximum frame, and the
//! product's Time.deltaTime and Time.unscaledDeltaTime are read the way the
//! weather host reads them. The engine takes its elapsed time in double
//! precision and the app's real clock counts whole nanoseconds. A receipt whose
//! frames carry the engine clock's nanosecond reading is fed those readings; the
//! other receipt's readings are rounded to the nearest nanosecond. A frame whose
//! nanosecond reading rounds to a different float under the engine's own law is
//! counted and left out; every other frame must equal the engine bit for bit.
//! The designed intervals of the first receipt cannot tell one rounding of the
//! nanosecond duration from two; the random whole-nanosecond frames of the
//! second can.
use super::*;
use bevy::time::{TimePlugin, TimeSystems, TimeUpdateStrategy};
use moly_law::particle::frame_time::{source_delta_seconds, unscaled_delta_seconds};
use serde_json::Value;
use std::time::Duration;

const SOURCE_SHA256: &str = "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9";

#[derive(Resource, Default)]
struct Observed {
    scaled: f32,
    unscaled: f32,
}

fn observe(time: Res<Time>, unscaled: Res<UnscaledFrameClock>, mut out: ResMut<Observed>) {
    out.scaled = source_delta_time(time.delta());
    out.unscaled = unscaled.delta();
}

/// An app with only the time plugin, the given virtual-clock maximum and the
/// product's unscaled holder in the place the game schedule gives it.
fn clock_app(max_delta: Duration) -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin);
    app.world_mut().resource_mut::<Time<Virtual>>().set_max_delta(max_delta);
    app.init_resource::<UnscaledFrameClock>()
        .init_resource::<Observed>()
        .add_systems(First, advance_unscaled_clock.after(TimeSystems))
        .add_systems(Update, observe);
    app
}

fn hex32(value: &Value) -> u32 {
    u32::from_str_radix(value.as_str().expect("hex word"), 16).expect("hex word")
}

fn clock_reading(frame: &Value) -> f64 {
    let bits = u64::from_str_radix(frame["nowBits"].as_str().expect("nowBits"), 16).expect("nowBits");
    f64::from_bits(bits)
}

fn nanoseconds(seconds: f64) -> u64 {
    assert!(seconds.is_finite() && seconds >= 0.0 && seconds < 1.8e10, "{seconds}");
    (seconds * 1e9).round() as u64
}

#[derive(Default, Debug)]
struct Counts {
    scaled: usize,
    scaled_miss: usize,
    scaled_clamped: usize,
    scaled_band: usize,
    scaled_sensitive: usize,
    unscaled: usize,
    unscaled_miss: usize,
    unscaled_sensitive: usize,
    unscaled_desynced: usize,
    separating: usize,
}

/// One receipt case through a fresh app. `unscaled` also compares the
/// unscaled holder; the control arm compares the scaled clock only.
fn run_case(case: &Value, max_delta: Duration, unscaled: bool, counts: &mut Counts) {
    let name = case["name"].as_str().unwrap_or("?");
    let words = case["serialized"].as_str().expect("serialized");
    assert_eq!(words.len(), 32, "{name}");
    let word = |i: usize| u32::from_str_radix(&words[i * 8..i * 8 + 8], 16).expect("word").swap_bytes();
    let player = PLAYER_TIME.maximum_delta_time;
    let player_maximum = word(1) == player.to_bits();
    let start = case["start"].as_f64().expect("start");
    let start_ns = case.get("startNanos").map_or_else(|| nanoseconds(start), |v| v.as_u64().expect("startNanos"));
    let capture_null = case["capture"].is_null();
    let scaled_case = player_maximum
        && word(2) == 0x3f80_0000
        && capture_null
        && case["worldPlaying"].as_bool() == Some(true)
        && start < 1e6;
    let compare_unscaled = unscaled && capture_null;
    if !scaled_case && !compare_unscaled {
        return;
    }
    let mut app = clock_app(max_delta);
    // The first update only starts the real clock at the case's start reading.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    let (mut previous, mut previous_ns) = (start, start_ns);
    let (mut reference, mut reference_ns) = (start, start_ns);
    let mut desynced = false;
    for frame in case["frames"].as_array().expect("frames") {
        let index = frame["frame"].as_u64().expect("frame");
        let now = clock_reading(frame);
        let now_ns = frame.get("nowNanos").map_or_else(|| nanoseconds(now), |v| v.as_u64().expect("nowNanos"));
        assert!(now_ns >= previous_ns, "{name} frame {index}: clock runs backwards");
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_nanos(now_ns - previous_ns)));
        app.update();
        let observed = app.world().resource::<Observed>();
        let (scaled_ours, unscaled_ours) = (observed.scaled, observed.unscaled);
        let getters = &frame["getters"];
        let elapsed = now - previous;
        let elapsed_ns = Duration::from_nanos(now_ns - previous_ns);
        previous = now;
        previous_ns = now_ns;
        let paused = frame["paused"].as_bool() == Some(true);
        if scaled_case && index >= 2 && !paused {
            let exact = source_delta_seconds(elapsed, player);
            let quantized = source_delta_seconds(elapsed_ns.as_secs_f64(), player);
            if exact.to_bits() != quantized.to_bits() {
                counts.scaled_sensitive += 1;
            } else {
                let native = f32::from_bits(hex32(&getters["deltaTime"]));
                counts.scaled += 1;
                if elapsed > f64::from(player) {
                    counts.scaled_clamped += 1;
                }
                if elapsed > 0.25 && elapsed <= f64::from(player) {
                    counts.scaled_band += 1;
                }
                if scaled_ours.to_bits() != native.to_bits() {
                    counts.scaled_miss += 1;
                    if unscaled {
                        eprintln!("scaled {name} frame {index}: native {:08x} ours {:08x}", native.to_bits(), scaled_ours.to_bits());
                    }
                }
            }
        }
        if !compare_unscaled || desynced {
            continue;
        }
        let exact = unscaled_delta_seconds(now - reference);
        let quantized = unscaled_delta_seconds(Duration::from_nanos(now_ns - reference_ns).as_secs_f64());
        if exact.consumed != quantized.consumed {
            // The nanosecond clock and the engine's double clock disagree on
            // whether this frame consumes its time; the holders part here, so
            // the rest of the case is not compared.
            desynced = true;
            counts.unscaled_desynced += 1;
            continue;
        }
        if exact.consumed {
            reference = now;
            reference_ns = now_ns;
        }
        // The getter reads the active holder, which the first frame after a
        // reset without capture does not refresh.
        if index == 0 {
            continue;
        }
        if exact.value.to_bits() != quantized.value.to_bits() {
            counts.unscaled_sensitive += 1;
            continue;
        }
        let native = f32::from_bits(hex32(&getters["unscaledDeltaTime"]));
        counts.unscaled += 1;
        if unscaled_ours.to_bits() != native.to_bits() {
            counts.unscaled_miss += 1;
            eprintln!("unscaled {name} frame {index}: native {:08x} ours {:08x}", native.to_bits(), unscaled_ours.to_bits());
        } else if player_maximum && native > player && scaled_ours.to_bits() == player.to_bits() {
            counts.separating += 1;
        }
    }
}

/// Every case of one receipt through the product and the control arms.
fn run_receipt(var: &str) -> (Counts, Counts) {
    let path = std::env::var_os(var).unwrap_or_else(|| panic!("{var}"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", std::path::Path::new(&path).display()));
    let receipt: Value = serde_json::from_slice(&bytes).expect("receipt json");
    assert_eq!(receipt["sha256"].as_str(), Some(SOURCE_SHA256), "{var}");
    let mut product = Counts::default();
    let mut control = Counts::default();
    for case in receipt["updateCases"].as_array().expect("updateCases") {
        run_case(case, PLAYER_MAXIMUM_DELTA, true, &mut product);
        run_case(case, Time::<Virtual>::default().max_delta(), false, &mut control);
    }
    println!("app clocks, {var}, product maximum: {product:?}");
    println!("app clocks, {var}, default virtual maximum (control): scaled {} mismatched {}", control.scaled, control.scaled_miss);
    (product, control)
}

#[test]
#[ignore = "MOLY_TIMESTEP_RECEIPT and MOLY_TIMESTEP_NS_RECEIPT must name the native TimeManager receipts"]
fn app_frame_clocks_match_native_time_manager() {
    for var in ["MOLY_TIMESTEP_RECEIPT", "MOLY_TIMESTEP_NS_RECEIPT"] {
        let (product, control) = run_receipt(var);
        assert!(product.scaled > 0 && product.unscaled > 0, "{var}: receipt compared nothing");
        // The frames that decide the finding must be present: frames the engine
        // clamps, and frames longer than the default virtual maximum that the
        // engine does not clamp.
        assert!(product.scaled_clamped > 0 && product.scaled_band > 0, "{var}: no clamped or 250 ms band frame compared");
        assert!(product.separating > 0, "{var}: no frame separates the unscaled clock from the clamped one");
        // The control arm shows the replay sees the default maximum.
        assert!(control.scaled_miss > 0, "{var}: the default virtual maximum went unnoticed");
        assert_eq!((product.scaled_miss, product.unscaled_miss), (0, 0), "{var}");
    }
}
