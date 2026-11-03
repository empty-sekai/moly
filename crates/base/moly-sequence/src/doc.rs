//! The document: what an author writes.
//!
//! Every object refuses fields it does not know, so a misspelt key is an
//! error rather than a value that silently keeps its default. Positions are
//! `[x, y, z]` in world space with +Y up; yaw is in degrees about +Y with 0
//! facing +Z; angles in a document are degrees, times are seconds.

use serde::Deserialize;
use serde_json::Value;

use crate::ease::Ease;
use crate::time::At;

/// A whole sequence document.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    /// The format version; only `1` exists.
    pub version: u32,
    #[serde(default)]
    pub name: String,
    /// The frame rate the sequence is meant to be rendered at, when it has one.
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub cast: Vec<Cast>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// Document-wide values for lines that do not set their own.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Defaults {
    /// Seconds between the end of a line and the start of the next one.
    pub line_gap: f32,
    /// Seconds a line stays up after its voice or its text reveal finishes.
    pub line_hold: f32,
    /// Characters revealed per second.
    pub reveal: f32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self { line_gap: 0.3, line_hold: 0.5, reveal: 20.0 }
    }
}

/// One actor of the sequence and where it stands before anything moves it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cast {
    pub id: String,
    /// What the player spawns for this actor; the format does not read it.
    pub unit: String,
    pub at: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub name: String,
    pub at: At,
}

/// A track: one lane of clips of one kind.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Track {
    Camera(CameraTrack),
    Motion(MotionTrack),
    Move(MoveTrack),
    Face(FaceTrack),
    Look(LookTrack),
    Audio(AudioTrack),
    Line(LineTrack),
    Caption(CaptionTrack),
    Screen(ScreenTrack),
    Event(EventTrack),
}

/// How a clip extends past one of its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extend {
    /// The clip is absent outside its span.
    None,
    /// The clip keeps its edge state.
    Hold,
    /// The clip repeats its span (after its end only).
    Loop,
}

fn one() -> f32 {
    1.0
}

/// Declares a clip type: the timing and blending fields every clip shares,
/// followed by the fields of its own kind.
macro_rules! clip {
    ($(#[$meta:meta])* $name:ident { $($(#[$field_meta:meta])* $field:ident : $ty:ty,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// The name other times refer to this clip by.
            #[serde(default)]
            pub id: Option<String>,
            /// When the clip starts; absent means when the previous clip of
            /// the track ends, or 0 for the first.
            #[serde(default)]
            pub start: Option<At>,
            #[serde(default)]
            pub duration: Option<f32>,
            /// When the clip ends; an alternative to `duration`.
            #[serde(default)]
            pub end: Option<At>,
            /// Seconds into the clip's content at its start.
            #[serde(default)]
            pub clip_in: f32,
            /// Content seconds per sequence second.
            #[serde(default = "one")]
            pub speed: f32,
            /// Seconds over which the clip's weight rises from 0 at its start.
            #[serde(default)]
            pub blend_in: f32,
            /// Seconds over which the clip's weight falls to 0 at its end.
            #[serde(default)]
            pub blend_out: f32,
            #[serde(default)]
            pub ease_in: Ease,
            #[serde(default)]
            pub ease_out: Ease,
            #[serde(default)]
            pub before: Option<Extend>,
            #[serde(default)]
            pub after: Option<Extend>,
            $($(#[$field_meta])* pub $field: $ty,)*
        }

        impl $name {
            pub(crate) fn envelope(&self) -> Envelope<'_> {
                Envelope {
                    id: self.id.as_deref(),
                    start: self.start.as_ref(),
                    duration: self.duration,
                    end: self.end.as_ref(),
                    clip_in: self.clip_in,
                    speed: self.speed,
                    blend_in: self.blend_in,
                    blend_out: self.blend_out,
                    ease_in: self.ease_in,
                    ease_out: self.ease_out,
                    before: self.before,
                    after: self.after,
                }
            }
        }
    };
}

/// The shared timing and blending fields of a clip, borrowed.
#[derive(Clone, Copy)]
pub(crate) struct Envelope<'a> {
    pub id: Option<&'a str>,
    pub start: Option<&'a At>,
    pub duration: Option<f32>,
    pub end: Option<&'a At>,
    pub clip_in: f32,
    pub speed: f32,
    pub blend_in: f32,
    pub blend_out: f32,
    pub ease_in: Ease,
    pub ease_out: Ease,
    pub before: Option<Extend>,
    pub after: Option<Extend>,
}

