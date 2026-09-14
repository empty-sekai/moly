//! Authored floor-grid lookup for furniture interaction.
//!
//! This is the tile input side of the interaction/navigation boundary.  It is
//! deliberately not a NavMesh implementation and does not infer a grid from a
//! rendered ground AABB.  The dimensions come from `Site::floor_grid` and the
//! origin is the active site's `SiteView` position, matching the source
//! `FixtureController` calls that subtract the current site position before
//! `FieldPosition.ToGridPosition` and `GetTileData(floor = 2)`.

use std::collections::HashMap;

use bevy::prelude::Vec3;
use moly_law::fixture::{position::TILE_SIZE, GridPosition};

use crate::{
    player_fixture_action::FixtureTileKnowledge,
    site::FloorGridLayout,
};

/// A saved/edited occupancy identity for one floor tile.  `Unknown` means the
/// editor or server supplied a cell but not a valid instance UID; it is not an
/// empty cell.  A package name is intentionally absent: multiple instances of
/// one package are distinct only by their saved UID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TileOccupancyIdentity {
    Uid(String),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TileOccupancyEntry {
    pub position: GridPosition,
    pub identity: TileOccupancyIdentity,
}

/// Whether absent entries can be interpreted as authored-empty.  A partial or
/// unavailable occupancy source must retain unknown occupants for valid tiles;
/// it must not turn an unconnected editor view into a permissive empty map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OccupancyCoverage {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TileOccupancySnapshot {
    pub coverage: Option<OccupancyCoverage>,
    pub entries: Vec<TileOccupancyEntry>,
}

impl TileOccupancySnapshot {
    pub(crate) fn complete(entries: Vec<TileOccupancyEntry>) -> Self {
        Self { coverage: Some(OccupancyCoverage::Complete), entries }
    }

    pub(crate) fn partial(entries: Vec<TileOccupancyEntry>) -> Self {
        Self { coverage: Some(OccupancyCoverage::Partial), entries }
    }

    pub(crate) fn unavailable() -> Self {
        Self { coverage: Some(OccupancyCoverage::Unavailable), entries: Vec::new() }
    }
}

/// A floor grid with source dimensions and a site-relative world origin.
#[derive(Clone, Debug)]
pub(crate) struct FixtureFloorTiles {
    pub(crate) layout: FloorGridLayout,
    pub(crate) site_origin: Vec3,
    cells: HashMap<GridPosition, FixtureTileKnowledge>,
    coverage: OccupancyCoverage,
    /// Entries outside the authored floor grid are retained as diagnostics,
    /// never wrapped into an i8 coordinate and never made occupied.
    pub(crate) ignored_out_of_grid: usize,
    /// Conflicting duplicate identities make that tile unknown.  This covers
    /// duplicate UIDs at one position and a UID/Unknown collision.
    pub(crate) conflicting_cells: usize,
}

impl FixtureFloorTiles {
    pub(crate) fn new(
        layout: FloorGridLayout,
        site_origin: Vec3,
        snapshot: TileOccupancySnapshot,
    ) -> Result<Self, String> {
        if layout.width <= 0 || layout.height <= 0 || layout.depth <= 0 {
            return Err(format!(
                "floor layout {} has non-positive cells {}x{}x{}",
                layout.layout_id, layout.width, layout.height, layout.depth
            ));
        }
        if !site_origin.is_finite() {
            return Err("active site origin is not finite".to_owned());
        }
        let coverage = snapshot.coverage.unwrap_or(OccupancyCoverage::Unavailable);
        let mut cells = HashMap::new();
        let mut ignored_out_of_grid = 0;
        let mut conflicting_cells = 0;
        for entry in snapshot.entries {
            if !contains_grid(&layout, entry.position) {
                ignored_out_of_grid += 1;
                continue;
            }
            let value = match entry.identity {
                TileOccupancyIdentity::Uid(uid) if !uid.is_empty() => FixtureTileKnowledge::Occupied(uid),
                TileOccupancyIdentity::Uid(_) | TileOccupancyIdentity::Unknown => {
                    FixtureTileKnowledge::ExistingUnknownOccupant
                }
            };
            match cells.get_mut(&entry.position) {
                None => {
                    cells.insert(entry.position, value);
                }
                Some(previous) if *previous == value => {}
                Some(previous) => {
                    *previous = FixtureTileKnowledge::ExistingUnknownOccupant;
                    conflicting_cells += 1;
                }
            }
        }
        Ok(Self {
            layout,
            site_origin,
            cells,
            coverage,
            ignored_out_of_grid,
            conflicting_cells,
        })
    }

    pub(crate) fn coverage(&self) -> OccupancyCoverage {
        self.coverage
    }

    /// Source `GridData.SetupGridData` coordinate domain:
    /// `[-ceil(width/2), ceil(width/2))`, `y in [0,height)`, and the same for z.
    pub(crate) fn contains_grid(&self, position: GridPosition) -> bool {
        contains_grid(&self.layout, position)
    }

    pub(crate) fn tile_at_grid(&self, position: GridPosition) -> FixtureTileKnowledge {
        if !self.contains_grid(position) {
            return FixtureTileKnowledge::Missing;
        }
        if let Some(value) = self.cells.get(&position) {
            return value.clone();
        }
        if self.coverage == OccupancyCoverage::Complete {
            FixtureTileKnowledge::Empty
        } else {
            FixtureTileKnowledge::ExistingUnknownOccupant
        }
    }

    /// Source `FieldPosition.ToGridPosition`: subtract site origin, divide by
    /// `TILE_SIZE`, floor x/z, and force relative Y to 0. The source fixture
    /// callers set `v61.y = 0.0` before calling `ToGridPosition`; this is not a
    /// generic world-height conversion. An i32 coordinate outside the
    /// GridPosition byte range is returned as
    /// `Missing` rather than wrapped into a different valid cell.
    pub(crate) fn tile_at_world(&self, world: Vec3) -> FixtureTileKnowledge {
        let Some(position) = self.world_to_grid(world) else {
            return FixtureTileKnowledge::Missing;
        };
        self.tile_at_grid(position)
    }

    pub(crate) fn world_to_grid(&self, world: Vec3) -> Option<GridPosition> {
        if !world.is_finite() {
            return None;
        }
        let relative = world - self.site_origin;
        let x = (relative.x / TILE_SIZE).floor();
        let y = 0i32;
        let z = (relative.z / TILE_SIZE).floor();
        if !x.is_finite() || !z.is_finite()
            || x < i32::MIN as f32 || x > i32::MAX as f32
            || z < i32::MIN as f32 || z > i32::MAX as f32
        {
            return None;
        }
        let position = GridPosition::new(x as i32 as i8, y as i32 as i8, z as i32 as i8);
        // Reject byte wrapping before exposing the result to the grid map.
        if x as i32 != position.x as i32 || z as i32 != position.z as i32 {
            return None;
        }
        Some(position)
    }
}

fn contains_grid(layout: &FloorGridLayout, position: GridPosition) -> bool {
    let half_width = (layout.width + 1) / 2;
    let half_depth = (layout.depth + 1) / 2;
    (-(half_width)..half_width).contains(&(position.x as i32))
        && (0..layout.height).contains(&(position.y as i32))
        && (-(half_depth)..half_depth).contains(&(position.z as i32))
}
