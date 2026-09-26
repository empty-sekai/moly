//! One NPC navigation agent's per-frame crowd step: steering, integration and
//! the path-length readings the move loop polls.
//!
//! The NPC hands its agent a destination per leg; the engine's crowd update
//! then moves the agent every frame, in this order: find the corners of the
//! agent's corridor and its known path length, steer (a desired velocity
//! toward the first corner at the agent's speed, zeroed inside the stopping
//! distance, snapped to the path end when the next step would pass it, braked
//! near the end when auto braking is on), integrate (the position first takes
//! a step with the velocity it had, then the velocity turns toward the
//! desired one by at most acceleration × dt), and move the result along the
//! navigation cells (the corridor move in `carve`). The move loop reads the
//! known path length back as the agent's remaining distance.
//!
//! Every comparison and every operation order here follows the engine body;
//! the float order matters because the answers are compared bit for bit.
//!
//! Obstacle avoidance and separation do not apply to a shown NPC: its
//! presenter's update sets the agent's avoidance type to none every frame
//! (the view raises it again only in photo mode), and the crowd then takes the
//! desired velocity as the new one and collects no neighbours, so the
//! separation term has nothing to add. The avoidance sampling itself is not
//! ported.

/// The NPC agent's radius (the view's agent setup; world units).
pub const NPC_AGENT_RADIUS: f32 = 0.3;

/// The NPC agent's height (the view's agent setup).
pub const NPC_AGENT_HEIGHT: f32 = 0.98;

/// The NPC agent's stopping distance (the view's agent setup).
pub const NPC_STOPPING_DISTANCE: f32 = 0.1;

/// The acceleration each leg's move setup writes before it hands the leg on.
pub const NPC_MOVE_ACCELERATION: f32 = 8.0;

/// The move loop turns auto braking on for a frame whenever the agent's
/// remaining distance is below this (and off otherwise); the agent setup
/// starts with it off.
pub const NPC_AUTO_BRAKING_DISTANCE: f32 = 0.3;

/// The crowd's small distance literal (f32 bits `0x38d1b717`): the squared
/// horizontal distance under which a leading corner is dropped, and the slack
/// of the braking arrival test.
pub const CROWD_EPSILON: f32 = 9.99999975e-05;

/// Corners the crowd asks the corridor for each frame (the start point
/// included, so at most three remain once it is dropped).
pub const FIND_CORNERS_MAX: usize = 4;

/// `PathCorridor::FindCorners` on a corridor's straight path (its start
/// point first, its target last): the path cut to [`FIND_CORNERS_MAX`]
/// points, the target flagged as the end only when it survives the cut, and
/// the leading points within the crowd's small horizontal distance of
/// `position` dropped. Returns the corners and whether the last is the end.
pub fn find_corners(position: [f32; 2], straight: &[[f32; 2]]) -> (Vec<[f32; 2]>, bool) {
    let limit = straight.len().min(FIND_CORNERS_MAX);
    let last_is_end = limit == straight.len();
    let kept = &straight[..limit];
    let first = kept
        .iter()
        .position(|corner| {
            let dx = position[0] - corner[0];
            let dz = position[1] - corner[1];
            (dx * dx + 0.0) + dz * dz > CROWD_EPSILON
        })
        .unwrap_or(kept.len());
    (kept[first..].to_vec(), last_is_end)
}

/// The agent parameters the crowd reads while steering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgentParams {
    pub radius: f32,
    pub max_speed: f32,
    pub acceleration: f32,
    pub stopping_distance: f32,
    pub auto_braking: bool,
    /// The agent's stop flag: the desired speed is zero.
    pub halted: bool,
}

/// Where the agent is heading this frame: the corridor's corners (at most
/// [`FIND_CORNERS_MAX`] minus the dropped start), whether the last of them is
/// the path end, the corridor's target and its cell count.
#[derive(Debug, Clone, Copy)]
pub struct Goal<'a> {
    pub corners: &'a [[f32; 3]],
    pub last_is_end: bool,
    pub target: [f32; 3],
    pub path_cells: usize,
}

fn length3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    ((dx * dx + dy * dy) + dz * dz).sqrt()
}

/// The distance from `npos` to the path end, at most `range`: to the target
/// when there is no corner, to the last corner when it is the end, and
/// `range` itself when the end is beyond the corners.
pub fn distance_to_goal(npos: [f32; 3], goal: &Goal, range: f32) -> f32 {
    let end = match goal.corners.last() {
        None => goal.target,
        Some(last) if goal.last_is_end => *last,
        Some(_) => return range,
    };
    let distance = length3(end, npos);
    if distance > range {
        range
    } else {
        distance
    }
}

/// The known path length from `pos`: the straight distance to the target
/// when there is no corner, the corner polyline's length when its last corner
/// is the end, and infinity otherwise.
pub fn known_path_length(pos: [f32; 3], goal: &Goal) -> f32 {
    if goal.corners.is_empty() {
        return length3(goal.target, pos);
    }
    if !goal.last_is_end {
        return f32::INFINITY;
    }
    let mut total = 0.0f32;
    let mut previous = pos;
    for &corner in goal.corners {
        total += length3(previous, corner);
        previous = corner;
    }
    total
}

