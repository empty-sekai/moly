//! The glyph atlas the UI's text and the balloons draw from: the baked glyph
//! cells, the atlas pages, and the background and arrow source images.

use bevy::prelude::*;
use std::collections::HashMap;

/// The balloon layer's render layer number: the main 3D camera sees only the
/// default layer 0, and the overlay camera sees only this layer.
pub const BALLOON_LAYER: usize = 1;

/// The atlas bake's font size (px/em): the display size's own value, so the
/// glyphs reach the screen 1:1.
pub const BAKE_PPEM: f32 = 32.0;

/// The layer a bubble is raised to while a screen covers the field: a
/// screen's UI layer sits above the HUD layer, so a bubble left on
/// [`BALLOON_LAYER`] would draw over the screen.
pub const SCREEN_UI_LAYER: usize = 2;

/// Code points TMP lays out with its synthesized zero-metric glyph: zero
/// advance, nothing drawn. They are not line breaks (the walk splits on
/// U+000A only) and not an image the open font could carry.
pub const TMP_ZERO_METRIC: [char; 6] = [
    '\u{0003}', '\u{061C}', '\u{200B}', '\u{200E}', '\u{200F}', '\u{2060}',
];

/// Whether TMP lays `ch` out with its synthesized zero-metric glyph (see
/// [`TMP_ZERO_METRIC`]); the open font's own glyph, if it has one, is not
/// what the source measures or draws.
pub fn tmp_zero_metric(ch: char) -> bool {
    TMP_ZERO_METRIC.contains(&ch)
}

/// One glyph's cell in the atlas: the cell's rectangle (image pixel
/// coordinates, min.y measured from the image's top) and its advance at the
/// base font size.
pub struct GlyphCell {
    pub rect: Rect,
    pub advance: f32,
    pub page: usize,
}

/// Every texture a balloon draws with: the glyph atlas, the background
/// 9-slice source image and the arrow source image.
#[derive(Resource)]
pub struct BalloonArt {
    /// The glyph atlas pages (white ink + coverage alpha; the sprite tints
    /// it). At least one page.
    pub images: Vec<Handle<Image>>,
    pub cells: HashMap<char, GlyphCell>,
    /// The cell's baseline row's distance from the cell's top and the pen
    /// point's column offset (placement reads them, in the bake's own form).
    pub baseline_from_top: f32,
    pub pen_x: f32,
    /// The cell's edge length (the two-pass bake sets it from the actual
    /// ink; see `bake_atlas` in `balloon`).
    pub cell: f32,
    /// The background 9-slice source image.
    pub bg: Handle<Image>,
    /// The arrow source image.
    pub arrow: Handle<Image>,
}

impl BalloonArt {
    /// A character's glyph cell (the rectangle in the atlas image's pixel
    /// coordinates plus the base advance). A character that was not baked
    /// returns `None` (the bake names every missing glyph and counts it).
    pub fn glyph_cell(&self, ch: char) -> Option<(Rect, f32)> {
        self.cells.get(&ch).map(|c| (c.rect, c.advance))
    }

    /// The base advance the line measurement reads for `ch`: zero for the
    /// code points TMP lays out with its synthesized zero-metric glyph, the
    /// baked glyph's advance otherwise (`None` = missing, the law's fallback).
    pub fn advance(&self, ch: char) -> Option<f32> {
        if tmp_zero_metric(ch) {
            return Some(0.0);
        }
        self.cells.get(&ch).map(|c| c.advance)
    }

    /// The first page's identity, for the whole atlas's cache invalidation;
    /// drawing a character takes [`Self::glyph_image_for`].
    pub fn glyph_image(&self) -> &Handle<Image> {
        &self.images[0]
    }

    /// The texture handle of the page a character was baked into; pair it
    /// with the same character's [`Self::glyph_cell`] rectangle. Callers
    /// handle a missing glyph through `glyph_cell` first; the first page is
    /// not a substitute for a glyph on a later page.
    pub fn glyph_image_for(&self, ch: char) -> &Handle<Image> {
        let cell = self
            .cells
            .get(&ch)
            .expect("glyph_image_for requires a baked glyph");
        &self.images[cell.page]
    }

    /// The cell geometry: the cell's edge length, the pen point's column
    /// offset and the baseline's distance from the cell's top. Placement
    /// converts with the same form as `tick`.
    pub fn cell_geometry(&self) -> (f32, f32, f32) {
        (self.cell, self.pen_x, self.baseline_from_top)
    }
}

/// A log guard: non-ASCII content becomes a placeholder, so a name that is
/// not ASCII never reaches a log line.
pub fn ascii_or(value: &str) -> &str {
    if value.is_ascii() {
        value
    } else {
        "<non-ascii>"
    }
}
