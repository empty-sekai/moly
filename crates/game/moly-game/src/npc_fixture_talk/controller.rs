//! The multiple-character fixture talk's timeline: the NPC fixture timeline
//! controller over every member of the talk (state 16's entry for the main).
//!
//! **Entry** (ShowMainNPCTimelineAsync, then CreateFixtureTimelineForNPCAsync
//! and SetupAsync, all before the controller's first await): the asset is
//! the main's own picked timeline; the locate list handed to the controller
//! is rebuilt from the **first** timeline of the same group. A missing
//! fixture, master fixture or group row, or a fixture height other than 0,
//! 0.22 or 0.35, ends the entry without any state change (the main then waits
//! in state 16 with no timeout, the members' waits run to their 60 s).
//! CanPlayFixtureTimeline needs every member to hold data on this fixture
//! (IsBookCharacter); otherwise the controller is disposed at once (state 7
//! for every member, no ForceUpdateObjective). Then talking is disabled for
//! every member and the assets load; a load failure disposes it the same way.
//!
//! **MoveAndPlayTimelineAsync** (a 30.0 s budget of scaled play time counted
//! from the Director's start, lead-in included, paused while a player
//! talks): an action point used by an owner outside this talk makes every
//! member ForceUpdateObjective and disposes it. Otherwise the action slots,
//! each member on its locate row's StartLoc and one Director binding every
//! member by its unit's track. Every per-member step walks the members in
//! the data's order (the unit group's slot order, which does not always put
//! the main first) and finds a member's row by its unit; a member without a
//! row takes the default row, action point 0. The loop clip with the talk
//! flag enables talking and changes every member to state 12 (unless one is
//! talking) and shows the pre-action tweet under the first member, since
//! every member's data holds the whole character list; leaving it disables
//! talking and hides the tweet of every member. At the timeline's end: state
//! 12 for every member, each on its EndLoc, the move back to the walk
//! surface one member after the other, the slots released,
//! ForceUpdateObjective on every member in the data's order, then the
//! disposal (the tweet of every member hidden, talking enabled, state 7).
//!
//! **Not carried, named:** IK and the off-screen renderer flags; the room
//! messages; the idle crossfade before the Director starts (the Director's
//! body tracks drive every bound member from its first frame); the tweet's
//! anchor (the source puts it at the float mean of the members' hip
//! positions, sampled once when the loop first plays; the balloon here
//! follows the first member); the member's pose under StartLoc and on its
//! EndLoc (the source keeps each member's own height and world rotation
//! under StartLoc and rewrites both locators' heights to it; the host places
//! a member at the full locator pose, as the single-character path does);
//! the slot test IsUsingActionSlot is folded into the other-user test (both
//! only fire for a point held by an owner outside this talk, as every point
//! this talk claims is the member's own); the host's pair talk keeps its
//! cast until a joined loop's tail ends, so a timeline that ends under a
//! player talk waits for the talk's end before its own end; a host timeline
//! failure (not a source state) ends it as the cancellation catch does.

use bevy::prelude::*;
use moly_law::objective::ObjectiveType;

