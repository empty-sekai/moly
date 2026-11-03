//! The sequence player: a compiled `moly-sequence` document played by the
//! game on the game's clock.
//!
//! A sequence is a function of time, so the player keeps one number, the
//! played time `t`, and lays the sample at `t` on the game every frame: the
//! camera through the camera lease and the held pose
//! ([`moly_features::camera_hold`]), each actor through a character lease and
//! its body's `MotionDriver` pose, the sounds through the cut-scene's voice
//! and BGM calls and the SE queue, the lines through the talk window, and
//! captions and screen colours through an overlay the player owns. The
//! one-shot actions of the event track are the only thing that depends on the
//! direction of playback: they fire when forward playback crosses their time.
//!
//! `t` advances by the game clock's own delta times, so `moly.time`'s scale,
//! pause and steps, and a recording's fixed frame step, drive playback as
//! they drive everything else. [`install`] registers the systems, the
//! `moly.sequence.*` methods of the control surface (native runs), the
//! `sequence.play` and `sequence.stop` actions, and the hot reload of a
//! loaded file.
//!
//! Ownership: the camera is held as the holder `sequence:<name>` of the
//! sequence rank, so a holder of the same or a higher rank (the director, a
//! person) takes it from the player, which then stops by name. The actors are
//! taken from their AI in [`moly_features::act::Taken`] under the ticket
//! [`SEQUENCE_TICKET`], and their bodies are entered as
//! `BodyOwner::Sequence`. Stopping, or a reload or a replay, gives all of it
//! back and plays each actor's idle through the transitions.

mod actors;
mod cameras;
mod controls;
mod lines;
#[cfg(not(target_arch = "wasm32"))]
mod remote;
mod run;
mod sounds;
mod surface;

pub use moly_sequence;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use moly_features::act::{Actions, Kinds};
use moly_features::character::BodyToken;
use moly_features::lease::{Holder, Rank};
use serde_json::Value;

use controls::Running;
use surface::Overlay;

/// The ticket the sequences' character takes are filed under: the sequence
/// player's own band of ticket numbers, above the director's and the
/// plugins'.
pub const SEQUENCE_TICKET: u64 = 1 << 33;

/// The holder id of the sequence named `name`.
pub fn holder_name(name: &str) -> String {
    format!("sequence:{name}")
}

/// The holder a sequence of this name plays as.
pub fn holder(name: &str) -> Holder {
    Holder::new(holder_name(name), Rank::Sequence)
}

/// What the player is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl State {
    /// The state as a status answer names it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }
}

/// A document the player holds, with the file it was read from.
pub(crate) struct Document {
    pub path: PathBuf,
    pub modified: Option<std::time::SystemTime>,
    pub compiled: moly_sequence::Compiled,
    /// The cast's id and unit, in the order of the actor frames.
    pub cast: Vec<(String, String)>,
}

impl Document {
    pub fn name(&self) -> &str {
        self.compiled.name()
    }
}

/// One cast member's hold.
pub(crate) struct Actor {
    pub unit: u32,
    pub entity: Option<Entity>,
    /// The body token while the player holds the body.
    pub token: Option<BodyToken>,
    pub held: bool,
    /// The eye and mouth patterns last written.
    pub eye: Option<String>,
    pub mouth: Option<String>,
    /// The metres the actor's move has carried it: the walk's loops follow
    /// the path's metres.
    pub walked: f32,
}

impl Default for Actor {
    fn default() -> Self {
        Self { unit: 0, entity: None, token: None, held: false, eye: None, mouth: None, walked: 0.0 }
    }
}

