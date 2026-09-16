//! Read-only geometry of the authored dialogue tree, not a second renderer.
use super::*;

#[derive(Component)]
pub(crate) struct ResponsiveDialogueMetrics {
    pub(crate) font_px: f32,
    pub(crate) line_count: usize,
    pub(crate) bounds: Rect,
}

pub(super) fn source_metrics(placement: Transform, text: &str) -> ResponsiveDialogueMetrics {
    let center = placement.transform_point(PANEL_CENTER.extend(0.)).truncate();
    let half = Vec2::new(PANEL_W, PANEL_H) * placement.scale.truncate() * 0.5;
    ResponsiveDialogueMetrics {
        font_px: CONTENT_FONT * placement.scale.y,
        line_count: text.split('\n').count(),
        bounds: Rect::from_corners(center - half, center + half),
    }
}
