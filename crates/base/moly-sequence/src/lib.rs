//! Authored sequences: the document format of a staged scene on one time
//! axis, and the evaluation of its state at any time on that axis.
//!
//! A [`Sequence`] is read from JSON and compiled once with [`compile`]
//! against a [`Catalog`] of what the host knows (sound and motion lengths,
//! camera rigs). Compiling resolves every relative time and checks every
//! reference, so a document either compiles completely or is refused with
//! an [`Error`] naming the place in the document.
//!
//! A [`Compiled`] sequence is a function of time: [`Compiled::sample`]
//! returns the camera, the actors, the sounds, the line, the captions and
//! the screen colour at any `t`, the same however playback reached `t`.
//! Only one-shot actions depend on direction: [`Compiled::events`] reports
//! those whose time lies between two sampled times.
//!
//! A camera clip may be previewed at one time of its own ([`Compiled::shot`]),
//! and one camera clip may be read back in the key form authors write
//! (`keys` for the eye, `dir` angles for the look; [`Compiled::shot_keys`],
//! [`ShotKeys`], [`EyeKey`], [`AimKey`]) or placed at a content time
//! ([`Compiled::shot`]).

mod actor;
mod bake;
mod camera;
pub mod doc;
pub mod ease;
mod lane;
mod noise;
mod sample;
mod spline;
pub mod time;

use std::fmt;

pub use bake::{compile, Catalog, NoCatalog};
pub use camera::{AimKey, CameraClip, EyeKey, ShotKeys};
pub use doc::Sequence;
pub use sample::{ActorFrame, CameraPose, CaptionFrame, Frame, Host, LineFrame, Look, MotionFrame, Moving, SoundFrame, SoundKey};

/// A refused document: where, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The place in the document, such as `tracks[2].clips[0]`.
    pub at: String,
    pub message: String,
}

impl Error {
    pub(crate) fn new(at: &str, message: impl Into<String>) -> Error {
        Error { at: at.to_owned(), message: message.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.at.is_empty() { write!(f, "{}", self.message) } else { write!(f, "{}: {}", self.at, self.message) }
    }
}

impl std::error::Error for Error {}

/// Reads a document from JSON.
pub fn parse(json: &str) -> Result<Sequence, Error> {
    serde_json::from_str(json).map_err(|error| Error::new("", format!("the document does not parse: {error}")))
}

/// A sequence ready to sample.
pub struct Compiled {
    name: String,
    fps: Option<f32>,
    duration: f32,
    markers: Vec<(String, f32)>,
    camera: lane::Lane<camera::Shot>,
    actors: Vec<actor::Actor>,
    audio: Vec<bake::AudioLane>,
    lines: Option<bake::Lines>,
    captions: Vec<lane::Lane<bake::Caption>>,
    screen: lane::Lane<[f32; 4]>,
    events: Vec<(f32, serde_json::Value)>,
}

impl Compiled {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The frame rate the document asks for, when it names one.
    pub fn fps(&self) -> Option<f32> {
        self.fps
    }

    /// Seconds from 0 to the latest time anything in the document reaches.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Named times, in document order.
    pub fn markers(&self) -> &[(String, f32)] {
        &self.markers
    }

    /// The cast as `(id, unit)`, in document order; actor frames come in
    /// the same order.
    pub fn cast(&self) -> impl Iterator<Item = (&str, &str)> {
        self.actors.iter().map(|actor| (actor.id.as_str(), actor.unit.as_str()))
    }
}
