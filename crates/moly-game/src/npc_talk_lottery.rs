//! NPC talk content: the talk-id lotteries over the client talk list and the
//! factories each decision row calls to build its talk data.
//!
//! Every lottery reads the one client talk list (the server's per-user list
//! with read flags, in the server's order), never the master catalogue:
//!
//! - **General lottery.** Six lazy filters over the list, per row in list
//!   order: the talk's speaker group is exactly this character; the talk has
//!   a general condition (read event story, phenomenon or visit count); it is
//!   not this character's own previous talk; its general conditions match
//!   (an OR over every row: event story and visit count are true, a
//!   phenomenon row compares with the current phenomenon, any other row type
//!   raises); one of its site group's sites has this character's own site
//!   type; its fixture condition is playable (always true here, since a
//!   fixture row already raised in the fourth filter). The kept talks take
//!   one engine integer range. With none kept the character's own previous
//!   talk id comes back with no draw, and without one the source raises.
//! - **Fixture lotteries** (unread fixture talks, read talks, already-read
//!   talks). The list is filtered (unread and carrying a fixture condition,
//!   or read), then the eight lottery gates keep the pool in list order, and
//!   the member-count lottery picks one talk (see `moly_law::talk::select`).
//!   The eight gates, in source order: the talk's first speaker is this
//!   character; not its previous talk; its conditions match (an OR: event
//!   story and visit count true, phenomenon compared, a fixture row true when
//!   a fixture of that id stands on this character's site, a fixture-tag row
//!   compared through tag groups, after-set-fixture skipped, anything else
//!   raises); a fixture of the talk's fixture id stands on this character's
//!   site; one of them has a motion area that overlaps nothing; the fixture
//!   condition is playable (every member present and free, and one fixture
//!   usable by the talk's action points and actionable by this character);
//!   the site group holds this character's site type; and, when the talk
//!   names the gate fixture, the character has existed longer than the
//!   configured gate delay (no gate on the site fails every fixture talk).
//!   Gates without a fixture condition pass. The playable-fixture gate's
//!   admissibility pair (the talk's pre-action timelines have playable action
//!   points on the fixture, and the fixture is actionable by this character)
//!   is evaluated by the host's fixture admission for each placed fixture of
//!   the talk's fixture id in the fixture manager's order; the first fixture
//!   that passes both makes the gate true.
//!
//! Every source exception is a [`Halt::Fault`]: the source's AI loop does not
//! catch it, so that character's AI stops. A predicate or table this host
//! does not carry, reached on the way, is a [`Halt::Gap`]: it is reported by
//! name and the decision ends as empty talk data. Neither is silent.
//!
//! - **Fixture-talk factories** (data by talk id). After the master lookup,
//!   three factories are tried in order, then a fresh general lottery:
//!   1. *some-character fixture action* (multiple-character fixture talk):
//!      with a pre-action timeline group it first picks the timeline (one
//!      sequence pick over the group's timelines, drawn before anything else
//!      is checked, so a talk it then refuses has still drawn); it refuses a
//!      missing unit group (logged), no timeline, fewer than two members, and
//!      a null or empty locate list; its target is the character's locate row
//!      through GetTargetPosition, whose bool it ignores;
//!   2. *waiting with a communication* (the pre-action names a together
//!      communication, and a fixture is chosen): no draw; the target is the
//!      character's locate row's StartLoc at the site's height, with that
//!      locator's rotation;
//!   3. *fixture action* (single-character fixture talk; a fixture is
//!      chosen): with a timeline group, its own timeline pick and the locate
//!      list (null raises, empty refuses); without, GetTargetPosition with
//!      index 0 and no locate list; it returns GetTargetPosition's bool, so a
//!      target that is not found falls to the general lottery.
//!   The locate list pairs each member slot of the unit group with the same
//!   column of the action-point row and finds, in the fixture's action-point
//!   order, the first locator whose StartLoc name carries that value.
//!
//!   GetTargetPosition has four arms: no master gives (false, zero); a
//!   talk without a pre-action takes GetLittleFarPosition, a miss giving
//!   (false, zero); a pre-action with neither a timeline group nor a
//!   together communication takes GetLittleFarPosition, a miss giving
//!   (false, the character's position); otherwise the locator's StartLoc at
//!   the site's height, true when the surface samples within 0.3 (the
//!   unsampled point is returned), else (false, the character's position).
//!   GetLittleFarPosition walks the one-cell ring around the fixture's
//!   footprint inside the floor grid, ordered by distance to the cell corner
//!   (stable), and returns the surface hit of the first corner (plus half a
//!   tile) that samples within the configured fixture move offset.
//!
//! - **Fixture-tag condition** (`IsMatchedFixtureTagCondition`): the
//!   condition's value is a fixture id; that fixture's master names a tag
//!   group, whose non-zero tag ids (up to five, in field order) are compared
//!   with the distinct tag ids of every placed fixture (the fixture manager's
//!   whole list, any site): true when one is shared. A condition fixture or
//!   a placed fixture without a master row, or a tag group id the tag-group
//!   table lacks, is the source's null dereference.
//! - **Fixture-common talk** (`CreateCommonFixtureActionAITalkData`, the
//!   fixture-common row of the objective lottery): one general lottery, the
//!   talk's master, and the talk's fixture through the two-argument chooser;
//!   no fixture builds nothing. Otherwise the character's fixture-common rows
//!   (source row order) whose furniture group holds the chosen fixture's id;
//!   none builds nothing; one sequence pick among them. The AI model's tweet
//!   id takes the picked row's id, the target is GetTargetPosition(the
//!   character's position, master, fixture, 0) with its bool dropped, and the
//!   data is type 5 with the character alone and no pre-action. Its caller
//!   then sets the tweet id from the talk's pre-action when one exists.
//!
//! Named gaps (reported by name if reached): the fixture admissibility pair
//! where the host's fixture admission cannot evaluate it (placement or
//! locator data it does not resolve); fixture geometry the host cannot
//! resolve for a factory; a talk table of [`TalkExtraTables`] (or the
//! together-communication table) that the asset source lacks.

use std::collections::{HashMap, HashSet};

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;

use moly_law::objective::random_fixture_action as fixture_action;
use moly_law::objective::{ObjectiveType, TalkType};
use moly_law::talk::select::{
    self as law, Candidate, GeneralPick, LotteryFault, LotteryWeights, UniformDraw, WeightedDraw,
};
use moly_law::talk::EnumerablePick;
use moly_law::talk::{
    condition_type_discriminant, CONDITION_AFTER_SET_FIXTURE,
    CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT, CONDITION_MYSEKAI_FIXTURE_ID,
    CONDITION_MYSEKAI_FIXTURE_TAG_ID, CONDITION_MYSEKAI_PHENOMENA_ID,
    CONDITION_READ_EVENT_STORY_EPISODE_ID,
};
use serde_json::{json, Value};

use crate::fixture_activity_data::FixtureActivityTables;
use crate::server_panel::TalkWithReadHistory;

/// Why a lottery or factory stopped.
#[derive(Debug, Clone)]
pub(crate) enum Halt {
    /// A source exception. The source's AI loop rethrows it and that
    /// character's AI ends.
    Fault(String),
    /// A predicate or table this host does not carry was reached.
    Gap(String),
}

impl Halt {
    fn fault(reason: impl Into<String>) -> Self {
        Halt::Fault(reason.into())
    }

    fn gap(reason: impl Into<String>) -> Self {
        Halt::Gap(reason.into())
    }
}

impl From<LotteryFault> for Halt {
    fn from(fault: LotteryFault) -> Self {
        match fault {
            LotteryFault::GeneralPoolEmpty => Halt::fault(
                "general lottery kept no talk and the character has no previous talk (index out of range)",
            ),
            LotteryFault::EmptyMemberCountPick { member_count } => Halt::fault(format!(
                "member-count bucket {member_count} holds no talk (sequence pick on an empty set)"
            )),
        }
    }
}

/// A fixture-common row: the character's common furniture reaction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixtureCommonRow {
    pub(crate) id: i32,
    pub(crate) unit: i32,
    pub(crate) fixture_group_id: i32,
}

/// A some-character talk row: a talk played as a some-character timeline
/// from its main character.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SomeCharacterTalkRow {
    pub(crate) id: i32,
    pub(crate) talk_id: i32,
    pub(crate) main_unit: i32,
}

/// The fixture tag groups: every fixture master's tag group id, and each
/// group's tag ids (the five fields in order, zeros kept).
#[derive(Debug, Clone, Default)]
pub(crate) struct FixtureTagGroups {
    pub(crate) group_of_fixture: HashMap<i32, i32>,
    pub(crate) tags_of_group: HashMap<i32, [i32; 5]>,
}

impl FixtureTagGroups {
    /// GetFixtureTagIdsFromTagGroupId: the group's non-zero tag ids in field
    /// order; `None` for a group the table lacks (the source's null
    /// dereference).
    fn tag_ids(&self, group: i32) -> Option<Vec<i32>> {
        self.tags_of_group
            .get(&group)
            .map(|tags| tags.iter().copied().filter(|tag| *tag != 0).collect())
    }
}

