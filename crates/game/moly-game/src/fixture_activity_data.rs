//! Source tables of the furniture activities (character talks, conditions,
//! timelines, action points, pre-actions, fixtures, site groups and no-talk
//! actions) and their joins.
//!
//! This module prepares data; it does not start an animation or reserve an
//! instance. Missing data and rejected source conditions are distinct
//! outcomes, neither an implicit General-talk fallback.

#[path = "fixture_activity_catalog.rs"]
mod catalog;
pub(crate) use catalog::{ActivityKey, ActivityOrigin, ActivitySpec};

#[path = "fixture_talk_action_data.rs"]
mod talk_action;
pub(crate) use talk_action::{FixtureTalkActionInput, FixtureTalkActionPlan};

use std::collections::{HashMap, HashSet};

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;
use serde_json::Value;

const PATHS: [&str; 11] = [
    "moly://mysekai-character-talks.json",
    "moly://mysekai-game-character-unit-groups.json",
    "moly://mysekai-character-talk-conditions.json",
    "moly://mysekai-character-talk-condition-groups.json",
    "moly://mysekai-character-talk-fixture-timelines.json",
    "moly://mysekai-character-talk-action-points.json",
    "moly://mysekai-fixture-player-timelines.json",
    "moly://tweet-tables.json",
    "moly://mysekai-fixtures.json",
    "moly://site-groups.json",
    "moly://mysekai-character-talk-no-talk-fixture-actions.json",
];

#[derive(Resource)]
pub(crate) struct ActivityTableRequests(Vec<Handle<JsonAsset>>);

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ActivityTableRequests(
        PATHS
            .iter()
            .map(|path| server.load::<JsonAsset>(*path))
            .collect(),
    ));
}

pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    request: Option<Res<ActivityTableRequests>>,
) {
    let Some(request) = request else {
        return;
    };
    let mut documents = Vec::with_capacity(PATHS.len());
    for (path, handle) in PATHS.iter().zip(&request.0) {
        if let LoadState::Failed(error) = server.load_state(handle) {
            panic!("activity table {path} failed to load: {error:?}");
        }
        let Some(asset) = jsons.get(handle) else {
            return;
        };
        documents.push(
            serde_json::from_str::<Value>(&asset.0)
                .unwrap_or_else(|error| panic!("activity table {path}: {error}")),
        );
    }
    let tables = FixtureActivityTables::from_documents(&documents)
        .unwrap_or_else(|error| panic!("activity table relationships: {error}"));
    info!(
        "[fixture-activity] data ready: {} masters, {} timeline rows, {} pre-actions",
        tables.talks.rows.len(),
        tables.timelines.rows.len(),
        tables.pre_actions.len()
    );
    commands.insert_resource(tables);
    commands.remove_resource::<ActivityTableRequests>();
}

/// The fixture together-communication table (id -> the action-point row its
/// members use), read only by the fixture talk that waits with a
/// communication. It loads on its own: an asset source without it keeps every
/// other table, and that factory names the missing table when it is reached.
#[derive(Resource)]
pub(crate) struct TogetherCommunicationTable(pub(crate) Result<HashMap<i32, i32>, String>);

#[derive(Resource)]
pub(crate) struct TogetherCommunicationRequest(Handle<JsonAsset>);

const TOGETHER_COMMUNICATIONS: &str = "moly://mysekai-character-talk-fixture-together-communications.json";

pub(crate) fn load_together(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(TogetherCommunicationRequest(
        server.load::<JsonAsset>(TOGETHER_COMMUNICATIONS),
    ));
}

pub(crate) fn parse_together(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    request: Option<Res<TogetherCommunicationRequest>>,
) {
    let Some(request) = request else {
        return;
    };
    let table = if let LoadState::Failed(error) = server.load_state(&request.0) {
        Err(format!(
            "the fixture together-communication table is not in this asset source ({error:?})"
        ))
    } else {
        let Some(asset) = jsons.get(&request.0) else {
            return;
        };
        (|| {
            let value: Value = serde_json::from_str(&asset.0).map_err(|error| error.to_string())?;
            let table = ordered(&value, |row| {
                Ok((int(row, "id")?, int(row, "mysekaiCharacterTalkActionPointId")?))
            })?;
            Ok(table.rows.into_iter().collect::<HashMap<_, _>>())
        })()
        .map_err(|error: String| format!("fixture together-communication table: {error}"))
    };
    match &table {
        Ok(rows) => info!("[fixture-activity] together-communication table ready: {} rows", rows.len()),
        Err(reason) => warn!("[fixture-activity] {reason}"),
    }
    commands.insert_resource(TogetherCommunicationTable(table));
    commands.remove_resource::<TogetherCommunicationRequest>();
}

