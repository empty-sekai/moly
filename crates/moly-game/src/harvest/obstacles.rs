//! The harvest objects' NavMeshObstacles: what they carve into the walk
//! field, and when.
//!
//! The source:
//! - Each harvest prefab carries its obstacles on its own nodes (the tree on
//!   its before mesh and its under part, the stone, the barrel, the toolbox,
//!   the treasure box and its lid); every one is `carving` with
//!   `carveOnlyStationary`. An obstacle takes part while its GameObject is
//!   active in the hierarchy and the component is enabled, so the views'
//!   ordinary switches move it: the tree's fall turns the before mesh off
//!   and the under part on; the stone's break turns the stone off; a view
//!   loaded as harvested turns its object off.
//! - `HarvestBaseView.SetupScaleRandom` (tree and stone Setup, before they
//!   hide anything) takes `GetComponentInChildren<NavMeshObstacle>()` (the
//!   first obstacle on an active GameObject, the view's own object first,
//!   then depth first in child order) and sets its radius to the scale draw
//!   times the radius. The same draw scales the transform, so that
//!   obstacle's world radius carries the draw twice.
//! - `NavMeshObstacle::SetRadius(v)` (the engine): `v = max(v, 1e-5)`, then
//!   the extents' x and z both become `v`.
//! - `MysekaiAreadDriftageView.ChangeAfterObject` (and its
//!   `ForceChangeAfterObject`, which runs it) first disables
//!   `_navmeshObstacle`, then waits 1.0 s and turns the barrel off.
//! - `MysekaiAreaTreasureBoxView.ChangeAfterObject` and
//!   `ForceChangeAfterObject` enable `_treasureBoxLidNavMeshObstacle` (the
//!   open lid's own box, disabled in the prefab).
//! - The engine updates each obstacle every frame
//!   (`NavMeshObstacle::UpdateState`): a dirty obstacle (a setter ran)
//!   snapshots its transform. With `carveOnlyStationary`: a stationary
//!   obstacle that `HasMoved(max(moveThreshold, 1e-5))` becomes moving (its
//!   carve is lifted), snapshots and zeroes its timer; a moving one that
//!   `HasMoved(max(moveThreshold * 0.1, 1e-5))` snapshots and zeroes its
//!   timer, otherwise adds the frame time and becomes stationary (carves
//!   again) once the timer passes `carvingTimeToStationary`.
//!   `HasMoved(t)`: the squared distance from the snapshot position above
//!   `t^2`, or the snapshot-to-now rotation angle (`2 acos(min(|dot|, 1))`)
//!   squared times the snapshot's squared lossy scale times its squared
//!   world extents above `t^2`, or the squared lossy-scale change times the
//!   squared world extents above `t^2`.
//!
//! Product mapping: a node is active when it is not source-inactive and no
//! node from it up to the harvest root is hidden (the harvest views' SetActive
//! is the product's visibility switch). [`HarvestNavObstacles`] lists every
//! carving obstacle with its pose; its `revision` advances whenever that list
//! changes, which is what the walk field's carve reads. A newly active
//! obstacle carves at once (the engine's initial moving state of a new
//! obstacle was not read).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use moly_assets::scene_state::SourceInactive;
use moly_assets::source_navigation::{
    SourceHarvestView, SourceNavMeshObstacle, SourceNavMeshObstacles, SourceObjectIdentity,
};

use super::{scales_randomly, HarvestObject, HarvestRoot, HarvestViewNodes, STATUS_HARVESTED};

/// `NavMeshObstacle::SetRadius` and `UpdateState`: the floor of a radius
/// and of a move threshold.
const FLOOR: f32 = 1e-5;
/// `UpdateState`: a moving obstacle's threshold factor.
const MOVING_THRESHOLD_FACTOR: f32 = 0.1;

/// One carving obstacle as the walk field's carve reads it.
#[derive(Clone, Debug)]
pub(crate) struct HarvestObstacle {
    /// The harvest root it belongs to.
    pub(crate) owner: Entity,
    /// Its node.
    pub(crate) node: Entity,
    /// Its data (the radius already set by `SetupScaleRandom`).
    pub(crate) obstacle: SourceNavMeshObstacle,
    /// The node's world transform at the last snapshot.
    pub(crate) world: GlobalTransform,
    /// The node's world rotation at the last snapshot.
    pub(crate) rotation: Quat,
}

/// Every harvest obstacle that carves now. `revision` advances whenever the
/// list changes (an obstacle starts or stops carving, or re-carves at a new
/// snapshot).
#[derive(Resource, Default, Debug)]
pub(crate) struct HarvestNavObstacles {
    pub(crate) revision: u64,
    pub(crate) rows: Vec<HarvestObstacle>,
}

