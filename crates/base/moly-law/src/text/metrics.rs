//! 行量律：把段装配成行量法——行宽（光标宽）、可见宽（行尾空白悬挂）、
//! 盒宽、锚基、行偏移。纯测量，不放字形；字形查表以闭包入参给出
//! （返回**基准字号下**的推进，未命中为 None）。
//!
//! 运算次序逐句对照真源 TextMeshPro 排版：量宽时推进先乘 `TEXT_SCALE`
//! 再除回去，**不是**恒等变换——浮点上这个往返留下了真源的位形，
//! 改动次序会挪动结果。全角/半角与回退推进见 [`super::advance`]。
//!
//! 怪癖按真源保留（各有测试钉住）：
//! * 行尾一个 `\n` 弹掉（`"A\n"` 是一行）；
//! * 段间空隙 `cspace` 在**行尾**额外补一次（只补最后一个非空段的）；
//! * `<space=N>` 固定段无消费跟踪：它的推进泄漏进它之后每一行；
//! * `scale` 乘进行宽、不进可见宽；
//! * `pos=` 变更把已积累的可见宽**清零**重起（光标宽保留）；
//! * 空行（max_size 近零）的行高复制上一行；
//! * 行偏移里 `line-height=` 覆盖一旦出现就**全程**生效（取首个段的值），
//!   且锚基旁路竖直界（voffset 上下界）。

use super::advance::{force_fallback_glyph, resolve_glyph_advance};
use super::tags::{parse_rich_segments, transformed_glyphs, Indent, SizeSpec};

/// 行量法输出。全部以**排版像素**计（`TEXT_SCALE` 已除回）。
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutMetrics {
    /// 每行光标宽：字符推进 + 段缩放，含行尾空白。
    pub line_widths: Vec<f32>,
    /// 每行可见宽：不含行尾空白（空格悬挂），pos 变更后重起。
    pub rect_widths: Vec<f32>,
    /// 可见宽最大值 + 常数内边距。
    pub box_w: f32,
    /// 竖直锚基：文字中线的 y 偏移（上行降与末行降的均值折半）。
    pub anchor_base: f32,
    /// 每行基线偏移（首行恒 0）。
    pub line_offsets: Vec<f32>,
}

/// 内部固定常数（真源 TextMeshPro 排版）：坐标倍率、点尺寸、
/// 上/下行线、行距、盒内边距。
const TEXT_SCALE: f32 = 2.0;
const TMP_POINT_SIZE: f32 = 75.0;
const ASCENT_LINE: f32 = 66.0;
const DESCENT_LINE: f32 = -9.0;
const LINE_GAP: f32 = 150.0 - (66.0 + 9.0) + 0.625;
const PAD_ORIGINAL: f32 = 64.0 / TEXT_SCALE;

