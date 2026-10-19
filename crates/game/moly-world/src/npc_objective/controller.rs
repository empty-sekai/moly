//! The controller's machine, per character: its phase, its two state
//! objects, the transition budget of a frame and the Inactive recovery.
//!
//! Source shape (`NPCHFSMController`):
//! - A controller is constructed Inactive, with an empty upper-weight list
//!   and nothing else set (inserting the components is the constructor).
//! - `OnUpdate` first zeroes the transition budget of the frame, then
//!   advances the phase's object: the upper state (Upper), the lower state
//!   (Lower and Rest), or the recovery timer (Inactive).
//! - Every upper transition (`TransitionToUpper`, `TransitionToPlan`) adds
//!   one to the budget first; the third transition of a frame parks the
//!   character Inactive, and so does a transition whose state is missing.
//! - `UpdateInactivePhase` counts `Time.deltaTime` only once the character
//!   has started, is not display-only and its recovery is not suppressed,
//!   and at 3.0 s zeroes the budget and decides again.
//! - `Start()` sets the started flag, clears the suppression, zeroes the
//!   timer and decides at once; `Stop()` suppresses the recovery and parks
//!   the character without touching any timer; `ForceStopToInactive()` parks
//!   it and zeroes the timer with the recovery left as it is.
//! - `Stop`/`ForceStopToInactive` exit both state objects. The exits run no
//!   body yet: the bodies arrive with their own units.
//!
//! The machine shares nothing with the objective loop: `npc_objective::decide`
//! keeps driving every character, and these phases stay where the start and
//! stop calls leave them.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_law::npc_hfsm::timing::{ACTION_STATE_TALK, MAX_TRANSITIONS_PER_FRAME};
use moly_law::npc_hfsm::upper::{decide_upper, UpperChoice};
use moly_law::npc_hfsm::{
    lower_state_for_upper, AvatarMoveStatus, LowerState, NpcPhase, UpperResult, UpperState,
    UpperStateWeight, UpperType, INACTIVE_RECOVERY_SECONDS,
};

use super::MemberRng;

/// `HFSMPhase`: the phase the character's machine is in.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcHfsmPhase(pub NpcPhase);

impl Default for NpcHfsmPhase {
    fn default() -> Self {
        Self(NpcPhase::Inactive)
    }
}

/// `_transitionsThisFrame`: the upper transitions made on the frame the
/// budget last stamped.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NpcTransitionBudget {
    pub frame: u32,
    pub count: i32,
}

/// `_inactiveElapsedTime`: how long the character has been Inactive.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct NpcRecoveryTimer(pub f32);

/// `_hasStarted` and `_suppressRecovery`: every entry point sets the first
/// and clears the second; `Stop()` alone sets the second.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NpcHfsmFlags {
    pub has_started: bool,
    pub suppress_recovery: bool,
}

/// `_isDisplayOnly`: a character owned by another player runs no decisions
/// and keeps its machine Inactive.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NpcDisplayOnly;

/// `_upperState`, `_lowerState` and `_lowerStateFactoryOverride`.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
pub struct NpcHfsmActor {
    pub upper: Option<UpperState>,
    pub lower: Option<LowerState>,
    pub lower_override: Option<LowerState>,
}

/// `_upperStateWeights`: empty until a presenter pushes weights (only the
/// tutorial branch does).
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct NpcUpperWeights(pub Vec<UpperStateWeight>);

/// A character's whole machine as the controller constructs it: Inactive,
/// with empty weights and nothing started.
#[derive(Bundle, Debug, Clone, Default)]
pub struct NpcController {
    pub phase: NpcHfsmPhase,
    pub budget: NpcTransitionBudget,
    pub recovery: NpcRecoveryTimer,
    pub flags: NpcHfsmFlags,
    pub actor: NpcHfsmActor,
    pub weights: NpcUpperWeights,
}

/// An NPC entity as the roster creates it: the identity and view, the motion
/// state a spawn places, the model's generator, the action state, the talk
/// data and the controller's machine. A tuple stops being a bundle past 15
/// elements, so the spawns take this one bundle.
#[derive(Bundle)]
pub struct NpcSpawn {
    pub unit: crate::npc::CharacterUnitId,
    pub transform: Transform,
    pub visibility: Visibility,
    pub path: crate::npc::PathSlot,
    pub speed: crate::npc::WalkSpeed,
    pub pause: crate::npc::PauseSeconds,
    pub rest: crate::npc::RestLifecycle,
    pub actions: crate::npc::NpcActions,
    pub walk: crate::npc::WalkState,
    pub target: crate::npc::MoveTarget,
    pub route: crate::npc::RouteStops,
    pub stuck: crate::npc::StuckBaseline,
    pub clips: crate::npc::MotionClips,
    pub talk: crate::npc_objective::TalkSlot,
    pub mind: crate::npc_objective::ObjectiveMind,
    pub rng: MemberRng,
    pub motion: crate::npc::MotionPhase,
    pub controller: NpcController,
}

