//! The client's copy of the birthday-party delivery user data.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde_json::{json, Value};

use crate::server::{with_model, ResponseKind};

pub(crate) use moly_protocol::api::{DeliveryResponse, GatherContent, GatherResponse};
pub(crate) use moly_protocol::json::{
    honor_value, party_value, quantities_value, SECTION_BIRTHDAY_PARTIES, SECTION_HONORS,
    SECTION_MASTER_CONFIGS, SECTION_MATERIALS, SECTION_MYSEKAI_MATERIALS,
};
pub(crate) use moly_protocol::rows::{BirthdayPartyRow, DeliveryUpdate, HonorRow};

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

    /// The `userMysekaiMaterials` rows: mysekai material id -> quantity.
    pub(crate) fn mysekai_materials(&self) -> &BTreeMap<i32, i32> {
        &self.mysekai_materials
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

/// `PutUserMysekaiBirthdayPartyDeliveryApi`: the reply, with its
/// `updatedResources` merged into the client's copies. The in-process model re-exports it.
#[allow(dead_code)]
pub(crate) fn put_birthday_party_delivery(
    client: &mut ClientBirthdayPartyData,
    birthday_party_id: i32,
    consumed_delivery_material_count: i32,
) -> Result<DeliveryResponse, String> {
    let (response, update) = with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let (response, changed) =
            model.birthday_party_delivery(birthday_party_id, consumed_delivery_material_count)?;
        let update = model.delivery_reply(ResponseKind::BirthdayPartyDelivery, &changed);
        crate::bus::emit(
            "delivery_done",
            serde_json::json!({
                "party_id": birthday_party_id,
                "consumed": consumed_delivery_material_count,
            }),
        );
        Ok((response, update))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()))
    .map_err(|reason| format!("PutUserMysekaiBirthdayPartyDeliveryApi: {reason}"))?;
    client.apply(update);
    Ok(response)
}

/// `PutUserMysekaiBirthdayPartyGatherApi`: the reply, with its
/// `updatedResources` merged into the client's copies. The in-process model re-exports it.
#[allow(dead_code)]
pub(crate) fn put_birthday_party_gather(
    client: &mut ClientBirthdayPartyData,
    contents: &[GatherContent],
) -> Result<GatherResponse, String> {
    let (response, update) = with_model(|model| {
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
