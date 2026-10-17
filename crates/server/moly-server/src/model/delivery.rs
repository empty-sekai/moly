//! The birthday-party delivery section of the server document and the two
//! requests the delivery site makes (`PutUserMysekaiBirthdayPartyDeliveryApi`
//! and `PutUserMysekaiBirthdayPartyGatherApi`).
//!
//! **Document keys** (the source's user-data keys, `SuiteUser`):
//! - `userBirthdayParties`: `UserBirthdayParty` rows (`birthdayPartyId`,
//!   `deliveryTotalPoint`, `droppedMysekaiMaterialCount`,
//!   `obtainedMysekaiMaterialCount`; the one user's `userId` is left out).
//! - `userMaterials`: `UserMaterial` (`materialId`, `quantity`): the
//!   delivery item's have-quantity (`UserDataUtility.GetHaveQuantity(id,
//!   material)`).
//! - `userMysekaiMaterials`: `UserMysekaiMaterial` (`mysekaiMaterialId`,
//!   `quantity`): the reward drops' material.
//! - `userCards`: `UserCard` rows reduced to `cardId`; read when
//!   `policies.ownedCards` is `stated`.
//! - `userHonors`: `UserHonor` (`honorId`, `level`, `obtainedAt`).
//!
//! The two master configs the delivery reads with
//! `MasterDataManager.GetMasterConfigToInt`
//! (`birthday_party_delivery_base_point`,
//! `birthday_party_delivery_reward_drop_upper_limit`) are master data: the
//! server model reads them from the configs master (`configs.json`) and the
//! join hands them to the client's copy, standing in for the master download.
//! A document written before this was known may carry a `masterConfigs` key;
//! it is read and ignored by name. A runtime root without the configs master
//! (the CN roots have no source for it) reads the two values from
//! `policies.masterConfigsStandIn` instead, named in the missing-master list.
//!
//! The mock's own keys: `policies.ownedCards`,
//! `policies.birthdayPlantRefreshPoints` and `policies.masterConfigsStandIn`.
//! Every key is optional when a
//! document is read (a document written before this section had none) and
//! is always written.
//!
//! **What stays client code**: the accrual, the reward loop count, the drop
//! limit, the drops and the gathering, and the master tables the client reads
//! (`birthdayPartyDeliveryRewards`, `birthdayPartyDeliveryPointBonuses`).
//!
//! **Named policies** (the server's rules are not on disk):
//! - *New party row*: a party in session at the server clock with no row
//!   gets `deliveryTotalPoint` 0, `droppedMysekaiMaterialCount`
//!   [`NEW_PARTY_DROPPED`] (unclaimed drops, so an arrival shows the pickup)
//!   and `obtainedMysekaiMaterialCount` 0.
//! - *Delivery item stock*: the delivery item of a party in session with no
//!   `userMaterials` row gets [`DELIVERY_ITEM_STOCK`].
//! - *Owned cards*: `everyPointBonusCard` (the user owns every card a point
//!   bonus row names) or `stated` (the `userCards` rows).
//! - *Delivery reply*: the server spends the sent count (capped at the
//!   have-quantity), adds the points the client's own accrual law gives it,
//!   counts the reward loops the points newly reach, keeps as dropped those
//!   that fit under the drop limit beside the drops already counted and
//!   counts the rest as obtained (they flew to the player); the obtained ones
//!   join the reward material.
//! - *Gather reply*: each gathered drop leaves the dropped count, joins the
//!   obtained count and the reward material.
//! - *Total rewards*: counted against `obtainedMysekaiMaterialCount`. The
//!   client shows a total reward row as obtained when its requirement is at
//!   most the row's `obtainedMysekaiMaterialCount`
//!   (`MysekaiBirthdayRewardListDialog.Setup`), the delivery checks that its
//!   obtained count grows by exactly the drops that flew to the player, and
//!   the gather reply carries total rewards too. So a reply grants every
//!   total reward row of the party whose requirement its obtained count
//!   reaches for the first time, with the contents of the row's box: an honor
//!   raises the `userHonors` row to the box level, a material or mysekai
//!   material adds to its quantity, and the other resource types are carried
//!   in the reply only (the document does not track them).
//! - *Birthday plant refresh* (an inference, not a read): the delivery
//!   reply's `isRefreshed` makes the client show the refresh-birthday-plant
//!   dialog and destroy the site's harvest fixtures
//!   (`ExecuteHarvestSiteRefresh`), and the server-only master
//!   `birthdayPartyMysekaiSiteHarvestFixtureRepeatRefreshes` has requirement
//!   10000 (seq 1) for every party on disk. The reply says `isRefreshed` when
//!   the party's `deliveryTotalPoint` passes a multiple of
//!   `policies.birthdayPlantRefreshPoints` (0: never). The gather reply has
//!   no such field.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::document::int32;
use super::{Masters, ResponseKind, ServerModel, Tables};

pub(crate) use moly_masters::delivery::DeliveryTables;
pub(crate) use moly_protocol::api::{DeliveryResponse, GatherContent, GatherResponse};
pub(crate) use moly_protocol::json::{
    honor_value, parse_cards, parse_honors, parse_parties, parse_quantities, party_value,
    quantities_value, SECTION_BIRTHDAY_PARTIES, SECTION_CARDS, SECTION_HONORS, SECTION_MASTER_CONFIGS,
    SECTION_MATERIALS, SECTION_MYSEKAI_MATERIALS,
};
pub(crate) use moly_protocol::rows::{BirthdayPartyRow, DeliveryUpdate, HonorRow, Reward};

