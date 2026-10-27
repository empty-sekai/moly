//! The navigation of a companion (`crate::companion`): a walker that is not
//! a roster member and runs no objective machine. It stands, walks and
//! arrives on the roster's own walk cells, waypoint table and crowd agent:
//! [`depart`] builds the route as an objective's `MoveAsync` does, and
//! [`advance`] takes one frame of the agent step's loop body.
//!
//! What the roster keeps on its members, the companion keeps on its own
//! walk runtime: the stuck watchdog, and the outcome of this frame's step.
//! Its state machine is not ticked and its route has check points only.

use super::*;

/// What the agent step did with a companion's route this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Walking,
    /// The move completed within its goal distance of the target.
    Arrived,
    /// The route ran out, had no corridor, or the companion stopped moving:
    /// the walk ended where it stands.
    Stopped,
}

/// Why a walk ended `Stopped`, as the companion's status names it.
pub const WALK_STOPPED: &str = "the walk stopped on the navigation before the target";

/// The stuck watchdog: the last position that moved, and the elapsed seconds
/// the window is measured on.
#[derive(Clone, Copy)]
pub struct Watch {
    baseline: Option<([f32; 3], f32)>,
    now: f32,
}

impl Watch {
    /// The watchdog as of `now` seconds of the roster clock.
    pub fn at(now: f32) -> Self {
        Self {
            baseline: None,
            now,
        }
    }

    /// The same watchdog, read at `now` seconds of the roster clock.
    pub fn at_time(self, now: f32) -> Self {
        Self { now, ..self }
    }

