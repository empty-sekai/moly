//! The harvest's client copies of the user data: the stamina pools, the
//! tools, the harvest maps and treasure boxes, the Mysekai materials and the
//! possession values the capacity check reads.
//!
//! The response reader ([`super::ClientCopies`]) writes these copies; the
//! harvest reads them. The rules that read them with master data (the
//! capacity check, the stamina rules of the action loop) are the harvest's.

use std::collections::BTreeMap;

use bevy::prelude::*;
use moly_protocol::harvest::{UserHarvestMap, UserTreasureBox};

/// The client's stamina triple (`StaminaData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamina {
    pub normal: i32,
    pub enhance: i32,
    pub boost: i32,
}

impl Stamina {
    /// `StaminaData.Sum`: enhance + normal + boost.
    pub fn sum(self) -> i32 {
        self.enhance + self.normal + self.boost
    }

    /// `StaminaData.IsEmptyStamina`.
    pub fn is_empty(self) -> bool {
        self.normal <= 0 && self.boost <= 0 && self.enhance < 1
    }

    /// `StaminaData.HasBoostOrEnhance`.
    pub fn has_boost_or_enhance(self) -> bool {
        self.boost > 0 || self.enhance > 0
    }

    /// `HarvestPlayerModel.DecreaseStamina`: the amount comes out of boost
    /// first; what boost cannot cover (a non-positive remainder) goes to the
    /// normal pool while enhance is below one, else to enhance, and boost
    /// becomes zero.
    pub fn decrease(&mut self, amount: i32) {
        let remainder = self.boost - amount;
        if remainder <= 0 {
            if self.enhance < 1 {
                self.normal += remainder;
            } else {
                self.enhance += remainder;
            }
            self.boost = 0;
        } else {
            self.boost = remainder;
        }
    }
}

/// One `UserMysekaiTool` row as the harvest reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct UserTool {
    pub tool_id: i64,
    pub quantity: i32,
    pub durability: i32,
}

/// The server's `userMysekaiTools` rows as the harvest reads them.
pub fn user_tools(rows: &[moly_protocol::rows::UserMysekaiTool]) -> Vec<UserTool> {
    rows.iter()
        .map(|row| UserTool {
            tool_id: i64::from(row.mysekai_tool_id),
            quantity: row.quantity,
            durability: row.durability,
        })
        .collect()
}

/// The client's user data as the server mock last replied it (the login
/// fetch, then each API reply merged in).
#[derive(Resource)]
pub struct HarvestUserData {
    pub maps: BTreeMap<u32, UserHarvestMap>,
    pub tools: Vec<UserTool>,
    pub stamina: Stamina,
    pub materials: BTreeMap<i64, i64>,
    /// `userMysekaiTreasureBoxes`.
    pub boxes: Vec<UserTreasureBox>,
}

/// `PossessionMock`: the user possession values the capacity check reads.
/// The replies' `userMysekaiGamedata` possession levels,
/// `userMysekaiMaterialPossession` and `userMysekaiFixtures` sections set
/// them; until a reply carries them, both levels are 1 (the lowest master
/// level), there are no fixture rows, and the material quantity is 0 or the
/// value `MOLY_HARVEST_MOCK_MATERIAL_POSSESSION` seats (an unparsable value
/// is refused loudly and the 0 stands).
#[derive(Resource, Debug, Clone)]
pub struct PossessionMock {
    /// `userMysekaiGamedata.mysekaiMaterialPossessionLevel`.
    pub material_level: i32,
    /// `userMysekaiGamedata.mysekaiFixturePossessionLevel`.
    pub fixture_level: i32,
    /// `userMysekaiMaterialPossession.quantity`.
    pub material_quantity: i64,
    /// `userMysekaiFixtures` quantities by fixture id.
    pub fixtures: BTreeMap<i64, i64>,
}

impl Default for PossessionMock {
    fn default() -> Self {
        let mut material_quantity = 0;
        if let Ok(raw) = std::env::var("MOLY_HARVEST_MOCK_MATERIAL_POSSESSION") {
            match raw.trim().parse::<i64>() {
                Ok(value) if value >= 0 => material_quantity = value,
                _ => error!(
                    "[harvest-mock] MOLY_HARVEST_MOCK_MATERIAL_POSSESSION={raw:?} is not a quantity; the mock's 0 stands"
                ),
            }
        }
        Self {
            material_level: 1,
            fixture_level: 1,
            material_quantity,
            fixtures: BTreeMap::new(),
        }
    }
}
