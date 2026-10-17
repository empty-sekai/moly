//! The data the placement judgements read: the fixture master's predicate
//! columns, the bundle meta's stacking grids, the site's unavailable zones and
//! the player's owned counts.
//!
//! The resolver and the validator receive this by reference; they never reach
//! an engine resource or an asset server. The caller assembles it from the
//! master tables, the site grid and the user data.

use std::collections::{HashMap, HashSet};

use moly_law::fixture::{GridPosition, Vector3Int};

/// One fixture master row, reduced to the columns the judgements read.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterFixture {
    /// `fixtureType`: `plant` and `house_plant` name the plant branch.
    pub fixture_type: String,
    /// Settable `layoutType`: `floor`, `rug` and the wall and road values.
    pub settable_layout: String,
    /// `putType`: `put_target`, `put_base`, `put_either` or none.
    pub put_type: String,
    /// `handleType`: `fence`, `road`, `block` and the other handles.
    pub handle_type: String,
    /// Unrotated extent (`gridWidth`, `gridHeight`, `gridDepth`).
    pub grid_size: Vector3Int,
    /// The model's stack height, when the bundle meta carries one. It affects
    /// the stacking presentation, not the source's put check.
    pub stack_height: Option<f32>,
}

/// A bundle's `stackEnables` / `AddUsingGrid` meta grids.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BundleMeta {
    pub stack_enables: Option<MetaGrid>,
    pub add_using: Option<MetaGrid>,
    /// The bundle's own stack height, when it carries one.
    pub stack_height: Option<f32>,
}

/// A boolean meta grid read from the bundle's records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaGrid {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<bool>,
}

impl MetaGrid {
    pub fn new(rows: usize, cols: usize, cells: Vec<bool>) -> Self {
        Self { rows, cols, cells }
    }

    /// One cell of the grid; a cell outside it reads false.
    pub fn cell(&self, row: usize, col: usize) -> bool {
        self.cells
            .get(row.saturating_mul(self.cols).saturating_add(col))
            .copied()
            .unwrap_or(false)
    }
}

/// One unavailable zone of the site layout: the master row's id and its
/// inclusive box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub id: i64,
    pub min: GridPosition,
    pub max: GridPosition,
}

/// The player's owned quantity of a fixture and texture, and how many of them
/// other sites already place. The edited site's draft count is in the
/// [`Layout`](crate::Layout), not here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnedCount {
    pub quantity: i32,
    pub placed_elsewhere: i32,
}

/// The inputs of the layout judgements. The caller fills it once per
/// transaction from the master tables, the site's layout row and the user
/// data.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    /// Fixture master rows by `mysekaiFixtureId`.
    pub fixtures: HashMap<i32, MasterFixture>,
    /// Bundle meta by the package name the row and layout row carry.
    pub bundles: HashMap<String, BundleMeta>,
    /// The edited site layout's unavailable zones.
    pub zones: Vec<Zone>,
    /// Owned counts by (`mysekaiFixtureId`, texture id).
    pub owned: HashMap<(i32, u32), OwnedCount>,
    /// Packages the player cannot store (the house and the gates).
    pub cleanup_protected: HashSet<String>,
}

impl Rules {
    pub fn new() -> Self {
        Self::default()
    }
}
