//! The sounds: the document's audio clips and its lines' voices.
//!
//! A [`SoundKey`] names one clip across samples. A voice or a BGM keeps its
//! playback while its key stays and its start keeps the same time; when
//! played time runs on at speed 1 it simply goes on. Any other sight (the
//! sound appears, a seek or a pause brought it back, the speed is not 1)
//! starts it at its local time: a voice with `play_voice`, a BGM with
//! `play_bgm`. An SE is a one-shot: it enters [`SeRequests`] once, when
//! forward playback crosses the clip's start. A sound that goes away stops:
//! a voice with `stop_voice`, a BGM with `release_bgm`.
//!
//! An SE that crosses its start before the audio routing knows its bank
//! waits up to [`SETTLE_SECONDS`] of played time and is then reported by name;
//! a file source has no stream in the game's audio tables, so the player
//! refuses a document that names one when it is loaded.

use std::collections::HashMap;

use bevy::prelude::*;
use moly_features::audio::cutscene::{self, bundle_package};
use moly_features::audio::SeRequests;
use moly_sequence::doc::{Sound, Track};
use moly_sequence::{SoundFrame, SoundKey};

use crate::{Player, State};

/// The package the shared MySekai sound bank (and the site's sounds) live
/// under: a SE cue of a sequence plays from it.
pub(crate) const SE_PACKAGE: &str = "mysekai__sound__se__se_mysekai";

/// The narrowest step of played time (seconds) a one-shot sound fires on: the
/// frame a seek or a replay lays is narrower than this, so it fires none.
const ONE_SHOT_STEP: f32 = 1e-3;

/// How long (played seconds) a one-shot waits for the audio routing to know
/// its bank before it is reported as not started.
const SETTLE_SECONDS: f32 = 2.0;

/// The stream a sound plays from: its cue, and the package the routing table
/// holds it under. `None` for a file, which has no stream.
pub(crate) fn stream_key(sound: &Sound) -> Option<(String, String)> {
    match sound {
        Sound::Voice(voice) | Sound::Bgm(voice) => Some((voice.cue.clone(), bundle_package(&voice.bundle))),
        Sound::Se(cue) => Some((cue.clone(), SE_PACKAGE.to_owned())),
        Sound::File(_) => None,
    }
}

/// A voice or BGM clip the player started.
struct Started {
    /// The played time at which the clip started.
    start: f32,
    /// The voice playback; a BGM has none of its own.
    playback: Option<Entity>,
    source: Source,
}

/// What a key's clip is: a voice, the BGM or an SE.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Voice,
    Bgm,
    Se,
}

fn source_of(sound: &Sound) -> Source {
    match sound {
        Sound::Voice(_) => Source::Voice,
        Sound::Bgm(_) => Source::Bgm,
        Sound::Se(_) => Source::Se,
        Sound::File(_) => Source::Se,
    }
}

/// A one-shot waiting for the audio routing to know its bank.
struct Pending {
    cue: String,
    /// The played time past which the sound gives up and says so.
    until: f32,
}

/// The sounds the player has started, by clip, and the one-shot sounds of
/// frames that crossed their start before the audio routing knew their bank.
#[derive(Default)]
pub struct Playing {
    sounds: HashMap<SoundKey, Started>,
    pending: HashMap<SoundKey, Pending>,
}

