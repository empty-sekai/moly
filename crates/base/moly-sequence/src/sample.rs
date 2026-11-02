//! What a compiled sequence says at a time: the pieces a player applies.

use bevy_math::{Quat, Vec3};
use serde_json::Value;

use crate::doc::{CaptionStyle, MotionMode, Part, Sound};
use crate::Compiled;

/// What the player knows while it samples.
pub trait Host {
    /// The camera as the game would place it without the sequence; shots
    /// blend from and to it, and inherit lens values they do not set.
    fn live_camera(&self) -> CameraPose;
    /// Metres above the feet of a body part of a unit.
    fn part_height(&self, unit: &str, part: Part) -> f32;
    /// The camera a named rig places `local` seconds into its clip; `None`
    /// leaves that shot at the live camera.
    fn rig(&self, name: &str, params: &Value, local: f32) -> Option<CameraPose>;
}

/// A camera placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub eye: Vec3,
    /// Looks along local -Z with local +Y up.
    pub rotation: Quat,
    /// Vertical field of view in degrees.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    /// Focus distance in metres.
    pub focus: Option<f32>,
    /// Aperture as an f-number.
    pub aperture: Option<f32>,
}

/// A compiled target: a point, or a point on an actor.
#[derive(Clone, Debug)]
pub(crate) enum Point {
    Fixed(Vec3),
    Actor { index: usize, part: Option<Part>, offset: Vec3 },
}

#[derive(Clone, Debug)]
pub struct ActorFrame<'a> {
    pub id: &'a str,
    pub unit: &'a str,
    pub position: Vec3,
    /// Degrees about +Y; 0 faces +Z.
    pub yaw: f32,
    /// Present while a move carries the actor along a path.
    pub moving: Option<Moving<'a>>,
    /// The body motions showing, bottom first; their weights sum to at most 1.
    pub motions: Vec<MotionFrame<'a>>,
    pub eye: Option<&'a str>,
    pub mouth: Option<&'a str>,
    pub look: Option<Look>,
}

#[derive(Clone, Copy, Debug)]
pub struct Moving<'a> {
    pub gait: Option<&'a str>,
    /// Metres per second along the path.
    pub speed: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionFrame<'a> {
    pub name: &'a str,
    /// Seconds into the motion; a looping motion is not wrapped here, since
    /// only the player knows its length.
    pub local: f32,
    pub weight: f32,
    pub mode: MotionMode,
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub target: Vec3,
    pub weight: f32,
}

/// Names one sound clip across samples, so a player can keep a playing
/// sound instead of restarting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundKey {
    pub track: usize,
    pub clip: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct SoundFrame<'a> {
    pub key: SoundKey,
    pub source: &'a Sound,
    /// Seconds into the sound.
    pub local: f32,
    pub volume: f32,
    pub speaker: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct LineFrame<'a> {
    /// 0-based position in the line track.
    pub index: usize,
    pub id: Option<&'a str>,
    pub speaker: Option<&'a str>,
    pub name: &'a str,
    pub text: &'a str,
    /// Characters of `text` revealed so far.
    pub shown: usize,
    /// Seconds since the line started.
    pub elapsed: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptionFrame<'a> {
    pub text: &'a str,
    pub style: CaptionStyle,
    pub opacity: f32,
}

/// Everything at one time.
#[derive(Clone, Debug)]
pub struct Frame<'a> {
    pub t: f32,
    pub camera: Option<CameraPose>,
    pub actors: Vec<ActorFrame<'a>>,
    pub sounds: Vec<SoundFrame<'a>>,
    pub line: Option<LineFrame<'a>>,
    pub captions: Vec<CaptionFrame<'a>>,
    /// A colour over the whole picture, `[r, g, b, a]`.
    pub screen: Option<[f32; 4]>,
}

impl Compiled {
    /// The sounds playing at `t`, line voices included.
    pub fn sounds(&self, t: f32) -> Vec<SoundFrame<'_>> {
        let mut out = Vec::new();
        for audio in &self.audio {
            for (clip, span) in audio.lane.spans.iter().enumerate() {
                let Some(presence) = span.presence(t) else { continue };
                if presence.weight <= 0.0 {
                    continue;
                }
                out.push(SoundFrame {
                    key: SoundKey { track: audio.track, clip },
                    source: &span.payload.source,
                    local: presence.local,
                    volume: span.payload.volume * presence.weight,
                    speaker: span.payload.speaker.map(|index| self.actors[index].id.as_str()),
                });
            }
        }
        if let Some(lines) = &self.lines {
            for (clip, line) in lines.lines.iter().enumerate() {
                let Some(voice) = &line.voice else { continue };
                let voice_end = line.voice_length.map_or(line.end, |length| line.start + length);
                if t >= line.start && t < voice_end.min(line.end) {
                    out.push(SoundFrame {
                        key: SoundKey { track: lines.track, clip },
                        source: voice,
                        local: t - line.start,
                        volume: 1.0,
                        speaker: line.speaker.map(|index| self.actors[index].id.as_str()),
                    });
                }
            }
        }
        out
    }

    /// The line up at `t`.
    pub fn line(&self, t: f32) -> Option<LineFrame<'_>> {
        let lines = self.lines.as_ref()?;
        let index = lines.lines.partition_point(|line| line.start <= t).checked_sub(1)?;
        let line = &lines.lines[index];
        if t >= line.end {
            return None;
        }
        let elapsed = t - line.start;
        Some(LineFrame {
            index,
            id: line.id.as_deref(),
            speaker: line.speaker.map(|index| self.actors[index].id.as_str()),
            name: &line.name,
            text: &line.text,
            shown: ((elapsed * line.reveal) as usize).min(line.chars),
            elapsed,
        })
    }

    pub fn captions(&self, t: f32) -> Vec<CaptionFrame<'_>> {
        self.captions
            .iter()
            .flat_map(|lane| lane.present(t))
            .filter(|(_, presence)| presence.weight > 0.0)
            .map(|(span, presence)| CaptionFrame { text: &span.payload.text, style: span.payload.style, opacity: presence.weight })
            .collect()
    }

    pub fn screen(&self, t: f32) -> Option<[f32; 4]> {
        let mut colour: Option<[f32; 4]> = None;
        for (span, presence) in self.screen.present(t) {
            let [r, g, b, a] = span.payload;
            let w = presence.weight;
            colour = Some(match colour {
                None => [r, g, b, a * w],
                Some(below) => std::array::from_fn(|i| below[i] + (span.payload[i] - below[i]) * w),
            });
        }
        colour
    }

    /// The actions whose time is in `(from, to]`, in time order. A player
    /// calls this with the previous and the current time while playing
    /// forward, and with `from` below 0 at the first frame.
    pub fn events(&self, from: f32, to: f32) -> impl Iterator<Item = (f32, &Value)> {
        let first = self.events.partition_point(|(at, _)| *at <= from);
        self.events[first..].iter().take_while(move |(at, _)| *at <= to).map(|(at, action)| (*at, action))
    }

    /// Everything at `t`.
    pub fn sample(&self, t: f32, host: &dyn Host) -> Frame<'_> {
        Frame {
            t,
            camera: self.camera(t, host),
            actors: self.actors(t, host),
            sounds: self.sounds(t),
            line: self.line(t),
            captions: self.captions(t),
            screen: self.screen(t),
        }
    }
}
