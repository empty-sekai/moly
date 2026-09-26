//! `CanCollectResource`: whether the inventory takes a drop, and the user
//! possession values it reads.
//!
//! The source (`HarvestUserDataManager.CanCollectResource`, a switch on the
//! resource type): a fixture (39) goes to `CanCollectFixture`, a Mysekai
//! material (41) to `CanCollectMaterial`; a blueprint (40), an item (42), a
//! music record (44) and a material (2) are always taken; a tool (43) and
//! every other type never.
//! - `CanCollectMaterial(id, qty)`: a material of type game_character or
//!   birthday_party is taken over the limit
//!   (`IsMaterialReceivableOverPossessionLimit`; an id without a master row
//!   logs an error and is not). Otherwise the quantities of every queued
//!   gather stack (whatever it gathered), plus `qty`, plus
//!   `userMysekaiMaterialPossession.quantity` must be at most the
//!   `possessionLimit` of the `mysekaiMaterialPossessions` row whose level is
//!   `userMysekaiGamedata.mysekaiMaterialPossessionLevel` (no such row: the
//!   source throws; here the drop is refused with an error line).
//! - `CanCollectFixture(qty)`: the same sum with the quantities of the user
//!   fixture rows (`GetFixtureTotalCount`) against the
//!   `mysekaiFixturePossessions` row of `mysekaiFixturePossessionLevel`.
//!
//! The limit tables are master data (both documents are inputs here); a
//! document that fails to load is named once and every material and fixture
//! drop is refused (no default limit).
//!
//! Server values: `PossessionMock`, the named panel. Its values are the
//! mock's own choices, not the server's: both possession levels 1 (the
//! lowest master level); `userMysekaiMaterialPossession.quantity` 0 at login,
//! then each gather reply adds the gathered quantity of every Mysekai
//! material that is not taken over the limit (the server's aggregation is
//! not on disk); no user fixture rows at login, then each gather reply adds
//! the gathered fixtures. `MOLY_HARVEST_MOCK_MATERIAL_POSSESSION` seats the
//! login's material quantity (an unparsable value is refused loudly and the
//! mock's 0 stands).

use std::collections::BTreeMap;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

use super::catalog::HarvestCatalog;
use super::queue::Stack;
use super::server_mock::UserDrop;
use super::{
    RT_MATERIAL, RT_MYSEKAI_BLUEPRINT, RT_MYSEKAI_FIXTURE, RT_MYSEKAI_ITEM, RT_MYSEKAI_MATERIAL,
    RT_MYSEKAI_MUSIC_RECORD,
};

/// `MysekaiMaterialType.game_character` and `birthday_party`.
const MATERIAL_TYPE_GAME_CHARACTER: i32 = 4;
const MATERIAL_TYPE_BIRTHDAY_PARTY: i32 = 7;

const LIMIT_INPUTS: [(&str, &str); 2] = [
    (
        "mysekai-material-possessions.json",
        "mysekaiMaterialPossessions",
    ),
    (
        "mysekai-fixture-possessions.json",
        "mysekaiFixturePossessions",
    ),
];

/// The two limit documents while they load.
#[derive(Resource)]
pub(crate) struct PossessionInputs(Vec<(&'static str, &'static str, Handle<JsonAsset>)>);

/// `possessionLimit` by `level`, for materials and fixtures.
#[derive(Resource, Debug, Default)]
pub(crate) struct PossessionLimits {
    pub(crate) material: BTreeMap<i32, i64>,
    pub(crate) fixture: BTreeMap<i32, i64>,
}

/// A limit document failed to load (named once; material and fixture drops
/// are refused).
#[derive(Resource)]
pub(crate) struct PossessionLimitsAbsent(pub(crate) String);

/// `PossessionMock`: the user possession values the capacity check reads
/// (see the module comment for the mock's choices).
#[derive(Resource, Debug, Clone)]
pub(crate) struct PossessionMock {
    /// `userMysekaiGamedata.mysekaiMaterialPossessionLevel`.
    pub(crate) material_level: i32,
    /// `userMysekaiGamedata.mysekaiFixturePossessionLevel`.
    pub(crate) fixture_level: i32,
    /// `userMysekaiMaterialPossession.quantity`.
    pub(crate) material_quantity: i64,
    /// `userMysekaiFixtures` quantities by fixture id.
    pub(crate) fixtures: BTreeMap<i64, i64>,
}

