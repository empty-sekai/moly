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
//! - `Stop`/`ForceStopToInactive` exit both state objects.
//!
//! The walks (`RandomWalk`, `MoveToFixture`, `MoveToPosition`, `MoveToNPC`,
//! `MoveToPlayer`, `ChangeSite`) and the lower fallback `Idle` (pause drawn
//! when it is built, completes at the pause) have their bodies here; the
//! other lowers' bodies and the decider arrive with their own units.
//!
//! A walk's mover is the host's route: `npc::depart_along_to` starts it on
//! the objective face's navigation, and each frame the controller polls it
//! and feeds the state. The random walk samples its targets on the objective
//! face ([`upper_walk::pick_target`]). A walk the controller has no route to
//! start (the moves whose plan factories are not ported) keeps its enter's
//! running flag and is settled by its mover's invalid-path branch on its first
//! update, at the character's place. The status the walk then reports is its
//! own: the random walk always answers `Success`; `MoveToPlayer`, `MoveToNPC`
//! and `ChangeSite` answer `NoneRoute`; `MoveToPosition` and `MoveToFixture`
//! answer `Success` when the character is already inside their arrival
//! distance. A missing agent or fixture fails the enter as `InValid`.
//!
//! The route components are taken out of the character for its frame and put
//! back after it: the host's walk writer (`npc::advance`) runs with its own
//! single-component parameters, so the controller holds the five components
//! in [`Navigator`] while a state's enter or poll changes them.
//!
//! The controller drives only the units named by `MOLY_NPC_HFSM_SCOPE`; every
//! other character stays with `npc_objective::decide`, which runs it exactly
//! as before.

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use moly_law::npc_hfsm::lower_idle::IdleLower;
use moly_law::npc_hfsm::plan::NpcPlan;
use moly_law::npc_hfsm::timing::{ACTION_STATE_TALK, AVOID_NEAR_MAX, MAX_TRANSITIONS_PER_FRAME};
use moly_law::npc_hfsm::upper::{
    decide_upper, ChangeSiteUpper, MoveToFixtureUpper, MoveToPositionUpper, MoveToTargetUpper,
    RandomWalkUpper, RestUpper, UpperChoice, WalkPoll, WalkStart,
};
use moly_law::npc_hfsm::upper_walk::{self, RandomWalkDraw};
use moly_law::npc_hfsm::{
    lower_state_for_upper, LowerState, NpcPhase, UpperResult, UpperState, UpperStateWeight,
    UpperType, INACTIVE_RECOVERY_SECONDS,
};

use super::{engine_float_draw, MemberRng};

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

/// The distance within which a character can be in `GetAvoidDirection`'s band
/// (`AVOID_NEAR_MAX`): the walk reads every character closer than this, and
/// the band test itself sits in the law.
const RANDOM_WALK_AVOID_RADIUS: f32 = AVOID_NEAR_MAX;

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

/// The upper state object the machine holds, built by the presenter's
/// `CreateUpperState`. It carries the base's entry fields (`IsComplete`,
/// `_elapsedTime`, the running flag) and the mover's own record; every frame
/// takes one [`UpperFrame`]: the face the walks run on and the frame's poll.
#[derive(Debug, Clone, PartialEq)]
pub enum UpperBody {
    /// `RestUpperState` / `StayUpperState`: no mover, no timer.
    Marker(RestUpper),
    /// `RandomWalkUpperState`, walking to its sampled target.
    RandomWalk(RandomWalkUpper),
    /// `MoveToFixtureUpperState`, walking to the fixture's action point.
    MoveToFixture(MoveToFixtureUpper),
    /// `MoveToPositionUpperState`.
    MoveToPosition(MoveToPositionUpper),
    /// `MoveToNPCUpperState` / `MoveToPlayerUpperState`.
    MoveToTarget(MoveToTargetUpper),
    /// `ChangeSiteUpperState`, walking to the door of the site being left.
    ChangeSite(ChangeSiteUpper),
}

/// One frame of the walk mover handed to the state body: the face the
/// character walks on, and the frame's poll of its route.
#[derive(Clone, Copy)]
pub struct UpperFrame<'a> {
    /// The objective face (`NPCAvatarPresenter`'s navigation surface); a
    /// character without one has no agent here.
    pub face: Option<&'a super::ObjectiveFace>,
    /// The frame's route poll. A walk the enter left unstarted still takes
    /// one, because its mover's invalid-path branch reports on the first
    /// update.
    pub poll: Option<WalkPoll>,
}

/// The move an upper state starts on its enter: the host's route as the
/// mover left it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperStart {
    /// What the host's mover answered when it started the walk.
    pub walk: WalkStart,
    /// `MoveToFixture` alone: the fixture's live position and the agent's
    /// stopping distance before the action point narrowed it.
    pub approach: Option<FixtureApproach>,
}

/// `MoveToFixture`'s mover data: the target fixture's `GlobalPosition` and
/// the agent's own stopping distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixtureApproach {
    pub fixture_position: Option<[f32; 3]>,
    pub stopping_before: f32,
}

impl UpperStart {
    /// A walk whose route the host started, completing at that distance.
    pub fn walking(stopping_distance: f32) -> Self {
        Self {
            walk: WalkStart::Running {
                stopping_distance,
            },
            approach: None,
        }
    }

    /// A walk the host has no route for: the state keeps `OnEnter`'s running
    /// flag and its mover reports the invalid path on the walk's first
    /// update, where the character stands.
    pub fn unstarted() -> Self {
        Self {
            walk: WalkStart::NoRoute,
            approach: None,
        }
    }
}

/// A walk's entry as the host left it, and the target it walks to (the
/// state's own target, for the mover to signal where it stands).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperWalk {
    pub start: UpperStart,
    pub target: Option<[f32; 3]>,
}

impl UpperWalk {
    /// No route started: the walk's target is still the sampled point, so
    /// `MoveToFixture` can measure the character against it and the random
    /// walk keeps the point it sampled.
    fn unstarted(target: [f32; 3]) -> Self {
        Self {
            start: UpperStart::unstarted(),
            target: Some(target),
        }
    }
}

impl UpperBody {
    /// `Enter()`: a fresh entry of `state`, whose mover's first answer is
    /// `start` and whose walk goes to `goal` (the mover's target; the
    /// character's own position for the states whose mover starts nothing).
    /// The enter runs no frame of its own.
    pub fn enter(
        state: UpperState,
        start: UpperStart,
        goal: Option<[f32; 3]>,
        position: [f32; 3],
    ) -> Self {
        let walk = start.walk;
        let goal = goal.unwrap_or(position);
        let body = match state {
            UpperState::Rest => UpperBody::Marker(RestUpper::rest()),
            UpperState::Stay => UpperBody::Marker(RestUpper::stay()),
            UpperState::RandomWalk => {
                UpperBody::RandomWalk(RandomWalkUpper::enter(goal, walk))
            }
            UpperState::MoveToPosition => {
                // `CreateUpperState(6)`: a move that does not tolerate a
                // missing route.
                UpperBody::MoveToPosition(MoveToPositionUpper::enter(goal, false, walk, position))
            }
            UpperState::ChangeSite => {
                // `ChangeSiteUpperState` walks to the door its host mover was
                // given; a walk that cannot run ends at the door it was
                // given.
                UpperBody::ChangeSite(ChangeSiteUpper::enter(goal, walk))
            }
            UpperState::MoveToNpc { npc } => {
                // No selected character (a zero target) builds the walk the
                // presenter's factory falls back to.
                if npc == 0 {
                    UpperBody::RandomWalk(RandomWalkUpper::enter(goal, walk))
                } else {
                    UpperBody::MoveToTarget(MoveToTargetUpper::npc(Some(goal), walk, position))
                }
            }
            UpperState::MoveToPlayer => {
                UpperBody::MoveToTarget(MoveToTargetUpper::player(Some(goal), walk, position))
            }
            UpperState::MoveToFixture { fixture, .. } => {
                // `CreateUpperState(1)`: the action point when the plan gives
                // one, else the fixture's own position.
                let approach = start.approach.unwrap_or(FixtureApproach {
                    fixture_position: None,
                    stopping_before: crate::npc::MOVE_GOAL_DISTANCE,
                });
                UpperBody::MoveToFixture(MoveToFixtureUpper::enter(
                    fixture,
                    Some(goal),
                    approach.fixture_position,
                    approach.stopping_before,
                    walk,
                    position,
                ))
            }
        };
        let mut body = body;
        if let UpperBody::Marker(state) = &mut body {
            state.enter();
        }
        body
    }

