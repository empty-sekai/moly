//! The harvest views' own writes to their NavMeshObstacles. The obstacles
//! themselves (their records, the engine's per-frame `UpdateState` and the
//! carve) are the walk field's runtime carving (`walk_face::RuntimeObstacles`),
//! which adopts every spawned node that carries obstacle records; this module
//! only performs what the harvest views change on top of the prefab.
//!
//! The source:
//! - `HarvestBaseView.SetupScaleRandom` (tree and stone Setup, before they
//!   hide anything) takes `GetComponentInChildren<NavMeshObstacle>()` (the
//!   first obstacle on an active GameObject: the view's own object first,
//!   then depth first in child order) and sets its radius to the scale draw
//!   times its radius. The same draw scales the transform, so that
//!   obstacle's world radius carries the draw twice.
//! - `NavMeshObstacle::SetRadius(v)` (the engine): `v = max(v, 1e-5)`, then
//!   the extents' x and z both become `v`, and the obstacle is marked dirty
//!   (its next `UpdateState` snapshots it again).
//! - `MysekaiAreadDriftageView.ChangeAfterObject` (and its
//!   `ForceChangeAfterObject`, which runs it) first disables
//!   `_navmeshObstacle`, then waits 1.0 s and turns the barrel off.
//! - `MysekaiAreaTreasureBoxView.ChangeAfterObject` and
//!   `ForceChangeAfterObject` enable `_treasureBoxLidNavMeshObstacle` (the
//!   open lid's box, disabled in the prefab).
//!
//! The other switches are the views' ordinary SetActive calls (the tree's
//! fall turns the before mesh off and the under part on, the stone's break
//! turns the stone off, a view loaded as harvested turns its object off):
//! the runtime carving already reads a node's activity.

use bevy::prelude::*;
use moly_assets::scene_state::SourceInactive;
use moly_assets::source_navigation::{
    SourceHarvestView, SourceNavMeshObstacles, SourceObjectIdentity,
};

use super::{scales_randomly, HarvestObject, HarvestViewNodes, STATUS_HARVESTED};
use crate::walk_face::RuntimeObstacles;

/// Frames a root may wait for the walk field to adopt its obstacle nodes
/// before the wait is reported.
const ADOPT_WAIT_REPORT_FRAMES: u32 = 120;

/// Marks a root whose Setup-time obstacle writes are done.
#[derive(Component)]
pub(crate) struct ObstaclesSetUp;

/// Frames a root has waited for the adoption of its obstacle nodes.
#[derive(Component, Default)]
pub(crate) struct ObstacleAdoptWait(u32);

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

/// The node that carries the obstacle component `id`, with the index of
/// that component among the node's obstacle records.
fn node_of_component(
    nodes: &[Entity],
    sources: &Query<&SourceNavMeshObstacles>,
    id: i64,
) -> Option<(Entity, usize)> {
    nodes.iter().find_map(|node| {
        let rows = sources.get(*node).ok()?;
        let index = rows.0.iter().position(|row| row.component_id == id)?;
        Some((*node, index))
    })
}

/// A view field's reference (`{file, id}`), as a component id of this scene.
fn field_component(
    nodes: &[Entity],
    views: &Query<&SourceHarvestView>,
    field: &str,
) -> Option<i64> {
    nodes.iter().find_map(|node| {
        let view = views.get(*node).ok()?;
        let fields: serde_json::Value = serde_json::from_str(&view.fields_json).ok()?;
        let reference = fields.get(field)?;
        if reference["file"].as_i64() != Some(0) {
            return None;
        }
        reference["id"]
            .as_str()
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|id| *id != 0)
    })
}

