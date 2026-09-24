//! Text measurement and placement.
//!
//! With the region root's TMP font assets, each character is looked up as
//! TMP looks it up (the text's font asset, its run-time fallback list, the
//! TMP settings fallbacks, then the missing-glyph replacement) and laid out
//! with the stored glyph advance, the resolving asset's FaceInfo and its
//! glyph pair records, in TMP's own factorization: the element scale is the
//! point size over the asset's sampling point size times its face scale,
//! times the character and glyph scales; the pen step is (advance times the
//! horizontal FX scale plus the pair advance) times that scale, plus the
//! spacing terms in em units (the pass's point size over 100), times one
//! less the width adjustment; a whitespace character or U+200B then adds the
//! word spacing as a separate addition. The line gap and the base scale are
//! the text's own asset's. Placement follows the text rect's corners in its
//! own local space (origin at the pivot): the vertical anchor from the first
//! line's ascender and the last line's descender, each line justified in the
//! text area width. A character a dynamic
//! asset adds at run time from its source font file has no stored advance:
//! it is laid out with the open font's advance at that asset's sampling
//! point size and reported, once per text, as unsourced.
//!
//! Without font assets (the shared root) the open font supplies advances and
//! pairs, with the text's own FaceInfo for the vertical metrics. The open
//! font always supplies the bitmap shapes. Preferred and draw layout consume
//! the same metrics and retain their distinct break rules.

use bevy::math::Vec2;
use moly_assets::ui_layout::{UiComponent, UiTextFace};
use moly_law::text::auto_size::{self, AutoSize};
use moly_law::text::tags::{
    Indent, LineIndent, SizeSpec, TextSegment, parse_rich_segments, transformed_glyphs,
};
use super::tmp_font::{Element, Face, TmpFonts};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use swash::shape::ShapeContext;
use swash::text::Codepoint;

const FONT: &[u8] = include_bytes!("../../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");
pub(super) const FONT_METRICS_REVISION: &str = "ResourceHanRoundedSC-Medium/metrics-wrap-1-source-face-1-tmp-assets-1";
const EPSILON: f32 = 0.0001;
const LARGE: f32 = 32767.0;
/// The margin a preferred-width query lays the text out in: TMP's
/// k_LargePositiveVector2, int.MaxValue in each component, as a float.
const LARGE_POSITIVE_VECTOR: f32 = i32::MAX as f32;
const MAX_ITERATIONS: usize = auto_size::MAX_ITERATION_COUNT as usize;

pub(super) struct TextRules {
    pub(super) leading: HashSet<char>,
    pub(super) following: HashSet<char>,
    pub(super) modern_hangul: bool,
    /// The root's TMP font assets; None on the shared root, which carries none.
    pub(super) fonts: Option<Arc<TmpFonts>>,
}

pub(super) struct PlacedGlyph {
    pub(super) ch: char,
    /// Character index in the input string, not a byte offset or a wrapped string.
    pub(super) source_index: usize,
    pub(super) font_size: f32,
    pub(super) scale: f32,
    pub(super) width_scale: f32,
    /// Pen origin / baseline in the text rect's local space (origin at its
    /// pivot), positive Y up.
    pub(super) pen: Vec2,
    pub(super) color: Option<[f32; 4]>,
}

pub(super) struct TextLayout {
    pub(super) glyphs: Vec<PlacedGlyph>,
}

/// Reports each (text component, finding) once: layout runs every time a
/// text is measured or redrawn.
fn report_once(path_id: i64, finding: String) {
    static REPORTED: Mutex<Option<HashSet<(i64, String)>>> = Mutex::new(None);
    let mut reported = REPORTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if reported.get_or_insert_with(HashSet::new).insert((path_id, finding.clone())) {
        bevy::log::warn!("UI text @{path_id}: {finding}");
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Preferred,
    Render,
}

struct Settings {
    face: VerticalFace,
    /// The text's own font asset in the root's font document.
    primary: Option<usize>,
    path_id: i64,
    size: f32,
    /// `m_fontSizeBase`: where a render pass starts an auto-size search.
    base: f32,
    min: f32,
    max: f32,
    auto: bool,
    wrap: bool,
    kern: bool,
    rich: bool,
    controls: bool,
    char_spacing: f32,
    word_spacing: f32,
    line_spacing: f32,
    paragraph_spacing: f32,
    line_spacing_min: f32,
    width_adjustment_max: f32,
    /// `m_charWidthMaxAdj` as serialized (percent).
    char_width_max_adj: f32,
    margin: [f32; 4],
    horizontal: i64,
    vertical: i64,
    overflow: i64,
    color: [f32; 4],
}

#[derive(Clone, Copy)]
struct VerticalFace {
    /// Source FaceInfo units converted to one font-size point.
    unit_scale: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
    /// The text's own font asset's FaceInfo, when the root carries it: the
    /// base scale and line gap then take TMP's factorization.
    asset: Option<Face>,
}

impl VerticalFace {
    fn read(face: &UiTextFace) -> Result<Self, String> {
        if ![
            face.point_size, face.scale, face.line_height,
            face.ascent_line, face.descent_line,
        ].into_iter().all(f32::is_finite)
            || face.point_size <= 0.0 || face.scale <= 0.0
            || face.line_height <= 0.0 || face.ascent_line <= face.descent_line
        {
            return Err("TMP invalid source font FaceInfo".into());
        }
        let unit_scale = face.scale / face.point_size;
        Ok(Self {
            unit_scale,
            ascent: face.ascent_line * unit_scale,
            // TMP stores a signed descender; Line stores depth below baseline.
            descent: -face.descent_line * unit_scale,
            leading: (face.line_height - (face.ascent_line - face.descent_line))
                * unit_scale,
            asset: None,
        })
    }

    fn from_asset(face: Face) -> Result<Self, String> {
        let mut vertical = Self::read(&UiTextFace {
            point_size: face.point_size,
            scale: face.scale,
            line_height: face.line_height,
            ascent_line: face.ascent_line,
            descent_line: face.descent_line,
        })?;
        vertical.asset = Some(face);
        Ok(vertical)
    }

    /// TMP's baseScale: the text's point size over its asset's sampling
    /// point size, times the face scale (orthographic).
    fn base_scale(&self, size: f32) -> f32 {
        match self.asset {
            Some(face) => size / face.point_size * face.scale,
            None => size * self.unit_scale,
        }
    }

    /// The face's line gap plus the automatic spacing delta, in line space:
    /// TMP's (lineGap + lineSpacingDelta) * baseScale, lineGap = lineHeight -
    /// (ascentLine - descentLine) of the text's own asset.
    fn gap(&self, size: f32, spacing_delta: f32) -> f32 {
        match self.asset {
            Some(face) => {
                (face.line_height - (face.ascent_line - face.descent_line) + spacing_delta)
                    * self.base_scale(size)
            }
            None => self.leading * size + spacing_delta * self.base_scale(size),
        }
    }
}

fn number(fields: &Value, key: &str) -> Result<f32, String> {
    let n = fields[key]
        .as_f64()
        .ok_or_else(|| format!("TMP missing numeric {key}"))? as f32;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("TMP non-finite {key}"))
    }
}
fn boolean(fields: &Value, key: &str) -> Result<bool, String> {
    fields[key]
        .as_bool()
        .ok_or_else(|| format!("TMP missing boolean {key}"))
}
fn integer(fields: &Value, key: &str) -> Result<i64, String> {
    fields[key]
        .as_i64()
        .ok_or_else(|| format!("TMP missing integer {key}"))
}

