//! Fixture talks with more than one member: the while-doing-wait talk (type
//! 6) and the multiple-character fixture talk (type 3).
//!
//! **The main's talk objective drafts the other members first**
//! (PassTalkDataToSubCharacters). A member whose action state is a
//! fixture-action state (11, 12, 17, 18, 19), or whose current objective can
//! be cancelled (it is not talking and has a live objective), is drafted: it
//! gets the data its ForceUpdate call builds, the interrupt marker of its sub
//! objective (9 for type 6, 6 for type 3) and the flag that skips its next
//! rest. A type-6 member's objective is always cancelled first (its
//! ForceUpdateCommunicationWhileDoingWaitObjective cancels it); a type-3
//! member in a fixture-action state keeps its current objective until it
//! ends by itself. The drafted member then decides its sub objective itself
//! (the decision ladder's interrupt row) and moves to its own data target.
//!
//! **The main's move** (MoveAsync): the fixture's action points are claimed
//! for the main and then for each other member in the data's order, from the
//! main's locate list and each member's own target fixture; the first
//! failure stops the claims. Then the gathering and the main's own move run
//! together and both must end. The gathering ends with success when every
//! other member has arrived, and with failure as soon as a member not yet
//! arrived has no data, another talk type or another master; a member
//! arrives within 0.1 of its data target and goes Idle. A failed gathering
//! makes every other member ForceUpdateObjective and fails the main's move.
//!
//! **Type 6** (WaitWhileCommunicationAsync, no script): after one Yield,
//! WaitAndRotateCharacter puts each member into state 20
//! (SomeCharacterCommunication) as it comes within 0.1 of its target (all
//! arrived members turn to their centroid); a member whose data is no longer
//! type 6 ends it. Then the pre-action tweet is shown for the main for up to
//! IntConfigs key 107 milliseconds of scaled time, ending early when the main
//! talks; then it is hidden, the main's talk is waited out, and every member
//! not talking ForceUpdateObjective. A member's sub objective (9) moves to
//! its target, goes Idle, waits for state 20 and then holds while every
//! member keeps type-6 data.
//!
//! **Type 3** (no script): after the gathering, OnArrive (every locate row's
//! member turns to its action point's StartLoc rotation, all at once, and
//! the main waits for the slowest), the refused tweet, then
//! TryPlayTimelineAsync: without a timeline it
//! returns false and the objective ends; with one the main changes to state
//! 16 and waits while its state is 4, 11, 12, 16 or 18, and state 16's entry
//! starts the timeline controller over every member (`controller`). A
//! member's sub objective (18) moves to its target, changes to state 12,
//! checks its data at the next poll, changes to state 18 and waits while its
//! state is 11, 12, 18 or 19, at most 60 s of scaled time. A cancel of the
//! main's objective while it waits (the controller's own ForceUpdateObjective
//! at the timeline's end, or another owner's) lands in the objective's catch:
//! every other member not talking ForceUpdateObjective.
//!
//! **ForceUpdateObjective counts.** presenter.ForceUpdateObjective is
//! TryCancelCurrentObjective (false while talking, without a current
//! objective, or on one that completed; true again on one cancelled before)
//! and, when it reported true, one ForceUpdateObjective after the ones the
//! cancelled objective's OnCancel makes: the no-talk objective's and the sub
//! objective 9's make one, the talk objective's only releases its claim, the
//! Rest's changes to Idle. The calls are made by the character's next
//! decision pass (see `npc_objective`).
//!
//! **Not carried, named:** the look and rotate tweens of type 6 and of the
//! sub objective (OnMoveComplete's look at the action points' centre,
//! RotateCenterAsync) are logged, not played; the idle animation's exit-time
//! wait before state 16 is not timed; the centroid tweet's anchor (the source
//! places it at the members' centroid) is the main; a drafted member
//! cancelled out of a no-talk objective does not draw that objective's own
//! ForceUpdateObjective cascade (its data is replaced by the reset that
//! follows); the claims are released with the talk instead of at each
//! member's next reset; the drafts land after the frame's decisions (the
//! members after the main in the avatar order are held from deciding until
//! then); for type 3, the frame order between the gathering's change to Idle
//! on an arriving member and that member's own changes to 12 and 18 is not
//! settled by the reading: this host runs the gathering first.

mod controller;

use std::collections::HashMap;

use bevy::prelude::*;
use moly_law::objective::{DelayPromise, InterruptMarker, ObjectiveType, TalkType};

use crate::fixture_activity_state::{
    FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
};
use crate::npc::{
    CharacterUnitId, MotionPhase, NpcAction, NpcActions, RestLifecycle, RouteOutcome, RouteStops,
    WalkState,
};
use crate::npc_objective::{AiTalkData, ObjectiveMind, TalkSlot};
use crate::npc_talk_lottery::LocateRow;

/// IntConfigs key 107 (CommunicationWhileDoingWaitTime): the while-doing-wait
/// tweet window, in milliseconds.
pub(crate) const KEY_COMMUNICATION_WHILE_DOING_WAIT_TIME: i32 = 107;

/// A member is at its data target within this distance (the gathering,
/// WaitAndRotateCharacter, GetArrivedCharacterCount).
const ARRIVED_DISTANCE: f32 = 0.1;

/// AvatarMoveStatus values the talk objective reads.
const MOVE_SUCCESS: i32 = 1;
const MOVE_STACKED: i32 = 2;
const MOVE_NONE_ROUTE: i32 = -2;

/// The source's float32 distance of these tests, sqrt(dz*dz + (dx*dx + dy*dy)).
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    (dz * dz + (dx * dx + dy * dy)).sqrt()
}

/// One other member as the main's PassTalkDataToSubCharacters met it.
#[derive(Clone, Debug)]
pub(crate) struct MemberDraft {
    pub(crate) entity: Entity,
    pub(crate) unit: u32,
    /// Its state is a fixture-action state, or its objective could be
    /// cancelled.
    pub(crate) drafted: bool,
    /// Its current objective is cancelled before the data is set.
    pub(crate) cancel: bool,
    /// The data its ForceUpdate call sets; `None` leaves it empty after the
    /// reset.
    pub(crate) data: Option<AiTalkData>,
}

/// A group talk the main's decision starts.
pub(crate) struct GroupStart {
    pub(crate) main: Entity,
    pub(crate) main_unit: u32,
    pub(crate) kind: TalkType,
    pub(crate) talk_id: i32,
    pub(crate) fixture: Entity,
    pub(crate) fixture_uid: String,
    /// The data's character list, by unit, in its order.
    pub(crate) members: Vec<u32>,
    pub(crate) locate: Vec<LocateRow>,
    /// The main data's picked fixture timeline (the multiple-character talk).
    pub(crate) timeline: Option<i32>,
    /// The fixture's StartLoc names in action-point array order.
    pub(crate) names: Vec<String>,
    pub(crate) drafts: Vec<MemberDraft>,
    /// The pre-action tweet (id, text).
    pub(crate) tweet: Option<(i32, String)>,
}

