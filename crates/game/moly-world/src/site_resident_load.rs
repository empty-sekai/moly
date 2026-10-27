//! Real asynchronous initialization of listed sites before their first visit.
//! Each job owns its roots, fixture loader, geometry and immutable input stamp.
use super::*;
use crate::fixture::residency::ResidentFixtureLoad;
use bevy::ecs::system::SystemState;

#[derive(Component)]
pub struct ActiveSiteMesh;
#[derive(Component)]
struct MeshOwnershipPublished(bool);
struct Load {
    active: SiteActive,
    roots: Vec<Entity>,
    handles: Vec<Handle<Mesh>>,
    fixtures: Option<ResidentFixtureLoad>,
    last_error: String,
}
#[derive(Resource, Default)]
pub(crate) struct Loads(HashMap<String, Load>);

/// A newly validated external import replaces every saved-site owner, including
/// unvisited preload jobs. It cannot restore hidden furniture from an old file.
pub fn reset_import(world:&mut World) {
    if let Some(loads)=world.remove_resource::<Loads>() {
        for (_,load) in loads.0 {
            despawn_now(world,&load.roots);
            if let Some(fixtures)=load.fixtures { fixtures.dispose(world); }
        }
    }
    if let Some(residents)=world.remove_resource::<ResidentSites>() {
        for site in residents.sites { despawn_now(world,site.view.roots()); }
    }
    crate::fixture::residency::clear_residents(world);
    world.insert_resource(ResidentSites::default());
    world.insert_resource(Loads::default());
    world.insert_resource(crate::npc::residency::SiteNavigation::default());
}

/// A handle may occur in two instances of the same room prefab. Navigation
/// reads only the mesh entities of the active instance, never by handle alone.
pub(crate) fn publish_mesh_owners(world: &mut World) {
    let roots: Vec<_> = world.query_filtered::<Entity, Or<(With<SiteRoot>, With<ResidentSiteRoot>)>>().iter(world).collect();
    for root in roots {
        let active = world.get::<SiteRoot>(root).is_some();
        let ready=world.get::<bevy::world_serialization::WorldInstance>(root).is_some_and(|instance|world.resource::<bevy::world_serialization::WorldInstanceSpawner>().instance_is_ready(**instance));
        if !ready || world.get::<MeshOwnershipPublished>(root).is_some_and(|prior|prior.0==active) { continue; }
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
            if world.get::<Mesh3d>(entity).is_none() { continue; }
            let mut node = world.entity_mut(entity);
            if active { node.insert(ActiveSiteMesh); } else { node.remove::<ActiveSiteMesh>(); }
        }
        world.entity_mut(root).insert(MeshOwnershipPublished(active));
    }
}

