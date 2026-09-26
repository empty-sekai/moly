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
use std::cell::RefCell;
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
    /// 单元所属的连通分量（沿邻接边可达的单元同号）。两端不同号时单元图上
    /// 没有路径，A\* 必然展开完整个起点分量后返回空；这里直接给出同一结论。
    components: Vec<u32>,
    /// 单元心的世界坐标，按查询时同一条式子预先算好；只在查询所用的格参数
    /// 与缓存时一致时读取（见 `cache_centres`）。
    centres: Vec<[f32; 2]>,
    centre_grid: Option<([f32; 2], f32)>,
}

/// A\* 的工作表：稠密数组加代号戳，免去每次查询的哈希表分配与哈希。
/// 浏览器与原生都在单线程里查询，一线程一份。
#[derive(Default)]
struct SearchScratch {
    best: Vec<f32>,
    came: Vec<u32>,
    stamp: Vec<u32>,
    generation: u32,
}

impl SearchScratch {
    fn begin(&mut self, len: usize) {
        if self.stamp.len() != len {
            self.best = vec![f32::MAX; len];
            self.came = vec![NO_NEIGHBOUR; len];
            self.stamp = vec![0; len];
            self.generation = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }

    fn best(&self, index: u32) -> f32 {
        let i = index as usize;
        if self.stamp[i] == self.generation {
            self.best[i]
        } else {
            f32::MAX
        }
    }

    fn came(&self, index: u32) -> Option<u32> {
        let i = index as usize;
        (self.stamp[i] == self.generation && self.came[i] != NO_NEIGHBOUR).then(|| self.came[i])
    }

    fn set(&mut self, index: u32, best: f32, came: u32) {
        let i = index as usize;
        self.stamp[i] = self.generation;
        self.best[i] = best;
        self.came[i] = came;
    }
}

thread_local! {
    static SCRATCH: RefCell<SearchScratch> = RefCell::new(SearchScratch::default());
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

    let components = connected_components(&neighbours);
    PolyMesh {
        verts,
        tris,
        neighbours,
        by_region,
        components,
        centres: Vec::new(),
        centre_grid: None,
    }
}

/// 沿邻接边的连通分量编号（按单元下标顺序起新分量）。
fn connected_components(neighbours: &[[u32; 3]]) -> Vec<u32> {
    let mut component = vec![NO_NEIGHBOUR; neighbours.len()];
    let mut stack = Vec::new();
    let mut next = 0u32;
    for seed in 0..neighbours.len() {
        if component[seed] != NO_NEIGHBOUR {
            continue;
        }
        component[seed] = next;
        stack.push(seed as u32);
        while let Some(current) = stack.pop() {
            for neighbour in neighbours[current as usize] {
                if neighbour != NO_NEIGHBOUR && component[neighbour as usize] == NO_NEIGHBOUR {
                    component[neighbour as usize] = next;
                    stack.push(neighbour);
                }
            }
        }
        next += 1;
    }
    component
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
    /// `MAX_SIMPLIFICATION_ERROR` 格，所以贴边的可走格可以落在多边形之外，
    /// 一个区也可能一个单元都没有三角化出来。引擎在这里用的是
    /// `NavMeshQuery.FindNearestPoly`：查询盒内**所有**单元里取最近点距离
    /// 最小的那个，且最近点必须落在查询盒内；它不看分区。含点失败时照此
    /// 取单元（`half_extent` 是查询盒的水平半宽）。可走性本身仍由格面裁定。
    fn locate(&self, grid: &Grid, regions: &Regions, p: [f32; 2], half_extent: f32) -> Option<u32> {
        let (cx, cz) = grid.cell_of(p[0], p[1]);
        if !grid.walkable_cell(cx, cz) {
            return None;
        }
        let region = regions.ids[cz as usize * grid.cols + cx as usize];
        if let Some(hit) = self.by_region.get(&region).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|index| self.contains(grid, *index, p))
        }) {
            return Some(hit);
        }
        let mut best: Option<(u32, f32)> = None;
        for index in 0..self.tris.len() as u32 {
            let corners = self.tris[index as usize].map(|v| to_world(grid, self.verts[v as usize]));
            let q = closest_on_triangle(p, corners);
            if (q[0] - p[0]).abs() > half_extent || (q[1] - p[1]).abs() > half_extent {
                continue;
            }
            let d = distance2(q, p);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((index, d));
            }
        }
        best.map(|(index, _)| index)
    }

    /// 点能否落进某个单元（[`Self::locate`] 成功与否）。
    pub(crate) fn locates(
        &self,
        grid: &Grid,
        regions: &Regions,
        p: [f32; 2],
        half_extent: f32,
    ) -> bool {
        self.locate(grid, regions, p, half_extent).is_some()
    }

    /// Diagnostics for [`Self::locate`]: the number of cells of `p`'s region,
    /// whether one of them contains `p`, and the horizontal distance from `p`
    /// to the nearest cell of any region.
    pub(crate) fn locate_report(
        &self,
        grid: &Grid,
        regions: &Regions,
        p: [f32; 2],
    ) -> (usize, bool, Option<f32>) {
        let (cx, cz) = grid.cell_of(p[0], p[1]);
        let (count, contained) = if grid.walkable_cell(cx, cz) {
            let region = regions.ids[cz as usize * grid.cols + cx as usize];
            let candidates = self.by_region.get(&region).map_or(&[][..], Vec::as_slice);
            (
                candidates.len(),
                candidates
                    .iter()
                    .any(|index| self.contains(grid, *index, p)),
            )
        } else {
            (0, false)
        };
        let nearest = (0..self.tris.len())
            .map(|index| {
                let corners = self.tris[index].map(|v| to_world(grid, self.verts[v as usize]));
                distance2(closest_on_triangle(p, corners), p)
            })
            .min_by(f32::total_cmp)
            .map(f32::sqrt);
        (count, contained, nearest)
    }

    fn contains(&self, grid: &Grid, index: u32, p: [f32; 2]) -> bool {
        let triangle = self.tris[index as usize];
        let corners = triangle.map(|v| to_world(grid, self.verts[v as usize]));
        let sign = |a: [f32; 2], b: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let s0 = sign(corners[0], corners[1]);
        let s1 = sign(corners[1], corners[2]);
        let s2 = sign(corners[2], corners[0]);
        (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
    }

    /// Precompute `centre` for the grid this mesh was baked on.
    pub(crate) fn cache_centres(&mut self, grid: &Grid) {
        self.centre_grid = None;
        self.centres = (0..self.tris.len() as u32)
            .map(|index| self.compute_centre(grid, index))
            .collect();
        self.centre_grid = Some((grid.origin, grid.voxel));
    }

    fn centre(&self, grid: &Grid, index: u32) -> [f32; 2] {
        if self.centre_grid == Some((grid.origin, grid.voxel)) {
            return self.centres[index as usize];
        }
        self.compute_centre(grid, index)
    }

    fn compute_centre(&self, grid: &Grid, index: u32) -> [f32; 2] {
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
        if self.components[from as usize] != self.components[to as usize] {
            return None;
        }
        SCRATCH.with(|scratch| self.search(grid, from, to, &mut scratch.borrow_mut()))
    }

    fn search(
        &self,
        grid: &Grid,
        from: u32,
        to: u32,
        scratch: &mut SearchScratch,
    ) -> Option<Vec<u32>> {
        scratch.begin(self.tris.len());
        let goal = self.centre(grid, to);
        let heuristic = |index: u32| {
            let c = self.centre(grid, index);
            ((c[0] - goal[0]).powi(2) + (c[1] - goal[1]).powi(2)).sqrt()
        };
        // 二叉堆是大顶堆，存 f 的负值把它当小顶堆用；f32 不是 Ord，按位序
        // 比较（这些值都非负有限，位序与数值序一致）。
        let key = |f: f32| -((f.to_bits()) as i64);
        let mut open = BinaryHeap::new();
        scratch.set(from, 0.0, NO_NEIGHBOUR);
        open.push((key(heuristic(from)), from));
        while let Some((_, current)) = open.pop() {
            if current == to {
                let mut chain = vec![current];
                let mut cursor = current;
                while let Some(previous) = scratch.came(cursor) {
                    cursor = previous;
                    chain.push(cursor);
                }
                chain.reverse();
                return Some(chain);
            }
            let cost = scratch.best(current);
            let here = self.centre(grid, current);
            for neighbour in self.neighbours[current as usize] {
                if neighbour == NO_NEIGHBOUR {
                    continue;
                }
                let there = self.centre(grid, neighbour);
                let step = ((there[0] - here[0]).powi(2) + (there[1] - here[1]).powi(2)).sqrt();
                let candidate = cost + step;
                if candidate < scratch.best(neighbour) {
                    scratch.set(neighbour, candidate, current);
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
        half_extent: f32,
    ) -> Option<Vec<[f32; 2]>> {
        let from = self.locate(grid, regions, start, half_extent)?;
        let to = self.locate(grid, regions, goal, half_extent)?;
        let corridor = self.find_corridor(grid, from, to)?;
        self.straighten(grid, &corridor, start, goal)
    }

    /// 完整或部分折线（Detour `findPath` 的两种成功结果）。两端都已映射到
    /// 可走格上。同一连通分量：完整折线，末点即 `goal`。不同分量：Detour
    /// 把走廊收在起点分量里启发距离最小的单元（搜索展开完整个分量后的
    /// `lastBestNode`），`findStraightPath` 再把终点夹到该单元上离目标
    /// 最近的点——引擎的 `CalculatePath` 对这种部分路径同样返回成功。
    /// 本网的单元位置是单元心（与本文件的 A\* 一致，Detour 用门中点），
    /// 部分终点另外落回可走格，使同一场上的后续查询与路线执行对得上。
    pub(crate) fn path_or_partial(
        &self,
        grid: &Grid,
        regions: &Regions,
        start: [f32; 2],
        goal: [f32; 2],
        half_extent: f32,
    ) -> Option<(Vec<[f32; 2]>, bool)> {
        let from = self.locate(grid, regions, start, half_extent)?;
        let to = self.locate(grid, regions, goal, half_extent)?;
        if self.components[from as usize] == self.components[to as usize] {
            let corridor = self.find_corridor(grid, from, to)?;
            return Some((self.straighten(grid, &corridor, start, goal)?, true));
        }
        let component = self.components[from as usize];
        // Iterator::min_by keeps the first of equal minima: ties resolve to the
        // lowest cell index, a stable stand-in for Detour's expansion order.
        let best = (0..self.tris.len() as u32)
            .filter(|index| self.components[*index as usize] == component)
            .min_by(|a, b| {
                distance2(self.centre(grid, *a), goal).total_cmp(&distance2(self.centre(grid, *b), goal))
            })?;
        let corridor = self.find_corridor(grid, from, best)?;
        let end = self.partial_end(grid, best, goal);
        Some((self.straighten(grid, &corridor, start, end)?, false))
    }

    fn straighten(
        &self,
        grid: &Grid,
        corridor: &[u32],
        start: [f32; 2],
        end: [f32; 2],
    ) -> Option<Vec<[f32; 2]>> {
        let mut portals = Vec::with_capacity(corridor.len().saturating_sub(1));
        for pair in corridor.windows(2) {
            portals.push(self.portal(grid, pair[0], pair[1])?);
        }
        Some(funnel::straight_path(start, end, &portals))
    }

    /// 部分路径终点：单元上离目标最近的点（`ClosestPointOnPoly`）。轮廓简化
    /// 允许单元边偏离格面，所以该点若不在可走格上，就沿它到单元心的线段
    /// 退回到第一处可走格；单元心也不可走时取离它最近的可走格心。
    fn partial_end(&self, grid: &Grid, index: u32, goal: [f32; 2]) -> [f32; 2] {
        let corners = self.tris[index as usize].map(|v| to_world(grid, self.verts[v as usize]));
        let closest = closest_on_triangle(goal, corners);
        let centre = self.centre(grid, index);
        const STEPS: u32 = 8;
        for step in 0..=STEPS {
            let t = step as f32 / STEPS as f32;
            let point = [
                closest[0] + (centre[0] - closest[0]) * t,
                closest[1] + (centre[1] - closest[1]) * t,
            ];
            if super::query::walkable_at(grid, point) {
                return point;
            }
        }
        super::query::nearest_walkable(grid, centre, None).unwrap_or(centre)
    }
}

// —— Agent movement on the cells ——

/// `NavMeshQuery::MoveAlongSurface` pads its search circle's radius by this
/// literal (the f32 nearest 0.001).
const MOVE_SEARCH_PAD: f32 = 0.001;
/// Its breadth-first queue holds at most this many nodes; a neighbour met
/// while it is full still counts as reached but is not expanded.
const MOVE_MAX_STACK: usize = 48;
/// The query runs on the navigation query's small node pool (64 nodes); a
/// neighbour that finds no free node is skipped.
const MOVE_NODE_POOL: usize = 64;
/// `PathCorridor::MovePosition` keeps at most this many visited cells.
pub(crate) const MOVE_MAX_VISITED: usize = 16;

/// One `MoveAlongSurface` answer: the reached point and the cells from the
/// start cell to the one it lies on (at most the requested count; `truncated`
/// when the chain was cut there).
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceMove {
    pub position: [f32; 3],
    pub visited: Vec<u32>,
    pub truncated: bool,
}

/// `SqrDistancePointSegment2D`: squared x/z distance from `p` to the segment
/// `a`-`b`, and the parameter of the closest point, clamped to [0, 1] with the
/// engine's max-then-min (0 for a zero-length segment).
pub(crate) fn sqr_distance_point_segment_2d(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let pqx = b[0] - a[0];
    let pqz = b[2] - a[2];
    let mut dx = p[0] - a[0];
    let mut dz = p[2] - a[2];
    let d = pqx * pqx + pqz * pqz;
    let mut t = 0.0;
    if d != 0.0 {
        t = engine_fmin(engine_fmax((pqx * dx + pqz * dz) / d, 0.0), 1.0);
        dx = pqx * t - dx;
        dz = pqz * t - dz;
    }
    (dx * dx + dz * dz, t)
}

/// The engine's single-precision max: NaN if either side is NaN, +0 over -0.
fn engine_fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() {
            b
        } else {
            a
        }
    } else if a > b {
        a
    } else {
        b
    }
}

/// The engine's single-precision min: NaN if either side is NaN, -0 under +0.
fn engine_fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        if a.is_sign_negative() {
            a
        } else {
            b
        }
    } else if a < b {
        a
    } else {
        b
    }
}

