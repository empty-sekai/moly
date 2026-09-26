//! The change-site objective (`NpcAvatarChangeSiteObjective`) and the
//! change-site state (`NPCAvatarChangeSiteState`) of an NPC the change-site
//! controller moves (see `npc::change_site`).
//!
//! The source's shape, in order:
//! - **The order** (`MoveNPC`): the NPC's current objective is cancelled
//!   (nothing more when that reports false); the AI model is reset and given
//!   change-site talk data aimed at the door action point of the NPC's own
//!   site (the house's inside door on home, the room door on a floor), the
//!   target site type is kept, the next rest is skipped and the change-site
//!   interrupt (5) is raised. The controller's monitor then reads the NPC's
//!   current objective type, which is still the cancelled objective's (the
//!   AI loop yields before it decides again), so it returns at once and the
//!   controller sends the next order in the same frame.
//! - **The objective** (the NPC's next decision, one frame later): when the
//!   site the player is on is not the target, `ForceUpdateObjective` and
//!   done. Otherwise it walks to that door (goal distance 0.1); a route that
//!   fails ends it the same way; then the state changes to ChangeSite (14)
//!   and the objective waits until the site change is over, then
//!   `ForceUpdateObjective`.
//! - **The state**, entering a floor (`FloorEnter`): stop; wait until the
//!   player, then every other NPC, is more than 0.7 from the state's start
//!   position (kept from the previous change, zero at the first); wait until
//!   the game is neither editing nor levelling up the room; the leaving leg
//!   on the site it leaves; then the start position becomes the room's door
//!   action point, the NPC is placed there on the target site type, hidden,
//!   turned to the door's forward; on the player's site the knock sound
//!   plays to its end and the door-open sound starts; the door opens; the
//!   NPC is shown and plays `mov_cw_all_house_in_outside_O`; the door closes
//!   (not awaited); then the room-entry tweet: it turns to the player, picks
//!   one of its site-entry tweets (one engine pick), shows it for 3000 ms
//!   and hides it; the next objective is set to run at once.
//! - Entering home (`HomeSiteEnter`): the start position is the house's
//!   outside door action point before the waits; after them the leaving leg,
//!   the placement there on home, hidden, a turn to the door's forward, the
//!   house's `NPCOn` trigger (played by the house lane, see
//!   `fixture_gimmick::house_door`), and `mov_cw_all_house_open_011_O` with
//!   the NPC shown one frame after it starts. No tweet.
//!
//! Host shape. This host holds one site at a time, and every order targets
//! the site the player has just entered, so the walk to the door and the
//! leaving leg always run on a site that is not loaded; the NPC is suspended
//! there (see `npc::residency`). Named gaps:
//! - the walk to the own door and the leaving leg (the house or room exit
//!   clip) are not run: the unseen site has no ground here; the objective
//!   proceeds as if the walk succeeded on its first frame;
//! - the other-NPC wait compares the NPCs on the loaded site only (an NPC on
//!   another site stands on that site's ground, away from this door in the
//!   source's world); the zero start position of a first change is this
//!   host's world origin;
//! - the entry turns are the view's look-at tween (see `npc_look_at`),
//!   awaited: on home towards the outside door's forward over the rotate
//!   time (the angle to turn / 60 s) with an out-quart ease, on a floor
//!   towards the room door's forward over 0 s (it completes at its first
//!   update, the next frame); the turn clip the presenter starts with each
//!   plays through the motion driver while the look-at runs (the character
//!   is hidden meanwhile), the idle clip after it is the driver's idle, and
//!   the wait on that idle clip's change is taken as none;
//! - the room-entry tweet's turn to the player is the same awaited look-at
//!   (towards the player's position over the rotate time, out-quart), with
//!   the same clips; the knock sound's wait and the door's open
//!   wait end at once (this host reads
//!   neither the sound nor the door clip's length back); the brightness
//!   fade, the tweet's eye, mouth and emoticon, and the facial reset are not
//!   played.

use std::collections::HashMap;

use bevy::animation::graph::{AnimationGraph, AnimationNodeIndex};
use bevy::ecs::system::SystemState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_law::objective::change_site as law;
use moly_law::objective::{InterruptMarker, ObjectiveType, TalkType};

use super::residency::{self, Away};
use crate::npc::{CharacterUnitId, NpcAction, NpcActions, RestLifecycle, WalkState};
use crate::npc_objective::{AiTalkData, ObjectiveMind, TalkSlot};

/// The change-site interrupt `MoveNPC` raises.
const CHANGE_SITE_INTERRUPT: InterruptMarker = InterruptMarker {
    marker_type: 5,
    can_interrupt: true,
};

/// The small step along the door's forward the entry turns aim at.
const DOOR_LOOK_OFFSET: f32 = 0.01;