/// One NPC machine's components as the phase body advances them.
pub struct Machine<'a> {
    pub unit: u32,
    pub phase: &'a mut NpcHfsmPhase,
    pub budget: &'a mut NpcTransitionBudget,
    pub recovery: &'a mut NpcRecoveryTimer,
    pub flags: &'a mut NpcHfsmFlags,
    pub actor: &'a mut NpcHfsmActor,
    pub weights: &'a NpcUpperWeights,
    pub display_only: bool,
    /// The action model's current state (`NpcActions::current`).
    pub action_state: i32,
}

/// The presenter's `CreateUpperState`: the upper object a type produces.
/// The upper state bodies arrive with their own units, so no type produces
/// one yet.
fn upper_body(_state: UpperType) -> Option<UpperState> {
    None
}

/// `Controller生成 pos={0}`: the constructor's one line, once per
/// character.
pub fn log_constructed(unit: u32, position: [f32; 3]) {
    info!(
        "[npc-hfsm] unit={unit} Controller pos=({:.3},{:.3},{:.3})",
        position[0], position[1], position[2]
    );
}

/// Whether the per-decision, per-transition and per-recovery lines are
/// written: `MOLY_NPC_HFSM_TRACE=1`. They go through `info!` only under this
/// switch, because the game's log filter keeps `info` on and drops `debug` and
/// `trace`; without the switch the only lines are the constructor and the
/// report.
pub fn trace_enabled() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| {
            matches!(
                std::env::var("MOLY_NPC_HFSM_TRACE").as_deref(),
                Ok("1" | "true" | "yes" | "on")
            )
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        // A browser page has no environment; the switch is native only.
        false
    }
}

/// One machine line, written only under the trace switch: `debug!` when the
/// build's filter keeps its level, and the same text as an `info!` line when
/// only `info` gets through (the game's stick).
macro_rules! hfsm_trace {
    ($($arg:tt)*) => {
        if crate::npc_objective::controller::trace_enabled() {
            debug!($($arg)*);
            if !bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
                info!("{}", format!($($arg)*));
            }
        }
    };
}
use hfsm_trace;

/// One `OnUpdate` of one machine (`OnUpdate` of the controller).
fn advance(machine: &mut Machine<'_>, dt: f32, draw: &mut dyn FnMut(f32) -> f32) {
    let frame = machine.budget.frame;
    machine.budget.count = 0;
    if machine.action_state == ACTION_STATE_TALK {
        // The talk short-circuit: the machine does not advance. TryCancel is
        // asked only from the Upper phase, and it answers only in the Lower or
        // Rest phase, so its two phase tests never hold together and the
        // character is never cancelled this way.
        if machine.phase.0 == NpcPhase::Upper {
            try_cancel(machine.unit, machine.phase, machine.actor);
        }
        return;
    }
    match machine.phase.0 {
        NpcPhase::Upper => update_upper_phase(machine, draw),
        NpcPhase::Lower | NpcPhase::Rest => update_lower_phase(machine, draw),
        NpcPhase::Inactive => update_inactive_phase(machine, frame, dt, draw),
    }
}

/// `Start()`: started, the recovery enabled, the timer zeroed, a decision
/// with the machine's own weights.
pub fn start(machine: &mut Machine<'_>, draw: &mut dyn FnMut(f32) -> f32) {
    hfsm_trace!("[npc-hfsm] unit={} Start()", machine.unit);
    machine.flags.has_started = true;
    machine.flags.suppress_recovery = false;
    machine.recovery.0 = 0.0;
    decide_and_transition(machine, draw);
}

/// `SetUpperStateWeight`.
pub fn set_upper_weight(weights: &mut NpcUpperWeights, state: UpperType, weight: f32) {
    moly_law::npc_hfsm::set_upper_weight(&mut weights.0, state, weight);
}

/// `Stop()`: the recovery is suppressed, both objects are dropped, the
/// character is Inactive, and the Inactive timer keeps its value.
pub fn stop(unit: u32, phase: &mut NpcHfsmPhase, flags: &mut NpcHfsmFlags, actor: &mut NpcHfsmActor) {
    flags.suppress_recovery = true;
    exit_current_states(actor);
    *phase = NpcHfsmPhase(NpcPhase::Inactive);
    hfsm_trace!("[npc-hfsm] unit={unit} Stop()");
}

