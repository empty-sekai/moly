//! 富文本标签律：把带标签的文本解析成段，每个标签一个分支。
//! 真源 TextMeshPro 的标签集，逐标签对照原实现移译；未知标签被整吞
//! （标签消失、不留字符、不留状态），与真源的「不认识就吃掉」一致。
//!
//! 段合并：追加字符时若与上一段除文本外全等则并入（跨标签边界也会
//! 合并——`<b>a</b><b>b</b>` 两段样式相同即一段）。`<space=N>` 产出
//! **空文本固定段**：文本为空、推进固定，参与后续行量法。
//!
//! 大小写栈与深度计数并存：颜色/字号/缩进走栈（`/x` 弹栈，弹空即无），
//! 粗斜下划删除上下标走深度（`/x` 减到 0 为止，减不满不动）。两级
//! 语义不同：栈是嵌套恢复，深度是开关叠加。

use super::advance::force_fallback_glyph;

/// 字号规格：绝对像素 · 增量 · 百分比 · em 倍数。
#[derive(Clone, PartialEq, Debug)]
pub enum SizeSpec {
    Absolute(f32),
    Delta(f32),
    Percent(f32),
    Em(f32),
}

/// 缩进类规格（`indent=` / `pos=` / `mspace=` 共用）：百分比 · 像素 · em。
#[derive(Clone, PartialEq, Debug)]
pub enum Indent {
    Percent(f32),
    Pixels(f32),
    Em(f32),
}

/// 行首缩进规格：百分比或像素。
#[derive(Clone, PartialEq, Debug)]
pub enum LineIndent {
    Percent(f32),
    Pixels(f32),
}

/// 大小写变换：无 · 大写 · 小写。
#[derive(Clone, PartialEq, Debug)]
pub enum CaseTransform {
    None,
    Upper,
    Lower,
}

/// 一段同质文本：文本与全部样式快照。段是行量法的输入单位。
#[derive(Clone, PartialEq, Debug)]
pub struct TextSegment {
    pub text: String,
    /// `<space=N>` 产出的固定推进段；文本为空。
    pub fixed_advance: Option<f32>,
    pub color: Option<[f32; 3]>,
    pub size: Option<SizeSpec>,
    pub scale: Option<f32>,
    pub alpha: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// 标记色：真源解析器无 mark 标签分支，此字段恒 None，按真源保留。
    pub mark_color: Option<[f32; 4]>,
    pub superscript: bool,
    pub subscript: bool,
    pub case_transform: CaseTransform,
    pub smallcaps: bool,
    pub voffset: Option<f32>,
    pub rotate: Option<f32>,
    pub cspace: Option<f32>,
    pub line_height: Option<f32>,
    pub line_indent: Option<LineIndent>,
    pub indent: Option<Indent>,
    pub position: Option<Indent>,
    pub monospace: Option<Indent>,
    pub duospace: bool,
    pub align: Option<String>,
}

#[derive(Default)]
struct ParseState {
    segs: Vec<TextSegment>,
    color_stack: Vec<ColorFrame>,
    current_color: Option<[f32; 3]>,
    size_stack: Vec<SizeSpec>,
    scale_stack: Vec<f32>,
    alpha_override: Option<f32>,
    bold_depth: i32,
    italic_depth: i32,
    underline_depth: i32,
    strikethrough_depth: i32,
    subscript_depth: i32,
    superscript_depth: i32,
    smallcaps_depth: i32,
    noparse_depth: i32,
    case_stack: Vec<CaseTransform>,
    voffset_stack: Vec<f32>,
    rotate_stack: Vec<f32>,
    cspace_override: Option<f32>,
    line_height_override: Option<f32>,
    line_indent_override: Option<LineIndent>,
    indent_stack: Vec<Indent>,
    position_stack: Vec<Indent>,
    monospace_stack: Vec<Indent>,
    duospace_stack: Vec<bool>,
    align_stack: Vec<String>,
}

