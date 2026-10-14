//! The executor: tracks of steps advanced once a frame. A track runs its
//! steps in order; `parallel` runs child tracks side by side and `repeat`
//! runs a block again. A step that ends on the frame it starts (a log, a
//! take, an assert) lets the next one start on the same frame.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::prelude::*;
use moly_law::action_button::{ButtonType, TargetId, ACTION_BUTTON_INPUT_INTERVAL};

use super::cond::{self, flat};
use super::hand::{self, Hand};
use super::script::{Join, Node, Step, Target, TargetKind};

/// Steps one track may start on one frame.
const STEPS_PER_FRAME: usize = 64;
/// Records the log keeps.
const RECORDS: usize = 256;
/// A corner is passed when the player is this close (metres).
const CORNER_REACHED: f32 = 0.15;
/// A walk that has not come this much closer (metres) in [`STALL_SECS`] plans
/// its path again.
const STALL_PROGRESS: f32 = 0.05;
const STALL_SECS: f32 = 2.0;
/// A goal that moved this far (metres) is planned again.
const GOAL_MOVED: f32 = 0.25;

/// One line of the run's log.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Record {
    pub(crate) t: f32,
    pub(crate) step: String,
    pub(crate) event: &'static str,
    pub(crate) note: String,
}

/// What a frame's poll of a step says.
enum Poll {
    Running(String),
    Done(String),
    Failed(String),
}

/// State shared by every track of a run.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) hand: Hand,
    /// The step whose walk holds the joystick finger.
    hand_owner: Option<String>,
    /// Characters taken by this run, released when it ends.
    pub(crate) owned: Vec<(u32, Entity)>,
    pub(crate) log: VecDeque<Record>,
    pub(crate) now: f32,
    /// Bumped on every record, so the snapshot is republished only on change.
    pub(crate) revision: u64,
    /// Fixture activities started by this run.
    activities: u64,
}

impl Shared {
    fn record(&mut self, step: &str, event: &'static str, note: String) {
        let line = format!("[director] {step}: {event}{}{note}", if note.is_empty() { "" } else { ": " });
        match event {
            "failed" => warn!("{line}"),
            _ => info!("{line}"),
        }
        if self.log.len() == RECORDS {
            self.log.pop_front();
        }
        self.log.push_back(Record { t: self.now, step: step.to_owned(), event, note });
        self.revision += 1;
    }

    fn release_hand_of(&mut self, world: &mut World, path: &str) {
        if self.hand_owner.as_deref() == Some(path) {
            self.hand.release(world);
            self.hand_owner = None;
        }
    }

    pub(crate) fn release_all(&mut self, world: &mut World) {
        self.hand.release(world);
        self.hand_owner = None;
        for (_, entity) in std::mem::take(&mut self.owned) {
            super::npc::release(world, entity);
        }
    }
}

struct Frame {
    block: Arc<[Step]>,
    next: usize,
    repeats_left: u32,
    path: String,
}

/// One running step.
struct Active {
    step: Step,
    path: String,
    deadline: Option<f32>,
    kind: Kind,
    /// The last running note, for the timeout's report.
    note: String,
}

enum Kind {
    Wait { until: f32 },
    WaitUntil(super::script::Cond),
    Walk(Walk),
    Interact(Interact),
    Stick { stick: Vec2, until: f32 },
    NpcWalk { entity: Entity, unit: u32, goal: Vec3, within: f32 },
    NpcFace { entity: Entity, unit: u32, towards: Vec3, secs: f32, started: bool },
    Emote { lease: crate::emoticon::timeline::EmoteLease, until: f32 },
    NpcUse { ticket: u64 },
    Tweet { entity: Entity, until: f32 },
    Parallel { tracks: Vec<Track>, join: Join },
    SiteSwitch { site: String },
}

impl Kind {
    /// Undoes what a step leaves running when it ends early (stopped, failed
    /// or timed out).
    fn stop(&self, world: &mut World) {
        match self {
            Kind::NpcWalk { entity, .. } => crate::npc::stop_for_external_activity(world, *entity),
            Kind::Emote { lease, .. } => crate::emoticon::timeline::hide(world, *lease, true),
            Kind::NpcUse { ticket } => super::npc::end_activity(world, *ticket, true),
            Kind::Tweet { entity, .. } => super::npc::hide_tweet(world, *entity),
            _ => {}
        }
    }
}

/// A sequence of steps.
pub(crate) struct Track {
    frames: Vec<Frame>,
    active: Option<Active>,
}

