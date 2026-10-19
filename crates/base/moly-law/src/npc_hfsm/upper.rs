//! The upper-state roulette (`DecideUpperState`, `SetUpperStateWeight`) and
//! the upper state bodies: the two markers and the six walks.
//!
//! `SetUpperStateWeight` adds an entry or replaces the entry of the same
//! state type in place (the list keeps its order, a zero weight stays).
//! Only the presenter's tutorial branch pushes weights (`RandomWalk` 5,
//! `MoveToFixture` 5, `Rest` 2); a normal session's list is empty.
//!
//! `DecideUpperState` with an empty list, or with a weight sum at or below
//! zero, returns `RandomWalk` **without drawing**. Otherwise it takes
//! exactly one engine float draw `r` in `[0, total)`, scans a single-
//! precision running sum and takes the first entry with `r <= acc`; the
//! trailing fallback (reached only through rounding) is the last entry.
//!
//! The markers (`Rest`, `Stay`) have no fields, no mover and no timer; neither
//! sets a running flag (the source's `OnUpdate` sets `IsComplete` outright), so
//! the first update completes them.
//!
//! A walk state is pure: the host starts the agent's route before the state
//! is built ([`WalkStart`]) and hands it the frame's poll ([`WalkPoll`]); the
//! state decides the arrival, the move status and the result. The navigation
//! itself stays the host's, in [`super::upper_walk`] for the random walk.
//!
//! `_isMoving`: every walk's `OnEnter` sets it and starts its mover; the
//! mover clears it on every exit branch (arrival, invalid path, absent agent,
//! timeout, stall) and `OnExit` clears it again. The inherited `OnUpdate`
//! completes a walk once the flag is clear, so a route the host could not
//! start ends the walk on its first update (the mover reaches the agent only
//! after its own awaits), while an absent agent ends it in the enter's own
//! synchronous segment. `_moveStatus` starts at `Success` (the source field
//! initializer); the mover writes another status only from its own branches,
//! and the random walk's result is always `Success`.

use super::timing::ALREADY_ARRIVED_DISTANCE;
use super::{AvatarMoveStatus, UpperBase, UpperEntry, UpperResult, UpperState, UpperType};

/// `RestUpperState` and `StayUpperState`: no fields, no mover, no timer. The
/// enter is empty; neither state sets a running flag, and `OnUpdate` sets
/// `IsComplete` unconditionally, so the first update completes it; the result
/// tags the state type, the presenter's position and `Success`.
#[derive(Debug, Clone, PartialEq)]
pub struct RestUpper {
    kind: UpperType,
    base: UpperBase,
    entry: Option<UpperEntry>,
}

impl RestUpper {
    /// `Rest` for the controller's rest marker.
    pub fn rest() -> Self {
        Self::of(UpperType::Rest)
    }

    /// `Stay`, the same marker under its own type.
    pub fn stay() -> Self {
        Self::of(UpperType::Stay)
    }

    fn of(kind: UpperType) -> Self {
        Self {
            kind,
            base: UpperBase::default(),
            entry: None,
        }
    }

    /// The upper state object.
    pub fn state(&self) -> UpperState {
        match self.kind {
            UpperType::Stay => UpperState::Stay,
            _ => UpperState::Rest,
        }
    }

    /// `Enter()`: the base's entry fields; the marker sets no running flag and
    /// its own `OnUpdate` completes it, so the first `Update()` does.
    pub fn enter(&mut self) {
        self.entry = Some(self.base.enter(false));
    }

    /// `Update()`: one frame of the base; the marker completes on this same
    /// update.
    pub fn update(&mut self, dt: f32) {
        if let Some(entry) = self.entry.as_mut() {
            entry.update(dt);
        }
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.entry.is_some_and(|entry| entry.is_complete)
    }

    /// `GetResult()`: the marker's type, the presenter's position, no
    /// fixture or character target, `Success`.
    pub fn result(&self, position: [f32; 3]) -> UpperResult {
        UpperResult {
            state: self.kind,
            arrival: position,
            target_fixture: None,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        }
    }

    /// `Exit()`.
    pub fn exit(&mut self) {
        if let Some(entry) = self.entry.as_mut() {
            entry.exit();
        }
    }
}

/// What the host's mover answered when it started the walk: the agent's
/// route runs, there is no agent to run it, or the agent found no route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WalkStart {
    /// The walk runs; the agent completes it at this stopping distance.
    Running { stopping_distance: f32 },
    /// The character has no usable navigation agent here (absent or
    /// inactive); an agent that is merely off the navmesh is waited for.
    NoAgent,
    /// The agent found no route to the target.
    NoRoute,
}

/// One frame of the host's mover: where the character stands, the agent's
/// distance to the walk's target, and whether the route ended without
/// arriving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WalkPoll {
    /// `presenter.Position`.
    pub position: [f32; 3],
    /// `NavMeshAgent.remainingDistance`.
    pub remaining: f32,
    /// The route ended without arriving (a lost or partial path).
    pub stopped: bool,
}

/// The fields every walk state carries: the running flag, the completion
/// distance, the move status, the arrival and the base clock.
#[derive(Debug, Clone, PartialEq)]
struct WalkFields {
    moving: bool,
    is_complete: bool,
    elapsed: f32,
    move_status: AvatarMoveStatus,
    arrival: [f32; 3],
    goal: f32,
    /// The host started no route: the mover's invalid-path branch has not run
    /// yet, because it reaches the agent only after its own awaits.
    unstarted: bool,
}

