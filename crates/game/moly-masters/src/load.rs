//! The five typed master tables and the one loader that fills them.
//!
//! Each table is a [`MasterTable`] requested from the asset layer; every
//! consumer in the process shares the request and the parsed value. A
//! resource appears once every table it is built from has resolved (a table
//! the layer cannot give leaves its own fields empty and is named in
//! [`MasterTables::missing`]).
//!
//! The assembly from a table's text to the resource field is here, so the
//! server and the client cannot disagree about a column name.

use std::collections::HashSet;

use bevy::prelude::*;
use moly_assets::json::master::{MasterData, MasterTable};
use serde_json::Value;

use crate::avatar::AvatarMasters;
use crate::craft::CraftMasters;
use crate::delivery::{BonusRow, DeliveryTables, RewardRow, TotalRow};
use crate::inventory::PossessionMasters;
use crate::StaminaMax;

/// The five master resources one loader fills, plus the row slices a client
/// reader parses for itself (the harvest drop classifier's ids and material
/// rows, from the same three tables).
#[derive(Resource, Default)]
pub struct MasterTables {
    /// `mysekaiBlueprints` and the five craft tables, if any are in.
    pub craft: Option<CraftMasters>,
    /// `mysekaiFixturePossessions` and `mysekaiMaterialPossessions`.
    pub possession: Option<PossessionMasters>,
    /// `avatarCostumes`, `avatarAccessories`, `avatarSkinColors`,
    /// `avatarCoordinates`.
    pub avatar: Option<AvatarMasters>,
    /// The three `birthdayPartyDelivery*` tables.
    pub delivery: Option<DeliveryTables>,
    /// `mysekaiStaminas`.
    pub stamina_max: Option<StaminaMax>,
    /// The blueprint ids the harvest drop classifier reads, from the same
    /// table the craft reader takes (no second request for it).
    pub blueprint_ids: Option<HashSet<i64>>,
    /// The item ids the harvest drop classifier reads, from the same table
    /// the craft reader takes.
    pub item_ids: Option<HashSet<i64>>,
    /// `mysekaiMaterials`: id, `mysekaiMaterialType` and `mysekaiSiteIds`,
    /// for the harvest drop classifier (the craft reader takes the types).
    pub material_rows: Option<Vec<(i64, String, Vec<u32>)>>,
    /// Tables still loading, by the names their consumers read.
    pub pending: Vec<&'static str>,
    /// Tables the master layer could not give.
    pub missing: Vec<String>,
    started: bool,
    parts: Parts,
    craft_left: usize,
    possession_left: usize,
    avatar_left: usize,
    delivery_left: usize,
    stamina_left: usize,
}

impl MasterTables {
    /// Every table requested, and each one taken (a table the layer could
    /// not give counts as taken: it is named in [`Self::missing`]).
    pub fn ready(&self) -> bool {
        self.started && self.pending.is_empty()
    }

    /// A small synthetic table set a harness seats directly: synthetic pool
    /// maxima and empty table resources; the harness fills what it drives.
    pub fn fixture() -> Self {
        Self {
            craft: Some(CraftMasters::default()),
            possession: Some(PossessionMasters::default()),
            avatar: Some(AvatarMasters::default()),
            delivery: None,
            stamina_max: Some(StaminaMax {
                normal: 1000,
                enhance: 1000,
                boost: 10000,
            }),
            blueprint_ids: Some(HashSet::new()),
            item_ids: Some(HashSet::new()),
            material_rows: Some(Vec::new()),
            started: true,
            ..Self::default()
        }
    }

