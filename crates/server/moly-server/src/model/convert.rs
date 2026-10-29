//! The convert screen's three requests: `PostUserMysekaiConvertStartApi`
//! (`UserMysekaiConvertStartRequest`), `PostUserMysekaiConvertReceiveApi`
//! (`UserMysekaiConvertReceiveRequest`) and
//! `PostUserMysekaiConvertShortenApi` (`UserMysekaiConvertShortenRequest`).
//! Each names its slot in the call's path; start and shorten answer with a
//! `SuiteUser` reply, receive answers with the resources it produced and the
//! tables it changed.
//!
//! **Document keys** (the source's `SuiteUser` keys; optional on read, always
//! written):
//! - `userMysekaiConvertSlots`: `UserMysekaiConvertSlot` rows (`slotId`,
//!   `mysekaiConvertItemId`, `choiceId`, `convertStartAt`,
//!   `onceConvertMinutes`, `convertCount`, `receivedProgress`,
//!   `mysekaiConvertSlotStatus`, and `userMysekaiConvertObtainResources`:
//!   `slotId`, `progress`, `seq`, `resourceType`, `resourceId`, `quantity`).
//!   A slot holds a number of converts of one item; its draws are the obtain
//!   rows of each completion step `progress`.
//! - `userMysekaiConvertItemHistories`: `UserMysekaiConvertItemHistory` rows
//!   (`mysekaiConvertItemId`, `convertCount`, `totalConvertCount`,
//!   `lastConvertStartAt`), for the items whose master row states a
//!   `convertLimit`.
//! - `userMysekaiGamedata.mysekaiConvertFixtureLevel`: the convert machine's
//!   level, which the join seats the slots' count against.
//!
//! **Named policies** (the server's rules are not in the client):
//! - *Slots*: `mysekaiConvertFixtureSlot` states the slot numbers a machine
//!   has and the ones a world pass extends it to; the join seats one slot per
//!   extended number (an inference: the client reads the extended number as
//!   the slot count and locks a slot above the plain number while the user
//!   holds no pass).
//! - *Convert time*: the master `convertMinutes` of the item times the
//!   pass's `mysekaiConvertFixtureConvertMinutesDecreaseRate`, rounded down,
//!   while the user's pass runs. `mysekaiColorfulPass` carries the rate as a
//!   fraction; a rate of 1 or more leaves the minutes as the master states
//!   them (an inference: a master that carried a percentage would need one,
//!   and `GetOnceConvertMinutes` has no such branch).
//! - *Machine level*: `mysekaiConvertFixtureLevels` seats a user without one
//!   at its first row's `level`; an item whose `requiredConvertFixtureLevel`
//!   is above it cannot start.
//! - *Boxes*: `resourceBoxes` is keyed by (`resourceBoxPurpose`, `id`), as
//!   the ids repeat across purposes. A convert spends the
//!   `mysekai_convert_consumption` box `consumeResourceBoxId` names, and a
//!   `resource_box` item draws the `mysekai_convert_obtain` box `externalId`
//!   names. Both purposes count every detail (`resourceQuantity`) and read
//!   them in `seq` order.
//! - *Draws*: the draws are decided when a convert starts (the source's
//!   convert responses carry no rewards; its slots hold the obtain rows). A
//!   `resource_box` item draws every detail of the box
//!   `mysekai_convert_obtain` / `externalId`, in `seq` order: the source turns
//!   the whole box into resources (`ResourceBoxUtility.GetResources(box,
//!   count)`, whose quantity column is `resourceQuantity`), and one convert
//!   is one completion step. A
//!   `mysekai_blueprint` item draws the quantity its
//!   `mysekaiConvertItemObtainMysekaiBlueprints` row states of a random
//!   `mysekaiBlueprints` row with `isObtainedByConvert` the user does not
//!   own, or the `white_blueprint` item for a `white_blueprint` row. A
//!   `mysekai_music_record` item draws a `mysekaiMusicRecords` row the user
//!   does not own. The pick is seeded by the slot, the step, the choice and
//!   the item, so the same document replays to the same draws.
//! - *Convertible count*: an item whose master row states a `convertLimit` is
//!   capped by the limit less its history row's `convertCount`, which a
//!   weekly or monthly refresh resets (`refreshCycle`), and by the resources
//!   its `mysekai_convert_consumption` box `consumeResourceBoxId` asks for
//!   (every detail's `resourceQuantity`, one box's worth per convert, capped
//!   at the master `onceConvertLimit`).
//! - *Start*: the slot must have no converts left; the item must be a master
//!   item, unlocked at the machine level, within its session, affordable from
//!   its consumption box and (on a pass slot) within the pass. The slot then
//!   takes the request's count, spends one box's detail quantities per
//!   convert, and starts at the server clock; the start is drawn and committed
//!   whole, so a draw that fails leaves the document as it was.
//! - *Receive*: the converted count is the slot's elapsed time over its
//!   per-convert minutes, at most its count; the request receives the steps
//!   above the received progress. Each draw is granted within its possession
//!   limit (the material and fixture limits at their first level, an
//!   inference: the source derives the level from the user's MySekai rank),
//!   and the receive stops at the first draw it cannot hold, which it still
//!   reports as obtained; the received progress then moves past every step
//!   received. The limits are read before anything is granted: a server
//!   without `mysekaiMaterialPossessions` or `mysekaiFixturePossessions`
//!   refuses the whole receive by name instead of reading a missing limit as
//!   "does not fit".
//! - *Shorten*: a processing slot's remaining minutes, rounded up, times the
//!   master config `mysekai_convert_time_shorten_consume_jewel_per_minute`
//!   are skipped for `consumeJewelQuantity` crystals. The crystals are the
//!   account's wallet, which the user document does not carry, so the amount
//!   is checked against the slot's price and not charged (an inference); a
//!   `policies.convertShorten.unjewelled` set to true skips the check.
//!
//! **Sections.** The join carries `userMysekaiConvertSlots`,
//! `userMysekaiConvertItemHistories` and `mysekaiConvertItems`; every
//! response carries `mysekaiConvertItems` (a response's section list, never
//! the document), because the client reads the convert items, their
//! `convertLimit` and `onceConvertLimit` from it. A reply's `updated` is the
//! sections that response changed.
//!
//! Native instrument: `MOLY_CONVERT_MOCK_REQUEST=start:slot:item:count[:choice]`,
//! `receive:slot:count` or `shorten:slot:price`; game mode reads no
//! environment variable.

use std::collections::BTreeSet;

use bevy::log::info;
use moly_protocol::api::{
    ConvertPost, SuiteUserReply, UserMysekaiConvertReceiveRequest,
    UserMysekaiConvertReceiveResponse, UserMysekaiConvertShortenRequest,
    UserMysekaiConvertStartRequest,
};
use moly_protocol::json::{
    self, SECTION_BLUEPRINTS, SECTION_CANVASES, SECTION_FIXTURES, SECTION_ITEMS as OWNED_ITEMS,
};
use moly_protocol::rows::{
    ConvertItemHistory, ConvertObtainResource, ConvertSlot, OwnedMusicRecord, SuiteUserSections,
    UserMysekaiBlueprint, UserMysekaiFixture, UserMysekaiItem, UserResource,
    CONVERT_SLOT_COMPLETE, CONVERT_SLOT_PROCESSING,
};
use moly_masters::clock::{CalendarRefresh, CONFIG_REFRESH_HOUR, CONFIG_WEEKLY_REFERENCE};
use serde_json::{json, Map, Value};

use super::delivery::SECTION_MYSEKAI_MATERIALS;
use super::music_play::RECORDS_SECTION;
use super::{ResponseKind, ServerModel, Tables, SECTION_GAMEDATA};

/// `userMysekaiConvertSlots`.
pub(crate) const SECTION_SLOTS: &str = "userMysekaiConvertSlots";
/// `userMysekaiConvertItemHistories`.
pub(crate) const SECTION_HISTORIES: &str = "userMysekaiConvertItemHistories";
/// The convert items a response carries: the master's own rows, which the
/// client builds its item list from (its document keeps none of them).
pub(crate) const SECTION_ITEMS: &str = "mysekaiConvertItems";
/// The document keys this part reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 2] = [SECTION_SLOTS, SECTION_HISTORIES];
/// `policies.convertShorten`.
pub(crate) const POLICY_SHORTEN: &str = "convertShorten";
/// The policy keys this part reads and writes.
pub(crate) const POLICY_KEYS: [&str; 1] = [POLICY_SHORTEN];
const POLICY_UNJEWELLED: &str = "unjewelled";
/// `userMysekaiGamedata.mysekaiConvertFixtureLevel`, written by the panel.
pub(crate) const GAMEDATA_LEVEL: &str = "mysekaiConvertFixtureLevel";

/// `mysekaiConvertItems.mysekaiConvertItemObtainType`.
const OBTAIN_RESOURCE_BOX: &str = "resource_box";
const OBTAIN_BLUEPRINT: &str = "mysekai_blueprint";
const OBTAIN_RECORD: &str = "mysekai_music_record";

/// `MysekaiConvertItemObtainMysekaiBlueprintType`.
const BLUEPRINT_RANDOM: &str = "random";
const BLUEPRINT_CHOICE: &str = "mysekai_fixture_main_genre_choice";
const BLUEPRINT_WHITE: &str = "white_blueprint";

/// `MysekaiConvertItemObtainMysekaiMusicRecordType`.
const RECORD_RANDOM: &str = "random";
const RECORD_CHOICE: &str = "mysekai_music_record_category_choice";

/// `MysekaiMusicTrackType.music`; the record categories exist only for the
/// choice draws' validation (an inference: the draws pick from every master
/// record, because the records master carries no category column here).
const TRACK_MUSIC: &str = "music";

/// `MasterMysekaiConvertFixtureLevelRelease.mysekaiConvertFixtureLevelReleaseType`
/// of the row that releases the shorten button.
const RELEASE_SHORTEN: &str = "convert_time_shorten";

/// `configs.mysekai_convert_time_shorten_consume_jewel_per_minute`: the key
/// the skip price reads.
const CONFIG_JEWEL_PER_MINUTE: &str = "mysekai_convert_time_shorten_consume_jewel_per_minute";

/// The possession limits one receive grants within, read before it hands
/// anything out.
///
/// `MysekaiUserDataUtility.GetMaterialMaxCount` and `GetFixtureMaxCount` read
/// the possession masters; a server without them cannot say whether a
/// resource fits, so the receive refuses by name instead of counting every
/// grant as over the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PossessionLimits {
    material: i32,
    fixture: i32,
}

impl PossessionLimits {
    /// The limits, or the refusal naming the table that is not seated.
    fn read(masters: &Tables<'_>) -> Result<Self, String> {
        let possession = masters.possession();
        let material = possession
            .material_max_count(POSSESSION_LEVEL)
            .ok_or_else(|| missing_master("mysekaiMaterialPossessions", "materials"))?;
        let fixture = possession
            .fixture_max_count(POSSESSION_LEVEL)
            .ok_or_else(|| missing_master("mysekaiFixturePossessions", "fixtures"))?;
        Ok(Self { material, fixture })
    }
}

/// Why a receive refuses when a possession master is not in: the resource it
/// cannot hold within a limit, and the table it lacks.
fn missing_master(table: &str, what: &str) -> String {
    format!(
        "the {table} master (level {POSSESSION_LEVEL}) is not seated: the receive's {what} cannot be checked against the possession limit, so nothing is granted"
    )
}

/// The possession level the material and fixture limits are read at (an
/// inference: the source derives it from the user's MySekai rank, which this
/// server does not derive; the first level stands).
const POSSESSION_LEVEL: i32 = 1;

/// The item kind a master `mysekaiConvertItems` row states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConvertItemKind {
    ResourceBox,
    Blueprint,
    Record,
}

impl ConvertItemKind {
    fn parse(word: &str) -> Option<Self> {
        match word {
            OBTAIN_RESOURCE_BOX => Some(Self::ResourceBox),
            OBTAIN_BLUEPRINT => Some(Self::Blueprint),
            OBTAIN_RECORD => Some(Self::Record),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ResourceBox => OBTAIN_RESOURCE_BOX,
            Self::Blueprint => OBTAIN_BLUEPRINT,
            Self::Record => OBTAIN_RECORD,
        }
    }
}

/// `MysekaiConvertItemRefreshCycle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConvertRefresh {
    None,
    Weekly,
    Monthly,
}

impl ConvertRefresh {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
}

/// One `mysekaiConvertItems` row, the columns this server reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConvertItemMaster {
    pub(crate) id: i32,
    pub(crate) tab_id: i32,
    /// The box of resources a convert spends.
    pub(crate) consume_box_id: i32,
    pub(crate) kind: ConvertItemKind,
    /// The box a `resource_box` item draws from, or the
    /// `mysekaiConvertItemObtainMysekaiBlueprints` /
    /// `mysekaiConvertItemObtainMysekaiMusicRecords` row it names.
    pub(crate) external_id: i32,
    pub(crate) required_level: i32,
    /// The master `convertMinutes`.
    pub(crate) convert_minutes: i32,
    /// 0 when the master row states no `convertLimit`.
    pub(crate) convert_limit: i32,
    pub(crate) once_convert_limit: i32,
    pub(crate) refresh: ConvertRefresh,
    /// The open window (0: none).
    pub(crate) start_at: i64,
    pub(crate) end_at: i64,
}

