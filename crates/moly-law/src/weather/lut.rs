//! 引擎原生调色的 LDR 查找表：烘焙（CPU 一次一份）与 uber 侧的取表律。
//!
//! 真源是两段：URP 的颜色分级 LUT pass 每帧把音量栈里的七个调色组件烘成
//! 一张 32³ 的查找表（摊平成 1024×32 的条带，`R8G8B8A8_UNorm`，双线性），
//! 引擎 uber pass 再把场景色乘曝光、钳到 0..1、按条带坐标取两次样、沿蓝
//! 通道插值。管线资产的颜色分级模式是 LDR、表边长 32；现象相机的
//! `renderPostProcessing` 是开的，所以这两段**每个现象都跑**——包括调色
//! 组件整个不活跃的晴天：那时烘出来的也不是恒等表（见
//! [`CurveTexture::identity`]：取到 1 的输入被钳在 127/128，8 位存储后
//! 线性 1.0 查出 253/255）。
//!
//! 本模块的烘焙律逐行转写自随包发布的 LDR 查找表构建程序（GLES3 片元
//! 程序），常数照抄程序里编译器折好的字面量（`0.0734997839`、
//! `13.6054821` 这一类），不回写成教科书式子；打包律转写自 URP 的
//! 颜色分级 LUT pass 与 `ColorUtils` 的三个 `Prepare*`。
//!
//! 输入值域：本模块只打包音量栈里 `ColorAdjustments`、`SplitToning`、
//! `WhiteBalance` 三个组件的采纳值；`ChannelMixer`、`ShadowsMidtonesHighlights`、
//! `LiftGammaGain`、`ColorCurves` 四个取构造默认（档案里没有任何一档带
//! 它们，[`super::PostProcessProfile::lut_stack`] 对带了它们的档案响亮拒绝）。

/// 查找表边长（管线资产 `m_ColorGradingLutSize`）。
pub const LUT_SIZE: usize = 32;
/// 条带宽（`LUT_SIZE²`）。
pub const LUT_WIDTH: usize = LUT_SIZE * LUT_SIZE;

/// 音量组件 `TextureCurve` 的 128 格烘焙：第 `i` 格存 `Evaluate(i / 128)`，
/// 半浮点、双线性、钳边。取样坐标由调用侧加半格偏移。
#[derive(Debug, Clone, PartialEq)]
pub struct CurveTexture(pub [f32; 128]);

impl CurveTexture {
    /// 主曲线与 RGB 三条的构造默认：两个键 `(0,0)`、`(1,1)`，切线都是 1，
    /// 求值恰为恒等；烘成 `i / 128`（半浮点精确可表）。最后一格是
    /// `127 / 128`，所以取到 1 的输入被钳在 `127 / 128`。
    #[must_use]
    pub fn identity() -> Self {
        let mut t = [0.0; 128];
        for (i, v) in t.iter_mut().enumerate() {
            *v = i as f32 / 128.0;
        }
        Self(t)
    }

    /// 色相/饱和度四条的构造默认：没有键，`zeroValue` = 0.5，整条常数。
    #[must_use]
    pub fn constant_half() -> Self {
        Self([0.5; 128])
    }

    /// 双线性、钳边取样（GL 纹素中心在 `(i + 0.5) / 128`）。
    #[must_use]
    pub fn sample(&self, u: f32) -> f32 {
        let p = u * 128.0 - 0.5;
        let i0 = p.floor();
        let f = p - i0;
        let at = |i: f32| self.0[(i.max(0.0) as usize).min(127)];
        let (a, b) = if i0 < 0.0 {
            (at(0.0), at(0.0))
        } else {
            (at(i0), at(i0 + 1.0))
        };
        a + f * (b - a)
    }
}

