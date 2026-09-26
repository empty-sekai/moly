//! The door answer: `HomeSiteController.ExecuteNPCRandomFixtureAction`, which
//! the room-to-home door move awaits (its core, step 5), and which the home
//! controller also starts once when the home site is set up.
//!
//! Contract with the door move: the door inserts
//! [`RandomFixtureActionPending`] at its step and waits until it is gone, but
//! only while [`RandomFixtureActionReader`] exists; this plugin inserts that
//! marker at startup. This side removes the pending resource where the
//! source's await returns.
//!
//! The source's body (see `moly_law::objective::random_fixture_action`):
//! the solo-owner-not-tutorial gate returns at once when it fails; otherwise
//! an engine-loop wait until the NPC list is non-empty, whose predicate is
//! first tested on the frame after the request; then, in that same frame, the
//! visible home NPCs are shuffled, a third of them (rounded up) is the target,
//! and at most two passes ask NPCs to take their fixture action. The await
//! returns when the second pass (or a first pass that reached the target)
//! ends, which is the frame the predicate first holds.
//!
//! The NPC's own action (`NPCAvatarPresenter.ExecuteRandomFixtureAction`),
//! evaluated for each try in pass order, after the tries before it forced
//! theirs:
//! - its unread fixture-action talks: the unread rows of the talk list that
//!   pass the fixture-action filters (see
//!   `npc_talk_lottery::fixture_action_talk_ids`);
//! - its no-talk actions: the NoTalk rows of its unit, in master order, whose
//!   fixture is placed and for which a fixture passes the no-talk tests;
//! - one engine pick of each non-empty list, then `Random.value` only when
//!   both exist (above 0.5 takes the talk);
//! - the no-talk action is forced on the character: its current objective is
//!   cancelled (nothing more when that reports false), the AI model is reset
//!   and given the action's no-talk data (the data's timeline pick draws
//!   here), the talk interrupt (4) is raised, the next rest is skipped, and
//!   the character is placed on the navigation surface within 0.25 of the
//!   action point's StartLoc when the surface has a point there. Its next
//!   decision runs the no-talk objective on that data.
//!
//! Host shape. This host has no multiplay: the player is always solo and the
//! room owner. A tutorial run returns at once in the source (the door goes on
//! in its own frame); here the answer comes on the next frame, when this side
//! first sees the request. The home controller's draws and the character's
//! share one generator, as the source's engine generator is one.
//!
//! The talk is forced on the talk's first speaker, who is this character
//! (the talk filters keep only its own talks): `ForceExecuteReadTalkFixtureTalk`
//! cancels its current objective (nothing more when that reports false),
//! then `ForceUpdateReadTalkFixtureTalk` resets the AI model and builds the
//! data (see `npc_talk_lottery::force_update_read_talk_fixture_talk`; the
//! tweet id is the pre-action's tweet), the next rest is skipped and the
//! talk interrupt (4) raised; `TrySetTalkDataSubCharacter` does nothing for
//! a data of one character; then the member is placed within 0.25 of its
//! locate row's StartLoc. Its next decision re-runs the talk objective on
//! that data.
//!
//! Not ported (named):
//! - the talk's some-character branch (a some-character row names the talk
//!   with this character as main: `CreateSomeCharacterFixtureTimelineAITalkData`)
//!   and a data of two or more characters (the others' sub-character
//!   objective 8): such a talk is logged and not forced, before the cancel.
//! - the cancelled objective's own ForceUpdateObjective cascade (a no-talk
//!   objective's cancel makes one) is not drawn: its data is replaced by the
//!   reset that follows.

use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use moly_law::objective::random_fixture_action as law;
use moly_law::objective::{InterruptMarker, TalkType};

use crate::npc::residency::Away;
use crate::npc::{CharacterUnitId, NpcActions, WalkState};
use crate::npc_objective::{ObjectiveFace, ObjectiveMind, TalkSlot};

/// Marker: a reader of [`RandomFixtureActionPending`] is installed. The door
/// move waits on the pending resource only while this exists.
#[derive(Resource, Debug, Default)]
pub(crate) struct RandomFixtureActionReader;

/// The room-to-home door move's await on
/// `HomeSiteController.ExecuteNPCRandomFixtureAction`: inserted by the door
/// at its step (with the frame it was requested on), removed here where the
/// source's await returns.
#[derive(Resource, Debug)]
pub(crate) struct RandomFixtureActionPending {
    pub(crate) requested_frame: u32,
}

/// The system set the door move orders itself after, so an answer removed on
/// a frame lets the door continue on that frame, as the source's awaiting
/// continuation runs inside the completion.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DoorAnswerSet;

/// The engine's global generator as the home controller and the characters'
/// random fixture actions draw on it.
#[derive(Resource)]
pub(crate) struct ControllerRandom(crate::npc_objective::MemberRng);