pub(crate) fn advance(world: &mut World) {
    let imported=world.contains_resource::<crate::browser_stage::BrowserStage>() && world.contains_resource::<crate::stage_layout::TransientExploration>();
    if world.contains_resource::<TemporarySiteActive>() || (world.contains_resource::<crate::browser_stage::BrowserStage>() && !imported) { return; }
    if !world.contains_resource::<crate::fixture::FixtureScenesReady>() { return; }
    let Some(mut loads) = world.remove_resource::<Loads>() else { return; };
    let listed: Vec<_> = world.resource::<ResidentSites>().sites.iter()
        .filter(|s| matches!(s.view,ResidentView::Loaded))
        .map(|s| (s.site_type.clone(),s.active.clone(),s._family.clone())).collect();
    let retired: Vec<_> = loads.0.keys().filter(|name| !listed.iter().any(|(n,_,_)| n == *name)).cloned().collect();
    for name in retired {
        if let Some(load) = loads.0.remove(&name) {
            despawn_now(world,&load.roots);
            if let Some(fixtures) = load.fixtures { fixtures.dispose(world); }
        }
    }
    // The source awaits each AddSite in order. This keeps one hidden fixture
    // decode/expansion budget alongside the current loader, with no root swap.
    for (name, active, assets) in listed {
        if !loads.0.contains_key(&name) {
            let planned = {
                let server = world.resource::<AssetServer>();
                let gltfs = world.resource::<Assets<Gltf>>();
                let meshes = world.resource::<Assets<GltfMesh>>();
                scene_plan(server,gltfs,meshes,&assets)
            };
            let Some(plan) = planned else { break; };
            let mut roots = Vec::new();
            for (index,(scene, navigation)) in plan.scenes.into_iter().enumerate() {
                let mut root = world.spawn((WorldAssetRoot(scene), Visibility::Hidden,
                    ResidentSiteRoot { site_type: name.clone(), local: Transform::IDENTITY, origin: index == 0 }));
                if navigation { root.insert(SiteNavigationRoot); }
                if let Some((_,level)) = plan.rings.iter().find(|(i,_)| *i == index) {
                    // The retained background remains hidden as a whole. The
                    // active obstacle pass restores the authored level on entry.
                    root.insert(HomeObstacleRing { level: *level });
                }
                roots.push(root.id());
            }
            loads.0.insert(name.clone(),Load { active, roots, handles: plan.face, fixtures: None, last_error: String::new() });
        }
        let load = loads.0.get_mut(&name).expect("inserted resident load");
        let result = advance_load(world,load);
        match result {
            Ok(Some((inputs, navigation))) => {
                let mut load = loads.0.remove(&name).expect("resident load");
                load.fixtures.take().expect("finished fixtures").publish(world,inputs);
                world.resource_mut::<crate::npc::residency::SiteNavigation>().insert(name.clone(),navigation);
                let origin = world.get_resource::<SiteActive>().and_then(|a| world.resource::<Sites>().placement(&a.site_type)).map(|p|p.product_origin()).unwrap_or_default();
                world.resource_scope(|world,mut residents: Mut<ResidentSites>| {
                    if let Some(index) = residents.position(&name) {
                        residents.sites[index].view = ResidentView::Hidden(load.roots);
                        residents.place(world,origin);
                    }
                });
                info!("[site-residency] AddSite({name}) initialized: real scenes, fixture views, occupancy and navigation");
            }
            Ok(None) => break,
            Err(reason) => {
                if load.last_error != reason {
                    info!("[site-residency] AddSite({name}) pending: {reason}");
                    load.last_error = reason;
                }
                break;
            }
        }
    }
    world.insert_resource(loads);
}

