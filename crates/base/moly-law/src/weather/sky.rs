//! Sky gradient rules: per-value recomputation of the shading formula of the
//! game's sky view script (`EnvironmentSkyView`).
//!
//! 天空不是天空盒，是一份产品网格加一条材质属性块：15 个现象共用同一份
//! 网格与材质，天气只差一张 32×1 的渐变条（`_RampTex1`/`_RampTex2`）。
//! 材质记录认属性不认名字——要的输入就是窗口两端（`_GradientMinY`/
//! `_GradientMaxY`）加两个渐变槽，谁带齐谁就是天空材质；两端缺一、读不
//! 成数、或不成正区间，调用方就停画天空，不许补默认值顶替。
//!
//! # 域
//!
//! **The whole module stays in the storage (encoded) domain.** The game is
//! built for the gamma color space: ramp texels are sampled by their stored
//! values, the cross-fade and the additive term are added in this domain, and
//! the result is written to the framebuffer as encoded values. ⇒ Nothing here
//! should decode to linear, compute, and re-encode — the only value that makes
//! that round trip is the additive color, and the whole trip happens on the
//! CPU before the ramp is sampled ([`additive_property`]).
//!
//! This repository's color target has an sRGB format, so the hardware encodes
//! once more on write, which requires the extra step
//! [`srgb_target_value`]: it is **not** part of the game's formula but a domain
//! adaptation.
//! 消费侧配套：渐变条按非 sRGB 装载，采样值与存储域同值。
//!
//! # 形状纪律
//!
//! 无 GPU 对象、无文件读取：调用方把材质记录里的浮点与渐变条纹理素拿来。
//! 渐变条按产品契约恒为 32×1、线性过滤、两端 clamp，本模块给出的
//! [`sample_ramp`] 与运行时 GPU 采样同式，供逐值比对与 CPU 侧读数。

use crate::powf::powf;

/// 渐变条的纹素宽：提取产物 15 份现象渐变条全部 32×1，这是产品契约不是采样值。
pub const RAMP_TEXELS: usize = 32;

/// 一条渐变条的纹理素（RGBA，取值与文件存储同域，不做色彩空间换算）。
pub type Ramp = [[f32; 4]; RAMP_TEXELS];

/// 网格 UV0 的纵向坐标换算成作者侧渐变坐标 v。
///
/// 导出几何按 glTF 的纵向纹理原点（下缘为 0）存 UV，与作者侧上下相反，
/// 所以 v = 1 - uv.y。这条翻转可核：这份天空网格的三个顶点环，天顶环
/// uv.y=0.008729→v=0.991271，地平环 0.5→0.5，底环 0.991271→0.008728。
#[inline]
pub fn gradient_v(uv_y: f32) -> f32 {
    1.0 - uv_y
}

/// 渐变窗口：材质记录里 `_GradientMinY`/`_GradientMaxY` 那一对浮点的合法形。
///
/// 两端都有限且成正经区间（max > min）才是窗口；缺一或退化返回 `None`，
/// 调用方据此停画天空。退化窗口在 GLSL 侧另有 `max(maxY-minY, 1e-6)` 的
/// 分母下限兜底——那是坏数据不涂黑画面的保险，不是数据里存在的形状。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientWindow {
    pub min_y: f32,
    pub max_y: f32,
}

impl GradientWindow {
    /// 材质记录里的窗口两端；非法即 `None`（fail-closed，不补默认值）。
    pub fn new(min_y: f32, max_y: f32) -> Option<Self> {
        if !(min_y.is_finite() && max_y.is_finite() && max_y > min_y) {
            return None;
        }
        Some(Self { min_y, max_y })
    }

    /// 渐变坐标 v 的采样坐标：`t = clamp((v-minY)/max(maxY-minY,1e-6),0,1)`，
    /// `uv = (1-t, 0.5)`。天顶端 → u≈0 → 纹素 0；窗口下沿及其以下 → t=0 →
    /// u=1 → 纹素 31。不做半纹素内缩——运行时拿 u 直接采样，内缩会把整条
    /// 取样位置挪动半个纹素，那是另一条律。
    pub fn ramp_uv(&self, v: f32) -> [f32; 2] {
        let span = (self.max_y - self.min_y).max(1e-6);
        let t = ((v - self.min_y) / span).clamp(0.0, 1.0);
        [1.0 - t, 0.5]
    }
}