/// The zero-target test of the awaited look-at: a target point this close
/// to the world origin (squared) makes no turn.
const LOOK_AT_ZERO_BOUND: f32 = 9.9999994e-11;

/// What follows an entry turn.
#[derive(Debug, Clone, Copy)]
enum AfterTurn {
    /// HomeSiteEnter: the house's trigger, then the entry clip.
    HomeClip,
    /// FloorEnter: the knock, the door, the entry clip.
    FloorKnock,
    /// ShowTweet: the tweet after the turn to the player.
    EntryTweet,
}

/// The floor entry clip.
const FLOOR_ENTER_CLIP: &str = "mov_cw_all_house_in_outside_O";
/// The home entry clip.
const HOME_ENTER_CLIP: &str = "mov_cw_all_house_open_011_O";

/// The wait or step one run is at. Every wait is created on `since` and first
/// tested on a later frame.
#[derive(Debug, Clone, Copy)]
enum Stage {
    /// `MoveNPC` ran; the AI loop yields, then decides the change-site
    /// objective.
    Ordered,
    /// The NPC's own decision built the objective (see
    /// [`ChangeSiteRuns::decided`]); its body starts at the next step.
    Objective,
    WaitPlayerLeaveDoor,
    WaitOtherNpcLeaveDoor,
    WaitNotMoveMode,
    /// FloorEnter's second wait, after the placement.
    WaitNotMoveModeAfterPlace,
    /// An awaited entry turn (the look-at tween) is running.
    Turning { next: AfterTurn },
    /// The entry clip plays; home shows the NPC one frame after it starts.
    EntryClip {
        node: AnimationNodeIndex,
        shown: bool,
    },
    /// The room-entry tweet is shown.
    Tweet {
        delay: moly_law::objective::DelayPromise,
    },
    /// The state's setup is over; the objective's wait sees it next frame.
    SetupDone,
}

struct Run {
    unit: u32,
    target: i32,
    stage: Stage,
    since: u32,
    door_forward: Vec3,
}

/// The runs in flight and each NPC's kept start position.
#[derive(Resource, Default)]
pub(crate) struct ChangeSiteRuns {
    runs: HashMap<Entity, Run>,
    /// `NPCAvatarChangeSiteState._animationStartPosition`: the state object is
    /// kept per NPC, so the value survives from one change to the next.
    start_positions: HashMap<Entity, Vec3>,
    /// The AI model's TargetMoveSiteType, written by `MoveNPC` and kept.
    target_sites: HashMap<Entity, i32>,
}

impl ChangeSiteRuns {
    /// The NPC's decision built a change-site objective (row 8, or the
    /// change-site interrupt): its body runs from the next step on, towards
    /// the kept target site (-1 when no order ever wrote one).
    pub(crate) fn decided(&mut self, actor: Entity, unit: u32, frame: u32) {
        let target = self.target_sites.get(&actor).copied().unwrap_or(-1);
        self.runs.insert(
            actor,
            Run {
                unit,
                target,
                stage: Stage::Objective,
                since: frame,
                door_forward: Vec3::Z,
            },
        );
    }
}

/// Marker: the change-site state plays a clip on this NPC; the locomotion
/// driver leaves its animation player alone.
#[derive(Component)]
pub(crate) struct ChangeSiteClip;

/// `MoveNPC(npc, target)`. Returns whether the order took.
pub(crate) fn move_npc(
    world: &mut World,
    actor: Entity,
    unit: u32,
    target: i32,
    frame: u32,
) -> bool {
    let (cancelled, owed) = world.resource_scope(
        |world, mut groups: Mut<crate::npc_fixture_talk::FixtureTalkGroups>| {
            crate::npc_fixture_talk::try_cancel_current(world, &mut groups, actor, frame)
        },
    );
    if !cancelled {
        info!(
            "[npc-change-site] unit={unit} frame={frame} MoveNPC(target {target}): TryCancelCurrentObjective false, no order"
        );
        return false;
    }
    // SetSiteChangeObjective: the door action point of the NPC's own site.
    let own = world
        .get::<NpcActions>(actor)
        .and_then(|actions| residency::site_type_value(&actions.site_type));
    let door = door_point(world, own);
    world
        .resource_mut::<crate::npc_fixture_activity::NpcFixtureActivities>()
        .clear_forced(actor);
    if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
        slot.reset_ai_talk_data();
        slot.set_current(AiTalkData {
            kind: TalkType::ChangeSite,
            content: None,
            target_fixture: None,
            target_position: door
                .map(|(position, _)| position.to_array())
                .unwrap_or_default(),
            main_character: unit,
            characters: vec![unit],
            pre_action: None,
            locate: None,
            pending_factory: None,
        });
        slot.interrupt = Some(CHANGE_SITE_INTERRUPT);
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        // The cancelled objective's own ForceUpdateObjective ran inside the
        // cancel, before this reset: nothing stays owed.
        mind.force_updates = 0;
        mind.skip_next_rest = true;
    }
    world
        .resource_mut::<ChangeSiteRuns>()
        .target_sites
        .insert(actor, target);
    world.resource_mut::<ChangeSiteRuns>().runs.insert(
        actor,
        Run {
            unit,
            target,
            stage: Stage::Ordered,
            since: frame,
            door_forward: Vec3::Z,
        },
    );
    info!(
        "[npc-change-site] unit={unit} frame={frame} MoveNPC(target {target}): cancelled{}, change-site data to the door of its site {own:?} ({}), target kept, next rest skipped, interrupt 5",
        if owed > 0 {
            format!(" (its cancel's {owed} ForceUpdateObjective not drawn: the reset follows)")
        } else {
            String::new()
        },
        match door {
            Some((position, _)) => format!("({:.2},{:.2},{:.2})", position.x, position.y, position.z),
            None => "that site is not loaded here".to_owned(),
        }
    );
    true
}