/// Talk tables the fixture-talk paths read beyond the activity tables. Each
/// loads on its own: an asset source without one keeps the others, and the
/// path that reads it names the missing table when it is reached.
#[derive(Resource)]
pub(crate) struct TalkExtraTables {
    /// Fixture-common rows, in source row order.
    pub(crate) fixture_commons: Result<Vec<FixtureCommonRow>, String>,
    /// Fixture-common furniture groups: group id -> fixture ids, row order.
    pub(crate) fixture_common_groups: Result<HashMap<i32, Vec<i32>>, String>,
    /// Some-character talk rows, in source row order.
    pub(crate) some_character_talks: Result<Vec<SomeCharacterTalkRow>, String>,
    pub(crate) fixture_tags: Result<FixtureTagGroups, String>,
}

/// The pending loads of [`TalkExtraTables`]; present until they settle.
#[derive(Resource)]
pub(crate) struct TalkExtraRequests([Handle<JsonAsset>; 4]);

const TALK_EXTRA_PATHS: [&str; 4] = [
    "moly://mysekai-character-talk-fixture-commons.json",
    "moly://mysekai-character-talk-fixture-common-fixture-groups.json",
    "moly://mysekai-character-talk-some-character-talks.json",
    "moly://mysekai-fixture-tag-groups.json",
];

pub(crate) fn load_talk_extras(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(TalkExtraRequests(
        TALK_EXTRA_PATHS.map(|path| server.load::<JsonAsset>(path)),
    ));
}

pub(crate) fn parse_talk_extras(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    request: Option<Res<TalkExtraRequests>>,
) {
    let Some(request) = request else {
        return;
    };
    let mut documents: Vec<Result<Value, String>> = Vec::with_capacity(4);
    for (path, handle) in TALK_EXTRA_PATHS.iter().zip(&request.0) {
        if let LoadState::Failed(error) = server.load_state(handle) {
            documents.push(Err(format!("{path} is not in this asset source ({error:?})")));
            continue;
        }
        let Some(asset) = jsons.get(handle) else {
            return;
        };
        documents.push(
            serde_json::from_str::<Value>(&asset.0).map_err(|error| format!("{path}: {error}")),
        );
    }
    let [commons, groups, some, tags]: [Result<Value, String>; 4] = documents
        .try_into()
        .expect("one document per path");
    let tables = TalkExtraTables {
        fixture_commons: commons.and_then(|doc| {
            table_rows(&doc, |row| {
                Ok(FixtureCommonRow {
                    id: row_int(row, "id")?,
                    unit: row_int(row, "gameCharacterUnitId")?,
                    fixture_group_id: row_int(
                        row,
                        "mysekaiCharacterTalkFixtureCommonMysekaiFixtureGroupId",
                    )?,
                })
            })
        }),
        fixture_common_groups: groups.and_then(|doc| {
            let mut groups: HashMap<i32, Vec<i32>> = HashMap::new();
            for (group, fixture) in table_rows(&doc, |row| {
                Ok((row_int(row, "groupId")?, row_int(row, "mysekaiFixtureId")?))
            })? {
                groups.entry(group).or_default().push(fixture);
            }
            Ok(groups)
        }),
        some_character_talks: some.and_then(|doc| {
            table_rows(&doc, |row| {
                Ok(SomeCharacterTalkRow {
                    id: row_int(row, "id")?,
                    talk_id: row_int(row, "mysekaiCharacterTalkId")?,
                    main_unit: row_int(row, "mainGameCharacterUnitId")?,
                })
            })
        }),
        fixture_tags: tags.and_then(|doc| {
            let mut tags = FixtureTagGroups::default();
            for (fixture, (group, ids)) in table_rows(&doc, |row| {
                let group = row
                    .get("mysekaiFixtureTagGroup")
                    .ok_or("missing mysekaiFixtureTagGroup")?;
                let mut ids = [0; 5];
                for (slot, id) in ids.iter_mut().enumerate() {
                    // An absent field is the member's default, 0.
                    *id = match group.get(format!("mysekaiFixtureTagId{}", slot + 1)) {
                        None => 0,
                        Some(value) => value_i32(value)?,
                    };
                }
                Ok((row_int(row, "id")?, (row_int(group, "id")?, ids)))
            })? {
                tags.group_of_fixture.insert(fixture, group);
                tags.tags_of_group.entry(group).or_insert(ids);
            }
            Ok(tags)
        }),
    };
    let counts = [
        tables.fixture_commons.as_ref().map(Vec::len),
        tables
            .fixture_common_groups
            .as_ref()
            .map(|groups| groups.values().map(Vec::len).sum()),
        tables.some_character_talks.as_ref().map(Vec::len),
        tables.fixture_tags.as_ref().map(|tags| tags.group_of_fixture.len()),
    ];
    for (path, count) in TALK_EXTRA_PATHS.iter().zip(counts) {
        match count {
            Ok(rows) => info!("[npc-talk] {path}: {rows} rows"),
            Err(reason) => warn!("[npc-talk] {reason}"),
        }
    }
    commands.insert_resource(tables);
    commands.remove_resource::<TalkExtraRequests>();
}

/// The rows of an exported master table, in its `rowOrder`.
fn table_rows<T>(
    doc: &Value,
    mut parse: impl FnMut(&Value) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let entries = doc
        .get("entries")
        .and_then(Value::as_object)
        .ok_or("table entries must be an object")?;
    let order = doc
        .get("rowOrder")
        .and_then(Value::as_array)
        .ok_or("missing array rowOrder")?;
    order
        .iter()
        .map(|id| {
            let key = value_i32(id)?.to_string();
            let row = entries
                .get(&key)
                .ok_or_else(|| format!("rowOrder references absent id {key}"))?;
            parse(row)
        })
        .collect()
}

fn value_i32(value: &Value) -> Result<i32, String> {
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("{value} is not a 32-bit integer"))
}

fn row_int(row: &Value, key: &str) -> Result<i32, String> {
    value_i32(row.get(key).ok_or_else(|| format!("missing integer {key}"))?)
}

/// One NPC as the talk lotteries read it from the avatar store.
#[derive(Debug, Clone)]
pub(crate) struct NpcView {
    pub(crate) unit: u32,
    /// The character's own current site type value.
    pub(crate) site_type: Option<i32>,
    /// The master id of the character's previous AI talk data; 0 = none.
    pub(crate) previous_talk_id: i32,
    /// The type of the character's current AI talk data.
    pub(crate) talk_type: Option<TalkType>,
    /// The character's current objective type.
    pub(crate) objective: Option<ObjectiveType>,
    /// The character's current action state value.
    pub(crate) state: u8,
    /// Seconds since the character's presenter was initialized.
    pub(crate) since_initialized: f32,
}

/// One placed fixture, in the fixture manager's insertion order.
#[derive(Debug, Clone)]
pub(crate) struct PlacedFixture {
    pub(crate) uid: String,
    pub(crate) fixture_id: i32,
    /// Site type value of the site the fixture stands on.
    pub(crate) located_site_type: Option<i32>,
    /// Its master's fixture type is the gate type (false without a master).
    pub(crate) is_gate: bool,
    /// Whether its current motion area overlaps another fixture; `Err` when
    /// the host cannot evaluate it.
    pub(crate) motion_overlap: Result<bool, String>,
}

/// Everything the talk lotteries read besides the draws.
pub(crate) struct LotteryScene<'a> {
    pub(crate) tables: &'a FixtureActivityTables,
    pub(crate) talk_list: &'a [TalkWithReadHistory],
    pub(crate) phenomena_id: i32,
    /// Site type value of a master site id.
    pub(crate) site_type_of_site: &'a dyn Fn(i32) -> Option<i32>,
    /// The avatar store's NPC list.
    pub(crate) npcs: &'a [NpcView],
    pub(crate) fixtures: &'a [PlacedFixture],
    pub(crate) weights: LotteryWeights,
    /// What the fixture-talk gates read beyond the lists; `None` in a scene
    /// that runs the general lottery only, which reaches no fixture gate.
    pub(crate) fixture_gates: Option<FixtureGateInputs<'a>>,
    /// The fixture-talk factories' host; `None` in a scene that runs the
    /// general lottery only.
    pub(crate) fixture_host: Option<&'a dyn FixtureTalkHost>,
    /// The together-communication table (the waiting factory's action-point
    /// rows), or why it is not in this asset source.
    pub(crate) together: Option<&'a Result<std::collections::HashMap<i32, i32>, String>>,
}

/// The fixture-talk gates' inputs for the deciding character.
pub(crate) struct FixtureGateInputs<'a> {
    /// The gate fixture delay in seconds (an integer client config).
    pub(crate) gate_action_elapsed_seconds: i32,
    /// The playable-fixture gate's admissibility pair: (talk id, placed
    /// fixture) -> usable action points and an actionable fixture; `Err`
    /// names host data it cannot evaluate.
    pub(crate) admissible: &'a dyn Fn(i32, &PlacedFixture) -> Result<bool, String>,
    /// The talk tables beyond the activity tables; `None` when their loader
    /// is not installed.
    pub(crate) extras: Option<&'a TalkExtraTables>,
}

/// The draw sources of one decision, with a record of every draw.
pub(crate) struct Draws<'a> {
    pub(crate) engine_int: &'a mut dyn FnMut(usize) -> usize,
    pub(crate) engine_float: &'a mut dyn FnMut(f32) -> f32,
    pub(crate) sequence_pick: &'a mut dyn FnMut(usize) -> Option<usize>,
    pub(crate) record: Vec<Value>,
}

/// The engine's integer range, max exclusive.
pub(crate) const SOURCE_ENGINE_INT: &str = "UnityEngine.Random.Range(int,int)";
/// The engine's float range, both ends included.
pub(crate) const SOURCE_ENGINE_FLOAT: &str = "UnityEngine.Random.Range(float,float)";
/// A pick over a sequence: a fresh generator per pick.
pub(crate) const SOURCE_SEQUENCE_PICK: &str = "RandomPick(IEnumerable): new System.Random().Next(n)";

