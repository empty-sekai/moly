//! Grid-space checks. Rendering bounds are never used as layout dimensions.
//!
//! The put check is the source's `CanPutFloor` (`tile_rules`) on the draft:
//! the committed rows are the tile data, the selected fixture and the
//! fixtures stacked on it are checked at their draft places.

use super::{
    assets::{FixtureAreas, MetaGrid},
    tile_rules::{self, Board, Missing, Piece, Refusal, Rules},
    PutStatus,
};
use crate::{fixture::EditableFixture, site::FloorGridLayout};
use moly_law::fixture::areas::{motion_area_bounds, rotated_center_grid, GridAreaData};
use moly_law::fixture::position::layout_type;
use std::collections::HashSet;

/// The floor editor's layouts (`FloorEditState` edits the floor and the rug
/// grids; walls, fences and roads have their own edit states, and the
/// fence/road handle types are refused where the master row is read).
pub(super) fn is_ground(item: &EditableFixture) -> bool {
    matches!(item.layout, layout_type::FLOOR | layout_type::RUG) && item.fixture_id > 0
}

pub(super) fn put_label(status: PutStatus) -> String {
    match status {
        PutStatus::Ok => "可以放置".into(),
        PutStatus::Refused(refusal) => tile_rules::refusal_label(refusal),
    }
}

/// `SiteLayoutUtility.CanPutFloor` for `item` at its draft place, with the
/// fixtures stacked on it (`stacked`) at theirs, against the tile data of
/// `rows`.
pub(super) fn put(
    item: &EditableFixture,
    stacked: &[EditableFixture],
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
    rules: Result<&Rules, Missing>,
) -> PutStatus {
    match check(item, stacked, rows, floor, rules) {
        Ok(()) => PutStatus::Ok,
        Err(refusal) => PutStatus::Refused(refusal),
    }
}

fn check(
    item: &EditableFixture,
    stacked: &[EditableFixture],
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
    rules: Result<&Rules, Missing>,
) -> Result<(), Refusal> {
    if !is_ground(item) {
        return Err(Refusal::Unsupported);
    }
    let floor = floor.ok_or(Refusal::Missing(Missing::FloorGrid))?;
    let rules = rules.map_err(Refusal::Missing)?;
    let board = Board::new(rows, floor, rules).map_err(Refusal::Missing)?;
    let piece = Piece::new(item, rules).map_err(Refusal::Missing)?;
    if piece.traits.joint {
        return Err(Refusal::Unsupported);
    }
    let stacked = stacked
        .iter()
        .map(|row| Piece::new(row, rules))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Refusal::Missing)?;
    board.can_put_floor(&piece, &stacked)
}

/// A cell of the product frame inside the floor grid's width (the cutscene
/// area check's bound).
fn axis_inside(x: i8, width: i32) -> bool {
    let half = (width + 1) / 2;
    (-half..half).contains(&(x as i32))
}

fn cutscene_cells(meta: &MetaGrid, owner: &EditableFixture) -> Result<HashSet<(i8, i8)>, String> {
    let mut area = GridAreaData::from_meta(meta.rows, meta.cols, |r, c| meta.cell(r, c));
    let (source_center, source_direction, _) = moly_assets::player_data::mirror_fixture_layout(
        owner.center,
        owner.grid_size,
        owner.direction,
        owner.layout,
    )?;
    area.rotate(source_direction, true);
    let center = rotated_center_grid(source_center, owner.grid_size, source_direction);
    area.enable_tiles
        .iter()
        .map(|tile| {
            Ok((
                i8::try_from(-i16::from(center.x.wrapping_add(tile.x)) - 1)
                    .map_err(|_| "cutscene cell exceeds grid domain")?,
                center.z.wrapping_add(tile.z),
            ))
        })
        .collect()
}

