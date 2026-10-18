//! The JSON form of the user data rows: the response keys' section names, the
//! renderers that write a row list as the key's array, and the parsers that
//! read a document's key back into rows.
//!
//! The renderers and parsers are plain functions over [`Value`]; the game
//! writes its copies from them and the server reads its document with them,
//! so the two sides cannot disagree about a field name.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::api::HousingLayoutFixture;
use crate::rows::{
    BirthdayPartyRow, Gamedata, HonorRow, MusicPlaySetting, OwnedMusicRecord, ScheduleRow, Stamina,
    UserAvatar, UserMysekaiBlueprint, UserMysekaiBlueprintLendCraftCount, UserMysekaiCanvas,
    UserMysekaiCharacterTalk, UserMysekaiFixture, UserMysekaiGamedata, UserMysekaiItem,
    UserMysekaiMaterial, UserMysekaiSystemFixtureAction, UserMysekaiTool, UserResource,
};

// ---------------------------------------------------------------------------
// Section names
// ---------------------------------------------------------------------------

/// `userMysekaiGamedata`.
pub const SECTION_GAMEDATA: &str = "userMysekaiGamedata";
/// `userMysekaiStamina`.
pub const SECTION_STAMINA: &str = "userMysekaiStamina";
/// `userMysekaiColorfulPass`.
pub const SECTION_PASS: &str = "userMysekaiColorfulPass";
/// `userBirthdayParties`.
pub const SECTION_BIRTHDAY_PARTIES: &str = "userBirthdayParties";
/// `userMaterials`.
pub const SECTION_MATERIALS: &str = "userMaterials";
/// `userMysekaiMaterials`.
pub const SECTION_MYSEKAI_MATERIALS: &str = "userMysekaiMaterials";
/// `userCards`.
pub const SECTION_CARDS: &str = "userCards";
/// `userHonors`.
pub const SECTION_HONORS: &str = "userHonors";
/// `masterConfigs`.
pub const SECTION_MASTER_CONFIGS: &str = "masterConfigs";
/// `userMysekaiMusicPlayFixtureSettings`.
pub const SECTION_MUSIC_SETTINGS: &str = "userMysekaiMusicPlayFixtureSettings";
/// `userAvatar`.
pub const SECTION_AVATAR: &str = "userAvatar";
/// `userMysekaiFixtures`.
pub const SECTION_FIXTURES: &str = "userMysekaiFixtures";
/// `userMysekaiCanvases`.
pub const SECTION_CANVASES: &str = "userMysekaiCanvases";
/// `userMysekaiBlueprints`.
pub const SECTION_BLUEPRINTS: &str = "userMysekaiBlueprints";
/// `userMysekaiItems`.
pub const SECTION_ITEMS: &str = "userMysekaiItems";
/// `userMysekaiCharacterTalks`.
pub const SECTION_CHARACTER_TALKS: &str = "userMysekaiCharacterTalks";
/// `userMysekaiMusicRecords`.
pub const SECTION_MUSIC_RECORDS: &str = "userMysekaiMusicRecords";
/// `userMysekaiSystemFixtureActions`.
pub const SECTION_SYSTEM_FIXTURE_ACTIONS: &str = "userMysekaiSystemFixtureActions";
/// `userMysekaiTools`.
pub const SECTION_TOOLS: &str = "userMysekaiTools";
/// `userMysekaiBlueprintLendCraftCounts`.
pub const SECTION_BLUEPRINT_LEND_CRAFT_COUNTS: &str = "userMysekaiBlueprintLendCraftCounts";

/// `UserAvatar`'s field names, in the source's order.
pub const AVATAR_FIELDS: [&str; 4] = [
    "avatarCostumeId",
    "avatarAccessoryId",
    "avatarSkinColorId",
    "avatarCoordinateId",
];

// ---------------------------------------------------------------------------
// Field readers
// ---------------------------------------------------------------------------

/// The object at `at`, refused by name otherwise.
pub fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{at} is not an object"))
}

/// Refuses a row that carries a field outside `allowed`.
pub fn only(value: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    match value.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("{at} has an unknown field {key}")),
        None => Ok(()),
    }
}

/// The field `key` of a row, refused by name when absent.
pub fn field<'a>(value: &'a Map<String, Value>, key: &str, at: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("{at} has no field {key}"))
}

/// An integer field of a row.
pub fn int64(value: &Map<String, Value>, key: &str, at: &str) -> Result<i64, String> {
    field(value, key, at)?
        .as_i64()
        .ok_or_else(|| format!("{at}.{key} is not an integer"))
}

