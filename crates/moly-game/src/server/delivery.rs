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
//! - `masterConfigs`: the two master configs the delivery reads with
//!   `MasterDataManager.GetMasterConfigToInt` (no master copy on disk has
//!   them, so the server document holds them).
//!
//! The mock's own keys: `policies.ownedCards` and
//! `policies.birthdayPlantRefreshPoints`. Every key is optional when a
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

use super::document::{int32, int64, object, only};
use super::{Masters, ResponseKind, ServerModel};

pub(crate) const SECTION_BIRTHDAY_PARTIES: &str = "userBirthdayParties";
pub(crate) const SECTION_MATERIALS: &str = "userMaterials";
pub(crate) const SECTION_MYSEKAI_MATERIALS: &str = "userMysekaiMaterials";
pub(crate) const SECTION_CARDS: &str = "userCards";
pub(crate) const SECTION_HONORS: &str = "userHonors";
pub(crate) const SECTION_MASTER_CONFIGS: &str = "masterConfigs";
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
/// `birthday_party_delivery_base_point` of a document that states none.
pub(crate) const DEFAULT_BASE_POINT: i32 = 100;
/// `birthday_party_delivery_reward_drop_upper_limit` of a document that
/// states none.
pub(crate) const DEFAULT_DROP_UPPER_LIMIT: i32 = 5;
/// `policies.birthdayPlantRefreshPoints` of a document that states none (the
/// repeat-refresh requirement of every party row on disk).
pub(crate) const DEFAULT_PLANT_REFRESH_POINTS: i32 = 10_000;

pub(crate) const OWNED_EVERY_BONUS_CARD: &str = "everyPointBonusCard";
pub(crate) const OWNED_STATED: &str = "stated";

/// `UserBirthdayParty`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BirthdayPartyRow {
    pub(crate) birthday_party_id: i32,
    pub(crate) delivery_total_point: i32,
    pub(crate) dropped_mysekai_material_count: i32,
    pub(crate) obtained_mysekai_material_count: i32,
}

/// `UserHonor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HonorRow {
    pub(crate) honor_id: i32,
    pub(crate) level: i32,
    pub(crate) obtained_at: i64,
}

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
    pub(crate) base_point: i32,
    pub(crate) drop_upper_limit: i32,
    pub(crate) plant_refresh_points: i32,
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
            base_point: DEFAULT_BASE_POINT,
            drop_upper_limit: DEFAULT_DROP_UPPER_LIMIT,
            plant_refresh_points: DEFAULT_PLANT_REFRESH_POINTS,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading and writing the section
// ---------------------------------------------------------------------------

/// The document keys this section reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 6] = SECTIONS;
/// The keys this section reads under `policies`.
pub(crate) const POLICY_KEYS: [&str; 2] = ["ownedCards", "birthdayPlantRefreshPoints"];

pub(crate) fn parse_parties(value: &Value) -> Result<Vec<BirthdayPartyRow>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{SECTION_BIRTHDAY_PARTIES} is not an array"))?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_BIRTHDAY_PARTIES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "birthdayPartyId",
                    "deliveryTotalPoint",
                    "droppedMysekaiMaterialCount",
                    "obtainedMysekaiMaterialCount",
                ],
                &at,
            )?;
            Ok(BirthdayPartyRow {
                birthday_party_id: int32(row, "birthdayPartyId", &at)?,
                delivery_total_point: int32(row, "deliveryTotalPoint", &at)?,
                dropped_mysekai_material_count: int32(row, "droppedMysekaiMaterialCount", &at)?,
                obtained_mysekai_material_count: int32(row, "obtainedMysekaiMaterialCount", &at)?,
            })
        })
        .collect()
}

/// `userMaterials` / `userMysekaiMaterials`: rows of an id and a quantity.
pub(crate) fn parse_quantities(
    value: &Value,
    at: &str,
    id_key: &str,
) -> Result<BTreeMap<i32, i32>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{at} is not an array"))?;
    let mut out = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let row_at = format!("{at}[{i}]");
        let row = object(row, &row_at)?;
        only(row, &[id_key, "quantity"], &row_at)?;
        let id = int32(row, id_key, &row_at)?;
        if out.insert(id, int32(row, "quantity", &row_at)?).is_some() {
            return Err(format!("{row_at} repeats {id_key} {id}"));
        }
    }
    Ok(out)
}

