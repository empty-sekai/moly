//! Persistent furniture instances and their site's immutable scene inputs.
//! Simulation stays in the owning site's local frame. FixtureRoot denotes the
//! active loader/editor/collision set; identity and timeline ownership survive
//! hiding a site, independently of that active marker.
use super::*;
use std::sync::Arc;

#[derive(Component, Clone)]
pub struct FixtureSiteOwner {
    pub site_id: u32,
    pub site_type: String,
}

#[derive(Component)]
pub struct RetainedFixture {
    visual: Option<FixtureVisualRoot>,
    visibility: Visibility,
}

pub struct FixtureSite {
    pub placements: FixturePlacements,
    pub inputs: Arc<crate::fixture_scene_inputs::FixtureSceneInputs>,
    pub attach: Option<crate::fixture_attach::AttachWorlds>,
    roots: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct FixtureSites(HashMap<String, FixtureSite>);
impl FixtureSites {
    pub fn get(&self, site: &str) -> Option<&FixtureSite> { self.0.get(site) }
}

pub fn actor_owns_fixture(world: &World, actor: Entity, fixture: Entity) -> bool {
    let Some(owner) = world.get::<FixtureSiteOwner>(fixture) else { return false; };
    world.get::<crate::npc::NpcActions>(actor).is_some_and(|actions| actions.site_type == owner.site_type)
}

/// Called before input invalidation and active-layout clearing.
pub fn retain_current(world: &mut World) {
    if world.contains_resource::<TemporaryFixtureLayout>() || !world.contains_resource::<FixtureScenesReady>() { return; }
    let Some(placements) = world.get_resource::<FixturePlacements>().cloned() else { return; };
    let Some(inputs) = world.get_resource::<crate::fixture_scene_inputs::FixtureSceneSupply>()
        .and_then(|s| s.current()).filter(|s| s.site_type == placements.site_type()).cloned() else { return; };
    let roots: Vec<Entity> = world.query_filtered::<(Entity, &FixtureSiteOwner), With<FixtureRoot>>()
        .iter(world).filter(|(_, owner)| owner.site_id == placements.site_id()).map(|(e, _)| e).collect();
    if roots.len() != placements.total() { return; }
    let attach = world.get_resource::<crate::fixture_attach::AttachWorlds>().cloned();
    for &root in &roots {
        let mut entity = world.entity_mut(root);
        let visual = entity.take::<FixtureVisualRoot>();
        let visibility = entity.get::<Visibility>().copied().unwrap_or(Visibility::Inherited);
        entity.remove::<FixtureRoot>();
        entity.insert((RetainedFixture { visual, visibility }, Visibility::Hidden));
    }
    let name = placements.site_type().to_owned();
    world.resource_mut::<FixtureSites>().0.insert(name, FixtureSite { placements, inputs, attach, roots });
}

/// The old active slot can be cleared while all retained scene owners live.
pub fn invalidate_active(world: &mut World) {
    let site = world.get_resource::<FixturePlacements>().map(|p| p.site_type().to_owned());
    let Some(site) = site else { return; };
    let hidden = world.get_resource::<FixtureSites>().and_then(|s| s.get(&site))
        .is_some_and(|s| s.roots.iter().all(|e| world.get::<RetainedFixture>(*e).is_some()));
    if !hidden { remove_site(world, &site); }
}

pub fn remove_site(world: &mut World, site: &str) {
    let Some(context) = world.get_resource_mut::<FixtureSites>().and_then(|mut s| s.0.remove(site)) else { return; };
    for root in context.roots { if let Ok(entity) = world.get_entity_mut(root) { entity.despawn(); } }
}

/// Re-enter the same retained room/home with its exact Entity/UID bindings.
pub fn restore_selected(world: &mut World) {
    if world.contains_resource::<crate::site::TemporarySiteActive>() { return; }
    let Some(selection) = world.get_resource::<crate::site::SiteSelection>() else { return; };
    let Some(sites) = world.get_resource::<crate::site::Sites>() else { return; };
    let Ok(floor) = selection.fixture_floor_grid(sites) else { return; };
    let name = selection.site_type().to_owned();
    if world.get_resource::<FixturePlacements>().is_some_and(|p| p.site_type() == name) { return; }
    let Some(context) = world.get_resource::<FixtureSites>().and_then(|s| s.get(&name)) else { return; };
    if context.placements.floor_grid() != Some(floor) || context.roots.iter().any(|e| world.get::<RetainedFixture>(*e).is_none()) {
        remove_site(world, &name);
        return;
    }
    let placements = context.placements.clone();
    let roots = context.roots.clone();
    let attach = context.attach.clone();
    for root in roots {
        let mut entity = world.entity_mut(root);
        let Some(retained) = entity.take::<RetainedFixture>() else { continue; };
        entity.remove::<crate::render_inputs::HiddenResidentVisual>();
        entity.insert((FixtureRoot, retained.visibility));
        if let Some(visual) = retained.visual { entity.insert(visual); }
    }
    let count = placements.total();
    world.insert_resource(placements);
    world.insert_resource(FixtureGltfAssets::default());
    world.insert_resource(FixtureSpawnedCount(count));
    world.insert_resource(FixtureScenesReadyCount(count));
    world.insert_resource(FixtureScenesReady);
    if let Some(attach) = attach { world.insert_resource(attach); }
    let mut revision = world.resource_mut::<FixtureLayoutRevision>();
    revision.0 = revision.0.checked_add(1).expect("fixture layout revision exhausted");
}

/// A hidden site's real loader. Each source handle transfers to its scene root;
/// neither its counters nor its placement rows replace the active site's state.
pub(crate) struct ResidentFixtureLoad {
    pub placements: FixturePlacements,
    assets: FixtureGltfAssets,
    pub roots: Vec<Entity>,
}

impl ResidentFixtureLoad {
    pub(crate) fn request(world: &World, mut placements: FixturePlacements) -> Result<Self, String> {
        let index = world.get_resource::<FixtureIndexAsset>().ok_or("fixture index pending")?;
        let json = world.resource::<Assets<moly_assets::json::JsonAsset>>().get(&index.0).ok_or("fixture index loading")?;
        let value: serde_json::Value = serde_json::from_str(&json.0).map_err(|e| e.to_string())?;
        let packages = value["packages"].as_object().ok_or("fixture index packages missing")?;
        let imported=world.contains_resource::<crate::browser_stage::BrowserStage>() && world.contains_resource::<crate::stage_layout::TransientExploration>();
        if !imported && placements.site_type() == "home_site" {
            let gate = world.get_resource::<crate::fixture_gate_model::HomeGateModel>().ok_or("home gate model loading")?;
            placements.show_gate_model(gate);
        }
        let mut paths = Vec::with_capacity(placements.total());
        for row in &placements.rows {
            let entry = packages.get(&row.package).ok_or_else(|| format!("resident fixture {} absent from index",row.package))?;
            moly_assets::coordinates::validate_document(entry)?;
            if entry["status"].as_str() != Some("exported") || entry["hasFixtureView"].as_bool() != Some(true) {
                return Err(format!("resident fixture {} is not an exported fixture view",row.package));
            }
            let file = entry["glb"].as_str().ok_or("fixture index glb missing")?;
            paths.push(format!("moly://fixture-models/{file}"));
        }
        Ok(Self { placements, assets: FixtureGltfAssets { paths, handles: HashMap::new() }, roots: Vec::new() })
    }