/// The monitor's predicate on one NPC: `true` while it keeps waiting (the
/// NPC's current objective is the change-site one and it is not yet on the
/// target).
pub(crate) fn monitor_waits(world: &World, actor: Entity, target: i32) -> bool {
    let objective = world
        .get::<ObjectiveMind>(actor)
        .and_then(|mind| mind.current)
        .map_or(0, |objective| objective as i32);
    let own = world
        .get::<NpcActions>(actor)
        .and_then(|actions| residency::site_type_value(&actions.site_type))
        .unwrap_or(-1);
    law::monitor_waits(false, objective, own, target)
}

/// The door action point of a site, when that site is the loaded one:
/// position and forward.
fn door_point(world: &mut World, site: Option<i32>) -> Option<(Vec3, Vec3)> {
    let loaded = world
        .get_resource::<crate::site::SiteActive>()
        .and_then(|site| residency::site_type_value(&site.site_type));
    if site.is_none() || site != loaded {
        return None;
    }
    let point = match site? {
        0 => match crate::fixture_gimmick::house_door::find_house(world) {
            crate::fixture_gimmick::house_door::HouseLookup::Found(binding) => binding.inside_door,
            _ => None,
        },
        1..=3 => crate::site_move::room_door::inside_point(world),
        _ => None,
    }?;
    let transform = world.get::<GlobalTransform>(point)?;
    Some((transform.translation(), transform.rotation() * Vec3::Z))
}

/// The entry point on the target site: the room's door action point, or the
/// house's outside door action point. `None` while it is not bound.
fn entry_point(world: &mut World, target: i32) -> Option<(Vec3, Vec3)> {
    let point = match target {
        0 => match crate::fixture_gimmick::house_door::find_house(world) {
            crate::fixture_gimmick::house_door::HouseLookup::Found(binding) => binding.outside_door,
            _ => None,
        },
        _ => crate::site_move::room_door::inside_point(world),
    }?;
    let transform = world.get::<GlobalTransform>(point)?;
    Some((transform.translation(), transform.rotation() * Vec3::Z))
}

/// Update: every run in flight, one step per frame.
pub(crate) fn advance(world: &mut World) {
    if world.resource::<ChangeSiteRuns>().runs.is_empty() {
        return;
    }
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let actors: Vec<Entity> = world
        .resource::<ChangeSiteRuns>()
        .runs
        .keys()
        .copied()
        .collect();
    for actor in actors {
        step(world, actor, frame);
    }
}

fn player_site(world: &World) -> Option<i32> {
    world
        .get_resource::<crate::site::SiteActive>()
        .and_then(|site| residency::site_type_value(&site.site_type))
}

fn set_stage(world: &mut World, actor: Entity, stage: Stage, frame: u32) {
    if let Some(run) = world.resource_mut::<ChangeSiteRuns>().runs.get_mut(&actor) {
        run.stage = stage;
        run.since = frame;
    }
}

fn end_run(world: &mut World, actor: Entity) {
    world.resource_mut::<ChangeSiteRuns>().runs.remove(&actor);
    world.entity_mut(actor).remove::<ChangeSiteClip>();
}

/// `ForceUpdateObjective` from inside the objective, which then ends.
fn force_update_objective(world: &mut World, actor: Entity, frame: u32) {
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        crate::npc_fixture_talk::model_force_update(&mut mind, frame);
    }
}

fn change_state(world: &mut World, actor: Entity, next: NpcAction) {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = query.get_mut(world, actor) {
        actions.change(next, &mut rest);
    }
}