impl Settings {
    fn read(component: &UiComponent, alignment: Option<i64>, fonts: Option<&TmpFonts>) -> Result<Self, String> {
        let f = &component.fields;
        let source_face = component.font.as_ref()
            .and_then(|font| font.source_face.as_ref())
            .ok_or_else(|| format!(
                "TMP source font FaceInfo missing for component @{}",
                component.path_id,
            ))?;
        let (face, primary) = match fonts {
            Some(fonts) => {
                let font = component.font.as_ref()
                    .ok_or_else(|| format!("TMP text @{} names no font asset", component.path_id))?;
                let (Some(file), Some(path_id)) = (font.serialized_file.as_deref(), font.path_id) else {
                    return Err(format!("TMP text @{} font reference has no serialized file or path id", component.path_id));
                };
                let primary = fonts.primary(file, path_id)?;
                let asset = fonts.asset(primary).face;
                // The layout document's copy of the text's FaceInfo and the
                // font document's must be the same asset's.
                if [asset.point_size, asset.scale, asset.line_height, asset.ascent_line, asset.descent_line]
                    != [source_face.point_size, source_face.scale, source_face.line_height,
                        source_face.ascent_line, source_face.descent_line]
                {
                    return Err(format!(
                        "TMP text @{}: its FaceInfo differs from {} in the font document",
                        component.path_id,
                        fonts.asset(primary).name
                    ));
                }
                (VerticalFace::from_asset(asset)?, Some(primary))
            }
            None => (VerticalFace::read(source_face)?, None),
        };
        if boolean(f, "m_isRightToLeft")? {
            return Err("TMP RTL placement is not implemented".into());
        }
        // The serialized `m_isOrthographic` is not read: the UI text
        // component's Awake sets it to true unconditionally and nothing sets
        // it again, so every UI text lays out orthographic (scale factor 1,
        // em unit size * 0.01) whatever the prefab stored.
        if integer(f, "m_fontStyle")? != 0 || integer(f, "m_fontWeight")? != 400 {
            return Err("TMP requested a font style not supplied by the current atlas".into());
        }
        // TMP TextAlignmentOptions combines the horizontal and vertical bits.
        // A runtime property assignment supersedes the serialized defaults.
        let (horizontal, vertical) = match alignment {
            Some(value) => (value & 0xff, value & !0xff),
            None => (
                integer(f, "m_HorizontalAlignment")?,
                integer(f, "m_VerticalAlignment")?,
            ),
        };
        if !matches!(horizontal, 1 | 2 | 4) || !matches!(vertical, 256 | 512 | 1024) {
            return Err(format!(
                "TMP alignment {horizontal}/{vertical} is not implemented"
            ));
        }
        let overflow = integer(f, "m_overflowMode")?;
        if !matches!(overflow, 0 | 2 | 3 | 4) {
            return Err(format!(
                "TMP overflow mode {overflow} requires a different renderer"
            ));
        }
        let margins = f["m_margin"]
            .as_array()
            .filter(|m| m.len() == 4)
            .ok_or("TMP requires its four serialized margins")?;
        let mut margin = [0.0; 4];
        for (i, v) in margins.iter().enumerate() {
            margin[i] = v.as_f64().ok_or("TMP margin is not numeric")? as f32;
            if !margin[i].is_finite() {
                return Err("TMP margin is not finite".into());
            }
        }
        let colors = f["m_fontColor"]
            .as_array()
            .filter(|v| v.len() == 4)
            .ok_or("TMP requires its four serialized color channels")?;
        let mut color = [0.0; 4];
        for (i, channel) in colors.iter().enumerate() {
            color[i] = channel.as_f64().ok_or("TMP color is not numeric")? as f32;
            if !color[i].is_finite() {
                return Err("TMP color is not finite".into());
            }
        }
        let result = Self {
            face,
            primary,
            path_id: component.path_id,
            size: number(f, "m_fontSize")?,
            base: number(f, "m_fontSizeBase")?,
            min: number(f, "m_fontSizeMin")?,
            max: number(f, "m_fontSizeMax")?,
            auto: boolean(f, "m_enableAutoSizing")?,
            wrap: boolean(f, "m_enableWordWrapping")?,
            kern: boolean(f, "m_enableKerning")?,
            rich: boolean(f, "m_isRichText")?,
            controls: boolean(f, "m_parseCtrlCharacters")?,
            char_spacing: number(f, "m_characterSpacing")?,
            word_spacing: number(f, "m_wordSpacing")?,
            line_spacing: number(f, "m_lineSpacing")?,
            paragraph_spacing: number(f, "m_paragraphSpacing")?,
            line_spacing_min: number(f, "m_lineSpacingMax")?,
            width_adjustment_max: number(f, "m_charWidthMaxAdj")? / 100.0,
            char_width_max_adj: number(f, "m_charWidthMaxAdj")?,
            margin,
            horizontal,
            vertical,
            overflow,
            color,
        };
        if result.size <= 0.0 || (result.auto && (result.min <= 0.0 || result.max < result.min)) {
            return Err("TMP invalid point-size bounds".into());
        }
        if !(0.0..1.0).contains(&result.width_adjustment_max) || result.line_spacing_min > 0.0 {
            return Err("TMP invalid automatic spacing bounds".into());
        }
        Ok(result)
    }
}

#[derive(Clone)]
struct Token {
    ch: char,
    source: usize,
    segment: usize,
    transform_scale: f32,
    fixed_space: Option<f32>,
}

struct Prepared {
    tokens: Vec<Token>,
    segments: Vec<TextSegment>,
}

