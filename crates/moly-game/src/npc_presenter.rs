//! The presenter's per-frame greeting gate (its tenth call) and the greeting
//! it executes.
//!
//! Source shape (every early return in the machine order):
//! 1. the tutorial is running;
//! 2. the character is over-prioritised by a character communication;
//! 3. the action state is already Greeting;
//! 4. the current objective is a site change (no objective reads as none);
//! 5. the game state is a cut scene (the editor and a talk do not block);
//! 6. the first talk is complete;
//! 7. the presenter has updated for less than 5.0 s since initialization;
//! 8. the action state is not Idle, AutoMove, Walk or Rest;
//! 9. a birthday context is open;
//! then the state and the first-talk flag are read again, and the full 3-D
//! float distance between the player and the character must be strictly
//! below 1.5 m. The gate does not run on a frame on which the overlap cancel
//! (the eighth call) fired.
//!
//! The execution cancels the current objective (none means no greeting this
//! frame; every objective this host runs can be cancelled; a Rest's cancel
//! changes to Idle; a no-talk objective's cancel releases its fixture
//! session), resets the AI model twice, draws a general talk (one
//! engine integer range over the character's general talks), writes greeting
//! data with that talk, the talk's pre-action tweet id and the flag that
//! skips the next Rest, and stops the character. The cancelled objective ends
//! on the next frame and the loop yields one more, so the greeting objective
//! starts two frames after the gate.
//!
//! Named inputs: the tutorial flag comes from the server panel; no character
//! communication, cut scene executor or birthday context exists in this
//! host, so rows 2 and 9 read false and row 5 reads the host's one cut-scene
//! flag. Named gap: a cancelled no-talk objective runs a ForceUpdateObjective
//! in the source (a reset and a row-1 cascade whose data the greeting's own
//! reset discards); its draws are not made here. A general lottery that raises, or a drawn talk this host cannot
//! resolve, is a source exception inside the presenter update: it is logged
//! by name and this character makes no further greeting attempt (the source
//! would raise again on every later frame).

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_law::objective::{ObjectiveType, TalkType};
use moly_law::talk::select::LotteryWeights;
use moly_law::tweet as tweet_law;

use crate::client_config::{
    ClientConfigs, KEY_NPC_LOTTERY_TALK1_WIGHT, KEY_NPC_LOTTERY_TALK2_WIGHT,
    KEY_NPC_LOTTERY_TALK3_WIGHT, KEY_NPC_LOTTERY_TALK4_WIGHT,
};
use crate::npc::{
    CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, RestLifecycle, RouteStops,
    WalkState,
};
use crate::npc_objective::{AiTalkData, MemberRng, ObjectiveMind, TalkSlot};

/// No character communication runs in this host.
const OVER_PRIORITY_SOME_CHARACTER: bool = false;
/// No birthday context runs in this host.
const BIRTHDAY_CONTEXT_OPEN: bool = false;

fn halt(mind: &mut ObjectiveMind, unit: u32, reason: String) {
    error!("[npc-greeting] unit={unit}: {reason}; no further greeting for this character");
    mind.greeting_halt = Some(reason);
}

