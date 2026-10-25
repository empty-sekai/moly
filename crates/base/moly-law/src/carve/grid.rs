//! 面的烘焙核：三角网栅格化 → 碰撞面 span 合并与过滤 → 从地面选面 →
//! 足迹标 null → 侵蚀 → 小区过滤。
//!
//! 每一步都是引擎原生烘焙链在 2D（单高度面）上的转录，锚点在
//! [`super`] 的模块注释里。本文件只有形状与常数，不出现任何 bevy。

use super::{
    BuildVolume, ColliderPolygon, Obstacle, SurfaceBake, AGENT_CLIMB, AGENT_HEIGHT,
    AGENT_SLOPE_DEGREES,
};

/// Every cell's span column, compressed by cell: the spans of all columns in
/// one flat vector, and for every cell where its column starts there. The
/// columns are packed back to back in cell order, so cell `i`'s column ends
/// where the next cell's begins; a cell without a column has an empty range,
/// and its start is the same as its predecessor's (or zero).
///
/// A column that is merged or has spans killed in it shortens: its live
/// length is the per-cell `u32` length the store's owner keeps, and the slots
/// it frees up stay at the end of its range, unused.
pub(crate) struct CellColumns {
    start: Vec<u32>,
    spans: Vec<Span>,
}

impl CellColumns {
    pub(crate) fn cells(&self) -> usize {
        self.start.len() - 1
    }

    /// The column of `index`, `None` when it has none.
    pub(crate) fn get(&self, index: usize, len: &[u32]) -> Option<&[Span]> {
        let live = len[index] as usize;
        (live > 0).then(|| &self.area(index)[..live])
    }

    /// The column of `index`, `None` when it has none.
    pub(crate) fn get_mut<'a>(&'a mut self, index: usize, len: &[u32]) -> Option<&'a mut [Span]> {
        let live = len[index] as usize;
        let at = self.start[index] as usize;
        (live > 0).then(|| &mut self.spans[at..at + live])
    }

    /// Whether `index` has a column (its live length is above zero).
    pub(crate) fn has(&self, index: usize, len: &[u32]) -> bool {
        len[index] > 0
    }

    /// The cells with a column, in ascending cell order, with their spans.
    pub(crate) fn columns<'a>(
        &'a self,
        len: &'a [u32],
    ) -> impl Iterator<Item = (usize, &'a [Span])> + 'a {
        (0..self.cells()).filter_map(move |index| {
            let live = len[index] as usize;
            let at = self.start[index] as usize;
            (live > 0).then(|| (index, &self.spans[at..at + live]))
        })
    }

    /// The slots reserved for `index`'s column.
    fn area(&self, index: usize) -> &[Span] {
        &self.spans[self.start[index] as usize..self.start[index + 1] as usize]
    }
}

/// Gather the spans of consecutive bands into cell columns. `parts` are the
/// spans each band produced, in that band's own order; cell `c` holds, in
/// order, the site face sampled from `site` at `c` and then the band spans
/// gathered for it. The bands cover disjoint cells, so a stable counting sort
/// by cell reproduces the sequential rasterization's order.
fn gather_columns(cells: usize, site: &[f32], parts: &[ColliderRaster]) -> (CellColumns, Vec<u32>) {
    let mut count = vec![0u32; cells];
    for part in parts {
        for &(cell, _) in &part.spans {
            count[cell as usize] += 1;
        }
    }
    let mut len = vec![0u32; cells];
    let mut start = vec![0u32; cells + 1];
    let mut total = 0u32;
    for (cell, &count) in count.iter().enumerate() {
        if count > 0 {
            len[cell] = count + 1;
            total += count + 1;
        }
        start[cell + 1] = total;
    }
    let mut spans = vec![Span::site(0.0); total as usize];
    // The slot each cell's next span goes to; the site face takes the first.
    let mut fill = vec![0u32; cells];
    for cell in 0..cells {
        if start[cell + 1] > start[cell] {
            spans[start[cell] as usize] = Span::site(site[cell]);
            fill[cell] = start[cell] + 1;
        }
    }
    for part in parts {
        for &(cell, span) in &part.spans {
            let at = fill[cell as usize] as usize;
            spans[at] = span;
            fill[cell as usize] += 1;
        }
    }
    debug_assert_eq!(start[cells], total);
    (CellColumns { start, spans }, len)
}