enum Stage {
    /// PassTalkDataToSubCharacters and the claims run on the next pass.
    Draft,
    /// MoveAsync: the gathering and the main's own move, both awaited. A
    /// Stacked move then owes a scaled 1.0 s delay (`stacked`, from the frame
    /// both ended) before the move fails.
    Moving {
        arrived: Vec<u32>,
        other: Option<i32>,
        own: Option<i32>,
        stacked: Option<(u32, DelayPromise)>,
    },
    /// WaitWhileCommunicationAsync's Yield.
    CommunicationYield { since: u32 },
    /// WaitAndRotateCharacter.
    Rotating { arrived: Vec<u32> },
    /// The tweet window.
    Tweeting { elapsed: f32 },
    /// WaitWhile(the main talks) after HideTweet.
    AfterTweet { since: u32 },
    /// Type 3: OnArrive's WhenAll over the locate rows' turns; the pre-action
    /// follows the slowest.
    ArriveTurning { turns: Vec<ArriveTurn> },
    /// Type 3: TryPlayTimelineAsync's WaitWhile(the main's state is 4, 11,
    /// 12, 16 or 18), polled from the frame after the change to state 16.
    TimelineWait { since: u32 },
}

/// One locate row's RotateCharacterToActionPointRotation: the row's member
/// tweens its rotation to the action point's StartLoc world rotation over
/// GetRotateTime, with the rotate motion of the yaw difference.
struct ArriveTurn {
    unit: u32,
    actor: Entity,
    from: Option<Quat>,
    to: Quat,
    duration: f32,
    elapsed: f32,
}

struct Group {
    start: GroupStart,
    stage: Stage,
    owner: Option<FixtureActivityOwner>,
    /// The main's move failed at its departure (gate closed or no route).
    departure_failed: bool,
}

/// A member's sub objective after its own decision.
#[derive(Clone, Copy, Debug)]
enum SubStage {
    Moving,
    /// Objective 9 after its move: Idle, then waiting for state 20 (phase A),
    /// then holding while every member keeps type-6 data (phase B).
    WaitCommunication {
        since: u32,
        communicating: bool,
    },
    /// Objective 18 after its move: state 12, then WaitUntil(its data check),
    /// first polled on the next frame.
    FixtureCheck {
        since: u32,
    },
    /// Objective 18 in state 18: WaitWhile(state 11, 12, 18 or 19), at most
    /// 60 s of scaled time.
    FixtureWait {
        since: u32,
        elapsed: f32,
    },
}

/// NPCAvatarSubCharacterFixtureActionObjective's TIMEOUT_SECONDS.
const SUB_FIXTURE_ACTION_TIMEOUT_SECONDS: f32 = 60.0;

struct Sub {
    unit: u32,
    objective: ObjectiveType,
    stage: SubStage,
    /// The members of its own data (their units), read when it starts.
    members: Vec<u32>,
}

/// The running group talks and the members' sub objectives.
#[derive(Resource, Default)]
pub(crate) struct FixtureTalkGroups {
    groups: Vec<Group>,
    subs: HashMap<Entity, Sub>,
    controllers: Vec<controller::Controller>,
}

impl FixtureTalkGroups {
    /// Whether a group talk or a sub objective drives `actor`, or a group
    /// talk's drafts will land on it in this frame's group pass (the main's
    /// PassTalkDataToSubCharacters runs at the main's decision, so a member
    /// after the main in the avatar order does not decide before it).
    pub(crate) fn owns(&self, actor: Entity) -> bool {
        self.subs.contains_key(&actor)
            || self.groups.iter().any(|group| {
                group.start.main == actor
                    || (matches!(group.stage, Stage::Draft)
                        && group.start.drafts.iter().any(|draft| draft.entity == actor))
            })
    }

    /// IsPlayingSomeCharacterTalkFixture: the main of the registered NPC
    /// timeline with more than one character that `unit` acts in.
    pub(crate) fn some_character_timeline(&self, unit: u32) -> Option<Entity> {
        self.controllers
            .iter()
            .find(|controller| controller.registered_with(unit))
            .map(|controller| controller.main)
    }

    /// The main's decision built a group talk; its drafts and claims run on
    /// the next pass.
    pub(crate) fn start(&mut self, start: GroupStart) {
        info!(
            "[npc-group] unit={} talk {} type {} on {}: members {:?}, drafts {:?}",
            start.main_unit,
            start.talk_id,
            start.kind as u8,
            start.fixture_uid,
            start.members,
            start
                .drafts
                .iter()
                .map(|draft| (
                    draft.unit,
                    draft.drafted,
                    draft.cancel,
                    draft.data.is_some()
                ))
                .collect::<Vec<_>>()
        );
        self.groups.push(Group {
            start,
            stage: Stage::Draft,
            owner: None,
            departure_failed: false,
        });
    }

    /// The main's move ended at its departure (NoneRoute): the gathering still
    /// runs to its end first.
    pub(crate) fn main_departure_failed(&mut self, main: Entity) {
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.start.main == main)
        {
            group.departure_failed = true;
        }
    }

    /// A drafted member decided its sub objective and departed.
    pub(crate) fn begin_sub(
        &mut self,
        actor: Entity,
        unit: u32,
        objective: ObjectiveType,
        members: Vec<u32>,
    ) {
        info!(
            "[npc-group] unit={unit} sub objective {} departs",
            objective as u8
        );
        self.subs.insert(
            actor,
            Sub {
                unit,
                objective,
                stage: SubStage::Moving,
                members,
            },
        );
    }
}

