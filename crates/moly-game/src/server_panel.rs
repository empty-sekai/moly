//! Server panel, NPC slice: the server-decided inputs NPC AI reads, in the
//! shapes the client receives them, filled from one named document.
//!
//! Only the server decides who visits, which talks are live and how they are
//! flagged as read, which phenomenon each refresh window shows, and the
//! server clock. The client code that consumes these values is ported
//! elsewhere; this module only stands in for the server and hands the client
//! its copies:
//!
//! - **Talk list.** The client keeps one list of `{talk id, is read}` rows
//!   and every NPC talk lottery reads it. Setting the list converts each row
//!   one to one, keeps the server's order and replaces the whole list. The
//!   client never flips a flag in its own copy: the read report goes to the
//!   server only, so a talk read in this session stays unread in the list
//!   until the server sends a new one.
//! - **Visitors.** Every gate-character row reaches the spawner, which reads
//!   only the row's unit group: groups expand to their non-zero unit fields
//!   in slot order, row after row, duplicates dropped. Neither the row's gate
//!   id nor the home gate record is read on that path.
//! - **Housing layouts.** The home site's starter placements (the compact
//!   layout the site restores when no layout of the player's is saved) are
//!   the document's housing layout block; the site and its layout owner hold
//!   until the document has installed them. Rows use the offline layout
//!   record's fields (package, master fixture id, texture, footprint corners,
//!   centre height, layout type bits, rotation); the source's anchor-and-
//!   degrees shape needs master grid sizes the panel does not carry. The
//!   default places the home gate (the visit block says it stands on the home
//!   site), so gate-dependent talk gates read a placed gate.
//! - **Phenomenon of the day.** The client picks the schedule row whose
//!   refresh window holds the server-stamped clock, once, when the schedule
//!   is written; no wall clock advances it. An empty schedule writes nothing,
//!   and a clock outside every window logs an error and keeps the previous
//!   phenomenon. This module is the only writer of the phenomenon request;
//!   the weather key, the browser weather dial and the document's optional
//!   advance control (a period in seconds after which the current row moves
//!   to the next phenomenon) edit the schedule row that is current instead of
//!   choosing a phenomenon themselves.
//!
//! Named mocks (server algorithms with no client code):
//! - The **talk-list policy** builds the list when a document does not state
//!   one. It keeps a master talk when every speaker is a visitor and its site
//!   group holds the site the list is built for, then applies the two
//!   condition types the client treats as always true: a visit-count row
//!   passes when its value is at most the speaker's visit count, and an
//!   event-story row passes when the document marks that episode as read.
//!   A fixture-condition row passes when that fixture is placed on the site.
//!   Rows of one talk combine as "any row passes". Phenomenon rows are left
//!   to the client. Every kept talk starts unread unless the server recorded
//!   its read. The list keeps the talk table's order. The talk-term start
//!   is not checked: the asset source carries no talk-term table.
//! - The **read report** handler records the talk as read on the server side
//!   and answers with no obtained resources.
//! - **Device time zone.** The client converts server timestamps to device
//!   local time. The browser supplies its own offset; the native host has no
//!   time-zone source and uses UTC. The checked-in default schedule covers
//!   three days of windows with the same phenomenon, so every offset from
//!   UTC-12 to UTC+14 resolves to it.
//!
//! The document comes from the asset source at `server-panel/npc.json` when
//! one is there (harness and owner states), else from the checked-in default
//! beside this module. The file is read through the asset source's own
//! reader, so its absence is the ordinary default case and reports nothing;
//! any other read failure is refused. A document that fails to parse, names
//! an unknown refresh window, or lists a talk the talk table does not hold is
//! refused loudly; no field has a silent default.

use std::collections::HashSet;

use bevy::asset::io::{AssetReaderError, AssetSourceId, Reader};
use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, IoTaskPool, Task};
use moly_assets::json::JsonAsset;
use serde_json::Value;

/// Asset source and path of a panel document that replaces the checked-in one.
const DOCUMENT_SOURCE: &str = "moly";
const DOCUMENT_FILE: &str = "server-panel/npc.json";

/// The checked-in default document.
const DEFAULT_DOCUMENT: &str = include_str!("server_panel/npc.json");

/// The phenomena index carries the refresh windows of the master table.
const PHENOMENA_INDEX: &str = "moly://phenomena/index.json";

const DOCUMENT_VERSION: u64 = 1;

// ---------------------------------------------------------------------------
// Source shapes
// ---------------------------------------------------------------------------

/// One row of the talk list: a talk id and its read flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TalkWithReadHistory {
    pub(crate) talk_id: i32,
    pub(crate) is_read: bool,
}

