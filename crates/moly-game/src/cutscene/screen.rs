//! The cut-scene screen's tracks (`Sekai.Mysekai.Timeline`): Text,
//! Label, FadeTalkWindow, Voice and BGM. The runner has no payload for
//! their clips, so the cut-scene takes these tracks off its request and
//! drives them here from the director's sampled time. A clip's playable is
//! active while `start <= time < end` (`OnBehaviourPlay` on entry,
//! `ProcessFrame` while active, `OnBehaviourPause` on exit and when the
//! graph stops).
//!
//! - `TextBehaviour`: on play (the first clip with `start < time < end`)
//!   `SetIsShowAllText(false)`; each frame `ShowText(text, count)` with
//!   `count = min(floor(clip time / 0.05), length)` (length in UTF-16 units)
//!   and `SetIsShowAllText(count >= length)`; on pause `ResetText()`.
//! - `LabelBehaviour`: each frame `ShowLabel(text)`; on pause
//!   `ShowLabel("")`.
//! - `FadeTalkWindowBehaviour`: the talk window's alpha fades in over the
//!   track's fade-in duration from the clip start, holds, and fades out over
//!   its fade-out duration before the clip end.
//! - `VoiceBehaviour.ProcessFrame`: while the director plays and no voice of
//!   the behaviour plays, `GetClip(track, (float)time)` (the first clip with
//!   `start <= time <= end`) and `PlayVoice(its cue)`:
//!   `PlayVoiceForMysekai(cue, time - start, 1)` with the track's
//!   character's lip sync; on pause `StopVoice`.
//! - `BGMBehaviour.OnBehaviourPlay`: the first clip of the track with
//!   `start < (float)time < end`; none, nothing plays (a clip whose play
//!   falls exactly on its start plays nothing); else `PlayBGM(its cue)`
//!   (the cue alone). The behaviour has no pause.
//!
//! The voice and the BGM are the audio channels' ([`crate::audio::cutscene`]).
//! Named gaps: the cut-scene screen's text window, label and talk window
//! are the UI layer's (screen 639), not drawn here, and each call is logged
//! with its values at its edge; a voice that played out while its clip
//! still holds is not started again (the source's `PlayVoiceOnFinish`
//! clears the playing flag and the next frame plays the cue again from the
//! clip time, past the waveform's end, so nothing sounds).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::fixture_activity_timeline::{TimelineClipKey, TimelineDefinition, TimelinePayload};

/// `TextBehaviour._showingTextIntervalTime`.
const TEXT_INTERVAL: f64 = 0.05;
/// The track classes driven here.
const TRACK_CLASSES: [&str; 5] = [
    "TextTrack",
    "LabelTrack",
    "FadeTalkWindowTrack",
    "VoiceTrack",
    "BGMTrack",
];

struct Clip {
    key: TimelineClipKey,
    start: f64,
    end: f64,
    /// The clip's text or cue.
    value: String,
    /// A BGM clip's bundle name.
    bundle: String,
}

struct Track {
    class: String,
    name: String,
    settings: serde_json::Value,
    clips: Vec<Clip>,
}

/// A voice behaviour's playing flag and its playback.
#[derive(Default)]
struct VoiceState {
    playing: bool,
    playback: Option<Entity>,
    replay_named: bool,
}

/// Who plays the cut-scene: the caller's name, the voice bank its caller
/// downloaded (a package of the stream table), and the cast's characters by
/// unit (a voice track is named by its character's unit).
pub(super) struct ScreenCast {
    pub(super) caller: &'static str,
    pub(super) voice_bank: Option<String>,
    pub(super) members: Vec<(u32, Entity)>,
}

/// The screen tracks of one cut-scene and what they last wrote.
#[derive(Default)]
pub(super) struct ScreenTracks {
    tracks: Vec<Track>,
    /// Text, Label and FadeTalkWindow clips holding the time strictly.
    within: HashSet<TimelineClipKey>,
    /// Voice and BGM clips whose playable is active.
    active: HashSet<TimelineClipKey>,
    /// Text clips: the last count shown.
    shown: HashMap<TimelineClipKey, usize>,
    voices: HashMap<TimelineClipKey, VoiceState>,
}

impl ScreenTracks {
    pub(super) fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub(super) fn describe(&self) -> String {
        let names: Vec<String> = self
            .tracks
            .iter()
            .map(|track| {
                format!(
                    "{} {:?} ({} clips)",
                    track.class,
                    track.name,
                    track.clips.len()
                )
            })
            .collect();
        format!(
            "screen tracks driven by the cut-scene: [{}]",
            names.join(", ")
        )
    }
}