// ---- camera ----

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub clips: Vec<CameraClip>,
}

clip! {
    /// A shot. Either `eye` (with `aim`, except along a path) or `rig`
    /// places the camera; lens values that are absent are the live camera's.
    CameraClip {
        #[serde(default)]
        eye: Option<Eye>,
        #[serde(default)]
        aim: Option<Aim>,
        /// Degrees about the view direction.
        #[serde(default)]
        roll: Option<Scalar>,
        /// Vertical field of view in degrees.
        #[serde(default)]
        fov: Option<Scalar>,
        #[serde(default)]
        near: Option<f32>,
        #[serde(default)]
        far: Option<f32>,
        #[serde(default)]
        focus: Option<Focus>,
        /// Lens aperture as an f-number.
        #[serde(default)]
        aperture: Option<f32>,
        #[serde(default)]
        shake: Option<Shake>,
        /// A camera placement the host computes.
        #[serde(default)]
        rig: Option<Rig>,
    }
}

/// Where the camera is.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eye {
    /// A fixed point.
    At([f32; 3]),
    /// Points at clip-local times, joined by straight segments.
    Keys(Vec<PointKey>),
    /// A smooth curve through points, travelled at constant speed.
    Path(PathEye),
    /// A circle around a target.
    Orbit(OrbitEye),
    /// An offset from an actor, in the actor's facing frame.
    Follow(FollowEye),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointKey {
    /// Clip-local seconds.
    pub t: f32,
    pub pos: [f32; 3],
    /// The ease of the segment that arrives at this key.
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEye {
    pub points: Vec<[f32; 3]>,
    #[serde(default)]
    pub closed: bool,
    /// The ease of the travel over the clip.
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitEye {
    pub around: Target,
    pub radius: f32,
    /// Height above the target.
    #[serde(default)]
    pub height: f32,
    /// Yaw of the eye around the target at the clip start, in degrees.
    pub from: f32,
    /// Yaw at the clip end.
    pub to: f32,
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowEye {
    pub actor: String,
    /// Metres right, up and forward of the actor as it faces.
    pub offset: [f32; 3],
    /// The camera trails the actor by this many seconds.
    #[serde(default)]
    pub lag: f32,
}

/// Where the camera looks: exactly one of `at` and `dir`. Without an aim a
/// path eye looks along its travel.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aim {
    #[serde(default)]
    pub at: Option<Target>,
    /// `[yaw, pitch]` in degrees; pitch is positive upwards.
    #[serde(default)]
    pub dir: Option<[f32; 2]>,
    /// The aim trails a moving target by this many seconds.
    #[serde(default)]
    pub lag: f32,
}

/// A point, or a point on an actor.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Point([f32; 3]),
    Actor(ActorPoint),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorPoint {
    pub actor: String,
    /// The height on the actor; absent means the feet.
    #[serde(default)]
    pub part: Option<Part>,
    /// A world-space offset from that point.
    #[serde(default)]
    pub offset: [f32; 3],
}

/// A height on an actor's body; the host knows how tall each unit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Head,
    Chest,
    Waist,
    Feet,
}

/// A number, or numbers at clip-local times.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Value(f32),
    Keys { keys: Vec<ScalarKey> },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarKey {
    pub t: f32,
    pub value: f32,
    #[serde(default)]
    pub ease: Ease,
}

/// The focus distance in metres, or the distance to a target.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Focus {
    Distance(f32),
    At { at: Target },
}

/// A seeded, repeatable tremble.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shake {
    /// Metres of positional tremble.
    #[serde(default)]
    pub amplitude: f32,
    /// Degrees of rotational tremble.
    #[serde(default)]
    pub rotation: f32,
    /// Base frequency in hertz.
    #[serde(default = "one")]
    pub frequency: f32,
    #[serde(default)]
    pub seed: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    pub name: String,
    #[serde(default)]
    pub params: Value,
}

