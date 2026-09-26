//! Fixture adapter for the same source-owned geometry, materials and admission
//! used by weather. A GLB emitter entity owns the already-converted transform;
//! never recompose or reflect its exported node TRS a second time.
//!
//! A system reached by `ParticleSystem.Play` (a placed prefab's play-on-awake
//! systems, and the explicit Play of the site move and the foot effects) runs
//! as the weather host runs its systems: its Play installs the native birth
//! owner where the route and modules qualify (seed reset, Noise, birth
//! events), and every frame steps the engine's per-frame update (the frame
//! head with the emitter velocity and the emission over distance, then the
//! incremental slices) on the clock its useUnscaledTime selects, under the
//! engine play state (Stop with StopEmitting, the non-looping end, the end of
//! play). A Director's ControlPlayable drives its systems by
//! `ParticleSystem.Simulate`: its Initialize makes the owner manual, every
//! restart resets the seeds from it (no shared-manager draw), plays and warms
//! a prewarm system, and each chunk is the same update without the backlog
//! widening, with the native birth owner where the route and modules qualify.
//!
//! A TrailModule system draws its trail as the renderer's second draw, as the
//! weather host does; a Local simulation's trail job reads owner words, which
//! this host composes for a placed fixture's prefab from the fixture's
//! placement ([`placed_fixture_owner`]) and refuses elsewhere.
//!
//! Named differences from the weather host: no culling pass (a system is
//! never culled), and no sub-emitter target (a target's owner words are
//! composed only for a site effect on its authored chain), so a parent's
//! commands are dropped, counted.
use super::*;
use moly_law::particle::owner::{OwnerMatrices, OwnerScaling, SourceTrs};

struct Candidate {
    anchor: Entity,
    plan: Planned,
    /// The prefab chain a Local trail's owner words are composed on.
    prefab: Option<Vec<SourceTrs>>,
}

#[derive(Component)]
pub(crate) struct Request(Vec<Candidate>);

/// A private, dormant GPU preparation owned by one source Control root.
#[derive(Component)]
pub(crate) struct ControlPreparation(Vec<Candidate>, f64);

pub(crate) fn discard_abandoned_controls(world: &mut World, now: f64) {
    let expired: Vec<_> = world.query::<(Entity, &ControlPreparation)>().iter(world)
        .filter_map(|(root, request)| (now - request.1 > 2.0).then_some(root)).collect();
    for root in expired {
        if let Some(request) = world.entity_mut(root).take::<ControlPreparation>() {
            for candidate in request.0 {
                if let Some((draw, _)) = candidate.plan.draw { world.despawn(draw); }
            }
        }
    }
}

/// The trail draw of a fixture-host system: the renderer's second draw, a
/// child of the particle draw (so it goes with it), and its mesh.
#[derive(Component)]
pub(crate) struct FixtureTrailDraw(pub(crate) Entity, pub(crate) Handle<Mesh>);

/// The prefab of a placed fixture: the fixture interface package, whose root
/// carries the FixtureView the placement writes.
fn placed_fixture_package(package: &str) -> bool {
    package.starts_with("mysekai__fixture__")
}

/// One record's serialized TRS words.
fn record_trs(record: &Value) -> Result<SourceTrs, String> {
    fn words<const N: usize>(record: &Value, key: &str) -> Option<[f32; N]> {
        let list = record.get(key)?.as_array().filter(|list| list.len() == N)?;
        let mut out = [0.0f32; N];
        for (slot, value) in out.iter_mut().zip(list) {
            *slot = value.as_f64()? as f32;
        }
        Some(out)
    }
    (|| Some(SourceTrs { t: words(record, "position")?, q: words(record, "rotation")?, s: words(record, "scale")? }))()
        .ok_or_else(|| format!("{}: authored TRS not exported", record["node"]))
}

/// A prefab's chain in source TRS words, the prefab root first: one entry per
/// prefix of the node's path. A document repeats a path for same-named
/// siblings; every record of a prefix must carry the same words.
fn prefab_chain(nodes: &[Value], path: &str) -> Result<Vec<SourceTrs>, String> {
    let parts: Vec<&str> = path.split('/').collect();
    let mut chain = Vec::with_capacity(parts.len());
    for depth in 1..=parts.len() {
        let prefix = parts[..depth].join("/");
        let mut records = nodes.iter().filter(|record| record["node"].as_str() == Some(prefix.as_str()));
        let first = record_trs(records.next().ok_or_else(|| format!("prefab chain: {prefix} has no record"))?)?;
        for record in records {
            if record_trs(record)? != first {
                return Err(format!("prefab chain: {prefix} names records with different TRS words"));
            }
        }
        chain.push(first);
    }
    Ok(chain)
}

/// Admission's half of a Local trail's owner words: a placed fixture's
/// prefab, whose chain is exported. The placement is read at install.
fn trail_owner_admissible(package: &str, nodes: &[Value], node: &str) -> Result<(), String> {
    if !placed_fixture_package(package) {
        return Err("owner words of a Local trail: this host composes them only for a placed fixture's prefab".into());
    }
    prefab_chain(nodes, node).map(|_| ())
}

/// The owner words of a system in a placed fixture's prefab, in its scaling
/// mode. The chain the engine's owner update walks: the site's fixture
/// container and the site view's parent, both at the identity (the container
/// is a new GameObject placed at the site view's position, and this host draws
/// the active site at its origin); the fixture view, which is the prefab root,
/// with the local position `FixtureView.SetPosition` writes (the field
/// position), the local rotation `FixtureView.ForceSetRotation` leaves
/// ([`moly_law::particle::placement::fixture_view_rotation`]) and its
/// authored scale; then the prefab below the root.
///
/// `placement` is the fixture root's transform in this runtime's axes (source
/// X reflected; the placement import swapped Left and Right, so the source
/// direction is the reflected yaw's quarter turns taken backwards).
fn placed_fixture_owner(placement: &Transform, prefab: &[SourceTrs], scaling: OwnerScaling) -> Result<OwnerMatrices, String> {
    let q = placement.rotation;
    let yaw = 2.0 * (q.y as f64).atan2(q.w as f64);
    let quarters = (yaw / std::f64::consts::FRAC_PI_2).round();
    if q.x != 0.0 || q.z != 0.0 || (yaw - quarters * std::f64::consts::FRAC_PI_2).abs() > 1e-5 || placement.scale != Vec3::ONE {
        return Err(format!("fixture placement {placement:?} is not a quarter-turn yaw at unit scale"));
    }
    let reflected = (quarters as i64).rem_euclid(4) as u8;
    let direction = (4 - reflected) % 4;
    let t = placement.translation;
    let x = if t.x == 0.0 { 0.0 } else { -t.x };
    let identity = SourceTrs { t: [0.0; 3], q: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] };
    let ancestors = [identity, identity];
    let root = prefab.first().ok_or("owner chain lacks the prefab root")?;
    let view = SourceTrs { t: [x, t.y, t.z], q: moly_law::particle::placement::fixture_view_rotation(direction, &ancestors), s: root.s };
    let chain: Vec<SourceTrs> = ancestors.iter().copied().chain(std::iter::once(view)).chain(prefab[1..].iter().copied()).collect();
    let owner = moly_law::particle::owner::owner_matrices(&chain, scaling).map_err(|refused| format!("owner words {refused:?}"))?;
    if !owner.invert_ok {
        return Err("owner matrix below the inverse's determinant threshold".into());
    }
    Ok(owner)
}

/// The fixture root above `anchor` and its transform, if `anchor` is in a
/// placed fixture.
fn fixture_placement(anchor: Entity, parent: impl Fn(Entity) -> Option<Entity>,
    root_transform: impl Fn(Entity) -> Option<Transform>) -> Option<Transform> {
    let mut current = Some(anchor);
    while let Some(entity) = current {
        if let Some(transform) = root_transform(entity) {
            return Some(transform);
        }
        current = parent(entity);
    }
    None
}