/// Source CheckSaveLayout status precedence: cutscene(3), animation space(2),
/// layout(1), success(0). The repair-confirmation UI is intentionally not
/// simulated by deleting rejected objects. Source master enforcement flags
/// still await a producer; the prior named empty enforcement mock is retained.
pub(super) fn save(
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
    areas: &FixtureAreas,
    rules: Result<&Rules, Missing>,
) -> Result<(), String> {
    if !areas.loaded {
        return Err("家具区域表仍在加载".into());
    }
    let Some(floor) = floor else {
        return Err("地图等级尺寸尚未就绪".into());
    };
    // Reject malformed drafts before any volume-based save checks. Valid
    // geometry still follows the source cutscene/animation/layout precedence.
    let footprints: Vec<_> = rows
        .iter()
        .map(|row| {
            row.footprint()
                .map_err(|error| format!("ErrorLayout(1)：{}：{error}", row.uid))
        })
        .collect::<Result<_, _>>()?;
    for owner in rows {
        if let Some(meta) = areas.cutscene.get(&owner.package) {
            let protected = cutscene_cells(meta, owner)?;
            if protected
                .iter()
                .any(|(x, z)| !axis_inside(*x, floor.width) || !axis_inside(*z, floor.depth))
                || rows
                    .iter()
                    .zip(&footprints)
                    .filter(|(row, _)| row.uid != owner.uid && row.layout == layout_type::FLOOR)
                    .any(|(_, (min, max))| {
                        (min.x..=max.x)
                            .any(|x| (min.z..=max.z).any(|z| protected.contains(&(x, z))))
                    })
            {
                return Err(
                    "CutsceneAreaOverridden(3)：有家具占用了切景区域，需要原作确认分支".into(),
                );
            }
        }
    }
    const ANIMATION_SPACE_ENFORCED: [&str; 0] = [];
    for item in rows {
        if ANIMATION_SPACE_ENFORCED.contains(&item.package.as_str()) {
            let Some(meta) = areas.motion.get(&item.package) else {
                return Err(format!(
                    "ErrorAnimationSpace(2)：{}缺少源动作区域",
                    item.uid
                ));
            };
            let (source_center, source_direction, _) =
                moly_assets::player_data::mirror_fixture_layout(
                    item.center,
                    item.grid_size,
                    item.direction,
                    item.layout,
                )?;
            let bounds = motion_area_bounds(
                meta.rows,
                meta.cols,
                |r, c| meta.cell(r, c),
                source_center,
                item.grid_size,
                source_direction,
            );
            if bounds.iter().any(|bound| {
                rows.iter()
                    .zip(&footprints)
                    .filter(|(row, _)| row.uid != item.uid && row.layout == layout_type::FLOOR)
                    .any(|(_, (a, b))| {
                        -i16::from(bound.max.x) - 1 <= i16::from(b.x)
                            && -i16::from(bound.min.x) - 1 >= i16::from(a.x)
                            && bound.min.z <= b.z
                            && bound.max.z >= a.z
                    })
            }) {
                return Err(format!(
                    "ErrorAnimationSpace(2)：{}动作空间被占用",
                    item.uid
                ));
            }
        }
    }
    let mut unique = HashSet::new();
    for item in rows {
        if item.uid.is_empty() || !unique.insert(item.uid.as_str()) {
            return Err("ErrorLayout(1)：家具UID为空或重复".into());
        }
    }
    // `IsValidPutStatus` for every placed fixture that is not on a wall: a
    // fence or road (`CanSaveJointObject`: the tile at its center holds it),
    // otherwise `CanPutFloor` in its placed layout with its stacked fixtures.
    // Wall fixtures (`CanPutWall`) belong to the wall edit branch.
    let rules = rules.map_err(|missing| format!("ErrorLayout(1)：摆放检查缺少输入 {missing:?}"))?;
    let board = Board::new(rows, floor, rules)
        .map_err(|missing| format!("ErrorLayout(1)：摆放检查缺少输入 {missing:?}"))?;
    for item in rows {
        if item.layout & layout_type::FIELD == 0 {
            continue;
        }
        let Some(piece) = board.piece(&item.uid) else {
            continue;
        };
        if piece.traits.joint {
            if !board.holds_own_center(piece) {
                return Err(format!("ErrorLayout(1)：{}的中心格不是它自己", item.uid));
            }
            continue;
        }
        let stacked: Vec<Piece> = board
            .stacked_on(&item.uid)
            .iter()
            .filter_map(|uid| board.piece(uid).cloned())
            .collect();
        if let Err(refusal) = board.can_put_floor(piece, &stacked) {
            return Err(format!(
                "ErrorLayout(1)：{}{}",
                item.uid,
                tile_rules::refusal_label(refusal)
            ));
        }
    }
    Ok(())
}
