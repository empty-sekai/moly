//! Actors: where each one stands and faces, and what it shows, at any time.
//!
//! An actor stands at its cast position until its first move, and keeps the
//! place and facing each move ends with until the next. A position never
//! depends on a facing, so facing toward another actor reads only positions
//! and cannot loop.

use bevy_math::Vec3;

use crate::doc::MotionMode;
use crate::ease::Ease;
use crate::lane::Lane;
use crate::sample::{ActorFrame, Host, Look, MotionFrame, Moving, Point};
use crate::spline::Spline;
use crate::Compiled;

#[derive(Clone, Debug)]
pub(crate) struct Actor {
    pub id: String,
    pub unit: String,
    pub at: Vec3,
    pub yaw: f32,
    /// Ordered and not overlapping.
    pub moves: Vec<Move>,
    pub motion: Lane<Motion>,
    pub face: Lane<Face>,
    pub look: Lane<Point>,
}

#[derive(Clone, Debug)]
pub(crate) struct Move {
    pub start: f32,
    pub end: f32,
    /// From where the actor stood before the move, through its path.
    pub spline: Spline,
    pub facing: Facing,
    pub gait: Option<String>,
    pub ease: Ease,
    pub turn: f32,
    /// The yaw before the move, and the yaw it ends with.
    pub yaw_from: f32,
    pub yaw_end: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum Facing {
    Path,
    Keep,
    Yaw(f32),
    Toward(Point),
}

#[derive(Clone, Debug)]
pub(crate) struct Motion {
    pub name: String,
    pub mode: MotionMode,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct Face {
    pub eye: Option<String>,
    pub mouth: Option<String>,
}

/// The yaw in degrees of a direction on the ground; `None` when it has none.
fn yaw_of(direction: Vec3) -> Option<f32> {
    let flat = Vec3::new(direction.x, 0.0, direction.z);
    (flat.length_squared() > 1e-10).then(|| flat.x.atan2(flat.z).to_degrees())
}

/// `from` turned `share` of the short way round toward `to`.
fn turn_toward(from: f32, to: f32, share: f32) -> f32 {
    let delta = (to - from + 180.0).rem_euclid(360.0) - 180.0;
    from + delta * share
}

impl Move {
    fn progress(&self, t: f32) -> f32 {
        ((t - self.start) / (self.end - self.start)).clamp(0.0, 1.0)
    }
}

impl Compiled {
    /// The move running or most recently finished at `t`.
    fn last_move(&self, actor: usize, t: f32) -> Option<&Move> {
        let moves = &self.actors[actor].moves;
        moves.partition_point(|m| m.start <= t).checked_sub(1).map(|index| &moves[index])
    }

    pub(crate) fn actor_position(&self, actor: usize, t: f32) -> Vec3 {
        match self.last_move(actor, t) {
            Some(m) => m.spline.at(m.ease.apply(m.progress(t))),
            None => self.actors[actor].at,
        }
    }

    pub(crate) fn actor_yaw(&self, actor: usize, t: f32) -> f32 {
        match self.last_move(actor, t) {
            Some(m) if t >= m.end => m.yaw_end,
            Some(m) => self.yaw_during(m, t),
            None => self.actors[actor].yaw,
        }
    }

    /// The yaw of `actor` at `t` inside move `m`, from the move's own rule.
    pub(crate) fn yaw_during(&self, m: &Move, t: f32) -> f32 {
        let along = m.ease.apply(m.progress(t));
        let wanted = match &m.facing {
            Facing::Path => yaw_of(m.spline.direction(along)),
            Facing::Keep => None,
            Facing::Yaw(yaw) => Some(*yaw),
            Facing::Toward(target) => {
                let here = m.spline.at(along);
                yaw_of(self.ground_point(target, t) - here)
            }
        };
        let Some(wanted) = wanted else {
            return m.yaw_from;
        };
        let share = if m.turn > 0.0 { Ease::Smooth.apply((t - m.start) / m.turn) } else { 1.0 };
        turn_toward(m.yaw_from, wanted, share)
    }

    /// A point without its height on the body, which a facing does not need.
    fn ground_point(&self, point: &Point, t: f32) -> Vec3 {
        match point {
            Point::Fixed(position) => *position,
            Point::Actor { index, offset, .. } => self.actor_position(*index, t) + *offset,
        }
    }

    /// A target point at `t`.
    pub(crate) fn point(&self, point: &Point, t: f32, host: &dyn Host) -> Vec3 {
        match point {
            Point::Fixed(position) => *position,
            Point::Actor { index, part, offset } => {
                let height = part.map_or(0.0, |part| host.part_height(&self.actors[*index].unit, part));
                self.actor_position(*index, t) + Vec3::Y * height + *offset
            }
        }
    }

    /// Works out where each move starts facing and how it ends facing,
    /// in time order per actor.
    pub(crate) fn settle_facings(&mut self) {
        for actor in 0..self.actors.len() {
            for index in 0..self.actors[actor].moves.len() {
                let from = match index.checked_sub(1) {
                    Some(previous) => self.actors[actor].moves[previous].yaw_end,
                    None => self.actors[actor].yaw,
                };
                self.actors[actor].moves[index].yaw_from = from;
                let m = &self.actors[actor].moves[index];
                let end = self.yaw_during(m, m.end);
                self.actors[actor].moves[index].yaw_end = end;
            }
        }
    }

    /// Every actor's state at `t`.
    pub fn actors(&self, t: f32, host: &dyn Host) -> Vec<ActorFrame<'_>> {
        (0..self.actors.len()).map(|index| self.actor(index, t, host)).collect()
    }

    fn actor(&self, index: usize, t: f32, host: &dyn Host) -> ActorFrame<'_> {
        let actor = &self.actors[index];
        let moving = self.last_move(index, t).filter(|m| t < m.end && m.spline.length() > 1e-4).map(|m| {
            let p = m.progress(t);
            let (a, b) = ((p - 1e-3).max(0.0), (p + 1e-3).min(1.0));
            let rate = (m.ease.apply(b) - m.ease.apply(a)) / (b - a);
            Moving { gait: m.gait.as_deref(), speed: rate * m.spline.length() / (m.end - m.start) }
        });
        let motions = actor
            .motion
            .layered(t)
            .into_iter()
            .map(|(span, presence, share)| MotionFrame {
                name: &span.payload.name,
                local: presence.local,
                weight: share * span.payload.weight,
                mode: span.payload.mode,
            })
            .collect();
        let (mut eye, mut mouth) = (None, None);
        for (span, _) in actor.face.present(t) {
            eye = span.payload.eye.as_deref().or(eye);
            mouth = span.payload.mouth.as_deref().or(mouth);
        }
        let mut look: Option<Look> = None;
        for (span, presence) in actor.look.present(t) {
            let target = self.point(&span.payload, t, host);
            let w = presence.weight;
            look = Some(match look {
                None => Look { target, weight: w },
                Some(below) => Look { target: below.target.lerp(target, w), weight: below.weight * (1.0 - w) + w },
            });
        }
        ActorFrame {
            id: &actor.id,
            unit: &actor.unit,
            position: self.actor_position(index, t),
            yaw: self.actor_yaw(index, t),
            moving,
            motions,
            eye,
            mouth,
            look,
        }
    }
}
