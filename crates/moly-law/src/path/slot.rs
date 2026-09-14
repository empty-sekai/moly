//! 一个行走 NPC 的路径，按行携带。
//!
//! 行走律每次收一行：该 NPC 的拐点表，按行走顺序。用行而不是「拐点池 +
//! 偏移列」，是因为池形状带着一条类型层无法强制的不变式——偏移列必须恰比
//! 行数长一、且从零开始——对不齐的池会用别的 NPC 的拐点应答而不是报错；
//! 行不会错位，拐点随行走者走。
//!
//! 拐点刻意**不含**行走者自身位置。真源暴露的路径在下标 0 带着行走者的
//! 位置（它的朝向代码于是读下标 1 当「前方拐点」），但那个位置是行走律
//! 自己的跨帧状态，不是路径生产者该供给的东西——把状态抄进输出，只能抄到
//! 一份滞留副本。行走律从持久位置出发按序穿过这些拐点，行的第 k 个拐点
//! 对应源路径的第 k+1 个。

/// 一个行走 NPC 的有序拐点表，不含行走者自身位置（理由见模块注释）。
///
/// 空表是合法行：表示该 NPC 本帧没有活动路径，不是错误。恰好一个拐点
/// 同样合法——直走到一个目的地。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NpcPathWalkSlot {
    corners: Vec<[f32; 3]>,
}

impl NpcPathWalkSlot {
    /// 按行走顺序，从拐点建行。
    pub fn from_corners(corners: Vec<[f32; 3]>) -> Self {
        Self { corners }
    }

    /// 拐点，按行走顺序。可能为空——那是「无活动路径」的情形，
    /// 行走律按常态处理它。
    pub fn corners(&self) -> &[[f32; 3]] {
        &self.corners
    }

    /// 行内拐点数。
    pub fn len(&self) -> usize {
        self.corners.len()
    }

    /// 行是否不含任何拐点。
    pub fn is_empty(&self) -> bool {
        self.corners.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_keeps_its_corners_in_order_and_reports_them_back() {
        let row = NpcPathWalkSlot::from_corners(vec![[1.0, 0.0, 0.0], [1.0, 2.0, 0.0]]);
        assert_eq!(row.len(), 2);
        assert!(!row.is_empty());
        assert_eq!(row.corners(), &[[1.0, 0.0, 0.0], [1.0, 2.0, 0.0]]);
    }

    #[test]
    fn an_empty_row_is_legal_and_reports_itself_as_empty() {
        let row = NpcPathWalkSlot::from_corners(Vec::new());
        assert!(row.is_empty());
        assert_eq!(row.len(), 0);
        let empty: &[[f32; 3]] = &[];
        assert_eq!(row.corners(), empty);
    }
}
