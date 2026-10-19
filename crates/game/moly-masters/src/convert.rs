//! The convert screen's master tables: the items, the machine's slots and
//! levels, the boxes a convert spends and draws, the obtain rows of items
//! that draw no box, the record categories, and the world pass's rate.
//!
//! The tables, as the master names them:
//!
//! - `mysekaiConvertItems`: the items. `consumeResourceBoxId` names the
//!   `mysekai_convert_consumption` box a convert spends;
//!   `mysekaiConvertItemObtainType` says what a convert yields
//!   (`resource_box`, the `mysekai_convert_obtain` box `externalId`;
//!   `mysekai_blueprint` / `mysekai_music_record`, the row of the same
//!   `externalId` in the obtain table below).
//! - `mysekaiConvertFixtureSlot`: the machine's slot numbers.
//! - `mysekaiConvertFixtureLevels` and `mysekaiConvertFixtureLevelReleases`:
//!   the machine's levels and what a level releases.
//! - `resourceBoxes`: keyed by (resourceBoxPurpose, id); a convert's boxes
//!   carry `mysekai_convert_consumption` or `mysekai_convert_obtain`. A
//!   detail's quantity is `resourceQuantity`.
//! - `mysekaiConvertItemObtainMysekaiBlueprints`,
//!   `mysekaiConvertItemObtainMysekaiMusicRecords`,
//!   `mysekaiMusicRecordCategories` and `mysekaiMusicRecords`.
//! - `colorfulPasses`: `mysekaiConvertFixtureConvertMinutesDecreaseRate`.
//! - `configs`: `mysekai_convert_time_shorten_consume_jewel_per_minute`.
//!
//! Every reader fails by name on a column it cannot read; the tables are
//! matched to the others by id, never by position.

use serde_json::Value;

/// The box a convert spends (`resourceBoxPurpose` of its rows).
pub const BOX_CONSUMPTION: &str = "mysekai_convert_consumption";
/// The box a `resource_box` convert draws.
pub const BOX_OBTAIN: &str = "mysekai_convert_obtain";
/// `configs.mysekai_convert_time_shorten_consume_jewel_per_minute`.
pub const CONFIG_JEWEL_PER_MINUTE: &str = "mysekai_convert_time_shorten_consume_jewel_per_minute";

fn string_field(row: &Value, key: &str) -> Result<String, String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} of {row} is not a string"))
}

/// A column some rows of the master lack: `None` when the row has no such
/// key; a key that is not a string is refused (never defaulted).
fn string_column(row: &Value, key: &str) -> Result<Option<String>, String> {
    match row.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| format!("{key} of {row} is not a string")),
    }
}

/// `mysekaiConvertItemObtainType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObtainType {
    /// `resource_box`: the `mysekai_convert_obtain` box `external_id`.
    ResourceBox,
    /// `mysekai_blueprint`: a `mysekaiConvertItemObtainMysekaiBlueprints`
    /// row.
    Blueprint,
    /// `mysekai_music_record`: a
    /// `mysekaiConvertItemObtainMysekaiMusicRecords` row.
    Record,
}

impl ObtainType {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "resource_box" => Some(Self::ResourceBox),
            "mysekai_blueprint" => Some(Self::Blueprint),
            "mysekai_music_record" => Some(Self::Record),
            _ => None,
        }
    }
}

/// `MysekaiConvertItemRefreshCycle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefreshCycle {
    /// `none`.
    None,
    /// `weekly`.
    Weekly,
    /// `monthly`.
    Monthly,
}

impl RefreshCycle {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "none" => Some(Self::None),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            _ => None,
        }
    }
}

/// One `mysekaiConvertItems` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertItem {
    pub id: i32,
    pub tab_id: i32,
    /// The `mysekai_convert_consumption` box a convert spends.
    pub consume_box_id: i32,
    pub obtain_type: ObtainType,
    /// The obtain box or the obtain row of `obtain_type`.
    pub external_id: i32,
    pub required_level: i32,
    pub convert_minutes: i32,
    /// 0 when the row states no `convertLimit`.
    pub convert_limit: i32,
    pub once_convert_limit: i32,
    pub refresh: RefreshCycle,
    pub start_at: i64,
    pub end_at: i64,
}