/// Takes the screen tracks off the definition (the runner would refuse
/// their clips) and keeps them for the cut-scene.
pub(super) fn take(definition: &mut TimelineDefinition) -> ScreenTracks {
    let mut screen = ScreenTracks::default();
    definition.tracks.retain(|track| {
        if !TRACK_CLASSES.contains(&track.class.as_str()) {
            return true;
        }
        let clips = track
            .clips
            .iter()
            .map(|clip| {
                let fields = match &clip.payload {
                    TimelinePayload::Unsupported { fields, .. } => fields.clone(),
                    _ => serde_json::Value::Null,
                };
                let text = |key: &str| fields[key].as_str().unwrap_or_default().to_owned();
                Clip {
                    key: clip.key.clone(),
                    start: clip.start,
                    end: clip.end(),
                    value: if fields.get("_text").is_some() {
                        text("_text")
                    } else {
                        text("_cueName")
                    },
                    bundle: text("_assetBundleName"),
                }
            })
            .collect();
        screen.tracks.push(Track {
            class: track.class.clone(),
            name: track.name.clone(),
            settings: track.settings.clone(),
            clips,
        });
        false
    });
    screen
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// One director frame at time `t`.
pub(super) fn frame(world: &mut World, screen: &mut ScreenTracks, t: f64, cast: &ScreenCast) {
    // The behaviours read the root playable's time as a float.
    let time = t as f32 as f64;
    let mut within = HashSet::new();
    let mut active = HashSet::new();
    for track in &screen.tracks {
        for clip in &track.clips {
            let bounds = format!("[{:.4}, {:.4})", clip.start, clip.end);
            let playing = clip.start <= t && t < clip.end;
            let entered = playing && !screen.active.contains(&clip.key);
            let exited = !playing && screen.active.contains(&clip.key);
            if playing {
                active.insert(clip.key.clone());
            }
            let now = clip.start < t && t < clip.end;
            let was = screen.within.contains(&clip.key);
            if now {
                within.insert(clip.key.clone());
            }
            match track.class.as_str() {
                "TextTrack" => {
                    if now {
                        let length = utf16_len(&clip.value);
                        let steps = ((t - clip.start) / TEXT_INTERVAL).floor();
                        let count = if steps < length as f64 {
                            steps as usize
                        } else {
                            length
                        };
                        if !was {
                            info!("[cutscene-screen] t={t:.4} TextBehaviour.OnBehaviourPlay {bounds}: SetIsShowAllText(false); ShowText({:?}, count) at {TEXT_INTERVAL} s per unit, {length} units (screen text window not drawn)", clip.value);
                        }
                        let last = screen.shown.insert(clip.key.clone(), count);
                        if count == length && last != Some(length) {
                            info!("[cutscene-screen] t={t:.4} TextBehaviour.ProcessFrame {bounds}: ShowText(count {count} of {length}); SetIsShowAllText(true)");
                        }
                    } else if was {
                        info!("[cutscene-screen] t={t:.4} TextBehaviour.OnBehaviourPause {bounds}: ResetText()");
                    }
                }
                "LabelTrack" => {
                    if now && !was {
                        info!("[cutscene-screen] t={t:.4} LabelBehaviour {bounds}: ShowLabel({:?}) each frame (not drawn)", clip.value);
                    } else if !now && was {
                        info!("[cutscene-screen] t={t:.4} LabelBehaviour.OnBehaviourPause {bounds}: ShowLabel(\"\")");
                    }
                }
                "FadeTalkWindowTrack" => {
                    if now && !was {
                        info!("[cutscene-screen] t={t:.4} FadeTalkWindowBehaviour.OnBehaviourPlay {bounds}: the talk window fades in, holds and fades out with the track's durations {} (not drawn)", track.settings);
                    } else if !now && was {
                        info!("[cutscene-screen] t={t:.4} FadeTalkWindowBehaviour.OnBehaviourPause {bounds}: the track's current clip is cleared");
                    }
                }
                "VoiceTrack" => {
                    if playing {
                        voice_frame(
                            world,
                            screen_voice(&mut screen.voices, &clip.key),
                            track,
                            time,
                            &bounds,
                            cast,
                        );
                    } else if exited {
                        voice_pause(world, &mut screen.voices, &clip.key, t, &bounds);
                    }
                }
                "BGMTrack" => {
                    if entered {
                        match track
                            .clips
                            .iter()
                            .find(|other| other.start < time && time < other.end)
                        {
                            Some(found) => {
                                info!("[cutscene-screen] t={t:.4} BGMBehaviour.OnBehaviourPlay {bounds}: the track's clip [{:.4}, {:.4}) holds the time: PlayBGM({})", found.start, found.end, found.value);
                                crate::audio::cutscene::play_bgm(world, &found.value, &found.bundle, cast.caller);
                            }
                            None => info!("[cutscene-screen] t={t:.4} BGMBehaviour.OnBehaviourPlay {bounds}: no clip of the track has start < {time:.4} < end: nothing plays"),
                        }
                    }
                }
                _ => {}
            }
        }
    }
    screen.within = within;
    screen.active = active;
}

fn screen_voice<'a>(
    voices: &'a mut HashMap<TimelineClipKey, VoiceState>,
    key: &TimelineClipKey,
) -> &'a mut VoiceState {
    voices.entry(key.clone()).or_default()
}

