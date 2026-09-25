//! Opt-in diagnostic over a caller-supplied extraction. This invokes the real
//! runtime admission path; its output is coverage, NOT a source correctness test.
//! No game data, private paths or fixed phenomenon counts are embedded here.
use super::*;
use bevy::asset::AssetPlugin;
use bevy::image::ImagePlugin;
use serde_json::json;

///
/// Instantiation follows the production selection (`source_environment_selection`) over every site of the
/// environment prefab census (MOLY_ENVIRONMENT_PREFAB_CENSUS), as the census replay does: a row is a runtime
/// row exactly when the loader picks its prefab for at least one site, with the kind the loader gives it.
#[test]
#[ignore = "requires MOLY_WEATHER_AUDIT_INDEX, MOLY_WEATHER_AUDIT_OUT and MOLY_ENVIRONMENT_PREFAB_CENSUS"]
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
    let census = read(std::path::Path::new(
        &std::env::var_os("MOLY_ENVIRONMENT_PREFAB_CENSUS").expect("supply the environment prefab census for its sites"),
    ));
    let sites: Vec<String> = census["sites"].as_array().expect("census sites").iter()
        .map(|site| site.as_str().unwrap_or_else(|| panic!("census site is not a string: {site}")).to_owned())
        .collect();
    assert!(!sites.is_empty(), "the census names no site");
    let mut naming_role_differs = Vec::new();
    // The collider export: the one the caller names, else the one the index
    // names; without either every collision system is refused by name.
    let collision_path = std::env::var_os("MOLY_WEATHER_AUDIT_COLLISION").map(std::path::PathBuf::from)
        .or_else(|| index.pointer("/collision/file").and_then(Value::as_str).map(|file| root.join(file)));
    let collision_document = match &collision_path {
        Some(path) => Ok(read(path)),
        None => Err("this asset root carries no collider export".to_owned()),
    };
    let collision_source = collision_path.as_ref().map(|path| {
        let bytes = std::fs::read(path).expect("read collider export");
        json!({"bytes": bytes.len()})
    });
    let mut scenes = crate::particle_runtime::collision_scene::SceneBuilder::new(collision_document);
    let app = corpus_app(root);
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
        let effects = doc["effects"].as_object().expect("effects map");
        // The loader's pick for every census site; a prefab has one kind whatever the site.
        let mut selected: HashMap<String, EffectKind> = HashMap::new();
        for site in &sites {
            for (prefab, _, kind) in source_environment_selection(effects, name, site) {
                let previous = selected.insert(prefab.clone(), kind);
                assert!(previous.is_none_or(|previous| previous == kind), "{name}/{prefab}: two kinds across sites");
            }
        }
        for (effect_name, effect) in effects {
            let animation = crate::weather_animation::Contract::compile(effect,animation_doc.as_ref());
            // The extractor's name-based kind is only an extraction contract check here.
            match effect["kind"].as_str() {
                Some("sky" | "camera" | "site" | "other") => {}
                other => panic!("unclassified effect kind {other:?}; do not silently omit"),
            }
            // The anchor is the loader's pick: a prefab the loader never instantiates on its own (a template
            // whose baked copies live inside a unique site prefab) stays in the census but is not a runtime row.
            let variant = effect["variant"].as_str()
                .unwrap_or_else(|| panic!("{name}/{effect_name}: package variant is not a string"));
            let kind = selected.get(effect_name.as_str()).copied();
            if kind != source_environment_role(effect_name, variant, name) {
                naming_role_differs.push(format!("{name}/{effect_name}"));
            }
            let by_path: HashMap<String, &Value> = effect["nodes"].as_array().expect("nodes")
                .iter().map(|n| (n["path"].as_str().expect("node path").to_owned(), n)).collect();
            let particles = effect["particles"].as_array().expect("particles");
            let sub_emitter_owners = source_sub_emitter_owners(particles);
            let ground = scenes.for_effect(effect_name);
            // Targets admitted by their own judgement and the birth edges of
            // every admitted parent of this effect: a target whose parent is
            // refused outside judge (its animation contract) is withdrawn below,
            // as the plan withdraws it.
            let mut target_rows: Vec<(usize, String)> = Vec::new();
            let mut delivered = std::collections::HashSet::<String>::new();
            for particle in particles {
                let mut tally = Tally::default();
                let animation_refusal = particle["node"].as_str().and_then(|node|animation.refusal(node));
                if let Some(reason) = animation_refusal { tally.animation_refused.push(reason.into()); }
                let planned = kind.filter(|_|animation_refusal.is_none()).and_then(|kind| judge(effect_name, particle, &by_path, &sub_emitter_owners, kind,
                    effect["effectiveRotation"].as_str() == Some("normal"),
                    WeatherEffectLifecycle::from_effect(effect).expect("source lifecycle metadata"), &ground, server, &mut tally))
                    .and_then(|planned| admit_animated(&animation, planned, &mut tally));
                let material = &particle["renderer"]["material"];
                let source_member = material["lightModes"].as_array()
                    .map(|tags| tags.iter().any(|tag| tag.as_str() == Some("MysekaiEffect")));
                // Admission only requests assets. GPU pass resolution happens later
                // in the renderer, so this diagnostic must not claim a tested route.
                let node = particle["node"].as_str().expect("particle node");
                let active = active_in_hierarchy(&by_path, node);
                let classification = if kind.is_none() {
                    // Never instantiated on its own by the source, whatever its serialized state; its baked
                    // copies, if any, are rows of the prefab that nests them.
                    "source_template_not_instantiated"
                } else if particle["renderer"]["enabled"] == false {
                    "source_renderer_disabled"
                } else if !active {
                    "source_hierarchy_inactive"
                } else if planned.is_some() {
                    "admitted_pending_gpu_and_behavior_verification"
                } else if tally.source_emission_disabled == 1 {
                    // The source module is off and no owner emits into it.
                    "source_emission_disabled"
                } else {
                    // Zero autonomous emission is not proof of invisibility:
                    // subemitters, animation and timeline can trigger emission.
                    "runtime_admission_rejected"
                };
                if let Some(plan) = &planned {
                    if plan.child_owner.is_some() {
                        target_rows.push((rows.len(), node.to_owned()));
                    }
                    delivered.extend(plan.event_edges.iter().flat_map(|edges| edges.targets().map(str::to_owned)));
                }
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
                    "sourceInstantiation": kind.map(|k| format!("{k:?}")),
                    "sourceRoute": format!("{:?}", crate::particle_runtime::source_route(&particle["system"])),
                    "firstPlayWarm": warm,
                    "nativeBirth": planned.as_ref().map(|p| if p.child_owner.is_some() {
                        json!({"path":"subEmitterTarget"})
                    } else {
                        crate::particle_runtime::native_birth_eligible(&p.emitter, &p.route)
                            .and_then(|()| crate::particle_runtime::native_shape_state_eligible(&p.emitter, Some(p.geometry.shape_evidence())))
                            .map_or_else(|reason| json!({"path":"legacy","reason":reason}), |()| json!({"path":"native"}))
                    }),
                    "gpuVerification": "not_run", "gates": format!("{tally:?}"),
                    "culling": planned.as_ref().map(|p| p.culling.label()),
                    "collisionScene": particle["system"]["collision"].is_object().then(|| match &ground {
                        Ok(scene) => json!({"bound": true, "colliders": scene.describe()}),
                        Err(reason) => json!({"bound": false, "reason": reason}),
                    }),
                    "animationRefusal":animation_refusal,"animationContract":animation.report,
                    "softKeyword": material["keywords"].as_array().is_some_and(|v|
                        v.iter().any(|k| k.as_str() == Some("_SOFT_PARTICLES_ENABLED"))),
                }));
            }
            for (row, node) in target_rows {
                if !delivered.contains(&node) {
                    admitted -= 1;
                    rows[row]["admitted"] = json!(false);
                    rows[row]["classification"] = json!("runtime_admission_rejected");
                    rows[row]["nativeBirth"] = Value::Null;
                    rows[row]["gates"] = json!(format!("sub-emitter target {node}: its parent is not admitted in this effect"));
                }
            }
        }
        let total = rows.len() - start;
        println!("{name}: records={total}, renderer_enabled={renderer_enabled}, admitted={admitted}");
        per_phenomenon.insert(name.clone(), json!({
            "records": total, "rendererEnabled": renderer_enabled,
            "admitted": admitted,
        }));
    }
    println!("instantiation: selection over {} census sites; naming role differs on {} effects {:?}",
        sites.len(), naming_role_differs.len(), naming_role_differs);
    let report = json!({
        "meaning": "Observed runtime admission over all exported effect variants, not simultaneous scene population and not proof of source equivalence.",
        "perPhenomenon": per_phenomenon,
        "records": rows.len(),
        "admitted": rows.iter().filter(|r| r["admitted"] == true).count(),
        "cullingExposed": rows.iter().filter(|r| r["culling"]["culled"] == "source").count(),
        "cullingPorted": rows.iter().filter(|r| r["culling"]["port"] == "cullable").count(),
        "collisionExport": collision_source,
        "gpuVerification": "not_run",
        "rows": rows,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write audit report");
}

/// Research instrument: the source environment loader's naming law against a census of every prefab in every
/// phenomenon package, evaluated from the bundle containers over all site package names. Per prefab,
/// `source_environment_role` must give the kind of the prefab's census anchors (none for a prefab the loader
/// never instantiates). With an extraction index, the production selection also runs for every phenomenon of
/// the index and every census site: each prefab must be selected at exactly its census anchors and no prefab
/// outside the census may be selected, so a wrong package or site choice cannot pass as a right role.
#[test]
#[ignore = "requires MOLY_ENVIRONMENT_PREFAB_CENSUS; the selection check also reads MOLY_WEATHER_AUDIT_INDEX (with MOLY_SOURCE_CORPUS_OVERLAY)"]
fn source_environment_role_matches_the_bundle_census() {
    use std::collections::{BTreeMap, BTreeSet};
    let read = |path: &std::path::Path| -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("read replay input")).expect("parse replay input")
    };
    let text = |value: &Value| -> String {
        value.as_str().unwrap_or_else(|| panic!("replay input field is not a string: {value}")).to_owned()
    };
    let label = |kind: EffectKind| match kind {
        EffectKind::Sky => "sky",
        EffectKind::Camera => "camera",
        EffectKind::Site => "site",
    };
    let census = read(std::path::Path::new(
        &std::env::var_os("MOLY_ENVIRONMENT_PREFAB_CENSUS").expect("supply the prefab census"),
    ));
    let cases = census["cases"].as_array().expect("census cases");
    assert!(!cases.is_empty());
    let mut expected: BTreeMap<(String, String, String), BTreeSet<String>> = BTreeMap::new();
    let mut instantiated = 0;
    for case in cases {
        let key = (text(&case["phenomenon"]), text(&case["variant"]), text(&case["prefab"]));
        let anchors: BTreeSet<String> =
            case["anchors"].as_array().expect("census anchors").iter().map(|anchor| text(anchor)).collect();
        let kinds: BTreeSet<&str> = anchors.iter()
            .map(|anchor| anchor.split_once('@').unwrap_or_else(|| panic!("census anchor {anchor}")).0)
            .collect();
        assert!(kinds.len() <= 1, "{key:?}: one role per prefab");
        let role = source_environment_role(&key.2, &key.1, &key.0);
        assert_eq!(role.map(label), kinds.first().copied(), "{key:?}");
        instantiated += usize::from(role.is_some());
        assert!(expected.insert(key, anchors).is_none(), "the census lists a prefab twice");
    }
    println!("census cases {} instantiated {instantiated}", cases.len());

    let Some(index_path) = std::env::var_os("MOLY_WEATHER_AUDIT_INDEX").map(std::path::PathBuf::from) else {
        println!("selection check not run: MOLY_WEATHER_AUDIT_INDEX not supplied");
        return;
    };
    let sites: Vec<String> = census["sites"].as_array().expect("census sites").iter().map(|site| text(site)).collect();
    assert!(!sites.is_empty());
    let root = index_path.parent().expect("index has a parent");
    let overlay = std::env::var_os("MOLY_SOURCE_CORPUS_OVERLAY").map(std::path::PathBuf::from);
    let index = read(&index_path);
    let phenomena = index["phenomena"].as_object().expect("phenomena map");
    let mut selected: BTreeMap<(String, String, String), BTreeSet<String>> = BTreeMap::new();
    for (name, item) in phenomena {
        let source_path = |file: &str| overlay.as_ref().map(|root| root.join(file))
            .filter(|path| path.is_file()).unwrap_or_else(|| root.join(file));
        let doc = read(&source_path(item["fx"]["file"].as_str().expect("fx file")));
        let effects = doc["effects"].as_object().expect("effects map");
        for site in &sites {
            for (prefab, effect, kind) in source_environment_selection(effects, name, site) {
                let variant = text(&effect["variant"]);
                selected.entry((name.clone(), variant, prefab)).or_default().insert(format!("{}@{site}", label(kind)));
            }
        }
    }
    for key in selected.keys() {
        assert!(expected.contains_key(key), "{key:?}: selected, but not a prefab of the census");
    }
    let none = BTreeSet::new();
    for (key, anchors) in &expected {
        assert_eq!(selected.get(key).unwrap_or(&none), anchors, "{key:?}");
    }
    println!(
        "selection phenomena {} sites {} cases {} anchor pairs {} selected prefabs {}",
        phenomena.len(),
        sites.len(),
        expected.len(),
        expected.values().map(BTreeSet::len).sum::<usize>(),
        selected.len(),
    );
}

