//! Where an operation puts a piece, before the resolver turns it into a grid
//! cell.

use crate::op::{Layer, Piece};

/// A grid cell in the layout's horizontal plane. The vertical position comes
/// from the target layer or from the piece's current height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub x: i8,
    pub z: i8,
}

impl Cell {
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
    /// An absolute cell; `layer` absent means the piece's own layout type.
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