    pub(crate) fn advance(&mut self, world: &mut World) -> Result<bool, String> {
        let server = world.resource::<AssetServer>().clone();
        self.assets.release_spawned(self.roots.len());
        let pending = self.assets.handles.values().filter(|h| !server.is_loaded_with_dependencies(*h)).count();
        let mut slots = FIXTURE_UNIQUE_LOADS_IN_FLIGHT.saturating_sub(pending);
        for path in &self.assets.paths[self.roots.len()..] {
            if slots == 0 { break; }
            if self.assets.handles.contains_key(path) { continue; }
            let handle = moly_assets::residency::load_gltf(&server, bevy::asset::AssetPath::from(path.clone()), moly_assets::residency::GltfResidency::CpuTextures);
            self.assets.handles.insert(path.clone(),handle);
            slots -= 1;
        }
        let waiting = self.roots.iter().filter(|&&root| world.get::<moly_assets::coordinates::CanonicalCoordinates>(root).is_none()).count();
        let budget = FIXTURE_SCENE_SPAWN_BUDGET.min(FIXTURE_SCENES_IN_FLIGHT.saturating_sub(waiting));
        let occupancy = self.placements.occupancy_rows();
        let mut to_spawn = Vec::new();
        for index in self.roots.len()..(self.roots.len()+budget).min(self.placements.total()) {
            let Some(handle) = self.assets.handles.get(&self.assets.paths[index]) else { break; };
            if let LoadState::Failed(error) = server.load_state(handle) { return Err(format!("resident fixture load: {error:?}")); }
            if let RecursiveDependencyLoadState::Failed(error) = server.recursive_dependency_load_state(handle) { return Err(format!("resident fixture dependency: {error:?}")); }
            if !server.is_loaded_with_dependencies(handle) { break; }
            let Some(gltf) = world.resource::<Assets<Gltf>>().get(handle) else { break; };
            let scene = gltf.default_scene.clone().ok_or("resident fixture has no default scene")?;
            to_spawn.push((index,handle.clone(),scene));
        }
        let mut state = bevy::ecs::system::SystemState::<Commands>::new(world);
        {
            let mut commands = state.get_mut(world).map_err(|error| error.to_string())?;
            for (index, handle, scene) in to_spawn {
                let root = spawn_instance(&mut commands,&self.placements,&occupancy,index,handle,scene);
                commands.entity(root).remove::<FixtureRoot>().insert(crate::render_inputs::HiddenResidentVisual);
                self.roots.push(root);
            }
        }
        state.apply(world);
        Ok(self.roots.len() == self.placements.total() && self.roots.iter().all(|&root| {
            world.get::<moly_assets::coordinates::CanonicalCoordinates>(root).is_some()
                && world.get::<FixtureIdentityResolved>(root).is_some()
                && world.get::<FixtureViewResolved>(root).is_some()
                && world.get::<crate::render_inputs::FixtureVisualReady>(root).is_some()
        }))
    }

