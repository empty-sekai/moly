//! Placement on each harvest-site arrival (`HarvestSiteController.Initialize`).
//!
//! The source reads the user harvest map of the site at site setup, before
//! the site is revealed, and loads one fixture per frame
//! (`LoadFixtureParallelAsync`, `DelayFrame(1)` per row); the unclaimed drops
//! of the map are re-created in parallel. Here the same runs when a harvest
//! site becomes active and settled (plain load or the cannon swap; the site
//! move's reveal hold keeps the new objects hidden until it lets go).
//!
//! Per row (`HarvestObjectPresenter.Setup`): world position = site origin +
//! (x, 0, z) snapped to the navigation surface
//! (`MoveUtility.TryGetCanNavmeshTargetPosition` from the master site
//! position, ten tries shrinking toward it), a log line when the snap moved
//! it more than 5 m; then by status: below harvested, `Setup` (one yaw draw,
//! and for trees and stones one scale draw in [key 130, key 131) that also
//! scales the radius) and the collision registration; harvested,
//! `ForceChangeAfterObject` (for every kind the mock places nothing remains)
//! and no collision.
//!
//! Product frame: one site is loaded at a time with its origin at the world
//! origin, and assets import with the source x axis reflected, so a
//! site-local (x, z) lands at (-x, z). Named substitutes: the snap uses the
//! product's walk field (nearest walkable point within 5 m on x/z, a path
//! from the site origin) for `NavMesh.SamplePosition` / `CalculatePath`; the
//! height comes from the navigation surface or the ground vertices; Unity's
//! `Random` is a seeded generator per arrival.

use bevy::asset::{LoadState, RecursiveDependencyLoadState};
use bevy::ecs::system::SystemParam;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use moly_assets::source_navigation::{SourceHarvestView, SourceObjectIdentity};

use super::catalog::{HarvestCatalog, HarvestUserData};
use super::server_mock::{UserDrop, UserFixture, DROP_BEFORE, DROP_DROPPED};
use super::{
    kind_cues, DropBatch, HarvestDocs, HarvestDropBatches, HarvestGltfs, HarvestObject,
    HarvestRoot, PendingDrop, STATUS_HARVESTED,
};
use crate::walk_face::WalkFace;

/// `GetCreateHarvestObjectPosition`: a snap further than this is logged.
const SNAP_LOG_DISTANCE: f32 = 5.0;
/// `NavMesh.SamplePosition` max distance of the snap.
const SAMPLE_DISTANCE: f32 = 5.0;
/// Tries of `TryGetCanNavmeshTargetPosition`.
const SNAP_TRIES: i32 = 10;

/// The current arrival's placement run.
#[derive(Resource, Default)]
pub(crate) struct HarvestArrival {
    placed_epoch: Option<u64>,
    run: Option<ArrivalRun>,
    next_uid: u64,
}

struct ArrivalRun {
    site_id: u32,
    epoch: u64,
    fixtures: Vec<UserFixture>,
    drops: Vec<UserDrop>,
    next: usize,
    unclaimed: bool,
    rng: super::Rng,
    /// `SetPosition(end)` after the ordinary placement, by fixture index
    /// (the paper airplane's box).
    forced: std::collections::HashMap<usize, Vec3>,
}

impl HarvestArrival {
    /// Objects of the current arrival still to spawn (the scene-ready latch
    /// counts against the spawned total).
    pub(crate) fn in_progress(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.next < run.fixtures.len() || !run.unclaimed)
    }

    pub(crate) fn site_id(&self) -> Option<u32> {
        self.run.as_ref().map(|run| run.site_id)
    }

    /// `CreateTreasureBox`: the transported box's fixture row (and its drop
    /// rows) go through the ordinary placement (`presenter.Setup`: the snap
    /// and one yaw draw), then `SetPosition(end)`. False without a run on
    /// that site.
    pub(crate) fn append_transported(
        &mut self,
        site_id: u32,
        fixture: UserFixture,
        drops: Vec<UserDrop>,
        end: Vec3,
    ) -> bool {
        let Some(run) = self.run.as_mut().filter(|run| run.site_id == site_id) else {
            return false;
        };
        run.forced.insert(run.fixtures.len(), end);
        run.fixtures.push(fixture);
        run.drops.extend(drops);
        true
    }

    /// The site goes; the last seen epoch stays (it is monotonic across
    /// transitions, and the next site starts only on a newer one).
    pub(crate) fn clear(&mut self) {
        self.run = None;
    }
}

