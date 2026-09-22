//! Bind source FenceView references and apply the connection law to scene nodes.
//! Meshes/materials stay shared, so Bevy can instance the visible pole/wing parts.

use super::{FixturePlacements, FixtureScenesReady};
use bevy::{gltf::GltfExtras, prelude::*};
use moly_assets::scene_state::SetSourceActive;
use moly_law::fixture::fence::{connected_parts, neighbor_positions, Neighbor};
use moly_law::fixture::{Direction, GridPosition};

#[derive(Component)]
pub(super) struct FixtureRow(pub usize);

struct Part {
    entity: Entity,
    pole: bool,
    index: usize,
}

#[derive(Component)]
pub(super) struct FenceNodes(Vec<Part>);

fn reflected_direction(direction: Direction) -> Direction {
    match direction {
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
        other => other,
    }
}

fn reflected_cell(position: GridPosition) -> GridPosition {
    // A closed grid cell [x, x+1] reflects to [-x-1, -x]. This is the
    // same involution as the player-data placement boundary, including -128.
    GridPosition::new((-i16::from(position.x) - 1) as i8, position.y, position.z)
}

/// FenceView's part IDs remain source-local even though the actual node TRS
/// and placement rows are canonical reflect-X. Evaluate the original neighbor
/// law in source space, then map its query cells back to the runtime grid.
/// Mirroring the meshes again would fix one end while breaking the others.
fn connection_queries(
    min: GridPosition,
    max: GridPosition,
    direction: Direction,
) -> (Direction, [GridPosition; 8]) {
    let source_direction = reflected_direction(direction);
    let source_min = GridPosition::new(reflected_cell(max).x, min.y, min.z);
    let source_max = GridPosition::new(reflected_cell(min).x, max.y, max.z);
    (
        source_direction,
        neighbor_positions(source_min, source_max, source_direction).map(reflected_cell),
    )
}

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
        let (source_direction, positions) =
            connection_queries(placed.min, placed.max, row.direction);
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
                    .map(|r| reflected_direction(r.2.direction)),
                joint_uids: occupants.iter().map(|r| r.1).collect(),
            }
        });
        let resolved = connected_parts(source_direction, &neighbors);
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

#[cfg(test)]
mod tests {
    use super::*;
    use moly_law::fixture::position::{layout_footprint, layout_type};
    use moly_law::fixture::Vector3Int;

    #[test]
    fn source_part_queries_round_trip_all_four_placement_directions() {
        let center = GridPosition::new(6, 0, -4);
        let size = Vector3Int::new(2, 2, 2);
        for direction in [Direction::Front, Direction::Left, Direction::Back, Direction::Right] {
            let (source_min, source_max) =
                layout_footprint(center, size, direction, layout_type::FLOOR).unwrap();
            let (canonical_center, canonical_direction, layout) =
                moly_assets::player_data::mirror_fixture_layout(
                    center, size, direction, layout_type::FLOOR,
                ).unwrap();
            let (canonical_min, canonical_max) =
                layout_footprint(canonical_center, size, canonical_direction, layout).unwrap();
            let (actual_direction, queries) =
                connection_queries(canonical_min, canonical_max, canonical_direction);
            assert_eq!(actual_direction, direction);
            assert_eq!(queries,
                neighbor_positions(source_min, source_max, direction).map(reflected_cell));
        }
    }

    #[test]
    fn reflected_front_endpoint_selects_the_wing_toward_its_neighbor() {
        let (direction, queries) = connection_queries(
            GridPosition::new(0, 0, 0), GridPosition::new(1, 1, 1), Direction::Front,
        );
        // Runtime +X is source left. The exported short_left wing runs from
        // the center toward +X; short_right would point away from this neighbor.
        let neighbors = queries.map(|p| {
            let occupied = (2..=3).contains(&p.x) && (0..=1).contains(&p.z);
            Neighbor {
                direction: occupied.then_some(Direction::Front),
                joint_uids: if occupied { vec![7] } else { vec![] },
            }
        });
        let parts = connected_parts(direction, &neighbors);
        assert!(parts.wings[2] && !parts.wings[1]);
        assert!(parts.poles[6] && !parts.poles[3]);
        assert!(parts.poles[0] && !parts.wings[0]);
    }
}