/// 烘焙程序的全部 uniform（已打包）。字段名对应程序里的 uniform 名。
#[derive(Debug, Clone, PartialEq)]
pub struct LdrLutInputs {
    /// `_ColorBalance.xyz`：`ColorUtils.ColorBalanceToLMSCoeffs(temperature, tint)`。
    pub color_balance: [f32; 3],
    /// `_ColorFilter.xyz`：`colorFilter.linear`。
    pub color_filter: [f32; 3],
    /// `_HueSatCon`：`(hueShift/360, saturation/100 + 1, contrast/100 + 1)`。
    pub hue_sat_con: [f32; 3],
    /// `_ChannelMixerRed/Green/Blue.xyz`（百分数 / 100）。
    pub channel_mixer: [[f32; 3]; 3],
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub shadows: [f32; 3],
    pub midtones: [f32; 3],
    pub highlights: [f32; 3],
    /// `_ShaHiLimits`：`(shadowsStart, shadowsEnd, highlightsStart, highlightsEnd)`。
    pub sha_hi_limits: [f32; 4],
    /// `_SplitShadows`：颜色原值（不转线性）+ `w = balance / 100`。
    pub split_shadows: [f32; 4],
    /// `_SplitHighlights.xyz`：颜色原值。
    pub split_highlights: [f32; 3],
    pub curve_master: CurveTexture,
    pub curve_red: CurveTexture,
    pub curve_green: CurveTexture,
    pub curve_blue: CurveTexture,
    pub curve_hue_vs_hue: CurveTexture,
    pub curve_hue_vs_sat: CurveTexture,
    pub curve_sat_vs_sat: CurveTexture,
    pub curve_lum_vs_sat: CurveTexture,
}

/// 音量栈里本模块读取的三个组件的采纳值（原始单位）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LutStack {
    pub post_exposure: f32,
    pub contrast: f32,
    pub color_filter: [f32; 4],
    pub hue_shift: f32,
    pub saturation: f32,
    pub split_shadows: [f32; 4],
    pub split_highlights: [f32; 4],
    pub split_balance: f32,
    pub white_balance_temperature: f32,
    pub white_balance_tint: f32,
}

/// `Mathf.GammaToLinearSpace`（与本 crate 其余处同一条三支律）。
fn gamma_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else if x < 1.0 {
        ((x + 0.055) / 1.055).powf(2.4)
    } else {
        x.powf(2.2)
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    c[0] * 0.2126729 + c[1] * 0.7151522 + c[2] * 0.072175
}

/// `ColorUtils.ColorBalanceToLMSCoeffs`。
fn color_balance_to_lms(temperature: f32, tint: f32) -> [f32; 3] {
    let t1 = temperature / 65.0;
    let t2 = tint / 65.0;
    let x = 0.31271 - t1 * if t1 < 0.0 { 0.1 } else { 0.05 };
    let y = (2.87 * x - 3.0 * x * x - 0.275_095_07) + t2 * 0.05;
    let big_x = x / y;
    let big_z = (1.0 - x - y) / y;
    let l = 0.7328 * big_x + 0.4296 - 0.1624 * big_z;
    let m = -0.7036 * big_x + 1.6975 + 0.0061 * big_z;
    let s = 0.0030 * big_x + 0.0136 + 0.9834 * big_z;
    [0.949237 / l, 1.03542 / m, 1.08728 / s]
}

/// `ColorUtils.PrepareShadowsMidtonesHighlights` 的一路。
fn prepare_smh(v: [f32; 4]) -> [f32; 3] {
    let w = v[3] * if v[3].is_sign_negative() { 1.0 } else { 4.0 };
    [
        (gamma_to_linear(v[0]) + w).max(0.0),
        (gamma_to_linear(v[1]) + w).max(0.0),
        (gamma_to_linear(v[2]) + w).max(0.0),
    ]
}

/// `ColorUtils.PrepareLiftGammaGain`。
fn prepare_lift_gamma_gain(
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let l = [
        gamma_to_linear(lift[0]) * 0.15,
        gamma_to_linear(lift[1]) * 0.15,
        gamma_to_linear(lift[2]) * 0.15,
    ];
    let lum = luminance(l);
    let lift_out = [
        l[0] - lum + lift[3],
        l[1] - lum + lift[3],
        l[2] - lum + lift[3],
    ];
    let g = [
        gamma_to_linear(gamma[0]) * 0.8,
        gamma_to_linear(gamma[1]) * 0.8,
        gamma_to_linear(gamma[2]) * 0.8,
    ];
    let lum = luminance(g);
    let gw = gamma[3] + 1.0;
    let gamma_out = [
        1.0 / (g[0] - lum + gw).max(1e-3),
        1.0 / (g[1] - lum + gw).max(1e-3),
        1.0 / (g[2] - lum + gw).max(1e-3),
    ];
    let n = [
        gamma_to_linear(gain[0]) * 0.8,
        gamma_to_linear(gain[1]) * 0.8,
        gamma_to_linear(gain[2]) * 0.8,
    ];
    let lum = luminance(n);
    let nw = gain[3] + 1.0;
    let gain_out = [n[0] - lum + nw, n[1] - lum + nw, n[2] - lum + nw];
    (lift_out, gamma_out, gain_out)
}

