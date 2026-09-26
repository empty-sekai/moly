//! The site model's walkable cell list (UpdateWalkableGridPositionList) on
//! this host's layout, the list the gate's random cells, its near members'
//! ring and the general talk's floor position draw from (see
//! [`moly_law::objective::walkable`] for the rule).
//!
//! Inputs: the current floor-grid snapshot (its tiles, the add-using cells
//! of a gate, a transparent block's handle type), the rug placements, the
//! site's system fixtures (a placement whose master is in the system
//! fixture table; the master's fixture type is the system type exactly for
//! those masters) with their system fixture type, and the navigation mesh
//! at each cell's corner.
//!
//! The source's cells run in source grid coordinates; this host's grid
//! mirrors x (a source cell x is this host's cell -x-1). The list is built
//! in the source's cell order and handed out as this host's cells, whose
//! corner positions the consumers already use (the named corner difference
//! of the mirrored frame stays as it was).
//!
//! Named assumptions: the rug grid has the floor grid's size; a rug holds
//! the cells of its bounding box at the heights of its master's grid height
//! (no add-using cells). A snapshot with unresolved occupants, or any input
//! still loading, gives no list: the consumers hold.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::GridPosition;
use moly_law::objective::walkable::{self as law, FloorTile, SystemFixture};
use moly_law::objective::Cell;

use crate::fixture::{FixturePlacements, FixtureRoot, OccupancyRow};
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::fixture_scene_inputs::FixtureSceneInputs;
use crate::fixture_tiles::OccupancyCoverage;
use crate::npc_objective::ObjectiveFace;
use crate::player_fixture_action::FixtureTileKnowledge;
use crate::site::GroundEpoch;

const SYSTEM_FIXTURES: &str = "moly://mysekai-system-fixtures.json";

/// One consumer's copy of the list, rebuilt when its inputs change.
#[derive(Default)]
pub(crate) struct WalkableCache {
    table_handle: Option<Handle<JsonAsset>>,
    /// Master fixture id -> system fixture type value.
    table: Option<Result<HashMap<i32, i32>, String>>,
    key: Option<(u64, u64, u64, usize)>,
    cells: Option<Result<Vec<Cell>, String>>,
    reported: Option<String>,
}

/// Everything the list reads, with the consumer's cache.
#[derive(SystemParam)]
pub(crate) struct WalkableSource<'w, 's> {
    server: Res<'w, AssetServer>,
    json: Res<'w, Assets<JsonAsset>>,
    supply: Res<'w, crate::fixture_scene_inputs::FixtureSceneSupply>,
    placements: Res<'w, FixturePlacements>,
    identities: Query<'w, 's, &'static FixtureActivityIdentity, With<FixtureRoot>>,
    tables: Option<Res<'w, crate::fixture_activity_data::FixtureActivityTables>>,
    areas: Res<'w, crate::npc_fixture_activity::NpcFixtureAreas>,
    face: Option<Res<'w, ObjectiveFace>>,
    epoch: Option<Res<'w, GroundEpoch>>,
    cache: Local<'s, WalkableCache>,
}

/// The system fixture type values by name (MysekaiSystemFixtureType).
fn system_type_value(name: &str) -> Option<i32> {
    Some(match name {
        "craft_tool" => 0,
        "chest" => 1,
        "home" => 2,
        "blueprint_shop" => 3,
        "convert" => 4,
        "mysekai_information" => 5,
        "music_play" => 6,
        "avatar_dress_up" => 7,
        "birthday" => 8,
        _ => return None,
    })
}

fn parse_table(text: &str) -> Result<HashMap<i32, i32>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("{SYSTEM_FIXTURES}: {error}"))?;
    let entries = value["entries"]
        .as_object()
        .ok_or_else(|| format!("{SYSTEM_FIXTURES}: no entries map"))?;
    let mut table = HashMap::new();
    for row in entries.values() {
        let id = row["mysekaiFixtureId"]
            .as_i64()
            .ok_or_else(|| format!("{SYSTEM_FIXTURES}: row without mysekaiFixtureId"))?;
        let name = row["mysekaiSystemFixtureType"]
            .as_str()
            .ok_or_else(|| format!("{SYSTEM_FIXTURES}: row without mysekaiSystemFixtureType"))?;
        let kind = system_type_value(name)
            .ok_or_else(|| format!("{SYSTEM_FIXTURES}: unknown system fixture type {name}"))?;
        table.insert(id as i32, kind);
    }
    Ok(table)
}