#[derive(SystemParam)]
pub(crate) struct ArrivalInputs<'w, 's> {
    server: Res<'w, AssetServer>,
    gltfs: Res<'w, Assets<Gltf>>,
    json: Res<'w, Assets<moly_assets::json::JsonAsset>>,
    catalog: Option<Res<'w, HarvestCatalog>>,
    user: Option<Res<'w, HarvestUserData>>,
    site: Option<Res<'w, crate::site::SiteActive>>,
    epoch: Option<Res<'w, crate::site::GroundEpoch>>,
    face: Option<Res<'w, WalkFace>>,
    objective_face: Option<Res<'w, crate::npc_objective::ObjectiveFace>>,
    ground: Option<Res<'w, crate::site::GroundMeshes>>,
    meshes: Res<'w, Assets<Mesh>>,
    parts: Query<'w, 's, (&'static Mesh3d, &'static GlobalTransform)>,
    configs: Option<Res<'w, crate::client_config::ClientConfigs>>,
}

/// Update: start a placement run on a new harvest-site arrival, then spawn
/// one fixture row per frame.
pub(crate) fn place(
    mut commands: Commands,
    inputs: ArrivalInputs,
    mut arrival: ResMut<HarvestArrival>,
    mut glbs: ResMut<HarvestGltfs>,
    mut docs: ResMut<HarvestDocs>,
    mut batches: ResMut<HarvestDropBatches>,
    mut ground_verts: ResMut<super::HarvestGroundVerts>,
    mut spawned: ResMut<super::HarvestSpawnedCount>,
    mut animator_calls: ResMut<super::prop_animator::PropAnimatorCalls>,
) {
    let (Some(catalog), Some(user), Some(site), Some(epoch)) = (
        inputs.catalog.as_deref(),
        inputs.user.as_deref(),
        inputs.site.as_deref(),
        inputs.epoch.as_deref(),
    ) else {
        return;
    };
    // The site's content settles when GroundEpoch advances (the settle
    // marker itself lives only between the scene spawn and the camera
    // framing of one frame). Every epoch is recorded, harvest site or not,
    // so a new site's SiteActive seen before its own settle (still at the
    // previous site's epoch) does not start an arrival on stale ground.
    if arrival.run.is_none() && arrival.placed_epoch != Some(epoch.0) {
        arrival.placed_epoch = Some(epoch.0);
        if site.category != "harvest" {
            return;
        }
        let Some(map) = user.maps.get(&site.site_id) else {
            warn!(
                "[harvest] HarvestSiteController.Initialize site {}: the user data has no harvest map for it (userMysekaiHarvestMap is null)",
                site.site_id
            );
            return;
        };
        for fixture in &map.fixtures {
            let def = catalog
                .fixtures
                .get(&fixture.fixture_id)
                .unwrap_or_else(|| panic!("map fixture {} has no master row", fixture.fixture_id));
            request(&inputs.server, catalog, &def.package, &mut glbs, &mut docs);
        }
        for drop in &map.drops {
            if let Some(package) = super::drop_model_package(catalog, drop) {
                request(&inputs.server, catalog, &package, &mut glbs, &mut docs);
            }
        }
        info!(
            "[harvest] HarvestSiteController.Initialize site {} ({}): user harvest map {} fixtures, {} drop rows (HarvestMapMock); loading one row per frame",
            site.site_id,
            site.site_type,
            map.fixtures.len(),
            map.drops.len()
        );
        arrival.run = Some(ArrivalRun {
            site_id: site.site_id,
            epoch: epoch.0,
            fixtures: map.fixtures.clone(),
            drops: map.drops.clone(),
            next: 0,
            unclaimed: false,
            rng: super::Rng(0x4152_5249_0000_0000 ^ ((site.site_id as u64) << 32) ^ epoch.0),
            forced: Default::default(),
        });
        spawned.0 = 0;
    }
    let (Some(face), Some(ground), Some(configs)) = (
        inputs.face.as_deref(),
        inputs.ground.as_deref(),
        inputs.configs.as_deref(),
    ) else {
        return;
    };
    let next_uid = arrival.next_uid;
    let Some(run) = arrival.run.as_mut() else {
        return;
    };
    if run.site_id != site.site_id || run.epoch != epoch.0 {
        return;
    }
    let verts = ground_verts
        .0
        .get_or_insert_with(|| super::ground_world_verts(ground, &inputs.meshes, &inputs.parts))
        .clone();
    let height_at = |x: f32, z: f32| -> f32 {
        let face_point = inputs
            .objective_face
            .as_deref()
            .filter(|surface| {
                surface.navigation_generation() == face.generation() && surface.is_fresh(epoch.0)
            })
            .and_then(|surface| surface.navigation_point_at([x, z]));
        match face_point {
            Some(point) => point[1],
            None => super::surface_y(&verts, x, z, 0.0) + face.height_offset([x, z]),
        }
    };
    // GenerateUnclaimedDropItemsAsync: rows already dropped whose threshold
    // is at or above the fixture's hp come back on the ground.
    if !run.unclaimed {
        let mut all_ready = true;
        let mut queued = 0usize;
        let mut pending_batches = Vec::new();
        for fixture in &run.fixtures {
            let rows: Vec<PendingDrop> = run
                .drops
                .iter()
                .filter(|drop| {
                    drop.position_x == fixture.position_x
                        && drop.position_z == fixture.position_z
                        && drop.status == DROP_DROPPED
                        && drop.hp >= fixture.hp
                })
                .map(|drop| super::pending_drop(catalog, drop))
                .collect();
            if rows.is_empty() {
                continue;
            }
            for row in &rows {
                if let super::DropPrefab::Model { package } = &row.prefab {
                    all_ready &=
                        glbs.ready(&inputs.server, package) && docs.ready(&inputs.server, package);
                }
            }
            queued += rows.len();
            let def = &catalog.fixtures[&fixture.fixture_id];
            pending_batches.push(DropBatch {
                origin: None,
                position_x: fixture.position_x,
                position_z: fixture.position_z,
                fixture_type: def.view_type,
                remaining: rows,
                delay: false,
            });
        }
        if all_ready {
            if queued > 0 {
                info!(
                    "[harvest] CreateUnclaimedDropItems: {queued} dropped rows re-created on the ground"
                );
            }
            batches.0.extend(pending_batches);
            run.unclaimed = true;
        }
    }
    let Some(fixture) = run.fixtures.get(run.next).cloned() else {
        return;
    };
    let def = catalog.fixtures[&fixture.fixture_id].clone();
    // A row appended after the arrival (the paper airplane's box) requests
    // its package here; the arrival's own rows were requested at its start.
    request(&inputs.server, catalog, &def.package, &mut glbs, &mut docs);
    if !glbs.ready(&inputs.server, &def.package) || !docs.ready(&inputs.server, &def.package) {
        return;
    }
    let (Some(gltf), Some(doc)) = (
        glbs.get(&def.package)
            .and_then(|handle| inputs.gltfs.get(handle)),
        docs.0
            .get(&def.package)
            .and_then(|handle| inputs.json.get(handle)),
    ) else {
        return;
    };
    let scene_index = super::prefab_scene_index(&doc.0, &def.leaf);
    let scene = gltf.scenes.get(scene_index).cloned().unwrap_or_else(|| {
        panic!(
            "{}: prefab scene {scene_index} is out of range ({})",
            def.leaf,
            gltf.scenes.len()
        )
    });
    // Site origin + (x, 0, z) in the source frame; reflected x in ours.
    let raw = Vec2::new(-(fixture.position_x as f32), fixture.position_z as f32);
    let (snapped, tries) = snap(face, raw);
    let shift = snapped.distance(raw);
    if shift > SNAP_LOG_DISTANCE {
        info!(
            "[harvest] {}#{} at ({}, {}): could not generate at the specified coordinates, generating it shifted by {shift:.2} m",
            def.leaf, def.id, fixture.position_x, fixture.position_z
        );
    }
    let y = height_at(snapped.x, snapped.y);
    let alive = fixture.status < STATUS_HARVESTED;
    // HarvestBaseView.Setup: Random.Range(0f, 360f) * 0.017453292 (the yaw),
    // then SetupScaleRandom for trees and stones (Random.Range(min, max) of
    // keys 130 / 131 scales the transform and the radius).
    let (yaw_deg, scale) = if alive {
        let yaw = run.rng.next_f32() * 360.0;
        let scale = if super::scales_randomly(def.class) {
            let min = configs.float(crate::client_config::KEY_HARVEST_OBJECT_SCALE_MIN);
            let max = configs.float(crate::client_config::KEY_HARVEST_OBJECT_SCALE_MAX);
            min + run.rng.next_f32() * (max - min)
        } else {
            1.0
        };
        (yaw, scale)
    } else {
        (0.0, 1.0)
    };
    let yaw = yaw_deg * 0.017_453_292;
    let pending_drops: Vec<PendingDrop> = run
        .drops
        .iter()
        .filter(|drop| {
            drop.position_x == fixture.position_x
                && drop.position_z == fixture.position_z
                && drop.status == DROP_BEFORE
                && drop.hp <= fixture.hp
        })
        .map(|drop| super::pending_drop(catalog, drop))
        .collect();
    let pending = pending_drops.len();
    let uid = next_uid + run.next as u64 + 1;
    let cues = kind_cues(def.class, def.is_rare);
    let forced = run.forced.remove(&run.next);
    if let Some(end) = forced {
        info!(
            "[harvest] {}#{} SetPosition({:.3}, {:.3}, {:.3}): the placed position gives way to the airplane's landing point",
            def.leaf, def.id, end.x, end.y, end.z
        );
    }
    let entity = commands
        .spawn((
            SceneRoot(scene),
            HarvestRoot,
            HarvestObject {
                uid,
                site_id: run.site_id,
                package: def.package.clone(),
                leaf: def.leaf.clone(),
                class: def.class,
                fixture_type: def.view_type,
                fixture_id: def.id,
                position_x: fixture.position_x,
                position_z: fixture.position_z,
                hp: fixture.hp,
                prev_hp: fixture.hp,
                status: fixture.status,
                is_last_attack: false,
                last_attack_stamina: def.last_attack_stamina,
                is_rare: def.is_rare,
                radius: def.radius * scale,
                scale,
                collision: alive,
                collision_type: def.collision_type,
                interface: def.interface,
                cues,
                assetbundle: def.assetbundle.clone(),
                pending_drops,
                hits_taken: 0,
            },
            Transform::from_translation(forced.unwrap_or(Vec3::new(snapped.x, y, snapped.y)))
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(scale)),
            if alive || def.class == "MysekaiAreaTreasureBoxView" {
                Visibility::default()
            } else {
                Visibility::Hidden
            },
        ))
        .id();
    // ForceChangeAfterObject of a box loaded as harvested: SetBool("opened")
    // (and its lid obstacle): the open box stays.
    if !alive && def.class == "MysekaiAreaTreasureBoxView" {
        animator_calls.0.push((
            entity,
            super::prop_animator::PropCall::SetBool("opened", true),
        ));
    }
    info!(
        "[harvest] placed {}#{} ({} {}) map (x, z) = ({}, {}) -> product ({:.2}, {:.2}) snapped ({:.2}, {:.2}, {:.2}) shift {:.2} m in {tries} tries, yaw {:.1} deg, scale {:.3}, radius {:.3}, hp {}, status {}{}, {pending} pending drop rows, entity {entity:?}",
        def.leaf,
        def.id,
        super::kind_word(def.master_kind),
        def.class,
        fixture.position_x,
        fixture.position_z,
        raw.x,
        raw.y,
        snapped.x,
        y,
        snapped.y,
        shift,
        yaw_deg,
        scale,
        def.radius * scale,
        fixture.hp,
        fixture.status,
        if alive {
            ""
        } else {
            if def.class == "MysekaiAreaTreasureBoxView" {
                " (loaded as harvested: ForceChangeAfterObject, the box stays open)"
            } else {
                " (loaded as harvested: ForceChangeAfterObject, nothing remains)"
            }
        },
    );
    run.next += 1;
    spawned.0 += 1;
    let finished = run.next == run.fixtures.len();
    if finished {
        info!(
            "[harvest] LoadFixtureParallelAsync done: {} rows on site {}",
            run.next, run.site_id
        );
    }
    let total = run.fixtures.len() as u64;
    if finished {
        arrival.next_uid += total;
    }
}

