//! The executor: tracks of steps advanced once a frame. A track runs its
//! steps in order; `parallel` runs child tracks side by side and `repeat`
//! runs a block again. A step that ends on the frame it starts (a log, a
//! take, an assert) lets the next one start on the same frame.
//!
//! A flow step is the runner's own (`Kind`). An action step is an action of
//! the hub's registry (`Node::Act`): the runner takes the hand for the
//! step's track, starts every spec the step stands for in turn and polls the
//! run until it ends, fails, times out, or another holder takes a lease from
//! it. The runner keeps what is the script's own: the log, the hand, the
//! characters its tracks took and the fixture activities they started.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use bevy::prelude::*;
use moly_features::act::{hand, ActionRun, ActionSpec, ActCtx, ActPoll, Card, Cond, Leaving, Taken};
use moly_features::lease::{Channel, Holder, Leases, Rank};

use super::script::{Join, Node, Step};

/// The ticket offset the steps asked for outside a script advance with:
/// their own ticket, that high in the range, so the director's run (ticket
/// `0`) and theirs never meet.
pub const ACTED_TICKET: u64 = 1 << 32;

/// Steps one track may start on one frame.
const STEPS_PER_FRAME: usize = 64;
/// Records the log keeps.
pub const RECORDS: usize = 256;

/// The name the director's own run takes its leases under.
pub const DIRECTOR: &str = "director";

/// What a script's hand step is refused with when another plugin's step
/// holds the hand, and what another plugin's hand step is refused with while
/// a director script holds it.
pub const HAND_HELD: &str = "the hand is held by another plugin's step";
pub const DIRECTOR_HOLDS: &str = "a director script holds the hand";

/// One line of the run's log.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Line {
    pub t: f32,
    pub step: String,
    pub event: &'static str,
    pub note: String,
}

/// What a frame's poll of a step says.
pub type Poll = ActPoll;

/// The flow part of a step, or the action the registry runs for it.
pub enum Kind {
    Wait { until: f32 },
    WaitUntil(Cond),
    Parallel { tracks: Vec<Track>, join: Join },
    /// An action of the registry.
    Act(ActStep),
}

/// The actions of one step.
pub struct ActStep {
    /// The specs the step stands for, in the order they run.
    specs: Arc<[Arc<dyn ActionSpec>]>,
    /// The index of the next spec to start.
    next: usize,
    /// The spec running now, and the run.
    run: Option<Box<dyn ActionRun>>,
    /// The holder the step's actions take their leases under.
    holder: String,
    /// The ticket of the spec running now: its characters and activities.
    step_ticket: u64,
    /// The card of the spec running now, kept with it when it ends.
    card: Card,
    /// Every card the step's specs laid, for the steps after it and the run's
    /// end.
    laid: Vec<Leaving>,
}

/// State shared by every track of a run.
pub struct Shared {
    pub log: VecDeque<Line>,
    pub now: f32,
    /// Bumped on every record, so the snapshot is republished only on change.
    pub revision: u64,
    /// The last steps that ended (at most [`RECORDS`]) with their tickets, for
    /// a step asked for outside a script to report its result.
    results: VecDeque<(u64, Line)>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

impl Shared {
    /// A run with nothing in it yet.
    pub fn new() -> Self {
        Self { log: VecDeque::new(), now: 0.0, revision: 0, results: VecDeque::new() }
    }

    pub fn record(&mut self, step: &str, event: &'static str, note: String) {
        let line = format!("[director] {step}: {event}{}{note}", if note.is_empty() { "" } else { ": " });
        match event {
            "failed" => warn!("{line}"),
            _ => info!("{line}"),
        }
        if self.log.len() == RECORDS {
            self.log.pop_front();
        }
        self.log.push_back(Line { t: self.now, step: step.to_owned(), event, note });
        self.revision += 1;
    }

    /// Notes a step of `ticket`'s run that ended, with its event and note, for
    /// a step asked for outside a script to report as its result.
    fn note_ended(&mut self, ticket: u64, event: &'static str, note: String) {
        if self.results.len() == RECORDS {
            self.results.pop_front();
        }
        self.results.push_back((ticket, Line { t: self.now, step: String::new(), event, note }));
        self.revision += 1;
    }

    /// How many steps ended so far, every run's together.
    pub fn ended(&self) -> usize {
        self.results.len()
    }

