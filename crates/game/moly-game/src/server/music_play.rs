//! The music player's two requests and the records the user owns.
//!
//! - `userMysekaiMusicRecords` (`UserMysekaiMusicRecord` {mysekaiMusicRecordId,
//!   obtainedAt}): the records the user owns. The record list of the BGM
//!   select screen builds a model for every master record and keeps the
//!   possessed ones (`MysekaiBGMSelectUtility.CreateMusicRecordModelList`,
//!   `MysekaiMusicRecordModel.SetupUserData`), so this is the list the screen
//!   offers. It is user data: the server document holds it and the join
//!   carries it to the client's copy
//!   ([`super::client::music_play::ClientMusicRecords`]).
//! - `PutUserMysekaiMusicPlaySetApi` (`UserMysekaiMusicPlaySetRequest`
//!   {mysekaiSiteId, mysekaiMusicRecordId, musicVocalId (nullable),
//!   isInstrumental, mysekaiFixtureId, textureId}) and
//!   `PutUserMysekaiMusicPlayEjectApi` (`UserMysekaiMusicPlayEjectRequest`
//!   {mysekaiSiteId, mysekaiFixtureId, textureId}). The screen sends one of
//!   them when it exits (`ScreenLayerMysekaiBGMSelect.OnWillExitAsync` ->
//!   `BGMSelectPresenter.ExecuteMysekaiMusicPlaySetApiIfNeeded`): the set when
//!   a record is chosen and the site's setting differs from it, the eject
//!   when the choice was cleared and the site has a setting. Both reply with
//!   `SuiteUser`; the client merges it (`UserDataManager.UpdateAll`).
//!
//! **Named default**: no owned record (the mock's user owns none; the
//! records the tutorial and the game grant are server-side and not
//! modelled). The panel grants records by editing the section; natively the
//! instrument `MOLY_MUSIC_MOCK_OWNED_RECORDS` (comma-separated record ids,
//! obtained at the server clock) seeds them.
//!
//! **Named policy** `policies.musicPlayReply` (the server's rules are not on
//! disk): `success` (the product default) applies a set or an eject to
//! `userMysekaiMusicPlayFixtureSettings` and replies with that section and
//! the pending ones; `failure` refuses every request (the client shows the
//! API error, and the site's setting stays). Under `success` the rules are:
//! - a set replaces the site's row with the request's record, vocal and
//!   `isInstrumental`; a null `musicVocalId` is stored as 0 (the row's field
//!   is an integer);
//! - a set is refused when the record is not a master record (when the
//!   master is present) or not an owned record;
//! - an eject removes the site's row; an eject for a site without a row
//!   succeeds and changes nothing;
//! - `mysekaiFixtureId` and `textureId` are logged, not checked: the site's
//!   layout is not read (whether a music-play fixture stands there is not
//!   modelled).

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::client::music::MusicPlaySetting;
use super::client::music_play::{
    MusicPlayEjectRequest, MusicPlayPost, MusicPlayReply, MusicPlaySetRequest, OwnedMusicRecord,
};
use super::document::{int32, int64, object, only};
use super::{ResponseKind, ServerModel};

pub(crate) const RECORDS_SECTION: &str = "userMysekaiMusicRecords";
pub(crate) const POLICY_KEY: &str = "musicPlayReply";
pub(crate) const REPLY_SUCCESS: &str = "success";
pub(crate) const REPLY_FAILURE: &str = "failure";
/// The native instrument of the owned records.
pub(crate) const OWNED_INSTRUMENT: &str = "MOLY_MUSIC_MOCK_OWNED_RECORDS";

// ---------------------------------------------------------------------------
// The owned records
// ---------------------------------------------------------------------------

pub(crate) fn parse_owned_rows(value: &Value) -> Result<Vec<OwnedMusicRecord>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{RECORDS_SECTION} is not an array"))?;
    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{RECORDS_SECTION}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiMusicRecordId", "obtainedAt"], &at)?;
            let record = OwnedMusicRecord {
                mysekai_music_record_id: int32(row, "mysekaiMusicRecordId", &at)?,
                obtained_at: int64(row, "obtainedAt", &at)?,
            };
            if !seen.insert(record.mysekai_music_record_id) {
                return Err(format!(
                    "{at} repeats record {}",
                    record.mysekai_music_record_id
                ));
            }
            Ok(record)
        })
        .collect()
}