/// The sequence player's state: the document, the played time and the
/// outputs it holds.
#[derive(Resource, Default)]
pub struct Player {
    pub state: State,
    pub(crate) doc: Option<std::sync::Arc<Document>>,
    /// The modification time of the file as last read, loaded or refused.
    pub(crate) modified: Option<std::time::SystemTime>,
    /// Seconds into the sequence.
    pub t: f32,
    /// Sequence seconds per game second.
    pub(crate) speed: f32,
    /// The reason the last load or playback failed, or the last reload
    /// refused the file.
    pub(crate) last_error: Option<String>,
    /// Playback came to the end of the document and is paused on its last
    /// frame; the next frame lets that frame's sounds go.
    pub(crate) ended: bool,
    /// The sounds start again on the next frame (a pause or a seek takes them
    /// down until then).
    pub(crate) resume_sounds: bool,
    /// The live camera as playback started; shots blend from it and return
    /// to it.
    pub(crate) live_camera: Option<moly_sequence::CameraPose>,
    /// How the last playback ended, for the action that waited for it.
    pub(crate) outcome: Option<String>,
    /// When the file was last looked at for a change.
    pub(crate) checked: f32,
    /// The played time moved (a seek, a reload, a pause's last frame): the
    /// next frame lays it.
    pub(crate) dirty: bool,
    pub(crate) actors: Vec<Actor>,
    pub(crate) camera: cameras::Held,
    pub(crate) sounds: sounds::Playing,
    pub(crate) events: Running,
    pub(crate) line: Option<lines::Shown>,
    pub(crate) overlay: Overlay,
    /// The library clip names the cast's bodies carry, in the cast's order.
    pub(crate) clips: Option<Vec<run::ClipNames>>,
    /// The library clips each cast member's body was last posed with, as
    /// (clip name, looping, seconds into it, weight): a frame that asks for
    /// the same pose again writes nothing.
    pub(crate) sampled: Vec<Vec<(String, bool, f32, f32)>>,
}

impl Player {
    /// The loaded file's path.
    pub fn path(&self) -> Option<&Path> {
        self.doc.as_ref().map(|doc| doc.path.as_path())
    }

    /// The loaded document's name.
    pub fn name(&self) -> Option<&str> {
        self.doc.as_deref().map(Document::name)
    }

    /// The loaded document's length in seconds.
    pub fn duration(&self) -> f32 {
        self.doc.as_ref().map_or(0.0, |doc| doc.compiled.duration())
    }

    /// Played to the end: paused on the last frame.
    pub fn at_end(&self) -> bool {
        self.doc.is_some() && self.state == State::Paused && self.t >= self.duration()
    }

    /// The reason the last load or playback failed.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// A refused load, play or seek, as the methods and the actions report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn refusal(message: impl Into<String>) -> Refusal {
    Refusal(message.into())
}

/// What a load, a play or another control ended with.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Loaded { name: String, duration: f32 },
    Playing { name: String, from: f32 },
    Paused { t: f32 },
    Seeked { t: f32 },
    Speed { scale: f32 },
    Stopped,
}

impl Reply {
    /// The reply as a log note.
    pub fn note(&self) -> String {
        match self {
            Self::Loaded { name, duration } => format!("{name} loaded ({duration:.2} s)"),
            Self::Playing { name, from } => format!("{name} playing from {from:.2} s"),
            Self::Paused { t } => format!("paused at {t:.2} s"),
            Self::Seeked { t } => format!("seeked to {t:.2} s"),
            Self::Speed { scale } => format!("speed {scale}"),
            Self::Stopped => "stopped".to_owned(),
        }
    }
}

/// Registers the player's resource, its commands, its frame in `Update` (the
/// answers of the methods queued since the last frame, the sample laid on the
/// game, the poses the frame planned, and then the captions and the screen
/// colour), the `sequence.play` and `sequence.stop` actions, and (native runs)
/// the control surface's `moly.sequence.*` methods. A client orders the
/// frame after its writers of the characters and the camera and before its
/// SE queue drains.
pub fn install(app: &mut App) {
    app.init_resource::<Player>()
        .init_resource::<actors::PosePlan>()
        .add_systems(
            Update,
            (
                run::frame,
                // The poses the frame planned go on bodies whose animators
                // the characters' own driver has advanced this update, and
                // the events' actions are polled after them.
                actors::apply.after(run::frame).after(moly_features::character::drive),
                actors::events.after(actors::apply),
                surface::overlay,
            )
                .chain(),
        );
    controls::register(app);
    #[cfg(not(target_arch = "wasm32"))]
    remote::install(app);
}

/// The `sequence.play` and `sequence.stop` actions, for a client that
/// registers the player's kinds itself.
pub fn kinds() -> Kinds {
    controls::kinds()
}

/// Registers the actions on the world's registry.
pub(crate) fn register_actions(world: &mut World) {
    world.resource_mut::<Actions>().register_kinds(kinds());
}

