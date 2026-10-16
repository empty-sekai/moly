//! The game's `CustomTextMesh` additions to TextMeshPro.
//!
//! `SetText(value, breakSpace = false)` replaces every U+0020 with U+00A0
//! unless `breakSpace` is true, then assigns the text; a null value stays
//! null. Text set from a wording key is assigned directly and keeps its
//! spaces. U+00A0 never offers a TMP line break, so the default call turns
//! every space of a dynamic string into a non-breaking one.
//!
//! Width scale fit (`enableWidthScaleFit`): the original local X scale is
//! captured once. Each `LateUpdate`, when the text changed or the rect width
//! moved by more than 0.01 since the last recorded width, the component
//! measures the text width with the original X scale applied and, when that
//! width is positive and exceeds a positive rect width, sets the X scale to
//! `(rectWidth / textWidth) * originalScaleX`; otherwise it sets the
//! original X scale. The measured width is the preferred width; when that is
//! not positive, twice the text bounds' X extent; when that is not positive
//! either, the preferred values' X for the text, if positive. With the
//! option off, a text change restores the original X scale if it differs.

/// `CustomTextMesh.SetText`'s text transform.
pub fn set_text(value: &str, break_space: bool) -> String {
    if break_space {
        value.to_owned()
    } else {
        value.replace('\u{20}', "\u{a0}")
    }
}

/// `CustomTextMesh.UpdateWordingText`'s `String.Format(wording, args)`:
/// .NET composite formatting. `{{` and `}}` are literal braces; a format item
/// is `{index[,alignment]}`: the index (digits, read while it stays below
/// 1000000), optional spaces, then optionally a comma, optional spaces, an
/// optional minus and the alignment width (digits, read while it stays below
/// 1000000) and optional spaces, then the closing brace. The argument's text
/// is padded with spaces to the width (UTF-16 units), on the left, or on the
/// right with the minus. An index past the arguments, a lone brace or any
/// other character where the item expects one is a `FormatException` in the
/// engine and is refused here.
///
/// A `:format` part is not ported and is refused by name: the engine formats
/// the boxed argument with its own type's formatter under the current culture
/// (the wordings that use one pass numbers and dates), which these text
/// arguments do not carry.
pub fn format_wording(format: &str, args: &[String]) -> Result<String, String> {
    const LIMIT: usize = 1_000_000;
    let chars: Vec<char> = format.chars().collect();
    let len = chars.len();
    let error = |what: &str| Err(format!("wording format {format:?}: {what}"));
    let mut out = String::with_capacity(format.len());
    let mut pos = 0;
    loop {
        while pos < len {
            let ch = chars[pos];
            pos += 1;
            if ch == '}' {
                if pos < len && chars[pos] == '}' {
                    pos += 1;
                } else {
                    return error("a lone closing brace");
                }
            }
            if ch == '{' {
                if pos < len && chars[pos] == '{' {
                    pos += 1;
                } else {
                    pos -= 1;
                    break;
                }
            }
            out.push(ch);
        }
        if pos == len {
            return Ok(out);
        }
        // The item's opening brace.
        pos += 1;
        if pos == len || !chars[pos].is_ascii_digit() {
            return error("a format item without an index");
        }
        let mut ch = chars[pos];
        let mut index = 0usize;
        loop {
            index = index * 10 + (ch as usize - '0' as usize);
            pos += 1;
            if pos == len {
                return error("an unterminated format item");
            }
            ch = chars[pos];
            if !(ch.is_ascii_digit() && index < LIMIT) {
                break;
            }
        }
        if index >= args.len() {
            return error(&format!("index {index} with {} arguments", args.len()));
        }
        while pos < len && chars[pos] == ' ' {
            pos += 1;
            ch = if pos < len { chars[pos] } else { ch };
        }
        let mut left_justify = false;
        let mut width = 0usize;
        if ch == ',' {
            pos += 1;
            while pos < len && chars[pos] == ' ' {
                pos += 1;
            }
            if pos == len {
                return error("an unterminated alignment");
            }
            ch = chars[pos];
            if ch == '-' {
                left_justify = true;
                pos += 1;
                if pos == len {
                    return error("an unterminated alignment");
                }
                ch = chars[pos];
            }
            if !ch.is_ascii_digit() {
                return error("an alignment without digits");
            }
            loop {
                width = width * 10 + (ch as usize - '0' as usize);
                pos += 1;
                if pos == len {
                    return error("an unterminated alignment");
                }
                ch = chars[pos];
                if !(ch.is_ascii_digit() && width < LIMIT) {
                    break;
                }
            }
        }
        while pos < len && chars[pos] == ' ' {
            pos += 1;
            ch = if pos < len { chars[pos] } else { ch };
        }
        if ch == ':' {
            return error("a format string part (formatting the argument by its type) is not ported");
        }
        if ch != '}' {
            return error("an unexpected character in a format item");
        }
        pos += 1;
        let text = &args[index];
        let pad = width.saturating_sub(text.encode_utf16().count());
        if !left_justify {
            out.extend(std::iter::repeat_n(' ', pad));
        }
        out.push_str(text);
        if left_justify {
            out.extend(std::iter::repeat_n(' ', pad));
        }
    }
}

