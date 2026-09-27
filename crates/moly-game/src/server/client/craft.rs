//! The craft screen's side of the craft loop: the masters it reads, the
//! client's own checks of a craft (`MysekaiUserDataUtility`), and the two
//! requests it sends.
//!
//! - `PostUserMysekaiCraftApi` (`user/{userId}/mysekai/craft`) carries
//!   [`UserMysekaiCraftRequest`]; the craft screen and the canvas screen both
//!   send it (`MysekaiCraftService.Execute`, which sends a quantity of 1 when
//!   the caller gives none).
//! - `PostUserMysekaiHousingSketchApi` (`user/{userId}/mysekai/housing/sketch`)
//!   carries [`UserMysekaiHousingSketchRequest`].
//!
//! Both reply with `SuiteUser`: the tables the request changed, which the
//! client merges before it reads its copies (`UserDataManager.UpdateAll`).
//! [`post_craft`] and [`post_sketch`] run the endpoints the server model
//! installs; by the time they return, the reply's tables are already in
//! [`ClientMysekaiInventory`](super::inventory::ClientMysekaiInventory)
//! (the rank and total experience reach the server module's `ClientUserData`
//! with the next response delivery, as every response does). Without an
//! endpoint a request is refused by name.
//!
//! The checks below are the client's; the server model applies the same
//! checks to refuse a request the craft screen would not have sent.

// The craft, canvas and sketch screens read these.
#![allow(dead_code)]

use std::collections::BTreeMap;

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

use super::inventory::{
    fixture_total_count, material_quantity, SuiteUserSections, UserMysekaiCanvas,
    UserMysekaiFixture, UserMysekaiMaterial,
};

/// `MysekaiCraftType`, in the source's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MysekaiCraftType {
    MysekaiFixture,
    MysekaiTool,
    MysekaiCanvas,
    Material,
}

impl MysekaiCraftType {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "mysekai_fixture" => Some(Self::MysekaiFixture),
            "mysekai_tool" => Some(Self::MysekaiTool),
            "mysekai_canvas" => Some(Self::MysekaiCanvas),
            "material" => Some(Self::Material),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::MysekaiFixture => "mysekai_fixture",
            Self::MysekaiTool => "mysekai_tool",
            Self::MysekaiCanvas => "mysekai_canvas",
            Self::Material => "material",
        }
    }
}

/// `MasterMysekaiBlueprint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MasterBlueprint {
    pub(crate) id: i32,
    pub(crate) craft_type: MysekaiCraftType,
    pub(crate) craft_target_id: i32,
    /// 0 when the master row has none (no limit).
    pub(crate) craft_count_limit: i32,
    pub(crate) is_enable_sketch: bool,
    pub(crate) is_obtained_by_convert: bool,
    pub(crate) is_available_without_possession: bool,
}

/// `MasterMysekaiBlueprintMysekaiMaterialCost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaterialCost {
    pub(crate) id: i32,
    pub(crate) mysekai_blueprint_id: i32,
    pub(crate) mysekai_material_id: i32,
    pub(crate) seq: i32,
    pub(crate) quantity: i32,
}

/// `MasterMysekaiBlueprintTerm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlueprintTerm {
    pub(crate) mysekai_blueprint_id: i32,
    pub(crate) start_at: i64,
    pub(crate) end_at: i64,
}

/// The craft masters the craft screen and the server read
/// (`mysekai-blueprints.json`, `mysekai-blueprint-material-costs.json`,
/// `mysekai-blueprint-terms.json`, `mysekai-rank-obtained-exps.json`,
/// `mysekai-materials.json`, `mysekai-items.json`). Each is `None` when the
/// runtime root lacks it; the server model names those.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct CraftMasters {
    pub(crate) blueprints: Option<BTreeMap<i32, MasterBlueprint>>,
    /// Blueprint id -> its cost rows, master order.
    pub(crate) costs: Option<BTreeMap<i32, Vec<MaterialCost>>>,
    pub(crate) terms: Option<Vec<BlueprintTerm>>,
    /// `MasterMysekaiRankObtainedExp` of `craft_mysekai_fixture_first_bonus`:
    /// `Some(None)` when the master is present without that row.
    pub(crate) first_craft_bonus: Option<Option<i32>>,
    /// Material id -> `mysekaiMaterialType`.
    pub(crate) material_types: Option<BTreeMap<i32, String>>,
    /// The first `white_blueprint` item id of the items master.
    pub(crate) white_blueprint_item: Option<Option<i32>>,
}

impl CraftMasters {
    pub(crate) fn blueprint(&self, id: i32) -> Option<&MasterBlueprint> {
        self.blueprints.as_ref()?.get(&id)
    }

    /// `MasterDataManager.GetMysekaiBlueprintMysekaiMaterialCost(id)`.
    pub(crate) fn blueprint_costs(&self, id: i32) -> Option<&[MaterialCost]> {
        self.costs.as_ref()?.get(&id).map(Vec::as_slice)
    }

    /// `MasterDataManager.GetMysekaiBlueprintTermByBlueprintId(id)`.
    pub(crate) fn term(&self, id: i32) -> Option<&BlueprintTerm> {
        self.terms
            .as_ref()?
            .iter()
            .find(|term| term.mysekai_blueprint_id == id)
    }
}

// ---------------------------------------------------------------------------
// The client's checks (`MysekaiUserDataUtility`)
// ---------------------------------------------------------------------------