/// How a track stands after a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TrackState {
    Running,
    Done,
    Failed(String),
}

impl Track {
    pub(crate) fn new(block: Arc<[Step]>, path: String) -> Self {
        Self { frames: vec![Frame { block, next: 0, repeats_left: 0, path }], active: None }
    }

    /// The paths of the steps running now, innermost last.
    pub(crate) fn current(&self, out: &mut Vec<String>) {
        if let Some(active) = &self.active {
            out.push(active.path.clone());
            if let Kind::Parallel { tracks, .. } = &active.kind {
                for track in tracks {
                    track.current(out);
                }
            }
        }
    }

    /// Ends the running step without finishing it (a stop, or another track
    /// of a `join: any` block finished first).
    pub(crate) fn abort(&mut self, world: &mut World, shared: &mut Shared) {
        if let Some(mut active) = self.active.take() {
            if let Kind::Parallel { tracks, .. } = &mut active.kind {
                for track in tracks {
                    track.abort(world, shared);
                }
            }
            active.kind.stop(world);
            shared.release_hand_of(world, &active.path);
            shared.record(&active.path, "stopped", String::new());
        }
        self.frames.clear();
    }

    pub(crate) fn tick(&mut self, world: &mut World, shared: &mut Shared) -> TrackState {
        for _ in 0..STEPS_PER_FRAME {
            if self.active.is_none() {
                let Some((step, path)) = self.next_step() else {
                    return TrackState::Done;
                };
                match start(world, shared, step, path) {
                    Started::Ended(path, Ok(note)) => {
                        shared.record(&path, "done", note);
                        continue;
                    }
                    Started::Ended(path, Err((optional, note))) => {
                        shared.record(&path, "failed", note.clone());
                        if optional {
                            continue;
                        }
                        return TrackState::Failed(format!("{path}: {note}"));
                    }
                    Started::Running(active) => self.active = Some(active),
                }
            }
            let active = self.active.as_mut().expect("set above");
            let poll = match active.deadline {
                Some(deadline) if shared.now >= deadline => Poll::Failed(format!(
                    "timed out after {:.1} s; {}",
                    active.step.timeout.unwrap_or_default(),
                    active.note
                )),
                _ => poll(world, shared, active),
            };
            match poll {
                Poll::Running(note) => {
                    active.note = note;
                    return TrackState::Running;
                }
                Poll::Done(note) => {
                    let mut active = self.active.take().expect("polled above");
                    shared.release_hand_of(world, &active.path);
                    if let Kind::Parallel { tracks, .. } = &mut active.kind {
                        for track in tracks {
                            track.abort(world, shared);
                        }
                    }
                    shared.record(&active.path, "done", note);
                }
                Poll::Failed(note) => {
                    let mut active = self.active.take().expect("polled above");
                    if let Kind::Parallel { tracks, .. } = &mut active.kind {
                        for track in tracks {
                            track.abort(world, shared);
                        }
                    }
                    active.kind.stop(world);
                    shared.release_hand_of(world, &active.path);
                    shared.record(&active.path, "failed", note.clone());
                    if !active.step.optional {
                        return TrackState::Failed(format!("{}: {note}", active.path));
                    }
                }
            }
        }
        TrackState::Running
    }

    fn next_step(&mut self) -> Option<(Step, String)> {
        loop {
            let frame = self.frames.last_mut()?;
            if frame.next < frame.block.len() {
                let index = frame.next;
                frame.next += 1;
                let step = frame.block[index].clone();
                let path = format!("{}/{}:{}", frame.path, index + 1, step.label);
                if let Node::Repeat { times, steps } = &step.node {
                    if *times > 0 {
                        self.frames.push(Frame { block: steps.clone(), next: 0, repeats_left: times - 1, path });
                    }
                    continue;
                }
                return Some((step, path));
            }
            if frame.repeats_left > 0 {
                frame.repeats_left -= 1;
                frame.next = 0;
                continue;
            }
            self.frames.pop();
        }
    }
}

enum Started {
    Ended(String, Result<String, (bool, String)>),
    Running(Active),
}