    pub(crate) fn dispose(self, world: &mut World) {
        for root in self.roots { if let Ok(entity) = world.get_entity_mut(root) { entity.despawn(); } }
    }

    pub(crate) fn publish(self, world: &mut World, inputs: Arc<crate::fixture_scene_inputs::FixtureSceneInputs>) {
        let attach = world.get_resource::<crate::fixture_attach::AttachPoints>().map(|points| crate::fixture_attach::compose(&self.placements,points));
        for &root in &self.roots {
            let mut entity = world.entity_mut(root);
            let visual = entity.take::<FixtureVisualRoot>();
            entity.insert((RetainedFixture { visual, visibility: Visibility::Inherited },Visibility::Hidden));
        }
        let name = self.placements.site_type().to_owned();
        let count = self.roots.len();
        world.resource_mut::<FixtureSites>().0.insert(name.clone(),FixtureSite {
            placements: self.placements, inputs, attach, roots: self.roots,
        });
        info!("[fixture-residency] initialized {name}: {count} real hidden fixture instances");
    }
}

/// Restore only the exact site of a validated external import. This is kept
/// within the fixture domain so callers cannot invoke local-layout fallback.
pub(crate) fn restore_imported(world: &World, site_id: u32, site_type: &str, level: u32) -> Result<FixturePlacements,String> {
    world.resource::<layouts::SiteFixtureLayouts>().restore_imported(site_id,site_type,level)
}

pub(crate) fn validate_imported(layouts: &layouts::SiteFixtureLayouts, site_id: u32, site_type: &str, floor: crate::site::FloorGridLayout) -> Result<(),String> {
    let placements=layouts.restore_imported(site_id,site_type,floor.level)?;
    layouts::validate_floor_layout(&placements,floor)
}

pub(crate) fn clear_residents(world: &mut World) {
    let names:Vec<_>=world.resource::<FixtureSites>().0.keys().cloned().collect();
    for name in names { remove_site(world,&name); }
}