impl Default for ControllerRandom {
    fn default() -> Self {
        ControllerRandom(crate::npc_objective::MemberRng::from_platform())
    }
}

/// This side's progress on one call of the body.
#[derive(Default)]
pub(crate) struct AnswerState {
    /// The start call from the home site's setup has run.
    initial_done: bool,
    /// The wait logged that the NPC list is still empty.
    empty_logged: bool,
}

/// Why a call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caller {
    /// `HomeSiteController.Initialize` (not awaited).
    Initialize,
    /// The room-to-home door move (awaited).
    DoorMove,
}

impl Caller {
    fn word(self) -> &'static str {
        match self {
            Caller::Initialize => "HomeSiteController.Initialize (not awaited)",
            Caller::DoorMove => "MyRoomToHome core step 5 (awaited)",
        }
    }
}

/// The talk interrupt a forced no-talk action raises.
const TALK_INTERRUPT: InterruptMarker = InterruptMarker {
    marker_type: 4,
    can_interrupt: true,
};


/// The body, one call per frame at most.
pub(crate) fn answer(world: &mut World, mut state: Local<AnswerState>) {
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let pending = world
        .get_resource::<RandomFixtureActionPending>()
        .map(|pending| pending.requested_frame);
    let caller = match pending {
        // The engine-loop wait tests its predicate from the frame after the
        // request on.
        Some(requested) if frame > requested => Caller::DoorMove,
        Some(_) => return,
        None if !state.initial_done && home_is_loaded(world) => Caller::Initialize,
        None => return,
    };
    // CanExecuteNPCRandomFixtureAction: solo and owner always hold here.
    let tutorial = world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|panel| panel.is_tutorial());
    if tutorial {
        info!(
            "[npc-door] ExecuteNPCRandomFixtureAction from {}: tutorial, returns at once (the source's door goes on in its own frame; here frame {frame}, the frame after)",
            caller.word()
        );
        finish(world, caller, &mut state);
        return;
    }
    // WaitForNPCInitialization: WaitUntil(npc list is non-empty).
    let mut list = world.query_filtered::<(
        &CharacterUnitId,
        &NpcActions,
        &Visibility,
    ), Without<crate::player::PlayerControlled>>();
    let present: Vec<(u32, String, bool)> = list
        .iter(world)
        .map(|(unit, actions, visibility)| {
            (
                unit.0,
                actions.site_type.clone(),
                *visibility != Visibility::Hidden,
            )
        })
        .collect();
    if present.is_empty() {
        if !state.empty_logged {
            state.empty_logged = true;
            info!(
                "[npc-door] ExecuteNPCRandomFixtureAction from {}: the NPC list is empty; the source's wait does not return until an NPC exists",
                caller.word()
            );
        }
        return;
    }
    // GetVisibleHomeSiteCharacterIds: NPC-list order (the roster's order).
    let order: Vec<u32> = world
        .get_resource::<crate::npc::Registry>()
        .map(|registry| registry.character_unit_ids.clone())
        .unwrap_or_default();
    let home = crate::npc::residency::SITE_TYPES[0];
    let mut ids: Vec<i32> = order
        .iter()
        .filter_map(|unit| present.iter().find(|(u, ..)| u == unit))
        .filter(|(_, site, visible)| *visible && site == home)
        .map(|(unit, ..)| *unit as i32)
        .collect();
    let candidates = ids.clone();
    let mut draws = Vec::new();
    {
        let mut random = world.get_resource_or_insert_with(ControllerRandom::default);
        law::shuffle_character_ids(&mut ids, &mut |lo, hi| {
            let value = lo
                + crate::npc_objective::engine_int_draw(&mut random.0, (hi - lo) as usize) as i32;
            draws.push((hi, value));
            value
        });
    }
    let target = law::target_action_count(ids.len() as i32);
    let mut tries = Vec::new();
    let (first, second) = law::run_with_retry(&ids, target, &mut |unit| {
        let found = present.iter().any(|(u, ..)| *u as i32 == unit);
        tries.push((unit, found));
        if found {
            execute_random_fixture_action(world, unit as u32, frame)
        } else {
            law::TryOutcome::NoNpc
        }
    });
    info!(
        "[npc-door] ExecuteNPCRandomFixtureAction from {} at frame {frame}: visible home NPCs {candidates:?}, shuffle draws (bound, value) {draws:?} -> {ids:?}, target ceil(n*0.3f) = {target}, pass 1 {first}/{target}{}, tries (unit, NPC found) {tries:?}",
        caller.word(),
        match second {
            Some(count) => format!(", pass 2 {count}/{}", target - first),
            None => String::new(),
        },
    );
    finish(world, caller, &mut state);
}

/// A character's two candidate lists, or why they could not be read.
struct Candidates {
    talks: Vec<i32>,
    actions: Vec<i32>,
    /// Host data a list could not be evaluated over (that list reads as
    /// empty).
    gaps: Vec<String>,
}