/// The engine's per-obstacle state (`UpdateState`).
#[derive(Clone, Debug)]
struct Tracked {
    owner: Entity,
    obstacle: SourceNavMeshObstacle,
    position: Vec3,
    rotation: Quat,
    lossy: Vec3,
    world: GlobalTransform,
    world_extents_sq: f32,
    moving: bool,
    timer: f32,
}

#[derive(Resource, Default)]
pub(crate) struct ObstacleStates(HashMap<(Entity, i64), Tracked>);

/// Marks a root whose Setup-time obstacle work is done.
#[derive(Component)]
pub(crate) struct ObstaclesSetUp;

/// The nodes of a root in the source's depth-first order (the root first,
/// each node's children in the source child order).
fn source_order(
    root: Entity,
    children: &Query<&Children>,
    identities: &Query<&SourceObjectIdentity>,
) -> Vec<Entity> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        order.push(entity);
        let Ok(kids) = children.get(entity) else {
            continue;
        };
        let mut kids: Vec<Entity> = kids.iter().collect();
        if let Ok(parent) = identities.get(entity) {
            let rank = |child: &Entity| {
                identities
                    .get(*child)
                    .ok()
                    .and_then(|identity| {
                        parent
                            .child_order
                            .iter()
                            .position(|id| *id == identity.transform)
                    })
                    .unwrap_or(usize::MAX)
            };
            kids.sort_by_key(rank);
        }
        // Pushed in reverse so the first child pops first.
        stack.extend(kids.into_iter().rev());
    }
    order
}

/// `NavMeshObstacle::SetRadius`.
fn set_radius(obstacle: &mut SourceNavMeshObstacle, value: f32) {
    let value = value.max(FLOOR);
    obstacle.extents.x = value;
    obstacle.extents.z = value;
}

/// Update, after the views bind: `SetupScaleRandom`'s obstacle radius (tree
/// and stone), and a treasure box loaded as harvested enables its lid
/// (`ForceChangeAfterObject`).
#[allow(clippy::type_complexity)]
pub(crate) fn set_up(
    mut commands: Commands,
    roots: Query<(Entity, &HarvestObject), (With<HarvestViewNodes>, Without<ObstaclesSetUp>)>,
    children: Query<&Children>,
    identities: Query<&SourceObjectIdentity>,
    inactive: Query<(), With<SourceInactive>>,
    mut obstacles: Query<&mut SourceNavMeshObstacles>,
) {
    for (root, object) in &roots {
        commands.entity(root).insert(ObstaclesSetUp);
        if object.status != STATUS_HARVESTED && scales_randomly(object.class) {
            // GetComponentInChildren: inactive GameObjects and everything
            // under them are skipped.
            let order = source_order(root, &children, &identities);
            let first = order.into_iter().find(|node| {
                inactive.get(*node).is_err()
                    && obstacles.get(*node).is_ok_and(|rows| !rows.0.is_empty())
            });
            match first.and_then(|node| obstacles.get_mut(node).ok().map(|rows| (node, rows))) {
                Some((node, mut rows)) => {
                    let before = rows.0[0].extents.x;
                    set_radius(&mut rows.0[0], object.scale * before);
                    info!(
                        "[harvest-obstacle] {}#{} SetupScaleRandom: obstacle {} on {node:?} radius {before:.3} -> {:.3} (scale {:.3}; the transform carries the same scale)",
                        object.leaf, object.fixture_id, rows.0[0].component_id, rows.0[0].extents.x, object.scale
                    );
                }
                None => info!(
                    "[harvest-obstacle] {}#{} SetupScaleRandom: no NavMeshObstacle on an active node",
                    object.leaf, object.fixture_id
                ),
            }
        }
        if object.status == STATUS_HARVESTED && object.class == "MysekaiAreaTreasureBoxView" {
            commands.queue(move |world: &mut World| {
                switch(
                    world,
                    root,
                    "_treasureBoxLidNavMeshObstacle",
                    true,
                    "ForceChangeAfterObject",
                );
            });
        }
    }
}

