//! BezierParam 求值——`BezierParam.Evaluate` / `CurveParam.Setup`
//! 的逐字转录（反编译已双读核对）。
//!
//! 反编译形（BezierParam.c / CurveParam.c）：
//! * `get_EndValue`：`useEndValue` 假 → 返回 `startValue`（end 槽
//!   被 start 顶替），求值退化为常值 start；
//! * `get_UseCurve`：`useEndValue && useCurveValue`；
//! * `Evaluate(t)`：`t` 先夹到 `[0,1]`；曲线臂（`useCurve` 且
//!   `curveValue != 0`）取二次贝塞尔，控制点 =
//!   `end + (start-end)·clamp01(curveValue·0.5 + 0.5)`（Setup 逐字）；
//!   否则 `start + t·(end' - start)` 线性。
//!
//! 数据面全部 31 份 rig 的深度值 `depth ∈ [0,1]`，夹取是防御臂。

/// rig `cloth.components[*].clothParams.<name>` 的贝塞尔参数块。
#[derive(Debug, Clone, PartialEq)]
pub struct BezierParam {
    pub start_value: f32,
    pub end_value: f32,
    pub use_end_value: bool,
    pub curve_value: f32,
    pub use_curve_value: bool,
}

impl BezierParam {
    /// 常值参数（`useEndValue=false` 的形）。
    pub fn constant(value: f32) -> Self {
        Self {
            start_value: value,
            end_value: 0.0,
            use_end_value: false,
            curve_value: 0.0,
            use_curve_value: false,
        }
    }

    /// `CurveParam.Setup` 的控制点槽：`end + (start-end)·k`，
    /// `k = clamp01(curveValue·0.5 + 0.5)`。
    fn control(&self) -> f32 {
        let end = self.effective_end();
        let k = (self.curve_value * 0.5 + 0.5).clamp(0.0, 1.0);
        end + (self.start_value - end) * k
    }

    /// `get_EndValue`：门关时 end 槽读作 start。
    fn effective_end(&self) -> f32 {
        if self.use_end_value {
            self.end_value
        } else {
            self.start_value
        }
    }

    /// 曲线臂门：`useEndValue && useCurveValue && curveValue != 0`。
    fn use_curve(&self) -> bool {
        self.use_end_value && self.use_curve_value && self.curve_value != 0.0
    }

