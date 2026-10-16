//! 文字排版律：真源 TextMeshPro 的断行、富文本标签、全半角与行量法，纯函数。
//!
//! 分四份：[`tags`] 把富文本解析成段（每个标签一个分支，逐字符合并成段）；
//! [`advance`] 给出每字符的推进宽（全半角码点表、空格推进比、缺字形回退）；
//! [`metrics`] 把段装配成行量法（行宽/可见宽/盒宽/锚基/行偏移）；
//! [`plain`] 把带标签的文本剥成纯文本并找数字串。
//!
//! 本模块只处理硬断行：`\n`、`<br>`、`<cr>`。这不是说真源没有软换行——
//! TextMeshPro 开启 `enableWordWrapping` 时按行宽自动折行（断点表、
//! U+00A0 不断行），UI 排版的软换行在 moly-game 的 TMP 排版里实现；
//! 本模块移植的那份排版律只有硬断行，需要软换行的消费方不从这里取。
//!
//! 字形查表以闭包入参给出：真源按「区域+字哈+字族+字符」四元组查图集，
//! 四元组的前三者在一次排版内是常量，等价于按字符查。
//! 闭包返回的是**基准字号下的推进**；字号换算在本模块内做。

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
