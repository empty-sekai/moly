//! The craft loop's two requests: `PostUserMysekaiCraftApi` (the craft screen
//! and the canvas screen, `UserMysekaiCraftRequest`) and
//! `PostUserMysekaiHousingSketchApi` (`UserMysekaiHousingSketchRequest`).
//! Both reply with `SuiteUser`, the tables they changed; the client merges
//! them before it reads its copies. The client sends them through
//! the client's `post_craft` and `post_sketch` request helpers;
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
//! *Tool craft*: a `mysekai_tool` blueprint passes the same checks (the
//! possession limit check is false for a tool: `IsReachedPossessionLimitToCraft`
//! reads fixtures and canvases only) and, in place of the fixture grant, adds
//! the quantity to the `userMysekaiTools` row of its craft target by the tool
//! craft policy ([`super::tools`]); the reply carries that section, and the
//! client's tool copy takes it at once. No texture is read (the craft screen
//! sends none for a tool, `GetSelectedMysekaiColorId`), and no experience is
//! granted (`IsFirstCraft` is false for a tool).
//!
//! *Refused by name*: a canvas blueprint (its character material needs the
//! card's character and rarity from the cards master, which loads after the
//! join; with it in, the character material rule is still not modelled), and
//! the material type (no master blueprint has it).
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

use bevy::prelude::*;
use serde_json::{json, Value};

pub(crate) use moly_masters::craft::{
    can_craft_count, craft_count_selector_max, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft, is_within_time, CraftOwned, MasterBlueprint, MaterialCost,
    CRAFT_COUNT_MAX, CRAFT_COUNT_MIN,
};
pub(crate) use moly_protocol::api::{
    MysekaiCraftType, SuiteUserReply, UserMysekaiCraftRequest, UserMysekaiHousingSketchRequest,
};
pub(crate) use moly_protocol::rows::{UserMysekaiBlueprint, UserMysekaiFixture};
use super::delivery::SECTION_MYSEKAI_MATERIALS;
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

/// `cards`: card id -> the card's character and rarity.
pub(crate) fn parse_cards(text: &str, masters: &mut Masters) -> Result<(), String> {
    let cards = moly_masters::craft::parse_cards(&moly_assets::json::master::rows(text)?)?;
    masters.cards = super::Deferred::Ready(cards);
    Ok(())
}

/// `mysekaiBlueprints`.
pub(crate) fn parse_blueprints(text: &str, masters: &mut Masters) -> Result<(), String> {
    let blueprints =
        moly_masters::craft::parse_blueprints(&table_rows(text, "mysekaiBlueprints")?)?;
    masters.craft.blueprints = Some(blueprints);
    Ok(())
}

/// `mysekaiBlueprintMysekaiMaterialCosts`: the rows per blueprint.
pub(crate) fn parse_costs(text: &str, masters: &mut Masters) -> Result<(), String> {
    let costs = moly_masters::craft::parse_costs(&table_rows(
        text,
        "mysekaiBlueprintMysekaiMaterialCosts",
    )?)?;
    masters.craft.costs = Some(costs);
    Ok(())
}

/// `mysekaiBlueprintTerms`.
pub(crate) fn parse_terms(text: &str, masters: &mut Masters) -> Result<(), String> {
    let terms = moly_masters::craft::parse_terms(&table_rows(text, "mysekaiBlueprintTerms")?)?;
    masters.craft.terms = Some(terms);
    Ok(())
}