/// The engine's point-in-polygon test inside `MoveAlongSurface`: for every
/// edge `a`→`b` (the closing edge first), `(p.x−a.x)(b.z−a.z) ≥ (p.z−a.z)(b.x−a.x)`
/// — the interior lies to the right of each edge, and a point on an edge is
/// inside. An unordered comparison does not reject.
fn engine_inside(verts: &[[f32; 3]; 3], p: [f32; 3]) -> bool {
    let (last, first) = (verts[2], verts[0]);
    if (p[0] - last[0]) * (first[2] - last[2]) < (p[2] - last[2]) * (first[0] - last[0]) {
        return false;
    }
    for k in 1..3 {
        let (a, b) = (verts[k - 1], verts[k]);
        if (p[0] - a[0]) * (b[2] - a[2]) < (p[2] - a[2]) * (b[0] - a[0]) {
            return false;
        }
    }
    true
}

/// `NavMeshNodePool` as `MoveAlongSurface` uses it: one node per cell, handed
/// out while the pool has room; the start node is the first.
struct MoveNodes {
    cell: Vec<u32>,
    closed: Vec<bool>,
    parent: Vec<Option<usize>>,
    index: HashMap<u32, usize>,
}

impl MoveNodes {
    fn get(&mut self, cell: u32) -> Option<usize> {
        if let Some(node) = self.index.get(&cell) {
            return Some(*node);
        }
        if self.cell.len() >= MOVE_NODE_POOL {
            return None;
        }
        self.cell.push(cell);
        self.closed.push(false);
        self.parent.push(None);
        self.index.insert(cell, self.cell.len() - 1);
        Some(self.cell.len() - 1)
    }
}

