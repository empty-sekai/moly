//! The mission screen's normal-mission reward call
//! (`PostUserMysekaiNormalMissionRewardApi`, `UserMysekaiNormalMissionRewardRequest`
//! and `UserMysekaiNormalMissionRewardResponse`).
//!
//! **Document keys** (the source's `SuiteUser` keys; optional on read, always
//! written):
//! - `userMysekaiNormalMissionSheet`: `UserMysekaiNormalMissionSheet`'s
//!   `mysekaiNormalMissionSheetId`, the sheet the user is on
//!   (`MysekaiMissionUtility.GetCurrentMissionSheetId`). 0 is the unseated
//!   state of a document that does not state one; the join seats the sheet of
//!   the lowest `seq` (`MysekaiMissionUtility.GetLastMissionSheet` reads the
//!   highest, so the first is the other end of the same order).
//! - `userMysekaiNormalMissions`: `UserMysekaiNormalMission` rows
//!   (`mysekaiNormalMissionId`, `progress`, `mysekaiMissionStatus`), one per
//!   mission the user has a row for. `mysekaiMissionStatus` is
//!   `not_achieved`, `achieved` or `received` (`MysekaiMissionStatus`).
//!
//! Which missions are achieved, and their progress, are server-decided
//! values with no client source: the panel edits the sheet and the rows
//! ([`edit_path`]), and the masters validate them ([`check_masters`]). A new
//! document is seated with one `not_achieved` row per mission of the first
//! sheet.
//!
//! **Named rules** (the server's side of the receive):
//! - *Receive*: every `mysekaiNormalMissionId` must be a master mission that
//!   is a row of the current sheet in status `achieved`; otherwise the whole
//!   call is refused with `normal_mission_refused` and nothing changes
//!   (single and bulk receive are one call). The rows are then marked
//!   `received`.
//! - *Rewards*: each received mission's `mysekaiNormalMissionRewards` rows, in
//!   `seq` order (`MysekaiMissionUtility.GetMissionRewards`). A `resource_box`
//!   reward yields every detail of its `mysekai_normal_mission_reward` box
//!   (`ResourceBoxUtility.GetResources`, purpose `MasterResourceBox.MYSEKAI_NORMAL_MISSION_REWARD`),
//!   granted through the same path the convert receive uses: a material or
//!   fixture within its possession limit, an item, blueprint or music record
//!   joined to its owned table; a type the document does not track is
//!   reported and not stored. A `mysekai_rank_obtained_exp` reward adds the
//!   `mysekaiNormalMissionRewardMysekaiRankObtainedExps` quantity its
//!   `externalId` names to `totalExp`. A possession or convert machine level
//!   reward raises that level to its `externalId`. A `mysekai_release_element`
//!   reward has no document table here and is reported only. A possession
//!   level reward's `externalId` is read as the level itself (an inference),
//!   and a material or fixture that does not fit the limit of the level the
//!   receive reaches is reported but not stored, as the convert receive
//!   stops at its first draw that does not fit; a grant whose limit the
//!   possession masters do not hold refuses the whole call by name.
//! - *Rank*: `mysekaiRank` follows `totalExp` through the master rank table
//!   (`MysekaiRankModel`), as every other server reply keeps it.
//! - *Sheet advance*: once the completion mission and every normal mission of
//!   the current sheet read `received`, the user moves to the sheet of the
//!   next `seq`, whose missions are seated `not_achieved` at progress 0. The
//!   last sheet stays current. The client's `MysekaiMissionService.IsCompleteSheet`
//!   (the request names the sheet's completion mission) is what starts the
//!   next sheet's dialog; the sheet itself is the server's.
//! - *Completion step*: after the received rows are marked and a sheet not
//!   complete is answered, the client re-reads the sheet
//!   (`ScreenLayerMysekaiMission.OnFinishReceiveDialog`), and the completion
//!   mission's row then reads `not_achieved` while the number of its normal
//!   missions that are achieved or received is below its `requirement1` and
//!   `achieved` at or above it
//!   (`MysekaiMissionUtility.CreateRecommendViewData`, counting
//!   `GetChildMissionProgressCount`). A `received` completion row is not
//!   moved, so a receive that named it (the bulk list names it, the per-cell
//!   call names only normal missions: `GetReceivableMissionAll`,
//!   `MysekaiMissionService.IsCompleteSheet`) keeps it `received`; a stored
//!   progress below the count is raised to it.
//!
//! Nothing changes until every check has passed: a refused call leaves the
//! rows, the sheet, the experience and the owned tables as stored.
//!
//! The reward resources are carried in `obtainedRewards`; the status rewards
//! reach the client through `updatedResources` (the gamedata and possession
//! levels), which its reward chain reads (`MysekaiMissionService.IsSekaiRankUp`,
//! `IsTracdeMachineLevelUp`, `IsMaterialPossessionLevelUp`,
//! `IsFixturePossessionLevelUp`). Whether the source's server also lists a
//! status reward in `obtainedRewards` is unverified: a status reward is not a
//! `UserResource` the client can draw.

use std::collections::BTreeSet;

use bevy::log::info;
use moly_protocol::api::UserMysekaiNormalMissionRewardResponse;
use moly_protocol::json;
use moly_protocol::rows::{
    UserMysekaiBlueprint, UserMysekaiFixture, UserMysekaiItem, UserMysekaiNormalMission,
    UserMysekaiNormalMissionSheet, UserResource, MISSION_ACHIEVED, MISSION_NOT_ACHIEVED,
    MISSION_RECEIVED,
};
use serde_json::{json as json_value, Map, Value};