/// `mysekaiRankObtainedExps`: the first-craft bonus row.
pub(crate) fn parse_rank_obtained_exps(text: &str, masters: &mut Masters) -> Result<(), String> {
    let bonus = moly_masters::craft::parse_rank_obtained_exps(&table_rows(
        text,
        "mysekaiRankObtainedExps",
    )?)?;
    masters.craft.first_craft_bonus = Some(bonus);
    Ok(())
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

/// What a craft request did, for its log line.
struct Crafted {
    blueprint: MasterBlueprint,
    /// The fixture texture (`None` for a tool).
    texture_id: Option<i32>,
    spent: Vec<(i32, i32)>,
    first: bool,
    exp: i32,
}

impl ServerModel {
    /// The craft policies; the sections the reply carries.
    fn craft(
        &mut self,
        request: &UserMysekaiCraftRequest,
        masters: &Masters,
    ) -> Result<(Crafted, Vec<&'static str>), String> {
        let craft_masters = &masters.craft;
        let blueprints = craft_masters.blueprints.as_ref().ok_or(
            "the blueprint master (mysekai-blueprints.json) is absent: the blueprint cannot be read",
        )?;
        let blueprint = *blueprints.get(&request.blueprint_id).ok_or_else(|| {
            format!(
                "blueprintId {} is not a master blueprint",
                request.blueprint_id
            )
        })?;
        let tool = blueprint.craft_type == MysekaiCraftType::MysekaiTool;
        match blueprint.craft_type {
            MysekaiCraftType::MysekaiFixture | MysekaiCraftType::MysekaiTool => {}
            MysekaiCraftType::MysekaiCanvas => {
                let reason = match masters.cards.get(
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
        let terms = craft_masters.terms.as_ref().ok_or(
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
            _ if tool => BASE_TEXTURE_ID,
            None => BASE_TEXTURE_ID,
            Some(id) if id >= 1 => id,
            Some(id) => {
                return Err(format!(
                    "textureId {id} is not a colour id (the client sends 1 in its place)"
                ))
            }
        };
        let tool_max = if tool {
            Some(self.crafted_tool_max(blueprint.craft_target_id, masters)?)
        } else {
            None
        };
        let costs: Vec<MaterialCost> = craft_masters
            .blueprint_costs(blueprint.id)
            .filter(|costs| !costs.is_empty())
            .ok_or_else(|| match &craft_masters.costs {
                None => {
                    "the material cost master (mysekai-blueprint-material-costs.json) is absent"
                        .to_owned()
                }
                Some(_) => format!("blueprint {} has no material cost rows", blueprint.id),
            })?
            .to_vec();
        let first_bonus = craft_masters.first_craft_bonus;
        let materials = self.material_rows();
        let inventory = &self.doc.inventory;
        let owned = CraftOwned {
            materials: &materials,
            fixtures: &inventory.fixtures,
            canvases: &inventory.canvases,
            fixture_max_count: masters
                .possession
                .fixture_max_count(inventory.fixture_possession_level),
        };
        if owned.fixture_max_count.is_none() {
            return Err(match &masters.possession.fixture {
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
                moly_masters::inventory::fixture_total_count(owned.fixtures)
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
            Some(masters.rank_of(total_exp).ok_or_else(|| {
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
        let mut changed = vec![SECTION_MYSEKAI_MATERIALS];
        if let Some(max) = tool_max {
            self.grant_crafted_tool(blueprint.craft_target_id, request.quantity, max, now);
            changed.push(super::tools::SECTION);
        } else {
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
            changed.push(SECTION_FIXTURES);
        }
        if let Some(rank) = rank {
            self.doc.gamedata.total_exp = total_exp;
            self.doc.gamedata.mysekai_rank = Some(rank);
            changed.push(SECTION_GAMEDATA);
        }
        self.commit();
        Ok((
            Crafted {
                blueprint,
                texture_id: (!tool).then_some(texture_id),
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
        masters: &Masters,
    ) -> Result<Vec<&'static str>, String> {
        let craft_masters = &masters.craft;
        let blueprint = *craft_masters
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
            .owned_blueprint_rows(masters)
            .iter()
            .any(|row| row.mysekai_blueprint_id == blueprint.id)
        {
            return Err(format!(
                "blueprint {} is already owned (HasTargetBluePrint; policies.ownedBlueprints)",
                blueprint.id
            ));
        }
        let white = match craft_masters.white_blueprint_item {
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
    pub(crate) fn craft_reply(
        &mut self,
        request: &UserMysekaiCraftRequest,
        masters: &Masters,
    ) -> Answer {
        let kind = match masters.craft.blueprint(request.blueprint_id) {
            Some(blueprint) if blueprint.craft_type == MysekaiCraftType::MysekaiCanvas => {
                ResponseKind::HomeActionCanvas
            }
            _ => ResponseKind::HomeActionCraft,
        };
        match self.craft(request, masters) {
            Ok((crafted, changed)) => {
                let target = crafted.blueprint.craft_target_id;
                let row = match crafted.texture_id {
                    Some(texture_id) => format!(
                        "fixture {target} texture {texture_id}; fixture row now {:?}",
                        self.doc.inventory.fixtures.iter().find(|row| {
                            row.mysekai_fixture_id == target && row.texture_id == texture_id
                        })
                    ),
                    None => format!(
                        "tool {target}; tool row now {:?} ({} tool rows)",
                        self.doc
                            .tools
                            .iter()
                            .flatten()
                            .find(|row| row.mysekai_tool_id == target),
                        self.doc.tools.as_ref().map_or(0, Vec::len)
                    ),
                };
                info!(
                    "[server] {}: {} x blueprint {} -> {row}; spent {:?}; first craft {} -> totalExp +{} (now {}, rank {:?}); userMysekaiMaterials now {:?}",
                    kind.name(),
                    request.quantity,
                    crafted.blueprint.id,
                    crafted.spent,
                    crafted.first,
                    crafted.exp,
                    self.doc.gamedata.total_exp,
                    self.doc.gamedata.mysekai_rank,
                    self.doc.delivery.mysekai_materials,
                );
                info!(
                    "[server] craft_done {}",
                    serde_json::json!({
                        "blueprint_id": crafted.blueprint.id,
                        "craft_type": crafted.blueprint.craft_type.name(),
                        "target_id": target,
                        "texture_id": crafted.texture_id,
                        "quantity": request.quantity,
                        "spent": crafted.spent.iter().map(|(material, quantity)| serde_json::json!({ "material_id": material, "quantity": quantity })).collect::<Vec<_>>(),
                        "first": crafted.first,
                        "exp": crafted.exp,
                    }),
                );
                self.answer(kind, &changed, masters)
            }
            Err(reason) => {
                info!(
                    "[server] craft_refused {}",
                    serde_json::json!({ "blueprint_id": request.blueprint_id, "quantity": request.quantity, "reason": reason }),
                );
                self.refuse(kind, reason)
            }
        }
    }

    pub(crate) fn sketch_reply(
        &mut self,
        request: &UserMysekaiHousingSketchRequest,
        masters: &Masters,
    ) -> Answer {
        let kind = ResponseKind::HomeActionSketch;
        match self.sketch(request, masters) {
            Ok(changed) => self.answer(kind, &changed, masters),
            Err(reason) => self.refuse(kind, reason),
        }
    }

    fn answer(&mut self, kind: ResponseKind, changed: &[&'static str], masters: &Masters) -> Answer {
        self.respond(kind, false, changed, masters);
        let response = self.responses.last().expect("the response just recorded");
        Answer {
            reply: SuiteUserReply {
                success: true,
                refusal: None,
                updated: response.sections.inventory.clone(),
            },
            delivery: Some(response.sections.delivery.clone()),
            tools: response.sections.tools.clone(),
        }
    }

    fn refuse(&mut self, kind: ResponseKind, reason: String) -> Answer {
        warn!("[server] {} refused: {reason}", kind.name());
        Answer {
            reply: SuiteUserReply::refused(format!("{}: {reason}", kind.name())),
            delivery: None,
            tools: None,
        }
    }

    /// The home action's craft request with the native instrument's
    /// blueprint: the craft policies, answered as a response the client
    /// receives with the next delivery.
    pub(super) fn instrument_craft(
        &mut self,
        request: &UserMysekaiCraftRequest,
        masters: &Masters,
    ) -> bool {
        self.craft_reply(request, masters).reply.success
    }
}

/// A reply and the delivery sections to merge with it.
pub(crate) struct Answer {
    pub(crate) reply: SuiteUserReply,
    pub(crate) delivery: Option<super::delivery::DeliveryUpdate>,
    /// `userMysekaiTools` when the reply carries it.
    pub(crate) tools: Option<Vec<super::tools::UserMysekaiTool>>,
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
        {"name": "craft refusals", "rule": "refuse unless: a master fixture or tool blueprint with cost rows; owned or available without possession (UserResourceFactory); inside its blueprint term when it has one (TimeUtility.IsWithinCurrentTime, MSG_CRAFT_TERM_LIMIT); quantity in 1..999 and at most the craftable count (CraftPreview count selector, GetCanCraftCount capped at the remaining fixture capacity); IsReachedPossessionLimitToCraft false (create count limit, fixture possession limit of the level); IsEnoughMaterialToCraft"},
        {"name": "craft spend", "rule": "each cost row's material loses cost.quantity x quantity (MaterialCostList need quantity)"},
        {"name": "craft grant", "rule": "the userMysekaiFixtures row of (craft target, texture) gains quantity, lastObtainedAt = server clock (CraftPreview.SetFixtureQuantityText reads that row); no texture -> 1 (an inference: the base colour)"},
        {"name": "first-craft experience", "rule": "IsFirstCraft before the grant -> totalExp + mysekaiRankObtainedExps craft_mysekai_fixture_first_bonus (CraftResultSubWindowDialog.SetupFirstCraftBonus); the rank follows totalExp. Inferences: once per request, and no experience for a craft that is not a first craft"},
        {"name": "tool craft", "rule": "a mysekai_tool blueprint adds the quantity to the craft target's userMysekaiTools row (the tool craft policy); no texture, no experience"},
        {"name": "craft types refused by name", "rule": "canvas (its character material needs the cards master, which loads after the join, and a rule that is not modelled), material"},
        {"name": "sketch", "rule": "refuse unless a master blueprint with isEnableSketch (IsFixtureCanSketch), not owned (HasTargetBluePrint) and a white_blueprint item above 0 (IsSketchAvailable); spend 1 white blueprint (an inference) and add the userMysekaiBlueprints row at the server clock; the owner and site are taken as stated (visiting is not in the product)"},
    ])
}
