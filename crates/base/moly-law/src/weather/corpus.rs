//! 语料比对：提取出的 15 份天气档案 × 各量，逐值对迁移前独立量得的
//! 真源锚。
//!
//! # 期望侧的来历
//!
//! 雾四元组（density/start/end/height）是实现存在之前从提取资产独立
//! 量得的表；近/远颜色与近/远密度是档案里其余的序列化
//! `MysekaiFogVolume` 值，逐字转写。期望 globals 在本测试内按
//! `MysekaiFogPass.ExecuteFog` 的转写手工展开——绝不调被测实现自己的
//! `globals()`，实现的式子错了不可能靠构造与期望一致。两边同序同
//! f32 运算，精确 `==` 是诚实的比较子。
//!
//! 档案根运行时读 `MOLY_ASSET_ROOT`；env 缺失时响亮说明「比了 0 份」
//! 再返回——该轮不产生证据，但也不假装失败与通过。

use std::path::{Path, PathBuf};

use super::{NotProfileDriven, PostProcessProfile, ProfileValue};

/// 一份天气的序列化 `MysekaiFogVolume` 值（从提取的
/// `postprocess.json` 转写，不在测试时解析）。
struct SerializedFog {
    weather: &'static str,
    density: f32,
    start: f32,
    end: f32,
    height: f32,
    near_color: [f32; 4],
    near_density: f32,
    far_color: [f32; 4],
    far_density: f32,
}