    /// The state object.
    pub fn state(&self) -> UpperState {
        match self {
            UpperBody::Marker(state) => state.state(),
            UpperBody::RandomWalk(_) => UpperState::RandomWalk,
            UpperBody::MoveToFixture(state) => UpperState::MoveToFixture {
                fixture: state.target_fixture,
                can_be_recruited: true,
            },
            UpperBody::MoveToPosition(_) => UpperState::MoveToPosition,
            UpperBody::MoveToTarget(state) => state.upper_state(),
            UpperBody::ChangeSite(_) => UpperState::ChangeSite,
        }
    }

    /// `Update()`: the frame's time and the mover's poll.
    pub fn update(&mut self, dt: f32, frame: &UpperFrame<'_>) {
        let poll = frame.poll;
        match self {
            UpperBody::Marker(state) => state.update(dt),
            UpperBody::RandomWalk(state) => {
                if let Some(poll) = poll {
                    state.update(dt, poll);
                }
            }
            UpperBody::MoveToFixture(state) => {
                if let Some(poll) = poll {
                    state.update(dt, poll);
                }
            }
            UpperBody::MoveToPosition(state) => {
                if let Some(poll) = poll {
                    state.update(dt, poll);
                }
            }
            UpperBody::MoveToTarget(state) => {
                if let Some(poll) = poll {
                    state.update(dt, poll);
                }
            }
            UpperBody::ChangeSite(state) => {
                if let Some(poll) = poll {
                    state.update(dt, poll);
                }
            }
        }
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        match self {
            UpperBody::Marker(state) => state.is_complete(),
            UpperBody::RandomWalk(state) => state.is_complete(),
            UpperBody::MoveToFixture(state) => state.is_complete(),
            UpperBody::MoveToPosition(state) => state.is_complete(),
            UpperBody::MoveToTarget(state) => state.is_complete(),
            UpperBody::ChangeSite(state) => state.is_complete(),
        }
    }

    /// The state's own `_isMoving`: its move still runs (false for the two
    /// markers).
    pub fn is_moving(&self) -> bool {
        match self {
            UpperBody::Marker(_) => false,
            UpperBody::RandomWalk(state) => state.moving(),
            UpperBody::MoveToFixture(state) => state.moving(),
            UpperBody::MoveToPosition(state) => state.moving(),
            UpperBody::MoveToTarget(state) => state.moving(),
            UpperBody::ChangeSite(state) => state.moving(),
        }
    }

    /// `GetResult()`.
    pub fn result(&self, position: [f32; 3]) -> UpperResult {
        match self {
            UpperBody::Marker(state) => state.result(position),
            UpperBody::RandomWalk(state) => state.result(),
            UpperBody::MoveToFixture(state) => state.result(),
            UpperBody::MoveToPosition(state) => state.result(),
            UpperBody::MoveToTarget(state) => state.result(None),
            UpperBody::ChangeSite(state) => state.result(),
        }
    }

    /// `OnExit()`: the agent's stopping distance to give back, when a
    /// fixture move narrowed it.
    pub fn exit(&mut self) -> Option<f32> {
        match self {
            UpperBody::Marker(state) => {
                state.exit();
                None
            }
            UpperBody::RandomWalk(state) => {
                state.exit();
                None
            }
            UpperBody::MoveToFixture(state) => state.exit(),
            UpperBody::MoveToPosition(state) => {
                state.exit();
                None
            }
            UpperBody::MoveToTarget(state) => {
                state.exit();
                None
            }
            UpperBody::ChangeSite(state) => {
                state.exit();
                None
            }
        }
    }
}

/// `NPCLowerStateBase`: `IsComplete` and `_elapsedTime`. `Enter()` zeroes the
/// clock; `Update()` adds one `Time.deltaTime` and then runs the body.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LowerClock {
    /// `IsComplete`.
    pub is_complete: bool,
    /// `_elapsedTime`.
    pub elapsed: f32,
}

impl LowerClock {
    /// `Update()`: one frame.
    pub fn update(&mut self, dt: f32) {
        self.elapsed += dt;
    }
}

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
    pub upper_body: NpcUpperBody,
    pub lower_clock: NpcLowerClock,
}

/// The upper state's object (`None` while the record holds no state).
#[derive(Component, Debug, Clone, PartialEq, Default)]
pub struct NpcUpperBody(pub Option<UpperBody>);

/// The lower state's entry clock (`None` while the record holds no state).
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct NpcLowerClock(pub Option<LowerClock>);

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
    /// The upper state's object, while the record holds a state.
    pub upper_body: &'a mut Option<UpperBody>,
    /// The lower state's entry clock, while the record holds a state.
    pub lower_clock: &'a mut Option<LowerClock>,
    /// The presenter's position (`NPCAvatarPresenter.Position`).
    pub position: [f32; 3],
    /// The presenter's action state (`ChangeStateTypeOnly`).
    pub actions: &'a mut crate::npc::NpcActions,
    /// The presenter's rest bookkeeping, for the rest state's enter.
    pub rest: &'a mut crate::npc::RestLifecycle,
    /// The character's `PauseTime` (the rest after an objective).
    pub pause: f32,
    /// The objective face the walks run on (`None` before the site's
    /// navigation exists: no agent here).
    pub face: Option<&'a super::ObjectiveFace>,
    /// The other presenters of the avatar list, for the random walk's
    /// avoidance direction.
    pub others: &'a [[f32; 3]],
    /// The character is held away from the loaded site: it walks nothing.
    pub away: bool,
    /// The character's route components, taken out for the frame: a walk
    /// state's `Enter` and `Exit` write them and the poll reads them.
    pub navigator: Option<Navigator>,
}

impl Machine<'_> {
    /// `agent.ResetPath()` (the presenter's `Stop` without the state write):
    /// the walk is over.
    fn stop_walk(&mut self) {
        if let Some(navigator) = self.navigator.as_mut() {
            navigator.reset();
        }
    }
}

/// The host's navigation agent of one character: the walk's route, the path
/// slot, the walk state, the stuck baseline and the motion phase.
pub struct Navigator {
    pub route: crate::npc::RouteStops,
    pub path: crate::npc::PathSlot,
    pub walk: crate::npc::WalkState,
    pub stuck: crate::npc::StuckBaseline,
    pub motion: crate::npc::MotionPhase,
}

impl Navigator {
    /// `agent.ResetPath()`: the walk is over.
    fn reset(&mut self) {
        crate::npc::stop_movement_components(
            &mut self.walk,
            &mut self.route,
            &mut self.path,
            &mut self.stuck,
            &mut self.motion,
        );
    }

    /// The frame's poll of the walk: the presenter's position, the distance
    /// the move still has to run and whether the route ended without
    /// arriving.
    fn poll(&self, position: [f32; 3], face: Option<&super::ObjectiveFace>) -> WalkPoll {
        let remaining = match (self.route.completion_target(), face) {
            (Some(target), Some(face)) => {
                let goal = self
                    .route
                    .goal_distance()
                    .unwrap_or(crate::npc::MOVE_GOAL_DISTANCE);
                crate::npc::remaining_distance(position, target, face.site_height(), goal)
            }
            // No completion record or no face: the host's own distance to
            // the route's goal stands for the agent's.
            _ => distance(
                self.walk.0.position,
                self.route.route_goal().unwrap_or(position),
            ),
        };
        WalkPoll {
            position,
            remaining,
            stopped: self.route.stopped_without_arriving(),
        }
    }
}

/// The five components of `entity` the walk writer runs on, copied out.
fn mover_parts(world: &World, entity: Entity) -> Option<Navigator> {
    Some(Navigator {
        route: world.get::<crate::npc::RouteStops>(entity)?.clone(),
        path: crate::npc::PathSlot(
            world.get::<crate::npc::PathSlot>(entity)?.0.clone(),
        ),
        walk: crate::npc::WalkState(world.get::<crate::npc::WalkState>(entity)?.0.clone()),
        stuck: *world.get::<crate::npc::StuckBaseline>(entity)?,
        motion: *world.get::<crate::npc::MotionPhase>(entity)?,
    })
}

