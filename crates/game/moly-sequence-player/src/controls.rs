//! The control surface's commands and the events' actions.
//!
//! A command is a load, a play, a pause, a seek, a speed or a stop, run
//! against the world on the player's frame, so the state a status answer
//! reads is the state the frame last laid. The methods queue a command and
//! take the answer of the next frame; the `sequence.play` and
//! `sequence.stop` actions call the commands directly, in their callers'
//! frames.
//!
//! The events' actions are parsed and started when forward playback crosses
//! their time, under the holder `sequence:events[track]` of the sequence
//! rank; the hand lease is taken first for a step that writes the hand, and
//! the finger another holder has down is lifted before that. Every run is
//! polled on each frame the player is not stopped ([`poll`]), and a refusal is
//! logged with its place; it does not stop the playback. A stop stops every
//! run.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use moly_features::act::{hand, ActCtx, ActPoll, ActionRun, ActionSpec, Card, Fields, Kinds, Leaving, Step, Taken};
use moly_features::lease::{Channel, Holder, Leases, Rank};
use serde_json::{Map, Value};

use crate::{refusal, Player, Refusal, Reply, State};

/// What a method asks the player to do.
#[derive(Clone, Debug)]
pub enum Command {
    Load { path: String },
    Play { from: Option<f32> },
    Pause,
    Seek { t: Option<f32>, marker: Option<String> },
    Speed { scale: f32 },
    Stop,
}

/// Runs one command against the world.
pub fn run(world: &mut World, command: &Command) -> Result<Reply, Refusal> {
    match command {
        Command::Load { path } => crate::load(world, std::path::Path::new(path)),
        Command::Play { from } => crate::play(world, *from),
        Command::Pause => crate::pause(world),
        Command::Seek { t: Some(t), .. } => crate::seek(world, *t),
        Command::Seek { t: None, marker: Some(marker) } => crate::seek_marker(world, marker),
        Command::Seek { .. } => Err(refusal("give t or marker")),
        Command::Speed { scale } => crate::set_speed(world, *scale),
        Command::Stop => Ok(crate::stop(world, "stopped by command")),
    }
}

/// The holder an event's action runs under.
fn event_holder(track: usize) -> Holder {
    Holder::new(format!("sequence:events[{track}]"), Rank::Sequence)
}

/// A step an event started.
pub(crate) struct Started {
    holder: Holder,
    ticket: u64,
    path: String,
    /// The specs the step stands for (a macro's parts), the next one, and
    /// the run in flight.
    specs: Arc<[Arc<dyn ActionSpec>]>,
    next: usize,
    active: Option<Box<dyn ActionRun>>,
    leavings: Vec<Leaving>,
}

impl Started {
    /// Ends the run in flight, without finishing it.
    fn stop_active(&mut self, world: &mut World, now: f32) {
        if let Some(mut active) = self.active.take() {
            let mut ctx = ActCtx::new(self.holder.clone(), self.ticket, &self.path, now);
            active.stop(world, &mut ctx);
            self.leavings.extend(ctx.card.all().iter().cloned());
        }
        hand::release_of(world, self.ticket, &self.path);
        release_leases(world, &self.holder);
    }
}

/// The runs the events started, and the number the next event takes.
#[derive(Default)]
pub(crate) struct Running {
    runs: HashMap<usize, Started>,
    next: usize,
}

/// Fires the events whose time the playback just crossed, in time order.
pub fn fire(world: &mut World, player: &mut Player, events: &[(f32, Value)]) {
    for (at, action) in events {
        let Some(name) = action.get("do").and_then(Value::as_str) else {
            warn!("[sequence] the event at {at:.3} s does not name its action in \"do\"");
            continue;
        };
        let track = player.events.next;
        player.events.next += 1;
        let holder = event_holder(track);
        let path = format!("events[{track}]");
        let fields: Map<String, Value> = match action.as_object() {
            Some(object) => object.iter().filter(|(key, _)| key.as_str() != "do").map(|(key, value)| (key.clone(), value.clone())).collect(),
            None => Map::new(),
        };
        let specs = match moly_features::act::parse(world, name, &fields, &path) {
            Ok(specs) => specs,
            Err(reason) => {
                warn!("[sequence] {path} at {at:.3} s: {reason}");
                continue;
            }
        };
        if specs.is_empty() {
            continue;
        }
        if specs.iter().any(|spec| spec.hand()) {
            if let Err(reason) = take_hand(world, &holder) {
                warn!("[sequence] {path} at {at:.3} s: {reason}");
                continue;
            }
        }
        info!("[sequence] {path} at {at:.3} s: {name}");
        player.events.runs.insert(
            track,
            Started { holder, ticket: crate::SEQUENCE_TICKET, path, specs: Arc::from(specs), next: 0, active: None, leavings: Vec::new() },
        );
    }
}

