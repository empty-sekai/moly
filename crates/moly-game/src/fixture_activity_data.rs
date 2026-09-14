//! Source-table joins for prepared single-character furniture activities.
//!
//! This module prepares data; it does not start an animation or reserve an
//! instance. The activity owner supplies the live lottery gates and publishes
//! the result atomically with its reservation. Missing data and rejected source
//! conditions are distinct outcomes, neither an implicit General-talk fallback.

use std::collections::{HashMap, HashSet};

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;
use moly_law::talk::{
    row::FixtureTalkRow,
    select::{matches_lottery_conditions, GateFlags, UniformDraw},
    TweetRef,
};
use serde_json::Value;

use crate::{fixture_attach::AttachPoints, talk::{TalkCandidates, TalkStore}};

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
        PATHS.iter().map(|path| server.load::<JsonAsset>(*path)).collect(),
    ));
}

pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    request: Option<Res<ActivityTableRequests>>,
) {
    let Some(request) = request else { return; };
    let mut documents = Vec::with_capacity(PATHS.len());
    for (path, handle) in PATHS.iter().zip(&request.0) {
        if let LoadState::Failed(error) = server.load_state(handle) {
            panic!("activity table {path} failed to load: {error:?}");
        }
        let Some(asset) = jsons.get(handle) else { return; };
        documents.push(serde_json::from_str::<Value>(&asset.0)
            .unwrap_or_else(|error| panic!("activity table {path}: {error}")));
    }
    let tables = FixtureActivityTables::from_documents(&documents)
        .unwrap_or_else(|error| panic!("activity table relationships: {error}"));
    info!("[fixture-activity] data ready: {} masters, {} timeline rows, {} pre-actions",
        tables.talks.rows.len(), tables.timelines.rows.len(), tables.pre_actions.len());
    commands.insert_resource(tables);
    commands.remove_resource::<ActivityTableRequests>();
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
pub(crate) enum PutType { None, Base, Target, Either }

#[derive(Debug, Clone)]
pub(crate) struct ActivityFixtureMaster {
    pub id: i32,
    pub model_name: String,
    pub grid_size: moly_law::fixture::Vector3Int,
    pub put_type: PutType,
    pub player_action_type: String,
    pub handle_type: String,
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
pub(crate) enum ReadSelection { Unread, Read }

pub(crate) struct QualificationInput<'a> {
    pub actor: Entity,
    pub unit: u32,
    pub fixture: Entity,
    pub fixture_id: i32,
    pub site_id: i32,
    pub phenomenon_id: i32,
    pub previous_talk_id: Option<i32>,
    /// None means the user-state supplier is not ready, not "unread".
    pub is_read: Option<bool>,
    pub read_selection: ReadSelection,
    /// Live MatchesLotteryConditions results from the activity/scene owner.
    /// This module additionally verifies its own table-derived predicates.
    pub gates: GateFlags,
    pub placed_fixture_ids: &'a [i32],
    pub placed_fixture_tag_ids: &'a [i32],
    /// The fixture-tag condition's value resolves through source master data.
    pub condition_fixture_tags: &'a HashMap<i32, Vec<i32>>,
}

/// Constructed only after all eight lottery gates and read-state filtering.
pub(crate) struct QualifiedSingleSelection {
    master: ActivityMaster,
    is_general: bool,
    actor: Entity,
    unit: u32,
    unit_slot: usize,
    fixture: Entity,
    fixture_id: i32,
    site_id: i32,
}

pub(crate) struct FixtureInstanceInput<'a> {
    pub entity: Entity,
    pub fixture_id: i32,
    pub model_package: &'a str,
    pub world: &'a GlobalTransform,
    pub site_id: i32,
    /// Actual source ViewObject.localPosition.y, not world/placement height.
    pub view_local_y: f32,
}