fn step(world: &mut World, actor: Entity, frame: u32) {
    let Some((unit, target, stage, since)) = world
        .resource::<ChangeSiteRuns>()
        .runs
        .get(&actor)
        .map(|run| (run.unit, run.target, run.stage, run.since))
    else {
        return;
    };
    if world.get_entity(actor).is_err() {
        world.resource_mut::<ChangeSiteRuns>().runs.remove(&actor);
        return;
    }
    if frame == since && !matches!(stage, Stage::Objective) {
        return;
    }
    // Another owner cancelled the objective (a draft, a forced action): the
    // change-site objective has ended; the state's steps stop with it.
    if !matches!(stage, Stage::Ordered) {
        let running = world.get::<ObjectiveMind>(actor).is_some_and(|mind| {
            mind.current == Some(ObjectiveType::ChangeSite) && !mind.cancelled
        });
        if !running {
            if world.get::<ChangeSiteClip>(actor).is_some() {
                end_clip(world, actor, unit);
            }
            if let Some(mut visibility) = world.get_mut::<Visibility>(actor) {
                *visibility = Visibility::Inherited;
            }
            info!("[npc-change-site] unit={unit} frame={frame} the change-site objective was cancelled by another owner: its state steps stop");
            world.resource_mut::<ChangeSiteRuns>().runs.remove(&actor);
            return;
        }
    }
    let target_name = residency::SITE_TYPES
        .get(target as usize)
        .copied()
        .unwrap_or("none");
    match stage {
        Stage::Ordered | Stage::Objective => {
            if matches!(stage, Stage::Ordered) {
                // The NPC's next decision: the change-site interrupt (5)
                // builds the change-site objective, and the interrupt flag is
                // cleared.
                if let Some(mut slot) = world.get_mut::<TalkSlot>(actor) {
                    if let Some(marker) = slot.interrupt.as_mut() {
                        marker.can_interrupt = false;
                    }
                }
                if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
                    mind.yield_since = None;
                    mind.skip_next_rest = false;
                    mind.begin_objective(ObjectiveType::ChangeSite);
                    mind.executing = true;
                }
            }
            if player_site(world) != Some(target) {
                info!(
                    "[npc-change-site] unit={unit} frame={frame} change-site objective: the player's site {:?} is not the target {target}: ForceUpdateObjective",
                    player_site(world)
                );
                force_update_objective(world, actor, frame);
                end_run(world, actor);
                return;
            }
            // MoveAsync to the own door runs on the unseen site (not run
            // here; see the module notes), then ChangeStatus(ChangeSite). An
            // objective the NPC decided on the loaded site walks to its
            // data's door position first; that walk is not run here either.
            change_state(world, actor, NpcAction::ChangeSite);
            let start = if target == 0 {
                // HomeSiteEnter takes its start position before the waits.
                match entry_point(world, 0) {
                    Some((position, forward)) => {
                        if let Some(run) =
                            world.resource_mut::<ChangeSiteRuns>().runs.get_mut(&actor)
                        {
                            run.door_forward = forward;
                        }
                        world
                            .resource_mut::<ChangeSiteRuns>()
                            .start_positions
                            .insert(actor, position);
                        Some(position)
                    }
                    None => {
                        warn!(
                            "[npc-change-site] unit={unit} frame={frame} HomeSiteEnter: the house's outside door action point is not bound here; the change ends (ForceUpdateObjective)"
                        );
                        force_update_objective(world, actor, frame);
                        end_run(world, actor);
                        return;
                    }
                }
            } else {
                None
            };
            info!(
                "[npc-change-site] unit={unit} frame={frame} change-site objective on target {target} ({target_name}): the walk to its own door is on the unseen site (not run), ChangeStatus(ChangeSite); {} starts, start position {:?}",
                if target == 0 { "HomeSiteEnter" } else { "FloorEnter" },
                start.or_else(|| world.resource::<ChangeSiteRuns>().start_positions.get(&actor).copied())
                    .unwrap_or(Vec3::ZERO),
            );
            set_stage(world, actor, Stage::WaitPlayerLeaveDoor, frame);
        }
        Stage::WaitPlayerLeaveDoor => {
            let start = start_position(world, actor);
            let player = player_position(world);
            if player.is_some_and(|player| law::clear_of_door(player.to_array(), start.to_array())) {
                set_stage(world, actor, Stage::WaitOtherNpcLeaveDoor, frame);
            }
        }
        Stage::WaitOtherNpcLeaveDoor => {
            let start = start_position(world, actor);
            let mut others = world.query_filtered::<(Entity, &Transform), (
                With<CharacterUnitId>,
                Without<Away>,
                Without<crate::player::PlayerControlled>,
            )>();
            let clear = others
                .iter(world)
                .filter(|(entity, _)| *entity != actor)
                .all(|(_, transform)| {
                    law::clear_of_door(transform.translation.to_array(), start.to_array())
                });
            if clear {
                set_stage(world, actor, Stage::WaitNotMoveMode, frame);
            }
        }
        Stage::WaitNotMoveMode => {
            if !can_show_entry_reaction(world) {
                return;
            }
            // The leaving leg runs on the site it leaves (not run here).
            let entry = entry_point(world, target);
            let Some((position, forward)) = entry else {
                warn!(
                    "[npc-change-site] unit={unit} frame={frame} the entry point of {target_name} is not bound here; the change ends (ForceUpdateObjective)"
                );
                change_state(world, actor, NpcAction::Idle);
                force_update_objective(world, actor, frame);
                end_run(world, actor);
                return;
            };
            if let Some(run) = world.resource_mut::<ChangeSiteRuns>().runs.get_mut(&actor) {
                run.door_forward = forward;
            }
            world
                .resource_mut::<ChangeSiteRuns>()
                .start_positions
                .insert(actor, position);
            place_on_target(world, actor, target, position, frame);
            info!(
                "[npc-change-site] unit={unit} frame={frame} placed at the entry point ({:.2},{:.2},{:.2}) of {target_name}, UpdateCurrentSiteType({target}), hidden",
                position.x, position.y, position.z
            );
            if target == 0 {
                // HomeSiteEnter: DoLookAtAsync(position + outside door forward
                // * 0.01, rotate time, out-quart), awaited.
                let towards = position + forward * DOOR_LOOK_OFFSET;
                if begin_turn(world, actor, unit, towards, None, frame) {
                    set_stage(world, actor, Stage::Turning { next: AfterTurn::HomeClip }, frame);
                } else {
                    home_clip(world, actor, unit, frame);
                }
            } else {
                set_stage(world, actor, Stage::WaitNotMoveModeAfterPlace, frame);
            }
        }
        Stage::WaitNotMoveModeAfterPlace => {
            if !can_show_entry_reaction(world) {
                return;
            }
            // DoLookAtAsync(position + room door forward * 0.01, 0), awaited.
            let position = world
                .get::<Transform>(actor)
                .map(|transform| transform.translation)
                .unwrap_or(Vec3::ZERO);
            let forward = world
                .resource::<ChangeSiteRuns>()
                .runs
                .get(&actor)
                .map(|run| run.door_forward)
                .unwrap_or(Vec3::Z);
            let towards = position + forward * DOOR_LOOK_OFFSET;
            if begin_turn(world, actor, unit, towards, Some(0.0), frame) {
                set_stage(world, actor, Stage::Turning { next: AfterTurn::FloorKnock }, frame);
            } else {
                floor_knock(world, actor, unit, frame);
            }
        }
        Stage::Turning { next } => {
            if world.get::<crate::npc_look_at::NpcLookAt>(actor).is_some() {
                return;
            }
            info!("[npc-change-site] unit={unit} frame={frame} entry turn done (the idle clip after it is not played)");
            match next {
                AfterTurn::HomeClip => home_clip(world, actor, unit, frame),
                AfterTurn::FloorKnock => floor_knock(world, actor, unit, frame),
                AfterTurn::EntryTweet => entry_tweet(world, actor, unit, frame),
            }
        }
        Stage::EntryClip { node, shown } => {
            if !shown {
                // ShowAfterOneFrame.
                if let Some(mut visibility) = world.get_mut::<Visibility>(actor) {
                    *visibility = Visibility::Inherited;
                }
                if let Some(run) = world.resource_mut::<ChangeSiteRuns>().runs.get_mut(&actor) {
                    run.stage = Stage::EntryClip { node, shown: true };
                }
            }
            if !clip_finished(world, actor, node) {
                return;
            }
            end_clip(world, actor, unit);
            if target == 0 {
                info!("[npc-change-site] unit={unit} frame={frame} HomeSiteEnter done (no entry tweet on home)");
                set_stage(world, actor, Stage::SetupDone, frame);
                return;
            }
            crate::site_move::room_door::close(world);
            // OnEntrySite: ShowTweet, which first turns to the player.
            match begin_entry_tweet(world, actor, unit, frame) {
                None => on_entry_site_end(world, actor, unit, frame),
                Some(true) => set_stage(
                    world,
                    actor,
                    Stage::Turning {
                        next: AfterTurn::EntryTweet,
                    },
                    frame,
                ),
                Some(false) => entry_tweet(world, actor, unit, frame),
            }
        }
        Stage::Tweet { mut delay } => {
            let dt = world.resource::<crate::npc_clock::NpcClock>().delta();
            if !delay.advance(dt) {
                if let Some(run) = world.resource_mut::<ChangeSiteRuns>().runs.get_mut(&actor) {
                    run.stage = Stage::Tweet { delay };
                }
                return;
            }
            let site = world
                .get::<NpcActions>(actor)
                .and_then(|actions| residency::site_type_value(&actions.site_type));
            world.write_message(crate::npc_state::TweetHudEvent::Hide {
                npc: actor,
                unit,
                site_type: site,
            });
            info!("[npc-change-site] unit={unit} frame={frame} HideTweet (room-entry tweet)");
            on_entry_site_end(world, actor, unit, frame);
        }
        Stage::SetupDone => {
            // TryWaitForFirstVisitRoom saw the site change end:
            // ForceUpdateObjective, and the objective ends.
            force_update_objective(world, actor, frame);
            info!("[npc-change-site] unit={unit} frame={frame} change-site objective ends: ForceUpdateObjective");
            end_run(world, actor);
        }
    }
}