/// presenter.TryCancelCurrentObjective: refused while the character talks
/// (IsEnabledCancelCondition) and without a current objective; an objective
/// cancelled before reports true again; a completed (disposed) one reports
/// false; a running one is cancelled here: its OnCancel, then its token (the
/// move stops, a fixture session is disposed) and its dispose. Returns
/// whether it reported true and how many ForceUpdateObjective calls the
/// OnCancel makes (the no-talk objective's and the sub objective 9's make
/// one). The loop then yields before its TryRest.
pub(crate) fn try_cancel_current(
    world: &mut World,
    groups: &mut FixtureTalkGroups,
    actor: Entity,
    frame: u32,
) -> (bool, u8) {
    if state_of(world, actor) == Some(NpcAction::Talk) {
        return (false, 0);
    }
    let Some(mind) = world.get::<ObjectiveMind>(actor) else {
        return (false, 0);
    };
    let Some(current) = mind.current else {
        return (false, 0);
    };
    if mind.cancelled {
        return (true, 0);
    }
    if !mind.cancel_reports() {
        return (false, 0);
    }
    let executing = mind.executing;
    let owed = match current {
        // The Rest objective's cancel changes to Idle.
        ObjectiveType::Rest => {
            change_state(world, actor, NpcAction::Idle);
            0
        }
        // The no-talk objective's OnCancel is Model.ForceUpdateObjective.
        ObjectiveType::NoneTalk => 1,
        ObjectiveType::SomeCharacterFixtureActionCommunicationWhileDoingWaitSub => {
            groups.subs.remove(&actor);
            change_state(world, actor, NpcAction::Idle);
            1
        }
        ObjectiveType::SubCharacterFixtureAction => {
            groups.subs.remove(&actor);
            change_state(world, actor, NpcAction::Idle);
            0
        }
        // The talk objective's OnCancel releases the main's claim; the group
        // sees the cancel at its next poll.
        ObjectiveType::Talk => {
            if let Some(owner) = groups
                .groups
                .iter()
                .find(|group| group.start.main == actor)
                .and_then(|group| group.owner)
            {
                world
                    .resource_mut::<FixtureActivityReservations>()
                    .release_owner(owner);
            }
            0
        }
        _ => 0,
    };
    if world
        .get_resource::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .is_some_and(|runtime| crate::npc_fixture_activity::owns_actor(runtime, actor))
    {
        crate::npc_fixture_activity::cancel_for_greeting(world, actor);
    }
    // The objective's token stops a running move (MoveAsync); a character
    // standing at its target keeps its state.
    let moving = matches!(
        state_of(world, actor),
        Some(NpcAction::AutoMove | NpcAction::Walk | NpcAction::Rotate)
    );
    if executing && moving {
        crate::npc::stop_for_external_activity(world, actor);
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        mind.cancelled = true;
        mind.executing = false;
        mind.body = None;
        mind.rest = None;
        mind.yield_since = Some(frame);
    }
    (true, owed)
}

/// presenter.ForceUpdateObjective: TryCancelCurrentObjective, and when it
/// reported true, Model.ForceUpdateObjective (one reset and row-1 cascade)
/// after the calls the cancelled objective's OnCancel made. The calls are
/// made by the character's next decision pass.
fn presenter_force_update(
    world: &mut World,
    groups: &mut FixtureTalkGroups,
    actor: Entity,
    unit: u32,
    frame: u32,
) {
    let (cancelled, owed) = try_cancel_current(world, groups, actor, frame);
    if !cancelled {
        info!(
            "[npc-group] unit={unit} frame={frame} ForceUpdateObjective: the cancel reported false (talking, or no running objective); nothing is updated"
        );
        return;
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        crate::npc_objective::owe_force_updates(&mut mind, frame, owed + 1);
    }
    info!(
        "[npc-group] unit={unit} frame={frame} ForceUpdateObjective: cancelled, {} calls owed",
        owed + 1
    );
}

fn change_state(world: &mut World, actor: Entity, state: NpcAction) {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = query.get_mut(world, actor) {
        actions.change(state, &mut rest);
    }
}

/// The avatar list: unit -> NPC entity.
type Roster = HashMap<u32, Entity>;

fn roster(world: &mut World) -> Roster {
    let mut query = world.query::<(Entity, &CharacterUnitId, &ObjectiveMind)>();
    query
        .iter(world)
        .map(|(entity, id, _)| (id.0, entity))
        .collect()
}

fn state_of(world: &World, actor: Entity) -> Option<NpcAction> {
    world
        .get::<NpcActions>(actor)
        .map(|actions| actions.current)
}

/// The slot TryParseActionPointNameToSlotId gives a StartLoc name: the
/// "loc_start"-stripped name's second '_' field (0 when it does not parse),
/// 0 without one.
fn slot_of_name(name: &str) -> i32 {
    let stripped = name.replace("loc_start", "");
    let parts: Vec<&str> = stripped.split('_').collect();
    if parts.len() > 1 {
        parts[1].parse::<i32>().unwrap_or(0)
    } else {
        0
    }
}

/// The group pass: after the decisions of the frame.
pub(crate) fn advance(world: &mut World) {
    if world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return;
    }
    let Some(mut groups) = world.remove_resource::<FixtureTalkGroups>() else {
        return;
    };
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let dt = world
        .get_resource::<crate::npc_clock::NpcClock>()
        .map_or(0.0, crate::npc_clock::NpcClock::delta);
    let window = world
        .get_resource::<crate::client_config::ClientConfigs>()
        .map(|config| config.int(KEY_COMMUNICATION_WHILE_DOING_WAIT_TIME));
    let roster = roster(world);
    let running = std::mem::take(&mut groups.groups);
    for mut group in running {
        match step_group(world, &roster, &mut groups, &mut group, frame, dt, window) {
            Ok(true) => {}
            Ok(false) => groups.groups.push(group),
            Err(reason) => {
                warn!(
                    "[npc-group] unit={} talk {} ends: {reason}",
                    group.start.main_unit, group.start.talk_id
                );
                end_group(
                    world,
                    &roster,
                    &mut groups,
                    &group,
                    frame,
                    GroupEnd::MainFails,
                );
            }
        }
    }
    let running = std::mem::take(&mut groups.controllers);
    let mut provider =
        world.remove_resource::<crate::fixture_activity_provider::FixtureActivityProvider>();
    for mut controller in running {
        if !controller::step(
            world,
            &roster,
            &mut groups,
            provider.as_mut(),
            &mut controller,
            frame,
            dt,
        ) {
            groups.controllers.push(controller);
        }
    }
    if let Some(provider) = provider {
        world.insert_resource(provider);
    }
    let subs: Vec<Entity> = groups.subs.keys().copied().collect();
    for actor in subs {
        if !step_sub(world, &roster, &mut groups, actor, frame, dt) {
            continue;
        }
        groups.subs.remove(&actor);
    }
    world.insert_resource(groups);
    crate::player_talk::report_closed_fixture_talks(world);
}

/// How a group talk ends.
enum GroupEnd {
    /// The main's move failed: its objective ends; nothing is owed.
    MainFails,
    /// The objective ran to its end without a cancel (type 3 without a
    /// timeline, or the state left the timeline wait's set); nothing is owed.
    Completes,
    /// The communication ended: every member not talking ForceUpdateObjective.
    Communicated,
}

fn end_group(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    group: &Group,
    frame: u32,
    end: GroupEnd,
) {
    if let Some(owner) = group.owner {
        if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
            reservations.release_owner(owner);
        }
    }
    let main = group.start.main;
    match end {
        GroupEnd::MainFails | GroupEnd::Completes => {
            if let Some(mut mind) = world.get_mut::<ObjectiveMind>(main) {
                mind.executing = false;
                mind.body = None;
                mind.yield_since = Some(frame);
            }
            info!(
                "[npc-group] unit={} frame={frame} talk {}: {}; objective ends",
                group.start.main_unit,
                group.start.talk_id,
                if matches!(end, GroupEnd::MainFails) {
                    "move failed"
                } else {
                    "completed"
                }
            );
        }
        GroupEnd::Communicated => {
            // For the main this cancels the talk objective itself (the claim
            // was released above); a member in the sub objective 9 gets its
            // OnCancel's call and then one more.
            for unit in &group.start.members {
                let Some(&actor) = roster.get(unit) else {
                    continue;
                };
                presenter_force_update(world, groups, actor, *unit, frame);
            }
            info!(
                "[npc-group] unit={} frame={frame} talk {}: communication ended; ForceUpdateObjective on every member not talking",
                group.start.main_unit, group.start.talk_id
            );
        }
    }
}