fn start(world: &mut World, shared: &mut Shared, step: Step, path: String) -> Started {
    shared.record(&path, "start", String::new());
    let ended = |path: String, result: Result<String, String>, optional: bool| {
        Started::Ended(path, result.map_err(|note| (optional, note)))
    };
    let optional = step.optional;
    let kind = match &step.node {
        Node::Wait { secs } => Kind::Wait { until: shared.now + secs },
        Node::WaitUntil { until } => Kind::WaitUntil(until.clone()),
        Node::Assert { that } => {
            let (holds, note) = cond::eval(world, that);
            return ended(path, if holds { Ok(note) } else { Err(format!("does not hold: {note}")) }, optional);
        }
        Node::Log { text } => return ended(path, Ok(text.clone()), optional),
        Node::Where { of } => {
            let mut notes = vec![match cond::player(world) {
                Ok(at) => format!("player ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z),
                Err(error) => error,
            }];
            for target in of {
                notes.push(match cond::resolve(world, target) {
                    Ok(at) => format!("{:?} ({:.2}, {:.2}, {:.2})", target.what, at.x, at.y, at.z),
                    Err(error) => format!("{:?}: {error}", target.what),
                });
            }
            return ended(path, Ok(notes.join("; ")), optional);
        }
        Node::Capture => {
            world.write_message(crate::frame_capture::CaptureFrame);
            return ended(path, Ok("frame capture requested".into()), optional);
        }
        Node::Release => {
            shared.hand.release(world);
            shared.hand_owner = None;
            return ended(path, Ok(String::new()), optional);
        }
        Node::Tap { at, fraction } => {
            let position = if *fraction {
                match hand::window_size(world) {
                    Some(size) => Vec2::new(at[0], at[1]) * size,
                    None => return ended(path, Err("no primary window".into()), optional),
                }
            } else {
                Vec2::new(at[0], at[1])
            };
            let result = hand::tap(world, position)
                .map(|()| format!("tapped ({:.0}, {:.0})", position.x, position.y))
                .map_err(str::to_owned);
            return ended(path, result, optional);
        }
        Node::NpcTake { unit } => {
            let result = super::npc::take(world, *unit).map(|entity| {
                if !shared.owned.iter().any(|(_, owned)| *owned == entity) {
                    shared.owned.push((*unit, entity));
                }
                format!("character {unit} taken")
            });
            return ended(path, result, optional);
        }
        Node::NpcRelease { unit } => {
            let Some(index) = shared.owned.iter().position(|(owned, _)| owned == unit) else {
                return ended(path, Err(format!("character {unit} is not taken by this script")), optional);
            };
            let (_, entity) = shared.owned.remove(index);
            super::npc::release(world, entity);
            return ended(path, Ok(format!("character {unit} released")), optional);
        }
        Node::Weather { phenomenon } => {
            world.write_message(crate::server_panel::PhenomenaScheduleEdit::Set(*phenomenon));
            return ended(path, Ok(format!("phenomenon {phenomenon} requested")), optional);
        }
        Node::SiteSwitch { site } => {
            world.insert_resource(crate::site::SiteChangeRequest(site.clone()));
            Kind::SiteSwitch { site: site.clone() }
        }
        Node::WalkTo { to, within } => Kind::Walk(Walk::new(to.clone(), *within)),
        Node::Stick { stick, secs } => Kind::Stick { stick: Vec2::from(*stick), until: shared.now + secs },
        Node::Interact { target, button, stop } => Kind::Interact(Interact {
            walk: Walk::new(target.clone(), *stop),
            button: *button,
            lifted_at: None,
            last_change: f32::NEG_INFINITY,
        }),
        Node::NpcWalkTo { unit, to, within, rests } => {
            let Some(&(_, entity)) = shared.owned.iter().find(|(owned, _)| owned == unit) else {
                return ended(path, Err(format!("character {unit} is not taken by this script (npc.take first)")), optional);
            };
            let goal = match cond::resolve(world, to) {
                Ok(goal) => goal,
                Err(error) => return ended(path, Err(error), optional),
            };
            if let Err(error) = super::npc::walk(world, entity, Vec2::new(goal.x, goal.z), *rests) {
                return ended(path, Err(error), optional);
            }
            Kind::NpcWalk { entity, unit: *unit, goal, within: *within }
        }
        Node::NpcFace { unit, to, secs } => {
            let Some(&(_, entity)) = shared.owned.iter().find(|(owned, _)| owned == unit) else {
                return ended(path, Err(format!("character {unit} is not taken by this script (npc.take first)")), optional);
            };
            let towards = match cond::resolve(world, to) {
                Ok(towards) => towards,
                Err(error) => return ended(path, Err(error), optional),
            };
            Kind::NpcFace { entity, unit: *unit, towards, secs: *secs, started: false }
        }
        Node::NpcEmote { unit, name, secs, use_root } => {
            let entity = match cond::npc(world, *unit) {
                Ok(entity) => entity,
                Err(error) => return ended(path, Err(error), optional),
            };
            match crate::emoticon::timeline::show(world, entity, name, *use_root) {
                Ok(lease) => Kind::Emote { lease, until: shared.now + secs },
                Err(error) => return ended(path, Err(error), optional),
            }
        }
        Node::NpcUse { unit, fixture, activity, variant, rests } => {
            let Some(&(_, entity)) = shared.owned.iter().find(|(owned, _)| owned == unit) else {
                return ended(path, Err(format!("character {unit} is not taken by this script (npc.take first)")), optional);
            };
            shared.activities += 1;
            let ticket = crate::npc_fixture_activity::preview::DIRECTOR_TICKET | shared.activities;
            match super::npc::begin_activity(world, entity, fixture, *activity, *variant, *rests, ticket) {
                Ok(note) => shared.record(&path, "begun", note),
                Err(error) => return ended(path, Err(error), optional),
            }
            Kind::NpcUse { ticket }
        }
        Node::NpcTweet { unit, tweet, secs } => {
            let Some(&(_, entity)) = shared.owned.iter().find(|(owned, _)| owned == unit) else {
                return ended(path, Err(format!("character {unit} is not taken by this script (npc.take first)")), optional);
            };
            match super::npc::show_tweet(world, entity, *tweet) {
                Ok(note) => shared.record(&path, "shown", note),
                Err(error) => return ended(path, Err(error), optional),
            }
            Kind::Tweet { entity, until: shared.now + secs }
        }
        Node::Parallel { tracks, join } => Kind::Parallel {
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(index, block)| Track::new(block.clone(), format!("{path}[{}]", index + 1)))
                .collect(),
            join: *join,
        },
        Node::Repeat { .. } => unreachable!("repeat blocks are entered by next_step"),
    };
    let deadline = step.timeout.map(|timeout| shared.now + timeout);
    Started::Running(Active { step, path, deadline, kind, note: String::new() })
}

