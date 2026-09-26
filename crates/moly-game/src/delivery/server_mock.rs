//! `DeliveryServerMock`: the one panel for the server-decided delivery data.
//!
//! The client reads its birthday-party user rows and the have-quantity of the
//! delivery item as user data, and posts two APIs (the delivery and the
//! gather). None of the server's rules are in the client, so this panel
//! stands in for them. It is open while at least one party is in session
//! (`GetMasterBirthdayPartiesInSession` at the birthday clock; the
//! `MOLY_BIRTHDAY_NOW_MS` hook pins that clock), and every value it invents
//! is labelled here as the mock's own choice. The client path around it
//! (the rate, the carry, the accrual, the drop count, the limit, the annulus,
//! the drops and the gathering) is client code and is not mocked.
//!
//! Rows, as the server replies are shaped:
//! - Party in session: the in-session rows of the birthday-party master at
//!   the birthday clock; the panel is off when there is none.
//! - `HaveQuantityMock`: the have-quantity of each party's delivery item,
//!   [`HAVE_QUANTITY`] at login (the mock's choice).
//! - `UserBirthdayPartyMock`: per party, `deliveryTotalPoint` 0 at login,
//!   `droppedMysekaiMaterialCount` [`UNCLAIMED_DROPS`] at login (unclaimed
//!   drops the arrival re-creates on the ground; the mock's choice, so an
//!   arrival shows the pickup), `obtainedMysekaiMaterialCount` 0.
//! - `OwnedCardsMock`: the user owns every card a point bonus row of the
//!   party names (the mock's choice); the member bonus is the client's sum
//!   over those rows.
//! - `BasePointMock`: `birthday_party_delivery_base_point`, a master config
//!   not on disk: [`BASE_POINT`].
//! - `DropUpperLimitMock`: `birthday_party_delivery_reward_drop_upper_limit`,
//!   a master config not on disk: [`DROP_UPPER_LIMIT`].
//! - `DeliveryApiMock`: the reply to the delivery request. The mock's rule
//!   (the server's is not on disk): it spends the sent count (capped at what
//!   the user has), adds the points by the client's own accrual law, counts
//!   the reward loops the points newly reach, keeps as dropped those that fit
//!   under the upper limit beside the drops already on the ground and counts
//!   the rest as obtained (the client flew them to the player), and grants
//!   every total reward row of the party whose requirement the new reward
//!   loop count reaches for the first time (the requirement read as a reward
//!   loop count), with the contents of its box. `isRefreshed` is false.
//! - `GatherApiMock`: the reply to the gather request: each gathered drop
//!   leaves the party's dropped count and joins the materials; no total
//!   reward.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

use super::DropModel;
use crate::birthday::InSessionParty;

/// `HaveQuantityMock` at login.
pub(crate) const HAVE_QUANTITY: i32 = 300;
/// `droppedMysekaiMaterialCount` at login.
pub(crate) const UNCLAIMED_DROPS: i32 = 3;
/// `birthday_party_delivery_base_point`.
pub(crate) const BASE_POINT: i32 = 100;
/// `birthday_party_delivery_reward_drop_upper_limit`.
pub(crate) const DROP_UPPER_LIMIT: i32 = 5;

/// The honor arm of `PlayTotalRewardAnimation` (`ResourceType` honor).
pub(crate) const HONOR: &str = "honor";

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

/// One resource of a reply (`UserResource`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reward {
    pub(crate) resource_type: String,
    pub(crate) resource_id: i64,
    pub(crate) quantity: i32,
    pub(crate) level: Option<i64>,
}

/// The delivery tables (`birthday-party-delivery.json`).
#[derive(Resource)]
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

    fn box_contents(&self, box_id: i64) -> Vec<Reward> {
        self.boxes
            .iter()
            .filter(|(id, _)| *id == box_id)
            .map(|(_, reward)| reward.clone())
            .collect()
    }
}