/// One pass of a group talk; `Ok(true)` when it ended.
#[allow(clippy::too_many_arguments)]
fn step_group(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    group: &mut Group,
    frame: u32,
    dt: f32,
    window: Option<i32>,
) -> Result<bool, String> {
    let main = group.start.main;
    if world.get::<CharacterUnitId>(main).is_none() {
        return Err("the main character is gone".into());
    }
    // Another owner (the greeting, a layout edit) cancelled the talk
    // objective: its OnCancel releases the claim; nothing is owed.
    let running = world
        .get::<ObjectiveMind>(main)
        .is_some_and(|mind| mind.executing && mind.current == Some(ObjectiveType::Talk));
    if !running && matches!(group.stage, Stage::TimelineWait { .. }) {
        // The OperationCanceledException of TryPlayPreActionAsync's await:
        // the catch makes every other member not talking ForceUpdateObjective.
        main_catch(world, roster, groups, group, frame);
        return Ok(true);
    }
    if !running {
        info!(
            "[npc-group] unit={} frame={frame} talk {}: the talk objective was cancelled by another owner",
            group.start.main_unit, group.start.talk_id
        );
        if let Some(owner) = group.owner {
            world
                .resource_mut::<FixtureActivityReservations>()
                .release_owner(owner);
        }
        return Ok(true);
    }
    match &mut group.stage {
        Stage::Draft => {
            draft_members(world, groups, group, frame);
            claim_action_points(world, roster, group);
            group.stage = Stage::Moving {
                arrived: Vec::new(),
                other: None,
                own: group.departure_failed.then_some(MOVE_NONE_ROUTE),
                stacked: None,
            };
            // The gathering's first pass runs in the frame MoveAsync starts.
            step_group(world, roster, groups, group, frame, dt, window)
        }
        Stage::Moving {
            arrived,
            other,
            own,
            stacked,
        } => {
            if let Some((since, delay)) = stacked {
                // After WhenAll: the Stacked move's delay, then the move fails.
                if *since == frame || !delay.advance(dt) {
                    return Ok(false);
                }
                info!(
                    "[npc-group] unit={} frame={frame} talk {}: stacked move: 1.0 s delay done, the move fails",
                    group.start.main_unit, group.start.talk_id
                );
                end_group(world, roster, groups, group, frame, GroupEnd::MainFails);
                return Ok(true);
            }
            if other.is_none() {
                *other = gathering(world, roster, &group.start, arrived, frame);
            }
            if own.is_none() {
                let outcome = world.get_mut::<RouteStops>(main).and_then(|mut route| {
                    route
                        .outcome
                        .take()
                        .map(|outcome| (outcome, route.take_stalled()))
                });
                *own = match outcome {
                    None => None,
                    Some((RouteOutcome::Arrived, _)) => Some(MOVE_SUCCESS),
                    Some((RouteOutcome::Stopped, true)) => Some(MOVE_STACKED),
                    Some((RouteOutcome::Stopped, false)) => Some(MOVE_NONE_ROUTE),
                };
            }
            let (Some(gathered), Some(status)) = (*other, *own) else {
                return Ok(false);
            };
            let unit = group.start.main_unit;
            if gathered == -1 {
                crate::npc::stop_for_external_activity(world, main);
                for member in group.start.members.iter().filter(|member| **member != unit) {
                    if let Some(&actor) = roster.get(member) {
                        presenter_force_update(world, groups, actor, *member, frame);
                    }
                }
                info!(
                    "[npc-group] unit={unit} frame={frame} talk {}: gathering failed; ForceUpdateObjective on the other members",
                    group.start.talk_id
                );
                end_group(world, roster, groups, group, frame, GroupEnd::MainFails);
                return Ok(true);
            }
            if status == MOVE_STACKED {
                // main.Stop(), then a scaled 1.0 s Delay.
                crate::npc::stop_for_external_activity(world, main);
                *stacked = Some((
                    frame,
                    DelayPromise::from_milliseconds(1000).expect("one second is a valid delay"),
                ));
                return Ok(false);
            }
            if status != MOVE_SUCCESS {
                info!(
                    "[npc-group] unit={unit} frame={frame} talk {}: own move status {status}",
                    group.start.talk_id
                );
                end_group(world, roster, groups, group, frame, GroupEnd::MainFails);
                return Ok(true);
            }
            match group.start.kind {
                TalkType::CommunicationWhileDoingWait => {
                    // OnArrive does nothing for a type-6 member; the
                    // pre-action waits with the communication.
                    info!(
                        "[npc-group] unit={unit} frame={frame} talk {}: gathered; WaitWhileCommunicationAsync (type Inprogress)",
                        group.start.talk_id
                    );
                    group.stage = Stage::CommunicationYield { since: frame };
                    Ok(false)
                }
                TalkType::MultipleCharacterFixture => {
                    let turns = on_arrive_some_character(world, roster, group, frame);
                    if turns.iter().any(|turn| turn.duration > 0.0) {
                        group.stage = Stage::ArriveTurning { turns };
                        return Ok(false);
                    }
                    for turn in &turns {
                        finish_turn(world, turn);
                    }
                    after_arrive_some_character(world, roster, groups, group, frame)
                }
                _ => Err(format!(
                    "group talk type {} has no execution",
                    group.start.kind as u8
                )),
            }
        }
        Stage::ArriveTurning { turns } => {
            let mut done = true;
            for turn in turns.iter_mut() {
                if turn.elapsed >= turn.duration {
                    continue;
                }
                // A member still on its own fit walk keeps it: its own last
                // turn targets the same StartLoc rotation.
                let moving = matches!(
                    world.get::<MotionPhase>(turn.actor),
                    Some(
                        MotionPhase::Walking
                            | MotionPhase::FitWalking { .. }
                            | MotionPhase::FitTurning { .. }
                    )
                );
                let Some(current) = world.get::<Transform>(turn.actor).copied() else {
                    turn.elapsed = turn.duration;
                    continue;
                };
                if moving {
                    turn.elapsed += dt;
                    done &= turn.elapsed >= turn.duration;
                    continue;
                }
                let from = *turn.from.get_or_insert(current.rotation);
                turn.elapsed = (turn.elapsed + dt).min(turn.duration);
                let t = turn.elapsed / turn.duration;
                let rotation = crate::npc::sample_turn_rotation(from, turn.to, t);
                let _ = crate::npc_fixture_activity::set_pose(
                    world,
                    turn.actor,
                    current.with_rotation(rotation),
                );
                done &= t >= 1.0;
            }
            if !done {
                return Ok(false);
            }
            let turns = std::mem::take(turns);
            for turn in &turns {
                finish_turn(world, turn);
            }
            info!(
                "[npc-group] unit={} frame={frame} talk {}: OnArrive: every turn ended {:?}",
                group.start.main_unit,
                group.start.talk_id,
                turns
                    .iter()
                    .map(|turn| (turn.unit, turn.duration))
                    .collect::<Vec<_>>()
            );
            after_arrive_some_character(world, roster, groups, group, frame)
        }
        Stage::TimelineWait { since } => {
            if frame == *since {
                return Ok(false);
            }
            let state = state_of(world, main).map(|state| state as u8);
            if state.is_some_and(|state| matches!(state, 4 | 11 | 12 | 16 | 18)) {
                return Ok(false);
            }
            info!(
                "[npc-group] unit={} frame={frame} talk {}: the main's state {:?} left the timeline wait; TryPlayTimelineAsync returns true",
                group.start.main_unit, group.start.talk_id, state
            );
            end_group(world, roster, groups, group, frame, GroupEnd::Completes);
            Ok(true)
        }
        Stage::CommunicationYield { since } => {
            if frame == *since {
                return Ok(false);
            }
            group.stage = Stage::Rotating {
                arrived: Vec::new(),
            };
            step_group(world, roster, groups, group, frame, dt, window)
        }
        Stage::Rotating { arrived } => {
            let members: Vec<(u32, Entity)> = group
                .start
                .members
                .iter()
                .filter_map(|unit| roster.get(unit).map(|actor| (*unit, *actor)))
                .collect();
            let all_waiting = members.iter().all(|(_, actor)| {
                world
                    .get::<TalkSlot>(*actor)
                    .and_then(|slot| slot.current.as_ref())
                    .is_some_and(|data| data.kind == TalkType::CommunicationWhileDoingWait)
            });
            if arrived.len() == members.len() {
                show_tweet(world, group, frame);
                let talking = state_of(world, main) == Some(NpcAction::Talk);
                group.stage = if talking {
                    hide_tweet(world, group, frame);
                    Stage::AfterTweet { since: frame }
                } else {
                    Stage::Tweeting { elapsed: dt }
                };
                return Ok(false);
            }
            if !all_waiting {
                // The wait returns false: the communication ends without the
                // tweet, and nothing is owed.
                info!(
                    "[npc-group] unit={} frame={frame} talk {}: a member left the while-doing-wait data; WaitAndRotateCharacter ends",
                    group.start.main_unit, group.start.talk_id
                );
                end_group(world, roster, groups, group, frame, GroupEnd::MainFails);
                return Ok(true);
            }
            for (unit, actor) in &members {
                if arrived.contains(unit) {
                    continue;
                }
                let at_target = world
                    .get::<WalkState>(*actor)
                    .zip(
                        world
                            .get::<TalkSlot>(*actor)
                            .and_then(|slot| slot.current.as_ref()),
                    )
                    .is_some_and(|(walk, data)| {
                        distance(walk.0.position, data.target_position) < ARRIVED_DISTANCE
                    });
                if at_target {
                    arrived.push(*unit);
                    change_state(world, *actor, NpcAction::Communication);
                    info!(
                        "[npc-group] unit={unit} frame={frame} talk {}: arrived; call=RotateCenterAsync({} members); change to SomeCharacterCommunication",
                        group.start.talk_id,
                        arrived.len()
                    );
                }
            }
            Ok(false)
        }
        Stage::Tweeting { elapsed } => {
            let limit = window.ok_or("client configs are not installed")? as f32 / 1000.0;
            let talking = state_of(world, main) == Some(NpcAction::Talk);
            if *elapsed >= limit || talking {
                hide_tweet(world, group, frame);
                group.stage = Stage::AfterTweet { since: frame };
                return Ok(false);
            }
            *elapsed += dt;
            Ok(false)
        }
        Stage::AfterTweet { since } => {
            if frame == *since || state_of(world, main) == Some(NpcAction::Talk) {
                return Ok(false);
            }
            end_group(world, roster, groups, group, frame, GroupEnd::Communicated);
            Ok(true)
        }
    }
}

