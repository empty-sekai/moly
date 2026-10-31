//! Text layout rules: line breaking, rich-text tags, full/half width and line
//! measures as TextMeshPro defines them, as pure functions.
//!
//! Four parts: [`tags`] parses rich text into segments (one branch per tag,
//! characters merged into segments); [`advance`] gives each character's advance
//! width (full/half-width code point table, space advance ratios, missing-glyph
//! fallback); [`metrics`] assembles segments into line measures
//! (line width/visible width/box width/anchor baseline/line offset); [`plain`]
//! strips tagged text to plain text and finds numeric runs.
//!
//! This module handles only hard line breaks: `\n`, `<br>`, `<cr>`. That does
//! not mean the game has no soft wrapping — TextMeshPro with
//! `enableWordWrapping` wraps at the line width (line-break table, U+00A0 does
//! not break), and the UI layout's soft wrapping is implemented in moly-game's
//! TMP layout; the layout rules here cover only hard breaks, so consumers that
//! need soft wrapping should not take it from here.
//!
//! Glyph lookup takes a closure: the game looks glyphs up in the atlas by the
//! tuple "region + character style + font family + character", the first three
//! of which are constant within one layout, which is equivalent to a lookup by
//! character. The closure returns advances **at the base font size**; scaling
//! to the current font size happens in this module.

pub mod advance;
pub mod auto_size;
pub mod custom_text_mesh;
pub mod metrics;
pub mod plain;
pub mod tags;

pub use advance::{fallback_advance, is_fullwidth, resolve_glyph_advance, space_advance_ratio};
pub use metrics::{layout_metrics, LayoutMetrics};
pub use plain::{numeric_text_runs, strip_tmp_tags, NumericTextRun};
pub use tags::{
    glyph_demand_chars, parse_rich_segments, transform_char, transformed_glyphs, CaseTransform,
    Indent, LineIndent, SizeSpec, TextSegment,
};
