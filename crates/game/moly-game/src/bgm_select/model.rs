//! The screen's data: the master tables it reads, the record models
//! (`MysekaiMusicRecordModel`) and the presenter's selection state
//! (`BGMSelectModel` as `BGMSelectPresenter` drives it).

use std::collections::{HashMap, HashSet};

use moly_assets::json::master::{self, MasterData, MasterError, MasterTable};
use serde_json::Value;

use crate::userdata::client::music::MusicPlaySetting;
use crate::userdata::client::music_play::{
    ClientMusicRecords, MusicPlayEjectRequest, MusicPlayPost, MusicPlaySetRequest,
};

// ---------------------------------------------------------------------------
// Master tables
// ---------------------------------------------------------------------------

/// `mysekaiMusicRecords`: track type and external id, in master order.
#[derive(Clone, Debug)]
pub(crate) struct RecordRow {
    pub(crate) id: i32,
    /// `MysekaiMusicTrackType.music_sound_track` (1); every other value is
    /// the music branch (0).
    pub(crate) soundtrack: bool,
    pub(crate) external_id: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct MusicRow {
    pub(crate) seq: i32,
    pub(crate) title: String,
    pub(crate) creator_artist_id: i32,
    pub(crate) published_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct VocalRow {
    pub(crate) id: i32,
    /// `musicVocalType`; `instrumental` is `MusicVocalType` 4.
    pub(crate) vocal_type: String,
    /// (`characterType`, `characterId`, `seq`).
    pub(crate) characters: Vec<(String, i32, i32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct SoundTrackRow {
    pub(crate) seq: i32,
    pub(crate) title: String,
    pub(crate) creator: String,
}

fn text_or_empty(row: &Value, field: &str) -> String {
    // A null or absent string formats as empty (`String.Format` of null).
    row.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

// Every table here is read row by row: a row the reader cannot read is
// dropped and named once by the master layer. The rows are independent
// entries, and a record whose music, soundtrack or vocal row is absent is
// refused by name when the record list is built (as the source logs the
// missing master row and leaves the model without a resource).

fn parse_records(text: &str) -> Result<Vec<RecordRow>, String> {
    master::read_rows(text, |row| {
        Ok(Some(RecordRow {
            id: master::int32(row, "id")?,
            soundtrack: master::text(row, "mysekaiMusicTrackType")? == "music_sound_track",
            external_id: master::int32(row, "externalId")?,
        }))
    })
}

fn parse_musics(text: &str) -> Result<HashMap<i32, MusicRow>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            MusicRow {
                seq: master::int32(row, "seq")?,
                title: text_or_empty(row, "title"),
                creator_artist_id: master::int32(row, "creatorArtistId").unwrap_or(0),
                published_at: master::int(row, "publishedAt")?,
            },
        )))
    })?;
    Ok(rows.into_iter().collect())
}

/// `musicVocals` by music id, in master order.
fn parse_vocals(text: &str) -> Result<HashMap<i32, Vec<VocalRow>>, String> {
    let rows = master::read_rows(text, |row| {
        let mut characters = Vec::new();
        for character in row["characters"].as_array().into_iter().flatten() {
            characters.push((
                master::text(character, "characterType")?.to_owned(),
                master::int32(character, "characterId")?,
                master::int32(character, "seq")?,
            ));
        }
        Ok(Some((
            master::int32(row, "musicId")?,
            VocalRow {
                id: master::int32(row, "id")?,
                vocal_type: master::text(row, "musicVocalType")?.to_owned(),
                characters,
            },
        )))
    })?;
    let mut by_music: HashMap<i32, Vec<VocalRow>> = HashMap::new();
    for (music, vocal) in rows {
        by_music.entry(music).or_default().push(vocal);
    }
    Ok(by_music)
}

fn parse_sound_tracks(text: &str) -> Result<HashMap<i32, SoundTrackRow>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            SoundTrackRow {
                seq: master::int32(row, "seq")?,
                title: text_or_empty(row, "title"),
                creator: text_or_empty(row, "creator"),
            },
        )))
    })?;
    Ok(rows.into_iter().collect())
}