/// `mysekaiConvertItems`, master order.
pub fn parse_items(text: &str) -> Result<Vec<ConvertItem>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertItems")?
        .iter()
        .map(|row| {
            let id = crate::int32(row, "id")?;
            let kind = row
                .get("mysekaiConvertItemObtainType")
                .and_then(Value::as_str)
                .and_then(ObtainType::parse)
                .ok_or_else(|| {
                    format!(
                        "mysekaiConvertItemObtainType of mysekaiConvertItems {id} is {:?}, not a type this reader knows",
                        row.get("mysekaiConvertItemObtainType")
                    )
                })?;
            let refresh = match row.get("refreshCycle") {
                None | Some(Value::Null) => RefreshCycle::None,
                Some(value) => value
                    .as_str()
                    .and_then(RefreshCycle::parse)
                    .ok_or_else(|| {
                        format!("refreshCycle of mysekaiConvertItems {id} is {value}, not a cycle this reader knows")
                    })?,
            };
            Ok(ConvertItem {
                id,
                tab_id: crate::int32(row, "mysekaiConvertItemTabId")?,
                consume_box_id: crate::int32(row, "consumeResourceBoxId")?,
                obtain_type: kind,
                external_id: crate::int32(row, "externalId")?,
                required_level: crate::int32(row, "requiredConvertFixtureLevel")?,
                convert_minutes: crate::int32(row, "convertMinutes")?,
                convert_limit: row.get("convertLimit").and_then(Value::as_i64).unwrap_or(0) as i32,
                once_convert_limit: crate::int32(row, "onceConvertLimit")?,
                refresh,
                start_at: row.get("startAt").and_then(Value::as_i64).unwrap_or(0),
                end_at: row.get("endAt").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect()
}

/// `mysekaiConvertItemTabs`: (id, `seq`), master order.
pub fn parse_tabs(text: &str) -> Result<Vec<(i32, i32)>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertItemTabs")?
        .iter()
        .map(|row| Ok((crate::int32(row, "id")?, crate::int32(row, "seq")?)))
        .collect()
}

/// `mysekaiConvertFixtureSlot`: (slotNoUpperLimit,
/// slotNoUpperLimitForMysekaiColorfulPass) of the first row.
pub fn parse_slot(text: &str) -> Result<(i32, i32), String> {
    let rows = crate::load::master_rows_of(text, "mysekaiConvertFixtureSlot")?;
    let row = rows.first().ok_or("mysekaiConvertFixtureSlot has no row")?;
    Ok((
        crate::int32(row, "slotNoUpperLimit")?,
        crate::int32(row, "slotNoUpperLimitForMysekaiColorfulPass")?,
    ))
}

/// `mysekaiConvertFixtureLevels`: the `level` column, id order.
pub fn parse_levels(text: &str) -> Result<Vec<i32>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertFixtureLevels")?
        .iter()
        .map(|row| crate::int32(row, "level"))
        .collect()
}

/// `mysekaiConvertFixtureLevelReleases`, master order.
pub fn parse_level_releases(text: &str) -> Result<Vec<LevelRelease>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertFixtureLevelReleases")?
        .iter()
        .map(|row| {
            Ok(LevelRelease {
                id: crate::int32(row, "id")?,
                level: crate::int32(row, "mysekaiConvertFixtureLevel")?,
                kind: string_field(row, "mysekaiConvertFixtureLevelReleaseType")?,
            })
        })
        .collect()
}