#[derive(Clone)]
struct ColorFrame {
    color: Option<[f32; 3]>,
    alpha: Option<f32>,
}

/// 把带标签文本解析成段。标签一律先转小写再匹配；`<noparse>` 内只认
/// `</noparse>`，其余字符原样保留（包括假标签的字面文本）。
/// 没闭合的 `<`（找不到 `>`）当普通字符保留。
pub fn parse_rich_segments(raw: &str) -> Vec<TextSegment> {
    let mut state = ParseState::default();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if state.noparse_depth > 0 {
            if chars[i] == '<' {
                if let Some(end) = find_gt(&chars, i) {
                    let tag: String = chars[i + 1..end].iter().collect();
                    let tag = tag.to_lowercase();
                    if tag == "/noparse" && handle_tag(&mut state, &tag) {
                        i = end + 1;
                        continue;
                    }
                }
            }
            append_char(&mut state, chars[i]);
            i += 1;
            continue;
        }
        if chars[i] == '<' {
            if let Some(end) = find_gt(&chars, i) {
                let tag: String = chars[i + 1..end].iter().collect();
                let tag = tag.to_lowercase();
                if handle_tag(&mut state, &tag) {
                    i = end + 1;
                    continue;
                }
            }
        }
        append_char(&mut state, chars[i]);
        i += 1;
    }
    state.segs
}

fn find_gt(chars: &[char], start: usize) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(idx, ch)| (*ch == '>').then_some(idx))
}

fn build_segment(state: &ParseState, text: String, fixed_advance: Option<f32>) -> TextSegment {
    TextSegment {
        text,
        fixed_advance,
        color: state.current_color,
        size: state.size_stack.last().cloned(),
        scale: state.scale_stack.last().copied(),
        alpha: state.alpha_override,
        bold: state.bold_depth > 0,
        italic: state.italic_depth > 0,
        underline: state.underline_depth > 0,
        strikethrough: state.strikethrough_depth > 0,
        mark_color: None,
        superscript: state.superscript_depth > 0,
        subscript: state.subscript_depth > 0,
        case_transform: state
            .case_stack
            .last()
            .cloned()
            .unwrap_or(CaseTransform::None),
        smallcaps: state.smallcaps_depth > 0,
        voffset: state.voffset_stack.last().copied(),
        rotate: state.rotate_stack.last().copied(),
        cspace: state.cspace_override,
        line_height: state.line_height_override,
        line_indent: state.line_indent_override.clone(),
        indent: state.indent_stack.last().cloned(),
        position: state.position_stack.last().cloned(),
        monospace: state.monospace_stack.last().cloned(),
        duospace: state.duospace_stack.last().copied().unwrap_or(false),
        align: state.align_stack.last().cloned(),
    }
}

fn append_char(state: &mut ParseState, ch: char) {
    let seg = build_segment(state, ch.to_string(), None);
    if let Some(last) = state.segs.last_mut() {
        if can_merge(last, &seg) {
            last.text.push(ch);
            return;
        }
    }
    state.segs.push(seg);
}

/// 换行类标签（`<br>`/`<cr>`）直接产新段，**不**并入上一段——
/// 行装配按段内 `\n` 切行，独立成段保证切点不被合并吞掉。
fn push_text(state: &mut ParseState, text: &str) {
    state
        .segs
        .push(build_segment(state, text.to_string(), None));
}

