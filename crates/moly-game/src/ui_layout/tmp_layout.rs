//! Text measurement and placement with source TMP vertical face metrics.
//!
//! The existing open font supplies glyph advances, pairs and bitmap shapes.
//! Authored line/paragraph spacing combines with the assigned TMP font's actual
//! FaceInfo, not the substitute TTF's different line height. Preferred and draw
//! layout consume the same source metadata and retain their distinct break rules.

use bevy::math::Vec2;
use moly_assets::ui_layout::{UiComponent, UiTextFace};
use moly_law::text::tags::{
    Indent, LineIndent, SizeSpec, TextSegment, parse_rich_segments, transformed_glyphs,
};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use swash::shape::ShapeContext;
use swash::text::Codepoint;

const FONT: &[u8] = include_bytes!("../../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");
pub(super) const FONT_METRICS_REVISION: &str = "ResourceHanRoundedSC-Medium/metrics-wrap-1-source-face-1";
const EPSILON: f32 = 0.0001;
const LARGE: f32 = 32767.0;
const MAX_ITERATIONS: usize = 100;

pub(super) struct TextRules {
    pub(super) leading: HashSet<char>,
    pub(super) following: HashSet<char>,
    pub(super) modern_hangul: bool,
}

pub(super) struct PlacedGlyph {
    pub(super) ch: char,
    /// Character index in the input string, not a byte offset or a wrapped string.
    pub(super) source_index: usize,
    pub(super) font_size: f32,
    pub(super) scale: f32,
    pub(super) width_scale: f32,
    /// Pen origin / baseline, relative to the text rect centre, positive Y up.
    pub(super) pen: Vec2,
    pub(super) color: Option<[f32; 4]>,
}

pub(super) struct TextLayout {
    pub(super) glyphs: Vec<PlacedGlyph>,
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Preferred,
    Render,
}