fn prepare(input: &str, settings: &Settings) -> Result<Prepared, String> {
    let raw: Vec<char> = input.chars().collect();
    let mut text = String::new();
    let mut source = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if settings.controls && raw[i] == '\\' && i + 1 < raw.len() {
            let escaped = match raw[i + 1] {
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                '\\' => Some('\\'),
                _ => None,
            };
            if let Some(ch) = escaped {
                text.push(ch);
                source.push(i);
                i += 2;
                continue;
            }
        }
        text.push(raw[i]);
        source.push(i);
        i += 1;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut mapped = Vec::new();
    let mut no_parse = false;
    i = 0;
    while i < chars.len() {
        if settings.rich && chars[i] == '<' {
            if let Some(relative) = chars[i..].iter().position(|ch| *ch == '>') {
                let end = i + relative;
                let tag: String = chars[i + 1..end].iter().collect::<String>().to_lowercase();
                if !no_parse || tag == "/noparse" {
                    if tag == "noparse" {
                        no_parse = true;
                    } else if tag == "/noparse" {
                        no_parse = false;
                    } else if tag == "br" || tag == "cr" {
                        mapped.push(('\n', source[i]));
                    } else if tag == "nbsp" {
                        mapped.push(('\u{a0}', source[i]));
                    } else {
                        let name = tag
                            .trim_start_matches('/')
                            .split(['=', ' '])
                            .next()
                            .unwrap_or("");
                        if !matches!(
                            name,
                            "color"
                                | "size"
                                | "scale"
                                | "alpha"
                                | "voffset"
                                | "cspace"
                                | "line-height"
                                | "line-indent"
                                | "indent"
                                | "pos"
                                | "mspace"
                                | "align"
                                | "space"
                                | "uppercase"
                                | "allcaps"
                                | "lowercase"
                                | "smallcaps"
                        ) && !name.starts_with('#')
                        {
                            return Err(format!(
                                "TMP rich-text tag <{tag}> has no implemented placement"
                            ));
                        }
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        mapped.push((chars[i], source[i]));
        i += 1;
    }
    let segments = if settings.rich {
        parse_rich_segments(&text)
    } else {
        let mut template = parse_rich_segments(" ").remove(0);
        template.text = text;
        vec![template]
    };
    let mut tokens = Vec::new();
    let mut index = 0;
    for (segment, style) in segments.iter().enumerate() {
        if style.bold
            || style.italic
            || style.underline
            || style.strikethrough
            || style.superscript
            || style.subscript
            || style.rotate.is_some()
            || style.duospace
        {
            return Err(
                "TMP rich style needs glyph decoration or a font face not supplied here".into(),
            );
        }
        // These tags change the pen while the shared segment parser does not
        // retain the tag's event position. Reject instead of replaying it on
        // every wrapped line or silently inventing a different tag machine.
        if style.fixed_advance.is_some()
            || style.position.is_some()
            || style.monospace.is_some()
            || style.line_indent.is_some()
            || style.indent.is_some()
            || style.cspace.is_some()
        {
            return Err(
                "TMP pen-control tags need source-position events from the shared parser".into(),
            );
        }
        if let Some(space) = style.fixed_advance {
            tokens.push(Token {
                ch: '\0',
                source: mapped.get(index).map_or(input.chars().count(), |v| v.1),
                segment,
                transform_scale: 1.0,
                fixed_space: Some(space),
            });
        }
        for ch in style.text.chars() {
            let &(expected, origin) = mapped
                .get(index)
                .ok_or("TMP rich-text source map exhausted")?;
            if ch != expected {
                return Err("TMP rich-text source map differs from segment parser".into());
            }
            index += 1;
            for (rendered, scale) in transformed_glyphs(ch, style) {
                tokens.push(Token {
                    ch: rendered,
                    source: origin,
                    segment,
                    transform_scale: scale,
                    fixed_space: None,
                });
            }
        }
    }
    if index != mapped.len() {
        return Err("TMP segment parser left unmapped characters".into());
    }
    Ok(Prepared { tokens, segments })
}

fn font() -> swash::FontRef<'static> {
    static VALUE: OnceLock<swash::FontRef<'static>> = OnceLock::new();
    *VALUE.get_or_init(|| swash::FontRef::from_index(FONT, 0).expect("embedded UI font"))
}

#[derive(Clone, Copy, Default)]
struct PairMetric {
    advance: [f32; 2],
    offset: [Vec2; 2],
}
thread_local! {
    static SHAPER: RefCell<ShapeContext> = RefCell::new(ShapeContext::new());
    static PAIRS: RefCell<HashMap<(char, char), Result<PairMetric, String>>> = RefCell::new(HashMap::new());
}

fn pair_metric(left: char, right: char) -> Result<PairMetric, String> {
    let left = metric_character(left);
    let right = metric_character(right);
    PAIRS.with(|cache| {
        if let Some(value) = cache.borrow().get(&(left, right)) {
            return value.clone();
        }
        let result = SHAPER.with(|context| {
            let font = font();
            let expected = [font.charmap().map(left), font.charmap().map(right)];
            let mut context = context.borrow_mut();
            let settings: Vec<swash::Setting<u16>> = font
                .features()
                .map(|feature| swash::Setting {
                    tag: feature.tag(),
                    value: u16::from(feature.tag() == swash::tag_from_bytes(b"kern")),
                })
                .collect();
            let mut shaper = context
                .builder(font)
                .script(left.script())
                .size(1.0)
                .features(settings)
                .build();
            let text: String = [left, right].into_iter().collect();
            shaper.add_str(&text);
            let mut glyphs = Vec::new();
            shaper.shape_with(|cluster| glyphs.extend_from_slice(cluster.glyphs));
            if glyphs.len() != 2 || glyphs[0].id != expected[0] || glyphs[1].id != expected[1] {
                return Err(format!(
                    "UI pair U+{:04X}/U+{:04X} requires a shaped-glyph atlas",
                    left as u32, right as u32
                ));
            }
            let metrics = font.glyph_metrics(&[]).scale(1.0);
            Ok(PairMetric {
                advance: [
                    glyphs[0].advance - metrics.advance_width(expected[0]),
                    glyphs[1].advance - metrics.advance_width(expected[1]),
                ],
                offset: [
                    Vec2::new(glyphs[0].x, glyphs[0].y),
                    Vec2::new(glyphs[1].x, glyphs[1].y),
                ],
            })
        });
        cache.borrow_mut().insert((left, right), result.clone());
        result
    })
}

/// The open font's advance of one character in em units, as `measure`
/// reads it before scaling by the point size (0 for the characters it gives
/// no advance), for the research instrument that feeds the same advances to
/// a transcription of the source layout.
#[cfg(test)]
pub(super) fn open_font_advance(ch: char) -> Option<f32> {
    if zero_advance(ch) {
        return Some(0.0);
    }
    let font = font();
    let gid = font.charmap().map(metric_character(ch));
    (gid != 0).then(|| font.glyph_metrics(&[]).scale(1.0).advance_width(gid))
}

/// The kerning pair `measure` applies between two characters: the advance
/// added to each of them and the placement offsets, in em units.
#[cfg(test)]
pub(super) fn open_font_pair(left: char, right: char) -> Result<([f32; 2], [[f32; 2]; 2]), String> {
    pair_metric(left, right).map(|pair| (pair.advance, [pair.offset[0].to_array(), pair.offset[1].to_array()]))
}

fn metric_character(ch: char) -> char {
    // TMP's font-asset lookup uses the space / hyphen glyph for these missing
    // nominal characters. Their original codepoint still controls line breaks.
    if font().charmap().map(ch) != 0 {
        return ch;
    }
    match ch {
        '\u{a0}' => ' ',
        '\u{2011}' => '-',
        _ => ch,
    }
}

fn hard_break(ch: char) -> bool {
    matches!(ch, '\n' | '\u{b}' | '\u{2028}' | '\u{2029}')
}
fn zero_advance(ch: char) -> bool {
    hard_break(ch) || matches!(ch, '\r' | '\u{200b}' | '\u{2060}' | '\u{feff}' | '\0')
}
fn visible(ch: char) -> bool {
    !ch.is_whitespace() && !zero_advance(ch) && ch != '\u{ad}'
}
fn east_asian(ch: char, modern: bool) -> bool {
    let c = ch as u32;
    (!modern
        && ((0x1100 < c && c < 0x11ff) || (0xa960 < c && c < 0xa97f) || (0xac00 < c && c < 0xd7ff)))
        || (0x2e80 < c && c < 0x9fff)
        || (0xf900 < c && c < 0xfaff)
        || (0xfe30 < c && c < 0xfe4f)
        || (0xff00 < c && c < 0xffef)
}
fn break_space(ch: char) -> bool {
    (ch.is_whitespace() || matches!(ch, '\u{200b}' | '-' | '\u{ad}'))
        && !matches!(
            ch,
            '\u{a0}' | '\u{2007}' | '\u{2011}' | '\u{202f}' | '\u{2060}'
        )
}

struct Measured {
    size: f32,
    scale: f32,
    fit: f32,
    step: f32,
    /// The word spacing a whitespace character or U+200B adds to the pen as
    /// its own addition after `step` (0 for every other character).
    word: f32,
    pair_offset: Vec2,
    ascent: f32,
    descent: f32,
    voffset: f32,
    cspace: f32,
    /// Subtracted from the pen after this character for the line's width:
    /// the spacing terms of its step.
    spacing_offset: f32,
    /// The character drawn (TMP's missing-glyph path can replace it).
    drawn: char,
    /// TMP's per-character baseline offset from its asset's FaceInfo baseline.
    baseline: f32,
}

fn segment_size(spec: &Option<SizeSpec>, base: f32) -> f32 {
    match spec {
        None => base,
        Some(SizeSpec::Absolute(n)) => *n,
        Some(SizeSpec::Delta(n)) => base + n,
        Some(SizeSpec::Percent(n)) => base * n / 100.0,
        Some(SizeSpec::Em(n)) => base * n,
    }
}
fn indent(value: &Option<Indent>, size: f32, width: f32) -> f32 {
    match value {
        None => 0.0,
        Some(Indent::Pixels(n)) => *n,
        Some(Indent::Em(n)) => n * size,
        Some(Indent::Percent(n)) => width * n / 100.0,
    }
}

fn measure(
    prepared: &Prepared,
    config: &Settings,
    fonts: Option<&TmpFonts>,
    purpose: Purpose,
    size: f32,
    adjustment: f32,
) -> Result<Vec<Measured>, String> {
    match (fonts, config.primary) {
        (Some(fonts), Some(primary)) => measure_assets(prepared, config, fonts, primary, purpose, size, adjustment),
        (None, None) => measure_open_font(prepared, config, size, adjustment),
        _ => Err("TMP text settings and font assets disagree on the font source".into()),
    }
}

/// The open font's advance of one character in em units (0 for the
/// characters that advance nothing), for a character no stored table carries.
fn open_font_em(ch: char) -> Result<f32, String> {
    if zero_advance(ch) {
        return Ok(0.0);
    }
    let font = font();
    let gid = font.charmap().map(metric_character(ch));
    if gid == 0 {
        return Err(format!("UI font has no glyph U+{:04X}", ch as u32));
    }
    Ok(font.glyph_metrics(&[]).scale(1.0).advance_width(gid))
}

/// The per-character metrics of TMP 3.0.7 with the root's font assets:
/// `GenerateTextMesh` for a render pass, `CalculatePreferredValues` for a
/// preferred-size pass (whose element scale has no glyph scale factor).
///
/// Every character takes the pen step from its glyph's advance, the
/// synthesized zero-metric control characters included (they still take the
/// spacing terms); a whitespace character or U+200B then adds the word
/// spacing as a second, separate addition.
fn measure_assets(
    prepared: &Prepared,
    config: &Settings,
    fonts: &TmpFonts,
    primary: usize,
    purpose: Purpose,
    size: f32,
    adjustment: f32,
) -> Result<Vec<Measured>, String> {
    // TMP's character array holds every character, not the pen events of
    // tags: the pair lookup reads the neighbours in that array.
    let mut resolved = Vec::with_capacity(prepared.tokens.len());
    for token in &prepared.tokens {
        if token.fixed_space.is_some() {
            resolved.push(None);
            continue;
        }
        let (hit, replaced) = fonts.resolve(token.ch as u32, primary)?;
        if replaced {
            report_once(config.path_id, format!(
                "U+{:04X} is in no font asset of {}'s chain; TMP replaces it by U+{:04X}",
                token.ch as u32, fonts.asset(primary).name, hit.unicode,
            ));
        }
        resolved.push(Some(hit));
    }
    let characters: Vec<usize> = (0..prepared.tokens.len()).filter(|i| resolved[*i].is_some()).collect();
    let mut unsourced: Vec<(char, usize)> = Vec::new();
    let mut result = Vec::with_capacity(prepared.tokens.len());
    for (i, token) in prepared.tokens.iter().enumerate() {
        let style = &prepared.segments[token.segment];
        let point_size = segment_size(&style.size, size);
        // <scale> sets TMP's horizontal FX scale; <smallcaps> its small caps multiplier.
        let fx = style.scale.unwrap_or(1.0);
        let small_caps = token.transform_scale;
        if point_size <= 0.0 || !point_size.is_finite() || fx <= 0.0 || !fx.is_finite() {
            return Err("TMP invalid rich-text point size or scale".into());
        }
        if token.ch == '\t' {
            return Err("TMP tab stops require a font-asset tab width".into());
        }
        if token.ch == '\u{ad}' {
            return Err("TMP soft hyphen substitution is not implemented".into());
        }
        // currentEmScale of an orthographic text: set once per pass from the
        // pass's point size, not from a <size> segment's.
        let em = size * 0.01;
        let cspace = style.cspace.unwrap_or(0.0);
        let voffset = style.voffset.unwrap_or(0.0);
        let Some(hit) = resolved[i] else {
            // A <space> pen event: no character, the text's own face.
            result.push(Measured {
                size: point_size, scale: fx * small_caps, fit: 0.0, step: 0.0, word: 0.0, pair_offset: Vec2::ZERO,
                ascent: config.face.ascent * point_size, descent: config.face.descent * point_size,
                voffset, cspace, spacing_offset: 0.0, drawn: token.ch, baseline: 0.0,
            });
            continue;
        };
        let asset = fonts.asset(hit.asset);
        let face = asset.face;
        let (character_scale, glyph) = match hit.element {
            Element::Stored { character_scale, glyph } => (character_scale, Some(glyph)),
            // A dynamic asset's run-time character and glyph are created with scale 1.
            Element::Dynamic => (1.0, None),
        };
        // adjustedScale and currentElementScale; the font scale
        // multiplier is 1 without sub/superscript, which is refused above.
        // The render pass multiplies by the glyph scale too; the preferred
        // pass stops at the character (text element) scale.
        let adjusted = point_size * small_caps / face.point_size * face.scale;
        let element_scale = match purpose {
            Purpose::Render => adjusted * 1.0 * character_scale * glyph.map_or(1.0, |g| g.scale),
            Purpose::Preferred => adjusted * 1.0 * character_scale,
        };
        let advance = match glyph {
            Some(glyph) => glyph.horizontal_advance,
            None if zero_advance(token.ch) => 0.0,
            None => {
                unsourced.push((token.ch, hit.asset));
                // The open font at the dynamic asset's sampling point size.
                open_font_em(metric_character(token.ch))? * face.point_size
            }
        };
        // Kerning: the first record of (this, next) and the
        // second record of (previous, this), in this character's asset.
        let mut x_advance = 0.0;
        let mut placement = Vec2::ZERO;
        let mut character_spacing = config.char_spacing;
        if config.kern {
            let position = characters.iter().position(|&c| c == i).expect("character index");
            let neighbour = |k: Option<usize>| -> Result<Option<u32>, String> {
                let Some(k) = k else { return Ok(None) };
                match resolved[characters[k]].map(|r| r.element) {
                    Some(Element::Stored { glyph, .. }) => Ok(Some(glyph.index)),
                    _ if asset.has_pairs() => Err(format!(
                        "TMP pair lookup in {} needs a run-time glyph index", asset.name
                    )),
                    _ => Ok(None),
                }
            };
            let own_index = match glyph {
                Some(glyph) => Some(glyph.index),
                None if asset.has_pairs() => {
                    return Err(format!("TMP pair lookup in {} needs a run-time glyph index", asset.name));
                }
                None => None,
            };
            if let Some(own_index) = own_index {
                let next = neighbour((position + 1 < characters.len()).then_some(position + 1))?;
                if let Some(record) = next.and_then(|next| asset.pair(own_index, next)) {
                    x_advance = record.first.x_advance;
                    placement = Vec2::new(record.first.x_placement, record.first.y_placement);
                    if record.ignore_spacing_adjustments {
                        character_spacing = 0.0;
                    }
                }
                let previous = neighbour(position.checked_sub(1))?;
                if let Some(record) = previous.and_then(|previous| asset.pair(previous, own_index)) {
                    x_advance += record.second.x_advance;
                    placement += Vec2::new(record.second.x_placement, record.second.y_placement);
                    if record.ignore_spacing_adjustments {
                        character_spacing = 0.0;
                    }
                }
            }
        }
        // The spacing terms of the step and of the line's max advance
        // (xAdvance and maxAdvanceOffset); bold spacing is 0 for regular text.
        let spacing = (asset.normal_spacing_offset + character_spacing + 0.0) * em;
        // The preferred pass's step has no horizontal FX factor on the advance.
        let fx_step = match purpose {
            Purpose::Render => fx,
            Purpose::Preferred => 1.0,
        };
        let step = ((advance * fx_step + x_advance) * element_scale + spacing + cspace) * (1.0 - adjustment);
        let word = if token.ch.is_whitespace() || token.ch == '\u{200b}' {
            config.word_spacing * em
        } else {
            0.0
        };
        let fit = advance * (1.0 - adjustment) * element_scale;
        // U+FEFF and U+0000 keep the zero advance they had: U+0000 ends the
        // source's processing array and never reaches here, and U+FEFF is
        // not drawn by this port (a named gap, not a TMP rule).
        let (step, word, fit) = if matches!(token.ch, '\u{feff}' | '\0') { (0.0, 0.0, 0.0) } else { (step, word, fit) };
        result.push(Measured {
            size: point_size,
            scale: fx * small_caps,
            fit,
            step,
            word,
            pair_offset: placement * element_scale,
            // Element ascender and descender in line space;
            // descent is stored as a depth below the baseline.
            ascent: face.ascent_line * element_scale / small_caps,
            descent: -(face.descent_line * element_scale / small_caps),
            voffset,
            cspace,
            spacing_offset: (spacing - cspace) * (1.0 - adjustment),
            // The bitmap is the open font's: its own substitutions apply.
            drawn: metric_character(char::from_u32(hit.unicode).ok_or("TMP drew a non-scalar code point")?),
            // baselineOffset
            baseline: face.baseline * adjusted * 1.0 * face.scale,
        });
    }
    if !unsourced.is_empty() {
        let mut by_asset: Vec<(usize, String)> = Vec::new();
        for (ch, asset) in unsourced {
            match by_asset.iter_mut().find(|(a, _)| *a == asset) {
                Some((_, text)) => text.push(ch),
                None => by_asset.push((asset, ch.to_string())),
            }
        }
        for (asset, text) in by_asset {
            report_once(config.path_id, format!(
                "{} characters {text:?} are added at run time by the dynamic font asset {} from its source \
                 font file; their advances are not stored, so they are laid out with the open font's advance \
                 at the asset's sampling point size (unsourced)",
                text.chars().count(), fonts.asset(asset).name,
            ));
        }
    }
    Ok(result)
}

fn measure_open_font(
    prepared: &Prepared,
    config: &Settings,
    size: f32,
    adjustment: f32,
) -> Result<Vec<Measured>, String> {
    let font = font();
    let glyph_metrics = font.glyph_metrics(&[]).scale(1.0);
    let mut result = Vec::with_capacity(prepared.tokens.len());
    for (i, token) in prepared.tokens.iter().enumerate() {
        let style = &prepared.segments[token.segment];
        let point_size = segment_size(&style.size, size);
        let scale = token.transform_scale * style.scale.unwrap_or(1.0);
        if point_size <= 0.0 || !point_size.is_finite() || scale <= 0.0 || !scale.is_finite() {
            return Err("TMP invalid rich-text point size or scale".into());
        }
        if token.ch == '\t' {
            return Err("TMP tab stops require a font-asset tab width".into());
        }
        if token.ch == '\u{ad}' {
            return Err("TMP soft hyphen substitution is not implemented".into());
        }
        let mut glyph_advance = 0.0;
        if !zero_advance(token.ch) {
            let gid = font.charmap().map(metric_character(token.ch));
            if gid == 0 {
                return Err(format!("UI font has no glyph U+{:04X}", token.ch as u32));
            }
            glyph_advance = glyph_metrics.advance_width(gid);
        }
        let mut pair_advance = 0.0;
        let mut pair_offset = Vec2::ZERO;
        if config.kern && !zero_advance(token.ch) {
            if let Some(next) = prepared
                .tokens
                .get(i + 1)
                .filter(|next| !zero_advance(next.ch))
            {
                let pair = pair_metric(token.ch, next.ch)?;
                pair_advance += pair.advance[0];
                pair_offset += pair.offset[0];
            }
            if i > 0 && !zero_advance(prepared.tokens[i - 1].ch) {
                let pair = pair_metric(prepared.tokens[i - 1].ch, token.ch)?;
                pair_advance += pair.advance[1];
                pair_offset += pair.offset[1];
            }
        }
        // The pass's em unit, as with the font assets.
        let em = size * 0.01;
        let cspace = style.cspace.unwrap_or(0.0);
        let fit = glyph_advance * point_size * scale * (1.0 - adjustment);
        let step = ((glyph_advance + pair_advance) * point_size * scale
            + config.char_spacing * em
            + cspace)
            * (1.0 - adjustment);
        let word = if token.ch.is_whitespace() || token.ch == '\u{200b}' {
            config.word_spacing * em
        } else {
            0.0
        };
        let (step, word) = if matches!(token.ch, '\u{feff}' | '\0') { (0.0, 0.0) } else { (step, word) };
        result.push(Measured {
            size: point_size,
            scale,
            fit,
            step,
            word,
            pair_offset: pair_offset * point_size * scale,
            ascent: config.face.ascent * point_size * scale,
            descent: config.face.descent * point_size * scale,
            voffset: style.voffset.unwrap_or(0.0),
            cspace,
            spacing_offset: (config.char_spacing * em - cspace) * (1.0 - adjustment),
            drawn: metric_character(token.ch),
            baseline: 0.0,
        });
    }
    Ok(result)
}

#[derive(Clone)]
struct Positioned {
    index: usize,
    pen: f32,
}
struct Line {
    positions: Vec<Positioned>,
    width: f32,
    preferred_width: f32,
    ascent: f32,
    descent: f32,
    offset: f32,
    hard: bool,
    start: usize,
}
struct Pass {
    lines: Vec<Line>,
    measured: Vec<Measured>,
    content: Vec2,
    /// `m_maxTextAscender`: the first line's maximum ascender.
    ascent: f32,
    /// `maxVisibleDescender`: the last laid-out line's descender (signed,
    /// line offset included).
    descender: f32,
    resize: Option<Resize>,
}
#[derive(Clone, Copy)]
enum Resize {
    Width { actual: f32, available: f32 },
    Height { actual: f32, lines: usize },
}

fn make_line(
    positions: Vec<Positioned>,
    start: usize,
    hard: bool,
    p: &Prepared,
    m: &[Measured],
) -> Line {
    // m_maxLineAscender / m_maxLineDescender start at -32767 / 32767 (the
    // descender is kept here as a depth below the baseline, so both start
    // at -32767). Only the line's first character and non-whitespace
    // characters raise them; with a baseline offset (<voffset>) a character
    // offers the larger of its shifted and unshifted extents, the unshifted
    // one computed back from the shifted value.
    let mut ascent: f32 = -LARGE;
    let mut descent: f32 = -LARGE;
    let mut width: f32 = 0.0;
    let mut preferred_width: f32 = 0.0;
    let mut before_carriage_return: f32 = 0.0;
    for (k, pos) in positions.iter().enumerate() {
        let token = &p.tokens[pos.index];
        let metric = &m[pos.index];
        if k == 0 || !token.ch.is_whitespace() {
            let element_ascender = metric.ascent + metric.voffset;
            let element_descender = -metric.descent + metric.voffset;
            let (adjusted_ascender, adjusted_descender) = if metric.voffset != 0.0 {
                (
                    ((element_ascender - metric.voffset) / 1.0).max(element_ascender),
                    ((element_descender - metric.voffset) / 1.0).min(element_descender),
                )
            } else {
                (element_ascender, element_descender)
            };
            ascent = adjusted_ascender.max(ascent);
            descent = (-adjusted_descender).max(descent);
        }
        if token.ch == '\r' {
            before_carriage_return = before_carriage_return.max(pos.pen);
        }
        if visible(token.ch) || (positions.len() == 1 && token.ch.is_whitespace()) {
            // maxAdvance subtracts (character spacing - rich cSpacing) from
            // the character's stored xAdvance (after the word spacing).
            // The minus on cSpacing is authored behaviour, not a typo: its
            // closing-tag event also adjusts xAdvance before this expression.
            width = pos.pen + metric.step + metric.word - metric.spacing_offset;
        }
        // Preferred keeps the last tested visible-character width, while a
        // carriage return saves the previous pen extent before zeroing it.
        // Trailing whitespace does not overwrite this width with penAfter.
        if visible(token.ch) {
            preferred_width = pos.pen.abs() + metric.fit;
        }
    }
    Line {
        positions,
        width,
        preferred_width: preferred_width.max(before_carriage_return),
        ascent,
        descent,
        offset: 0.0,
        hard,
        start,
    }
}

fn pass(
    p: &Prepared,
    cfg: &Settings,
    rules: &TextRules,
    purpose: Purpose,
    size: f32,
    adjustment: f32,
    spacing_delta: f32,
    width: f32,
    // Percentage units (<indent>, <line-indent>, <pos>) are fractions of
    // the component's own margin width, not of the width laid out in.
    percent_base: f32,
    height: f32,
    wrapping: bool,
    resize_allowed: bool,
    // The preferred pass's own character array (`m_internalCharacterInfo`):
    // the pass writes each character into it as it processes it, and its
    // "next character is a following character" test reads the next slot
    // from it, which holds what an earlier pass of this component (or this
    // pass before a line wrap went back) wrote there, or U+0000 in a slot
    // nothing wrote since the array was allocated. None for a render pass,
    // which tests the parsed text.
    mut internal: Option<&mut Vec<char>>,
) -> Result<Pass, String> {
    let measured = measure(p, cfg, rules.fonts.as_deref(), purpose, size, adjustment)?;
    let total = p.tokens.len();
    if let Some(array) = internal.as_deref_mut() {
        // TMP reallocates the array (zeroed) only when it is missing or
        // shorter than the text: to the next power of two, or the count plus
        // 256 above 1024 characters.
        if array.len() < total {
            let length = if total > 1024 { total + 256 } else { total.next_power_of_two() };
            *array = vec!['\0'; length];
        }
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut previous_hard = true;
    let mut resize = None;
    let mut truncated = false;
    let mut last_soft_used = None;
    while start < p.tokens.len() && !truncated {
        let first_style = &p.segments[p.tokens[start].segment];
        let mut pen = indent(&first_style.indent, size, percent_base);
        if previous_hard {
            pen += match &first_style.line_indent {
                None => 0.0,
                Some(LineIndent::Pixels(n)) => *n,
                Some(LineIndent::Percent(n)) => n * percent_base / 100.0,
            };
        }
        let mut positions = Vec::new();
        let mut saved: Option<(usize, usize)> = None;
        let mut soft: Option<(usize, usize)> = None;
        let mut first_word = true;
        let mut last_cjk = false;
        let mut previous_position: Option<Indent> = None;
        let mut next_start = p.tokens.len();
        let mut hard = false;
        for i in start..p.tokens.len() {
            let token = &p.tokens[i];
            let m = &measured[i];
            let style = &p.segments[token.segment];
            if let Some(array) = internal.as_deref_mut() {
                array[i] = token.ch;
            }
            if style.position != previous_position {
                if style.position.is_some() {
                    pen = indent(&style.position, m.size, percent_base);
                }
                previous_position = style.position.clone();
            }
            if let Some(fixed) = token.fixed_space {
                pen += fixed;
                positions.push(Positioned { index: i, pen });
                continue;
            }
            let mut origin = pen;
            let mono = style
                .monospace
                .as_ref()
                .map(|_| indent(&style.monospace, m.size, percent_base));
            if let Some(mono) = mono {
                origin += (mono - m.fit) * 0.5;
            }
            let fit = origin.abs() + m.fit;
            if visible(token.ch) && fit > width + EPSILON {
                let can_wrap = wrapping && i != start;
                if resize_allowed
                    && ((can_wrap && first_word) || (!can_wrap && purpose == Purpose::Render))
                {
                    // The reduction's base is the text area width the
                    // overflow test compares against: the margin width plus
                    // 0.0001 (less the zero tag margins).
                    resize = Some(Resize::Width {
                        actual: fit,
                        available: width + EPSILON,
                    });
                    break;
                }
                if can_wrap {
                    let take_soft = purpose == Purpose::Render
                        && first_word
                        && soft.is_some()
                        && soft.map(|v| v.0) != last_soft_used;
                    if let Some((end, count)) = if take_soft { soft } else { saved } {
                        positions.truncate(count);
                        next_start = end;
                        if take_soft {
                            last_soft_used = Some(end);
                        }
                        break;
                    }
                }
                if purpose == Purpose::Render && cfg.overflow == 3 {
                    truncated = true;
                    break;
                }
            }
            positions.push(Positioned {
                index: i,
                pen: origin,
            });
            pen += mono.map_or(m.step, |mono| {
                mono * (1.0 - adjustment) + cfg.char_spacing * size * 0.01 + m.cspace
            });
            // The word spacing is its own addition to the pen, after the step.
            pen += m.word;
            if hard_break(token.ch) {
                next_start = i + 1;
                hard = true;
                break;
            }
            // A carriage return resets the pen and, being whitespace, then
            // saves a word-break state like any breaking space.
            if token.ch == '\r' {
                pen = indent(&style.indent, m.size, percent_base);
            }
            if break_space(token.ch) {
                saved = Some((i + 1, positions.len()));
                soft = None;
                first_word = false;
                last_cjk = false;
            } else if east_asian(token.ch, rules.modern_hangul) {
                let leading = rules.leading.contains(&token.ch);
                let following = i + 1 < total
                    && match internal.as_deref() {
                        Some(array) => rules.following.contains(&array[i + 1]),
                        None => rules.following.contains(&p.tokens[i + 1].ch),
                    };
                let permitted = !leading || (purpose == Purpose::Preferred && first_word);
                if permitted {
                    if !following {
                        saved = Some((i + 1, positions.len()));
                        first_word = false;
                    }
                    if first_word {
                        if token.ch.is_whitespace() {
                            soft = Some((i + 1, positions.len()));
                        }
                        saved = Some((i + 1, positions.len()));
                    }
                } else if first_word && i == start {
                    saved = Some((i + 1, positions.len()));
                }
                last_cjk = true;
            } else if purpose == Purpose::Preferred && last_cjk {
                if !rules.leading.contains(&token.ch) {
                    saved = Some((i + 1, positions.len()));
                }
                last_cjk = false;
            } else if first_word {
                if token.ch.is_whitespace() {
                    soft = Some((i + 1, positions.len()));
                }
                saved = Some((i + 1, positions.len()));
                last_cjk = false;
            }
        }
        if resize.is_some() {
            break;
        }
        lines.push(make_line(
            positions, start, hard, p, &measured,
        ));
        if next_start <= start {
            return Err("TMP word-wrap failed to advance its source cursor".into());
        }
        start = next_start;
        previous_hard = hard;
    }
    let mut content = Vec2::ZERO;
    let mut ascent: f32 = 0.0;
    // m_ElementDescender at each line end: that line's descender (the
    // text's height is the first line's ascender less the current line's
    // descender, not a running minimum).
    let mut lowest: f32 = 0.0;
    for i in 0..lines.len() {
        if i == 0 {
            ascent = lines[i].ascent;
        } else {
            let previous = &lines[i - 1];
            let style = &p.segments[p.tokens[lines[i].start].segment];
            let paragraph =
                if previous.hard && matches!(p.tokens[lines[i].start - 1].ch, '\n' | '\u{2029}') {
                    cfg.paragraph_spacing
                } else {
                    0.0
                };
            let gap = style.line_height.unwrap_or(
                previous.descent + lines[i].ascent + cfg.face.gap(size, spacing_delta),
            );
            lines[i].offset = if cfg.face.asset.is_some() {
                // lineOffset += ... + (lineSpacing + paragraphSpacing) * currentEmScale
                previous.offset + (gap + (cfg.line_spacing + paragraph) * (size * 0.01))
            } else {
                previous.offset + gap + (cfg.line_spacing + paragraph) * size * 0.01
            };
        }
        lowest = -lines[i].descent - lines[i].offset;
        content.x = content.x.max(if purpose == Purpose::Preferred {
            lines[i].preferred_width
        } else {
            lines[i].width
        });
        let extent = ascent - lowest;
        if purpose == Purpose::Render && extent > height + EPSILON {
            if resize_allowed {
                resize = Some(Resize::Height {
                    actual: extent,
                    lines: i,
                });
                break;
            }
            if cfg.overflow == 3 {
                lines.truncate(i);
                break;
            }
        }
        content.y = extent;
    }
    Ok(Pass {
        lines,
        measured,
        content,
        ascent,
        descender: lowest,
        resize,
    })
}

/// `(int)(value * 100 + 1f) / 100f` as the game compiles it: the float to
/// int conversion saturates, gives 0 for NaN, and gives int.MinValue for
/// +infinity (the compiled code tests that one value).
fn preferred_round(value: f32) -> f32 {
    let scaled = value * 100.0 + 1.0;
    if scaled == f32::INFINITY {
        return i32::MIN as f32 / 100.0;
    }
    (scaled as i32) as f32 / 100.0
}

fn calculate(
    p: &Prepared,
    cfg: &Settings,
    rules: &TextRules,
    purpose: Purpose,
    rect: Vec2,
    width_only: bool,
    mut internal: Option<&mut Vec<char>>,
) -> Result<(Pass, f32, f32), String> {
    // Render passes start the search at the base size clamped to the bounds;
    // preferred-size queries start at the max size.
    let start = match purpose {
        Purpose::Render => auto_size::render_start_size(cfg.auto, cfg.size, cfg.base, cfg.min, cfg.max),
        Purpose::Preferred => auto_size::preferred_start_size(cfg.auto, cfg.size, cfg.max),
    };
    let mut search = AutoSize::new(start, cfg.min, cfg.max);
    let inner_width = rect.x - cfg.margin[0] - cfg.margin[2];
    // A render pass clamps the margin width and height at 0 (`m_marginWidth
    // > 0 ? m_marginWidth : 0`); the preferred-height query passes the
    // margin width unclamped, 32767 when it is exactly 0.
    let width = if width_only {
        LARGE_POSITIVE_VECTOR
    } else if purpose == Purpose::Preferred {
        if inner_width != 0.0 { inner_width } else { LARGE }
    } else if inner_width > 0.0 {
        inner_width
    } else {
        0.0
    };
    let height = if purpose == Purpose::Preferred {
        LARGE
    } else {
        let inner_height = rect.y - cfg.margin[1] - cfg.margin[3];
        if inner_height > 0.0 { inner_height } else { 0.0 }
    };
    // Justified and flush alignments are rejected when the settings are read,
    // so the width adjustment never takes the justified target.
    let justified_or_flush = false;
    for iteration in 0..=MAX_ITERATIONS {
        // The iteration counter the resize tests read. The render loop adds
        // one after each generation pass. The preferred-height loop also adds
        // one after each pass, and each preferred pass adds one more at its
        // start, so its k-th pass (from 0) tests 2k + 1 against the limit.
        search.iteration = match purpose {
            Purpose::Render => iteration as u32,
            Purpose::Preferred => 2 * iteration as u32 + 1,
        };
        let can_resize = cfg.auto && !width_only && search.below_iteration_limit();
        let size = search.font_size;
        // The source continues the same pass when a resize it asked for is
        // not allowed; here that pass is run again without resizing, so the
        // character array goes back to its state before the attempt.
        let before_attempt = internal.as_deref().cloned();
        let attempt = pass(
            p,
            cfg,
            rules,
            purpose,
            size,
            search.char_width_adj_delta,
            search.line_spacing_delta,
            width,
            inner_width,
            height,
            !width_only && cfg.wrap,
            can_resize
                && (search.may_reduce_point_size(cfg.min)
                    || search.may_reduce_char_width(cfg.char_width_max_adj)
                    || search.line_spacing_delta > cfg.line_spacing_min),
            internal.as_deref_mut(),
        )?;
        match attempt.resize {
            Some(Resize::Width { actual, available })
                if can_resize && search.may_reduce_char_width(cfg.char_width_max_adj) =>
            {
                search.reduce_char_width(actual, available, justified_or_flush, cfg.char_width_max_adj);
                continue;
            }
            Some(Resize::Height { actual, lines })
                if can_resize && search.may_reduce_line_spacing(cfg.line_spacing_min, lines > 0) =>
            {
                search.reduce_line_spacing(
                    height,
                    actual,
                    lines as u32,
                    cfg.face.base_scale(size),
                    cfg.line_spacing_min,
                );
                continue;
            }
            Some(_) if can_resize && search.may_reduce_point_size(cfg.min) => {
                search.reduce_point_size(cfg.min);
                continue;
            }
            Some(_) => {
                if let (Some(array), Some(saved)) = (internal.as_deref_mut(), before_attempt) {
                    *array = saved;
                }
                let final_pass = pass(
                    p,
                    cfg,
                    rules,
                    purpose,
                    size,
                    search.char_width_adj_delta,
                    search.line_spacing_delta,
                    width,
                    inner_width,
                    height,
                    !width_only && cfg.wrap,
                    false,
                    internal.as_deref_mut(),
                )?;
                return Ok((final_pass, size, search.char_width_adj_delta));
            }
            None => {}
        }
        if !width_only && search.may_increase_point_size(cfg.auto, cfg.max) {
            search.increase_point_size(cfg.max, cfg.char_width_max_adj);
            continue;
        }
        return Ok((attempt, size, search.char_width_adj_delta));
    }
    Err("TMP automatic point size exceeded its source iteration bound".into())
}

/// TMP's early exit: with an empty text-processing array, or one whose first
/// code point is U+0000, mesh generation clears the mesh and the preferred
/// size is zero, before any font scale or layout setting is read.
fn no_text(text: &str) -> bool {
    text.chars().next().is_none_or(|first| first == '\0')
}

/// The preferred width (axis 0) or height (axis 1) of a text component.
///
/// `internal` is the component's own character array, which its preferred
/// passes read and write (see [`pass`]); the caller keeps one per component
/// for as long as the component lives, starting empty.
pub(super) fn preferred_axis(
    text: &str,
    component: &UiComponent,
    axis: usize,
    rect_size: Vec2,
    rules: &TextRules,
    internal: &mut Vec<char>,
) -> Result<f32, String> {
    if axis > 1 || !rect_size.is_finite() {
        return Err("TMP invalid layout axis or rect".into());
    }
    if no_text(text) {
        return Ok(0.0);
    }
    // Alignment changes only the placement inside the rect, not preferred size.
    let config = Settings::read(component, None, rules.fonts.as_deref())?;
    let prepared = prepare(text, &config)?;
    // A text of tags only still runs the pass (no characters, size 0) and
    // then the margins and the rounding below: 0.01 with zero margins.
    let (result, _, _) = calculate(
        &prepared,
        &config,
        rules,
        Purpose::Preferred,
        rect_size,
        axis == 0,
        Some(internal),
    )?;
    // Each positive margin is added on its own, leading side first.
    let (leading, trailing) = if axis == 0 {
        (config.margin[0], config.margin[2])
    } else {
        (config.margin[1], config.margin[3])
    };
    let positive = |margin: f32| if margin > 0.0 { margin } else { 0.0 };
    Ok(preferred_round(result.content[axis] + positive(leading) + positive(trailing)))
}

pub(super) fn layout(
    text: &str,
    component: &UiComponent,
    rect_size: Vec2,
    pivot: Vec2,
    rules: &TextRules,
    alignment: Option<i64>,
) -> Result<TextLayout, String> {
    if !rect_size.is_finite() || !pivot.is_finite() {
        return Err("TMP non-finite render rect".into());
    }
    if no_text(text) {
        return Ok(TextLayout { glyphs: Vec::new() });
    }
    let config = Settings::read(component, alignment, rules.fonts.as_deref())?;
    let prepared = prepare(text, &config)?;
    if prepared.tokens.is_empty() {
        return Ok(TextLayout { glyphs: Vec::new() });
    }
    let (result, _, adjustment) =
        calculate(&prepared, &config, rules, Purpose::Render, rect_size, false, None)?;
    let margin = config.margin;
    // The rect's local corners (bottom-left, top-left), origin at the pivot.
    let x_min = -(pivot.x * rect_size.x);
    let y_min = -(pivot.y * rect_size.y);
    let y_max = y_min + rect_size.y;
    let (ascender, descender) = (result.ascent, result.descender);
    // The vertical anchor offset, from the corners, the margins, the first
    // line's ascender and the last line's descender.
    let anchor = match config.vertical {
        256 => Vec2::new(x_min + (0.0 + margin[0]), y_max + ((0.0 - ascender) - margin[1])),
        512 => Vec2::new(
            (x_min + x_min) * 0.5 + (0.0 + margin[0]),
            (y_min + y_max) * 0.5 + ((((margin[1] + ascender) + descender) - margin[3]) * -0.5 + 0.0),
        ),
        1024 => Vec2::new(x_min + (0.0 + margin[0]), y_min + ((0.0 - descender) + margin[3])),
        _ => unreachable!(),
    };
    // Each line records the text area width, which TMP computes as the
    // margin width (clamped at 0) plus 0.0001 less the `<margin>` tag
    // margins; that tag has no placement here (it is refused above), so
    // those margins are 0, and so is the line's left margin in the
    // justification offsets below.
    let inner_width = rect_size.x - margin[0] - margin[2];
    let margin_width = if inner_width > 0.0 { inner_width } else { 0.0 };
    let line_width = margin_width + EPSILON;
    let mut glyphs = Vec::new();
    for line in &result.lines {
        let style = &prepared.segments[prepared.tokens[line.start].segment];
        let horizontal = match style.align.as_deref() {
            Some("left") => 1,
            Some("center") => 2,
            Some("right") => 4,
            _ => config.horizontal,
        };
        // Left: margin left; Center: margin left + width / 2 - max advance / 2;
        // Right: margin left + width - max advance.
        let justification = match horizontal {
            1 => 0.0 + 0.0,
            2 => (0.0 + line_width * 0.5) - line.width * 0.5,
            4 => (0.0 + line_width) - line.width,
            _ => unreachable!(),
        };
        let offset = Vec2::new(anchor.x + justification, anchor.y + 0.0);
        for placed in &line.positions {
            let token = &prepared.tokens[placed.index];
            if !visible(token.ch) || token.fixed_space.is_some() {
                continue;
            }
            let m = &result.measured[placed.index];
            let style = &prepared.segments[token.segment];
            let color = if style.color.is_some() || style.alpha.is_some() {
                let rgb =
                    style
                        .color
                        .unwrap_or([config.color[0], config.color[1], config.color[2]]);
                Some([
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    style.alpha.unwrap_or(config.color[3]),
                ])
            } else {
                None
            };
            glyphs.push(PlacedGlyph {
                ch: m.drawn,
                source_index: token.source,
                font_size: m.size,
                scale: m.scale,
                width_scale: 1.0 - adjustment,
                // origin += offset.x; baseLine (element baseline offset less
                // the line offset, plus the <voffset> offset) += offset.y.
                // The pair placement is this port's pen shift for the glyph
                // bitmap (0 without pair records).
                pen: Vec2::new(
                    (placed.pen + offset.x) + m.pair_offset.x * (1.0 - adjustment),
                    (((m.baseline - line.offset) + m.voffset) + offset.y) + m.pair_offset.y,
                ),
                color,
            });
        }
    }
    Ok(TextLayout { glyphs })
}
