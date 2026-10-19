//! PhysicsCollider inputs for the single-surface navigation host.
//!
//! This consumes canonical source geometry, never logical occupancy boxes.
//! A MeshCollider contributes its shared mesh's own triangles, as separate
//! surfaces, whether or not it is marked convex. The engine's collider-to-
//! navigation-source conversion reads only the collider's mesh reference and
//! its transform (position, rotation, world scale) and emits a Mesh source; it
//! never reads the convex flag, and the builder then rasterizes that Mesh's
//! vertex and index data. The physics hull, cooked or mathematical, is not a
//! navigation input: baking it would fill, for example, the space between a
//! house's low porch steps and its eaves, and push the walkable edge out past
//! the door action point.

use bevy::{
    ecs::system::{SystemParam, SystemState},
    gltf::{Gltf, GltfExtras, GltfNode},
    prelude::*,
};
use moly_law::carve::ColliderPolygon;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod documents;
mod primitives;

/// Publishes the fixture collision scene for the particle host, after the
/// furniture loader has placed this frame's layout and before the weather host
/// reads it. A scene is rebuilt only when the layout generation or the site
/// changes, as the host's own install gate did.
pub fn publish(world: &mut World) {
    let revision = world
        .get_resource::<crate::render_inputs::FixtureLayoutRevision>()
        .map_or(0, |revision| revision.0);
    let Some((site_id, rows)) = world
        .get_resource::<crate::fixture::FixturePlacements>()
        .map(|placements| (placements.site_id(), placements.placed_rows().len()))
    else {
        world.remove_resource::<crate::render_inputs::FixtureCollisionScene>();
        return;
    };
    if rows == 0 {
        world.remove_resource::<crate::render_inputs::FixtureCollisionScene>();
        return;
    }
    if world
        .get_resource::<crate::render_inputs::FixtureCollisionScene>()
        .is_some_and(|scene| {
            scene.built.is_some()
                && scene.site_id == site_id
                && scene.revision == revision
                && scene.rows == rows
        })
    {
        return;
    }
    let (built, error) = match build_render_scene(world) {
        Ok(scene) => (Some(scene), None),
        Err(reason) => (None, Some(reason)),
    };
    world.insert_resource(crate::render_inputs::FixtureCollisionScene {
        site_id,
        revision,
        rows,
        built,
        error,
    });
}

const AGENT: i32 = -1372625422;

/// Builds the placed fixtures' collision scene for the particle physics host,
/// in source axes: each collider and each placed view's touch box at the
/// placement's pose. The particle host keeps the scene it gets here.
fn build_render_scene(world: &mut World)
    -> Result<std::sync::Arc<crate::particle_runtime::collision_scene::GroundScene>, String>
{
    let mut system = SystemState::<CollisionInputs>::new(world);
    let collision = system.get(world);
    let placements = world
        .get_resource::<crate::fixture::FixturePlacements>()
        .ok_or_else(|| "no placements".to_owned())?;
    let parts = fixture_parts(&collision, placements)?;
    Ok(crate::particle_runtime::collision_scene::fixture_scene(parts))
}

/// The placed fixtures' colliders in source axes: the colliders of their
/// collision scenes, and each placed view's touch box at the placement's pose.
/// The view receives the fixture's own (unturned) grid size for its touch box
/// and turns its own transform to the direction, so the touch box, a child
/// at the view's origin, turns with it.
fn fixture_parts(
    collision: &CollisionInputs,
    placements: &crate::fixture::FixturePlacements,
) -> Result<Vec<crate::particle_runtime::collision_scene::FixturePart>, String> {
    use crate::particle_runtime::collision_scene::{touch_box, FixturePart, FixtureShape, TOUCH_BOX_LAYERS};
    let reflect = |p: [f32; 3]| [-p[0], p[1], p[2]];
    let mut parts = Vec::new();
    // Each row's slot in the site view's fixture list (layout type and the
    // footprint's minimum y, in the placement mock's row order); a collider
    // finds its row by its fixture's package when no other row shares it.
    let rows = placements.editor_rows();
    let keys: Vec<Option<(u8, i32)>> =
        rows.iter().map(|row| row.footprint().ok().map(|(min, _)| (row.layout, i32::from(min.y)))).collect();
    let slots: Vec<Option<usize>> = if keys.iter().all(Option::is_some) {
        let keys: Vec<(u8, i32)> = keys.iter().flatten().copied().collect();
        crate::particle_runtime::collision_scene::fixture_slots(&keys).into_iter().map(Some).collect()
    } else {
        vec![None; rows.len()]
    };
    let slot_of = |package: &str| {
        let mut found = rows.iter().enumerate().filter(|(_, row)| row.package == package);
        match (found.next(), found.next()) {
            (Some((r, _)), None) => slots[r],
            _ => None,
        }
    };
    for (i, collider) in collision.physics_colliders(placements.total())?.into_iter().enumerate() {
        let shape = match collider.shape {
            PhysicsShape::Mesh { convex, cooking, positions, triangles } => FixtureShape::Mesh {
                convex, cooking,
                positions: positions.into_iter().map(reflect).collect(),
                // The canonical frame reverses the winding.
                triangles: triangles.into_iter().map(|t| [t[0], t[2], t[1]].map(|v| v as u32)).collect(),
            },
            PhysicsShape::Box { center, size } => FixtureShape::Box { center: reflect(center), half: size.map(|v| v * 0.5) },
            PhysicsShape::Other(kind) => FixtureShape::Unknown(kind),
        };
        let (scale, rotation, translation) = collider.world.to_scale_rotation_translation();
        let pose = (scale == Vec3::ONE).then(|| source_pose(rotation, translation));
        let slot = collider.package.as_deref().and_then(slot_of);
        parts.push(FixturePart { what: format!("collider {i}"), layers: 1 << collider.layer,
            world: source_matrix(&collider.world), pose, shape, slot, touch: false });
    }
    for (r, row) in rows.iter().enumerate() {
        let view = row.pose()?;
        let world = source_matrix(&GlobalTransform::from(view));
        let pose = (view.scale == Vec3::ONE).then(|| source_pose(view.rotation, view.translation));
        let (center, half) = touch_box([row.grid_size.x, row.grid_size.y, row.grid_size.z], row.layout);
        let shape = FixtureShape::Box { center, half };
        parts.push(FixturePart { what: format!("{} touch box", row.package), layers: TOUCH_BOX_LAYERS, world, pose,
            shape, slot: slots[r], touch: true });
    }
    Ok(parts)
}

/// A canonical-frame pose (source X reflected) in source axes: the rotation
/// (x, y, z, w) and the translation.
fn source_pose(rotation: Quat, translation: Vec3) -> ([f32; 4], [f32; 3]) {
    ([rotation.x, -rotation.y, -rotation.z, rotation.w], [-translation.x, translation.y, translation.z])
}

/// A canonical-frame matrix (source X reflected) in source axes.
fn source_matrix(world: &GlobalTransform) -> [f32; 16] {
    let m = world.to_matrix().to_cols_array();
    let sign = |i: usize| if i == 0 { -1.0 } else { 1.0 };
    std::array::from_fn(|n| sign(n % 4) * sign(n / 4) * m[n])
}

#[derive(Resource, Clone, Debug, Default)]
pub struct CollisionBakeStatus {
    pub ready: bool,
    pub reason: String,
    pub colliders: usize,
    pub polygons: usize,
}

#[derive(SystemParam)]
pub struct CollisionInputs<'w, 's> {
    roots: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static crate::fixture::FixtureSource>,
            Option<&'static crate::fixture_colors::FixtureColorChoice>,
        ),
        With<crate::fixture::FixtureRoot>,
    >,
    gltfs: Res<'w, Assets<Gltf>>,
    gltf_nodes: Res<'w, Assets<GltfNode>>,
    nodes: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static ChildOf>,
            Option<&'static Transform>,
            Option<&'static GltfExtras>,
            Option<&'static moly_assets::scene_state::SourceInactive>,
            Option<&'static moly_assets::scene_state::SourceNodeActivity>,
        ),
    >,
    ready: Option<Res<'w, crate::fixture::FixtureScenesReady>>,
}

