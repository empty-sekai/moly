//! 家具占用格律：布尔格 × 旋转 → 世界格占用表。
//!
//! 源链（一次移动/旋转后的重建）：
//! `FixtureController.UpdateMotionAreaBoundList` → 旋转后的中心格
//! （`GetRotatedCenterGrid`）→ 当前朝向的占用格列表
//! （`FixtureModel.GetCurrentMotionAreaGridList`：拷贝 meta 布尔格
//! 装配的 `GridAreaData`，按朝向 Rotate）→ 每个 cell 一个单格
//! `GridBound`（Min = Max = 中心格 + cell）。
//!
//! 旋转的三个基础件（`SiteLayoutUtility`）逐值转录：
//! - `CalculateRotatedPosition(tile, center, dir)`：绕 center 旋转。
//!   d = tile − center，四朝向的 (x,z) 映射是
//!   Front:(x,z) · Left:(z,−x) · Back:(−x,−z) · Right:(−z,x)，
//!   y 分量原样透传，结果 = center + 旋转后的 d；
//! - `AdjustRotatePosition(tile, size, dir)`：把旋转后的落位平移回
//!   非负象限。m = max(size.x, size.z) − 1（负夹 0）；
//!   Right:+(m,0) · Back:+(m,m) · Left:+(0,m) · Front:原样；
//! - `GridBound` 构造：Min = position，**Max = position + (TileCount − 1)**
//!   （闭区间两端，不是 Min + TileCount）。单格 bound（TileCount =
//!   One）退化为 Min == Max。

use crate::fixture::{floor_half, max0, Direction, GridPosition, Vector3Int};

/// 世界格占用盒。Min/Max 是格坐标闭区间的两端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridBound {
    pub min: GridPosition,
    pub max: GridPosition,
    pub tile_count: Vector3Int,
}

/// `GridBound` 构造律：Max = Min + (TileCount − 1)，逐轴。
/// 源在机器码里就是这么算的（helper 返回 position + (count − 1)），
/// 写成 Min + TileCount 的话每个 bound 会宽一格。
pub fn grid_bound(position: GridPosition, tile_count: Vector3Int) -> GridBound {
    GridBound {
        min: position,
        max: GridPosition::new(
            position.x.wrapping_add((tile_count.x - 1) as i8),
            position.y.wrapping_add((tile_count.y - 1) as i8),
            position.z.wrapping_add((tile_count.z - 1) as i8),
        ),
        tile_count,
    }
}

/// `CalculateRotatedPosition`：tile 绕 center 转 dir。
pub fn calculate_rotated_position(
    tile: GridPosition,
    center: GridPosition,
    dir: Direction,
) -> GridPosition {
    let d = tile - center;
    let (new_x, new_z) = match dir {
        Direction::Front => (d.x, d.z),
        Direction::Left => (d.z, -d.x),
        Direction::Back => (-d.x, -d.z),
        Direction::Right => (-d.z, d.x),
    };
    center + GridPosition::new(new_x, d.y, new_z)
}

/// `AdjustRotatePosition`：旋转后的落位按占用尺寸平移回非负象限。
/// m = max(x,z) − 1 后负夹 0（源是同一条位式）。
pub fn adjust_rotate_position(
    tile: GridPosition,
    size: Vector3Int,
    dir: Direction,
) -> GridPosition {
    let m = max0(size.x.max(size.z) - 1);
    match dir {
        Direction::Front => tile,
        Direction::Left => tile + GridPosition::new(0, 0, m as i8),
        Direction::Back => tile + GridPosition::new(m as i8, 0, m as i8),
        Direction::Right => tile + GridPosition::new(m as i8, 0, 0),
    }
}

/// `GridAreaData.GetSquareMin(size)`（私有重载）：正方化后的最小角。
/// 与 Setup 的 min 不是同一个式子——这里带交叉项：
/// x' = −(floor(max(x−1,0)/2) + floor(max(z−x,0)/2))，z' 对称。
/// 交叉项丢了的话，扁长 footprint 的旋转落位会整体偏。
pub fn area_square_min(size: Vector3Int) -> GridPosition {
    GridPosition::new(
        -(floor_half(max0(size.x - 1)) + floor_half(max0(size.z - size.x))) as i8,
        0,
        -(floor_half(max0(size.z - 1)) + floor_half(max0(size.x - size.z))) as i8,
    )
}