pub(crate) struct ActorInput {
    pub entity: Entity,
    pub unit: u32,
    pub site_id: i32,
    pub position: [f32; 3],
    pub site_y: f32,
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

#[derive(Debug, Clone)]
pub(crate) struct PreparedTimeline {
    pub master: ActivityTimeline,
    pub package: String,
    pub prefab_name: String,
}

/// Immutable factory output. The execution owner resolves this prefab through
/// its actual Director/Timeline identity and validates nonempty actor clips;
/// no animation handle (empty or otherwise) is created by this data factory.
#[derive(Debug, Clone)]
pub(crate) struct SingleFixtureActivity {
    pub master: ActivityMaster,
    /// Source general-condition classification, not inferred from the parser
    /// backend or from SingleCharacterFixture's activity type.
    pub is_general: bool,
    pub fixture: Entity,
    pub fixture_id: i32,
    pub actor: Entity,
    pub unit: u32,
    pub target_position: [f32; 3],
    pub pre_action: Option<ActivityPreAction>,
    pub tweet: Option<TweetRef>,
    pub timeline: Option<PreparedTimeline>,
    pub locate: Option<ActivityLocate>,
}

pub(crate) enum TargetRequest {
    /// Return the original position if the source .3m sample succeeds. Do not
    /// substitute hit.position into the activity's target-position binding.
    ActionPoint { position: [f32; 3], sample_distance: f32 },
    /// The no-timeline single-character branch uses GetLittleFarPosition.
    NearFixture { actor_position: [f32; 3], search_range: i32 },
}

impl FixtureActivityTables {
    /// Iterate the existing parsed script rows in original master order. This
    /// is not an eligible pool: the caller retains its existing candidate view
    /// and must call qualify_single with the live source gates for every entry.
    pub(crate) fn ordered_rows<'a>(
        &'a self, store: &'a TalkStore, candidates: &'a TalkCandidates,
    ) -> impl Iterator<Item = &'a FixtureTalkRow> {
        self.talks.rows.iter().filter(move |master| candidates.contains_talk(master.id))
            .filter_map(move |master| store.row(master.id))
    }

    pub(crate) fn player_timelines(&self, fixture_id: i32) -> impl Iterator<Item = &PlayerTimelineRow> {
        self.player_timelines.rows.iter().filter(move |row| row.fixture_id == fixture_id)
    }

    pub(crate) fn sd_visual_rows(&self, unit: u32, fixture_id: i32) -> impl Iterator<Item = &NoTalkVisualRow> {
        self.no_talk_visuals.rows.iter().filter(move |row| row.unit == unit && row.fixture_id == fixture_id)
    }

    /// The full source row order, before the no-talk Guid-key permutation.
    pub(crate) fn no_talk_rows(&self) -> &[NoTalkVisualRow] {
        &self.no_talk_visuals.rows
    }

    /// CreateNoneTalkData uses the first group whose Unit1 is this actor,
    /// and the first timeline of its action group to assign locators. The
    /// later random timeline choice is independent of this assignment.
    pub(crate) fn no_talk_point(&self, unit: u32, first_timeline: &ActivityTimeline) -> Result<i32, PrepareError> {
        let group = self.units.rows.iter().find(|group| group.units[0] == Some(unit as i32))
            .ok_or_else(|| PrepareError::DataMissing(format!("no source Unit1 group for NPC {unit}")))?;
        if group.units.iter().skip(1).any(|unit| unit.is_some_and(|unit| unit != 0)) {
            return Err(PrepareError::OtherActivity("source no-talk unit group needs multiple actor ownership"));
        }
        self.action_point_value(first_timeline.action_point_definition, 0)?
            .filter(|point| *point != 0)
            .ok_or(PrepareError::Rejected("source no-talk actor has no action-point assignment"))
    }

    pub(crate) fn timeline_rows(&self, group_id: i32) -> impl Iterator<Item = &ActivityTimeline> {
        self.timelines.rows.iter().filter(move |row| row.group_id == group_id)
    }

    pub(crate) fn action_point_value(&self, definition: i32, unit_slot: usize) -> Result<Option<i32>, PrepareError> {
        let row = self.action_points.get(definition).ok_or_else(|| missing("action point", definition))?;
        row.points.get(unit_slot).copied()
            .ok_or_else(|| PrepareError::InvalidData("action point unit slot exceeds source definition".into()))
    }

    pub(crate) fn fixture_master(&self, id: i32) -> Option<&ActivityFixtureMaster> {
        self.fixtures.get(&id)
    }

    /// Legacy offline layout seeds sometimes name only a model. Resolve that
    /// seed only when the source master has exactly one such row; variants
    /// sharing a model still need their explicit saved master identity.
    pub(crate) fn unique_fixture_master_for_model(&self, model: &str) -> Option<&ActivityFixtureMaster> {
        let mut rows = self.fixtures.values().filter(|row| row.model_name == model);
        let row = rows.next()?;
        rows.next().is_none().then_some(row)
    }

    pub(crate) fn qualify_single(
        &self,
        row: &FixtureTalkRow,
        candidates: &TalkCandidates,
        input: QualificationInput<'_>,
    ) -> Result<QualifiedSingleSelection, PrepareError> {
        if !candidates.contains_talk(row.talk_id) {
            return Err(PrepareError::Rejected("master is not in the current fixture-talk candidate view"));
        }
        let master = self.talks.get(row.talk_id).ok_or_else(|| missing("talk", row.talk_id))?;
        if master.lua != row.lua || master.condition_group_id != row.condition_group_id
            || master.site_group_id != row.site_group_id || master.term_id != row.term_id {
            return Err(PrepareError::InvalidData(format!("script/master mismatch for talk {}", row.talk_id)));
        }
        let group = self.units.get(master.unit_group_id).ok_or_else(|| missing("unit group", master.unit_group_id))?;
        let members: Vec<_> = group.units.iter().enumerate()
            .filter_map(|(slot, unit)| unit.filter(|unit| *unit != 0).map(|unit| (slot, unit))).collect();
        if members.len() != 1 { return Err(PrepareError::OtherActivity("not a single-character master")); }
        let (unit_slot, unit) = members[0];
        if row.unit_ids.as_slice() != [unit].as_slice() {
            return Err(PrepareError::InvalidData(format!("script/unit-group mismatch for talk {}", row.talk_id)));
        }
        let is_read = input.is_read.ok_or_else(|| PrepareError::DataMissing("read-state supplier is not ready".into()))?;
        if is_read != matches!(input.read_selection, ReadSelection::Read) {
            return Err(PrepareError::Rejected("read-state filter"));
        }
        let sites = self.site_groups.get(&master.site_group_id).ok_or_else(|| missing("site group", master.site_group_id))?;
        if !row.fixture_ids.contains(&input.fixture_id) {
            return Err(PrepareError::Rejected("instance is not a fixture of the selected master"));
        }
        let mut has_fixture_condition = false;
        let mut is_general = false;
        let mut condition_matches = false;
        let conditions: Vec<_> = self.condition_groups.rows.iter()
            .filter(|entry| entry.group_id == master.condition_group_id).collect();
        if conditions.is_empty() { return Err(missing("condition group", master.condition_group_id)); }
        for entry in conditions {
            let condition = self.conditions.get(entry.condition_id).ok_or_else(|| missing("condition", entry.condition_id))?;
            is_general |= matches!(condition.kind.as_str(), "read_event_story_episode_id"
                | "mysekai_character_visit_count" | "mysekai_phenomena_id");
            condition_matches |= match condition.kind.as_str() {
                // These TalkUtility predicates are unconditional in the source
                // client. Greeting visit-count ranges belong to another path.
                "read_event_story_episode_id" | "mysekai_character_visit_count" => true,
                "mysekai_phenomena_id" => condition.value == input.phenomenon_id,
                "mysekai_fixture_id" => {
                    has_fixture_condition = true;
                    input.placed_fixture_ids.contains(&condition.value)
                }
                "mysekai_fixture_tag_id" => {
                    let tags = input.condition_fixture_tags.get(&condition.value)
                        .ok_or_else(|| missing("fixture tag-group resolution", condition.value))?;
                    tags.iter().any(|tag| input.placed_fixture_tag_ids.contains(tag))
                }
                "after_set_fixture" => false,
                _ => return Err(PrepareError::InvalidData(format!("unknown condition type {} on {}", condition.kind, condition.id))),
            };
        }
        if !has_fixture_condition { return Err(PrepareError::OtherActivity("not a fixture-action master")); }
        let mut gates = input.gates;
        gates.character_condition &= unit > 0 && unit as u32 == input.unit;
        gates.prev_talk &= input.previous_talk_id != Some(master.id);
        gates.condition &= condition_matches;
        gates.environment_site_condition &= sites.contains(&input.site_id);
        if !matches_lottery_conditions(&gates) { return Err(PrepareError::Rejected("MatchesLotteryConditions")); }
        Ok(QualifiedSingleSelection {
            master: master.clone(), is_general, actor: input.actor, unit: input.unit, unit_slot,
            fixture: input.fixture, fixture_id: input.fixture_id, site_id: input.site_id,
        })
    }

    pub(crate) fn prepare_single(
        &self,
        row: &FixtureTalkRow,
        selected: QualifiedSingleSelection,
        fixture: &FixtureInstanceInput<'_>,
        actor: &ActorInput,
        draw: &mut impl UniformDraw,
        attachments: &AttachPoints,
        mut resolve_target: impl FnMut(TargetRequest) -> Result<Option<[f32; 3]>, PrepareError>,
    ) -> Result<SingleFixtureActivity, PrepareError> {
        if row.talk_id != selected.master.id || actor.entity != selected.actor || actor.unit != selected.unit
            || fixture.entity != selected.fixture || fixture.fixture_id != selected.fixture_id
            || actor.site_id != selected.site_id || fixture.site_id != selected.site_id {
            return Err(PrepareError::Rejected("qualified actor/instance/site changed"));
        }
        let fixture_master = self.fixtures.get(&fixture.fixture_id).ok_or_else(|| missing("fixture", fixture.fixture_id))?;
        if fixture.model_package != format!("mysekai__fixture__{}", fixture_master.model_name) {
            return Err(PrepareError::InvalidData("instance model/master mismatch".into()));
        }
        if !actor.position.into_iter().chain([actor.site_y, fixture.view_local_y]).all(f32::is_finite) {
            return Err(PrepareError::InvalidData("non-finite activity pose".into()));
        }
        let (scale, rotation, position) = fixture.world.to_scale_rotation_translation();
        if !scale.is_finite() || !rotation.is_finite() || !position.is_finite() {
            return Err(PrepareError::InvalidData("non-finite fixture instance transform".into()));
        }
        let pre_action = self.pre_action_by_talk.get(&row.talk_id).map(|index| self.pre_actions[*index].clone());
        if pre_action.as_ref().and_then(|pre| pre.fixture_together_communication_id).is_some_and(|id| id != 0) {
            return Err(PrepareError::OtherActivity("fixture-together waiting must be prepared by its own factory"));
        }
        let tweet = match pre_action.as_ref().map(|pre| pre.tweet_id) {
            Some(id) if id != 0 => {
                if row.tweet.id != id { return Err(PrepareError::InvalidData("script/pre-action tweet mismatch".into())); }
                Some(row.tweet.clone())
            }
            _ => None,
        };
        let group_id = pre_action.as_ref().and_then(|pre| pre.timeline_group_id).filter(|id| *id != 0);
        let (timeline, locate, request) = if let Some(group_id) = group_id {
            let pool: Vec<_> = self.timelines.rows.iter().filter(|row| row.group_id == group_id).collect();
            if pool.is_empty() { return Err(missing("timeline group", group_id)); }
            let index = draw.draw(pool.len());
            let timeline = (**pool.get(index).ok_or_else(|| PrepareError::InvalidData("timeline draw outside its pool".into()))?).clone();
            let points = self.action_points.get(timeline.action_point_definition)
                .ok_or_else(|| missing("action-point definition", timeline.action_point_definition))?;
            let value = points.points.get(selected.unit_slot).copied().flatten().filter(|point| *point != 0)
                .ok_or_else(|| PrepareError::Rejected("selected actor has no action-point assignment"))?;
            let action_point_index = attachments.instance_index(fixture.model_package, value)
                // This API also returns None for legacy data without a source
                // index or ambiguous multi-view entries. Neither is a proven
                // source rejection and neither may trigger a General fallback.
                .ok_or_else(|| PrepareError::DataMissing(format!(
                    "action-point identity {value} is unavailable or ambiguous for {}", fixture.model_package
                )))?;
            let pair = attachments.instance_poses(fixture.model_package, value, fixture.world)
                .ok_or_else(|| PrepareError::InvalidData("resolved action-point index has no pose".into()))?;
            let finite_pose = |pose: crate::fixture_attach::AttachPose| {
                pose.position.into_iter().all(f32::is_finite) && pose.rotation.is_finite()
            };
            if !finite_pose(pair.start) || pair.end.is_some_and(|pose| !finite_pose(pose)) {
                return Err(PrepareError::InvalidData("non-finite action-point pose".into()));
            }
            let locate = ActivityLocate {
                unit: actor.unit, unit_slot: selected.unit_slot, action_point_value: value, action_point_index,
                start: ActivityPose { position: pair.start.position, rotation: pair.start.rotation },
                end: pair.end.map(|pose| ActivityPose { position: pose.position, rotation: pose.rotation }),
            };
            let (package, prefab_name) = timeline_asset(&timeline.asset_name, fixture_master.put_type, fixture.view_local_y)?;
            let request = TargetRequest::ActionPoint {
                position: [locate.start.position[0], actor.site_y, locate.start.position[2]], sample_distance: 0.3,
            };
            (Some(PreparedTimeline { master: timeline, package, prefab_name }), Some(locate), request)
        } else {
            (None, None, TargetRequest::NearFixture { actor_position: actor.position, search_range: 2 })
        };
        let original_action_target = match &request {
            TargetRequest::ActionPoint { position, .. } => Some(*position),
            TargetRequest::NearFixture { .. } => None,
        };
        let resolved = resolve_target(request)?.ok_or(PrepareError::Rejected("source target-position calculation failed"))?;
        if !resolved.into_iter().all(f32::is_finite) { return Err(PrepareError::InvalidData("non-finite target".into())); }
        let target_position = original_action_target.unwrap_or(resolved);
        Ok(SingleFixtureActivity {
            master: selected.master.clone(), is_general: selected.is_general,
            fixture: fixture.entity, fixture_id: fixture.fixture_id,
            actor: actor.entity, unit: actor.unit, target_position, pre_action, tweet, timeline, locate,
        })
    }

    fn from_documents(d: &[Value]) -> Result<Self, String> {
        let talks = ordered(&d[0], |row| Ok(ActivityMaster {
            id: int(row, "id")?, unit_group_id: int(row, "mysekaiGameCharacterUnitGroupId")?,
            condition_group_id: int(row, "mysekaiCharacterTalkConditionGroupId")?,
            site_group_id: int(row, "mysekaiSiteGroupId")?, term_id: int(row, "mysekaiCharacterTalkTermId")?,
            lua: string(row, "lua")?,
        }))?;
        let units = ordered(&d[1], |row| Ok(UnitGroup { id: int(row, "id")?, units: [
            optional_int(row, "gameCharacterUnitId1")?, optional_int(row, "gameCharacterUnitId2")?,
            optional_int(row, "gameCharacterUnitId3")?, optional_int(row, "gameCharacterUnitId4")?,
            optional_int(row, "gameCharacterUnitId5")?,
        ] }))?;
        let conditions = ordered(&d[2], |row| Ok(Condition {
            id: int(row, "id")?, kind: string(row, "mysekaiCharacterTalkConditionType")?,
            value: int(row, "mysekaiCharacterTalkConditionTypeValue")?,
        }))?;
        let condition_groups = ordered(&d[3], |row| Ok(ConditionGroupRow {
            group_id: int(row, "groupId")?, condition_id: int(row, "mysekaiCharacterTalkConditionId")?,
        }))?;
        let timelines = ordered(&d[4], |row| Ok(ActivityTimeline {
            id: int(row, "id")?, group_id: int(row, "groupId")?, asset_name: string(row, "assetbundleName")?,
            action_point_definition: int(row, "mysekaiCharacterTalkActionPointId")?,
        }))?;
        let action_points = ordered(&d[5], |row| Ok(ActionPointDefinition { points: [
            optional_int(row, "gameCharacterUnitId1ActionPoint")?, optional_int(row, "gameCharacterUnitId2ActionPoint")?,
            optional_int(row, "gameCharacterUnitId3ActionPoint")?, optional_int(row, "gameCharacterUnitId4ActionPoint")?,
        ] }))?;
        let player_timelines = ordered(&d[6], |row| Ok(PlayerTimelineRow {
            id: int(row, "id")?, fixture_id: int(row, "mysekaiFixtureId")?,
            asset_name: string(row, "assetbundleName")?, action_point: int(row, "actionPoint")?,
        }))?;
        let mut pre_actions = Vec::new();
        let mut pre_action_by_talk = HashMap::new();
        for row in array(&d[7], "talkPreActions")? {
            for field in ["timelineGroupId", "fixtureTogetherCommunicationId"] {
                if row.get(field).is_none() { return Err(format!("pre-action export does not carry {field}")); }
            }
            let value = ActivityPreAction {
                id: int(row, "id")?, talk_id: int(row, "talkId")?, tweet_id: int(row, "tweetId")?,
                timeline_group_id: optional_int(row, "timelineGroupId")?,
                fixture_together_communication_id: optional_int(row, "fixtureTogetherCommunicationId")?,
            };
            if pre_action_by_talk.insert(value.talk_id, pre_actions.len()).is_some() {
                return Err(format!("duplicate pre-action talk {}", value.talk_id));
            }
            pre_actions.push(value);
        }
        let mut fixtures = HashMap::new();
        for row in array(&d[8], "fixtures")? {
            let id = int(row, "id")?;
            let put_type = match string(row, "putType")?.as_str() {
                "none" => PutType::None, "put_base" => PutType::Base,
                "put_target" => PutType::Target, "put_either" => PutType::Either,
                other => return Err(format!("unknown put type {other}")),
            };
            if fixtures.insert(id, ActivityFixtureMaster { id, model_name: string(row, "assetbundleName")?,
                grid_size: moly_law::fixture::Vector3Int::new(int(row, "gridWidth")?, int(row, "gridHeight")?, int(row, "gridDepth")?),
                put_type, player_action_type: string(row, "playerActionType")?,
                handle_type: string(row, "handleType")? }).is_some() {
                return Err(format!("duplicate fixture {id}"));
            }
        }
        let mut site_groups = HashMap::new();
        for row in array(&d[9], "groups")? {
            let id = int(row, "siteGroupId")?;
            let sites = array(row, "sites")?.iter().map(value_int).collect::<Result<_, _>>()?;
            if site_groups.insert(id, sites).is_some() { return Err(format!("duplicate site group {id}")); }
        }
        let no_talk_visuals = ordered(&d[10], |row| Ok(NoTalkVisualRow {
            id: int(row, "id")?,
            unit: u32::try_from(int(row, "gameCharacterUnitId")?).map_err(|_| "negative visual unit")?,
            fixture_id: int(row, "mysekaiFixtureId")?,
            timeline_group_id: int(row, "mysekaiCharacterTalkFixtureTimelineGroupId")?,
        }))?;
        Ok(Self { talks, units, conditions, condition_groups, timelines, action_points,
            player_timelines, no_talk_visuals, pre_actions, pre_action_by_talk, fixtures, site_groups })
    }
}