struct IntDraw<'a, 'b> {
    draws: &'a mut Draws<'b>,
    use_word: &'static str,
}

impl UniformDraw for IntDraw<'_, '_> {
    fn draw(&mut self, len: usize) -> usize {
        let value = (self.draws.engine_int)(len);
        self.draws.record.push(json!({
            "use": self.use_word, "value": value, "range": [0, len], "source": SOURCE_ENGINE_INT,
        }));
        value
    }
}

/// Which fixture lottery filters the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixtureLottery {
    /// Unread talks carrying a fixture condition.
    NotYetRead,
    /// Read talks.
    Read,
    /// Read talks (the already-read lottery keeps the same rows).
    AlreadyRead,
}

impl FixtureLottery {
    fn word(self) -> &'static str {
        match self {
            FixtureLottery::NotYetRead => "not_yet_read_fixture_talk",
            FixtureLottery::Read => "read_talk",
            FixtureLottery::AlreadyRead => "already_read_talk",
        }
    }
}

/// What a talk factory left in the AI talk data.
#[derive(Debug, Clone)]
pub(crate) enum TalkPlan {
    /// A general talk, to be built with its target position; `interrupt`
    /// is the Talk interrupt the factory raises with it.
    General { talk_id: i32, interrupt: bool },
    /// The factory left the data empty; the talk objective completes at once.
    Null { reason: String, interrupt: bool },
    /// A fixture talk built by one of the three fixture-talk factories.
    Fixture(FixtureTalkData),
    /// A fixture-common talk (type 5): the general talk, the chosen placed
    /// fixture, GetTargetPosition's point (its bool is dropped) and the
    /// fixture-common row the pick chose. The Talk interrupt is raised.
    CommonFixture {
        talk_id: i32,
        fixture: String,
        target_position: [f32; 3],
        target_found: bool,
        common_id: i32,
    },
}

/// One row of a fixture talk's locate list (the source's
/// FixtureNpcActionLocateData): a member's unit, the index of its locator in
/// the fixture's action-point array, and the locator's slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocateRow {
    pub(crate) unit: i32,
    pub(crate) index: usize,
    pub(crate) slot_id: i32,
}

/// What a fixture-talk factory put in the AI talk data.
#[derive(Debug, Clone)]
pub(crate) struct FixtureTalkData {
    /// SingleCharacterFixture, MultipleCharacterFixture or
    /// CommunicationWhileDoingWait.
    pub(crate) kind: TalkType,
    pub(crate) talk_id: i32,
    /// The placed fixture (placement uid).
    pub(crate) fixture: String,
    pub(crate) target_position: [f32; 3],
    /// The locator rotation (xyzw) the data carries; the waiting factory
    /// sets it, the others leave it zero.
    pub(crate) rotation: Option<[f32; 4]>,
    /// The picked fixture timeline id, when the pre-action has a group.
    pub(crate) timeline: Option<i32>,
    /// The locate list; `None` for a fixture action without a timeline.
    pub(crate) locate: Option<Vec<LocateRow>>,
    /// The data's character list (NPCAvatarCharacters) by unit: the calling
    /// character alone for the fixture-action factory; the unit group's
    /// non-zero slots in slot order for the some-character factory, a unit
    /// without a presenter kept (a null entry in the source); every slot's
    /// presenter in slot order, the missing ones dropped, for the waiting
    /// factory.
    pub(crate) members: Vec<u32>,
    /// GetTargetPosition's bool (the some-character factory ignores it).
    pub(crate) target_found: bool,
}

/// What the fixture-talk factories read from the host besides the tables:
/// the avatar store and the placed fixture's geometry. Every `Err` names host
/// data it cannot resolve (a gap, not a source state).
pub(crate) trait FixtureTalkHost {
    /// AvatarDataStore.FindNPC(unit).Position; `None` without a presenter.
    fn npc_position(&self, unit: u32) -> Option<[f32; 3]>;
    /// The StartLoc names of the fixture's action points, in the fixture's
    /// action-point array order.
    fn action_point_names(&self, fixture: &str) -> Result<Vec<String>, Halt>;
    /// fixture.ActionPoints[index].StartLoc world position and rotation
    /// (xyzw). The index is within the array.
    fn start_loc(&self, fixture: &str, index: usize) -> Result<([f32; 3], [f32; 4]), Halt>;
    /// SitePosition.y of the site the fixture stands on; `None` without a
    /// site.
    fn site_height(&self, fixture: &str) -> Result<Option<f32>, Halt>;
    /// GetLittleFarPosition(position, fixture, 2): the surface hit of the
    /// first ring cell that samples, `None` when none does (or the site has
    /// no floor grid).
    fn little_far_position(&self, position: [f32; 3], fixture: &str) -> Result<Option<[f32; 3]>, Halt>;
    /// NavMesh.SamplePosition(point, radius, all areas).hit.
    fn sample_hit(&self, point: [f32; 3], radius: f32) -> bool;
}

fn condition_kind(kind: &str) -> Option<i32> {
    condition_type_discriminant(kind)
}

/// IsGeneralTalk(talk): any condition row of type 0, 1 or 4.
pub(crate) fn is_general_talk(tables: &FixtureActivityTables, talk_id: i32) -> Result<bool, String> {
    Ok(tables
        .talk_conditions(talk_id)?
        .iter()
        .any(|(kind, _)| is_general_kind(kind)))
}

fn is_general_kind(kind: &str) -> bool {
    matches!(
        condition_kind(kind),
        Some(
            CONDITION_READ_EVENT_STORY_EPISODE_ID
                | CONDITION_MYSEKAI_PHENOMENA_ID
                | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT
        )
    )
}

