//! The executor: tracks of steps advanced once a frame. A track runs its
//! steps in order; `parallel` runs child tracks side by side and `repeat`
//! runs a block again. A step that ends on the frame it starts (a log, a
//! take, an assert) lets the next one start on the same frame.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::input::touch::TouchPhase;
use bevy::prelude::*;
use moly_law::action_button::{ButtonType, TargetId, ACTION_BUTTON_INPUT_INTERVAL};
use moly_link::ServerLink;
use moly_protocol::{AdminCall, Call, CallKind};
use serde_json::{json, Value};

use crate::userdata::{Answered, AnsweredLog};

use super::cond::{self, flat};
use super::hand::{self, Hand};
use super::script::{GatherRef, Join, Node, Step, Target, TargetKind};

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
    /// The server exchange the step's effect goes through, once it is sent.
    server: Option<ServerWait>,
    /// The last running note, for the timeout's report.
    note: String,
}

/// Waits for the server's answer to the step's own request.
#[derive(Clone, Copy)]
struct ServerWait {
    call: CallKind,
    /// The answer log's length when the step started: only a later answer is
    /// this step's.
    since: usize,
    /// The request has been seen in flight, and, when the feature exposes
    /// it, the sequence number it is waiting for.
    seq: Option<u32>,
    seen: bool,
}

impl ServerWait {
    fn new(world: &World, call: CallKind) -> Self {
        Self {
            call,
            since: world.resource::<crate::userdata::AnsweredLog>().len(),
            seq: None,
            seen: false,
        }
    }
}

/// The call the feature that owns `call` is waiting for, when it exposes one.
fn feature_pending(world: &World, call: CallKind) -> Option<crate::userdata::ServerPending> {
    match call {
        CallKind::Craft => world
            .get_resource::<crate::home_action::craft::CraftScreen>()
            .and_then(|screen| screen.server_pending())
            .copied(),
        CallKind::HousingLayout => world
            .get_resource::<crate::fixture_edit::EditSession>()
            .and_then(|session| session.server_pending())
            .copied(),
        CallKind::BirthdayPartyDelivery => world
            .get_resource::<crate::delivery::flow::DeliveryFlow>()
            .and_then(|flow| flow.server_pending())
            .copied(),
        _ => None,
    }
}

