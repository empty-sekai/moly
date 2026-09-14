//! 待机动作域的主表行：与真源 XLua 剧本表的行结构一一对应。
//!
//! 源是 XLua 宿主：场景剧本按角色单位各存一份文本资产（表行只带
//! 单位 id 与剧本名三列），选取与步进逻辑写在剧本内。这里的行类型
//! 镜像**剧本产物的编排形状**——scenario（触发条件 + 步表）、tail
//! （每轮收尾步表）、step（一拍编排）。数值语义归
//! [`super::select`] 与 [`super::step`]；本层只给形状，不做任何
//! 判断。
//!
//! 数据文件里 `scenario.kind` 与 `scenario.trigger.kind` 双写同值
//! （333 场景现算 333/333 恒同），类型上消掉冗余列：[`Trigger`]
//! 的变体即 kind。

/// 一条待机场景：触发条件 + 编排步表。
///
/// `id` 在源数据是字符串，且**单位内不保证唯一**（实测有单位存在
/// 两条同 id 场景，触发条件相同、动作不同）——id 不作键，
/// timeGated 的去重键来自脚本的数值槽，不从场景 id 推断。
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub id: String,
    pub trigger: Trigger,
    pub steps: Vec<Step>,
}

/// 触发条件。变体即场景 kind；闭集两值。
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// 脚本启动后的上限窗口、整数概率、可选槽记忆。
    TimeGated(TimeGatedTrigger),
    /// 权重分支：按一次 `[0,100)` 掷点落段。
    RandomBranch(RandomBranchTrigger),
}

/// timeGated 触发行。
///
/// `time_limit_name` / `probability_name` 是源表的**名字列**，与值
/// 列解耦（实测有名 `timeLimit_10` 值为 5 的行）——门只读值列，
/// 名字列原样携带不解释。
#[derive(Debug, Clone, PartialEq)]
pub struct TimeGatedTrigger {
    pub time_limit_name: String,
    /// 脚本启动后的上限窗口（秒，不含上界）。
    pub time_limit_seconds: f64,
    pub probability_name: String,
    /// 命中概率，`[0,1]`。
    pub probability: f64,
    /// 去重槽的数值键；None 表示完全不检查槽记忆。
    pub motion_slot: Option<String>,
    /// 槽记忆时长（秒）：上次选中该槽后多久内不再选。
    pub slot_memory_seconds: f64,
}

/// randomBranch 触发行。
///
/// `low`/`high` 是 `[low, high)` 半开段边界（百分尺）；`weight` 是
/// 名义权重（实测段宽恒 20、权重恒 0.2，两者一致），选取只按段
/// 边界落点，不读 `weight`。
#[derive(Debug, Clone, PartialEq)]
pub struct RandomBranchTrigger {
    pub low: f64,
    pub high: f64,
    pub weight: f64,
}

/// Control flow extracted from the script, independently of display labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    SequentialIfs,
    RandomBranch,
}

#[derive(Debug, Clone)]
pub struct ProgramBlock {
    pub scenario: usize,
    /// A block-tail write of the cached loop-start wall second.
    pub remember_slot: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub kind: ProgramKind,
    pub blocks: Vec<ProgramBlock>,
}

/// 一拍编排。变体即 op；闭集六值（`emoticon` 与 `hideEmoticon` 是
/// 两个独立 op，不是一开一合的同键）。
///
/// 动作、表情（eye/mouth）、气泡（emoticon）是三条**独立通道**：
/// 配对只存在于本编排数据里，通道间无耦合。`wait` 不出事件——它是
/// 标称时间轴的承重（每步的 `t` == 此前各 `wait` 的 `seconds` 之
/// 和，数据 333/333 场景零容差成立）。
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// 换眼图样。`pattern` 是 facial 眼表的 `PatternName` 键——与
    /// tweet 域 `TweetFace::eye` 同一键域。
    ChangeEye {
        t: f32,
        pattern: String,
        alias: String,
    },
    /// 换口型。`pattern` 是 facial 口型表的 `Name` 键——与
    /// tweet 域 `TweetFace::mouth` 同一键域。
    ChangeMouth {
        t: f32,
        pattern: String,
        alias: String,
    },
    /// 换动作。
    ChangeAnimation {
        t: f32,
        /// 动作库段名（基名；段后缀族见 [`SegmentSuffix`]）。
        motion: String,
        alias: String,
        /// 标称播放速度。**0 是哨值**：源执行门读到 0 时按 1.0 播
        /// （换算见 [`super::step::effective_speed`]）。
        speed: f32,
        playback_speed: f32,
        play_end_motion: bool,
        /// 入场混合时长（秒）。
        blend: f32,
        phase: Option<SegmentSuffix>,
        phase_source: Option<SegmentSuffix>,
    },
    /// 出头顶表情件。保留原调用的展示时长；宿主不拿它倒计时。
    /// 包装函数产生的 Hide 是另一条有序指令。
    ShowEmoticon {
        t: f32,
        name: String,
        /// Authored value, not an automatic hide timer.
        show_seconds: f32,
        /// Authored wrapper arguments, kept separate from the managed binding.
        time_arg: Option<f64>,
        not_play_se_arg: Option<bool>,
        host_not_play_se: bool,
    },
    /// 收头顶表情件。
    HideEmoticon { t: f32 },
    /// 等待。承重时间轴；见枚举文档。
    Wait { t: f32, seconds: f32 },
}

/// 动作段后缀族：`_S` / `_L` / `_E` / `_O`。
///
/// 值到序号的映射是源常量表（S=0 / L=1 / E=2 / O=3）；编排步的
/// `phase`/`phaseSource` 两列实测只出现 `E`。四字母的展开语义
/// 无可读来源，不猜测命名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentSuffix {
    S,
    L,
    E,
    O,
}

impl SegmentSuffix {
    /// 源常量表给的序号。
    pub fn index(self) -> i32 {
        match self {
            SegmentSuffix::S => 0,
            SegmentSuffix::L => 1,
            SegmentSuffix::E => 2,
            SegmentSuffix::O => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_suffix_indices_match_constant_table() {
        // 源常量表：L=1 / S=0 / E=2 / O=3
        assert_eq!(SegmentSuffix::S.index(), 0);
        assert_eq!(SegmentSuffix::L.index(), 1);
        assert_eq!(SegmentSuffix::E.index(), 2);
        assert_eq!(SegmentSuffix::O.index(), 3);
    }
}