/// The sections of this part of the document a response carries.
pub(crate) const SECTIONS: [&str; 6] = [
    SECTION_BIRTHDAY_PARTIES,
    SECTION_MATERIALS,
    SECTION_MYSEKAI_MATERIALS,
    SECTION_CARDS,
    SECTION_HONORS,
    SECTION_MASTER_CONFIGS,
];

pub(crate) const CONFIG_BASE_POINT: &str = "birthday_party_delivery_base_point";
pub(crate) const CONFIG_DROP_UPPER_LIMIT: &str = "birthday_party_delivery_reward_drop_upper_limit";

/// `ResourceType` names of a reward row.
pub(crate) const HONOR: &str = "honor";
const MATERIAL: &str = "material";
const MYSEKAI_MATERIAL: &str = "mysekai_material";

/// The new party row policy's `droppedMysekaiMaterialCount`.
pub(crate) const NEW_PARTY_DROPPED: i32 = 3;
/// The delivery item stock policy's have-quantity.
pub(crate) const DELIVERY_ITEM_STOCK: i32 = 300;
/// `policies.birthdayPlantRefreshPoints` of a document that states none (the
/// repeat-refresh requirement of every party row on disk).
pub(crate) const DEFAULT_PLANT_REFRESH_POINTS: i32 = 10_000;
/// `policies.masterConfigsStandIn` of a document that states none: the JP
/// configs master's values of the two keys. Read only when the runtime root
/// has no configs master.
pub(crate) const DEFAULT_CONFIGS_STAND_IN: [(&str, i32); 2] =
    [(CONFIG_BASE_POINT, 100), (CONFIG_DROP_UPPER_LIMIT, 25)];
const POLICY_CONFIGS_STAND_IN: &str = "masterConfigsStandIn";

pub(crate) const OWNED_EVERY_BONUS_CARD: &str = "everyPointBonusCard";
pub(crate) const OWNED_STATED: &str = "stated";

/// `policies.ownedCards`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OwnedCards {
    EveryPointBonusCard,
    Stated,
}

/// The delivery part of the server document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeliveryDoc {
    pub(crate) parties: Vec<BirthdayPartyRow>,
    /// `userMaterials`: material id -> quantity.
    pub(crate) materials: BTreeMap<i32, i32>,
    /// `userMysekaiMaterials`: mysekai material id -> quantity.
    pub(crate) mysekai_materials: BTreeMap<i32, i32>,
    pub(crate) owned_cards: OwnedCards,
    /// `userCards` card ids (read when the owned cards policy is stated).
    pub(crate) cards: Vec<i64>,
    pub(crate) honors: Vec<HonorRow>,
    pub(crate) plant_refresh_points: i32,
    /// `policies.masterConfigsStandIn`: the two master configs for a runtime
    /// root without the configs master.
    pub(crate) configs_stand_in: BTreeMap<String, i32>,
}

