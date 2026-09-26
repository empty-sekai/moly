//! The client's copies of the user's owned MySekai data and the possession
//! masters the inventory and craft screens read them against.
//!
//! The rows are the source's `SuiteUser` tables: `userMysekaiFixtures`
//! (`UserMysekaiFixture`), `userMysekaiCanvases` (`UserMysekaiCanvas`),
//! `userMysekaiBlueprints` (`UserMysekaiBlueprint`), `userMysekaiItems`
//! (`UserMysekaiItem`), `userMysekaiMaterials` (`UserMysekaiMaterial`, the
//! id and quantity the server document holds), `userMysekaiCharacterTalks`
//! (`UserMysekaiCharacterTalk`, the talks' read records),
//! `userMysekaiMaterialPossession`
//! (`UserMysekaiMaterialPossession`) and the `userMysekaiGamedata` fields the
//! two screens read (rank, total experience and the two possession levels).
//! `UserDataManager.UpdateAll` replaces each table a reply carries as a whole,
//! and so does [`ClientMysekaiInventory::apply`].
//!
//! The server model is the only writer: its responses and the craft and
//! sketch replies set the copy, and it inserts [`PossessionMasters`] once its
//! masters have resolved (each list is `None` when the runtime root lacks its
//! master). The `userMysekaiMaterials` copy here is a read view of the one
//! the birthday-party delivery also reads; the server model keeps the two
//! equal.

// The inventory and craft screens read these.
#![allow(dead_code)]

use bevy::prelude::*;
use serde_json::{json, Value};

use super::talk_read::UserMysekaiCharacterTalk;

/// `UserMysekaiFixture`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiFixture {
    pub(crate) mysekai_fixture_id: i32,
    pub(crate) texture_id: i32,
    pub(crate) quantity: i32,
    pub(crate) last_obtained_at: i64,
}

/// `UserMysekaiCanvas`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiCanvas {
    pub(crate) mysekai_fixture_id: i32,
    pub(crate) card_id: i32,
    pub(crate) is_special_training: bool,
    pub(crate) quantity: i32,
    pub(crate) last_obtained_at: i64,
}

/// `UserMysekaiBlueprint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiBlueprint {
    pub(crate) mysekai_blueprint_id: i32,
    pub(crate) obtained_at: i64,
}

/// `UserMysekaiItem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiItem {
    pub(crate) mysekai_item_id: i32,
    pub(crate) quantity: i32,
    pub(crate) last_obtained_at: i64,
}

/// `UserMysekaiMaterial` (id and quantity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiMaterial {
    pub(crate) mysekai_material_id: i32,
    pub(crate) quantity: i32,
}

/// `UserMysekaiMaterialPossession`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiMaterialPossession {
    pub(crate) quantity: i32,
}

/// The `UserMysekaiGamedata` fields the inventory and craft screens read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiGamedata {
    /// `None` until the server has seated the rank from `totalExp`.
    pub(crate) mysekai_rank: Option<i32>,
    pub(crate) total_exp: i32,
    pub(crate) mysekai_material_possession_level: i32,
    pub(crate) mysekai_fixture_possession_level: i32,
}

/// The `SuiteUser` tables a response or reply carries; `None` for a table it
/// does not carry. Applying it replaces each carried table whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SuiteUserSections {
    pub(crate) user_mysekai_character_talks: Option<Vec<UserMysekaiCharacterTalk>>,
    pub(crate) user_mysekai_materials: Option<Vec<UserMysekaiMaterial>>,
    pub(crate) user_mysekai_material_possession: Option<UserMysekaiMaterialPossession>,
    pub(crate) user_mysekai_fixtures: Option<Vec<UserMysekaiFixture>>,
    pub(crate) user_mysekai_canvases: Option<Vec<UserMysekaiCanvas>>,
    pub(crate) user_mysekai_blueprints: Option<Vec<UserMysekaiBlueprint>>,
    pub(crate) user_mysekai_items: Option<Vec<UserMysekaiItem>>,
    pub(crate) user_mysekai_gamedata: Option<UserMysekaiGamedata>,
}

impl SuiteUserSections {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `MasterMysekaiFixturePossession` / `MasterMysekaiMaterialPossession`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PossessionRow {
    pub(crate) level: i32,
    pub(crate) possession_limit: i32,
}

/// The possession masters (`mysekai-fixture-possessions.json`,
/// `mysekai-material-possessions.json`), master order.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct PossessionMasters {
    pub(crate) fixture: Option<Vec<PossessionRow>>,
    pub(crate) material: Option<Vec<PossessionRow>>,
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
    pub(crate) fn fixture_max_count(&self, level: i32) -> Option<i32> {
        first_limit(self.fixture.as_ref(), level)
    }

    /// `MysekaiUserDataUtility.GetMaterialMaxCount(level)`.
    pub(crate) fn material_max_count(&self, level: i32) -> Option<i32> {
        first_limit(self.material.as_ref(), level)
    }
}

