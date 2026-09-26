//! The floor editor's put rules (`SiteLayoutUtility.CanPutFloor` and the
//! checks it calls), evaluated on the edit draft in the source's grid frame
//! (the product frame is its X mirror: a product cell x is the source cell
//! -x - 1; rows cross it through `mirror_fixture_layout`).
//!
//! **Tile data** (`GridData`, one per layout grid: floor, rug, road). The grid
//! allocates every tile of its box (`SetupGridData`, [`TileBox`]); a position
//! outside it reads `TileData.Null`. A placed fixture writes its uid
//! (`GridData.AddTileData`):
//! - a fixture that is not a joint: every cell of its current box
//!   (`AddGridSizeTileData`: x and z from the minimum corner over the current
//!   grid size, y from its center's y over its height), and every add-using
//!   cell at its center's y (`AddUsingGridTileData`);
//! - a joint, i.e. a fence or a road (`GridData.AddJointData`): its minimum
//!   corner only, and the joint mark on the cells of its current size at y 0.
//! An add that finds the tile taken is refused by the source; the first row
//! keeps it here too.
//!
//! **Predicates** (`MysekaiFixtureUtility`, on the master row; no row answers
//! false to all of them): `IsPlant` = fixture type plant or house_plant;
//! `IsRug` = settable layout type rug; `IsPutTarget` = put type put_target or
//! put_either; `IsPutBase` = put type put_base or put_either; `IsJoint` =
//! handle type fence or road.
//!
//! **`CanPutFloor(layout, fixture)`**:
//! - a plant: for the floor and the rug grid, `CanPutFloorGridSize` and
//!   `CanPutFloorAddUsingBoundList`;
//! - a rug (`CanPutFloorCaseRug`): every bottom cell is a tile of the floor
//!   and of the rug grid, and an object there other than the rug is neither a
//!   rug nor a plant;
//! - otherwise `CanPutFloorGridSize`, `CanPutFloorAddUsingBoundList`, and
//!   `CanPutStackedFixture` for every fixture stacked on it (the list is
//!   also the ignore list).
//!
//! `CanPutFloorGridSize`: only the bottom layer, the cells min + (i, 0, k)
//! for i, k over the current grid size. Each is a tile, holds no other object
//! (`IsExistsObj`) and no joint mark unless the fixture is a joint
//! (`IsExistsJointObject`); a cell at y >= 1 must stand on a base
//! (`CanPutOnBaseFixture`); and no unavailable zone of the site layout
//! contains it in x and z (`GridBound.IntersectsXZ`, both ends inclusive).
//! The zones come from the layout row of the site, the layout type and the
//! rank (`GetUnavailableZone`). The master's zone rows name floor and wall
//! layouts only (CN and JP 6.8.1: 10 rows each, identical), so the rug and
//! road grids have none; a floor check without the zone table is refused.
//!
//! `CanPutOnBaseFixture(cell)`: the fixture is a put target and the tile below
//! holds another object (`ValidateBasicPlacementConditions`); that object is
//! the base; `CanStack(cell, base)` holds, and the cell at the fixture's
//! height minus one above the cell is a tile (`ValidateStackingAndSpace`).
//! `CanStack`: the base is a put base with stack data, and its stack-enable
//! list has the cell's x and z. The list (`FixtureModel.CreateStackEnableData`)
//! holds the true cells of the bundle meta's `stackEnables` at
//! (center.x - ((columns - 1) >> 1) + column, center.z - ((rows - 1) >> 1) +
//! row), rotated about the layout's square minimum and adjusted by its grid
//! size for its direction.
//!
//! `CanPutFloorAddUsingBoundList`: every add-using cell, taken at y 0, passes
//! the per-tile check (`CanPutFloor(target)`).
//!
//! `CanPutStackedFixture(child)`: the cell at the child's maximum corner is a
//! tile; each of its bottom cells is a tile, and an object there other than
//! the child is one of the stacked fixtures.
//!
//! **Stacking relation** (`SiteLayoutLoader.SetupStackedFixture`, and
//! `FloorEditState.SetStackDataIfNeeded` on decide): a put target whose
//! center is at y >= 1 is stacked on the object in the tile below its minimum
//! corner. Decide adds it only when that object is a put base, which the put
//! check already required, so the relation follows from the tiles.
//! `GetStackedFixtures` collects the stacked fixtures and theirs, depth first.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::fixture::areas::{
    adjust_rotate_position, calculate_rotated_position, layout_square_min, rotated_center_grid,
    GridAreaData,
};
use moly_law::fixture::position::{layout_footprint, layout_type};
use moly_law::fixture::{Direction, GridPosition, Vector3Int};