/// 行量法主入口。
///
/// * `text`：带标签的富文本；
/// * `font_size` / `line_spacing` / `font_family`：排版参数（真源层属性）；
/// * `base_size`：字形表的基准字号（图集烘制时的字号）；
/// * `glyph_advance`：按字符查字形推进（基准字号下的值），空格与
///   不换行空格**不会被查**（强制回退），未命中返回 None 走回退。
pub fn layout_metrics(
    text: &str,
    font_size: f32,
    font_family: &str,
    line_spacing: f32,
    base_size: f32,
    glyph_advance: &dyn Fn(char) -> Option<f32>,
) -> LayoutMetrics {
    let segments = parse_rich_segments(text);
    let mut clean: String = segments.iter().map(|seg| seg.text.as_str()).collect();
    if clean.ends_with('\n') {
        clean.pop();
    }
    let line_texts: Vec<String> = clean
        .split('\n')
        .map(|part: &str| part.to_string())
        .collect();

    let seg_cleans: Vec<Vec<char>> = segments
        .iter()
        .map(|seg| seg.text.chars().filter(|ch| *ch != '\n').collect())
        .collect();
    let mut measure_consumed = vec![0usize; segments.len()];
    let mut line_widths = Vec::new();
    let mut rect_widths = Vec::new();
    let mut line_max_sizes = Vec::new();
    let mut vbounds_max_top = f32::NEG_INFINITY;
    let mut vbounds_min_bottom = f32::INFINITY;

    for line_text in &line_texts {
        let mut remaining: Vec<char> = line_text.chars().collect::<Vec<char>>();
        let mut w_scaled = 0.0f32;
        let mut max_seg_size: f32 = 0.0;
        let mut prev_cspace: Option<f32> = None;
        let mut cpv_xadv_tmp = 0.0f32;
        let mut max_cpv_width_tmp = 0.0f32;
        let mut has_chars = false;
        let mut current_position: Option<Indent> = None;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            if let Some(fixed) = seg.fixed_advance {
                // 固定段（<space=N>）：无消费跟踪，逐行重放。
                let seg_font_size = resolve_segment_font_size(&seg.size, font_size);
                if seg.position != current_position {
                    if let Some(pos_shift) = resolve_indent_value(&seg.position, seg_font_size, 0.0)
                    {
                        cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                    }
                    current_position = seg.position.clone();
                }
                let adv = fixed / TEXT_SCALE;
                w_scaled += adv;
                cpv_xadv_tmp += fixed;
                max_cpv_width_tmp =
                    update_cpv_width(max_cpv_width_tmp, cpv_xadv_tmp, 0.0);
                has_chars = true;
                max_seg_size = max_seg_size.max(seg_font_size);
                continue;
            }

            let sc = &seg_cleans[si];
            if sc.is_empty() || measure_consumed[si] >= sc.len() {
                continue;
            }
            let seg_rest = &sc[measure_consumed[si]..];
            let part: Vec<char>;
            if starts_with_chars(&remaining, seg_rest) {
                part = seg_rest.to_vec();
                remaining.drain(..seg_rest.len());
                measure_consumed[si] = sc.len();
            } else if starts_with_chars(seg_rest, &remaining) {
                part = remaining.clone();
                measure_consumed[si] += remaining.len();
                remaining.clear();
            } else {
                continue;
            }
            if part.is_empty() {
                continue;
            }

            let seg_size = resolve_segment_font_size(&seg.size, font_size);
            if seg.position != current_position {
                if let Some(pos_shift) = resolve_indent_value(&seg.position, seg_size, 0.0) {
                    cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                    // pos 变更把已积累的可见宽清零重起。
                    max_cpv_width_tmp = 0.0;
                }
                current_position = seg.position.clone();
            }

            let measure_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let seg_scale = seg.scale.unwrap_or(1.0);
            let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);
            let voffset_tmp = seg.voffset.unwrap_or(0.0);
            let mut measured = 0.0f32;
            let mut rendered_count = 0usize;
            for raw_ch in part {
                for (rendered_ch, char_scale) in transformed_glyphs(raw_ch, seg) {
                    let glyph = if force_fallback_glyph(rendered_ch) {
                        None
                    } else {
                        glyph_advance(rendered_ch)
                    };
                    let glyph_advance_tmp = resolve_glyph_advance(
                        glyph,
                        rendered_ch,
                        measure_size,
                        base_size,
                        font_family,
                    ) * char_scale
                        * TEXT_SCALE;
                    // 乘 TEXT_SCALE 再除回去——保留真源的位形。
                    measured += (glyph_advance_tmp * seg_scale) / TEXT_SCALE;
                    rendered_count += 1;
                    max_cpv_width_tmp = update_cpv_width_for_char(
                        max_cpv_width_tmp,
                        cpv_xadv_tmp,
                        glyph_advance_tmp,
                        raw_ch,
                    );
                    let glyph_asc_tmp = measure_size * (66.0 / 75.0) * TEXT_SCALE;
                    let glyph_des_tmp = measure_size * (9.0 / 75.0) * TEXT_SCALE;
                    vbounds_max_top = vbounds_max_top.max(voffset_tmp + glyph_asc_tmp);
                    vbounds_min_bottom = vbounds_min_bottom.min(voffset_tmp - glyph_des_tmp);
                    cpv_xadv_tmp += glyph_advance_tmp + cspace_raw_tmp;
                }
            }
            let cspace = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            w_scaled += measured + cspace * rendered_count as f32;
            has_chars = true;
            prev_cspace = Some(cspace);
            max_seg_size = max_seg_size.max(seg_size);
        }

        if !remaining.is_empty() {
            // 剩余字符（无段认领，如固定段之后的纯文本）按层字号走。
            for raw_ch in remaining {
                let glyph = if force_fallback_glyph(raw_ch) {
                    None
                } else {
                    glyph_advance(raw_ch)
                };
                let glyph_advance_tmp =
                    resolve_glyph_advance(glyph, raw_ch, font_size, base_size, font_family)
                        * TEXT_SCALE;
                w_scaled += glyph_advance_tmp / TEXT_SCALE;
                max_cpv_width_tmp = update_cpv_width_for_char(
                    max_cpv_width_tmp,
                    cpv_xadv_tmp,
                    glyph_advance_tmp,
                    raw_ch,
                );
                cpv_xadv_tmp += glyph_advance_tmp;
                vbounds_max_top = vbounds_max_top.max(font_size * (66.0 / 75.0) * TEXT_SCALE);
                vbounds_min_bottom =
                    vbounds_min_bottom.min(-font_size * (9.0 / 75.0) * TEXT_SCALE);
            }
            has_chars = true;
            max_seg_size = max_seg_size.max(font_size);
        }

        if max_seg_size < 0.001 {
            // 空行（全空白/空）：字号取最后被消费段的字号，无则首段。
            let consumed_idx = measure_consumed.iter().rposition(|value| *value > 0);
            let active = consumed_idx
                .and_then(|idx| segments.get(idx))
                .or_else(|| segments.first());
            max_seg_size = active
                .map(|seg| resolve_segment_font_size(&seg.size, font_size))
                .unwrap_or(font_size);
        }
        if let Some(cspace) = prev_cspace {
            // 行尾补一次最后段的 cspace。
            w_scaled += cspace;
        }
        line_widths.push(w_scaled);
        rect_widths.push(if has_chars {
            max_cpv_width_tmp / TEXT_SCALE
        } else {
            0.0
        });
        line_max_sizes.push(max_seg_size);
    }

    let mut line_asc = Vec::new();
    let mut line_des = Vec::new();
    for (i, max_size) in line_max_sizes.iter().copied().enumerate() {
        let scale = (max_size / TMP_POINT_SIZE) * TEXT_SCALE;
        let asc = scale * ASCENT_LINE;
        let des = scale * DESCENT_LINE;
        if i == 0 || max_size > 0.001 {
            line_asc.push(asc);
            line_des.push(des);
        } else {
            // 近零行复制上一行。
            line_asc.push(*line_asc.last().unwrap_or(&asc));
            line_des.push(*line_des.last().unwrap_or(&des));
        }
    }

    let mut line_offsets = vec![0.0f32; line_max_sizes.len()];
    let lh_override = segments.iter().find_map(|seg| seg.line_height);
    let ls_tmp = line_spacing * font_size * TEXT_SCALE / TMP_POINT_SIZE;
    for i in 1..line_offsets.len() {
        let delta = if let Some(lh) = lh_override {
            lh + ls_tmp
        } else {
            line_asc[i]
                + line_des[i - 1].abs()
                + LINE_GAP * ((font_size / TMP_POINT_SIZE) * TEXT_SCALE)
                + ls_tmp
        };
        line_offsets[i] = line_offsets[i - 1] + delta;
    }

    let logical_max_asc = line_asc.first().copied().unwrap_or(0.0);
    let logical_min_des =
        line_des.last().copied().unwrap_or(0.0) - line_offsets.last().copied().unwrap_or(0.0);
    let effective_max_asc = if lh_override.is_none() && vbounds_max_top.is_finite() {
        logical_max_asc.max(vbounds_max_top)
    } else {
        logical_max_asc
    };
    let effective_min_des = if lh_override.is_none() && vbounds_min_bottom.is_finite() {
        logical_min_des.min(vbounds_min_bottom)
    } else {
        logical_min_des
    };
    let anchor_base = (effective_max_asc + effective_min_des) / (2.0 * TEXT_SCALE);
    let max_rw = rect_widths.iter().copied().fold(0.0, f32::max);
    let box_w = max_rw + PAD_ORIGINAL;

    LayoutMetrics {
        line_widths,
        rect_widths,
        box_w,
        anchor_base,
        line_offsets,
    }
}

