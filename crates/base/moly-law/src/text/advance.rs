//! 推进律：一个字符占多宽。三层：字形表命中走 [`resolve_glyph_advance`]，
//! 缺字形走 [`fallback_advance`]，空格按字族的 [`space_advance_ratio`]。
//!
//! 全半角判定是码点区间表（真源 TextMeshPro 的并集），不查字形——
//! 未知字符的宽由它的码点位置决定，这在真源是缺字形回退的规则。
//! 空格与不换行空格**永远**不走字形表（真源对它们强制取回退路径），
//! 即使图集里查得到。

/// 全角码点区间并集（真源 TextMeshPro）。边界值是测量的一部分：
/// 表内每区间的首尾码点在测试里成对钉住。
pub fn is_fullwidth(ch: char) -> bool {
    let cp = ch as u32;
    matches!(
        cp,
        0x2000..=0x206f
            | 0x2190..=0x21ff
            | 0x2200..=0x22ff
            | 0x2300..=0x23ff
            | 0x2460..=0x24ff
            | 0x2500..=0x259f
            | 0x25a0..=0x25ff
            | 0x2600..=0x26ff
            | 0x2700..=0x27bf
            | 0x3000..=0x30ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0xfe30..=0xfe4f
            | 0xff01..=0xff60
    )
}

/// 空格与不换行空格强制走回退：真源不查它们的字形。
pub fn force_fallback_glyph(ch: char) -> bool {
    ch == ' ' || ch == '\u{00a0}'
}

/// 字族的空格推进比（真源按字族名 contains 匹配）：
/// 少儿体 1/4 · 正黑体与 SkipPro 4/15 · 默认 5/24。
pub fn space_advance_ratio(family: &str) -> f32 {
    if family.contains("FZShaoEr") {
        0.25
    } else if family.contains("FZZhengHei") || family.contains("SkipPro") {
        4.0 / 15.0
    } else {
        5.0 / 24.0
    }
}

/// 缺字形时的回退推进：不换行空格 = 字号；空格 = 字号×推进比取整
/// （四舍五入，0.5 向上）；全角 = 字号；其余 = 字号的一半。
pub fn fallback_advance(ch: char, font_size: f32, family: &str) -> f32 {
    if ch == '\u{00a0}' {
        return font_size;
    }
    if ch == ' ' {
        return (font_size * space_advance_ratio(family)).round();
    }
    if is_fullwidth(ch) {
        font_size
    } else {
        font_size * 0.5
    }
}

