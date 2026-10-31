//! The item select list's grid and its scroll.
//!
//! `MysekaiConvertFurnitureItemSelectListView` lays its cells out with a
//! `FixedSizeReuseScroller` over a vertical `CustomScrollRect`: the cells
//! fill the content column in rows of `splitCount`, the content grows with
//! its rows, and `Refresh(count, resetContentPosition)` either puts the
//! content back at its top or keeps the scroll where it is.
//!
//! The placement follows the scroller's serialized grid (`layoutInfo`
//! spacing and split, the padding) measured from the content's own top-left
//! pivot: a cell at index `i` sits at `padding.left + column * (cell.x +
//! spacing.x)` across and `row * (cell.y + spacing.y)` down. The cells are a
//! reuse pool: the page asks which data indices the viewport shows at the
//! current offset and draws each pool slot with one of them.
//!
//! Named: the source's inertia and elastic movement are not run (a drag
//! follows the pointer and the content stops at its ends), and the vertical
//! bar's handle is not moved.

use bevy::prelude::*;
use moly_assets::ui_layout::{RectTransform, UiPrefab};
use serde_json::Value;

/// The source's drag threshold: `min(screen width, screen height) / 100` on
/// a 1080-high canvas (the gesture layer freezes the same value once).
pub const DRAG_THRESHOLD: f32 = 10.8;

/// `FixedSizeReuseScroller`'s grid over the list's content.
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    /// One cell's size.
    pub cell: Vec2,
    /// `layoutInfo.itemSpacing`.
    pub spacing: Vec2,
    /// `layoutInfo.splitCount`.
    pub columns: usize,
    /// `padding` (left, top, right, bottom).
    pub padding: [f32; 4],
    /// `m_ScrollSensitivity`.
    pub sensitivity: f32,
    /// The source's drag threshold.
    pub drag_threshold: f32,
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| value as f32)
        .ok_or_else(|| format!("{what} is not a number"))
}

fn pair(value: Option<&Value>, what: &str) -> Result<Vec2, String> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == 2)
        .ok_or_else(|| format!("{what} is not a pair"))?;
    Ok(Vec2::new(
        number(&values[0], what)?,
        number(&values[1], what)?,
    ))
}

fn component<'a>(
    doc: &'a UiPrefab,
    path: &str,
    suffix: &str,
) -> Result<&'a moly_assets::ui_layout::UiComponent, String> {
    let node = &doc.nodes[doc.find(path)?];
    node.components
        .iter()
        .find(|component| component.class.ends_with(suffix))
        .ok_or_else(|| format!("{path} has no {suffix}"))
}

impl Grid {
    /// The list's serialized scroller and the cell template's size.
    pub fn read(doc: &UiPrefab, list: &str, cell: Vec2) -> Result<Self, String> {
        let scroller = component(doc, list, "FixedSizeReuseScroller")?;
        let layout = &scroller.fields["layoutInfo"];
        let columns = layout["splitCount"]
            .as_u64()
            .filter(|count| *count > 0)
            .map_or(3, |count| count as usize);
        let spacing = pair(layout.get("itemSpacing"), "layoutInfo.itemSpacing")?;
        let padding_values = scroller.fields["padding"]
            .as_array()
            .filter(|values| values.len() == 4)
            .ok_or("the scroller's padding is not four numbers")?;
        let mut padding = [0.0; 4];
        for (out, value) in padding.iter_mut().zip(padding_values) {
            *out = number(value, "padding")?;
        }
        let scroll = component(doc, list, "CustomScrollRect")?;
        // `ScrollRect.m_ScrollSensitivity`: the scroll rect beside the
        // scroller carries the authored value.
        let sensitivity = match scroll.fields.get("m_ScrollSensitivity") {
            Some(value) if !value.is_null() => number(value, "m_ScrollSensitivity")?,
            _ => 1.0,
        };
        Ok(Self {
            cell,
            spacing,
            columns,
            padding,
            sensitivity,
            drag_threshold: DRAG_THRESHOLD,
        })
    }

    /// The content's height for `count` cells.
    pub fn content_height(&self, count: usize) -> f32 {
        let rows = count.div_ceil(self.columns);
        if rows == 0 {
            return 0.0;
        }
        let gaps = rows.saturating_sub(1) as f32;
        rows as f32 * self.cell.y + gaps * self.spacing.y + self.padding[1] + self.padding[3]
    }

    /// A cell's anchored position from the content's top-left pivot.
    pub fn slot(&self, index: usize) -> Vec2 {
        let row = index / self.columns;
        let column = index % self.columns;
        Vec2::new(
            self.padding[0] + column as f32 * (self.cell.x + self.spacing.x),
            -(self.padding[1] + row as f32 * (self.cell.y + self.spacing.y)),
        )
    }

    /// The data indices the viewport at `offset` shows, in data order.
    pub fn visible(&self, offset: f32, viewport_height: f32, count: usize) -> Vec<usize> {
        (0..count)
            .filter(|index| {
                let top = -self.slot(*index).y;
                top + self.cell.y > offset && top < offset + viewport_height
            })
            .collect()
    }