/// The asset server the admission path loads source materials through, rooted
/// at the extraction that holds the phenomena directory.
fn corpus_app(root: &std::path::Path) -> App {
    let mut app = App::new();
    moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir {
        path: root.parent().expect("phenomena directory has a parent").to_path_buf(),
    });
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ImagePlugin::default()));
    moly_assets::source_shader::loader::register(&mut app);
    app.init_asset::<Gltf>();
    app.finish();
    app.cleanup();
    app
}

/// Replay of a native run of the engine's incremental particle update over the
/// weather systems whose serialized EmissionModule is disabled. With the
/// module's enabled byte clear, every update on both routes and every recorded
/// frame-time chain enters no emission kernel, starts no particle, records no
/// emit and leaves the emission state untouched; only the end-of-duration clear
/// runs. With the byte set, the same configurations enter EmitOverTime. Each
/// effect's kind is the loader's pick, the production selection
/// (`source_environment_selection`) over the census sites
/// (MOLY_ENVIRONMENT_PREFAB_CENSUS), as in the corpus admission: an executed
/// record of a prefab the loader never instantiates on its own is never judged. The production admission must install nothing for exactly the
/// other executed records, name them source-silent rather than refused, stop
/// calling a record silent once its inventory lists the module as enabled, and
/// never call a record silent that the native run did not execute. Every
/// executed record must exist in the extraction.
#[test]
#[ignore = "requires MOLY_NO_EMISSION_OWNERS_RECEIPT, MOLY_ENVIRONMENT_PREFAB_CENSUS and the MOLY_WEATHER_AUDIT_INDEX extraction (with MOLY_SOURCE_CORPUS_OVERLAY) it was taken from"]
fn emission_disabled_weather_systems_follow_the_native_update() {
    use std::collections::BTreeSet;
    let read = |path: &std::path::Path| -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("read replay input"))
            .expect("parse replay input")
    };
    let receipt = read(std::path::Path::new(&std::env::var_os("MOLY_NO_EMISSION_OWNERS_RECEIPT")
        .expect("supply the native emission-disabled update receipt")));
    assert_eq!(
        receipt["library"]["sha256"],
        "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
    );
    let triple = |value: &Value| -> (String, String, String) {
        let text = |v: &Value| v.as_str().expect("record key").to_owned();
        match value {
            Value::Array(parts) if parts.len() == 3 => (text(&parts[0]), text(&parts[1]), text(&parts[2])),
            record => (text(&record["phenomenon"]), text(&record["effect"]), text(&record["node"])),
        }
    };
    let mut executed_off = BTreeSet::new();
    let mut off_updates = 0usize;
    for chain in receipt["source"].as_array().expect("module-off chains") {
        assert_eq!(chain["enabledByte"], 0);
        for row in chain["rows"].as_array().expect("updates") {
            assert_eq!(row["count"], 0, "native particle count");
            assert_eq!(row["records"], 0, "native emit records");
            assert_eq!(row["emissionStateUnchanged"], true);
            assert!(row["births"].as_array().expect("birth entries").is_empty());
            assert!(row["hits"].as_array().expect("kernel entries").iter()
                .all(|hit| hit == "EndOfDurationClear"), "{row}");
            off_updates += 1;
        }
        executed_off.insert(triple(chain));
    }
    // The harness is sensitive: with the byte set it sees emission on every
    // configuration the module-off arm executed.
    let mut sensed = BTreeSet::new();
    for chain in receipt["positive"].as_array().expect("module-on chains") {
        assert_ne!(chain["enabledByte"], 0);
        if chain["rows"].as_array().expect("updates").iter()
            .any(|row| row["hits"].as_array().is_some_and(|hits| hits.iter().any(|hit| hit == "EmitOverTime")))
        {
            sensed.extend(chain["members"].as_array().expect("members").iter().map(triple));
        }
    }
    assert!(!executed_off.is_empty());
    assert_eq!(sensed, executed_off);

    let index_path = std::path::PathBuf::from(
        std::env::var_os("MOLY_WEATHER_AUDIT_INDEX").expect("supply extraction index"),
    );
    let root = index_path.parent().expect("index has a parent");
    let overlay = std::env::var_os("MOLY_SOURCE_CORPUS_OVERLAY").map(std::path::PathBuf::from);
    let index = read(&index_path);
    // The kind is the loader's pick over the census sites, as in the corpus admission.
    let census = read(std::path::Path::new(
        &std::env::var_os("MOLY_ENVIRONMENT_PREFAB_CENSUS").expect("supply the environment prefab census for its sites"),
    ));
    let sites: Vec<String> = census["sites"].as_array().expect("census sites").iter()
        .map(|site| site.as_str().unwrap_or_else(|| panic!("census site is not a string: {site}")).to_owned())
        .collect();
    assert!(!sites.is_empty(), "the census names no site");
    // The collider export, resolved as the corpus admission resolves it.
    let collision_document = std::env::var_os("MOLY_WEATHER_AUDIT_COLLISION").map(std::path::PathBuf::from)
        .or_else(|| index.pointer("/collision/file").and_then(Value::as_str).map(|file| root.join(file)))
        .map(|path| read(&path)).ok_or_else(|| "this asset root carries no collider export".to_owned());
    let mut scenes = crate::particle_runtime::collision_scene::SceneBuilder::new(collision_document);
    let app = corpus_app(root);
    let server = app.world().resource::<AssetServer>();
    let (mut silent, mut found, mut unjudged, mut records) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), 0usize);
    let mut naming_role_differs = 0usize;
    for (name, item) in index["phenomena"].as_object().expect("phenomena map") {
        let source_path = |file: &str| overlay.as_ref().map(|root| root.join(file))
            .filter(|path| path.is_file()).unwrap_or_else(|| root.join(file));
        let doc = read(&source_path(item["fx"]["file"].as_str().expect("fx file")));
        let animation_doc = item["animations"]["file"].as_str().map(|file| read(&source_path(file)))
            .or_else(|| overlay.as_ref().map(|root| root.join(name).join("animations.json"))
                .filter(|path| path.is_file()).map(|path| read(&path)));
        let effects = doc["effects"].as_object().expect("effects map");
        let mut selected: HashMap<String, EffectKind> = HashMap::new();
        for site in &sites {
            for (prefab, _, kind) in source_environment_selection(effects, name, site) {
                let previous = selected.insert(prefab.clone(), kind);
                assert!(previous.is_none_or(|previous| previous == kind), "{name}/{prefab}: two kinds across sites");
            }
        }
        for (effect_name, effect) in effects {
            let animation = crate::weather_animation::Contract::compile(effect, animation_doc.as_ref());
            let variant = effect["variant"].as_str()
                .unwrap_or_else(|| panic!("{name}/{effect_name}: package variant is not a string"));
            let kind = selected.get(effect_name.as_str()).copied();
            if kind != source_environment_role(effect_name, variant, name) { naming_role_differs += 1; }
            let by_path: HashMap<String, &Value> = effect["nodes"].as_array().expect("nodes")
                .iter().map(|n| (n["path"].as_str().expect("node path").to_owned(), n)).collect();
            let particles = effect["particles"].as_array().expect("particles");
            let owners = source_sub_emitter_owners(particles);
            let ground = scenes.for_effect(effect_name);
            // As in the admission, only an effect that is judged needs its lifecycle metadata.
            let lifecycle = kind.map(|_| WeatherEffectLifecycle::from_effect(effect).expect("source lifecycle metadata"));
            let judged = |particle: &Value| {
                let mut tally = Tally::default();
                let node = particle["node"].as_str().expect("particle node");
                let planned = kind.zip(lifecycle).filter(|_| animation.refusal(node).is_none()).and_then(|(kind, lifecycle)|
                    judge(effect_name, particle, &by_path, &owners, kind,
                        effect["effectiveRotation"].as_str() == Some("normal"), lifecycle, &ground, server, &mut tally));
                (planned.is_some(), tally)
            };
            for particle in particles {
                records += 1;
                let key = (name.clone(), effect_name.clone(), particle["node"].as_str().expect("particle node").to_owned());
                let (planned, tally) = judged(particle);
                if tally.source_emission_disabled != 0 {
                    assert!(!planned && tally.source_emission_disabled == 1, "{key:?}: {tally:?}");
                    silent.insert(key.clone());
                }
                if !executed_off.contains(&key) { continue; }
                found.insert(key.clone());
                if kind.is_none() {
                    // A prefab the loader never instantiates on its own: the admission never judges it.
                    assert!(!planned && tally.records == 0 && tally.source_emission_disabled == 0,
                        "{key:?}: a record without a loader role was judged: {tally:?}");
                    unjudged.insert(key);
                    continue;
                }
                assert!(!planned, "{key:?}: the engine births nothing, so nothing may be installed");
                assert_eq!(tally.source_emission_disabled, 1, "{key:?}: {tally:?}");
                // The same record with the module listed as enabled is the
                // native module-on arm, which emits: it must not read as silent.
                let mut enabled = particle.clone();
                let inventory = enabled["system"]["sourceModules"]["enabled"].as_array_mut().expect("module inventory");
                inventory.push(json!("EmissionModule"));
                inventory.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
                let (_, control) = judged(&enabled);
                assert_eq!(control.source_emission_disabled, 0, "{key:?}: {control:?}");
            }
        }
    }
    assert_eq!(found, executed_off, "every native record exists in the extraction");
    let judged_off: BTreeSet<_> = executed_off.difference(&unjudged).cloned().collect();
    assert_eq!(silent, judged_off,
        "the source-silent class is exactly the native module-off records the loader instantiates");
    println!("emission-disabled replay: {} records, {} native module-off records over {} updates, {} without a loader role (not judged), {} source-silent; kind from the selection over {} census sites, naming role differs on {} effects",
        records, executed_off.len(), off_updates, unjudged.len(), silent.len(), sites.len(), naming_role_differs);
}