use super::assets::MetaGrid;
use crate::fixture::EditableFixture;
use crate::site::FloorGridLayout;

const ZONES: &str = "moly://mysekai-site-housing-layout-unavailable-zones.json";

/// The master-row predicates the rules read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Traits {
    pub plant: bool,
    pub rug: bool,
    pub put_target: bool,
    pub put_base: bool,
    pub joint: bool,
}

impl Traits {
    /// From a fixture row's `fixtureType`, settable `layoutType`, `putType`
    /// and `handleType`.
    pub(crate) fn from_row(
        fixture_type: &str,
        settable_layout: &str,
        put_type: &str,
        handle_type: &str,
    ) -> Self {
        Self {
            plant: matches!(fixture_type, "plant" | "house_plant"),
            rug: settable_layout == "rug",
            put_target: matches!(put_type, "put_target" | "put_either"),
            put_base: matches!(put_type, "put_base" | "put_either"),
            joint: matches!(handle_type, "fence" | "road"),
        }
    }
}

/// The `FixtureBundleMeta` arrays the rules read.
#[derive(Clone, Default)]
pub(crate) struct StackMeta {
    pub stack_enables: Option<MetaGrid>,
    pub add_using: Option<MetaGrid>,
}

/// An unavailable zone as its grid bound (source frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Zone {
    pub id: i64,
    pub min: GridPosition,
    pub max: GridPosition,
}

impl Zone {
    /// `GetUnavailableZone`: each axis spans start ..= start + size - 1, the
    /// smaller end as Min; the grid position keeps the low byte.
    fn from_row(id: i64, start: [i64; 3], size: [i64; 3]) -> Self {
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
    fn intersects_xz(&self, x: i8, z: i8) -> bool {
        (self.min.x..=self.max.x).contains(&x) && (self.min.z..=self.max.z).contains(&z)
    }
}

/// Why a check could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    FixtureTable,
    AreaTable,
    ZoneTable,
    FloorGrid,
    /// A row's footprint does not fit the signed grid (or the mirror).
    Footprint,
}

/// Why a cell fails the per-tile check (`CanPutFloor(target)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellFail {
    /// Not a tile of the layout's grid (`TileData.Null`).
    NotATile,
    /// Another object's tile (`IsExistsObj`).
    Occupied,
    /// A fence or road's joint mark (`IsExistsJointObject`).
    Joint,
}

/// The first check `CanPutFloor` fails, with its cell (source frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A bottom cell (or, with `add_using`, an add-using cell at y 0).
    Cell {
        layout: u8,
        cell: GridPosition,
        why: CellFail,
        add_using: bool,
    },
    /// A raised cell: not a put target, or no object below.
    NoBase { cell: GridPosition },
    /// The object below is not a put base or does not enable this cell.
    NotStackable { cell: GridPosition },
    /// The fixture's top above this cell leaves the grid.
    NoHeadroom { cell: GridPosition },
    /// An unavailable zone of the site layout contains the cell.
    UnavailableZone { cell: GridPosition, zone: i64 },
    /// A rug over a rug or a plant.
    RugOverRugOrPlant { layout: u8, cell: GridPosition },
    /// A fixture stacked on it does not fit at its moved place.
    StackedChild { cell: GridPosition },
    /// The fixture's branch is not the floor editor's (a wall, a fence, a road).
    Unsupported,
    /// An input the check reads is missing.
    Missing(Missing),
}

