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
//! The responses and the craft and sketch replies set the copy.
//! [`PossessionMasters`] holds the possession limits once their master has
//! resolved (each list is `None` when the runtime root lacks its master). The
//! `userMysekaiMaterials` copy here is a read view of the one the
//! birthday-party delivery also reads; both are set from the same response
//! sections.

// The inventory and craft screens read these.
#![allow(dead_code)]

use bevy::prelude::*;
use serde_json::{json, Value};

use super::talk_read::UserMysekaiCharacterTalk;

pub use moly_masters::inventory::{
    fixture_total_count, material_quantity, PossessionMasters,
};
pub use moly_protocol::json::{
    canvases_value, fixtures_value, items_value, materials_value,
    user_gamedata_value as gamedata_value,
};
pub use moly_protocol::rows::{
    UserMysekaiFixture, UserMysekaiCanvas, UserMysekaiBlueprint, UserMysekaiItem,
    UserMysekaiMaterial, UserMysekaiMaterialPossession, UserMysekaiGamedata, SuiteUserSections,
};

/// The client's copy, set by the server model only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientMysekaiInventory {
    fixtures: Vec<UserMysekaiFixture>,
    canvases: Vec<UserMysekaiCanvas>,
    blueprints: Vec<UserMysekaiBlueprint>,
    items: Vec<UserMysekaiItem>,
    materials: Vec<UserMysekaiMaterial>,
    material_possession: Option<UserMysekaiMaterialPossession>,
    gamedata: Option<UserMysekaiGamedata>,
    character_talks: Vec<UserMysekaiCharacterTalk>,
    /// Applications that carried a table (0: none yet).
    pub revision: u64,
}

impl ClientMysekaiInventory {
    pub fn fixtures(&self) -> &[UserMysekaiFixture] {
        &self.fixtures
    }

    pub fn canvases(&self) -> &[UserMysekaiCanvas] {
        &self.canvases
    }

    pub fn blueprints(&self) -> &[UserMysekaiBlueprint] {
        &self.blueprints
    }

    pub fn items(&self) -> &[UserMysekaiItem] {
        &self.items
    }

    pub fn materials(&self) -> &[UserMysekaiMaterial] {
        &self.materials
    }

    /// `userMysekaiCharacterTalks`: the talks' read records.
    pub fn character_talks(&self) -> &[UserMysekaiCharacterTalk] {
        &self.character_talks
    }

    /// Whether a talk's read record says it has been read.
    pub fn is_talk_read(&self, mysekai_character_talk_id: i32) -> bool {
        self.character_talks
            .iter()
            .any(|row| row.mysekai_character_talk_id == mysekai_character_talk_id && row.is_read)
    }

    /// `userMysekaiGamedata` as last carried (`None` before the join).
    pub fn gamedata(&self) -> Option<UserMysekaiGamedata> {
        self.gamedata
    }

    /// `UserDataManager.GetUserMysekaiFixture(id)`: the first row of the
    /// fixture, any texture.
    pub fn fixture(&self, mysekai_fixture_id: i32) -> Option<&UserMysekaiFixture> {
        self.fixtures
            .iter()
            .find(|row| row.mysekai_fixture_id == mysekai_fixture_id)
    }

    /// `UserDataManager.GetUserMysekaiFixture(id, textureId)`.
    pub fn fixture_of_texture(
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
    pub fn fixture_total_count(&self) -> i32 {
        fixture_total_count(&self.fixtures)
    }

    /// The canvases of a fixture summed (`CraftPreview.SetCanvasQuantityText`).
    pub fn canvas_quantity(&self, mysekai_fixture_id: i32) -> i32 {
        self.canvases
            .iter()
            .filter(|row| row.mysekai_fixture_id == mysekai_fixture_id)
            .map(|row| row.quantity)
            .sum()
    }

    /// `SketchUtility.HasTargetBluePrint`.
    pub fn has_blueprint(&self, mysekai_blueprint_id: i32) -> bool {
        self.blueprints
            .iter()
            .any(|row| row.mysekai_blueprint_id == mysekai_blueprint_id)
    }

    /// The quantity of an item row (0 without one).
    pub fn item_quantity(&self, mysekai_item_id: i32) -> i32 {
        self.items
            .iter()
            .find(|row| row.mysekai_item_id == mysekai_item_id)
            .map_or(0, |row| row.quantity)
    }

    /// The quantity of a `userMysekaiMaterials` row (0 without one).
    pub fn material_quantity(&self, mysekai_material_id: i32) -> i32 {
        material_quantity(&self.materials, mysekai_material_id)
    }

    /// `MysekaiUserDataUtility.GetTotalMaterialPossession`: the possession
    /// row's quantity, 0 without one.
    pub fn total_material_possession(&self) -> i32 {
        self.material_possession.map_or(0, |row| row.quantity)
    }

    /// `MysekaiUserDataUtility.GetFixtureMaxCount()`: the limit of the
    /// gamedata's fixture possession level; 0 without gamedata.
    pub fn fixture_max_count(&self, masters: &PossessionMasters) -> Option<i32> {
        match self.gamedata {
            None => Some(0),
            Some(gamedata) => masters.fixture_max_count(gamedata.mysekai_fixture_possession_level),
        }
    }

    /// `MysekaiUserDataUtility.GetMaterialMaxCount()`.
    pub fn material_max_count(&self, masters: &PossessionMasters) -> Option<i32> {
        masters.material_max_count(self.gamedata?.mysekai_material_possession_level)
    }

    /// `MysekaiUserDataUtility.GetRemainingFixtureCapacity`: the fixture
    /// limit less the total count, never below 0.
    pub fn remaining_fixture_capacity(&self, masters: &PossessionMasters) -> Option<i32> {
        Some((self.fixture_max_count(masters)? - self.fixture_total_count()).max(0))
    }

    /// `UserDataManager.UpdateAll`: each carried table replaces the copy.
    pub fn apply(&mut self, sections: SuiteUserSections) {
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
            ..
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
    pub fn view(&self) -> Value {
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

/// The birthday-party delivery merges its replies' `userMysekaiMaterials`
/// straight into its copy; this keeps the inventory copy's read view equal.
pub fn mirror_materials(
    birthday: Res<super::birthday_party::ClientBirthdayPartyData>,
    mut inventory: ResMut<ClientMysekaiInventory>,
) {
    if !birthday.is_changed() {
        return;
    }
    let rows: Vec<UserMysekaiMaterial> = birthday
        .mysekai_materials()
        .iter()
        .map(|(id, quantity)| UserMysekaiMaterial {
            mysekai_material_id: *id,
            quantity: *quantity,
        })
        .collect();
    if rows != inventory.materials() {
        inventory.apply(SuiteUserSections {
            user_mysekai_materials: Some(rows),
            ..Default::default()
        });
    }
}
