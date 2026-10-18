//! The data the placement judgements read: the fixture master's predicate
//! columns, the bundle meta's stacking grids, the site's unavailable zones and
//! the player's owned counts.
//!
//! The resolver and the validator receive this by reference; they never reach
//! an engine resource or an asset server. The caller assembles it from the
//! master tables, the site grid and the user data.

use std::collections::{HashMap, HashSet};

use moly_law::fixture::{GridPosition, Vector3Int};

use crate::validate::Traits;

/// A fixture master row the transaction needs to build a new piece: the
/// bundle, its unrotated extent and the settable layout type.
#[derive(Clone, Debug, PartialEq)]
pub struct FixtureMaster {
    pub package: String,
    pub grid_size: Vector3Int,
    pub layout: u8,
    pub traits: Traits,
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

impl Zone {
    /// `GetUnavailableZone`: each axis spans start ..= start + size - 1, the
    /// smaller end as Min; the grid position keeps the low byte.
    pub fn from_row(id: i64, start: [i64; 3], size: [i64; 3]) -> Self {
        let axis = |a: usize| {
            let end = start[a] + size[a] - 1;
            (start[a].min(end) as i8, start[a].max(end) as i8)
        };
        let (x, y, z) = (axis(0), axis(1), axis(2));
        Self {
            id,
            min: GridPosition::new(x.0, y.0, z.0),
            max: GridPosition::new(x.1, y.1, z.1),
        }
    }

    /// `GridBound.IntersectsXZ`.
    pub fn intersects_xz(&self, x: i8, z: i8) -> bool {
        (self.min.x..=self.max.x).contains(&x) && (self.min.z..=self.max.z).contains(&z)
    }
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
    /// The master rows' predicates by `mysekaiFixtureId`.
    pub traits: HashMap<i32, Traits>,
    /// The full master rows a placement needs, by `mysekaiFixtureId`.
    pub masters: HashMap<i32, FixtureMaster>,
    /// Bundle meta by the package name the row and layout row carry.
    pub bundles: HashMap<String, BundleMeta>,
    /// The edited site layout's unavailable zones; `None` while the zone
    /// table is not read (a floor-grid check is then refused).
    pub zones: Option<Vec<Zone>>,
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
