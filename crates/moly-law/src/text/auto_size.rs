//! TextMeshPro auto-size search (TMP 3.0.x, the game's runtime).
//!
//! Rendering starts the search at the base size clamped to [min, max];
//! the preferred-size queries start at the max size instead. Each generation
//! pass either finishes or changes one quantity and runs again, up to 100
//! passes (the iteration counter is the number of finished passes):
//!
//! * text too tall: first pull the line spacing in (only while the spacing
//!   delta is above its limit and the current line is not the first), else
//!   shrink the point size;
//! * a word too wide on the first word of a line (wrapping) or any overflow
//!   without wrapping: first narrow the characters (while the width delta is
//!   below its limit), else shrink the point size;
//! * everything fits but the search span is still wider than 0.051 and the
//!   size is below max: grow the point size, resetting the character width
//!   delta when it is below its limit.
//!
//! Shrinking halves the distance to the lower bound (at least 0.05), snaps
//! to the 1/20 point grid by truncating `size * 20 + 0.5`, and never goes
//! below min; growing mirrors it toward the upper bound and never exceeds max.

use crate::ui::unity_math;

/// `TMP_Text.m_AutoSizeMaxIterationCount` default.
pub const MAX_ITERATION_COUNT: u32 = 100;

/// Smallest point-size step of the search.
const MIN_STEP: f32 = 0.05;

/// The point-size grid (1/20 point).
const GRID: f32 = 20.0;

/// Search span below which growing stops.
const CONVERGED_SPAN: f32 = 0.051;

/// The first point size of a render pass.
pub fn render_start_size(auto_size: bool, font_size: f32, font_size_base: f32, min: f32, max: f32) -> f32 {
    if auto_size {
        unity_math::clamp(font_size_base, min, max)
    } else {
        font_size
    }
}

/// The first point size of a preferred-width/height query.
pub fn preferred_start_size(auto_size: bool, font_size: f32, max: f32) -> f32 {
    if auto_size {
        max
    } else {
        font_size
    }
}

/// The search state TMP keeps across generation passes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoSize {
    /// Current point size.
    pub font_size: f32,
    /// Lower bound of the search.
    pub min_font_size: f32,
    /// Upper bound of the search.
    pub max_font_size: f32,
    /// Character width reduction (0..=char_width_max_adj / 100).
    pub char_width_adj_delta: f32,
    /// Line spacing reduction (at most 0, never below the limit).
    pub line_spacing_delta: f32,
    /// Finished generation passes.
    pub iteration: u32,
}

impl AutoSize {
    /// The reset before the first pass.
    pub fn new(start: f32, font_size_min: f32, font_size_max: f32) -> Self {
        Self {
            font_size: start,
            min_font_size: font_size_min,
            max_font_size: font_size_max,
            char_width_adj_delta: 0.0,
            line_spacing_delta: 0.0,
            iteration: 0,
        }
    }

    /// Another resize is still allowed in this pass.
    pub fn below_iteration_limit(&self) -> bool {
        self.iteration < MAX_ITERATION_COUNT
    }

    /// The character-width limit as a fraction.
    pub fn char_width_limit(char_width_max_adj_percent: f32) -> f32 {
        char_width_max_adj_percent / 100.0
    }

    /// Line spacing can still be pulled in for a too-tall text.
    pub fn may_reduce_line_spacing(&self, line_spacing_max: f32, line_offset_positive: bool) -> bool {
        self.line_spacing_delta > line_spacing_max && line_offset_positive && self.below_iteration_limit()
    }

    /// Pull the line spacing in so the text height meets the margin height.
    pub fn reduce_line_spacing(
        &mut self,
        margin_height: f32,
        text_height: f32,
        line_number: u32,
        base_scale: f32,
        line_spacing_max: f32,
    ) {
        let adjustment_delta = (margin_height - text_height) / line_number as f32;
        self.line_spacing_delta =
            unity_math::max(self.line_spacing_delta + adjustment_delta / base_scale, line_spacing_max);
    }

    /// Characters can still be narrowed for a too-wide word.
    pub fn may_reduce_char_width(&self, char_width_max_adj_percent: f32) -> bool {
        self.char_width_adj_delta < Self::char_width_limit(char_width_max_adj_percent)
    }

    /// Narrow the characters so the text width meets the text area width.
    pub fn reduce_char_width(
        &mut self,
        text_width: f32,
        width_of_text_area: f32,
        justified_or_flush: bool,
        char_width_max_adj_percent: f32,
    ) {
        let mut adjusted_text_width = text_width;
        if self.char_width_adj_delta > 0.0 {
            adjusted_text_width /= 1.0 - self.char_width_adj_delta;
        }
        let adjustment_delta =
            text_width - (width_of_text_area - 0.0001) * if justified_or_flush { 1.05 } else { 1.0 };
        self.char_width_adj_delta += adjustment_delta / adjusted_text_width;
        self.char_width_adj_delta = unity_math::min(
            self.char_width_adj_delta,
            Self::char_width_limit(char_width_max_adj_percent),
        );
    }

    /// The point size can still shrink.
    pub fn may_reduce_point_size(&self, font_size_min: f32) -> bool {
        self.font_size > font_size_min
    }

    /// Shrink the point size toward the lower bound.
    pub fn reduce_point_size(&mut self, font_size_min: f32) {
        self.max_font_size = self.font_size;
        let size_delta = unity_math::max((self.font_size - self.min_font_size) / 2.0, MIN_STEP);
        self.font_size -= size_delta;
        self.font_size =
            unity_math::max((self.font_size * GRID + 0.5) as i32 as f32 / GRID, font_size_min);
    }

    /// Everything fit: the search can still grow.
    pub fn may_increase_point_size(&self, auto_size: bool, font_size_max: f32) -> bool {
        auto_size
            && self.max_font_size - self.min_font_size > CONVERGED_SPAN
            && self.font_size < font_size_max
            && self.below_iteration_limit()
    }

    /// Grow the point size toward the upper bound.
    pub fn increase_point_size(&mut self, font_size_max: f32, char_width_max_adj_percent: f32) {
        if self.char_width_adj_delta < Self::char_width_limit(char_width_max_adj_percent) {
            self.char_width_adj_delta = 0.0;
        }
        self.min_font_size = self.font_size;
        let size_delta = unity_math::max((self.max_font_size - self.font_size) / 2.0, MIN_STEP);
        self.font_size += size_delta;
        self.font_size =
            unity_math::min((self.font_size * GRID + 0.5) as i32 as f32 / GRID, font_size_max);
    }
}