/// Presenter call 10, per character in the avatar list order.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn try_greeting(
    mut commands: Commands,
    frame: Res<FrameCount>,
    fixture_sessions: Option<Res<crate::npc_fixture_activity::NpcFixtureActivities>>,
    panel: Option<Res<crate::server_panel::ServerPanel>>,
    gate: Res<crate::alone_action_runtime::AloneExecutionGate>,
    catalog: crate::player_talk::TalkCatalog,
    talk_list: Option<Res<crate::server_panel::TalkDataStore>>,
    tables: Option<Res<crate::fixture_activity_data::FixtureActivityTables>>,
    configs: Option<Res<ClientConfigs>>,
    site: Option<Res<crate::site::SiteActive>>,
    players: Query<&Transform, With<crate::player::PlayerControlled>>,
    mut npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &Transform,
            &mut NpcActions,
            &mut ObjectiveMind,
            &mut TalkSlot,
            &mut MemberRng,
            &mut RouteStops,
            &mut PathSlot,
            &mut WalkState,
            &mut MotionPhase,
            &mut RestLifecycle,
        ),
        (
            Without<crate::player::PlayerControlled>,
            Without<crate::npc::residency::Away>,
        ),
    >,
) {
    // The avatar store's per-frame NPC update, of which this is call 10.
    if !crate::npc::residency::loaded_site_runs(site.as_deref()) {
        return;
    }
    let frame = frame.0;
    let (Some(panel), Some(talk_list), Some(tables), Some(configs)) = (
        panel.as_deref(),
        talk_list.as_deref(),
        tables.as_deref(),
        configs.as_deref(),
    ) else {
        return;
    };
    if !catalog.ready() {
        return;
    }
    let weights = LotteryWeights {
        talk1: configs.float(KEY_NPC_LOTTERY_TALK1_WIGHT),
        talk2: configs.float(KEY_NPC_LOTTERY_TALK2_WIGHT),
        talk3: configs.float(KEY_NPC_LOTTERY_TALK3_WIGHT),
        talk4: configs.float(KEY_NPC_LOTTERY_TALK4_WIGHT),
    };
    let player = players.single().ok().map(|transform| transform.translation);
    for (
        entity,
        unit,
        transform,
        mut actions,
        mut mind,
        mut slot,
        mut rng,
        mut route,
        mut path,
        mut walk,
        mut phase,
        mut rest,
    ) in &mut npcs
    {
        let unit = unit.0;
        if !actions.ready() || mind.ai_stopped.is_some() || mind.greeting_halt.is_some() {
            continue;
        }
        if mind.overlap_cancel_frame == Some(frame) {
            continue;
        }
        let greeting_state = |actions: &NpcActions| actions.current == NpcAction::Greeting;
        if panel.is_tutorial()
            || OVER_PRIORITY_SOME_CHARACTER
            || greeting_state(&actions)
            || mind.current == Some(ObjectiveType::ChangeSite)
            || gate.cutscene_active
            || actions.first_talk_complete
            || actions.since_initialized < tweet_law::GREETING_AFTER_INIT_SECONDS
            || !matches!(
                actions.current,
                NpcAction::Idle | NpcAction::AutoMove | NpcAction::Walk | NpcAction::Rest
            )
            || BIRTHDAY_CONTEXT_OPEN
        {
            continue;
        }
        if greeting_state(&actions) || actions.first_talk_complete {
            continue;
        }
        // The gate reads the player presenter before the distance; the host
        // without a player cannot open it.
        let Some(player) = player else {
            continue;
        };
        let npc = transform.translation;
        let (dx, dy, dz) = (player.x - npc.x, player.y - npc.y, player.z - npc.z);
        let distance = ((dx * dx + dy * dy) + dz * dz).sqrt();
        if !(distance < tweet_law::GREETING_TRIGGER_DISTANCE) {
            continue;
        }
        // ExecuteGreeting. The current objective must be cancellable and
        // still running; an objective that already ended this frame is
        // disposed and cancels nothing.
        let Some(current) = mind.current else {
            continue;
        };
        if mind.yield_since.is_some() || mind.force_updates > 0 {
            continue;
        }
        if mind.rest.is_some() {
            // The Rest objective's cancel changes to Idle.
            mind.rest = None;
            actions.change(NpcAction::Idle, &mut rest);
        } else if current == ObjectiveType::NoneTalk && mind.executing {
            // The no-talk objective's cancel runs ForceUpdateObjective, whose
            // data the greeting's own reset discards (its draws are not made
            // here); its dispose releases the fixture it walks to.
            if fixture_sessions
                .as_deref()
                .is_some_and(|sessions| crate::npc_fixture_activity::owns_actor(sessions, entity))
            {
                commands.queue(move |world: &mut World| {
                    crate::npc_fixture_activity::cancel_for_greeting(world, entity);
                });
            }
            warn!(
                "[npc-greeting] unit={unit}: the cancelled no-talk objective's ForceUpdateObjective cascade is not drawn"
            );
        }
        mind.executing = false;
        mind.body = None;
        // The cancelled objective ends on the next frame; the loop then
        // yields once more before TryRest.
        mind.yield_since = Some(frame.wrapping_add(1));
        // Reset, then UpdateGreetingObjective's own reset.
        slot.reset_ai_talk_data();
        slot.reset_ai_talk_data();
        let seeker = crate::npc_talk_lottery::NpcView {
            unit,
            site_type: catalog.site_type_value(&actions.site_type),
            previous_talk_id: slot.previous_id().unwrap_or(0),
            talk_type: slot.kind(),
            objective: mind.current,
            state: actions.current as u8,
            since_initialized: actions.since_initialized,
        };
        let calls_before = rng.calls();
        let site_type_of_site = |site: i32| catalog.site_type_of_site(site);
        let scene = crate::npc_talk_lottery::LotteryScene {
            tables,
            talk_list: talk_list.talk_list(),
            phenomena_id: catalog.phenomena_id(),
            site_type_of_site: &site_type_of_site,
            npcs: &[],
            fixtures: &[],
            weights,
            fixture_gates: None,
            fixture_host: None,
            together: None,
        };
        let (pick, record) = {
            let engine = std::cell::RefCell::new(&mut *rng);
            let mut engine_int = |len: usize| {
                let mut guard = engine.borrow_mut();
                crate::npc_objective::engine_int_draw(&mut **guard, len)
            };
            let mut engine_float = |_total: f32| -> f32 {
                unreachable!("the general lottery draws no float range")
            };
            let mut sequence_pick = |_count: usize| -> Option<usize> {
                unreachable!("the general lottery makes no sequence pick")
            };
            let mut draws = crate::npc_talk_lottery::Draws {
                engine_int: &mut engine_int,
                engine_float: &mut engine_float,
                sequence_pick: &mut sequence_pick,
                record: Vec::new(),
            };
            let pick = crate::npc_talk_lottery::lottery_general_talk_id(
                &scene,
                &seeker,
                &mut draws,
                "greeting_general_talk_pick",
            );
            (pick, std::mem::take(&mut draws.record))
        };
        let calls = rng.calls() - calls_before;
        let talk_id = match pick {
            Ok(pick) => pick.talk_id(),
            Err(crate::npc_talk_lottery::Halt::Fault(reason))
            | Err(crate::npc_talk_lottery::Halt::Gap(reason)) => {
                halt(&mut mind, unit, format!("greeting general lottery: {reason}"));
                continue;
            }
        };
        if talk_id == 0 {
            halt(
                &mut mind,
                unit,
                "greeting data is null (general talk id 0); the presenter update dereferences it"
                    .into(),
            );
            continue;
        }
        let Some(resolved) = catalog.resolve(talk_id) else {
            halt(
                &mut mind,
                unit,
                format!("general talk {talk_id} is not in the loaded talk scripts"),
            );
            continue;
        };
        let pre_action = resolved.pre_action();
        actions.tweet_id = pre_action.id;
        slot.set_current(AiTalkData {
            kind: TalkType::Greeting,
            content: Some(resolved.content()),
            target_fixture: None,
            target_position: npc.to_array(),
            main_character: unit,
            characters: vec![unit],
            pre_action: Some(pre_action),
            locate: None,
            pending_factory: None,
        });
        mind.skip_next_rest = true;
        crate::npc::stop_agent(&mut route, &mut path, &mut walk, &mut phase);
        info!(
            "[npc-greeting] {}",
            serde_json::json!({
                "unit": unit,
                "frame": frame,
                "cancelled": format!("{current:?}"),
                "distance": distance,
                "since_initialized": actions.since_initialized,
                "talk_id": talk_id,
                "tweet_id": actions.tweet_id,
                "phenomenon": catalog.phenomena_id(),
                "draws": record,
                "rng_calls": calls,
            })
        );
    }
}

