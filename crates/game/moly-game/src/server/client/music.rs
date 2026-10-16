//! The client's copy of the music record settings
//! (`UserDataManager.UserMysekaiMusicPlayFixtureSettings`,
//! `UserMysekaiMusicPlayFixtureSetting`: one row per site with the record,
//! the vocal version and whether it plays instrumental). The server model's
//! responses set it; the BGM reads the setting of the site it plays on.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde_json::{json, Value};

/// `UserMysekaiMusicPlayFixtureSetting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MusicPlaySetting {
    pub(crate) mysekai_site_id: i32,
    pub(crate) mysekai_music_record_id: i32,
    pub(crate) music_vocal_id: i32,
    pub(crate) is_instrumental: bool,
}

/// The rows as the response key's JSON array.
pub(crate) fn rows_value(rows: &[MusicPlaySetting]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiSiteId": row.mysekai_site_id,
                    "mysekaiMusicRecordId": row.mysekai_music_record_id,
                    "musicVocalId": row.music_vocal_id,
                    "isInstrumental": row.is_instrumental,
                })
            })
            .collect(),
    )
}

/// The client's copy, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ClientMusicPlaySettings {
    per_site: BTreeMap<u32, MusicPlaySetting>,
    /// Responses that carried the section.
    pub(crate) revision: u64,
}

#[allow(dead_code)] // Read by the BGM owner's seam.
impl ClientMusicPlaySettings {
    /// The setting of a site: (record id, vocal id).
    pub(crate) fn setting(&self, site_id: u32) -> Option<(i64, i64)> {
        self.per_site.get(&site_id).map(|row| {
            (
                i64::from(row.mysekai_music_record_id),
                i64::from(row.music_vocal_id),
            )
        })
    }

    /// The whole row of a site.
    pub(crate) fn row(&self, site_id: u32) -> Option<&MusicPlaySetting> {
        self.per_site.get(&site_id)
    }

    /// A response's rows replace the copy (a row with a negative site id
    /// names no site and is dropped).
    pub(crate) fn apply(&mut self, rows: Vec<MusicPlaySetting>) {
        self.per_site = rows
            .into_iter()
            .filter_map(|row| {
                u32::try_from(row.mysekai_site_id)
                    .ok()
                    .map(|site| (site, row))
            })
            .collect();
        self.revision += 1;
    }

    pub(crate) fn view(&self) -> Value {
        rows_value(&self.per_site.values().copied().collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_answers_per_site() {
        let mut client = ClientMusicPlaySettings::default();
        assert_eq!(client.setting(5), None);
        client.apply(vec![MusicPlaySetting {
            mysekai_site_id: 5,
            mysekai_music_record_id: 12,
            music_vocal_id: 3,
            is_instrumental: false,
        }]);
        assert_eq!(client.revision, 1);
        assert_eq!(client.setting(5), Some((12, 3)));
        assert_eq!(client.setting(6), None);
        assert_eq!(client.view()[0]["mysekaiMusicRecordId"], 12);
    }
}