struct Settings {
    face: VerticalFace,
    size: f32,
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
        })
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
    fn read(component: &UiComponent, alignment: Option<i64>) -> Result<Self, String> {
        let f = &component.fields;
        let source_face = component.font.as_ref()
            .and_then(|font| font.source_face.as_ref())
            .ok_or_else(|| format!(
                "TMP source font FaceInfo missing for component @{}",
                component.path_id,
            ))?;
        if boolean(f, "m_isRightToLeft")? {
            return Err("TMP RTL placement is not implemented".into());
        }
        if !boolean(f, "m_isOrthographic")? {
            return Err("TMP perspective text is not a UI rect".into());
        }
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
            face: VerticalFace::read(source_face)?,
            size: number(f, "m_fontSize")?,
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
    pair_offset: Vec2,
    ascent: f32,
    descent: f32,
    voffset: f32,
    cspace: f32,
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
        let em = point_size * 0.01;
        let cspace = style.cspace.unwrap_or(0.0);
        let fit = glyph_advance * point_size * scale * (1.0 - adjustment);
        let mut step = ((glyph_advance + pair_advance) * point_size * scale
            + config.char_spacing * em
            + cspace)
            * (1.0 - adjustment);
        if token.ch.is_whitespace() || token.ch == '\u{200b}' {
            step += config.word_spacing * em;
        }
        if zero_advance(token.ch) {
            step = 0.0;
        }
        result.push(Measured {
            size: point_size,
            scale,
            fit,
            step,
            pair_offset: pair_offset * point_size * scale,
            ascent: config.face.ascent * point_size * scale,
            descent: config.face.descent * point_size * scale,
            voffset: style.voffset.unwrap_or(0.0),
            cspace,
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
    ascent: f32,
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
    cfg: &Settings,
    adjustment: f32,
) -> Line {
    let mut ascent: f32 = 0.0;
    let mut descent: f32 = 0.0;
    let mut width: f32 = 0.0;
    let mut preferred_width: f32 = 0.0;
    let mut before_carriage_return: f32 = 0.0;
    for pos in &positions {
        let token = &p.tokens[pos.index];
        let metric = &m[pos.index];
        ascent = ascent.max(metric.ascent + metric.voffset);
        descent = descent.max(metric.descent - metric.voffset);
        if token.ch == '\r' {
            before_carriage_return = before_carriage_return.max(pos.pen);
        }
        if visible(token.ch) || (positions.len() == 1 && token.ch.is_whitespace()) {
            // maxAdvance subtracts (character spacing - rich cSpacing).
            // The minus on cSpacing is authored behaviour, not a typo: its
            // closing-tag event also adjusts xAdvance before this expression.
            width = pos.pen + metric.step
                - (cfg.char_spacing * metric.size * 0.01 - metric.cspace) * (1.0 - adjustment);
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
    height: f32,
    wrapping: bool,
    resize_allowed: bool,
) -> Result<Pass, String> {
    let measured = measure(p, cfg, size, adjustment)?;
    let mut lines = Vec::new();
    let mut start = 0;
    let mut previous_hard = true;
    let mut resize = None;
    let mut truncated = false;
    let mut last_soft_used = None;
    while start < p.tokens.len() && !truncated {
        let first_style = &p.segments[p.tokens[start].segment];
        let mut pen = indent(&first_style.indent, size, width);
        if previous_hard {
            pen += match &first_style.line_indent {
                None => 0.0,
                Some(LineIndent::Pixels(n)) => *n,
                Some(LineIndent::Percent(n)) => n * width / 100.0,
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
            if style.position != previous_position {
                if style.position.is_some() {
                    pen = indent(&style.position, m.size, width);
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
                .map(|_| indent(&style.monospace, m.size, width));
            if let Some(mono) = mono {
                origin += (mono - m.fit) * 0.5;
            }
            let fit = origin.abs() + m.fit;
            if visible(token.ch) && fit > width + EPSILON {
                let can_wrap = wrapping && i != start;
                if resize_allowed
                    && ((can_wrap && first_word) || (!can_wrap && purpose == Purpose::Render))
                {
                    resize = Some(Resize::Width {
                        actual: fit,
                        available: width,
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
                mono * (1.0 - adjustment) + cfg.char_spacing * m.size * 0.01 + m.cspace
            });
            if hard_break(token.ch) {
                next_start = i + 1;
                hard = true;
                break;
            }
            if token.ch == '\r' {
                pen = indent(&style.indent, m.size, width);
                continue;
            }
            if break_space(token.ch) {
                saved = Some((i + 1, positions.len()));
                soft = None;
                first_word = false;
                last_cjk = false;
            } else if east_asian(token.ch, rules.modern_hangul) {
                let leading = rules.leading.contains(&token.ch);
                let following = p
                    .tokens
                    .get(i + 1)
                    .is_some_and(|next| rules.following.contains(&next.ch));
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
            positions, start, hard, p, &measured, cfg, adjustment,
        ));
        if next_start <= start {
            return Err("TMP word-wrap failed to advance its source cursor".into());
        }
        start = next_start;
        previous_hard = hard;
    }
    let base_scale = size * cfg.face.unit_scale;
    let mut content = Vec2::ZERO;
    let mut ascent: f32 = 0.0;
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
                previous.descent + lines[i].ascent + cfg.face.leading * size
                    + spacing_delta * base_scale,
            );
            lines[i].offset = previous.offset + gap + (cfg.line_spacing + paragraph) * size * 0.01;
        }
        lowest = lowest.min(-lines[i].descent - lines[i].offset);
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
        resize,
    })
}

fn preferred_round(value: f32) -> f32 {
    ((value * 100.0 + 1.0) as i32) as f32 / 100.0
}

fn calculate(
    p: &Prepared,
    cfg: &Settings,
    rules: &TextRules,
    purpose: Purpose,
    rect: Vec2,
    width_only: bool,
) -> Result<(Pass, f32, f32), String> {
    let mut size = if cfg.auto { cfg.max } else { cfg.size };
    let mut lower = cfg.min;
    let mut upper = cfg.max;
    let mut adjustment = 0.0;
    let mut spacing_delta = 0.0;
    let inner_width = rect.x - cfg.margin[0] - cfg.margin[2];
    let width = if width_only || (purpose == Purpose::Preferred && inner_width == 0.0) {
        LARGE
    } else {
        inner_width
    };
    let height = if purpose == Purpose::Preferred {
        LARGE
    } else {
        rect.y - cfg.margin[1] - cfg.margin[3]
    };
    for iteration in 0..=MAX_ITERATIONS {
        let can_resize = cfg.auto && !width_only && iteration < MAX_ITERATIONS;
        let attempt = pass(
            p,
            cfg,
            rules,
            purpose,
            size,
            adjustment,
            spacing_delta,
            width,
            height,
            !width_only && cfg.wrap,
            can_resize
                && (size > cfg.min
                    || adjustment < cfg.width_adjustment_max
                    || spacing_delta > cfg.line_spacing_min),
        )?;
        match attempt.resize {
            Some(Resize::Width { actual, available })
                if can_resize && adjustment < cfg.width_adjustment_max =>
            {
                let unadjusted = actual / (1.0 - adjustment);
                adjustment =
                    (adjustment + (actual - available) / unadjusted).min(cfg.width_adjustment_max);
                continue;
            }
            Some(Resize::Height { actual, lines })
                if can_resize && lines > 0 && spacing_delta > cfg.line_spacing_min =>
            {
                spacing_delta = (spacing_delta
                    + (height - actual)
                        / lines as f32
                        / (size * cfg.face.unit_scale))
                    .max(cfg.line_spacing_min);
                continue;
            }
            Some(_) if can_resize && size > cfg.min => {
                upper = size;
                size =
                    (((size - ((size - lower) * 0.5).max(0.05)) * 20.0 + 0.5) as i32) as f32 / 20.0;
                size = size.max(cfg.min);
                continue;
            }
            Some(_) => {
                let final_pass = pass(
                    p,
                    cfg,
                    rules,
                    purpose,
                    size,
                    adjustment,
                    spacing_delta,
                    width,
                    height,
                    !width_only && cfg.wrap,
                    false,
                )?;
                return Ok((final_pass, size, adjustment));
            }
            None => {}
        }
        if can_resize && upper - lower > 0.051 && size < cfg.max {
            if adjustment < cfg.width_adjustment_max {
                adjustment = 0.0;
            }
            lower = size;
            size = (((size + ((upper - size) * 0.5).max(0.05)) * 20.0 + 0.5) as i32) as f32 / 20.0;
            size = size.min(cfg.max);
            continue;
        }
        return Ok((attempt, size, adjustment));
    }
    Err("TMP automatic point size exceeded its source iteration bound".into())
}

pub(super) fn preferred_axis(
    text: &str,
    component: &UiComponent,
    axis: usize,
    rect_size: Vec2,
    rules: &TextRules,
) -> Result<f32, String> {
    if axis > 1 || !rect_size.is_finite() {
        return Err("TMP invalid layout axis or rect".into());
    }
    // Alignment changes only the placement inside the rect, not preferred size.
    let config = Settings::read(component, None)?;
    let prepared = prepare(text, &config)?;
    if prepared.tokens.is_empty() {
        return Ok(0.0);
    }
    let (result, _, _) = calculate(
        &prepared,
        &config,
        rules,
        Purpose::Preferred,
        rect_size,
        axis == 0,
    )?;
    let margins = if axis == 0 {
        config.margin[0].max(0.0) + config.margin[2].max(0.0)
    } else {
        config.margin[1].max(0.0) + config.margin[3].max(0.0)
    };
    Ok(preferred_round(result.content[axis] + margins))
}

pub(super) fn layout(
    text: &str,
    component: &UiComponent,
    rect_size: Vec2,
    rules: &TextRules,
    alignment: Option<i64>,
) -> Result<TextLayout, String> {
    if !rect_size.is_finite() {
        return Err("TMP non-finite render rect".into());
    }
    let config = Settings::read(component, alignment)?;
    let prepared = prepare(text, &config)?;
    if prepared.tokens.is_empty() {
        return Ok(TextLayout { glyphs: Vec::new() });
    }
    let (result, _, adjustment) =
        calculate(&prepared, &config, rules, Purpose::Render, rect_size, false)?;
    let inner = Vec2::new(
        rect_size.x - config.margin[0] - config.margin[2],
        rect_size.y - config.margin[1] - config.margin[3],
    );
    let left = -rect_size.x * 0.5 + config.margin[0];
    let top = rect_size.y * 0.5 - config.margin[1];
    let y_shift = match config.vertical {
        256 => 0.0,
        512 => (inner.y - result.content.y) * 0.5,
        1024 => inner.y - result.content.y,
        _ => unreachable!(),
    };
    let mut glyphs = Vec::new();
    for line in &result.lines {
        let style = &prepared.segments[prepared.tokens[line.start].segment];
        let horizontal = match style.align.as_deref() {
            Some("left") => 1,
            Some("center") => 2,
            Some("right") => 4,
            _ => config.horizontal,
        };
        let x_shift = match horizontal {
            1 => 0.0,
            2 => (inner.x - line.width) * 0.5,
            4 => inner.x - line.width,
            _ => unreachable!(),
        };
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
                ch: metric_character(token.ch),
                source_index: token.source,
                font_size: m.size,
                scale: m.scale,
                width_scale: 1.0 - adjustment,
                pen: Vec2::new(
                    left + x_shift + placed.pen + m.pair_offset.x * (1.0 - adjustment),
                    top - y_shift - result.ascent - line.offset + m.voffset + m.pair_offset.y,
                ),
                color,
            });
        }
    }
    Ok(TextLayout { glyphs })
}
