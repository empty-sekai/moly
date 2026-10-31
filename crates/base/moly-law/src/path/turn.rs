//! Turn rule: angle-to-segment selection and turn duration when the target
//! heading changes.
//!
//! Changing the target heading involves two parallel things: choosing a turn
//! animation segment and rotating the heading over a span of time. This
//! module recomputes the **pure-function part** of both:
//!
//! * **Segment angle**: the caller converts the current heading and the
//!   target heading each into an Euler y (the engine's quaternion→Euler
//!   reading passes through a wrapper that folds negative angles into
//!   [0,360), and [`make_positive_yaw`] recomputes that wrapper's y
//!   component); the segment angle is the absolute value of their difference,
//!   in [0,360). [`turn_motion`] maps it through the six-band chain to a turn
//!   segment base name — three right-turn bands and three left-turn bands, the
//!   six literals of the segment name constants `NPC_ROTATE_MOTION_NAME_*`
//!   pinned one by one (always the `mov_cw_normal` family: the constant
//!   literals are so, and do not switch with the character's own displacement
//!   segment family).
//! * **Turn duration**: the angle divided by 60 — `MysekaiUtility.GetRotateTime`
//!   computes `|angle between the two vectors| / 60` (seconds). The input is
//!   the unsigned angle between the current heading and the target direction,
//!   and [`angle_between`] recomputes it with the same formula as the engine's
//!   `Vector3.Angle` (dot product divided by the magnitudes, clamped to
//!   [-1,1], acos, converted to degrees — without the clamp floating-point
//!   error can feed acos a NaN). This angle is unsigned.
//!
//! The two angles **are not the same quantity**: the segment angle is the
//! difference of the Euler y values (up to 360; it decides left versus right),
//! while the duration angle is the angle between the two directions (at most
//! 180, i.e. the shortest angular distance actually travelled). A turn with a
//! gap of 350° picks the left 45° segment yet takes only 10°/60 seconds — each
//! quantity has its own job, and merging them would be an invention.
//!
//! # The heading interpolation itself is not part of the rule
//!
//! The turn runs through the animation tween library (`DOLocalRotateAsync`):
//! shortest angular path, duration equal to [`rotate_time`] above, default
//! easing. Sampling a tween and manipulating quaternions are engine-side
//! mechanisms; the wiring happens on the consuming side (the npc domain), and
//! the rule only supplies its two deciding quantities.

/// 转身段：六段链的输出。序数（[`Self::index`]）只作动画域建表用，
/// 不承载语义。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnMotion {
    /// 差角 ∈ \[0,60\]：右转 45° 段。
    Turn45R,
    /// 差角 ∈ (60,120]：右转 90° 段。
    Turn90R,
    /// 差角 ∈ (120,180]：右转 180° 段。
    Turn180R,
    /// 差角 ∈ (180,240]：左转 180° 段。
    Turn180L,
    /// 差角 ∈ (240,300]：左转 90° 段。
    Turn90L,
    /// 差角 > 300 或为负：左转 45° 段。
    Turn45L,
}

impl TurnMotion {
    /// 全部六段：选段链的陪域，测试与建表共用。
    pub const ALL: [TurnMotion; 6] = [
        TurnMotion::Turn45R,
        TurnMotion::Turn90R,
        TurnMotion::Turn180R,
        TurnMotion::Turn180L,
        TurnMotion::Turn90L,
        TurnMotion::Turn45L,
    ];

    /// Segment base name: the literal of the game's segment constant. The
    /// animation domain appends a segment suffix (`_S` start / `_L` loop) to
    /// the base name to form the clip name in the shared action library.
    pub fn base_name(self) -> &'static str {
        match self {
            TurnMotion::Turn45R => "mov_cw_normal_turn45_r",
            TurnMotion::Turn90R => "mov_cw_normal_turn90_r",
            TurnMotion::Turn180R => "mov_cw_normal_turn180_r",
            TurnMotion::Turn180L => "mov_cw_normal_turn180_l",
            TurnMotion::Turn90L => "mov_cw_normal_turn90_l",
            TurnMotion::Turn45L => "mov_cw_normal_turn45_l",
        }
    }

    /// 日志短名：与常量名去掉族前缀的形状一致（`turn90_l` 一类）。
    pub fn label(self) -> &'static str {
        match self {
            TurnMotion::Turn45R => "turn45_r",
            TurnMotion::Turn90R => "turn90_r",
            TurnMotion::Turn180R => "turn180_r",
            TurnMotion::Turn180L => "turn180_l",
            TurnMotion::Turn90L => "turn90_l",
            TurnMotion::Turn45L => "turn45_l",
        }
    }

    /// 序数：动画域按下标建段节点表用，`[TurnMotion::ALL]` 的下标一致。
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Turn rate (degrees/second): the divisor of the duration formula, literal
/// 60.0.
pub const ROTATE_DEGREES_PER_SECOND: f32 = 60.0;

