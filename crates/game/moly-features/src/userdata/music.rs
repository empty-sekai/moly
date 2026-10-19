//! The client's copy of the music record settings
//! (`UserDataManager.UserMysekaiMusicPlayFixtureSettings`,
//! `UserMysekaiMusicPlayFixtureSetting`: one row per site with the record,
//! the vocal version and whether it plays instrumental). The server model's
//! responses set it; the BGM reads the setting of the site it plays on.

pub use moly_protocol::json::music_settings_value as rows_value;
pub use moly_protocol::rows::MusicPlaySetting;

pub use crate::audio_inputs::ClientMusicPlaySettings;

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