impl LdrLutInputs {
    /// 按 URP 颜色分级 LUT pass 的打包律，从三个组件的采纳值与其余四个
    /// 组件的构造默认打出烘焙 uniform。
    #[must_use]
    pub fn from_stack(s: &LutStack) -> Self {
        let unit = [1.0, 1.0, 1.0, 0.0];
        let (lift, gamma, gain) = prepare_lift_gamma_gain(unit, unit, unit);
        Self {
            color_balance: color_balance_to_lms(s.white_balance_temperature, s.white_balance_tint),
            color_filter: [
                gamma_to_linear(s.color_filter[0]),
                gamma_to_linear(s.color_filter[1]),
                gamma_to_linear(s.color_filter[2]),
            ],
            hue_sat_con: [
                s.hue_shift / 360.0,
                s.saturation / 100.0 + 1.0,
                s.contrast / 100.0 + 1.0,
            ],
            channel_mixer: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            lift,
            gamma,
            gain,
            shadows: prepare_smh(unit),
            midtones: prepare_smh(unit),
            highlights: prepare_smh(unit),
            sha_hi_limits: [0.0, 0.3, 0.55, 1.0],
            split_shadows: [
                s.split_shadows[0],
                s.split_shadows[1],
                s.split_shadows[2],
                s.split_balance / 100.0,
            ],
            split_highlights: [
                s.split_highlights[0],
                s.split_highlights[1],
                s.split_highlights[2],
            ],
            curve_master: CurveTexture::identity(),
            curve_red: CurveTexture::identity(),
            curve_green: CurveTexture::identity(),
            curve_blue: CurveTexture::identity(),
            curve_hue_vs_hue: CurveTexture::constant_half(),
            curve_hue_vs_sat: CurveTexture::constant_half(),
            curve_sat_vs_sat: CurveTexture::constant_half(),
            curve_lum_vs_sat: CurveTexture::constant_half(),
        }
    }
}

const LUMA: [f32; 3] = [0.212_672_9, 0.715_152_2, 0.072_175_004];

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn map3(v: [f32; 3], f: impl Fn(f32) -> f32) -> [f32; 3] {
    [f(v[0]), f(v[1]), f(v[2])]
}

/// 分离色调的一次 soft light（程序里两支都算、按 `blend >= 0.5` 选）。
fn soft_light(base: [f32; 3], blend: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for i in 0..3 {
        let two_b = base[i] + base[i];
        let r1 = two_b * blend[i] + base[i] * base[i] * (1.0 - 2.0 * blend[i]);
        let r2 = base[i].sqrt() * (blend[i] * 2.0 - 1.0) + two_b * (1.0 - blend[i]);
        out[i] = if blend[i] >= 0.5 { r2 } else { r1 };
    }
    out
}

/// 构建程序里的 RGB→HSV，照程序自己的算术形状转写：两次「按 step 线性
/// 插值」都写成 `底 + step · (目标 − 底)`，常数取程序里折好的字面量
/// （`p.w` 在绿不小于蓝时是 `−1 + 0.666666687`，不是精确的 −1/3）。用选择
/// 支写同一件事会在末位上与程序不同，而这张表要与程序逐字节一致。
fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0], c[1], c[2]);
    let s1 = if g >= b { 1.0f32 } else { 0.0 };
    let p = [
        b + s1 * (g - b),
        g + s1 * (b - g),
        s1 * 1.0 + -1.0,
        s1 * -1.0 + 0.666_666_7,
    ];
    let s2 = if r >= p[0] { 1.0f32 } else { 0.0 };
    let q = [
        p[0] + s2 * (r - p[0]),
        p[1],
        p[3] + s2 * (p[2] - p[3]),
        r + s2 * (p[0] - r),
    ];
    let d = q[0] - q[3].min(q[1]);
    let e = 1.0e-4;
    [
        (q[2] + (q[3] - q[1]) / (d * 6.0 + e)).abs(),
        d / (q[0] + e),
        q[0],
    ]
}