/// Takes the hand for a sequence step: the finger another holder has down is
/// lifted, and the lease is refused by name while a holder of the same or a
/// higher rank keeps it.
fn take_hand(world: &mut World, holder: &Holder) -> Result<(), String> {
    if let Some((owner, step)) = hand::leaning(world) {
        let step = step.to_owned();
        hand::release_of(world, owner, &step);
    }
    world.resource_mut::<Leases>().take(Channel::Hand, holder).map_err(|refusal| refusal.to_string())
}

/// Polls the event runs: each starts its next spec when the one before ends,
/// and the run ends when all of them have. A spec that keeps running is a
/// step of its own, so a long walk does not hold the run beside it, and a
/// step that has shown it is running is polled again next frame.
///
/// Polls every run the events started. The player's frame calls it on each
/// frame the player is not stopped; [`stop`] ends the runs after it.
pub(crate) fn poll(world: &mut World, player: &mut Player) {
    let now = world.resource::<Time>().elapsed_secs();
    let mut ended = Vec::new();
    let tracks: Vec<usize> = player.events.runs.keys().copied().collect();
    for track in tracks {
        let Some(run) = player.events.runs.get_mut(&track) else {
            continue;
        };
        let mut finished = false;
        loop {
            let mut progressed = false;
            if let Some(mut active) = run.active.take() {
                let mut ctx = ActCtx::new(run.holder.clone(), run.ticket, &run.path, now);
                ctx.card = Card::of(std::mem::take(&mut run.leavings));
                let result = active.poll(world, &mut ctx);
                run.leavings = ctx.card.all().to_vec();
                match result {
                    ActPoll::Running(_) => {
                        run.active = Some(active);
                        break;
                    }
                    ActPoll::Done(note) => info!("[sequence] {}: done ({note})", run.path),
                    ActPoll::Failed(reason) => {
                        warn!("[sequence] {}: failed: {reason}", run.path);
                        let mut ctx = ActCtx::new(run.holder.clone(), run.ticket, &run.path, now);
                        active.stop(world, &mut ctx);
                    }
                }
                hand::release_of(world, run.ticket, &run.path);
                release_leases(world, &run.holder);
                progressed = true;
            }
            if run.next >= run.specs.len() {
                finished = true;
                break;
            }
            if progressed {
                // The step's successor starts next frame, so a step that
                // spans frames is never confused with a poll of its own.
                break;
            }
            let spec = run.specs[run.next].clone();
            run.next += 1;
            let mut ctx = ActCtx::new(run.holder.clone(), run.ticket, &run.path, now);
            ctx.card = Card::of(std::mem::take(&mut run.leavings));
            match spec.start(world, &mut ctx) {
                Ok(active) => {
                    run.leavings = ctx.card.all().to_vec();
                    run.active = Some(active);
                    break;
                }
                Err(reason) => {
                    run.leavings = ctx.card.all().to_vec();
                    warn!("[sequence] {}: {} failed: {reason}", run.path, spec.name());
                    hand::release_of(world, run.ticket, &run.path);
                    release_leases(world, &run.holder);
                }
            }
        }
        if finished {
            ended.push(track);
        }
    }
    for track in ended {
        if let Some(mut run) = player.events.runs.remove(&track) {
            for leaving in std::mem::take(&mut run.leavings) {
                hand_back(world, leaving);
            }
            release_leases(world, &run.holder);
        }
    }
}

/// Lets go every lease a run's holder took.
fn release_leases(world: &mut World, holder: &Holder) {
    world.resource_mut::<Leases>().release_prefix(&holder.id);
}

/// Gives a card back: an activity is cancelled, a character handed to its AI.
fn hand_back(world: &mut World, leaving: Leaving) {
    match leaving {
        Leaving::Activity(activity) => moly_features::npc_fixture_activity::preview::cancel(world, activity),
        Leaving::Talk(entity) => Taken::hand_back(world, entity),
    }
}

