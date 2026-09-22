//! Opt-in diagnostic over a caller-supplied extraction. This invokes the real
//! runtime admission path; its output is coverage, NOT a source correctness test.
//! No game data, private paths or fixed phenomenon counts are embedded here.
use super::*;
use bevy::asset::AssetPlugin;
use bevy::image::ImagePlugin;
use serde_json::json;

#[test]
#[ignore = "requires MOLY_WEATHER_AUDIT_INDEX and MOLY_WEATHER_AUDIT_OUT"]
fn current_corpus_admission() {
    let index_path = std::path::PathBuf::from(
        std::env::var_os("MOLY_WEATHER_AUDIT_INDEX").expect("supply extraction index"),
    );
    let output = std::path::PathBuf::from(
        std::env::var_os("MOLY_WEATHER_AUDIT_OUT").expect("supply diagnostic output"),
    );
    let root = index_path.parent().expect("index has a parent");
    let overlay = std::env::var_os("MOLY_SOURCE_CORPUS_OVERLAY").map(std::path::PathBuf::from);
    let read = |path: &std::path::Path| -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("read source extraction"))
            .expect("parse source extraction")
    };
    let index = read(&index_path);
    let mut app = App::new();
    moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir {
        path: root.parent().expect("phenomena directory has a parent").to_path_buf(),
    });
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ImagePlugin::default()));
    moly_assets::source_shader::loader::register(&mut app);
    app.init_asset::<Gltf>();
    app.finish();
    app.cleanup();
    let server = app.world().resource::<AssetServer>();
    let mut rows = Vec::new();
    let mut per_phenomenon = serde_json::Map::new();
    for (name, item) in index["phenomena"].as_object().expect("phenomena map") {
        let source_path = |file: &str| overlay.as_ref().map(|root|root.join(file)).filter(|path|path.is_file()).unwrap_or_else(||root.join(file));
        let path = source_path(item["fx"]["file"].as_str().expect("fx file"));
        let doc = read(&path);
        let animation_doc = item["animations"]["file"].as_str().map(|file|read(&source_path(file)))
            .or_else(||overlay.as_ref().map(|root|root.join(name).join("animations.json")).filter(|path|path.is_file()).map(|path|read(&path)));
        let start = rows.len();
        let mut admitted = 0;
        let mut renderer_enabled = 0;
        for (effect_name, effect) in doc["effects"].as_object().expect("effects map") {
            let animation = crate::weather_animation::Contract::compile(effect,animation_doc.as_ref());
            let kind = match effect["kind"].as_str() {
                Some("sky") => Some(EffectKind::Sky),
                Some("camera") => Some(EffectKind::Camera),
                Some("site") => Some(EffectKind::Site),
                // plan() only selects sky/camera/site. Keep other exported
                // prefabs in the census, but do not invent a runtime anchor.
                Some("other") => None,
                other => panic!("unclassified effect kind {other:?}; do not silently omit"),
            };
            let by_path: HashMap<String, &Value> = effect["nodes"].as_array().expect("nodes")
                .iter().map(|n| (n["path"].as_str().expect("node path").to_owned(), n)).collect();
            let particles = effect["particles"].as_array().expect("particles");
            let sub_emitter_owners = source_sub_emitter_owners(particles);
            for particle in particles {
                let mut tally = Tally::default();
                let animation_refusal = particle["node"].as_str().and_then(|node|animation.refusal(node));
                if let Some(reason) = animation_refusal { tally.animation_refused.push(reason.into()); }
                let planned = kind.filter(|_|animation_refusal.is_none()).and_then(|kind| judge(effect_name, particle, &by_path, &sub_emitter_owners, kind,
                    effect["effectiveRotation"].as_str() == Some("normal"),
                    WeatherEffectLifecycle::from_effect(effect).expect("source lifecycle metadata"), server, &mut tally));
                let material = &particle["renderer"]["material"];
                let source_member = material["lightModes"].as_array()
                    .map(|tags| tags.iter().any(|tag| tag.as_str() == Some("MysekaiEffect")));
                // Admission only requests assets. GPU pass resolution happens later
                // in the renderer, so this diagnostic must not claim a tested route.
                let node = particle["node"].as_str().expect("particle node");
                let active = active_in_hierarchy(&by_path, node);
                let classification = if particle["renderer"]["enabled"] == false {
                    "source_renderer_disabled"
                } else if !active {
                    "source_hierarchy_inactive"
                } else if kind.is_none() {
                    "runtime_anchor_unresolved"
                } else if planned.is_some() {
                    "admitted_pending_gpu_and_behavior_verification"
                } else {
                    // Zero autonomous emission is not proof of invisibility:
                    // subemitters, animation and timeline can trigger emission.
                    "runtime_admission_rejected"
                };
                admitted += usize::from(planned.is_some());
                renderer_enabled += usize::from(particle["renderer"]["enabled"].as_bool() == Some(true));
                rows.push(json!({
                    "phenomenon": name, "effect": effect_name, "node": particle["node"],
                    "kind": effect["kind"], "variant": effect["variant"],
                    "site": effect["site"], "classification": classification,
                    "activeInHierarchy": active,
                    "rendererEnabled": particle["renderer"]["enabled"],
                    "renderMode": particle["renderer"]["renderMode"],
                    "shader": material["shader"], "lightModes": material["lightModes"],
                    "admitted": planned.is_some(), "sourceEffectPassDeclared": source_member,
                    "gpuVerification": "not_run", "gates": format!("{tally:?}"),
                    "animationRefusal":animation_refusal,"animationContract":animation.report,
                    "softKeyword": material["keywords"].as_array().is_some_and(|v|
                        v.iter().any(|k| k.as_str() == Some("_SOFT_PARTICLES_ENABLED"))),
                }));
            }
        }
        let total = rows.len() - start;
        println!("{name}: records={total}, renderer_enabled={renderer_enabled}, admitted={admitted}");
        per_phenomenon.insert(name.clone(), json!({
            "records": total, "rendererEnabled": renderer_enabled,
            "admitted": admitted,
        }));
    }
    let report = json!({
        "meaning": "Observed runtime admission over all exported effect variants, not simultaneous scene population and not proof of source equivalence.",
        "perPhenomenon": per_phenomenon,
        "records": rows.len(),
        "admitted": rows.iter().filter(|r| r["admitted"] == true).count(),
        "gpuVerification": "not_run",
        "rows": rows,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write audit report");
}

#[test]
fn enabled_unimplemented_module_is_never_a_partially_simulated_emitter() {
    let clean = json!({"sourceModules":{"version":1,"enabled":["InitialModule"],"unsupported":[]},
        "ringBufferMode":0,"start":{"randomizeRotationDirection":0}});
    assert!(source_simulation_admission(&clean).is_ok());
    for module in ["NoiseModule", "CollisionModule", "TrailModule", "SubModule", "InheritVelocityModule"] {
        let mut system = clean.clone();
        let mut enabled = vec!["InitialModule", module]; enabled.sort();
        system["sourceModules"]["enabled"] = json!(enabled);
        let error = source_simulation_admission(&system).unwrap_err();
        assert!(error.contains(module), "{error}");
    }
    let mut system = clean.clone();
    system["sourceModules"]["enabled"] = json!(["ColorModule", "InitialModule"]);
    assert!(source_simulation_admission(&system).unwrap_err().contains("colorOverLifetime"));
    for mode in [1, 2] {
        let mut system = clean.clone(); system["ringBufferMode"] = json!(mode);
        assert!(source_simulation_admission(&system).is_err());
    }
    let mut system = clean;
    system["start"]["randomizeRotationDirection"] = json!(1.0);
    assert!(source_simulation_admission(&system).is_err());
}

#[test]
fn authored_null_submodule_edge_has_no_child_owner_obligation() {
    let mut system = json!({"sourceModules":{"version":1,
        "enabled":["InitialModule", "SubModule"],"unsupported":[]},
        "ringBufferMode":0,"start":{"randomizeRotationDirection":0},
        "subEmitters":[{"emitter":null,"type":"birth",
            "sourcePointer":{"fileId":0,"pathId":"0"}}]});
    assert!(source_simulation_admission(&system).is_ok());
    for pointer in [Value::Null, json!({"fileId":1,"pathId":"0"}),
        json!({"fileId":0,"pathId":"8536188417864660738"})] {
        let mut unresolved = system.clone();
        unresolved["subEmitters"][0]["sourcePointer"] = pointer;
        assert!(source_simulation_admission(&unresolved).is_err(), "unresolved is not authored null");
    }
    system["subEmitters"][0]["emitter"] = json!("root/child");
    assert!(source_simulation_admission(&system).is_err());
    system["subEmitters"][0].as_object_mut().unwrap().remove("emitter");
    assert!(source_simulation_admission(&system).is_err());
}

#[test]
fn noise_module_parameters_do_not_qualify_full_birth_prewarm_composition() {
    let system = json!({"sourceModules":{"version":1,
        "enabled":["InitialModule","NoiseModule"],"unsupported":[]},
        "ringBufferMode":0,"prewarm":true,
        "start":{"randomizeRotationDirection":0},
        "noise":{"dimensions":3,"quality":"high","octaves":1,
            "separateAxes":false,"remapEnabled":false}});
    let refusal = source_simulation_admission(&system).unwrap_err();
    assert!(refusal.contains("NoiseModule") && refusal.contains("composition"), "{refusal}");
}

#[test]
fn preserved_infinite_burst_interval_does_not_enable_unverified_scheduling() {
    let mut system = json!({"sourceModules":{"version":1,
        "enabled":["EmissionModule","InitialModule"],"unsupported":[]},
        "ringBufferMode":0,"start":{"randomizeRotationDirection":0},
        "emission":{"bursts":[{"cycleCount":0,"repeatInterval":"Infinity"}]}});
    assert!(source_simulation_admission(&system).unwrap_err().contains("infinite burst"));
    system["emission"]["bursts"][0]["repeatInterval"] = json!(1.0);
    assert!(source_simulation_admission(&system).is_ok());
}

#[test]
fn source_sub_emitter_targets_keep_their_event_owners() {
    let particles = vec![json!({"node":"root/a","system":{"subEmitters":[{"emitter":"root/b"},{"emitter":"root/c"}]}}),
        json!({"node":"root/b","system":{}}), json!({"node":"root/c","system":{"subEmitters":[{"emitter":"root/b"}]}})];
    let owners = source_sub_emitter_owners(&particles);
    assert_eq!(owners["root/b"], vec!["root/a", "root/c"]);
    assert_eq!(owners["root/c"], vec!["root/a"]);
    assert!(!owners.contains_key("root/a"));
}