    /// The notes of the steps `ticket`'s run ended since `from`, in order.
    pub fn ended_after(&self, ticket: u64, from: usize) -> Vec<&str> {
        self.results
            .iter()
            .skip(from)
            .filter(|(owner, _)| *owner == ticket)
            .map(|(_, record)| record.note.as_str())
            .collect::<Vec<&str>>()
    }

    /// What a step that ended leaves the steps after it: a character it
    /// handed back to its AI is its AI's own from now on, and a later step
    /// that plays with it takes it again.
    pub fn keep(&mut self, world: &mut World, cards: &[Leaving]) {
        for leaving in cards {
            if let Leaving::Talk(entity) = leaving {
                Taken::hand_back(world, *entity);
            }
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

/// A sequence of steps, run as the ticket `ticket`.
pub struct Track {
    frames: Vec<Frame>,
    active: Option<Active>,
    ticket: u64,
}

/// How a track stands after a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackState {
    Running,
    Done,
    Failed(String),
}

/// The ticket of `base`'s `index`th part, parallel track or step: a run's
/// parallel tracks and the parts of its steps each keep their characters and
/// activities apart.
fn sub_ticket(base: u64, index: u64) -> u64 {
    (base << 8) | index
}

impl Track {
    /// A track of `block` that runs as the ticket `ticket`: the script's own
    /// run is `0`, a plugin's step its offset ticket, and a parallel track or
    /// a step's part one of its own (`sub_ticket`).
    pub fn new(block: Arc<[Step]>, path: String, ticket: u64) -> Self {
        Self { frames: vec![Frame { block, next: 0, repeats_left: 0, path }], active: None, ticket }
    }

    /// The ticket the track's characters and activities belong to.
    pub fn ticket(&self) -> u64 {
        self.ticket
    }

    /// The steps the track was built with.
    pub fn steps(&self) -> &[Step] {
        &self.frames[0].block
    }

    /// The paths of the steps running now, innermost last.
    pub fn current(&self, out: &mut Vec<String>) {
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
    pub fn abort(&mut self, world: &mut World, shared: &mut Shared, tickets: &mut BTreeSet<u64>) {
        if let Some(mut active) = self.active.take() {
            if let Kind::Parallel { tracks, .. } = &mut active.kind {
                for track in tracks {
                    track.abort(world, shared, tickets);
                }
            }
            stop(world, shared, tickets, &active.path, &mut active.kind);
            shared.record(&active.path, "stopped", String::new());
        }
        self.frames.clear();
    }

    /// Runs the track's steps for this frame, noting in `tickets` the tickets
    /// of the parts it starts.
    pub fn tick(&mut self, world: &mut World, shared: &mut Shared, tickets: &mut BTreeSet<u64>) -> TrackState {
        let ticket = self.ticket;
        for _ in 0..STEPS_PER_FRAME {
            if self.active.is_none() {
                let Some((step, path)) = self.next_step() else {
                    return TrackState::Done;
                };
                match start(world, shared, ticket, step, path, tickets) {
                    Started::Ended(path, Ok(note)) => {
                        shared.record(&path, "done", note.clone());
                        shared.note_ended(ticket, "done", note);
                        continue;
                    }
                    Started::Ended(path, Err((optional, note))) => {
                        shared.record(&path, "failed", note.clone());
                        shared.note_ended(ticket, "failed", note.clone());
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
                _ => poll(world, shared, ticket, active, tickets),
            };
            match poll {
                Poll::Running(note) => {
                    active.note = note;
                    return TrackState::Running;
                }
                Poll::Done(note) => {
                    let mut active = self.active.take().expect("polled above");
                    release_step(world, ticket, &active.path, &mut active.kind);
                    shared.record(&active.path, "done", note.clone());
                    shared.note_ended(ticket, "done", note);
                }
                Poll::Failed(note) => {
                    let mut active = self.active.take().expect("polled above");
                    if let Kind::Parallel { tracks, .. } = &mut active.kind {
                        for track in tracks {
                            track.abort(world, shared, tickets);
                        }
                    }
                    stop(world, shared, tickets, &active.path, &mut active.kind);
                    shared.record(&active.path, "failed", note.clone());
                    shared.note_ended(ticket, "failed", note.clone());
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

/// The name a step's actions take their leases under: the script's run and
/// the step, or the plugin's step.
fn holder_name(ticket: u64, path: &str) -> String {
    if ticket & ACTED_TICKET == 0 {
        format!("{DIRECTOR}:{path}")
    } else {
        format!("plugin:{}:{path}", ticket & !ACTED_TICKET)
    }
}

/// Takes the hand for the step `path` of the run `ticket`, refused with the
/// word the hand refuses with: a script's step names the plugin that holds
/// the hand or the director's own, and a plugin's step names the director's
/// script or another plugin's step.
pub fn take_hand(world: &mut World, ticket: u64, path: &str) -> Result<(), String> {
    let holder = Holder::new(holder_name(ticket, path), Rank::Director);
    let mut leases = world.resource_mut::<Leases>();
    if leases.holder(&Channel::Hand) == Some(&holder) {
        return Ok(());
    }
    let script = ticket & ACTED_TICKET == 0;
    if let Some(sitting) = leases.holder(&Channel::Hand).filter(|sitting| sitting.rank >= Rank::Director) {
        if script {
            return Err(if sitting.id.starts_with("plugin:") { HAND_HELD.to_owned() } else { DIRECTOR_HOLDS.to_owned() });
        }
        // A plugin's step names the holder: the director's script, or the
        // plugin whose step holds the hand.
        let other = sitting.id.strip_prefix("plugin:").and_then(|rest| rest.split(':').next()).unwrap_or_default();
        return Err(if other.is_empty() { HAND_HELD.to_owned() } else { format!("{HAND_HELD} ({other})") });
    }
    match leases.take(Channel::Hand, &holder) {
        Ok(()) => Ok(()),
        Err(_) => Err(HAND_HELD.to_owned()),
    }
}

fn start(
    world: &mut World,
    shared: &mut Shared,
    ticket: u64,
    step: Step,
    path: String,
    tickets: &mut BTreeSet<u64>,
) -> Started {
    shared.record(&path, "start", String::new());
    let optional = step.optional;
    let kind = match &step.node {
        Node::Wait { secs } => Kind::Wait { until: shared.now + secs },
        Node::WaitUntil { until } => Kind::WaitUntil(until.clone()),
        Node::Assert { that } => {
            let (holds, note) = moly_features::act::eval(world, that);
            let result = if holds { Ok(note) } else { Err(format!("does not hold: {note}")) };
            return Started::Ended(path, result.map_err(|note| (optional, note)));
        }
        Node::Log { text } => return Started::Ended(path, Ok(text.clone())),
        Node::Where { of } => return Started::Ended(path, Ok(where_notes(world, of))),
        Node::Repeat { .. } => unreachable!("repeat blocks are entered by next_step"),
        Node::Capture => {
            world.write_message(moly_features::frame_capture::CaptureFrame);
            return Started::Ended(path, Ok("frame capture requested".into()));
        }
        Node::Parallel { tracks, join } => {
            let inner: Vec<Track> = tracks
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    let sub = sub_ticket(ticket, index as u64 + 1);
                    tickets.insert(sub);
                    Track::new(block.clone(), format!("{path}[{}]", index + 1), sub)
                })
                .collect();
            Kind::Parallel { tracks: inner, join: *join }
        }
        Node::Act(specs) => {
            if specs.is_empty() {
                return Started::Ended(path, Ok(String::new()));
            }
            tickets.insert(sub_ticket(ticket, 1));
            match act_step(world, ticket, &path, specs.clone()) {
                Ok(act) => Kind::Act(act),
                Err(note) => return Started::Ended(path, Err((optional, note))),
            }
        }
    };
    let deadline = step.timeout.map(|timeout| shared.now + timeout);
    Started::Running(Active { step, path, deadline, kind, note: String::new() })
}

/// The steps a registry action starts as, and the holder of its leases.
fn act_step(world: &mut World, ticket: u64, path: &str, specs: Arc<[Arc<dyn ActionSpec>]>) -> Result<ActStep, String> {
    if specs.iter().any(|spec| spec.hand()) {
        take_hand(world, ticket, path)?;
    }
    Ok(ActStep {
        specs,
        next: 0,
        run: None,
        holder: holder_name(ticket, path),
        step_ticket: sub_ticket(ticket, 1),
        card: Card::new(),
        laid: Vec::new(),
    })
}

fn poll(world: &mut World, shared: &mut Shared, ticket: u64, active: &mut Active, tickets: &mut BTreeSet<u64>) -> Poll {
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
            let (holds, note) = moly_features::act::eval(world, until);
            if holds {
                Poll::Done(note)
            } else {
                Poll::Running(note)
            }
        }
        Kind::Parallel { tracks, join } => {
            let mut running = 0;
            let mut end = None;
            for track in tracks.iter_mut() {
                match track.tick(world, shared, tickets) {
                    TrackState::Running => running += 1,
                    TrackState::Done if *join == Join::Any => {
                        end = Some(Poll::Done("one track ended".into()));
                        break;
                    }
                    TrackState::Done => {}
                    TrackState::Failed(error) => {
                        end = Some(Poll::Failed(error));
                        break;
                    }
                }
            }
            if let Some(end) = end {
                for track in tracks.iter_mut() {
                    track.abort(world, shared, tickets);
                }
                return end;
            }
            if running == 0 {
                Poll::Done(String::new())
            } else {
                Poll::Running(format!("{running} tracks running"))
            }
        }
        Kind::Act(act) => {
            if act.run.is_none() {
                if act.next >= act.specs.len() {
                    return Poll::Done(String::new());
                }
                let spec = act.specs[act.next].clone();
                act.next += 1;
                // Each part of a step is a track of its own: the first is
                // the step's ticket, the ones after it their own numbers from
                // the step's place.
                if act.next > 1 {
                    act.step_ticket = sub_ticket(sub_ticket(ticket, 1), act.next as u64);
                    tickets.insert(act.step_ticket);
                }
                let holder = holder_name(ticket, &path);
                let mut ctx = ActCtx::new(Holder::new(holder, Rank::Director), act.step_ticket, &path, shared.now);
                ctx.card = Card::new();
                return match spec.start(world, &mut ctx) {
                    Ok(run) => {
                        act.run = Some(run);
                        act.card = ctx.card;
                        Poll::Running(String::new())
                    }
                    Err(error) => Poll::Failed(error),
                };
            }
            // A lease taken from the step's holder ends it with the reason
            // the taker gave.
            let holder = act.holder.clone();
            let revoked = world.resource_mut::<Leases>().revoked(&holder).first().map(|(_, reason)| reason.clone());
            if let Some(reason) = revoked {
                return Poll::Failed(reason);
            }
            let mut ctx = ActCtx::new(Holder::new(holder, Rank::Director), act.step_ticket, &path, shared.now);
            ctx.card = std::mem::take(&mut act.card);
            let run = act.run.as_mut().expect("started above");
            let poll = run.poll(world, &mut ctx);
            if !matches!(poll, ActPoll::Running(_)) {
                act.run = None;
                act.laid.extend(ctx.card.all().iter().cloned());
                shared.keep(world, ctx.card.all());
            }
            act.card = ctx.card;
            poll
        }
    }
}

/// Lets a step that ended give its hand and camera back; its characters and
/// activities stay its track's.
fn release_step(world: &mut World, ticket: u64, path: &str, kind: &mut Kind) {
    if let Kind::Act(act) = kind {
        hand::release_of(world, ticket, path);
        let holder = act.holder.clone();
        let mut leases = world.resource_mut::<Leases>();
        leases.release(&Channel::Hand, &holder);
        leases.release(&Channel::Camera, &holder);
    }
}

/// Ends a step early: its run stopped, its hand lifted, its characters given
/// back to their AI and its activities ended.
fn stop(world: &mut World, shared: &mut Shared, _tickets: &mut BTreeSet<u64>, path: &str, kind: &mut Kind) {
    if let Kind::Act(act) = kind {
        if let Some(mut run) = act.run.take() {
            let holder = act.holder.clone();
            let mut ctx = ActCtx::new(Holder::new(holder, Rank::Director), act.step_ticket, path, shared.now);
            run.stop(world, &mut ctx);
        }
        let mut laid = std::mem::take(&mut act.laid);
        laid.extend_from_slice(act.card.all());
        for leaving in laid {
            match leaving {
                Leaving::Activity(activity) => moly_features::npc_fixture_activity::preview::cancel(world, activity),
                Leaving::Talk(entity) => Taken::hand_back(world, entity),
            }
        }
        let holder = act.holder.clone();
        hand::release_of(world, act.step_ticket, path);
        world.resource_mut::<Leases>().release_prefix(&holder);
    }
}

/// What a `where` step writes: the player's position and each target's.
fn where_notes(world: &mut World, of: &[moly_features::act::Target]) -> String {
    let mut notes = vec![match moly_features::act::targets::player(world) {
        Ok(at) => format!("player ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z),
        Err(error) => error,
    }];
    for target in of {
        notes.push(match moly_features::act::targets::resolve(world, target) {
            Ok(at) => format!("{:?} ({:.2}, {:.2}, {:.2})", target.what, at.x, at.y, at.z),
            Err(error) => format!("{:?}: {error}", target.what),
        });
    }
    notes.join("; ")
}