/// A 32-bit integer field of a row.
pub fn int32(value: &Map<String, Value>, key: &str, at: &str) -> Result<i32, String> {
    let raw = int64(value, key, at)?;
    i32::try_from(raw).map_err(|_| format!("{at}.{key} = {raw} does not fit a 32-bit integer"))
}

/// A boolean field of a row.
pub fn boolean(value: &Map<String, Value>, key: &str, at: &str) -> Result<bool, String> {
    field(value, key, at)?
        .as_bool()
        .ok_or_else(|| format!("{at}.{key} is not a boolean"))
}

/// An array field of a row.
pub fn array<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<&'a Vec<Value>, String> {
    field(value, key, at)?
        .as_array()
        .ok_or_else(|| format!("{at}.{key} is not an array"))
}

/// The rows of an array value, refused by name otherwise.
pub fn rows<'a>(value: &'a Value, at: &str) -> Result<&'a Vec<Value>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{at} is not an array"))
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

/// `userMysekaiStamina`.
pub fn stamina_value(stamina: Stamina) -> Value {
    json!({
        "normalStamina": stamina.normal,
        "enhanceStamina": stamina.enhance,
        "boostStamina": stamina.boost,
    })
}

/// `userMysekaiGamedata`.
pub fn gamedata_value(gamedata: Gamedata) -> Value {
    json!({
        "mysekaiRank": gamedata.mysekai_rank,
        "totalExp": gamedata.total_exp,
        "refreshedAt": gamedata.refreshed_at,
        "isMysekaiTutorialEnd": gamedata.is_mysekai_tutorial_end,
    })
}

/// `mysekaiPhenomenaSchedules`.
pub fn schedule_rows_value(rows: &[ScheduleRow]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiRefreshTimePeriodId": row.refresh_time_period_id,
                    "scheduleDate": row.schedule_date,
                    "mysekaiPhenomenaId": row.phenomena_id,
                })
            })
            .collect(),
    )
}

/// A `userBirthdayParties` row.
pub fn party_value(row: BirthdayPartyRow) -> Value {
    json!({
        "birthdayPartyId": row.birthday_party_id,
        "deliveryTotalPoint": row.delivery_total_point,
        "droppedMysekaiMaterialCount": row.dropped_mysekai_material_count,
        "obtainedMysekaiMaterialCount": row.obtained_mysekai_material_count,
    })
}

/// A `userHonors` row.
pub fn honor_value(row: HonorRow) -> Value {
    json!({"honorId": row.honor_id, "level": row.level, "obtainedAt": row.obtained_at})
}

/// A table of an id and a quantity (`userMaterials`,
/// `userMysekaiMaterials`).
pub fn quantities_value(map: &BTreeMap<i32, i32>, id_key: &str) -> Value {
    Value::Array(
        map.iter()
            .map(|(id, quantity)| json!({id_key: id, "quantity": quantity}))
            .collect(),
    )
}

/// `userMysekaiFixtures`.
pub fn fixtures_value(rows: &[UserMysekaiFixture]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiFixtureId": row.mysekai_fixture_id,
                    "textureId": row.texture_id,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

/// `userMysekaiCanvases`.
pub fn canvases_value(rows: &[UserMysekaiCanvas]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiFixtureId": row.mysekai_fixture_id,
                    "cardId": row.card_id,
                    "isSpecialTraining": row.is_special_training,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

/// `userMysekaiBlueprints`.
pub fn blueprints_value(rows: &[UserMysekaiBlueprint]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiBlueprintId": row.mysekai_blueprint_id,
                    "obtainedAt": row.obtained_at,
                })
            })
            .collect(),
    )
}

/// `userMysekaiItems`.
pub fn items_value(rows: &[UserMysekaiItem]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiItemId": row.mysekai_item_id,
                    "quantity": row.quantity,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

/// `userMysekaiMaterials`.
pub fn materials_value(rows: &[UserMysekaiMaterial]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| json!({"mysekaiMaterialId": row.mysekai_material_id, "quantity": row.quantity}))
            .collect(),
    )
}

/// `userMysekaiCharacterTalks`.
pub fn character_talks_value(rows: &[UserMysekaiCharacterTalk]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({"mysekaiCharacterTalkId": row.mysekai_character_talk_id, "isRead": row.is_read})
            })
            .collect(),
    )
}