/// What the report says about one character's machine.
#[derive(Clone, Debug, PartialEq)]
pub struct NpcHfsmRow {
    pub unit: u32,
    pub phase: NpcPhase,
    pub upper: Option<UpperType>,
    pub lower: Option<moly_law::npc_hfsm::LowerType>,
    pub transitions: i32,
    pub inactive: f32,
    pub started: bool,
    pub suppressed: bool,
    pub display_only: bool,
}

/// The controller's report: the machines' state every two seconds, as one
/// `[npc-hfsm] report` line and the `npc_ai` view of `moly.observe`.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct NpcHfsmReport(pub Vec<NpcHfsmRow>);

impl NpcHfsmReport {
    /// The machine of `unit`.
    pub fn row(&self, unit: u32) -> Option<&NpcHfsmRow> {
        self.0.iter().find(|row| row.unit == unit)
    }

    /// Every machine as JSON, for the control surface.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.0
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "unit": row.unit,
                        "phase": phase_word(row.phase),
                        "upper": row.upper.map(|upper| format!("{upper:?}")),
                        "lower": row.lower.map(|lower| format!("{lower:?}")),
                        "transitions": row.transitions,
                        "inactive": row.inactive,
                        "started": row.started,
                        "suppressed": row.suppressed,
                        "display_only": row.display_only,
                    })
                })
                .collect(),
        )
    }
}

/// The phase's name for logs and the control surface.
pub fn phase_word(phase: NpcPhase) -> &'static str {
    match phase {
        NpcPhase::Upper => "Upper",
        NpcPhase::Lower => "Lower",
        NpcPhase::Rest => "Rest",
        NpcPhase::Inactive => "Inactive",
    }
}

impl NpcHfsmRow {
    fn of(
        unit: u32,
        phase: NpcHfsmPhase,
        budget: NpcTransitionBudget,
        recovery: NpcRecoveryTimer,
        flags: NpcHfsmFlags,
        actor: NpcHfsmActor,
        display_only: bool,
    ) -> Self {
        Self {
            unit,
            phase: phase.0,
            upper: actor.upper.as_ref().map(UpperState::upper_type),
            lower: actor.lower.as_ref().map(LowerState::lower_type),
            transitions: budget.count,
            inactive: recovery.0,
            started: flags.has_started,
            suppressed: flags.suppress_recovery,
            display_only,
        }
    }
}

/// Record the machines of every character present.
pub fn report_snapshot(
    mut report: ResMut<NpcHfsmReport>,
    npcs: Query<(
        &crate::npc::CharacterUnitId,
        &NpcHfsmPhase,
        &NpcTransitionBudget,
        &NpcRecoveryTimer,
        &NpcHfsmFlags,
        &NpcHfsmActor,
        Has<NpcDisplayOnly>,
    )>,
) {
    let mut rows: Vec<NpcHfsmRow> = npcs
        .iter()
        .map(|(unit, phase, budget, recovery, flags, actor, display_only)| {
            NpcHfsmRow::of(unit.0, *phase, *budget, *recovery, *flags, *actor, display_only)
        })
        .collect();
    rows.sort_by_key(|row| row.unit);
    if report.0 == rows {
        return;
    }
    if !rows.is_empty() {
        let line = rows
            .iter()
            .map(|row| {
                format!(
                    "{}:{}",
                    row.unit,
                    row.lower
                        .map(|lower| format!("{lower:?}"))
                        .or_else(|| row.upper.map(|upper| format!("{upper:?}")))
                        .unwrap_or_else(|| phase_word(row.phase).to_owned()),
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        // One summary per two seconds, while the machines change.
        info!(
            "[npc-hfsm] report: {} machines; inactive {}s*; unit=upper/lower {}",
            rows.len(),
            INACTIVE_RECOVERY_SECONDS,
            line
        );
    }
    report.0 = rows;
}

/// `IsStoppedIntentionally`.
pub fn is_stopped_intentionally(flags: &NpcHfsmFlags, phase: &NpcHfsmPhase) -> bool {
    flags.suppress_recovery && phase.0 == NpcPhase::Inactive
}

/// `ForceStopToInactive()`: both objects dropped, Inactive, the timer
/// zeroed; the recovery is left as it is.
pub fn force_stop_to_inactive(
    unit: u32,
    phase: &mut NpcHfsmPhase,
    recovery: &mut NpcRecoveryTimer,
    actor: &mut NpcHfsmActor,
) {
    let was = phase.0;
    exit_current_states(actor);
    *phase = NpcHfsmPhase(NpcPhase::Inactive);
    recovery.0 = 0.0;
    hfsm_trace!("[npc-hfsm] unit={unit} ForceStopToInactive() phase={}", was.value());
}

/// `ExitCurrentStates`: both objects are dropped; their exits run no body
/// yet.
fn exit_current_states(actor: &mut NpcHfsmActor) {
    actor.upper = None;
    actor.lower = None;
}

/// One entry into the transition budget: the transitions add one first and
/// refuse the third of a frame.
fn enter_transition(budget: &mut NpcTransitionBudget) -> bool {
    budget.count += 1;
    budget.count <= MAX_TRANSITIONS_PER_FRAME
}

/// `DecideAndTransitionToNext` with no decider plan: `DecideUpperState`,
/// then `TransitionToUpper`.
fn decide_and_transition(machine: &mut Machine<'_>, draw: &mut dyn FnMut(f32) -> f32) {
    let choice = decide_upper(&machine.weights.0, draw);
    match choice.draw {
        Some(drawn) => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: {:?} (rand={drawn:.2}/{:.1})",
            machine.unit, choice.state, choice.total
        ),
        None if machine.weights.0.is_empty() => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: weights empty -> RandomWalk",
            machine.unit
        ),
        None => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: totalWeight<=0 -> RandomWalk",
            machine.unit
        ),
    }
    transition_to_upper(machine, &choice);
}