impl Default for PossessionMock {
    fn default() -> Self {
        let mut material_quantity = 0;
        if let Ok(raw) = std::env::var("MOLY_HARVEST_MOCK_MATERIAL_POSSESSION") {
            match raw.trim().parse::<i64>() {
                Ok(value) if value >= 0 => material_quantity = value,
                _ => error!(
                    "[harvest-mock] MOLY_HARVEST_MOCK_MATERIAL_POSSESSION={raw:?} is not a quantity; the mock's 0 stands"
                ),
            }
        }
        Self {
            material_level: 1,
            fixture_level: 1,
            material_quantity,
            fixtures: BTreeMap::new(),
        }
    }
}

impl PossessionMock {
    /// `GatherApiMock`'s possession side (the mock's rule): the gathered
    /// Mysekai materials not taken over the limit add to the material
    /// quantity, the gathered fixtures to the fixture rows.
    pub(crate) fn on_gather(&mut self, drops: &[UserDrop], catalog: &HarvestCatalog) {
        for drop in drops {
            match drop.resource_type {
                RT_MYSEKAI_MATERIAL => {
                    let over = catalog.materials.get(&drop.resource_id).is_some_and(|row| {
                        row.material_type == MATERIAL_TYPE_GAME_CHARACTER
                            || row.material_type == MATERIAL_TYPE_BIRTHDAY_PARTY
                    });
                    if !over {
                        self.material_quantity += drop.quantity as i64;
                    }
                }
                RT_MYSEKAI_FIXTURE => {
                    *self.fixtures.entry(drop.resource_id).or_default() += drop.quantity as i64;
                }
                _ => {}
            }
        }
    }
}

/// Startup: request both limit documents.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(PossessionInputs(
        LIMIT_INPUTS
            .iter()
            .map(|(path, table)| {
                (
                    *path,
                    *table,
                    server.load(bevy::asset::AssetPath::from(format!("moly://{path}"))),
                )
            })
            .collect(),
    ));
}

/// `{level: possessionLimit}` of one keyed possession master.
fn limits(
    value: &serde_json::Value,
    path: &str,
    table: &str,
) -> Result<BTreeMap<i32, i64>, String> {
    if value["version"].as_u64() != Some(1) {
        return Err(format!("{path}: unsupported master version"));
    }
    if value["semantics"]["table"].as_str() != Some(table) {
        return Err(format!("{path}: table identity is not {table}"));
    }
    let entries = value["entries"]
        .as_object()
        .ok_or_else(|| format!("{path}: no entries"))?;
    let mut out = BTreeMap::new();
    for row in entries.values() {
        let (Some(level), Some(limit)) = (row["level"].as_i64(), row["possessionLimit"].as_i64())
        else {
            return Err(format!("{path}: a row without level or possessionLimit"));
        };
        // FirstOrDefault by level: the first row of a level in master order.
        let id = row["id"].as_i64().unwrap_or(i64::MAX);
        out.entry(level as i32)
            .and_modify(|entry: &mut (i64, i64)| {
                if id < entry.0 {
                    *entry = (id, limit);
                }
            })
            .or_insert((id, limit));
    }
    Ok(out
        .into_iter()
        .map(|(level, (_, limit))| (level, limit))
        .collect())
}

