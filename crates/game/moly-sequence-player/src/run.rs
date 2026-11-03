//! The player's frame: the played time, the scene taken when playback
//! starts, the sample laid on the game, the end of playback, and the hot
//! reload of the file.

use std::collections::HashMap;

use bevy::animation::AnimationClip;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use moly_features::act::Taken;
use moly_features::character::BodyOwner;
use moly_features::npc::CharacterUnitId;
use moly_sequence::doc::{Part, Sound};
use moly_sequence::Host as SampleHost;

use crate::controls;
use crate::{actors, cameras, lines, sounds, Actor, Player, State};

/// How often a loaded file is looked at for a change.
pub(crate) const RELOAD_SECONDS: f32 = 0.5;

/// The unit a cast member's `unit` names: its number.
pub fn unit_of(unit: &str) -> Result<u32, String> {
    unit.parse::<u32>().map_err(|_| format!("{unit:?} is not a unit number"))
}

/// The body of a unit on this site, named `who` in a refusal.
pub fn body_of(world: &mut World, unit: u32, who: &str) -> Result<Entity, String> {
    body(world, unit).ok_or_else(|| format!("{who} (unit {unit}) is not on this site with a body"))
}

/// The body of a unit: the entity that carries its animator and model.
pub fn body(world: &mut World, unit: u32) -> Option<Entity> {
    let mut bodies = world.query::<(Entity, &CharacterUnitId)>();
    bodies.iter(world).find(|(_, id)| id.0 == unit).map(|(entity, _)| entity)
}

/// What the host knows while a document compiles: the lengths of the sounds
/// in the audio routing, the lengths of the shared motion library's clips
/// (each by its exact name), and no camera rigs.
pub struct WorldCatalog {
    /// (cue, package) → seconds.
    sounds: HashMap<(String, String), f32>,
    /// Library clip name → seconds.
    motions: HashMap<Box<str>, f32>,
}

impl WorldCatalog {
    /// The catalogue as the world's loaded tables and library have it now.
    pub fn read(world: &World) -> Self {
        let mut sounds = HashMap::new();
        let mut motions: HashMap<Box<str>, f32> = HashMap::new();
        if let Some(routing) = world.get_resource::<moly_audio::audio::Routing>() {
            for (cue, package) in routing.stream_keys() {
                if let Some(length) = routing.stream_length(&cue, &package) {
                    sounds.insert((cue, package), length);
                }
            }
        }
        if let (Some(library), Some(libraries)) =
            (world.get_resource::<moly_features::character::MotionLibrary>(), world.get_resource::<Assets<Gltf>>())
        {
            if let (Some(gltf), Some(clips)) = (libraries.get(&library.gltf), world.get_resource::<Assets<AnimationClip>>()) {
                for (name, clip) in &gltf.named_animations {
                    if let Some(asset) = clips.get(clip) {
                        motions.insert(name.clone(), asset.duration());
                    }
                }
            }
        }
        Self { sounds, motions }
    }
}

impl moly_sequence::Catalog for WorldCatalog {
    fn sound_length(&self, sound: &Sound) -> Option<f32> {
        let key = sounds::stream_key(sound)?;
        self.sounds.get(&key).copied()
    }

    fn motion_length(&self, _unit: &str, motion: &str) -> Option<f32> {
        self.motions.get(motion).copied()
    }

    fn has_rig(&self, _name: &str) -> bool {
        false
    }
}

/// The library clip names a body carries for its idle and walk families: the
/// loop (`family_L`) and, when the library has it, the start (`family_S`).
pub struct ClipNames {
    pub idle: [Option<String>; 2],
    pub walk: [Option<String>; 2],
}

