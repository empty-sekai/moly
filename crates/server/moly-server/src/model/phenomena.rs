//! `userMysekaiPhenomenas`: the phenomena the user has learned, and the
//! request that records one.
//!
//! **Document key** (the source's `SuiteUser` key, optional on read, always
//! written): `userMysekaiPhenomenas`, `UserMysekaiPhenomena` rows
//! (`mysekaiPhenomenaId`, `obtainedAt`), one per learned phenomenon. An
//! absent key is a user who has learned none; `[]` is the same, written.
//!
//! **Request**: `PostUserMysekaiReleaseApi` (`UserMysekaiReleaseRequest`:
//! `userId`, `mysekaiSiteId`), sent by `SiteEnvironmentUtility.ExecuteReleaseAPI`
//! when a harvest site's arrival learns its environment
//! (`LearnSiteEnvironment`). The reply is a `SuiteUser` answer; the client's
//! handler merges it with `UpdateAll`, which replaces its row list whole.
//!
//! **Named policy** (the server's rule is not in the client): the reply
//! carries `userMysekaiPhenomenas`. The learned phenomenon is today's
//! (`UserDataManager.TodayMysekaiPhenomenaId`): with the daily schedule, the
//! phenomenon of the refresh window that holds the server clock; with stated
//! rows, the phenomenon of the row whose `scheduleDate` is the local
//! midnight of the server clock's day (with no such row, the lowest
//! phenomenon the stated rows name, so a fixed-clock slice still learns one).
//! A clock outside every window learns
//! nothing, quietly, and the request is still answered. A phenomenon the
//! user has not learned joins the rows with `obtainedAt` at the server clock;
//! one already learned adds no row, so the reply's list says whether the call
//! learned a phenomenon.
//!
//! The site the request names does not enter the row (the rows have none)
//! and is not checked against the site master: the phenomenon is the
//! schedule's.
//!
//! The client's copy is [`ClientMysekaiPhenomena`]: the responses that carry
//! the section set it, the join included, and the release reply sets it
//! before the learn flow reads its rows.
//!
//! Native instrument: `MOLY_HARVEST_MOCK_PHENOMENA`
//! (`mysekaiPhenomenaId:obtainedAt`, comma separated) states the native
//! document's rows; game mode reads no environment variable.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde_json::{json, Map, Value};

use super::document::{SchedulePolicy, ServerDocument};
use super::{ResponseKind, ServerModel, Tables};

pub(crate) use moly_protocol::json::SECTION_PHENOMENA as SECTION;
pub(crate) use moly_protocol::json::phenomena_value as rows_value;
pub(crate) use moly_protocol::rows::UserMysekaiPhenomena;

/// The API's name in the source.
pub(crate) const API: &str = "PostUserMysekaiReleaseApi";
const INSTRUMENT: &str = "MOLY_HARVEST_MOCK_PHENOMENA";

/// This part of the server document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PhenomenaDoc {
    /// `userMysekaiPhenomenas` (empty: none learned, whichever way the
    /// document said so).
    pub(crate) rows: Vec<UserMysekaiPhenomena>,
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

fn parse_rows(value: &Value) -> Result<Vec<UserMysekaiPhenomena>, String> {
    let rows = moly_protocol::json::parse_phenomena(value)?;
    check_structure(&rows)?;
    Ok(rows)
}

/// A phenomenon id of at least 1 once per row, an `obtainedAt` that is not
/// negative.
fn check_structure(rows: &[UserMysekaiPhenomena]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for (i, row) in rows.iter().enumerate() {
        let at = format!("{SECTION}[{i}]");
        if row.mysekai_phenomena_id < 1 {
            return Err(format!(
                "{at}.mysekaiPhenomenaId = {} is below 1",
                row.mysekai_phenomena_id
            ));
        }
        if row.obtained_at < 0 {
            return Err(format!("{at}.obtainedAt = {} is negative", row.obtained_at));
        }
        if !ids.insert(row.mysekai_phenomena_id) {
            return Err(format!(
                "{at} repeats phenomenon {}",
                row.mysekai_phenomena_id
            ));
        }
    }
    Ok(())
}

impl PhenomenaDoc {
    /// This part from a schemaVersion 2 document (absent: none learned).
    pub(crate) fn parse(doc: &Map<String, Value>) -> Result<Self, String> {
        Ok(Self {
            rows: doc.get(SECTION).map_or(Ok(Vec::new()), parse_rows)?,
        })
    }

    pub(crate) fn check_structure(&self) -> Result<(), String> {
        check_structure(&self.rows)
    }

    pub(crate) fn write(&self, top: &mut Map<String, Value>) {
        top.insert(SECTION.into(), rows_value(&self.rows));
    }

    /// Whether the user has learned `phenomena_id`.
    pub(crate) fn contains(&self, phenomena_id: i32) -> bool {
        self.rows
            .iter()
            .any(|row| row.mysekai_phenomena_id == phenomena_id)
    }

    /// The rows as the client's copy reads them.
    pub(crate) fn view(&self) -> Vec<UserMysekaiPhenomena> {
        self.rows.clone()
    }
}