/// The prefab chain of an admitted plan whose system is a Local simulation
/// with a trail (`None` for any other plan).
fn trail_prefab(plan: &Planned, nodes: &[Value]) -> Result<Option<Vec<SourceTrs>>, String> {
    if plan.trail.is_none() || plan.emitter.simulation_space != SimulationSpace::Local {
        return Ok(None);
    }
    prefab_chain(nodes, &plan.node).map(Some)
}

/// The trail owner words of a plan with a prefab chain, from its fixture's
/// placement.
fn plan_trail_owner(plan: &Planned, prefab: Option<&[SourceTrs]>, placement: Option<Transform>)
    -> Result<Option<crate::particle_runtime::TrailOwner>, String> {
    let Some(prefab) = prefab else { return Ok(None) };
    let placement = placement.ok_or("owner words of a Local trail: the emitter is not in a placed fixture")?;
    let scaling = match &plan.geometry {
        PlannedGeometry::Billboard(draw) => draw.scaling,
        PlannedGeometry::Mesh { scaling, .. } | PlannedGeometry::EmptyMesh { scaling, .. } => *scaling,
    };
    let owner = placed_fixture_owner(&placement, prefab, owner_scaling(scaling))?;
    Ok(Some(crate::particle_runtime::TrailOwner::from_matrices(&owner)))
}

/// After the Play's install: attach a Local trail's owner words, and refuse
/// a trail system left without its trail state or owner words.
fn attach_installed_trail(system: &mut Runtime, plan: &Planned, owner: Option<crate::particle_runtime::TrailOwner>)
    -> Result<(), String> {
    if let Some(owner) = owner {
        crate::particle_runtime::attach_trail_owner(system, owner).map_err(str::to_owned)?;
    }
    if plan.trail.is_some() && (system.trail.is_none() || !crate::particle_runtime::trail_owner_ready(system)) {
        return Err("trail system installed without its trail state or owner words".into());
    }
    Ok(())
}

/// Prepare the selected emitters of an instance for an owner that plays them
/// with `ParticleSystem.Play` (the site move, the foot effects): each system
/// is installed with the native birth owner and steps its own per-frame
/// update (see [`Played`]). `Ok(None)` while geometry or GPU preparation is
/// pending; call again with the same root.
pub(crate) fn prepare_control(
    world: &mut World,
    root: Entity,
    doc: &Value,
    selected: &[(Entity, usize)],
) -> Result<Option<Vec<Entity>>, String> {
    prepare(world, root, doc, selected, Stepping::Played)
}

/// The same for a Director's ControlPlayable: the systems wait for its paused
/// clock and step by its `ParticleSystem.Simulate` time; each restart installs
/// the birth owner from the manual seed (see
/// [`crate::particle_runtime::director_restart`]).
pub(crate) fn prepare_director_control(
    world: &mut World,
    root: Entity,
    doc: &Value,
    selected: &[(Entity, usize)],
) -> Result<Option<Vec<Entity>>, String> {
    prepare(world, root, doc, selected, Stepping::Director)
}

fn prepare(
    world: &mut World,
    root: Entity,
    doc: &Value,
    selected: &[(Entity, usize)],
    stepping: Stepping,
) -> Result<Option<Vec<Entity>>, String> {
    let server = world.resource::<AssetServer>().clone();
    let mut preparation = match world.entity_mut(root).take::<ControlPreparation>() {
        Some(preparation) => preparation,
        None => {
            let particles = doc["emitters"].as_array().ok_or("missing source emitter list")?;
            let nodes = doc["nodes"].as_array().ok_or("missing source node list")?;
            let by_path = nodes.iter().filter_map(|node|
                Some((node["node"].as_str()?.to_owned(), node))).collect();
            let owners = source_sub_emitter_owners(particles);
            let package = doc["name"].as_str().unwrap_or("fixture");
            let mut candidates = Vec::new();
            for &(anchor, ordinal) in selected {
                let particle = &particles[ordinal];
                let mut plan = admit(package, particle, &by_path, nodes, &owners, &server, Path::Control, stepping)?;
                plan.ordinal = ordinal;
                // A Control-driven emitter is a renderer of the fixture prefab
                // too, so the fixture setup forced its material (see `plan`).
                plan.source.force_phenomena_lighting();
                if let Some(trail) = plan.trail.as_mut() { trail.source.force_phenomena_lighting(); }
                let prefab = trail_prefab(&plan, nodes)
                    .map_err(|reason| format!("{node}: source particle control rejected {reason}", node = particle["node"]))?;
                candidates.push(Candidate { anchor, plan, prefab });
            }
            ControlPreparation(candidates, 0.0)
        }
    };
    preparation.1 = crate::fixture_timeline_particles::realtime(world);
    let result = (|| {
        for candidate in &mut preparation.0 {
            let plan = &mut candidate.plan;
            let ready = prepare_geometry(plan, &server, world.resource::<Assets<Gltf>>(),
                world.resource::<Assets<GltfNode>>(), world.resource::<Assets<GltfMesh>>(),
                world.resource::<Assets<Mesh>>())?;
            if !ready { return Ok(None); }
            if !plan.source.resolve(&server, world.resource::<Assets<SourceShaderCatalogue>>()).map_err(|e| e.0)? {
                return Ok(None);
            }
            if plan.draw.is_none() {
                let mesh = world.resource_mut::<Assets<Mesh>>().add(billboard::empty_mesh());
                let draw = world.spawn((Mesh3d(mesh.clone()), plan.source.clone(), Transform::IDENTITY,
                    NoFrustumCulling, crate::shadowmap::NoShadowCast, ChildOf(root))).id();
                plan.draw = Some((draw, mesh));
            }
            match plan.source.readiness.lock().unwrap().clone() {
                ParticleReadiness::Pending => return Ok(None),
                ParticleReadiness::Failed(error) => return Err(error),
                ParticleReadiness::Ready => {}
            }
            let leader = plan.draw.as_ref().expect("particle draw spawned").0;
            if let Some(trail) = plan.trail.as_mut() {
                if !trail.source.resolve(&server, world.resource::<Assets<SourceShaderCatalogue>>()).map_err(|e| e.0)? {
                    return Ok(None);
                }
                if trail.draw.is_none() {
                    let mesh = world.resource_mut::<Assets<Mesh>>().add(billboard::empty_mesh());
                    let draw = world.spawn((Mesh3d(mesh.clone()), trail.source.clone(), Transform::IDENTITY,
                        NoFrustumCulling, crate::shadowmap::NoShadowCast, ChildOf(leader))).id();
                    trail.draw = Some((draw, mesh));
                }
                match trail.source.readiness.lock().unwrap().clone() {
                    ParticleReadiness::Pending => return Ok(None),
                    ParticleReadiness::Failed(error) => return Err(format!("trail draw: {error}")),
                    ParticleReadiness::Ready => {}
                }
            }
        }
        let mut draws = Vec::new();
        for candidate in &preparation.0 {
            let (draw, mesh) = candidate.plan.draw.clone().expect("prepared control draw");
            let mut system = runtime(&candidate.plan, candidate.anchor, mesh);
            system.geometry.keep_unit_chain(unit_ancestry(candidate.anchor,
                |entity| world.get::<Transform>(entity).map(|t| t.scale),
                |entity| world.get::<ChildOf>(entity).map(ChildOf::parent)));
            let placement = fixture_placement(candidate.anchor, |entity| world.get::<ChildOf>(entity).map(ChildOf::parent),
                |entity| world.get::<crate::fixture::FixtureRoot>(entity).and_then(|_| world.get::<Transform>(entity).copied()));
            let trail_owner = plan_trail_owner(&candidate.plan, candidate.prefab.as_deref(), placement)
                .map_err(|reason| format!("{}: source particle control rejected {reason}", candidate.plan.node))?;
            match stepping {
                // No playOnAwake here. The owning Director installs a paused
                // clock; its first restart installs the owner.
                Stepping::Director => {
                    // Each restart installs the trail with the words kept here.
                    if let Some(owner) = trail_owner {
                        crate::particle_runtime::attach_trail_owner(&mut system, owner)
                            .map_err(|reason| format!("{}: source particle control rejected {reason}", candidate.plan.node))?;
                    }
                    world.entity_mut(draw).insert((crate::uber_particle::FixtureParticleLive(system),
                        DirectorRoute(candidate.plan.route.clone())));
                }
                Stepping::Played => {
                    let played = world.resource_scope(|_, mut seeds: Mut<crate::particle_runtime::seed::SystemSeedManager>|
                        install(&mut system, &candidate.plan, &mut seeds))
                        .and_then(|played| attach_installed_trail(&mut system, &candidate.plan, trail_owner).map(|()| played))
                        .map_err(|reason| format!("{}: source particle control rejected {reason}",
                            doc["emitters"][candidate.plan.ordinal]["node"]))?;
                    world.entity_mut(draw).insert((crate::uber_particle::FixtureParticleLive(system), played));
                }
            }
            if let Some((trail, trail_mesh)) = candidate.plan.trail.as_ref().and_then(|trail| trail.draw.clone()) {
                let mut source = candidate.plan.trail.as_ref().expect("trail plan").source.clone();
                source.enabled = true;
                source.follows = Some(draw);
                world.entity_mut(trail).insert(source);
                world.entity_mut(draw).insert(FixtureTrailDraw(trail, trail_mesh));
            }
            draws.push(draw);
        }
        Ok(Some(draws))
    })();
    match &result {
        Ok(None) => { world.entity_mut(root).insert(preparation); }
        Err(_) => for candidate in preparation.0 {
            if let Some((draw, _)) = candidate.plan.draw { world.despawn(draw); }
        },
        Ok(Some(_)) => {}
    }
    result
}