/// The navigator back on the character.
fn put_mover(world: &mut World, entity: Entity, navigator: Navigator) {
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert((
            navigator.route,
            navigator.path,
            navigator.walk,
            navigator.stuck,
            navigator.motion,
        ));
    }
}

/// The walk a fresh object of `state` starts: its target sampled, its route
/// started on the objective face, and the entry the host's mover left (no
/// route when nothing could be started; the walk reports that on its first
/// update).
fn start_walk(machine: &mut Machine<'_>, state: UpperState, rng: &mut MemberRng) -> UpperWalk {
    if !state.has_mover() {
        return UpperWalk::unstarted(machine.position);
    }
    let Some(face) = machine.face else {
        return UpperWalk::unstarted(machine.position);
    };
    let Some(from) = machine.navigator.as_ref().map(|navigator| navigator.walk.0.position) else {
        return UpperWalk::unstarted(machine.position);
    };
    let unit = machine.unit;
    let position = machine.position;
    let others = machine.others;
    // The walk's target: the random walk samples it around the character,
    // every other walk goes to a point of its own (the plan factories build
    // those once they are ported).
    let target = match state {
        UpperState::RandomWalk => {
            let near = neighbours_within(from, others, RANDOM_WALK_AVOID_RADIUS);
            let mut draw = HostWalkDraw { rng };
            let target =
                upper_walk::pick_target(&mut draw, from, &near, &mut |point, tolerance| {
                    face.sample(point, tolerance)
                });
            hfsm_trace!(
                "[npc-hfsm] unit={unit} random_walk neighbours={} target=({:.3},{:.3},{:.3})",
                near.len(),
                target[0],
                target[1],
                target[2]
            );
            target
        }
        _ => position,
    };
    let polyline = face
        .field
        .generate_path([from[0], from[2]], [target[0], target[2]]);
    if polyline.is_empty() {
        return UpperWalk::unstarted(target);
    }
    let Some(navigator) = machine.navigator.as_mut() else {
        return UpperWalk::unstarted(target);
    };
    let goal = crate::npc::MOVE_GOAL_DISTANCE;
    let walk_face = face.walk_face();
    let phase = crate::npc::depart_along_to(
        &crate::npc::CharacterUnitId(unit),
        &mut navigator.walk.0,
        &mut navigator.path.0,
        &mut navigator.route,
        &walk_face,
        face,
        target,
        None,
        rng,
        polyline,
        goal,
    );
    match phase {
        Some(phase) => {
            navigator.motion = phase;
            hfsm_trace!(
                "[npc-hfsm] unit={unit} walk start: {:?} → ({:.3},{:.3},{:.3})",
                state.upper_type(),
                target[0],
                target[1],
                target[2]
            );
            UpperWalk {
                start: UpperStart::walking(goal),
                target: Some(target),
            }
        }
        None => UpperWalk::unstarted(target),
    }
}

/// The other characters that can be in the walk's avoidance band of
/// `position`: every presenter closer than `far`, except the ones standing on
/// the same point. The band itself (`GetAvoidDirection`'s squared distance
/// window) is the law's.
fn neighbours_within(position: [f32; 3], others: &[[f32; 3]], far: f32) -> Vec<[f32; 3]> {
    others
        .iter()
        .copied()
        .filter(|other| {
            let dx = position[0] - other[0];
            let dy = position[1] - other[1];
            let dz = position[2] - other[2];
            let squared = dx * dx + dy * dy + dz * dz;
            squared > 0.0 && squared < far * far
        })
        .collect()
}

/// The random walk's engine draws on the character's member generator.
struct HostWalkDraw<'a> {
    rng: &'a mut MemberRng,
}

impl RandomWalkDraw for HostWalkDraw<'_> {
    fn unit_circle(&mut self) -> [f32; 2] {
        // `Random.insideUnitCircle`: a direction of the unit disc, its
        // radius the square root of one draw's fraction.
        let angle = engine_float_draw(self.rng, 1.0) * std::f32::consts::TAU;
        let radius = engine_float_draw(self.rng, 1.0).sqrt();
        [radius * angle.cos(), radius * angle.sin()]
    }

    fn unit_sphere(&mut self) -> [f32; 3] {
        // `Random.insideUnitSphere`: a direction of the unit ball.
        let z = super::uni_draw(self.rng, -1.0, 1.0);
        let angle = engine_float_draw(self.rng, 1.0) * std::f32::consts::TAU;
        let radius = (1.0 - z * z).max(0.0).sqrt();
        [radius * angle.cos(), radius * angle.sin(), z]
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        super::uni_draw(self.rng, min, max)
    }
}

/// The distance between two points.
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let x = a[0] - b[0];
    let y = a[1] - b[1];
    let z = a[2] - b[2];
    (x * x + y * y + z * z).sqrt()
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

/// One `OnUpdate` of one machine (`OnUpdate` of the controller): the budget
/// of the frame is zeroed, then the phase's body runs.
fn advance(machine: &mut Machine<'_>, frame: u32, dt: f32, rng: &mut MemberRng) {
    machine.budget.frame = frame;
    machine.budget.count = 0;
    if machine.action_state == ACTION_STATE_TALK {
        // The talk short-circuit: the machine does not advance. TryCancel is
        // asked only from the Upper phase, and it answers only in the Lower or
        // Rest phase, so its two phase tests never hold together and the
        // character is never cancelled this way.
        if machine.phase.0 == NpcPhase::Upper {
            try_cancel(machine);
        }
        return;
    }
    match machine.phase.0 {
        NpcPhase::Upper => update_upper_phase(machine, dt, rng),
        NpcPhase::Lower | NpcPhase::Rest => update_lower_phase(machine, dt, rng),
        NpcPhase::Inactive => update_inactive_phase(machine, frame, dt, rng),
    }
}

/// `Start()`: started, the recovery enabled, the timer zeroed, a decision
/// with the machine's own weights.
pub fn start(machine: &mut Machine<'_>, rng: &mut MemberRng) {
    hfsm_trace!("[npc-hfsm] unit={} Start()", machine.unit);
    machine.flags.has_started = true;
    machine.flags.suppress_recovery = false;
    machine.recovery.0 = 0.0;
    decide_and_transition(machine, rng);
}

/// `SetUpperStateWeight`.
pub fn set_upper_weight(weights: &mut NpcUpperWeights, state: UpperType, weight: f32) {
    moly_law::npc_hfsm::set_upper_weight(&mut weights.0, state, weight);
}

/// `Stop()`: the recovery is suppressed, both objects are dropped, the
/// character is Inactive, and the Inactive timer keeps its value.
pub fn stop(
    unit: u32,
    phase: &mut NpcHfsmPhase,
    flags: &mut NpcHfsmFlags,
    actor: &mut NpcHfsmActor,
    upper_body: &mut NpcUpperBody,
) {
    flags.suppress_recovery = true;
    exit_upper_body(upper_body);
    exit_current_states(actor);
    *phase = NpcHfsmPhase(NpcPhase::Inactive);
    hfsm_trace!("[npc-hfsm] unit={unit} Stop()");
}

/// The upper state object's `Exit()`: the walk gives the agent's stopping
/// distance back.
fn exit_upper_body(upper_body: &mut NpcUpperBody) {
    if let Some(mut body) = upper_body.0.take() {
        body.exit();
    }
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
    upper_body: &mut NpcUpperBody,
) {
    let was = phase.0;
    exit_upper_body(upper_body);
    exit_current_states(actor);
    *phase = NpcHfsmPhase(NpcPhase::Inactive);
    recovery.0 = 0.0;
    hfsm_trace!(
        "[npc-hfsm] unit={unit} ForceStopToInactive() phase={}",
        was.value()
    );
}

