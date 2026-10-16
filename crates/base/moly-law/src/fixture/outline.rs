//! 家具 outline 五参数的下发律。
//!
//! 源路径：站点环境配置里的 `GlobalFixtureSettingsData`（五个 SerializeField
//! 浮点/颜色）→ 每帧 `EnvironmentShaderView.OnUpdate` 的家具块 → 五个全局
//! shader 量。本文件转录那五次写入的取值式，不猜任何配置值。
//!
//! 换算与阈值的要点（都钉在源方法体上）：
//! - Width、DepthOffset 是「配置单位 → 世界单位」的千分比：写之前 ×0.001；
//!   MinRate/MaxRate 直写不换算；
//! - 颜色 RGBA 四分量、以及每个浮点，各自独立过一个 0.001 的近零阈：
//!   低于阈的分量写 0，**不是跳过这次写**——全局量恒被赋值。这一点
//!   写错了，消费端会把「写 0」误做成「保持上一帧」，轮廓会在参数
//!   归零时残留旧值。

/// 源侧近零阈的具名值：安全写入族（SetGlobalFloatSafe 一系）的默认参数。
pub const ZERO_VALUE_THRESHOLD: f32 = 0.001;

/// 配置单位到世界单位的千分比换算，Width/DepthOffset 共用。
pub const CONFIG_TO_WORLD: f32 = 0.001;

/// 配置面：字段与 `GlobalFixtureSettingsData` 的序列化字段一一对应。
/// 值域声明（Width ∈ [0,50]、DepthOffset ∈ [-10,10]、两个 Rate ∈ [0,1]）
/// 是编辑器滑条约束，运行时律不依赖它。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlobalFixtureSettings {
    pub outline_width: f32,
    pub outline_depth_offset: f32,
    pub outline_width_min_rate: f32,
    pub outline_width_max_rate: f32,
    pub outline_color: [f32; 4],
}

/// 一帧的全局量，字段与源的全局量组一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FixtureOutlineGlobals {
    pub outline_color: [f32; 4],
    pub outline_width: f32,
    pub outline_depth_offset: f32,
    pub outline_width_min_rate: f32,
    pub outline_width_max_rate: f32,
}

/// 近零阈的取值式：`threshold <= |value| ? value : 0`。
/// 与源的差别只在「不写」被折掉了——纯函数没有「上一帧」，
/// 返回 0.0 就是源那次 `SetGlobalFloat(0)` 的效果。
fn safe_float(value: f32) -> f32 {
    if ZERO_VALUE_THRESHOLD <= value.abs() {
        value
    } else {
        0.0
    }
}

/// 颜色四分量各过各的近零阈，互不影响。
fn safe_color(color: [f32; 4]) -> [f32; 4] {
    [
        safe_float(color[0]),
        safe_float(color[1]),
        safe_float(color[2]),
        safe_float(color[3]),
    ]
}

/// 家具 outline 五参数的一次完整下发。
/// 源的写入顺序是 Color → DepthOffset → Width → MinRate → MaxRate，
/// 顺序对纯函数的结果无影响，保留在此注释里供对账。
pub fn outline_globals(settings: &GlobalFixtureSettings) -> FixtureOutlineGlobals {
    FixtureOutlineGlobals {
        outline_color: safe_color(settings.outline_color),
        outline_depth_offset: safe_float(settings.outline_depth_offset * CONFIG_TO_WORLD),
        outline_width: safe_float(settings.outline_width * CONFIG_TO_WORLD),
        outline_width_min_rate: safe_float(settings.outline_width_min_rate),
        outline_width_max_rate: safe_float(settings.outline_width_max_rate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::close;

    #[test]
    fn width_converts_milli_units_and_survives_threshold() {
        // 25（配置单位）× 0.001 = 0.025，过阈保留。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width: 25.0,
            ..Default::default()
        });
        close(g.outline_width, 0.025);
        // 换算后仍在阈上：2 × 0.001 = 0.002，不被清零。
        // 若源根本不做近零判定，这一臂不会红——上一臂负责抓「清零过头」。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width: 2.0,
            ..Default::default()
        });
        close(g.outline_width, 0.002);
    }

    #[test]
    fn width_below_threshold_after_conversion_writes_zero() {
        // 0.5 × 0.001 = 0.0005 < 0.001：写 0。阈在换算后的值上，
        // 所以 0.5 这个原始值虽然远大于 0.001，也照样清零。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width: 0.5,
            ..Default::default()
        });
        assert_eq!(g.outline_width, 0.0);
    }

    #[test]
    fn threshold_boundary_keeps_the_value() {
        // 1.0 × 0.001 = 0.001 恰在阈上：源判定是 <=，保留。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width: 1.0,
            ..Default::default()
        });
        assert_eq!(g.outline_width, 0.001);
        // 直写的 rate 同边界：0.001 保留。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width_min_rate: 0.001,
            ..Default::default()
        });
        assert_eq!(g.outline_width_min_rate, 0.001);
    }

    #[test]
    fn depth_offset_keeps_sign() {
        // -3 × 0.001 = -0.003：负值过阈后带符号保留。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_depth_offset: -3.0,
            ..Default::default()
        });
        close(g.outline_depth_offset, -0.003);
        // -0.5 × 0.001：清零（写 0，不是写 -0.0005）。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_depth_offset: -0.5,
            ..Default::default()
        });
        assert_eq!(g.outline_depth_offset, 0.0);
    }

    #[test]
    fn rates_are_written_without_conversion() {
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width_min_rate: 0.25,
            outline_width_max_rate: 0.6,
            ..Default::default()
        });
        close(g.outline_width_min_rate, 0.25);
        close(g.outline_width_max_rate, 0.6);
        // 直写但仍有近零阈：0.0004 → 0。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width_min_rate: 0.0004,
            ..Default::default()
        });
        assert_eq!(g.outline_width_min_rate, 0.0);
    }

    #[test]
    fn color_components_are_thresholded_independently() {
        let g = outline_globals(&GlobalFixtureSettings {
            outline_color: [0.0005, 0.6, -0.0004, 1.0],
            ..Default::default()
        });
        // 近零分量各自写 0，其余分量原样通过，负近零分量同样清零。
        assert_eq!(g.outline_color, [0.0, 0.6, 0.0, 1.0]);
    }

    #[test]
    fn zero_config_writes_all_zeros_not_stale_values() {
        // 提取侧现状是五参全 0：全部落进近零阈，一次下发写出五个 0。
        // 「写 0」与「不写」在这条路径上必须可区分——这是本族律的
        // 红线：安全写入族恒写，低于阈写的是 0，全局量不会残留。
        let g = outline_globals(&GlobalFixtureSettings {
            outline_width: 0.0,
            outline_depth_offset: 0.0,
            outline_width_min_rate: 0.0,
            outline_width_max_rate: 0.0,
            outline_color: [0.0; 4],
        });
        assert_eq!(g, FixtureOutlineGlobals::default());
    }
}