/// One frame's steering answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steering {
    /// The desired velocity.
    pub dvel: [f32; 3],
    /// The desired speed it was scaled by.
    pub desired_speed: f32,
    /// The agent arrives this frame: it is put at the corridor's target and
    /// its velocity is zeroed before integration.
    pub arrived: bool,
    /// Arrival or braking set the crowd's braking flag: integration then
    /// follows the desired velocity directly when it is not faster.
    pub braking: bool,
}

/// The crowd's steering for an agent at `npos` moving with `vel`, `known`
/// being the known path length the corner search just gave.
pub fn steer(
    params: &AgentParams,
    npos: [f32; 3],
    vel: [f32; 3],
    goal: &Goal,
    known: f32,
    dt: f32,
) -> Steering {
    let aim = match goal.corners.first() {
        Some(corner) => Some(*corner),
        None if goal.path_cells >= 1 => Some(goal.target),
        None => None,
    };
    let mut dir = [0.0f32; 3];
    if let Some(aim) = aim {
        let d = [aim[0] - npos[0], aim[1] - npos[1], aim[2] - npos[2]];
        let len2 = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
        dir = d;
        if len2 != 0.0 {
            let inverse = 1.0 / len2.sqrt();
            dir = [d[0] * inverse, d[1] * inverse, d[2] * inverse];
        }
    }
    let mut speed = if params.halted { 0.0 } else { params.max_speed };
    if known < params.stopping_distance && (!goal.corners.is_empty() || goal.path_cells >= 2) {
        speed = 0.0;
    }
    let speed2 = (vel[0] * vel[0] + vel[1] * vel[1]) + vel[2] * vel[2];
    let current = speed2.sqrt();
    let step = dt * current;
    let mut arrived = false;
    let mut braking = false;
    if !params.auto_braking {
        if distance_to_goal(npos, goal, step) < step {
            arrived = true;
        }
    } else {
        let reach = params.radius + params.radius;
        let distance = distance_to_goal(npos, goal, reach);
        if distance < step + CROWD_EPSILON && distance < reach {
            arrived = true;
        } else {
            if reach < step && distance_to_goal(npos, goal, step) < step {
                arrived = true;
            }
            if !arrived && distance < reach && distance * params.max_speed < current * reach {
                speed = current - (dt * speed2) / (distance + distance);
                braking = true;
            }
        }
    }
    if arrived {
        speed = 0.0;
        braking = true;
    }
    Steering {
        dvel: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
        desired_speed: speed,
        arrived,
        braking,
    }
}

/// `vel` turned toward `toward` by at most `max_delta`.
fn approach(vel: [f32; 3], toward: [f32; 3], max_delta: f32) -> [f32; 3] {
    let dv = [toward[0] - vel[0], toward[1] - vel[1], toward[2] - vel[2]];
    let d2 = (dv[0] * dv[0] + dv[1] * dv[1]) + dv[2] * dv[2];
    if !(d2 <= max_delta * max_delta) {
        let scale = max_delta / d2.sqrt();
        [
            vel[0] + dv[0] * scale,
            vel[1] + dv[1] * scale,
            vel[2] + dv[2] * scale,
        ]
    } else {
        toward
    }
}

/// The crowd's integration: the position steps with the velocity it had,
/// then the velocity turns toward the new one (`nvel`, the desired velocity
/// here) by at most `dt × acceleration`. Under the braking flag a desired
/// velocity no faster than the current one is taken at once. Returns the new
/// position and velocity.
pub fn integrate(
    npos: [f32; 3],
    vel: [f32; 3],
    nvel: [f32; 3],
    dvel: [f32; 3],
    braking: bool,
    acceleration: f32,
    dt: f32,
) -> ([f32; 3], [f32; 3]) {
    let moved = [
        dt * vel[0] + npos[0],
        dt * vel[1] + npos[1],
        dt * vel[2] + npos[2],
    ];
    let max_delta = dt * acceleration;
    let next = if !braking {
        approach(vel, nvel, max_delta)
    } else {
        let desired2 = (dvel[0] * dvel[0] + dvel[1] * dvel[1]) + dvel[2] * dvel[2];
        let current2 = (vel[0] * vel[0] + vel[1] * vel[1]) + vel[2] * vel[2];
        if desired2 > current2 {
            approach(vel, dvel, max_delta)
        } else {
            dvel
        }
    };
    (moved, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_literals_are_the_engine_words() {
        assert_eq!(CROWD_EPSILON.to_bits(), 0x38d1_b717);
        assert_eq!(NPC_AGENT_RADIUS.to_bits(), 0x3e99_999a);
        assert_eq!(NPC_AGENT_HEIGHT.to_bits(), 0x3f7a_e148);
        assert_eq!(NPC_STOPPING_DISTANCE.to_bits(), 0x3dcc_cccd);
    }
}
