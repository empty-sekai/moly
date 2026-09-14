//! 游走目的地链（源 `GetRandomPosition` → `GetRandomNotPlacedFloorPosition`
//! → `SearchMovableFloorPositionInTargetRange`）。
//!
//! 源的形状：
//! - 距离档按站点取（`NPCRandomMoveRange` 的 `MinDistance`/
//!   `MaxDistance`，服务端下发）；
//! - 候选 = 可行格**减去**玩家与 NPC 的占格（调用方带入差集，本律
//!   不重造占格簿）；
//! - 环滤 `dx²+dz² ∈ [min², max²]`（**双端含**，`max²` 与整型上界取
//!   小）；
//! - 环做一次均匀置换（源 `OrderBy(Guid.NewGuid())`——每格一枚新键，
//!   全部被排序消费），取**前 5**；
//! - 逐格双探：先表面采样（容差 = 格距 [`TILE_SCALE`]），命中再验
//!   「出发点到采样点有路」；首个双过即目的地；
//! - 采样未命中**不验路**（源先 `SamplePosition` 后 `CalculatePath`，
//!   短路序）；
//! - 出发点 = 出发格的格角世界位（不是角色脚点）；
//! - 5 格全 miss 或环空 ⇒ 未命中，调用方保持原位。
//!
//! 抽签次数：恰一次置换调用，键数 = 环大小（空环零键）；无其他随机源。

use super::{Cell, SurfaceProbe};

/// 格距（源 `TILE_SCALE`，格坐标到世界坐标的换算单位）：游走采样的
/// 表面容差。
///
/// 双源：同族常量 `TILE_SIZE` 的 y/z 就地为 0.25；同仓换算族
/// （格 ↔ 场地）既有实现取同一值。
pub const TILE_SCALE: f32 = 0.25;

/// 置换后取前几格逐探（源 `Take(5)`）。
pub const WANDER_TAKE: usize = 5;

/// 环滤谓词：`dx² + dz²` 落在 `[min², max²]`（双端含）。
///
/// `max²` 与整型上界取小（源 `Math.Min(max*max, int.MaxValue)`）；
/// 距离只在 xz 平面量，格坐标不含高度轴。
pub fn ring_filter(origin: Cell, candidate: Cell, min_cells: i32, max_cells: i32) -> bool {
    let dx = candidate.0 as i64 - origin.0 as i64;
    let dz = candidate.1 as i64 - origin.1 as i64;
    let distance_sq = dx * dx + dz * dz;
    let min_sq = (min_cells as i64) * (min_cells as i64);
    let max_sq = ((max_cells as i64) * (max_cells as i64)).min(i32::MAX as i64);
    min_sq <= distance_sq && distance_sq <= max_sq
}

/// 均匀置换（源 `OrderBy(Guid.NewGuid())`）：对环取一个等分布的
/// 探测序。键数 = `len`（每格一枚新键、全部被消费；`len = 0` 零键）。
pub trait Permute {
    /// 返回 `0..len` 的一个置换：下标 0 先探。
    fn permutation(&mut self, len: usize) -> Vec<usize>;
}