impl WalkableSource<'_, '_> {
    /// The list as this host's cells, in the source's order; `Err` names
    /// the input that is missing or unresolved (the caller holds).
    pub(crate) fn cells(&mut self) -> Result<Vec<Cell>, String> {
        let result = self.build();
        if let Err(reason) = &result {
            if self.cache.reported.as_deref() != Some(reason.as_str()) {
                info!("[npc-walkable] the walkable list is not available: {reason}");
                self.cache.reported = Some(reason.clone());
            }
        }
        result
    }

    fn build(&mut self) -> Result<Vec<Cell>, String> {
        if self.cache.table.is_none() {
            let handle = self
                .cache
                .table_handle
                .get_or_insert_with(|| self.server.load::<JsonAsset>(SYSTEM_FIXTURES))
                .clone();
            match self.json.get(&handle) {
                Some(asset) => self.cache.table = Some(parse_table(&asset.0)),
                None => return Err("the system fixture table is loading".into()),
            }
        }
        let table = match self.cache.table.as_ref() {
            Some(Ok(table)) => table.clone(),
            Some(Err(reason)) => return Err(reason.clone()),
            None => return Err("the system fixture table is loading".into()),
        };
        let epoch = self.epoch.as_deref().map_or(0, |epoch| epoch.0);
        let face = self
            .face
            .as_deref()
            .ok_or("the objective face is not built")?;
        if !face.is_fresh(epoch) {
            return Err("the objective face belongs to another site generation".into());
        }
        let inputs = self
            .supply
            .current()
            .ok_or("the fixture scene inputs are not ready")?
            .clone();
        if inputs.stamp.site_epoch != epoch {
            return Err("the fixture scene inputs belong to another site generation".into());
        }
        let key = (
            epoch,
            inputs.stamp.input_revision,
            face.navigation_generation(),
            Arc::as_ptr(&inputs) as usize,
        );
        if self.cache.key != Some(key) {
            let cells = self.compute(&inputs, &table, face);
            self.cache.key = Some(key);
            self.cache.cells = Some(cells);
        }
        self.cache
            .cells
            .clone()
            .unwrap_or_else(|| Err("the walkable list was not built".into()))
    }