/// Starts an awaited entry turn: the look-at tween towards `towards` over
/// `duration`, or over the rotate time (the angle from the character's
/// forward to `towards - position`, / 60 s) when `None`, eased out-quart.
/// `false` when the target point is at the world origin (no turn).
fn begin_turn(
    world: &mut World,
    actor: Entity,
    unit: u32,
    towards: Vec3,
    duration: Option<f32>,
    frame: u32,
) -> bool {
    if towards.length_squared() < LOOK_AT_ZERO_BOUND {
        return false;
    }
    let position = world
        .get::<Transform>(actor)
        .map(|transform| transform.translation)
        .unwrap_or(Vec3::ZERO);
    let forward = world
        .get::<WalkState>(actor)
        .map(|walk| walk.0.forward)
        .unwrap_or([0.0, 0.0, 1.0]);
    let duration = duration.unwrap_or_else(|| {
        moly_law::path::rotate_time(moly_law::path::angle_between(
            forward,
            (towards - position).to_array(),
        ))
    });
    world
        .entity_mut(actor)
        .insert(crate::npc_look_at::NpcLookAt::new(
            towards,
            forward,
            duration,
            moly_law::ui::dotween::Ease::OutQuart,
            frame,
        ));
    info!(
        "[npc-change-site] unit={unit} frame={frame} DoLookAtAsync(({:.3},{:.3},{:.3}), {duration} s, out-quart), awaited",
        towards.x, towards.y, towards.z
    );
    true
}

