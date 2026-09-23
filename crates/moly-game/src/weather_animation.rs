//! Source-scoped weather Transform animation preparation.
//!
//! A model clip is not an automatic player. Controller layer metadata and scalar
//! coefficients can be compiled without pretending that additive reference poses
//! or Animator's initial evaluation phase have been reproduced. Those gates stay
//! closed until a qualified native evaluation supplies the missing semantics.

use bevy::prelude::{GlobalTransform, Quat, Transform, Vec3};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::source_curve::Curve;

/// Effect instance age, independent of particle playback, Stop and camera access.
/// This is deliberately not called an Animator phase.
pub(crate) struct EffectClock {
    installed_at: f64,
    elapsed_bits: AtomicU64,
}

impl EffectClock {
    pub(crate) fn new(installed_at: f64) -> Self {
        assert!(installed_at.is_finite());
        Self {
            installed_at,
            elapsed_bits: AtomicU64::new(0.0f64.to_bits()),
        }
    }

    pub(crate) fn observe(&self, now: f64) {
        if now.is_finite() {
            self.elapsed_bits.store(
                (now - self.installed_at).max(0.0).to_bits(),
                Ordering::Relaxed,
            );
        }
    }

    pub(crate) fn age(&self) -> f64 {
        f64::from_bits(self.elapsed_bits.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Identity {
    bundle: String,
    archive: String,
    path_id: String,
}

impl Identity {
    fn read(value: &Value) -> Result<Self, String> {
        let result = Self {
            bundle: string(value, "bundle")?.into(),
            archive: string(value, "archive")?.into(),
            path_id: string(value, "pathId")?.into(),
        };
        // A decimal string avoids all JSON/JavaScript precision loss.
        if result.path_id.parse::<i64>().is_err() || result.path_id == "0" {
            return Err("source identity requires a non-null signed path ID string".into());
        }
        Ok(result)
    }

    fn same_file(&self, other: &Self) -> bool {
        self.bundle == other.bundle && self.archive == other.archive
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string {key}"))
}

fn number(value: &Value) -> Result<f32, String> {
    value
        .as_f64()
        .map(|n| n as f32)
        .filter(|n| n.is_finite())
        .ok_or_else(|| "non-finite or missing scalar".into())
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array {key}"))
}

fn flag(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| {
        value
            .as_i64()
            .filter(|n| *n == 0 || *n == 1)
            .map(|n| n != 0)
    })
}

fn in_scope(path: &str, root: &str) -> bool {
    root.is_empty()
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

/// A prepared scalar channel. Sampling requires caller-supplied clip time;
/// effect age is never silently treated as normalized Animator state time.
struct BoundCurve {
    path: String,
    attribute: u64,
    component: u64,
    curve: Curve,
}

impl BoundCurve {
    fn read(
        row: &Value,
        animator: &Identity,
        scope: &Value,
        effect_path: &str,
    ) -> Result<Self, String> {
        let targets = array(row, "targets")?;
        let [target] = targets.as_slice() else {
            return Err("curve requires exactly one source target".into());
        };
        if string(target, "scopeKind")? != "animator"
            || Identity::read(&target["scope"])? != *animator
        {
            return Err("curve target is outside its exact Animator scope".into());
        }
        let relative = string(target, "path")?;
        let candidates: Vec<_> = array(scope, "nodes")?
            .iter()
            .filter(|node| node["path"] == relative)
            .collect();
        let [node] = candidates.as_slice() else {
            return Err("curve path is missing or ambiguous in declared scope".into());
        };
        if Identity::read(&target["target"])? != Identity::read(&node["transform"])?
            || Identity::read(&target["gameObject"])? != Identity::read(&node["source"])?
            || row["binding"]["pathHash"] != node["pathHash"]
        {
            return Err("curve binding identity disagrees with the source hierarchy".into());
        }
        let binding = &row["binding"];
        let attribute = binding["attribute"]
            .as_u64()
            .ok_or("missing binding attribute")?;
        let component = binding["component"]
            .as_u64()
            .ok_or("missing binding component")?;
        if binding["typeId"] != 4
            || !(1..=4).contains(&attribute)
            || component >= if attribute == 2 { 4 } else { 3 }
        {
            return Err("non-Transform or unsupported Transform channel".into());
        }
        let curve = match string(row, "kind")? {
            "const" => {
                let points = array(row, "points")?;
                let [point] = points.as_slice() else {
                    return Err("constant curve needs one point".into());
                };
                let pair = point
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .ok_or("invalid constant point")?;
                number(&pair[0])?;
                Curve::Const(number(&pair[1])?)
            }
            "cubic" => {
                let mut keys = Vec::new();
                for point in array(row, "points")? {
                    let pair = point
                        .as_array()
                        .filter(|p| p.len() == 2)
                        .ok_or("invalid cubic key")?;
                    let time = number(&pair[0])?;
                    let coefficients = pair[1]
                        .as_array()
                        .filter(|p| p.len() == 4)
                        .ok_or("invalid cubic coefficients")?;
                    let mut values = [0.0; 4];
                    for (out, value) in values.iter_mut().zip(coefficients) {
                        *out = number(value)?;
                    }
                    if keys.last().is_some_and(|(previous, _)| *previous >= time) {
                        return Err("unordered cubic keys".into());
                    }
                    keys.push((time, values));
                }
                if keys.is_empty() {
                    return Err("empty cubic curve".into());
                }
                Curve::Cubic(keys)
            }
            _ => return Err("unsupported curve encoding".into()),
        };
        let path = match (effect_path.is_empty(), relative.is_empty()) {
            (true, _) => relative.into(),
            (_, true) => effect_path.into(),
            _ => format!("{effect_path}/{relative}"),
        };
        Ok(Self {
            path,
            attribute,
            component,
            curve,
        })
    }
}

/// Per-effect multi-gap contract; only affected nodes and their descendants are
/// rejected when every target is known. An unresolved owner rejects its scope.
pub(crate) struct Contract {
    pub(crate) report: Value,
    blocked: Vec<(String, String)>,
    curves: Vec<BoundCurve>,
}

impl Contract {
    pub(crate) fn refusal(&self, node: &str) -> Option<&str> {
        self.blocked
            .iter()
            .find(|(root, _)| in_scope(node, root))
            .map(|(_, reason)| reason.as_str())
    }

    pub(crate) fn compile(effect: &Value, document: Option<&Value>) -> Self {
        let mut result = Self {
            report: json!({}),
            blocked: Vec::new(),
            curves: Vec::new(),
        };
        let mut reports = Vec::new();
        let declarations = effect["animationComponents"].as_array();
        let inventoried = effect["animationSchemaVersion"] == 1 && declarations.is_some();
        let mut declared = HashSet::new();
        if let Some(declarations) = declarations {
            for declaration in declarations {
                let path = declaration["node"].as_str().unwrap_or("");
                let component = declaration["component"].as_str().unwrap_or("unknown");
                let identity = Identity::read(&declaration["source"]);
                if let Ok(identity) = &identity {
                    declared.insert(identity.clone());
                }
                let compiled = identity.and_then(|identity| {
                    let document = document.ok_or("animation document unavailable")?;
                    if document["schemaVersion"] != 1 { return Err("unsupported animation document version".into()); }
                    let kind = match component { "Animator" => "animators", "PlayableDirector" => "directors", _ => return Err("unknown animation component".into()) };
                    let rows: Vec<_> = array(document, "packages")?.iter().flat_map(|p| p[kind].as_array().into_iter().flatten())
                        .filter(|row| Identity::read(&row["source"]).as_ref() == Ok(&identity)).collect();
                    let [row] = rows.as_slice() else { return Err("animation component source missing or ambiguous".into()); };
                    if component == "PlayableDirector" {
                        let owner = Identity::read(&row["gameObject"]["target"])?;
                        if exact_effect_owner(effect, &owner) != Some(path) { return Err("director GameObject does not match exact effect node owner".into()); }
                        let empty = row["playableAsset"]["pointer"]["pathId"] == "0"
                            && array(row, "sceneBindings")?.iter().all(|binding| binding["key"]["pointer"]["pathId"] == "0"
                                && binding["value"]["pointer"]["pathId"] == "0");
                        if !empty { return Err("PlayableDirector execution unverified".into()); }
                        return Ok(json!({"component":component,"node":path,"status":"authored_empty_director"}));
                    }
                    result.compile_animator(effect, document, row, &identity, path)
                });
                match compiled {
                    Ok(report) => reports.push(report),
                    Err(reason) => {
                        result.blocked.push((path.into(), reason.clone()));
                        reports.push(json!({"component":component,"node":path,"status":"refused","reason":reason}));
                    }
                }
            }
        }
        // Older effects must not quietly imply an absence of animation. Detect
        // retained component omissions and exact manifest owners as well.
        for key in ["unsupported", "omitted"] {
            for row in effect[key].as_array().into_iter().flatten() {
                if matches!(
                    row["component"].as_str(),
                    Some("Animator" | "PlayableDirector")
                ) {
                    let path = row["node"].as_str().unwrap_or("");
                    result.blocked.push((
                        path.into(),
                        "animation component missing a source contract".into(),
                    ));
                }
            }
        }
        if let Some(document) = document {
            for package in document["packages"].as_array().into_iter().flatten() {
                for row in package["animators"].as_array().into_iter().flatten() {
                    let (Ok(identity), Ok(owner)) = (
                        Identity::read(&row["source"]),
                        Identity::read(&row["scope"]["gameObject"]),
                    ) else {
                        continue;
                    };
                    if !declared.contains(&identity) {
                        if let Some(path) = exact_effect_owner(effect, &owner) {
                            result.blocked.push((
                                path.into(),
                                "Animator source owner present but effect declaration missing"
                                    .into(),
                            ));
                        }
                    }
                }
            }
        }
        let model_only_clips = document
            .into_iter()
            .flat_map(|document| document["packages"].as_array().into_iter().flatten())
            .flat_map(|package| package["clips"].as_array().into_iter().flatten())
            .filter(|clip| {
                clip["scopes"].as_array().is_some_and(|scopes| {
                    !scopes.is_empty() && scopes.iter().all(|scope| scope["kind"] == "modelAsset")
                })
            })
            .count();
        result.report = json!({"inventory":if inventoried {"declared"} else {"legacy_animation_inventory_unknown"},
            "components":reports,"preparedCurveSlots":result.curves.len(),
            "modelOnlyClipsWithoutPlaybackOwner":model_only_clips,
            "channels":result.curves.iter().map(|curve|json!({"node":curve.path,"attribute":curve.attribute,"component":curve.component})).collect::<Vec<_>>(),
            "blockedScopes":result.blocked.iter().map(|(path,reason)|json!({"node":path,"reason":reason})).collect::<Vec<_>>(),
            "playbackAccepted":false});
        result
    }

    fn compile_animator(
        &mut self,
        effect: &Value,
        document: &Value,
        row: &Value,
        identity: &Identity,
        path: &str,
    ) -> Result<Value, String> {
        let scope = &row["scope"];
        let owner = Identity::read(&scope["gameObject"])?;
        if exact_effect_owner(effect, &owner) != Some(path)
            || Identity::read(&scope["source"])? != *identity
        {
            return Err("Animator GameObject does not match the exact effect node owner".into());
        }
        if flag(&row["fields"]["m_Enabled"]) == Some(false) {
            return Ok(json!({"component":"Animator","node":path,"status":"source_disabled"}));
        }
        if flag(&row["fields"]["m_Enabled"]) != Some(true)
            || row["fields"]["m_UpdateMode"] != 0
            || row["fields"]["m_CullingMode"] != 0
            || flag(&row["fields"]["m_ApplyRootMotion"]) != Some(false)
        {
            return Err("Animator enable/update/culling/root-motion controls unverified".into());
        }
        let controller_id = Identity::read(&row["controller"]["target"])?;
        let controllers: Vec<_> = array(document, "packages")?
            .iter()
            .flat_map(|p| p["controllers"].as_array().into_iter().flatten())
            .filter(|c| Identity::read(&c["source"]).as_ref() == Ok(&controller_id))
            .collect();
        let [controller] = controllers.as_slice() else {
            return Err("Animator controller source missing or ambiguous".into());
        };
        let decoded = &controller["decoded"];
        if decoded["status"] != "decoded" || decoded["motionMapping"]["status"] != "resolved" {
            return Err("controller motion identity mapping unresolved".into());
        }
        if !array(decoded, "parameters")?.is_empty() || !array(decoded, "behaviours")?.is_empty() {
            return Err("controller parameters or behaviours need a source executor".into());
        }
        let mut layers = Vec::new();
        let mut affected = HashSet::new();
        for (layer_index, layer) in array(decoded, "layers")?.iter().enumerate() {
            let layer = layer.get("data").unwrap_or(layer);
            let machine_index = layer["m_StateMachineIndex"]
                .as_u64()
                .ok_or("missing state machine index")?;
            let machines: Vec<_> = array(decoded, "stateMachines")?
                .iter()
                .filter(|machine| machine["index"] == machine_index)
                .collect();
            let [machine] = machines.as_slice() else {
                return Err("state machine index missing or ambiguous".into());
            };
            let states = array(machine, "states")?;
            let blending = layer["(int&)m_LayerBlendingMode"]
                .as_u64()
                .ok_or("missing layer blend mode")?;
            let weight = number(&layer["m_DefaultWeight"])?;
            if states.is_empty() {
                layers.push(json!({"index":layer_index,"blending":blending,"serializedWeight":weight,"state":"empty"}));
                continue;
            }
            if states.len() != 1
                || !array(machine, "anyStateTransitions")?.is_empty()
                || states[0]["index"] != machine["defaultState"]
                || !array(&states[0], "transitions")?.is_empty()
            {
                return Err("controller state transitions unverified".into());
            }
            let state = &states[0];
            let motions = array(state, "motions")?;
            let [motion] = motions.as_slice() else {
                return Err("controller blend tree execution unverified".into());
            };
            let source = &motion["clip"]["source"];
            if motion["clip"]["status"] != "resolved" {
                return Err("motion clip is unresolved".into());
            }
            let clip_id = Identity {
                bundle: controller_id.bundle.clone(),
                archive: string(source, "file")?.into(),
                path_id: string(source, "pathId")?.into(),
            };
            let clips: Vec<_> = array(document, "packages")?
                .iter()
                .flat_map(|p| p["clips"].as_array().into_iter().flatten())
                .filter(|clip| Identity::read(&clip["source"]).as_ref() == Ok(&clip_id))
                .collect();
            let [clip] = clips.as_slice() else {
                return Err("motion clip source missing or ambiguous".into());
            };
            let start = number(&clip["startTime"])?;
            let stop = number(&clip["stopTime"])?;
            if stop <= start
                || !array(clip, "events")?.is_empty()
                || !array(&clip["accounting"], "unresolved")?.is_empty()
            {
                return Err("clip clock/events/bindings unverified".into());
            }
            let mut slots = HashSet::new();
            for curve in array(clip, "curves")? {
                if !slots.insert(curve["slot"].as_u64().ok_or("missing curve slot")?) {
                    return Err("duplicate curve slot".into());
                }
                let bound = BoundCurve::read(curve, identity, scope, path)?;
                let target = &curve["targets"][0];
                if exact_effect_owner(effect, &Identity::read(&target["gameObject"])?)
                    != Some(bound.path.as_str())
                {
                    return Err(
                        "animated Transform is not the exact exported effect descendant".into(),
                    );
                }
                // Exercise the exact scalar coefficients during preparation,
                // independently from any unqualified blend/evaluation phase.
                if !bound.curve.sample(start).is_finite() || !bound.curve.sample(stop).is_finite() {
                    return Err("non-finite sampled curve".into());
                }
                affected.insert(bound.path.clone());
                self.curves.push(bound);
            }
            if clip["accounting"]["decodedSlots"].as_u64() != Some(slots.len() as u64)
                || clip["accounting"]["bindingSlots"].as_u64() != Some(slots.len() as u64)
            {
                return Err("clip scalar binding accounting mismatch".into());
            }
            layers.push(json!({"index":layer_index,"blending":blending,"serializedWeight":weight,
                "stateIndex":state["index"],"speed":state["speed"],"clip":clip["source"],"startTime":start,"stopTime":stop,
                "loopTime":clip["loopTime"],"curveSlots":slots.len(),
                "gates":if blending == 1 {vec!["additive_reference_pose_and_blending_unverified","animator_initial_phase_unverified"]}
                    else {vec!["controller_layer_evaluation_unverified","animator_initial_phase_unverified"]}}));
        }
        for affected in affected {
            self.blocked.push((
                affected,
                "Animator layer evaluation and initial phase unverified".into(),
            ));
        }
        Ok(
            json!({"component":"Animator","node":path,"status":"prepared_playback_refused","layers":layers}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(path: &str) -> Value {
        json!({"bundle":"package","archive":"archive","pathId":path})
    }

    fn node(path: &str, parent: Value, id: &str, position: [f32; 3]) -> Value {
        json!({"path":path,"parent":parent,"gameObjectPathId":id,"position":position,
            "rotation":[0,0,0,1],"scale":[1,1,1]})
    }

    #[test]
    fn model_scopes_do_not_create_automatic_players_and_legacy_is_unknown() {
        let effect = json!({"source":identity("1"),"nodes":[node("",Value::Null,"1",[0.0;3])]});
        let document = json!({"schemaVersion":1,"packages":[{"animators":[],"clips":[{"scopes":[{"kind":"modelAsset"}]}]}]});
        let contract = Contract::compile(&effect, Some(&document));
        assert_eq!(
            contract.report["inventory"],
            "legacy_animation_inventory_unknown"
        );
        assert_eq!(contract.report["modelOnlyClipsWithoutPlaybackOwner"], 1);
        assert!(contract.curves.is_empty());
        assert!(contract.refusal("").is_none());
    }

    #[test]
    fn missing_and_ambiguous_component_identities_fail_closed() {
        let effect = json!({"source":identity("1"),"nodes":[node("",Value::Null,"1",[0.0;3])],
            "animationSchemaVersion":1,"animationComponents":[{"node":"","component":"Animator","source":identity("2")}]});
        assert_eq!(
            Contract::compile(&effect, None).refusal("any/child"),
            Some("animation document unavailable")
        );
        let row = json!({"source":identity("2")});
        let document = json!({"schemaVersion":1,"packages":[{"animators":[row,row]}]});
        assert_eq!(
            Contract::compile(&effect, Some(&document)).refusal(""),
            Some("animation component source missing or ambiguous")
        );
    }

    #[test]
    fn legacy_animator_omissions_are_explicit_and_subtree_matching_is_exact() {
        let effect = json!({"omitted":[{"node":"root/body","component":"Animator"}]});
        let contract = Contract::compile(&effect, None);
        assert!(contract.refusal("root/body/child").is_some());
        assert!(contract.refusal("root/bodyguard").is_none());
    }

    #[test]
    fn undeclared_exact_animator_owner_is_refused_without_name_fallback() {
        let effect = json!({"source":identity("1"),"nodes":[node("",Value::Null,"1",[0.0;3]),node("animated",json!(""),"3",[0.0;3])]});
        let document = json!({"schemaVersion":1,"packages":[{"animators":[{
            "source":identity("2"),"scope":{"gameObject":identity("3")}}]}]});
        let contract = Contract::compile(&effect, Some(&document));
        assert!(contract.refusal("animated/child").is_some());
        assert!(contract.refusal("static").is_none());
        let mut wrong_archive = document;
        wrong_archive["packages"][0]["animators"][0]["scope"]["gameObject"]["archive"] =
            json!("other_archive");
        assert!(
            Contract::compile(&effect, Some(&wrong_archive))
                .blocked
                .is_empty()
        );
    }

    #[test]
    fn cubic_coefficients_and_exact_transform_binding_are_preserved() {
        let owner = Identity::read(&identity("2")).unwrap();
        let scope = json!({"nodes":[{"path":"child","pathHash":17,"source":identity("3"),"transform":identity("4")}]});
        let mut row = json!({"binding":{"typeId":4,"attribute":4,"component":1,"pathHash":17},
            "kind":"cubic","points":[[0,[0.25,-1.0,2.0,4.0]],[2,[0,0,0,6]]],
            "targets":[{"scopeKind":"animator","scope":identity("2"),"path":"child","gameObject":identity("3"),"target":identity("4")}]});
        let bound = BoundCurve::read(&row, &owner, &scope, "root").unwrap();
        assert_eq!(bound.path, "root/child");
        assert_eq!(bound.curve.sample(1.0), 5.25);
        assert_eq!(bound.curve.sample(2.0), 6.0);
        row["targets"][0]["target"] = identity("5");
        assert!(BoundCurve::read(&row, &owner, &scope, "").is_err());
    }

    #[test]
    fn hierarchy_recomposes_descendants_without_incremental_drift() {
        let hierarchy = Hierarchy::read(&[
            node("", Value::Null, "1", [2.0, 0.0, 0.0]),
            node("rotator", json!(""), "2", [0.0; 3]),
            node("rotator/emitter", json!("rotator"), "3", [0.0, 0.0, 3.0]),
        ])
        .unwrap();
        let pose = HashMap::from([(
            "rotator".into(),
            Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        )]);
        for _ in 0..3 {
            let affines = hierarchy.affines(&pose).unwrap();
            assert!(
                (affines["rotator/emitter"].translation() - Vec3::new(1.0, 0.0, 0.0)).length()
                    < 1e-5
            );
        }
        assert_eq!(
            hierarchy.affines(&HashMap::new()).unwrap()["rotator/emitter"].translation(),
            Vec3::new(-2.0, 0.0, 3.0)
        );
        assert!(
            Hierarchy::read(&[
                node("a", json!("b"), "1", [0.0; 3]),
                node("b", json!("a"), "2", [0.0; 3])
            ])
            .is_err()
        );
        assert!(Hierarchy::read(&[node("a", json!("missing"), "1", [0.0; 3])]).is_err());
    }

    #[test]
    fn shared_effect_clock_is_independent_of_particle_speed_and_stop() {
        let clock = std::sync::Arc::new(EffectClock::new(12.0));
        let second_emitter = clock.clone();
        clock.observe(13.5);
        assert_eq!(second_emitter.age(), 1.5);
        drop(clock); // First emitter can retire/die without resetting its siblings.
        second_emitter.observe(15.0);
        assert_eq!(second_emitter.age(), 3.0);
    }

    #[test]
    #[ignore = "requires MOLY_WEATHER_ANIMATION_DOCUMENT, MOLY_WEATHER_ANIMATION_EFFECTS and MOLY_WEATHER_ANIMATION_OUT"]
    fn current_source_animation_contract() {
        let read = |key: &str| -> Value {
            serde_json::from_slice(
                &std::fs::read(std::env::var_os(key).expect(key)).expect("read animation source"),
            )
            .expect("parse animation source")
        };
        let document = read("MOLY_WEATHER_ANIMATION_DOCUMENT");
        let effects = read("MOLY_WEATHER_ANIMATION_EFFECTS");
        let mut reports = serde_json::Map::new();
        let mut prepared = 0usize;
        for (name, effect) in effects["effects"].as_object().expect("effects") {
            let contract = Contract::compile(effect, Some(&document));
            for row in contract.report["components"].as_array().unwrap() {
                assert_ne!(
                    row["status"], "refused",
                    "source contract unexpectedly unresolved: {row}"
                );
            }
            for curve in &contract.curves {
                assert!(
                    contract.refusal(&curve.path).is_some(),
                    "unqualified layer must stay gated"
                );
            }
            prepared += contract.curves.len();
            reports.insert(name.clone(), contract.report);
        }
        let expected: usize = document["packages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|package| package["clips"].as_array().into_iter().flatten())
            .filter(|clip| {
                clip["scopes"]
                    .as_array()
                    .is_some_and(|scopes| scopes.iter().any(|scope| scope["kind"] == "animator"))
            })
            .map(|clip| clip["curves"].as_array().unwrap().len())
            .sum();
        assert_eq!(
            prepared, expected,
            "all exact Animator clip slots must reach preparation"
        );
        let receipt = json!({"effects":reports,"preparedCurveSlots":prepared,"playbackAccepted":false,
            "limitations":["no native Animator sampling or initial phase validation","no additive reference pose execution","no source pixel comparison"]});
        std::fs::write(
            std::env::var_os("MOLY_WEATHER_ANIMATION_OUT").expect("output path"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}

fn exact_effect_owner<'a>(effect: &'a Value, owner: &Identity) -> Option<&'a str> {
    let root = Identity::read(&effect["source"]).ok()?;
    if !root.same_file(owner) {
        return None;
    }
    let mut matches = effect["nodes"]
        .as_array()?
        .iter()
        .filter(|node| node["gameObjectPathId"].as_str() == Some(&owner.path_id));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    first["path"].as_str()
}

/// Recompose complete parent chains from local transforms, once an animation
/// evaluator has supplied qualified local TRS values. This helper never blends
/// Euler channels or guesses reference poses, and never accumulates frame deltas.
#[allow(dead_code)]
pub(crate) struct Hierarchy {
    nodes: HashMap<String, (Option<String>, Transform)>,
}

#[allow(dead_code)]
impl Hierarchy {
    pub(crate) fn read(nodes: &[Value]) -> Result<Self, String> {
        let mut result = Self {
            nodes: HashMap::new(),
        };
        for node in nodes {
            let path = string(node, "path")?.to_owned();
            let vector = |key: &str, count: usize| -> Result<Vec<f32>, String> {
                let values = array(node, key)?;
                if values.len() != count {
                    return Err(format!("invalid {key} width"));
                }
                values.iter().map(number).collect()
            };
            let p = vector("position", 3)?;
            let q = vector("rotation", 4)?;
            let s = vector("scale", 3)?;
            let transform = Transform {
                translation: Vec3::new(-p[0], p[1], p[2]),
                rotation: Quat::from_xyzw(q[0], -q[1], -q[2], q[3]),
                scale: Vec3::new(s[0], s[1], s[2]),
            };
            let parent = match &node["parent"] {
                Value::Null => None,
                Value::String(p) => Some(p.clone()),
                _ => return Err("invalid parent path".into()),
            };
            if result.nodes.insert(path, (parent, transform)).is_some() {
                return Err("duplicate hierarchy path".into());
            }
        }
        result.affines(&HashMap::new())?;
        Ok(result)
    }

    pub(crate) fn affines(
        &self,
        local_overrides: &HashMap<String, Transform>,
    ) -> Result<HashMap<String, GlobalTransform>, String> {
        if local_overrides
            .keys()
            .any(|path| !self.nodes.contains_key(path))
        {
            return Err("local animation target absent from source hierarchy".into());
        }
        let mut result = HashMap::new();
        for path in self.nodes.keys() {
            self.compose(path, local_overrides, &mut HashSet::new(), &mut result)?;
        }
        Ok(result)
    }

    fn compose(
        &self,
        path: &str,
        overrides: &HashMap<String, Transform>,
        visiting: &mut HashSet<String>,
        result: &mut HashMap<String, GlobalTransform>,
    ) -> Result<GlobalTransform, String> {
        if let Some(affine) = result.get(path) {
            return Ok(*affine);
        }
        if !visiting.insert(path.into()) {
            return Err("cycle in source animation hierarchy".into());
        }
        let (parent, local) = self
            .nodes
            .get(path)
            .ok_or("source animation parent missing")?;
        let parent = match parent {
            Some(parent) => self.compose(parent, overrides, visiting, result)?,
            None => GlobalTransform::IDENTITY,
        };
        let affine = parent * GlobalTransform::from(*overrides.get(path).unwrap_or(local));
        visiting.remove(path);
        result.insert(path.into(), affine);
        Ok(affine)
    }
}