/// `NPCAvatarPresenter.ExecuteRandomFixtureAction` for `unit`; returns what
/// the pass reads back from it.
fn execute_random_fixture_action(world: &mut World, unit: u32, frame: u32) -> law::TryOutcome {
    let Some(actor) = find_npc(world, unit) else {
        return law::TryOutcome::NoNpc;
    };
    let mut line = serde_json::Map::new();
    line.insert("v".into(), 1.into());
    line.insert("unit".into(), unit.into());
    line.insert("frame".into(), frame.into());
    let candidates = match candidates(world, actor, unit) {
        Ok(candidates) => candidates,
        Err(reason) => {
            line.insert("refused".into(), reason.clone().into());
            warn!("[npc-door-action] {}", serde_json::Value::Object(line));
            // Nothing is evaluated for this character: no draw, failure.
            return law::TryOutcome::Failed;
        }
    };
    line.insert("talks".into(), serde_json::json!(candidates.talks));
    line.insert("no_talk_rows".into(), serde_json::json!(candidates.actions));
    if !candidates.gaps.is_empty() {
        line.insert("gaps".into(), serde_json::json!(candidates.gaps));
    }
    // RandomPick of each list (none for an empty one), then Random.value
    // only when both are non-empty.
    let mut draw_log = Vec::new();
    let (talk, action, pick) = world.resource_scope(|_, mut random: Mut<ControllerRandom>| {
        let talk = (!candidates.talks.is_empty()).then(|| {
            let index =
                crate::npc_objective::engine_int_draw(&mut random.0, candidates.talks.len());
            draw_log.push(serde_json::json!({"use": "talk_pick", "value": index, "range": [0, candidates.talks.len()]}));
            candidates.talks[index]
        });
        let action = (!candidates.actions.is_empty()).then(|| {
            let index =
                crate::npc_objective::engine_int_draw(&mut random.0, candidates.actions.len());
            draw_log.push(serde_json::json!({"use": "no_talk_pick", "value": index, "range": [0, candidates.actions.len()]}));
            candidates.actions[index]
        });
        let pick = law::pick_npc_fixture_action(talk.is_some(), action.is_some(), &mut || {
            let value = crate::npc_objective::engine_float_draw(&mut random.0, 1.0);
            draw_log.push(serde_json::json!({"use": "talk_or_no_talk", "value": value, "bits": format!("{:08x}", value.to_bits())}));
            value
        });
        (talk, action, pick)
    });
    let reports_success = match pick {
        law::NpcFixtureActionPick::NoTalk { reports_success } => {
            let row = action.expect("the no-talk arm has an action");
            line.insert("pick".into(), "no_talk".into());
            line.insert("no_talk_row".into(), row.into());
            force_no_talk_action(world, actor, unit, row, frame, &mut line, &mut draw_log);
            reports_success
        }
        law::NpcFixtureActionPick::Talk { reports_success } => {
            let talk = talk.expect("the talk arm has a talk");
            line.insert("pick".into(), "talk".into());
            line.insert("talk_id".into(), talk.into());
            force_read_talk(world, actor, unit, talk, frame, &mut line, &mut draw_log);
            reports_success
        }
        law::NpcFixtureActionPick::Nothing => {
            line.insert("pick".into(), "nothing".into());
            false
        }
    };
    line.insert("draws".into(), serde_json::Value::Array(draw_log));
    line.insert("returns".into(), reports_success.into());
    info!("[npc-door-action] {}", serde_json::Value::Object(line));
    if !reports_success {
        return law::TryOutcome::Failed;
    }
    // The pass reads the talk data the character now carries.
    let data = world
        .get::<TalkSlot>(actor)
        .and_then(|slot| slot.current.as_ref())
        .map(|data| {
            (
                data.characters
                    .iter()
                    .map(|unit| *unit as i32)
                    .collect::<Vec<_>>(),
                data.kind == TalkType::MultipleCharacterFixture,
            )
        });
    law::TryOutcome::Succeeded {
        multiple: data.as_ref().is_some_and(|(_, multiple)| *multiple),
        cast: data.map(|(cast, _)| cast),
    }
}