/// `ExitCurrentStates`: both objects are dropped; the phase's own clock is
/// cleared where it is held (`park_inactive`, `try_cancel`).
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
fn decide_and_transition(machine: &mut Machine<'_>, rng: &mut MemberRng) {
    // `DecideUpperState`'s one draw is `Random.Range(0.0f, total)` on the
    // character's generator.
    let mut draw = |total: f32| super::engine_float_draw(rng, total);
    let choice = decide_upper(&machine.weights.0, &mut draw);
    match choice.draw {
        Some(drawn) => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: {:?} (rand={drawn:.2}/{:.1})",
            machine.unit, choice.state, choice.total
        ),
        None if machine.weights.0.is_empty() => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: weights empty → RandomWalk",
            machine.unit
        ),
        None => hfsm_trace!(
            "[npc-hfsm] unit={} DecideUpperState: totalWeight<=0 → RandomWalk",
            machine.unit
        ),
    }
    transition_to_upper(machine, &choice, rng);
}

/// The presenter's `CreateUpperState(type)` as far as the factory alone goes:
/// the record a type builds. The presenter's own fallbacks (a move without a
/// selectable fixture or character) are the state body's enter; the route
/// data a fixture or position move carries comes from its plan, which the
/// plan factories build once they are ported.
fn create_upper(state: UpperType) -> UpperState {
    match state {
        UpperType::RandomWalk => UpperState::RandomWalk,
        UpperType::MoveToFixture => UpperState::MoveToFixture {
            fixture: None,
            can_be_recruited: true,
        },
        UpperType::MoveToPlayer => UpperState::MoveToPlayer,
        UpperType::MoveToNPC => UpperState::MoveToNpc { npc: 0 },
        UpperType::Rest => UpperState::Rest,
        UpperType::ChangeSite => UpperState::ChangeSite,
        UpperType::MoveToPosition => UpperState::MoveToPosition,
        UpperType::Stay => UpperState::Stay,
    }
}

/// `TransitionToUpper`: the budget pays first; past it, or with no upper
/// object, the character parks Inactive with the timer zeroed.
fn transition_to_upper(machine: &mut Machine<'_>, choice: &UpperChoice, rng: &mut MemberRng) {
    if !enter_transition(machine.budget) {
        hfsm_trace!(
            "[npc-hfsm] unit={} *** transition cap exceeded! transitions={} → Inactive ***",
            machine.unit, machine.budget.count
        );
        park_inactive(machine);
        return;
    }
    let state = create_upper(choice.state);
    // The walk's mover: the route the host starts, or the enter's own answer
    // when there is nothing to start (an absent agent ends the walk in the
    // enter; a missing route is left for the walk's first update).
    let walk = start_walk(machine, state, rng);
    let body = UpperBody::enter(state, walk.start, walk.target, machine.position);
    machine.actor.upper = Some(state);
    *machine.upper_body = Some(body);
    machine.phase.0 = NpcPhase::Upper;
    hfsm_trace!(
        "[npc-hfsm] unit={} Upper start: {:?} (requested={:?}) transitions={} pos=({:.3},{:.3},{:.3})",
        machine.unit,
        state,
        choice.state,
        machine.budget.count,
        machine.position[0],
        machine.position[1],
        machine.position[2]
    );
}

/// `_currentPhase = Inactive; _inactiveElapsedTime = 0` with no state left.
fn park_inactive(machine: &mut Machine<'_>) {
    let mut body = machine.upper_body.take();
    if let Some(body) = body.as_mut() {
        body.exit();
    }
    machine.phase.0 = NpcPhase::Inactive;
    machine.recovery.0 = 0.0;
    machine.actor.upper = None;
    machine.actor.lower = None;
    *machine.lower_clock = None;
}

/// `UpdateUpperPhase`: the state advances; a move still running returns, and
/// completion hands the result to the lower layer. A phase with no state
/// object returns (the source does not recover from that here).
fn update_upper_phase(machine: &mut Machine<'_>, dt: f32, rng: &mut MemberRng) {
    if machine.actor.upper.is_none() || machine.upper_body.is_none() {
        return;
    }
    let poll = match (machine.navigator.as_ref(), machine.face) {
        (Some(navigator), face) => navigator.poll(machine.position, face),
        // No route components: there is no agent to poll, but the state's
        // own update still runs, as the base's does. A walk the enter left
        // unstarted is settled by its mover's invalid-path branch on this
        // update; a walk that had a route cannot be here (the host starts no
        // route without the components).
        (None, _) => WalkPoll {
            position: machine.position,
            remaining: f32::INFINITY,
            stopped: false,
        },
    };
    let frame = UpperFrame {
        face: machine.face,
        poll: Some(poll),
    };
    let mut marker = false;
    if let Some(body) = machine.upper_body.as_mut() {
        marker = matches!(body, UpperBody::Marker(_));
        body.update(dt, &frame);
    }
    let Some(body) = machine.upper_body.as_ref() else {
        return;
    };
    if !body.is_complete() {
        return;
    }
    let result = body.result(machine.position);
    hfsm_trace!(
        "[npc-hfsm] unit={} Upper done: {:?} result={:?} status={:?} pos=({:.3},{:.3},{:.3})",
        machine.unit,
        result.state,
        result.state,
        result.move_status,
        result.arrival[0],
        result.arrival[1],
        result.arrival[2]
    );
    match body {
        UpperBody::RandomWalk(state) => hfsm_trace!(
            "[npc-hfsm] unit={} random_walk target=({:.3},{:.3},{:.3}) status={:?}",
            machine.unit,
            state.arrival[0],
            state.arrival[1],
            state.arrival[2],
            result.move_status
        ),
        UpperBody::MoveToFixture(state) => hfsm_trace!(
            "[npc-hfsm] unit={} upper MoveToFixture target={:?} arrival=({:.3},{:.3},{:.3}) status={:?}",
            machine.unit,
            state.target_fixture,
            result.arrival[0],
            result.arrival[1],
            result.arrival[2],
            result.move_status
        ),
        UpperBody::MoveToPosition(_) => hfsm_trace!(
            "[npc-hfsm] unit={} upper MoveToPosition arrival=({:.3},{:.3},{:.3}) status={:?}",
            machine.unit,
            result.arrival[0],
            result.arrival[1],
            result.arrival[2],
            result.move_status
        ),
        UpperBody::MoveToTarget(state) => hfsm_trace!(
            "[npc-hfsm] unit={} upper {:?} arrival=({:.3},{:.3},{:.3}) status={:?}",
            machine.unit,
            state.upper_state(),
            result.arrival[0],
            result.arrival[1],
            result.arrival[2],
            result.move_status
        ),
        UpperBody::ChangeSite(state) => hfsm_trace!(
            "[npc-hfsm] unit={} upper ChangeSite door=({:.3},{:.3},{:.3}) arrival=({:.3},{:.3},{:.3}) status={:?}",
            machine.unit,
            state.door[0],
            state.door[1],
            state.door[2],
            result.arrival[0],
            result.arrival[1],
            result.arrival[2],
            result.move_status
        ),
        UpperBody::Marker(_) => {}
    }
    if !marker {
        machine.stop_walk();
    }
    if let Some(mut body) = machine.upper_body.take() {
        body.exit();
    }
    complete_upper(machine, &result, rng);
}

/// `UpdateUpperPhase` after a result: a failed move clears the pending talk
/// data and rests; a success selects the lower state (the plan's override
/// first) and enters it. `MoveStatus == Success` is the whole test: a running
/// walk never completes, so a completing walk always carries its mover's final
/// status.
fn complete_upper(machine: &mut Machine<'_>, result: &UpperResult, rng: &mut MemberRng) {
    machine.actor.upper = None;
    if result.is_move_failed() {
        hfsm_trace!(
            "[npc-hfsm] unit={} Upper failed: result={:?} status={:?} → rest before deciding",
            machine.unit,
            result.state,
            result.move_status
        );
        machine.actor.lower_override = None;
        enter_rest_or_decide_next(machine, rng);
        return;
    }
    let overridden = machine.actor.lower_override.take();
    let from_override = overridden.is_some();
    let lower = match overridden {
        Some(lower) => lower,
        // `CreateLowerState`: the presenter's factory. Its fallback is always
        // the idle state, whose pause is drawn on this construction.
        None => {
            let lower = lower_state_for_upper(result);
            if matches!(lower, LowerState::Idle { .. }) {
                // `IdleLowerState..ctor` draws one `UnityEngine.Random.Range(5.0f,
                // 8.0f)` before the base constructor; the presenter's factory
                // hands the drawn pause to the no-draw constructor.
                let idle =
                    IdleLower::with_draw(&mut |low, high| super::uni_draw(rng, low, high));
                hfsm_trace!(
                    "[npc-hfsm] unit={} idle duration={:.2}s drawn",
                    machine.unit,
                    idle.duration()
                );
                idle.state()
            } else {
                lower
            }
        }
    };
    let kind = lower.lower_type();
    machine.actor.lower = Some(lower);
    *machine.lower_clock = Some(LowerClock::default());
    machine.phase.0 = NpcPhase::Lower;
    if from_override {
        hfsm_trace!("[npc-hfsm] unit={} Lower generated (override): {kind:?}", machine.unit);
    } else {
        hfsm_trace!("[npc-hfsm] unit={} Lower generated: {kind:?}", machine.unit);
    }
    enter_lower(machine);
}

