//! The editor's fixture list from the owned fixtures.
//!
//! `SiteEditFixtureContentListView.ConvertUserFixturesToThumbnailData` walks
//! the user's `UserMysekaiFixture` rows in their order and keeps a row when
//! its placeable count is above 0 or `MysekaiFixtureUtility.IsSelectableFixture`
//! holds; the cell shows the placeable count. The placeable count is
//! `FixtureSiteLayoutedManager.GetFixturePlaceableCount(id, texture)`: the
//! row's quantity less the fixture's layouted count (the fixtures of that id
//! and texture placed on the sites). Placing or storing a fixture changes the
//! layouted count; the owned quantity stays.
//!
//! Here the layouted count is the draft of the edited site plus the other
//! sites' layouts ([`crate::fixture::layouts::SiteFixtureLayouts::placed_elsewhere`]).
//! The quantities are the client's copy of the server data
//! (`crate::server::client::inventory`).
//!
//! Named gaps: `IsSelectableFixture` (the home fixture and the wall and floor
//! appearance fixtures) is not applied, because this floor editor edits
//! neither; rows whose master layout is not floor or rug (walls, roads,
//! appearances: their own edit states) and rows whose model the fixture-model
//! index does not export with a fixture view are left out and counted in the
//! log; the list is not filtered by the selector's tab.

use std::collections::HashMap;

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;
use moly_law::fixture::Vector3Int;
use moly_law::fixture::position::layout_type;
use serde_json::Value;

use super::assets::CandidateAssets;
use crate::fixture::EditableFixture;
use crate::server::client::inventory::ClientMysekaiInventory;

const FIXTURES: &str = "moly://mysekai-fixtures.json";

/// A master row as the list reads it.
struct MasterRow {
    package: String,
    grid_size: Vector3Int,
    /// The layout type bit of a floor or rug fixture; `None` for the other
    /// settable layouts.
    layout: Option<u8>,
}

/// The fixture master rows by id, or why the master could not be read.
#[derive(Resource)]
pub(super) struct OwnedMaster(Result<HashMap<i32, MasterRow>, String>);

#[derive(Resource)]
pub(super) struct OwnedMasterRequest(Handle<JsonAsset>);

pub(super) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(OwnedMasterRequest(server.load(FIXTURES)));
}

fn parse_master(value: &Value) -> Result<HashMap<i32, MasterRow>, String> {
    let rows = value["fixtures"]
        .as_array()
        .ok_or("the fixture master has no fixtures array")?;
    rows.iter()
        .map(|row| {
            let id = row["id"]
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or("a fixture master row has no 32-bit id")?;
            let int = |field: &str| {
                row[field]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| format!("fixture master row {id} has no {field}"))
            };
            let bundle = row["assetbundleName"]
                .as_str()
                .ok_or_else(|| format!("fixture master row {id} has no assetbundleName"))?;
            let layout = match row["layoutType"].as_str() {
                Some("floor") => Some(layout_type::FLOOR),
                Some("rug") => Some(layout_type::RUG),
                Some(_) => None,
                None => return Err(format!("fixture master row {id} has no layoutType")),
            };
            Ok((
                id,
                MasterRow {
                    package: format!("mysekai__fixture__{bundle}"),
                    grid_size: Vector3Int::new(
                        int("gridWidth")?,
                        int("gridHeight")?,
                        int("gridDepth")?,
                    ),
                    layout,
                },
            ))
        })
        .collect()
}

pub(super) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    request: Option<Res<OwnedMasterRequest>>,
) {
    let Some(request) = request else {
        return;
    };
    let parsed = match server.load_state(&request.0) {
        LoadState::Failed(error) => Err(format!("{FIXTURES} failed to load: {error:?}")),
        _ => match json.get(&request.0) {
            None => return,
            Some(asset) => serde_json::from_str::<Value>(&asset.0)
                .map_err(|error| format!("{FIXTURES}: {error}"))
                .and_then(|value| parse_master(&value)),
        },
    };
    if let Err(error) = &parsed {
        error!("[edit-owned] {error}; the editor lists no owned fixtures");
    }
    commands.insert_resource(OwnedMaster(parsed));
    commands.remove_resource::<OwnedMasterRequest>();
}

/// One listed owned row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OwnedRow {
    pub fixture_id: i32,
    pub texture_id: u32,
    pub package: String,
    pub grid_size: Vector3Int,
    pub layout: u8,
    pub quantity: i32,
    /// The layouted count: the edited site's draft and the other sites.
    pub layouted: i32,
}

impl OwnedRow {
    /// `GetFixturePlaceableCount(id, texture)`.
    pub fn placeable(&self) -> i32 {
        self.quantity - self.layouted
    }

    /// A new placement of this row (its UID given by the editor).
    pub fn fixture(&self, uid: String) -> EditableFixture {
        EditableFixture {
            uid,
            package: self.package.clone(),
            texture_id: self.texture_id,
            fixture_id: self.fixture_id,
            center: moly_law::fixture::GridPosition::ZERO,
            grid_size: self.grid_size,
            layout: self.layout,
            direction: moly_law::fixture::Direction::Front,
        }
    }
}

/// What the list could not include, for the log.
#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Omitted {
    pub no_master: usize,
    pub other_layout: usize,
    pub no_model: usize,
    pub none_placeable: usize,
}

/// The owned rows the editor lists for the edited site, in the copy's row
/// order: those with a placeable count above 0.
pub(super) fn catalog(
    world: &World,
    site_id: u32,
    draft: &[EditableFixture],
) -> Result<(Vec<OwnedRow>, Omitted), String> {
    let owned = world
        .get_resource::<ClientMysekaiInventory>()
        .ok_or("the server model keeps no inventory copy in this build")?;
    let master = match world.get_resource::<OwnedMaster>() {
        None => return Err("the fixture master is still loading".into()),
        Some(OwnedMaster(Err(error))) => return Err(error.clone()),
        Some(OwnedMaster(Ok(master))) => master,
    };
    let paths = &world
        .get_resource::<CandidateAssets>()
        .ok_or("the fixture-model index is not requested")?
        .paths;
    let elsewhere = world
        .get_resource::<crate::fixture::layouts::SiteFixtureLayouts>()
        .ok_or("the layout owner is not installed")?
        .placed_elsewhere(site_id)?;
    let mut omitted = Omitted::default();
    let mut rows = Vec::new();
    for row in owned.fixtures() {
        let Some(entry) = master.get(&row.mysekai_fixture_id) else {
            omitted.no_master += 1;
            continue;
        };
        let Some(layout) = entry.layout else {
            omitted.other_layout += 1;
            continue;
        };
        if !paths.contains_key(&entry.package) {
            omitted.no_model += 1;
            continue;
        }
        let Ok(texture_id) = u32::try_from(row.texture_id) else {
            omitted.no_master += 1;
            continue;
        };
        let key = (row.mysekai_fixture_id, texture_id);
        let here = draft
            .iter()
            .filter(|item| item.fixture_id == key.0 && item.texture_id == key.1)
            .count() as i32;
        let listed = OwnedRow {
            fixture_id: row.mysekai_fixture_id,
            texture_id,
            package: entry.package.clone(),
            grid_size: entry.grid_size,
            layout,
            quantity: row.quantity,
            layouted: here + elsewhere.get(&key).copied().unwrap_or(0),
        };
        if listed.placeable() > 0 {
            rows.push(listed);
        } else {
            omitted.none_placeable += 1;
        }
    }
    Ok((rows, omitted))
}