/// `musicTags`: the music ids of each tag.
fn parse_tags(text: &str) -> Result<HashMap<String, HashSet<i32>>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::text(row, "musicTag")?.to_owned(),
            master::int32(row, "musicId")?,
        )))
    })?;
    let mut by_tag: HashMap<String, HashSet<i32>> = HashMap::new();
    for (tag, music) in rows {
        by_tag.entry(tag).or_default().insert(music);
    }
    Ok(by_tag)
}

/// `id` -> `name` (`musicArtists`, `outsideCharacters`).
fn parse_names(text: &str) -> Result<HashMap<i32, String>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((master::int32(row, "id")?, text_or_empty(row, "name"))))
    })?;
    Ok(rows.into_iter().collect())
}

/// `gameCharacters`: `MasterGameCharacter.FullName` = `"{0}{1}"` of
/// `firstName` and `givenName`.
fn parse_game_characters(text: &str) -> Result<HashMap<i32, String>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            format!(
                "{}{}",
                text_or_empty(row, "firstName"),
                text_or_empty(row, "givenName")
            ),
        )))
    })?;
    Ok(rows.into_iter().collect())
}

pub(crate) const RECORDS: MasterTable<Vec<RecordRow>> = MasterTable {
    table: "mysekaiMusicRecords",
    name: "mysekaiMusicRecords (the BGM select records)",
    parse: parse_records,
};
pub(crate) const MUSICS: MasterTable<HashMap<i32, MusicRow>> = MasterTable {
    table: "musics",
    name: "musics (the BGM select records)",
    parse: parse_musics,
};
pub(crate) const VOCALS: MasterTable<HashMap<i32, Vec<VocalRow>>> = MasterTable {
    table: "musicVocals",
    name: "musicVocals (the BGM select records)",
    parse: parse_vocals,
};
pub(crate) const SOUND_TRACKS: MasterTable<HashMap<i32, SoundTrackRow>> = MasterTable {
    table: "musicSoundTracks",
    name: "musicSoundTracks (the BGM select records)",
    parse: parse_sound_tracks,
};
pub(crate) const TAGS: MasterTable<HashMap<String, HashSet<i32>>> = MasterTable {
    table: "musicTags",
    name: "musicTags (the BGM select tabs)",
    parse: parse_tags,
};
pub(crate) const ARTISTS: MasterTable<HashMap<i32, String>> = MasterTable {
    table: "musicArtists",
    name: "musicArtists (the BGM select artist names)",
    parse: parse_names,
};
pub(crate) const GAME_CHARACTERS: MasterTable<HashMap<i32, String>> = MasterTable {
    table: "gameCharacters",
    name: "gameCharacters (the BGM select vocal names)",
    parse: parse_game_characters,
};
pub(crate) const OUTSIDE_CHARACTERS: MasterTable<HashMap<i32, String>> = MasterTable {
    table: "outsideCharacters",
    name: "outsideCharacters (the BGM select vocal names)",
    parse: parse_names,
};

/// Requests every table the screen reads.
pub(crate) fn request(masters: &mut MasterData) {
    masters.request(&RECORDS);
    masters.request(&MUSICS);
    masters.request(&VOCALS);
    masters.request(&SOUND_TRACKS);
    masters.request(&TAGS);
    masters.request(&ARTISTS);
    masters.request(&GAME_CHARACTERS);
    masters.request(&OUTSIDE_CHARACTERS);
}

/// The tables, as they resolve.
#[derive(Default)]
pub(crate) struct Pending {
    records: Option<Result<Vec<RecordRow>, MasterError>>,
    musics: Option<Result<HashMap<i32, MusicRow>, MasterError>>,
    vocals: Option<Result<HashMap<i32, Vec<VocalRow>>, MasterError>>,
    sound_tracks: Option<Result<HashMap<i32, SoundTrackRow>, MasterError>>,
    tags: Option<Result<HashMap<String, HashSet<i32>>, MasterError>>,
    artists: Option<Result<HashMap<i32, String>, MasterError>>,
    game_characters: Option<Result<HashMap<i32, String>, MasterError>>,
    outside_characters: Option<Result<HashMap<i32, String>, MasterError>>,
}

