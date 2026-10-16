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

use super::inventory::SuiteUserSections;

/// `UserMysekaiCharacterTalk`: whether a talk has been read (the character
/// archive reads these rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UserMysekaiCharacterTalk {
    pub(crate) mysekai_character_talk_id: i32,
    pub(crate) is_read: bool,
}

/// `UserResource`, the fields a reply's `obtainedResources` row carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserResource {
    pub(crate) resource_type: String,
    pub(crate) resource_id: i32,
    pub(crate) resource_level: i32,
    pub(crate) quantity: i32,
}

/// `UserMysekaiCharacterTalkReadResponse`, or a refusal the client shows as
/// the API error.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TalkReadReply {
    pub(crate) success: bool,
    /// Why the server refused (`None` on success).
    pub(crate) refusal: Option<String>,
    /// `obtainedResources`.
    pub(crate) obtained_resources: Vec<UserResource>,
    /// `updatedResources` (already merged into the client copy).
    pub(crate) updated: SuiteUserSections,
}

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