/// Update, after the views bind: `SetupScaleRandom`'s obstacle radius (tree
/// and stone), and a treasure box loaded as harvested enables its lid
/// (`ForceChangeAfterObject`). A root waits until the walk field has adopted
/// the obstacle node it writes.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn set_up(
    mut commands: Commands,
    mut roots: Query<
        (Entity, &HarvestObject, Option<&mut ObstacleAdoptWait>),
        (With<HarvestViewNodes>, Without<ObstaclesSetUp>),
    >,
    children: Query<&Children>,
    identities: Query<&SourceObjectIdentity>,
    views: Query<&SourceHarvestView>,
    inactive: Query<(), With<SourceInactive>>,
    sources: Query<&SourceNavMeshObstacles>,
    mut runtime: Query<&mut RuntimeObstacles>,
) {
    for (root, object, wait) in &mut roots {
        let order = source_order(root, &children, &identities);
        // SetupScaleRandom: GetComponentInChildren skips inactive
        // GameObjects and everything under them (SourceInactive is the
        // inherited state).
        let radius_node = (object.status != STATUS_HARVESTED && scales_randomly(object.class))
            .then(|| {
                order.iter().copied().find(|node| {
                    inactive.get(*node).is_err()
                        && sources.get(*node).is_ok_and(|rows| !rows.0.is_empty())
                })
            });
        // ForceChangeAfterObject of a box loaded as harvested.
        let lid = (object.status == STATUS_HARVESTED
            && object.class == "MysekaiAreaTreasureBoxView")
            .then(|| {
                field_component(&order, &views, "_treasureBoxLidNavMeshObstacle")
                    .and_then(|id| node_of_component(&order, &sources, id))
            });
        let pending: Vec<Entity> = radius_node
            .flatten()
            .into_iter()
            .chain(lid.flatten().map(|(node, _)| node))
            .filter(|node| runtime.get(*node).is_err())
            .collect();
        if !pending.is_empty() {
            match wait {
                Some(mut wait) => {
                    wait.0 += 1;
                    if wait.0 == ADOPT_WAIT_REPORT_FRAMES {
                        error!(
                            "[harvest-obstacle] {}#{}: the walk field has not adopted obstacle nodes {pending:?} after {} frames; the view's obstacle writes wait",
                            object.leaf, object.fixture_id, wait.0
                        );
                    }
                }
                None => {
                    commands.entity(root).insert(ObstacleAdoptWait(1));
                }
            }
            continue;
        }
        commands
            .entity(root)
            .insert(ObstaclesSetUp)
            .remove::<ObstacleAdoptWait>();
        match radius_node {
            Some(Some(node)) => {
                let mut records = runtime.get_mut(node).expect("adopted above");
                match records.set_radius(0, |radius| object.scale * radius) {
                    Some((before, after)) => info!(
                        "[harvest-obstacle] {}#{} SetupScaleRandom: the first obstacle ({node:?}) radius {before:.3} -> {after:.3} (scale {:.3}; the transform carries the same scale)",
                        object.leaf, object.fixture_id, object.scale
                    ),
                    None => error!(
                        "[harvest-obstacle] {}#{} SetupScaleRandom: node {node:?} has no obstacle record 0",
                        object.leaf, object.fixture_id
                    ),
                }
            }
            Some(None) => info!(
                "[harvest-obstacle] {}#{} SetupScaleRandom: no NavMeshObstacle on an active node",
                object.leaf, object.fixture_id
            ),
            None => {}
        }
        match lid {
            Some(Some((node, index))) => {
                let mut records = runtime.get_mut(node).expect("adopted above");
                switch_record(&mut records, index, true, object, "_treasureBoxLidNavMeshObstacle", "ForceChangeAfterObject");
            }
            Some(None) => error!(
                "[harvest-obstacle] {}#{} ForceChangeAfterObject: _treasureBoxLidNavMeshObstacle names no obstacle of this prefab",
                object.leaf, object.fixture_id
            ),
            None => {}
        }
    }
}

/// `NavMeshObstacle.enabled = value` on the node's record `index`. The walk
/// field's records switch per node; a node whose other records would switch
/// with it is refused by name.
fn switch_record(
    records: &mut RuntimeObstacles,
    index: usize,
    value: bool,
    object: &HarvestObject,
    field: &str,
    reason: &str,
) {
    if records.len() != 1 || index != 0 {
        error!(
            "[harvest-obstacle] {}#{} {reason}: {field} is record {index} of {} on its node; a per-record switch is not available, enabled = {value} not applied",
            object.leaf,
            object.fixture_id,
            records.len()
        );
        return;
    }
    records.set_enabled(value);
    info!(
        "[harvest-obstacle] {}#{} {reason}: {field} enabled = {value}",
        object.leaf, object.fixture_id
    );
}

/// A view's `NavMeshObstacle.enabled = value` on the obstacle its serialized
/// `field` references (queued from the views' ChangeAfterObject).
pub(crate) fn switch(
    world: &mut World,
    root: Entity,
    field: &'static str,
    value: bool,
    reason: &'static str,
) {
    let mut state: bevy::ecs::system::SystemState<(
        Query<&HarvestObject>,
        Query<&Children>,
        Query<&SourceObjectIdentity>,
        Query<&SourceHarvestView>,
        Query<&SourceNavMeshObstacles>,
        Query<&mut RuntimeObstacles>,
    )> = bevy::ecs::system::SystemState::new(world);
    let (objects, children, identities, views, sources, mut runtime) = state.get_mut(world);
    let Ok(object) = objects.get(root) else {
        return;
    };
    let order = source_order(root, &children, &identities);
    let Some((node, index)) = field_component(&order, &views, field)
        .and_then(|id| node_of_component(&order, &sources, id))
    else {
        error!(
            "[harvest-obstacle] {}#{} {reason}: the view field {field} names no obstacle of this prefab; enabled = {value} not applied",
            object.leaf, object.fixture_id
        );
        return;
    };
    match runtime.get_mut(node) {
        Ok(mut records) => switch_record(&mut records, index, value, object, field, reason),
        Err(_) => error!(
            "[harvest-obstacle] {}#{} {reason}: the walk field has not adopted {field}'s node {node:?}; enabled = {value} not applied",
            object.leaf, object.fixture_id
        ),
    }
}
