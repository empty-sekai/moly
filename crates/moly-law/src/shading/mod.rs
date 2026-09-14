//! 站点族的着色律：每族一份文件，逐句翻译真源的程序，纯 Rust、无引擎依赖。
//! 每族带一份移植时的逐值比对（采样输入对源程序独立求值的期望值），在各文件的
//! `#[cfg(test)]` 里。

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

/// 逐值比对的位级窗：期望位来自源程序的独立求值；平台 libm 的
/// exp2/powf 实现之间、以及优化器对二者间的改写，会让结果挪 1 ULP，
/// 公式级错误挪动远超 1 ULP。只对正规格数成立（位表示随值单调）。
#[cfg(test)]
pub(crate) fn within_one_ulp(a: f32, b: f32) -> bool {
    a.to_bits().abs_diff(b.to_bits()) <= 1
}