/// `resourceBoxes`: each box's details, `seq` order.
pub fn parse_boxes(text: &str) -> Result<Vec<ResourceBox>, String> {
    crate::load::master_rows_of(text, "resourceBoxes")?
        .iter()
        .filter(|row| row.get("details").is_some_and(|details| !details.is_null()))
        .map(|row| {
            let id = crate::int32(row, "id")?;
            let purpose = string_field(row, "resourceBoxPurpose")?;
            let details = row["details"].as_array().ok_or_else(|| {
                format!("details of resourceBoxes {purpose} {id} is not an array")
            })?;
            let mut details = details.iter().collect::<Vec<_>>();
            details.sort_by_key(|detail| detail.get("seq").and_then(Value::as_i64).unwrap_or(0));
            let details = details
                .into_iter()
                .map(|detail| {
                    Ok(BoxDetail {
                        resource_type: string_field(detail, "resourceType")?,
                        resource_id: detail.get("resourceId").and_then(Value::as_i64).unwrap_or(0) as i32,
                        quantity: detail
                            .get("resourceQuantity")
                            .and_then(Value::as_i64)
                            .unwrap_or(1) as i32,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ResourceBox {
                id,
                purpose,
                name: string_column(row, "name")?,
                description: string_column(row, "description")?,
                details,
            })
        })
        .collect()
}

/// `mysekaiConvertItemObtainMysekaiBlueprints`, id order.
pub fn parse_blueprint_obtains(text: &str) -> Result<Vec<BlueprintObtain>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertItemObtainMysekaiBlueprints")?
        .iter()
        .map(|row| {
            Ok(BlueprintObtain {
                id: crate::int32(row, "id")?,
                kind: string_field(row, "mysekaiConvertItemObtainMysekaiBlueprintType")?,
                quantity: row.get("quantity").and_then(Value::as_i64).unwrap_or(1) as i32,
            })
        })
        .collect()
}

/// `mysekaiConvertItemObtainMysekaiMusicRecords`, id order.
pub fn parse_record_obtains(text: &str) -> Result<Vec<RecordObtain>, String> {
    crate::load::master_rows_of(text, "mysekaiConvertItemObtainMysekaiMusicRecords")?
        .iter()
        .map(|row| {
            Ok(RecordObtain {
                id: crate::int32(row, "id")?,
                kind: string_field(row, "mysekaiConvertItemObtainMysekaiMusicRecordType")?,
                quantity: row.get("quantity").and_then(Value::as_i64).unwrap_or(1) as i32,
            })
        })
        .collect()
}

/// `mysekaiMusicRecordCategories`, master order.
pub fn parse_record_categories(text: &str) -> Result<Vec<RecordCategory>, String> {
    crate::load::master_rows_of(text, "mysekaiMusicRecordCategories")?
        .iter()
        .map(|row| {
            Ok(RecordCategory {
                id: crate::int32(row, "id")?,
                track_type: string_field(row, "mysekaiMusicTrackType")?,
                seq: crate::int32(row, "seq")?,
            })
        })
        .collect()
}

/// `mysekaiMusicRecords`: id and category, master order.
pub fn parse_music_records(text: &str) -> Result<Vec<MusicRecord>, String> {
    crate::load::master_rows_of(text, "mysekaiMusicRecords")?
        .iter()
        .map(|row| {
            Ok(MusicRecord {
                id: crate::int32(row, "id")?,
                category: row
                    .get("externalId")
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32,
            })
        })
        .collect()
}

/// `colorfulPasses.mysekaiConvertFixtureConvertMinutesDecreaseRate` of the
/// first row: the fraction a convert's minutes are multiplied by while the
/// pass runs.
pub fn parse_pass_rate(text: &str) -> Result<f32, String> {
    let rows = crate::load::master_rows_of(text, "colorfulPasses")?;
    let row = rows.first().ok_or("colorfulPasses has no row")?;
    let raw = row
        .get("mysekaiConvertFixtureConvertMinutesDecreaseRate")
        .and_then(Value::as_f64)
        .ok_or("colorfulPasses has no mysekaiConvertFixtureConvertMinutesDecreaseRate")?
        as f32;
    if !raw.is_finite() || raw < 0.0 {
        return Err(format!(
            "colorfulPasses.mysekaiConvertFixtureConvertMinutesDecreaseRate = {raw} is negative"
        ));
    }
    Ok(raw)
}

/// `configs`: the value of `mysekai_convert_time_shorten_consume_jewel_per_minute`.
pub fn parse_shorten_config(text: &str) -> Result<i32, String> {
    let rows = crate::load::master_rows_of(text, "configs")?;
    let row = rows
        .iter()
        .find(|row| row.get("configKey").and_then(Value::as_str) == Some(CONFIG_JEWEL_PER_MINUTE))
        .ok_or_else(|| format!("configs has no {CONFIG_JEWEL_PER_MINUTE} row"))?;
    row.get("value")
        .and_then(Value::as_str)
        .and_then(|value| value.trim().parse::<i32>().ok())
        .ok_or_else(|| {
            format!("configs {CONFIG_JEWEL_PER_MINUTE} value {:?} is not an integer", row.get("value"))
        })
}

/// One `mysekaiConvertFixtureLevelReleases` row: a level and what it
/// releases (`mysekaiConvertFixtureLevelReleaseType`, `convert_time_shorten`
/// releases the skip).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelRelease {
    pub id: i32,
    pub level: i32,
    pub kind: String,
}

/// One detail of a `resourceBoxes` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxDetail {
    pub resource_type: String,
    /// 0 when the detail names no resource (a `coin` row, for one).
    pub resource_id: i32,
    pub quantity: i32,
}

/// One `resourceBoxes` row: a (purpose, id) pair, its details, and the row's
/// `name` and `description`. Most rows of the master carry neither column
/// (e.g. `id` 1 has a description and no name in the 2026-10-18 master), so
/// both are `None` where the row lacks them; a column that is present must be
/// a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBox {
    pub id: i32,
    pub purpose: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub details: Vec<BoxDetail>,
}

/// One `mysekaiConvertItemObtainMysekaiBlueprints` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlueprintObtain {
    pub id: i32,
    pub kind: String,
    pub quantity: i32,
}

/// One `mysekaiConvertItemObtainMysekaiMusicRecords` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordObtain {
    pub id: i32,
    pub kind: String,
    pub quantity: i32,
}

/// One `mysekaiMusicRecordCategories` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordCategory {
    pub id: i32,
    pub track_type: String,
    /// `seq`: `GetValidMusicRecordCategories` orders the categories by it.
    pub seq: i32,
}