/// Replay of the engine's owner matrix for an emitter below an animated Transform, recorded at sampled Animator
/// frames: ParticleSystem::UpdateLocalToWorldMatrixAndScales (Local scaling) over the TransformHierarchy chain
/// anchor -> effect root -> ... -> emitter, every link at the export's serialized TRS except the emitter's own node,
/// whose rotation is the one the engine's Animator wrote at that frame (or a sampled unit rotation on the same
/// chain); a synthetic family carries its own chains of the admitted form (identity rotations above the emitter,
/// unit scales, at most two links with a translation, depth three or four). The product's world matrix of the
/// same emitter is `NodeChain::affine` over `authored_chain` with that rotation in place of the serialized one,
/// carried to the world by `compose_to_world` under a translation-only sky anchor. Taken back from the product
/// frame (X reflected), it must equal the native localToWorld word for word (+-0 equal), and it must move with
/// the rotation: the serialized chain gives another matrix on some case.
#[test]
#[ignore = "requires MOLY_ANIMATED_CHAIN_NATIVE and MOLY_ANIMATED_CHAIN_EFFECTS"]
fn animated_chain_world_matrix_matches_native_owner_matrix() {
    let read = |key: &str| -> Value {
        let path = std::env::var_os(key).unwrap_or_else(|| panic!("{key} not set"));
        serde_json::from_slice(&std::fs::read(path).expect("read replay input")).expect("parse replay input")
    };
    let receipt = read("MOLY_ANIMATED_CHAIN_NATIVE");
    assert_eq!(receipt["library"]["sha256"], "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
    let effects = read("MOLY_ANIMATED_CHAIN_EFFECTS");
    let word = |v: &Value| u32::try_from(v.as_u64().expect("u32 word")).expect("u32 word");
    let words = |v: &Value| -> Vec<u32> { v.as_array().expect("word list").iter().map(word).collect() };
    let serialized = |v: &Value| -> Vec<u32> {
        v.as_array().expect("serialized list").iter().map(|x| (x.as_f64().expect("number") as f32).to_bits()).collect()
    };
    let same = |a: u32, b: u32| a == b || (a & 0x7fff_ffff == 0 && b & 0x7fff_ffff == 0);
    let one = 1.0f32.to_bits();
    // Product frame to engine frame: a 3x3 entry changes sign when exactly one of its row and column is X; the
    // translation changes sign in X.
    let engine_words = |world: &GlobalTransform| -> [u32; 12] {
        let affine = world.affine();
        let columns = [affine.matrix3.x_axis, affine.matrix3.y_axis, affine.matrix3.z_axis, affine.translation];
        std::array::from_fn(|i| {
            let (c, r) = (i / 3, i % 3);
            let flip = if c < 3 { (r == 0) != (c == 0) } else { r == 0 };
            columns[c][r].to_bits() ^ if flip { 0x8000_0000 } else { 0 }
        })
    };
    let (mut cases, mut moved) = (0usize, 0usize);
    let mut mismatches = Vec::new();
    for case in receipt["cases"].as_array().expect("cases") {
        let effect_name = case["effect"].as_str().expect("effect");
        let node = case["node"].as_str().expect("node");
        // A case of the synthetic family carries its own export-shaped node list.
        let nodes = case.get("nodes").unwrap_or(&effects["effects"][effect_name]["nodes"]);
        let by_path: HashMap<String, &Value> = nodes.as_array().expect("nodes")
            .iter().map(|n| (n["path"].as_str().expect("node path").to_owned(), n)).collect();
        let chain = authored_chain(&by_path, node).unwrap_or_else(|| panic!("{effect_name}/{node}: chain does not resolve"));
        let written = words(&case["written"]);
        let anchor = words(&case["anchor"]);
        // The chain the engine ran: the anchor, then the export's links at their serialized TRS with the emitter's
        // rotation replaced by the written one.
        let links = case["links"].as_array().expect("links");
        assert_eq!(links.len(), chain.links.len() + 1, "{effect_name}/{node}: chain length");
        assert_eq!(words(&links[0]["t"]), vec![anchor[0] ^ 0x8000_0000, anchor[1], anchor[2]], "anchor translation");
        assert_eq!((words(&links[0]["q"]), words(&links[0]["s"])), (vec![0, 0, 0, one], vec![one; 3]), "anchor");
        for (link, native) in chain.links.iter().zip(&links[1..]) {
            assert_eq!(native["path"].as_str(), Some(link.path.as_str()), "{effect_name}/{node}: link order");
            let n = by_path[&link.path];
            let rotation = if link.path == node { written.clone() } else { serialized(&n["rotation"]) };
            assert_eq!((words(&native["t"]), words(&native["q"]), words(&native["s"])),
                (serialized(&n["position"]), rotation, serialized(&n["scale"])), "{}: native link", link.path);
        }
        let rotation: [f32; 4] = std::array::from_fn(|i| f32::from_bits(written[i]));
        let ctx = crate::particle_runtime::Context {
            sky: GlobalTransform::from_translation(Vec3::new(f32::from_bits(anchor[0]), f32::from_bits(anchor[1]),
                f32::from_bits(anchor[2]))),
            camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY,
        };
        let mut system = crate::particle_runtime::test_support::runtime();
        system.kind = EffectKind::Sky;
        system.node_affine = chain.affine(|path| (path == node).then_some(rotation));
        let ours = engine_words(&crate::particle_runtime::compose_to_world(&system, &ctx));
        system.node_affine = chain.serialized_affine();
        let still = engine_words(&crate::particle_runtime::compose_to_world(&system, &ctx));
        let native = words(&case["localToWorld"]);
        let expected: [u32; 12] = std::array::from_fn(|i| native[4 * (i / 3) + i % 3]);
        if (0..12).any(|i| !same(ours[i], expected[i])) {
            mismatches.push(format!("{node} {} frame {} anchor {anchor:08x?}: native {expected:08x?} ours {ours:08x?}",
                case["seq"], case["frame"]));
        }
        moved += usize::from((0..12).any(|i| !same(ours[i], still[i])));
        cases += 1;
    }
    println!("animated chain: {cases} cases, {} mismatched, matrix moved by the rotation in {moved}", mismatches.len());
    assert!(mismatches.is_empty(), "{} of {cases} mismatched:\n{}", mismatches.len(),
        mismatches.iter().take(8).cloned().collect::<Vec<_>>().join("\n"));
    assert!(cases > 0 && moved > 0, "cases={cases} moved={moved}");
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
        PlannedGeometry::Mesh { alignment, scaling, pivot, flip, .. } => crate::particle_runtime::Geometry::Mesh(
            crate::particle_geometry::MeshDraw {
                source: Arc::new(crate::particle_geometry::SourceMesh { positions: Vec::new(), normals: Vec::new(),
                    uv: Vec::new(), colours: Vec::new(), indices: Vec::new(), bounds_size: Vec3::ZERO }),
                alignment: *alignment, scaling: *scaling, pivot: *pivot, flip: *flip,
            }),
    };
    system.emitter = e.clone();
    system.cone_angle = planned.cone_angle;
    system.rol = planned.rol.clone();
    system.limit = planned.limit.clone();
    system.velocity_law = e.velocity_over_lifetime.as_ref().map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission"));
    system.force_law = e.force.as_ref().map(|p| moly_law::particle::force::ForceOverLifetime::from_params(p).expect("force validated during admission"));
    system.gravity_law = moly_law::particle::gravity::Gravity::new(&e.start.gravity_modifier).expect("curves validated during admission");
    system.size_law = e.size_over_lifetime.as_ref().map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission"));
    system.color_law = e.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = e.custom_data.as_ref().map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission"));
    let collision = planned.collision_scene.clone().map(|scene| crate::particle_runtime::CollisionInstall {
        scene: Box::new(crate::particle_runtime::collision_scene::GroundQuery::new(scene)),
        owner: planned.collision_owner,
    });
    let path = match crate::particle_runtime::install_native_birth(&mut system, seeds, &planned.route, collision) {
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