/// The resolved tables. The artist and character names degrade to empty
/// texts when their table is absent (named once); the others are required.
pub(crate) struct BgmMasters {
    pub(crate) records: Vec<RecordRow>,
    pub(crate) musics: HashMap<i32, MusicRow>,
    pub(crate) vocals: HashMap<i32, Vec<VocalRow>>,
    pub(crate) sound_tracks: HashMap<i32, SoundTrackRow>,
    pub(crate) tags: HashMap<String, HashSet<i32>>,
    pub(crate) artists: HashMap<i32, String>,
    pub(crate) game_characters: HashMap<i32, String>,
    pub(crate) outside_characters: HashMap<i32, String>,
    /// The optional tables that did not resolve, for the log.
    pub(crate) degraded: Vec<String>,
}

impl Pending {
    /// Takes what resolved; `Some` once every table has.
    pub(crate) fn poll(&mut self, data: &mut MasterData) -> Option<Result<BgmMasters, String>> {
        fn take<T: 'static>(
            slot: &mut Option<Result<T, MasterError>>,
            data: &mut MasterData,
            table: &MasterTable<T>,
        ) {
            if slot.is_none() {
                *slot = data.take(table);
            }
        }
        take(&mut self.records, data, &RECORDS);
        take(&mut self.musics, data, &MUSICS);
        take(&mut self.vocals, data, &VOCALS);
        take(&mut self.sound_tracks, data, &SOUND_TRACKS);
        take(&mut self.tags, data, &TAGS);
        take(&mut self.artists, data, &ARTISTS);
        take(&mut self.game_characters, data, &GAME_CHARACTERS);
        take(&mut self.outside_characters, data, &OUTSIDE_CHARACTERS);
        let (
            records,
            musics,
            vocals,
            sound_tracks,
            tags,
            artists,
            game_characters,
            outside_characters,
        ) = match std::mem::take(self) {
            Pending {
                records: Some(records),
                musics: Some(musics),
                vocals: Some(vocals),
                sound_tracks: Some(sound_tracks),
                tags: Some(tags),
                artists: Some(artists),
                game_characters: Some(game_characters),
                outside_characters: Some(outside_characters),
            } => (
                records,
                musics,
                vocals,
                sound_tracks,
                tags,
                artists,
                game_characters,
                outside_characters,
            ),
            // Not every table has resolved: keep what has.
            incomplete => {
                *self = incomplete;
                return None;
            }
        };
        let required = |error: MasterError| error.to_string();
        let mut degraded = Vec::new();
        let mut optional = |slot: Result<HashMap<i32, String>, MasterError>| match slot {
            Ok(table) => table,
            Err(error) => {
                degraded.push(error.to_string());
                HashMap::new()
            }
        };
        let artists = optional(artists);
        let game_characters = optional(game_characters);
        let outside_characters = optional(outside_characters);
        Some((|| -> Result<BgmMasters, String> {
            Ok(BgmMasters {
                records: records.map_err(required)?,
                musics: musics.map_err(required)?,
                vocals: vocals.map_err(required)?,
                sound_tracks: sound_tracks.map_err(required)?,
                tags: tags.map_err(required)?,
                artists,
                game_characters,
                outside_characters,
                degraded,
            })
        })())
    }
}

// ---------------------------------------------------------------------------
// The record model
// ---------------------------------------------------------------------------

/// The two wordings `MusicUtility.GetVocalText` formats with.
pub(crate) struct VocalWordings {
    /// `MSG_MUSIC_VOCAL` (`{0}` = the joined character names).
    pub(crate) vocal: String,
    /// `WORD_INSTRUMENTLE_VERSION` (an instrumental vocal's text).
    pub(crate) instrumental: String,
}

/// `string.Format` with one argument, the only form these texts use.
fn format1(format: &str, arg: &str) -> String {
    format.replace("{0}", arg)
}

/// `MusicUtility.GetVocalText(vocal, addVo: true)`: an instrumental vocal
/// is `WORD_INSTRUMENTLE_VERSION`; any other is `GetVocalInfoStr`: the
/// characters by `seq`, each name followed by `、` (a game character's
/// `FullName`, an outside character's `name`; a character of another type
/// adds nothing), the last `、` cut, formatted into `MSG_MUSIC_VOCAL`.
pub(crate) fn vocal_text(vocal: &VocalRow, masters: &BgmMasters, words: &VocalWordings) -> String {
    if vocal.vocal_type == "instrumental" {
        return words.instrumental.clone();
    }
    let mut characters: Vec<&(String, i32, i32)> = vocal.characters.iter().collect();
    characters.sort_by_key(|(_, _, seq)| *seq);
    let mut names = String::new();
    for (kind, id, _) in characters {
        let name = match kind.as_str() {
            "game_character" => masters.game_characters.get(id),
            "outside_character" => masters.outside_characters.get(id),
            _ => None,
        };
        if let Some(name) = name {
            names.push_str(name);
            names.push('、');
        }
    }
    names.pop();
    format1(&words.vocal, &names)
}

