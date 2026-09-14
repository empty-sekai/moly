//! 靠近家具目的地链（源 `GetLittleFarPosition(position, targetFixture,
//! searchRange)`）。
//!
//! 源的形状：
//! - 环带 = 家具包围盒外扩一圈：`x ∈ [min.x+1−sr, max.x+sr)`、`z` 同形
//!   （半开；`sr = 2` 时含两端恰为 `[min−1, max+1]`），**盒内格剔除**、
//!   **站点网格界外格剔除**；
//! - 环按「出发点到格角的 3D 距离」**稳定升序**（排序键不含半格偏移）；
//! - 逐格探「格角 + 半格偏移 [`APPROACH_HALF_TILE_OFFSET`]」、容差
//!   = 靠近家具的移动容差（服务端下发），**只采样不验路**；
//! - 首个命中即目的地（返回**采样命中位**）；耗尽未命中。
//!
//! 抽签次数：**零**——整条链无随机源，序由距离完全决定。

use super::{Cell, SurfaceProbe};

/// 探测目标的半格偏移（源 `GetLittleFarPosition` 的采样目标在格角上
/// 加 `(0.125, 0, 0.125)`；排序键**不加**）。
pub const APPROACH_HALF_TILE_OFFSET: f32 = 0.125;

/// 源调用点的搜索圈缺省（环带在包围盒外的厚度）。
pub const APPROACH_SEARCH_RANGE: i32 = 2;

/// 环带格：包围盒外扩 `search_range` 的圈（半开区间），盒内与站点
/// 网格界外的格剔除。
///
/// 构造序：`x` 外层、`z` 内层（稳定排序在距离并列时按此序）。
/// `search_range = 1` 时环带恰与盒重合 ⇒ 空环（源同形）。
pub fn approach_ring_cells(
    box_min: Cell,
    box_max: Cell,
    search_range: i32,
    grid_min: Cell,
    grid_max: Cell,
) -> Vec<Cell> {
    let mut cells = Vec::new();
    let x_lo = box_min.0 + 1 - search_range;
    let x_hi = box_max.0 + search_range;
    let z_lo = box_min.1 + 1 - search_range;
    let z_hi = box_max.1 + search_range;
    for x in x_lo..x_hi {
        for z in z_lo..z_hi {
            // 盒内格跳过（含端点）。
            if x >= box_min.0 && x <= box_max.0 && z >= box_min.1 && z <= box_max.1 {
                continue;
            }
            // 站点网格界外格跳过（含端点为界）。
            if x < grid_min.0 || x > grid_max.0 || z < grid_min.1 || z > grid_max.1 {
                continue;
            }
            cells.push((x, z));
        }
    }
    cells
}

/// 靠近家具目的地：环带按到出发点 3D 距离稳定升序，逐格探
/// 「格角 + 半格偏移」（容差 `move_offset`），首个命中返回采样位；
/// 耗尽 `None`（调用方保持原位）。
///
/// `world_of` 是格角世界位映射；偏移由本律加，排序键用**不含偏移**的
/// 格角距离。零抽签、不验路。
pub fn approach_target(
    cells: &[Cell],
    from: [f32; 3],
    world_of: impl Fn(Cell) -> [f32; 3],
    probe: &mut impl SurfaceProbe,
    move_offset: f32,
) -> Option<[f32; 3]> {
    let mut ordered: Vec<Cell> = cells.to_vec();
    ordered.sort_by(|a, b| {
        let da = world_of(*a);
        let db = world_of(*b);
        let dda = distance_sq(from, da);
        let ddb = distance_sq(from, db);
        dda.partial_cmp(&ddb)
            .expect("距离键不含 NaN（格角映射有限）")
    });
    for cell in ordered {
        let corner = world_of(cell);
        let target = [
            corner[0] + APPROACH_HALF_TILE_OFFSET,
            corner[1],
            corner[2] + APPROACH_HALF_TILE_OFFSET,
        ];
        if let Some(hit) = probe.sample(target, move_offset) {
            return Some(hit);
        }
    }
    None
}

