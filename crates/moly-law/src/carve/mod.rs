//! 可行走面的烘焙与查询律：站点面三角网 → 格面 → 家具足迹挖洞 →
//! 半径侵蚀 → 小区过滤，以及面上的可走性 / 最近点 / 折线查询。
//!
//! # 烘焙参数从哪来
//!
//! * **站点体素**：`NavMeshField.InitializeNavMesh` 按区域和站点枚举值
//!   分派：CN 6.0 仅值 4（草原）写 0.05；JP 6.7 的值 4..=8（全部
//!   户外站）写 0.05。其余写 0.01。见 [`voxel_size`]。
//! * **agent 参数**：`NavMeshField` 把自己的 agent 类型设为
//!   `MysekaiUtility.GetMysekaiAgentTypeId()` 的返回——按名字
//!   `"MysekaiCharacter"` 在导航设置表里查到的类型。该类型的烘焙参数
//!   （radius 0.24 · height 0.98 · slope 45 · climb 0.1 ·
//!   minRegionArea 2.0）读自工程导航设置。摆放后重烘用的就是这套；
//!   角色组件上 `NavMeshAgent.radius` 的 0.3 是转向半径，两回事。
//! * **换算式**（原生烘焙构造 Recast 配置的指令级取证，四舍五入方式
//!   逐条对应）：半径格数 = `ceil(agentRadius/cs + cs·(−0.01))`（向正
//!   无穷取整）；可行走高度格数 = `floor(agentHeight/ch)`、`ch = cs·0.5`
//!   （向负无穷取整）；小区面积阈 = `2.0/(cs·cs)` 的 f32 除法后向零
//!   截断（0.01 → 20000；0.05 的 f32 平方让 2.0/0.0025000001 截成
//!   799 而非 800——原生同形，照抄）。
//!
//! # 烘焙链（四步，序照原生调度）
//!
//! 1. **栅格化**：面三角按格中心含点判入面。
//! 2. **足迹标 null**：格中心落在阻挡足迹矩形内即杀。
//! 3. **侵蚀**（`rcErodeWalkableArea` 的 2D 转录）：距离场初值 null=0、
//!    可走=255；可行走格的四邻有 null **或出界（虚空）** → 距离 0
//!    ——外边界与障碍同为 0 距离源，外缘同样被侵蚀；两遍 chamfer
//!    （基数向 +2、斜向 +3）；距离 < 2×半径格的可走格杀掉。
//! 4. **小区过滤**（`rcBuildRegions` 小区规则的 2D 形）：4 连通区域格数
//!    低于阈值的整区杀掉。
//!
//! 家具足迹的**参与门**是低高度过滤（`rcFilterWalkableLowHeightSpans`）
//! 的 2D 投影：地面 span 头顶的净空 = 家具底面高（0.25 × center_y），
//! 低于可行走高度（0.98）→ 地面 span 被杀 → 足迹挖洞；底面 ≥ 0.98
//! （如挂在 1.5m/2.0m 墙位的家具）→ 地面照走，不挖。见
//! [`blocking_footprint`]。
//!
//! # 查询链
//!
//! 折线查询转录 `MoveUtility.TryGetCanNavmeshTargetPosition` 的形状：
//! 目标 5.0 内吸附 → 路径计算；任一步失败把目标按 t = 0.9…0.0 拉向
//! 起点逐档重试（每档重新吸附）；全败返回 `None`（源的同族方法返回
//! 起点本身——「原地」的策略留给调用方）。折线整直用「格上 A\* +
//! 贪心视线拉直」，与原生代理「搜索 + 直线路径后处理」同形（后处理
//! 本体在原生层，不可读）。
//!
//! # 具名边界（未建模的东西，与为什么）
//!
//! * **瓦边豁免不建模**：源按瓦分区，连瓦边的小区不杀（真实面积跨瓦
//!   不可估）；本烘焙是单张整场格，没有瓦缝。整场格的外缘在源里恰是
//!   瓦边 ⇒ 若整片面小于小区阈（2 m²）会被这里误杀——真实站点的主
//!   面远大于它，测试面也保持大于它。
//! * **多层面不建模**：家具顶面若矮于 climb（0.1）是「走上去」而不是
//!   「绕开」，矮障碍的顶面会并进可行走面。当前碰撞栅格对每个源三角
//!   使用局部 min/max 高度；高度在 climb 内的贴地三角退出挖洞，避免
//!   把 rug/mat 等低表面误杀。仍未完全等价 Unity 的多层 span 合并。
//! * **墙格足迹链未转录**：墙布局的足迹走另一条派生链
//!   （`CreateWallTileData`），而当前全部墙位摆放的底面高（1.5/2.0）
//!   都在 0.98 之上、本就不挖——不转录无损。
//! * **源物理几何的单层投影**：原版收集 PhysicsColliders。宿主走
//!   `bake_colliders`，保留真实节点变换、源三角轮廓和每三角高度范围；
//!   旧的 `BakeInput` 矩形接口仅供独立律的兼容测试。本地投影不处理
//!   台阶顶面或多层连通，凸 MeshCollider 仍是源三角 raster 近似，不等价
//!   于 Unity 的三维烘焙与 convex cooking。
//! * **重烘触发沿**：执行侧监听放稳足迹变化，每次变化重烘。