/// `MysekaiMusicRecordModel`, the fields the screen reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordModel {
    /// `SelectedId`: the record id.
    pub(crate) record_id: i32,
    pub(crate) soundtrack: bool,
    /// The music id (music records), for the vocal list.
    pub(crate) music_id: Option<i32>,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) vocal_name: String,
    /// `MusicVocalId` (null for a soundtrack).
    pub(crate) vocal_id: Option<i32>,
    pub(crate) multiple_vocals: bool,
    pub(crate) seq: i32,
    /// `BGMResourceInfo` is set: the record can play.
    pub(crate) has_resource: bool,
    /// `IsTabFilterExist(ALL, model)`.
    pub(crate) in_all_tab: bool,
}

/// `CreateMusicRecordModelList` with the presenter's filter model (tab ALL,
/// no free word, sort Default): a model for every record row
/// (`new MysekaiMusicRecordModel(id)`: `SetupMasterData`, `SetupUserData`),
/// the possessed ones kept, ordered by track type, `Seq`, then id.
///
/// `vocal_choice` is `MysekaiRecordSettingDataList`'s vocal of a record
/// (`GetDefaultMusicVocalModel`: with several vocals, the vocal whose id is
/// the remembered one, else the first; with one, the first). `now_ms` is the
/// server clock for `IsPublished`.
///
/// Named assumptions (the music composite `MasterMusicAll` is assembled
/// outside the readable client): its artist is the `musicArtists` row of
/// the music's `creatorArtistId`, and its vocals are the music's
/// `musicVocals` rows in master order.
pub(crate) fn record_models(
    masters: &BgmMasters,
    owned: &ClientMusicRecords,
    vocal_choice: &HashMap<i32, i32>,
    words: &VocalWordings,
    now_ms: i64,
) -> (Vec<RecordModel>, Vec<String>) {
    let mut models = Vec::new();
    let mut refused = Vec::new();
    for row in &masters.records {
        // SetupUserData: InPossession.
        if owned.possession(row.id).is_none() {
            continue;
        }
        let all_tag = masters.tags.get("all");
        let model = if row.soundtrack {
            // SetupMusicSoundtrack: the soundtrack row named by externalId.
            match masters.sound_tracks.get(&row.external_id) {
                Some(track) => RecordModel {
                    record_id: row.id,
                    soundtrack: true,
                    music_id: None,
                    title: track.title.clone(),
                    artist: track.creator.clone(),
                    vocal_name: String::new(),
                    vocal_id: None,
                    multiple_vocals: false,
                    seq: track.seq,
                    has_resource: true,
                    in_all_tab: true,
                },
                None => {
                    refused.push(format!(
                        "record {}: soundtrack {} is not in musicSoundTracks (no resource)",
                        row.id, row.external_id
                    ));
                    RecordModel {
                        record_id: row.id,
                        soundtrack: true,
                        music_id: None,
                        title: String::new(),
                        artist: String::new(),
                        vocal_name: String::new(),
                        vocal_id: None,
                        multiple_vocals: false,
                        seq: 0,
                        has_resource: false,
                        in_all_tab: true,
                    }
                }
            }
        } else {
            let Some(music) = masters.musics.get(&row.external_id) else {
                // SetupMasterData logs an error; the model has no resource
                // and no music: IsTabFilterExist reads IsPublished off it.
                refused.push(format!(
                    "record {}: music {} is not in musics (no resource)",
                    row.id, row.external_id
                ));
                continue;
            };
            let vocals = masters
                .vocals
                .get(&row.external_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let multiple = vocals.len() > 1;
            let chosen = if multiple {
                vocal_choice
                    .get(&row.id)
                    .and_then(|id| vocals.iter().find(|vocal| vocal.id == *id))
                    .or_else(|| vocals.first())
            } else {
                vocals.first()
            };
            let Some(vocal) = chosen else {
                // First() of an empty vocal list throws in the model's
                // construction; the list logs the exception and skips it.
                refused.push(format!(
                    "record {}: music {} has no vocal (the model's construction throws)",
                    row.id, row.external_id
                ));
                continue;
            };
            RecordModel {
                record_id: row.id,
                soundtrack: false,
                music_id: Some(row.external_id),
                title: music.title.clone(),
                artist: masters
                    .artists
                    .get(&music.creator_artist_id)
                    .cloned()
                    .unwrap_or_default(),
                vocal_name: vocal_text(vocal, masters, words),
                vocal_id: Some(vocal.id),
                multiple_vocals: multiple,
                seq: music.seq,
                has_resource: true,
                // IsTabFilterExist(ALL): published (TimeUtility
                // .IsPassedCurrentTime(publishedAt)) and tagged "all".
                in_all_tab: now_ms >= music.published_at
                    && all_tag.is_some_and(|ids| ids.contains(&row.external_id)),
            }
        };
        models.push(model);
    }
    // OrderBy(MusicTrackType).ThenBy(Seq).ThenBy(SelectedId); stable.
    models.sort_by_key(|model| (model.soundtrack, model.seq, model.record_id));
    (models, refused)
}

// ---------------------------------------------------------------------------
// The presenter's state
// ---------------------------------------------------------------------------

/// `ScreenLayerMysekaiBGMSelectBootData`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BootData {
    pub(crate) site_id: u32,
    pub(crate) fixture_id: i32,
    pub(crate) color_id: i32,
}

