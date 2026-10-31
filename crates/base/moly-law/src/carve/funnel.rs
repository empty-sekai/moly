//! Funnel straightening: reduces "a sequence of portals passed in order" to a
//! sequence of corners.
//!
//! # Its place in the chain
//!
//! The query side is Detour: `findPath` runs A\* on the polygon mesh and
//! returns a sequence of polygons; `findStraightPath` then reduces the
//! **shared edges (portals)** between those polygons to corners. The
//! `corners` returned by the managed `NavMeshAgent.CalculatePath` are the
//! product of the latter — not a cell chain.
//!
//! # Algorithm
//!
//! ⚠ **This file implements Detour's straight path funnel.** The algorithm is
//! public, following upstream Detour.
//!
//! # Shape
//!
//! Starting from the origin, the funnel maintains a wedge bounded by a left
//! and a right ray. It tightens portal by portal: if the new left point does
//! not cross the right boundary, the left boundary tightens; if it does, the
//! right point is a true corner — emit it and reopen the funnel from it. The
//! right side is symmetric.

/// 2D cross product: `(b-a) × (c-a)`. A positive sign means c is to the left of
/// a→b.
///
/// ⚠ The inside test of the funnel follows from this: with the left boundary
/// `L` above and the right boundary `R` below, a point `P` is inside the
/// funnel ⟺ `cross(A,L,P) <= 0 ∧ cross(A,R,P) >= 0` (to the right of the left
/// boundary and to the left of the right boundary). All four comparisons
/// below follow this convention. Upstream Detour's `triarea2` is `-cross`, so
/// Detour's inequality directions would flip every test.
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

    /// Paired test: a series of portals that pin the corridor to one side
    /// **must** produce an intermediate corner; the same start and end
    /// points with an open corridor **must** straighten into a direct
    /// two-point path.
    ///
    /// The test also checks the opposite case: without the open-corridor
    /// case, a wrong implementation that returns every portal midpoint would
    /// also have intermediate points and look equally like it went around the
    /// corner.
    ///
    /// ⚠ The pinched portals must **really** block the straight line. The
    /// line from (0,0.5) to (3,1.8) has y=1.367 at x=2, crossing the left
    /// boundary of 1.0 pushed down by the third portal ⇒ it must turn at
    /// (2,1). (With the end at (3,0.9) the line has only y=0.767 at x=2, still
    /// inside the portals — that portal series never forced a turn and the
    /// test would measure a path that was direct anyway.)
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