fn missing(table: &str, id: i32) -> PrepareError {
    PrepareError::DataMissing(format!("missing {table} {id}"))
}

fn ordered<T>(value: &Value, mut parse: impl FnMut(&Value) -> Result<T, String>) -> Result<OrderedTable<T>, String> {
    let entries = value.get("entries").and_then(Value::as_object).ok_or("table entries must be an object")?;
    let order = array(value, "rowOrder")?;
    let mut rows = Vec::with_capacity(order.len());
    let mut by_id = HashMap::with_capacity(order.len());
    let mut seen = HashSet::new();
    for entry in order {
        let id = value_int(entry)?;
        if !seen.insert(id) { return Err(format!("duplicate rowOrder id {id}")); }
        let row = entries.get(&id.to_string()).ok_or_else(|| format!("rowOrder references absent id {id}"))?;
        if int(row, "id")? != id { return Err(format!("entry id mismatch {id}")); }
        by_id.insert(id, rows.len());
        rows.push(parse(row)?);
    }
    if entries.len() != rows.len() { return Err("entries not covered by rowOrder".into()); }
    Ok(OrderedTable { rows, by_id })
}

fn value_int(value: &Value) -> Result<i32, String> {
    value.as_i64().and_then(|value| i32::try_from(value).ok()).ok_or_else(|| "expected i32 integer".into())
}
fn int(value: &Value, key: &str) -> Result<i32, String> {
    value.get(key).ok_or_else(|| format!("missing integer {key}")).and_then(value_int)
}
fn optional_int(value: &Value, key: &str) -> Result<Option<i32>, String> {
    match value.get(key) { None | Some(Value::Null) => Ok(None), Some(value) => value_int(value).map(Some) }
}
fn string(value: &Value, key: &str) -> Result<String, String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("missing string {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value.get(key).and_then(Value::as_array).ok_or_else(|| format!("missing array {key}"))
}

