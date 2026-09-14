//! 查询层：点的可走性、最近可走点、沿面折线（A\* + 视线整直）。
//!
//! 查询的形状锚在真源 `MoveUtility` 的两个同族方法上：
//!
//! * `TryGetCanNavmeshTargetPosition(from, target)`——目标先过一次
//!   `NavMesh.SamplePosition(·, 5.0)` 吸附，再 `CalculatePath`；任一步
//!   失败就把目标沿「目标→起点」方向拉回一档（t = 0.9, 0.8, …, 0.0，
//!   共十档，首试是原目标）重来；全败时返回起点本身（「哪儿也去不了
//!   就原地」）。本模块的 [`path`] 转录这条链，只把「返回末拐点」放宽
//!   为「返回整条折线」（末拐点是折线的最后一个元素，消费侧自取），
//!   全败折成 `None`（原地把不走折线的决定留给调用方）。
//! * 同文件的 `CanNavmeshMoveTargetPosition` 证明严判形态（不吸附、
//!   不拉回、按末拐点与目标的水平距离过阈）也在源里并存——本模块
//!   不做严判变体，消费侧需要时对折线末点自比即可。
//!
//! 折线的整直：真源 `CalculatePath` 返回的 corners 是整直后的直线路
//! 径（原生代理内部的搜索 + straight path 后处理在原生墙后）。本模块
//! 用「格上 A\* + 贪心视线拉直」做同形转录：A\* 给出绕洞的格链，视线
//! 拉直把能直连的连续段合并。视线判定走 Amanatides-Woo 超覆盖：线段
//! 穿过的每一格都必须可行走；恰好过角点时两侧格与对角格都要可走
//! （保守侧：只有半径一格的站上，角点邻格可能是洞）。

use super::grid::Grid;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// 端点吸附上限：真源移动侧查询里 `SamplePosition` 的 maxDistance 字面量。
pub const SNAP_MAX_DISTANCE: f32 = 5.0;

/// 点是否落在可行走格上（出界按不可走）。
pub(crate) fn walkable_at(grid: &Grid, p: [f32; 2]) -> bool {
    if !p.into_iter().all(f32::is_finite) {
        return false;
    }
    let (cx, cz) = grid.cell_of(p[0], p[1]);
    grid.walkable_cell(cx, cz)
}

/// 最近可走点（`SamplePosition` 的格面形态）。
///
/// 点自己所在格可行走时返回点本身（在面上的点，最近面点就是它）；
/// 否则环形外扫找最近的可走格中心。`max_dist` 是吸附上限，最近的
/// 可走格中心超限时返回 `None`（吸附失败，不是静默取远点）。
pub(crate) fn nearest_walkable(
    grid: &Grid,
    p: [f32; 2],
    max_dist: Option<f32>,
) -> Option<[f32; 2]> {
    if !p.into_iter().all(f32::is_finite)
        || max_dist.is_some_and(|d| !d.is_finite() || d < 0.0)
    {
        return None;
    }
    let (cx, cz) = grid.cell_of(p[0], p[1]);
    if grid.walkable_cell(cx, cz) {
        return Some(p);
    }
    let limit_r = max_dist.map(|d| (d / grid.voxel).ceil() as isize + 1);
    let hard_r = grid.cols.max(grid.rows) as isize;
    let mut best: Option<([f32; 2], f32)> = None;
    for r in 1..=hard_r {
        if let Some(limit) = limit_r {
            if r > limit {
                break;
            }
        }
        // 环 r：Chebyshev 距离恰为 r 的格。
        for dz in -r..=r {
            let full_row = dz.abs() == r;
            let count = if full_row { 2 * r + 1 } else { 2 };
            for index in 0..count {
                let dx = if full_row { index - r } else if index == 0 { -r } else { r };
                if !grid.walkable_cell(cx + dx, cz + dz) {
                    continue;
                }
                let center = grid.cell_center((cx + dx) as usize, (cz + dz) as usize);
                let d = (center[0] - p[0]).powi(2) + (center[1] - p[1]).powi(2);
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((center, d));
                }
            }
        }
        // 扫完一环后用保守距离下界提前终止。
        if let Some((_, bd)) = best {
            // 查询点不必在格中心；下一环最近边相距至少 (r - 0.5) 格。
            let floor = ((r as f32 - 0.5) * grid.voxel).powi(2);
            if bd <= floor {
                break;
            }
        }
    }
    match best {
        Some((center, d)) => {
            if max_dist.map_or(true, |m| d <= m * m) {
                Some(center)
            } else {
                None
            }
        }
        None => None,
    }
}

