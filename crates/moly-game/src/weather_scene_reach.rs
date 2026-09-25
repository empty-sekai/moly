//! Opt-in diagnostic: the reach of the order refusal over every shipped
//! weather change at every site. Coverage over a caller-supplied extraction,
//! NOT a source correctness test; no game data, private paths or fixed counts
//! are embedded here.
//!
//! A weather change stops the old phenomenon's effects (the site effect
//! always, the sky and camera effects when the new phenomenon installs other
//! ones) and installs the new ones; the stopped effects' systems keep
//! simulating without emitting and their colliders stay in the physics scene
//! until each effect's own destroy delay has run out. During that window
//! every collision query can meet both phenomena's ground colliders. This
//! runs, for every site and every ordered pair of phenomena whose window
//! holds two ground colliders and at least one admitted collision system,
//! the old phenomenon's admitted collision systems to a steady state with
//! its own scene, then the window with both scenes: the old systems stopped,
//! the new ones installed (their first-Play warm included), at the product's
//! frame step. It counts, over the window, the particle queries whose box
//! meets more than one collider that gives different answers, those where
//! more than one such collider hits, the lanes the law selected over every
//! order (all orders agreeing) and the lanes it refuses as order dependent.
//! A refused call is continued in the scene's listed order for the count
//! only.
use super::*;
use crate::particle_runtime::collision_scene::{GroundQuery, Installation, SceneBuilder, SiteScene};
use bevy::asset::AssetPlugin;
use bevy::image::ImagePlugin;
use moly_law::particle::collision_query::{Candidate, CollisionScene, OverlapQuery, SweepHit, SweepRequest};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The product's frame step for the count.
const FRAME: f32 = 1.0 / 60.0;

#[derive(Default, Clone, Copy, Debug)]
struct Reach {
    /// Particle queries that swept at least one collider.
    lanes: u64,
    /// Queries whose box met more than one collider giving different answers.
    distinct_candidates: u64,
    /// Queries hit by more than one collider giving different answers.
    distinct_hits: u64,
}

/// The live scene with the per-lane count of the colliders each query meets.
struct Probe {
    inner: GroundQuery,
    reach: Arc<Mutex<Reach>>,
    counting: bool,
    lanes: BTreeMap<u32, Vec<(usize, bool)>>,
}

impl Probe {
    fn flush(&mut self) {
        let mut reach = self.reach.lock().unwrap();
        for swept in self.lanes.values() {
            let mut classes: Vec<usize> = Vec::new();
            let mut hit_classes: Vec<usize> = Vec::new();
            for &(shape, hit) in swept {
                if !classes.iter().any(|&c| self.inner.same_answers(c, shape)) {
                    classes.push(shape);
                }
                if hit && !hit_classes.iter().any(|&c| self.inner.same_answers(c, shape)) {
                    hit_classes.push(shape);
                }
            }
            reach.lanes += 1;
            reach.distinct_candidates += u64::from(classes.len() > 1);
            reach.distinct_hits += u64::from(hit_classes.len() > 1);
        }
        self.lanes.clear();
    }
}

/// The probe behind the scene boundary. The law asks `order_known` once per
/// call it runs with the scene's own answer (it takes `&self`, hence the
/// lock); the listed-order continuation of a refused call does not ask, so
/// its sweeps are not counted twice.
struct Counted(Mutex<Probe>);

impl CollisionScene for Counted {
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate> {
        let probe = self.0.get_mut().unwrap();
        probe.flush();
        probe.counting = false;
        probe.inner.overlap(query)
    }
    fn reachable(&self, collides_with: u32) -> Vec<Candidate> {
        self.0.lock().unwrap().inner.reachable(collides_with)
    }
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit> {
        let probe = self.0.get_mut().unwrap();
        let hit = probe.inner.sweep_sphere(request);
        if probe.counting {
            probe.lanes.entry(request.particle).or_default().push((request.shape, hit.is_some()));
        }
        hit
    }
    fn refusal(&self) -> Option<&'static str> {
        self.0.lock().unwrap().inner.refusal()
    }
    fn order_known(&self) -> bool {
        let mut probe = self.0.lock().unwrap();
        probe.counting = true;
        probe.inner.order_known()
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        if let Ok(probe) = self.0.get_mut() {
            probe.flush();
        }
    }
}

/// One admitted collision system of a plan.
struct System {
    planned: Planned,
    /// The effect's own destroy delay after Stop.
    destroy_delay: f64,
    /// The effect is a sky or camera effect (kept across a change that
    /// installs the same one).
    global: bool,
}