impl ClipNames {
    /// Finds them for each cast member, in the order of `cast`.
    pub fn read(world: &mut World, cast: &[(String, String)]) -> Result<Vec<ClipNames>, String> {
        let mut families: HashMap<u32, (String, String)> = HashMap::new();
        let mut units = world.query::<(&CharacterUnitId, &moly_features::npc::MotionClips)>();
        for (id, clips) in units.iter(world) {
            families.insert(id.0, (clips.idle.clone(), clips.walk.clone()));
        }
        let library = world.get_resource::<moly_features::character::MotionLibrary>().ok_or("the shared motion library is not installed")?;
        let gltf = world
            .get_resource::<Assets<Gltf>>()
            .and_then(|libraries| libraries.get(&library.gltf))
            .ok_or("the shared motion library is not loaded")?;
        let find = |stem: &str, suffix: &str| {
            let name = format!("{stem}{suffix}");
            gltf.named_animations.contains_key(name.as_str()).then_some(name)
        };
        let mut out = Vec::with_capacity(cast.len());
        for (id, unit) in cast {
            let number = unit_of(unit).map_err(|error| format!("cast member {id}: {error}"))?;
            let (idle, walk) = families.get(&number).ok_or_else(|| format!("cast member {id}: unit {number} has no motion families"))?;
            out.push(ClipNames { idle: [find(idle, "_L"), find(idle, "_S")], walk: [find(walk, "_L"), find(walk, "_S")] });
        }
        Ok(out)
    }
}

/// Advances the played time by `delta` game seconds at the player's speed;
/// the end stops on the last frame and pauses.
fn advance(player: &mut Player, delta: f32) -> (f32, f32) {
    let from = player.t;
    let duration = player.duration();
    let scaled = delta * player.speed;
    let to = (from + scaled).min(duration);
    player.t = to;
    if scaled > 0.0 && to >= duration {
        player.state = State::Paused;
        player.ended = true;
        player.outcome = Some(format!("the sequence played out at {duration:.2} s"));
    }
    (from, to)
}

/// The player's frame, once an update (`Update`): the file's change, the
/// played time and the sample; the end holds its last frame.
/// The actors' poses are planned here (in `actors::PosePlan`) and applied by
/// `actors::apply`, which has the animation stores.
pub fn frame(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs();
    let delta = world.resource::<Time>().delta_secs();
    reload(world, now);
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    let (from, to) = if player.state == State::Playing {
        player.resume_sounds = true;
        advance(&mut player, delta)
    } else if player.state == State::Paused && player.dirty {
        player.resume_sounds = true;
        (player.t - 1e-4, player.t)
    } else {
        world.insert_resource(player);
        return;
    };
    player.dirty = false;
    let laid = lay(world, &mut player, from, to);
    // The events' actions are polled after this frame, by `actors::events`;
    // a still frame, and a stop, end the runs there.
    // A frame that refused ends the playback by name: the action that waited
    // for it ends with the reason, and the status keeps it.
    let failed = laid.err();
    // The end stays on its last frame, its camera, line, captions and screen
    // colour held until a stop; only the sounds, the events' runs and the
    // camera framing (which has taken the camera by then) end.
    if player.ended {
        player.ended = false;
        sounds::down(world, &mut player);
        controls::stop(world, &mut player);
        if player.framing {
            // The played time did not move: the camera the page framed at
            // is the sequence's again.
            cameras::release(world, &mut player);
            player.framing = false;
        }
    }
    world.insert_resource(player);
    if let Some(reason) = failed {
        let outcome = format!("the sequence failed: {reason}");
        stop(world, &outcome);
        crate::refuse(world, format!("playback failed: {reason}"));
        if let Some(mut player) = world.get_resource_mut::<Player>() {
            player.outcome = Some(outcome);
        }
    }
}

/// Lays the sample between two played times on the game: the events fire
/// first, then the camera, the actors' plans, the sounds, the line, and
/// finally the captions and the screen colour.
fn lay(world: &mut World, player: &mut Player, from: f32, to: f32) -> Result<(), String> {
    let Some(doc) = player.doc.clone() else {
        return Ok(());
    };
    let events: Vec<(f32, serde_json::Value)> = doc.compiled.events(from, to).map(|(at, action)| (at, action.clone())).collect();
    let live = player.live_camera.ok_or("the live camera is not known")?;
    let host = Host { live, heights: heights(world, player)? };
    let frame = doc.compiled.sample(to, &host);
    cameras::lay(world, player, frame.camera)?;
    actors::plan(world, player, &frame.actors, from, to)?;
    sounds::lay(world, player, &frame.sounds, from, to)?;
    lines::lay(world, player, frame.line);
    player.overlay.lay(&frame.captions, frame.screen);
    controls::fire(world, player, &events);
    Ok(())
}

