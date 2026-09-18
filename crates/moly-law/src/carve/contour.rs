//! 轮廓追踪与简化（`rcBuildContours` 的 2D 转录）。
//!
//! 分区之后，每个区的边界被追成一条闭合折线，再按最大偏差简化。简化后的
//! 折线是多边形网那一步的输入。
//!
//! # 锚点
//!
//! 逐条对着 CN `libunity.so` 的 `FUN_0090b68c` 写。原生把 `walkContour` /
//! `simplifyContour` 等全部内联进了一个函数体，所以下面按算法阶段分节，
//! 不按被调函数分。
//!
//! # 我方 2D 形下塌缩掉的两位标志
//!
//! 原生给每个轮廓顶点存一个「区域 + 标志」字：低 16 位邻区号、bit 16
//! 「特殊边界顶点」、bit 17「area 分界」。我方两位恒不置位，第四分量因此
//! 塌缩成纯邻区号：
//!
//! * **bit 16** 标的是「这个角碰到了带 `0x8000` 的边界区」。边界区是分瓦的
//!   产物（每瓦 padding 27 体素），我方单张整场格不刷边界区 ⇒ 恒不触发。
//! * **bit 17** 标的是「这条边两侧的 area 类不同」。我方的面只有「可走」
//!   一个 area 类 ⇒ 恒不触发。
//!
//! ⇒ 原生用掩码 `0x2ffff`（低 16 位 + bit 17）判「要不要插初始点」，在我方
//! 等价于「邻区号变了」；用「邻区号为 0 **或** bit 17 置位」判「要不要细分」，
//! 在我方等价于「这是一条外墙边」。两处都按塌缩后的形态写，差异记在这里。
//!
//! # 一处读不出来的量
//!
//! ⚠ 方向偏移表 `dirOffsetX/Y` 的四个具体数值**在本次导出的证据里读不到**
//! （它们是两张全局数据表，不在那 27 个导出函数体内）。能从反编译确定的只有
//! 转向规则本身：**产出顶点后 `dir+1`、走到邻格后 `dir+3`（即 -1）**。
//! 这里用的是 upstream 的表（档次「知识库」，低于本文件其余部分的「反编译」）；
//! 它决定的是绕行手性，而手性只影响轮廓的环绕方向，不影响顶点集合。

use super::grid::Grid;
use super::region::Regions;

/// 方向偏移。⚠ 见模块注释：数值锚的是 upstream，不是反编译。
const DIR_X: [isize; 4] = [-1, 0, 1, 0];
const DIR_Z: [isize; 4] = [0, 1, 0, -1];

/// 追踪的迭代上限。原生守门在循环**顶部**、用 `==`、`iter` 在**底部**自增
/// ⇒ 循环体最多执行 39999 次；触顶直接停，**不回滚已产出的顶点**。
const MAX_WALK_ITERATIONS: u32 = 39999;

/// 一个轮廓顶点：格角坐标，以及「本顶点 → 下一顶点」这条边外侧的区号。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ContourVert {
    pub(crate) x: i32,
    pub(crate) z: i32,
    /// 边外侧的区号；0 = 外墙（面的边界，不是另一个区）。
    pub(crate) neighbour: u32,
}

/// 一个区的简化轮廓。
pub(crate) struct Contour {
    pub(crate) verts: Vec<ContourVert>,
    pub(crate) region: u32,
}

/// 格 `(cx, cz)` 在方向 `dir` 上的邻区号；出界或不可走都是 0。
fn neighbour_region(grid: &Grid, regions: &Regions, cx: isize, cz: isize, dir: usize) -> u32 {
    let (nx, nz) = (cx + DIR_X[dir], cz + DIR_Z[dir]);
    if !grid.walkable_cell(nx, nz) {
        return 0;
    }
    regions.ids[nz as usize * grid.cols + nx as usize]
}

