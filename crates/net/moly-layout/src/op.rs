//! The closed operation set, its rotations and its layers.

use crate::anchor::Anchor;

/// A session-local piece number. It identifies a placed fixture inside one
/// edit session; it is not stored in the layout row or in a document row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Piece(pub u32);

/// One of the source's four directions, named by its quarter turn in degrees.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rot {
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}

impl Rot {
    /// The rotation for a served row's degree value.
    pub fn from_degrees(degrees: u16) -> Option<Self> {
        Some(match degrees {
            0 => Self::Deg0,
            90 => Self::Deg90,
            180 => Self::Deg180,
            270 => Self::Deg270,
            _ => return None,
        })
    }

    /// The degree value this rotation writes back to a served row.
    pub fn degrees(self) -> u16 {
        self as u16
    }

    /// The source direction of this quarter turn.
    pub fn direction(self) -> moly_law::fixture::Direction {
        match self {
            Self::Deg0 => moly_law::fixture::Direction::Front,
            Self::Deg90 => moly_law::fixture::Direction::Left,
            Self::Deg180 => moly_law::fixture::Direction::Back,
            Self::Deg270 => moly_law::fixture::Direction::Right,
        }
    }

    /// The operation rotation of a served row's direction index.
    pub fn from_row(rotation: u8) -> Option<Self> {
        Self::from_degrees(u16::from(rotation) * 90)
    }
}

/// The layer a piece occupies. Floor and rug are the two edited ground grids;
/// wall and top are named so an anchor can request them and the validator can
/// refuse a mismatch with a closed code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Floor,
    Rug,
    Wall,
    Top,
}

/// One operation of an edit transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutOp {
    /// Put a new piece. `texture` absent asks for the master's default.
    Place {
        fixture: i32,
        texture: Option<u32>,
        at: Anchor,
        rotation: Rot,
    },
    /// Move a placed piece; `rotation` absent keeps its current one.
    Move {
        piece: Piece,
        at: Anchor,
        rotation: Option<Rot>,
    },
    /// Turn a placed piece in place.
    Rotate { piece: Piece, rotation: Rot },
    /// Take a placed piece back to storage.
    Remove { piece: Piece },
    /// Change a placed piece's texture.
    Recolor { piece: Piece, texture: u32 },
}
