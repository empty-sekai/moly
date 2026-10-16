//! The engine math helpers the UI code calls, with their .NET semantics.
//!
//! `Mathf.Log(f, p)` is `(float)Math.Log(f, p)` and `Math.Log(a, b)` is
//! `Log(a) / Log(b)` after its NaN/degenerate-base guards; `Mathf.Pow` is
//! `(float)Math.Pow`; `Mathf.Lerp(a, b, t)` is `a + (b - a) * Clamp01(t)` in
//! float; `Mathf.Round` and `Mathf.RoundToInt` go through `Math.Round`, which
//! rounds half to even. The Color to Color32 conversion rounds
//! `Clamp01(channel) * 255` per channel with that same rounding.
//!
//! The transcendental calls use the host libm in double precision. Two correct
//! libm implementations can differ by one unit in the last place of the double
//! result, which disappears in the cast to float except at a rounding tie.

/// `Math.Log(a, newBase)` with its guard clauses.
fn dotnet_log(a: f64, new_base: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if new_base.is_nan() {
        return new_base;
    }
    if new_base == 1.0 {
        return f64::NAN;
    }
    if a != 1.0 && (new_base == 0.0 || new_base == f64::INFINITY) {
        return f64::NAN;
    }
    a.ln() / new_base.ln()
}

/// `Mathf.Log(float f, float p)`.
pub fn log(f: f32, p: f32) -> f32 {
    dotnet_log(f as f64, p as f64) as f32
}

/// `Mathf.Pow(float f, float p)`.
pub fn pow(f: f32, p: f32) -> f32 {
    (f as f64).powf(p as f64) as f32
}

/// `Mathf.Clamp01`: the two comparisons in the engine's order (NaN passes through).
pub fn clamp01(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

/// `Mathf.Clamp(value, min, max)`: min is tested first.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// `Mathf.Lerp(a, b, t)`.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * clamp01(t)
}

/// `Mathf.Round`: `Math.Round(double)` rounds half to even.
pub fn round(f: f32) -> f32 {
    (f as f64).round_ties_even() as f32
}

/// `Mathf.RoundToInt`.
pub fn round_to_int(f: f32) -> i32 {
    (f as f64).round_ties_even() as i32
}

/// `Mathf.Max(float, float)` returns `a > b ? a : b`.
pub fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `Mathf.Min(float, float)` returns `a < b ? a : b`.
pub fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `Mathf.Epsilon`: the smallest denormal unless the platform flushes
/// denormals to zero (arm64 players do not).
pub const MATHF_EPSILON: f32 = f32::from_bits(1);

/// `Mathf.Approximately(a, b)`.
pub fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < max(0.000001 * max(a.abs(), b.abs()), MATHF_EPSILON * 8.0)
}

/// Implicit `Color` to `Color32` conversion, one channel.
pub fn color_channel_to_byte(channel: f32) -> u8 {
    round(clamp01(channel) * 255.0) as u8
}

/// Implicit `Color` to `Color32` conversion (r, g, b, a).
pub fn color32(color: [f32; 4]) -> [u8; 4] {
    [
        color_channel_to_byte(color[0]),
        color_channel_to_byte(color[1]),
        color_channel_to_byte(color[2]),
        color_channel_to_byte(color[3]),
    ]
}