/// The client's copy, set by the server model only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientMysekaiInventory {
    fixtures: Vec<UserMysekaiFixture>,
    canvases: Vec<UserMysekaiCanvas>,
    blueprints: Vec<UserMysekaiBlueprint>,
    items: Vec<UserMysekaiItem>,
    materials: Vec<UserMysekaiMaterial>,
    material_possession: Option<UserMysekaiMaterialPossession>,
    gamedata: Option<UserMysekaiGamedata>,
    character_talks: Vec<UserMysekaiCharacterTalk>,
    /// Applications that carried a table (0: none yet).
    pub(crate) revision: u64,
}

impl ClientMysekaiInventory {
    pub(crate) fn fixtures(&self) -> &[UserMysekaiFixture] {
        &self.fixtures
    }

    pub(crate) fn canvases(&self) -> &[UserMysekaiCanvas] {
        &self.canvases
    }

    pub(crate) fn blueprints(&self) -> &[UserMysekaiBlueprint] {
        &self.blueprints
    }

    pub(crate) fn items(&self) -> &[UserMysekaiItem] {
        &self.items
    }

    pub(crate) fn materials(&self) -> &[UserMysekaiMaterial] {
        &self.materials
    }

    /// `userMysekaiCharacterTalks`: the talks' read records.
    pub(crate) fn character_talks(&self) -> &[UserMysekaiCharacterTalk] {
        &self.character_talks
    }

    /// Whether a talk's read record says it has been read.
    pub(crate) fn is_talk_read(&self, mysekai_character_talk_id: i32) -> bool {
        self.character_talks
            .iter()
            .any(|row| row.mysekai_character_talk_id == mysekai_character_talk_id && row.is_read)
    }

    /// `userMysekaiGamedata` as last carried (`None` before the join).
    pub(crate) fn gamedata(&self) -> Option<UserMysekaiGamedata> {
        self.gamedata
    }

    /// `UserDataManager.GetUserMysekaiFixture(id)`: the first row of the
    /// fixture, any texture.
    pub(crate) fn fixture(&self, mysekai_fixture_id: i32) -> Option<&UserMysekaiFixture> {
        self.fixtures
            .iter()
            .find(|row| row.mysekai_fixture_id == mysekai_fixture_id)
    }

    /// `UserDataManager.GetUserMysekaiFixture(id, textureId)`.
    pub(crate) fn fixture_of_texture(
        &self,
        mysekai_fixture_id: i32,
        texture_id: i32,
    ) -> Option<&UserMysekaiFixture> {
        self.fixtures.iter().find(|row| {
            row.mysekai_fixture_id == mysekai_fixture_id && row.texture_id == texture_id
        })
    }

    /// `MysekaiUserDataUtility.GetFixtureTotalCount`: the sum of every
    /// fixture row's quantity (canvases are not counted).
    pub(crate) fn fixture_total_count(&self) -> i32 {
        fixture_total_count(&self.fixtures)
    }

    /// The canvases of a fixture summed (`CraftPreview.SetCanvasQuantityText`).
    pub(crate) fn canvas_quantity(&self, mysekai_fixture_id: i32) -> i32 {
        self.canvases
            .iter()
            .filter(|row| row.mysekai_fixture_id == mysekai_fixture_id)
            .map(|row| row.quantity)
            .sum()
    }

    /// `SketchUtility.HasTargetBluePrint`.
    pub(crate) fn has_blueprint(&self, mysekai_blueprint_id: i32) -> bool {
        self.blueprints
            .iter()
            .any(|row| row.mysekai_blueprint_id == mysekai_blueprint_id)
    }

    /// The quantity of an item row (0 without one).
    pub(crate) fn item_quantity(&self, mysekai_item_id: i32) -> i32 {
        self.items
            .iter()
            .find(|row| row.mysekai_item_id == mysekai_item_id)
            .map_or(0, |row| row.quantity)
    }

    /// The quantity of a `userMysekaiMaterials` row (0 without one).
    pub(crate) fn material_quantity(&self, mysekai_material_id: i32) -> i32 {
        material_quantity(&self.materials, mysekai_material_id)
    }

    /// `MysekaiUserDataUtility.GetTotalMaterialPossession`: the possession
    /// row's quantity, 0 without one.
    pub(crate) fn total_material_possession(&self) -> i32 {
        self.material_possession.map_or(0, |row| row.quantity)
    }

    /// `MysekaiUserDataUtility.GetFixtureMaxCount()`: the limit of the
    /// gamedata's fixture possession level; 0 without gamedata.
    pub(crate) fn fixture_max_count(&self, masters: &PossessionMasters) -> Option<i32> {
        match self.gamedata {
            None => Some(0),
            Some(gamedata) => masters.fixture_max_count(gamedata.mysekai_fixture_possession_level),
        }
    }

