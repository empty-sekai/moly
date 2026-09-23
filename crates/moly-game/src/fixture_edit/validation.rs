//! Grid-space checks. Rendering bounds are never used as layout dimensions.

use super::{
    assets::{FixtureAreas, MetaGrid},
    PutStatus,
};
use crate::{fixture::EditableFixture, site::FloorGridLayout};
use moly_law::fixture::areas::{motion_area_bounds, rotated_center_grid, GridAreaData};
use moly_law::fixture::position::layout_type;
use std::collections::HashSet;

/// This slice edits ground furniture and rugs. Roads, walls and nonzero-height
/// stacks remain in the draft unchanged; source-specific controls come later.
pub(super) fn is_ground(item: &EditableFixture) -> bool {
    item.center.y == 0
        && matches!(item.layout, layout_type::FLOOR | layout_type::RUG)
        && item.fixture_id > 0
}

pub(super) fn put_label(status: PutStatus) -> &'static str {
    match status {
        PutStatus::Ok => "可以放置",
        PutStatus::OutOfBounds { .. } => "超出该地图当前等级的可摆范围",
        PutStatus::Overlap { .. } => "与同一布局层的家具重叠",
        PutStatus::Unsupported => "需要尚未接入的布局分支",
    }
}

fn axis_inside(x: i8, width: i32) -> bool {
    let half = (width + 1) / 2;
    (-half..half).contains(&(x as i32))
}

pub(super) fn put(
    item: &EditableFixture,
    rows: &[EditableFixture],
    floor: Option<FloorGridLayout>,
) -> PutStatus {
    if !is_ground(item) {
        return PutStatus::Unsupported;
    }
    let Some(floor) = floor else {
        return PutStatus::OutOfBounds { cells: 0 };
    };
    let Ok((min, max)) = item.footprint() else {
        return PutStatus::OutOfBounds { cells: 0 };
    };
    let occupied: Result<Vec<_>, _> = rows
        .iter()
        .filter(|row| row.uid != item.uid && row.layout == item.layout)
        .map(EditableFixture::footprint)
        .collect();
    let Ok(occupied) = occupied else {
        return PutStatus::OutOfBounds { cells: 0 };
    };
    let mut outside = 0;
    let mut overlap = 0;
    let height = if item.layout == layout_type::FLOOR {
        floor.height
    } else {
        1
    };
    // Check the full source volume, including height. Separate floor/rug tile
    // tables may overlap in x/z without incorrectly rejecting a rug under a chair.
    for x in min.x as i32..=max.x as i32 {
        for y in min.y as i32..=max.y as i32 {
            for z in min.z as i32..=max.z as i32 {
                if !axis_inside(x as i8, floor.width)
                    || !axis_inside(z as i8, floor.depth)
                    || y < 0
                    || y >= height
                {
                    outside += 1;
                } else if occupied.iter().any(|(a, b)| {
                    x >= a.x as i32
                        && x <= b.x as i32
                        && y >= a.y as i32
                        && y <= b.y as i32
                        && z >= a.z as i32
                        && z <= b.z as i32
                }) {
                    overlap += 1;
                }
            }
        }
    }
    if outside > 0 {
        PutStatus::OutOfBounds { cells: outside }
    } else if overlap > 0 {
        PutStatus::Overlap { cells: overlap }
    } else {
        PutStatus::Ok
    }
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
        if is_ground(item) {
            let status = put(item, rows, Some(floor));
            if status != PutStatus::Ok {
                return Err(format!("ErrorLayout(1)：{}{}", item.uid, put_label(status)));
            }
        }
    }
    Ok(())
}