// ---- actors ----

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub actor: String,
    pub clips: Vec<MotionClip>,
}

/// How a motion plays inside its clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionMode {
    /// Once, then the last frame.
    #[default]
    Once,
    Loop,
}

clip! {
    /// A body motion. Without a duration it lasts the motion's own length.
    MotionClip {
        motion: String,
        #[serde(default)]
        mode: MotionMode,
        #[serde(default = "one")]
        weight: f32,
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub actor: String,
    pub clips: Vec<MoveClip>,
}

/// How an actor faces while it moves.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Facing {
    /// Along the direction of travel.
    #[default]
    Path,
    /// As it faced before the clip.
    Keep,
    /// A fixed yaw in degrees.
    Yaw(f32),
    /// Toward a target, followed while the clip runs.
    Toward(Target),
}

fn default_turn() -> f32 {
    0.3
}

clip! {
    /// A walk from where the actor stands through `path`. An empty path
    /// turns the actor in place.
    MoveClip {
        #[serde(default)]
        path: Vec<[f32; 3]>,
        #[serde(default)]
        face: Facing,
        /// The locomotion the player shows while the actor moves.
        #[serde(default)]
        gait: Option<String>,
        /// The ease of the travel over the clip.
        #[serde(default)]
        ease: Ease,
        /// Metres per second; gives the duration when none is written.
        #[serde(default)]
        pace: Option<f32>,
        /// Seconds the actor takes to turn to its new facing.
        #[serde(default = "default_turn")]
        turn: f32,
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub actor: String,
    pub clips: Vec<FaceClip>,
}

clip! {
    /// Facial expression names, in the actor's own vocabulary.
    FaceClip {
        #[serde(default)]
        eye: Option<String>,
        #[serde(default)]
        mouth: Option<String>,
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub actor: String,
    pub clips: Vec<LookClip>,
}

clip! {
    /// The actor's head and eyes turn toward a target.
    LookClip {
        at: Target,
    }
}

// ---- sound and text ----

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub clips: Vec<AudioClip>,
}

/// A sound, named in the host's catalogue.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sound {
    Voice(Voice),
    /// A BGM cue and the cue sheet bundle that holds it, as a voice names
    /// its own.
    Bgm(Voice),
    Se(String),
    File(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub cue: String,
    pub bundle: String,
}

clip! {
    /// A sound. Without a duration it lasts the sound's own length, or
    /// until the sequence ends when that length is not known.
    AudioClip {
        source: Sound,
        #[serde(default = "one")]
        volume: f32,
        /// The actor the sound comes from.
        #[serde(default)]
        speaker: Option<String>,
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub clips: Vec<LineClip>,
}

/// A line of dialogue.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineClip {
    #[serde(default)]
    pub id: Option<String>,
    /// Absent means the line gap after the previous line, or 0 for the first.
    #[serde(default)]
    pub start: Option<At>,
    /// Absent means the voice length plus the line hold, or for a line
    /// without a voice its reveal time plus the line hold.
    #[serde(default)]
    pub duration: Option<f32>,
    /// The actor who speaks.
    #[serde(default)]
    pub speaker: Option<String>,
    /// The name shown with the line.
    #[serde(default)]
    pub name: String,
    pub text: String,
    #[serde(default)]
    pub voice: Option<Voice>,
    /// Characters revealed per second.
    #[serde(default)]
    pub reveal: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptionTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub clips: Vec<CaptionClip>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionStyle {
    #[default]
    Subtitle,
    Location,
    Title,
    Fullscreen,
}

clip! {
    /// Text over the picture.
    CaptionClip {
        text: String,
        #[serde(default)]
        style: CaptionStyle,
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub clips: Vec<ScreenClip>,
}

clip! {
    /// A colour laid over the whole picture, `[r, g, b, a]` in 0..=1.
    ScreenClip {
        fade: [f32; 4],
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventTrack {
    #[serde(default)]
    pub name: Option<String>,
    pub events: Vec<EventSpec>,
}

/// A one-shot action, fired when playback crosses its time going forward.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSpec {
    pub at: At,
    /// An object naming its action in `"do"`; the player runs it.
    pub action: Value,
}
