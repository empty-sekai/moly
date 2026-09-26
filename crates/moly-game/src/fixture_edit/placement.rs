//! The tile a fixture put from the catalog or the inventory goes to.
//!
//! Source (`FloorEditState.PutFixture`, a selector cell): the new fixture's
//! layout type is its put layout type, its direction is the camera yaw's
//! (`MysekaiFixtureUtility.RotationToDirection`), and its tile is
//! `GetPlaceableTile(GetCurrentLockAtGrid(FieldCamera.LockAt))`. When no tile
//! is found (`GridPosition.INVALID_VALUE`) the state calls `CantPutFixture`:
//! the "no space to put" dialog, and the new fixture is removed.
//!
//! `RotationToDirection(yaw)`: `a = yaw < 0 ? yaw + 360 : yaw`, then
//! `(ceil(a) % 360) / 90` in integers (an infinite `a` gives -1).
//!
//! `GetCurrentLockAtGrid(lookAt)`: the look-at relative to the site view's
//! position becomes a grid position (`FieldPosition.ToGridPosition`: the floor
//! of each axis over the tile size 0.25, y from max(y, 0)). The start is the
//! ground tile (y 0) of the layout's grid nearest to it by Euclidean length;
//! the ground tiles are enumerated from the minimum corner, z outer and x
//! inner, and a later tile replaces the best only when strictly nearer.
//!
//! `GetPlaceableTile(start)`: when `start` is a tile of the grid and the
//! fixture can be put there (`SiteLayoutUtility.CanPutFloor`), it is the tile.
//! Otherwise rings of radius 1 up to `min(max(tile count x, tile count z),
//! 30)` around the start are searched (`SearchSpiralOnPlane`), each ring in
//! four legs: z = cz - r for x = cx - r ..= cx + r; x = cx + r for z = cz - r
//! + 1 ..= cz + r; z = cz + r for x = cx + r - 1 down to cx - r; x = cx - r for
//! z = cz + r - 1 down to cz - r + 1. A candidate is taken when it is a tile of
//! the grid (`TileDataDictionary.ContainsKey`: the box [-ceil(w/2),
//! ceil(w/2)) x [0, h) x [-ceil(d/2), ceil(d/2)) that `SetupGridData` fills)
//! and `CanPutFloor` holds with the fixture moved there. A fixture whose put
//! type is neither put_target (1) nor put_either (2), or that has no master
//! row, searches the rings at y 0. The other two search, for each radius, the
//! rings at y from max(y0 - r, 0) up to min(h - 1, y0 + r).
//!
//! This runs in the source's grid frame. The product frame is its X mirror
//! (a product cell x is the source cell -x - 1); a candidate's product center
//! and direction come from the mirror the saved layouts go through.
//!
//! `CanPutFloor` is the editor's put check (`validation::put`, the check the
//! decide button uses): every cell of the footprint inside the floor grid and
//! not in a fixture of the same layout table. Named differences from the
//! source's `CanPutFloor`:
//! - the source checks only the bottom layer of the footprint; the put check
//!   also rejects cells above the grid's height;
//! - the site layout's unavailable zones (the master has them only for the
//!   room levels 8 to 12), joint objects, and the plant and rug branches are
//!   not ported;
//! - a raised candidate (the put_target search) whose cells stand on a fixture
//!   goes to the source's stacking branch, which is not ported: it is
//!   rejected as such. A raised candidate over an empty cell is rejected by
//!   the source too (`CanPutOnBaseFixture` needs an object below).

use bevy::prelude::*;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition};

use super::{validation, PutStatus};
use crate::fixture::EditableFixture;
use crate::site::FloorGridLayout;

/// `MysekaiConstants.TILE_SIZE` (all three axes).
const TILE_SIZE: f32 = 0.25;
/// `GetPlaceableTile`'s cap on the spiral's radius.
const MAX_RADIUS: i32 = 30;

/// `MysekaiFixtureUtility.RotationToDirection(float)`. `None` for the
/// source's -1 (an infinite angle) and for the negative results of angles
/// below -360, which name no direction.
pub(super) fn rotation_to_direction(yaw: f32) -> Option<Direction> {
    let angle = if yaw < 0.0 { yaw + 360.0 } else { yaw };
    let up = angle.ceil();
    if up == f32::INFINITY {
        return None;
    }
    // `fcvtps` saturates and maps NaN to 0, as `as` does.
    let whole = up as i32;
    let rest = i32::from((whole % 360) as i16);
    u8::try_from(rest / 90).ok().and_then(Direction::from_u8)
}

/// One axis of `FieldPosition.ToGridPosition`: floor, then the grid
/// position's byte (a positive infinity converts to `i32::MIN`).
fn grid_axis(value: f32) -> i8 {
    let floor = value.floor();
    let whole = if floor == f32::INFINITY {
        i32::MIN
    } else {
        floor as i32
    };
    whole as i8
}