/// Reads, parses and compiles the file at `path` and holds it, stopped, from
/// its start. What the host knows (the sound lengths and the motion library)
/// comes from the world. The player's earlier playback is stopped first.
pub fn load(world: &mut World, path: &Path) -> Result<Reply, Refusal> {
    let read = read_document(world, path);
    let (doc, reply, name) = match read {
        Ok(loaded) => loaded,
        Err(error) => {
            refuse(world, error.0.clone());
            return Err(error);
        }
    };
    // The new document is held before the old cast is let go: a player that
    // is stopped has no takes to hand back, and a running one's takes are
    // made again by the play that follows.
    hold(world, doc, name, reply)
}

/// Holds a document that was read already.
fn hold(world: &mut World, doc: Document, name: String, reply: Reply) -> Result<Reply, Refusal> {
    let path = doc.path.clone();
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    player.t = 0.0;
    player.modified = doc.modified;
    player.doc = Some(std::sync::Arc::new(doc));
    player.last_error = None;
    player.outcome = None;
    player.speed = 1.0;
    player.checked = 0.0;
    player.dirty = true;
    world.insert_resource(player);
    info!("[sequence] {} ({name}): {}", path.display(), reply.note());
    Ok(reply)
}

/// Reads, parses and compiles the file at `path` and leaves it held, stopped,
/// without touching the player: its commands are queued between the frames,
/// where nothing is playing that the new cast could be taken from.
pub(crate) fn hold_file(world: &mut World, path: &Path) -> Result<Reply, Refusal> {
    let (doc, reply, name) = read_document(world, path).inspect_err(|error| {
        refuse(world, error.0.clone());
    })?;
    hold(world, doc, name, reply)
}

/// Reads a file into a document, its reply and its name: it must be read,
/// parse and compile, name no file sound, and its cast must be on this site
/// with a body.
pub(crate) fn read_document(world: &mut World, path: &Path) -> Result<(Document, Reply, String), Refusal> {
    let text = std::fs::read_to_string(path).map_err(|error| refusal(format!("cannot read {}: {error}", path.display())))?;
    let modified = std::fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
    let sequence = moly_sequence::parse(&text).map_err(|error| refusal(format!("{}: {error}", path.display())))?;
    let catalog = run::WorldCatalog::read(world);
    let compiled = moly_sequence::compile(&sequence, &catalog).map_err(|error| refusal(format!("{}: {error}", path.display())))?;
    sounds::refuse_files(&sequence).map_err(|error| refusal(format!("{}: {error}", path.display())))?;
    lines::refuse_missing_glyphs(world, &sequence).map_err(|error| refusal(format!("{}: {error}", path.display())))?;
    let cast = cast_of(world, &compiled)?;
    let duration = compiled.duration();
    let name = compiled.name().to_owned();
    let doc = Document { path: path.to_path_buf(), modified, compiled, cast };
    Ok((doc, Reply::Loaded { name: name.clone(), duration }, name))
}

/// The cast's ids and units, each unit found on the site with a body.
pub(crate) fn cast_of(world: &mut World, compiled: &moly_sequence::Compiled) -> Result<Vec<(String, String)>, Refusal> {
    let mut cast = Vec::new();
    for (id, unit) in compiled.cast() {
        let number = run::unit_of(unit).map_err(|error| refusal(format!("cast member {id}: {error}")))?;
        run::body_of(world, number, "a cast member").map_err(|error| refusal(format!("cast member {id}: {error}")))?;
        cast.push((id.to_owned(), unit.to_owned()));
    }
    Ok(cast)
}

/// Plays the loaded document from `from` seconds, from the played time when
/// it is paused or stopped on a frame, or from the start. The camera and
/// every actor are taken before the first frame; a refusal leaves the player
/// stopped with nothing taken.
pub fn play(world: &mut World, from: Option<f32>) -> Result<Reply, Refusal> {
    let start = {
        let player = world.resource::<Player>();
        if player.doc.is_none() {
            return Err(refusal("no sequence is loaded"));
        }
        let from = from.filter(|from| from.is_finite() && *from >= 0.0);
        let start = from.unwrap_or_else(|| if player.state != State::Stopped && player.t < player.duration() { player.t } else { 0.0 });
        start.clamp(0.0, player.duration())
    };
    run::stop(world, "played again");
    play_from(world, start)
}

/// Enters the document at `t` and leaves it playing.
fn play_from(world: &mut World, t: f32) -> Result<Reply, Refusal> {
    if let Err(reason) = run::begin(world, t) {
        refuse(world, format!("cannot play: {reason}"));
        return Err(refusal(reason));
    }
    {
        let mut player = world.resource_mut::<Player>();
        player.last_error = None;
        player.outcome = None;
    }
    let (name, from) = {
        let player = world.resource::<Player>();
        (player.name().unwrap_or_default().to_owned(), player.t)
    };
    let reply = Reply::Playing { name, from };
    info!("[sequence] {}", reply.note());
    Ok(reply)
}