/// The lower state entered (`lower.Enter()`): the presenter stops or reports
/// the action state the state's own `OnEnter` asks for.
fn enter_lower(machine: &mut Machine<'_>) {
    let Some(lower) = machine.actor.lower else {
        return;
    };
    let (stop, action_state) = match lower {
        LowerState::Idle { duration } => {
            let mut idle = IdleLower::with_duration(duration);
            let enter = idle.enter();
            (enter.stop, enter.action_state)
        }
        // The other lowers' enters (and their action state writes through the
        // host's `npc_state` change) arrive with their bodies.
        _ => (false, None),
    };
    if stop {
        machine.stop_walk();
    }
    if let Some(state) = action_state {
        hfsm_trace!(
            "[npc-hfsm] unit={} ChangeStateTypeOnly({state})",
            machine.unit
        );
    }
}

/// `UpdateLowerPhase` and `UpdateRestPhase`: the lower object advances and,
/// complete, leads back to the decision. A phase with no lower object does
/// nothing.
fn update_lower_phase(machine: &mut Machine<'_>, dt: f32, rng: &mut MemberRng) {
    let Some(lower) = machine.actor.lower else {
        return;
    };
    let Some(clock) = machine.lower_clock.as_mut() else {
        return;
    };
    clock.update(dt);
    let pause = match lower {
        LowerState::Idle { duration } | LowerState::Rest { duration } => duration,
        // The other bodies' timers arrive with their units.
        _ => return,
    };
    if !clock.is_complete && clock.elapsed >= pause {
        clock.is_complete = true;
    }
    if !clock.is_complete {
        return;
    }
    hfsm_trace!(
        "[npc-hfsm] unit={} Lower done: {:?} elapsed={:.1}s",
        machine.unit,
        lower.lower_type(),
        clock.elapsed
    );
    machine.actor.lower = None;
    *machine.lower_clock = None;
    if machine.phase.0 == NpcPhase::Rest {
        // The rest phase goes straight back to the decision.
        decide_and_transition(machine, rng);
        return;
    }
    enter_rest_or_decide_next(machine, rng);
}

/// `EnterRestOrDecideNext`: a character that only displays parks; otherwise
/// it rests for `max(PauseTime, 0)`, and a zero pause decides at once.
fn enter_rest_or_decide_next(machine: &mut Machine<'_>, rng: &mut MemberRng) {
    if machine.display_only {
        park_inactive(machine);
        return;
    }
    let pause = machine.pause.max(0.0);
    if pause <= 0.0 {
        hfsm_trace!("[npc-hfsm] unit={} Rest start: duration=0, deciding at once", machine.unit);
        decide_and_transition(machine, rng);
        return;
    }
    machine.phase.0 = NpcPhase::Rest;
    machine.actor.lower = Some(LowerState::Rest { duration: pause });
    *machine.lower_clock = Some(LowerClock::default());
    // `RestLowerState.OnEnter`: `Stop()`.
    machine.stop_walk();
    hfsm_trace!("[npc-hfsm] unit={} Rest start: duration={pause:.1}s", machine.unit);
}

/// `TransitionToPlan`: the budget pays first; the plan's `CreateUpper`
/// answers the state (none parks the character), and the plan's override is
/// installed for the upper phase whether or not the factory uses it.
pub fn transition_to_plan(machine: &mut Machine<'_>, plan: &NpcPlan) {
    hfsm_trace!(
        "[npc-hfsm] unit={} StartWithPlan(hasPlan={}) phase={}",
        machine.unit,
        true,
        machine.phase.0.value()
    );
    if !enter_transition(machine.budget) {
        hfsm_trace!(
            "[npc-hfsm] unit={} *** transition cap exceeded! transitions={} → Inactive ***",
            machine.unit, machine.budget.count
        );
        park_inactive(machine);
        return;
    }
    let upper = plan.create_upper();
    machine.actor.lower_override = plan.create_lower_override();
    machine.actor.upper = Some(upper);
    if let Some(mut body) = machine.upper_body.take() {
        body.exit();
    }
    *machine.upper_body = Some(UpperBody::enter(
        upper,
        UpperStart::unstarted(),
        None,
        machine.position,
    ));
    machine.phase.0 = NpcPhase::Upper;
    hfsm_trace!(
        "[npc-hfsm] unit={} Upper start (plan): {:?} transitions={} pos=({:.3},{:.3},{:.3})",
        machine.unit,
        upper.upper_type(),
        machine.budget.count,
        machine.position[0],
        machine.position[1],
        machine.position[2]
    );
}

/// `UpdateInactivePhase`: the timer counts, and at 3.0 s the budget of the
/// frame is zeroed and the character decides again.
fn update_inactive_phase(machine: &mut Machine<'_>, frame: u32, dt: f32, rng: &mut MemberRng) {
    if machine.display_only || !machine.flags.has_started || machine.flags.suppress_recovery {
        return;
    }
    machine.recovery.0 += dt;
    if machine.recovery.0 >= INACTIVE_RECOVERY_SECONDS {
        hfsm_trace!(
            "[npc-hfsm] unit={} Inactive auto-recovery: {:.1}s",
            machine.unit, machine.recovery.0
        );
        machine.budget.frame = frame;
        machine.budget.count = 0;
        decide_and_transition(machine, rng);
    }
}

