//! The server document: every server-decided value the client receives, in
//! one text the player edits (schemaVersion 2).
//!
//! Sections use the source's response keys (`userMysekaiGamedata`,
//! `userMysekaiStamina`, `userMysekaiColorfulPass`,
//! `mysekaiPhenomenaSchedules`, `userMysekaiGateCharacterVisit`,
//! `userMysekaiSiteHousingLayouts`, and these, optional on read: the
//! birthday-party delivery's `userBirthdayParties`, `userMaterials`,
//! `userMysekaiMaterials`, `userCards` and `userHonors`, the music record
//! settings' `userMysekaiMusicPlayFixtureSettings`, the avatar's
//! `userAvatar`, and the owned data's `userMysekaiFixtures`,
//! `userMysekaiCanvases`, `userMysekaiBlueprints`, `userMysekaiItems` and
//! the two possession levels in `userMysekaiGamedata`). A `masterConfigs` key of an earlier document is read and
//! ignored: those values are master data. The rest are the mock's own keys and
//! name themselves as such: `clock` (the one server clock), `policies` (the
//! server rules this mock stands in for), `phenomenaSchedulePolicy`,
//! `talkListPolicy` and `controls`.
//!
//! A schemaVersion 1 document is the NPC slice of the server panel. Two
//! migrations exist and say which: a slice read from the asset source (a
//! harness or owner state) keeps its fixed clock at its `refreshedAt` and its
//! stated schedule rows, so replays built on it see the same values; the
//! checked-in slice becomes the product default, with the device clock and
//! a daily schedule of the same phenomenon per refresh window.
//!
//! Parsing is strict: an unknown key, a missing key, a wrong type, or a
//! schemaVersion other than 2 is refused by name. Nothing is reset silently.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

pub(crate) const SCHEMA_VERSION: u64 = 2;
const V1: u64 = 1;

/// The named stamina refresh policy, as the schema and the document spell it.
pub(crate) const REFILL_POLICY: &str = "refillNormalToMasterMaxOnNewRefreshWindow";
/// Its description in the schema.
pub(crate) const REFILL_POLICY_TEXT: &str =
    "refill normal to master max on entering a new refresh window";
/// No automatic refill: stamina changes only through the harvest replies and
/// panel edits.
pub(crate) const NO_REFILL_POLICY: &str = "none";

/// The server clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clock {
    /// Device time plus an offset (milliseconds).
    Device { offset_ms: i64 },
    /// A fixed epoch millisecond that never advances.
    Fixed { at_ms: i64 },
}

/// `UserMysekaiStamina`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamina {
    pub(crate) normal: i32,
    pub(crate) enhance: i32,
    pub(crate) boost: i32,
}

/// `UserMysekaiGamedata`, the fields the client reads here.
/// `mysekai_rank` is `None` until the server derives it from `totalExp`
/// through the master rank table (a default or migrated document).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gamedata {
    pub(crate) mysekai_rank: Option<i32>,
    pub(crate) total_exp: i32,
    pub(crate) refreshed_at: i64,
    pub(crate) is_mysekai_tutorial_end: bool,
}

/// `UserMysekaiColorfulPass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColorfulPass {
    pub(crate) id: i32,
    pub(crate) expired_at: i64,
}

/// `MysekaiPhenomenaSchedule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScheduleRow {
    pub(crate) refresh_time_period_id: i32,
    pub(crate) schedule_date: i64,
    pub(crate) phenomena_id: i32,
}

/// Where the served `mysekaiPhenomenaSchedules` come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SchedulePolicy {
    /// The document's rows, verbatim.
    Stated,
    /// Rows for the device-local day before, the day of and the day after the
    /// server clock, one per master refresh window, each with the phenomenon
    /// this map gives its window.
    Daily(BTreeMap<i32, i32>),
}

/// The stamina refresh policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StaminaRefresh {
    /// [`REFILL_POLICY_TEXT`].
    RefillNormal,
    /// [`NO_REFILL_POLICY`].
    None,
}

/// `UserMysekaiGate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) gate_id: i32,
    pub(crate) skin_id: i32,
    pub(crate) level: i32,
    pub(crate) visit_count: i32,
    pub(crate) last_refreshed_at: i64,
    pub(crate) is_setting_at_home_site: bool,
}

/// `UserMysekaiGateCharacter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GateCharacter {
    pub(crate) gate_id: i32,
    pub(crate) unit_group_id: i32,
    pub(crate) is_reservation: bool,
    pub(crate) visit_count: i32,
}

