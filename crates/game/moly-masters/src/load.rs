//! The typed master tables and the loaders that fill them, plus the row
//! tables a reader parses for itself.
//!
//! Each table is a [`MasterTable`] requested from the asset layer; every
//! consumer in the process shares the request and the parsed value. A
//! resource appears once every table it is built from has resolved (a table
//! the layer cannot give leaves its own fields empty and is named in
//! [`MasterTables::missing`]).
//!
//! The assembly from a table's text to the resource field is here, so the
//! server and the client cannot disagree about a column name.

use std::collections::{BTreeMap, HashSet};

use bevy::prelude::*;
use moly_assets::json::master::{MasterData, MasterTable};
use serde_json::Value;

use crate::avatar::AvatarMasters;
use crate::craft::CraftMasters;
use crate::delivery::{BonusRow, DeliveryTables, RewardRow, TotalRow};
use crate::inventory::PossessionMasters;
use crate::mission::MissionMasters;
use crate::recovery::{BoostItemRow, RecoveryRow};
use crate::StaminaMax;

/// The master resources one loader fills, plus the row slices a reader
/// parses for itself (the acquisition dialogs' six tables, the harvest drop
/// classifier's ids and material rows), from the same requested tables.
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
    /// The four mission tables (`mysekaiNormalMissionSheets`,
    /// `mysekaiNormalMissions`, `mysekaiNormalMissionRewards` and the
    /// mission rank-experience rows), if any are in.
    pub mission: Option<MissionMasters>,
    /// `mysekaiStaminas`.
    pub stamina_max: Option<StaminaMax>,
    /// `mysekaiStaminaRecovery`: the live bonus one recovery spends and the
    /// boost stamina it restores.
    pub stamina_recovery: Option<RecoveryRow>,
    /// `boostItems` (master order).
    pub boost_items: Option<Vec<BoostItemRow>>,
    /// The blueprint ids the harvest drop classifier reads, from the same
    /// table the craft reader takes (no second request for it).
    pub blueprint_ids: Option<HashSet<i64>>,
    /// The item ids the harvest drop classifier reads, from the same table
    /// the craft reader takes.
    pub item_ids: Option<HashSet<i64>>,
    /// `mysekaiMaterials`: id, `mysekaiMaterialType` and `mysekaiSiteIds`,
    /// for the harvest drop classifier (the craft reader takes the types).
    pub material_rows: Option<Vec<(i64, String, Vec<u32>)>>,
    /// Tables a reader parses for itself, as their files hold them, by the
    /// upstream table: [`MasterRowsPlugin`] adds the fixture, tool and record
    /// rows, and the loader adds the material and item rows it parses anyway.
    pub rows: BTreeMap<&'static str, Vec<Value>>,
    /// Every row table [`MasterRowsPlugin`] requests has been taken.
    rows_ready: bool,
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
    mission_left: usize,
    stamina_left: usize,
    convert_left: usize,
    /// The convert screen's tables (`mysekaiConvertItems`, the machine's
    /// slots, levels and releases, `resourceBoxes`, the obtain rows, the
    /// record categories and the pass's rate), in the convert module's own
    /// group.
    pub convert: Option<crate::convert::ConvertMasters>,
}

impl MasterTables {
    /// Every table requested, and each one taken (a table the layer could
    /// not give counts as taken: it is named in [`Self::missing`]).
    pub fn ready(&self) -> bool {
        self.started && self.pending.is_empty()
    }

    /// Every row table [`MasterRowsPlugin`] requests has been taken (a table
    /// the layer could not give counts as taken and is named in
    /// [`Self::missing`]). A process without that plugin never reaches it.
    pub fn rows_ready(&self) -> bool {
        self.rows_ready
    }