/// A view's `NavMeshObstacle.enabled = value` on the obstacle its serialized
/// `field` references.
pub(crate) fn switch(world: &mut World, root: Entity, field: &str, value: bool, reason: &str) {
    let leaf = world
        .get::<HarvestObject>(root)
        .map(|object| format!("{}#{}", object.leaf, object.fixture_id))
        .unwrap_or_else(|| format!("{root:?}"));
    let mut nodes = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        nodes.push(entity);
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    let id = nodes.iter().find_map(|node| {
        let view = world.get::<SourceHarvestView>(*node)?;
        let fields: serde_json::Value = serde_json::from_str(&view.fields_json).ok()?;
        let reference = fields.get(field)?;
        if reference["file"].as_i64() != Some(0) {
            return None;
        }
        reference["id"]
            .as_str()
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|id| *id != 0)
    });
    let Some(id) = id else {
        error!("[harvest-obstacle] {leaf} {reason}: the view field {field} names no obstacle of this prefab; enabled = {value} not applied");
        return;
    };
    for node in nodes {
        let Some(mut rows) = world.get_mut::<SourceNavMeshObstacles>(node) else {
            continue;
        };
        if let Some(row) = rows.0.iter_mut().find(|row| row.component_id == id) {
            row.enabled = value;
            info!("[harvest-obstacle] {leaf} {reason}: {field} ({id}) enabled = {value}");
            return;
        }
    }
    error!("[harvest-obstacle] {leaf} {reason}: {field} names component {id}, which no node of the scene carries; enabled = {value} not applied");
}

/// `Quaternion.Angle`'s engine form: `2 acos(min(|dot|, 1))`.
fn angle(a: Quat, b: Quat) -> f32 {
    2.0 * a.dot(b).abs().min(1.0).acos()
}

/// `NavMeshObstacle::HasMoved(t)` against the snapshot.
fn has_moved(tracked: &Tracked, position: Vec3, rotation: Quat, lossy: Vec3, t: f32) -> bool {
    let t2 = t * t;
    if (position - tracked.position).length_squared() > t2 {
        return true;
    }
    let a = angle(tracked.rotation, rotation);
    if a * a * tracked.lossy.length_squared() * tracked.world_extents_sq > t2 {
        return true;
    }
    (tracked.lossy - lossy).length_squared() * tracked.world_extents_sq > t2
}

/// `NavMeshObstacle::GetWorldExtents` squared length: the extents times the
/// absolute lossy scale, the capsule's radii sharing the larger of x and z.
fn world_extents_sq(obstacle: &SourceNavMeshObstacle, lossy: Vec3) -> f32 {
    let lossy = lossy.abs();
    let e = obstacle.extents;
    let world = match obstacle.shape {
        0 => {
            let r = e.x * lossy.x.max(lossy.z);
            Vec3::new(r, e.y * lossy.y, r)
        }
        _ => e * lossy,
    };
    world.length_squared()
}