/// HomeSiteEnter after its turn: `HouseView.SetAnimationTrigger("NPCOn")`
/// on the house (`FixtureManager.GetHouseView`), then the clip and the show
/// one frame after it starts.
fn home_clip(world: &mut World, actor: Entity, unit: u32, frame: u32) {
    use crate::fixture_gimmick::house_door::{self, HouseLookup, HouseTrigger};
    match house_door::find_house(world) {
        HouseLookup::Found(house) => match house_door::set_trigger(world, &house, HouseTrigger::NpcOn) {
            Ok(()) => info!("[npc-change-site] unit={unit} frame={frame} house SetAnimationTrigger(NPCOn)"),
            Err(reason) => warn!("[npc-change-site] unit={unit} frame={frame} house SetAnimationTrigger(NPCOn) refused: {reason}"),
        },
        HouseLookup::Absent => info!("[npc-change-site] unit={unit} frame={frame} house SetAnimationTrigger(NPCOn): no house placed"),
        HouseLookup::Pending(reason) | HouseLookup::Failed(reason) => {
            warn!("[npc-change-site] unit={unit} frame={frame} house SetAnimationTrigger(NPCOn) not set: {reason}")
        }
    }
    start_clip(world, actor, unit, HOME_ENTER_CLIP, frame, false);
}

/// FloorEnter after its turn: on the player's site the knock sound and the
/// door-open sound, the door opens, the character is shown and plays its
/// entry clip.
fn floor_knock(world: &mut World, actor: Entity, unit: u32, frame: u32) {
    let own = world
        .get::<NpcActions>(actor)
        .and_then(|actions| residency::site_type_value(&actions.site_type));
    if own == player_site(world) {
        push_se(world, "se_knock_door");
        push_se(world, "se_door_open");
        info!("[npc-change-site] unit={unit} frame={frame} se_knock_door (its end not awaited here), se_door_open");
    }
    crate::site_move::room_door::open(world);
    // The brightness fade is not played; SetTransparencyEnabled(true).
    if let Some(mut visibility) = world.get_mut::<Visibility>(actor) {
        *visibility = Visibility::Inherited;
    }
    start_clip(world, actor, unit, FLOOR_ENTER_CLIP, frame, true);
}

/// OnEntrySite's end: the facial reset (not played here) and the flag that
/// skips the next rest; then the state's setup ends.
fn on_entry_site_end(world: &mut World, actor: Entity, unit: u32, frame: u32) {
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
        mind.skip_next_rest = true;
    }
    info!("[npc-change-site] unit={unit} frame={frame} OnEntrySite done: ImmediatelyExecuteNextObjective");
    set_stage(world, actor, Stage::SetupDone, frame);
}