impl LotteryScene<'_> {
    fn npc(&self, unit: u32) -> Option<&NpcView> {
        self.npcs.iter().find(|npc| npc.unit == unit)
    }

    fn conditions(&self, talk_id: i32) -> Result<Vec<(&str, i32)>, Halt> {
        self.tables
            .talk_conditions(talk_id)
            .map_err(|reason| Halt::fault(format!("talk conditions: {reason}")))
    }

    /// The non-zero units of a talk's speaker group, in slot order.
    fn members(&self, talk_id: i32) -> Result<Vec<u32>, Halt> {
        let master = self
            .tables
            .talk_master(talk_id)
            .ok_or_else(|| Halt::fault(format!("talk {talk_id} has no master row")))?;
        self.tables
            .unit_ids_of_group(master.unit_group_id)
            .map_err(Halt::fault)
    }

    /// First non-zero unit of the speaker group, else 0 (a missing group 0).
    fn first_member(&self, talk_id: i32) -> i32 {
        let Some(master) = self.tables.talk_master(talk_id) else {
            return 0;
        };
        self.tables
            .unit_group_slots(master.unit_group_id)
            .and_then(|slots| slots.into_iter().flatten().find(|unit| *unit != 0))
            .unwrap_or(0)
    }

    /// Whether one of the talk's site group's sites has this site type. A
    /// site the master site table lacks is the source's null dereference.
    fn site_matches(&self, talk_id: i32, site_type: Option<i32>) -> Result<bool, Halt> {
        let master = self
            .tables
            .talk_master(talk_id)
            .ok_or_else(|| Halt::fault(format!("talk {talk_id} has no master row")))?;
        let sites = self.tables.site_group_sites(master.site_group_id).ok_or_else(|| {
            Halt::gap(format!(
                "site group {} of talk {talk_id} is absent from the host tables",
                master.site_group_id
            ))
        })?;
        let site_type = site_type
            .ok_or_else(|| Halt::gap("the character's site type is not in the site table"))?;
        for site in sites {
            let kind = (self.site_type_of_site)(*site).ok_or_else(|| {
                Halt::fault(format!("site {site} of group {} has no master site row", master.site_group_id))
            })?;
            if kind == site_type {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn placed_of(&self, fixture_id: i32) -> impl Iterator<Item = &PlacedFixture> {
        self.fixtures.iter().filter(move |f| f.fixture_id == fixture_id)
    }

    fn fixture_gates(&self) -> &FixtureGateInputs<'_> {
        self.fixture_gates
            .as_ref()
            .expect("a scene that reaches a fixture gate carries the fixture-gate inputs")
    }
}

/// The first row of a condition list whose type is the fixture-id type.
fn first_fixture_row(conditions: &[(&str, i32)]) -> Option<i32> {
    conditions
        .iter()
        .find(|(kind, _)| condition_kind(kind) == Some(CONDITION_MYSEKAI_FIXTURE_ID))
        .map(|(_, value)| *value)
}

/// The general lottery for `seeker` (see the module notes).
pub(crate) fn lottery_general_talk_id(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
    use_word: &'static str,
) -> Result<GeneralPick, Halt> {
    let mut pool = Vec::new();
    for row in scene.talk_list {
        let talk = row.talk_id;
        // 1: the speaker group resolves to exactly this character.
        let members = scene.members(talk)?;
        if !(members.len() == 1 && members[0] == seeker.unit) {
            continue;
        }
        // 2: a general condition row.
        let conditions = scene.conditions(talk)?;
        if !conditions.iter().any(|(kind, _)| is_general_kind(kind)) {
            continue;
        }
        // 3: not this character's own previous talk.
        if seeker.previous_talk_id != 0 && seeker.previous_talk_id == talk {
            continue;
        }
        // 4: every row is read; any row type outside the general set raises.
        let mut matched = false;
        for (kind, value) in &conditions {
            match condition_kind(kind) {
                Some(CONDITION_READ_EVENT_STORY_EPISODE_ID | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT) => {
                    matched = true
                }
                Some(CONDITION_MYSEKAI_PHENOMENA_ID) => matched |= *value == scene.phenomena_id,
                _ => {
                    return Err(Halt::fault(format!(
                        "general condition of talk {talk} has row type {kind} (argument out of range)"
                    )))
                }
            }
        }
        if !matched {
            continue;
        }
        // 5: the site group holds this character's own site type.
        if !scene.site_matches(talk, seeker.site_type)? {
            continue;
        }
        // 6: playable fixture condition: a talk that reached here has no
        // fixture row (the fourth filter raised on one), so it is true.
        pool.push(talk);
    }
    if pool.is_empty() && seeker.previous_talk_id == 0 {
        // With no talk kept and no previous talk the source still takes its
        // engine range over the empty array, Range(0, 0) = 0, and then
        // raises on the index: the draw is made and recorded before the
        // fault.
        IntDraw { draws, use_word }.draw(0);
    }
    let pick = law::lottery_general_talk_id(
        &pool,
        seeker.previous_talk_id,
        &mut IntDraw { draws, use_word },
    )?;
    Ok(pick)
}

/// IsMatchedFixtureTagCondition for a condition whose value is `fixture_id`
/// (see the module notes).
fn matches_fixture_tag_condition(
    scene: &LotteryScene<'_>,
    talk: i32,
    fixture_id: i32,
) -> Result<bool, Halt> {
    let tags = match scene.fixture_gates().extras.map(|extras| &extras.fixture_tags) {
        Some(Ok(tags)) => tags,
        Some(Err(reason)) => return Err(Halt::gap(reason.clone())),
        None => return Err(Halt::gap("the fixture tag-group table is not loaded")),
    };
    let tag_ids_of = |fixture: i32, what: &str| -> Result<Vec<i32>, Halt> {
        let group = tags.group_of_fixture.get(&fixture).ok_or_else(|| {
            Halt::fault(format!(
                "talk {talk}: {what} fixture {fixture} has no master row (null dereference)"
            ))
        })?;
        tags.tag_ids(*group).ok_or_else(|| {
            Halt::fault(format!(
                "talk {talk}: tag group {group} of fixture {fixture} is absent (null dereference)"
            ))
        })
    };
    // The condition's tags are read (ToArray) before the placed fixtures.
    let wanted = tag_ids_of(fixture_id, "the condition's")?;
    let mut placed: Vec<i32> = Vec::new();
    for fixture in scene.fixtures {
        for tag in tag_ids_of(fixture.fixture_id, "placed")? {
            if !placed.contains(&tag) {
                placed.push(tag);
            }
        }
    }
    Ok(wanted.iter().any(|tag| placed.contains(tag)))
}

/// `CanPlayMultiCharacterFixtureActionStatus` of one character.
fn can_play_multi_character_fixture_action(npc: &NpcView) -> bool {
    npc.talk_type != Some(TalkType::MultipleCharacterFixture)
        && !matches!(
            npc.objective,
            Some(
                ObjectiveType::MainSomeCharacterCommunication
                    | ObjectiveType::SubCharacterFixtureAction
                    | ObjectiveType::SomeCharacterFixtureActionSub
            )
        )
        && !matches!(npc.state, 16 | 17 | 18 | 19 | 4 | 11 | 12 | 14)
}

/// The eight lottery gates of one talk for `seeker`, or (with `store`, every
/// NPC of the avatar store as unit and site type) the nine filters of the
/// fixture-action talk list. Gates are pure, so a gap in one gate is only
/// reported when no later gate rejects the talk.
///
/// The fixture-action filters are the lottery's gates 1 to 7 (its first two
/// in the other order; both are pure and raise nothing, so the order does
/// not show), then the pre-action timeline test, then the sub-character site
/// test; the lottery's gate-delay gate is not among them.
fn matches_lottery_conditions(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    store: Option<&[(u32, Option<i32>)]>,
) -> Result<bool, Halt> {
    let conditions = scene.conditions(talk)?;
    let fixture_value = first_fixture_row(&conditions);
    let seeker_site = seeker.site_type;
    let mut pending_gap: Option<String> = None;
    let mut gates: Vec<Box<dyn Fn() -> Result<bool, Halt> + '_>> = Vec::with_capacity(9);
    // 1: the talk's first speaker is this character.
    gates.push(Box::new(move || Ok(scene.first_member(talk) == seeker.unit as i32)));
    // 2: not this character's previous talk.
    gates.push(Box::new(move || {
        Ok(!(seeker.previous_talk_id != 0 && seeker.previous_talk_id == talk))
    }));
    // 3: the conditions match (an OR over every row).
    let rows = conditions.clone();
    gates.push(Box::new(move || {
        let mut matched = false;
        for (kind, value) in &rows {
            match condition_kind(kind) {
                Some(CONDITION_READ_EVENT_STORY_EPISODE_ID | CONDITION_MYSEKAI_CHARACTER_VISIT_COUNT) => {
                    matched = true
                }
                Some(CONDITION_MYSEKAI_PHENOMENA_ID) => matched |= *value == scene.phenomena_id,
                Some(CONDITION_MYSEKAI_FIXTURE_ID) => {
                    // The talk's first fixture row names the fixture id;
                    // true when one of those fixtures stands on this site.
                    matched |= fixture_value.is_some_and(|id| {
                        scene
                            .placed_of(id)
                            .any(|f| f.located_site_type.is_some() && f.located_site_type == seeker_site)
                    });
                }
                Some(CONDITION_MYSEKAI_FIXTURE_TAG_ID) => {
                    matched |= matches_fixture_tag_condition(scene, talk, *value)?;
                }
                Some(CONDITION_AFTER_SET_FIXTURE) => {}
                _ => {
                    return Err(Halt::fault(format!(
                        "condition of talk {talk} has row type {kind} (argument out of range)"
                    )))
                }
            }
        }
        Ok(matched)
    }));
    // 4: a fixture of the talk's fixture id stands on this site.
    gates.push(Box::new(move || {
        Ok(fixture_value.is_none_or(|id| {
            scene
                .placed_of(id)
                .any(|f| f.located_site_type.is_some() && f.located_site_type == seeker_site)
        }))
    }));
    // 5: one of them has a motion area that overlaps nothing.
    gates.push(Box::new(move || {
        let Some(id) = fixture_value else {
            return Ok(true);
        };
        for fixture in scene.placed_of(id) {
            match &fixture.motion_overlap {
                Ok(false) => return Ok(true),
                Ok(true) => {}
                Err(reason) => {
                    return Err(Halt::gap(format!(
                        "motion area of {} cannot be evaluated: {reason}",
                        fixture.uid
                    )))
                }
            }
        }
        Ok(false)
    }));
    // 6: the fixture condition is playable.
    gates.push(Box::new(move || {
        let Some(id) = fixture_value else {
            return Ok(true);
        };
        for member in scene.members(talk)? {
            match scene.npc(member) {
                None => return Ok(false),
                Some(npc) if !can_play_multi_character_fixture_action(npc) => return Ok(false),
                Some(_) => {}
            }
        }
        for fixture in scene.placed_of(id) {
            match (scene.fixture_gates().admissible)(talk, fixture) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(reason) => {
                    return Err(Halt::gap(format!(
                        "talk {talk}: the fixture admissibility pair of {} cannot be evaluated: {reason}",
                        fixture.uid
                    )))
                }
            }
        }
        Ok(false)
    }));
    // 7: the site group holds this character's site type.
    gates.push(Box::new(move || scene.site_matches(talk, seeker_site)));
    match store {
        // 8: a talk naming the gate fixture waits for the gate delay.
        None => gates.push(Box::new(move || {
            let Some(id) = fixture_value else {
                return Ok(true);
            };
            let Some(gate) = scene.fixtures.iter().find(|f| f.is_gate) else {
                return Ok(false);
            };
            if gate.fixture_id != id {
                return Ok(true);
            }
            // The configured delay is an integer compared as a float.
            Ok((scene.fixture_gates().gate_action_elapsed_seconds as f32) < seeker.since_initialized)
        })),
        Some(store) => {
            // IsUseTimeline: the talk's pre-action exists and names a
            // timeline group (a group id of 0 names none).
            gates.push(Box::new(move || {
                Ok(fixture_action::is_use_timeline(
                    scene
                        .tables
                        .pre_action_of(talk)
                        .map(|pre| pre.timeline_group_id.unwrap_or(0)),
                ))
            }));
            // IsMatchedSubCharacterSite: when this character is one of the
            // talk's speakers, every NPC of the store stands on this
            // character's site type (an empty store passes); otherwise it
            // passes.
            gates.push(Box::new(move || {
                let sites: Vec<Option<i32>> = store.iter().map(|(_, site)| *site).collect();
                Ok(fixture_action::sub_character_site_matches(
                    scene.members(talk)?.contains(&seeker.unit),
                    seeker_site,
                    &sites,
                ))
            }));
        }
    }
    for gate in gates {
        match gate() {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(Halt::Gap(reason)) => {
                pending_gap.get_or_insert(reason);
            }
            Err(Halt::Fault(reason)) => {
                return Err(match pending_gap {
                    // Whether the source reaches this raise depends on the
                    // earlier gate this host could not evaluate.
                    Some(gap) => Halt::gap(format!("{gap}; then: {reason}")),
                    None => Halt::Fault(reason),
                });
            }
        }
    }
    match pending_gap {
        Some(gap) => Err(Halt::Gap(gap)),
        None => Ok(true),
    }
}