/// One visitor row of the gate visit block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateCharacter {
    pub(crate) gate_id: i32,
    pub(crate) unit_group_id: i32,
    pub(crate) is_reservation: bool,
    pub(crate) visit_count: i32,
}

/// The home gate record of the visit block. The NPC path never reads it;
/// it is kept so a document states the whole block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Gate {
    pub(crate) gate_id: i32,
    pub(crate) level: i32,
    pub(crate) visit_count: i32,
    pub(crate) last_refreshed_at: i64,
    pub(crate) is_setting_at_home_site: bool,
}

/// One schedule row: refresh window, day (epoch milliseconds) and phenomenon.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PhenomenaSchedule {
    pub(crate) refresh_time_period_id: i32,
    pub(crate) schedule_date: i64,
    pub(crate) phenomena_id: i32,
}

/// One refresh window of the master table (hours may pass 24).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RefreshTimePeriod {
    pub(crate) id: i32,
    pub(crate) start_hour: i64,
    pub(crate) end_hour: i64,
}

/// Which read event stories the talk-list policy counts as read.
#[derive(Clone, Debug)]
enum ReadEventStories {
    All,
    Episodes(HashSet<i32>),
}

/// Where the talk list of a document comes from.
#[derive(Clone, Debug)]
enum TalkListSource {
    /// The document states the server's list.
    Stated(Vec<TalkWithReadHistory>),
    /// The talk-list policy builds it.
    Policy { read_event_stories: ReadEventStories },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentOrigin {
    AssetSource,
    CheckedIn,
}

// ---------------------------------------------------------------------------
// The mock server
// ---------------------------------------------------------------------------

/// The mock server's state. Written by the document loader and by the named
/// handlers only; the client reads its own copies ([`TalkDataStore`],
/// [`VisitingCharacters`], the phenomenon request), never this.
#[derive(Resource)]
pub(crate) struct ServerPanel {
    origin: DocumentOrigin,
    gate: Gate,
    gate_characters: Vec<GateCharacter>,
    talk_list: TalkListSource,
    schedules: Vec<PhenomenaSchedule>,
    refreshed_at: i64,
    tutorial_end: bool,
    /// Talk ids the read-report handler recorded, in report order.
    reported_reads: Vec<i32>,
    /// The home site's starter placements (housing layout block rows).
    home_starter: Vec<Value>,
    /// Control: advance the current schedule row every this many seconds.
    schedule_advance_seconds: Option<f32>,
}

impl ServerPanel {
    /// Whether the tutorial is running, as the client reads the finished
    /// flag of the user game data.
    pub(crate) fn is_tutorial(&self) -> bool {
        !self.tutorial_end
    }

    /// The read-report handler: the server records the talk as read. The
    /// answer carries no obtained resources, so the client has nothing to
    /// merge. The client's own talk list is left as it is.
    pub(crate) fn report_talk_read(&mut self, talk_id: i32) {
        self.reported_reads.push(talk_id);
        info!(
            "[server-panel] read report: talk {talk_id} recorded on the server side ({} reports); the client's talk list keeps its flag until the server sends a new list",
            self.reported_reads.len()
        );
    }

    fn visit_count_of(&self, groups: &[(i32, Vec<u32>)], unit: u32) -> Option<i32> {
        self.gate_characters
            .iter()
            .zip(groups)
            .find(|(_, (_, units))| units.contains(&unit))
            .map(|(row, _)| row.visit_count)
    }
}

// ---------------------------------------------------------------------------
// The client's copies
// ---------------------------------------------------------------------------

/// The client's talk list. Every NPC talk lottery reads this list and no
/// other pool.
#[derive(Resource)]
pub(crate) struct TalkDataStore {
    list: Vec<TalkWithReadHistory>,
    /// Counts list replacements (a new list from the server).
    revision: u64,
}

impl TalkDataStore {
    /// Setting the list: one client row per server row, same order, the
    /// previous list replaced whole.
    fn set_talk_list(&mut self, rows: &[TalkWithReadHistory]) {
        self.list = rows.to_vec();
        self.revision += 1;
    }

    pub(crate) fn talk_list(&self) -> &[TalkWithReadHistory] {
        &self.list
    }