/// 沿面折线：起点、终点，返回折线（路点表）。
///
/// 转录 `TryGetCanNavmeshTargetPosition` 的查询链：起点吸附（失败即
/// `None`）；目标先原样试，失败按 t = 0.9…0.0 拉向起点逐档重试（每档
/// 都重新吸附）；首个「吸附成功 ∧ A\* 连通」的档出折线。全败 `None`。
/// 折线首点是起点吸附点、末点是目标吸附点，中间路点是格中心，整段
/// 都在可行走格上。
pub(crate) fn path(grid: &Grid, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
    let start_pt = nearest_walkable(grid, start, Some(SNAP_MAX_DISTANCE))?;
    for k in (0..=10).rev() {
        let t = k as f32 / 10.0;
        let candidate = [
            start[0] + (goal[0] - start[0]) * t,
            start[1] + (goal[1] - start[1]) * t,
        ];
        let Some(goal_pt) = nearest_walkable(grid, candidate, Some(SNAP_MAX_DISTANCE)) else {
            continue;
        };
        if let Some(points) = path_exact(grid, start_pt, goal_pt) {
            return Some(points);
        }
    }
    None
}

pub(crate) fn path_exact(grid: &Grid, start: [f32; 2], goal: [f32; 2]) -> Option<Vec<[f32; 2]>> {
    if !walkable_at(grid, start) || !walkable_at(grid, goal) {
        return None;
    }
    if visible(grid, start, goal) {
        return Some(vec![start, goal]);
    }
    let cells = astar(grid, grid.cell_of(start[0], start[1]), grid.cell_of(goal[0], goal[1]))?;
    // 保留起终格中心：任意格内端点直接替换中心会让首/末斜腿切角。
    let mut points = Vec::with_capacity(cells.len() + 2);
    points.push(start);
    points.extend(cells.into_iter().map(|(x, z)| grid.cell_center(x, z)));
    points.push(goal);
    smooth(grid, &mut points);
    Some(points)
}

// —— A* ——

/// 基数步代价（十分之一格单位）：与斜向步 14 配成 octile 量纲。
const COST_CARDINAL: u32 = 10;
const COST_DIAGONAL: u32 = 14;

struct HeapItem {
    f: u32,
    g: u32,
    index: usize,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap 是大顶堆：更小的 f、同 f 时更大的 g 排「更大」
        // （先出堆），等值项按格下标稳定化。
        other
            .f
            .cmp(&self.f)
            .then(other.g.cmp(&self.g))
            .then(self.index.cmp(&other.index))
    }
}

/// 八连通 A\*（HashMap 存开放表状态，不按整格分配）。
/// 斜向步不许切角：两个基数邻格都得可行走。
/// 起终点已假定可行走（调用方吸附过）。
fn astar(grid: &Grid, from: (isize, isize), to: (isize, isize)) -> Option<Vec<(usize, usize)>> {
    let cols = grid.cols as isize;
    let index = |cx: isize, cz: isize| (cz * cols + cx) as usize;
    let octile = |a: (isize, isize), b: (isize, isize)| -> u32 {
        let dx = (a.0 - b.0).unsigned_abs() as u32;
        let dz = (a.1 - b.1).unsigned_abs() as u32;
        COST_CARDINAL * dx.max(dz) + (COST_DIAGONAL - COST_CARDINAL) * dx.min(dz)
    };
    let start_i = index(from.0, from.1);
    let goal_i = index(to.0, to.1);
    let mut g_score: HashMap<usize, u32> = HashMap::new();
    let mut came_from: HashMap<usize, usize> = HashMap::new();
    g_score.insert(start_i, 0);
    let mut heap = BinaryHeap::new();
    heap.push(HeapItem {
        f: octile(from, to),
        g: 0,
        index: start_i,
    });
    while let Some(item) = heap.pop() {
        if item.g > *g_score.get(&item.index).unwrap_or(&u32::MAX) {
            continue; // 陈旧堆项
        }
        if item.index == goal_i {
            let mut cells = Vec::new();
            let mut cur = goal_i;
            loop {
                cells.push(((cur % cols as usize), (cur / cols as usize)));
                if cur == start_i {
                    break;
                }
                cur = *came_from.get(&cur).expect("A* 回溯链断裂");
            }
            cells.reverse();
            return Some(cells);
        }
        let cx = (item.index % cols as usize) as isize;
        let cz = (item.index / cols as usize) as isize;
        for dz in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                let (nx, nz) = (cx + dx, cz + dz);
                if !grid.walkable_cell(nx, nz) {
                    continue;
                }
                if dx != 0 && dz != 0 {
                    // 不切角：两个基数邻格都得可行走。
                    if !grid.walkable_cell(cx + dx, cz) || !grid.walkable_cell(cx, cz + dz) {
                        continue;
                    }
                }
                let cost = if dx != 0 && dz != 0 {
                    COST_DIAGONAL
                } else {
                    COST_CARDINAL
                };
                let next_i = index(nx, nz);
                let next_g = item.g + cost;
                if next_g < *g_score.get(&next_i).unwrap_or(&u32::MAX) {
                    g_score.insert(next_i, next_g);
                    came_from.insert(next_i, item.index);
                    heap.push(HeapItem {
                        f: next_g + octile((nx, nz), to),
                        g: next_g,
                        index: next_i,
                    });
                }
            }
        }
    }
    None
}