/// Whether the emitter's instance node and every ancestor above it carry scale
/// exactly one (an entity without a Transform counts as one): the run-time
/// half of Local scaling's unit-chain evidence, whose document half the
/// admission read. Native Local scaling builds the owner from the hierarchy's
/// rotation and translation only, which is the composed owner only then.
fn unit_ancestry(anchor: Entity, scale: impl Fn(Entity) -> Option<Vec3>, parent: impl Fn(Entity) -> Option<Entity>) -> bool {
    let mut current = Some(anchor);
    while let Some(entity) = current {
        if scale(entity).is_some_and(|scale| scale != Vec3::ONE) {
            return false;
        }
        current = parent(entity);
    }
    true
}

pub(crate) fn is_source_particle(particle: &Value) -> bool {
    particle.pointer("/renderer/material").is_some_and(|material|
        material.get("sourceMaterial").is_some() || material.get("shaderProgram").is_some()
            || material.get("sourceError").is_some())
        || particle.pointer("/renderer/geometryError").is_some()
}

/// How a fixture-host system reaches this admission: `Autonomous` is a
/// play-on-awake system of a placed prefab ([`plan`]); `Control` is a system
/// prepared for an owner that plays it ([`prepare_control`]: the site move's
/// explicit Play, the foot effects; [`prepare_director_control`]: a Director).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Path {
    Autonomous,
    Control,
}

/// Who steps an admitted system. `Played`: its own per-frame update after a
/// `ParticleSystem.Play` (play on awake or explicit), which this host runs as
/// the weather host does, with the native birth owner. `Director`:
/// `ParticleSystem.Simulate` from a ControlPlayable, whose restarts install
/// the birth owner from the manual seed and whose chunks run the same update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stepping {
    Played,
    Director,
}

/// The source route of a system a Director prepared, which its restarts read.
#[derive(Component, Clone)]
pub(crate) struct DirectorRoute(pub(crate) crate::particle_runtime::SourceRoute);

/// emitterVelocityMode 1 reads a Rigidbody on the system's GameObject or a
/// Transform ancestor (see [`resolve_velocity_mode`]). A fixture document
/// carries every node's declared component inventory (`componentClasses`,
/// the GameObject's component list; a list the export could not resolve is
/// absent), so a document whose every node has an inventory without a
/// Rigidbody or Rigidbody2D rules one out on the prefab. Its run-time
/// ancestors outside the prefab are this runtime's anchors, which carry none
/// (the same boundary as the weather host's census).
pub(crate) fn census_names_no_body(by_path: &HashMap<String, &Value>) -> Result<(), String> {
    for (path, node) in by_path {
        let Some(classes) = node.get("componentClasses").and_then(Value::as_array) else {
            return Err(format!("{path}: component inventory not exported"));
        };
        for class in classes {
            match class.as_str() {
                Some(class) if class.starts_with("Rigidbody") => return Err(format!("{class} on {path}")),
                Some(_) => {}
                None => return Err(format!("{path}: a component of unread kind")),
            }
        }
    }
    Ok(())
}

/// The fixture host's admission of one source emitter: the shared source
/// judgement, then the host's own gates. `Err` names the reason; on the
/// control path every refusal carries the `<node>: source particle control
/// rejected` prefix its callers skip single emitters by.
#[allow(clippy::too_many_arguments)]
fn admit(
    package: &str,
    particle: &Value,
    by_path: &HashMap<String, &Value>,
    nodes: &[Value],
    owners: &SubEmitterGraph<'_>,
    server: &AssetServer,
    path: Path,
    stepping: Stepping,
) -> Result<Planned, String> {
    let node = &particle["node"];
    if path == Path::Control && !is_source_particle(particle) {
        return Err(format!("{node}: source-owned shader/material export required"));
    }
    let refuse = |reason: String| match path {
        Path::Control => format!("{node}: source particle control rejected {reason}"),
        Path::Autonomous => reason,
    };
    let mut tally = Tally::default();
    let plan = judge_in_host(package, particle, by_path, owners, EffectKind::Site, false,
        None, true, "fixture-particles-v2", Some(GlobalTransform::IDENTITY),
        &Err("collision scene: fixture particles carry no collider export".to_owned()),
        &census_names_no_body(by_path), server, &mut tally);
    let Some(plan) = plan else {
        return Err(match path {
            Path::Control => format!("{node}: source particle control rejected {tally:?}"),
            Path::Autonomous => format!("rejected {tally:?}"),
        });
    };
    // A TrailModule draws with the renderer's trail material in a second
    // draw; a Local simulation's trail job reads the owner words this host
    // composes for a placed fixture's prefab.
    if plan.trail.is_some() && plan.emitter.simulation_space == SimulationSpace::Local {
        trail_owner_admissible(package, nodes, &plan.node).map_err(refuse)?;
    }
    if stepping == Stepping::Played {
        // Noise, sub-emitter events and emission over distance run with the
        // native birth owner this host installs (a sub-emitter target is
        // refused by the judgement: its owner words exist only for a site
        // effect on its authored chain, so a parent's commands find no
        // installed target and are dropped, counted).
        return Ok(plan);
    }
    // The Director's Simulate simulates a system's sub-emitters before the
    // system itself (SimulateChildrenRecursive), which this host does not run.
    if plan.event_edges.is_some() {
        return Err(refuse("sub-emitter events: the Director's Simulate steps sub-emitters first, which this host does not run".into()));
    }
    crate::particle_runtime::director_restart_warm(&plan.emitter, &plan.route, plan.sub_emitter_max_lifetime)
        .map_err(refuse)?;
    director_birth_path(&plan).map_err(refuse)?;
    Ok(plan)
}