/// The presenter update's guard, read by every per-frame call: the
/// presenter is initialized (this host's ready actions) and its model is
/// visible (the character body is shown). A disposed presenter is not in
/// this host (its NPC entity is removed).
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct PresenterGate<'w, 's> {
    visible: Query<
        'w,
        's,
        (&'static CharacterUnitId, &'static InheritedVisibility),
        With<crate::character::MotionDriver>,
    >,
    npcs: Query<
        'w,
        's,
        (&'static CharacterUnitId, &'static NpcActions),
        Without<crate::npc::residency::Away>,
    >,
    site: Option<Res<'w, crate::site::SiteActive>>,
}

impl PresenterGate<'_, '_> {
    /// The avatar store runs the presenter's per-frame update for every NPC
    /// only while the current site is home or a floor; a suspended member
    /// has no presenter frame here (see `npc::residency`).
    pub(crate) fn runs(&self, entity: Entity) -> bool {
        if !crate::npc::residency::loaded_site_runs(self.site.as_deref()) {
            return false;
        }
        let Ok((unit, actions)) = self.npcs.get(entity) else {
            return false;
        };
        actions.ready()
            && self
                .visible
                .iter()
                .any(|(id, visibility)| id.0 == unit.0 && visibility.get())
    }
}

/// Presenter call 4: `NavMeshAgent.obstacleAvoidanceType = 0`
/// (NoObstacleAvoidance), every frame, on every NPC. This host's agent has no
/// obstacle avoidance to switch; the write leaves it as it is.
pub(crate) fn write_avoidance() {}

/// Presenter call 6: `UpdateIK` (the free look-on timers and the IK target
/// choice). Not in this host yet; until then no IK target is chosen.
pub(crate) fn update_ik() {}

/// Presenter call 7: `TryResetNavMesh`. When the agent's destination x is
/// infinite (an agent that is not bound to the navigation mesh, which is the
/// case while its local fit has it disabled) it calls the view's
/// `SetNavMeshAgentActive(false)` then `(true)`: angular speed 0 then 256,
/// acceleration 0 then 12, and a stop and restart that act only on an agent
/// on the mesh. It never enables, warps or re-places the agent. This host's
/// agent has neither angular speed nor acceleration, so the call changes
/// nothing observable here.
pub(crate) fn try_reset_navmesh() {}