/// `FieldPosition.ToGridPosition` of a position relative to the site.
pub(super) fn to_grid(position: Vec3) -> GridPosition {
    GridPosition::new(
        grid_axis(position.x / TILE_SIZE),
        grid_axis(position.y.max(0.0) / TILE_SIZE),
        grid_axis(position.z / TILE_SIZE),
    )
}

/// The tiles `GridData.SetupGridData` adds for a layout's grid size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TileBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
    /// `GridData.TileCount` x and z.
    pub count_x: i32,
    pub count_z: i32,
}

impl TileBox {
    /// The floor layout's grid, or the rug grid (the same width and depth,
    /// height 1) for the other ground layouts.
    pub(super) fn of(floor: FloorGridLayout, layout: u8) -> Self {
        // `x / 2`, plus one for a positive odd count.
        let half = |n: i32| n / 2 + i32::from(n % 2 == 1);
        let height = if layout == layout_type::FLOOR {
            floor.height
        } else {
            1
        };
        let (hx, hz) = (half(floor.width), half(floor.depth));
        Self {
            min: [-hx, 0, -hz],
            max: [hx - 1, height - 1, hz - 1],
            count_x: floor.width,
            count_z: floor.depth,
        }
    }

    /// `TileDataDictionary.ContainsKey`.
    fn contains(&self, p: GridPosition) -> bool {
        let v = [i32::from(p.x), i32::from(p.y), i32::from(p.z)];
        (0..3).all(|axis| (self.min[axis]..=self.max[axis]).contains(&v[axis]))
    }

    fn height(&self) -> i32 {
        self.max[1] + 1
    }
}

/// `GetCurrentLockAtGrid` after the look-at's grid position.
pub(super) fn nearest_ground_tile(grid: GridPosition, tiles: TileBox) -> GridPosition {
    let mut best = f32::MAX;
    let mut result = GridPosition::ZERO;
    for z in tiles.min[2]..=tiles.max[2] {
        for x in tiles.min[0]..=tiles.max[0] {
            let tile = GridPosition::new(x as i8, 0, z as i8);
            let d = grid - tile;
            let (dx, dy, dz) = (f32::from(d.x), f32::from(d.y), f32::from(d.z));
            let magnitude = (dx * dx + dy * dy + dz * dz).sqrt();
            if best > magnitude {
                best = magnitude;
                result = tile;
            }
        }
    }
    result
}

/// `SearchSpiralOnPlane`'s visiting order of one ring.
pub(super) fn ring(center: GridPosition, radius: i32, y: i32) -> Vec<GridPosition> {
    let (cx, cz) = (i32::from(center.x), i32::from(center.z));
    let at = |x: i32, z: i32| GridPosition::new(x as i8, y as i8, z as i8);
    let r = radius;
    let mut out = Vec::with_capacity((8 * r.max(0)) as usize);
    for dx in -r..=r {
        out.push(at(cx + dx, cz - r));
    }
    for dz in (1 - r)..=r {
        out.push(at(cx + r, cz + dz));
    }
    for dx in (-r..=(r - 1)).rev() {
        out.push(at(cx + dx, cz + r));
    }
    for dz in ((1 - r)..=(r - 1)).rev() {
        out.push(at(cx - r, cz + dz));
    }
    out
}

/// Why a spiral candidate was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Reject {
    /// Not a tile of the layout's grid.
    NotATile,
    /// The editor's put check refused it.
    Put(PutStatus),
    /// The footprint leaves the signed grid domain in the mirror.
    Mirror(String),
    /// A raised cell has no object below (`CanPutOnBaseFixture`).
    NoBase,
    /// A raised cell stands on a fixture: the stacking branch, not ported.
    Stacking,
}

/// The whole search, for the log.
pub(super) struct Placement {
    /// Camera look-at relative to the site, in the source frame.
    pub look_at: Vec3,
    pub look_at_grid: GridPosition,
    /// `GetCurrentLockAtGrid` (source frame).
    pub start: GridPosition,
    pub source_direction: Direction,
    pub search_raised: bool,
    pub max_radius: i32,
    /// The source tile, the product center and the product direction.
    pub chosen: Option<(GridPosition, GridPosition, Direction)>,
    /// Candidates before the chosen one, in visiting order (source frame).
    pub rejected: Vec<(GridPosition, Reject)>,
}

/// Is a put type one of the two that search raised rings (put_target,
/// put_either)?
pub(super) fn searches_raised(put_type: Option<&str>) -> bool {
    matches!(put_type, Some("put_target" | "put_either"))
}

fn occupies(rows: &[(GridPosition, GridPosition)], x: i32, y: i32, z: i32) -> bool {
    rows.iter().any(|(a, b)| {
        (i32::from(a.x)..=i32::from(b.x)).contains(&x)
            && (i32::from(a.y)..=i32::from(b.y)).contains(&y)
            && (i32::from(a.z)..=i32::from(b.z)).contains(&z)
    })
}

