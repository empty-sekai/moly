//! The fixture and material possession masters, and the count rules the
//! inventory and craft screens read the owned rows against.

use bevy::prelude::*;
use moly_protocol::rows::{UserMysekaiFixture, UserMysekaiMaterial};
use serde_json::Value;

/// `MasterMysekaiFixturePossession` / `MasterMysekaiMaterialPossession`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PossessionRow {
    pub level: i32,
    pub possession_limit: i32,
}

/// The possession masters (`mysekai-fixture-possessions.json`,
/// `mysekai-material-possessions.json`), master order.
#[derive(Resource, Clone, Debug, Default)]
pub struct PossessionMasters {
    pub fixture: Option<Vec<PossessionRow>>,
    pub material: Option<Vec<PossessionRow>>,
}

/// The sum of the rows' quantities (`GetFixtureTotalCount`; canvases are not
/// counted).
pub fn fixture_total_count(rows: &[UserMysekaiFixture]) -> i32 {
    rows.iter().map(|row| row.quantity).sum()
}

/// The quantity of a material row (0 without one).
pub fn material_quantity(rows: &[UserMysekaiMaterial], mysekai_material_id: i32) -> i32 {
    rows.iter()
        .find(|row| row.mysekai_material_id == mysekai_material_id)
        .map_or(0, |row| row.quantity)
}

fn first_limit(rows: Option<&Vec<PossessionRow>>, level: i32) -> Option<i32> {
    rows?
        .iter()
        .find(|row| row.level == level)
        .map(|row| row.possession_limit)
}

impl PossessionMasters {
    /// `MysekaiUserDataUtility.GetFixtureMaxCount(level)`: the first row of
    /// the level (`None` where the source would dereference a missing row).
    pub fn fixture_max_count(&self, level: i32) -> Option<i32> {
        first_limit(self.fixture.as_ref(), level)
    }

    /// `MysekaiUserDataUtility.GetMaterialMaxCount(level)`.
    pub fn material_max_count(&self, level: i32) -> Option<i32> {
        first_limit(self.material.as_ref(), level)
    }
}

/// The possession rows of a fixture or material possession master.
pub fn parse_possession_rows(rows: &[Value]) -> Result<Vec<PossessionRow>, String> {
    rows.iter()
        .map(|row| {
            Ok(PossessionRow {
                level: crate::int32(row, "level")?,
                possession_limit: crate::int32(row, "possessionLimit")?,
            })
        })
        .collect()
}