/// The `userMysekaiGamedata` fields the inventory screens read.
pub fn user_gamedata_value(gamedata: UserMysekaiGamedata) -> Value {
    json!({
        "mysekaiRank": gamedata.mysekai_rank,
        "totalExp": gamedata.total_exp,
        "mysekaiMaterialPossessionLevel": gamedata.mysekai_material_possession_level,
        "mysekaiFixturePossessionLevel": gamedata.mysekai_fixture_possession_level,
    })
}

/// A `UserResource` row list.
pub fn resources_value(rows: &[UserResource]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "resourceType": row.resource_type,
                    "resourceId": row.resource_id,
                    "resourceLevel": row.resource_level,
                    "quantity": row.quantity,
                })
            })
            .collect(),
    )
}

/// `userMysekaiMusicPlayFixtureSettings`.
pub fn music_settings_value(rows: &[MusicPlaySetting]) -> Value {
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

/// `userMysekaiMusicRecords`.
pub fn music_records_value(rows: &[OwnedMusicRecord]) -> Value {
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

/// `userAvatar`: the wear's four nullable ids.
pub fn avatar_value(avatar: &UserAvatar) -> Value {
    Value::Object(
        [
            (AVATAR_FIELDS[0], avatar.costume),
            (AVATAR_FIELDS[1], avatar.accessory),
            (AVATAR_FIELDS[2], avatar.skin_color),
            (AVATAR_FIELDS[3], avatar.coordinate),
        ]
        .into_iter()
        .map(|(field, id)| (field.to_owned(), json!(id)))
        .collect(),
    )
}

/// `userMysekaiSystemFixtureActions`.
pub fn system_fixture_actions_value(rows: &[UserMysekaiSystemFixtureAction]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| json!({"mysekaiSystemFixtureId": row.mysekai_system_fixture_id}))
            .collect(),
    )
}

/// `userMysekaiTools`.
pub fn tools_value(rows: &[UserMysekaiTool]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiToolId": row.mysekai_tool_id,
                    "quantity": row.quantity,
                    "durability": row.durability,
                    "lastObtainedAt": row.last_obtained_at,
                })
            })
            .collect(),
    )
}

/// `userMysekaiTools`'s value in a document (`null` while not seated).
pub fn tools_section_value(rows: &Option<Vec<UserMysekaiTool>>) -> Value {
    rows.as_deref().map_or(Value::Null, tools_value)
}

/// `userMysekaiBlueprintLendCraftCounts`.
pub fn blueprint_lend_craft_counts_value(rows: &[UserMysekaiBlueprintLendCraftCount]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "mysekaiBlueprintTermId": row.mysekai_blueprint_term_id,
                    "craftCount": row.craft_count,
                })
            })
            .collect(),
    )
}

/// `userMysekaiSiteHousingLayouts`'s `mysekaiFixtures` of one site, in the
/// served record form.
pub fn housing_layout_fixtures_value(fixtures: &[HousingLayoutFixture]) -> Value {
    Value::Array(
        fixtures
            .iter()
            .map(|row| {
                json!({
                    "package": row.package,
                    "mysekaiFixtureId": row.mysekai_fixture_id,
                    "minimum": {
                        "x": row.minimum.x,
                        "y": row.minimum.y,
                        "z": row.minimum.z,
                    },
                    "maximum": {
                        "x": row.maximum.x,
                        "y": row.maximum.y,
                        "z": row.maximum.z,
                    },
                    "centerY": row.center_y,
                    "layoutType": row.layout_type,
                    "rotation": row.rotation,
                    "textureId": row.texture_id,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// `userBirthdayParties`.
pub fn parse_parties(value: &Value) -> Result<Vec<BirthdayPartyRow>, String> {
    let rows = rows(value, SECTION_BIRTHDAY_PARTIES)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_BIRTHDAY_PARTIES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "birthdayPartyId",
                    "deliveryTotalPoint",
                    "droppedMysekaiMaterialCount",
                    "obtainedMysekaiMaterialCount",
                ],
                &at,
            )?;
            Ok(BirthdayPartyRow {
                birthday_party_id: int32(row, "birthdayPartyId", &at)?,
                delivery_total_point: int32(row, "deliveryTotalPoint", &at)?,
                dropped_mysekai_material_count: int32(row, "droppedMysekaiMaterialCount", &at)?,
                obtained_mysekai_material_count: int32(row, "obtainedMysekaiMaterialCount", &at)?,
            })
        })
        .collect()
}

/// `userMaterials` / `userMysekaiMaterials`: rows of an id and a quantity.
pub fn parse_quantities(
    value: &Value,
    at: &str,
    id_key: &str,
) -> Result<BTreeMap<i32, i32>, String> {
    let rows = rows(value, at)?;
    let mut out = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let row_at = format!("{at}[{i}]");
        let row = object(row, &row_at)?;
        only(row, &[id_key, "quantity"], &row_at)?;
        let id = int32(row, id_key, &row_at)?;
        if out.insert(id, int32(row, "quantity", &row_at)?).is_some() {
            return Err(format!("{row_at} repeats {id_key} {id}"));
        }
    }
    Ok(out)
}

/// `userCards`: the owned card ids.
pub fn parse_cards(value: &Value) -> Result<Vec<i64>, String> {
    let rows = rows(value, SECTION_CARDS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_CARDS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["cardId"], &at)?;
            int64(row, "cardId", &at)
        })
        .collect()
}