/// One document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ServerDocument {
    pub(crate) about: String,
    pub(crate) clock: Clock,
    pub(crate) stamina_refresh: StaminaRefresh,
    pub(crate) gamedata: Gamedata,
    /// `None` until the server seats a new user's pools from the masters.
    pub(crate) stamina: Option<Stamina>,
    pub(crate) colorful_pass: Option<ColorfulPass>,
    pub(crate) schedule_policy: SchedulePolicy,
    pub(crate) schedules: Vec<ScheduleRow>,
    pub(crate) gate: Gate,
    pub(crate) gate_characters: Vec<GateCharacter>,
    /// `userMysekaiGateCharacterVisit.mysekaiCharacterTalkWithReadHistories`
    /// when the document states the talk list (verbatim rows).
    pub(crate) talk_histories: Option<Value>,
    /// `talkListPolicy` when the server's talk-list policy builds the list.
    pub(crate) talk_list_policy: Option<Value>,
    /// `userMysekaiSiteHousingLayouts`, verbatim (the server panel parses it).
    pub(crate) layouts: Value,
    /// `controls`, verbatim (the server panel parses it).
    pub(crate) controls: Value,
    /// The birthday-party delivery sections ([`super::delivery`]).
    pub(crate) delivery: super::delivery::DeliveryDoc,
    /// `policies.homeActionReply` ([`super::home_action`]).
    pub(crate) home_action_reply: super::home_action::HomeActionReplyPolicy,
    /// `userMysekaiMusicPlayFixtureSettings` ([`super::music`]).
    pub(crate) music_settings: Vec<super::client::music::MusicPlaySetting>,
    /// `userAvatar` ([`super::avatar`]).
    pub(crate) avatar: super::client::avatar::UserAvatar,
    /// The owned MySekai tables and possession levels ([`super::inventory`]).
    pub(crate) inventory: super::inventory::InventoryDoc,
}

/// Which migration a schemaVersion 1 slice takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Migration {
    /// A slice from the asset source: fixed clock at its `refreshedAt`,
    /// stated rows.
    AssetSource,
    /// The checked-in slice as the product default: device clock, daily rows.
    CheckedInDefault,
}

/// The total experience of a document that does not state one (the menu
/// mock's earlier chosen value).
pub(crate) const DEFAULT_TOTAL_EXP: i32 = 180_000;

// ---------------------------------------------------------------------------
// Field readers
// ---------------------------------------------------------------------------

pub(super) fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{at} is not an object"))
}

pub(super) fn only(value: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    match value.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("{at} has an unknown field {key}")),
        None => Ok(()),
    }
}

pub(super) fn field<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("{at} has no field {key}"))
}

pub(super) fn int64(value: &Map<String, Value>, key: &str, at: &str) -> Result<i64, String> {
    field(value, key, at)?
        .as_i64()
        .ok_or_else(|| format!("{at}.{key} is not an integer"))
}

pub(super) fn int32(value: &Map<String, Value>, key: &str, at: &str) -> Result<i32, String> {
    let raw = int64(value, key, at)?;
    i32::try_from(raw).map_err(|_| format!("{at}.{key} = {raw} does not fit a 32-bit integer"))
}

pub(super) fn boolean(value: &Map<String, Value>, key: &str, at: &str) -> Result<bool, String> {
    field(value, key, at)?
        .as_bool()
        .ok_or_else(|| format!("{at}.{key} is not a boolean"))
}

pub(super) fn array<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<&'a Vec<Value>, String> {
    field(value, key, at)?
        .as_array()
        .ok_or_else(|| format!("{at}.{key} is not an array"))
}

/// The schemaVersion of a document text, refused by name when absent.
pub(crate) fn schema_version(value: &Value, what: &str) -> Result<u64, String> {
    value
        .get("schemaVersion")
        .or_else(|| value.get("version"))
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("the {what} document has no integer schemaVersion"))
}

// ---------------------------------------------------------------------------
// Sections shared by both versions
// ---------------------------------------------------------------------------

pub(crate) fn parse_schedules(rows: &[Value], at: &str) -> Result<Vec<ScheduleRow>, String> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{at}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiRefreshTimePeriodId",
                    "scheduleDate",
                    "mysekaiPhenomenaId",
                ],
                &at,
            )?;
            Ok(ScheduleRow {
                refresh_time_period_id: int32(row, "mysekaiRefreshTimePeriodId", &at)?,
                schedule_date: int64(row, "scheduleDate", &at)?,
                phenomena_id: int32(row, "mysekaiPhenomenaId", &at)?,
            })
        })
        .collect()
}