// ---- the scene at the start of playback ----

/// Takes the camera and every actor of the loaded document and leaves it
/// playing from `t`. A refusal leaves the player stopped with nothing taken.
pub fn begin(world: &mut World, t: f32) -> Result<(), String> {
    let (name, cast) = {
        let player = world.resource::<Player>();
        let Some(doc) = player.doc.as_ref() else {
            return Err("no sequence is loaded".to_owned());
        };
        (doc.name().to_owned(), doc.cast.clone())
    };
    let holder = crate::holder(&name);
    let live = cameras::live(world);
    let clips = ClipNames::read(world, &cast)?;
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    // The bodies taken here, when a take refuses part way: the ones the
    // player's ticket had before are not its own to give back.
    let before: Vec<u32> = player.actors.iter().map(|actor| actor.unit).collect();
    let entered = enter(world, &mut player, &holder, &cast);
    if let Err(reason) = entered {
        players_back(world, &mut player);
        give_back_new(world, &before);
        player.state = State::Stopped;
        player.actors.clear();
        player.clips = None;
        player.sampled.clear();
        player.live_camera = None;
        player.dirty = false;
        world.insert_resource(player);
        return Err(reason);
    }
    player.clips = Some(clips);
    player.state = State::Playing;
    // The sequence owns the camera again once it plays.
    player.framing = false;
    player.camera.holder = Some(holder.clone());
    player.sampled = vec![Vec::new(); player.actors.len()];
    player.t = t;
    player.live_camera = live;
    player.ended = false;
    player.dirty = true;
    if !(player.speed.is_finite() && player.speed > 0.0) {
        player.speed = 1.0;
    }
    world.insert_resource(player);
    Ok(())
}

/// Takes the camera, then each cast member's character and body, for the
/// holder.
fn enter(world: &mut World, player: &mut Player, holder: &moly_features::lease::Holder, cast: &[(String, String)]) -> Result<(), String> {
    cameras::claim(world, player, holder)?;
    let mut actors = Vec::with_capacity(cast.len());
    for (id, unit) in cast {
        let number = unit_of(unit).map_err(|error| format!("cast member {id}: {error}"))?;
        let entity = body_of(world, number, "a cast member")?;
        Taken::take(world, crate::SEQUENCE_TICKET, number)?;
        actors.push(Actor { unit: number, entity: Some(entity), ..Actor::default() });
    }
    player.actors = actors;
    for actor in &mut player.actors {
        let Some(entity) = actor.entity else {
            continue;
        };
        let unit = actor.unit;
        let Some(mut driver) = world.get_mut::<moly_features::character::MotionDriver>(entity) else {
            return Err(format!("character {unit} has no animator"));
        };
        driver.enter(&mut actor.token, BodyOwner::Sequence);
        actor.held = true;
    }
    Ok(())
}

/// Lets go every output the player took: the camera, each actor's body (its
/// idle plays when the body is still the player's) and the characters of the
/// sequence's ticket.
pub(crate) fn players_back(world: &mut World, player: &mut Player) {
    cameras::release(world, player);
    actors::settle(world, player.actors.iter_mut().filter(|actor| actor.held));
    Taken::release(world, crate::SEQUENCE_TICKET);
}

/// Gives back the bodies taken since efore was the player's cast: the
/// ones a refused take left behind.
fn give_back_new(world: &mut World, before: &[u32]) {
    let fresh: Vec<u32> = world.resource::<Taken>().units(crate::SEQUENCE_TICKET).into_iter().filter(|unit| !before.contains(unit)).collect();
    for unit in fresh {
        Taken::untake(world, unit);
    }
}

/// Pauses: the sounds go down and the events stop; the frame stays as it was
/// laid (the camera, the actors, the line, the captions and the screen
/// colour) until a play or a stop, and a seek lays the frame at its new time.
pub fn pause(world: &mut World) {
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    if player.state == State::Playing {
        player.state = State::Paused;
        sounds::down(world, &mut player);
        controls::stop(world, &mut player);
        player.dirty = false;
    }
    world.insert_resource(player);
}

