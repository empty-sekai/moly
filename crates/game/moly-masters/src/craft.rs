//! The craft masters (blueprints, their costs and terms, the first-craft
//! bonus, the material types and the white blueprint item) and the pure
//! checks the craft screen and the server apply to a request.

use std::collections::BTreeMap;

use bevy::prelude::*;
use moly_protocol::api::MysekaiCraftType;
use moly_protocol::rows::{UserMysekaiCanvas, UserMysekaiFixture, UserMysekaiMaterial};
use serde_json::Value;

use crate::inventory::{fixture_total_count, material_quantity};

/// `MasterMysekaiBlueprint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasterBlueprint {
    pub id: i32,
    pub craft_type: MysekaiCraftType,
    pub craft_target_id: i32,
    /// 0 when the master row has none (no limit).
    pub craft_count_limit: i32,
    pub is_enable_sketch: bool,
    pub is_obtained_by_convert: bool,
    pub is_available_without_possession: bool,
}

/// `MasterMysekaiBlueprintMysekaiMaterialCost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialCost {
    pub id: i32,
    pub mysekai_blueprint_id: i32,
    pub mysekai_material_id: i32,
    pub seq: i32,
    pub quantity: i32,
}

/// `MasterMysekaiBlueprintTerm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlueprintTerm {
    pub mysekai_blueprint_id: i32,
    pub start_at: i64,
    pub end_at: i64,
}

/// A cards master row as the canvas craft reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Read by the canvas craft once it is modelled.
pub struct CardRow {
    pub character_id: i32,
    /// `cardRarityType` (`rarity_1` to `rarity_4`, `rarity_birthday`).
    pub rarity: String,
}

/// The craft masters the craft screen and the server read
/// (`mysekai-blueprints.json`, `mysekai-blueprint-material-costs.json`,
/// `mysekai-blueprint-terms.json`, `mysekai-rank-obtained-exps.json`,
/// `mysekai-materials.json`, `mysekai-items.json`). Each is `None` when the
/// runtime root lacks it.
#[derive(Resource, Clone, Debug, Default)]
pub struct CraftMasters {
    pub blueprints: Option<BTreeMap<i32, MasterBlueprint>>,
    /// Blueprint id -> its cost rows, master order.
    pub costs: Option<BTreeMap<i32, Vec<MaterialCost>>>,
    pub terms: Option<Vec<BlueprintTerm>>,
    /// `MasterMysekaiRankObtainedExp` of `craft_mysekai_fixture_first_bonus`:
    /// `Some(None)` when the master is present without that row.
    pub first_craft_bonus: Option<Option<i32>>,
    /// Material id -> `mysekaiMaterialType`.
    pub material_types: Option<BTreeMap<i32, String>>,
    /// The first `white_blueprint` item id of the items master.
    pub white_blueprint_item: Option<Option<i32>>,
}

impl CraftMasters {
    pub fn blueprint(&self, id: i32) -> Option<&MasterBlueprint> {
        self.blueprints.as_ref()?.get(&id)
    }

    /// `MasterDataManager.GetMysekaiBlueprintMysekaiMaterialCost(id)`.
    pub fn blueprint_costs(&self, id: i32) -> Option<&[MaterialCost]> {
        self.costs.as_ref()?.get(&id).map(Vec::as_slice)
    }

    /// `MasterDataManager.GetMysekaiBlueprintTermByBlueprintId(id)`.
    pub fn term(&self, id: i32) -> Option<&BlueprintTerm> {
        self.terms
            .as_ref()?
            .iter()
            .find(|term| term.mysekai_blueprint_id == id)
    }
}

/// The `mysekaiRankObtainedExps` row type the first-craft bonus reads.
const FIRST_BONUS_TYPE: &str = "craft_mysekai_fixture_first_bonus";

