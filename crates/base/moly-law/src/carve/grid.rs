//! 面的烘焙核：三角网栅格化 → 碰撞面 span 合并与过滤 → 从地面选面 →
//! 足迹标 null → 侵蚀 → 小区过滤。
//!
//! 每一步都是引擎原生烘焙链在 2D（单高度面）上的转录，锚点在
//! [`super`] 的模块注释里。本文件只有形状与常数，不出现任何 bevy。

use std::collections::HashMap;

use super::{
    BuildVolume, ColliderPolygon, Obstacle, AGENT_CLIMB, AGENT_HEIGHT, AGENT_SLOPE_DEGREES,
};

/// 烘焙格面：世界 xz 平面上的等距格，`walkable` 是侵蚀与小区过滤都
/// 过完之后的净可行走表。格原点是面三角网包围盒的最小角——引擎侧
/// 高度场的原点取自烘焙体包围盒，同为「任意相位」，足迹对格的相位
/// 误差按一格计（与原生同形状的量化噪声）。
#[derive(Clone)]
pub(crate) struct Grid {
    pub(crate) origin: [f32; 2],
    pub(crate) voxel: f32,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) walkable: Vec<bool>,
}

impl Grid {
    /// 点落在哪一格（可能出界）。
    pub(crate) fn cell_of(&self, x: f32, z: f32) -> (isize, isize) {
        (
            ((x - self.origin[0]) / self.voxel).floor() as isize,
            ((z - self.origin[1]) / self.voxel).floor() as isize,
        )
    }

    /// 格中心的世界位。
    pub(crate) fn cell_center(&self, cx: usize, cz: usize) -> [f32; 2] {
        [
            self.origin[0] + (cx as f32 + 0.5) * self.voxel,
            self.origin[1] + (cz as f32 + 0.5) * self.voxel,
        ]
    }

    fn in_bounds(&self, cx: isize, cz: isize) -> bool {
        cx >= 0 && cz >= 0 && (cx as usize) < self.cols && (cz as usize) < self.rows
    }

    /// 格是否可行走（出界按 false）。
    pub(crate) fn walkable_cell(&self, cx: isize, cz: isize) -> bool {
        self.in_bounds(cx, cz) && self.walkable[cz as usize * self.cols + cx as usize]
    }
}

/// 三角网栅格化：每格按**格中心**的含点判入面（与引擎侧凸多边形标记
/// 的中心采样同形）。逐三角只扫自己的包围盒格，代价按三角面积摊。
///
/// With a build `volume` the grid covers only the part of the faces'
/// bounds inside it: nothing outside the volume is rasterized, and the
/// volume's sides are the grid's outer edge, which the erosion treats as
/// unwalkable like any other edge.
pub(crate) fn rasterize(tris: &[[[f32; 2]; 3]], voxel: f32, volume: Option<BuildVolume>) -> Grid {
    let mut lo = [f32::MAX; 2];
    let mut hi = [-f32::MAX; 2];
    for tri in tris {
        for p in tri {
            for axis in 0..2 {
                assert!(p[axis].is_finite(), "可行走面三角有非有限顶点，烘焙拒绝");
                lo[axis] = lo[axis].min(p[axis]);
                hi[axis] = hi[axis].max(p[axis]);
            }
        }
    }
    assert!(!tris.is_empty(), "可行走面为空，烘焙拒绝");
    assert!(voxel.is_finite() && voxel > 0.0, "voxel 必须为正有限值");
    if let Some(volume) = volume {
        for axis in 0..2 {
            lo[axis] = lo[axis].max(volume.min[axis]);
            hi[axis] = hi[axis].min(volume.max[axis]);
        }
        assert!(
            lo[0] < hi[0] && lo[1] < hi[1],
            "烘焙体积 {volume:?} 与可行走面不相交，烘焙拒绝"
        );
    }
    let cols = (((hi[0] - lo[0]) / voxel).ceil() as usize).max(1);
    let rows = (((hi[1] - lo[1]) / voxel).ceil() as usize).max(1);
    let mut grid = Grid {
        origin: lo,
        voxel,
        cols,
        rows,
        walkable: vec![false; cols * rows],
    };
    for tri in tris {
        let mut tlo = [f32::MAX; 2];
        let mut thi = [-f32::MAX; 2];
        for p in tri {
            for axis in 0..2 {
                tlo[axis] = tlo[axis].min(p[axis]);
                thi[axis] = thi[axis].max(p[axis]);
            }
        }
        let (base_cx, base_cz) = grid.cell_of(tlo[0], tlo[1]);
        let span_x = (((thi[0] - tlo[0]) / voxel).ceil() as isize) + 1;
        let span_z = (((thi[1] - tlo[1]) / voxel).ceil() as isize) + 1;
        for dz in 0..span_z {
            for dx in 0..span_x {
                let (cx, cz) = (base_cx + dx, base_cz + dz);
                if !grid.in_bounds(cx, cz) {
                    continue;
                }
                let center = grid.cell_center(cx as usize, cz as usize);
                if point_in_tri(center, tri) {
                    grid.walkable[cz as usize * cols + cx as usize] = true;
                }
            }
        }
    }
    grid
}

