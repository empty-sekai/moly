//! 单调分区（`rcBuildRegionsMonotone` 的 2D 转录）。
//!
//! 烘焙链在小区过滤之后进入原生 Recast 的后三段：分区 → 轮廓 → 多边形网。
//! 本站点走的是**单调分区**这一支：原生调度里这一步之前没有距离场构建，
//! 一个调 `rcBuildDistanceField` + 分水岭 `rcBuildRegions` 的实现与它不等价。
//!
//! # 形状
//!
//! 逐行扫描。每行里，一段连续的可行走格构成一个「跨段」；跨段先按**西邻**
//! 继承上一个跨段的临时号，再看**北邻**：北邻若唯一，跨段就并进那个区；
//! 北邻多于一个，跨段自立新区。一行扫完后统一定号，再把本行的临时号重映射
//! 成定号。
//!
//! # 与原生的两处具名差异
//!
//! * **区号用 `u32`，不是原生的 `u16`。** 原生把 `0x8000` 位借去当边界区
//!   标志，区号因此只剩 15 位；那个上限是**跟着分瓦来的**——原生按 100 体素
//!   一瓦烘，单瓦内的区数远够用。本烘焙是单张整场格（3000² 量级），1 cm
//!   体素下区数可以轻易越过 65535，沿用 `u16` 会静默截断。
//! * **`borderSize` 恒 0，四条边界区不刷。** 边界区是瓦缝的产物：原生给每瓦
//!   padding 27 体素，用 `0x8001..0x8004` 四个带标志位的区把 padding 圈起来，
//!   好让跨瓦的区不被当成本瓦的小区杀掉。单张整场格没有瓦缝，那四个区无处
//!   安放，随之「北邻带边界标志则不并」这条判据也恒不触发。
//!
//! 这两处差异只改**分区的编号与边界处理**，不改**哪些格可行走**——可走集
//! 在本模块之前就已经由栅格化、挖洞、侵蚀、小区过滤定死了。

use super::grid::Grid;

/// 「上一行邻区多于一个」的哨兵（原生是 `u16` 的 `0xffff`）。
const NULL_NEI: u32 = u32::MAX;

/// 一次行扫描里的一个跨段。
#[derive(Clone, Copy, Default)]
struct Sweep {
    /// 定案后的区号。
    id: u32,
    /// 与 `nei` 那个区相邻的格数。
    ns: u32,
    /// 上一行的邻区号：0 = 还没有，[`NULL_NEI`] = 多于一个。
    nei: u32,
}

/// 分区结果。
pub(crate) struct Regions {
    /// 每格的区号；0 = 不可行走或无区。长度 `cols * rows`。
    pub(crate) ids: Vec<u32>,
    /// 区号上界：有效区号是 `1..max`。
    pub(crate) max: u32,
}

/// 逐行单调分区。
pub(crate) fn build_monotone(grid: &Grid) -> Regions {
    let (cols, rows) = (grid.cols, grid.rows);
    let mut ids = vec![0u32; cols * rows];
    // 原生在刷完四条边界区后从 5 起号；没有边界区就从 1 起。
    let mut next_id: u32 = 1;
    let mut sweeps: Vec<Sweep> = Vec::new();
    // 上一行每个区在本行被并进来的格数。原生每行把它整块清零；这里只清
    // 本行碰过的下标——`prev` 的读与写都只落在本行引用过的区号上，
    // 所以两种清法结果相同，而按行整清是 O(区数 × 行数)。
    let mut prev: Vec<u32> = Vec::new();
    let mut touched: Vec<u32> = Vec::new();

    for cz in 0..rows {
        let row = cz * cols;
        // 本行的临时号从 1 起；0 留给「还没号」。
        let mut rid: u32 = 1;
        sweeps.clear();
        sweeps.push(Sweep::default()); // 下标 0 不用
        for cx in 0..cols {
            let index = row + cx;
            if !grid.walkable[index] {
                continue;
            }
            // —— 西邻：连通就继承它的临时号 ——
            let mut previd = if cx > 0 && grid.walkable[index - 1] {
                ids[index - 1]
            } else {
                0
            };
            if previd == 0 {
                previd = rid;
                sweeps.push(Sweep::default());
                rid += 1;
            }
            // —— 北邻：唯一就并进去，多于一个就作废 ——
            if cz > 0 && grid.walkable[index - cols] {
                let neighbour = ids[index - cols];
                if neighbour != 0 {
                    let sweep = &mut sweeps[previd as usize];
                    if sweep.nei == 0 || sweep.nei == neighbour {
                        sweep.nei = neighbour;
                        sweep.ns += 1;
                        let slot = neighbour as usize;
                        if prev.len() <= slot {
                            prev.resize(slot + 1, 0);
                        }
                        if prev[slot] == 0 {
                            touched.push(neighbour);
                        }
                        prev[slot] += 1;
                    } else {
                        sweep.nei = NULL_NEI;
                    }
                }
            }
            ids[index] = previd;
        }
        // —— 定号：北邻唯一、且那个区在本行的相邻格数恰好全落在本跨段上，
        //    才并进北邻；否则自立新区 ——
        for sweep in sweeps.iter_mut().take(rid as usize).skip(1) {
            let joins = sweep.nei != 0
                && sweep.nei != NULL_NEI
                && prev.get(sweep.nei as usize).copied() == Some(sweep.ns);
            if joins {
                sweep.id = sweep.nei;
            } else {
                sweep.id = next_id;
                next_id += 1;
            }
        }
        // —— 本行重映射 ——
        for cx in 0..cols {
            let index = row + cx;
            let temporary = ids[index];
            if temporary != 0 && temporary < rid {
                ids[index] = sweeps[temporary as usize].id;
            }
        }
        for id in touched.drain(..) {
            prev[id as usize] = 0;
        }
    }

    Regions { ids, max: next_id }
}