/// Pauses at the played time; the next play goes on from there. The sounds go
/// down until then.
pub fn pause(world: &mut World) -> Result<Reply, Refusal> {
    if world.resource::<Player>().state != State::Playing {
        return Err(refusal("the sequence is not playing"));
    }
    run::pause(world);
    let t = world.resource::<Player>().t;
    let reply = Reply::Paused { t };
    info!("[sequence] {}", reply.note());
    Ok(reply)
}

/// Moves the played time to `t` seconds, clamped to the document. Nothing is
/// fired for the times passed over: the sounds go down and start again at
/// their new times on the next frame, and a paused player stays paused.
pub fn seek(world: &mut World, t: f32) -> Result<Reply, Refusal> {
    if !t.is_finite() {
        return Err(refusal("the time to seek to is not a number"));
    }
    let state = world.resource::<Player>().state;
    if state == State::Stopped {
        return Err(refusal("no sequence is playing"));
    }
    let duration = world.resource::<Player>().duration();
    let t = t.clamp(0.0, duration);
    run::seek(world, t);
    let reply = Reply::Seeked { t };
    info!("[sequence] {}", reply.note());
    Ok(reply)
}

/// Moves the played time to the named marker. A stopped player with a
/// document plays from the marker.
pub fn seek_marker(world: &mut World, marker: &str) -> Result<Reply, Refusal> {
    let (at, state) = {
        let player = world.resource::<Player>();
        let Some(doc) = player.doc.as_ref() else {
            return Err(refusal("no sequence is loaded"));
        };
        let at = match doc.compiled.markers().iter().find(|(name, _)| name == marker) {
            Some((_, at)) => *at,
            None => {
                let names: Vec<&str> = doc.compiled.markers().iter().map(|(name, _)| name.as_str()).collect();
                return Err(refusal(format!("there is no marker {marker:?}; the markers are {}", names.join(", "))));
            }
        };
        (at, player.state)
    };
    if state == State::Stopped {
        return play_from(world, at);
    }
    seek(world, at)
}

/// Sets the playback rate: sequence seconds per game second.
pub fn set_speed(world: &mut World, scale: f32) -> Result<Reply, Refusal> {
    if !(scale.is_finite() && scale >= 0.0) {
        return Err(refusal("the speed must be a number of at least 0"));
    }
    world.resource_mut::<Player>().speed = scale;
    let reply = Reply::Speed { scale };
    info!("[sequence] {}", reply.note());
    Ok(reply)
}

/// Stops the player and gives every output back: the events, the sounds, the
/// line, the overlay, the camera and each actor (settled into its idle when
/// its body was still the player's), then the character leases and takes.
pub fn stop(world: &mut World, why: &str) -> Reply {
    run::stop(world, why);
    Reply::Stopped
}

/// The player as the status answer holds it.
pub fn status(world: &World) -> Value {
    let player = world.resource::<Player>();
    let Some(doc) = player.doc.as_ref() else {
        return serde_json::json!({
            "name": null,
            "state": player.state.word(),
            "t": player.t,
            "duration": 0.0,
            "markers": [],
            "cast": [],
            "ended": false,
            "last_error": player.last_error,
        });
    };
    let markers: Vec<Value> = doc.compiled.markers().iter().map(|(name, at)| serde_json::json!({ "name": name, "at": at })).collect();
    let cast: Vec<Value> = doc
        .cast
        .iter()
        .zip(player.actors.iter())
        .map(|((id, unit), actor)| serde_json::json!({ "id": id, "unit": unit, "held": actor.held }))
        .collect();
    serde_json::json!({
        "name": doc.compiled.name(),
        "path": doc.path,
        "state": player.state.word(),
        "t": player.t,
        "duration": doc.compiled.duration(),
        "speed": player.speed,
        "markers": markers,
        "cast": cast,
        "ended": player.at_end(),
        "last_error": player.last_error,
    })
}

/// Keeps the reason of a refused load or a failed playback in the status and
/// the log.
pub(crate) fn refuse(world: &mut World, reason: String) {
    warn!("[sequence] {reason}");
    world.resource_mut::<Player>().last_error = Some(reason);
}