pub(crate) fn refusal_label(refusal: Refusal) -> String {
    let layout = |layout: u8| match layout {
        layout_type::FLOOR => "floor",
        layout_type::RUG => "rug",
        layout_type::ROAD => "road",
        _ => "other",
    };
    let cell = |c: GridPosition| format!("({}, {}, {})", c.x, c.y, c.z);
    match refusal {
        Refusal::Cell {
            layout: l,
            cell: c,
            why,
            add_using,
        } => format!(
            "{} cell {} of the {} grid: {}",
            if add_using { "add-using" } else { "bottom" },
            cell(c),
            layout(l),
            match why {
                CellFail::NotATile => "not a tile",
                CellFail::Occupied => "another object",
                CellFail::Joint => "a fence or road joint",
            }
        ),
        Refusal::NoBase { cell: c } => format!("raised cell {} has no base below", cell(c)),
        Refusal::NotStackable { cell: c } => {
            format!(
                "the object below raised cell {} does not take stacking there",
                cell(c)
            )
        }
        Refusal::NoHeadroom { cell: c } => {
            format!("the fixture's top above {} leaves the grid", cell(c))
        }
        Refusal::UnavailableZone { cell: c, zone } => {
            format!("cell {} is in unavailable zone {zone}", cell(c))
        }
        Refusal::RugOverRugOrPlant { layout: l, cell: c } => {
            format!(
                "cell {} of the {} grid holds a rug or a plant",
                cell(c),
                layout(l)
            )
        }
        Refusal::StackedChild { cell: c } => {
            format!("a stacked fixture does not fit at {}", cell(c))
        }
        Refusal::Unsupported => "the wall, fence and road edit branch".into(),
        Refusal::Missing(missing) => format!("input missing: {missing:?}"),
    }
}

/// The tiles `GridData.SetupGridData` allocates for a layout's grid: the
/// floor layout's box, or (rug, road) the same width and depth with height 1,
/// as the site layout rows are. `x / 2` plus one for a positive odd count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TileBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
    /// `GridData.TileCount` x and z.
    pub count_x: i32,
    pub count_z: i32,
}

impl TileBox {
    pub(super) fn of(floor: FloorGridLayout, layout: u8) -> Self {
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
    pub(super) fn contains(&self, p: GridPosition) -> bool {
        let v = [i32::from(p.x), i32::from(p.y), i32::from(p.z)];
        (0..3).all(|axis| (self.min[axis]..=self.max[axis]).contains(&v[axis]))
    }

    /// `GridData.TileCount.y`.
    pub(super) fn height(&self) -> i32 {
        self.max[1] + 1
    }
}

/// One fixture in the source frame.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub uid: String,
    pub layout: u8,
    pub center: GridPosition,
    pub direction: Direction,
    /// The current box (`BoundingBox`).
    pub min: GridPosition,
    pub max: GridPosition,
    pub traits: Traits,
    /// Add-using cells (x, center y, z).
    add_using: Vec<GridPosition>,
    /// Enabled stack cells (x, z).
    stack_enables: Vec<(i8, i8)>,
}

impl Piece {
    /// A draft row, crossed into the source frame.
    pub(crate) fn new(item: &EditableFixture, rules: &Rules) -> Result<Self, Missing> {
        let (center, direction, layout) = moly_assets::player_data::mirror_fixture_layout(
            item.center,
            item.grid_size,
            item.direction,
            item.layout,
        )
        .map_err(|_| Missing::Footprint)?;
        Self::at(item, rules, center, direction, layout)
    }