/// One fixture lottery for `seeker`: filter the list, gate it, then the
/// member-count lottery. Returns 0 when no talk passed.
pub(crate) fn lottery_fixture_talk_id(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    lottery: FixtureLottery,
    draws: &mut Draws<'_>,
) -> Result<i32, Halt> {
    let mut candidates = Vec::new();
    for row in scene.talk_list {
        let keep = match lottery {
            FixtureLottery::NotYetRead => {
                first_fixture_row(&scene.conditions(row.talk_id)?).is_some() && !row.is_read
            }
            FixtureLottery::Read | FixtureLottery::AlreadyRead => row.is_read,
        };
        if !keep {
            continue;
        }
        let passes_gates = matches_lottery_conditions(scene, seeker, row.talk_id, None)?;
        let member_count = if passes_gates {
            scene.members(row.talk_id)?.len() as i32
        } else {
            0
        };
        candidates.push(Candidate {
            talk_id: row.talk_id,
            member_count,
            passes_gates,
        });
    }
    if !candidates.iter().any(|c| c.passes_gates) {
        return Ok(0);
    }
    let word = lottery.word();
    let weights = scene.weights;
    // The weighted draw and the pick are recorded in order.
    let mut float_values = Vec::new();
    let mut pick_values = Vec::new();
    let outcome = {
        let Draws {
            engine_float,
            sequence_pick,
            ..
        } = &mut *draws;
        struct Weighted<'x> {
            inner: &'x mut dyn FnMut(f32) -> f32,
            seen: &'x mut Vec<(f32, f32)>,
        }
        impl WeightedDraw for Weighted<'_> {
            fn draw_weight(&mut self, total: f32) -> f32 {
                let value = (self.inner)(total);
                self.seen.push((value, total));
                value
            }
        }
        struct Pick<'x> {
            inner: &'x mut dyn FnMut(usize) -> Option<usize>,
            seen: &'x mut Vec<(Option<usize>, usize)>,
        }
        impl EnumerablePick for Pick<'_> {
            fn pick(&mut self, count: usize) -> Option<usize> {
                let value = (self.inner)(count);
                self.seen.push((value, count));
                value
            }
        }
        let mut weighted = Weighted {
            inner: &mut **engine_float,
            seen: &mut float_values,
        };
        let mut pick = Pick {
            inner: &mut **sequence_pick,
            seen: &mut pick_values,
        };
        law::lottery_talk_id(&candidates, &weights, &mut weighted, &mut pick)
    };
    for (value, total) in float_values {
        draws.record.push(json!({
            "use": format!("{word}:member_count"), "value": value,
            "bits": format!("{:08x}", value.to_bits()), "range": [0.0, total],
            "source": SOURCE_ENGINE_FLOAT,
        }));
    }
    for (value, count) in pick_values {
        draws.record.push(json!({
            "use": format!("{word}:talk_by_member_count"), "value": value, "range": [0, count],
            "source": SOURCE_SEQUENCE_PICK,
        }));
    }
    Ok(outcome?.talk_id)
}

/// `GetUnReadFixtureActionTalkId` for `seeker`: the unread rows of the talk
/// list, in list order, that pass the fixture-action filters (see
/// [`matches_lottery_conditions`]; `store` is every NPC of the avatar store
/// as unit and site type), and of those the talks with a fixture-id
/// condition row, as talk ids. An empty result draws nothing (the source
/// returns null when no row passes the filters, and an empty list when rows
/// pass but none has a fixture condition; its pick reads both as none).
pub(crate) fn fixture_action_talk_ids(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    store: &[(u32, Option<i32>)],
) -> Result<Vec<i32>, Halt> {
    let mut ids = Vec::new();
    for row in scene.talk_list.iter().filter(|row| !row.is_read) {
        if !matches_lottery_conditions(scene, seeker, row.talk_id, Some(store))? {
            continue;
        }
        // IsFixtureAction: any condition row of the fixture-id type (a row
        // type outside the enum never equals it).
        let kinds: Vec<i32> = scene
            .conditions(row.talk_id)?
            .iter()
            .map(|(kind, _)| condition_kind(kind).unwrap_or(-1))
            .collect();
        if fixture_action::is_fixture_action(&kinds) {
            ids.push(row.talk_id);
        }
    }
    Ok(ids)
}

/// The admissibility pair of one placed fixture for the deciding character.
fn fixture_admissible(scene: &LotteryScene<'_>, talk: i32, fixture: &PlacedFixture) -> Result<bool, Halt> {
    (scene.fixture_gates().admissible)(talk, fixture).map_err(|reason| {
        Halt::gap(format!(
            "talk {talk}: the fixture admissibility pair of {} cannot be evaluated: {reason}",
            fixture.uid
        ))
    })
}

/// The source's exception when no placed fixture of the talk's fixture
/// condition is admissible.
fn no_admissible_fixture(talk: i32, fixture_id: i32) -> Halt {
    Halt::fault(format!(
        "talk {talk}: no furniture satisfying the lottery conditions (fixture {fixture_id})"
    ))
}

/// The first admissible fixture of the talk's fixture condition, in the
/// fixture manager's order (three argument form); none raises.
fn target_character_talk_fixture_by_condition(
    scene: &LotteryScene<'_>,
    talk: i32,
    fixture_id: i32,
) -> Result<Option<String>, Halt> {
    for fixture in scene.placed_of(fixture_id) {
        if fixture_admissible(scene, talk, fixture)? {
            return Ok(Some(fixture.uid.clone()));
        }
    }
    Err(no_admissible_fixture(talk, fixture_id))
}

/// The fixture chooser of the already-read and fixture-common paths (two
/// argument form): the talk's first fixture row (none gives no fixture),
/// every present member free for a multi-character action, then the first
/// admissible fixture.
fn target_character_talk_fixture(
    scene: &LotteryScene<'_>,
    talk: i32,
) -> Result<Option<String>, Halt> {
    if scene.tables.talk_master(talk).is_none() {
        return Ok(None);
    }
    let conditions = scene.conditions(talk)?;
    let Some(fixture_id) = first_fixture_row(&conditions) else {
        return Ok(None);
    };
    for member in scene.members(talk)? {
        if let Some(npc) = scene.npc(member) {
            if !can_play_multi_character_fixture_action(npc) {
                return Ok(None);
            }
        }
    }
    target_character_talk_fixture_by_condition(scene, talk, fixture_id)
}

/// The row-1 fixture chooser: the talk's first fixture row (none gives no
/// fixture and no draw); every admissible fixture of that id in the fixture
/// manager's order (a set, so each once), then one sequence pick among them;
/// none admissible raises.
fn random_character_talk_fixture(
    scene: &LotteryScene<'_>,
    talk: i32,
    draws: &mut Draws<'_>,
) -> Result<Option<String>, Halt> {
    if scene.tables.talk_master(talk).is_none() {
        return Ok(None);
    }
    let conditions = scene.conditions(talk)?;
    let Some(fixture_id) = first_fixture_row(&conditions) else {
        return Ok(None);
    };
    let mut admissible: Vec<&PlacedFixture> = Vec::new();
    for fixture in scene.placed_of(fixture_id) {
        if fixture_admissible(scene, talk, fixture)? && !admissible.iter().any(|kept| kept.uid == fixture.uid) {
            admissible.push(fixture);
        }
    }
    if admissible.is_empty() {
        return Err(no_admissible_fixture(talk, fixture_id));
    }
    let count = admissible.len();
    let index = (draws.sequence_pick)(count)
        .expect("a sequence pick over a non-empty set yields an index");
    draws.record.push(json!({
        "use": "row1_fixture_pick", "value": index, "range": [0, count],
        "source": SOURCE_SEQUENCE_PICK,
    }));
    Ok(Some(admissible[index].uid.clone()))
}