fn parse_gate_characters(rows: &[Value], at: &str) -> Result<Vec<GateCharacter>, String> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{at}[{i}]");
            let row = object(row, &at)?;
            only(
                row,
                &[
                    "mysekaiGateId",
                    "mysekaiGameCharacterUnitGroupId",
                    "isReservation",
                    "visitCount",
                ],
                &at,
            )?;
            Ok(GateCharacter {
                gate_id: int32(row, "mysekaiGateId", &at)?,
                unit_group_id: int32(row, "mysekaiGameCharacterUnitGroupId", &at)?,
                is_reservation: boolean(row, "isReservation", &at)?,
                visit_count: int32(row, "visitCount", &at)?,
            })
        })
        .collect()
}

/// The visit block: the gate, its characters and an optional stated talk
/// list. Version 1 has no skin id; version 2 states one.
fn parse_visit(
    value: &Value,
    with_skin: bool,
) -> Result<(Gate, Vec<GateCharacter>, Option<Value>), String> {
    const AT: &str = "userMysekaiGateCharacterVisit";
    let visit = object(value, AT)?;
    only(
        visit,
        &[
            "userMysekaiGate",
            "userMysekaiGateCharacters",
            "mysekaiCharacterTalkWithReadHistories",
        ],
        AT,
    )?;
    let gate_value = object(field(visit, "userMysekaiGate", AT)?, "userMysekaiGate")?;
    let mut gate_fields = vec![
        "mysekaiGateId",
        "mysekaiGateLevel",
        "visitCount",
        "lastRefreshedAt",
        "isSettingAtHomeSite",
    ];
    if with_skin {
        gate_fields.push("mysekaiGateSkinId");
    }
    only(gate_value, &gate_fields, "userMysekaiGate")?;
    let gate = Gate {
        gate_id: int32(gate_value, "mysekaiGateId", "userMysekaiGate")?,
        skin_id: if with_skin {
            int32(gate_value, "mysekaiGateSkinId", "userMysekaiGate")?
        } else {
            0
        },
        level: int32(gate_value, "mysekaiGateLevel", "userMysekaiGate")?,
        visit_count: int32(gate_value, "visitCount", "userMysekaiGate")?,
        last_refreshed_at: int64(gate_value, "lastRefreshedAt", "userMysekaiGate")?,
        is_setting_at_home_site: boolean(gate_value, "isSettingAtHomeSite", "userMysekaiGate")?,
    };
    let characters = parse_gate_characters(
        array(visit, "userMysekaiGateCharacters", AT)?,
        "userMysekaiGateCharacters",
    )?;
    let talks = visit.get("mysekaiCharacterTalkWithReadHistories").cloned();
    if let Some(talks) = &talks {
        check_talk_histories(talks)?;
    }
    Ok((gate, characters, talks))
}

fn check_talk_histories(value: &Value) -> Result<(), String> {
    const AT: &str = "mysekaiCharacterTalkWithReadHistories";
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{AT} is not an array"))?;
    let mut seen = std::collections::HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let at = format!("{AT}[{i}]");
        let row = object(row, &at)?;
        only(row, &["mysekaiCharacterTalkId", "isRead"], &at)?;
        let id = int32(row, "mysekaiCharacterTalkId", &at)?;
        boolean(row, "isRead", &at)?;
        if !seen.insert(id) {
            return Err(format!("{at} repeats talk {id}"));
        }
    }
    Ok(())
}

