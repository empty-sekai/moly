//! Times on the sequence axis.
//!
//! A time is written either as seconds from the start of the sequence or
//! relative to something else in the document: a line, a marker, or a clip
//! with an `id`. Relative times are resolved once, when the document is
//! compiled; a reference cycle is refused.

use serde::{Deserialize, Deserializer};

/// A time as written in a document.
#[derive(Clone, Debug, PartialEq)]
pub enum At {
    /// Seconds from the start of the sequence.
    Seconds(f32),
    /// An edge of a line, plus an offset in seconds.
    Line { line: LineRef, edge: Edge, offset: f32 },
    /// A marker, plus an offset in seconds.
    Marker { marker: String, offset: f32 },
    /// An edge of a clip with an `id`, plus an offset in seconds.
    Clip { clip: String, edge: Edge, offset: f32 },
}

/// A line named by its `id` or by its 1-based position in the line track.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LineRef {
    Number(u32),
    Id(String),
}

impl std::fmt::Display for LineRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineRef::Number(n) => write!(f, "line {n}"),
            LineRef::Id(id) => write!(f, "line {id:?}"),
        }
    }
}

/// Which end of a span a time refers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    #[default]
    Start,
    End,
}

impl<'de> Deserialize<'de> for At {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Relative {
            #[serde(default)]
            line: Option<LineRef>,
            #[serde(default)]
            marker: Option<String>,
            #[serde(default)]
            clip: Option<String>,
            #[serde(default)]
            edge: Option<Edge>,
            #[serde(default)]
            offset: f32,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Seconds(f32),
            Relative(Relative),
        }
        use serde::de::Error;
        let relative = match Repr::deserialize(deserializer)? {
            Repr::Seconds(seconds) => {
                return if seconds.is_finite() {
                    Ok(At::Seconds(seconds))
                } else {
                    Err(D::Error::custom("a time must be finite"))
                };
            }
            Repr::Relative(relative) => relative,
        };
        if !relative.offset.is_finite() {
            return Err(D::Error::custom("a time offset must be finite"));
        }
        let offset = relative.offset;
        match (relative.line, relative.marker, relative.clip) {
            (Some(line), None, None) => Ok(At::Line { line, edge: relative.edge.unwrap_or_default(), offset }),
            (None, Some(marker), None) => {
                if relative.edge.is_some() {
                    return Err(D::Error::custom("a marker is a single time and has no edge"));
                }
                Ok(At::Marker { marker, offset })
            }
            (None, None, Some(clip)) => Ok(At::Clip { clip, edge: relative.edge.unwrap_or_default(), offset }),
            _ => Err(D::Error::custom(
                "a time is seconds, or an object with exactly one of \"line\", \"marker\" or \"clip\" (plus \"edge\" and \"offset\")",
            )),
        }
    }
}
