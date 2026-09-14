//! 家具摆放位置律：足迹对角中点 + 墙布局外推 + 朝向。
//!
//! 源链：`SetGridPosition` → `FixtureLayoutData`（min/max 格、Center、
//! GridSize、Direction、LayoutType）→ `GetFieldPosition` → 视图落位。
//! 本文件转录这条链的两段：**足迹两端怎么来的**（`InitializeTiles` +
//! `RotationTile`，见 [`footprint_rotated`]）与**足迹两端怎么变成世界位**
//! （`GetFieldPosition`），外加视图那一侧的朝向角（[`direction_yaw_degrees`]）。
//!
//! ⚠ **朝向不是只作用在模型的旋转上，它先改足迹。** min/max 在源里
//! 不是存档列而是从 (Center, GridSize, Direction) 推出来的，而世界位读
//! 的就是推出来的那对 min/max ⇒ 同一件家具换个朝向，**世界位本身会动**
//! （偶数边足迹上实测动 0.125，见测试 `even_span_position_moves_with_direction`）。
//! 把朝向只接到模型旋转上、位置沿用 Front 的那对 min/max，在奇数边足迹
//! 上看不出差别，在偶数边上错半格。
//!
//! 三个钉在源方法体上的要点：
//! - 位置是**足迹对角中点**：min 角取 `Location.Min`（格值 ×0.25），
//!   max 角取 `Location.Max`（格值 ×0.25 再 +0.25，即那格的远角），
//!   x/z 各取两角的中点；
//! - **y 不走中点**：y = 格值 0.25 × Center.Y。伪代码里这一段的
//!   返回寄存器混写会把 y 也写成中点，原生反编译钉死它不是；
//! - 墙布局（LayoutType 命中 0xF0 任一位）时 center −= 墙法线 ×0.125
//!   （= 格值 0.25 × 0.5），把家具从墙格中心推离墙面。

use crate::fixture::areas::{
    adjust_rotate_position, calculate_rotated_position, layout_square_min,
};
use crate::fixture::{floor_half, max0, Direction, GridPosition, Vector3Int};

/// 格的世界尺寸，三轴同值（源常量表 MysekaiConstants 的 tile 项）。
pub const TILE_SIZE: f32 = 0.25;

/// 墙外推量：TILE_SIZE × 0.5。
pub const WALL_PUSHOUT: f32 = 0.125;

/// 墙布局掩码：LayoutType 的 wall 四位任一命中即视为墙摆放。
pub const WALL_LAYOUT_MASK: u8 = 0xF0;

/// 源 MysekaiLayoutType 的取值，按位组合。
pub mod layout_type {
    pub const NONE: u8 = 1;
    pub const FLOOR: u8 = 2;
    pub const RUG: u8 = 4;
    pub const ROAD: u8 = 8;
    pub const WALL_FRONT: u8 = 16;
    pub const WALL_BACK: u8 = 32;
    pub const WALL_RIGHT: u8 = 64;
    pub const WALL_LEFT: u8 = 128;
    pub const FIELD: u8 = 14;
    pub const WALL: u8 = 240;
    pub const ALL: u8 = 254;
}

/// 源 GridType 的取值（墙类别 + 地板）。None 只出现在错误路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridType {
    Floor = 1,
    Front = 2,
    Back = 3,
    Right = 4,
    Left = 5,
}

/// `SiteLayoutUtility.GetGridType`：LayoutType → 格类别。
///
/// 源的分支序是**判定的优先序**：地板位（bit1）先于一切墙位，
/// 墙位里 Front > Back > Left > Right——bit7（Left）压过 bit6（Right），
/// 不是按数值大小排的。四个墙位都不命中时源走错误日志并返回 0，
/// 这里折成 Err：rug/road/none 一类没有地板位也没有墙位的取值
/// 只会从直呼本函数的路径进来，墙外推路径根本碰不到它们。
pub fn grid_type(layout: u8) -> Result<GridType, String> {
    if (layout >> 1) & 1 != 0 {
        return Ok(GridType::Floor);
    }
    if (layout >> 4) & 1 != 0 {
        return Ok(GridType::Front);
    }
    if (layout >> 5) & 1 != 0 {
        return Ok(GridType::Back);
    }
    if (layout >> 7) & 1 != 0 {
        return Ok(GridType::Left);
    }
    if (layout >> 6) & 1 != 0 {
        return Ok(GridType::Right);
    }
    Err(format!("layout type {layout:#04x} has no grid type"))
}