/// Moves the played time; nothing fires for the times passed over, and the
/// sounds start again at their new times on the next frame.
pub fn seek(world: &mut World, t: f32) {
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    sounds::down(world, &mut player);
    controls::stop(world, &mut player);
    lines::hide(world, &mut player);
    player.t = t;
    player.ended = false;
    player.dirty = true;
    player.resume_sounds = true;
    world.insert_resource(player);
}

/// Stops the player: the events, the sounds, the line, the overlay, the
/// camera and the actors (settled into their idle), then the takes.
pub fn stop(world: &mut World, why: &str) {
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    let name = player.doc.as_ref().map(|doc| doc.name().to_owned());
    let at = player.t;
    controls::stop(world, &mut player);
    sounds::stop(world, &mut player);
    lines::hide(world, &mut player);
    player.overlay.hide(world);
    players_back(world, &mut player);
    let was_playing = player.state != State::Stopped;
    player.state = State::Stopped;
    player.framing = false;
    player.actors.clear();
    player.sampled.clear();
    player.clips = None;
    player.live_camera = None;
    player.ended = false;
    player.resume_sounds = false;
    player.dirty = false;
    if let Some(mut plan) = world.get_resource_mut::<actors::PosePlan>() {
        plan.clear();
    }
    world.insert_resource(player);
    if was_playing {
        info!("[sequence] {} stopped at {at:.2} s ({why})", name.as_deref().unwrap_or_default());
    }
}

// ---- framing a shot ----

/// Lets the camera go to the free camera while the player is paused, or takes
/// it back for the sequence: the frames that follow lay everything but the
/// camera while it is free (`cameras::lay` has no holder to write under).
pub fn framing(world: &mut World, free: bool) {
    let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
    if player.state == State::Paused && player.framing != free {
        if free {
            cameras::release(world, &mut player);
        } else if let Some(doc) = player.doc.as_ref() {
            let holder = crate::holder(doc.name());
            match cameras::claim(world, &mut player, &holder) {
                Ok(()) => {
                    // The camera the clip gives at the played time is laid
                    // again on the next frame.
                    player.dirty = true;
                }
                Err(reason) => crate::refuse(world, format!("the camera is not the sequence's: {reason}")),
            }
        }
        player.framing = free;
    }
    world.insert_resource(player);
}

/// Writes the camera as it stands into the document at the time `at` (the
/// played time when absent) of the camera clip that holds it, then saves the
/// file, which the reload that follows reads back. The JSON the file holds
/// is edited: a file that no longer parses is refused and left as it was,
/// and a file that was formatted by hand loses that formatting only where it
/// is written.
pub fn capture_key(world: &mut World, at: Option<f32>) -> Result<crate::Reply, crate::Refusal> {
    let (path, clip) = {
        let player = world.resource::<Player>();
        let Some(doc) = player.doc.as_ref() else {
            return Err(crate::refusal("no sequence is loaded"));
        };
        let time = at.unwrap_or(player.t);
        let Some(clip) = doc.compiled.camera_clip(time) else {
            return Err(crate::refusal(format!("no camera clip holds {time:.3} s")));
        };
        doc.compiled.shot_keys(clip.index).map_err(|shape| {
            crate::refusal(format!(
                "the camera clip {} is not written from keys: its eye is a \"{shape}\" {}",
                clip.index,
                match shape {
                    "target" => "look, which keeps its target; only a \"dir\" or an absent look takes the camera's angles",
                    "rig" => "shot, which the host places",
                    _ => "and only a \"keys\" eye or one the clip leaves out takes a key",
                }
            ))
        })?;
        (doc.path.clone(), clip)
    };
    let Some(pose) = cameras::live(world) else {
        return Err(crate::refusal("the live camera is not known"));
    };
    let aim = crate::keys::Aim { eye: pose.eye, target: pose.eye + pose.rotation * Vec3::NEG_Z, fov: pose.fov };
    let text = std::fs::read_to_string(&path).map_err(|error| crate::refusal(format!("cannot read {}: {error}", path.display())))?;
    let mut document: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| crate::refusal(format!("{} does not parse as JSON: {error}", path.display())))?;
    let capture = crate::keys::capture(&mut document, &clip, &aim).map_err(crate::refusal)?;
    let saved = write_document(&path, &document);
    match saved {
        Ok(()) => {
            // The reload that follows reads this save back, at the played
            // time the frame already stands on.
            info!("[sequence] {}: {}", path.display(), capture.note());
            Ok(crate::Reply::Keyed(capture))
        }
        Err(reason) => {
            crate::refuse(world, reason.clone());
            Err(crate::refusal(reason))
        }
    }
}