/// One phenomenon's plan at one site: its selected effects, their colliders
/// and its admitted collision systems.
struct SitePlan {
    effects: Vec<String>,
    colliders: Vec<Arc<crate::particle_runtime::collision_scene::GroundScene>>,
    systems: Vec<System>,
    /// Collision systems refused, with the reason.
    refused: Vec<String>,
    /// Admitted collision systems the count cannot run (named).
    unrun: Vec<String>,
}

fn runtime_for(planned: &Planned, seeds: &mut crate::particle_runtime::seed::SystemSeedManager, site: &SiteScene,
    reach: &Arc<Mutex<Reach>>) -> Result<Runtime, String> {
    let e = &planned.emitter;
    let mut system = crate::particle_runtime::test_support::runtime();
    system.pool.clear();
    system.side.clear();
    system.born_total = 0;
    system.node = planned.node.clone();
    system.effect = planned.effect.clone();
    system.kind = planned.kind;
    system.camera_rotation = planned.camera_rotation;
    system.node_affine = planned.node_affine;
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
    let probe = Probe { inner: GroundQuery::live(site.clone()), reach: reach.clone(), counting: false, lanes: BTreeMap::new() };
    let collision = crate::particle_runtime::CollisionInstall { scene: Box::new(Counted(Mutex::new(probe))),
        owner: planned.collision_owner };
    match crate::particle_runtime::install_native_birth(&mut system, seeds, &planned.route, Some(collision)) {
        Ok(crate::particle_runtime::BirthPath::Native) => {}
        Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => return Err(format!("legacy path: {reason}")),
        Err(error) => return Err(error.to_string()),
    }
    // Events are recorded and counted as installed; no child is simulated.
    if let Some(edges) = planned.event_edges.clone() {
        if let Some(collision) = system.collision.as_mut() {
            collision.attach_edges(edges.collisions.clone());
        }
        system.native_birth.as_mut().expect("native birth owner just installed").events =
            Some(crate::particle_runtime::BirthEvents::with_edges(edges));
    }
    system.collision.as_mut().ok_or("installed without its collision state")?.listed_order_fallback = true;
    Ok(system)
}