fn request(
    server: &AssetServer,
    catalog: &HarvestCatalog,
    package: &str,
    glbs: &mut HarvestGltfs,
    docs: &mut HarvestDocs,
) {
    let files = catalog
        .packages
        .get(package)
        .unwrap_or_else(|| panic!("harvest package {package} is not in the index"));
    glbs.by_key.entry(package.to_owned()).or_insert_with(|| {
        moly_assets::residency::load_gltf(
            server,
            bevy::asset::AssetPath::from(format!("moly://site/{}", files.glb)),
            moly_assets::residency::GltfResidency::GpuTextures,
        )
    });
    docs.0.entry(package.to_owned()).or_insert_with(|| {
        server.load::<moly_assets::json::JsonAsset>(bevy::asset::AssetPath::from(format!(
            "moly://site/{}",
            files.document
        )))
    });
}

impl HarvestGltfs {
    pub(crate) fn get(&self, package: &str) -> Option<&Handle<Gltf>> {
        self.by_key.get(package)
    }

    /// Loaded with dependencies; a failed load stops loudly.
    pub(crate) fn ready(&self, server: &AssetServer, package: &str) -> bool {
        let Some(handle) = self.by_key.get(package) else {
            return false;
        };
        if let LoadState::Failed(error) = server.load_state(handle) {
            panic!("harvest glb {package} failed to load: {error:?}");
        }
        if let RecursiveDependencyLoadState::Failed(error) =
            server.recursive_dependency_load_state(handle)
        {
            panic!("harvest glb {package} dependencies failed to load: {error:?}");
        }
        server.is_loaded_with_dependencies(handle)
    }
}