/// The birth path a Director's restart installs for an admitted plan (the
/// route and emitter-state test on the plan's geometry evidence): `None` for
/// the native birth owner, the legacy step's reason otherwise, and `Err` for
/// what the legacy step cannot run. No seed is drawn.
fn director_birth_path(plan: &Planned) -> Result<Option<String>, String> {
    let evidence = match &plan.geometry {
        PlannedGeometry::Billboard(draw) =>
            crate::particle_runtime::ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: false },
        PlannedGeometry::Mesh { scaling, .. } | PlannedGeometry::EmptyMesh { scaling, .. } =>
            crate::particle_runtime::ShapeEmitterEvidence { scaling: *scaling, mesh_renderer: true },
    };
    match crate::particle_runtime::native_birth_path(&plan.emitter, &plan.route, Some(evidence)) {
        Ok(()) => Ok(None),
        Err(reason) => {
            let tracks_storage = plan.emitter.custom_data.as_ref().is_some_and(|p|
                crate::particle_runtime::custom_data_law(p).expect("curves validated during admission").tracks_storage());
            match legacy_refusal_for(&plan.emitter, tracks_storage, plan.event_edges.is_some()) {
                Some(refusal) => Err(format!("{refusal}: {reason}")),
                None => Ok(Some(reason)),
            }
        }
    }
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
        match admit(package, particle, &by_path, nodes, &owners, server, Path::Autonomous, Stepping::Played) {
            Err(reason) => warn!("[fixture-source] {package}/{node}: {reason}"),
            Ok(mut plan) => {
                plan.ordinal = ordinal;
                // FixtureController.Setup -> FixtureView.SetupRenderer calls
                // SetPhenomenaLighting(true) on every material gathered by
                // CollectRenderersAndMaterials: all Renderer components under
                // the fixture, inactive ones included, with no renderer-subtype
                // filter (only a null material or a null shader is skipped), so
                // particle renderers are forced like meshes. Authored flags of
                // fixture particle materials therefore never reach the draw.
                // The renderer's trail material is one of its materials too.
                plan.source.force_phenomena_lighting();
                if let Some(trail) = plan.trail.as_mut() { trail.source.force_phenomena_lighting(); }
                match trail_prefab(&plan, nodes) {
                    Ok(prefab) => candidates.push(Candidate { anchor, plan, prefab }),
                    Err(reason) => warn!("[fixture-source] {package}/{node}: {reason}"),
                }
            }
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
    fn abandoned_control_gpu_preparation_does_not_become_a_fixture_cache() {
        let mut world = World::new();
        let root = world.spawn(ControlPreparation(Vec::new(), 0.0)).id();
        discard_abandoned_controls(&mut world, 1.0);
        assert!(world.get::<ControlPreparation>(root).is_some());
        discard_abandoned_controls(&mut world, 3.0);
        assert!(world.get::<ControlPreparation>(root).is_none());
        assert!(world.get_entity(root).is_ok());
    }

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
                    None, "fixture-particles-v2", Some(GlobalTransform::IDENTITY), &Err("collision scene: fixture particles carry no collider export".to_owned()),
                    &Err("fixture particles carry no component census".to_owned()), server, &mut tally);
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
                    .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission"));
                runtime.size_law = runtime.emitter.size_over_lifetime.as_ref()
                    .map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission"));
                runtime.color_law = runtime.emitter.color_over_lifetime.as_ref()
                    .map(moly_law::particle::color::ColorOverLifetime::from_params);
                let context = Context { site: GlobalTransform::IDENTITY, sky: GlobalTransform::IDENTITY, camera: GlobalTransform::IDENTITY };
                for _ in 0..60 { crate::particle_runtime::simulate(&mut runtime, 1.0 / 60.0, &context); }
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

    /// The fixture host's played systems against the native per-frame
    /// Update1b rows of the distance-emission and per-frame receipts: each
    /// case's system is installed by this host's installer
    /// (the native birth owner at its Play) and stepped by this host's frame
    /// entry (`step_played`: the play state, the frame clock and the frame
    /// head with the emission over distance, then the slices). The product
    /// must match every native frame; each mutant must differ somewhere on the
    /// same rows: the host without the birth owner (the base fixture host),
    /// the foot effect's own one-slice step (`step_frame` of the scaled dt),
    /// a system stopped at its Play, and the runtime's two distance arms.
    #[test]
    #[ignore = "MOLY_UPDATE1B_FRAMES, MOLY_UPDATE1B_FRAMES_EXTRA and MOLY_UPDATE1B_FRAMES_MODULES must identify the current JP per-frame Update1b receipts"]
    fn fixture_host_distance_emission_matches_native_update1b_rows() {
        use crate::particle_runtime::frame_samples::{replay_through_host, with_arm};
        use crate::particle_runtime::SourceRoute;
        let clocks = |dt: f32| FrameClocks { scaled: dt, unscaled: Some(dt), now: 0.0 };
        let install_host = |system: &mut Runtime, seeds: &mut crate::particle_runtime::seed::SystemSeedManager| {
            let played = install_played(system, &SourceRoute::Ordinary, seeds).expect("fixture host install");
            assert!(played.legacy.is_none(), "the fixture host left a harness system on the legacy step");
            played
        };
        let step_host = |system: &mut Runtime, played: &mut Played, dt: f32, ctx: &Context| {
            step_played(system, played, &clocks(dt), ctx).expect("fixture host step");
        };
        let product = replay_through_host(&install_host, &step_host);
        // The base fixture host: no birth owner (the installer's legacy step).
        let no_owner = replay_through_host(
            &|_, _| Played {
                play: lifecycle::PlayState::played(lifecycle::Culling::Refused("mutant".into()), false),
                culling: lifecycle::Culling::Refused("mutant".into()), procedural_warm: false, emitting: true,
                route: SourceRoute::Ordinary, event_edges: None, legacy: Some("mutant: no birth owner".into()), dropped: 0,
            },
            &|system, played, dt, ctx| { let _ = step_played(system, played, &clocks(dt), ctx); });
        // The foot effect's own step: one explicit slice of the scaled dt.
        let one_slice = replay_through_host(&install_host, &|system, _, dt, ctx| {
            let step = dt * system.emitter.simulation_speed;
            if step > 0.0 { let _ = crate::particle_runtime::step_frame(system, step, ctx, true); }
        });
        // Stop at Play: the system never emits.
        let stopped = replay_through_host(
            &|system, seeds| { let mut played = install_host(system, seeds); played.emitting = false; played },
            &step_host);
        let late = with_arm(Some("distanceAfterSlices"), || replay_through_host(&install_host,
            &|system, played, dt, ctx| { let _ = step_played(system, played, &clocks(dt), ctx); }));
        let unpending = with_arm(Some("elapsedWithoutPending"), || replay_through_host(&install_host,
            &|system, played, dt, ctx| { let _ = step_played(system, played, &clocks(dt), ctx); }));
        let report = json!({
            "cases": product.0, "frames": product.1, "mismatchedFrames": product.2, "firstMismatches": product.3,
            "mutants": {
                "noBirthOwner": [no_owner.1, no_owner.2, no_owner.3],
                "footOneSliceStep": [one_slice.1, one_slice.2, one_slice.3],
                "stoppedAtPlay": [stopped.1, stopped.2, stopped.3],
                "distanceAfterSlices": [late.1, late.2, late.3],
                "elapsedWithoutPending": [unpending.1, unpending.2, unpending.3],
            },
        });
        println!("{report}");
        if let Some(path) = std::env::var_os("MOLY_FIXTURE_HOST_REPLAY_REPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert_eq!((product.0, product.1), (24, 847), "{report}");
        assert_eq!(product.2, 0, "{report}");
        for mutant in [&no_owner, &one_slice, &stopped, &late, &unpending] {
            assert_eq!(mutant.1, product.1, "{report}");
            assert!(mutant.2 > 0, "{report}");
        }
    }

    /// Coverage diagnostic, not a correctness test: every source emitter of
    /// every package the fixture particle index lists, judged by the fixture
    /// host's admission on the path its product owner reaches it by, with the
    /// refusal reason, one line per emitter, then counts by host and reason.
    /// The denominator is the release root's `fixture-particles-v2/index.json`
    /// package list (each package's document once). Hosts: the site move's
    /// pooled effects (Flying by explicit Play, the two landings play on
    /// awake), the cannon prefab (play on awake), the foot effects (explicit
    /// Play), and every other package as a placed fixture (play-on-awake
    /// systems autonomous; the rest are reached only when a Director binds
    /// them). Records without a source-owned renderer go to the legacy
    /// summary path and are counted, not judged.
    #[test]
    #[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing a release root's fixture-particles-v2"]
    fn fixture_host_refusal_census() {
        use crate::site_move::effects::EffectType;
        let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
        let index: Value = serde_json::from_slice(&std::fs::read(root.join("fixture-particles-v2/index.json")).unwrap()).unwrap();
        let packages = index["packages"].as_object().expect("index packages");
        let mut app = App::new();
        moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir { path: root.clone() });
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::image::ImagePlugin::default()));
        moly_assets::source_shader::loader::register(&mut app);
        app.init_asset::<Gltf>();
        app.finish(); app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let control: Vec<String> = [EffectType::Flying, EffectType::Dash, EffectType::WalkWater]
            .into_iter().map(EffectType::package).collect();
        let autonomous: Vec<String> = [EffectType::SiteMoveEndPlayer, EffectType::SiteMoveFailedPlayer]
            .into_iter().map(EffectType::package)
            // The cannon prefab's particle package (the site move's private
            // `cannon::PARTICLE_PACKAGE`).
            .chain(["mysekai__site__move__cannon".to_owned()]).collect();
        let mut files: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (package, entry) in packages {
            let Some(file) = entry["file"].as_str() else { continue; };
            files.entry(file.to_owned()).or_default().push(package.clone());
        }
        let mut reasons: std::collections::BTreeMap<(String, String), usize> = Default::default();
        let (mut rows, mut legacy, mut admitted, mut unplayed) = (0usize, 0usize, 0usize, 0usize);
        for (file, names) in &files {
            let doc: Value = serde_json::from_slice(&std::fs::read(root.join("fixture-particles-v2").join(file)).unwrap()).unwrap();
            let Some(particles) = doc["emitters"].as_array() else { continue; };
            let Some(nodes) = doc["nodes"].as_array() else { continue; };
            let by_path: HashMap<String, &Value> = nodes.iter()
                .filter_map(|node| Some((node["node"].as_str()?.to_owned(), node))).collect();
            let owners = source_sub_emitter_owners(particles);
            let package = doc["name"].as_str().unwrap_or("fixture");
            let host = if names.iter().any(|name| control.contains(name) || autonomous.contains(name)) {
                names.iter().find(|name| control.contains(name) || autonomous.contains(name)).unwrap().clone()
            } else {
                "fixture".to_owned()
            };
            for particle in particles {
                let node = particle["node"].as_str().unwrap_or("");
                if !is_source_particle(particle) {
                    legacy += 1;
                    continue;
                }
                let played = particle["activeInHierarchy"] == true && particle["system"]["playOnAwake"] == true;
                let (path, stepping) = if control.contains(&host) {
                    (Path::Control, Stepping::Played)
                } else if played {
                    (Path::Autonomous, Stepping::Played)
                } else if host == "fixture" {
                    // Reached only through a Director's control binding.
                    (Path::Control, Stepping::Director)
                } else {
                    // Neither played on awake nor by an explicit Play.
                    unplayed += 1;
                    println!("fixture-host-census | {host} | {file} | {node} | not played");
                    continue;
                };
                rows += 1;
                let verdict = census_admit(package, particle, &by_path, nodes, &owners, server, path, stepping);
                let reason = match verdict {
                    Ok(label) => { admitted += 1; label }
                    Err(reason) => census_reason(&reason, &particle["node"]),
                };
                println!("fixture-host-census | {host} | {file} | {node} | {path:?} | {reason}");
                let label = if host == "fixture" { format!("fixture {path:?}") } else { host.clone() };
                *reasons.entry((label, reason)).or_default() += 1;
            }
        }
        for ((host, reason), count) in &reasons {
            println!("fixture-host-census count {count} | {host} | {reason}");
        }
        println!("fixture-host-census documents {} packages {} judged {rows} admitted {admitted} legacy-summary {legacy} not-played {unplayed}",
            files.len(), packages.len());
        assert!(rows > 0, "the index lists no source emitter");
    }

    /// The census verdict: the admission and, for a played system, the birth
    /// owner's install decision (the installer's route and emitter-state test
    /// on the plan's geometry evidence, and its refusal of what the legacy
    /// step cannot run); no seed is drawn.
    #[allow(clippy::too_many_arguments)]
    fn census_admit(package: &str, particle: &Value, by_path: &HashMap<String, &Value>, nodes: &[Value],
        owners: &SubEmitterGraph<'_>, server: &AssetServer, path: Path, stepping: Stepping) -> Result<String, String> {
        let plan = admit(package, particle, by_path, nodes, owners, server, path, stepping)?;
        if stepping == Stepping::Director {
            return Ok(match director_birth_path(&plan)? {
                None => "admitted, Director native birth owner".to_owned(),
                Some(reason) => format!("admitted, Director legacy step: {reason}"),
            });
        }
        let evidence = match &plan.geometry {
            PlannedGeometry::Billboard(draw) =>
                crate::particle_runtime::ShapeEmitterEvidence { scaling: draw.scaling, mesh_renderer: false },
            PlannedGeometry::Mesh { scaling, .. } | PlannedGeometry::EmptyMesh { scaling, .. } =>
                crate::particle_runtime::ShapeEmitterEvidence { scaling: *scaling, mesh_renderer: true },
        };
        match crate::particle_runtime::native_birth_path(&plan.emitter, &plan.route, Some(evidence)) {
            Ok(()) => Ok("admitted, native birth owner".to_owned()),
            Err(reason) => {
                let mut system = crate::particle_runtime::test_support::runtime();
                system.emitter = plan.emitter.clone();
                system.custom_law = plan.emitter.custom_data.as_ref()
                    .map(|p| crate::particle_runtime::custom_data_law(p).expect("curves validated during admission"));
                match legacy_refusal(&system, plan.event_edges.is_some()) {
                    Some(refusal) => Err(format!("{refusal}: {reason}")),
                    None => Ok(format!("admitted, legacy step: {reason}")),
                }
            }
        }
    }

    /// One refusal of the census, without its node and with the judgement's
    /// tally reduced to the bucket that refused.
    fn census_reason(reason: &str, node: &Value) -> String {
        let reason = reason.strip_prefix(&format!("{node}: ")).unwrap_or(reason);
        let Some(tally) = reason.strip_prefix("source particle control rejected ").or_else(|| reason.strip_prefix("rejected ")) else {
            return reason.to_owned();
        };
        // `Tally { records: 1, .., bucket: value, .. }`: keep the one bucket
        // that is not empty or zero.
        let node = node.as_str().unwrap_or("");
        let body = tally.trim_start_matches("Tally { ").trim_end_matches(" }");
        let mut kept = Vec::new();
        let mut depth = 0i32;
        let mut start = 0usize;
        let bytes = body.as_bytes();
        let mut fields = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'[' | b'{' | b'(' => depth += 1,
                b']' | b'}' | b')' => depth -= 1,
                b',' if depth == 0 => { fields.push(&body[start..i]); start = i + 1; }
                _ => {}
            }
        }
        fields.push(&body[start..]);
        for field in fields {
            let field = field.trim();
            let Some((key, value)) = field.split_once(": ") else { continue; };
            if key == "records" || value == "0" || value == "[]" { continue; }
            let value = value.replace(&format!("\\\"{node}: "), "\\\"").replace(&format!("{node}: "), "");
            kept.push(format!("{key}: {value}"));
        }
        if kept.is_empty() { "judgement refused, no bucket".to_owned() } else { kept.join("; ") }
    }

    #[test]
    fn census_reason_keeps_the_refusing_bucket() {
        let node = json!("a/b");
        assert_eq!(census_reason("\"a/b\": source particle control rejected Tally { records: 1, no_renderer: 0, law_reject: [\"a/b: emission over distance requires the native birth path: x\"], admitted: 0 }", &node),
            "law_reject: [\"emission over distance requires the native birth path: x\"]");
        assert_eq!(census_reason("rejected Tally { records: 1, render_mode: [\"unsupported source render mode VerticalBillboard\"], admitted: 0 }", &node),
            "render_mode: [\"unsupported source render mode VerticalBillboard\"]");
        assert_eq!(census_reason("\"a/b\": emission over distance needs the native birth owner", &node),
            "emission over distance needs the native birth owner");
    }

    /// Coverage diagnostic, not a correctness test: where every ring-buffer
    /// system of the supplied fixture docs stops in the fixture host's
    /// admission. Prints one line per system.
    #[test]
    #[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing exported source fixture particles"]
    fn source_fixture_ring_systems_census() {
        let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
        let mut app = App::new();
        moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir { path: root.clone() });
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::image::ImagePlugin::default()));
        moly_assets::source_shader::loader::register(&mut app);
        app.init_asset::<Gltf>();
        app.finish(); app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let mut rows = 0;
        for entry in std::fs::read_dir(root.join("fixture-particles-v2")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let Some(particles) = doc["emitters"].as_array() else { continue; };
            let by_path = doc["nodes"].as_array().unwrap().iter()
                .map(|node| (node["node"].as_str().unwrap().to_owned(), node)).collect();
            let owners = source_sub_emitter_owners(particles);
            for particle in particles {
                if particle["system"]["ringBufferMode"].as_u64().is_none_or(|mode| mode == 0) { continue; }
                let mut tally = Tally::default();
                let planned = judge_in_archive("fixture", particle, &by_path, &owners, EffectKind::Site, false,
                    None, "fixture-particles-v2", Some(GlobalTransform::IDENTITY),
                    &Err("collision scene: fixture particles carry no collider export".to_owned()),
                    &Err("fixture particles carry no component census".to_owned()), server, &mut tally);
                println!("ring {} {}: admitted={} {tally:?}", path.file_name().unwrap().to_string_lossy(),
                    particle["node"].as_str().unwrap_or(""), planned.is_some());
                rows += 1;
            }
        }
        println!("ring systems {rows}");
        assert!(rows > 0, "the supplied fixture docs hold no ring-buffer system");
    }

    /// Coverage diagnostic, not a correctness test: where every system of the
    /// supplied fixture docs whose shape runs only with the native birth owner
    /// (Box, the BurstSpread circle and edge, the Loop, PingPong and
    /// BurstSpread cone) stops in the fixture host's admission, and whether
    /// its node is active in the exported hierarchy (the next gate). Every
    /// system of a doc whose file name holds `MOLY_FIXTURE_CENSUS_CONTROL` is
    /// printed as a control row. One line per system, then counts by reason.
    #[test]
    #[ignore = "requires MOLY_FIXTURE_PARTICLE_AUDIT_ROOT containing exported source fixture particles"]
    fn source_fixture_native_only_shape_census() {
        let root = std::path::PathBuf::from(std::env::var_os("MOLY_FIXTURE_PARTICLE_AUDIT_ROOT").expect("source directory"));
        let control = std::env::var("MOLY_FIXTURE_CENSUS_CONTROL").ok().filter(|name| !name.is_empty());
        let mut app = App::new();
        moly_assets::install(&mut app, moly_assets::AssetSource::NativeDir { path: root.clone() });
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::image::ImagePlugin::default()));
        moly_assets::source_shader::loader::register(&mut app);
        app.init_asset::<Gltf>();
        app.finish(); app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let mut rows = 0;
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        let mut entries: Vec<_> = std::fs::read_dir(root.join("fixture-particles-v2")).unwrap()
            .map(|entry| entry.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let Some(particles) = doc["emitters"].as_array() else { continue; };
            let Some(nodes) = doc["nodes"].as_array() else { continue; };
            let by_path: HashMap<String, &Value> = nodes.iter()
                .filter_map(|node| Some((node["node"].as_str()?.to_owned(), node))).collect();
            let owners = source_sub_emitter_owners(particles);
            let is_control = control.as_deref().is_some_and(|name| file.contains(name));
            for particle in particles {
                let system = &particle["system"];
                let shape = &system["shape"];
                let kind = shape["type"].as_str().unwrap_or("");
                let mode = if kind == "SingleSidedEdge" { &shape["radiusMode"] } else { &shape["arcMode"] };
                let native_only = system["shapeEnabled"] == true && match (kind, mode.as_str()) {
                    ("Box", _) => true,
                    ("Circle" | "SingleSidedEdge", Some("BurstSpread")) => true,
                    ("Cone", Some("Loop" | "PingPong" | "BurstSpread")) => true,
                    _ => false,
                };
                if !native_only && !is_control { continue; }
                let node = particle["node"].as_str().unwrap_or("");
                let mut tally = Tally::default();
                let planned = judge_in_archive("fixture", particle, &by_path, &owners, EffectKind::Site, false,
                    None, "fixture-particles-v2", Some(GlobalTransform::IDENTITY),
                    &Err("collision scene: fixture particles carry no collider export".to_owned()),
                    &Err("fixture particles carry no component census".to_owned()), server, &mut tally);
                let reason = tally.shape.first().or(tally.law_reject.first()).cloned()
                    .map(|text| text.strip_prefix(node).map(str::to_owned).unwrap_or(text))
                    .unwrap_or_else(|| if planned.is_some() { "admitted".into() }
                        else if tally.node_inactive > 0 { "node inactive".into() } else { format!("{tally:?}") });
                let label = if native_only { format!("{kind} {}", mode.as_str().unwrap_or("-")) } else { "control".into() };
                println!("native-only-census {label} | {file} | {node} | admitted={} active_in_hierarchy={} | {reason}",
                    planned.is_some(), active_in_hierarchy(&by_path, node));
                if native_only {
                    *reasons.entry(format!("{label}: {reason}")).or_default() += 1;
                    rows += 1;
                }
            }
        }
        for (reason, count) in &reasons { println!("native-only-census count {count} | {reason}"); }
        println!("native-only-census systems {rows}");
        assert!(rows > 0, "the supplied fixture docs hold no native-only shape system");
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut requests: Query<(Entity, &mut Request)>,
    mut seeds: ResMut<crate::particle_runtime::seed::SystemSeedManager>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    catalogues: Res<Assets<SourceShaderCatalogue>>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    ancestry: Query<(Option<&Transform>, Option<&ChildOf>)>,
    fixture_roots: Query<&Transform, With<crate::fixture::FixtureRoot>>,
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
            let leader = planned.draw.as_ref().expect("particle draw spawned").0;
            if let Some(trail) = planned.trail.as_mut() {
                match trail.source.resolve(&server, &catalogues) {
                    Ok(true) => {}
                    Ok(false) => { pending.push(candidate); continue; }
                    Err(error) => {
                        warn!("[fixture-source] {} trail draw: {}", planned.node, error.0);
                        commands.entity(leader).try_despawn();
                        continue;
                    }
                }
                if trail.draw.is_none() {
                    let mesh = meshes.add(billboard::empty_mesh());
                    let draw = commands.spawn((Mesh3d(mesh.clone()), trail.source.clone(), Transform::IDENTITY,
                        NoFrustumCulling, crate::shadowmap::NoShadowCast, ChildOf(leader))).id();
                    trail.draw = Some((draw, mesh));
                }
                let readiness = trail.source.readiness.lock().unwrap().clone();
                match readiness {
                    ParticleReadiness::Pending => { pending.push(candidate); continue; }
                    ParticleReadiness::Failed(error) => {
                        warn!("[fixture-source] {} trail GPU: {error}", planned.node);
                        commands.entity(leader).try_despawn();
                        continue;
                    }
                    ParticleReadiness::Ready => {}
                }
            }
            let Candidate { anchor, plan: planned, prefab } = candidate;
            let (draw, mesh) = planned.draw.clone().expect("prepared source draw");
            let mut runtime = runtime(&planned, anchor, mesh);
            runtime.geometry.keep_unit_chain(unit_ancestry(anchor,
                |entity| ancestry.get(entity).ok().and_then(|(t, _)| t.map(|t| t.scale)),
                |entity| ancestry.get(entity).ok().and_then(|(_, p)| p.map(ChildOf::parent))));
            let placement = fixture_placement(anchor, |entity| ancestry.get(entity).ok().and_then(|(_, p)| p.map(ChildOf::parent)),
                |entity| fixture_roots.get(entity).ok().copied());
            // Play on awake: the first Play installs the birth owner.
            let played = match plan_trail_owner(&planned, prefab.as_deref(), placement)
                .and_then(|owner| install(&mut runtime, &planned, &mut seeds)
                    .and_then(|played| attach_installed_trail(&mut runtime, &planned, owner).map(|()| played))) {
                Ok(played) => played,
                Err(reason) => {
                    error!("[fixture-source] {}: {reason}; not installed", planned.node);
                    commands.entity(draw).try_despawn();
                    continue;
                }
            };
            let mut source = planned.source;
            source.enabled = true;
            info!("[fixture-source] {}: installed ({}); fixture setup phenomena-lighting write {:?}",
                planned.node, played.birth_path(), source.phenomena_lighting_written);
            if let Some((trail, trail_mesh)) = planned.trail.as_ref().and_then(|trail| trail.draw.clone()) {
                let mut trail_source = planned.trail.as_ref().expect("trail plan").source.clone();
                trail_source.enabled = true;
                trail_source.follows = Some(draw);
                commands.entity(trail).try_insert(trail_source);
                commands.entity(draw).try_insert(FixtureTrailDraw(trail, trail_mesh));
            }
            // The prefab can be destroyed on the frame its draw turns ready (a
            // cannon's effects go with the cannon); its draw is gone then, and
            // so is the system the source played, so nothing is left to attach.
            commands.entity(draw).try_insert((source, crate::uber_particle::FixtureParticleLive(runtime), played));
        }
        request.0 = pending;
        if request.0.is_empty() { commands.entity(root).try_remove::<Request>(); }
    }
}