/// 双 ramp 过渡：换现象的交叉淡化期间两张渐变条同时在位，按进度逐分量 mix。
/// 进度先 clamp，出界的进度不是加速淡化。
#[inline]
pub fn blend_ramps(a: [f32; 3], b: [f32; 3], progress: f32) -> [f32; 3] {
    let w = progress.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * w,
        a[1] + (b[1] - a[1]) * w,
        a[2] + (b[2] - a[2]) * w,
    ]
}

/// 引擎的 `Mathf.GammaToLinearSpace`：sRGB 分段解码，**三支**。
///
/// The thresholds and constants agree branch by branch and constant by
/// constant with the engine's own two spellings: the byte-level implementation
/// of the same conversion in the engine's managed source, and the branch the
/// engine's built-in shader library names as the "exact version". The managed
/// entry point is an internal call whose body is a free function in the
/// engine's native library. The native body short-circuits at `x == 1` and
/// returns 1 directly, the same value as `powf(1, 2.2) == 1`.
///
/// The power is a call to the C library `powf`, which on a phone resolves to
/// the system libm. This calls [`crate::powf::powf`] (a bit-level
/// implementation of the system libm `powf` of Android 10 and later), **not**
/// `f32::powf`: the latter is a different implementation (the FreeBSD one) in
/// browser targets and differs value by value on these two conversions. The
/// value comparison (see `color_lerp`) runs the native body itself plus the
/// `powf` of the Android 10+ system image's libm, and this function matches it
/// bit for bit on both the local native and the wasm32 targets. The Android 7–9
/// system libm uses the FreeBSD `powf`, which this function does not reproduce.
///
/// 第三支必须在：附加强度可以把乘积抬过 1，那一支走的是 2.2 幂而不是
/// 2.4 那一支的延伸。后处理装箱（泛光 tint 与 threshold、调色 colorFilter）
/// 走的也是本函数。
pub fn gamma_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else if x < 1.0 {
        powf((x + 0.055) / 1.055, 2.4)
    } else {
        powf(x, 2.2)
    }
}

/// 引擎的 `Mathf.LinearToGammaSpace`：sRGB 分段编码，**四支**。
///
/// The exponents are the single-precision constants of the engine's native
/// method body: `0.4166667` (bits 0x3ed55556) and `0.45454547` (bits
/// 0x3ee8ba2f). The latter is not `0.45454545`, which rounds to the adjacent
/// 0x3ee8ba2e and differs by 1 ULP on the `x > 1` branch (the value comparison
/// against the native body catches exactly this). `1.0/2.4` and `1.0/2.2` are
/// not written. The first branch maps non-positive input to zero; it is part
/// of the original method, not a clamp added here. The `x == 1` case is
/// covered by the last branch, which returns 1 directly. After the linear
/// segment, the result is rounded once when multiplying by 1.055 and once when
/// adding -0.055 (the native body uses no fused multiply-add). The power is
/// the same as in [`gamma_to_linear`]: the device libm `powf`, not
/// `f32::powf`.
pub fn linear_to_gamma(x: f32) -> f32 {
    if x <= 0.0 {
        0.0
    } else if x <= 0.003_130_8 {
        12.92 * x
    } else if x < 1.0 {
        1.055 * powf(x, 0.416_666_7) - 0.055
    } else {
        powf(x, 0.454_545_47)
    }
}

