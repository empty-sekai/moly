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
//! Host shape. This host has no multiplay: the player is always solo and the
//! room owner. A tutorial run returns at once in the source (the door goes on
//! in its own frame); here the answer comes on the next frame, when this side
//! first sees the request.
//!
//! Not ported yet (named): the NPC's own action, which picks one unread
//! fixture-action talk and one no-talk fixture action of that character,
//! forces one of them as its next objective and places the cast near the
//! fixture's action points. Every try here reports failure without drawing,
//! so both passes run and nobody is forced; the selection, its draws and the
//! await's timing are the source's.

use bevy::prelude::*;
use moly_law::objective::random_fixture_action as law;

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

/// The home controller's draws on the engine's global generator (the shuffle
/// is its only draw while the NPC's own action is not ported).
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
        &crate::npc::CharacterUnitId,
        &crate::npc::NpcActions,
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
            // The NPC's own action is not ported (see the module notes).
            law::TryOutcome::Failed
        } else {
            law::TryOutcome::NoNpc
        }
    });
    info!(
        "[npc-door] ExecuteNPCRandomFixtureAction from {} at frame {frame}: visible home NPCs {candidates:?}, shuffle draws (bound, value) {draws:?} -> {ids:?}, target ceil(n*0.3f) = {target}, pass 1 {first}/{target}{}, tries (unit, NPC found) {tries:?}; the NPC's own action is not ported: each try reports failure",
        caller.word(),
        match second {
            Some(count) => format!(", pass 2 {count}/{}", target - first),
            None => String::new(),
        },
    );
    finish(world, caller, &mut state);
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