/// Update, after the harvest action set: the active carving obstacles, run
/// through the engine's stationary rule, published with a revision.
#[allow(clippy::type_complexity)]
pub(crate) fn publish(
    time: Res<Time>,
    roots: Query<(Entity, &Transform), (With<HarvestRoot>, With<ObstaclesSetUp>)>,
    children: Query<&Children>,
    nodes: Query<(Option<&Transform>, Option<&Visibility>, Has<SourceInactive>)>,
    obstacles: Query<&SourceNavMeshObstacles>,
    mut states: ResMut<ObstacleStates>,
    mut published: ResMut<HarvestNavObstacles>,
) {
    let dt = time.delta_secs();
    let mut seen: HashSet<(Entity, i64)> = HashSet::new();
    let mut changed = false;
    for (root, root_transform) in &roots {
        // Walk down with the composed transform and the activity so far.
        let mut stack = vec![(root, GlobalTransform::from(*root_transform), true)];
        while let Some((entity, world, parent_active)) = stack.pop() {
            let Ok((_, visibility, source_inactive)) = nodes.get(entity) else {
                continue;
            };
            let active = parent_active
                && !source_inactive
                && !matches!(visibility, Some(Visibility::Hidden));
            if !active {
                continue;
            }
            if let Ok(rows) = obstacles.get(entity) {
                for row in rows.0.iter().filter(|row| row.enabled && row.carve) {
                    let key = (entity, row.component_id);
                    seen.insert(key);
                    let (lossy, rotation, position) = world.to_scale_rotation_translation();
                    match states.0.get_mut(&key) {
                        None => {
                            states.0.insert(
                                key,
                                Tracked {
                                    owner: root,
                                    obstacle: row.clone(),
                                    position,
                                    rotation,
                                    lossy,
                                    world,
                                    world_extents_sq: world_extents_sq(row, lossy),
                                    moving: false,
                                    timer: 0.0,
                                },
                            );
                            changed = true;
                        }
                        Some(tracked) => {
                            // A setter ran (radius, extents): dirty, snapshot.
                            if tracked.obstacle != *row {
                                tracked.obstacle = row.clone();
                                snapshot(tracked, world, position, rotation, lossy);
                                if !tracked.moving {
                                    changed = true;
                                }
                            }
                            changed |= update_state(tracked, world, position, rotation, lossy, dt);
                        }
                    }
                }
            }
            if let Ok(kids) = children.get(entity) {
                for kid in kids.iter() {
                    let local = nodes
                        .get(kid)
                        .ok()
                        .and_then(|(transform, _, _)| transform.copied())
                        .unwrap_or_default();
                    stack.push((kid, world.mul_transform(local), active));
                }
            }
        }
    }
    states.0.retain(|key, tracked| {
        let keep = seen.contains(key);
        if !keep && !tracked.moving {
            changed = true;
        }
        keep
    });
    if changed {
        published.revision += 1;
        published.rows = states
            .0
            .iter()
            .filter(|(_, tracked)| !tracked.moving)
            .map(|((node, _), tracked)| HarvestObstacle {
                owner: tracked.owner,
                node: *node,
                obstacle: tracked.obstacle.clone(),
                world: tracked.world,
                rotation: tracked.rotation,
            })
            .collect();
        published
            .rows
            .sort_by_key(|row| (row.node, row.obstacle.component_id));
        info!(
            "[harvest-obstacle] carving obstacles: {} (revision {}; {} tracked, {} moving): {:?}",
            published.rows.len(),
            published.revision,
            states.0.len(),
            states.0.values().filter(|tracked| tracked.moving).count(),
            published
                .rows
                .iter()
                .map(|row| {
                    let at = row.world.translation();
                    format!(
                        "{:?}/{:?} #{} shape {} extents ({:.3}, {:.3}, {:.3}) at ({:.2}, {:.2}, {:.2}) yaw {:.1}",
                        row.owner,
                        row.node,
                        row.obstacle.component_id,
                        row.obstacle.shape,
                        row.obstacle.extents.x,
                        row.obstacle.extents.y,
                        row.obstacle.extents.z,
                        at.x,
                        at.y,
                        at.z,
                        row.rotation.to_euler(EulerRot::YXZ).0.to_degrees()
                    )
                })
                .collect::<Vec<_>>()
        );
    }
}

fn snapshot(
    tracked: &mut Tracked,
    world: GlobalTransform,
    position: Vec3,
    rotation: Quat,
    lossy: Vec3,
) {
    tracked.position = position;
    tracked.rotation = rotation;
    tracked.lossy = lossy;
    tracked.world = world;
    tracked.world_extents_sq = world_extents_sq(&tracked.obstacle, lossy);
}

/// `NavMeshObstacle::UpdateState` with `carveOnlyStationary`; returns
/// whether the carved state changed (a carve lifted or laid again).
fn update_state(
    tracked: &mut Tracked,
    world: GlobalTransform,
    position: Vec3,
    rotation: Quat,
    lossy: Vec3,
    dt: f32,
) -> bool {
    let threshold = tracked.obstacle.move_threshold;
    if !tracked.obstacle.only_stationary {
        // Carving while moving: every move re-carves at the new snapshot.
        if has_moved(tracked, position, rotation, lossy, threshold.max(FLOOR)) {
            snapshot(tracked, world, position, rotation, lossy);
            tracked.moving = false;
            tracked.timer = 0.0;
            return true;
        }
        tracked.moving = false;
        tracked.timer = 0.0;
        return false;
    }
    if tracked.moving {
        let t = (threshold * MOVING_THRESHOLD_FACTOR).max(FLOOR);
        if has_moved(tracked, position, rotation, lossy, t) {
            tracked.timer = 0.0;
            snapshot(tracked, world, position, rotation, lossy);
            return false;
        }
        tracked.timer += dt;
        if tracked.timer > tracked.obstacle.stationary_time {
            tracked.moving = false;
            return true;
        }
        false
    } else if has_moved(tracked, position, rotation, lossy, threshold.max(FLOOR)) {
        tracked.moving = true;
        tracked.timer = 0.0;
        snapshot(tracked, world, position, rotation, lossy);
        true
    } else {
        false
    }
}

/// Queued by the site transition: the obstacles go with the site.
pub(crate) fn clear_for_site_change(world: &mut World) {
    world.resource_mut::<ObstacleStates>().0.clear();
    let mut published = world.resource_mut::<HarvestNavObstacles>();
    if !published.rows.is_empty() {
        published.rows.clear();
        published.revision += 1;
    }
}