    /// `MysekaiUserDataUtility.GetMaterialMaxCount()`.
    pub(crate) fn material_max_count(&self, masters: &PossessionMasters) -> Option<i32> {
        masters.material_max_count(self.gamedata?.mysekai_material_possession_level)
    }

    /// `MysekaiUserDataUtility.GetRemainingFixtureCapacity`: the fixture
    /// limit less the total count, never below 0.
    pub(crate) fn remaining_fixture_capacity(&self, masters: &PossessionMasters) -> Option<i32> {
        Some((self.fixture_max_count(masters)? - self.fixture_total_count()).max(0))
    }

    /// `UserDataManager.UpdateAll`: each carried table replaces the copy.
    pub(crate) fn apply(&mut self, sections: SuiteUserSections) {
        if sections.is_empty() {
            return;
        }
        let SuiteUserSections {
            user_mysekai_character_talks,
            user_mysekai_materials,
            user_mysekai_material_possession,
            user_mysekai_fixtures,
            user_mysekai_canvases,
            user_mysekai_blueprints,
            user_mysekai_items,
            user_mysekai_gamedata,
        } = sections;
        if let Some(rows) = user_mysekai_materials {
            self.materials = rows;
        }
        if let Some(row) = user_mysekai_material_possession {
            self.material_possession = Some(row);
        }
        if let Some(rows) = user_mysekai_fixtures {
            self.fixtures = rows;
        }
        if let Some(rows) = user_mysekai_canvases {
            self.canvases = rows;
        }
        if let Some(rows) = user_mysekai_blueprints {
            self.blueprints = rows;
        }
        if let Some(rows) = user_mysekai_items {
            self.items = rows;
        }
        if let Some(gamedata) = user_mysekai_gamedata {
            self.gamedata = Some(gamedata);
        }
        if let Some(rows) = user_mysekai_character_talks {
            self.character_talks = rows;
        }
        self.revision += 1;
    }

    /// The copy as the page's document view shows it.
    pub(crate) fn view(&self) -> Value {
        json!({
            "userMysekaiFixtures": fixtures_value(&self.fixtures),
            "userMysekaiCanvases": canvases_value(&self.canvases),
            "userMysekaiBlueprintCount": self.blueprints.len(),
            "userMysekaiItems": items_value(&self.items),
            "userMysekaiMaterials": materials_value(&self.materials),
            "userMysekaiMaterialPossession": self.material_possession.map(|row| json!({"quantity": row.quantity})),
            "userMysekaiGamedata": self.gamedata.map(gamedata_value),
            "userMysekaiCharacterTalkReadCount": self.character_talks.iter().filter(|row| row.is_read).count(),
            "revision": self.revision,
        })
    }
}

/// The sum of the rows' quantities (`GetFixtureTotalCount`).
pub(crate) fn fixture_total_count(rows: &[UserMysekaiFixture]) -> i32 {
    rows.iter().map(|row| row.quantity).sum()
}

/// The quantity of a material row (0 without one).
pub(crate) fn material_quantity(rows: &[UserMysekaiMaterial], mysekai_material_id: i32) -> i32 {
    rows.iter()
        .find(|row| row.mysekai_material_id == mysekai_material_id)
        .map_or(0, |row| row.quantity)
}

// ---------------------------------------------------------------------------
// The response keys' JSON
// ---------------------------------------------------------------------------

pub(crate) fn fixtures_value(rows: &[UserMysekaiFixture]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiFixtureId": row.mysekai_fixture_id,
                    "textureId": row.texture_id,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

pub(crate) fn canvases_value(rows: &[UserMysekaiCanvas]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiFixtureId": row.mysekai_fixture_id,
                    "cardId": row.card_id,
                    "isSpecialTraining": row.is_special_training,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

pub(crate) fn blueprints_value(rows: &[UserMysekaiBlueprint]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiBlueprintId": row.mysekai_blueprint_id,
                    "obtainedAt": row.obtained_at,
                })
            })
            .collect(),
    )
}

pub(crate) fn items_value(rows: &[UserMysekaiItem]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiItemId": row.mysekai_item_id,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

pub(crate) fn materials_value(rows: &[UserMysekaiMaterial]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| json!({"mysekaiMaterialId": row.mysekai_material_id, "quantity": row.quantity}))
            .collect(),
    )
}

pub(crate) fn character_talks_value(rows: &[UserMysekaiCharacterTalk]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({"mysekaiCharacterTalkId": row.mysekai_character_talk_id, "isRead": row.is_read})
            })
            .collect(),
    )
}

pub(crate) fn gamedata_value(gamedata: UserMysekaiGamedata) -> Value {
    json!({
        "mysekaiRank": gamedata.mysekai_rank,
        "totalExp": gamedata.total_exp,
        "mysekaiMaterialPossessionLevel": gamedata.mysekai_material_possession_level,
        "mysekaiFixturePossessionLevel": gamedata.mysekai_fixture_possession_level,
    })
}
