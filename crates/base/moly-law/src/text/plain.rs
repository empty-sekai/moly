//! 纯文本律：把带标签的文本剥成纯文本，并在纯文本上找 ASCII 数字串。
//! 剥标签不认识标签语义：任何 `<` 到最近 `>` 的跨度整体丢弃；
//! 没闭合的 `<`（吃到底也没有 `>`）原样保留。

/// 剥掉富文本标签后的纯文本。
pub fn strip_tmp_tags(source: &str) -> String {
    let mut plain = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '<' {
            plain.push(ch);
            continue;
        }
        let mut candidate = String::from('<');
        let mut closed = false;
        for next in chars.by_ref() {
            candidate.push(next);
            if next == '>' {
                closed = true;
                break;
            }
        }
        if !closed {
            plain.push_str(&candidate);
        }
    }
    plain
}

/// 纯文本里的一段连续 ASCII 数字（含起止下标，按剥后的字符计）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NumericTextRun {
    pub text: String,
    pub plain_start: u32,
    pub plain_end: u32,
}

/// 找全部数字串：可见符号与换行都会切断连续数字（跨标签接起来的
/// 数字算同一串——标签剥掉后字符相邻）。
pub fn numeric_text_runs(source: &str) -> Vec<NumericTextRun> {
    let plain = strip_tmp_tags(source);
    let chars: Vec<char> = plain.chars().collect();
    let mut runs = Vec::new();
    let mut cursor = 0;
    while cursor < chars.len() {
        if !chars[cursor].is_ascii_digit() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < chars.len() && chars[cursor].is_ascii_digit() {
            cursor += 1;
        }
        runs.push(NumericTextRun {
            text: chars[start..cursor].iter().collect(),
            plain_start: start as u32,
            plain_end: cursor as u32,
        });
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_any_tag_span() {
        assert_eq!(strip_tmp_tags("<color=#fff>12</color><b>34</b>"), "1234");
        assert_eq!(strip_tmp_tags("<br>"), "");
        assert_eq!(strip_tmp_tags("no tags"), "no tags");
        // 空 <> 也是标签跨度。
        assert_eq!(strip_tmp_tags("a<>b"), "ab");
    }

    #[test]
    fn strip_keeps_unclosed_angle_literally() {
        // 找不到 >：candidate 原样保留（含那个 <）。
        assert_eq!(strip_tmp_tags("a<b"), "a<b");
        assert_eq!(strip_tmp_tags("a<bc"), "a<bc");
        assert_eq!(strip_tmp_tags(""), "");
    }

    #[test]
    fn numeric_runs_join_across_tags_and_split_on_symbols() {
        // 标签两侧的数字接成一串（真源用例）。
        assert_eq!(
            numeric_text_runs("<color=#fff>12</color><b>34</b>"),
            vec![NumericTextRun {
                text: "1234".into(),
                plain_start: 0,
                plain_end: 4,
            }]
        );
        assert_eq!(
            numeric_text_runs("0012-34\n56"),
            vec![
                NumericTextRun {
                    text: "0012".into(),
                    plain_start: 0,
                    plain_end: 4
                },
                NumericTextRun {
                    text: "34".into(),
                    plain_start: 5,
                    plain_end: 7
                },
                NumericTextRun {
                    text: "56".into(),
                    plain_start: 8,
                    plain_end: 10
                },
            ]
        );
        // 前导零保留；单数字成串；无数字为空。
        assert_eq!(
            numeric_text_runs("9"),
            vec![NumericTextRun {
                text: "9".into(),
                plain_start: 0,
                plain_end: 1
            }]
        );
        assert!(numeric_text_runs("").is_empty());
        assert!(numeric_text_runs("abc").is_empty());
        // 未知标签照剥（剥标签不认识语义）：数字接上。
        assert_eq!(
            numeric_text_runs("1<xyz>2"),
            vec![NumericTextRun {
                text: "12".into(),
                plain_start: 0,
                plain_end: 2
            }]
        );
    }
}