impl WalkFields {
    /// `OnEnter` sets `_isMoving` before the mover runs; a mover that cannot
    /// start at all (no agent) clears it in the enter's own synchronous
    /// segment, and a mover whose path is invalid clears it on the first
    /// resumed segment, the walk's first update.
    fn entering() -> Self {
        Self {
            moving: true,
            is_complete: false,
            elapsed: 0.0,
            // `_moveStatus` is the source's field initializer: `Success`.
            move_status: AvatarMoveStatus::Success,
            arrival: [0.0; 3],
            goal: 0.0,
            unstarted: false,
        }
    }

    /// The mover started: running until the completion distance. `OnEnter`
    /// owns the running flag; this only records the distance.
    fn running(&mut self, stopping_distance: f32) {
        self.goal = stopping_distance;
    }

    /// The host started no route: the mover's invalid-path branch
    /// (`MoveTo…CoreAsync`) can only report after it has awaited the agent, so
    /// the flag stays as `OnEnter` set it until the walk's first update.
    fn defer_invalid_path(&mut self) {
        self.unstarted = true;
    }

    /// The walk ended: the place it ended and the status its `GetResult`
    /// carries. Every mover's settled branch writes it, the async wrapper
    /// clears the flag once more, and `OnExit` clears it again; the controller
    /// separates arrival from failure by `NPCUpperStateResult.MoveStatus`
    /// alone.
    fn finish(&mut self, position: [f32; 3], status: AvatarMoveStatus) {
        self.arrival = position;
        self.end(status);
    }

    /// The walk ended where its arrival already points: every mover's
    /// absent-agent branch clears the flag without touching `_arrivalPosition`
    /// and writes the status (`InValid`, or the random walk's untouched
    /// `Success`).
    fn end(&mut self, status: AvatarMoveStatus) {
        self.move_status = status;
        self.moving = false;
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// `Update()`: add the frame's time; the inherited `OnUpdate` completes
    /// the state once the running flag is clear.
    fn frame(&mut self, dt: f32) {
        self.elapsed += dt;
        if !self.moving {
            self.is_complete = true;
        }
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let x = a[0] - b[0];
    let y = a[1] - b[1];
    let z = a[2] - b[2];
    (x * x + y * y + z * z).sqrt()
}

/// `MoveToFixtureUpperState`: a walk to a fixture's action point, or to the
/// fixture's own position when there is no action point. The mover sets the
/// agent's stopping distance to 0.01 for an action point and gives the
/// previous distance back on exit. A missing fixture and action point fails
/// the state as `InValid` in the enter; a route that ends without arriving
/// completes where the character stands when it is within the previous
/// stopping distance plus 0.3 (`ALREADY_ARRIVED_DISTANCE`), else `NoneRoute`.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveToFixtureUpper {
    /// `_targetFixture`: the placement id.
    pub target_fixture: Option<u32>,
    /// `_actionPointPosition`.
    pub action_point: Option<[f32; 3]>,
    /// The place the walk goes to.
    pub target: [f32; 3],
    /// The agent's stopping distance before the walk.
    stopping_before: f32,
    fields: WalkFields,
}

impl MoveToFixtureUpper {
    /// `OnEnter` and the mover's start: `fixture_position` is the live
    /// fixture's `GlobalPosition`, `stopping_before` the agent's distance
    /// before the mover narrowed it, and `start` the route it asked for. The
    /// character's position is unused: the enter's own branches keep the
    /// arrival the constructor holds.
    ///
    /// The mover walks to the action point, or to the fixture's position when
    /// the state has no action point.
    #[allow(clippy::too_many_arguments)]
    pub fn enter(
        target_fixture: Option<u32>,
        action_point: Option<[f32; 3]>,
        fixture_position: Option<[f32; 3]>,
        stopping_before: f32,
        start: WalkStart,
        _position: [f32; 3],
    ) -> Self {
        let mut state = Self {
            target_fixture,
            action_point,
            target: [0.0; 3],
            stopping_before,
            fields: WalkFields::entering(),
        };
        let Some(fixture_position) = fixture_position.or(action_point) else {
            // `OnEnter`'s first branch: no fixture and no action point writes
            // the status and clears the flag, leaving `_arrivalPosition` at
            // its default.
            state.fields.end(AvatarMoveStatus::InValid);
            return state;
        };
        state.target = action_point.unwrap_or(fixture_position);
        state.fields.arrival = state.target;
        match start {
            WalkStart::Running {
                stopping_distance,
            } => state.fields.running(stopping_distance),
            // The mover reaches the agent after its first await, so a route
            // the host could not start is reported on the walk's first
            // update, not here.
            WalkStart::NoRoute => state.fields.defer_invalid_path(),
            // The agent is read before the mover's first await: no agent ends
            // the state in the enter's own segment, at the arrival it already
            // holds.
            WalkStart::NoAgent => state.fields.end(AvatarMoveStatus::InValid),
        }
        state
    }

    /// The mover's invalid-path branch: a walk that stopped without arriving
    /// arrives only when the character already stands within the agent's
    /// previous stopping distance plus 0.3.
    fn settle_without_route(&mut self, position: [f32; 3]) {
        let status = if distance(position, self.target)
            <= self.stopping_before + ALREADY_ARRIVED_DISTANCE
        {
            AvatarMoveStatus::Success
        } else {
            AvatarMoveStatus::NoneRoute
        };
        self.fields.finish(position, status);
    }

    /// `Update()`: the frame's poll, then the invalid-path branch a route the
    /// host could not start reaches here.
    pub fn update(&mut self, dt: f32, poll: WalkPoll) {
        if self.fields.unstarted {
            self.fields.unstarted = false;
            self.settle_without_route(poll.position);
        } else if self.fields.moving {
            if poll.stopped {
                self.settle_without_route(poll.position);
            } else if poll.remaining <= self.fields.goal {
                self.fields.finish(poll.position, AvatarMoveStatus::Success);
            }
        }
        self.fields.frame(dt);
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.fields.is_complete
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.fields.moving()
    }