    /// `BezierParam.Evaluate(depth)`。
    pub fn evaluate(&self, depth: f32) -> f32 {
        let t = depth.clamp(0.0, 1.0);
        let start = self.start_value;
        let end = self.effective_end();
        if self.use_curve() {
            let c = self.control();
            let s = 1.0 - t;
            s * s * start + 2.0 * s * t * c + t * t * end
        } else {
            start + t * (end - start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常值形：`useEndValue=false` 时 end 槽被 start 顶替（get_EndValue）。
    #[test]
    fn constant_form_ignores_end() {
        let p = BezierParam {
            start_value: 0.03,
            end_value: 0.02,
            use_end_value: false,
            curve_value: 0.0,
            use_curve_value: false,
        };
        assert_eq!(p.evaluate(0.0), 0.03);
        assert_eq!(p.evaluate(0.5), 0.03);
        assert_eq!(p.evaluate(1.0), 0.03);
        // sd_101 chain0 structDistanceStiffness 的原样：1.0 常值
        let stiff = BezierParam::constant(1.0);
        assert_eq!(stiff.evaluate(0.7), 1.0);
    }

    /// 线性臂：drag 0.03→0.02（useEndValue=1）。
    #[test]
    fn linear_interpolation() {
        let p = BezierParam {
            start_value: 0.03,
            end_value: 0.02,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        assert_eq!(p.evaluate(0.0), 0.03);
        assert!((p.evaluate(0.5) - 0.025).abs() < 1e-8);
        assert_eq!(p.evaluate(1.0), 0.02);
        assert!((p.evaluate(0.25) - 0.0275).abs() < 1e-8);
        // 深度夹取：越界深度钳回两端（Evaluate 的 clamp01）
        assert_eq!(p.evaluate(-1.0), 0.03);
        assert_eq!(p.evaluate(2.0), 0.02);
    }

    /// 曲线臂：start=0 end=1 cv=0.5 → 控制点 = 1 + (0-1)·(0.25+0.5)
    /// = 0.25；t=0.5 → 0.25·0 + 0.5·0.25 + 0.25·1 = 0.375；
    /// t=0.25 → 2·0.25·0.75·0.25 + 0.0625 = 0.15625。手算精确。
    /// 注意 cv=1 时 k=1、控制点=start（退化成 t² 曲线）。
    #[test]
    fn curve_arm_quadratic_bezier() {
        let p = BezierParam {
            start_value: 0.0,
            end_value: 1.0,
            use_end_value: true,
            curve_value: 0.5,
            use_curve_value: true,
        };
        assert_eq!(p.evaluate(0.0), 0.0);
        assert_eq!(p.evaluate(1.0), 1.0);
        assert_eq!(p.evaluate(0.5), 0.375);
        assert!((p.evaluate(0.25) - 0.15625).abs() < 1e-9);
    }

    /// 曲线臂的门：`useCurveValue=1` 但 `curveValue=0` → 落回线性
    /// （BezierParam.Evaluate 的 `curveValue != 0.0` 判）。
    #[test]
    fn zero_curve_value_falls_back_to_linear() {
        let p = BezierParam {
            start_value: 0.0,
            end_value: 1.0,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: true,
        };
        assert_eq!(p.evaluate(0.5), 0.5);
        // useCurveValue=0 同样线性
        let q = BezierParam {
            use_curve_value: false,
            curve_value: 1.0,
            ..p.clone()
        };
        assert_eq!(q.evaluate(0.5), 0.5);
    }

    /// sd_101 chain0 depthInfluence 原样：0.1→1.0、cv=0.5 →
    /// 控制点 = 1.0 + (0.1-1.0)·(0.25+0.5) = 0.325；
    /// t=0.5 → 0.25·0.1 + 0.5·0.325 + 0.25·1.0 = 0.4375。
    #[test]
    fn depth_influence_from_sd101() {
        let p = BezierParam {
            start_value: 0.1,
            end_value: 1.0,
            use_end_value: true,
            curve_value: 0.5,
            use_curve_value: true,
        };
        let v = p.evaluate(0.5);
        assert!((v - 0.4375).abs() < 1e-6, "got {v}");
    }
}

/// demo 对照臂（node 直挂 game/vendor/viewer/cloth.js 的 evalBezier
/// 对同参数逐值求值，2026-09-07 实跑抄数）——sd_101 chain0 三参数：
/// * drag 0.03→0.02：depth 0.25/0.5/0.75 → 0.027499999/0.024999999/
///   0.022499999（f32 与 JS double 的表示差 < 1e-9）；
/// * depthInfluence 0.1→1.0 cv=0.5 → 0.240625001/0.437500001/
///   0.690625001；
/// * clampRotationAngle 3°→20° → 7.25/11.5/15.75。
#[cfg(test)]
mod demo_arm {
    use super::*;

    #[test]
    fn matches_demo_eval_bezier_on_sd101_params() {
        let drag = BezierParam {
            start_value: 0.029_999_999,
            end_value: 0.019_999_996,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        for (d, want) in [
            (0.25f32, 0.027_499_999),
            (0.5, 0.024_999_999),
            (0.75, 0.022_499_999),
        ] {
            assert!((drag.evaluate(d) - want).abs() < 1e-8);
        }
        let depth_inf = BezierParam {
            start_value: 0.1,
            end_value: 1.0,
            use_end_value: true,
            curve_value: 0.5,
            use_curve_value: true,
        };
        for (d, want) in [(0.25f32, 0.240_625), (0.5, 0.437_5), (0.75, 0.690_625)] {
            assert!((depth_inf.evaluate(d) - want).abs() < 1e-6, "d={d}");
        }
        let clamp_ang = BezierParam {
            start_value: 3.0,
            end_value: 20.0,
            use_end_value: true,
            curve_value: 0.0,
            use_curve_value: false,
        };
        for (d, want) in [(0.25f32, 7.25), (0.5, 11.5), (0.75, 15.75)] {
            assert!((clamp_ang.evaluate(d) - want).abs() < 1e-6);
        }
    }
}
