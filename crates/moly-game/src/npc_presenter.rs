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
        Without<crate::player::PlayerControlled>,
    >,
) {
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