/// `GetPlaceableTile(GetCurrentLockAtGrid(lookAt))` for `item` (its layout,
/// fixture and grid size; its center and direction are replaced).
pub(super) fn place(
    item: &EditableFixture,
    source_direction: Direction,
    look_at: Vec3,
    rows: &[EditableFixture],
    floor: FloorGridLayout,
    put_type: Option<&str>,
) -> Placement {
    let tiles = TileBox::of(floor, item.layout);
    let look_at_grid = to_grid(look_at);
    let start = nearest_ground_tile(look_at_grid, tiles);
    let search_raised = searches_raised(put_type);
    let max_radius = tiles.count_x.max(tiles.count_z).min(MAX_RADIUS);
    let occupied: Vec<_> = rows
        .iter()
        .filter(|row| row.uid != item.uid && row.layout == item.layout)
        .filter_map(|row| row.footprint().ok())
        .collect();
    let mut rejected = Vec::new();
    let mut try_place = |p: GridPosition| -> Option<(GridPosition, Direction)> {
        let verdict = candidate(item, source_direction, p, tiles, rows, &occupied, floor);
        match verdict {
            Ok(found) => Some(found),
            Err(reason) => {
                rejected.push((p, reason));
                None
            }
        }
    };
    let mut chosen = try_place(start).map(|(center, direction)| (start, center, direction));
    if chosen.is_none() {
        'search: for r in 1..=max_radius {
            let ys: Vec<i32> = if search_raised {
                let y0 = i32::from(start.y);
                ((y0 - r).max(0)..=(tiles.height() - 1).min(y0 + r)).collect()
            } else {
                vec![0]
            };
            for y in ys {
                for p in ring(start, r, y) {
                    if let Some((center, direction)) = try_place(p) {
                        chosen = Some((p, center, direction));
                        break 'search;
                    }
                }
            }
        }
    }
    Placement {
        look_at,
        look_at_grid,
        start,
        source_direction,
        search_raised,
        max_radius,
        chosen,
        rejected,
    }
}

/// `TryPlaceAt` for one candidate (source frame).
fn candidate(
    item: &EditableFixture,
    source_direction: Direction,
    p: GridPosition,
    tiles: TileBox,
    rows: &[EditableFixture],
    occupied: &[(GridPosition, GridPosition)],
    floor: FloorGridLayout,
) -> Result<(GridPosition, Direction), Reject> {
    if !tiles.contains(p) {
        return Err(Reject::NotATile);
    }
    let (center, direction, layout) = moly_assets::player_data::mirror_fixture_layout(
        p,
        item.grid_size,
        source_direction,
        item.layout,
    )
    .map_err(Reject::Mirror)?;
    let moved = EditableFixture {
        center,
        direction,
        layout,
        ..item.clone()
    };
    if p.y == 0 {
        return match validation::put(&moved, rows, Some(floor)) {
            PutStatus::Ok => Ok((center, direction)),
            status => Err(Reject::Put(status)),
        };
    }
    let (min, max) = moved.footprint().map_err(Reject::Mirror)?;
    let y = i32::from(min.y);
    let mut outside = 0;
    let mut overlap = 0;
    for x in i32::from(min.x)..=i32::from(max.x) {
        for z in i32::from(min.z)..=i32::from(max.z) {
            // The product cell x is the source cell -x - 1.
            if !tiles.contains(GridPosition::new(-(x + 1) as i8, y as i8, z as i8)) {
                outside += 1;
            } else if occupies(occupied, x, y, z) {
                overlap += 1;
            }
        }
    }
    if outside > 0 {
        return Err(Reject::Put(PutStatus::OutOfBounds { cells: outside }));
    }
    if overlap > 0 {
        return Err(Reject::Put(PutStatus::Overlap { cells: overlap }));
    }
    let bare = (i32::from(min.x)..=i32::from(max.x))
        .flat_map(|x| (i32::from(min.z)..=i32::from(max.z)).map(move |z| (x, z)))
        .any(|(x, z)| !occupies(occupied, x, y - 1, z));
    if bare {
        Err(Reject::NoBase)
    } else {
        Err(Reject::Stacking)
    }
}

pub(super) fn reject_label(reason: &Reject) -> String {
    match reason {
        Reject::NotATile => "not a tile of the grid".into(),
        Reject::Put(PutStatus::OutOfBounds { cells }) => {
            format!("footprint outside the floor grid ({cells} cells)")
        }
        Reject::Put(PutStatus::Overlap { cells }) => {
            format!("overlaps a fixture of the same layout ({cells} cells)")
        }
        Reject::Put(status) => format!("put check {status:?}"),
        Reject::Mirror(error) => format!("mirror: {error}"),
        Reject::NoBase => "raised cell with nothing below".into(),
        Reject::Stacking => "raised cell on a fixture (stacking branch not ported)".into(),
    }
}