/// The section from a document (no row when absent: the named default).
pub(crate) fn parse(doc: &Map<String, Value>) -> Result<Vec<OwnedMusicRecord>, String> {
    doc.get(RECORDS_SECTION)
        .map_or(Ok(Vec::new()), parse_owned_rows)
}

/// A `server.edit` of the section (next response); `None` for another path.
pub(crate) fn edit_records(
    rows: &mut Vec<OwnedMusicRecord>,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [RECORDS_SECTION] => Some(parse_owned_rows(value).map(|parsed| *rows = parsed)),
        _ => None,
    }
}

/// The owned record ids against the master, when it is present.
pub(crate) fn check_owned(
    rows: &[OwnedMusicRecord],
    records: Option<&[i32]>,
) -> Result<(), String> {
    let Some(records) = records else {
        return Ok(());
    };
    for (i, row) in rows.iter().enumerate() {
        if !records.contains(&row.mysekai_music_record_id) {
            return Err(format!(
                "{RECORDS_SECTION}[{i}].mysekaiMusicRecordId = {} is not a master music record",
                row.mysekai_music_record_id
            ));
        }
    }
    Ok(())
}

/// The native instrument (`id,id,...`) as rows obtained at `now`; a
/// malformed or repeated id stops loudly.
pub(crate) fn parse_instrument(raw: &str, now: i64) -> Vec<OwnedMusicRecord> {
    let mut rows: Vec<OwnedMusicRecord> = Vec::new();
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let Ok(id) = entry.parse::<i32>() else {
            panic!("{OWNED_INSTRUMENT} entry {entry:?} is not an integer record id");
        };
        if rows.iter().any(|row| row.mysekai_music_record_id == id) {
            panic!("{OWNED_INSTRUMENT} names record {id} twice");
        }
        rows.push(OwnedMusicRecord {
            mysekai_music_record_id: id,
            obtained_at: now,
        });
    }
    rows
}

/// The native overlay of the owned-record instrument onto the document.
pub(crate) fn native_overlay(doc: &mut super::document::ServerDocument) {
    let Some(raw) = super::instrument_env(OWNED_INSTRUMENT) else {
        return;
    };
    let now = super::clock::server_now_ms(doc.clock);
    doc.music_records = parse_instrument(&raw, now);
    info!(
        "[server] native overlay: {OWNED_INSTRUMENT} -> {RECORDS_SECTION} ({} records, obtainedAt {now})",
        doc.music_records.len()
    );
}

// ---------------------------------------------------------------------------
// The reply policy
// ---------------------------------------------------------------------------

/// `policies.musicPlayReply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MusicPlayReplyPolicy {
    #[default]
    Success,
    Failure,
}

pub(crate) fn parse_policy(value: &Value) -> Result<MusicPlayReplyPolicy, String> {
    match value.as_str() {
        Some(REPLY_SUCCESS) => Ok(MusicPlayReplyPolicy::Success),
        Some(REPLY_FAILURE) => Ok(MusicPlayReplyPolicy::Failure),
        _ => Err(format!(
            "policies.{POLICY_KEY} is neither \"{REPLY_SUCCESS}\" nor \"{REPLY_FAILURE}\""
        )),
    }
}

/// The policy from a document's `policies` (the default when absent).
pub(crate) fn parse_reply_policy(
    policies: &Map<String, Value>,
) -> Result<MusicPlayReplyPolicy, String> {
    policies
        .get(POLICY_KEY)
        .map_or(Ok(MusicPlayReplyPolicy::default()), parse_policy)
}

pub(crate) fn write_policy(policy: MusicPlayReplyPolicy, policies: &mut Map<String, Value>) {
    policies.insert(
        POLICY_KEY.into(),
        json!(match policy {
            MusicPlayReplyPolicy::Success => REPLY_SUCCESS,
            MusicPlayReplyPolicy::Failure => REPLY_FAILURE,
        }),
    );
}