    /// The same fixture at a source-frame center and direction.
    pub(crate) fn at(
        item: &EditableFixture,
        rules: &Rules,
        center: GridPosition,
        direction: Direction,
        layout: u8,
    ) -> Result<Self, Missing> {
        let (min, max) = layout_footprint(center, item.grid_size, direction, layout)
            .map_err(|_| Missing::Footprint)?;
        let traits = rules
            .traits
            .get(&item.fixture_id)
            .copied()
            .unwrap_or_default();
        let meta = rules.meta.get(&item.package);
        let add_using = meta
            .and_then(|meta| meta.add_using.as_ref())
            .map(|grid| {
                let mut area =
                    GridAreaData::from_meta(grid.rows, grid.cols, |r, c| grid.cell(r, c));
                area.rotate(direction, true);
                let rotated = rotated_center_grid(center, item.grid_size, direction);
                area.enable_tiles
                    .iter()
                    .map(|tile| rotated + *tile)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let stack_enables = meta
            .and_then(|meta| meta.stack_enables.as_ref())
            .map(|grid| stack_enable_cells(grid, center, item.grid_size, direction))
            .unwrap_or_default();
        Ok(Self {
            uid: item.uid.clone(),
            layout,
            center,
            direction,
            min,
            max,
            traits,
            add_using,
            stack_enables,
        })
    }

    /// How many cells take stacking (instrument).
    pub(crate) fn stack_cell_count(&self) -> usize {
        self.stack_enables.len()
    }

    /// How many add-using cells it has (instrument).
    pub(crate) fn add_using_count(&self) -> usize {
        self.add_using.len()
    }

    /// `CurrentGridSize` (the box's extent).
    pub(crate) fn current_size(&self) -> Vector3Int {
        Vector3Int::new(
            i32::from(self.max.x) - i32::from(self.min.x) + 1,
            i32::from(self.max.y) - i32::from(self.min.y) + 1,
            i32::from(self.max.z) - i32::from(self.min.z) + 1,
        )
    }

    fn bottom_cells(&self) -> impl Iterator<Item = GridPosition> + '_ {
        let size = self.current_size();
        (0..size.x).flat_map(move |i| {
            (0..size.z).map(move |k| self.min + GridPosition::new(i as i8, 0, k as i8))
        })
    }
}

/// `FixtureModel.CreateStackEnableData`: the true cells, as (x, z).
fn stack_enable_cells(
    grid: &MetaGrid,
    center: GridPosition,
    size: Vector3Int,
    direction: Direction,
) -> Vec<(i8, i8)> {
    let half = |n: usize| {
        let n = n as i32 - 1;
        (if n < 0 { n + 1 } else { n }) >> 1
    };
    let (hx, hz) = (half(grid.cols), half(grid.rows));
    let square = layout_square_min(center, size);
    let mut out = Vec::new();
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            if !grid.cell(row, col) {
                continue;
            }
            let x = i32::from(center.x) - hx + col as i32;
            let z = i32::from(center.z) - hz + row as i32;
            let tile = GridPosition::new(x as i8, center.y, z as i8);
            let p = adjust_rotate_position(
                calculate_rotated_position(tile, square, direction),
                size,
                direction,
            );
            out.push((p.x, p.z));
        }
    }
    out
}

/// `SiteLayoutUtility.CalculateRotatedPosition(tile, center, size,
/// Direction.Left)` (the four-argument overload): d = tile - center and
/// m = max(size.x, size.z) - 1 give center + (d.z, d.y, m - d.x).
pub(crate) fn rotate_right(
    tile: GridPosition,
    center: GridPosition,
    size: Vector3Int,
) -> GridPosition {
    let d = tile - center;
    let m = (size.x as i8).max(size.z as i8).wrapping_sub(1);
    center + GridPosition::new(d.z, d.y, m.wrapping_sub(d.x))
}

/// The tables the rules read, shared by every check of a frame.
#[derive(Clone)]
pub(crate) struct Rules {
    pub traits: Arc<HashMap<i32, Traits>>,
    pub meta: Arc<HashMap<String, StackMeta>>,
    /// The unavailable zones of the current floor layout; `None` while the
    /// zone table is not read (a floor-grid check is then refused).
    pub zones: Option<Arc<Vec<Zone>>>,
}

/// One tile read (`GetTileData`) of an allocated position.
#[derive(Clone, Copy)]
struct Tile<'a> {
    occupant: Option<&'a Piece>,
    joint: bool,
}

/// The committed rows' tile data, one dictionary per grid.
pub(crate) struct Board {
    pieces: Vec<Piece>,
    floor: FloorGridLayout,
    zones: Option<Arc<Vec<Zone>>>,
    occupant: HashMap<(u8, GridPosition), usize>,
    joints: std::collections::HashSet<(u8, GridPosition)>,
}

