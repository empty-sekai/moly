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
    // Fixed entropy: the warm-cost measurement needs owners, not a live draw.
    let mut warm_seeds = crate::particle_runtime::seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
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
                let warm = planned.as_ref().and_then(|p| first_play_warm_cost(p, &mut warm_seeds));
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
                    "sourceRoute": format!("{:?}", crate::particle_runtime::source_route(&particle["system"])),
                    "firstPlayWarm": warm,
                    "nativeBirth": planned.as_ref().map(|p| crate::particle_runtime::native_birth_eligible(&p.emitter, &p.route)
                        .and_then(|()| crate::particle_runtime::native_shape_state_eligible(&p.emitter, Some(p.geometry.shape_evidence())))
                        .map_or_else(|reason| json!({"path":"legacy","reason":reason}), |()| json!({"path":"native"}))),
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

/// First-Play warm of one admitted system on the path production would take,
/// timed on this machine. Mesh-surface emitters need a loaded surface and are
/// reported as unmeasured rather than warmed without it.
fn first_play_warm_cost(planned: &Planned, seeds: &mut crate::particle_runtime::seed::SystemSeedManager) -> Option<Value> {
    let e = &planned.emitter;
    if !(e.prewarm && e.looping) { return None; }
    if planned.emission_surface.is_some() {
        return Some(json!({"measured": false, "reason": "mesh emission surface not loaded in the audit"}));
    }
    let mut system = crate::particle_runtime::test_support::runtime();
    system.pool.clear();
    system.side.clear();
    system.born_total = 0;
    system.node = planned.node.clone();
    system.effect = planned.effect.clone();
    system.kind = planned.kind;
    system.camera_rotation = planned.camera_rotation;
    system.node_affine = planned.node_affine;
    // The warm never draws. The planned render mode and scaling are what the
    // birth path reads; an empty mesh stands in for an unloaded source GLB.
    system.geometry = match &planned.geometry {
        PlannedGeometry::Billboard(draw) => crate::particle_runtime::Geometry::SourceBillboard(draw.clone()),
        PlannedGeometry::Mesh { alignment, scaling, pivot, .. } => crate::particle_runtime::Geometry::Mesh(
            crate::particle_geometry::MeshDraw {
                source: Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
                    uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), bounds_size: Vec3::ZERO }),
                alignment: *alignment, scaling: *scaling, pivot: *pivot,
            }),
    };
    system.emitter = e.clone();
    system.cone_angle = planned.cone_angle;
    system.rol = planned.rol.clone();
    system.limit = planned.limit.clone();
    system.velocity_law = e.velocity_over_lifetime.as_ref().map(moly_law::particle::velocity::VelocityOverLifetime::from_params);
    system.force_law = e.force.as_ref().map(|p| moly_law::particle::force::ForceOverLifetime::from_params(p).expect("force validated during admission"));
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&e.start.gravity_modifier);
    system.size_law = e.size_over_lifetime.as_ref().map(moly_law::particle::size::SizeOverLifetime::from_params);
    system.color_law = e.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = e.custom_data.as_ref().map(moly_law::particle::custom_data::CustomData::from_params);
    let path = match crate::particle_runtime::install_native_birth(&mut system, seeds, &planned.route) {
        Ok(crate::particle_runtime::BirthPath::Native) => "native",
        Ok(crate::particle_runtime::BirthPath::Legacy(_)) => "legacy",
        Err(error) => return Some(json!({"measured": false, "reason": error.to_string()})),
    };
    let ctx = crate::particle_runtime::Context {
        sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY,
    };
    let started = std::time::Instant::now();
    let result = crate::particle_runtime::prewarm_first_play(&mut system, &ctx);
    let micros = started.elapsed().as_micros() as u64;
    Some(json!({"measured": true, "path": path, "micros": micros, "alive": system.pool.len(),
        "born": system.born_total, "refused": result.err()}))
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
fn noise_module_needs_its_authored_block_and_takes_the_ordinary_route() {
    let mut system = json!({"sourceModules":{"version":1,
        "enabled":["InitialModule","NoiseModule"],"unsupported":[]},
        "ringBufferMode":0,"prewarm":true,
        "start":{"randomizeRotationDirection":0}});
    let refusal = source_simulation_admission(&system).unwrap_err();
    assert!(refusal.contains("NoiseModule"), "{refusal}");
    system["noise"] = json!({"dimensions":3,"quality":"high","octaves":1,
        "separateAxes":false,"remapEnabled":false});
    assert!(source_simulation_admission(&system).is_ok());
}

