//! Monotone region building (implements `rcBuildRegionsMonotone` in 2D).
//!
//! After small-region filtering, the bakes chain enters the last three Recast
//! stages: regions → contours → polygon mesh. This site takes the **monotone
//! regions** branch: the schedule builds no distance field before this step,
//! and an implementation calling `rcBuildDistanceField` plus the watershed
//! `rcBuildRegions` is not equivalent to it.
//!
//! # Shape
//!
//! Row-by-row sweep. In each row a run of consecutive walkable cells forms a
//! "sweep"; a sweep first inherits the temporary id of the previous sweep via
//! the **west neighbour**, then looks at the **north neighbour**: if the north
//! neighbour is unique, the sweep joins that region; if there is more than one,
//! the sweep starts a new region. After a row is swept all ids are fixed, and
//! the row's temporary ids are remapped to the fixed ids.
//!
//! # Two named differences from Recast
//!
//! * **Region ids are `u32`, not Recast's `u16`.** Recast borrows the `0x8000`
//!   bit as the border-region flag, leaving only 15 bits for the id; that
//!   limit **comes with tiling** — Recast bakes one 100-voxel tile at a time,
//!   and the regions within a single tile fit far below it. This bake is one
//!   whole-field grid (3000² scale), where with 1 cm voxels the region count
//!   can easily pass 65535; keeping `u16` would silently truncate.
//! * **`borderSize` is always 0; the four border regions are not emitted.**
//!   Border regions are a product of tile seams: Recast pads each tile by 27
//!   voxels and rings the padding with four flagged regions `0x8001..0x8004`,
//!   so that regions crossing tiles are not killed as small regions of this
//!   tile. A single whole-field grid has no tile seams, the four regions have
//!   nowhere to go, so the check "do not join when the north neighbour carries
//!   the border flag" is never triggered either.
//!
//! These two differences only change **the numbering of regions and the border
//! handling**, not **which cells are walkable** — the walkable set is already
//! fixed before this module by rasterization, carving, erosion and small-region
//! filtering.

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
#[derive(Clone)]
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