/// 段字号规格解析：None → 层字号；绝对即绝对；增量加在层字号上；
/// 百分比乘层字号；em 乘层字号。
fn resolve_segment_font_size(size: &Option<SizeSpec>, base: f32) -> f32 {
    match size {
        None => base,
        Some(SizeSpec::Absolute(value)) => *value,
        Some(SizeSpec::Delta(value)) => base + *value,
        Some(SizeSpec::Percent(value)) => base * *value / 100.0,
        Some(SizeSpec::Em(value)) => base * *value,
    }
}

/// 缩进类值折算成排版像素：像素折半（TEXT_SCALE）、em 乘字号折半、
/// 百分比乘盒宽（测量 pass 传 0 ⇒ percent 在此为 0）。
fn resolve_indent_value(spec: &Option<Indent>, base_font_size: f32, box_w: f32) -> Option<f32> {
    match spec {
        None => None,
        Some(Indent::Pixels(value)) => Some(*value / TEXT_SCALE),
        Some(Indent::Em(value)) => Some(*value * base_font_size / TEXT_SCALE),
        Some(Indent::Percent(value)) => Some(box_w * *value / 100.0),
    }
}

/// 可见宽推进（不判空白）：当前值与「已走宽 + 本字宽」取大。
fn update_cpv_width(current: f32, before: f32, glyph_advance: f32) -> f32 {
    current.max(before.abs() + glyph_advance)
}

