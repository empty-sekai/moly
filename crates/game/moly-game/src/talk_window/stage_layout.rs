//! Read-only geometry of the authored dialogue tree, not a second renderer.
use super::*;

#[derive(Component)]
pub(crate) struct ResponsiveDialogueMetrics {
    pub(crate) font_px: f32,
    pub(crate) line_count: usize,
    pub(crate) bounds: Rect,
}

/// The panel's resolved rect in logical window pixels (origin at the canvas
/// centre, y up) and the content text's font size in logical pixels.
pub(super) fn source_metrics(
    scale: f32,
    panel: &UiRect,
    font_size: f32,
    text: &str,
) -> ResponsiveDialogueMetrics {
    let min = -panel.pivot * panel.size;
    let max = min + panel.size;
    let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
        .map(|corner| panel.world.transform_point3(corner.extend(0.)).truncate() * scale);
    let low = corners.into_iter().reduce(Vec2::min).unwrap_or_default();
    let high = corners.into_iter().reduce(Vec2::max).unwrap_or_default();
    ResponsiveDialogueMetrics {
        font_px: font_size * scale,
        line_count: text.split('\n').count(),
        bounds: Rect::from_corners(low, high),
    }
}