use super::delivery::{SECTION_MATERIALS, SECTION_MYSEKAI_MATERIALS};
use super::inventory::{SECTION_BLUEPRINTS, SECTION_FIXTURES, SECTION_ITEMS};
use super::music_play::RECORDS_SECTION;
use super::{ResponseKind, ServerModel, Tables, SECTION_GAMEDATA};

/// `userMysekaiNormalMissionSheet`.
pub(crate) const SECTION_SHEET: &str = "userMysekaiNormalMissionSheet";
/// `userMysekaiNormalMissions`.
pub(crate) const SECTION_MISSIONS: &str = "userMysekaiNormalMissions";
/// The document keys this part reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 2] = [SECTION_SHEET, SECTION_MISSIONS];
/// `MasterResourceBox.MYSEKAI_NORMAL_MISSION_REWARD`: the resource box a
/// mission's `resource_box` reward names.
const BOX_PURPOSE: &str = "mysekai_normal_mission_reward";

/// This part of the server document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NormalMissionDoc {
    /// `userMysekaiNormalMissionSheet.mysekaiNormalMissionSheetId`; 0 while a
    /// document states no sheet (the join seats the first one).
    pub(crate) sheet_id: i32,
    /// `userMysekaiNormalMissions`.
    pub(crate) rows: Vec<UserMysekaiNormalMission>,
}

impl NormalMissionDoc {
    /// This part from a schemaVersion 2 document: each key optional (the
    /// unseated state when absent), each present one strict.
    pub(crate) fn parse(doc: &Map<String, Value>) -> Result<Self, String> {
        let mut out = Self::default();
        if let Some(value) = doc.get(SECTION_SHEET) {
            out.sheet_id = parse_sheet(value)?;
        }
        if let Some(value) = doc.get(SECTION_MISSIONS) {
            out.rows = parse_rows(value)?;
        }
        out.check_structure()?;
        Ok(out)
    }

    /// This part as a schemaVersion 2 document reads it, newly seated: the
    /// sheet and its rows as given.
    #[cfg(test)]
    pub(crate) fn new(sheet_id: i32, rows: Vec<UserMysekaiNormalMission>) -> Self {
        Self { sheet_id, rows }
    }

    pub(crate) fn check_structure(&self) -> Result<(), String> {
        if self.sheet_id < 0 {
            return Err(format!(
                "{SECTION_SHEET}.mysekaiNormalMissionSheetId = {} is negative",
                self.sheet_id
            ));
        }
        let mut ids = BTreeSet::new();
        for (i, row) in self.rows.iter().enumerate() {
            let at = format!("{SECTION_MISSIONS}[{i}]");
            if row.mysekai_normal_mission_id < 1 {
                return Err(format!(
                    "{at}.mysekaiNormalMissionId = {} is below 1",
                    row.mysekai_normal_mission_id
                ));
            }
            if row.progress < 0 {
                return Err(format!("{at}.progress = {} is negative", row.progress));
            }
            if !row.status_is_known() {
                return Err(format!(
                    "{at}.mysekaiMissionStatus = {:?} is not {MISSION_NOT_ACHIEVED:?}, {MISSION_ACHIEVED:?} or {MISSION_RECEIVED:?}",
                    row.mysekai_mission_status
                ));
            }
            if !ids.insert(row.mysekai_normal_mission_id) {
                return Err(format!(
                    "{at} repeats mission {}",
                    row.mysekai_normal_mission_id
                ));
            }
        }
        Ok(())
    }

    /// Writes this part's keys into the document object.
    pub(crate) fn write(&self, doc: &mut Map<String, Value>) {
        doc.insert(
            SECTION_SHEET.into(),
            json_value!({"mysekaiNormalMissionSheetId": self.sheet_id}),
        );
        doc.insert(SECTION_MISSIONS.into(), rows_value(&self.rows));
    }

    /// The row of `mission_id`, if the user has one.
    fn row(&self, mission_id: i32) -> Option<&UserMysekaiNormalMission> {
        self.rows
            .iter()
            .find(|row| row.mysekai_normal_mission_id == mission_id)
    }

    fn row_mut(&mut self, mission_id: i32) -> Option<&mut UserMysekaiNormalMission> {
        self.rows
            .iter_mut()
            .find(|row| row.mysekai_normal_mission_id == mission_id)
    }

    /// Seats one `not_achieved` row per mission of `sheet_id` that has none.
    pub(super) fn seat_sheet(
        &mut self,
        sheet_id: i32,
        missions: &moly_masters::mission::MissionMasters,
    ) {
        for mission in missions.sheet_missions(sheet_id) {
            if self.row(mission.id).is_none() {
                self.rows.push(UserMysekaiNormalMission {
                    mysekai_normal_mission_id: mission.id,
                    progress: 0,
                    mysekai_mission_status: MISSION_NOT_ACHIEVED.to_owned(),
                });
            }
        }
    }
}

/// `userMysekaiNormalMissionSheet`: the object holding the sheet id.
fn parse_sheet(value: &Value) -> Result<i32, String> {
    const AT: &str = SECTION_SHEET;
    let row = json::object(value, AT)?;
    json::only(row, &["mysekaiNormalMissionSheetId"], AT)?;
    json::int32(row, "mysekaiNormalMissionSheetId", AT)
}

