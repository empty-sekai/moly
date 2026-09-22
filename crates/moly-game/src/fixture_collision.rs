//! PhysicsCollider inputs for the single-surface navigation host.
//!
//! This consumes canonical source geometry, never logical occupancy boxes.
//! Convex colliders use the XZ projection of their input hull; nonconvex meshes
//! retain individual projected triangles. Curved primitives use circumscribed
//! polygons. These are explicit conservative 2D operations, not Unity cooking.

use bevy::{ecs::system::SystemParam, gltf::GltfExtras, prelude::*};
use moly_law::carve::ColliderPolygon;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

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
    roots: Query<'w, 's, Entity, With<crate::fixture::FixtureRoot>>,
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
        let roots: HashSet<_> = self.roots.iter().collect();
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
            let value: serde_json::Value = serde_json::from_str(&extras.value)
                .map_err(|error| format!("fixture collision extras: {error}"))?;
            if let Some(value) = value.get("fixtureCollision") {
                let document: Document =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                validate_contract(document.schema_version, &document.coordinate_contract)?;
                if document.units != "source-unity-unit" {
                    return Err("fixture collision unit contract unsupported".into());
                }
                if !document.gaps.is_empty() {
                    return Err(format!(
                        "fixture collision extraction has {} unresolved inputs",
                        document.gaps.len()
                    ));
                }
                if documents.insert(root, document).is_some() {
                    return Err("fixture has multiple collision scene roots".into());
                }
            }
            if let Some(value) = value.get("sourceCollision") {
                let node: Node =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
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
            for local in chain.into_iter().rev() {
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
                let center = Vec3::from(obstacle.center.ok_or("obstacle center missing")?);
                let extents = Vec3::from(obstacle.extents.ok_or("obstacle extents missing")?);
                let points = match obstacle.shape {
                    Some(1) => box_points(center, extents * 2.0)?,
                    Some(0) => curved_points(center, extents.x.max(extents.z), extents.y * 2.0, 1)?,
                    _ => return Err("unknown NavMeshObstacle shape".into()),
                };
                result.polygons.push(project(points, transform)?);
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

fn collider_polygons(
    collider: &Collider,
    geometry: &[Geometry],
    transform: &GlobalTransform,
) -> Result<Vec<ColliderPolygon>, String> {
    let points = match collider.kind.as_str() {
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
            if collider.convex == Some(false) {
                return mesh
                    .triangles
                    .iter()
                    .map(|tri| {
                        let points: Result<Vec<_>, _> = tri
                            .iter()
                            .map(|i| {
                                mesh.positions
                                    .get(*i)
                                    .copied()
                                    .map(Vec3::from)
                                    .ok_or("collider index out of range".to_owned())
                            })
                            .collect();
                        project(points?, transform)
                    })
                    .collect();
            }
            if collider.convex != Some(true) {
                return Err("MeshCollider convex state missing".into());
            }
            mesh.positions.iter().copied().map(Vec3::from).collect()
        }
        "BoxCollider" => box_points(
            Vec3::from(collider.center.ok_or("box center missing")?),
            Vec3::from(collider.size.ok_or("box size missing")?),
        )?,
        "SphereCollider" => curved_points(
            Vec3::from(collider.center.ok_or("sphere center missing")?),
            collider.radius.ok_or("sphere radius missing")?,
            0.0,
            1,
        )?,
        "CapsuleCollider" => curved_points(
            Vec3::from(collider.center.ok_or("capsule center missing")?),
            collider.radius.ok_or("capsule radius missing")?,
            collider.height.ok_or("capsule height missing")?,
            collider.direction.ok_or("capsule direction missing")?,
        )?,
        other => return Err(format!("unsupported collider shape {other}")),
    };
    Ok(vec![project(points, transform)?])
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

fn curved_points(center: Vec3, radius: f32, height: f32, axis: usize) -> Result<Vec<Vec3>, String> {
    if !center.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || !height.is_finite()
        || height < 0.0
        || axis > 2
    {
        return Err("invalid curved collider geometry".into());
    }
    // A circumscribed 32-sided cylinder contains the capsule. This is labelled
    // conservative projection rather than native capsule tessellation.
    let radial = radius / (std::f32::consts::PI / 32.0).cos();
    let half = (height * 0.5).max(radius);
    let mut points = Vec::with_capacity(64);
    for end in [-half, half] {
        for step in 0..32 {
            let angle = step as f32 * std::f32::consts::TAU / 32.0;
            let mut p = Vec3::ZERO;
            p[axis] = end;
            p[(axis + 1) % 3] = radial * angle.cos();
            p[(axis + 2) % 3] = radial * angle.sin();
            points.push(center + p);
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
        assert!(result.polygons[0]
            .vertices
            .iter()
            .all(|p| p[0] >= 9.5 && p[0] <= 10.5 && p[1] >= 2.5 && p[1] <= 3.5));
    }

    #[test]
    fn collector_rejects_missing_contract_instead_of_using_occupancy() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
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
        assert!(polygon
            .vertices
            .iter()
            .any(|p| (p[0] - 6.0).abs() < 1e-5 && (p[1] - 3.0).abs() < 1e-5));
    }

    #[test]
    fn curved_projection_is_conservative_and_invalid_inputs_fail() {
        let points = curved_points(Vec3::ZERO, 0.5, 2.0, 1).unwrap();
        assert_eq!(points.len(), 64);
        assert!(points.iter().all(|p| Vec2::new(p.x, p.z).length() >= 0.5));
        assert!(box_points(Vec3::ZERO, Vec3::new(-1.0, 1.0, 1.0)).is_err());
        assert!(validate_contract(1, "legacy-unity").is_err());
    }
}