/// One `mysekaiMusicRecords` row: the record and its category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicRecord {
    pub id: i32,
    /// The `externalId` column (0 when absent), which names the category.
    pub category: i32,
}

/// The convert screen's master tables. Each is `None` until its table is
/// in; a reader that needs a missing table refuses by name.
#[derive(bevy::prelude::Resource, Clone, Debug, Default)]
pub struct ConvertMasters {
    pub items: Option<Vec<ConvertItem>>,
    pub tabs: Option<Vec<(i32, i32)>>,
    /// (slotNoUpperLimit, slotNoUpperLimitForMysekaiColorfulPass).
    pub slots: Option<(i32, i32)>,
    pub levels: Option<Vec<i32>>,
    pub level_releases: Option<Vec<LevelRelease>>,
    pub boxes: Option<Vec<ResourceBox>>,
    pub blueprint_obtains: Option<Vec<BlueprintObtain>>,
    pub record_obtains: Option<Vec<RecordObtain>>,
    pub record_categories: Option<Vec<RecordCategory>>,
    pub music_records: Option<Vec<MusicRecord>>,
    pub pass_rate: Option<f32>,
    pub shorten_jewel_per_minute: Option<i32>,
    /// `configs.mysekai_refresh_hour` and `mysekai_weekly_reference`: the
    /// calendar weekly and monthly rows refresh on.
    pub calendar: Option<crate::clock::CalendarRefresh>,
}

impl ConvertMasters {
    /// A table set of zero rows: every reader that needs a table refuses.
    pub fn empty() -> Self {
        Self::default()
    }

    /// `MasterDataManager.GetMasterMysekaiConvertItem(id)`.
    pub fn item(&self, id: i32) -> Option<&ConvertItem> {
        self.items.as_ref()?.iter().find(|item| item.id == id)
    }

    /// The box a purpose and an id name.
    pub fn box_of(&self, purpose: &str, id: i32) -> Option<&ResourceBox> {
        self.boxes
            .as_ref()?
            .iter()
            .find(|box_| box_.purpose == purpose && box_.id == id)
    }

    /// The boxes of one purpose, id order.
    pub fn boxes_of<'a>(&'a self, purpose: &'a str) -> Vec<&'a ResourceBox> {
        let mut boxes = self
            .boxes
            .as_ref()
            .map(|boxes| {
                boxes
                    .iter()
                    .filter(|box_| box_.purpose == purpose)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        boxes.sort_by_key(|box_| box_.id);
        boxes
    }

    pub fn blueprint_obtain(&self, id: i32) -> Option<&BlueprintObtain> {
        self.blueprint_obtains
            .as_ref()?
            .iter()
            .find(|row| row.id == id)
    }

    pub fn record_obtain(&self, id: i32) -> Option<&RecordObtain> {
        self.record_obtains.as_ref()?.iter().find(|row| row.id == id)
    }

    /// The level the `convert_time_shorten` release row names, if any.
    pub fn released_shorten_level(&self) -> Option<i32> {
        self.level_releases
            .as_ref()?
            .iter()
            .find(|row| row.kind == "convert_time_shorten")
            .map(|row| row.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `resourceBoxes` rows carry `name` and `description` where the master
    /// gives them (`id` 1 in the 2026-10-18 master has a description and no
    /// name): a missing column reads `None`, a present one reads its text,
    /// and a present column that is not a string is refused, never defaulted.
    #[test]
    fn a_box_column_some_rows_lack_is_none_not_defaulted() {
        let rows = r#"[
            {"id": 1, "resourceBoxPurpose": "p", "resourceBoxType": "expand",
             "description": "only a description",
             "details": [{"seq": 1, "resourceType": "material", "resourceId": 6, "resourceQuantity": 100}]},
            {"id": 2, "resourceBoxPurpose": "p", "resourceBoxType": "expand",
             "name": "box name", "description": "box description",
             "details": [{"seq": 1, "resourceType": "material", "resourceId": 7, "resourceQuantity": 1},
                         {"seq": 2, "resourceType": "material", "resourceId": 8, "resourceQuantity": 3}]}
        ]"#;
        let boxes = parse_boxes(rows).expect("the master's columns");
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].name, None);
        assert_eq!(boxes[0].description.as_deref(), Some("only a description"));
        assert_eq!(boxes[1].name.as_deref(), Some("box name"));
        assert_eq!(boxes[1].description.as_deref(), Some("box description"));
        assert_eq!(boxes[1].details.len(), 2);
        let bad = r#"[{"id": 3, "resourceBoxPurpose": "p", "name": 5,
                      "details": [{"seq": 1, "resourceType": "material", "resourceId": 1, "resourceQuantity": 1}]}]"#;
        assert!(parse_boxes(bad).is_err());
    }
}
