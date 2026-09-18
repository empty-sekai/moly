//! Bind source FenceView references and apply the connection law to scene nodes.
//! Meshes/materials stay shared, so Bevy can instance the visible pole/wing parts.

use super::{FixturePlacements, FixtureScenesReady};
use bevy::{gltf::GltfExtras, prelude::*};
use moly_assets::scene_state::SetSourceActive;
use moly_law::fixture::fence::{connected_parts, neighbor_positions, Neighbor};

#[derive(Component)]
pub(super) struct FixtureRow(pub usize);

struct Part {
    entity: Entity,
    pole: bool,
    index: usize,
}

#[derive(Component)]
pub(super) struct FenceNodes(Vec<Part>);

pub(super) fn bind_scene(
    root: Entity,
    children: &Query<&Children>,
    extras: &Query<&GltfExtras>,
    commands: &mut Commands,
) {
    let mut stack = vec![root];
    let mut parts = Vec::new();
    let mut fence = false;
    while let Some(entity) = stack.pop() {
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
        let Ok(extras) = extras.get(entity) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&extras.value) else {
            continue;
        };
        fence |= value.get("fenceView").and_then(|v| v.as_bool()) == Some(true);
        let Some(active) = value.get("fenceActive").and_then(|v| v.as_bool()) else {
            continue;
        };
        commands.queue(SetSourceActive { entity, active });
        if let Some(part) = value.get("fencePart") {
            let pole = match part.get("kind").and_then(|v| v.as_str()) {
                Some("pole") => true,
                Some("wing") => false,
                _ => panic!("FenceView part kind is invalid"),
            };
            let index = part
                .get("type")
                .and_then(|v| v.as_u64())
                .filter(|&v| v < 9)
                .expect("FenceView part type must be 0..8") as usize;
            parts.push(Part {
                entity,
                pole,
                index,
            });
        }
    }
    if fence {
        commands.entity(root).insert(FenceNodes(parts));
    }
}

pub(super) fn update_connections(
    ready: Option<Res<FixtureScenesReady>>,
    placements: Res<FixturePlacements>,
    fences: Query<(Entity, &FixtureRow, Ref<FenceNodes>)>,
    mut commands: Commands,
) {
    let Some(ready) = ready else {
        return;
    };
    if !ready.is_added()
        && !placements.is_changed()
        && !fences.iter().any(|(_, _, nodes)| nodes.is_added())
    {
        return;
    }
    let mut rows: Vec<_> = fences
        .iter()
        .map(|(entity, index, _)| {
            let row = &placements.rows[index.0];
            (index.0, entity.to_bits(), row, row.placed())
        })
        .collect();
    rows.sort_by_key(|r| r.0);
    for (_, index, nodes) in &fences {
        let row = &placements.rows[index.0];
        let placed = row.placed();
        let positions = neighbor_positions(placed.min, placed.max, row.direction);
        let neighbors = positions.map(|p| {
            // Source TileData stores joint UID order; first joint selects kind
            // and direction, while UID overlap examines all occupants.
            let occupants: Vec<_> = rows
                .iter()
                .filter(|(_, _, _, bounds)| {
                    p.x >= bounds.min.x
                        && p.x <= bounds.max.x
                        && p.z >= bounds.min.z
                        && p.z <= bounds.max.z
                        && p.y == bounds.min.y
                })
                .collect();
            Neighbor {
                // These seven source fixture IDs map one-to-one to packages;
                // color choices live inside the same package and still connect.
                direction: occupants
                    .first()
                    .filter(|r| r.2.package == row.package)
                    .map(|r| r.2.direction),
                joint_uids: occupants.iter().map(|r| r.1).collect(),
            }
        });
        let resolved = connected_parts(row.direction, &neighbors);
        for part in &nodes.0 {
            let active = if part.pole {
                resolved.poles[part.index]
            } else {
                resolved.wings[part.index]
            };
            commands.queue(SetSourceActive {
                entity: part.entity,
                active,
            });
        }
        info!(
            "围栏连接：row {} {:?} poles {:?} wings {:?}",
            index.0, row.direction, resolved.poles, resolved.wings
        );
    }
}