fn hsv_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let k = [1.0, 2.0 / 3.0, 1.0 / 3.0];
    let mut out = [0.0; 3];
    for i in 0..3 {
        let x = c[0] + k[i];
        let p = ((x - x.floor()) * 6.0 - 3.0).abs();
        out[i] = c[2] * (1.0 + c[1] * ((p - 1.0).clamp(0.0, 1.0) - 1.0));
    }
    out
}

/// 一格的烘焙：条带坐标 `(r, g, b)`（线性域、0..1）进，调好的颜色出。
/// 逐行对应构建程序的片元主体。
#[must_use]
pub fn grade(inputs: &LdrLutInputs, rgb: [f32; 3]) -> [f32; 3] {
    // 白平衡（LMS 域）。
    let lms = [
        dot3([0.390_405, 0.549_941, 0.008_926_32], rgb),
        dot3([0.070_841_6, 0.963_172, 0.001_357_75], rgb),
        dot3([0.023_108_2, 0.128_021, 0.936_245], rgb),
    ];
    let lms = [
        lms[0] * inputs.color_balance[0],
        lms[1] * inputs.color_balance[1],
        lms[2] * inputs.color_balance[2],
    ];
    let mut c = [
        dot3([2.858_47, -1.628_79, -0.024_891], lms),
        dot3([-0.210_182, 1.158_2, 0.000_324_281], lms),
        dot3([-0.041_812, -0.118_169, 1.068_67], lms),
    ];
    // LogC 域绕 ACEScc 中灰的对比（编译器折好的常数）。
    c = map3(c, |x| {
        let t = (x * 5.555_556 + 0.047_996).max(0.0).log2();
        let t = t * 0.073_499_784 + -0.027_552_396;
        let t = t * inputs.hue_sat_con[2] + 0.027_552_396;
        ((t * 13.605_482).exp2() + -0.047_996) * 0.179_999_99
    });
    // 滤色：无钳制乘子，随后抹平负值。
    c = [
        (c[0] * inputs.color_filter[0]).max(0.0),
        (c[1] * inputs.color_filter[1]).max(0.0),
        (c[2] * inputs.color_filter[2]).max(0.0),
    ];
    // 分离色调（gamma 域；亮度从进入这一步时的颜色取一次，两次 soft light 共用）。
    let gamma = map3(c, |x| (x.log2() * 0.454_545_47).exp2());
    let luma = (dot3(map3(gamma, |x| x.min(1.0)), LUMA) + inputs.split_shadows[3]).clamp(0.0, 1.0);
    let sh = [
        (1.0 - luma) * (inputs.split_shadows[0] - 0.5) + 0.5,
        (1.0 - luma) * (inputs.split_shadows[1] - 0.5) + 0.5,
        (1.0 - luma) * (inputs.split_shadows[2] - 0.5) + 0.5,
    ];
    let hi = [
        luma * (inputs.split_highlights[0] - 0.5) + 0.5,
        luma * (inputs.split_highlights[1] - 0.5) + 0.5,
        luma * (inputs.split_highlights[2] - 0.5) + 0.5,
    ];
    let gamma = soft_light(soft_light(gamma, sh), hi);
    c = map3(gamma, |x| (x.abs().log2() * 2.2).exp2());
    // 通道混合。
    c = [
        dot3(c, inputs.channel_mixer[0]),
        dot3(c, inputs.channel_mixer[1]),
        dot3(c, inputs.channel_mixer[2]),
    ];
    // 阴影 / 中间调 / 高光。
    let l = dot3(c, LUMA);
    let lim = inputs.sha_hi_limits;
    let smooth = |x: f32, e0: f32, e1: f32| {
        let t = ((x - e0) * (1.0 / (e1 - e0))).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let shadows_f = 1.0 - smooth(l, lim[0], lim[1]);
    let highlights_f = smooth(l, lim[2], lim[3]);
    let midtones_f = 1.0 - shadows_f - highlights_f;
    c = [
        c[0] * inputs.shadows[0] * shadows_f
            + c[0] * inputs.midtones[0] * midtones_f
            + c[0] * inputs.highlights[0] * highlights_f,
        c[1] * inputs.shadows[1] * shadows_f
            + c[1] * inputs.midtones[1] * midtones_f
            + c[1] * inputs.highlights[1] * highlights_f,
        c[2] * inputs.shadows[2] * shadows_f
            + c[2] * inputs.midtones[2] * midtones_f
            + c[2] * inputs.highlights[2] * highlights_f,
    ];
    // 升 / 伽马 / 增益：sign(x) * |x|^gamma。
    for i in 0..3 {
        let x = c[i] * inputs.gain[i] + inputs.lift[i];
        let sign = if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        };
        c[i] = sign * (x.abs().log2() * inputs.gamma[i]).exp2();
    }
    // HSV：色相偏移 + 色相-色相曲线；饱和倍数取三条饱和度曲线。
    let mut hsv = rgb_to_hsv(c);
    let sat_mult = inputs.curve_hue_vs_sat.sample(hsv[0]).clamp(0.0, 1.0)
        * 2.0
        * (inputs.curve_sat_vs_sat.sample(hsv[1]).clamp(0.0, 1.0) * 2.0)
        * (inputs
            .curve_lum_vs_sat
            .sample(dot3(c, LUMA))
            .clamp(0.0, 1.0)
            * 2.0);
    let mut hue = hsv[0] + inputs.hue_sat_con[0];
    hue += inputs.curve_hue_vs_hue.sample(hue).clamp(0.0, 1.0) - 0.5;
    hsv[0] = if hue < 0.0 {
        hue + 1.0
    } else if hue > 1.0 {
        hue - 1.0
    } else {
        hue
    };
    c = hsv_to_rgb(hsv);
    // 整体饱和。
    let l = dot3(c, LUMA);
    let s = inputs.hue_sat_con[1] * sat_mult;
    c = [l + s * (c[0] - l), l + s * (c[1] - l), l + s * (c[2] - l)];
    // YRGB 曲线：取样坐标各加半格（1/256）。
    const HALF_TEXEL: f32 = 1.0 / 256.0;
    let m = [
        inputs
            .curve_master
            .sample(c[0] + HALF_TEXEL)
            .clamp(0.0, 1.0),
        inputs
            .curve_master
            .sample(c[1] + HALF_TEXEL)
            .clamp(0.0, 1.0),
        inputs
            .curve_master
            .sample(c[2] + HALF_TEXEL)
            .clamp(0.0, 1.0),
    ];
    [
        inputs.curve_red.sample(m[0] + HALF_TEXEL).clamp(0.0, 1.0),
        inputs.curve_green.sample(m[1] + HALF_TEXEL).clamp(0.0, 1.0),
        inputs.curve_blue.sample(m[2] + HALF_TEXEL).clamp(0.0, 1.0),
    ]
}