/// Extract the substring between the first and fourth underscores, exactly
/// as the factory does; the timeline can intentionally use another model's
/// compatible template. Model identity is not a shortcut for this parsing.
fn timeline_template(logical: &str) -> Result<&str, PrepareError> {
    let underscores: Vec<_> = logical.match_indices('_').take(4).map(|(index, _)| index).collect();
    if underscores.len() != 4 { return Err(PrepareError::InvalidData(format!("invalid timeline asset name {logical}"))); }
    Ok(&logical[underscores[0] + 1..underscores[3]])
}

/// Source asset-name package routing, without applying any NPC placement
/// variant rule to the player's original timeline or its sound companions.
pub(crate) fn timeline_package(logical: &str) -> Result<String, PrepareError> {
    Ok(format!("mysekai__fixture_timeline__mdl_{}", timeline_template(logical)?))
}

pub(crate) fn timeline_asset(logical: &str, put_type: PutType, view_y: f32) -> Result<(String, String), PrepareError> {
    let fixture_name = timeline_template(logical)?;
    let suffix = if view_y == 0.0 { "-ground" } else if view_y == 0.22_f32 { "-low" }
        else if view_y == 0.35_f32 { "" } else { return Err(PrepareError::Rejected("unsupported source view-local placement height")); };
    let prefab = if put_type == PutType::Target && !suffix.is_empty() {
        logical.replace(fixture_name, &format!("{fixture_name}{suffix}"))
    } else { logical.to_owned() };
    Ok((timeline_package(logical)?, prefab))
}