/// `userMysekaiNormalMission` rows, refused by name when a row is not one.
fn parse_rows(value: &Value) -> Result<Vec<UserMysekaiNormalMission>, String> {
    const AT: &str = SECTION_MISSIONS;
    let rows = json::rows(value, AT)?;
    let mut seen = BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{AT}[{i}]");
            let row = json::object(row, &at)?;
            json::only(
                row,
                &[
                    "mysekaiNormalMissionId",
                    "progress",
                    "mysekaiMissionStatus",
                ],
                &at,
            )?;
            let id = json::int32(row, "mysekaiNormalMissionId", &at)?;
            if !seen.insert(id) {
                return Err(format!("{at} repeats mission {id}"));
            }
            let parsed = UserMysekaiNormalMission {
                mysekai_normal_mission_id: id,
                progress: json::int32(row, "progress", &at)?,
                mysekai_mission_status: row
                    .get("mysekaiMissionStatus")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{at}.mysekaiMissionStatus is not a string"))?
                    .to_owned(),
            };
            if !parsed.status_is_known() {
                return Err(format!(
                    "{at}.mysekaiMissionStatus = {:?} is not {MISSION_NOT_ACHIEVED:?}, {MISSION_ACHIEVED:?} or {MISSION_RECEIVED:?}",
                    parsed.mysekai_mission_status
                ));
            }
            Ok(parsed)
        })
        .collect()
}