#[derive(Resource)]
pub(crate) struct DeliveryTablesHandle(Handle<JsonAsset>);

/// Startup: request the delivery tables.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(DeliveryTablesHandle(
        server.load::<JsonAsset>(moly_assets::birthday_party_delivery()),
    ));
}

fn int(row: &serde_json::Value, table: &str, field: &str) -> i64 {
    row.get(field).and_then(|v| v.as_i64()).unwrap_or_else(|| {
        panic!(
            "delivery tables: {table} row {:?} has no integer {field}",
            row.get("id")
        )
    })
}

fn rows<'a>(doc: &'a serde_json::Value, key: &str) -> &'a Vec<serde_json::Value> {
    doc.get(key)
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("delivery tables: no {key} array"))
}

/// Update: parse the tables once loaded. A failed load or a missing field
/// stops loudly: the delivery cannot count points or rewards without them.
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<DeliveryTablesHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("delivery tables failed to load: {err:?}");
    }
    let Some(json) = jsons.get(&handle.0) else {
        return;
    };
    let doc: serde_json::Value = serde_json::from_str(&json.0)
        .unwrap_or_else(|err| panic!("delivery tables are not JSON: {err}"));
    let rewards = rows(&doc, "rewards")
        .iter()
        .map(|row| RewardRow {
            party: int(row, "rewards", "birthdayPartyId"),
            requirement: int(row, "rewards", "requirement") as i32,
        })
        .collect::<Vec<_>>();
    let bonuses = rows(&doc, "pointBonuses")
        .iter()
        .map(|row| BonusRow {
            party: int(row, "pointBonuses", "birthdayPartyId"),
            card: int(row, "pointBonuses", "cardId"),
            rate: int(row, "pointBonuses", "rate") as i32,
        })
        .collect::<Vec<_>>();
    let totals = rows(&doc, "totalRewards")
        .iter()
        .map(|row| TotalRow {
            id: int(row, "totalRewards", "id"),
            party: int(row, "totalRewards", "birthdayPartyId"),
            requirement: int(row, "totalRewards", "requirement") as i32,
            box_id: int(row, "totalRewards", "resourceBoxId"),
        })
        .collect::<Vec<_>>();
    let boxes = rows(&doc, "totalRewardBoxes")
        .iter()
        .map(|row| {
            (
                int(row, "totalRewardBoxes", "resourceBoxId"),
                Reward {
                    resource_type: row
                        .get("resourceType")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| {
                            panic!("delivery tables: a total reward box row has no resourceType")
                        })
                        .to_owned(),
                    resource_id: int(row, "totalRewardBoxes", "resourceId"),
                    quantity: int(row, "totalRewardBoxes", "resourceQuantity") as i32,
                    level: row.get("resourceLevel").and_then(|v| v.as_i64()),
                },
            )
        })
        .collect::<Vec<_>>();
    info!(
        "[delivery] tables ready: {} reward rows, {} point bonus rows, {} total reward rows, {} box rows",
        rewards.len(),
        bonuses.len(),
        totals.len(),
        boxes.len()
    );
    commands.insert_resource(DeliveryTables {
        rewards,
        bonuses,
        totals,
        boxes,
    });
    commands.remove_resource::<DeliveryTablesHandle>();
}

/// `UserBirthdayParty` of one party, as the panel holds it.
#[derive(Clone, Debug)]
pub(crate) struct MockParty {
    pub(crate) row: InSessionParty,
    pub(crate) delivery_total_point: i32,
    pub(crate) dropped_count: i32,
    pub(crate) obtained_count: i32,
    member_bonus: f32,
    requirement: i32,
}

/// The delivery reply (`UserBirthdayPartyDeliveryResponse`).
#[derive(Clone, Debug)]
pub(crate) struct DeliveryReply {
    pub(crate) dropped_reward_count: i32,
    pub(crate) total_rewards: Vec<Reward>,
    pub(crate) is_refreshed: bool,
}