/// `CustomTextMesh.RECT_WIDTH_CHANGE_THRESHOLD`.
pub const RECT_WIDTH_CHANGE_THRESHOLD: f32 = 0.01;

/// `CustomTextMesh.CalculateScaleX`.
pub fn calculate_scale_x(text_width: f32, rect_width: f32, original_scale_x: f32) -> f32 {
    if text_width > rect_width {
        (rect_width / text_width) * original_scale_x
    } else {
        original_scale_x
    }
}

/// `CustomTextMesh.IsRectSizeChanged`.
pub fn is_rect_size_changed(current_rect_width: f32, last_rect_width: f32) -> bool {
    (current_rect_width - last_rect_width).abs() > RECT_WIDTH_CHANGE_THRESHOLD
}

/// The three width measurements `GetTextWidth` falls back through, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextWidthProbe {
    /// `preferredWidth` after a forced mesh update and layout rebuild.
    pub preferred_width: f32,
    /// `textBounds.extents.x`.
    pub bounds_extent_x: f32,
    /// `GetPreferredValues(text).x`.
    pub preferred_values_x: f32,
}

/// `CustomTextMesh.GetTextWidth` given its measurements (0 for empty text).
pub fn text_width(text_is_empty: bool, probe: TextWidthProbe) -> f32 {
    if text_is_empty {
        return 0.0;
    }
    // The engine keeps the preferred width unless it is zero or negative
    // (an unordered NaN keeps it too).
    let width = probe.preferred_width;
    if !(width <= 0.0) {
        return width;
    }
    let bounds_width = probe.bounds_extent_x + probe.bounds_extent_x;
    if bounds_width > 0.0 {
        return bounds_width;
    }
    if probe.preferred_values_x > 0.0 {
        probe.preferred_values_x
    } else {
        width
    }
}

/// Width scale fit state kept by the component.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthScaleFit {
    pub enabled: bool,
    pub original_scale_x: f32,
    pub original_scale_x_initialized: bool,
    pub last_rect_width: f32,
}

impl WidthScaleFit {
    /// The constructor's values (original X scale 1, not yet captured).
    pub fn new(enabled: bool) -> Self {
        Self { enabled, original_scale_x: 1.0, original_scale_x_initialized: false, last_rect_width: 0.0 }
    }

    /// `InitializeOriginalScaleX`.
    pub fn initialize_original_scale_x(&mut self, local_scale_x: f32) {
        if !self.original_scale_x_initialized {
            self.original_scale_x = local_scale_x;
            self.original_scale_x_initialized = true;
        }
    }

    /// `Start`'s part: capture the original X scale and, when enabled, the rect width.
    pub fn start(&mut self, local_scale_x: f32, rect_width: f32) {
        self.initialize_original_scale_x(local_scale_x);
        if self.enabled {
            self.last_rect_width = rect_width;
        }
    }

    /// `UpdateWidthScaleFit`: the new local X scale, or None when unchanged.
    /// `measure` runs `GetTextWidth` (it is only called when the engine calls it).
    pub fn update(
        &mut self,
        local_scale_x: f32,
        rect_width: f32,
        text_is_empty: bool,
        measure: impl FnOnce() -> f32,
    ) -> Option<f32> {
        self.initialize_original_scale_x(local_scale_x);
        if !self.enabled {
            return (local_scale_x != self.original_scale_x).then_some(self.original_scale_x);
        }
        if !(rect_width > 0.0) || text_is_empty {
            return None;
        }
        let text_width = measure();
        // Zero or negative widths return; an unordered NaN proceeds and
        // leaves the original scale (the engine's branch conditions).
        if text_width <= 0.0 {
            return None;
        }
        Some(calculate_scale_x(text_width, rect_width, self.original_scale_x))
    }

    /// `LateUpdate`'s width-fit part. Returns whether `UpdateWidthScaleFit`
    /// runs this frame; the caller then calls [`WidthScaleFit::update`].
    /// The text-changed flag is computed by the caller before any update.
    pub fn late_update(&mut self, text_changed: bool, rect_width: f32) -> bool {
        if !self.enabled {
            return text_changed;
        }
        let rect_changed = is_rect_size_changed(rect_width, self.last_rect_width);
        let runs = text_changed || rect_changed;
        if rect_changed {
            self.last_rect_width = rect_width;
        }
        runs
    }
}