/// A system this host plays with `ParticleSystem.Play` (play on awake or an
/// explicit Play): the engine play state the weather host keeps
/// ([`lifecycle::PlayState`]: Stop(StopEmitting), the non-looping end, and the
/// end of play once emission has stopped and no particle is left), whether it
/// emits, and the birth owner installed at its Play. Stepped by
/// [`step_played`]; stopped and played again by [`stop_emitting`] and
/// [`play`]. A system without it is Director-driven or on the legacy summary
/// path.
#[derive(Component)]
pub(crate) struct Played {
    play: lifecycle::PlayState,
    culling: lifecycle::Culling,
    procedural_warm: bool,
    emitting: bool,
    route: crate::particle_runtime::SourceRoute,
    event_edges: Option<crate::particle_runtime::EventEdges>,
    /// Why the installer left the system on the legacy step (`None` on the
    /// native birth path).
    legacy: Option<String>,
    /// Sub-emitter commands with no installed target (this host installs no
    /// target), dropped.
    dropped: u64,
}

impl Played {
    /// The source route its Play decided the birth path on.
    pub(crate) fn route(&self) -> &crate::particle_runtime::SourceRoute {
        &self.route
    }

    pub(crate) fn birth_path(&self) -> String {
        match &self.legacy {
            None => "native birth owner".to_owned(),
            Some(reason) => format!("legacy step: {reason}"),
        }
    }