/// A `server.edit` of the policy (live); `None` for another path.
pub(crate) fn edit_policy(
    policy: &mut MusicPlayReplyPolicy,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        ["policies", POLICY_KEY] => Some(parse_policy(value).map(|parsed| *policy = parsed)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The two endpoints
// ---------------------------------------------------------------------------

fn kind(request: &MusicPlayPost) -> ResponseKind {
    match request {
        MusicPlayPost::Set(_) => ResponseKind::MusicPlaySet,
        MusicPlayPost::Eject(_) => ResponseKind::MusicPlayEject,
    }
}

impl ServerModel {
    /// The set rules of the success policy: the row the request stores, or
    /// the refusal.
    fn music_play_set_row(
        &self,
        request: &MusicPlaySetRequest,
    ) -> Result<MusicPlaySetting, String> {
        let record = request.mysekai_music_record_id;
        if let Some(records) = self.masters.music_records.as_deref() {
            if !records.contains(&record) {
                return Err(format!("record {record} is not a master music record"));
            }
        }
        if !self
            .doc
            .music_records
            .iter()
            .any(|row| row.mysekai_music_record_id == record)
        {
            return Err(format!(
                "record {record} is not an owned record ({RECORDS_SECTION})"
            ));
        }
        Ok(MusicPlaySetting {
            mysekai_site_id: request.mysekai_site_id,
            mysekai_music_record_id: record,
            music_vocal_id: request.music_vocal_id.unwrap_or(0),
            is_instrumental: request.is_instrumental,
        })
    }

    pub(super) fn music_play(&mut self, request: MusicPlayPost) -> MusicPlayReply {
        let slot = match request {
            MusicPlayPost::Set(_) => 0,
            MusicPlayPost::Eject(_) => 1,
        };
        self.music_play_replies[slot] += 1;
        let count = self.music_play_replies[slot];
        let name = kind(&request).name();
        let (fixture, texture) = match request {
            MusicPlayPost::Set(set) => (set.mysekai_fixture_id, set.texture_id),
            MusicPlayPost::Eject(eject) => (eject.mysekai_fixture_id, eject.texture_id),
        };
        if self.doc.music_play_reply == MusicPlayReplyPolicy::Failure {
            info!(
                "[server] {name} for site {} (fixture {fixture}, texture {texture}): refused (policies.{POLICY_KEY} = {REPLY_FAILURE}; request {count})",
                request.site_id()
            );
            return MusicPlayReply { success: false };
        }
        let site = request.site_id();
        let before = self
            .doc
            .music_settings
            .iter()
            .find(|row| row.mysekai_site_id == site)
            .copied();
        match request {
            MusicPlayPost::Set(set) => {
                let row = match self.music_play_set_row(&set) {
                    Ok(row) => row,
                    Err(reason) => {
                        warn!(
                            "[server] {name} for site {site} refused: {reason} (request {count})"
                        );
                        return MusicPlayReply { success: false };
                    }
                };
                let rows = &mut self.doc.music_settings;
                rows.retain(|known| known.mysekai_site_id != site);
                rows.push(row);
                rows.sort_by_key(|known| known.mysekai_site_id);
                info!(
                    "[server] {name} for site {site} (fixture {fixture}, texture {texture}; the site layout is not checked): {before:?} -> {row:?} (request {count})"
                );
            }
            MusicPlayPost::Eject(MusicPlayEjectRequest { .. }) => {
                self.doc
                    .music_settings
                    .retain(|known| known.mysekai_site_id != site);
                info!(
                    "[server] {name} for site {site} (fixture {fixture}, texture {texture}; the site layout is not checked): {before:?} -> no row (request {count})"
                );
            }
        }
        self.commit();
        self.respond(kind(&request), false, &[super::music::SECTION]);
        MusicPlayReply { success: true }
    }
}

/// The endpoint the server plugin installs for the client's requests
/// ([`super::client::music_play::post`]). Refused while no server has joined
/// the client.
pub(super) fn handle(In(request): In<MusicPlayPost>) -> MusicPlayReply {
    super::with_model(|model| {
        if !model.joined {
            warn!(
                "[server] {} refused: the server has not joined the client yet",
                request.api_name()
            );
            return MusicPlayReply { success: false };
        }
        model.music_play(request)
    })
    .unwrap_or_else(|| {
        warn!(
            "[server] {} refused: the server model is not installed",
            request.api_name()
        );
        MusicPlayReply { success: false }
    })
}

// ---------------------------------------------------------------------------
// The panel's schema
// ---------------------------------------------------------------------------

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": RECORDS_SECTION,
        "title": "Owned music records",
        "delivery": "next-response",
        "fields": [{
            "path": RECORDS_SECTION,
            "type": "rows",
            "row": {"mysekaiMusicRecordId": "int", "obtainedAt": "int"},
            "note": "the records the user owns; the BGM select screen lists only these (named default: none)",
        }],
    }, {
        "key": "musicPlay",
        "title": "Music player set and eject replies",
        "delivery": "live",
        "fields": [{
            "path": format!("policies.{POLICY_KEY}"),
            "type": "enum",
            "values": [
                {"value": REPLY_SUCCESS, "label": "a set stores the site's record, an eject removes it; the reply carries the settings and the pending sections"},
                {"value": REPLY_FAILURE, "label": "every request is refused (the client shows the API error)"},
            ],
            "note": "the fixture and texture ids of a request are not checked against the site layout",
        }],
    }])
}