/// UNorm 8 位存储：`round(clamp(x, 0, 1) * 255)`。
fn unorm8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 烘整张条带：`LUT_WIDTH × LUT_SIZE` 个 RGBA8，行 0 是纹理坐标 v = 0
/// 的一行（与构建程序的片元坐标同一原点：`uv = ((x + 0.5) / 1024,
/// (y + 0.5) / 32)`）。格坐标照程序的写法从片元坐标反推（`r` 取
/// `frac(u·32)`、`b` 取 `floor(u·32)`、都乘 `32/31` 再去半格）。
#[must_use]
pub fn bake(inputs: &LdrLutInputs) -> Vec<u8> {
    let mut out = vec![0u8; LUT_WIDTH * LUT_SIZE * 4];
    for y in 0..LUT_SIZE {
        for x in 0..LUT_WIDTH {
            let c = grade(inputs, strip_input(x, y));
            let at = (y * LUT_WIDTH + x) * 4;
            out[at] = unorm8(c[0]);
            out[at + 1] = unorm8(c[1]);
            out[at + 2] = unorm8(c[2]);
            out[at + 3] = 255;
        }
    }
    out
}

/// 条带纹素 `(x, y)` 的输入颜色（线性域）：构建程序从片元坐标反推格坐标
/// 的那几步，逐项照程序写（`_Lut_Params` 是 `(32, 0.5/1024, 0.5/32, 32/31)`）。
#[must_use]
pub fn strip_input(x: usize, y: usize) -> [f32; 3] {
    let size = LUT_SIZE as f32;
    let params = [
        size,
        0.5 / LUT_WIDTH as f32,
        0.5 / size,
        size / (size - 1.0),
    ];
    let u = (x as f32 + 0.5) / LUT_WIDTH as f32;
    let v = (y as f32 + 0.5) / size;
    let slice = (u * params[0]).floor();
    let r = (u * params[0] - slice - params[2]) * params[3];
    let g = (v - params[2]) * params[3];
    let b = slice * params[2] * params[3] * 2.0;
    [r, g, b]
}