/// `TryCancel`: in Lower or Rest, an interruptible lower is dropped with the
/// upper and the character parks Inactive.
fn try_cancel(machine: &mut Machine<'_>) {
    let unit = machine.unit;
    if !machine.phase.0.is_lower_running() {
        return;
    }
    if let Some(lower) = machine.actor.lower {
        if !lower.flags().can_interrupt {
            hfsm_trace!(
                "[npc-hfsm] unit={unit} TryCancel refused: {:?} cannot be interrupted",
                lower.lower_type()
            );
            return;
        }
    }
    hfsm_trace!(
        "[npc-hfsm] unit={} TryCancel: phase={}",
        unit,
        machine.phase.0.value()
    );
    if let Some(mut body) = machine.upper_body.take() {
        body.exit();
    }
    exit_current_states(machine.actor);
    *machine.lower_clock = None;
    machine.actor.lower_override = None;
    machine.phase.0 = NpcPhase::Inactive;
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

/// The machine components of one character, copied out for the set-up's
/// start (which runs outside the controller's system).
type MachineParts = (
    NpcHfsmPhase,
    NpcTransitionBudget,
    NpcRecoveryTimer,
    NpcHfsmFlags,
    NpcHfsmActor,
);

fn machine_of(world: &World, entity: Entity) -> Option<MachineParts> {
    Some((
        *world.get::<NpcHfsmPhase>(entity)?,
        *world.get::<NpcTransitionBudget>(entity)?,
        *world.get::<NpcRecoveryTimer>(entity)?,
        *world.get::<NpcHfsmFlags>(entity)?,
        *world.get::<NpcHfsmActor>(entity)?,
    ))
}

/// The AI's set-up of one character: its controller is constructed when it
/// has none (`InitializeHFSM` builds the controller and the presenter's
/// factories), then `Start()`. The start decides with the weights the
/// presenter pushed (empty outside the tutorial) and leaves the state it
/// starts on the character; its walk needs the site's navigation, which the
/// set-up does not hold here, so the walk starts unstarted.
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
    // The start's decision runs on the weights the presenter pushed (empty
    // outside the tutorial, so it draws nothing); the state it starts and its
    // object go back on the character below.
    let weights = world
        .get::<NpcUpperWeights>(entity)
        .cloned()
        .unwrap_or_default();
    let display_only = world.get::<NpcDisplayOnly>(entity).is_some();
    let action_state = world
        .get::<crate::npc::NpcActions>(entity)
        .map_or(0, |actions| actions.current as i32);
    let pause = world
        .get::<crate::npc::PauseSeconds>(entity)
        .map_or(0.0, |pause| pause.0);
    let position = world
        .get::<Transform>(entity)
        .map_or([0.0; 3], |transform| transform.translation.to_array());
    let Some((mut phase, mut budget, mut recovery, mut flags, mut actor)) = machine_of(world, entity)
    else {
        return;
    };
    if world.get::<MemberRng>(entity).is_none() {
        return;
    }
    // The set-up's start runs outside the walk writer's schedule, so the
    // character's route components travel in the machine and go back after
    // it. (No borrow is held while they are read.)
    let mut navigator = mover_parts(world, entity);
    // The start's state object and the lower's clock go back on the
    // character below, the route components after them.
    let mut upper_body = None;
    let mut lower_clock = None;
    let mut actions = crate::npc::NpcActions::default();
    let mut rest = crate::npc::RestLifecycle::default();
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
            upper_body: &mut upper_body,
            lower_clock: &mut lower_clock,
            position,
            actions: &mut actions,
            rest: &mut rest,
            pause,
            face: None,
            others: &[],
            away: false,
            navigator: navigator.take(),
        };
        // Without the site's face the start's walk has no agent here; the
        // components go back below either way. The start spends the
        // character's own generator, which is in the world for this set-up
        // (only the route components are taken out).
        if let Some(mut rng) = world.get_mut::<MemberRng>(entity) {
            start(&mut machine, &mut rng);
        }
        navigator = machine.navigator.take();
    }
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert((
            phase,
            budget,
            recovery,
            flags,
            actor,
            NpcUpperBody(upper_body),
            NpcLowerClock(lower_clock),
        ));
    }
    if let Some(navigator) = navigator {
        put_mover(world, entity, navigator);
    }
}

/// The controller's scope: the units whose machine it drives. Every other
/// character stays with the objective loop, which runs it as before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NpcHfsmScope {
    units: Vec<u32>,
}

impl NpcHfsmScope {
    /// The scope a variable value names: unit ids separated by commas or
    /// spaces, or `all` for every unit.
    pub fn parse(raw: &str) -> Self {
        let mut units = Vec::new();
        for word in raw.split(|c: char| c == ',' || c.is_whitespace()) {
            match word {
                "" => {}
                "all" => units.push(u32::MAX),
                _ => {
                    if let Ok(unit) = word.parse::<u32>() {
                        units.push(unit);
                    }
                }
            }
        }
        Self { units }
    }

    /// The scope `MOLY_NPC_HFSM_SCOPE` names; empty without it.
    pub fn from_env() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match std::env::var("MOLY_NPC_HFSM_SCOPE") {
                Ok(raw) => Self::parse(&raw),
                Err(_) => Self::default(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // A browser page has no environment; the switch is native only.
            Self::default()
        }
    }

    /// Whether `unit` is driven by the controller.
    pub fn enrols(&self, unit: u32) -> bool {
        self.units.contains(&unit) || self.units.contains(&u32::MAX)
    }