    /// `GetResult()`.
    pub fn result(&self) -> UpperResult {
        UpperResult {
            state: UpperType::MoveToFixture,
            arrival: self.fields.arrival,
            target_fixture: self.target_fixture,
            target_npc: None,
            move_status: self.fields.move_status,
        }
    }

    /// `OnExit`: the agent's stopping distance to restore, when the mover
    /// narrowed it.
    pub fn exit(&mut self) -> Option<f32> {
        self.fields.moving = false;
        (self.action_point.is_some()).then_some(self.stopping_before)
    }
}

/// `MoveToPositionUpperState`: a walk to a given position. A walk that ends
/// without arriving completes where the character stands when the position
/// is within the agent's stopping distance plus 0.3, or when
/// `tolerates_none_route` (`_toleratesNoneRoute`, false from the factory) is
/// set; otherwise it fails `NoneRoute`.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveToPositionUpper {
    /// The place the walk goes to.
    pub target: [f32; 3],
    /// `_toleratesNoneRoute`.
    pub tolerates_none_route: bool,
    fields: WalkFields,
}

impl MoveToPositionUpper {
    /// `OnEnter` and the mover's start. The character's position is unused:
    /// the mover's branches report the presenter's own position, which the
    /// poll carries.
    pub fn enter(
        target: [f32; 3],
        tolerates_none_route: bool,
        start: WalkStart,
        _position: [f32; 3],
    ) -> Self {
        let mut state = Self {
            target,
            tolerates_none_route,
            fields: WalkFields::entering(),
        };
        state.fields.arrival = target;
        match start {
            WalkStart::Running {
                stopping_distance,
            } => state.fields.running(stopping_distance),
            // The mover's absent-agent branch: the status changes and the
            // arrival stays the target.
            WalkStart::NoAgent => state.fields.end(AvatarMoveStatus::InValid),
            // The mover's invalid-path branch runs on the walk's first
            // update, after it has awaited the route.
            WalkStart::NoRoute => state.fields.defer_invalid_path(),
        }
        state
    }

    /// The mover's arrival tests on a route that ended: the distance to the
    /// target against the stopping distance plus 0.3, and the tolerance.
    fn settle_without_route(&mut self, position: [f32; 3]) {
        let near =
            distance(position, self.target) <= self.fields.goal + ALREADY_ARRIVED_DISTANCE;
        let status = if self.tolerates_none_route || near {
            AvatarMoveStatus::Success
        } else {
            AvatarMoveStatus::NoneRoute
        };
        self.fields.finish(position, status);
    }

    /// `Update()`: the frame's poll, then the invalid-path branch a route the
    /// host could not start reaches here.
    pub fn update(&mut self, dt: f32, poll: WalkPoll) {
        if self.fields.unstarted {
            self.fields.unstarted = false;
            self.settle_without_route(poll.position);
        } else if self.fields.moving {
            if poll.stopped {
                self.settle_without_route(poll.position);
            } else if poll.remaining <= self.fields.goal {
                self.fields.finish(poll.position, AvatarMoveStatus::Success);
            }
        }
        self.fields.frame(dt);
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.fields.is_complete
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.fields.moving()
    }

    /// `GetResult()`.
    pub fn result(&self) -> UpperResult {
        UpperResult {
            state: UpperType::MoveToPosition,
            arrival: self.fields.arrival,
            target_fixture: None,
            target_npc: None,
            move_status: self.fields.move_status,
        }
    }

    /// `OnExit`.
    pub fn exit(&mut self) {
        self.fields.moving = false;
    }
}

/// `MoveToNPCUpperState` and `MoveToPlayerUpperState`: a walk to a target
/// snapshotted on enter. No target fails `InValid`; no route fails
/// `NoneRoute`; an arrival is `remainingDistance <= stoppingDistance`.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveToTargetUpper {
    kind: UpperType,
    /// The target snapshotted on enter.
    pub target: [f32; 3],
    fields: WalkFields,
}

impl MoveToTargetUpper {
    /// `MoveToNPCUpperState.OnEnter`.
    pub fn npc(target: Option<[f32; 3]>, start: WalkStart, position: [f32; 3]) -> Self {
        Self::enter(UpperType::MoveToNPC, target, start, position)
    }

    /// `MoveToPlayerUpperState.OnEnter`.
    pub fn player(target: Option<[f32; 3]>, start: WalkStart, position: [f32; 3]) -> Self {
        Self::enter(UpperType::MoveToPlayer, target, start, position)
    }

    fn enter(
        kind: UpperType,
        target: Option<[f32; 3]>,
        start: WalkStart,
        position: [f32; 3],
    ) -> Self {
        let mut state = Self {
            kind,
            target: position,
            fields: WalkFields::entering(),
        };
        let Some(target) = target else {
            state.fields.finish(position, AvatarMoveStatus::InValid);
            return state;
        };
        state.target = target;
        state.fields.arrival = target;
        match start {
            WalkStart::Running {
                stopping_distance,
            } => state.fields.running(stopping_distance),
            // The mover's absent-agent branch: the status changes and the
            // arrival stays the target's snapshot.
            WalkStart::NoAgent => state.fields.end(AvatarMoveStatus::InValid),
            // The mover's invalid-path branch runs on the walk's first
            // update, after it has awaited the route.
            WalkStart::NoRoute => state.fields.defer_invalid_path(),
        }
        state
    }