/// 引擎四元数→欧拉包装的 y 分量：负过 `-0.0001` 弧度折角加 360，
/// 正过 `360 + 折角` 减 360——折角 = `-0.0001 * 57.29578`，引擎字面量。
/// 作用域是 (−180,180] 的欧拉读数：折完落在 [0,360)。
pub fn make_positive_yaw(deg: f32) -> f32 {
    const NEGATIVE_FLIP: f32 = -0.0001 * 57.29578;
    const POSITIVE_FLIP: f32 = 360.0 + NEGATIVE_FLIP;
    if deg < NEGATIVE_FLIP {
        deg + 360.0
    } else if deg > POSITIVE_FLIP {
        deg - 360.0
    } else {
        deg
    }
}

/// 朝向的水平航向角（度）：`atan2(x, z)`。引擎侧同一读数取自
/// 「面朝该方向的旋转」的欧拉 y，值与此式一致（正向 y 把前向 +Z
/// 转向 +X，两侧坐标系同式）。
pub fn heading_yaw(forward: [f32; 3]) -> f32 {
    forward[0].atan2(forward[2]).to_degrees()
}

/// 选段角：两侧各过 [`make_positive_yaw`] 后取差的绝对值。
/// 输出 [0,360)。
pub fn turn_angle(current_yaw_deg: f32, target_yaw_deg: f32) -> f32 {
    (make_positive_yaw(target_yaw_deg) - make_positive_yaw(current_yaw_deg)).abs()
}

/// Six-band segment selection. The comparison chain reads: the first band
/// `[0,60]` includes both bounds; every other band is open below and closed
/// above (each guard excludes the lower bound and below, or above the upper
/// bound, which reads as open below / closed above); angles greater than 300
/// and **negative angles** both land in the left 45° band — negative angles
/// take the left arm of the inner band;
/// NaN: every guard of the compiled chain branches on an unordered compare
/// (`b.lt` / `b.hi` / `b.le` all take it), so NaN runs to the last band,
/// whose select sees the equal flag set and gives the left 45° clip.
pub fn turn_motion(angle: f32) -> TurnMotion {
    if angle.is_nan() {
        return TurnMotion::Turn45L;
    }
    if angle < 0.0 || 60.0 < angle {
        if angle <= 60.0 || 120.0 < angle {
            if angle <= 120.0 || 180.0 < angle {
                if angle <= 180.0 || 240.0 < angle {
                    // Inner band (angle > 240 or negative): the selection
                    // recomposes three booleans; b3 is always false (NaN
                    // never reaches here). Read literally:
                    // (240,300] is left 90; negative angles and >300 are
                    // left 45.
                    let over_240 = if angle <= 300.0 { angle < 240.0 } else { false };
                    let at_240 = if angle <= 300.0 { angle == 240.0 } else { true };
                    if !at_240 && !over_240 {
                        TurnMotion::Turn90L
                    } else {
                        TurnMotion::Turn45L
                    }
                } else {
                    TurnMotion::Turn180L
                }
            } else {
                TurnMotion::Turn180R
            }
        } else {
            TurnMotion::Turn90R
        }
    } else {
        TurnMotion::Turn45R
    }
}

/// 转体时长（秒）：`|角| / 60`。输入是两方向的非带符号夹角。
pub fn rotate_time(angle_deg: f32) -> f32 {
    angle_deg.abs() / ROTATE_DEGREES_PER_SECOND
}