/// meta 布尔格装配出的占用格数据（`GridAreaData.Setup`）。
///
/// Setup 的要点：min 角 = −floor((count−1)/2) 逐轴（**没有**交叉项），
/// cell 的 X 取列、Z 取行——布尔格的行列和格坐标的 x/z 是转置关系。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GridAreaData {
    pub is_overlap: bool,
    pub min: GridPosition,
    pub max: GridPosition,
    pub enable_tiles: Vec<GridPosition>,
}

impl GridAreaData {
    /// `Setup(D2Array<bool>)`：布尔格 → 有效格列表。
    /// `rows`/`cols` 是布尔格的行列数，`cell(row, col)` 取格值。
    pub fn from_meta(
        rows: usize,
        cols: usize,
        cell: impl Fn(usize, usize) -> bool,
    ) -> GridAreaData {
        let rows = rows as i32;
        let cols = cols as i32;
        let min = GridPosition::new(
            -floor_half(max0(cols - 1)) as i8,
            0,
            -floor_half(max0(rows - 1)) as i8,
        );
        let max = min + GridPosition::new((cols - 1) as i8, 0, (rows - 1) as i8);
        let mut enable_tiles = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                if cell(row as usize, col as usize) {
                    enable_tiles.push(min + GridPosition::new(col as i8, 0, row as i8));
                }
            }
        }
        GridAreaData { is_overlap: false, min, max, enable_tiles }
    }

    /// `Rotate(direction, isCenterZero)`。
    ///
    /// - cell：isCenterZero 时绕 GridPosition.Zero 转、**不做** Adjust；
    ///   否则绕正方化中心转、再做 Adjust；
    /// - 循环后的 min/max：无论哪个模式，都按「绕正方化中心转 + Adjust」
    ///   重算两端，再各取 X/Z 的 min/max。只有 X/Z 参与重算，y 留旧值
    ///   （Setup 之后恒为 0，cell 的 y 也在旋转里原样透传）。
    ///
    /// 旋转尺寸取的是**旋转前**的 min/max 跨度，循环内 cell 的移动
    /// 不影响它——源的尺寸在函数入口算一次。
    pub fn rotate(&mut self, direction: Direction, is_center_zero: bool) {
        let size = Vector3Int::new(
            (self.max.x as i32 - self.min.x as i32) + 1,
            0,
            (self.max.z as i32 - self.min.z as i32) + 1,
        );
        let square_min = area_square_min(size);
        for tile in &mut self.enable_tiles {
            *tile = if is_center_zero {
                calculate_rotated_position(*tile, GridPosition::ZERO, direction)
            } else {
                adjust_rotate_position(
                    calculate_rotated_position(*tile, square_min, direction),
                    size,
                    direction,
                )
            };
        }
        let rot_min = adjust_rotate_position(
            calculate_rotated_position(self.min, square_min, direction),
            size,
            direction,
        );
        let rot_max = adjust_rotate_position(
            calculate_rotated_position(self.max, square_min, direction),
            size,
            direction,
        );
        self.min.x = rot_min.x.min(rot_max.x);
        self.min.z = rot_min.z.min(rot_max.z);
        self.max.x = rot_min.x.max(rot_max.x);
        self.max.z = rot_min.z.max(rot_max.z);
    }
}

/// `FixtureLayoutData.GetSquareMin()`（无参重载）：摆放数据自己的
/// 正方化中心角，含与 `area_square_min` 同形的交叉项，锚在 Center 上。
pub fn layout_square_min(center: GridPosition, grid_size: Vector3Int) -> GridPosition {
    let a = floor_half(max0(grid_size.x - 1)) + floor_half(max0(grid_size.z - grid_size.x));
    let b = floor_half(max0(grid_size.z - 1)) + floor_half(max0(grid_size.x - grid_size.z));
    GridPosition::new(
        (center.x as i32 - a) as i8,
        center.y,
        (center.z as i32 - b) as i8,
    )
}