impl PolyMesh {
    /// One cell as the engine's polygon queries read it: the vertices in the
    /// engine's winding (the reverse of this mesh's counter-clockwise order,
    /// so the interior is on the right of each edge), edge `j` running from
    /// vertex `j` to vertex `j + 1`, and each edge's neighbour cell. The
    /// single-layer field has no height, so y is 0; the movement query reads
    /// x and z only, except for its search radius, which takes the two end
    /// points' own heights.
    pub(crate) fn engine_cell(&self, grid: &Grid, index: u32) -> ([[f32; 3]; 3], [u32; 3]) {
        let [a, b, c] = self.tris[index as usize];
        let at = |v: u32| {
            let p = to_world(grid, self.verts[v as usize]);
            [p[0], 0.0, p[1]]
        };
        let n = self.neighbours[index as usize];
        ([at(a), at(c), at(b)], [n[2], n[1], n[0]])
    }

    /// Number of cells.
    pub(crate) fn cell_count(&self) -> usize {
        self.tris.len()
    }

    /// The cell an agent standing at `p` is on: the first cell (in cell order)
    /// whose engine inside test holds, else the cell whose closest point is
    /// nearest within the square of half-width `half_extent` around `p`
    /// (`FindNearestPoly`). No walk-cell gate: an agent lives on the cells.
    pub(crate) fn agent_cell(&self, grid: &Grid, p: [f32; 2], half_extent: f32) -> Option<u32> {
        let point = [p[0], 0.0, p[1]];
        if let Some(inside) = (0..self.tris.len() as u32)
            .find(|index| engine_inside(&self.engine_cell(grid, *index).0, point))
        {
            return Some(inside);
        }
        let mut best: Option<(u32, f32)> = None;
        for index in 0..self.tris.len() as u32 {
            let corners = self.tris[index as usize].map(|v| to_world(grid, self.verts[v as usize]));
            let q = closest_on_triangle(p, corners);
            if (q[0] - p[0]).abs() > half_extent || (q[1] - p[1]).abs() > half_extent {
                continue;
            }
            let d = distance2(q, p);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((index, d));
            }
        }
        best.map(|(index, _)| index)
    }

    /// `NavMeshQuery::MoveAlongSurface` from `start` on cell `start_cell`
    /// towards `end`, on this mesh (one tile without a tile transform; every
    /// cell passes the filter).
    ///
    /// A breadth-first search from the start cell, limited to cells whose
    /// shared edge comes within the circle around the move's midpoint of
    /// radius `|end − start| / 2 + 0.001`. The first expanded cell that
    /// contains `end` ends the search at `end`. Otherwise, per cell, every
    /// edge that put no neighbour into the queue — a wall, or an edge to a
    /// cell already reached or outside the circle — offers its closest point
    /// to `end` (`t·vi + (1−t)·vj`); the strictly nearest one is kept, the
    /// first found on ties. The visited chain runs from the start cell to the
    /// cell the answer lies on, cut at `max_visited` (at least 1).
    pub(crate) fn move_along_surface(
        &self,
        grid: &Grid,
        start_cell: u32,
        start: [f32; 3],
        end: [f32; 3],
        max_visited: usize,
    ) -> SurfaceMove {
        let mut nodes = MoveNodes {
            cell: vec![start_cell],
            closed: vec![true],
            parent: vec![None],
            index: HashMap::from([(start_cell, 0)]),
        };
        let search = [
            start[0] * 0.5 + end[0] * 0.5,
            start[1] * 0.5 + end[1] * 0.5,
            start[2] * 0.5 + end[2] * 0.5,
        ];
        let (dx, dy, dz) = (end[0] - start[0], end[1] - start[1], end[2] - start[2]);
        let radius = (dx * dx + dy * dy + dz * dz).sqrt() * 0.5 + MOVE_SEARCH_PAD;
        let radius_sqr = radius * radius;
        let mut best_position = start;
        let mut best_distance = f32::MAX;
        let mut best_node: Option<usize> = None;
        let mut queue: Vec<usize> = vec![0];
        'search: while !queue.is_empty() {
            let current = queue.remove(0);
            let (verts, neighbours) = self.engine_cell(grid, nodes.cell[current]);
            if engine_inside(&verts, end) {
                best_position = end;
                best_node = Some(current);
                break 'search;
            }
            let mut j = 2;
            for i in 0..3 {
                let (vj, vi) = (verts[j], verts[i]);
                let mut reached = 0;
                let neighbour = neighbours[j];
                if neighbour != NO_NEIGHBOUR {
                    if let Some(node) = nodes.get(neighbour) {
                        if !nodes.closed[node] {
                            let (d, _) = sqr_distance_point_segment_2d(search, vj, vi);
                            if !(d > radius_sqr) {
                                if queue.len() < MOVE_MAX_STACK {
                                    nodes.parent[node] = Some(current);
                                    nodes.closed[node] = true;
                                    queue.push(node);
                                }
                                reached += 1;
                            }
                        }
                    }
                }
                if reached == 0 {
                    let (d, t) = sqr_distance_point_segment_2d(end, vj, vi);
                    if d < best_distance {
                        let u = 1.0 - t;
                        best_position = [
                            t * vi[0] + u * vj[0],
                            vi[1] * t + vj[1] * u,
                            vi[2] * t + vj[2] * u,
                        ];
                        best_distance = d;
                        best_node = Some(current);
                    }
                }
                j = i;
            }
        }
        let mut chain = Vec::new();
        let mut cursor = best_node;
        while let Some(node) = cursor {
            chain.push(nodes.cell[node]);
            cursor = nodes.parent[node];
        }
        chain.reverse();
        let keep = max_visited.max(1);
        let truncated = chain.len() >= keep;
        chain.truncate(keep);
        SurfaceMove {
            position: best_position,
            visited: chain,
            truncated,
        }
    }
}