/// `TransitionToUpper`: the budget pays first; past it, or with no upper
/// object, the character parks Inactive with the timer zeroed.
fn transition_to_upper(machine: &mut Machine<'_>, choice: &UpperChoice) {
    if !enter_transition(machine.budget) {
        hfsm_trace!(
            "[npc-hfsm] unit={} *** transition cap exceeded! transitions={} -> Inactive ***",
            machine.unit, machine.budget.count
        );
        park_inactive(machine);
        return;
    }
    let Some(upper) = upper_body(choice.state) else {
        hfsm_trace!(
            "[npc-hfsm] unit={} *** UpperStateFactory returned null for {:?} -> Inactive ***",
            machine.unit, choice.state
        );
        park_inactive(machine);
        return;
    };
    machine.actor.upper = Some(upper);
    machine.phase.0 = NpcPhase::Upper;
    hfsm_trace!(
        "[npc-hfsm] unit={} Upper start: {:?} (requested={:?}) transitions={}",
        machine.unit, choice.state, choice.state, machine.budget.count
    );
}

/// `_currentPhase = Inactive; _inactiveElapsedTime = 0` with no state left.
fn park_inactive(machine: &mut Machine<'_>) {
    machine.phase.0 = NpcPhase::Inactive;
    machine.recovery.0 = 0.0;
    machine.actor.upper = None;
    machine.actor.lower = None;
}

/// `UpdateUpperPhase`: a move still running returns; completion hands the
/// result to the lower layer.
fn update_upper_phase(machine: &mut Machine<'_>, draw: &mut dyn FnMut(f32) -> f32) {
    let Some(upper) = machine.actor.upper else {
        return;
    };
    hfsm_trace!(
        "[npc-hfsm] unit={} Upper done: {:?}",
        machine.unit,
        upper.upper_type()
    );
    let result = UpperResult {
        state: upper.upper_type(),
        arrival: [0.0; 3],
        target_fixture: None,
        target_npc: None,
        move_status: AvatarMoveStatus::Success,
    };
    complete_upper(machine, &result, draw);
}

/// `UpdateUpperPhase` after a result: a failed move clears the pending talk
/// data and rests; a success selects the lower state (the plan's override
/// first) and enters it.
fn complete_upper(machine: &mut Machine<'_>, result: &UpperResult, draw: &mut dyn FnMut(f32) -> f32) {
    machine.actor.upper = None;
    if result.is_move_failed() {
        hfsm_trace!(
            "[npc-hfsm] unit={} Upper failed: status={:?} -> rest before deciding",
            machine.unit,
            result.move_status
        );
        machine.actor.lower_override = None;
        enter_rest_or_decide_next(machine, draw);
        return;
    }
    let overridden = machine.actor.lower_override.take();
    let lower = overridden.unwrap_or_else(|| lower_state_for_upper(result));
    // The fixture result's talk-data factory picks among three fixture
    // lowers; it arrives with the plan factories, so the result's own
    // choice stands here.
    let kind = lower.lower_type();
    machine.actor.lower = Some(lower);
    machine.phase.0 = NpcPhase::Lower;
    hfsm_trace!(
        "[npc-hfsm] unit={} Lower start: {kind:?} (override={})",
        machine.unit,
        overridden.is_some()
    );
}

