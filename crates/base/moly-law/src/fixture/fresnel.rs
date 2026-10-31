//! The two additive-color rules of the fixture Basic material
//! branch: fresnel (`fresnel_add`) and
//! reflection (`reflection_add`). The two are **independent, parallel switches**: a
//! material can enable only one, or both; when both are enabled the `1−N·V` is computed only once and each
//! branch takes a copy (one takes the clamped copy, one takes the unclamped copy, see `reflection_add`).
//!
//! The following describes the fresnel branch. The source shader's (the variant with the
//! material branch enabled) fragment tail is
//! "additive" rather than "multiplicative": fresnel term = exp2(log2(clamp(1−N·V, 0, 1))
//! × power) × color.rgb × color.a, added per channel to the
//! rgb after the treasure shadow and before fog. Three essential points, missing one gives a different formula:
//!
//! - **clamp(1−N·V, 0, 1) first, then take the logarithm** — when N·V slightly exceeds 1 (the normal and view
//!   are each normalized then dotted, entirely possible), without the clamp it would go log2(negative) = NaN;
//! - **color's alpha is a factor** — the term is further multiplied by color.a (bike1 real value
//!   0.15686…, reducing the magnitude to one sixth), not only by rgb;
//! - **the view direction has two branches**: a perspective camera normalizes "camera position − fragment position" (no
//!   epsilon clamp, unlike the normal branch); an orthographic camera takes the third row of the view matrix
//!   directly (column-major \[2\]/\[6\]/\[10\]), selected by whether unity_OrthoParams.w is non-zero.
//!
//! Expected bits are evaluated independently in float32 following the
//! source program's operation order (numpy, one operator at a time),
//! independently of this crate; asserts open a ±1 ULP window to absorb
//! exp2/log2 libm implementation differences, while a formula-level
//! error moves far more than 1 ULP.

/// 视线方向：`ortho_params_w` 非零走正交支（视图矩阵第三行，列主序
/// 下标 \[2\]/\[6\]/\[10\]），否则透视支（相机位减片元位后按
/// inversesqrt(dot) 归一化——与法线支不同，源在这里没有 epsilon
/// 钳制，零向量会照原样产生 NaN，不替源修）。
#[must_use]
pub fn view_direction(
    camera_position: [f32; 3],
    world_position: [f32; 3],
    view_matrix: [f32; 16],
    ortho_params_w: f32,
) -> [f32; 3] {
    if ortho_params_w != 0.0 {
        [view_matrix[2], view_matrix[6], view_matrix[10]]
    } else {
        let to_camera = [
            camera_position[0] - world_position[0],
            camera_position[1] - world_position[1],
            camera_position[2] - world_position[2],
        ];
        let dot = to_camera[0] * to_camera[0]
            + to_camera[1] * to_camera[1]
            + to_camera[2] * to_camera[2];
        let inv = 1.0 / dot.sqrt();
        [
            to_camera[0] * inv,
            to_camera[1] * inv,
            to_camera[2] * inv,
        ]
    }
}

/// 菲涅尔项逐通道加回 rgb：按源的运算顺序
/// `(-ndv+1) → clamp(0,1) → log2 → ×power → exp2 → ×color.rgb →
/// ×color.a → +rgb`。mediump 标记是 GLES 翻译产物，源 HLSL 是
/// float32，本律按 float32 求值。
#[must_use]
pub fn fresnel_add(
    rgb: [f32; 3],
    n_dot_v: f32,
    fresnel_power: f32,
    fresnel_color: [f32; 4],
) -> [f32; 3] {
    let one_minus = (-n_dot_v + 1.0).clamp(0.0, 1.0);
    let fresnel = (one_minus.log2() * fresnel_power).exp2();
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = (fresnel * fresnel_color[c]) * fresnel_color[3] + rgb[c];
    }
    out
}

/// The value sampled from an **unbound** cube-map sampler; constant per channel.
///
/// The engine prepares a built-in default texture for every texture
/// dimension; during drawing, when a sampler has no bound object, the entry of that table is indexed by **dimension**
/// (all three drawing-time paths — texture parameter preparation, fallback when the property table
/// has no entry, shared material data access — index by dimension;
/// none resolves by the default name in the declaration; that name is only read by editor-side tools).
/// The built-in texture in the cube-map slot clears **each of the six faces entirely to the same color**, so:
///
/// **The sampled value is independent of sample direction, mip level, mip bias and filter mode** — for a
/// texture with six faces of the same color, any direction, any mip, any interpolation weight returns that one color.
/// This is why the whole branch can be folded into a constant; it does not depend on the texture's size or level count.
///
/// Construction: the cube-map slot of that table and the "all black cube map" go
/// through **the same constructor**, differing only in the fill argument
/// (0x80808080 vs 0), in both the editor binary and the shipped engine dynamic library,
/// with the same two fill values (the all-black entry serves as a positive control).
/// Value = 128/255 per channel. The fill color is fed to the clear call as per-channel 8-bit values
/// ⇒ R=G=B=A=128. The project runs in gamma space (the color-space
/// setting selects the gamma option) ⇒ the engine's default texture uses
/// the 8-bit normalized format without an sRGB flag ⇒ **no sRGB decode
/// at sample time, 128/255 is not reduced**.
///
/// The literal is written out rather than as `128.0 / 255.0`: that division
/// takes a different precision path in constant evaluation of the two languages
/// (single rounding on one side, high precision then back down — two roundings — on the other);
/// as a decimal literal both sides parse the same bits. This decimal is the
/// shortest round-trip form of that f32; do not "simplify" it back to the division.
pub const UNBOUND_CUBE_SAMPLE: f32 = 0.5019608;

