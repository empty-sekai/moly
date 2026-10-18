//! The transaction executor: an all-or-nothing sequence of [`LayoutOp`]s over
//! one [`Layout`], with the same put rules the editor uses.

use moly_law::fixture::position::{footprint_to_center_size, layout_footprint, layout_type};
use moly_law::fixture::{Direction, GridPosition, Vector3Int};
use moly_protocol::api::{HousingLayoutCorner, HousingLayoutFixture};

use crate::anchor::{Anchor, Side};
use crate::op::{LayoutOp, Layer, Piece, Rot};
use crate::result::{Apply, ApplyOutcome, Applied, OpResult, Refusal, RefusalCode, Rejected};
use crate::rules::{FixtureMaster, Rules};
use crate::validate::{
    Board, Missing, Piece as PlacedPiece, Refusal as CellRefusal, RowLike,
};
use crate::Layout;

/// A layout row as the validator reads it, with the session's piece number as
/// its stable identity.
struct ApplyRow {
    uid: String,
    package: String,
    fixture_id: i32,
    center: GridPosition,
    grid_size: Vector3Int,
    layout: u8,
    direction: Direction,
}

impl RowLike for ApplyRow {
    fn uid(&self) -> &str {
        &self.uid
    }
    fn package(&self) -> &str {
        &self.package
    }
    fn fixture_id(&self) -> i32 {
        self.fixture_id
    }
    fn center(&self) -> GridPosition {
        self.center
    }
    fn grid_size(&self) -> Vector3Int {
        self.grid_size
    }
    fn layout(&self) -> u8 {
        self.layout
    }
    fn direction(&self) -> Direction {
        self.direction
    }
}

fn piece_uid(piece: Piece) -> String {
    format!("piece:{}", piece.0)
}

fn corner(position: GridPosition) -> HousingLayoutCorner {
    HousingLayoutCorner {
        x: position.x,
        y: position.y,
        z: position.z,
    }
}

fn grid_of(position: GridPosition) -> GridPosition {
    position
}

fn row_center(row: &HousingLayoutFixture) -> Result<(GridPosition, Vector3Int), Refusal> {
    let min = grid_of(GridPosition::new(
        row.minimum.x,
        row.minimum.y,
        row.minimum.z,
    ));
    let max = grid_of(GridPosition::new(
        row.maximum.x,
        row.maximum.y,
        row.maximum.z,
    ));
    let (mut center, size) =
        footprint_to_center_size(min, max).map_err(|_| Refusal::new(RefusalCode::OutOfBounds))?;
    center.y = row.center_y;
    Ok((center, size))
}

fn direction_of(row: &HousingLayoutFixture) -> Direction {
    Rot::from_row(row.rotation)
        .map(Rot::direction)
        .unwrap_or(Direction::Front)
}

fn row_from(
    master: &FixtureMaster,
    fixture: i32,
    texture: u32,
    center: GridPosition,
    direction: Direction,
) -> Result<HousingLayoutFixture, Refusal> {
    let (min, max) = layout_footprint(center, master.grid_size, direction, master.layout)
        .map_err(|_| Refusal::new(RefusalCode::OutOfBounds))?;
    Ok(HousingLayoutFixture {
        package: master.package.clone(),
        mysekai_fixture_id: fixture,
        minimum: corner(min),
        maximum: corner(max),
        center_y: center.y,
        layout_type: master.layout,
        rotation: direction as u8,
        texture_id: texture,
    })
}

fn apply_row(piece: Piece, row: &HousingLayoutFixture, rules: &Rules) -> Result<ApplyRow, Refusal> {
    let (center, derived) = row_center(row)?;
    let grid_size = rules
        .masters
        .get(&row.mysekai_fixture_id)
        .map_or(derived, |master| master.grid_size);
    Ok(ApplyRow {
        uid: piece_uid(piece),
        package: row.package.clone(),
        fixture_id: row.mysekai_fixture_id,
        center,
        grid_size,
        layout: row.layout_type,
        direction: direction_of(row),
    })
}

fn board(layout: &Layout, rules: &Rules) -> Result<Board, Refusal> {
    let rows = layout
        .rows
        .iter()
        .map(|placed| apply_row(placed.piece, &placed.row, rules))
        .collect::<Result<Vec<_>, _>>()?;
    Board::new(&rows, layout.grid, rules).map_err(missing_refusal)
}