impl Board {
    /// The tile data of `rows` (their draft order is the add order).
    pub(crate) fn new(
        rows: &[EditableFixture],
        floor: FloorGridLayout,
        rules: &Rules,
    ) -> Result<Self, Missing> {
        let mut pieces = Vec::with_capacity(rows.len());
        for row in rows {
            if row.layout & layout_type::FIELD == 0 {
                continue;
            }
            pieces.push(Piece::new(row, rules)?);
        }
        let mut occupant = HashMap::new();
        let mut joints = std::collections::HashSet::new();
        for (index, piece) in pieces.iter().enumerate() {
            let tiles = TileBox::of(floor, piece.layout);
            let mut add = |p: GridPosition| {
                if tiles.contains(p) {
                    occupant.entry((piece.layout, p)).or_insert(index);
                }
            };
            if piece.traits.joint {
                add(piece.min);
                let size = piece.current_size();
                for i in 0..size.x {
                    for k in 0..size.z {
                        let p = GridPosition::new(
                            (i32::from(piece.min.x) + i) as i8,
                            0,
                            (i32::from(piece.min.z) + k) as i8,
                        );
                        if tiles.contains(p) {
                            joints.insert((piece.layout, p));
                        }
                    }
                }
                continue;
            }
            let size = piece.current_size();
            for i in 0..size.x {
                for k in 0..size.z {
                    for j in 0..size.y {
                        add(GridPosition::new(
                            (i32::from(piece.min.x) + i) as i8,
                            (i32::from(piece.center.y) + j) as i8,
                            (i32::from(piece.min.z) + k) as i8,
                        ));
                    }
                }
            }
            for cell in &piece.add_using {
                add(GridPosition::new(cell.x, piece.center.y, cell.z));
            }
        }
        Ok(Self {
            pieces,
            floor,
            zones: rules.zones.clone(),
            occupant,
            joints,
        })
    }

    pub(crate) fn tiles(&self, layout: u8) -> TileBox {
        TileBox::of(self.floor, layout)
    }