    /// `ParticleSystem.isPlaying` of the system (default culling: this host
    /// runs no culling pass, so the system is never culled).
    pub(crate) fn playing(&self, system: &Runtime, now: f64) -> bool {
        self.play.is_playing(system, now)
    }
}

/// Why a system the native birth installer left on the legacy step cannot
/// run there: the weather host's installer refuses the same set (a CustomData
/// law that follows the engine's storage, birth events, emission over
/// distance, a collision module and a shape only the native owner samples).
fn legacy_refusal(system: &Runtime, events: bool) -> Option<&'static str> {
    legacy_refusal_for(&system.emitter, system.custom_law.as_ref().is_some_and(|custom| custom.tracks_storage()), events)
}

fn legacy_refusal_for(emitter: &EmitterParams, custom_tracks_storage: bool, events: bool) -> Option<&'static str> {
    if custom_tracks_storage {
        Some("CustomData curve-cache system refused by the native birth installer")
    } else if events {
        Some("sub-emitter parent refused by the native birth installer")
    } else if crate::particle_runtime::has_distance_emission(emitter) {
        Some("distance-emitting system refused by the native birth installer")
    } else if emitter.collision.is_some() {
        Some("collision system refused by the native birth installer")
    } else if emitter.shape.as_ref().is_some_and(shape_needs_native_birth) {
        Some("native-only shape without its native birth owner")
    } else {
        None
    }
}

