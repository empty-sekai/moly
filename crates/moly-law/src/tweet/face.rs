//! tweet 行到角色域的装配输入：动作段、眼/口图样、头顶表情件。
//!
//! 只给数据形状，不接渲染。三个键的消费去向：
//! - `motion` 是动作库段名（段后缀族 `_S`/`_L`/`_E`/`_O` 的衔接归
//!   上屏单）；
//! - `eye`/`mouth` 即 facial 两表的图样键——眼表的 `PatternName`、
//!   口型表的 `Name`；
//! - `emoticon` 是头顶表情件名。
//!
//! 源 TweetState 对三个键的空值处理不同：动作与表情件可空
//! （空 ⇒ 不播/不出），眼/口名空走 LogError 错误路径。

use super::row::TweetRow;

/// 一条 tweet 对角色域的全部装配输入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TweetFace<'a> {
    /// 动作段名；`None` = 本次不换动作。
    pub motion: Option<&'a str>,
    /// 眼图样名（facial 眼表 `PatternName` 键）。
    pub eye: &'a str,
    /// 口型图样名（facial 口型表 `Name` 键）。
    pub mouth: &'a str,
    /// 头顶表情件名；`None` = 不出表情件。
    ///
    /// 源 TweetState 以**空引用**为门（空串会照发）；问候路径用的是
    /// 空引用或空串都拦的门。此处保留行数据原样，把空串留给调用方
    /// 按它走的路径裁决。
    pub emoticon: Option<&'a str>,
}

impl TweetRow {
    /// 该 tweet 的角色域装配输入。
    pub fn face(&self) -> TweetFace<'_> {
        TweetFace {
            motion: self.motion_name.as_deref(),
            eye: &self.eye_name,
            mouth: &self.mouth_name,
            emoticon: self.emoticon_name.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_maps_all_four_keys() {
        let row = TweetRow {
            id: 1,
            motion_name: Some("mov_cw_adult_01joy001".to_string()),
            emoticon_name: Some("fx_emote_001".to_string()),
            eye_name: "smile".to_string(),
            mouth_name: "smile01".to_string(),
            text: "synthetic".to_string(),
        };
        let face = row.face();
        assert_eq!(face.motion, Some("mov_cw_adult_01joy001"));
        assert_eq!(face.eye, "smile");
        assert_eq!(face.mouth, "smile01");
        assert_eq!(face.emoticon, Some("fx_emote_001"));
    }

    #[test]
    fn face_none_arms_keep_empty_strings() {
        // motion/emoticon 空引用 ⇒ None；eye/mouth 无 None 臂（行类型
        // 即不可空）；emoticon 的空串保留为 Some("")——门是调用方的事
        let row = TweetRow {
            id: 2,
            motion_name: None,
            emoticon_name: Some(String::new()),
            eye_name: "normal".to_string(),
            mouth_name: "normal01".to_string(),
            text: String::new(),
        };
        let face = row.face();
        assert_eq!(face.motion, None);
        assert_eq!(face.emoticon, Some(""));
        assert_eq!(face.eye, "normal");
        assert_eq!(face.mouth, "normal01");
    }
}
