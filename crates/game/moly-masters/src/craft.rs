//! The craft masters (blueprints, their costs and terms, the first-craft
//! bonus, the material types and the white blueprint item) and the pure
//! checks the craft screen and the server apply to a request.

use std::collections::BTreeMap;

use bevy::prelude::*;
use moly_protocol::api::MysekaiCraftType;
use moly_protocol::rows::{
    UserMysekaiBlueprintLendCraftCount, UserMysekaiCanvas, UserMysekaiFixture,
    UserMysekaiMaterial,
};
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

/// `MysekaiBlueprintTermTabType`: the craft tab a term's rows belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlueprintTermTabType {
    /// `limited_term`.
    LimitedTerm,
    /// `birthday_anniversary`.
    BirthdayAnniversary,
}

impl BlueprintTermTabType {
    /// The spelling the term table's `mysekaiBlueprintTermTabType` column
    /// carries; `None` for a value this reader does not know.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "limited_term" => Some(Self::LimitedTerm),
            "birthday_anniversary" => Some(Self::BirthdayAnniversary),
            _ => None,
        }
    }

    /// The enum's wire spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::LimitedTerm => "limited_term",
            Self::BirthdayAnniversary => "birthday_anniversary",
        }
    }
}

/// `MasterMysekaiBlueprintTerm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlueprintTerm {
    pub id: i32,
    pub mysekai_blueprint_id: i32,
    pub start_at: i64,
    pub end_at: i64,
    /// The tab whose rows the term belongs to.
    pub tab_type: BlueprintTermTabType,
    /// `mysekaiBlueprintTermMysekaiMaterialCostGroupId`: below 1 when the term
    /// does not override the blueprint's material costs.
    pub material_cost_group_id: i32,
    /// `craftLimit`: the crafts the term allows; below 1 when it has no cap.
    pub craft_limit: i32,
}

/// `MasterMysekaiBlueprintTermMysekaiMaterialCost`: one material row of a
/// term's cost group. A term's rows replace the blueprint's cost rows whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermCost {
    pub id: i32,
    pub group_id: i32,
    pub mysekai_material_id: i32,
    pub seq: i32,
    pub quantity: i32,
}

impl TermCost {
    /// The row in the craft checks' cost shape. A term cost row names no
    /// blueprint, so the blueprint id is 0; its own id is kept (it can never
    /// be the canvas stand-in row, which has a negative id).
    pub fn as_material_cost(self) -> MaterialCost {
        MaterialCost {
            id: self.id,
            mysekai_blueprint_id: 0,
            mysekai_material_id: self.mysekai_material_id,
            seq: self.seq,
            quantity: self.quantity,
        }
    }
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
/// `mysekai-blueprint-terms.json`,
/// `mysekai-blueprint-term-mysekai-material-costs.json`,
/// `mysekai-rank-obtained-exps.json`, `mysekai-materials.json`,
/// `mysekai-items.json`). Each is `None` when the runtime root lacks it.
#[derive(Resource, Clone, Debug, Default)]
pub struct CraftMasters {
    pub blueprints: Option<BTreeMap<i32, MasterBlueprint>>,
    /// Blueprint id -> its cost rows, master order.
    pub costs: Option<BTreeMap<i32, Vec<MaterialCost>>>,
    pub terms: Option<Vec<BlueprintTerm>>,
    /// Term cost group id -> its rows, master order.
    pub term_costs: Option<BTreeMap<i32, Vec<TermCost>>>,
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

    /// The first term row of a blueprint
    /// (`GetMysekaiBlueprintTermByBlueprintId`); a blueprint may have several.
    pub fn term(&self, id: i32) -> Option<&BlueprintTerm> {
        let terms = self.terms_of_blueprint(id);
        terms.first().copied()
    }

    /// `MasterDataManager.GetMysekaiBlueprintTerm(id)`: the term by its own
    /// id.
    pub fn term_by_id(&self, id: i32) -> Option<&BlueprintTerm> {
        self.terms.as_ref()?.iter().find(|term| term.id == id)
    }