impl HarvestDocs {
    pub(crate) fn ready(&self, server: &AssetServer, package: &str) -> bool {
        let Some(handle) = self.0.get(package) else {
            return false;
        };
        match server.load_state(handle) {
            LoadState::Failed(error) => {
                panic!("harvest document {package} failed to load: {error:?}")
            }
            LoadState::Loaded => true,
            _ => false,
        }
    }
}

/// `MoveUtility.TryGetCanNavmeshTargetPosition(from = site origin, target)`:
/// ten tries of sample-then-path; after a failed try the target moves toward
/// `from` by `clamp01(k / 10)` with k from 9 down; all failing returns `from`.
pub(crate) fn snap(face: &WalkFace, target: Vec2) -> (Vec2, i32) {
    snap_from(face, Vec2::ZERO, target)
}

/// `TryGetCanNavmeshTargetPosition(from, target)` on the walk field: ten
/// tries, each sampling within 5 m and requiring a path from `from`,
/// shrinking the target toward `from` between tries; `from` itself when
/// none succeeds.
pub(crate) fn snap_from(face: &WalkFace, from: Vec2, target: Vec2) -> (Vec2, i32) {
    let mut current = target;
    for (attempt, k) in (0..SNAP_TRIES).rev().enumerate() {
        if let Some(hit) = face.sample(current.to_array(), SAMPLE_DISTANCE) {
            if face.path(from.to_array(), hit).is_some() {
                return (Vec2::from_array(hit), attempt as i32 + 1);
            }
        }
        current = from + (current - from) * (k as f32 / 10.0).clamp(0.0, 1.0);
    }
    (from, SNAP_TRIES)
}