/// What the bake reads back from the span pipeline besides the walkable mask:
/// the collider plane that produced a cell's surface (no plane elsewhere) and
/// the ceiling of the span the ground flood chose (no ceiling elsewhere).
pub(crate) struct SpanTables {
    /// The plane table index of every cell, `u32::MAX` when its surface is
    /// not a collider's.
    cell_type: Vec<u32>,
    planes: Vec<[f32; 3]>,
    /// The table index of every plane, by its three f32 bit patterns.
    plane_ids: std::collections::HashMap<[u32; 3], u32>,
    ceilings: Vec<f32>,
}

impl SpanTables {
    fn new(cells: usize) -> Self {
        Self {
            cell_type: vec![u32::MAX; cells],
            planes: Vec::new(),
            plane_ids: std::collections::HashMap::new(),
            ceilings: vec![f32::INFINITY; cells],
        }
    }

    fn set_ceiling(&mut self, index: usize, ceiling: f32) {
        self.ceilings[index] = ceiling;
    }

    /// Give `index` its plane, stored among the existing planes when the same
    /// plane (the same three f32 bits) is already there.
    fn set_plane(&mut self, index: usize, plane: [f32; 3]) {
        let bits = plane.map(f32::to_bits);
        let planes = &mut self.planes;
        let id = *self.plane_ids.entry(bits).or_insert_with(|| {
            planes.push(plane);
            (planes.len() - 1) as u32
        });
        self.cell_type[index] = id;
    }
}

/// The collider planes that give a cell's surface, with the table index of
/// every cell's plane (`u32::MAX` where the surface is not a collider's).
/// Cloning shares the arrays.
#[derive(Clone)]
pub(crate) struct SurfacePlanes {
    cell_type: std::sync::Arc<[u32]>,
    planes: std::sync::Arc<[[f32; 3]]>,
}

impl SurfacePlanes {
    fn new(tables: &SpanTables) -> Self {
        Self {
            cell_type: tables.cell_type.clone().into(),
            planes: tables.planes.clone().into(),
        }
    }

    /// Nothing: no cell's surface is a collider's.
    pub(crate) fn none() -> Self {
        Self {
            cell_type: Vec::new().into(),
            planes: Vec::new().into(),
        }
    }

    /// The plane of a cell whose surface a collider produced.
    pub(crate) fn plane(&self, index: usize) -> Option<[f32; 3]> {
        let id = *self.cell_type.get(index)?;
        (id != u32::MAX).then(|| self.planes[id as usize])
    }
}

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
pub(crate) struct Span {
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
    /// The site walk face as a column's base span, at the sampled height
    /// `height` of its cell.
    pub(crate) fn site(height: f32) -> Self {
        Self {
            min: height,
            max: height,
            walkable: true,
            surface: None,
            reached: false,
        }
    }
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

/// The rows one rasterizing worker visits, `start..=end`. Rows are the unit
/// of the split: every cell of a row is visited by the same worker.
#[derive(Clone, Copy)]
struct Band {
    start: usize,
    end: usize,
}

impl Band {
    /// The band's rows. A band is never empty (see [`row_bands`]), and it
    /// lies inside the grid's rows.
    fn rows(&self) -> core::ops::RangeInclusive<usize> {
        self.start..=self.end
    }
}

/// Contiguous bands over `rows` grid rows, one per worker and at least one
/// row each: every band but the last holds `rows / workers` rows.
fn row_bands(rows: usize, workers: usize) -> Vec<Band> {
    let workers = workers.clamp(1, rows.max(1));
    let per = rows / workers;
    let mut bands = Vec::with_capacity(workers);
    let mut start = 0;
    for worker in 0..workers {
        let len = if worker + 1 == workers { rows - start } else { per };
        bands.push(Band {
            start,
            end: start + len - 1,
        });
        start += len;
    }
    bands
}

/// Visit every cell column the triangle touches whose row lies in `rows`,
/// with the triangle's vertical interval inside it. The caller decides what
/// the interval yields; nothing is allocated per triangle.
fn for_each_triangle_cell(
    grid: &Grid,
    tri: &[[f32; 3]; 3],
    rows: core::ops::RangeInclusive<isize>,
    mut visit: impl FnMut(usize, [f32; 2]),
) {
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
    let lo = z0.max(*rows.start());
    let hi = z1.min(*rows.end());
    if lo > hi {
        return;
    }
    for z in lo..=hi {
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
                visit(index, span);
            }
        }
    }
}