/// `mysekaiBlueprints`.
pub fn parse_blueprints(rows: &[Value]) -> Result<BTreeMap<i32, MasterBlueprint>, String> {
    rows.iter()
        .map(|row| {
            let kind = row["mysekaiCraftType"].as_str().unwrap_or_default();
            let craft_type = MysekaiCraftType::parse(kind)
                .ok_or_else(|| format!("mysekaiCraftType {kind:?} of {row} is not known"))?;
            let id = crate::int32(row, "id")?;
            Ok((
                id,
                MasterBlueprint {
                    id,
                    craft_type,
                    craft_target_id: crate::int32(row, "craftTargetId")?,
                    craft_count_limit: crate::optional_int32(row, "craftCountLimit")?,
                    is_enable_sketch: crate::flag(row, "isEnableSketch")?,
                    is_obtained_by_convert: crate::flag(row, "isObtainedByConvert")?,
                    is_available_without_possession: crate::flag(row,
                        "isAvailableWithoutPossession")?,
                },
            ))
        })
        .collect()
}

/// `mysekaiBlueprintMysekaiMaterialCosts`: the rows per blueprint.
pub fn parse_costs(rows: &[Value]) -> Result<BTreeMap<i32, Vec<MaterialCost>>, String> {
    let mut costs: BTreeMap<i32, Vec<MaterialCost>> = BTreeMap::new();
    for row in rows {
        let cost = MaterialCost {
            id: crate::int32(row, "id")?,
            mysekai_blueprint_id: crate::int32(row, "mysekaiBlueprintId")?,
            mysekai_material_id: crate::int32(row, "mysekaiMaterialId")?,
            seq: crate::int32(row, "seq")?,
            quantity: crate::int32(row, "quantity")?,
        };
        costs
            .entry(cost.mysekai_blueprint_id)
            .or_default()
            .push(cost);
    }
    Ok(costs)
}

/// `mysekaiBlueprintTerms`.
pub fn parse_terms(rows: &[Value]) -> Result<Vec<BlueprintTerm>, String> {
    rows.iter()
        .map(|row| {
            Ok(BlueprintTerm {
                mysekai_blueprint_id: crate::int32(row, "mysekaiBlueprintId")?,
                start_at: row["startAt"].as_i64().ok_or("startAt is not an integer")?,
                end_at: row["endAt"].as_i64().ok_or("endAt is not an integer")?,
            })
        })
        .collect()
}

/// `mysekaiRankObtainedExps`: the first-craft bonus row's quantity.
pub fn parse_rank_obtained_exps(rows: &[Value]) -> Result<Option<i32>, String> {
    rows.iter()
        .find(|row| row["mysekaiRankObtainedExpType"].as_str() == Some(FIRST_BONUS_TYPE))
        .map(|row| crate::int32(row, "quantity"))
        .transpose()
}

/// `mysekaiMaterials`: material id -> `mysekaiMaterialType`.
pub fn parse_material_types(rows: &[Value]) -> Result<BTreeMap<i32, String>, String> {
    rows.iter()
        .map(|row| {
            let kind = row["mysekaiMaterialType"]
                .as_str()
                .ok_or_else(|| format!("mysekaiMaterialType of {row} is not a string"))?;
            Ok((crate::int32(row, "id")?, kind.to_owned()))
        })
        .collect()
}

/// `mysekaiItems`: the first `white_blueprint` item id.
pub fn parse_white_blueprint_item(rows: &[Value]) -> Result<Option<i32>, String> {
    rows.iter()
        .find(|row| row["mysekaiItemType"].as_str() == Some("white_blueprint"))
        .map(|row| crate::int32(row, "id"))
        .transpose()
}