/// `BGMSelectModel`: the records, the displayed ones, the chosen record
/// (`settingMusicCellData`), the highlighted one (`selectMusicCellData`) and
/// whether a record plays (`_isPlayingRecord`). Records are named by id: the
/// presenter compares the models by identity, and each record has one model
/// in the list.
#[derive(Clone, Debug)]
pub(crate) struct Presenter {
    pub(crate) boot: BootData,
    pub(crate) all: Vec<RecordModel>,
    /// Indices into `all`.
    pub(crate) display: Vec<usize>,
    pub(crate) setting: Option<i32>,
    pub(crate) select: Option<i32>,
    pub(crate) playing_record: bool,
}

impl Presenter {
    /// `Initialize` (`LoadSettingFromSaveData`, then `Resume(true)`'s data
    /// part). `site_setting` is the site's `UserMysekaiMusicPlayFixtureSetting`
    /// record id.
    pub(crate) fn initialize(
        boot: BootData,
        all: Vec<RecordModel>,
        site_setting: Option<i32>,
    ) -> Self {
        let mut presenter = Presenter {
            boot,
            all,
            display: Vec::new(),
            setting: None,
            select: None,
            playing_record: false,
        };
        presenter.create_display();
        // GetDefaultSelectedData: the site's setting record among the list,
        // else the list's first.
        let found = site_setting.filter(|id| presenter.all.iter().any(|m| m.record_id == *id));
        let (default, is_setting) = match found {
            Some(id) => (Some(id), true),
            None => (presenter.all.first().map(|m| m.record_id), false),
        };
        presenter.setting = if is_setting { default } else { None };
        presenter.select = default;
        presenter.playing_record = is_setting;
        // CreateSelectMusicCellData.
        if presenter.display.is_empty() {
            presenter.select = None;
        }
        if presenter.select.is_none_or(|id| id == 0) {
            presenter.select = presenter.first_displayed();
        }
        presenter.resume();
        presenter
    }

    /// `Resume`'s data part: the display list again; the highlighted record
    /// stays when it is displayed, else the first displayed one.
    pub(crate) fn resume(&mut self) {
        self.create_display();
        let shown = self
            .select
            .is_some_and(|id| self.display.iter().any(|&i| self.all[i].record_id == id));
        if !shown {
            self.select = self.first_displayed();
        }
    }

    fn first_displayed(&self) -> Option<i32> {
        self.display.first().map(|&i| self.all[i].record_id)
    }

    /// `CreateDisplayMusicData` for tab ALL and sort key Default
    /// (`SortDisplayDataList`: OrderBy(Seq).ThenBy(SelectedId)).
    fn create_display(&mut self) {
        let mut display: Vec<usize> = (0..self.all.len())
            .filter(|&i| self.all[i].in_all_tab)
            .collect();
        display.sort_by_key(|&i| (self.all[i].seq, self.all[i].record_id));
        self.display = display;
    }

