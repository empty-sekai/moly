//! Fixture adapter for the same source-owned geometry, materials and admission
//! used by weather. A GLB emitter entity owns the already-converted transform;
//! never recompose or reflect its exported node TRS a second time.
use super::*;

struct Candidate {
    anchor: Entity,
    plan: Planned,
}

#[derive(Component)]
pub(crate) struct Request(Vec<Candidate>);

pub(crate) fn is_source_particle(particle: &Value) -> bool {
    particle.pointer("/renderer/material").is_some_and(|material|
        material.get("sourceMaterial").is_some() || material.get("shaderProgram").is_some()
            || material.get("sourceError").is_some())
        || particle.pointer("/renderer/geometryError").is_some()
}

pub(crate) fn plan(
    commands: &mut Commands,
    root: Entity,
    doc: &Value,
    anchors: &HashMap<String, Vec<Entity>>,
    server: &AssetServer,
) {
    let Some(particles) = doc["emitters"].as_array() else { return; };
    let Some(nodes) = doc["nodes"].as_array() else { return; };
    let by_path: HashMap<String, &Value> = nodes.iter().filter_map(|node|
        Some((node["node"].as_str()?.to_owned(), node))).collect();
    let owners = source_sub_emitter_owners(particles);
    let package = doc["name"].as_str().unwrap_or("fixture");
    let mut candidates = Vec::new();
    for (ordinal, particle) in particles.iter().enumerate().filter(|(_, p)| is_source_particle(p)) {
        let node = particle["node"].as_str().unwrap_or("");
        if particle["activeInHierarchy"] != true || particle["system"]["playOnAwake"] != true {
            continue;
        }
        if let Some(error) = particle.pointer("/renderer/material/sourceError").and_then(Value::as_str)
            .or_else(|| particle.pointer("/renderer/geometryError").and_then(Value::as_str)) {
            warn!("[fixture-source] {package}/{node}: source export failed: {error}"); continue;
        }
        let Some(anchor) = anchors.get(&format!("/{node}")).and_then(|list| list.first()).copied() else {
            warn!("[fixture-source] {package}/{node}: emitter instance missing"); continue;
        };
        let mut tally = Tally::default();
        match judge_in_archive(package, particle, &by_path, &owners, EffectKind::Site, false,
            None, "fixture-particles-v2", Some(GlobalTransform::IDENTITY), server, &mut tally) {
            Some(mut plan) => {
                plan.ordinal = ordinal;
                candidates.push(Candidate { anchor, plan });
            }
            None => warn!("[fixture-source] {package}/{node}: rejected {tally:?}"),
        }
    }
    if !candidates.is_empty() {
        info!("[fixture-source] {package}: {} source emitters awaiting geometry/GPU", candidates.len());
        commands.entity(root).insert(Request(candidates));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn incomplete_source_contract_never_becomes_a_legacy_material() {
        assert!(!is_source_particle(&json!({"renderer":{"material":{"shader":"legacy"}}})));
        for material in [json!({"sourceMaterial":null}), json!({"shaderProgram":{}}),
            json!({"sourceError":"unresolved source shader"})] {
            assert!(is_source_particle(&json!({"renderer":{"material":material}})));
        }
        assert!(is_source_particle(&json!({"renderer":{"geometryError":"missing source mesh"}})));
    }

    #[test]
    #[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing freshly exported source fixture particles"]
    fn source_fixture_mesh_admission_and_bursts() {
        let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
        let mut app = App::new();
        moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir { path: root.clone() });
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::image::ImagePlugin::default()));
        moly_assets::source_shader::loader::register(&mut app);
        app.init_asset::<Gltf>();
        app.finish(); app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let mut mesh_count = 0;
        for entry in std::fs::read_dir(root.join("fixture-particles-v2")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("json")
                || !path.file_name().unwrap().to_string_lossy().contains("fountain") { continue; }
            let doc: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let particles = doc["emitters"].as_array().unwrap();
            let by_path = doc["nodes"].as_array().unwrap().iter()
                .map(|node| (node["node"].as_str().unwrap().to_owned(), node)).collect();
            let owners = source_sub_emitter_owners(particles);
            for particle in particles {
                if particle["renderer"]["renderMode"] != "Mesh" || particle["renderer"]["enabled"] != true
                    || particle["activeInHierarchy"] != true || particle["system"]["playOnAwake"] != true { continue; }
                let node = particle["node"].as_str().unwrap();
                let mut tally = Tally::default();
                let planned = judge_in_archive("fixture", particle, &by_path, &owners, EffectKind::Site, false,
                    None, "fixture-particles-v2", Some(GlobalTransform::IDENTITY), server, &mut tally);
                assert!(planned.is_some(), "{node}: {tally:?}");
                let planned = planned.unwrap();
                assert!(planned.source.catalogue.path().unwrap().path().starts_with("fixture-particles-v2"));
                assert!(matches!(planned.geometry, PlannedGeometry::Mesh { .. }));
                // Exercise real source emission and curves independently of GPU readiness.
                let mut runtime = crate::particle_runtime::test_support::runtime();
                runtime.emitter = planned.emitter;
                runtime.pool.clear(); runtime.side.clear(); runtime.born_total = 0;
                runtime.kind = EffectKind::Site;
                runtime.cone_angle = planned.cone_angle;
                runtime.rol = planned.rol; runtime.limit = planned.limit;
                runtime.velocity_law = runtime.emitter.velocity_over_lifetime.as_ref()
                    .map(moly_law::particle::velocity::VelocityOverLifetime::from_params);
                runtime.size_law = runtime.emitter.size_over_lifetime.as_ref()
                    .map(moly_law::particle::size::SizeOverLifetime::from_params);
                runtime.color_law = runtime.emitter.color_over_lifetime.as_ref()
                    .map(moly_law::particle::color::ColorOverLifetime::from_params);
                let context = Context { site: GlobalTransform::IDENTITY, sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY };
                for _ in 0..60 { simulate(&mut runtime, 1.0 / 60.0, &context); }
                assert!(runtime.born_total > 0, "{node}: source emitter never emitted");
                assert_eq!(runtime.refused_total, 0, "{node}: source simulation refused");
                for quad in crate::particle_runtime::build_quads(&runtime, &GlobalTransform::IDENTITY) {
                    assert!(quad.centre.is_finite() && quad.size.is_finite());
                    assert!(quad.custom1.iter().chain(quad.custom2.iter()).all(|v| v.is_finite()), "{node}: nonfinite custom stream");
                }
                println!("{node}: mesh source admitted, born={}, alive={}", runtime.born_total, runtime.pool.len());
                mesh_count += 1;
            }
        }
        assert!(mesh_count > 0, "source fixture corpus did not contain any active Mesh emitters");
    }
}