    /// `GetMysekaiBlueprintTermsByBlueprintId(id)`: every term row of a
    /// blueprint, master order, with no time filter.
    pub fn terms_of_blueprint(&self, id: i32) -> Vec<&BlueprintTerm> {
        self.terms
            .as_ref()
            .map(|terms| {
                terms
                    .iter()
                    .filter(|term| term.mysekai_blueprint_id == id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `GetMysekaiBlueprintTermsInSessionByBlueprintId(id)`: the terms of a
    /// blueprint that hold `now`, master order.
    pub fn terms_in_session_of_blueprint(&self, id: i32, now: i64) -> Vec<&BlueprintTerm> {
        self.terms_of_blueprint(id)
            .into_iter()
            .filter(|term| is_within_time(now, term.start_at, term.end_at))
            .collect()
    }

    /// `GetMysekaiBlueprintsTermInSession()`, or its tab-type form when
    /// `tab_type` is given: the terms that hold `now`, master order.
    pub fn terms_in_session(
        &self,
        now: i64,
        tab_type: Option<BlueprintTermTabType>,
    ) -> Vec<&BlueprintTerm> {
        self.terms
            .as_ref()
            .map(|terms| {
                terms
                    .iter()
                    .filter(|term| is_within_time(now, term.start_at, term.end_at))
                    .filter(|term| tab_type.is_none_or(|kind| term.tab_type == kind))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `HasMysekaiBlueprintTerm(blueprintId)`: whether the blueprint has any
    /// term row. A master-only question: the clock is not read, so a term
    /// that has ended still counts.
    pub fn has_blueprint_term(&self, id: i32) -> bool {
        !self.terms_of_blueprint(id).is_empty()
    }

    /// `IsAllMysekaiBlueprintTermTabType(blueprintId, tabType)`: whether every
    /// term row of the blueprint has the given tab type. Every row is read,
    /// expired ones too, and a blueprint without a term row is not all of the
    /// type.
    pub fn is_all_blueprint_term_tab_type(&self, id: i32, tab_type: BlueprintTermTabType) -> bool {
        let terms = self.terms_of_blueprint(id);
        !terms.is_empty() && terms.iter().all(|term| term.tab_type == tab_type)
    }

    /// `MasterDataManager.GetMysekaiBlueprintMysekaiMaterialTermCost(groupId)`.
    pub fn term_costs(&self, group_id: i32) -> Option<&[TermCost]> {
        self.term_costs
            .as_ref()?
            .get(&group_id)
            .map(Vec::as_slice)
    }

    /// The cost rows a term charges: its own group when the term names one
    /// and the group has rows, otherwise the blueprint's rows. `None` when
    /// neither exists. The term's rows replace the blueprint's, they do not
    /// add to them.
    pub fn costs_for(
        &self,
        blueprint_id: i32,
        term: Option<&BlueprintTerm>,
    ) -> Option<Vec<MaterialCost>> {
        if let Some(term) = term.filter(|term| term.material_cost_group_id >= 1) {
            if let Some(costs) = self
                .term_costs(term.material_cost_group_id)
                .filter(|costs| !costs.is_empty())
            {
                return Some(costs.iter().map(|cost| cost.as_material_cost()).collect());
            }
        }
        self.blueprint_costs(blueprint_id).map(|costs| costs.to_vec())
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

/// One `mysekaiBlueprintTerms` row. A row without a `mysekaiBlueprintTermTabType`
/// column (or with an empty one) reads as `limited_term`, the enum's zero
/// value, which is what the source's deserialiser leaves it as; a value this
/// reader does not know is refused by name, so the row is dropped and reported.
pub fn parse_term(row: &Value) -> Result<BlueprintTerm, String> {
    let tab_type = match row.get("mysekaiBlueprintTermTabType") {
        None | Some(Value::Null) => BlueprintTermTabType::LimitedTerm,
        Some(Value::String(word)) if word.is_empty() => BlueprintTermTabType::LimitedTerm,
        Some(Value::String(word)) => BlueprintTermTabType::parse(word).ok_or_else(|| {
            format!(
                "mysekaiBlueprintTermTabType {word:?} of {row} is not a blueprint term tab type"
            )
        })?,
        Some(other) => {
            return Err(format!(
                "mysekaiBlueprintTermTabType of {row} is not a string: {other}"
            ))
        }
    };
    Ok(BlueprintTerm {
        id: crate::int32(row, "id")?,
        mysekai_blueprint_id: crate::int32(row, "mysekaiBlueprintId")?,
        start_at: row["startAt"].as_i64().ok_or("startAt is not an integer")?,
        end_at: row["endAt"].as_i64().ok_or("endAt is not an integer")?,
        tab_type,
        material_cost_group_id: crate::optional_int32(
            row,
            "mysekaiBlueprintTermMysekaiMaterialCostGroupId",
        )?,
        craft_limit: crate::optional_int32(row, "craftLimit")?,
    })
}

/// [`parse_term`] over rows already parsed; every row must read.
pub fn parse_terms(rows: &[Value]) -> Result<Vec<BlueprintTerm>, String> {
    rows.iter().map(parse_term).collect()
}

/// `mysekaiBlueprintTerms` read row by row: a row [`parse_term`] refuses is
/// dropped and named once per reason, and the rest of the table stands.
pub fn read_terms(text: &str) -> Result<Vec<BlueprintTerm>, String> {
    moly_assets::json::master::read_rows(text, |row| Ok(Some(parse_term(row)?)))
}

/// `mysekaiBlueprintTermMysekaiMaterialCosts`: the rows per cost group, in
/// the given order.
pub fn parse_term_costs(rows: &[Value]) -> Result<BTreeMap<i32, Vec<TermCost>>, String> {
    let mut costs: BTreeMap<i32, Vec<TermCost>> = BTreeMap::new();
    for row in rows {
        let cost = TermCost {
            id: crate::int32(row, "id")?,
            group_id: crate::int32(row, "groupId")?,
            mysekai_material_id: crate::int32(row, "mysekaiMaterialId")?,
            seq: crate::int32(row, "seq")?,
            quantity: crate::int32(row, "quantity")?,
        };
        costs.entry(cost.group_id).or_default().push(cost);
    }
    Ok(costs)
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

// ---------------------------------------------------------------------------
// The term-aware checks
// ---------------------------------------------------------------------------

/// `UserDataManager.GetMysekaiBlueprintLendCraftCount(termId)`: the crafts the
/// term has used, by its `userMysekaiBlueprintLendCraftCounts` row (0 without
/// one).
pub fn term_craft_count(lends: &[UserMysekaiBlueprintLendCraftCount], term_id: i32) -> i32 {
    lends
        .iter()
        .find(|row| row.mysekai_blueprint_term_id == term_id)
        .map_or(0, |row| row.craft_count)
}

/// `GetTermCraftRemaining(term)`: `MAX` when the term has no cap, otherwise
/// the cap less the crafts it has used, never below 0.
pub fn term_craft_remaining(
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
) -> i32 {
    match term.filter(|term| term.craft_limit >= 1) {
        None => i32::MAX,
        Some(term) => (term.craft_limit - term_craft_count(lends, term.id)).max(0),
    }
}

/// `IsTermCraftCountLimit(term)`: the term's cap is used up.
pub fn is_term_craft_count_limit(
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
) -> bool {
    term_craft_remaining(term, lends) < 1
}

/// `IsAcrossColorCraftLimit(term)`: a `limited_term` term counts its crafts
/// across every colour; any other term (and no term) does not.
pub fn is_across_color_craft_limit(term: Option<&BlueprintTerm>) -> bool {
    term.is_some_and(|term| term.tab_type == BlueprintTermTabType::LimitedTerm)
}

/// `IsAcrossColorCreateCountLimit(blueprint, term)`: for a `limited_term`
/// term, whether the term's crafts used reach [`craft_count_limit`]; any other
/// term (and no term) is never at the limit.
pub fn is_across_color_create_count_limit(
    blueprint: &MasterBlueprint,
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
    owned: &CraftOwned,
) -> Result<bool, String> {
    let Some(term) = term.filter(|term| term.tab_type == BlueprintTermTabType::LimitedTerm) else {
        return Ok(false);
    };
    Ok(term_craft_count(lends, term.id) >= craft_count_limit(blueprint, owned)?)
}

/// `IsEnoughMaterialToCraft(blueprint, craftCount, useDummyMaterial, term)`:
/// the term's cost rows when it names a group with rows, otherwise the
/// blueprint's own rows (an absent cost list is not enough).
pub fn is_enough_material_to_craft_with_term(
    masters: &CraftMasters,
    blueprint_id: i32,
    term: Option<&BlueprintTerm>,
    craft_count: i32,
    use_dummy_material: bool,
    materials: &[UserMysekaiMaterial],
) -> bool {
    match masters.costs_for(blueprint_id, term) {
        Some(costs) => {
            is_enough_material_to_craft(&costs, craft_count, use_dummy_material, materials)
        }
        None => false,
    }
}

/// `GetCanCraftCountByTerm(blueprint, term)`: the crafts the term's own cost
/// rows pay for, capped at the remaining fixture capacity. A term without a
/// positive cost group (or whose group is empty) falls back to
/// [`can_craft_count`] on the blueprint's rows. This is its own algorithm: it
/// has no per-colour count, and a blueprint limited to one craft is 1 before
/// any material is read.
pub fn can_craft_count_by_term(
    masters: &CraftMasters,
    blueprint: &MasterBlueprint,
    term: Option<&BlueprintTerm>,
    owned: &CraftOwned,
) -> Result<i32, String> {
    let term_costs = term
        .filter(|term| term.material_cost_group_id >= 1)
        .and_then(|term| masters.term_costs(term.material_cost_group_id))
        .filter(|costs| !costs.is_empty());
    let Some(term_costs) = term_costs else {
        let costs = masters.blueprint_costs(blueprint.id).unwrap_or_default();
        return can_craft_count(blueprint, costs, None, owned);
    };
    if blueprint.craft_count_limit == 1 {
        return Ok(1);
    }
    let mut count = i32::MAX;
    for cost in term_costs {
        let Some(row) = owned
            .materials
            .iter()
            .find(|row| row.mysekai_material_id == cost.mysekai_material_id)
        else {
            return Ok(0);
        };
        let crafts = if cost.quantity == 0 {
            0
        } else {
            row.quantity.wrapping_div(cost.quantity)
        };
        count = count.min(crafts);
    }
    if count < 1 {
        return Ok(0);
    }
    Ok(count.min(owned.remaining_fixture_capacity()?))
}

/// `CraftPreview.SetupCraftCountInputSelector` on a term cell: the term-capped
/// count and the term's remaining crafts, whichever is lower, then the
/// selector's own bounds.
pub fn term_capped_selector_max(can_craft_count: i32, term_remaining: i32) -> i32 {
    craft_count_selector_max(can_craft_count.min(term_remaining))
}

/// `IsCreateCountLimit(blueprint, textureId, term)`: [`is_across_color_create_count_limit`]
/// first (a `limited_term` term whose crafts used reach the count limit), then
/// the owned quantity of the fixture in this texture against the count limit.
pub fn is_create_count_limit_with_term(
    blueprint: &MasterBlueprint,
    texture_id: i32,
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
    owned: &CraftOwned,
) -> Result<bool, String> {
    if is_across_color_create_count_limit(blueprint, term, lends, owned)? {
        return Ok(true);
    }
    is_create_count_limit(blueprint, texture_id, owned)
}

/// `IsReachedPossessionLimitToCraft(blueprint, textureId, term)`: the term-aware
/// create count limit, or a fixture total at or above the fixture limit.
pub fn is_reached_possession_limit_to_craft_with_term(
    blueprint: &MasterBlueprint,
    texture_id: i32,
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
    owned: &CraftOwned,
) -> Result<bool, String> {
    if !matches!(
        blueprint.craft_type,
        MysekaiCraftType::MysekaiFixture | MysekaiCraftType::MysekaiCanvas
    ) {
        return Ok(false);
    }
    if is_create_count_limit_with_term(blueprint, texture_id, term, lends, owned)? {
        return Ok(true);
    }
    let max = owned.fixture_max_count.ok_or(
        "the fixture possession limit of the user's level is not in the possession master",
    )?;
    Ok(max <= fixture_total_count(owned.fixtures))
}

/// `IsCanCraft(blueprint, textureId, term)`: with a capped `limited_term`
/// term the term's remaining crafts decide instead of the per-colour count
/// limit; then the possession limit and the material check (on the term's
/// costs when it overrides them). The tutorial limit is left out (the tutorial
/// is out of scope).
pub fn is_can_craft(
    masters: &CraftMasters,
    blueprint: &MasterBlueprint,
    texture_id: i32,
    term: Option<&BlueprintTerm>,
    lends: &[UserMysekaiBlueprintLendCraftCount],
    owned: &CraftOwned,
) -> Result<bool, String> {
    let capped_limited_term = term.is_some_and(|term| {
        term.craft_limit >= 1 && term.tab_type == BlueprintTermTabType::LimitedTerm
    });
    if capped_limited_term {
        if term_craft_remaining(term, lends) <= 0 {
            return Ok(false);
        }
    } else if is_create_count_limit_with_term(blueprint, texture_id, term, lends, owned)? {
        return Ok(false);
    }
    if is_reached_possession_limit_to_craft_with_term(blueprint, texture_id, term, lends, owned)? {
        return Ok(false);
    }
    Ok(is_enough_material_to_craft_with_term(
        masters,
        blueprint.id,
        term,
        1,
        true,
        owned.materials,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn blueprint(id: i32) -> MasterBlueprint {
        MasterBlueprint {
            id,
            craft_type: MysekaiCraftType::MysekaiFixture,
            craft_target_id: id,
            craft_count_limit: 0,
            is_enable_sketch: false,
            is_obtained_by_convert: false,
            is_available_without_possession: false,
        }
    }

    fn term(id: i32, blueprint_id: i32, start_at: i64, end_at: i64) -> BlueprintTerm {
        BlueprintTerm {
            id,
            mysekai_blueprint_id: blueprint_id,
            start_at,
            end_at,
            tab_type: BlueprintTermTabType::LimitedTerm,
            material_cost_group_id: 0,
            craft_limit: 0,
        }
    }

    fn lend(term_id: i32, craft_count: i32) -> UserMysekaiBlueprintLendCraftCount {
        UserMysekaiBlueprintLendCraftCount {
            mysekai_blueprint_term_id: term_id,
            craft_count,
        }
    }

    fn owned<'a>(
        materials: &'a [UserMysekaiMaterial],
        fixtures: &'a [UserMysekaiFixture],
    ) -> CraftOwned<'a> {
        CraftOwned {
            materials,
            fixtures,
            canvases: &[],
            fixture_max_count: Some(1000),
        }
    }

    fn masters() -> CraftMasters {
        CraftMasters {
            blueprints: Some(
                [(1, blueprint(1))]
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
            ),
            costs: Some(
                [(
                    1,
                    vec![MaterialCost {
                        id: 1,
                        mysekai_blueprint_id: 1,
                        mysekai_material_id: 100,
                        seq: 1,
                        quantity: 10,
                    }],
                )]
                .into_iter()
                .collect(),
            ),
            terms: Some(vec![
                term(7, 1, 1_000, 2_000),
                BlueprintTerm {
                    tab_type: BlueprintTermTabType::BirthdayAnniversary,
                    ..term(8, 1, 3_000, 4_000)
                },
            ]),
            term_costs: Some(
                [(
                    55,
                    vec![TermCost {
                        id: 900,
                        group_id: 55,
                        mysekai_material_id: 200,
                        seq: 1,
                        quantity: 4,
                    }],
                )]
                .into_iter()
                .collect(),
            ),
            ..CraftMasters::default()
        }
    }

    // MasterMysekaiBlueprintTerm: an unknown `mysekaiBlueprintTermTabType` is
    // a loud row refusal, not a silent fallback (spec 1.1, 6.4 covers the
    // value set: only `limited_term` and `birthday_anniversary` exist).
    #[test]
    fn a_term_row_of_an_unknown_tab_type_is_refused_by_name() {
        let row = json!({
            "id": 3,
            "mysekaiBlueprintId": 1,
            "startAt": 0,
            "endAt": 0,
            "mysekaiBlueprintTermTabType": "term_of_something_else",
        });
        let error = match parse_term(&row) {
            Ok(_) => panic!("the row was taken"),
            Err(error) => error,
        };
        assert!(error.contains("mysekaiBlueprintTermTabType"), "{error}");
        assert!(error.contains("term_of_something_else"), "{error}");
    }

    // MasterMysekaiBlueprintTerm: one row of an unknown tab type is dropped
    // and reported, and the table's other rows still load.
    #[test]
    fn a_term_table_keeps_its_rows_around_the_one_it_refuses() {
        let text = json!([
            {"id": 1, "mysekaiBlueprintId": 1, "startAt": 10, "endAt": 20},
            {"id": 2, "mysekaiBlueprintId": 1, "startAt": 10, "endAt": 20,
             "mysekaiBlueprintTermTabType": "term_of_something_else"},
            {"id": 3, "mysekaiBlueprintId": 1, "startAt": 10, "endAt": 20,
             "mysekaiBlueprintTermTabType": "birthday_anniversary"},
        ])
        .to_string();
        moly_assets::json::master::take_refused_rows();
        let terms = read_terms(&text).unwrap();
        assert_eq!(terms.iter().map(|term| term.id).collect::<Vec<_>>(), vec![1, 3]);
        let refused = moly_assets::json::master::take_refused_rows();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].row, "row 2");
        assert!(refused[0].reason.contains("term_of_something_else"));
    }

    // MasterMysekaiBlueprintTerm: the string column is the only wire form and
    // its two values map to the two tabs; a row without it is `limited_term`,
    // the enum's zero value (spec 1.1).
    #[test]
    fn a_term_row_reads_its_tab_type_and_its_optional_numbers() {
        let row = json!({
            "id": 4,
            "mysekaiBlueprintId": 2,
            "startAt": 10,
            "endAt": 20,
            "mysekaiBlueprintTermTabType": "birthday_anniversary",
            "mysekaiBlueprintTermMysekaiMaterialCostGroupId": 55,
            "craftLimit": 3,
        });
        let term = parse_term(&row).unwrap();
        assert_eq!(
            term,
            BlueprintTerm {
                id: 4,
                mysekai_blueprint_id: 2,
                start_at: 10,
                end_at: 20,
                tab_type: BlueprintTermTabType::BirthdayAnniversary,
                material_cost_group_id: 55,
                craft_limit: 3,
            }
        );
        let bare = json!({"id": 5, "mysekaiBlueprintId": 2, "startAt": 10, "endAt": 20});
        let term = parse_term(&bare).unwrap();
        assert_eq!(term.tab_type, BlueprintTermTabType::LimitedTerm);
        assert_eq!(term.material_cost_group_id, 0);
        assert_eq!(term.craft_limit, 0);
    }

    // MasterMysekaiBlueprintTermMysekaiMaterialCost: the rows are grouped by
    // `groupId` (spec 1.2).
    #[test]
    fn the_term_costs_are_grouped_by_their_group_id() {
        let rows = [
            json!({"id": 1, "groupId": 7, "mysekaiMaterialId": 100, "seq": 1, "quantity": 2}),
            json!({"id": 2, "groupId": 7, "mysekaiMaterialId": 101, "seq": 2, "quantity": 3}),
            json!({"id": 3, "groupId": 8, "mysekaiMaterialId": 100, "seq": 1, "quantity": 4}),
        ];
        let costs = parse_term_costs(&rows).unwrap();
        assert_eq!(costs[&7].len(), 2);
        assert_eq!(costs[&8][0].quantity, 4);
    }

    // HasMysekaiBlueprintTerm is a master-only question: an expired term still
    // gives the blueprint its term status (spec 2.2, 2.5).
    #[test]
    fn has_term_ignores_the_clock_but_the_session_lookup_does_not() {
        let masters = masters();
        assert!(masters.has_blueprint_term(1));
        assert!(!masters.has_blueprint_term(2));
        // 1_500 is inside the first row's session only, so the second row is
        // not in session although it belongs to the same blueprint.
        let session = masters.terms_in_session(1_500, None);
        let ids = session.iter().map(|term| term.id).collect::<Vec<_>>();
        assert_eq!(ids, vec![7]);
        let birthday =
            masters.terms_in_session(1_500, Some(BlueprintTermTabType::BirthdayAnniversary));
        assert!(birthday.is_empty());
        assert_eq!(masters.terms_in_session_of_blueprint(1, 3_500).len(), 1);
        // IsAllMysekaiBlueprintTermTabType reads every row: one birthday row
        // makes the blueprint not all-limited.
        assert!(!masters.is_all_blueprint_term_tab_type(1, BlueprintTermTabType::LimitedTerm));
        assert!(!masters.is_all_blueprint_term_tab_type(9, BlueprintTermTabType::LimitedTerm));
    }

    // GetTermCraftRemaining / IsTermCraftCountLimit: a term cap below 1 is no
    // cap; otherwise the cap less the lend counter, floored at 0 (spec 2.6,
    // 4 item 3).
    #[test]
    fn the_term_remaining_is_the_cap_less_the_lend_counter() {
        let mut capped = term(7, 1, 0, 0);
        capped.craft_limit = 3;
        assert_eq!(term_craft_remaining(Some(&capped), &[]), 3);
        assert_eq!(term_craft_remaining(Some(&capped), &[lend(7, 2)]), 1);
        assert_eq!(term_craft_remaining(Some(&capped), &[lend(7, 9)]), 0);
        assert!(!is_term_craft_count_limit(Some(&capped), &[lend(7, 2)]));
        assert!(is_term_craft_count_limit(Some(&capped), &[lend(7, 3)]));
        // The counter is keyed by term id, not by blueprint.
        let uncapped = term(7, 1, 0, 0);
        assert_eq!(
            term_craft_remaining(Some(&uncapped), &[lend(7, 99)]),
            i32::MAX
        );
        assert_eq!(term_craft_remaining(None, &[lend(7, 99)]), i32::MAX);
    }

    // IsAcrossColorCreateCountLimit / IsAcrossColorCraftLimit: only a
    // `limited_term` counts crafts across colours (spec 2.6).
    #[test]
    fn only_a_limited_term_counts_crafts_across_colours() {
        let blueprint = blueprint(1);
        let inventory: [UserMysekaiFixture; 0] = [];
        let owned = owned(&[], &inventory);
        let limited = term(7, 1, 0, 0);
        let birthday = BlueprintTerm {
            tab_type: BlueprintTermTabType::BirthdayAnniversary,
            ..limited
        };
        assert!(is_across_color_craft_limit(Some(&limited)));
        assert!(!is_across_color_craft_limit(Some(&birthday)));
        assert!(!is_across_color_craft_limit(None));
        // The count limit is unlimited here, so a used counter of 0 is not at
        // it; the birthday term is never at it.
        assert!(
            !is_across_color_create_count_limit(&blueprint, Some(&limited), &[], &owned).unwrap()
        );
        assert!(!is_across_color_create_count_limit(
            &blueprint,
            Some(&birthday),
            &[lend(8, 5)],
            &owned
        )
        .unwrap());
    }

    // ResolveMaterialCostList: a term with a positive cost group replaces the
    // blueprint's rows; a term without one keeps them; the group is not added
    // to them (spec 2.4, 4 item 4).
    #[test]
    fn a_term_cost_group_replaces_the_blueprints_rows_and_does_not_add_to_them() {
        let masters = masters();
        let plain = masters.costs_for(1, None).unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].mysekai_material_id, 100);
        let mut with_group = term(7, 1, 0, 0);
        with_group.material_cost_group_id = 55;
        let replaced = masters.costs_for(1, Some(&with_group)).unwrap();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].mysekai_material_id, 200);
        assert_eq!(replaced[0].id, 900);
        // A group with no rows leaves the blueprint's rows in place.
        let mut empty_group = with_group;
        empty_group.material_cost_group_id = 404;
        assert_eq!(masters.costs_for(1, Some(&empty_group)).unwrap(), plain);
    }

    // GetCanCraftCountByTerm: with a positive cost group it counts the term
    // rows only, and a blueprint limited to one craft answers 1; without a
    // group it falls back to GetCanCraftCount (spec 2.7, 4 item 5).
    #[test]
    fn the_term_craftable_count_uses_the_term_rows_and_the_fixture_capacity() {
        let masters = masters();
        let materials = [
            UserMysekaiMaterial {
                mysekai_material_id: 100,
                quantity: 100,
            },
            UserMysekaiMaterial {
                mysekai_material_id: 200,
                quantity: 9,
            },
        ];
        let fixtures: [UserMysekaiFixture; 0] = [];
        let held = owned(&materials, &fixtures);
        let mut with_group = term(7, 1, 0, 0);
        with_group.material_cost_group_id = 55;
        // 9 / 4 = 2 crafts on the term row, although the blueprint's row pays
        // for 10.
        assert_eq!(
            can_craft_count_by_term(&masters, &blueprint(1), Some(&with_group), &held).unwrap(),
            2
        );
        assert_eq!(
            can_craft_count_by_term(&masters, &blueprint(1), None, &held).unwrap(),
            10
        );
        // The term-capped selector maximum is the lower of the two.
        assert_eq!(term_capped_selector_max(2, 1), 1);
        assert_eq!(term_capped_selector_max(2, 5), 2);
        // A material row the term needs but the user lacks is 0 crafts.
        let poor = [UserMysekaiMaterial {
            mysekai_material_id: 100,
            quantity: 100,
        }];
        let held = owned(&poor, &fixtures);
        assert_eq!(
            can_craft_count_by_term(&masters, &blueprint(1), Some(&with_group), &held).unwrap(),
            0
        );
    }

    // IsCanCraft: a capped `limited_term` substitutes the term's remaining
    // crafts for the per-colour count limit (spec 2.6).
    #[test]
    fn a_capped_limited_term_can_craft_switches_on_the_term_remaining() {
        let masters = masters();
        let materials = [UserMysekaiMaterial {
            mysekai_material_id: 100,
            quantity: 100,
        }];
        let fixtures: [UserMysekaiFixture; 0] = [];
        let owned = owned(&materials, &fixtures);
        let first = blueprint(1);
        let mut capped = term(7, 1, 0, 0);
        capped.craft_limit = 1;
        assert!(is_can_craft(&masters, &first, 1, Some(&capped), &[], &owned).unwrap());
        assert!(
            !is_can_craft(&masters, &first, 1, Some(&capped), &[lend(7, 1)], &owned).unwrap()
        );
        // Without a cap the per-colour count limit is still read (unlimited
        // here), and a missing blueprint cost row means no craft.
        assert!(is_can_craft(&masters, &first, 1, None, &[], &owned).unwrap());
        assert!(!is_can_craft(&masters, &blueprint(2), 1, None, &[], &owned).unwrap());
    }
}
