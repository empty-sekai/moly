//! Easing: how a progress in `[0, 1]` maps onto the share of a change.
//!
//! A document names an ease as a string — `linear`, `step`, `smooth`, or a
//! family with its direction, such as `out_quad` or `in_out_sine` — or as a
//! cubic Bézier `{ "bezier": [x1, y1, x2, y2] }` whose end points are
//! `(0, 0)` and `(1, 1)`, as in CSS.

use serde::{Deserialize, Deserializer};

/// The shape of a change over its progress.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ease {
    /// Progress unchanged.
    #[default]
    Linear,
    /// Nothing until the end, then the whole change.
    Step,
    /// Smoothstep: slow at both ends.
    Smooth,
    In(Family),
    Out(Family),
    InOut(Family),
    /// A cubic Bézier through `(0, 0)`, `(x1, y1)`, `(x2, y2)`, `(1, 1)`.
    Bezier([f32; 4]),
}

/// The curve family of an `in` / `out` / `in_out` ease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Sine,
    Quad,
    Cubic,
    Quart,
    Quint,
    Expo,
    Circ,
    Back,
}

impl Family {
    const ALL: [(&'static str, Family); 8] = [
        ("sine", Family::Sine),
        ("quad", Family::Quad),
        ("cubic", Family::Cubic),
        ("quart", Family::Quart),
        ("quint", Family::Quint),
        ("expo", Family::Expo),
        ("circ", Family::Circ),
        ("back", Family::Back),
    ];

    /// The ease-in curve of the family: 0 at 0, 1 at 1.
    fn ease_in(self, t: f32) -> f32 {
        use std::f32::consts::FRAC_PI_2;
        match self {
            Family::Sine => 1.0 - (t * FRAC_PI_2).cos(),
            Family::Quad => t * t,
            Family::Cubic => t * t * t,
            Family::Quart => t * t * t * t,
            Family::Quint => t * t * t * t * t,
            Family::Expo => {
                if t <= 0.0 {
                    0.0
                } else {
                    (2.0f32).powf(10.0 * (t - 1.0))
                }
            }
            Family::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            Family::Back => {
                const S: f32 = 1.70158;
                t * t * ((S + 1.0) * t - S)
            }
        }
    }
}

impl Ease {
    /// The share of the change at progress `t`; `t` is clamped to `[0, 1]`.
    pub fn apply(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Ease::Linear => t,
            Ease::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Ease::Smooth => t * t * (3.0 - 2.0 * t),
            Ease::In(family) => family.ease_in(t),
            Ease::Out(family) => 1.0 - family.ease_in(1.0 - t),
            Ease::InOut(family) => {
                if t < 0.5 {
                    0.5 * family.ease_in(2.0 * t)
                } else {
                    1.0 - 0.5 * family.ease_in(2.0 - 2.0 * t)
                }
            }
            Ease::Bezier([x1, y1, x2, y2]) => bezier_y_at_x(x1, y1, x2, y2, t),
        }
    }

    fn from_name(name: &str) -> Option<Ease> {
        match name {
            "linear" => return Some(Ease::Linear),
            "step" => return Some(Ease::Step),
            "smooth" => return Some(Ease::Smooth),
            _ => {}
        }
        let (make, family): (fn(Family) -> Ease, &str) = if let Some(rest) = name.strip_prefix("in_out_") {
            (Ease::InOut, rest)
        } else if let Some(rest) = name.strip_prefix("in_") {
            (Ease::In, rest)
        } else {
            (Ease::Out, name.strip_prefix("out_")?)
        };
        Family::ALL.iter().find(|(key, _)| *key == family).map(|(_, family)| make(*family))
    }

    /// The names a document may use, for refusals.
    pub fn names() -> String {
        let families: Vec<&str> = Family::ALL.iter().map(|(name, _)| *name).collect();
        format!(
            "linear, step, smooth, {{in_,out_,in_out_}}{{{}}}, or {{\"bezier\": [x1, y1, x2, y2]}}",
            families.join(",")
        )
    }
}

/// The y of a CSS cubic Bézier at x: the curve's parameter is found by
/// Newton steps, falling back to bisection where the slope is too flat.
fn bezier_y_at_x(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let coordinate = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let slope = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    let mut s = x;
    for _ in 0..8 {
        let error = coordinate(x1, x2, s) - x;
        if error.abs() < 1e-6 {
            return coordinate(y1, y2, s);
        }
        let d = slope(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - error / d).clamp(0.0, 1.0);
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    s = x;
    for _ in 0..32 {
        let value = coordinate(x1, x2, s);
        if (value - x).abs() < 1e-6 {
            break;
        }
        if value < x {
            low = s;
        } else {
            high = s;
        }
        s = 0.5 * (low + high);
    }
    coordinate(y1, y2, s)
}

impl<'de> Deserialize<'de> for Ease {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Name(String),
            Bezier {
                bezier: [f32; 4],
            },
        }
        match Repr::deserialize(deserializer)? {
            Repr::Name(name) => Ease::from_name(&name)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown ease {name:?}; the eases are {}", Ease::names()))),
            Repr::Bezier { bezier } => {
                if bezier.iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&bezier[0]) && (0.0..=1.0).contains(&bezier[2]) {
                    Ok(Ease::Bezier(bezier))
                } else {
                    Err(serde::de::Error::custom("a bezier ease needs finite points with x1 and x2 in 0..=1"))
                }
            }
        }
    }
}