/// uber 侧的 `_Lut_Params`：`(1/宽, 1/高, 高 − 1, 2^postExposure)`。曝光
/// 倍数照引擎 `Mathf.Pow` 的做法在双精度里取幂再截回单精度。
#[must_use]
pub fn uber_lut_params(post_exposure: f32) -> [f32; 4] {
    [
        1.0 / LUT_WIDTH as f32,
        1.0 / LUT_SIZE as f32,
        LUT_SIZE as f32 - 1.0,
        2.0f64.powf(f64::from(post_exposure)) as f32,
    ]
}

/// uber 侧取表（研究仪器用的 CPU 版；运行时那份在着色器里）：
/// 输入是线性域颜色，先乘曝光、钳到 0..1，再按 `ApplyLut2D` 的条带坐标
/// 双线性取两次、沿蓝插值。`lut` 是 [`bake`] 的输出。
#[must_use]
pub fn apply(lut: &[u8], params: [f32; 4], linear: [f32; 3]) -> [f32; 3] {
    let c = map3(linear, |x| (x * params[3]).clamp(0.0, 1.0));
    let (w, h) = (LUT_WIDTH as f32, LUT_SIZE as f32);
    let z = c[2] * params[2];
    let shift = z.floor();
    let uy = c[0] * params[2] * params[0] + params[0] * 0.5;
    let vz = c[1] * params[2] * params[1] + params[1] * 0.5;
    let ux = shift * params[1] + uy;
    let texel = |x: i64, y: i64| -> [f32; 3] {
        let xi = x.clamp(0, LUT_WIDTH as i64 - 1) as usize;
        let yi = y.clamp(0, LUT_SIZE as i64 - 1) as usize;
        let at = (yi * LUT_WIDTH + xi) * 4;
        [
            lut[at] as f32 / 255.0,
            lut[at + 1] as f32 / 255.0,
            lut[at + 2] as f32 / 255.0,
        ]
    };
    let bilinear = |u: f32, v: f32| -> [f32; 3] {
        let px = u * w - 0.5;
        let py = v * h - 0.5;
        let (x0, y0) = (px.floor(), py.floor());
        let (fx, fy) = (px - x0, py - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = texel(x0, y0);
        let b = texel(x0 + 1, y0);
        let cc = texel(x0, y0 + 1);
        let d = texel(x0 + 1, y0 + 1);
        let mut o = [0.0; 3];
        for i in 0..3 {
            let top = a[i] + fx * (b[i] - a[i]);
            let bot = cc[i] + fx * (d[i] - cc[i]);
            o[i] = top + fy * (bot - top);
        }
        o
    };
    let s0 = bilinear(ux, vz);
    let s1 = bilinear(ux + params[1], vz);
    let t = z - shift;
    [
        s0[0] + t * (s1[0] - s0[0]),
        s0[1] + t * (s1[1] - s0[1]),
        s0[2] + t * (s1[2] - s0[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::PostProcessProfile;

    /// Research instrument: the Rust bake against the shipped LDR LUT builder
    /// program, executed texel by texel on the uniforms the source LUT pass
    /// packs (receipt directory `MOLY_LUT_ORACLE`: per profile a
    /// `<profile>.lut.bin` with the stored bytes and a `<profile>.texels.json`
    /// with the program's binary32 output before storage; profile names are
    /// relative to `<MOLY_ASSET_ROOT>/phenomena` with `/` spelled `__`).
    ///
    /// Two arms. Values: every channel of every texel within a bound of the
    /// program output. The two evaluations run the same binary32 operations in
    /// the same order; they differ only in `log2`/`exp2`, which the executor
    /// takes from its numeric library and Rust from the platform's.
    /// Replaying this module's arithmetic with correctly rounded `log2`/`exp2`
    /// gives the Rust value of the worst texel bit for bit, and with the
    /// executor's library the program value bit for bit, so the residual is
    /// that last-place difference, amplified by up to about forty where the
    /// HSV round trip rebuilds a channel next to one far above 1. A device runs
    /// the program with its own `log2`/`exp2`, so this band is not fixed by the
    /// source either. Bytes: every stored byte equal, except where the program
    /// output and ours straddle a rounding boundary of the 8-bit store; such a
    /// byte may differ by one and only there.
    #[test]
    #[ignore = "MOLY_LUT_ORACLE must name the executed LutBuilderLdr receipts; MOLY_ASSET_ROOT the profiles"]
    fn bake_matches_the_shipped_lut_builder_program() {
        use crate::weather::json;
        const VALUE_BOUND: f32 = 1.0e-5;
        let oracle = std::path::PathBuf::from(
            std::env::var("MOLY_LUT_ORACLE")
                .expect("MOLY_LUT_ORACLE must name the receipt directory"),
        );
        let root = std::path::PathBuf::from(
            std::env::var("MOLY_ASSET_ROOT").expect("MOLY_ASSET_ROOT must name the runtime root"),
        )
        .join("phenomena");
        let mut names: Vec<String> = std::fs::read_dir(&oracle)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".lut.bin").map(str::to_string))
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no receipts in {oracle:?}");
        let (mut tables, mut bytes, mut distinct_from_first) = (0usize, 0usize, 0usize);
        let (mut worst, mut straddles, mut nontrivial) = (0.0f32, 0usize, 0usize);
        let mut worst_at = String::new();
        let mut first: Option<Vec<u8>> = None;
        for name in &names {
            let path = root.join(name.replace("__", "/")).join("postprocess.json");
            let profile = PostProcessProfile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
            let stack = profile
                .lut_stack()
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let inputs = LdrLutInputs::from_stack(&stack);
            let ours = bake(&inputs);
            let source = std::fs::read(oracle.join(format!("{name}.lut.bin"))).unwrap();
            assert_eq!(source.len(), LUT_WIDTH * LUT_SIZE * 4, "{name}");
            let floats =
                json::parse(&std::fs::read(oracle.join(format!("{name}.texels.json"))).unwrap())
                    .unwrap();
            let texels = floats
                .get("texels")
                .and_then(|t| t.as_array())
                .expect("texels");
            assert_eq!(
                texels.len(),
                LUT_WIDTH * LUT_SIZE,
                "{name}: the value receipt must cover every texel"
            );
            let mut seen = vec![false; LUT_WIDTH * LUT_SIZE];
            for t in texels {
                let t = t.as_array().unwrap();
                let (x, y) = (
                    t[0].as_f64().unwrap() as usize,
                    t[1].as_f64().unwrap() as usize,
                );
                let program: Vec<f32> = t[2]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap() as f32)
                    .collect();
                let index = y * LUT_WIDTH + x;
                assert!(!seen[index], "{name}: texel ({x}, {y}) twice");
                seen[index] = true;
                let c = grade(&inputs, strip_input(x, y));
                for ch in 0..3 {
                    let (a, b) = (c[ch], program[ch]);
                    let diff = (a.clamp(0.0, 1.0) - b.clamp(0.0, 1.0)).abs();
                    if diff > worst {
                        worst = diff;
                        worst_at = format!(
                            "{name} texel ({x}, {y}) channel {ch}: ours {a:e} program {b:e}"
                        );
                    }
                    nontrivial += usize::from(b > 0.0 && b < 1.0);
                    let at = index * 4 + ch;
                    if ours[at] != source[at] {
                        let low = ours[at].min(source[at]);
                        let boundary = (f32::from(low) + 0.5) / 255.0;
                        let (lo, hi) = (a.min(b), a.max(b));
                        assert!(
                            ours[at].abs_diff(source[at]) == 1 && lo <= boundary + 1e-7 && boundary <= hi + 1e-7,
                            "{name} texel ({x}, {y}) channel {ch}: bytes ours {} source {} not a straddle (ours {a:e} program {b:e})",
                            ours[at],
                            source[at]
                        );
                        straddles += 1;
                    }
                }
                assert_eq!(
                    ours[index * 4 + 3],
                    source[index * 4 + 3],
                    "{name}: alpha at ({x}, {y})"
                );
            }
            match &first {
                None => first = Some(source.clone()),
                Some(f) => distinct_from_first += usize::from(f != &source),
            }
            tables += 1;
            bytes += source.len();
        }
        eprintln!(
            "{tables} tables, {bytes} bytes; worst |ours - program| = {worst:e} ({worst_at}); {straddles} bytes straddle a rounding boundary; {nontrivial} channels strictly inside (0, 1); {distinct_from_first} tables differ from the first"
        );
        assert!(
            worst <= VALUE_BOUND,
            "worst value difference {worst:e} at {worst_at}"
        );
        // Positive arms: the receipts are not one table repeated, and the
        // compared channels are not all clamped to 0 or 1.
        assert!(tables < 2 || distinct_from_first > 0);
        assert!(nontrivial > tables * LUT_WIDTH * LUT_SIZE);
    }

    /// Research instrument: the CPU lookup against the shipped engine uber
    /// program (no bloom, no vignette, no user LUT) executed on sampled scene
    /// colours with a receipt table bound as the internal LUT (receipt file
    /// `MOLY_LUT_APPLY_ORACLE`: `{"profile", "lut", "lutParams", "rows": [[in r,
    /// g, b, out r, g, b], ...]}`, colours in the encoded domain as the program
    /// reads and writes them). The program's own decode and encode are
    /// transcribed here with its literals.
    #[test]
    #[ignore = "MOLY_LUT_APPLY_ORACLE must name the executed engine uber receipt"]
    fn apply_matches_the_shipped_engine_uber_program() {
        use crate::weather::json;
        let path = std::env::var("MOLY_LUT_APPLY_ORACLE").expect("MOLY_LUT_APPLY_ORACLE");
        let receipt = json::parse(&std::fs::read(&path).unwrap()).unwrap();
        let lut_path = std::path::Path::new(&path)
            .with_file_name(receipt.get("lut").unwrap().as_str().unwrap());
        let lut = std::fs::read(lut_path).unwrap();
        let p: Vec<f32> = receipt
            .get("lutParams")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        let params = [p[0], p[1], p[2], p[3]];
        let decode = |x: f32| {
            let x = x.min(100.0);
            if 0.040_45 >= x {
                x * 0.077_399_38
            } else {
                (((x + 0.055) * 0.947_867_3).abs().log2() * 2.4).exp2()
            }
        };
        let encode = |x: f32| {
            if 0.003_130_8 >= x {
                x * 12.923_21
            } else {
                (x.abs().log2() * 0.416_666_66).exp2() * 1.055 + -0.055
            }
        };
        let rows = receipt.get("rows").unwrap().as_array().unwrap();
        assert!(!rows.is_empty());
        let mut worst = 0.0f32;
        for row in rows {
            let r: Vec<f32> = row
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap() as f32)
                .collect();
            let linear = [decode(r[0]), decode(r[1]), decode(r[2])];
            let out = apply(&lut, params, linear).map(encode);
            for i in 0..3 {
                worst = worst.max((out[i] - r[3 + i]).abs());
            }
        }
        eprintln!("{} rows, worst |ours - program| = {worst:e}", rows.len());
        // The program and this transcription evaluate the same operations in
        // the same order in binary32; the bound is a few ulps of the encode.
        assert!(worst <= 2.0e-6, "worst {worst}");
    }
}
