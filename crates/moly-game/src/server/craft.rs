//! The craft loop's two requests: `PostUserMysekaiCraftApi` (the craft screen
//! and the canvas screen, `UserMysekaiCraftRequest`) and
//! `PostUserMysekaiHousingSketchApi` (`UserMysekaiHousingSketchRequest`).
//! Both reply with `SuiteUser`, the tables they changed; the client merges
//! them before it reads its copies. The client sends them through
//! [`super::client::craft::post_craft`] and [`super::client::craft::post_sketch`];
//! [`install`] puts the endpoints in place.
//!
//! **Named policies.** The server's rules are not in the client, so each
//! rule below is the client's own check or display of the same values, or a
//! master table, or is named as an inference.
//!
//! *Craft refusals* (the checks the craft screen makes before it sends; the
//! server refuses a request that fails one, and the client shows the API
//! error and starts nothing):
//! - the blueprint is a master blueprint and has cost rows
//!   (`MasterDataManager.GetMysekaiBlueprintMysekaiMaterialCost`);
//! - the user owns it or it is available without possession (the craft
//!   list's filter in `UserResourceFactory`), by the owned blueprints policy;
//! - its blueprint term, when it has one, holds the server clock
//!   (`CraftPreview`'s send: `HasMysekaiBlueprintTerm`, then
//!   `TimeUtility.IsWithinCurrentTime(startAt, endAt)`, else the
//!   `MSG_CRAFT_TERM_LIMIT` dialog);
//! - the quantity is within the count selector (`CraftPreview`: at least 1,
//!   at most 999 and at most the craftable count, `GetCanCraftCount`, which
//!   caps it at the remaining fixture capacity);
//! - `IsReachedPossessionLimitToCraft` is false (the per-blueprint create
//!   count limit, `IsCreateCountLimit`, and the fixture possession limit of
//!   the user's level);
//! - `IsEnoughMaterialToCraft(costs, quantity)`.
//!
//! *Craft spend*: each cost row's material loses `cost.quantity * quantity`
//! (`MaterialCostList`'s need quantity, the same product the sufficiency
//! check compares).
//!
//! *Craft grant*: a fixture blueprint adds `quantity` to the
//! `userMysekaiFixtures` row of (craft target, texture) and stamps its
//! `lastObtainedAt` with the server clock; the craft screen reads that row
//! for the owned count (`CraftPreview.SetFixtureQuantityText`). A request
//! without a texture takes texture 1 (an inference: the client's base colour,
//! `IsCanCraft(blueprint, 1)`).
//!
//! *First-craft experience*: when `IsFirstCraft` holds before the grant (no
//! fixture row of the target), `totalExp` gains the master
//! `mysekaiRankObtainedExps` row `craft_mysekai_fixture_first_bonus`, and the
//! rank follows by the rank from experience policy; the result dialog shows
//! that row's quantity for a first craft
//! (`CraftResultSubWindowDialog.SetupFirstCraftBonus`) and the experience as
//! the total after the reply less the total before it. Two inferences: the
//! bonus is granted once per request, not per crafted piece (the balloon
//! shows the row's quantity alone), and a craft that is not a first craft
//! grants no experience (the master holds no other row).
//!
//! *Refused by name*: a tool blueprint (the tool rows belong to the harvest
//! mock), a canvas blueprint (its character material needs the card's
//! character and rarity from the cards master, which loads after the join;
//! with it in, the character material rule is still not modelled), and the
//! material type (no master blueprint has it).
//!
//! *Sketch* (the client offers it only in another user's MySekai,
//! `SketchUtility.IsSketchAvailable`, and visiting is not in the product, so
//! the owner and site are taken as stated): refused unless the blueprint is a
//! master blueprint with `isEnableSketch` (`SketchUtility.IsFixtureCanSketch`),
//! not owned (`SketchUtility.HasTargetBluePrint`), and the user holds a
//! `white_blueprint` item (`IsSketchAvailable`: quantity above 0). The reply
//! spends one white blueprint (an inference: the client only checks for one)
//! and adds the `userMysekaiBlueprints` row with `obtainedAt` at the server
//! clock.
//!
//! Native instrument: `MOLY_CRAFT_MOCK_REQUEST=blueprintId[:quantity[:textureId]]`
//! makes the home action's craft request (which carries no blueprint) a craft
//! of that blueprint; game mode reads no environment variable.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::{json, Value};