mod contour;
mod funnel;
mod grid;
mod polymesh;
mod query;
mod region;

use crate::fixture::position::{layout_type, TILE_SIZE};
use crate::fixture::GridPosition;
pub use query::SNAP_MAX_DISTANCE;

/// 烘焙 agent 半径（米）：`"MysekaiCharacter"` agent 类型的表值。
/// f32 表示即 0.23999999463…，与本常量的 f32 位形一致。
pub const AGENT_RADIUS: f32 = 0.24;

/// 烘焙 agent 高度（米）：同表值（可行走净空阈）。
pub const AGENT_HEIGHT: f32 = 0.98;

/// 烘焙 agent 可攀越高度（米）：低于此高度的贴地碰撞面会作为新的
/// walkable span，而不是把原地面挖掉。Unity 的 Recast 构造使用同一
/// MysekaiCharacter agent 的 climb 值；这里不能把所有 PhysicsCollider
/// 都当作垂直障碍。
pub const AGENT_CLIMB: f32 = 0.1;

/// 小区面积阈（平方米）：同表值，换算成格数见 [`min_region_spans`]。
pub const MIN_REGION_AREA: f32 = 2.0;

/// 轮廓简化的最大偏差（格）：原生构造 Recast 配置时写入的常量，
/// f32 位形 1.2999999523162842。它不随站点变化。
pub const MAX_SIMPLIFICATION_ERROR: f32 = 1.3;

/// 站点枚举名 → 值（`MysekaiSiteType` 的九值闭集）。
pub fn site_type_value(name: &str) -> Option<u32> {
    Some(match name {
        "home_site" => 0,
        "first_floor" => 1,
        "second_floor" => 2,
        "third_floor" => 3,
        "grassland" => 4,
        "shore" => 5,
        "flower_garden" => 6,
        "memorial_place" => 7,
        "festival_garden" => 8,
        _ => return None,
    })
}

/// Regional source implementation of `NavMeshField.InitializeNavMesh`.
/// This is selected from the loaded snapshot identity, never its directory name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavMeshRegion {
    Cn,
    Jp,
}

/// Source voxel size in metres. CN 6.0 selects 0.05 only for type 4;
/// JP 6.7 (RVA 0x58313D8) selects it when unsigned `siteType - 4 < 5`.
/// Both source implementations select 0.01 for the other values.
pub fn voxel_size(region: NavMeshRegion, site_type_value: u32) -> f32 {
    let outdoor = match region {
        NavMeshRegion::Cn => site_type_value == 4,
        NavMeshRegion::Jp => (4..=8).contains(&site_type_value),
    };
    if outdoor {
        0.05
    } else {
        0.01
    }
}

/// 侵蚀半径格数：`ceil(agentRadius/cs + cs·(−0.01))`（0.01 → 24 格、
/// 0.05 → 5 格；负项让恰好的整数半径不被再进一）。
pub fn radius_cells(voxel: f32) -> u32 {
    (AGENT_RADIUS / voxel + voxel * (-0.01)).ceil() as u32
}

/// 小区面积阈（格数）：`2.0/(cs·cs)` 的 f32 除法向零截断。
/// 0.01 → 20000；0.05 → 799（f32 的 cs² 略大于 0.0025）。
pub fn min_region_spans(voxel: f32) -> usize {
    (MIN_REGION_AREA / (voxel * voxel)) as usize
}

/// 可行走净空阈（米，格量化后）：`floor(agentHeight/ch)·ch`，`ch = cs·0.5`。
/// 0.01 → 0.98；0.05 → 0.975。
fn walkable_height_world(voxel: f32) -> f32 {
    let ch = voxel * 0.5;
    (AGENT_HEIGHT / ch).floor() * ch
}

/// 一条阻挡足迹（世界系 xz 矩形，半开 [min, max)）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obstacle {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// A source collider's projected geometry in the single-surface host. The
/// vertical interval is retained so elevated fixtures do not block the floor.
/// This is a conservative projection, not Unity's three-dimensional bake.
#[derive(Debug, Clone)]
pub struct ColliderPolygon {
    /// One source triangle (or primitive perimeter) projected to XZ.  The
    /// runtime intentionally receives triangles for convex meshes too: this
    /// preserves local vertical spans and avoids a polygon-wide min/max-Y
    /// false obstacle. It is still a 2D approximation of Unity cooked hulls.
    pub vertices: Vec<[f32; 2]>,
    /// Vertical bounds of this projected source primitive in world Y.
    pub min_y: f32,
    pub max_y: f32,
}