    /// The objective lottery's unread test: any row of the whole list not
    /// read, general talks and every character included.
    pub(crate) fn any_unread(&self) -> bool {
        self.list.iter().any(|row| !row.is_read)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Row count, unread count and a digest of the list: FNV-1a 64 over
    /// each row in order as the talk id (4 bytes, little endian) followed by
    /// the read flag (1 byte). A replay recomputes it from its own document.
    pub(crate) fn digest(&self) -> (usize, usize, u64) {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for row in &self.list {
            for byte in row
                .talk_id
                .to_le_bytes()
                .into_iter()
                .chain([row.is_read as u8])
            {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let unread = self.list.iter().filter(|row| !row.is_read).count();
        (self.list.len(), unread, hash)
    }
}

/// The visitors the spawner creates: the gate-character rows and their unit
/// ids after the client's expansion (row order, duplicates dropped).
#[derive(Resource)]
pub(crate) struct VisitingCharacters {
    units: Vec<u32>,
    /// (unit group, expanded units) per gate-character row, row order.
    groups: Vec<(i32, Vec<u32>)>,
}

impl VisitingCharacters {
    pub(crate) fn units(&self) -> &[u32] {
        &self.units
    }
}

/// A visitor's visit count, read from the visitor row whose group holds the
/// unit. How the client carries a row's count into an NPC's own count is not
/// traced; this is the row value.
pub(crate) fn visit_count_of(
    panel: &ServerPanel,
    visitors: &VisitingCharacters,
    unit: u32,
) -> Option<i32> {
    panel.visit_count_of(&visitors.groups, unit)
}

/// The phenomenon of the day as the client computed it from the schedule:
/// `None` until a schedule write selected a row.
#[derive(Resource, Default)]
pub(crate) struct TodayPhenomena {
    index: Option<usize>,
    phenomena_id: Option<i32>,
    /// The panel's first schedule write has run (whatever it selected).
    written: bool,
}

impl TodayPhenomena {
    /// The phenomenon the schedule write selected, if any.
    pub(crate) fn phenomena_id(&self) -> Option<i32> {
        self.phenomena_id
    }

    /// Whether the first schedule write has run.
    pub(crate) fn written(&self) -> bool {
        self.written
    }
}

/// A control edit of the schedule row that is current.
#[derive(Message, Clone, Copy, Debug)]
pub enum PhenomenaScheduleEdit {
    /// The next phenomenon of the loaded catalogue (the native key).
    Next,
    /// A named phenomenon (the browser dial).
    Set(i32),
}

// ---------------------------------------------------------------------------
// Document parsing
// ---------------------------------------------------------------------------

fn field<'a>(value: &'a Value, key: &str, at: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("{at} has no field {key}"))
}

fn int(value: &Value, key: &str, at: &str) -> Result<i64, String> {
    field(value, key, at)?
        .as_i64()
        .ok_or_else(|| format!("{at}.{key} is not an integer"))
}

fn int32(value: &Value, key: &str, at: &str) -> Result<i32, String> {
    let raw = int(value, key, at)?;
    i32::try_from(raw).map_err(|_| format!("{at}.{key} = {raw} does not fit a 32-bit integer"))
}

fn boolean(value: &Value, key: &str, at: &str) -> Result<bool, String> {
    field(value, key, at)?
        .as_bool()
        .ok_or_else(|| format!("{at}.{key} is not a boolean"))
}

fn rows<'a>(value: &'a Value, key: &str, at: &str) -> Result<&'a Vec<Value>, String> {
    field(value, key, at)?
        .as_array()
        .ok_or_else(|| format!("{at}.{key} is not an array"))
}

