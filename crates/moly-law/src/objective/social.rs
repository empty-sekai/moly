//! 社交目的地链（源 `GetNearOtherCharacterPosition` →
//! `GetNonOverlappingNpcPositions` → `GetNearbyNavigablePositions`）。
//!
//! 源的形状：
//! - 候选 = NPC 列表过三道滤：非自身、有对话数据、同站点（调用方带入
//!   过滤后的数组；「有对话数据」是输入合同——终滤要读对话目的地，
//!   源对空对话数据直接抛错，本律把该字段做成必填）；
//! - **目的地重叠跳过**：候选与任一其他候选的**当前导航目的地**距离
//!   小于 1.0 ⇒ 该候选整个跳过（12 个点位一个不生成）；
//! - 每个存活候选生成 ≤12 个点位：4 个世界轴向 × 3 个尺度
//!   （0.7 起步进 0.5），点位 = 候选位置 + 轴向·尺度（高度取候选的）；
//! - 生成滤：任一 NPC（含寻求者）距点位 < 0.7 ⇒ 弃；
//! - 可导航谓词 [`is_navigable`]：采样容差 **0.0** + 验路；**采样命中
//!   位丢弃**——目的地是原始生成点位，不是采样位；
//! - 终滤：点位对每个候选的**当前位置**与**对话目的地**都保持 0.7
//!   外（含端点）；
//! - 池非空 ⇒ **恰一次均匀抽**；空 ⇒ 未命中，调用方保持原位。
//!
//! 抽签次数：候选空或池空零抽签；否则恰 1 次。重叠跳过与两道距离滤
//! 都无随机。

use crate::talk::select::UniformDraw;

use super::SurfaceProbe;

/// 点位生成的 4 个世界轴向（前、后、左、右；高度分量恒 0）。
pub const NPC_SPOT_DIRECTIONS: [[f32; 3]; 4] =
    [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];

/// 点位尺度序列：0.7 起、步进 0.5、过 2.0 止（源 do-while 的三值）。
pub const SPOT_SCALES: [f32; 3] = [0.7, 0.7 + 0.5, 0.7 + 1.0];

/// 点位净空阈：与 NPC / 候选当前位置 / 候选对话目的地的距离下界
/// （含端点，`< 0.7` 才排除）。
pub const SPOT_CLEARANCE: f32 = 0.7;

/// 候选间导航目的地重叠阈（`< 1.0` 即重叠，含端点不重叠）。
pub const DESTINATION_OVERLAP: f32 = 1.0;

/// 社交候选（源三道滤之后的成员）：当前位置、当前导航目的地、对话
/// 目的地。后两者分别供重叠跳过与终滤读取。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SocialCandidate {
    /// 候选当前位置。
    pub position: [f32; 3],
    /// 候选当前导航目的地（源 `NavMeshAgent.destination`）。
    pub destination: [f32; 3],
    /// 候选对话数据的目的地（源 `TalkData.TargetPosition`；输入合同
    /// = 候选必有对话数据）。
    pub talk_target: [f32; 3],
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// 可导航谓词（源 `IsNavigable`）：目标点先表面采样（容差 **0.0**），
/// 未命中即不可导航；命中再验「出发点到**采样位**有路」。采样位丢弃
/// ——本谓词不产出目的地。
pub fn is_navigable(
    source: [f32; 3],
    target: [f32; 3],
    probe: &mut impl SurfaceProbe,
) -> bool {
    match probe.sample(target, 0.0) {
        None => false,
        Some(hit) => probe.has_path(source, hit),
    }
}

/// 目的地重叠跳过：候选 `index` 与任一其他候选的导航目的地距离
/// `< 1.0` ⇒ 重叠（该候选整个跳过）。恰在 1.0 上不重叠。
pub fn destination_overlaps(candidates: &[SocialCandidate], index: usize) -> bool {
    let mine = candidates[index].destination;
    candidates.iter().enumerate().any(|(other, candidate)| {
        other != index && dist3(mine, candidate.destination) < DESTINATION_OVERLAP
    })
}

/// 单候选点位生成：4 轴向 × 3 尺度，点位 = 候选位置 + 轴向·尺度。
///
/// 生成滤：任一 NPC（含寻求者）距点位 `< 0.7` ⇒ 弃；再过可导航谓词
/// 才收。收集序 = 轴向外层、尺度内层（终池的均匀抽对序敏感，保持源
/// 序）。
pub fn nearby_spots(
    candidate_position: [f32; 3],
    source_position: [f32; 3],
    npc_positions: &[[f32; 3]],
    probe: &mut impl SurfaceProbe,
) -> Vec<[f32; 3]> {
    let mut spots = Vec::new();
    for dir in NPC_SPOT_DIRECTIONS {
        for &scale in SPOT_SCALES.iter() {
            let spot = [
                candidate_position[0] + dir[0] * scale,
                candidate_position[1] + dir[1] * scale,
                candidate_position[2] + dir[2] * scale,
            ];
            // 生成滤：全 NPC 净空（任一命中即弃，含寻求者）。
            if npc_positions
                .iter()
                .any(|&npc| dist3(npc, spot) < SPOT_CLEARANCE)
            {
                continue;
            }
            if is_navigable(source_position, spot, probe) {
                spots.push(spot);
            }
        }
    }
    spots
}