/// 命中字形的推进：字形在基准字号下的推进 ×（当前字号 / 基准字号）；
/// 未命中走回退。`glyph_advance` 是**基准字号下**的值（查表方给的）。
pub fn resolve_glyph_advance(
    glyph_advance: Option<f32>,
    ch: char,
    font_size: f32,
    base_size: f32,
    family: &str,
) -> f32 {
    glyph_advance
        .map(|advance| advance * (font_size / base_size))
        .unwrap_or_else(|| fallback_advance(ch, font_size, family))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(cp: u32) -> char {
        char::from_u32(cp).expect("valid code point")
    }

    #[test]
    fn fullwidth_table_boundaries() {
        // 每区间首尾在内、外邻在外。成对写：区间端点必须命中，
        // 它两侧的码点必须不命中——判据会红的输入是任何一个边界漂移。
        let inside: &[u32] = &[
            0x2000, 0x206f, 0x2190, 0x21ff, 0x2200, 0x22ff, 0x2300, 0x23ff, 0x2460, 0x24ff,
            0x2500, 0x259f, 0x25a0, 0x25ff, 0x2600, 0x26ff, 0x2700, 0x27bf, 0x3000, 0x30ff,
            0x3400, 0x4dbf, 0x4e00, 0x9fff, 0xf900, 0xfaff, 0xfe30, 0xfe4f, 0xff01, 0xff60,
        ];
        for cp in inside {
            assert!(is_fullwidth(ch(*cp)), "{cp:04x} should be fullwidth");
        }
        let outside: &[u32] = &[
            0x1fff, 0x2070, 0x218f, 0x2400, 0x245f, 0x27c0, 0x2fff, 0x3100, 0x33ff, 0x4dc0,
            0xa000, 0xf8ff, 0xfb00, 0xfe2f, 0xfe50, 0xff00, 0xff61, 0xffe0,
        ];
        for cp in outside {
            assert!(!is_fullwidth(ch(*cp)), "{cp:04x} should not be fullwidth");
        }
    }

    #[test]
    fn space_ratio_by_family_contains_match() {
        assert_eq!(space_advance_ratio("FZShaoEr"), 0.25);
        assert_eq!(space_advance_ratio("FZShaoEr-M01"), 0.25);
        // contains 匹配：族名里含关键字即命中。
        assert_eq!(space_advance_ratio("xxFZShaoErx"), 0.25);
        assert_eq!(space_advance_ratio("FZZhengHei-B03"), 4.0 / 15.0);
        assert_eq!(space_advance_ratio("SkipPro"), 4.0 / 15.0);
        assert_eq!(space_advance_ratio("Sans"), 5.0 / 24.0);
        assert_eq!(space_advance_ratio("Other"), 5.0 / 24.0);
    }

    #[test]
    fn fallback_advance_classes() {
        // 空格：字号×推进比取整。
        assert_eq!(fallback_advance(' ', 24.0, "Sans"), 5.0);
        assert_eq!(fallback_advance(' ', 24.0, "FZShaoEr-M01"), 6.0);
        // 25×4/15 = 6.66.. → 7（四舍五入）。
        assert_eq!(fallback_advance(' ', 25.0, "FZZhengHei-B03"), 7.0);
        // 26×0.25 = 6.5 → 7（0.5 向上，round 半取整的浮点行为）。
        assert_eq!(fallback_advance(' ', 26.0, "FZShaoEr-M01"), 7.0);
        assert_eq!(fallback_advance(' ', 25.0, "SkipPro"), 7.0);
        assert_eq!(fallback_advance(' ', 25.0, "Sans"), 5.0);
        // 不换行空格 = 字号。
        assert_eq!(fallback_advance('\u{00a0}', 24.0, "Sans"), 24.0);
        // 全角 = 字号。
        assert_eq!(fallback_advance('\u{4e00}', 24.0, "Sans"), 24.0);
        assert_eq!(fallback_advance('\u{ff60}', 24.0, "Sans"), 24.0);
        // 半角 = 字号一半。
        assert_eq!(fallback_advance('a', 24.0, "Sans"), 12.0);
        assert_eq!(fallback_advance('\u{ff61}', 24.0, "Sans"), 12.0);
        // 半角码点表外邻（0xff61 在表外）。
        assert_eq!(fallback_advance('\u{2070}', 24.0, "Sans"), 12.0);
        // 非整数也如实：12.5×1 = 12.5。
        assert_eq!(fallback_advance('\u{4e00}', 12.5, "Sans"), 12.5);
    }

    #[test]
    fn force_fallback_for_space_and_nbsp_only() {
        assert!(force_fallback_glyph(' '));
        assert!(force_fallback_glyph('\u{00a0}'));
        assert!(!force_fallback_glyph('A'));
        assert!(!force_fallback_glyph('\u{4e00}'));
        assert!(!force_fallback_glyph('\u{3000}'));
    }

    #[test]
    fn glyph_advance_scales_then_falls_back() {
        // 命中：advance × (size/base)。
        assert_eq!(resolve_glyph_advance(Some(55.0), 'A', 24.0, 75.0, "Sans"), 17.6);
        assert_eq!(resolve_glyph_advance(Some(55.0), 'A', 12.0, 75.0, "Sans"), 8.8);
        // 浮点非整数性如实保留：60×24/75 = 19.2 的 f32 近似。
        assert_eq!(resolve_glyph_advance(Some(60.0), 'B', 24.0, 75.0, "Sans"), 19.199999);
        // 未命中走回退：空格 5.0、半角 12.0、全角 24.0。
        assert_eq!(resolve_glyph_advance(None, ' ', 24.0, 75.0, "Sans"), 5.0);
        assert_eq!(resolve_glyph_advance(None, 'a', 24.0, 75.0, "Sans"), 12.0);
        assert_eq!(resolve_glyph_advance(None, '\u{4e00}', 24.0, 75.0, "Sans"), 24.0);
    }
}