/// Why the forced read talk is not forced on this host, read before the
/// cancel from the tables alone: the some-character branch, or a data of two
/// or more characters (whose others take objective 8).
fn read_talk_unported(world: &mut World, unit: u32, talk: i32) -> Option<String> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Option<Res<crate::npc_talk_lottery::TalkExtraTables>>,
    )>::new(world);
    let (factory, extras) = params.get_mut(world);
    let tables = factory.tables()?;
    let master = tables.talk_master(talk)?;
    // Which data builder runs is the some-character table's to say.
    let rows = match extras.as_deref().map(|extras| &extras.some_character_talks) {
        Some(Ok(rows)) => rows,
        Some(Err(reason)) => return Some(format!("host gap: {reason}")),
        None => return Some("host gap: the some-character talk table is not loaded".into()),
    };
    {
        if let Some(row) = crate::npc_talk_lottery::read_talk_some_character_row(rows, talk, unit) {
            return Some(format!(
                "not ported: some-character row {} names talk {talk} with main unit {unit} (CreateSomeCharacterFixtureTimelineAITalkData)",
                row.id
            ));
        }
    }
    let members = tables
        .unit_group_slots(master.unit_group_id)
        .map(|slots| slots.iter().filter(|unit| unit.unwrap_or(0) != 0).count())
        .unwrap_or(0);
    (members >= 2).then(|| {
        format!("not ported: talk {talk} has {members} characters, whose others take the sub-character objective (8)")
    })
}

/// `ForceExecuteReadTalkFixtureTalk(talk)` on `actor`.
fn force_read_talk(
    world: &mut World,
    actor: Entity,
    unit: u32,
    talk: i32,
    frame: u32,
    line: &mut serde_json::Map<String, serde_json::Value>,
    draw_log: &mut Vec<serde_json::Value>,
) {
    if let Some(reason) = read_talk_unported(world, unit, talk) {
        line.insert("forced".into(), reason.into());
        return;
    }
    // TryCancelCurrentObjective.
    let (cancelled, owed) = world.resource_scope(
        |world, mut groups: Mut<crate::npc_fixture_talk::FixtureTalkGroups>| {
            crate::npc_fixture_talk::try_cancel_current(world, &mut groups, actor, frame)
        },
    );
    line.insert("cancel".into(), cancelled.into());
    if !cancelled {
        return;
    }
    if owed > 0 {
        line.insert("cancel_force_updates_not_drawn".into(), owed.into());
    }
    // ForceUpdateReadTalkFixtureTalk: Reset, then the data.
    let built = build_read_talk(world, actor, unit, talk);
    let (plan, draws) = match built {
        Ok(built) => built,
        Err(reason) => {
            line.insert("data_gap".into(), reason.into());
            if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
                slot.reset_ai_talk_data();
            }
            return;
        }
    };
    draw_log.extend(draws);
    world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .clear_forced(actor);
    let (data, tweet_id, fixture_uid) = match plan {
        Ok(ReadTalk { data, tweet_id, fixture_uid }) => (data, tweet_id, fixture_uid),
        Err(reason) => {
            // A source exception inside the forced update: the data stays
            // reset.
            line.insert("source_exception".into(), reason.into());
            if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
                slot.reset_ai_talk_data();
            }
            return;
        }
    };
    line.insert("forced".into(), "data".into());
    if let Some(fixture) = &data {
        line.insert("fixture".into(), fixture.fixture.clone().into());
        line.insert("timeline".into(), fixture.timeline.into());
        line.insert("target".into(), serde_json::json!(fixture.target_position));
        line.insert("target_found".into(), fixture.target_found.into());
    } else {
        line.insert("data".into(), "general (fixture without action points)".into());
    }
    let ai_data = data.as_ref().map(|fixture| fixture_ai_data(world, unit, fixture, fixture_uid.as_deref()));
    if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
        slot.reset_ai_talk_data();
        if let Some(Some(ai_data)) = ai_data {
            slot.set_current(ai_data);
        }
        slot.interrupt = Some(TALK_INTERRUPT);
    }
    if let Some(mut actions) = world.get_mut::<NpcActions>(actor) {
        actions.tweet_id = tweet_id;
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        mind.force_updates = 0;
        // ImmediatelyExecuteNextObjective.
        mind.skip_next_rest = true;
    }
    let Some(fixture) = data else {
        return;
    };
    // ForceSetFixtureActionPointNearPosition: the member (this character
    // alone) at its locate row's StartLoc, sampled within 0.25; a row with
    // unit 0 or no surface point there places nothing.
    let row = fixture
        .locate
        .as_deref()
        .and_then(|locate| locate.iter().find(|row| row.unit == unit as i32).copied());
    let placed = match row {
        None => serde_json::Value::from("no locate row for the character"),
        Some(row) => {
            let start = start_loc(world, &fixture.fixture, row.index);
            let hit = start.and_then(|start| {
                world
                    .get_resource::<ObjectiveFace>()
                    .and_then(|face| face.sample(start, law::NEAR_ACTION_POINT_RADIUS))
            });
            match hit {
                Some(hit) => {
                    crate::npc::force_set_position(world, actor, hit);
                    serde_json::json!(hit)
                }
                None => "no surface point within 0.25 of the StartLoc".into(),
            }
        }
    };
    line.insert("placed".into(), placed);
    world
        .get_resource_or_insert_with(crate::npc_objective::ForcedFixtureTalks::default)
        .0
        .insert(actor, fixture);
}

