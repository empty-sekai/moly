//! The layout an edit session works on: the site's grid and its placed rows,
//! each carrying the session's piece number.

use moly_protocol::api::HousingLayoutFixture;

use crate::op::Piece;

/// The site's placement grid, as the edited site serves it: the layout row's
/// id and the box the floor grid fills. Rug and road grids share `width` and
/// `depth` and are one cell high.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SiteGrid {
    pub level: u32,
    pub layout_id: u32,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
}

impl SiteGrid {
    pub fn new(level: u32, layout_id: u32, width: i32, height: i32, depth: i32) -> Self {
        Self {
            level,
            layout_id,
            width,
            height,
            depth,
        }
    }
}

/// One placed piece: the served row and the session's number for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedRow {
    pub piece: Piece,
    pub row: HousingLayoutFixture,
}

/// The layout an edit session holds. The rows stay in the served form (note
/// that `HousingLayoutFixture.rotation` is the direction index `0..3`, not a
/// degree value: the operation's [`Rot`](crate::Rot) is the quarter turn and
/// the caller maps them); the piece number lives beside the row, never inside
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub site_id: u32,
    pub site_type: String,
    pub grid: SiteGrid,
    pub rows: Vec<PlacedRow>,
}

impl Layout {
    pub fn new(site_id: u32, site_type: String, grid: SiteGrid, rows: Vec<PlacedRow>) -> Self {
        Self {
            site_id,
            site_type,
            grid,
            rows,
        }
    }
}
