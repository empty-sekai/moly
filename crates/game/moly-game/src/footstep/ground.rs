//! `FootSEController.GetGroundFixture(position)`: the fixture under the
//! foot on a housing site, or none.
//!
//! The position (relative to the site view) becomes a grid cell with
//! `FieldPosition.ToGridPosition`: `floor(x / tile)`, `floor(max(y, 0) /
//! tile)`, `floor(z / tile)`. Then three passes over the site's grid data,
//! each over the grids of one layout type:
//!
//! 1. floor (2): the first grid whose tile there is not null or empty and
//!    whose fixture is not a transparent block gives that fixture;
//! 2. rug (4): the first grid whose tile there exists gives its fixture;
//! 3. road (8): the first grid whose tile there carries a joint whose first
//!    joint fixture is still placed gives that fixture's id.
//!
//! Otherwise -1 (the ground cue plays).
//!
//! Product inputs: the floor grid is the fixture scene input's floor tile
//! map (the same `GetTileData(floor)` model the fixture interaction reads);
//! rug and road tiles are the footprints of the placed rows of that layout.
//! A floor tile whose occupant is not known stops the footstep with a WARN
//! instead of guessing between the fixture and the ground cue.

use bevy::prelude::*;
use moly_law::fixture::{
    position::{layout_type, TILE_SIZE},
    GridPosition,
};
use serde_json::Value;
use std::collections::HashMap;

use crate::player_fixture_action::FixtureTileKnowledge;

pub(crate) const PLAYER_DATA: &str = "moly://fixture-models/player-data.json";

/// The fixture master columns the footstep reads, per fixture id.
#[derive(Clone, Debug)]
pub(crate) struct FixtureRow {
    /// `mysekaiFixtureFootstepId`; None when the row does not carry it
    /// (the deserializer's 0).
    pub footstep: Option<i64>,
    pub handle_type: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FixtureRows {
    pub rows: HashMap<i32, FixtureRow>,
    /// Whether any row carries the footstep column. A document without it
    /// predates the column and cannot decide a fixture's footstep at all.
    pub column: bool,
}

impl FixtureRows {
    pub(crate) fn parse(player_data: &Value) -> Result<Self, String> {
        let list = player_data["tables"]["mysekaiFixtures"]
            .as_array()
            .ok_or("fixture catalogue has no mysekaiFixtures table")?;
        let mut rows = HashMap::with_capacity(list.len());
        let mut column = false;
        for row in list {
            let id = row["id"]
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or("mysekaiFixtures row without an integer id")?;
            let footstep = match row.get("mysekaiFixtureFootstepId") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    column = true;
                    Some(value.as_i64().ok_or_else(|| {
                        format!("mysekaiFixtures {id}: mysekaiFixtureFootstepId is not an integer")
                    })?)
                }
            };
            let handle_type = row["mysekaiFixtureHandleType"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            rows.insert(
                id,
                FixtureRow {
                    footstep,
                    handle_type,
                },
            );
        }
        Ok(Self { rows, column })
    }

    /// `MysekaiFixtureUtility.IsTransparentBlock`: the master handle type is
    /// `block_transparent`.
    fn is_transparent_block(&self, fixture_id: i32) -> bool {
        self.rows
            .get(&fixture_id)
            .is_some_and(|row| row.handle_type == "block_transparent")
    }
}

/// `FieldPosition.ToGridPosition` of a site-relative position, or None when
/// the cell lies outside the grid coordinate range (no grid has a tile
/// there).
pub(crate) fn grid_position(relative: Vec3) -> Option<GridPosition> {
    let cell = |value: f32| -> Option<i8> {
        let cell = (value / TILE_SIZE).floor();
        if cell.is_finite() && cell >= i8::MIN as f32 && cell <= i8::MAX as f32 {
            Some(cell as i8)
        } else {
            None
        }
    };
    Some(GridPosition::new(
        cell(relative.x)?,
        cell(relative.y.max(0.0))?,
        cell(relative.z)?,
    ))
}

/// One placed fixture as the passes see it.
pub(crate) struct Placed<'a> {
    pub uid: &'a str,
    pub master_id: i32,
    pub layout: u8,
    pub min: GridPosition,
    pub max: GridPosition,
    pub center_y: i8,
    pub height: i32,
}

#[derive(Debug, PartialEq)]
pub(crate) enum GroundFixture {
    Fixture { id: i32, pass: &'static str },
    None,
}

/// `GetGroundFixture` over the product inputs. Err names an undecidable
/// input.
pub(crate) fn ground_fixture(
    cell: Option<GridPosition>,
    floor: &crate::fixture_tiles::FixtureFloorTiles,
    placed: &[Placed<'_>],
    rows: &FixtureRows,
) -> Result<GroundFixture, String> {
    let Some(cell) = cell else {
        return Ok(GroundFixture::None);
    };
    let by_uid: HashMap<&str, &Placed<'_>> = placed.iter().map(|p| (p.uid, p)).collect();
    // 1. Floor grid.
    match floor.tile_at_grid(cell) {
        FixtureTileKnowledge::Missing | FixtureTileKnowledge::Empty => {}
        FixtureTileKnowledge::Occupied(uid) => {
            let placed = by_uid
                .get(uid.as_str())
                .ok_or_else(|| format!("floor tile {cell:?} names {uid}, which is not placed"))?;
            if !rows.is_transparent_block(placed.master_id) {
                return Ok(GroundFixture::Fixture {
                    id: placed.master_id,
                    pass: "floor",
                });
            }
        }
        FixtureTileKnowledge::ExistingUnknownOccupant | FixtureTileKnowledge::Unknown => {
            return Err(format!("floor tile {cell:?} occupant is not known"));
        }
    }
    let covers = |p: &&Placed<'_>, y_range: std::ops::Range<i32>| {
        (p.min.x..=p.max.x).contains(&cell.x)
            && (p.min.z..=p.max.z).contains(&cell.z)
            && y_range.contains(&(cell.y as i32))
    };
    // 2. Rug grid: the rug's footprint over its height.
    if let Some(rug) = placed.iter().find(|p| {
        p.layout & layout_type::RUG != 0
            && covers(p, p.center_y as i32..p.center_y as i32 + p.height.max(1))
    }) {
        return Ok(GroundFixture::Fixture {
            id: rug.master_id,
            pass: "rug",
        });
    }
    // 3. Road grid: the joint lies on every footprint cell at y = 0, and its
    // first joint fixture is placed (it is one of the placed rows).
    if let Some(road) = placed
        .iter()
        .find(|p| p.layout & layout_type::ROAD != 0 && covers(p, 0..1))
    {
        return Ok(GroundFixture::Fixture {
            id: road.master_id,
            pass: "road",
        });
    }
    Ok(GroundFixture::None)
}