/// The forced update's outcome: the fixture data (`None` for the general
/// data a fixture without action points gives), the tweet id and the
/// fixture placement.
struct ReadTalk {
    data: Option<crate::npc_talk_lottery::FixtureTalkData>,
    tweet_id: i32,
    fixture_uid: Option<String>,
}

/// The fixture's StartLoc world position at `index`.
fn start_loc(world: &mut World, uid: &str, index: usize) -> Option<[f32; 3]> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Res<crate::fixture::FixturePlacements>,
    )>::new(world);
    let (factory, placements) = params.get_mut(world);
    let geometry = factory.talk_fixture_geometry(uid, &placements).ok()?;
    geometry.locators.get(index).map(|(_, pair)| pair.start.position)
}

/// The AI talk data CreateCharacterTalkData leaves for a forced fixture talk.
fn fixture_ai_data(
    world: &mut World,
    unit: u32,
    data: &crate::npc_talk_lottery::FixtureTalkData,
    _uid: Option<&str>,
) -> Option<crate::npc_objective::AiTalkData> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Res<crate::fixture::FixturePlacements>,
        crate::player_talk::TalkCatalog<'_>,
    )>::new(world);
    let (factory, placements, catalog) = params.get_mut(world);
    let resolved = catalog.resolve(data.talk_id)?;
    let geometry = factory.talk_fixture_geometry(&data.fixture, &placements).ok()?;
    let is_general = factory
        .tables()
        .and_then(|tables| crate::npc_talk_lottery::is_general_talk(tables, data.talk_id).ok());
    Some(crate::npc_objective::AiTalkData {
        kind: data.kind,
        content: Some(crate::npc_objective::TalkContent {
            is_general,
            ..resolved.content()
        }),
        target_fixture: Some(geometry.entity),
        target_position: data.target_position,
        main_character: unit,
        characters: data.members.clone(),
        pre_action: Some(resolved.pre_action()),
        locate: data.locate.clone(),
        pending_factory: None,
    })
}