/// Writes a document to its file: pretty JSON with two spaces, LF line ends
/// and no byte order mark, replaced in one step so a refused write leaves the
/// old file whole.
fn write_document(path: &std::path::Path, document: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(document).map_err(|error| format!("the document does not write: {error}"))?;
    text.push('\n');
    let beside = path.with_extension("json.tmp");
    std::fs::write(&beside, text.as_bytes()).map_err(|error| format!("cannot write {}: {error}", beside.display()))?;
    std::fs::rename(&beside, path).map_err(|error| {
        let _ = std::fs::remove_file(&beside);
        format!("cannot replace {}: {error}", path.display())
    })
}

// ---- the host the sample reads ----

/// What the host knows while a document is sampled: the live camera as
/// playback took it, and each cast member's part heights over its feet.
struct Host {
    live: moly_sequence::CameraPose,
    heights: Vec<PartHeights>,
}

/// A cast member's unit, and the parts of its body in metres over the feet.
type PartHeights = (u32, Vec<(Part, f32)>);

impl SampleHost for Host {
    fn live_camera(&self) -> moly_sequence::CameraPose {
        self.live
    }

    fn part_height(&self, unit: &str, part: Part) -> f32 {
        unit.parse::<u32>()
            .ok()
            .and_then(|number| self.heights.iter().find(|(cast, _)| *cast == number))
            .and_then(|(_, parts)| parts.iter().find(|(known, _)| *known == part))
            .map_or(0.0, |(_, height)| *height)
    }

    fn rig(&self, _name: &str, _params: &serde_json::Value, _local: f32) -> Option<moly_sequence::CameraPose> {
        None
    }
}

/// The heights of each cast member's parts, read once a frame.
fn heights(world: &mut World, player: &Player) -> Result<Vec<PartHeights>, String> {
    let mut heights = Vec::with_capacity(player.actors.len());
    for actor in &player.actors {
        let Some(entity) = actor.entity else {
            heights.push((actor.unit, Vec::new()));
            continue;
        };
        let unit = actor.unit;
        heights.push((unit, actors::heights(world, entity, unit)?));
    }
    Ok(heights)
}

// ---- hot reload ----

/// Looks at the loaded file every [`RELOAD_SECONDS`]: a changed file is read
/// and compiled again and, when it is, replaces the document at the played
/// time (a playing player enters it again from there); a refused file keeps
/// the document and names the refusal. A refused file is read again only
/// after it is saved again.
fn reload(world: &mut World, now: f32) {
    let Some(path) = world.resource::<Player>().doc.as_ref().map(|doc| doc.path.clone()) else {
        return;
    };
    if now - world.resource::<Player>().checked < RELOAD_SECONDS {
        return;
    }
    world.resource_mut::<Player>().checked = now;
    let modified = std::fs::metadata(&path).ok().and_then(|metadata| metadata.modified().ok());
    let player = world.resource::<Player>();
    if player.doc.is_none() || player.modified == modified {
        return;
    }
    match crate::read_document(world, &path) {
        Ok((doc, _, name)) => {
            let (state, t, duration) = {
                let mut player = std::mem::take(&mut *world.resource_mut::<Player>());
                player.modified = doc.modified;
                player.doc = Some(std::sync::Arc::new(doc));
                player.t = player.t.min(player.duration());
                let out = (player.state, player.t, player.duration());
                world.insert_resource(player);
                out
            };
            info!("[sequence] {}: {name} reloaded ({duration:.2} s)", path.display());
            // A refused entry into the reloaded document is given back at
            // once: the cast may have left the site.
            if state != State::Stopped && begin(world, t).is_err() {
                stop(world, "the reloaded document could not play");
            }
        }
        Err(error) => {
            crate::refuse(world, format!("reload refused: {}", error.0));
            if let Some(mut player) = world.get_resource_mut::<Player>() {
                player.modified = modified;
            }
        }
    }
}