/// `VoiceBehaviour.ProcessFrame` while the clip's playable is active.
fn voice_frame(
    world: &mut World,
    state: &mut VoiceState,
    track: &Track,
    time: f64,
    bounds: &str,
    cast: &ScreenCast,
) {
    if state.playing {
        // PlayVoiceOnFinish: the voice played out (the bus reaped it).
        let sounding = state.playback.is_some_and(|playback| {
            world.resource::<crate::audio::VoiceChannel>().active_sink() == Some(playback)
        });
        if state.playback.is_some() && !sounding {
            state.playback = None;
            if !state.replay_named {
                state.replay_named = true;
                info!("[cutscene-screen] t={time:.4} VoiceBehaviour {bounds}: PlayVoiceOnFinish (lip sync off); the source plays the cue again from the clip time, past the waveform's end: nothing sounds, not repeated here");
            }
        }
        return;
    }
    let Some(clip) = track
        .clips
        .iter()
        .find(|clip| clip.start <= time && time <= clip.end)
    else {
        return;
    };
    // The flag is set once a clip is found, whether or not the cue plays.
    state.playing = true;
    let start = (time - clip.start) as f32;
    let speaker = track.name.parse::<u32>().ok().and_then(|unit| {
        cast.members
            .iter()
            .find(|(member, _)| *member == unit)
            .map(|(_, entity)| *entity)
    });
    if speaker.is_none() {
        warn!("[cutscene-screen] t={time:.4} VoiceBehaviour {bounds}: track {:?} binds no cast character: no lip sync", track.name);
    }
    let Some(bank) = cast.voice_bank.as_deref() else {
        error!("[cutscene-screen] t={time:.4} VoiceBehaviour.PlayVoice({}) {bounds}: the caller {} downloads no voice bank for its cut-scene; nothing plays", clip.value, cast.caller);
        return;
    };
    match crate::audio::cutscene::play_voice(world, &clip.value, bank, start, speaker) {
        Ok(playback) => state.playback = Some(playback),
        Err(reason) => warn!("[cutscene-screen] t={time:.4} VoiceBehaviour.PlayVoice({}) {bounds}: {reason}; nothing plays", clip.value),
    }
}

/// `VoiceBehaviour.OnBehaviourPause`: `StopVoice`.
fn voice_pause(
    world: &mut World,
    voices: &mut HashMap<TimelineClipKey, VoiceState>,
    key: &TimelineClipKey,
    t: f64,
    bounds: &str,
) {
    let Some(state) = voices.remove(key) else {
        return;
    };
    info!("[cutscene-screen] t={t:.4} VoiceBehaviour.OnBehaviourPause {bounds}: StopVoice");
    if let Some(playback) = state.playback {
        crate::audio::cutscene::stop_voice(world, playback);
    }
}

/// The director's graph stops: every active playable pauses.
pub(super) fn stop(world: &mut World, screen: &mut ScreenTracks) {
    for track in &screen.tracks {
        for clip in &track.clips {
            let bounds = format!("[{:.4}, {:.4})", clip.start, clip.end);
            match track.class.as_str() {
                "TextTrack" if screen.within.contains(&clip.key) => {
                    info!("[cutscene-screen] graph stop: TextBehaviour.OnBehaviourPause {bounds}: ResetText()");
                }
                "LabelTrack" if screen.within.contains(&clip.key) => {
                    info!("[cutscene-screen] graph stop: LabelBehaviour.OnBehaviourPause {bounds}: ShowLabel(\"\")");
                }
                "VoiceTrack" => {
                    if let Some(state) = screen.voices.remove(&clip.key) {
                        info!("[cutscene-screen] graph stop: VoiceBehaviour.OnBehaviourPause {bounds}: StopVoice");
                        if let Some(playback) = state.playback {
                            crate::audio::cutscene::stop_voice(world, playback);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    screen.within.clear();
    screen.active.clear();
}