/// 足迹障碍标 null：格中心落在障碍矩形（半开 [min, max)）内即杀。
/// 返回杀掉的可行走格数（账目面：重烘前后可对账）。障碍在面外或
/// 落在已死格上的部分是空操作。
pub(crate) fn mark_obstacles(grid: &mut Grid, obstacles: &[Obstacle]) -> usize {
    let mut nulled = 0;
    for ob in obstacles {
        let (base_cx, base_cz) = grid.cell_of(ob.min[0], ob.min[1]);
        let span_x = (((ob.max[0] - ob.min[0]) / grid.voxel).ceil() as isize) + 2;
        let span_z = (((ob.max[1] - ob.min[1]) / grid.voxel).ceil() as isize) + 2;
        for dz in 0..span_z {
            for dx in 0..span_x {
                let (cx, cz) = (base_cx + dx, base_cz + dz);
                if !grid.in_bounds(cx, cz) {
                    continue;
                }
                let center = grid.cell_center(cx as usize, cz as usize);
                let inside = (center[0] >= ob.min[0])
                    && (center[0] < ob.max[0])
                    && (center[1] >= ob.min[1])
                    && (center[1] < ob.max[1]);
                if inside {
                    let index = cz as usize * grid.cols + cx as usize;
                    if grid.walkable[index] {
                        grid.walkable[index] = false;
                        nulled += 1;
                    }
                }
            }
        }
    }
    nulled
}

/// The walk face's height at a point, when one of its triangles covers it.
/// The same vertical sample the rasterizer uses: the highest covering face.
pub(crate) fn triangle_height(tri: &[[f32; 3]; 3], p: [f32; 2]) -> Option<f32> {
    let [a, b, c] = *tri;
    let determinant = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let u = ((b[2] - c[2]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[2])) / determinant;
    let v = ((c[2] - a[2]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[2])) / determinant;
    (u >= -1e-5 && v >= -1e-5 && u + v <= 1.00001)
        .then(|| u * a[1] + v * b[1] + (1.0 - u - v) * c[1])
}