    /// `Update()`: the frame's poll, then the invalid-path branch a route the
    /// host could not start reaches here.
    pub fn update(&mut self, dt: f32, poll: WalkPoll) {
        if self.fields.unstarted {
            self.fields.unstarted = false;
            self.fields.finish(poll.position, AvatarMoveStatus::NoneRoute);
        } else if self.fields.moving {
            if poll.stopped {
                self.fields.finish(poll.position, AvatarMoveStatus::NoneRoute);
            } else if poll.remaining <= self.fields.goal {
                self.fields.finish(poll.position, AvatarMoveStatus::Success);
            }
        }
        self.fields.frame(dt);
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.fields.is_complete
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.fields.moving()
    }

    /// The state object: `MoveToNPC` or `MoveToPlayer`.
    pub fn upper_state(&self) -> UpperState {
        match self.kind {
            UpperType::MoveToPlayer => UpperState::MoveToPlayer,
            _ => UpperState::MoveToNpc { npc: 0 },
        }
    }

    /// `GetResult()`.
    pub fn result(&self, target_npc: Option<u32>) -> UpperResult {
        UpperResult {
            state: self.kind,
            arrival: self.fields.arrival,
            target_fixture: None,
            target_npc: if self.kind == UpperType::MoveToNPC {
                target_npc
            } else {
                None
            },
            move_status: self.fields.move_status,
        }
    }

    /// `OnExit`.
    pub fn exit(&mut self) {
        self.fields.moving = false;
    }
}

/// `RandomWalkUpperState`: a walk to a point sampled around the character.
/// The result is always `Success`; a walk that cannot run ends where the
/// character stands.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomWalkUpper {
    /// `_arrivalPosition`: the sampled target, then the place the walk ended.
    pub arrival: [f32; 3],
    fields: WalkFields,
}

impl RandomWalkUpper {
    /// `OnEnter`: `target` is the accepted sample and `start` the route it
    /// asked for. The arrival stays the sample; the mover writes no status.
    pub fn enter(target: [f32; 3], start: WalkStart) -> Self {
        let mut state = Self {
            arrival: target,
            fields: WalkFields::entering(),
        };
        state.fields.arrival = target;
        match start {
            WalkStart::Running {
                stopping_distance,
            } => state.fields.running(stopping_distance),
            // Read synchronously: no agent ends the walk in the enter, where
            // the character stands, and the status is already `Success`.
            WalkStart::NoAgent => state.fields.end(AvatarMoveStatus::Success),
            // The mover's invalid-path branch runs on the walk's first
            // update, after it has awaited the route.
            WalkStart::NoRoute => state.fields.defer_invalid_path(),
        }
        state
    }

    /// `Update()`: the frame's poll. The state ends where the walk stops,
    /// with `Success` whatever the walk's end.
    pub fn update(&mut self, dt: f32, poll: WalkPoll) {
        if self.fields.unstarted {
            self.fields.unstarted = false;
            self.arrival = poll.position;
            self.fields.finish(poll.position, AvatarMoveStatus::Success);
        } else if self.fields.moving && (poll.stopped || poll.remaining <= self.fields.goal) {
            self.arrival = poll.position;
            self.fields.finish(poll.position, AvatarMoveStatus::Success);
        }
        self.fields.frame(dt);
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.fields.is_complete
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.fields.moving()
    }

    /// `GetResult()`: `Success`, the place the walk ended, no targets.
    pub fn result(&self) -> UpperResult {
        UpperResult {
            state: UpperType::RandomWalk,
            arrival: self.arrival,
            target_fixture: None,
            target_npc: None,
            move_status: AvatarMoveStatus::Success,
        }
    }

    /// `OnExit`.
    pub fn exit(&mut self) {
        self.fields.moving = false;
    }
}

/// `ChangeSiteUpperState`: a walk to the door of the site being left. A
/// missing agent fails `InValid`; a door the agent cannot reach fails
/// `NoneRoute`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSiteUpper {
    /// The door the walk goes to.
    pub door: [f32; 3],
    fields: WalkFields,
}

impl ChangeSiteUpper {
    /// `OnEnter` and the mover's start: `door` is the door point the host
    /// snapped onto the site's navigation, and the arrival the constructor
    /// already holds.
    pub fn enter(door: [f32; 3], start: WalkStart) -> Self {
        let mut state = Self {
            door,
            fields: WalkFields::entering(),
        };
        state.fields.arrival = door;
        match start {
            WalkStart::Running {
                stopping_distance,
            } => state.fields.running(stopping_distance),
            // The mover's absent-agent branch: the status changes and the
            // arrival stays the door.
            WalkStart::NoAgent => state.fields.end(AvatarMoveStatus::InValid),
            // The mover's invalid-path branch runs on the walk's first
            // update, after it has awaited the route.
            WalkStart::NoRoute => state.fields.defer_invalid_path(),
        }
        state
    }

    /// `Update()`: the frame's poll, then the invalid-path branch a route the
    /// host could not start reaches here.
    pub fn update(&mut self, dt: f32, poll: WalkPoll) {
        if self.fields.unstarted {
            self.fields.unstarted = false;
            self.fields.finish(poll.position, AvatarMoveStatus::NoneRoute);
        } else if self.fields.moving {
            if poll.stopped {
                self.fields.finish(poll.position, AvatarMoveStatus::NoneRoute);
            } else if poll.remaining <= self.fields.goal {
                self.fields.finish(poll.position, AvatarMoveStatus::Success);
            }
        }
        self.fields.frame(dt);
    }

    /// `IsComplete`.
    pub fn is_complete(&self) -> bool {
        self.fields.is_complete
    }