/// The talk list must come from exactly one of the two sources.
fn check_talk_source(stated: &Option<Value>, policy: &Option<Value>) -> Result<(), String> {
    match (stated, policy) {
        (Some(_), Some(_)) => {
            Err("the document both states a talk list and asks the talk-list policy for one".into())
        }
        (None, None) => Err(
            "the document neither states a talk list nor asks the talk-list policy for one".into(),
        ),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Version 2
// ---------------------------------------------------------------------------

const V2_FIELDS: [&str; 14] = [
    "schemaVersion",
    "about",
    "clock",
    "policies",
    "userMysekaiGamedata",
    "userMysekaiStamina",
    "userMysekaiColorfulPass",
    "phenomenaSchedulePolicy",
    "mysekaiPhenomenaSchedules",
    "userMysekaiGateCharacterVisit",
    "userMysekaiSiteHousingLayouts",
    "talkListPolicy",
    "controls",
    "pending",
];

pub(crate) fn parse_clock(value: &Value) -> Result<Clock, String> {
    let clock = object(value, "clock")?;
    match field(clock, "mode", "clock")?.as_str() {
        Some("device") => {
            only(clock, &["mode", "offsetMs"], "clock")?;
            Ok(Clock::Device {
                offset_ms: int64(clock, "offsetMs", "clock")?,
            })
        }
        Some("fixed") => {
            only(clock, &["mode", "at"], "clock")?;
            let at_ms = int64(clock, "at", "clock")?;
            if at_ms <= 0 {
                return Err(format!(
                    "clock.at = {at_ms} is not a positive epoch millisecond"
                ));
            }
            Ok(Clock::Fixed { at_ms })
        }
        _ => Err("clock.mode is neither \"device\" nor \"fixed\"".into()),
    }
}

pub(crate) fn parse_stamina_refresh(value: &Value) -> Result<StaminaRefresh, String> {
    match value.as_str() {
        Some(REFILL_POLICY) => Ok(StaminaRefresh::RefillNormal),
        Some(NO_REFILL_POLICY) => Ok(StaminaRefresh::None),
        _ => Err(format!(
            "policies.staminaRefresh is neither \"{REFILL_POLICY}\" ({REFILL_POLICY_TEXT}) nor \"{NO_REFILL_POLICY}\""
        )),
    }
}

pub(crate) fn parse_stamina(value: &Value) -> Result<Option<Stamina>, String> {
    const AT: &str = "userMysekaiStamina";
    if value.is_null() {
        return Ok(None);
    }
    let stamina = object(value, AT)?;
    only(
        stamina,
        &["normalStamina", "enhanceStamina", "boostStamina"],
        AT,
    )?;
    Ok(Some(Stamina {
        normal: int32(stamina, "normalStamina", AT)?,
        enhance: int32(stamina, "enhanceStamina", AT)?,
        boost: int32(stamina, "boostStamina", AT)?,
    }))
}

pub(crate) fn parse_colorful_pass(value: &Value) -> Result<Option<ColorfulPass>, String> {
    const AT: &str = "userMysekaiColorfulPass";
    if value.is_null() {
        return Ok(None);
    }
    let pass = object(value, AT)?;
    only(pass, &["mysekaiColorfulPassId", "expiredAt"], AT)?;
    Ok(Some(ColorfulPass {
        id: int32(pass, "mysekaiColorfulPassId", AT)?,
        expired_at: int64(pass, "expiredAt", AT)?,
    }))
}

pub(crate) fn parse_schedule_policy(value: &Value) -> Result<SchedulePolicy, String> {
    const AT: &str = "phenomenaSchedulePolicy";
    let policy = object(value, AT)?;
    match field(policy, "kind", AT)?.as_str() {
        Some("stated") => {
            only(policy, &["kind"], AT)?;
            Ok(SchedulePolicy::Stated)
        }
        Some("daily") => {
            only(policy, &["kind", "byRefreshTimePeriod"], AT)?;
            let map = object(
                field(policy, "byRefreshTimePeriod", AT)?,
                "phenomenaSchedulePolicy.byRefreshTimePeriod",
            )?;
            let mut by_period = BTreeMap::new();
            for (key, phenomenon) in map {
                let period = key.parse::<i32>().map_err(|_| {
                    format!("phenomenaSchedulePolicy.byRefreshTimePeriod key {key} is not a refresh window id")
                })?;
                let phenomenon = phenomenon
                    .as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    .ok_or_else(|| {
                        format!("phenomenaSchedulePolicy.byRefreshTimePeriod.{key} is not a phenomenon id")
                    })?;
                by_period.insert(period, phenomenon);
            }
            if by_period.is_empty() {
                return Err("phenomenaSchedulePolicy.byRefreshTimePeriod is empty".into());
            }
            Ok(SchedulePolicy::Daily(by_period))
        }
        _ => Err("phenomenaSchedulePolicy.kind is neither \"stated\" nor \"daily\"".into()),
    }
}

fn parse_gamedata(value: &Value) -> Result<Gamedata, String> {
    const AT: &str = "userMysekaiGamedata";
    let gamedata = object(value, AT)?;
    only(
        gamedata,
        &[
            "mysekaiRank",
            "totalExp",
            "refreshedAt",
            "isMysekaiTutorialEnd",
            // Read by the owned-data part (the possession levels).
            "mysekaiMaterialPossessionLevel",
            "mysekaiFixturePossessionLevel",
        ],
        AT,
    )?;
    let mysekai_rank = match field(gamedata, "mysekaiRank", AT)? {
        Value::Null => None,
        _ => Some(int32(gamedata, "mysekaiRank", AT)?),
    };
    Ok(Gamedata {
        mysekai_rank,
        total_exp: int32(gamedata, "totalExp", AT)?,
        refreshed_at: int64(gamedata, "refreshedAt", AT)?,
        is_mysekai_tutorial_end: boolean(gamedata, "isMysekaiTutorialEnd", AT)?,
    })
}

/// A schemaVersion 2 text. `pending` (the sections the next response will
/// carry) is returned beside the document.
pub(crate) fn parse_v2(text: &str) -> Result<(ServerDocument, Vec<String>), String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("the server document is not JSON: {error}"))?;
    let version = schema_version(&value, "server")?;
    if version != SCHEMA_VERSION {
        return Err(format!(
            "server document schemaVersion {version} is not supported (this engine reads {SCHEMA_VERSION})"
        ));
    }
    let doc = object(&value, "the server document")?;
    let allowed: Vec<&str> = V2_FIELDS
        .iter()
        .chain(super::delivery::DOCUMENT_KEYS.iter())
        .chain([super::music::SECTION, super::avatar::SECTION].iter())
        .chain(super::inventory::DOCUMENT_KEYS.iter())
        .copied()
        .collect();
    only(doc, &allowed, "the server document")?;
    for key in V2_FIELDS {
        if key != "talkListPolicy" && key != "pending" && !doc.contains_key(key) {
            return Err(format!("the server document has no field {key}"));
        }
    }
    let about = field(doc, "about", "the server document")?
        .as_str()
        .ok_or("about is not a string")?
        .to_owned();
    let policies = object(field(doc, "policies", "the server document")?, "policies")?;
    let allowed: Vec<&str> = std::iter::once("staminaRefresh")
        .chain(super::delivery::POLICY_KEYS.iter().copied())
        .chain(std::iter::once(super::home_action::POLICY_KEY))
        .chain(super::inventory::POLICY_KEYS.iter().copied())
        .collect();
    only(policies, &allowed, "policies")?;
    let delivery = super::delivery::DeliveryDoc::parse(doc, policies)?;
    let home_action_reply = super::home_action::parse(policies)?;
    let music_settings = super::music::parse(doc)?;
    let avatar = super::avatar::parse(doc)?;
    let inventory = super::inventory::InventoryDoc::parse(doc, policies)?;
    let (gate, gate_characters, talk_histories) = parse_visit(
        field(doc, "userMysekaiGateCharacterVisit", "the server document")?,
        true,
    )?;
    let talk_list_policy = doc.get("talkListPolicy").cloned();
    check_talk_source(&talk_histories, &talk_list_policy)?;
    let layouts = field(doc, "userMysekaiSiteHousingLayouts", "the server document")?.clone();
    if !layouts.is_array() {
        return Err("userMysekaiSiteHousingLayouts is not an array".into());
    }
    let controls = field(doc, "controls", "the server document")?.clone();
    if !controls.is_object() {
        return Err("controls is not an object".into());
    }
    let pending = match doc.get("pending") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or("pending is not an array")?
            .iter()
            .map(|section| {
                section
                    .as_str()
                    .filter(|name| super::edit::PENDING_SECTIONS.contains(name))
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        format!(
                            "pending names {section}, which is not a section a response carries"
                        )
                    })
            })
            .collect::<Result<_, _>>()?,
    };
    let document = ServerDocument {
        about,
        clock: parse_clock(field(doc, "clock", "the server document")?)?,
        stamina_refresh: parse_stamina_refresh(field(policies, "staminaRefresh", "policies")?)?,
        gamedata: parse_gamedata(field(doc, "userMysekaiGamedata", "the server document")?)?,
        stamina: parse_stamina(field(doc, "userMysekaiStamina", "the server document")?)?,
        colorful_pass: parse_colorful_pass(field(
            doc,
            "userMysekaiColorfulPass",
            "the server document",
        )?)?,
        schedule_policy: parse_schedule_policy(field(
            doc,
            "phenomenaSchedulePolicy",
            "the server document",
        )?)?,
        schedules: parse_schedules(
            array(doc, "mysekaiPhenomenaSchedules", "the server document")?,
            "mysekaiPhenomenaSchedules",
        )?,
        gate,
        gate_characters,
        talk_histories,
        talk_list_policy,
        layouts,
        controls,
        delivery,
        home_action_reply,
        music_settings,
        avatar,
        inventory,
    };
    document.check_structure()?;
    Ok((document, pending))
}

