//! Where an operation puts a piece, before the resolver turns it into a grid
//! cell.

use crate::op::{Layer, Piece};

/// A grid column in the layout's horizontal plane (x and z). A [`Anchor::Cell`]
/// anchor names its height: the lowest y at the footprint's minimum corner
/// with no other object, so a column that holds other objects is raised
/// instead of refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub x: i8,
    pub z: i8,
}

impl Cell {
    /// x and z in the site's centered grid frame (see [`Anchor::Cell`]), not
    /// zero-based.
    pub fn new(x: i8, z: i8) -> Self {
        Self { x, z }
    }
}

/// One side of a piece, for a neighbour anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Front,
    Back,
    Left,
    Right,
}

/// A room wall, for an indoor placement. `Nearest` lets the resolver choose
/// the wall closest to the piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wall {
    North,
    East,
    South,
    West,
    Nearest,
}

/// An anchor as an operation names it. The resolver owns the conversion to a
/// concrete cell and direction; this type never carries a grid cell that the
/// caller computed itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// An absolute cell in the site's centered grid frame, not a zero-based
    /// index: the same frame the placed rows' centers and corners use, whose
    /// floor cells run from `-ceil(width/2)` to `ceil(width/2) - 1` in x and
    /// likewise in z with the depth, so a 9-wide grid spans x -5..=4 and an
    /// 8-wide one -4..=3. `layer` absent means the piece's own layout type.
    Cell {
        x: i8,
        z: i8,
        layer: Option<Layer>,
    },
    /// Beside another piece, on the given side, at the given gap.
    Near {
        of: Piece,
        side: Side,
        gap: u8,
    },
    /// On top of another piece, under the source's stacking rule.
    OnTopOf { of: Piece },
    /// Against a room wall.
    Against { wall: Wall },
    /// One position of a ring around another piece.
    Around { of: Piece, index: u8 },
}