use super::client::craft::{
    can_craft_count, craft_count_selector_max, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft, BlueprintTerm, CraftEndpoints, CraftOwned,
    MasterBlueprint, MaterialCost, MysekaiCraftType, SuiteUserReply, UserMysekaiCraftRequest,
    UserMysekaiHousingSketchRequest, CRAFT_COUNT_MAX, CRAFT_COUNT_MIN,
};
use super::client::inventory::{ClientMysekaiInventory, UserMysekaiBlueprint, UserMysekaiFixture};
use super::delivery::{ClientBirthdayPartyData, SECTION_MYSEKAI_MATERIALS};
use super::inventory::{SECTION_BLUEPRINTS, SECTION_FIXTURES, SECTION_ITEMS};
use super::{Masters, ResponseKind, ServerModel, SECTION_GAMEDATA};

/// The texture a request without one takes (the client's base colour).
pub(crate) const BASE_TEXTURE_ID: i32 = 1;
/// The white blueprints one sketch spends.
pub(crate) const SKETCH_WHITE_BLUEPRINTS: i32 = 1;
const FIRST_BONUS_TYPE: &str = "craft_mysekai_fixture_first_bonus";

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

fn table_rows(text: &str, table: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    super::master_rows(&value, table)
}

fn optional_int(row: &Value, key: &str) -> Result<i32, String> {
    match row.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(_) => super::int32(row, key),
    }
}

fn flag(row: &Value, key: &str) -> Result<bool, String> {
    row[key]
        .as_bool()
        .ok_or_else(|| format!("{key} of {row} is not a boolean"))
}

/// A cards master row as the canvas craft reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Read by the canvas craft once it is modelled.
pub(crate) struct CardRow {
    pub(crate) character_id: i32,
    /// `cardRarityType` (`rarity_1` to `rarity_4`, `rarity_birthday`).
    pub(crate) rarity: String,
}