/// 顶点坐标：由 `dir` 决定加不加 1（原生 c:340-354 的四支表）。
fn corner(cx: isize, cz: isize, dir: usize) -> (i32, i32) {
    match dir {
        0 => (cx as i32, cz as i32 + 1),
        1 => (cx as i32 + 1, cz as i32 + 1),
        2 => (cx as i32 + 1, cz as i32),
        _ => (cx as i32, cz as i32),
    }
}

/// 轮廓追踪 + 简化。`max_error` 单位是**格**（原生 `maxSimplificationError`，
/// 本站点取 1.2999999523162842）。
pub(crate) fn build_contours(grid: &Grid, regions: &Regions, max_error: f32) -> Vec<Contour> {
    let (cols, rows) = (grid.cols, grid.rows);
    // 阶段 A：预标记。第 dir 位置 1 ⟺ 该方向是一条轮廓边（邻区与自己不同）。
    // 原生先按「邻区相同」置位再整体 `^ 0x0f`，等价。
    let mut flags = vec![0u8; cols * rows];
    for cz in 0..rows as isize {
        for cx in 0..cols as isize {
            let index = cz as usize * cols + cx as usize;
            let region = regions.ids[index];
            if !grid.walkable[index] || region == 0 {
                continue;
            }
            let mut edges = 0u8;
            for dir in 0..4 {
                if neighbour_region(grid, regions, cx, cz, dir) != region {
                    edges |= 1 << dir;
                }
            }
            flags[index] = edges;
        }
    }

    let mut contours = Vec::new();
    let mut raw: Vec<ContourVert> = Vec::new();
    // 阶段 B：扫描序 z 外、x 内，与阶段 A 相同。
    for cz in 0..rows as isize {
        for cx in 0..cols as isize {
            let start = cz as usize * cols + cx as usize;
            // 四面皆边界的孤立格与无边界格都跳过；**前者要把 flags 清零**
            // （原生 c:205-207），否则它会在后面被当作可起步的格反复命中。
            if flags[start] == 0x0f || flags[start] == 0 {
                flags[start] = 0;
                continue;
            }
            let region = regions.ids[start];
            if region == 0 {
                continue;
            }
            raw.clear();
            walk_contour(grid, regions, &mut flags, cx, cz, &mut raw);
            if raw.is_empty() {
                continue;
            }
            let verts = simplify(&raw, max_error);
            if verts.len() >= 3 {
                contours.push(Contour { verts, region });
            }
        }
    }
    contours
}

/// 沿一个区的边界绕行，产出未简化的顶点环。
fn walk_contour(
    grid: &Grid,
    regions: &Regions,
    flags: &mut [u8],
    start_x: isize,
    start_z: isize,
    out: &mut Vec<ContourVert>,
) {
    let cols = grid.cols;
    let start_index = start_z as usize * cols + start_x as usize;
    // 起始方向取 flags 的最低置位位。
    let mut dir = 0usize;
    while flags[start_index] & (1 << dir) == 0 {
        dir += 1;
    }
    let start_dir = dir;
    let (mut cx, mut cz) = (start_x, start_z);
    let mut index = start_index;
    let mut iterations = 0u32;
    loop {
        if iterations == MAX_WALK_ITERATIONS {
            break;
        }
        let delta = if flags[index] & (1 << dir) != 0 {
            let (x, z) = corner(cx, cz, dir);
            out.push(ContourVert {
                x,
                z,
                neighbour: neighbour_region(grid, regions, cx, cz, dir),
            });
            flags[index] &= !(1 << dir);
            1
        } else {
            let (nx, nz) = (cx + DIR_X[dir], cz + DIR_Z[dir]);
            if !grid.walkable_cell(nx, nz) {
                break; // 原生的 slot == 0x3f
            }
            cx = nx;
            cz = nz;
            index = nz as usize * cols + nx as usize;
            3
        };
        iterations += 1;
        dir = (dir + delta) & 3;
        // 终止判据在循环底部 ⇒ 起点那条边会被产出一次。
        if index == start_index && dir == start_dir {
            break;
        }
    }
}