/// One `resourceBoxes` row: the box's details, `seq` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceBox {
    /// `resourceBoxPurpose`: a box is keyed by (purpose, id), because ids
    /// repeat across purposes (the convert consumption and obtain boxes
    /// share id 10001, for one).
    pub(crate) purpose: String,
    pub(crate) id: i32,
    /// (resourceType, resourceId, resourceLevel, quantity): the master
    /// detail's `resourceType`, `resourceId` (0 when the detail names none),
    /// the resource's source level (0: unset) and `resourceQuantity`.
    pub(crate) details: Vec<(String, i32, i32, i32)>,
}

/// `MysekaiConvertItemObtainType.resource_box`'s consumption box
/// (`resourceBoxPurpose` of the rows a convert spends).
pub(crate) const BOX_CONSUMPTION: &str = "mysekai_convert_consumption";
/// The box a `resource_box` item draws from.
pub(crate) const BOX_OBTAIN: &str = "mysekai_convert_obtain";

/// One `mysekaiConvertItemObtainMysekaiBlueprints` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlueprintObtain {
    pub(crate) id: i32,
    pub(crate) kind: String,
    pub(crate) quantity: i32,
}

/// One `mysekaiConvertItemObtainMysekaiMusicRecords` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordObtain {
    pub(crate) id: i32,
    pub(crate) kind: String,
    pub(crate) quantity: i32,
}

/// One `mysekaiMusicRecordCategories` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordCategory {
    pub(crate) id: i32,
    pub(crate) track_type: String,
}

/// The convert master tables the model reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConvertMasters {
    pub(crate) items: Option<Vec<ConvertItemMaster>>,
    /// `mysekaiConvertFixtureSlot`: (slotNoUpperLimit,
    /// slotNoUpperLimitForMysekaiColorfulPass).
    pub(crate) slots: Option<(i32, i32)>,
    pub(crate) level_releases: Option<i32>,
    pub(crate) levels: Option<Vec<i32>>,
    pub(crate) resource_boxes: Option<Vec<ResourceBox>>,
    pub(crate) blueprint_obtains: Option<Vec<BlueprintObtain>>,
    pub(crate) record_obtains: Option<Vec<RecordObtain>>,
    pub(crate) record_categories: Option<Vec<RecordCategory>>,
    /// `mysekaiColorfulPass.mysekaiConvertFixtureConvertMinutesDecreaseRate` as
    /// the fraction the convert time multiplies by.
    pub(crate) pass_rate: Option<f32>,
}

impl Default for ConvertMasters {
    fn default() -> Self {
        Self {
            items: None,
            slots: None,
            level_releases: None,
            levels: None,
            resource_boxes: None,
            blueprint_obtains: None,
            record_obtains: None,
            record_categories: None,
            pass_rate: None,
        }
    }
}

impl ConvertMasters {
    pub(crate) fn item(&self, id: i32) -> Option<&ConvertItemMaster> {
        self.items.as_ref()?.iter().find(|row| row.id == id)
    }

    /// The box a purpose and an id name.
    pub(crate) fn box_details(&self, purpose: &str, id: i32) -> Option<&ResourceBox> {
        self.resource_boxes
            .as_ref()?
            .iter()
            .find(|row| row.purpose == purpose && row.id == id)
    }

    /// Whether the tables every convert request reads are seated.
    pub(crate) fn seated(&self) -> bool {
        self.items.is_some() && self.slots.is_some()
    }

}

/// `policies.convertShorten`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConvertPolicy {
    /// Whether the skip's price is not checked (the crystals are not
    /// charged either way).
    pub(crate) unjewelled: bool,
}

/// This part of the server document.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConvertDoc {
    /// `userMysekaiConvertSlots`.
    pub(crate) slots: Vec<ConvertSlot>,
    /// `userMysekaiConvertItemHistories`.
    pub(crate) histories: Vec<ConvertItemHistory>,
    /// `policies.convertShorten`.
    pub(crate) policy: ConvertPolicy,
}

impl ConvertDoc {
    /// This part from a schemaVersion 2 document: each key optional (the
    /// empty list when absent), each present one strict.
    pub(crate) fn parse(
        doc: &Map<String, Value>,
        policies: &Map<String, Value>,
    ) -> Result<Self, String> {
        let mut out = Self::default();
        if let Some(value) = doc.get(SECTION_SLOTS) {
            out.slots = json::parse_convert_slots(value)?;
        }
        if let Some(value) = doc.get(SECTION_HISTORIES) {
            out.histories = json::parse_convert_histories(value)?;
        }
        if let Some(value) = policies.get(POLICY_SHORTEN) {
            out.policy = parse_policy(value)?;
        }
        out.check_structure()?;
        Ok(out)
    }