fn start_position(world: &World, actor: Entity) -> Vec3 {
    world
        .resource::<ChangeSiteRuns>()
        .start_positions
        .get(&actor)
        .copied()
        .unwrap_or(Vec3::ZERO)
}

fn player_position(world: &mut World) -> Option<Vec3> {
    let mut players = world.query_filtered::<&Transform, With<crate::player::PlayerControlled>>();
    players
        .iter(world)
        .next()
        .map(|transform| transform.translation)
}

/// `CanShowEntryReaction`: the game is neither editing the layout nor
/// levelling up the room.
///
/// The host's game state is Edit while the layout editor is active; it has
/// no room level-up state.
fn can_show_entry_reaction(world: &World) -> bool {
    let editing = world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active());
    law::can_show_entry_reaction(if editing { law::STATE_EDIT } else { 0 })
}

/// ForceSetPosition(entry point), UpdateCurrentSiteType(target) and
/// SetTransparencyEnabled(false).
fn place_on_target(world: &mut World, actor: Entity, target: i32, position: Vec3, _frame: u32) {
    let epoch = world
        .get_resource::<crate::site::GroundEpoch>()
        .map(|epoch| epoch.0)
        .unwrap_or_default();
    world.entity_mut(actor).remove::<Away>();
    crate::npc::force_set_position(world, actor, position.to_array());
    if let Some(mut visibility) = world.get_mut::<Visibility>(actor) {
        *visibility = Visibility::Hidden;
    }
    if let Some(mut actions) = world.get_mut::<NpcActions>(actor) {
        actions.site_type = residency::SITE_TYPES[target as usize].to_owned();
        actions.resume_on_site(epoch);
    }
    change_state(world, actor, NpcAction::ChangeSite);
}

fn push_se(world: &mut World, cue: &str) {
    if let Some(mut requests) = world.get_resource_mut::<crate::audio::SeRequests>() {
        requests.0.push(crate::audio::SeRequest {
            owner: None,
            cue: cue.to_owned(),
            class: crate::audio::SeClass::Ingame,
            source: "npc-change-site",
        });
    }
}