/// 平面三角形上离 `p` 最近的点：`p` 在三角形内即原样返回，否则取三条边上
/// 最近点中最近者。
fn closest_on_triangle(p: [f32; 2], corners: [[f32; 2]; 3]) -> [f32; 2] {
    let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let s0 = side(corners[0], corners[1]);
    let s1 = side(corners[1], corners[2]);
    let s2 = side(corners[2], corners[0]);
    if (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0) {
        return p;
    }
    let on_segment = |a: [f32; 2], b: [f32; 2]| {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let length2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if length2 > 0.0 {
            (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        [a[0] + ab[0] * t, a[1] + ab[1] * t]
    };
    [
        on_segment(corners[0], corners[1]),
        on_segment(corners[1], corners[2]),
        on_segment(corners[2], corners[0]),
    ]
    .into_iter()
    .min_by(|a, b| distance2(*a, p).total_cmp(&distance2(*b, p)))
    .expect("three candidate points")
}

#[cfg(test)]
mod tests {
    use super::super::contour::{Contour, ContourVert};
    use super::*;

    fn square(region: u32, x0: i32, z0: i32, x1: i32, z1: i32) -> Contour {
        let v = |x, z| ContourVert { x, z, neighbour: 0 };
        Contour {
            verts: vec![v(x0, z0), v(x1, z0), v(x1, z1), v(x0, z1)],
            region,
        }
    }

    /// A 20 x 10 grid, voxel 0.1: region 1 on cells x 0..10, region 2 on
    /// x 10..20. Only region 1 has navigation cells; region 2 is walkable but
    /// was not triangulated (a region can lose its cells to contour
    /// simplification or a failed triangulation).
    fn fixture() -> (Grid, Regions, PolyMesh) {
        let (cols, rows) = (20usize, 10usize);
        let grid = Grid {
            origin: [0.0, 0.0],
            voxel: 0.1,
            cols,
            rows,
            walkable: vec![true; cols * rows],
        };
        let ids = (0..cols * rows)
            .map(|i| if i % cols < 10 { 1 } else { 2 })
            .collect();
        let regions = Regions { ids, max: 3 };
        let mut polys = build(&[square(1, 0, 0, 10, 10)]);
        polys.cache_centres(&grid);
        (grid, regions, polys)
    }

    /// Source rule (NavMeshQuery.FindNearestPoly): every polygon in the query
    /// box competes by closest-point distance, regions play no part, and the
    /// closest point must lie inside the box.
    #[test]
    fn locate_takes_the_nearest_cell_of_any_region_inside_the_query_box() {
        let (grid, regions, polys) = fixture();
        // Region 2 point 0.15 m from region 1's cells: a 0.24 box reaches them.
        let near = [1.15, 0.5];
        let hit = polys
            .locate(&grid, &regions, near, 0.24)
            .expect("nearest cell in the box");
        let corners = polys.tris[hit as usize].map(|v| to_world(&grid, polys.verts[v as usize]));
        let closest = closest_on_triangle(near, corners);
        assert!((distance2(closest, near).sqrt() - 0.15).abs() < 1e-5);
        // The same point with a box narrower than that distance: no cell.
        assert!(polys.locate(&grid, &regions, near, 0.1).is_none());
        // 0.45 m away: the static 0.5 box reaches, the agent 0.24 box does not.
        let far = [1.45, 0.5];
        assert!(polys.locate(&grid, &regions, far, 0.5).is_some());
        assert!(polys.locate(&grid, &regions, far, 0.24).is_none());
        // Containment in the own region is the fast path and wins.
        let inside = [0.35, 0.45];
        let own = polys.locate(&grid, &regions, inside, 0.0).unwrap();
        assert!(polys.contains(&grid, own, inside));
    }

    /// A mesh from recorded parts (vertices, cells, per-edge neighbours in this
    /// module's order); only the cell geometry the movement query reads.
    fn mesh_from_parts(
        verts: Vec<[i32; 2]>,
        tris: Vec<[u32; 3]>,
        neighbours: Vec<[u32; 3]>,
    ) -> PolyMesh {
        let components = vec![0; tris.len()];
        PolyMesh {
            verts,
            tris,
            neighbours,
            by_region: HashMap::new(),
            components,
            centres: Vec::new(),
            centre_grid: None,
        }
    }

    /// Research instrument: `NavMeshQuery::MoveAlongSurface` executed in an
    /// ARMv8 emulator on the current engine library, over navigation meshes
    /// baked by this module and queries of every kind: the approach loop's own
    /// requests, random moves from 0.5 mm to 2.5 m with and without a height
    /// change, vertex and edge starts, zero moves, far ends that exhaust the
    /// node pool, starts off their cell, and visited budgets 0 to 5. Point
    /// MOLY_NAV_MOVE_ROWS at the recorded word rows. Every mesh's engine view
    /// must equal the recorded cells, and every row's answer (three f32
    /// words), visited cells and truncation must match.
    #[test]
    #[ignore = "needs MOLY_NAV_MOVE_ROWS"]
    fn move_along_surface_matches_native_rows() {
        let path = std::env::var("MOLY_NAV_MOVE_ROWS").expect("MOLY_NAV_MOVE_ROWS");
        let text = std::fs::read_to_string(&path).expect("read rows");
        let word = |s: &str| s.parse::<u64>().expect("word") as u32;
        let mut meshes: Vec<(
            Grid,
            Vec<[i32; 2]>,
            Vec<[u32; 3]>,
            Vec<[u32; 3]>,
            Vec<Vec<u32>>,
        )> = Vec::new();
        let mut built: Vec<PolyMesh> = Vec::new();
        let (mut rows, mut cells_checked, mut failures) = (0usize, 0usize, Vec::new());
        let mut families: HashMap<String, (usize, usize)> = HashMap::new();
        let mut sha = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.first().copied() {
                Some("S") => sha = Some(f[1].to_owned()),
                Some("M") => meshes.push((
                    Grid {
                        origin: [f32::from_bits(word(f[2])), f32::from_bits(word(f[3]))],
                        voxel: f32::from_bits(word(f[4])),
                        cols: 0,
                        rows: 0,
                        walkable: Vec::new(),
                    },
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
                Some("V") => meshes
                    .last_mut()
                    .unwrap()
                    .1
                    .push([f[1].parse().unwrap(), f[2].parse().unwrap()]),
                Some("T") => {
                    let m = meshes.last_mut().unwrap();
                    m.2.push([word(f[1]), word(f[2]), word(f[3])]);
                    m.3.push([word(f[4]), word(f[5]), word(f[6])]);
                }
                Some("C") => meshes
                    .last_mut()
                    .unwrap()
                    .4
                    .push(f[1..].iter().map(|w| word(w)).collect()),
                Some("R") => {
                    while built.len() < meshes.len() {
                        let (grid, verts, tris, neighbours, cells) = &meshes[built.len()];
                        let mesh = mesh_from_parts(verts.clone(), tris.clone(), neighbours.clone());
                        for (i, recorded) in cells.iter().enumerate() {
                            let (v, n) = mesh.engine_cell(grid, i as u32);
                            let view: Vec<u32> = v
                                .iter()
                                .flat_map(|p| p.map(f32::to_bits))
                                .chain(n)
                                .collect();
                            assert_eq!(&view, recorded, "engine view of cell {i}");
                            cells_checked += 1;
                        }
                        built.push(mesh);
                    }
                    let family = f[1].to_owned();
                    let m = word(f[2]) as usize;
                    let cell = word(f[3]);
                    let v3 =
                        |k: usize| [word(f[k]), word(f[k + 1]), word(f[k + 2])].map(f32::from_bits);
                    let (start, end) = (v3(4), v3(7));
                    let max_visited = word(f[10]) as usize;
                    let status = word(f[11]);
                    let position = [word(f[12]), word(f[13]), word(f[14])];
                    let count = word(f[15]) as usize;
                    let visited: Vec<u32> = f[16..16 + count].iter().map(|w| word(w)).collect();
                    let got =
                        built[m].move_along_surface(&meshes[m].0, cell, start, end, max_visited);
                    let ok = status & 0x4000_0000 != 0
                        && got.position.map(f32::to_bits) == position
                        && got.visited == visited
                        && got.truncated == (status & 0x10 != 0);
                    let entry = families.entry(family.clone()).or_default();
                    entry.0 += 1;
                    if !ok {
                        entry.1 += 1;
                        if failures.len() < 12 {
                            failures.push(format!(
                                "{family} mesh {m} cell {cell} start {start:?} end {end:?}: native {:?} {visited:?} status {status:#x}, port {:?} {:?} {}",
                                position.map(f32::from_bits),
                                got.position,
                                got.visited,
                                got.truncated
                            ));
                        }
                    }
                    rows += 1;
                }
                _ => {}
            }
        }
        assert_eq!(
            sha.as_deref(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9")
        );
        let mismatched: usize = families.values().map(|(_, bad)| bad).sum();
        let mut summary: Vec<_> = families.iter().collect();
        summary.sort();
        eprintln!(
            "move along surface: {} meshes, {cells_checked} cells, {rows} rows, {mismatched} mismatched; per family {summary:?}",
            meshes.len()
        );
        for failure in &failures {
            eprintln!("  {failure}");
        }
        assert!(rows > 0 && cells_checked > 0);
        assert_eq!(mismatched, 0);
    }
}