impl Default for DeliveryDoc {
    fn default() -> Self {
        Self {
            parties: Vec::new(),
            materials: BTreeMap::new(),
            mysekai_materials: BTreeMap::new(),
            owned_cards: OwnedCards::EveryPointBonusCard,
            cards: Vec::new(),
            honors: Vec::new(),
            plant_refresh_points: DEFAULT_PLANT_REFRESH_POINTS,
            configs_stand_in: DEFAULT_CONFIGS_STAND_IN
                .iter()
                .map(|&(key, value)| (key.to_owned(), value))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Reading and writing the section
// ---------------------------------------------------------------------------

/// The document keys this section reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 6] = SECTIONS;
/// The keys this section reads under `policies`.
pub(crate) const POLICY_KEYS: [&str; 3] =
    ["ownedCards", "birthdayPlantRefreshPoints", POLICY_CONFIGS_STAND_IN];

pub(crate) fn parse_owned_cards(value: &Value) -> Result<OwnedCards, String> {
    match value.as_str() {
        Some(OWNED_EVERY_BONUS_CARD) => Ok(OwnedCards::EveryPointBonusCard),
        Some(OWNED_STATED) => Ok(OwnedCards::Stated),
        _ => Err(format!(
            "policies.ownedCards is neither \"{OWNED_EVERY_BONUS_CARD}\" (the user owns every card a point bonus row names) nor \"{OWNED_STATED}\" (the userCards rows)"
        )),
    }
}

impl DeliveryDoc {
    /// The section from a schemaVersion 2 document: each key optional (the
    /// default when absent), each present one strict.
    pub(crate) fn parse(
        doc: &Map<String, Value>,
        policies: &Map<String, Value>,
    ) -> Result<Self, String> {
        let mut out = Self::default();
        if let Some(value) = doc.get(SECTION_BIRTHDAY_PARTIES) {
            out.parties = parse_parties(value)?;
        }
        if let Some(value) = doc.get(SECTION_MATERIALS) {
            out.materials = parse_quantities(value, SECTION_MATERIALS, "materialId")?;
        }
        if let Some(value) = doc.get(SECTION_MYSEKAI_MATERIALS) {
            out.mysekai_materials =
                parse_quantities(value, SECTION_MYSEKAI_MATERIALS, "mysekaiMaterialId")?;
        }
        if let Some(value) = doc.get(SECTION_CARDS) {
            out.cards = parse_cards(value)?;
        }
        if let Some(value) = doc.get(SECTION_HONORS) {
            out.honors = parse_honors(value)?;
        }
        if doc.contains_key(SECTION_MASTER_CONFIGS) {
            warn!("[server] the document's {SECTION_MASTER_CONFIGS} is ignored: {CONFIG_BASE_POINT} and {CONFIG_DROP_UPPER_LIMIT} are master data (configs.json)");
        }
        if let Some(value) = policies.get("ownedCards") {
            out.owned_cards = parse_owned_cards(value)?;
        }
        if policies.contains_key("birthdayPlantRefreshPoints") {
            out.plant_refresh_points = int32(policies, "birthdayPlantRefreshPoints", "policies")?;
        }
        if let Some(value) = policies.get(POLICY_CONFIGS_STAND_IN) {
            out.configs_stand_in = parse_configs_stand_in(value)?;
        }
        out.check_structure()?;
        Ok(out)
    }

    pub(crate) fn check_structure(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for (i, row) in self.parties.iter().enumerate() {
            let at = format!("{SECTION_BIRTHDAY_PARTIES}[{i}]");
            if row.birthday_party_id < 1 {
                return Err(format!(
                    "{at}.birthdayPartyId = {} is below 1",
                    row.birthday_party_id
                ));
            }
            if !ids.insert(row.birthday_party_id) {
                return Err(format!("{at} repeats party {}", row.birthday_party_id));
            }
            for (name, value) in [
                ("deliveryTotalPoint", row.delivery_total_point),
                (
                    "droppedMysekaiMaterialCount",
                    row.dropped_mysekai_material_count,
                ),
                (
                    "obtainedMysekaiMaterialCount",
                    row.obtained_mysekai_material_count,
                ),
            ] {
                if value < 0 {
                    return Err(format!("{at}.{name} = {value} is negative"));
                }
            }
        }
        for (at, map) in [
            (SECTION_MATERIALS, &self.materials),
            (SECTION_MYSEKAI_MATERIALS, &self.mysekai_materials),
        ] {
            for (id, quantity) in map {
                if *id < 1 {
                    return Err(format!("{at}: material id {id} is below 1"));
                }
                if *quantity < 0 {
                    return Err(format!(
                        "{at}: material {id} has a negative quantity {quantity}"
                    ));
                }
            }
        }
        let mut cards = BTreeSet::new();
        for (i, card) in self.cards.iter().enumerate() {
            if !cards.insert(*card) {
                return Err(format!("{SECTION_CARDS}[{i}] repeats card {card}"));
            }
        }
        let mut honors = BTreeSet::new();
        for (i, row) in self.honors.iter().enumerate() {
            if row.level < 1 {
                return Err(format!(
                    "{SECTION_HONORS}[{i}].level = {} is below 1",
                    row.level
                ));
            }
            if !honors.insert(row.honor_id) {
                return Err(format!(
                    "{SECTION_HONORS}[{i}] repeats honor {}",
                    row.honor_id
                ));
            }
        }
        if self.plant_refresh_points < 0 {
            return Err(format!(
                "policies.birthdayPlantRefreshPoints = {} is negative (0 turns the refresh off)",
                self.plant_refresh_points
            ));
        }
        Ok(())
    }

    pub(crate) fn parties_value(&self) -> Value {
        Value::Array(self.parties.iter().map(|row| party_value(*row)).collect())
    }

    /// Writes the section's keys into the document object and its policies.
    pub(crate) fn write(&self, doc: &mut Map<String, Value>, policies: &mut Map<String, Value>) {
        doc.insert(SECTION_BIRTHDAY_PARTIES.into(), self.parties_value());
        doc.insert(
            SECTION_MATERIALS.into(),
            quantities_value(&self.materials, "materialId"),
        );
        doc.insert(
            SECTION_MYSEKAI_MATERIALS.into(),
            quantities_value(&self.mysekai_materials, "mysekaiMaterialId"),
        );
        doc.insert(
            SECTION_CARDS.into(),
            Value::Array(
                self.cards
                    .iter()
                    .map(|card| json!({"cardId": card}))
                    .collect(),
            ),
        );
        doc.insert(
            SECTION_HONORS.into(),
            Value::Array(self.honors.iter().map(|row| honor_value(*row)).collect()),
        );
        policies.insert(
            "ownedCards".into(),
            json!(match self.owned_cards {
                OwnedCards::EveryPointBonusCard => OWNED_EVERY_BONUS_CARD,
                OwnedCards::Stated => OWNED_STATED,
            }),
        );
        policies.insert(
            "birthdayPlantRefreshPoints".into(),
            json!(self.plant_refresh_points),
        );
        policies.insert(POLICY_CONFIGS_STAND_IN.into(), json!(self.configs_stand_in));
    }

    fn party_mut(&mut self, id: i32) -> Option<&mut BirthdayPartyRow> {
        self.parties
            .iter_mut()
            .find(|row| row.birthday_party_id == id)
    }
}

/// `policies.masterConfigsStandIn`: an object of the two master config keys,
/// each an int32; a key it leaves out takes its default.
fn parse_configs_stand_in(value: &Value) -> Result<BTreeMap<String, i32>, String> {
    let at = format!("policies.{POLICY_CONFIGS_STAND_IN}");
    let object = value
        .as_object()
        .ok_or_else(|| format!("{at} is not an object"))?;
    if let Some(key) = object
        .keys()
        .find(|key| !DEFAULT_CONFIGS_STAND_IN.iter().any(|(name, _)| name == key))
    {
        return Err(format!("{at} has {key}, which is not one of the delivery's master configs"));
    }
    DEFAULT_CONFIGS_STAND_IN
        .iter()
        .map(|&(key, default)| {
            let value = if object.contains_key(key) { int32(object, key, &at)? } else { default };
            Ok((key.to_owned(), value))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Master tables (the total reward boxes)
// ---------------------------------------------------------------------------

/// The integer field of a table row, named by the row's id.
fn table_int(row: &Value, table: &str, key: &str) -> Result<i64, String> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{table} row {:?} has no integer {key}", row.get("id")))
}

/// Logs the total reward box rows once they arrive.
fn log_boxes(boxes: &[(i64, Reward)], table: &str) {
    info!(
        "[server] total reward boxes ({table}): {} box rows",
        boxes.len()
    );
}

/// The purpose of the boxes a total reward row names.
const TOTAL_REWARD_BOX_PURPOSE: &str = "birthday_party_delivery_total_reward";

/// The box rows of the total reward purpose: what each granted box holds,
/// master order.
fn total_reward_boxes<'a>(
    rows: impl Iterator<Item = &'a Value>,
    table: &str,
) -> Result<Vec<(i64, Reward)>, String> {
    rows.filter(|row| row["resourceBoxPurpose"].as_str() == Some(TOTAL_REWARD_BOX_PURPOSE))
        .map(|row| {
            Ok((
                table_int(row, table, "resourceBoxId")?,
                Reward {
                    resource_type: row
                        .get("resourceType")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("a {table} row has no resourceType"))?
                        .to_owned(),
                    resource_id: table_int(row, table, "resourceId")?,
                    quantity: table_int(row, table, "resourceQuantity")? as i32,
                    level: row.get("resourceLevel").and_then(Value::as_i64),
                },
            ))
        })
        .collect()
}

/// `resourceBoxDetails`: a master that carries the box rows as a table of
/// their own.
pub(crate) fn parse_box_details(text: &str, masters: &mut Masters) -> Result<(), String> {
    let rows = moly_assets::json::master::rows(text)?;
    let boxes = total_reward_boxes(rows.iter(), "resourceBoxDetails")?;
    log_boxes(&boxes, "resourceBoxDetails");
    masters.reward_boxes = super::Deferred::Ready(boxes);
    Ok(())
}

/// `resourceBoxes`: a master that nests each box's rows in its `details`,
/// read box by box in master order.
pub(crate) fn parse_nested_boxes(text: &str, masters: &mut Masters) -> Result<(), String> {
    let boxes = moly_assets::json::master::rows(text)?;
    let mut details = Vec::new();
    for resource_box in &boxes {
        if resource_box["resourceBoxPurpose"].as_str() != Some(TOTAL_REWARD_BOX_PURPOSE) {
            continue;
        }
        match resource_box.get("details") {
            None | Some(Value::Null) => {}
            Some(Value::Array(rows)) => details.extend(rows),
            Some(_) => {
                return Err(format!(
                    "resourceBoxes box {:?}: details is not a list",
                    resource_box.get("id")
                ))
            }
        }
    }
    let boxes = total_reward_boxes(details.into_iter(), "resourceBoxes[].details")?;
    log_boxes(&boxes, "resourceBoxes[].details");
    masters.reward_boxes = super::Deferred::Ready(boxes);
    Ok(())
}

// ---------------------------------------------------------------------------
// Client copies
// ---------------------------------------------------------------------------

/// A party in session as the server sees its master row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PartyMaster {
    pub(crate) birthday_party_id: i32,
    pub(crate) delivery_item_material_id: i32,
    pub(crate) delivery_reward_mysekai_material_id: i32,
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

impl ServerModel {
    /// The owned card ids the policy gives.
    fn owned_cards(&self, masters: &Tables<'_>) -> BTreeSet<i64> {
        match self.doc.delivery.owned_cards {
            OwnedCards::Stated => self.doc.delivery.cards.iter().copied().collect(),
            OwnedCards::EveryPointBonusCard => masters
                .delivery()
                .map(DeliveryTables::bonus_cards)
                .unwrap_or_default(),
        }
    }

    /// The full delivery sections named in `sections`.
    pub(super) fn delivery_update(&self, sections: &[String], masters: &Tables<'_>) -> DeliveryUpdate {
        let has = |name: &str| sections.iter().any(|section| section == name);
        let doc = &self.doc.delivery;
        DeliveryUpdate {
            parties: has(SECTION_BIRTHDAY_PARTIES).then(|| doc.parties.clone()),
            materials: has(SECTION_MATERIALS).then(|| doc.materials.clone()),
            mysekai_materials: has(SECTION_MYSEKAI_MATERIALS)
                .then(|| doc.mysekai_materials.clone()),
            cards: has(SECTION_CARDS).then(|| self.owned_cards(masters)),
            honors: has(SECTION_HONORS).then(|| doc.honors.clone()),
            configs: has(SECTION_MASTER_CONFIGS)
                .then(|| self.delivery_configs(masters))
                .flatten(),
        }
    }

    /// The delivery's master configs the configs master holds as integers;
    /// the document's stand-in when the runtime root has no configs master.
    fn delivery_configs(&self, masters: &Tables<'_>) -> Option<BTreeMap<String, i32>> {
        let Some(configs) = masters.configs.as_ref() else {
            return Some(self.doc.delivery.configs_stand_in.clone());
        };
        Some(
            [CONFIG_BASE_POINT, CONFIG_DROP_UPPER_LIMIT]
                .into_iter()
                .filter_map(|key| {
                    let value = configs.get(key)?.trim().parse::<i32>().ok()?;
                    Some((key.to_owned(), value))
                })
                .collect(),
        )
    }

    /// One delivery master config (`GetMasterConfigToInt`), refused by name.
    /// Without the configs master (requests come after the masters resolve)
    /// the document's stand-in answers.
    fn master_config_int(&self, key: &str, masters: &Tables<'_>) -> Result<i32, String> {
        let Some(configs) = masters.configs.as_ref() else {
            return self.doc.delivery.configs_stand_in.get(key).copied().ok_or_else(|| {
                format!("policies.{POLICY_CONFIGS_STAND_IN} has no {key}")
            });
        };
        let raw = configs
            .get(key)
            .ok_or_else(|| format!("the configs master has no {key}"))?;
        raw.trim()
            .parse::<i32>()
            .map_err(|_| format!("the configs master's {key} = {raw:?} is not an integer"))
    }

    /// The new party row and delivery item stock policies for the parties in
    /// session. Returns the sections it changed.
    pub(crate) fn seat_parties(&mut self, in_session: &[PartyMaster]) -> Vec<&'static str> {
        self.party_masters = in_session.to_vec();
        let mut changed = Vec::new();
        for party in in_session {
            let delivery = &mut self.doc.delivery;
            if delivery.party_mut(party.birthday_party_id).is_none() {
                delivery.parties.push(BirthdayPartyRow {
                    birthday_party_id: party.birthday_party_id,
                    delivery_total_point: 0,
                    dropped_mysekai_material_count: NEW_PARTY_DROPPED,
                    obtained_mysekai_material_count: 0,
                });
                info!(
                    "[server] new party row for party {}: deliveryTotalPoint 0, droppedMysekaiMaterialCount {NEW_PARTY_DROPPED} (unclaimed drops), obtainedMysekaiMaterialCount 0",
                    party.birthday_party_id
                );
                if !changed.contains(&SECTION_BIRTHDAY_PARTIES) {
                    changed.push(SECTION_BIRTHDAY_PARTIES);
                }
            }
            if !delivery
                .materials
                .contains_key(&party.delivery_item_material_id)
            {
                delivery
                    .materials
                    .insert(party.delivery_item_material_id, DELIVERY_ITEM_STOCK);
                info!(
                    "[server] delivery item stock: material {} of party {} at {DELIVERY_ITEM_STOCK}",
                    party.delivery_item_material_id, party.birthday_party_id
                );
                if !changed.contains(&SECTION_MATERIALS) {
                    changed.push(SECTION_MATERIALS);
                }
            }
        }
        changed
    }

    fn party_master(&self, birthday_party_id: i32) -> Result<PartyMaster, String> {
        self.party_masters
            .iter()
            .find(|party| party.birthday_party_id == birthday_party_id)
            .copied()
            .ok_or_else(|| match &self.parties_missing {
                Some(reason) => format!(
                    "party {birthday_party_id} is not in session at the server clock: no party is, since {reason}"
                ),
                None => format!("party {birthday_party_id} is not in session at the server clock"),
            })
    }

    /// Refuses, before a reply changes anything, an obtained count that
    /// newly reaches a total reward row while the box rows are not in.
    fn check_totals_grantable(
        &self,
        party: i32,
        before: i32,
        after: i32,
        masters: &Tables<'_>,
    ) -> Result<(), String> {
        if after <= before {
            return Ok(());
        }
        let tables = masters.delivery().ok_or(TABLES_ABSENT)?;
        if tables
            .totals_reached(i64::from(party), before, after)
            .is_empty()
        {
            return Ok(());
        }
        masters.reward_boxes.get(BOXES_REFUSED).map(|_| ())
    }

    /// The total reward rows the obtained count newly reaches, granted.
    fn grant_totals(
        &mut self,
        party: i32,
        before: i32,
        after: i32,
        now: i64,
        changed: &mut Vec<&'static str>,
        masters: &Tables<'_>,
    ) -> Result<Vec<Reward>, String> {
        if after <= before {
            return Ok(Vec::new());
        }
        let tables = masters.delivery().ok_or(TABLES_ABSENT)?;
        let rows = tables.totals_reached(i64::from(party), before, after);
        let rewards: Vec<Reward> = if rows.is_empty() {
            Vec::new()
        } else {
            let boxes = masters.reward_boxes.get(BOXES_REFUSED)?;
            rows.iter()
                .flat_map(|(_, _, box_id)| {
                    boxes
                        .iter()
                        .filter(move |(id, _)| id == box_id)
                        .map(|(_, reward)| reward.clone())
                })
                .collect()
        };
        let mut untracked = Vec::new();
        for reward in &rewards {
            let id = i32::try_from(reward.resource_id)
                .map_err(|_| format!("reward resource id {} does not fit", reward.resource_id))?;
            let delivery = &mut self.doc.delivery;
            match reward.resource_type.as_str() {
                HONOR => {
                    let level = reward.level.unwrap_or(1) as i32;
                    match delivery.honors.iter_mut().find(|row| row.honor_id == id) {
                        Some(row) => row.level = row.level.max(level),
                        None => delivery.honors.push(HonorRow {
                            honor_id: id,
                            level,
                            obtained_at: now,
                        }),
                    }
                    push_once(changed, SECTION_HONORS);
                }
                MATERIAL => {
                    *delivery.materials.entry(id).or_insert(0) += reward.quantity;
                    push_once(changed, SECTION_MATERIALS);
                }
                MYSEKAI_MATERIAL => {
                    *delivery.mysekai_materials.entry(id).or_insert(0) += reward.quantity;
                    push_once(changed, SECTION_MYSEKAI_MATERIALS);
                }
                other => untracked.push(format!("{other} {id} x{}", reward.quantity)),
            }
        }
        info!(
            "[server] total rewards for party {party}: obtainedMysekaiMaterialCount {before} -> {after} reaches rows {rows:?} -> {} resources; not tracked by the document: {untracked:?}",
            rewards.len()
        );
        Ok(rewards)
    }

    /// The delivery reply policy.
    pub(crate) fn birthday_party_delivery(
        &mut self,
        birthday_party_id: i32,
        consumed: i32,
        masters: &Tables<'_>,
    ) -> Result<(DeliveryResponse, Vec<&'static str>), String> {
        let master = self.party_master(birthday_party_id)?;
        if consumed < 0 {
            return Err(format!(
                "consumedDeliveryMaterialCount {consumed} is negative"
            ));
        }
        let (requirement, member_bonus) = {
            let tables = masters.delivery().ok_or(
                "the delivery tables are absent (not all three delivery master tables are in): the points cannot be counted",
            )?;
            let owned = self.owned_cards(masters);
            (
                tables.reward_loop_requirement(i64::from(birthday_party_id)),
                tables.member_bonus(i64::from(birthday_party_id), &owned),
            )
        };
        let base_point = self.master_config_int(CONFIG_BASE_POINT, masters)?;
        let drop_upper_limit = self.master_config_int(CONFIG_DROP_UPPER_LIMIT, masters)?;
        let now = self.now_ms();
        let delivery = &self.doc.delivery;
        let row = *delivery
            .parties
            .iter()
            .find(|row| row.birthday_party_id == birthday_party_id)
            .ok_or_else(|| format!("party {birthday_party_id} has no userBirthdayParties row"))?;
        let have = delivery
            .materials
            .get(&master.delivery_item_material_id)
            .copied()
            .unwrap_or(0);
        let mut tally = moly_law::delivery::PartyTally {
            synchronized_cost_quantity: have,
            unsynchronized_cost: 0,
            synchronized_points: row.delivery_total_point,
            unsynchronized_points: 0,
            base_point,
            member_bonus,
            reward_loop_requirement: requirement,
            max_drop_item_count: drop_upper_limit,
        };
        let loops_before = tally.total_drop_count();
        let spent = tally.spend(consumed);
        let loops_after = tally.total_drop_count();
        let new_loops = loops_after - loops_before;
        let free = (drop_upper_limit - row.dropped_mysekai_material_count).max(0);
        let dropped = new_loops.min(free);
        let obtained = new_loops - dropped;
        let points_before = row.delivery_total_point;
        let points_after = tally.current_points();
        let refresh = delivery.plant_refresh_points;
        let is_refreshed = refresh > 0 && points_after / refresh > points_before / refresh;
        self.check_totals_grantable(
            birthday_party_id,
            row.obtained_mysekai_material_count,
            row.obtained_mysekai_material_count + obtained,
            masters,
        )?;
        let delivery = &mut self.doc.delivery;
        let party = delivery
            .party_mut(birthday_party_id)
            .expect("the row found above");
        party.delivery_total_point = points_after;
        party.dropped_mysekai_material_count += dropped;
        party.obtained_mysekai_material_count += obtained;
        *delivery
            .materials
            .entry(master.delivery_item_material_id)
            .or_insert(0) -= spent;
        let mut changed = vec![SECTION_BIRTHDAY_PARTIES, SECTION_MATERIALS];
        if obtained > 0 {
            *delivery
                .mysekai_materials
                .entry(master.delivery_reward_mysekai_material_id)
                .or_insert(0) += obtained;
            changed.push(SECTION_MYSEKAI_MATERIALS);
        }
        let rewards = self.grant_totals(
            birthday_party_id,
            row.obtained_mysekai_material_count,
            row.obtained_mysekai_material_count + obtained,
            now,
            &mut changed,
            masters,
        )?;
        info!(
            "[server] delivery reply party {birthday_party_id}: sent {consumed}, spent {spent} (have {have}); points {points_before} -> {points_after} (base {base_point}, bonus {member_bonus}, loop requirement {requirement}); loops {loops_before} -> {loops_after}; dropped {dropped} (limit {drop_upper_limit}), obtained {obtained}; isRefreshed {is_refreshed} (birthday plant refresh every {refresh} points, an inference)"
        );
        Ok((
            DeliveryResponse {
                dropped_reward_count: dropped,
                obtained_delivery_total_rewards: rewards,
                is_refreshed,
            },
            changed,
        ))
    }

    /// The gather reply policy.
    pub(crate) fn birthday_party_gather(
        &mut self,
        contents: &[GatherContent],
        masters: &Tables<'_>,
    ) -> Result<(GatherResponse, Vec<&'static str>), String> {
        let now = self.now_ms();
        let mut changed = vec![SECTION_BIRTHDAY_PARTIES];
        let mut rewards = Vec::new();
        let mut seen = BTreeSet::new();
        for content in contents {
            let row = self
                .doc
                .delivery
                .parties
                .iter()
                .find(|row| row.birthday_party_id == content.birthday_party_id);
            if let Some(party) = row {
                let before = party.obtained_mysekai_material_count;
                self.check_totals_grantable(
                    content.birthday_party_id,
                    before,
                    before + content.gathered_count.max(0),
                    masters,
                )?;
            }
        }
        for content in contents {
            if !seen.insert(content.birthday_party_id) {
                return Err(format!(
                    "the gather request names party {} twice",
                    content.birthday_party_id
                ));
            }
            if content.gathered_count < 0 {
                return Err(format!(
                    "gatheredCount {} of party {} is negative",
                    content.gathered_count, content.birthday_party_id
                ));
            }
            let master = self.party_master(content.birthday_party_id)?;
            let delivery = &mut self.doc.delivery;
            let party = delivery
                .party_mut(content.birthday_party_id)
                .ok_or_else(|| {
                    format!(
                        "party {} has no userBirthdayParties row",
                        content.birthday_party_id
                    )
                })?;
            let before = party.obtained_mysekai_material_count;
            party.dropped_mysekai_material_count =
                (party.dropped_mysekai_material_count - content.gathered_count).max(0);
            party.obtained_mysekai_material_count += content.gathered_count;
            let after = party.obtained_mysekai_material_count;
            *delivery
                .mysekai_materials
                .entry(master.delivery_reward_mysekai_material_id)
                .or_insert(0) += content.gathered_count;
            push_once(&mut changed, SECTION_MYSEKAI_MATERIALS);
            rewards.extend(self.grant_totals(
                content.birthday_party_id,
                before,
                after,
                now,
                &mut changed,
                masters,
            )?);
        }
        info!(
            "[server] gather reply {contents:?}: {} total rewards",
            rewards.len()
        );
        Ok((
            GatherResponse {
                obtained_delivery_total_rewards: rewards,
            },
            changed,
        ))
    }

    /// A delivery-site reply: the document changed, a response is recorded
    /// (its other sections reach the client copies through the response
    /// queue), and the delivery sections it carries are returned for the
    /// caller to merge at once, as the API executor does.
    pub(crate) fn delivery_reply(
        &mut self,
        kind: ResponseKind,
        changed: &[&'static str],
        masters: &Tables<'_>,
    ) -> DeliveryUpdate {
        let mut sections: Vec<String> = changed.iter().map(|s| (*s).to_owned()).collect();
        for section in &self.pending {
            if SECTIONS.contains(&section.as_str()) && !sections.contains(section) {
                sections.push(section.clone());
            }
        }
        self.pending
            .retain(|section| !SECTIONS.contains(&section.as_str()));
        let update = self.delivery_update(&sections, masters);
        self.respond(kind, false, &[], masters);
        update
    }
}

/// The refusal of a total reward grant without the delivery tables.
const TABLES_ABSENT: &str =
    "the delivery tables are absent (not all three delivery master tables are in): the total rewards cannot be granted";
/// What a total reward grant cannot do without the box rows.
const BOXES_REFUSED: &str = "the total rewards cannot be granted";

fn push_once(list: &mut Vec<&'static str>, section: &'static str) {
    if !list.contains(&section) {
        list.push(section);
    }
}

/// `PutUserMysekaiBirthdayPartyDeliveryApi`: the reply, with its
// ---------------------------------------------------------------------------
// Panel edits and schema
// ---------------------------------------------------------------------------

fn edit_int(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} = {raw} does not fit a 32-bit integer"))
}

/// A `server.edit` of this section: `Some(section)` names the section the
/// next response carries (`None` for a live policy); `Err` when the path is
/// not this section's.
pub(crate) fn edit_path(
    doc: &mut DeliveryDoc,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<Option<&'static str>, String>> {
    let result = match parts {
        [SECTION_BIRTHDAY_PARTIES] => parse_parties(value).map(|rows| {
            doc.parties = rows;
            Some(SECTION_BIRTHDAY_PARTIES)
        }),
        [SECTION_BIRTHDAY_PARTIES, index, key] => (|| {
            let index = index
                .parse::<usize>()
                .map_err(|_| format!("{index} is not a row index"))?;
            let len = doc.parties.len();
            let row = doc
                .parties
                .get_mut(index)
                .ok_or_else(|| format!("{SECTION_BIRTHDAY_PARTIES} has {len} rows"))?;
            let amount = edit_int(value, path)?;
            match *key {
                "deliveryTotalPoint" => row.delivery_total_point = amount,
                "droppedMysekaiMaterialCount" => row.dropped_mysekai_material_count = amount,
                "obtainedMysekaiMaterialCount" => row.obtained_mysekai_material_count = amount,
                _ => return Err(format!("{path} is not an editable party row field")),
            }
            Ok(Some(SECTION_BIRTHDAY_PARTIES))
        })(),
        [SECTION_MATERIALS] => {
            parse_quantities(value, SECTION_MATERIALS, "materialId").map(|map| {
                doc.materials = map;
                Some(SECTION_MATERIALS)
            })
        }
        [SECTION_MATERIALS, id] => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{id} is not a material id"))?;
            doc.materials.insert(id, edit_int(value, path)?);
            Ok(Some(SECTION_MATERIALS))
        })(),
        [SECTION_MYSEKAI_MATERIALS] => {
            parse_quantities(value, SECTION_MYSEKAI_MATERIALS, "mysekaiMaterialId").map(|map| {
                doc.mysekai_materials = map;
                Some(SECTION_MYSEKAI_MATERIALS)
            })
        }
        [SECTION_MYSEKAI_MATERIALS, id] => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{id} is not a mysekai material id"))?;
            doc.mysekai_materials.insert(id, edit_int(value, path)?);
            Ok(Some(SECTION_MYSEKAI_MATERIALS))
        })(),
        [SECTION_CARDS] => parse_cards(value).map(|cards| {
            doc.cards = cards;
            Some(SECTION_CARDS)
        }),
        [SECTION_HONORS] => parse_honors(value).map(|rows| {
            doc.honors = rows;
            Some(SECTION_HONORS)
        }),
        ["policies", "ownedCards"] => parse_owned_cards(value).map(|policy| {
            doc.owned_cards = policy;
            // The client's card copy follows the policy.
            Some(SECTION_CARDS)
        }),
        ["policies", "birthdayPlantRefreshPoints"] => edit_int(value, path).map(|points| {
            doc.plant_refresh_points = points;
            None
        }),
        _ => return None,
    };
    Some(result)
}

