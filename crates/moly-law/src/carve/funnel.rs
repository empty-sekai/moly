//! 漏斗整直：把「按序穿过的一串门」收成拐点序列。
//!
//! # 它在链条里的位置
//!
//! 原生查询侧是 Detour：`findPath` 在多边形图上做 A\*，给出一串多边形；
//! `findStraightPath` 再把这串多边形之间的**公共边（门）**收成拐点。托管层
//! `NavMeshAgent.CalculatePath` 返回的 `corners` 就是后者的产物——不是格链。
//!
//! # 锚点档次
//!
//! ⚠ **本文件锚的是 upstream Detour，不是反编译。** 烘焙侧（分区/轮廓/多边形网）
//! 的原生函数有 27 个导出可读，**查询侧一个都没有**——它们在 Detour 那半边，
//! 不在本次导出的搜索空间里。所以这里写的是公开算法的转录，档次是「知识库」，
//! 低于本仓其它地方的「反编译」。落这条注释是为了后来的人能一眼看出
//! 哪些结论可以照抄、哪些需要回真源重新导出。
//!
//! # 形状
//!
//! 自起点起维护一个由左右两条射线夹出的楔形（漏斗）。逐门收紧：新的左点
//! 若没有越过右边界就收紧左边界，越过了就说明右点是一个真拐点——把右点吐
//! 出来，漏斗从它重新张开。右侧对称。

/// 二维叉积：`(b-a) × (c-a)`。正号表示 c 在 a→b 的左侧。
///
/// ⚠ 漏斗内部的判据由此定死：左边界 `L` 在上、右边界 `R` 在下时，
/// 点 `P` 在漏斗内 ⟺ `cross(A,L,P) <= 0 ∧ cross(A,R,P) >= 0`
/// （在左边界的右侧、且在右边界的左侧）。下面四处比较全按这条写。
/// upstream Detour 的 `triarea2` 是 `-cross`，照抄它的不等号方向会整个反掉。
fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// 门：穿过它时的左点与右点（相对于前进方向）。
pub(crate) type Portal = [[f32; 2]; 2];

/// 漏斗整直。
///
/// `portals` 是按行进顺序排好的门。返回的拐点序列首点恒为 `start`、末点恒为
/// `end`；中间每个拐点都是某道门的一个端点。
///
/// 退化的门（左右两点重合）不特殊处理：它只是让漏斗在那一处收成一条线，
/// 算法自然继续——这与「把它当成拐点吐出来」不同，后者会造出原本不存在的
/// 折点。
pub(crate) fn straight_path(start: [f32; 2], end: [f32; 2], portals: &[Portal]) -> Vec<[f32; 2]> {
    let mut out = vec![start];
    // 漏斗顶点与左右边界；末尾补一道退化门代表终点。
    let mut apex = start;
    let (mut left, mut right) = (start, start);
    let (mut left_at, mut right_at) = (0usize, 0usize);
    let mut index = 0usize;
    while index < portals.len() + 1 {
        let (portal_left, portal_right) = if index < portals.len() {
            (portals[index][0], portals[index][1])
        } else {
            (end, end)
        };

        // —— 收紧右边界：新右点在旧右边界的左侧（或重合）才算收紧 ——
        if cross(apex, right, portal_right) >= 0.0 {
            if apex == right || cross(apex, left, portal_right) <= 0.0 {
                right = portal_right;
                right_at = index;
            } else {
                // 右边界越过了左边界 ⇒ 左点是拐点。
                if out.last() != Some(&left) {
                    out.push(left);
                }
                apex = left;
                right = apex;
                left = apex;
                right_at = left_at;
                index = left_at + 1;
                continue;
            }
        }
        // —— 收紧左边界：新左点在旧左边界的右侧（或重合）才算收紧 ——
        if cross(apex, left, portal_left) <= 0.0 {
            if apex == left || cross(apex, right, portal_left) >= 0.0 {
                left = portal_left;
                left_at = index;
            } else {
                if out.last() != Some(&right) {
                    out.push(right);
                }
                apex = right;
                left = apex;
                right = apex;
                left_at = right_at;
                index = right_at + 1;
                continue;
            }
        }
        index += 1;
    }
    if out.last() != Some(&end) {
        out.push(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 成对判据：一串把通道逼到同一侧的门**必须**产出中间拐点；
    /// 同一对起终点在通道张开时**必须**整直成直达两点。
    ///
    /// 反向臂在这里是承重的：只看正向臂的话，一个「原样返回每道门中点」的
    /// 错误实现也会有中间点，看起来同样像绕过了拐角。
    ///
    /// ⚠ 正向臂的门必须**真的**挡住那条直线。起终点 (0,0.5)→(3,1.8) 的直线
    /// 在 x=2 处 y=1.367，越过了第三道门压到 1.0 的左边界 ⇒ 必须在 (2,1) 拐。
    /// （初稿终点写成 (3,0.9)，直线在 x=2 处才 0.767，仍在门内——那串门根本
    /// 没逼人拐弯，正向臂量的是一条本来就直达的路。）
    #[test]
    fn funnel_turns_only_where_the_corridor_forces_it() {
        let start = [0.0, 0.5];
        let end = [3.0, 1.8];
        // 走廊向右（左侧 = +y）。第三道门把左边界压到 y=1。
        let pinched = [
            [[0.0, 2.0], [0.0, 0.0]],
            [[1.0, 2.0], [1.0, 0.0]],
            [[2.0, 1.0], [2.0, 0.0]],
            [[3.0, 2.0], [3.0, 0.0]],
        ];
        let path = straight_path(start, end, &pinched);
        assert_eq!(path, vec![start, [2.0, 1.0], end], "应在被压低的门角上拐");

        // 反向臂：同样四道门，左边界不压低 ⇒ 同一对起终点整直成直达。
        let open = [
            [[0.0, 2.0], [0.0, 0.0]],
            [[1.0, 2.0], [1.0, 0.0]],
            [[2.0, 2.0], [2.0, 0.0]],
            [[3.0, 2.0], [3.0, 0.0]],
        ];
        assert_eq!(straight_path(start, end, &open), vec![start, end], "张开的走廊应整直");
    }
}