/// Update: once both documents are in, seat the limit tables.
pub(crate) fn build(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    inputs: Option<Res<PossessionInputs>>,
) {
    let Some(inputs) = inputs else {
        return;
    };
    let mut tables = Vec::new();
    for (path, table, handle) in &inputs.0 {
        match server.load_state(handle) {
            LoadState::Failed(error) => {
                let line = format!("{path} ({table}): {error}");
                error!("[harvest-pickup] possession limit input absent, material and fixture drops are refused: {line}");
                commands.insert_resource(PossessionLimitsAbsent(line));
                commands.remove_resource::<PossessionInputs>();
                return;
            }
            LoadState::Loaded => {}
            _ => return,
        }
        let Some(asset) = json.get(handle) else {
            return;
        };
        let parsed = serde_json::from_str::<serde_json::Value>(&asset.0)
            .map_err(|error| format!("{path}: not JSON: {error}"))
            .and_then(|value| limits(&value, path, table));
        match parsed {
            Ok(rows) => tables.push(rows),
            Err(line) => {
                error!("[harvest-pickup] possession limit input refused, material and fixture drops are refused: {line}");
                commands.insert_resource(PossessionLimitsAbsent(line));
                commands.remove_resource::<PossessionInputs>();
                return;
            }
        }
    }
    let [material, fixture]: [BTreeMap<i32, i64>; 2] =
        tables.try_into().expect("two limit documents");
    info!("[harvest-pickup] possession limits: materials {material:?}, fixtures {fixture:?}");
    commands.insert_resource(PossessionLimits { material, fixture });
    commands.remove_resource::<PossessionInputs>();
}

/// What the capacity check reads.
pub(crate) struct Capacity<'a> {
    pub(crate) stacks: &'a [Stack],
    pub(crate) catalog: &'a HarvestCatalog,
    pub(crate) limits: Option<&'a PossessionLimits>,
    pub(crate) mock: &'a PossessionMock,
}

impl Capacity<'_> {
    /// The quantities of every queued gather stack.
    fn queued(&self) -> i64 {
        self.stacks
            .iter()
            .map(|stack| match stack {
                Stack::Gather { drop, .. } => drop.quantity as i64,
                Stack::Harvest(_) => 0,
            })
            .sum()
    }

    /// `CanCollectResource(resourceId, resourceType, quantity)`.
    pub(crate) fn can_collect(&self, resource_type: i32, resource_id: i64, quantity: i32) -> bool {
        match resource_type {
            RT_MYSEKAI_FIXTURE => self.can_collect_fixture(quantity),
            RT_MYSEKAI_MATERIAL => self.can_collect_material(resource_id, quantity),
            RT_MYSEKAI_BLUEPRINT | RT_MYSEKAI_ITEM | RT_MYSEKAI_MUSIC_RECORD | RT_MATERIAL => true,
            _ => false,
        }
    }

    /// `IsMaterialReceivableOverPossessionLimit`.
    fn over_limit_receivable(&self, resource_id: i64) -> bool {
        match self.catalog.materials.get(&resource_id) {
            Some(row) => {
                row.material_type == MATERIAL_TYPE_GAME_CHARACTER
                    || row.material_type == MATERIAL_TYPE_BIRTHDAY_PARTY
            }
            None => {
                error!("[harvest-pickup] MasterMysekaiMaterial is absent: id {resource_id}");
                false
            }
        }
    }

    /// `CanCollectMaterial(resourceId, quantity)`.
    pub(crate) fn can_collect_material(&self, resource_id: i64, quantity: i32) -> bool {
        if self.over_limit_receivable(resource_id) {
            return true;
        }
        let Some(limits) = self.limits else {
            error!("[harvest-pickup] CanCollectMaterial: the material possession limits are absent; refused");
            return false;
        };
        let Some(&limit) = limits.material.get(&self.mock.material_level) else {
            error!(
                "[harvest-pickup] CanCollectMaterial: no mysekaiMaterialPossessions row of level {}; refused",
                self.mock.material_level
            );
            return false;
        };
        self.queued() + quantity as i64 + self.mock.material_quantity <= limit
    }

    /// `CanCollectFixture(quantity)`.
    pub(crate) fn can_collect_fixture(&self, quantity: i32) -> bool {
        let Some(limits) = self.limits else {
            error!("[harvest-pickup] CanCollectFixture: the fixture possession limits are absent; refused");
            return false;
        };
        let Some(&limit) = limits.fixture.get(&self.mock.fixture_level) else {
            error!(
                "[harvest-pickup] CanCollectFixture: no mysekaiFixturePossessions row of level {}; refused",
                self.mock.fixture_level
            );
            return false;
        };
        let total: i64 = self.mock.fixtures.values().sum();
        self.queued() + quantity as i64 + total <= limit
    }
}