/// 15 份提取天气档案（`001_sunny`..`999_festivalgarden`）。下面的值
/// 全是序列化资产值；为什么是字面量而不是运行时读取，见模块 doc。
const SERIALIZED: &[SerializedFog] = &[
    SerializedFog {
        weather: "001_sunny",
        density: 1.0, start: 10.0, end: 30.0, height: 15.0,
        near_color: [0.0, 0.0, 0.0, 0.0], near_density: 0.0,
        far_color: [0.3254716992378235, 0.7585550546646118, 1.0, 1.0], far_density: 0.5,
    },
    SerializedFog {
        weather: "002_evening",
        density: 0.6000000238418579, start: 10.0, end: 30.0, height: 10.0,
        near_color: [1.0, 0.8025323152542114, 0.2783018946647644, 0.0], near_density: 0.5,
        far_color: [0.9811320900917053, 0.2535712718963623, 0.18974722921848297, 0.0],
        far_density: 0.5600000023841858,
    },
    SerializedFog {
        weather: "003_night",
        density: 0.5, start: 20.0, end: 50.0, height: 5.0,
        near_color: [0.0038093572948127985, 0.0, 0.6320754289627075, 0.0], near_density: 2.0,
        far_color: [0.09825561195611954, 0.17212681472301483, 0.30188679695129395, 0.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "004_fine",
        density: 1.0, start: 15.0, end: 30.0, height: 15.0,
        near_color: [0.36320751905441284, 0.7940084934234619, 1.0, 0.0], near_density: 1.0,
        far_color: [0.1933962106704712, 0.5887903571128845, 1.0, 0.0],
        far_density: 0.4000000059604645,
    },
    SerializedFog {
        weather: "005_fullmoon",
        density: 0.5, start: 5.0, end: 45.0, height: 30.0,
        near_color: [0.663999617099762, 0.5796546339988708, 0.7358490228652954, 0.0],
        near_density: 1.5,
        far_color: [0.05420079454779625, 0.2011111080646515, 0.3962264060974121, 1.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "006_rain",
        density: 0.800000011920929, start: 5.0, end: 40.0, height: 20.0,
        near_color: [0.21715910732746124, 0.7547169923782349, 0.7242892384529114, 0.0],
        near_density: 2.0,
        far_color: [0.4159843623638153, 0.4354034960269928, 0.7169811725616455, 0.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "007_rainnight",
        density: 0.800000011920929, start: 2.0, end: 40.0, height: 30.0,
        near_color: [0.25, 0.9327033758163452, 1.0, 0.0], near_density: 2.0,
        far_color: [0.16645820438861847, 0.08664116263389587, 0.5566037893295288, 1.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "008_thunder",
        density: 0.5, start: 5.0, end: 30.0, height: 30.0,
        near_color: [0.46952304244041443, 0.3524830639362335, 0.8396226167678833, 0.0],
        near_density: 0.5,
        far_color: [0.19811320304870605, 0.043921321630477905, 0.19811320304870605, 0.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "009_meteorshower",
        density: 0.6000000238418579, start: 5.0, end: 20.0, height: 30.0,
        near_color: [0.6135053634643555, 0.26032397150993347, 0.849056601524353, 0.0],
        near_density: 2.0,
        far_color: [0.09371664375066757, 0.09849496185779572, 0.5094339847564697, 0.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "010_snow",
        density: 0.46700000762939453, start: 1.0, end: 30.0, height: 15.0,
        near_color: [0.3254716992378235, 0.7127814292907715, 1.0, 0.0], near_density: 1.0,
        far_color: [0.6462264060974121, 0.814289391040802, 1.0, 0.0], far_density: 1.5,
    },
    SerializedFog {
        weather: "011_snownight",
        density: 1.0, start: 2.0, end: 45.0, height: 30.0,
        near_color: [0.47641509771347046, 0.64871746301651, 1.0, 0.0], near_density: 2.0,
        far_color: [0.3507857024669647, 0.31176573038101196, 0.5849056243896484, 0.0],
        far_density: 1.0,
    },
    SerializedFog {
        weather: "014_sekai",
        density: 1.0, start: 15.0, end: 30.0, height: 1.5,
        near_color: [0.641064465045929, 0.8585600256919861, 0.9245283007621765, 0.0],
        near_density: 0.6499999761581421,
        far_color: [0.1745283007621765, 0.5407149791717529, 1.0, 0.0],
        far_density: 0.28999999165534973,
    },
    SerializedFog {
        weather: "015_cloud",
        density: 0.625, start: 15.0, end: 30.0, height: 8.300000190734863,
        near_color: [0.13176396489143372, 0.47869569063186646, 0.5943396091461182, 0.0],
        near_density: 1.5499999523162842,
        far_color: [0.4494927227497101, 0.6734676361083984, 0.8584905862808228, 0.0],
        far_density: 1.2000000476837158,
    },
    SerializedFog {
        weather: "017_rainbow",
        density: 0.5, start: 10.0, end: 30.0, height: 3.0,
        near_color: [0.1933962106704712, 0.510165810585022, 1.0, 0.0], near_density: 1.5,
        far_color: [0.5896226167678833, 0.9837150573730469, 1.0, 0.0], far_density: 1.0,
    },
    SerializedFog {
        weather: "999_festivalgarden",
        density: 0.625, start: 15.0, end: 30.0, height: 8.300000190734863,
        near_color: [0.03301888704299927, 1.0, 0.9185848832130432, 0.0],
        near_density: 1.5499999523162842,
        far_color: [0.4494927227497101, 0.6734676361083984, 0.8584905862808228, 0.0],
        far_density: 1.2000000476837158,
    },
];

/// `<MOLY_ASSET_ROOT>/phenomena`，env 未设则 `None`。
fn phenomena_dir() -> Option<PathBuf> {
    let root = std::env::var("MOLY_ASSET_ROOT").ok()?;
    Some(Path::new(&root).join("phenomena"))
}

#[test]
fn all_15_weather_fog_globals_equal_the_hand_formula_from_serialized_values() {
    let Some(dir) = phenomena_dir() else {
        eprintln!(
            "MOLY_ASSET_ROOT not set: 0 of the 15 weather profiles were compared, \
             no evidence about the fog parser."
        );
        return;
    };

    let mut compared = 0usize;
    for s in SERIALIZED {
        let path = dir.join(s.weather).join("postprocess.json");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
        let profile = PostProcessProfile::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{}: parse failed: {e}", s.weather));
        let resolved = profile
            .resolve()
            .unwrap_or_else(|e| panic!("{}: resolve failed: {e}", s.weather));
        let fog = resolved.fog.params;

        // 采纳字段必须等于序列化值：15 份档案的雾字段 overrideState
        // 全为 true，采纳在此恰是直通。
        assert_eq!(fog.density, s.density, "{}: density", s.weather);
        assert_eq!(fog.start, s.start, "{}: start", s.weather);
        assert_eq!(fog.end, s.end, "{}: end", s.weather);
        assert_eq!(fog.height, s.height, "{}: height", s.weather);
        assert_eq!(&fog.near_color, &s.near_color, "{}: nearColor", s.weather);
        assert_eq!(fog.near_density, s.near_density, "{}: nearDensity", s.weather);
        assert_eq!(&fog.far_color, &s.far_color, "{}: farColor", s.weather);
        assert_eq!(fog.far_density, s.far_density, "{}: farDensity", s.weather);
        assert!(resolved.fog.enabled, "{}: fog enabled", s.weather);

        // globals：按 ExecuteFog 转写在测试内手算——刻意不经被测实现。
        let expected = [
            // fog_params = (-1/(end-start), end/(end-start), 1/fogHeight, 0)
            [-1.0 / (s.end - s.start), s.end / (s.end - s.start), 1.0 / s.height, 0.0],
            // fog_near_color = (nearColor.rgb, nearDensity * density)
            [
                s.near_color[0],
                s.near_color[1],
                s.near_color[2],
                s.near_density * s.density,
            ],
            // fog_far_color = (farColor.rgb, farDensity * density)
            [
                s.far_color[0],
                s.far_color[1],
                s.far_color[2],
                s.far_density * s.density,
            ],
        ];
        let got = [
            fog.globals(false).fog_params,
            fog.globals(false).fog_near_color,
            fog.globals(false).fog_far_color,
        ];
        for g in 0..3 {
            for c in 0..4 {
                assert_eq!(
                    got[g][c], expected[g][c],
                    "{}: globals[{g}][{c}] (got {} expect {})",
                    s.weather, got[g][c], expected[g][c],
                );
            }
        }
        compared += 1;
    }
    assert_eq!(compared, 15, "all 15 extracted weather profiles must be compared");
}

/// 14 现象闭集（主表现象表：ids 1-11、14、15、17——12/13/16 不存在）
/// 与逐现象手工量得的轴状态。`999_festivalgarden` 是站点皮肤不是
/// 现象，不进任何计数（它自己的轴在下面另行解析核对）。
#[test]
fn uber_axis_census_matches_the_hand_measurement() {
    let Some(dir) = phenomena_dir() else {
        eprintln!(
            "MOLY_ASSET_ROOT not set: 0 profiles were resolved, \
             no evidence about the axis census."
        );
        return;
    };

    const CLOSED_SET: &[(&str, u32)] = &[
        ("001_sunny", 1),
        ("002_evening", 2),
        ("003_night", 3),
        ("004_fine", 4),
        ("005_fullmoon", 5),
        ("006_rain", 6),
        ("007_rainnight", 7),
        ("008_thunder", 8),
        ("009_meteorshower", 9),
        ("010_snow", 10),
        ("011_snownight", 11),
        ("014_sekai", 14),
        ("015_cloud", 15),
        ("017_rainbow", 17),
    ];
    let id = |weather: &str| CLOSED_SET.iter().find(|(w, _)| *w == weather).map(|(_, i)| *i);

    let mut bloom_on = 0;
    let mut diffusion_on = 0;
    let mut screen_on = 0;
    let mut sun_on = 0;
    let mut sun_on_ids = Vec::new();
    for (weather, _) in CLOSED_SET {
        let bytes = std::fs::read(dir.join(weather).join("postprocess.json"))
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        let resolved = PostProcessProfile::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{weather}: {e}"))
            .resolve()
            .unwrap_or_else(|e| panic!("{weather}: {e}"));

        if resolved.bloom_lq.enabled {
            bloom_on += 1;
        }
        if resolved.sky_diffusion.enabled {
            diffusion_on += 1;
        }
        if resolved.screen_flarepara.enabled {
            screen_on += 1;
        }
        if resolved.sun_flarepara.enabled {
            sun_on += 1;
            sun_on_ids.push(id(weather).unwrap());
        }

        // 灰度/畸变一对对任何天气都不是档案驱动，必须解析成类型化的
        // 恒关成员。
        assert_eq!(resolved.grayscale, NotProfileDriven);
        assert!(!NotProfileDriven::ENABLED);
    }

    assert_eq!(bloom_on, 14, "_BLOOM_LQ must be 14/14 across the closed set");
    assert_eq!(diffusion_on, 14, "_SKY_DIFFUSION must be 14/14 across the closed set");
    assert_eq!(screen_on, 13, "_SCREEN_FLAREPARA must be 13/14 across the closed set");
    assert_eq!(sun_on, 4, "_SUN_FLAREPARA must be 4/14 across the closed set");
    // 引擎原生调色的采纳门：闭集里 13 档非恒等（唯一恒等的是 id 1
    // SUNNY——它的五项全落构造默认）。这个数与「打包值普查」分离：后者
    // 量的是装箱结果，这里量的是**采纳门**本身。
    let mut grading_on = 0;
    for (weather, _) in CLOSED_SET {
        let bytes = std::fs::read(dir.join(weather).join("postprocess.json"))
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        let resolved = PostProcessProfile::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{weather}: {e}"))
            .resolve()
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        if resolved.color_grading.enabled {
            grading_on += 1;
        }
    }
    assert_eq!(
        grading_on, 13,
        "ColorAdjustments.IsActive must be 13/14 across the closed set (only 001_sunny is identity)"
    );

    assert_eq!(
        sun_on_ids,
        vec![2, 4, 5, 9],
        "the four sun-flare phenomena are id 2/4/5/9 -- id 6 and id 14 serialize \
         isSunFlareActive value=1 with overrideState=false and must stay OFF (the \
         adoption gate, not the value, decides)"
    );

    // 唯一屏幕光晕关着的是 id 1 SUNNY（强度 0.0 过不了 >0.001 阈）。
    let sunny = std::fs::read(dir.join("001_sunny").join("postprocess.json")).unwrap();
    let sunny = PostProcessProfile::from_bytes(&sunny)
        .unwrap()
        .resolve()
        .unwrap();
    assert!(!sunny.screen_flarepara.enabled);
    assert_eq!(
        sunny.screen_flarepara.params.intensity, 0.0,
        "sunny's serialized screenFlareIntensity, the reason the gate closes"
    );

    // 999_festivalgarden：站点皮肤。可解析、轴照常解析，但它不是第
    // 15 个现象，不进上面任何计数。
    let skin = std::fs::read(dir.join("999_festivalgarden").join("postprocess.json")).unwrap();
    let skin = PostProcessProfile::from_bytes(&skin)
        .unwrap()
        .resolve()
        .unwrap();
    assert!(skin.fog.enabled && skin.bloom_lq.enabled && skin.sky_diffusion.enabled);
    assert!(skin.screen_flarepara.enabled && !skin.sun_flarepara.enabled);
}

/// 采纳门负例立在真语料上：id 6 RAIN 与 id 14 SEKAI 的档案*确实*
/// 序列化了 `isSunFlareActive value=1` 且 `overrideState=false`——
/// 负例是数据不是虚构；解析必须让太阳轴 OFF。顺带钉住逐字段的
/// 采纳语义：id 6 的 `sunFlareIntensity` 自己的门开着，序列化的
/// 1.2300000190734863 照样被采纳（帧由 off 位保持，不靠清 lane）；
/// id 14 连强度的门也关着，落构造默认 1.0。
#[test]
fn rain_and_sekai_serialize_sun_flare_flag_with_the_gate_closed() {
    let Some(dir) = phenomena_dir() else {
        eprintln!(
            "MOLY_ASSET_ROOT not set: the adoption-gate negative was not checked \
             against the real profiles, no evidence."
        );
        return;
    };

    for weather in ["006_rain", "014_sekai"] {
        let bytes = std::fs::read(dir.join(weather).join("postprocess.json"))
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        let profile = PostProcessProfile::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{weather}: {e}"));

        // 序列化侧：旗标真是 value=1、门真是关的。
        let flare = profile
            .component("MysekaiFlareParaVolume")
            .unwrap_or_else(|| panic!("{weather}: no MysekaiFlareParaVolume"));
        let flag = flare
            .parameter("isSunFlareActive")
            .unwrap_or_else(|| panic!("{weather}: no isSunFlareActive"));
        assert!(!flag.override_state, "{weather}: the gate is serialized closed");
        assert_eq!(
            flag.value,
            ProfileValue::Scalar(1.0),
            "{weather}: the serialized value is 1 -- reading the value would light the flare"
        );

        let resolved = profile.resolve().unwrap_or_else(|e| panic!("{weather}: {e}"));
        assert!(
            !resolved.sun_flarepara.enabled,
            "{weather}: value=1 with the gate closed must not light the sun flare"
        );
    }

    // 逐字段采纳：id 6 强度字段自己的门开着，序列化值活着。
    let rain = std::fs::read(dir.join("006_rain").join("postprocess.json")).unwrap();
    let rain = PostProcessProfile::from_bytes(&rain).unwrap().resolve().unwrap();
    assert_eq!(
        rain.sun_flarepara.params.intensity, 1.2300000190734863,
        "rain's sunFlareIntensity has its own gate open: the serialized value is adopted"
    );

    // id 14 的强度门也关着：构造默认 1.0，不是序列化值之外的任何数。
    let sekai = std::fs::read(dir.join("014_sekai").join("postprocess.json")).unwrap();
    let sekai = PostProcessProfile::from_bytes(&sekai).unwrap().resolve().unwrap();
    assert_eq!(
        sekai.sun_flarepara.params.intensity, 1.0,
        "sekai's sunFlareIntensity is gate-closed too: the ctor default 1.0"
    );
}

/// 泛光 `clamp` 的构造默认在真语料上**可达**：8 档把它的采纳门写
/// 成关，门关时真游戏取构造默认 `0x477fc000` = 65472.0（不是此前写
/// 错的 65520）。这一档的判别式：把档案里那些「门关、值 65472」的
/// 序列化改成任何别的数，真游戏仍取 65472 ⇒ 这里的断言不会红；但把
/// 构造默认改回 65520，下面两档就会红——它钉的是**构造默认本身**。
#[test]
fn bloom_clamp_ctor_default_is_reachable_and_is_65472() {
    let Some(dir) = phenomena_dir() else {
        eprintln!("MOLY_ASSET_ROOT not set: 0 profiles were resolved.");
        return;
    };
    let mut gate_closed = 0;
    let mut gate_open = 0;
    for d in std::fs::read_dir(&dir).unwrap() {
        let d = d.unwrap().file_name();
        let d = d.to_str().unwrap();
        if !d.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let bytes = std::fs::read(dir.join(d).join("postprocess.json")).unwrap();
        let r = PostProcessProfile::from_bytes(&bytes)
            .unwrap()
            .resolve()
            .unwrap();
        let bloom = &r.bloom_lq.params;
        // 采纳后的值：门关 ⇒ 构造默认，门开 ⇒ 序列化值。两边都是 65472。
        assert_eq!(bloom.clamp, 65472.0, "{d}: adopted clamp");
        let comp = PostProcessProfile::from_bytes(&bytes)
            .unwrap();
        let comp = comp
            .components
            .iter()
            .find(|c| c.class == "MysekaiParticleBloomVolume")
            .unwrap()
            .parameter("clamp")
            .unwrap();
        if comp.override_state {
            gate_open += 1;
        } else {
            gate_closed += 1;
        }
    }
    assert_eq!(gate_closed, 8, "8 profiles close the clamp gate (ctor default reachable)");
    assert_eq!(gate_open, 7, "7 profiles serialize clamp explicitly");
}

/// 引擎原生调色（`ColorAdjustments`）在真语料上的逐项核对：15 档的
/// 采纳值与手工从档案读出的数逐项相等，且 14 档非恒等（唯一恒等的是
/// 001_sunny——它的五项全落构造默认）。这一条是「产品里 14 档逐档不同
/// 的调色没进像素」这一缺的**反面**：现在它进了，且数对得上。
#[test]
fn color_adjustments_pack_matches_hand_measurement() {
    let Some(dir) = phenomena_dir() else {
        eprintln!("MOLY_ASSET_ROOT not set: 0 profiles were resolved.");
        return;
    };
    // 手工从 15 档档案读出的采纳值（EV, 色相度, 饱和百分, 对比百分,
    // 滤色 RGB gamma）。001_sunny 全落构造默认（恒等）。
    let hand: &[(&str, f32, f32, f32, f32, [f32; 3], bool)] = &[
        ("001_sunny", 0.0, 0.0, 0.0, 0.0, [1.0, 1.0, 1.0], false),
        ("002_evening", -0.15, 0.0, 0.0, 15.0, [1.0, 1.0, 1.0], true),
        ("003_night", -0.4, 0.0, -17.5, 0.0, [1.0, 1.0, 1.0], true),
        ("004_fine", -1.6, 0.0, 1.0, 1.0, [1.7104409, 1.6727897, 1.653964], true),
        ("005_fullmoon", 1.0, 0.0, -25.0, 5.0, [0.6525454, 0.7177411, 0.9811321], true),
        ("006_rain", 0.2, 0.0, -17.6, 1.0, [0.8443396, 0.8754528, 1.0], true),
        ("007_rainnight", -0.4, 0.0, -15.0, 10.0, [0.6258770, 0.5537572, 0.9245283], true),
        ("008_thunder", -0.8, 0.0, 10.0, -8.0, [0.7865169, 0.8934156, 1.3781321], true),
        ("009_meteorshower", 0.5, 0.0, 4.0, 1.0, [0.8469433, 0.9019775, 0.9811321], true),
        ("010_snow", 0.3, 0.0, -4.0, 10.0, [0.7877358, 0.9143497, 1.0], true),
        ("011_snownight", -1.0, 0.0, -10.0, -5.0, [0.75, 0.8846154, 1.0], true),
        ("014_sekai", -0.16, 0.0, -3.0, 10.0, [0.9386792, 0.9590566, 1.0], true),
        ("015_cloud", -0.3, 0.0, -15.0, 20.0, [0.7877358, 0.8134528, 1.0], true),
        ("017_rainbow", 0.05, 0.0, 5.0, 10.0, [1.0, 0.9834355, 0.9058824], true),
        ("999_festivalgarden", 0.0, 0.0, -15.0, 20.0, [0.7877358, 0.8134528, 1.0], true),
    ];
    assert_eq!(hand.len(), 15, "hand table must cover all 15 profiles");
    for (weather, pe, hue, sat, con, filt, on) in hand {
        let bytes = std::fs::read(dir.join(weather).join("postprocess.json"))
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        let r = PostProcessProfile::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("{weather}: {e}"))
            .resolve()
            .unwrap_or_else(|e| panic!("{weather}: {e}"));
        let g = &r.color_grading;
        assert_eq!(g.enabled, *on, "{weather}: IsActive gate");
        // 采纳门：门关时落构造默认（001_sunny 就是这条路径）。
        if !on {
            continue;
        }
        let p = g.params;
        assert!(
            (p.post_exposure - pe).abs() < 1e-6,
            "{weather}: postExposure {pe} vs {}",
            p.post_exposure
        );
        assert!((p.hue_shift - hue).abs() < 1e-4, "{weather}: hueShift {hue} vs {}", p.hue_shift);
        assert!((p.saturation - sat).abs() < 1e-4, "{weather}: saturation {sat} vs {}", p.saturation);
        assert!((p.contrast - con).abs() < 1e-4, "{weather}: contrast {con} vs {}", p.contrast);
        for ch in 0..3 {
            assert!(
                (p.color_filter[ch] - filt[ch]).abs() < 1e-4,
                "{weather}: colorFilter[{ch}] {} vs {}",
                filt[ch],
                p.color_filter[ch]
            );
        }
        // 打包一跳：曝光倍数与两个系数逐项照引擎的式子。
        let pack = p.pack();
        let exp_linear = 2.0f32.powf(*pe);
        assert!(
            (pack.post_exposure_linear - exp_linear).abs() < 1e-6,
            "{weather}: 2^EV {exp_linear} vs {}",
            pack.post_exposure_linear
        );
        assert!(
            (pack.hue_sat_con[0] - hue / 360.0).abs() < 1e-6,
            "{weather}: hue/360"
        );
        assert!(
            (pack.hue_sat_con[1] - (sat / 100.0 + 1.0)).abs() < 1e-6,
            "{weather}: sat/100+1"
        );
        assert!(
            (pack.hue_sat_con[2] - (con / 100.0 + 1.0)).abs() < 1e-6,
            "{weather}: con/100+1"
        );
    }
}
