//! 语料测试：rain 家族三份 effects.json 全量走 schema 并抽几条
//! 手算锚（非整档比对——逐发射器语义已在合成测试钉住，这里钉的是
//! 「真档案能整体落进来且关键值不变形」）。
//!
//! `#[ignore]` + env 门（`MOLY_ASSET_ROOT`）：默认跳过时**是红的
//! skip**（`cargo test -- --ignored` 列表可见），env 在时真断言。
//! 不用「打印+return 绿」的形态——那会把「没比」伪装成「通过」。

use crate::particle::buffer::RingBufferMode;
use crate::particle::schema::{Effects, SimulationSpace};
use crate::particle::value::MinMaxCurve;

/// rain 家族的三份档案（相对 `<MOLY_ASSET_ROOT>/phenomena/`）。
const RAIN_FAMILY: [&str; 3] = ["006_rain", "007_rainnight", "008_thunder"];

/// 雨滴主发射器的手算锚，取自 `006_rain` 的
/// `fx_env_site_006_common_raindrop`（S22 消费侧同名条目）。
const RAINDROP_ANCHORS: &str = include_str!(
    // 合成的锚点档案（手写，非资产拷贝），与真档案同名发射器的
    // 关键参数逐值一致——见 `synthetic_raindrop_anchor.json` 与
    // corpus 测试的对照断言。
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
    // 194 = 三现象发射器总数（现算：006=48 + 007=53 + 008=93）。
    let total: usize = RAIN_FAMILY
        .iter()
        .filter_map(|p| load(p))
        .map(|fx| fx.emitters.len())
        .sum();
    assert_eq!(total, 194, "rain family emitter count drifted");
    // 006 自身 48 个发射器；雨滴主发射器见下条锚。
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
    // 手算锚（提取侧实测值，schema 不得变形）：
    assert_eq!(e.duration, 1.0);
    assert!(e.prewarm && e.looping);
    assert_eq!(e.simulation_space, SimulationSpace::World);
    assert_eq!(e.ring_buffer_mode, RingBufferMode::Disabled);
    assert_eq!(e.max_particles, 300);
    assert_eq!(e.start.lifetime, MinMaxCurve::Constant(5.0));
    // 提取侧实测：raindrop_01 的 speed 是 twoConstants{0,0}（雨滴速度
    // 全靠 velocityOverLifetime，出生速度为零族）。
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
    // 值链：VoL y 在 lerp=0.5 处 = -0.75（rain 的下落中值速度形状）。
    assert!((vol.y.evaluate(0.0, 0.5) + 0.75).abs() < 1e-6);
}

#[test]
#[ignore = "MOLY_ASSET_ROOT not set: 0 of 3 rain-family profiles were parsed"]
fn synthetic_anchor_file_agrees_with_real_raindrop() {
    // 锚档案与真档案同名发射器逐值一致（锚档案本身是合成的，此测试
    // 防的是锚档案腐烂，不是防提取侧变形）。
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
    // 识别未映射的键必须出现在清单里——「数据在、律没管」要可见。
    // rain 家族实测必含 subEmitters / customData / renderer
    // （rotationOverLifetime/limitVelocity 已映射进参数，不再落清单）。
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
    // 全族逐发射器：start 各 MinMax 值在 (t, lerp) 网格上求值必须有限
    // ——语料里任何一条曲线/梯度解析成 NaN 都在此暴露。
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