    /// The state's own `_isMoving`: the move still runs.
    pub fn moving(&self) -> bool {
        self.fields.moving()
    }

    /// `GetResult()`.
    pub fn result(&self) -> UpperResult {
        UpperResult {
            state: UpperType::ChangeSite,
            arrival: self.fields.arrival,
            target_fixture: None,
            target_npc: None,
            move_status: self.fields.move_status,
        }
    }

    /// `OnExit`.
    pub fn exit(&mut self) {
        self.fields.moving = false;
    }
}

/// `UpperStateWeightEntry`: one state type and its weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperStateWeight {
    /// The state type.
    pub state: UpperType,
    /// Its weight.
    pub weight: f32,
}

/// `SetUpperStateWeight`: adds `entry`, replacing an entry of the same state
/// type in place.
pub fn set_upper_weight(weights: &mut Vec<UpperStateWeight>, state: UpperType, weight: f32) {
    if let Some(existing) = weights.iter_mut().find(|entry| entry.state == state) {
        existing.weight = weight;
    } else {
        weights.push(UpperStateWeight { state, weight });
    }
}

/// What `DecideUpperState` answered, and whether it drew.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpperChoice {
    /// The state type.
    pub state: UpperType,
    /// The draw, when one was made.
    pub draw: Option<f32>,
    /// The weight sum the draw was taken in.
    pub total: f32,
    /// The entry index the scan accepted (`None` on the no-draw paths and on
    /// the trailing fallback).
    pub entry: Option<usize>,
}

