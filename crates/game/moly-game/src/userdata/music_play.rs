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
use serde_json::Value;

pub(crate) use moly_protocol::json::music_records_value as owned_rows_value;
pub(crate) use moly_protocol::rows::OwnedMusicRecord;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_answers_possession() {
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
