//! External current-lib ordinary StartParticles runs with authored start
//! colours, through the explicit birth entry: the colour a newborn stores
//! depends on the time the command hands InitialModule::Start. No generated
//! reference data; every expected byte is a native observation.
use super::*;
use moly_law::particle::gradient::{
    Gradient, GradientAlphaKey, GradientColorKey, GradientColorSpace, GradientMode,
};
use moly_law::particle::initial::initial_reciprocal;
use moly_law::particle::schema::StartParams;
use moly_law::particle::seed_owner::ModuleRandom;
use moly_law::particle::sub_emission::{BirthBatch, BirthDistribution};
use moly_law::particle::{MinMaxCurve, MinMaxGradient, RingBufferMode};
use serde_json::Value;

fn exact(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("native integer observation")).unwrap()
}

fn bits(value: &Value) -> f32 {
    f32::from_bits(exact(value))
}

fn words(value: &Value) -> ModuleRandom {
    let list = value.as_array().unwrap();
    assert_eq!(list.len(), 16);
    ModuleRandom {
        words: std::array::from_fn(|word| std::array::from_fn(|lane| exact(&list[word * 4 + lane]))),
    }
}

fn component(value: &Value) -> f32 {
    value.as_f64().expect("finite exported component") as f32
}

/// The exported block shape the receipt stores verbatim. The production
/// decoder of these blocks is exercised by the law-level replay; this entry
/// only needs the values.
fn gradient(value: &Value) -> Gradient {
    Gradient {
        color_keys: value["colorKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| GradientColorKey {
                time: component(&key["time"]),
                color: std::array::from_fn(|c| component(&key["color"][c])),
            })
            .collect(),
        alpha_keys: value["alphaKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| GradientAlphaKey {
                time: component(&key["time"]),
                alpha: component(&key["alpha"]),
            })
            .collect(),
        mode: match value["interpolation"].as_str() {
            Some("blend") => GradientMode::Blend,
            Some("fixed") => GradientMode::Fixed,
            other => panic!("interpolation {other:?}"),
        },
        color_space: GradientColorSpace::Unspecified,
    }
}

fn block(value: &Value) -> MinMaxGradient {
    let vec4 = |name: &str| -> [f32; 4] { std::array::from_fn(|c| component(&value[name][c])) };
    match value["mode"].as_str() {
        Some("color") => MinMaxGradient::Color(vec4("color")),
        Some("twoColors") => MinMaxGradient::TwoColors {
            min: vec4("min"),
            max: vec4("max"),
        },
        Some("gradient") => MinMaxGradient::Gradient(gradient(&value["gradient"])),
        Some("randomColor") => MinMaxGradient::RandomColor(gradient(&value["gradient"])),
        Some("twoGradients") => MinMaxGradient::TwoGradients {
            min: gradient(&value["minGradient"]),
            max: gradient(&value["maxGradient"]),
        },
        other => panic!("mode {other:?}"),
    }
}

#[test]
#[ignore = "MOLY_INITIAL_COLOUR_RECEIPT must identify the current native start colour receipt"]
fn ordinary_birth_colour_matches_current_native_start_particles() {
    let path = std::env::var("MOLY_INITIAL_COLOUR_RECEIPT")
        .expect("MOLY_INITIAL_COLOUR_RECEIPT must name the native start colour receipt");
    let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        receipt["library"]["sha256"],
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
    );
    let blocks: std::collections::HashMap<&str, MinMaxGradient> = receipt["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry["id"].as_str().unwrap(), block(&entry["block"])))
        .collect();
    let context = Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    };
    let (mut runs, mut compared, mut time_dependent) = (0, 0, 0);
    for row in receipt["pipeline"].as_array().unwrap() {
        let run = exact(&row["id"]);
        let colour = blocks[row["blockRef"].as_str().unwrap()].clone();
        time_dependent += usize::from(matches!(
            colour,
            MinMaxGradient::Gradient(_) | MinMaxGradient::TwoGradients { .. }
        ));
        let mut system = test_support::runtime();
        system.pool.clear();
        system.side.clear();
        system.born_total = 0;
        system.emitter.max_particles = 32;
        system.emitter.prewarm = false;
        system.prewarmed = true;
        system.emitter.ring_buffer_mode = RingBufferMode::Disabled;
        system.emitter.simulation_space = SimulationSpace::Local;
        system.emitter.shape = None;
        system.emitter.shape_enabled = Some(false);
        // The harness's Initial configuration: two-constant scalar curves.
        system.emitter.start = StartParams {
            lifetime: MinMaxCurve::TwoConstants { min: 2.0, max: 4.0 },
            speed: MinMaxCurve::Constant(0.0),
            size: MinMaxCurve::TwoConstants { min: 1.0, max: 2.0 },
            size_y: None,
            size_z: None,
            size3d: false,
            rotation: MinMaxCurve::TwoConstants { min: 0.5, max: 1.0 },
            rotation_x: None,
            rotation_y: None,
            rotation3d: false,
            color: colour,
            gravity_modifier: MinMaxCurve::Constant(0.0),
        };
        let duration = bits(&row["durationBits"]);
        let current = bits(&row["currentBits"]);
        let dt = bits(&row["dtBits"]);
        system.emitter.duration = duration;
        // The ordinary slice's normalized endpoints, as the autonomous
        // schedule produces them from the duration reciprocal.
        let inverse = initial_reciprocal(duration).unwrap();
        let distribution = &row["distributionBits"];
        let batch = BirthBatch {
            count: exact(&row["requested"]),
            rate_count: exact(&row["rateCount"]),
            distribution: BirthDistribution {
                spacing: bits(&distribution[0]),
                offset: bits(&distribution[1]),
                burst_fraction: bits(&distribution[2]),
            },
        };
        let mut random = words(&row["rngBefore"]);
        super::birth::start_explicit(
            &mut system,
            &mut random,
            batch,
            dt,
            (current - dt) * inverse,
            current * inverse,
            &context,
        )
        .unwrap_or_else(|refused| panic!("run {run} refused {refused:?}"));
        let expected = row["colourBytesAfterPack"].as_array().unwrap();
        assert_eq!(system.side.len(), exact(&row["bornCount"]) as usize, "run {run} born");
        assert_eq!(expected.len(), system.side.len());
        for (index, (side, bytes)) in system.side.iter().zip(expected).enumerate() {
            let bytes: [u8; 4] = std::array::from_fn(|c| u8::try_from(exact(&bytes[c])).unwrap());
            let stored = moly_law::particle::gradient::rgba8_to_float(bytes);
            assert_eq!(
                side.colour.map(f32::to_bits),
                stored.map(f32::to_bits),
                "run {run} particle {index} stored colour"
            );
            compared += 1;
        }
        assert_eq!(random, words(&row["rngAfter"]), "run {run} Initial words after");
        runs += 1;
    }
    assert_eq!((runs, compared), (25, 150));
    println!(
        "ordinary birth colour: {runs} native runs ({time_dependent} with a time-evaluated colour), \
         {compared} stored colours exact"
    );
}