/// OnArrive for the multiple-character talk. The member loop does nothing:
/// its look reads the main data's locator rotation, which is the zero
/// quaternion (the engine turns a quaternion this short into the identity,
/// so the look's gate is not passed), and the fixture look is only for
/// types 2 and 5. Then, concurrently, every locate row with a unit turns the
/// member of the first row with that action point to the point's StartLoc
/// world rotation: DOLocalRotateAsync with time 0, so the time is
/// GetRotateTime of the Euler end value taken as a position, and the rotate
/// motion of the yaw difference. A row outside the action points is logged
/// and skipped.
///
/// Not carried, named: the rotate motion clip (the host's turn clip belongs
/// to its navigation turn phase, which would also start the member's next
/// waypoint), so a member turns in its current pose, yaw only; the tween's
/// ease is the host's turn ease; a member
/// still on its own fit walk keeps that walk and its last turn (to the same
/// StartLoc rotation) while its OnArrive turn's time runs (in the source the
/// later of the two rotation tweens kills the other); the frame on which a
/// turn of time 0 completes.
fn on_arrive_some_character(
    world: &mut World,
    roster: &Roster,
    group: &Group,
    frame: u32,
) -> Vec<ArriveTurn> {
    let points = world
        .get::<crate::fixture_activity_state::FixtureActivityIdentity>(group.start.fixture)
        .map(|identity| identity.model_package.clone())
        .zip(world.get::<GlobalTransform>(group.start.fixture).copied())
        .and_then(|(package, fixture)| {
            world
                .get_resource::<crate::fixture_attach::AttachPoints>()?
                .source_array(&package, &fixture)
                .ok()
        })
        .unwrap_or_default();
    let mut turns = Vec::new();
    for row in group.start.locate.iter().filter(|row| row.unit != 0) {
        let Some((_, pair)) = points.get(row.index) else {
            warn!(
                "[npc-group] unit={} frame={frame} talk {}: OnArrive: action point {} is outside the {} action points; skipped",
                group.start.main_unit,
                group.start.talk_id,
                row.index,
                points.len()
            );
            continue;
        };
        let owner = group
            .start
            .locate
            .iter()
            .find(|first| first.index == row.index)
            .map_or(row.unit, |first| first.unit) as u32;
        let Some(&actor) = roster.get(&owner) else {
            continue;
        };
        let Some(current) = world.get::<Transform>(actor).copied() else {
            continue;
        };
        let to = pair.start.rotation.normalize();
        let (z, x, y) = to.to_euler(EulerRot::ZXY);
        let end_value = Vec3::new(
            moly_law::path::make_positive_yaw(x.to_degrees()),
            moly_law::path::make_positive_yaw(y.to_degrees()),
            moly_law::path::make_positive_yaw(z.to_degrees()),
        );
        let duration = moly_law::path::rotate_time(moly_law::path::angle_between(
            (current.rotation * Vec3::Z).to_array(),
            (end_value - current.translation).to_array(),
        ));
        turns.push(ArriveTurn {
            unit: owner,
            actor,
            from: None,
            to,
            duration,
            elapsed: 0.0,
        });
    }
    info!(
        "[npc-group] unit={} frame={frame} talk {}: OnArrive: call=RotateCharacterToActionPointRotation {:?} (unit, action point index, GetRotateTime)",
        group.start.main_unit,
        group.start.talk_id,
        group
            .start
            .locate
            .iter()
            .filter(|row| row.unit != 0)
            .map(|row| (
                row.unit,
                row.index,
                turns
                    .iter()
                    .find(|turn| turn.unit as i32 == row.unit)
                    .map(|turn| turn.duration)
            ))
            .collect::<Vec<_>>()
    );
    turns
}