    pub(crate) fn check_structure(&self) -> Result<(), String> {
        let mut slots = BTreeSet::new();
        for (i, row) in self.slots.iter().enumerate() {
            let at = format!("{SECTION_SLOTS}[{i}]");
            if row.slot_id < 0 {
                return Err(format!("{at}.slotId = {} is negative", row.slot_id));
            }
            if !slots.insert(row.slot_id) {
                return Err(format!("{at} repeats slot {}", row.slot_id));
            }
            if row.once_convert_minutes < 0 {
                return Err(format!(
                    "{at}.onceConvertMinutes = {} is negative",
                    row.once_convert_minutes
                ));
            }
            if row.convert_count < 0 {
                return Err(format!("{at}.convertCount = {} is negative", row.convert_count));
            }
            if row.received_progress < 0 || row.received_progress > row.convert_count {
                return Err(format!(
                    "{at}.receivedProgress = {} is outside 0..{}",
                    row.received_progress, row.convert_count
                ));
            }
            if row.mysekai_convert_slot_status != CONVERT_SLOT_PROCESSING
                && row.mysekai_convert_slot_status != CONVERT_SLOT_COMPLETE
            {
                return Err(format!(
                    "{at}.mysekaiConvertSlotStatus = {:?} is neither {CONVERT_SLOT_PROCESSING:?} nor {CONVERT_SLOT_COMPLETE:?}",
                    row.mysekai_convert_slot_status
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (i, row) in self.histories.iter().enumerate() {
            let at = format!("{SECTION_HISTORIES}[{i}]");
            if row.mysekai_convert_item_id < 1 {
                return Err(format!(
                    "{at}.mysekaiConvertItemId = {} is below 1",
                    row.mysekai_convert_item_id
                ));
            }
            if row.convert_count < 0 || row.total_convert_count < row.convert_count {
                return Err(format!(
                    "{at} counts convertCount {} and totalConvertCount {}",
                    row.convert_count, row.total_convert_count
                ));
            }
            if !ids.insert(row.mysekai_convert_item_id) {
                return Err(format!(
                    "{at} repeats convert item {}",
                    row.mysekai_convert_item_id
                ));
            }
        }
        Ok(())
    }

    /// Writes this part's keys into the document object and its policies.
    pub(crate) fn write(&self, doc: &mut Map<String, Value>, policies: &mut Map<String, Value>) {
        doc.insert(SECTION_SLOTS.into(), json::convert_slots_value(&self.slots));
        doc.insert(
            SECTION_HISTORIES.into(),
            json::convert_histories_value(&self.histories),
        );
        policies.insert(
            POLICY_SHORTEN.into(),
            json!({POLICY_UNJEWELLED: self.policy.unjewelled}),
        );
    }
}

fn parse_policy(value: &Value) -> Result<ConvertPolicy, String> {
    let at = format!("policies.{POLICY_SHORTEN}");
    let row = json::object(value, &at)?;
    json::only(row, &[POLICY_UNJEWELLED], &at)?;
    Ok(ConvertPolicy {
        unjewelled: match row.get(POLICY_UNJEWELLED) {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("{at}.{POLICY_UNJEWELLED} is not a boolean"))?,
        },
    })
}

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

/// A master row's string field, refused by name when absent.
fn row_text(row: &Value, key: &str, table: &str) -> Result<String, String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{table} row {:?} has no text {key}", row.get("id")))
}

/// A master row's integer field, 0 when absent.
fn row_int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// A master table's rows, refused by name when it is not an array of rows.
fn table_rows(text: &str, table: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut rows = value
        .as_array()
        .ok_or_else(|| format!("{table} is not an array of rows"))?
        .clone();
    rows.sort_by_key(|row| row["id"].as_i64().unwrap_or(i64::MAX));
    Ok(rows)
}

/// The row of a one-row master table, which upstream publishes as one JSON
/// object (an array's first row is read as well).
fn single_row(text: &str, table: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    match value {
        Value::Object(_) => Ok(value),
        Value::Array(rows) => rows.into_iter().next().ok_or(format!("{table} has no row")),
        _ => Err(format!("{table} is neither a row object nor an array of rows")),
    }
}

/// `mysekaiConvertFixtureSlot`: (slotNoUpperLimit,
/// slotNoUpperLimitForMysekaiColorfulPass).
pub(crate) fn parse_convert_fixture_slot(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    let row = single_row(text, "mysekaiConvertFixtureSlot")?;
    masters.convert_slots = Some((
        super::int32(&row, "slotNoUpperLimit")?,
        super::int32(&row, "slotNoUpperLimitForMysekaiColorfulPass")?,
    ));
    Ok(())
}

/// `mysekaiConvertFixtureLevelReleases`: the machine level of the first row
/// whose release type is `convert_time_shorten`.
pub(crate) fn parse_convert_level_releases(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    let rows = table_rows(text, "mysekaiConvertFixtureLevelReleases")?;
    let row = rows
        .iter()
        .find(|row| row["mysekaiConvertFixtureLevelReleaseType"].as_str() == Some(RELEASE_SHORTEN))
        .ok_or("mysekaiConvertFixtureLevelReleases has no convert_time_shorten row")?;
    masters.convert_level_releases = Some(super::int32(row, "mysekaiConvertFixtureLevel")?);
    Ok(())
}

/// `mysekaiConvertFixtureLevels`: the `level` column, id order.
pub(crate) fn parse_convert_levels(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    moly_assets::json::master::rows(text).and_then(|rows| {
        masters.convert_levels = Some(
            rows.iter()
                .map(|row| super::int32(row, "level"))
                .collect::<Result<Vec<_>, String>>()?,
        );
        Ok(())
    })
}

/// `resourceBoxes`: each box's details (resourceType, resourceId,
/// resourceQuantity), id order, through the shared reader so the server and
/// the client read one set of columns (a detail's level reads 0: the shared
/// reader does not carry it).
pub(crate) fn parse_resource_boxes(text: &str) -> Result<Vec<ResourceBox>, String> {
    Ok(moly_masters::convert::parse_boxes(text)?
        .into_iter()
        .map(|boxed| ResourceBox {
            purpose: boxed.purpose,
            id: boxed.id,
            details: boxed
                .details
                .into_iter()
                .map(|detail| (detail.resource_type, detail.resource_id, 0, detail.quantity))
                .collect(),
        })
        .collect())
}

/// `resourceBoxes` as the server's own join table (a `Masters`, not the
/// shared tables) seats them: into `convert_box_list`, which
/// `Masters::join_parts` moves into the convert tables before any user
/// joins.
pub(crate) fn parse_resource_boxes_only(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    masters.convert_box_list = Some(parse_resource_boxes(text)?);
    Ok(())
}

/// `mysekaiConvertItems`: the convert screen's items, id order. The server
/// reads the table itself rather than through the shared master layer,
/// whose craft resource does not carry the `convertLimit` and
/// `onceConvertLimit` columns these rules spend.
pub(crate) fn parse_convert_items(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    const TABLE: &str = "mysekaiConvertItems";
    let rows = table_rows(text, TABLE)?;
    let items = rows
        .iter()
        .map(|row| {
            let id = super::int32(row, "id")?;
            let kind_word = row_text(row, "mysekaiConvertItemObtainType", TABLE)?;
            let kind = ConvertItemKind::parse(&kind_word).ok_or_else(|| {
                format!("{TABLE} row {id}: mysekaiConvertItemObtainType {kind_word:?} is not known")
            })?;
            let refresh = match row.get("refreshCycle").and_then(Value::as_str) {
                None | Some("none") => ConvertRefresh::None,
                Some("weekly") => ConvertRefresh::Weekly,
                Some("monthly") => ConvertRefresh::Monthly,
                Some(other) => {
                    return Err(format!(
                        "{TABLE} row {id}: refreshCycle {other:?} is not known"
                    ))
                }
            };
            Ok(ConvertItemMaster {
                id,
                tab_id: super::int32(row, "mysekaiConvertItemTabId")?,
                consume_box_id: super::int32(row, "consumeResourceBoxId")?,
                kind,
                external_id: super::int32(row, "externalId")?,
                required_level: super::int32(row, "requiredConvertFixtureLevel")?,
                convert_minutes: super::int32(row, "convertMinutes")?,
                convert_limit: i32::try_from(row_int(row, "convertLimit")).map_err(|_| {
                    format!("{TABLE} row {id}: convertLimit is too large")
                })?,
                once_convert_limit: super::int32(row, "onceConvertLimit")?,
                refresh,
                start_at: row_int(row, "startAt"),
                end_at: row_int(row, "endAt"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.convert_items = Some(items);
    Ok(())
}

/// `mysekaiConvertItemObtainMysekaiBlueprints`, id order.
pub(crate) fn parse_convert_blueprint_obtains(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    const TABLE: &str = "mysekaiConvertItemObtainMysekaiBlueprints";
    let rows = table_rows(text, TABLE)?;
    let obtains = rows
        .iter()
        .map(|row| {
            let kind = row_text(row, "mysekaiConvertItemObtainMysekaiBlueprintType", TABLE)?;
            if !matches!(kind.as_str(), BLUEPRINT_RANDOM | BLUEPRINT_CHOICE | BLUEPRINT_WHITE) {
                return Err(format!(
                    "{TABLE} row {:?}: type {kind:?} is not known",
                    row.get("id")
                ));
            }
            Ok(BlueprintObtain {
                id: super::int32(row, "id")?,
                kind,
                quantity: row_int(row, "quantity").max(1) as i32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.convert_blueprint_obtains = Some(obtains);
    Ok(())
}

/// `mysekaiConvertItemObtainMysekaiMusicRecords`, id order.
pub(crate) fn parse_convert_record_obtains(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    const TABLE: &str = "mysekaiConvertItemObtainMysekaiMusicRecords";
    let rows = table_rows(text, TABLE)?;
    let obtains = rows
        .iter()
        .map(|row| {
            let kind = row_text(row, "mysekaiConvertItemObtainMysekaiMusicRecordType", TABLE)?;
            if !matches!(kind.as_str(), RECORD_RANDOM | RECORD_CHOICE) {
                return Err(format!(
                    "{TABLE} row {:?}: type {kind:?} is not known",
                    row.get("id")
                ));
            }
            Ok(RecordObtain {
                id: super::int32(row, "id")?,
                kind,
                quantity: row_int(row, "quantity").max(1) as i32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.convert_record_obtains = Some(obtains);
    Ok(())
}

/// `mysekaiMusicRecordCategories`: (id, mysekaiMusicTrackType), id order.
pub(crate) fn parse_convert_record_categories(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    const TABLE: &str = "mysekaiMusicRecordCategories";
    let rows = table_rows(text, TABLE)?;
    let categories = rows
        .iter()
        .map(|row| {
            Ok(RecordCategory {
                id: super::int32(row, "id")?,
                track_type: row_text(row, "mysekaiMusicTrackType", TABLE)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    masters.convert_record_categories = Some(categories);
    Ok(())
}

/// `mysekaiColorfulPass`: `mysekaiConvertFixtureConvertMinutesDecreaseRate` of the
/// first row, as the fraction the convert time multiplies by.
pub(crate) fn parse_convert_pass_rate(
    text: &str,
    masters: &mut super::Masters,
) -> Result<(), String> {
    let row = single_row(text, "mysekaiColorfulPass")?;
    let raw = row
        .get("mysekaiConvertFixtureConvertMinutesDecreaseRate")
        .and_then(Value::as_f64)
        .ok_or(
            "mysekaiColorfulPass has no mysekaiConvertFixtureConvertMinutesDecreaseRate",
        )? as f32;
    if !raw.is_finite() || raw < 0.0 {
        return Err(format!(
            "mysekaiColorfulPass.mysekaiConvertFixtureConvertMinutesDecreaseRate = {raw} is negative"
        ));
    }
    // The rate is the fraction `GetOnceConvertMinutes` multiplies by; 1 or
    // more leaves the minutes as the master states them (the source has no
    // percentage branch).
    masters.convert_pass_rate = Some(raw);
    Ok(())
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

/// `MysekaiUtility.HasMysekaiColorfulPass`: a pass whose `expiredAt` is after
/// the server clock.
fn has_pass(doc: &super::document::ServerDocument, now: i64) -> bool {
    doc.colorful_pass.is_some_and(|pass| now < pass.expired_at)
}

/// The minutes one convert of the item takes: the master minutes, decreased
/// by the pass's rate while the pass runs.
fn once_minutes(item: &ConvertItemMaster, convert: &ConvertMasters, pass: bool) -> i32 {
    if pass {
        if let Some(rate) = convert.pass_rate {
            if rate > 0.0 && rate <= 1.0 {
                let minutes = item.convert_minutes as f32 * rate;
                // f32 as i32 saturates, so a huge value cannot wrap.
                return minutes.floor() as i32;
            }
        }
    }
    item.convert_minutes
}

/// Whether the slot number lies above the machine's plain slots (a pass
/// slot).
fn is_pass_slot(convert: &ConvertMasters, slot_id: i32) -> bool {
    convert.slots.is_some_and(|(limit, _)| slot_id >= limit)
}

/// The completions the item has used since its last refresh (the source's
/// `SetupConvertibleCount`: a weekly or monthly row whose last start crossed
/// the refresh reads 0).
fn used_convert_count(
    doc: &ConvertDoc,
    item: &ConvertItemMaster,
    now: i64,
    masters: &Tables<'_>,
) -> Result<i32, String> {
    let Some(history) = doc
        .histories
        .iter()
        .find(|row| row.mysekai_convert_item_id == item.id)
    else {
        return Ok(0);
    };
    if is_refreshed(item.refresh, history.last_convert_start_at, now, masters)? {
        return Ok(0);
    }
    Ok(history.convert_count)
}

/// `MysekaiUtility.HasCrossedWeeklyRefresh` / `HasCrossedMonthlyRefresh`,
/// on the calendar the master `configs` name and the clock's local offset.
fn is_refreshed(
    refresh: ConvertRefresh,
    last_start_at: i64,
    now: i64,
    masters: &Tables<'_>,
) -> Result<bool, String> {
    if last_start_at <= 0 || refresh == ConvertRefresh::None {
        return Ok(false);
    }
    let configs = masters
        .configs
        .as_ref()
        .ok_or("configs is absent: the convert refresh has no calendar")?;
    let calendar = CalendarRefresh::from_values(
        configs.get(CONFIG_REFRESH_HOUR).map(String::as_str),
        configs.get(CONFIG_WEEKLY_REFERENCE).map(String::as_str),
    )?;
    let offset = super::clock::device_utc_offset_ms(now);
    Ok(match refresh {
        ConvertRefresh::None => false,
        ConvertRefresh::Weekly => calendar.has_crossed_weekly(last_start_at, now, offset),
        ConvertRefresh::Monthly => calendar.has_crossed_monthly(last_start_at, now, offset),
    })
}

/// A small deterministic generator for the draws (SplitMix64).
#[derive(Clone, Copy, Debug)]
struct Seeded(u64);

impl Seeded {
    fn pick(&mut self, bound: usize) -> usize {
        if bound <= 1 {
            return 0;
        }
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z % bound as u64) as usize
    }

    fn seed(parts: &[i64]) -> Self {
        let mut state = 0x243F_6A88_85A3_08D3u64;
        for part in parts {
            state = state
                .wrapping_mul(0x0000_0100_0000_01B3)
                .wrapping_add(*part as u64);
            state = state.rotate_left(17) ^ (state >> 7);
        }
        Self(state)
    }
}

/// One drawn resource.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Drawn {
    resource_type: String,
    resource_id: i32,
    quantity: i32,
}

impl ServerModel {
    /// The convert machine level, seated at the join from the first master
    /// level while the document states none.
    pub(super) fn seat_convert_level(&mut self, masters: &Tables<'_>) -> bool {
        if self.doc.gamedata.mysekai_convert_fixture_level.is_some() {
            return false;
        }
        let first = masters
            .convert
            .as_ref()
            .and_then(|convert| convert.levels.as_ref())
            .and_then(|levels| levels.first().copied());
        let Some(first) = first else {
            self.push_error(
                "userMysekaiGamedata.mysekaiConvertFixtureLevel is not seated: the mysekaiConvertFixtureLevels master is absent".into(),
            );
            return false;
        };
        self.doc.gamedata.mysekai_convert_fixture_level = Some(first);
        info!("[server] mysekaiConvertFixtureLevel {first} (the first master level)");
        true
    }

    /// The join's convert slots: one per master slot number for a user whose
    /// slots hold no converts.
    pub(super) fn seat_convert_slots(&mut self, masters: &Tables<'_>) -> bool {
        let Some((_, count)) = masters.convert.as_ref().and_then(|convert| convert.slots) else {
            return false;
        };
        if self.doc.convert.slots.iter().any(|slot| slot.convert_count > 0) {
            return false;
        }
        // The idle slot: nothing converting, nothing received.
        let idle: Vec<ConvertSlot> = (0..count.max(0))
            .map(|id| ConvertSlot {
                slot_id: id,
                mysekai_convert_item_id: 0,
                choice_id: 0,
                convert_start_at: 0,
                once_convert_minutes: 0,
                convert_count: 0,
                received_progress: 0,
                mysekai_convert_slot_status: CONVERT_SLOT_COMPLETE.to_owned(),
                obtain_resources: Vec::new(),
            })
            .collect();
        if idle == self.doc.convert.slots {
            return false;
        }
        info!(
            "[server] userMysekaiConvertSlots: {count} slot(s) seated from mysekaiConvertFixtureSlot"
        );
        self.doc.convert.slots = idle;
        true
    }

    /// A processing slot whose converts have all completed by the server
    /// clock takes the complete status (the document's own record follows the
    /// clock, as the client's copy does).
    fn settle_slots(&mut self, now: i64) -> bool {
        let mut changed = false;
        for slot in &mut self.doc.convert.slots {
            if slot.is_processing() && slot.end_at() <= now {
                *slot = slot.completed();
                changed = true;
            }
        }
        if changed {
            self.commit();
        }
        changed
    }

    /// The converted steps the possession limits hold, from the received
    /// progress (the source's receivable count, used by a client that asks
    /// the receive's count by itself).
    #[allow(dead_code)] // The client derives its own count; the panel may read it.
    pub(crate) fn convert_receivable(
        &self,
        slot_id: i32,
        now: i64,
        masters: &Tables<'_>,
    ) -> i32 {
        let Some(slot) = self.doc.convert.slots.iter().find(|slot| slot.slot_id == slot_id) else {
            return 0;
        };
        self.receivable_steps(slot, now, masters)
    }

    /// `MysekaiConvertUtility.GetConvertReceivableCount`: the completion
    /// steps above the received progress that the possession limits hold (0
    /// when the limits are not in, the count the receive itself refuses).
    fn receivable_steps(&self, slot: &ConvertSlot, now: i64, masters: &Tables<'_>) -> i32 {
        let Ok(limits) = PossessionLimits::read(masters) else {
            return 0;
        };
        let converted = slot.completed_count(now);
        let mut count = 0;
        for progress in (slot.received_progress + 1)..=converted {
            let holds = slot
                .obtain_resources
                .iter()
                .filter(|row| row.progress == progress)
                .all(|row| self.resource_fits(&row.to_resource(), &limits, masters));
            if !holds {
                break;
            }
            count += 1;
        }
        count
    }

    /// Whether one resource fits its possession limit (the receivable preview;
    /// the limits are [`PossessionLimits`]).
    fn resource_fits(
        &self,
        resource: &UserResource,
        limits: &PossessionLimits,
        masters: &Tables<'_>,
    ) -> bool {
        match resource.resource_type.as_str() {
            "mysekai_material" => {
                let possession = self.material_possession(masters).map_or(0, |row| row.quantity);
                possession.saturating_add(resource.quantity) <= limits.material
            }
            "mysekai_fixture" => {
                let total: i32 = self.doc.inventory.fixtures.iter().map(|row| row.quantity).sum();
                total.saturating_add(resource.quantity) <= limits.fixture
            }
            // The other tables have no possession limit here.
            _ => true,
        }
    }

    /// The masters a convert request reads.
    fn convert_masters<'a>(
        &self,
        masters: &'a Tables<'_>,
    ) -> Result<&'a ConvertMasters, String> {
        let convert = masters.convert.as_ref();
        let seated = convert.is_some_and(ConvertMasters::seated);
        match convert.filter(|_| seated) {
            Some(convert) => Ok(convert),
            None => Err(
                "the convert masters are not seated: mysekaiConvertItems and mysekaiConvertFixtureSlot are needed".into(),
            ),
        }
    }

    /// One convert request, answered: `Err` refuses without a change.
    pub(crate) fn convert_reply(
        &mut self,
        slot_id: i32,
        post: ConvertPost,
        masters: &Tables<'_>,
    ) -> Result<moly_protocol::Reply, String> {
        let now = self.now_ms();
        self.settle_slots(now);
        self.convert_masters(masters)?;
        match post {
            ConvertPost::Start(request) => {
                let changed = self.convert_start(slot_id, &request, now, masters)?;
                self.respond(ResponseKind::ConvertStart, false, &changed, masters);
                let updated = self.convert_updated(&changed, masters);
                Ok(moly_protocol::Reply::Convert(SuiteUserReply {
                    success: true,
                    refusal: None,
                    updated,
                }))
            }
            ConvertPost::Receive(request) => {
                let (resources, changed) = self.convert_receive(slot_id, &request, now, masters)?;
                self.respond(ResponseKind::ConvertReceive, false, &changed, masters);
                let updated = self.convert_updated(&changed, masters);
                Ok(moly_protocol::Reply::ConvertReceive(
                    UserMysekaiConvertReceiveResponse {
                        obtained_resources: resources,
                        updated,
                    },
                ))
            }
            ConvertPost::Shorten(request) => {
                let changed = self.convert_shorten(slot_id, &request, now, masters)?;
                self.respond(ResponseKind::ConvertShorten, false, &changed, masters);
                let updated = self.convert_updated(&changed, masters);
                Ok(moly_protocol::Reply::Convert(SuiteUserReply {
                    success: true,
                    refusal: None,
                    updated,
                }))
            }
        }
    }

    /// Whether the masters the draws read are all seated.
    fn convert_draw_masters(&self, masters: &Tables<'_>) -> Result<(), String> {
        let convert = masters
            .convert
            .as_ref()
            .ok_or("the convert masters are not seated")?;
        if convert.resource_boxes.is_none() {
            return Err("resourceBoxes is not seated: a convert's rewards cannot be drawn".into());
        }
        Ok(())
    }

    /// The resources one convert of the item spends: every detail of the
    /// `mysekai_convert_consumption` box `consumeResourceBoxId` names, one
    /// box's worth a convert (`GetConsumedResourceList` over
    /// `ResourceBoxUtility.GetResources(consumeBox, convertCount)`).
    fn convert_consumption(
        &self,
        item: &ConvertItemMaster,
        masters: &Tables<'_>,
    ) -> Result<Vec<(String, i32, i32)>, String> {
        let convert = masters
            .convert
            .as_ref()
            .ok_or("the convert masters are not seated")?;
        let Some(resource_box) = convert.box_details(BOX_CONSUMPTION, item.consume_box_id) else {
            return Err(format!(
                "resourceBoxes does not hold the {BOX_CONSUMPTION} box {}, which convert item {} spends",
                item.consume_box_id, item.id
            ));
        };
        Ok(resource_box
            .details
            .iter()
            .map(|(kind, id, _, quantity)| (kind.clone(), *id, *quantity))
            .collect())
    }

    /// The user's quantity of one resource; a type the owned tables do not
    /// carry is not limited (`i32::MAX`).
    fn held_count(&self, kind: &str, id: i32) -> i32 {
        match kind {
            "mysekai_material" => self
                .doc
                .delivery
                .mysekai_materials
                .get(&id)
                .copied()
                .unwrap_or(0),
            "mysekai_fixture" => self
                .doc
                .inventory
                .fixtures
                .iter()
                .filter(|row| row.mysekai_fixture_id == id)
                .map(|row| row.quantity)
                .sum(),
            "mysekai_canvas" => self
                .doc
                .inventory
                .canvases
                .iter()
                .filter(|row| row.mysekai_fixture_id == id)
                .map(|row| row.quantity)
                .sum(),
            "mysekai_blueprint" => i32::from(
                self.doc
                    .inventory
                    .blueprints
                    .iter()
                    .any(|row| row.mysekai_blueprint_id == id),
            ),
            "mysekai_item" => self
                .doc
                .inventory
                .items
                .iter()
                .find(|row| row.mysekai_item_id == id)
                .map_or(0, |row| row.quantity),
            "mysekai_tool" => self
                .doc
                .tools
                .iter()
                .flatten()
                .find(|row| row.mysekai_tool_id == id)
                .map_or(0, |row| row.quantity),
            "mysekai_music_record" => i32::from(
                self.doc
                    .music_records
                    .iter()
                    .any(|row| row.mysekai_music_record_id == id),
            ),
            _ => i32::MAX,
        }
    }

    /// How many converts the consumption box pays for.
    fn affordable_converts(&self, need: &[(String, i32, i32)]) -> i32 {
        let mut fewest = i32::MAX;
        for (kind, id, each) in need {
            if *each <= 0 {
                continue;
            }
            fewest = fewest.min(self.held_count(kind, *id) / each);
        }
        fewest.max(0)
    }

    /// The number of converts the item allows now on a pass slot (the
    /// source's `GetOnceConvertibleCount`: capped at what the pass's
    /// remaining minutes hold).
    fn pass_slot_capacity(
        &self,
        slot_id: i32,
        item: &ConvertItemMaster,
        now: i64,
        masters: &Tables<'_>,
    ) -> Result<i32, String> {
        let convert = self.convert_masters(masters)?;
        let once = self.item_once_convertible(item, now, masters)?;
        if !is_pass_slot(convert, slot_id) {
            return Ok(once);
        }
        if !has_pass(&self.doc, now) {
            return Ok(0);
        }
        let remaining_minutes = self
            .doc
            .colorful_pass
            .map_or(0, |pass| (pass.expired_at - now).max(0) / 60_000);
        let each = i64::from(item.convert_minutes.max(1));
        let by_pass = (remaining_minutes / each).clamp(0, i64::from(i32::MAX)) as i32;
        Ok(by_pass.clamp(1, once))
    }

    /// The item's converts allowed now, by its limit and its consumption box
    /// (the source's `OnceConvertibleCount`).
    fn item_once_convertible(
        &self,
        item: &ConvertItemMaster,
        now: i64,
        masters: &Tables<'_>,
    ) -> Result<i32, String> {
        let limited = item.convert_limit > 0;
        let left = if limited {
            (item.convert_limit - used_convert_count(&self.doc.convert, item, now, masters)?).max(0)
        } else {
            i32::MAX
        };
        let need = self.convert_consumption(item, masters)?;
        let affordable = self.affordable_converts(&need);
        let by_count = if limited { left.min(affordable) } else { affordable };
        let once = if limited {
            item.once_convert_limit.min(by_count)
        } else {
            item.once_convert_limit
        };
        Ok(once.max(1))
    }

    /// Draws one completion step of the slot: the resources it produces.
    fn draw(
        &self,
        slot_id: i32,
        progress: i32,
        item: &ConvertItemMaster,
        choice_id: i32,
        masters: &Tables<'_>,
    ) -> Result<Vec<Drawn>, String> {
        let convert = masters
            .convert
            .as_ref()
            .ok_or("the convert masters are not seated")?;
        self.convert_draw_masters(masters)?;
        match item.kind {
            // `ResourceBoxUtility.GetResources(box, 1)`: every detail of the
            // `mysekai_convert_obtain` box `externalId` names, in `seq`
            // order.
            ConvertItemKind::ResourceBox => {
                let resource_box =
                    convert
                        .box_details(BOX_OBTAIN, item.external_id)
                        .ok_or_else(|| {
                            format!(
                                "resourceBoxes does not hold the {BOX_OBTAIN} box {}, which convert item {} draws from",
                                item.external_id, item.id
                            )
                        })?;
                if resource_box.details.is_empty() {
                    return Err(format!(
                        "resourceBoxes {BOX_OBTAIN} {} has no detail: convert item {} has nothing to draw",
                        item.external_id, item.id
                    ));
                }
                Ok(resource_box
                    .details
                    .iter()
                    .map(|(kind, id, _, quantity)| Drawn {
                        resource_type: kind.clone(),
                        resource_id: *id,
                        quantity: *quantity,
                    })
                    .collect())
            }
            ConvertItemKind::Blueprint => {
                let obtains = convert.blueprint_obtains.as_ref().ok_or(
                    "mysekaiConvertItemObtainMysekaiBlueprints is absent: a blueprint draw cannot be made",
                )?;
                let obtain = obtains
                    .iter()
                    .find(|row| row.id == item.external_id)
                    .ok_or_else(|| {
                        format!(
                            "convert item {} names obtain row {}, which mysekaiConvertItemObtainMysekaiBlueprints does not hold",
                            item.id, item.external_id
                        )
                    })?;
                if obtain.kind == BLUEPRINT_WHITE {
                    let white = masters
                        .craft()
                        .white_blueprint_item
                        .flatten()
                        .ok_or("the items master has no white_blueprint item")?;
                    // `MysekaiConvertBlueprintItemModel.Setup`: a white
                    // blueprint convert yields the obtain row's quantity.
                    return Ok(vec![Drawn {
                        resource_type: "mysekai_item".to_owned(),
                        resource_id: white,
                        quantity: obtain.quantity,
                    }]);
                }
                let blueprints = masters.craft().blueprints.as_ref().ok_or(
                    "the blueprint master (mysekai-blueprints.json) is absent: a blueprint draw cannot be made",
                )?;
                let owned: BTreeSet<i32> = self
                    .doc
                    .inventory
                    .blueprints
                    .iter()
                    .map(|row| row.mysekai_blueprint_id)
                    .collect();
                let mut pool: Vec<i32> = blueprints
                    .values()
                    .filter(|row| row.is_obtained_by_convert && !owned.contains(&row.id))
                    .map(|row| row.id)
                    .collect();
                pool.sort_unstable();
                if pool.is_empty() {
                    return Err(format!(
                        "no master blueprint with isObtainedByConvert is left for convert item {}",
                        item.id
                    ));
                }
                let mut seed = Seeded::seed(&[
                    i64::from(slot_id),
                    i64::from(progress),
                    i64::from(choice_id),
                    i64::from(item.id),
                ]);
                let pick = pool[seed.pick(pool.len())];
                Ok(vec![Drawn {
                    resource_type: "mysekai_blueprint".to_owned(),
                    resource_id: pick,
                    quantity: obtain.quantity,
                }])
            }
            ConvertItemKind::Record => {
                let obtains = convert.record_obtains.as_ref().ok_or(
                    "mysekaiConvertItemObtainMysekaiMusicRecords is absent: a record draw cannot be made",
                )?;
                let obtain = obtains
                    .iter()
                    .find(|row| row.id == item.external_id)
                    .ok_or_else(|| {
                        format!(
                            "convert item {} names obtain row {}, which mysekaiConvertItemObtainMysekaiMusicRecords does not hold",
                            item.id, item.external_id
                        )
                    })?;
                if obtain.kind == RECORD_CHOICE {
                    let categories = convert.record_categories.as_ref().ok_or(
                        "mysekaiMusicRecordCategories is absent: a record choice cannot be read",
                    )?;
                    if !categories
                        .iter()
                        .any(|row| row.id == choice_id && row.track_type == TRACK_MUSIC)
                    {
                        return Err(format!(
                            "choiceId {choice_id} is not a master music record category"
                        ));
                    }
                }
                let records = masters.music_records.as_deref().ok_or(
                    "mysekaiMusicRecords is absent: a record draw cannot be made",
                )?;
                let owned: BTreeSet<i32> = self
                    .doc
                    .music_records
                    .iter()
                    .map(|row| row.mysekai_music_record_id)
                    .collect();
                let mut pool: Vec<i32> = records
                    .iter()
                    .copied()
                    .filter(|id| !owned.contains(id))
                    .collect();
                pool.sort_unstable();
                if pool.is_empty() {
                    return Err(format!(
                        "every master music record is owned: convert item {} has nothing to draw",
                        item.id
                    ));
                }
                let mut seed = Seeded::seed(&[
                    i64::from(slot_id),
                    i64::from(progress),
                    i64::from(choice_id),
                    i64::from(item.id),
                ]);
                let pick = pool[seed.pick(pool.len())];
                Ok(vec![Drawn {
                    resource_type: "mysekai_music_record".to_owned(),
                    resource_id: pick,
                    quantity: obtain.quantity,
                }])
            }
        }
    }

    /// `PostUserMysekaiConvertStartApi`.
    fn convert_start(
        &mut self,
        slot_id: i32,
        request: &UserMysekaiConvertStartRequest,
        now: i64,
        masters: &Tables<'_>,
    ) -> Result<Vec<&'static str>, String> {
        // A start is drawn and answered, never half applied: the model is
        // worked through on a clone whose document is put back only when
        // every draw, spend and history row succeeds. The reads below (the
        // affordability, the pass's room, the draws, the spend) are the
        // model's, so they take the clone and see its document.
        let mut next = self.clone();
        let pass = has_pass(&next.doc, now);
        let (item, each, need, pass_slot) = {
            let convert = self.convert_masters(masters)?;
            let extended = convert.slots.map_or(0, |(_, extended)| extended);
            if slot_id < 0 || slot_id >= extended {
                return Err(format!(
                    "slot {slot_id} is not one of the machine's {extended} slots (mysekaiConvertFixtureSlot)"
                ));
            }
            if is_pass_slot(convert, slot_id) && !pass {
                return Err(format!(
                    "slot {slot_id} is extended by the world pass and the user holds none"
                ));
            }
            let item = *convert.item(request.mysekai_convert_item_id).ok_or_else(|| {
                format!(
                    "mysekaiConvertItemId {} is not a master convert item",
                    request.mysekai_convert_item_id
                )
            })?;
            let each = once_minutes(&item, convert, pass);
            let need = self.convert_consumption(&item, masters)?;
            (item, each, need, is_pass_slot(convert, slot_id))
        };
        if request.convert_count < 1 {
            return Err(format!("convertCount {} is below 1", request.convert_count));
        }
        if request.convert_count > item.once_convert_limit.max(1) {
            return Err(format!(
                "convertCount {} is above the item's onceConvertLimit {}",
                request.convert_count, item.once_convert_limit
            ));
        }
        let level = next
            .doc
            .gamedata
            .mysekai_convert_fixture_level
            .unwrap_or(0);
        if item.required_level > level {
            return Err(format!(
                "convert item {} needs machine level {}, and the user's is {level}",
                item.id, item.required_level
            ));
        }
        if item.start_at > 0 || item.end_at > 0 {
            let open = if item.end_at == 0 {
                item.start_at != 0 && item.start_at <= now
            } else {
                item.start_at <= now && now < item.end_at
            };
            if !open {
                return Err(format!(
                    "convert item {} is outside its session {}..{} at the server clock {now}",
                    item.id, item.start_at, item.end_at
                ));
            }
        }
        let limited = item.convert_limit > 0;
        let capacity = if limited {
            let used = used_convert_count(&next.doc.convert, &item, now, masters)?;
            Some((item.convert_limit - used).max(0))
        } else {
            None
        };
        if let Some(left) = capacity {
            if request.convert_count > left {
                return Err(format!(
                    "convert item {} has {left} convert(s) left of its convertLimit {}",
                    item.id, item.convert_limit
                ));
            }
        }
        let affordable = next.affordable_converts(&need);
        if request.convert_count > affordable {
            return Err(format!(
                "the consumption box {} pays for {affordable} convert(s) of item {}",
                item.consume_box_id, item.id
            ));
        }
        if pass_slot {
            let allowed = next.pass_slot_capacity(slot_id, &item, now, masters)?;
            if allowed < request.convert_count {
                return Err(format!(
                    "the world pass ends before {} converts of item {}; it allows {allowed}",
                    request.convert_count, item.id
                ));
            }
        }
        if each < 1 {
            return Err(format!(
                "convert item {} takes less than a minute and cannot be placed on the clock",
                item.id
            ));
        }
        // Check the draws before anything changes.
        self.convert_draw_masters(masters)?;
        let current = next
            .doc
            .convert
            .slots
            .iter()
            .find(|slot| slot.slot_id == slot_id)
            .cloned()
            .ok_or_else(|| format!("userMysekaiConvertSlots holds no slot {slot_id}"))?;
        if current.is_processing() && current.completed_count(now) < current.convert_count {
            return Err(format!(
                "slot {slot_id} is still converting {} of item {}",
                current.convert_count - current.completed_count(now),
                current.mysekai_convert_item_id
            ));
        }
        // One completion step per convert, each step's resources numbered in
        // the order they were drawn (`seq`). The obtain box is a different
        // row from the consumption box (they are keyed by purpose and id), so
        // the spend below never takes from what a step draws.
        let mut obtain = Vec::new();
        for offset in 0..request.convert_count {
            let progress = current.received_progress + offset + 1;
            let drawn = next.draw(slot_id, progress, &item, request.choice_id, masters)?;
            for (seq, resource) in drawn.into_iter().enumerate() {
                obtain.push(ConvertObtainResource {
                    slot_id,
                    progress,
                    seq: seq as i32,
                    resource_type: resource.resource_type,
                    resource_id: resource.resource_id,
                    quantity: resource.quantity,
                });
            }
        }
        // Every check passed: spend, start, count the history. The cost is
        // one consumption box's worth per convert.
        for (kind, id, quantity) in &need {
            next.spend_held(kind, *id, quantity.saturating_mul(request.convert_count));
        }
        let started = ConvertSlot {
            slot_id,
            mysekai_convert_item_id: item.id,
            choice_id: request.choice_id,
            convert_start_at: now,
            once_convert_minutes: each,
            convert_count: request.convert_count,
            received_progress: current.received_progress,
            mysekai_convert_slot_status: CONVERT_SLOT_PROCESSING.to_owned(),
            obtain_resources: obtain,
        };
        if let Some(slot) = next
            .doc
            .convert
            .slots
            .iter_mut()
            .find(|slot| slot.slot_id == slot_id)
        {
            *slot = started;
        }
        if limited {
            let used = used_convert_count(&next.doc.convert, &item, now, masters)?;
            let histories = &mut next.doc.convert.histories;
            match histories
                .iter_mut()
                .find(|row| row.mysekai_convert_item_id == item.id)
            {
                Some(row) => {
                    row.convert_count = used.max(0) + request.convert_count;
                    row.total_convert_count += request.convert_count;
                    row.last_convert_start_at = now;
                }
                None => histories.push(ConvertItemHistory {
                    mysekai_convert_item_id: item.id,
                    convert_count: request.convert_count,
                    total_convert_count: request.convert_count,
                    last_convert_start_at: now,
                }),
            }
        }
        self.doc = next.doc;
        self.commit();
        info!(
            "[server] PostUserMysekaiConvertStartApi: slot {slot_id} converts {} x item {} (choice {}), {each} minute(s) each from {now}",
            request.convert_count, item.id, request.choice_id
        );
        let mut changed = vec![SECTION_SLOTS];
        if limited {
            changed.push(SECTION_HISTORIES);
        }
        self.mark_spent_sections(&need, &mut changed);
        Ok(changed)
    }

    /// The sections the spent resources belong to.
    fn mark_spent_sections(&self, need: &[(String, i32, i32)], changed: &mut Vec<&'static str>) {
        for (kind, _, _) in need {
            let section = match kind.as_str() {
                "mysekai_material" => Some(SECTION_MYSEKAI_MATERIALS),
                "mysekai_fixture" => Some(SECTION_FIXTURES),
                "mysekai_canvas" => Some(SECTION_CANVASES),
                "mysekai_blueprint" => Some(SECTION_BLUEPRINTS),
                "mysekai_item" => Some(OWNED_ITEMS),
                "mysekai_tool" => Some(super::tools::SECTION),
                _ => None,
            };
            if let Some(section) = section {
                if !changed.contains(&section) {
                    changed.push(section);
                }
            }
        }
    }

    /// Spends one resource of a consumption box from the owned tables.
    fn spend_held(&mut self, kind: &str, id: i32, quantity: i32) {
        match kind {
            "mysekai_material" => {
                if let Some(row) = self.doc.delivery.mysekai_materials.get_mut(&id) {
                    *row -= quantity;
                }
                self.doc
                    .delivery
                    .mysekai_materials
                    .retain(|_, quantity| *quantity > 0);
            }
            "mysekai_fixture" => {
                let mut left = quantity;
                let fixtures = &mut self.doc.inventory.fixtures;
                for row in fixtures.iter_mut().filter(|row| row.mysekai_fixture_id == id) {
                    let take = row.quantity.min(left);
                    row.quantity -= take;
                    left -= take;
                    if left == 0 {
                        break;
                    }
                }
                fixtures.retain(|row| row.quantity > 0);
            }
            "mysekai_item" => {
                let items = &mut self.doc.inventory.items;
                if let Some(row) = items.iter_mut().find(|row| row.mysekai_item_id == id) {
                    row.quantity = (row.quantity - quantity).max(0);
                }
                items.retain(|row| row.quantity > 0);
            }
            "mysekai_tool" => {
                if let Some(rows) = &mut self.doc.tools {
                    if let Some(row) = rows.iter_mut().find(|row| row.mysekai_tool_id == id) {
                        row.quantity = (row.quantity - quantity).max(0);
                    }
                    rows.retain(|row| row.quantity > 0);
                }
            }
            // A type the consumption boxes of this master do not name: it
            // stays owned.
            _ => {}
        }
    }

    /// The tables a reply's `updated` carries: every section the response
    /// carried, whole, as the client merges them.
    fn convert_updated(&mut self, changed: &[&'static str], masters: &Tables<'_>) -> SuiteUserSections {
        let sections: Vec<String> = changed.iter().map(|s| (*s).to_owned()).collect();
        let has = |name: &str| sections.iter().any(|section| section == name);
        let mut updated = self.inventory_update(&sections, masters);
        if has(SECTION_MYSEKAI_MATERIALS) {
            updated.user_mysekai_materials = Some(self.material_rows());
            updated.user_mysekai_material_possession = self.material_possession(masters);
        }
        if has(RECORDS_SECTION) {
            updated.user_mysekai_music_records = Some(self.doc.music_records.clone());
        }
        if has(super::tools::SECTION) {
            updated.user_mysekai_tools = Some(self.doc.tools.clone().unwrap_or_default());
        }
        if has(SECTION_SLOTS) {
            updated.user_mysekai_convert_slots = Some(self.doc.convert.slots.clone());
        }
        if has(SECTION_HISTORIES) {
            updated.user_mysekai_convert_item_histories = Some(self.doc.convert.histories.clone());
        }
        updated
    }

    /// The master's convert item rows as a response carries them.
    pub(super) fn convert_items(&self, masters: &Tables<'_>) -> Vec<moly_protocol::rows::UserMysekaiConvertItem> {
        masters
            .convert
            .as_ref()
            .and_then(|convert| convert.items.as_ref())
            .map(|items| {
                items
                    .iter()
                    .map(|item| moly_protocol::rows::UserMysekaiConvertItem {
                        mysekai_convert_item_id: item.id,
                        obtain_type: item.kind.name().to_owned(),
                        external_id: item.external_id,
                        required_convert_fixture_level: item.required_level,
                        convert_minutes: item.convert_minutes,
                        convert_limit: item.convert_limit,
                        once_convert_limit: item.once_convert_limit,
                        refresh_cycle: item.refresh.name().to_owned(),
                        start_at: item.start_at,
                        end_at: item.end_at,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `PostUserMysekaiConvertReceiveApi`: the resources the completed steps
    /// above the received progress produce, granted within the possession
    /// limits.
    fn convert_receive(
        &mut self,
        slot_id: i32,
        request: &UserMysekaiConvertReceiveRequest,
        now: i64,
        masters: &Tables<'_>,
    ) -> Result<(Vec<UserResource>, Vec<&'static str>), String> {
        let slot = self
            .doc
            .convert
            .slots
            .iter()
            .find(|slot| slot.slot_id == slot_id)
            .cloned()
            .ok_or_else(|| format!("userMysekaiConvertSlots holds no slot {slot_id}"))?;
        let converted = slot.completed_count(now);
        let receivable = converted - slot.received_progress;
        // A slot whose rows cover its converted steps is the slot a start
        // wrote (one step per convert, one row per obtain detail); a slot
        // without them (a panel clear leaves the steps behind the received
        // progress out) has nothing to hand out even when its clock says so.
        let drawn_steps = slot
            .obtain_resources
            .iter()
            .map(|row| row.progress)
            .max()
            .unwrap_or(0)
            .max(slot.received_progress);
        if drawn_steps < converted {
            return Err(format!(
                "slot {slot_id} has no drawn rows for the {} convert(s) the clock counts (the slot was not started)",
                converted - slot.received_progress
            ));
        }
        if request.receive_convert_count < 1 || request.receive_convert_count > receivable {
            return Err(format!(
                "receiveConvertCount {} is outside the {receivable} completed convert(s) of slot {slot_id}",
                request.receive_convert_count
            ));
        }
        // The possession limits a grant reads must be in: without them the
        // receive would have to guess whether a resource fits, so it refuses
        // before anything is handed out, naming the table it lacks.
        let limits = PossessionLimits::read(masters)?;
        let mut obtained = Vec::new();
        let mut changed: Vec<&'static str> = Vec::new();
        let mut received_to = slot.received_progress;
        'steps: for progress in (slot.received_progress + 1)..=(slot.received_progress + request.receive_convert_count) {
            for row in slot
                .obtain_resources
                .iter()
                .filter(|row| row.progress == progress)
            {
                let resource = row.to_resource();
                if !self.grant_resource(&resource, now, &limits, masters) {
                    info!(
                        "[server] slot {slot_id}: the {} {} at step {progress} does not fit its possession limit; it is reported but not granted",
                        resource.resource_type, resource.resource_id
                    );
                    obtained.push(resource);
                    break 'steps;
                }
                let section = grant_section(&resource.resource_type);
                if !changed.contains(&section) {
                    changed.push(section);
                }
                obtained.push(resource);
            }
            received_to = progress;
        }
        for slot in &mut self.doc.convert.slots {
            if slot.slot_id == slot_id {
                slot.received_progress = received_to;
            }
        }
        if !changed.contains(&SECTION_SLOTS) {
            changed.push(SECTION_SLOTS);
        }
        // The material possession follows the materials the grants changed.
        if changed.contains(&SECTION_MYSEKAI_MATERIALS) && !changed.contains(&SECTION_GAMEDATA) {
            changed.push(SECTION_GAMEDATA);
        }
        self.commit();
        info!(
            "[server] PostUserMysekaiConvertReceiveApi: slot {slot_id} received {} convert(s) up to {received_to}: obtained {:?}",
            request.receive_convert_count, obtained
        );
        Ok((obtained, changed))
    }

    /// Grants one resource within its possession limit; false when it does
    /// not fit (nothing changes then). The limits are read before the
    /// receive ([`PossessionLimits::read`]), so a missing master refuses the
    /// whole request by name instead of reading every grant as not fitting.
    fn grant_resource(
        &mut self,
        resource: &UserResource,
        now: i64,
        limits: &PossessionLimits,
        masters: &Tables<'_>,
    ) -> bool {
        match resource.resource_type.as_str() {
            "mysekai_material" => {
                // The limits are in (PossessionLimits::read ran first), so
                // the possession the client derives is the one to count.
                let limit = limits.material;
                let possession = self.material_possession(masters).map_or(0, |row| row.quantity);
                if possession.saturating_add(resource.quantity) > limit {
                    return false;
                }
                *self
                    .doc
                    .delivery
                    .mysekai_materials
                    .entry(resource.resource_id)
                    .or_insert(0) += resource.quantity;
                true
            }
            "mysekai_fixture" => {
                let limit = limits.fixture;
                let total: i32 = self.doc.inventory.fixtures.iter().map(|row| row.quantity).sum();
                if total.saturating_add(resource.quantity) > limit {
                    return false;
                }
                let fixtures = &mut self.doc.inventory.fixtures;
                match fixtures
                    .iter_mut()
                    .find(|row| row.mysekai_fixture_id == resource.resource_id)
                {
                    Some(row) => {
                        row.quantity += resource.quantity;
                        row.last_obtained_at = now;
                    }
                    None => fixtures.push(UserMysekaiFixture {
                        mysekai_fixture_id: resource.resource_id,
                        texture_id: resource.resource_level.max(1),
                        quantity: resource.quantity,
                        last_obtained_at: now,
                    }),
                }
                true
            }
            "mysekai_item" => {
                if resource.quantity < 1 {
                    return true;
                }
                let items = &mut self.doc.inventory.items;
                match items
                    .iter_mut()
                    .find(|row| row.mysekai_item_id == resource.resource_id)
                {
                    Some(row) => {
                        row.quantity += resource.quantity;
                        row.last_obtained_at = now;
                    }
                    None => items.push(UserMysekaiItem {
                        mysekai_item_id: resource.resource_id,
                        quantity: resource.quantity,
                        last_obtained_at: now,
                    }),
                }
                true
            }
            "mysekai_blueprint" => {
                let blueprints = &mut self.doc.inventory.blueprints;
                if blueprints
                    .iter()
                    .any(|row| row.mysekai_blueprint_id == resource.resource_id)
                {
                    return true;
                }
                blueprints.push(UserMysekaiBlueprint {
                    mysekai_blueprint_id: resource.resource_id,
                    obtained_at: now,
                });
                true
            }
            "mysekai_music_record" => {
                let records = &mut self.doc.music_records;
                if records
                    .iter()
                    .any(|row| row.mysekai_music_record_id == resource.resource_id)
                {
                    return true;
                }
                records.push(OwnedMusicRecord {
                    mysekai_music_record_id: resource.resource_id,
                    obtained_at: now,
                });
                true
            }
            // A type the owned tables do not carry: it is reported and not
            // granted.
            _ => true,
        }
    }

    /// `PostUserMysekaiConvertShortenApi`: the processing slot's remaining
    /// time is skipped for the crystals the slot priced.
    fn convert_shorten(
        &mut self,
        slot_id: i32,
        request: &UserMysekaiConvertShortenRequest,
        now: i64,
        masters: &Tables<'_>,
    ) -> Result<Vec<&'static str>, String> {
        let slot = self
            .doc
            .convert
            .slots
            .iter()
            .find(|slot| slot.slot_id == slot_id)
            .cloned()
            .ok_or_else(|| format!("userMysekaiConvertSlots holds no slot {slot_id}"))?;
        if !slot.is_processing() {
            return Err(format!("slot {slot_id} is not converting"));
        }
        let reached = slot.completed_count(now);
        let remaining_ms =
            i64::from(slot.once_convert_minutes) * i64::from(slot.convert_count - reached) * 60_000;
        if remaining_ms <= 0 {
            return Err(format!("slot {slot_id} has no time left to skip"));
        }
        let price = skip_price(remaining_ms, masters)?;
        if !self.doc.convert.policy.unjewelled && request.consume_jewel_quantity != price {
            return Err(format!(
                "consumeJewelQuantity {} is not the slot's price {price} (configs.{CONFIG_JEWEL_PER_MINUTE})",
                request.consume_jewel_quantity
            ));
        }
        for slot in &mut self.doc.convert.slots {
            if slot.slot_id == slot_id {
                *slot = slot.completed();
            }
        }
        self.commit();
        info!(
            "[server] PostUserMysekaiConvertShortenApi: slot {slot_id} skipped {remaining_ms} ms for {} crystals (the crystal wallet is not charged)",
            request.consume_jewel_quantity
        );
        Ok(vec![SECTION_SLOTS])
    }
}

/// `MysekaiConvertFurnitureSlotModel.SetupConsumedJewelQuantityForShorten`:
/// ceil(minutes) times the config's crystals per minute.
fn skip_price(remaining_ms: i64, masters: &Tables<'_>) -> Result<i32, String> {
    let per_minute = masters
        .configs
        .as_ref()
        .and_then(|configs| configs.get(CONFIG_JEWEL_PER_MINUTE))
        .and_then(|value| value.trim().parse::<i32>().ok())
        .ok_or_else(|| {
            format!("configs.{CONFIG_JEWEL_PER_MINUTE} is absent: the skip has no price")
        })?;
    let minutes = (remaining_ms.max(0) as f64 / 60_000.0).ceil() as i64;
    Ok(minutes
        .saturating_mul(i64::from(per_minute))
        .clamp(0, i64::from(i32::MAX)) as i32)
}

/// The section a granted resource's table is carried in.
fn grant_section(kind: &str) -> &'static str {
    match kind {
        "mysekai_material" => SECTION_MYSEKAI_MATERIALS,
        "mysekai_fixture" => SECTION_FIXTURES,
        "mysekai_item" => OWNED_ITEMS,
        "mysekai_blueprint" => SECTION_BLUEPRINTS,
        "mysekai_music_record" => RECORDS_SECTION,
        "mysekai_canvas" => SECTION_CANVASES,
        _ => SECTION_GAMEDATA,
    }
}

/// The native instrument `MOLY_CONVERT_MOCK_REQUEST` as a request; a
/// malformed entry stops loudly.
pub(crate) fn instrument_request() -> Option<(i32, ConvertPost)> {
    const NAME: &str = "MOLY_CONVERT_MOCK_REQUEST";
    let raw = super::instrument_env(NAME)?;
    let parts: Vec<&str> = raw.split(':').map(str::trim).collect();
    let number = |index: usize, what: &str| -> i32 {
        parts
            .get(index)
            .and_then(|part| part.parse::<i32>().ok())
            .unwrap_or_else(|| panic!("{NAME} {raw:?}: {what} is not an integer"))
    };
    let post = match parts.first().copied() {
        Some("start") => ConvertPost::Start(UserMysekaiConvertStartRequest {
            mysekai_convert_item_id: number(2, "the convert item id"),
            convert_count: number(3, "the convert count"),
            choice_id: parts.get(4).and_then(|part| part.parse().ok()).unwrap_or(0),
        }),
        Some("receive") => ConvertPost::Receive(UserMysekaiConvertReceiveRequest {
            receive_convert_count: number(2, "the receive count"),
        }),
        Some("shorten") => ConvertPost::Shorten(UserMysekaiConvertShortenRequest {
            consume_jewel_quantity: number(2, "the crystal price"),
        }),
        other => panic!("{NAME} {raw:?}: {other:?} is not start, receive or shorten"),
    };
    Some((number(1, "the slot id"), post))
}

// ---------------------------------------------------------------------------
// Panel edits and schema
// ---------------------------------------------------------------------------

fn edit_int(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} = {raw} does not fit a 32-bit integer"))
}

/// An edit of `userMysekaiGamedata.mysekaiConvertFixtureLevel`: the machine
/// level, at least 1.
pub(crate) fn parse_level(value: &Value, path: &str) -> Result<i32, String> {
    let level = edit_int(value, path)?;
    if level < 1 {
        return Err(format!("{path} = {level} is below 1"));
    }
    Ok(level)
}

/// A `server.edit` of this part: the sections the next response carries;
/// `None` when the path is not this part's.
pub(crate) fn edit_path(
    doc: &mut ConvertDoc,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<Vec<&'static str>, String>> {
    let result = match parts {
        [SECTION_SLOTS] => json::parse_convert_slots(value).map(|rows| {
            doc.slots = rows;
            vec![SECTION_SLOTS]
        }),
        [SECTION_HISTORIES] => json::parse_convert_histories(value).map(|rows| {
            doc.histories = rows;
            vec![SECTION_HISTORIES]
        }),
        // Clear a slot: nothing converts and nothing was received.
        [SECTION_SLOTS, id] if value.is_null() => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{path}: {id} is not a slot id"))?;
            match doc.slots.iter_mut().find(|slot| slot.slot_id == id) {
                Some(slot) => {
                    slot.convert_count = 0;
                    slot.received_progress = 0;
                    slot.convert_start_at = 0;
                    slot.obtain_resources.clear();
                    slot.mysekai_convert_slot_status = CONVERT_SLOT_COMPLETE.to_owned();
                    Ok(vec![SECTION_SLOTS])
                }
                None => Err(format!("{path}: userMysekaiConvertSlots holds no slot {id}")),
            }
        })(),
        [SECTION_SLOTS, id, "receivedProgress"] => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{path}: {id} is not a slot id"))?;
            let progress = edit_int(value, path)?;
            match doc.slots.iter_mut().find(|slot| slot.slot_id == id) {
                Some(slot) if progress >= 0 && progress <= slot.convert_count => {
                    slot.received_progress = progress;
                    Ok(vec![SECTION_SLOTS])
                }
                Some(slot) => Err(format!(
                    "{path} = {progress} is outside 0..{}",
                    slot.convert_count
                )),
                None => Err(format!("{path}: userMysekaiConvertSlots holds no slot {id}")),
            }
        })(),
        ["policies", POLICY_SHORTEN] => parse_policy(value).map(|policy| {
            doc.policy = policy;
            Vec::new()
        }),
        _ => return None,
    };
    Some(result)
}

/// The panel's sections.
pub(crate) fn schema_sections() -> Value {
    json!([
        {
            "key": "mysekaiConvert",
            "title": "Convert slots and item histories",
            "delivery": "next-response",
            "fields": [
                {"path": SECTION_SLOTS, "type": "rows", "row": {
                    "slotId": "int", "mysekaiConvertItemId": "int", "choiceId": "int",
                    "convertStartAt": "epoch-ms", "onceConvertMinutes": "int",
                    "convertCount": "int", "receivedProgress": "int",
                    "mysekaiConvertSlotStatus": "text",
                    "userMysekaiConvertObtainResources": "rows"},
                    "note": "one row per machine slot, seated at the join; the start call fills it"},
                {"path": "userMysekaiConvertSlots.<slotId>", "type": "null",
                    "note": "clear a slot: no converts, nothing received"},
                {"path": "userMysekaiConvertSlots.<slotId>.receivedProgress", "type": "int", "min": 0},
                {"path": SECTION_HISTORIES, "type": "rows", "row": {
                    "mysekaiConvertItemId": "int", "convertCount": "int",
                    "totalConvertCount": "int", "lastConvertStartAt": "epoch-ms"},
                    "note": "the items' convert counts against their master convertLimit"},
                {"path": "userMysekaiGamedata.mysekaiConvertFixtureLevel", "type": "int",
                    "min": 1, "levels": "masters.convertLevels",
                    "note": "the machine's level; an item whose requiredConvertFixtureLevel is above it cannot start"},
            ],
        },
        {
            "key": "mysekaiConvertPolicies",
            "title": "Convert server policies",
            "delivery": "live",
            "fields": [
                {"path": "policies.convertShorten", "type": "object",
                    "object": {"unjewelled": "bool"},
                    "note": "unjewelled: the skip is answered whatever crystal amount the request names"},
            ],
        },
    ])
}

/// The panel's named policies of this part.
pub(crate) fn schema_policies() -> Value {
    json!([
        {"name": "convert slots", "rule": "mysekaiConvertFixtureSlot gives the slot numbers (slotNoUpperLimit) and the ones a world pass extends the machine to (slotNoUpperLimitForMysekaiColorfulPass); the join seats one slot per extended number, and a slot above slotNoUpperLimit needs the pass"},
        {"name": "convert time", "rule": "a convert takes the master convertMinutes of the item; while the world pass runs, times mysekaiColorfulPass.mysekaiConvertFixtureConvertMinutesDecreaseRate, rounded down; a rate of 1 or more leaves the minutes as the master states them"},
        {"name": "convert machine level", "rule": "userMysekaiGamedata.mysekaiConvertFixtureLevel seats at the first mysekaiConvertFixtureLevels level; an item whose requiredConvertFixtureLevel is above it cannot start"},
        {"name": "convert draws (server-decided)", "rule": "a convert's resources are drawn when it starts: a resource_box item draws every detail (seq order, resourceQuantity) of the mysekai_convert_obtain box <externalId> as one step; the cost is every detail of the mysekai_convert_consumption box <consumeResourceBoxId>; a mysekai_blueprint item draws a random mysekaiBlueprints row with isObtainedByConvert the user does not own (the white blueprint item for white_blueprint); a mysekai_music_record item draws a mysekaiMusicRecords row the user does not own. The pick is seeded by the slot, the step, the choice and the item"},
        {"name": "convert convertible count", "rule": "an item with convertLimit is capped by the limit less its history convertCount (reset on a weekly or monthly refresh) and by the resources of its consumeResourceBoxId box, at onceConvertLimit at most; a pass slot is also capped by the pass's remaining minutes"},
        {"name": "convert start", "rule": "the slot must have no converts left; the request's count is within the item's onceConvertLimit, its consumption and (on a pass slot) the pass; the slot then converts from the server clock, and the items with a convertLimit count the request in userMysekaiConvertItemHistories"},
        {"name": "convert receive", "rule": "the converted count is the slot's elapsed time over its per-convert minutes; the resources of the received steps are granted within the material and fixture possession limits of possession level 1 and stop at the first draw that does not fit (reported, not granted)"},
        {"name": "convert shorten", "rule": "a processing slot's remaining minutes, rounded up, times configs.mysekai_convert_time_shorten_consume_jewel_per_minute is skipped for that many crystals; the crystal wallet is not carried by the document and is not charged"},
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed of the material a test's consumption box spends. In the JP
    /// master a `resource_box` item spends the `mysekai_convert_consumption`
    /// box `consumeResourceBoxId` names and draws the `mysekai_convert_obtain`
    /// box `externalId` names: two boxes under two purposes, so a box is
    /// (purpose, id). The tests' item `n` spends consumption box `n` (one
    /// material 5) and draws obtain box `n`.
    fn seed_box_materials(model: &mut ServerModel) {
        // Enough of material 5 for the starts: the cost of one convert.
        model.doc.delivery.mysekai_materials.insert(5, 20);
    }

    /// A convert item of the tests: item `item`, converted `minutes` a
    /// piece, spending consumption box `item` and drawing obtain box `item`.
    fn stable(item: i32, minutes: i32, limit: i32) -> ConvertItemMaster {
        ConvertItemMaster {
            id: item,
            tab_id: 1,
            consume_box_id: item,
            kind: ConvertItemKind::ResourceBox,
            external_id: item,
            required_level: 1,
            convert_minutes: minutes,
            convert_limit: limit,
            once_convert_limit: 10,
            refresh: ConvertRefresh::None,
            start_at: 0,
            end_at: 0,
        }
    }

    /// A server model on the checked-in default with a fixed clock, joined.
    fn joined_model() -> ServerModel {
        let doc = super::super::document::migrate_v1(
            crate::defaults::CHECKED_IN_SLICE,
            super::super::document::Migration::CheckedInDefault,
        )
        .expect("the checked-in slice migrates");
        let mut model = ServerModel::new(doc, super::super::Origin::CheckedInDefault, Vec::new());
        model.doc.clock = super::super::document::Clock::Fixed {
            at_ms: 1_790_300_000_000,
        };
        model
    }

    /// `masters.configs` with the convert skip price of 1 crystal a minute.
    fn fixture_masters() -> super::super::Masters {
        let mut masters = super::super::Masters::fixture();
        let mut configs = std::collections::BTreeMap::new();
        configs.insert(CONFIG_JEWEL_PER_MINUTE.to_owned(), "1".to_owned());
        masters.configs = Some(configs);
        masters
    }

    /// The tests' boxes, as the master keys them: a consumption box of one
    /// material 5 a convert (the spend), and obtain boxes of two details
    /// (box 3: materials 7 ×1 and 9 ×2; box 5: material 5 ×1; box 14:
    /// material 7 ×1). Every box holds `mysekai_material`s, as the master's
    /// convert boxes do.
    fn convert_test_boxes() -> Vec<ResourceBox> {
        let consumption = |id: i32| ResourceBox {
            purpose: BOX_CONSUMPTION.to_owned(),
            id,
            details: vec![("mysekai_material".to_owned(), 5, 1, 1)],
        };
        let obtain = |id: i32, details: Vec<(String, i32, i32, i32)>| ResourceBox {
            purpose: BOX_OBTAIN.to_owned(),
            id,
            details,
        };
        vec![
            consumption(3),
            consumption(5),
            consumption(14),
            obtain(
                3,
                vec![
                    ("mysekai_material".to_owned(), 7, 1, 1),
                    ("mysekai_material".to_owned(), 9, 1, 2),
                ],
            ),
            obtain(5, vec![("mysekai_material".to_owned(), 5, 1, 1)]),
            obtain(14, vec![("mysekai_material".to_owned(), 7, 1, 1)]),
        ]
    }

    /// The tests' convert tables: the fixture's machine, the items and the
    /// boxes.
    fn fixture_convert(items: Vec<ConvertItemMaster>) -> super::super::Masters {
        let mut masters = fixture_masters();
        let mut convert = masters.convert.clone().expect("the fixture convert tables");
        convert.items = Some(items);
        convert.resource_boxes = Some(convert_test_boxes());
        masters.convert = Some(convert);
        masters
    }

    #[test]
    fn the_master_rows_parse_as_the_masters_read_them() {
        // Resource boxes as the JP master writes them: a box row names its
        // `resourceBoxPurpose` beside its `id` (the two purposes reuse ids),
        // a detail's quantity is `resourceQuantity` and a detail need not
        // name a `resourceId` (the obtain box 10002 yields a `boost_item`
        // with one). Details read in `seq` order even when the rows do not.
        let boxes = parse_resource_boxes(
            concat!(
                r#"[{"resourceBoxPurpose":"mysekai_convert_consumption","id":10001,"resourceBoxType":"expand","details":[{"resourceBoxPurpose":"mysekai_convert_consumption","resourceBoxId":10001,"seq":2,"resourceType":"coin","resourceQuantity":1000},"#,
                r#"{"resourceBoxPurpose":"mysekai_convert_consumption","resourceBoxId":10001,"seq":1,"resourceType":"mysekai_material","resourceId":6,"resourceQuantity":100}]},"#,
                r#"{"resourceBoxPurpose":"mysekai_convert_obtain","id":10001,"resourceBoxType":"expand","details":[{"resourceBoxPurpose":"mysekai_convert_obtain","resourceBoxId":10001,"seq":1,"resourceType":"material","resourceId":15,"resourceQuantity":15}]},"#,
                r#"{"resourceBoxPurpose":"mysekai_convert_obtain","id":10002,"resourceBoxType":"expand","details":[{"resourceBoxPurpose":"mysekai_convert_obtain","resourceBoxId":10002,"seq":1,"resourceType":"boost_item","resourceQuantity":3}]}]"#,
            ),
        )
        .unwrap();
        // Three rows in, three boxes out: the consumption 10001, the obtain
        // 10001 and the obtain 10002.
        assert_eq!(boxes.len(), 3);
        // Sorted by id, the two 10001 rows come in table order (the
        // consumption row first here): the purpose tells them apart. Each
        // box's details are in `seq` order (the consumption box's coin row is
        // second in the table and second in the box).
        assert_eq!(boxes[0].purpose, BOX_CONSUMPTION);
        assert_eq!(boxes[0].id, 10001);
        assert_eq!(
            boxes[0].details,
            vec![
                ("mysekai_material".to_owned(), 6, 0, 100),
                ("coin".to_owned(), 0, 0, 1000),
            ]
        );
        assert_eq!(boxes[1].purpose, BOX_OBTAIN);
        assert_eq!(boxes[1].id, 10001);
        assert_eq!(boxes[1].details, vec![("material".to_owned(), 15, 0, 15)]);
        assert_eq!(boxes[2].purpose, BOX_OBTAIN);
        assert_eq!(boxes[2].id, 10002);
        assert_eq!(boxes[2].details, vec![("boost_item".to_owned(), 0, 0, 3)]);
        // The lookup is by purpose and id.
        let convert = ConvertMasters {
            items: Some(Vec::new()),
            slots: Some((1, 3)),
            resource_boxes: Some(boxes),
            ..ConvertMasters::default()
        };
        assert_eq!(
            convert.box_details(BOX_CONSUMPTION, 10001).map(|box_| box_.purpose.as_str()),
            Some(BOX_CONSUMPTION)
        );
        assert_eq!(
            convert.box_details(BOX_OBTAIN, 10001).map(|box_| box_.purpose.as_str()),
            Some(BOX_OBTAIN)
        );
        // The machine's slots: the plain number and the pass's.
        let mut masters = super::super::Masters::default();
        assert_eq!(
            parse_convert_fixture_slot(
                r#"[{"id":1,"slotNoLowerLimit":1,"slotNoUpperLimit":1,"slotNoUpperLimitForMysekaiColorfulPass":3}]"#,
                &mut masters
            ),
            Ok(())
        );
        assert_eq!(masters.convert_slots, Some((1, 3)));
        // A release table without the shorten row is refused by name.
        assert_eq!(
            parse_convert_level_releases(
                r#"[{"id":1,"mysekaiConvertFixtureLevel":2,"mysekaiConvertFixtureLevelReleaseType":"build"}]"#,
                &mut masters
            ),
            Err("mysekaiConvertFixtureLevelReleases has no convert_time_shorten row".to_owned())
        );
        // The pass's fraction is read as it is; 1 or more leaves the
        // minutes as the master states them.
        assert_eq!(
            parse_convert_pass_rate(
                r#"[{"id":1,"mysekaiConvertFixtureConvertMinutesDecreaseRate":0.5}]"#,
                &mut masters
            ),
            Ok(())
        );
        assert_eq!(masters.convert_pass_rate, Some(0.5));
        assert_eq!(
            parse_convert_pass_rate(
                r#"[{"id":1,"mysekaiConvertFixtureConvertMinutesDecreaseRate":100}]"#,
                &mut masters
            ),
            Ok(())
        );
        assert_eq!(masters.convert_pass_rate, Some(100.0));
        assert!(parse_convert_pass_rate(r#"[{"id":1}]"#, &mut masters).is_err());
        // An unknown obtain type is refused by name.
        assert!(parse_convert_blueprint_obtains(
            r#"[{"id":1,"mysekaiConvertItemObtainMysekaiBlueprintType":"x"}]"#,
            &mut masters
        )
        .is_err());
        assert_eq!(
            parse_convert_record_obtains(
                r#"[{"id":2,"mysekaiConvertItemObtainMysekaiMusicRecordType":"mysekai_music_record_category_choice","quantity":1}]"#,
                &mut masters
            ),
            Ok(())
        );
        assert_eq!(
            masters.convert_record_obtains.as_ref().unwrap()[0].kind,
            RECORD_CHOICE
        );
    }

    #[test]
    fn a_start_takes_the_consumption_and_draws_each_convert() {
        // Item 5 converts in 12 minutes and is once limited to 2 (its row's
        // `onceConvertLimit`); its consumption box 5 spends one material 5 a
        // convert and its obtain box 5 draws one material 5 a step (distinct
        // boxes, as the master's purposes make them).
        let mut item = stable(5, 12, 0);
        item.once_convert_limit = 2;
        let masters = fixture_convert(vec![item]);
        let tables = Tables::new(&masters, None);
        let mut model = joined_model();
        model.seat_convert_level(&tables);
        model.seat_convert_slots(&tables);
        seed_box_materials(&mut model);
        let now = model.now_ms();
        let changed = model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 5,
                    convert_count: 2,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .expect("the start is accepted");
        assert!(changed.contains(&SECTION_SLOTS));
        // Two converts spend two of the box's material 5, so the reply
        // carries the materials.
        assert!(changed.contains(&SECTION_MYSEKAI_MATERIALS));
        assert!(model.doc.convert.slots[0].is_processing());
        let slot = &model.doc.convert.slots[0];
        assert_eq!(slot.convert_count, 2);
        assert_eq!(slot.once_convert_minutes, 12);
        assert_eq!(slot.convert_start_at, now);
        assert!(slot.is_processing());
        // One convert is one completion step, so step 1 is at the first
        // convert's end and step 2 twelve minutes later.
        assert_eq!(slot.completed_count(now), 0);
        assert_eq!(slot.completed_count(now + 12 * 60_000), 1);
        assert_eq!(slot.completed_count(now + 24 * 60_000), 2);
        // One convert is one step, and the obtain box is not the consumption
        // box: both steps draw the obtain box's one material 5.
        assert_eq!(slot.obtain_resources.len(), 2);
        assert_eq!(slot.obtain_resources[0].progress, 1);
        assert_eq!(slot.obtain_resources[1].progress, 2);
        assert_eq!(slot.obtain_resources[0].quantity, 1);
        assert_eq!(slot.obtain_resources[1].quantity, 1);
        // Both converts' cost is spent: material 5 ×20 less ×2.
        assert_eq!(model.doc.delivery.mysekai_materials.get(&5), Some(&18));
        // A slot with converts left refuses another start.
        assert!(model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 5,
                    convert_count: 1,
                    choice_id: 0,
                },
                now + 60_000,
                &tables,
            )
            .is_err());
        // A pass slot without a pass refuses a start.
        assert!(model
            .convert_start(
                1,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 5,
                    convert_count: 1,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .is_err());
    }

    /// The master's obtain boxes carry more than one detail (obtain box
    /// 10009 yields five materials), so a `resource_box` step is one obtain
    /// row per detail, in `seq` order, numbered by `seq` from 0.
    #[test]
    fn every_detail_of_a_box_is_one_obtain_row_of_the_step() {
        let mut item = stable(3, 1, 0);
        item.once_convert_limit = 10;
        let masters = fixture_convert(vec![item]);
        let tables = Tables::new(&masters, None);
        let mut model = joined_model();
        model.seat_convert_level(&tables);
        model.seat_convert_slots(&tables);
        seed_box_materials(&mut model);
        let now = model.now_ms();
        model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 3,
                    convert_count: 1,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .expect("the start is accepted");
        let slot = &model.doc.convert.slots[0];
        assert_eq!(slot.obtain_resources.len(), 2);
        assert_eq!(slot.obtain_resources[0].progress, 1);
        assert_eq!(slot.obtain_resources[0].seq, 0);
        assert_eq!(slot.obtain_resources[0].resource_id, 7);
        assert_eq!(slot.obtain_resources[0].quantity, 1);
        assert_eq!(slot.obtain_resources[1].progress, 1);
        assert_eq!(slot.obtain_resources[1].seq, 1);
        assert_eq!(slot.obtain_resources[1].resource_id, 9);
        assert_eq!(slot.obtain_resources[1].quantity, 2);
        // The one convert's cost (one material 5) is spent; the obtain
        // box's materials are only drawn.
        assert_eq!(model.doc.delivery.mysekai_materials.get(&5), Some(&19));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&7), None);
        assert_eq!(model.doc.delivery.mysekai_materials.get(&9), None);
    }

    #[test]
    fn a_limited_item_counts_its_history() {
        let masters = fixture_convert(vec![stable(14, 2, 3), stable(3, 1, 0)]);
        let tables = Tables::new(&masters, None);
        let mut model = joined_model();
        model.seat_convert_level(&tables);
        model.seat_convert_slots(&tables);
        seed_box_materials(&mut model);
        let now = model.now_ms();
        model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 14,
                    convert_count: 2,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .expect("two of three");
        let history = model.doc.convert.histories[0];
        assert_eq!(history.convert_count, 2);
        assert_eq!(history.total_convert_count, 2);
        // One convert is left of the limit (3 less the 2 used), and the
        // consumption box's material is held by nobody, so it pays for any
        // number.
        let item = *tables
            .convert
            .as_ref()
            .and_then(|convert| convert.item(14))
            .expect("the master item");
        assert_eq!(model.item_once_convertible(&item, now, &tables), Ok(1));
    }

    #[test]
    fn a_receive_grants_the_steps_above_the_received_progress() {
        // Item 3's box 3 holds two details, so one convert's step is two
        // resources: the master's `ResourceBoxUtility.GetResources(box, 1)`
        // returns every detail.
        let mut item = stable(3, 1, 0);
        item.once_convert_limit = 2;
        let masters = fixture_convert(vec![item]);
        // A receive grants within the material and fixture possession
        // limits, so the shared tables carry one limit row of each at the
        // level the server reads.
        let limit = moly_masters::inventory::PossessionRow {
            level: POSSESSION_LEVEL,
            possession_limit: 1000,
        };
        let mut shared = moly_masters::load::MasterTables::fixture();
        shared.possession = Some(moly_masters::inventory::PossessionMasters {
            fixture: Some(vec![limit]),
            material: Some(vec![limit]),
        });
        let tables = Tables::new(&masters, Some(&shared));
        let mut model = joined_model();
        model.seat_convert_level(&tables);
        model.seat_convert_slots(&tables);
        // The start spends material 5 out of consumption box 3, so the test
        // seeds the material it holds (an empty row would make the cost
        // unaffordable and the start refuse).
        model.doc.delivery.mysekai_materials.insert(5, 20);
        let now = model.now_ms();
        let started = model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 3,
                    convert_count: 2,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .expect("the start is accepted");
        // The start committed: the two converts' cost (one material 5 each)
        // is out of the consumption box before the receive reads it.
        assert!(started.contains(&SECTION_MYSEKAI_MATERIALS));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&5), Some(&18));
        // The slot holds two steps, each the obtain box's two details: that
        // is what a receive grants (`UserMysekaiConvertSlot.GetObtainResources`
        // per step above the received progress).
        let slot = &model.doc.convert.slots[0];
        assert_eq!(slot.obtain_resources.len(), 4);
        assert_eq!(
            slot.obtain_resources
                .iter()
                .map(|row| (row.progress, row.seq, row.resource_id, row.quantity))
                .collect::<Vec<_>>(),
            vec![(1, 0, 7, 1), (1, 1, 9, 2), (2, 0, 7, 1), (2, 1, 9, 2)]
        );
        // Both one-minute converts are done after two minutes.
        let (resources, changed) = model
            .convert_receive(
                0,
                &UserMysekaiConvertReceiveRequest {
                    receive_convert_count: 2,
                },
                now + 120_000,
                &tables,
            )
            .expect("the receive is accepted");
        // Two steps of two details: four resources, in the slot's row order.
        // The expected rows are the slot's own under the rule
        // `ResourceBoxUtility.GetResources` follows (a box's details become
        // resources), the same rows the start drew.
        assert_eq!(resources.len(), 4);
        assert_eq!(
            resources
                .iter()
                .map(|resource| (resource.resource_id, resource.quantity))
                .collect::<Vec<_>>(),
            vec![(7, 1), (9, 2), (7, 1), (9, 2)]
        );
        assert!(changed.contains(&SECTION_SLOTS));
        // The receive grants materials (7 ×1 twice, 9 ×2 twice); those
        // grants are the reply's own record, and the tables they change are
        // carried. Material 5 takes no grant.
        assert!(changed.contains(&SECTION_MYSEKAI_MATERIALS));
        assert!(changed.contains(&SECTION_GAMEDATA));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&5), Some(&18));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&7), Some(&2));
        assert_eq!(model.doc.delivery.mysekai_materials.get(&9), Some(&4));
        assert_eq!(model.doc.convert.slots[0].received_progress, 2);
        // Nothing is left to receive.
        assert!(model
            .convert_receive(
                0,
                &UserMysekaiConvertReceiveRequest {
                    receive_convert_count: 1,
                },
                now + 200_000,
                &tables,
            )
            .is_err());
    }

    #[test]
    fn a_shorten_refuses_another_price_and_completes_the_slot() {
        let mut item = stable(5, 2, 0);
        item.once_convert_limit = 2;
        let masters = fixture_convert(vec![item]);
        let tables = Tables::new(&masters, None);
        let mut model = joined_model();
        model.seat_convert_level(&tables);
        model.seat_convert_slots(&tables);
        seed_box_materials(&mut model);
        let now = model.now_ms();
        model
            .convert_start(
                0,
                &UserMysekaiConvertStartRequest {
                    mysekai_convert_item_id: 5,
                    convert_count: 2,
                    choice_id: 0,
                },
                now,
                &tables,
            )
            .expect("the start is accepted");
        // The skip is offered on a processing slot only
        // (`MysekaiConvertFurniturePresenter.OnShortenSlot`); the start left
        // the slot processing.
        assert!(model.doc.convert.slots[0].is_processing());
        // Two two-minute converts left: four minutes, four crystals, each
        // minute rounded up (1 crystal a minute in the configs).
        assert!(model
            .convert_shorten(
                0,
                &UserMysekaiConvertShortenRequest {
                    consume_jewel_quantity: 3
                },
                now,
                &tables,
            )
            .is_err());
        let changed = model
            .convert_shorten(
                0,
                &UserMysekaiConvertShortenRequest {
                    consume_jewel_quantity: 4
                },
                now,
                &tables,
            )
            .unwrap();
        assert_eq!(changed, vec![SECTION_SLOTS]);
        // The skip completes the slot: every convert is done at the moment of
        // the skip (`MysekaiConvertFurnitureSlotModel.SetupInComplete`:
        // status complete, normalized progress 1, every convert completed).
        let slot = &model.doc.convert.slots[0];
        assert_eq!(slot.mysekai_convert_slot_status, CONVERT_SLOT_COMPLETE);
        assert_eq!(slot.completed_count(now), 2);
    }

    fn item(minutes: i32) -> ConvertItemMaster {
        ConvertItemMaster {
            id: 3,
            tab_id: 1,
            consume_box_id: 10003,
            kind: ConvertItemKind::ResourceBox,
            external_id: 3,
            required_level: 1,
            convert_minutes: minutes,
            convert_limit: 0,
            once_convert_limit: 100,
            refresh: ConvertRefresh::None,
            start_at: 0,
            end_at: 0,
        }
    }

    #[test]
    fn the_skip_price_is_the_minutes_times_the_config() {
        let masters = fixture_masters();
        let tables = Tables::new(&masters, None);
        assert_eq!(skip_price(0, &tables), Ok(0));
        assert_eq!(skip_price(1, &tables), Ok(1));
        assert_eq!(skip_price(60_000, &tables), Ok(1));
        assert_eq!(skip_price(60_001, &tables), Ok(2));
        assert_eq!(skip_price(125 * 60_000, &tables), Ok(125));
        let mut bare = fixture_masters();
        bare.configs = None;
        assert!(skip_price(1, &Tables::new(&bare, None)).is_err());
    }

    #[test]
    fn a_slot_completes_one_convert_each_of_its_minutes() {
        let slot = ConvertSlot {
            slot_id: 0,
            mysekai_convert_item_id: 1,
            choice_id: 0,
            convert_start_at: 1_000_000,
            once_convert_minutes: 2,
            convert_count: 3,
            received_progress: 0,
            mysekai_convert_slot_status: CONVERT_SLOT_PROCESSING.to_owned(),
            obtain_resources: Vec::new(),
        };
        assert_eq!(slot.completed_count(1_000_000 - 1), 0);
        assert_eq!(slot.completed_count(1_000_000), 0);
        assert_eq!(slot.completed_count(1_000_000 + 119_999), 0);
        assert_eq!(slot.completed_count(1_000_000 + 120_000), 1);
        assert_eq!(slot.completed_count(1_000_000 + 360_000), 3);
        assert_eq!(slot.completed_count(i64::MAX), 3);
        assert_eq!(slot.end_at(), 1_000_000 + 360_000);
        // `SetupInComplete`: a complete slot has every convert done, at any
        // clock (a skip sets it complete before its time runs out).
        let done = slot.completed();
        assert_eq!(done.mysekai_convert_slot_status, CONVERT_SLOT_COMPLETE);
        assert!(!done.is_processing());
        assert_eq!(done.completed_count(1_000_000), 3);
    }

    #[test]
    fn the_pass_rate_is_a_fraction() {
        // mysekaiColorfulPass.mysekaiConvertFixtureConvertMinutesDecreaseRate is
        // 0.5: a 12-minute convert takes 6 minutes while the pass runs.
        let convert = ConvertMasters {
            pass_rate: Some(0.5),
            ..ConvertMasters::default()
        };
        assert_eq!(once_minutes(&item(12), &convert, true), 6);
        assert_eq!(once_minutes(&item(12), &convert, false), 12);
        assert_eq!(once_minutes(&item(2640), &convert, true), 1320);
    }

    #[test]
    fn the_draw_is_seeded_by_the_slot_and_step() {
        let mut a = Seeded::seed(&[0, 1, 0, 22]);
        let mut b = Seeded::seed(&[0, 1, 0, 22]);
        let mut c = Seeded::seed(&[0, 2, 0, 22]);
        assert_eq!(a.pick(5), b.pick(5));
        assert!((0..5).contains(&c.pick(5)));
        assert_eq!(Seeded::seed(&[0]).pick(1), 0);
    }
}
