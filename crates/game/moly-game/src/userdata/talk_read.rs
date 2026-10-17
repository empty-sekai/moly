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
//! The request is one `Call::CharacterTalkRead` on the server link; the
//! answer's tables reach the copies with the arrived sections.

// The talk owners read these.
#![allow(dead_code)]

pub(crate) use moly_protocol::rows::{UserMysekaiCharacterTalk, UserResource};