/// 游走目的地：环滤 → 均匀置换取前 [`WANDER_TAKE`] → 逐格双探
/// （采样过才验路），首个双过返回**采样命中位**；否则 `None`（调用
/// 方保持原位）。
///
/// `world_of` 是格角世界位映射（格 → 世界，不含半格偏移；游走链的
/// 探测目标就是格角本身）。`origin` 的格角作为验路出发点。
pub fn wander_target(
    origin: Cell,
    min_cells: i32,
    max_cells: i32,
    eligible: &[Cell],
    world_of: impl Fn(Cell) -> [f32; 3],
    rand: &mut impl Permute,
    probe: &mut impl SurfaceProbe,
) -> Option<[f32; 3]> {
    let ring: Vec<Cell> = eligible
        .iter()
        .copied()
        .filter(|&candidate| ring_filter(origin, candidate, min_cells, max_cells))
        .collect();
    // 恰一次置换：键数 = 环大小（空环零键、零探测）。
    let order = rand.permutation(ring.len());
    let source = world_of(origin);
    for &index in order.iter().take(WANDER_TAKE) {
        // 置换域就是 0..环大小；越界即替身违约。
        let &cell = ring
            .get(index)
            .expect("置换下标必须落在环内");
        let target = world_of(cell);
        // 先采样（容差 = 格距），未命中不验路（源短路序）。
        let Some(hit) = probe.sample(target, TILE_SCALE) else {
            continue;
        };
        if probe.has_path(source, hit) {
            return Some(hit);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 置换替身：固定序并数键。
    struct FixedPermute {
        order: Vec<usize>,
        keys: usize,
        calls: usize,
    }

    impl FixedPermute {
        fn identity(len: usize) -> Self {
            Self { order: (0..len).collect(), keys: 0, calls: 0 }
        }

        fn reversed(len: usize) -> Self {
            Self { order: (0..len).rev().collect(), keys: 0, calls: 0 }
        }
    }

    impl Permute for FixedPermute {
        fn permutation(&mut self, len: usize) -> Vec<usize> {
            self.calls += 1;
            self.keys += len;
            assert_eq!(self.order.len(), len, "替身置换长度必须等于环大小");
            self.order.clone()
        }
    }

    /// 探测替身：脚本化应答并记录全部调用。
    struct ScriptProbe {
        /// 每次采样的应答：Some(dy) = 命中且命中位抬 dy；None = 未命中。
        sample_script: Vec<Option<f32>>,
        /// 每次验路的应答。
        path_script: Vec<bool>,
        samples: Vec<[f32; 3]>,
        tolerances: Vec<f32>,
        path_queries: Vec<([f32; 3], [f32; 3])>,
    }

    impl ScriptProbe {
        fn new(sample_script: Vec<Option<f32>>, path_script: Vec<bool>) -> Self {
            Self {
                sample_script,
                path_script,
                samples: Vec::new(),
                tolerances: Vec::new(),
                path_queries: Vec::new(),
            }
        }
    }

    impl SurfaceProbe for ScriptProbe {
        fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
            self.samples.push(target);
            self.tolerances.push(tolerance);
            let step = *self
                .sample_script
                .get(self.samples.len() - 1)
                .expect("采样脚本覆盖全部调用");
            step.map(|dy| [target[0], target[1] + dy, target[2]])
        }

        fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool {
            let answer = *self
                .path_script
                .get(self.path_queries.len())
                .expect("验路脚本覆盖全部调用");
            self.path_queries.push((source, target));
            answer
        }
    }

    /// 测试用格角映射：格 (x, z) → 世界 (x·0.25, 0, z·0.25)。
    fn corner(cell: Cell) -> [f32; 3] {
        [cell.0 as f32 * TILE_SCALE, 0.0, cell.1 as f32 * TILE_SCALE]
    }

    #[test]
    fn ring_filter_bounds_are_inclusive() {
        // 原点出发，档 [2, 3]：d² ∈ [4, 9] 双端含
        assert!(ring_filter((0, 0), (2, 0), 2, 3), "d²=4 恰在下界");
        assert!(ring_filter((0, 0), (3, 0), 2, 3), "d²=9 恰在上界");
        assert!(!ring_filter((0, 0), (1, 0), 2, 3), "d²=1 低于下界");
        assert!(!ring_filter((0, 0), (4, 0), 2, 3), "d²=16 超上界");
        assert!(ring_filter((0, 0), (2, 1), 2, 3), "d²=5 在带内");
        assert!(ring_filter((0, 0), (2, 2), 2, 3), "d²=8 在带内");
        assert!(!ring_filter((0, 0), (1, 1), 2, 3), "d²=2 低于下界");
    }

    #[test]
    fn ring_filter_measures_from_origin_not_zero() {
        // 距离从出发点量起，不是从格 (0,0)
        assert!(ring_filter((10, 10), (12, 10), 2, 3), "相对出发格 d²=4");
        assert!(!ring_filter((10, 10), (10, 10), 2, 3), "自身 d²=0");
        assert!(ring_filter((-5, 0), (-8, 0), 2, 3), "负向格 d²=9 恰上界");
        assert!(!ring_filter((-5, 0), (-9, 0), 2, 3), "负向格 d²=16 超上界");
    }

    #[test]
    fn ring_filter_clamps_max_square_to_int_bound() {
        // max² 超整型上界时钳到上界：46341² = 2147488281 > 2147483647
        assert!(ring_filter((0, 0), (46340, 0), 2, 46341), "d²=2147395600 未超钳后上界");
        assert!(
            !ring_filter((0, 0), (46341, 0), 2, 46341),
            "d²=2147488281 超钳后上界"
        );
    }

    #[test]
    fn empty_eligible_yields_none_without_probing() {
        // 占格差集为空：未命中、零探测、零键
        let mut rand = FixedPermute::identity(0);
        let mut probe = ScriptProbe::new(vec![], vec![]);
        assert_eq!(
            wander_target((0, 0), 2, 3, &[], corner, &mut rand, &mut probe),
            None
        );
        assert_eq!(rand.keys, 0);
        assert!(probe.samples.is_empty());
        assert!(probe.path_queries.is_empty());
    }

    #[test]
    fn ring_out_of_band_yields_none_without_probing() {
        // 差集非空但环空（都出带）：零键零探测
        let eligible = [(0, 0), (1, 0), (100, 100)];
        let mut rand = FixedPermute::identity(0);
        let mut probe = ScriptProbe::new(vec![], vec![]);
        assert_eq!(
            wander_target((0, 0), 12, 16, &eligible, corner, &mut rand, &mut probe),
            None
        );
        assert_eq!(rand.keys, 0);
        assert!(probe.samples.is_empty());
    }

    #[test]
    fn first_double_pass_wins_with_sampled_position() {
        // 首格双过：返回采样命中位（不是格角），恰 1 采样 1 验路
        let eligible = [(2, 0), (3, 0)];
        let mut rand = FixedPermute::identity(2);
        let mut probe = ScriptProbe::new(vec![Some(0.5)], vec![true]);
        let found = wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(found, Some([0.5, 0.5, 0.0]));
        assert_eq!(probe.samples.len(), 1);
        assert_eq!(probe.path_queries.len(), 1);
        // 采样容差 = 格距
        assert_eq!(probe.tolerances, vec![TILE_SCALE]);
    }

    #[test]
    fn take_is_five_even_when_ring_is_larger() {
        // 环 8 格只探前 5：全 miss 恰 5 次采样、零验路、未命中
        let eligible = [(2, 0), (2, 1), (2, 2), (3, 0), (-2, 0), (-2, 1), (0, 2), (1, 2)];
        let mut rand = FixedPermute::identity(8);
        let mut probe =
            ScriptProbe::new(vec![None; 8], vec![]);
        assert_eq!(
            wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe),
            None
        );
        assert_eq!(probe.samples.len(), WANDER_TAKE);
        assert!(probe.path_queries.is_empty(), "采样全 miss 不验路");
        assert_eq!(rand.keys, 8, "键数 = 环大小（8），不是取 5");
    }

    #[test]
    fn path_failure_walks_to_next_cell() {
        // 前 2 格验路失败、第 3 格双过：3 采样 2 验路，返回第 3 格命中位
        let eligible = [(2, 0), (3, 0), (2, 1)];
        let mut rand = FixedPermute::identity(3);
        let mut probe = ScriptProbe::new(
            vec![Some(0.1), Some(0.2), Some(0.3)],
            vec![false, false, true],
        );
        let found = wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(found, Some([0.5, 0.3, 0.25]));
        assert_eq!(probe.samples.len(), 3);
        assert_eq!(probe.path_queries.len(), 3, "每次采样命中都验一次路（2 败 1 成）");
    }

    #[test]
    fn sample_miss_skips_path_check() {
        // 首格采样 miss：不验路，第 2 格双过
        let eligible = [(2, 0), (3, 0)];
        let mut rand = FixedPermute::identity(2);
        let mut probe = ScriptProbe::new(vec![None, Some(0.0)], vec![true]);
        let found = wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(found, Some([0.75, 0.0, 0.0]));
        assert_eq!(probe.path_queries.len(), 1, "采样 miss 的格不验路");
    }

    #[test]
    fn probe_order_follows_permutation() {
        // 逆序置换：先探环尾
        let eligible = [(2, 0), (3, 0), (2, 1)];
        let mut rand = FixedPermute::reversed(3);
        let mut probe = ScriptProbe::new(vec![None; 3], vec![]);
        wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(
            probe.samples,
            vec![corner((2, 1)), corner((3, 0)), corner((2, 0))]
        );
    }

    #[test]
    fn path_source_is_origin_cell_corner() {
        // 验路出发点 = 出发格的格角世界位（不是角色脚点）
        let eligible = [(12, 4)];
        let mut rand = FixedPermute::identity(1);
        let mut probe = ScriptProbe::new(vec![Some(0.0)], vec![true]);
        wander_target((10, 4), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(probe.path_queries.len(), 1);
        assert_eq!(probe.path_queries[0].0, corner((10, 4)));
    }

    #[test]
    fn permutation_is_called_exactly_once_with_ring_size() {
        // 恰一次置换调用，键数 = 环大小（6 格全在带内）
        let eligible = [(2, 0), (2, 1), (2, 2), (3, 0), (-2, 0), (0, 2)];
        let mut rand = FixedPermute::identity(6);
        let mut probe = ScriptProbe::new(vec![Some(0.0)], vec![true]);
        wander_target((0, 0), 2, 3, &eligible, corner, &mut rand, &mut probe);
        assert_eq!(rand.calls, 1);
        assert_eq!(rand.keys, 6);
    }
}