/// TryFindActionPointByGameCharacterUnitId: the first locator, in the
/// fixture's action-point order, whose StartLoc name's second '_' field,
/// reduced to its digits, parses to the action-point value. A name with
/// fewer than two fields raises (index out of range). Locator names are
/// ASCII, so the digit filter is the ASCII one.
fn find_locator(names: &[String], value: i32) -> Result<Option<usize>, Halt> {
    for (index, name) in names.iter().enumerate() {
        let parts: Vec<&str> = name.split('_').collect();
        if parts.len() < 2 {
            return Err(Halt::fault(format!(
                "locator name {name} has no second '_' field (index out of range)"
            )));
        }
        let digits: String = parts[1].chars().filter(char::is_ascii_digit).collect();
        if digits.parse::<i32>().ok() == Some(value) {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// TryGenerateFixtureActionLocateData for one member slot: the matched
/// locator's index, and its slot: the StartLoc name with "loc_start" removed,
/// split on '_'; with two or more fields the second field's integer (0 when
/// it does not parse), otherwise the member slot. A value of -1 logs an
/// invalid name and makes no row.
fn generate_locate(
    member_slot: usize,
    unit: i32,
    value: i32,
    names: &[String],
) -> Result<Option<LocateRow>, Halt> {
    let Some(index) = find_locator(names, value)? else {
        return Ok(None);
    };
    let stripped = names[index].replace("loc_start", "");
    let parts: Vec<&str> = stripped.split('_').collect();
    let slot_id = if parts.len() >= 2 {
        parts[1].parse::<i32>().unwrap_or(0)
    } else {
        member_slot as i32
    };
    if value == -1 {
        return Ok(None);
    }
    Ok(Some(LocateRow {
        unit,
        index,
        slot_id,
    }))
}

fn locate_json(locate: &[LocateRow]) -> Value {
    Value::Array(
        locate
            .iter()
            .map(|row| json!([row.unit, row.index, row.slot_id]))
            .collect(),
    )
}

/// CreateFixtureNpcActionLocateDataList for a fixture whose StartLoc names
/// `names` gives (read only once the timeline's action-point row exists):
/// `None` for an absent action-point row or unit group; otherwise one row per
/// member slot whose unit and action-point column are non-zero and whose
/// locator is found.
pub(crate) fn locate_rows(
    tables: &FixtureActivityTables,
    unit_group_id: i32,
    timeline: i32,
    names: impl FnOnce() -> Result<Vec<String>, Halt>,
) -> Result<Option<Vec<LocateRow>>, Halt> {
    let timeline = tables
        .timeline(timeline)
        .ok_or_else(|| Halt::fault(format!("timeline {timeline} has no master row")))?;
    let Some(columns) = tables.action_point_columns(timeline.action_point_definition) else {
        return Ok(None);
    };
    let names = names()?;
    let Some(slots) = tables.unit_group_slots(unit_group_id) else {
        return Ok(None);
    };
    let mut rows = Vec::new();
    for (slot, unit) in slots.iter().enumerate() {
        let unit = unit.unwrap_or(0);
        if unit == 0 {
            continue;
        }
        // The fifth column is absent from every shipped row (0).
        let value = columns.get(slot).copied().flatten().unwrap_or(0);
        if value == 0 {
            continue;
        }
        if let Some(row) = generate_locate(slot, unit, value, &names)? {
            rows.push(row);
        }
    }
    Ok(Some(rows))
}

impl std::fmt::Display for Halt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Halt::Fault(reason) => write!(f, "source exception: {reason}"),
            Halt::Gap(reason) => write!(f, "host gap: {reason}"),
        }
    }
}

/// The deciding character's locate row index (FirstOrDefault on the unit
/// id), else the default row's index 0.
fn own_index(locate: &[LocateRow], unit: u32) -> usize {
    locate
        .iter()
        .find(|row| row.unit == unit as i32)
        .map_or(0, |row| row.index)
}