/// 点到线段的垂距平方，参数钳在 `[0,1]`。全程 f32。
fn distance_to_segment(px: i32, pz: i32, ax: i32, az: i32, bx: i32, bz: i32) -> f32 {
    let pqx = (bx - ax) as f32;
    let pqz = (bz - az) as f32;
    let dx = (px - ax) as f32;
    let dz = (pz - az) as f32;
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    t = t.clamp(0.0, 1.0);
    let ex = pqx * t - dx;
    let ez = pqz * t - dz;
    ex * ex + ez * ez
}

/// 简化：先选初始点，再逐边细分到最大偏差不超过 `max_error`。
fn simplify(raw: &[ContourVert], max_error: f32) -> Vec<ContourVert> {
    let count = raw.len();
    // 简化中第三个分量临时存的是 raw 点下标，最后才换回邻区号。
    let mut kept: Vec<(i32, i32, usize)> = Vec::new();
    let connected = raw.iter().any(|v| v.neighbour != 0);
    if connected && count > 3 {
        // 路径 A：在「邻区号变了」的每条边上插一个初始点。
        for i in 0..count {
            let next = (i + 1) % count;
            if raw[i].neighbour != raw[next].neighbour {
                kept.push((raw[i].x, raw[i].z, i));
            }
        }
    }
    if kept.is_empty() {
        // 路径 B：取字典序最小与最大的两个角。触发条件是「还没有初始点」，
        // 不是「没有跨区边」——全环同一个邻区时也走这里。
        let mut lower = (raw[0].x, raw[0].z, 0usize);
        let mut upper = lower;
        for (i, v) in raw.iter().enumerate() {
            if v.x < lower.0 || (v.x == lower.0 && v.z < lower.1) {
                lower = (v.x, v.z, i);
            }
            if v.x > upper.0 || (v.x == upper.0 && v.z > upper.1) {
                upper = (v.x, v.z, i);
            }
        }
        kept.push(lower);
        kept.push(upper);
    }

    let threshold = max_error * max_error;
    let mut i = 0usize;
    while i < kept.len() {
        let next = (i + 1) % kept.len();
        let (ax, az, ai) = kept[i];
        let (bx, bz, bi) = kept[next];
        // P 恒取字典序较小的那个端点。float 不满足结合律，同一条物理边从两个
        // 方向遇到时必须算出逐位相同的距离，否则共享边上会出现方向相关的分叉。
        let (px, pz, qx, qz, step, mut ci, end) = if bx > ax || (bx == ax && bz > az) {
            (ax, az, bx, bz, 1usize, (ai + 1) % count, bi)
        } else {
            (bx, bz, ax, az, count - 1, (bi + count - 1) % count, ai)
        };
        // 细分门：只对外墙边（邻区号 0）开。读的是 ci 初值那一个点，不是逐点判。
        let mut max_distance = 0.0f32;
        let mut max_at: Option<usize> = None;
        if ci != end && raw[ci].neighbour == 0 {
            loop {
                let distance = distance_to_segment(raw[ci].x, raw[ci].z, px, pz, qx, qz);
                if distance > max_distance {
                    max_distance = distance;
                    max_at = Some(ci);
                }
                ci = (ci + step) % count;
                if ci == end {
                    break;
                }
            }
        }
        match max_at {
            // 严格大于，平方比平方。
            Some(at) if max_distance > threshold => {
                kept.insert(i + 1, (raw[at].x, raw[at].z, at));
                // i 不前进：新插的点自己还要再判一次。
            }
            _ => i += 1,
        }
    }

    // 换回邻区号：这条边的邻区取**下一个** raw 点的邻区。
    kept.into_iter()
        .map(|(x, z, index)| ContourVert {
            x,
            z,
            neighbour: raw[(index + 1) % count].neighbour,
        })
        .collect()
}