/// 终滤：点位对每个候选的当前位置与对话目的地都保持 `>= 0.7`
/// （含端点）。
pub fn spot_clear_of_candidates(spot: [f32; 3], candidates: &[SocialCandidate]) -> bool {
    candidates.iter().all(|candidate| {
        dist3(spot, candidate.position) >= SPOT_CLEARANCE
            && dist3(spot, candidate.talk_target) >= SPOT_CLEARANCE
    })
}

/// 社交目的地全链：候选空 ⇒ 未命中；逐存活候选（先过重叠跳过）收集
/// 点位；终滤；池非空 ⇒ 恰一次均匀抽返回**原始点位**。
///
/// `source_position` 是寻求者位置（生成滤的 NPC 表应含寻求者；源经
/// 角色数据仓回查寻求者，查无即记错并空手而归——本律由调用方自持
/// 位置，该失败形态不迁）。抽签次数：池空零抽、池非空恰 1 次。
pub fn social_target(
    candidates: &[SocialCandidate],
    source_position: [f32; 3],
    npc_positions: &[[f32; 3]],
    probe: &mut impl SurfaceProbe,
    rand: &mut impl UniformDraw,
) -> Option<[f32; 3]> {
    if candidates.is_empty() {
        return None;
    }
    let mut spots: Vec<[f32; 3]> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        // 目的地与任一其他候选重叠：整员跳过。
        if destination_overlaps(candidates, index) {
            continue;
        }
        spots.extend(nearby_spots(
            candidate.position,
            source_position,
            npc_positions,
            probe,
        ));
    }
    let pool: Vec<[f32; 3]> = spots
        .into_iter()
        .filter(|&spot| spot_clear_of_candidates(spot, candidates))
        .collect();
    if pool.is_empty() {
        return None;
    }
    // 恰一次均匀抽，返回原始点位（非采样位）。
    Some(pool[rand.draw(pool.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 均匀抽替身：固定索引并数调用。
    struct FixedDraw {
        index: usize,
        calls: usize,
        last_len: usize,
    }

    impl FixedDraw {
        fn at(index: usize) -> Self {
            Self { index, calls: 0, last_len: 0 }
        }
    }

    impl UniformDraw for FixedDraw {
        fn draw(&mut self, len: usize) -> usize {
            self.calls += 1;
            self.last_len = len;
            self.index.min(len.saturating_sub(1))
        }
    }

    /// 探测替身：采样恒命中（命中位抬 0.1），`miss` 内的目标采样未命中；
    /// 验路按谓词应答。记录全部调用。
    struct PredicateProbe {
        navigable: fn([f32; 3]) -> bool,
        miss: Vec<[f32; 3]>,
        samples: Vec<([f32; 3], f32)>,
        paths: Vec<([f32; 3], [f32; 3])>,
    }

    impl PredicateProbe {
        fn new(navigable: fn([f32; 3]) -> bool) -> Self {
            Self {
                navigable,
                miss: Vec::new(),
                samples: Vec::new(),
                paths: Vec::new(),
            }
        }

        fn missing(mut self, target: [f32; 3]) -> Self {
            self.miss.push(target);
            self
        }
    }

    impl SurfaceProbe for PredicateProbe {
        fn sample(&mut self, target: [f32; 3], tolerance: f32) -> Option<[f32; 3]> {
            self.samples.push((target, tolerance));
            if self.miss.contains(&target) {
                return None;
            }
            Some([target[0], target[1] + 0.1, target[2]])
        }

        fn has_path(&mut self, source: [f32; 3], target: [f32; 3]) -> bool {
            self.paths.push((source, target));
            (self.navigable)(target)
        }
    }

    fn always(_: [f32; 3]) -> bool {
        true
    }

    fn never(_: [f32; 3]) -> bool {
        false
    }

    /// 候选（在原点，目的地与对话目的地都摆远）。
    fn candidate_at(position: [f32; 3], destination: [f32; 3], talk_target: [f32; 3]) -> SocialCandidate {
        SocialCandidate { position, destination, talk_target }
    }

    #[test]
    fn empty_candidates_yield_none_without_drawing() {
        // 候选空：零探测零抽签
        let mut probe = PredicateProbe::new(always);
        let mut rand = FixedDraw::at(0);
        assert_eq!(social_target(&[], [0.0; 3], &[], &mut probe, &mut rand), None);
        assert!(probe.samples.is_empty());
        assert_eq!(rand.calls, 0);
    }

    #[test]
    fn spots_cover_four_directions_and_three_scales() {
        // 无 NPC 挡、恒可导航：恰 12 点，序 = 轴向外层、尺度内层
        let mut probe = PredicateProbe::new(always);
        let spots = nearby_spots([0.0; 3], [0.0; 3], &[], &mut probe);
        // 按生成式重算期望（值 = 轴向·尺度，高度取候选的 0）
        let mut want = Vec::new();
        for dir in NPC_SPOT_DIRECTIONS {
            for &scale in SPOT_SCALES.iter() {
                want.push([dir[0] * scale, 0.0, dir[2] * scale]);
            }
        }
        assert_eq!(spots.len(), 12);
        assert_eq!(spots, want);
    }

    #[test]
    fn spot_within_clearance_of_any_npc_is_dropped() {
        // NPC 在 (0.4,0,0.4)（对角距 0.5）：挡掉前向与右向的 0.7 尺度
        // 两点（距 0.5 < 0.7）；1.2/1.7 尺度距约 0.89 不挡
        let mut probe = PredicateProbe::new(always);
        let spots = nearby_spots([0.0; 3], [0.0; 3], &[[0.4, 0.0, 0.4]], &mut probe);
        assert!(!spots.contains(&[0.0, 0.0, SPOT_SCALES[0]]), "前向 0.7 被挡");
        assert!(!spots.contains(&[SPOT_SCALES[0], 0.0, 0.0]), "右向 0.7 被挡");
        assert!(spots.contains(&[0.0, 0.0, -SPOT_SCALES[0]]), "背向不挡");
        assert_eq!(spots.len(), 10);
        assert_eq!(probe.samples.len(), 10);
    }

    #[test]
    fn seeker_counts_in_generation_filter() {
        // 生成滤含寻求者自身：寻求者贴着候选 ⇒ 近点位被自己挡掉
        let mut probe = PredicateProbe::new(always);
        let spots = nearby_spots([0.0; 3], [0.4, 0.0, 0.4], &[[0.4, 0.0, 0.4]], &mut probe);
        assert!(!spots.contains(&[0.0, 0.0, SPOT_SCALES[0]]));
    }

    #[test]
    fn navigable_requires_sample_then_path() {
        // 采样未命中 ⇒ 不可导航且不验路；命中 + 验路失败 ⇒ 不可导航；
        // 命中 + 验路通过 ⇒ 可导航
        let mut probe = PredicateProbe::new(never).missing([0.0, 0.0, 1.7]);
        assert!(!is_navigable([0.0; 3], [0.0, 0.0, 1.7], &mut probe));
        assert_eq!(probe.paths.len(), 0, "采样 miss 不验路");

        let mut probe = PredicateProbe::new(never);
        assert!(!is_navigable([0.0; 3], [0.0, 0.0, 1.7], &mut probe));
        assert_eq!(probe.paths.len(), 1, "命中后验路一次");

        let mut probe = PredicateProbe::new(always);
        assert!(is_navigable([0.0; 3], [0.0, 0.0, 1.7], &mut probe));
    }

    #[test]
    fn navigable_samples_with_zero_tolerance_and_paths_to_hit() {
        // 容差恒 0.0；验路目标是采样命中位（不是原始点位）
        let mut probe = PredicateProbe::new(always);
        assert!(is_navigable([1.0, 2.0, 3.0], [0.0, 0.0, 1.7], &mut probe));
        assert_eq!(probe.samples[0], ([0.0, 0.0, 1.7], 0.0));
        assert_eq!(probe.paths[0], ([1.0, 2.0, 3.0], [0.0, 0.1, 1.7]));
    }

    #[test]
    fn non_navigable_spots_are_not_collected() {
        // 验路谓词只放行 x 正向：仅 4 个右向点位入池
        fn right_only(target: [f32; 3]) -> bool {
            target[0] > 0.5
        }
        let mut probe = PredicateProbe::new(right_only);
        let spots = nearby_spots([0.0; 3], [0.0; 3], &[], &mut probe);
        assert_eq!(spots.len(), 3, "仅右向 3 尺度");
        assert!(spots.iter().all(|s| s[0] > 0.5));
    }

    #[test]
    fn destination_overlap_skips_whole_candidate() {
        // 候选 0 与 1 的导航目的地相距 0.5 < 1.0 ⇒ 双双整员跳过；
        // 候选 2 的目的地远离 ⇒ 只有它的点位被生成
        let candidates = [
            candidate_at([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], [50.0; 3]),
            candidate_at([20.0, 0.0, 0.0], [0.5, 0.0, 0.0], [50.0; 3]),
            candidate_at([0.0, 0.0, 30.0], [40.0, 0.0, 0.0], [50.0; 3]),
        ];
        let mut probe = PredicateProbe::new(always);
        let mut rand = FixedDraw::at(0);
        let found = social_target(&candidates, [0.0; 3], &[], &mut probe, &mut rand);
        // 只有候选 2 贡献点位；全部采样目标都在它周围 ±1.7 内
        assert!(found.is_some());
        for (target, _) in &probe.samples {
            assert!(
                target[0].abs() <= 1.75 && (target[2] - 30.0).abs() <= 1.75,
                "采样目标必须全部来自候选 2 的邻域：{target:?}"
            );
        }
    }

    #[test]
    fn exact_overlap_boundary_is_not_overlap() {
        // 目的地恰距 1.0：不重叠（严格小于才跳）
        let candidates = [
            candidate_at([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], [50.0; 3]),
            candidate_at([20.0, 0.0, 0.0], [1.0, 0.0, 0.0], [50.0; 3]),
        ];
        assert!(!destination_overlaps(&candidates, 0));
        assert!(!destination_overlaps(&candidates, 1));
        let closer = [
            candidate_at([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], [50.0; 3]),
            candidate_at([20.0, 0.0, 0.0], [0.5, 0.0, 0.0], [50.0; 3]),
        ];
        assert!(destination_overlaps(&closer, 0), "0.5 < 1.0 重叠");
    }

    #[test]
    fn final_filter_uses_candidate_position_and_talk_target() {
        // 候选在原点、对话目的地在 (0,0,5)：两侧都得保持 0.7 外
        // （含端点；贴到 0.5 即弃，1.0 即留——测试点离界留裕量，
        // 恰 0.7 的端点语义由 `<` 比较承载）
        let candidates = [candidate_at([0.0; 3], [40.0; 3], [0.0, 0.0, 5.0])];
        assert!(
            !spot_clear_of_candidates([0.5, 0.0, 0.0], &candidates),
            "贴候选当前位置 ⇒ 弃"
        );
        assert!(
            !spot_clear_of_candidates([0.0, 0.0, 4.5], &candidates),
            "贴候选对话目的地 ⇒ 弃"
        );
        assert!(spot_clear_of_candidates([1.0, 0.0, 0.0], &candidates));
        assert!(spot_clear_of_candidates([1.0, 0.0, 5.0], &candidates));
    }

    #[test]
    fn full_chain_returns_raw_spot_with_exactly_one_draw() {
        // 四个对角 NPC（各距两相邻轴的 0.7 尺度点 0.5）挡掉全部首档
        // 点位 ⇒ 池 = 8 个 1.2/1.7 尺度点；恰一次均匀抽，返回原始点位
        let candidates = [candidate_at([0.0; 3], [40.0; 3], [50.0; 3])];
        let npc_positions = [
            [0.4, 0.0, 0.4],
            [-0.4, 0.0, 0.4],
            [0.4, 0.0, -0.4],
            [-0.4, 0.0, -0.4],
        ];
        let mut probe = PredicateProbe::new(always);
        let mut rand = FixedDraw::at(3);
        let found =
            social_target(&candidates, [0.0; 3], &npc_positions, &mut probe, &mut rand);
        assert_eq!(rand.calls, 1, "池非空恰一次均匀抽");
        assert_eq!(rand.last_len, 8);
        // 池序 = 轴向外层、尺度内层（0.7 档被挡）：第 4 项 = 背向 1.7
        assert_eq!(found, Some([0.0, 0.0, -SPOT_SCALES[2]]));
    }

    #[test]
    fn empty_pool_yields_none_without_drawing() {
        // 点位全被终滤之外的关卡挡掉（此处用验路恒败）⇒ 池空 ⇒
        // 零抽签
        let candidates = [candidate_at([0.0; 3], [40.0; 3], [50.0; 3])];
        let mut probe = PredicateProbe::new(never);
        let mut rand = FixedDraw::at(0);
        let found = social_target(&candidates, [0.0; 3], &[], &mut probe, &mut rand);
        assert_eq!(found, None);
        assert_eq!(rand.calls, 0, "池空零抽签");
        assert_eq!(probe.samples.len(), 12, "12 点全探测、全败在验路");
    }

    #[test]
    fn spot_scales_follow_source_doubling() {
        // 尺度 = 0.7、+0.5、+0.5（do-while 三值），非独立字面量
        assert_eq!(SPOT_SCALES[0], 0.7f32);
        assert_eq!(SPOT_SCALES[1], 0.7f32 + 0.5f32);
        assert_eq!(SPOT_SCALES[2], 0.7f32 + 1.0f32);
        // 下一档 2.2 已过 2.0 上界 ⇒ 恰三档
        assert!(0.7f32 + 1.5f32 > 2.0f32);
    }
}