fn missing_refusal(_missing: Missing) -> Refusal {
    // A missing input cannot be named by the operation contract's closed
    // codes; the caller only reaches this with an incomplete master set, which
    // it reports as a layer refusal rather than silently placing.
    Refusal::new(RefusalCode::WrongLayer)
}

fn cell_refusal(refusal: CellRefusal) -> Refusal {
    Refusal::new(refusal.code().unwrap_or(RefusalCode::WrongLayer))
}

fn master(rules: &Rules, fixture: i32) -> Result<&FixtureMaster, Refusal> {
    rules
        .masters
        .get(&fixture)
        .ok_or_else(|| Refusal::new(RefusalCode::NotOwned))
}

fn placed_row<'a>(layout: &'a Layout, piece: Piece) -> Result<&'a HousingLayoutFixture, Refusal> {
    layout
        .rows
        .iter()
        .find(|row| row.piece == piece)
        .map(|row| &row.row)
        .ok_or_else(|| {
            let mut refusal = Refusal::new(RefusalCode::UnknownPiece);
            refusal.piece = Some(piece);
            refusal
        })
}

fn base_center(layout: &Layout, piece: Piece) -> Result<(GridPosition, Vector3Int), Refusal> {
    let row = placed_row(layout, piece)?;
    row_center(row)
}

fn anchor_center(
    layout: &Layout,
    anchor: &Anchor,
    master: &FixtureMaster,
    current: Option<GridPosition>,
) -> Result<GridPosition, Refusal> {
    match anchor {
        Anchor::Cell { x, z, layer } => {
            if let Some(layer) = layer {
                let matches = match layer {
                    Layer::Floor => master.layout & layout_type::FLOOR != 0,
                    Layer::Rug => master.layout & layout_type::RUG != 0,
                    Layer::Wall | Layer::Top => false,
                };
                if !matches {
                    return Err(Refusal::new(RefusalCode::WrongLayer));
                }
            }
            Ok(GridPosition::new(*x, 0, *z))
        }
        Anchor::Near { of, side, gap } => {
            let (center, size) = base_center(layout, *of)?;
            let half = |axis: i32| (axis - 1) / 2;
            let gap = i32::from(*gap);
            let (dx, dz) = match side {
                Side::Front => (0, half(size.z) + half(master.grid_size.z) + gap + 1),
                Side::Back => (0, -(half(size.z) + half(master.grid_size.z) + gap + 1)),
                Side::Right => (half(size.x) + half(master.grid_size.x) + gap + 1, 0),
                Side::Left => (-(half(size.x) + half(master.grid_size.x) + gap + 1), 0),
            };
            Ok(GridPosition::new(
                (i32::from(center.x) + dx) as i8,
                center.y,
                (i32::from(center.z) + dz) as i8,
            ))
        }
        Anchor::OnTopOf { of } => {
            let (center, size) = base_center(layout, *of)?;
            Ok(GridPosition::new(
                center.x,
                (i32::from(center.y) + size.y) as i8,
                center.z,
            ))
        }
        Anchor::Around { of, index } => {
            let (center, _) = base_center(layout, *of)?;
            let offsets = [
                (1, 0),
                (1, 1),
                (0, 1),
                (-1, 1),
                (-1, 0),
                (-1, -1),
                (0, -1),
                (1, -1),
            ];
            let (dx, dz) = offsets[usize::from(*index) % offsets.len()];
            Ok(GridPosition::new(
                (i32::from(center.x) + dx) as i8,
                center.y,
                (i32::from(center.z) + dz) as i8,
            ))
        }
        Anchor::Against { .. } => {
            let _ = current;
            Err(Refusal::new(RefusalCode::WrongLayer))
        }
    }
}

fn check(
    layout: &Layout,
    rules: &Rules,
    piece: Piece,
    row: &HousingLayoutFixture,
) -> Result<PlacedPiece, Refusal> {
    let apply = apply_row(piece, row, rules)?;
    let placed = PlacedPiece::new(&apply, rules).map_err(missing_refusal)?;
    board(layout, rules)?.can_put_floor(&placed, &[]).map_err(cell_refusal)?;
    Ok(placed)
}

