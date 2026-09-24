//! Current-native channel observations replayed through simulation and presentation.
use super::*;
use moly_law::particle::{Curve, CurveKey, MinMaxCurve, custom_data::CustomData,
    schema::{CustomDataParams, CustomDataSlot}};
use serde_json::{Value, json};

fn number(value: &Value) -> f32 { value.as_f64().unwrap() as f32 }

fn curve(value: &Value) -> MinMaxCurve {
    let line = |key: &str| {
        let a = number(&value[key][0]);
        let b = number(&value[key][1]);
        let slope = if value["linear"] == true { b - a } else { 0.0 };
        Curve { multiplier: 1.0, keys: [0.0, 1.0].map(|time| CurveKey {
            time, value: if time == 0.0 { a } else { b },
            in_slope: slope, out_slope: slope, weighted_mode: 0, in_weight: 0.0, out_weight: 0.0,
        }).to_vec() }
    };
    match value["mode"].as_u64().unwrap() {
        0 => MinMaxCurve::Constant(number(&value["max"])),
        3 => MinMaxCurve::TwoConstants { min: number(&value["min"]), max: number(&value["max"]) },
        1 => MinMaxCurve::Curve { multiplier: number(&value["multiplier"]), max: line("high") },
        2 => MinMaxCurve::TwoCurves { multiplier: number(&value["multiplier"]), min: line("low"), max: line("high") },
        mode => panic!("unsupported source observation mode {mode}"),
    }
}

fn context() -> Context {
    Context { sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY }
}

#[test]
fn rendering_reads_custom_channels_from_before_the_age_step() {
    let mut system = test_support::runtime();
    let data = CustomDataParams {
        custom1: Some(CustomDataSlot { component_count: 1,
            components: vec![curve(&json!({"mode":1, "multiplier":1, "linear":true, "high":[0,1]}))] }),
        custom2: None,
    };
    system.custom_law = Some(CustomData::from_params(&data));
    system.emitter.custom_data = Some(data);
    system.pool[0] = Particle::born([0.0;3], [0.0;3], 1.0);
    system.pool[0].age_percent = 50.0;
    system.side[0].custom_data = [[-777.0;4];2];
    simulate_stopped(&mut system, 0.25, &context());
    assert_eq!(system.pool[0].age_percent, 75.0);
    for _ in 0..2 {
        let quad = &build_quads(&system, &GlobalTransform::IDENTITY)[0];
        assert_eq!(quad.custom1, [0.5,-777.0,-777.0,-777.0]);
        assert_eq!(quad.custom2, [-777.0;4]);
    }
}

#[test]
#[ignore = "MOLY_PARTICLE_CUSTOM_SAMPLES must identify current-native observations"]
fn custom_data_matches_native_channel_hashes() {
    let path = std::env::var("MOLY_PARTICLE_CUSTOM_SAMPLES").expect("native samples required");
    let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows = data["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "empty observations cannot validate a consumer");
    let mut failures = Vec::new();
    let mut active_channels = 0;
    let mut retained_channels = 0;
    for (row_index, row) in rows.iter().enumerate() {
        let mut system = test_support::runtime();
        let side = system.side[0];
        system.pool.clear(); system.side.clear();
        let slot = |stream: usize| row["enabled"][stream].as_bool().unwrap().then(|| {
            let count = row["counts"][stream].as_u64().unwrap() as usize;
            CustomDataSlot { component_count: count,
                components: (0..count).map(|channel| curve(&row["curves"][stream*4+channel])).collect() }
        });
        let params = CustomDataParams { custom1: slot(0), custom2: slot(1) };
        system.custom_law = Some(CustomData::from_params(&params));
        system.emitter.custom_data = Some(params);
        for lane in 0..4 {
            let mut p = Particle::born([0.0;3], [0.0;3], 100.0);
            p.age_percent = number(&row["ages"][lane]);
            system.pool.push(p);
            system.side.push(Side { seed: row["seeds"][lane].as_u64().unwrap() as u32,
                custom_data: [[-777.0;4];2], ..side });
        }
        simulate_stopped(&mut system, 0.125, &context());
        assert_eq!(system.pool.len(), 4);
        for frame in 0..2 {
            for (lane, quad) in build_quads(&system, &GlobalTransform::IDENTITY).iter().enumerate() {
                let values = [quad.custom1, quad.custom2];
                for stream in 0..2 {
                    for channel in 0..4 {
                        if frame == 0 {
                            if row["enabled"][stream] == true && channel < row["counts"][stream].as_u64().unwrap() as usize {
                                active_channels += 1;
                            } else { retained_channels += 1; }
                        }
                        let actual = values[stream][channel];
                        let expected = number(&row["expected"][lane][stream*4+channel]);
                        if actual.to_bits() != expected.to_bits() {
                            failures.push(json!({"row":row_index,"frame":frame,"lane":lane,"stream":stream,
                                "channel":channel,"actual":actual,"expected":expected,
                                "actualBits":actual.to_bits(),"expectedBits":expected.to_bits()}));
                        }
                    }
                }
            }
        }
    }
    let report = json!({"cases":rows.len(),"particles":rows.len()*4,"activeChannels":active_channels,
        "retainedChannels":retained_channels,"presentationReads":rows.len()*4*8*2,
        "comparison":"bit exact including signed zero","failureCount":failures.len(),
        "firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path) = std::env::var("MOLY_PARTICLE_CUSTOM_REPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}"); assert!(failures.is_empty(), "{report}");
}

