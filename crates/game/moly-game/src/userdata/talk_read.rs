//! The talk read report as the client sends it:
//! `PutUserMysekaiCharacterTalkReadApi`
//! (`PUT user/{userId}/mysekai/character-talk/read/{mysekaiCharacterTalkId}`,
//! an empty body). The reply (`UserMysekaiCharacterTalkReadResponse`) carries
//! `obtainedResources` (`UserResource` rows) and `updatedResources`
//! (`SuiteUser`), which the client merges before it reads its copies
//! (`NPCTalkFinishReadService.OnFinishAPI`, `UserDataManager.UpdateAll`).
//!
//! When to send it is the caller's: the client skips it in the tutorial,
//! when the player is not the room owner, and when
//! `NPCAvatarModel.EnableSendReadAPI` is off; it sends it after the talk in
//! the general, playing-fixture, some-character and tutorial-default paths
//! and before the talk in the set-talk path; birthday-party and forced
//! tutorial talks send none.
//!
//! [`put_talk_read`] runs the endpoint the server model installs; when it
//! returns, the reply's tables are already in
//! [`ClientMysekaiInventory`](super::inventory::ClientMysekaiInventory)
//! (its `userMysekaiCharacterTalks` read view). Without an endpoint the
//! request is refused by name.

// The talk owners read these.
#![allow(dead_code)]

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

use super::birthday_party::ClientBirthdayPartyData;
use super::inventory::{ClientMysekaiInventory, SuiteUserSections};
use crate::server::{with_model, ResponseKind};

pub(crate) use moly_protocol::rows::{UserMysekaiCharacterTalk, UserResource};

pub(crate) use moly_protocol::api::TalkReadReply;

const API: &str = "PutUserMysekaiCharacterTalkReadApi";

/// The server model's endpoint for the talk read report.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct TalkReadEndpoint(pub(crate) SystemId<In<i32>, TalkReadReply>);

/// `PutUserMysekaiCharacterTalkReadApi`: report a talk as read and read the
/// reply.
pub(crate) fn put_talk_read(world: &mut World, mysekai_character_talk_id: i32) -> TalkReadReply {
    let refused = |reason: String| {
        warn!("[server] PutUserMysekaiCharacterTalkReadApi refused: {reason}");
        TalkReadReply {
            success: false,
            refusal: Some(format!("PutUserMysekaiCharacterTalkReadApi: {reason}")),
            obtained_resources: Vec::new(),
            updated: SuiteUserSections::default(),
        }
    };
    let Some(endpoint) = world.get_resource::<TalkReadEndpoint>().copied() else {
        return refused("no server endpoint is installed".into());
    };
    world
        .run_system_with(endpoint.0, mysekai_character_talk_id)
        .unwrap_or_else(|error| refused(error.to_string()))
}

/// The endpoint the server plugin installs for the report: the reply's
/// `updatedResources` are merged into the client's copies before it returns.
pub(crate) fn handle(
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
    let answer = with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let (resources, changed) = model.talk_read(mysekai_character_talk_id)?;
        model.respond(ResponseKind::CharacterTalkRead, false, &changed);
        let response = model.responses.last().expect("the response just recorded");
        Ok((
            resources,
            response.sections.inventory.clone(),
            response.sections.delivery.clone(),
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