    fn settle(&mut self, group: Group, name: &'static str, commands: &mut Commands) {
        self.pending.retain(|known| *known != name);
        let left = match group {
            Group::Craft => &mut self.craft_left,
            Group::Possession => &mut self.possession_left,
            Group::Avatar => &mut self.avatar_left,
            Group::Delivery => &mut self.delivery_left,
            Group::Stamina => &mut self.stamina_left,
        };
        *left = left.saturating_sub(1);
        if *left > 0 {
            return;
        }
        match group {
            Group::Craft => {
                let value = std::mem::take(&mut self.parts.craft);
                commands.insert_resource(value.clone());
                self.craft = Some(value);
            }
            Group::Possession => {
                let value = std::mem::take(&mut self.parts.possession);
                commands.insert_resource(value.clone());
                self.possession = Some(value);
            }
            Group::Avatar => {
                let value = std::mem::take(&mut self.parts.avatar);
                commands.insert_resource(value.clone());
                self.avatar = Some(value);
            }
            Group::Delivery => {
                if let Some(value) = self.parts.delivery.complete() {
                    commands.insert_resource(value.clone());
                    self.delivery = Some(value);
                }
            }
            Group::Stamina => {
                if let Some(value) = self.parts.stamina_max.take() {
                    commands.insert_resource(value);
                    self.stamina_max = Some(value);
                }
            }
        }
    }
}

/// The tables one resource is built from, and how many are still out.
#[derive(Clone, Copy)]
enum Group {
    Craft,
    Possession,
    Avatar,
    Delivery,
    Stamina,
}

const CRAFT_TABLES: usize = 6;
const POSSESSION_TABLES: usize = 2;
const AVATAR_TABLES: usize = 4;
const DELIVERY_TABLES: usize = 3;
const STAMINA_TABLES: usize = 1;

/// One table's parse result, before it is seated into its resource.
#[derive(Default)]
struct Parts {
    craft: CraftMasters,
    possession: PossessionMasters,
    avatar: AvatarMasters,
    delivery: DeliveryParts,
    stamina_max: Option<StaminaMax>,
    blueprint_ids: Option<HashSet<i64>>,
    item_ids: Option<HashSet<i64>>,
    material_rows: Option<Vec<(i64, String, Vec<u32>)>>,
}

/// One master table the loader requests and where its rows go.
struct Entry {
    table: MasterTable<Parts>,
    /// Moves the one field a table fills out of the parse result.
    seat: fn(&mut MasterTables, Parts),
    group: Group,
}

macro_rules! table {
    ($table:literal, $name:literal, $parse:path, $resource:ident, $field:ident, $group:ident) => {
        Entry {
            table: MasterTable {
                table: $table,
                name: $name,
                parse: |text| {
                    let mut parts = Parts::default();
                    $parse(text, &mut parts.$resource)?;
                    Ok(parts)
                },
            },
            seat: |tables, partial| {
                tables.parts.$resource.$field = partial.$resource.$field;
            },
            group: Group::$group,
        }
    };
}

macro_rules! root_table {
    ($table:literal, $name:literal, $parse:path, $field:ident, $group:ident) => {
        Entry {
            table: MasterTable {
                table: $table,
                name: $name,
                parse: |text| {
                    let mut parts = Parts::default();
                    $parse(text, &mut parts.$field)?;
                    Ok(parts)
                },
            },
            seat: |tables, partial| tables.parts.$field = partial.$field,
            group: Group::$group,
        }
    };
}