/// `cards`: card id -> the card's character and rarity.
pub fn parse_cards(rows: &[Value]) -> Result<BTreeMap<i32, CardRow>, String> {
    rows.iter()
        .map(|row| {
            let id = crate::int32(row, "id")?;
            let rarity = row["cardRarityType"]
                .as_str()
                .ok_or_else(|| format!("card {id} has no cardRarityType"))?;
            Ok((
                id,
                CardRow {
                    character_id: crate::int32(row, "characterId")?,
                    rarity: rarity.to_owned(),
                },
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The client's checks (`MysekaiUserDataUtility`)
// ---------------------------------------------------------------------------

/// `MysekaiCanvasUtility.CanvasMaterialId`: the cost id of the canvas's
/// stand-in character material row (the canvas screen's "any member" cell).
pub const CANVAS_MATERIAL_ID: i32 = -1;

/// `CraftPreview`'s count selector: at least 1, at most 999.
pub const CRAFT_COUNT_MIN: i32 = 1;
pub const CRAFT_COUNT_MAX: i32 = 999;

/// The owned data a check reads.
#[derive(Clone, Copy, Debug)]
pub struct CraftOwned<'a> {
    pub materials: &'a [UserMysekaiMaterial],
    pub fixtures: &'a [UserMysekaiFixture],
    pub canvases: &'a [UserMysekaiCanvas],
    /// `GetFixtureMaxCount()`: the fixture possession limit of the user's
    /// level (`None` when the master lacks the level).
    pub fixture_max_count: Option<i32>,
}

impl CraftOwned<'_> {
    /// `GetRemainingFixtureCapacity`: the limit less the total, never below 0.
    fn remaining_fixture_capacity(&self) -> Result<i32, String> {
        let max = self.fixture_max_count.ok_or(
            "the fixture possession limit of the user's level is not in the possession master",
        )?;
        Ok((max - fixture_total_count(self.fixtures)).max(0))
    }
}

/// `IsEnoughMaterialToCraft(list, craftCount, useDummyMaterial)`: every cost
/// row has a material row holding at least `cost.quantity * craftCount`; with
/// `use_dummy_material` a row whose id is [`CANVAS_MATERIAL_ID`] is skipped.
/// An empty list is not enough.
pub fn is_enough_material_to_craft(
    costs: &[MaterialCost],
    craft_count: i32,
    use_dummy_material: bool,
    materials: &[UserMysekaiMaterial],
) -> bool {
    if costs.is_empty() {
        return false;
    }
    costs.iter().all(|cost| {
        if use_dummy_material && cost.id == CANVAS_MATERIAL_ID {
            return true;
        }
        materials
            .iter()
            .find(|row| row.mysekai_material_id == cost.mysekai_material_id)
            .is_some_and(|row| row.quantity >= cost.quantity.wrapping_mul(craft_count))
    })
}

/// `GetCanCraftCount(blueprint, additionalMaterialCost)`: 1 for a blueprint
/// limited to one craft; otherwise the fewest whole crafts any cost row's
/// material pays for (0 when a material row is missing), capped at the
/// remaining fixture capacity. The tutorial cap is left out (the tutorial is
/// out of scope). `Err` names a missing master.
pub fn can_craft_count(
    blueprint: &MasterBlueprint,
    costs: &[MaterialCost],
    additional: Option<&MaterialCost>,
    owned: &CraftOwned,
) -> Result<i32, String> {
    if costs.is_empty() {
        return Ok(0);
    }
    if blueprint.craft_count_limit == 1 {
        return Ok(1);
    }
    let mut count = i32::MAX;
    for cost in costs.iter().chain(additional) {
        let Some(row) = owned
            .materials
            .iter()
            .find(|row| row.mysekai_material_id == cost.mysekai_material_id)
        else {
            return Ok(0);
        };
        // A signed division that gives 0 for a zero divisor, as the game's
        // own build computes it.
        let crafts = if cost.quantity == 0 {
            0
        } else {
            row.quantity.wrapping_div(cost.quantity)
        };
        count = count.min(crafts);
    }
    Ok(count.min(owned.remaining_fixture_capacity()?))
}

/// `GetCraftCountLimit`: the blueprint's limit (unlimited when 0), lowered
/// to the remaining fixture capacity when that is below the master limit.
pub fn craft_count_limit(blueprint: &MasterBlueprint, owned: &CraftOwned) -> Result<i32, String> {
    let limit = if blueprint.craft_count_limit != 0 {
        blueprint.craft_count_limit
    } else {
        i32::MAX
    };
    let remaining = owned.remaining_fixture_capacity()?;
    Ok(if remaining < blueprint.craft_count_limit {
        remaining
    } else {
        limit
    })
}

/// `IsCreateCountLimit`: the owned quantity of the fixture in this texture
/// is at least [`craft_count_limit`].
pub fn is_create_count_limit(
    blueprint: &MasterBlueprint,
    texture_id: i32,
    owned: &CraftOwned,
) -> Result<bool, String> {
    let have = owned
        .fixtures
        .iter()
        .find(|row| {
            row.mysekai_fixture_id == blueprint.craft_target_id && row.texture_id == texture_id
        })
        .map_or(0, |row| row.quantity);
    Ok(have >= craft_count_limit(blueprint, owned)?)
}

/// `IsReachedPossessionLimitToCraft`: for a fixture or canvas blueprint, the
/// create count limit, or a fixture total at or above the fixture limit.
pub fn is_reached_possession_limit_to_craft(
    blueprint: &MasterBlueprint,
    texture_id: i32,
    owned: &CraftOwned,
) -> Result<bool, String> {
    if !matches!(
        blueprint.craft_type,
        MysekaiCraftType::MysekaiFixture | MysekaiCraftType::MysekaiCanvas
    ) {
        return Ok(false);
    }
    if is_create_count_limit(blueprint, texture_id, owned)? {
        return Ok(true);
    }
    let max = owned.fixture_max_count.ok_or(
        "the fixture possession limit of the user's level is not in the possession master",
    )?;
    Ok(max <= fixture_total_count(owned.fixtures))
}

/// `IsFirstCraft`: a fixture blueprint with no fixture row of its target (any
/// texture), or a canvas blueprint with no canvas row of its target; never
/// for the other types.
pub fn is_first_craft(blueprint: &MasterBlueprint, owned: &CraftOwned) -> bool {
    match blueprint.craft_type {
        MysekaiCraftType::MysekaiFixture => !owned
            .fixtures
            .iter()
            .any(|row| row.mysekai_fixture_id == blueprint.craft_target_id),
        MysekaiCraftType::MysekaiCanvas => !owned
            .canvases
            .iter()
            .any(|row| row.mysekai_fixture_id == blueprint.craft_target_id),
        _ => false,
    }
}

/// The quantity a cost row needs for a craft count (`MaterialCostList`'s
/// `cost.quantity * craftCount`).
pub fn need_quantity(cost: &MaterialCost, craft_count: i32) -> i32 {
    cost.quantity.wrapping_mul(craft_count)
}

/// The owned quantity of a cost row's material (0 without a row).
pub fn have_quantity(cost: &MaterialCost, materials: &[UserMysekaiMaterial]) -> i32 {
    material_quantity(materials, cost.mysekai_material_id)
}

/// `CraftPreview.SetupCraftCountInputSelector`: the selector's maximum, the
/// craftable count when above 1 (else 1), capped at 999.
pub fn craft_count_selector_max(can_craft_count: i32) -> i32 {
    let count = if can_craft_count > 1 {
        can_craft_count
    } else {
        1
    };
    count.min(CRAFT_COUNT_MAX)
}

/// `TimeUtility.IsWithinTime(checkAt, startAt, endAt)`: from `startAt` up to,
/// not including, `endAt`; an `endAt` of 0 has no end.
pub fn is_within_time(check_at: i64, start_at: i64, end_at: i64) -> bool {
    if end_at == 0 {
        return start_at != 0 && start_at <= check_at;
    }
    start_at <= check_at && check_at < end_at
}