/// 可见宽推进（判空白）：空白字符不推进可见宽（空格悬挂）。
fn update_cpv_width_for_char(current: f32, before: f32, glyph_advance: f32, ch: char) -> f32 {
    if ch.is_whitespace() {
        current
    } else {
        update_cpv_width(current, before, glyph_advance)
    }
}

fn starts_with_chars(left: &[char], prefix: &[char]) -> bool {
    left.len() >= prefix.len() && left.iter().zip(prefix.iter()).all(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无字形表：全部走回退推进（真源锚样例 e02–e05 的口径）。
    fn no_glyphs() -> impl Fn(char) -> Option<f32> {
        |_ch: char| None
    }

    /// 指定字符表：命中给基准推进，其余 None。
    fn table(glyphs: &[(char, f32)]) -> impl Fn(char) -> Option<f32> + '_ {
        move |ch: char| glyphs
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, adv)| *adv)
    }

    fn metrics(text: &str) -> LayoutMetrics {
        layout_metrics(text, 24.0, "Sans", 0.0, 75.0, &no_glyphs())
    }

    fn metrics_glyphs(text: &str, glyphs: &[(char, f32)]) -> LayoutMetrics {
        layout_metrics(text, 24.0, "Sans", 0.0, 75.0, &table(glyphs))
    }

    fn f(a: &[f32]) -> Vec<f32> {
        a.to_vec()
    }

    // ---- 单行基础 ----

    #[test]
    fn fallback_only_single_line() {
        // 半角 12 + 半角 12 = 24。
        let m = metrics("AB");
        assert_eq!(m.line_widths, f(&[24.0]));
        assert_eq!(m.rect_widths, f(&[24.0]));
        assert_eq!(m.box_w, 56.0);
        assert_eq!(m.anchor_base, 9.12);
        assert_eq!(m.line_offsets, f(&[0.0]));
    }

    #[test]
    fn glyph_table_single_line() {
        // A=55、B=60（基准 75）：(55+60)×24/75 = 36.8。
        let m = metrics_glyphs("AB", &[('A', 55.0), ('B', 60.0)]);
        assert_eq!(m.line_widths, f(&[36.8]));
        assert_eq!(m.box_w, 68.8);
    }

    #[test]
    fn mixed_fullwidth_halfwidth() {
        // 半角 12 + 全角 24 = 36。
        let m = metrics("A\u{4e00}");
        assert_eq!(m.line_widths, f(&[36.0]));
        // 全角在表内的边界：0xff60 全角 24、0xff61 半角 12。
        let m = metrics("\u{ff60}\u{ff61}");
        assert_eq!(m.line_widths, f(&[36.0]));
        // 字形表与全角混排：A(55)+汉字(24)+B(60)。
        let m = metrics_glyphs("A\u{4e00}B", &[('A', 55.0), ('B', 60.0)]);
        assert_eq!(m.line_widths, f(&[60.799995]));
    }

    #[test]
    fn smaller_font_size_scales_everything() {
        // 12px：宽 12、锚基折半 4.56、盒宽 12+32=44。
        let m = layout_metrics("AB", 12.0, "Sans", 0.0, 75.0, &no_glyphs());
        assert_eq!(m.line_widths, f(&[12.0]));
        assert_eq!(m.anchor_base, 4.56);
        assert_eq!(m.box_w, 44.0);
    }

    // ---- 空格与悬挂 ----

    #[test]
    fn space_advance_by_family() {
        // Sans 24：space=round(24×5/24)=5 ⇒ 12+5+12=29。
        let m = metrics("A B");
        assert_eq!(m.line_widths, f(&[29.0]));
        assert_eq!(m.rect_widths, f(&[29.0]));
        // FZShaoEr 24：space=round(24×0.25)=6 ⇒ 30。
        let m = layout_metrics("A B", 24.0, "FZShaoEr-M01", 0.0, 75.0, &no_glyphs());
        assert_eq!(m.line_widths, f(&[30.0]));
    }

    #[test]
    fn trailing_spaces_hang_out_of_rect_but_count_in_caret() {
        // "AB   "：行宽 24+15=39（空间计入光标宽），可见宽 24（悬挂）。
        let m = metrics("AB   ");
        assert_eq!(m.line_widths, f(&[39.0]));
        assert_eq!(m.rect_widths, f(&[24.0]));
        // 盒宽按可见宽：24+32=56。
        assert_eq!(m.box_w, 56.0);
        // 中间空格不悬挂：24+5+24=53？真源：51（中间空格推进可见宽）。
        let m = metrics("AB   C");
        assert_eq!(m.line_widths, f(&[51.0]));
        assert_eq!(m.rect_widths, f(&[51.0]));
    }

    #[test]
    fn nbsp_counts_as_advance_not_glyph() {
        // nbsp 推进 = 字号 24：12+24+12=48，且计入可见宽（非空白码点？不——
        // U+00A0 是空白，悬挂）。真源：行宽 48、可见宽 48。
        let m = metrics("A\u{00a0}B");
        assert_eq!(m.line_widths, f(&[48.0]));
        assert_eq!(m.rect_widths, f(&[48.0]));
    }

    // ---- 硬断行 ----

    #[test]
    fn hard_breaks_split_lines() {
        // "A\nB"：两行 [12,12]；行偏移 96.399994。
        let m = metrics("A\nB");
        assert_eq!(m.line_widths, f(&[12.0, 12.0]));
        assert_eq!(m.anchor_base, -14.98);
        assert_eq!(m.line_offsets, f(&[0.0, 96.399994]));
        // <br> 与 <cr> 同效。
        let m = metrics("A<br>B");
        assert_eq!(m.line_widths, f(&[12.0, 12.0]));
        let m = metrics("A<cr>B");
        assert_eq!(m.line_widths, f(&[12.0, 12.0]));
    }

    #[test]
    fn trailing_newline_is_popped() {
        // 行尾 \n 弹掉：一行。
        let m = metrics("A\n");
        assert_eq!(m.line_widths, f(&[12.0]));
        assert_eq!(m.line_offsets, f(&[0.0]));
    }

    #[test]
    fn blank_line_between_text_keeps_middle_line() {
        // "A\n\nB"：三行 [12,0,12]，空行可见宽 0。
        let m = metrics("A\n\nB");
        assert_eq!(m.line_widths, f(&[12.0, 0.0, 12.0]));
        assert_eq!(m.rect_widths, f(&[12.0, 0.0, 12.0]));
        assert_eq!(m.anchor_base, -39.079994);
        assert_eq!(m.line_offsets, f(&[0.0, 96.399994, 192.79999]));
    }

    // ---- 字号与行距 ----

    #[test]
    fn size_tag_changes_line_geometry() {
        // <size=48>B：48px 半角 24 ⇒ 行宽 12+24+12=48。
        let m = metrics("A<size=48>B</size>C");
        assert_eq!(m.line_widths, f(&[48.0]));
        assert_eq!(m.anchor_base, 18.24);
    }

    #[test]
    fn line_spacing_adds_to_offsets() {
        // line_spacing 0.5：偏移从 96.399994 → 96.71999。
        let m = layout_metrics("A\nB", 24.0, "Sans", 0.5, 75.0, &no_glyphs());
        assert_eq!(m.line_offsets, f(&[0.0, 96.71999]));
    }

    #[test]
    fn line_height_override_replaces_offsets_and_bypasses_vbounds() {
        // line-height=200：偏移 [0,200]，锚基走逻辑值（旁路竖直界）。
        let m = metrics("<line-height=200>A\nB</line-height>");
        assert_eq!(m.line_offsets, f(&[0.0, 200.0]));
        assert_eq!(m.anchor_base, -40.879997);
    }

    #[test]
    fn multi_size_lines_stack_offsets() {
        // 第二行 48px：偏移 138.64。
        let m = metrics("A\n<size=48>B</size>");
        assert_eq!(m.line_widths, f(&[12.0, 24.0]));
        assert_eq!(m.line_offsets, f(&[0.0, 138.64]));
    }

    // ---- 怪癖 ----

    #[test]
    fn cspace_added_per_char_and_once_at_line_end() {
        // <cspace=10>AB：10/2=5 每字符 + 行尾再 5 ⇒ 12+5+12+5=37？
        // 真源：行宽 39（12+5+12+5+5？不——cspace 10/TEXT_SCALE=5，
        // 每字符计 5 两次 =10，行尾补 5）⇒ 12+10+5 = 27？真源给 39。
        let m = metrics("<cspace=10>AB</cspace>");
        // 期望值取自真源锚：行宽 39、可见宽 29。
        assert_eq!(m.line_widths, f(&[39.0]));
        assert_eq!(m.rect_widths, f(&[29.0]));
    }

    #[test]
    fn fixed_space_segment_leaks_into_following_lines() {
        // "A<space=30>B\nC"：固定段推进 30/2=15 无消费跟踪——
        // 行 0 = 12+15+12=39；行 1 重放 = 12+15=27（泄漏）。
        let m = metrics("A<space=30>B\nC");
        assert_eq!(m.line_widths, f(&[39.0, 27.0]));
        // 泄漏只进行宽、不进可见宽（固定段经 update_cpv_width）：
        // 真源 rectWidths=[39,27]。锚定真源。
        assert_eq!(m.rect_widths, f(&[39.0, 27.0]));
    }

    #[test]
    fn scale_multiplies_line_width_but_not_rect() {
        // <scale=2>A：行宽 24、可见宽 12（scale 不进可见宽）。
        let m = metrics("<scale=2>A");
        assert_eq!(m.line_widths, f(&[24.0]));
        assert_eq!(m.rect_widths, f(&[12.0]));
    }

    #[test]
    fn position_shift_resets_visible_width() {
        // <pos=50px>AB：50/2=25 起点，可见宽 = 25+24=49；行宽 24。
        let m = metrics("<pos=50px>AB");
        assert_eq!(m.line_widths, f(&[24.0]));
        assert_eq!(m.rect_widths, f(&[49.0]));
        assert_eq!(m.box_w, 81.0);
    }

    #[test]
    fn voffset_pushes_anchor_down() {
        // <voffset=10>A：竖直上界 +10 ⇒ 锚基 9.12+10/2=14.12？真源 11.62。
        let m = metrics("<voffset=10>A</voffset>");
        assert_eq!(m.anchor_base, 11.62);
    }

    #[test]
    fn sub_halves_size_for_measure() {
        // <sub>A：测量字号 12 ⇒ 行宽 6。
        let m = metrics("<sub>A</sub>");
        assert_eq!(m.line_widths, f(&[6.0]));
    }

    #[test]
    fn leading_whitespace_line_has_chars_with_zero_rect() {
        // "   \nAB"：首行全是空白——has_chars=true（段消费过）但可见宽 0；
        // 行宽 15（3 空格×5）。真源：[15,24] / [0,24]。
        let m = metrics("   \nAB");
        assert_eq!(m.line_widths, f(&[15.0, 24.0]));
        assert_eq!(m.rect_widths, f(&[0.0, 24.0]));
    }

    // ---- 变换族进量法 ----

    #[test]
    fn uppercase_uses_transformed_glyphs() {
        // 小写查不到字形走回退 12；大写查到 55/60。
        let m = metrics_glyphs("<uppercase>ab</uppercase>", &[('A', 55.0), ('B', 60.0)]);
        assert_eq!(m.line_widths, f(&[36.8]));
    }

    #[test]
    fn smallcaps_small_letters_get_point_eight() {
        // a→A×0.8、B 不动：55×0.8×24/75 + 60×24/75 = 14.08+19.2=33.28。
        let m = metrics_glyphs("<smallcaps>aB</smallcaps>", &[('A', 55.0), ('B', 60.0)]);
        assert_eq!(m.line_widths, f(&[33.28]));
    }

    #[test]
    fn uppercase_sharp_s_lays_out_two_glyphs() {
        // \u{df} → S,S：2×55×24/75 = 35.2。
        let m = metrics_glyphs("<uppercase>\u{df}</uppercase>", &[('S', 55.0)]);
        assert_eq!(m.line_widths, f(&[35.2]));
    }

    #[test]
    fn nbsp_tag_is_character_advance() {
        // A<nbsp>B：12+24+12=48。
        let m = metrics("A<nbsp>B");
        assert_eq!(m.line_widths, f(&[48.0]));
    }

    // ---- 不变量 ----

    #[test]
    fn empty_text_is_legal_with_no_lines() {
        let m = metrics("");
        // 空文本：clean 为空串，split('\n') 产一行空文本。
        // 真源 e2e 无实例输出（量不到）；单元口径：一行全空。
        assert!(m.line_widths.len() <= 1);
        if m.line_widths.len() == 1 {
            assert_eq!(m.line_widths[0], 0.0);
        }
    }
}
