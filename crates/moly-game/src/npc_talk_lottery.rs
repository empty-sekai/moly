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
//! Named gaps (reported by name if reached): the fixture admissibility pair
//! where the host's fixture admission cannot evaluate it (placement or
//! locator data it does not resolve); the fixture-tag condition (no tag
//! groups in the host); the three fixture-talk factories that need a
//! timeline, a together-communication or a fixture; the fixture-common
//! tables.

use std::collections::HashSet;

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
}

/// The fixture-talk gates' inputs for the deciding character.
pub(crate) struct FixtureGateInputs<'a> {
    /// The gate fixture delay in seconds (an integer client config).
    pub(crate) gate_action_elapsed_seconds: i32,
    /// The playable-fixture gate's admissibility pair: (talk id, placed
    /// fixture) -> usable action points and an actionable fixture; `Err`
    /// names host data it cannot evaluate.
    pub(crate) admissible: &'a dyn Fn(i32, &PlacedFixture) -> Result<bool, String>,
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
}

fn condition_kind(kind: &str) -> Option<i32> {
    condition_type_discriminant(kind)
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

/// The eight lottery gates of one talk for `seeker`. Gates are pure, so a
/// gap in one gate is only reported when no later gate rejects the talk.
fn matches_lottery_conditions(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
) -> Result<bool, Halt> {
    let conditions = scene.conditions(talk)?;
    let fixture_value = first_fixture_row(&conditions);
    let seeker_site = seeker.site_type;
    let mut pending_gap: Option<String> = None;
    let mut gates: Vec<Box<dyn Fn() -> Result<bool, Halt> + '_>> = Vec::with_capacity(8);
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
                    return Err(Halt::gap(format!(
                        "talk {talk} has a fixture-tag condition; fixture tag groups are not in the host"
                    )))
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
    // 8: a talk naming the gate fixture waits for the gate delay.
    gates.push(Box::new(move || {
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
    }));
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
        let passes_gates = matches_lottery_conditions(scene, seeker, row.talk_id)?;
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

/// Builds data by talk id: the three fixture-talk factories in order, then a
/// fresh general lottery. A general talk without a fixture fails all three
/// with no draw, so its data comes from the second general draw.
fn create_ai_talk_data_by_talk_id(
    scene: &LotteryScene<'_>,
    seeker: &NpcView,
    talk: i32,
    fixture: Option<String>,
    draws: &mut Draws<'_>,
) -> Result<TalkPlan, Halt> {
    if scene.tables.talk_master(talk).is_some() {
        let pre = scene.tables.pre_action_of(talk);
        if pre.is_some_and(|pre| pre.timeline_group_id.is_some_and(|id| id != 0)) {
            return Err(Halt::gap(format!(
                "talk {talk} has a timeline group; the some-character fixture-action factory is not ported"
            )));
        }
        if pre.is_some_and(|pre| pre.fixture_together_communication_id.is_some_and(|id| id > 0)) {
            return Err(Halt::gap(format!(
                "talk {talk} waits with a communication; that factory is not ported"
            )));
        }
        if fixture.is_some() {
            return Err(Halt::gap(format!(
                "talk {talk} has a fixture; the fixture-action factory is not ported"
            )));
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
    match target_character_talk_fixture(scene, talk)? {
        None => Ok(TalkPlan::Null {
            reason: format!("general talk {talk} has no admissible fixture"),
            interrupt: true,
        }),
        Some(uid) => Err(Halt::gap(format!(
            "talk {talk} chose fixture {uid}; the fixture-common tables are not in the host"
        ))),
    }
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