// ---------------------------------------------------------------------------
// The policy
// ---------------------------------------------------------------------------

impl ServerModel {
    /// The release policy: today's phenomenon joins the rows when it is not
    /// learned yet, and the reply carries the section.
    pub(crate) fn release(&mut self, site_id: i32, masters: &Tables<'_>) -> Result<(), String> {
        let now = self.now_ms();
        let today = self.today_phenomenon(now, masters);
        match today {
            Some(id) if !self.doc.phenomena.contains(id) => {
                self.doc.phenomena.rows.push(UserMysekaiPhenomena {
                    mysekai_phenomena_id: id,
                    obtained_at: now,
                });
                self.commit();
                info!(
                    "[server] {API}(site {site_id}): phenomenon {id} learned at {now}; {SECTION} rows {}",
                    self.doc.phenomena.rows.len()
                );
            }
            Some(id) => info!(
                "[server] {API}(site {site_id}): phenomenon {id} was learned already; no row added"
            ),
            None => info!(
                "[server] {API}(site {site_id}): the schedule names no phenomenon for the server clock {now}; no row added"
            ),
        }
        self.respond(ResponseKind::Release, false, &[SECTION], masters);
        Ok(())
    }

    /// `UserDataManager.TodayMysekaiPhenomenaId` at the server clock `now`
    /// (the module comment names the two schedules).
    fn today_phenomenon(&self, now: i64, masters: &Tables<'_>) -> Option<i32> {
        let offset = |at: i64| super::clock::device_utc_offset_ms(at);
        match &self.doc.schedule_policy {
            SchedulePolicy::Daily(map) => {
                let periods = masters.periods.as_ref()?;
                let start = super::clock::window_start(now, periods, offset)?;
                periods
                    .iter()
                    .find(|period| now - start < period.end_hour * HOUR_MS)
                    .and_then(|period| map.get(&period.id).copied())
            }
            SchedulePolicy::Stated => {
                let today = super::clock::local_today(now, offset)?;
                self.doc
                    .schedules
                    .iter()
                    .find(|row| row.schedule_date == today)
                    .map(|row| row.phenomena_id)
                    .or_else(|| self.doc.schedules.iter().map(|row| row.phenomena_id).min())
            }
        }
    }
}

const HOUR_MS: i64 = 3_600_000;

// ---------------------------------------------------------------------------
// Native instrument
// ---------------------------------------------------------------------------

/// `MOLY_HARVEST_MOCK_PHENOMENA` over the native document; a malformed entry
/// stops loudly.
pub(crate) fn native_overlay(doc: &mut ServerDocument) {
    let Some(raw) = super::instrument_env(INSTRUMENT) else {
        return;
    };
    let mut rows = Vec::new();
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let parts: Vec<Option<i64>> = entry
            .split(':')
            .map(|part| part.trim().parse::<i64>().ok())
            .collect();
        let [Some(id), Some(obtained_at)] = parts.as_slice() else {
            panic!("{INSTRUMENT} entry {entry:?} is not mysekaiPhenomenaId:obtainedAt");
        };
        let id = i32::try_from(*id).unwrap_or_else(|_| {
            panic!("{INSTRUMENT} entry {entry:?}: the phenomenon id does not fit a 32-bit integer")
        });
        rows.push(UserMysekaiPhenomena {
            mysekai_phenomena_id: id,
            obtained_at: *obtained_at,
        });
    }
    if let Err(reason) = check_structure(&rows) {
        panic!("{INSTRUMENT}={raw:?}: {reason}");
    }
    info!(
        "[server] native overlay: {INSTRUMENT} -> {SECTION} {:?}",
        rows
    );
    doc.phenomena = PhenomenaDoc { rows };
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// A panel edit of the section (`None`: not this part's path).
pub(crate) fn edit_path(
    phenomena: &mut PhenomenaDoc,
    parts: &[&str],
    value: &Value,
) -> Option<Result<(), String>> {
    match parts {
        [SECTION] => Some(parse_rows(value).map(|rows| *phenomena = PhenomenaDoc { rows })),
        _ => None,
    }
}

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": "phenomena",
        "title": "Learned phenomena",
        "delivery": "next-response",
        "fields": [{
            "path": SECTION,
            "type": "rows",
            "row": {"mysekaiPhenomenaId": "int", "obtainedAt": "epoch-ms"},
            "note": "the phenomena the user has learned (HasTodayEnvironment reads them; a harvest site's arrival learns today's once); absent: none learned",
        }],
    }])
}

pub(crate) fn schema_policies() -> Value {
    json!([{
        "name": "release",
        "rule": "PostUserMysekaiReleaseApi: today's phenomenon (the phenomenon of the refresh window holding the server clock, or of the stated row of the local day, the lowest stated phenomenon when no row is of that day) joins userMysekaiPhenomenas with obtainedAt at the server clock when it is not learned yet (a learned one adds no row); the reply is a SuiteUser answer carrying the section",
    }])
}
