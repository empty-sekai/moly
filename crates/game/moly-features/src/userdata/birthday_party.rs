//! The client's copy of the birthday-party delivery user data.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde_json::{json, Value};

pub use moly_protocol::json::{
    honor_value, party_value, quantities_value, SECTION_BIRTHDAY_PARTIES, SECTION_HONORS,
    SECTION_MASTER_CONFIGS, SECTION_MATERIALS, SECTION_MYSEKAI_MATERIALS,
};
pub use moly_protocol::rows::{BirthdayPartyRow, DeliveryUpdate, HonorRow};

/// `MasterDataManager.GetMasterConfigToInt` keys the delivery site reads.
pub const CONFIG_BASE_POINT: &str = "birthday_party_delivery_base_point";
pub const CONFIG_DROP_UPPER_LIMIT: &str = "birthday_party_delivery_reward_drop_upper_limit";
/// `ResourceType.honor` as a reward row names it.
pub const HONOR: &str = "honor";

/// The client's copies of the delivery user data (`UserDataManager`) and of
/// the two master configs (`MasterDataManager`), set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientBirthdayPartyData {
    parties: Vec<BirthdayPartyRow>,
    materials: BTreeMap<i32, i32>,
    mysekai_materials: BTreeMap<i32, i32>,
    cards: BTreeSet<i64>,
    honors: Vec<HonorRow>,
    configs: BTreeMap<String, i32>,
    /// Responses that carried a delivery section.
    pub revision: u64,
}

#[allow(dead_code)] // Read by the delivery owner's seam.
impl ClientBirthdayPartyData {
    /// `UserDataManager.GetUserBirthdayParty(id)`.
    pub fn user_birthday_party(&self, birthday_party_id: i32) -> Option<&BirthdayPartyRow> {
        self.parties
            .iter()
            .find(|row| row.birthday_party_id == birthday_party_id)
    }

    /// `UserDataUtility.GetHaveQuantity(id, ResourceType.material)`: 0
    /// without a row.
    pub fn have_quantity(&self, material_id: i32) -> i32 {
        self.materials.get(&material_id).copied().unwrap_or(0)
    }

    /// The `userMysekaiMaterials` quantity: 0 without a row.
    pub fn mysekai_material_quantity(&self, mysekai_material_id: i32) -> i32 {
        self.mysekai_materials
            .get(&mysekai_material_id)
            .copied()
            .unwrap_or(0)
    }

    /// The `userMysekaiMaterials` rows: mysekai material id -> quantity.
    pub fn mysekai_materials(&self) -> &BTreeMap<i32, i32> {
        &self.mysekai_materials
    }

    /// The card ids of `userCards` (`UserDataManager.GetCard(id) != null`).
    pub fn cards(&self) -> &BTreeSet<i64> {
        &self.cards
    }

    pub fn honors(&self) -> &[HonorRow] {
        &self.honors
    }

    /// `MasterDataManager.GetMasterConfigToInt(key)`; `None` before the
    /// join delivered it.
    pub fn master_config_int(&self, key: &str) -> Option<i32> {
        self.configs.get(key).copied()
    }

    /// `updatedResources`: the carried sections replace the copies.
    pub fn apply(&mut self, update: DeliveryUpdate) {
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

    pub fn view(&self) -> Value {
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