/// Presenter call 9: `TrySomeCharacterCommunication`. Not in this host; until
/// then it never starts a communication (the source's result 1 would end the
/// update before the greeting gate).
pub(crate) fn try_some_character_communication() {}

/// Presenter call 8: `TryCancelIfCharacterOverlap`, per NPC in the avatar
/// list order. The timer rule is [`moly_law::objective::overlap::overlap_frame`];
/// the overlap is another NPC's position within the character overlap
/// distance (3-D, strict); the cancel is the presenter's
/// `TryCancelCurrentObjective` (refused while talking and without a running
/// objective), and a cancel that reports true sets the flag that skips the
/// next Rest and resets the timer. A true result ends this NPC's update
/// before calls 9 and 10. The cancelled objective ends on the next frame and
/// the loop yields one more before its TryRest.
pub(crate) fn try_cancel_if_overlap(world: &mut World) {
    use crate::client_config::{KEY_CHARACTER_OVERLAP_DISTANCE, KEY_CHARACTER_OVERLAP_TIME};
    let frame = world.resource::<FrameCount>().0;
    let dt = world.resource::<crate::npc_clock::NpcClock>().delta();
    // The avatar store's per-frame NPC update, of which this is call 8.
    if !crate::npc::residency::loaded_site_runs(world.get_resource::<crate::site::SiteActive>()) {
        return;
    }
    let Some((limit, reach)) = world.get_resource::<ClientConfigs>().map(|configs| {
        (
            configs.float(KEY_CHARACTER_OVERLAP_TIME),
            configs.float(KEY_CHARACTER_OVERLAP_DISTANCE),
        )
    }) else {
        return;
    };
    let shown: Vec<u32> = world
        .query_filtered::<(&CharacterUnitId, &InheritedVisibility), With<crate::character::MotionDriver>>()
        .iter(world)
        .filter(|(_, visibility)| visibility.get())
        .map(|(unit, _)| unit.0)
        .collect();
    let snaps: Vec<(Entity, u32, Vec3)> = world
        .query_filtered::<(Entity, &CharacterUnitId, &Transform), (
            With<ObjectiveMind>,
            Without<crate::player::PlayerControlled>,
            Without<crate::npc::residency::Away>,
        )>()
        .iter(world)
        .map(|(entity, unit, transform)| (entity, unit.0, transform.translation))
        .collect();
    for &(entity, unit, position) in &snaps {
        let Some(actions) = world.get::<NpcActions>(entity) else {
            continue;
        };
        if !actions.ready() || !shown.contains(&unit) {
            continue;
        }
        let state = actions.current as u32;
        let Some(mind) = world.get::<ObjectiveMind>(entity) else {
            continue;
        };
        if mind.ai_stopped.is_some() {
            continue;
        }
        let current = mind.current;
        let kind = world.get::<TalkSlot>(entity).and_then(|slot| slot.kind());
        let group_talk = moly_law::objective::overlap::has_group_talk(current, kind);
        let overlaps = snaps.iter().any(|&(other, _, other_position)| {
            other != entity && position.distance(other_position) < reach
        });
        let mut elapsed = mind.overlap_seconds;
        let attempt = moly_law::objective::overlap::overlap_frame(
            &mut elapsed,
            dt,
            limit,
            overlaps,
            group_talk,
            state,
        );
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
            mind.overlap_seconds = elapsed;
        }
        if !attempt {
            continue;
        }
        let mut groups = world
            .remove_resource::<crate::npc_fixture_talk::FixtureTalkGroups>()
            .unwrap_or_default();
        let (cancelled, owed) =
            crate::npc_fixture_talk::try_cancel_current(world, &mut groups, entity, frame);
        world.insert_resource(groups);
        if !cancelled {
            // The timer stays at or past the limit; the next frame tries again.
            continue;
        }
        let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) else {
            continue;
        };
        if owed > 0 {
            crate::npc_objective::owe_force_updates(&mut mind, frame.wrapping_add(1), owed);
        } else {
            mind.yield_since = Some(frame.wrapping_add(1));
        }
        // SetImmediatelyExecuteNextObjective(true), after the cancel.
        mind.skip_next_rest = true;
        mind.overlap_seconds = 0.0;
        mind.overlap_cancel_frame = Some(frame);
        info!(
            "[npc unit={unit}] 角色重叠超过源时限，取消目标并立即重选 (frame {frame}, {owed} calls owed)"
        );
    }
}

/// Presenter call 5: `UpdateNpcDither` (see [`crate::npc_dither`]).
pub(crate) use crate::npc_dither::update_dither;