fn distance_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探测替身：记录目标与容差，脚本化应答。
    struct ScriptProbe {
        /// 每次采样的应答：Some(dy) = 命中且命中位抬 dy；None = 未命中。
        script: Vec<Option<f32>>,
        targets: Vec<[f32; 3]>,
        tolerances: Vec<f32>,
        path_queries: usize,
    }

    impl ScriptProbe {
        fn new(script: Vec<Option<f32>>) -> Self {
            Self { script, targets: Vec::new(), tolerances: Vec::new(), path_queries: 0 }
        }
    }

    impl SurfaceProbe for ScriptProbe {
        fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
            self.targets.push(target);
            self.tolerances.push(tolerance);
            let step = *self
                .script
                .get(self.targets.len() - 1)
                .expect("采样脚本覆盖全部调用");
            step.map(|dy| [target[0], target[1] + dy, target[2]])
        }

        fn has_path(&mut self, _source: [f32; 3], _target: [f32; 3]) -> bool {
            self.path_queries += 1;
            true
        }
    }

    /// 测试用格角映射：格 (x, z) → 世界 (x·0.25, 0, z·0.25)。
    fn corner(cell: Cell) -> [f32; 3] {
        [cell.0 as f32 * 0.25, 0.0, cell.1 as f32 * 0.25]
    }

    #[test]
    fn ring_band_is_one_cell_thick_around_box() {
        // 盒 (2,2)-(4,4)、sr=2：带 [1,5] 含端 = 5×5 减盒 3×3 = 16 格
        let cells = approach_ring_cells((2, 2), (4, 4), 2, (0, 0), (10, 10));
        assert_eq!(cells.len(), 16);
        for &(x, z) in &cells {
            let in_box = (2..=4).contains(&x) && (2..=4).contains(&z);
            assert!(!in_box, "盒内格不进环：({x},{z})");
            assert!((1..=5).contains(&x) && (1..=5).contains(&z), "环格出带：({x},{z})");
        }
    }

    #[test]
    fn ring_construction_order_is_x_outer_z_inner() {
        // 首格 (1,1)（x 先行），末行 x=5 收尾
        let cells = approach_ring_cells((2, 2), (4, 4), 2, (0, 0), (10, 10));
        assert_eq!(cells[0], (1, 1));
        // x=1 行是整行 5 格（都在盒外）
        assert_eq!(&cells[..5], &[(1, 1), (1, 2), (1, 3), (1, 4), (1, 5)]);
        // x=2 行只剩盒外两端
        assert_eq!(&cells[5..7], &[(2, 1), (2, 5)]);
        assert_eq!(*cells.last().unwrap(), (5, 5));
    }

    #[test]
    fn search_range_one_yields_empty_ring() {
        // sr=1：带 [2,4] 与盒重合 ⇒ 全被盒剔除 ⇒ 空环
        let cells = approach_ring_cells((2, 2), (4, 4), 1, (0, 0), (10, 10));
        assert!(cells.is_empty());
    }

    #[test]
    fn ring_skips_cells_outside_site_grid() {
        // 网格界 (0,0)-(3,3)：带内 x/z ∈ {4,5} 的格全被剔除
        let cells = approach_ring_cells((2, 2), (4, 4), 2, (0, 0), (3, 3));
        let expected: Vec<Cell> = [(1, 1), (1, 2), (1, 3), (2, 1), (3, 1)]
            .into_iter()
            .collect();
        assert_eq!(cells, expected);
    }

    #[test]
    fn grid_bounds_are_inclusive() {
        // 恰在网格界上的格保留（界含端点）
        let on_bound = approach_ring_cells((2, 2), (2, 2), 2, (0, 0), (3, 3));
        assert!(on_bound.contains(&(3, 3)), "界上格保留");
        // 界收紧到 (2,2)：带内的 3 出界 ⇒ 整行整列剔除
        let tight = approach_ring_cells((2, 2), (2, 2), 2, (0, 0), (2, 2));
        assert_eq!(tight, vec![(1, 1), (1, 2), (2, 1)]);
    }

    #[test]
    fn nearest_cell_is_probed_first() {
        // 出发点靠近 (3,0)：首探 (3,0) 的格角 + 半格偏移
        let cells = [(2, 0), (3, 0), (0, 5)];
        let mut probe = ScriptProbe::new(vec![Some(0.0)]);
        let found = approach_target(&cells, [0.7, 0.0, 0.1], corner, &mut probe, 0.2);
        assert_eq!(found, Some([0.875, 0.0, 0.125]));
        assert_eq!(probe.targets.len(), 1);
    }

    #[test]
    fn sort_key_is_corner_distance_without_half_tile_offset() {
        // 格角序与加偏移后的序相反的场景：排序必须跟格角序
        let cells = [(-1, 0), (0, 1)];
        // from=(0.1,0,0)：格角距 (0,1)=0.25 < (-1,0)=0.35 ⇒ 先探 (0,1)；
        // 若误用「格角+偏移」做键则 (-1,0) 先（0.257 < 0.376）
        let mut probe = ScriptProbe::new(vec![Some(0.0)]);
        approach_target(&cells, [0.1, 0.0, 0.0], corner, &mut probe, 0.2);
        assert_eq!(probe.targets[0], [0.125, 0.0, 0.375], "先探格角更近的 (0,1)");
    }

    #[test]
    fn stable_order_on_distance_ties() {
        // 距离并列：保持输入序（稳定排序）
        let cells = [(0, 1), (1, 0), (2, 2)];
        let mut probe = ScriptProbe::new(vec![None; 3]);
        approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2);
        // (0,1) 与 (1,0) 格角距同为 0.25 ⇒ 输入序 (0,1) 先
        assert_eq!(probe.targets[0], [0.125, 0.0, 0.375]);
        assert_eq!(probe.targets[1], [0.375, 0.0, 0.125]);
        // (2,2) 最远最后
        assert_eq!(probe.targets[2], [0.625, 0.0, 0.625]);
    }

    #[test]
    fn probe_target_is_corner_plus_half_tile_offset() {
        // 采样目标 = 格角 + (0.125, 0, 0.125)，y 不动
        let cells = [(2, 3)];
        let mut probe = ScriptProbe::new(vec![Some(0.0)]);
        approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2);
        assert_eq!(probe.targets[0], [0.625, 0.0, 0.875]);
    }

    #[test]
    fn tolerance_is_move_offset() {
        // 容差 = 调用方带入的移动容差（面板值 0.2）
        let cells = [(2, 3)];
        let mut probe = ScriptProbe::new(vec![Some(0.0)]);
        approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2);
        assert_eq!(probe.tolerances, vec![0.2]);
    }

    #[test]
    fn no_path_check_on_this_chain() {
        // 本链只采样不验路（源无 CalculatePath）
        let cells = [(2, 3), (3, 3)];
        let mut probe = ScriptProbe::new(vec![None, Some(0.0)]);
        let found = approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2);
        assert_eq!(found, Some([0.875, 0.0, 0.875]));
        assert_eq!(probe.path_queries, 0, "靠近家具链不验路");
    }

    #[test]
    fn first_hit_returns_sampled_position() {
        // 命中位不是格角也不是探测目标：返回采样命中位
        let cells = [(2, 0)];
        let mut probe = ScriptProbe::new(vec![Some(0.5)]);
        let found = approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2);
        assert_eq!(found, Some([0.625, 0.5, 0.125]));
    }

    #[test]
    fn exhausted_ring_returns_none() {
        // 全 miss：逐格探完、返回未命中
        let cells = [(2, 0), (3, 0), (0, 2)];
        let mut probe = ScriptProbe::new(vec![None; 3]);
        assert_eq!(approach_target(&cells, [0.0, 0.0, 0.0], corner, &mut probe, 0.2), None);
        assert_eq!(probe.targets.len(), 3);
    }

    #[test]
    fn empty_ring_returns_none_without_probing() {
        // 空环：零探测
        let mut probe = ScriptProbe::new(vec![]);
        assert_eq!(approach_target(&[], [0.0, 0.0, 0.0], corner, &mut probe, 0.2), None);
        assert!(probe.targets.is_empty());
    }
}
