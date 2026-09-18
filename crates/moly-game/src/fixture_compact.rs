//! Small product-owned offline layout. Coordinates are mock saved input;
//! dimensions and master IDs are from the same source table as activity lookup.
//! The full showcase stays available through scene_content=full. No assets or
//! user-edited/saved placements are deleted when this preset is selected.

use super::{layout_type, Direction, GridPosition, PlacementMock};

pub(super) const PLACEMENTS: [PlacementMock; 6] = [
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0001_fixture_chair1",
        min: GridPosition { x: -16, y: 0, z: 0 },
        max: GridPosition { x: -15, y: 1, z: 1 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 8,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0001_fixture_lamp1",
        // Fits even the source L1 home grid [-18,18), without changing any
        // already-saved layout. This is a starter position, not source geometry.
        min: GridPosition { x: -18, y: 0, z: 0 },
        max: GridPosition { x: -17, y: 3, z: 1 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 146,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0002_fixture_fridge1",
        min: GridPosition { x: -16, y: 0, z: 8 },
        max: GridPosition { x: -15, y: 3, z: 9 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 157,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0014_fixture_synthesizer1",
        min: GridPosition { x: -4, y: 0, z: 8 },
        max: GridPosition { x: -1, y: 1, z: 9 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 297,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_chr0004_fixture_nenerobo1",
        min: GridPosition { x: -10, y: 0, z: 0 },
        max: GridPosition { x: -9, y: 2, z: 1 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 423,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_cncollect_rug_star3",
        min: GridPosition {
            x: -14,
            y: 0,
            z: 10,
        },
        max: GridPosition { x: -7, y: 0, z: 17 },
        center_y: 0,
        layout: layout_type::RUG,
        direction: Direction::Front,
        fixture_id: 90005,
    },
];