impl LotteryScene<'_> {
    fn host(&self) -> &dyn FixtureTalkHost {
        self.fixture_host
            .expect("a scene that reaches a fixture-talk factory carries the fixture-talk host")
    }

    /// CreateFixtureNpcActionLocateDataList(fixture, unit group, timeline):
    /// `None` for no fixture, no timeline, an absent action-point row or an
    /// absent unit group; otherwise one row per member slot whose unit and
    /// action-point column are non-zero and whose locator is found.
    fn locate_list(
        &self,
        fixture: Option<&str>,
        unit_group_id: i32,
        timeline: Option<i32>,
    ) -> Result<Option<Vec<LocateRow>>, Halt> {
        let (Some(fixture), Some(timeline)) = (fixture, timeline) else {
            return Ok(None);
        };
        locate_rows(self.tables, unit_group_id, timeline, || {
            self.host().action_point_names(fixture)
        })
    }

    /// CreateFixtureNpcActionLocateDataFprWhileDoingTalk(master, fixture):
    /// the talk's unit group, its pre-action's together communication and
    /// that row's action points, one row per member slot as in
    /// [`Self::locate_list`]. Nothing for an absent group, pre-action or
    /// communication row; an absent action-point row raises once a non-zero
    /// member slot is reached. The source enumerates it lazily three times;
    /// every pass computes the same rows, so one eager pass raises exactly
    /// when some pass does.
    fn locate_list_while_doing(&self, talk: i32, fixture: &str) -> Result<Vec<LocateRow>, Halt> {
        let master = self
            .tables
            .talk_master(talk)
            .ok_or_else(|| Halt::fault(format!("talk {talk} has no master row")))?;
        let Some(slots) = self.tables.unit_group_slots(master.unit_group_id) else {
            return Ok(Vec::new());
        };
        let Some(pre) = self.tables.pre_action_of(talk) else {
            return Ok(Vec::new());
        };
        let together = match self.together {
            Some(Ok(table)) => table,
            Some(Err(reason)) => return Err(Halt::gap(reason.clone())),
            None => return Err(Halt::gap("the together-communication table is not loaded")),
        };
        let Some(action_point) = pre
            .fixture_together_communication_id
            .and_then(|id| together.get(&id))
        else {
            return Ok(Vec::new());
        };
        let columns = self.tables.action_point_columns(*action_point);
        let names = self.host().action_point_names(fixture)?;
        let mut rows = Vec::new();
        for (slot, unit) in slots.iter().enumerate() {
            let unit = unit.unwrap_or(0);
            if unit == 0 {
                continue;
            }
            let columns = columns.ok_or_else(|| {
                Halt::fault(format!(
                    "talk {talk}: together-communication action-point row {action_point} is absent (null dereference)"
                ))
            })?;
            let value = columns.get(slot).copied().flatten().unwrap_or(0);
            if value == 0 {
                continue;
            }
            if let Some(row) = generate_locate(slot, unit, value, &names)? {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// The pre-action's fixture timeline: one sequence pick over its group's
    /// timelines (a fresh generator; an empty group raises after the pick's
    /// count). `None` without a pre-action or with group 0.
    fn pick_timeline(
        &self,
        talk: i32,
        draws: &mut Draws<'_>,
        use_word: &'static str,
    ) -> Result<Option<i32>, Halt> {
        let group = self
            .tables
            .pre_action_of(talk)
            .and_then(|pre| pre.timeline_group_id)
            .unwrap_or(0);
        if group == 0 {
            return Ok(None);
        }
        let timelines: Vec<i32> = self.tables.timeline_rows(group).map(|row| row.id).collect();
        let index = (draws.sequence_pick)(timelines.len());
        draws.record.push(json!({
            "use": use_word, "value": index, "range": [0, timelines.len()],
            "group": group, "source": SOURCE_SEQUENCE_PICK,
        }));
        let index = index.ok_or_else(|| {
            Halt::fault(format!(
                "talk {talk}: timeline group {group} has no timeline (sequence pick on an empty set)"
            ))
        })?;
        Ok(Some(timelines[index]))
    }

    /// GetTargetPosition(npc position, master, fixture, index) for a talk
    /// with a master row (see the module notes). The arm and its inputs go
    /// into `steps`.
    fn target_position(
        &self,
        npc_position: [f32; 3],
        talk: i32,
        fixture: &str,
        index: usize,
        steps: &mut Vec<Value>,
    ) -> Result<(bool, [f32; 3]), Halt> {
        let host = self.host();
        let pre = self.tables.pre_action_of(talk);
        let little_far = |miss: [f32; 3], arm: &str, steps: &mut Vec<Value>| {
            let hit = host.little_far_position(npc_position, fixture)?;
            steps.push(json!({ "target_arm": arm, "little_far": hit }));
            Ok::<_, Halt>(match hit {
                Some(point) => (true, point),
                None => (false, miss),
            })
        };
        let Some(pre) = pre else {
            return little_far([0.0; 3], "no_pre_action", steps);
        };
        let waits = pre.fixture_together_communication_id.is_some_and(|id| id > 0);
        if pre.timeline_group_id.unwrap_or(0) == 0 && !waits {
            return little_far(npc_position, "little_far", steps);
        }
        let names = host.action_point_names(fixture)?;
        if names.is_empty() {
            // "no action point; check the fixture" is logged.
            steps.push(json!({ "target_arm": "no_action_point" }));
            return Ok((false, npc_position));
        }
        if index >= names.len() {
            return Err(Halt::fault(format!(
                "locator index {index} of {fixture} is outside its {} action points (argument out of range)",
                names.len()
            )));
        }
        let (start, _) = host.start_loc(fixture, index)?;
        let Some(site_y) = host.site_height(fixture)? else {
            steps.push(json!({ "target_arm": "no_site" }));
            return Ok((false, npc_position));
        };
        let point = [start[0], site_y, start[2]];
        let hit = host.sample_hit(point, 0.3);
        steps.push(json!({
            "target_arm": "start_loc", "index": index, "start": start, "site_y": site_y,
            "sample_hit": hit,
        }));
        Ok(if hit { (true, point) } else { (false, npc_position) })
    }
}

/// TryCreateFixtureActionSomeCharacterTalkData.
fn try_some_character_fixture(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<&str>,
    draws: &mut Draws<'_>,
    steps: &mut Vec<Value>,
) -> Result<Option<FixtureTalkData>, Halt> {
    let master = scene
        .tables
        .talk_master(talk)
        .ok_or_else(|| Halt::fault(format!("talk {talk} has no master row")))?;
    // The timeline is picked before anything else is checked.
    let timeline = scene.pick_timeline(talk, draws, "fixture_timeline_pick:some_character")?;
    let refuse = |steps: &mut Vec<Value>, result: &str| {
        steps.push(json!({ "factory": "some_character", "result": result, "timeline": timeline }));
        Ok(None)
    };
    let Some(slots) = scene.tables.unit_group_slots(master.unit_group_id) else {
        // The missing unit group is logged.
        return refuse(steps, "unit_group_absent");
    };
    let Some(picked) = timeline else {
        return refuse(steps, "no_timeline");
    };
    let members: Vec<u32> = slots
        .iter()
        .filter_map(|unit| unit.filter(|unit| *unit != 0))
        .map(|unit| unit as u32)
        .collect();
    if members.len() < 2 {
        return refuse(steps, "single_member");
    }
    let Some(locate) = scene.locate_list(fixture, master.unit_group_id, Some(picked))? else {
        return refuse(steps, "locate_null");
    };
    if locate.is_empty() {
        return refuse(steps, "locate_empty");
    }
    let fixture = fixture.expect("a locate list is only built for a fixture");
    let position = scene.host().npc_position(seeker.unit).ok_or_else(|| {
        Halt::fault(format!("unit {} has no presenter (null dereference)", seeker.unit))
    })?;
    let index = own_index(&locate, seeker.unit);
    // The bool is dropped: a target that is not found still builds.
    let (found, target) = scene.target_position(position, talk, fixture, index, steps)?;
    steps.push(json!({
        "factory": "some_character", "result": "built", "timeline": picked,
        "locate": locate_json(&locate), "index": index, "target": target, "found": found,
    }));
    Ok(Some(FixtureTalkData {
        kind: TalkType::MultipleCharacterFixture,
        talk_id: talk,
        fixture: fixture.to_owned(),
        target_position: target,
        rotation: None,
        timeline: Some(picked),
        locate: Some(locate),
        members,
        target_found: found,
    }))
}

/// ForceUpdateSomeCharacterActionFixtureObjective's data for one other member
/// of a multiple-character fixture talk: the member's position in the main
/// data's character list picks its locate row (a list shorter than that
/// position leaves the member without data); then
/// CreateSomeCharacterFixtureTimelineAITalkData: one sequence pick over the
/// pre-action's timeline group, the member's presenter, GetTargetPosition
/// from the member's position on that row's locator (not found: no data),
/// and the locator's StartLoc rotation. The main's master, fixture,
/// character list and locate list are carried over.
pub(crate) fn some_character_data_for_member(
    scene: &LotteryScene<'_>,
    draws: &mut Draws<'_>,
    member: u32,
    main: &FixtureTalkData,
) -> Result<(Option<FixtureTalkData>, Vec<Value>), Halt> {
    let mut steps = Vec::new();
    let locate = main.locate.as_deref().unwrap_or(&[]);
    let Some(position_in_list) = main.members.iter().position(|unit| *unit == member) else {
        steps.push(json!({ "factory": "some_character_member", "result": "not_in_list" }));
        return Ok((None, steps));
    };
    let Some(row) = locate.get(position_in_list).copied() else {
        // The short locate list is logged with every row.
        steps.push(json!({
            "factory": "some_character_member", "result": "locate_short",
            "position": position_in_list, "locate": locate_json(locate),
        }));
        return Ok((None, steps));
    };
    let timeline = scene.pick_timeline(main.talk_id, draws, "fixture_timeline_pick:some_character_member")?;
    let host = scene.host();
    let Some(position) = host.npc_position(member) else {
        steps.push(json!({ "factory": "some_character_member", "result": "no_presenter" }));
        return Ok((None, steps));
    };
    let (found, target) = scene.target_position(position, main.talk_id, &main.fixture, row.index, &mut steps)?;
    if !found {
        steps.push(json!({ "factory": "some_character_member", "result": "target_not_found", "index": row.index }));
        return Ok((None, steps));
    }
    let (_, rotation) = host.start_loc(&main.fixture, row.index)?;
    steps.push(json!({
        "factory": "some_character_member", "result": "built", "timeline": timeline,
        "index": row.index, "target": target, "rotation": rotation,
    }));
    Ok((
        Some(FixtureTalkData {
            kind: TalkType::MultipleCharacterFixture,
            talk_id: main.talk_id,
            fixture: main.fixture.clone(),
            target_position: target,
            rotation: Some(rotation),
            timeline,
            locate: main.locate.clone(),
            members: main.members.clone(),
            target_found: true,
        }),
        steps,
    ))
}

/// TryCreateWaitingWithCommunicationTalkData, the 5-argument overload: only
/// a pre-action that waits with a communication reaches the body. No draw.
fn try_waiting_with_communication(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<&str>,
    steps: &mut Vec<Value>,
) -> Result<Option<FixtureTalkData>, Halt> {
    let waits = scene
        .tables
        .pre_action_of(talk)
        .is_some_and(|pre| pre.fixture_together_communication_id.is_some_and(|id| id > 0));
    if !waits {
        return Ok(None);
    }
    waiting_with_communication(scene, seeker, talk, fixture, steps)
}

/// ForceUpdateCommunicationWhileDoingWaitObjective's data for one member of
/// a while-doing-wait talk: the 4-argument overload of
/// TryCreateWaitingWithCommunicationTalkData with that member as the
/// character (its locate row picks its locator; it is the data's main). No
/// draw. The factory's steps come back with the data.
pub(crate) fn waiting_data_for_member(
    scene: &LotteryScene<'_>,
    member: &NpcView,
    talk: i32,
    fixture: &str,
) -> Result<(Option<FixtureTalkData>, Vec<Value>), Halt> {
    let mut steps = Vec::new();
    let data = waiting_with_communication(scene, member, talk, Some(fixture), &mut steps)?;
    Ok((data, steps))
}

/// TryCreateWaitingWithCommunicationTalkData, the 4-argument overload.
fn waiting_with_communication(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<&str>,
    steps: &mut Vec<Value>,
) -> Result<Option<FixtureTalkData>, Halt> {
    let Some(fixture) = fixture else {
        steps.push(json!({ "factory": "waiting_with_communication", "result": "no_fixture" }));
        return Ok(None);
    };
    let master = scene
        .tables
        .talk_master(talk)
        .ok_or_else(|| Halt::fault(format!("talk {talk} has no master row")))?;
    let slots = scene.tables.unit_group_slots(master.unit_group_id).ok_or_else(|| {
        Halt::fault(format!(
            "talk {talk}: unit group {} is absent (null dereference)",
            master.unit_group_id
        ))
    })?;
    let locate = scene.locate_list_while_doing(talk, fixture)?;
    if locate.is_empty() {
        steps.push(json!({ "factory": "waiting_with_communication", "result": "locate_empty" }));
        return Ok(None);
    }
    let index = own_index(&locate, seeker.unit);
    let names = scene.host().action_point_names(fixture)?;
    if index >= names.len() {
        return Err(Halt::fault(format!(
            "locator index {index} of {fixture} is outside its {} action points (argument out of range)",
            names.len()
        )));
    }
    let (start, rotation) = scene.host().start_loc(fixture, index)?;
    let site_y = scene.host().site_height(fixture)?.ok_or_else(|| {
        Halt::fault(format!("{fixture} stands on no site (null dereference)"))
    })?;
    let target = [start[0], site_y, start[2]];
    let members: Vec<u32> = slots
        .iter()
        .map(|unit| unit.unwrap_or(0))
        .filter(|unit| *unit > 0 && scene.host().npc_position(*unit as u32).is_some())
        .map(|unit| unit as u32)
        .collect();
    steps.push(json!({
        "factory": "waiting_with_communication", "result": "built",
        "locate": locate_json(&locate), "index": index, "target": target, "rotation": rotation,
    }));
    Ok(Some(FixtureTalkData {
        kind: TalkType::CommunicationWhileDoingWait,
        talk_id: talk,
        fixture: fixture.to_owned(),
        target_position: target,
        rotation: Some(rotation),
        timeline: None,
        locate: Some(locate),
        members,
        target_found: true,
    }))
}

/// TryCreateFixtureActionTalkData.
fn try_fixture_action(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<&str>,
    draws: &mut Draws<'_>,
    steps: &mut Vec<Value>,
) -> Result<Option<FixtureTalkData>, Halt> {
    let Some(fixture) = fixture else {
        return Ok(None);
    };
    let master = scene
        .tables
        .talk_master(talk)
        .ok_or_else(|| Halt::fault(format!("talk {talk} has no master row")))?;
    let timeline = scene.pick_timeline(talk, draws, "fixture_timeline_pick:fixture_action")?;
    let (locate, index) = match timeline {
        None => (None, 0),
        Some(picked) => {
            if scene.tables.unit_group_slots(master.unit_group_id).is_none() {
                return Err(Halt::fault(format!(
                    "talk {talk}: unit group {} is absent (null dereference)",
                    master.unit_group_id
                )));
            }
            let locate = scene
                .locate_list(Some(fixture), master.unit_group_id, Some(picked))?
                .ok_or_else(|| {
                    Halt::fault(format!(
                        "talk {talk}: the locate list of timeline {picked} is null (null dereference)"
                    ))
                })?;
            if locate.is_empty() {
                steps.push(json!({
                    "factory": "fixture_action", "result": "locate_empty", "timeline": picked,
                }));
                return Ok(None);
            }
            let index = own_index(&locate, seeker.unit);
            (Some(locate), index)
        }
    };
    let position = scene.host().npc_position(seeker.unit).ok_or_else(|| {
        Halt::fault(format!("unit {} has no presenter (null dereference)", seeker.unit))
    })?;
    let (found, target) = scene.target_position(position, talk, fixture, index, steps)?;
    steps.push(json!({
        "factory": "fixture_action", "result": if found { "built" } else { "target_not_found" },
        "timeline": timeline, "locate": locate.as_deref().map(locate_json), "index": index,
        "target": target, "found": found,
    }));
    if !found {
        // The data is assigned, and the caller ignores it.
        return Ok(None);
    }
    Ok(Some(FixtureTalkData {
        kind: TalkType::SingleCharacterFixture,
        talk_id: talk,
        fixture: fixture.to_owned(),
        target_position: target,
        rotation: None,
        timeline,
        locate,
        members: vec![seeker.unit],
        target_found: found,
    }))
}

/// The three fixture-talk factories in order; the first that builds wins.
fn run_fixture_factories(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<&str>,
    draws: &mut Draws<'_>,
    steps: &mut Vec<Value>,
) -> Result<Option<FixtureTalkData>, Halt> {
    if let Some(data) = try_some_character_fixture(scene, seeker, talk, fixture, draws, steps)? {
        return Ok(Some(data));
    }
    if let Some(data) = try_waiting_with_communication(scene, seeker, talk, fixture, steps)? {
        return Ok(Some(data));
    }
    try_fixture_action(scene, seeker, talk, fixture, draws, steps)
}

/// Builds data by talk id (CreateAITalkDataByTalkId): the three fixture-talk
/// factories in order, then a fresh general lottery. A general talk without
/// a fixture fails all three with no draw, so its data comes from the second
/// general draw. The factories' steps go into the record.
fn create_ai_talk_data_by_talk_id(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<String>,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    if scene.tables.talk_master(talk).is_some() {
        let fixture = fixture.as_deref();
        let mut steps = Vec::new();
        let built = run_fixture_factories(scene, seeker, talk, fixture, draws, &mut steps);
        draws.record.push(json!({
            "use": "fixture_factory_steps", "talk_id": talk, "fixture": fixture, "steps": steps,
        }));
        if let Some(data) = built? {
            return Ok(TalkPlan::Fixture(data));
        }
    }
    lottery_and_create_general_talk_data(scene, seeker, draws)
}

/// A fresh general lottery, then its data. A talk without a master row
/// leaves the factory result null, which its caller dereferences.
fn lottery_and_create_general_talk_data(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let pick = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick:redraw")?;
    let talk = pick.talk_id();
    if scene.tables.talk_master(talk).is_none() {
        return Err(Halt::fault(format!(
            "talk data for general talk {talk} is null (no master row); the caller dereferences it"
        )));
    }
    Ok(TalkPlan::General {
        talk_id: talk,
        interrupt: false,
    })
}

/// Row 1 (empty AI talk data): unread fixture talk, then read talk, then a
/// general talk; the row-1 fixture chooser; then data by talk id. The
/// factory's third argument is unused by the source body. The tutorial arm
/// is not taken: the tutorial is finished in this host.
pub(crate) fn create_ai_talk_data(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let mut talk = lottery_fixture_talk_id(scene, seeker, FixtureLottery::NotYetRead, draws)?;
    if talk == 0 {
        talk = lottery_fixture_talk_id(scene, seeker, FixtureLottery::Read, draws)?;
    }
    if talk == 0 {
        talk = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick")?.talk_id();
    }
    let fixture = random_character_talk_fixture(scene, talk, draws)?;
    create_ai_talk_data_by_talk_id(scene, seeker, talk, fixture, draws)
}

/// Branch A (an unread talk exists): unread fixture talk, then read talk,
/// then one general lottery built directly; a fixture talk id takes its first
/// fixture row's first admissible fixture, then data by talk id.
pub(crate) fn force_update_yet_read_talk_fixture_talk(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let mut talk = lottery_fixture_talk_id(scene, seeker, FixtureLottery::NotYetRead, draws)?;
    if talk == 0 {
        talk = lottery_fixture_talk_id(scene, seeker, FixtureLottery::Read, draws)?;
    }
    if talk == 0 {
        let general = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick")?.talk_id();
        if scene.tables.talk_master(general).is_none() {
            return Err(Halt::fault(format!(
                "general talk {general} has no master row (the tweet id write dereferences it)"
            )));
        }
        return Ok(TalkPlan::General {
            talk_id: general,
            interrupt: false,
        });
    }
    if scene.tables.talk_master(talk).is_none() {
        return Err(Halt::fault(format!("talk {talk} has no master row")));
    }
    let fixture = match first_fixture_row(&scene.conditions(talk)?) {
        Some(fixture_id) => target_character_talk_fixture_by_condition(scene, talk, fixture_id)?,
        None => None,
    };
    create_ai_talk_data_by_talk_id(scene, seeker, talk, fixture, draws)
}

/// Branch D: one general lottery; no master leaves the data empty; data
/// raises the Talk interrupt.
pub(crate) fn force_update_general_talk_objective(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let talk = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick")?.talk_id();
    if scene.tables.talk_master(talk).is_none() {
        return Ok(TalkPlan::Null {
            reason: format!("general talk {talk} has no master row"),
            interrupt: false,
        });
    }
    Ok(TalkPlan::General {
        talk_id: talk,
        interrupt: true,
    })
}

/// Branch F: the already-read lottery; 0 builds one general lottery
/// directly; otherwise the talk's fixture, then data by talk id.
pub(crate) fn force_update_already_read_talk_fixture_talk(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let talk = lottery_fixture_talk_id(scene, seeker, FixtureLottery::AlreadyRead, draws)?;
    if talk == 0 {
        let general = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick")?.talk_id();
        if scene.tables.talk_master(general).is_none() {
            return Err(Halt::fault(format!(
                "general talk {general} has no master row (the tweet id write dereferences it)"
            )));
        }
        return Ok(TalkPlan::General {
            talk_id: general,
            interrupt: false,
        });
    }
    let fixture = target_character_talk_fixture(scene, talk)?;
    create_ai_talk_data_by_talk_id(scene, seeker, talk, fixture, draws)
}

/// Branch G: one general lottery, its master, and the talk's fixture; no
/// fixture leaves the data empty. The Talk interrupt is raised either way.
pub(crate) fn change_fixture_common_talk_objective(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    let talk = lottery_general_talk_id(scene, seeker, draws, "general_talk_pick")?.talk_id();
    if scene.tables.talk_master(talk).is_none() {
        return Err(Halt::fault(format!("general talk {talk} has no master row")));
    }
    let Some(uid) = target_character_talk_fixture(scene, talk)? else {
        return Ok(TalkPlan::Null {
            reason: format!("general talk {talk} has no admissible fixture"),
            interrupt: true,
        });
    };
    let fixture = scene
        .fixtures
        .iter()
        .find(|fixture| fixture.uid == uid)
        .expect("the chooser returns a placed fixture")
        .fixture_id;
    let extras = scene.fixture_gates().extras;
    let (commons, groups) = match extras.map(|extras| (&extras.fixture_commons, &extras.fixture_common_groups)) {
        Some((Ok(commons), Ok(groups))) => (commons, groups),
        Some((Err(reason), _)) | Some((_, Err(reason))) => return Err(Halt::gap(reason.clone())),
        None => return Err(Halt::gap("the fixture-common tables are not loaded")),
    };
    // GetMysekaiCharacterTalkFixtureCommonByCharacterUnitId, then the rows
    // whose furniture group holds the chosen fixture's id.
    let kept: Vec<&FixtureCommonRow> = commons
        .iter()
        .filter(|common| common.unit == seeker.unit as i32)
        .filter(|common| {
            groups
                .get(&common.fixture_group_id)
                .is_some_and(|fixtures| fixtures.contains(&fixture))
        })
        .collect();
    if kept.is_empty() {
        return Ok(TalkPlan::Null {
            reason: format!(
                "general talk {talk} on {uid}: no fixture-common row of unit {} holds fixture {fixture}",
                seeker.unit
            ),
            interrupt: true,
        });
    }
    let index = (draws.sequence_pick)(kept.len())
        .expect("a sequence pick over a non-empty set yields an index");
    draws.record.push(json!({
        "use": "fixture_common_pick", "value": index, "range": [0, kept.len()],
        "source": SOURCE_SEQUENCE_PICK,
    }));
    let common = kept[index];
    let position = scene.host().npc_position(seeker.unit).ok_or_else(|| {
        Halt::fault(format!("unit {} has no presenter (null dereference)", seeker.unit))
    })?;
    let mut steps = Vec::new();
    let (found, target) = scene.target_position(position, talk, &uid, 0, &mut steps)?;
    steps.push(json!({
        "factory": "fixture_common", "result": "built", "common": common.id,
        "target": target, "found": found,
    }));
    draws.record.push(json!({
        "use": "fixture_factory_steps", "talk_id": talk, "fixture": uid, "steps": steps,
    }));
    Ok(TalkPlan::CommonFixture {
        talk_id: talk,
        fixture: uid,
        target_position: target,
        target_found: found,
        common_id: common.id,
    })
}

/// The fixture lotteries' candidate facts that do not depend on the seeker,
/// for a record: how many list rows each lottery filter keeps.
pub(crate) fn list_counts(scene: &LotteryScene<'_>) -> Value {
    let mut unread_fixture = 0;
    let mut read = 0;
    let mut seen = HashSet::new();
    for row in scene.talk_list {
        seen.insert(row.talk_id);
        if row.is_read {
            read += 1;
        } else if scene
            .tables
            .talk_conditions(row.talk_id)
            .map(|c| first_fixture_row(&c).is_some())
            .unwrap_or(false)
        {
            unread_fixture += 1;
        }
    }
    json!({ "unread_fixture": unread_fixture, "read": read, "distinct": seen.len() })
}