/// 处理一个标签。返回它是否被当作标签消费（真源恒真：未知标签也吞）。
/// 标签先被整体转小写，所以大小写不敏感；数值串里混进的 `#` 被剥掉。
fn handle_tag(state: &mut ParseState, tag: &str) -> bool {
    if let Some(rest) = tag.strip_prefix("color=#") {
        return push_color(state, rest);
    }
    if let Some(rest) = tag.strip_prefix('#') {
        return push_color(state, rest);
    }
    if tag == "/color" {
        // /color 只弹嵌套层：栈里只剩底帧（首个压栈的颜色）时弹不动、
        // 样式照旧回落到它——首个颜色一旦压栈就清不掉；空栈才回落无色。
        if state.color_stack.len() > 1 {
            state.color_stack.pop();
        }
        let prev = state
            .color_stack
            .last()
            .cloned()
            .unwrap_or(ColorFrame {
                color: None,
                alpha: None,
            });
        state.current_color = prev.color;
        state.alpha_override = prev.alpha;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("size=") {
        if let Some(parsed) = parse_size(rest) {
            state.size_stack.push(parsed);
        }
        return true;
    }
    if tag == "/size" {
        state.size_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("scale=") {
        if let Some(value) = parse_loose(&rest.replace('#', "")) {
            state.scale_stack.push(value);
        }
        return true;
    }
    if tag == "/scale" {
        state.scale_stack.pop();
        return true;
    }
    if tag == "br" || tag == "cr" {
        push_text(state, "\n");
        return true;
    }
    if tag == "nbsp" {
        append_char(state, '\u{00a0}');
        return true;
    }
    if let Some(rest) = tag.strip_prefix("space=") {
        if let Some(value) = parse_loose(&rest.replace('#', "")) {
            state
                .segs
                .push(build_segment(state, String::new(), Some(value)));
        }
        return true;
    }
    if let Some(rest) = tag.strip_prefix("alpha=") {
        // 只认前两个十六进制字符；空串当 ff。多字节输入会在此切片上
        // panic——真源同样 panic，保持原样（响的拒绝优于静默取值）。
        let hex = rest.replace('#', "");
        let slice = &hex[..hex.len().min(2)];
        state.alpha_override =
            u8::from_str_radix(if slice.is_empty() { "ff" } else { slice }, 16)
                .ok()
                .map(|value| value as f32 / 255.0);
        return true;
    }
    if let Some(rest) = tag.strip_prefix("voffset=") {
        return push_number(&mut state.voffset_stack, rest);
    }
    if tag == "/voffset" {
        state.voffset_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("rotate=") {
        return push_number(&mut state.rotate_stack, rest);
    }
    if tag == "/rotate" {
        state.rotate_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("cspace=") {
        state.cspace_override = parse_loose(&rest.replace('#', ""));
        return true;
    }
    if tag == "/cspace" {
        state.cspace_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("line-height=") {
        state.line_height_override = parse_loose(&rest.replace('#', ""));
        return true;
    }
    if tag == "/line-height" {
        state.line_height_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("line-indent=") {
        state.line_indent_override = parse_line_indent(rest);
        return true;
    }
    if tag == "/line-indent" {
        state.line_indent_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("indent=") {
        return push_indent(&mut state.indent_stack, rest);
    }
    if tag == "/indent" {
        state.indent_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("pos=") {
        return push_indent(&mut state.position_stack, rest);
    }
    if tag == "/pos" {
        state.position_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("mspace=") {
        // `<mspace=N duospace=1>`：首词是格宽，空格分词后找 duospace=1。
        let mut parts = rest.split_whitespace();
        if let Some(first) = parts.next() {
            if let Some(parsed) = parse_indent(first) {
                state.monospace_stack.push(parsed);
                state
                    .duospace_stack
                    .push(parts.any(|part| part.replace('#', "") == "duospace=1"));
            }
        }
        return true;
    }
    if tag == "/mspace" {
        state.monospace_stack.pop();
        state.duospace_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("align=") {
        // justify 不在集合里：不被压栈（值被吞但无效果）。
        if rest == "left" || rest == "center" || rest == "right" {
            state.align_stack.push(rest.to_string());
        }
        return true;
    }
    if tag == "/align" {
        state.align_stack.pop();
        return true;
    }
    match tag {
        "b" => state.bold_depth += 1,
        "/b" => state.bold_depth = (state.bold_depth - 1).max(0),
        "i" => state.italic_depth += 1,
        "/i" => state.italic_depth = (state.italic_depth - 1).max(0),
        "u" => state.underline_depth += 1,
        "/u" => state.underline_depth = (state.underline_depth - 1).max(0),
        "s" => state.strikethrough_depth += 1,
        "/s" => state.strikethrough_depth = (state.strikethrough_depth - 1).max(0),
        "sub" => state.subscript_depth += 1,
        "/sub" => state.subscript_depth = (state.subscript_depth - 1).max(0),
        "sup" => state.superscript_depth += 1,
        "/sup" => state.superscript_depth = (state.superscript_depth - 1).max(0),
        "uppercase" | "allcaps" => state.case_stack.push(CaseTransform::Upper),
        "/uppercase" | "/allcaps" => {
            state.case_stack.pop();
        }
        "lowercase" => state.case_stack.push(CaseTransform::Lower),
        "/lowercase" => {
            state.case_stack.pop();
        }
        "smallcaps" => state.smallcaps_depth += 1,
        "/smallcaps" => state.smallcaps_depth = (state.smallcaps_depth - 1).max(0),
        "noparse" => state.noparse_depth += 1,
        "/noparse" => state.noparse_depth = (state.noparse_depth - 1).max(0),
        _ => {}
    }
    true
}

fn can_merge(a: &TextSegment, b: &TextSegment) -> bool {
    let mut aa = a.clone();
    let mut bb = b.clone();
    aa.text.clear();
    bb.text.clear();
    aa == bb
}

/// 大小写与小型大写变换后的字符。smallcaps 只作用于「小写与自身相等
/// 且大写与自身不等」的字符（纯小写字母），映射到大写并缩放 0.8；
/// 已是大写或非字母的字符不动。变换可能一对多（如德语双 s → 两个大写 s）。
pub fn transform_char(ch: char, seg: &TextSegment) -> (String, f32) {
    if seg.smallcaps
        && ch.to_lowercase().to_string() == ch.to_string()
        && ch.to_uppercase().to_string() != ch.to_string()
    {
        return (ch.to_uppercase().collect::<String>(), 0.8);
    }
    match seg.case_transform {
        CaseTransform::Upper => (ch.to_uppercase().collect::<String>(), 1.0),
        CaseTransform::Lower => (ch.to_lowercase().collect::<String>(), 1.0),
        CaseTransform::None => (ch.to_string(), 1.0),
    }
}

/// 变换展平成 (字符, 缩放) 序列：一个源字符可产出多个字形。
pub fn transformed_glyphs(ch: char, seg: &TextSegment) -> Vec<(char, f32)> {
    let (text, scale) = transform_char(ch, seg);
    text.chars().map(|value| (value, scale)).collect()
}

/// 排版需要请求的字形（去重前）：可见、已变换的字符，剔除换行、回车、
/// 空格与不换行空格（后两者不走字形表，走回退推进）。
pub fn glyph_demand_chars(raw: &str) -> Vec<char> {
    parse_rich_segments(raw)
        .iter()
        .flat_map(|segment| {
            segment.text.chars().flat_map(|source| {
                if source == '\n' || source == '\r' || force_fallback_glyph(source) {
                    Vec::new()
                } else {
                    transformed_glyphs(source, segment)
                        .into_iter()
                        .map(|(value, _)| value)
                        .collect()
                }
            })
        })
        .collect()
}

fn push_color(state: &mut ParseState, hex: &str) -> bool {
    if let Some(parsed) = parse_hex_color(hex) {
        state.current_color = Some([parsed[0], parsed[1], parsed[2]]);
        state.alpha_override = if parsed[3].is_nan() {
            None
        } else {
            Some(parsed[3] / 255.0)
        };
        state.color_stack.push(ColorFrame {
            color: state.current_color,
            alpha: state.alpha_override,
        });
    }
    true
}

/// 十六进制颜色：3/4 位展开（每位×17），6/8 位按对解析；8 位带 alpha，
/// 6 位的 alpha 位是 NaN（上层据此判「没写」）。非法长度返回 None。
fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let value = hex.replace('#', "");
    if value.len() == 3 || value.len() == 4 {
        let mut nums = value
            .chars()
            .filter_map(|ch| ch.to_digit(16).map(|value| value as f32 * 17.0))
            .collect::<Vec<_>>();
        if nums.len() < 3 {
            return None;
        }
        while nums.len() < 4 {
            nums.push(f32::NAN);
        }
        return nums.try_into().ok();
    }
    if value.len() == 6 || value.len() == 8 {
        let r = u8::from_str_radix(&value[0..2], 16).ok()? as f32;
        let g = u8::from_str_radix(&value[2..4], 16).ok()? as f32;
        let b = u8::from_str_radix(&value[4..6], 16).ok()? as f32;
        let a = if value.len() == 8 {
            u8::from_str_radix(&value[6..8], 16).ok()? as f32
        } else {
            f32::NAN
        };
        return Some([r, g, b, a]);
    }
    None
}

fn push_number(stack: &mut Vec<f32>, raw: &str) -> bool {
    if let Some(value) = parse_loose(&raw.replace('#', "")) {
        stack.push(value);
    }
    true
}

fn push_indent(stack: &mut Vec<Indent>, raw: &str) -> bool {
    if let Some(value) = parse_indent(raw) {
        stack.push(value);
    }
    true
}

/// `<size=` 值：`Nem` 倍数 · `N%` 百分比 · `+N/-N` 增量 · 其余绝对。
/// 前缀/后缀都不匹配的串（含 `30#`——`#` 先被剥）按绝对值解析，失败即 None。
fn parse_size(raw: &str) -> Option<SizeSpec> {
    let value = raw.replace('#', "");
    if let Some(rest) = value.strip_suffix("em") {
        parse_as(rest, SizeSpec::Em)
    } else if let Some(rest) = value.strip_suffix('%') {
        parse_as(rest, SizeSpec::Percent)
    } else if let Some(rest) = value.strip_prefix('+') {
        parse_as(rest, SizeSpec::Delta)
    } else if let Some(rest) = value.strip_prefix('-') {
        parse_as(rest, |n| SizeSpec::Delta(-n))
    } else {
        parse_as(&value, SizeSpec::Absolute)
    }
}

/// 缩进类值：`N%` · `Nem` · `Npx` · 裸数按像素。
fn parse_indent(raw: &str) -> Option<Indent> {
    let value = raw.replace('#', "");
    if let Some(rest) = value.strip_suffix('%') {
        parse_as(rest, Indent::Percent)
    } else if let Some(rest) = value.strip_suffix("em") {
        parse_as(rest, Indent::Em)
    } else if let Some(rest) = value.strip_suffix("px") {
        parse_as(rest, Indent::Pixels)
    } else {
        parse_as(&value, Indent::Pixels)
    }
}

fn parse_line_indent(raw: &str) -> Option<LineIndent> {
    if let Some(rest) = raw.strip_suffix('%') {
        parse_as(rest, LineIndent::Percent)
    } else {
        parse_as(raw, LineIndent::Pixels)
    }
}

fn parse_as<T>(raw: &str, map: impl FnOnce(f32) -> T) -> Option<T> {
    parse_loose(raw).map(map)
}

/// 宽松数值解析：从头连吃数字与 `+-.eE`，把吃到的前缀解析成有限 f32；
/// 第一个字符就不合格返回 None。
fn parse_loose(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    let mut end = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        let ok =
            ch.is_ascii_digit() || ch == '+' || ch == '-' || ch == '.' || ch == 'e' || ch == 'E';
        if ok {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    trimmed[..end]
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(raw: &str) -> Vec<TextSegment> {
        parse_rich_segments(raw)
    }

    fn one(raw: &str) -> TextSegment {
        let segs = seg(raw);
        assert_eq!(segs.len(), 1, "{raw} -> {segs:?}");
        segs.into_iter().next().unwrap()
    }

    // ---- 基础与合并 ----

    #[test]
    fn plain_text_is_one_segment_with_default_style() {
        let s = one("abc");
        assert_eq!(s.text, "abc");
        assert_eq!(s.bold, false);
        assert_eq!(s.case_transform, CaseTransform::None);
        assert!(s.color.is_none());
    }

    #[test]
    fn same_style_tags_merge_across_boundary() {
        // 两个 <b> 相邻、内容相接 → 一段（can_merge 只比样式）。
        let segs = seg("<b>a</b><b>b</b>");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "ab");
        assert!(segs[0].bold);
    }

    #[test]
    fn br_and_cr_push_newline_segment_without_merge() {
        // <br> 不并入前段：切行靠段内 \n，独立成段保切点。
        let segs = seg("a<br>b");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "a");
        assert_eq!(segs[1].text, "\nb");
        let segs = seg("a<cr>b");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].text, "\nb");
    }

    #[test]
    fn nbsp_is_a_character_not_a_break() {
        let s = one("A<nbsp>B");
        assert_eq!(s.text, "A\u{00a0}B");
    }

    // ---- 标签大小写与未知标签 ----

    #[test]
    fn tags_are_case_insensitive() {
        let s = one("<B>a</B>");
        assert!(s.bold);
    }

    #[test]
    fn unknown_tag_is_consumed_silently() {
        // 未知标签整吞：字符不留、状态不动。
        let s = one("a<xyz>b</xyz>c");
        assert_eq!(s.text, "abc");
    }

    #[test]
    fn unclosed_angle_is_kept_as_text() {
        // 找不到 > 的 < 当普通字符。
        let s = one("a<b");
        assert_eq!(s.text, "a<b");
        let s = one("a<bc");
        assert_eq!(s.text, "a<bc");
    }

    #[test]
    fn tags_tolerate_spaces_before_value() {
        // 标签已整体转小写，`<color = #ff0000>` 里值含前导空格——
        // 十六进制解析在 color=# 之后原样吃，非法字符即 None。
        let s = one("<color = #ff0000>a");
        assert!(s.color.is_none(), "{s:?}");
    }

    // ---- 颜色族 ----

    #[test]
    fn color_hex6_sets_rgb_without_alpha() {
        // /color 只弹嵌套层：栈里只剩首个颜色时弹不动，后续文本仍是该色
        // 且与前段合并成一段（真源锚样例）。
        let segs = seg("<color=#ff0000>a</color>b");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "ab");
        assert_eq!(segs[0].color, Some([255.0, 0.0, 0.0]));
    }

    #[test]
    fn hash_short_form_is_color() {
        let s = one("<#00ff00>a");
        assert_eq!(s.color, Some([0.0, 255.0, 0.0]));
    }

    #[test]
    fn color_hex3_expands_by_17() {
        // f00 → (0xf*17, 0, 0) = (255, 0, 0)。
        let s = one("<color=#f00>a");
        assert_eq!(s.color, Some([255.0, 0.0, 0.0]));
    }

    #[test]
    fn color_hex8_sets_alpha_from_last_pair() {
        let s = one("<color=#ff000080>a");
        assert_eq!(s.color, Some([255.0, 0.0, 0.0]));
        assert_eq!(s.alpha, Some(128.0 / 255.0));
    }

    #[test]
    fn invalid_color_keeps_previous() {
        let s = one("<color=zzz>a");
        assert!(s.color.is_none());
    }

    #[test]
    fn nested_colors_restore_on_close() {
        let segs = seg("<color=#ff0000>a<color=#00ff00>b</color>c</color>d");
        assert_eq!(segs[0].color, Some([255.0, 0.0, 0.0]));
        assert_eq!(segs[1].color, Some([0.0, 255.0, 0.0]));
        assert_eq!(segs[2].text, "cd");
        assert_eq!(segs[2].color, Some([255.0, 0.0, 0.0]));
        assert_eq!(segs.len(), 3);
    }

    #[test]
    fn close_color_beyond_bottom_keeps_bottom_color() {
        // 空栈时 /color 回落到无色（没有底帧）。
        let segs = seg("a</color>b");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "ab");
        assert!(segs[0].color.is_none());
        // 只有底帧时 /color 清不掉首个颜色。
        let segs = seg("<color=#ff0000>a</color>b");
        assert_eq!(segs[0].color, Some([255.0, 0.0, 0.0]));
    }

    // ---- 字号族 ----

    #[test]
    fn size_forms() {
        assert_eq!(one("<size=50%>a").size, Some(SizeSpec::Percent(50.0)));
        assert_eq!(one("<size=+2>a").size, Some(SizeSpec::Delta(2.0)));
        assert_eq!(one("<size=-2>a").size, Some(SizeSpec::Delta(-2.0)));
        assert_eq!(one("<size=1.5em>a").size, Some(SizeSpec::Em(1.5)));
        assert_eq!(one("<size=30>a").size, Some(SizeSpec::Absolute(30.0)));
        // 数值串里的 # 被剥掉后照常解析。
        assert_eq!(one("<size=30#>a").size, Some(SizeSpec::Absolute(30.0)));
        // 解析失败 → 无字号。
        assert_eq!(one("<size=abc>a").size, None);
    }

    #[test]
    fn scale_tag_pushes_scale() {
        let segs = seg("<scale=2>a</scale>b");
        assert_eq!(segs[0].scale, Some(2.0));
        assert_eq!(segs[1].scale, None);
    }

    // ---- alpha 族 ----

    #[test]
    fn alpha_two_hex_digits() {
        assert_eq!(one("<alpha=#cc>a").alpha, Some(0.8));
        assert_eq!(one("<alpha=cc>a").alpha, Some(0.8));
        // 空 alpha 当 ff（完全不透明）。
        assert_eq!(one("<alpha=#>a").alpha, Some(1.0));
        // 只取前两位：0x12 → 18/255 = 0.07058824。
        assert_eq!(one("<alpha=#1234>a").alpha, Some(18.0 / 255.0));
    }

    // ---- 深度族 ----

    #[test]
    fn boolean_tags_toggle() {
        assert!(one("<i>a</i>").italic);
        assert!(one("<u>a</u>").underline);
        assert!(one("<s>a</s>").strikethrough);
        assert!(one("<sub>a</sub>").subscript);
        assert!(one("<sup>a</sup>").superscript);
    }

    #[test]
    fn depth_counters_never_go_negative() {
        // 减不满不动：额外 /b 不会把深度减到负。
        let s = one("</b><b>a");
        assert!(s.bold);
    }

    // ---- 大小写族 ----

    #[test]
    fn case_tags_stack_and_transform() {
        assert_eq!(one("<uppercase>ab</uppercase>").case_transform, CaseTransform::Upper);
        assert_eq!(one("<allcaps>ab</allcaps>").case_transform, CaseTransform::Upper);
        assert_eq!(one("<lowercase>AB</lowercase>").case_transform, CaseTransform::Lower);
        assert_eq!(one("<smallcaps>a</smallcaps>").smallcaps, true);
    }

    // ---- noparse ----

    #[test]
    fn noparse_preserves_tag_characters_literally() {
        let s = one("<noparse><b>x</noparse>y");
        assert_eq!(s.text, "<b>xy");
    }

    // ---- 排版类标签 ----

    #[test]
    fn layout_tags_push_overrides_and_stacks() {
        assert_eq!(one("<voffset=5>a").voffset, Some(5.0));
        assert_eq!(one("<rotate=15>a").rotate, Some(15.0));
        assert_eq!(one("<cspace=2.5>a").cspace, Some(2.5));
        assert_eq!(one("<line-height=40>a").line_height, Some(40.0));
        assert_eq!(
            one("<line-indent=50%>a").line_indent,
            Some(LineIndent::Percent(50.0))
        );
        assert_eq!(one("<line-indent=40>a").line_indent, Some(LineIndent::Pixels(40.0)));
        assert_eq!(one("<indent=2em>a").indent, Some(Indent::Em(2.0)));
        assert_eq!(one("<indent=10%>a").indent, Some(Indent::Percent(10.0)));
        assert_eq!(one("<indent=5px>a").indent, Some(Indent::Pixels(5.0)));
        assert_eq!(one("<indent=5>a").indent, Some(Indent::Pixels(5.0)));
        assert_eq!(one("<pos=10px>a").position, Some(Indent::Pixels(10.0)));
    }

    #[test]
    fn mspace_forms() {
        assert_eq!(one("<mspace=20>a").monospace, Some(Indent::Pixels(20.0)));
        assert_eq!(one("<mspace=20 duospace=1>a").duospace, true);
        assert_eq!(one("<mspace=20 duospace=0>a").duospace, false);
    }

    #[test]
    fn align_only_three_values() {
        assert_eq!(one("<align=center>a").align.as_deref(), Some("center"));
        // justify 不在集合里：标签被吞但不压栈。
        assert_eq!(one("<align=justify>a").align, None);
    }

    // ---- 固定段 ----

    #[test]
    fn space_tag_makes_empty_fixed_segment() {
        let segs = seg("<space=25.5>");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "");
        assert_eq!(segs[0].fixed_advance, Some(25.5));
    }

    // ---- 变换 ----

    #[test]
    fn transform_upper_and_lower() {
        let segs = seg("<uppercase>a</uppercase>");
        assert_eq!(transform_char('a', &segs[0]), ("A".to_string(), 1.0));
        let segs = seg("<lowercase>A</lowercase>");
        assert_eq!(transform_char('A', &segs[0]), ("a".to_string(), 1.0));
        // 数字不变换。
        let segs = seg("<uppercase>1</uppercase>");
        assert_eq!(transform_char('1', &segs[0]), ("1".to_string(), 1.0));
    }

    #[test]
    fn transform_smallcaps_only_lowercase_letters() {
        let segs = seg("<smallcaps>a</smallcaps>");
        assert_eq!(transform_char('a', &segs[0]), ("A".to_string(), 0.8));
        // 已是大写：不动、不缩。
        let segs = seg("<smallcaps>A</smallcaps>");
        assert_eq!(transform_char('A', &segs[0]), ("A".to_string(), 1.0));
        // 数字不动。
        let segs = seg("<smallcaps>1</smallcaps>");
        assert_eq!(transform_char('1', &segs[0]), ("1".to_string(), 1.0));
    }

    #[test]
    fn transformed_glyphs_flatten_multi_char() {
        // 德语双 s → 两个大写 s（一对多变换）。
        let segs = seg("<uppercase>\u{df}</uppercase>");
        assert_eq!(
            transformed_glyphs('\u{df}', &segs[0]),
            vec![('S', 1.0), ('S', 1.0)]
        );
        let segs = seg("<smallcaps>\u{df}</smallcaps>");
        assert_eq!(
            transformed_glyphs('\u{df}', &segs[0]),
            vec![('S', 0.8), ('S', 0.8)]
        );
    }

    // ---- 字形需求 ----

    #[test]
    fn glyph_demand_drops_breaks_spaces_and_transforms() {
        assert_eq!(
            glyph_demand_chars("<uppercase>a\u{df}</uppercase> <noparse><b></noparse>"),
            vec!['A', 'S', 'S', '<', 'b', '>']
        );
        assert_eq!(glyph_demand_chars("a b\u{a0}c"), vec!['a', 'b', 'c']);
        assert_eq!(glyph_demand_chars("<smallcaps>a</smallcaps>"), vec!['A']);
        assert_eq!(glyph_demand_chars("x<br>y"), vec!['x', 'y']);
        assert!(glyph_demand_chars("").is_empty());
    }
}