#[test]
#[ignore = "requires MOLY_WEATHER_AUDIT_INDEX, MOLY_WEATHER_AUDIT_COLLISION and MOLY_WEATHER_REACH_OUT"]
fn transition_order_reach() {
    let index_path = std::path::PathBuf::from(std::env::var_os("MOLY_WEATHER_AUDIT_INDEX").expect("supply extraction index"));
    let output = std::path::PathBuf::from(std::env::var_os("MOLY_WEATHER_REACH_OUT").expect("supply diagnostic output"));
    let root = index_path.parent().expect("index has a parent");
    let overlay = std::env::var_os("MOLY_SOURCE_CORPUS_OVERLAY").map(std::path::PathBuf::from);
    let read = |path: &std::path::Path| -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("read source extraction")).expect("parse source extraction")
    };
    let source_path = |file: &str| overlay.as_ref().map(|root| root.join(file)).filter(|path| path.is_file())
        .unwrap_or_else(|| root.join(file));
    let index = read(&index_path);
    let collision = std::path::PathBuf::from(std::env::var_os("MOLY_WEATHER_AUDIT_COLLISION").expect("supply the collider export"));
    let mut scenes = SceneBuilder::new(Ok(read(&collision)));
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
    let phenomena: Vec<(String, Value, Option<Value>)> = index["phenomena"].as_object().expect("phenomena map").iter()
        .map(|(name, item)| {
            let doc = read(&source_path(item["fx"]["file"].as_str().expect("fx file")));
            let animations = item["animations"]["file"].as_str().map(|file| read(&source_path(file)))
                .or_else(|| overlay.as_ref().map(|root| root.join(name).join("animations.json")).filter(|path| path.is_file()).map(|path| read(&path)));
            (name.clone(), doc, animations)
        }).collect();
    let sites: std::collections::BTreeSet<String> = phenomena.iter().flat_map(|(_, doc, _)| {
        doc["effects"].as_object().expect("effects map").values()
            .filter_map(|effect| effect["variant"].as_str().and_then(|v| v.strip_prefix("unique__")).map(str::to_owned))
            .collect::<Vec<_>>()
    }).collect();
    // Every phenomenon's plan at every site: the plan's selection, its
    // effects' colliders and its admitted collision systems.
    let mut plans: BTreeMap<(String, String), SitePlan> = BTreeMap::new();
    for site in &sites {
        for (name, doc, animations) in &phenomena {
            let effects = doc["effects"].as_object().expect("effects map");
            let selected = selected_effects(effects, site);
            let names: Vec<&str> = selected.iter().map(|(n, _)| n.as_str()).collect();
            scenes.select(Installation::Together(&names));
            let mut plan = SitePlan { effects: names.iter().map(|n| (*n).to_owned()).collect(), colliders: Vec::new(),
                systems: Vec::new(), refused: Vec::new(), unrun: Vec::new() };
            for (effect_name, effect) in &selected {
                plan.colliders.push(scenes.effect_colliders(effect_name).expect("effect colliders"));
                let kind = match effect["kind"].as_str() {
                    Some("sky") => EffectKind::Sky,
                    Some("camera") => EffectKind::Camera,
                    Some("site") => EffectKind::Site,
                    _ => continue,
                };
                let lifecycle = WeatherEffectLifecycle::from_effect(effect).expect("source lifecycle metadata");
                let animation = crate::weather_animation::Contract::compile(effect, animations.as_ref());
                let by_path: HashMap<String, &Value> = effect["nodes"].as_array().expect("nodes")
                    .iter().map(|n| (n["path"].as_str().expect("node path").to_owned(), n)).collect();
                let particles = effect["particles"].as_array().expect("particles");
                let owners = source_sub_emitter_owners(particles);
                let ground = scenes.for_effect(effect_name);
                for particle in particles.iter().filter(|p| p["system"]["collision"].is_object()) {
                    let node = particle["node"].as_str().unwrap_or("");
                    if let Some(reason) = animation.refusal(node) {
                        plan.refused.push(format!("{effect_name}/{node}: {reason}"));
                        continue;
                    }
                    let mut tally = Tally::default();
                    match judge(effect_name, particle, &by_path, &owners, kind, effect["effectiveRotation"].as_str() == Some("normal"),
                        lifecycle, &ground, server, &mut tally) {
                        Some(planned) if planned.collision_scene.is_some() && planned.child_owner.is_none() => {
                            if planned.emission_surface.is_some() {
                                plan.unrun.push(format!("{effect_name}/{node}: mesh emission surface not loaded here"));
                            } else {
                                plan.systems.push(System { planned, destroy_delay: lifecycle.time_until_destroy(),
                                    global: kind != EffectKind::Site });
                            }
                        }
                        Some(_) => plan.unrun.push(format!("{effect_name}/{node}: admitted without a collision install")),
                        None => plan.refused.push(format!("{effect_name}/{node}: {tally:?}")),
                    }
                }
            }
            plans.insert((site.clone(), name.clone()), plan);
        }
    }
    let ctx = crate::particle_runtime::Context {
        sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY,
    };
    let mut seeds = crate::particle_runtime::seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    let mut rows = Vec::new();
    let mut totals = json!({"transitions": 0, "simulated": 0, "oneColliderAtMost": 0, "noCollisionSystem": 0,
        "lanes": 0, "distinctCandidates": 0, "distinctHits": 0, "orderFreeLanes": 0, "orderDependentLanes": 0});
    let bump = |totals: &mut Value, key: &str, by: u64| { totals[key] = json!(totals[key].as_u64().unwrap() + by); };
    for site in &sites {
        for (a, _, _) in &phenomena {
            for (b, _, _) in &phenomena {
                if a == b {
                    continue;
                }
                bump(&mut totals, "transitions", 1);
                let (pa, pb) = (&plans[&(site.clone(), a.clone())], &plans[&(site.clone(), b.clone())]);
                let colliders = pa.colliders.iter().chain(&pb.colliders).map(|c| c.collider_count()).sum::<usize>();
                if colliders < 2 {
                    bump(&mut totals, "oneColliderAtMost", 1);
                    continue;
                }
                if pa.systems.is_empty() && pb.systems.is_empty() {
                    bump(&mut totals, "noCollisionSystem", 1);
                    continue;
                }
                bump(&mut totals, "simulated", 1);
                // The old phenomenon at its steady state with its own scene.
                let scene = SiteScene::default();
                for c in &pa.colliders {
                    scene.install(c.clone());
                }
                let reach = Arc::new(Mutex::new(Reach::default()));
                let mut old: Vec<(Runtime, f64, bool)> = Vec::new();
                let mut errors = Vec::new();
                for system in &pa.systems {
                    match runtime_for(&system.planned, &mut seeds, &scene, &reach) {
                        Ok(runtime) => old.push((runtime, system.destroy_delay,
                            system.global && pb.effects.contains(&system.planned.effect))),
                        Err(reason) => errors.push(format!("{}: {reason}", system.planned.node)),
                    }
                }
                let steady = old.iter().map(|(r, _, _)| match r.emitter.start.lifetime {
                    MinMaxCurve::Constant(v) => v,
                    MinMaxCurve::TwoConstants { max, .. } => max,
                    _ => 5.0,
                }).fold(0.0f32, f32::max).min(20.0) + 1.0;
                for (runtime, _, _) in &mut old {
                    if runtime.emitter.prewarm && runtime.emitter.looping {
                        let _ = crate::particle_runtime::prewarm_first_play(runtime, &ctx);
                    }
                    runtime.prewarmed = true;
                }
                let mut t = 0.0f32;
                while t < steady {
                    for (runtime, _, _) in &mut old {
                        crate::particle_runtime::advance_frame(runtime, FRAME, &ctx, true);
                    }
                    t += FRAME;
                }
                // Count only the window.
                let (before_free, before_dependent): (u64, u64) = old.iter().filter_map(|(r, _, _)| r.collision.as_ref())
                    .fold((0, 0), |(f, d), c| (f + c.order_free, d + c.order_dependent));
                *reach.lock().unwrap() = Reach::default();
                // The change: the new effects' colliders join; the old site
                // effect (and a sky or camera effect the new plan replaces)
                // stops and is destroyed after its own delay.
                for c in &pb.colliders {
                    scene.install(c.clone());
                }
                let mut new: Vec<Runtime> = Vec::new();
                for system in &pb.systems {
                    if system.global && pa.effects.contains(&system.planned.effect) {
                        continue;
                    }
                    match runtime_for(&system.planned, &mut seeds, &scene, &reach) {
                        Ok(mut runtime) => {
                            if runtime.emitter.prewarm && runtime.emitter.looping {
                                let _ = crate::particle_runtime::prewarm_first_play(&mut runtime, &ctx);
                            }
                            runtime.prewarmed = true;
                            new.push(runtime);
                        }
                        Err(reason) => errors.push(format!("{}: {reason}", system.planned.node)),
                    }
                }
                let window = pa.systems.iter().map(|s| s.destroy_delay).chain(std::iter::once(2.0f64))
                    .fold(0.0f64, f64::max);
                let mut t = 0.0f64;
                while t < window {
                    for (runtime, delay, kept) in &mut old {
                        if *kept || t < *delay {
                            crate::particle_runtime::advance_frame(runtime, FRAME, &ctx, *kept);
                        }
                    }
                    for runtime in &mut new {
                        crate::particle_runtime::advance_frame(runtime, FRAME, &ctx, true);
                    }
                    t += f64::from(FRAME);
                }
                let (free, dependent, refused, calls, hits) = old.iter().map(|(r, _, _)| r).chain(&new)
                    .fold((0u64, 0u64, 0u64, 0u64, 0u64), |(f, d, x, c, h), r| {
                        let col = r.collision.as_ref();
                        (f + col.map_or(0, |c| c.order_free), d + col.map_or(0, |c| c.order_dependent),
                            x + r.refused_total, c + col.map_or(0, |c| c.calls), h + col.map_or(0, |c| c.hits))
                    });
                let free = free - before_free;
                let dependent = dependent - before_dependent;
                // Dropping the systems flushes the probes' last calls.
                drop(old);
                drop(new);
                let counted = *reach.lock().unwrap();
                bump(&mut totals, "lanes", counted.lanes);
                bump(&mut totals, "distinctCandidates", counted.distinct_candidates);
                bump(&mut totals, "distinctHits", counted.distinct_hits);
                bump(&mut totals, "orderFreeLanes", free);
                bump(&mut totals, "orderDependentLanes", dependent);
                rows.push(json!({
                    "site": site, "from": a, "to": b, "groundColliders": colliders,
                    "fromSystems": pa.systems.iter().map(|s| s.planned.node.clone()).collect::<Vec<_>>(),
                    "toSystems": pb.systems.iter().map(|s| s.planned.node.clone()).collect::<Vec<_>>(),
                    "windowSeconds": window, "steadySeconds": steady,
                    "lanes": counted.lanes, "distinctCandidates": counted.distinct_candidates,
                    "distinctHits": counted.distinct_hits, "orderFreeLanes": free, "orderDependentLanes": dependent,
                    "callsTotal": calls, "hitsTotal": hits, "refusedFrames": refused, "installErrors": errors,
                }));
            }
        }
    }
    let plan_rows: Vec<Value> = plans.iter().map(|((site, name), plan)| json!({
        "site": site, "phenomenon": name, "effects": plan.effects,
        "groundColliders": plan.colliders.iter().map(|c| c.collider_count()).sum::<usize>(),
        "collisionSystems": plan.systems.iter().map(|s| format!("{}/{}", s.planned.effect, s.planned.node)).collect::<Vec<_>>(),
        "refused": plan.refused, "unrun": plan.unrun,
    })).collect();
    println!("transition order reach: {totals}");
    std::fs::write(output, serde_json::to_vec_pretty(&json!({
        "meaning": "Order-refusal reach over every ordered weather change at every site, at the product frame step; the old phenomenon runs to a steady state first. Coverage, not source equivalence.",
        "frameSeconds": FRAME, "totals": totals, "transitions": rows, "plans": plan_rows,
    })).unwrap()).expect("write reach report");
}