/// 摆放行是否参与挖洞，参与则给它的世界足迹矩形。
///
/// 参与门（旧布局足迹兼容接口）：地板类布局（布局含地板位）且
/// 底面高 0.25 × `center_y` 低于可行走净空阈 → 挖；墙位（1.5/2.0）、
/// 高台位（1.0）不挖。源 PhysicsCollider 的实际烘焙走
/// [`WalkField::bake_colliders`]，低于 [`AGENT_CLIMB`] 的贴地三角由
/// span 规则保留。格角到世界角的换算与
/// 落位律同式：min 角 = min × 0.25，max 角 = max × 0.25 + 0.25。
pub fn blocking_footprint(
    min: GridPosition,
    max: GridPosition,
    center_y: i8,
    layout: u8,
    voxel: f32,
) -> Option<Obstacle> {
    if layout & layout_type::FLOOR == 0 {
        return None;
    }
    if TILE_SIZE * center_y as f32 >= walkable_height_world(voxel) {
        return None;
    }
    Some(Obstacle {
        min: [min.x as f32 * TILE_SIZE, min.z as f32 * TILE_SIZE],
        max: [
            max.x as f32 * TILE_SIZE + TILE_SIZE,
            max.z as f32 * TILE_SIZE + TILE_SIZE,
        ],
    })
}

/// 一次烘焙的输入：面三角网（世界 xz）、阻挡足迹、体素。
pub struct BakeInput {
    pub tris: Vec<[[f32; 2]; 3]>,
    pub obstacles: Vec<Obstacle>,
    pub voxel: f32,
}

/// 烘焙账目（每阶段的格数，重烘前后可对账）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BakeCounts {
    /// 场格总数（cols × rows）。
    pub cells: usize,
    /// 栅格化入面的格数。
    pub face: usize,
    /// 足迹标 null 杀掉的格数。
    pub obstacle_nulled: usize,
    /// 侵蚀杀掉的格数（含外缘）。
    pub erosion_nulled: usize,
    /// 小区过滤杀掉的格数。
    pub region_nulled: usize,
    /// 最终可行走格数。
    pub walkable: usize,
    /// 单调分区产出的区数（有效区号是 `1..=regions`）。
    pub regions: u32,
    /// 简化后的轮廓条数。
    pub contours: usize,
    /// 全部轮廓的顶点总数（简化后）。
    pub contour_verts: usize,
    /// 导航多边形网的单元数（三角形；凸合并未实现，见 polymesh 模块注释）。
    pub polygons: usize,
}

/// 烘好的可行走场：格面 + 分区 + 轮廓 + 账目。查询全部走它。
pub struct WalkField {
    grid: grid::Grid,
    regions: region::Regions,
    #[allow(dead_code)] // 轮廓建完多边形网后只出账目；分瓦时它还要被读。
    contours: Vec<contour::Contour>,
    polys: polymesh::PolyMesh,
    counts: BakeCounts,
}

impl WalkField {
    /// 烘焙：栅格化 → 足迹标 null → 侵蚀 → 小区过滤 → 单调分区 → 轮廓，
    /// 账目逐段记。
    pub fn bake(input: &BakeInput) -> WalkField {
        let mut grid = grid::rasterize(&input.tris, input.voxel);
        let face = grid.walkable.iter().filter(|w| **w).count();
        let obstacle_nulled = grid::mark_obstacles(&mut grid, &input.obstacles);
        Self::finish_bake(grid, face, obstacle_nulled, input.voxel)
    }

    /// Bake actual collider footprints using the existing 2D host. Geometry
    /// comes from PhysicsCollider inputs rather than logical occupancy boxes.
    pub fn bake_colliders(
        surface: &[[[f32; 3]; 3]],
        colliders: &[ColliderPolygon],
        voxel: f32,
    ) -> WalkField {
        let tris: Vec<_> = surface
            .iter()
            .map(|tri| tri.map(|p| [p[0], p[2]]))
            .collect();
        let mut grid = grid::rasterize(&tris, voxel);
        let face = grid.walkable.iter().filter(|w| **w).count();
        let heights = grid::surface_heights(&grid, surface);
        let obstacle_nulled = grid::mark_collider_polygons(&mut grid, colliders, &heights);
        Self::finish_bake(grid, face, obstacle_nulled, voxel)
    }

