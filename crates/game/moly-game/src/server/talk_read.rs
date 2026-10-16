//! The talk read report (`PutUserMysekaiCharacterTalkReadApi`, an empty
//! body naming the talk) and its reply (`UserMysekaiCharacterTalkReadResponse`:
//! `obtainedResources` and `updatedResources`). The client sends it through
//! [`super::client::talk_read::put_talk_read`]; [`install`] puts the endpoint
//! in place.
//!
//! **Named policies** (the server's rules are not in the client):
//! - *Talk read record*: the reply sets the talk's `userMysekaiCharacterTalks`
//!   row to `isRead` true (adding the row), the read record the character
//!   archive reads (`MysekaiState` groups the rows by talk and the archive
//!   cells ask whether one is read). The talk id is taken as stated: the
//!   server model does not read the talk master.
//! - *Talk read resources*: `obtainedResources` is
//!   `policies.talkReadObtainedResources` (none by default: no master table
//!   names a talk reward; the master's reward and obtain tables are for
//!   events, gacha, lessons, missions, convert and birthday parties).
//!   A `mysekai_material`, `material` or `mysekai_item` row joins the owned
//!   table; the other resource types are carried in the reply only (the
//!   document does not track them).

use bevy::prelude::*;

use super::client::inventory::ClientMysekaiInventory;
use super::client::talk_read::{TalkReadEndpoint, TalkReadReply, UserResource};
use super::delivery::{ClientBirthdayPartyData, SECTION_MATERIALS, SECTION_MYSEKAI_MATERIALS};
use super::inventory::{SECTION_CHARACTER_TALKS, SECTION_ITEMS};
use super::{ResponseKind, ServerModel};

const API: &str = "PutUserMysekaiCharacterTalkReadApi";

impl ServerModel {
    /// The talk read policies: the resources and the sections the reply
    /// carries.
    fn talk_read(
        &mut self,
        mysekai_character_talk_id: i32,
    ) -> Result<(Vec<UserResource>, Vec<&'static str>), String> {
        if mysekai_character_talk_id < 1 {
            return Err(format!(
                "mysekaiCharacterTalkId {mysekai_character_talk_id} is not a talk id"
            ));
        }
        let now = self.now_ms();
        let inventory = &mut self.doc.inventory;
        match inventory
            .character_talks
            .iter_mut()
            .find(|row| row.mysekai_character_talk_id == mysekai_character_talk_id)
        {
            Some(row) => row.is_read = true,
            None => {
                inventory
                    .character_talks
                    .push(super::client::talk_read::UserMysekaiCharacterTalk {
                        mysekai_character_talk_id,
                        is_read: true,
                    })
            }
        }
        let resources = inventory.talk_read_resources.clone();
        let mut changed = vec![SECTION_CHARACTER_TALKS];
        let mut untracked = Vec::new();
        for resource in &resources {
            let (id, quantity) = (resource.resource_id, resource.quantity);
            let section = match resource.resource_type.as_str() {
                "mysekai_material" => {
                    *self.doc.delivery.mysekai_materials.entry(id).or_insert(0) += quantity;
                    SECTION_MYSEKAI_MATERIALS
                }
                "material" => {
                    *self.doc.delivery.materials.entry(id).or_insert(0) += quantity;
                    SECTION_MATERIALS
                }
                "mysekai_item" => {
                    let items = &mut self.doc.inventory.items;
                    match items.iter_mut().find(|row| row.mysekai_item_id == id) {
                        Some(row) => {
                            row.quantity += quantity;
                            row.last_obtained_at = now;
                        }
                        None => items.push(super::client::inventory::UserMysekaiItem {
                            mysekai_item_id: id,
                            quantity,
                            last_obtained_at: now,
                        }),
                    }
                    SECTION_ITEMS
                }
                other => {
                    untracked.push(format!("{other} {id} x{quantity}"));
                    continue;
                }
            };
            if !changed.contains(&section) {
                changed.push(section);
            }
        }
        self.commit();
        info!(
            "[server] {API}: talk {mysekai_character_talk_id} read; obtainedResources {} (policies.talkReadObtainedResources); not tracked by the document: {untracked:?}",
            resources.len()
        );
        Ok((resources, changed))
    }
}

/// The talk read endpoint ([`super::client::talk_read::put_talk_read`]).
pub(super) fn handle(
    In(mysekai_character_talk_id): In<i32>,
    mut inventory: ResMut<ClientMysekaiInventory>,
    mut birthday: ResMut<ClientBirthdayPartyData>,
) -> TalkReadReply {
    let refused = |reason: String| {
        warn!("[server] {API} refused: {reason}");
        TalkReadReply {
            success: false,
            refusal: Some(format!("{API}: {reason}")),
            obtained_resources: Vec::new(),
            updated: Default::default(),
        }
    };
    let answer = super::with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let (resources, changed) = model.talk_read(mysekai_character_talk_id)?;
        model.respond(ResponseKind::CharacterTalkRead, false, &changed);
        let response = model.responses.last().expect("the response just recorded");
        Ok((
            resources,
            response.inventory.clone(),
            response.delivery.clone(),
        ))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()));
    match answer {
        Ok((obtained_resources, updated, delivery)) => {
            birthday.apply(delivery);
            inventory.apply(updated.clone());
            TalkReadReply {
                success: true,
                refusal: None,
                obtained_resources,
                updated,
            }
        }
        Err(reason) => refused(reason),
    }
}

/// The endpoint.
pub(crate) fn install(app: &mut App) {
    let endpoint = app.world_mut().register_system(handle);
    app.insert_resource(TalkReadEndpoint(endpoint));
}