/// Clip one source triangle to every cell column it touches (rows in `rows`)
/// and append its interval.
#[allow(clippy::too_many_arguments)]
fn rasterize_triangle_spans(
    grid: &Grid,
    spans: &mut Vec<(u32, Span)>,
    heights: &[f32],
    tri: &[[f32; 3]; 3],
    normal: [f32; 3],
    walkable: bool,
    rows: core::ops::RangeInclusive<isize>,
) {
    let plane = surface_plane(tri, normal);
    for_each_triangle_cell(grid, tri, rows, |index, span| {
        if !heights[index].is_finite() {
            return;
        }
        spans.push((
            index as u32,
            Span {
                min: span[0],
                max: span[1],
                walkable,
                surface: Some(plane.unwrap_or([0.0, 0.0, span[1]])),
                reached: false,
            },
        ));
    });
}

/// Spans of a source that has no triangle list: the projected hull with its
/// own vertical bounds, one interval per cell inside the hull, for the cells
/// whose row lies in `rows`. Explicit carving is never surface rasterization
/// (see the caller).
fn polygon_spans(
    grid: &Grid,
    polygon: &ColliderPolygon,
    rows: core::ops::RangeInclusive<isize>,
) -> Vec<(usize, [f32; 2])> {
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
    let lo = z0.max(*rows.start());
    let hi = z1.min(*rows.end());
    if lo > hi {
        return spans;
    }
    for z in lo..=hi {
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
    near: &[bool],
    grid: &Grid,
    columns: &CellColumns,
    lengths: &[u32],
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
        // Only a cell next to a column has a span stack of its own; every
        // other cell's whole column is its site face.
        let spans: &[Span] = if near[index] {
            columns
                .get(index, lengths)
                .map_or_else(|| std::slice::from_ref(&base), |list| list)
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

/// [`span_is_ledge`] for a cell whose own column and whose four neighbour
/// columns are the site face alone: every neighbour span is one base span with
/// an unbounded ceiling, so the verdict is a handful of height comparisons and
/// no span stack is built.
fn site_span_is_ledge(bot: f32, cx: usize, cz: usize, grid: &Grid, heights: &[f32]) -> bool {
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
        let neighbour = heights[index];
        // The space below the neighbour's base span, then the base span itself
        // (its ceiling is unbounded, so that gap always qualifies).
        if neighbour - bot.max(-AGENT_CLIMB) >= AGENT_HEIGHT - 1e-5 {
            lowest_difference = lowest_difference.min(-AGENT_CLIMB - bot);
        }
        lowest_difference = lowest_difference.min(neighbour - bot);
        if (neighbour - bot).abs() <= AGENT_CLIMB {
            lowest = lowest.min(neighbour);
            highest = highest.max(neighbour);
        }
    }
    lowest_difference < -AGENT_CLIMB || highest - lowest > AGENT_CLIMB
}

/// Whether the chosen span of a walkable cell is disconnected from a walkable
/// neighbour's chosen span. The erosion's own walkability boundary covers
/// neighbours that are out of bounds or not walkable; this is the part the
/// shared walkable flag cannot express.
fn span_is_disconnected(
    index: usize,
    cx: usize,
    cz: usize,
    near: &[bool],
    grid: &Grid,
    ceilings: &[f32],
    heights: &[f32],
) -> bool {
    let top = heights[index];
    let ceiling = if near[index] {
        ceilings[index]
    } else {
        f32::INFINITY
    };
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
        // A cell away from every column has no ceiling entry.
        let next = if near[nindex] {
            ceilings[nindex]
        } else {
            f32::INFINITY
        };
        if !connected(top, ceiling, heights[nindex], next) {
            return true;
        }
    }
    false
}

/// What one rasterizing band produced: the collider spans it appended to
/// cells, `(cell, span)` in the order it appended them, and the explicit
/// carving footprints it found. The spans become cell columns with
/// [`gather_columns`].
pub(crate) struct ColliderRaster {
    spans: Vec<(u32, Span)>,
    carving: Vec<(usize, [f32; 2])>,
}

/// Collider spans and carving footprints of `polygons` for the cells whose
/// row lies in `rows`. Polygons, and the triangles within a polygon, are
/// walked in their own order, so a cell's spans are appended in the same
/// order whatever the row range.
pub(crate) fn rasterize_colliders(
    grid: &Grid,
    heights: &[f32],
    polygons: &[ColliderPolygon],
    rows: core::ops::RangeInclusive<usize>,
) -> ColliderRaster {
    let rows = *rows.start() as isize..=*rows.end() as isize;
    let slope_cos = AGENT_SLOPE_DEGREES.to_radians().cos();
    let mut spans: Vec<(u32, Span)> = Vec::new();
    let mut carving: Vec<(usize, [f32; 2])> = Vec::new();
    for polygon in polygons {
        if polygon.carve {
            // Explicit carving is not a walkable surface: its footprint blocks
            // whatever surface the bake selected, tested below.
            carving.extend(polygon_spans(grid, polygon, rows.clone()));
            continue;
        }
        if polygon.triangles.is_empty() {
            // A hull-only source (the legacy projected prism) contributes one
            // solid interval per cell, whatever its own face slopes are.
            for (index, span) in polygon_spans(grid, polygon, rows.clone()) {
                if !heights[index].is_finite() {
                    continue;
                }
                spans.push((
                    index as u32,
                    Span {
                        min: span[0],
                        max: span[1],
                        walkable: true,
                        surface: Some([0.0, 0.0, span[1]]),
                        reached: false,
                    },
                ));
            }
            continue;
        }
        if polygon.solid {
            // A closed convex source fills its interval in every cell it
            // covers: one span per cell from the merged interval, never an
            // empty room inside a box. Its top is taken as flat at the cell's
            // own height, the conservative reading the foot support used.
            let mut intervals: Vec<(u32, [f32; 2])> = Vec::new();
            for triangle in &polygon.triangles {
                for_each_triangle_cell(grid, triangle, rows.clone(), |index, interval| {
                    intervals.push((index as u32, interval));
                });
            }
            intervals.sort_unstable_by_key(|(index, _)| *index);
            let mut at = 0;
            while at < intervals.len() {
                let index = intervals[at].0 as usize;
                let mut span = intervals[at].1;
                at += 1;
                while at < intervals.len() && intervals[at].0 as usize == index {
                    span[0] = span[0].min(intervals[at].1[0]);
                    span[1] = span[1].max(intervals[at].1[1]);
                    at += 1;
                }
                if !heights[index].is_finite() {
                    continue;
                }
                spans.push((
                    index as u32,
                    Span {
                        min: span[0],
                        max: span[1],
                        walkable: true,
                        surface: Some([0.0, 0.0, span[1]]),
                        reached: false,
                    },
                ));
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
            rasterize_triangle_spans(
                grid,
                &mut spans,
                heights,
                triangle,
                normal,
                walkable,
                rows.clone(),
            );
        }
    }
    ColliderRaster { spans, carving }
}

/// What the banded rasterization produced, one part per band in band order,
/// and how many cells the bake covers.
pub(crate) struct BandRaster {
    parts: Vec<ColliderRaster>,
    cells: usize,
}

impl BandRaster {
    /// The cell columns from the band parts, cells in ascending order, and the
    /// explicit carving footprints in band order.
    pub(crate) fn columns(&self, site: &[f32]) -> (CellColumns, Vec<u32>) {
        gather_columns(self.cells, site, &self.parts)
    }

    /// The explicit carving footprints, in band order.
    pub(crate) fn carving(&self) -> Vec<(usize, [f32; 2])> {
        self.parts.iter().flat_map(|part| part.carving.iter().copied()).collect()
    }
}

/// [`rasterize_colliders`] split into contiguous row bands, one per worker.
/// One worker, or a grid with fewer rows than workers, runs the single band of
/// every row on this thread.
pub(crate) fn rasterize_collider_bands(
    grid: &Grid,
    heights: &[f32],
    polygons: &[ColliderPolygon],
    workers: usize,
) -> BandRaster {
    let bands = row_bands(grid.rows, workers);
    let parts = if bands.len() == 1 {
        vec![rasterize_colliders(grid, heights, polygons, bands[0].rows())]
    } else {
        run_bands(grid, heights, polygons, &bands)
    };
    BandRaster {
        parts,
        cells: grid.cols * grid.rows,
    }
}

/// Work every band on its own thread where threads exist, and return the
/// parts in band order.
#[cfg(not(target_arch = "wasm32"))]
fn run_bands(
    grid: &Grid,
    heights: &[f32],
    polygons: &[ColliderPolygon],
    bands: &[Band],
) -> Vec<ColliderRaster> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = bands
            .iter()
            .map(|band| {
                let rows = band.rows();
                scope.spawn(move || rasterize_colliders(grid, heights, polygons, rows))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| match handle.join() {
                Ok(part) => part,
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    })
}

/// Without threads the bands run one after another on this thread.
#[cfg(target_arch = "wasm32")]
fn run_bands(
    grid: &Grid,
    heights: &[f32],
    polygons: &[ColliderPolygon],
    bands: &[Band],
) -> Vec<ColliderRaster> {
    bands
        .iter()
        .map(|band| rasterize_colliders(grid, heights, polygons, band.rows()))
        .collect()
}

/// Merge one cell column the way a heightfield span stack merges: spans that
/// overlap or touch become one; a merge whose tops are within one climb also
/// merges the walkable mark, so a short steep lip on a walkable floor stays
/// walkable. Then a non-walkable span just above a walkable one inherits it
/// when the climb covers the step (the low-hanging filter).
fn merge_span_column(list: &mut [Span]) -> u32 {
    list.sort_by(|a, b| a.min.total_cmp(&b.min).then(a.max.total_cmp(&b.max)));
    // Merge in place: `write` never overtakes `read`, so each merged span
    // can be stored where the first span of its group was.
    let mut write = 0;
    for read in 0..list.len() {
        let span = list[read];
        if write > 0 {
            let last = &mut list[write - 1];
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
        list[write] = span;
        write += 1;
    }
    let list = &mut list[..write];
    let mut previous_walkable = false;
    let mut previous_max = 0.0;
    for span in list.iter_mut() {
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
    write as u32
}

/// Resolve the non-carving collider sources already rasterized by
/// [`rasterize_collider_bands`] into the walk surface the way the engine's
/// bake builds its span columns, then carve the explicit obstacles against
/// the selected surface (see the caller). Returns the collision surfaces, the
/// multi-surface count and the erosion edges; `grid.walkable` and `heights`
/// come back holding the chosen surface of every walkable cell.
pub(crate) fn mark_collider_spans(
    grid: &mut Grid,
    heights: &mut [f32],
    raster: BandRaster,
) -> SurfaceBake {
    let cells = grid.cols * grid.rows;
    debug_assert_eq!(heights.len(), cells);
    let (mut columns, mut lengths) = raster.columns(heights);
    let carving = raster.carving();
    // Merge each column as a heightfield span stack; a column whose site has
    // no height sample is all site face.
    for index in 0..cells {
        if lengths[index] == 0 {
            continue;
        }
        if heights[index].is_finite() {
            if let Some(slots) = columns.get_mut(index, &lengths) {
                lengths[index] = merge_span_column(slots);
            }
        } else {
            lengths[index] = 1;
        }
    }

    // Cells next to a column are the only ones whose ledge test has to look at
    // span stacks; everywhere else the site face is the whole neighbour column.
    let mut near = vec![false; cells];
    for index in 0..cells {
        if lengths[index] == 0 {
            continue;
        }
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
    // Range of the sampled site heights over the walkable cells, needed later
    // to know whether untouched cells can have a site step at all.
    let (mut site_min, mut site_max) = (f32::INFINITY, f32::NEG_INFINITY);
    for cz in 0..grid.rows {
        for cx in 0..grid.cols {
            let index = cz * grid.cols + cx;
            if !grid.walkable[index] {
                continue;
            }
            let height = heights[index];
            // A face rasterized at the cell's centre without a height sample
            // (a degenerate triangle) has no surface to stand on.
            if !height.is_finite() {
                cell_kills.push(index);
                continue;
            }
            site_min = site_min.min(height);
            site_max = site_max.max(height);
            if !near[index] {
                // No cell within reach has a collider column: the site face is
                // every neighbour's whole column.
                if site_span_is_ledge(height, cx, cz, grid, heights) {
                    cell_kills.push(index);
                }
                continue;
            }
            match columns.get(index, &lengths) {
                Some(list) => {
                    for (k, span) in list.iter().enumerate() {
                        if !span.walkable {
                            continue;
                        }
                        let ceiling = list.get(k + 1).map_or(f32::INFINITY, |next| next.min);
                        if span_is_ledge(
                            span.max,
                            ceiling,
                            cx,
                            cz,
                            &near,
                            grid,
                            &columns,
                            &lengths,
                            heights,
                        ) {
                            span_kills.push((index, k));
                        }
                    }
                }
                None => {
                    if span_is_ledge(
                        height,
                        f32::INFINITY,
                        cx,
                        cz,
                        &near,
                        grid,
                        &columns,
                        &lengths,
                        heights,
                    ) {
                        cell_kills.push(index);
                    }
                }
            }
        }
    }
    for (index, k) in span_kills {
        if let Some(span) = columns.get_mut(index, &lengths).and_then(|list| list.get_mut(k)) {
            span.walkable = false;
        }
    }
    for index in cell_kills {
        grid.walkable[index] = false;
    }

    // Low-height filter: a walkable span needs the agent's height up to the
    // next span in its own column.
    for index in 0..cells {
        if let Some(list) = columns.get_mut(index, &lengths) {
            for k in 0..list.len() {
                let ceiling = list.get(k + 1).map_or(f32::INFINITY, |next| next.min);
                if list[k].walkable && ceiling - list[k].max < AGENT_HEIGHT {
                    list[k].walkable = false;
                }
            }
        }
    }

    // Select the surface by flooding from the ground: every walkable span
    // within one climb of the site face is a seed (a rug, a low pad, the walk
    // face itself), and connections carry the flood outward (the bridge ramp).
    // Unreached spans (a table top, a lantern cap) are not part of the field.
    let mut seeds: Vec<(usize, usize)> = Vec::new();
    for (index, list) in columns.columns(&lengths) {
        let height = heights[index];
        let (cx, cz) = (index % grid.cols, index / grid.cols);
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
                    // A cell away from every column has no span stack of its
                    // own; only a cell next to one needs its column.
                    if !grid.walkable[nindex] || (near[nindex] && columns.has(nindex, &lengths)) {
                        continue;
                    }
                    if connected(span.max, ceiling, heights[nindex], f32::INFINITY) {
                        reachable = true;
                        break;
                    }
                }
            }
            if reachable {
                seeds.push((index, k));
            }
        }
    }
    let mut queue: Vec<(usize, usize)> = Vec::with_capacity(seeds.len());
    for (index, k) in seeds {
        if let Some(span) = columns.get_mut(index, &lengths).and_then(|list| list.get_mut(k)) {
            if !span.reached {
                span.reached = true;
                queue.push((index, k));
            }
        }
    }
    let mut found: Vec<(usize, usize)> = Vec::new();
    while let Some((index, k)) = queue.pop() {
        let (top, ceiling) = {
            let list = columns.get(index, &lengths).expect("a reached span's cell has a column");
            let span = &list[k];
            (
                span.max,
                list.get(k + 1).map_or(f32::INFINITY, |next| next.min),
            )
        };
        let (cx, cz) = (index % grid.cols, index / grid.cols);
        found.clear();
        for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let nx = cx as isize + dx;
            let nz = cz as isize + dz;
            if !grid.in_bounds(nx, nz) {
                continue;
            }
            let nindex = nz as usize * grid.cols + nx as usize;
            // A cell away from every column has no span stack to reach into.
            if !grid.walkable[nindex] || !near[nindex] {
                continue;
            }
            let Some(list) = columns.get(nindex, &lengths) else {
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
        for &(nindex, j) in &found {
            if let Some(span) = columns
                .get_mut(nindex, &lengths)
                .and_then(|list| list.get_mut(j))
            {
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
    let mut tables = SpanTables::new(cells);
    let mut multi_surface = 0;
    for (index, list) in columns.columns(&lengths) {
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
                heights[index] = span.max;
                tables.set_ceiling(
                    index,
                    list.get(k + 1).map_or(f32::INFINITY, |next| next.min),
                );
                if let Some(plane) = span.surface {
                    tables.set_plane(index, plane);
                }
                if reached > 1 {
                    multi_surface += 1;
                }
            }
            None => grid.walkable[index] = false,
        }
    }

    // Explicit obstacles test their own hull against the surface that was
    // selected, not against the original floor; the carving spans come in
    // band order after a banded rasterization, an order the cell count does not
    // depend on.
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

    // Span-connection edges the walkable mask cannot express: a deck cell next
    // to the ground below it is walkable on both sides but not connected, and
    // the agent's radius must not overhang. Every other edge (an out-of-bounds
    // or unwalkable neighbour) is the erosion's own walkability boundary.
    //
    // A cell whose own column and whose four neighbour columns are the site
    // face alone can only disconnect on a site step. When the sampled site
    // heights span at most one climb there is no such step, so only cells next
    // to a collider column need the check.
    let mut edges: Vec<u32> = Vec::new();
    // The sparse walk is only worth it when the near region is smaller than
    // the grid; otherwise the plain scan is cheaper and equally correct.
    let columns_with_spans = lengths.iter().filter(|len| **len > 0).count();
    if site_max - site_min <= AGENT_CLIMB && columns_with_spans.saturating_mul(5) < cells {
        for column in 0..cells {
            if lengths[column] == 0 {
                continue;
            }
            let (cx, cz) = (column % grid.cols, column / grid.cols);
            if grid.walkable[column]
                && span_is_disconnected(column, cx, cz, &near, grid, &tables.ceilings, heights)
            {
                edges.push(column as u32);
            }
            for (dx, dz) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                let nx = cx as isize + dx;
                let nz = cz as isize + dz;
                if nx < 0 || nz < 0 || (nx as usize) >= grid.cols || (nz as usize) >= grid.rows {
                    continue;
                }
                let nindex = nz as usize * grid.cols + nx as usize;
                if grid.walkable[nindex]
                    && span_is_disconnected(
                        nindex,
                        nx as usize,
                        nz as usize,
                        &near,
                        grid,
                        &tables.ceilings,
                        heights,
                    )
                {
                    edges.push(nindex as u32);
                }
            }
        }
    } else {
        for cz in 0..grid.rows {
            for cx in 0..grid.cols {
                let index = cz * grid.cols + cx;
                if grid.walkable[index]
                    && span_is_disconnected(index, cx, cz, &near, grid, &tables.ceilings, heights)
                {
                    edges.push(index as u32);
                }
            }
        }
    }
    SurfaceBake {
        obstacle_nulled: nulled,
        site: Vec::new(),
        surfaces: SurfacePlanes::new(&tables),
        multi_surface,
        edges,
    }
}

/// 侵蚀（`rcErodeWalkableArea` 的 2D 转录）：
/// * 距离场初值：null 格 0，可行走格 255；
/// * 边界标记：可行走格的四邻有 null **或出界（虚空）** → 距离 0 ——
///   虚空与障碍同为 0 距离源，外边界同样被侵蚀；`extra_edges` 再补上
///   四邻可走性表达不了的 span 连接断点（两块都走但不同层）；
/// * 两遍 chamfer（基数向步长 2、斜向 3，u8 饱和 255）；
/// * 距离 < 2×半径格的可行走格杀掉。
///
/// 返回杀掉的可行走格数（账目面：重烘前后可对账）。
pub(crate) fn erode(grid: &mut Grid, radius_cells: u32, extra_edges: &[u32]) -> usize {
    let threshold = radius_cells
        .checked_mul(2)
        .filter(|value| *value <= 255)
        .expect("侵蚀半径格数溢出 u8 距离场（voxel 过小），烘焙拒绝");
    let threshold = threshold as u8;
    let (cols, rows) = (grid.cols, grid.rows);
    let mut dist = vec![255u8; cols * rows];
    // Walkability boundary, unchanged from before the span pipeline: a walkable
    // cell next to a null cell or the void is a distance source.
    for cz in 0..rows as isize {
        for cx in 0..cols as isize {
            let index = cz as usize * cols + cx as usize;
            if !grid.walkable[index] {
                dist[index] = 0;
                continue;
            }
            if [(cx + 1, cz), (cx - 1, cz), (cx, cz + 1), (cx, cz - 1)]
                .into_iter()
                .any(|(nx, nz)| !grid.walkable_cell(nx, nz))
            {
                dist[index] = 0;
            }
        }
    }
    // Span-connection edges the walkable mask cannot express.
    for &index in extra_edges {
        dist[index as usize] = 0;
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
