//! The game-side inputs of the layout rules: the current site's grid, the
//! fixture master predicates, the bundle meta and the unavailable zone table.
//!
//! The rules themselves live in `moly_layout`; this module only reads the
//! engine resources and the site's asset.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::asset::LoadState;
use moly_assets::json::JsonAsset;
use moly_layout::validate::Missing;
use moly_layout::{Rules, SiteGrid, Zone};

use super::{assets::FixtureAreas, put_effect};
use crate::site::FloorGridLayout;

const ZONES: &str = "moly://mysekai-site-housing-layout-unavailable-zones.json";

/// The zone table's state and the per-layout zones.
#[derive(Resource, Default)]
pub struct ZoneTable {
    handle: Option<Handle<JsonAsset>>,
    read: Option<Result<Arc<HashMap<u32, Vec<Zone>>>, String>>,
}

/// Test hook: a read zone table with no zones.
#[cfg(test)]
pub fn empty_zone_table() -> ZoneTable {
    ZoneTable {
        handle: None,
        read: Some(Ok(Arc::new(HashMap::new()))),
    }
}

pub fn request_zones(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ZoneTable {
        handle: Some(server.load(ZONES)),
        read: None,
    });
}

fn parse_zones(value: &serde_json::Value) -> Result<HashMap<u32, Vec<Zone>>, String> {
    let entries = value["entries"]
        .as_object()
        .ok_or("the unavailable zone table has no entries")?;
    let mut out: HashMap<u32, Vec<Zone>> = HashMap::new();
    let mut rows: Vec<&serde_json::Value> = entries.values().collect();
    rows.sort_by_key(|row| row["id"].as_i64());
    for row in rows {
        let int = |key: &str| {
            row[key]
                .as_i64()
                .ok_or_else(|| format!("unavailable zone row carries no integer {key}"))
        };
        let layout = u32::try_from(int("mysekaiSiteLayoutId")?)
            .map_err(|_| "unavailable zone site layout id out of range")?;
        out.entry(layout).or_default().push(Zone::from_row(
            int("id")?,
            [int("startX")?, int("startY")?, int("startZ")?],
            [int("width")?, int("height")?, int("depth")?],
        ));
    }
    Ok(out)
}

pub fn read_zones(
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut table: ResMut<ZoneTable>,
) {
    if table.read.is_some() {
        return;
    }
    let Some(handle) = table.handle.clone() else {
        return;
    };
    let read = match server.load_state(&handle) {
        LoadState::Loaded => json
            .get(&handle)
            .ok_or_else(|| format!("{ZONES}: loaded but not in the assets"))
            .and_then(|asset| {
                serde_json::from_str::<serde_json::Value>(&asset.0)
                    .map_err(|error| format!("{ZONES}: not JSON: {error}"))
            })
            .and_then(|value| parse_zones(&value)),
        LoadState::Failed(error) => Err(format!("{ZONES} failed to load: {error}")),
        _ => return,
    };
    match &read {
        Ok(zones) => info!(
            "[edit-put] unavailable zone table read: {} zones on {} site layouts",
            zones.values().map(Vec::len).sum::<usize>(),
            zones.len()
        ),
        Err(error) => {
            warn!("[edit-put] {error}: floor puts are refused until the table is in the root")
        }
    }
    table.read = Some(read.map(Arc::new));
    table.handle = None;
}

/// The unavailable zones of one floor layout, or `None` while the table is
/// not read (or failed to read).
pub fn zones_of(world: &World, layout_id: u32) -> Option<Vec<Zone>> {
    match world
        .get_resource::<ZoneTable>()
        .and_then(|table| table.read.as_ref())
    {
        Some(Ok(zones)) => Some(zones.get(&layout_id).cloned().unwrap_or_default()),
        _ => None,
    }
}

/// The rules for the current floor layout, or what is missing.
pub fn rules(world: &World, floor: Option<FloorGridLayout>) -> Result<Rules, Missing> {
    let floor = floor.ok_or(Missing::FloorGrid)?;
    let traits = put_effect::traits_table(world).ok_or(Missing::FixtureTable)?;
    let masters = put_effect::masters_table(world).ok_or(Missing::FixtureTable)?;
    let bundles = world
        .get_resource::<FixtureAreas>()
        .filter(|areas| areas.loaded)
        .map(|areas| areas.stack.clone())
        .ok_or(Missing::AreaTable)?;
    let zones = zones_of(world, floor.layout_id);
    Ok(Rules {
        traits: (*traits).clone(),
        masters: (*masters).clone(),
        bundles: (*bundles).clone(),
        zones,
        ..Rules::default()
    })
}
