//! 多边形网与其上的查询（`rcBuildPolyMesh` + Detour 的 2D 转录）。
//!
//! 轮廓之后，每条轮廓被三角化成凸单元，相邻单元之间的公共边就是「门」。
//! 寻路在这张图上做 A\*，再用漏斗把多边形序列收成拐点——这与原生一致：
//! 原生查的是多边形 navmesh，不是烘焙用的体素场。
//!
//! # 锚点档次（必须分清，不许混成一句）
//!
//! * **反编译档**：多边形网的输出形状与 portal 位布局（`0x8000 | dir`，
//!   `dir` 取 0:`x==0` / 1:`z==h` / 2:`x==w` / 3:`z==0`）已在
//!   `FUN_009121c0` 里逐条坐实。我方未分瓦 ⇒ 没有跨瓦边，这套标记用不上，
//!   但记在这里，将来补分瓦时直接照用。
//! * **知识库档**：三角化的选耳规则、顶点哈希式子、邻接边配对。原生这三段
//!   在 `FUN_00913c24` / `FUN_00914164` / `FUN_00914590` 三个**函数体没有被
//!   导出**的独立函数里（`find` 命中 0，同命令阳性对照命中 2）⇒ 读不到。
//!   这里写的是公开算法，档次低于上一条。补齐的唯一路径是重跑一次导出，
//!   不是再读调用方。
//!
//! # 一处具名的范围缺口
//!
//! ⚠ **凸多边形合并（`getPolyMergeValue` / `mergePolyVerts`，nvp = 6）没有实现。**
//! 它的评分与凸性谓词同样落在那批读不到的函数体里。对寻路而言它是**计数
//! 优化，不是正确性要求**：三角网本身已经是合法的凸分解，漏斗在任何凸分解
//! 上都给出走廊内的紧绷路径。代价是多边形数约为原生的 2–3 倍，而 A\* 在
//! 平面图上的展开量不随之线性增长。补它之前，别把本模块的多边形数拿去与
//! 原生对账——两边数的不是同一种单元。

use super::contour::Contour;
use super::funnel::{self, Portal};
use super::grid::Grid;
use super::region::Regions;
use std::collections::{BinaryHeap, HashMap};

/// 「没有邻居」。
const NO_NEIGHBOUR: u32 = u32::MAX;

/// 三角化后的导航多边形网。
pub(crate) struct PolyMesh {
    /// 顶点，格坐标。
    verts: Vec<[i32; 2]>,
    /// 每个单元的三个顶点下标（逆时针）。
    tris: Vec<[u32; 3]>,
    /// 每个单元三条边的邻居单元；边 `k` 是 `tris[k] → tris[(k+1)%3]`。
    neighbours: Vec<[u32; 3]>,
    /// 区号 → 该区的单元下标。点定位时用它把候选集缩小到一个区。
    by_region: HashMap<u32, Vec<u32>>,
}

impl PolyMesh {
    pub(crate) fn polygon_count(&self) -> usize {
        self.tris.len()
    }
}

/// 三角形两两之间的有向边 → (单元, 边序号)，用来配对邻接。
type EdgeKey = (u32, u32);

/// 由轮廓建多边形网。
pub(crate) fn build(contours: &[Contour]) -> PolyMesh {
    let mut verts: Vec<[i32; 2]> = Vec::new();
    let mut lookup: HashMap<[i32; 2], u32> = HashMap::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut regions: Vec<u32> = Vec::new();

    for contour in contours {
        let ring: Vec<[i32; 2]> = contour.verts.iter().map(|v| [v.x, v.z]).collect();
        let indices = triangulate(&ring);
        for [a, b, c] in indices {
            let mut global = [0u32; 3];
            for (slot, local) in [a, b, c].into_iter().enumerate() {
                let point = ring[local];
                let id = *lookup.entry(point).or_insert_with(|| {
                    verts.push(point);
                    (verts.len() - 1) as u32
                });
                global[slot] = id;
            }
            // 退化三角（去重后两点重合）丢掉——它没有面积，做不了门。
            if global[0] == global[1] || global[1] == global[2] || global[2] == global[0] {
                continue;
            }
            tris.push(global);
            regions.push(contour.region);
        }
    }

    // 邻接：同一条边被两个三角以相反方向各用一次。
    let mut edges: HashMap<EdgeKey, (u32, usize)> = HashMap::new();
    let mut neighbours = vec![[NO_NEIGHBOUR; 3]; tris.len()];
    for (index, triangle) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (triangle[k], triangle[(k + 1) % 3]);
            if let Some((other, other_k)) = edges.remove(&(b, a)) {
                neighbours[index][k] = other;
                neighbours[other as usize][other_k] = index as u32;
            } else {
                edges.insert((a, b), (index as u32, k));
            }
        }
    }

    let mut by_region: HashMap<u32, Vec<u32>> = HashMap::new();
    for (index, region) in regions.iter().enumerate() {
        by_region.entry(*region).or_default().push(index as u32);
    }

    PolyMesh {
        verts,
        tris,
        neighbours,
        by_region,
    }
}