use super::{
    change_state, presenter_force_update, state_of, tweet_hide, tweet_show, FixtureTalkGroups,
    GroupStart, Roster,
};
use crate::{
    character::MotionDriver,
    fixture_activity_data::{timeline_asset, FixtureActivityTables, PrepareError},
    fixture_activity_provider::{FixtureActivityProvider, SourceFixtureViewLocalY},
    fixture_activity_state::{
        FixtureActivityIdentity, FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
    },
    fixture_activity_timeline::{
        self as timeline, FixtureActivityTimelines, StartTimeline, TimelineOwner,
        TimelineOwnerKind, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::AttachPoints,
    npc::{MotionPhase, NpcAction, NpcActions, PathSlot, RouteStops},
    npc_fixture_activity::{
        prepare_exit, set_pose, LocalMove, NpcFixtureAnimationOwner, NpcFixtureMotionOwner,
    },
    npc_objective::{ObjectiveMind, TalkSlot},
    npc_talk_lottery::LocateRow,
};

/// MoveAndPlayTimelineAsync's loop budget.
const LOOP_BUDGET_SECONDS: f64 = 30.0;

pub(super) struct Controller {
    pub(super) main: Entity,
    main_unit: u32,
    talk_id: i32,
    owner: FixtureActivityOwner,
    target: FixtureTarget,
    model_package: String,
    names: Vec<String>,
    /// data.NPCAvatarCharacters in the data's order.
    characters: Vec<(u32, Entity)>,
    locate: Vec<LocateRow>,
    package: String,
    prefab: String,
    tweet: Option<(i32, String)>,
    tweet_shown: bool,
    /// In the fixture timeline store (IsPlayingSomeCharacterTalkFixture).
    registered: bool,
    phase: Phase,
}

enum Phase {
    Loading {
        request: Option<StartTimeline>,
        last_pending: Option<String>,
    },
    Playing {
        token: TimelineToken,
        talk_enabled: bool,
    },
    /// Every member is leased and on its StartLoc; the host's preflight of
    /// the bound Director runs before PlayTimelineAsync starts it.
    Preflight { request: Option<StartTimeline> },
    /// The Director ended while a player talk holds the cast.
    TalkHeld,
    /// MoveToBeforePositionAsync.
    Exiting {
        moves: Vec<(Entity, LocalMove, bool)>,
    },
    /// The token and the leases are released.
    Released,
}

/// What `step` does this pass, read before any world change.
enum Pass {
    Loading,
    Playing(TimelineToken, bool),
    Preflight,
    TalkHeld,
    Exiting,
    Released,
}

impl Controller {
    pub(super) fn registered_with(&self, unit: u32) -> bool {
        self.registered
            && self.characters.len() > 1
            && self.characters.iter().any(|(member, _)| *member == unit)
    }

    /// `locate.Find(row => row.unit == member)`: the member's action point
    /// index; the default row (index 0) when it has none.
    fn point_of(&self, unit: u32) -> usize {
        self.locate
            .iter()
            .find(|row| row.unit == unit as i32)
            .map_or(0, |row| row.index)
    }

    fn log(&self, frame: u32, text: impl std::fmt::Display) {
        info!(
            "[npc-group] unit={} frame={frame} talk {} timeline: {text}",
            self.main_unit, self.talk_id
        );
    }
}

/// State 16's entry up to the controller's first await; `None` when it ended
/// there (an early return, or a disposal).
pub(super) fn start(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    start: &GroupStart,
    frame: u32,
) -> Option<Controller> {
    let _ = groups;
    let early = |reason: String| {
        warn!(
            "[npc-group] unit={} frame={frame} talk {} timeline: {reason}; the entry returns without a state change",
            start.main_unit, start.talk_id
        );
        None
    };
    let identity = match world.get::<FixtureActivityIdentity>(start.fixture) {
        Some(identity) if identity.uid == start.fixture_uid => identity.clone(),
        _ => return early("the target fixture does not exist".into()),
    };
    let Some(tables) = world.get_resource::<FixtureActivityTables>() else {
        return early("the talk tables are not installed (host)".into());
    };
    let Some(master_fixture) = tables.fixture_master(identity.master_id) else {
        return early(format!(
            "the master fixture {} does not exist",
            identity.master_id
        ));
    };
    let put_type = master_fixture.put_type;
    let Some(picked) = start.timeline.and_then(|id| tables.timeline(id)).cloned() else {
        return early(format!(
            "the picked timeline {:?} has no master row",
            start.timeline
        ));
    };
    let Some(first) = tables.timeline_rows(picked.group_id).next().cloned() else {
        return early(format!("timeline group {} has no row", picked.group_id));
    };
    let Some(master) = tables.talk_master(start.talk_id) else {
        return early(format!("talk {} has no master row", start.talk_id));
    };
    let names = start.names.clone();
    let locate =
        match crate::npc_talk_lottery::locate_rows(tables, master.unit_group_id, first.id, || {
            Ok(names.clone())
        }) {
            Ok(Some(rows)) => rows,
            Ok(None) => {
                return early(format!(
                    "the locate list of timeline {} is null (no action-point row or unit group)",
                    first.id
                ))
            }
            Err(halt) => return early(halt.to_string()),
        };
    let Some(view_y) = world
        .get::<SourceFixtureViewLocalY>(start.fixture)
        .map(|y| y.0)
    else {
        return early("the fixture's view object does not exist".into());
    };
    let (package, prefab) = match timeline_asset(&picked.asset_name, put_type, view_y) {
        Ok(asset) => asset,
        Err(PrepareError::Rejected(reason)) => {
            return early(format!("invalid fixture height {view_y} ({reason})"))
        }
        Err(error) => return early(format!("timeline asset {}: {error:?}", picked.asset_name)),
    };
    let mut characters = Vec::new();
    for unit in &start.members {
        let Some(&actor) = roster.get(unit) else {
            return early(format!("member {unit} does not exist"));
        };
        characters.push((*unit, actor));
    }
    let owner = world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .next_owner(start.main);
    let mut controller = Controller {
        main: start.main,
        main_unit: start.main_unit,
        talk_id: start.talk_id,
        owner,
        target: FixtureTarget {
            entity: start.fixture,
            uid: identity.uid.clone(),
        },
        model_package: identity.model_package.clone(),
        names: start.names.clone(),
        characters,
        locate,
        package,
        prefab,
        tweet: start.tweet.clone(),
        tweet_shown: false,
        registered: false,
        phase: Phase::Loading {
            request: None,
            last_pending: None,
        },
    };
    controller.log(
        frame,
        format_args!(
            "asset {} (timeline {}), locate {:?} from the group's first timeline {}",
            controller.prefab,
            picked.id,
            controller
                .locate
                .iter()
                .map(|row| (row.unit, row.index, row.slot_id))
                .collect::<Vec<_>>(),
            first.id
        ),
    );
    // SetupAsync: CanPlayFixtureTimeline (IsBookCharacter for every member).
    let unbooked: Vec<u32> = controller
        .characters
        .iter()
        .filter(|(_, actor)| {
            world
                .get::<TalkSlot>(*actor)
                .and_then(|slot| slot.current.as_ref())
                .and_then(|data| data.target_fixture)
                != Some(start.fixture)
        })
        .map(|(unit, _)| *unit)
        .collect();
    if !unbooked.is_empty() {
        controller.log(
            frame,
            format_args!(
                "the fixture reaction could not play; another character may be using it (not booked: {unbooked:?}); disposed"
            ),
        );
        dispose(world, &mut controller, frame);
        return None;
    }
    controller.registered = true;
    for (_, actor) in &controller.characters {
        set_enable_talk(world, *actor, false);
    }
    controller.log(
        frame,
        "SetupAsync: talking disabled for every member; loading",
    );
    Some(controller)
}

fn set_enable_talk(world: &mut World, actor: Entity, enabled: bool) {
    if let Some(mut actions) = world.get_mut::<NpcActions>(actor) {
        actions.enable_talk = enabled;
    }
}

/// The main's objective token (linked into the state's): cancelled once the
/// main's talk objective is no longer running.
fn objective_cancelled(world: &World, main: Entity) -> bool {
    !world.get::<ObjectiveMind>(main).is_some_and(|mind| {
        mind.executing && !mind.cancelled && mind.current == Some(ObjectiveType::Talk)
    })
}

/// One pass; `true` when the controller is gone.
#[allow(clippy::too_many_arguments)]
pub(super) fn step(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    provider: Option<&mut FixtureActivityProvider>,
    controller: &mut Controller,
    frame: u32,
    dt: f32,
) -> bool {
    if controller
        .characters
        .iter()
        .any(|(_, actor)| world.get::<NpcActions>(*actor).is_none())
        || world
            .get::<FixtureActivityIdentity>(controller.target.entity)
            .is_none()
    {
        controller.log(frame, "a member or the fixture is gone; released");
        release(world, controller);
        return true;
    }
    let pass = match &controller.phase {
        Phase::Loading { .. } => Pass::Loading,
        Phase::Playing {
            token,
            talk_enabled,
        } => Pass::Playing(*token, *talk_enabled),
        Phase::Preflight { .. } => Pass::Preflight,
        Phase::TalkHeld => Pass::TalkHeld,
        Phase::Exiting { .. } => Pass::Exiting,
        Phase::Released => Pass::Released,
    };
    match pass {
        Pass::Released => true,
        Pass::Loading => {
            if objective_cancelled(world, controller.main) {
                // The load's await throws; the state's catch ends silently
                // after the set-up's error path disposed it.
                controller.log(frame, "the objective was cancelled while loading; disposed");
                dispose(world, controller, frame);
                return true;
            }
            match load(world, provider, controller) {
                Ok(Some(request)) => {
                    move_and_play(world, roster, groups, controller, request, frame)
                }
                Ok(None) => false,
                Err(reason) => {
                    controller.log(
                        frame,
                        format_args!(
                            "the requested fixture reaction could not be done ({reason}); disposed"
                        ),
                    );
                    dispose(world, controller, frame);
                    true
                }
            }
        }
        Pass::Playing(token, was_enabled) => {
            if objective_cancelled(world, controller.main) {
                controller.log(
                    frame,
                    "the objective was cancelled; the catch releases and ForceUpdates every member",
                );
                finish(world, roster, groups, controller, frame);
                return true;
            }
            let in_talk = world.contains_resource::<crate::player_talk::PlayerTalkSession>()
                || world
                    .get_resource::<crate::talk::ActiveTalk>()
                    .is_some_and(|talk| talk.includes_player());
            let status = {
                let mut timelines = world.resource_mut::<FixtureActivityTimelines>();
                // The view's elapsed time does not advance while talking.
                timelines.set_timeout_budget(token, Some(!in_talk));
                timelines.status(token).cloned()
            };
            let joined = joined_talk(world, controller);
            if joined && controller.tweet_shown {
                // PlaySomeCharacterTalkFixture hides the loop's tweet.
                hide_loop_tweet(world, controller, frame);
            }
            match status {
                Some(TimelineStatus::Preparing) => false,
                Some(TimelineStatus::Playing { enable_talk, .. }) => {
                    for (unit, actor) in controller.characters.clone() {
                        if let Some(pose) = locator_pose(world, controller, unit, false) {
                            let _ = set_pose(world, actor, pose);
                        }
                    }
                    if enable_talk && !was_enabled {
                        loop_talk_enabled(world, controller, frame);
                    } else if !enable_talk && was_enabled {
                        for (_, actor) in controller.characters.clone() {
                            set_enable_talk(world, actor, false);
                        }
                        hide_loop_tweet(world, controller, frame);
                        controller.log(frame, "the loop clip ended: talking disabled");
                    }
                    if let Phase::Playing { talk_enabled, .. } = &mut controller.phase {
                        *talk_enabled = enable_talk;
                    }
                    false
                }
                Some(TimelineStatus::Completed) => {
                    timeline::cancel_and_release(world, token);
                    controller.phase = Phase::TalkHeld;
                    release_animation(world, controller);
                    if joined {
                        for (unit, actor) in controller.characters.clone() {
                            if let Some(pose) = locator_pose(world, controller, unit, true) {
                                let _ = set_pose(world, actor, pose);
                            }
                            let _ = crate::character::resume_idle_after_fixture(world, actor);
                        }
                        controller.log(
                            frame,
                            "the Director ended under a player talk; held until the talk ends",
                        );
                        return false;
                    }
                    controller.log(frame, "the Director ended");
                    begin_exit(world, controller, frame, false)
                }
                Some(TimelineStatus::Cancelled) | Some(TimelineStatus::Failed(_)) | None => {
                    controller.log(
                        frame,
                        format_args!("the host timeline ended without its end ({status:?}); ended as the catch does"),
                    );
                    finish(world, roster, groups, controller, frame);
                    true
                }
            }
        }
        Pass::Preflight => {
            if objective_cancelled(world, controller.main) {
                controller.log(
                    frame,
                    "the objective was cancelled; the catch releases and ForceUpdates every member",
                );
                finish(world, roster, groups, controller, frame);
                return true;
            }
            preflight(world, controller, frame)
        }
        Pass::TalkHeld => {
            if joined_talk(world, controller) {
                return false;
            }
            begin_exit(world, controller, frame, true)
        }
        Pass::Exiting => {
            let Phase::Exiting { moves } = &mut controller.phase else {
                return true;
            };
            // MoveToBeforePositionAsync awaits each member's move before the
            // next member's starts.
            let mut step = None;
            if let Some((actor, movement, finished)) =
                moves.iter_mut().find(|(_, _, finished)| !*finished)
            {
                let (pose, phase, end) = movement.step(dt);
                *finished = end;
                step = Some((*actor, pose, phase));
            }
            let done = moves.iter().all(|(_, _, finished)| *finished);
            if let Some((actor, pose, phase)) = step {
                let _ = set_pose(world, actor, pose);
                world.entity_mut(actor).insert(phase);
            }
            if !done {
                return false;
            }
            finish(world, roster, groups, controller, frame);
            true
        }
    }
}

/// Whether a player talk holds a member on this fixture.
fn joined_talk(world: &World, controller: &Controller) -> bool {
    world
        .get_resource::<crate::talk::ActiveTalk>()
        .is_some_and(|talk| {
            talk.participants().iter().any(|(_, entity)| {
                controller
                    .characters
                    .iter()
                    .any(|(_, actor)| actor == entity)
            })
        })
        || world
            .get_resource::<crate::player_talk::PlayerTalkSession>()
            .is_some_and(|session| {
                controller
                    .characters
                    .iter()
                    .any(|(_, actor)| *actor == session.npc_entity())
            })
}

/// The loop clip with the talk flag: talking enabled for every member, state
/// 12 for every member unless one is talking, and the pre-action tweet of
/// the first member whose data has more than one character.
fn loop_talk_enabled(world: &mut World, controller: &mut Controller, frame: u32) {
    for (_, actor) in controller.characters.clone() {
        set_enable_talk(world, actor, true);
    }
    let talking = controller
        .characters
        .iter()
        .any(|(_, actor)| state_of(world, *actor) == Some(NpcAction::Talk));
    if !talking {
        for (_, actor) in controller.characters.clone() {
            change_state(world, actor, NpcAction::FixtureActionIdle);
        }
    }
    // OnPlayLoopBehaviour: the first member whose data holds more than one
    // character (every member's, for this talk).
    let speaker = controller.characters.iter().copied().find(|(_, actor)| {
        world
            .get::<TalkSlot>(*actor)
            .and_then(|slot| slot.current.as_ref())
            .is_some_and(|data| data.characters.len() > 1)
    });
    if let (Some((unit, actor)), Some((id, text))) = (speaker, controller.tweet.clone()) {
        if id != 0 {
            tweet_show(world, actor, unit, id, text);
            controller.tweet_shown = true;
        }
    }
    controller.log(
        frame,
        format_args!(
            "the loop clip enables talking; {}; tweet {}",
            if talking {
                "a member is talking, no state change"
            } else {
                "every member changes to FixtureActionIdle"
            },
            if controller.tweet_shown {
                "shown (the source anchors it at the members' hip centroid; here the first member)"
            } else {
                "none"
            }
        ),
    );
}

/// DisposeTweet: a hide for every member, whether or not a tweet is shown.
fn hide_loop_tweet(world: &mut World, controller: &mut Controller, frame: u32) {
    let shown = std::mem::replace(&mut controller.tweet_shown, false);
    for (unit, actor) in controller.characters.clone() {
        tweet_hide(world, actor, unit);
    }
    controller.log(
        frame,
        format_args!(
            "DisposeTweet: hidden for every member ({})",
            if shown {
                "the loop tweet was shown"
            } else {
                "none shown"
            }
        ),
    );
}

/// The member's StartLoc (or, with `end`, its EndLoc, else StartLoc) on its
/// locate row's action point (the default row's point 0 without a row).
fn locator_pose(world: &World, controller: &Controller, unit: u32, end: bool) -> Option<Transform> {
    let index = controller.point_of(unit);
    let fixture = world.get::<GlobalTransform>(controller.target.entity)?;
    let points = world.get_resource::<AttachPoints>()?;
    let array = points
        .source_array(&controller.model_package, fixture)
        .ok()?;
    let (_, pair) = array.get(index)?;
    let pose = if end {
        pair.end.unwrap_or(pair.start)
    } else {
        pair.start
    };
    Some(Transform::from_translation(Vec3::from(pose.position)).with_rotation(pose.rotation))
}

/// The asset load: `Ok(Some(request))` once every binding passed its
/// preflight, `Ok(None)` while loading, `Err` on a load failure.
fn load(
    world: &mut World,
    provider: Option<&mut FixtureActivityProvider>,
    controller: &mut Controller,
) -> Result<Option<StartTimeline>, String> {
    let Phase::Loading {
        request,
        last_pending,
    } = &mut controller.phase
    else {
        return Ok(None);
    };
    let Some(provider) = provider else {
        return Ok(None);
    };
    if request.is_none() {
        match provider.definition(world, &controller.package, &controller.prefab) {
            Ok(definition) => {
                *request = Some(StartTimeline {
                    owner: TimelineOwner {
                        activity: controller.owner,
                        kind: TimelineOwnerKind::Npc,
                    },
                    fixture: controller.target.entity,
                    definition,
                    bindings: Default::default(),
                    companions: Vec::new(),
                    timeout_secs: LOOP_BUDGET_SECONDS,
                    timeout_budget: TimelineTimeoutBudget::OwnerGated {
                        advance: Some(true),
                    },
                });
            }
            Err(issue) if issue.retryable => {
                note_pending(controller.main_unit, last_pending, &issue.reason);
                return Ok(None);
            }
            Err(issue) => return Err(format!("{}: {}", issue.stage, issue.reason)),
        }
    }
    let mut cast = Vec::new();
    for (unit, actor) in &controller.characters {
        let Some(driver) = world.get::<MotionDriver>(*actor) else {
            note_pending(
                controller.main_unit,
                last_pending,
                "a member's animator is loading",
            );
            return Ok(None);
        };
        cast.push((*unit, *actor, driver.player, driver.graph.clone()));
    }
    let prepared = request.as_mut().expect("installed request");
    match provider.prepare_talk_cast_bindings(world, prepared, &cast) {
        Ok(()) => {}
        Err(issue) if issue.retryable => {
            note_pending(controller.main_unit, last_pending, &issue.reason);
            return Ok(None);
        }
        Err(issue) => return Err(format!("{}: {}", issue.stage, issue.reason)),
    }
    // A member without a track of its unit is not animated by the Director
    // (the source binds by name and leaves an unnamed character alone).
    let unbound: Vec<u32> = cast
        .iter()
        .filter(|(_, _, animator, _)| {
            !prepared
                .bindings
                .animations
                .values()
                .any(|binding| binding.animator == *animator)
        })
        .map(|(unit, _, _, _)| *unit)
        .collect();
    if !unbound.is_empty() && last_pending.as_deref() != Some("unbound") {
        info!(
            "[npc-group] unit={} timeline {}: no body track for units {unbound:?}",
            controller.main_unit, controller.prefab
        );
    }
    Ok(request.take())
}

fn note_pending(unit: u32, last: &mut Option<String>, reason: &str) {
    if last.as_deref() != Some(reason) {
        info!("[npc-group] unit={unit} timeline loading: {reason}");
        *last = Some(reason.to_owned());
    }
}

/// MoveAndPlayTimelineAsync up to PlayTimelineAsync; `true` when it ended.
fn move_and_play(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    controller: &mut Controller,
    request: StartTimeline,
    frame: u32,
) -> bool {
    // IsOtherCharacterUsing: an action point in use by an owner outside this
    // talk (the members' own claims are exempt).
    let group_owner = groups
        .groups
        .iter()
        .find(|group| group.start.main == controller.main)
        .and_then(|group| group.owner);
    let other = {
        let reservations = world.resource::<FixtureActivityReservations>();
        controller
            .names
            .iter()
            .enumerate()
            .find_map(|(index, name)| {
                let slot = super::slot_of_name(name);
                let held = reservations
                    .action_slot_owner(&controller.target, slot)
                    .is_some_and(|owner| owner != controller.owner && Some(owner) != group_owner);
                (held || reservations.player_point_in_use(&controller.target, index))
                    .then(|| format!("action point {index} (slot {slot})"))
            })
    };
    if let Some(point) = other {
        controller.log(
            frame,
            format_args!("{point} is used by another character; SendFixtureTimelineCancel, ForceUpdateObjective on every member"),
        );
        finish(world, roster, groups, controller, frame);
        return true;
    }
    // SetActionSlot(true): per member, SetActionSlotFromLocateIndex on its
    // row's action point (the slot its StartLoc name carries); then the
    // NavMesh agents off, the idle motion, StartLoc.
    let slots: Vec<i32> = controller
        .characters
        .iter()
        .map(|(unit, _)| {
            controller
                .names
                .get(controller.point_of(*unit))
                .map_or(0, |name| super::slot_of_name(name))
        })
        .collect();
    for slot in &slots {
        world
            .resource_mut::<FixtureActivityReservations>()
            .reserve_action_slot(&controller.target, *slot, controller.owner);
    }
    controller.phase = Phase::Preflight {
        request: Some(request),
    };
    for (unit, actor) in controller.characters.clone() {
        world.entity_mut(actor).insert((
            NpcFixtureMotionOwner(controller.owner),
            NpcFixtureAnimationOwner(controller.owner),
        ));
        if let Some(mut route) = world.get_mut::<RouteStops>(actor) {
            route.hold_for_fixture();
        }
        if let Some(mut path) = world.get_mut::<PathSlot>(actor) {
            path.0 = moly_law::path::NpcPathWalkSlot::from_corners(Vec::new());
        }
        world
            .entity_mut(actor)
            .insert(MotionPhase::Dwelling { remaining: None });
        if let Some(mut driver) = world.get_mut::<MotionDriver>(actor) {
            driver.playing = None;
        }
        if let Some(pose) = locator_pose(world, controller, unit, false) {
            if let Err(reason) = set_pose(world, actor, pose) {
                warn!("[npc-group] unit={unit} StartLoc pose: {reason}");
            }
        }
    }
    controller.log(
        frame,
        format_args!("MoveAndPlayTimelineAsync: slots {slots:?}, every member on its StartLoc"),
    );
    preflight(world, controller, frame)
}

/// The host's preflight of the bound Director (not a source step), then
/// PlayTimelineAsync; a preflight that cannot pass ends as the load failure.
fn preflight(world: &mut World, controller: &mut Controller, frame: u32) -> bool {
    let Phase::Preflight { request } = &mut controller.phase else {
        return false;
    };
    let Some(prepared) = request.as_ref() else {
        return false;
    };
    match timeline::validate_start(world, prepared) {
        Ok(()) => {}
        Err(error) if error.retryable => return false,
        Err(error) => {
            controller.log(
                frame,
                format_args!(
                    "the requested fixture reaction could not be done ({error}); disposed"
                ),
            );
            dispose(world, controller, frame);
            return true;
        }
    }
    let request = request.take().expect("checked above");
    let token = world
        .resource_mut::<FixtureActivityTimelines>()
        .request_start(request);
    controller.phase = Phase::Playing {
        token,
        talk_enabled: false,
    };
    controller.log(
        frame,
        format_args!("PlayTimelineAsync({LOOP_BUDGET_SECONDS})"),
    );
    false
}

/// ChangeStateNPCs(12), each member on its EndLoc, then the move back to the
/// walk surface; `true` when it ended.
fn begin_exit(world: &mut World, controller: &mut Controller, frame: u32, from_live: bool) -> bool {
    let mut moves = Vec::new();
    for (unit, actor) in controller.characters.clone() {
        change_state(world, actor, NpcAction::FixtureActionIdle);
        let Some(end) = locator_pose(world, controller, unit, true) else {
            continue;
        };
        let start = if from_live {
            world.get::<Transform>(actor).copied().unwrap_or(end)
        } else {
            let _ = set_pose(world, actor, end);
            end
        };
        match prepare_exit(world, actor, start, end) {
            Ok(movement) => moves.push((actor, movement, false)),
            Err(reason) => warn!("[npc-group] unit={unit} exit: {reason}"),
        }
    }
    controller.log(
        frame,
        "every member changes to FixtureActionIdle, on its EndLoc; MoveToBeforePositionAsync",
    );
    controller.phase = Phase::Exiting { moves };
    false
}

fn release_animation(world: &mut World, controller: &Controller) {
    for (_, actor) in &controller.characters {
        if world
            .get::<NpcFixtureAnimationOwner>(*actor)
            .is_some_and(|held| held.0 == controller.owner)
        {
            world
                .entity_mut(*actor)
                .remove::<NpcFixtureAnimationOwner>();
            if let Some(mut driver) = world.get_mut::<MotionDriver>(*actor) {
                driver.playing = None;
            }
        }
    }
}

/// The token, the leases and the slots; no state change.
fn release(world: &mut World, controller: &mut Controller) {
    if let Phase::Playing { token, .. } = controller.phase {
        timeline::cancel_and_release(world, token);
    }
    controller.phase = Phase::Released;
    release_animation(world, controller);
    for (_, actor) in &controller.characters {
        if world
            .get::<NpcFixtureMotionOwner>(*actor)
            .is_some_and(|held| held.0 == controller.owner)
        {
            world.entity_mut(*actor).remove::<NpcFixtureMotionOwner>();
            if let Some(mut route) = world.get_mut::<RouteStops>(*actor) {
                route.finish_fixture();
            }
        }
    }
    if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
        reservations.release_owner(controller.owner);
    }
    controller.registered = false;
}

/// The controller's end (the timeline's end, the other-user test, the
/// cancellation catch): the slots released, ForceUpdateObjective on every
/// member in the data's order, the disposal, then the main's catch.
fn finish(
    world: &mut World,
    roster: &Roster,
    groups: &mut FixtureTalkGroups,
    controller: &mut Controller,
    frame: u32,
) {
    release(world, controller);
    for (unit, actor) in controller.characters.clone() {
        presenter_force_update(world, groups, actor, unit, frame);
    }
    dispose(world, controller, frame);
    groups.catch_now(world, roster, controller.main, frame);
}

/// Dispose: the tweet disposed, talking enabled, out of the store, state 7
/// for every member.
fn dispose(world: &mut World, controller: &mut Controller, frame: u32) {
    hide_loop_tweet(world, controller, frame);
    release(world, controller);
    for (_, actor) in controller.characters.clone() {
        set_enable_talk(world, actor, true);
        change_state(world, actor, NpcAction::Rest);
    }
    controller.log(
        frame,
        "disposed: talking enabled, every member changes to Rest (7)",
    );
}
