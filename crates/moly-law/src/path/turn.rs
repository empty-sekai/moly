//! 转身律：换目标朝向时的角度选段与转体时长。
//!
//! 真源换目标时的转身是两件并行的事：选一个转身动画段、把朝向在一段
//! 时间内转过去。本模块复算的是这两件事的**纯函数部分**：
//!
//! * **选段角**：调用方把当前朝向与目标朝向各折成欧拉 y（引擎的
//!   四元数→欧拉读数会过一个「把负角折进 [0,360)」的包装，
//!   [`make_positive_yaw`] 复算该包装的 y 分量），选段角 = 两者之差的
//!   绝对值，落在 [0,360)。[`turn_motion`] 按六段链把它映成转身段基名
//!   ——右转三段、左转三段，段名常量 `NPC_ROTATE_MOTION_NAME_*` 六个
//!   字面量逐个钉住（恒 `mov_cw_normal` 族：常量字面量如此，不随角色
//!   自身的位移段族换）。
//! * **转体时长**：角度除以 60——真源 `MysekaiUtility.GetRotateTime` 的
//!   式子是 `|两向量夹角| / 60`（秒）。输入是当前朝向与目标方向的非带
//!   符号夹角，[`angle_between`] 按引擎 `Vector3.Angle` 同式复算（点积
//!   除模长、夹到 [-1,1]、acos、折度——不含夹持时浮点误差会把 acos 喂
//!   出 NaN）。
//!
//! 两个角**不是同一个量**：选段角是欧拉 y 的差（可到 360，左/右由它
//! 分段），时长角是两方向的夹角（至多 180，即实际要走的最短角路程）。
//! 差 350° 的转身选 45° 左段、却只花 10°/60 秒——两个量各司其职，
//! 合并成一个是编造。
//!
//! # 朝向的插值本身不在律里
//!
//! 真源的转体走动画补间库（`DOLocalRotateAsync`）：最短角路径、时长
//! 即上面的 [`rotate_time`]、缺省缓动。补间的采样与四元数操作是引擎侧
//! 机制，接线在消费侧（npc 域）做，律只供它的两个决定量。

/// 转身段：六段链的输出。序数（[`Self::index`]）只作动画域建表用，
/// 不承载语义。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnMotion {
    /// 差角 ∈ [0,60]：右转 45° 段。
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

    /// 段基名：真源选段常量的字面量。动画域在基名上拼段后缀
    /// （`_S` 起始 / `_L` 循环）成共享动作库里的剪辑名。
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

/// 转体角速度（度/秒）：时长式子的除数，真源字面量 60.0。
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

/// 六段链选段。真源比较链直译：首段 `[0,60]` 上下都含等号；其余各段
/// 下开上闭（卫兵写的是「排除下界及以下、或上界以上」，读法即下开上
/// 闭）；大于 300 与**负角**同落 45° 左段——负角走的是内段的左转臂；
/// NaN 过不了首卫兵，落 45° 右段。
pub fn turn_motion(angle: f32) -> TurnMotion {
    if angle < 0.0 || 60.0 < angle {
        if angle <= 60.0 || 120.0 < angle {
            if angle <= 120.0 || 180.0 < angle {
                if angle <= 180.0 || 240.0 < angle {
                    // 内段（角 > 240 或为负）：真源以三个布尔重组选择，
                    // b3 恒假（NaN 到不了这里），字面直译：
                    // (240,300] 左 90、负角与 >300 左 45。
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
        // 锚：真源选段返回的字符串就是这六个常量。动画域按 `基名 + 段
        // 后缀` 找剪辑，基名错一个字母整族断线。
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
        // 锚：真源卫兵链的读法是下开上闭——上界本身留在本段。每个上界
        // 是一条独立断言，错一段只红一条。
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
        // 锚：真源比较链里负角进不了任何右段（首卫兵放行后一路走到内
        // 段的左臂）。这不是死代码——调用方喂的选段角恒非负，但链条
        // 本身对负角有确定输出，钉住它防止「优化」掉这个分支。
        assert_eq!(turn_motion(-0.001), TurnMotion::Turn45L);
        assert_eq!(turn_motion(-90.0), TurnMotion::Turn45L);
        assert_eq!(turn_motion(-359.0), TurnMotion::Turn45L);
    }

    #[test]
    fn nan_falls_to_the_right_45_band() {
        // 锚：NaN 与任何比较都为假，首卫兵（`< 0 || > 60`）取假直接进
        // 右 45 段。钉住它：内段的 NaN 重组臂永远不可达。
        assert_eq!(turn_motion(f32::NAN), TurnMotion::Turn45R);
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
        // 锚：时长式 `|角|/60`。60° 恰 1 秒；负角取绝对值——真源式子
        // 外面套了绝对值。
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