/// This frame's value of `_AdditiveColor` in the sky material's property block
/// (the thunder-flash family): the game computes it once per frame on the CPU,
/// per channel `LinearToGammaSpace(intensity × GammaToLinearSpace(base))`, then
/// calls `SetColor` with `ShaderPropertyId.AdditiveColor`. The base is the
/// static color set in `EnvironmentSkyView`; the intensity is the phenomenon
/// timeline's skyAdditiveIntensity.
///
/// Three facts follow from `EnvironmentSkyView.OnUpdate` and the sky shader
/// itself; each differs from an earlier form of this code and is recorded
/// here so it is not changed back:
///
/// 1. **Only the additive term makes the round trip; the base (the ramp
///    sample) does not take part at all.** The whole trip happens on the CPU
///    before the ramp is sampled; the shader receives an already computed
///    encoded-domain color, and combining it with the ramp is a single
///    addition (see [`with_additive`]).
/// 2. **The base color's alpha is never read.** The round trip uses only the
///    r/g/b channels; the alpha of the color written into the property block
///    is the literal 0, and the shader declares this uniform with three
///    components, so there is no fourth channel to read. ⇒ Any `× alpha` would
///    cancel the additive term entirely at the alpha the game actually sends
///    (0).
/// 3. **The round-trip curve is the engine's sRGB piecewise pair, not a pure
///    2.2 power.**
///
/// 没有时间轴驱动时强度底值是 **0**：`GammaToLinearSpace` 的乘积随之为 0，
/// [`linear_to_gamma`] 的取零支给出恰 0，附加项整项不出力——底值若为 1，
/// 任何一份非零附加色会在没人驱动时也悄悄进画面。
pub fn additive_property(color: [f32; 3], intensity: f32) -> [f32; 3] {
    [
        linear_to_gamma(intensity * gamma_to_linear(color[0])),
        linear_to_gamma(intensity * gamma_to_linear(color[1])),
        linear_to_gamma(intensity * gamma_to_linear(color[2])),
    ]
}

/// 着色器侧的合成：`mix(ramp1, ramp2, 进度) + _AdditiveColor`——存储（编码）
/// 域里的一次普通逐通道相加。源天空着色程序全文**没有一个幂运算**：渐变条
/// 采样、交叉淡化、附加项相加、写出，一路都在存储域。
///
/// `additive` 收的是 [`additive_property`] 的结果，不是材质记录里的原值。
pub fn with_additive(base: [f32; 3], additive: [f32; 3]) -> [f32; 3] {
    [base[0] + additive[0], base[1] + additive[1], base[2] + additive[2]]
}

/// The value handed to the sRGB color target. ⚠ **This is not part of the
/// game's formula but a domain adaptation**, kept here only because the shader
/// mirrors this module formula by formula and this step also needs a
/// recomputable definition.
///
/// The game is built for the gamma color space: the result of
/// [`with_additive`] is written directly to the framebuffer as encoded values,
/// with no hardware encoding step. This repository's main color target has an
/// sRGB format, so the hardware encodes once more (sRGB OETF) on write — handing
/// it encoded values directly encodes twice. Measured cost (8-bit codes, in the
/// normal state with flash intensity 0): 0.25 → +73 codes · 0.5 → +60 · 0.75 →
/// +34, i.e. the whole image is too bright by the same sign. So the value is
/// decoded once with the format's EOTF before writing, and the hardware's
/// encoding restores it exactly.
///
/// This uses the **format's** EOTF (the sRGB standard piecewise curve, power
/// 2.4), a separate definition from [`gamma_to_linear`]: the first is the spec
/// of this repository's target format, the second is the engine's conversion.
/// The two agree on every operation over 0..1, and a test rather than shared
/// piecewise code enforces that identity, so a mistake in either one shows up
/// on its own. Both share [`crate::powf::powf`] for the power: the identity
/// needs the same power function, and `f32::powf` is not the same function on
/// every target.
///
/// The value is clamped to 0..1 before decoding: a UNORM target cannot store
/// values outside that range, clamping before is equivalent to the hardware
/// clamping on write (both ends are monotone), and it also makes every power
/// base positive.
pub fn srgb_target_value(encoded: [f32; 3]) -> [f32; 3] {
    fn eotf(x: f32) -> f32 {
        if x <= 0.04045 {
            x / 12.92
        } else {
            powf((x + 0.055) / 1.055, 2.4)
        }
    }
    [
        eotf(encoded[0].clamp(0.0, 1.0)),
        eotf(encoded[1].clamp(0.0, 1.0)),
        eotf(encoded[2].clamp(0.0, 1.0)),
    ]
}