pub(crate) fn schema_policies() -> Value {
    json!([{
        "name": "music play reply",
        "rule": "success: a set replaces the site's row (a null vocal is stored as 0) unless the record is not a master or not an owned record; an eject removes the site's row; failure refuses both",
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(record: i32, vocal: Option<i32>) -> MusicPlayPost {
        MusicPlayPost::Set(MusicPlaySetRequest {
            mysekai_site_id: 5,
            mysekai_music_record_id: record,
            music_vocal_id: vocal,
            is_instrumental: false,
            mysekai_fixture_id: 70,
            texture_id: 1,
        })
    }

    #[test]
    fn set_and_eject_change_the_site_row_and_reply_with_it() {
        let mut model = super::super::tests::model();
        model.joined = true;
        model.masters.music_records = Some(vec![1, 12, 10001]);
        model.doc.music_records = parse_instrument("12, 10001", 7);
        assert_eq!(model.doc.music_records[1].obtained_at, 7);
        // Not owned, then not a master record: refused, no response.
        assert!(!model.music_play(set(1, Some(1))).success);
        assert!(!model.music_play(set(99, Some(1))).success);
        assert!(model.responses.is_empty());
        assert!(model.music_play(set(12, Some(3))).success);
        assert!(model.music_play(set(10001, None)).success);
        assert_eq!(
            model.doc.music_settings,
            vec![MusicPlaySetting {
                mysekai_site_id: 5,
                mysekai_music_record_id: 10001,
                music_vocal_id: 0,
                is_instrumental: false,
            }]
        );
        let reply = model.responses.last().unwrap();
        assert_eq!(reply.kind, ResponseKind::MusicPlaySet);
        assert_eq!(reply.music.as_ref().unwrap().len(), 1);
        let eject = MusicPlayPost::Eject(MusicPlayEjectRequest {
            mysekai_site_id: 5,
            mysekai_fixture_id: 70,
            texture_id: 1,
        });
        assert!(model.music_play(eject).success);
        assert!(model.doc.music_settings.is_empty());
        assert_eq!(
            model
                .responses
                .last()
                .unwrap()
                .music
                .as_ref()
                .unwrap()
                .len(),
            0
        );
        model.doc.music_play_reply = MusicPlayReplyPolicy::Failure;
        assert!(!model.music_play(set(12, Some(3))).success);
        assert_eq!(model.music_play_replies, [5, 1]);
    }

    #[test]
    fn the_sections_parse_strictly_and_round_trip() {
        let rows = parse_instrument("12,3", 100);
        let value = super::super::client::music_play::owned_rows_value(&rows);
        assert_eq!(parse_owned_rows(&value).unwrap(), rows);
        assert!(parse(&Map::new()).unwrap().is_empty());
        assert!(
            parse_owned_rows(&json!([{"mysekaiMusicRecordId": 1, "obtainedAt": 0},
            {"mysekaiMusicRecordId": 1, "obtainedAt": 5}]))
            .unwrap_err()
            .contains("repeats record 1")
        );
        assert!(check_owned(&rows, Some(&[12])).unwrap_err().contains("= 3"));
        let mut policies = Map::new();
        write_policy(MusicPlayReplyPolicy::Failure, &mut policies);
        assert_eq!(
            parse_reply_policy(&policies).unwrap(),
            MusicPlayReplyPolicy::Failure
        );
        assert_eq!(
            parse_reply_policy(&Map::new()).unwrap(),
            MusicPlayReplyPolicy::Success
        );
        assert!(parse_policy(&json!("maybe")).is_err());
    }
}