/// `PlayAnimation(clip, 1.0)` on the NPC's own animation player.
fn start_clip(world: &mut World, actor: Entity, unit: u32, clip: &str, frame: u32, shown: bool) {
    let mut params = SystemState::<(
        Res<crate::character::MotionLibrary>,
        Res<Assets<Gltf>>,
        ResMut<Assets<AnimationGraph>>,
        Query<&mut crate::character::MotionDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (library, gltfs, mut graphs, mut drivers, mut players) = params.get_mut(world);
    let node = (|| {
        let lib = gltfs.get(&library.gltf)?;
        let mut driver = drivers.get_mut(actor).ok()?;
        let mut nodes = HashMap::new();
        let node = crate::alone_action_runtime::node_for(
            &mut nodes,
            &mut graphs,
            lib,
            &driver.graph,
            clip,
            unit,
        );
        let (mut player, mut transitions) = players.get_mut(driver.player).ok()?;
        transitions
            .play(&mut player, node, crate::character::SEGMENT_BLEND)
            .set_speed(1.0);
        driver.playing = None;
        Some(node)
    })();
    match node {
        Some(node) => {
            world.entity_mut(actor).insert(ChangeSiteClip);
            info!("[npc-change-site] unit={unit} frame={frame} PlayAnimation({clip}, 1.0)");
            set_stage(world, actor, Stage::EntryClip { node, shown }, frame);
        }
        None => {
            warn!("[npc-change-site] unit={unit} frame={frame} PlayAnimation({clip}): the motion library or the NPC's player is not ready; the clip is skipped");
            if let Some(mut visibility) = world.get_mut::<Visibility>(actor) {
                *visibility = Visibility::Inherited;
            }
            set_stage(world, actor, Stage::SetupDone, frame);
        }
    }
}

fn clip_finished(world: &mut World, actor: Entity, node: AnimationNodeIndex) -> bool {
    let Some(player) = world
        .get::<crate::character::MotionDriver>(actor)
        .map(|driver| driver.player)
    else {
        return true;
    };
    world
        .get::<AnimationPlayer>(player)
        .and_then(|player| player.animation(node))
        .is_none_or(|animation| animation.is_finished())
}

/// The clip is over: back to idle, the locomotion driver takes the player.
fn end_clip(world: &mut World, actor: Entity, unit: u32) {
    world.entity_mut(actor).remove::<ChangeSiteClip>();
    let mut params = SystemState::<(
        Query<&mut crate::character::MotionDriver>,
        Query<&mut AnimationPlayer>,
        Query<&mut AnimationTransitions>,
    )>::new(world);
    let (mut drivers, mut players, mut transitions) = params.get_mut(world);
    if let Ok(mut driver) = drivers.get_mut(actor) {
        crate::alone_action_runtime::play_idle(&mut driver, &mut players, &mut transitions, unit);
    }
}

/// ShowTweet's first part: `CanShowEntryReaction` (`None` when it fails),
/// `Stop`, then `DoLookAtAsync(player position, GetRotateTime(presenter,
/// player position), out-quart)`, awaited: `Some(true)` while it turns,
/// `Some(false)` when there is no turn (no player, or a target at the world
/// origin).
fn begin_entry_tweet(world: &mut World, actor: Entity, unit: u32, frame: u32) -> Option<bool> {
    if !can_show_entry_reaction(world) {
        return None;
    }
    crate::npc::stop_for_external_activity(world, actor);
    let Some(player) = player_position(world) else {
        return Some(false);
    };
    Some(begin_turn(world, actor, unit, player, None, frame))
}

/// ShowTweet after its turn: the tweet, then the 3000 ms wait; OnEntrySite's
/// end when there is none.
fn entry_tweet(world: &mut World, actor: Entity, unit: u32, frame: u32) {
    match show_entry_tweet(world, actor, unit, frame) {
        Some(delay) => set_stage(world, actor, Stage::Tweet { delay }, frame),
        None => on_entry_site_end(world, actor, unit, frame),
    }
}

/// ShowTweet's pick and show. Returns the display delay when a tweet is
/// shown.
fn show_entry_tweet(
    world: &mut World,
    actor: Entity,
    unit: u32,
    frame: u32,
) -> Option<moly_law::objective::DelayPromise> {
    let tables = world.get_resource::<crate::npc_tweet::TweetTables>()?;
    let entries = &tables.site_entries;
    let rows: Vec<(i32, i32)> = tables
        .wrt
        .iter()
        .filter(|row| row.game_character_unit_id == unit as i32 && entries.contains(&row.id))
        .map(|row| (row.id, row.tweet_id))
        .collect();
    if rows.is_empty() {
        info!("[npc-change-site] unit={unit} frame={frame} ShowTweet: no site-entry tweet of this character");
        return None;
    }
    let len = rows.len();
    let index = {
        let mut rng = world.get_mut::<crate::npc_objective::MemberRng>(actor)?;
        crate::npc_objective::engine_int_draw(&mut rng, len)
    };
    let (row_id, tweet_id) = rows[index];
    let tables = world.get_resource::<crate::npc_tweet::TweetTables>()?;
    let Some(tweet) = tables.tweet(tweet_id) else {
        info!("[npc-change-site] unit={unit} frame={frame} ShowTweet: tweet {tweet_id} has no master row");
        return None;
    };
    let text = tweet.text.clone();
    let site = world
        .get::<NpcActions>(actor)
        .and_then(|actions| residency::site_type_value(&actions.site_type));
    world.write_message(crate::npc_state::TweetHudEvent::Show {
        npc: actor,
        unit,
        tweet_id,
        text,
        site_type: site,
    });
    info!(
        "[npc-change-site] unit={unit} frame={frame} ShowTweet: site-entry rows {len}, RandomPick engine Range(0,{len}) = {index} -> row {row_id}, tweet {tweet_id} shown for {} ms",
        law::ENTRY_TWEET_MILLISECONDS
    );
    moly_law::objective::DelayPromise::from_milliseconds(law::ENTRY_TWEET_MILLISECONDS).ok()
}

/// The runs end when the loaded site changes under them (the objective's
/// site-change handler cancels it; OnCancel changes to Idle).
pub(crate) fn cancel_on_site_change(world: &mut World, mut last: Local<Option<String>>) {
    let loaded = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone());
    if *last == loaded {
        return;
    }
    *last = loaded;
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let actors: Vec<(Entity, u32)> = world
        .resource::<ChangeSiteRuns>()
        .runs
        .iter()
        .map(|(actor, run)| (*actor, run.unit))
        .collect();
    for (actor, unit) in actors {
        if world.get_entity(actor).is_err() {
            world.resource_mut::<ChangeSiteRuns>().runs.remove(&actor);
            continue;
        }
        change_state(world, actor, NpcAction::Idle);
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(actor) {
            mind.cancelled = true;
            mind.executing = false;
            mind.yield_since = Some(frame);
        }
        if world.get::<ChangeSiteClip>(actor).is_some() {
            end_clip(world, actor, unit);
        }
        world.resource_mut::<ChangeSiteRuns>().runs.remove(&actor);
        info!("[npc-change-site] unit={unit} frame={frame} the loaded site changed: the change-site objective is cancelled (OnCancel: Idle)");
    }
}