/// The panel's delivery sections.
pub(crate) fn schema_sections(model: Option<&ServerModel>) -> Value {
    let parties: Vec<Value> = model
        .map(|model| {
            model
                .party_masters
                .iter()
                .map(|party| {
                    json!({
                        "birthdayPartyId": party.birthday_party_id,
                        "deliveryItemMaterialId": party.delivery_item_material_id,
                        "deliveryRewardMysekaiMaterialId": party.delivery_reward_mysekai_material_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!([
        {
            "key": "birthdayParty",
            "title": "Birthday party delivery",
            "delivery": "next-response",
            "partiesInSession": parties,
            "fields": [
                {"path": "userBirthdayParties.<index>.deliveryTotalPoint", "type": "int", "min": 0},
                {"path": "userBirthdayParties.<index>.droppedMysekaiMaterialCount", "type": "int", "min": 0},
                {"path": "userBirthdayParties.<index>.obtainedMysekaiMaterialCount", "type": "int", "min": 0},
                {"path": "userBirthdayParties", "type": "rows", "row": {
                    "birthdayPartyId": "int", "deliveryTotalPoint": "int",
                    "droppedMysekaiMaterialCount": "int", "obtainedMysekaiMaterialCount": "int"}},
                {"path": "userMaterials", "type": "rows", "row": {"materialId": "int", "quantity": "int"},
                    "note": "the delivery item's have-quantity"},
                {"path": "userMysekaiMaterials", "type": "rows", "row": {"mysekaiMaterialId": "int", "quantity": "int"},
                    "note": "the reward drops' material"},
                {"path": "userHonors", "type": "rows", "row": {"honorId": "int", "level": "int", "obtainedAt": "epoch-ms"}},
                {"path": "userCards", "type": "rows", "row": {"cardId": "int"},
                    "when": {"policies.ownedCards": OWNED_STATED}},
            ],
        },
        {
            "key": "birthdayPartyPolicies",
            "title": "Birthday party server policies",
            "delivery": "live",
            "fields": [
                {"path": "policies.ownedCards", "type": "enum", "values": [
                    {"value": OWNED_EVERY_BONUS_CARD, "label": "the user owns every card a point bonus row names"},
                    {"value": OWNED_STATED, "label": "the userCards rows"},
                ]},
                {"path": "policies.birthdayPlantRefreshPoints", "type": "int", "min": 0,
                    "note": "an inference, not a read: the delivery reply says isRefreshed (the refresh-birthday-plant dialog, the site's harvest fixtures destroyed) when deliveryTotalPoint passes a multiple of this; the server-only repeat-refresh master has 10000 for every party on disk; 0 never"},
            ],
        },
    ])
}

/// The panel's named delivery policies.
pub(crate) fn schema_policies() -> Value {
    json!([
        {"name": "new party row", "rule": format!("a party in session at the server clock with no userBirthdayParties row gets deliveryTotalPoint 0, droppedMysekaiMaterialCount {NEW_PARTY_DROPPED} (unclaimed drops) and obtainedMysekaiMaterialCount 0")},
        {"name": "delivery item stock", "rule": format!("the delivery item of a party in session with no userMaterials row gets {DELIVERY_ITEM_STOCK}")},
        {"name": "delivery reply", "rule": "spend the sent count capped at the have-quantity; add the points the client's accrual law gives; the reward loops newly reached drop while they fit under the drop limit beside the drops already counted, the rest are obtained and join the reward material"},
        {"name": "gather reply", "rule": "each gathered drop leaves droppedMysekaiMaterialCount and joins obtainedMysekaiMaterialCount and the reward material"},
        {"name": "total rewards", "rule": "a reply grants every total reward row whose requirement its obtainedMysekaiMaterialCount reaches for the first time (the client shows a total reward as obtained when requirement <= obtainedMysekaiMaterialCount); honors raise userHonors to the box level, materials and mysekai materials add to their quantity, other resource types are carried in the reply only"},
        {"name": "birthday plant refresh (inference)", "rule": "the delivery reply says isRefreshed when deliveryTotalPoint passes a multiple of policies.birthdayPlantRefreshPoints (0 never)"},
    ])
}