/// TryPlayPreActionAsync (the tweet is refused) and TryPlayTimelineAsync of
/// the multiple-character talk, after OnArrive.
fn after_arrive_some_character(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    group: &mut Group,
    frame: u32,
) -> Result<bool, String> {
    let unit = group.start.main_unit;
    let main = group.start.main;
    info!(
        "[npc-group] unit={unit} frame={frame} talk {}: TryPlayPreActionAsync: the tweet is refused for a multiple-character talk",
        group.start.talk_id
    );
    if group.start.timeline.is_none() {
        info!(
            "[npc-group] unit={unit} frame={frame} talk {}: TryPlayTimelineAsync: no fixture timeline; returns false",
            group.start.talk_id
        );
        end_group(world, roster, groups, group, frame, GroupEnd::Completes);
        return Ok(true);
    }
    // ChangeAnimation(idle, exit time, 0.5, 1.0) is logged;
    // more than one character -> state 16.
    change_state(world, main, NpcAction::MainSomeFixtureAction);
    info!(
        "[npc-group] unit={unit} frame={frame} talk {}: TryPlayTimelineAsync: call=ChangeAnimation(mov_cw_normal_idle001); change to MainNPCAvatarSomeNpcFixtureAction (16)",
        group.start.talk_id
    );
    // State 16's entry runs up to the controller's first await.
    if let Some(controller) = controller::start(world, roster, groups, &group.start, frame) {
        groups.controllers.push(controller);
    }
    group.stage = Stage::TimelineWait { since: frame };
    Ok(false)
}

/// A turn's end: the StartLoc world rotation (a member still on its own fit
/// walk ends it on the same rotation). The member's motion phase is left to
/// its own route: the navigation's turn phase would start its next waypoint.
fn finish_turn(world: &mut World, turn: &ArriveTurn) {
    if matches!(
        world.get::<MotionPhase>(turn.actor),
        Some(
            MotionPhase::Walking | MotionPhase::FitWalking { .. } | MotionPhase::FitTurning { .. }
        )
    ) {
        return;
    }
    if let Some(current) = world.get::<Transform>(turn.actor).copied() {
        let _ = crate::npc_fixture_activity::set_pose(
            world,
            turn.actor,
            current.with_rotation(turn.to),
        );
    }
}

/// The talk objective's catch after a cancel during TryPlayPreActionAsync:
/// every other member not talking ForceUpdateObjective (the main's own calls
/// are the canceller's).
fn main_catch(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    group: &Group,
    frame: u32,
) {
    if let Some(owner) = group.owner {
        if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
            reservations.release_owner(owner);
        }
    }
    let unit = group.start.main_unit;
    let mut updated = Vec::new();
    for member in group.start.members.iter().filter(|member| **member != unit) {
        let Some(&actor) = roster.get(member) else {
            continue;
        };
        if state_of(world, actor) == Some(NpcAction::Talk) {
            continue;
        }
        presenter_force_update(world, groups, actor, *member, frame);
        updated.push(*member);
    }
    info!(
        "[npc-group] unit={unit} frame={frame} talk {}: the objective was cancelled; its catch: ForceUpdateObjective on {:?}",
        group.start.talk_id, updated
    );
}

impl FixtureTalkGroups {
    /// The main's catch for the group of `main`, run by the timeline
    /// controller right after its own ForceUpdateObjective calls (the source
    /// runs it at the objective's next poll; the members' current objectives
    /// stay the cancelled ones until then, so the counts are the same).
    fn catch_now(&mut self, world: &mut World, roster: &Roster, main: Entity, frame: u32) {
        let Some(index) = self.groups.iter().position(|group| {
            group.start.main == main && matches!(group.stage, Stage::TimelineWait { .. })
        }) else {
            return;
        };
        let group = self.groups.remove(index);
        main_catch(world, roster, self, &group, frame);
    }
}

fn site_type_value(world: &World, npc: Entity) -> Option<i32> {
    let site_type = world
        .get::<NpcActions>(npc)
        .map(|actions| actions.site_type.clone());
    site_type.and_then(|name| {
        world
            .get_resource::<crate::player_talk::PlayerTalkSites>()
            .and_then(|sites| sites.type_value_of_name(&name))
    })
}

fn tweet_show(world: &mut World, npc: Entity, unit: u32, tweet_id: i32, text: String) {
    let site = site_type_value(world, npc);
    world.write_message(crate::npc_state::TweetHudEvent::Show {
        npc,
        unit,
        tweet_id,
        text,
        site_type: site,
    });
}

fn tweet_hide(world: &mut World, npc: Entity, unit: u32) {
    let site = site_type_value(world, npc);
    world.write_message(crate::npc_state::TweetHudEvent::Hide {
        npc,
        unit,
        site_type: site,
    });
}

fn show_tweet(world: &mut World, group: &Group, frame: u32) {
    let Some((id, text)) = group.start.tweet.clone() else {
        return;
    };
    tweet_show(world, group.start.main, group.start.main_unit, id, text);
    info!(
        "[npc-group] unit={} frame={frame} talk {}: ShowTweet {id} (the source anchors it at the members' centroid)",
        group.start.main_unit, group.start.talk_id
    );
}

fn hide_tweet(world: &mut World, group: &Group, frame: u32) {
    tweet_hide(world, group.start.main, group.start.main_unit);
    info!(
        "[npc-group] unit={} frame={frame} talk {}: HideTweet",
        group.start.main_unit, group.start.talk_id
    );
}

