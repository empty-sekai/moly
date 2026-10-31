//! The path of one walking NPC, carried as a row.
//!
//! The walking rule takes one row at a time: that NPC's corner table, in
//! walking order. A row is used instead of "a corner pool + an offset column"
//! because the pool shape carries an invariant the type layer cannot enforce —
//! the offset column must be exactly one longer than the row count and start
//! at zero — and a misaligned pool answers with another NPC's corners instead
//! of failing; a row cannot shift, and its corners travel with the walker.
//!
//! The corners deliberately **exclude** the walker's own position. A path
//! carrying the walker's position at index 0 (whose heading code therefore
//! reads index 1 as the "forward corner") would embed the walking rule's own
//! cross-frame state, which a path producer should not supply — copying state
//! into the output only reproduces a stale copy. The walking rule starts from
//! the persistent position and passes the corners in order: corner k of the
//! row corresponds to corner k+1 of the path with the walker's position
//! removed.

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