type Ready = (std::sync::Arc<crate::fixture_scene_inputs::FixtureSceneInputs>,crate::npc::residency::Navigation);
fn advance_load(world: &mut World, load: &mut Load) -> Result<Option<Ready>,String> {
    let spawner = world.resource::<bevy::world_serialization::WorldInstanceSpawner>();
    if load.roots.iter().any(|root| world.get::<bevy::world_serialization::WorldInstance>(*root).is_none_or(|instance| !spawner.instance_is_ready(**instance))) { return Ok(None); }
    if load.fixtures.is_none() {
        let sites = world.resource::<Sites>();
        let selection = world.resource::<SiteSelection>();
        let content = selection.content();
        let imported=world.contains_resource::<crate::browser_stage::BrowserStage>() && world.contains_resource::<crate::stage_layout::TransientExploration>();
        if !imported && crate::fixture::waits_for_panel_layout(&load.active.site_type,content) { return Ok(None); }
        let homes = world.get_resource::<crate::house::HomeFixtures>();
        if !imported && load.active.site_type == "home_site" && homes.is_none() { return Ok(None); }
        let floor = sites.floor_grid(&load.active.site_type,load.active.level)?;
        let mut placements = if imported { crate::fixture::residency::restore_imported(world,load.active.site_id,&load.active.site_type,load.active.level)? } else { world.resource::<crate::fixture::layouts::SiteFixtureLayouts>().restore(
            load.active.site_id,&load.active.site_type,load.active.level,content,
            world.get_resource::<NavMeshSourceRegion>().copied(),homes)? };
        crate::fixture::layouts::validate_floor_layout(&placements,floor)?;
        placements.set_resident_floor(floor);
        load.fixtures = Some(ResidentFixtureLoad::request(world,placements)?);
    }
    let fixtures = load.fixtures.as_mut().expect("resident fixture loader");
    if !fixtures.advance(world)? { return Ok(None); }
    if world.get_resource::<crate::fixture_attach::AttachPoints>().is_none() { return Ok(None); }
    // Resolve the slow house catalogue before baking so retries never rebake.
    let mut house_door=None;
    let mut house_exit=None;
    if load.active.site_type=="home_site" && !world.contains_resource::<crate::browser_stage::BrowserStage>() {
        match crate::fixture_gimmick::house_door::find_house(world) {
            crate::fixture_gimmick::house_door::HouseLookup::Found(house)=>{
                house_exit=house.npc_exit_program().ok();
                house_door=house.inside_door.and_then(|e|crate::fixture_scene_inputs::live_matrix(world,e)).map(|m|(m.transform_point3(Vec3::ZERO),m.transform_vector3(Vec3::Z).normalize_or_zero()));
            }
            crate::fixture_gimmick::house_door::HouseLookup::Pending(reason)|crate::fixture_gimmick::house_door::HouseLookup::Failed(reason)=>return Err(reason),
            crate::fixture_gimmick::house_door::HouseLookup::Absent=>{}
        }
    }
    let Some(tris) = triangles(world,&load.roots,&load.handles) else { return Ok(None); };
    let epoch = u64::MAX - u64::from(load.active.site_id);
    let inputs = crate::fixture_scene_inputs::resident_inputs(world,&fixtures.placements,&fixtures.roots,load.roots[0],epoch)?;
    let state = &mut SystemState::<crate::fixture_collision::CollisionInputs>::new(world);
    let polygons = state.get(world).map_err(|error| error.to_string())?.collect_roots(&fixtures.roots)?;
    let source = world.get_resource::<NavMeshSourceRegion>().ok_or("navigation source region pending")?;
    let voxel = crate::walk_face::site_voxel(&load.active,source);
    let volume = crate::walk_face::site_volume(world.resource::<Sites>(),&load.active);
    let field = moly_law::carve::WalkField::bake_colliders_in(&tris,&polygons.polygons,voxel,Some(volume));
    let walk = crate::walk_face::WalkFace::from_field(std::sync::Arc::new(field),1);
    // Each immutable owner gets its own epoch; active epochs never overwrite it.
    let objective = crate::npc_objective::face_from_triangles(tris,&walk,epoch,load.active.position[1]);
    let door = if load.active.is_indoor() {
        let mut found = Vec::new();
        for &root in &load.roots {
            let mut stack = vec![root];
            while let Some(entity) = stack.pop() {
                if world.get::<Name>(entity).is_some_and(|n| n.as_str()=="loc_inside") { found.push(entity); }
                if let Some(children)=world.get::<Children>(entity) { stack.extend(children.iter()); }
            }
        }
        match found.as_slice() {
            [point] => crate::fixture_scene_inputs::live_matrix(world,*point).map(|m| (m.transform_point3(Vec3::ZERO),m.transform_vector3(Vec3::Z).normalize_or_zero())),
            _ => None,
        }
    } else { None };
    let navigation = crate::npc::residency::Navigation { walk, objective, door: door.or(house_door), house_exit };
    Ok(Some((inputs,navigation)))
}

fn triangles(world: &World,roots: &[Entity],handles: &[Handle<Mesh>]) -> Option<Vec<[[f32;3];3]>> {
    let meshes=world.resource::<Assets<Mesh>>();
    let mut seen=vec![false;handles.len()];
    let mut result=Vec::new();
    for &root in roots {
        let mut stack=vec![root];
        while let Some(entity)=stack.pop() {
            if let Some(children)=world.get::<Children>(entity) { stack.extend(children.iter()); }
            if world.get::<moly_assets::scene_state::SourceInactive>(entity).is_some() { continue; }
            let Some(mesh)=world.get::<Mesh3d>(entity) else { continue; };
            let Some(index)=handles.iter().position(|h|h.id()==mesh.0.id()) else { continue; };
            seen[index]=true;
            let mesh=meshes.get(&mesh.0)?;
            let vertices=mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
            let matrix=crate::fixture_scene_inputs::live_matrix(world,entity)?;
            let indices: Vec<usize>=mesh.indices().map(|i|i.iter().collect()).unwrap_or_else(||(0..vertices.len()).collect());
            for tri in indices.chunks_exact(3) {
                result.push([tri[0],tri[1],tri[2]].map(|i|matrix.transform_point3(Vec3::from(vertices[i])).to_array()));
            }
        }
    }
    (seen.iter().all(|v|*v) && !result.is_empty()).then_some(result)
}