/// `DecideUpperState` over `weights`; `draw` is called only when the sum is
/// positive and receives the sum.
pub fn decide_upper(
    weights: &[UpperStateWeight],
    draw: &mut dyn FnMut(f32) -> f32,
) -> UpperChoice {
    if weights.is_empty() {
        return UpperChoice {
            state: UpperType::RandomWalk,
            draw: None,
            total: 0.0,
            entry: None,
        };
    }
    let mut total = 0.0f32;
    for entry in weights {
        total += entry.weight;
    }
    if total <= 0.0 {
        return UpperChoice {
            state: UpperType::RandomWalk,
            draw: None,
            total,
            entry: None,
        };
    }
    let drawn = draw(total);
    let mut acc = 0.0f32;
    for (index, entry) in weights.iter().enumerate() {
        acc += entry.weight;
        if drawn <= acc {
            return UpperChoice {
                state: entry.state,
                draw: Some(drawn),
                total,
                entry: Some(index),
            };
        }
    }
    UpperChoice {
        state: weights[weights.len() - 1].state,
        draw: Some(drawn),
        total,
        entry: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: UpperType, weight: f32) -> UpperStateWeight {
        UpperStateWeight { state, weight }
    }

    const RUN: WalkStart = WalkStart::Running {
        stopping_distance: 0.1,
    };

    fn poll(position: [f32; 3], remaining: f32, stopped: bool) -> WalkPoll {
        WalkPoll {
            position,
            remaining,
            stopped,
        }
    }

    #[test]
    fn a_fixture_state_targets_the_action_point_else_the_fixture() {
        // OnEnter: the action point when the data has one, else the
        // fixture's GlobalPosition.
        let state = MoveToFixtureUpper::enter(
            Some(4),
            Some([5.0, 0.5, 6.0]),
            Some([7.0, 0.5, 8.0]),
            0.1,
            RUN,
            [0.0; 3],
        );
        assert_eq!(state.target, [5.0, 0.5, 6.0]);
        assert_eq!(state.result().arrival, [5.0, 0.5, 6.0]);
        assert_eq!(state.result().target_fixture, Some(4));
        let state = MoveToFixtureUpper::enter(
            Some(4),
            None,
            Some([7.0, 0.5, 8.0]),
            0.1,
            RUN,
            [0.0; 3],
        );
        assert_eq!(state.target, [7.0, 0.5, 8.0]);
        assert_eq!(state.result().arrival, [7.0, 0.5, 8.0]);
    }

    #[test]
    fn a_fixture_state_narrows_the_stopping_distance_and_restores_it() {
        // The mover sets 0.01 while the action point's walk runs and gives
        // the agent's previous distance back on exit. `MoveToFixtureUpperState`
        // `OnEnter` sets `_isMoving`, and `_moveStatus` starts at the field
        // initializer `Success`; the mover only ever writes a failure status,
        // so a walk that is still running answers `Success`.
        let mut state = MoveToFixtureUpper::enter(
            Some(1),
            Some([1.0, 0.0, 1.0]),
            Some([1.0, 0.0, 1.0]),
            0.1,
            WalkStart::Running {
                stopping_distance: 0.01,
            },
            [0.0; 3],
        );
        state.update(1.0 / 60.0, poll([0.0; 3], 0.5, false));
        assert!(state.moving());
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(1.0 / 60.0, poll([0.995, 0.0, 1.0], 0.005, false));
        assert!(state.is_complete());
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        assert_eq!(state.result().arrival, [0.995, 0.0, 1.0]);
        assert_eq!(state.exit(), Some(0.1));
        // A walk without an action point keeps the agent's own distance.
        let mut state = MoveToFixtureUpper::enter(
            Some(1),
            None,
            Some([1.0, 0.0, 1.0]),
            0.1,
            RUN,
            [0.0; 3],
        );
        state.update(0.0, poll([0.95, 0.0, 1.0], 0.05, false));
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        assert_eq!(state.exit(), None);
    }

    #[test]
    fn a_fixture_state_without_a_fixture_or_an_action_point_fails() {
        // OnEnter: no fixture and no action point -> InValid, not moving,
        // the arrival at its default.
        let mut state =
            MoveToFixtureUpper::enter(None, None, None, 0.1, WalkStart::NoAgent, [0.0; 3]);
        assert_eq!(state.result().move_status, AvatarMoveStatus::InValid);
        assert!(!state.moving());
        assert_eq!(state.result().arrival, [0.0; 3]);
        state.update(0.0, poll([0.0; 3], f32::INFINITY, false));
        assert!(state.is_complete());
        // A fixture whose position this host cannot read fails the same way.
        let state = MoveToFixtureUpper::enter(Some(3), None, None, 0.1, RUN, [0.0; 3]);
        assert_eq!(state.result().move_status, AvatarMoveStatus::InValid);
        assert_eq!(state.result().arrival, [0.0; 3]);
    }

    #[test]
    fn a_fixture_state_without_route_arrives_only_within_the_previous_distance() {
        // The mover's invalid-path branch: distance to the target at or below
        // the agent's previous stopping distance plus 0.3 is an arrival, else
        // NoneRoute. The route starts, then ends without arriving. A started
        // walk keeps its `_moveStatus` (`Success`, its initializer) until its
        // mover reports a failure; `MoveToTargetCoreAsync` writes the status
        // and clears `_isMoving`.
        for (target, want) in [
            ([0.3, 0.0, 0.0], AvatarMoveStatus::Success),
            ([3.0, 0.0, 4.0], AvatarMoveStatus::NoneRoute),
        ] {
            let mut state = MoveToFixtureUpper::enter(
                Some(1),
                Some(target),
                Some(target),
                0.1,
                RUN,
                [0.0; 3],
            );
            assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
            state.update(0.0, poll([0.0; 3], 5.0, true));
            assert!(state.is_complete());
            assert_eq!(state.result().move_status, want);
        }
        // No route at all: the mover's invalid-path branch reaches the state
        // on its first update, where the character stands; past the previous
        // stopping distance plus 0.3 that is NoneRoute.
        let mut state = MoveToFixtureUpper::enter(
            Some(1),
            Some([3.0, 0.0, 4.0]),
            Some([3.0, 0.0, 4.0]),
            0.1,
            WalkStart::NoRoute,
            [0.0; 3],
        );
        assert!(
            state.moving(),
            "the route the host could not start still sets OnEnter's flag"
        );
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(0.0, poll([0.0; 3], 0.0, false));
        assert!(state.is_complete());
        assert!(!state.moving());
        assert_eq!(state.result().move_status, AvatarMoveStatus::NoneRoute);
        assert_eq!(state.result().arrival, [0.0; 3]);
        // The agent's own failure is InValid, not NoneRoute.
        let state = MoveToFixtureUpper::enter(
            Some(1),
            Some([3.0, 0.0, 4.0]),
            Some([3.0, 0.0, 4.0]),
            0.1,
            WalkStart::NoAgent,
            [0.0; 3],
        );
        assert_eq!(state.result().move_status, AvatarMoveStatus::InValid);
    }

    #[test]
    fn a_position_state_honours_the_none_route_tolerance() {
        // No route, target out of reach, strict: `OnEnter` sets `_isMoving`
        // and the status stays `Success` until the mover's invalid-path branch
        // runs on the first update; the target is past the stopping distance
        // plus 0.3, so the branch writes NoneRoute.
        let mut state =
            MoveToPositionUpper::enter([3.0, 0.0, 4.0], false, WalkStart::NoRoute, [0.0; 3]);
        assert!(state.moving());
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(0.0, poll([0.0; 3], 0.0, false));
        assert!(state.is_complete());
        assert_eq!(state.result().move_status, AvatarMoveStatus::NoneRoute);
        assert_eq!(state.result().arrival, [0.0; 3]);
        // A route that ends without arriving keeps the same status.
        let mut state = MoveToPositionUpper::enter(
            [3.0, 0.0, 4.0],
            false,
            WalkStart::Running {
                stopping_distance: 0.1,
            },
            [0.0; 3],
        );
        state.update(0.0, poll([0.0; 3], 0.0, true));
        assert!(state.is_complete());
        assert_eq!(state.result().move_status, AvatarMoveStatus::NoneRoute);
        // No route at all, tolerant: the state ends where the character
        // stands and keeps `Success`.
        let mut state =
            MoveToPositionUpper::enter([3.0, 0.0, 4.0], true, WalkStart::NoRoute, [0.0; 3]);
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(0.0, poll([0.0; 3], 0.0, false));
        assert!(state.is_complete());
        assert_eq!(state.result().arrival, [0.0; 3]);
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        // Within the stopping distance (the agent's, given to the mover) plus
        // 0.3 the strict state arrives too; past it it fails.
        let mut state = MoveToPositionUpper::enter(
            [0.39, 0.0, 0.0],
            false,
            WalkStart::Running {
                stopping_distance: 0.1,
            },
            [0.0; 3],
        );
        state.update(0.0, poll([0.0; 3], 0.5, true));
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        let mut state = MoveToPositionUpper::enter(
            [0.41, 0.0, 0.0],
            false,
            WalkStart::Running {
                stopping_distance: 0.1,
            },
            [0.0; 3],
        );
        state.update(0.0, poll([0.0; 3], 0.5, true));
        assert_eq!(state.result().move_status, AvatarMoveStatus::NoneRoute);
    }

    #[test]
    fn a_position_state_without_agent_is_invalid() {
        let mut state =
            MoveToPositionUpper::enter([1.0, 0.0, 0.0], true, WalkStart::NoAgent, [0.0; 3]);
        assert_eq!(state.result().move_status, AvatarMoveStatus::InValid);
        state.update(0.0, poll([0.0; 3], 0.0, false));
        assert!(state.is_complete());
    }

    #[test]
    fn the_target_moves_snapshot_their_target_and_fail_without_one() {
        // `MoveToNPCUpperState.OnEnter` sets `_isMoving = 1` before the mover
        // starts; `MoveToNPCCoreAsync` polls `remainingDistance <=
        // stoppingDistance` and settles the status. `_moveStatus` starts at its
        // field initializer `Success` and the mover writes nothing while it
        // runs.
        let mut state = MoveToTargetUpper::npc(Some([6.0, 0.0, 5.0]), RUN, [0.0; 3]);
        assert_eq!(state.target, [6.0, 0.0, 5.0]);
        assert_eq!(state.result(Some(9)).target_npc, Some(9));
        state.update(1.0 / 60.0, poll([0.0; 3], 8.0, false));
        assert!(state.moving());
        assert_eq!(state.result(Some(9)).move_status, AvatarMoveStatus::Success);
        state.update(1.0 / 60.0, poll([6.0, 0.0, 5.0], 0.0, false));
        assert!(state.is_complete());
        assert_eq!(state.result(Some(9)).move_status, AvatarMoveStatus::Success);
        // No target: InValid, the arrival at the presenter's position.
        let mut state = MoveToTargetUpper::player(None, RUN, [1.0, 2.0, 3.0]);
        assert_eq!(state.result(None).move_status, AvatarMoveStatus::InValid);
        assert_eq!(state.result(None).arrival, [1.0, 2.0, 3.0]);
        state.update(0.0, poll([1.0, 2.0, 3.0], 0.0, false));
        assert!(state.is_complete());
        // A player move reports no target character.
        assert_eq!(
            MoveToTargetUpper::player(Some([0.0; 3]), RUN, [0.0; 3])
                .result(Some(9))
                .target_npc,
            None
        );
    }

    #[test]
    fn a_target_move_without_route_is_none_route() {
        // The invalid-path branch runs on the first update; the enter keeps
        // `OnEnter`'s flag and the initializer's status.
        let mut state =
            MoveToTargetUpper::npc(Some([3.0, 0.0, 4.0]), WalkStart::NoRoute, [0.0; 3]);
        assert!(state.moving());
        assert_eq!(state.result(None).move_status, AvatarMoveStatus::Success);
        state.update(0.0, poll([0.0; 3], 0.0, false));
        assert!(state.is_complete());
        assert_eq!(state.result(None).move_status, AvatarMoveStatus::NoneRoute);
        let state = MoveToTargetUpper::npc(Some([3.0, 0.0, 4.0]), WalkStart::NoAgent, [0.0; 3]);
        assert_eq!(state.result(None).move_status, AvatarMoveStatus::InValid);
    }

    #[test]
    fn the_random_walk_succeeds_even_when_the_walk_cannot_run() {
        // GetResult is always Success; an absent agent ends the state in the
        // enter, where the character stands, and a route the host could not
        // start ends it on the first update.
        let mut state = RandomWalkUpper::enter([2.0, 0.0, 2.0], WalkStart::NoAgent);
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(0.0, poll([2.0, 0.0, 2.0], f32::INFINITY, false));
        assert!(state.is_complete());
        assert_eq!(state.result().arrival, [2.0, 0.0, 2.0]);
        let mut state = RandomWalkUpper::enter([2.0, 0.0, 2.0], WalkStart::NoRoute);
        assert!(state.moving());
        state.update(0.0, poll([2.0, 0.0, 2.0], f32::INFINITY, false));
        assert!(state.is_complete());
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        assert_eq!(state.result().arrival, [2.0, 0.0, 2.0]);
        // A stopped route still reports Success at the character's place.
        let mut state = RandomWalkUpper::enter([5.0, 0.0, 5.0], RUN);
        state.update(0.0, poll([1.0, 0.0, 1.0], 5.0, true));
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        assert_eq!(state.result().arrival, [1.0, 0.0, 1.0]);
        // A running walk completes only once the remaining distance is within
        // the mover's stopping distance, where it stands.
        let mut state = RandomWalkUpper::enter(
            [6.0, 0.0, 0.0],
            WalkStart::Running {
                stopping_distance: 0.1,
            },
        );
        state.update(0.0, poll([5.8, 0.0, 0.0], 0.5, false));
        assert!(!state.is_complete());
        assert!(state.moving());
        state.update(0.0, poll([5.95, 0.0, 0.0], 0.05, false));
        assert!(state.is_complete());
        assert!(!state.moving());
        assert_eq!(state.result().arrival, [5.95, 0.0, 0.0]);
    }

    #[test]
    fn a_change_site_walk_stops_or_arrives() {
        // `ChangeSiteUpperState.OnEnter` sets `_isMoving = 1`; `OnUpdate` only
        // completes once it is clear (`ChangeSiteUpperState::OnUpdate`), so a
        // look that is still turning at the door keeps the initializer's
        // `Success`.
        assert!(UpperState::ChangeSite.can_interrupt());
        assert!(UpperState::RandomWalk.can_interrupt());
        let mut state = ChangeSiteUpper::enter([1.0, 0.0, 2.0], RUN);
        state.update(0.1, poll([0.0; 3], 2.0, false));
        assert_eq!(state.result().move_status, AvatarMoveStatus::Success);
        state.update(0.1, poll([0.5, 0.0, 1.0], 1.0, true));
        assert!(state.is_complete());
        assert_eq!(state.result().move_status, AvatarMoveStatus::NoneRoute);
    }

    #[test]
    fn rest_and_stay_need_their_first_update_to_complete() {
        // The upper base's OnUpdate alone: the running flag never sets, so
        // the first update completes the marker (the enter leaves it open).
        for (marker, kind) in [
            (RestUpper::rest(), UpperType::Rest),
            (RestUpper::stay(), UpperType::Stay),
        ] {
            let mut marker = marker;
            assert!(!marker.is_complete(), "not complete before the entry");
            marker.enter();
            assert!(!marker.is_complete(), "the entry does not complete");
            marker.update(1.0 / 60.0);
            assert!(marker.is_complete(), "one update completes");
            let result = marker.result([1.5, 2.0, 3.5]);
            assert_eq!(result.state, kind);
            assert_eq!(result.arrival, [1.5, 2.0, 3.5]);
            assert_eq!(result.move_status, AvatarMoveStatus::Success);
            assert_eq!((result.target_fixture, result.target_npc), (None, None));
            assert!(!result.is_move_failed());
            marker.exit();
        }
        assert_eq!(RestUpper::rest().state(), UpperState::Rest);
        assert_eq!(RestUpper::stay().state(), UpperState::Stay);
    }

    #[test]
    fn the_upper_base_keeps_the_frame_time() {
        // Update() adds Time.deltaTime before OnUpdate; a state with a mover
        // stays incomplete while the flag is set.
        let mut base = UpperBase::default();
        let mut entry = base.enter(true);
        assert!(!entry.is_complete);
        assert!(entry.is_moving);
        entry.update(0.5);
        entry.update(0.25);
        assert_eq!(entry.elapsed, 0.75);
        assert!(!entry.is_complete);
        assert!(entry.can_interrupt());
        entry.is_moving = false;
        entry.update(0.0);
        assert!(entry.is_complete);
        assert!(!UpperState::Rest.has_mover());
        assert!(!UpperState::Stay.has_mover());
        assert!(UpperState::MoveToPosition.has_mover());
    }

    #[test]
    fn empty_and_non_positive_lists_return_random_walk_without_a_draw() {
        let mut draws = 0;
        let mut draw = |_total: f32| {
            draws += 1;
            0.5
        };
        for weights in [
            vec![],
            vec![entry(UpperType::Rest, 0.0)],
            vec![entry(UpperType::Rest, 0.0), entry(UpperType::Stay, 0.0)],
            vec![entry(UpperType::Rest, -1.0)],
        ] {
            let choice = decide_upper(&weights, &mut draw);
            assert_eq!(choice.state, UpperType::RandomWalk);
            assert_eq!(choice.draw, None);
        }
        assert_eq!(draws, 0);
    }

    #[test]
    fn one_draw_over_the_sum_and_the_first_entry_at_or_above_it() {
        // The tutorial's three weights: 5, 5, 2 over 12.
        let weights = [
            entry(UpperType::RandomWalk, 5.0),
            entry(UpperType::MoveToFixture, 5.0),
            entry(UpperType::Rest, 2.0),
        ];
        for (drawn, want) in [
            (0.0, UpperType::RandomWalk),
            (4.999, UpperType::RandomWalk),
            (5.0, UpperType::RandomWalk),
            (5.5, UpperType::MoveToFixture),
            (9.999, UpperType::MoveToFixture),
            (10.0, UpperType::MoveToFixture),
            (10.5, UpperType::Rest),
            (11.999, UpperType::Rest),
        ] {
            let mut calls = 0;
            let choice = decide_upper(&weights, &mut |_total| {
                calls += 1;
                drawn
            });
            assert_eq!(choice.state, want, "draw {drawn}");
            assert_eq!(choice.draw, Some(drawn));
            assert_eq!(choice.total, 12.0);
            assert!(choice.entry.is_some(), "draw {drawn} lands on an entry");
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn zero_weight_entries_can_only_be_taken_at_an_exact_boundary() {
        // A zero-weight entry is taken only when the draw equals the running
        // sum before it (the scan accepts `r <= acc`).
        let weights = [entry(UpperType::Rest, 0.0), entry(UpperType::Stay, 3.0)];
        assert_eq!(decide_upper(&weights, &mut |_| 0.0).state, UpperType::Rest);
        assert_eq!(decide_upper(&weights, &mut |_| 0.5).state, UpperType::Stay);
    }

    #[test]
    fn a_draw_past_the_sum_falls_to_the_last_entry() {
        // The source scan leaves by `r <= acc` on every entry and answers the
        // last entry after the loop.
        let weights = [entry(UpperType::RandomWalk, 5.0), entry(UpperType::Rest, 2.0)];
        let choice = decide_upper(&weights, &mut |_| 7.5);
        assert_eq!(choice.state, UpperType::Rest);
        assert_eq!(choice.entry, None);
    }

    #[test]
    fn set_upper_weight_replaces_in_place_or_appends() {
        // The source scans for the same state type and overwrites its
        // weight; a new type is appended (the order is kept).
        let mut weights = vec![
            entry(UpperType::RandomWalk, 5.0),
            entry(UpperType::Rest, 2.0),
        ];
        set_upper_weight(&mut weights, UpperType::RandomWalk, 1.0);
        assert_eq!(
            weights,
            vec![
                entry(UpperType::RandomWalk, 1.0),
                entry(UpperType::Rest, 2.0)
            ]
        );
        set_upper_weight(&mut weights, UpperType::MoveToFixture, 5.0);
        set_upper_weight(&mut weights, UpperType::Rest, 0.0);
        assert_eq!(
            weights,
            vec![
                entry(UpperType::RandomWalk, 1.0),
                entry(UpperType::Rest, 0.0),
                entry(UpperType::MoveToFixture, 5.0),
            ]
        );
    }
}