    fn finish_bake(
        mut grid: grid::Grid,
        face: usize,
        obstacle_nulled: usize,
        voxel: f32,
    ) -> WalkField {
        let erosion_nulled = grid::erode(&mut grid, radius_cells(voxel));
        let region_nulled = grid::filter_regions(&mut grid, min_region_spans(voxel));
        let walkable = grid.walkable.iter().filter(|w| **w).count();
        let regions = region::build_monotone(&grid);
        let contours = contour::build_contours(&grid, &regions, MAX_SIMPLIFICATION_ERROR);
        let polys = polymesh::build(&contours);
        WalkField {
            counts: BakeCounts {
                cells: grid.cols * grid.rows,
                face,
                obstacle_nulled,
                erosion_nulled,
                region_nulled,
                walkable,
                regions: regions.max.saturating_sub(1),
                contours: contours.len(),
                contour_verts: contours.iter().map(|c| c.verts.len()).sum(),
                polygons: polys.polygon_count(),
            },
            regions,
            contours,
            polys,
            grid,
        }
    }

    /// 点是否可行走（所在格为可行走格；出界按不可走）。
    pub fn walkable_at(&self, p: [f32; 2]) -> bool {
        query::walkable_at(&self.grid, p)
    }

    /// 最近可走点（吸附上限 `max_dist`，超限返回 `None`）。
    pub fn nearest_walkable(&self, p: [f32; 2], max_dist: Option<f32>) -> Option<[f32; 2]> {
        query::nearest_walkable(&self.grid, p, max_dist)
    }

    /// 沿面折线：转录 `TryGetCanNavmeshTargetPosition` 的查询链
    /// （吸附 + 拉回梯度），见模块注释。全败返回 `None`。
    pub fn path(&self, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
        query::path(&self.grid, &self.polys, &self.regions, start, goal)
    }

    /// 严格完整路线：不拉回目标，起终点必须已在可走场上。
    pub fn path_exact(&self, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
        query::path_exact(&self.grid, &self.polys, &self.regions, start, goal)
    }

    /// 整条位移线段的超覆盖判定（同路线拉直，不只检查终点）。
    pub fn segment_walkable(&self, start: [f32; 2], goal: [f32; 2]) -> bool {
        query::visible(&self.grid, start, goal)
    }

    /// 沿请求位移走到第一处阻挡前；不把被挡终点吸到家具另一边。
    ///
    /// 到达障碍边缘后，尝试把剩余位移投影到两个轴向切向分量。Unity
    /// 的 `NavMeshAgent.Move` 会沿 navmesh 边界继续走；只做前缀二分会把
    /// 任何斜向擦边都错误地停死。每一步仍经过同一整段可走性裁决，
    /// 因而不会跨越薄障碍或穿过洞。
    pub fn constrain_move(&self, start: [f32; 2], goal: [f32; 2]) -> [f32; 2] {
        if !start.into_iter().chain(goal).all(f32::is_finite) || !self.walkable_at(start) {
            return start;
        }
        if self.segment_walkable(start, goal) {
            return goal;
        }
        // 可达前缀是单调区间。每次候选都验整段，任意 dt 下不可跨薄障碍。
        let mut lo = 0.0;
        let mut hi = 1.0;
        let mut accepted = start;
        for _ in 0..24 {
            let t = (lo + hi) * 0.5;
            let candidate = [
                start[0] + (goal[0] - start[0]) * t,
                start[1] + (goal[1] - start[1]) * t,
            ];
            if self.segment_walkable(start, candidate) {
                lo = t;
                accepted = candidate;
            } else {
                hi = t;
            }
        }
        // Resolve the remaining displacement against the two coordinate
        // tangents.  The order is distance based so a diagonal input follows
        // the side that makes the most progress toward its actual target.
        let mut current = accepted;
        for _ in 0..4 {
            let dx = goal[0] - current[0];
            let dz = goal[1] - current[1];
            if dx.abs() <= self.grid.voxel * 0.25 && dz.abs() <= self.grid.voxel * 0.25 {
                break;
            }
            let candidates = [[goal[0], current[1]], [current[0], goal[1]]];
            let mut best = current;
            let mut best_d = (goal[0] - current[0]).powi(2) + (goal[1] - current[1]).powi(2);
            for candidate in candidates {
                if !self.segment_walkable(current, candidate) {
                    continue;
                }
                let d = (goal[0] - candidate[0]).powi(2) + (goal[1] - candidate[1]).powi(2);
                if d < best_d {
                    best = candidate;
                    best_d = d;
                }
            }
            if best == current {
                break;
            }
            current = best;
        }
        current
    }

    /// 烘焙账目。
    pub fn counts(&self) -> &BakeCounts {
        &self.counts
    }

    /// 体素（米）。
    pub fn voxel(&self) -> f32 {
        self.grid.voxel
    }