/// Lays the sounds at this frame.
pub fn lay(world: &mut World, player: &mut Player, frames: &[SoundFrame<'_>], from: f32, to: f32) -> Result<(), String> {
    let forward = to > from;
    let resume = std::mem::take(&mut player.resume_sounds);
    let mut wanted: HashMap<SoundKey, SoundFrame<'_>> = frames.iter().map(|frame| (frame.key, *frame)).collect();
    let gone: Vec<SoundKey> = player.sounds.sounds.keys().copied().filter(|key| !wanted.contains_key(key)).collect();
    for key in gone {
        let Some(started) = player.sounds.sounds.remove(&key) else {
            continue;
        };
        match started.source {
            Source::Voice => {
                if let Some(playback) = started.playback {
                    cutscene::stop_voice(world, playback);
                }
            }
            Source::Bgm => cutscene::release_bgm(world, "the clip ends"),
            Source::Se => {}
        }
    }
    // The one-shots whose start a frame passed before the routing knew their
    // bank are started here, or reported once the wait is over.
    settle(world, player, to);
    let caller = player.name().unwrap_or("sequence").to_owned();
    for (key, frame) in wanted.drain() {
        let start = to - frame.local;
        let running = player.sounds.sounds.get(&key);
        let same_clip = running.is_some_and(|started| started.source == source_of(frame.source));
        // A sound already started goes on while its start stays the
        // document's and playback runs forward at speed 1.
        let going_on = same_clip
            && !resume
            && player.state == State::Playing
            && player.speed == 1.0
            && running.is_some_and(|started| (started.start - start).abs() <= 1e-3)
            && to > from;
        if going_on {
            continue;
        }
        match frame.source {
            Sound::Voice(voice) => {
                if let Some(started) = player.sounds.sounds.remove(&key) {
                    if let Some(playback) = started.playback {
                        cutscene::stop_voice(world, playback);
                    }
                }
                let package = bundle_package(&voice.bundle);
                let speaker = frame.speaker.and_then(|id| speaker(player, id));
                let playback = cutscene::play_voice(world, &voice.cue, &package, frame.local, speaker)?;
                player.sounds.sounds.insert(key, Started { start, playback: Some(playback), source: Source::Voice });
            }
            Sound::Bgm(bgm) => {
                // The cut-scene screen's BGM call stands over the channel's
                // own choice from its play until it is released; the channel
                // starts the cue at the next of its frames that is not
                // blocked (`cutscene::stands`).
                if bgm.cue.is_empty() {
                    return Err("BGMBehaviour.PlayBGM with an empty cue".to_owned());
                }
                cutscene::release_bgm(world, "the sequence plays another clip");
                cutscene::play_bgm(world, &bgm.cue, &bgm.bundle, &caller);
                player.sounds.sounds.insert(key, Started { start, playback: None, source: Source::Bgm });
            }
            Sound::Se(cue) => {
                // A one-shot enters the queue when forward playback crosses
                // the clip's start; a frame a seek or a replay laid (a step
                // narrower than [`ONE_SHOT_STEP`]) fires nothing.
                let crossed = forward && from < start && start <= to;
                if crossed && to - from > ONE_SHOT_STEP && !same_clip {
                    let known = world
                        .get_resource::<moly_audio::audio::Routing>()
                        .is_some_and(|routing| routing.stream_length(cue, SE_PACKAGE).is_some());
                    if known {
                        world.resource_mut::<SeRequests>().ingame(cue, "sequence");
                    } else {
                        info!("[sequence] the sound {cue} waits for the audio routing to know {SE_PACKAGE}");
                        player.sounds.pending.insert(key, Pending { cue: cue.clone(), until: to + SETTLE_SECONDS });
                    }
                }
                player.sounds.sounds.entry(key).or_insert(Started { start, playback: None, source: Source::Se });
            }
            Sound::File(_) => {}
        }
    }
    Ok(())
}

/// Starts the one-shots that waited for the routing, and names the ones that
/// waited past [`SETTLE_SECONDS`] without it.
fn settle(world: &mut World, player: &mut Player, to: f32) {
    let waiting: Vec<SoundKey> = player.sounds.pending.keys().copied().collect();
    for key in waiting {
        let known = player
            .sounds
            .pending
            .get(&key)
            .is_some_and(|pending| world.get_resource::<moly_audio::audio::Routing>().is_some_and(|routing| routing.stream_length(&pending.cue, SE_PACKAGE).is_some()));
        match (known, player.sounds.pending.get(&key).is_some_and(|pending| pending.until <= to)) {
            (true, _) => {
                if let Some(pending) = player.sounds.pending.remove(&key) {
                    info!("[sequence] the sound {} starts once the routing knows its bank", pending.cue);
                    world.resource_mut::<SeRequests>().ingame(&pending.cue, "sequence");
                }
            }
            (false, true) => {
                if let Some(pending) = player.sounds.pending.remove(&key) {
                    error!("[sequence] the sound {} did not start: {SE_PACKAGE} is not in the audio routing's stream table", pending.cue);
                }
            }
            (false, false) => {}
        }
    }
}

/// The body of the cast member `id`, for a voice's lip sync.
fn speaker(player: &Player, id: &str) -> Option<Entity> {
    let index = player.doc.as_ref()?.cast.iter().position(|(cast, _)| cast == id)?;
    player.actors.get(index)?.entity
}

/// Takes every sound down at once (a pause, a seek, the end).
pub fn down(world: &mut World, player: &mut Player) {
    let keys: Vec<SoundKey> = player.sounds.sounds.keys().copied().collect();
    for key in keys {
        if let Some(started) = player.sounds.sounds.remove(&key) {
            if let Some(playback) = started.playback {
                cutscene::stop_voice(world, playback);
            }
        }
    }
    cutscene::release_bgm(world, "the sequence is not playing");
}

/// Stops the sounds of a playback that ends, and forgets them.
pub fn stop(world: &mut World, player: &mut Player) {
    down(world, player);
    player.sounds.sounds.clear();
}

/// Refuses a document whose audio track names a file: the game's audio tables
/// hold no stream for a file, so it cannot be played.
pub(crate) fn refuse_files(sequence: &moly_sequence::Sequence) -> Result<(), String> {
    for (index, track) in sequence.tracks.iter().enumerate() {
        let Track::Audio(audio) = track else {
            continue;
        };
        for (clip, source) in audio.clips.iter().enumerate() {
            if let Sound::File(path) = &source.source {
                return Err(format!("tracks[{index}].clips[{clip}]: the file sound {path:?} is not played"));
            }
        }
    }
    Ok(())
}