#[derive(Resource)]
pub(crate) struct DeliveryServerMock {
    pub(crate) parties: Vec<MockParty>,
    /// The user's material have-quantities the delivery reads and changes.
    pub(crate) materials: BTreeMap<i64, i32>,
    pub(crate) base_point: i32,
    pub(crate) drop_upper_limit: i32,
    pub(crate) delivery_calls: usize,
    pub(crate) gather_calls: usize,
}

impl DeliveryServerMock {
    fn party(&self, id: i32) -> Option<&MockParty> {
        self.parties.iter().find(|p| p.row.id == id as i64)
    }

    fn party_mut(&mut self, id: i32) -> Option<&mut MockParty> {
        self.parties.iter_mut().find(|p| p.row.id == id as i64)
    }

    /// The rows `UpdateSynchronizedData` reads: the delivery item's
    /// have-quantity and the party's `deliveryTotalPoint`.
    pub(crate) fn user_rows(&self, party_id: i32, item: i64) -> (i32, i32) {
        (
            self.materials.get(&item).copied().unwrap_or(0),
            self.party(party_id).map_or(0, |p| p.delivery_total_point),
        )
    }

    pub(crate) fn dropped_count(&self, party_id: i32) -> i32 {
        self.party(party_id).map_or(0, |p| p.dropped_count)
    }

    pub(crate) fn obtained_count(&self, party_id: i32) -> i32 {
        self.party(party_id).map_or(0, |p| p.obtained_count)
    }

    /// The party constants the client builds its site data from.
    pub(crate) fn party_constants(&self, party_id: i32) -> Option<(f32, i32)> {
        self.party(party_id)
            .map(|p| (p.member_bonus, p.requirement))
    }

    /// `DeliveryApiMock` (the rule in the module comment).
    pub(crate) fn deliver(
        &mut self,
        party_id: i32,
        cost: i32,
        tables: &DeliveryTables,
    ) -> Option<DeliveryReply> {
        let base_point = self.base_point;
        let limit = self.drop_upper_limit;
        let item = self.party(party_id)?.row.delivery_item_material_id;
        let reward_material = self.party(party_id)?.row.delivery_reward_material_id;
        let have = self.materials.get(&item).copied().unwrap_or(0);
        let party = self.party_mut(party_id)?;
        let mut tally = moly_law::delivery::PartyTally {
            synchronized_cost_quantity: have,
            unsynchronized_cost: 0,
            synchronized_points: party.delivery_total_point,
            unsynchronized_points: 0,
            base_point,
            member_bonus: party.member_bonus,
            reward_loop_requirement: party.requirement,
            max_drop_item_count: limit,
        };
        let before_loops = tally.total_drop_count();
        let spent = tally.spend(cost);
        let after_loops = tally.total_drop_count();
        let new_loops = after_loops - before_loops;
        let free = (limit - party.dropped_count).max(0);
        let dropped = new_loops.min(free);
        let obtained = new_loops - dropped;
        party.delivery_total_point = tally.current_points();
        party.dropped_count += dropped;
        party.obtained_count += obtained;
        let party_key = party.row.id;
        let mut total_rewards = Vec::new();
        let mut granted = Vec::new();
        for row in tables.totals.iter().filter(|row| row.party == party_key) {
            if before_loops < row.requirement && row.requirement <= after_loops {
                granted.push((row.id, row.requirement, row.box_id));
                total_rewards.extend(tables.box_contents(row.box_id));
            }
        }
        *self.materials.entry(item).or_insert(0) -= spent;
        *self.materials.entry(reward_material).or_insert(0) += obtained;
        self.delivery_calls += 1;
        info!(
            "[delivery-api] PostUserBirthdayPartyDeliveryApi (DeliveryApiMock, the mock's rule) party {party_id}: sent {cost} items, spent {spent}; points {} -> {}; reward loops {before_loops} -> {after_loops} (requirement {}); dropped {dropped} (limit {limit}), obtained {obtained}; total reward rows granted {granted:?} -> {} resources",
            tally.synchronized_points,
            tally.current_points(),
            tally.reward_loop_requirement,
            total_rewards.len()
        );
        Some(DeliveryReply {
            dropped_reward_count: dropped,
            total_rewards,
            is_refreshed: false,
        })
    }