    /// `GetTileData`: `None` for `TileData.Null`.
    fn tile(&self, layout: u8, p: GridPosition) -> Option<Tile<'_>> {
        if !self.tiles(layout).contains(p) {
            return None;
        }
        Some(Tile {
            occupant: self
                .occupant
                .get(&(layout, p))
                .map(|&index| &self.pieces[index]),
            joint: self.joints.contains(&(layout, p)),
        })
    }

    /// `IsExistsObj(tile, fixture)`: the object there, when it is not the
    /// fixture itself.
    fn other<'a>(tile: &Tile<'a>, uid: &str) -> Option<&'a Piece> {
        tile.occupant.filter(|piece| piece.uid != uid)
    }

    /// `CanPutFloor(layout, fixture, target)`.
    fn cell_ok(&self, layout: u8, fixture: &Piece, p: GridPosition) -> Result<(), CellFail> {
        let tile = self.tile(layout, p).ok_or(CellFail::NotATile)?;
        if Self::other(&tile, &fixture.uid).is_some() {
            return Err(CellFail::Occupied);
        }
        if !fixture.traits.joint && tile.joint {
            return Err(CellFail::Joint);
        }
        Ok(())
    }

    /// `CanPutOnBaseFixture(layout, fixture, cell)`.
    fn on_base(&self, layout: u8, fixture: &Piece, cell: GridPosition) -> Result<(), Refusal> {
        let below = cell - GridPosition::new(0, 1, 0);
        let base = self
            .tile(layout, below)
            .and_then(|tile| Self::other(&tile, &fixture.uid));
        let Some(base) = base.filter(|_| fixture.traits.put_target) else {
            return Err(Refusal::NoBase { cell });
        };
        let stackable = base.traits.put_base
            && base
                .stack_enables
                .iter()
                .any(|&(x, z)| x == cell.x && z == cell.z);
        if !stackable {
            return Err(Refusal::NotStackable { cell });
        }
        let top = cell + GridPosition::new(0, (fixture.current_size().y - 1) as i8, 0);
        if self.tile(layout, top).is_none() {
            return Err(Refusal::NoHeadroom { cell });
        }
        Ok(())
    }

    /// `CanPutFloorGridSize(layout, fixture)`.
    fn grid_size(&self, layout: u8, fixture: &Piece) -> Result<(), Refusal> {
        let zones: &[Zone] = if layout == layout_type::FLOOR {
            self.zones
                .as_deref()
                .ok_or(Refusal::Missing(Missing::ZoneTable))?
        } else {
            &[]
        };
        for cell in fixture.bottom_cells() {
            self.cell_ok(layout, fixture, cell)
                .map_err(|why| Refusal::Cell {
                    layout,
                    cell,
                    why,
                    add_using: false,
                })?;
            if cell.y >= 1 {
                self.on_base(layout, fixture, cell)?;
            }
            if let Some(zone) = zones.iter().find(|zone| zone.intersects_xz(cell.x, cell.z)) {
                return Err(Refusal::UnavailableZone {
                    cell,
                    zone: zone.id,
                });
            }
        }
        Ok(())
    }

    /// `CanPutFloorAddUsingBoundList(layout, fixture)`.
    fn add_using(&self, layout: u8, fixture: &Piece) -> Result<(), Refusal> {
        for cell in &fixture.add_using {
            let cell = GridPosition::new(cell.x, 0, cell.z);
            self.cell_ok(layout, fixture, cell)
                .map_err(|why| Refusal::Cell {
                    layout,
                    cell,
                    why,
                    add_using: true,
                })?;
        }
        Ok(())
    }

    /// `CanPutStackedFixture(layout, child, ignore)`.
    fn stacked_child(&self, layout: u8, child: &Piece, ignore: &[&str]) -> Result<(), Refusal> {
        if self.tile(layout, child.max).is_none() {
            return Err(Refusal::StackedChild { cell: child.max });
        }
        for cell in child.bottom_cells() {
            let Some(tile) = self.tile(layout, cell) else {
                return Err(Refusal::StackedChild { cell });
            };
            if let Some(other) = Self::other(&tile, &child.uid) {
                if !ignore.contains(&other.uid.as_str()) {
                    return Err(Refusal::StackedChild { cell });
                }
            }
        }
        Ok(())
    }

    /// `CanPutFloorCaseRug(fixture)`.
    fn case_rug(&self, fixture: &Piece) -> Result<(), Refusal> {
        for cell in fixture.bottom_cells() {
            for layout in [layout_type::FLOOR, layout_type::RUG] {
                let Some(tile) = self.tile(layout, cell) else {
                    return Err(Refusal::Cell {
                        layout,
                        cell,
                        why: CellFail::NotATile,
                        add_using: false,
                    });
                };
                if Self::other(&tile, &fixture.uid)
                    .is_some_and(|other| other.traits.rug || other.traits.plant)
                {
                    return Err(Refusal::RugOverRugOrPlant { layout, cell });
                }
            }
        }
        Ok(())
    }

    /// `SiteLayoutUtility.CanPutFloor(layout, fixture)` for a fixture at its
    /// draft place, with the fixtures stacked on it at theirs.
    pub(crate) fn can_put_floor(&self, fixture: &Piece, stacked: &[Piece]) -> Result<(), Refusal> {
        if fixture.traits.plant {
            for layout in [layout_type::FLOOR, layout_type::RUG] {
                self.grid_size(layout, fixture)?;
                self.add_using(layout, fixture)?;
            }
            return Ok(());
        }
        if fixture.traits.rug {
            return self.case_rug(fixture);
        }
        let layout = fixture.layout;
        self.grid_size(layout, fixture)?;
        self.add_using(layout, fixture)?;
        let ignore: Vec<&str> = stacked.iter().map(|piece| piece.uid.as_str()).collect();
        for child in stacked {
            self.stacked_child(layout, child, &ignore)?;
        }
        Ok(())
    }

    /// `SiteLayoutUtility.HasStackedFixtureCrossingMultipleBaseFixtures`
    /// for a put base at its draft place: a fixture stacked directly on it
    /// (its committed relation) whose draft box leaves the base's box in x or
    /// z (`IsStackedFixtureOutsideOfBase`), or an object in the committed
    /// tiles of the layer right above the base that is not stacked directly
    /// on it.
    pub(crate) fn crossing_multiple_bases(&self, base: &Piece, stacked: &[Piece]) -> bool {
        if !base.traits.put_base {
            return false;
        }
        let direct: Vec<&str> = self
            .pieces
            .iter()
            .filter(|piece| self.under(piece).is_some_and(|under| under.uid == base.uid))
            .map(|piece| piece.uid.as_str())
            .collect();
        let size = base.current_size();
        let (bx, bz) = (i32::from(base.min.x), i32::from(base.min.z));
        for uid in &direct {
            let Some(child) = stacked.iter().find(|piece| piece.uid == *uid) else {
                continue;
            };
            let own = child.current_size();
            let (cx, cz) = (i32::from(child.min.x), i32::from(child.min.z));
            if cx < bx || cx + own.x > bx + size.x || cz < bz || cz + own.z > bz + size.z {
                return true;
            }
        }
        let y = i32::from(base.min.y) + size.y;
        for i in 0..size.x {
            for k in 0..size.z {
                let p = GridPosition::new((bx + i) as i8, y as i8, (bz + k) as i8);
                let other = self
                    .tile(base.layout, p)
                    .and_then(|tile| tile.occupant)
                    .filter(|occupant| !direct.contains(&occupant.uid.as_str()));
                if other.is_some() {
                    return true;
                }
            }
        }
        false
    }

    /// `CanSaveJointObject`: the tile at the fixture's center is allocated,
    /// holds an object, and that object is the fixture.
    pub(crate) fn holds_own_center(&self, piece: &Piece) -> bool {
        self.tile(piece.layout, piece.center)
            .and_then(|tile| tile.occupant)
            .is_some_and(|occupant| occupant.uid == piece.uid)
    }

    /// The committed piece of a uid.
    pub(crate) fn piece(&self, uid: &str) -> Option<&Piece> {
        self.pieces.iter().find(|piece| piece.uid == uid)
    }

    /// `SetupStackedFixture`: the object a piece stands on.
    pub(crate) fn under(&self, piece: &Piece) -> Option<&Piece> {
        if !piece.traits.put_target || piece.center.y < 1 {
            return None;
        }
        let below = piece.min - GridPosition::new(0, 1, 0);
        self.tile(piece.layout, below)
            .and_then(|tile| tile.occupant)
            .filter(|under| under.uid != piece.uid)
    }

    /// `GetStackedFixtures(base)`: the uids stacked on it and on those,
    /// depth first.
    pub(crate) fn stacked_on(&self, base: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_stacked(base, &mut out);
        out
    }

    fn collect_stacked(&self, base: &str, out: &mut Vec<String>) {
        for piece in &self.pieces {
            if out.iter().any(|uid| *uid == piece.uid) || piece.uid == base {
                continue;
            }
            if self.under(piece).is_some_and(|under| under.uid == base) {
                out.push(piece.uid.clone());
                self.collect_stacked(&piece.uid.clone(), out);
            }
        }
    }

    /// `FloorEditState.ClampPosition(target)`: the center that keeps the
    /// box inside the grid's bound in x and z; y is the target's.
    pub(crate) fn clamp(&self, fixture: &Piece, target: GridPosition) -> GridPosition {
        let tiles = self.tiles(fixture.layout);
        let (c, lo, hi) = (fixture.center, fixture.min, fixture.max);
        let axis = |t: i8, low: i32, high: i32| -> i8 {
            let t32 = i32::from(t);
            if t32 < low {
                low as i8
            } else if t32 > high {
                high as i8
            } else {
                t
            }
        };
        let x = axis(
            target.x,
            tiles.min[0] + (i32::from(c.x) - i32::from(lo.x)),
            tiles.max[0] - (i32::from(hi.x) - i32::from(c.x)),
        );
        let z = axis(
            target.z,
            tiles.min[2] + (i32::from(c.z) - i32::from(lo.z)),
            tiles.max[2] - (i32::from(hi.z) - i32::from(c.z)),
        );
        GridPosition::new(x, target.y, z)
    }

    /// `FloorEditState.AdjustSelectTileHeight(min, fixture)`: the lowest y of
    /// the column whose tile holds no other object; 0 when a tile of the
    /// column is not in the grid; the top tile when every one is taken.
    pub(crate) fn column_height(&self, layout: u8, at: GridPosition, uid: &str) -> i8 {
        let count = self.tiles(layout).height();
        if count < 1 {
            return 0;
        }
        let mut y = 0;
        loop {
            let Some(tile) = self.tile(layout, GridPosition::new(at.x, y as i8, at.z)) else {
                return 0;
            };
            if Self::other(&tile, uid).is_none() {
                return y as i8;
            }
            if y + 1 >= count {
                return y as i8;
            }
            y += 1;
        }
    }
}