/// 渐变条采样：32×1 纹理、线性过滤、两端 clamp——与 GPU 的双线性加
/// clamp-to-edge 同式。u=0 恰得纹素 0，u=1 恰得纹素 31（半纹素偏移在两端
/// 被 clamp 收拢），中间按相邻两纹素线性插值。u 出界不是错误，是 clamp。
pub fn sample_ramp(ramp: &Ramp, u: f32) -> [f32; 3] {
    let x = u * RAMP_TEXELS as f32 - 0.5;
    let i0 = x.floor();
    let frac = x - i0;
    let texel = |i: f32| ramp[i.clamp(0.0, (RAMP_TEXELS - 1) as f32) as usize];
    let a = texel(i0);
    let b = texel(i0 + 1.0);
    [
        a[0] + (b[0] - a[0]) * frac,
        a[1] + (b[1] - a[1]) * frac,
        a[2] + (b[2] - a[2]) * frac,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values: the 32×1 gradient ramp of 001_sunny (phenomenon id 1),
    /// each texel decoded from ramp.png (RGBA8, decoded to 0..1 floats). Deep
    /// blue at the zenith side (texel 0), pale blue at the horizon side (texel
    /// 31); channel a is always 1.
    fn ramp_001_sunny() -> Ramp {
        const R: [f32; 32] = [
            0.176471, 0.180392, 0.184314, 0.188235, 0.192157, 0.196078, 0.2, 0.203922, 0.207843,
            0.211765, 0.215686, 0.219608, 0.227451, 0.231373, 0.235294, 0.239216, 0.243137,
            0.25098, 0.258824, 0.266667, 0.27451, 0.282353, 0.290196, 0.294118, 0.301961,
            0.309804, 0.317647, 0.32549, 0.333333, 0.341176, 0.392157, 0.552941,
        ];
        const G: [f32; 32] = [
            0.301961, 0.313725, 0.32549, 0.337255, 0.34902, 0.360784, 0.372549, 0.384314,
            0.396078, 0.407843, 0.419608, 0.431373, 0.443137, 0.454902, 0.466667, 0.478431,
            0.490196, 0.505882, 0.521569, 0.537255, 0.552941, 0.568627, 0.584314, 0.6, 0.611765,
            0.627451, 0.643137, 0.658824, 0.67451, 0.690196, 0.72549, 0.807843,
        ];
        let mut ramp = [[0.0f32; 4]; RAMP_TEXELS];
        for (i, texel) in ramp.iter_mut().enumerate() {
            *texel = [R[i], G[i], 1.0, 1.0];
        }
        ramp
    }

    /// Reference values: the window ends in the sky material record (the
    /// original floats, copied as is).
    fn common_sky_window() -> GradientWindow {
        GradientWindow::new(0.449_999_98, 1.0).expect("the reference window is a proper interval")
    }

    /// ±ULP 窗（同 powf 族范式）：powf 随平台 libm 差 1 ULP，且优化器会在
    /// 等价式间改写；窗不吸收公式级错误——错一位式子偏差远超 1 ULP。
    fn assert_ulp(actual: f32, expected: f32) {
        let scale = expected.abs().max(1.0);
        let ulp = f32::EPSILON * scale;
        assert!(
            (actual - expected).abs() <= 2.0 * ulp,
            "actual {actual} vs expected {expected}"
        );
    }

    #[test]
    fn gradient_v_flips_gltf_uv_three_rings() {
        // 环锚引用自网格顶点 UV，只有 6 位小数的字深——按锚自身的字深比对，
        // 不套 ULP 窗（1-u 在小数量级的舍入远超 2 ULP 是字深问题不是律错）。
        for (uv, v) in [(0.008_729, 0.991_271), (0.5, 0.5), (0.991_271, 0.008_728)] {
            let actual = gradient_v(uv);
            assert!((actual - v).abs() < 1e-5, "uv={uv} v={actual}");
        }
        // 翻转是自己的逆：v 域往返是恒等。
        assert_eq!(gradient_v(gradient_v(0.37)), 0.37);
    }

    #[test]
    fn window_rejects_degenerate_and_non_finite() {
        assert_eq!(GradientWindow::new(0.45, 1.0), Some(GradientWindow { min_y: 0.45, max_y: 1.0 }));
        // The material window's lower bound is 0.44999998807907104; it is kept as is, not smoothed.
        assert!(GradientWindow::new(0.449_999_988_079_071_04, 1.0).is_some());
        assert_eq!(GradientWindow::new(1.0, 1.0), None); // 退化：两端相等
        assert_eq!(GradientWindow::new(1.0, 0.45), None); // 倒挂
        assert_eq!(GradientWindow::new(f32::NAN, 1.0), None);
        assert_eq!(GradientWindow::new(0.45, f32::INFINITY), None);
    }

    #[test]
    fn ramp_uv_endpoints_and_clamp() {
        let w = common_sky_window();
        // 天顶环：v≈0.9913 落在窗口上沿附近 → t≈0.984 → u≈0.0159（纹素 0 侧）。
        let [u, v] = w.ramp_uv(0.991_271);
        assert!((u - 0.015_870_908_7).abs() < 1e-7, "u={u}");
        assert!((v - 0.5).abs() < f32::EPSILON);
        // 地平环：v=0.5 → t=0.5/5.5… → u≈0.9091。
        let [u, _] = w.ramp_uv(0.5);
        assert!((u - 0.909_090_889_4).abs() < 1e-7, "u={u}");
        // 底端及窗口下沿以下：t 钳 0 → u=1 → 纹素 31（地平色铺到底）。
        assert_eq!(w.ramp_uv(0.0)[0], 1.0);
        // 超出上沿：t 钳 1 → u=0 → 纹素 0。
        assert_eq!(w.ramp_uv(1.2)[0], 0.0);
    }

    #[test]
    fn sample_ramp_hits_exact_texels_at_both_ends() {
        let ramp = ramp_001_sunny();
        // u=0 → x=-0.5 → 两个采样下标都 clamp 到 0 → 恰纹素 0。
        let a = sample_ramp(&ramp, 0.0);
        assert_eq!(a, [ramp[0][0], ramp[0][1], ramp[0][2]]);
        // u=1 → x=31.5 → 下标 31/32 都 clamp 到 31 → 恰纹素 31。
        let b = sample_ramp(&ramp, 1.0);
        assert_eq!(b, [ramp[31][0], ramp[31][1], ramp[31][2]]);
        // 出界即 clamp。
        assert_eq!(sample_ramp(&ramp, -0.25), a);
        assert_eq!(sample_ramp(&ramp, 1.5), b);
    }

    #[test]
    fn sample_ramp_interpolates_half_texel() {
        let ramp = ramp_001_sunny();
        // u=0.5 → x=15.5 → 纹素 15/16 的中点。
        let c = sample_ramp(&ramp, 0.5);
        assert!((c[0] - (ramp[15][0] + ramp[16][0]) * 0.5).abs() < 1e-6);
        assert!((c[1] - (ramp[15][1] + ramp[16][1]) * 0.5).abs() < 1e-6);
        assert!((c[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn blend_ramps_mixes_and_clamps_progress() {
        let a = [0.1, 0.2, 0.3];
        let b = [0.5, 0.6, 0.7];
        let m = blend_ramps(a, b, 0.25);
        assert!((m[0] - 0.2).abs() < 1e-6);
        assert!((m[1] - 0.3).abs() < 1e-6);
        assert!((m[2] - 0.4).abs() < 1e-6);
        assert_eq!(blend_ramps(a, b, -1.0), a);
        assert_eq!(blend_ramps(a, b, 2.0), b);
    }

    #[test]
    fn additive_zero_intensity_leaves_base_bit_exact() {
        // 强度 0：乘积为 0 → linear_to_gamma 的取零支给出恰 0 → 相加是恒等。
        // 这里**不给 ULP 窗**：形状对了以后底值根本不进任何 powf，位级相等
        // 是可判的。要是谁把底值重新塞回一次往返，这条立刻红。
        //
        // ⚠ The base must come in through black_box. The test runs in an
        // optimized build, where constant folding of `powf` computes at higher
        // precision at compile time — **a shape that puts the base back into
        // the round trip would still be bit-equal**, and the test would have no
        // signal on the dimension it is meant to catch (measured: without
        // black_box, routing the base through a 2.2-power round trip still
        // passes). The values are chosen too: 0.301961 and 1.0 happen to be
        // restored bit-exactly by a 2.2-power round trip, so on their own they
        // would not catch it either.
        let prop = additive_property([0.7, 0.3, 1.0], 0.0);
        assert_eq!(prop, [0.0, 0.0, 0.0]);
        for base in [
            [0.176_471f32, 0.301_961, 1.0],
            [0.5, 0.25, 0.35],
            [0.1, 0.75, 0.0],
        ] {
            let base = std::hint::black_box(base);
            assert_eq!(with_additive(base, std::hint::black_box(prop)), base);
        }
    }

    /// An earlier shape of this code, kept only as a contrast arm: the base
    /// also enters the round trip and the additive term is scaled by an alpha.
    /// It makes "how far the two shapes differ on the same input" testable —
    /// the correct shape is described in [`additive_property`]'s doc (the three
    /// differences are listed there).
    fn previous_shape(base: [f32; 3], additive: [f32; 4], intensity: f32) -> [f32; 3] {
        const GAMMA: f32 = 2.2;
        let mut lin = [0.0f32; 3];
        for i in 0..3 {
            lin[i] = base[i].max(0.0).powf(GAMMA)
                + additive[i].max(0.0).powf(GAMMA) * additive[3] * intensity;
        }
        [lin[0].powf(1.0 / GAMMA), lin[1].powf(1.0 / GAMMA), lin[2].powf(1.0 / GAMMA)]
    }

    #[test]
    fn additive_survives_the_alpha_the_source_actually_delivers() {
        // The test point is the alpha the game actually sends: the color the
        // CPU writes into the property block has alpha literal 0, and the
        // shader's uniform has only three components and cannot read it.
        // ⇒ The additive term must contribute **in full**; a shape that
        // multiplies by alpha cancels it entirely.
        let base = [0.5, 0.25, 0.0];
        let add_rgb = [1.0, 0.5, 0.0];
        let intensity = 0.25;

        let prop = additive_property(add_rgb, intensity);
        let out = with_additive(base, prop);
        // The additive term is really there (positive arm > 0: without it this
        // test measures nothing). The computed value is 0.537099; the test only
        // pins the "more than half the range" band, not the decimals.
        assert!(prop[0] > 0.5, "附加项被抹平了：{prop:?}");

        // 同一输入喂旧形状：alpha=0 让它恰回底值。
        let old = previous_shape(base, [add_rgb[0], add_rgb[1], add_rgb[2], 0.0], intensity);
        assert_ulp(old[0], base[0]);
        // The two shapes differ by half the range here — the test has a signal on this dimension.
        assert!(out[0] - old[0] > 0.5, "new {out:?} vs previous {old:?}");

        // 「第四个通道进不来」这件事由类型担着（[f32; 3]），不由运行时断言
        // 担着——本仓的规矩是能被类型消掉的就消掉。这里只留这句说明。
    }

    #[test]
    fn additive_only_roundtrip_leaves_base_out_of_the_powf_chain() {
        // 底值不进往返 ⇒ 出值随底值**逐一等量**平移。旧形状不具备这条性质
        // （底值在 pow 里，平移量随底值变）。取两个底值现算斜率。
        let add_rgb = [0.5, 0.5, 0.5];
        let intensity = 0.5;
        let prop = additive_property(add_rgb, intensity);
        for base in [0.1f32, 0.4, 0.9] {
            let out = with_additive([base; 3], prop);
            assert_ulp(out[0] - base, prop[0]);
        }
        // 判别：旧形状在同样两点上的平移量不相等（差 > 0.05）。
        let shift = |b: f32| {
            previous_shape([b; 3], [add_rgb[0], add_rgb[1], add_rgb[2], 1.0], intensity)[0] - b
        };
        assert!((shift(0.1) - shift(0.9)).abs() > 0.05, "旧形状的平移量看起来是常量，判别失效");
    }

    /// 色彩目标写出时硬件那一次编码（sRGB OETF，按格式规范拼写；指数是
    /// exact `1/2.4`, not the engine's truncated literal). It lives outside this
    /// module, so the test spells it out separately: [`srgb_target_value`]
    /// must be its inverse.
    fn hardware_srgb_encode(x: f32) -> f32 {
        if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    }

    /// 8 位色彩目标存下来的码值。
    fn code8(x: f32) -> i32 {
        (x.clamp(0.0, 1.0) * 255.0).round() as i32
    }

    #[test]
    fn target_value_survives_the_hardware_encode_on_all_256_codes() {
        // 正向臂：交给 sRGB 目标的值，被硬件编码回来后必须**逐码复原**。
        // 全部 256 个码值一个不落，且浮点误差报出来（不是「差不多」）。
        let mut worst = 0.0f32;
        for c in 0..=255u32 {
            let gamma = c as f32 / 255.0;
            let handed = srgb_target_value([gamma; 3]);
            let back = hardware_srgb_encode(handed[0]);
            assert_eq!(code8(back), c as i32, "码 {c}：回来成了 {back}");
            worst = worst.max((back - gamma).abs());
        }
        // 现算上界：单精度往返 5.96e-8（= 1 ULP @1.0）。窗按它定，不放宽。
        assert!(worst <= 6.0e-8, "往返误差 {worst} 超出单精度往返上界");
    }

    #[test]
    fn handing_the_encoded_value_straight_to_the_target_encodes_twice() {
        // 判别臂：把编码值直接交给 sRGB 目标（= 没有 srgb_target_value 那一步）
        // 时，存下来的码值**在每一个非饱和码上都偏高**。这条保证上面那条正向
        // 臂在这一维上有信号——两种写法处处可分，不是碰巧在采样点上相等。
        let mut min_delta = i32::MAX;
        for c in 1..=254u32 {
            let gamma = c as f32 / 255.0;
            let delta = code8(hardware_srgb_encode(gamma)) - c as i32;
            assert!(delta > 0, "码 {c} 上两种写法不可分（delta {delta}）");
            min_delta = min_delta.min(delta);
        }
        assert_eq!(min_delta, 1);
        // 三个具名探针：整片天空恒亮一档的实测幅度。
        for (gamma, expect) in [(0.25f32, 73), (0.5, 60), (0.75, 34)] {
            let delta = code8(hardware_srgb_encode(gamma)) - code8(gamma);
            assert_eq!(delta, expect, "底值 {gamma} 的偏移应为 +{expect} 码");
        }
    }

    #[test]
    fn target_eotf_and_engine_decode_agree_across_the_unit_interval() {
        // 两个来源（目标格式的规范 · 引擎的换算）在 0..1 上的同一性：
        // doc 里声明了它，这里钉住它。任一份被改错，这条就红。
        for c in 0..=255u32 {
            let gamma = c as f32 / 255.0;
            let by_format = srgb_target_value([gamma; 3])[0];
            let by_engine = gamma_to_linear(gamma);
            assert_eq!(by_format, by_engine, "码 {c} 上两份分段不一致");
        }
        // 而 1 之外它们必须**分开**：格式那边钳住，引擎那边走 2.2 幂那一支。
        assert_eq!(srgb_target_value([1.5; 3])[0], srgb_target_value([1.0; 3])[0]);
        assert!(gamma_to_linear(1.5) > 2.0, "引擎的第三支不见了");
    }

    #[test]
    fn engine_gamma_curves_are_a_matched_pair_across_all_branches() {
        // 分界处两支必须接得上——这一条钉的是「常量是配套的那一组」：
        // 谁把 12.92 或 0.055 写错，接缝立刻裂开。
        let seam = 0.04045f32;
        assert!((seam / 12.92 - ((seam + 0.055) / 1.055).powf(2.4)).abs() < 1e-7);
        let seam_lin = 0.003_130_8f32;
        assert!((12.92 * seam_lin - (1.055 * seam_lin.powf(0.416_666_7) - 0.055)).abs() < 1e-5);

        // 互逆：三支/四支全覆盖（含 1 以上那一支），指数配对错了就红。
        for x in [0.0f32, 0.01, 0.04045, 0.2, 0.5, 0.999, 1.0, 1.5, 4.0] {
            let back = linear_to_gamma(gamma_to_linear(x));
            assert!((back - x).abs() <= 1e-5 * x.max(1.0), "x={x} 往返成了 {back}");
        }
        // 非正输入走取零支（不是负数按线性放行）。
        assert_eq!(linear_to_gamma(0.0), 0.0);
        assert_eq!(linear_to_gamma(-1.0), 0.0);

        // 判别：低支是**线性除**不是幂。2.2 纯幂在这里差一个数量级，
        // 所以「曲线是分段的」这件事在这一点上可判。
        let low = gamma_to_linear(0.02);
        assert_ulp(low, 0.02 / 12.92);
        assert!(low / 0.02f32.powf(2.2) > 5.0, "低支与 2.2 纯幂不可分");
        // 判别：1 以上走 2.2 幂，不是把 2.4 那一支延伸出去。
        assert_ulp(gamma_to_linear(1.5), 1.5f32.powf(2.2));
        assert!((gamma_to_linear(1.5) - ((1.5 + 0.055) / 1.055f32).powf(2.4)).abs() > 0.05);
    }
}
