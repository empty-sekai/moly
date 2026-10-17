//! The music player's requests and the owned records, as the client holds
//! them.
//!
//! - `UserDataManager.UserMysekaiMusicRecords` (`UserMysekaiMusicRecord`:
//!   the record id and when it was obtained): [`ClientMusicRecords`], set by
//!   the server model's responses only. `MysekaiMusicRecordModel.SetupUserData`
//!   reads it for `InPossession` and `ObtainedAt`, and the record list keeps
//!   only the possessed records.
//! - The two requests `MysekaiBGMSelectUtility` sends for a music-play
//!   fixture: `PutUserMysekaiMusicPlaySetApi` (`UserMysekaiMusicPlaySetRequest`)
//!   and `PutUserMysekaiMusicPlayEjectApi` (`UserMysekaiMusicPlayEjectRequest`).
//!   Both reply with the updated user data (`SuiteUser`), which the executor
//!   merges into the client copies before the caller reads them.
//!
//! A request runs through [`MusicPlayEndpoint`], a one-shot system the
//! server model installs; [`post`] runs it and returns the reply. Without an
//! endpoint the request is refused by name.

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use serde_json::{json, Value};

pub(crate) use moly_protocol::rows::OwnedMusicRecord;

/// The rows as the response key's JSON array.
pub(crate) fn owned_rows_value(rows: &[OwnedMusicRecord]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiMusicRecordId": row.mysekai_music_record_id,
                    "obtainedAt": row.obtained_at,
                })
            })
            .collect(),
    )
}

/// The client's copy of the owned records, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientMusicRecords {
    rows: Vec<OwnedMusicRecord>,
    /// Responses that carried the section.
    pub(crate) revision: u64,
}

#[allow(dead_code)] // Read by the record list's seam.
impl ClientMusicRecords {
    /// `SetupUserData`: the first row of the record, whose presence is
    /// `InPossession` and whose `obtainedAt` is `ObtainedAt` (0 when absent).
    pub(crate) fn possession(&self, record_id: i32) -> Option<i64> {
        self.rows
            .iter()
            .find(|row| row.mysekai_music_record_id == record_id)
            .map(|row| row.obtained_at)
    }

    /// A response's rows replace the copy.
    pub(crate) fn apply(&mut self, rows: Vec<OwnedMusicRecord>) {
        self.rows = rows;
        self.revision += 1;
    }

    pub(crate) fn view(&self) -> Value {
        owned_rows_value(&self.rows)
    }
}

pub(crate) use moly_protocol::api::{
    MusicPlaySetRequest, MusicPlayEjectRequest, MusicPlayPost, MusicPlayReply,
};

/// The server model's endpoint for the music player's requests.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct MusicPlayEndpoint(pub(crate) SystemId<In<MusicPlayPost>, MusicPlayReply>);

/// Send a request and read its reply.
#[allow(dead_code)] // Called by the record list's seam.
pub(crate) fn post(world: &mut World, request: MusicPlayPost) -> MusicPlayReply {
    let Some(endpoint) = world.get_resource::<MusicPlayEndpoint>().copied() else {
        warn!(
            "[server] {} refused: no server endpoint is installed",
            request.api_name()
        );
        return MusicPlayReply { success: false };
    };
    world
        .run_system_with(endpoint.0, request)
        .unwrap_or_else(|error| {
            warn!("[server] {} refused: {error}", request.api_name());
            MusicPlayReply { success: false }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(In(request): In<MusicPlayPost>) -> MusicPlayReply {
        MusicPlayReply {
            success: matches!(request, MusicPlayPost::Set(_)),
        }
    }

    #[test]
    fn a_request_runs_through_the_endpoint_and_the_copy_answers_possession() {
        let set = MusicPlayPost::Set(MusicPlaySetRequest {
            mysekai_site_id: 5,
            mysekai_music_record_id: 12,
            music_vocal_id: Some(3),
            is_instrumental: false,
            mysekai_fixture_id: 70,
            texture_id: 1,
        });
        let eject = MusicPlayPost::Eject(MusicPlayEjectRequest {
            mysekai_site_id: 5,
            mysekai_fixture_id: 70,
            texture_id: 1,
        });
        let mut world = World::new();
        assert!(!post(&mut world, set).success);
        let endpoint = world.register_system(echo);
        world.insert_resource(MusicPlayEndpoint(endpoint));
        assert!(post(&mut world, set).success);
        assert!(!post(&mut world, eject).success);
        let mut copy = ClientMusicRecords::default();
        assert_eq!(copy.possession(12), None);
        copy.apply(vec![OwnedMusicRecord {
            mysekai_music_record_id: 12,
            obtained_at: 1_000,
        }]);
        assert_eq!(copy.possession(12), Some(1_000));
        assert_eq!(copy.revision, 1);
        assert_eq!(copy.view()[0]["obtainedAt"], 1_000);
    }
}
