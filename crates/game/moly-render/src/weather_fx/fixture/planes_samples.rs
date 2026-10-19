//! Diagnostic, not a correctness test: every Planes-type collision system of
//! the supplied fixture docs, judged by this host's Director path, then
//! restarted and stepped the way a Director's ControlPlayable drives it
//! (`director_restart`, then `director_chunk`), with its CollisionModule
//! installed from its plane slots. The instance stands at two arbitrary site
//! poses, one metre and five centimetres above the world origin (the plane of
//! the fixture effect's slots is the world plane y = 0); the seed is the
//! system's own. One line per system, pose and second of play: the birth
//! path, the live count, the births and deaths, and the collision calls and
//! hits.
use super::*;

#[test]
#[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing exported source fixture particles"]
fn fixture_planes_collision_systems_simulate() {
    let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
    let mut app = App::new();
    moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir { path: root.clone() });
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::image::ImagePlugin::default()));
    moly_assets::source_shader::loader::register(&mut app);
    app.init_asset::<Gltf>();
    app.finish(); app.cleanup();
    let server = app.world().resource::<AssetServer>();
    let mut entries: Vec<_> = std::fs::read_dir(root.join("fixture-particles-v2")).unwrap()
        .map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    let (mut systems, mut admitted, mut stepped) = (0usize, 0usize, 0usize);
    for path in entries {
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let Some(particles) = doc["emitters"].as_array() else { continue; };
        let Some(nodes) = doc["nodes"].as_array() else { continue; };
        let by_path: HashMap<String, &Value> = nodes.iter()
            .filter_map(|node| Some((node["node"].as_str()?.to_owned(), node))).collect();
        let owners = source_sub_emitter_owners(particles);
        let package = doc["name"].as_str().unwrap_or("fixture");
        for particle in particles {
            if particle["system"]["collision"]["type"] != "planes" { continue; }
            systems += 1;
            let node = particle["node"].as_str().unwrap_or("");
            let plan = match admit(package, particle, &by_path, nodes, &owners, server, Path::Control, Stepping::Director) {
                Ok(plan) => plan,
                Err(reason) => {
                    println!("planes-sim | {file} | {node} | refused | {}", reason.strip_prefix(node).unwrap_or(&reason));
                    continue;
                }
            };
            admitted += 1;
            for height in [1.0f32, 0.05] {
            let ctx = Context { site: GlobalTransform::from_translation(Vec3::new(0.0, height, 0.0)),
                sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY };
            let mut system = runtime(&plan, Entity::PLACEHOLDER, Handle::default());
            system.emitter.random_seed = Some(system.emitter.random_seed.unwrap_or(0));
            system.emitter.auto_random_seed = Some(false);
            let birth = match crate::particle_runtime::director_restart(&mut system, &plan.route, plan.event_edges.as_ref(), &ctx) {
                Ok(crate::particle_runtime::BirthPath::Native) => "native".to_owned(),
                Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => format!("legacy: {reason}"),
                Err(reason) => { println!("planes-sim | {file} | {node} | y {height} | restart refused | {reason}"); continue; }
            };
            let planes = system.collision.as_ref().map(|collision| collision.planes.as_ref().map_or(0, Vec::len));
            println!("planes-sim | {file} | {node} | y {height} | admitted | birth {birth} | collision {} | plane slots {planes:?} | space {:?}",
                system.collision.is_some(), system.emitter.simulation_space);
            let mut stop = false;
            let mut failed = None;
            for frame in 1..=600u32 {
                let emitting = !stop;
                let result = crate::particle_runtime::director_chunk(&mut system, 1.0 / 60.0, emitting, &ctx, |s| {
                    if !s.emitter.looping && s.playback_head >= s.emitter.duration { stop = true; }
                });
                if let Err(reason) = result { failed = Some((frame, reason)); break; }
                if frame % 60 == 0 {
                    let (calls, hits) = system.collision.as_ref().map_or((0, 0), |c| (c.calls, c.hits));
                    let lowest = system.pool.iter().map(|p| p.position[1]).fold(f32::INFINITY, f32::min);
                    println!("planes-sim | {file} | {node} | y {height} | t {:.0} s | live {} | born {} | died {} | collision calls {calls} hits {hits} | lowest y {lowest}",
                        frame as f32 / 60.0, system.pool.len(), system.born_total, system.died_total);
                }
            }
            match failed {
                Some((frame, reason)) => println!("planes-sim | {file} | {node} | y {height} | step refused at frame {frame} | {reason}"),
                None => stepped += 1,
            }
            }
        }
    }
    println!("planes-sim systems {systems} admitted {admitted} stepped {stepped}");
    assert!(systems > 0, "the supplied fixture docs hold no Planes-type collision system");
}