/// 有符号面积的两倍（整数，精确）。逆时针为正。
fn area2(a: [i32; 2], b: [i32; 2], c: [i32; 2]) -> i64 {
    let (ax, az) = (a[0] as i64, a[1] as i64);
    let (bx, bz) = (b[0] as i64, b[1] as i64);
    let (cx, cz) = (c[0] as i64, c[1] as i64);
    (bx - ax) * (cz - az) - (bz - az) * (cx - ax)
}

/// 耳切三角化。
///
/// ⚠ 知识库档（见模块注释）：原生的选耳谓词读不到，这里用公开的
/// 「凸角 + 无其它顶点落入」规则。整数叉积，精确无浮点误差。
///
/// 轮廓的环绕方向由追踪的手性决定，而那取决于一张读不到的方向偏移表
/// ⇒ 这里先按有符号面积把环统一成逆时针，不假定手性。
fn triangulate(ring: &[[i32; 2]]) -> Vec<[usize; 3]> {
    let count = ring.len();
    if count < 3 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..count).collect();
    let mut total = 0i64;
    for i in 0..count {
        let j = (i + 1) % count;
        total += (ring[i][0] as i64) * (ring[j][1] as i64) - (ring[j][0] as i64) * (ring[i][1] as i64);
    }
    if total < 0 {
        order.reverse();
    }

    let mut out = Vec::with_capacity(count.saturating_sub(2));
    let mut guard = 0usize;
    while order.len() > 3 {
        // 每轮至少切一只耳；切不动就停（自交或退化的环，不硬凑）。
        let before = order.len();
        let mut at = 0usize;
        while at < order.len() {
            let n = order.len();
            let (prev, current, next) = (
                order[(at + n - 1) % n],
                order[at],
                order[(at + 1) % n],
            );
            if is_ear(ring, &order, prev, current, next) {
                out.push([prev, current, next]);
                order.remove(at);
                break;
            }
            at += 1;
        }
        if order.len() == before {
            return out; // 切不动，交已切出的部分
        }
        guard += 1;
        if guard > count {
            return out;
        }
    }
    if order.len() == 3 {
        out.push([order[0], order[1], order[2]]);
    }
    out
}

/// `current` 是不是一只耳：凸角，且环上其它顶点都不落在这个三角里。
fn is_ear(ring: &[[i32; 2]], order: &[usize], prev: usize, current: usize, next: usize) -> bool {
    let (a, b, c) = (ring[prev], ring[current], ring[next]);
    if area2(a, b, c) <= 0 {
        return false; // 凹角或共线
    }
    for &other in order {
        if other == prev || other == current || other == next {
            continue;
        }
        let p = ring[other];
        if area2(a, b, p) >= 0 && area2(b, c, p) >= 0 && area2(c, a, p) >= 0 {
            return false;
        }
    }
    true
}

// —— 查询 ——

/// 格坐标 → 世界坐标（格角，不是格心）。
fn to_world(grid: &Grid, v: [i32; 2]) -> [f32; 2] {
    [
        grid.origin[0] + v[0] as f32 * grid.voxel,
        grid.origin[1] + v[1] as f32 * grid.voxel,
    ]
}

/// 平方距离。
fn distance2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

impl PolyMesh {
    /// 点落在哪个单元里。先用分区把候选集缩到一个区，再逐个含点判定。
    ///
    /// ⚠ **含点判定失败不等于不可走。** 轮廓简化允许折线偏离真实边界最多
    /// `MAX_SIMPLIFICATION_ERROR` 格，所以贴边的可走格可以落在多边形之外
    /// ——这是原生也有的性质（Detour 的 navmesh 并不精确贴合体素场），
    /// 原生查询侧用带容差的 `findNearestPoly` 兜底。这里同形：含点失败就
    /// 退到「同区内单元心最近的那个」。可走性本身仍由格面裁定，不由这里裁定。
    fn locate(&self, grid: &Grid, regions: &Regions, p: [f32; 2]) -> Option<u32> {
        let (cx, cz) = grid.cell_of(p[0], p[1]);
        if !grid.walkable_cell(cx, cz) {
            return None;
        }
        let region = regions.ids[cz as usize * grid.cols + cx as usize];
        let candidates = self.by_region.get(&region)?;
        if let Some(hit) = candidates
            .iter()
            .copied()
            .find(|index| self.contains(grid, *index, p))
        {
            return Some(hit);
        }
        candidates
            .iter()
            .copied()
            .min_by(|a, b| {
                let da = distance2(self.centre(grid, *a), p);
                let db = distance2(self.centre(grid, *b), p);
                da.total_cmp(&db)
            })
    }