/// `userHonors`.
pub fn parse_honors(value: &Value) -> Result<Vec<HonorRow>, String> {
    let rows = rows(value, SECTION_HONORS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_HONORS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["honorId", "level", "obtainedAt"], &at)?;
            Ok(HonorRow {
                honor_id: int32(row, "honorId", &at)?,
                level: int32(row, "level", &at)?,
                obtained_at: int64(row, "obtainedAt", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiFixtures`.
pub fn parse_fixtures(value: &Value) -> Result<Vec<UserMysekaiFixture>, String> {
    let rows = rows(value, SECTION_FIXTURES)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_FIXTURES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &["mysekaiFixtureId", "textureId", "quantity", "lastObtainedAt"],
                &at,
            )?;
            Ok(UserMysekaiFixture {
                mysekai_fixture_id: int32(row, "mysekaiFixtureId", &at)?,
                texture_id: int32(row, "textureId", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiCanvases`.
pub fn parse_canvases(value: &Value) -> Result<Vec<UserMysekaiCanvas>, String> {
    let rows = rows(value, SECTION_CANVASES)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_CANVASES}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiFixtureId",
                    "cardId",
                    "isSpecialTraining",
                    "quantity",
                    "lastObtainedAt",
                ],
                &at,
            )?;
            Ok(UserMysekaiCanvas {
                mysekai_fixture_id: int32(row, "mysekaiFixtureId", &at)?,
                card_id: int32(row, "cardId", &at)?,
                is_special_training: boolean(row, "isSpecialTraining", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiBlueprints`.
pub fn parse_blueprints(value: &Value) -> Result<Vec<UserMysekaiBlueprint>, String> {
    let rows = rows(value, SECTION_BLUEPRINTS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_BLUEPRINTS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiBlueprintId", "obtainedAt"], &at)?;
            Ok(UserMysekaiBlueprint {
                mysekai_blueprint_id: int32(row, "mysekaiBlueprintId", &at)?,
                obtained_at: int64(row, "obtainedAt", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiItems`.
pub fn parse_items(value: &Value) -> Result<Vec<UserMysekaiItem>, String> {
    let rows = rows(value, SECTION_ITEMS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_ITEMS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiItemId", "quantity", "lastObtainedAt"], &at)?;
            Ok(UserMysekaiItem {
                mysekai_item_id: int32(row, "mysekaiItemId", &at)?,
                quantity: int32(row, "quantity", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiCharacterTalks`.
pub fn parse_character_talks(value: &Value) -> Result<Vec<UserMysekaiCharacterTalk>, String> {
    let rows = rows(value, SECTION_CHARACTER_TALKS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_CHARACTER_TALKS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiCharacterTalkId", "isRead"], &at)?;
            Ok(UserMysekaiCharacterTalk {
                mysekai_character_talk_id: int32(row, "mysekaiCharacterTalkId", &at)?,
                is_read: boolean(row, "isRead", &at)?,
            })
        })
        .collect()
}

/// `policies.talkReadObtainedResources`: `UserResource` rows.
pub fn parse_resources(value: &Value) -> Result<Vec<UserResource>, String> {
    const AT: &str = "policies.talkReadObtainedResources";
    let rows = rows(value, AT)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{AT}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &["resourceType", "resourceId", "resourceLevel", "quantity"],
                &at,
            )?;
            Ok(UserResource {
                resource_type: row
                    .get("resourceType")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{at}.resourceType is not a string"))?
                    .to_owned(),
                resource_id: int32(row, "resourceId", &at)?,
                resource_level: int32(row, "resourceLevel", &at)?,
                quantity: int32(row, "quantity", &at)?,
            })
        })
        .collect()
}

/// `userMysekaiMusicPlayFixtureSettings`, one row per site.
pub fn parse_music_settings(value: &Value) -> Result<Vec<MusicPlaySetting>, String> {
    let rows = rows(value, SECTION_MUSIC_SETTINGS)?;
    let mut sites = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_MUSIC_SETTINGS}[{i}]");
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
                is_instrumental: boolean(row, "isInstrumental", &at)?,
            };
            if !sites.insert(setting.mysekai_site_id) {
                return Err(format!("{at} repeats site {}", setting.mysekai_site_id));
            }
            Ok(setting)
        })
        .collect()
}

/// `userMysekaiMusicRecords`.
pub fn parse_music_records(value: &Value) -> Result<Vec<OwnedMusicRecord>, String> {
    let rows = rows(value, SECTION_MUSIC_RECORDS)?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_MUSIC_RECORDS}[{i}]");
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

/// One nullable id of `userAvatar`, refused by name when it is neither null
/// nor a 32-bit id.
fn nullable_id(value: &Value, at: &str) -> Result<Option<i32>, String> {
    match value {
        Value::Null => Ok(None),
        _ => value
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .map(Some)
            .ok_or_else(|| format!("{at} is neither null nor an id")),
    }
}

/// `userAvatar`.
pub fn parse_avatar(value: &Value) -> Result<UserAvatar, String> {
    let row = object(value, SECTION_AVATAR)?;
    only(row, &AVATAR_FIELDS, SECTION_AVATAR)?;
    let mut avatar = UserAvatar::default();
    for (field, slot) in [
        (AVATAR_FIELDS[0], &mut avatar.costume),
        (AVATAR_FIELDS[1], &mut avatar.accessory),
        (AVATAR_FIELDS[2], &mut avatar.skin_color),
        (AVATAR_FIELDS[3], &mut avatar.coordinate),
    ] {
        let at = format!("{SECTION_AVATAR}.{field}");
        *slot = nullable_id(row.get(field).unwrap_or(&Value::Null), &at)?;
    }
    Ok(avatar)
}

/// `userMysekaiSystemFixtureActions`.
pub fn parse_system_fixture_actions(
    value: &Value,
) -> Result<Vec<UserMysekaiSystemFixtureAction>, String> {
    let rows = rows(value, SECTION_SYSTEM_FIXTURE_ACTIONS)?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_SYSTEM_FIXTURE_ACTIONS}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiSystemFixtureId"], &at)?;
            let id = int32(row, "mysekaiSystemFixtureId", &at)?;
            if id < 1 {
                return Err(format!("{at}.mysekaiSystemFixtureId = {id} is below 1"));
            }
            if !seen.insert(id) {
                return Err(format!("{at} repeats system fixture {id}"));
            }
            Ok(UserMysekaiSystemFixtureAction {
                mysekai_system_fixture_id: id,
            })
        })
        .collect()
}

/// `userMysekaiBlueprintLendCraftCounts`: one row per term, a term named once.
pub fn parse_blueprint_lend_craft_counts(
    value: &Value,
) -> Result<Vec<UserMysekaiBlueprintLendCraftCount>, String> {
    const AT: &str = SECTION_BLUEPRINT_LEND_CRAFT_COUNTS;
    let rows = rows(value, AT)?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{AT}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiBlueprintTermId", "craftCount"], &at)?;
            let term = UserMysekaiBlueprintLendCraftCount {
                mysekai_blueprint_term_id: int32(row, "mysekaiBlueprintTermId", &at)?,
                craft_count: int32(row, "craftCount", &at)?,
            };
            if !seen.insert(term.mysekai_blueprint_term_id) {
                return Err(format!(
                    "{at} repeats blueprint term {}",
                    term.mysekai_blueprint_term_id
                ));
            }
            Ok(term)
        })
        .collect()
}

/// `userMysekaiTools` (`null`: not seated).
pub fn parse_tools(value: &Value) -> Result<Option<Vec<UserMysekaiTool>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let rows = rows(value, SECTION_TOOLS)?;
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION_TOOLS}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &["mysekaiToolId", "quantity", "durability", "lastObtainedAt"],
                &at,
            )?;
            Ok(UserMysekaiTool {
                mysekai_tool_id: int32(row, "mysekaiToolId", &at)?,
                quantity: int32(row, "quantity", &at)?,
                durability: int32(row, "durability", &at)?,
                last_obtained_at: int64(row, "lastObtainedAt", &at)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Some)
}