/// A harvest view's node references, resolved once the prefab scene exists
/// (`SourceHarvestView` fields hold source object ids; the nodes carry their
/// `SourceObjectIdentity`).
#[derive(Component, Debug, Default)]
pub(crate) struct HarvestViewNodes {
    /// Tree `woodBeforeObject`, stone `stoneObject`, plant
    /// `_plantBeforeObject`, junk and toolbox `_junkPrefab`, driftage
    /// `_driftageObject`, birthday plant `_objectPrefab`.
    pub(crate) object: Option<Entity>,
    /// Tree `woodAfterMeshRenderer` (the trunk top that falls).
    pub(crate) after: Option<Entity>,
    /// Tree `FindUnderTree` (the part that stays).
    pub(crate) under: Option<Entity>,
    /// Tree `_woodDeleteEffectPosition`.
    pub(crate) delete_at: Option<Entity>,
}

/// Update: bind each spawned harvest scene's view nodes once and apply the
/// view's Setup visibility (a tree hides its after mesh and its under part).
#[allow(clippy::type_complexity)]
pub(crate) fn bind_views(
    mut commands: Commands,
    roots: Query<
        (Entity, &HarvestObject, &Children),
        (With<HarvestRoot>, Without<HarvestViewNodes>),
    >,
    children: Query<&Children>,
    views: Query<&SourceHarvestView>,
    identities: Query<&SourceObjectIdentity>,
    names: Query<&Name>,
    mut visibility: Query<&mut Visibility>,
) {
    for (root, object, _) in &roots {
        let mut nodes = Vec::new();
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            nodes.push(entity);
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        let Some(view) = nodes.iter().find_map(|entity| views.get(*entity).ok()) else {
            // The scene has not expanded to its view node yet.
            continue;
        };
        let fields: serde_json::Value = serde_json::from_str(&view.fields_json)
            .unwrap_or_else(|error| panic!("{}: view fields are not JSON: {error}", object.leaf));
        // A reference in the view's exported fields is `{file, id}` with the
        // signed path id as a decimal string; id 0 is a null reference, and a
        // reference into another file names no node of this scene.
        let id_of = |field: &str| {
            let reference = fields.get(field)?;
            if reference["file"].as_i64() != Some(0) {
                return None;
            }
            reference["id"]
                .as_str()
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id != 0)
        };
        let by_game_object = |id: i64| {
            nodes.iter().copied().find(|entity| {
                identities
                    .get(*entity)
                    .is_ok_and(|identity| identity.game_object == id)
            })
        };
        let by_component = |id: i64| {
            nodes.iter().copied().find(|entity| {
                identities
                    .get(*entity)
                    .is_ok_and(|identity| identity.components.contains(&id))
            })
        };
        let by_transform = |id: i64| {
            nodes.iter().copied().find(|entity| {
                identities
                    .get(*entity)
                    .is_ok_and(|identity| identity.transform == id)
            })
        };
        let object_field = match object.class {
            "MysekaiAreaTreeView" => "woodBeforeObject",
            "MysekaiAreaStoneView" => "stoneObject",
            "MysekaiAreaPlantView" => "_plantBeforeObject",
            "MysekaiAreaJunkView" | "MysekaiAreaToolBoxView" => "_junkPrefab",
            "MysekaiAreadDriftageView" => "_driftageObject",
            "MysekaiBirthdayPlantView" => "_objectPrefab",
            _ => "",
        };
        let mut bound = HarvestViewNodes {
            object: id_of(object_field).and_then(by_game_object),
            ..default()
        };
        if object.class == "MysekaiAreaTreeView" {
            bound.after = id_of("woodAfterMeshRenderer").and_then(by_component);
            bound.delete_at = id_of("_woodDeleteEffectPosition").and_then(by_transform);
            // FindUnderTree: the first transform under the view whose name
            // contains "under", ignoring case (GetComponentsInChildren order is
            // depth first; ours is the scene's depth-first order too).
            bound.under = nodes.iter().copied().skip(1).find(|entity| {
                names
                    .get(*entity)
                    .is_ok_and(|name| name.as_str().to_ascii_lowercase().contains("under"))
            });
            // Setup: the after mesh and the under part start inactive.
            for node in [bound.after, bound.under].into_iter().flatten() {
                if let Ok(mut vis) = visibility.get_mut(node) {
                    *vis = Visibility::Hidden;
                }
            }
        }
        if !object_field.is_empty() && bound.object.is_none() {
            warn!(
                "[harvest] {}#{}: view field {object_field} names no node of the scene; its disappearance hides the whole object",
                object.leaf, object.fixture_id
            );
        }
        info!(
            "[harvest] view bound {}#{} {}: object {:?} after {:?} under {:?} delete-position {:?}",
            object.leaf,
            object.fixture_id,
            object.class,
            bound.object,
            bound.after,
            bound.under,
            bound.delete_at
        );
        commands.entity(root).insert(bound);
    }
}