/// `UpdateLowerPhase` and `UpdateRestPhase`: the lower object advances and,
/// complete, leads back to the decision.
fn update_lower_phase(machine: &mut Machine<'_>, draw: &mut dyn FnMut(f32) -> f32) {
    let Some(lower) = machine.actor.lower else {
        return;
    };
    hfsm_trace!(
        "[npc-hfsm] unit={} Lower done: {:?}",
        machine.unit,
        lower.lower_type()
    );
    machine.actor.lower = None;
    enter_rest_or_decide_next(machine, draw);
}

/// `EnterRestOrDecideNext`: a character that only displays parks; with the
/// skip flag set (or, later, a talk reservation) it decides at once;
/// otherwise it rests for `max(PauseTime, 0)`. The rest's timer, its skip
/// flag and its lower object arrive with the state bodies.
fn enter_rest_or_decide_next(machine: &mut Machine<'_>, draw: &mut dyn FnMut(f32) -> f32) {
    if machine.display_only {
        park_inactive(machine);
        return;
    }
    machine.phase.0 = NpcPhase::Rest;
    hfsm_trace!("[npc-hfsm] unit={} Rest start", machine.unit);
    let _ = draw;
}

/// `UpdateInactivePhase`: the timer counts, and at 3.0 s the budget of the
/// frame is zeroed and the character decides again.
fn update_inactive_phase(machine: &mut Machine<'_>, frame: u32, dt: f32, draw: &mut dyn FnMut(f32) -> f32) {
    if machine.display_only || !machine.flags.has_started || machine.flags.suppress_recovery {
        return;
    }
    machine.recovery.0 += dt;
    if machine.recovery.0 >= INACTIVE_RECOVERY_SECONDS {
        hfsm_trace!(
            "[npc-hfsm] unit={} Inactive recovery: {:.1}s",
            machine.unit, machine.recovery.0
        );
        machine.budget.frame = frame;
        machine.budget.count = 0;
        // The transitions run here, between two frame resets of the budget;
        // their count stays on the frame until the next reset.
        decide_and_transition(machine, draw);
    }
}

/// `TryCancel`: in Lower or Rest, an interruptible lower is dropped with the
/// upper and the character parks Inactive.
fn try_cancel(unit: u32, phase: &mut NpcHfsmPhase, actor: &mut NpcHfsmActor) {
    if !phase.0.is_lower_running() {
        return;
    }
    if let Some(lower) = actor.lower {
        if !lower.flags().can_interrupt {
            hfsm_trace!(
                "[npc-hfsm] unit={unit} TryCancel refused: {:?} cannot be interrupted",
                lower.lower_type()
            );
            return;
        }
    }
    hfsm_trace!("[npc-hfsm] unit={unit} TryCancel: phase={}", phase.0.value());
    exit_current_states(actor);
    actor.lower_override = None;
    phase.0 = NpcPhase::Inactive;
}

/// `InitializeHFSM`'s `Start()` for every member of a new roster: the units
/// whose entities the spawn commands hold are started once they are
/// interleaved, on the frame they are first present.
pub fn start_roster(commands: &mut Commands, units: &[u32]) {
    if units.is_empty() {
        return;
    }
    let wanted: std::collections::HashSet<u32> = units.iter().copied().collect();
    commands.queue(move |world: &mut World| {
        let mut pending: Vec<(u32, Entity)> = world
            .query::<(Entity, &crate::npc::CharacterUnitId, &NpcHfsmFlags)>()
            .iter(world)
            .filter(|(_, unit, flags)| wanted.contains(&unit.0) && !flags.has_started)
            .map(|(entity, unit, _)| (unit.0, entity))
            .collect();
        pending.sort_unstable();
        for (_, entity) in pending {
            start_in_world(world, entity);
        }
    });
}

/// The controller's constructor for every character that has none: an
/// Inactive machine with empty weights, as `NPCHFSMController..ctor` leaves
/// it, and the constructor's one line.
pub fn insert_missing(
    mut commands: Commands,
    spawned: Query<
        (
            Entity,
            &crate::npc::CharacterUnitId,
            &crate::npc::WalkState,
            Option<&NpcHfsmPhase>,
        ),
        (With<MemberRng>, Without<NpcDisplayOnly>),
    >,
) {
    for (entity, unit, walk, phase) in &spawned {
        if phase.is_some() {
            continue;
        }
        log_constructed(unit.0, walk.0.position);
        commands.entity(entity).insert(NpcController::default());
    }
}