/// The zone table's state and the per-layout zones.
#[derive(Resource, Default)]
pub(crate) struct ZoneTable {
    handle: Option<Handle<JsonAsset>>,
    read: Option<Result<Arc<HashMap<u32, Vec<Zone>>>, String>>,
}

/// Test hook: a read zone table with no zones.
#[cfg(test)]
pub(super) fn empty_zone_table() -> ZoneTable {
    ZoneTable {
        handle: None,
        read: Some(Ok(Arc::new(HashMap::new()))),
    }
}

pub(super) fn request_zones(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ZoneTable {
        handle: Some(server.load(ZONES)),
        read: None,
    });
}

fn parse_zones(value: &serde_json::Value) -> Result<HashMap<u32, Vec<Zone>>, String> {
    let entries = value["entries"]
        .as_object()
        .ok_or("the unavailable zone table has no entries")?;
    let mut out: HashMap<u32, Vec<Zone>> = HashMap::new();
    let mut rows: Vec<&serde_json::Value> = entries.values().collect();
    rows.sort_by_key(|row| row["id"].as_i64());
    for row in rows {
        let int = |key: &str| {
            row[key]
                .as_i64()
                .ok_or_else(|| format!("unavailable zone row carries no integer {key}"))
        };
        let layout = u32::try_from(int("mysekaiSiteLayoutId")?)
            .map_err(|_| "unavailable zone site layout id out of range")?;
        out.entry(layout).or_default().push(Zone::from_row(
            int("id")?,
            [int("startX")?, int("startY")?, int("startZ")?],
            [int("width")?, int("height")?, int("depth")?],
        ));
    }
    Ok(out)
}