/// PassTalkDataToSubCharacters, per other member in the data's order: a
/// member in a fixture-action state (11, 12, 17, 18, 19), or one whose
/// TryCancelCurrentObjective reports true, is handed the data; any other
/// gets nothing. For the while-doing-wait talk the hand-over is
/// ForceUpdateCommunicationWhileDoingWaitObjective, which cancels first and
/// goes on only when that reports true (a second cancel on an objective
/// already cancelled reports true): the AI model's reset, then the data.
/// Then the interrupt marker (9, or 6 for the multiple-character talk) and
/// the flag that skips the next rest are set.
fn draft_members(world: &mut World, groups: &mut FixtureTalkGroups, group: &Group, frame: u32) {
    let marker_type = match group.start.kind {
        TalkType::CommunicationWhileDoingWait => 9,
        _ => 6,
    };
    for draft in &group.start.drafts {
        let state = state_of(world, draft.entity);
        let fixture_state =
            state.is_some_and(|state| matches!(state as u8, 11 | 12 | 17 | 18 | 19));
        let eligible = fixture_state
            || {
                let (cancelled, owed) = try_cancel_current(world, groups, draft.entity, frame);
                if owed > 0 {
                    warn!(
                    "[npc-group] unit={} frame={frame} talk {}: the cancelled objective's own ForceUpdateObjective cascade is not drawn (its data is replaced by the reset that follows)",
                    draft.unit, group.start.talk_id
                );
                }
                cancelled
            };
        if !eligible {
            info!(
                "[npc-group] unit={} frame={frame} talk {}: not drafted (state {:?}: talking, or no running objective)",
                draft.unit, group.start.talk_id, state
            );
            continue;
        }
        let handed = match group.start.kind {
            TalkType::CommunicationWhileDoingWait => {
                try_cancel_current(world, groups, draft.entity, frame).0
            }
            _ => true,
        };
        if handed {
            if let Some(mut slot) = world.get_mut::<TalkSlot>(draft.entity) {
                slot.reset_ai_talk_data();
                if let Some(data) = draft.data.clone() {
                    slot.set_current(data);
                }
            }
        }
        if let Some(mut slot) = world.get_mut::<TalkSlot>(draft.entity) {
            slot.interrupt = Some(InterruptMarker {
                marker_type,
                can_interrupt: true,
            });
        }
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(draft.entity) {
            mind.skip_next_rest = true;
        }
        info!(
            "[npc-group] unit={} frame={frame} talk {}: drafted (data handed {}, data {}), interrupt {marker_type}, next rest skipped",
            draft.unit,
            group.start.talk_id,
            handed,
            draft.data.is_some()
        );
    }
}

/// SetUsingFixtureActionPoint: the main first, then the other members in
/// the data's order; the first failure stops.
fn claim_action_points(world: &mut World, roster: &Roster, group: &mut Group) {
    let owner = world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .next_owner(group.start.main);
    group.owner = Some(owner);
    let target = FixtureTarget {
        entity: group.start.fixture,
        uid: group.start.fixture_uid.clone(),
    };
    let mut ids = vec![group.start.main_unit];
    ids.extend(
        group
            .start
            .members
            .iter()
            .copied()
            .filter(|unit| *unit != group.start.main_unit),
    );
    let mut claimed = Vec::new();
    for unit in ids {
        let Some(&actor) = roster.get(&unit) else {
            break;
        };
        let Some(row) = group
            .start
            .locate
            .iter()
            .find(|row| row.unit == unit as i32)
        else {
            break;
        };
        let holds_fixture = world
            .get::<TalkSlot>(actor)
            .and_then(|slot| slot.current.as_ref())
            .is_some_and(|data| data.target_fixture.is_some());
        if !holds_fixture {
            break;
        }
        let Some(name) = group.start.names.get(row.index) else {
            break;
        };
        let slot = slot_of_name(name);
        world
            .resource_mut::<FixtureActivityReservations>()
            .reserve_action_slot(&target, slot, owner);
        claimed.push((unit, row.index, slot));
    }
    info!(
        "[npc-group] unit={} talk {}: SetUsingFixtureActionPoint {:?}",
        group.start.main_unit, group.start.talk_id, claimed
    );
}

/// One pass of WaitOtherCharacterGathering; `Some` when it ended.
fn gathering(
    world: &mut World,
    roster: &Roster,
    start: &GroupStart,
    arrived: &mut Vec<u32>,
    frame: u32,
) -> Option<i32> {
    let targets: Vec<(u32, Entity)> = start
        .members
        .iter()
        .filter(|unit| **unit != start.main_unit)
        .filter_map(|unit| roster.get(unit).map(|actor| (*unit, *actor)))
        .collect();
    if arrived.len() >= targets.len() {
        return Some(1);
    }
    let main_master = world
        .get::<TalkSlot>(start.main)
        .and_then(|slot| slot.current.as_ref())
        .and_then(|data| data.content.as_ref())
        .map(|content| content.master_id);
    for (unit, actor) in targets {
        let Some(data) = world
            .get::<TalkSlot>(actor)
            .and_then(|slot| slot.current.clone())
        else {
            return Some(-1);
        };
        if arrived.contains(&unit) {
            continue;
        }
        if data.kind != start.kind {
            return Some(-1);
        }
        let master = data.content.as_ref().map(|content| content.master_id);
        if main_master.is_none() || master != main_master {
            return Some(-1);
        }
        let position = world.get::<WalkState>(actor)?.0.position;
        if distance(position, data.target_position) < ARRIVED_DISTANCE {
            arrived.push(unit);
            change_state(world, actor, NpcAction::Idle);
            info!(
                "[npc-group] unit={unit} frame={frame} talk {}: gathered ({} of the other members)",
                start.talk_id,
                arrived.len()
            );
        }
    }
    None
}

/// GetArrivedCharacterCount: the members within 0.1 of their own data
/// targets.
fn arrived_count(world: &World, roster: &Roster, members: &[u32]) -> usize {
    members
        .iter()
        .filter(|unit| {
            roster.get(unit).is_some_and(|actor| {
                world
                    .get::<WalkState>(*actor)
                    .zip(
                        world
                            .get::<TalkSlot>(*actor)
                            .and_then(|slot| slot.current.as_ref()),
                    )
                    .is_some_and(|(walk, data)| {
                        distance(walk.0.position, data.target_position) < ARRIVED_DISTANCE
                    })
            })
        })
        .count()
}

/// Model.ForceUpdateObjective from inside a running objective, which then
/// ends without a cancel: one reset and row-1 cascade on the next decision
/// pass.
pub(crate) fn model_force_update(mind: &mut ObjectiveMind, frame: u32) {
    mind.executing = false;
    mind.body = None;
    mind.rest = None;
    mind.yield_since = None;
    mind.skip_next_rest = false;
    mind.force_updates = mind.force_updates.saturating_add(1);
    mind.force_update_end = frame;
}

