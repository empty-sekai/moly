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

    /// The rows as an editing session loads them: the served rows in order,
    /// each given the next piece number from 1.
    pub fn from_rows(
        site_id: u32,
        site_type: String,
        grid: SiteGrid,
        rows: Vec<HousingLayoutFixture>,
    ) -> Self {
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| PlacedRow {
                piece: crate::Piece(index as u32 + 1),
                row,
            })
            .collect();
        Self::new(site_id, site_type, grid, rows)
    }

    /// The served rows, for a save; the piece numbers are session-local and
    /// do not leave the session.
    pub fn into_rows(self) -> Vec<HousingLayoutFixture> {
        self.rows.into_iter().map(|row| row.row).collect()
    }

    /// The row a piece names.
    pub fn row(&self, piece: crate::Piece) -> Option<&HousingLayoutFixture> {
        self.rows
            .iter()
            .find(|row| row.piece == piece)
            .map(|row| &row.row)
    }
}
