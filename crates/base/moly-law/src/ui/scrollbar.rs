//! ScrollRect scrollbar geometry from UGUI: visibility, thumb size and anchors.
//! The values are content/view bounds in the same local units. Hosts retain
//! pointer ownership and provide the source scrollbar's direction.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
    BottomToTop,
    TopToBottom,
}

impl Direction {
    pub fn from_source(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::LeftToRight),
            1 => Some(Self::RightToLeft),
            2 => Some(Self::BottomToTop),
            3 => Some(Self::TopToBottom),
            _ => None,
        }
    }

    pub fn axis(self) -> usize {
        match self {
            Self::LeftToRight | Self::RightToLeft => 0,
            Self::BottomToTop | Self::TopToBottom => 1,
        }
    }

    pub fn reversed(self) -> bool {
        matches!(self, Self::RightToLeft | Self::TopToBottom)
    }
}

/// ScrollRect.get_vScrollingNeeded while playing (also used for horizontal).
pub fn scrolling_needed(content_extent: f32, view_extent: f32) -> bool {
    content_extent > view_extent + 0.01
}

/// ScrollRect.UpdateScrollbars; overscroll shrinks the thumb at either edge.
pub fn thumb_size(content_extent: f32, view_extent: f32, offset: f32) -> f32 {
    if content_extent <= 0.0 {
        1.0
    } else {
        ((view_extent - offset.abs()) / content_extent).clamp(0.0, 1.0)
    }
}

/// Scrollbar.UpdateVisuals writes only the handle anchors. The source handle
/// sizeDelta, anchoredPosition and pivot must remain unchanged.
pub fn handle_anchors(direction: Direction, value: f32, size: f32) -> ([f32; 2], [f32; 2]) {
    let mut min = [0.0; 2];
    let mut max = [1.0; 2];
    let movement = (1.0 - size) * value.clamp(0.0, 1.0);
    let axis = direction.axis();
    if direction.reversed() {
        max[axis] = 1.0 - movement;
        min[axis] = max[axis] - size;
    } else {
        min[axis] = movement;
        max[axis] = size + movement;
    }
    (min, max)
}

/// Scrollbar.UpdateDrag / DoUpdateDrag, after the pointer is converted to the
/// container's local axis. `handle_corner` already subtracts the captured grab
/// offset and half of the handle's anchored span (excluding sizeDelta).
pub fn drag_value(direction: Direction, handle_corner: f32, remaining_size: f32) -> Option<f32> {
    if remaining_size <= 0.0 {
        return None;
    }
    let ratio = handle_corner / remaining_size;
    Some(if direction.reversed() { 1.0 - ratio } else { ratio }.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_content_hides_scrollbar_and_does_not_drag() {
        assert!(!scrolling_needed(724.0, 773.0));
        assert_eq!(thumb_size(724.0, 773.0, 0.0), 1.0);
        assert_eq!(drag_value(Direction::BottomToTop, 0.0, 0.0), None);
        assert!(!scrolling_needed(100.01, 100.0));
        assert!(scrolling_needed(100.02, 100.0));
    }

    #[test]
    fn overflowing_content_thumb_reaches_both_ends_and_middle() {
        let size = thumb_size(800.0, 200.0, 0.0);
        assert_eq!(size, 0.25);
        assert_eq!(handle_anchors(Direction::BottomToTop, 1.0, size), ([0.0, 0.75], [1.0, 1.0]));
        assert_eq!(handle_anchors(Direction::BottomToTop, 0.0, size), ([0.0, 0.0], [1.0, 0.25]));
        assert_eq!(handle_anchors(Direction::BottomToTop, 0.5, size), ([0.0, 0.375], [1.0, 0.625]));
        assert_eq!(thumb_size(800.0, 200.0, -40.0), 0.2);
    }

    #[test]
    fn drag_uses_thumb_travel_and_preserves_direction() {
        assert_eq!(drag_value(Direction::BottomToTop, 75.0, 150.0), Some(0.5));
        assert_eq!(drag_value(Direction::BottomToTop, 200.0, 150.0), Some(1.0));
        assert_eq!(drag_value(Direction::TopToBottom, 30.0, 150.0), Some(0.8));
        assert_eq!(handle_anchors(Direction::RightToLeft, 1.0, 0.25), ([0.0, 0.0], [0.25, 1.0]));
    }
}