    /// The scroll's largest offset.
    pub fn max_offset(&self, count: usize, viewport_height: f32) -> f32 {
        (self.content_height(count) - viewport_height).max(0.0)
    }
}

/// The content's authored anchored position, which the scroll moves.
pub fn content_position(doc: &UiPrefab, content: &str) -> Result<Vec2, String> {
    let index = doc.find(content)?;
    let rect: &RectTransform = &doc.nodes[index].rect;
    Ok(Vec2::from_array(rect.anchored_position))
}

/// The cell template's size (its root's `sizeDelta`).
pub fn template_size(doc: &UiPrefab, root: &str) -> Result<Vec2, String> {
    let index = doc.find(root)?;
    Ok(Vec2::from_array(doc.nodes[index].rect.size_delta))
}

// ---------------------------------------------------------------------------
// Scrolling
// ---------------------------------------------------------------------------

/// The list's scroll and the pointer on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scroll {
    /// The content's top from the viewport's top.
    pub offset: f32,
    /// The pointer that went down inside the viewport, where, and how far it
    /// has come.
    pressing: Option<(i32, Vec2, f32)>,
    /// The pointer that scrolls, and where it last was.
    dragging: Option<(i32, Vec2)>,
}

impl Scroll {
    /// `Refresh(resetContentPosition)`.
    pub fn refresh(&mut self, reset: bool) {
        if reset {
            self.offset = 0.0;
            self.pressing = None;
            self.dragging = None;
        }
    }

    /// Keeps the content inside its ends.
    pub fn settle(&mut self, max: f32) {
        self.offset = self.offset.clamp(0.0, max);
    }

    /// A press inside the viewport.
    pub fn pressed(&mut self, pointer: i32, point: Vec2, inside: bool) {
        self.pressing = inside.then_some((pointer, point, 0.0));
    }

    /// `OnDrag`: a press that moves past the threshold scrolls; answers
    /// whether the pointer scrolls now.
    pub fn moved(&mut self, pointer: i32, point: Vec2, grid: &Grid) -> bool {
        if let Some((id, last)) = self.dragging.as_mut() {
            if *id == pointer {
                self.offset -= point.y - last.y;
                *last = point;
                return true;
            }
        }
        let Some((id, start, travelled)) = self.pressing.as_mut() else {
            return false;
        };
        if *id != pointer {
            return false;
        }
        *travelled = travelled.max((*start - point).length());
        if *travelled > grid.drag_threshold {
            self.dragging = Some((pointer, point));
            self.offset -= point.y - start.y;
            return true;
        }
        false
    }

    /// The pointer's release or loss.
    pub fn released(&mut self, pointer: i32) {
        if self.pressing.is_some_and(|(id, _, _)| id == pointer) {
            self.pressing = None;
        }
        if self.dragging.is_some_and(|(id, _)| id == pointer) {
            self.dragging = None;
        }
    }

    /// The wheel over the viewport.
    pub fn wheel(&mut self, inside: bool, lines: f32, grid: &Grid) {
        if inside {
            self.offset -= lines * (grid.cell.y + grid.spacing.y) * grid.sensitivity;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: Grid = Grid {
        cell: Vec2::new(152.0, 152.0),
        spacing: Vec2::new(28.0, 28.0),
        columns: 3,
        padding: [16.0, 0.0, 16.0, 0.0],
        sensitivity: 1.0,
        drag_threshold: 10.8,
    };

    #[test]
    fn a_grid_places_rows_of_three_and_sizes_its_content() {
        // 4 cells: two rows of 152 and one 28 gap.
        assert_eq!(GRID.content_height(4), 332.0);
        assert_eq!(GRID.content_height(0), 0.0);
        assert_eq!(GRID.slot(0), Vec2::new(16.0, 0.0));
        assert_eq!(GRID.slot(2), Vec2::new(16.0 + 2.0 * 180.0, 0.0));
        assert_eq!(GRID.slot(3), Vec2::new(16.0, -180.0));
        assert_eq!(GRID.max_offset(4, 300.0), 32.0);
        assert_eq!(GRID.max_offset(1, 300.0), 0.0);
    }

    #[test]
    fn the_viewport_shows_the_rows_it_covers() {
        // At the top of a 300-high viewport rows 0 and 1 show (indices 0..6).
        assert_eq!(GRID.visible(0.0, 300.0, 6), vec![0, 1, 2, 3, 4, 5]);
        // Scrolled one row down, the first row leaves.
        assert_eq!(GRID.visible(180.0, 300.0, 6), vec![3, 4, 5]);
    }

    #[test]
    fn a_refresh_resets_the_scroll_and_a_selection_keeps_it() {
        let mut scroll = Scroll::default();
        scroll.offset = 120.0;
        scroll.refresh(false);
        assert_eq!(scroll.offset, 120.0);
        scroll.refresh(true);
        assert_eq!(scroll.offset, 0.0);
        scroll.offset = -40.0;
        scroll.settle(0.0);
        assert_eq!(scroll.offset, 0.0);
    }
}