/// Adds the material reflection term to rgb per channel, **in the form folded for the unbound cube map**.
///
/// Operation order: `d = N·V` → `1−d` → `log2` → `×power` → `exp2` → `min(·, 1)` →
/// `×cube sample` → `×intensity` → `+rgb`. Four essential points:
///
/// - **`log2`'s input has no lower clamp** — unlike the adjacent fresnel branch. Both branches
///   **share the same `1−d`** (in the variant where both are on, that subtraction is computed once): the fresnel branch
///   takes the `clamp(·,0,1)` copy, the reflection branch takes the **unclamped original**.
///   Clamping once for each branch, or letting the reflection branch take the clamped value, gives a different function —
///   when `N·V` is negative (normal facing away from the view), the unclamped `1−d` exceeds 1, which is exactly why the
///   upper clamp below exists.
/// - **only the upper clamp `min(·, 1)`** exists; it is written as an
///   explicit comparison to keep the `min` semantics of the language
///   the formula is written for (1.0 is taken only when
///   `1.0 < fresnel`) ⇒ when `N·V` slightly exceeds 1 due to floating
///   point error, `log2` receives a negative number, the result is NaN,
///   and that NaN **passes straight through**. That is the shape of the
///   source, and no lower clamp is added.
/// - **the multiplication order is `(sample × fresnel) × intensity`**: two multiplications, the sample constant
///   multiplies first. Pre-multiplying the two constants into one is an algebraic reassociation, and f32 multiplication
///   is not associative — the fold only replaces the "fetch that texel" step; it does not change the operation order.
/// - **additive, not multiplicative**, and alpha does not participate and is not rewritten.
///
/// When `N·V` is exactly 1 (facing the view), `log2(0)` is IEEE negative infinity and `exp2` brings it back to 0
/// ⇒ the term is zero, consistent with the limit approaching the facing case.
///
/// ⚠ **Boundary of applicability: holds only for materials whose
/// cube-map slot is empty.** In the game this branch is on for two
/// families of materials: the egg family's slot is an **empty
/// reference** (re-checked: every main texture in the same texture
/// table is non-empty, so "empty" is not a reading error) ⇒ uses this
/// function; the vehicle family **binds a real cube map** (three
/// materials point at the same object, 128×128, with a complete
/// eight-level mip chain on disk) ⇒ **this function is wrong for
/// them**, they need a real sample, and there the mip bias will really
/// select a mip. None of the vehicle family is placed yet.
///
/// ⚠ **Do not "add a gray cube map" and sample it instead.** That would
/// expand a value already known to be constant into a texture read, adding a texture, a sampler and a
/// path that can drift, for no bit of accuracy — the sample of a
/// six-faces-same-color texture is direction-independent, which is
/// stated in the note on the constant above.
#[must_use]
pub fn reflection_add(
    rgb: [f32; 3],
    n_dot_v: f32,
    reflection_fresnel_power: f32,
    reflection_intensity: f32,
) -> [f32; 3] {
    let one_minus = -n_dot_v + 1.0;
    let fresnel = (one_minus.log2() * reflection_fresnel_power).exp2();
    let clamped = if 1.0 < fresnel { 1.0 } else { fresnel };
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = (UNBOUND_CUBE_SAMPLE * clamped) * reflection_intensity + rgb[c];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::within_one_ulp;

    /// bike1 材质的真实参数（资产 extras 里的 _FresnelPower / _FresnelColor）。
    const POWER: f32 = 3.0;
    const COLOR: [f32; 4] = [
        1.0,
        0.980_063_6,
        0.796_078_4,
        0.156_862_75,
    ];
    const RGB: [f32; 3] = [0.22, 0.47, 0.81];

    /// Expected bits: the source operation order evaluated independently
    /// in float32 (numpy, one operator at a time).
    /// Each row picks an N·V that pins down one element of the formula:
    /// - 1.0000001: without the clamp first, log2(negative)=NaN ⇒ the clamp order;
    /// - 0.0 and −0.70710678 at the same bits: the negative half saturates at the clamp ⇒ the clamp exists;
    /// - the absolute value at 0.0: term = color.rgb×color.a+rgb ⇒ the alpha factor and the power;
    /// - 0.5/0.25: the exponent value (power=2 or using x² moves outside the window).
    #[test]
    fn fresnel_add_matches_source_evaluated_bits() {
        let cases: [(f32, [u32; 3]); 7] = [
            (1.0, [0x3e61_47ae, 0x3ef0_a3d7, 0x3f4f_5c29]),
            (1.0000001, [0x3e61_47ae, 0x3ef0_a3d7, 0x3f4f_5c29]),
            (0.92387953, [0x3e61_59d1, 0x3ef0_acba, 0x3f4f_5fc5]),
            (0.70710678, [0x3e65_50e3, 0x3ef2_9e25, 0x3f50_29ca]),
            (0.5, [0x3e75_5bc2, 0x3efa_7aa4, 0x3f53_5b23]),
            (0.25, [0x3e92_85b9, 0x3f08_ec66, 0x3f5c_d8b5]),
            (0.0, [0x3ec0_f428, 0x3f1f_ad21, 0x3f6f_53f9]),
        ];
        for (n_dot_v, want_bits) in cases {
            let got = fresnel_add(RGB, n_dot_v, POWER, COLOR);
            for c in 0..3 {
                assert!(
                    within_one_ulp(got[c], f32::from_bits(want_bits[c])),
                    "ndv={n_dot_v} channel {c}: got {:08x}, expected {:08x}",
                    got[c].to_bits(),
                    want_bits[c]
                );
            }
        }
    }

    /// 负半轴 N·V 全部饱和在 clamp 上界——三种差异很大的输入同一位，
    /// 钉住「钳制发生在 log2 之前」；若先 pow 再钳，−1 的输入会给出
    /// exp2(3)=8 一类的爆炸值。
    #[test]
    fn negative_n_dot_v_saturates_at_the_same_bits() {
        for n_dot_v in [0.0, -0.70710678, -1.0, -3.0] {
            let got = fresnel_add(RGB, n_dot_v, POWER, COLOR);
            for c in 0..3 {
                assert_eq!(
                    got[c].to_bits(),
                    fresnel_add(RGB, 0.0, POWER, COLOR)[c].to_bits(),
                    "ndv={n_dot_v} channel {c} 未饱和于 clamp 上界"
                );
            }
        }
    }

    /// N·V=1（正对相机）时项恒为 0，输出逐位等于入参 rgb——加色而非
    /// 乘色的钉子：任何「乘以颜色」的写法在这里都会偏离。
    #[test]
    fn head_on_view_returns_rgb_unchanged() {
        let got = fresnel_add(RGB, 1.0, POWER, COLOR);
        for c in 0..3 {
            assert_eq!(got[c].to_bits(), RGB[c].to_bits());
        }
    }

    /// 透视支的期望位（cam=(3,5,10), pos=(1.5,−2,4) →
    /// (0.1605863, 0.7494028, 0.6423453)）。
    #[test]
    fn perspective_view_direction_matches_source_evaluated_bits() {
        let got = view_direction(
            [3.0, 5.0, 10.0],
            [1.5, -2.0, 4.0],
            [0.0; 16],
            0.0,
        );
        let want = [0x3e24_70be, 0x3f3f_d8dd, 0x3f24_70be];
        for c in 0..3 {
            assert!(
                within_one_ulp(got[c], f32::from_bits(want[c])),
                "channel {c}: got {:08x}, expected {:08x}",
                got[c].to_bits(),
                want[c]
            );
        }
    }

    /// 正交支取视图矩阵第三行（列主序 [2]/[6]/[10]）：喂一个 16 元素
    /// 互不相同的矩阵，读错任何下标（平移列 [3]/[7]/[11]、行主序）都
    /// 会给出别的值。
    #[test]
    fn ortho_view_direction_reads_the_third_row() {
        let matrix: [f32; 16] = core::array::from_fn(|i| (i as f32 + 1.0) / 64.0);
        let got = view_direction([9.0, 9.0, 9.0], [1.0, 2.0, 3.0], matrix, 1.0);
        let want = [matrix[2], matrix[6], matrix[10]];
        assert_eq!(got, want);
        // 选择器是「w 是否非零」：0.5 也走正交支，恰 0 才走透视支。
        let half = view_direction([9.0, 9.0, 9.0], [1.0, 2.0, 3.0], matrix, 0.5);
        assert_eq!(half, want);
        let persp = view_direction([3.0, 5.0, 10.0], [1.5, -2.0, 4.0], matrix, 0.0);
        assert_ne!(persp, want);
    }
}