// —— 视线与整直 ——

/// 两点间视线：Amanatides-Woo 超覆盖，线段穿过的每一格都必须可行走。
/// 恰过角点时（两轴 t 同时到界），两侧格与对角格一并要求。
pub(crate) fn visible(grid: &Grid, a: [f32; 2], b: [f32; 2]) -> bool {
    if !a.into_iter().chain(b).all(f32::is_finite) {
        return false;
    }
    let (mut cx, mut cz) = grid.cell_of(a[0], a[1]);
    let (tx, tz) = grid.cell_of(b[0], b[1]);
    if !grid.walkable_cell(cx, cz) || !grid.walkable_cell(tx, tz) {
        return false;
    }
    if (cx, cz) == (tx, tz) {
        return true;
    }
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let step_x = if dx > 0.0 {
        1
    } else if dx < 0.0 {
        -1
    } else {
        0
    };
    let step_z = if dz > 0.0 {
        1
    } else if dz < 0.0 {
        -1
    } else {
        0
    };
    let t_delta_x = if step_x != 0 {
        grid.voxel / dx.abs()
    } else {
        f32::INFINITY
    };
    let t_delta_z = if step_z != 0 {
        grid.voxel / dz.abs()
    } else {
        f32::INFINITY
    };
    let bound_x = if step_x > 0 {
        grid.origin[0] + (cx as f32 + 1.0) * grid.voxel
    } else {
        grid.origin[0] + cx as f32 * grid.voxel
    };
    let bound_z = if step_z > 0 {
        grid.origin[1] + (cz as f32 + 1.0) * grid.voxel
    } else {
        grid.origin[1] + cz as f32 * grid.voxel
    };
    let mut t_max_x = if step_x != 0 {
        (bound_x - a[0]) / dx
    } else {
        f32::INFINITY
    };
    let mut t_max_z = if step_z != 0 {
        (bound_z - a[1]) / dz
    } else {
        f32::INFINITY
    };
    loop {
        if t_max_x < t_max_z {
            cx += step_x;
            t_max_x += t_delta_x;
            if !grid.walkable_cell(cx, cz) {
                return false;
            }
        } else if t_max_z < t_max_x {
            cz += step_z;
            t_max_z += t_delta_z;
            if !grid.walkable_cell(cx, cz) {
                return false;
            }
        } else {
            // 角点：两侧格与对角格全部要求可行走。
            if !grid.walkable_cell(cx + step_x, cz) || !grid.walkable_cell(cx, cz + step_z) {
                return false;
            }
            cx += step_x;
            cz += step_z;
            t_max_x += t_delta_x;
            t_max_z += t_delta_z;
            if !grid.walkable_cell(cx, cz) {
                return false;
            }
        }
        if (cx, cz) == (tx, tz) {
            return true;
        }
    }
}

/// 贪心视线拉直：从首点起，每次直连最远的可见后继。
fn smooth(grid: &Grid, pts: &mut Vec<[f32; 2]>) {
    if pts.len() <= 2 {
        return;
    }
    let mut result = vec![pts[0]];
    let mut i = 0;
    while i + 1 < pts.len() {
        let mut j = pts.len() - 1;
        while j > i + 1 && !visible(grid, pts[i], pts[j]) {
            j -= 1;
        }
        result.push(pts[j]);
        i = j;
    }
    *pts = result;
}