    /// `GatherApiMock`.
    pub(crate) fn gather(&mut self, drops: &[DropModel]) -> Vec<Reward> {
        for drop in drops {
            if let Some(party) = self.party_mut(drop.party_id) {
                party.dropped_count = (party.dropped_count - 1).max(0);
            }
            *self.materials.entry(drop.material_id).or_insert(0) += 1;
        }
        self.gather_calls += 1;
        info!(
            "[delivery-api] PostUserBirthdayPartyGatherApi (GatherApiMock) {} drops {:?}: dropped counts now {:?}; no total reward",
            drops.len(),
            drops.iter().map(|d| d.uid).collect::<Vec<_>>(),
            self.parties
                .iter()
                .map(|p| (p.row.id, p.dropped_count))
                .collect::<Vec<_>>()
        );
        Vec::new()
    }
}

/// Update: open the panel once the master and the tables are in, when a
/// party is in session at the birthday clock.
pub(crate) fn open_panel(
    mut commands: Commands,
    parties: Option<Res<crate::birthday::BirthdayParties>>,
    tables: Option<Res<DeliveryTables>>,
    panel: Option<Res<DeliveryServerMock>>,
    mut checked: Local<bool>,
) {
    if *checked || panel.is_some() {
        return;
    }
    let (Some(parties), Some(tables)) = (parties, tables) else {
        return;
    };
    *checked = true;
    let now = crate::birthday::now_ms();
    let rows = parties.in_session(now);
    if rows.is_empty() {
        info!(
            "[delivery] DeliveryServerMock off: no birthday party is in session at {now} (the panel opens only while one is)"
        );
        return;
    }
    let owned_cards: BTreeSet<i64> = tables
        .bonuses
        .iter()
        .filter(|row| rows.iter().any(|party| party.id == row.party))
        .map(|row| row.card)
        .collect();
    let mut materials = BTreeMap::new();
    let mut mock_parties = Vec::new();
    for row in &rows {
        materials.insert(row.delivery_item_material_id, HAVE_QUANTITY);
        let member_bonus = tables.member_bonus(row.id, &owned_cards);
        let requirement = tables.reward_loop_requirement(row.id);
        mock_parties.push(MockParty {
            row: row.clone(),
            delivery_total_point: 0,
            dropped_count: UNCLAIMED_DROPS,
            obtained_count: 0,
            member_bonus,
            requirement,
        });
    }
    info!(
        "[delivery] DeliveryServerMock on at {now}: parties in session {:?}; HaveQuantityMock {HAVE_QUANTITY} of {:?}; UserBirthdayPartyMock deliveryTotalPoint 0, droppedMysekaiMaterialCount {UNCLAIMED_DROPS} (unclaimed drops), obtained 0; OwnedCardsMock {} cards -> member bonus {:?}; BasePointMock {BASE_POINT}; DropUpperLimitMock {DROP_UPPER_LIMIT}; reward loop requirement {:?}",
        rows.iter().map(|r| (r.id, r.label.clone())).collect::<Vec<_>>(),
        materials.keys().collect::<Vec<_>>(),
        owned_cards.len(),
        mock_parties
            .iter()
            .map(|p| (p.row.id, p.member_bonus))
            .collect::<Vec<_>>(),
        mock_parties
            .iter()
            .map(|p| (p.row.id, p.requirement))
            .collect::<Vec<_>>(),
    );
    commands.insert_resource(DeliveryServerMock {
        parties: mock_parties,
        materials,
        base_point: BASE_POINT,
        drop_upper_limit: DROP_UPPER_LIMIT,
        delivery_calls: 0,
        gather_calls: 0,
    });
}