pub(crate) fn parse_cards(value: &Value) -> Result<Vec<i64>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{SECTION_CARDS} is not an array"))?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_CARDS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["cardId"], &at)?;
            int64(row, "cardId", &at)
        })
        .collect()
}

pub(crate) fn parse_honors(value: &Value) -> Result<Vec<HonorRow>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{SECTION_HONORS} is not an array"))?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_HONORS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["honorId", "level", "obtainedAt"], &at)?;
            Ok(HonorRow {
                honor_id: int32(row, "honorId", &at)?,
                level: int32(row, "level", &at)?,
                obtained_at: int64(row, "obtainedAt", &at)?,
            })
        })
        .collect()
}

pub(crate) fn parse_owned_cards(value: &Value) -> Result<OwnedCards, String> {
    match value.as_str() {
        Some(OWNED_EVERY_BONUS_CARD) => Ok(OwnedCards::EveryPointBonusCard),
        Some(OWNED_STATED) => Ok(OwnedCards::Stated),
        _ => Err(format!(
            "policies.ownedCards is neither \"{OWNED_EVERY_BONUS_CARD}\" (the user owns every card a point bonus row names) nor \"{OWNED_STATED}\" (the userCards rows)"
        )),
    }
}

/// `masterConfigs`: (base point, drop upper limit).
pub(crate) fn parse_master_configs(value: &Value) -> Result<(i32, i32), String> {
    let configs = object(value, SECTION_MASTER_CONFIGS)?;
    only(
        configs,
        &[CONFIG_BASE_POINT, CONFIG_DROP_UPPER_LIMIT],
        SECTION_MASTER_CONFIGS,
    )?;
    Ok((
        int32(configs, CONFIG_BASE_POINT, SECTION_MASTER_CONFIGS)?,
        int32(configs, CONFIG_DROP_UPPER_LIMIT, SECTION_MASTER_CONFIGS)?,
    ))
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
        if let Some(value) = doc.get(SECTION_MASTER_CONFIGS) {
            (out.base_point, out.drop_upper_limit) = parse_master_configs(value)?;
        }
        if let Some(value) = policies.get("ownedCards") {
            out.owned_cards = parse_owned_cards(value)?;
        }
        if policies.contains_key("birthdayPlantRefreshPoints") {
            out.plant_refresh_points = int32(policies, "birthdayPlantRefreshPoints", "policies")?;
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
        for (name, value) in [
            (CONFIG_BASE_POINT, self.base_point),
            (CONFIG_DROP_UPPER_LIMIT, self.drop_upper_limit),
        ] {
            if value < 0 {
                return Err(format!(
                    "{SECTION_MASTER_CONFIGS}.{name} = {value} is negative"
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
        doc.insert(
            SECTION_MASTER_CONFIGS.into(),
            json!({CONFIG_BASE_POINT: self.base_point, CONFIG_DROP_UPPER_LIMIT: self.drop_upper_limit}),
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
    }

    fn party_mut(&mut self, id: i32) -> Option<&mut BirthdayPartyRow> {
        self.parties
            .iter_mut()
            .find(|row| row.birthday_party_id == id)
    }
}

pub(crate) fn party_value(row: BirthdayPartyRow) -> Value {
    json!({
        "birthdayPartyId": row.birthday_party_id,
        "deliveryTotalPoint": row.delivery_total_point,
        "droppedMysekaiMaterialCount": row.dropped_mysekai_material_count,
        "obtainedMysekaiMaterialCount": row.obtained_mysekai_material_count,
    })
}

fn honor_value(row: HonorRow) -> Value {
    json!({"honorId": row.honor_id, "level": row.level, "obtainedAt": row.obtained_at})
}

fn quantities_value(map: &BTreeMap<i32, i32>, id_key: &str) -> Value {
    Value::Array(
        map.iter()
            .map(|(id, quantity)| json!({id_key: id, "quantity": quantity}))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Master tables (`birthday-party-delivery.json`)
// ---------------------------------------------------------------------------

/// One resource of a reply (`UserResource`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reward {
    pub(crate) resource_type: String,
    pub(crate) resource_id: i64,
    pub(crate) quantity: i32,
    pub(crate) level: Option<i64>,
}

#[derive(Clone, Debug)]
struct RewardRow {
    party: i64,
    requirement: i32,
}

#[derive(Clone, Debug)]
struct BonusRow {
    party: i64,
    card: i64,
    rate: i32,
}

#[derive(Clone, Debug)]
struct TotalRow {
    id: i64,
    party: i64,
    requirement: i32,
    box_id: i64,
}

/// The delivery master tables (`birthday-party-delivery.json`): the reward
/// rows and point bonus rows the client reads, and the total reward rows and
/// their boxes the server grants from. The server model reads them as a
/// master and inserts this resource for the client's readers.
#[derive(Resource, Clone, Debug)]
pub(crate) struct DeliveryTables {
    rewards: Vec<RewardRow>,
    bonuses: Vec<BonusRow>,
    totals: Vec<TotalRow>,
    boxes: Vec<(i64, Reward)>,
}

impl DeliveryTables {
    /// The reward loop requirement of a party: its last reward row's, or 1.
    pub(crate) fn reward_loop_requirement(&self, party: i64) -> i32 {
        let rows: Vec<i32> = self
            .rewards
            .iter()
            .filter(|row| row.party == party)
            .map(|row| row.requirement)
            .collect();
        moly_law::delivery::reward_loop_requirement(&rows)
    }

    /// `MemberBonus`: the rates of the bonus rows whose card the user owns.
    pub(crate) fn member_bonus(&self, party: i64, owned: &BTreeSet<i64>) -> f32 {
        let rates: Vec<i32> = self
            .bonuses
            .iter()
            .filter(|row| row.party == party && owned.contains(&row.card))
            .map(|row| row.rate)
            .collect();
        moly_law::delivery::member_bonus(&rates)
    }

    /// Every card a point bonus row names (the owned cards policy
    /// `everyPointBonusCard`).
    pub(crate) fn bonus_cards(&self) -> BTreeSet<i64> {
        self.bonuses.iter().map(|row| row.card).collect()
    }

    fn box_contents(&self, box_id: i64) -> Vec<Reward> {
        self.boxes
            .iter()
            .filter(|(id, _)| *id == box_id)
            .map(|(_, reward)| reward.clone())
            .collect()
    }

    /// The total reward rows of a party an obtained count newly reaches:
    /// `before < requirement <= after`, master order.
    fn totals_reached(&self, party: i64, before: i32, after: i32) -> Vec<(i64, i32, i64)> {
        self.totals
            .iter()
            .filter(|row| {
                row.party == party && before < row.requirement && row.requirement <= after
            })
            .map(|row| (row.id, row.requirement, row.box_id))
            .collect()
    }
}

fn table_int(row: &Value, table: &str, key: &str) -> Result<i64, String> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{table} row {:?} has no integer {key}", row.get("id")))
}

fn table_rows<'a>(doc: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    doc.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("no {key} array"))
}

/// The master parser of `birthday-party-delivery.json`.
pub(crate) fn parse_tables(text: &str, masters: &mut Masters) -> Result<(), String> {
    let doc: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let rewards = table_rows(&doc, "rewards")?
        .iter()
        .map(|row| {
            Ok(RewardRow {
                party: table_int(row, "rewards", "birthdayPartyId")?,
                requirement: table_int(row, "rewards", "requirement")? as i32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let bonuses = table_rows(&doc, "pointBonuses")?
        .iter()
        .map(|row| {
            Ok(BonusRow {
                party: table_int(row, "pointBonuses", "birthdayPartyId")?,
                card: table_int(row, "pointBonuses", "cardId")?,
                rate: table_int(row, "pointBonuses", "rate")? as i32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let totals = table_rows(&doc, "totalRewards")?
        .iter()
        .map(|row| {
            Ok(TotalRow {
                id: table_int(row, "totalRewards", "id")?,
                party: table_int(row, "totalRewards", "birthdayPartyId")?,
                requirement: table_int(row, "totalRewards", "requirement")? as i32,
                box_id: table_int(row, "totalRewards", "resourceBoxId")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let boxes = table_rows(&doc, "totalRewardBoxes")?
        .iter()
        .map(|row| {
            Ok((
                table_int(row, "totalRewardBoxes", "resourceBoxId")?,
                Reward {
                    resource_type: row
                        .get("resourceType")
                        .and_then(Value::as_str)
                        .ok_or("a total reward box row has no resourceType")?
                        .to_owned(),
                    resource_id: table_int(row, "totalRewardBoxes", "resourceId")?,
                    quantity: table_int(row, "totalRewardBoxes", "resourceQuantity")? as i32,
                    level: row.get("resourceLevel").and_then(Value::as_i64),
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    info!(
        "[server] delivery tables: {} reward rows, {} point bonus rows, {} total reward rows, {} box rows",
        rewards.len(),
        bonuses.len(),
        totals.len(),
        boxes.len()
    );
    masters.delivery = Some(DeliveryTables {
        rewards,
        bonuses,
        totals,
        boxes,
    });
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

/// The delivery sections a response carries (full sections, replacing the
/// client's copies).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DeliveryUpdate {
    pub(crate) parties: Option<Vec<BirthdayPartyRow>>,
    pub(crate) materials: Option<BTreeMap<i32, i32>>,
    pub(crate) mysekai_materials: Option<BTreeMap<i32, i32>>,
    pub(crate) cards: Option<BTreeSet<i64>>,
    pub(crate) honors: Option<Vec<HonorRow>>,
    pub(crate) configs: Option<BTreeMap<String, i32>>,
}

impl DeliveryUpdate {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The client's copies of the delivery user data (`UserDataManager`) and of
/// the two master configs (`MasterDataManager`), set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientBirthdayPartyData {
    parties: Vec<BirthdayPartyRow>,
    materials: BTreeMap<i32, i32>,
    mysekai_materials: BTreeMap<i32, i32>,
    cards: BTreeSet<i64>,
    honors: Vec<HonorRow>,
    configs: BTreeMap<String, i32>,
    /// Responses that carried a delivery section.
    pub(crate) revision: u64,
}

#[allow(dead_code)] // Read by the delivery owner's seam.
impl ClientBirthdayPartyData {
    /// `UserDataManager.GetUserBirthdayParty(id)`.
    pub(crate) fn user_birthday_party(&self, birthday_party_id: i32) -> Option<&BirthdayPartyRow> {
        self.parties
            .iter()
            .find(|row| row.birthday_party_id == birthday_party_id)
    }

    /// `UserDataUtility.GetHaveQuantity(id, ResourceType.material)`: 0
    /// without a row.
    pub(crate) fn have_quantity(&self, material_id: i32) -> i32 {
        self.materials.get(&material_id).copied().unwrap_or(0)
    }

    /// The `userMysekaiMaterials` quantity: 0 without a row.
    pub(crate) fn mysekai_material_quantity(&self, mysekai_material_id: i32) -> i32 {
        self.mysekai_materials
            .get(&mysekai_material_id)
            .copied()
            .unwrap_or(0)
    }

    /// The card ids of `userCards` (`UserDataManager.GetCard(id) != null`).
    pub(crate) fn cards(&self) -> &BTreeSet<i64> {
        &self.cards
    }

    pub(crate) fn honors(&self) -> &[HonorRow] {
        &self.honors
    }

    /// `MasterDataManager.GetMasterConfigToInt(key)`; `None` before the
    /// join delivered it.
    pub(crate) fn master_config_int(&self, key: &str) -> Option<i32> {
        self.configs.get(key).copied()
    }

    /// `updatedResources`: the carried sections replace the copies.
    pub(crate) fn apply(&mut self, update: DeliveryUpdate) {
        if update.is_empty() {
            return;
        }
        if let Some(parties) = update.parties {
            self.parties = parties;
        }
        if let Some(materials) = update.materials {
            self.materials = materials;
        }
        if let Some(materials) = update.mysekai_materials {
            self.mysekai_materials = materials;
        }
        if let Some(cards) = update.cards {
            self.cards = cards;
        }
        if let Some(honors) = update.honors {
            self.honors = honors;
        }
        if let Some(configs) = update.configs {
            self.configs = configs;
        }
        self.revision += 1;
    }

    pub(crate) fn view(&self) -> Value {
        json!({
            SECTION_BIRTHDAY_PARTIES: self.parties.iter().map(|row| party_value(*row)).collect::<Vec<_>>(),
            SECTION_MATERIALS: quantities_value(&self.materials, "materialId"),
            SECTION_MYSEKAI_MATERIALS: quantities_value(&self.mysekai_materials, "mysekaiMaterialId"),
            "userCardCount": self.cards.len(),
            SECTION_HONORS: self.honors.iter().map(|row| honor_value(*row)).collect::<Vec<_>>(),
            SECTION_MASTER_CONFIGS: self.configs,
            "revision": self.revision,
        })
    }
}

/// The delivery reply (`UserBirthdayPartyDeliveryResponse`); its
/// `updatedResources` are already in the client's copies.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // Read by the delivery owner's seam.
pub(crate) struct DeliveryResponse {
    pub(crate) dropped_reward_count: i32,
    pub(crate) obtained_delivery_total_rewards: Vec<Reward>,
    pub(crate) is_refreshed: bool,
}

/// The gather reply (`UserBirthdayPartyGatherResponse`); its
/// `updatedResources` are already in the client's copies.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // Read by the delivery owner's seam.
pub(crate) struct GatherResponse {
    pub(crate) obtained_delivery_total_rewards: Vec<Reward>,
}

/// One row of the gather request (`UserBirthdayPartyGatherRequestContent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Built by the delivery owner's seam.
pub(crate) struct GatherContent {
    pub(crate) birthday_party_id: i32,
    pub(crate) gathered_count: i32,
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

impl ServerModel {
    /// The owned card ids the policy gives.
    fn owned_cards(&self) -> BTreeSet<i64> {
        match self.doc.delivery.owned_cards {
            OwnedCards::Stated => self.doc.delivery.cards.iter().copied().collect(),
            OwnedCards::EveryPointBonusCard => self
                .masters
                .delivery
                .as_ref()
                .map(DeliveryTables::bonus_cards)
                .unwrap_or_default(),
        }
    }

    /// The full delivery sections named in `sections`.
    pub(super) fn delivery_update(&self, sections: &[String]) -> DeliveryUpdate {
        let has = |name: &str| sections.iter().any(|section| section == name);
        let doc = &self.doc.delivery;
        DeliveryUpdate {
            parties: has(SECTION_BIRTHDAY_PARTIES).then(|| doc.parties.clone()),
            materials: has(SECTION_MATERIALS).then(|| doc.materials.clone()),
            mysekai_materials: has(SECTION_MYSEKAI_MATERIALS)
                .then(|| doc.mysekai_materials.clone()),
            cards: has(SECTION_CARDS).then(|| self.owned_cards()),
            honors: has(SECTION_HONORS).then(|| doc.honors.clone()),
            configs: has(SECTION_MASTER_CONFIGS).then(|| {
                BTreeMap::from([
                    (CONFIG_BASE_POINT.to_owned(), doc.base_point),
                    (CONFIG_DROP_UPPER_LIMIT.to_owned(), doc.drop_upper_limit),
                ])
            }),
        }
    }

    /// The new party row and delivery item stock policies for the parties in
    /// session. Returns the sections it changed.
    pub(super) fn seat_parties(&mut self, in_session: &[PartyMaster]) -> Vec<&'static str> {
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
            .ok_or_else(|| {
                format!("party {birthday_party_id} is not in session at the server clock")
            })
    }

    /// The total reward rows the obtained count newly reaches, granted.
    fn grant_totals(
        &mut self,
        party: i32,
        before: i32,
        after: i32,
        now: i64,
        changed: &mut Vec<&'static str>,
    ) -> Result<Vec<Reward>, String> {
        if after <= before {
            return Ok(Vec::new());
        }
        let tables = self.masters.delivery.as_ref().ok_or(
            "the delivery tables (birthday-party-delivery.json) are absent: the total rewards cannot be granted",
        )?;
        let rows = tables.totals_reached(i64::from(party), before, after);
        let rewards: Vec<Reward> = rows
            .iter()
            .flat_map(|(_, _, box_id)| tables.box_contents(*box_id))
            .collect();
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
    fn birthday_party_delivery(
        &mut self,
        birthday_party_id: i32,
        consumed: i32,
    ) -> Result<(DeliveryResponse, Vec<&'static str>), String> {
        let master = self.party_master(birthday_party_id)?;
        if consumed < 0 {
            return Err(format!(
                "consumedDeliveryMaterialCount {consumed} is negative"
            ));
        }
        let (requirement, member_bonus) = {
            let tables = self.masters.delivery.as_ref().ok_or(
                "the delivery tables (birthday-party-delivery.json) are absent: the points cannot be counted",
            )?;
            let owned = self.owned_cards();
            (
                tables.reward_loop_requirement(i64::from(birthday_party_id)),
                tables.member_bonus(i64::from(birthday_party_id), &owned),
            )
        };
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
            base_point: delivery.base_point,
            member_bonus,
            reward_loop_requirement: requirement,
            max_drop_item_count: delivery.drop_upper_limit,
        };
        let loops_before = tally.total_drop_count();
        let spent = tally.spend(consumed);
        let loops_after = tally.total_drop_count();
        let new_loops = loops_after - loops_before;
        let free = (delivery.drop_upper_limit - row.dropped_mysekai_material_count).max(0);
        let dropped = new_loops.min(free);
        let obtained = new_loops - dropped;
        let points_before = row.delivery_total_point;
        let points_after = tally.current_points();
        let refresh = delivery.plant_refresh_points;
        let is_refreshed = refresh > 0 && points_after / refresh > points_before / refresh;
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
        )?;
        info!(
            "[server] delivery reply party {birthday_party_id}: sent {consumed}, spent {spent} (have {have}); points {points_before} -> {points_after} (base {}, bonus {member_bonus}, loop requirement {requirement}); loops {loops_before} -> {loops_after}; dropped {dropped} (limit {}), obtained {obtained}; isRefreshed {is_refreshed} (birthday plant refresh every {refresh} points, an inference)",
            self.doc.delivery.base_point,
            self.doc.delivery.drop_upper_limit
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
    fn birthday_party_gather(
        &mut self,
        contents: &[GatherContent],
    ) -> Result<(GatherResponse, Vec<&'static str>), String> {
        let now = self.now_ms();
        let mut changed = vec![SECTION_BIRTHDAY_PARTIES];
        let mut rewards = Vec::new();
        let mut seen = BTreeSet::new();
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
    fn delivery_reply(&mut self, kind: ResponseKind, changed: &[&'static str]) -> DeliveryUpdate {
        let mut sections: Vec<String> = changed.iter().map(|s| (*s).to_owned()).collect();
        for section in &self.pending {
            if SECTIONS.contains(&section.as_str()) && !sections.contains(section) {
                sections.push(section.clone());
            }
        }
        self.pending
            .retain(|section| !SECTIONS.contains(&section.as_str()));
        let update = self.delivery_update(&sections);
        self.respond(kind, false, &[]);
        update
    }
}

fn push_once(list: &mut Vec<&'static str>, section: &'static str) {
    if !list.contains(&section) {
        list.push(section);
    }
}

/// `PutUserMysekaiBirthdayPartyDeliveryApi`: the reply, with its
/// `updatedResources` merged into the client's copies.
#[allow(dead_code)] // Read by the delivery owner's seam.
pub(crate) fn put_birthday_party_delivery(
    client: &mut ClientBirthdayPartyData,
    birthday_party_id: i32,
    consumed_delivery_material_count: i32,
) -> Result<DeliveryResponse, String> {
    let (response, update) = super::with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let (response, changed) =
            model.birthday_party_delivery(birthday_party_id, consumed_delivery_material_count)?;
        let update = model.delivery_reply(ResponseKind::BirthdayPartyDelivery, &changed);
        Ok((response, update))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()))
    .map_err(|reason| format!("PutUserMysekaiBirthdayPartyDeliveryApi: {reason}"))?;
    client.apply(update);
    Ok(response)
}

/// `PutUserMysekaiBirthdayPartyGatherApi`: the reply, with its
/// `updatedResources` merged into the client's copies.
#[allow(dead_code)] // Read by the delivery owner's seam.
pub(crate) fn put_birthday_party_gather(
    client: &mut ClientBirthdayPartyData,
    contents: &[GatherContent],
) -> Result<GatherResponse, String> {
    let (response, update) = super::with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let (response, changed) = model.birthday_party_gather(contents)?;
        let update = model.delivery_reply(ResponseKind::BirthdayPartyGather, &changed);
        Ok((response, update))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()))
    .map_err(|reason| format!("PutUserMysekaiBirthdayPartyGatherApi: {reason}"))?;
    client.apply(update);
    Ok(response)
}

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
        [SECTION_MASTER_CONFIGS, key] => (|| {
            let amount = edit_int(value, path)?;
            match *key {
                CONFIG_BASE_POINT => doc.base_point = amount,
                CONFIG_DROP_UPPER_LIMIT => doc.drop_upper_limit = amount,
                _ => return Err(format!("{path} is not a delivery master config")),
            }
            Ok(Some(SECTION_MASTER_CONFIGS))
        })(),
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
                {"path": format!("{SECTION_MASTER_CONFIGS}.{CONFIG_BASE_POINT}"), "type": "int", "min": 0,
                    "note": "a master config no master copy on disk has"},
                {"path": format!("{SECTION_MASTER_CONFIGS}.{CONFIG_DROP_UPPER_LIMIT}"), "type": "int", "min": 0,
                    "note": "a master config no master copy on disk has"},
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> DeliveryTables {
        let text = json!({
            "rewards": [{"id": 1, "birthdayPartyId": 1, "requirement": 10000}],
            "pointBonuses": [{"id": 1, "birthdayPartyId": 1, "cardId": 7, "rate": 50}],
            "totalRewards": [
                {"id": 1, "birthdayPartyId": 1, "requirement": 2, "resourceBoxId": 20},
                {"id": 2, "birthdayPartyId": 1, "requirement": 3, "resourceBoxId": 30},
            ],
            "totalRewardBoxes": [
                {"resourceBoxId": 20, "resourceType": "honor", "resourceId": 6818, "resourceQuantity": 1, "resourceLevel": 1},
                {"resourceBoxId": 20, "resourceType": "boost_item", "resourceId": 1, "resourceQuantity": 2},
                {"resourceBoxId": 30, "resourceType": "honor", "resourceId": 6818, "resourceQuantity": 1, "resourceLevel": 2},
                {"resourceBoxId": 30, "resourceType": "material", "resourceId": 15, "resourceQuantity": 50},
            ],
        })
        .to_string();
        let mut masters = Masters::default();
        parse_tables(&text, &mut masters).unwrap();
        masters.delivery.unwrap()
    }

    const PARTY: PartyMaster = PartyMaster {
        birthday_party_id: 1,
        delivery_item_material_id: 179,
        delivery_reward_mysekai_material_id: 72,
    };

    fn model() -> ServerModel {
        let mut model = super::super::tests::model();
        model.masters.delivery = Some(tables());
        model.joined = true;
        model
    }

    #[test]
    fn a_new_party_is_seated_by_the_named_policies() {
        let mut model = model();
        let changed = model.seat_parties(&[PARTY]);
        assert_eq!(changed, vec![SECTION_BIRTHDAY_PARTIES, SECTION_MATERIALS]);
        assert_eq!(
            model.doc.delivery.parties,
            vec![BirthdayPartyRow {
                birthday_party_id: 1,
                delivery_total_point: 0,
                dropped_mysekai_material_count: NEW_PARTY_DROPPED,
                obtained_mysekai_material_count: 0,
            }]
        );
        assert_eq!(model.doc.delivery.materials[&179], DELIVERY_ITEM_STOCK);
        assert!(model.seat_parties(&[PARTY]).is_empty());
    }

    #[test]
    fn totals_count_against_the_obtained_count_on_both_replies() {
        let mut model = model();
        model.seat_parties(&[PARTY]);
        model.doc.delivery.parties[0].dropped_mysekai_material_count = 5;
        // Base 100 with the owned bonus card (rate 50): 1.5 x 100 per item;
        // 200 items -> 30000 points -> 3 loops, all over the limit of 5.
        let (reply, _) = model.birthday_party_delivery(1, 200).unwrap();
        assert_eq!(reply.dropped_reward_count, 0);
        let row = model.doc.delivery.parties[0];
        assert_eq!(row.delivery_total_point, 30000);
        assert_eq!(row.obtained_mysekai_material_count, 3);
        assert_eq!(
            model.doc.delivery.materials[&179],
            DELIVERY_ITEM_STOCK - 200
        );
        assert_eq!(model.doc.delivery.mysekai_materials[&72], 3);
        // Obtained 0 -> 3 reaches the rows of requirement 2 and 3.
        assert_eq!(reply.obtained_delivery_total_rewards.len(), 4);
        assert_eq!(model.doc.delivery.honors.len(), 1);
        assert_eq!(model.doc.delivery.honors[0].level, 2);
        assert_eq!(model.doc.delivery.materials[&15], 50);
        // 0 -> 30000 passes three multiples of 10000.
        assert!(reply.is_refreshed);
        // A gather moves dropped to obtained; no row is newly reached.
        let (gather, _) = model
            .birthday_party_gather(&[GatherContent {
                birthday_party_id: 1,
                gathered_count: 2,
            }])
            .unwrap();
        assert!(gather.obtained_delivery_total_rewards.is_empty());
        let row = model.doc.delivery.parties[0];
        assert_eq!(row.dropped_mysekai_material_count, 3);
        assert_eq!(row.obtained_mysekai_material_count, 5);
    }

    #[test]
    fn a_gather_can_reach_a_total_reward() {
        let mut model = model();
        model.seat_parties(&[PARTY]);
        let (gather, changed) = model
            .birthday_party_gather(&[GatherContent {
                birthday_party_id: 1,
                gathered_count: 2,
            }])
            .unwrap();
        assert_eq!(gather.obtained_delivery_total_rewards.len(), 2);
        assert!(changed.contains(&SECTION_HONORS));
        assert_eq!(model.doc.delivery.honors[0].level, 1);
        assert!(model
            .birthday_party_gather(&[GatherContent {
                birthday_party_id: 9,
                gathered_count: 1,
            }])
            .is_err());
    }

    #[test]
    fn the_drop_limit_splits_dropped_and_obtained() {
        let mut model = model();
        model.seat_parties(&[PARTY]);
        model.doc.delivery.owned_cards = OwnedCards::Stated;
        model.doc.delivery.plant_refresh_points = 0;
        // No card owned: 100 per item; 250 items -> 25000 points -> 2 loops;
        // 3 dropped already under a limit of 5: both drop.
        let (reply, _) = model.birthday_party_delivery(1, 250).unwrap();
        assert_eq!(reply.dropped_reward_count, 2);
        assert!(!reply.is_refreshed);
        let row = model.doc.delivery.parties[0];
        assert_eq!(
            (
                row.dropped_mysekai_material_count,
                row.obtained_mysekai_material_count
            ),
            (5, 0)
        );
        assert!(reply.obtained_delivery_total_rewards.is_empty());
    }

    #[test]
    fn the_section_round_trips_and_is_optional() {
        let mut doc = DeliveryDoc::default();
        doc.parties.push(BirthdayPartyRow {
            birthday_party_id: 2,
            delivery_total_point: 5,
            dropped_mysekai_material_count: 1,
            obtained_mysekai_material_count: 4,
        });
        doc.materials.insert(176, 12);
        doc.cards = vec![3, 4];
        doc.owned_cards = OwnedCards::Stated;
        let (mut top, mut policies) = (Map::new(), Map::new());
        doc.write(&mut top, &mut policies);
        assert_eq!(DeliveryDoc::parse(&top, &policies).unwrap(), doc);
        assert_eq!(
            DeliveryDoc::parse(&Map::new(), &Map::new()).unwrap(),
            DeliveryDoc::default()
        );
        top.insert(SECTION_CARDS.into(), json!([{"cardId": 3}, {"cardId": 3}]));
        assert!(DeliveryDoc::parse(&top, &policies)
            .unwrap_err()
            .contains("repeats card 3"));
    }

    #[test]
    fn replies_merge_their_sections_into_the_client_copy() {
        let mut model = model();
        model.seat_parties(&[PARTY]);
        let update = model.delivery_update(&SECTIONS.map(str::to_owned));
        let mut client = ClientBirthdayPartyData::default();
        client.apply(update);
        assert_eq!(client.have_quantity(179), DELIVERY_ITEM_STOCK);
        assert_eq!(
            client
                .user_birthday_party(1)
                .unwrap()
                .dropped_mysekai_material_count,
            NEW_PARTY_DROPPED
        );
        assert!(client.cards().contains(&7));
        assert_eq!(
            client.master_config_int(CONFIG_BASE_POINT),
            Some(DEFAULT_BASE_POINT)
        );
        let (_, changed) = model.birthday_party_delivery(1, 10).unwrap();
        let update = model.delivery_reply(ResponseKind::BirthdayPartyDelivery, &changed);
        client.apply(update);
        assert_eq!(client.have_quantity(179), DELIVERY_ITEM_STOCK - 10);
        assert_eq!(
            client.user_birthday_party(1).unwrap().delivery_total_point,
            1500
        );
    }
}