// ---------------------------------------------------------------------------
// Version 1 (the NPC slice)
// ---------------------------------------------------------------------------

/// A schemaVersion 1 slice, migrated.
pub(crate) fn migrate_v1(text: &str, migration: Migration) -> Result<ServerDocument, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("the server panel slice is not JSON: {error}"))?;
    let version = schema_version(&value, "server panel slice")?;
    if version != V1 {
        return Err(format!(
            "server panel slice version {version} is not version {V1}"
        ));
    }
    let doc = object(&value, "the slice")?;
    only(
        doc,
        &[
            "version",
            "about",
            "userMysekaiGamedata",
            "mysekaiPhenomenaSchedules",
            "userMysekaiGateCharacterVisit",
            "userMysekaiSiteHousingLayouts",
            "talkListPolicy",
            "controls",
        ],
        "the slice",
    )?;
    let gamedata = object(
        field(doc, "userMysekaiGamedata", "the slice")?,
        "userMysekaiGamedata",
    )?;
    only(
        gamedata,
        &["refreshedAt", "isMysekaiTutorialEnd"],
        "userMysekaiGamedata",
    )?;
    let refreshed_at = int64(gamedata, "refreshedAt", "userMysekaiGamedata")?;
    let schedules = parse_schedules(
        array(doc, "mysekaiPhenomenaSchedules", "the slice")?,
        "mysekaiPhenomenaSchedules",
    )?;
    let (gate, gate_characters, talk_histories) = parse_visit(
        field(doc, "userMysekaiGateCharacterVisit", "the slice")?,
        false,
    )?;
    let talk_list_policy = doc.get("talkListPolicy").cloned();
    check_talk_source(&talk_histories, &talk_list_policy)?;
    let (clock, schedule_policy) = match migration {
        Migration::AssetSource => (
            Clock::Fixed {
                at_ms: refreshed_at,
            },
            SchedulePolicy::Stated,
        ),
        Migration::CheckedInDefault => {
            let mut by_period = BTreeMap::new();
            for row in &schedules {
                by_period
                    .entry(row.refresh_time_period_id)
                    .or_insert(row.phenomena_id);
            }
            if by_period.is_empty() {
                return Err(
                    "the checked-in slice has no schedule row to take the daily phenomena from"
                        .into(),
                );
            }
            (
                Clock::Device { offset_ms: 0 },
                SchedulePolicy::Daily(by_period),
            )
        }
    };
    let about = match migration {
        Migration::AssetSource => format!(
            "Migrated from a schemaVersion 1 server panel slice of the asset source: the clock is fixed at its refreshedAt and its schedule rows are stated, so a replay built on it sees the same values. Stamina, rank and colorful pass are seated by the server's named policies. Slice note: {}",
            doc.get("about").and_then(Value::as_str).unwrap_or("")
        ),
        Migration::CheckedInDefault => format!(
            "Product default: the checked-in server panel slice with the server clock on device time and a daily schedule that gives each refresh window the slice's phenomenon. A new user's stamina is seated from the masters (normal pool at its master maximum, enhance 0, boost one recovery grant); the rank follows totalExp {DEFAULT_TOTAL_EXP} through the master rank table; no colorful pass. Slice note: {}",
            doc.get("about").and_then(Value::as_str).unwrap_or("")
        ),
    };
    let document = ServerDocument {
        about,
        clock,
        stamina_refresh: StaminaRefresh::RefillNormal,
        gamedata: Gamedata {
            mysekai_rank: None,
            total_exp: DEFAULT_TOTAL_EXP,
            refreshed_at,
            is_mysekai_tutorial_end: boolean(
                gamedata,
                "isMysekaiTutorialEnd",
                "userMysekaiGamedata",
            )?,
        },
        stamina: None,
        colorful_pass: None,
        schedule_policy,
        schedules,
        gate,
        gate_characters,
        talk_histories,
        talk_list_policy,
        layouts: field(doc, "userMysekaiSiteHousingLayouts", "the slice")?.clone(),
        controls: field(doc, "controls", "the slice")?.clone(),
        delivery: super::delivery::DeliveryDoc::default(),
        home_action_reply: Default::default(),
        music_settings: Vec::new(),
        avatar: Default::default(),
        inventory: Default::default(),
    };
    document.check_structure()?;
    Ok(document)
}