pub(super) fn read_zones(
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut table: ResMut<ZoneTable>,
) {
    if table.read.is_some() {
        return;
    }
    let Some(handle) = table.handle.clone() else {
        return;
    };
    let read = match server.load_state(&handle) {
        bevy::asset::LoadState::Loaded => json
            .get(&handle)
            .ok_or_else(|| format!("{ZONES}: loaded but not in the assets"))
            .and_then(|asset| {
                serde_json::from_str::<serde_json::Value>(&asset.0)
                    .map_err(|error| format!("{ZONES}: not JSON: {error}"))
            })
            .and_then(|value| parse_zones(&value)),
        bevy::asset::LoadState::Failed(error) => Err(format!("{ZONES} failed to load: {error}")),
        _ => return,
    };
    match &read {
        Ok(zones) => info!(
            "[edit-put] unavailable zone table read: {} zones on {} site layouts",
            zones.values().map(Vec::len).sum::<usize>(),
            zones.len()
        ),
        Err(error) => {
            warn!("[edit-put] {error}: floor puts are refused until the table is in the root")
        }
    }
    table.read = Some(read.map(Arc::new));
    table.handle = None;
}

/// The rules for the current floor layout, or what is missing.
pub(crate) fn rules(world: &World, floor: Option<FloorGridLayout>) -> Result<Rules, Missing> {
    let floor = floor.ok_or(Missing::FloorGrid)?;
    let traits = super::put_effect::traits_table(world).ok_or(Missing::FixtureTable)?;
    let meta = world
        .get_resource::<super::assets::FixtureAreas>()
        .filter(|areas| areas.loaded)
        .map(|areas| areas.stack.clone())
        .ok_or(Missing::AreaTable)?;
    let zones = match world
        .get_resource::<ZoneTable>()
        .and_then(|table| table.read.as_ref())
    {
        Some(Ok(zones)) => Some(Arc::new(
            zones.get(&floor.layout_id).cloned().unwrap_or_default(),
        )),
        _ => None,
    };
    Ok(Rules {
        traits,
        meta,
        zones,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_bounds_follow_the_min_max_of_start_and_end() {
        // A master row: startX 4, width 4 -> x 4..=7; startZ -6, depth 4 -> z -6..=-3.
        let zone = Zone::from_row(1, [4, 0, -6], [4, 1, 4]);
        assert_eq!(zone.min, GridPosition::new(4, 0, -6));
        assert_eq!(zone.max, GridPosition::new(7, 0, -3));
        assert!(zone.intersects_xz(4, -6) && zone.intersects_xz(7, -3));
        assert!(!zone.intersects_xz(8, -3) && !zone.intersects_xz(4, -2));
    }

    #[test]
    fn master_predicates_read_the_row_columns() {
        let t = Traits::from_row("house_plant", "floor", "put_either", "fence");
        assert!(t.plant && !t.rug && t.put_target && t.put_base && t.joint);
        let t = Traits::from_row("normal", "rug", "none", "none");
        assert!(!t.plant && t.rug && !t.put_target && !t.put_base && !t.joint);
    }
}