/// `userMysekaiNormalMissions` as the document and the client write it.
pub(crate) fn rows_value(rows: &[UserMysekaiNormalMission]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                json_value!({
                    "mysekaiNormalMissionId": row.mysekai_normal_mission_id,
                    "progress": row.progress,
                    "mysekaiMissionStatus": row.mysekai_mission_status,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Master checks
// ---------------------------------------------------------------------------

/// The checks against the masters; an absent master skips its check.
pub(crate) fn check_masters(doc: &NormalMissionDoc, masters: &Tables<'_>) -> Result<(), String> {
    let Some(missions) = masters.mission() else {
        return Ok(());
    };
    let sheet_id = doc.sheet_id;
    let current = match sheet_id {
        0 => return Ok(()),
        id => missions.sheet(id).ok_or_else(|| {
            format!("{SECTION_SHEET}.mysekaiNormalMissionSheetId = {id} is not a master mission sheet")
        })?,
    };
    for (i, row) in doc.rows.iter().enumerate() {
        let at = format!("{SECTION_MISSIONS}[{i}]");
        let mission = missions.mission(row.mysekai_normal_mission_id).ok_or_else(|| {
            format!(
                "{at}.mysekaiNormalMissionId = {} is not a master mission",
                row.mysekai_normal_mission_id
            )
        })?;
        let sheet = missions.sheet(mission.sheet_id).ok_or_else(|| {
            format!(
                "{at}.mysekaiNormalMissionId = {} names sheet {}, which the master does not hold",
                mission.id, mission.sheet_id
            )
        })?;
        if sheet.seq > current.seq {
            return Err(format!(
                "{at}.mysekaiNormalMissionId = {} belongs to sheet {} (seq {}), after the current sheet {sheet_id} (seq {})",
                mission.id, sheet.id, sheet.seq, current.seq
            ));
        }
        if row.is_complete() && row.progress < mission.requirement1 {
            return Err(format!(
                "{at} is {} at progress {}, below the mission's requirement1 {}",
                row.mysekai_mission_status, row.progress, mission.requirement1
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Panel edits and schema
// ---------------------------------------------------------------------------

/// A panel edit of this part: the sections the next response carries; `None`
/// when the path is not this part's.
pub(crate) fn edit_path(
    doc: &mut NormalMissionDoc,
    masters: &Tables<'_>,
    parts: &[&str],
    path: &str,
    value: &Value,
) -> Option<Result<Vec<&'static str>, String>> {
    let result = match parts {
        [SECTION_SHEET] => parse_sheet(value).and_then(|sheet_id| {
            set_sheet(doc, masters, sheet_id, path)?;
            Ok(vec![SECTION_SHEET, SECTION_MISSIONS])
        }),
        [SECTION_SHEET, "mysekaiNormalMissionSheetId"] => (|| {
            let sheet_id = edit_int(value, path)?;
            set_sheet(doc, masters, sheet_id, path)?;
            Ok(vec![SECTION_SHEET, SECTION_MISSIONS])
        })(),
        [SECTION_MISSIONS] => parse_rows(value).map(|rows| {
            doc.rows = rows;
            vec![SECTION_MISSIONS]
        }),
        [SECTION_MISSIONS, id, key] => (|| {
            let id = id
                .parse::<i32>()
                .map_err(|_| format!("{path}: {id} is not a mission id"))?;
            let row = doc
                .row_mut(id)
                .ok_or_else(|| format!("{path}: {SECTION_MISSIONS} holds no row of mission {id}"))?;
            match *key {
                "progress" => row.progress = edit_int(value, path)?,
                "mysekaiMissionStatus" => {
                    let status = value
                        .as_str()
                        .ok_or_else(|| format!("{path} must be a status word"))?;
                    let parsed = UserMysekaiNormalMission {
                        mysekai_normal_mission_id: id,
                        progress: row.progress,
                        mysekai_mission_status: status.to_owned(),
                    };
                    if !parsed.status_is_known() {
                        return Err(format!(
                            "{path} = {status:?} is not {MISSION_NOT_ACHIEVED:?}, {MISSION_ACHIEVED:?} or {MISSION_RECEIVED:?}"
                        ));
                    }
                    row.mysekai_mission_status = status.to_owned();
                }
                other => return Err(format!("{path}: {other} is not a mission row field")),
            }
            Ok(vec![SECTION_MISSIONS])
        })(),
        _ => return None,
    };
    Some(result)
}

/// Sets the current sheet: a master sheet, whose missions get a
/// `not_achieved` row each if the user has none.
fn set_sheet(
    doc: &mut NormalMissionDoc,
    masters: &Tables<'_>,
    sheet_id: i32,
    path: &str,
) -> Result<(), String> {
    if sheet_id < 1 {
        return Err(format!("{path} = {sheet_id} is below 1"));
    }
    let missions = masters
        .mission()
        .ok_or_else(|| format!("{path} needs the mission masters, which this root lacks"))?;
    if missions.sheet(sheet_id).is_none() {
        return Err(format!("{path} = {sheet_id} is not a master mission sheet"));
    }
    doc.sheet_id = sheet_id;
    doc.seat_sheet(sheet_id, missions);
    Ok(())
}

fn edit_int(value: &Value, path: &str) -> Result<i32, String> {
    let raw = value
        .as_i64()
        .ok_or_else(|| format!("{path} must be an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{path} = {raw} does not fit a 32-bit integer"))
}

/// The panel's sections.
pub(crate) fn schema_sections(masters: &Tables<'_>) -> Value {
    let sheets: Vec<i32> = masters
        .mission()
        .map(|mission| mission.sheets_by_seq().into_iter().map(|s| s.id).collect())
        .unwrap_or_default();
    json_value!([
        {
            "key": "mysekaiNormalMission",
            "title": "Normal mission sheet and rows",
            "delivery": "next-response",
            "fields": [
                {"path": format!("{SECTION_SHEET}.mysekaiNormalMissionSheetId"), "type": "int",
                    "min": 1, "values": sheets,
                    "note": "the sheet the user is on; changing it seats a not_achieved row for every mission of that sheet the user has none of"},
                {"path": SECTION_MISSIONS, "type": "rows", "row": {
                    "mysekaiNormalMissionId": "int", "progress": "int",
                    "mysekaiMissionStatus": "text"},
                    "note": "one row per mission the user has; the status is not_achieved, achieved or received"},
                {"path": format!("{SECTION_MISSIONS}.<mysekaiNormalMissionId>.progress"),
                    "type": "int", "min": 0},
                {"path": format!("{SECTION_MISSIONS}.<mysekaiNormalMissionId>.mysekaiMissionStatus"),
                    "type": "enum", "values": [MISSION_NOT_ACHIEVED, MISSION_ACHIEVED, MISSION_RECEIVED]},
            ],
        },
    ])
}

/// The panel's named policy of this part.
pub(crate) fn schema_policies() -> Value {
    json_value!([
        {"name": "normal mission receive", "rule": "PostUserMysekaiNormalMissionRewardApi: every mysekaiNormalMissionId is a userMysekaiNormalMissions row of the current sheet with mysekaiMissionStatus achieved, or the whole call is refused and nothing changes; the reply marks the rows received, grants each mysekaiNormalMissionRewards row (a resource_box reward is every detail of its mysekai_normal_mission_reward box within the possession limits; a mysekai_rank_obtained_exp reward adds the mysekaiNormalMissionRewardMysekaiRankObtainedExps quantity to totalExp; a possession or convert machine level reward raises that level to its externalId), recomputes mysekaiRank from totalExp, keeps an achieved or received completion mission's progress at its achieved or received normal missions (never below its requirement1), and moves to the sheet of the next seq once the completion mission and every normal mission of the sheet are received"},
    ])
}

// ---------------------------------------------------------------------------
// The server's side
// ---------------------------------------------------------------------------

/// What one receive changed.
struct Applied {
    /// The sections the response carries.
    changed: Vec<&'static str>,
    /// The sheet the receive moved to, when the current one was complete.
    advanced: Option<i32>,
    /// What the document could not carry, for the log.
    untracked: Vec<String>,
}

/// The possession limits a receive grants within, read before it grants
/// anything: the level each limit is read at is the level the receive's
/// possession rewards raise to.
///
/// `MysekaiUserDataUtility.GetMaterialMaxCount` and `GetFixtureMaxCount` read
/// the possession masters; a receive whose grants would need a limit the
/// masters do not hold is refused by name instead of counting every grant as
/// over the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PossessionLimits {
    material_level: i32,
    fixture_level: i32,
    material: Option<i32>,
    fixture: Option<i32>,
}

impl PossessionLimits {
    /// The limits at the given levels.
    fn read(masters: &Tables<'_>, material_level: i32, fixture_level: i32) -> Self {
        let possession = masters.possession();
        Self {
            material_level,
            fixture_level,
            material: possession.material_max_count(material_level),
            fixture: possession.fixture_max_count(fixture_level),
        }
    }

    /// Whether the resources fit the limit their table is held within.
    fn holds(
        &self,
        resource: &UserResource,
        possession: i32,
        fixtures: i32,
    ) -> Result<bool, String> {
        match resource.resource_type.as_str() {
            "mysekai_material" => {
                let limit = self.material.ok_or_else(|| {
                    format!(
                        "the mysekaiMaterialPossessions master holds no row of level {}: the receive's material {} cannot be checked against the possession limit, so nothing is granted",
                        self.material_level, resource.resource_id
                    )
                })?;
                Ok(possession.saturating_add(resource.quantity) <= limit)
            }
            "mysekai_fixture" => {
                let limit = self.fixture.ok_or_else(|| {
                    format!(
                        "the mysekaiFixturePossessions master holds no row of level {}: the receive's fixture {} cannot be checked against the possession limit, so nothing is granted",
                        self.fixture_level, resource.resource_id
                    )
                })?;
                Ok(fixtures.saturating_add(resource.quantity) <= limit)
            }
            _ => Ok(true),
        }
    }
}

impl ServerModel {
    /// Seats a document that states no sheet: the first sheet by `seq`, a
    /// `not_achieved` row per mission of it. Returns whether it changed.
    pub(crate) fn seat_normal_missions(&mut self, masters: &Tables<'_>) -> bool {
        if self.doc.normal_mission.sheet_id != 0 {
            return false;
        }
        let Some(missions) = masters.mission() else {
            self.push_error(
                "userMysekaiNormalMissionSheet is not seated: the mission masters are absent".into(),
            );
            return false;
        };
        let Some(sheet) = missions.first_sheet() else {
            self.push_error(
                "userMysekaiNormalMissionSheet is not seated: the mission sheets master has no row"
                    .into(),
            );
            return false;
        };
        let sheet_id = sheet.id;
        self.doc.normal_mission.sheet_id = sheet_id;
        self.doc.normal_mission.seat_sheet(sheet_id, missions);
        info!(
            "[server] userMysekaiNormalMissionSheet seated at the first sheet {sheet_id}; {} not_achieved row(s)",
            self.doc.normal_mission.rows.len()
        );
        true
    }

    /// `PostUserMysekaiNormalMissionRewardApi`: one call for a single mission's
    /// reward and for the bulk receive. Every check runs before the first
    /// change; `Err` refuses without a change.
    pub(crate) fn normal_mission_reward(
        &mut self,
        mission_ids: &[i32],
        masters: &Tables<'_>,
    ) -> Result<UserMysekaiNormalMissionRewardResponse, String> {
        let missions = masters.mission().ok_or(
            "the mission masters are not seated: mysekaiNormalMissionSheets, mysekaiNormalMissions, mysekaiNormalMissionRewards and the rank-experience rows are needed",
        )?;
        let sheet_id = self.doc.normal_mission.sheet_id;
        if sheet_id == 0 {
            return Err(
                "the user has no current mission sheet (userMysekaiNormalMissionSheet)".into(),
            );
        }
        if missions.sheet(sheet_id).is_none() {
            return Err(format!(
                "userMysekaiNormalMissionSheet.mysekaiNormalMissionSheetId = {sheet_id} is not a master mission sheet"
            ));
        }
        if mission_ids.is_empty() {
            return Err("mysekaiNormalMissionIds is empty".into());
        }
        // Every id is a row of the current sheet in status achieved; the
        // whole call is refused otherwise, before anything changes.
        let mut ids = Vec::with_capacity(mission_ids.len());
        for id in mission_ids {
            if ids.contains(id) {
                return Err(format!("mysekaiNormalMissionIds repeats mission {id}"));
            }
            let mission = missions
                .mission(*id)
                .ok_or_else(|| format!("mission {id} is not a master mission"))?;
            let row = self
                .doc
                .normal_mission
                .row(*id)
                .ok_or_else(|| format!("mission {id} has no userMysekaiNormalMissions row"))?;
            if mission.sheet_id != sheet_id {
                return Err(format!(
                    "mission {id} belongs to sheet {} and the user is on sheet {sheet_id}",
                    mission.sheet_id
                ));
            }
            if row.mysekai_mission_status != MISSION_ACHIEVED {
                return Err(format!(
                    "mission {id} is {}; only an achieved mission can be received",
                    row.mysekai_mission_status
                ));
            }
            ids.push(*id);
        }
        // The reward rows of the received missions, in the request's order
        // and each mission's `seq` order.
        let mut resources = Vec::new();
        let mut exp = 0i32;
        let mut material_level = self.doc.inventory.material_possession_level;
        let mut fixture_level = self.doc.inventory.fixture_possession_level;
        let mut convert_level = self.doc.gamedata.mysekai_convert_fixture_level;
        let mut untracked = Vec::new();
        for id in &ids {
            for reward in missions.rewards_of(*id) {
                match reward.reward_type {
                    moly_masters::mission::RewardType::ResourceBox => {
                        let details = box_details(masters, reward.external_id).ok_or_else(|| {
                            format!(
                                "resourceBoxes does not hold the {BOX_PURPOSE} box {}, which mission {id}'s reward {} names",
                                reward.external_id, reward.id
                            )
                        })?;
                        for (kind, resource_id, level, quantity) in details {
                            resources.push(UserResource {
                                resource_type: kind.clone(),
                                resource_id: *resource_id,
                                resource_level: *level,
                                quantity: *quantity,
                            });
                        }
                    }
                    moly_masters::mission::RewardType::RankObtainedExp => {
                        let row = missions.rank_exp(reward.external_id).ok_or_else(|| {
                            format!(
                                "mysekaiNormalMissionRewardMysekaiRankObtainedExps holds no row {}, which mission {id}'s reward {} names",
                                reward.external_id, reward.id
                            )
                        })?;
                        exp = exp.saturating_add(row.quantity);
                    }
                    // The level the reward's `externalId` names, raised to
                    // when it is higher than the stored one (the document's
                    // level is always stated: a part that reads none starts
                    // at its default).
                    moly_masters::mission::RewardType::MaterialPossession => {
                        material_level = material_level.max(reward.external_id);
                    }
                    moly_masters::mission::RewardType::FixturePossession => {
                        fixture_level = fixture_level.max(reward.external_id);
                    }
                    moly_masters::mission::RewardType::ConvertFixtureLevel => {
                        convert_level = Some(convert_level.unwrap_or(0).max(reward.external_id));
                    }
                    moly_masters::mission::RewardType::ReleaseElement => {
                        untracked.push(format!(
                            "mysekai_release_element {} of mission {id}",
                            reward.external_id
                        ));
                    }
                }
            }
        }
        let total_exp = self.doc.gamedata.total_exp.saturating_add(exp);
        let rank = if exp == 0 {
            self.doc.gamedata.mysekai_rank
        } else {
            Some(masters.rank_of(total_exp).ok_or_else(|| {
                format!(
                    "mysekaiRank does not follow totalExp {total_exp}: the mysekaiRanks master has no row at or below it"
                )
            })?)
        };
        // The grants' checks, on the tables as they are before any grant:
        // every resource fits one limit (a material or fixture within its
        // possession, the other types have none here), so one grant does not
        // read another's. The types the owned tables do not carry are
        // reported only.
        let limits = PossessionLimits::read(masters, material_level, fixture_level);
        let possession = self.material_possession(masters).map_or(0, |row| row.quantity);
        let fixtures: i32 = self.doc.inventory.fixtures.iter().map(|row| row.quantity).sum();
        let mut over = Vec::new();
        for (at, resource) in resources.iter().enumerate() {
            if !limits.holds(resource, possession, fixtures)? {
                info!(
                    "[server] mission reward: {} {} x{} does not fit its possession limit; it is reported but not granted",
                    resource.resource_type, resource.resource_id, resource.quantity
                );
                over.push(at);
            }
        }
        // Every check passed: apply.
        let answer = self.normal_mission_apply(
            missions,
            sheet_id,
            &ids,
            &resources,
            &over,
            material_level,
            fixture_level,
            convert_level,
            total_exp,
            rank,
        )?;
        let Applied {
            changed,
            advanced,
            untracked: mut notes,
        } = answer;
        untracked.append(&mut notes);
        if !untracked.is_empty() {
            info!(
                "[server] PostUserMysekaiNormalMissionRewardApi: not carried as rewards: {untracked:?}"
            );
        }
        self.commit();
        self.respond(ResponseKind::NormalMissionReward, false, &changed, masters);
        // The reply's `updatedResources`: the tables the request changed,
        // through the same readers the responses use.
        let sections: Vec<String> = changed.iter().map(|section| (*section).to_owned()).collect();
        let mut updated = self.inventory_update(&sections, masters);
        if sections.iter().any(|section| section == SECTION_MYSEKAI_MATERIALS) {
            // A grant changes the possession the client derives beside the
            // materials themselves.
            updated.user_mysekai_material_possession = self.material_possession(masters);
        }
        if sections.iter().any(|section| section == RECORDS_SECTION) {
            updated.user_mysekai_music_records = Some(self.doc.music_records.clone());
        }
        // The reply always carries the gamedata: the client's reward chain
        // reads totalExp and the rank from it whichever reward the mission
        // gave.
        updated.user_mysekai_gamedata = Some(self.gamedata_row());
        updated.user_mysekai_normal_missions = Some(self.doc.normal_mission.rows.clone());
        updated.user_mysekai_normal_mission_sheet = Some(UserMysekaiNormalMissionSheet {
            mysekai_normal_mission_sheet_id: self.doc.normal_mission.sheet_id,
        });
        info!(
            "[server] PostUserMysekaiNormalMissionRewardApi: missions {ids:?} received; {} reward resource(s) reported, {} not granted, totalExp {total_exp}, sheet {}{}",
            resources.len(),
            over.len(),
            self.doc.normal_mission.sheet_id,
            match advanced {
                Some(next) => format!(" (advanced from sheet {sheet_id} to {next})"),
                None => String::new(),
            },
        );
        Ok(UserMysekaiNormalMissionRewardResponse {
            obtained_rewards: resources,
            updated_resources: updated,
        })
    }

    /// The receive's changes, once every check has passed: the rows read
    /// `received`, the completion mission's row counts them, the levels and
    /// the experience are stored, the rewards that fit are granted, and the
    /// sheet advances when it is complete. `over` names the resources (by
    /// their place in `resources`) that do not fit; they are reported but
    /// not stored.
    #[allow(clippy::too_many_arguments)]
    fn normal_mission_apply(
        &mut self,
        missions: &moly_masters::mission::MissionMasters,
        sheet_id: i32,
        ids: &[i32],
        resources: &[UserResource],
        over: &[usize],
        material_level: i32,
        fixture_level: i32,
        convert_level: Option<i32>,
        total_exp: i32,
        rank: Option<i32>,
    ) -> Result<Applied, String> {
        let mut applied = Applied {
            changed: Vec::new(),
            advanced: None,
            untracked: Vec::new(),
        };
        let now = self.now_ms();
        applied.changed = vec![SECTION_MISSIONS, SECTION_SHEET];
        if total_exp != self.doc.gamedata.total_exp
            || material_level != self.doc.inventory.material_possession_level
            || fixture_level != self.doc.inventory.fixture_possession_level
            || convert_level != self.doc.gamedata.mysekai_convert_fixture_level
            || rank != self.doc.gamedata.mysekai_rank
        {
            applied.changed.push(SECTION_GAMEDATA);
        }
        for id in ids {
            if let Some(row) = self.doc.normal_mission.row_mut(*id) {
                row.mysekai_mission_status = MISSION_RECEIVED.to_owned();
            }
        }
        // The completion step (`MysekaiMissionUtility.CreateRecommendViewData`,
        // counting `GetChildMissionProgressCount`) owns the completion
        // mission's status: the row reads `not_achieved` while the count is
        // below its `requirement1` and `achieved` at or above it. A row the
        // step owns that reads `received` keeps it, so a receive that named
        // the completion mission stays received; a stored progress below the
        // count is raised to the count.
        let counted = missions
            .child_missions(sheet_id)
            .into_iter()
            .filter(|mission| {
                self.doc
                    .normal_mission
                    .row(mission.id)
                    .is_some_and(|row| row.is_complete())
            })
            .count() as i32;
        if let Some(parent) = missions.parent_mission(sheet_id) {
            let status = match self.doc.normal_mission.row(parent.id) {
                Some(row) if row.mysekai_mission_status == MISSION_RECEIVED => None,
                Some(_) if counted >= parent.requirement1 => Some(MISSION_ACHIEVED.to_owned()),
                Some(row) if row.is_complete() => {
                    applied.untracked.push(format!(
                        "mission {} reads not_achieved: its {} achieved or received normal mission(s) are below its requirement1 {}",
                        parent.id, counted, parent.requirement1
                    ));
                    Some(MISSION_NOT_ACHIEVED.to_owned())
                }
                _ => None,
            };
            if let Some(status) = status {
                if let Some(row) = self.doc.normal_mission.row_mut(parent.id) {
                    row.progress = row.progress.max(counted);
                    row.mysekai_mission_status = status;
                }
            }
        }
        self.doc.inventory.material_possession_level = material_level;
        self.doc.inventory.fixture_possession_level = fixture_level;
        self.doc.gamedata.mysekai_convert_fixture_level = convert_level;
        self.doc.gamedata.total_exp = total_exp;
        self.doc.gamedata.mysekai_rank = rank;
        for (at, resource) in resources.iter().enumerate() {
            let fits = !over.contains(&at);
            let section = grant_mission_resource(&mut self.doc, resource, fits, now);
            match section {
                Some(section) => {
                    if !applied.changed.contains(&section) {
                        applied.changed.push(section);
                    }
                }
                None => applied.untracked.push(format!(
                    "{} {} x{}",
                    resource.resource_type, resource.resource_id, resource.quantity
                )),
            }
        }
        // The sheet advances once its completion mission and every normal
        // mission are received, and only while a later sheet exists.
        let complete = missions.sheet_missions(sheet_id).iter().all(|mission| {
            self.doc
                .normal_mission
                .row(mission.id)
                .is_some_and(|row| row.mysekai_mission_status == MISSION_RECEIVED)
        });
        if complete {
            if let Some(next) = missions.next_sheet(sheet_id) {
                let next = next.id;
                self.doc.normal_mission.sheet_id = next;
                self.doc.normal_mission.seat_sheet(next, missions);
                applied.advanced = Some(next);
                info!(
                    "[server] PostUserMysekaiNormalMissionRewardApi: sheet {sheet_id} is complete; the next sheet is {next}"
                );
            }
        }
        Ok(applied)
    }
}

/// Grants one mission reward resource into the document's owned tables, the
/// way the convert receive grants one: the material rows and their
/// possession, a fixture row (its texture the resource's level), an item, a
/// blueprint or a music record (owned once). Whether a material or a fixture
/// fits its possession limit is checked before this ([`PossessionLimits`]):
/// `fits` false stores nothing but still names the table the resource
/// belongs to. The other types have no limit here. `None` when the owned
/// tables have none for the type.
fn grant_mission_resource(
    doc: &mut super::document::ServerDocument,
    resource: &UserResource,
    fits: bool,
    now: i64,
) -> Option<&'static str> {
    if !fits {
        return match resource.resource_type.as_str() {
            "mysekai_material" => Some(SECTION_MYSEKAI_MATERIALS),
            "material" => Some(SECTION_MATERIALS),
            "mysekai_fixture" => Some(SECTION_FIXTURES),
            "mysekai_item" => Some(SECTION_ITEMS),
            "mysekai_blueprint" => Some(SECTION_BLUEPRINTS),
            "mysekai_music_record" => Some(RECORDS_SECTION),
            _ => None,
        };
    }
    match resource.resource_type.as_str() {
        "mysekai_material" => {
            *doc.delivery
                .mysekai_materials
                .entry(resource.resource_id)
                .or_insert(0) += resource.quantity;
            Some(SECTION_MYSEKAI_MATERIALS)
        }
        "material" => {
            *doc.delivery
                .materials
                .entry(resource.resource_id)
                .or_insert(0) += resource.quantity;
            Some(SECTION_MATERIALS)
        }
        "mysekai_fixture" => {
            let fixtures = &mut doc.inventory.fixtures;
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
            Some(SECTION_FIXTURES)
        }
        "mysekai_item" => {
            if resource.quantity < 1 {
                return Some(SECTION_ITEMS);
            }
            let items = &mut doc.inventory.items;
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
            Some(SECTION_ITEMS)
        }
        "mysekai_blueprint" => {
            let blueprints = &mut doc.inventory.blueprints;
            if !blueprints
                .iter()
                .any(|row| row.mysekai_blueprint_id == resource.resource_id)
            {
                blueprints.push(UserMysekaiBlueprint {
                    mysekai_blueprint_id: resource.resource_id,
                    obtained_at: now,
                });
            }
            Some(SECTION_BLUEPRINTS)
        }
        "mysekai_music_record" => {
            let records = &mut doc.music_records;
            if !records
                .iter()
                .any(|row| row.mysekai_music_record_id == resource.resource_id)
            {
                records.push(moly_protocol::rows::OwnedMusicRecord {
                    mysekai_music_record_id: resource.resource_id,
                    obtained_at: now,
                });
            }
            Some(RECORDS_SECTION)
        }
        // A type the owned tables do not carry: it is reported and not
        // granted.
        _ => None,
    }
}

/// The details of the `mysekai_normal_mission_reward` box `id` names, in
/// `seq` order, as `(resourceType, resourceId, resourceLevel, quantity)`.
/// The box table is the one the convert part seats (`resourceBoxes`, keyed by
/// purpose and id), so a mission's box and a convert's box are read alike.
fn box_details<'a>(masters: &'a Tables<'_>, id: i32) -> Option<&'a [(String, i32, i32, i32)]> {
    let convert = masters.convert.as_ref()?;
    let boxed = convert.box_details(BOX_PURPOSE, id)?;
    Some(boxed.details.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    // The server's own master tables, which `Tables` borrows.
    use crate::masters::Masters;
    use serde_json::json;

    fn doc_with(sheet_id: i32, rows: Value) -> Map<String, Value> {
        let mut doc = Map::new();
        doc.insert(
            SECTION_SHEET.into(),
            json!({"mysekaiNormalMissionSheetId": sheet_id}),
        );
        doc.insert(SECTION_MISSIONS.into(), rows);
        doc
    }

    /// The sheet and its rows read and write as the document holds them.
    #[test]
    fn the_sheet_and_its_rows_round_trip() {
        let mut doc = doc_with(
            54,
            json!([
                {"mysekaiNormalMissionId": 53, "progress": 2, "mysekaiMissionStatus": "achieved"},
                {"mysekaiNormalMissionId": 52, "progress": 0, "mysekaiMissionStatus": "not_achieved"},
            ]),
        );
        let parsed = NormalMissionDoc::parse(&doc).expect("the part reads");
        assert_eq!(parsed.sheet_id, 54);
        assert_eq!(parsed.rows.len(), 2);
        assert!(parsed.rows[0].is_complete());
        assert!(!parsed.rows[1].is_complete());
        parsed.write(&mut doc);
        assert_eq!(NormalMissionDoc::parse(&doc), Ok(parsed));
    }

    /// A document that states neither key parses as the unseated state (the
    /// join seats the first sheet).
    #[test]
    fn a_new_document_starts_unseated() {
        let parsed = NormalMissionDoc::parse(&Map::new()).expect("both keys are optional");
        assert_eq!(parsed, NormalMissionDoc::default());
        let mut doc = doc_with(1, json!([]));
        doc.remove(SECTION_SHEET);
        assert_eq!(NormalMissionDoc::parse(&doc).unwrap().sheet_id, 0);
    }

    /// A status word the client does not know, a repeated mission, a
    /// negative progress, a mission id below 1, a missing status and a
    /// negative sheet are refused by name.
    #[test]
    fn malformed_rows_are_refused() {
        for rows in [
            json!([{"mysekaiNormalMissionId": 1, "progress": 0, "mysekaiMissionStatus": "done"}]),
            json!([
                {"mysekaiNormalMissionId": 1, "progress": 0, "mysekaiMissionStatus": "achieved"},
                {"mysekaiNormalMissionId": 1, "progress": 1, "mysekaiMissionStatus": "achieved"},
            ]),
            json!([{"mysekaiNormalMissionId": 1, "progress": -1, "mysekaiMissionStatus": "achieved"}]),
            json!([{"mysekaiNormalMissionId": 0, "progress": 0, "mysekaiMissionStatus": "achieved"}]),
            json!([{"mysekaiNormalMissionId": 1, "progress": 0}]),
        ] {
            let doc = doc_with(1, rows.clone());
            assert!(NormalMissionDoc::parse(&doc).is_err(), "{rows} parsed");
        }
        let mut doc = doc_with(1, json!([]));
        doc.insert(
            SECTION_SHEET.into(),
            json!({"mysekaiNormalMissionSheetId": -1}),
        );
        assert!(NormalMissionDoc::parse(&doc).is_err());
    }

    /// The panel's paths: one row's progress and status. A path of another
    /// part is not this part's, and a row the document has none of, an
    /// unknown status and a sheet the masters do not hold are refused.
    #[test]
    fn edits_touch_one_row() {
        let mut doc = NormalMissionDoc::parse(&doc_with(
            1,
            json!([{"mysekaiNormalMissionId": 7, "progress": 2, "mysekaiMissionStatus": "not_achieved"}]),
        ))
        .unwrap();
        let seated = Masters::default();
        let masters = Tables::new(&seated, None);
        assert!(edit_path(&mut doc, &masters, &["other"], "other", &json!(1)).is_none());
        assert_eq!(
            edit_path(
                &mut doc,
                &masters,
                &[SECTION_MISSIONS, "7", "progress"],
                "userMysekaiNormalMissions.7.progress",
                &json!(3),
            ),
            Some(Ok(vec![SECTION_MISSIONS]))
        );
        assert_eq!(doc.rows[0].progress, 3);
        assert!(edit_path(
            &mut doc,
            &masters,
            &[SECTION_MISSIONS, "7", "mysekaiMissionStatus"],
            "userMysekaiNormalMissions.7.mysekaiMissionStatus",
            &json!("half"),
        )
        .expect("the row's path")
        .is_err());
        assert_eq!(
            edit_path(
                &mut doc,
                &masters,
                &[SECTION_MISSIONS, "7", "mysekaiMissionStatus"],
                "userMysekaiNormalMissions.7.mysekaiMissionStatus",
                &json!("received"),
            ),
            Some(Ok(vec![SECTION_MISSIONS]))
        );
        assert_eq!(doc.rows[0].mysekai_mission_status, MISSION_RECEIVED);
        assert!(edit_path(
            &mut doc,
            &masters,
            &[SECTION_MISSIONS, "8", "progress"],
            "userMysekaiNormalMissions.8.progress",
            &json!(1),
        )
        .expect("the row's path")
        .is_err());
        // The sheet needs the mission masters, which this table set lacks.
        assert!(edit_path(
            &mut doc,
            &masters,
            &[SECTION_SHEET, "mysekaiNormalMissionSheetId"],
            "userMysekaiNormalMissionSheet.mysekaiNormalMissionSheetId",
            &json!(2),
        )
        .expect("the sheet's path")
        .is_err());
    }
}
