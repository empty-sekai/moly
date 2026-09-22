//! 面的烘焙核：三角网栅格化 → 足迹标 null → 侵蚀 → 小区过滤。
//!
//! 每一步都是引擎原生烘焙链在 2D（单高度面）上的转录，锚点在
//! [`super`] 的模块注释里。本文件只有形状与常数，不出现任何 bevy。

use super::{ColliderPolygon, Obstacle, AGENT_CLIMB, AGENT_HEIGHT};

/// 烘焙格面：世界 xz 平面上的等距格，`walkable` 是侵蚀与小区过滤都
/// 过完之后的净可行走表。格原点是面三角网包围盒的最小角——引擎侧
/// 高度场的原点取自烘焙体包围盒，同为「任意相位」，足迹对格的相位
/// 误差按一格计（与原生同形状的量化噪声）。
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
pub(crate) fn rasterize(tris: &[[[f32; 2]; 3]], voxel: f32) -> Grid {
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
        let [a, b, c] = projected;
        let determinant = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if determinant.abs() < 1e-12 {
            continue;
        }
        for z in z0.max(0)..=z1.min(grid.rows as isize - 1) {
            for x in x0.max(0)..=x1.min(grid.cols as isize - 1) {
                let p = grid.cell_center(x as usize, z as usize);
                let u =
                    ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / determinant;
                let v =
                    ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / determinant;
                if u >= -1e-5 && v >= -1e-5 && u + v <= 1.00001 {
                    let y = u * tri[0][1] + v * tri[1][1] + (1.0 - u - v) * tri[2][1];
                    let index = z as usize * grid.cols + x as usize;
                    heights[index] = heights[index].max(y);
                }
            }
        }
    }
    heights
}

pub(crate) fn mark_collider_polygons(
    grid: &mut Grid,
    polygons: &[ColliderPolygon],
    heights: &[f32],
) -> usize {
    let mut nulled = 0;
    for polygon in polygons {
        if !polygon.triangles.is_empty() {
            nulled += mark_triangle_geometry(grid, polygon, heights);
            continue;
        }
        if polygon.vertices.is_empty() {
            continue;
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
                if !grid.walkable[index]
                    || polygon.max_y <= heights[index] + 1e-4
                    || polygon.min_y >= heights[index] + AGENT_HEIGHT
                {
                    continue;
                }
                // PhysicsColliders are also used for thin floor/rug/mat
                // surfaces.  Recast folds a contact whose top is within the
                // agent climb height into the walkable span; it does not carve
                // the original floor beneath it.  The previous projection
                // treated every overlapping polygon as an obstacle and made
                // these areas impossible to traverse.
                if polygon.max_y - heights[index] <= AGENT_CLIMB {
                    continue;
                }
                let p = grid.cell_center(x as usize, z as usize);
                // Vertical triangles project to line segments. Retain every
                // touched raster cell instead of silently dropping the wall.
                if polygon.vertices.len() < 3 {
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
                    if distance2 <= grid.voxel * grid.voxel * 0.5 {
                        grid.walkable[index] = false;
                        nulled += 1;
                    }
                    continue;
                }
                // Input is convex (mesh hull or one triangle), in either winding.
                let mut positive = false;
                let mut negative = false;
                for i in 0..polygon.vertices.len() {
                    let a = polygon.vertices[i];
                    let b = polygon.vertices[(i + 1) % polygon.vertices.len()];
                    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                    positive |= cross > 1e-6;
                    negative |= cross < -1e-6;
                }
                if !(positive && negative) {
                    grid.walkable[index] = false;
                    nulled += 1;
                }
            }
        }
    }
    nulled
}

/// Rasterize each face only over its own cells, clipping its geometry to each
/// cell before measuring height. A global triangle Y range would still block
/// the clear end of a slope/raised canopy. Convex shapes merge these intervals
/// before carving, so a tall closed box never becomes an empty walkable room.
fn mark_triangle_geometry(grid: &mut Grid, polygon: &ColliderPolygon, heights: &[f32]) -> usize {
    let mut spans = std::collections::HashMap::<usize, [f32; 2]>::new();
    let mut nulled = 0;
    for tri in &polygon.triangles {
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
                let Some(span) = triangle_cell_interval(tri, cell_min, grid.voxel) else {
                    continue;
                };
                if polygon.solid {
                    spans
                        .entry(index)
                        .and_modify(|range| {
                            range[0] = range[0].min(span[0]);
                            range[1] = range[1].max(span[1]);
                        })
                        .or_insert(span);
                } else if blocks_ground(span, heights[index]) {
                    grid.walkable[index] = false;
                    nulled += 1;
                }
            }
        }
    }
    for (index, span) in spans {
        if blocks_ground(span, heights[index]) {
            grid.walkable[index] = false;
            nulled += 1;
        }
    }
    nulled
}

fn blocks_ground(span: [f32; 2], ground: f32) -> bool {
    // Low tops are traversable; raised sources with enough clearance are not
    // walls. Heights remain relative to the sampled surface, not world zero.
    span[1] > ground + AGENT_CLIMB + 1e-5 && span[0] < ground + AGENT_HEIGHT
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

/// 侵蚀（`rcErodeWalkableArea` 的 2D 转录）：
/// * 距离场初值：null 格 0，可行走格 255；
/// * 边界标记：可行走格的四邻有 null 格**或出界（虚空）** → 距离 0
///   ——虚空与障碍同为 0 距离源，外边界同样被侵蚀；
/// * 两遍 chamfer（基数向步长 2、斜向 3，u8 饱和 255）；
/// * 距离 < 2×半径格的可行走格杀掉。
///
/// 返回杀掉的可行走格数（账目面：重烘前后可对账）。
pub(crate) fn erode(grid: &mut Grid, radius_cells: u32) -> usize {
    let threshold = radius_cells
        .checked_mul(2)
        .filter(|value| *value <= 255)
        .expect("侵蚀半径格数溢出 u8 距离场（voxel 过小），烘焙拒绝");
    let threshold = threshold as u8;
    let (cols, rows) = (grid.cols, grid.rows);
    let mut dist = vec![255u8; cols * rows];
    for (index, walkable) in grid.walkable.iter().enumerate() {
        if !walkable {
            dist[index] = 0;
        }
    }
    // 边界标记（先于 chamfer，与源同序）。
    for cz in 0..rows as isize {
        for cx in 0..cols as isize {
            let index = cz as usize * cols + cx as usize;
            if !grid.walkable[index] {
                continue;
            }
            let boundary = [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)]
                .into_iter()
                .any(|(nx, nz)| !grid.walkable_cell(nx, nz));
            if boundary {
                dist[index] = 0;
            }
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