    fn contains(&self, grid: &Grid, index: u32, p: [f32; 2]) -> bool {
        let triangle = self.tris[index as usize];
        let corners: Vec<[f32; 2]> = triangle
            .iter()
            .map(|v| to_world(grid, self.verts[*v as usize]))
            .collect();
        let sign = |a: [f32; 2], b: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let s0 = sign(corners[0], corners[1]);
        let s1 = sign(corners[1], corners[2]);
        let s2 = sign(corners[2], corners[0]);
        (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
    }

    fn centre(&self, grid: &Grid, index: u32) -> [f32; 2] {
        let triangle = self.tris[index as usize];
        let mut sum = [0.0f32; 2];
        for v in triangle {
            let world = to_world(grid, self.verts[v as usize]);
            sum[0] += world[0];
            sum[1] += world[1];
        }
        [sum[0] / 3.0, sum[1] / 3.0]
    }

    /// 单元图上的 A\*，返回单元序列。代价与启发都用单元心的欧氏距离
    /// （启发可采纳：直线距离不超过任何实际路径长）。
    fn find_corridor(&self, grid: &Grid, from: u32, to: u32) -> Option<Vec<u32>> {
        if from == to {
            return Some(vec![from]);
        }
        let goal = self.centre(grid, to);
        let heuristic = |index: u32| {
            let c = self.centre(grid, index);
            ((c[0] - goal[0]).powi(2) + (c[1] - goal[1]).powi(2)).sqrt()
        };
        // 二叉堆是大顶堆，存 f 的负值把它当小顶堆用；f32 不是 Ord，按位序
        // 比较（这些值都非负有限，位序与数值序一致）。
        let key = |f: f32| -((f.to_bits()) as i64);
        let mut open = BinaryHeap::new();
        let mut best: HashMap<u32, f32> = HashMap::new();
        let mut came: HashMap<u32, u32> = HashMap::new();
        best.insert(from, 0.0);
        open.push((key(heuristic(from)), from));
        while let Some((_, current)) = open.pop() {
            if current == to {
                let mut chain = vec![current];
                let mut cursor = current;
                while let Some(previous) = came.get(&cursor) {
                    cursor = *previous;
                    chain.push(cursor);
                }
                chain.reverse();
                return Some(chain);
            }
            let cost = best.get(&current).copied().unwrap_or(f32::MAX);
            let here = self.centre(grid, current);
            for neighbour in self.neighbours[current as usize] {
                if neighbour == NO_NEIGHBOUR {
                    continue;
                }
                let there = self.centre(grid, neighbour);
                let step = ((there[0] - here[0]).powi(2) + (there[1] - here[1]).powi(2)).sqrt();
                let candidate = cost + step;
                if candidate < best.get(&neighbour).copied().unwrap_or(f32::MAX) {
                    best.insert(neighbour, candidate);
                    came.insert(neighbour, current);
                    open.push((key(candidate + heuristic(neighbour)), neighbour));
                }
            }
        }
        None
    }

    /// 走廊里相邻两单元的公共边，按前进方向定左右。
    fn portal(&self, grid: &Grid, from: u32, to: u32) -> Option<Portal> {
        let triangle = self.tris[from as usize];
        let k = self.neighbours[from as usize]
            .iter()
            .position(|n| *n == to)?;
        let a = to_world(grid, self.verts[triangle[k] as usize]);
        let b = to_world(grid, self.verts[triangle[(k + 1) % 3] as usize]);
        // 三角形按逆时针存 ⇒ 沿边 k（a→b）走时内部在左，**穿出**这条边的
        // 前进方向是 a→b 的右侧；面朝该方向时左手侧是 b、右手侧是 a。
        // 写成 [a, b] 会让漏斗左右互换，路径会被甩到走廊外的远角上。
        Some([b, a])
    }

    /// 沿面折线：单元 A\* + 漏斗整直。目标不可达时返回 `None`。
    pub(crate) fn path(
        &self,
        grid: &Grid,
        regions: &Regions,
        start: [f32; 2],
        goal: [f32; 2],
    ) -> Option<Vec<[f32; 2]>> {
        let from = self.locate(grid, regions, start)?;
        let to = self.locate(grid, regions, goal)?;
        let corridor = self.find_corridor(grid, from, to)?;
        let mut portals = Vec::with_capacity(corridor.len().saturating_sub(1));
        for pair in corridor.windows(2) {
            portals.push(self.portal(grid, pair[0], pair[1])?);
        }
        Some(funnel::straight_path(start, goal, &portals))
    }
}