/// `ForceUpdateReadTalkFixtureTalk`'s data for `unit` on `actor`, with its
/// draws: `Err` names host data that could not be read; the inner `Err` is
/// a source exception.
#[allow(clippy::type_complexity)]
fn build_read_talk(
    world: &mut World,
    actor: Entity,
    unit: u32,
    talk: i32,
) -> Result<(Result<ReadTalk, String>, Vec<serde_json::Value>), String> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Res<crate::fixture::FixturePlacements>,
        Option<Res<ObjectiveFace>>,
        crate::player_talk::TalkCatalog<'_>,
        Option<Res<crate::client_config::ClientConfigs>>,
        Option<Res<crate::server_panel::TalkDataStore>>,
        Option<Res<crate::site::SiteSelection>>,
        Option<Res<crate::npc_talk_lottery::TalkExtraTables>>,
        Query<
            (
                Entity,
                &CharacterUnitId,
                &TalkSlot,
                &ObjectiveMind,
                &NpcActions,
                &WalkState,
            ),
            Without<Away>,
        >,
        ResMut<crate::npc_objective::SequencePickSeeder>,
    )>::new(world);
    let (factory, placements, face, catalog, configs, talk_list, selection, extras, npcs, mut seeder) =
        params.get_mut(world);
    let face = face.ok_or("the objective face is not built")?;
    let config = configs.ok_or("the client configs are not loaded")?;
    let talk_list = talk_list.ok_or("the talk list is not set")?;
    let tables = factory
        .tables()
        .ok_or("the talk and fixture master tables are still loading")?;
    let (_, _, _, mind, _, walk) = npcs
        .get(actor)
        .map_err(|_| format!("unit {unit} is not on the loaded site"))?;
    let views: Vec<crate::npc_talk_lottery::NpcView> = npcs
        .iter()
        .map(
            |(_, unit, slot, mind, actions, _)| crate::npc_talk_lottery::NpcView {
                unit: unit.0,
                site_type: catalog.site_type_value(&actions.site_type),
                previous_talk_id: slot.previous_id().unwrap_or(0),
                talk_type: slot.kind(),
                objective: mind.current,
                state: actions.current as u8,
                since_initialized: actions.since_initialized,
            },
        )
        .collect();
    let seeker = views
        .iter()
        .find(|view| view.unit == unit)
        .expect("the character is in the list it was read from");
    let fixtures = factory.talk_lottery_fixtures(
        &placements,
        selection
            .as_deref()
            .and_then(|site| catalog.site_type_value(site.site_type())),
    );
    let other_targets: Vec<(Entity, Option<Entity>)> = npcs
        .iter()
        .map(|(entity, _, slot, ..)| {
            (
                entity,
                slot.current.as_ref().and_then(|data| data.target_fixture),
            )
        })
        .collect();
    let position = walk.0.position;
    let unmovable: &[String] = &mind.unmovable_fixtures;
    let admissible = |talk: i32, fixture: &crate::npc_talk_lottery::PlacedFixture| {
        factory.talk_fixture_admissible(
            actor,
            position,
            talk,
            &fixture.uid,
            &placements,
            &other_targets,
            unmovable,
            &face,
        )
    };
    let geometry = |uid: &str| factory.talk_fixture_geometry(uid, &placements);
    let host = crate::npc_objective::TalkFixtureHost::new(
        npcs.iter()
            .map(|(_, unit, _, _, _, walk)| (unit.0, walk.0.position))
            .collect(),
        &geometry,
        &face,
        config.float(crate::client_config::KEY_CHARACTER_FIXTURE_MOVE_OFFSET),
    );
    let site_type_of_site = |site: i32| catalog.site_type_of_site(site);
    let scene = crate::npc_talk_lottery::LotteryScene {
        tables,
        talk_list: talk_list.talk_list(),
        phenomena_id: catalog.phenomena_id(),
        site_type_of_site: &site_type_of_site,
        npcs: &views,
        fixtures: &fixtures,
        // The forced update draws no weighted lottery.
        weights: moly_law::talk::select::LotteryWeights {
            talk1: 0.0,
            talk2: 0.0,
            talk3: 0.0,
            talk4: 0.0,
        },
        fixture_gates: Some(crate::npc_talk_lottery::FixtureGateInputs {
            gate_action_elapsed_seconds: config
                .int(crate::client_config::KEY_CHARACTER_GATE_ACTION_ELAPSED_TIME),
            admissible: &admissible,
            extras: extras.as_deref(),
        }),
        fixture_host: Some(&host),
        together: None,
    };
    // Both picks are RandomPick over a sequence: a fresh System.Random each.
    let mut engine_int = |_: usize| -> usize {
        unreachable!("the forced read-talk update draws no engine integer")
    };
    let mut engine_float = |_: f32| -> f32 {
        unreachable!("the forced read-talk update draws no engine float")
    };
    let mut sequence_pick = |count: usize| seeder.pick(count);
    let mut draws = crate::npc_talk_lottery::Draws {
        engine_int: &mut engine_int,
        engine_float: &mut engine_float,
        sequence_pick: &mut sequence_pick,
        record: Vec::new(),
    };
    let result = crate::npc_talk_lottery::force_update_read_talk_fixture_talk(
        &scene, seeker, talk, &mut draws,
    );
    let record = std::mem::take(&mut draws.record);
    let tweet_id = tables.pre_action_of(talk).map_or(0, |pre| pre.tweet_id);
    let outcome = match result {
        Ok(crate::npc_talk_lottery::ReadTalkPlan::Fixture(data)) => {
            let uid = data.fixture.clone();
            Ok(ReadTalk {
                data: Some(data),
                tweet_id,
                fixture_uid: Some(uid),
            })
        }
        Ok(crate::npc_talk_lottery::ReadTalkPlan::General { .. }) => Ok(ReadTalk {
            data: None,
            tweet_id,
            fixture_uid: None,
        }),
        Err(crate::npc_talk_lottery::Halt::Fault(reason)) => Err(reason),
        Err(crate::npc_talk_lottery::Halt::Gap(reason)) => return Err(reason),
    };
    Ok((outcome, record))
}

/// `AvatarDataStore.FindNPC(unit)`.
fn find_npc(world: &mut World, unit: u32) -> Option<Entity> {
    let mut query = world
        .query_filtered::<(Entity, &CharacterUnitId), Without<crate::player::PlayerControlled>>();
    query
        .iter(world)
        .find(|(_, id)| id.0 == unit)
        .map(|(entity, _)| entity)
}