/// The AI's set-up of one character: its controller is constructed when it
/// has none (`InitializeHFSM` builds the controller, the presenter's
/// factories and the decider), then `Start()`. The weights are the empty
/// list; the state objects arrive with the state bodies.
pub fn start_in_world(world: &mut World, entity: Entity) {
    let Some(unit) = world.get::<crate::npc::CharacterUnitId>(entity).map(|unit| unit.0) else {
        return;
    };
    if world.get::<NpcHfsmPhase>(entity).is_none() {
        if let Some(walk) = world.get::<crate::npc::WalkState>(entity).map(|walk| walk.0.position) {
            log_constructed(unit, walk);
        }
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(NpcController::default());
        }
    }
    let weights = world
        .get::<NpcUpperWeights>(entity)
        .cloned()
        .unwrap_or_default();
    let display_only = world.get::<NpcDisplayOnly>(entity).is_some();
    let action_state = world
        .get::<crate::npc::NpcActions>(entity)
        .map_or(0, |actions| actions.current as i32);
    // The draw borrows the character's engine sequence; the machine's small
    // components are copied out and written back around it, and the draws
    // stay on the sequence for every later decision.
    let Some((mut phase, mut budget, mut recovery, mut flags, mut actor)) = machine_of(world, entity)
    else {
        return;
    };
    let Some(mut source) = world.get_entity_mut(entity).ok() else {
        return;
    };
    let mut rng = source
        .get::<MemberRng>()
        .map(MemberRng::clone)
        .unwrap_or_else(MemberRng::from_platform);
    {
        let mut machine = Machine {
            unit,
            phase: &mut phase,
            budget: &mut budget,
            recovery: &mut recovery,
            flags: &mut flags,
            actor: &mut actor,
            weights: &weights,
            display_only,
            action_state,
        };
        start(&mut machine, &mut |total| super::engine_float_draw(&mut rng, total));
    }
    let machine = NpcController { phase, budget, recovery, flags, actor, weights };
    source.insert((machine, rng));
}

/// One character's machine components, copied out.
type Parts = (NpcHfsmPhase, NpcTransitionBudget, NpcRecoveryTimer, NpcHfsmFlags, NpcHfsmActor);

fn machine_of(world: &World, entity: Entity) -> Option<Parts> {
    Some((
        *world.get::<NpcHfsmPhase>(entity)?,
        *world.get::<NpcTransitionBudget>(entity)?,
        *world.get::<NpcRecoveryTimer>(entity)?,
        *world.get::<NpcHfsmFlags>(entity)?,
        *world.get::<NpcHfsmActor>(entity)?,
    ))
}

/// `OnUpdate` for every character, in the avatar list's order.
pub fn on_update(
    frame: Res<FrameCount>,
    clock: Res<crate::npc_clock::NpcClock>,
    mut npcs: Query<(
        &crate::npc::CharacterUnitId,
        &mut MemberRng,
        &crate::npc::NpcActions,
        &mut NpcHfsmPhase,
        &mut NpcTransitionBudget,
        &mut NpcRecoveryTimer,
        &mut NpcHfsmFlags,
        &mut NpcHfsmActor,
        &NpcUpperWeights,
        Has<NpcDisplayOnly>,
    )>,
) {
    if !clock.ready() {
        return;
    }
    let frame = frame.0;
    let dt = clock.delta();
    for (unit, mut rng, actions, mut phase, mut budget, mut recovery, mut flags, mut actor, weights, display_only) in
        &mut npcs
    {
        budget.frame = frame;
        let mut machine = Machine {
            unit: unit.0,
            phase: &mut phase,
            budget: &mut budget,
            recovery: &mut recovery,
            flags: &mut flags,
            actor: &mut actor,
            weights: &weights,
            display_only,
            action_state: actions.current as i32,
        };
        advance(&mut machine, dt, &mut |total| super::engine_float_draw(&mut rng, total));
    }
}