fn poll(world: &mut World, shared: &mut Shared, active: &mut Active) -> Poll {
    let path = active.path.clone();
    match &mut active.kind {
        Kind::Wait { until } => {
            if shared.now >= *until {
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{:.1} s left", *until - shared.now))
            }
        }
        Kind::WaitUntil(until) => {
            let (holds, note) = cond::eval(world, until);
            if holds { Poll::Done(note) } else { Poll::Running(note) }
        }
        Kind::Walk(walk) => walk.poll(world, shared, &path),
        Kind::Stick { stick, until } => {
            if shared.now >= *until {
                shared.release_hand_of(world, &path);
                return Poll::Done(String::new());
            }
            if let Some(owner) = shared.hand_owner.as_deref().filter(|owner| *owner != path) {
                return Poll::Failed(format!("the joystick finger is walking for {owner}"));
            }
            shared.hand_owner = Some(path.clone());
            match shared.hand.lean(world, *stick) {
                Ok(()) => Poll::Running(format!("{:.1} s left", *until - shared.now)),
                Err(error) => Poll::Running(error.to_owned()),
            }
        }
        Kind::Interact(interact) => interact.poll(world, shared, &path),
        Kind::NpcWalk { entity, unit, goal, within } => {
            let at = world.get::<GlobalTransform>(*entity).map(GlobalTransform::translation);
            let Some(at) = at else {
                return Poll::Failed(format!("character {unit} is gone"));
            };
            let distance = flat(at, *goal);
            match super::npc::walk_outcome(world, *entity) {
                Some(true) => Poll::Done(format!("arrived at {distance:.2} m")),
                _ if distance <= *within => {
                    // Close enough: the character stops where it is.
                    crate::npc::stop_for_external_activity(world, *entity);
                    Poll::Done(format!("within {distance:.2} m; stopped there"))
                }
                Some(false) => Poll::Failed(format!("stopped short at {distance:.2} m")),
                None => Poll::Running(format!("{distance:.2} m to go")),
            }
        }
        Kind::NpcFace { entity, unit, towards, secs, started } => {
            if world.get_entity(*entity).is_err() {
                return Poll::Failed(format!("character {unit} is gone"));
            }
            if !*started {
                // The turn starts once the character stands.
                if !super::npc::standing(world, *entity) {
                    return Poll::Running(format!("waiting for character {unit} to stand"));
                }
                if let Err(error) = super::npc::face(world, *entity, *towards, *secs) {
                    return Poll::Failed(error);
                }
                *started = true;
                return Poll::Running(format!("character {unit} turning"));
            }
            if super::npc::facing(world, *entity) {
                Poll::Running(format!("character {unit} turning"))
            } else {
                Poll::Done(String::new())
            }
        }
        Kind::Emote { lease, until } => {
            if shared.now >= *until {
                crate::emoticon::timeline::hide(world, *lease, false);
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{:.1} s left", *until - shared.now))
            }
        }
        Kind::NpcUse { ticket } => {
            use crate::npc_fixture_activity::preview::PreviewPhase;
            let ticket = *ticket;
            let poll = match super::npc::activity_phase(world, ticket) {
                None => Poll::Failed("the activity's record is gone".into()),
                Some((PreviewPhase::Completed, _)) => Poll::Done("the activity completed".into()),
                Some((PreviewPhase::Cancelled, _)) => Poll::Failed("the activity was cancelled".into()),
                Some((PreviewPhase::Failed, error)) => {
                    Poll::Failed(error.unwrap_or_else(|| "the activity failed".into()))
                }
                Some((phase, note)) => {
                    return Poll::Running(match note {
                        Some(note) => format!("{phase:?}: {note}"),
                        None => format!("{phase:?}"),
                    });
                }
            };
            super::npc::end_activity(world, ticket, false);
            poll
        }
        Kind::Tweet { entity, until } => {
            if shared.now >= *until {
                super::npc::hide_tweet(world, *entity);
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{:.1} s left", *until - shared.now))
            }
        }
        Kind::Parallel { tracks, join } => {
            let mut running = 0;
            for track in tracks.iter_mut() {
                match track.tick(world, shared) {
                    TrackState::Running => running += 1,
                    TrackState::Done if *join == Join::Any => return Poll::Done("one track ended".into()),
                    TrackState::Done => {}
                    TrackState::Failed(error) => return Poll::Failed(error),
                }
            }
            if running == 0 { Poll::Done(String::new()) } else { Poll::Running(format!("{running} tracks running")) }
        }
        Kind::SiteSwitch { site } => {
            let current = world.get_resource::<crate::site::SiteActive>().map(|active| active.site_type.clone());
            let moving = world.contains_resource::<crate::site_move::SiteMoveActive>();
            if current.as_deref() == Some(site.as_str()) && !moving {
                Poll::Done(format!("on {site}"))
            } else {
                Poll::Running(format!("site {current:?}, moving {moving}"))
            }
        }
    }
}