/// Height is sampled vertically at the exact grid center. No closest-point
/// operation is allowed to move x/z while the collision test retains it.
pub(crate) fn surface_heights(grid: &Grid, triangles: &[[[f32; 3]; 3]]) -> Vec<f32> {
    let mut heights = vec![f32::NEG_INFINITY; grid.walkable.len()];
    for tri in triangles {
        let projected = tri.map(|p| [p[0], p[2]]);
        let min = projected
            .iter()
            .fold([f32::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let max = projected.iter().fold([f32::NEG_INFINITY; 2], |a, p| {
            [a[0].max(p[0]), a[1].max(p[1])]
        });
        let (x0, z0) = grid.cell_of(min[0], min[1]);
        let (x1, z1) = grid.cell_of(max[0], max[1]);
        for z in z0.max(0)..=z1.min(grid.rows as isize - 1) {
            for x in x0.max(0)..=x1.min(grid.cols as isize - 1) {
                let p = grid.cell_center(x as usize, z as usize);
                if let Some(y) = triangle_height(tri, p) {
                    let index = z as usize * grid.cols + x as usize;
                    heights[index] = heights[index].max(y);
                }
            }
        }
    }
    heights
}

/// One entry of a cell column: the vertical interval a source surface covers
/// in it. The span's `max` is its floor (the top face); `min` bounds it from
/// below. The walkable flag is the rasterized face's own, before the column
/// filters.
#[derive(Clone, Copy)]
struct Span {
    min: f32,
    max: f32,
    walkable: bool,
    /// The top face's plane `y = a*x + b*z + c` when a collider produced it;
    /// `None` when the top face is the site walk face.
    surface: Option<[f32; 3]>,
    /// The flood from the ground reached this span.
    reached: bool,
}

impl Span {
    fn site(height: f32) -> Self {
        Self {
            min: height,
            max: height,
            walkable: true,
            surface: None,
            reached: false,
        }
    }
}

/// What the span pipeline resolved for the walk surface: the source triangles
/// kept for exact height reads, the collider plane of every cell whose surface
/// a collider produced, how many cells reached more than one span, and the
/// connection edges the erosion measures from.
pub(crate) struct SurfaceBake {
    /// Cells the explicit carving obstacles removed (the bake's foot null).
    pub(crate) obstacle_nulled: usize,
    /// Walk face triangles, kept to evaluate the site surface exactly.
    pub(crate) site: Vec<[[f32; 3]; 3]>,
    /// Cell index → the collider top face's plane.
    pub(crate) surfaces: HashMap<usize, [f32; 3]>,
    /// Cells where more than one walkable span was reached from the ground;
    /// the highest one became the cell's surface.
    pub(crate) multi_surface: usize,
    /// Walkable cells adjacent to a direction they are not connected to.
    pub(crate) boundary: Vec<bool>,
}

fn triangle_normal(tri: &[[f32; 3]; 3]) -> Option<[f32; 3]> {
    let u = [
        tri[1][0] - tri[0][0],
        tri[1][1] - tri[0][1],
        tri[1][2] - tri[0][2],
    ];
    let v = [
        tri[2][0] - tri[0][0],
        tri[2][1] - tri[0][1],
        tri[2][2] - tri[0][2],
    ];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (length > 1e-12).then(|| [n[0] / length, n[1] / length, n[2] / length])
}

/// The top face's plane, when the face is shallow enough to evaluate at an
/// arbitrary point. A face steeper than about 78 degrees gets a flat top at
/// the cell's own span height instead: an almost vertical plane has no stable
/// value to extrapolate across a cell.
fn surface_plane(tri: &[[f32; 3]; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    let ny = normal[1];
    if ny.abs() < 0.2 {
        return None;
    }
    let a = -normal[0] / ny;
    let b = -normal[2] / ny;
    let c = tri[0][1] - a * tri[0][0] - b * tri[0][2];
    (a.is_finite() && b.is_finite() && c.is_finite()).then_some([a, b, c])
}

fn push_span(columns: &mut HashMap<usize, Vec<Span>>, heights: &[f32], index: usize, span: Span) {
    if !heights[index].is_finite() {
        return;
    }
    columns
        .entry(index)
        .or_insert_with(|| vec![Span::site(heights[index])])
        .push(span);
}

/// Clip one source triangle to every cell column it touches and return the
/// cells' vertical intervals.
fn triangle_cell_intervals(grid: &Grid, tri: &[[f32; 3]; 3]) -> Vec<(usize, [f32; 2])> {
    let mut spans = Vec::new();
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in tri {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[2]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[2]);
    }
    // Retain boundary-touching vertical faces, including both cells when
    // a zero-width wall lies exactly on a raster boundary.
    let epsilon = grid.voxel * 1e-4;
    let (x0, z0) = grid.cell_of(min[0] - epsilon, min[1] - epsilon);
    let (x1, z1) = grid.cell_of(max[0] + epsilon, max[1] + epsilon);
    for z in z0.max(0)..=z1.min(grid.rows as isize - 1) {
        for x in x0.max(0)..=x1.min(grid.cols as isize - 1) {
            let index = z as usize * grid.cols + x as usize;
            if !grid.walkable[index] {
                continue;
            }
            let cell_min = [
                grid.origin[0] + x as f32 * grid.voxel,
                grid.origin[1] + z as f32 * grid.voxel,
            ];
            if let Some(span) = triangle_cell_interval(tri, cell_min, grid.voxel) {
                spans.push((index, span));
            }
        }
    }
    spans
}

/// Clip one source triangle to every cell column it touches and add its
/// interval. The column's site face is inserted first, so the collider surface
/// is placed above or below it by the merge.
fn rasterize_triangle_spans(
    grid: &Grid,
    columns: &mut HashMap<usize, Vec<Span>>,
    heights: &[f32],
    tri: &[[f32; 3]; 3],
    normal: [f32; 3],
    walkable: bool,
) {
    let plane = surface_plane(tri, normal);
    for (index, span) in triangle_cell_intervals(grid, tri) {
        push_span(
            columns,
            heights,
            index,
            Span {
                min: span[0],
                max: span[1],
                walkable,
                surface: Some(plane.unwrap_or([0.0, 0.0, span[1]])),
                reached: false,
            },
        );
    }
}

/// Spans of a source that has no triangle list: the projected hull with its
/// own vertical bounds, one interval per cell inside the hull. Explicit
/// carving is never surface rasterization (see the caller).
fn polygon_spans(grid: &Grid, polygon: &ColliderPolygon) -> Vec<(usize, [f32; 2])> {
    let mut spans = Vec::new();
    if polygon.vertices.is_empty() {
        return spans;
    }
    let min = polygon
        .vertices
        .iter()
        .fold([f32::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
    let max = polygon
        .vertices
        .iter()
        .fold([f32::NEG_INFINITY; 2], |a, p| {
            [a[0].max(p[0]), a[1].max(p[1])]
        });
    let (x0, z0) = grid.cell_of(min[0] - grid.voxel, min[1] - grid.voxel);
    let (x1, z1) = grid.cell_of(max[0] + grid.voxel, max[1] + grid.voxel);
    for z in z0.max(0)..=z1.min(grid.rows as isize - 1) {
        for x in x0.max(0)..=x1.min(grid.cols as isize - 1) {
            let index = z as usize * grid.cols + x as usize;
            if !grid.walkable[index] {
                continue;
            }
            let p = grid.cell_center(x as usize, z as usize);
            let inside = if polygon.vertices.len() < 3 {
                let a = polygon.vertices[0];
                let b = *polygon.vertices.last().unwrap();
                let dx = b[0] - a[0];
                let dz = b[1] - a[1];
                let length = dx * dx + dz * dz;
                let t = if length > 0.0 {
                    (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance2 = (p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dz).powi(2);
                distance2 <= grid.voxel * grid.voxel * 0.5
            } else {
                let mut positive = false;
                let mut negative = false;
                for i in 0..polygon.vertices.len() {
                    let a = polygon.vertices[i];
                    let b = polygon.vertices[(i + 1) % polygon.vertices.len()];
                    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                    positive |= cross > 1e-6;
                    negative |= cross < -1e-6;
                }
                !(positive && negative)
            };
            if inside {
                spans.push((index, [polygon.min_y, polygon.max_y]));
            }
        }
    }
    spans
}

/// Whether two spans in neighbouring cells are connected: the gap between the
/// higher floor and the lower ceiling holds the agent's height, and their
/// floors are within one climb.
fn connected(a_top: f32, a_ceiling: f32, b_top: f32, b_ceiling: f32) -> bool {
    a_ceiling.min(b_ceiling) - a_top.max(b_top) >= AGENT_HEIGHT - 1e-5
        && (a_top - b_top).abs() <= AGENT_CLIMB + 1e-5
}

/// Whether a walkable span is a ledge: a reachable neighbour floor lies more
/// than one climb below it, or the reachable neighbour floors span more than
/// one climb. A reachable neighbour is a span with at least the agent height
/// between the higher floor and the lower ceiling; a missing neighbour column
/// is a drop.
#[allow(clippy::too_many_arguments)]
fn span_is_ledge(
    bot: f32,
    ceiling: f32,
    cx: usize,
    cz: usize,
    near: bool,
    grid: &Grid,
    columns: &HashMap<usize, Vec<Span>>,
    heights: &[f32],
) -> bool {
    let mut lowest_difference = f32::INFINITY;
    let mut lowest = bot;
    let mut highest = bot;
    for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
        let nx = cx as isize + dx;
        let nz = cz as isize + dz;
        if !grid.in_bounds(nx, nz) {
            return true;
        }
        let index = nz as usize * grid.cols + nx as usize;
        if !grid.walkable[index] || !heights[index].is_finite() {
            return true;
        }
        let base = Span::site(heights[index]);
        let spans: &[Span] = if near {
            columns
                .get(&index)
                .map_or_else(|| std::slice::from_ref(&base), |list| list.as_slice())
        } else {
            std::slice::from_ref(&base)
        };
        // The open space between this floor and the neighbour's first span is
        // itself a reachable step down when it is taller than the agent.
        let sentinel = -AGENT_CLIMB;
        let first_top = spans[0].min;
        if ceiling.min(first_top) - bot.max(sentinel) >= AGENT_HEIGHT - 1e-5 {
            lowest_difference = lowest_difference.min(sentinel - bot);
        }
        for (j, span) in spans.iter().enumerate() {
            let next = spans.get(j + 1).map_or(f32::INFINITY, |span| span.min);
            if ceiling.min(next) - bot.max(span.max) >= AGENT_HEIGHT - 1e-5 {
                lowest_difference = lowest_difference.min(span.max - bot);
                if (span.max - bot).abs() <= AGENT_CLIMB {
                    lowest = lowest.min(span.max);
                    highest = highest.max(span.max);
                }
            }
        }
    }
    lowest_difference < -AGENT_CLIMB || highest - lowest > AGENT_CLIMB
}

/// Resolve the non-carving collider sources into the walk surface the way the
/// engine's bake builds its span columns, then carve the explicit obstacles
/// against the selected surface (see the caller). Returns the collision
/// surfaces, the multi-surface count and the erosion edges; `grid.walkable`
/// and `heights` come back holding the chosen surface of every walkable cell.
pub(crate) fn mark_collider_spans(
    grid: &mut Grid,
    heights: &mut [f32],
    polygons: &[ColliderPolygon],
) -> SurfaceBake {
    let cells = grid.cols * grid.rows;
    debug_assert_eq!(heights.len(), cells);
    let slope_cos = AGENT_SLOPE_DEGREES.to_radians().cos();
    let mut columns: HashMap<usize, Vec<Span>> = HashMap::new();
    let mut carving: Vec<(usize, [f32; 2])> = Vec::new();
    for polygon in polygons {
        if polygon.carve {
            // Explicit carving is not a walkable surface: its footprint blocks
            // whatever surface the bake selected, tested below.
            carving.extend(polygon_spans(grid, polygon));
            continue;
        }
        if polygon.triangles.is_empty() {
            // A hull-only source (the legacy projected prism) contributes one
            // solid interval per cell, whatever its own face slopes are.
            for (index, span) in polygon_spans(grid, polygon) {
                push_span(
                    &mut columns,
                    heights,
                    index,
                    Span {
                        min: span[0],
                        max: span[1],
                        walkable: true,
                        surface: Some([0.0, 0.0, span[1]]),
                        reached: false,
                    },
                );
            }
            continue;
        }
        if polygon.solid {
            // A closed convex source fills its interval in every cell it
            // covers: one span per cell from the merged interval, never an
            // empty room inside a box. Its top is taken as flat at the cell's
            // own height, the conservative reading the foot support used.
            let mut per_cell: HashMap<usize, [f32; 2]> = HashMap::new();
            for triangle in &polygon.triangles {
                for (index, interval) in triangle_cell_intervals(grid, triangle) {
                    per_cell
                        .entry(index)
                        .and_modify(|span| {
                            span[0] = span[0].min(interval[0]);
                            span[1] = span[1].max(interval[1]);
                        })
                        .or_insert(interval);
                }
            }
            for (index, span) in per_cell {
                push_span(
                    &mut columns,
                    heights,
                    index,
                    Span {
                        min: span[0],
                        max: span[1],
                        walkable: true,
                        surface: Some([0.0, 0.0, span[1]]),
                        reached: false,
                    },
                );
            }
            continue;
        }
        for triangle in &polygon.triangles {
            let Some(normal) = triangle_normal(triangle) else {
                continue;
            };
            // A triangle is walkable when its up component beats the agent
            // slope's cosine (strictly, as the rasterizer marks it).
            let walkable = normal[1] > slope_cos;
            rasterize_triangle_spans(grid, &mut columns, heights, triangle, normal, walkable);
        }
    }

    // Merge each column the way a heightfield span stack merges: spans that
    // overlap or touch become one; a merge whose tops are within one climb
    // also merges the walkable mark, so a short steep lip on a walkable floor
    // stays walkable. Then a non-walkable span just above a walkable one
    // inherits it when the climb covers the step (the low-hanging filter).
    for list in columns.values_mut() {
        list.sort_by(|a, b| a.min.total_cmp(&b.min).then(a.max.total_cmp(&b.max)));
        let mut merged: Vec<Span> = Vec::with_capacity(list.len());
        for span in list.drain(..) {
            if let Some(last) = merged.last_mut() {
                if span.min <= last.max + 1e-6 {
                    if (span.max - last.max).abs() <= AGENT_CLIMB + 1e-5 {
                        last.walkable |= span.walkable;
                    } else {
                        // Outside the merge threshold the incoming span's own
                        // face decides the merged span's walkability.
                        last.walkable = span.walkable;
                    }
                    if span.max > last.max {
                        last.max = span.max;
                        last.surface = span.surface;
                    }
                    continue;
                }
            }
            merged.push(span);
        }
        let mut previous_walkable = false;
        let mut previous_max = 0.0;
        for span in merged.iter_mut() {
            let original = span.walkable;
            if !original
                && previous_walkable
                && (span.max - previous_max).abs() <= AGENT_CLIMB + 1e-5
            {
                span.walkable = true;
            }
            // The low-hanging rule never chains through two non-walkable spans.
            previous_walkable = original;
            previous_max = span.max;
        }
        *list = merged;
    }

    // Cells next to a column are the only ones whose ledge test has to look at
    // span stacks; everywhere else the site face is the whole neighbour column.
    let mut near = vec![false; cells];
    for &index in columns.keys() {
        near[index] = true;
        let (cx, cz) = (index % grid.cols, index / grid.cols);
        for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let nx = cx as isize + dx;
            let nz = cz as isize + dz;
            if nx >= 0 && nz >= 0 && (nx as usize) < grid.cols && (nz as usize) < grid.rows {
                near[nz as usize * grid.cols + nx as usize] = true;
            }
        }
    }

    // Ledge filter: a walkable span drops away from its reachable neighbour
    // floors on one side, or those floors are spread over more than a climb.
    let mut span_kills: Vec<(usize, usize)> = Vec::new();
    let mut cell_kills: Vec<usize> = Vec::new();
    for index in 0..cells {
        if !grid.walkable[index] {
            continue;
        }
        // A face rasterized at the cell's centre without a height sample (a
        // degenerate triangle) has no surface to stand on.
        if !heights[index].is_finite() {
            cell_kills.push(index);
            continue;
        }
        let (cx, cz) = (index % grid.cols, index / grid.cols);
        if !near[index] {
            // No cell within reach has a collider column: the site face is
            // every neighbour's whole column, so no map lookup is needed.
            if span_is_ledge(
                heights[index],
                f32::INFINITY,
                cx,
                cz,
                false,
                grid,
                &columns,
                heights,
            ) {
                cell_kills.push(index);
            }
            continue;
        }
        match columns.get(&index) {
            Some(list) => {
                for (k, span) in list.iter().enumerate() {
                    if !span.walkable {
                        continue;
                    }
                    let ceiling = list.get(k + 1).map_or(f32::INFINITY, |next| next.min);
                    if span_is_ledge(span.max, ceiling, cx, cz, true, grid, &columns, heights) {
                        span_kills.push((index, k));
                    }
                }
            }
            None => {
                if span_is_ledge(
                    heights[index],
                    f32::INFINITY,
                    cx,
                    cz,
                    true,
                    grid,
                    &columns,
                    heights,
                ) {
                    cell_kills.push(index);
                }
            }
        }
    }
    for (index, k) in span_kills {
        if let Some(span) = columns.get_mut(&index).and_then(|list| list.get_mut(k)) {
            span.walkable = false;
        }
    }
    for index in cell_kills {
        grid.walkable[index] = false;
    }

    // Low-height filter: a walkable span needs the agent's height up to the
    // next span in its own column.
    for list in columns.values_mut() {
        for k in 0..list.len() {
            let ceiling = list.get(k + 1).map_or(f32::INFINITY, |next| next.min);
            if list[k].walkable && ceiling - list[k].max < AGENT_HEIGHT {
                list[k].walkable = false;
            }
        }
    }

    // Select the surface by flooding from the ground: every walkable span
    // within one climb of the site face is a seed (a rug, a low pad, the walk
    // face itself), and connections carry the flood outward (the bridge ramp).
    // Unreached spans (a table top, a lantern cap) are not part of the field.
    let mut seeds: Vec<(usize, usize)> = Vec::new();
    for (index, list) in columns.iter() {
        let height = heights[*index];
        let (cx, cz) = (*index % grid.cols, *index / grid.cols);
        for (k, span) in list.iter().enumerate() {
            if !span.walkable {
                continue;
            }
            let ceiling = list.get(k + 1).map_or(f32::INFINITY, |next| next.min);
            let mut reachable = (span.max - height).abs() <= AGENT_CLIMB + 1e-5;
            if !reachable {
                for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                    let nx = cx as isize + dx;
                    let nz = cz as isize + dz;
                    if !grid.in_bounds(nx, nz) {
                        continue;
                    }
                    let nindex = nz as usize * grid.cols + nx as usize;
                    if !grid.walkable[nindex] || columns.contains_key(&nindex) {
                        continue;
                    }
                    if connected(span.max, ceiling, heights[nindex], f32::INFINITY) {
                        reachable = true;
                        break;
                    }
                }
            }
            if reachable {
                seeds.push((*index, k));
            }
        }
    }
    let mut queue: Vec<(usize, usize)> = Vec::with_capacity(seeds.len());
    for (index, k) in seeds {
        if let Some(span) = columns.get_mut(&index).and_then(|list| list.get_mut(k)) {
            if !span.reached {
                span.reached = true;
                queue.push((index, k));
            }
        }
    }
    while let Some((index, k)) = queue.pop() {
        let (top, ceiling) = {
            let list = &columns[&index];
            let span = &list[k];
            (
                span.max,
                list.get(k + 1).map_or(f32::INFINITY, |next| next.min),
            )
        };
        let (cx, cz) = (index % grid.cols, index / grid.cols);
        let mut found: Vec<(usize, usize)> = Vec::new();
        for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let nx = cx as isize + dx;
            let nz = cz as isize + dz;
            if !grid.in_bounds(nx, nz) {
                continue;
            }
            let nindex = nz as usize * grid.cols + nx as usize;
            if !grid.walkable[nindex] {
                continue;
            }
            let Some(list) = columns.get(&nindex) else {
                continue;
            };
            for (j, span) in list.iter().enumerate() {
                if !span.walkable || span.reached {
                    continue;
                }
                let next = list.get(j + 1).map_or(f32::INFINITY, |next| next.min);
                if connected(top, ceiling, span.max, next) {
                    found.push((nindex, j));
                }
            }
        }
        for (nindex, j) in found {
            if let Some(span) = columns.get_mut(&nindex).and_then(|list| list.get_mut(j)) {
                if !span.reached {
                    span.reached = true;
                    queue.push((nindex, j));
                }
            }
        }
    }

    // The chosen surface per cell: the highest reached span, or no surface at
    // all when nothing was reached. A column that reached several spans is
    // counted; a single-surface host cannot represent more than one.
    let mut surfaces: HashMap<usize, [f32; 3]> = HashMap::new();
    let mut ceilings: HashMap<usize, f32> = HashMap::new();
    let mut multi_surface = 0;
    for (index, list) in columns.iter() {
        let mut best: Option<usize> = None;
        let mut reached = 0;
        for (k, span) in list.iter().enumerate() {
            if !span.walkable || !span.reached {
                continue;
            }
            reached += 1;
            if best.is_none_or(|best| span.max > list[best].max) {
                best = Some(k);
            }
        }
        match best {
            Some(k) => {
                let span = list[k];
                heights[*index] = span.max;
                ceilings.insert(
                    *index,
                    list.get(k + 1).map_or(f32::INFINITY, |next| next.min),
                );
                if let Some(plane) = span.surface {
                    surfaces.insert(*index, plane);
                }
                if reached > 1 {
                    multi_surface += 1;
                }
            }
            None => grid.walkable[*index] = false,
        }
    }

    // Explicit obstacles test their own hull against the surface that was
    // selected, not against the original floor.
    let mut nulled = 0;
    for (index, span) in carving {
        if !grid.walkable[index] {
            continue;
        }
        let surface = heights[index];
        if span[1] > surface + 1e-5 && span[0] < surface + AGENT_HEIGHT {
            grid.walkable[index] = false;
            nulled += 1;
        }
    }

    // Erosion edges come from connectivity, not from a shared walkable flag: a
    // deck cell next to the ground below it is not connected to it even though
    // both cells are walkable, and the agent's radius must not overhang.
    let mut boundary = vec![false; cells];
    for index in 0..cells {
        if !grid.walkable[index] {
            continue;
        }
        let (cx, cz) = (index % grid.cols, index / grid.cols);
        let top = heights[index];
        let near_columns = near[index];
        let ceiling = if near_columns {
            ceilings.get(&index).copied().unwrap_or(f32::INFINITY)
        } else {
            f32::INFINITY
        };
        let mut edge = false;
        for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let nx = cx as isize + dx;
            let nz = cz as isize + dz;
            if !grid.in_bounds(nx, nz) {
                edge = true;
                break;
            }
            let nindex = nz as usize * grid.cols + nx as usize;
            if !grid.walkable[nindex] {
                edge = true;
                break;
            }
            let linked = if near_columns {
                let next = ceilings.get(&nindex).copied().unwrap_or(f32::INFINITY);
                connected(top, ceiling, heights[nindex], next)
            } else {
                // Both columns are site face only: no ceiling, so only a site
                // step can disconnect them.
                (top - heights[nindex]).abs() <= AGENT_CLIMB + 1e-5
            };
            if !linked {
                edge = true;
                break;
            }
        }
        boundary[index] = edge;
    }
    SurfaceBake {
        obstacle_nulled: nulled,
        site: Vec::new(),
        surfaces,
        multi_surface,
        boundary,
    }
}

/// Erosion edges for a field with no collider spans: a walkable cell is an
/// edge when a four-neighbour is not walkable, exactly as before the span
/// pipeline (all its surfaces are the site face, one height per cell).
pub(crate) fn walkable_edges(grid: &Grid) -> Vec<bool> {
    let mut boundary = vec![false; grid.cols * grid.rows];
    for cz in 0..grid.rows as isize {
        for cx in 0..grid.cols as isize {
            let index = cz as usize * grid.cols + cx as usize;
            if !grid.walkable[index] {
                continue;
            }
            boundary[index] = [(cx + 1, cz), (cx - 1, cz), (cx, cz + 1), (cx, cz - 1)]
                .into_iter()
                .any(|(nx, nz)| !grid.walkable_cell(nx, nz));
        }
    }
    boundary
}

/// 侵蚀（`rcErodeWalkableArea` 的 2D 转录）：
/// * 距离场初值：null 格 0，可行走格 255；
/// * 边界标记：`boundary` 里的格距离为 0——它由 span 连接性（或纯站点
///   面的四邻可走性）给出，虚空与障碍同为 0 距离源，外边界同样被侵蚀；
/// * 两遍 chamfer（基数向步长 2、斜向 3，u8 饱和 255）；
/// * 距离 < 2×半径格的可行走格杀掉。
///
/// 返回杀掉的可行走格数（账目面：重烘前后可对账）。
pub(crate) fn erode(grid: &mut Grid, radius_cells: u32, boundary: &[bool]) -> usize {
    let threshold = radius_cells
        .checked_mul(2)
        .filter(|value| *value <= 255)
        .expect("侵蚀半径格数溢出 u8 距离场（voxel 过小），烘焙拒绝");
    let threshold = threshold as u8;
    let (cols, rows) = (grid.cols, grid.rows);
    assert_eq!(boundary.len(), cols * rows, "侵蚀边界表与格面不同尺寸");
    let mut dist = vec![255u8; cols * rows];
    for (index, walkable) in grid.walkable.iter().enumerate() {
        if !walkable || boundary[index] {
            dist[index] = 0;
        }
    }
    // 读邻格距离：出界（虚空）按 0。
    let read = |dist: &[u8], cx: isize, cz: isize| -> u8 {
        if cx >= 0 && cz >= 0 && (cx as usize) < cols && (cz as usize) < rows {
            dist[cz as usize * cols + cx as usize]
        } else {
            0
        }
    };
    let relax = |dist: &mut Vec<u8>, cx: isize, cz: isize, sources: [(isize, isize, u8); 4]| {
        let index = cz as usize * cols + cx as usize;
        let mut best = dist[index];
        for (sx, sz, step) in sources {
            let candidate = read(dist, sx, sz).saturating_add(step).min(255);
            if candidate < best {
                best = candidate;
            }
        }
        dist[index] = best;
    };
    // 第一遍（z 升 x 升）：读西 (+2)、西北 (+3)、北 (+2)、东北 (+3)。
    for cz in 0..rows as isize {
        for cx in 0..cols as isize {
            relax(
                &mut dist,
                cx,
                cz,
                [
                    (cx - 1, cz, 2),
                    (cx - 1, cz - 1, 3),
                    (cx, cz - 1, 2),
                    (cx + 1, cz - 1, 3),
                ],
            );
        }
    }
    // 第二遍（z 降 x 降）：读东 (+2)、东南 (+3)、南 (+2)、西南 (+3)。
    for cz in (0..rows as isize).rev() {
        for cx in (0..cols as isize).rev() {
            relax(
                &mut dist,
                cx,
                cz,
                [
                    (cx + 1, cz, 2),
                    (cx + 1, cz + 1, 3),
                    (cx, cz + 1, 2),
                    (cx - 1, cz + 1, 3),
                ],
            );
        }
    }
    let mut nulled = 0;
    for (index, walkable) in grid.walkable.iter_mut().enumerate() {
        if *walkable && dist[index] < threshold {
            *walkable = false;
            nulled += 1;
        }
    }
    nulled
}

/// 小区过滤（`rcBuildRegions` 的小区规则的 2D 形）：4 连通区域格数
/// 低于阈值的整区杀掉。返回杀掉的格数（账目面）。
///
/// 瓦边豁免不建模：源按瓦（tile）分区、连瓦的区域不杀（其真实面积
/// 跨瓦无法估计）；本烘焙是单张整场格，没有瓦缝，具名差异记录在
/// [`super`] 模块注释。
pub(crate) fn filter_regions(grid: &mut Grid, min_spans: usize) -> usize {
    let (cols, rows) = (grid.cols, grid.rows);
    let total = cols * rows;
    let mut label: Vec<usize> = vec![usize::MAX; total];
    let mut sizes: Vec<usize> = Vec::new();
    for seed in 0..total {
        if !grid.walkable[seed] || label[seed] != usize::MAX {
            continue;
        }
        let id = sizes.len();
        let mut size = 0;
        let mut stack = vec![seed];
        label[seed] = id;
        while let Some(index) = stack.pop() {
            size += 1;
            let (cx, cz) = (index % cols, index / cols);
            for (nx, nz) in [
                (cx + 1, cz),
                (cx.wrapping_sub(1), cz),
                (cx, cz + 1),
                (cx, cz.wrapping_sub(1)),
            ] {
                if nx >= cols || nz >= rows {
                    continue;
                }
                let next = nz * cols + nx;
                if grid.walkable[next] && label[next] == usize::MAX {
                    label[next] = id;
                    stack.push(next);
                }
            }
        }
        sizes.push(size);
    }
    let mut nulled = 0;
    for index in 0..total {
        if grid.walkable[index] && sizes[label[index]] < min_spans {
            grid.walkable[index] = false;
            nulled += 1;
        }
    }
    nulled
}

/// Sutherland-Hodgman against the four vertical cell planes, retaining the Y
/// coordinate at every cut. Stack buffers avoid a heap allocation per voxel.
fn triangle_cell_interval(tri: &[[f32; 3]; 3], min: [f32; 2], size: f32) -> Option<[f32; 2]> {
    let mut input = [[0.0; 3]; 12];
    input[..3].copy_from_slice(tri);
    let mut len = 3;
    for (axis, bound, sign) in [
        (0, min[0], 1.0),
        (0, min[0] + size, -1.0),
        (2, min[1], 1.0),
        (2, min[1] + size, -1.0),
    ] {
        let mut output = [[0.0; 3]; 12];
        let mut count = 0;
        let mut previous = input[len - 1];
        let mut prior_distance = (previous[axis] - bound) * sign;
        for current in input[..len].iter().copied() {
            let distance = (current[axis] - bound) * sign;
            if (distance >= 0.0) != (prior_distance >= 0.0) {
                let t = prior_distance / (prior_distance - distance);
                output[count] =
                    std::array::from_fn(|i| previous[i] + (current[i] - previous[i]) * t);
                count += 1;
            }
            if distance >= 0.0 {
                output[count] = current;
                count += 1;
            }
            previous = current;
            prior_distance = distance;
        }
        if count == 0 {
            return None;
        }
        input = output;
        len = count;
    }
    Some(
        input[..len]
            .iter()
            .fold([f32::INFINITY, f32::NEG_INFINITY], |a, p| {
                [a[0].min(p[1]), a[1].max(p[1])]
            }),
    )
}

/// 三角形含点（符号法，顺逆时针皆收；退化三角形自然为假）。
fn point_in_tri(p: [f32; 2], tri: &[[f32; 2]; 3]) -> bool {
    let mut sign = 0.0f32;
    for edge in 0..3 {
        let a = tri[edge];
        let b = tri[(edge + 1) % 3];
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross != 0.0 {
            if sign == 0.0 {
                sign = cross;
            } else if (cross > 0.0) != (sign > 0.0) {
                return false;
            }
        }
    }
    true
}
