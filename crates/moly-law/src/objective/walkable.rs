//! The site's walkable cell list (the site model's
//! `UpdateWalkableGridPositionList`), in the source's cell order.
//!
//! The list is built from:
//! 1. every floor-grid cell at height 0 whose tile is empty, or whose
//!    fixture lets a character through (a gate inside its add-using grid, a
//!    transparent block), in the grid's loop order: x from `-width` while
//!    `x < width - 1`, z the same inside it (the loop runs past the
//!    grid's own cells, and a cell the grid does not hold is skipped);
//! 2. then every rug-grid cell at height 0 that holds a rug, in the same
//!    loop order;
//! 3. without repeats (the first occurrence keeps its place);
//! 4. without the cells around each system fixture of the site: its
//!    bounding box widened by 1 cell in x and z for the home and by 2 for
//!    every other system fixture, at the fixture's own grid height (so a
//!    fixture standing above height 0 removes nothing from the list);
//! 5. keeping only the cells whose corner the navigation mesh holds within
//!    [`NAVMESH_SAMPLE_DISTANCE`].
//!
//! Every cell coordinate is a signed byte, as the source's grid position
//! packs it.

use crate::fixture::GridPosition;

/// The navigation mesh sample distance of step 5 (a float literal).
pub const NAVMESH_SAMPLE_DISTANCE: f32 = 0.01;

/// The system fixture type of the home (the one with the narrow margin).
pub const SYSTEM_FIXTURE_TYPE_HOME: i32 = 2;

/// A floor tile as the movable-cell loop reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorTile {
    /// The grid holds no tile at this cell (outside its own cells).
    Absent,
    /// The tile holds no fixture.
    Empty,
    /// The tile holds a fixture a character can pass (a gate inside its
    /// add-using grid, or a transparent block).
    Passable,
    /// The tile holds any other fixture.
    Blocked,
}

/// A system fixture of the site as step 4 reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFixture {
    /// Its bounding box, both ends included.
    pub min: GridPosition,
    pub max: GridPosition,
    /// Its own grid position's height.
    pub grid_y: i8,
    /// Its system fixture type, when it has one.
    pub system_type: Option<i32>,
}

/// The margin of step 4: 1 for the home, 2 otherwise (no type included).
pub fn system_margin(system_type: Option<i32>) -> i32 {
    if system_type == Some(SYSTEM_FIXTURE_TYPE_HOME) {
        1
    } else {
        2
    }
}

/// Step 1 over a grid of `width` x `depth` tiles.
pub fn movable_cells(
    width: i32,
    depth: i32,
    mut tile: impl FnMut(i8, i8) -> FloorTile,
) -> Vec<GridPosition> {
    let mut cells = Vec::new();
    let mut x = -width;
    while x < width - 1 {
        let mut z = -depth;
        while z < depth - 1 {
            let (cx, cz) = (x as i8, z as i8);
            if matches!(tile(cx, cz), FloorTile::Empty | FloorTile::Passable) {
                cells.push(GridPosition::new(cx, 0, cz));
            }
            z += 1;
        }
        x += 1;
    }
    cells
}

/// Step 2 over a grid of `width` x `depth` tiles: `holds` answers `None`
/// for a cell the grid does not hold, else whether a fixture is there.
pub fn placed_cells(
    width: i32,
    depth: i32,
    mut holds: impl FnMut(i8, i8) -> Option<bool>,
) -> Vec<GridPosition> {
    let mut cells = Vec::new();
    let mut x = -width;
    while x < width - 1 {
        let mut z = -depth;
        while z < depth - 1 {
            let (cx, cz) = (x as i8, z as i8);
            if holds(cx, cz) == Some(true) {
                cells.push(GridPosition::new(cx, 0, cz));
            }
            z += 1;
        }
        x += 1;
    }
    cells
}

/// Step 4's removed cells, fixture by fixture, x outer and z inner.
pub fn excluded_cells(system: &[SystemFixture]) -> Vec<GridPosition> {
    let mut cells = Vec::new();
    for fixture in system {
        let margin = system_margin(fixture.system_type);
        let min_x = (fixture.min.x as i32 - margin) as i8;
        let min_z = (fixture.min.z as i32 - margin) as i8;
        let max_x = (fixture.max.x as i32 + margin) as i8;
        let max_z = (fixture.max.z as i32 + margin) as i8;
        let mut x = min_x as i32;
        while x <= max_x as i32 {
            let mut z = min_z as i32;
            while z <= max_z as i32 {
                cells.push(GridPosition::new(x as i8, fixture.grid_y, z as i8));
                z += 1;
            }
            x += 1;
        }
    }
    cells
}

/// Steps 2 to 5: the rug cells appended, repeats dropped, the excluded
/// cells removed, then the navigation mesh test (`navmesh` is asked once
/// per remaining cell, in order).
pub fn walkable_cells(
    movable: &[GridPosition],
    rugs: &[GridPosition],
    excluded: &[GridPosition],
    mut navmesh: impl FnMut(GridPosition) -> bool,
) -> Vec<GridPosition> {
    let removed: std::collections::HashSet<GridPosition> = excluded.iter().copied().collect();
    let mut seen = std::collections::HashSet::new();
    let mut cells = Vec::new();
    for &cell in movable.iter().chain(rugs) {
        if !seen.insert(cell) || removed.contains(&cell) {
            continue;
        }
        if navmesh(cell) {
            cells.push(cell);
        }
    }
    cells
}