/// The character's unread fixture-action talks and its no-talk actions.
#[allow(clippy::type_complexity)]
fn candidates(world: &mut World, actor: Entity, unit: u32) -> Result<Candidates, String> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Res<crate::fixture::FixturePlacements>,
        Option<Res<ObjectiveFace>>,
        crate::player_talk::TalkCatalog<'_>,
        Option<Res<crate::client_config::ClientConfigs>>,
        Option<Res<crate::server_panel::TalkDataStore>>,
        Option<Res<crate::site::GroundEpoch>>,
        Option<Res<crate::site::SiteSelection>>,
        Query<
            (
                Entity,
                &CharacterUnitId,
                &TalkSlot,
                &ObjectiveMind,
                &NpcActions,
                &WalkState,
            ),
            Without<Away>,
        >,
        Query<(&CharacterUnitId, &NpcActions), Without<crate::player::PlayerControlled>>,
        Option<Res<crate::npc_talk_lottery::TalkExtraTables>>,
    )>::new(world);
    let (factory, placements, face, catalog, configs, talk_list, epoch, selection, npcs, store, extras) =
        params.get_mut(world);
    let face = face.ok_or("the objective face is not built")?;
    let config = configs.ok_or("the client configs are not loaded")?;
    let talk_list = talk_list.ok_or("the talk list is not set")?;
    let epoch = epoch.ok_or("no site generation is set")?.0;
    let tables = factory
        .tables()
        .ok_or("the talk and fixture master tables are still loading")?;
    let (_, _, _, mind, actions, walk) = npcs
        .get(actor)
        .map_err(|_| format!("unit {unit} is not on the loaded site"))?;
    let views: Vec<crate::npc_talk_lottery::NpcView> = npcs
        .iter()
        .map(
            |(_, unit, slot, mind, actions, _)| crate::npc_talk_lottery::NpcView {
                unit: unit.0,
                site_type: catalog.site_type_value(&actions.site_type),
                previous_talk_id: slot.previous_id().unwrap_or(0),
                talk_type: slot.kind(),
                objective: mind.current,
                state: actions.current as u8,
                since_initialized: actions.since_initialized,
            },
        )
        .collect();
    // IsMatchedSubCharacterSite reads every NPC of the store, on any site.
    let store: Vec<(u32, Option<i32>)> = store
        .iter()
        .map(|(unit, actions)| (unit.0, catalog.site_type_value(&actions.site_type)))
        .collect();
    let seeker = views
        .iter()
        .find(|view| view.unit == unit)
        .expect("the character is in the list it was read from");
    let fixtures = factory.talk_lottery_fixtures(
        &placements,
        selection
            .as_deref()
            .and_then(|site| catalog.site_type_value(site.site_type())),
    );
    let other_targets: Vec<(Entity, Option<Entity>)> = npcs
        .iter()
        .map(|(entity, _, slot, ..)| {
            (
                entity,
                slot.current.as_ref().and_then(|data| data.target_fixture),
            )
        })
        .collect();
    let position = walk.0.position;
    let unmovable: &[String] = &mind.unmovable_fixtures;
    let admissible = |talk: i32, fixture: &crate::npc_talk_lottery::PlacedFixture| {
        factory.talk_fixture_admissible(
            actor,
            position,
            talk,
            &fixture.uid,
            &placements,
            &other_targets,
            unmovable,
            &face,
        )
    };
    let site_type_of_site = |site: i32| catalog.site_type_of_site(site);
    let scene = crate::npc_talk_lottery::LotteryScene {
        tables,
        talk_list: talk_list.talk_list(),
        phenomena_id: catalog.phenomena_id(),
        site_type_of_site: &site_type_of_site,
        npcs: &views,
        fixtures: &fixtures,
        // The fixture-action filters draw nothing; the weights are unread.
        weights: moly_law::talk::select::LotteryWeights {
            talk1: 0.0,
            talk2: 0.0,
            talk3: 0.0,
            talk4: 0.0,
        },
        fixture_gates: Some(crate::npc_talk_lottery::FixtureGateInputs {
            gate_action_elapsed_seconds: config
                .int(crate::client_config::KEY_CHARACTER_GATE_ACTION_ELAPSED_TIME),
            admissible: &admissible,
            extras: extras.as_deref(),
        }),
        fixture_host: None,
        together: None,
    };
    let mut gaps = Vec::new();
    let talks = match crate::npc_talk_lottery::fixture_action_talk_ids(&scene, seeker, &store) {
        Ok(talks) => talks,
        Err(crate::npc_talk_lottery::Halt::Gap(reason)) => {
            gaps.push(format!("talks: {reason}"));
            Vec::new()
        }
        Err(crate::npc_talk_lottery::Halt::Fault(reason)) => {
            return Err(format!(
                "the fixture-action talk filters raise in the source ({reason})"
            ));
        }
    };
    let actions = match factory.no_talk_action_rows(
        actor,
        unit,
        &actions.site_type,
        epoch,
        position,
        &placements,
        &other_targets,
        unmovable,
        &face,
    ) {
        Ok(rows) => rows,
        Err(crate::npc_fixture_activity::FactoryIssue::Pending(reason))
        | Err(crate::npc_fixture_activity::FactoryIssue::Gap(reason)) => {
            gaps.push(format!("no-talk actions: {reason}"));
            Vec::new()
        }
    };
    Ok(Candidates {
        talks,
        actions,
        gaps,
    })
}