/// `WallMathUtility.GetWallNormal`（GridType 重载）的表：墙格朝向的法线，
/// y 恒 0。表体读自原生本体的静态数据段。
pub fn wall_normal(grid: GridType) -> [f32; 3] {
    match grid {
        GridType::Front => [0.0, 0.0, 1.0],
        GridType::Back => [0.0, 0.0, -1.0],
        GridType::Right => [-1.0, 0.0, 0.0],
        GridType::Left => [1.0, 0.0, 0.0],
        // Floor 在墙法线的表外（源会落进错误日志 + 零向量），
        // 调用方不该把 Floor 喂进来；给零向量保持源的兜底形状。
        GridType::Floor => [0.0, 0.0, 0.0],
    }
}

/// 一根格轴上的位置式：`(min 格角 + max 格角) × 0.5`。
/// min 角 = min × 0.25，max 角 = max × 0.25 + 0.25（远角）。
fn axis_midpoint(min: i8, max: i8) -> f32 {
    (min as f32 * TILE_SIZE + (max as f32 * TILE_SIZE + TILE_SIZE)) * 0.5
}

/// `FixtureLayoutData.GetFieldPosition`：家具的世界落位。
///
/// x/z 是足迹对角中点；y = 0.25 × `center_y`（不走中点）；
/// 墙布局时沿墙法线外推 0.125（法线按 LayoutType 推出的格类别查表，
/// 即 `GetWallNormal(GetGridType(layoutType))` 的复合）。
pub fn field_position(
    min: GridPosition,
    max: GridPosition,
    center_y: i8,
    layout: u8,
) -> Result<[f32; 3], String> {
    let mut position = [
        axis_midpoint(min.x, max.x),
        TILE_SIZE * center_y as f32,
        axis_midpoint(min.z, max.z),
    ];
    if layout & WALL_LAYOUT_MASK != 0 {
        let normal = wall_normal(grid_type(layout)?);
        position[0] -= normal[0] * WALL_PUSHOUT;
        position[1] -= normal[1] * WALL_PUSHOUT;
        position[2] -= normal[2] * WALL_PUSHOUT;
    }
    Ok(position)
}

/// `FixtureView.ForceSetRotation`：摆放件的世界朝向角（度）。
///
/// 视图侧两条朝向方法给的是**同一个目标角**：载入那条
/// （`ForceSetRotation`，站点布局载入器经 `SetupLayout` → `ForceSetRotate`
/// 走它）直接写 `transform.eulerAngles = (0, 朝向字节 × 90, 0)`；交互那条
/// （`SetRotation`）把同一个 `(0, 朝向字节 × 90, 0)` 交给补间去插值。
/// 原生本体里那个乘数是立即数 0x5a。
///
/// 三件由此定死，不必猜：
/// - **轴**是 Y（euler 的另两个分量是字面 0）；
/// - **枢轴**是视图自己的 transform 原点 —— `eulerAngles` 就是绕自身原点
///   转，源里没有任何配套的平移补偿；朝向带来的位移全在足迹那一侧
///   （见 [`footprint_rotated`]），不在这个角里；
/// - **符号**是正角。本库的世界系与源同为左手 Y 向上、格轴不取负，家具
///   模型也是照 authored 空间原样取件（提取侧对家具明确不做轴反射），
///   而 `Quat::from_rotation_y(t)` 与源的 `Euler(0, t, 0)` 是同一个矩阵
///   （两者都把 +Z 映到 `(sin t, 0, cos t)`）⇒ 直接用正角，不翻符号。
pub fn direction_yaw_degrees(direction: Direction) -> f32 {
    (direction as i32) as f32 * 90.0
}