    /// One frame at `position`: `true` once it stayed within
    /// [`STUCK_DISTANCE`] of the baseline for longer than [`STUCK_SECONDS`].
    /// The baseline restarts on every move and after the report.
    fn update(&mut self, position: [f32; 3]) -> bool {
        match self.baseline {
            None => {
                self.baseline = Some((position, self.now));
                false
            }
            Some((last, since)) => {
                if Vec3::from(position).distance(Vec3::from(last)) >= STUCK_DISTANCE {
                    self.baseline = Some((position, self.now));
                    false
                } else if self.now - since > STUCK_SECONDS {
                    self.baseline = None;
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// Builds the route of the companion of `unit` to `corner` from where it
/// stands: the walk cells' path, the waypoint table and the first leg. The
/// move completes within `goal_distance` of `corner`. A refusal is named.
/// The walk's speed is the agent step's ([`advance`]).
#[allow(clippy::too_many_arguments)]
pub fn depart(
    unit: u32,
    state: &mut WalkState,
    slot: &mut PathSlot,
    route: &mut RouteStops,
    walk_face: &crate::walk_face::WalkFace,
    objective_face: &crate::npc_objective::ObjectiveFace,
    corner: [f32; 3],
    goal_distance: f32,
) -> Result<MotionPhase, &'static str> {
    let mut at = [state.0.position[0], state.0.position[2]];
    // The source's agent stands on its navigation mesh at every frame; the
    // agent step can end a walk just beside the walk cells, and the walk
    // starts from the nearest cell, as the agent would.
    if !walk_face.on_face(at[0], at[1]) {
        let (x, z) = walk_face.seat(at[0], at[1]);
        if !walk_face.on_face(x, z) {
            return Err("the companion stands off the walk cells");
        }
        at = [x, z];
        state.0.position[0] = x;
        state.0.position[2] = z;
    }
    if objective_face.navigation_point_at([corner[0], corner[2]]).is_none() {
        return Err("the target is off the navigation cells");
    }
    if walk_face.path(at, [corner[0], corner[2]]).is_none() {
        return Err("the navigation has no path to the target");
    }
    let unit = CharacterUnitId(unit);
    let mut rng = crate::npc_objective::MemberRng::from_platform();
    let phase = depart_toward(
        &unit,
        &mut state.0,
        &mut slot.0,
        route,
        walk_face,
        objective_face,
        corner,
        None,
        &mut rng,
        goal_distance,
    )
    .ok_or("the route has no waypoint beyond the companion")?;
    Ok(phase)
}

/// One frame of the companion's route with the roster's agent step: the
/// corridor move, the walker's turn, the check points and the move's
/// outcome. `phase`, `transform` and `watch` are written.
#[allow(clippy::too_many_arguments)]
pub fn advance(
    unit: u32,
    speed: f32,
    state: &mut WalkState,
    slot: &mut PathSlot,
    route: &mut RouteStops,
    watch: &mut Watch,
    transform: &mut Transform,
    phase: &mut MotionPhase,
    walk_face: &crate::walk_face::WalkFace,
    objective_face: &crate::npc_objective::ObjectiveFace,
    dt: f32,
) -> Motion {
    let unit = CharacterUnitId(unit);
    if !matches!(*phase, MotionPhase::Walking) {
        route.crowd.velocity = [0.0; 3];
    }
    // The leg's target is the destination submitted, not the corridor's end:
    // a partial corridor's end is no arrival.
    let leg_arrival = if matches!(*phase, MotionPhase::Walking) && !slot.0.is_empty() {
        route
            .navigation_destination
            .map(|target| leg_distance(state.0.position, target))
            .filter(|distance| *distance < ARRIVAL_DISTANCE)
    } else {
        None
    };
    let mut verdict = WalkVerdict::Idle;
    if leg_arrival.is_some() {
        if route
            .stops
            .get(route.next)
            .is_some_and(|point| point.kind == WaypointKind::CheckPoint)
        {
            slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
            state.0.next_corner = 0;
            watch.baseline = None;
            *phase = next_waypoint_or_stop(
                &unit,
                &mut state.0,
                &mut slot.0,
                route,
                walk_face,
                objective_face,
            );
        } else {
            verdict = WalkVerdict::Arrived(leg_arrival.unwrap_or(0.0));
        }
    }
    let completed = route.completed(state.0.position);
    if !completed
        && matches!(*phase, MotionPhase::Walking)
        && !matches!(verdict, WalkVerdict::Arrived(_))
    {
        if route.agent_stopped {
            route.crowd.velocity = [0.0; 3];
            verdict = WalkVerdict::Walking(0.0);
        } else {
            if !std::mem::take(&mut route.crowd.fresh_leg) {
                if let Some(remaining) = route.crowd.remaining {
                    route.crowd.auto_braking =
                        remaining < moly_law::path::crowd::NPC_AUTO_BRAKING_DISTANCE;
                }
            }
            let mut turned = transform.rotation;
            match crowd_step(
                &mut state.0,
                &mut route.crowd,
                speed,
                moly_law::path::crowd::NPC_STOPPING_DISTANCE,
                walk_face,
                objective_face,
                &mut turned,
                dt,
            ) {
                Some(remaining) => {
                    verdict = WalkVerdict::Walking(remaining);
                    transform.rotation = turned;
                }
                None => {
                    route.stop();
                    *phase = MotionPhase::Dwelling { remaining: None };
                    transform.translation = Vec3::from(state.0.position);
                    return Motion::Stopped;
                }
            }
        }
    }
    match verdict {
        _ if completed => {
            watch.baseline = None;
            *phase = complete_move(&unit, &mut state.0, &mut slot.0, route);
        }
        WalkVerdict::Walking(_) => {
            *phase = MotionPhase::Walking;
            if watch.update(state.0.position) {
                // Source IsPositionStacked: the move ends as stopped.
                route.stop();
                route.stalled = true;
                *phase = MotionPhase::Dwelling { remaining: None };
                transform.translation = Vec3::from(state.0.position);
                return Motion::Stopped;
            }
        }
        WalkVerdict::Arrived(distance) => {
            // The last check point: the route's own goal still completes the
            // move.
            debug!(
                "[npc unit={}] companion check point {}/{}, {distance:.3}m from its target",
                unit.0,
                route.next + 1,
                route.stops.len()
            );
            route.next += 1;
            slot.0 = NpcPathWalkSlot::from_corners(Vec::new());
            state.0.next_corner = 0;
            *phase = next_waypoint_or_stop(
                &unit,
                &mut state.0,
                &mut slot.0,
                route,
                walk_face,
                objective_face,
            );
        }
        // Past the waypoints with a completion still to come: the move waits
        // on the stuck watchdog.
        WalkVerdict::Idle
            if matches!(*phase, MotionPhase::Dwelling { remaining: None })
                && route.stalling
                && watch.update(state.0.position) =>
        {
            route.stop();
            route.stalled = true;
            transform.translation = Vec3::from(state.0.position);
            return Motion::Stopped;
        }
        WalkVerdict::Idle => {}
        WalkVerdict::Refused => warn!(
            "[npc unit={}] the walk law refused this frame (speed {speed:.3}, dt {dt:.3})",
            unit.0
        ),
    }
    transform.translation = Vec3::from(state.0.position);
    if matches!(*phase, MotionPhase::Dwelling { remaining: None }) && !route.stalling {
        return Motion::Arrived;
    }
    Motion::Walking
}