// ---------------------------------------------------------------------------
// Checks and output
// ---------------------------------------------------------------------------

impl ServerDocument {
    /// The checks that need no master data.
    pub(crate) fn check_structure(&self) -> Result<(), String> {
        if let Some(stamina) = self.stamina {
            for (name, value) in [
                ("normalStamina", stamina.normal),
                ("enhanceStamina", stamina.enhance),
                ("boostStamina", stamina.boost),
            ] {
                if value < 0 {
                    return Err(format!("userMysekaiStamina.{name} = {value} is negative"));
                }
            }
        }
        if let Some(rank) = self.gamedata.mysekai_rank {
            if rank < 1 {
                return Err(format!(
                    "userMysekaiGamedata.mysekaiRank = {rank} is below 1"
                ));
            }
        }
        if self.gamedata.total_exp < 0 {
            return Err(format!(
                "userMysekaiGamedata.totalExp = {} is negative",
                self.gamedata.total_exp
            ));
        }
        if self.gamedata.refreshed_at < 0 {
            return Err(format!(
                "userMysekaiGamedata.refreshedAt = {} is negative",
                self.gamedata.refreshed_at
            ));
        }
        if let Some(pass) = self.colorful_pass {
            if pass.id < 1 {
                return Err(format!(
                    "userMysekaiColorfulPass.mysekaiColorfulPassId = {} is below 1",
                    pass.id
                ));
            }
        }
        if self.gate.gate_id < 1 || self.gate.skin_id < 0 || self.gate.level < 1 {
            return Err(format!(
                "userMysekaiGate (gate {}, skin {}, level {}) needs a gate id and level of at least 1 and a skin id of at least 0",
                self.gate.gate_id, self.gate.skin_id, self.gate.level
            ));
        }
        let mut groups = std::collections::HashSet::new();
        for (i, row) in self.gate_characters.iter().enumerate() {
            if row.unit_group_id < 1 {
                return Err(format!(
                    "userMysekaiGateCharacters[{i}].mysekaiGameCharacterUnitGroupId = {} is below 1",
                    row.unit_group_id
                ));
            }
            if row.visit_count < 0 {
                return Err(format!(
                    "userMysekaiGateCharacters[{i}].visitCount = {} is negative",
                    row.visit_count
                ));
            }
            if !groups.insert(row.unit_group_id) {
                return Err(format!(
                    "userMysekaiGateCharacters[{i}] repeats unit group {}",
                    row.unit_group_id
                ));
            }
        }
        self.delivery.check_structure()?;
        self.inventory.check_structure()
    }