    /// Whether any unit is driven.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// The controller's switch, installed once from `MOLY_NPC_HFSM_SCOPE`.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct NpcHfsmScopeResource(pub NpcHfsmScope);

/// The controller's `OnUpdate` for the characters in its scope: each one's
/// machine in unit order, its route components held for the frame and put
/// back after it.
pub fn on_update(world: &mut World) {
    let Some(scope) = world
        .get_resource::<NpcHfsmScopeResource>()
        .map(|scope| scope.0.clone())
    else {
        return;
    };
    if scope.is_empty() {
        return;
    }
    let Some(dt) = world
        .get_resource::<crate::npc_clock::NpcClock>()
        .filter(|clock| clock.ready())
        .map(|clock| clock.delta())
    else {
        return;
    };
    let frame = world.resource::<FrameCount>().0;
    let mut units: Vec<(u32, Entity)> = world
        .query_filtered::<(Entity, &crate::npc::CharacterUnitId), With<NpcHfsmPhase>>()
        .iter(world)
        .filter(|(_, unit)| scope.enrols(unit.0))
        .map(|(entity, unit)| (unit.0, entity))
        .collect();
    units.sort_unstable();
    for (unit, entity) in units {
        update_one(world, entity, unit, frame, dt);
    }
}

/// One character's `OnUpdate`.
fn update_one(world: &mut World, entity: Entity, unit: u32, frame: u32, dt: f32) {
    // The walk reads the other presenters' positions (the random walk's
    // avoidance direction) and the character's own suspension.
    let mut others_query = world.query::<(Entity, &Transform)>();
    let others: Vec<[f32; 3]> = others_query
        .iter(world)
        .filter(|(other, _)| *other != entity)
        .map(|(_, transform)| transform.translation.to_array())
        .collect();
    let away = crate::npc::is_away(world, entity);
    if world.get::<MemberRng>(entity).is_none() {
        return;
    }
    // No borrow is held while the route components are read; the generator
    // and the site's face are taken for the frame below.
    let mut navigator = mover_parts(world, entity);
    let navigator_slot = &mut navigator;
    // The generator is copied out for the frame and written back after it,
    // so no component borrow spans the scope below.
    let Some(mut rng) = world.get::<MemberRng>(entity).cloned() else {
        return;
    };
    world.resource_scope(|world, face: Mut<super::ObjectiveFace>| {
        let mut state = SystemState::<(
            Query<(
                &mut NpcHfsmPhase,
                &mut NpcTransitionBudget,
                &mut NpcRecoveryTimer,
                &mut NpcHfsmFlags,
                &mut NpcHfsmActor,
                &NpcUpperWeights,
                &mut NpcUpperBody,
                &mut NpcLowerClock,
                Has<NpcDisplayOnly>,
            )>,
            Query<(&crate::npc::PauseSeconds, &Transform)>,
            Query<(&mut crate::npc::NpcActions, &mut crate::npc::RestLifecycle)>,
        )>::new(world);
        let (mut machines, rows, mut action_rows) = state.get_mut(world).expect("system params");
        let Ok((
            mut phase,
            mut budget,
            mut recovery,
            mut flags,
            mut actor,
            weights,
            mut upper_body,
            mut lower_clock,
            display_only,
        )) = machines.get_mut(entity)
        else {
            return;
        };
        let Ok((pause, transform)) = rows.get(entity) else {
            return;
        };
        let Ok((mut actions, mut rest)) = action_rows.get_mut(entity) else {
            return;
        };
        let action_state = actions.current as i32;
        let mut machine = Machine {
            unit,
            phase: &mut phase,
            budget: &mut budget,
            recovery: &mut recovery,
            flags: &mut flags,
            actor: &mut actor,
            weights,
            display_only,
            action_state,
            upper_body: &mut upper_body.0,
            lower_clock: &mut lower_clock.0,
            position: transform.translation.to_array(),
            actions: &mut actions,
            rest: &mut rest,
            pause: pause.0,
            face: Some(&face),
            others: &others,
            away,
            navigator: navigator_slot.take(),
        };
        advance(&mut machine, frame, dt, &mut rng);
        *navigator_slot = machine.navigator.take();
    });
    if let Some(mut stored) = world.get_mut::<MemberRng>(entity) {
        *stored = rng;
    }
    if let Some(navigator) = navigator {
        put_mover(world, entity, navigator);
    }
}

/// The plugin's share: the report resource and its line every two seconds,
/// after the machine's update, and the controller's scope.
pub fn install(app: &mut App) {
    app.insert_resource(NpcHfsmScopeResource(NpcHfsmScope::from_env()))
        .init_resource::<NpcHfsmReport>()
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
    let scope = app.world().resource::<NpcHfsmScopeResource>().0.clone();
    if !scope.is_empty() {
        info!("[npc-hfsm] scope: the controller drives units {scope:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_law::npc_hfsm::AvatarMoveStatus;

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
        upper_body: Option<UpperBody>,
        lower_clock: Option<LowerClock>,
        pause: f32,
        position: [f32; 3],
        actions: crate::npc::NpcActions,
        rest: crate::npc::RestLifecycle,
        navigator: Option<Navigator>,
        draws: Vec<f32>,
        rng: MemberRng,
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
                upper_body: None,
                lower_clock: None,
                pause: 0.0,
                position: [0.0; 3],
                actions: crate::npc::NpcActions::default(),
                rest: crate::npc::RestLifecycle::default(),
                navigator: None,
                draws: Vec::new(),
                rng: MemberRng::seeded(1),
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

        /// One frame (`frame` is the frame count the budget stamps); the
        /// generator calls it spends are recorded. The rig's route components
        /// travel in the machine for the frame and come back after it.
        fn frame(&mut self, frame: u32, dt: f32) {
            let calls = self.rng.calls();
            let Self {
                phase,
                budget,
                recovery,
                flags,
                actor,
                weights,
                display_only,
                action_state,
                upper_body,
                lower_clock,
                position,
                actions,
                rest,
                pause,
                navigator,
                rng,
                ..
            } = self;
            let mut machine = Machine {
                unit: 1,
                phase,
                budget,
                recovery,
                flags,
                actor,
                weights,
                display_only: *display_only,
                action_state: *action_state,
                upper_body,
                lower_clock,
                position: *position,
                actions,
                rest,
                pause: *pause,
                face: None,
                others: &[],
                away: false,
                navigator: navigator.take(),
            };
            advance(&mut machine, frame, dt, rng);
            *navigator = machine.navigator.take();
            let drawn = rng.calls() - calls;
            self.draws.extend(std::iter::repeat_n(1.0, drawn as usize));
        }

        /// `Start()`; the generator calls it spends are recorded.
        fn start(&mut self) {
            let calls = self.rng.calls();
            let Self {
                phase,
                budget,
                recovery,
                flags,
                actor,
                weights,
                display_only,
                action_state,
                upper_body,
                lower_clock,
                position,
                actions,
                rest,
                pause,
                navigator,
                rng,
                ..
            } = self;
            let mut machine = Machine {
                unit: 1,
                phase,
                budget,
                recovery,
                flags,
                actor,
                weights,
                display_only: *display_only,
                action_state: *action_state,
                upper_body,
                lower_clock,
                position: *position,
                actions,
                rest,
                pause: *pause,
                face: None,
                others: &[],
                away: false,
                navigator: navigator.take(),
            };
            super::start(&mut machine, rng);
            *navigator = machine.navigator.take();
            let drawn = rng.calls() - calls;
            self.draws.extend(std::iter::repeat_n(1.0, drawn as usize));
        }

        /// The machine alone (no route components), for the tests that run a
        /// single transition rather than a frame.
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
                upper_body: &mut self.upper_body,
                lower_clock: &mut self.lower_clock,
                position: self.position,
                actions: &mut self.actions,
                rest: &mut self.rest,
                pause: self.pause,
                face: None,
                others: &[],
                away: false,
                navigator: None,
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
        rig.frame(2, 1.0);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        rig.frame(3, 1.0);
        assert_eq!(rig.recovery.0, 2.0);
        // Empty weights decide RandomWalk without a draw; the rig has no
        // route components and no face, so the walk starts unstarted and the
        // machine is Upper with it.
        rig.frame(4, 1.0);
        assert!(rig.draws.is_empty());
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::RandomWalk));
        // `RandomWalkUpperState`: `OnEnter` set `_isMoving`, and the mover's
        // invalid-path branch runs on the walk's first update, where the
        // character stands, with `Success` (`GetResult` is always `Success`).
        // `UpdateUpperPhase` reads `MoveStatus == Success` and builds the
        // lower for a random walk (`GeneralTalkLowerState`, no draw). No engine
        // draw is spent on any of it.
        rig.frame(5, 1.0);
        assert!(rig.draws.is_empty());
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.upper, None);
        assert_eq!(rig.actor.lower, Some(LowerState::GeneralTalk));
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
            rig.frame(6, 1.0);
            assert_eq!(rig.recovery.0, 10.0);
            assert_eq!(rig.phase.0, NpcPhase::Inactive);
        }
    }

    #[test]
    fn a_weighted_decision_draws_once() {
        // The roulette spends one engine draw over the weight sum, and the
        // rest marker is built with its body.
        let mut rig = Rig::default().started();
        rig.frame(7, 1.0);
        rig.frame(8, 1.0);
        rig.weights = NpcUpperWeights(vec![entry(UpperType::Rest, 1.0)]);
        rig.frame(9, 1.0);
        assert_eq!(rig.rng.calls(), 1);
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::Rest));
        assert!(matches!(rig.upper_body, Some(UpperBody::Marker(_))));
        // The marker completes on its first update; the presenter's rest
        // lower (5 s) enters with no further draw.
        rig.frame(10, 1.0 / 60.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 5.0 }));
        assert_eq!(rig.rng.calls(), 1);
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
        rig.frame(11, 1.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::GeneralTalk));
        // The frame still resets the transition budget, and the frozen frame
        // advances nothing else (the Inactive timer is not reachable here).
        assert_eq!(rig.recovery.0, 1.0);
        assert_eq!(rig.budget.count, 0);

        let mut rig = Rig::default().started();
        rig.action_state = ACTION_STATE_TALK;
        rig.frame(12, 1.0);
        assert_eq!(rig.phase.0, NpcPhase::Inactive);
        assert!(!rig.flags.suppress_recovery);
    }

    #[test]
    fn stop_suppresses_and_keeps_the_timer() {
        let mut rig = Rig::default().started();
        rig.phase = NpcHfsmPhase(NpcPhase::Lower);
        rig.recovery = NpcRecoveryTimer(1.5);
        let mut body = NpcUpperBody(rig.upper_body.take());
        super::stop(1, &mut rig.phase, &mut rig.flags, &mut rig.actor, &mut body);
        assert!(rig.flags.suppress_recovery);
        assert!(is_stopped_intentionally(&rig.flags, &rig.phase));
        assert_eq!(rig.recovery.0, 1.5);
        rig.frame(13, 1.0);
        assert_eq!(rig.recovery.0, 1.5);
        // ForceStopToInactive leaves the recovery enabled and zeroes the
        // timer.
        super::force_stop_to_inactive(
            1,
            &mut rig.phase,
            &mut rig.recovery,
            &mut rig.actor,
            &mut body,
        );
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
        rig.start();
        assert!(rig.flags.has_started);
        assert!(!rig.flags.suppress_recovery);
        assert!(rig.draws.is_empty());
        // The empty weight list answers RandomWalk without a draw; the walk
        // has no route here, so it keeps its enter's running flag and the
        // machine is Upper with it.
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::RandomWalk));
        assert_eq!(rig.rng.calls(), 0);
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

    #[test]
    fn a_rest_upper_completes_into_the_five_second_rest_and_back_to_the_rest() {
        // The marker completes on its first update. The presenter's factory
        // builds `RestLowerState(presenter, model, result, 5.0)` for a `Rest`
        // result (case 4), so the lower rests 5 s and draws nothing; the idle
        // draw belongs to the factory's default arm only.
        let mut rig = Rig { pause: 4.0, ..Rig::default().started() };
        rig.weights = NpcUpperWeights(vec![entry(UpperType::Rest, 1.0)]);
        rig.phase = NpcHfsmPhase(NpcPhase::Inactive);
        rig.recovery = NpcRecoveryTimer(3.0);
        rig.frame(15, 1.0);
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::Rest));
        assert_eq!(rig.rng.calls(), 1);
        // The upper's first update completes the marker and hands the result
        // to the presenter's factory, whose `Rest` case is a 5 s rest.
        rig.frame(16, 1.0 / 60.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 5.0 }));
        // The roulette spent the one draw of the rest frame; the rest itself
        // draws nothing.
        assert_eq!(rig.rng.calls(), 1);
        // The 5 s rest completes on the Lower phase, and
        // `EnterRestOrDecideNext` starts the PauseTime rest (4 s) in the Rest
        // phase; that entry draws nothing either.
        rig.frame(17, 5.1);
        assert_eq!(rig.phase.0, NpcPhase::Rest);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 4.0 }));
        assert_eq!(rig.rng.calls(), 1);
        // The PauseTime rest completes and the Rest phase goes straight back
        // to the decision: the roulette's one draw over the 1.0 weight lands
        // on the rest marker, which completes on its next update into the 5 s
        // rest again.
        rig.frame(18, 4.1);
        assert_eq!(rig.rng.calls(), 2);
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::Rest));
        rig.frame(19, 1.0 / 60.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 5.0 }));
        assert_eq!(rig.rng.calls(), 2);
    }

    #[test]
    fn a_plan_starts_its_marker_and_passes_over_an_unset_override() {
        let mut rig = Rig::default().started();
        let plan = NpcPlan::marker(moly_law::npc_hfsm::PlanUpper::Rest);
        {
            let mut machine = rig.machine();
            transition_to_plan(&mut machine, &plan);
        }
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::Rest));
        assert_eq!(rig.actor.lower_override, None);
        assert_eq!(rig.budget.count, 1);
        // The override is consumed after the upper completes.
        let mut rig = Rig::default().started();
        let mut plan = NpcPlan::marker(moly_law::npc_hfsm::PlanUpper::Stay);
        plan.lower_override = Some(LowerState::Rest { duration: 2.0 });
        {
            let mut machine = rig.machine();
            transition_to_plan(&mut machine, &plan);
        }
        assert_eq!(rig.actor.lower_override, Some(LowerState::Rest { duration: 2.0 }));
        rig.frame(21, 1.0 / 60.0);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 2.0 }));
        assert_eq!(rig.actor.lower_override, None);
    }

    #[test]
    fn a_zero_pause_decides_at_once_from_the_rest() {
        // EnterRestOrDecideNext with PauseTime 0: the rest is skipped, and the
        // decision runs at once (here: no route, so the walk is unstarted).
        let mut rig = Rig::default().started();
        rig.actor.lower = Some(LowerState::Idle { duration: 0.0 });
        rig.lower_clock = Some(LowerClock::default());
        rig.phase = NpcHfsmPhase(NpcPhase::Lower);
        rig.frame(22, 0.5);
        assert_eq!(rig.actor.lower, None);
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::RandomWalk));
    }

    #[test]
    fn a_walk_without_a_route_completes_into_its_lower() {
        // A random walk with no route keeps `OnEnter`'s `_isMoving` and
        // completes on its next update, where the character stands, with
        // `Success` (`RandomWalkUpperState.GetResult` is always `Success`).
        // `UpdateUpperPhase`'s success branch builds the random walk's lower
        // (`GeneralTalkLowerState`, no draw). Nothing draws on either frame.
        let mut rig = Rig::default().started();
        rig.phase = NpcHfsmPhase(NpcPhase::Inactive);
        rig.recovery = NpcRecoveryTimer(3.0);
        rig.frame(23, 0.1);
        assert_eq!(rig.actor.upper, Some(UpperState::RandomWalk));
        assert_eq!(rig.rng.calls(), 0);
        rig.frame(24, 0.1);
        assert_eq!(rig.phase.0, NpcPhase::Lower);
        assert_eq!(rig.actor.upper, None);
        assert_eq!(rig.actor.lower, Some(LowerState::GeneralTalk));
        assert_eq!(rig.rng.calls(), 0);
    }

    #[test]
    fn a_failed_walk_rests_for_the_pause_time() {
        // A random walk never fails (`GetResult` is always `Success`), so the
        // controller's failure branch is driven with a walk that does: a
        // `ChangeSite` the host started no route for ends `NoneRoute` on its
        // first update, at the character's place. UpdateUpperPhase's
        // `MoveStatus != Success` branch clears the pending talk data and calls
        // `EnterRestOrDecideNext`, which rests for max(PauseTime, 0) in the
        // Rest phase (`RestLowerState`, no draw); the rest's completion
        // returns to the decision.
        let mut rig = Rig {
            pause: 2.0,
            ..Rig::default().started()
        };
        rig.phase = NpcHfsmPhase(NpcPhase::Inactive);
        rig.recovery = NpcRecoveryTimer(3.0);
        rig.weights = NpcUpperWeights(vec![entry(UpperType::ChangeSite, 1.0)]);
        rig.frame(25, 0.1);
        assert_eq!(rig.actor.upper, Some(UpperState::ChangeSite));
        assert_eq!(rig.rng.calls(), 1);
        rig.frame(26, 0.1);
        assert_eq!(rig.phase.0, NpcPhase::Rest);
        assert_eq!(rig.actor.lower, Some(LowerState::Rest { duration: 2.0 }));
        assert_eq!(rig.actor.upper, None);
        rig.frame(27, 2.1);
        assert_eq!(rig.phase.0, NpcPhase::Upper);
        assert_eq!(rig.actor.upper, Some(UpperState::ChangeSite));
        assert_eq!(rig.rng.calls(), 2);
    }

    #[test]
    fn the_upper_factory_builds_every_type() {
        // `CreateUpperState` keeps every type, with the presenter's one
        // fallback: a `MoveToNPC` with no selectable character builds the
        // random walk. The two markers (`RestUpperState` / `StayUpperState`)
        // never set `_isMoving`, so their own `OnUpdate` completes them on
        // their first update with `Success` at the presenter's position.
        //
        // Every walk's `OnEnter` sets `_isMoving` and starts its mover; a route
        // the host could not start is the mover's invalid-path branch on the
        // walk's first update (`MoveTo…CoreAsync` awaits the agent first). The
        // walk then ends where the character stands: the random walk's
        // `GetResult` is always `Success`; `MoveToPosition` and
        // `MoveToFixture` measure the character against the place the host sent
        // them, which is where it stands, and answer `Success`; the player and
        // change-site walks report `NoneRoute` without measuring.
        for state in UpperType::ALL {
            let mut body = UpperBody::enter(
                create_upper(state),
                UpperStart::unstarted(),
                None,
                [1.0, 0.0, 2.0],
            );
            let built = body.state();
            let built = match built {
                UpperState::MoveToFixture { .. } => UpperType::MoveToFixture,
                other => other.upper_type(),
            };
            let want_state = match state {
                UpperType::MoveToNPC => UpperType::RandomWalk,
                other => other,
            };
            assert_eq!(built, want_state, "{state:?} keeps its type");
            assert!(!body.is_complete(), "{state:?} is not complete on enter");
            let frame = UpperFrame {
                face: None,
                poll: Some(WalkPoll {
                    position: [1.0, 0.0, 2.0],
                    remaining: f32::INFINITY,
                    stopped: false,
                }),
            };
            body.update(0.0, &frame);
            let result = body.result([1.0, 0.0, 2.0]);
            assert!(body.is_complete(), "{state:?} settles on its first update");
            assert!(!body.is_moving(), "{state:?} clears its flag");
            let want = match state {
                // `MoveToNPC` fell back to the random walk above, whose result
                // is always `Success`. The player and the change-site walks
                // report the missing route.
                UpperType::MoveToPlayer | UpperType::ChangeSite => AvatarMoveStatus::NoneRoute,
                _ => AvatarMoveStatus::Success,
            };
            assert_eq!(result.move_status, want, "{state:?} unstarted");
            assert_eq!(result.arrival, [1.0, 0.0, 2.0], "{state:?} where it stands");
        }
    }

    #[test]
    fn the_scope_parses_units_and_all() {
        assert!(NpcHfsmScope::parse("").is_empty());
        assert!(NpcHfsmScope::parse("  ").is_empty());
        let scope = NpcHfsmScope::parse("3, 17 4");
        assert!(scope.enrols(3) && scope.enrols(17) && scope.enrols(4));
        assert!(!scope.enrols(5));
        assert!(NpcHfsmScope::parse("all").enrols(9_999));
        assert!(!NpcHfsmScope::parse("x,7y").enrols(7));
        assert!(NpcHfsmScope::parse("x,7y").is_empty());
    }
}