/// The player walking to a target along the navigation field.
struct Walk {
    to: Target,
    within: f32,
    corners: Vec<Vec2>,
    next: usize,
    /// The goal and field generation the corners were planned for.
    planned: Option<(Vec2, u64)>,
    /// The closest the player has come, and when.
    best: (f32, f32),
}

impl Walk {
    fn new(to: Target, within: f32) -> Self {
        Self { to, within, corners: Vec::new(), next: 0, planned: None, best: (f32::INFINITY, 0.0) }
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared, path: &str) -> Poll {
        let goal = match cond::resolve(world, &self.to) {
            Ok(goal) => goal,
            Err(error) => {
                shared.release_hand_of(world, path);
                return Poll::Running(error);
            }
        };
        let player = match cond::player(world) {
            Ok(player) => player,
            Err(error) => return Poll::Running(error),
        };
        let distance = flat(player, goal);
        if distance <= self.within {
            shared.release_hand_of(world, path);
            return Poll::Done(format!("arrived at {distance:.2} m"));
        }
        // The stick takes no press while it is disabled (the entry before
        // its finish, a dialog, an edit, a site move): stand and wait.
        let enabled = world.get_resource::<crate::joystick::JoystickState>().is_some_and(|stick| stick.enabled);
        if !enabled {
            shared.release_hand_of(world, path);
            return Poll::Running(format!("the joystick is disabled; {distance:.2} m to go"));
        }
        if let Some(owner) = shared.hand_owner.as_deref().filter(|owner| *owner != path) {
            return Poll::Failed(format!("the joystick finger is walking for {owner}"));
        }
        let goal_xz = Vec2::new(goal.x, goal.z);
        let player_xz = Vec2::new(player.x, player.z);
        let generation = world.get_resource::<crate::walk_face::WalkFace>().map_or(0, |face| face.generation());
        if distance < self.best.0 - STALL_PROGRESS {
            self.best = (distance, shared.now);
        }
        let stalled = shared.now - self.best.1 > STALL_SECS;
        let replan = match self.planned {
            None => true,
            Some((planned_goal, planned_generation)) => {
                planned_goal.distance(goal_xz) > GOAL_MOVED || planned_generation != generation || stalled
            }
        };
        if replan {
            self.corners = world
                .get_resource::<crate::walk_face::WalkFace>()
                .and_then(|face| face.path(player_xz.into(), goal_xz.into()))
                .map(|corners| corners.into_iter().map(Vec2::from).collect())
                .unwrap_or_else(|| vec![goal_xz]);
            self.next = 0;
            self.planned = Some((goal_xz, generation));
            self.best = (distance, shared.now);
        }
        while self.next < self.corners.len() && self.corners[self.next].distance(player_xz) < CORNER_REACHED {
            self.next += 1;
        }
        let aim = self.corners.get(self.next).copied().unwrap_or(goal_xz);
        shared.hand_owner = Some(path.to_owned());
        if let Err(error) = shared.hand.steer(world, aim - player_xz) {
            return Poll::Running(error.to_owned());
        }
        Poll::Running(format!(
            "{distance:.2} m to go, corner {}/{}",
            (self.next + 1).min(self.corners.len()),
            self.corners.len()
        ))
    }
}