    /// A small synthetic table set a harness seats directly: synthetic pool
    /// maxima and empty table resources; the harness fills what it drives.
    pub fn fixture() -> Self {
        Self {
            craft: Some(CraftMasters::default()),
            possession: Some(PossessionMasters::default()),
            avatar: Some(AvatarMasters::default()),
            delivery: None,
            mission: None,
            stamina_max: Some(StaminaMax {
                normal: 1000,
                enhance: 1000,
                boost: 10000,
            }),
            stamina_recovery: Some(RecoveryRow {
                boost_quantity: 1,
                recovery_boost_stamina: 100,
            }),
            boost_items: Some(Vec::new()),
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
            // A table outside the five resources counts toward none of them.
            Group::Convert => &mut self.convert_left,
            Group::Craft => &mut self.craft_left,
            Group::Possession => &mut self.possession_left,
            Group::Avatar => &mut self.avatar_left,
            Group::Delivery => &mut self.delivery_left,
            Group::Mission => &mut self.mission_left,
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
            Group::Mission => {
                let value = std::mem::take(&mut self.parts.mission);
                commands.insert_resource(value.clone());
                self.mission = Some(value);
            }
            Group::Stamina => {
                if let Some(value) = self.parts.stamina_max.take() {
                    commands.insert_resource(value);
                    self.stamina_max = Some(value);
                }
                self.stamina_recovery = self.parts.stamina_recovery;
                self.boost_items = self.parts.boost_items.take();
            }
            Group::Convert => {
                let value = std::mem::take(&mut self.parts.convert);
                commands.insert_resource(value.clone());
                self.convert = Some(value);
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
    Mission,
    Stamina,
    /// The convert screen's tables, which make one resource of their own.
    Convert,
}

const CRAFT_TABLES: usize = 7;
const POSSESSION_TABLES: usize = 2;
const AVATAR_TABLES: usize = 4;
const DELIVERY_TABLES: usize = 3;
const MISSION_TABLES: usize = 4;
const STAMINA_TABLES: usize = 3;
/// The convert module's tables (see `crate::convert`): ten of its own plus
/// `configs`, whose skip price it reads.
const CONVERT_TABLES: usize = 11;

/// One table's parse result, before it is seated into its resource.
#[derive(Default)]
struct Parts {
    craft: CraftMasters,
    possession: PossessionMasters,
    avatar: AvatarMasters,
    delivery: DeliveryParts,
    mission: MissionMasters,
    stamina_max: Option<StaminaMax>,
    stamina_recovery: Option<RecoveryRow>,
    boost_items: Option<Vec<BoostItemRow>>,
    blueprint_ids: Option<HashSet<i64>>,
    item_ids: Option<HashSet<i64>>,
    material_rows: Option<Vec<(i64, String, Vec<u32>)>>,
    rows: Vec<(&'static str, Vec<Value>)>,
    convert: crate::convert::ConvertMasters,
}

/// One master table the loader requests and where its rows go.
struct Entry {
    table: MasterTable<Parts>,
    /// Moves the one field a table fills out of the parse result.
    seat: fn(&mut MasterTables, Parts),
    group: Group,
}

/// A table whose parser fills one field of its resource.
macro_rules! field_table {
    ($table:literal, $name:literal, $parse:path, $resource:ident, $field:ident, $group:ident) => {
        Entry {
            table: MasterTable {
                table: $table,
                name: $name,
                parse: |text| {
                    let mut parts = Parts::default();
                    $parse(text, &mut parts.$resource.$field)?;
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

/// Every resource's tables, in request order.
static TABLES: &[Entry] = &[
    root_table!(
        "mysekaiStaminas",
        "mysekaiStaminas (the server's pool maxima)",
        parse_staminas,
        stamina_max,
        Stamina
    ),
    root_table!(
        "mysekaiStaminaRecovery",
        "mysekaiStaminaRecovery (the recover dialog's rate and yield)",
        parse_stamina_recovery,
        stamina_recovery,
        Stamina
    ),
    root_table!(
        "boostItems",
        "boostItems (what a boost item recovers)",
        parse_boost_items,
        boost_items,
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
    field_table!(
        "mysekaiConvertItems",
        "mysekaiConvertItems (the convert screen's items)",
        parse_convert_items,
        convert,
        items,
        Convert
    ),
    field_table!(
        "mysekaiConvertItemTabs",
        "mysekaiConvertItemTabs (the convert screen's item tabs)",
        parse_convert_tabs,
        convert,
        tabs,
        Convert
    ),
    field_table!(
        "mysekaiConvertFixtureSlot",
        "mysekaiConvertFixtureSlot (the convert machine's slots)",
        parse_convert_slots,
        convert,
        slots,
        Convert
    ),
    field_table!(
        "mysekaiConvertFixtureLevels",
        "mysekaiConvertFixtureLevels (the convert machine's levels)",
        parse_convert_levels,
        convert,
        levels,
        Convert
    ),
    field_table!(
        "mysekaiConvertFixtureLevelReleases",
        "mysekaiConvertFixtureLevelReleases (what a convert machine level releases)",
        parse_convert_level_releases,
        convert,
        level_releases,
        Convert
    ),
    field_table!(
        "mysekaiConvertItemObtainMysekaiBlueprints",
        "mysekaiConvertItemObtainMysekaiBlueprints (the convert blueprint draws)",
        parse_convert_blueprint_obtains,
        convert,
        blueprint_obtains,
        Convert
    ),
    field_table!(
        "mysekaiConvertItemObtainMysekaiMusicRecords",
        "mysekaiConvertItemObtainMysekaiMusicRecords (the convert record draws)",
        parse_convert_record_obtains,
        convert,
        record_obtains,
        Convert
    ),
    field_table!(
        "mysekaiMusicRecordCategories",
        "mysekaiMusicRecordCategories (the convert record choices)",
        parse_convert_record_categories,
        convert,
        record_categories,
        Convert
    ),
    field_table!(
        "mysekaiColorfulPass",
        "mysekaiColorfulPass (the convert time's pass decrease)",
        parse_convert_pass_rate,
        convert,
        pass_rate,
        Convert
    ),
    Entry {
        table: MasterTable {
            table: "resourceBoxes",
            name: "resourceBoxes (the convert items' boxes)",
            parse: |text| {
                let mut parts = Parts::default();
                parts.convert.boxes = Some(crate::convert::parse_boxes(text)?);
                Ok(parts)
            },
        },
        seat: |tables, partial| tables.parts.convert.boxes = partial.convert.boxes,
        group: Group::Convert,
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
        "mysekaiBlueprintTermMysekaiMaterialCosts",
        "mysekaiBlueprintTermMysekaiMaterialCosts (the server's per-term material costs)",
        parse_term_costs,
        craft,
        term_costs,
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
                let rows = table_rows(text, "mysekaiMaterials")?;
                parts.material_rows = material_rows(&rows).ok();
                parts.rows.push(("mysekaiMaterials", rows));
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.craft.material_types = partial.craft.material_types;
            tables.material_rows = partial.material_rows;
            tables.rows.extend(partial.rows);
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
                let rows = table_rows(text, "mysekaiItems")?;
                parts.item_ids = row_ids(&rows).ok();
                parts.rows.push(("mysekaiItems", rows));
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.craft.white_blueprint_item = partial.craft.white_blueprint_item;
            tables.item_ids = partial.item_ids;
            tables.rows.extend(partial.rows);
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
    table!(
        "mysekaiNormalMissionSheets",
        "mysekaiNormalMissionSheets (the mission sheets)",
        parse_mission_sheets,
        mission,
        sheets,
        Mission
    ),
    table!(
        "mysekaiNormalMissions",
        "mysekaiNormalMissions (the mission rows)",
        parse_missions,
        mission,
        missions,
        Mission
    ),
    table!(
        "mysekaiNormalMissionRewards",
        "mysekaiNormalMissionRewards (the mission reward rows)",
        parse_mission_rewards,
        mission,
        rewards,
        Mission
    ),
    table!(
        "mysekaiNormalMissionRewardMysekaiRankObtainedExps",
        "mysekaiNormalMissionRewardMysekaiRankObtainedExps (the missions' rank-experience rows)",
        parse_mission_rank_exps,
        mission,
        rank_exps,
        Mission
    ),
    Entry {
        table: MasterTable {
            table: "configs",
            name: "configs (the convert skip price and the refresh calendar)",
            parse: |text| {
                let mut parts = Parts::default();
                parse_convert_configs(text, &mut parts.convert)?;
                Ok(parts)
            },
        },
        seat: |tables, partial| {
            tables.parts.convert.shorten_jewel_per_minute =
                partial.convert.shorten_jewel_per_minute;
            tables.parts.convert.calendar = partial.convert.calendar;
        },
        group: Group::Convert,
    },
];

/// Requests every resource's tables once, and starts their counters.
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
    tables.mission_left = MISSION_TABLES;
    tables.stamina_left = STAMINA_TABLES;
    tables.convert_left = CONVERT_TABLES;
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

/// The shared resources' loader.
pub struct MasterTablesPlugin;

impl Plugin for MasterTablesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterTables>()
            .add_systems(Startup, load)
            .add_systems(
                PreUpdate,
                resolve_tables.after(moly_assets::json::master::resolve),
            );
    }
}

/// The row tables a reader parses itself, each requested once per process:
/// the acquisition dialogs read the fixture, tool and record rows from
/// [`MasterTables::rows`] instead of requesting the tables again.
static ROW_TABLES: &[MasterTable<Vec<Value>>] = &[
    MasterTable {
        table: "mysekaiSiteHarvestFixtures",
        name: "mysekaiSiteHarvestFixtures (harvest settlement)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiSiteBulkHarvests",
        name: "mysekaiSiteBulkHarvests (harvest settlement)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiSiteBulkHarvestTargets",
        name: "mysekaiSiteBulkHarvestTargets (harvest settlement)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiSiteBulkHarvestTargetGroups",
        name: "mysekaiSiteBulkHarvestTargetGroups (harvest settlement)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiFixtures",
        name: "mysekaiFixtures (the acquisition dialogs' fixture rows)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiTools",
        name: "mysekaiTools (the acquisition dialogs' and the harvest tools)",
        parse: moly_assets::json::master::rows,
    },
    MasterTable {
        table: "mysekaiMusicRecords",
        name: "mysekaiMusicRecords (the acquisition dialogs' and the harvest records)",
        parse: moly_assets::json::master::rows,
    },
];

/// Which row tables are still out.
#[derive(Resource, Default)]
struct MasterRows {
    pending: Vec<&'static str>,
    started: bool,
}

/// Requests the row tables once.
fn load_rows(mut data: ResMut<MasterData>, mut rows: ResMut<MasterRows>) {
    if rows.started {
        return;
    }
    rows.started = true;
    rows.pending = ROW_TABLES.iter().map(|table| table.name).collect();
    for table in ROW_TABLES {
        data.request(table);
    }
}

/// Seats each row table as it resolves; a table the layer cannot give is
/// named in [`MasterTables::missing`] and counts as taken.
fn resolve_rows(
    mut data: ResMut<MasterData>,
    mut rows: ResMut<MasterRows>,
    mut tables: ResMut<MasterTables>,
) {
    if !rows.started {
        return;
    }
    for table in ROW_TABLES {
        if !rows.pending.contains(&table.name) {
            continue;
        }
        match data.take(table) {
            None => continue,
            Some(Ok(values)) => {
                tables.rows.insert(table.table, values);
            }
            Some(Err(error)) => tables.missing.push(error.to_string()),
        }
        rows.pending.retain(|known| *known != table.name);
    }
    if rows.pending.is_empty() {
        tables.rows_ready = true;
    }
}

/// The extra row tables' loader. A process that reads no rows for itself
/// leaves it out, and [`MasterTables::rows_ready`] stays false there.
pub struct MasterRowsPlugin;

impl Plugin for MasterRowsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterTables>()
            .init_resource::<MasterRows>()
            .add_systems(Startup, load_rows)
            .add_systems(
                PreUpdate,
                resolve_rows.after(moly_assets::json::master::resolve),
            );
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

/// The convert module's tables, each filling its one field (see
/// [`crate::convert`] for the readers).
fn parse_convert_items(
    text: &str,
    rows: &mut Option<Vec<crate::convert::ConvertItem>>,
) -> Result<(), String> {
    *rows = Some(crate::convert::parse_items(text)?);
    Ok(())
}

fn parse_convert_tabs(text: &str, rows: &mut Option<Vec<(i32, i32)>>) -> Result<(), String> {
    *rows = Some(crate::convert::parse_tabs(text)?);
    Ok(())
}

fn parse_convert_slots(text: &str, slots: &mut Option<(i32, i32)>) -> Result<(), String> {
    *slots = Some(crate::convert::parse_slot(text)?);
    Ok(())
}

fn parse_convert_levels(text: &str, rows: &mut Option<Vec<i32>>) -> Result<(), String> {
    *rows = Some(crate::convert::parse_levels(text)?);
    Ok(())
}

fn parse_convert_level_releases(
    text: &str,
    rows: &mut Option<Vec<crate::convert::LevelRelease>>,
) -> Result<(), String> {
    *rows = Some(crate::convert::parse_level_releases(text)?);
    Ok(())
}

fn parse_convert_blueprint_obtains(
    text: &str,
    rows: &mut Option<Vec<crate::convert::BlueprintObtain>>,
) -> Result<(), String> {
    *rows = Some(crate::convert::parse_blueprint_obtains(text)?);
    Ok(())
}

fn parse_convert_record_obtains(
    text: &str,
    rows: &mut Option<Vec<crate::convert::RecordObtain>>,
) -> Result<(), String> {
    *rows = Some(crate::convert::parse_record_obtains(text)?);
    Ok(())
}

fn parse_convert_record_categories(
    text: &str,
    rows: &mut Option<Vec<crate::convert::RecordCategory>>,
) -> Result<(), String> {
    *rows = Some(crate::convert::parse_record_categories(text)?);
    Ok(())
}

fn parse_convert_pass_rate(text: &str, rate: &mut Option<f32>) -> Result<(), String> {
    *rate = Some(crate::convert::parse_pass_rate(text)?);
    Ok(())
}

/// `configs`: the convert skip price and the MySekai refresh calendar.
fn parse_convert_configs(
    text: &str,
    convert: &mut crate::convert::ConvertMasters,
) -> Result<(), String> {
    convert.shorten_jewel_per_minute = Some(crate::convert::parse_shorten_config(text)?);
    let rows = table_rows(text, "configs")?;
    convert.calendar = Some(crate::clock::CalendarRefresh::from_configs(&rows)?);
    Ok(())
}

/// An upstream row table, sorted by `id` (a table without ids, such as
/// `configs`, keeps its master order).
fn table_rows(text: &str, table: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let Value::Array(rows) = value else {
        return Err(format!("{table} is not an array of rows"));
    };
    if let Some(index) = rows.iter().position(|row| !row.is_object()) {
        return Err(format!("{table} row {index} is not an object"));
    }
    let mut rows = rows;
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    Ok(rows)
}

/// [`table_rows`] for the readers of other modules of this crate.
pub(crate) fn master_rows_of(text: &str, table: &str) -> Result<Vec<Value>, String> {
    table_rows(text, table)
}

/// The row of a one-row table, which upstream publishes as one JSON object
/// (`mysekaiConvertFixtureSlot`, `mysekaiColorfulPass`); an array's first row
/// is read as well.
pub(crate) fn master_single_row(text: &str, table: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    match value {
        row @ Value::Object(_) => Ok(row),
        Value::Array(rows) => match rows.into_iter().next() {
            Some(row @ Value::Object(_)) => Ok(row),
            Some(other) => Err(format!("{table} row 0 is {other}, not an object")),
            None => Err(format!("{table} has no row")),
        },
        _ => Err(format!("{table} is neither a row object nor an array of rows")),
    }
}

/// `mysekaiStaminaRecovery`: the single row's two columns.
pub fn parse_stamina_recovery(text: &str, row: &mut Option<RecoveryRow>) -> Result<(), String> {
    let value = Value::Object(moly_assets::json::master::object(text)?);
    *row = Some(RecoveryRow {
        boost_quantity: crate::int32(&value, "boostQuantity")?,
        recovery_boost_stamina: crate::int32(&value, "recoveryBoostStamina")?,
    });
    Ok(())
}

/// `boostItems`: the id and `recoveryValue` of each row.
pub fn parse_boost_items(text: &str, items: &mut Option<Vec<BoostItemRow>>) -> Result<(), String> {
    let rows = table_rows(text, "boostItems")?;
    *items = Some(
        rows.iter()
            .map(|row| {
                Ok(BoostItemRow {
                    id: crate::int32(row, "id")?,
                    recovery_value: crate::int32(row, "recoveryValue")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    Ok(())
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

/// `mysekaiBlueprintTerms`: a row this reader does not take (an unknown
/// `mysekaiBlueprintTermTabType`) is dropped and reported by
/// [`crate::craft::read_terms`], not the whole table.
pub fn parse_terms(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.terms = Some(crate::craft::read_terms(text)?);
    Ok(())
}

/// `mysekaiBlueprintTermMysekaiMaterialCosts`: the rows per cost group.
pub fn parse_term_costs(text: &str, craft: &mut CraftMasters) -> Result<(), String> {
    craft.term_costs = Some(crate::craft::parse_term_costs(&table_rows(
        text,
        "mysekaiBlueprintTermMysekaiMaterialCosts",
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
    parts.rewards = Some(crate::delivery::parse_rewards(
        &moly_assets::json::master::rows(text)?,
    )?);
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

/// `mysekaiNormalMissionSheets`, master order.
pub fn parse_mission_sheets(text: &str, parts: &mut MissionMasters) -> Result<(), String> {
    parts.sheets = crate::mission::parse_sheets(&moly_assets::json::master::rows(text)?)?;
    Ok(())
}

/// `mysekaiNormalMissions`, master order.
pub fn parse_missions(text: &str, parts: &mut MissionMasters) -> Result<(), String> {
    parts.missions = crate::mission::parse_missions(&moly_assets::json::master::rows(text)?)?;
    Ok(())
}

/// `mysekaiNormalMissionRewards`, master order.
pub fn parse_mission_rewards(text: &str, parts: &mut MissionMasters) -> Result<(), String> {
    parts.rewards = crate::mission::parse_rewards(&moly_assets::json::master::rows(text)?)?;
    Ok(())
}

/// `mysekaiNormalMissionRewardMysekaiRankObtainedExps`, master order.
pub fn parse_mission_rank_exps(text: &str, parts: &mut MissionMasters) -> Result<(), String> {
    parts.rank_exps = crate::mission::parse_rank_exps(&moly_assets::json::master::rows(text)?)?;
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