/// The `ParticleSystem.Play` of an admitted system: the seed reset of every
/// system (an automatic owner takes the next word of the shared seed
/// manager), and the native birth owner where the route and modules qualify,
/// with its birth events. A pooled copy draws its seed when the host prepares
/// it, which is earlier in the shared manager's order than the source's Play;
/// the words of an automatic owner are the manager's, so which system takes
/// which word is the only difference.
fn install(system: &mut Runtime, planned: &Planned,
    seeds: &mut crate::particle_runtime::seed::SystemSeedManager) -> Result<Played, String> {
    install_with(system, &planned.route, planned.event_edges.clone(), planned.culling.clone(), seeds)
}

/// [`install`] for another host's admitted system (the harvest stay
/// particles), which carries no birth events and runs no culling pass.
pub(crate) fn install_played(system: &mut Runtime, route: &crate::particle_runtime::SourceRoute,
    seeds: &mut crate::particle_runtime::seed::SystemSeedManager) -> Result<Played, String> {
    install_with(system, route, None, lifecycle::Culling::Refused("the host runs no culling pass".into()), seeds)
}

fn install_with(system: &mut Runtime, route: &crate::particle_runtime::SourceRoute,
    events: Option<crate::particle_runtime::EventEdges>, culling: lifecycle::Culling,
    seeds: &mut crate::particle_runtime::seed::SystemSeedManager) -> Result<Played, String> {
    if system.emitter.random_seed.is_none() || system.emitter.auto_random_seed.is_none() {
        return Err("source seed ownership is unknown".into());
    }
    if system.emitter.auto_random_seed == Some(true) {
        seeds.try_init().map_err(|error| format!("seed entropy preparation failed: {error}"))?;
    }
    let legacy = match crate::particle_runtime::install_native_birth(system, seeds, route, None) {
        Ok(crate::particle_runtime::BirthPath::Native) => {
            if let Some(edges) = events.clone() {
                system.native_birth.as_mut().expect("native birth owner just installed").events =
                    Some(crate::particle_runtime::BirthEvents::with_edges(edges));
            }
            None
        }
        Ok(crate::particle_runtime::BirthPath::Legacy(reason)) => {
            if let Some(refusal) = legacy_refusal(system, events.is_some()) {
                return Err(format!("{refusal}: {reason}"));
            }
            Some(reason)
        }
        Err(error) => return Err(format!("source seed owner unavailable: {error}")),
    };
    let procedural_warm = system.emitter.prewarm && system.emitter.looping
        && *route == crate::particle_runtime::SourceRoute::Procedural;
    Ok(Played {
        play: lifecycle::PlayState::played(culling.clone(), procedural_warm),
        culling,
        procedural_warm,
        emitting: true,
        route: route.clone(),
        event_edges: events,
        legacy,
        dropped: 0,
    })
}

/// One frame's clocks: `Time.deltaTime`, `Time.unscaledDeltaTime` when the
/// player keeps it, and the frame's time for the play state.
pub(crate) struct FrameClocks {
    pub(crate) scaled: f32,
    pub(crate) unscaled: Option<f32>,
    pub(crate) now: f64,
}

