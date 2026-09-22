//! PhysicsCollider inputs for the single-surface navigation host.
//!
//! This consumes canonical source geometry, never logical occupancy boxes.
//! Convex MeshColliders retain their source solid-hull semantics: their point
//! clouds are convex-hulled in 3-D, then the world-space boundary triangles are
//! supplied to the rasterizer as one filled solid. Non-convex meshes retain
//! their authored triangles without filling between separate surfaces.
//! This models mathematical convexity, not PhysX's exact cooking/simplification.

use bevy::{
    ecs::system::SystemParam,
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

const AGENT: i32 = -1372625422;

#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct CollisionBakeStatus {
    pub ready: bool,
    pub reason: String,
    pub colliders: usize,
    pub polygons: usize,
}

#[derive(SystemParam)]
pub(crate) struct CollisionInputs<'w, 's> {
    roots: Query<
        'w,
        's,
        (Entity, Option<&'static crate::fixture::FixtureSource>),
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

pub(crate) struct Collected {
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
    convex: Option<bool>,
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
    pub(crate) fn collect(&self, expected: usize) -> Result<Collected, String> {
        if self.ready.is_none() {
            return Err("fixture collision scenes loading".into());
        }
        let roots: HashSet<_> = self.roots.iter().map(|(entity, _)| entity).collect();
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
                    .and_then(|(_, source)| source)
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
        let mut result = Collected {
            polygons: Vec::new(),
            colliders: 0,
        };
        for (entity, (node, inactive)) in &source {
            if *inactive {
                continue;
            }
            if node.active_self.is_none() || node.active_in_hierarchy.is_none() {
                return Err("fixture collider active state missing".into());
            }
            let mut current = *entity;
            let mut active = true;
            loop {
                let state = activities
                    .get(&current)
                    .copied()
                    .or_else(|| source.get(&current).and_then(|(node, _)| node.active_self));
                if state == Some(false) {
                    active = false;
                    break;
                }
                let Some(parent) = parents.get(&current) else {
                    break;
                };
                current = *parent;
            }
            if !active {
                continue;
            }
            if !(node.fixture_nav_layer || node.authored_layer == Some(9)) {
                continue;
            }
            // SceneReady is raised before transform propagation. Compose the
            // current local chain to avoid freezing the first bake at stale
            // identity GlobalTransforms or one-frame-old editor positions.
            let mut chain = Vec::new();
            let mut current = *entity;
            let mut seen = HashSet::new();
            loop {
                if !seen.insert(current) {
                    return Err("fixture transform cycle".into());
                }
                chain.push(
                    *locals
                        .get(&current)
                        .ok_or("fixture collider local transform missing")?,
                );
                let Some(parent) = parents.get(&current) else {
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
            let transform = &composed;
            if !transform.to_matrix().is_finite() {
                return Err("fixture collider world transform nonfinite".into());
            }
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
    // Unity 2022.3.62f2 x86_64 NavMeshObstacle::GetWorldExtents (0xbacede)
    // uses absolute lossy scale, sharing max(X,Z) between capsule radii.
    // GetWorldCenterAndAxes (0xbacfa4) transforms the center and separately
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
            // Both CalcCapsuleWorldExtents (0xbcc43a) and the capsule branch
            // of CarveNavMeshTile (0xbcdeb4..0xbcded3) use max(halfHeight-r,0).
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
            if !matches!(collider.convex, Some(false) | Some(true)) {
                return Err("MeshCollider convex state missing".into());
            }
            // Unity cooks convex=true from the whole point cloud. Source
            // triangles can be concave or disconnected; passing them through
            // as a non-convex surface would invent openings in a solid hull.
            let solid = collider.convex == Some(true);
            let (positions, triangles) = if solid {
                convex_mesh(&mesh.positions)?
            } else {
                (mesh.positions.clone(), mesh.triangles.clone())
            };
            vec![project_mesh(&positions, &triangles, transform, solid)?]
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
    fn convex_mesh_fills_disconnected_source_surfaces_as_one_solid() {
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
        let mut collider = Collider {
            kind: "MeshCollider".into(),
            enabled: Some(true),
            is_trigger: Some(false),
            convex: Some(true),
            geometry_id: Some("mesh".into()),
            gap: None,
            center: None,
            size: None,
            radius: None,
            height: None,
            direction: None,
        };
        let polygons = collider_polygons(
            &collider,
            &[mesh.clone()],
            &GlobalTransform::IDENTITY,
            Quat::IDENTITY,
        )
        .unwrap();
        assert_eq!(polygons.len(), 1);
        assert!(polygons[0].solid);
        assert_eq!(
            polygons[0].triangles.len(),
            12,
            "convex=true closes the box"
        );
        collider.convex = Some(false);
        let polygons = collider_polygons(
            &collider,
            &[mesh],
            &GlobalTransform::IDENTITY,
            Quat::IDENTITY,
        )
        .unwrap();
        assert!(!polygons[0].solid);
        assert_eq!(
            polygons[0].triangles.len(),
            4,
            "non-convex retains open surfaces"
        );
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

    #[test]
    #[ignore = "requires source GLBs via MOLY_COLLISION_GLB_DIR"]
    fn canonical_rug_tree_gazebo_convex_hulls() {
        let directory = std::path::PathBuf::from(
            std::env::var("MOLY_COLLISION_GLB_DIR").expect("source fixture-models directory"),
        );
        for name in [
            "mysekai__fixture__mdl_env0001_rug_picnicsheet1.glb",
            "mysekai__fixture__mdl_env0001_fixture_tree1.glb",
            "mysekai__fixture__mdl_ext0020_fixture_gazebo1.glb",
        ] {
            let bytes = std::fs::read(directory.join(name)).unwrap();
            assert_eq!(&bytes[..4], b"glTF");
            let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let gltf: serde_json::Value =
                serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
            let document: Document =
                serde_json::from_value(gltf["extras"]["fixtureCollision"].clone()).unwrap();
            let mut count = 0;
            for node in gltf["nodes"].as_array().unwrap() {
                let Some(source) = node
                    .get("extras")
                    .and_then(|extras| extras.get("sourceCollision"))
                else {
                    continue;
                };
                let node: Node = serde_json::from_value(source.clone()).unwrap();
                for collider in node.colliders.iter().filter(|collider| {
                    collider.kind == "MeshCollider" && collider.convex == Some(true)
                }) {
                    let start = std::time::Instant::now();
                    let polygons = collider_polygons(
                        collider,
                        &document.geometry,
                        &GlobalTransform::IDENTITY,
                        Quat::IDENTITY,
                    )
                    .unwrap();
                    assert_eq!(polygons.len(), 1);
                    assert!(polygons[0].solid);
                    assert!(!polygons[0].triangles.is_empty());
                    eprintln!(
                        "{name}: hull {} triangles in {:?}",
                        polygons[0].triangles.len(),
                        start.elapsed()
                    );
                    let surface = [
                        [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
                        [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
                    ];
                    for voxel in [0.01, 0.05] {
                        let start = std::time::Instant::now();
                        let field =
                            moly_law::carve::WalkField::bake_colliders(&surface, &polygons, voxel);
                        eprintln!(
                            "{name}: {}m raster {} cells / {} obstacle cells in {:?}",
                            voxel,
                            field.counts().cells,
                            field.counts().obstacle_nulled,
                            start.elapsed()
                        );
                    }
                    count += 1;
                }
            }
            assert!(count > 0, "{name} has a source convex collider");
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
}