/// `FixtureController.GetRotatedCenterGrid`：中心格按朝向旋转后的落位
/// = Center 绕自己的正方化中心角转，再按**未换朝向的** GridSize 做
/// Adjust（这里不做 x/z 互换——那条换尺寸的律不在本链上）。
pub fn rotated_center_grid(
    center: GridPosition,
    grid_size: Vector3Int,
    direction: Direction,
) -> GridPosition {
    let square_min = layout_square_min(center, grid_size);
    adjust_rotate_position(
        calculate_rotated_position(center, square_min, direction),
        grid_size,
        direction,
    )
}

/// `UpdateMotionAreaBoundList`：世界格占用表。
///
/// meta 布尔格 → 拷贝后按朝向 Rotate（运动区走 isCenterZero = true，
/// 绕原点转、不 Adjust）→ 每个 cell 一个单格 GridBound，位置 =
/// 旋转后的中心格 + cell。调用侧每次移动/旋转重建整张表，不增量维护
/// ——这也是源的形状：列表先 Clear 再逐格 Add。
pub fn motion_area_bounds(
    rows: usize,
    cols: usize,
    cell: impl Fn(usize, usize) -> bool,
    center: GridPosition,
    grid_size: Vector3Int,
    direction: Direction,
) -> Vec<GridBound> {
    let mut area = GridAreaData::from_meta(rows, cols, cell);
    area.rotate(direction, true);
    let rotated_center = rotated_center_grid(center, grid_size, direction);
    area.enable_tiles
        .iter()
        .map(|&tile| grid_bound(rotated_center + tile, Vector3Int::ONE))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_true(_: usize, _: usize) -> bool {
        true
    }

    #[test]
    fn grid_bound_max_is_min_plus_count_minus_one() {
        // Min=(0,0,0)、TileCount=(3,2,5)：Max = Min + (2,1,4)。
        // 写成 Min + TileCount 的话这里是 (3,2,5)——这一臂抓它。
        let b = grid_bound(GridPosition::new(0, 0, 0), Vector3Int::new(3, 2, 5));
        assert_eq!(b.max, GridPosition::new(2, 1, 4));
        // 单格退化为 Min == Max。
        let b = grid_bound(GridPosition::new(7, -1, 4), Vector3Int::ONE);
        assert_eq!(b.min, GridPosition::new(7, -1, 4));
        assert_eq!(b.max, GridPosition::new(7, -1, 4));
    }

    #[test]
    fn setup_places_cells_with_x_from_columns_z_from_rows() {
        // 2 行 × 3 列全真：min = (−1,0,0)，max = (1,0,1)。
        // X 取列、Z 取行（转置）：cell 集合是 {−1..1} × {0..1}。
        let area = GridAreaData::from_meta(2, 3, all_true);
        assert_eq!(area.min, GridPosition::new(-1, 0, 0));
        assert_eq!(area.max, GridPosition::new(1, 0, 1));
        assert_eq!(area.enable_tiles.len(), 6);
        for x in -1..=1 {
            for z in 0..=1 {
                assert!(
                    area.enable_tiles.contains(&GridPosition::new(x, 0, z)),
                    "missing ({x},0,{z})"
                );
            }
        }
        // min 角是 −floor((count−1)/2)：4 列 → −1（不是 −2）。
        let area = GridAreaData::from_meta(1, 4, all_true);
        assert_eq!(area.min.x, -1);
        assert_eq!(area.max.x, 2);
    }

    #[test]
    fn setup_only_enables_true_cells() {
        // 2×2 只留对角：cells 恰好两个，min/max 仍按整格算。
        let area = GridAreaData::from_meta(2, 2, |r, c| (r + c) % 2 == 0);
        assert_eq!(area.min, GridPosition::ZERO);
        assert_eq!(area.max, GridPosition::new(1, 0, 1));
        assert_eq!(
            area.enable_tiles,
            vec![GridPosition::new(0, 0, 0), GridPosition::new(1, 0, 1)]
        );
    }

    #[test]
    fn rotation_maps_each_direction() {
        // 绕原点转 (1, 2, 3)：四朝向的 (x,z) 映射逐个对。
        let tile = GridPosition::new(1, 2, 3);
        let zero = GridPosition::ZERO;
        assert_eq!(
            calculate_rotated_position(tile, zero, Direction::Front),
            GridPosition::new(1, 2, 3)
        );
        assert_eq!(
            calculate_rotated_position(tile, zero, Direction::Left),
            GridPosition::new(3, 2, -1)
        );
        assert_eq!(
            calculate_rotated_position(tile, zero, Direction::Back),
            GridPosition::new(-1, 2, -3)
        );
        assert_eq!(
            calculate_rotated_position(tile, zero, Direction::Right),
            GridPosition::new(-3, 2, 1)
        );
        // y 透传、非零 center 的等价性：先平移到原点转再平移回去，
        // 与直接绕该 center 转，结果一致。
        let center = GridPosition::new(5, 0, 6);
        assert_eq!(
            calculate_rotated_position(tile, center, Direction::Left),
            center + calculate_rotated_position(tile - center, zero, Direction::Left)
        );
    }

    #[test]
    fn adjust_uses_max_axis_minus_one() {
        // size (3,·,1)：m = max(3,1)−1 = 2。Back 加 (m,0,m)。
        // 用 min 轴算 m 的话加的是 0——这一臂抓它。
        let got = adjust_rotate_position(
            GridPosition::ZERO,
            Vector3Int::new(3, 0, 1),
            Direction::Back,
        );
        assert_eq!(got, GridPosition::new(2, 0, 2));
        // Right 只加 x，Left 只加 z，Front 原样。
        let got = adjust_rotate_position(
            GridPosition::ZERO,
            Vector3Int::new(3, 0, 1),
            Direction::Right,
        );
        assert_eq!(got, GridPosition::new(2, 0, 0));
        let got = adjust_rotate_position(
            GridPosition::ZERO,
            Vector3Int::new(3, 0, 1),
            Direction::Left,
        );
        assert_eq!(got, GridPosition::new(0, 0, 2));
        let got = adjust_rotate_position(
            GridPosition::new(1, 0, 1),
            Vector3Int::new(3, 0, 1),
            Direction::Front,
        );
        assert_eq!(got, GridPosition::new(1, 0, 1));
    }

    #[test]
    fn area_square_min_carries_cross_terms() {
        // 4(x) × 2(z)：x' = −1，z' = −(0 + 1) = −1。
        // 交叉项被丢的话 z' 会是 0——这一臂抓它。
        let got = area_square_min(Vector3Int::new(4, 0, 2));
        assert_eq!(got, GridPosition::new(-1, 0, -1));
        // 对称尺寸退化为纯半宽：3×3 → (−1,0,−1)。
        assert_eq!(
            area_square_min(Vector3Int::new(3, 0, 3)),
            GridPosition::new(-1, 0, -1)
        );
    }

    #[test]
    fn layout_square_min_anchors_on_center() {
        // Center=(4,0,6)、GridSize=(1,1,1)：偏移全零，square == center。
        assert_eq!(
            layout_square_min(GridPosition::new(4, 0, 6), Vector3Int::ONE),
            GridPosition::new(4, 0, 6)
        );
        // GridSize=(4,·,2)、Center=(10,0,10)：
        // a = 1 + 0 = 1 → x = 9；b = 0 + 1 = 1 → z = 9。y 透传。
        assert_eq!(
            layout_square_min(GridPosition::new(10, 3, 10), Vector3Int::new(4, 0, 2)),
            GridPosition::new(9, 3, 9)
        );
    }

    #[test]
    fn rotate_center_zero_keeps_cells_near_origin() {
        // 1 行 × 2 列（cells (0,0,0)、(1,0,0)）绕原点右转：
        // Right: (x,z)→(−z,x) → cells (0,0,0)、(0,0,1)。
        let mut area = GridAreaData::from_meta(1, 2, all_true);
        area.rotate(Direction::Right, true);
        assert_eq!(
            area.enable_tiles,
            vec![GridPosition::new(0, 0, 0), GridPosition::new(0, 0, 1)]
        );
        // min/max 走「绕正方化中心 + Adjust」的重算：
        // 尺寸 (2,1)、square_min (0,0,0)，
        // rot(min=(0,0,0)) = Adjust((0,0,0)) = +(1,0,0) = (1,0,0)，
        // rot(max=(1,0,0)) = Adjust((0,0,1)) = (1,0,1)，
        // → min=(1,0,0)、max=(1,0,1)。
        assert_eq!(area.min, GridPosition::new(1, 0, 0));
        assert_eq!(area.max, GridPosition::new(1, 0, 1));
    }

    #[test]
    fn rotate_not_center_zero_adjusts_cells() {
        // 同一张格、isCenterZero=false：cell 走 Adjust 路径，
        // (0,0,0)→(1,0,0)、(1,0,0)→先转成 (0,0,1)→Adjust→(1,0,1)。
        let mut area = GridAreaData::from_meta(1, 2, all_true);
        area.rotate(Direction::Right, false);
        assert_eq!(
            area.enable_tiles,
            vec![GridPosition::new(1, 0, 0), GridPosition::new(1, 0, 1)]
        );
    }

    #[test]
    fn motion_area_of_single_cell_fixture() {
        // 1×1 meta、1×1 格、Center=(4,0,6)、Front：唯一 bound 在中心格。
        let bounds = motion_area_bounds(1, 1, all_true, GridPosition::new(4, 0, 6), Vector3Int::ONE, Direction::Front);
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0].min, GridPosition::new(4, 0, 6));
        assert_eq!(bounds[0].max, GridPosition::new(4, 0, 6));
        assert_eq!(bounds[0].tile_count, Vector3Int::ONE);
    }

    #[test]
    fn motion_area_of_wide_fixture_turning_right() {
        // 2 宽(x) × 1 深(z) 的家具，Center=(3,0,7)，右转 90°：
        // cells (0,0,0)、(1,0,0) 绕原点右转成 (0,0,0)、(0,0,1)；
        // 中心格 = Center 绕自身 square（重合）转，再 Right-Adjust
        // m = max(2,1)−1 = 1 → (4,0,7)。
        // 占用表 = {(4,0,7), (4,0,8)}——宽边转到了 z 上。
        let bounds = motion_area_bounds(
            1,
            2,
            all_true,
            GridPosition::new(3, 0, 7),
            Vector3Int::new(2, 1, 1),
            Direction::Right,
        );
        let positions: Vec<GridPosition> = bounds.iter().map(|b| b.min).collect();
        assert_eq!(
            positions,
            vec![GridPosition::new(4, 0, 7), GridPosition::new(4, 0, 8)]
        );
        // 方向感错了这一臂就红：若按 Left 转，中心落在 (3,0,8)、
        // 占用是 {(3,7),(3,8)}，与上面的集合在两根轴上都不同。
        for b in &bounds {
            assert_eq!(b.min, b.max);
        }
    }

    #[test]
    fn motion_area_of_symmetric_fixture_turning_left() {
        // 2×2 家具，Center=(5,0,5)，左转：cells 绕原点左转成
        // {(0,0,0),(0,0,−1),(1,0,0),(1,0,−1)}；中心 = (5,0,5) 绕自身
        // square 转 + Left-Adjust m=1 → (5,0,6)。
        // 占用 = {(5,5),(5,6),(6,5),(6,6)}。
        let bounds = motion_area_bounds(
            2,
            2,
            all_true,
            GridPosition::new(5, 0, 5),
            Vector3Int::new(2, 1, 2),
            Direction::Left,
        );
        let mut positions: Vec<GridPosition> = bounds.iter().map(|b| b.min).collect();
        positions.sort_by_key(|p| (p.x, p.z));
        assert_eq!(
            positions,
            vec![
                GridPosition::new(5, 0, 5),
                GridPosition::new(5, 0, 6),
                GridPosition::new(6, 0, 5),
                GridPosition::new(6, 0, 6),
            ]
        );
    }
}