/// Stops every run the events started.
pub fn stop(world: &mut World, player: &mut Player) {
    let now = world.resource::<Time>().elapsed_secs();
    let tracks: Vec<usize> = player.events.runs.keys().copied().collect();
    for track in tracks {
        let Some(mut run) = player.events.runs.remove(&track) else {
            continue;
        };
        run.stop_active(world, now);
        for leaving in std::mem::take(&mut run.leavings) {
            hand_back(world, leaving);
        }
    }
}

// ---- the actions ----

/// `sequence.play {path, from?}`: holds the document at `path` when one is
/// named, plays it from `from` seconds and ends when playback ends: the
/// document played out or stopped (done), or playback failed (failed with its
/// reason). Stopped by its caller, it stops the playback.
#[derive(Debug)]
pub(crate) struct Play {
    path: Option<String>,
    from: Option<f32>,
}

fn read_play(fields: &mut Fields<'_>) -> Result<Play, String> {
    let at = fields.at().to_owned();
    let path = fields.opt_str("path")?.map(str::to_owned);
    let from = match fields.opt("from") {
        None => None,
        Some(from) => Some(
            from.as_f64()
                .filter(|from| from.is_finite() && *from >= 0.0)
                .map(|from| from as f32)
                .ok_or_else(|| format!("{at}: from must be a number of seconds, 0 or more"))?,
        ),
    };
    Ok(Play { path, from })
}

impl ActionSpec for Play {
    fn name(&self) -> &str {
        "sequence.play"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let started = ctx.now;
        let at = ctx.path.clone();
        if let Some(path) = &self.path {
            crate::hold_file(world, std::path::Path::new(path)).map_err(|error| format!("{at}: {error}"))?;
        }
        crate::play(world, self.from).map_err(|error| format!("{at}: {error}"))?;
        Ok(Box::new(Waiting { started, stopped: false }))
    }
}

/// A `sequence.play` step waiting for the playback it began.
#[derive(Debug)]
pub(crate) struct Waiting {
    started: f32,
    stopped: bool,
}

impl ActionRun for Waiting {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let elapsed = ctx.now - self.started;
        let player = world.resource::<Player>();
        if player.at_end() {
            return ActPoll::Done(format!("played to the end after {elapsed:.1} s"));
        }
        match player.state {
            State::Playing | State::Paused => {
                let (t, duration, name) = (player.t, player.duration(), player.name().unwrap_or_default().to_owned());
                ActPoll::Running(format!("{name} at {t:.2} of {duration:.2} s after {elapsed:.1} s"))
            }
            State::Stopped if self.stopped => ActPoll::Done(format!("stopped after {elapsed:.1} s")),
            State::Stopped => match player.outcome.clone() {
                Some(outcome) => ActPoll::Failed(outcome),
                None => ActPoll::Done(format!("played to {} s and stopped", player.t)),
            },
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        self.stopped = true;
        if world.resource::<Player>().state != State::Stopped {
            crate::stop(world, "stopped by its step");
        }
    }
}

/// `sequence.stop`: stops the playback; a player that is not playing is done
/// already.
#[derive(Debug)]
pub(crate) struct Stop;

impl ActionSpec for Stop {
    fn name(&self) -> &str {
        "sequence.stop"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let note = if world.resource::<Player>().state != State::Stopped {
            crate::stop(world, "stopped by its step");
            "the sequence is stopped"
        } else {
            "no sequence was playing"
        };
        Ok(Box::new(Over { note }))
    }
}

/// A step that ended on the frame it started.
#[derive(Debug)]
pub(crate) struct Over {
    note: &'static str,
}

impl ActionRun for Over {
    fn poll(&mut self, _world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        ActPoll::Done(self.note.to_owned())
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// The kinds `sequence.play` and `sequence.stop` are registered as.
pub fn kinds() -> Kinds {
    Kinds::new()
        .step(Step::new("sequence.play", false, None, |fields| Ok(Box::new(read_play(fields)?) as Box<dyn ActionSpec>)))
        .step(Step::new("sequence.stop", false, None, |_fields| Ok(Box::new(Stop) as Box<dyn ActionSpec>)))
}

/// Registers the two actions at startup, once the hub's registry exists.
pub fn register(app: &mut App) {
    app.add_systems(Startup, |world: &mut World| {
        if world.get_resource::<moly_features::act::Actions>().is_some() {
            crate::register_actions(world);
        }
    });
}