fn parse_document(text: &str, origin: DocumentOrigin) -> Result<ServerPanel, String> {
    let doc: Value =
        serde_json::from_str(text).map_err(|error| format!("not valid JSON: {error}"))?;
    let version = doc
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("the document has no integer version")?;
    if version != DOCUMENT_VERSION {
        return Err(format!(
            "document version {version} is not the supported version {DOCUMENT_VERSION}"
        ));
    }
    let gamedata = field(&doc, "userMysekaiGamedata", "document")?;
    let refreshed_at = int(gamedata, "refreshedAt", "userMysekaiGamedata")?;
    let tutorial_end = boolean(gamedata, "isMysekaiTutorialEnd", "userMysekaiGamedata")?;
    let schedules = rows(&doc, "mysekaiPhenomenaSchedules", "document")?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("mysekaiPhenomenaSchedules[{i}]");
            Ok(PhenomenaSchedule {
                refresh_time_period_id: int32(row, "mysekaiRefreshTimePeriodId", &at)?,
                schedule_date: int(row, "scheduleDate", &at)?,
                phenomena_id: int32(row, "mysekaiPhenomenaId", &at)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let visit = field(&doc, "userMysekaiGateCharacterVisit", "document")?;
    let gate_value = field(visit, "userMysekaiGate", "userMysekaiGateCharacterVisit")?;
    let gate = Gate {
        gate_id: int32(gate_value, "mysekaiGateId", "userMysekaiGate")?,
        level: int32(gate_value, "mysekaiGateLevel", "userMysekaiGate")?,
        visit_count: int32(gate_value, "visitCount", "userMysekaiGate")?,
        last_refreshed_at: int(gate_value, "lastRefreshedAt", "userMysekaiGate")?,
        is_setting_at_home_site: boolean(gate_value, "isSettingAtHomeSite", "userMysekaiGate")?,
    };
    let gate_characters = rows(visit, "userMysekaiGateCharacters", "userMysekaiGateCharacterVisit")?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("userMysekaiGateCharacters[{i}]");
            Ok(GateCharacter {
                gate_id: int32(row, "mysekaiGateId", &at)?,
                unit_group_id: int32(row, "mysekaiGameCharacterUnitGroupId", &at)?,
                is_reservation: boolean(row, "isReservation", &at)?,
                visit_count: int32(row, "visitCount", &at)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let layouts = rows(&doc, "userMysekaiSiteHousingLayouts", "document")?;
    let mut home_starter = None;
    for (i, layout) in layouts.iter().enumerate() {
        let at = format!("userMysekaiSiteHousingLayouts[{i}]");
        let site = field(layout, "siteType", &at)?
            .as_str()
            .ok_or_else(|| format!("{at}.siteType is not a string"))?;
        let content = field(layout, "content", &at)?
            .as_str()
            .ok_or_else(|| format!("{at}.content is not a string"))?;
        if site != "home_site" || content != "compact" {
            return Err(format!(
                "{at} is for {site} / {content}; only the home site's compact starter layout is read"
            ));
        }
        if home_starter.is_some() {
            return Err(format!("{at} repeats the home site's starter layout"));
        }
        home_starter = Some(rows(layout, "mysekaiFixtures", &at)?.clone());
    }
    let home_starter =
        home_starter.ok_or("userMysekaiSiteHousingLayouts has no home site starter layout")?;
    let controls = field(&doc, "controls", "document")?;
    let schedule_advance_seconds = match field(controls, "phenomenaScheduleAdvanceSeconds", "controls")? {
        Value::Null => None,
        value => {
            let seconds = value
                .as_f64()
                .ok_or("controls.phenomenaScheduleAdvanceSeconds is neither null nor a number")?;
            if !(seconds > 0.0 && seconds.is_finite()) {
                return Err(format!(
                    "controls.phenomenaScheduleAdvanceSeconds = {seconds} is not a positive number of seconds"
                ));
            }
            Some(seconds as f32)
        }
    };
    let stated = visit.get("mysekaiCharacterTalkWithReadHistories");
    let policy = doc.get("talkListPolicy");
    let talk_list = match (stated, policy) {
        (Some(_), Some(_)) => {
            return Err(
                "the document both states a talk list and asks the talk-list policy for one"
                    .into(),
            )
        }
        (None, None) => {
            return Err(
                "the document neither states a talk list nor asks the talk-list policy for one"
                    .into(),
            )
        }
        (Some(list), None) => {
            let list = list
                .as_array()
                .ok_or("mysekaiCharacterTalkWithReadHistories is not an array")?;
            let mut seen = HashSet::new();
            let mut rows = Vec::with_capacity(list.len());
            for (i, row) in list.iter().enumerate() {
                let at = format!("mysekaiCharacterTalkWithReadHistories[{i}]");
                let talk_id = int32(row, "mysekaiCharacterTalkId", &at)?;
                if !seen.insert(talk_id) {
                    return Err(format!("{at} repeats talk {talk_id}"));
                }
                rows.push(TalkWithReadHistory {
                    talk_id,
                    is_read: boolean(row, "isRead", &at)?,
                });
            }
            TalkListSource::Stated(rows)
        }
        (None, Some(policy)) => {
            let stories = field(policy, "readEventStoryEpisodes", "talkListPolicy")?;
            let read_event_stories = match stories {
                Value::String(word) if word == "all" => ReadEventStories::All,
                Value::Array(ids) => ReadEventStories::Episodes(
                    ids.iter()
                        .map(|id| {
                            id.as_i64()
                                .and_then(|id| i32::try_from(id).ok())
                                .ok_or("talkListPolicy.readEventStoryEpisodes holds a non-integer")
                        })
                        .collect::<Result<_, _>>()?,
                ),
                _ => {
                    return Err(
                        "talkListPolicy.readEventStoryEpisodes is neither \"all\" nor a list of episode ids"
                            .into(),
                    )
                }
            };
            TalkListSource::Policy { read_event_stories }
        }
    };
    Ok(ServerPanel {
        origin,
        gate,
        gate_characters,
        talk_list,
        schedules,
        refreshed_at,
        tutorial_end,
        reported_reads: Vec::new(),
        home_starter,
        schedule_advance_seconds,
    })
}

fn parse_periods(text: &str) -> Result<Vec<RefreshTimePeriod>, String> {
    let doc: Value =
        serde_json::from_str(text).map_err(|error| format!("not valid JSON: {error}"))?;
    rows(&doc, "refreshTimePeriods", "phenomena index")?
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("refreshTimePeriods[{i}]");
            Ok(RefreshTimePeriod {
                id: int32(row, "id", &at)?,
                start_hour: int(row, "startHour", &at)?,
                end_hour: int(row, "endHour", &at)?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Phenomenon of the day
// ---------------------------------------------------------------------------

/// Offset of device local time from UTC, in seconds, at an epoch second.
#[cfg(target_arch = "wasm32")]
fn device_utc_offset_seconds(epoch_seconds: i64) -> i64 {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
        epoch_seconds as f64 * 1000.0,
    ));
    // getTimezoneOffset is UTC minus local, in minutes.
    -(date.get_timezone_offset() as i64) * 60
}

/// The native host has no time-zone source; UTC stands in (named above).
#[cfg(not(target_arch = "wasm32"))]
fn device_utc_offset_seconds(_epoch_seconds: i64) -> i64 {
    0
}

/// What a schedule write selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TodayIndex {
    /// The schedule is empty: nothing is written.
    Unwritten,
    /// The row whose window holds the clock.
    Row(usize),
    /// No window holds the clock: an error is logged, nothing is written.
    NoWindow,
}

/// The row whose refresh window holds the server clock. Timestamps are
/// milliseconds; both are cut to whole seconds (truncating) and moved to
/// device local time. A window starts at the row's day shifted by
/// `startHour - hour of that local time` hours and ends likewise with
/// `endHour`, so a day's minutes carry into both edges and an end hour past
/// 24 runs into the next day. A row naming a window the master does not hold
/// is the source's null dereference and is an error.
fn today_index(
    schedules: &[PhenomenaSchedule],
    periods: &[RefreshTimePeriod],
    refreshed_at_ms: i64,
    offset: impl Fn(i64) -> i64,
) -> Result<TodayIndex, String> {
    if schedules.is_empty() {
        return Ok(TodayIndex::Unwritten);
    }
    let now_utc = refreshed_at_ms / 1000;
    let now = now_utc + offset(now_utc);
    for (index, row) in schedules.iter().enumerate() {
        let period = periods
            .iter()
            .find(|period| period.id == row.refresh_time_period_id)
            .ok_or_else(|| {
                format!(
                    "schedule row {index} names refresh window {} that the master does not hold",
                    row.refresh_time_period_id
                )
            })?;
        let base_utc = row.schedule_date / 1000;
        let base = base_utc + offset(base_utc);
        let hour = base.div_euclid(3600).rem_euclid(24);
        let start = base + (period.start_hour - hour) * 3600;
        let end = base + (period.end_hour - hour) * 3600;
        if start <= now && now < end {
            return Ok(TodayIndex::Row(index));
        }
    }
    Ok(TodayIndex::NoWindow)
}

/// A schedule write: select the row and cache its phenomenon. An empty
/// schedule leaves the cached value alone; a clock outside every window logs
/// an error and leaves it alone too.
fn write_schedule(
    panel: &ServerPanel,
    periods: &[RefreshTimePeriod],
    today: &mut TodayPhenomena,
) {
    today.written = true;
    match today_index(
        &panel.schedules,
        periods,
        panel.refreshed_at,
        device_utc_offset_seconds,
    ) {
        Ok(TodayIndex::Row(index)) => {
            today.index = Some(index);
            today.phenomena_id = Some(panel.schedules[index].phenomena_id);
            info!(
                "[server-panel] phenomenon of the day: schedule row {index} of {} (window {}, day {}) -> phenomenon {}",
                panel.schedules.len(),
                panel.schedules[index].refresh_time_period_id,
                panel.schedules[index].schedule_date,
                panel.schedules[index].phenomena_id
            );
        }
        Ok(TodayIndex::Unwritten) => {
            info!("[server-panel] the phenomena schedule is empty; the phenomenon of the day is not written");
        }
        Ok(TodayIndex::NoWindow) => {
            today.index = None;
            error!(
                "[server-panel] no refresh window of the {} schedule rows holds the server clock {}; the phenomenon of the day stays {:?}",
                panel.schedules.len(),
                panel.refreshed_at,
                today.phenomena_id
            );
        }
        Err(reason) => panic!("server panel schedule: {reason}"),
    }
}

// ---------------------------------------------------------------------------
// Talk-list policy (named mock)
// ---------------------------------------------------------------------------

struct PolicyInputs<'a> {
    visitors: &'a VisitingCharacters,
    panel: &'a ServerPanel,
    site_id: i32,
    placed_fixture_ids: &'a [i32],
    read_event_stories: &'a ReadEventStories,
}

fn policy_talk_list(
    tables: &crate::fixture_activity_data::FixtureActivityTables,
    inputs: &PolicyInputs<'_>,
) -> Result<Vec<TalkWithReadHistory>, String> {
    let visitors: HashSet<u32> = inputs.visitors.units.iter().copied().collect();
    let reported: HashSet<i32> = inputs.panel.reported_reads.iter().copied().collect();
    let mut list = Vec::new();
    for talk in tables.talk_list_facts()? {
        if talk.units.is_empty() || !talk.units.iter().all(|unit| visitors.contains(unit)) {
            continue;
        }
        let sites = talk
            .sites
            .ok_or_else(|| format!("talk {} names a site group the site-group table does not hold", talk.id))?;
        if !sites.contains(&inputs.site_id) {
            continue;
        }
        let speaker_visits = inputs.panel.visit_count_of(&inputs.visitors.groups, talk.units[0]);
        let mut any_row_passes = false;
        for (kind, value) in &talk.conditions {
            any_row_passes |= match *kind {
                "mysekai_character_visit_count" => {
                    speaker_visits.is_some_and(|visits| *value <= visits)
                }
                "read_event_story_episode_id" => match inputs.read_event_stories {
                    ReadEventStories::All => true,
                    ReadEventStories::Episodes(read) => read.contains(value),
                },
                "mysekai_fixture_id" => inputs.placed_fixture_ids.contains(value),
                "mysekai_phenomena_id" => true,
                other => {
                    return Err(format!(
                        "talk {} carries condition type {other}, which the talk-list policy has no rule for",
                        talk.id
                    ))
                }
            };
        }
        if any_row_passes {
            list.push(TalkWithReadHistory {
                talk_id: talk.id,
                is_read: reported.contains(&talk.id),
            });
        }
    }
    Ok(list)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct PanelRequests {
    document: DocumentRequest,
    periods: Handle<JsonAsset>,
}

/// The panel document: being read, or its text and origin.
enum DocumentRequest {
    Reading(Task<Result<Option<String>, String>>),
    Ready(String, DocumentOrigin),
}

/// Reads the document file from the asset source: `Ok(None)` when the source
/// has no such file.
async fn read_document(server: AssetServer) -> Result<Option<String>, String> {
    let source = server
        .get_source(AssetSourceId::from(DOCUMENT_SOURCE))
        .map_err(|error| error.to_string())?;
    let mut reader = match source
        .reader()
        .read(std::path::Path::new(DOCUMENT_FILE))
        .await
    {
        Ok(reader) => reader,
        Err(AssetReaderError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// The refresh windows, parsed once from the phenomena index.
#[derive(Resource)]
struct RefreshTimePeriods(Vec<RefreshTimePeriod>);

fn load(mut commands: Commands, server: Res<AssetServer>) {
    let reader = server.clone();
    commands.insert_resource(PanelRequests {
        document: DocumentRequest::Reading(
            IoTaskPool::get().spawn(async move { read_document(reader).await }),
        ),
        periods: server.load::<JsonAsset>(PHENOMENA_INDEX),
    });
}

/// The document from the asset source when it is there, else the checked-in
/// one; then the refresh windows; then the first schedule write.
fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    requests: Option<ResMut<PanelRequests>>,
    mut today: ResMut<TodayPhenomena>,
) {
    let Some(mut requests) = requests else {
        return;
    };
    if let DocumentRequest::Reading(task) = &mut requests.document {
        let Some(read) = block_on(future::poll_once(task)) else {
            return;
        };
        requests.document = match read {
            Ok(Some(text)) => DocumentRequest::Ready(text, DocumentOrigin::AssetSource),
            Ok(None) => {
                DocumentRequest::Ready(DEFAULT_DOCUMENT.to_owned(), DocumentOrigin::CheckedIn)
            }
            Err(error) => panic!(
                "server panel document {DOCUMENT_SOURCE}://{DOCUMENT_FILE} failed to load: {error}"
            ),
        };
    }
    let DocumentRequest::Ready(text, origin) = &requests.document else {
        return;
    };
    let (text, origin) = (text.clone(), *origin);
    if let LoadState::Failed(error) = server.load_state(&requests.periods) {
        panic!("phenomena index {PHENOMENA_INDEX} failed to load: {error:?}");
    }
    let Some(index) = jsons.get(&requests.periods) else {
        return;
    };
    let periods = parse_periods(&index.0)
        .unwrap_or_else(|reason| panic!("phenomena index refresh windows: {reason}"));
    let panel = parse_document(&text, origin).unwrap_or_else(|reason| {
        panic!("server panel document ({origin:?}) is refused: {reason}")
    });
    info!(
        "[server-panel] document ready ({}): gate {} level {} (home {}, visit count {}, refreshed {}), {} visitor rows, talk list {}, {} schedule rows, server clock {}, tutorial finished {}",
        match panel.origin {
            DocumentOrigin::AssetSource => "asset source",
            DocumentOrigin::CheckedIn => "checked-in default",
        },
        panel.gate.gate_id,
        panel.gate.level,
        panel.gate.is_setting_at_home_site,
        panel.gate.visit_count,
        panel.gate.last_refreshed_at,
        panel.gate_characters.len(),
        match &panel.talk_list {
            TalkListSource::Stated(rows) => format!("stated ({} rows)", rows.len()),
            TalkListSource::Policy { .. } => "built by the talk-list policy".to_owned(),
        },
        panel.schedules.len(),
        panel.refreshed_at,
        panel.tutorial_end,
    );
    let placed = crate::fixture::layouts::install_home_starter_records(&panel.home_starter)
        .unwrap_or_else(|reason| panic!("server panel housing layout is refused: {reason}"));
    info!(
        "[server-panel] housing layout: home site starter, {placed} rows: {}",
        serde_json::json!(panel
            .home_starter
            .iter()
            .map(|row| serde_json::json!([row["mysekaiFixtureId"], row["package"]]))
            .collect::<Vec<_>>())
    );
    if let Some(seconds) = panel.schedule_advance_seconds {
        info!("[server-panel] control: the current schedule row advances to the next phenomenon every {seconds} s");
    }
    write_schedule(&panel, &periods, &mut today);
    commands.insert_resource(RefreshTimePeriods(periods));
    commands.insert_resource(panel);
    commands.remove_resource::<PanelRequests>();
}

/// The visitor expansion, once the unit-group table is in.
fn boot_visitors(
    mut commands: Commands,
    panel: Option<Res<ServerPanel>>,
    tables: Option<Res<crate::fixture_activity_data::FixtureActivityTables>>,
    visitors: Option<Res<VisitingCharacters>>,
) {
    if visitors.is_some() {
        return;
    }
    let (Some(panel), Some(tables)) = (panel, tables) else {
        return;
    };
    let mut groups = Vec::with_capacity(panel.gate_characters.len());
    let mut units = Vec::new();
    for row in &panel.gate_characters {
        let expanded = tables
            .unit_ids_of_group(row.unit_group_id)
            .unwrap_or_else(|reason| panic!("server panel visitor row: {reason}"));
        for unit in &expanded {
            if !units.contains(unit) {
                units.push(*unit);
            }
        }
        groups.push((row.unit_group_id, expanded));
    }
    info!(
        "[server-panel] visitors: rows {:?} (gate, group, reservation, visit count) -> units {units:?}",
        panel
            .gate_characters
            .iter()
            .map(|row| (row.gate_id, row.unit_group_id, row.is_reservation, row.visit_count))
            .collect::<Vec<_>>()
    );
    commands.insert_resource(VisitingCharacters { units, groups });
}

/// The talk list for the booted site, set into the client's store once. A
/// stated list is checked against the talk table; the policy waits for the
/// booted site's placements.
fn boot_talk_list(
    mut commands: Commands,
    panel: Option<Res<ServerPanel>>,
    tables: Option<Res<crate::fixture_activity_data::FixtureActivityTables>>,
    visitors: Option<Res<VisitingCharacters>>,
    store: Option<Res<TalkDataStore>>,
    selection: Res<crate::site::SiteSelection>,
    placements: Res<crate::fixture::FixturePlacements>,
) {
    if store.is_some() {
        return;
    }
    let (Some(panel), Some(tables), Some(visitors)) = (panel, tables, visitors) else {
        return;
    };
    let (rows, source_word) = match &panel.talk_list {
        TalkListSource::Stated(rows) => {
            let absent: Vec<i32> = rows
                .iter()
                .map(|row| row.talk_id)
                .filter(|id| !tables.has_talk(*id))
                .collect();
            if !absent.is_empty() {
                panic!(
                    "server panel talk list names talks this asset source's talk table does not hold: {absent:?}"
                );
            }
            (rows.clone(), "stated by the document".to_owned())
        }
        TalkListSource::Policy { read_event_stories } => {
            if placements.site_id() == 0 || placements.site_type() != selection.site_type() {
                return; // the booted site's layout is not restored yet
            }
            let site_id = i32::try_from(placements.site_id()).expect("site id fits i32");
            let placed = placements.fixture_ids();
            let rows = policy_talk_list(
                &tables,
                &PolicyInputs {
                    visitors: &visitors,
                    panel: &panel,
                    site_id,
                    placed_fixture_ids: &placed,
                    read_event_stories,
                },
            )
            .unwrap_or_else(|reason| panic!("talk-list policy: {reason}"));
            (
                rows,
                format!(
                    "built by the talk-list policy for site {site_id} with placed fixtures {placed:?} (talk-term start not checked: no talk-term table in this asset source)"
                ),
            )
        }
    };
    let mut store = TalkDataStore {
        list: Vec::new(),
        revision: 0,
    };
    // The boot path sets the list only when the server sent at least one
    // row; otherwise the store keeps the empty list it starts with.
    if !rows.is_empty() {
        store.set_talk_list(&rows);
    }
    let (n, unread, digest) = store.digest();
    let json: Vec<Value> = store
        .talk_list()
        .iter()
        .map(|row| serde_json::json!([row.talk_id, row.is_read]))
        .collect();
    info!(
        "[server-panel] talk list set: {n} rows, {unread} unread, digest {digest:016x}, {source_word}"
    );
    info!(
        "[server-panel-talk-list] {}",
        serde_json::json!({ "v": 1, "revision": store.revision(), "digest": format!("{digest:016x}"), "rows": json })
    );
    commands.insert_resource(store);
}

/// The document's advance control: every period, one edit of the current
/// schedule row to the next phenomenon.
fn advance_schedule_control(
    time: Res<Time>,
    panel: Option<Res<ServerPanel>>,
    mut elapsed: Local<f32>,
    mut edits: MessageWriter<PhenomenaScheduleEdit>,
) {
    let Some(period) = panel.as_deref().and_then(|panel| panel.schedule_advance_seconds) else {
        return;
    };
    *elapsed += time.delta_secs();
    if *elapsed >= period {
        *elapsed = 0.0;
        edits.write(PhenomenaScheduleEdit::Next);
    }
}

/// Control edits of the current schedule row, then a new schedule write.
fn apply_schedule_edits(
    mut edits: MessageReader<PhenomenaScheduleEdit>,
    panel: Option<ResMut<ServerPanel>>,
    periods: Option<Res<RefreshTimePeriods>>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    mut today: ResMut<TodayPhenomena>,
) {
    let (Some(mut panel), Some(periods)) = (panel, periods) else {
        return;
    };
    for edit in edits.read() {
        let Some(index) = today.index else {
            warn!("[server-panel] schedule edit {edit:?} refused: no schedule row is current");
            continue;
        };
        let current = panel.schedules[index].phenomena_id;
        let next = match *edit {
            PhenomenaScheduleEdit::Set(id) => id,
            PhenomenaScheduleEdit::Next => {
                let mut ids: Vec<i32> = catalogue.0.iter().map(|option| option.id).collect();
                ids.sort_unstable();
                ids.dedup();
                if ids.len() < 2 {
                    continue;
                }
                match ids.iter().position(|id| *id == current) {
                    Some(at) => ids[(at + 1) % ids.len()],
                    None => ids[0],
                }
            }
        };
        panel.schedules[index].phenomena_id = next;
        info!(
            "[server-panel] schedule edit {edit:?}: row {index} phenomenon {current} -> {next}; the schedule is written again"
        );
        write_schedule(&panel, &periods.0, &mut today);
    }
}

/// The only writer of the phenomenon request: ask the weather chain for the
/// phenomenon of the day until it is the current one.
fn publish_today_phenomena(
    today: Res<TodayPhenomena>,
    catalogue: Res<crate::weather::PhenomenonCatalogue>,
    current: Res<crate::weather::CurrentPhenomenonId>,
    mut requests: MessageWriter<crate::weather::WeatherRequest>,
    mut refused: Local<Option<i32>>,
) {
    let Some(id) = today.phenomena_id else {
        return;
    };
    if catalogue.0.is_empty() {
        return; // the weather catalogue is still loading
    }
    if !catalogue.0.iter().any(|option| option.id == id) {
        if *refused != Some(id) {
            warn!("[server-panel] phenomenon {id} of the day is not in this asset source's phenomenon catalogue; the weather is not changed");
            *refused = Some(id);
        }
        return;
    }
    if current.0 != id {
        requests.write(crate::weather::WeatherRequest(id));
    }
}

pub(crate) struct ServerPanelPlugin;

impl Plugin for ServerPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PhenomenaScheduleEdit>()
            .add_message::<crate::weather::WeatherRequest>()
            .init_resource::<crate::weather::PhenomenonCatalogue>()
            .init_resource::<crate::weather::CurrentPhenomenonId>()
            .init_resource::<TodayPhenomena>()
            .add_systems(Startup, load)
            .add_systems(
                Update,
                (
                    parse,
                    boot_visitors,
                    boot_talk_list,
                    advance_schedule_control,
                    apply_schedule_edits,
                    publish_today_phenomena,
                )
                    .chain()
                    .after(crate::fixture_activity_data::parse),
            );
    }
}