/// Walking to a target until its action button heads the stack, then
/// pressing it.
struct Interact {
    walk: Walk,
    button: ButtonType,
    lifted_at: Option<f32>,
    last_change: f32,
}

impl Interact {
    /// The stack entry this interaction presses: a character's or a placed
    /// fixture's own entry when the target names one.
    fn wanted(&self, world: &World) -> Option<TargetId> {
        let state = world.get_resource::<crate::action_button::ActionButtonState>()?;
        match &self.walk.to.what {
            TargetKind::Npc(unit) => Some(TargetId::Character(*unit)),
            TargetKind::FixtureUid(uid) => state.fixture_key(uid).map(TargetId::Fixture),
            _ => None,
        }
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared, path: &str) -> Poll {
        let wanted = self.wanted(world);
        let Some(state) = world.get_resource::<crate::action_button::ActionButtonState>() else {
            return Poll::Running("the action buttons are not set up".into());
        };
        let head = state.current();
        let want = self.button;
        let ours = move |(button, target): (ButtonType, TargetId)| {
            button == want && wanted.is_none_or(|wanted| wanted == target)
        };
        let stacked = match wanted {
            Some(target) => state.contains(self.button, target),
            None => state.stacked(self.button),
        };
        let change_shown = state.change_shown();
        let now = shared.now;
        if head.is_some_and(ours) && now - self.last_change >= ACTION_BUTTON_INPUT_INTERVAL + 0.05 {
            // Lift the joystick finger first and tap on a later frame, once
            // the player stands, if the button still heads the stack.
            if shared.hand.pressing() || self.lifted_at.is_none() {
                shared.release_hand_of(world, path);
                self.lifted_at = Some(now);
                return Poll::Running(format!("{:?} heads the stack; standing", self.button));
            }
            let Some(position) = hand::button_position(world, self.button) else {
                return Poll::Running(format!("{:?} heads the stack but its view is not laid out", self.button));
            };
            return match hand::tap(world, position) {
                Ok(()) => Poll::Done(format!("pressed {:?} at ({:.0}, {:.0})", self.button, position.x, position.y)),
                Err(error) => Poll::Running(error.to_owned()),
            };
        }
        self.lifted_at = None;
        if let Some((shown, _)) = head.filter(|head| !ours(*head)) {
            if stacked && change_shown && now - self.last_change >= 1.0 {
                if let Some(position) = hand::change_position(world, shown) {
                    shared.release_hand_of(world, path);
                    if hand::tap(world, position).is_ok() {
                        self.last_change = now;
                        return Poll::Running(format!("{:?} is stacked behind {shown:?}; tapped the change-target button", self.button));
                    }
                }
            }
        }
        match self.walk.poll(world, shared, path) {
            Poll::Done(note) => Poll::Running(format!("{note}; waiting for {:?} to head the stack (head {head:?})", self.button)),
            Poll::Running(note) => Poll::Running(format!("{note}; head {head:?}")),
            failed => failed,
        }
    }
}