    fn gamedata_value(&self) -> Value {
        json!({
            "mysekaiRank": self.gamedata.mysekai_rank,
            "totalExp": self.gamedata.total_exp,
            "refreshedAt": self.gamedata.refreshed_at,
            "isMysekaiTutorialEnd": self.gamedata.is_mysekai_tutorial_end,
        })
    }

    fn visit_value(&self, with_skin: bool) -> Value {
        let mut gate = json!({
            "mysekaiGateId": self.gate.gate_id,
            "mysekaiGateLevel": self.gate.level,
            "visitCount": self.gate.visit_count,
            "lastRefreshedAt": self.gate.last_refreshed_at,
            "isSettingAtHomeSite": self.gate.is_setting_at_home_site,
        });
        if with_skin {
            gate["mysekaiGateSkinId"] = json!(self.gate.skin_id);
        }
        let mut visit = json!({
            "userMysekaiGate": gate,
            "userMysekaiGateCharacters": self.gate_characters.iter().map(|row| json!({
                "mysekaiGateId": row.gate_id,
                "mysekaiGameCharacterUnitGroupId": row.unit_group_id,
                "isReservation": row.is_reservation,
                "visitCount": row.visit_count,
            })).collect::<Vec<_>>(),
        });
        if let Some(talks) = &self.talk_histories {
            visit["mysekaiCharacterTalkWithReadHistories"] = talks.clone();
        }
        visit
    }

    /// The schemaVersion 2 text, with the sections the next response will
    /// carry.
    pub(crate) fn to_value(&self, pending: &[String]) -> Value {
        let mut value = json!({
            "schemaVersion": SCHEMA_VERSION,
            "about": self.about,
            "clock": clock_value(self.clock),
            "policies": {"staminaRefresh": match self.stamina_refresh {
                StaminaRefresh::RefillNormal => REFILL_POLICY,
                StaminaRefresh::None => NO_REFILL_POLICY,
            }},
            "userMysekaiGamedata": self.gamedata_value(),
            "userMysekaiStamina": self.stamina.map(stamina_value),
            "userMysekaiColorfulPass": self.colorful_pass.map(|pass| json!({
                "mysekaiColorfulPassId": pass.id,
                "expiredAt": pass.expired_at,
            })),
            "phenomenaSchedulePolicy": match &self.schedule_policy {
                SchedulePolicy::Stated => json!({"kind": "stated"}),
                SchedulePolicy::Daily(map) => json!({
                    "kind": "daily",
                    "byRefreshTimePeriod": map.iter().map(|(period, id)| (period.to_string(), json!(id))).collect::<Map<_, _>>(),
                }),
            },
            "mysekaiPhenomenaSchedules": schedule_rows_value(&self.schedules),
            "userMysekaiGateCharacterVisit": self.visit_value(true),
            "userMysekaiSiteHousingLayouts": self.layouts,
            "controls": self.controls,
            "pending": pending,
        });
        if let Some(policy) = &self.talk_list_policy {
            value["talkListPolicy"] = policy.clone();
        }
        let mut policies = value["policies"].as_object().cloned().unwrap_or_default();
        if let Some(top) = value.as_object_mut() {
            self.delivery.write(top, &mut policies);
            super::home_action::write(self.home_action_reply, &mut policies);
            top.insert(
                super::music::SECTION.into(),
                super::client::music::rows_value(&self.music_settings),
            );
            top.insert(
                super::avatar::SECTION.into(),
                super::client::avatar::value(&self.avatar),
            );
            self.inventory.write(top, &mut policies);
            top.insert("policies".into(), Value::Object(policies));
        }
        value
    }