fn load_mesh(
    reference: &ParticleMeshReference,
    handle: &Handle<Gltf>,
    server: &AssetServer,
    gltfs: &Assets<Gltf>,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Result<Option<Arc<crate::particle_geometry::SourceMesh>>, String> {
    match (server.load_state(handle), server.recursive_dependency_load_state(handle)) {
        (LoadState::Failed(error), _) => return Err(format!("{}: {error}", reference.file)),
        (_, RecursiveDependencyLoadState::Failed(error)) => return Err(format!("{} dependency: {error}", reference.file)),
        _ => {}
    }
    if !server.is_loaded_with_dependencies(handle) { return Ok(None); }
    let Some(gltf) = gltfs.get(handle) else { return Ok(None); };
    let node_handle = gltf.named_nodes.get(reference.node.as_str())
        .ok_or_else(|| format!("{} lacks source mesh node {}", reference.file, reference.node))?;
    let Some(node) = nodes.get(node_handle) else { return Ok(None); };
    let mesh_handle = node.mesh.as_ref().ok_or("source particle node lacks mesh")?;
    let Some(gltf_mesh) = gltf_meshes.get(mesh_handle) else { return Ok(None); };
    let Some(primitives) = gltf_mesh.primitives.iter().map(|p| meshes.get(&p.mesh)).collect::<Option<Vec<_>>>() else {
        return Ok(None);
    };
    crate::particle_geometry::SourceMesh::from_primitives(&primitives, Vec3::from_array(reference.size()))
        .map(|mesh| Some(Arc::new(mesh)))
}

fn prepare_geometry(
    plan: &mut Planned,
    server: &AssetServer,
    gltfs: &Assets<Gltf>,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Result<bool, String> {
    if let Some(surface) = &mut plan.emission_surface {
        if surface.source.is_none() {
            let Some(mesh) = load_mesh(&surface.reference, &surface.glb, server, gltfs, nodes, gltf_meshes, meshes)? else {
                return Ok(false);
            };
            surface.source = Some(Arc::new(crate::particle_mesh_emission::EmissionSurface::from_source(&mesh)?));
        }
    }
    if let PlannedGeometry::Mesh { reference, glb, source, .. } = &mut plan.geometry {
        if source.is_none() {
            *source = load_mesh(reference, glb, server, gltfs, nodes, gltf_meshes, meshes)?;
            if source.is_none() { return Ok(false); }
        }
    }
    Ok(true)
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut requests: Query<(Entity, &mut Request)>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    catalogues: Res<Assets<SourceShaderCatalogue>>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
) {
    for (root, mut request) in &mut requests {
        let mut pending = Vec::new();
        for mut candidate in std::mem::take(&mut request.0) {
            let planned = &mut candidate.plan;
            let preparation = prepare_geometry(planned, &server, &gltfs, &nodes, &gltf_meshes, &meshes)
                .and_then(|ready| if ready { planned.source.resolve(&server, &catalogues).map_err(|e| e.0) } else { Ok(false) });
            match preparation {
                Ok(false) => { pending.push(candidate); continue; }
                Err(error) => {
                    warn!("[fixture-source] {}: {error}", planned.node);
                    if let Some((draw, _)) = planned.draw { commands.entity(draw).try_despawn(); }
                    continue;
                }
                Ok(true) => {}
            }
            if planned.draw.is_none() {
                let mesh = meshes.add(billboard::empty_mesh());
                let draw = commands.spawn((Mesh3d(mesh.clone()), planned.source.clone(), Transform::IDENTITY,
                    NoFrustumCulling, crate::shadowmap::NoShadowCast, ChildOf(root))).id();
                planned.draw = Some((draw, mesh));
            }
            let readiness = planned.source.readiness.lock().unwrap().clone();
            match readiness {
                ParticleReadiness::Pending => { pending.push(candidate); continue; }
                ParticleReadiness::Failed(error) => {
                    warn!("[fixture-source] {} GPU: {error}", planned.node);
                    commands.entity(planned.draw.as_ref().unwrap().0).try_despawn();
                    continue;
                }
                ParticleReadiness::Ready => {}
            }
            let Candidate { anchor, plan: planned } = candidate;
            let (draw, mesh) = planned.draw.clone().expect("prepared source draw");
            let runtime = runtime(&planned, anchor, mesh);
            let mut source = planned.source;
            source.enabled = true;
            commands.entity(draw).insert((source, crate::uber_particle::FixtureParticleLive(runtime)));
            info!("[fixture-source] {}: installed", planned.node);
        }
        request.0 = pending;
        if request.0.is_empty() { commands.entity(root).remove::<Request>(); }
    }
}

fn runtime(planned: &Planned, anchor: Entity, mesh: Handle<Mesh>) -> Runtime {
    let geometry = match &planned.geometry {
        PlannedGeometry::Billboard(draw) => crate::particle_runtime::Geometry::SourceBillboard(draw.clone()),
        PlannedGeometry::Mesh { source, alignment, scaling, pivot, .. } => crate::particle_runtime::Geometry::Mesh(
            crate::particle_geometry::MeshDraw { source: source.clone().expect("prepared source mesh"),
                alignment: *alignment, scaling: *scaling, pivot: *pivot }),
    };
    Runtime {
        node: planned.node.clone(), effect: planned.effect.clone(), emitter: planned.emitter.clone(),
        kind: EffectKind::Site, camera_rotation: false, node_affine: GlobalTransform::IDENTITY,
        mesh, anchor: Some(anchor), geometry,
        emission_surface: planned.emission_surface.as_ref().map(|surface| surface.source.clone().expect("prepared source surface")),
        ring_cursor: 0, pool: Vec::new(), side: Vec::new(), emission: EmissionState::default(),
        playback_head: 0.0, previous_head: 0.0, emission_started: false,
        rng: Rng(RNG_SEED ^ (planned.ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        prewarmed: false, cone_angle: planned.cone_angle, rol: planned.rol.clone(), limit: planned.limit.clone(),
        velocity_law: planned.emitter.velocity_over_lifetime.as_ref().map(moly_law::particle::velocity::VelocityOverLifetime::from_params),
        size_law: planned.emitter.size_over_lifetime.as_ref().map(moly_law::particle::size::SizeOverLifetime::from_params),
        color_law: planned.emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params),
        born_total: 0, died_total: 0, full_total: 0, refused_total: 0,
    }
}