/// The plugin's share: the report resource and its line every two seconds,
/// after the machine's update.
pub fn install(app: &mut App) {
    app.init_resource::<NpcHfsmReport>()
        .add_systems(
            Update,
            report_snapshot
                .after(on_update)
                .run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_secs(2))),
        );
    // The per-decision lines are written only through the trace switch (see
    // `trace_enabled`); one line at start-up says so.
    if trace_enabled() {
        info!("[npc-hfsm] trace on: per-decision and per-transition lines follow");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_line_and_phase_words() {
        // The constructor's line and the phase labels the report uses.
        assert_eq!(phase_word(NpcPhase::Upper), "Upper");
        assert_eq!(phase_word(NpcPhase::Lower), "Lower");
        assert_eq!(phase_word(NpcPhase::Rest), "Rest");
        assert_eq!(phase_word(NpcPhase::Inactive), "Inactive");
        assert!(NpcHfsmReport::default().0.is_empty());
    }

    fn entry(state: UpperType, weight: f32) -> UpperStateWeight {
        UpperStateWeight { state, weight }
    }

    /// One machine's components, as the insert leaves them.
    struct Rig {
        phase: NpcHfsmPhase,
        budget: NpcTransitionBudget,
        recovery: NpcRecoveryTimer,
        flags: NpcHfsmFlags,
        actor: NpcHfsmActor,
        weights: NpcUpperWeights,
        display_only: bool,
        action_state: i32,
        draws: Vec<f32>,
    }

    impl Default for Rig {
        fn default() -> Self {
            Self {
                phase: NpcHfsmPhase::default(),
                budget: NpcTransitionBudget::default(),
                recovery: NpcRecoveryTimer::default(),
                flags: NpcHfsmFlags::default(),
                actor: NpcHfsmActor::default(),
                weights: NpcUpperWeights::default(),
                display_only: false,
                action_state: 0,
                draws: Vec::new(),
            }
        }
    }

    impl Rig {
        fn started(self) -> Self {
            Self {
                flags: NpcHfsmFlags { has_started: true, suppress_recovery: false },
                ..self
            }
        }

        /// One frame; `fraction` of each draw's range is drawn.
        fn frame(&mut self, dt: f32, fraction: f32) {
            let mut drawn = Vec::new();
            let mut machine = self.machine();
            advance(&mut machine, dt, &mut |total| {
                drawn.push(total);
                total * fraction
            });
            self.draws.extend(drawn);
        }

        /// `Start()`; `fraction` of each draw's range is drawn.
        fn start(&mut self, fraction: f32) {
            let mut drawn = Vec::new();
            let mut machine = self.machine();
            super::start(&mut machine, &mut |total| {
                drawn.push(total);
                total * fraction
            });
            self.draws.extend(drawn);
        }

        fn machine(&mut self) -> Machine<'_> {
            Machine {
                unit: 1,
                phase: &mut self.phase,
                budget: &mut self.budget,
                recovery: &mut self.recovery,
                flags: &mut self.flags,
                actor: &mut self.actor,
                weights: &self.weights,
                display_only: self.display_only,
                action_state: self.action_state,
            }
        }
    }

    #[test]
    fn a_fresh_controller_is_inactive_and_empty() {
        let rig = Rig::default();
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert!(rig.weights.0.is_empty());
        assert!(!rig.flags.has_started);
        assert!(!rig.flags.suppress_recovery);
        assert_eq!(rig.recovery.0, 0.0);
        assert_eq!(rig.actor, NpcHfsmActor::default());
    }

    #[test]
    fn the_third_transition_of_a_frame_is_refused() {
        // TransitionToPlan / TransitionToUpper: count ++, then refused at 3.
        let mut budget = NpcTransitionBudget::default();
        assert!(enter_transition(&mut budget));
        assert!(enter_transition(&mut budget));
        assert!(!enter_transition(&mut budget));
        assert_eq!(budget.count, 3);
    }

    #[test]
    fn inactive_counts_and_decides_at_three_seconds() {
        // The timer counts by the frame's delta; the 3.0 s mark decides.
        let mut rig = Rig::default().started();
        rig.frame(1.0, 0.0);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        rig.frame(1.0, 0.0);
        assert_eq!(rig.recovery.0, 2.0);
        // Empty weights decide RandomWalk without a draw, and the missing
        // upper object parks the machine with the timer zeroed.
        rig.frame(1.0, 0.0);
        assert!(rig.draws.is_empty());
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert_eq!(rig.recovery.0, 0.0);
        // With no upper object, the machine recovers again three seconds
        // later.
        rig.frame(1.0, 0.0);
        rig.frame(1.0, 0.0);
        assert_eq!(rig.recovery.0, 2.0);
        rig.frame(1.0, 0.0);
        assert_eq!(rig.recovery.0, 0.0);
        // A started machine decides once per recovery, never on a frame its
        // upper state would run.
        assert!(!rig.flags.suppress_recovery);
    }

    #[test]
    fn not_started_or_suppressed_never_counts() {
        let rigs = [
            Rig { recovery: NpcRecoveryTimer(10.0), ..Rig::default() },
            Rig {
                recovery: NpcRecoveryTimer(10.0),
                flags: NpcHfsmFlags { has_started: true, suppress_recovery: true },
                ..Rig::default()
            },
            Rig { recovery: NpcRecoveryTimer(10.0), display_only: true, ..Rig::default().started() },
        ];
        for mut rig in rigs {
            rig.frame(1.0, 0.0);
            assert_eq!(rig.recovery.0, 10.0);
            assert_eq!(rig.phase.0, NpcPhase::Inactive);
        }
    }

    #[test]
    fn a_weighted_decision_draws_once() {
        let mut rig = Rig::default().started();
        rig.frame(1.0, 0.0);
        rig.frame(1.0, 0.0);
        rig.weights = NpcUpperWeights(vec![entry(UpperType::RandomWalk, 5.0), entry(UpperType::Rest, 2.0)]);
        rig.frame(1.0, 0.9);
        // The third idle second decides: one draw over the weight sum, then
        // the missing upper object parks the machine with the timer zeroed.
        assert_eq!(rig.draws, vec![7.0]);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert_eq!(rig.recovery.0, 0.0);
    }

    #[test]
    fn talk_freezes_a_lower_phase_and_parks_an_upper_one() {
        // OnUpdate: with Talk current the machine does not advance; TryCancel
        // is asked only in the Upper phase and answers only in Lower or Rest,
        // so those two tests never hold together. A Lower character therefore
        // keeps its phase and its state.
        let mut rig = Rig::default().started();
        rig.phase = NpcHfsmPhase(NpcPhase::Lower);
        rig.actor.lower = Some(LowerState::GeneralTalk);
        rig.recovery = NpcRecoveryTimer(1.0);
        rig.action_state = ACTION_STATE_TALK;
        rig.frame(1.0, 0.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::GeneralTalk));
        // The frame still resets the transition budget, and the frozen frame
        // advances nothing else (the Inactive timer is not reachable here).
        assert_eq!(rig.recovery.0, 1.0);
        assert_eq!(rig.budget.count, 0);

        let mut rig = Rig::default().started();
        rig.action_state = ACTION_STATE_TALK;
        rig.frame(1.0, 0.0);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert!(!rig.flags.suppress_recovery);
    }

    #[test]
    fn stop_suppresses_and_keeps_the_timer() {
        let mut rig = Rig::default().started();
        rig.phase = NpcHfsmPhase(NpcPhase::Lower);
        rig.recovery = NpcRecoveryTimer(1.5);
        super::stop(1, &mut rig.phase, &mut rig.flags, &mut rig.actor);
        assert!(rig.flags.suppress_recovery);
        assert!(is_stopped_intentionally(&rig.flags, &rig.phase));
        assert_eq!(rig.recovery.0, 1.5);
        rig.frame(1.0, 0.0);
        assert_eq!(rig.recovery.0, 1.5);
        // ForceStopToInactive leaves the recovery enabled and zeroes the
        // timer.
        super::force_stop_to_inactive(1, &mut rig.phase, &mut rig.recovery, &mut rig.actor);
        assert_eq!(rig.recovery.0, 0.0);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
    }

    #[test]
    fn the_report_row_reads_the_machine() {
        let row = NpcHfsmRow::of(
            7,
            NpcHfsmPhase(NpcPhase::Lower),
            NpcTransitionBudget { frame: 3, count: 1 },
            NpcRecoveryTimer(0.5),
            NpcHfsmFlags { has_started: true, suppress_recovery: false },
            NpcHfsmActor { lower: Some(LowerState::Idle { duration: 6.0 }), ..Default::default() },
            false,
        );
        assert_eq!(row.phase, NpcPhase::Lower);
        assert_eq!(row.lower, Some(LowerState::Idle { duration: 6.0 }.lower_type()));
        assert_eq!(row.upper, None);
        assert!(row.started);
        assert_eq!(phase_word(NpcPhase::Rest), "Rest");
    }

    #[test]
    fn start_sets_the_flags_and_decides_without_weights() {
        let mut rig = Rig { recovery: NpcRecoveryTimer(2.0), ..Rig::default() };
        rig.start(0.0);
        assert!(rig.flags.has_started);
        assert!(!rig.flags.suppress_recovery);
        assert!(rig.draws.is_empty());
        // The empty weight list answers RandomWalk without a draw; no upper
        // object exists yet, so the start parks the machine with its timer
        // zeroed (the park path, not the start, zeroes it).
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert_eq!(rig.recovery.0, 0.0);
        assert!(rig.budget.count > 0);
    }

    #[test]
    fn set_upper_weight_replaces_in_place() {
        let mut weights = NpcUpperWeights::default();
        set_upper_weight(&mut weights, UpperType::RandomWalk, 5.0);
        set_upper_weight(&mut weights, UpperType::MoveToFixture, 5.0);
        set_upper_weight(&mut weights, UpperType::Rest, 2.0);
        set_upper_weight(&mut weights, UpperType::RandomWalk, 1.0);
        assert_eq!(
            weights.0,
            vec![
                entry(UpperType::RandomWalk, 1.0),
                entry(UpperType::MoveToFixture, 5.0),
                entry(UpperType::Rest, 2.0),
            ]
        );
    }
}