    /// The server panel's NPC slice (schemaVersion 1): the rows served now
    /// and the client's `refreshedAt`.
    #[allow(dead_code)] // Read by the owners' seams (harvest, menu, server panel, gate flow, site expansion).
    pub(crate) fn npc_slice(&self, served: &[ScheduleRow], refreshed_at: i64) -> Value {
        let mut value = json!({
            "version": V1,
            "about": "The server panel's NPC slice of the server document.",
            "userMysekaiGamedata": {
                "refreshedAt": refreshed_at,
                "isMysekaiTutorialEnd": self.gamedata.is_mysekai_tutorial_end,
            },
            "mysekaiPhenomenaSchedules": schedule_rows_value(served),
            "userMysekaiGateCharacterVisit": self.visit_value(false),
            "userMysekaiSiteHousingLayouts": self.layouts,
            "controls": self.controls,
        });
        if let Some(policy) = &self.talk_list_policy {
            value["talkListPolicy"] = policy.clone();
        }
        value
    }
}

pub(crate) fn clock_value(clock: Clock) -> Value {
    match clock {
        Clock::Device { offset_ms } => json!({"mode": "device", "offsetMs": offset_ms}),
        Clock::Fixed { at_ms } => json!({"mode": "fixed", "at": at_ms}),
    }
}

pub(crate) fn stamina_value(stamina: Stamina) -> Value {
    json!({
        "normalStamina": stamina.normal,
        "enhanceStamina": stamina.enhance,
        "boostStamina": stamina.boost,
    })
}

pub(crate) fn schedule_rows_value(rows: &[ScheduleRow]) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SLICE: &str = include_str!("../server_panel/npc.json");

    #[test]
    fn checked_in_slice_migrates_to_device_clock_and_daily_rows() {
        let doc = migrate_v1(SLICE, Migration::CheckedInDefault).unwrap();
        assert_eq!(doc.clock, Clock::Device { offset_ms: 0 });
        let SchedulePolicy::Daily(map) = &doc.schedule_policy else {
            panic!("not daily")
        };
        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&2), Some(&1));
        assert_eq!(doc.gamedata.mysekai_rank, None);
    }

    #[test]
    fn asset_source_slice_keeps_its_fixed_clock_and_rows() {
        let doc = migrate_v1(SLICE, Migration::AssetSource).unwrap();
        assert_eq!(
            doc.clock,
            Clock::Fixed {
                at_ms: 1_790_218_800_000
            }
        );
        assert_eq!(doc.schedule_policy, SchedulePolicy::Stated);
        assert_eq!(doc.schedules.len(), 6);
    }

    #[test]
    fn v2_text_round_trips() {
        let doc = migrate_v1(SLICE, Migration::CheckedInDefault).unwrap();
        let text = doc.to_value(&["userMysekaiStamina".to_owned()]).to_string();
        let (back, pending) = parse_v2(&text).unwrap();
        assert_eq!(back, doc);
        assert_eq!(pending, vec!["userMysekaiStamina".to_owned()]);
    }

    #[test]
    fn newer_schema_is_refused_by_name() {
        let error = parse_v2(r#"{"schemaVersion": 3}"#).unwrap_err();
        assert!(
            error.contains("schemaVersion 3 is not supported"),
            "{error}"
        );
    }

    #[test]
    fn unknown_field_is_refused_by_name() {
        let doc = migrate_v1(SLICE, Migration::CheckedInDefault).unwrap();
        let mut value = doc.to_value(&[]);
        value["surprise"] = json!(1);
        let error = parse_v2(&value.to_string()).unwrap_err();
        assert!(error.contains("unknown field surprise"), "{error}");
    }
}