/// `FixtureLayoutData.InitializeTiles`：朝向复位成 Front 后的足迹两端。
///
/// min = Center − floor((GridSize − 1) / 2)（x/z 两轴；**y 取 Center.y
/// 原样**，不减半），max = min + (GridSize − 1) 逐轴（闭区间两端，与
/// `GridBound` 同一条约定）。
///
/// 源那个除以二是 16 位的「向零取整」，与这里的 `floor_half(max0(..))`
/// 只在 GridSize 某轴 ≤ 0 时才分道；本函数的调用方要么给 master 的
/// gridSize、要么给 [`footprint_to_center_size`] 的返回值，后者按构造
/// 每轴 ≥ 1（它拒绝 max < min）⇒ 两式在可达域上逐值相同。写成
/// `floor_half(max0(..))` 是为了和同族的 `layout_square_min` 一个形状。
pub fn footprint_front(
    center: GridPosition,
    grid_size: Vector3Int,
) -> (GridPosition, GridPosition) {
    let min = GridPosition::new(
        (center.x as i32 - floor_half(max0(grid_size.x - 1))) as i8,
        center.y,
        (center.z as i32 - floor_half(max0(grid_size.z - 1))) as i8,
    );
    let max = min
        + GridPosition::new(
            (grid_size.x - 1) as i8,
            (grid_size.y - 1) as i8,
            (grid_size.z - 1) as i8,
        );
    (min, max)
}

/// `FixtureLayoutData.SetRotate` 的非墙支（`InitializeTiles` →
/// `RotationTile`）：当前朝向下的足迹两端。
///
/// `RotationTile` 的形状有两处容易读丢，两处都会在扁长足迹上错位：
/// - 转的是 **Front 足迹的那两个端点**，绕的是 `GetSquareMin()`
///   （= [`layout_square_min`]，带交叉项的正方化角，**不是**足迹中心），
///   每个端点转完再各做一次 `AdjustRotatePosition`（把落位平移回非负
///   象限，平移量由 max(GridSize.x, GridSize.z) − 1 定）；
/// - 两个端点转完之后**逐轴取 min/max 当新的两端**。转 90° 会把原来的
///   min 端送到 max 那一侧，少了这一步得到的是一个上下端点颠倒的盒子。
///
/// 「转 90°/270° 时 w 与 d 互换」是这条律在方形格上的**表象**，不是它的
/// 定义：偶数边足迹上除了互换，两端还会整体平移半格（见测试）。
pub fn footprint_rotated(
    center: GridPosition,
    grid_size: Vector3Int,
    direction: Direction,
) -> (GridPosition, GridPosition) {
    let (front_min, front_max) = footprint_front(center, grid_size);
    let square_min = layout_square_min(center, grid_size);
    let rotate = |tile: GridPosition| {
        adjust_rotate_position(
            calculate_rotated_position(tile, square_min, direction),
            grid_size,
            direction,
        )
    };
    let a = rotate(front_min);
    let b = rotate(front_max);
    (
        GridPosition::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
        GridPosition::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
    )
}

/// `InitializeTiles` 的逆：Front 足迹的两端 → (Center, GridSize)。
///
/// 存档列带的是足迹两端，而上面两条律要的是 (Center, GridSize)，所以
/// 需要这一步。GridSize = max − min + 1 逐轴；Center = min +
/// floor((GridSize − 1) / 2)（x/z），Center.y = min.y（`InitializeTiles`
/// 把 Center.y 原样写进 min.y）。与 [`footprint_front`] 严格互逆。
///
/// max 某轴小于 min 是无效足迹（闭区间两端颠倒），**明确拒绝**：让它按
/// 「跨度 0 或负」算下去会得到一个静默错位的落点。
pub fn footprint_to_center_size(
    min: GridPosition,
    max: GridPosition,
) -> Result<(GridPosition, Vector3Int), String> {
    if max.x < min.x || max.y < min.y || max.z < min.z {
        return Err(format!(
            "footprint corners are inverted: min {:?} max {:?}",
            (min.x, min.y, min.z),
            (max.x, max.y, max.z),
        ));
    }
    let grid_size = Vector3Int::new(
        max.x as i32 - min.x as i32 + 1,
        max.y as i32 - min.y as i32 + 1,
        max.z as i32 - min.z as i32 + 1,
    );
    let center = GridPosition::new(
        (min.x as i32 + floor_half(max0(grid_size.x - 1))) as i8,
        min.y,
        (min.z as i32 + floor_half(max0(grid_size.z - 1))) as i8,
    );
    Ok((center, grid_size))
}