pub struct Collected {
    pub polygons: Vec<ColliderPolygon>,
    pub colliders: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    schema_version: u32,
    coordinate_contract: String,
    units: String,
    geometry: Vec<Geometry>,
    #[serde(default)]
    gaps: Vec<serde_json::Value>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    geometry_id: String,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[usize; 3]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    schema_version: u32,
    coordinate_contract: String,
    active_self: Option<bool>,
    active_in_hierarchy: Option<bool>,
    authored_layer: Option<u32>,
    fixture_nav_layer: bool,
    colliders: Vec<Collider>,
    modifiers: Vec<Modifier>,
    has_nav_mesh_agent: bool,
    nav_mesh_obstacles: Vec<NavObstacle>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Collider {
    kind: String,
    enabled: Option<bool>,
    is_trigger: Option<bool>,
    #[serde(default)]
    convex: Option<bool>,
    #[serde(default)]
    cooking_options: Option<u32>,
    geometry_id: Option<String>,
    gap: Option<String>,
    center: Option<[f32; 3]>,
    size: Option<[f32; 3]>,
    radius: Option<f32>,
    height: Option<f32>,
    direction: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Modifier {
    enabled: Option<bool>,
    override_area: Option<bool>,
    area: Option<i32>,
    ignore_from_build: Option<bool>,
    affected_agents: Option<Vec<i32>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavObstacle {
    enabled: Option<bool>,
    shape: Option<u32>,
    center: Option<[f32; 3]>,
    extents: Option<[f32; 3]>,
    carve: Option<bool>,
    only_stationary: Option<bool>,
}

impl CollisionInputs<'_, '_> {
    /// The placed fixtures' collision documents and node records, with the
    /// hierarchy both readers walk.
    fn gather(&self, expected: usize) -> Result<Gathered, String> {
        if self.ready.is_none() {
            return Err("fixture collision scenes loading".into());
        }
        let roots: HashSet<_> = self.roots.iter().map(|(entity, ..)| entity).collect();
        if roots.len() != expected {
            return Err(format!(
                "fixture collision root count {}/{}",
                roots.len(),
                expected
            ));
        }
        let mut owner = HashMap::new();
        let mut parents = HashMap::new();
        let mut locals = HashMap::new();
        let mut source = HashMap::new();
        let mut activities = HashMap::new();
        let mut documents = HashMap::new();
        // One decoded document per unique source node for this bake, not per
        // placed instance. This cache owns no asset handles and ends with the
        // bake; source residency remains owned by the active FixtureSource.
        let mut document_cache = HashMap::new();
        for (entity, parent, local, extras, inactive, activity) in &self.nodes {
            if let Some(activity) = activity {
                activities.insert(entity, activity.active_self());
            }
            if let Some(parent) = parent {
                parents.insert(entity, parent.parent());
            }
            if let Some(local) = local {
                locals.insert(entity, *local);
            }
            let mut current = entity;
            let mut seen = HashSet::new();
            let root = loop {
                if roots.contains(&current) {
                    break Some(current);
                }
                if !seen.insert(current) {
                    return Err("fixture hierarchy cycle".into());
                }
                let Ok((_, Some(parent), _, _, _, _)) = self.nodes.get(current) else {
                    break None;
                };
                current = parent.parent();
            };
            let Some(root) = root else {
                continue;
            };
            owner.insert(entity, root);
            let Some(extras) = extras else {
                continue;
            };
            let value: documents::CollisionExtras = serde_json::from_str(&extras.value)
                .map_err(|error| format!("fixture collision extras: {error}"))?;
            if value.fixture_collision.is_some() && value.fixture_collision_ref.is_some() {
                return Err("fixture collision document and reference are ambiguous".into());
            }
            let document = if let Some(document) = value.fixture_collision {
                documents::validate(&document)?;
                Some(Arc::new(document))
            } else if let Some(reference) = value.fixture_collision_ref {
                let source = self
                    .roots
                    .get(root)
                    .ok()
                    .and_then(|(_, source, _)| source)
                    .ok_or("fixture collision reference has no source asset owner")?;
                Some(documents::resolve(
                    &reference,
                    source,
                    &self.gltfs,
                    &self.gltf_nodes,
                    &mut document_cache,
                )?)
            } else {
                None
            };
            if let Some(document) = document {
                if documents.insert(root, document).is_some() {
                    return Err("fixture has multiple collision scene roots".into());
                }
            }
            if let Some(node) = value.source_collision {
                validate_contract(node.schema_version, &node.coordinate_contract)?;
                source.insert(entity, (node, inactive.is_some()));
            }
        }
        if documents.len() != roots.len() {
            return Err("fixture collision contract missing; re-export snapshot".into());
        }
        Ok(Gathered { roots, owner, parents, locals, source, activities, documents })
    }

    pub fn collect(&self, expected: usize) -> Result<Collected, String> {
        let gathered = self.gather(expected)?;
        let Gathered { roots, owner, parents, source, documents, .. } = &gathered;
        let mut result = Collected {
            polygons: Vec::new(),
            colliders: 0,
        };
        for (entity, (node, _)) in source {
            if !gathered.active(*entity, node)? {
                continue;
            }
            if !(node.fixture_nav_layer || node.authored_layer == Some(9)) {
                continue;
            }
            let (composed, world_rotation) = gathered.world(*entity)?;
            let transform = &composed;
            let mut current = *entity;
            let mut ignored = false;
            loop {
                if let Some((ancestor, _)) = source.get(&current) {
                    for modifier in &ancestor.modifiers {
                        if modifier.enabled == Some(false) {
                            continue;
                        }
                        if modifier.enabled != Some(true) {
                            return Err("NavMeshModifier enabled state missing".into());
                        }
                        let agents = modifier
                            .affected_agents
                            .as_ref()
                            .ok_or("NavMeshModifier affected agents missing")?;
                        if !agents.contains(&-1) && !agents.contains(&AGENT) {
                            continue;
                        }
                        if modifier.ignore_from_build == Some(true) {
                            ignored = true;
                        }
                        if modifier.ignore_from_build.is_none() || modifier.override_area.is_none()
                        {
                            return Err("NavMeshModifier build controls missing".into());
                        }
                        if modifier.override_area == Some(true) && modifier.area != Some(0) {
                            return Err("non-default NavMeshModifier areas unsupported by single-surface host".into());
                        }
                    }
                }
                let Some(parent) = parents.get(&current) else {
                    break;
                };
                current = *parent;
                if roots.contains(&current) {
                    break;
                }
            }
            let document = &documents[&owner[entity]];
            // NavMeshSurface ignores physics sources with an agent or obstacle
            // on the same GameObject. Obstacle carving is a separate input.
            if !ignored && !node.has_nav_mesh_agent && node.nav_mesh_obstacles.is_empty() {
                for collider in &node.colliders {
                    if collider.enabled == Some(false) || collider.is_trigger == Some(true) {
                        continue;
                    }
                    if collider.enabled != Some(true) || collider.is_trigger != Some(false) {
                        return Err("collider enabled/trigger state missing".into());
                    }
                    if let Some(gap) = &collider.gap {
                        return Err(gap.clone());
                    }
                    result.polygons.extend(collider_polygons(
                        collider,
                        &document.geometry,
                        transform,
                        world_rotation,
                    )?);
                    result.colliders += 1;
                }
            }
            for obstacle in &node.nav_mesh_obstacles {
                if obstacle.enabled == Some(false) || obstacle.carve == Some(false) {
                    continue;
                }
                if obstacle.enabled != Some(true) || obstacle.carve != Some(true) {
                    return Err("NavMeshObstacle enabled/carve state missing".into());
                }
                if obstacle.only_stationary != Some(true) {
                    return Err("moving NavMeshObstacle needs dynamic carving support".into());
                }
                result
                    .polygons
                    .push(obstacle_polygon(obstacle, transform, world_rotation)?);
                result.colliders += 1;
            }
        }
        Ok(result)
    }

    /// The placed fixtures' colliders as the physics scene holds them: every
    /// enabled, non-trigger collider on an active node, in the canonical
    /// frame, on the layer the fixture view leaves it on (it moves every node
    /// of a fixture to the navigation build layer when it attaches its
    /// components; a node outside that recursion keeps its authored layer).
    /// A trigger is left out: the particle collision module drops a
    /// trigger's hits and a touching trigger does not end a lane.
    pub fn physics_colliders(&self, expected: usize) -> Result<Vec<PhysicsCollider>, String> {
        let gathered = self.gather(expected)?;
        // Each fixture root's package, which names its placement row.
        let packages: HashMap<Entity, &str> = self.roots.iter()
            .filter_map(|(entity, _, choice)| choice.map(|choice| (entity, choice.package.as_str())))
            .collect();
        let mut out = Vec::new();
        for (entity, (node, _)) in &gathered.source {
            if !gathered.active(*entity, node)? {
                continue;
            }
            let layer = if node.fixture_nav_layer { 9 } else {
                node.authored_layer.ok_or("fixture collider layer missing")?
            };
            let (world, _) = gathered.world(*entity)?;
            let document = &gathered.documents[&gathered.owner[entity]];
            for collider in &node.colliders {
                if collider.enabled == Some(false) || collider.is_trigger == Some(true) {
                    continue;
                }
                if collider.enabled != Some(true) || collider.is_trigger != Some(false) {
                    return Err("collider enabled/trigger state missing".into());
                }
                if let Some(gap) = &collider.gap {
                    return Err(gap.clone());
                }
                let shape = match collider.kind.as_str() {
                    "MeshCollider" => {
                        let id = collider.geometry_id.as_ref().ok_or("MeshCollider geometry missing")?;
                        let mesh = document.geometry.iter().find(|mesh| &mesh.geometry_id == id)
                            .ok_or("collider geometry reference unresolved")?;
                        PhysicsShape::Mesh {
                            convex: collider.convex.ok_or("MeshCollider convex flag missing")?,
                            cooking: collider.cooking_options,
                            positions: mesh.positions.clone(),
                            triangles: mesh.triangles.clone(),
                        }
                    }
                    "BoxCollider" => PhysicsShape::Box {
                        center: collider.center.ok_or("box center missing")?,
                        size: collider.size.ok_or("box size missing")?,
                    },
                    other => PhysicsShape::Other(other.to_owned()),
                };
                let package = gathered.owner.get(entity).and_then(|root| packages.get(root)).map(|p| (*p).to_owned());
                out.push(PhysicsCollider { shape, layer, world, package });
            }
        }
        Ok(out)
    }
}

/// One placed fixture collider for the physics scene.
pub struct PhysicsCollider {
    pub shape: PhysicsShape,
    pub layer: u32,
    /// The node's world transform in the canonical frame.
    pub world: GlobalTransform,
    /// The package of the fixture it belongs to.
    pub package: Option<String>,
}

pub enum PhysicsShape {
    /// The shared mesh in the canonical frame (source X reflected, winding
    /// reversed), with the convex flag and the cooking options when known.
    Mesh { convex: bool, cooking: Option<u32>, positions: Vec<[f32; 3]>, triangles: Vec<[usize; 3]> },
    /// Centre and size in the node's frame (positive source magnitudes; the
    /// centre's X reflected).
    Box { center: [f32; 3], size: [f32; 3] },
    Other(String),
}

/// What both readers walk: the fixture roots, each node's root, parent and
/// local transform, the node records and activity, and each root's document.
struct Gathered {
    roots: HashSet<Entity>,
    owner: HashMap<Entity, Entity>,
    parents: HashMap<Entity, Entity>,
    locals: HashMap<Entity, Transform>,
    source: HashMap<Entity, (Node, bool)>,
    activities: HashMap<Entity, bool>,
    documents: HashMap<Entity, Arc<Document>>,
}

impl Gathered {
    /// Whether the node is in the scene: not source-inactive, and no node up
    /// its chain inactive.
    fn active(&self, entity: Entity, node: &Node) -> Result<bool, String> {
        if self.source.get(&entity).is_some_and(|(_, inactive)| *inactive) {
            return Ok(false);
        }
        if node.active_self.is_none() || node.active_in_hierarchy.is_none() {
            return Err("fixture collider active state missing".into());
        }
        let mut current = entity;
        loop {
            let state = self
                .activities
                .get(&current)
                .copied()
                .or_else(|| self.source.get(&current).and_then(|(node, _)| node.active_self));
            if state == Some(false) {
                return Ok(false);
            }
            let Some(parent) = self.parents.get(&current) else {
                return Ok(true);
            };
            current = *parent;
        }
    }

    /// The node's world transform and rotation. SceneReady is raised before
    /// transform propagation: compose the current local chain to avoid
    /// freezing the first bake at stale identity GlobalTransforms or
    /// one-frame-old editor positions.
    fn world(&self, entity: Entity) -> Result<(GlobalTransform, Quat), String> {
        let mut chain = Vec::new();
        let mut current = entity;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current) {
                return Err("fixture transform cycle".into());
            }
            chain.push(*self.locals.get(&current).ok_or("fixture collider local transform missing")?);
            let Some(parent) = self.parents.get(&current) else {
                break;
            };
            current = *parent;
        }
        let mut composed = GlobalTransform::IDENTITY;
        let mut world_rotation = Quat::IDENTITY;
        for local in chain.into_iter().rev() {
            world_rotation *= local.rotation;
            composed = composed.mul_transform(local);
        }
        if !composed.to_matrix().is_finite() {
            return Err("fixture collider world transform nonfinite".into());
        }
        Ok((composed, world_rotation))
    }
}

fn validate_contract(version: u32, contract: &str) -> Result<(), String> {
    if version != 1 || contract != moly_assets::coordinates::CONTRACT {
        return Err("fixture collision coordinate/schema contract unsupported".into());
    }
    Ok(())
}

fn obstacle_polygon(
    obstacle: &NavObstacle,
    transform: &GlobalTransform,
    world_rotation: Quat,
) -> Result<ColliderPolygon, String> {
    // Unity 2022.3.62f2 x86_64 NavMeshObstacle::GetWorldExtents
    // uses absolute lossy scale, sharing max(X,Z) between capsule radii.
    // GetWorldCenterAndAxes transforms the center and separately
    // converts Transform::GetRotation to orthonormal axes (not affine skew).
    let center = transform.transform_point(Vec3::from(
        obstacle.center.ok_or("obstacle center missing")?,
    ));
    let extents = Vec3::from(obstacle.extents.ok_or("obstacle extents missing")?);
    if !extents.is_finite() || extents.min_element() < 0.0 {
        return Err("invalid obstacle extents".into());
    }
    let scale = primitives::scale_magnitudes(transform);
    let mut polygon = match obstacle.shape {
        Some(1) => {
            let (points, triangles) = primitives::box_mesh(Vec3::ZERO, extents * scale * 2.0)?;
            let pose = GlobalTransform::from(
                Transform::from_translation(center).with_rotation(world_rotation),
            );
            project_mesh(&points, &triangles, &pose, true)?
        }
        Some(0) => {
            let radius = extents.x * scale.x.max(scale.z);
            // Both CalcCapsuleWorldExtents and the capsule branch
            // of CarveNavMeshTile use max(halfHeight-r,0).
            // A serialized "thin capsule" is therefore a sphere, NOT a disk.
            let points = primitives::nav_capsule_points(
                center,
                world_rotation,
                radius,
                extents.y * scale.y,
            )?;
            let (points, triangles) = convex_mesh(&points)?;
            project_mesh(&points, &triangles, &GlobalTransform::IDENTITY, true)?
        }
        _ => return Err("unknown NavMeshObstacle shape".into()),
    };
    polygon.carve = true;
    Ok(polygon)
}

fn collider_polygons(
    collider: &Collider,
    geometry: &[Geometry],
    transform: &GlobalTransform,
    world_rotation: Quat,
) -> Result<Vec<ColliderPolygon>, String> {
    let polygons = match collider.kind.as_str() {
        "MeshCollider" => {
            let id = collider
                .geometry_id
                .as_ref()
                .ok_or("MeshCollider geometry missing")?;
            let mesh = geometry
                .iter()
                .find(|mesh| &mesh.geometry_id == id)
                .ok_or("collider geometry reference unresolved")?;
            if mesh.positions.is_empty() || mesh.triangles.is_empty() {
                return Err("empty collider geometry".into());
            }
            // The navigation source is the shared mesh itself (see the module
            // comment): the convex flag is a physics cooking option only, so
            // it is neither required nor consulted here. An open or concave
            // source mesh stays open, as the builder rasterizes it.
            vec![project_mesh(
                &mesh.positions,
                &mesh.triangles,
                transform,
                false,
            )?]
        }
        "BoxCollider" => {
            let center =
                transform.transform_point(Vec3::from(collider.center.ok_or("box center missing")?));
            let size = Vec3::from(collider.size.ok_or("box size missing")?)
                * primitives::physics_scale(transform)?;
            let (points, triangles) = primitives::box_mesh(Vec3::ZERO, size)?;
            let pose = GlobalTransform::from(
                Transform::from_translation(center).with_rotation(world_rotation),
            );
            vec![project_mesh(&points, &triangles, &pose, true)?]
        }
        "SphereCollider" => {
            let center = transform
                .transform_point(Vec3::from(collider.center.ok_or("sphere center missing")?));
            let radius = collider.radius.ok_or("sphere radius missing")?
                * primitives::physics_scale(transform)?.max_element();
            let (points, triangles) = primitives::rounded_solid(center, Vec3::Y, radius, 0.0)?;
            vec![project_mesh(
                &points,
                &triangles,
                &GlobalTransform::IDENTITY,
                true,
            )?]
        }
        "CapsuleCollider" => {
            let direction = collider.direction.ok_or("capsule direction missing")?;
            if direction > 2 {
                return Err("invalid capsule direction".into());
            }
            let height = collider.height.ok_or("capsule height missing")?;
            if !height.is_finite() || height < 0.0 {
                return Err("invalid capsule height".into());
            }
            let scale = primitives::physics_scale(transform)?;
            let radius = collider.radius.ok_or("capsule radius missing")?
                * scale[(direction + 1) % 3].max(scale[(direction + 2) % 3]);
            let center = transform
                .transform_point(Vec3::from(collider.center.ok_or("capsule center missing")?));
            let mut axis = Vec3::ZERO;
            axis[direction] = 1.0;
            let axis = (world_rotation * axis).normalize();
            let segment_half = (height * scale[direction] * 0.5 - radius).max(0.0);
            let (points, triangles) =
                primitives::rounded_solid(center, axis, radius, segment_half)?;
            vec![project_mesh(
                &points,
                &triangles,
                &GlobalTransform::IDENTITY,
                true,
            )?]
        }
        other => return Err(format!("unsupported collider shape {other}")),
    };
    Ok(polygons)
}

fn convex_mesh(positions: &[[f32; 3]]) -> Result<(Vec<[f32; 3]>, Vec<[usize; 3]>), String> {
    let mut points: Vec<_> = positions
        .iter()
        .copied()
        .map(parry3d::math::Vector::from_array)
        .collect();
    if points.iter().any(|point| !point.is_finite()) {
        return Err("nonfinite convex collision geometry".into());
    }
    points.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    points.dedup();
    let (points, triangles) = parry3d::transformation::try_convex_hull(&points)
        .map_err(|error| format!("convex collision hull failed: {error:?}"))?;
    if points.is_empty() || triangles.is_empty() {
        return Err("empty convex collision hull".into());
    }
    Ok((
        points.into_iter().map(|point| point.to_array()).collect(),
        triangles
            .into_iter()
            .map(|tri| tri.map(|i| i as usize))
            .collect(),
    ))
}

fn project_mesh(
    positions: &[[f32; 3]],
    indices: &[[usize; 3]],
    transform: &GlobalTransform,
    solid: bool,
) -> Result<ColliderPolygon, String> {
    let points: Vec<_> = positions
        .iter()
        .copied()
        .map(|p| transform.transform_point(Vec3::from(p)))
        .collect();
    let mut projected = project(points.clone(), &GlobalTransform::IDENTITY)?;
    projected.triangles = indices
        .iter()
        .map(|tri| {
            let point = |i: usize| {
                points
                    .get(i)
                    .copied()
                    .map(|point| point.to_array())
                    .ok_or_else(|| "collider index out of range".to_owned())
            };
            Ok([point(tri[0])?, point(tri[1])?, point(tri[2])?])
        })
        .collect::<Result<Vec<_>, String>>()?;
    projected.solid = solid;
    Ok(projected)
}

fn box_points(center: Vec3, size: Vec3) -> Result<Vec<Vec3>, String> {
    if !center.is_finite() || !size.is_finite() || size.min_element() <= 0.0 {
        return Err("invalid box geometry".into());
    }
    let mut points = Vec::with_capacity(8);
    for x in [-0.5, 0.5] {
        for y in [-0.5, 0.5] {
            for z in [-0.5, 0.5] {
                points.push(center + size * Vec3::new(x, y, z));
            }
        }
    }
    Ok(points)
}

fn project(points: Vec<Vec3>, transform: &GlobalTransform) -> Result<ColliderPolygon, String> {
    let points: Vec<_> = points
        .into_iter()
        .map(|p| transform.transform_point(p))
        .collect();
    if points.iter().any(|p| !p.is_finite()) {
        return Err("nonfinite collision geometry".into());
    }
    let min_y = points.iter().fold(f32::INFINITY, |y, p| y.min(p.y));
    let max_y = points.iter().fold(f32::NEG_INFINITY, |y, p| y.max(p.y));
    let vertices = hull(points.iter().map(|p| [p.x, p.z]).collect());
    Ok(ColliderPolygon {
        vertices,
        min_y,
        max_y,
        triangles: Vec::new(),
        solid: true,
        carve: false,
    })
}

fn hull(mut points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut result = Vec::new();
    for point in &points {
        while result.len() >= 2
            && cross(result[result.len() - 2], result[result.len() - 1], *point) <= 0.0
        {
            result.pop();
        }
        result.push(*point);
    }
    let lower = result.len();
    for point in points[..points.len() - 1].iter().rev() {
        while result.len() > lower
            && cross(result[result.len() - 2], result[result.len() - 1], *point) <= 0.0
        {
            result.pop();
        }
        result.push(*point);
    }
    result.pop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_record(colliders: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"schemaVersion":1,"coordinateContract":moly_assets::coordinates::CONTRACT,
            "activeSelf":true,"activeInHierarchy":true,"authoredLayer":9,"fixtureNavLayer":true,
            "colliders":colliders,"modifiers":[],"hasNavMeshAgent":false,"navMeshObstacles":[]})
    }

    #[test]
    fn collector_uses_live_local_chain_before_global_propagation() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        world.init_resource::<Assets<Gltf>>();
        world.init_resource::<Assets<GltfNode>>();
        world.insert_resource(crate::fixture::FixtureScenesReady);
        let root = world
            .spawn((
                crate::fixture::FixtureRoot,
                Transform::from_xyz(7.0, 0.0, 3.0),
            ))
            .id();
        let document = serde_json::json!({"schemaVersion":1,"coordinateContract":moly_assets::coordinates::CONTRACT,
            "units":"source-unity-unit","geometry":[],"gaps":[]});
        let scene = world.spawn((ChildOf(root), Transform::from_xyz(1.0,0.0,0.0),
            GltfExtras { value: serde_json::json!({"fixtureCollision":document,"sourceCollision":node_record(serde_json::json!([]))}).to_string() })).id();
        let collider = serde_json::json!({"kind":"BoxCollider","enabled":true,"isTrigger":false,
            "center":[0.0,0.5,0.0],"size":[1.0,1.0,1.0]});
        world.spawn((ChildOf(scene), Transform::from_xyz(2.0,0.0,0.0), GlobalTransform::IDENTITY,
            GltfExtras { value: serde_json::json!({"sourceCollision":node_record(serde_json::json!([collider]))}).to_string() }));
        let mut system = SystemState::<CollisionInputs>::new(&mut world);
        let result = system.get(&world).collect(1).unwrap();
        assert_eq!(result.colliders, 1);
        assert_eq!(result.polygons.len(), 1);
        assert!(
            result.polygons[0]
                .vertices
                .iter()
                .all(|p| p[0] >= 9.5 && p[0] <= 10.5 && p[1] >= 2.5 && p[1] <= 3.5)
        );
    }

    #[test]
    fn collector_rejects_missing_contract_instead_of_using_occupancy() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        world.init_resource::<Assets<Gltf>>();
        world.init_resource::<Assets<GltfNode>>();
        world.insert_resource(crate::fixture::FixtureScenesReady);
        world.spawn((crate::fixture::FixtureRoot, Transform::IDENTITY));
        let mut system = SystemState::<CollisionInputs>::new(&mut world);
        let error = system.get(&world).collect(1).err().unwrap();
        assert!(error.contains("re-export"));
    }

    #[test]
    fn actual_geometry_preserves_empty_aabb_corners_and_live_rotation() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(0.0, 0.5, 2.0),
        ];
        let pose = GlobalTransform::from(
            Transform::from_xyz(4.0, 0.0, 3.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        );
        let polygon = project(points, &pose).unwrap();
        assert_eq!(polygon.vertices.len(), 3);
        assert!((polygon.min_y - 0.0).abs() < 1e-6);
        assert!((polygon.max_y - 1.0).abs() < 1e-6);
        assert!(
            polygon
                .vertices
                .iter()
                .any(|p| (p[0] - 6.0).abs() < 1e-5 && (p[1] - 3.0).abs() < 1e-5)
        );
    }

    #[test]
    fn mesh_collider_source_is_its_own_triangles_whatever_the_convex_flag() {
        // The engine's collider-to-source conversion never reads `convex`:
        // both flags must give the same open surfaces, not a closed hull.
        let mesh = Geometry {
            geometry_id: "mesh".into(),
            positions: vec![
                [-1.0, 0.0, -1.0],
                [-1.0, 1.0, -1.0],
                [-1.0, 1.0, 1.0],
                [-1.0, 0.0, 1.0],
                [1.0, 0.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
            // Two separate vertical walls: the source mesh itself is open.
            triangles: vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        };
        let pose = GlobalTransform::from(
            Transform::from_xyz(2.0, 0.5, -1.0)
                .with_rotation(Quat::from_rotation_y(0.7))
                .with_scale(Vec3::new(1.5, 2.0, 0.5)),
        );
        let mut outputs = Vec::new();
        for convex in [true, false] {
            let collider: Collider = serde_json::from_value(serde_json::json!({
                "kind": "MeshCollider", "enabled": true, "isTrigger": false,
                "convex": convex, "geometryId": "mesh"
            }))
            .unwrap();
            let polygons =
                collider_polygons(&collider, &[mesh.clone()], &pose, Quat::IDENTITY).unwrap();
            assert_eq!(polygons.len(), 1);
            assert!(!polygons[0].solid, "convex={convex} stays open surfaces");
            assert_eq!(polygons[0].triangles.len(), mesh.triangles.len());
            for (tri, indices) in polygons[0].triangles.iter().zip(&mesh.triangles) {
                for (point, index) in tri.iter().zip(indices) {
                    let expected = pose.transform_point(Vec3::from(mesh.positions[*index]));
                    assert!(Vec3::from(*point).distance(expected) < 1e-5);
                }
            }
            outputs.push(polygons[0].triangles.clone());
        }
        assert_eq!(outputs[0], outputs[1]);
    }

    #[test]
    fn convex_hull_supports_planar_rugs_and_rejects_nonfinite_geometry() {
        let points = [
            [-1.0, 0.01, -1.0],
            [1.0, 0.01, -1.0],
            [1.0, 0.01, 1.0],
            [-1.0, 0.01, 1.0],
        ];
        let (points, triangles) = convex_mesh(&points).unwrap();
        assert_eq!(points.len(), 4);
        assert_eq!(triangles.len(), 4, "planar hull retains both faces");
        assert!(points.iter().all(|point| point[1] == 0.01));
        assert!(convex_mesh(&[[f32::NAN, 0.0, 0.0], [0.0; 3], [1.0; 3]]).is_err());
    }

    /// Reads a fixture GLB's JSON chunk.
    fn glb_json(path: &std::path::Path) -> serde_json::Value {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"glTF", "{}", path.display());
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        serde_json::from_slice(&bytes[20..20 + json_length]).unwrap()
    }

    /// Every collision document a GLB carries (root extras or a node's extras,
    /// including the isolated metadata node).
    fn glb_documents(gltf: &serde_json::Value) -> Vec<Document> {
        let mut values = vec![gltf["extras"]["fixtureCollision"].clone()];
        for node in gltf["nodes"].as_array().into_iter().flatten() {
            values.push(node["extras"]["fixtureCollision"].clone());
        }
        values
            .into_iter()
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value).unwrap())
            .collect()
    }

    /// Source rows: the engine emits a MeshCollider as a Mesh source of its own
    /// shared mesh, convex flag unread. Every MeshCollider in the corpus must
    /// therefore come out as exactly its geometry's triangles, open.
    #[test]
    #[ignore = "requires source GLBs via MOLY_COLLISION_GLB_DIR"]
    fn corpus_mesh_colliders_bake_their_own_triangles() {
        let directory = std::path::PathBuf::from(
            std::env::var("MOLY_COLLISION_GLB_DIR").expect("source fixture-models directory"),
        );
        let mut files: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "glb"))
            .collect();
        files.sort();
        let (mut rows, mut convex_rows, mut mismatches) = (0usize, 0usize, 0usize);
        for path in &files {
            let gltf = glb_json(path);
            let documents = glb_documents(&gltf);
            for node in gltf["nodes"].as_array().into_iter().flatten() {
                let Some(colliders) = node["extras"]["sourceCollision"]["colliders"].as_array()
                else {
                    continue;
                };
                for raw in colliders {
                    if raw["kind"] != "MeshCollider" || !raw["gap"].is_null() {
                        continue;
                    }
                    let collider: Collider = serde_json::from_value(raw.clone()).unwrap();
                    let id = collider.geometry_id.clone().unwrap();
                    let Some(mesh) = documents
                        .iter()
                        .flat_map(|document| document.geometry.iter())
                        .find(|mesh| mesh.geometry_id == id)
                    else {
                        continue;
                    };
                    rows += 1;
                    convex_rows += usize::from(raw["convex"] == true);
                    let polygons = collider_polygons(
                        &collider,
                        std::slice::from_ref(mesh),
                        &GlobalTransform::IDENTITY,
                        Quat::IDENTITY,
                    )
                    .unwrap();
                    let expected: Vec<[[f32; 3]; 3]> = mesh
                        .triangles
                        .iter()
                        .map(|tri| tri.map(|index| mesh.positions[index]))
                        .collect();
                    if polygons.len() != 1 || polygons[0].solid || polygons[0].triangles != expected
                    {
                        mismatches += 1;
                        eprintln!("mismatch {} {id}", path.display());
                    }
                }
            }
        }
        eprintln!(
            "mesh collider rows {rows} (convex {convex_rows}) over {} glbs: {mismatches} mismatches",
            files.len()
        );
        assert!(
            rows > 0 && convex_rows > 0,
            "corpus must exercise convex colliders"
        );
        assert_eq!(mismatches, 0);
    }

    /// The house door: CanNavmeshMoveTargetPosition(loc_outside, loc_inside,
    /// 0.03) over a flat ground carrying the house's source colliders and its
    /// carving obstacle, at the offline layout's placement (world (2.5, 0,
    /// -2.75), yaw 180). The hull arm is the former bake, kept to show that
    /// the measured dimension is not trivially satisfied.
    #[test]
    #[ignore = "requires source GLBs via MOLY_COLLISION_GLB_DIR"]
    fn house_inside_door_point_is_reachable_on_the_source_bake() {
        let directory = std::path::PathBuf::from(
            std::env::var("MOLY_COLLISION_GLB_DIR").expect("source fixture-models directory"),
        );
        let gltf = glb_json(&directory.join("mysekai__fixture__mdl_mis0001_house_house1.glb"));
        let geometry: Vec<Geometry> = glb_documents(&gltf)
            .iter()
            .flat_map(|document| document.geometry.iter().cloned())
            .collect();
        let rotation = Quat::from_rotation_y(std::f32::consts::PI);
        let placement =
            GlobalTransform::from(Transform::from_xyz(2.5, 0.0, -2.75).with_rotation(rotation));
        let mut source = Vec::new();
        let mut hulls = Vec::new();
        let mut locator = HashMap::new();
        let nodes = gltf["nodes"].as_array().unwrap();
        for (index, node) in nodes.iter().enumerate() {
            let (local, local_rotation) = test_node_world_transform(nodes, index);
            let transform = placement.mul_transform(local.compute_transform());
            let world_rotation = rotation * local_rotation;
            let name = node["name"].as_str().unwrap_or_default();
            if name == "loc_inside" || name == "loc_outside" {
                locator.insert(name.to_owned(), transform.translation());
            }
            let record = &node["extras"]["sourceCollision"];
            if record.is_null() {
                continue;
            }
            let record: Node = serde_json::from_value(record.clone()).unwrap();
            for collider in &record.colliders {
                if collider.kind != "MeshCollider" {
                    continue;
                }
                source.extend(
                    collider_polygons(collider, &geometry, &transform, world_rotation).unwrap(),
                );
                let mesh = geometry
                    .iter()
                    .find(|mesh| Some(&mesh.geometry_id) == collider.geometry_id.as_ref())
                    .unwrap();
                let (points, hull) = convex_mesh(&mesh.positions).unwrap();
                hulls.push(project_mesh(&points, &hull, &transform, true).unwrap());
            }
            for obstacle in &record.nav_mesh_obstacles {
                if obstacle.carve == Some(true) {
                    let polygon = obstacle_polygon(obstacle, &transform, world_rotation).unwrap();
                    source.push(polygon.clone());
                    hulls.push(polygon);
                }
            }
        }
        assert!(!source.is_empty() && !hulls.is_empty());
        let inside = locator["loc_inside"];
        let outside = locator["loc_outside"];
        let ground: Vec<[[f32; 3]; 3]> = vec![
            [[-6.0, 0.0, -10.0], [10.0, 0.0, -10.0], [10.0, 0.0, 6.0]],
            [[-6.0, 0.0, -10.0], [10.0, 0.0, 6.0], [-6.0, 0.0, 6.0]],
        ];
        let reach = |polygons: &[ColliderPolygon], voxel: f32| {
            let field = moly_law::carve::WalkField::bake_colliders(&ground, polygons, voxel);
            let path = field
                .calculate_path(
                    [outside.x, outside.z],
                    [inside.x, inside.z],
                    moly_law::carve::STATIC_QUERY_HALF_EXTENT,
                )
                .expect("static path query maps both ends");
            let last = *path.corners.last().unwrap();
            let distance = ((last[0] - inside.x).powi(2) + (last[1] - inside.z).powi(2)).sqrt();
            let admitted = field.can_navmesh_move_target_position(
                [outside.x, outside.z],
                [inside.x, inside.z],
                0.03,
            );
            (distance, admitted)
        };
        for voxel in [0.01, 0.05] {
            let (after, admitted) = reach(&source, voxel);
            let (before, hull_admitted) = reach(&hulls, voxel);
            eprintln!(
                "voxel {voxel}: inside door ({:.4}, {:.4}): path end distance hull {before:.4} -> source mesh {after:.4}",
                inside.x, inside.z
            );
            assert!(
                after < 0.03 && admitted,
                "source bake reaches the door point"
            );
            assert!(
                before >= 0.03 && !hull_admitted,
                "hull arm reproduces the former gap"
            );
        }
    }

    #[test]
    fn invalid_primitive_inputs_fail() {
        assert!(primitives::nav_capsule_points(Vec3::ZERO, Quat::IDENTITY, 0.5, -1.0).is_err());
        assert!(box_points(Vec3::ZERO, Vec3::new(-1.0, 1.0, 1.0)).is_err());
        assert!(validate_contract(1, "legacy-unity").is_err());
    }

    #[test]
    fn physics_round_primitives_use_unity_scale_and_capsule_height_rules() {
        let mut collider: Collider = serde_json::from_value(serde_json::json!({
            "kind":"SphereCollider", "center":[0.1,0.2,0.3], "radius":0.2
        }))
        .unwrap();
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let transform = GlobalTransform::from(
            Transform::from_xyz(1.0, 2.0, 3.0)
                .with_rotation(rotation)
                .with_scale(Vec3::new(-2.0, 0.5, 3.0)),
        );
        let center = transform.transform_point(Vec3::new(0.1, 0.2, 0.3));
        let sphere = collider_polygons(&collider, &[], &transform, rotation)
            .unwrap()
            .remove(0);
        let radius = 0.6;
        assert!(!sphere.carve && sphere.solid);
        assert!(
            sphere.triangles.iter().flatten().all(|point| {
                (Vec3::from(*point).distance(center) - radius * primitives::rounded_inflation())
                    .abs()
                    < 2e-5
            }),
            "sphere uses maximum absolute scale, not an ellipsoid"
        );
        collider.kind = "CapsuleCollider".into();
        collider.direction = Some(1);
        collider.height = Some(1.0);
        let capsule = collider_polygons(&collider, &[], &transform, rotation)
            .unwrap()
            .remove(0);
        assert!(
            capsule.triangles.iter().flatten().all(|point| {
                (Vec3::from(*point).distance(center) - radius * primitives::rounded_inflation())
                    .abs()
                    < 2e-5
            }),
            "height smaller than scaled diameter becomes a sphere"
        );
        collider.height = Some(4.0);
        let capsule = collider_polygons(&collider, &[], &transform, rotation)
            .unwrap()
            .remove(0);
        let axis = rotation * Vec3::Y;
        assert!(capsule.triangles.iter().flatten().all(|point| {
            let delta = Vec3::from(*point) - center;
            let closest = axis * delta.dot(axis).clamp(-0.4, 0.4);
            delta.distance(closest) <= radius * primitives::rounded_inflation() + 2e-5
        }));
    }

    #[test]
    fn explicit_short_carve_uses_native_sphere_minimum_height() {
        let obstacle = NavObstacle {
            enabled: Some(true),
            shape: Some(0),
            center: Some([0.0, 0.07, 0.0]),
            extents: Some([0.08, 0.00001, 0.08]),
            carve: Some(true),
            only_stationary: Some(true),
        };
        let polygon =
            obstacle_polygon(&obstacle, &GlobalTransform::IDENTITY, Quat::IDENTITY).unwrap();
        assert!(polygon.carve && polygon.solid && !polygon.triangles.is_empty());
        let carved_radius = 0.08 * 1.082_392_2_f32;
        assert!((polygon.min_y - (0.07 - carved_radius)).abs() < 1e-6);
        assert!((polygon.max_y - (0.07 + carved_radius)).abs() < 1e-6);
    }

    #[test]
    fn sideways_native_capsule_keeps_center_blocked_but_releases_cylinder_corner() {
        // JP con0005 pond authors this radius/height and X=90deg obstacle.
        let obstacle = NavObstacle {
            enabled: Some(true),
            shape: Some(0),
            center: Some([0.0; 3]),
            extents: Some([0.37, 0.625, 0.37]),
            carve: Some(true),
            only_stationary: Some(true),
        };
        let rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let transform = GlobalTransform::from(Transform::from_rotation(rotation));
        let polygon = obstacle_polygon(&obstacle, &transform, rotation).unwrap();
        let surface = [
            [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
            [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
        ];
        let field = moly_law::carve::WalkField::bake_colliders(&surface, &[polygon], 0.01);
        assert!(!field.walkable_at([0.0, 0.0]));
        assert!(
            field.walkable_at([0.50, 0.775]),
            "source rounded end is not the old cylinder corner"
        );
        let radial = 0.37 / (std::f32::consts::PI / 32.0).cos();
        let mut old = project(
            vec![
                Vec3::new(-radial, -radial, -0.625),
                Vec3::new(radial, radial, -0.625),
                Vec3::new(radial, radial, 0.625),
                Vec3::new(-radial, -radial, 0.625),
            ],
            &GlobalTransform::IDENTITY,
        )
        .unwrap();
        old.carve = true;
        let field = moly_law::carve::WalkField::bake_colliders(&surface, &[old], 0.01);
        assert!(
            !field.walkable_at([0.50, 0.775]),
            "regression point distinguishes the old cylinder"
        );
    }

    #[test]
    #[ignore = "requires source GLBs via MOLY_COLLISION_SOURCE_ROOT"]
    fn canonical_cn_short_and_jp_sideways_obstacles_use_native_envelopes() {
        let directory = std::path::PathBuf::from(
            std::env::var("MOLY_COLLISION_SOURCE_ROOT")
                .expect("source root with cn/jp directories"),
        );
        for (region, name) in [
            ("cn", "mysekai__fixture__mdl_env0003_fixture_pond1.glb"),
            ("cn", "mysekai__fixture__mdl_con0003_fixture_taiyaki1.glb"),
            ("jp", "mysekai__fixture__mdl_con0005_fixture_pond1.glb"),
        ] {
            let bytes =
                std::fs::read(directory.join(region).join("fixture-models").join(name)).unwrap();
            let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let gltf: serde_json::Value =
                serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
            let nodes = gltf["nodes"].as_array().unwrap();
            let mut tested = 0;
            for (node_index, node) in nodes.iter().enumerate() {
                let Some(source) = node
                    .get("extras")
                    .and_then(|extras| extras.get("sourceCollision"))
                else {
                    continue;
                };
                let source: Node = serde_json::from_value(source.clone()).unwrap();
                for obstacle in source.nav_mesh_obstacles.iter().filter(|o| {
                    o.enabled == Some(true) && o.carve == Some(true) && o.shape == Some(0)
                }) {
                    let center = obstacle.center.unwrap();
                    let extents = obstacle.extents.unwrap();
                    let (transform, rotation) = test_node_world_transform(nodes, node_index);
                    let polygon = obstacle_polygon(obstacle, &transform, rotation).unwrap();
                    assert!(polygon.carve);
                    assert!(!polygon.triangles.is_empty());
                    let scale = primitives::scale_magnitudes(&transform);
                    let radius = extents[0] * scale.x.max(scale.z);
                    let half_height = extents[1] * scale.y;
                    if half_height < radius {
                        let native_radius = radius * 1.082_392_2_f32;
                        let world_center = transform.transform_point(Vec3::from(center));
                        assert!(
                            polygon
                                .triangles
                                .iter()
                                .flatten()
                                .all(|p| Vec3::from(*p).distance(world_center)
                                    <= native_radius + 1e-5)
                        );
                    }
                    eprintln!(
                        "{region}/{name}: serialized radius {} height {}, native envelope {} triangles",
                        extents[0],
                        extents[1] * 2.0,
                        polygon.triangles.len(),
                    );
                    tested += 1;
                }
            }
            assert!(tested > 0, "real source includes capsule-shaped obstacles");
        }
    }

    fn test_node_world_transform(
        nodes: &[serde_json::Value],
        node_index: usize,
    ) -> (GlobalTransform, Quat) {
        let mut chain = Vec::new();
        let mut current = node_index;
        loop {
            let node = &nodes[current];
            assert!(
                node.get("matrix").is_none(),
                "fixture source uses explicit TRS"
            );
            let translation = node
                .get("translation")
                .map(|v| serde_json::from_value::<[f32; 3]>(v.clone()).unwrap())
                .unwrap_or([0.0; 3]);
            let rotation = node
                .get("rotation")
                .map(|v| serde_json::from_value::<[f32; 4]>(v.clone()).unwrap())
                .unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let scale = node
                .get("scale")
                .map(|v| serde_json::from_value::<[f32; 3]>(v.clone()).unwrap())
                .unwrap_or([1.0; 3]);
            chain.push(Transform {
                translation: Vec3::from(translation),
                rotation: Quat::from_array(rotation),
                scale: Vec3::from(scale),
            });
            let Some(parent) = nodes.iter().position(|node| {
                node["children"].as_array().is_some_and(|children| {
                    children
                        .iter()
                        .any(|child| child.as_u64() == Some(current as u64))
                })
            }) else {
                break;
            };
            current = parent;
            assert!(chain.len() <= nodes.len());
        }
        let mut world = GlobalTransform::IDENTITY;
        let mut rotation = Quat::IDENTITY;
        for local in chain.into_iter().rev() {
            world = world.mul_transform(local);
            rotation *= local.rotation;
        }
        (world, rotation)
    }

    const CENSUS_VOXEL: f32 = 0.01;
    const CENSUS_HALF: f32 = 3.0;
    /// One more cell than the grid's `6.0 / 0.01` cell count rounds to; points
    /// outside the baked grid are not walkable, so a superset only costs empty
    /// lookups.
    const CENSUS_CELLS: usize = 601;

    /// A fixture glb's collision sources in fixture-local space.
    struct CensusSource {
        physics: Vec<ColliderPolygon>,
        carving: Vec<ColliderPolygon>,
        kinds: [usize; 4],
        other: usize,
        errors: usize,
    }

    /// Reads logical asset paths the way the game's asset root does: either a
    /// plain export tree, or an installed resource directory whose catalog maps
    /// each logical path to a package manifest and then to a blob.
    struct CensusAssets {
        root: std::path::PathBuf,
        tree: bool,
        manifests: std::collections::HashMap<String, String>,
    }

    impl CensusAssets {
        fn open(root: &std::path::Path) -> Self {
            let tree = root.join("fixture-models/index.json").is_file();
            let mut manifests = std::collections::HashMap::new();
            if !tree {
                let record_path = root.join("moly-resources.json");
                let record: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&record_path)
                        .unwrap_or_else(|error| panic!("{}: {error}", record_path.display())),
                )
                .unwrap_or_else(|error| panic!("{}: {error}", record_path.display()));
                let catalog = std::env::var("MOLY_ASSET_CATALOG").ok().unwrap_or_else(|| {
                    record["snapshots"]
                        .as_array()
                        .and_then(|snapshots| {
                            snapshots
                                .iter()
                                .max_by_key(|snapshot| snapshot["installed_at"].as_u64().unwrap_or(0))
                        })
                        .and_then(|snapshot| snapshot["catalog"].as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            panic!(
                                "{} names no installed catalog; set MOLY_ASSET_CATALOG",
                                root.display()
                            )
                        })
                });
                let path = root.join("catalogs").join(format!("{catalog}.json"));
                let document: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&path)
                        .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
                )
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                assert_eq!(
                    document["schema"], "moly-asset-packs/2",
                    "{} is not a version-2 asset catalog",
                    path.display()
                );
                for package in document["packages"].as_array().into_iter().flatten() {
                    let Some(manifest) = package["manifest"].as_str() else {
                        continue;
                    };
                    for logical in package["paths"].as_array().into_iter().flatten() {
                        if let Some(logical) = logical.as_str() {
                            manifests.insert(logical.to_owned(), manifest.to_owned());
                        }
                    }
                }
            }
            Self {
                root: root.to_path_buf(),
                tree,
                manifests,
            }
        }

        fn read(&self, logical: &str) -> Option<Vec<u8>> {
            if self.tree {
                return std::fs::read(self.root.join(logical)).ok();
            }
            let manifest = self.manifests.get(logical)?;
            let path = self.root.join(manifest.as_str());
            let document: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            let entry = document["entries"]
                .as_array()?
                .iter()
                .find(|entry| entry["path"].as_str() == Some(logical))?;
            let blob = entry["blob"].as_str()?;
            std::fs::read(self.root.join("blobs").join(blob)).ok()
        }
    }

    /// The JSON chunk of a glTF binary.
    fn census_glb(bytes: &[u8]) -> Option<serde_json::Value> {
        if bytes.len() < 20 || &bytes[..4] != b"glTF" {
            return None;
        }
        let length = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        serde_json::from_slice(bytes.get(20..20 + length)?).ok()
    }

    /// A glb's collision documents and per-node source records, turned into
    /// collider polygons in the fixture's own space. The filters mirror the
    /// bake's collection: active nodes, the navigation layer, the modifier
    /// recursion, and the rule that drops physics sources on an object that
    /// carries an agent or a carving obstacle.
    fn census_source(gltf: &serde_json::Value) -> CensusSource {
        let mut documents = Vec::new();
        let mut errors = 0usize;
        let mut values = vec![gltf["extras"]["fixtureCollision"].clone()];
        for node in gltf["nodes"].as_array().into_iter().flatten() {
            values.push(node["extras"]["fixtureCollision"].clone());
        }
        for value in values.into_iter().filter(|value| !value.is_null()) {
            match serde_json::from_value::<Document>(value) {
                Ok(document) => documents.push(document),
                Err(_) => errors += 1,
            }
        }
        let geometry: Vec<Geometry> = documents
            .iter()
            .flat_map(|document| document.geometry.iter().cloned())
            .collect();
        let nodes: Vec<serde_json::Value> = gltf["nodes"].as_array().cloned().unwrap_or_default();
        let mut source = CensusSource {
            physics: Vec::new(),
            carving: Vec::new(),
            kinds: [0; 4],
            other: 0,
            errors,
        };
        for (index, node) in nodes.iter().enumerate() {
            let extras = &node["extras"];
            if extras["active"].as_bool() == Some(false) {
                continue;
            }
            let raw = &extras["sourceCollision"];
            if raw.is_null() {
                continue;
            }
            let record: Node = match serde_json::from_value(raw.clone()) {
                Ok(record) => record,
                Err(_) => {
                    source.errors += 1;
                    continue;
                }
            };
            if !(record.fixture_nav_layer || record.authored_layer == Some(9)) {
                continue;
            }
            if census_ignored(&nodes, index, &mut source.errors) {
                continue;
            }
            let (transform, rotation) = test_node_world_transform(&nodes, index);
            if !record.has_nav_mesh_agent && record.nav_mesh_obstacles.is_empty() {
                for collider in &record.colliders {
                    match (collider.enabled, collider.is_trigger) {
                        (Some(false), _) | (_, Some(true)) => continue,
                        (Some(true), Some(false)) => {}
                        _ => {
                            source.errors += 1;
                            continue;
                        }
                    }
                    if collider.gap.is_some() {
                        source.errors += 1;
                        continue;
                    }
                    match collider.kind.as_str() {
                        "MeshCollider" => source.kinds[0] += 1,
                        "BoxCollider" => source.kinds[1] += 1,
                        "SphereCollider" => source.kinds[2] += 1,
                        "CapsuleCollider" => source.kinds[3] += 1,
                        _ => source.other += 1,
                    }
                    match collider_polygons(collider, &geometry, &transform, rotation) {
                        Ok(polygons) => source.physics.extend(polygons),
                        Err(_) => source.errors += 1,
                    }
                }
            }
            for obstacle in &record.nav_mesh_obstacles {
                match (obstacle.enabled, obstacle.carve) {
                    (Some(false), _) | (_, Some(false)) => continue,
                    (Some(true), Some(true)) => {}
                    _ => {
                        source.errors += 1;
                        continue;
                    }
                }
                match obstacle_polygon(obstacle, &transform, rotation) {
                    Ok(polygon) => source.carving.push(polygon),
                    Err(_) => source.errors += 1,
                }
            }
        }
        source
    }

    /// Whether a node's ancestor chain marks its sources ignored
    /// (`ignoreFromBuild` on a modifier that affects the home agent).
    /// Malformed modifiers count as errors, as the bake refuses them.
    fn census_ignored(nodes: &[serde_json::Value], start: usize, errors: &mut usize) -> bool {
        let mut current = start;
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current) {
                *errors += 1;
                return true;
            }
            if let Some(raw) = nodes[current]["extras"].get("sourceCollision") {
                if let Ok(record) = serde_json::from_value::<Node>(raw.clone()) {
                    for modifier in &record.modifiers {
                        match modifier.enabled {
                            Some(false) => continue,
                            Some(true) => {}
                            None => {
                                *errors += 1;
                                return true;
                            }
                        }
                        let Some(agents) = modifier.affected_agents.as_ref() else {
                            *errors += 1;
                            return true;
                        };
                        if !agents.contains(&-1) && !agents.contains(&super::AGENT) {
                            continue;
                        }
                        if modifier.override_area.is_none() || modifier.ignore_from_build.is_none() {
                            *errors += 1;
                            return true;
                        }
                        if modifier.override_area == Some(true) && modifier.area != Some(0) {
                            *errors += 1;
                            return true;
                        }
                        if modifier.ignore_from_build == Some(true) {
                            return true;
                        }
                    }
                }
            }
            let Some(parent) = nodes.iter().position(|node| {
                node["children"].as_array().is_some_and(|children| {
                    children
                        .iter()
                        .any(|child| child.as_u64() == Some(current as u64))
                })
            }) else {
                break;
            };
            current = parent;
        }
        false
    }

    /// Cell mask of the fixture's projected source footprint (collider meshes,
    /// solid primitives and carving hulls) at the bake's cell centres.
    fn census_footprint(source: &CensusSource) -> Vec<bool> {
        let mut mask = vec![false; CENSUS_CELLS * CENSUS_CELLS];
        for polygon in source.physics.iter().chain(source.carving.iter()) {
            if polygon.solid || polygon.triangles.is_empty() {
                census_hull_mask(&polygon.vertices, &mut mask);
            } else {
                for triangle in &polygon.triangles {
                    census_triangle_mask(triangle, &mut mask);
                }
            }
        }
        mask
    }

    fn census_hull_mask(vertices: &[[f32; 2]], mask: &mut [bool]) {
        if vertices.len() < 3 {
            return;
        }
        let min = vertices
            .iter()
            .fold([f32::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let max = vertices
            .iter()
            .fold([f32::NEG_INFINITY; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
        let (x0, z0) = census_cell_of(min[0], min[1]);
        let (x1, z1) = census_cell_of(max[0], max[1]);
        for z in z0.max(0)..=z1.min(CENSUS_CELLS as isize - 1) {
            for x in x0.max(0)..=x1.min(CENSUS_CELLS as isize - 1) {
                let point = [census_centre(x as usize), census_centre(z as usize)];
                let mut positive = false;
                let mut negative = false;
                for i in 0..vertices.len() {
                    let a = vertices[i];
                    let b = vertices[(i + 1) % vertices.len()];
                    let cross = (b[0] - a[0]) * (point[1] - a[1])
                        - (b[1] - a[1]) * (point[0] - a[0]);
                    positive |= cross > 1e-6;
                    negative |= cross < -1e-6;
                }
                if !(positive && negative) {
                    mask[z as usize * CENSUS_CELLS + x as usize] = true;
                }
            }
        }
    }

    fn census_triangle_mask(triangle: &[[f32; 3]; 3], mask: &mut [bool]) {
        let min = [
            triangle.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
            triangle.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min),
        ];
        let max = [
            triangle.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
            triangle.iter().map(|p| p[2]).fold(f32::NEG_INFINITY, f32::max),
        ];
        let (x0, z0) = census_cell_of(min[0], min[1]);
        let (x1, z1) = census_cell_of(max[0], max[1]);
        for z in z0.max(0)..=z1.min(CENSUS_CELLS as isize - 1) {
            for x in x0.max(0)..=x1.min(CENSUS_CELLS as isize - 1) {
                let point = [census_centre(x as usize), census_centre(z as usize)];
                if census_point_in_triangle(point, triangle) {
                    mask[z as usize * CENSUS_CELLS + x as usize] = true;
                }
            }
        }
    }

    fn census_cell_of(x: f32, z: f32) -> (isize, isize) {
        (
            ((x + CENSUS_HALF) / CENSUS_VOXEL).floor() as isize,
            ((z + CENSUS_HALF) / CENSUS_VOXEL).floor() as isize,
        )
    }

    fn census_centre(index: usize) -> f32 {
        -CENSUS_HALF + (index as f32 + 0.5) * CENSUS_VOXEL
    }

    fn census_point_in_triangle(point: [f32; 2], triangle: &[[f32; 3]; 3]) -> bool {
        let a = [triangle[0][0], triangle[0][2]];
        let b = [triangle[1][0], triangle[1][2]];
        let c = [triangle[2][0], triangle[2][2]];
        let determinant = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if determinant.abs() < 1e-12 {
            return false;
        }
        let u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / determinant;
        let v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / determinant;
        u >= -1e-5 && v >= -1e-5 && u + v <= 1.0 + 1e-5
    }

    /// Bakes every fixture glb that carries collider or obstacle extras over a
    /// flat six-metre floor and writes one row per fixture to `MOLY_CENSUS_OUT`.
    /// The row reports the source counts, the walkable cell count, the cells
    /// whose walk surface is more than one climb above the floor, and the
    /// walkable area inside the fixture's own projected source footprint.
    #[test]
    #[ignore = "requires MOLY_ASSET_ROOT and MOLY_CENSUS_OUT"]
    fn fixture_walk_census() {
        let root = std::path::PathBuf::from(
            std::env::var("MOLY_ASSET_ROOT")
                .expect("MOLY_ASSET_ROOT must point at the asset root"),
        );
        assert!(
            root.is_dir(),
            "MOLY_ASSET_ROOT is not a directory: {}",
            root.display()
        );
        let output = std::path::PathBuf::from(
            std::env::var("MOLY_CENSUS_OUT").expect("MOLY_CENSUS_OUT must name the output file"),
        );
        let assets = CensusAssets::open(&root);
        let index = assets
            .read("fixture-models/index.json")
            .unwrap_or_else(|| panic!("{} has no fixture-models/index.json", root.display()));
        let index: serde_json::Value =
            serde_json::from_slice(&index).expect("fixture-models/index.json is not JSON");
        let master: std::collections::HashMap<String, u64> = assets
            .read("mysekai-fixtures.json")
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|master| {
                master["fixtures"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| {
                        let bundle = row["assetbundleName"].as_str()?;
                        Some((format!("mysekai__fixture__{bundle}"), row["id"].as_u64()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let floor: [[[f32; 3]; 3]; 2] = [
            [
                [-CENSUS_HALF, 0.0, -CENSUS_HALF],
                [CENSUS_HALF, 0.0, -CENSUS_HALF],
                [CENSUS_HALF, 0.0, CENSUS_HALF],
            ],
            [
                [-CENSUS_HALF, 0.0, -CENSUS_HALF],
                [CENSUS_HALF, 0.0, CENSUS_HALF],
                [-CENSUS_HALF, 0.0, CENSUS_HALF],
            ],
        ];
        let volume = moly_law::carve::BuildVolume::centred(
            [0.0, 0.0],
            [CENSUS_HALF * 2.0, CENSUS_HALF * 2.0],
        );
        let mut lines = vec![
            "# fixture walk census".to_owned(),
            format!("# root {}", root.display()),
            "# name\tglb\tid\tmesh\tbox\tsphere\tcapsule\tother\tobstacles\terrors\twalkable\tabove_climb\tmax_offset\tfootprint_m2\twalkable_inside_m2\tmulti_surface".to_owned(),
        ];
        let (mut scanned, mut selected, mut missing, mut errors) = (0usize, 0usize, 0usize, 0usize);
        let (mut walkable_total, mut above_total, mut multi_total) = (0usize, 0usize, 0usize);
        let packages = index["packages"]
            .as_object()
            .expect("fixture-models/index.json has no packages");
        for (name, entry) in packages {
            if entry["status"] != "exported" {
                continue;
            }
            let Some(glb) = entry["glb"].as_str() else {
                continue;
            };
            scanned += 1;
            let Some(bytes) = assets.read(&format!("fixture-models/{glb}")) else {
                missing += 1;
                continue;
            };
            let Some(gltf) = census_glb(&bytes) else {
                errors += 1;
                continue;
            };
            let source = census_source(&gltf);
            errors += source.errors;
            if source.physics.is_empty() && source.carving.is_empty() {
                continue;
            }
            selected += 1;
            let mut collision_sources = source.physics.clone();
            collision_sources.extend(source.carving.iter().cloned());
            let field = moly_law::carve::WalkField::bake_colliders_in(
                &floor,
                &collision_sources,
                CENSUS_VOXEL,
                Some(volume),
            );
            let footprint = census_footprint(&source);
            let (mut above, mut inside_walkable, mut inside_footprint) = (0usize, 0usize, 0usize);
            let mut max_offset = 0.0f32;
            for z in 0..CENSUS_CELLS {
                for x in 0..CENSUS_CELLS {
                    let point = [census_centre(x), census_centre(z)];
                    let inside = footprint[z * CENSUS_CELLS + x];
                    if inside {
                        inside_footprint += 1;
                    }
                    if field.walkable_at(point) {
                        // The census floor is flat at y = 0, so the surface
                        // height is also the height above the floor.
                        let height = field.surface_height(point).unwrap_or_default();
                        max_offset = max_offset.max(height);
                        if height > moly_law::carve::AGENT_CLIMB + 1e-4 {
                            above += 1;
                        }
                        if inside {
                            inside_walkable += 1;
                        }
                    }
                }
            }
            walkable_total += field.counts().walkable;
            above_total += above;
            multi_total += field.counts().multi_surface;
            let id = master
                .get(name)
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_owned());
            let cell_area = CENSUS_VOXEL * CENSUS_VOXEL;
            lines.push(format!(
                "{name}\t{glb}\t{id}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{max_offset:.4}\t{:.4}\t{:.4}\t{}",
                source.kinds[0],
                source.kinds[1],
                source.kinds[2],
                source.kinds[3],
                source.other,
                source.carving.len(),
                source.errors,
                field.counts().walkable,
                above,
                inside_footprint as f32 * cell_area,
                inside_walkable as f32 * cell_area,
                field.counts().multi_surface,
            ));
        }
        assert!(
            scanned > 0,
            "{} listed no exported fixture packages",
            root.display()
        );
        assert!(
            selected > 0,
            "{} exported {scanned} fixture glbs but none carried collider or obstacle extras",
            root.display()
        );
        lines.push(format!(
            "# summary scanned={scanned} selected={selected} missing_glb={missing} errors={errors} walkable={walkable_total} above_climb={above_total} multi_surface={multi_total}"
        ));
        std::fs::write(&output, lines.join("\n") + "\n")
            .unwrap_or_else(|error| panic!("{}: {error}", output.display()));
        eprintln!(
            "fixture walk census: {selected} of {scanned} exported packages carry sources; wrote {}",
            output.display()
        );
    }
}