enum Kind {
    Wait { until: f32 },
    WaitUntil(super::script::Cond),
    Walk(Walk),
    Interact(Interact),
    Gather(Gather),
    Craft(CraftStep),
    LayoutPlace(LayoutPlace),
    Deliver(Deliver),
    DialogAnswer(DialogAnswer),
    Stick { stick: Vec2, until: f32 },
    NpcWalk { entity: Entity, unit: u32, goal: Vec3, within: f32 },
    NpcFace { entity: Entity, unit: u32, towards: Vec3, secs: f32, started: bool },
    Emote { lease: crate::emoticon::timeline::EmoteLease, until: f32 },
    NpcUse { ticket: u64 },
    Tweet { entity: Entity, until: f32 },
    /// A talk requested of a character, until its decision builds it.
    NpcTalk { entity: Entity },
    /// `down` is false until the finger has touched down at `from`.
    CameraDrag { from: Vec2, by: Vec2, start: f32, secs: f32, down: bool },
    CameraZoom { left: f32, per_sec: f32, until: f32 },
    Parallel { tracks: Vec<Track>, join: Join },
    /// `moved` once the move has been seen in progress.
    SiteSwitch { site: String, moved: bool },
    /// One `server.edit`, until the server answers the call numbered `seq`.
    ServerEdit { seq: u32 },
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
            Kind::NpcTalk { entity } => super::npc::drop_talk(world, *entity),
            // A press left holding the button is released.
            Kind::Deliver(_) => {
                world.write_message(crate::delivery::DeliveryRequest::End);
            }
            // A selection left in the editor is cancelled: the editor's own
            // cancel restores the pose the fixture was selected at.
            Kind::LayoutPlace(_) => {
                world.write_message(crate::fixture_edit::EditCommand::Cancel);
            }
            Kind::CameraDrag { from, by, down: true, .. } => {
                let _ = hand::drag_touch(world, TouchPhase::Ended, *from + *by);
            }
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
            let poll = match poll {
                Poll::Done(note) => wait_server(world, active, note),
                other => other,
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
        Node::Wear { unit } => {
            use crate::player_avatar::oc::{CharacterChoice, OcRequest};
            let (request, note) = match unit {
                Some(unit) => (OcRequest::Character(CharacterChoice::Unit(*unit)), format!("character body {unit} requested")),
                None => (OcRequest::Avatar, "avatar body requested".to_owned()),
            };
            world.write_message(request);
            return ended(path, Ok(note), optional);
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
        Node::UiTap { view, path: node } => {
            let result = hand::ui_point(world, view, node).and_then(|position| {
                hand::tap(world, position)
                    .map(|()| format!("tapped {view} {node} at ({:.0}, {:.0})", position.x, position.y))
                    .map_err(str::to_owned)
            });
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
        Node::ServerEdit { command } => {
            let command_json = Value::Object(command.clone()).to_string();
            let Some(seq) = world
                .get_resource_mut::<ServerLink>()
                .map(|mut link| link.call(Call::Admin(AdminCall::Edit { command_json })))
            else {
                return ended(
                    path,
                    Err("the game has no server link".to_owned()),
                    optional,
                );
            };
            Kind::ServerEdit { seq }
        }
        Node::SiteSwitch { site } => {
            world.insert_resource(crate::site::SiteChangeRequest(site.clone()));
            Kind::SiteSwitch { site: site.clone(), moved: false }
        }
        Node::WalkTo { to, within } => Kind::Walk(Walk::new(to.clone(), *within)),
        Node::Gather { what, collect } => Kind::Gather(Gather::new(what.clone(), *collect)),
        Node::Craft { blueprint } => Kind::Craft(CraftStep::new(*blueprint)),
        Node::LayoutPlace { fixture, texture, center, rotate, decide, save } => {
            Kind::LayoutPlace(LayoutPlace::new(*fixture, *texture, *center, *rotate, *decide, *save))
        }
        Node::Deliver { party, hold } => match Deliver::press(world, *party, *hold, shared.now) {
            Ok(deliver) => Kind::Deliver(deliver),
            Err(error) => return ended(path, Err(error), optional),
        },
        Node::DialogAnswer { ok } => Kind::DialogAnswer(DialogAnswer { ok: *ok, tapped: None, pressed: None }),
        Node::BackKey => {
            let result = hand::back_key(world).map(|()| "the back key pressed".to_owned()).map_err(str::to_owned);
            return ended(path, result, optional);
        }
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
        Node::NpcTalk { unit, talk } => {
            let entity = match super::npc::open_talk(world, *unit, *talk) {
                Ok(entity) => entity,
                Err(error) => return ended(path, Err(error), optional),
            };
            // The character is its AI's again; the AI plays the talk.
            shared.owned.retain(|(_, owned)| *owned != entity);
            Kind::NpcTalk { entity }
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
        Node::CameraDrag { by, secs, from } => {
            let Some(size) = hand::window_size(world) else {
                return ended(path, Err("no primary window".into()), optional);
            };
            Kind::CameraDrag {
                from: Vec2::from(*from) * size,
                by: Vec2::from(*by),
                start: shared.now,
                secs: *secs,
                down: false,
            }
        }
        Node::CameraZoom { by, secs } => Kind::CameraZoom {
            left: *by,
            per_sec: if *secs > 0.0 { by / secs } else { f32::INFINITY },
            until: shared.now + secs,
        },
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
    let server = match &step.node {
        Node::Gather { .. } => Some(ServerWait::new(world, CallKind::Harvest)),
        Node::Craft { .. } => Some(ServerWait::new(world, CallKind::Craft)),
        Node::LayoutPlace { save, .. } if *save => {
            Some(ServerWait::new(world, CallKind::HousingLayout))
        }
        Node::Deliver { .. } => Some(ServerWait::new(world, CallKind::BirthdayPartyDelivery)),
        _ => None,
    };
    let deadline = step.timeout.map(|timeout| shared.now + timeout);
    Started::Running(Active { step, path, deadline, kind, server, note: String::new() })
}

fn wait_server(world: &World, active: &mut Active, note: String) -> Poll {
    let Some(mut wait) = active.server else {
        return Poll::Done(note);
    };
    // The feature's own pending call first: its sequence number is the one the
    // answer will carry and it clears once the feature has read the answer.
    if let Some(pending) = feature_pending(world, wait.call) {
        if let Some(seq) = pending.seq {
            wait.seq = Some(seq);
            wait.seen = true;
            active.server = Some(wait);
            return Poll::Running(format!(
                "{note} · waiting for the server ({}, seq {seq})",
                crate::userdata::call_name(wait.call)
            ));
        }
    }
    if wait.seq.is_none() {
        // A feature without its own pending accessor (the harvest call): the
        // link names the request in flight.
        if world
            .resource::<ServerLink>()
            .waiting()
            .any(|(_, kind)| *kind == wait.call)
        {
            wait.seen = true;
            active.server = Some(wait);
            return Poll::Running(format!(
                "{note} · waiting for the server ({})",
                crate::userdata::call_name(wait.call)
            ));
        }
    }
    let log = world.resource::<AnsweredLog>();
    if !wait.seen {
        // Not seen in flight: either the request went out and was answered
        // while the step itself was still running, or none was sent.
        return Poll::Done(match log.newest_since(wait.call, wait.since) {
            Some(answer) => format!("{note} · {}", describe_answer(answer)),
            None => note,
        });
    }
    let answer = wait
        .seq
        .and_then(|seq| log.get(seq))
        .or_else(|| log.newest_since(wait.call, wait.since));
    match answer {
        Some(answer) => {
            let text = describe_answer(answer);
            active.server = None;
            Poll::Done(format!("{note} · {text}"))
        }
        None => Poll::Running(format!(
            "{note} · waiting for the server's answer ({})",
            crate::userdata::call_name(wait.call)
        )),
    }
}

/// A step's server answer as the run log reports it.
fn describe_answer(answer: &Answered) -> String {
    let call = crate::userdata::call_name(answer.call);
    match &answer.outcome {
        Ok(_) => format!("{{seq: {}, call: {call}, ok}}", answer.seq),
        Err(refusal) => format!(
            "{{seq: {}, call: {call}, refusal: {}}}",
            answer.seq, refusal.code
        ),
    }
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
        Kind::Gather(gather) => gather.poll(world, shared, &path),
        Kind::Craft(craft) => craft.poll(world, shared),
        Kind::LayoutPlace(place) => place.poll(world, shared),
        Kind::Deliver(deliver) => deliver.poll(world, shared),
        Kind::ServerEdit { seq } => {
            let answered = world.resource::<AnsweredLog>().get(*seq).cloned();
            if let Some(answer) = answered {
                return match &answer.outcome {
                    Ok(_) => Poll::Done(describe_answer(&answer)),
                    Err(refusal) => Poll::Failed(format!(
                        "{}: {}",
                        describe_answer(&answer),
                        refusal.reason
                    )),
                };
            }
            if world
                .resource::<ServerLink>()
                .waiting()
                .any(|(waiting, _)| waiting == seq)
            {
                Poll::Running(format!("waiting for the server's answer (seq {seq})"))
            } else {
                Poll::Failed(format!("the server edit (seq {seq}) has no answer"))
            }
        }
        Kind::DialogAnswer(answer) => answer.poll(world, shared),
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
        Kind::NpcTalk { entity } => match super::npc::talk_outcome(world, *entity) {
            Some(Ok(note)) => Poll::Done(note),
            Some(Err(reason)) => Poll::Failed(reason),
            None => Poll::Running("waiting for the character's next decision".into()),
        },
        Kind::Tweet { entity, until } => {
            if shared.now >= *until {
                super::npc::hide_tweet(world, *entity);
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{:.1} s left", *until - shared.now))
            }
        }
        Kind::CameraDrag { from, by, start, secs, down } => {
            if !*down {
                if let Err(error) = hand::drag_touch(world, TouchPhase::Started, *from) {
                    return Poll::Failed(error.into());
                }
                *down = true;
                *start = shared.now;
                return Poll::Running("finger down".into());
            }
            let done = (shared.now - *start) / secs.max(f32::EPSILON);
            let at = *from + *by * done.min(1.0);
            if let Err(error) = hand::drag_touch(world, TouchPhase::Moved, at) {
                return Poll::Failed(error.into());
            }
            if done < 1.0 {
                return Poll::Running(format!("{:.0} %", done * 100.0));
            }
            // The last move lands on the end point before the finger lifts.
            *down = false;
            match hand::drag_touch(world, TouchPhase::Ended, at) {
                Ok(()) => Poll::Done(format!("dragged ({:.0}, {:.0}) px", by.x, by.y)),
                Err(error) => Poll::Failed(error.into()),
            }
        }
        Kind::CameraZoom { left, per_sec, until } => {
            let step = if shared.now >= *until {
                *left
            } else {
                let delta = world.resource::<Time>().delta_secs();
                (*per_sec * delta).clamp(-left.abs(), left.abs())
            };
            if step != 0.0 {
                if let Err(error) = hand::wheel(world, step) {
                    return Poll::Failed(error.into());
                }
                *left -= step;
            }
            if shared.now >= *until || *left == 0.0 {
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{left:.0} px left"))
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
        Kind::SiteSwitch { site, moved } => {
            let current = world.get_resource::<crate::site::SiteActive>().map(|active| active.site_type.clone());
            let moving = world.contains_resource::<crate::site_move::SiteMoveActive>();
            if current.as_deref() == Some(site.as_str()) && !moving {
                Poll::Done(format!("on {site}"))
            } else if *moved && !moving {
                // A move that ends elsewhere was refused.
                Poll::Failed(format!("the move ended on {current:?}"))
            } else {
                *moved |= moving;
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

/// Drops are taken from this far around the harvested object (metres).
const DROP_REACH: f32 = 6.0;
/// A drop the player has stood on this long (seconds) is not taken.
const DROP_REFUSED_SECS: f32 = 3.0;
/// A press that has started no swing after this long (seconds) is refused.
const PRESS_ANSWER_SECS: f32 = 1.5;
/// Presses are this far apart (seconds), as a hand taps.
const PRESS_INTERVAL: f32 = 0.3;

enum GatherPhase {
    /// Walking into the object's contact circle until it is the target.
    Approach,
    /// Tapping the harvest button while the object has hit points left.
    Swing,
    /// Walking over the landed drops.
    Collect,
}

/// `player.gather`: one harvest object through the harvest button, then its
/// drops. The object is chosen once, when the step starts; the button is
/// tapped as a finger taps it, so the harvest screen's own rules (the tool
/// for the kind, the stamina, the swing loop) decide each press.
struct Gather {
    what: GatherRef,
    collect: bool,
    /// The object: its entity, UID and position.
    object: Option<(Entity, u64, Vec3)>,
    walk: Option<Walk>,
    phase: GatherPhase,
    /// When the joystick finger was lifted for the presses.
    lifted_at: Option<f32>,
    last_press: Option<f32>,
    presses: u32,
    /// The hit points when the last unanswered press went out.
    hp_at_press: i32,
    /// The drop walked to, and since when the player has stood on it.
    drop: Option<(Vec3, Option<f32>)>,
    drops_taken: u32,
}

impl Gather {
    fn new(what: GatherRef, collect: bool) -> Self {
        Self {
            what,
            collect,
            object: None,
            walk: None,
            phase: GatherPhase::Approach,
            lifted_at: None,
            last_press: None,
            presses: 0,
            hp_at_press: 0,
            drop: None,
            drops_taken: 0,
        }
    }

    /// The harvest object the step names: entity, UID, position, radius.
    fn find(&self, world: &mut World) -> Result<(Entity, u64, Vec3, f32), String> {
        let from = cond::player(world).unwrap_or(Vec3::ZERO);
        let mut objects = world.query::<(Entity, &crate::harvest::HarvestObject, &GlobalTransform)>();
        let rows: Vec<(Entity, u64, Vec3, f32, bool, i32)> = objects
            .iter(world)
            .map(|(entity, object, at)| {
                (
                    entity,
                    object.uid,
                    at.translation(),
                    object.radius,
                    object.status == crate::harvest::STATUS_HARVESTED,
                    object.fixture_type,
                )
            })
            .collect();
        if rows.is_empty() {
            return Err("this site has no gatherables".into());
        }
        match &self.what {
            GatherRef::Uid(uid) => {
                let (entity, uid, at, radius, harvested, _) = rows
                    .into_iter()
                    .find(|row| row.1 == *uid)
                    .ok_or_else(|| format!("no gatherable with UID {uid} on this site"))?;
                if harvested {
                    return Err(format!("gatherable {uid} is already harvested"));
                }
                Ok((entity, uid, at, radius))
            }
            GatherRef::Nearest { kind } => rows
                .into_iter()
                .filter(|row| !row.4)
                .filter(|row| kind.as_deref().is_none_or(|kind| crate::harvest::kind_word(row.5) == kind))
                .min_by(|a, b| a.2.distance_squared(from).total_cmp(&b.2.distance_squared(from)))
                .map(|(entity, uid, at, radius, _, _)| (entity, uid, at, radius))
                .ok_or_else(|| match kind {
                    Some(kind) => format!("no {kind} gatherable left to harvest on this site"),
                    None => "no gatherable left to harvest on this site".to_owned(),
                }),
        }
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared, path: &str) -> Poll {
        let (entity, uid, origin) = match self.object {
            Some(object) => object,
            None => match self.find(world) {
                Ok((entity, uid, at, radius)) => {
                    self.object = Some((entity, uid, at));
                    // Contact is the player inside the object's circle.
                    let within = (radius * 0.6).max(0.15);
                    self.walk = Some(Walk::new(Target { what: TargetKind::Gatherable(uid), offset: [0.0; 2] }, within));
                    (entity, uid, at)
                }
                Err(error) => {
                    shared.release_hand_of(world, path);
                    return Poll::Failed(error);
                }
            },
        };
        let Some(object) = world.get::<crate::harvest::HarvestObject>(entity) else {
            shared.release_hand_of(world, path);
            return Poll::Failed(format!("gatherable {uid} left the site"));
        };
        let (hp, kind, harvested) =
            (object.hp, object.fixture_type, object.status == crate::harvest::STATUS_HARVESTED);
        let targeted = world
            .get_resource::<crate::harvest::action::HarvestTargeting>()
            .and_then(|targeting| targeting.target);
        let swinging = world
            .get_resource::<crate::harvest::action::HarvestAction>()
            .is_some_and(crate::harvest::action::HarvestAction::in_progress);
        let now = shared.now;
        match self.phase {
            GatherPhase::Approach => {
                if harvested {
                    self.phase = GatherPhase::Collect;
                    return Poll::Running(format!("gatherable {uid} is harvested"));
                }
                if targeted == Some(entity) {
                    self.phase = GatherPhase::Swing;
                    return Poll::Running(format!("gatherable {uid} is the target"));
                }
                let walk = self.walk.as_mut().expect("the walk is planned with the object");
                match walk.poll(world, shared, path) {
                    // In reach while another object is the target: walk on
                    // into this one, which the player then faces.
                    Poll::Done(note) => {
                        walk.within = 0.05;
                        Poll::Running(format!("{note}; the target is {targeted:?}, not gatherable {uid}"))
                    }
                    other => other,
                }
            }
            GatherPhase::Swing => {
                if harvested {
                    if swinging {
                        return Poll::Running(format!("gatherable {uid} harvested; the swing ends"));
                    }
                    if !self.collect {
                        return Poll::Done(self.summary(uid, hp));
                    }
                    self.phase = GatherPhase::Collect;
                    return Poll::Running(format!("gatherable {uid} harvested after {} presses", self.presses));
                }
                if swinging {
                    return Poll::Running(format!("swinging: hp {hp}"));
                }
                if targeted != Some(entity) {
                    self.phase = GatherPhase::Approach;
                    self.lifted_at = None;
                    return Poll::Running(format!("gatherable {uid} is no longer the target ({targeted:?})"));
                }
                // Lift the joystick finger first and press on a later frame.
                if shared.hand.pressing() || self.lifted_at.is_none() {
                    shared.release_hand_of(world, path);
                    self.lifted_at = Some(now);
                    return Poll::Running("standing for the presses".into());
                }
                let Some(button) = world.get_resource::<crate::harvest::HarvestButton>() else {
                    return Poll::Running("the harvest screen is not set up".into());
                };
                if button.cannot {
                    let tool = (0..=9).contains(&kind).then(|| crate::harvest::law::tool_type_for(kind)).flatten();
                    return Poll::Failed(format!(
                        "the harvest button shows it cannot act: no tool for gatherable {uid} (it needs {tool:?})"
                    ));
                }
                if !(button.shown && button.interactable) {
                    return Poll::Running(format!(
                        "the harvest button is not ready (shown {}, interactable {})",
                        button.shown, button.interactable
                    ));
                }
                if let Some(last) = self.last_press {
                    if hp == self.hp_at_press && now - last > PRESS_ANSWER_SECS {
                        let stamina = world
                            .get_resource::<crate::harvest::action::HarvestPlayerModel>()
                            .map(|model| model.stamina.sum());
                        return Poll::Failed(format!(
                            "the press on gatherable {uid} started no swing (hp {hp}, stamina {stamina:?}, {} presses)",
                            self.presses
                        ));
                    }
                    if now - last < PRESS_INTERVAL {
                        return Poll::Running(format!("hp {hp}"));
                    }
                }
                let (view, node) = crate::harvest::action_button_node();
                let position = match hand::ui_point(world, view, node) {
                    Ok(position) => position,
                    Err(error) => return Poll::Running(error),
                };
                match hand::tap(world, position) {
                    Ok(()) => {
                        self.hp_at_press = hp;
                        self.last_press = Some(now);
                        self.presses += 1;
                        Poll::Running(format!("pressed the harvest button ({}), hp {hp}", self.presses))
                    }
                    Err(error) => Poll::Running(error.to_owned()),
                }
            }
            GatherPhase::Collect => self.collect(world, shared, path, uid, origin, hp),
        }
    }

    fn collect(&mut self, world: &mut World, shared: &mut Shared, path: &str, uid: u64, origin: Vec3, hp: i32) -> Poll {
        let mut landed = Vec::new();
        let mut falling = 0;
        let mut drops = world.query::<(&crate::harvest::HarvestDropItem, &GlobalTransform)>();
        for (drop, at) in drops.iter(world) {
            let at = at.translation();
            if flat(at, origin) > DROP_REACH {
                continue;
            }
            if drop.radius > 0.0 {
                landed.push(at);
            } else {
                falling += 1;
            }
        }
        let player = match cond::player(world) {
            Ok(player) => player,
            Err(error) => return Poll::Running(error),
        };
        // The drop walked to is taken once nothing lies where it was.
        if let Some((at, _)) = self.drop {
            if !landed.iter().any(|drop| drop.distance(at) < 0.05) {
                self.drop = None;
                self.walk = None;
                self.drops_taken += 1;
            }
        }
        if self.drop.is_none() {
            let Some(next) = landed.iter().copied().min_by(|a, b| flat(*a, player).total_cmp(&flat(*b, player))) else {
                if falling > 0 {
                    return Poll::Running(format!("{falling} drops still falling"));
                }
                shared.release_hand_of(world, path);
                return Poll::Done(self.summary(uid, hp));
            };
            // Pickups happen only while the player moves, so the walk aims
            // past the drop: a drop already underfoot is still crossed.
            let away = Vec2::new(next.x - player.x, next.z - player.z).normalize_or_zero();
            let beyond = if away == Vec2::ZERO { Vec2::X } else { away } * 0.5;
            self.drop = Some((next, None));
            self.walk = Some(Walk::new(
                Target { what: TargetKind::Point([next.x + beyond.x, next.z + beyond.y]), offset: [0.0; 2] },
                0.15,
            ));
        }
        let walk = self.walk.as_mut().expect("the walk is planned with the drop");
        let polled = walk.poll(world, shared, path);
        let (at, since) = self.drop.as_mut().expect("a drop is chosen above");
        match polled {
            Poll::Done(_) => {
                let since = *since.get_or_insert(shared.now);
                if shared.now - since <= DROP_REFUSED_SECS {
                    return Poll::Running(format!("on the drop at ({:.2}, {:.2})", at.x, at.z));
                }
                let at = *at;
                let bag = world
                    .get_resource::<crate::userdata::client::inventory::ClientMysekaiInventory>()
                    .map(|inventory| {
                        let limit = world
                            .get_resource::<crate::userdata::client::inventory::PossessionMasters>()
                            .and_then(|masters| inventory.material_max_count(masters));
                        (inventory.total_material_possession(), limit)
                    });
                shared.release_hand_of(world, path);
                Poll::Failed(format!(
                    "the drop at ({:.2}, {:.2}) is not taken after {DROP_REFUSED_SECS} s on it (bag held and limit {bag:?})",
                    at.x, at.z
                ))
            }
            other => other,
        }
    }

    fn summary(&self, uid: u64, hp: i32) -> String {
        format!("gatherable {uid} harvested: {} presses, hp {hp}, {} drops taken", self.presses, self.drops_taken)
    }
}

/// A tap on the craft screen that has not shown its answer after this long
/// (seconds) is refused.
const CRAFT_ANSWER_SECS: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CraftStepPhase {
    /// Waiting for the open screen, then choosing the blueprint.
    Choose,
    /// The choice is handed over; reading the apply button.
    Check,
    /// The apply button is tapped; the confirm is answered.
    Confirm,
    /// The confirm's OK is tapped; the craft runs to its result.
    Craft,
}

/// `player.craft`'s last step: one blueprint on the open craft screen. The
/// screen's own checks decide (the apply button's balloon, the term, the
/// server's answer), and each refusal ends the step with its reason.
struct CraftStep {
    blueprint: i32,
    phase: CraftStepPhase,
    /// When the last tap or key went out.
    input_at: f32,
    executed: bool,
    chosen: String,
}

impl CraftStep {
    fn new(blueprint: i32) -> Self {
        Self { blueprint, phase: CraftStepPhase::Choose, input_at: 0.0, executed: false, chosen: String::new() }
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared) -> Poll {
        use crate::home_action::craft::{self as craft, CraftPhase};
        let now = shared.now;
        let screen = craft::phase(world);
        match self.phase {
            CraftStepPhase::Choose => {
                if screen != CraftPhase::Idle {
                    return Poll::Running(format!("waiting for the open craft screen ({screen:?})"));
                }
                match craft::choose(world, self.blueprint) {
                    Ok(note) => {
                        self.chosen = note;
                        self.phase = CraftStepPhase::Check;
                        self.input_at = now;
                        Poll::Running(format!("blueprint {} chosen: {}", self.blueprint, self.chosen))
                    }
                    Err(error) => Poll::Failed(error),
                }
            }
            CraftStepPhase::Check => match craft::selected(world) {
                Some((id, state)) if id == self.blueprint => match state {
                    Err(reason) => Poll::Failed(format!("blueprint {} cannot be crafted: {reason}", self.blueprint)),
                    Ok(()) => {
                        let Some((view, node)) = crate::home_action::craft::view::apply_button_node(world) else {
                            return Poll::Running("the craft screen's view is not built".into());
                        };
                        let position = match hand::ui_point(world, view, &node) {
                            Ok(position) => position,
                            Err(error) => return Poll::Running(error),
                        };
                        match hand::tap(world, position) {
                            Ok(()) => {
                                self.phase = CraftStepPhase::Confirm;
                                self.input_at = now;
                                Poll::Running("the apply button tapped".into())
                            }
                            Err(error) => Poll::Running(error.to_owned()),
                        }
                    }
                },
                _ if now - self.input_at > CRAFT_ANSWER_SECS => {
                    Poll::Failed(format!("blueprint {} was not selected ({})", self.blueprint, self.chosen))
                }
                _ => Poll::Running(format!("waiting for blueprint {} to be selected", self.blueprint)),
            },
            CraftStepPhase::Confirm => match screen {
                CraftPhase::Confirm => {
                    let (_, caller, view, node) = match crate::common_dialog::shown_button(world, true) {
                        Ok(button) => button,
                        Err(error) => return Poll::Running(error),
                    };
                    let position = match hand::ui_point(world, view, &node) {
                        Ok(position) => position,
                        Err(error) => return Poll::Running(error),
                    };
                    match hand::tap(world, position) {
                        Ok(()) => {
                            self.phase = CraftStepPhase::Craft;
                            self.input_at = now;
                            Poll::Running(format!("the confirm's OK tapped ({caller})"))
                        }
                        Err(error) => Poll::Running(error.to_owned()),
                    }
                }
                CraftPhase::TermLimit => {
                    Poll::Failed(format!("blueprint {}'s term is over: the term limit dialog shows", self.blueprint))
                }
                _ if now - self.input_at > CRAFT_ANSWER_SECS => Poll::Failed(format!(
                    "the apply tap opened no confirm ({screen:?}){}",
                    craft::refusal(world).map(|reason| format!(": {reason}")).unwrap_or_default()
                )),
                _ => Poll::Running("waiting for the confirm".into()),
            },
            CraftStepPhase::Craft => match screen {
                CraftPhase::Executing | CraftPhase::Dialogs | CraftPhase::Done => {
                    self.executed = true;
                    Poll::Running(format!("crafting ({screen:?})"))
                }
                CraftPhase::Result => {
                    self.executed = true;
                    // The result dialog closes on the back key, as on its
                    // close area.
                    if now - self.input_at > 1.0 {
                        self.input_at = now;
                        if let Err(error) = hand::back_key(world) {
                            return Poll::Running(error.to_owned());
                        }
                        return Poll::Running("the result shows; the back key closes it".into());
                    }
                    Poll::Running("the result shows".into())
                }
                CraftPhase::Idle | CraftPhase::Closed if self.executed => {
                    Poll::Done(format!("blueprint {} crafted ({})", self.blueprint, self.chosen))
                }
                CraftPhase::Confirm if now - self.input_at > CRAFT_ANSWER_SECS => {
                    Poll::Failed("the confirm's OK tap did not answer it".into())
                }
                _ if now - self.input_at > CRAFT_ANSWER_SECS => Poll::Failed(format!(
                    "the craft was not done: {}",
                    craft::refusal(world).unwrap_or_else(|| format!("the screen is {screen:?}"))
                )),
                _ => Poll::Running("waiting for the craft".into()),
            },
        }
    }
}

/// How long one layout command may take to show its effect (seconds).
const LAYOUT_ANSWER_SECS: f32 = 15.0;
/// How many times a move is sent while the reported center stands off the
/// wanted tile.
const LAYOUT_MOVES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlacePhase {
    /// Waiting for the open editor.
    Enter,
    /// Choosing the owned catalog row.
    Select,
    /// Turning the selection a quarter turn at a time.
    Rotate,
    /// Moving the selection onto the wanted tile.
    Move,
    /// Deciding the placement.
    Decide,
    /// Saving the draft.
    Save,
}

/// `layout.place`: one owned fixture through the layout editor's own commands.
/// Each command object is the one `moly.layout.command` takes and is read by
/// that method's own reader, so a step and the editor's buttons, list cells
/// and drags go through the same checks; every phase reads the editor's view
/// to see what its command did. The editor stays open afterwards, as it does
/// after the same sequence of buttons.
struct LayoutPlace {
    fixture: i32,
    texture: Option<u32>,
    center: [i8; 3],
    rotate: u32,
    decide: bool,
    save: bool,
    phase: PlacePhase,
    /// The catalog row chosen, the turns and the moves sent.
    row: Option<usize>,
    turns: u32,
    moves: u32,
    /// The tile the next move names: `move_to` takes the touched tile, and
    /// the reported center can stand off it by a tile.
    target: [i32; 3],
    /// The selection the moves belong to.
    uid: String,
    /// The direction the turn in flight started from.
    direction: Option<String>,
    /// A command of this phase is in flight, and when it went out.
    sent: bool,
    sent_at: f32,
    save_sent: bool,
    /// When the save confirmation was last tapped.
    tapped_at: f32,
}

impl LayoutPlace {
    fn new(fixture: i32, texture: Option<u32>, center: [i8; 3], rotate: u32, decide: bool, save: bool) -> Self {
        Self {
            fixture,
            texture,
            center,
            rotate,
            decide,
            save,
            phase: PlacePhase::Enter,
            row: None,
            turns: 0,
            moves: 0,
            target: [center[0] as i32, center[1] as i32, center[2] as i32],
            uid: String::new(),
            direction: None,
            sent: false,
            sent_at: 0.0,
            save_sent: false,
            tapped_at: f32::NEG_INFINITY,
        }
    }

    /// The command the current phase waits on.
    fn word(&self) -> &'static str {
        match self.phase {
            PlacePhase::Enter => "enter",
            PlacePhase::Select => "select_catalog",
            PlacePhase::Rotate => "rotate",
            PlacePhase::Move => "move_to",
            PlacePhase::Decide => "decide",
            PlacePhase::Save => "save",
        }
    }

    fn begin(&mut self, phase: PlacePhase, now: f32) {
        self.phase = phase;
        self.sent = false;
        self.sent_at = now;
    }

    /// Sends one command through the control surface's own reader and marks
    /// this phase as waiting for its effect.
    fn send(&mut self, world: &mut World, params: Value, now: f32) -> Poll {
        let queued = match layout_command(world, params) {
            Ok(queued) => queued,
            Err(error) => return Poll::Failed(error),
        };
        self.sent = true;
        self.sent_at = now;
        Poll::Running(format!("sent {queued}"))
    }

    /// What the view shows now, for a wait that has to report itself.
    fn reading(&self, view: &crate::fixture_edit::EditView) -> String {
        let selected = match &view.selected {
            Some(selected) => format!(
                "{} fixture {} at [{}, {}, {}] {:?}",
                selected.item.uid,
                selected.item.fixture_id,
                selected.item.center.x,
                selected.item.center.y,
                selected.item.center.z,
                selected.put_status
            ),
            None => "no selection".to_owned(),
        };
        format!("{selected}; {} placed rows; feedback {}", view.placed_rows.len(), view.feedback)
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared) -> Poll {
        let Some(view) = world.get_resource::<crate::fixture_edit::EditView>().cloned() else {
            return Poll::Failed("the layout editor is not set up".into());
        };
        let now = shared.now;
        if self.phase != PlacePhase::Enter && !view.active {
            return Poll::Failed("the layout editor closed during the placement".into());
        }
        // The editor answers in the frame after a command arrives. While its
        // own save confirmation is up, the save has an answer to wait for.
        if self.sent && self.phase != PlacePhase::Save && now - self.sent_at > LAYOUT_ANSWER_SECS {
            return Poll::Failed(format!(
                "the {} command was not answered in {LAYOUT_ANSWER_SECS:.0} s ({})",
                self.word(),
                self.reading(&view)
            ));
        }
        match self.phase {
            PlacePhase::Enter => {
                if view.active {
                    self.begin(PlacePhase::Select, now);
                    return Poll::Running("the layout editor is open".into());
                }
                if self.sent {
                    return Poll::Running("waiting for the layout editor to open".into());
                }
                self.send(world, json!({ "command": "enter" }), now)
            }
            PlacePhase::Select => {
                if let Some(selected) = view.selected.as_ref() {
                    if selected.item.fixture_id != self.fixture
                        || self.texture.is_some_and(|texture| selected.item.texture_id != texture)
                    {
                        return Poll::Failed(format!(
                            "the editor selected fixture {} texture {} instead of fixture {}{}",
                            selected.item.fixture_id,
                            selected.item.texture_id,
                            self.fixture,
                            self.texture.map_or(String::new(), |texture| format!(" texture {texture}"))
                        ));
                    }
                    self.uid = selected.item.uid.clone();
                    self.begin(PlacePhase::Rotate, now);
                    return Poll::Running(format!("selected {} of the owned catalog", self.uid));
                }
                if self.sent {
                    return Poll::Running(format!("waiting for catalog row {} to be selected", self.row.unwrap_or_default()));
                }
                let row = view.catalog.iter().find(|row| {
                    row.fixture_id == self.fixture
                        && self.texture.is_none_or(|texture| row.texture_id == texture)
                        && row.placeable > 0
                });
                let Some(row) = row else {
                    return Poll::Failed(self.no_row(&view));
                };
                self.row = Some(row.index);
                self.send(world, json!({ "command": "select_catalog", "index": row.index }), now)
            }
            PlacePhase::Rotate => {
                let Some(selected) = view.selected.as_ref() else {
                    return Poll::Failed(format!("the selection was dropped before the turns ({})", view.feedback));
                };
                if self.turns == self.rotate {
                    self.begin(PlacePhase::Move, now);
                    return Poll::Running(format!("turned {} quarter turn(s)", self.turns));
                }
                let direction = format!("{:?}", selected.item.direction);
                if self.sent {
                    if self.direction.as_deref() != Some(direction.as_str()) {
                        self.turns += 1;
                        self.direction = None;
                        self.sent = false;
                        return Poll::Running(format!("turn {} of {} done, now {direction}", self.turns, self.rotate));
                    }
                    return Poll::Running(format!("waiting for the turn; the direction is {direction}"));
                }
                self.direction = Some(direction);
                self.send(world, json!({ "command": "rotate" }), now)
            }
            PlacePhase::Move => {
                let Some(selected) = view.selected.as_ref() else {
                    return Poll::Failed(format!("the selection was dropped before the move ({})", view.feedback));
                };
                let at = selected.item.center;
                if at.x == self.center[0] && at.z == self.center[2] {
                    self.begin(PlacePhase::Decide, now);
                    return Poll::Running(format!("the fixture is on tile [{}, {}, {}]", at.x, at.y, at.z));
                }
                if self.sent {
                    return Poll::Running(format!("waiting for the move; the center is [{}, {}, {}]", at.x, at.y, at.z));
                }
                if self.moves == LAYOUT_MOVES {
                    return Poll::Failed(format!(
                        "the editor kept the fixture's center at [{}, {}, {}] instead of tile [{}, ?, {}] ({})",
                        at.x, at.y, at.z, self.center[0], self.center[2], view.feedback
                    ));
                }
                // move_to takes the tile a drag touched; the difference
                // between it and the reported center is added on the next
                // send, as a drag correcting itself does.
                let target = [
                    self.target[0] + self.center[0] as i32 - at.x as i32,
                    self.center[1] as i32,
                    self.target[2] + self.center[2] as i32 - at.z as i32,
                ];
                let (Ok(x), Ok(y), Ok(z)) = (
                    i8::try_from(target[0]),
                    i8::try_from(target[1]),
                    i8::try_from(target[2]),
                ) else {
                    return Poll::Failed(format!("the corrected tile {target:?} is outside the layout grid"));
                };
                self.target = target;
                self.moves += 1;
                self.send(world, json!({ "command": "move_to", "center": [x, y, z] }), now)
            }
            PlacePhase::Decide => {
                if view.selected.is_none() {
                    let placed = view.placed_rows.iter().find(|row| row.uid == self.uid);
                    let Some(placed) = placed else {
                        return Poll::Failed(format!(
                            "the editor lists no fixture with uid {} after the decision ({})",
                            self.uid, view.feedback
                        ));
                    };
                    let at = placed.center;
                    let note = format!("placed fixture {} ({}) at [{}, {}, {}]", self.fixture, self.uid, at.x, at.y, at.z);
                    if !self.save {
                        return Poll::Done(note);
                    }
                    self.begin(PlacePhase::Save, now);
                    return Poll::Running(format!("{note}; saving"));
                }
                let selected = view.selected.as_ref().expect("checked above");
                if let crate::fixture_edit::PutStatus::Refused(refusal) = selected.put_status {
                    return Poll::Failed(format!(
                        "the editor refuses to put fixture {} on tile [{}, {}, {}]: {refusal:?}",
                        self.fixture,
                        selected.item.center.x,
                        selected.item.center.y,
                        selected.item.center.z
                    ));
                }
                if self.sent {
                    return Poll::Running("waiting for the decision".into());
                }
                self.send(world, json!({ "command": "decide" }), now)
            }
            PlacePhase::Save => {
                if view.save_dialog {
                    // The editor asks before it saves over a cutscene area or
                    // with invalid fixtures listed; its OK is the answer a
                    // player gives.
                    if now - self.tapped_at < 1.0 {
                        return Poll::Running("waiting for the save confirmation to close".into());
                    }
                    let (_, caller, key, node) = match crate::common_dialog::shown_button(world, true) {
                        Ok(button) => button,
                        Err(error) => return Poll::Running(error),
                    };
                    let position = match hand::ui_point(world, key, &node) {
                        Ok(position) => position,
                        Err(error) => return Poll::Running(error),
                    };
                    return match hand::tap(world, position) {
                        Ok(()) => {
                            self.tapped_at = now;
                            Poll::Running(format!("tapped OK on {key} from {caller}"))
                        }
                        Err(error) => Poll::Running(error.to_owned()),
                    };
                }
                if self.save_sent {
                    if !view.dirty {
                        let at = view.placed_rows.iter().find(|row| row.uid == self.uid).map(|row| row.center);
                        return Poll::Done(match at {
                            Some(at) => format!("placed fixture {} ({}) at [{}, {}, {}] and saved", self.fixture, self.uid, at.x, at.y, at.z),
                            None => format!("placed fixture {} ({}) and saved", self.fixture, self.uid),
                        });
                    }
                    if now - self.sent_at > LAYOUT_ANSWER_SECS {
                        return Poll::Failed(format!(
                            "the save left the draft dirty for {LAYOUT_ANSWER_SECS:.0} s (selected {}, save dialog {})",
                            view.selected.is_some(),
                            view.save_dialog
                        ));
                    }
                    return Poll::Running("waiting for the save".into());
                }
                if !view.can_save {
                    return Poll::Running(format!(
                        "waiting to save (selected {}, dialog {})",
                        view.selected.is_some(),
                        view.save_dialog
                    ));
                }
                self.save_sent = true;
                self.send(world, json!({ "command": "save" }), now)
            }
        }
    }

    /// Why no catalog row can be selected.
    fn no_row(&self, view: &crate::fixture_edit::EditView) -> String {
        let rows: Vec<usize> = view
            .catalog
            .iter()
            .filter(|row| row.fixture_id == self.fixture)
            .map(|row| row.index)
            .collect();
        let texture = self.texture.map_or(String::new(), |texture| format!(" texture {texture}"));
        if rows.is_empty() {
            format!("fixture {}{texture} is not in the owned catalog", self.fixture)
        } else {
            format!("fixture {}{texture}: every owned copy is placed (catalog rows {rows:?})", self.fixture)
        }
    }
}

/// One layout editor command built the way `moly.layout.command` builds it.
fn layout_command(world: &mut World, params: Value) -> Result<String, String> {
    let Value::Object(params) = params else {
        return Err("a layout command is an object".to_owned());
    };
    let command = crate::control::layout::command(&params)?;
    let queued = format!("{command:?}");
    world.write_message(command);
    Ok(queued)
}

/// A tap that has started no delivery after this long (seconds) is refused.
const DELIVER_ANSWER_SECS: f32 = 2.0;

/// `player.deliver`: the delivery button's press and release, the two
/// requests the delivery screen sends, on a birthday party in session. The
/// delivery's own rules decide what a press does (the release window,
/// `CanDelivery`, the ends of the hold loop and the end action); the step
/// ends once the end action has run, with the points the party gained.
struct Deliver {
    /// The party the press named.
    party: i32,
    /// Seconds to hold the button; none holds it until the party has nothing
    /// left to deliver.
    hold: Option<f32>,
    /// When the press went out, and when the delivery was seen running.
    asked_at: f32,
    started_at: Option<f32>,
    /// The release has been sent.
    released: bool,
    points_at_press: i32,
    /// The refresh dialog's back key was pressed.
    closed_refresh: bool,
}

impl Deliver {
    /// The press: refused before it is sent when no party is in session.
    fn press(world: &mut World, party: Option<i32>, hold: Option<f32>, now: f32) -> Result<Self, String> {
        let Some(model) = world.get_resource::<crate::delivery::DeliveryModel>() else {
            return Err("the delivery screen is not set up".to_owned());
        };
        if !model.parties.iter().any(|site| party.is_none_or(|id| site.id == id)) {
            return Err(match party {
                Some(id) => format!("no birthday party {id} is in session"),
                None => "no birthday party is in session".to_owned(),
            });
        }
        let id = party.unwrap_or_else(|| model.parties[0].id);
        let points = model.party(id).map_or(0, |site| site.tally.current_points());
        world.write_message(crate::delivery::DeliveryRequest::Start(Some(id)));
        Ok(Self {
            party: id,
            hold,
            asked_at: now,
            started_at: None,
            released: false,
            points_at_press: points,
            closed_refresh: false,
        })
    }

    fn poll(&mut self, world: &mut World, shared: &mut Shared) -> Poll {
        let Some(model) = world.get_resource::<crate::delivery::DeliveryModel>() else {
            return Poll::Failed("the delivery screen is not set up".into());
        };
        let state = model.state;
        let executing = model.executing;
        let home = world
            .get_resource::<crate::site::SiteActive>()
            .map(|site| site.site_type.clone());
        let (points, can, left) = model
            .party(self.party)
            .map_or((0, false, 0), |site| {
                (site.tally.current_points(), site.tally.can_delivery(), site.tally.remaining())
            });
        if !executing && self.started_at.is_none() {
            // The press starts the flow in the frame after the request; a
            // press the flow refuses leaves the state where it was.
            if shared.now - self.asked_at <= DELIVER_ANSWER_SECS {
                return Poll::Running(format!("waiting for the delivery of party {} to start (state {state:?})", self.party));
            }
            return Poll::Failed(format!(
                "the press on party {} started no delivery (state {state:?}, {} left to deliver, site {home:?})",
                self.party, left
            ));
        }
        if self.started_at.is_none() {
            self.started_at = Some(shared.now);
        }
        if !self.released {
            let held = self.hold.is_some_and(|hold| shared.now - self.asked_at >= hold);
            if held || !can {
                world.write_message(crate::delivery::DeliveryRequest::End);
                self.released = true;
                return Poll::Running(format!(
                    "released the delivery button ({}, {left} left to deliver)",
                    if held { "held long enough" } else { "nothing left to deliver" }
                ));
            }
        }
        if !executing && state == crate::delivery::DeliveryActionState::Idle {
            return Poll::Done(format!(
                "party {} delivered: points {} to {}",
                self.party, self.points_at_press, points
            ));
        }
        // The end action waits for the refresh dialog's close, which the back
        // key gives it.
        if !self.closed_refresh
            && world
                .get_resource::<crate::delivery::honor::DialogAwait>()
                .is_some_and(|awaiting| awaiting.refresh.is_some())
        {
            self.closed_refresh = true;
            return match hand::back_key(world) {
                Ok(()) => Poll::Running("pressed the back key on the refresh dialog".into()),
                Err(error) => Poll::Running(error.to_owned()),
            };
        }
        Poll::Running(format!(
            "delivering party {}: state {state:?}, {points} points, {left} left to deliver",
            self.party
        ))
    }
}

/// `dialog.answer`: a tap on the shown common dialog's OK or cancel button,
/// done once the dialog is gone. A sub-window dialog with no common button
/// (the learn-phenomenon one) is answered with the back key its own class
/// registers, which is the answer it takes from the screen.
struct DialogAnswer {
    ok: bool,
    tapped: Option<(crate::ui_layers::DialogId, f32)>,
    /// The sub-window dialog answered with the back key, and when.
    pressed: Option<(crate::ui_layers::DialogId, f32)>,
}

/// Whether the screen manager still shows the dialog `id`.
fn dialog_open(world: &World, id: crate::ui_layers::DialogId) -> bool {
    world.get_resource::<crate::ui_layers::ScreenManager>().is_some_and(|screens| {
        screens
            .dialogs()
            .any(|(open, _, state)| open == id && state != crate::ui_layers::DialogState::Closed)
    })
}

/// The topmost dialog still open, with its handle: the one the screen
/// manager hands the back key to.
fn topmost_dialog(world: &World) -> Option<(crate::ui_layers::DialogId, crate::ui_layers::DialogType)> {
    world
        .get_resource::<crate::ui_layers::ScreenManager>()?
        .dialogs()
        .filter(|(_, _, state)| *state != crate::ui_layers::DialogState::Closed)
        .map(|(id, dialog, _)| (id, dialog))
        .last()
}

impl DialogAnswer {
    fn poll(&mut self, world: &mut World, shared: &mut Shared) -> Poll {
        if let Some((id, at)) = self.pressed {
            if !dialog_open(world, id) {
                return Poll::Done(format!("closed the dialog ({})", if self.ok { "answer" } else { "cancel" }));
            }
            if shared.now - at < 1.0 {
                return Poll::Running("waiting for the dialog to close".into());
            }
        }
        if let Some((id, at)) = self.tapped {
            if !crate::common_dialog::is_shown(world, id) {
                return Poll::Done(format!("answered {}", if self.ok { "OK" } else { "cancel" }));
            }
            if shared.now - at < 1.0 {
                return Poll::Running("waiting for the dialog to close".into());
            }
        }
        let (id, caller, view, node) = match crate::common_dialog::shown_button(world, self.ok) {
            Ok(button) => button,
            Err(error) => return self.answer_without_button(world, shared, error),
        };
        let position = match hand::ui_point(world, view, &node) {
            Ok(position) => position,
            Err(error) => return Poll::Running(error),
        };
        match hand::tap(world, position) {
            Ok(()) => {
                self.tapped = Some((id, shared.now));
                Poll::Running(format!("tapped {} on {view} from {caller}", if self.ok { "OK" } else { "cancel" }))
            }
            Err(error) => Poll::Running(error.to_owned()),
        }
    }

    /// The shown dialog that has no common button. The learn-phenomenon one
    /// takes the back key its own class registers as its whole answer: its
    /// close area, its panel body and the hardware back key all call
    /// `CloseProcess -> Close`, and the run that opened it waits for its
    /// close. Another family's dialog is named and left alone.
    fn answer_without_button(&mut self, world: &mut World, shared: &mut Shared, error: String) -> Poll {
        let Some((id, dialog)) = topmost_dialog(world) else {
            return Poll::Running(error);
        };
        if dialog != crate::ui_layers::DialogType::LearnPhenomenaSubWindowDialog {
            return Poll::Running(format!("{error}; the topmost dialog is {dialog:?}"));
        }
        match hand::back_key(world) {
            Ok(()) => {
                self.pressed = Some((id, shared.now));
                Poll::Running(format!("pressed the back key on {dialog:?}"))
            }
            Err(error) => Poll::Running(error.to_owned()),
        }
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