/// One pass of a member's sub objective; `true` when it ended.
fn step_sub(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    actor: Entity,
    frame: u32,
    dt: f32,
) -> bool {
    let Some(sub) = groups.subs.get_mut(&actor) else {
        return true;
    };
    if world.get::<CharacterUnitId>(actor).is_none() {
        return true;
    }
    let running = world
        .get::<ObjectiveMind>(actor)
        .is_some_and(|mind| mind.executing && mind.current == Some(sub.objective));
    if !running {
        // Another owner (the greeting, a layout edit) cancelled it; the
        // cancel's own ForceUpdateObjective belongs to that owner's path and
        // is not drawn here.
        warn!(
            "[npc-group] unit={} frame={frame} sub objective {} was cancelled by another owner; its cancel's ForceUpdateObjective is not drawn",
            sub.unit, sub.objective as u8
        );
        return true;
    }
    match sub.stage {
        SubStage::Moving => {
            let outcome = world.get_mut::<RouteStops>(actor).and_then(|mut route| {
                route
                    .outcome
                    .take()
                    .map(|outcome| (outcome, route.take_stalled()))
            });
            match outcome {
                None => false,
                Some((RouteOutcome::Arrived, _))
                    if sub.objective == ObjectiveType::SubCharacterFixtureAction =>
                {
                    // Objective 18 after its move: the look at the action
                    // points' centre while not every member has arrived
                    // (Forget, logged), then state 12.
                    let arrived = arrived_count(world, roster, &sub.members);
                    if arrived < sub.members.len() {
                        info!(
                            "[npc-group] unit={} frame={frame} sub objective 18: {arrived} of {} members arrived; call=LookAtActionPointCenterAsync (not played)",
                            sub.unit,
                            sub.members.len()
                        );
                    }
                    change_state(world, actor, NpcAction::FixtureActionIdle);
                    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                        mind.executing = true;
                    }
                    info!(
                        "[npc-group] unit={} frame={frame} sub objective 18 arrived; change to FixtureActionIdle",
                        sub.unit
                    );
                    sub.stage = SubStage::FixtureCheck { since: frame };
                    false
                }
                Some((RouteOutcome::Stopped, _))
                    if sub.objective == ObjectiveType::SubCharacterFixtureAction =>
                {
                    // A failed move status: Model.ForceUpdateObjective, end
                    // (no cancel).
                    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                        model_force_update(&mut mind, frame);
                    }
                    info!(
                        "[npc-group] unit={} frame={frame} sub objective 18 move failed; Model.ForceUpdateObjective",
                        sub.unit
                    );
                    true
                }
                Some((RouteOutcome::Arrived, _)) => {
                    // The sub objective 9: Idle after its move.
                    change_state(world, actor, NpcAction::Idle);
                    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                        mind.executing = true;
                    }
                    info!(
                        "[npc-group] unit={} frame={frame} sub objective {} arrived; change to Idle, waiting for SomeCharacterCommunication",
                        sub.unit, sub.objective as u8
                    );
                    sub.stage = SubStage::WaitCommunication {
                        since: frame,
                        communicating: false,
                    };
                    false
                }
                Some((RouteOutcome::Stopped, _)) => {
                    // A failed move throws the cancellation: OnCancel
                    // changes to Idle and runs one ForceUpdateObjective.
                    change_state(world, actor, NpcAction::Idle);
                    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                        crate::npc_objective::owe_force_updates(&mut mind, frame, 1);
                    }
                    info!(
                        "[npc-group] unit={} frame={frame} sub objective {} move failed; ForceUpdateObjective",
                        sub.unit, sub.objective as u8
                    );
                    true
                }
            }
        }
        SubStage::FixtureCheck { since } => {
            if frame == since {
                return false;
            }
            let own = world
                .get::<TalkSlot>(actor)
                .and_then(|slot| slot.current.clone());
            let Some(own) = own else {
                // The check sets isWaitingStop, then reads the null data's
                // TargetFixture: a null dereference ends this AI.
                let reason =
                    "sub objective 18: its talk data is null at the data check (null dereference)";
                error!("[npc-group] unit={} frame={frame} {reason}", sub.unit);
                if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                    mind.ai_stopped = Some(reason.into());
                    mind.executing = false;
                }
                return true;
            };
            let intact = own.characters.iter().all(|unit| {
                roster
                    .get(unit)
                    .and_then(|member| world.get::<TalkSlot>(*member))
                    .and_then(|slot| slot.current.as_ref())
                    .is_some_and(|data| data.kind == TalkType::MultipleCharacterFixture)
            });
            if !intact {
                if own.target_fixture.is_some() {
                    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                        model_force_update(&mut mind, frame);
                    }
                } else if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                    mind.executing = false;
                    mind.yield_since = Some(frame);
                }
                info!(
                    "[npc-group] unit={} frame={frame} sub objective 18: a member's data is not the multiple-character talk; stops{}",
                    sub.unit,
                    if own.target_fixture.is_some() { " (Model.ForceUpdateObjective)" } else { "" }
                );
                return true;
            }
            change_state(world, actor, NpcAction::SomeFixtureAction);
            info!(
                "[npc-group] unit={} frame={frame} sub objective 18: change to NPCAvatarSomeNpcFixtureAction (18), waiting at most {SUB_FIXTURE_ACTION_TIMEOUT_SECONDS} s",
                sub.unit
            );
            sub.stage = SubStage::FixtureWait {
                since: frame,
                elapsed: 0.0,
            };
            false
        }
        SubStage::FixtureWait { since, elapsed } => {
            if frame == since {
                return false;
            }
            let state = state_of(world, actor).map(|state| state as u8);
            let holding = state.is_some_and(|state| matches!(state, 11 | 12 | 18 | 19));
            let elapsed = elapsed + dt;
            if holding && elapsed < SUB_FIXTURE_ACTION_TIMEOUT_SECONDS {
                sub.stage = SubStage::FixtureWait { since, elapsed };
                return false;
            }
            if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                mind.executing = false;
                mind.body = None;
                mind.yield_since = Some(frame);
            }
            info!(
                "[npc-group] unit={} frame={frame} sub objective 18 ends ({})",
                sub.unit,
                if holding {
                    format!("{SUB_FIXTURE_ACTION_TIMEOUT_SECONDS} s timeout")
                } else {
                    format!("state {state:?} left the wait")
                }
            );
            true
        }
        SubStage::WaitCommunication {
            since,
            communicating,
        } => {
            if frame == since {
                return false;
            }
            if !communicating {
                if state_of(world, actor) == Some(NpcAction::Communication) {
                    sub.stage = SubStage::WaitCommunication {
                        since,
                        communicating: true,
                    };
                }
                return false;
            }
            let members = sub.members.clone();
            let all_waiting = members.iter().all(|unit| {
                roster
                    .get(unit)
                    .and_then(|member| world.get::<TalkSlot>(*member))
                    .and_then(|slot| slot.current.as_ref())
                    .is_some_and(|data| data.kind == TalkType::CommunicationWhileDoingWait)
            });
            if !all_waiting {
                // The cancellation it throws: OnCancel, Idle and one
                // ForceUpdateObjective.
                change_state(world, actor, NpcAction::Idle);
                if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                    crate::npc_objective::owe_force_updates(&mut mind, frame, 1);
                }
                info!(
                    "[npc-group] unit={} frame={frame} sub objective 9: a member left the while-doing-wait data; cancelled",
                    sub.unit
                );
                return true;
            }
            false
        }
    }
}