    fn compute(
        &self,
        inputs: &FixtureSceneInputs,
        table: &HashMap<i32, i32>,
        face: &ObjectiveFace,
    ) -> Result<Vec<Cell>, String> {
        let tables = self
            .tables
            .as_deref()
            .ok_or("the fixture masters are not installed")?;
        if !inputs.gaps.is_empty() {
            return Err(format!(
                "the floor snapshot has {} gap(s), first: {}",
                inputs.gaps.len(),
                inputs.gaps[0]
            ));
        }
        let floor = &inputs.floor;
        if floor.coverage() != OccupancyCoverage::Complete {
            return Err("the floor snapshot's occupancy is incomplete".into());
        }
        let rows = self.placements.occupancy_rows();
        let masters: HashMap<&str, i32> = self
            .identities
            .iter()
            .map(|identity| (identity.uid.as_str(), identity.master_id))
            .collect();
        let master_of = |uid: &str| -> Result<i32, String> {
            masters
                .get(uid)
                .copied()
                .ok_or_else(|| format!("placement {uid} has no live master identity"))
        };
        let row_of = |uid: &str| -> Result<&OccupancyRow, String> {
            rows.iter()
                .find(|row| row.uid == uid)
                .ok_or_else(|| format!("tile occupant {uid} has no placement"))
        };
        // This host's cell of a source cell.
        let host = |x: i8, z: i8| GridPosition::new((-i16::from(x) - 1) as i8, 0, z);
        let width = floor.layout.width;
        let depth = floor.layout.depth;
        let mut failure: Option<String> = None;
        let movable = law::movable_cells(width, depth, |x, z| {
            let cell = host(x, z);
            if !floor.contains_grid(cell) {
                return FloorTile::Absent;
            }
            let tile = (|| -> Result<FloorTile, String> {
                match floor.tile_at_grid(cell) {
                    FixtureTileKnowledge::Empty => Ok(FloorTile::Empty),
                    FixtureTileKnowledge::Occupied(uid) => {
                        let id = master_of(&uid)?;
                        let master = tables
                            .fixture_master(id)
                            .ok_or_else(|| format!("fixture master {id} is missing"))?;
                        let passable = if master.is_gate {
                            self.areas
                                .add_using_contains(row_of(&uid)?, cell.x, cell.z)?
                        } else {
                            master.handle_type == "block_transparent"
                        };
                        Ok(if passable {
                            FloorTile::Passable
                        } else {
                            FloorTile::Blocked
                        })
                    }
                    other => Err(format!("floor tile {cell:?} is {other:?}")),
                }
            })();
            tile.unwrap_or_else(|reason| {
                failure.get_or_insert(reason);
                FloorTile::Blocked
            })
        });
        if let Some(reason) = failure.take() {
            return Err(reason);
        }
        // Rug rows: their bounding box at height 0 when their grid height
        // covers it.
        let mut rug_cells: Vec<(i8, i8)> = Vec::new();
        for row in rows.iter().filter(|row| row.layout & layout_type::RUG != 0) {
            let id = master_of(&row.uid)?;
            let master = tables
                .fixture_master(id)
                .ok_or_else(|| format!("fixture master {id} is missing"))?;
            let low = row.center_y as i32;
            if !(low <= 0 && 0 < low + master.grid_size.y) {
                continue;
            }
            for x in row.min.x..=row.max.x {
                for z in row.min.z..=row.max.z {
                    rug_cells.push((x, z));
                }
            }
        }
        let rugs = law::placed_cells(width, depth, |x, z| {
            let cell = host(x, z);
            floor
                .contains_grid(cell)
                .then(|| rug_cells.contains(&(cell.x, cell.z)))
        });
        // System fixtures in source coordinates.
        let mut system = Vec::new();
        for row in &rows {
            let id = master_of(&row.uid)?;
            let Some(&kind) = table.get(&id) else {
                continue;
            };
            let source_x = |x: i8| (-i16::from(x) - 1) as i8;
            system.push(SystemFixture {
                min: GridPosition::new(source_x(row.max.x), row.min.y, row.min.z),
                max: GridPosition::new(source_x(row.min.x), row.max.y, row.max.z),
                grid_y: row.center_y,
                system_type: Some(kind),
            });
        }
        let excluded = law::excluded_cells(&system);
        let origin = floor.site_origin;
        let cells = law::walkable_cells(&movable, &rugs, &excluded, |cell| {
            let at = host(cell.x, cell.z);
            let corner = [
                at.x as f32 * moly_law::objective::TILE_SCALE + origin.x,
                origin.y,
                at.z as f32 * moly_law::objective::TILE_SCALE + origin.z,
            ];
            face.sample(corner, law::NAVMESH_SAMPLE_DISTANCE).is_some()
        });
        let host_cells: Vec<Cell> = cells
            .iter()
            .map(|cell| {
                let at = host(cell.x, cell.z);
                (at.x as i32, at.z as i32)
            })
            .collect();
        info!(
            "[npc-walkable] {}",
            serde_json::json!({
                "epoch": inputs.stamp.site_epoch,
                "revision": inputs.stamp.input_revision,
                "floor": [width, depth],
                "movable": movable.len(),
                "rugs": rugs.len(),
                "system": system.len(),
                "excluded": excluded.len(),
                "walkable": host_cells.len(),
            })
        );
        Ok(host_cells)
    }
}