/// 存档列（Front 足迹两端）+ 朝向 → 当前朝向下的足迹两端。
///
/// [`footprint_to_center_size`] 与 [`footprint_rotated`] 的复合：摆放列
/// 记的是 Front 足迹（朝向 Front 时 `RotationTile` 是恒等，两者同值），
/// 消费侧只有这一个入口，落地件与占用面不会各算一份。
pub fn placed_footprint(
    front_min: GridPosition,
    front_max: GridPosition,
    direction: Direction,
) -> Result<(GridPosition, GridPosition), String> {
    let (center, grid_size) = footprint_to_center_size(front_min, front_max)?;
    Ok(footprint_rotated(center, grid_size, direction))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_position_is_footprint_diagonal_midpoint() {
        // min=(0,0,0) max=(2,0,0)：x 中点 = (0 + 0.75)×0.5 = 0.375。
        // z 上 min=max=0：z = (0 + 0.25)×0.5 = 0.125。
        // y = 0.25 × 3 = 0.75，与对角无关。
        let p = field_position(
            GridPosition::new(0, 0, 0),
            GridPosition::new(2, 0, 0),
            3,
            layout_type::FLOOR,
        )
        .unwrap();
        assert_eq!(p, [0.375, 0.75, 0.125]);
    }

    #[test]
    fn min_corner_uses_near_corner_not_far_corner() {
        // min=(1,·,0) max=(3,·,0)：x = (0.25 + 1.0)×0.5 = 0.625。
        // 若 min 角误取远角（0.5），会得到 0.75——这一臂抓它。
        let p = field_position(
            GridPosition::new(1, 0, 0),
            GridPosition::new(3, 0, 0),
            0,
            layout_type::FLOOR,
        )
        .unwrap();
        assert_eq!(p[0], 0.625);
        // z 负方向同理：min.z=-1 max.z=1 → (−0.25 + 0.5)×0.5 = 0.125。
        let p = field_position(
            GridPosition::new(0, 0, -1),
            GridPosition::new(0, 0, 1),
            0,
            layout_type::FLOOR,
        )
        .unwrap();
        assert_eq!(p[2], 0.125);
    }

    #[test]
    fn y_uses_center_grid_not_diagonal_midpoint() {
        // min.y=max.y=0 而 center_y=2：y = 0.5（0.25 × 2）。
        // 若 y 也走对角中点会得到 0.125——这一臂抓「y 取中点」的误读。
        let p = field_position(
            GridPosition::new(0, 0, 0),
            GridPosition::new(0, 0, 0),
            2,
            layout_type::FLOOR,
        )
        .unwrap();
        assert_eq!(p[1], 0.5);
    }

    #[test]
    fn wall_front_pushes_back_along_z() {
        // 前墙：法线 (0,0,1)，z -= 0.125。单格足迹 z = 0.125 → 0。
        let p = field_position(
            GridPosition::ZERO,
            GridPosition::ZERO,
            0,
            layout_type::WALL_FRONT,
        )
        .unwrap();
        assert_eq!(p, [0.125, 0.0, 0.0]);
        // 后墙：法线 (0,0,-1)，z 反向外推 → 0.25。法线方向写反时这一臂红。
        let p = field_position(
            GridPosition::ZERO,
            GridPosition::ZERO,
            0,
            layout_type::WALL_BACK,
        )
        .unwrap();
        assert_eq!(p, [0.125, 0.0, 0.25]);
    }

    #[test]
    fn wall_left_and_right_push_along_x() {
        // 左墙法线 (1,0,0)：x -= 0.125 → 0。右墙 (-1,0,0)：x += 0.125 → 0.25。
        let p = field_position(
            GridPosition::ZERO,
            GridPosition::ZERO,
            0,
            layout_type::WALL_LEFT,
        )
        .unwrap();
        assert_eq!(p[0], 0.0);
        let p = field_position(
            GridPosition::ZERO,
            GridPosition::ZERO,
            0,
            layout_type::WALL_RIGHT,
        )
        .unwrap();
        assert_eq!(p[0], 0.25);
    }

    #[test]
    fn non_wall_layouts_do_not_push() {
        // road/rug 没有墙位也没有地板位：不外推、位置照算（也不报错，
        // 报错只属于直呼 grid_type 的路径）。
        for layout in [layout_type::ROAD, layout_type::RUG] {
            let p = field_position(
                GridPosition::ZERO,
                GridPosition::ZERO,
                0,
                layout,
            )
            .unwrap();
            assert_eq!(p, [0.125, 0.0, 0.125], "layout {layout:#04x}");
        }
    }

    #[test]
    fn grid_type_priority_is_floor_then_front_back_left_right() {
        // 地板位压过一切墙位：all=254 含全部墙位，仍判 Floor。
        assert_eq!(grid_type(layout_type::ALL).unwrap(), GridType::Floor);
        assert_eq!(grid_type(layout_type::FLOOR).unwrap(), GridType::Floor);
        assert_eq!(grid_type(layout_type::FIELD).unwrap(), GridType::Floor);
        // 墙位优先序：Front > Back > Left > Right。
        assert_eq!(grid_type(layout_type::WALL_FRONT).unwrap(), GridType::Front);
        assert_eq!(grid_type(layout_type::WALL_BACK).unwrap(), GridType::Back);
        assert_eq!(grid_type(layout_type::WALL_LEFT).unwrap(), GridType::Left);
        assert_eq!(grid_type(layout_type::WALL_RIGHT).unwrap(), GridType::Right);
        // bit7(Left) 压过 bit6(Right)——按位数值排序的误读在这一臂红。
        assert_eq!(
            grid_type(layout_type::WALL_LEFT | layout_type::WALL_RIGHT).unwrap(),
            GridType::Left
        );
        assert_eq!(
            grid_type(layout_type::WALL_FRONT | layout_type::WALL_BACK).unwrap(),
            GridType::Front
        );
        // 全墙面（240）：Front 先命中。
        assert_eq!(grid_type(layout_type::WALL).unwrap(), GridType::Front);
        // 没有地板位也没有墙位：响亮拒绝。
        for layout in [
            layout_type::NONE,
            layout_type::RUG,
            layout_type::ROAD,
        ] {
            assert!(grid_type(layout).is_err(), "layout {layout:#04x}");
        }
    }

    // ---- 朝向：四臂 + 各自的反向臂 -------------------------------------
    //
    // ⚠ 这一族判据**不能靠现有摆放数据跑起来**：那批摆放行的朝向全是
    // Front，而 Front ⇒ 角 0 ⇒ 旋转矩阵是单位元、足迹律是恒等
    // ⇒ 接与不接逐值相同。所以每一臂的非 Front 输入都在这里现构造。

    const ALL_DIRECTIONS: [Direction; 4] = [
        Direction::Front,
        Direction::Left,
        Direction::Back,
        Direction::Right,
    ];

    #[test]
    fn yaw_is_ninety_degrees_per_direction_step() {
        // 源：视图侧把角算成朝向字节 × 90（原生本体里立即数 0x5a）。
        assert_eq!(direction_yaw_degrees(Direction::Front), 0.0);
        assert_eq!(direction_yaw_degrees(Direction::Left), 90.0);
        assert_eq!(direction_yaw_degrees(Direction::Back), 180.0);
        assert_eq!(direction_yaw_degrees(Direction::Right), 270.0);
        // 成对的那一半：四个角必须两两不等。少了这一臂，一个恒返回 0
        // 的实现（= 根本没接朝向）能过上面四行里的第一行并在其余三行红，
        // 但一个「全都返回同一个非零角」的实现只有这一臂抓得到。
        for (i, a) in ALL_DIRECTIONS.iter().enumerate() {
            for b in &ALL_DIRECTIONS[i + 1..] {
                assert_ne!(
                    direction_yaw_degrees(*a),
                    direction_yaw_degrees(*b),
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    /// 足迹两端 → (x 跨度, z 跨度)。判据按跨度对账 w/d 互换。
    fn spans(min: GridPosition, max: GridPosition) -> (i32, i32) {
        (
            max.x as i32 - min.x as i32 + 1,
            max.z as i32 - min.z as i32 + 1,
        )
    }

    #[test]
    fn front_arm_is_identical_to_the_unrotated_footprint() {
        // ⭐ 成对写的那一半：Front 臂必须与「不接旋转」逐值相同。
        // 这条挡住的是「接朝向时顺手引入了一个常量偏移」——那种错在
        // 三个非 Front 臂上都看不出来（它们本来就该动）。
        for (center, size) in [
            (GridPosition::new(0, 0, 0), Vector3Int::new(1, 1, 1)),
            (GridPosition::new(0, 0, 0), Vector3Int::new(3, 1, 1)),
            (GridPosition::new(0, 0, 0), Vector3Int::new(2, 1, 1)),
            (GridPosition::new(-4, 2, 7), Vector3Int::new(2, 3, 5)),
        ] {
            let unrotated = footprint_front(center, size);
            let rotated = footprint_rotated(center, size, Direction::Front);
            assert_eq!(rotated, unrotated, "center {center:?} size {size:?}");
        }
    }

    #[test]
    fn quarter_turns_swap_the_footprint_spans() {
        // 3x1 的扁长足迹，中心在原点。Front 足迹：x 从 -1 到 1、z 单格。
        let center = GridPosition::new(0, 0, 0);
        let size = Vector3Int::new(3, 1, 1);
        assert_eq!(
            footprint_front(center, size),
            (GridPosition::new(-1, 0, 0), GridPosition::new(1, 0, 0))
        );

        // 四臂的逐值落位。这些数手算自源式子：正方化角 = (-1, 0, -1)
        // （带交叉项；丢了交叉项 Left 臂会落到 x = -1 上）、Adjust 的
        // 平移量 m = max(3, 1) - 1 = 2。
        let arms: [(Direction, (GridPosition, GridPosition)); 4] = [
            (
                Direction::Front,
                (GridPosition::new(-1, 0, 0), GridPosition::new(1, 0, 0)),
            ),
            (
                Direction::Left,
                (GridPosition::new(0, 0, -1), GridPosition::new(0, 0, 1)),
            ),
            (
                Direction::Back,
                (GridPosition::new(-1, 0, 0), GridPosition::new(1, 0, 0)),
            ),
            (
                Direction::Right,
                (GridPosition::new(0, 0, -1), GridPosition::new(0, 0, 1)),
            ),
        ];
        for (direction, expected) in arms {
            assert_eq!(
                footprint_rotated(center, size, direction),
                expected,
                "{direction:?}"
            );
        }

        // w/d 互换（本单要交付的那条一致性）：四分之一转把跨度换轴。
        let (fw, fd) = spans(arms[0].1 .0, arms[0].1 .1);
        let (lw, ld) = spans(arms[1].1 .0, arms[1].1 .1);
        assert_eq!((fw, fd), (3, 1));
        assert_eq!((lw, ld), (1, 3));
        assert_eq!((fw, fd), (ld, lw));
        // 半转不换轴（180 度是同一对跨度）——这条挡住「所有非 Front
        // 都换轴」的误读。
        assert_eq!(spans(arms[2].1 .0, arms[2].1 .1), (3, 1));
        assert_eq!(spans(arms[3].1 .0, arms[3].1 .1), (1, 3));
    }

    #[test]
    fn rotated_corners_are_reduced_to_an_aabb_not_kept_in_place() {
        // 转 90 度会把 Front 的 min 端送到 max 那一侧：3x1 的 Left 臂里
        // 「转完的 min 端」是 z = 1、「转完的 max 端」是 z = -1。
        // 源在两端各转完之后逐轴取 min/max，少了那一步得到的是一个
        // 上下端点颠倒的盒子。
        let (min, max) = footprint_rotated(
            GridPosition::new(0, 0, 0),
            Vector3Int::new(3, 1, 1),
            Direction::Left,
        );
        assert!(min.x <= max.x && min.y <= max.y && min.z <= max.z);
        assert_eq!((min.z, max.z), (-1, 1));
        // ⚠ 这个错**世界位抓不到**：对角中点对颠倒的两端给出同一个数
        // （min=1/max=-1 与 min=-1/max=1 的中点都是 0.125）⇒ 只有足迹
        // 本身的判据会红。占用面与挖洞侧读的是足迹，不是中点。
        assert_eq!(axis_midpoint(1, -1), axis_midpoint(-1, 1));
    }

    #[test]
    fn adjust_step_anchors_the_rotated_footprint() {
        // 3x1 的 Left 臂：两端转完（未 Adjust）是 z = -1 与 z = -3，
        // Adjust 的 +2 把它搬回 z 的 -1..1。丢了 Adjust 这一步，足迹
        // 与世界位一起沿 z 偏 2 格 = 0.5。
        let (min, max) = footprint_rotated(
            GridPosition::new(0, 0, 0),
            Vector3Int::new(3, 1, 1),
            Direction::Left,
        );
        assert_eq!((min.z, max.z), (-1, 1));
        let position = field_position(min, max, 0, layout_type::FLOOR).unwrap();
        assert_eq!(position, [0.125, 0.0, 0.125]);
        // 未 Adjust 的那对端点会给出 z = -0.375，与上面差 0.5。
        assert_eq!(
            field_position(
                GridPosition::new(0, 0, -3),
                GridPosition::new(0, 0, -1),
                0,
                layout_type::FLOOR
            )
            .unwrap()[2],
            -0.375
        );
    }

    #[test]
    fn even_span_position_moves_with_direction() {
        // ⭐ 这一臂是「枢轴不是原地转」的证据：奇数边足迹上四个朝向的
        // 世界位恰好重合（见 quarter_turns 那条 3x1，Front 与 Left 都落
        // 在 0.125/0.125），**偶数边足迹上会动**。一个「位置沿用 Front
        // 的足迹、只把模型转起来」的实现在奇数边上永远看不出错。
        let center = GridPosition::new(0, 0, 0);
        let size = Vector3Int::new(2, 1, 1);
        let at = |direction| {
            let (min, max) = footprint_rotated(center, size, direction);
            field_position(min, max, 0, layout_type::FLOOR).unwrap()
        };
        assert_eq!(at(Direction::Front), [0.25, 0.0, 0.125]);
        assert_eq!(at(Direction::Left), [0.125, 0.0, 0.25]);
        // 半转也动：Adjust 对 Back 两轴各加 m = max(2,1) - 1 = 1，而 z
        // 的跨度只有 1 ⇒ 整盒沿 z 搬一格。这是源的算法本身，不是笔误。
        assert_eq!(at(Direction::Back), [0.25, 0.0, 0.375]);
        assert_eq!(at(Direction::Right), [0.375, 0.0, 0.25]);
        // 四个朝向的世界位两两不等——奇数边上做不到这一点，所以这一臂
        // 必须用偶数边足迹。
        for (i, a) in ALL_DIRECTIONS.iter().enumerate() {
            for b in &ALL_DIRECTIONS[i + 1..] {
                assert_ne!(at(*a), at(*b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn footprint_to_center_size_inverts_initialize_tiles() {
        for (center, size) in [
            (GridPosition::new(0, 0, 0), Vector3Int::new(1, 1, 1)),
            (GridPosition::new(0, 0, 0), Vector3Int::new(3, 1, 1)),
            (GridPosition::new(0, 0, 0), Vector3Int::new(2, 1, 1)),
            (GridPosition::new(2, 3, -5), Vector3Int::new(4, 2, 7)),
            (GridPosition::new(-9, 0, 11), Vector3Int::new(6, 1, 6)),
        ] {
            let (min, max) = footprint_front(center, size);
            let (back_center, back_size) = footprint_to_center_size(min, max).unwrap();
            assert_eq!(back_center, center, "center {center:?} size {size:?}");
            assert_eq!(back_size, size, "center {center:?} size {size:?}");
        }
        // 颠倒的两端：明确拒绝，不按「跨度 0 或负」算下去。逐轴各一臂
        // ——只查一根轴的实现会在另两根上静默错位。
        for (min, max) in [
            (GridPosition::new(0, 0, 0), GridPosition::new(-1, 0, 0)),
            (GridPosition::new(0, 0, 0), GridPosition::new(0, -1, 0)),
            (GridPosition::new(0, 0, 0), GridPosition::new(0, 0, -1)),
        ] {
            assert!(
                footprint_to_center_size(min, max).is_err(),
                "min {min:?} max {max:?}"
            );
        }
    }

    #[test]
    fn placed_footprint_swaps_spans_away_from_the_origin() {
        // 消费侧入口：存档列带的是 Front 足迹，不是 (Center, GridSize)。
        // 中心离开原点，确认 w/d 互换不是原点上的巧合。
        let front_min = GridPosition::new(5, 0, -12);
        let front_max = GridPosition::new(8, 0, -12);
        let front = placed_footprint(front_min, front_max, Direction::Front).unwrap();
        assert_eq!(front, (front_min, front_max));
        assert_eq!(spans(front.0, front.1), (4, 1));
        let left = placed_footprint(front_min, front_max, Direction::Left).unwrap();
        let (lw, ld) = spans(left.0, left.1);
        assert_eq!((lw, ld), (1, 4));
        assert_eq!(spans(front.0, front.1), (ld, lw));
        // 颠倒的存档列一路拒绝上来，不在消费侧静默变成一个错落点。
        assert!(placed_footprint(front_max, front_min, Direction::Front).is_err());
    }
}