/// The owned data a check reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CraftOwned<'a> {
    pub(crate) materials: &'a [UserMysekaiMaterial],
    pub(crate) fixtures: &'a [UserMysekaiFixture],
    pub(crate) canvases: &'a [UserMysekaiCanvas],
    /// `GetFixtureMaxCount()`: the fixture possession limit of the user's
    /// level (`None` when the master lacks the level).
    pub(crate) fixture_max_count: Option<i32>,
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

/// `MysekaiCanvasUtility.CanvasMaterialId`: the cost id of the canvas's
/// stand-in character material row (the canvas screen's "any member" cell).
pub(crate) const CANVAS_MATERIAL_ID: i32 = -1;

/// `IsEnoughMaterialToCraft(list, craftCount, useDummyMaterial)`: every cost
/// row has a material row holding at least `cost.quantity * craftCount`; with
/// `use_dummy_material` a row whose id is [`CANVAS_MATERIAL_ID`] is skipped.
/// An empty list is not enough.
pub(crate) fn is_enough_material_to_craft(
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
pub(crate) fn can_craft_count(
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
pub(crate) fn craft_count_limit(
    blueprint: &MasterBlueprint,
    owned: &CraftOwned,
) -> Result<i32, String> {
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
pub(crate) fn is_create_count_limit(
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
pub(crate) fn is_reached_possession_limit_to_craft(
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
pub(crate) fn is_first_craft(blueprint: &MasterBlueprint, owned: &CraftOwned) -> bool {
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
pub(crate) fn need_quantity(cost: &MaterialCost, craft_count: i32) -> i32 {
    cost.quantity.wrapping_mul(craft_count)
}

/// The owned quantity of a cost row's material (0 without a row).
pub(crate) fn have_quantity(cost: &MaterialCost, materials: &[UserMysekaiMaterial]) -> i32 {
    material_quantity(materials, cost.mysekai_material_id)
}

/// `CraftPreview`'s count selector: at least 1, at most 999.
pub(crate) const CRAFT_COUNT_MIN: i32 = 1;
pub(crate) const CRAFT_COUNT_MAX: i32 = 999;

/// `CraftPreview.SetupCraftCountInputSelector`: the selector's maximum, the
/// craftable count when above 1 (else 1), capped at 999.
pub(crate) fn craft_count_selector_max(can_craft_count: i32) -> i32 {
    let count = if can_craft_count > 1 {
        can_craft_count
    } else {
        1
    };
    count.min(CRAFT_COUNT_MAX)
}

// ---------------------------------------------------------------------------
// Requests and replies
// ---------------------------------------------------------------------------

/// `UserMysekaiCraftRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiCraftRequest {
    pub(crate) blueprint_id: i32,
    pub(crate) texture_id: Option<i32>,
    pub(crate) card_id: Option<i32>,
    pub(crate) is_special_training: Option<bool>,
    pub(crate) quantity: i32,
}

/// `UserMysekaiHousingSketchRequest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiHousingSketchRequest {
    pub(crate) mysekai_owner_user_id: i64,
    pub(crate) mysekai_site_id: i32,
    pub(crate) mysekai_blueprint_id: i32,
}

/// A `SuiteUser` reply: success with the tables the request changed, or a
/// refusal the client shows as the API error (it starts nothing).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SuiteUserReply {
    pub(crate) success: bool,
    /// Why the server refused (`None` on success).
    pub(crate) refusal: Option<String>,
    /// The tables the reply carries (already merged into the client copy).
    pub(crate) updated: SuiteUserSections,
}

impl SuiteUserReply {
    pub(crate) fn refused(reason: String) -> Self {
        Self {
            success: false,
            refusal: Some(reason),
            updated: SuiteUserSections::default(),
        }
    }
}

/// The server model's endpoints for the two requests.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct CraftEndpoints {
    pub(crate) craft: SystemId<In<UserMysekaiCraftRequest>, SuiteUserReply>,
    pub(crate) sketch: SystemId<In<UserMysekaiHousingSketchRequest>, SuiteUserReply>,
}

/// `PostUserMysekaiCraftApi`: send a craft or canvas request and read its
/// reply.
#[allow(dead_code)] // Called by the craft and canvas screens.
pub(crate) fn post_craft(world: &mut World, request: UserMysekaiCraftRequest) -> SuiteUserReply {
    let Some(endpoints) = world.get_resource::<CraftEndpoints>().copied() else {
        return refused(
            "PostUserMysekaiCraftApi",
            "no server endpoint is installed".into(),
        );
    };
    world
        .run_system_with(endpoints.craft, request)
        .unwrap_or_else(|error| refused("PostUserMysekaiCraftApi", error.to_string()))
}

/// `PostUserMysekaiHousingSketchApi`: send a sketch request and read its
/// reply.
#[allow(dead_code)] // Called by the sketch screen.
pub(crate) fn post_sketch(
    world: &mut World,
    request: UserMysekaiHousingSketchRequest,
) -> SuiteUserReply {
    let Some(endpoints) = world.get_resource::<CraftEndpoints>().copied() else {
        return refused(
            "PostUserMysekaiHousingSketchApi",
            "no server endpoint is installed".into(),
        );
    };
    world
        .run_system_with(endpoints.sketch, request)
        .unwrap_or_else(|error| refused("PostUserMysekaiHousingSketchApi", error.to_string()))
}

fn refused(api: &str, reason: String) -> SuiteUserReply {
    warn!("[server] {api} refused: {reason}");
    SuiteUserReply::refused(format!("{api}: {reason}"))
}