/// The five resources' tables, in request order.
static TABLES: &[Entry] = &[
    root_table!(
        "mysekaiStaminas",
        "mysekaiStaminas (the server's pool maxima)",
        parse_staminas,
        stamina_max,
        Stamina
    ),
    Entry {
        table: MasterTable {
            table: "mysekaiBlueprints",
            name: "mysekaiBlueprints (the server's blueprints)",
            parse: |text| {
                let mut parts = Parts::default();
                parse_blueprints(text, &mut parts.craft)?;
                parts.blueprint_ids = table_rows(text, "mysekaiBlueprints")
                    .and_then(|rows| row_ids(&rows))
                    .ok();
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.craft.blueprints = partial.craft.blueprints;
            tables.blueprint_ids = partial.blueprint_ids;
        },
        group: Group::Craft,
    },
    table!(
        "mysekaiBlueprintMysekaiMaterialCosts",
        "mysekaiBlueprintMysekaiMaterialCosts (the server's blueprint material costs)",
        parse_costs,
        craft,
        costs,
        Craft
    ),
    table!(
        "mysekaiBlueprintTerms",
        "mysekaiBlueprintTerms (the server's blueprint craft terms)",
        parse_terms,
        craft,
        terms,
        Craft
    ),
    table!(
        "mysekaiRankObtainedExps",
        "mysekaiRankObtainedExps (the server's first-craft bonus)",
        parse_rank_obtained_exps,
        craft,
        first_craft_bonus,
        Craft
    ),
    Entry {
        table: MasterTable {
            table: "mysekaiMaterials",
            name: "mysekaiMaterials (the server's material types)",
            parse: |text| {
                let mut parts = Parts::default();
                parse_material_types(text, &mut parts.craft)?;
                parts.material_rows = table_rows(text, "mysekaiMaterials")
                    .and_then(|rows| material_rows(&rows))
                    .ok();
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.craft.material_types = partial.craft.material_types;
            tables.material_rows = partial.material_rows;
        },
        group: Group::Craft,
    },
    Entry {
        table: MasterTable {
            table: "mysekaiItems",
            name: "mysekaiItems (the server's white blueprint item)",
            parse: |text| {
                let mut parts = Parts::default();
                parse_white_blueprint_item(text, &mut parts.craft)?;
                parts.item_ids = table_rows(text, "mysekaiItems")
                    .and_then(|rows| row_ids(&rows))
                    .ok();
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.craft.white_blueprint_item = partial.craft.white_blueprint_item;
            tables.item_ids = partial.item_ids;
        },
        group: Group::Craft,
    },
    table!(
        "avatarCostumes",
        "avatarCostumes (the server's avatar costumes)",
        parse_costumes,
        avatar,
        costumes,
        Avatar
    ),
    table!(
        "avatarAccessories",
        "avatarAccessories (the server's avatar accessories)",
        parse_accessories,
        avatar,
        accessories,
        Avatar
    ),
    table!(
        "avatarSkinColors",
        "avatarSkinColors (the server's avatar skin colours)",
        parse_skin_colors,
        avatar,
        skin_colors,
        Avatar
    ),
    table!(
        "avatarCoordinates",
        "avatarCoordinates (the server's avatar coordinates)",
        parse_coordinates,
        avatar,
        coordinates,
        Avatar
    ),
    table!(
        "mysekaiFixturePossessions",
        "mysekaiFixturePossessions (the server's fixture possession limits)",
        parse_fixture_possessions,
        possession,
        fixture,
        Possession
    ),
    table!(
        "mysekaiMaterialPossessions",
        "mysekaiMaterialPossessions (the server's material possession limits)",
        parse_material_possessions,
        possession,
        material,
        Possession
    ),
    table!(
        "birthdayPartyDeliveryRewards",
        "birthdayPartyDeliveryRewards (the delivery's reward rows)",
        parse_rewards,
        delivery,
        rewards,
        Delivery
    ),
    table!(
        "birthdayPartyDeliveryPointBonuses",
        "birthdayPartyDeliveryPointBonuses (the delivery's point bonus rows)",
        parse_point_bonuses,
        delivery,
        bonuses,
        Delivery
    ),
    table!(
        "birthdayPartyDeliveryTotalRewards",
        "birthdayPartyDeliveryTotalRewards (the delivery's total reward rows)",
        parse_total_rewards,
        delivery,
        totals,
        Delivery
    ),
];

/// Requests the five resources' tables once, and starts their counters.
fn load(mut data: ResMut<MasterData>, mut tables: ResMut<MasterTables>) {
    if tables.started {
        return;
    }
    tables.started = true;
    tables.pending = TABLES.iter().map(|entry| entry.table.name).collect();
    tables.craft_left = CRAFT_TABLES;
    tables.possession_left = POSSESSION_TABLES;
    tables.avatar_left = AVATAR_TABLES;
    tables.delivery_left = DELIVERY_TABLES;
    tables.stamina_left = STAMINA_TABLES;
    for entry in TABLES {
        data.request(&entry.table);
    }
}

/// Takes each table as it resolves, seats its rows and inserts the resource
/// once its group is complete.
pub fn resolve_tables(
    mut commands: Commands,
    mut data: ResMut<MasterData>,
    mut tables: ResMut<MasterTables>,
) {
    if !tables.started {
        return;
    }
    for entry in TABLES {
        if !tables.pending.contains(&entry.table.name) {
            continue;
        }
        let Some(result) = data.take(&entry.table) else {
            continue;
        };
        match result {
            Ok(partial) => (entry.seat)(&mut tables, partial),
            Err(error) => tables.missing.push(error.to_string()),
        }
        tables.settle(entry.group, entry.table.name, &mut commands);
    }
}

/// The five resources' loader.
pub struct MasterTablesPlugin;

impl Plugin for MasterTablesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterTables>()
            .add_systems(Startup, load)
            .add_systems(PreUpdate, resolve_tables.after(moly_assets::json::master::resolve));
    }
}

// ---------------------------------------------------------------------------
// The assembly
// ---------------------------------------------------------------------------

/// The row ids a drop classifier reads (the same set a table's id column
/// names).
fn row_ids(rows: &[Value]) -> Result<HashSet<i64>, String> {
    rows.iter()
        .map(|row| {
            row["id"]
                .as_i64()
                .ok_or_else(|| format!("id of {row} is not an integer"))
        })
        .collect()
}

/// `mysekaiMaterials`: id, `mysekaiMaterialType` and `mysekaiSiteIds`.
fn material_rows(rows: &[Value]) -> Result<Vec<(i64, String, Vec<u32>)>, String> {
    rows.iter()
        .map(|row| {
            let id = row["id"]
                .as_i64()
                .ok_or_else(|| format!("id of {row} is not an integer"))?;
            let word = row["mysekaiMaterialType"]
                .as_str()
                .ok_or_else(|| format!("mysekaiMaterialType of {row} is not a string"))?
                .to_owned();
            let sites = row["mysekaiSiteIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|site| site.as_u64().map(|site| site as u32))
                .collect();
            Ok((id, word, sites))
        })
        .collect()
}

/// An upstream row table, sorted by `id` (a table without ids keeps its
/// master order).
fn table_rows(text: &str, table: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut rows = value
        .as_array()
        .ok_or_else(|| format!("{table} is not an array of rows"))?
        .clone();
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    Ok(rows)
}

/// `mysekaiStaminas`: the normal, enhance and boost rows' `maxStamina`.
pub fn parse_staminas(text: &str, max: &mut Option<StaminaMax>) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiStaminas")?;
    let pool = |kind: &str| -> Result<i32, String> {
        rows.iter()
            .find(|row| row["mysekaiStaminaType"].as_str() == Some(kind))
            .ok_or_else(|| format!("no {kind} stamina row"))
            .and_then(|row| crate::int32(row, "maxStamina"))
    };
    *max = Some(StaminaMax {
        normal: pool("normal")?,
        enhance: pool("enhance")?,
        boost: pool("boost")?,
    });
    Ok(())
}

/// `mysekaiBlueprints`.
pub fn parse_blueprints(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.blueprints = Some(crate::craft::parse_blueprints(&table_rows(
        text,
        "mysekaiBlueprints",
    )?)?);
    Ok(())
}

/// `mysekaiBlueprintMysekaiMaterialCosts`: the rows per blueprint.
pub fn parse_costs(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.costs = Some(crate::craft::parse_costs(&table_rows(
        text,
        "mysekaiBlueprintMysekaiMaterialCosts",
    )?)?);
    Ok(())
}

/// `mysekaiBlueprintTerms`.
pub fn parse_terms(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.terms = Some(crate::craft::parse_terms(&table_rows(
        text,
        "mysekaiBlueprintTerms",
    )?)?);
    Ok(())
}

/// `mysekaiRankObtainedExps`: the first-craft bonus row.
pub fn parse_rank_obtained_exps(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.first_craft_bonus = Some(crate::craft::parse_rank_obtained_exps(&table_rows(
        text,
        "mysekaiRankObtainedExps",
    )?)?);
    Ok(())
}

/// `mysekaiMaterials`: material id -> `mysekaiMaterialType`.
pub fn parse_material_types(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.material_types = Some(crate::craft::parse_material_types(&table_rows(
        text,
        "mysekaiMaterials",
    )?)?);
    Ok(())
}

/// `mysekaiItems`: the first `white_blueprint` item.
pub fn parse_white_blueprint_item(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.white_blueprint_item = Some(crate::craft::parse_white_blueprint_item(&table_rows(
        text,
        "mysekaiItems",
    )?)?);
    Ok(())
}

/// `avatarCostumes`.
pub fn parse_costumes(text: &str, avatar: &mut AvatarMasters) -> Result<(), String> {
    avatar.costumes = Some(crate::avatar::parse_costumes(&table_rows(
        text,
        "avatarCostumes",
    )?)?);
    Ok(())
}

/// `avatarAccessories`.
pub fn parse_accessories(text: &str, avatar: &mut AvatarMasters) -> Result<(), String> {
    avatar.accessories = Some(crate::avatar::parse_accessories(&table_rows(
        text,
        "avatarAccessories",
    )?)?);
    Ok(())
}

/// `avatarSkinColors`.
pub fn parse_skin_colors(text: &str, avatar: &mut AvatarMasters) -> Result<(), String> {
    avatar.skin_colors = Some(crate::avatar::parse_skin_colors(&table_rows(
        text,
        "avatarSkinColors",
    )?)?);
    Ok(())
}

/// `avatarCoordinates`.
pub fn parse_coordinates(text: &str, avatar: &mut AvatarMasters) -> Result<(), String> {
    avatar.coordinates = Some(crate::avatar::parse_coordinates(&table_rows(
        text,
        "avatarCoordinates",
    )?)?);
    Ok(())
}

/// `mysekaiFixturePossessions`.
pub fn parse_fixture_possessions(
    text: &str,
    possession: &mut PossessionMasters,
) -> Result<(), String> {
    possession.fixture = Some(crate::inventory::parse_possession_rows(&table_rows(
        text,
        "mysekaiFixturePossessions",
    )?)?);
    Ok(())
}

/// `mysekaiMaterialPossessions`.
pub fn parse_material_possessions(
    text: &str,
    possession: &mut PossessionMasters,
) -> Result<(), String> {
    possession.material = Some(crate::inventory::parse_possession_rows(&table_rows(
        text,
        "mysekaiMaterialPossessions",
    )?)?);
    Ok(())
}

/// `birthdayPartyDeliveryRewards`, master order.
pub fn parse_rewards(text: &str, parts: &mut DeliveryParts) -> Result<(), String> {
    parts.rewards = Some(crate::delivery::parse_rewards(&moly_assets::json::master::rows(
        text,
    )?)?);
    Ok(())
}

/// `birthdayPartyDeliveryPointBonuses`, master order.
pub fn parse_point_bonuses(text: &str, parts: &mut DeliveryParts) -> Result<(), String> {
    parts.bonuses = Some(crate::delivery::parse_point_bonuses(
        &moly_assets::json::master::rows(text)?,
    )?);
    Ok(())
}

/// `birthdayPartyDeliveryTotalRewards`, master order.
pub fn parse_total_rewards(text: &str, parts: &mut DeliveryParts) -> Result<(), String> {
    parts.totals = Some(crate::delivery::parse_total_rewards(
        &moly_assets::json::master::rows(text)?,
    )?);
    Ok(())
}

/// The delivery tables as their three tables resolve.
#[derive(Default)]
pub struct DeliveryParts {
    pub rewards: Option<Vec<RewardRow>>,
    pub bonuses: Option<Vec<BonusRow>>,
    pub totals: Option<Vec<TotalRow>>,
}

impl DeliveryParts {
    /// The tables, once all three are in.
    pub fn complete(&mut self) -> Option<DeliveryTables> {
        if self.rewards.is_none() || self.bonuses.is_none() || self.totals.is_none() {
            return None;
        }
        let tables = DeliveryTables::new(
            self.rewards.take()?,
            self.bonuses.take()?,
            self.totals.take()?,
        );
        let (rewards, bonuses, totals) = tables.counts();
        info!(
            "[masters] delivery tables: {rewards} reward rows, {bonuses} point bonus rows, {totals} total reward rows"
        );
        Some(tables)
    }
}