fn result_of(piece: Piece, row: &HousingLayoutFixture) -> OpResult {
    let min = GridPosition::new(row.minimum.x, row.minimum.y, row.minimum.z);
    let max = GridPosition::new(row.maximum.x, row.maximum.y, row.maximum.z);
    OpResult {
        piece,
        minimum: min,
        maximum: max,
        center_y: row.center_y,
        rotation: Rot::from_row(row.rotation).unwrap_or(Rot::Deg0),
    }
}

fn owned_available(layout: &Layout, rules: &Rules, fixture: i32, texture: u32) -> i32 {
    let placed = layout
        .rows
        .iter()
        .filter(|row| row.row.mysekai_fixture_id == fixture && row.row.texture_id == texture)
        .count() as i32;
    let Some(owned) = rules.owned.get(&(fixture, texture)).copied() else {
        // An incomplete caller (no owned table) does not make the piece
        // unowned; the validator still checks the layout.
        return i32::MAX;
    };
    owned.quantity - owned.placed_elsewhere - placed
}

fn refused(index: usize, refusal: Refusal, results: &[OpResult]) -> Rejected {
    Rejected {
        index,
        refusal,
        results_before: results.to_vec(),
    }
}

fn apply_one(
    layout: &mut Layout,
    rules: &Rules,
    op: &LayoutOp,
) -> Result<OpResult, Refusal> {
    match op {
        LayoutOp::Place {
            fixture,
            texture,
            at,
            rotation,
        } => {
            let master = master(rules, *fixture)?;
            let texture = texture.unwrap_or(1);
            if master.layout & layout_type::FIELD == 0 {
                return Err(Refusal::new(RefusalCode::WrongLayer));
            }
            if owned_available(layout, rules, *fixture, texture) <= 0 {
                return Err(Refusal::new(RefusalCode::NotOwned));
            }
            let center = anchor_center(layout, at, master, None)?;
            let row = row_from(master, *fixture, texture, center, rotation.direction())?;
            let next = layout
                .rows
                .iter()
                .map(|placed| placed.piece.0)
                .max()
                .unwrap_or(0)
                + 1;
            let piece = Piece(next);
            check(layout, rules, piece, &row)?;
            let result = result_of(piece, &row);
            layout.rows.push(crate::PlacedRow { piece, row });
            Ok(result)
        }
        LayoutOp::Move {
            piece,
            at,
            rotation,
        } => {
            let current = placed_row(layout, *piece)?.clone();
            let master = master(rules, current.mysekai_fixture_id)?;
            let center = anchor_center(layout, at, master, Some(row_center(&current)?.0))?;
            let direction = rotation
                .map(Rot::direction)
                .unwrap_or_else(|| direction_of(&current));
            let row = row_from(
                master,
                current.mysekai_fixture_id,
                current.texture_id,
                center,
                direction,
            )?;
            check(layout, rules, *piece, &row)?;
            let index = layout
                .rows
                .iter()
                .position(|placed| placed.piece == *piece)
                .expect("the piece was found above");
            let result = result_of(*piece, &row);
            layout.rows[index].row = row;
            Ok(result)
        }
        LayoutOp::Rotate { piece, rotation } => {
            let current = placed_row(layout, *piece)?.clone();
            let master = master(rules, current.mysekai_fixture_id)?;
            let (center, _) = row_center(&current)?;
            let row = row_from(
                master,
                current.mysekai_fixture_id,
                current.texture_id,
                center,
                rotation.direction(),
            )?;
            let candidate = check(layout, rules, *piece, &row)?;
            let board = board(layout, rules)?;
            let stacked = board
                .stacked_on(&piece_uid(*piece))
                .iter()
                .filter_map(|uid| board.piece(uid).cloned())
                .collect::<Vec<_>>();
            if board.crossing_multiple_bases(&candidate, &stacked) {
                return Err(Refusal::new(RefusalCode::StackNotAllowed));
            }
            let index = layout
                .rows
                .iter()
                .position(|placed| placed.piece == *piece)
                .expect("the piece was found above");
            let result = result_of(*piece, &row);
            layout.rows[index].row = row;
            Ok(result)
        }
        LayoutOp::Remove { piece } => {
            let index = layout
                .rows
                .iter()
                .position(|placed| placed.piece == *piece)
                .ok_or_else(|| {
                    let mut refusal = Refusal::new(RefusalCode::UnknownPiece);
                    refusal.piece = Some(*piece);
                    refusal
                })?;
            let result = result_of(*piece, &layout.rows[index].row);
            layout.rows.remove(index);
            Ok(result)
        }
        LayoutOp::Recolor { piece, texture } => {
            let index = layout
                .rows
                .iter()
                .position(|placed| placed.piece == *piece)
                .ok_or_else(|| {
                    let mut refusal = Refusal::new(RefusalCode::UnknownPiece);
                    refusal.piece = Some(*piece);
                    refusal
                })?;
            layout.rows[index].row.texture_id = *texture;
            let result = result_of(*piece, &layout.rows[index].row);
            Ok(result)
        }
    }
}

