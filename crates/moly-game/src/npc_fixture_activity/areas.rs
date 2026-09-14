//! Source MotionArea overlap for the current complete floor layout.
//! The existing areas loader supplies the document once; no second IO path.

use bevy::prelude::*;
use moly_law::fixture::areas::{GridAreaData, rotated_center_grid};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::{
    fixture::OccupancyRow, fixture_tiles::FixtureFloorTiles,
    player_fixture_action::FixtureTileKnowledge,
};

#[derive(Resource, Default)]
pub(crate) struct NpcFixtureAreas {
    rows: HashMap<String, Result<(GridAreaData, GridAreaData), String>>,
}

impl NpcFixtureAreas {
    pub(crate) fn replace_document(&mut self, document: &Value) -> Result<(), String> {
        let packages = document["packages"]
            .as_object()
            .ok_or("areas document lacks packages")?;
        let mut rows = HashMap::new();
        for (name, source) in packages {
            let parsed = (|| {
                if source["hasMeta"].as_bool() != Some(true) || !source["readError"].is_null() {
                    return Err(format!("{name} has no complete source FixtureBundleMeta"));
                }
                Ok((
                    parse_grid(source, "motionArea")?,
                    parse_grid(source, "AddUsingGrid")?,
                ))
            })();
            rows.insert(name.clone(), parsed);
        }
        self.rows = rows;
        Ok(())
    }

    fn row(&self, package: &str) -> Result<&(GridAreaData, GridAreaData), String> {
        self.rows
            .get(package)
            .ok_or_else(|| format!("source areas not loaded for {package}"))?
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(crate) fn motion_overlaps(
        &self,
        row: &OccupancyRow,
        rows: &[OccupancyRow],
        floor: &FixtureFloorTiles,
    ) -> Result<bool, String> {
        let mut motion = self.row(row.package)?.0.clone();
        motion.rotate(row.direction, true);
        // FixtureManager first collects the other fixtures through the floor
        // grid cells of UpdateMotionAreaBoundList. Rugs do not become floor
        // obstacles merely because their renderer covers the same XZ.
        let bound_center =
            rotated_center_grid(row.layout_center, row.layout_grid_size, row.direction);
        let mut candidates = HashSet::new();
        for cell in &motion.enable_tiles {
            match floor.tile_at_grid(bound_center + *cell) {
                FixtureTileKnowledge::Occupied(uid) if uid != row.uid => {
                    candidates.insert(uid);
                }
                FixtureTileKnowledge::ExistingUnknownOccupant | FixtureTileKnowledge::Unknown => {
                    return Err("motion-area overlap needs complete floor occupants".into());
                }
                _ => {}
            }
        }
        // GridAreaData.GetOverlapState separately uses Controller.Center +
        // each rotated cell; do not substitute the bound-list center here.
        for uid in candidates {
            let mut others = rows.iter().filter(|other| other.uid == uid);
            let other = others
                .next()
                .ok_or_else(|| format!("motion-area occupant {uid} has no placement"))?;
            if others.next().is_some() {
                return Err(format!("motion-area occupant {uid} is ambiguous"));
            }
            let mut add = self.row(other.package)?.1.clone();
            add.rotate(other.direction, true);
            let add_center =
                rotated_center_grid(other.layout_center, other.layout_grid_size, other.direction);
            for cell in &motion.enable_tiles {
                let position = row.layout_center + *cell;
                let in_box = other.min.x <= position.x
                    && position.x <= other.max.x
                    && other.min.z <= position.z
                    && position.z <= other.max.z;
                let in_add = add.enable_tiles.iter().any(|cell| {
                    let point = add_center + *cell;
                    point.x == position.x && point.z == position.z
                });
                if in_box || in_add {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn parse_grid(row: &Value, key: &str) -> Result<GridAreaData, String> {
    let source = &row[key];
    if source["ragged"].as_bool() != Some(false)
        || source["rowsWithoutColumns"].as_u64() != Some(0)
        || !source["anomaly"].is_null()
    {
        return Err(format!("{key} matrix shape is unresolved"));
    }
    let rows = source["dims"]["rows"]
        .as_u64()
        .ok_or_else(|| format!("{key} rows missing"))? as usize;
    let cols = source["dims"]["cols"]
        .as_u64()
        .ok_or_else(|| format!("{key} columns missing"))? as usize;
    let cells = source["cells"]
        .as_array()
        .ok_or_else(|| format!("{key} cells missing"))?;
    if cells.len() != rows {
        return Err(format!("{key} row count disagrees"));
    }
    let mut parsed = Vec::new();
    for cell_row in cells {
        let cell_row = cell_row
            .as_array()
            .ok_or_else(|| format!("{key} row is not an array"))?;
        if cell_row.len() != cols {
            return Err(format!("{key} column count disagrees"));
        }
        for cell in cell_row {
            parsed.push(
                cell.as_bool()
                    .ok_or_else(|| format!("{key} cell is not boolean"))?,
            );
        }
    }
    Ok(GridAreaData::from_meta(rows, cols, |r, c| {
        parsed[r * cols + c]
    }))
}