/// `cards`: card id -> the card's character and rarity.
pub(crate) fn parse_cards(text: &str, masters: &mut Masters) -> Result<(), String> {
    let cards = moly_assets::json::master::rows(text)?
        .iter()
        .map(|row| {
            let id = super::int32(row, "id")?;
            let rarity = row["cardRarityType"]
                .as_str()
                .ok_or_else(|| format!("card {id} has no cardRarityType"))?;
            Ok((
                id,
                CardRow {
                    character_id: super::int32(row, "characterId")?,
                    rarity: rarity.to_owned(),
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    masters.cards = super::Deferred::Ready(cards);
    Ok(())
}

/// `mysekaiBlueprints`.
pub(crate) fn parse_blueprints(text: &str, masters: &mut Masters) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiBlueprints")?;
    let blueprints = rows
        .iter()
        .map(|row| {
            let kind = row["mysekaiCraftType"].as_str().unwrap_or_default();
            let craft_type = MysekaiCraftType::parse(kind)
                .ok_or_else(|| format!("mysekaiCraftType {kind:?} of {row} is not known"))?;
            let id = super::int32(row, "id")?;
            Ok((
                id,
                MasterBlueprint {
                    id,
                    craft_type,
                    craft_target_id: super::int32(row, "craftTargetId")?,
                    craft_count_limit: optional_int(row, "craftCountLimit")?,
                    is_enable_sketch: flag(row, "isEnableSketch")?,
                    is_obtained_by_convert: flag(row, "isObtainedByConvert")?,
                    is_available_without_possession: flag(row, "isAvailableWithoutPossession")?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    masters.craft.blueprints = Some(blueprints);
    Ok(())
}

/// `mysekaiBlueprintMysekaiMaterialCosts`: the rows per blueprint.
pub(crate) fn parse_costs(text: &str, masters: &mut Masters) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiBlueprintMysekaiMaterialCosts")?;
    let mut costs: BTreeMap<i32, Vec<MaterialCost>> = BTreeMap::new();
    for row in &rows {
        let cost = MaterialCost {
            id: super::int32(row, "id")?,
            mysekai_blueprint_id: super::int32(row, "mysekaiBlueprintId")?,
            mysekai_material_id: super::int32(row, "mysekaiMaterialId")?,
            seq: super::int32(row, "seq")?,
            quantity: super::int32(row, "quantity")?,
        };
        costs
            .entry(cost.mysekai_blueprint_id)
            .or_default()
            .push(cost);
    }
    masters.craft.costs = Some(costs);
    Ok(())
}

/// `mysekaiBlueprintTerms`.
pub(crate) fn parse_terms(text: &str, masters: &mut Masters) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiBlueprintTerms")?;
    let terms = rows
        .iter()
        .map(|row| {
            Ok(BlueprintTerm {
                mysekai_blueprint_id: super::int32(row, "mysekaiBlueprintId")?,
                start_at: row["startAt"].as_i64().ok_or("startAt is not an integer")?,
                end_at: row["endAt"].as_i64().ok_or("endAt is not an integer")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.craft.terms = Some(terms);
    Ok(())
}

/// `mysekaiRankObtainedExps`: the first-craft bonus row.
pub(crate) fn parse_rank_obtained_exps(text: &str, masters: &mut Masters) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiRankObtainedExps")?;
    let bonus = rows
        .iter()
        .find(|row| row["mysekaiRankObtainedExpType"].as_str() == Some(FIRST_BONUS_TYPE))
        .map(|row| super::int32(row, "quantity"))
        .transpose()?;
    masters.craft.first_craft_bonus = Some(bonus);
    Ok(())
}

/// `TimeUtility.IsWithinTime(checkAt, startAt, endAt)`: from `startAt` up to,
/// not including, `endAt`; an `endAt` of 0 has no end.
pub(crate) fn is_within_time(check_at: i64, start_at: i64, end_at: i64) -> bool {
    if end_at == 0 {
        return start_at != 0 && start_at <= check_at;
    }
    start_at <= check_at && check_at < end_at
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

/// What a craft request did, for its log line.
struct Crafted {
    blueprint: MasterBlueprint,
    texture_id: i32,
    spent: Vec<(i32, i32)>,
    first: bool,
    exp: i32,
}

impl ServerModel {
    /// The craft policies; the sections the reply carries.
    fn craft(
        &mut self,
        request: &UserMysekaiCraftRequest,
    ) -> Result<(Crafted, Vec<&'static str>), String> {
        let masters = &self.masters.craft;
        let blueprints = masters.blueprints.as_ref().ok_or(
            "the blueprint master (mysekai-blueprints.json) is absent: the blueprint cannot be read",
        )?;
        let blueprint = *blueprints.get(&request.blueprint_id).ok_or_else(|| {
            format!(
                "blueprintId {} is not a master blueprint",
                request.blueprint_id
            )
        })?;
        match blueprint.craft_type {
            MysekaiCraftType::MysekaiFixture => {}
            MysekaiCraftType::MysekaiCanvas => {
                let reason = match self.masters.cards.get(
                    "its character material needs the card's character and rarity",
                ) {
                    Err(reason) => reason,
                    Ok(_) => "the cards master is in, but the canvas craft's character material is not modelled".to_owned(),
                };
                return Err(format!(
                    "blueprint {} is a canvas (cardId {:?}): {reason}",
                    blueprint.id, request.card_id
                ));
            }
            MysekaiCraftType::MysekaiTool => {
                return Err(format!(
                    "blueprint {} is a tool: the tool rows belong to the harvest mock; the tool craft is not modelled",
                    blueprint.id
                ))
            }
            MysekaiCraftType::Material => {
                return Err(format!(
                    "blueprint {} has the material craft type, which the craft screen refuses (MysekaiCraftType is not valid there)",
                    blueprint.id
                ))
            }
        }
        if !blueprint.is_available_without_possession && !self.owns_blueprint(&blueprint) {
            return Err(format!(
                "blueprint {} is not owned (policies.ownedBlueprints) and is not available without possession",
                blueprint.id
            ));
        }
        let now = self.now_ms();
        let terms = masters.terms.as_ref().ok_or(
            "the blueprint term master (mysekai-blueprint-terms.json) is absent: the craft term cannot be checked",
        )?;
        if let Some(term) = terms
            .iter()
            .find(|term| term.mysekai_blueprint_id == blueprint.id)
        {
            if !is_within_time(now, term.start_at, term.end_at) {
                return Err(format!(
                    "blueprint {} is outside its craft term {}..{} at the server clock {now} (MSG_CRAFT_TERM_LIMIT)",
                    blueprint.id, term.start_at, term.end_at
                ));
            }
        }
        if request.quantity < CRAFT_COUNT_MIN || request.quantity > CRAFT_COUNT_MAX {
            return Err(format!(
                "quantity {} is outside the count selector's {CRAFT_COUNT_MIN}..{CRAFT_COUNT_MAX}",
                request.quantity
            ));
        }
        let texture_id = match request.texture_id {
            None => BASE_TEXTURE_ID,
            Some(id) if id >= 1 => id,
            Some(id) => {
                return Err(format!(
                    "textureId {id} is not a colour id (the client sends 1 in its place)"
                ))
            }
        };
        let costs: Vec<MaterialCost> = masters
            .blueprint_costs(blueprint.id)
            .filter(|costs| !costs.is_empty())
            .ok_or_else(|| match &masters.costs {
                None => {
                    "the material cost master (mysekai-blueprint-material-costs.json) is absent"
                        .to_owned()
                }
                Some(_) => format!("blueprint {} has no material cost rows", blueprint.id),
            })?
            .to_vec();
        let first_bonus = masters.first_craft_bonus;
        let materials = self.material_rows();
        let inventory = &self.doc.inventory;
        let owned = CraftOwned {
            materials: &materials,
            fixtures: &inventory.fixtures,
            canvases: &inventory.canvases,
            fixture_max_count: self
                .masters
                .possession
                .fixture_max_count(inventory.fixture_possession_level),
        };
        if owned.fixture_max_count.is_none() {
            return Err(match &self.masters.possession.fixture {
                None => {
                    "the fixture possession master (mysekai-fixture-possessions.json) is absent"
                        .to_owned()
                }
                Some(_) => format!(
                    "the fixture possession level {} is not in the fixture possession master",
                    inventory.fixture_possession_level
                ),
            });
        }
        if is_reached_possession_limit_to_craft(&blueprint, texture_id, &owned)? {
            return Err(format!(
                "IsReachedPossessionLimitToCraft: fixture {} texture {texture_id} is at its create count limit or the fixtures are at the possession limit {} (total {})",
                blueprint.craft_target_id,
                owned.fixture_max_count.unwrap_or_default(),
                super::client::inventory::fixture_total_count(owned.fixtures)
            ));
        }
        let can = can_craft_count(&blueprint, &costs, None, &owned)?;
        let most = craft_count_selector_max(can);
        if request.quantity > most {
            return Err(format!(
                "quantity {} is above the count selector's maximum {most} (craftable count {can})",
                request.quantity
            ));
        }
        if !is_enough_material_to_craft(&costs, request.quantity, true, &materials) {
            return Err(format!(
                "IsEnoughMaterialToCraft: the materials do not cover {} x blueprint {}",
                request.quantity, blueprint.id
            ));
        }
        let first = is_first_craft(&blueprint, &owned);
        let exp = if first {
            match first_bonus {
                Some(Some(bonus)) => bonus,
                Some(None) => {
                    return Err(format!(
                        "the rank obtained experience master has no {FIRST_BONUS_TYPE} row: a first craft cannot be answered"
                    ))
                }
                None => {
                    return Err("the rank obtained experience master (mysekai-rank-obtained-exps.json) is absent: a first craft cannot be answered".into())
                }
            }
        } else {
            0
        };
        let total_exp = self
            .doc
            .gamedata
            .total_exp
            .checked_add(exp)
            .ok_or("totalExp overflows")?;
        let rank = if exp > 0 {
            Some(self.masters.rank_of(total_exp).ok_or_else(|| {
                format!("no master rank table row for totalExp {total_exp}: the rank cannot follow the experience")
            })?)
        } else {
            None
        };
        // Every check passed: spend, grant, count the experience.
        let mut spent = Vec::new();
        for cost in &costs {
            let need = cost.quantity * request.quantity;
            let quantity = self
                .doc
                .delivery
                .mysekai_materials
                .entry(cost.mysekai_material_id)
                .or_insert(0);
            *quantity -= need;
            spent.push((cost.mysekai_material_id, need));
        }
        let fixtures = &mut self.doc.inventory.fixtures;
        match fixtures.iter_mut().find(|row| {
            row.mysekai_fixture_id == blueprint.craft_target_id && row.texture_id == texture_id
        }) {
            Some(row) => {
                row.quantity += request.quantity;
                row.last_obtained_at = now;
            }
            None => fixtures.push(UserMysekaiFixture {
                mysekai_fixture_id: blueprint.craft_target_id,
                texture_id,
                quantity: request.quantity,
                last_obtained_at: now,
            }),
        }
        let mut changed = vec![SECTION_MYSEKAI_MATERIALS, SECTION_FIXTURES];
        if let Some(rank) = rank {
            self.doc.gamedata.total_exp = total_exp;
            self.doc.gamedata.mysekai_rank = Some(rank);
            changed.push(SECTION_GAMEDATA);
        }
        self.commit();
        Ok((
            Crafted {
                blueprint,
                texture_id,
                spent,
                first,
                exp,
            },
            changed,
        ))
    }

    /// The sketch policies; the sections the reply carries.
    fn sketch(
        &mut self,
        request: &UserMysekaiHousingSketchRequest,
    ) -> Result<Vec<&'static str>, String> {
        let masters = &self.masters.craft;
        let blueprint = *masters
            .blueprints
            .as_ref()
            .ok_or("the blueprint master (mysekai-blueprints.json) is absent")?
            .get(&request.mysekai_blueprint_id)
            .ok_or_else(|| {
                format!(
                    "mysekaiBlueprintId {} is not a master blueprint",
                    request.mysekai_blueprint_id
                )
            })?;
        if !blueprint.is_enable_sketch {
            return Err(format!(
                "blueprint {} cannot be sketched (IsFixtureCanSketch: isEnableSketch is false)",
                blueprint.id
            ));
        }
        if self
            .owned_blueprint_rows()
            .iter()
            .any(|row| row.mysekai_blueprint_id == blueprint.id)
        {
            return Err(format!(
                "blueprint {} is already owned (HasTargetBluePrint; policies.ownedBlueprints)",
                blueprint.id
            ));
        }
        let white = match masters.white_blueprint_item {
            Some(Some(id)) => id,
            Some(None) => return Err("the items master has no white_blueprint item".into()),
            None => return Err("the items master (mysekai-items.json) is absent: the white blueprint cannot be read".into()),
        };
        let now = self.now_ms();
        let inventory = &mut self.doc.inventory;
        let row = inventory
            .items
            .iter_mut()
            .find(|row| row.mysekai_item_id == white)
            .filter(|row| row.quantity > 0)
            .ok_or_else(|| {
                format!("the user holds no white blueprint (item {white}; IsSketchAvailable needs a quantity above 0)")
            })?;
        row.quantity = (row.quantity - SKETCH_WHITE_BLUEPRINTS).max(0);
        inventory.blueprints.push(UserMysekaiBlueprint {
            mysekai_blueprint_id: blueprint.id,
            obtained_at: now,
        });
        self.commit();
        info!(
            "[server] PostUserMysekaiHousingSketchApi: blueprint {} (fixture {}) for owner {} site {} (stated, not checked); white blueprint item {white} -{SKETCH_WHITE_BLUEPRINTS} (an inference)",
            blueprint.id, blueprint.craft_target_id, request.mysekai_owner_user_id, request.mysekai_site_id
        );
        Ok(vec![SECTION_ITEMS, SECTION_BLUEPRINTS])
    }

    /// A craft request answered: the document changed and a response is
    /// recorded; the reply's tables are returned for the caller to merge at
    /// once, as the API executor merges them before the caller reads them.
    pub(super) fn craft_reply(&mut self, request: &UserMysekaiCraftRequest) -> Answer {
        let kind = match self.masters.craft.blueprint(request.blueprint_id) {
            Some(blueprint) if blueprint.craft_type == MysekaiCraftType::MysekaiCanvas => {
                ResponseKind::HomeActionCanvas
            }
            _ => ResponseKind::HomeActionCraft,
        };
        match self.craft(request) {
            Ok((crafted, changed)) => {
                let row = self.doc.inventory.fixtures.iter().find(|row| {
                    row.mysekai_fixture_id == crafted.blueprint.craft_target_id
                        && row.texture_id == crafted.texture_id
                });
                info!(
                    "[server] {}: {} x blueprint {} -> fixture {} texture {}; spent {:?}; first craft {} -> totalExp +{} (now {}, rank {:?}); userMysekaiMaterials now {:?}; fixture row now {:?}",
                    kind.name(),
                    request.quantity,
                    crafted.blueprint.id,
                    crafted.blueprint.craft_target_id,
                    crafted.texture_id,
                    crafted.spent,
                    crafted.first,
                    crafted.exp,
                    self.doc.gamedata.total_exp,
                    self.doc.gamedata.mysekai_rank,
                    self.doc.delivery.mysekai_materials,
                    row,
                );
                self.answer(kind, &changed)
            }
            Err(reason) => self.refuse(kind, reason),
        }
    }

    pub(super) fn sketch_reply(&mut self, request: &UserMysekaiHousingSketchRequest) -> Answer {
        let kind = ResponseKind::HomeActionSketch;
        match self.sketch(request) {
            Ok(changed) => self.answer(kind, &changed),
            Err(reason) => self.refuse(kind, reason),
        }
    }

    fn answer(&mut self, kind: ResponseKind, changed: &[&'static str]) -> Answer {
        self.respond(kind, false, changed);
        let response = self.responses.last().expect("the response just recorded");
        Answer {
            reply: SuiteUserReply {
                success: true,
                refusal: None,
                updated: response.inventory.clone(),
            },
            delivery: Some(response.delivery.clone()),
        }
    }

    fn refuse(&mut self, kind: ResponseKind, reason: String) -> Answer {
        warn!("[server] {} refused: {reason}", kind.name());
        Answer {
            reply: SuiteUserReply::refused(format!("{}: {reason}", kind.name())),
            delivery: None,
        }
    }

    /// The home action's craft request with the native instrument's
    /// blueprint: the craft policies, answered as a response the client
    /// receives with the next delivery.
    pub(super) fn instrument_craft(&mut self, request: &UserMysekaiCraftRequest) -> bool {
        self.craft_reply(request).reply.success
    }
}

/// A reply and the delivery sections to merge with it.
pub(super) struct Answer {
    pub(super) reply: SuiteUserReply,
    pub(super) delivery: Option<super::delivery::DeliveryUpdate>,
}

fn run(
    api: &str,
    inventory: &mut ClientMysekaiInventory,
    birthday: &mut ClientBirthdayPartyData,
    answer: impl FnOnce(&mut ServerModel) -> Answer,
) -> SuiteUserReply {
    let answer = super::with_model(|model| {
        if !model.joined {
            return Answer {
                reply: SuiteUserReply::refused(format!(
                    "{api}: the server has not joined the client yet"
                )),
                delivery: None,
            };
        }
        answer(model)
    })
    .unwrap_or_else(|| Answer {
        reply: SuiteUserReply::refused(format!("{api}: the server model is not installed")),
        delivery: None,
    });
    if let Some(delivery) = answer.delivery {
        birthday.apply(delivery);
    }
    inventory.apply(answer.reply.updated.clone());
    answer.reply
}

/// The craft endpoint ([`super::client::craft::post_craft`]).
pub(super) fn handle_craft(
    In(request): In<UserMysekaiCraftRequest>,
    mut inventory: ResMut<ClientMysekaiInventory>,
    mut birthday: ResMut<ClientBirthdayPartyData>,
) -> SuiteUserReply {
    run(
        "PostUserMysekaiCraftApi",
        &mut inventory,
        &mut birthday,
        |model| model.craft_reply(&request),
    )
}

/// The sketch endpoint ([`super::client::craft::post_sketch`]).
pub(super) fn handle_sketch(
    In(request): In<UserMysekaiHousingSketchRequest>,
    mut inventory: ResMut<ClientMysekaiInventory>,
    mut birthday: ResMut<ClientBirthdayPartyData>,
) -> SuiteUserReply {
    run(
        "PostUserMysekaiHousingSketchApi",
        &mut inventory,
        &mut birthday,
        |model| model.sketch_reply(&request),
    )
}

/// The native instrument `MOLY_CRAFT_MOCK_REQUEST`
/// (`blueprintId[:quantity[:textureId]]`); a malformed value stops loudly.
pub(crate) fn instrument_request() -> Option<UserMysekaiCraftRequest> {
    const NAME: &str = "MOLY_CRAFT_MOCK_REQUEST";
    let raw = super::instrument_env(NAME)?;
    let parts: Vec<&str> = raw.trim().split(':').map(str::trim).collect();
    let int = |text: &str| {
        text.parse::<i32>()
            .unwrap_or_else(|_| panic!("{NAME}={raw:?}: {text:?} is not an integer"))
    };
    let (blueprint_id, quantity, texture_id) = match parts.as_slice() {
        [blueprint] => (int(blueprint), 1, None),
        [blueprint, quantity] => (int(blueprint), int(quantity), None),
        [blueprint, quantity, texture] => (int(blueprint), int(quantity), Some(int(texture))),
        _ => panic!("{NAME}={raw:?} is not blueprintId[:quantity[:textureId]]"),
    };
    Some(UserMysekaiCraftRequest {
        blueprint_id,
        texture_id,
        card_id: None,
        is_special_training: None,
        quantity,
    })
}

/// The endpoints and the client copy.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ClientMysekaiInventory>();
    let craft = app.world_mut().register_system(handle_craft);
    let sketch = app.world_mut().register_system(handle_sketch);
    app.insert_resource(CraftEndpoints { craft, sketch });
    super::talk_read::install(app);
    app.add_systems(
        PreUpdate,
        super::inventory::mirror_materials.after(super::deliver),
    );
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// The panel's craft section (the replies' rules; nothing to edit here).
pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": "craftReplies",
        "title": "Craft and sketch replies",
        "delivery": "live",
        "fields": [],
        "note": "PostUserMysekaiCraftApi and PostUserMysekaiHousingSketchApi answer by the named craft and sketch policies; the owned tables, materials, possession levels and blueprint policy are edited in their own sections",
    }])
}

/// The panel's named craft policies.
pub(crate) fn schema_policies() -> Value {
    json!([
        {"name": "craft refusals", "rule": "refuse unless: a master fixture blueprint with cost rows; owned or available without possession (UserResourceFactory); inside its blueprint term when it has one (TimeUtility.IsWithinCurrentTime, MSG_CRAFT_TERM_LIMIT); quantity in 1..999 and at most the craftable count (CraftPreview count selector, GetCanCraftCount capped at the remaining fixture capacity); IsReachedPossessionLimitToCraft false (create count limit, fixture possession limit of the level); IsEnoughMaterialToCraft"},
        {"name": "craft spend", "rule": "each cost row's material loses cost.quantity x quantity (MaterialCostList need quantity)"},
        {"name": "craft grant", "rule": "the userMysekaiFixtures row of (craft target, texture) gains quantity, lastObtainedAt = server clock (CraftPreview.SetFixtureQuantityText reads that row); no texture -> 1 (an inference: the base colour)"},
        {"name": "first-craft experience", "rule": "IsFirstCraft before the grant -> totalExp + mysekaiRankObtainedExps craft_mysekai_fixture_first_bonus (CraftResultSubWindowDialog.SetupFirstCraftBonus); the rank follows totalExp. Inferences: once per request, and no experience for a craft that is not a first craft"},
        {"name": "craft types refused by name", "rule": "tool (the harvest mock's tool rows), canvas (its character material needs the cards master, which loads after the join, and a rule that is not modelled), material"},
        {"name": "sketch", "rule": "refuse unless a master blueprint with isEnableSketch (IsFixtureCanSketch), not owned (HasTargetBluePrint) and a white_blueprint item above 0 (IsSketchAvailable); spend 1 white blueprint (an inference) and add the userMysekaiBlueprints row at the server clock; the owner and site are taken as stated (visiting is not in the product)"},
    ])
}

#[cfg(test)]
mod tests {
    use super::super::client::inventory::UserMysekaiFixture;
    use super::*;

    /// A master table in its upstream form.
    fn upstream(rows: Value) -> String {
        rows.to_string()
    }

    /// The JP master's rows verbatim: blueprints 1 (limited to one craft) and
    /// 4, their cost rows, the first-craft bonus row, the level 1 fixture
    /// possession row and a term row.
    fn model() -> ServerModel {
        let mut model = super::super::tests::model();
        let masters = &mut model.masters;
        parse_blueprints(&upstream(json!([
            {"id": 1, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 1, "isEnableSketch": true, "isObtainedByConvert": true, "craftCountLimit": 1, "isAvailableWithoutPossession": false},
            {"id": 4, "mysekaiCraftType": "mysekai_fixture", "craftTargetId": 4, "isEnableSketch": true, "isObtainedByConvert": true, "isAvailableWithoutPossession": false},
        ])), masters).unwrap();
        parse_costs(&upstream(json!([
            {"id": 1, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 1, "seq": 1, "quantity": 100},
            {"id": 2, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 2, "seq": 2, "quantity": 20},
            {"id": 3, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 14, "seq": 3, "quantity": 20},
            {"id": 4, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 15, "seq": 4, "quantity": 20},
            {"id": 5, "mysekaiBlueprintId": 1, "mysekaiMaterialId": 8, "seq": 5, "quantity": 10},
            {"id": 10, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 1, "seq": 1, "quantity": 30},
            {"id": 11, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 21, "seq": 2, "quantity": 3},
            {"id": 12, "mysekaiBlueprintId": 4, "mysekaiMaterialId": 10, "seq": 3, "quantity": 3},
        ])), masters).unwrap();
        parse_terms(&upstream(json!([
            {"id": 1, "mysekaiBlueprintId": 844, "startAt": 1_759_330_800_000_i64, "endAt": 1_759_849_199_000_i64},
        ])), masters).unwrap();
        parse_rank_obtained_exps(&upstream(json!([
            {"id": 1, "mysekaiRankObtainedExpType": "craft_mysekai_fixture_first_bonus", "quantity": 1000},
        ])), masters).unwrap();
        super::super::inventory::parse_fixture_possessions(
            &upstream(json!([
                {"id": 1, "level": 1, "possessionLimit": 1000},
            ])),
            masters,
        )
        .unwrap();
        model.doc.gamedata.total_exp = 0;
        model.doc.gamedata.mysekai_rank = Some(1);
        model.doc.clock = super::super::document::Clock::Fixed {
            at_ms: 1_790_300_000_000,
        };
        model.joined = true;
        model
    }

    fn request(blueprint_id: i32, quantity: i32) -> UserMysekaiCraftRequest {
        UserMysekaiCraftRequest {
            blueprint_id,
            texture_id: None,
            card_id: None,
            is_special_training: None,
            quantity,
        }
    }

    fn materials(model: &mut ServerModel, rows: &[(i32, i32)]) {
        model.doc.delivery.mysekai_materials = rows.iter().copied().collect();
    }

    fn fixture(id: i32, quantity: i32) -> UserMysekaiFixture {
        UserMysekaiFixture {
            mysekai_fixture_id: id,
            texture_id: BASE_TEXTURE_ID,
            quantity,
            last_obtained_at: 0,
        }
    }

    #[test]
    fn a_craft_spends_cost_times_quantity_and_the_first_one_earns_the_master_bonus() {
        let mut model = model();
        materials(&mut model, &[(1, 130), (21, 13), (10, 13)]);
        let reply = model.craft_reply(&request(4, 3)).reply;
        assert!(reply.success, "{reply:?}");
        // MaterialCostList: cost.quantity x craftCount per row (30, 3, 3).
        let left = &model.doc.delivery.mysekai_materials;
        assert_eq!((left[&1], left[&21], left[&10]), (40, 4, 4));
        assert_eq!(
            model.doc.inventory.fixtures,
            vec![UserMysekaiFixture {
                last_obtained_at: model.now_ms(),
                ..fixture(4, 3)
            }]
        );
        // The master's craft_mysekai_fixture_first_bonus row.
        assert_eq!(model.doc.gamedata.total_exp, 1000);
        let rows = reply
            .updated
            .user_mysekai_fixtures
            .expect("the fixtures are carried");
        assert_eq!(rows[0].quantity, 3);
        // Not a first craft any more: no experience.
        assert!(model.craft_reply(&request(4, 1)).reply.success);
        assert_eq!(model.doc.gamedata.total_exp, 1000);
        assert_eq!(model.doc.inventory.fixtures[0].quantity, 4);
        let left = &model.doc.delivery.mysekai_materials;
        assert_eq!((left[&1], left[&21], left[&10]), (10, 1, 1));
    }

    #[test]
    fn the_client_checks_refuse_and_change_nothing() {
        let mut model = model();
        materials(
            &mut model,
            &[
                (1, 1000),
                (2, 100),
                (14, 100),
                (15, 100),
                (8, 100),
                (21, 100),
                (10, 100),
            ],
        );
        // GetCanCraftCount is 1 for a blueprint limited to one craft.
        assert!(!model.craft_reply(&request(1, 2)).reply.success);
        assert!(model.craft_reply(&request(1, 1)).reply.success);
        // IsCreateCountLimit: the one allowed piece is owned.
        assert!(!model.craft_reply(&request(1, 1)).reply.success);
        // The count selector's bounds.
        assert!(!model.craft_reply(&request(4, 0)).reply.success);
        assert!(!model.craft_reply(&request(4, 1000)).reply.success);
        // The level 1 fixture limit (1000): one piece of room left.
        model.doc.inventory.fixtures.push(fixture(99, 998));
        let before = model.doc.clone();
        assert!(!model.craft_reply(&request(4, 2)).reply.success);
        assert_eq!(model.doc, before);
        assert!(model.craft_reply(&request(4, 1)).reply.success);
        assert!(!model.craft_reply(&request(4, 1)).reply.success);
        // IsEnoughMaterialToCraft.
        model
            .doc
            .inventory
            .fixtures
            .retain(|row| row.mysekai_fixture_id != 99);
        materials(&mut model, &[(1, 29), (21, 100), (10, 100)]);
        let before = model.doc.clone();
        assert!(!model.craft_reply(&request(4, 1)).reply.success);
        assert_eq!(model.doc, before);
    }
}