/// Runs an [`Apply`] against `layout`.
///
/// All operations or none: every operation is first applied to a private
/// copy, and the caller's layout is replaced only when all of them succeed.
/// A dry run returns the same result without writing the copy back. The
/// operation contract's closed refusal codes are used; a missing master or
/// zone input is reported as a layer refusal because the closed set has no
/// input code.
pub fn apply(layout: &mut Layout, rules: &Rules, request: &Apply) -> ApplyOutcome {
    let mut work = layout.clone();
    let mut results = Vec::with_capacity(request.ops.len());
    for (index, op) in request.ops.iter().enumerate() {
        match apply_one(&mut work, rules, op) {
            Ok(result) => results.push(result),
            Err(refusal) => return Err(refused(index, refusal, &results)),
        }
    }
    if !request.dry_run {
        *layout = work;
    }
    Ok(Applied {
        results,
        revision: None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{PlacedRow, SiteGrid};
    use std::collections::HashMap;

    fn master() -> FixtureMaster {
        FixtureMaster {
            package: "pkg".into(),
            grid_size: Vector3Int::new(1, 1, 1),
            layout: moly_law::fixture::position::layout_type::FLOOR,
            traits: crate::validate::Traits::from_row("normal", "floor", "put_target", "none"),
        }
    }

    fn rules() -> Rules {
        let mut traits = HashMap::new();
        let mut masters = HashMap::new();
        traits.insert(1, master().traits);
        masters.insert(1, master());
        Rules {
            traits,
            masters,
            bundles: HashMap::new(),
            zones: Some(Vec::new()),
            owned: HashMap::new(),
            cleanup_protected: std::collections::HashSet::new(),
        }
    }

    fn row(piece: u32, x: i8, z: i8) -> PlacedRow {
        PlacedRow {
            piece: Piece(piece),
            row: row_from(
                &master(),
                1,
                1,
                GridPosition::new(x, 0, z),
                Direction::Front,
            )
            .expect("a one-cell row"),
        }
    }

    fn layout() -> Layout {
        Layout::new(
            1,
            "home_site".into(),
            SiteGrid::new(1, 1, 8, 4, 8),
            vec![row(1, 0, 0)],
        )
    }

    #[test]
    fn a_failed_transaction_leaves_the_layout_untouched() {
        let mut layout = layout();
        let before = layout.clone();
        let request = Apply {
            ops: vec![
                LayoutOp::Remove { piece: Piece(1) },
                LayoutOp::Remove { piece: Piece(9) },
            ],
            dry_run: false,
            base: None,
            check_paths: false,
        };
        let rejected = apply(&mut layout, &rules(), &request).expect_err("the second piece is unknown");
        assert_eq!(rejected.index, 1);
        assert_eq!(rejected.refusal.code, RefusalCode::UnknownPiece);
        assert_eq!(rejected.results_before.len(), 1);
        assert_eq!(layout, before);
    }

    #[test]
    fn a_dry_run_returns_results_without_changing_the_layout() {
        let mut layout = layout();
        let before = layout.clone();
        let request = Apply {
            ops: vec![LayoutOp::Remove { piece: Piece(1) }],
            dry_run: true,
            base: None,
            check_paths: false,
        };
        let applied = apply(&mut layout, &rules(), &request).expect("the piece exists");
        assert_eq!(applied.results.len(), 1);
        assert_eq!(applied.results[0].piece, Piece(1));
        assert_eq!(layout, before);
    }
}