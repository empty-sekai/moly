//! The birthday-party delivery master tables: the reward rows and point
//! bonus rows the client reads, and the total reward rows the server grants
//! from.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde_json::Value;

/// One `birthdayPartyDeliveryRewards` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardRow {
    party: i64,
    requirement: i32,
}

/// One `birthdayPartyDeliveryPointBonuses` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BonusRow {
    party: i64,
    card: i64,
    rate: i32,
}

/// One `birthdayPartyDeliveryTotalRewards` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TotalRow {
    id: i64,
    party: i64,
    requirement: i32,
    box_id: i64,
}

/// The delivery master tables. The server model reads them as masters and
/// inserts this resource for the client's readers.
#[derive(Resource, Clone, Debug)]
pub struct DeliveryTables {
    rewards: Vec<RewardRow>,
    bonuses: Vec<BonusRow>,
    totals: Vec<TotalRow>,
}

impl DeliveryTables {
    /// The three tables, once all of them are in.
    pub fn new(rewards: Vec<RewardRow>, bonuses: Vec<BonusRow>, totals: Vec<TotalRow>) -> Self {
        Self {
            rewards,
            bonuses,
            totals,
        }
    }

    /// The number of reward, point bonus and total reward rows.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.rewards.len(), self.bonuses.len(), self.totals.len())
    }

    /// The reward loop requirement of a party: its last reward row's, or 1.
    pub fn reward_loop_requirement(&self, party: i64) -> i32 {
        let rows: Vec<i32> = self
            .rewards
            .iter()
            .filter(|row| row.party == party)
            .map(|row| row.requirement)
            .collect();
        moly_law::delivery::reward_loop_requirement(&rows)
    }

    /// `MemberBonus`: the rates of the bonus rows whose card the user owns.
    pub fn member_bonus(&self, party: i64, owned: &BTreeSet<i64>) -> f32 {
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
    pub fn bonus_cards(&self) -> BTreeSet<i64> {
        self.bonuses.iter().map(|row| row.card).collect()
    }

    /// The total reward rows of a party an obtained count newly reaches:
    /// `before < requirement <= after`, master order; each as (row id,
    /// requirement, box id).
    pub fn totals_reached(&self, party: i64, before: i32, after: i32) -> Vec<(i64, i32, i64)> {
        self.totals
            .iter()
            .filter(|row| {
                row.party == party && before < row.requirement && row.requirement <= after
            })
            .map(|row| (row.id, row.requirement, row.box_id))
            .collect()
    }
}

/// The integer field of a table row, named by the row's id.
fn table_int(row: &Value, table: &str, key: &str) -> Result<i64, String> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{table} row {:?} has no integer {key}", row.get("id")))
}

/// `birthdayPartyDeliveryRewards`, master order.
pub fn parse_rewards(rows: &[Value]) -> Result<Vec<RewardRow>, String> {
    const TABLE: &str = "birthdayPartyDeliveryRewards";
    rows.iter()
        .map(|row| {
            Ok(RewardRow {
                party: table_int(row, TABLE, "birthdayPartyId")?,
                requirement: table_int(row, TABLE, "requirement")? as i32,
            })
        })
        .collect()
}

/// `birthdayPartyDeliveryPointBonuses`, master order.
pub fn parse_point_bonuses(rows: &[Value]) -> Result<Vec<BonusRow>, String> {
    const TABLE: &str = "birthdayPartyDeliveryPointBonuses";
    rows.iter()
        .map(|row| {
            Ok(BonusRow {
                party: table_int(row, TABLE, "birthdayPartyId")?,
                card: table_int(row, TABLE, "cardId")?,
                rate: table_int(row, TABLE, "rate")? as i32,
            })
        })
        .collect()
}

/// `birthdayPartyDeliveryTotalRewards`, master order.
pub fn parse_total_rewards(rows: &[Value]) -> Result<Vec<TotalRow>, String> {
    const TABLE: &str = "birthdayPartyDeliveryTotalRewards";
    rows.iter()
        .map(|row| {
            Ok(TotalRow {
                id: table_int(row, TABLE, "id")?,
                party: table_int(row, TABLE, "birthdayPartyId")?,
                requirement: table_int(row, TABLE, "requirement")? as i32,
                box_id: table_int(row, TABLE, "resourceBoxId")?,
            })
        })
        .collect()
}