    pub(crate) fn model(&self, id: Option<i32>) -> Option<&RecordModel> {
        let id = id?;
        self.all.iter().find(|model| model.record_id == id)
    }

    /// `SetupBGMPlayerView`'s `isSettingMusic`.
    pub(crate) fn is_setting_music(&self) -> bool {
        self.playing_record
            && self.setting.is_some()
            && self.select.is_some()
            && self.setting == self.select
    }

    /// `OnSelectMusic`.
    pub(crate) fn on_select(&mut self, record_id: i32) {
        self.select = Some(record_id);
        self.playing_record = true;
    }

    /// `SwitchBGMStatus`.
    pub(crate) fn switch_status(&mut self) {
        if self.playing_record {
            if self.select != self.setting {
                self.setting = self.select;
            } else {
                self.playing_record = false;
                self.setting = None;
            }
        } else {
            self.setting = self.select;
            self.playing_record = true;
        }
    }

    /// `ExecuteMysekaiMusicPlaySetApiIfNeeded` on exit: with a chosen record,
    /// the set request unless the site's setting names the same record (and,
    /// for a song, the same vocal); without one, the eject request when the
    /// site has a setting.
    pub(crate) fn exit_request(
        &self,
        site_row: Option<&MusicPlaySetting>,
    ) -> Option<MusicPlayPost> {
        let site = i32::try_from(self.boot.site_id).ok()?;
        match self.model(self.setting) {
            Some(model) => {
                if let Some(row) = site_row {
                    if row.mysekai_music_record_id == model.record_id
                        && (model.soundtrack || Some(row.music_vocal_id) == model.vocal_id)
                    {
                        return None;
                    }
                }
                Some(MusicPlayPost::Set(MusicPlaySetRequest {
                    mysekai_site_id: site,
                    mysekai_music_record_id: model.record_id,
                    music_vocal_id: model.vocal_id,
                    is_instrumental: false,
                    mysekai_fixture_id: self.boot.fixture_id,
                    texture_id: self.boot.color_id,
                }))
            }
            None => site_row.map(|_| {
                MusicPlayPost::Eject(MusicPlayEjectRequest {
                    mysekai_site_id: site,
                    mysekai_fixture_id: self.boot.fixture_id,
                    texture_id: self.boot.color_id,
                })
            }),
        }
    }
}

#[cfg(test)]
mod row_refusal_tests {
    use super::*;

    #[test]
    fn a_record_row_the_reader_cannot_read_is_dropped_and_named() {
        master::take_refused_rows();
        let records = parse_records(
            r#"[
                {"id": 1, "mysekaiMusicTrackType": "music", "externalId": 10},
                {"id": 2, "mysekaiMusicTrackType": "music"},
                {"id": 3, "mysekaiMusicTrackType": "music_sound_track", "externalId": 30}
            ]"#,
        )
        .expect("a table of rows");
        let kept: Vec<(i32, bool, i32)> = records
            .iter()
            .map(|row| (row.id, row.soundtrack, row.external_id))
            .collect();
        assert_eq!(kept, [(1, false, 10), (3, true, 30)]);
        let refused = master::take_refused_rows();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].row, "row 2");
        assert_eq!(refused[0].reason, "externalId is not an integer");
    }

    #[test]
    fn a_vocal_with_a_malformed_character_is_dropped_and_its_music_keeps_the_others() {
        master::take_refused_rows();
        let vocals = parse_vocals(
            r#"[
                {"id": 1, "musicId": 7, "musicVocalType": "sekai", "characters": [
                    {"characterType": "game_character", "characterId": 1, "seq": 1}]},
                {"id": 2, "musicId": 7, "musicVocalType": "sekai", "characters": [
                    {"characterType": "game_character", "characterId": "x", "seq": 1}]},
                {"id": 3, "musicId": 7, "musicVocalType": "instrumental", "characters": []}
            ]"#,
        )
        .expect("a table of rows");
        let ids: Vec<i32> = vocals[&7].iter().map(|vocal| vocal.id).collect();
        assert_eq!(ids, [1, 3]);
        assert_eq!(master::take_refused_rows().len(), 1);
    }

    #[test]
    fn a_text_that_is_not_a_table_refuses_the_table() {
        assert!(parse_tags("{}").is_err());
        assert!(parse_names("not json").is_err());
    }
}