/// `rows` follows the document's rowOrder, never map iteration or numeric sort.
struct OrderedTable<T> {
    rows: Vec<T>,
    by_id: HashMap<i32, usize>,
}

impl<T> OrderedTable<T> {
    fn get(&self, id: i32) -> Option<&T> {
        self.by_id.get(&id).map(|index| &self.rows[*index])
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityMaster {
    pub id: i32,
    pub unit_group_id: i32,
    pub condition_group_id: i32,
    pub site_group_id: i32,
    pub term_id: i32,
    pub lua: String,
}

struct UnitGroup {
    id: i32,
    // Five source positions; missing fields retain their absence.
    units: [Option<i32>; 5],
}

struct Condition {
    id: i32,
    kind: String,
    value: i32,
}

struct ConditionGroupRow {
    group_id: i32,
    condition_id: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityTimeline {
    pub id: i32,
    pub group_id: i32,
    pub asset_name: String,
    pub action_point_definition: i32,
}

struct ActionPointDefinition {
    points: [Option<i32>; 4],
}

#[derive(Debug, Clone)]
pub(crate) struct PlayerTimelineRow {
    pub id: i32,
    pub fixture_id: i32,
    pub asset_name: String,
    pub action_point: i32,
}

/// Authored character-to-fixture visual relation. Using it for the selected
/// SD player appearance does not import NPC activity rules into player control.
#[derive(Debug, Clone)]
pub(crate) struct NoTalkVisualRow {
    pub id: i32,
    pub unit: u32,
    pub fixture_id: i32,
    pub timeline_group_id: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityPreAction {
    pub id: i32,
    pub talk_id: i32,
    pub tweet_id: i32,
    pub timeline_group_id: Option<i32>,
    pub fixture_together_communication_id: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PutType {
    None,
    Base,
    Target,
    Either,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityFixtureMaster {
    pub id: i32,
    pub model_name: String,
    pub grid_size: moly_law::fixture::Vector3Int,
    pub put_type: PutType,
    pub player_action_type: String,
    pub handle_type: String,
    /// The master's fixture type is the gate type.
    pub is_gate: bool,
}

#[derive(Resource)]
pub(crate) struct FixtureActivityTables {
    talks: OrderedTable<ActivityMaster>,
    units: OrderedTable<UnitGroup>,
    conditions: OrderedTable<Condition>,
    condition_groups: OrderedTable<ConditionGroupRow>,
    timelines: OrderedTable<ActivityTimeline>,
    action_points: OrderedTable<ActionPointDefinition>,
    player_timelines: OrderedTable<PlayerTimelineRow>,
    no_talk_visuals: OrderedTable<NoTalkVisualRow>,
    pre_actions: Vec<ActivityPreAction>,
    pre_action_by_talk: HashMap<i32, usize>,
    fixtures: HashMap<i32, ActivityFixtureMaster>,
    site_groups: HashMap<i32, Vec<i32>>,
    /// Condition ids of each condition group, in the group table's row order.
    conditions_by_group: HashMap<i32, Vec<i32>>,
}

/// Preparation errors are not candidate-pool emptiness. A caller must not turn
/// DataMissing/InvalidData/OtherActivity into a General selection.
#[derive(Debug, Clone)]
pub(crate) enum PrepareError {
    DataMissing(String),
    InvalidData(String),
    Rejected(&'static str),
    OtherActivity(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivityPose {
    pub position: [f32; 3],
    pub rotation: Quat,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityLocate {
    pub unit: u32,
    pub unit_slot: usize,
    pub action_point_value: i32,
    pub action_point_index: usize,
    pub start: ActivityPose,
    pub end: Option<ActivityPose>,
}

/// One master talk as the server panel's talk-list policy reads it. The
/// policy is a named mock of a server algorithm; these are client master
/// facts only, in the talk table's row order.
pub(crate) struct TalkListFacts<'a> {
    pub id: i32,
    /// The speaker group expanded the way the client expands it.
    pub units: Vec<u32>,
    /// Site ids of the talk's site group; `None` when the group is absent.
    pub sites: Option<&'a [i32]>,
    /// (condition type, condition value) of every row in the condition group.
    pub conditions: Vec<(&'a str, i32)>,
}

impl FixtureActivityTables {
    /// The client's group expansion: the five unit fields in slot order,
    /// zero fields skipped. A group missing from the table is the source's
    /// null dereference, so it is an error, never an empty cast.
    pub(crate) fn unit_ids_of_group(&self, group_id: i32) -> Result<Vec<u32>, String> {
        let group = self
            .units
            .get(group_id)
            .ok_or_else(|| format!("unit group {group_id} is not in the unit-group table"))?;
        group
            .units
            .iter()
            .filter_map(|unit| unit.filter(|unit| *unit != 0))
            .map(|unit| {
                u32::try_from(unit).map_err(|_| format!("unit group {group_id} holds unit {unit}"))
            })
            .collect()
    }

    /// Whether a talk id names a row of the loaded talk table.
    pub(crate) fn has_talk(&self, talk_id: i32) -> bool {
        self.talks.get(talk_id).is_some()
    }

    /// The master talk row of an id.
    pub(crate) fn talk_master(&self, talk_id: i32) -> Option<&ActivityMaster> {
        self.talks.get(talk_id)
    }

    /// The five unit fields of a unit group, in slot order; `None` when the
    /// group is not in the table.
    pub(crate) fn unit_group_slots(&self, group_id: i32) -> Option<[Option<i32>; 5]> {
        self.units.get(group_id).map(|group| group.units)
    }

    /// (condition type, value) of every condition of a talk's condition
    /// group, in the group table's row order. A group or condition the
    /// talk names and the tables lack is an error.
    pub(crate) fn talk_conditions(&self, talk_id: i32) -> Result<Vec<(&str, i32)>, String> {
        let master = self
            .talks
            .get(talk_id)
            .ok_or_else(|| format!("talk {talk_id} is not in the talk table"))?;
        let ids = self
            .conditions_by_group
            .get(&master.condition_group_id)
            .ok_or_else(|| {
                format!(
                    "talk {talk_id} names absent condition group {}",
                    master.condition_group_id
                )
            })?;
        ids.iter()
            .map(|id| {
                self.conditions
                    .get(*id)
                    .map(|condition| (condition.kind.as_str(), condition.value))
                    .ok_or_else(|| format!("talk {talk_id}: condition {id} is absent"))
            })
            .collect()
    }

    /// The pre-action row of a talk, if the talk has one.
    pub(crate) fn pre_action_of(&self, talk_id: i32) -> Option<&ActivityPreAction> {
        self.pre_action_by_talk
            .get(&talk_id)
            .map(|index| &self.pre_actions[*index])
    }

    /// Site ids of a site group; `None` when the group is absent.
    pub(crate) fn site_group_sites(&self, group_id: i32) -> Option<&[i32]> {
        self.site_groups.get(&group_id).map(Vec::as_slice)
    }

    /// Every master talk in row order, with the facts the talk-list policy
    /// filters on. A condition group or condition that a talk names and the
    /// tables do not hold is an error: the policy must not silently drop or
    /// keep such a talk. Groups no talk names are not resolved (shipped
    /// tables carry such groups whose conditions are absent).
    pub(crate) fn talk_list_facts(&self) -> Result<Vec<TalkListFacts<'_>>, String> {
        let mut by_group: HashMap<i32, Vec<i32>> = HashMap::new();
        for entry in &self.condition_groups.rows {
            by_group
                .entry(entry.group_id)
                .or_default()
                .push(entry.condition_id);
        }
        self.talks
            .rows
            .iter()
            .map(|master| {
                let condition_ids = by_group.get(&master.condition_group_id).ok_or_else(|| {
                    format!(
                        "talk {} names absent condition group {}",
                        master.id, master.condition_group_id
                    )
                })?;
                let conditions = condition_ids
                    .iter()
                    .map(|id| {
                        self.conditions
                            .get(*id)
                            .map(|condition| (condition.kind.as_str(), condition.value))
                            .ok_or_else(|| {
                                format!(
                                    "talk {} names condition group {} whose condition {id} is absent",
                                    master.id, master.condition_group_id
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(TalkListFacts {
                    id: master.id,
                    units: self.unit_ids_of_group(master.unit_group_id)?,
                    sites: self
                        .site_groups
                        .get(&master.site_group_id)
                        .map(Vec::as_slice),
                    conditions,
                })
            })
            .collect()
    }

    pub(crate) fn player_timelines(
        &self,
        fixture_id: i32,
    ) -> impl Iterator<Item = &PlayerTimelineRow> {
        self.player_timelines
            .rows
            .iter()
            .filter(move |row| row.fixture_id == fixture_id)
    }

    /// The full source row order, before the no-talk Guid-key permutation.
    pub(crate) fn no_talk_rows(&self) -> &[NoTalkVisualRow] {
        &self.no_talk_visuals.rows
    }

    /// CreateNoneTalkData uses the first group whose Unit1 is this actor,
    /// and the first timeline of its action group to assign locators. The
    /// later random timeline choice is independent of this assignment.
    pub(crate) fn no_talk_point(
        &self,
        unit: u32,
        first_timeline: &ActivityTimeline,
    ) -> Result<i32, PrepareError> {
        let group = self
            .units
            .rows
            .iter()
            .find(|group| group.units[0] == Some(unit as i32))
            .ok_or_else(|| {
                PrepareError::DataMissing(format!("no source Unit1 group for NPC {unit}"))
            })?;
        if group
            .units
            .iter()
            .skip(1)
            .any(|unit| unit.is_some_and(|unit| unit != 0))
        {
            return Err(PrepareError::OtherActivity(
                "source no-talk unit group needs multiple actor ownership",
            ));
        }
        self.action_point_value(first_timeline.action_point_definition, 0)?
            .filter(|point| *point != 0)
            .ok_or(PrepareError::Rejected(
                "source no-talk actor has no action-point assignment",
            ))
    }

    pub(crate) fn timeline_rows(&self, group_id: i32) -> impl Iterator<Item = &ActivityTimeline> {
        self.timelines
            .rows
            .iter()
            .filter(move |row| row.group_id == group_id)
    }

    pub(crate) fn action_point_value(
        &self,
        definition: i32,
        unit_slot: usize,
    ) -> Result<Option<i32>, PrepareError> {
        let row = self
            .action_points
            .get(definition)
            .ok_or_else(|| missing("action point", definition))?;
        row.points.get(unit_slot).copied().ok_or_else(|| {
            PrepareError::InvalidData("action point unit slot exceeds source definition".into())
        })
    }

    /// The non-zero action points of an action-point row in member-slot
    /// order, as the source's action-point dictionary enumerates them; `None`
    /// when the row is absent. The fifth member slot is not carried: it is 0
    /// on every shipped row.
    pub(crate) fn action_points_of(&self, definition: i32) -> Option<Vec<i32>> {
        let row = self.action_points.get(definition)?;
        Some(row.points.iter().flatten().copied().filter(|point| *point != 0).collect())
    }

    /// The member-slot columns of an action-point row (`gameCharacterUnitId{n}
    /// ActionPoint`, n = 1..4; an absent column is `None`); `None` when the row
    /// is absent. The source's fifth column is absent from every shipped row.
    pub(crate) fn action_point_columns(&self, definition: i32) -> Option<[Option<i32>; 4]> {
        self.action_points.get(definition).map(|row| row.points)
    }

    /// A fixture-timeline row by id.
    pub(crate) fn timeline(&self, id: i32) -> Option<&ActivityTimeline> {
        self.timelines.get(id)
    }

    pub(crate) fn fixture_master(&self, id: i32) -> Option<&ActivityFixtureMaster> {
        self.fixtures.get(&id)
    }

    /// Legacy offline layout seeds sometimes name only a model. Resolve that
    /// seed only when the source master has exactly one such row; variants
    /// sharing a model still need their explicit saved master identity.
    pub(crate) fn unique_fixture_master_for_model(
        &self,
        model: &str,
    ) -> Option<&ActivityFixtureMaster> {
        let mut rows = self.fixtures.values().filter(|row| row.model_name == model);
        let row = rows.next()?;
        rows.next().is_none().then_some(row)
    }

    fn from_documents(d: &[Value]) -> Result<Self, String> {
        let talks = ordered(&d[0], |row| {
            Ok(ActivityMaster {
                id: int(row, "id")?,
                unit_group_id: int(row, "mysekaiGameCharacterUnitGroupId")?,
                condition_group_id: int(row, "mysekaiCharacterTalkConditionGroupId")?,
                site_group_id: int(row, "mysekaiSiteGroupId")?,
                term_id: int(row, "mysekaiCharacterTalkTermId")?,
                lua: string(row, "lua")?,
            })
        })?;
        let units = ordered(&d[1], |row| {
            Ok(UnitGroup {
                id: int(row, "id")?,
                units: [
                    optional_int(row, "gameCharacterUnitId1")?,
                    optional_int(row, "gameCharacterUnitId2")?,
                    optional_int(row, "gameCharacterUnitId3")?,
                    optional_int(row, "gameCharacterUnitId4")?,
                    optional_int(row, "gameCharacterUnitId5")?,
                ],
            })
        })?;
        let conditions = ordered(&d[2], |row| {
            Ok(Condition {
                id: int(row, "id")?,
                kind: string(row, "mysekaiCharacterTalkConditionType")?,
                value: int(row, "mysekaiCharacterTalkConditionTypeValue")?,
            })
        })?;
        let condition_groups = ordered(&d[3], |row| {
            Ok(ConditionGroupRow {
                group_id: int(row, "groupId")?,
                condition_id: int(row, "mysekaiCharacterTalkConditionId")?,
            })
        })?;
        let timelines = ordered(&d[4], |row| {
            Ok(ActivityTimeline {
                id: int(row, "id")?,
                group_id: int(row, "groupId")?,
                asset_name: string(row, "assetbundleName")?,
                action_point_definition: int(row, "mysekaiCharacterTalkActionPointId")?,
            })
        })?;
        let action_points = ordered(&d[5], |row| {
            Ok(ActionPointDefinition {
                points: [
                    optional_int(row, "gameCharacterUnitId1ActionPoint")?,
                    optional_int(row, "gameCharacterUnitId2ActionPoint")?,
                    optional_int(row, "gameCharacterUnitId3ActionPoint")?,
                    optional_int(row, "gameCharacterUnitId4ActionPoint")?,
                ],
            })
        })?;
        let player_timelines = ordered(&d[6], |row| {
            Ok(PlayerTimelineRow {
                id: int(row, "id")?,
                fixture_id: int(row, "mysekaiFixtureId")?,
                asset_name: string(row, "assetbundleName")?,
                action_point: int(row, "actionPoint")?,
            })
        })?;
        let mut pre_actions = Vec::new();
        let mut pre_action_by_talk = HashMap::new();
        for row in array(&d[7], "talkPreActions")? {
            for field in ["timelineGroupId", "fixtureTogetherCommunicationId"] {
                if row.get(field).is_none() {
                    return Err(format!("pre-action export does not carry {field}"));
                }
            }
            let value = ActivityPreAction {
                id: int(row, "id")?,
                talk_id: int(row, "talkId")?,
                tweet_id: int(row, "tweetId")?,
                timeline_group_id: optional_int(row, "timelineGroupId")?,
                fixture_together_communication_id: optional_int(
                    row,
                    "fixtureTogetherCommunicationId",
                )?,
            };
            if pre_action_by_talk
                .insert(value.talk_id, pre_actions.len())
                .is_some()
            {
                return Err(format!("duplicate pre-action talk {}", value.talk_id));
            }
            pre_actions.push(value);
        }
        let mut fixtures = HashMap::new();
        for row in array(&d[8], "fixtures")? {
            let id = int(row, "id")?;
            let put_type = match string(row, "putType")?.as_str() {
                "none" => PutType::None,
                "put_base" => PutType::Base,
                "put_target" => PutType::Target,
                "put_either" => PutType::Either,
                other => return Err(format!("unknown put type {other}")),
            };
            if fixtures
                .insert(
                    id,
                    ActivityFixtureMaster {
                        id,
                        model_name: string(row, "assetbundleName")?,
                        grid_size: moly_law::fixture::Vector3Int::new(
                            int(row, "gridWidth")?,
                            int(row, "gridHeight")?,
                            int(row, "gridDepth")?,
                        ),
                        put_type,
                        player_action_type: string(row, "playerActionType")?,
                        handle_type: string(row, "handleType")?,
                        is_gate: string(row, "fixtureType")? == "gate",
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate fixture {id}"));
            }
        }
        let mut site_groups = HashMap::new();
        for row in array(&d[9], "groups")? {
            let id = int(row, "siteGroupId")?;
            let sites = array(row, "sites")?
                .iter()
                .map(value_int)
                .collect::<Result<_, _>>()?;
            if site_groups.insert(id, sites).is_some() {
                return Err(format!("duplicate site group {id}"));
            }
        }
        let no_talk_visuals = ordered(&d[10], |row| {
            Ok(NoTalkVisualRow {
                id: int(row, "id")?,
                unit: u32::try_from(int(row, "gameCharacterUnitId")?)
                    .map_err(|_| "negative visual unit")?,
                fixture_id: int(row, "mysekaiFixtureId")?,
                timeline_group_id: int(row, "mysekaiCharacterTalkFixtureTimelineGroupId")?,
            })
        })?;
        let mut conditions_by_group: HashMap<i32, Vec<i32>> = HashMap::new();
        for entry in &condition_groups.rows {
            conditions_by_group
                .entry(entry.group_id)
                .or_default()
                .push(entry.condition_id);
        }
        Ok(Self {
            talks,
            units,
            conditions,
            condition_groups,
            conditions_by_group,
            timelines,
            action_points,
            player_timelines,
            no_talk_visuals,
            pre_actions,
            pre_action_by_talk,
            fixtures,
            site_groups,
        })
    }
}

fn missing(table: &str, id: i32) -> PrepareError {
    PrepareError::DataMissing(format!("missing {table} {id}"))
}

fn ordered<T>(
    value: &Value,
    mut parse: impl FnMut(&Value) -> Result<T, String>,
) -> Result<OrderedTable<T>, String> {
    let entries = value
        .get("entries")
        .and_then(Value::as_object)
        .ok_or("table entries must be an object")?;
    let order = array(value, "rowOrder")?;
    let mut rows = Vec::with_capacity(order.len());
    let mut by_id = HashMap::with_capacity(order.len());
    let mut seen = HashSet::new();
    for entry in order {
        let id = value_int(entry)?;
        if !seen.insert(id) {
            return Err(format!("duplicate rowOrder id {id}"));
        }
        let row = entries
            .get(&id.to_string())
            .ok_or_else(|| format!("rowOrder references absent id {id}"))?;
        if int(row, "id")? != id {
            return Err(format!("entry id mismatch {id}"));
        }
        by_id.insert(id, rows.len());
        rows.push(parse(row)?);
    }
    if entries.len() != rows.len() {
        return Err("entries not covered by rowOrder".into());
    }
    Ok(OrderedTable { rows, by_id })
}

fn value_int(value: &Value) -> Result<i32, String> {
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| "expected i32 integer".into())
}
fn int(value: &Value, key: &str) -> Result<i32, String> {
    value
        .get(key)
        .ok_or_else(|| format!("missing integer {key}"))
        .and_then(value_int)
}
fn optional_int(value: &Value, key: &str) -> Result<Option<i32>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value_int(value).map(Some),
    }
}
fn string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array {key}"))
}

/// Extract the substring between the first and fourth underscores, exactly
/// as the factory does; the timeline can intentionally use another model's
/// compatible template. Model identity is not a shortcut for this parsing.
fn timeline_template(logical: &str) -> Result<&str, PrepareError> {
    let underscores: Vec<_> = logical
        .match_indices('_')
        .take(4)
        .map(|(index, _)| index)
        .collect();
    if underscores.len() != 4 {
        return Err(PrepareError::InvalidData(format!(
            "invalid timeline asset name {logical}"
        )));
    }
    Ok(&logical[underscores[0] + 1..underscores[3]])
}

/// Source asset-name package routing, without applying any NPC placement
/// variant rule to the player's original timeline or its sound companions.
pub(crate) fn timeline_package(logical: &str) -> Result<String, PrepareError> {
    Ok(format!(
        "mysekai__fixture_timeline__mdl_{}",
        timeline_template(logical)?
    ))
}

pub(crate) fn timeline_asset(
    logical: &str,
    put_type: PutType,
    view_y: f32,
) -> Result<(String, String), PrepareError> {
    let fixture_name = timeline_template(logical)?;
    let suffix = if view_y == 0.0 {
        "-ground"
    } else if view_y == 0.22_f32 {
        "-low"
    } else if view_y == 0.35_f32 {
        ""
    } else {
        return Err(PrepareError::Rejected(
            "unsupported source view-local placement height",
        ));
    };
    let prefab = if put_type == PutType::Target && !suffix.is_empty() {
        logical.replace(fixture_name, &format!("{fixture_name}{suffix}"))
    } else {
        logical.to_owned()
    };
    Ok((timeline_package(logical)?, prefab))
}

#[cfg(test)]
#[path = "fixture_activity_data_harness.rs"]
mod harness;