/// One frame of a played system, as the weather host steps its systems: the
/// first Play's warm before its first frame (a looping prewarm system), then,
/// while the play state keeps it in the manager, the per-frame update on the
/// clock its useUnscaledTime selects (`advance_frame`: the frame head with
/// the emitter velocity and the emission over distance, then the incremental
/// slices), emitting until Stop. Birth-event commands find no installed
/// target in this host and are dropped, counted. `Err` names a refused step;
/// the caller retires the system, which draws nothing more.
pub(crate) fn step_played(system: &mut Runtime, played: &mut Played, clocks: &FrameClocks, ctx: &Context)
    -> Result<(), String> {
    if !system.prewarmed {
        system.prewarmed = true;
        if system.emitter.prewarm && system.emitter.looping && played.emitting {
            if let Err(error) = crate::particle_runtime::prewarm_first_play(system, ctx) {
                error!(%error, effect=%system.effect, node=%system.node, "fixture prewarm refused");
            }
        }
    }
    if played.play.managed() {
        let dt = match system.emitter.use_unscaled_time {
            Some(true) => clocks.unscaled,
            _ => Some(clocks.scaled),
        };
        let Some(dt) = dt else {
            system.refused_total += 1;
            return Err("useUnscaledTime system has no unscaled clock this frame".into());
        };
        let now = clocks.now;
        let play = &mut played.play;
        crate::particle_runtime::advance_frame(system, dt, played.emitting, ctx, |s| play.slice_start(s, now))?;
        play.end_update(system, now, dt != 0.0);
    }
    let dropped = system.native_birth.as_mut().and_then(|birth| birth.events.as_mut())
        .map_or(0, |events| events.take_commands().len() as u64);
    if dropped > 0 {
        if played.dropped == 0 {
            warn!(effect=%system.effect, node=%system.node,
                "sub-emitter commands dropped: this host installs no sub-emitter target");
        }
        played.dropped += dropped;
    }
    Ok(())
}

/// `ParticleSystem.Stop()` on the root system of an instance, which the
/// effect components call (`ManagedEffect.Stop`): `Stop(withChildren: true,
/// StopEmitting)`, so every played system under `root` stops emitting and its
/// live particles finish their lifetimes; a system holding no particle is
/// cleared and ends its play (see [`lifecycle::PlayState`]). Returns how many
/// systems stopped.
pub(crate) fn stop_emitting(world: &mut World, root: Entity) -> usize {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let draws = played_draws(world, root);
    for &draw in &draws {
        let mut entity = world.entity_mut(draw);
        let mut played = match entity.take::<Played>() { Some(played) => played, None => continue };
        played.emitting = false;
        if let Some(mut live) = entity.get_mut::<crate::uber_particle::FixtureParticleLive>() {
            if let Some(reason) = played.play.stop(&mut live.0, now) {
                warn!("[fixture-source] {reason}");
            }
        }
        entity.insert(played);
    }
    draws.len()
}

/// `ParticleSystem.Play()` again on the root system of an instance (children
/// included). A system that still holds particles keeps its seeds and warm
/// and restarts its clock (`later_play`); one that holds none plays as at its
/// first Play: its seeds are reset (an automatic owner takes a new word) and a
/// looping prewarm system warms again.
pub(crate) fn play(world: &mut World, root: Entity) -> Result<usize, String> {
    let draws = played_draws(world, root);
    world.resource_scope(|world, mut seeds: Mut<crate::particle_runtime::seed::SystemSeedManager>| {
        for &draw in &draws {
            let mut entity = world.entity_mut(draw);
            let mut played = match entity.take::<Played>() { Some(played) => played, None => continue };
            let Some(mut live) = entity.get_mut::<crate::uber_particle::FixtureParticleLive>() else {
                entity.insert(played);
                continue;
            };
            let system = &mut live.0;
            played.emitting = true;
            played.play = lifecycle::PlayState::played(played.culling.clone(), played.procedural_warm);
            if let Err(error) = crate::particle_runtime::play_after_stop(system, &mut seeds, &played.route,
                played.event_edges.clone()) {
                return Err(format!("{}: {error}", system.node));
            }
            entity.insert(played);
        }
        Ok(draws.len())
    })
}

/// `ParticleSystem.Play()` after Stop on played systems a host moved out of
/// [`crate::uber_particle::FixtureParticleLive`] into its own component `C`
/// (their [`Played`] record stays on the draw), as [`play`] plays its own:
/// see [`crate::particle_runtime::play_after_stop`]. A refused system is
/// named at ERROR and left as it was.
pub(crate) fn play_moved<C: Component<Mutability = bevy::ecs::component::Mutable>>(world: &mut World,
    draws: &[Entity], runtime: impl Fn(&mut C) -> &mut Runtime) {
    world.resource_scope(|world, mut seeds: Mut<crate::particle_runtime::seed::SystemSeedManager>| {
        for &draw in draws {
            let mut entity = world.entity_mut(draw);
            let Some(played) = entity.get::<Played>() else { continue };
            let (route, edges) = (played.route.clone(), played.event_edges.clone());
            let Some(mut component) = entity.get_mut::<C>() else { continue };
            let system = runtime(&mut *component);
            if let Err(error) = crate::particle_runtime::play_after_stop(system, &mut seeds, &route, edges) {
                error!("[fixture-source] {}: Play after Stop refused: {error}", system.node);
            }
        }
    });
}

/// Whether any played system under `root` is playing (`None` when it has
/// none): what `OnShotEffect.Update` reads to finish an effect.
pub(crate) fn instance_playing(world: &World, root: Entity) -> Option<bool> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut any = None;
    for draw in played_draws(world, root) {
        let (Some(live), Some(played)) = (world.get::<crate::uber_particle::FixtureParticleLive>(draw),
            world.get::<Played>(draw)) else { continue; };
        let playing = played.playing(&live.0, now);
        any = Some(any.unwrap_or(false) || playing);
    }
    any
}

/// The played draws of an instance: its children that carry [`Played`].
fn played_draws(world: &World, root: Entity) -> Vec<Entity> {
    world.get::<Children>(root).map(|children| children.iter()
        .filter(|child| world.get::<Played>(*child).is_some()).collect()).unwrap_or_default()
}

fn runtime(planned: &Planned, anchor: Entity, mesh: Handle<Mesh>) -> Runtime {
    let geometry = match &planned.geometry {
        PlannedGeometry::Billboard(draw) => crate::particle_runtime::Geometry::SourceBillboard(draw.clone()),
        PlannedGeometry::Mesh { source, alignment, scaling, pivot, flip, axis_body, .. } => crate::particle_runtime::Geometry::Mesh(
            crate::particle_geometry::MeshDraw { source: source.clone().expect("prepared source mesh"),
                alignment: *alignment, scaling: *scaling, pivot: *pivot, flip: *flip, axis_body: *axis_body }),
        PlannedGeometry::EmptyMesh { alignment, scaling, pivot } => crate::particle_runtime::Geometry::Mesh(
            crate::particle_geometry::MeshDraw::empty(*alignment, *scaling, *pivot)),
    };
    Runtime {
        node: planned.node.clone(), effect: planned.effect.clone(), emitter: planned.emitter.clone(),
        kind: EffectKind::Site, camera_rotation: false, node_affine: GlobalTransform::IDENTITY,
        mesh, anchor: Some(anchor), geometry,
        emission_surface: planned.emission_surface.as_ref().map(|surface| surface.source.clone().expect("prepared source surface")),
        ring_cursor: 0, pool: Vec::new(), side: Vec::new(), emission: EmissionState::default(),
        playback_head: 0.0, previous_head: 0.0, emission_started: false,
        native_birth: None, noise: None, trail: None, collision: None,
        rng: Rng(RNG_SEED ^ (planned.ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        prewarmed: false, pending: 0.0, sub_emitter_max_lifetime: planned.sub_emitter_max_lifetime, cone_angle: planned.cone_angle, rol: planned.rol.clone(), limit: planned.limit.clone(),
        velocity_law: planned.emitter.velocity_over_lifetime.as_ref().map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission")),
        force_law: planned.emitter.force.as_ref().map(|p|
            moly_law::particle::force::ForceOverLifetime::from_params(p).expect("force validated during admission")),
        gravity_law: moly_law::particle::gravity::Gravity::new(&planned.emitter.start.gravity_modifier).expect("curves validated during admission"),
        custom_law: planned.emitter.custom_data.as_ref().map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission")),
        texture_sheet: planned.emitter.texture_sheet.as_ref().map(|p|
            moly_law::particle::texture_sheet::TextureSheet::from_params(p).expect("sheet validated during admission")),
        sort_mode: planned.source.sort_mode,
        size_law: planned.emitter.size_over_lifetime.as_ref().map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission")),
        color_law: planned.emitter.color_over_lifetime.as_ref().map(moly_law::particle::color::ColorOverLifetime::from_params),
        born_total: 0, died_total: 0, full_total: 0, refused_total: 0,
    }
}
