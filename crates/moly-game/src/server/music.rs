//! The user's music record settings (`userMysekaiMusicPlayFixtureSettings`,
//! `UserMysekaiMusicPlayFixtureSetting`: one row per site with the record,
//! the vocal version and whether it plays instrumental). The BGM reads the
//! setting of the site it plays on; the setting is user data, so it lives in
//! the server document and reaches the client as its copy
//! ([`super::client::music::ClientMusicPlaySettings`]).
//!
//! Named default: no row (no site has a setting; the default BGM choice
//! plays). The record master (`mysekai-music-records.json`) checks the record
//! ids when it is present; its absence is a named missing master.

use serde_json::{json, Map, Value};

use super::client::music::MusicPlaySetting;
use super::document::{int32, object, only};

pub(crate) const SECTION: &str = "userMysekaiMusicPlayFixtureSettings";

pub(crate) fn parse_rows(value: &Value) -> Result<Vec<MusicPlaySetting>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{SECTION} is not an array"))?;
    let mut sites = std::collections::BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiSiteId",
                    "mysekaiMusicRecordId",
                    "musicVocalId",
                    "isInstrumental",
                ],
                &at,
            )?;
            let setting = MusicPlaySetting {
                mysekai_site_id: int32(row, "mysekaiSiteId", &at)?,
                mysekai_music_record_id: int32(row, "mysekaiMusicRecordId", &at)?,
                music_vocal_id: int32(row, "musicVocalId", &at)?,
                is_instrumental: row
                    .get("isInstrumental")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| format!("{at}.isInstrumental is not a boolean"))?,
            };
            if !sites.insert(setting.mysekai_site_id) {
                return Err(format!("{at} repeats site {}", setting.mysekai_site_id));
            }
            Ok(setting)
        })
        .collect()
}

/// The section from a document (no row when absent).
pub(crate) fn parse(doc: &Map<String, Value>) -> Result<Vec<MusicPlaySetting>, String> {
    doc.get(SECTION).map_or(Ok(Vec::new()), parse_rows)
}

/// A `server.edit` of the section (next response); `None` for another path.
pub(crate) fn edit_path(
    rows: &mut Vec<MusicPlaySetting>,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [SECTION] => Some(parse_rows(value).map(|parsed| *rows = parsed)),
        _ => None,
    }
}

/// The record ids the master holds, checked when it is present.
pub(crate) fn check_records(
    rows: &[MusicPlaySetting],
    records: Option<&[i32]>,
) -> Result<(), String> {
    let Some(records) = records else {
        return Ok(());
    };
    for (i, row) in rows.iter().enumerate() {
        if !records.contains(&row.mysekai_music_record_id) {
            return Err(format!(
                "{SECTION}[{i}].mysekaiMusicRecordId = {} is not a master music record",
                row.mysekai_music_record_id
            ));
        }
    }
    Ok(())
}

/// The master parser of `mysekai-music-records.json`: the record ids.
pub(crate) fn parse_records(text: &str, masters: &mut super::Masters) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let rows = super::keyed_rows(&value, "mysekaiMusicRecords")?;
    let ids = rows
        .iter()
        .map(|row| super::int32(row, "id"))
        .collect::<Result<Vec<_>, String>>()?;
    masters.music_records = Some(ids);
    Ok(())
}

/// The native instrument `MOLY_AUDIO_MOCK_MUSIC_RECORD`
/// (`site:record:vocal`, comma-separated) as rows; a malformed entry stops
/// loudly.
pub(crate) fn parse_instrument(name: &str, raw: &str) -> Vec<MusicPlaySetting> {
    let mut rows: Vec<MusicPlaySetting> = Vec::new();
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let parts: Vec<&str> = entry.split(':').map(str::trim).collect();
        let [site, record, vocal] = parts.as_slice() else {
            panic!("{name} entry {entry:?} is not site:record:vocal");
        };
        let (Ok(site), Ok(record), Ok(vocal)) = (
            site.parse::<i32>(),
            record.parse::<i32>(),
            vocal.parse::<i32>(),
        ) else {
            panic!("{name} entry {entry:?} has a value that is not an integer");
        };
        if rows.iter().any(|row| row.mysekai_site_id == site) {
            panic!("{name} names site {site} twice");
        }
        rows.push(MusicPlaySetting {
            mysekai_site_id: site,
            mysekai_music_record_id: record,
            music_vocal_id: vocal,
            is_instrumental: false,
        });
    }
    rows
}

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": SECTION,
        "title": "Music record settings",
        "delivery": "next-response",
        "fields": [{
            "path": SECTION,
            "type": "rows",
            "row": {"mysekaiSiteId": "int", "mysekaiMusicRecordId": "int", "musicVocalId": "int", "isInstrumental": "bool"},
            "note": "one row per site; the BGM of a housing site plays the record set for it (a record whose package the root's record sidecar has not swept is refused by name and the default choice plays)",
        }],
    }])
}

#[cfg(test)]
mod tests {
    use super::super::client::music::rows_value;
    use super::*;

    #[test]
    fn rows_round_trip_and_refuse_by_name() {
        let rows = parse_instrument("X", "5:12:3, 7:1:1");
        assert_eq!(rows.len(), 2);
        let value = rows_value(&rows);
        assert_eq!(parse_rows(&value).unwrap(), rows);
        let mut doc = Map::new();
        assert!(parse(&doc).unwrap().is_empty());
        doc.insert(SECTION.into(), json!([{"mysekaiSiteId": 5, "mysekaiMusicRecordId": 1, "musicVocalId": 1, "isInstrumental": false},
            {"mysekaiSiteId": 5, "mysekaiMusicRecordId": 2, "musicVocalId": 1, "isInstrumental": true}]));
        assert!(parse(&doc).unwrap_err().contains("repeats site 5"));
        assert!(check_records(&rows, Some(&[12]))
            .unwrap_err()
            .contains("mysekaiMusicRecordId = 1"));
        assert!(check_records(&rows, None).is_ok());
    }
}