    /// 场格的世界包围盒（min 角 / max 远角）。
    pub fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        (
            self.grid.origin,
            [
                self.grid.origin[0] + self.grid.cols as f32 * self.grid.voxel,
                self.grid.origin[1] + self.grid.rows as f32 * self.grid.voxel,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_triangle_does_not_turn_into_its_occupancy_rectangle() {
        let surface = vec![
            [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
            [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
        ];
        let collider = ColliderPolygon {
            vertices: vec![[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
            min_y: 0.0,
            max_y: 1.0,
        };
        let field = WalkField::bake_colliders(&surface, &[collider], 0.05);
        assert!(!field.walkable_at([0.5, 0.5]));
        assert!(field.walkable_at([1.75, 1.75]));
        assert!(!field.segment_walkable([-1.0, 0.5], [3.0, 0.5]));
    }

    #[test]
    fn vertical_wall_projection_does_not_disappear() {
        let surface = vec![
            [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
            [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
        ];
        let wall = ColliderPolygon {
            vertices: vec![[0.0, -2.0], [0.0, 2.0]],
            min_y: 0.0,
            max_y: 1.5,
        };
        let field = WalkField::bake_colliders(&surface, &[wall], 0.05);
        assert!(!field.segment_walkable([-1.0, 0.0], [1.0, 0.0]));
        assert!(field.walkable_at([-1.0, 0.0]));
    }

    #[test]
    fn elevated_collider_preserves_head_clearance_and_surface_height() {
        let surface = vec![
            [[-4.0, 2.0, -4.0], [4.0, 2.0, -4.0], [4.0, 2.0, 4.0]],
            [[-4.0, 2.0, -4.0], [4.0, 2.0, 4.0], [-4.0, 2.0, 4.0]],
        ];
        let mut collider = ColliderPolygon {
            vertices: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            min_y: 3.0,
            max_y: 4.0,
        };
        assert!(
            WalkField::bake_colliders(&surface, &[collider.clone()], 0.05).walkable_at([0.0, 0.0])
        );
        collider.min_y = 2.5;
        assert!(!WalkField::bake_colliders(&surface, &[collider], 0.05).walkable_at([0.0, 0.0]));
    }

    #[test]
    fn thin_ground_collider_is_a_walkable_span_not_a_hole() {
        let surface = vec![
            [[-4.0, 0.0, -4.0], [4.0, 0.0, -4.0], [4.0, 0.0, 4.0]],
            [[-4.0, 0.0, -4.0], [4.0, 0.0, 4.0], [-4.0, 0.0, 4.0]],
        ];
        let rug = ColliderPolygon {
            vertices: vec![[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
            min_y: 0.0,
            max_y: 0.05,
        };
        let field = WalkField::bake_colliders(&surface, &[rug], 0.05);
        assert!(field.walkable_at([0.5, 0.5]));
        let step = ColliderPolygon {
            vertices: vec![[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
            min_y: 0.0,
            max_y: 0.2,
        };
        let field = WalkField::bake_colliders(&surface, &[step], 0.05);
        assert!(!field.walkable_at([0.5, 0.5]));
    }

    #[test]
    fn blocked_diagonal_can_slide_along_a_clear_axis() {
        let face = quad([-4.0, -4.0], [4.0, 4.0]);
        let obstacle = Obstacle {
            min: [0.0, -1.0],
            max: [1.0, 1.0],
        };
        let field = bake(face, vec![obstacle], 0.05);
        let start = [-1.0, -1.0];
        let goal = [2.0, 2.0];
        let accepted = field.constrain_move(start, goal);
        // The two-axis projection follows the lower/side tangent and can
        // complete the move around the rectangle in this single frame.
        assert!((accepted[0] - goal[0]).abs() < 0.1);
        assert!((accepted[1] - goal[1]).abs() < 0.1);
    }

    /// 轴对齐四边形 → 两三角。
    fn quad(min: [f32; 2], max: [f32; 2]) -> Vec<[[f32; 2]; 3]> {
        vec![
            [[min[0], min[1]], [max[0], min[1]], [max[0], max[1]]],
            [[min[0], min[1]], [max[0], max[1]], [min[0], max[1]]],
        ]
    }

    /// 线段与闭矩形是否相交（slab 法；测试侧独立几何，不复用被测的
    /// 视线实现）。
    fn seg_hits_rect(a: [f32; 2], b: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for axis in 0..2 {
            let d = b[axis] - a[axis];
            if d.abs() <= f32::EPSILON {
                if a[axis] < min[axis] || a[axis] > max[axis] {
                    return false;
                }
            } else {
                let mut ta = (min[axis] - a[axis]) / d;
                let mut tb = (max[axis] - a[axis]) / d;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }

    fn bake(tris: Vec<[[f32; 2]; 3]>, obstacles: Vec<Obstacle>, voxel: f32) -> WalkField {
        WalkField::bake(&BakeInput {
            tris,
            obstacles,
            voxel,
        })
    }

    // —— 站点与换算律 ——

    #[test]
    fn site_names_map_to_enum_values() {
        assert_eq!(site_type_value("home_site"), Some(0));
        assert_eq!(site_type_value("grassland"), Some(4));
        assert_eq!(site_type_value("festival_garden"), Some(8));
        assert_eq!(site_type_value("unknown"), None);
    }

    #[test]
    fn cn_voxel_branches_match_initialize_navmesh() {
        for value in [0u32, 1, 2, 3, 5, 6, 7, 8, 9, u32::MAX] {
            assert_eq!(voxel_size(NavMeshRegion::Cn, value), 0.01, "value {value}");
        }
        assert_eq!(voxel_size(NavMeshRegion::Cn, 4), 0.05);
    }

    #[test]
    fn jp_voxel_branches_match_initialize_navmesh() {
        for value in 4..=8 {
            assert_eq!(voxel_size(NavMeshRegion::Jp, value), 0.05, "value {value}");
        }
        for value in [0u32, 1, 2, 3, 9, u32::MAX] {
            assert_eq!(voxel_size(NavMeshRegion::Jp, value), 0.01, "value {value}");
        }
    }

    #[test]
    fn radius_cells_match_native_conversion() {
        // 0.01 → ceil(24 − 0.0001) = 24；0.05 → ceil(4.8 − 0.0005) = 5。
        // 若半径误取角色组件的 0.3：0.05 会得 6、0.01 会得 30——红。
        assert_eq!(radius_cells(0.01), 24);
        assert_eq!(radius_cells(0.05), 5);
    }

    #[test]
    fn min_region_spans_match_f32_truncation() {
        assert_eq!(min_region_spans(0.01), 20000);
        // f32 的 0.05² 略大 → 799 而非 800（原生同形）。
        assert_eq!(min_region_spans(0.05), 799);
    }

    // —— 阻挡带（足迹参与门）——

    #[test]
    fn low_floor_footprints_block() {
        let voxel = 0.05;
        let min = GridPosition::new(0, 0, 0);
        let max = GridPosition::new(1, 0, 1);
        // 地板位 + 底面 0：挖。
        let ob = blocking_footprint(min, max, 0, layout_type::FLOOR, voxel).unwrap();
        // 格角换算：min 角 0，max 远角 1×0.25+0.25 = 0.5。
        assert_eq!(ob.min, [0.0, 0.0]);
        assert_eq!(ob.max, [0.5, 0.5]);
        // 高台位 center_y=3（底 0.75 < 0.975）：仍挖。
        assert!(blocking_footprint(min, max, 3, layout_type::FLOOR, voxel).is_some());
        // 墙位底面（6/8 → 1.5/2.0）与高台位 4（底 1.0 ≥ 阈）：不挖。
        assert!(blocking_footprint(min, max, 4, layout_type::FLOOR, voxel).is_none());
        assert!(blocking_footprint(min, max, 6, layout_type::WALL_FRONT, voxel).is_none());
        // 平铺类（RUG/ROAD）没有布局足迹；源碰撞烘焙的低表面规则在
        // `mark_collider_polygons` 中按实际几何高度处理。
        assert!(blocking_footprint(min, max, 0, layout_type::RUG, voxel).is_none());
        assert!(blocking_footprint(min, max, 0, layout_type::ROAD, voxel).is_none());
        // 地板族复合位（FIELD 含地板位）：挖。
        assert!(blocking_footprint(min, max, 0, layout_type::FIELD, voxel).is_some());
    }

    #[test]
    fn blocking_threshold_quantizes_per_voxel() {
        // 0.01 体素的高阈是 0.98：底 1.0 不挖；0.05 体素是 0.975：
        // 同一座 1.0 也不挖。center_y=3 的 0.75 两个体素下都挖。
        for voxel in [0.01, 0.05] {
            let min = GridPosition::new(0, 0, 0);
            let max = GridPosition::new(0, 0, 0);
            assert!(blocking_footprint(min, max, 4, layout_type::FLOOR, voxel).is_none());
            assert!(blocking_footprint(min, max, 3, layout_type::FLOOR, voxel).is_some());
        }
    }

    // —— 成对判据一：足迹挖洞（内不可走 ∧ 外可走 ∧ 反向臂）——

    #[test]
    fn footprint_carves_inside_and_spare_outside() {
        // 体素 0.01（居住类参数）让两臂的相位余量 ≫ 格尺度。
        let face = quad([0.0, 0.0], [10.0, 10.0]);
        let wall = Obstacle {
            min: [4.0, 4.0],
            max: [6.0, 6.0],
        };
        let carved = bake(face.clone(), vec![wall], 0.01);
        // 臂 A：足迹内的点不可走。
        assert!(!carved.walkable_at([5.0, 5.0]));
        // 臂 B：足迹外（越过侵蚀带）的点可走。若把整片面挖没了，
        // 这条臂红——只有臂 A 会让那种实现变绿。
        assert!(carved.walkable_at([8.0, 8.0]));
        // 反向臂：不挖时同一点可走 ⇒ 臂 A 测的是挖洞，不是别的。
        let clean = bake(face, vec![], 0.01);
        assert!(clean.walkable_at([5.0, 5.0]));
        // 账目：2m×2m 的足迹矩形在 0.01 体素下按格中心入判，恰
        // 200×200 = 40000 格被杀。
        assert_eq!(carved.counts().obstacle_nulled, 40000);
    }

    // —— 成对判据二：半径内缩可量测 ——

    #[test]
    fn agent_radius_inflates_the_carve() {
        let face = quad([0.0, 0.0], [10.0, 10.0]);
        let wall = Obstacle {
            min: [4.0, 4.0],
            max: [6.0, 6.0],
        };
        let field = bake(face, vec![wall], 0.01);
        // 紧贴障碍 0.2m（矩形边 x=6、边中点 z=5）：不可走。
        // 半径不参与（挖原始足迹）时这一点可走——臂红。
        assert!(!field.walkable_at([6.2, 5.0]));
        // 0.4m：可走（工单口径的一对）。
        assert!(field.walkable_at([6.4, 5.0]));
        // 0.27m：可走。这一臂把内缩量钉死在 (0.2, 0.27) 内——
        // 半径误取 0.3（角色组件值）时 0.27 会被误杀，红。
        assert!(field.walkable_at([6.27, 5.0]));
    }

    // —— 成对判据三：外边界同样侵蚀 ——

    #[test]
    fn outer_boundary_erodes_too() {
        // 无障碍：外缘的侵蚀只能来自虚空边界。
        let field = bake(quad([0.0, 0.0], [10.0, 10.0]), vec![], 0.01);
        assert!(!field.walkable_at([0.1, 5.0]));
        assert!(field.walkable_at([0.5, 5.0]));
    }

    // —— 成对判据四：小区过滤（小岛死 ∧ 大岛活）——

    #[test]
    fn small_islands_die_large_survive() {
        // 0.05 体素：阈 799 格（≈2 m²）。
        // 岛 B 1×1m：20×20 格，侵蚀后剩 10×10 = 100 格 < 799 → 整区杀。
        // 岛 C 4×4m：80×80 格，侵蚀后剩 70×70 = 4900 格 > 799 → 活。
        // 主面 10×10m：40000 格，侵蚀后 36100 格 → 活。
        let mut tris = quad([0.0, 0.0], [10.0, 10.0]);
        tris.extend(quad([20.0, 20.0], [21.0, 21.0]));
        tris.extend(quad([30.0, 30.0], [34.0, 34.0]));
        let field = bake(tris, vec![], 0.05);
        // 臂 A：小岛上的点不可走（小区过滤杀的——侵蚀只吃它外缘 5 格，
        // 剩 10×10 是过滤整区杀掉的）。
        assert!(!field.walkable_at([20.5, 20.5]));
        // 臂 B：大岛中心可走。
        assert!(field.walkable_at([32.0, 32.0]));
        // 主面可走。
        assert!(field.walkable_at([5.0, 5.0]));
        // 账目：恰 100 格（小岛侵蚀后的全部）被小区过滤杀掉。
        assert_eq!(field.counts().region_nulled, 100);
    }

    // —— 烘焙账目可复算 ——

    #[test]
    fn bake_counts_recompute_exactly() {
        // 10×10m 面、0.05 体素：200×200 = 40000 格入面；侵蚀杀 5 圈
        // （距离 < 2×5 半格单位）剩 190×190 = 36100；单区 36100 > 799
        // → 小区过滤 0。逐格可复算。
        let field = bake(quad([0.0, 0.0], [10.0, 10.0]), vec![], 0.05);
        assert_eq!(field.counts().cells, 40000);
        assert_eq!(field.counts().face, 40000);
        assert_eq!(field.counts().erosion_nulled, 3900);
        assert_eq!(field.counts().region_nulled, 0);
        assert_eq!(field.counts().walkable, 36100);
        // 单调分区：侵蚀后是一整块实心矩形，每行恰一个跨段，北邻唯一且
        // 「本行并进该区的格数」与「该区被并的格数」相等 ⇒ 每行都并进上
        // 一行那个区，全场恰 1 区。跨段没并上会得到 190。
        assert_eq!(field.counts().regions, 1);
        // 轮廓：那一块实心矩形只有一条外边界，简化后恰 4 个角。
        // 全环邻区号都是 0（外墙）⇒ 走路径 B，先取字典序最小/最大的两个
        // 对角点；另两个角离那条对角线约 95 格，远超 1.3 格的偏差阈 ⇒ 各被
        // 插入一次；之后每条边都精确落在直线上 ⇒ 收在 4 个顶点。
        assert_eq!(field.counts().contours, 1);
        assert_eq!(field.counts().contour_verts, 4);
        let (min, max) = field.bounds();
        assert_eq!(min, [0.0, 0.0]);
        assert_eq!(max, [10.0, 10.0]);
    }

    // —— 成对判据五：折线绕洞（正向不穿 ∧ 反向臂确实穿）——

    #[test]
    fn path_detours_the_carved_hole_and_reverse_crosses() {
        // 竖墙 x[5,7] z[2,10] 把 12×12 的面左右分开，上下各留 2m 通道
        // （侵蚀后仍 > 1.5m）。
        let face = quad([0.0, 0.0], [12.0, 12.0]);
        let wall = Obstacle {
            min: [5.0, 2.0],
            max: [7.0, 10.0],
        };
        let start = [2.0, 6.0];
        let goal = [10.0, 6.0];
        let carved = bake(face.clone(), vec![wall], 0.05);
        let path = carved.path(start, goal).expect("绕洞折线应存在");
        // 正向臂：每一段都不与已挖的洞（足迹矩形）相交；路点全可走。
        assert!(path.len() >= 2);
        for window in path.windows(2) {
            assert!(
                !seg_hits_rect(window[0], window[1], wall.min, wall.max),
                "段 {:?}→{:?} 穿过洞",
                window[0],
                window[1]
            );
        }
        // 路点落在可走集的**闭包**上。
        //
        // ⚠ 这里不能用 `walkable_at`：搜索从格面搬到多边形网之后，拐点是
        // 轮廓顶点，而轮廓顶点是**格角**，恰好落在最后一个可走格与第一个
        // 死格的公共边上——按 `cell_of` 的半开约定归进死格，`walkable_at`
        // 因此对一个完全正确的拐点报假。
        //
        // 而这正是侵蚀的用意：面已经按 agent 半径内缩过，agent 的中心本来
        // 就可以走到这条边界上。所以判据问的应是「偏离可走集不超过一格」，
        // 不是「格心可走」。真正的安全性质由上面那条「每段都不穿洞」承担。
        for p in &path {
            assert!(
                carved.nearest_walkable(*p, Some(0.05)).is_some(),
                "路点 {:?} 离可走集超过一格",
                p
            );
        }
        // 反向臂：不挖时同一对起终点的路径确实穿过那个位置
        // ⇒ 正向臂量的是挖洞，不是「本来就得绕」。
        let clean = bake(face, vec![], 0.05);
        let direct = clean.path(start, goal).expect("无洞直达应存在");
        assert_eq!(direct.len(), 2, "无洞时应整直成直达段");
        assert!(seg_hits_rect(direct[0], direct[1], wall.min, wall.max));
        assert_eq!(direct[0], start);
        assert_eq!(direct[1], goal);
    }

    // —— 查询链：吸附、拉回梯度、不可达 ——

    #[test]
    fn path_snaps_endpoints_onto_the_face() {
        let field = bake(quad([0.0, 0.0], [10.0, 10.0]), vec![], 0.05);
        // 起点在面外 0.05（侵蚀带内）：5.0 吸附内拉回首格。
        let path = field.path([-0.05, 5.0], [5.0, 5.0]).expect("吸附后应可走");
        assert!(
            path[0][0] >= 0.2 && path[0][0] <= 0.35,
            "首点 {:?}",
            path[0]
        );
        assert!(field.walkable_at(path[0]));
    }

    #[test]
    fn unreachable_goal_pulls_back_along_the_ladder() {
        // 两个不连通的面：目标在大岛上。直达失败后目标按 0.9…0.0
        // 拉向起点，落在小岛上的档位出折线——末端不是原目标。
        let mut tris = quad([0.0, 0.0], [10.0, 10.0]);
        tris.extend(quad([30.0, 30.0], [34.0, 34.0]));
        let field = bake(tris, vec![], 0.05);
        let path = field
            .path([5.0, 5.0], [32.0, 32.0])
            .expect("拉回梯度应找到可达档");
        assert_ne!(*path.last().unwrap(), [32.0, 32.0]);
        assert!(field.walkable_at(*path.last().unwrap()));
        // 起点自身吸不上（离任何面都远超 5.0）：None——梯度的锚点
        // 不在面上时，全梯度都失败。
        assert!(field.path([200.0, 200.0], [5.0, 5.0]).is_none());
    }

    #[test]
    fn same_cell_path_is_two_points() {
        let field = bake(quad([0.0, 0.0], [10.0, 10.0]), vec![], 0.05);
        let path = field.path([5.0, 5.0], [5.02, 5.03]).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], [5.0, 5.0]);
        assert_eq!(path[1], [5.02, 5.03]);
    }
}
