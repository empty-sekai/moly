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

/// `MysekaiLayoutStatus`, the verdict of `SiteLayoutEditor.CheckSaveLayout`,
/// plus `Unavailable` for a check whose inputs are not ready (no verdict).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LayoutStatus {
    ErrorLayout = 1,
    ErrorAnimationSpace = 2,
    CutsceneAreaOverridden = 3,
    Unavailable,
}

/// `CheckSaveLayout(shouldValidateCutsceneArea)`: the cutscene-area check runs
/// only when `validate_cutscene` is set (the confirmation's OK runs the save
/// again without it).
pub(super) fn save(
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
    areas: &FixtureAreas,
    rules: Result<&Rules, Missing>,
    validate_cutscene: bool,
) -> Result<(), (LayoutStatus, String)> {
    check_save(rows, floor, areas, rules, validate_cutscene).map_err(|message| {
        let status = if message.starts_with("CutsceneAreaOverridden") {
            LayoutStatus::CutsceneAreaOverridden
        } else if message.starts_with("ErrorAnimationSpace") {
            LayoutStatus::ErrorAnimationSpace
        } else if message.starts_with("ErrorLayout") {
            LayoutStatus::ErrorLayout
        } else {
            LayoutStatus::Unavailable
        };
        (status, message)
    })
}

/// Source CheckSaveLayout status precedence: cutscene(3), animation space(2),
/// layout(1), success(0). Source master enforcement flags still await a
/// producer; the prior named empty enforcement mock is retained.
fn check_save(
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
    areas: &FixtureAreas,
    rules: Result<&Rules, Missing>,
    validate_cutscene: bool,
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
    for owner in rows.iter().filter(|_| validate_cutscene) {
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
                    "CutsceneAreaOverridden(3)：有家具占用了切景区域".into(),
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

/// `InValidLayoutFixtureListData.CreateInValidFixtureList` over the placed
/// floor-grid fixtures: each one whose `IsValidPutStatus` fails goes on the
/// list with what is stacked on it (`AddStackFixtureToInValidFixtureList`).
/// A failing house or gate (`AddInValidFixtureListCaseHouseOrGate`) puts the
/// fixtures standing on its ground tiles there instead, with their stacks,
/// and itself only when its box leaves the grid. The list is in row order.
///
/// Not covered: the `GetPlacedLayoutType` branch (its fixtures are listed
/// without the put check) and the house's extra entrance bounds; both read
/// fields of the scene's fixture controllers this draft does not carry.
pub(super) fn invalid_fixtures(
    rows: &[EditableFixture],
    floor: FloorGridLayout,
    rules: &Rules,
    is_house_or_gate: impl Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    let board = Board::new(rows, floor, rules)
        .map_err(|missing| format!("摆放检查缺少输入 {missing:?}"))?;
    let mut listed: Vec<String> = Vec::new();
    let add = |uid: &str, listed: &mut Vec<String>| {
        for uid in std::iter::once(uid.to_owned()).chain(board.stacked_on(uid)) {
            if !listed.contains(&uid) {
                listed.push(uid);
            }
        }
    };
    for item in rows {
        if item.layout & layout_type::FIELD == 0 {
            continue;
        }
        let Some(piece) = board.piece(&item.uid) else {
            continue;
        };
        let valid = if piece.traits.joint {
            board.holds_own_center(piece)
        } else {
            let stacked: Vec<Piece> = board
                .stacked_on(&item.uid)
                .iter()
                .filter_map(|uid| board.piece(uid).cloned())
                .collect();
            board.can_put_floor(piece, &stacked).is_ok()
        };
        if valid {
            continue;
        }
        if !is_house_or_gate(&item.package) {
            add(&item.uid, &mut listed);
            continue;
        }
        let (min, max) = item.footprint()?;
        let inside = axis_inside(min.x, floor.width)
            && axis_inside(max.x, floor.width)
            && axis_inside(min.z, floor.depth)
            && axis_inside(max.z, floor.depth);
        if !inside && !listed.contains(&item.uid) {
            listed.push(item.uid.clone());
        }
        for other in rows {
            if other.uid == item.uid || other.layout & layout_type::FIELD == 0 {
                continue;
            }
            let Ok((a, b)) = other.footprint() else {
                continue;
            };
            let on_ground = a.y <= 0 && b.y >= 0;
            let overlaps = a.x <= max.x && b.x >= min.x && a.z <= max.z && b.z >= min.z;
            if on_ground && overlaps {
                add(&other.uid, &mut listed);
            }
        }
    }
    Ok(listed)
}
