//! Shading rules for the site families: one file per family, following the
//! game's programs formula by formula, pure Rust with no engine dependency.
//! Each family has a value-by-value comparison (expected values computed
//! independently from sampled inputs) in the `#[cfg(test)]` section of its
//! file.

pub mod avatar;
pub mod character;
pub mod fieldobject;
pub mod ground;
pub mod tree;
pub mod water;

/// 站点族 Base 片元恒写 `SV_Target1 = vec4(0.0)`；只写单目标的深度/阴影 pass 不在此列。
pub const SECOND_COLOR_TARGET: [f32; 4] = [0.0; 4];

/// 一个屏幕位置上的源抖动阈值（表元 × 缩放 + 偏置），与 Avatar 一族共用
/// 同一张 4×4 bayer 表与同一对缩放偏置常数。屏幕坐标非负，恒走源的带符号
/// fract 的正支路。
#[must_use]
pub fn bayer_value_avatar(screen_position: [f32; 4], screen_params: [f32; 4]) -> f32 {
    // 表与常数照 Avatar 程序的逐字面值；与站点族/Character 同表同向。
    const BAYER: [[f32; 4]; 4] = [
        [0.0, 12.0, 3.0, 15.0],
        [8.0, 4.0, 11.0, 7.0],
        [2.0, 14.0, 1.0, 13.0],
        [10.0, 6.0, 9.0, 5.0],
    ];
    const DITHER_SCALE: f32 = 0.0618750006;
    const DITHER_BIAS: f32 = 0.00999999978;
    let qx = screen_position[0] / screen_position[3] * screen_params[0] * 0.25;
    let qy = screen_position[1] / screen_position[3] * screen_params[1] * 0.25;
    let ix = (qx.abs().fract() * 4.0) as usize;
    let iy = (qy.abs().fract() * 4.0) as usize;
    BAYER[iy.min(3)][ix.min(3)] * DITHER_SCALE + DITHER_BIAS
}

/// 宝藏阴影的三个常数，站点各族片元里字面出现、值相同：
/// 径向边缘、衰减范围倒数、强度缩放。
pub const TREASURE_SHADOW_EDGE: f32 = 0.959999979;
pub const TREASURE_SHADOW_INV_RANGE: f32 = 24.9999866;
pub const TREASURE_SHADOW_SCALE: f32 = 0.5;

#[cfg(test)]
pub(crate) fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

/// Bit-level window for the value-by-value comparison: the expected bits come
/// from an independent evaluation of the program; platform libm's
/// exp2/powf implementations, and optimizer rewrites between them, can shift
/// a result by 1 ULP, while a formula-level error shifts it far more than 1
/// ULP. Only valid for normal numbers (bit patterns are monotone in value).
#[cfg(test)]
pub(crate) fn within_one_ulp(a: f32, b: f32) -> bool {
    a.to_bits().abs_diff(b.to_bits()) <= 1
}