#[test]
fn route_controls_each_move_a_procedural_system_to_the_ordinary_route() {
    use crate::particle_runtime::{source_route, SourceRoute};
    // Every control the native decision reads, set to its procedural value.
    let procedural = json!({"sourceModules":{"version":1,"enabled":["InitialModule"],"unsupported":[]},
        "simulationSpace":"Local","stopAction":0,
        "emission":{"rateOverDistance":{"mode":"constant","value":0.0}},
        "start":{"gravityModifier":{"mode":"constant","value":0.0},
            "lifetime":{"mode":"constant","value":1.0}}});
    assert_eq!(source_route(&procedural), SourceRoute::Procedural);
    for module in ["NoiseModule", "SubModule", "CollisionModule", "TriggerModule",
        "ClampVelocityModule", "ExternalForcesModule", "RotationBySpeedModule"] {
        let mut system = procedural.clone();
        system["sourceModules"]["enabled"].as_array_mut().unwrap().push(json!(module));
        assert_eq!(source_route(&system), SourceRoute::Ordinary, "{module}");
    }
    // The decision does not read the Lights module.
    let mut lights = procedural.clone();
    lights["sourceModules"]["enabled"].as_array_mut().unwrap().push(json!("LightsModule"));
    assert_eq!(source_route(&lights), SourceRoute::Procedural);
    // Only a per-particle trail fails; ribbons pass; an unexported mode is undecided.
    let mut trail = procedural.clone();
    trail["sourceModules"]["enabled"].as_array_mut().unwrap().push(json!("TrailModule"));
    assert!(matches!(source_route(&trail), SourceRoute::Undecided(_)));
    trail["trails"] = json!({"mode":"perParticle"});
    assert_eq!(source_route(&trail), SourceRoute::Ordinary);
    trail["trails"] = json!({"mode":"ribbon"});
    assert_eq!(source_route(&trail), SourceRoute::Procedural);
    // Gravity and lifetime are read only while Initial is enabled.
    let mut no_initial = procedural.clone();
    no_initial["sourceModules"]["enabled"] = json!([]);
    no_initial["start"]["lifetime"] = json!({"mode":"constant","value":"Infinity"});
    assert_eq!(source_route(&no_initial), SourceRoute::Procedural);
    let mut world = procedural.clone();
    world["simulationSpace"] = json!("World");
    assert_eq!(source_route(&world), SourceRoute::Ordinary);
    let mut infinite = procedural.clone();
    infinite["start"]["lifetime"] = json!({"mode":"constant","value":"Infinity"});
    assert_eq!(source_route(&infinite), SourceRoute::Ordinary);
    let mut distance = procedural.clone();
    distance["emission"]["rateOverDistance"]["value"] = json!(1.0);
    assert_eq!(source_route(&distance), SourceRoute::Ordinary);
    // A disabled Emission module has no block; the system-level copy decides.
    let mut disabled = procedural.clone();
    disabled.as_object_mut().unwrap().remove("emission");
    assert!(matches!(source_route(&disabled), SourceRoute::Undecided(_)));
    disabled["emissionRateOverDistance"] = json!({"mode":"constant","value":0.0});
    assert_eq!(source_route(&disabled), SourceRoute::Procedural);
    disabled["emissionRateOverDistance"]["value"] = json!(2.0);
    assert_eq!(source_route(&disabled), SourceRoute::Ordinary);
    // A velocity curve that does not span [0, 1] depends on its wrap mode.
    let mut wrapped = procedural.clone();
    wrapped["sourceModules"]["enabled"].as_array_mut().unwrap().push(json!("VelocityModule"));
    let axis = json!({"mode":"curve","multiplier":1.0,"keys":[{"time":0.0,"value":0.0},{"time":0.5,"value":1.0}]});
    let zero = json!({"mode":"constant","value":0.0});
    wrapped["velocityOverLifetime"] = json!({"x":zero,"y":axis,"z":zero,
        "orbitalX":zero,"orbitalY":zero,"orbitalZ":zero,"radial":zero});
    assert!(matches!(source_route(&wrapped), SourceRoute::Undecided(_)));
    // A clamped post-wrap (2 or more) makes the open end valid; a lower one fails.
    wrapped["velocityOverLifetime"]["y"]["postInfinity"] = json!(2);
    assert_eq!(source_route(&wrapped), SourceRoute::Procedural);
    wrapped["velocityOverLifetime"]["y"]["postInfinity"] = json!(1);
    assert_eq!(source_route(&wrapped), SourceRoute::Ordinary);
    wrapped["velocityOverLifetime"]["y"]["postInfinity"] = json!(2);
    // Eight keys spanning [0, 0.5] plus one clamped end reach nine segments.
    let keys: Vec<_> = (0..9).map(|i| json!({"time": i as f64 / 16.0, "value": 0.0})).collect();
    wrapped["velocityOverLifetime"]["y"]["keys"] = json!(keys);
    assert_eq!(source_route(&wrapped), SourceRoute::Ordinary);
    wrapped["velocityOverLifetime"]["y"]["keys"] = json!([{"time":0.0,"value":0.0},{"time":0.5,"value":1.0}]);
    wrapped["velocityOverLifetime"]["orbitalY"] = json!({"mode":"constant","value":0.05});
    assert_eq!(source_route(&wrapped), SourceRoute::Ordinary, "a failing control beats an undecided one");
    // Missing inputs are undecided, never a default.
    let mut missing = procedural.clone();
    missing.as_object_mut().unwrap().remove("simulationSpace");
    assert!(matches!(source_route(&missing), SourceRoute::Undecided(_)));
}

#[test]
fn preserved_infinite_burst_interval_does_not_enable_unverified_scheduling() {
    let mut system = json!({"sourceModules":{"version":1,
        "enabled":["EmissionModule","InitialModule"],"unsupported":[]},
        "ringBufferMode":0,"start":{"randomizeRotationDirection":0},
        "emission":{"bursts":[{"cycleCount":0,"repeatInterval":"Infinity"}]}});
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