/// `ForceExecuteNoneTalkFixtureActionObjective(action)` on `actor`.
fn force_no_talk_action(
    world: &mut World,
    actor: Entity,
    unit: u32,
    row: i32,
    frame: u32,
    line: &mut serde_json::Map<String, serde_json::Value>,
    draw_log: &mut Vec<serde_json::Value>,
) {
    // TryCancelCurrentObjective.
    let (cancelled, owed) = world.resource_scope(
        |world, mut groups: Mut<crate::npc_fixture_talk::FixtureTalkGroups>| {
            crate::npc_fixture_talk::try_cancel_current(world, &mut groups, actor, frame)
        },
    );
    line.insert("cancel".into(), cancelled.into());
    if !cancelled {
        return;
    }
    if owed > 0 {
        line.insert("cancel_force_updates_not_drawn".into(), owed.into());
    }
    // ForceUpdateNoneTalkFixtureActionObjective: Reset, then SetAITalkData(
    // CreateNoneTalkData(character, action)), then SetCanInterruptTalkData(4).
    let selected = world.resource_scope(|world, mut random: Mut<ControllerRandom>| {
        select_row(world, actor, unit, row, &mut random.0)
    });
    let selection = match selected {
        Ok((selection, draws)) => {
            if let Some((value, len)) = draws.timeline_pick {
                draw_log.push(serde_json::json!({"use": "none_talk_timeline_pick", "value": value, "range": [0, len]}));
            }
            selection
        }
        Err(reason) => {
            line.insert("data_gap".into(), reason.into());
            None
        }
    };
    world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .clear_forced(actor);
    if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
        slot.reset_ai_talk_data();
        if let Some(selection) = &selection {
            slot.set_current(selection.ai_data());
        }
        slot.interrupt = Some(TALK_INTERRUPT);
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        // The cancelled objective's own ForceUpdateObjective ran inside the
        // cancel, before this reset: nothing stays owed.
        mind.force_updates = 0;
        // ImmediatelyExecuteNextObjective.
        mind.skip_next_rest = true;
    }
    let Some(selection) = selection else {
        line.insert("data".into(), "null".into());
        return;
    };
    line.insert("fixture".into(), selection.target.uid.clone().into());
    line.insert("timeline".into(), selection.timeline_id().into());
    // ForceSetFixtureActionPointNearPosition: the StartLoc, sampled within
    // 0.25; no surface point there places nothing.
    let hit = world
        .get_resource::<ObjectiveFace>()
        .and_then(|face| face.sample(selection.start_loc, law::NEAR_ACTION_POINT_RADIUS));
    match hit {
        Some(hit) => {
            crate::npc::force_set_position(world, actor, hit);
            line.insert("placed".into(), serde_json::json!(hit));
        }
        None => {
            line.insert(
                "placed".into(),
                "no surface point within 0.25 of the StartLoc".into(),
            );
        }
    }
    world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .force(actor, selection);
}

/// CreateNoneTalkData(character, action) for one NoTalk row.
#[allow(clippy::type_complexity)]
fn select_row(
    world: &mut World,
    actor: Entity,
    unit: u32,
    row: i32,
    random: &mut crate::npc_objective::MemberRng,
) -> Result<
    (
        Option<crate::npc_fixture_activity::Selection>,
        crate::npc_fixture_activity::NoneTalkDraws,
    ),
    String,
> {
    let mut params = SystemState::<(
        crate::npc_fixture_activity::Factory<'_, '_>,
        Res<crate::fixture::FixturePlacements>,
        Option<Res<ObjectiveFace>>,
        Option<Res<crate::site::GroundEpoch>>,
        Query<(Entity, &TalkSlot, &ObjectiveMind, &NpcActions, &WalkState), Without<Away>>,
    )>::new(world);
    let (factory, placements, face, epoch, npcs) = params.get_mut(world);
    let face = face.ok_or("the objective face is not built")?;
    let epoch = epoch.ok_or("no site generation is set")?.0;
    let (_, _, mind, actions, walk) = npcs
        .get(actor)
        .map_err(|_| format!("unit {unit} is not on the loaded site"))?;
    let other_targets: Vec<(Entity, Option<Entity>)> = npcs
        .iter()
        .map(|(entity, slot, ..)| {
            (
                entity,
                slot.current.as_ref().and_then(|data| data.target_fixture),
            )
        })
        .collect();
    let mut draws = crate::npc_fixture_activity::NoneTalkDraws::default();
    let selection = factory
        .select_no_talk_row(
            row,
            actor,
            unit,
            &actions.site_type,
            epoch,
            walk.0.position,
            &placements,
            &other_targets,
            &mind.unmovable_fixtures,
            &face,
            random,
            &mut draws,
        )
        .map_err(|issue| match issue {
            crate::npc_fixture_activity::FactoryIssue::Pending(reason)
            | crate::npc_fixture_activity::FactoryIssue::Gap(reason) => reason,
        })?;
    Ok((selection, draws))
}

fn finish(world: &mut World, caller: Caller, state: &mut AnswerState) {
    state.initial_done = true;
    state.empty_logged = false;
    if caller == Caller::DoorMove {
        world.remove_resource::<RandomFixtureActionPending>();
        info!(
            "[npc-door] ExecuteNPCRandomFixtureAction returned: RandomFixtureActionPending removed"
        );
    }
}

fn home_is_loaded(world: &World) -> bool {
    world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == crate::npc::residency::SITE_TYPES[0])
}