/// 两方向的非带符号夹角（度）：引擎 `Vector3.Angle` 同式——点积除以
/// 模长积、夹到 [-1,1]、acos、折度。模长积近零（任一向量近零）时回
/// 0，同引擎。夹持不是装饰：单位向量自乘的浮点残差会让点积出
/// `1.0000001`，不夹持的 acos 直接产 NaN。
pub fn angle_between(from: [f32; 3], to: [f32; 3]) -> f32 {
    let dot = from[0] * to[0] + from[1] * to[1] + from[2] * to[2];
    let from_mag = (from[0] * from[0] + from[1] * from[1] + from[2] * from[2]).sqrt();
    let to_mag = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
    let denom = from_mag * to_mag;
    if denom < 1e-15 {
        return 0.0;
    }
    let cos = (dot / denom).clamp(-1.0, 1.0);
    cos.acos().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 段基名：六常量字面量逐个钉住 ----------------------------------

    #[test]
    fn base_names_are_the_six_rotate_motion_constants() {
        // Check: the strings returned by segment selection are exactly these
        // six constants. The animation domain looks up clips by `base name +
        // segment suffix`; a single wrong letter in a base name breaks the
        // whole family.
        assert_eq!(TurnMotion::Turn45R.base_name(), "mov_cw_normal_turn45_r");
        assert_eq!(TurnMotion::Turn90R.base_name(), "mov_cw_normal_turn90_r");
        assert_eq!(TurnMotion::Turn180R.base_name(), "mov_cw_normal_turn180_r");
        assert_eq!(TurnMotion::Turn180L.base_name(), "mov_cw_normal_turn180_l");
        assert_eq!(TurnMotion::Turn90L.base_name(), "mov_cw_normal_turn90_l");
        assert_eq!(TurnMotion::Turn45L.base_name(), "mov_cw_normal_turn45_l");
    }

    #[test]
    fn labels_match_their_base_names() {
        // 锚：短名是基名去掉族前缀——日志行里的段名要能对回常量。
        for motion in TurnMotion::ALL {
            let base = motion.base_name();
            let expected = &base["mov_cw_normal_".len()..];
            assert_eq!(motion.label(), expected);
        }
    }

    // ---- 选段链：段内取值与两侧边界成对 --------------------------------

    #[test]
    fn band_interiors_select_their_motion() {
        // 锚：六段内部各取一值，手算对段。取值避开 1.0/0.0 型的退化位。
        let cases = [
            (30.0, TurnMotion::Turn45R),
            (90.0, TurnMotion::Turn90R),
            (150.0, TurnMotion::Turn180R),
            (210.0, TurnMotion::Turn180L),
            (270.0, TurnMotion::Turn90L),
            (330.0, TurnMotion::Turn45L),
        ];
        for (angle, expected) in cases {
            assert_eq!(turn_motion(angle), expected, "angle {angle}");
        }
    }

    #[test]
    fn upper_band_bounds_are_inclusive() {
        // Check: the guard chain reads as open below / closed above — the
        // upper bound itself stays in its band. Each upper bound is a separate
        // assertion, so a wrong band only fails one test.
        assert_eq!(turn_motion(0.0), TurnMotion::Turn45R);
        assert_eq!(turn_motion(60.0), TurnMotion::Turn45R);
        assert_eq!(turn_motion(120.0), TurnMotion::Turn90R);
        assert_eq!(turn_motion(180.0), TurnMotion::Turn180R);
        assert_eq!(turn_motion(240.0), TurnMotion::Turn180L);
        assert_eq!(turn_motion(300.0), TurnMotion::Turn90L);
        assert_eq!(turn_motion(360.0), TurnMotion::Turn45L);
    }

    #[test]
    fn bounds_plus_epsilon_select_the_next_band() {
        // 成对臂：上界加一个可见的最小量必须换段——等号方向错了这里红。
        // 1e-3 远大于 f32 在 60 附近的步长，可见且稳定。
        assert_eq!(turn_motion(60.0 + 1e-3), TurnMotion::Turn90R);
        assert_eq!(turn_motion(120.0 + 1e-3), TurnMotion::Turn180R);
        assert_eq!(turn_motion(180.0 + 1e-3), TurnMotion::Turn180L);
        assert_eq!(turn_motion(240.0 + 1e-3), TurnMotion::Turn90L);
        assert_eq!(turn_motion(300.0 + 1e-3), TurnMotion::Turn45L);
    }

    #[test]
    fn negative_angles_fall_to_the_left_45_band() {
        // Check: in the comparison chain a negative angle cannot reach any
        // right band (after the first guard lets it through it runs down to
        // the left arm of the inner band). This is not dead code — callers
        // always feed a non-negative segment angle, but the chain itself has a
        // well-defined output for negative angles, and the test pins it so the
        // branch is not "optimized" away.
        assert_eq!(turn_motion(-0.001), TurnMotion::Turn45L);
        assert_eq!(turn_motion(-90.0), TurnMotion::Turn45L);
        assert_eq!(turn_motion(-359.0), TurnMotion::Turn45L);
    }

    #[test]
    fn nan_falls_to_the_left_45_band() {
        // The compiled chain branches on unordered compares down to the
        // last band, which selects the left 45° clip.
        assert_eq!(turn_motion(f32::NAN), TurnMotion::Turn45L);
    }

    // ---- make_positive_yaw：引擎包装的手算锚 --------------------------

    #[test]
    fn make_positive_wraps_only_past_the_flip_thresholds() {
        // 锚：包装阈 = -0.0001 弧度折角。-0.005 高于阈不动，-1 加 360，
        // 360 本身高于正阈减 360 成 0。折角带内（-0.005）的字节不动。
        assert_eq!(make_positive_yaw(0.0), 0.0);
        assert_eq!(make_positive_yaw(-0.005), -0.005);
        assert_eq!(make_positive_yaw(-1.0), 359.0);
        assert_eq!(make_positive_yaw(-180.0), 180.0);
        assert_eq!(make_positive_yaw(180.0), 180.0);
        assert_eq!(make_positive_yaw(359.9), 359.9);
        assert_eq!(make_positive_yaw(360.0), 0.0);
    }

    // ---- heading_yaw ---------------------------------------------------

    #[test]
    fn heading_yaw_matches_atan2_in_degrees() {
        // 锚：+Z 前向读 0°、+X 读 90°、−Z 读 180°、−X 读 −90°——与
        // 引擎「面朝该方向的旋转」的欧拉 y 逐值同。
        assert!((heading_yaw([0.0, 0.0, 1.0]) - 0.0).abs() < 1e-5);
        assert!((heading_yaw([1.0, 0.0, 0.0]) - 90.0).abs() < 1e-5);
        assert!((heading_yaw([0.0, 0.0, -1.0]) - 180.0).abs() < 1e-5);
        assert!((heading_yaw([-1.0, 0.0, 0.0]) + 90.0).abs() < 1e-5);
    }

    // ---- turn_angle：选段角 ---------------------------------------------

    #[test]
    fn turn_angle_is_the_abs_of_the_made_positive_difference() {
        // 锚：手算四组。(−90, 90)：折角 270 与 90，差 180。
        // (−90, 91)：179。(0, 90) 与 (90, 0)：同为 90——绝对值无向。
        assert!((turn_angle(0.0, 90.0) - 90.0).abs() < 1e-5);
        assert!((turn_angle(90.0, 0.0) - 90.0).abs() < 1e-5);
        assert!((turn_angle(-90.0, 90.0) - 180.0).abs() < 1e-5);
        assert!((turn_angle(-90.0, 91.0) - 179.0).abs() < 1e-4);
        // 近 330 的差角：(10, −20) 折角后 10 与 340 相减——选段角要的
        // 是欧拉差的绝对值（330，左段即最短方向 −30°），最短路程 30°
        // 是时长角的事。
        assert!((turn_angle(10.0, -20.0) - 330.0).abs() < 1e-4);
    }

    #[test]
    fn turn_angle_spanning_zero_takes_the_long_arithmetic_path() {
        // 锚：跨零的 ±10 差角折角后是 340 不是 20——make-positive 的差
        // 不取跨零短路；短方向由选段带（>300 → 左）正确给出。
        assert!((turn_angle(-10.0, 10.0) - 340.0).abs() < 1e-5);
        assert!((turn_angle(10.0, -10.0) - 340.0).abs() < 1e-5);
    }

    // ---- rotate_time ----------------------------------------------------

    #[test]
    fn rotate_time_divides_the_angle_by_sixty() {
        // Check: the duration formula `|angle|/60`. 60° is exactly 1 second;
        // a negative angle is taken by absolute value — the formula applies
        // the absolute value to its input.
        assert_eq!(rotate_time(0.0), 0.0);
        assert_eq!(rotate_time(60.0), 1.0);
        assert_eq!(rotate_time(90.0), 1.5);
        assert_eq!(rotate_time(180.0), 3.0);
        assert_eq!(rotate_time(7.5), 0.125);
        assert_eq!(rotate_time(-60.0), 1.0);
    }

    #[test]
    fn rotate_time_is_proportional_to_the_angle() {
        // 成对臂：时长随角线性——角翻倍时长翻倍，且绝对锚 90°=1.5s。
        let half = rotate_time(45.0);
        let full = rotate_time(90.0);
        assert!((half - 0.75).abs() < 1e-5);
        assert!((full - 2.0 * half).abs() < 1e-5);
    }

    // ---- angle_between --------------------------------------------------

    #[test]
    fn angle_between_axes_is_quarter_and_half_turns() {
        // 锚：手算正交/反向组。+Z 与 +X 夹 90；+Z 与 −Z 反向夹 180；
        // 向上 90。
        assert!((angle_between([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]) - 0.0).abs() < 1e-5);
        assert!((angle_between([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]) - 90.0).abs() < 1e-5);
        assert!((angle_between([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]) - 180.0).abs() < 1e-4);
        assert!((angle_between([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]) - 90.0).abs() < 1e-5);
    }

    #[test]
    fn angle_between_is_symmetric_and_sign_blind() {
        // 成对臂：非带符号——交换两臂同值、镜像同值。带符号角在这里
        // 必须不可见。
        let a = angle_between([1.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
        let b = angle_between([0.0, 0.0, 1.0], [1.0, 0.0, 1.0]);
        assert!((a - 45.0).abs() < 1e-4, "diagonal should be 45°, got {a}");
        assert!((a - b).abs() < 1e-5, "angle must be symmetric");
        let mirrored = angle_between([-1.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
        assert!((mirrored - 45.0).abs() < 1e-4);
    }

    #[test]
    fn angle_between_clamps_away_from_nan() {
        // 锚：夹持臂。单位向量乘出 1+ε 的点积时，不夹持的 acos 产
        // NaN——这里构造一次残差（归一化后再自乘）必须仍得有限数。
        let v = [3.0, 4.0, 5.0];
        let mag = (9.0 + 16.0 + 25.0f32).sqrt();
        let unit = [v[0] / mag, v[1] / mag, v[2] / mag];
        let angle = angle_between(unit, unit);
        assert!(angle.is_finite(), "unclamped acos produced {angle}");
        assert!(angle < 1e-3, "self-angle should be ~0, got {angle}");
    }

    #[test]
    fn angle_between_zero_vector_returns_zero() {
        // 锚：模长积近零回 0，同引擎——零向量的夹角没有定义，回零是
        // 引擎自己的选择，照抄而不是发明。
        assert_eq!(angle_between([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]), 0.0);
        assert_eq!(angle_between([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]), 0.0);
    }

    // ---- 组合：选段角与时长角的分工 ------------------------------------

    #[test]
    fn a_350_degree_yaw_gap_selects_left_45_but_lasts_10_over_60_seconds() {
        // 锚：两个角不是同一个量——差 350° 的转身选左 45 段（左是
        // 最短方向），时长却按最短路程 10°/60 秒计。合并这两个量的
        // 实现会在这一红。5° 与 −5° 的朝向：折角后 5 与 355 相减 350。
        let current = heading_yaw([(5.0f32).to_radians().sin(), 0.0, (5.0f32).to_radians().cos()]);
        let target = heading_yaw([(-5.0f32).to_radians().sin(), 0.0, (-5.0f32).to_radians().cos()]);
        let angle = turn_angle(current, target);
        assert!((angle - 350.0).abs() < 1e-3);
        assert_eq!(turn_motion(angle), TurnMotion::Turn45L);
        // 最短路程：+5° 与 −5° 两方向夹角恰 10°。
        let heading = angle_between(
            [(5.0f32).to_radians().sin(), 0.0, (5.0f32).to_radians().cos()],
            [(-5.0f32).to_radians().sin(), 0.0, (-5.0f32).to_radians().cos()],
        );
        assert!((heading - 10.0).abs() < 1e-3);
        assert!((rotate_time(heading) - (10.0 / 60.0)).abs() < 1e-4);
    }

    #[test]
    fn a_quarter_turn_right_selects_90_r_and_costs_one_and_a_half_seconds() {
        // 正臂：最常见的 90° 右转——差角 90 落 (60,120] 右段、时长
        // 90/60 = 1.5 秒。选段与时长都要能对上日志行的两个数。
        let current = heading_yaw([0.0, 0.0, 1.0]);
        let target = heading_yaw([1.0, 0.0, 0.0]);
        let angle = turn_angle(current, target);
        assert!((angle - 90.0).abs() < 1e-5);
        assert_eq!(turn_motion(angle), TurnMotion::Turn90R);
        let heading = angle_between([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        assert_eq!(rotate_time(heading), 1.5);
    }
}
