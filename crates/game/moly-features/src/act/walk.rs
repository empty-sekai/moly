//! The player's walk law: the corners planned on the walk field, replanned
//! when the goal moves, the field's generation changes or the walk stalls,
//! and the joystick steered at the next corner. Every step that walks the
//! player (to a place, to a button, to a gatherable, across the drops) walks
//! with it.

use bevy::prelude::*;

use super::targets;
use super::{ActPoll, Target};

/// A corner is passed when the player is this close (metres).
const CORNER_REACHED: f32 = 0.15;
/// A walk that has not come this much closer (metres) in [`STALL_SECS`] plans
/// its path again.
const STALL_PROGRESS: f32 = 0.05;
const STALL_SECS: f32 = 2.0;
/// A goal that moved this far (metres) is planned again.
const GOAL_MOVED: f32 = 0.25;

/// The player walking to a target along the navigation field.
#[derive(Debug)]
pub struct Walk {
    to: Target,
    within: f32,
    corners: Vec<Vec2>,
    next: usize,
    /// The goal and field generation the corners were planned for.
    planned: Option<(Vec2, u64)>,
    /// The closest the player has come, and when.
    best: (f32, f32),
    /// Whether a goal the walk field cannot reach within `within` refuses: a
    /// point walk (`player.walk_to`) wants a place to stand; an approach to a
    /// target's contact circle or a button, and a drop heading to cross, stop
    /// before their point and opt out ([`Walk::contact`]).
    refuse_unreachable: bool,
}

/// What one frame of a walk says: the same as a step's poll, from the walk's
/// point of view.
pub enum WalkPoll {
    Running(String),
    Done(String),
    Failed(String),
}

impl From<WalkPoll> for ActPoll {
    fn from(poll: WalkPoll) -> Self {
        match poll {
            WalkPoll::Running(note) => Self::Running(note),
            WalkPoll::Done(note) => Self::Done(note),
            WalkPoll::Failed(note) => Self::Failed(note),
        }
    }
}

impl Walk {
    pub fn new(to: Target, within: f32) -> Self {
        Self {
            to,
            within,
            corners: Vec::new(),
            next: 0,
            planned: None,
            best: (f32::INFINITY, 0.0),
            refuse_unreachable: true,
        }
    }

    /// A walk that ends on a contact circle or a button near the target, not
    /// on the target point: the field may snap the point outward and that is
    /// not a refusal.
    pub fn contact(to: Target, within: f32) -> Self {
        Self {
            refuse_unreachable: false,
            ..Self::new(to, within)
        }
    }

    /// The target the walk is aimed at.
    pub fn to(&self) -> &Target {
        &self.to
    }

    /// What the walk is aimed at.
    pub fn what(&self) -> &super::TargetKind {
        &self.to.what
    }

    /// The distance a walk ends within.
    pub fn within(&self) -> f32 {
        self.within
    }

    /// The distance a walk ends within, changed while it runs.
    pub fn set_within(&mut self, within: f32) {
        self.within = within;
    }

    /// One frame of the walk, for the step `path` of the run `ticket`; the
    /// joystick is lifted when the walk ends there.
    pub fn poll(&mut self, world: &mut World, ticket: u64, path: &str, now: f32) -> WalkPoll {
        let goal = match targets::resolve(world, &self.to) {
            Ok(goal) => goal,
            Err(error) => {
                release_hand_of(world, ticket, path);
                return WalkPoll::Running(error);
            }
        };
        let player = match targets::player(world) {
            Ok(player) => player,
            Err(error) => return WalkPoll::Running(error),
        };
        let distance = targets::flat(player, goal);
        if distance <= self.within {
            release_hand_of(world, ticket, path);
            return WalkPoll::Done(format!("arrived at {distance:.2} m"));
        }
        // The stick takes no press while it is disabled (the entry before
        // its finish, a dialog, an edit, a site move): stand and wait.
        let enabled = world
            .get_resource::<crate::joystick::JoystickState>()
            .is_some_and(|stick| stick.enabled);
        if !enabled {
            release_hand_of(world, ticket, path);
            return WalkPoll::Running(format!("the joystick is disabled; {distance:.2} m to go"));
        }
        if let Some((owner, _)) = super::hand::leaning(world).filter(|(owner, _)| *owner != ticket) {
            return WalkPoll::Failed(format!("the joystick finger is walking for {owner}"));
        }
        let goal_xz = Vec2::new(goal.x, goal.z);
        let player_xz = Vec2::new(player.x, player.z);
        let generation = world.get_resource::<crate::walk_face::WalkFace>().map_or(0, |face| face.generation());
        if distance < self.best.0 - STALL_PROGRESS {
            self.best = (distance, now);
        }
        let stalled = now - self.best.1 > STALL_SECS;
        let replan = match self.planned {
            None => true,
            Some((planned_goal, planned_generation)) => {
                planned_goal.distance(goal_xz) > GOAL_MOVED || planned_generation != generation || stalled
            }
        };
        if replan {
            let planned = world
                .get_resource::<crate::walk_face::WalkFace>()
                .map(|face| face.path(player_xz.into(), goal_xz.into()));
            match planned {
                // No walk field (a stage or a headless run): the straight line
                // stays the plan.
                None => self.corners = vec![goal_xz],
                Some(Some(corners)) => {
                    let corners: Vec<Vec2> = corners.into_iter().map(Vec2::from).collect();
                    if self.refuse_unreachable {
                        let reached = corners.last().copied().unwrap_or(goal_xz);
                        let off = reached.distance(goal_xz);
                        if off > self.within {
                            release_hand_of(world, ticket, path);
                            return WalkPoll::Failed(format!(
                                "the goal ({:.2}, {:.2}) is not reachable on the walk field: the nearest reachable point is ({:.2}, {:.2}), {off:.2} m away, more than within {:.2}",
                                goal_xz.x, goal_xz.y, reached.x, reached.y, self.within
                            ));
                        }
                    }
                    self.corners = corners;
                }
                Some(None) => {
                    if self.refuse_unreachable {
                        release_hand_of(world, ticket, path);
                        let nearest = world
                            .get_resource::<crate::walk_face::WalkFace>()
                            .and_then(|face| face.sample(goal_xz.into(), 1.0e4))
                            .map_or_else(
                                || "none".to_owned(),
                                |point| format!("({:.2}, {:.2})", point[0], point[1]),
                            );
                        return WalkPoll::Failed(format!(
                            "no walk path from ({:.2}, {:.2}) to the goal ({:.2}, {:.2}); the nearest walkable point to the goal is {nearest}",
                            player_xz.x, player_xz.y, goal_xz.x, goal_xz.y
                        ));
                    }
                    self.corners = vec![goal_xz];
                }
            }
            self.next = 0;
            self.planned = Some((goal_xz, generation));
            self.best = (distance, now);
        }
        while self.next < self.corners.len() && self.corners[self.next].distance(player_xz) < CORNER_REACHED {
            self.next += 1;
        }
        let aim = self.corners.get(self.next).copied().unwrap_or(goal_xz);
        if let Err(error) = super::hand::steer(world, ticket, path, aim - player_xz) {
            return WalkPoll::Running(error.to_owned());
        }
        WalkPoll::Running(format!(
            "{distance:.2} m to go, corner {}/{}",
            (self.next + 1).min(self.corners.len()),
            self.corners.len()
        ))
    }
}

/// Lifts the joystick finger when the step `path` of the run `ticket` holds
/// it.
pub fn release_hand_of(world: &mut World, ticket: u64, path: &str) {
    super::hand::release_of(world, ticket, path);
}
